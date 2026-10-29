//! The dataset's big files (over [`super::WHOLE_FILE_BYTES`] compressed: the densest cities', Los Angeles' 171 MB,
//! Tokyo's 300 MB), read as they stream in: inflated a line at a time, each footprint written to the level-
//! [`SPLIT_LEVEL`] quadkey tile holding its middle (about 4 km on a side at Los Angeles' latitude). A chunk then reads
//! only the few tiles under it, so memory stays bounded however big the file is, and the tiles serve every scenario.
//!
//! A tile's file: [`MAGIC`], then each footprint as its height (f32, NaN where none), its ring's length (u16) and its
//! points (longitude and latitude, i32 in 1e-7 degrees: about a centimetre), little-endian. The tiles are written into
//! a folder of their own that is moved into place only once the whole stream has been read and its gzip trailers
//! (CRC-32 and length) checked, so a download cut short or damaged is never taken for the whole file.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{RawFootprint, parse_line, quadkey};

/// The tiles' quadkey level: 256 tiles to a level-9 file.
pub const SPLIT_LEVEL: u32 = 13;
/// The first bytes of a tile's file (its format's version in the last two).
pub const MAGIC: &[u8; 8] = b"NBFOOT01";
/// A tile's footprints are held in memory up to this many bytes before they are appended to its file: 16 MB at most
/// for a file's 256 tiles.
const FLUSH_BYTES: usize = 64 << 10;

/// What a split wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub footprints: usize,
    pub tiles: usize,
    /// Lines that held no footprint.
    pub bad: usize,
}

/// A reader counting the bytes through it into `count` (the compressed bytes, for the worker's status).
struct Counting<'a, R> {
    inner: R,
    count: &'a AtomicU64,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// A footprint's tile: the quadkey of [`SPLIT_LEVEL`] holding its ring's mean point.
pub fn tile_of(ring: &[[f64; 2]]) -> String {
    let n = ring.len().max(1) as f64;
    let (lon, lat) = ring
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p[0] / n, s.1 + p[1] / n));
    quadkey(lat, lon, SPLIT_LEVEL)
}

/// Appends one footprint's record to `out`.
pub fn encode(raw: &RawFootprint, out: &mut Vec<u8>) {
    let n = raw.ring.len().min(usize::from(u16::MAX));
    out.extend(raw.height.unwrap_or(f32::NAN).to_le_bytes());
    out.extend((n as u16).to_le_bytes());
    for p in &raw.ring[..n] {
        for v in [p[0], p[1]] {
            out.extend(((v * 1e7).round() as i32).to_le_bytes());
        }
    }
}

/// A tile file's footprints; an error for a file that is not one or is cut short.
pub fn decode(bytes: &[u8]) -> Result<Vec<RawFootprint>, String> {
    let mut rest = bytes
        .strip_prefix(MAGIC.as_slice())
        .ok_or("not a footprint tile")?;
    let mut out = Vec::new();
    while !rest.is_empty() {
        if rest.len() < 6 {
            return Err("a footprint cut short".into());
        }
        let height = f32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
        let n = usize::from(u16::from_le_bytes([rest[4], rest[5]]));
        let points = rest.get(6..6 + n * 8).ok_or("a ring cut short")?;
        let ring = points
            .as_chunks::<8>()
            .0
            .iter()
            .map(|p| {
                let lon = i32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                let lat = i32::from_le_bytes([p[4], p[5], p[6], p[7]]);
                [f64::from(lon) * 1e-7, f64::from(lat) * 1e-7]
            })
            .collect();
        out.push(RawFootprint {
            ring,
            height: (!height.is_nan()).then_some(height),
        });
        rest = &rest[6 + n * 8..];
    }
    Ok(out)
}

/// The tiles being written: each one's footprints not yet appended, and those whose file exists.
struct Tiles<'a> {
    dir: &'a Path,
    pending: HashMap<String, Vec<u8>>,
    started: HashSet<String>,
}

impl Tiles<'_> {
    fn add(&mut self, raw: &RawFootprint) -> std::io::Result<()> {
        let tile = tile_of(&raw.ring);
        let buf = self.pending.entry(tile.clone()).or_default();
        encode(raw, buf);
        if buf.len() >= FLUSH_BYTES {
            self.flush(&tile)?;
        }
        Ok(())
    }

    fn flush(&mut self, tile: &str) -> std::io::Result<()> {
        let Some(buf) = self.pending.get_mut(tile).filter(|b| !b.is_empty()) else {
            return Ok(());
        };
        let path = self.dir.join(format!("{tile}.bin"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        if self.started.insert(tile.to_owned()) {
            file.write_all(MAGIC)?;
        }
        file.write_all(buf)?;
        buf.clear();
        Ok(())
    }

    fn finish(mut self) -> std::io::Result<usize> {
        let tiles: Vec<String> = self.pending.keys().cloned().collect();
        for tile in tiles {
            self.flush(&tile)?;
        }
        Ok(self.started.len())
    }
}

/// Splits a gzip stream of the dataset's lines (one member or several) into tile files in `dir`, which must be new
/// and empty; `count` takes the compressed bytes read. An error when the stream is cut short or damaged (a deflate
/// stream left open, a trailer's CRC-32 or length wrong) or a file cannot be written: the caller then throws `dir`
/// away.
pub fn split(reader: impl Read, dir: &Path, count: &AtomicU64) -> Result<Summary, String> {
    let gz = flate2::read::MultiGzDecoder::new(BufReader::with_capacity(
        1 << 16,
        Counting {
            inner: reader,
            count,
        },
    ));
    let mut lines = BufReader::with_capacity(1 << 16, gz);
    let mut tiles = Tiles {
        dir,
        pending: HashMap::new(),
        started: HashSet::new(),
    };
    let mut summary = Summary::default();
    let mut line = Vec::with_capacity(4096);
    loop {
        line.clear();
        let n = lines
            .read_until(b'\n', &mut line)
            .map_err(|e| format!("reading the stream: {e}"))?;
        if n == 0 {
            break;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            summary.bad += 1;
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        match parse_line(text) {
            Some(raw) => {
                tiles
                    .add(&raw)
                    .map_err(|e| format!("writing a tile: {e}"))?;
                summary.footprints += 1;
            }
            None => summary.bad += 1,
        }
    }
    summary.tiles = tiles.finish().map_err(|e| format!("writing a tile: {e}"))?;
    Ok(summary)
}

/// Splits `reader` into the folder `done`: into a working folder beside it first, moved into place only when the
/// whole stream was read; on any error the working folder is removed and `done` left as it was (absent).
pub fn split_into(reader: impl Read, done: &Path, count: &AtomicU64) -> Result<Summary, String> {
    let parent = done.parent().ok_or("no parent folder")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    let mut work = done.as_os_str().to_owned();
    work.push(format!(".{}.part", std::process::id()));
    let work = PathBuf::from(work);
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let result = split(reader, &work, count).and_then(|summary| {
        match std::fs::rename(&work, done) {
            Ok(()) => Ok(summary),
            // Another process finished the same file first: its tiles stand.
            Err(_) if done.is_dir() => {
                let _ = std::fs::remove_dir_all(&work);
                Ok(summary)
            }
            Err(e) => Err(format!("moving {} into place: {e}", work.display())),
        }
    });
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&work);
    }
    result
}

/// The footprints of tile `tile` in a split folder: none when the tile has no file (no footprint stands in it); an
/// error when the folder itself is gone.
pub fn read_tile(dir: &Path, tile: &str) -> Result<Vec<RawFootprint>, String> {
    let path = dir.join(format!("{tile}.bin"));
    match std::fs::read(&path) {
        Ok(bytes) => decode(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && dir.is_dir() => Ok(Vec::new()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}
