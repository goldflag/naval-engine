//! The near surface (`native/docs/terrain/surface.md`, "Near tiles"): finer squares of the
//! same sources as a window's maps, about the camera. A window's maps are 23 m a texel (a sea window's) or 117 m (an air
//! window's), soft within a few kilometres of a ship close inshore or a plane low over the land; the near tiles carry
//! the imagery's own finest pixel ([`texel_m`]: 19 m at the equator, 12 m at 51 degrees, 8.4 m at 64, no finer than
//! [`MIN_TEXEL_M`]) and WorldCover's full 10 m raster, 512 texels a tile ([`tile_m`]).
//!
//! - **Where they add.** Only where a near texel is at most [`GAIN`] of the window's (`install`): every air window, and
//!   a sea window north of about 40 degrees; a sea window in the tropics is already at the imagery's pixel.
//! - **The grid.** Tiles are fixed squares of the theatre's metres (tile (i, j) from (i, j) × the tile's side, the
//!   scenario's latitude's), so one made once serves every scene that comes back to it. A land layer holds a block of
//!   them about the camera, as many a side as its window's maps have tiles' worth of texels (8 for a sea window's
//!   4096², 4 for an air window's 2048²), the camera in its middle two tiles; the two share the layer's textures
//!   (`naval_terrain::NearGrid`).
//! - **Streamed** like the window's maps, from the same cache: one worker thread (`theatre-surface-near`) takes the
//!   tiles the camera's block lacks, nearest first, [`BATCH`] at a time (their missing imagery and cover fetched
//!   together, [`super::surface::FETCHERS`] requests at a time), resamples each, builds its colour's mip chain, and keeps
//!   it in memory ([`KEPT`] tiles). The frame loop hands each made tile to the land layer, which uploads a few a frame
//!   (`TerrainLayer::queue_near_tile`), so a re-cut never waits for a scene build and never blocks a frame.
//! - **Moving.** As the camera crosses into another tile's middle, the block moves a column or a row: the tiles coming
//!   into it are fetched and uploaded into the slots of the ones leaving, which keep drawing until replaced.
//! - **A past period's ground** (`period_land`): in `Period::Year` a tile's cover is marked where the land was country
//!   as it is made (its period one of the tiles' keys), and marked again by the worker, ahead of new tiles, when what
//!   is known of its ground changes (the buildings' answer about the camera, the window's estimate); the frame loop then
//!   rewrites the drawn tile's cover (`TerrainLayer::queue_cover_patch`).

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::LatLon;
use naval_terrain::buildings::Period;
use naval_terrain::surface_maps::NEAR_TILE_TEXELS;
use naval_terrain::{CoverLayer, CoverPatch, NearGrid, NearTile};

use super::period_land;
use super::surface::{self, FetchError, Heights, Plan, SurfaceKey};

/// The finest near texel, metres (the imagery's pixel is finer only north of 64 degrees; WorldCover's is 9 m).
pub const MIN_TEXEL_M: f64 = 8.0;
/// Near tiles are drawn only where their texel is at most this share of the window maps' (`install`).
pub const GAIN: f64 = 0.75;
/// Tiles the worker fetches and makes together.
pub const BATCH: usize = 8;
/// Tiles kept in memory once made (1.66 MB each, 0.6 MB with their colour in BC7): a sea window's block and a column or
/// two beyond it.
pub const KEPT: usize = 96;

/// Whether the worker makes tiles with their colour in BC7 (`NearTile::from_maps_bc7`): the device samples it, as
/// [`install`] finds (the land encodes a tile made otherwise as it uploads it).
static COMPRESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A tile whose fetch failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Whether near tiles are drawn: with streaming on ([`surface::enabled`]) unless `NAVAL_SURFACE_NEAR` is `off` (or `0`),
/// which leaves the window's maps alone, as before them (for comparisons).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_SURFACE_NEAR").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

/// Metres a near texel of Sentinel-2 in the scenario about `geo`: the imagery's finest pixel at its latitude, at least
/// [`MIN_TEXEL_M`].
pub fn texel_m(geo: LatLon) -> f64 {
    surface::source_pixel_m(geo.lat).max(MIN_TEXEL_M)
}

/// The finest window maps' texel near tiles take NAIP over (a sea window's 23 m; an air window's 117 m keeps Sentinel-2's
/// near tiles, whose 4 × 4 block then reaches 30 km rather than NAIP's 4).
pub const NAIP_WINDOW_MAX_M: f64 = 50.0;

/// Metres a near texel over a window of `window_texel_m` texels in the scenario about `geo`: NAIP's (`naip::TEXEL_M`, a
/// kilometre a tile) over a sea window in the United States (`naip::serves`), else Sentinel-2's ([`texel_m`]).
pub fn texel_for(geo: LatLon, window_texel_m: f64) -> f64 {
    if window_texel_m <= NAIP_WINDOW_MAX_M && super::naip::serves(geo) {
        return super::naip::TEXEL_M;
    }
    texel_m(geo)
}

/// Metres a near tile's side in the scenario about `geo`.
pub fn tile_m(geo: LatLon) -> f64 {
    texel_m(geo) * f64::from(NEAR_TILE_TEXELS)
}

/// The tile (i, j) of side `tile_m` holding theatre point `p`.
pub fn tile_of(p: [f64; 2], tile_m: f64) -> [i64; 2] {
    [
        (p[0] / tile_m).floor() as i64,
        (p[1] / tile_m).floor() as i64,
    ]
}

/// The near grid of tiles of side `tile_m` on a land layer whose chart point (0, 0) is theatre point `origin` and whose
/// maps are `size` texels a side, and the tile its tile (0, 0) is: tile (i, j) of the theatre is the layer's (i, j) less
/// it.
pub fn grid(origin: [f64; 2], size: u32, tile_m: f64) -> (NearGrid, [i64; 2]) {
    let base = tile_of(origin, tile_m);
    (
        NearGrid {
            origin: [
                base[0] as f64 * tile_m - origin[0],
                base[1] as f64 * tile_m - origin[1],
            ],
            tile_m,
            slots: (size / NEAR_TILE_TEXELS).max(1),
        },
        base,
    )
}

/// The block of `slots`² tiles of side `tile_m` about theatre point `at`, nearest first: the camera in its middle two
/// tiles a side.
pub fn block(at: [f64; 2], slots: u32, tile_m: f64) -> Vec<[i64; 2]> {
    let half = i64::from(slots / 2);
    let lo = [
        (at[0] / tile_m + 0.5).floor() as i64 - half,
        (at[1] / tile_m + 0.5).floor() as i64 - half,
    ];
    let mut tiles: Vec<[i64; 2]> = (0..i64::from(slots))
        .flat_map(|j| (0..i64::from(slots)).map(move |i| [lo[0] + i, lo[1] + j]))
        .collect();
    let distance = |t: &[i64; 2]| {
        let c = [
            (t[0] as f64 + 0.5) * tile_m - at[0],
            (t[1] as f64 + 0.5) * tile_m - at[1],
        ];
        c[0].hypot(c[1])
    };
    tiles.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    tiles
}

/// A window's heights kept in its host's own type (the client's window holds the simulation's field), read as the maps
/// read them.
pub trait WindowHeights: Send + Sync {
    fn heights(&self) -> Heights<'_>;

    /// The ground at chart point (`x`, `z`), metres above the sea, as the host samples its heights (the buildings and
    /// the infrastructure stand on it).
    fn height(&self, x: f64, z: f64) -> f64;
}

/// The land the tiles are made over: a window's heights (which mark the land, as for its maps) and the theatre point of
/// their chart's (0, 0).
#[derive(Clone)]
pub enum Source {
    Window {
        field: Arc<dyn WindowHeights>,
        origin: [f64; 2],
    },
    Battle {
        field: Arc<naval_terrain::Heightfield>,
        origin: [f64; 2],
    },
}

impl Source {
    /// The theatre point of the land's chart (0, 0).
    pub fn origin(&self) -> [f64; 2] {
        match self {
            Self::Window { origin, .. } | Self::Battle { origin, .. } => *origin,
        }
    }

    fn heights(&self) -> Heights<'_> {
        match self {
            Self::Window { field, .. } => field.heights(),
            Self::Battle { field, .. } => Heights::of_drawn(field),
        }
    }

    /// The maps' key for tile `index` of side `side` in the scenario about `geo`.
    fn key(&self, geo: LatLon, side: f64, index: [i64; 2]) -> SurfaceKey {
        let o = self.origin();
        SurfaceKey {
            geo,
            centre: o,
            origin: [index[0] as f64 * side - o[0], index[1] as f64 * side - o[1]],
            extent_m: side,
            size: NEAR_TILE_TEXELS,
        }
    }
}

/// A made tile: the scenario's frame, the tile's side, the period its cover is marked for (`Period::key`) and its index.
type Key = ([u64; 4], [i64; 2]);

fn key(geo: LatLon, side: f64, period: Period, index: [i64; 2]) -> Key {
    (
        [
            geo.lat.to_bits(),
            geo.lon.to_bits(),
            side.to_bits(),
            u64::from(period.key()),
        ],
        index,
    )
}

/// The scenario's frame.
fn frame(geo: LatLon) -> [u64; 2] {
    [geo.lat.to_bits(), geo.lon.to_bits()]
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    /// Tiles asked for and not begun.
    pub queued: usize,
    /// Tiles of the batch in hand.
    pub working: usize,
    /// Its fetch: resources fetched of those missing.
    pub fetched: usize,
    pub fetching: usize,
    /// Tiles that failed in the last [`RETRY_AFTER`].
    pub failed: usize,
}

struct State {
    geo: Option<LatLon>,
    /// The side of the tiles asked for, and the period they are marked for.
    side: f64,
    period: Period,
    source: Option<Source>,
    wanted: VecDeque<[i64; 2]>,
    /// Made tiles whose ground is known better now, to be marked again (ahead of `wanted`).
    remark: VecDeque<[i64; 2]>,
    /// Made tiles (none: no land or no imagery there), oldest first in `order`.
    made: HashMap<Key, Option<Arc<NearTile>>>,
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
            side: 0.0,
            period: Period::Today,
            source: None,
            wanted: VecDeque::new(),
            remark: VecDeque::new(),
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

/// Tile `index` of side `side` in the scenario about `geo`, its cover marked for `period`: `Some(Some(tile))` made,
/// `Some(None)` made with nothing to draw (no land, no imagery), `None` not yet.
pub fn made(
    geo: LatLon,
    side: f64,
    period: Period,
    index: [i64; 2],
) -> Option<Option<Arc<NearTile>>> {
    lock().made.get(&key(geo, side, period, index)).cloned()
}

/// Asks for `tiles` of side `side` (nearest first) over `source`, their covers marked for `period`, in place of whatever
/// was asked before; a tile made, or failed lately, is left out. Nothing with streaming off.
pub fn want(geo: LatLon, side: f64, period: Period, source: &Source, tiles: &[[i64; 2]]) {
    if !surface::enabled() {
        return;
    }
    let mut state = lock();
    let now = Instant::now();
    state
        .failed
        .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    let wanted: VecDeque<[i64; 2]> = tiles
        .iter()
        .copied()
        .filter(|&t| {
            let k = key(geo, side, period, t);
            !state.made.contains_key(&k) && !state.failed.contains_key(&k)
        })
        .collect();
    if state.geo.is_none_or(|g| frame(g) != frame(geo))
        || state.side != side
        || state.period != period
    {
        state.remark.clear();
    }
    state.geo = Some(geo);
    state.side = side;
    state.period = period;
    state.source = Some(source.clone());
    state.status.queued = wanted.len();
    state.wanted = wanted;
    if state.wanted.is_empty() {
        return;
    }
    wake(&mut state);
}

/// Asks for made tiles of side `side` in the scenario about `geo`, marked for `period`, to be marked again from what is
/// known of their ground now (`period_land`), ahead of the tiles wanted. Whether the worker took them: not while it
/// serves another scenario, side or period ([`want`] sets which).
pub fn remark(geo: LatLon, side: f64, period: Period, tiles: &[[i64; 2]]) -> bool {
    if tiles.is_empty() {
        return true;
    }
    let mut state = lock();
    if state.geo.is_none_or(|g| frame(g) != frame(geo))
        || state.side != side
        || state.period != period
    {
        return false;
    }
    for &tile in tiles {
        if !state.remark.contains(&tile) {
            state.remark.push_back(tile);
        }
    }
    wake(&mut state);
    true
}

/// Starts the worker if it is not running, and wakes it.
fn wake(state: &mut State) {
    if !state.worker {
        state.worker = true;
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-surface-near", work);
        if let Err(e) = spawned {
            state.worker = false;
            surface::log_once("near-thread", &format!("no near worker thread: {e}"));
        }
    }
    streamer().wake.notify_one();
}

/// Lays `period`'s marks over a copy of `tile`'s cover (tile `index` of side `side` in the scenario about `geo`) from
/// what is known of its ground now; none for today's, or while nothing is known of the scenario's past ground.
fn marked(
    geo: LatLon,
    side: f64,
    period: Period,
    index: [i64; 2],
    tile: &NearTile,
) -> Option<NearTile> {
    let answers = period_land::answers(geo, period)?;
    let mut cover = tile.cover.clone();
    let grid = period_land::Grid {
        corner: [index[0] as f64 * side, index[1] as f64 * side],
        texel_m: side / f64::from(NEAR_TILE_TEXELS),
        size: [NEAR_TILE_TEXELS as usize; 2],
    };
    period_land::mark(&answers, &grid, &mut cover);
    Some(NearTile {
        levels: tile.levels.clone(),
        bc7: tile.bc7,
        cover,
        marks: period_land::marks(period, answers.version()),
    })
}

/// Marks again the made tiles of `batch` (`remark`): each in its place in the cache.
fn remark_batch(geo: LatLon, side: f64, period: Period, batch: &[[i64; 2]]) {
    for &index in batch {
        let k = key(geo, side, period, index);
        let Some(Some(tile)) = lock().made.get(&k).cloned() else {
            continue;
        };
        if let Some(again) = marked(geo, side, period, index, &tile) {
            let mut state = lock();
            if let Some(entry) = state.made.get_mut(&k) {
                *entry = Some(Arc::new(again));
            }
        }
    }
}

/// What the worker is doing.
pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        failed: state.failed.len(),
        ..state.status
    }
}

fn work() {
    loop {
        let (geo, side, period, source, batch) = {
            let mut state = lock();
            loop {
                let (side, period) = (state.side, state.period);
                // Tiles to mark again first: a few at a time, quick (no fetching), and what is drawn now.
                if let Some(geo) = state.geo
                    && !state.remark.is_empty()
                {
                    let take = state.remark.len().min(BATCH);
                    let batch: Vec<[i64; 2]> = state.remark.drain(..take).collect();
                    drop(state);
                    remark_batch(geo, side, period, &batch);
                    state = lock();
                    continue;
                }
                if let (Some(geo), Some(source)) = (state.geo, state.source.clone()) {
                    // (A tile asked for again while its batch was in hand is made by now.)
                    let mut batch = Vec::with_capacity(BATCH);
                    while batch.len() < BATCH
                        && let Some(index) = state.wanted.pop_front()
                    {
                        if !state.made.contains_key(&key(geo, side, period, index)) {
                            batch.push(index);
                        }
                    }
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (geo, side, period, source, batch);
                    }
                }
                state.status.working = 0;
                state = streamer()
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = Instant::now();
        let result = make(geo, side, period, &source, &batch);
        let mut state = lock();
        state.status.fetched = 0;
        state.status.fetching = 0;
        match result {
            Ok((tiles, fetched)) => {
                if fetched > 0 {
                    surface::log_line(&format!(
                        "near tiles: {} made ({} resources fetched) in {:.1} s",
                        tiles.len(),
                        fetched,
                        started.elapsed().as_secs_f64()
                    ));
                }
                for (index, tile) in tiles {
                    let k = key(geo, side, period, index);
                    if state.made.insert(k, tile).is_none() {
                        state.order.push_back(k);
                    }
                }
                while state.order.len() > KEPT {
                    if let Some(old) = state.order.pop_front() {
                        state.made.remove(&old);
                    }
                }
            }
            Err(e) => {
                if matches!(e, FetchError::Offline(_)) {
                    surface::mark_offline();
                }
                surface::log_once(
                    "near",
                    &format!("near tiles: {e}; the window's maps stay near the camera"),
                );
                let now = Instant::now();
                for &index in &batch {
                    state.failed.insert(key(geo, side, period, index), now);
                }
                // What was asked after this batch would fail the same way: asked again by the next frame that wants it.
                state.wanted.clear();
                state.status.queued = 0;
            }
        }
    }
}

/// Fetches what the batch's tiles lack and makes them: each tile's colour and cover (marked for `period`), or none where
/// it holds no land. The tiles and the resources fetched.
#[allow(clippy::type_complexity)]
fn make(
    geo: LatLon,
    side: f64,
    period: Period,
    source: &Source,
    batch: &[[i64; 2]],
) -> Result<(Vec<([i64; 2], Option<Arc<NearTile>>)>, usize), FetchError> {
    if surface::offline() {
        return Err(FetchError::Offline("tried lately".into()));
    }
    let cache = surface::cache();
    let heights = source.heights();
    let plans: Vec<([i64; 2], Plan)> = batch
        .iter()
        .map(|&index| (index, Plan::new(source.key(geo, side, index), &heights)))
        .collect();
    let progress = |done: usize, total: usize| {
        let mut state = lock();
        state.status.fetched = done;
        state.status.fetching = total;
    };
    // The headers first (they say where the cover's tiles are), then everything else at once.
    let headers: BTreeSet<_> = plans
        .iter()
        .filter(|(_, p)| p.has_land())
        .flat_map(|(_, p)| surface::missing_headers(cache, p))
        .collect();
    let headers: Vec<_> = headers.into_iter().collect();
    let mut fetched = surface::fetch_list(cache, &headers, &HashMap::new(), &progress)?;
    let mut wanted = BTreeSet::new();
    let mut cogs = HashMap::new();
    for (_, plan) in plans.iter().filter(|(_, p)| p.has_land()) {
        let (resources, headers) = surface::missing(cache, plan)
            .ok_or_else(|| FetchError::Server("a WorldCover header did not arrive".into()))?;
        wanted.extend(resources);
        cogs.extend(headers);
    }
    let wanted: Vec<_> = wanted.into_iter().collect();
    fetched += surface::fetch_list(cache, &wanted, &cogs, &progress)?;
    let mut tiles = Vec::with_capacity(plans.len());
    let mut seen = HashSet::new();
    for (index, plan) in &plans {
        if !seen.insert(*index) {
            continue;
        }
        let tile = if plan.has_land() {
            match surface::assemble(cache, plan, false) {
                Ok(maps) if maps.albedo.chunks(4).any(|p| p[3] == 255) => {
                    let made = if COMPRESS.load(std::sync::atomic::Ordering::Relaxed) {
                        NearTile::from_maps_bc7(&maps).ok()
                    } else {
                        NearTile::from_maps(&maps).ok()
                    };
                    // A past period's ground marked over its cover (`period_land`).
                    made.map(|tile| {
                        Arc::new(marked(geo, side, period, *index, &tile).unwrap_or(tile))
                    })
                }
                Ok(_) => None,
                Err(e) => {
                    surface::log_once("near-assemble", &format!("making near tiles: {e}"));
                    None
                }
            }
        } else {
            None
        };
        tiles.push((*index, tile));
    }
    Ok((tiles, fetched))
}

/// Where the camera stood at the last frame a scene fed its near tiles (the scenario's frame, the theatre point).
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 2])>> = Mutex::new(None);

/// Gives a land layer being built (off the frame loop's thread, with its scene) near tiles on its grid (its chart's
/// (0, 0) at theatre point `origin`, its maps `size` texels of `window_texel_m` a side) where they add to its maps
/// ([`GAIN`]), and uploads the made tiles of the block about where the camera last stood (marked for `period`), so a
/// scene rebuilt where the old one stood draws them from its first frame.
pub fn install(
    ctx: &mut naval_render::RenderContext<'_>,
    land: &mut naval_terrain::TerrainLayer,
    geo: LatLon,
    period: Period,
    origin: [f64; 2],
    size: u32,
    window_texel_m: f64,
) {
    COMPRESS.store(
        naval_terrain::bc7::supported(ctx.device),
        std::sync::atomic::Ordering::Relaxed,
    );
    let texel = texel_for(geo, window_texel_m);
    if !enabled() || texel > GAIN * window_texel_m {
        return;
    }
    let (grid, base) = grid(origin, size, texel * f64::from(NEAR_TILE_TEXELS));
    if let Err(e) = land.set_near_grid(ctx, grid) {
        surface::log_once("near-grid", &format!("near tiles: {e}"));
        return;
    }
    let last = *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    let Some((_, at)) = last.filter(|(f, _)| *f == frame(geo)) else {
        return;
    };
    for index in block(at, grid.slots, grid.tile_m) {
        if let Some(Some(tile)) = made(geo, grid.tile_m, period, index) {
            land.queue_near_tile(
                [(index[0] - base[0]) as i32, (index[1] - base[1]) as i32],
                tile,
            );
        }
    }
    land.flush_near_tiles(ctx);
}

/// What a scene on screen does with its near tiles, kept by the driver from frame to frame.
#[derive(Default)]
pub struct Feed {
    /// The scene it feeds (the driver's swap count when it was made).
    pub scene: u64,
    /// The block about the camera, nearest first, and the tile the block was made about.
    block: Vec<[i64; 2]>,
    centre: Option<[i64; 2]>,
    /// Tiles made with nothing to draw.
    empty: HashSet<[i64; 2]>,
    /// The tiles last asked of the worker, and when.
    asked: Vec<[i64; 2]>,
    asked_at: Option<Instant>,
    /// The block's tiles not yet made.
    pub pending: usize,
    /// The period's ground's version when the drawn tiles' marks were last checked, whether they are to be checked anyway
    /// (a tile's cover rewritten, a tile queued, the block moved), and the tiles asked to be marked again (with the
    /// version they were asked at).
    checked: u64,
    patched: bool,
    remarking: HashMap<[i64; 2], u64>,
}

impl Feed {
    pub fn new(scene: u64) -> Self {
        Self {
            scene,
            ..Self::default()
        }
    }

    /// Hands `land` the made tiles of the block about its camera, marked for `period`, and asks the worker for the rest;
    /// rewrites a drawn tile's cover once the worker has marked it again. Cheap enough for every frame: a lookup a tile.
    pub fn feed(
        &mut self,
        land: &mut naval_terrain::TerrainLayer,
        geo: LatLon,
        period: Period,
        source: &Source,
    ) {
        let Some(grid) = land.near_grid() else {
            return;
        };
        let Some(camera) = land.camera_chart() else {
            return;
        };
        let origin = source.origin();
        let at = [camera[0] + origin[0], camera[1] + origin[1]];
        *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner()) = Some((frame(geo), at));
        let base = tile_of(origin, grid.tile_m);
        let centre = [
            (at[0] / grid.tile_m + 0.5).floor() as i64,
            (at[1] / grid.tile_m + 0.5).floor() as i64,
        ];
        if self.centre != Some(centre) {
            self.centre = Some(centre);
            self.block = block(at, grid.slots, grid.tile_m);
            // Tiles come into the block: their marks are checked.
            self.patched = true;
        }
        let mut missing = Vec::new();
        for k in 0..self.block.len() {
            let index = self.block[k];
            let local = [(index[0] - base[0]) as i32, (index[1] - base[1]) as i32];
            if self.empty.contains(&index) {
                continue;
            }
            if land.holds_near_tile(local) {
                self.refresh(land, geo, period, grid.tile_m, index, local);
                continue;
            }
            match made(geo, grid.tile_m, period, index) {
                Some(Some(tile)) => {
                    land.queue_near_tile(local, tile);
                    // (A tile made before what is known now: its marks are checked.)
                    self.patched = true;
                }
                Some(None) => {
                    self.empty.insert(index);
                }
                None => missing.push(index),
            }
        }
        self.pending = missing.len();
        // Asked again when the list changes, and now and then while it does not (a failed fetch is retried); once by a new
        // scene whatever it lacks (the worker then serves its scenario, side and period: its tiles' marks, `remark`).
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if missing != self.asked || self.asked_at.is_none() || (!missing.is_empty() && stale) {
            want(geo, grid.tile_m, period, source, &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
        self.check_marks(land, geo, period, grid.tile_m, base);
    }

    /// A drawn tile's cover rewritten when the worker has marked it again since it was drawn.
    fn refresh(
        &mut self,
        land: &mut naval_terrain::TerrainLayer,
        geo: LatLon,
        period: Period,
        side: f64,
        index: [i64; 2],
        local: [i32; 2],
    ) {
        if period == Period::Today {
            return;
        }
        let Some(drawn) = land.near_tile_marks(local) else {
            return;
        };
        if let Some(Some(tile)) = made(geo, side, period, index)
            && period_land::marks_version(tile.marks) > period_land::marks_version(drawn)
        {
            land.queue_cover_patch(CoverPatch {
                layer: CoverLayer::Near(local),
                at: [0, 0],
                size: [NEAR_TILE_TEXELS; 2],
                classes: tile.cover.clone(),
                marks: tile.marks,
            });
            self.remarking.remove(&index);
            self.patched = true;
        }
    }

    /// When what is known of the period's ground has changed since the drawn tiles were marked (or a tile's cover was
    /// rewritten), asks the worker to mark again those it touches.
    fn check_marks(
        &mut self,
        land: &naval_terrain::TerrainLayer,
        geo: LatLon,
        period: Period,
        side: f64,
        base: [i64; 2],
    ) {
        if period == Period::Today {
            return;
        }
        let now = period_land::version();
        if now == self.checked && !self.patched {
            return;
        }
        self.checked = now;
        self.patched = false;
        let block = &self.block;
        self.remarking.retain(|index, _| block.contains(index));
        let mut again = Vec::new();
        for &index in &self.block {
            let local = [(index[0] - base[0]) as i32, (index[1] - base[1]) as i32];
            let Some(drawn) = land.near_tile_marks(local) else {
                continue;
            };
            if self.remarking.get(&index).is_some_and(|&at| at >= now) {
                continue;
            }
            let chunks = tile_chunks(index, side);
            if period_land::changed_since(
                geo,
                period,
                period_land::marks_version(drawn),
                chunks,
                true,
            ) {
                again.push(index);
            }
        }
        if remark(geo, side, period, &again) {
            for index in again {
                self.remarking.insert(index, now);
            }
        } else {
            // Checked again next frame (the worker taken up by this scene's `want`).
            self.patched = true;
        }
    }
}

/// The buildings' chunks (first column, first row, last column, last row) under near tile `index` of side `side`.
fn tile_chunks(index: [i64; 2], side: f64) -> [i32; 4] {
    let chunk = naval_terrain::buildings::CHUNK_M;
    let lo = |i: i64| (i as f64 * side / chunk).floor() as i32;
    let hi = |i: i64| (((i + 1) as f64 * side) / chunk).ceil() as i32 - 1;
    [lo(index[0]), lo(index[1]), hi(index[0]), hi(index[1])]
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: f64 = 4096.0;

    #[test]
    fn the_block_keeps_the_camera_in_its_middle_and_the_grid_matches_the_theatres_tiles() {
        // A sea window's 8 a side about a point inside tile (2, -3): the camera at least 3.5 tiles from the block's
        // edge, the nearest tile first.
        let at = [2.3 * T, -2.6 * T];
        let tiles = block(at, 8, T);
        assert_eq!(tiles.len(), 64);
        assert_eq!(tiles[0], tile_of(at, T));
        let (lo_i, hi_i) = (
            tiles.iter().map(|t| t[0]).min().unwrap(),
            tiles.iter().map(|t| t[0]).max().unwrap() + 1,
        );
        let (lo_j, hi_j) = (
            tiles.iter().map(|t| t[1]).min().unwrap(),
            tiles.iter().map(|t| t[1]).max().unwrap() + 1,
        );
        for (lo, hi, v) in [(lo_i, hi_i, at[0]), (lo_j, hi_j, at[1])] {
            assert_eq!(hi - lo, 8);
            let (lo, hi) = (lo as f64 * T, hi as f64 * T);
            assert!(v - lo >= 3.5 * T && hi - v >= 3.5 * T, "{v} in {lo}..{hi}");
        }
        // An air window's 4: 1.5 tiles at least.
        let tiles = block(at, 4, T);
        assert_eq!(tiles.len(), 16);
        let lo = tiles.iter().map(|t| t[0]).min().unwrap() as f64 * T;
        assert!(at[0] - lo >= 1.5 * T);

        // The layer's grid: a theatre tile's chart corner is its grid corner, whatever the window's origin.
        let origin = [-37_123.4, 81_906.7];
        let (g, base) = grid(origin, 4096, T);
        assert_eq!(g.slots, 8);
        assert_eq!(g.tile_m, T);
        let index = [-12, 22];
        let local = [index[0] - base[0], index[1] - base[1]];
        let chart = [
            g.origin[0] + local[0] as f64 * T,
            g.origin[1] + local[1] as f64 * T,
        ];
        assert!((chart[0] + origin[0] - index[0] as f64 * T).abs() < 1e-6);
        assert!((chart[1] + origin[1] - index[1] as f64 * T).abs() < 1e-6);
        assert!(g.origin[0] <= 0.0 && g.origin[0] > -T);
        assert_eq!(grid(origin, 2048, T).0.slots, 4);
    }

    #[test]
    fn a_near_tile_takes_the_imagerys_own_pixel_and_the_full_cover() {
        // The imagery's finest pixel: 19 m off Guadalcanal (where a sea window's 23 m texel already takes it, so no near
        // tiles there), 12 m at Dover, 8.4 m at Reykjavik; an air window's 117 m takes near tiles everywhere.
        let at = |lat: f64| LatLon { lat, lon: 0.0 };
        let sea = 96_000.0 / f64::from(surface::SEA_TEXELS);
        let air = 239_680.0 / f64::from(surface::AIR_TEXELS);
        for (lat, texel) in [(-9.4, 18.9), (51.1, 12.0), (64.1, 8.4)] {
            assert!(
                (texel_m(at(lat)) - texel).abs() < 0.1,
                "{lat}: {}",
                texel_m(at(lat))
            );
            assert_eq!(
                surface::imagery_zoom(texel_m(at(lat)), lat),
                surface::MAX_ZOOM
            );
            assert!(texel_m(at(lat)) <= GAIN * air);
        }
        assert!(texel_m(at(-9.4)) > GAIN * sea);
        assert!(texel_m(at(51.1)) <= GAIN * sea);
        assert_eq!(texel_m(at(80.0)), MIN_TEXEL_M);
        assert_eq!(tile_m(at(64.1)), texel_m(at(64.1)) * 512.0);
        // WorldCover's full raster.
        assert_eq!(surface::cover_level(texel_m(at(51.1))), 0);
        // NAIP's texel only with NAIP on (never in the library's tests), and never over an air window.
        let santa_cruz = LatLon {
            lat: 36.97,
            lon: -122.03,
        };
        assert_eq!(texel_for(santa_cruz, sea), texel_m(santa_cruz));
        assert_eq!(texel_for(santa_cruz, air), texel_m(santa_cruz));
        assert!(air > NAIP_WINDOW_MAX_M && sea <= NAIP_WINDOW_MAX_M);
    }
}
