//! Trees where real trees stand (native/docs/terrain/trees.md, "Trees where real trees stand"): Meta and the World
//! Resources
//! Institute's 1 m canopy height map brought to the land layer's canopy pages (`naval_terrain::canopy`) about the camera,
//! so the theatre's trees stand on the real crowns, as tall as they were measured. Presentation only, as the rest of the
//! theatre's land.
//!
//! - **The source** ([`URL`], CC BY 4.0: [`ATTRIBUTION`]). One BigTIFF a zoom-9 web-mercator tile, named by its
//!   quadkey: 65,536 pixels a side (the zoom-17 pixel, 1.19 m at the equator), a byte a pixel (metres), a strip a row,
//!   deflated with the horizontal predictor. A tile it does not cover (the open sea) is missing (404). A tile's strip
//!   table (1 MB) is read once and kept.
//! - **Chunks** (what is kept on disk, under `surface::cache_dir()`'s `canopy-chm-v1`): [`CHUNK_PX`]² pixels of a tile,
//!   the tallest of each 2 × 2 kept (1,024² cells, 2.4 m at the equator, 2 m at 35°). A chunk row's strips are read
//!   whole (2,048 rows of 2–9 KB deflated, in [`FETCHERS`] ranged requests at once) and all 32 of its chunks kept, so a
//!   place read once loads from the disk.
//! - **Pages.** [`PAGE_TEXELS`]² texels of [`TEXEL_M`] (2 km) fixed in the theatre's metres, [`SLOTS`]² about the camera
//!   (8 km each way). The worker makes the nearest missing page and those of its row with it (they share their chunk
//!   rows): each 2 m cell takes the tallest of the chunk cells under it, and `CanopyPage::from_heights` finds the crowns.
//!   The frame loop hands made pages to the layer ([`Feed`]), which uploads a few a frame.
//! - **Switched** with the surface streaming (`surface::enabled`), on the High terrain setting (the trees') only;
//!   `NAVAL_CANOPY=off` leaves it off alone.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::{LatLon, to_lat_lon, wrap_lon};
use naval_terrain::canopy::{CROWN_REACH_CELLS, PAGE_TEXELS, UNKNOWN};
use naval_terrain::{CanopyGrid, CanopyPage};

use super::surface::{self, FETCHERS, FetchError, Fetched};

/// The canopy height map's tiles (`{URL}/{quadkey}.tif`).
pub const URL: &str =
    "https://dataforgood-fb-data.s3.amazonaws.com/forests/v1/alsgedi_global_v6_float/chm";
/// The credit its licence (CC BY 4.0) asks for.
pub const ATTRIBUTION: &str = "Canopy height: Meta and World Resources Institute (WRI), 2024, High Resolution Canopy \
     Height Maps (CHM); source imagery for CHM © 2016 Maxar; CC BY 4.0";
/// The zoom of the tiles the map is cut into, and of its pixels.
const TILE_ZOOM: u32 = 9;
const PX_ZOOM: u32 = 17;
/// Pixels on a tile's side.
const TILE_PX: i64 = 65_536;
/// Pixels round the world (a column past the antimeridian wraps).
const WORLD_PX: i64 = TILE_PX << TILE_ZOOM;
/// Pixels on a chunk's side, and its cells (the tallest of each 2 × 2 pixels).
pub const CHUNK_PX: i64 = 2048;
const CHUNK_CELLS: usize = 1024;
/// Chunks on a tile's side.
const CHUNKS: i64 = TILE_PX / CHUNK_PX;
/// A page's texel (m), and the cells (m) its crowns are found on.
pub const TEXEL_M: f64 = 8.0;
const CELL_M: f64 = 2.0;
const CELLS: usize = 4;
/// Cells about a page that its crowns are compared with.
const MARGIN: usize = CROWN_REACH_CELLS;
/// Pages a side the layer holds about the camera.
pub const SLOTS: u32 = 8;
/// Pages the worker makes together (one row of them).
pub const BATCH: usize = 8;
/// Made pages kept in memory (256 KB each): the block and a row or two beyond.
pub const KEPT: usize = 96;
/// Chunks kept in memory (1 MB each).
const CHUNKS_KEPT: usize = 40;
/// The bytes read for a tile's header (its directory; the tables after it are read by where it says).
const HEADER_BYTES: u64 = 16 << 10;
/// A page whose making failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);

const _: () = assert!(CELLS as f64 * CELL_M == TEXEL_M);
const _: () = assert!(CHUNK_CELLS as i64 * 2 == CHUNK_PX);

/// Whether the canopy is read: with the surface streaming on (`surface::enabled`), unless `NAVAL_CANOPY` is `off` (or
/// `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_CANOPY").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

static SHOWN: AtomicBool = AtomicBool::new(true);

/// Reads or ignores the canopy on screen without dropping its pages (for before-and-after captures and frame
/// measurements): taken up by the next frame's feed.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------------------------------------------------
// The source's tiles.

/// The quadkey naming zoom-9 tile (x, y).
pub fn quadkey(x: i64, y: i64) -> String {
    (1..=TILE_ZOOM)
        .rev()
        .map(|bit| {
            let mask = 1 << (bit - 1);
            char::from(b'0' + u8::from(x & mask != 0) + 2 * u8::from(y & mask != 0))
        })
        .collect()
}

/// A chunk: its tile (x, y at zoom 9) and its place in the tile (column, row).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkKey {
    pub tile: [i64; 2],
    pub chunk: [i64; 2],
}

impl ChunkKey {
    /// The chunk holding global zoom-17 pixel `px` (its column wrapped round the world).
    pub fn of(px: [i64; 2]) -> Self {
        let px = [px[0].rem_euclid(WORLD_PX), px[1]];
        Self {
            tile: px.map(|p| p.div_euclid(TILE_PX)),
            chunk: px.map(|p| p.rem_euclid(TILE_PX) / CHUNK_PX),
        }
    }
}

/// Where a tile's strip tables lie and how its strips are stored, from its directory.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub compression: u16,
    pub predictor: u16,
    /// Each table's place (bytes from the start), entries and bytes an entry.
    pub offsets: (u64, u64, u64),
    pub counts: (u64, u64, u64),
}

fn le16(b: &[u8], at: u64) -> Option<u16> {
    let at = usize::try_from(at).ok()?;
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn le32(b: &[u8], at: u64) -> Option<u32> {
    let at = usize::try_from(at).ok()?;
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le64(b: &[u8], at: u64) -> Option<u64> {
    let at = usize::try_from(at).ok()?;
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Reads a tile's directory (a little-endian BigTIFF, as the map's tiles all are) from its first bytes, holding it to
/// the layout read here: 65,536² one-byte pixels, a strip a row, deflated or not, with or without the horizontal
/// predictor.
pub fn parse_header(b: &[u8]) -> Result<Header, String> {
    if b.get(..4) != Some(&[b'I', b'I', 43, 0]) {
        return Err("not a little-endian BigTIFF".into());
    }
    let short = || "the header ends early".to_string();
    let ifd = le64(b, 8).ok_or_else(short)?;
    let entries = le64(b, ifd).ok_or_else(short)?;
    if entries > 512 {
        return Err(format!("{entries} directory entries"));
    }
    let mut tags: HashMap<u16, (u16, u64, u64)> = HashMap::new();
    for k in 0..entries {
        let at = ifd + 8 + k * 20;
        let tag = le16(b, at).ok_or_else(short)?;
        let kind = le16(b, at + 2).ok_or_else(short)?;
        let count = le64(b, at + 4).ok_or_else(short)?;
        // The value where it fits in the entry's 8 bytes (one short, long or long8 here), else where its values lie.
        let size = match kind {
            1 | 2 | 6 | 7 => 1,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            _ => 8,
        };
        let value = match kind {
            3 if count * size <= 8 => u64::from(le16(b, at + 12).ok_or_else(short)?),
            4 if count * size <= 8 => u64::from(le32(b, at + 12).ok_or_else(short)?),
            _ => le64(b, at + 12).ok_or_else(short)?,
        };
        tags.insert(tag, (kind, count, value));
    }
    let single = |tag: u16, default: Option<u64>| -> Result<u64, String> {
        match tags.get(&tag) {
            Some(&(3 | 4 | 16, 1, v)) => Ok(v),
            Some(other) => Err(format!("tag {tag}: {other:?}")),
            None => default.ok_or_else(|| format!("no tag {tag}")),
        }
    };
    let width = single(256, None)?;
    let height = single(257, None)?;
    if [width, height] != [TILE_PX as u64; 2] {
        return Err(format!("{width} x {height} pixels"));
    }
    if single(258, Some(1))? != 8 || single(277, Some(1))? != 1 || single(284, Some(1))? != 1 {
        return Err("not one byte a pixel".into());
    }
    if single(278, None)? != 1 {
        return Err("not a strip a row".into());
    }
    let compression = single(259, Some(1))? as u16;
    if !matches!(compression, 1 | 8 | 32946) {
        return Err(format!("compression {compression}"));
    }
    let predictor = single(317, Some(1))? as u16;
    if !matches!(predictor, 1 | 2) {
        return Err(format!("predictor {predictor}"));
    }
    let table = |tag: u16| -> Result<(u64, u64, u64), String> {
        match tags.get(&tag) {
            Some(&(kind @ (4 | 16), count, at)) if count == TILE_PX as u64 => {
                Ok((at, count, if kind == 4 { 4 } else { 8 }))
            }
            other => Err(format!("strip table {tag}: {other:?}")),
        }
    };
    Ok(Header {
        compression,
        predictor,
        offsets: table(273)?,
        counts: table(279)?,
    })
}

/// A tile's strips: where each row's bytes lie and how they are stored.
#[derive(Clone, Debug, PartialEq)]
pub struct Strips {
    pub compression: u16,
    pub predictor: u16,
    pub offsets: Vec<u64>,
    pub counts: Vec<u64>,
}

impl Strips {
    /// The tables from `bytes`, the file's bytes from `start` on.
    fn read(header: &Header, bytes: &[u8], start: u64) -> Option<Self> {
        let table = |(at, count, size): (u64, u64, u64)| -> Option<Vec<u64>> {
            (0..count)
                .map(|k| {
                    let p = at.checked_sub(start)? + k * size;
                    if size == 4 {
                        le32(bytes, p).map(u64::from)
                    } else {
                        le64(bytes, p)
                    }
                })
                .collect()
        };
        Some(Self {
            compression: header.compression,
            predictor: header.predictor,
            offsets: table(header.offsets)?,
            counts: table(header.counts)?,
        })
    }

    /// As kept on disk: compression and predictor, then the offsets and the counts.
    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + 16 * self.offsets.len());
        out.extend(self.compression.to_le_bytes());
        out.extend(self.predictor.to_le_bytes());
        for v in self.offsets.iter().chain(&self.counts) {
            out.extend(v.to_le_bytes());
        }
        out
    }

    fn from_bytes(b: &[u8]) -> Option<Self> {
        let n = TILE_PX as usize;
        if b.len() != 4 + 16 * n {
            return None;
        }
        let at = |k: usize| le64(b, (4 + 8 * k) as u64);
        Some(Self {
            compression: le16(b, 0)?,
            predictor: le16(b, 2)?,
            offsets: (0..n).map(at).collect::<Option<_>>()?,
            counts: (n..2 * n).map(at).collect::<Option<_>>()?,
        })
    }
}

/// A row's pixels from its stored bytes (65,536 of them, the predictor undone).
pub fn decode_row(strips: &Strips, stored: &[u8]) -> Result<Vec<u8>, String> {
    let n = TILE_PX as usize;
    if stored.is_empty() {
        return Ok(vec![0; n]);
    }
    let mut row = match strips.compression {
        1 => stored.to_vec(),
        _ => miniz_oxide::inflate::decompress_to_vec_zlib(stored)
            .map_err(|e| format!("inflating a row: {e:?}"))?,
    };
    if row.len() < n {
        return Err(format!("a row of {} pixels", row.len()));
    }
    row.truncate(n);
    if strips.predictor == 2 {
        for k in 1..n {
            row[k] = row[k].wrapping_add(row[k - 1]);
        }
    }
    Ok(row)
}

// ---------------------------------------------------------------------------------------------------------------------
// The disk and the network.

fn dir() -> PathBuf {
    surface::cache_dir().join("canopy-chm-v1")
}

fn tile_dir(tile: [i64; 2]) -> PathBuf {
    dir().join(quadkey(tile[0], tile[1]))
}

fn absent_marker(tile: [i64; 2]) -> PathBuf {
    dir().join(format!("{}.absent", quadkey(tile[0], tile[1])))
}

fn chunk_path(key: ChunkKey) -> PathBuf {
    tile_dir(key.tile)
        .join(key.chunk[1].to_string())
        .join(format!("{}.bin", key.chunk[0]))
}

/// Written whole, then moved into place: a reader never sees half a file.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
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
    FetchError::Server(format!("the canopy's cache: {e}"))
}

/// A chunk's cells, or none where the map has no tile.
type Chunk = Option<Arc<Vec<u8>>>;

/// A chunk from the disk: `Some(chunk)` when kept (or its tile known missing), `None` when not read yet.
fn chunk_from_disk(key: ChunkKey) -> Option<Chunk> {
    if absent_marker(key.tile).exists() {
        return Some(None);
    }
    let stored = std::fs::read(chunk_path(key)).ok()?;
    let cells = miniz_oxide::inflate::decompress_to_vec_zlib(&stored).ok()?;
    (cells.len() == CHUNK_CELLS * CHUNK_CELLS).then(|| Some(Arc::new(cells)))
}

/// Tile `tile`'s strips, from the disk or the network; none where the map has no tile.
fn strips(tile: [i64; 2]) -> Result<Option<Arc<Strips>>, FetchError> {
    type Kept = ([i64; 2], Option<Arc<Strips>>);
    static KEPT_STRIPS: Mutex<Vec<Kept>> = Mutex::new(Vec::new());
    if let Some((_, s)) = KEPT_STRIPS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(t, _)| *t == tile)
    {
        return Ok(s.clone());
    }
    let path = tile_dir(tile).join("strips.bin");
    let found = if absent_marker(tile).exists() {
        None
    } else if let Some(s) = std::fs::read(&path)
        .ok()
        .and_then(|b| Strips::from_bytes(&b))
    {
        Some(Arc::new(s))
    } else {
        let url = format!("{URL}/{}.tif", quadkey(tile[0], tile[1]));
        match surface::get(&url, Some((0, HEADER_BYTES)))? {
            Fetched::Absent => {
                write_file(&absent_marker(tile), b"").map_err(io_error)?;
                None
            }
            Fetched::Bytes(head) => {
                let header =
                    parse_header(&head).map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
                let ends = |(at, count, size): (u64, u64, u64)| (at, at + count * size);
                let (a0, a1) = ends(header.offsets);
                let (b0, b1) = ends(header.counts);
                let (start, end) = (a0.min(b0), a1.max(b1));
                let Fetched::Bytes(tables) = surface::get(&url, Some((start, end - start)))? else {
                    return Err(FetchError::Server(format!(
                        "{url}: its tables went missing"
                    )));
                };
                let s = Strips::read(&header, &tables, start)
                    .ok_or_else(|| FetchError::Server(format!("{url}: short strip tables")))?;
                surface::log_line(&format!(
                    "the canopy's tile {} found (its tables {} KB)",
                    quadkey(tile[0], tile[1]),
                    tables.len() >> 10
                ));
                write_file(&path, &s.to_bytes()).map_err(io_error)?;
                Some(Arc::new(s))
            }
        }
    };
    let mut kept = KEPT_STRIPS.lock().unwrap_or_else(|e| e.into_inner());
    kept.push((tile, found.clone()));
    if kept.len() > 4 {
        kept.remove(0);
    }
    Ok(found)
}

/// Bytes read from the network this session.
static FETCHED_BYTES: AtomicU64 = AtomicU64::new(0);
/// Chunk rows read from the network this session, and the milliseconds reading them took (fetching and decoding),
/// and keeping their chunks on disk.
static ROWS_READ: AtomicU64 = AtomicU64::new(0);
static ROWS_READ_MS: AtomicU64 = AtomicU64::new(0);
static ROWS_KEPT_MS: AtomicU64 = AtomicU64::new(0);

/// Reads chunk row `row` of `tile` (2,048 strips) whole, a few ranged requests at once, and returns its 32 chunks'
/// cells (the tallest of each 2 × 2 pixels).
fn read_chunk_row(tile: [i64; 2], row: i64, strips: &Strips) -> Result<Vec<Vec<u8>>, FetchError> {
    let url = format!("{URL}/{}.tif", quadkey(tile[0], tile[1]));
    let first = (row * CHUNK_PX) as usize;
    // Groups of rows (pairs kept together), each one request.
    let group = (CHUNK_PX as usize / FETCHERS).max(2) & !1;
    let groups: Vec<std::ops::Range<usize>> = (first..first + CHUNK_PX as usize)
        .step_by(group)
        .map(|r| r..(r + group).min(first + CHUNK_PX as usize))
        .collect();
    let width = TILE_PX as usize / 2;
    let next = AtomicUsize::new(0);
    type Group = Option<Result<Vec<u8>, FetchError>>;
    let results: Mutex<Vec<Group>> = Mutex::new((0..groups.len()).map(|_| None).collect());
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..FETCHERS.min(groups.len()) {
            scope.spawn(|| {
                qos.apply();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(rows) = groups.get(k) else {
                        break;
                    };
                    let result = (|| {
                        let start = rows.clone().map(|r| strips.offsets[r]).min().unwrap_or(0);
                        let end = rows
                            .clone()
                            .map(|r| strips.offsets[r] + strips.counts[r])
                            .max()
                            .unwrap_or(0);
                        let stored: u64 = rows.clone().map(|r| strips.counts[r]).sum();
                        // The strips lie one after another; scattered ones are not read here.
                        if end - start > stored + stored / 8 + 4096 || end - start > 48 << 20 {
                            return Err(FetchError::Server(format!(
                                "{url}: rows {rows:?} scattered over {} bytes",
                                end - start
                            )));
                        }
                        let bytes = if end > start {
                            match surface::get(&url, Some((start, end - start)))? {
                                Fetched::Bytes(b) => b,
                                Fetched::Absent => {
                                    return Err(FetchError::Server(format!("{url}: went missing")));
                                }
                            }
                        } else {
                            Vec::new()
                        };
                        FETCHED_BYTES.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                        let mut out = vec![0u8; rows.len() / 2 * width];
                        for (n, r) in rows.clone().enumerate() {
                            let at = (strips.offsets[r] - start) as usize;
                            let stored =
                                bytes
                                    .get(at..at + strips.counts[r] as usize)
                                    .ok_or_else(|| {
                                        FetchError::Server(format!("{url}: a short read"))
                                    })?;
                            let pixels = decode_row(strips, stored)
                                .map_err(|e| FetchError::Server(format!("{url}: row {r}: {e}")))?;
                            let line = &mut out[n / 2 * width..][..width];
                            for (c, pair) in line.iter_mut().zip(pixels.as_chunks::<2>().0) {
                                *c = (*c).max(pair[0]).max(pair[1]);
                            }
                        }
                        Ok(out)
                    })();
                    results.lock().unwrap_or_else(|e| e.into_inner())[k] = Some(result);
                }
            });
        }
    });
    let mut chunks = vec![vec![0u8; CHUNK_CELLS * CHUNK_CELLS]; CHUNKS as usize];
    let mut line = 0;
    for result in results.into_inner().unwrap_or_else(|e| e.into_inner()) {
        let out = result.unwrap_or_else(|| Err(FetchError::Server("a group not read".into())))?;
        for pooled in out.chunks_exact(width) {
            for (c, chunk) in chunks.iter_mut().enumerate() {
                // Heights of 255 m and over are none the map holds (its tallest trees are about 100 m): kept below
                // the layer's unknown.
                for (to, from) in chunk[line * CHUNK_CELLS..][..CHUNK_CELLS]
                    .iter_mut()
                    .zip(&pooled[c * CHUNK_CELLS..][..CHUNK_CELLS])
                {
                    *to = (*from).min(UNKNOWN - 1);
                }
            }
            line += 1;
        }
    }
    Ok(chunks)
}

/// Chunks kept in memory, the latest last.
static CHUNK_CACHE: Mutex<Vec<(ChunkKey, Chunk)>> = Mutex::new(Vec::new());

fn keep_chunk(key: ChunkKey, chunk: Chunk) {
    let mut kept = CHUNK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|(k, _)| *k != key);
    kept.push((key, chunk));
    if kept.len() > CHUNKS_KEPT {
        let extra = kept.len() - CHUNKS_KEPT;
        kept.drain(..extra);
    }
}

/// The chunks `keys`: from memory, the disk, or the network (a chunk row at a time, every chunk of it kept on disk).
fn load_chunks(keys: &BTreeSet<ChunkKey>) -> Result<HashMap<ChunkKey, Chunk>, FetchError> {
    let mut found = HashMap::new();
    let mut missing: BTreeMap<([i64; 2], i64), Vec<ChunkKey>> = BTreeMap::new();
    {
        let kept = CHUNK_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        for key in keys {
            if let Some((_, c)) = kept.iter().find(|(k, _)| k == key) {
                found.insert(*key, c.clone());
            }
        }
    }
    for key in keys {
        if found.contains_key(key) {
            continue;
        }
        match chunk_from_disk(*key) {
            Some(chunk) => {
                keep_chunk(*key, chunk.clone());
                found.insert(*key, chunk);
            }
            None => missing
                .entry((key.tile, key.chunk[1]))
                .or_default()
                .push(*key),
        }
    }
    if !missing.is_empty() && surface::offline() {
        return Err(FetchError::Offline("the network was missing lately".into()));
    }
    for ((tile, row), wanted) in missing {
        let Some(strips) = strips(tile)? else {
            for key in wanted {
                keep_chunk(key, None);
                found.insert(key, None);
            }
            continue;
        };
        let started = Instant::now();
        let chunks = read_chunk_row(tile, row, &strips)?;
        let read = Instant::now();
        ROWS_READ.fetch_add(1, Ordering::Relaxed);
        ROWS_READ_MS.fetch_add((read - started).as_millis() as u64, Ordering::Relaxed);
        for (column, cells) in chunks.into_iter().enumerate() {
            let key = ChunkKey {
                tile,
                chunk: [column as i64, row],
            };
            // The fastest level: the cells are mostly runs of bare ground, which it packs as well.
            let stored = miniz_oxide::deflate::compress_to_vec_zlib(&cells, 1);
            write_file(&chunk_path(key), &stored).map_err(io_error)?;
            if wanted.contains(&key) {
                let chunk = Some(Arc::new(cells));
                keep_chunk(key, chunk.clone());
                found.insert(key, chunk);
            }
        }
        ROWS_KEPT_MS.fetch_add(read.elapsed().as_millis() as u64, Ordering::Relaxed);
        surface::log_once(
            "canopy-row",
            &format!(
                "the canopy's first chunk row read in {} ms ({} MB read this session)",
                started.elapsed().as_millis(),
                FETCHED_BYTES.load(Ordering::Relaxed) >> 20
            ),
        );
    }
    Ok(found)
}

// ---------------------------------------------------------------------------------------------------------------------
// Pages.

/// A page's side, metres.
pub fn page_m() -> f64 {
    TEXEL_M * f64::from(PAGE_TEXELS)
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
pub fn grid(origin: [f64; 2]) -> (CanopyGrid, [i64; 2]) {
    let base = page_of(origin);
    let side = page_m();
    let grid = CanopyGrid {
        origin: [
            base[0] as f64 * side - origin[0],
            base[1] as f64 * side - origin[1],
        ],
        texel_m: TEXEL_M,
        slots: SLOTS,
    };
    (grid, base)
}

/// Page `index`'s map from its metres (x east, z south from its corner) to global zoom-17 pixels: bilinear between its
/// corners' (the theatre's frame turns against the mercator's away from its origin, and a page is small enough for it).
/// The corners' longitudes are unwrapped about the theatre's, so a page across the antimeridian stays continuous (its
/// columns wrap in `ChunkKey::of`).
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
            surface::mercator_px(p.lat, geo.lon + wrap_lon(p.lon - geo.lon), PX_ZOOM)
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

    /// The chunks under the page and its margin.
    fn chunks(&self) -> BTreeSet<ChunkKey> {
        let reach = MARGIN as f64 * CELL_M + CELL_M;
        let side = page_m();
        let edge = [
            [-reach, -reach],
            [side + reach, -reach],
            [-reach, side + reach],
            [side + reach, side + reach],
        ]
        .map(|m| self.px(m));
        let lo = [0, 1].map(|k| {
            edge.iter()
                .map(|p| p[k])
                .fold(f64::INFINITY, f64::min)
                .floor() as i64
        });
        let hi = [0, 1].map(|k| {
            edge.iter()
                .map(|p| p[k])
                .fold(f64::NEG_INFINITY, f64::max)
                .floor() as i64
        });
        // Chunk columns and rows, unwrapped (`ChunkKey::of` wraps them).
        let (ca, cb) = (
            lo.map(|p| p.div_euclid(CHUNK_PX)),
            hi.map(|p| p.div_euclid(CHUNK_PX)),
        );
        let mut out = BTreeSet::new();
        for y in ca[1]..=cb[1] {
            for x in ca[0]..=cb[0] {
                out.insert(ChunkKey::of([x * CHUNK_PX, y * CHUNK_PX]));
            }
        }
        out
    }
}

/// Page `index`'s grid of 2 m cells about `geo` (with its margin), each the tallest of the chunks' cells under it,
/// [`UNKNOWN`] where the map has no tile.
pub fn page_heights(geo: LatLon, index: [i64; 2], chunks: &HashMap<ChunkKey, Chunk>) -> Vec<u8> {
    let map = PageMap::new(geo, index);
    let width = PAGE_TEXELS as usize * CELLS + 2 * MARGIN;
    let mut out = vec![UNKNOWN; width * width];
    let mut last: Option<(ChunkKey, Option<&Vec<u8>>)> = None;
    for cy in 0..width {
        for cx in 0..width {
            let middle = [
                ((cx as f64 - MARGIN as f64) + 0.5) * CELL_M,
                ((cy as f64 - MARGIN as f64) + 0.5) * CELL_M,
            ];
            let mut tallest: Option<u8> = None;
            for d in [[-0.5, -0.5], [0.5, -0.5], [-0.5, 0.5], [0.5, 0.5]] {
                let g = map.px([middle[0] + d[0], middle[1] + d[1]]);
                let px = [g[0].floor() as i64, g[1].floor() as i64];
                let key = ChunkKey::of(px);
                let cells = match last {
                    Some((k, cells)) if k == key => cells,
                    _ => {
                        let cells = chunks.get(&key).and_then(|c| c.as_deref());
                        last = Some((key, cells));
                        cells
                    }
                };
                if let Some(cells) = cells {
                    let inside = px.map(|p| (p.rem_euclid(TILE_PX) % CHUNK_PX / 2) as usize);
                    let h = cells[inside[1] * CHUNK_CELLS + inside[0]];
                    tallest = Some(tallest.map_or(h, |t| t.max(h)));
                }
            }
            if let Some(h) = tallest {
                out[cy * width + cx] = h;
            }
        }
    }
    out
}

/// Page `index` about `geo` from `chunks`.
pub fn make_page(geo: LatLon, index: [i64; 2], chunks: &HashMap<ChunkKey, Chunk>) -> CanopyPage {
    CanopyPage::from_heights(&page_heights(geo, index, chunks), CELLS, MARGIN)
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

type Key = ([u64; 2], [i64; 2]);

fn key(geo: LatLon, index: [i64; 2]) -> Key {
    ([geo.lat.to_bits(), geo.lon.to_bits()], index)
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    pub queued: usize,
    pub working: usize,
    /// Pages made this session, how long the last batch took (ms, its reading and making), and the bytes read from
    /// the network.
    pub made: usize,
    pub last_batch_ms: u64,
    pub fetched_bytes: u64,
    /// Chunk rows read from the network, the milliseconds reading them took, and keeping their chunks on disk.
    pub rows_read: u64,
    pub rows_read_ms: u64,
    pub rows_kept_ms: u64,
    pub failed: usize,
}

struct State {
    geo: Option<LatLon>,
    wanted: VecDeque<[i64; 2]>,
    made: HashMap<Key, Arc<CanopyPage>>,
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

/// Page `index` about `geo`, if made.
pub fn made(geo: LatLon, index: [i64; 2]) -> Option<Arc<CanopyPage>> {
    lock().made.get(&key(geo, index)).cloned()
}

/// Asks for `pages` (in the order given) about `geo`, in place of whatever was asked before; a page made, or failed
/// lately, is left out.
pub fn want(geo: LatLon, pages: &[[i64; 2]]) {
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
            let k = key(geo, i);
            !state.made.contains_key(&k) && !state.failed.contains_key(&k)
        })
        .collect();
    state.geo = Some(geo);
    state.status.queued = wanted.len();
    state.wanted = wanted;
    if state.wanted.is_empty() {
        return;
    }
    if !state.worker {
        state.worker = true;
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-canopy", work);
        if let Err(e) = spawned {
            state.worker = false;
            eprintln!("the theatre's canopy: no worker thread: {e}");
        }
    }
    streamer().wake.notify_one();
}

pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        failed: state.failed.len(),
        fetched_bytes: FETCHED_BYTES.load(Ordering::Relaxed),
        rows_read: ROWS_READ.load(Ordering::Relaxed),
        rows_read_ms: ROWS_READ_MS.load(Ordering::Relaxed),
        rows_kept_ms: ROWS_KEPT_MS.load(Ordering::Relaxed),
        ..state.status
    }
}

fn work() {
    loop {
        let (geo, batch) = {
            let mut state = lock();
            loop {
                if let Some(geo) = state.geo
                    && let Some(first) = state.wanted.pop_front()
                {
                    // The nearest page, and the others asked for on its row (they share their chunk rows).
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
                    batch.retain(|&i| !state.made.contains_key(&key(geo, i)));
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (geo, batch);
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
        let result = make_batch(geo, &batch);
        let mut state = lock();
        state.status.working = 0;
        match result {
            Ok(pages) => {
                state.status.last_batch_ms = started.elapsed().as_millis() as u64;
                for (i, page) in pages {
                    let k = key(geo, i);
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
                surface::log_once("canopy-failed", &format!("the canopy: {e}"));
                // The batch waits a minute; what was asked after it would most likely fail the same way.
                let now = Instant::now();
                for &i in &batch {
                    state.failed.insert(key(geo, i), now);
                }
                state.wanted.clear();
                state.status.queued = 0;
            }
        }
    }
}

/// Reads the chunks under the batch's pages and makes them, a few threads at a time.
#[allow(clippy::type_complexity)]
fn make_batch(
    geo: LatLon,
    batch: &[[i64; 2]],
) -> Result<Vec<([i64; 2], Arc<CanopyPage>)>, FetchError> {
    let mut all = BTreeSet::new();
    for &i in batch {
        all.extend(PageMap::new(geo, i).chunks());
    }
    let chunks = load_chunks(&all)?;
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Arc<CanopyPage>>>> = Mutex::new(vec![None; batch.len()]);
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
                    let page = Arc::new(make_page(geo, i, &chunks));
                    results.lock().unwrap_or_else(|e| e.into_inner())[k] = Some(page);
                }
            });
        }
    });
    Ok(batch
        .iter()
        .copied()
        .zip(results.into_inner().unwrap_or_else(|e| e.into_inner()))
        .filter_map(|(i, p)| Some((i, p?)))
        .collect())
}

// ---------------------------------------------------------------------------------------------------------------------
// The layer's side.

/// Where the camera stood at the last frame a scene fed its pages (the scenario's frame, the theatre point).
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 2])>> = Mutex::new(None);

/// Gives a land layer being built (off the frame loop's thread, with its scene) the canopy's grid (its chart's (0, 0)
/// at theatre point `origin`) and uploads the made pages about where the camera last stood, so a scene rebuilt where
/// the old one stood stands its trees on them from its first frame. Nothing with the canopy off, nor on a land without
/// trees (the Medium terrain setting).
pub fn install(
    ctx: &mut naval_render::RenderContext<'_>,
    land: &mut naval_terrain::TerrainLayer,
    geo: LatLon,
    origin: [f64; 2],
) {
    if !enabled() || !land.settings().quality.trees {
        return;
    }
    let (grid, base) = grid(origin);
    if let Err(e) = land.set_canopy_grid(ctx, grid) {
        eprintln!("the theatre's canopy: {e}");
        return;
    }
    let last = *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    let frame = [geo.lat.to_bits(), geo.lon.to_bits()];
    let Some((_, at)) = last.filter(|(f, _)| *f == frame) else {
        return;
    };
    for index in block(at, grid.slots) {
        if let Some(page) = made(geo, index) {
            land.queue_canopy_page(local(index, base), page);
        }
    }
    land.flush_canopy_pages(ctx);
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
    asked: Vec<[i64; 2]>,
    asked_at: Option<Instant>,
    /// The block's pages not yet made.
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
        land.show_canopy(SHOWN.load(Ordering::Relaxed));
        let Some(grid) = land.canopy_grid() else {
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
        let base = page_of(origin);
        let mut missing = Vec::new();
        for &index in &self.block {
            let l = local(index, base);
            if land.holds_canopy_page(l) {
                continue;
            }
            match made(geo, index) {
                Some(page) => land.queue_canopy_page(l, page),
                None => missing.push(index),
            }
        }
        self.pending = missing.len();
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > RETRY_AFTER);
        if missing != self.asked || (!missing.is_empty() && stale) {
            want(geo, &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
    }
}

/// The canopy's state for the RPC (`theatre_state.canopy`): the grid's pages, those read by the trees, waiting for
/// their upload and still to be made, and the worker's state; none with no grid on screen.
pub fn status_json(
    land: Option<&naval_terrain::TerrainLayer>,
    feed: &Feed,
    fresh: bool,
) -> Option<serde_json::Value> {
    let land = land?;
    let grid = land.canopy_grid()?;
    let (read, uploading) = land.canopy_page_counts();
    let worker = status();
    let pages = (grid.slots * grid.slots) as usize;
    Some(serde_json::json!({
        "pages": pages,
        "texelM": TEXEL_M,
        "read": read,
        "uploading": uploading,
        "pending": if fresh { feed.pending } else { pages },
        "shown": SHOWN.load(Ordering::Relaxed),
        "worker": {
            "queued": worker.queued,
            "working": worker.working,
            "made": worker.made,
            "lastBatchMs": worker.last_batch_ms,
            "fetchedBytes": worker.fetched_bytes,
            "rowsRead": worker.rows_read,
            "rowsReadMs": worker.rows_read_ms,
            "rowsKeptMs": worker.rows_kept_ms,
            "failed": worker.failed,
        },
    }))
}

#[cfg(test)]
#[path = "canopy_tests.rs"]
mod tests;
