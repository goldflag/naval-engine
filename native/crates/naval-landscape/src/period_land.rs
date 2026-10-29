//! The ground as it stood in a past period (native/docs/terrain/period.md, "The period's ground"): which of today's
//! built-up land stood in the scenario's year, and the marks laid over the land cover where it did not
//! (`naval_terrain::surface_maps::cover::PAST_COUNTRY`), which the land draws as the period's country. What reads it:
//! the marks, the roads drawn on the land (a street of land the period leaves empty is not drawn). Presentation only, as
//! the rest of the theatre's land.
//!
//! - **The buildings' answer.** Where the buildings' thinning has run for a chunk (`buildings::period`, every chunk the
//!   buildings draw about the camera) the ground answers as they stand (`Thinning::stood`): a point stood where the
//!   score between its cells' centres reaches its cell's bar unlifted, or on the land the buildings kept whole (a listed
//!   yard, a port's working waterfront, the ground about what a land's lift kept: `Thinning::kept`; a church the
//!   worship's lift keeps stands on the period's country). The buildings' worker hands each chunk's answer in as it is
//!   meshed ([`thinned`]).
//! - **The cover's estimate** beyond them ([`estimate`]), from the window's land cover alone, by the buildings' rules:
//!   each 100 m cell scored by its share built up today ([`COVERAGE`] of a cell's ground covered by footprints a whole
//!   share), smoothed as the footprints are and with the same noise; each 5' cell's bar keeping its share of today's
//!   built-up land, densest first (a whole cell counted as [`FOOTPRINTS_PER_CELL`] footprints), never under the
//!   region's floor; each cell's bar blended between the four 5' cells about it; a listed yard standing whole
//!   (`thinning::YARDS`; a working waterfront cannot be told from the cover alone).
//! - **The marks** ([`mark`]): a texel built up today where its ground did not stand, and any other land (gardens, lawns,
//!   street trees, a park) inside a cell whose land is [`MOSTLY_BUILT`] today; never water, snow or the unknown. The
//!   window's maps are marked with the scene's build ([`mark_window`]), each near tile as it is made (`near`); where
//!   the buildings' answer comes after, the driver marks the chunk again in the window's cover ([`Feed`]) and the near
//!   tiles' worker its tiles.
//! - **Today** answers that every ground stood and marks nothing: [`Period::Today`] draws as before, bit for bit.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use naval_conditions::SurfaceStyle;
use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_terrain::buildings::thinning::{self, CELL_M, CELLS, Distribution, RING, Thinning};
use naval_terrain::buildings::towns::Towns;
use naval_terrain::buildings::{CHUNK_M, Period, Region, chunk_corner, chunk_of};
use naval_terrain::surface_maps::cover;
use naval_terrain::{CoverLayer, CoverPatch, SurfaceMaps, TerrainLayer};

/// The ground a cell's footprints cover (a share of the cell) for each share of the cell built up today: what the cover's
/// estimate scores a cell by in place of its footprints' (calibrated against the buildings' answer where both are had,
/// native/docs/terrain/period.md, "The period's ground").
pub const COVERAGE: f32 = 0.3;
/// The footprints a cell wholly built up counts as in its 5' cell's distribution and least sample.
pub const FOOTPRINTS_PER_CELL: f32 = 12.0;
/// A cell this much of whose land is built up today is a town's: its other land (gardens, lawns, street trees, a park)
/// is marked with its built-up land where it did not stand.
pub const MOSTLY_BUILT: f32 = 0.5;
/// The buildings' chunks kept (about 1 kB each), as many as the buildings keep chunks' footprints.
const THINNED_KEPT: usize = 8192;
/// Changes remembered for the lands marked from older answers ([`changed_since`]).
const CHANGES_KEPT: usize = 4096;
/// Windows' estimates kept (a sea window's and an air window's, and one being built).
const ESTIMATES_KEPT: usize = 3;
/// The window's chunks marked again in a frame at most, and the time they may take ([`Feed`]).
pub const FRAME_CHUNKS: usize = 32;
pub const FRAME_BUDGET: Duration = Duration::from_micros(500);
/// Bits of a marks number ([`marks`]) holding the answers' version; the year above them.
const VERSION_BITS: u32 = 48;

/// How far from a theatre's origin the listed towns are placed (m): beyond the widest theatre.
pub const TOWNS_REACH_M: f64 = 2_000_000.0;

/// The listed towns standing in `year` on the theatre about `geo` (`thinning`'s, [`Towns`]); none with
/// `NAVAL_TOWNS=off` (for a before and after).
pub fn towns(geo: LatLon, year: u16) -> Towns {
    static OFF: OnceLock<bool> = OnceLock::new();
    if *OFF.get_or_init(|| std::env::var("NAVAL_TOWNS").is_ok_and(|v| v == "off")) {
        return Towns::default();
    }
    Towns::new(
        year,
        &|lat, lon| to_local(geo, LatLon { lat, lon }),
        TOWNS_REACH_M,
    )
}

/// The scenario's frame (its origin's bits).
fn frame(geo: LatLon) -> [u64; 2] {
    [geo.lat.to_bits(), geo.lon.to_bits()]
}

// ---------------------------------------------------------------------------------------------------------------------
// The answers.

/// One chunk's answer from the land cover ([`estimate`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Estimated {
    /// The scores at the centres of the chunk's cells and of the ring about them, and its cells' bars, as a buildings'
    /// chunk's (no cap, no landmarks).
    pub thinning: Thinning,
    /// The share of the land built up today at the same centres (12 × 12, `thinning::ring_at`).
    pub built: Vec<f32>,
}

/// A window's estimate: chunks `first` to `first + size` (columns, rows), each with built-up land's answer, from cover
/// of `texel_m` texels.
#[derive(Debug)]
pub struct Estimate {
    pub first: [i32; 2],
    pub size: [usize; 2],
    pub texel_m: f64,
    chunks: Vec<Option<Arc<Estimated>>>,
}

impl Estimate {
    fn slot(&self, index: [i32; 2]) -> Option<usize> {
        let (x, z) = (index[0] - self.first[0], index[1] - self.first[1]);
        ((0..self.size[0] as i32).contains(&x) && (0..self.size[1] as i32).contains(&z))
            .then(|| z as usize * self.size[0] + x as usize)
    }

    /// Whether the window covers chunk `index`.
    pub fn covers(&self, index: [i32; 2]) -> bool {
        self.slot(index).is_some()
    }

    /// Chunk `index`'s answer, none where it holds no built-up land (or lies outside the window).
    pub fn get(&self, index: [i32; 2]) -> Option<&Estimated> {
        self.slot(index).and_then(|k| self.chunks[k].as_deref())
    }

    /// The chunks holding built-up land.
    pub fn built_chunks(&self) -> usize {
        self.chunks.iter().flatten().count()
    }

    /// The chunks covered (first column, first row, last column, last row).
    fn rect(&self) -> [i32; 4] {
        [
            self.first[0],
            self.first[1],
            self.first[0] + self.size[0] as i32 - 1,
            self.first[1] + self.size[1] as i32 - 1,
        ]
    }
}

/// The eight chunks about a chunk (columns, rows), as [`Ground::neighbours`] holds them.
const NEIGHBOURS: [[i32; 2]; 8] = [
    [-1, -1],
    [0, -1],
    [1, -1],
    [-1, 0],
    [1, 0],
    [-1, 1],
    [0, 1],
    [1, 1],
];

/// What is known of a chunk's ground: the buildings' answer, the cover's, and the buildings' answers about it (the
/// ground they kept over their edges).
#[derive(Clone, Copy, Default)]
pub struct Ground<'a> {
    pub thinned: Option<&'a Thinning>,
    pub estimated: Option<&'a Estimated>,
    pub neighbours: [Option<&'a Thinning>; 8],
}

impl Ground<'_> {
    /// Whether anything is known.
    pub fn known(&self) -> bool {
        self.thinned.is_some() || self.estimated.is_some()
    }

    /// Whether the ground at chunk-local point `p` stood: as the buildings stand there (their score unlifted, or the
    /// land they kept whole: `Thinning::stood`), else as the cover's estimate (with the listed yards); stood where
    /// nothing is known, or where a neighbour's buildings kept the ground over their common edge.
    pub fn stood(&self, p: [f32; 2]) -> bool {
        let own = match (self.thinned, self.estimated) {
            (Some(t), _) => t.stood(p),
            (None, Some(e)) => e.thinning.stood(p),
            (None, None) => true,
        };
        own || self.kept_about(p)
    }

    /// Whether a neighbouring chunk's buildings kept the ground at chunk-local point `p` (about a footprint over their
    /// common edge, `thinning::KEPT_BORDER`).
    fn kept_about(&self, p: [f32; 2]) -> bool {
        let side = CHUNK_M as f32;
        let m = thinning::KEPT_BORDER as f32 * thinning::KEPT_CELL_M;
        if p[0] >= m && p[0] < side - m && p[1] >= m && p[1] < side - m {
            return false;
        }
        NEIGHBOURS.iter().zip(&self.neighbours).any(|(d, n)| {
            n.is_some_and(|t| {
                t.kept
                    .holds([p[0] - d[0] as f32 * side, p[1] - d[1] as f32 * side])
            })
        })
    }

    /// The share of the land built up today at chunk-local point `p` (none without the cover's estimate).
    pub fn built(&self, p: [f32; 2]) -> f32 {
        self.estimated
            .map_or(0.0, |e| thinning::ring_at(&e.built, p))
    }
}

/// Everything known of a scenario's ground in a past year: the buildings' chunks and the windows' estimates (the finest
/// first, the newest first of the same texels, each with the version it was held at). Shared and never changed once
/// made: a change makes another.
#[derive(Clone, Default)]
pub struct Answers {
    frame: [u64; 2],
    year: u16,
    version: u64,
    thinned: HashMap<[i32; 2], Arc<Thinning>>,
    estimates: Vec<(u64, Arc<Estimate>)>,
}

impl Answers {
    /// Whether these are the answers of the scenario about `geo` in `year`.
    fn holds(&self, geo: LatLon, year: u16) -> bool {
        self.year == year && self.frame == frame(geo)
    }

    /// The version these answers were made at ([`version`]).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// What is known of chunk `index`'s ground: the buildings' answer, and the cover's from the finest window over it
    /// (a sea window's over an air window's, so a sea scene's tiles read as its window was marked).
    pub fn ground(&self, index: [i32; 2]) -> Ground<'_> {
        Ground {
            thinned: self.thinned.get(&index).map(Arc::as_ref),
            estimated: self
                .estimates
                .iter()
                .find(|(_, e)| e.covers(index))
                .and_then(|(_, e)| e.get(index)),
            neighbours: NEIGHBOURS.map(|d| {
                self.thinned
                    .get(&[index[0] + d[0], index[1] + d[1]])
                    .map(Arc::as_ref)
            }),
        }
    }

    /// Whether the ground at theatre point `p` stood.
    pub fn stood(&self, p: [f64; 2]) -> bool {
        let index = chunk_of(p);
        let c = chunk_corner(index);
        self.ground(index)
            .stood([(p[0] - c[0]) as f32, (p[1] - c[1]) as f32])
    }

    /// The chunks the buildings have answered for.
    pub fn thinned_chunks(&self) -> impl Iterator<Item = [i32; 2]> + '_ {
        self.thinned.keys().copied()
    }
}

/// A change to the answers: the version it made, the chunks it touched (first column, first row, last column, last
/// row), and whether it was a window's estimate (else the buildings' chunks).
#[derive(Clone, Copy, Debug)]
struct Change {
    version: u64,
    chunks: [i32; 4],
    estimate: bool,
}

/// What is held of one scenario's ground in one year.
struct Context {
    answers: Arc<Answers>,
    /// The buildings' chunks, oldest first.
    order: VecDeque<[i32; 2]>,
    changes: VecDeque<Change>,
    /// Every change after this version is in `changes`.
    floor: u64,
}

impl Context {
    fn new(geo: LatLon, year: u16) -> Self {
        let version = bump();
        Self {
            answers: Arc::new(Answers {
                frame: frame(geo),
                year,
                version,
                ..Answers::default()
            }),
            order: VecDeque::new(),
            changes: VecDeque::new(),
            floor: version,
        }
    }

    fn note(&mut self, change: Change) {
        self.changes.push_back(change);
        while self.changes.len() > CHANGES_KEPT {
            if let Some(old) = self.changes.pop_front() {
                self.floor = self.floor.max(old.version);
            }
        }
    }

    /// Whether the answers changed after `since` over chunks `chunks` (first column, first row, last column, last row):
    /// by the buildings, or with `estimates` by a window's estimate too; true when the changes since are no longer
    /// known.
    fn changed_since(&self, since: u64, chunks: [i32; 4], estimates: bool) -> bool {
        since < self.floor
            || self
                .changes
                .iter()
                .rev()
                .take_while(|c| c.version > since)
                .any(|c| {
                    (estimates || !c.estimate)
                        && c.chunks[0] <= chunks[2]
                        && chunks[0] <= c.chunks[2]
                        && c.chunks[1] <= chunks[3]
                        && chunks[1] <= c.chunks[3]
                })
    }
}

/// The scenarios' contexts, the one last used last: a batch of the buildings' finishing for the scenario left behind
/// answers for it alone, and a scenario returned to (its buildings' meshes cached, never thinned again) finds its
/// answers still held.
struct State {
    contexts: Vec<Context>,
}

/// Contexts held (the scenario on screen and the one before it).
const CONTEXTS_KEPT: usize = 2;

static VERSION: AtomicU64 = AtomicU64::new(0);

fn state() -> MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(State {
                contexts: Vec::new(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// The next version.
fn bump() -> u64 {
    VERSION.fetch_add(1, Ordering::Relaxed) + 1
}

impl State {
    /// The context of the scenario about `geo` in `year`, if held.
    fn find(&self, geo: LatLon, year: u16) -> Option<&Context> {
        self.contexts.iter().find(|c| c.answers.holds(geo, year))
    }

    /// The context of the scenario about `geo` in `year`, made when none is held (the least lately used let go), and
    /// now the last used.
    fn context(&mut self, geo: LatLon, year: u16) -> &mut Context {
        let context = match self
            .contexts
            .iter()
            .position(|c| c.answers.holds(geo, year))
        {
            Some(k) => self.contexts.remove(k),
            None => Context::new(geo, year),
        };
        self.contexts.push(context);
        while self.contexts.len() > CONTEXTS_KEPT {
            self.contexts.remove(0);
        }
        self.contexts.last_mut().expect("a context")
    }
}

/// Whether the ground at theatre point `p` (metres, in the frame whose origin is `geo`) stood in `period`: false only
/// where today's built-up land was country then. Always true for [`Period::Today`], and wherever nothing is known yet.
pub fn stood(geo: LatLon, period: Period, p: [f64; 2]) -> bool {
    answers(geo, period).is_none_or(|a| a.stood(p))
}

/// Rises whenever what [`stood`] answers about some ground changes (the buildings' thinning taking over from the
/// cover's estimate about the camera, a window's estimate), so what was made from it can be made again.
pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

/// The answers held for the scenario about `geo` in `period`: none for today, or while none are held for it.
pub fn answers(geo: LatLon, period: Period) -> Option<Arc<Answers>> {
    let Period::Year(year) = period else {
        return None;
    };
    state().find(geo, year).map(|c| c.answers.clone())
}

/// The buildings' thinning of `chunks` in the scenario about `geo` in `year` (`buildings::period::prepare`, each chunk
/// gathered in full): the ground answers as the buildings do there from now on.
pub fn thinned(geo: LatLon, year: u16, chunks: Vec<([i32; 2], Thinning)>) {
    if chunks.is_empty() {
        return;
    }
    let mut state = state();
    let context = state.context(geo, year);
    let mut answers = (*context.answers).clone();
    let mut changed = Vec::new();
    for (index, t) in chunks {
        // The same answer again (a chunk made again, or at its other level of detail, which reads the same map in the
        // period): nothing new.
        if answers.thinned.get(&index).is_some_and(|held| {
            held.score == t.score
                && held.bar == t.bar
                && held.yards == t.yards
                && held.kept == t.kept
                && held.town == t.town
        }) {
            continue;
        }
        let t = Thinning {
            tall: Vec::new(),
            later: Vec::new(),
            not_yet: Vec::new(),
            ..t
        };
        let spills = t.kept.spills();
        let held = answers.thinned.insert(index, Arc::new(t));
        // Ground kept over the chunk's edges (now or before) changes its neighbours' too.
        if spills || held.as_ref().is_some_and(|h| h.kept.spills()) {
            changed.extend(NEIGHBOURS.map(|d| [index[0] + d[0], index[1] + d[1]]));
        }
        if held.is_none() {
            context.order.push_back(index);
        }
        changed.push(index);
    }
    while context.order.len() > THINNED_KEPT {
        if let Some(old) = context.order.pop_front() {
            if answers
                .thinned
                .remove(&old)
                .is_some_and(|t| t.kept.spills())
            {
                changed.extend(NEIGHBOURS.map(|d| [old[0] + d[0], old[1] + d[1]]));
            }
            changed.push(old);
        }
    }
    if changed.is_empty() {
        return;
    }
    changed.sort_unstable();
    changed.dedup();
    let version = bump();
    answers.version = version;
    for index in changed {
        context.note(Change {
            version,
            chunks: [index[0], index[1], index[0], index[1]],
            estimate: false,
        });
    }
    context.answers = Arc::new(answers);
}

/// Holds `estimate` (a window's, [`estimate`]) for the scenario about `geo` in `year`, over any older one of the same
/// chunks and texels, the oldest let go beyond [`ESTIMATES_KEPT`]: the answers now held.
pub fn hold_estimate(geo: LatLon, year: u16, estimate: Arc<Estimate>) -> Arc<Answers> {
    let mut state = state();
    let context = state.context(geo, year);
    let mut answers = (*context.answers).clone();
    let version = bump();
    answers.estimates.retain(|(_, e)| {
        e.first != estimate.first || e.size != estimate.size || e.texel_m != estimate.texel_m
    });
    answers.estimates.push((version, estimate.clone()));
    while answers.estimates.len() > ESTIMATES_KEPT {
        if let Some(oldest) = (0..answers.estimates.len()).min_by_key(|&k| answers.estimates[k].0) {
            answers.estimates.remove(oldest);
        }
    }
    answers
        .estimates
        .sort_by(|a, b| a.1.texel_m.total_cmp(&b.1.texel_m).then(b.0.cmp(&a.0)));
    answers.version = version;
    context.note(Change {
        version,
        chunks: estimate.rect(),
        estimate: true,
    });
    context.answers = Arc::new(answers);
    context.answers.clone()
}

/// Whether the answers about the scenario at `geo` in `period` changed after `since` over chunks `chunks` (first
/// column, first row, last column, last row): by the buildings, or with `estimates` by a window's estimate too. True
/// when the changes since are no longer known; false for today, and while nothing is known of the scenario's past
/// ground (marking again would lay nothing new).
pub fn changed_since(
    geo: LatLon,
    period: Period,
    since: u64,
    chunks: [i32; 4],
    estimates: bool,
) -> bool {
    let Period::Year(year) = period else {
        return false;
    };
    state()
        .find(geo, year)
        .is_some_and(|c| c.changed_since(since, chunks, estimates))
}

/// The chunks the buildings answered for anew after `since` in the scenario about `geo` in `year`, or none when the
/// changes since are no longer known.
fn thinned_since(geo: LatLon, year: u16, since: u64) -> Option<Vec<[i32; 2]>> {
    let state = state();
    let context = state.find(geo, year)?;
    if since < context.floor {
        return None;
    }
    let mut out: Vec<[i32; 2]> = context
        .changes
        .iter()
        .rev()
        .take_while(|c| c.version > since)
        .filter(|c| !c.estimate)
        .map(|c| [c.chunks[0], c.chunks[1]])
        .collect();
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// The number a land keeps for the marks over a cover (`TerrainLayer::set_cover_marks`, `NearTile::marks`): the
/// period's year and the version of the answers they were laid from; 0 for today's (none).
pub fn marks(period: Period, version: u64) -> u64 {
    match period {
        Period::Year(year) => {
            (u64::from(year) << VERSION_BITS) | (version & ((1 << VERSION_BITS) - 1))
        }
        Period::Today => 0,
    }
}

/// The period a marks number ([`marks`]) was laid for.
pub fn marks_period(marks: u64) -> Period {
    match (marks >> VERSION_BITS) as u16 {
        0 => Period::Today,
        year => Period::Year(year),
    }
}

/// The version of the answers a marks number ([`marks`]) was laid from.
pub fn marks_version(marks: u64) -> u64 {
    marks & ((1 << VERSION_BITS) - 1)
}

// ---------------------------------------------------------------------------------------------------------------------
// The marks.

/// A rectangle of cover texels on the theatre: texel (i, j) from theatre point `corner` + (i, j) × `texel_m`, `size`
/// columns and rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub corner: [f64; 2],
    pub texel_m: f64,
    pub size: [usize; 2],
}

impl Grid {
    /// A window's maps whose chart (0, 0) is theatre point `centre`.
    pub fn of_maps(maps: &SurfaceMaps, centre: [f64; 2]) -> Self {
        Self {
            corner: [centre[0] + maps.origin[0], centre[1] + maps.origin[1]],
            texel_m: maps.extent_m / f64::from(maps.size),
            size: [maps.size as usize; 2],
        }
    }

    /// The texels from `at`, `size` of them.
    pub fn part(&self, at: [usize; 2], size: [usize; 2]) -> Self {
        Self {
            corner: [
                self.corner[0] + at[0] as f64 * self.texel_m,
                self.corner[1] + at[1] as f64 * self.texel_m,
            ],
            texel_m: self.texel_m,
            size,
        }
    }

    /// The texels along axis `a` (0 columns, 1 rows) whose centres lie in chunk column (or row) `c`: first and end.
    pub fn span(&self, a: usize, c: i32) -> (usize, usize) {
        let edge = |c: i32| {
            let t = ((f64::from(c) * CHUNK_M - self.corner[a]) / self.texel_m - 0.5).ceil();
            (t.max(0.0) as usize).min(self.size[a])
        };
        (edge(c), edge(c + 1))
    }

    /// The first and last chunk holding a texel's centre.
    pub fn chunks(&self) -> ([i32; 2], [i32; 2]) {
        let at = |a: usize, k: usize| {
            ((self.corner[a] + (k as f64 + 0.5) * self.texel_m) / CHUNK_M).floor() as i32
        };
        (
            [at(0, 0), at(1, 0)],
            [
                at(0, self.size[0].saturating_sub(1)),
                at(1, self.size[1].saturating_sub(1)),
            ],
        )
    }
}

/// Whether a class today is land the period may leave as country: built-up land, or the other land of a town's cells.
fn country_class(class: u8) -> bool {
    matches!(
        class,
        cover::TREE
            | cover::SHRUB
            | cover::GRASS
            | cover::CROP
            | cover::BUILT
            | cover::BARE
            | cover::WETLAND
            | cover::MANGROVE
            | cover::MOSS
    )
}

/// The marks over one band of rows (chunk row `cz`, its first row `j0` of `grid`) for chunk columns `columns`.
fn mark_rows(
    answers: &Answers,
    grid: &Grid,
    cz: i32,
    j0: usize,
    rows: &mut [u8],
    columns: std::ops::RangeInclusive<i32>,
) -> usize {
    let w = grid.size[0];
    let h = rows.len() / w;
    let t = grid.texel_m;
    let mut marked = 0;
    for cx in columns {
        let (i0, i1) = grid.span(0, cx);
        if i0 >= i1 {
            continue;
        }
        let ground = answers.ground([cx, cz]);
        if !ground.known() {
            for r in 0..h {
                for class in &mut rows[r * w + i0..r * w + i1] {
                    *class = cover::today(*class);
                }
            }
            continue;
        }
        let corner = chunk_corner([cx, cz]);
        for r in 0..h {
            let z = (grid.corner[1] + ((j0 + r) as f64 + 0.5) * t - corner[1]) as f32;
            for i in i0..i1 {
                let k = r * w + i;
                let class = cover::today(rows[k]);
                let country = country_class(class) && {
                    let x = (grid.corner[0] + (i as f64 + 0.5) * t - corner[0]) as f32;
                    (class == cover::BUILT || ground.built([x, z]) >= MOSTLY_BUILT)
                        && !ground.stood([x, z])
                };
                rows[k] = if country {
                    marked += 1;
                    class | cover::PAST_COUNTRY
                } else {
                    class
                };
            }
        }
    }
    marked
}

/// Lays the period's marks over `classes` (the cover of `grid`, row by row; any marks there taken off first) from
/// `answers`: a texel built up today where its ground did not stand, and other land inside a cell whose land is
/// [`MOSTLY_BUILT`] today likewise; never water, snow or the unknown. How many texels are marked.
pub fn mark(answers: &Answers, grid: &Grid, classes: &mut [u8]) -> usize {
    let [w, h] = grid.size;
    assert_eq!(classes.len(), w * h, "a class a texel");
    if w == 0 || h == 0 {
        return 0;
    }
    let (first, last) = grid.chunks();
    // Each chunk row's rows, in order (the rows' centres fall in one chunk row each).
    let mut bands: Vec<(i32, usize, &mut [u8])> = Vec::new();
    let mut rest = classes;
    let mut at = 0;
    for cz in first[1]..=last[1] {
        let (j0, j1) = grid.span(1, cz);
        if j0 >= j1 {
            continue;
        }
        let (_, tail) = std::mem::take(&mut rest).split_at_mut((j0 - at) * w);
        let (band, tail) = tail.split_at_mut((j1 - j0) * w);
        bands.push((cz, j0, band));
        rest = tail;
        at = j1;
    }
    let columns = first[0]..=last[0];
    let threads = if w * h >= 1 << 18 {
        std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .clamp(1, 8)
    } else {
        1
    };
    if threads <= 1 || bands.len() <= 1 {
        return bands
            .into_iter()
            .map(|(cz, j0, band)| mark_rows(answers, grid, cz, j0, band, columns.clone()))
            .sum();
    }
    let per = bands.len().div_ceil(threads);
    // At the caller's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        let mut bands = bands;
        while !bands.is_empty() {
            let take = per.min(bands.len());
            let part: Vec<_> = bands.drain(..take).collect();
            let columns = columns.clone();
            handles.push(s.spawn(move || {
                qos.apply();
                part.into_iter()
                    .map(|(cz, j0, band)| mark_rows(answers, grid, cz, j0, band, columns.clone()))
                    .sum::<usize>()
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("a marking thread panicked"))
            .sum()
    })
}

/// The window's cover marked for `period` (its maps `maps`, their chart's (0, 0) at theatre point `centre`, in the
/// scenario about `geo` with the land's `style`), the window's estimate held for the ground beyond the buildings', and
/// its marks number ([`marks`]); none for today (the maps' own cover is drawn, as before). Slow (tens of milliseconds
/// for a sea window's 4096²): with the scene's build, off the frame loop's thread.
pub fn mark_window(
    geo: LatLon,
    period: Period,
    style: &SurfaceStyle,
    maps: &SurfaceMaps,
    centre: [f64; 2],
) -> Option<(Vec<u8>, u64)> {
    let Period::Year(year) = period else {
        return None;
    };
    if !maps.is_consistent() {
        return None;
    }
    let started = Instant::now();
    let estimate = Arc::new(estimate(geo, year, style, maps, centre));
    let estimated = started.elapsed();
    let answers = hold_estimate(geo, year, estimate.clone());
    let mut classes = maps.cover.clone();
    let marked = mark(&answers, &Grid::of_maps(maps, centre), &mut classes);
    eprintln!(
        "the period's ground: {}² ({:.0} m texels, {year}): {} chunks with built-up land, {} texels marked; \
         estimated in {:.0} ms, marked in {:.0} ms",
        maps.size,
        maps.extent_m / f64::from(maps.size),
        estimate.built_chunks(),
        marked,
        estimated.as_secs_f64() * 1e3,
        (started.elapsed() - estimated).as_secs_f64() * 1e3,
    );
    Some((classes, marks(period, answers.version())))
}

// ---------------------------------------------------------------------------------------------------------------------
// The cover's estimate.

/// A chunk's cells' shares built up today: of their ground, and of their land.
struct Shares {
    ground: [f32; CELLS * CELLS],
    land: [f32; CELLS * CELLS],
}

/// Chunk `index`'s shares from `classes` over `grid`, sampled `n` × `n` a cell; none where it holds no built-up land.
fn shares_of(grid: &Grid, classes: &[u8], index: [i32; 2], n: usize) -> Option<Box<Shares>> {
    let corner = chunk_corner(index);
    let along = |a: usize| -> Vec<Option<usize>> {
        (0..CELLS * n)
            .map(|s| {
                let v = corner[a] + (s as f64 + 0.5) * f64::from(CELL_M) / n as f64;
                let t = ((v - grid.corner[a]) / grid.texel_m).floor();
                (t >= 0.0 && t < grid.size[a] as f64).then_some(t as usize)
            })
            .collect()
    };
    let (xs, zs) = (along(0), along(1));
    if xs.iter().all(Option::is_none) || zs.iter().all(Option::is_none) {
        return None;
    }
    let w = grid.size[0];
    let mut built = [0u16; CELLS * CELLS];
    let mut land = [0u16; CELLS * CELLS];
    let mut any = false;
    for (sz, z) in zs.iter().enumerate() {
        let Some(z) = z else {
            continue;
        };
        let row = &classes[z * w..(z + 1) * w];
        let cj = sz / n;
        for (sx, x) in xs.iter().enumerate() {
            let Some(x) = x else {
                continue;
            };
            let class = cover::today(row[*x]);
            let k = cj * CELLS + sx / n;
            if class == cover::BUILT {
                built[k] += 1;
                any = true;
            }
            if !matches!(class, cover::UNKNOWN | cover::WATER) {
                land[k] += 1;
            }
        }
    }
    if !any {
        return None;
    }
    let samples = (n * n) as f32;
    Some(Box::new(Shares {
        ground: built.map(|b| f32::from(b) / samples),
        land: std::array::from_fn(|k| {
            if land[k] > 0 {
                f32::from(built[k]) / f32::from(land[k])
            } else {
                0.0
            }
        }),
    }))
}

/// Chunks to a side of a block whose scores are smoothed together (with a chunk's halo about it).
const BLOCK: usize = 8;

/// The cover's estimate over a window's maps (`maps`, their chart's (0, 0) at theatre point `centre`) in the scenario
/// about `geo` in `year`, the land's `style` (its regions): each chunk with built-up land today, its scores, its cells'
/// bars and its land's built share (module docs).
pub fn estimate(
    geo: LatLon,
    year: u16,
    style: &SurfaceStyle,
    maps: &SurfaceMaps,
    centre: [f64; 2],
) -> Estimate {
    estimate_with(geo, year, style, maps, centre, COVERAGE)
}

/// [`estimate`] with a cell wholly built up today scored as footprints covering `coverage` of its ground (in place of
/// [`COVERAGE`]: its calibration, `tests/theatre_period_ground.rs`).
pub fn estimate_with(
    geo: LatLon,
    year: u16,
    style: &SurfaceStyle,
    maps: &SurfaceMaps,
    centre: [f64; 2],
    coverage: f32,
) -> Estimate {
    let grid = Grid::of_maps(maps, centre);
    let (first, last) = grid.chunks();
    let size = [
        (last[0] - first[0] + 1).max(0) as usize,
        (last[1] - first[1] + 1).max(0) as usize,
    ];
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8);
    let index_of = |k: usize| {
        [
            first[0] + (k % size[0]) as i32,
            first[1] + (k / size[0]) as i32,
        ]
    };
    // The cells' shares, a few samples a cell (each a texel of a sea window's maps; one of an air window's).
    let n = ((f64::from(CELL_M) / grid.texel_m).round() as usize).clamp(1, 8);
    let slots: Vec<usize> = (0..size[0] * size[1]).collect();
    let shares: Vec<Option<Box<Shares>>> = super::surface::parallel_map(&slots, threads, |&k| {
        shares_of(&grid, &maps.cover, index_of(k), n)
    });
    let share_at = |index: [i32; 2]| -> Option<&Shares> {
        let (x, z) = (index[0] - first[0], index[1] - first[1]);
        ((0..size[0] as i32).contains(&x) && (0..size[1] as i32).contains(&z))
            .then(|| shares[z as usize * size[0] + x as usize].as_deref())
            .flatten()
    };
    // The scores of each chunk with built-up land and the ring about it, smoothed a block of chunks at a time with a
    // chunk's halo (the smoothing reaches four cells).
    let blocks = [size[0].div_ceil(BLOCK), size[1].div_ceil(BLOCK)];
    let block_list: Vec<usize> = (0..blocks[0] * blocks[1]).collect();
    let rings: Vec<Vec<(usize, Vec<f32>)>> =
        super::surface::parallel_map(&block_list, threads, |&b| {
            let (bx, bz) = (b % blocks[0] * BLOCK, b / blocks[0] * BLOCK);
            let inner = [BLOCK.min(size[0] - bx), BLOCK.min(size[1] - bz)];
            let wanted: Vec<usize> = (0..inner[1])
                .flat_map(|z| (0..inner[0]).map(move |x| (bz + z) * size[0] + bx + x))
                .filter(|&k| shares[k].is_some())
                .collect();
            if wanted.is_empty() {
                return Vec::new();
            }
            let from = [first[0] + bx as i32 - 1, first[1] + bz as i32 - 1];
            let (w, h) = ((inner[0] + 2) * CELLS, (inner[1] + 2) * CELLS);
            let mut share = vec![0f32; w * h];
            for cz in 0..inner[1] + 2 {
                for cx in 0..inner[0] + 2 {
                    let Some(s) = share_at([from[0] + cx as i32, from[1] + cz as i32]) else {
                        continue;
                    };
                    for j in 0..CELLS {
                        for i in 0..CELLS {
                            share[(cz * CELLS + j) * w + cx * CELLS + i] =
                                coverage * s.ground[j * CELLS + i];
                        }
                    }
                }
            }
            let scores = thinning::smoothed_scores(
                [
                    i64::from(from[0]) * CELLS as i64,
                    i64::from(from[1]) * CELLS as i64,
                ],
                [w, h],
                &share,
            );
            wanted
                .into_iter()
                .map(|k| {
                    let (cx, cz) = (k % size[0] - bx + 1, k / size[0] - bz + 1);
                    let ring = (0..RING * RING)
                        .map(|r| {
                            scores[(cz * CELLS + r / RING - 1) * w + cx * CELLS + r % RING - 1]
                        })
                        .collect();
                    (k, ring)
                })
                .collect()
        });
    let mut score: HashMap<usize, Vec<f32>> = rings.into_iter().flatten().collect();
    // The chunks' corners' latitudes and longitudes, the cells' middles between them.
    let built_chunks: Vec<usize> = (0..shares.len()).filter(|&k| shares[k].is_some()).collect();
    let corners: Vec<[LatLon; 4]> = super::surface::parallel_map(&built_chunks, threads, |&k| {
        let index = index_of(k);
        let c = chunk_corner(index);
        [
            [0.0, 0.0],
            [CHUNK_M, 0.0],
            [0.0, CHUNK_M],
            [CHUNK_M, CHUNK_M],
        ]
        .map(|[dx, dz]| to_lat_lon(geo, [c[0] + dx, c[1] + dz]))
    });
    let middle = |c: &[LatLon; 4], k: usize| -> (f64, f64) {
        let u = ((k % CELLS) as f64 + 0.5) / CELLS as f64;
        let v = ((k / CELLS) as f64 + 0.5) / CELLS as f64;
        let lon0 = c[0].lon;
        let unwrap = |lon: f64| lon0 + ((lon - lon0 + 540.0).rem_euclid(360.0) - 180.0);
        let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
        let lat = lerp(lerp(c[0].lat, c[1].lat, u), lerp(c[2].lat, c[3].lat, u), v);
        let lon = lerp(
            lerp(lon0, unwrap(c[1].lon), u),
            lerp(unwrap(c[2].lon), unwrap(c[3].lon), u),
            v,
        );
        (lat, (lon + 540.0).rem_euclid(360.0) - 180.0)
    };
    // The listed towns standing in the year (their land kept apart in the distributions, their ground standing).
    let towns = towns(geo, year);
    // Each 5' cell's built-up land by its cells' scores.
    let mut distributions: HashMap<[i32; 2], Distribution> = HashMap::new();
    for (corner, &k) in corners.iter().zip(&built_chunks) {
        let (Some(s), Some(ring)) = (shares[k].as_deref(), score.get(&k)) else {
            continue;
        };
        let c = chunk_corner(index_of(k));
        let towns = towns.about(c);
        for cell in 0..CELLS * CELLS {
            let count = (s.ground[cell] * FOOTPRINTS_PER_CELL).round() as u32;
            if count == 0 {
                continue;
            }
            let (lat, lon) = middle(corner, cell);
            let at = (cell / CELLS + 1) * RING + cell % CELLS + 1;
            let d = distributions
                .entry(thinning::hyde_cell(lat, lon))
                .or_default();
            let m = thinning::cell_middle(cell);
            if towns.holds([c[0] + f64::from(m[0]), c[1] + f64::from(m[1])]) {
                d.add_town(ring[at], count);
            } else {
                d.add(ring[at], count);
            }
        }
    }
    // Every 5' cell's bar the window's cells may blend: those about its corners and a cell beyond.
    let mut fives: HashMap<[i32; 2], thinning::FiveBars> = HashMap::new();
    for corner in &corners {
        for c in corner {
            let [x, y] = thinning::hyde_cell(c.lat, c.lon);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    // (Wrapped across the date line as `hyde_about` wraps them.)
                    let cell = {
                        let (lat, lon) = thinning::hyde_middle([x + dx, y + dy]);
                        thinning::hyde_cell(lat, lon)
                    };
                    fives.entry(cell).or_insert_with(|| {
                        let (lat, lon) = thinning::hyde_middle(cell);
                        let region = Region::at(lat, lon, style);
                        let d = distributions.get(&cell);
                        thinning::cell_bars(cell, d, year, region)
                    });
                }
            }
        }
    }
    // The listed yards (standing in `year`) on the theatre, for the chunks they reach.
    let yards: Vec<([f64; 2], f32)> = thinning::listed(year)
        .filter(|l| l.3)
        .map(|(lat, lon, r, _)| (to_local(geo, LatLon { lat, lon }), r))
        .collect();
    // Each chunk's answer.
    let answered: Vec<(usize, Arc<Estimated>)> = super::surface::parallel_map(
        &(0..built_chunks.len()).collect::<Vec<_>>(),
        threads,
        |&b| {
            let k = built_chunks[b];
            let corner = &corners[b];
            let bar: Vec<f32> = (0..CELLS * CELLS)
                .map(|cell| {
                    let (lat, lon) = middle(corner, cell);
                    thinning::bar_about(lat, lon, &mut |five| {
                        fives
                            .get(&five)
                            .copied()
                            .unwrap_or(thinning::FiveBars::NONE)
                    })
                })
                .collect();
            let index = index_of(k);
            let built = (0..RING * RING)
                .map(|r| {
                    let (i, j) = ((r % RING) as i32 - 1, (r / RING) as i32 - 1);
                    let (ci, cj) = (i.div_euclid(CELLS as i32), j.div_euclid(CELLS as i32));
                    share_at([index[0] + ci, index[1] + cj]).map_or(0.0, |s| {
                        s.land[(j.rem_euclid(CELLS as i32) as usize) * CELLS
                            + i.rem_euclid(CELLS as i32) as usize]
                    })
                })
                .collect();
            let c = chunk_corner(index);
            let (lat, lon) = middle(corner, CELLS * CELLS / 2);
            let mut thinning = Thinning {
                bar,
                ..Thinning::default()
            }
            .with_towns(&towns, c, thinning::core_floor(Region::at(lat, lon, style)));
            for &(p, r) in &yards {
                thinning.add_listed([(p[0] - c[0]) as f32, (p[1] - c[1]) as f32], r, true);
            }
            (k, Arc::new(Estimated { thinning, built }))
        },
    );
    let mut chunks: Vec<Option<Arc<Estimated>>> = vec![None; size[0] * size[1]];
    for (k, mut e) in answered {
        if let Some(ring) = score.remove(&k) {
            Arc::make_mut(&mut e).thinning.score = ring;
            chunks[k] = Some(e);
        }
    }
    Estimate {
        first,
        size,
        texel_m: grid.texel_m,
        chunks,
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The scene's side.

/// What the scene on screen does with its window's marks from frame to frame: the chunks the buildings answered for
/// since they were laid, marked again in the window's cover a few a frame ([`FRAME_CHUNKS`], [`FRAME_BUDGET`]).
#[derive(Default)]
pub struct Feed {
    /// The scene it feeds (the driver's swap count).
    pub scene: u64,
    pending: VecDeque<[i32; 2]>,
    queued: HashSet<[i32; 2]>,
}

impl Feed {
    pub fn new(scene: u64) -> Self {
        Self {
            scene,
            ..Self::default()
        }
    }

    /// Chunks waiting to be marked again.
    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Marks again the chunks of `land`'s window (its maps `maps`, their chart's (0, 0) at theatre point `centre`, in
    /// the scenario about `geo`) the buildings answered for since its marks were laid. Nothing for today's land.
    pub fn feed(
        &mut self,
        land: &mut TerrainLayer,
        geo: LatLon,
        maps: &SurfaceMaps,
        centre: [f64; 2],
    ) {
        let laid = land.cover_marks();
        let period = marks_period(laid);
        let Period::Year(year) = period else {
            return;
        };
        let now = version();
        if now <= marks_version(laid) && self.pending.is_empty() {
            return;
        }
        // Only over the maps the land draws (to the f32 uniforms' precision).
        let drawn = land.surface_photo().is_some_and(|p| {
            p.texels == maps.size
                && (p.extent_m - maps.extent_m).abs() < 1.0
                && (p.origin[0] - maps.origin[0]).abs() < 1.0
                && (p.origin[1] - maps.origin[1]).abs() < 1.0
        });
        if !drawn {
            return;
        }
        let grid = Grid::of_maps(maps, centre);
        let (first, last) = grid.chunks();
        let inside = |c: &[i32; 2]| {
            (first[0]..=last[0]).contains(&c[0]) && (first[1]..=last[1]).contains(&c[1])
        };
        if now > marks_version(laid) {
            let Some(answers) = answers(geo, period) else {
                return;
            };
            let changed = thinned_since(geo, year, marks_version(laid))
                .unwrap_or_else(|| answers.thinned_chunks().collect());
            for chunk in changed.into_iter().filter(inside) {
                if self.queued.insert(chunk) {
                    self.pending.push_back(chunk);
                }
            }
            land.set_cover_marks(marks(period, answers.version()));
        }
        if self.pending.is_empty() {
            return;
        }
        let Some(answers) = answers(geo, period) else {
            self.pending.clear();
            self.queued.clear();
            return;
        };
        let started = Instant::now();
        let mut done = 0;
        while done < FRAME_CHUNKS
            && started.elapsed() < FRAME_BUDGET
            && let Some(chunk) = self.pending.pop_front()
        {
            self.queued.remove(&chunk);
            done += 1;
            let (i0, i1) = grid.span(0, chunk[0]);
            let (j0, j1) = grid.span(1, chunk[1]);
            if i0 >= i1 || j0 >= j1 {
                continue;
            }
            let (w, h) = (i1 - i0, j1 - j0);
            let side = grid.size[0];
            let mut classes = Vec::with_capacity(w * h);
            for j in j0..j1 {
                classes.extend_from_slice(&maps.cover[j * side + i0..j * side + i1]);
            }
            mark(&answers, &grid.part([i0, j0], [w, h]), &mut classes);
            land.queue_cover_patch(CoverPatch {
                layer: CoverLayer::Window,
                at: [i0 as u32, j0 as u32],
                size: [w as u32, h as u32],
                classes,
                marks: 0,
            });
        }
    }
}

#[cfg(test)]
#[path = "period_land_tests.rs"]
mod tests;
