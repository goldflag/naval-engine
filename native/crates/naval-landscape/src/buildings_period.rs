//! A past period's thinning, the host's half (native/docs/terrain/buildings.md, "Period"): what
//! `naval_terrain::buildings::thinning` decides by, gathered from the footprints' stores before a batch's chunks are
//! meshed. A chunk's scores read Microsoft's footprints of it and its eight neighbours ([`Built`], kept per chunk); each
//! of its 100 m cells' bars blends the bars of the four 5' cells about it, each from that cell's footprints' scores
//! ([`Distribution`], kept per cell) and its share of the built-up history.
//!
//! A big city's footprints come from its big file's level-13 tiles, each a few kilometres: a 5' cell's distribution
//! reads every tile under it, and is kept only once all of them are had. Until a big file under a chunk's cells is
//! split the chunk is made as waiting for it (`Done::awaiting`) and made again when it is.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_terrain::buildings::thinning::{
    self, Built, CELLS, Distribution, LATER, TALL, Thinning, hyde_middle,
};
use naval_terrain::buildings::towns::Towns;
use naval_terrain::buildings::{CHUNK_M, Footprint, Region, chunk_corner, infill};

use super::{
    Context, FetchError, Index, Land, WHOLE_FILE_BYTES, quad, quadkeys_of, quadkeys_over,
    request_split, split, surface, tile_store, tiles_path,
};

/// Chunks' footprints by cell kept (about 800 bytes each), and 5' cells' distributions (4 kB each).
const BUILT_KEPT: usize = 8192;
const SUMMARIES_KEPT: usize = 256;
/// The most chunks to a side a 5' cell's distribution reads (a cell is 9.3 km across at most).
const MOST_CHUNKS: i32 = 16;

/// A frame's (its origin's bits) chunk or 5' cell.
type Id = ([u64; 2], [i32; 2]);

/// The listed towns standing in a year about a theatre (the period's ground's, which reads them too).
pub use super::super::period_land::{TOWNS_REACH_M, towns};

#[derive(Default)]
struct Kept {
    built: HashMap<Id, Option<Arc<Built>>>,
    built_order: VecDeque<Id>,
    /// By frame, cell and year (the listed towns standing in it).
    summaries: HashMap<(Id, u16), Arc<Distribution>>,
    summary_order: VecDeque<(Id, u16)>,
}

fn kept() -> MutexGuard<'static, Kept> {
    static KEPT: OnceLock<Mutex<Kept>> = OnceLock::new();
    KEPT.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// What gathering lacked: a big file still to be split, or a file not had (offline, or a server's error).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lacking {
    pub awaiting: bool,
    pub missing: bool,
}

impl Lacking {
    fn any(self) -> bool {
        self.awaiting || self.missing
    }

    fn add(&mut self, other: Self) {
        self.awaiting |= other.awaiting;
        self.missing |= other.missing;
    }
}

/// A chunk's thinning, whether it is to be raised by the chunk's own footprints (none of Microsoft's about it:
/// `Thinning::with_own`), and what gathering it lacked.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub thinning: Thinning,
    pub own: bool,
    pub lacking: Lacking,
}

impl Prepared {
    /// The thinning for the chunk drawn with `footprints` (its own and the infill's lots).
    pub fn thinning<'a>(
        &'a self,
        index: [i32; 2],
        footprints: impl IntoIterator<Item = &'a Footprint>,
    ) -> Thinning {
        if self.own {
            self.thinning
                .clone()
                .with_own(index, &Built::of(footprints))
        } else {
            self.thinning.clone()
        }
    }
}

/// One batch's gathering: what it found (had in full or not), the files it could not have (asked for once a batch),
/// and the files it fetched.
struct Gather<'a> {
    context: &'a Context,
    year: u16,
    frame: [u64; 2],
    built: HashMap<[i32; 2], (Option<Arc<Built>>, Lacking)>,
    cover_built: HashMap<[i32; 2], Option<Arc<Built>>>,
    summaries: HashMap<[i32; 2], (Arc<Distribution>, Lacking)>,
    towns: Towns,
    failed: HashSet<String>,
    index_failed: bool,
    fetched: usize,
}

/// A failed fetch: offline, the footprints' fetching waits a while (as `make`'s does).
fn note_failure(e: &FetchError) {
    if matches!(e, FetchError::Offline(_)) {
        surface::mark_offline();
    }
}

impl<'a> Gather<'a> {
    fn new(context: &'a Context, year: u16) -> Self {
        Self {
            context,
            year,
            frame: [context.geo.lat.to_bits(), context.geo.lon.to_bits()],
            built: HashMap::new(),
            cover_built: HashMap::new(),
            summaries: HashMap::new(),
            towns: towns(context.geo, year),
            failed: HashSet::new(),
            index_failed: false,
            fetched: 0,
        }
    }

    /// The footprints' index, unless it could not be had already this batch.
    fn index(&mut self) -> Option<Arc<Index>> {
        if self.index_failed {
            return None;
        }
        super::index()
            .inspect_err(|e| {
                note_failure(e);
                self.index_failed = true;
            })
            .ok()
    }

    /// Chunk `index`'s footprints by cell from its stores (as `make`'s: its whole files', then its big files' tiles).
    fn built(&mut self, index: [i32; 2]) -> (Option<Arc<Built>>, Lacking) {
        if let Some(found) = self.built.get(&index) {
            return found.clone();
        }
        if let Some(found) = kept().built.get(&(self.frame, index)).cloned() {
            self.built
                .insert(index, (found.clone(), Lacking::default()));
            return (found, Lacking::default());
        }
        let geo = self.context.geo;
        let mut lacking = Lacking::default();
        let mut built = Built::default();
        let keys = quadkeys_of(geo, index);
        for key in &keys {
            if self.failed.contains(key) {
                lacking.missing = true;
                continue;
            }
            match quad(geo, key, &mut self.fetched) {
                Ok(store) => {
                    for f in store
                        .chunks
                        .get(&index)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                    {
                        built.add(f);
                    }
                }
                Err(e) => {
                    note_failure(&e);
                    self.failed.insert(key.clone());
                    lacking.missing = true;
                }
            }
        }
        match self.index() {
            Some(files) => {
                for key in &keys {
                    for file in files.files.get(key).map(Vec::as_slice).unwrap_or_default() {
                        if file.bytes <= WHOLE_FILE_BYTES {
                            continue;
                        }
                        let dir = tiles_path(file, key);
                        if !dir.is_dir() {
                            request_split(file, key);
                            lacking.awaiting = true;
                            continue;
                        }
                        let name = format!("{}/{key}", file.location);
                        for tile in quadkeys_over(geo, index, split::SPLIT_LEVEL) {
                            match tile_store(geo, &dir, &name, &tile) {
                                Some(store) => {
                                    for f in store
                                        .chunks
                                        .get(&index)
                                        .map(Vec::as_slice)
                                        .unwrap_or_default()
                                    {
                                        built.add(f);
                                    }
                                }
                                None => {
                                    request_split(file, key);
                                    lacking.awaiting = true;
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            None => lacking.missing = true,
        }
        let found = (built.footprints() > 0).then(|| Arc::new(built));
        if !lacking.any() {
            let mut kept = kept();
            let id = (self.frame, index);
            if kept.built.insert(id, found.clone()).is_none() {
                kept.built_order.push_back(id);
            }
            while kept.built_order.len() > BUILT_KEPT {
                if let Some(old) = kept.built_order.pop_front() {
                    kept.built.remove(&old);
                }
            }
        }
        self.built.insert(index, (found.clone(), lacking));
        (found, lacking)
    }

    /// Chunk `index`'s infill lots on all its built-up cover by cell (none left to a footprint): what a place without
    /// Microsoft's footprints is scored by, the same from any chunk and level of detail. Kept for the batch only: the
    /// window's maps change under a frame.
    fn cover_built(&mut self, index: [i32; 2]) -> Option<Arc<Built>> {
        if let Some(found) = self.cover_built.get(&index) {
            return found.clone();
        }
        let context = self.context;
        let land = Land {
            ground: &context.ground,
            cover: context.cover.as_ref(),
        };
        let lots = infill::lots(index, &land, &[false; 100], &|p| context.region_at(p));
        let found = (!lots.is_empty()).then(|| Arc::new(Built::of(&lots)));
        self.cover_built.insert(index, found.clone());
        found
    }

    /// [`Self::cover_built`] of chunk `index` and its eight neighbours, row by row.
    fn cover_around(&mut self, index: [i32; 2]) -> Vec<Option<Arc<Built>>> {
        (0..9i32)
            .map(|k| self.cover_built([index[0] - 1 + k % 3, index[1] - 1 + k / 3]))
            .collect()
    }

    /// 5' cell `cell`'s footprints by their cells' scores: every chunk its sides reach, scored with a chunk about them.
    fn summary(&mut self, cell: [i32; 2]) -> (Arc<Distribution>, Lacking) {
        if let Some(found) = self.summaries.get(&cell) {
            return found.clone();
        }
        if let Some(found) = kept()
            .summaries
            .get(&((self.frame, cell), self.year))
            .cloned()
        {
            self.summaries
                .insert(cell, (found.clone(), Lacking::default()));
            return (found, Lacking::default());
        }
        let geo = self.context.geo;
        let step = 1.0 / thinning::HYDE_PER_DEGREE;
        let (lat0, lon0) = (f64::from(cell[1]) * step, f64::from(cell[0]) * step);
        let mut lo = [f64::INFINITY; 2];
        let mut hi = [f64::NEG_INFINITY; 2];
        for (fy, fx) in [
            (0.0, 0.0),
            (0.0, 0.5),
            (0.0, 1.0),
            (0.5, 0.0),
            (0.5, 1.0),
            (1.0, 0.0),
            (1.0, 0.5),
            (1.0, 1.0),
        ] {
            let p = to_local(
                geo,
                LatLon {
                    lat: lat0 + fy * step,
                    lon: lon0 + fx * step,
                },
            );
            for a in 0..2 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let first = [
            (lo[0] / CHUNK_M).floor() as i32,
            (lo[1] / CHUNK_M).floor() as i32,
        ];
        let last = [
            (hi[0] / CHUNK_M).floor() as i32,
            (hi[1] / CHUNK_M).floor() as i32,
        ];
        let mut distribution = Distribution::default();
        let mut lacking = Lacking::default();
        if last[0] - first[0] < MOST_CHUNKS && last[1] - first[1] < MOST_CHUNKS {
            // The cell's chunks and a ring about them, row by row.
            let from = [first[0] - 1, first[1] - 1];
            let size = [
                (last[0] - first[0] + 3) as usize,
                (last[1] - first[1] + 3) as usize,
            ];
            let mut builts = Vec::with_capacity(size[0] * size[1]);
            for j in 0..size[1] as i32 {
                for i in 0..size[0] as i32 {
                    let (b, l) = self.built([from[0] + i, from[1] + j]);
                    lacking.add(l);
                    builts.push(b);
                }
            }
            let refs: Vec<Option<&Built>> = builts.iter().map(Option::as_deref).collect();
            let scores = thinning::window_scores(from, size, &refs);
            let w = size[0] * CELLS;
            for cj in 1..size[1] - 1 {
                for ci in 1..size[0] - 1 {
                    let Some(b) = refs[cj * size[0] + ci] else {
                        continue;
                    };
                    let corner = chunk_corner([from[0] + ci as i32, from[1] + cj as i32]);
                    let towns = self.towns.about(corner);
                    for k in 0..CELLS * CELLS {
                        if b.count[k] == 0 {
                            continue;
                        }
                        let m = thinning::cell_middle(k);
                        let at = to_lat_lon(
                            geo,
                            [corner[0] + f64::from(m[0]), corner[1] + f64::from(m[1])],
                        );
                        if thinning::hyde_cell(at.lat, at.lon) != cell {
                            continue;
                        }
                        let (x, y) = (ci * CELLS + k % CELLS, cj * CELLS + k / CELLS);
                        let middle = [corner[0] + f64::from(m[0]), corner[1] + f64::from(m[1])];
                        if towns.holds(middle) {
                            distribution.add_town(scores[y * w + x], b.count[k]);
                        } else {
                            distribution.add(scores[y * w + x], b.count[k]);
                        }
                    }
                }
            }
        }
        let distribution = Arc::new(distribution);
        if !lacking.any() {
            let mut kept = kept();
            let id = ((self.frame, cell), self.year);
            if kept.summaries.insert(id, distribution.clone()).is_none() {
                kept.summary_order.push_back(id);
            }
            while kept.summary_order.len() > SUMMARIES_KEPT {
                if let Some(old) = kept.summary_order.pop_front() {
                    kept.summaries.remove(&old);
                }
            }
        }
        self.summaries.insert(cell, (distribution.clone(), lacking));
        (distribution, lacking)
    }

    /// Chunk `index`'s thinning.
    fn prepare(&mut self, index: [i32; 2]) -> Prepared {
        let context = self.context;
        let geo = context.geo;
        let mut lacking = Lacking::default();
        let mut around: Vec<Option<Arc<Built>>> = Vec::with_capacity(9);
        for k in 0..9i32 {
            let (b, l) = self.built([index[0] - 1 + k % 3, index[1] - 1 + k / 3]);
            lacking.add(l);
            around.push(b);
        }
        let mut own = around.iter().all(Option::is_none);
        if own && context.cover.is_some() {
            // None of Microsoft's footprints about: the chunk and its neighbours scored by the infill the cover would
            // raise on them, so neighbouring chunks agree along their edges. Without the cover, the chunk's own drawn
            // footprints (`Thinning::with_own`).
            around = self.cover_around(index);
            own = false;
        }
        let refs: [Option<&Built>; 9] = std::array::from_fn(|k| around[k].as_deref());
        let window = thinning::chunk_window(index, &refs);
        // Each 5' cell's bar (its region's floor and share), once per chunk.
        let mut bars: HashMap<[i32; 2], thinning::FiveBars> = HashMap::new();
        let corner = chunk_corner(index);
        let mut bar = Vec::with_capacity(CELLS * CELLS);
        for k in 0..CELLS * CELLS {
            let m = thinning::cell_middle(k);
            let at = to_lat_lon(
                geo,
                [corner[0] + f64::from(m[0]), corner[1] + f64::from(m[1])],
            );
            bar.push(thinning::bar_about(at.lat, at.lon, &mut |cell| {
                if let Some(found) = bars.get(&cell) {
                    return *found;
                }
                let (distribution, l) = self.summary(cell);
                lacking.add(l);
                let (lat, lon) = hyde_middle(cell);
                let region = Region::at(lat, lon, &context.style);
                let found = thinning::cell_bars(cell, Some(&distribution), self.year, region);
                bars.insert(cell, found);
                found
            }));
        }
        let middle = to_lat_lon(geo, [corner[0] + CHUNK_M * 0.5, corner[1] + CHUNK_M * 0.5]);
        let region = context.region_of(index);
        let cap = thinning::height_cap(region, middle.lat, middle.lon);
        let mut thinning = Thinning::new(&window, bar, cap).with_towns(
            &self.towns,
            corner,
            thinning::core_floor(region),
        );
        let local = |lat: f64, lon: f64| {
            let p = to_local(geo, LatLon { lat, lon });
            [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32]
        };
        let near = |p: [f32; 2]| {
            (-100.0..CHUNK_M as f32 + 100.0).contains(&p[0])
                && (-100.0..CHUNK_M as f32 + 100.0).contains(&p[1])
        };
        for &(lat, lon, height) in TALL {
            let p = local(lat, lon);
            if near(p) {
                thinning.tall.push((p, height));
            }
        }
        for &(lat, lon, from) in LATER {
            let p = local(lat, lon);
            if self.year < from && near(p) {
                thinning.later.push(p);
            }
        }
        // The listed yards and later land about the chunk, and whether its port's town stood (its working waterfront
        // whole): by the history's share, or a listed town about it.
        let mut thinning = thinning.with_listed(self.year, &local);
        thinning.port = !thinning.town.is_empty()
            || thinning::port_stood(middle.lat, middle.lon, self.year, &|lat, lon| {
                Region::at(lat, lon, &context.style)
            });
        Prepared {
            thinning,
            own,
            lacking,
        }
    }
}

/// For calibrating (naval-client's `report_the_period_about_a_place`): over the chunks `reach` about chunk (0, 0), each
/// 5' cell's footprints, its history's share and the share kept, its bar and its scores' deciles; and the share of
/// Microsoft's footprints kept per chunk row (by their cells' middles, unlifted).
pub fn report(context: &Context, reach: i32) -> String {
    use std::fmt::Write as _;
    let naval_terrain::buildings::Period::Year(year) = context.period else {
        return "today".into();
    };
    let mut gather = Gather::new(context, year);
    let mut out = String::new();
    let (mut all, mut kept) = (0u32, 0u32);
    let mut cells: Vec<[i32; 2]> = Vec::new();
    for j in -reach..=reach {
        let mut row = String::new();
        for i in -reach..=reach {
            let prepared = gather.prepare([i, j]);
            let (built, _) = gather.built([i, j]);
            let (mut n, mut k) = (0u32, 0u32);
            if let Some(b) = built {
                for c in 0..CELLS * CELLS {
                    n += b.count[c];
                    if prepared.thinning.keeps(thinning::cell_middle(c), 0.0) {
                        k += b.count[c];
                    }
                }
            }
            all += n;
            kept += k;
            let _ = write!(
                row,
                "{:>4}",
                if n == 0 {
                    "  .".into()
                } else {
                    format!("{:3.0}", 100.0 * k as f32 / n as f32)
                }
            );
            let corner = chunk_corner([i, j]);
            let at = to_lat_lon(context.geo, [corner[0] + 500.0, corner[1] + 500.0]);
            let cell = thinning::hyde_cell(at.lat, at.lon);
            if !cells.contains(&cell) {
                cells.push(cell);
            }
        }
        let _ = writeln!(out, "{row}");
    }
    let _ = writeln!(
        out,
        "kept {kept} of {all} ({:.0}%)",
        100.0 * kept as f32 / all.max(1) as f32
    );
    for cell in cells {
        let (d, _) = gather.summary(cell);
        let (lat, lon) = hyde_middle(cell);
        let region = Region::at(lat, lon, &context.style);
        let (raw, around) = thinning::history_about(cell, year);
        let share = thinning::period_share(raw, around, region, year);
        let deciles: Vec<String> = [0.9f32, 0.75, 0.5, 0.25, 0.1]
            .iter()
            .map(|q| format!("{:.2}", d.bar(*q)))
            .collect();
        let _ = writeln!(
            out,
            "5' cell {cell:?} ({lat:.3}, {lon:.3}) {region:?}: {} footprints ({} in towns, {} of them kept), history \
             {raw:?} -> {share:.2}, bar {:.2}, scores at 90/75/50/25/10% kept {}",
            d.footprints(),
            d.town,
            d.town_kept(thinning::core_floor(region)),
            thinning::bar(Some(&d), share, region),
            deciles.join(" ")
        );
    }
    out
}

/// For calibrating (naval-client's `report_old_cities`): each 100 m cell holding Microsoft's footprints in chunks
/// `from` to `to` (first and last column and row): its middle, its footprints and whether it stands (unlifted, at its
/// middle); with the listed towns, or as if none were listed (`towns` false; the 5' cells' kept distributions are let
/// go first).
pub fn kept_cells(
    context: &Context,
    from: [i32; 2],
    to: [i32; 2],
    towns: bool,
) -> Vec<(LatLon, u32, bool)> {
    let naval_terrain::buildings::Period::Year(year) = context.period else {
        return Vec::new();
    };
    {
        let mut kept = kept();
        kept.summaries.clear();
        kept.summary_order.clear();
    }
    let mut gather = Gather::new(context, year);
    if !towns {
        gather.towns = Towns::default();
    }
    let mut out = Vec::new();
    for j in from[1]..=to[1] {
        for i in from[0]..=to[0] {
            let (Some(built), _) = gather.built([i, j]) else {
                continue;
            };
            let prepared = gather.prepare([i, j]);
            let corner = chunk_corner([i, j]);
            for c in 0..CELLS * CELLS {
                if built.count[c] == 0 {
                    continue;
                }
                let m = thinning::cell_middle(c);
                let at = to_lat_lon(
                    context.geo,
                    [corner[0] + f64::from(m[0]), corner[1] + f64::from(m[1])],
                );
                out.push((at, built.count[c], prepared.thinning.keeps(m, 0.0)));
            }
        }
    }
    out
}

/// For tests: chunk `index`'s thinning (no bar, no cap) where none of Microsoft's footprints lie about it, scored by
/// the infill the cover raises on it and its neighbours.
#[cfg(test)]
pub fn by_cover(context: &Context, index: [i32; 2]) -> Thinning {
    let mut gather = Gather::new(context, 1942);
    let around = gather.cover_around(index);
    let refs: [Option<&Built>; 9] = std::array::from_fn(|k| around[k].as_deref());
    let window = thinning::chunk_window(index, &refs);
    Thinning::new(&window, vec![0.0; CELLS * CELLS], 1000.0)
}

/// The thinning of each of `batch`'s chunks with land (`land`) in a past period (none today, or at sea), and the files
/// fetched for it.
pub fn prepare(
    context: &Context,
    batch: &[naval_terrain::buildings::ChunkKey],
    land: &[bool],
) -> (Vec<Option<Prepared>>, usize) {
    let naval_terrain::buildings::Period::Year(year) = context.period else {
        return (vec![None; batch.len()], 0);
    };
    let mut gather = Gather::new(context, year);
    let prepared: Vec<Option<Prepared>> = batch
        .iter()
        .zip(land)
        .map(|(chunk, land)| land.then(|| gather.prepare(chunk.index)))
        .collect();
    (prepared, gather.fetched)
}

/// Hands the period's ground (`period_land`) the batch's answers once its chunks are meshed: each chunk gathered in
/// full and thinned by Microsoft's footprints or the cover's infill about it (not by its own drawn footprints, which
/// differ between its levels of detail), with the land its buildings kept whole (`ChunkMesh::kept`: a listed yard, a
/// working waterfront, the ground about what a land's lift kept), so the ground answers as the buildings stand.
pub fn answer(
    context: &Context,
    batch: &[naval_terrain::buildings::ChunkKey],
    prepared: &[Option<Prepared>],
    kept: &[Option<thinning::Kept>],
) {
    let naval_terrain::buildings::Period::Year(year) = context.period else {
        return;
    };
    let thinned = batch
        .iter()
        .zip(prepared)
        .zip(kept)
        .filter_map(|((chunk, p), kept)| {
            let p = p.as_ref()?;
            (!p.own && !p.lacking.any()).then(|| {
                let mut t = p.thinning.clone();
                t.kept = kept.clone().unwrap_or_default();
                (chunk.index, t)
            })
        })
        .collect();
    super::super::period_land::thinned(context.geo, year, thinned);
}
