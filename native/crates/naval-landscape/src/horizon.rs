//! The land beyond a theatre window (native/docs/terrain/layer.md, "The horizon ring"): the real world's heights and
//! colour
//! about the scene out to the far land a view of it sees, made off the frame loop's thread for the land layer's ring
//! (`naval_terrain::horizon`). Presentation only, as the rest of the theatre's land: the simulation reads the region's
//! heights as before, and nothing here reaches it.
//!
//! - **The sources** ([`ATTRIBUTION`]): the AWS Terrain Tiles (Terrarium, `relief::TERRARIUM_URL`) at about the
//!   ring's cells (zoom 10 for the near level's 120 m at 35°, zoom 8 for the far level's 480 m: SRTM, GMTED2010 and
//!   ETOPO1 about the world, and at zoom 10 the national models the tiles hold), and EOX's Sentinel-2 cloudless 2016
//!   imagery (`surface::IMAGERY_URL`) at zooms 9 and 7.
//! - **Tiles** (what is kept on disk, under `surface::cache_dir()`'s `horizon-v1`): each tile as fetched, a marker
//!   where the source has none; the folder is held under [`CACHE_BYTES`], the least recently read going first. A first
//!   ring fetches about 200 tiles (20 MB); one recentred 40 km on, a few dozen.
//! - **Maps** ([`naval_terrain::HorizonMaps`]): each level's texels resampled from the tiles through the theatre's
//!   projection about the scenario's origin, made about a centre on a 3.84 km grid near the anchor, again once the
//!   anchor is [`RECENTRE_M`] from it. The latest asked for is made next; the last [`READY_KEPT`] made are kept.
//! - **Switched** with the surface streaming (`surface::enabled`), on the High terrain setting only
//!   (`TerrainQuality::horizon`); `NAVAL_HORIZON=off` leaves it off alone.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::{LatLon, to_lat_lon, wrap_lon};
use naval_terrain::HorizonMaps;
use naval_terrain::horizon::{COLOUR_TEXELS, FAR_HALF_M, HEIGHT_TEXELS, NEAR_HALF_M, SEA_FLOOR_M};

use super::surface::{self, FETCHERS, FetchError, Fetched};

/// What the game credits for the ring: the Terrain Tiles' datasets at its zooms (tilezen/joerd's attribution page:
/// SRTM, GMTED2010, ETOPO1 and 3DEP public domain, ArcticDEM unlicensed; at zoom 10 also the Environment Agency's under
/// the Open Government Licence v3, Kartverket's under CC BY 4.0, LINZ's under CC BY 3.0 NZ, Canada's CDEM under the
/// Open Government Licence – Canada, Austria's DGM under CC BY 3.0 AT, INEGI's under its free-use licence; EU-DEM's
/// Copernicus terms) and EOX's imagery (CC BY 4.0).
pub const ATTRIBUTION: [&str; 2] = [
    "Elevation: Terrain Tiles (Mapzen, tilezen/joerd) on the AWS Registry of Open Data: SRTM and GMTED2010 data \
     courtesy of the U.S. Geological Survey; Global ETOPO1 terrain data U.S. National Oceanic and Atmospheric \
     Administration; 3DEP data courtesy of the U.S. Geological Survey; DEM(s) were created from DigitalGlobe, Inc., \
     imagery and funded under National Science Foundation awards 1043681, 1559691, and 1542736 (ArcticDEM); \
     Produced using Copernicus data and information funded by the European Union - EU-DEM layers; © Environment \
     Agency copyright and/or database right 2015, all rights reserved; © Kartverket; Copyright 2011 Crown copyright \
     (c) Land Information New Zealand and the New Zealand Government, all rights reserved; Contains information \
     licensed under the Open Government Licence – Canada; © offene Daten Österreichs – Digitales Geländemodell (DGM) \
     Österreich; Source: INEGI, Continental relief, 2016",
    "Sentinel-2 cloudless - https://s2maps.eu by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016)",
];
/// The tile cache's bound on disk: past it the least recently read tiles go, down to three quarters of it.
pub const CACHE_BYTES: u64 = 256 << 20;
/// The maps are made again about the anchor once it is this far from their centre: the near level (123 km about it)
/// then still reaches past a sea window's edge on every side.
pub const RECENTRE_M: f64 = 40_000.0;
/// The grid the centres stand on (m): eight far texels, so the texels of maps made about neighbouring centres fall on
/// the same places.
pub const SNAP_M: f64 = 3_840.0;
/// Maps kept made: the one drawn and the one before it.
pub const READY_KEPT: usize = 2;
/// Maps whose making failed are not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// Tiles fetched between checks of the cache's size.
const PRUNE_EVERY: u64 = 128;
/// Texels between the points whose place on the globe is worked out exactly; those between are interpolated (well
/// under a metre's error over 1.9 km at the far level).
const NODE_TEXELS: usize = 16;
/// Pixels on a tile's side.
const TILE_PX: i64 = 256;
/// A level's tiles at most: a bound on what one level can ask of the network.
const MAX_TILES: usize = 400;
/// The imagery's grey where it has no tile (beyond 85° of latitude).
const MISSING_COLOUR: [u8; 3] = [96, 96, 96];

/// Whether the ring is drawn: with the surface streaming on (`surface::enabled`), unless `NAVAL_HORIZON` is `off` (or
/// `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_HORIZON").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

static SHOWN: AtomicBool = AtomicBool::new(true);

/// Draws or hides the ring without dropping its maps (for before-and-after captures and frame measurements): taken up
/// by the next frame's feed.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------------------------------------------------
// The sources.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Source {
    Heights,
    Imagery,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Heights => "terrarium",
            Self::Imagery => "s2cloudless-2016",
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Heights => "png",
            Self::Imagery => "jpg",
        }
    }

    fn url(self, z: u32, x: i64, y: i64) -> String {
        match self {
            Self::Heights => format!(
                "{}/{}",
                super::relief::TERRARIUM_URL,
                naval_region_bake::terrarium::tile_path(z, x, y)
            ),
            Self::Imagery => format!("{}/{z}/{y}/{x}.jpg", surface::IMAGERY_URL),
        }
    }

    /// A tile's samples: heights (m) for the Terrain Tiles, RGB bytes for the imagery.
    fn decode(self, bytes: &[u8]) -> Result<Samples, String> {
        match self {
            Self::Heights => naval_region_bake::terrarium::decode(bytes)
                .map(Samples::Heights)
                .map_err(|e| e.to_string()),
            Self::Imagery => {
                let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
                    .map_err(|e| e.to_string())?
                    .to_rgb8();
                if image.width() != TILE_PX as u32 || image.height() != TILE_PX as u32 {
                    return Err(format!("{}×{} pixels", image.width(), image.height()));
                }
                Ok(Samples::Colour(image.into_raw()))
            }
        }
    }
}

enum Samples {
    Heights(Vec<f32>),
    Colour(Vec<u8>),
}

type TileKey = (Source, u32, i64, i64);

// ---------------------------------------------------------------------------------------------------------------------
// The disk and the network.

fn dir() -> PathBuf {
    surface::cache_dir().join("horizon-v1")
}

fn tile_path(key: TileKey, extension: &str) -> PathBuf {
    let (source, z, x, y) = key;
    dir()
        .join(source.name())
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.{extension}"))
}

fn io_error(e: std::io::Error) -> FetchError {
    FetchError::Server(format!("the far land's cache: {e}"))
}

/// Bytes fetched this session, tiles fetched, and the cache's size at its last check (bytes).
static FETCHED_BYTES: AtomicU64 = AtomicU64::new(0);
static TILES_FETCHED: AtomicU64 = AtomicU64::new(0);
static CACHE_SIZE: AtomicU64 = AtomicU64::new(0);

/// Tile `key` (x wrapped onto the world), from the disk or the network; none where the source has none.
fn load_tile(key: TileKey) -> Result<Option<Samples>, FetchError> {
    let (source, z, x, y) = key;
    let absent = tile_path(key, "absent");
    if absent.exists() {
        super::relief::touch(&absent);
        return Ok(None);
    }
    let path = tile_path(key, source.extension());
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(samples) = source.decode(&bytes) {
            super::relief::touch(&path);
            return Ok(Some(samples));
        }
        // Damaged: fetched again.
        let _ = std::fs::remove_file(&path);
    }
    if surface::offline() {
        return Err(FetchError::Offline(
            "the network was missing a moment ago".into(),
        ));
    }
    let url = source.url(z, x, y);
    let found = match surface::get(&url, None)? {
        Fetched::Absent => {
            super::relief::write_file(&absent, b"").map_err(io_error)?;
            None
        }
        Fetched::Bytes(bytes) => {
            let samples = source
                .decode(&bytes)
                .map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
            super::relief::write_file(&path, &bytes).map_err(io_error)?;
            FETCHED_BYTES.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            CACHE_SIZE.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Some(samples)
        }
    };
    if TILES_FETCHED.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
        prune();
    }
    Ok(found)
}

/// Holds the cache to [`CACHE_BYTES`].
fn prune() {
    let (kept, _) = super::relief::prune_files(&dir(), CACHE_BYTES);
    CACHE_SIZE.store(kept, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------------------------------
// Resampling.

/// Where each texel of a level lies in web mercator: its pixel at zoom 0 (times 2^z at zoom z), worked out exactly every
/// [`NODE_TEXELS`] and interpolated between.
struct Places {
    nodes: usize,
    step: usize,
    px: Vec<[f64; 2]>,
}

impl Places {
    /// The places of the `texels`² texels of the square `half` about theatre point `centre` (`geo` the scenario's
    /// origin). Longitudes run on through the antimeridian from the centre's, so the square stays continuous.
    fn new(geo: LatLon, centre: [f64; 2], half: f64, texels: usize) -> Self {
        let step = NODE_TEXELS;
        let nodes = texels / step + 1;
        let cell = 2.0 * half / texels as f64;
        let middle = to_lat_lon(geo, centre).lon;
        let mut px = Vec::with_capacity(nodes * nodes);
        for b in 0..nodes {
            for a in 0..nodes {
                let p = [
                    centre[0] - half + (a * step) as f64 * cell,
                    centre[1] - half + (b * step) as f64 * cell,
                ];
                let at = to_lat_lon(geo, p);
                let lon = middle + wrap_lon(at.lon - middle);
                px.push(surface::mercator_px(at.lat, lon, 0));
            }
        }
        Self { nodes, step, px }
    }

    /// Texel (i, j)'s centre at zoom `z`, in pixels.
    fn at(&self, i: usize, j: usize, z: u32) -> [f64; 2] {
        let last = (self.nodes - 1) as f64;
        let u = ((i as f64 + 0.5) / self.step as f64).min(last);
        let v = ((j as f64 + 0.5) / self.step as f64).min(last);
        let (a, b) = (
            (u as usize).min(self.nodes - 2),
            (v as usize).min(self.nodes - 2),
        );
        let (s, t) = (u - a as f64, v - b as f64);
        let n = |a: usize, b: usize| self.px[b * self.nodes + a];
        let (p00, p10, p01, p11) = (n(a, b), n(a + 1, b), n(a, b + 1), n(a + 1, b + 1));
        let scale = f64::from(1u32 << z);
        let mix = |k: usize| {
            let top = p00[k] * (1.0 - s) + p10[k] * s;
            let bottom = p01[k] * (1.0 - s) + p11[k] * s;
            (top * (1.0 - t) + bottom * t) * scale
        };
        [mix(0), mix(1)]
    }

    /// The tiles at zoom `z` under the texels (a pixel's margin each way, for the bilinear reads).
    fn tiles(&self, z: u32) -> (i64, i64, i64, i64) {
        let scale = f64::from(1u32 << z);
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in &self.px {
            x0 = x0.min(p[0] * scale);
            y0 = y0.min(p[1] * scale);
            x1 = x1.max(p[0] * scale);
            y1 = y1.max(p[1] * scale);
        }
        let n = 1i64 << z;
        let tile = |v: f64| (v / TILE_PX as f64).floor() as i64;
        (
            tile(x0 - 1.0),
            tile(y0 - 1.0).clamp(0, n - 1),
            tile(x1 + 1.0),
            tile(y1 + 1.0).clamp(0, n - 1),
        )
    }
}

/// One source's tiles under a level at one zoom, read pixel by pixel across their edges.
struct Mosaic {
    z: u32,
    x0: i64,
    y0: i64,
    width: i64,
    height: i64,
    tiles: Vec<Option<Samples>>,
}

impl Mosaic {
    /// Reads (or fetches) every tile under `places` at zoom `z`, [`FETCHERS`] at a time.
    fn load(source: Source, places: &Places, z: u32) -> Result<Self, FetchError> {
        let (x0, y0, x1, y1) = places.tiles(z);
        let (width, height) = (x1 - x0 + 1, y1 - y0 + 1);
        let count = (width * height) as usize;
        if count > MAX_TILES {
            return Err(FetchError::Server(format!(
                "{count} {} tiles at zoom {z}",
                source.name()
            )));
        }
        let wrap = 1i64 << z;
        let next = AtomicUsize::new(0);
        let tiles: Mutex<Vec<Option<Samples>>> = Mutex::new((0..count).map(|_| None).collect());
        let failure: Mutex<Option<FetchError>> = Mutex::new(None);
        let qos = crate::host::Inherit::here();
        std::thread::scope(|scope| {
            for _ in 0..FETCHERS.min(count) {
                scope.spawn(|| {
                    qos.apply();
                    loop {
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        if k >= count || failure.lock().unwrap_or_else(|e| e.into_inner()).is_some()
                        {
                            break;
                        }
                        let (x, y) = (x0 + (k as i64 % width), y0 + (k as i64 / width));
                        match load_tile((source, z, x.rem_euclid(wrap), y)) {
                            Ok(samples) => {
                                tiles.lock().unwrap_or_else(|e| e.into_inner())[k] = samples;
                            }
                            Err(e) => {
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
        if let Some(e) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(e);
        }
        Ok(Self {
            z,
            x0,
            y0,
            width,
            height,
            tiles: tiles.into_inner().unwrap_or_else(|e| e.into_inner()),
        })
    }

    /// The tile holding global pixel (x, y), and the pixel's index in it; none outside the mosaic or where the source
    /// has none.
    fn pixel(&self, x: i64, y: i64) -> Option<(&Samples, usize)> {
        let (tx, ty) = (
            x.div_euclid(TILE_PX) - self.x0,
            y.div_euclid(TILE_PX) - self.y0,
        );
        if tx < 0 || ty < 0 || tx >= self.width || ty >= self.height {
            return None;
        }
        let tile = self.tiles[(ty * self.width + tx) as usize].as_ref()?;
        let (px, py) = (x.rem_euclid(TILE_PX), y.rem_euclid(TILE_PX));
        Some((tile, (py * TILE_PX + px) as usize))
    }

    /// The four pixels about `p` (pixel centres at half pixels) and their weights.
    fn corners(p: [f64; 2]) -> ([[i64; 2]; 4], [f64; 4]) {
        let (fx, fy) = (p[0] - 0.5, p[1] - 0.5);
        let (x, y) = (fx.floor(), fy.floor());
        let (s, t) = (fx - x, fy - y);
        let (x, y) = (x as i64, y as i64);
        (
            [[x, y], [x + 1, y], [x, y + 1], [x + 1, y + 1]],
            [(1.0 - s) * (1.0 - t), s * (1.0 - t), (1.0 - s) * t, s * t],
        )
    }

    /// The height (m) at global pixel `p`, bilinear; the open sea's floor where no tile holds it.
    fn height(&self, p: [f64; 2]) -> f32 {
        let (corners, weights) = Self::corners(p);
        let mut sum = 0.0;
        for (c, w) in corners.iter().zip(weights) {
            let h = match self.pixel(c[0], c[1]) {
                Some((Samples::Heights(h), k)) => f64::from(h[k]),
                _ => f64::from(SEA_FLOOR_M),
            };
            sum += h * w;
        }
        sum as f32
    }

    /// The colour (sRGB bytes) at global pixel `p`, bilinear; grey where no tile holds it.
    fn colour(&self, p: [f64; 2]) -> [u8; 3] {
        let (corners, weights) = Self::corners(p);
        let mut sum = [0.0f64; 3];
        for (c, w) in corners.iter().zip(weights) {
            let rgb = match self.pixel(c[0], c[1]) {
                Some((Samples::Colour(rgb), k)) => [rgb[k * 3], rgb[k * 3 + 1], rgb[k * 3 + 2]],
                _ => MISSING_COLOUR,
            };
            for (s, v) in sum.iter_mut().zip(rgb) {
                *s += f64::from(v) * w;
            }
        }
        sum.map(|v| v.round().clamp(0.0, 255.0) as u8)
    }
}

/// `f(j)` for each row `j` of `rows`, a few threads at a time, written into `out` (`row_len` values a row).
fn rows<T: Send + Copy + Default>(
    rows: usize,
    row_len: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) -> Vec<T> {
    let mut out = vec![T::default(); rows * row_len];
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 4);
    let chunk = rows.div_ceil(threads);
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for (c, slice) in out.chunks_mut(chunk * row_len).enumerate() {
            let f = &f;
            scope.spawn(move || {
                qos.apply();
                for (r, row) in slice.chunks_mut(row_len).enumerate() {
                    f(c * chunk + r, row);
                }
            });
        }
    });
    out
}

/// The zoom whose pixel is about `cell_m` at `lat` (the Terrain Tiles' rule, `terrarium::zoom_for`), at most `max`.
fn zoom(lat: f64, cell_m: f64, max: u32) -> u32 {
    naval_region_bake::terrarium::zoom_for(lat, cell_m).min(max)
}

/// The maps about theatre point `centre` in the scenario about `geo`: each level's heights (held to the open sea's
/// floor) and colour. Slow (fetches, then a second or so): on the worker.
pub fn make(geo: LatLon, centre: [f64; 2]) -> Result<HorizonMaps, FetchError> {
    let lat = to_lat_lon(geo, centre).lat;
    let mut heights = Vec::with_capacity(2);
    let mut colours = Vec::with_capacity(2);
    for half in [NEAR_HALF_M, FAR_HALF_M] {
        let n = HEIGHT_TEXELS as usize;
        let places = Places::new(geo, centre, half, n);
        let z = zoom(lat, 2.0 * half / n as f64, 12);
        let mosaic = Mosaic::load(Source::Heights, &places, z)?;
        heights.push(rows(n, n, |j, row: &mut [f32]| {
            for (i, h) in row.iter_mut().enumerate() {
                *h = mosaic.height(places.at(i, j, mosaic.z)).max(SEA_FLOOR_M);
            }
        }));
        let c = COLOUR_TEXELS as usize;
        let places = Places::new(geo, centre, half, c);
        let z = zoom(lat, 2.0 * half / c as f64, surface::MAX_ZOOM);
        let mosaic = Mosaic::load(Source::Imagery, &places, z)?;
        colours.push(rows(c, c * 4, |j, row: &mut [u8]| {
            for (i, texel) in row.chunks_mut(4).enumerate() {
                let [r, g, b] = mosaic.colour(places.at(i, j, mosaic.z));
                texel.copy_from_slice(&[r, g, b, 255]);
            }
        }));
    }
    let far_colour = colours.pop().expect("two levels");
    let near_colour = colours.pop().expect("two levels");
    let far = heights.pop().expect("two levels");
    let near = heights.pop().expect("two levels");
    Ok(HorizonMaps::new(
        centre,
        [near, far],
        [near_colour, far_colour],
    ))
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// Maps about one centre in one scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Key {
    geo: [u64; 2],
    centre: [i64; 2],
}

impl Key {
    /// The centre on [`SNAP_M`]'s grid nearest theatre point `at`.
    fn about(geo: LatLon, at: [f64; 2]) -> Self {
        Self {
            geo: [geo.lat.to_bits(), geo.lon.to_bits()],
            centre: [
                (at[0] / SNAP_M).round() as i64,
                (at[1] / SNAP_M).round() as i64,
            ],
        }
    }

    fn centre(self) -> [f64; 2] {
        [
            self.centre[0] as f64 * SNAP_M,
            self.centre[1] as f64 * SNAP_M,
        ]
    }

    fn geo(self) -> LatLon {
        LatLon {
            lat: f64::from_bits(self.geo[0]),
            lon: f64::from_bits(self.geo[1]),
        }
    }
}

/// What the worker is doing, for the RPC and the harness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    pub working: bool,
    /// Maps made this session, and how long the last took (ms: its tiles read or fetched and its texels made).
    pub made: usize,
    pub last_ms: u64,
    /// Tiles and bytes fetched from the network this session, and the disk cache's size at its last check.
    pub tiles_fetched: u64,
    pub fetched_bytes: u64,
    pub cache_bytes: u64,
    pub failed: usize,
}

struct State {
    wanted: Option<Key>,
    /// The maps the worker is making, so a frame asking for them meanwhile does not queue them again.
    making: Option<Key>,
    ready: Vec<(Key, Arc<HorizonMaps>)>,
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
            wanted: None,
            making: None,
            ready: Vec::new(),
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

/// The maps made in the scenario about `geo` whose centre lies nearest theatre point `at`.
fn nearest(state: &State, geo: LatLon, at: [f64; 2]) -> Option<(f64, Arc<HorizonMaps>)> {
    let bits = [geo.lat.to_bits(), geo.lon.to_bits()];
    state
        .ready
        .iter()
        .filter(|(k, _)| k.geo == bits)
        .map(|(_, maps)| {
            let d = (maps.centre[0] - at[0]).hypot(maps.centre[1] - at[1]);
            (d, maps.clone())
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The maps to draw for an anchor at theatre point `at` in the scenario about `geo`: the nearest made, while the worker
/// makes maps about the anchor once none made lies within [`RECENTRE_M`] (the latest asked for made next). None until
/// the first are made, and with the ring off.
pub fn maps_for(geo: LatLon, at: [f64; 2]) -> Option<Arc<HorizonMaps>> {
    if !enabled() {
        return None;
    }
    let mut state = lock();
    let found = nearest(&state, geo, at);
    if found.as_ref().is_some_and(|(d, _)| *d <= RECENTRE_M) {
        return found.map(|(_, maps)| maps);
    }
    let key = Key::about(geo, at);
    let now = Instant::now();
    state
        .failed
        .retain(|_, when| now.duration_since(*when) < RETRY_AFTER);
    if state.wanted != Some(key) && state.making != Some(key) && !state.failed.contains_key(&key) {
        state.wanted = Some(key);
        if !state.worker {
            state.worker = true;
            // Below the frame loop it would inherit and the simulation (`host`).
            if let Err(e) = crate::host::spawn_utility("theatre-horizon", work) {
                state.worker = false;
                eprintln!("the theatre's far land: no worker thread: {e}");
            }
        }
        streamer().wake.notify_one();
    }
    found.map(|(_, maps)| maps)
}

pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        failed: state.failed.len(),
        tiles_fetched: TILES_FETCHED.load(Ordering::Relaxed),
        fetched_bytes: FETCHED_BYTES.load(Ordering::Relaxed),
        cache_bytes: CACHE_SIZE.load(Ordering::Relaxed),
        ..state.status
    }
}

fn work() {
    // The cache held to its bound once a session before anything is read.
    prune();
    loop {
        let key = {
            let mut state = lock();
            loop {
                if let Some(key) = state.wanted.take() {
                    if state.ready.iter().any(|(k, _)| *k == key) {
                        continue;
                    }
                    state.status.working = true;
                    state.making = Some(key);
                    break key;
                }
                state.status.working = false;
                state = streamer()
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = Instant::now();
        let result = make(key.geo(), key.centre());
        let mut state = lock();
        state.status.working = false;
        state.making = None;
        match result {
            Ok(maps) => {
                state.status.made += 1;
                state.status.last_ms = started.elapsed().as_millis() as u64;
                state.ready.retain(|(k, _)| *k != key);
                state.ready.push((key, Arc::new(maps)));
                while state.ready.len() > READY_KEPT {
                    state.ready.remove(0);
                }
            }
            Err(e) => {
                if matches!(e, FetchError::Offline(_)) {
                    surface::mark_offline();
                }
                surface::log_once("horizon-failed", &format!("the far land: {e}"));
                state.failed.insert(key, Instant::now());
            }
        }
    }
}

/// The ring's status for the RPC: what the worker is doing and what the scene draws.
pub fn status_json(drawn: Option<naval_terrain::horizon::HorizonCounts>) -> serde_json::Value {
    let s = status();
    let mut value = serde_json::json!({
        "enabled": enabled(),
        "working": s.working,
        "made": s.made,
        "lastMs": s.last_ms,
        "tilesFetched": s.tiles_fetched,
        "fetchedBytes": s.fetched_bytes,
        "cacheBytes": s.cache_bytes,
        "cacheBound": CACHE_BYTES,
        "failed": s.failed,
    });
    if let Some(counts) = drawn {
        value["drawn"] = serde_json::json!({
            "drawn": counts.drawn,
            "uploading": counts.uploading,
            "patches": counts.patches,
            "gpuBytes": counts.bytes,
        });
    }
    value
}

/// The ring's tile cache folder (for the harness's report of its size).
pub fn cache_root() -> PathBuf {
    dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_interpolate_the_projection_and_run_on_through_the_antimeridian() {
        // Tarawa's neighbour across the date line: the square about a point near 180° stays continuous in x.
        let geo = LatLon {
            lat: 1.4,
            lon: 179.5,
        };
        let places = Places::new(geo, [0.0, 0.0], FAR_HALF_M, HEIGHT_TEXELS as usize);
        let z = 8;
        let n = HEIGHT_TEXELS as usize;
        let row: Vec<f64> = (0..n).map(|i| places.at(i, n / 2, z)[0]).collect();
        assert!(
            row.windows(2).all(|w| w[1] > w[0]),
            "x runs east without a jump"
        );
        // A texel between nodes lies where the projection puts it, to well under a texel.
        let (i, j) = (1000, 777);
        let cell = 2.0 * FAR_HALF_M / n as f64;
        let p = [
            -FAR_HALF_M + (i as f64 + 0.5) * cell,
            -FAR_HALF_M + (j as f64 + 0.5) * cell,
        ];
        let at = to_lat_lon(geo, p);
        let lon = geo.lon + wrap_lon(at.lon - geo.lon);
        let exact = surface::mercator_px(at.lat, lon, z);
        let got = places.at(i, j, z);
        assert!(
            (exact[0] - got[0]).abs() < 0.05 && (exact[1] - got[1]).abs() < 0.05,
            "{exact:?} {got:?}"
        );
        // The tiles under the far level at zoom 8 on the equator: about 8 a side.
        let (x0, y0, x1, y1) = places.tiles(z);
        assert!((7..=10).contains(&(x1 - x0 + 1)) && (7..=10).contains(&(y1 - y0 + 1)));
    }

    #[test]
    fn a_mosaic_reads_across_its_tiles_edges() {
        let tile = |base: f32| Some(Samples::Heights(vec![base; (TILE_PX * TILE_PX) as usize]));
        let mosaic = Mosaic {
            z: 3,
            x0: 2,
            y0: 5,
            width: 2,
            height: 1,
            tiles: vec![tile(100.0), tile(300.0)],
        };
        // Within a tile its own height; on the seam between pixel centres, halfway.
        assert_eq!(
            mosaic.height([2.0 * 256.0 + 10.0, 5.0 * 256.0 + 10.0]),
            100.0
        );
        assert_eq!(mosaic.height([3.0 * 256.0, 5.0 * 256.0 + 10.0]), 200.0);
        // Off the mosaic: the open sea's floor.
        assert_eq!(mosaic.height([0.0, 0.0]), SEA_FLOOR_M);
    }

    #[test]
    fn the_centre_snaps_and_recentres_well_inside_the_near_level() {
        let geo = LatLon {
            lat: 35.0,
            lon: 139.4,
        };
        let key = Key::about(geo, [10_000.0, -1_000.0]);
        assert_eq!(key.centre(), [3.0 * SNAP_M, 0.0]);
        assert_eq!(key.geo(), geo);
        // A sea window (48 km) about an anchor up to the recentring distance off the centre stays in the near level.
        const { assert!(RECENTRE_M + SNAP_M + 48_000.0 < NEAR_HALF_M - 32.0 * 120.0) };
        assert_eq!(SNAP_M % (2.0 * FAR_HALF_M / f64::from(HEIGHT_TEXELS)), 0.0);
    }
}
