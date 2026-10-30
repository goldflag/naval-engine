//! Copernicus GLO-30 as a fine-relief source, a test (`NAVAL_RELIEF_SOURCE=glo30`; native/docs/terrain/ground.md, "Fine
//! relief near the camera"): the DEM's 1″ heights (30 m north to south; 2400 columns a degree from 50° to 60°, 3600
//! nearer the equator) through the same residual as a finer model's (`relief::residual_texels`), so a page never moves
//! the window's broad shape or its coast.
//!
//! - **Open ground only.** GLO-30 is a surface model: it measures the canopy and the roofs, so every forest edge or
//!   clearing would draw as a 10-30 m step. Its detail is kept only where ESA WorldCover's 10 m classes are open
//!   ([`open`]: grass, crops, bare or sparse, snow and ice, water, wetland, moss), none within [`DEAD_M`] of anything
//!   else (a lattice cell: no sample the window's bilinear surface reads there stands on a canopy) and all of it
//!   [`FADE_M`] further in, by the distance to the nearest closed pixel, so the mask's edge makes no step either.
//! - **The reads.** A page's cells' headers and the 1024² blocks its reach needs, by HTTP range from the AWS copy
//!   (`naval_region_bake::glo30`), kept as fetched under the relief's cache (`relief-dem-v1/glo30/<cell>/`, held to
//!   its bound with the tiles) and decoded in memory ([`BLOCKS_KEPT`]); WorldCover's full-resolution tiles through the
//!   surface cache (`surface::cover_patch`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use naval_earth::geo::{LatLon, to_lat_lon, wrap_lon};
use naval_region_bake::glo30::{self, Header, Sparse};
use naval_terrain::ReliefPage;
use naval_terrain::relief::{GUTTER, PAGE_TEXELS};
use naval_terrain::surface_maps::cover;

use super::relief::{self, Lattice, TEXEL_M};
use super::relief_limit::{self, Water};
use super::surface::{self, CoverPatch, FetchError, Fetched};

/// No GLO-30 detail within this of closed ground (a tree, a roof, a shrub): a lattice cell, so none of the samples the
/// window's surface is taken through there stands on the canopy.
pub const DEAD_M: f64 = 40.0;
/// Past [`DEAD_M`], the detail comes in over this (a smoothstep of the distance).
pub const FADE_M: f64 = 60.0;
/// Decoded GLO-30 blocks kept in memory (4 MiB each; a block is about 30 km by 20-30 km).
pub const BLOCKS_KEPT: usize = 16;
/// A page whose largest residual is under this holds nothing (all of it closed ground, or flat).
const EMPTY_M: f32 = 0.05;

/// Whether WorldCover's class is open ground, where a surface model measures the ground: grass, crops, bare or sparse,
/// snow and ice, water (flattened in GLO-30), herbaceous wetland, moss and lichen. Trees, shrubs, the built-up,
/// mangroves and the unknown are not.
pub fn open(class: u8) -> bool {
    matches!(
        cover::today(class),
        cover::GRASS
            | cover::CROP
            | cover::BARE
            | cover::SNOW
            | cover::WATER
            | cover::WETLAND
            | cover::MOSS
    )
}

/// The detail's weight at `d` metres from the nearest closed pixel: 0 within [`DEAD_M`], a smoothstep to 1 over
/// [`FADE_M`].
pub fn weight_at(d: f32) -> f32 {
    let t = ((d - DEAD_M as f32) / FADE_M as f32).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------------------------------------------------
// The page's place.

/// Page `index`'s map from its metres (x east, z south from its corner) to latitude and longitude (the longitude
/// unwrapped about the frame's): bilinear between its corners, carried past them for its gutters and margins.
pub struct GeoMap {
    corners: [[f64; 2]; 4],
}

impl GeoMap {
    pub fn new(geo: LatLon, index: [i64; 2]) -> Self {
        let side = relief::page_m();
        let at = |di: f64, dj: f64| {
            let p = to_lat_lon(
                geo,
                [(index[0] as f64 + di) * side, (index[1] as f64 + dj) * side],
            );
            [p.lat, geo.lon + wrap_lon(p.lon - geo.lon)]
        };
        Self {
            corners: [at(0.0, 0.0), at(1.0, 0.0), at(0.0, 1.0), at(1.0, 1.0)],
        }
    }

    /// Latitude and longitude of page metres `m`.
    pub fn at(&self, m: [f64; 2]) -> [f64; 2] {
        let side = relief::page_m();
        let (u, v) = (m[0] / side, m[1] / side);
        let [a, b, c, d] = self.corners;
        std::array::from_fn(|k| {
            (1.0 - u) * (1.0 - v) * a[k]
                + u * (1.0 - v) * b[k]
                + (1.0 - u) * v * c[k]
                + u * v * d[k]
        })
    }

    /// South, north, west and east of the page's metres from `lo` to `hi` each way.
    pub fn bounds(&self, lo: f64, hi: f64) -> [f64; 4] {
        let edge = [[lo, lo], [hi, lo], [lo, hi], [hi, hi]].map(|m| self.at(m));
        let min = |k: usize| edge.iter().map(|p| p[k]).fold(f64::INFINITY, f64::min);
        let max = |k: usize| edge.iter().map(|p| p[k]).fold(f64::NEG_INFINITY, f64::max);
        [min(0), max(0), min(1), max(1)]
    }
}

/// The page metres a page reads about its own: its residual's reach and, for the mask, the distance past which the
/// mask is full.
fn margin(lattice: &Lattice) -> (f64, f64) {
    let (lo, hi) = relief::reach(lattice);
    // The mask's reach or the band's blur (two passes of its box), whichever is wider, past the combined source's
    // extended grid (`EXTRA`), which holds every sample the page's residual stands on.
    let band = 2.0 * (BAND_M / TEXEL_M / 2.0).round() * TEXEL_M;
    let m = (DEAD_M + FADE_M + TEXEL_M).max(band) + EXTRA as f64 * TEXEL_M + 2.0 * TEXEL_M;
    (
        lo.min(-f64::from(GUTTER) * TEXEL_M - m),
        hi.max(relief::page_m() + f64::from(GUTTER) * TEXEL_M + m),
    )
}

// ---------------------------------------------------------------------------------------------------------------------
// GLO-30's reads.

fn cell_dir(cell: (i32, i32)) -> std::path::PathBuf {
    relief::dir()
        .join("glo30")
        .join(glo30::name(cell.0, cell.1))
}

fn server(e: impl std::fmt::Display) -> FetchError {
    FetchError::Server(e.to_string())
}

/// Which of a cell's files: its heights, or its filling mask (`AUXFILES/..._FLM.tif`: where the heights are TanDEM-X's
/// own, and where they were filled from another model under radar shadow and layover).
#[derive(Clone, Copy, PartialEq, Eq)]
enum File {
    Dem,
    Flm,
}

impl File {
    fn url(self, cell: (i32, i32)) -> String {
        match self {
            Self::Dem => glo30::url(cell.0, cell.1),
            Self::Flm => {
                let name = glo30::name(cell.0, cell.1);
                let stem = name.strip_suffix("_DEM").unwrap_or(&name);
                format!("https://{}/{name}/AUXFILES/{stem}_FLM.tif", glo30::HOST)
            }
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Dem => "",
            Self::Flm => "flm-",
        }
    }
}

/// Bytes `offset..offset + length` of cell `cell`'s file, from the disk or the network; none where the bucket has no
/// such file (the sea).
fn range(
    cell: (i32, i32),
    file: File,
    offset: u64,
    length: u64,
    name: &str,
) -> Result<Option<Vec<u8>>, FetchError> {
    let dir = cell_dir(cell);
    let prefix = file.prefix();
    let absent = dir.join(format!("{prefix}absent"));
    if absent.exists() {
        relief::touch(&absent);
        return Ok(None);
    }
    let path = dir.join(format!("{prefix}{offset}-{length}.{name}"));
    if let Ok(bytes) = std::fs::read(&path)
        && bytes.len() as u64 == length
    {
        relief::touch(&path);
        return Ok(Some(bytes));
    }
    if surface::offline() {
        return Err(FetchError::Offline(
            "the network was missing a moment ago".into(),
        ));
    }
    let url = file.url(cell);
    match surface::get(&url, Some((offset, length)))? {
        Fetched::Absent => {
            relief::write_file(&absent, b"").map_err(server)?;
            Ok(None)
        }
        Fetched::Bytes(bytes) => {
            relief::write_file(&path, &bytes).map_err(server)?;
            Ok(Some(bytes))
        }
    }
}

/// Headers read this session (none: the sea), by cell.
static HEADERS: Mutex<Vec<((i32, i32), Option<Arc<Header>>)>> = Mutex::new(Vec::new());

/// Cell `cell`'s header (its first [`glo30::HEADER_BYTES`], more when its tables reach past them); none for the sea.
fn header(cell: (i32, i32)) -> Result<Option<Arc<Header>>, FetchError> {
    if let Some((_, h)) = HEADERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| *k == cell)
    {
        return Ok(h.clone());
    }
    let name = glo30::name(cell.0, cell.1);
    let mut length = glo30::HEADER_BYTES;
    let mut found = None;
    for _ in 0..3 {
        let Some(bytes) = range(cell, File::Dem, 0, length, "header")? else {
            break;
        };
        match glo30::parse_header(&bytes, &name) {
            Ok(h) => {
                found = Some(Arc::new(h));
                break;
            }
            Err(glo30::HeaderError::Short(need)) if need as u64 > length => {
                length = (need as u64).div_ceil(4096) * 4096;
            }
            Err(e) => return Err(server(format!("{name}: {e:?}"))),
        }
    }
    let mut held = HEADERS.lock().unwrap_or_else(|e| e.into_inner());
    held.retain(|(k, _)| *k != cell);
    held.push((cell, found.clone()));
    Ok(found)
}

type BlockKey = ((i32, i32), usize);
static BLOCKS: Mutex<Vec<(BlockKey, Arc<Vec<f32>>)>> = Mutex::new(Vec::new());

/// Block `k` of cell `cell`, decoded.
fn block(cell: (i32, i32), header: &Header, k: usize) -> Result<Arc<Vec<f32>>, FetchError> {
    {
        let mut kept = BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = kept.iter().position(|(key, _)| *key == (cell, k)) {
            let entry = kept.remove(at);
            let data = entry.1.clone();
            kept.push(entry);
            return Ok(data);
        }
    }
    let name = glo30::name(cell.0, cell.1);
    let (offset, count) = (header.offsets[k], header.counts[k]);
    let bytes = range(cell, File::Dem, offset, count, "block")?
        .ok_or_else(|| server(format!("{name}: its block {k} is gone")))?;
    let data = match glo30::decode_block(&bytes, header.block) {
        Ok(d) => d,
        Err(e) => {
            // Damaged on disk: fetched again next time.
            let _ = std::fs::remove_file(cell_dir(cell).join(format!("{offset}-{count}.block")));
            return Err(server(format!("{name} block {k}: {e}")));
        }
    };
    let data = Arc::new(data);
    let mut kept = BLOCKS.lock().unwrap_or_else(|e| e.into_inner());
    kept.push(((cell, k), data.clone()));
    if kept.len() > BLOCKS_KEPT {
        kept.remove(0);
    }
    Ok(data)
}

/// GLO-30's heights over a box: its mosaic, NaN where a block is missing, 0 where the bucket has no cell (the sea).
pub struct Dem {
    window: glo30::Window,
    mosaic: glo30::Mosaic,
}

impl Dem {
    /// The cells and blocks over `[south, north, west, east]` (degrees, longitude unwrapped).
    pub fn load(bounds: [f64; 4]) -> Result<Self, FetchError> {
        let [south, north, west, east] = bounds;
        let win = glo30::window(south, north, west, east);
        let mut sparse = Sparse::default();
        for s in win.s0..=win.s1 {
            for w in win.w0..=win.w1 {
                let cell = (s, (w + 180).rem_euclid(360) - 180);
                let Some(header) = header(cell)? else {
                    sparse.cells.insert(cell, None);
                    continue;
                };
                let mut blocks = HashMap::new();
                if let Some(slice) = glo30::slice(&win, s, w, header.width) {
                    for k in glo30::slice_blocks(&header, &slice) {
                        blocks.insert(k, block(cell, &header, k)?);
                    }
                }
                sparse.cells.insert(cell, Some(((*header).clone(), blocks)));
            }
        }
        let (window, mosaic) = sparse.mosaic(south, north, west, east).map_err(server)?;
        Ok(Self { window, mosaic })
    }

    /// The bilinear height at (`lat`, `lon`), m above EGM2008.
    pub fn height(&self, lat: f64, lon: f64) -> f32 {
        self.mosaic.bilinear(&self.window, lat, lon) as f32
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The filling mask.

/// Whether a filling mask's value is not TanDEM-X's own: void (0) or filled from another model (3 ASTER, 4 SRTM90, 5
/// SRTM30, 6 GMTED2010, 7 SRTM30plus, 8 TerraSAR-X radargrammetry, 9 AW3D30, 100 and up national models); 1 (edited,
/// such as flattened water) and 2 (neither) are its own.
pub fn filled(value: u8) -> bool {
    !matches!(value, 1 | 2)
}

static FLM_HEADERS: Mutex<Vec<((i32, i32), Option<Arc<surface::Cog>>)>> = Mutex::new(Vec::new());
static FLM_TILES: Mutex<Vec<(BlockKey, Arc<Vec<u8>>)>> = Mutex::new(Vec::new());
const FLM_TILES_KEPT: usize = 16;

/// Cell `cell`'s filling mask's header; none where the bucket has none (the sea).
fn flm_header(cell: (i32, i32)) -> Result<Option<Arc<surface::Cog>>, FetchError> {
    if let Some((_, h)) = FLM_HEADERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(k, _)| *k == cell)
    {
        return Ok(h.clone());
    }
    let mut length = glo30::HEADER_BYTES;
    let mut found = None;
    for _ in 0..3 {
        let Some(bytes) = range(cell, File::Flm, 0, length, "header")? else {
            break;
        };
        match surface::parse_cog(&bytes) {
            Ok(cog) => {
                found = Some(Arc::new(cog));
                break;
            }
            Err(surface::CogError::Short(need)) if need > length => {
                length = need.div_ceil(4096) * 4096;
            }
            Err(e) => {
                let name = glo30::name(cell.0, cell.1);
                return Err(server(format!("{name}'s filling mask: {e:?}")));
            }
        }
    }
    let mut held = FLM_HEADERS.lock().unwrap_or_else(|e| e.into_inner());
    held.retain(|(k, _)| *k != cell);
    held.push((cell, found.clone()));
    Ok(found)
}

/// Tile `k` of cell `cell`'s filling mask, decoded.
fn flm_tile(cell: (i32, i32), cog: &surface::Cog, k: usize) -> Result<Arc<Vec<u8>>, FetchError> {
    {
        let mut kept = FLM_TILES.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(at) = kept.iter().position(|(key, _)| *key == (cell, k)) {
            let entry = kept.remove(at);
            let data = entry.1.clone();
            kept.push(entry);
            return Ok(data);
        }
    }
    let level = &cog.levels[0];
    let (offset, count) = (level.offsets[k], level.counts[k]);
    let data = if count == 0 {
        vec![0u8; (level.tile_width * level.tile_height) as usize]
    } else {
        let bytes = range(cell, File::Flm, offset, count, "tile")?
            .ok_or_else(|| server("a filling mask's tile is gone"))?;
        surface::decode_cog_tile(level, &bytes).map_err(server)?
    };
    let data = Arc::new(data);
    let mut kept = FLM_TILES.lock().unwrap_or_else(|e| e.into_inner());
    kept.push(((cell, k), data.clone()));
    if kept.len() > FLM_TILES_KEPT {
        kept.remove(0);
    }
    Ok(data)
}

/// One cell's filling mask over a box: its pixels from `first` (column east from the cell's west edge, row south from
/// its north edge; pixel-is-point, `per` a degree each way).
struct FillPart {
    south: i32,
    west: i32,
    per: [f64; 2],
    first: [i64; 2],
    width: usize,
    height: usize,
    values: Vec<u8>,
}

/// GLO-30's filling mask over a box.
pub struct Fill {
    parts: Vec<FillPart>,
}

impl Fill {
    /// None: every height taken as GLO-30's own.
    pub fn none() -> Self {
        Self { parts: Vec::new() }
    }

    /// The masks of the cells over `[south, north, west, east]` (degrees, longitude unwrapped).
    pub fn load(bounds: [f64; 4]) -> Result<Self, FetchError> {
        let [south, north, west, east] = bounds;
        let mut parts = Vec::new();
        for s in south.floor() as i32..=north.floor() as i32 {
            for w in west.floor() as i32..=east.floor() as i32 {
                let cell = (s, (w + 180).rem_euclid(360) - 180);
                let Some(cog) = flm_header(cell)? else {
                    continue;
                };
                let level = &cog.levels[0];
                let (cols, rows) = (i64::from(level.width), i64::from(level.height));
                let per = [cols as f64, rows as f64];
                let col = |lon: f64| ((lon - f64::from(w)) * per[0]).round() as i64;
                let row = |lat: f64| ((f64::from(s) + 1.0 - lat) * per[1]).round() as i64;
                let first = [col(west).clamp(0, cols - 1), row(north).clamp(0, rows - 1)];
                let last = [col(east).clamp(0, cols - 1), row(south).clamp(0, rows - 1)];
                let (width, height) = (
                    (last[0] - first[0] + 1) as usize,
                    (last[1] - first[1] + 1) as usize,
                );
                let mut values = vec![0u8; width * height];
                let (tw, th) = (i64::from(level.tile_width), i64::from(level.tile_height));
                let across = level.tiles_across() as i64;
                for ty in first[1] / th..=last[1] / th {
                    for tx in first[0] / tw..=last[0] / tw {
                        let tile = flm_tile(cell, &cog, (ty * across + tx) as usize)?;
                        for y in first[1].max(ty * th)..=last[1].min(ty * th + th - 1) {
                            for x in first[0].max(tx * tw)..=last[0].min(tx * tw + tw - 1) {
                                let v = tile[((y - ty * th) * tw + (x - tx * tw)) as usize];
                                values[(y - first[1]) as usize * width + (x - first[0]) as usize] =
                                    v;
                            }
                        }
                    }
                }
                parts.push(FillPart {
                    south: s,
                    west: w,
                    per,
                    first,
                    width,
                    height,
                    values,
                });
            }
        }
        Ok(Self { parts })
    }

    /// Whether GLO-30's height at (`lat`, `lon`) was filled from another model ([`filled`]); not where no mask covers.
    pub fn filled_at(&self, lat: f64, lon: f64) -> bool {
        self.value_at(lat, lon).is_some_and(filled)
    }

    /// The filling mask's value at (`lat`, `lon`) (its nearest pixel); none where no mask covers.
    pub fn value_at(&self, lat: f64, lon: f64) -> Option<u8> {
        let (s, w) = (lat.floor() as i32, lon.floor() as i32);
        let p = self.parts.iter().find(|p| p.south == s && p.west == w)?;
        let x = ((lon - f64::from(w)) * p.per[0]).round() as i64 - p.first[0];
        let y = ((f64::from(s) + 1.0 - lat) * p.per[1]).round() as i64 - p.first[1];
        if x < 0 || y < 0 || x >= p.width as i64 || y >= p.height as i64 {
            return None;
        }
        Some(p.values[y as usize * p.width + x as usize])
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The options.

/// How the GLO-30 pages are made (`NAVAL_RELIEF_GLO30`, comma-separated): `fills` keeps the heights GLO-30 filled from
/// other models (by default masked as closed ground is); `band` lets GLO-30 replace the window's own heights at
/// wavelengths under a few hundred metres ([`band_texels`]: the window's samples move) rather than only add what lies
/// between the samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub keep_fills: bool,
    pub band: bool,
}

impl Options {
    pub fn parse(v: &str) -> Result<Self, String> {
        let mut o = Self::default();
        for word in v.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            match word {
                "fills" => o.keep_fills = true,
                "band" => o.band = true,
                other => return Err(format!("{other}: fills or band")),
            }
        }
        Ok(o)
    }

    /// `NAVAL_RELIEF_GLO30`'s, read once.
    pub fn from_env() -> Self {
        static OPTIONS: std::sync::OnceLock<Options> = std::sync::OnceLock::new();
        *OPTIONS.get_or_init(|| match std::env::var("NAVAL_RELIEF_GLO30") {
            Ok(v) => Self::parse(&v).unwrap_or_else(|e| {
                eprintln!("NAVAL_RELIEF_GLO30: {e}");
                Self::default()
            }),
            Err(_) => Self::default(),
        })
    }
}

/// The band variant's box (metres): two passes of it blurred are taken off, so it keeps wavelengths under about twice
/// this.
pub const BAND_M: f64 = 164.0;

/// The band variant's texels: GLO-30's heights less the window's (`fine`, `window` at the page's metres), less the
/// same blurred (two passes of a [`BAND_M`] box): GLO-30's own shape under a few hundred metres and the window's above,
/// the same at a texel whichever page holds it.
pub fn band_texels(fine: &dyn Fn([f64; 2]) -> f32, window: &dyn Fn([f64; 2]) -> f32) -> Vec<f32> {
    band_grid(fine, window, 0)
}

/// Texels a side the combined source's grids reach past a page's own (with its gutters): every lattice sample its
/// residual's surface stands on over the grid it is made on ([`relief_limit::MARGIN`] past the page's), and a texel
/// more for their slopes.
pub const EXTRA: usize = relief_limit::MARGIN + 12;

/// [`band_texels`] over the page's texels and `extra` more each way (`PAGE_TEXELS + 2 extra` a side).
pub fn band_grid(
    fine: &dyn Fn([f64; 2]) -> f32,
    window: &dyn Fn([f64; 2]) -> f32,
    extra: usize,
) -> Vec<f32> {
    let n = PAGE_TEXELS as usize + 2 * extra;
    let g = f64::from(GUTTER) + extra as f64;
    let r = (BAND_M / TEXEL_M / 2.0).round() as usize;
    let pad = 2 * r;
    let s = n + 2 * pad;
    let mut d = vec![0f32; s * s];
    for j in 0..s {
        for i in 0..s {
            let m = [
                (i as f64 - pad as f64 - g + 0.5) * TEXEL_M,
                (j as f64 - pad as f64 - g + 0.5) * TEXEL_M,
            ];
            d[j * s + i] = fine(m) - window(m);
        }
    }
    // A box 2r + 1 wide along the rows, then the columns, twice: each pass leaves r less a side valid, the page's own
    // texels and gutters inside them.
    let pass = |src: &[f32], stride: usize, step: usize| {
        let mut out = src.to_vec();
        for line in 0..s {
            let at = |k: usize| f64::from(src[line * stride + k * step]);
            let mut sum: f64 = (0..=2 * r).map(at).sum();
            for k in r..s - r {
                out[line * stride + k * step] = (sum / (2 * r + 1) as f64) as f32;
                if k + r + 1 < s {
                    sum += at(k + r + 1) - at(k - r);
                }
            }
        }
        out
    };
    let mut b = d.clone();
    for _ in 0..2 {
        b = pass(&b, s, 1);
        b = pass(&b, 1, s);
    }
    let mut out = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            let k = (j + pad) * s + i + pad;
            out[j * n + i] = d[k] - b[k];
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The page.

/// What a GLO-30 page reads: its cells, their filling masks and WorldCover's classes over its reach and margin, its map
/// and the options.
pub struct Inputs {
    pub map: GeoMap,
    pub dem: Dem,
    pub fill: Fill,
    pub cover: CoverPatch,
    pub options: Options,
}

impl Inputs {
    pub fn load(geo: LatLon, lattice: &Lattice, index: [i64; 2]) -> Result<Self, FetchError> {
        let options = Options::from_env();
        Self::load_with(geo, lattice, index, options, !options.keep_fills)
    }

    /// [`Inputs::load`] with `options`, the filling mask read only with `fill`.
    pub fn load_with(
        geo: LatLon,
        lattice: &Lattice,
        index: [i64; 2],
        options: Options,
        fill: bool,
    ) -> Result<Self, FetchError> {
        let map = GeoMap::new(geo, index);
        let (lo, hi) = margin(lattice);
        let [s, n, w, e] = map.bounds(lo, hi);
        // A pixel's more about them each way (30 m; WorldCover's 10 m).
        let pad = 1.5 / 3600.0;
        let bounds = [s - pad, n + pad, w - 2.0 * pad, e + 2.0 * pad];
        let dem = Dem::load(bounds)?;
        let fill = if fill {
            Fill::load(bounds)?
        } else {
            Fill::none()
        };
        let cover = surface::cover_patch(bounds[0], bounds[1], bounds[2], bounds[3])?;
        Ok(Self {
            map,
            dem,
            fill,
            cover,
            options,
        })
    }

    /// GLO-30's height at page metres `m`.
    pub fn height(&self, m: [f64; 2]) -> f32 {
        let [lat, lon] = self.map.at(m);
        self.dem.height(lat, lon)
    }

    /// Whether GLO-30's height at page metres `m` was filled from another model.
    pub fn filled(&self, m: [f64; 2]) -> bool {
        let [lat, lon] = self.map.at(m);
        self.fill.filled_at(lat, lon)
    }
}

/// A GLO-30 page's parts, `PAGE_TEXELS`² each (with the gutters): GLO-30's heights, the residual through the lattice
/// unmasked, and the mask's weight. The page's residual is `residual × weight`.
pub struct Parts {
    pub fine: Vec<f32>,
    pub residual: Vec<f32>,
    pub weight: Vec<f32>,
}

/// The mask's weight at a page's texels from the classes at its metres (`class`), by the distance to the nearest closed
/// pixel (an 8-neighbour chamfer over the page and a margin wide enough that a closed pixel beyond it is past the fade).
pub fn weights(class: &dyn Fn([f64; 2]) -> u8) -> Vec<f32> {
    weights_grid(&|m| open(class(m)), 0)
}

/// The mask's weight over the page's texels and `extra` more each way, from whether the ground at a page's metres is
/// open (`open_at`).
pub fn weights_grid(open_at: &dyn Fn([f64; 2]) -> bool, extra: usize) -> Vec<f32> {
    let n = PAGE_TEXELS as usize + 2 * extra;
    let g = f64::from(GUTTER) + extra as f64;
    let pad = ((DEAD_M + FADE_M) / TEXEL_M).ceil() as usize + 1;
    let s = n + 2 * pad;
    let far = 1e6f32;
    let mut d = vec![far; s * s];
    for j in 0..s {
        for i in 0..s {
            let m = [
                (i as f64 - pad as f64 - g + 0.5) * TEXEL_M,
                (j as f64 - pad as f64 - g + 0.5) * TEXEL_M,
            ];
            if !open_at(m) {
                d[j * s + i] = 0.0;
            }
        }
    }
    let (a, b) = (TEXEL_M as f32, TEXEL_M as f32 * std::f32::consts::SQRT_2);
    for j in 0..s {
        for i in 0..s {
            let k = j * s + i;
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + a);
            }
            if j > 0 {
                v = v.min(d[k - s] + a);
                if i > 0 {
                    v = v.min(d[k - s - 1] + b);
                }
                if i + 1 < s {
                    v = v.min(d[k - s + 1] + b);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..s).rev() {
        for i in (0..s).rev() {
            let k = j * s + i;
            let mut v = d[k];
            if i + 1 < s {
                v = v.min(d[k + 1] + a);
            }
            if j + 1 < s {
                v = v.min(d[k + s] + a);
                if i + 1 < s {
                    v = v.min(d[k + s + 1] + b);
                }
                if i > 0 {
                    v = v.min(d[k + s - 1] + b);
                }
            }
            d[k] = v;
        }
    }
    let mut out = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            out[j * n + i] = weight_at(d[(j + pad) * s + i + pad]);
        }
    }
    out
}

/// Page `index`'s parts on `lattice` from heights (`height`) and classes (`class`) at its metres.
pub fn parts(
    lattice: &Lattice,
    index: [i64; 2],
    height: &dyn Fn([f64; 2]) -> f32,
    class: &dyn Fn([f64; 2]) -> u8,
) -> Parts {
    let n = PAGE_TEXELS as usize;
    let g = f64::from(GUTTER);
    let floored = |m: [f64; 2]| {
        let h = height(m);
        if h.is_finite() { h.max(-5.0) } else { 0.0 }
    };
    let residual = relief::residual_texels(lattice, index, &floored);
    let mut fine = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            fine[j * n + i] = floored([
                (i as f64 - g + 0.5) * TEXEL_M,
                (j as f64 - g + 0.5) * TEXEL_M,
            ]);
        }
    }
    Parts {
        fine,
        residual,
        weight: weights(class),
    }
}

/// Page `index`'s parts from its inputs: a height filled from another model is closed ground (unless the options keep
/// the fills).
pub fn page_parts(lattice: &Lattice, index: [i64; 2], inputs: &Inputs) -> Parts {
    let height = |m: [f64; 2]| inputs.height(m);
    let class = |m: [f64; 2]| {
        let [lat, lon] = inputs.map.at(m);
        if !inputs.options.keep_fills && inputs.fill.filled_at(lat, lon) {
            cover::UNKNOWN
        } else {
            inputs.cover.class_at(lat, lon)
        }
    };
    parts(lattice, index, &height, &class)
}

/// Page `index` about `geo` on `lattice` from its inputs: GLO-30's residual on open ground (with the band option, its
/// band against the window's heights, `coarse`); none where that holds under 5 cm.
pub fn make_page(
    _geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    inputs: &Inputs,
    coarse: Option<&relief::Coarse>,
    water: &Water,
) -> Option<ReliefPage> {
    let texels = held(lattice, index, inputs, coarse, water);
    let mut page = ReliefPage::from_residual(&texels, TEXEL_M as f32);
    page.real = false;
    (page.peak >= EMPTY_M).then_some(page)
}

/// Page `index`'s residuals as [`make_page`] takes them: GLO-30's residual (or its band against the window's heights,
/// `coarse`) on open ground, made [`relief_limit::MARGIN`] past its texels, held to the window and off `water`.
pub fn held(
    lattice: &Lattice,
    index: [i64; 2],
    inputs: &Inputs,
    coarse: Option<&relief::Coarse>,
    water: &Water,
) -> Vec<f32> {
    let extra = relief_limit::MARGIN;
    let side = relief::page_m();
    let corner = [index[0] as f64 * side, index[1] as f64 * side];
    let class = |m: [f64; 2]| {
        let [lat, lon] = inputs.map.at(m);
        if !inputs.options.keep_fills && inputs.fill.filled_at(lat, lon) {
            cover::UNKNOWN
        } else {
            inputs.cover.class_at(lat, lon)
        }
    };
    let residual = match coarse {
        Some(coarse) if inputs.options.band => {
            let window = |m: [f64; 2]| coarse.height([corner[0] + m[0], corner[1] + m[1]]);
            band_grid(&|m| inputs.height(m).max(-5.0), &window, extra)
        }
        _ => {
            let floored = |m: [f64; 2]| {
                let h = inputs.height(m);
                if h.is_finite() { h.max(-5.0) } else { 0.0 }
            };
            relief::residual_grid(lattice, index, &floored, extra)
        }
    };
    let weight = weights_grid(&|m| open(class(m)), extra);
    let grid: Vec<f32> = residual.iter().zip(&weight).map(|(r, w)| r * w).collect();
    // With no window known, its height taken as past the shore's band.
    let window = |p: [f64; 2]| coarse.map_or(f32::MAX, |c| c.height(p));
    relief_limit::hold(&grid, &relief_limit::weights(index, water, &window))
}

#[cfg(test)]
#[path = "relief_glo30_tests.rs"]
mod tests;
