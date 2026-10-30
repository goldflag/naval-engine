//! Fine relief near the camera (native/docs/terrain/ground.md, "Fine relief near the camera"): finer elevation models
//! than the
//! window's 40 m grid brought to the land layer's relief pages (`naval_terrain::relief`) about the camera, so cliffs,
//! gullies, crater rims and ridges stand as they are within a few kilometres. Presentation only, as the rest of the
//! theatre's land: the simulation reads the region's heights as before.
//!
//! - **The sources** ([`ATTRIBUTION`]), where they are finer than the window's (`REGIONS`): the AWS Terrain Tiles
//!   (Terrarium, [`TERRARIUM_URL`]) at zoom 15 in the United States (3DEP, 1–10 m), England (the Environment Agency's
//!   LIDAR), Norway (Kartverket's 10 m) and New Zealand (LINZ's 8 m); in Japan the Geospatial Information Authority's
//!   elevation tiles ([`GSI_URL`]), its 5 m model from airborne laser at zoom 15 and its 10 m model at zoom 14 where the
//!   5 m has none. Elsewhere none: the Terrain Tiles there are SRTM's, no finer than the window's own.
//! - **Tiles** (what is kept on disk, under `surface::cache_dir()`'s `relief-dem-v1`): each source's PNG as fetched, a
//!   marker where the source has none; the folder is held under [`CACHE_BYTES`], the least recently read going first.
//! - **Pages.** [`naval_terrain::relief::PAGE_TEXELS`]² texels of [`TEXEL_M`] (2,016 m of their own) fixed in the
//!   theatre's metres, [`SLOTS`]² about the camera (8 km each way), made only where the window has land and a source
//!   covers. A texel holds the finer model's height less the window grid's bilinear surface through the finer model's
//!   heights at the grid's samples ([`Lattice`]): a page is made for one window's lattice, again after each cut.
//! - **Synthetic** ([`Source::Synthetic`], `relief_synth`): detail made from the window's
//!   own heights (gullies down the fall lines, as deep as the slope and the land's range about), through the same
//!   residual. `NAVAL_RELIEF_SOURCE` (`auto`, `real`, `synthetic`, `glo30`, `combined`, `off`; [`mode`]) chooses for
//!   comparisons.
//! - **GLO-30** ([`Source::Glo30`], `relief_glo30`), a test source (`glo30` only, not in `auto`): Copernicus's 30 m
//!   surface model through the same residual, kept to open ground by WorldCover so canopy and roofs make no steps.
//! - **Combined** ([`Source::Combined`], `relief_combined`): GLO-30's band (its fills kept) for the shapes from about
//!   60 m up, the synthetic's detail under that on top, the synthetic alone where GLO-30 is masked or absent; what
//!   `auto` takes where no finer model covers.
//! - **Real or made**: a finer model's page is [`ReliefPage::real`] (its ledges are the ground's, so the land draws
//!   none of its own stepped beds under it); a synthetic, GLO-30 or combined page is not.
//! - **Switched** with the surface streaming (`surface::enabled`), on the High terrain setting only;
//!   `NAVAL_RELIEF=off` leaves it off alone.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use naval_earth::geo::{LatLon, to_lat_lon, wrap_lon};
use naval_terrain::field::PreparedField;
use naval_terrain::relief::{CONTENT, GUTTER, PAGE_TEXELS};
use naval_terrain::{ReliefGrid, ReliefPage};

use super::relief_combined;
use super::relief_glo30;
use super::relief_limit;
use super::relief_synth;
use super::surface::{self, FETCHERS, FetchError, Fetched};

/// The AWS Terrain Tiles (`{TERRARIUM_URL}/{z}/{x}/{y}.png`).
pub const TERRARIUM_URL: &str = naval_region_bake::terrarium::TERRARIUM_URL;
/// The Geospatial Information Authority of Japan's elevation tiles (`{GSI_URL}/{dem5a_png,dem_png}/{z}/{x}/{y}.png`).
pub const GSI_URL: &str = "https://cyberjapandata.gsi.go.jp/xyz";
/// The credits the sources ask for: the Terrain Tiles' (tilezen/joerd's attribution page, for the datasets drawn
/// here: 3DEP, SRTM, GMTED2010 and ETOPO1 public domain, the Environment Agency's under the Open Government Licence v3,
/// Kartverket's under CC BY 4.0, LINZ's under CC BY 3.0 NZ) and the Geospatial Information Authority of Japan's (its
/// content terms, the Public Data License 1.0: the source named, with the tile list's page, and the processing stated).
pub const ATTRIBUTION: [&str; 2] = [
    "Elevation: Terrain Tiles (Mapzen, tilezen/joerd) on the AWS Registry of Open Data: 3DEP data courtesy of the U.S. \
     Geological Survey; © Environment Agency copyright and/or database right 2015, all rights reserved; © Kartverket; \
     Copyright 2011 Crown copyright (c) Land Information New Zealand and the New Zealand Government, all rights \
     reserved; SRTM and GMTED2010 data courtesy of the U.S. Geological Survey; Global ETOPO1 terrain data U.S. National \
     Oceanic and Atmospheric Administration",
    "Elevation (Japan): 出典：国土地理院 標高タイル（加工して作成） (Geospatial Information Authority of Japan, elevation tiles, \
     https://maps.gsi.go.jp/development/ichiran.html)",
];
/// A page's texel (m).
pub const TEXEL_M: f64 = 4.0;
/// Pages a side the layer holds about the camera.
pub const SLOTS: u32 = 8;
/// Pages the worker makes together (of one row: they share their tiles).
pub const BATCH: usize = 4;
/// Made pages kept in memory (1.4 MB each, heights and slopes): the block and a row beyond.
pub const KEPT: usize = 72;
/// Decoded tiles kept in memory (256 KB each).
const TILES_KEPT: usize = 96;
/// The tile cache's bound on disk: past it the least recently read tiles go, down to three quarters of it.
pub const CACHE_BYTES: u64 = 512 << 20;
/// Tiles fetched between checks of the cache's size.
const PRUNE_EVERY: u64 = 256;
/// A page whose making failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// The lowest height a source gives that the relief takes (m): the land's relief stops at the sea, and the Terrain
/// Tiles hold spikes tens of metres deep just off some coasts.
const FLOOR_M: f32 = -5.0;
/// Pixels on a source tile's side.
const TILE_PX: i64 = 256;

/// Whether the relief is drawn: with the surface streaming on (`surface::enabled`), unless `NAVAL_RELIEF` is `off` (or
/// `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_RELIEF").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off && mode() != Mode::Off
}

/// Which sources the pages take (`NAVAL_RELIEF_SOURCE`): a finer model where one covers, else the combined (`auto`, the
/// default; the synthetic alone offline); a finer model only (`real`); the synthetic everywhere, a finer model's place
/// too (`synthetic`, to compare it with the real); GLO-30 everywhere (`glo30`, a test); the combined everywhere
/// (`combined`); none (`off`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Auto,
    Real,
    Synthetic,
    Glo30,
    Combined,
    Off,
}

impl Mode {
    pub fn parse(v: &str) -> Option<Self> {
        match v.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Some(Self::Auto),
            "real" => Some(Self::Real),
            "synthetic" | "synth" => Some(Self::Synthetic),
            "glo30" | "glo-30" => Some(Self::Glo30),
            "combined" => Some(Self::Combined),
            "off" | "0" | "none" => Some(Self::Off),
            _ => None,
        }
    }
}

/// The sources' mode, `NAVAL_RELIEF_SOURCE`'s (read once), `auto` without it.
pub fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("NAVAL_RELIEF_SOURCE") {
        Ok(v) => Mode::parse(&v).unwrap_or_else(|| {
            eprintln!(
                "NAVAL_RELIEF_SOURCE={v}: auto, real, synthetic, glo30, combined or off; auto"
            );
            Mode::Auto
        }),
        Err(_) => Mode::Auto,
    })
}

static SHOWN: AtomicBool = AtomicBool::new(true);

/// Draws or hides the relief without dropping its pages (for before-and-after captures and frame measurements): taken
/// up by the next frame's feed.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------------------------------
// The sources.

/// Which source a page reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The AWS Terrain Tiles at zoom 15.
    Terrarium,
    /// The GSI's 5 m model at zoom 15, its 10 m at zoom 14 where the 5 m has none.
    Gsi,
    /// Made from the window's own heights (`relief_synth`): no tiles.
    Synthetic,
    /// Copernicus GLO-30 on open ground (`relief_glo30`), a test: its own cells and WorldCover's tiles, none of these.
    Glo30,
    /// GLO-30's band (its fills kept) for the shapes from about 60 m up, the synthetic's detail under that on top, the
    /// synthetic alone where GLO-30 is masked or absent (`relief_combined`).
    Combined,
}

impl Source {
    /// Whether the source is a finer model's measurement of the ground ([`ReliefPage::real`]), not a page made.
    pub fn real(self) -> bool {
        matches!(self, Self::Terrarium | Self::Gsi)
    }

    /// Whether the source's pages are made from the window's heights where nothing finer is held (so made only where
    /// the window has relief, [`page_has_relief`]).
    pub fn synthetic(self) -> bool {
        matches!(self, Self::Synthetic | Self::Combined)
    }
}

/// Where a source finer than the window's covers (south, north, west, east, degrees): the Terrain Tiles' 3DEP over the
/// United States (the conterminous states, Hawaii, Puerto Rico and the Virgin Islands), the Environment Agency's LIDAR
/// over England, Kartverket's over Norway, LINZ's over New Zealand; the GSI's over Japan. A box takes a neighbour's
/// coarser data at its edges (Wales, Sweden, Canada, Mexico): no worse than the window's.
pub const REGIONS: &[([f64; 4], Source)] = &[
    ([24.4, 49.4, -124.9, -66.9], Source::Terrarium),
    ([18.8, 22.4, -160.4, -154.7], Source::Terrarium),
    ([17.6, 18.6, -67.4, -64.5], Source::Terrarium),
    ([49.9, 55.85, -5.8, 1.8], Source::Terrarium),
    ([57.9, 71.3, 4.4, 31.2], Source::Terrarium),
    ([-47.4, -34.3, 166.3, 178.7], Source::Terrarium),
    ([24.0, 45.6, 122.9, 146.0], Source::Gsi),
];

/// The source covering a latitude and longitude, if any.
pub fn source_at(lat: f64, lon: f64) -> Option<Source> {
    let lon = wrap_lon(lon);
    REGIONS
        .iter()
        .find(|([s, n, w, e], _)| (*s..=*n).contains(&lat) && (*w..=*e).contains(&lon))
        .map(|(_, source)| *source)
}

/// A dataset's tiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Dataset {
    Terrarium,
    Gsi5m,
    Gsi10m,
}

impl Dataset {
    pub fn zoom(self) -> u32 {
        match self {
            Self::Terrarium | Self::Gsi5m => 15,
            Self::Gsi10m => 14,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Terrarium => "terrarium",
            Self::Gsi5m => "gsi-dem5a",
            Self::Gsi10m => "gsi-dem",
        }
    }

    fn url(self, x: i64, y: i64) -> String {
        let z = self.zoom();
        match self {
            Self::Terrarium => format!("{TERRARIUM_URL}/{z}/{x}/{y}.png"),
            Self::Gsi5m => format!("{GSI_URL}/dem5a_png/{z}/{x}/{y}.png"),
            Self::Gsi10m => format!("{GSI_URL}/dem_png/{z}/{x}/{y}.png"),
        }
    }

    /// A tile's heights (m, row-major; NaN where the dataset has none).
    fn decode(self, bytes: &[u8]) -> Result<Vec<f32>, String> {
        match self {
            Self::Terrarium => {
                naval_region_bake::terrarium::decode(bytes).map_err(|e| e.to_string())
            }
            Self::Gsi5m | Self::Gsi10m => {
                naval_region_bake::terrarium::rgb(bytes).map(|rgb| gsi_heights(&rgb))
            }
        }
    }
}

/// The GSI's elevation PNG's RGB as metres: 2¹⁶ R + 2⁸ G + B hundredths of a metre, two's complement in 24 bits, 2²³
/// (128, 0, 0) where it has none (NaN).
pub fn gsi_heights(rgb: &[u8]) -> Vec<f32> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let v = (i32::from(p[0]) << 16) | (i32::from(p[1]) << 8) | i32::from(p[2]);
            match v.cmp(&(1 << 23)) {
                std::cmp::Ordering::Less => v as f32 * 0.01,
                std::cmp::Ordering::Equal => f32::NAN,
                std::cmp::Ordering::Greater => (v - (1 << 24)) as f32 * 0.01,
            }
        })
        .collect()
}

/// A tile: its dataset, and x (wrapped round the world) and y at the dataset's zoom.
pub type TileKey = (Dataset, i64, i64);

fn tile_key(dataset: Dataset, x: i64, y: i64) -> TileKey {
    (dataset, x.rem_euclid(1 << dataset.zoom()), y)
}

/// A tile's heights, none where the dataset has no tile.
pub type Tile = Option<Arc<Vec<f32>>>;

// ---------------------------------------------------------------------------------------------------------------------
// The disk and the network.

pub(crate) fn dir() -> PathBuf {
    surface::cache_dir().join("relief-dem-v1")
}

fn tile_path(key: TileKey, extension: &str) -> PathBuf {
    let (dataset, x, y) = key;
    dir()
        .join(dataset.name())
        .join(dataset.zoom().to_string())
        .join(x.to_string())
        .join(format!("{y}.{extension}"))
}

/// Written whole, then moved into place: a reader never sees half a file.
pub fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.part", std::process::id()));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

fn io_error(e: std::io::Error) -> FetchError {
    FetchError::Server(format!("the relief's cache: {e}"))
}

/// Marks a file read now, for the cache's pruning (at most daily, so reading writes little).
pub fn touch(path: &std::path::Path) {
    let day = Duration::from_secs(86_400);
    let stale = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > day));
    if stale && let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

/// Bytes fetched this session, tiles fetched, and the cache's size at its last check (bytes).
static FETCHED_BYTES: AtomicU64 = AtomicU64::new(0);
static TILES_FETCHED: AtomicU64 = AtomicU64::new(0);
static CACHE_SIZE: AtomicU64 = AtomicU64::new(0);

/// Tile `key`, from the disk or the network.
fn load_tile(key: TileKey) -> Result<Tile, FetchError> {
    let (dataset, x, y) = key;
    let absent = tile_path(key, "absent");
    if absent.exists() {
        touch(&absent);
        return Ok(None);
    }
    let path = tile_path(key, "png");
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(heights) = dataset.decode(&bytes) {
            touch(&path);
            return Ok(Some(Arc::new(heights)));
        }
        // Damaged: fetched again.
        let _ = std::fs::remove_file(&path);
    }
    if surface::offline() {
        return Err(FetchError::Offline(
            "the network was missing a moment ago".into(),
        ));
    }
    let url = dataset.url(x, y);
    let found = match surface::get(&url, None)? {
        Fetched::Absent => {
            write_file(&absent, b"").map_err(io_error)?;
            None
        }
        Fetched::Bytes(bytes) => {
            let heights = dataset
                .decode(&bytes)
                .map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
            write_file(&path, &bytes).map_err(io_error)?;
            FETCHED_BYTES.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            CACHE_SIZE.fetch_add(bytes.len() as u64, Ordering::Relaxed);
            Some(Arc::new(heights))
        }
    };
    if TILES_FETCHED.fetch_add(1, Ordering::Relaxed) % PRUNE_EVERY == PRUNE_EVERY - 1 {
        prune(&dir(), CACHE_BYTES);
    }
    Ok(found)
}

/// Holds the relief's files under `root` to `bound` bytes ([`prune_files`]), and notes the bytes kept for the status.
pub fn prune(root: &std::path::Path, bound: u64) -> (u64, usize) {
    let (kept, removed) = prune_files(root, bound);
    CACHE_SIZE.store(kept, Ordering::Relaxed);
    (kept, removed)
}

/// Holds the files under `root` to `bound` bytes: when they come to more, the least recently modified (read: `touch`)
/// go until they come to three quarters of it. The bytes kept, and how many went.
pub fn prune_files(root: &std::path::Path, bound: u64) -> (u64, usize) {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((modified, meta.len(), entry.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    let mut removed = 0;
    if total > bound {
        files.sort_by_key(|f| f.0);
        for (_, len, path) in files {
            if total <= bound / 4 * 3 {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
                removed += 1;
            }
        }
    }
    (total, removed)
}

/// Decoded tiles kept in memory, the most recently used last.
static TILE_CACHE: Mutex<Vec<(TileKey, Tile)>> = Mutex::new(Vec::new());

fn keep_tile(key: TileKey, tile: Tile) {
    let mut kept = TILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(k, _)| *k != key);
    kept.push((key, tile));
    if kept.len() > TILES_KEPT {
        let extra = kept.len() - TILES_KEPT;
        kept.drain(..extra);
    }
}

/// The tiles `keys`, from memory, the disk or the network, a few at once.
pub fn load_tiles(keys: &BTreeSet<TileKey>) -> Result<HashMap<TileKey, Tile>, FetchError> {
    let mut found = HashMap::new();
    let mut missing = Vec::new();
    {
        let mut kept = TILE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        for &key in keys {
            if let Some(at) = kept.iter().position(|(k, _)| *k == key) {
                let entry = kept.remove(at);
                found.insert(key, entry.1.clone());
                kept.push(entry);
            } else {
                missing.push(key);
            }
        }
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(TileKey, Result<Tile, FetchError>)>> = Mutex::new(Vec::new());
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..FETCHERS.min(missing.len()) {
            scope.spawn(|| {
                qos.apply();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&key) = missing.get(k) else {
                        break;
                    };
                    let result = load_tile(key);
                    results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((key, result));
                }
            });
        }
    });
    for (key, result) in results.into_inner().unwrap_or_else(|e| e.into_inner()) {
        let tile = result?;
        keep_tile(key, tile.clone());
        found.insert(key, tile);
    }
    Ok(found)
}

// ---------------------------------------------------------------------------------------------------------------------
// Pages.

/// A page's side, metres (its own texels').
pub fn page_m() -> f64 {
    TEXEL_M * f64::from(CONTENT)
}

/// The page holding theatre point `p`.
pub fn page_of(p: [f64; 2]) -> [i64; 2] {
    let side = page_m();
    [(p[0] / side).floor() as i64, (p[1] / side).floor() as i64]
}

/// The block of `slots`² pages about theatre point `at`, nearest first: the camera in its middle two pages a side.
pub fn block(at: [f64; 2], slots: u32) -> Vec<[i64; 2]> {
    let side = page_m();
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

/// The grid on a land layer whose chart point (0, 0) is theatre point `origin`, and the page its page (0, 0) is.
pub fn grid(origin: [f64; 2]) -> (ReliefGrid, [i64; 2]) {
    let base = page_of(origin);
    let side = page_m();
    let grid = ReliefGrid {
        origin: [
            base[0] as f64 * side - origin[0],
            base[1] as f64 * side - origin[1],
        ],
        texel_m: TEXEL_M,
        slots: SLOTS,
    };
    (grid, base)
}

/// The window's sample lattice in the theatre's metres: a sample at `node`, the samples `cell` apart.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lattice {
    pub node: [f64; 2],
    pub cell: f64,
}

impl Lattice {
    /// The lattice of a land layer whose chart point (0, 0) is theatre point `origin`.
    pub fn of(field: &naval_terrain::field::PreparedField, origin: [f64; 2]) -> Self {
        Self {
            node: [
                origin[0] + f64::from(field.origin[0]),
                origin[1] + f64::from(field.origin[1]),
            ],
            cell: field.grid.cell,
        }
    }

    /// The same lattice whichever sample it is named by: millimetres.
    fn key(&self) -> [i64; 3] {
        let mm = |v: f64| (v * 1000.0).round() as i64;
        [
            mm(self.node[0].rem_euclid(self.cell)),
            mm(self.node[1].rem_euclid(self.cell)),
            mm(self.cell),
        ]
    }
}

/// Page `index`'s map from its metres (x east, z south from its corner) to global zoom-15 pixels: bilinear between its
/// corners' (as the canopy's pages), carried past them for its gutters and the lattice samples about it.
struct PageMap {
    corners: [[f64; 2]; 4],
}

impl PageMap {
    fn new(geo: LatLon, index: [i64; 2]) -> Self {
        let side = page_m();
        let at = |di: f64, dj: f64| {
            let p = to_lat_lon(
                geo,
                [(index[0] as f64 + di) * side, (index[1] as f64 + dj) * side],
            );
            surface::mercator_px(p.lat, geo.lon + wrap_lon(p.lon - geo.lon), 15)
        };
        Self {
            corners: [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)],
        }
    }

    fn px(&self, m: [f64; 2]) -> [f64; 2] {
        let side = page_m();
        let (u, v) = (m[0] / side, m[1] / side);
        let [a, b, c, d] = self.corners;
        std::array::from_fn(|k| {
            (1.0 - u) * (1.0 - v) * a[k]
                + u * (1.0 - v) * b[k]
                + (1.0 - u) * v * c[k]
                + u * v * d[k]
        })
    }

    /// The zoom-15 pixels (inclusive) under the page's metres from `lo` to `hi` each way, a pixel more about them.
    fn pixels(&self, lo: f64, hi: f64) -> ([i64; 2], [i64; 2]) {
        let edge = [[lo, lo], [hi, lo], [lo, hi], [hi, hi]].map(|m| self.px(m));
        let low = [0, 1].map(|k| {
            edge.iter()
                .map(|p| p[k])
                .fold(f64::INFINITY, f64::min)
                .floor() as i64
                - 1
        });
        let high = [0, 1].map(|k| {
            edge.iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
                .ceil() as i64
                + 1
        });
        (low, high)
    }
}

/// A dataset's pixels over a rectangle of its global pixels (columns unwrapped), NaN where it has none.
struct Raster {
    first: [i64; 2],
    width: usize,
    height: usize,
    values: Vec<f32>,
}

impl Raster {
    /// The rectangle `lo` to `hi` (inclusive, zoom 15's pixels) at `dataset`'s zoom.
    fn tiles(dataset: Dataset, lo: [i64; 2], hi: [i64; 2]) -> BTreeSet<TileKey> {
        let shift = 15 - dataset.zoom();
        let (lo, hi) = (lo.map(|p| p >> shift), hi.map(|p| p >> shift));
        let mut out = BTreeSet::new();
        for ty in lo[1].div_euclid(TILE_PX)..=hi[1].div_euclid(TILE_PX) {
            for tx in lo[0].div_euclid(TILE_PX)..=hi[0].div_euclid(TILE_PX) {
                out.insert(tile_key(dataset, tx, ty));
            }
        }
        out
    }

    fn new(dataset: Dataset, lo: [i64; 2], hi: [i64; 2], tiles: &HashMap<TileKey, Tile>) -> Self {
        let shift = 15 - dataset.zoom();
        let (lo, hi) = (lo.map(|p| p >> shift), hi.map(|p| p >> shift));
        let (width, height) = ((hi[0] - lo[0] + 1) as usize, (hi[1] - lo[1] + 1) as usize);
        let mut values = vec![f32::NAN; width * height];
        let rows = 1i64 << (dataset.zoom() + 8);
        for y in 0..height {
            let gy = lo[1] + y as i64;
            if !(0..rows).contains(&gy) {
                continue;
            }
            let mut last: Option<(TileKey, Option<&Arc<Vec<f32>>>)> = None;
            for x in 0..width {
                let gx = lo[0] + x as i64;
                let key = tile_key(dataset, gx.div_euclid(TILE_PX), gy.div_euclid(TILE_PX));
                let tile = match last {
                    Some((k, t)) if k == key => t,
                    _ => {
                        let t = tiles.get(&key).and_then(|t| t.as_ref());
                        last = Some((key, t));
                        t
                    }
                };
                if let Some(tile) = tile {
                    let (px, py) = (gx.rem_euclid(TILE_PX), gy.rem_euclid(TILE_PX));
                    values[y * width + x] = tile[(py * TILE_PX + px) as usize];
                }
            }
        }
        Self {
            first: lo,
            width,
            height,
            values,
        }
    }

    /// The bilinear height at global pixel `px` of the raster's zoom (pixel middles at half pixels), NaN where any of
    /// the four is none or outside.
    fn sample(&self, px: [f64; 2]) -> f32 {
        let x = px[0] - 0.5 - self.first[0] as f64;
        let y = px[1] - 0.5 - self.first[1] as f64;
        let (x0, y0) = (x.floor(), y.floor());
        if x0 < 0.0 || y0 < 0.0 || x0 + 1.0 >= self.width as f64 || y0 + 1.0 >= self.height as f64 {
            return f32::NAN;
        }
        let (u, v) = ((x - x0) as f32, (y - y0) as f32);
        let (i, j) = (x0 as usize, y0 as usize);
        let at = |i: usize, j: usize| self.values[j * self.width + i];
        let top = at(i, j) * (1.0 - u) + at(i + 1, j) * u;
        let bottom = at(i, j + 1) * (1.0 - u) + at(i + 1, j + 1) * u;
        top * (1.0 - v) + bottom * v
    }
}

/// What a page reads: its source's rasters over the page and its margin.
struct Heights {
    source: Source,
    rasters: Vec<Raster>,
}

impl Heights {
    fn datasets(source: Source) -> &'static [Dataset] {
        match source {
            Source::Terrarium => &[Dataset::Terrarium],
            Source::Gsi => &[Dataset::Gsi5m, Dataset::Gsi10m],
            Source::Synthetic | Source::Glo30 | Source::Combined => &[],
        }
    }

    /// The height at global zoom-15 pixel `px` (m): the GSI's 5 m model, else its 10 m; the sea (0) where none has
    /// any; never below [`FLOOR_M`].
    fn at(&self, px: [f64; 2]) -> f32 {
        let h = match self.source {
            Source::Terrarium => self.rasters[0].sample(px),
            Source::Synthetic | Source::Glo30 | Source::Combined => f32::NAN,
            Source::Gsi => {
                let fine = self.rasters[0].sample(px);
                if fine.is_finite() {
                    fine
                } else {
                    self.rasters[1].sample([px[0] / 2.0, px[1] / 2.0])
                }
            }
        };
        if h.is_finite() { h.max(FLOOR_M) } else { 0.0 }
    }
}

/// The metres about a page's own the page reads: its gutter, and a lattice cell beyond for the samples about it.
pub(crate) fn reach(lattice: &Lattice) -> (f64, f64) {
    let g = f64::from(GUTTER) * TEXEL_M;
    (-g - lattice.cell, page_m() + g + lattice.cell)
}

/// The tiles page `index` about `geo` reads from `source`.
pub fn page_tiles(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    source: Source,
) -> BTreeSet<TileKey> {
    let map = PageMap::new(geo, index);
    let (lo, hi) = reach(lattice);
    let (low, high) = map.pixels(lo, hi);
    Heights::datasets(source)
        .iter()
        .flat_map(|&d| Raster::tiles(d, low, high))
        .collect()
}

/// Page `index` about `geo` on `lattice` from `source`'s `tiles`: each texel the finer model's height less the
/// lattice's bilinear surface through the finer model's heights at its samples.
pub fn make_page(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    source: Source,
    tiles: &HashMap<TileKey, Tile>,
) -> ReliefPage {
    let map = PageMap::new(geo, index);
    let (lo, hi) = reach(lattice);
    let (low, high) = map.pixels(lo, hi);
    let heights = Heights {
        source,
        rasters: Heights::datasets(source)
            .iter()
            .map(|&d| Raster::new(d, low, high, tiles))
            .collect(),
    };
    let texels = residual_texels(lattice, index, &|m| heights.at(map.px(m)));
    ReliefPage::from_residual(&texels, TEXEL_M as f32)
}

/// Page `index`'s texels (with its gutters) on `lattice` from a finer model's heights at the page's metres (x east, z
/// south from its corner; `fine`): each the model's height less the lattice's bilinear surface through the model's
/// heights at its samples, so zero at the samples and agreeing with a neighbour's in the gutters.
pub(crate) fn residual_texels(
    lattice: &Lattice,
    index: [i64; 2],
    fine: &dyn Fn([f64; 2]) -> f32,
) -> Vec<f32> {
    residual_grid(lattice, index, fine, 0)
}

/// [`residual_texels`] over the page's texels and `extra` more each way (`PAGE_TEXELS + 2 extra` a side).
pub(crate) fn residual_grid(
    lattice: &Lattice,
    index: [i64; 2],
    fine: &dyn Fn([f64; 2]) -> f32,
    extra: usize,
) -> Vec<f32> {
    let (lo, hi) = reach(lattice);
    let wide = extra as f64 * TEXEL_M;
    let (lo, hi) = (lo - wide, hi + wide);
    let side = page_m();
    let corner = [index[0] as f64 * side, index[1] as f64 * side];
    // The lattice's samples about the page: k from `first` (theatre node + k × cell), `count` a side.
    let first = [0, 1].map(|k| ((corner[k] + lo - lattice.node[k]) / lattice.cell).floor() as i64);
    let last = [0, 1].map(|k| ((corner[k] + hi - lattice.node[k]) / lattice.cell).ceil() as i64);
    let count = [0, 1].map(|k| (last[k] - first[k] + 1) as usize);
    let mut nodes = vec![0f32; count[0] * count[1]];
    for j in 0..count[1] {
        for i in 0..count[0] {
            let p = [
                lattice.node[0] + (first[0] + i as i64) as f64 * lattice.cell,
                lattice.node[1] + (first[1] + j as i64) as f64 * lattice.cell,
            ];
            nodes[j * count[0] + i] = fine([p[0] - corner[0], p[1] - corner[1]]);
        }
    }
    let n = PAGE_TEXELS as usize + 2 * extra;
    let g = f64::from(GUTTER) + extra as f64;
    let mut texels = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            let m = [
                (i as f64 - g + 0.5) * TEXEL_M,
                (j as f64 - g + 0.5) * TEXEL_M,
            ];
            let h = fine(m);
            let t = [0, 1]
                .map(|k| (corner[k] + m[k] - lattice.node[k]) / lattice.cell - first[k] as f64);
            let (a, b) = (t[0].floor(), t[1].floor());
            let (u, v) = ((t[0] - a) as f32, (t[1] - b) as f32);
            let (a, b) = (
                (a as usize).min(count[0] - 2),
                (b as usize).min(count[1] - 2),
            );
            let at = |i: usize, j: usize| nodes[j * count[0] + i];
            let surface = (at(a, b) * (1.0 - u) + at(a + 1, b) * u) * (1.0 - v)
                + (at(a, b + 1) * (1.0 - u) + at(a + 1, b + 1) * u) * v;
            texels[j * n + i] = h - surface;
        }
    }
    texels
}

/// The source page `index` about `geo` takes ([`mode`]): the finer model covering its middle, else the synthetic.
pub fn page_source(geo: LatLon, index: [i64; 2]) -> Option<Source> {
    page_source_in(mode(), geo, index)
}

/// [`page_source`] in `mode`.
pub fn page_source_in(mode: Mode, geo: LatLon, index: [i64; 2]) -> Option<Source> {
    let real = || {
        let side = page_m();
        let p = to_lat_lon(
            geo,
            [
                (index[0] as f64 + 0.5) * side,
                (index[1] as f64 + 0.5) * side,
            ],
        );
        source_at(p.lat, p.lon)
    };
    match mode {
        Mode::Auto => real().or(Some(Source::Combined)),
        Mode::Real => real(),
        Mode::Synthetic => Some(Source::Synthetic),
        Mode::Glo30 => Some(Source::Glo30),
        Mode::Combined => Some(Source::Combined),
        Mode::Off => None,
    }
}

/// Whether the land layer's field (its chart's (0, 0) at theatre point `origin`) has land on page `index`: any block of
/// its bounds' finest level over the page rising above the sea.
pub fn page_has_land(
    field: &naval_terrain::field::PreparedField,
    origin: [f64; 2],
    index: [i64; 2],
) -> bool {
    let Some(level) = field.bounds.levels.first() else {
        return false;
    };
    let side = page_m();
    let block = field.bounds.base as f64 * field.grid.cell;
    let chart = [0, 1].map(|k| index[k] as f64 * side - origin[k] - f64::from(field.origin[k]));
    let first = chart.map(|c| (c / block).floor().max(0.0) as usize);
    let last = chart.map(|c| ((c + side) / block).floor());
    if last[0] < 0.0 || last[1] < 0.0 {
        return false;
    }
    let last = last.map(|l| (l as usize).min(level.size.saturating_sub(1)));
    (first[1]..=last[1]).any(|j| (first[0]..=last[0]).any(|i| level.max[j * level.size + i] > 0.0))
}

/// Whether the field has land on page `index` rising [`SYNTHETIC_RANGE_M`] or more within one of its bounds' finest
/// blocks (320 m on a 40 m window): what a synthetic page needs to hold anything.
pub fn page_has_relief(field: &PreparedField, origin: [f64; 2], index: [i64; 2]) -> bool {
    let Some(level) = field.bounds.levels.first() else {
        return false;
    };
    let side = page_m();
    let block = field.bounds.base as f64 * field.grid.cell;
    let chart = [0, 1].map(|k| index[k] as f64 * side - origin[k] - f64::from(field.origin[k]));
    let first = chart.map(|c| (c / block).floor().max(0.0) as usize);
    let last = chart.map(|c| ((c + side) / block).floor());
    if last[0] < 0.0 || last[1] < 0.0 {
        return false;
    }
    let last = last.map(|l| (l as usize).min(level.size.saturating_sub(1)));
    (first[1]..=last[1]).any(|j| {
        (first[0]..=last[0]).any(|i| {
            let k = j * level.size + i;
            level.max[k] > 0.0 && level.max[k] - level.min[k].max(0.0) >= SYNTHETIC_RANGE_M
        })
    })
}

/// The least rise within a block of a page's land for a synthetic page (m).
pub const SYNTHETIC_RANGE_M: f32 = 20.0;

/// The window's heights a synthetic page reads: the land layer's field, its chart's (0, 0) at theatre point `origin`.
#[derive(Clone)]
pub struct Coarse {
    pub field: Arc<PreparedField>,
    pub origin: [f64; 2],
}

impl Coarse {
    /// The field's height at theatre point `p` (m).
    pub fn height(&self, p: [f64; 2]) -> f32 {
        self.field
            .height(p[0] - self.origin[0], p[1] - self.origin[1]) as f32
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// A page about a scenario's frame on a lattice: the frame, the lattice and the page.
type Key = ([u64; 2], [i64; 3], [i64; 2]);

fn key(geo: LatLon, lattice: &Lattice, index: [i64; 2]) -> Key {
    ([geo.lat.to_bits(), geo.lon.to_bits()], lattice.key(), index)
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    pub queued: usize,
    pub working: usize,
    /// Pages made this session, and how long the last batch took (ms: its tiles read and its pages made).
    pub made: usize,
    pub last_batch_ms: u64,
    /// Tiles and bytes fetched from the network this session, and the disk cache's size at its last check.
    pub tiles_fetched: u64,
    pub fetched_bytes: u64,
    pub cache_bytes: u64,
    pub failed: usize,
}

struct State {
    job: Option<(LatLon, Lattice, Option<Coarse>)>,
    wanted: VecDeque<[i64; 2]>,
    /// Pages made: none where the source holds nothing there.
    made: HashMap<Key, Option<Arc<ReliefPage>>>,
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
            job: None,
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

/// Page `index` about `geo` on `lattice`, if made: `Some(None)` where the source holds nothing there.
pub fn made(geo: LatLon, lattice: &Lattice, index: [i64; 2]) -> Option<Option<Arc<ReliefPage>>> {
    lock().made.get(&key(geo, lattice, index)).cloned()
}

/// Asks for `pages` (in the order given) about `geo` on `lattice`, in place of whatever was asked before; a page made,
/// or failed lately, is left out. `coarse`: the window's heights, for synthetic pages.
pub fn want(geo: LatLon, lattice: Lattice, coarse: Option<Coarse>, pages: &[[i64; 2]]) {
    if !enabled() {
        return;
    }
    let mut state = lock();
    let now = Instant::now();
    state
        .failed
        .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    let wanted: VecDeque<_> = pages
        .iter()
        .copied()
        .filter(|&i| {
            let k = key(geo, &lattice, i);
            !state.made.contains_key(&k) && !state.failed.contains_key(&k)
        })
        .collect();
    state.job = Some((geo, lattice, coarse));
    state.status.queued = wanted.len();
    state.wanted = wanted;
    if state.wanted.is_empty() {
        return;
    }
    if !state.worker {
        state.worker = true;
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-relief", work);
        if let Err(e) = spawned {
            state.worker = false;
            eprintln!("the theatre's relief: no worker thread: {e}");
        }
    }
    streamer().wake.notify_one();
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
    prune(&dir(), CACHE_BYTES);
    loop {
        let (geo, lattice, coarse, batch) = {
            let mut state = lock();
            loop {
                if let Some((geo, lattice, coarse)) = state.job.clone()
                    && let Some(first) = state.wanted.pop_front()
                {
                    // The nearest page, and the others asked for on its row (they share their tiles).
                    let mut batch = vec![first];
                    let mut rest = VecDeque::with_capacity(state.wanted.len());
                    while let Some(i) = state.wanted.pop_front() {
                        if i[1] == first[1] && batch.len() < BATCH {
                            batch.push(i);
                        } else {
                            rest.push_back(i);
                        }
                    }
                    state.wanted = rest;
                    batch.retain(|&i| !state.made.contains_key(&key(geo, &lattice, i)));
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (geo, lattice, coarse, batch);
                    }
                    continue;
                }
                state.status.working = 0;
                state = streamer()
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = Instant::now();
        let result = make_batch(geo, &lattice, coarse.as_ref(), &batch);
        let mut state = lock();
        state.status.working = 0;
        match result {
            Ok(pages) => {
                state.status.last_batch_ms = started.elapsed().as_millis() as u64;
                for (i, page) in pages {
                    let k = key(geo, &lattice, i);
                    state.status.made += 1;
                    if state.made.insert(k, page).is_none() {
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
                surface::log_once("relief-failed", &format!("the relief: {e}"));
                // The batch waits a minute; what was asked after it would most likely fail the same way.
                let now = Instant::now();
                for &i in &batch {
                    state.failed.insert(key(geo, &lattice, i), now);
                }
                state.wanted.clear();
                state.status.queued = 0;
            }
        }
    }
}

/// Reads the tiles under the batch's pages and makes them, a few threads at a time.
#[allow(clippy::type_complexity)]
fn make_batch(
    geo: LatLon,
    lattice: &Lattice,
    coarse: Option<&Coarse>,
    batch: &[[i64; 2]],
) -> Result<Vec<([i64; 2], Option<Arc<ReliefPage>>)>, FetchError> {
    let sources: Vec<Option<Source>> = batch.iter().map(|&i| page_source(geo, i)).collect();
    let mut all = BTreeSet::new();
    for (&i, source) in batch.iter().zip(&sources) {
        if let Some(source) = source {
            all.extend(page_tiles(geo, lattice, i, *source));
        }
    }
    let tiles = load_tiles(&all)?;
    // GLO-30's cells and WorldCover's tiles under each GLO-30 page, read before the pages are made.
    let glo30 = batch
        .iter()
        .zip(&sources)
        .map(|(&i, source)| match source {
            Some(Source::Glo30) => relief_glo30::Inputs::load(geo, lattice, i).map(Some),
            // The combined source's GLO-30, its band with the fills and their mask; offline, the synthetic alone.
            Some(Source::Combined) => {
                match relief_glo30::Inputs::load_with(geo, lattice, i, relief_combined::GLO30, true)
                {
                    Ok(inputs) => Ok(Some(inputs)),
                    Err(FetchError::Offline(e)) => {
                        surface::log_once(
                            "relief-combined-offline",
                            &format!("the relief's GLO-30: {e}"),
                        );
                        Ok(None)
                    }
                    Err(e) => Err(e),
                }
            }
            _ => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Arc<ReliefPage>>>> = Mutex::new(vec![None; batch.len()]);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 4)
        .min(batch.len());
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                qos.apply();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&i) = batch.get(k) else {
                        break;
                    };
                    let Some(source) = sources[k] else {
                        continue;
                    };
                    // A made page's water: WorldCover's and the features' (`relief_limit::Water`).
                    let made =
                        matches!(source, Source::Synthetic | Source::Combined | Source::Glo30);
                    let water = made.then(|| relief_limit::Water::load(geo, i));
                    if source == Source::Synthetic {
                        let (Some(coarse), Some(water)) = (coarse, &water) else {
                            continue;
                        };
                        let height = |p: [f64; 2]| coarse.height(p);
                        if let Some(page) = relief_synth::make_page(geo, lattice, i, &height, water)
                        {
                            results.lock().unwrap_or_else(|e| e.into_inner())[k] =
                                Some(Arc::new(page));
                        }
                        continue;
                    }
                    if source == Source::Combined {
                        let (Some(coarse), Some(water)) = (coarse, &water) else {
                            continue;
                        };
                        if let Some(page) = relief_combined::make_page(
                            geo,
                            lattice,
                            i,
                            glo30[k].as_ref(),
                            coarse,
                            water,
                        ) {
                            results.lock().unwrap_or_else(|e| e.into_inner())[k] =
                                Some(Arc::new(page));
                        }
                        continue;
                    }
                    if source == Source::Glo30 {
                        if let Some(inputs) = &glo30[k]
                            && let Some(water) = &water
                            && let Some(page) =
                                relief_glo30::make_page(geo, lattice, i, inputs, coarse, water)
                        {
                            results.lock().unwrap_or_else(|e| e.into_inner())[k] =
                                Some(Arc::new(page));
                        }
                        continue;
                    }
                    // A page none of whose tiles the source holds is none.
                    let held = page_tiles(geo, lattice, i, source)
                        .iter()
                        .any(|t| tiles.get(t).is_some_and(Option::is_some));
                    if held {
                        let page = Arc::new(make_page(geo, lattice, i, source, &tiles));
                        results.lock().unwrap_or_else(|e| e.into_inner())[k] = Some(page);
                    }
                }
            });
        }
    });
    Ok(batch
        .iter()
        .copied()
        .zip(results.into_inner().unwrap_or_else(|e| e.into_inner()))
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// The layer's side.

/// Where the camera stood at the last frame a scene fed its pages (the scenario's frame, the theatre point).
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 2])>> = Mutex::new(None);

/// Gives a land layer being built (off the frame loop's thread, with its scene) the relief's grid (its chart's (0, 0) at
/// theatre point `origin`) and uploads the made pages about where the camera last stood, so a scene rebuilt where the
/// old one stood draws them from its first frame. Nothing with the relief off, nor on a land without room for it (the
/// Medium terrain setting).
pub fn install(
    ctx: &mut naval_render::RenderContext<'_>,
    land: &mut naval_terrain::TerrainLayer,
    geo: LatLon,
    origin: [f64; 2],
) {
    if !enabled() || !land.takes_relief() {
        return;
    }
    let (grid, base) = grid(origin);
    if let Err(e) = land.set_relief_grid(ctx, grid) {
        eprintln!("the theatre's relief: {e}");
        return;
    }
    let last = *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    let frame = [geo.lat.to_bits(), geo.lon.to_bits()];
    let Some((_, at)) = last.filter(|(f, _)| *f == frame) else {
        return;
    };
    let lattice = Lattice::of(land.field(), origin);
    for index in block(at, grid.slots) {
        if let Some(Some(page)) = made(geo, &lattice, index) {
            land.queue_relief_page(local(index, base), page);
        }
    }
    land.flush_relief_pages(ctx);
}

fn local(index: [i64; 2], base: [i64; 2]) -> [i32; 2] {
    [(index[0] - base[0]) as i32, (index[1] - base[1]) as i32]
}

/// What a scene on screen does with its pages, kept by the driver from frame to frame.
#[derive(Default)]
pub struct Feed {
    /// The scene it feeds (the driver's swap count when it was made).
    pub scene: u64,
    block: Vec<[i64; 2]>,
    centre: Option<[i64; 2]>,
    /// Whether each page of the block is drawn at all (a source covers it and the window has land on it).
    drawn: HashMap<[i64; 2], bool>,
    asked: Vec<[i64; 2]>,
    asked_at: Option<Instant>,
    /// The block's pages to draw not yet made.
    pub pending: usize,
}

impl Feed {
    pub fn new(scene: u64) -> Self {
        Self {
            scene,
            ..Self::default()
        }
    }

    /// Hands `land` the made pages of the block about its camera and asks the worker for the rest. `origin`: the
    /// theatre point of the layer's chart (0, 0).
    pub fn feed(&mut self, land: &mut naval_terrain::TerrainLayer, geo: LatLon, origin: [f64; 2]) {
        land.show_relief(SHOWN.load(Ordering::Relaxed));
        let Some(grid) = land.relief_grid() else {
            return;
        };
        let Some(camera) = land.camera_chart() else {
            return;
        };
        let at = [camera[0] + origin[0], camera[1] + origin[1]];
        *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(([geo.lat.to_bits(), geo.lon.to_bits()], at));
        let side = page_m();
        let centre = [
            (at[0] / side + 0.5).floor() as i64,
            (at[1] / side + 0.5).floor() as i64,
        ];
        if self.centre != Some(centre) {
            self.centre = Some(centre);
            self.block = block(at, grid.slots);
        }
        let lattice = Lattice::of(land.field(), origin);
        let base = page_of(origin);
        let mut missing = Vec::new();
        for &index in &self.block {
            let drawn = *self
                .drawn
                .entry(index)
                .or_insert_with(|| match page_source(geo, index) {
                    Some(s) if s.synthetic() => page_has_relief(land.field(), origin, index),
                    Some(_) => page_has_land(land.field(), origin, index),
                    None => false,
                });
            let l = local(index, base);
            if !drawn || land.holds_relief_page(l) {
                continue;
            }
            match made(geo, &lattice, index) {
                Some(Some(page)) => land.queue_relief_page(l, page),
                Some(None) => {}
                None => missing.push(index),
            }
        }
        self.pending = missing.len();
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if missing != self.asked || (!missing.is_empty() && stale) {
            let coarse = Coarse {
                field: land.shared_field(),
                origin,
            };
            want(geo, lattice, Some(coarse), &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
    }
}

/// The relief's state for the RPC (`theatre_state.relief`): the grid's pages, those drawn, waiting for their upload and
/// still to be made, the level-0 patches drawn dense over them, and the worker's state; none with no grid on screen.
pub fn status_json(
    land: Option<&naval_terrain::TerrainLayer>,
    feed: &Feed,
    fresh: bool,
) -> Option<serde_json::Value> {
    let land = land?;
    let grid = land.relief_grid()?;
    let (drawn, uploading, dense) = land.relief_counts();
    let worker = status();
    let pages = (grid.slots * grid.slots) as usize;
    Some(serde_json::json!({
        "pages": pages,
        "texelM": TEXEL_M,
        "drawn": drawn,
        "uploading": uploading,
        "pending": if fresh { feed.pending } else { pages },
        "densePatches": dense,
        "shown": SHOWN.load(Ordering::Relaxed),
        "worker": {
            "queued": worker.queued,
            "working": worker.working,
            "made": worker.made,
            "lastBatchMs": worker.last_batch_ms,
            "tilesFetched": worker.tiles_fetched,
            "fetchedBytes": worker.fetched_bytes,
            "cacheBytes": worker.cache_bytes,
            "failed": worker.failed,
        },
    }))
}

#[cfg(test)]
#[path = "relief_tests.rs"]
mod tests;
