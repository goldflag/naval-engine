//! The real world's surface over the theatre's land windows (`native/docs/terrain/surface.md`): satellite colour and
//! land cover fetched from the open sources, cached on disk, and resampled onto each
//! window's square ([`naval_terrain::SurfaceMaps`]) for the land layer to shade from
//! (`TerrainLayer::set_surface_maps`). Presentation only, as the rest of the theatre's land.
//!
//! - **Sources.** Colour: EOX's Sentinel-2 cloudless, its 2016 edition (CC BY 4.0; later editions are non-commercial),
//!   web-mercator JPEG tiles ([`IMAGERY_URL`]), no finer than zoom [`MAX_ZOOM`] (19 m pixels at the equator, 8.4 m at
//!   64 degrees: the layer's own; deeper tiles are the same pixels enlarged). Cover: ESA WorldCover 2021 v200 (CC BY 4.0), a Cloud-Optimized GeoTIFF a 3-degree
//!   square ([`COVER_URL`]), read a 1024² tile at a time by HTTP range from the overview whose pixel is no coarser than a
//!   texel ([`parse_cog`]). [`ATTRIBUTION`] is what the game credits.
//! - **The maps.** A sea window's (and a world's battle's) 96 km at [`SEA_TEXELS`]² (23 m), an air window's 240 km at
//!   [`AIR_TEXELS`]² (117 m). Colour bilinear from the imagery at the zoom whose pixel is no coarser than a texel there
//!   (cos(latitude) narrows a mercator pixel), cover nearest from the overview. Only [`BLOCK`]² squares of texels with
//!   land in or beside them (from the window's heights) are fetched and filled; the rest stay empty (alpha 0, cover 0).
//! - **Cache.** Every tile is kept as fetched under [`cache_dir`] (the imagery's JPEGs, WorldCover's headers and its
//!   compressed tiles), a missing file remembered as absent, so a second visit is read from disk and works offline.
//! - **Never in the way.** [`lookup`] runs with a scene's build, off the frame loop's thread: maps already made, or a
//!   window whose every tile is on disk (assembled there, under a second), are drawn with it; otherwise the window draws
//!   its procedural cover, the fetch goes to one worker thread ([`FETCHERS`] requests at a time), and the driver
//!   rebuilds the scene once [`ready`] says its maps are made. Any failure (offline, outside the coverage, a server's
//!   error) leaves the procedural cover, logged once.
//! - **Switched** on by the client's executable ([`enable_from_env`]; `NAVAL_SURFACE=off` keeps it off), never in the
//!   library's tests, so no test reaches the network unless it asks to.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::f64::consts::PI;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::{LatLon, to_lat_lon};
use naval_terrain::SurfaceMaps;
use naval_terrain::surface_maps::cover;

/// A sea window's maps, texels on a side: 23 m on its 96 km.
pub const SEA_TEXELS: u32 = 4096;
/// An air window's, texels on a side: 117 m on its 240 km.
pub const AIR_TEXELS: u32 = 2048;
/// The finest imagery zoom fetched: the 2016 layer's own finest (measured, 2026-10-04: its zoom-14 tiles over
/// Guadalcanal and Dover hold no more detail than its zoom-13 tiles enlarged, where its 13 holds four times its 12's),
/// 19 m a pixel at the equator, 12 m at 51 degrees, 8.4 m at 64.
pub const MAX_ZOOM: u32 = 13;
/// Texels on a side of a square of the land mask: fetched and filled only where land lies in it or beside it.
pub const BLOCK: u32 = 64;
/// How far past the land a window's or tile's maps are filled at least, metres.
const SHORE_REACH_M: f64 = 300.0;
/// Texels between the nodes of the grid the projection is computed on (each texel interpolated between them).
const GRID_STEP: u32 = 32;
/// Requests in flight at once to either source (the imagery server answers a tile in about half a second from Europe,
/// so this many keep a window's few hundred tiles to a quarter of a minute without crowding it).
pub const FETCHERS: usize = 8;
/// The imagery: EOX's Sentinel-2 cloudless 2016 layer in web mercator (`s2cloudless_3857`), `{z}/{y}/{x}`.
pub const IMAGERY_URL: &str = "https://tiles.maps.eox.at/wmts/1.0.0/s2cloudless_3857/default/g";
/// The land cover: ESA WorldCover 2021 v200's 3-degree Cloud-Optimized GeoTIFFs, named by their south-west corner.
pub const COVER_URL: &str = "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map";
/// What the game credits for the maps.
pub const ATTRIBUTION: [&str; 2] = [
    "Sentinel-2 cloudless - https://s2maps.eu by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016 & 2017)",
    "ESA WorldCover project 2021 / Contains modified Copernicus Sentinel data (2021) processed by ESA WorldCover consortium",
];
const USER_AGENT: &str = concat!(
    "naval-native/",
    env!("CARGO_PKG_VERSION"),
    " (theatre land surface; tile cache on disk)"
);
/// The web-mercator sphere's circumference, metres.
const MERCATOR_CIRCUMFERENCE_M: f64 = 2.0 * PI * 6_378_137.0;
/// Metres of a degree of latitude.
const DEGREE_M: f64 = 111_320.0;
const TILE_PX: i64 = 256;
/// WorldCover's squares: 3 degrees, 36000 pixels on a side at full resolution, overviews halving it down to 1125
/// (level 5; the next, 562, does not divide the square evenly and is never read).
const COVER_SQUARE_DEG: f64 = 3.0;
const COVER_FULL_PX: i64 = 36_000;
const COVER_LEVELS: usize = 6;
const COVER_TILE_PX: i64 = 1024;
/// A WorldCover header's first read: its IFDs and tile tables lie in the first 28 KB.
const HEADER_BYTES: u64 = 65_536;
/// After a job finds no network, none is tried again for this long.
const OFFLINE_BACKOFF: Duration = Duration::from_secs(60);
/// Maps kept in memory once made, for the driver's rebuild to pick up (the scene holds its own afterwards).
const READY_KEPT: usize = 2;

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns streaming on unless `NAVAL_SURFACE` is `off` (or `0`): the client's executable calls this; the library's
/// tests never do, so they draw the procedural cover and reach no network.
pub fn enable_from_env() {
    let off = std::env::var("NAVAL_SURFACE").is_ok_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false"
        )
    });
    set_enabled(!off);
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Where the tiles are kept: `NAVAL_SURFACE_DIR` when set, else the user data directory's `surface-cache` (shared by
/// every checkout and the packaged game, since the world does not change with the build).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_SURFACE_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    crate::host::data_dir().join("surface-cache")
}

// ---------------------------------------------------------------------------------------------------------------------
// Tile math.

/// The global web-mercator pixel (x east from the antimeridian, y south from 85.05° N) of a latitude and longitude at
/// `zoom`, `TILE_PX` pixels a tile. Longitude is not wrapped, so a window across the antimeridian stays continuous.
pub fn mercator_px(lat: f64, lon: f64, zoom: u32) -> [f64; 2] {
    let n = (TILE_PX << zoom) as f64;
    let phi = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let x = (lon + 180.0) / 360.0 * n;
    let y = (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / PI) / 2.0 * n;
    [x, y]
}

/// The imagery tile (x, y) holding a latitude and longitude at `zoom`.
pub fn imagery_tile(lat: f64, lon: f64, zoom: u32) -> [i64; 2] {
    let [x, y] = mercator_px(lat, lon, zoom);
    let n = 1i64 << zoom;
    [
        ((x / TILE_PX as f64).floor() as i64).rem_euclid(n),
        ((y / TILE_PX as f64).floor() as i64).clamp(0, n - 1),
    ]
}

/// The latitude and longitude of an imagery tile's north-west corner.
pub fn imagery_corner(x: i64, y: i64, zoom: u32) -> LatLon {
    let n = (1i64 << zoom) as f64;
    let lon = x as f64 / n * 360.0 - 180.0;
    let lat = (PI * (1.0 - 2.0 * y as f64 / n)).sinh().atan().to_degrees();
    LatLon { lat, lon }
}

/// The imagery zoom for texels of `texel_m` at latitude `lat`: the shallowest whose pixel is no coarser than a texel,
/// at most [`MAX_ZOOM`].
pub fn imagery_zoom(texel_m: f64, lat: f64) -> u32 {
    let ground = MERCATOR_CIRCUMFERENCE_M * lat.to_radians().cos().max(0.01) / TILE_PX as f64;
    (0..=MAX_ZOOM)
        .find(|z| ground / f64::from(1u32 << z) <= texel_m)
        .unwrap_or(MAX_ZOOM)
}

/// The imagery's finest pixel at latitude `lat`, metres: its pixel at [`MAX_ZOOM`], narrowed by cos(latitude).
pub fn source_pixel_m(lat: f64) -> f64 {
    MERCATOR_CIRCUMFERENCE_M * lat.to_radians().cos().max(0.01)
        / TILE_PX as f64
        / f64::from(1u32 << MAX_ZOOM)
}

/// The WorldCover overview for texels of `texel_m`: the coarsest whose pixel (its north-south side, the longer) is no
/// coarser than a texel (level 0 is the full 10 m raster, each level after halves it).
pub fn cover_level(texel_m: f64) -> usize {
    (0..COVER_LEVELS)
        .rev()
        .find(|&k| COVER_SQUARE_DEG / (COVER_FULL_PX >> k) as f64 * DEGREE_M <= texel_m)
        .unwrap_or(0)
}

/// WorldCover's name for the 3-degree square whose south-west corner is (`lat0`, `lon0`) (multiples of 3):
/// `N63W024`, `S12E159`.
pub fn cover_square_name(lat0: i32, lon0: i32) -> String {
    format!(
        "{}{:02}{}{:03}",
        if lat0 >= 0 { 'N' } else { 'S' },
        lat0.abs(),
        if lon0 >= 0 { 'E' } else { 'W' },
        lon0.abs()
    )
}

/// The name of the WorldCover square holding a latitude and longitude.
pub fn cover_square_of(lat: f64, lon: f64) -> String {
    let lon = naval_earth::geo::wrap_lon(lon);
    let lat0 = (lat / COVER_SQUARE_DEG).floor() as i32 * 3;
    let lon0 = (lon / COVER_SQUARE_DEG).floor() as i32 * 3;
    cover_square_name(lat0, lon0)
}

/// Pixels on a side of a WorldCover square at `level`.
fn cover_side(level: usize) -> i64 {
    COVER_FULL_PX >> level
}

/// The global WorldCover pixel (x east from the antimeridian, y south from the pole) at `level`; longitude unwrapped.
fn cover_px(lat: f64, lon: f64, level: usize) -> [f64; 2] {
    let side = cover_side(level) as f64;
    [
        (lon + 180.0) / COVER_SQUARE_DEG * side,
        (90.0 - lat) / COVER_SQUARE_DEG * side,
    ]
}

/// The square (column east from the antimeridian, row south from the pole, both in squares) and tile in it holding
/// global WorldCover tile (`gx`, `gy`) at `level` (the squares' tiles laid side by side, the last of each partial).
fn cover_tile_place(gx: i64, gy: i64, level: usize) -> ([i64; 2], [i64; 2]) {
    let per = cover_tiles_per_side(level);
    (
        [gx.div_euclid(per), gy.div_euclid(per)],
        [gx.rem_euclid(per), gy.rem_euclid(per)],
    )
}

fn cover_tiles_per_side(level: usize) -> i64 {
    (cover_side(level) + COVER_TILE_PX - 1) / COVER_TILE_PX
}

/// The name of the square at column `sx` (east from the antimeridian, unwrapped) and row `sy` (south from the pole).
fn cover_square_at(sx: i64, sy: i64) -> String {
    let lat0 = 90 - 3 * (sy as i32 + 1);
    let lon0 = -180 + 3 * sx.rem_euclid(120) as i32;
    cover_square_name(lat0, lon0)
}

// ---------------------------------------------------------------------------------------------------------------------
// Windows and their plans.

/// A window's heights, as the land mask reads them: chart metres, `quanta > 0` land. A host keeping its heights in a
/// type of its own (the client's window, the simulation's field) makes the view of it.
#[derive(Clone, Copy)]
pub struct Heights<'a> {
    pub columns: usize,
    pub rows: usize,
    pub cell: f64,
    pub origin: [f64; 2],
    pub quanta: &'a [i16],
}

impl<'a> Heights<'a> {
    pub fn of_drawn(f: &'a naval_terrain::Heightfield) -> Self {
        Self {
            columns: f.columns as usize,
            rows: f.rows as usize,
            cell: f64::from(f.cell),
            origin: [f64::from(f.origin_x), f64::from(f.origin_z)],
            quanta: &f.quanta,
        }
    }

    /// The square the heights span, chart metres: its corner and side.
    pub fn square(&self) -> ([f64; 2], f64) {
        let side = (self.columns.max(self.rows) - 1) as f64 * self.cell;
        (self.origin, side)
    }
}

/// Which maps a window takes: the scenario's frame (`geo`, its origin), the theatre point of the window's chart (0, 0)
/// (`centre`), and the square of the chart they cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceKey {
    pub geo: LatLon,
    pub centre: [f64; 2],
    pub origin: [f64; 2],
    pub extent_m: f64,
    pub size: u32,
}

impl SurfaceKey {
    /// The maps of `size`² texels for heights about theatre point `centre` in the scenario about `geo`.
    pub fn of(geo: LatLon, centre: [f64; 2], heights: &Heights, size: u32) -> Self {
        let (origin, extent_m) = heights.square();
        Self {
            geo,
            centre,
            origin,
            extent_m,
            size,
        }
    }

    fn bits(&self) -> [u64; 8] {
        [
            self.geo.lat.to_bits(),
            self.geo.lon.to_bits(),
            self.centre[0].to_bits(),
            self.centre[1].to_bits(),
            self.origin[0].to_bits(),
            self.origin[1].to_bits(),
            self.extent_m.to_bits(),
            u64::from(self.size),
        ]
    }

    /// Metres a texel.
    pub fn texel_m(&self) -> f64 {
        self.extent_m / f64::from(self.size)
    }

    /// The chart point at the centre of texel (`i`, `j`) (fractional texels allowed).
    pub fn chart_of_texel(&self, i: f64, j: f64) -> [f64; 2] {
        let t = self.texel_m();
        [
            self.origin[0] + (i + 0.5) * t,
            self.origin[1] + (j + 0.5) * t,
        ]
    }

    /// The texel (column, row) holding chart point `c`, or none outside the square.
    pub fn texel_of_chart(&self, c: [f64; 2]) -> Option<[u32; 2]> {
        let t = self.texel_m();
        let i = ((c[0] - self.origin[0]) / t).floor();
        let j = ((c[1] - self.origin[1]) / t).floor();
        let n = f64::from(self.size);
        ((0.0..n).contains(&i) && (0.0..n).contains(&j)).then_some([i as u32, j as u32])
    }

    /// The latitude and longitude of chart point `c` (theatre point `centre + c`).
    pub fn lat_lon_of_chart(&self, c: [f64; 2]) -> LatLon {
        to_lat_lon(self.geo, [self.centre[0] + c[0], self.centre[1] + c[1]])
    }
}

/// One tile of either source.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Resource {
    /// An imagery tile, its column wrapped.
    Imagery { z: u32, x: i64, y: i64 },
    /// A NAIP tile (`naip`: zoom `naip::ZOOM`, `naip::TILE_PX`² pixels), its column wrapped.
    Naip { x: i64, y: i64 },
    /// A WorldCover square's header.
    CoverHeader { square: String },
    /// A WorldCover tile, by its index in its level's tile table.
    CoverTile {
        square: String,
        level: usize,
        index: usize,
    },
}

/// What a window's maps need: the projection on a coarse grid, the squares of texels holding land, the imagery zoom
/// and tiles, the WorldCover overview, squares and tiles (global tile coordinates).
pub struct Plan {
    pub key: SurfaceKey,
    /// Grid nodes on a side; node (a, b) at texel centre (a, b) × `GRID_STEP`, its values below.
    nodes: usize,
    /// The imagery's global pixel at `zoom` and WorldCover's at `level`, each node.
    image_px: Vec<[f64; 2]>,
    cover_px: Vec<[f64; 2]>,
    /// Blocks on a side and which hold land in or beside them.
    blocks: usize,
    wanted: Vec<bool>,
    pub zoom: u32,
    pub level: usize,
    /// Imagery tiles (x unwrapped, y).
    pub imagery: BTreeSet<[i64; 2]>,
    /// WorldCover tiles, global (x unwrapped, y) at `level`.
    pub cover: BTreeSet<[i64; 2]>,
    /// NAIP's global pixel at each node (zoom `naip::ZOOM + 1`'s, `naip::TILE_PX` a tile), and its tiles (x unwrapped, y): empty
    /// but for a near tile at NAIP's texel in the United States (`naip`).
    naip_px: Vec<[f64; 2]>,
    pub naip: BTreeSet<[i64; 2]>,
}

impl Plan {
    pub fn new(key: SurfaceKey, heights: &Heights) -> Self {
        let size = key.size.max(GRID_STEP);
        let nodes = (size / GRID_STEP) as usize + 1;
        let t = key.texel_m();
        let middle =
            key.lat_lon_of_chart(key.chart_of_texel(f64::from(size) / 2.0, f64::from(size) / 2.0));
        let zoom = imagery_zoom(t, middle.lat);
        let level = cover_level(t);
        let mut image_px = Vec::with_capacity(nodes * nodes);
        // NAIP for a near tile at its texel whose middle or a corner is in the United States.
        let naip = t <= super::naip::TEXEL_M * 1.01
            && super::naip::enabled()
            && [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)]
                .iter()
                .any(|&(a, b)| {
                    let ll = key.lat_lon_of_chart(
                        key.chart_of_texel(a * f64::from(size), b * f64::from(size)),
                    );
                    super::naip::in_conus(ll.lat, ll.lon)
                });
        let mut naip_px = Vec::with_capacity(if naip { nodes * nodes } else { 0 });
        let mut cover_px_nodes = Vec::with_capacity(nodes * nodes);
        for b in 0..nodes {
            for a in 0..nodes {
                let c = key
                    .chart_of_texel((a as u32 * GRID_STEP) as f64, (b as u32 * GRID_STEP) as f64);
                let ll = key.lat_lon_of_chart(c);
                // Longitude kept continuous about the window's middle (a window across the antimeridian).
                let lon = middle.lon + naval_earth::geo::wrap_lon(ll.lon - middle.lon);
                image_px.push(mercator_px(ll.lat, lon, zoom));
                if naip {
                    naip_px.push(mercator_px(ll.lat, lon, super::naip::ZOOM + 1));
                }
                cover_px_nodes.push(cover_px(ll.lat, lon, level));
            }
        }
        // The land mask: a block holding any land sample, and the blocks beside it (the beach, the imagery's edge).
        let blocks = (size / BLOCK).max(1) as usize;
        let mut land = vec![false; blocks * blocks];
        let block_m = t * f64::from(BLOCK);
        for j in 0..heights.rows {
            let z = heights.origin[1] + j as f64 * heights.cell;
            let bj = ((z - key.origin[1]) / block_m).floor();
            if bj < 0.0 || bj >= blocks as f64 {
                continue;
            }
            let row = &heights.quanta[j * heights.columns..(j + 1) * heights.columns];
            for (i, &q) in row.iter().enumerate() {
                if q <= 0 {
                    continue;
                }
                let x = heights.origin[0] + i as f64 * heights.cell;
                let bi = ((x - key.origin[0]) / block_m).floor();
                if bi >= 0.0 && bi < blocks as f64 {
                    land[bj as usize * blocks + bi as usize] = true;
                }
            }
        }
        // The beside reaches at least `SHORE_REACH_M` (fine texels' blocks are small: a block's edge on the beach would
        // show as steps where the photograph's coast and the heights' disagree).
        let reach = ((SHORE_REACH_M / block_m).ceil() as usize).max(1);
        let mut wanted = vec![false; blocks * blocks];
        for bj in 0..blocks {
            for bi in 0..blocks {
                if !land[bj * blocks + bi] {
                    continue;
                }
                for dj in bj.saturating_sub(reach)..=(bj + reach).min(blocks - 1) {
                    for di in bi.saturating_sub(reach)..=(bi + reach).min(blocks - 1) {
                        wanted[dj * blocks + di] = true;
                    }
                }
            }
        }
        let mut plan = Self {
            key,
            nodes,
            image_px,
            cover_px: cover_px_nodes,
            blocks,
            wanted,
            zoom,
            level,
            imagery: BTreeSet::new(),
            cover: BTreeSet::new(),
            naip_px,
            naip: BTreeSet::new(),
        };
        // Each wanted block's tiles: the box of its grid nodes, a pixel wider for the bilinear filter.
        let per_block = (BLOCK / GRID_STEP).max(1) as usize;
        for bj in 0..blocks {
            for bi in 0..blocks {
                if !plan.wanted[bj * blocks + bi] {
                    continue;
                }
                let (mut lo_i, mut hi_i) = ([f64::MAX; 2], [f64::MIN; 2]);
                let (mut lo_c, mut hi_c) = ([f64::MAX; 2], [f64::MIN; 2]);
                for b in bj * per_block..=((bj + 1) * per_block).min(nodes - 1) {
                    for a in bi * per_block..=((bi + 1) * per_block).min(nodes - 1) {
                        let p = plan.image_px[b * nodes + a];
                        let c = plan.cover_px[b * nodes + a];
                        for k in 0..2 {
                            lo_i[k] = lo_i[k].min(p[k]);
                            hi_i[k] = hi_i[k].max(p[k]);
                            lo_c[k] = lo_c[k].min(c[k]);
                            hi_c[k] = hi_c[k].max(c[k]);
                        }
                    }
                }
                let rows = (TILE_PX << zoom) - 1;
                let tile = |v: f64| (v / TILE_PX as f64).floor() as i64;
                // A pixel more for the bilinear filter; with NAIP, a match square (64 m) more for its colour match.
                let pad = if plan.naip_px.is_empty() { 1.0 } else { 8.0 };
                for y in tile((lo_i[1] - pad).max(0.0))..=tile((hi_i[1] + pad).min(rows as f64)) {
                    for x in tile(lo_i[0] - pad)..=tile(hi_i[0] + pad) {
                        plan.imagery.insert([x, y]);
                    }
                }
                // Global cover tiles: each square's tiles side by side, so a pixel's tile is found through its square.
                let global = |px: f64| {
                    let side = cover_side(level);
                    let p = px.floor() as i64;
                    p.div_euclid(side) * cover_tiles_per_side(level)
                        + p.rem_euclid(side) / COVER_TILE_PX
                };
                for gy in global(lo_c[1].max(0.0))..=global(hi_c[1]) {
                    for gx in global(lo_c[0])..=global(hi_c[0]) {
                        plan.cover.insert([gx, gy]);
                    }
                }
                if !plan.naip_px.is_empty() {
                    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                    for b in bj * per_block..=((bj + 1) * per_block).min(nodes - 1) {
                        for a in bi * per_block..=((bi + 1) * per_block).min(nodes - 1) {
                            let p = plan.naip_px[b * nodes + a];
                            for k in 0..2 {
                                lo[k] = lo[k].min(p[k]);
                                hi[k] = hi[k].max(p[k]);
                            }
                        }
                    }
                    let px = super::naip::TILE_PX as f64;
                    let rows = (1i64 << super::naip::ZOOM) - 1;
                    let tile = |v: f64| (v / px).floor() as i64;
                    for y in tile(lo[1] - 40.0).max(0)..=tile(hi[1] + 40.0).min(rows) {
                        for x in tile(lo[0] - 40.0)..=tile(hi[0] + 40.0) {
                            plan.naip.insert([x, y]);
                        }
                    }
                }
            }
        }
        plan
    }

    /// Whether any texel holds land, so the maps are worth making.
    pub fn has_land(&self) -> bool {
        self.wanted.iter().any(|&w| w)
    }

    /// The share of the window's blocks fetched and filled.
    pub fn wanted_share(&self) -> f64 {
        self.wanted.iter().filter(|&&w| w).count() as f64 / self.wanted.len() as f64
    }

    fn squares(&self) -> BTreeSet<String> {
        self.cover
            .iter()
            .map(|&[gx, gy]| {
                let (square, _) = cover_tile_place(gx, gy, self.level);
                cover_square_at(square[0], square[1])
            })
            .collect()
    }

    fn imagery_resources(&self) -> impl Iterator<Item = Resource> + '_ {
        let n = 1i64 << self.zoom;
        self.imagery.iter().map(move |&[x, y]| Resource::Imagery {
            z: self.zoom,
            x: x.rem_euclid(n),
            y,
        })
    }

    fn naip_resources(&self) -> impl Iterator<Item = Resource> + '_ {
        let n = 1i64 << super::naip::ZOOM;
        self.naip.iter().map(move |&[x, y]| Resource::Naip {
            x: x.rem_euclid(n),
            y,
        })
    }

    /// The WorldCover tiles of `square`'s header the plan needs (tile-table indices at its level).
    fn cover_indices<'a>(&'a self, square: &'a str) -> impl Iterator<Item = usize> + 'a {
        let per = cover_tiles_per_side(self.level);
        self.cover.iter().filter_map(move |&[gx, gy]| {
            let (sq, tile) = cover_tile_place(gx, gy, self.level);
            (cover_square_at(sq[0], sq[1]) == square).then_some((tile[1] * per + tile[0]) as usize)
        })
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The GeoTIFF reader.

/// A Cloud-Optimized GeoTIFF's layout: its full raster and overviews, and where its pixel (0, 0) corner stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Cog {
    pub levels: Vec<CogLevel>,
    /// Longitude and latitude of the raster's north-west corner (the tie point).
    pub corner: [f64; 2],
    /// Degrees a full-resolution pixel, east and south.
    pub scale: [f64; 2],
}

/// One image of a GeoTIFF (the full raster, or an overview), tiled.
#[derive(Clone, Debug, PartialEq)]
pub struct CogLevel {
    pub width: u32,
    pub height: u32,
    pub tile_width: u32,
    pub tile_height: u32,
    pub compression: u16,
    pub predictor: u16,
    pub offsets: Vec<u64>,
    pub counts: Vec<u64>,
}

impl CogLevel {
    pub fn tiles_across(&self) -> usize {
        self.width.div_ceil(self.tile_width) as usize
    }
}

/// Why a header would not parse.
#[derive(Clone, Debug, PartialEq)]
pub enum CogError {
    /// The bytes end before what the header points at: read this many from the start and try again.
    Short(u64),
    Invalid(String),
}

impl std::fmt::Display for CogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Short(n) => write!(f, "the header needs the first {n} bytes"),
            Self::Invalid(why) => write!(f, "{why}"),
        }
    }
}

struct TiffReader<'a> {
    bytes: &'a [u8],
    little: bool,
}

impl TiffReader<'_> {
    fn slice(&self, at: u64, len: u64) -> Result<&[u8], CogError> {
        let end = at
            .checked_add(len)
            .ok_or(CogError::Invalid("offset overflows".into()))?;
        if end > self.bytes.len() as u64 {
            return Err(CogError::Short(end));
        }
        Ok(&self.bytes[at as usize..end as usize])
    }

    fn u16(&self, at: u64) -> Result<u16, CogError> {
        let b: [u8; 2] = self.slice(at, 2)?.try_into().unwrap();
        Ok(if self.little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }

    fn u32(&self, at: u64) -> Result<u32, CogError> {
        let b: [u8; 4] = self.slice(at, 4)?.try_into().unwrap();
        Ok(if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }

    fn u64(&self, at: u64) -> Result<u64, CogError> {
        let b: [u8; 8] = self.slice(at, 8)?.try_into().unwrap();
        Ok(if self.little {
            u64::from_le_bytes(b)
        } else {
            u64::from_be_bytes(b)
        })
    }

    /// An entry's values as integers (or doubles' bits for type 12), wherever they are stored.
    fn values(&self, entry: u64) -> Result<(u16, Vec<u64>), CogError> {
        let kind = self.u16(entry + 2)?;
        let count = u64::from(self.u32(entry + 4)?);
        let size = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 | 16 | 17 => 8,
            _ => return Err(CogError::Invalid(format!("TIFF field type {kind}"))),
        };
        let at = if count * size <= 4 {
            entry + 8
        } else {
            u64::from(self.u32(entry + 8)?)
        };
        self.slice(at, count * size)?;
        let values = (0..count)
            .map(|k| {
                let p = at + k * size;
                match size {
                    1 => Ok(u64::from(self.bytes[p as usize])),
                    2 => self.u16(p).map(u64::from),
                    4 => self.u32(p).map(u64::from),
                    _ => self.u64(p),
                }
            })
            .collect::<Result<_, _>>()?;
        Ok((kind, values))
    }
}

/// Parses a classic TIFF's image file directories as a Cloud-Optimized GeoTIFF lays them out: the first the full raster
/// (with the GeoTIFF tie point and pixel scale), each after an overview, all tiled. A mask image is skipped.
pub fn parse_cog(bytes: &[u8]) -> Result<Cog, CogError> {
    let little = match bytes.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        Some(_) => return Err(CogError::Invalid("not a TIFF".into())),
        None => return Err(CogError::Short(8)),
    };
    let r = TiffReader { bytes, little };
    match r.u16(2)? {
        42 => {}
        43 => return Err(CogError::Invalid("BigTIFF is not read".into())),
        m => return Err(CogError::Invalid(format!("TIFF magic {m}"))),
    }
    let mut next = u64::from(r.u32(4)?);
    let mut levels = Vec::new();
    let (mut corner, mut scale) = (None, None);
    let mut seen = HashSet::new();
    while next != 0 {
        if !seen.insert(next) {
            return Err(CogError::Invalid("the IFDs loop".into()));
        }
        let count = u64::from(r.u16(next)?);
        r.slice(next, 2 + count * 12 + 4)?;
        let mut tags: HashMap<u16, u64> = HashMap::new();
        let (mut offsets, mut counts) = (Vec::new(), Vec::new());
        for k in 0..count {
            let entry = next + 2 + k * 12;
            let tag = r.u16(entry)?;
            match tag {
                324 | 325 | 273 | 279 => {
                    let (_, v) = r.values(entry)?;
                    if matches!(tag, 324 | 273) {
                        offsets = v;
                    } else {
                        counts = v;
                    }
                }
                33550 if levels.is_empty() => {
                    let (_, v) = r.values(entry)?;
                    if v.len() >= 2 {
                        scale = Some([f64::from_bits(v[0]), f64::from_bits(v[1])]);
                    }
                }
                33922 if levels.is_empty() => {
                    let (_, v) = r.values(entry)?;
                    if v.len() >= 6 {
                        // (I, J, K) raster to (X, Y, Z) model: the corner is pixel (I, J).
                        let [i, j, x, y] = [v[0], v[1], v[3], v[4]].map(f64::from_bits);
                        corner = Some([x, y, i, j]);
                    }
                }
                256 | 257 | 259 | 317 | 322 | 323 | 254 | 258 | 277 => {
                    let (_, v) = r.values(entry)?;
                    tags.insert(tag, v.first().copied().unwrap_or(0));
                }
                _ => {}
            }
        }
        next = u64::from(r.u32(next + 2 + count * 12)?);
        // A mask (subfile type bit 4) is not an overview.
        if tags.get(&254).is_some_and(|t| t & 4 != 0) {
            continue;
        }
        let need = |tag: u16| {
            tags.get(&tag)
                .copied()
                .ok_or_else(|| CogError::Invalid(format!("an image without tag {tag}")))
        };
        if tags.get(&258).is_some_and(|&b| b != 8) || tags.get(&277).is_some_and(|&s| s != 1) {
            return Err(CogError::Invalid("not one byte a pixel".into()));
        }
        let level = CogLevel {
            width: need(256)? as u32,
            height: need(257)? as u32,
            tile_width: need(322)? as u32,
            tile_height: need(323)? as u32,
            compression: tags.get(&259).copied().unwrap_or(1) as u16,
            predictor: tags.get(&317).copied().unwrap_or(1) as u16,
            offsets,
            counts,
        };
        let tiles = level.tiles_across() * level.height.div_ceil(level.tile_height) as usize;
        if level.offsets.len() != tiles || level.counts.len() != tiles {
            return Err(CogError::Invalid("a tile table of the wrong length".into()));
        }
        levels.push(level);
    }
    let corner = corner.ok_or_else(|| CogError::Invalid("no tie point".into()))?;
    let scale = scale.ok_or_else(|| CogError::Invalid("no pixel scale".into()))?;
    if levels.is_empty() {
        return Err(CogError::Invalid("no image".into()));
    }
    Ok(Cog {
        levels,
        corner: [
            corner[0] - corner[2] * scale[0],
            corner[1] + corner[3] * scale[1],
        ],
        scale,
    })
}

/// A tile's bytes as stored to its pixels (`tile_width * tile_height`, one byte each).
pub fn decode_cog_tile(level: &CogLevel, stored: &[u8]) -> Result<Vec<u8>, String> {
    let n = level.tile_width as usize * level.tile_height as usize;
    if stored.is_empty() {
        return Ok(vec![0; n]);
    }
    let mut pixels = match level.compression {
        1 => stored.to_vec(),
        8 | 32946 => miniz_oxide::inflate::decompress_to_vec_zlib(stored)
            .map_err(|e| format!("inflating a tile: {e:?}"))?,
        c => return Err(format!("TIFF compression {c} is not read")),
    };
    if pixels.len() < n {
        return Err(format!("a tile of {} bytes, not {n}", pixels.len()));
    }
    pixels.truncate(n);
    if level.predictor == 2 {
        for row in pixels.chunks_mut(level.tile_width as usize) {
            for k in 1..row.len() {
                row[k] = row[k].wrapping_add(row[k - 1]);
            }
        }
    }
    Ok(pixels)
}

// ---------------------------------------------------------------------------------------------------------------------
// The disk cache and the network.

pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    fn path(&self, r: &Resource) -> PathBuf {
        match r {
            Resource::Imagery { z, x, y } => self
                .dir
                .join("s2cloudless-2016")
                .join(z.to_string())
                .join(x.to_string())
                .join(format!("{y}.jpg")),
            Resource::Naip { x, y } => self
                .dir
                .join("naip-usgs")
                .join(super::naip::ZOOM.to_string())
                .join(x.to_string())
                .join(format!("{y}.img")),
            Resource::CoverHeader { square } => self
                .dir
                .join("worldcover-2021")
                .join(square)
                .join("header.tif"),
            Resource::CoverTile {
                square,
                level,
                index,
            } => self
                .dir
                .join("worldcover-2021")
                .join(square)
                .join(level.to_string())
                .join(format!("{index}.bin")),
        }
    }

    /// The marker a resource's source has none of.
    fn absent(&self, r: &Resource) -> PathBuf {
        let mut p = self.path(r).into_os_string();
        p.push(".absent");
        p.into()
    }

    /// The resource's bytes on disk: `Some(Some(bytes))`, `Some(None)` when the source has none, `None` when not fetched.
    fn read(&self, r: &Resource) -> Option<Option<Vec<u8>>> {
        match std::fs::read(self.path(r)) {
            Ok(bytes) => Some(Some(bytes)),
            Err(_) if self.absent(r).exists() => Some(None),
            Err(_) => None,
        }
    }

    fn holds(&self, r: &Resource) -> bool {
        self.path(r).exists() || self.absent(r).exists()
    }

    fn write(&self, r: &Resource, bytes: Option<&[u8]>) -> std::io::Result<()> {
        let path = match bytes {
            Some(_) => self.path(r),
            None => self.absent(r),
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Written whole, then moved into place: a reader never sees half a tile.
        let mut temp = path.clone().into_os_string();
        temp.push(format!(".{}.part", std::process::id()));
        let temp = PathBuf::from(temp);
        std::fs::write(&temp, bytes.unwrap_or_default())?;
        std::fs::rename(&temp, &path)
    }

    /// A WorldCover square's header from disk, parsed; none when absent, not fetched or unreadable.
    fn header(&self, square: &str) -> Option<Cog> {
        let r = Resource::CoverHeader {
            square: square.into(),
        };
        parse_cog(&self.read(&r)??).ok()
    }
}

/// What a fetch came to.
pub enum Fetched {
    Bytes(Vec<u8>),
    /// The source has none (404, or S3's 403 for a key it does not hold).
    Absent,
}

/// Why a fetch failed: no network at all, or the server's error.
#[derive(Debug)]
pub enum FetchError {
    Offline(String),
    Server(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline(why) => write!(f, "no network: {why}"),
            Self::Server(why) => write!(f, "{why}"),
        }
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .max_idle_connections_per_host(FETCHERS)
            .build()
            .into()
    })
}

pub fn get(url: &str, range: Option<(u64, u64)>) -> Result<Fetched, FetchError> {
    let mut request = agent().get(url);
    if let Some((from, len)) = range {
        request = request.header("Range", format!("bytes={from}-{}", from + len - 1));
    }
    let mut response = request.call().map_err(|e| match e {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => FetchError::Offline(format!("{url}: {e}")),
        other => FetchError::Server(format!("{url}: {other}")),
    })?;
    let status = response.status().as_u16();
    match status {
        200 | 206 => {
            let bytes = response
                .body_mut()
                .with_config()
                .limit(64 << 20)
                .read_to_vec()
                .map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
            if let Some((_, len)) = range {
                // A server ignoring the range sends the whole file: keep what was asked for.
                if status == 200 && bytes.len() as u64 > len {
                    return Err(FetchError::Server(format!(
                        "{url}: the range was not honoured"
                    )));
                }
            }
            Ok(Fetched::Bytes(bytes))
        }
        403 | 404 => Ok(Fetched::Absent),
        _ => Err(FetchError::Server(format!("{url}: HTTP {status}"))),
    }
}

/// Fetches `square`'s header and keeps it (the first [`HEADER_BYTES`], more when its tables reach past them).
fn fetch_header(cache: &Cache, square: &str) -> Result<(), FetchError> {
    let url = format!("{COVER_URL}/ESA_WorldCover_10m_2021_v200_{square}_Map.tif");
    let r = Resource::CoverHeader {
        square: square.into(),
    };
    let mut len = HEADER_BYTES;
    for _ in 0..3 {
        match get(&url, Some((0, len)))? {
            Fetched::Absent => {
                cache.write(&r, None).map_err(io_error)?;
                return Ok(());
            }
            Fetched::Bytes(bytes) => match parse_cog(&bytes) {
                Ok(_) => {
                    cache.write(&r, Some(&bytes)).map_err(io_error)?;
                    return Ok(());
                }
                Err(CogError::Short(need)) if need > len => len = need.next_multiple_of(4096),
                Err(e) => return Err(FetchError::Server(format!("{url}: {e}"))),
            },
        }
    }
    Err(FetchError::Server(format!(
        "{url}: the header kept growing"
    )))
}

fn fetch(cache: &Cache, r: &Resource, headers: &HashMap<String, Cog>) -> Result<(), FetchError> {
    let fetched = match r {
        Resource::Imagery { z, x, y } => get(&format!("{IMAGERY_URL}/{z}/{y}/{x}.jpg"), None)?,
        Resource::Naip { x, y } => {
            // The National Map's server answers a 502 now and then: twice more after a pause, then the tile is done
            // without NAIP (Sentinel-2 alone, nothing kept: a later tile asks again) rather than failing its batch.
            let url = super::naip::tile_url(*x, *y);
            let mut last = None;
            for attempt in 0..3 {
                if attempt > 0 {
                    std::thread::sleep(Duration::from_millis(800 * attempt));
                }
                match get(&url, None) {
                    // An ArcGIS error comes back as a 200 with JSON: not kept.
                    Ok(Fetched::Bytes(b)) if !super::naip::is_image(&b) => {
                        last = Some(format!("{url}: not an image"));
                    }
                    Ok(fetched) => {
                        let written = match fetched {
                            Fetched::Bytes(bytes) => cache.write(r, Some(&bytes)),
                            Fetched::Absent => cache.write(r, None),
                        };
                        return written.map_err(io_error);
                    }
                    // NAIP is optional: an unreachable server (the rest of the network may be fine) is NAIP missing.
                    Err(FetchError::Server(why) | FetchError::Offline(why)) => last = Some(why),
                }
            }
            if let Some(why) = last {
                log_once("naip", &format!("NAIP: {why}; Sentinel-2 alone there"));
            }
            return Ok(());
        }
        Resource::CoverHeader { square } => return fetch_header(cache, square),
        Resource::CoverTile {
            square,
            level,
            index,
        } => {
            let cog = headers
                .get(square)
                .ok_or_else(|| FetchError::Server(format!("{square}: no header")))?;
            let l = &cog.levels[*level];
            let (offset, count) = (l.offsets[*index], l.counts[*index]);
            if count == 0 {
                Fetched::Bytes(Vec::new())
            } else {
                let url = format!("{COVER_URL}/ESA_WorldCover_10m_2021_v200_{square}_Map.tif");
                match get(&url, Some((offset, count)))? {
                    Fetched::Bytes(b) if b.len() as u64 != count => {
                        return Err(FetchError::Server(format!(
                            "{url}: {} bytes of a tile's {count}",
                            b.len()
                        )));
                    }
                    other => other,
                }
            }
        }
    };
    match fetched {
        Fetched::Bytes(bytes) => cache.write(r, Some(&bytes)),
        Fetched::Absent => cache.write(r, None),
    }
    .map_err(io_error)
}

fn io_error(e: std::io::Error) -> FetchError {
    FetchError::Server(format!("the surface cache: {e}"))
}

/// Whether a square's header suits the reader: the 36000² raster with its overviews halving, 1024² tiles, the corner on
/// the square's.
fn header_fits(cog: &Cog, square: &str, level: usize) -> bool {
    let Some(l) = cog.levels.get(level) else {
        return false;
    };
    let (lat0, lon0) = parse_square(square);
    cog.levels[0].width as i64 == COVER_FULL_PX
        && l.width as i64 == cover_side(level)
        && l.tile_width as i64 == COVER_TILE_PX
        && l.tile_height as i64 == COVER_TILE_PX
        && (cog.corner[0] - f64::from(lon0)).abs() < 1e-6
        && (cog.corner[1] - f64::from(lat0 + 3)).abs() < 1e-6
}

fn parse_square(square: &str) -> (i32, i32) {
    let lat: i32 = square[1..3].parse().unwrap_or(0);
    let lon: i32 = square[4..7].parse().unwrap_or(0);
    (
        if square.starts_with('S') { -lat } else { lat },
        if &square[3..4] == "W" { -lon } else { lon },
    )
}

/// Everything the plan needs that the cache does not hold, its headers read from disk; `None` while a header is
/// missing (it is fetched first).
pub fn missing(cache: &Cache, plan: &Plan) -> Option<(Vec<Resource>, HashMap<String, Cog>)> {
    let mut wanted: Vec<Resource> = plan
        .imagery_resources()
        .chain(plan.naip_resources())
        .filter(|r| !cache.holds(r))
        .collect();
    let mut headers = HashMap::new();
    for square in plan.squares() {
        let r = Resource::CoverHeader {
            square: square.clone(),
        };
        if !cache.holds(&r) {
            return None;
        }
        let Some(cog) = cache.header(&square) else {
            continue;
        };
        if header_fits(&cog, &square, plan.level) {
            for index in plan.cover_indices(&square) {
                let r = Resource::CoverTile {
                    square: square.clone(),
                    level: plan.level,
                    index,
                };
                if !cache.holds(&r) {
                    wanted.push(r);
                }
            }
        }
        headers.insert(square, cog);
    }
    Some((wanted, headers))
}

// ---------------------------------------------------------------------------------------------------------------------
// Making the maps.

/// The plan's maps from the tiles on disk (a tile not there leaves its texels empty).
/// `log`: a line with the timings (the near tiles, made by the dozen, say nothing).
pub fn assemble(cache: &Cache, plan: &Plan, log: bool) -> Result<SurfaceMaps, String> {
    let started = Instant::now();
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8);
    // The imagery tiles, decoded in parallel, laid out over their box.
    let (ix0, iy0) = (
        plan.imagery.iter().map(|t| t[0]).min().unwrap_or(0),
        plan.imagery.iter().map(|t| t[1]).min().unwrap_or(0),
    );
    let (ix1, iy1) = (
        plan.imagery.iter().map(|t| t[0]).max().unwrap_or(-1),
        plan.imagery.iter().map(|t| t[1]).max().unwrap_or(-1),
    );
    let (iw, ih) = (
        (ix1 - ix0 + 1).max(0) as usize,
        (iy1 - iy0 + 1).max(0) as usize,
    );
    let n = 1i64 << plan.zoom;
    let list: Vec<[i64; 2]> = plan.imagery.iter().copied().collect();
    let decoded = parallel_map(&list, threads, |&[x, y]| {
        let r = Resource::Imagery {
            z: plan.zoom,
            x: x.rem_euclid(n),
            y,
        };
        let bytes = cache.read(&r)??;
        let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Jpeg).ok()?;
        let rgb = image.to_rgb8();
        (rgb.width() == TILE_PX as u32 && rgb.height() == TILE_PX as u32).then(|| rgb.into_raw())
    });
    let mut imagery: Vec<Option<Vec<u8>>> = vec![None; iw * ih];
    for ([x, y], tile) in list.iter().zip(decoded) {
        imagery[(y - iy0) as usize * iw + (x - ix0) as usize] = tile;
    }
    let decoded_imagery = started.elapsed();
    // The cover tiles likewise.
    let (cx0, cy0) = (
        plan.cover.iter().map(|t| t[0]).min().unwrap_or(0),
        plan.cover.iter().map(|t| t[1]).min().unwrap_or(0),
    );
    let (cx1, cy1) = (
        plan.cover.iter().map(|t| t[0]).max().unwrap_or(-1),
        plan.cover.iter().map(|t| t[1]).max().unwrap_or(-1),
    );
    let (cw, ch) = (
        (cx1 - cx0 + 1).max(0) as usize,
        (cy1 - cy0 + 1).max(0) as usize,
    );
    let headers: HashMap<String, Option<Cog>> = plan
        .squares()
        .into_iter()
        .map(|s| {
            let cog = cache.header(&s).filter(|c| header_fits(c, &s, plan.level));
            (s, cog)
        })
        .collect();
    let per = cover_tiles_per_side(plan.level);
    let list: Vec<[i64; 2]> = plan.cover.iter().copied().collect();
    let decoded = parallel_map(&list, threads, |&[gx, gy]| {
        let (sq, tile) = cover_tile_place(gx, gy, plan.level);
        let square = cover_square_at(sq[0], sq[1]);
        let cog = headers.get(&square)?.as_ref()?;
        let index = (tile[1] * per + tile[0]) as usize;
        let r = Resource::CoverTile {
            square,
            level: plan.level,
            index,
        };
        let stored = cache.read(&r)??;
        decode_cog_tile(&cog.levels[plan.level], &stored).ok()
    });
    let mut cover_tiles: Vec<Option<Vec<u8>>> = vec![None; cw * ch];
    for ([x, y], tile) in list.iter().zip(decoded) {
        cover_tiles[(y - cy0) as usize * cw + (x - cx0) as usize] = tile;
    }
    let decoded_cover = started.elapsed();
    // Resampled a band of rows a thread.
    let size = plan.key.size as usize;
    let mut albedo = vec![0u8; size * size * 4];
    let mut cover_map = vec![0u8; size * size];
    let side = cover_side(plan.level);
    let image_px = |gx: i64, gy: i64| -> Option<[u8; 3]> {
        let (tx, ty) = (gx.div_euclid(TILE_PX) - ix0, gy.div_euclid(TILE_PX) - iy0);
        if tx < 0 || ty < 0 || tx >= iw as i64 || ty >= ih as i64 {
            return None;
        }
        let tile = imagery[ty as usize * iw + tx as usize].as_ref()?;
        let k = (gy.rem_euclid(TILE_PX) * TILE_PX + gx.rem_euclid(TILE_PX)) as usize * 3;
        Some([tile[k], tile[k + 1], tile[k + 2]])
    };
    let cover_at = |px: i64, py: i64| -> u8 {
        let gx = px.div_euclid(side) * per + px.rem_euclid(side) / COVER_TILE_PX;
        let gy = py.div_euclid(side) * per + py.rem_euclid(side) / COVER_TILE_PX;
        let (tx, ty) = (gx - cx0, gy - cy0);
        if tx < 0 || ty < 0 || tx >= cw as i64 || ty >= ch as i64 {
            return cover::UNKNOWN;
        }
        let Some(tile) = cover_tiles[ty as usize * cw + tx as usize].as_ref() else {
            return cover::UNKNOWN;
        };
        let (lx, ly) = (
            px.rem_euclid(side) % COVER_TILE_PX,
            py.rem_euclid(side) % COVER_TILE_PX,
        );
        tile[(ly * COVER_TILE_PX + lx) as usize]
    };
    // A texel under 0.4 of the cover's pixel (its full raster: 9.3 m north to south).
    let fine_cover = plan.key.texel_m() < 0.4 * 9.26 * f64::from(1u32 << plan.level);
    let rows_each = size.div_ceil(threads);
    // At the caller's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|s| {
        for (band, (albedo, cover_rows)) in albedo
            .chunks_mut(rows_each * size * 4)
            .zip(cover_map.chunks_mut(rows_each * size))
            .enumerate()
        {
            let image_px = &image_px;
            let cover_at = &cover_at;
            s.spawn(move || {
                qos.apply();
                let first = band * rows_each;
                for (r, (row, crow)) in albedo
                    .chunks_mut(size * 4)
                    .zip(cover_rows.chunks_mut(size))
                    .enumerate()
                {
                    let j = first + r;
                    let bj = (j / BLOCK as usize).min(plan.blocks - 1);
                    let v = j as f64 / f64::from(GRID_STEP);
                    let b = (v.floor() as usize).min(plan.nodes - 2);
                    let fv = v - b as f64;
                    for (i, (texel, class)) in row
                        .as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .zip(crow.iter_mut())
                        .enumerate()
                    {
                        let bi = (i / BLOCK as usize).min(plan.blocks - 1);
                        if !plan.wanted[bj * plan.blocks + bi] {
                            continue;
                        }
                        let u = i as f64 / f64::from(GRID_STEP);
                        let a = (u.floor() as usize).min(plan.nodes - 2);
                        let fu = u - a as f64;
                        let lerp = |grid: &[[f64; 2]]| {
                            let at = |a: usize, b: usize| grid[b * plan.nodes + a];
                            let (p00, p10, p01, p11) =
                                (at(a, b), at(a + 1, b), at(a, b + 1), at(a + 1, b + 1));
                            [0, 1].map(|k| {
                                (p00[k] * (1.0 - fu) + p10[k] * fu) * (1.0 - fv)
                                    + (p01[k] * (1.0 - fu) + p11[k] * fu) * fv
                            })
                        };
                        // Colour: bilinear between the four imagery pixels about the texel's centre.
                        let [px, py] = lerp(&plan.image_px);
                        let (x, y) = (px - 0.5, py - 0.5);
                        let (x0, y0) = (x.floor(), y.floor());
                        let (fx, fy) = (x - x0, y - y0);
                        let (x0, y0) = (x0 as i64, y0 as i64);
                        let mut sum = [0.0f64; 3];
                        let mut weight = 0.0;
                        for (dx, dy, w) in [
                            (0, 0, (1.0 - fx) * (1.0 - fy)),
                            (1, 0, fx * (1.0 - fy)),
                            (0, 1, (1.0 - fx) * fy),
                            (1, 1, fx * fy),
                        ] {
                            if let Some(p) = image_px(x0 + dx, y0 + dy) {
                                for k in 0..3 {
                                    sum[k] += f64::from(p[k]) * w;
                                }
                                weight += w;
                            }
                        }
                        if weight > 1e-6 {
                            for k in 0..3 {
                                texel[k] = (sum[k] / weight).round().clamp(0.0, 255.0) as u8;
                            }
                            texel[3] = 255;
                        }
                        // Cover: the overview's pixel under the texel's centre; texels much finer than its pixel (NAIP's 2 m
                        // over its 10 m) look it up through a smooth warp of about a pixel, so a class's edge is ragged,
                        // not the raster's steps.
                        let [cx, cy] = lerp(&plan.cover_px);
                        let (cx, cy) = if fine_cover {
                            (
                                cx + (value_noise(cx * 0.6, cy * 0.6, 17) - 0.5) * 1.4,
                                cy + (value_noise(cx * 0.6, cy * 0.6, 91) - 0.5) * 1.4,
                            )
                        } else {
                            (cx, cy)
                        };
                        *class = cover_at(cx.floor() as i64, cy.floor() as i64);
                    }
                }
            });
        }
    });
    if !plan.naip.is_empty() {
        overlay_naip(cache, plan, &mut albedo, threads, &image_px);
    }
    if log {
        log_line(&format!(
            "surface maps {}² ({:.0} m texels, zoom {}, cover level {}): {} imagery and {} cover tiles; decoded in {:.0} + {:.0} ms, resampled in {:.0} ms",
            size,
            plan.key.texel_m(),
            plan.zoom,
            plan.level,
            plan.imagery.len(),
            plan.cover.len(),
            decoded_imagery.as_secs_f64() * 1e3,
            (decoded_cover - decoded_imagery).as_secs_f64() * 1e3,
            (started.elapsed() - decoded_cover).as_secs_f64() * 1e3,
        ));
    }
    Ok(SurfaceMaps {
        origin: plan.key.origin,
        extent_m: plan.key.extent_m,
        size: plan.key.size,
        albedo,
        cover: cover_map,
    })
}

/// Smooth value noise in 0-1 at (`x`, `y`) (a lattice of hashed values a unit apart, smoothstepped between), `seed` its
/// own field: the same everywhere a tile or window takes it, so tiles meet.
fn value_noise(x: f64, y: f64, seed: u64) -> f64 {
    let hash = |i: i64, j: i64| -> f64 {
        let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9);
        h ^= h >> 31;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 29;
        (h >> 11) as f64 / (1u64 << 53) as f64
    };
    let (i, j) = (x.floor(), y.floor());
    let (fx, fy) = (x - i, y - j);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (i, j) = (i as i64, j as i64);
    let top = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let bottom = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    top + (bottom - top) * sy
}

/// `f` over `items` on up to `threads` threads, in order.
pub fn parallel_map<T: Sync, U: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> U + Sync,
) -> Vec<U> {
    if items.is_empty() {
        return Vec::new();
    }
    let chunk = items.len().div_ceil(threads);
    // At the caller's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|s| {
        let f = &f;
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    qos.apply();
                    c.iter().map(f).collect::<Vec<U>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a surface worker panicked"))
            .collect()
    })
}

// ---------------------------------------------------------------------------------------------------------------------
// NAIP over a near tile.

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb_byte(l: f32) -> u8 {
    let l = l.clamp(0.0, 1.0);
    let c = if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    };
    (c * 255.0).round() as u8
}

/// The ratio of Sentinel-2's local mean to NAIP's over each square of `naip::MATCH_TEXELS` texels (linear light, per
/// channel, clamped to a quarter and four times): squares with too little of both filled from their neighbours, any left
/// from the whole tile's, else none (1).
pub fn match_ratios(
    size: usize,
    s2: &[Option<[f32; 3]>],
    naip: &[Option<[f32; 3]>],
) -> (usize, Vec<[f32; 3]>) {
    let m = super::naip::MATCH_TEXELS;
    let cells = size.div_ceil(m);
    let mut sums = vec![([0f64; 3], [0f64; 3], 0usize); cells * cells];
    let mut total = ([0f64; 3], [0f64; 3], 0usize);
    for j in 0..size {
        for i in 0..size {
            let k = j * size + i;
            if let (Some(a), Some(b)) = (s2[k], naip[k]) {
                let cell = &mut sums[(j / m) * cells + i / m];
                for c in 0..3 {
                    cell.0[c] += f64::from(a[c]);
                    cell.1[c] += f64::from(b[c]);
                    total.0[c] += f64::from(a[c]);
                    total.1[c] += f64::from(b[c]);
                }
                cell.2 += 1;
                total.2 += 1;
            }
        }
    }
    let ratio_of = |s: &([f64; 3], [f64; 3], usize)| -> [f32; 3] {
        [0, 1, 2].map(|c| ((s.0[c] + 1e-4) / (s.1[c] + 1e-4)).clamp(0.25, 4.0) as f32)
    };
    let enough = m * m / 8;
    let mut ratios: Vec<Option<[f32; 3]>> = sums
        .iter()
        .map(|s| (s.2 >= enough).then(|| ratio_of(s)))
        .collect();
    // Gaps filled from their neighbours, ring by ring.
    for _ in 0..cells {
        if ratios.iter().all(Option::is_some) {
            break;
        }
        let before = ratios.clone();
        for cj in 0..cells {
            for ci in 0..cells {
                if before[cj * cells + ci].is_some() {
                    continue;
                }
                let mut sum = [0f32; 3];
                let mut count = 0;
                for (dj, di) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                    let (nj, ni) = (cj as i64 + dj, ci as i64 + di);
                    if nj < 0 || ni < 0 || nj >= cells as i64 || ni >= cells as i64 {
                        continue;
                    }
                    if let Some(r) = before[nj as usize * cells + ni as usize] {
                        for c in 0..3 {
                            sum[c] += r[c];
                        }
                        count += 1;
                    }
                }
                if count > 0 {
                    ratios[cj * cells + ci] = Some(sum.map(|s| s / count as f32));
                }
            }
        }
    }
    let whole = if total.2 > 0 {
        ratio_of(&total)
    } else {
        [1.0; 3]
    };
    (
        cells,
        ratios.into_iter().map(|r| r.unwrap_or(whole)).collect(),
    )
}

/// The value of a node grid (`nodes`² nodes `GRID_STEP` texels apart) at texel (`u`, `v`) in texels, interpolated
/// bilinearly inside it and extrapolated from its edge cells past it (a margin a few dozen texels wide: the projection is
/// as good as linear there).
fn grid_at(grid: &[[f64; 2]], nodes: usize, u: f64, v: f64) -> [f64; 2] {
    let (u, v) = (u / f64::from(GRID_STEP), v / f64::from(GRID_STEP));
    let a = (u.floor().max(0.0) as usize).min(nodes - 2);
    let b = (v.floor().max(0.0) as usize).min(nodes - 2);
    let (fu, fv) = (u - a as f64, v - b as f64);
    let at = |a: usize, b: usize| grid[b * nodes + a];
    let (p00, p10, p01, p11) = (at(a, b), at(a + 1, b), at(a, b + 1), at(a + 1, b + 1));
    [0, 1].map(|k| {
        (p00[k] * (1.0 - fu) + p10[k] * fu) * (1.0 - fv) + (p01[k] * (1.0 - fu) + p11[k] * fu) * fv
    })
}

/// Bilinear in linear light from `sample` (global pixels: an sRGB colour and its coverage 0-1, none outside) at global
/// pixel `p`; the colour and the coverage, none where nothing covers it.
fn bilinear_linear(
    lut: &[f32],
    p: [f64; 2],
    sample: &dyn Fn(i64, i64) -> Option<([u8; 3], f64)>,
) -> Option<([f32; 3], f32)> {
    let (x, y) = (p[0] - 0.5, p[1] - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut sum = [0f64; 3];
    let mut weight = 0.0;
    for (dx, dy, w) in [
        (0, 0, (1.0 - fx) * (1.0 - fy)),
        (1, 0, fx * (1.0 - fy)),
        (0, 1, (1.0 - fx) * fy),
        (1, 1, fx * fy),
    ] {
        if let Some((c, cover)) = sample(x0 + dx, y0 + dy)
            && cover > 0.0
        {
            let w = w * cover;
            for k in 0..3 {
                sum[k] += f64::from(lut[c[k] as usize]) * w;
            }
            weight += w;
        }
    }
    (weight > 1e-3).then(|| (sum.map(|s| (s / weight) as f32), weight.min(1.0) as f32))
}

/// NAIP over a near tile's Sentinel-2 colour (`naip`): each texel's NAIP colour sampled bilinearly from its tiles (none
/// where the mosaic has none: the sea, past the border), then matched to the Sentinel-2 colour under it at the scale of
/// `naip::MATCH_TEXELS` texels ([`match_ratios`]: NAIP's detail times the ratio of the two's local means, in linear
/// light), so the land's calibration holds and no seam shows where NAIP ends. The match squares lie on the theatre's
/// own grid (a near tile is a whole number of them) and the ratios are measured over a square's margin past the tile's
/// edge too (its sources fetched with it, `Plan::new`), so two neighbouring tiles measure the squares along their edge
/// alike and interpolate across it as one. `image_px`: the Sentinel-2 imagery's global pixel, as `assemble` decoded it.
fn overlay_naip(
    cache: &Cache,
    plan: &Plan,
    albedo: &mut [u8],
    threads: usize,
    image_px: &(dyn Fn(i64, i64) -> Option<[u8; 3]> + Sync),
) {
    use super::naip;
    let n = 1i64 << naip::ZOOM;
    let list: Vec<[i64; 2]> = plan.naip.iter().copied().collect();
    let decoded = parallel_map(&list, threads, |&[x, y]| {
        let bytes = cache.read(&Resource::Naip {
            x: x.rem_euclid(n),
            y,
        })??;
        naip::decode(&bytes)
    });
    let tiles: HashMap<[i64; 2], Vec<u8>> = list
        .iter()
        .copied()
        .zip(decoded)
        .filter_map(|(k, t)| Some((k, t?)))
        .collect();
    if tiles.is_empty() {
        return;
    }
    let lut: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let size = plan.key.size as usize;
    let px = naip::TILE_PX;
    let naip_sample = |gx: i64, gy: i64| -> Option<([u8; 3], f64)> {
        let t = tiles.get(&[gx.div_euclid(px), gy.div_euclid(px)])?;
        let k = ((gy.rem_euclid(px) * px + gx.rem_euclid(px)) * 4) as usize;
        Some(([t[k], t[k + 1], t[k + 2]], f64::from(t[k + 3]) / 255.0))
    };
    let s2_sample = |gx: i64, gy: i64| image_px(gx, gy).map(|c| (c, 1.0));
    // Both sources over the tile and a match square's margin about it: texel (i, j) of the tile at (i + m, j + m).
    let m = naip::MATCH_TEXELS;
    let wide = size + 2 * m;
    let mut naip_texels: Vec<Option<[f32; 3]>> = vec![None; wide * wide];
    let mut coverage = vec![0f32; wide * wide];
    let mut s2: Vec<Option<[f32; 3]>> = vec![None; wide * wide];
    let rows_each = wide.div_ceil(threads);
    // At the caller's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|s| {
        for (band, ((naip_rows, cover_rows), s2_rows)) in naip_texels
            .chunks_mut(rows_each * wide)
            .zip(coverage.chunks_mut(rows_each * wide))
            .zip(s2.chunks_mut(rows_each * wide))
            .enumerate()
        {
            let (naip_sample, s2_sample, lut) = (&naip_sample, &s2_sample, &lut);
            s.spawn(move || {
                qos.apply();
                for (r, ((naip_row, cover_row), s2_row)) in naip_rows
                    .chunks_mut(wide)
                    .zip(cover_rows.chunks_mut(wide))
                    .zip(s2_rows.chunks_mut(wide))
                    .enumerate()
                {
                    let j = (band * rows_each + r) as f64 - m as f64;
                    for (e, ((naip, cover), s2)) in naip_row
                        .iter_mut()
                        .zip(cover_row.iter_mut())
                        .zip(s2_row.iter_mut())
                        .enumerate()
                    {
                        let i = e as f64 - m as f64;
                        let n = grid_at(&plan.naip_px, plan.nodes, i, j);
                        if let Some((c, w)) = bilinear_linear(lut, n, naip_sample) {
                            *naip = Some(c);
                            *cover = w;
                        }
                        let p = grid_at(&plan.image_px, plan.nodes, i, j);
                        *s2 = bilinear_linear(lut, p, s2_sample).map(|(c, _)| c);
                    }
                }
            });
        }
    });
    let (cells, ratios) = match_ratios(wide, &s2, &naip_texels);
    let ratio_at = |e: usize, f: usize| -> [f32; 3] {
        let mf = m as f32;
        let u = ((e as f32 + 0.5) / mf - 0.5).clamp(0.0, (cells - 1) as f32);
        let v = ((f as f32 + 0.5) / mf - 0.5).clamp(0.0, (cells - 1) as f32);
        let (a, b) = (u.floor() as usize, v.floor() as usize);
        let (a1, b1) = ((a + 1).min(cells - 1), (b + 1).min(cells - 1));
        let (fu, fv) = (u - a as f32, v - b as f32);
        let r = |a: usize, b: usize| ratios[b * cells + a];
        [0, 1, 2].map(|c| {
            (r(a, b)[c] * (1.0 - fu) + r(a1, b)[c] * fu) * (1.0 - fv)
                + (r(a, b1)[c] * (1.0 - fu) + r(a1, b1)[c] * fu) * fv
        })
    };
    for j in 0..size {
        let bj = (j / BLOCK as usize).min(plan.blocks - 1);
        for i in 0..size {
            let bi = (i / BLOCK as usize).min(plan.blocks - 1);
            if !plan.wanted[bj * plan.blocks + bi] {
                continue;
            }
            let (e, f) = (i + m, j + m);
            let k = f * wide + e;
            let Some(naip) = naip_texels[k] else {
                continue;
            };
            let ratio = ratio_at(e, f);
            let matched = [0, 1, 2].map(|c| naip[c] * ratio[c]);
            let w = coverage[k];
            let texel = &mut albedo[(j * size + i) * 4..(j * size + i) * 4 + 4];
            let base = (texel[3] == 255).then(|| {
                [
                    lut[texel[0] as usize],
                    lut[texel[1] as usize],
                    lut[texel[2] as usize],
                ]
            });
            let out = match base {
                Some(base) => [0, 1, 2].map(|c| base[c] + (matched[c] - base[c]) * w),
                None if w > 0.5 => matched,
                None => continue,
            };
            for c in 0..3 {
                texel[c] = linear_to_srgb_byte(out[c]);
            }
            texel[3] = 255;
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The streamer: one worker thread, its queue, and the maps it has made.

/// Where a window's maps stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Streaming is off, or nothing was asked for these maps.
    Off,
    /// Waiting for the worker.
    Queued,
    /// Fetching: tiles fetched of those missing.
    Fetching { done: usize, total: usize },
    /// Made, and waiting for the scene to be rebuilt with them.
    Ready,
    /// The fetch failed: the procedural cover stays.
    Failed,
}

/// A scene's maps: none (streaming off, no land, no region), drawn, or waiting for the worker.
#[derive(Clone, Default)]
pub enum Surface {
    #[default]
    None,
    Waiting(SurfaceKey),
    Drawn(Arc<SurfaceMaps>),
}

impl std::fmt::Debug for Surface {
    /// The maps' size, never their buffers.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "Surface::None"),
            Self::Waiting(key) => write!(f, "Surface::Waiting({key:?})"),
            Self::Drawn(maps) => {
                write!(f, "Surface::Drawn({}² over {} m)", maps.size, maps.extent_m)
            }
        }
    }
}

impl Surface {
    pub fn maps(&self) -> Option<&Arc<SurfaceMaps>> {
        match self {
            Self::Drawn(maps) => Some(maps),
            _ => None,
        }
    }

    /// The state the RPC reports: drawn, or what the worker is doing for it.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::None => {
                serde_json::json!({"drawn": false, "state": if enabled() { "none" } else { "off" }, "pending": 0})
            }
            Self::Drawn(maps) => {
                serde_json::json!({"drawn": true, "state": "drawn", "pending": 0, "size": maps.size})
            }
            Self::Waiting(key) => {
                let (state, pending, total) = match status(key) {
                    Status::Off => ("off", 0, 0),
                    Status::Queued => ("queued", 1, 0),
                    Status::Fetching { done, total } => ("fetching", (total - done).max(1), total),
                    // Made: drawn once the scene is rebuilt with them.
                    Status::Ready => ("ready", 1, 0),
                    Status::Failed => ("failed", 0, 0),
                };
                serde_json::json!({"drawn": false, "state": state, "pending": pending, "total": total})
            }
        }
    }
}

struct State {
    queue: VecDeque<Plan>,
    status: HashMap<[u64; 8], Status>,
    ready: VecDeque<([u64; 8], Arc<SurfaceMaps>)>,
    offline_until: Option<Instant>,
    worker: bool,
}

/// The surface's streamer: its cache, its queue of plans and their status, and its worker's wake.
pub struct Streamer {
    cache: Cache,
    state: Mutex<State>,
    wake: Condvar,
}

fn streamer() -> &'static Streamer {
    static STREAMER: OnceLock<Streamer> = OnceLock::new();
    STREAMER.get_or_init(|| streamer_at(cache_dir()))
}

/// A streamer of its own over the cache at `dir`, whose plans a caller runs on its own thread ([`Streamer::run`]): a
/// test's, which fetches a window's maps cold and then warm (the client's Reykjavik window).
pub fn streamer_at(dir: PathBuf) -> Streamer {
    Streamer {
        cache: Cache { dir },
        state: Mutex::new(State {
            queue: VecDeque::new(),
            status: HashMap::new(),
            ready: VecDeque::new(),
            offline_until: None,
            worker: false,
        }),
        wake: Condvar::new(),
    }
}

/// Logs a line once a session.
pub fn log_once(kind: &str, line: &str) {
    static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged
        .get_or_insert_with(HashSet::new)
        .insert(kind.to_owned())
    {
        eprintln!("the theatre's surface: {line}");
    }
}

pub fn log_line(line: &str) {
    eprintln!("the theatre's surface: {line}");
}

/// The maps for `key` (heights `heights`): made already, or every tile on disk (assembled now: off the frame loop's
/// thread, with the scene's build), else asked of the worker ([`Surface::Waiting`]). [`Surface::None`] with streaming
/// off or no land in the window.
pub fn lookup(key: SurfaceKey, heights: &Heights) -> Surface {
    if !enabled() {
        return Surface::None;
    }
    let s = streamer();
    let bits = key.bits();
    if let Some(maps) = s.ready(&bits) {
        return Surface::Drawn(maps);
    }
    let plan = Plan::new(key, heights);
    if !plan.has_land() {
        return Surface::None;
    }
    if let Some((wanted, _)) = missing(&s.cache, &plan)
        && wanted.is_empty()
    {
        match assemble(&s.cache, &plan, true) {
            Ok(maps) => {
                let maps = Arc::new(maps);
                s.keep(bits, maps.clone());
                return Surface::Drawn(maps);
            }
            Err(e) => log_once("assemble", &format!("making maps from the cache: {e}")),
        }
    }
    s.request(plan);
    Surface::Waiting(key)
}

/// Whether the maps for `key` are made and waiting: the scene asked for them should be rebuilt.
pub fn ready(key: &SurfaceKey) -> bool {
    enabled() && streamer().ready(&key.bits()).is_some()
}

/// Where the maps for `key` stand.
pub fn status(key: &SurfaceKey) -> Status {
    if !enabled() {
        return Status::Off;
    }
    let state = streamer().state.lock().unwrap_or_else(|e| e.into_inner());
    state
        .status
        .get(&key.bits())
        .copied()
        .unwrap_or(Status::Off)
}

impl Streamer {
    fn ready(&self, bits: &[u64; 8]) -> Option<Arc<SurfaceMaps>> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .ready
            .iter()
            .find(|(k, _)| k == bits)
            .map(|(_, m)| m.clone())
    }

    fn keep(&self, bits: [u64; 8], maps: Arc<SurfaceMaps>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.ready.retain(|(k, _)| *k != bits);
        state.ready.push_back((bits, maps));
        while state.ready.len() > READY_KEPT {
            state.ready.pop_front();
        }
        state.status.insert(bits, Status::Ready);
    }

    fn request(&'static self, plan: Plan) {
        let bits = plan.key.bits();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.offline_until.is_some_and(|t| Instant::now() < t) {
            state.status.insert(bits, Status::Failed);
            return;
        }
        if let Some(Status::Queued | Status::Fetching { .. } | Status::Ready) =
            state.status.get(&bits)
        {
            return;
        }
        state.status.insert(bits, Status::Queued);
        // The newest window first; one asked for long ago and not begun is dropped.
        state.queue.push_back(plan);
        while state.queue.len() > 3 {
            if let Some(old) = state.queue.pop_front() {
                state.status.remove(&old.key.bits());
            }
        }
        if !state.worker {
            state.worker = true;
            // Below the frame loop it would inherit and the simulation (`host`).
            let spawned = crate::host::spawn_utility("theatre-surface", move || self.work());
            if let Err(e) = spawned {
                state.worker = false;
                log_once("thread", &format!("no worker thread: {e}"));
            }
        }
        self.wake.notify_one();
    }

    fn work(&'static self) {
        loop {
            let plan = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                loop {
                    if let Some(plan) = state.queue.pop_back() {
                        break plan;
                    }
                    state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            };
            let bits = plan.key.bits();
            let started = Instant::now();
            match self.run(&plan) {
                Ok((fetched, bytes)) => match assemble(&self.cache, &plan, true) {
                    Ok(maps) => {
                        log_line(&format!(
                            "{} tiles ({:.1} MB) fetched and the maps made in {:.1} s",
                            fetched,
                            bytes as f64 / 1e6,
                            started.elapsed().as_secs_f64()
                        ));
                        self.keep(bits, Arc::new(maps));
                    }
                    Err(e) => {
                        log_once("assemble", &format!("making maps: {e}"));
                        self.set(bits, Status::Failed);
                    }
                },
                Err(e) => {
                    if matches!(e, FetchError::Offline(_)) {
                        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                        state.offline_until = Some(Instant::now() + OFFLINE_BACKOFF);
                    }
                    log_once(
                        match e {
                            FetchError::Offline(_) => "offline",
                            FetchError::Server(_) => "server",
                        },
                        &format!("{e}; the land keeps its procedural cover"),
                    );
                    self.set(bits, Status::Failed);
                }
            }
        }
    }

    fn set(&self, bits: [u64; 8], status: Status) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.status.insert(bits, status);
    }

    /// The tiles' cache.
    pub fn cache(&self) -> &Cache {
        &self.cache
    }

    /// Fetches what the plan needs and the cache lacks: the headers first, then every tile, [`FETCHERS`] at a time.
    /// The tiles fetched and their bytes on disk.
    pub fn run(&self, plan: &Plan) -> Result<(usize, u64), FetchError> {
        let bits = plan.key.bits();
        let headers = missing_headers(&self.cache, plan);
        let mut fetched = self.fetch_all(bits, &headers, &HashMap::new())?;
        let (wanted, headers) = missing(&self.cache, plan)
            .ok_or_else(|| FetchError::Server("a WorldCover header did not arrive".into()))?;
        fetched += self.fetch_all(bits, &wanted, &headers)?;
        let bytes = wanted
            .iter()
            .filter_map(|r| std::fs::metadata(self.cache.path(r)).ok())
            .map(|m| m.len())
            .sum();
        Ok((fetched, bytes))
    }

    fn fetch_all(
        &self,
        bits: [u64; 8],
        list: &[Resource],
        headers: &HashMap<String, Cog>,
    ) -> Result<usize, FetchError> {
        fetch_list(&self.cache, list, headers, &|done, total| {
            self.set(bits, Status::Fetching { done, total });
        })
    }
}

/// Fetches every resource of `list` into the cache, [`FETCHERS`] at a time, `progress` told (done, total) as each
/// arrives; the first failure ends it (what arrived stays cached). How many were fetched.
pub fn fetch_list(
    cache: &Cache,
    list: &[Resource],
    headers: &HashMap<String, Cog>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<usize, FetchError> {
    if list.is_empty() {
        return Ok(0);
    }
    progress(0, list.len());
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let failure: Mutex<Option<FetchError>> = Mutex::new(None);
    let stop = AtomicBool::new(false);
    // At the caller's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|s| {
        for _ in 0..FETCHERS.min(list.len()) {
            s.spawn(|| {
                qos.apply();
                while !stop.load(Ordering::Relaxed) {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(r) = list.get(k) else {
                        break;
                    };
                    match fetch(cache, r, headers) {
                        Ok(()) => {
                            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                            progress(d, list.len());
                        }
                        Err(e) => {
                            // One failure ends the job: the window keeps its procedural cover, and a later one tries
                            // again (what arrived stays cached).
                            stop.store(true, Ordering::Relaxed);
                            failure
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get_or_insert(e);
                        }
                    }
                }
            });
        }
    });
    match failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(e) => Err(e),
        None => Ok(done.into_inner()),
    }
}

/// The WorldCover headers `plan` needs that the cache does not hold (fetched before anything else: they say where the
/// tiles are).
pub fn missing_headers(cache: &Cache, plan: &Plan) -> Vec<Resource> {
    plan.squares()
        .into_iter()
        .map(|square| Resource::CoverHeader { square })
        .filter(|r| !cache.holds(r))
        .collect()
}

/// Whether a job found no network within the last [`OFFLINE_BACKOFF`]: nothing is fetched meanwhile.
pub fn offline() -> bool {
    let state = streamer().state.lock().unwrap_or_else(|e| e.into_inner());
    state.offline_until.is_some_and(|t| Instant::now() < t)
}

/// A job found no network: nothing is fetched for [`OFFLINE_BACKOFF`].
pub fn mark_offline() {
    let mut state = streamer().state.lock().unwrap_or_else(|e| e.into_inner());
    state.offline_until = Some(Instant::now() + OFFLINE_BACKOFF);
}

/// The tile cache every streamer reads and writes.
pub fn cache() -> &'static Cache {
    &streamer().cache
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
