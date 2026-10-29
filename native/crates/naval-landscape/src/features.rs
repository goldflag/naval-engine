//! Roads, railways, rivers and canals on the theatre's land (`native/docs/terrain/surface.md`, "Map features on the
//! ground"): OpenStreetMap's ways from `osm`, drawn by the land layer from distance-field pages about the camera
//! (`naval_terrain::features`, native/docs/terrain/surface.md). Presentation only, as the rest of the theatre's land.
//!
//! - **What is drawn** ([`style_of`], [`waterway_style`]): roads by class, motorway to track, at their real widths,
//!   paved or unpaved by class and `surface`; railways (not trams, which run in the street, nor anything in a tunnel);
//!   rivers, streams, canals and ditches as lines with a width by class, riverbanks and docks as areas. Footpaths are left
//!   out (sidewalks are mapped beside their road and would widen it), so are lakes and ponds (another lane's).
//! - **Pages.** Two levels of [`PAGE_TEXELS`]² pages fixed in the theatre's metres: [`TEXEL_M`] 2 m (512 m a page) and 8 m
//!   (2 km). The land layer holds [`SLOTS`]² of each about the camera (4 km and 16 km), the camera in the middle two a
//!   side. One worker thread (`theatre-features`) makes the pages the camera's block lacks, nearest first and level 0
//!   first, [`BATCH`] at a time: their OSM tiles loaded (`osm::load`, from the disk or the network), the ways under each
//!   page turned into strokes and rasterised, a few threads at a time; made pages are kept ([`KEPT`]). The frame loop
//!   hands made pages to the layer ([`Feed`]), which uploads a few a frame.
//! - **Switched** with the OSM tiles (`osm::enabled`); `NAVAL_FEATURES=off` leaves them off alone; drawn on the High terrain setting
//!   only (`install`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::LatLon;
use naval_osm::{Brunnel, Geometry, RoadSurface, Transportation, Waterway};
use naval_terrain::buildings::Period;
use naval_terrain::features::{LEVELS, PAGE_TEXELS, RANGE_TEXELS};
use naval_terrain::{FeatureGrid, FeaturePage, Stroke, StrokeKind, WaterArea};

use super::osm::{self, OsmTile};
use super::period_land;
use super::surface::FetchError;

#[path = "features_period.rs"]
pub mod period;

pub use period::{Roads, TileRoads};

/// Each level's texel, metres: crisp roads close in, the coarser level to the distance a road still shows.
pub const TEXEL_M: [f64; LEVELS] = [2.0, 8.0];
/// Pages a side each level holds about the camera.
pub const SLOTS: u32 = 8;
/// Pages the worker makes together.
pub const BATCH: usize = 8;
/// Made pages kept in memory (512 KB each): both levels' blocks and a column or two beyond.
pub const KEPT: usize = 192;
/// The zoom of the OSM tiles the pages are made from (the deepest: every road).
pub const ZOOM: u8 = osm::ZOOM;
/// The widest half width drawn (a motorway's carriageway), metres: how far past a page its ways reach.
const MAX_HALF_WIDTH_M: f64 = 8.0;
/// A page whose making failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Whether the features are drawn: with the OSM tiles on (`osm::enabled`), unless `NAVAL_FEATURES` is `off` (or `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_FEATURES").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    osm::enabled() && !off
}

static SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Shows or hides the features on the land on screen without dropping their pages (for before-and-after captures and
/// frame measurements): taken up by the next frame's feed.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------------------------------
// What each way is drawn as.

/// How a `transportation` way is drawn: its kind, half width (m) and tone (0-1, a paved road's shade: the major roads'
/// darker asphalt toward 1); none for what is not drawn (footpaths, trams, tunnels, ferries, ways being built).
pub fn style_of<P>(t: &Transportation<P>) -> Option<(StrokeKind, f32, f32)> {
    if t.brunnel == Brunnel::Tunnel {
        return None;
    }
    let (kind, half, tone) = class_style(
        &t.class,
        t.ramp,
        t.service.as_deref(),
        t.subclass.as_deref(),
    )?;
    let kind = match (kind, t.surface) {
        (StrokeKind::Paved, Some(RoadSurface::Unpaved)) => StrokeKind::Unpaved,
        (StrokeKind::Unpaved, Some(RoadSurface::Paved)) => StrokeKind::Paved,
        (k, _) => k,
    };
    let tone = if kind == StrokeKind::Paved && tone == 0.0 {
        0.4
    } else {
        tone
    };
    Some((kind, half, tone))
}

/// A trunk road's main way as [`style_of`] draws it (a past period's motorway that was an ordinary road then).
pub fn trunk_style() -> (StrokeKind, f32, f32) {
    class_style("trunk", false, None, None).unwrap_or((StrokeKind::Paved, 5.0, 0.9))
}

/// How a way of OpenMapTiles' `class` is drawn before its own surface: a ramp (`ramp`), a service road's kind
/// (`service`), a railway's (`subclass`).
fn class_style(
    class: &str,
    ramp: bool,
    service: Option<&str>,
    subclass: Option<&str>,
) -> Option<(StrokeKind, f32, f32)> {
    Some(match class {
        // A motorway's carriageways are mapped one way each: three lanes and a shoulder.
        "motorway" => (StrokeKind::Paved, if ramp { 3.5 } else { 6.5 }, 1.0),
        "trunk" => (StrokeKind::Paved, if ramp { 3.5 } else { 5.0 }, 0.9),
        "primary" => (StrokeKind::Paved, if ramp { 3.5 } else { 4.5 }, 0.75),
        "secondary" => (StrokeKind::Paved, 4.0, 0.65),
        "tertiary" => (StrokeKind::Paved, 3.5, 0.55),
        "minor" => (StrokeKind::Paved, 3.0, 0.45),
        "service" => {
            let narrow = matches!(service, Some("driveway" | "parking_aisle" | "alley"));
            (StrokeKind::Paved, if narrow { 1.8 } else { 2.5 }, 0.35)
        }
        "busway" => (StrokeKind::Paved, 3.5, 0.5),
        "raceway" => (StrokeKind::Paved, 5.0, 0.8),
        "track" => (StrokeKind::Unpaved, 1.6, 0.0),
        "rail" | "transit" => match subclass {
            Some("tram" | "funicular" | "monorail") => return None,
            _ => (StrokeKind::Rail, 0.0, 0.0),
        },
        _ => return None,
    })
}

/// A waterway's half width (m), none where it is not drawn (in a culvert, or a stream that runs only now and then).
pub fn waterway_style<P>(w: &Waterway<P>) -> Option<f32> {
    if w.brunnel == Brunnel::Tunnel {
        return None;
    }
    let half = match w.class.as_str() {
        "river" => 8.0,
        "canal" => 6.0,
        "stream" => 1.5,
        "drain" => 1.2,
        "ditch" => 0.8,
        _ => return None,
    };
    match (w.intermittent, w.class.as_str()) {
        (true, "river") => Some(4.0),
        (true, _) => None,
        _ => Some(half),
    }
}

/// Whether a water area is drawn here: riverbanks and docks (lakes and ponds are drawn as water by another lane; the
/// ocean is the sea).
pub fn water_area_drawn(class: &str, intermittent: bool) -> bool {
    matches!(class, "river" | "dock") && !intermittent
}

/// The strokes and water areas of `tiles` that reach the rectangle from `min` to `max` (theatre metres), the roads as
/// `roads`' period draws them.
pub fn shapes(
    tiles: &[Arc<OsmTile>],
    min: [f64; 2],
    max: [f64; 2],
    roads: &Roads<'_>,
) -> (Vec<Stroke>, Vec<WaterArea>) {
    let mut strokes = Vec::new();
    let mut areas = Vec::new();
    for tile in tiles {
        let ways = TileRoads::new(roads, tile, min, max);
        for t in tile.transportation_in(min, max) {
            let (Some(style), Geometry::Lines(lines)) = (roads.style(t), &t.geometry) else {
                continue;
            };
            strokes.extend(ways.lines(t, style, lines).into_iter().map(
                |((kind, half, tone), run)| Stroke {
                    kind,
                    half_width: half,
                    tone,
                    points: run.points,
                },
            ));
        }
        for w in tile.waterway_in(min, max) {
            let (Some(half), Geometry::Lines(lines)) = (waterway_style(w), &w.geometry) else {
                continue;
            };
            strokes.extend(lines.iter().map(|line| Stroke {
                kind: StrokeKind::Water,
                half_width: half,
                tone: 0.0,
                points: line.clone(),
            }));
        }
        for w in tile.water_in(min, max) {
            let Geometry::Polygons(polys) = &w.geometry else {
                continue;
            };
            if !water_area_drawn(&w.class, w.intermittent) {
                continue;
            }
            areas.extend(polys.iter().map(|p| {
                WaterArea {
                    rings: std::iter::once(p.exterior.clone())
                        .chain(p.holes.iter().cloned())
                        .collect(),
                }
            }));
        }
    }
    (strokes, areas)
}

/// A page's side at `level`, metres.
pub fn page_m(level: usize) -> f64 {
    TEXEL_M[level] * f64::from(PAGE_TEXELS)
}

/// How far past a page's edge a way still draws into it at `level`, metres.
fn reach_m(level: usize) -> f64 {
    RANGE_TEXELS * TEXEL_M[level] + MAX_HALF_WIDTH_M
}

/// Page `index` of `level`'s theatre rectangle (min, max).
pub fn page_rect(level: usize, index: [i64; 2]) -> ([f64; 2], [f64; 2]) {
    let side = page_m(level);
    let min = [index[0] as f64 * side, index[1] as f64 * side];
    (min, [min[0] + side, min[1] + side])
}

/// The page of `level` holding theatre point `p`.
pub fn page_of(level: usize, p: [f64; 2]) -> [i64; 2] {
    let side = page_m(level);
    [(p[0] / side).floor() as i64, (p[1] / side).floor() as i64]
}

/// The block of `slots`² pages of `level` about theatre point `at`, nearest first: the camera in its middle two pages a
/// side (as the near tiles' block).
pub fn block(level: usize, at: [f64; 2], slots: u32) -> Vec<[i64; 2]> {
    let side = page_m(level);
    let half = i64::from(slots / 2);
    let lo = [
        (at[0] / side + 0.5).floor() as i64 - half,
        (at[1] / side + 0.5).floor() as i64 - half,
    ];
    let mut pages: Vec<[i64; 2]> = (0..i64::from(slots))
        .flat_map(|j| (0..i64::from(slots)).map(move |i| [lo[0] + i, lo[1] + j]))
        .collect();
    let distance = |p: &[i64; 2]| {
        ((p[0] as f64 + 0.5) * side - at[0]).hypot((p[1] as f64 + 0.5) * side - at[1])
    };
    pages.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    pages
}

/// The grid on a land layer whose chart point (0, 0) is theatre point `origin`, and each level's page that its page
/// (0, 0) is: page (i, j) of the theatre is the layer's (i, j) less it.
pub fn grid(origin: [f64; 2]) -> (FeatureGrid, [[i64; 2]; LEVELS]) {
    let base: [[i64; 2]; LEVELS] = std::array::from_fn(|l| page_of(l, origin));
    let grid = FeatureGrid {
        origin: std::array::from_fn(|l| {
            [
                base[l][0] as f64 * page_m(l) - origin[0],
                base[l][1] as f64 * page_m(l) - origin[1],
            ]
        }),
        texel_m: TEXEL_M,
        slots: SLOTS,
    };
    (grid, base)
}

/// Makes page `index` of `level` from `tiles` (which cover it and its reach), its roads as `roads` draws them.
pub fn make_page(
    level: usize,
    index: [i64; 2],
    tiles: &[Arc<OsmTile>],
    roads: &Roads<'_>,
) -> FeaturePage {
    page_and_hash(level, index, tiles, roads).0
}

/// Page `index` of `level` and a hash of its strokes ([`period::strokes_hash`]: a page made again the same is the
/// one already made).
fn page_and_hash(
    level: usize,
    index: [i64; 2],
    tiles: &[Arc<OsmTile>],
    roads: &Roads<'_>,
) -> (FeaturePage, u64) {
    let (min, max) = page_rect(level, index);
    let r = reach_m(level);
    let (strokes, areas) = shapes(
        tiles,
        [min[0] - r, min[1] - r],
        [max[0] + r, max[1] + r],
        roads,
    );
    let hash = period::strokes_hash(&strokes);
    (
        FeaturePage::rasterize(min, TEXEL_M[level], &strokes, &areas),
        hash,
    )
}

/// The middle of page `index` of `level` (theatre metres).
fn page_middle(level: usize, index: [i64; 2]) -> [f64; 2] {
    let (min, max) = page_rect(level, index);
    [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// A made page's key: the scenario's frame, the period (`Period::key`), the level and the page.
type Key = ([u64; 2], u16, usize, [i64; 2]);

fn key(geo: LatLon, period: Period, level: usize, index: [i64; 2]) -> Key {
    (
        [geo.lat.to_bits(), geo.lon.to_bits()],
        period.key(),
        level,
        index,
    )
}

/// A made page: the page, `period_land::version` when it was begun (or last found unchanged under it), its strokes'
/// hash and when it was last made.
struct Made {
    page: Arc<FeaturePage>,
    version: u64,
    hash: u64,
    at: Instant,
}

/// The least time between a past period's page being made and made again: the buildings' thinning arrives a few
/// chunks a frame about the camera (`period_land::FRAME_CHUNKS`), and each would otherwise remake the pages over it.
const REMAKE_AFTER: Duration = Duration::from_secs(1);

/// The buildings' chunks (first column, first row, last column, last row) whose ground page `index` of `level` reads:
/// the page, its strokes' reach and an island's length beyond (a street's run is judged along it).
fn page_chunks(level: usize, index: [i64; 2]) -> [i32; 4] {
    let (min, max) = page_rect(level, index);
    let r = reach_m(level) + period::ISLAND_M;
    let a = naval_terrain::buildings::chunk_of([min[0] - r, min[1] - r]);
    let b = naval_terrain::buildings::chunk_of([max[0] + r, max[1] + r]);
    [a[0], a[1], b[0], b[1]]
}

impl Made {
    /// Whether it is the period's page `index` of `level` about `geo` as the ground now stands (today's always is).
    fn fresh(&mut self, geo: LatLon, period: Period, level: usize, index: [i64; 2]) -> bool {
        let elapsed = self.at.elapsed();
        self.fresh_with(period, period_land::version(), elapsed, |since| {
            period_land::changed_since(geo, period, since, page_chunks(level, index), true)
        })
    }

    /// Whether it is fresh with the ground at `version`, `elapsed` after it was made, `changed(since)` telling whether
    /// the ground under it changed after `since`: made at the version, or nothing changed under it since (its version
    /// then brought up to `version`), or made again too lately to be made again yet ([`REMAKE_AFTER`]).
    fn fresh_with(
        &mut self,
        period: Period,
        version: u64,
        elapsed: Duration,
        changed: impl FnOnce(u64) -> bool,
    ) -> bool {
        if !period.is_past() || self.version >= version || elapsed < REMAKE_AFTER {
            return true;
        }
        if changed(self.version) {
            return false;
        }
        self.version = version;
        true
    }
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    pub queued: usize,
    pub working: usize,
    /// Pages made this session, and how long the last batch took to rasterise (ms).
    pub made: usize,
    pub last_batch_ms: u64,
    pub failed: usize,
    /// Pages made again for a newer ground (`period_land::version`), and of those the ones that came out the same.
    pub remade: usize,
    pub unchanged: usize,
}

struct State {
    geo: Option<LatLon>,
    period: Period,
    wanted: VecDeque<(usize, [i64; 2])>,
    made: HashMap<Key, Made>,
    order: VecDeque<Key>,
    failed: HashMap<Key, Instant>,
    status: WorkerStatus,
    worker: bool,
}

struct Streamer {
    state: Mutex<State>,
    wake: Condvar,
}

fn streamer() -> &'static Streamer {
    static STREAMER: OnceLock<Streamer> = OnceLock::new();
    STREAMER.get_or_init(|| Streamer {
        state: Mutex::new(State {
            geo: None,
            period: Period::Today,
            wanted: VecDeque::new(),
            made: HashMap::new(),
            order: VecDeque::new(),
            failed: HashMap::new(),
            status: WorkerStatus::default(),
            worker: false,
        }),
        wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    streamer().state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Page `index` of `level` about `geo` in `period`, if made: the page, whether it is fresh (made from the ground as it
/// now stands; a stale page is drawn until it is made again) and its strokes' hash (what tells it from the page made
/// before it).
pub fn made(
    geo: LatLon,
    period: Period,
    level: usize,
    index: [i64; 2],
) -> Option<(Arc<FeaturePage>, bool, u64)> {
    lock()
        .made
        .get_mut(&key(geo, period, level, index))
        .map(|m| (m.page.clone(), m.fresh(geo, period, level, index), m.hash))
}

/// Asks for `pages` (level and index, in the order given) about `geo` in `period`, in place of whatever was asked
/// before; a page made fresh, or failed lately, is left out.
pub fn want(geo: LatLon, period: Period, pages: &[(usize, [i64; 2])]) {
    if !enabled() {
        return;
    }
    let mut state = lock();
    let now = Instant::now();
    state
        .failed
        .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    let mut wanted = VecDeque::new();
    for &(l, i) in pages {
        let k = key(geo, period, l, i);
        let fresh = state
            .made
            .get_mut(&k)
            .is_some_and(|m| m.fresh(geo, period, l, i));
        if !fresh && !state.failed.contains_key(&k) {
            wanted.push_back((l, i));
        }
    }
    state.geo = Some(geo);
    state.period = period;
    state.status.queued = wanted.len();
    state.wanted = wanted;
    if state.wanted.is_empty() {
        return;
    }
    if !state.worker {
        state.worker = true;
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-features", work);
        if let Err(e) = spawned {
            state.worker = false;
            eprintln!("the theatre's map features: no worker thread: {e}");
        }
    }
    streamer().wake.notify_one();
}

pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        failed: state.failed.len(),
        ..state.status
    }
}

fn work() {
    loop {
        let (geo, period, batch) = {
            let mut state = lock();
            loop {
                if let Some(geo) = state.geo {
                    let period = state.period;
                    let mut batch = Vec::with_capacity(BATCH);
                    while batch.len() < BATCH
                        && let Some((l, i)) = state.wanted.pop_front()
                    {
                        let fresh = state
                            .made
                            .get_mut(&key(geo, period, l, i))
                            .is_some_and(|m| m.fresh(geo, period, l, i));
                        if !fresh {
                            batch.push((l, i));
                        }
                    }
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (geo, period, batch);
                    }
                }
                state.status.working = 0;
                state = streamer()
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        // The ground's version before the pages read it: one that rises while they are made leaves them stale.
        let version = period_land::version();
        let result = make_batch(geo, period, &batch);
        let mut state = lock();
        state.status.working = 0;
        match result {
            Ok((pages, ms)) => {
                state.status.last_batch_ms = ms;
                for ((l, i), (page, hash)) in pages {
                    let k = key(geo, period, l, i);
                    state.status.made += 1;
                    let made = Made {
                        page,
                        version,
                        hash,
                        at: Instant::now(),
                    };
                    match state.made.get_mut(&k) {
                        // Made again the same: the page drawn stays (no upload).
                        Some(old) if old.hash == hash => {
                            old.version = version;
                            old.at = made.at;
                            state.status.remade += 1;
                            state.status.unchanged += 1;
                        }
                        Some(old) => {
                            *old = made;
                            state.status.remade += 1;
                        }
                        None => {
                            state.made.insert(k, made);
                            state.order.push_back(k);
                        }
                    }
                }
                while state.order.len() > KEPT {
                    if let Some(old) = state.order.pop_front() {
                        state.made.remove(&old);
                    }
                }
            }
            Err(_) => {
                // (`osm::load` logged it.) The batch waits a minute; what was asked after it would fail the same way.
                let now = Instant::now();
                for &(l, i) in &batch {
                    state.failed.insert(key(geo, period, l, i), now);
                }
                state.wanted.clear();
                state.status.queued = 0;
            }
        }
    }
}

/// Loads the batch's tiles and makes its pages in `period` (their streets where `period_land::stood` says the ground
/// stood), four threads at a time. The pages with their strokes' hashes, and the milliseconds rasterising took.
#[allow(clippy::type_complexity)]
fn make_batch(
    geo: LatLon,
    period: Period,
    batch: &[(usize, [i64; 2])],
) -> Result<(Vec<((usize, [i64; 2]), (Arc<FeaturePage>, u64))>, u64), FetchError> {
    let mut tiles_of: Vec<Vec<naval_osm::TileId>> = Vec::with_capacity(batch.len());
    let mut all = HashSet::new();
    for &(l, i) in batch {
        let (min, max) = page_rect(l, i);
        let r = reach_m(l);
        let ids = osm::tiles_over(
            geo,
            [min[0] - r, min[1] - r],
            [max[0] + r, max[1] + r],
            ZOOM,
        );
        all.extend(ids.iter().copied());
        tiles_of.push(ids);
    }
    let all: Vec<_> = all.into_iter().collect();
    let area = osm::load(geo, &all)?;
    let by_id: HashMap<naval_osm::TileId, Arc<OsmTile>> =
        area.tiles.into_iter().map(|t| (t.id, t)).collect();
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<(Arc<FeaturePage>, u64)>>> = Mutex::new(vec![None; batch.len()]);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 4)
        .min(batch.len());
    let stood = move |p: [f64; 2]| period_land::stood(geo, period, p);
    // At the worker's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                qos.apply();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(l, i)) = batch.get(k) else {
                        break;
                    };
                    let tiles: Vec<Arc<OsmTile>> = tiles_of[k]
                        .iter()
                        .filter_map(|id| by_id.get(id).cloned())
                        .collect();
                    let roads = Roads::new(period, geo, page_middle(l, i), &stood);
                    let (mut page, mut hash) = page_and_hash(l, i, &tiles, &roads);
                    if l == naval_terrain::night::LAMP_LEVEL {
                        page.lamps = super::lamps::page_lamps(geo, i, &tiles, &roads);
                        hash = period::lamps_hash(hash, &page.lamps);
                    }
                    let page = Arc::new(page);
                    results.lock().unwrap_or_else(|e| e.into_inner())[k] = Some((page, hash));
                }
            });
        }
    });
    let ms = started.elapsed().as_millis() as u64;
    let pages = batch
        .iter()
        .copied()
        .zip(results.into_inner().unwrap_or_else(|e| e.into_inner()))
        .filter_map(|(b, p)| Some((b, p?)))
        .collect();
    Ok((pages, ms))
}

// ---------------------------------------------------------------------------------------------------------------------
// The layer's side.

/// Where the camera stood at the last frame a scene fed its pages (the scenario's frame, the theatre point).
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 2])>> = Mutex::new(None);

/// Gives a land layer being built (off the frame loop's thread, with its scene) the features' grid (its chart's (0, 0)
/// at theatre point `origin`) and uploads the made pages of `period` (the buildings', Graphics → Buildings) about where
/// the camera last stood, so a scene rebuilt where the old one stood draws them from its first frame. Nothing with the
/// features off, nor on the Medium terrain setting (their cost, 0.0-0.3 ms a frame measured, is the High setting's:
/// native/docs/terrain/surface.md, "Map features on the ground").
pub fn install(
    ctx: &mut naval_render::RenderContext<'_>,
    land: &mut naval_terrain::TerrainLayer,
    geo: LatLon,
    origin: [f64; 2],
    period: Period,
) {
    if !enabled() || !land.settings().quality.detailed {
        return;
    }
    let (grid, base) = grid(origin);
    if let Err(e) = land.set_feature_grid(ctx, grid) {
        eprintln!("the theatre's map features: {e}");
        return;
    }
    land.set_night_origin(origin);
    let last = *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    let frame = [geo.lat.to_bits(), geo.lon.to_bits()];
    let Some((_, at)) = last.filter(|(f, _)| *f == frame) else {
        return;
    };
    for (level, base) in base.iter().enumerate() {
        for index in block(level, at, grid.slots) {
            if let Some((page, ..)) = made(geo, period, level, index) {
                land.queue_feature_page(level, local(index, *base), page);
            }
        }
    }
    land.flush_feature_pages(ctx);
}

fn local(index: [i64; 2], base: [i64; 2]) -> [i32; 2] {
    [(index[0] - base[0]) as i32, (index[1] - base[1]) as i32]
}

/// What a scene on screen does with its pages, kept by the driver from frame to frame.
#[derive(Default)]
pub struct Feed {
    /// The scene it feeds (the driver's swap count when it was made).
    pub scene: u64,
    /// The period its pages are drawn in (its land's, `period_land::marks_period`).
    pub period: Period,
    blocks: [Vec<[i64; 2]>; LEVELS],
    centres: [Option<[i64; 2]>; LEVELS],
    asked: Vec<(usize, [i64; 2])>,
    asked_at: Option<Instant>,
    /// A past period's pages handed to the layer (level and page: the made page's strokes' hash), so a page made again
    /// changed is handed again.
    handed: HashMap<(usize, [i64; 2]), u64>,
    /// The blocks' pages not yet made (or made from an older ground).
    pub pending: usize,
}

impl Feed {
    /// The feed of scene `scene`.
    pub fn new(scene: u64) -> Self {
        Self {
            scene,
            ..Self::default()
        }
    }

    /// Hands `land` the made pages of the blocks about its camera and asks the worker for the rest (level 0's first);
    /// in a past period, a held page made again changed (the ground's version having risen) is handed again.
    /// `origin`: the theatre point of the layer's chart (0, 0).
    pub fn feed(&mut self, land: &mut naval_terrain::TerrainLayer, geo: LatLon, origin: [f64; 2]) {
        land.show_features(SHOWN.load(Ordering::Relaxed));
        let Some(grid) = land.feature_grid() else {
            return;
        };
        let Some(camera) = land.camera_chart() else {
            return;
        };
        // The period the land was built in (its cover's marks carry it).
        let period = period_land::marks_period(land.cover_marks());
        if period != self.period {
            self.period = period;
            self.handed.clear();
        }
        let past = period.is_past();
        let at = [camera[0] + origin[0], camera[1] + origin[1]];
        *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(([geo.lat.to_bits(), geo.lon.to_bits()], at));
        let mut missing = Vec::new();
        let mut moved = false;
        for level in 0..LEVELS {
            let side = page_m(level);
            let centre = [
                (at[0] / side + 0.5).floor() as i64,
                (at[1] / side + 0.5).floor() as i64,
            ];
            if self.centres[level] != Some(centre) {
                self.centres[level] = Some(centre);
                self.blocks[level] = block(level, at, grid.slots);
                moved = true;
            }
            let base = page_of(level, origin);
            for &index in &self.blocks[level] {
                let l = local(index, base);
                let held = land.holds_feature_page(level, l);
                if held && !past {
                    continue;
                }
                match made(geo, period, level, index) {
                    Some((page, fresh, hash)) => {
                        if !fresh {
                            missing.push((level, index));
                        }
                        // A page held that this feed never handed (installed with its scene, or before the block
                        // moved) is handed once again: what it holds is not known.
                        if past && held && self.handed.get(&(level, index)) == Some(&hash) {
                            continue;
                        }
                        if past {
                            self.handed.insert((level, index), hash);
                        }
                        land.queue_feature_page(level, l, page);
                    }
                    None if held => {}
                    None => missing.push((level, index)),
                }
            }
        }
        if moved && past {
            let blocks = &self.blocks;
            self.handed
                .retain(|(level, index), _| blocks[*level].contains(index));
        }
        self.pending = missing.len();
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if missing != self.asked || (!missing.is_empty() && stale) {
            want(geo, period, &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
    }
}

/// The features' state for the RPC (`theatre_state.features`): the grid's pages, those drawn, waiting for their upload
/// and still to be made, the worker's and the OSM tiles' state; none with no grid on screen.
pub fn status_json(
    land: Option<&naval_terrain::TerrainLayer>,
    feed: &Feed,
    fresh: bool,
) -> Option<serde_json::Value> {
    let land = land?;
    let grid = land.feature_grid()?;
    let (drawn, uploading) = land.feature_page_counts();
    let worker = status();
    let tiles = osm::status();
    let pages = (grid.slots * grid.slots) as usize * LEVELS;
    Some(serde_json::json!({
        "pages": pages,
        "texelM": TEXEL_M,
        "drawn": drawn,
        "uploading": uploading,
        "pending": if fresh { feed.pending } else { pages },
        "period": feed.period.key(),
        "worker": {
            "queued": worker.queued,
            "working": worker.working,
            "made": worker.made,
            "lastBatchMs": worker.last_batch_ms,
            "failed": worker.failed,
            "remade": worker.remade,
            "unchanged": worker.unchanged,
        },
        "osm": {
            "kept": tiles.kept,
            "keptBytes": tiles.kept_bytes,
            "fetched": tiles.fetched,
            "failed": tiles.failed,
        },
    }))
}

#[cfg(test)]
#[path = "features_tests.rs"]
mod tests;
