//! The sea floor of a release from `land-v2` on (step C, BAKE-3): GEBCO's global grid, the 2026 release, read from one
//! pinned Cloud-Optimized GeoTIFF by HTTP range.
//!
//! - **Why GEBCO.** Compared at the places the game sails (BAKE-3's probe table): Terrarium's sea is ETOPO1 (1′, about
//!   1.8 km) and, on coasts it fills, nothing at all: it reads 0 m over False Bay, the Monterey canyon and 7 km off Big
//!   Sur (NED's sea fill), and has no Varne Bank (−19 to −22 m where GEBCO shows the bank at 6–11 m). GEBCO's 15″ grid
//!   (about 460 m) shows them all; in the open ocean the two agree within 1 %.
//! - **The file.** `GEBCO_2026.tif` in the Australian Antarctic Division's copy on Source Cooperative
//!   ([`BUCKET`]/[`KEY`]): a little-endian BigTIFF, 86,400 × 43,200 signed 16-bit heights in metres (pixel-is-area, row
//!   0 the northern edge at 90° N, column 0 the western at 180° W, 1/240° a pixel), tiled 512², DEFLATE with the
//!   horizontal predictor. One object: its ETag and length go into the source manifest when the release is made, and
//!   every read sends that ETag (`If-Match`), so a changed file fails the bake. GEBCO's licence: free to use with the
//!   attribution [`ATTRIBUTION`].
//! - **By place.** A lattice point's height is the bilinear interpolation of the four pixels about it, its pixel and
//!   fraction exact integers of its lattice steps ([`pixel_of`]), so every square that holds the point reads the same
//!   value. Columns wrap at ±180°.
use std::collections::HashMap;
use std::sync::Arc;

use crate::BakeError;
use crate::finish::lattice::{Plane, Region};
use naval_land_format::lattice::PER_DEGREE;

/// The bucket (S3-compatible, listable) and the object.
pub const BUCKET: &str = "https://data.source.coop/ausantarctic";
pub const KEY: &str = "gebco/GEBCO_2026.tif";
/// The credit the squares' heights carry where they are sea.
pub const ATTRIBUTION: &str = "GEBCO Compilation Group (2026) GEBCO 2026 Grid";
/// Pixels a degree, and the image's size.
pub const PER_DEGREE_G: i64 = 240;
pub const WIDTH: usize = 360 * PER_DEGREE_G as usize;
pub const HEIGHT: usize = 180 * PER_DEGREE_G as usize;
/// The header's first read: the first IFD and its tile tables (about 180 kB) lie inside it.
pub const HEADER_BYTES: u64 = 262_144;

/// The object's URL.
pub fn url() -> String {
    format!("{BUCKET}/{KEY}")
}

/// The full-resolution image: its blocks and their byte ranges.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub block: usize,
    /// Blocks across a row of blocks.
    pub across: usize,
    pub offsets: Vec<u64>,
    pub counts: Vec<u64>,
}

/// Why a header did not parse: `Short(n)` when it reaches byte `n` past those read (read that many and parse again).
#[derive(Debug, PartialEq)]
pub enum HeaderError {
    Short(usize),
    Bad(String),
}

/// The first IFD of the file, refusing anything but GEBCO's layout (a little-endian BigTIFF of signed 16-bit heights,
/// DEFLATE with the horizontal predictor, one plane, square blocks, 1/240° pixels from 180° W, 90° N).
pub fn parse_header(bytes: &[u8]) -> Result<Header, HeaderError> {
    let bad = |why: String| HeaderError::Bad(format!("GEBCO: {why}"));
    let need = |end: usize| {
        if end > bytes.len() {
            Err(HeaderError::Short(end))
        } else {
            Ok(())
        }
    };
    need(16)?;
    if bytes[..4] != [0x49, 0x49, 0x2b, 0x00] {
        return Err(bad("not a little-endian BigTIFF".into()));
    }
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let offset = u64_at(8) as usize;
    need(offset + 8)?;
    let count = u64_at(offset) as usize;
    if count > 64 {
        return Err(bad(format!("{count} tags in its first IFD")));
    }
    need(offset + 8 + 20 * count)?;
    let mut tags: HashMap<u16, Vec<f64>> = HashMap::new();
    for e in 0..count {
        let at = offset + 8 + 20 * e;
        let (tag, kind, n) = (u16_at(at), u16_at(at + 2), u64_at(at + 4) as usize);
        let size: usize = match kind {
            3 => 2,
            4 => 4,
            12 | 16 => 8,
            _ => continue,
        };
        let total = size
            .checked_mul(n)
            .ok_or_else(|| bad("a tag's length overflows".into()))?;
        let start = if total <= 8 {
            at + 12
        } else {
            u64_at(at + 12) as usize
        };
        need(start + total)?;
        let values = (0..n)
            .map(|k| {
                let p = start + k * size;
                match kind {
                    3 => f64::from(u16_at(p)),
                    4 => f64::from(u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap())),
                    12 => f64::from_le_bytes(bytes[p..p + 8].try_into().unwrap()),
                    _ => u64_at(p) as f64,
                }
            })
            .collect();
        tags.insert(tag, values);
    }
    let first = |tag: u16| {
        tags.get(&tag)
            .and_then(|v| v.first().copied())
            .ok_or_else(|| bad(format!("no TIFF tag {tag}")))
    };
    let layout = [
        first(256)?,
        first(257)?,
        first(258)?,
        first(259)?,
        first(277)?,
        first(284)?,
        first(317)?,
        first(339)?,
    ];
    if layout != [WIDTH as f64, HEIGHT as f64, 16.0, 8.0, 1.0, 1.0, 2.0, 2.0] {
        return Err(bad(format!(
            "expected {WIDTH} × {HEIGHT} signed 16-bit heights, DEFLATE, the horizontal predictor, one plane \
             ({layout:?})"
        )));
    }
    let block = first(322)? as usize;
    if first(323)? as usize != block || block == 0 || !block.is_multiple_of(16) {
        return Err(bad("blocks are not square multiples of 16".into()));
    }
    let across = WIDTH.div_ceil(block);
    let blocks = across * HEIGHT.div_ceil(block);
    let take = |tag: u16| -> Vec<u64> {
        tags.get(&tag)
            .map(|v| v.iter().map(|&x| x as u64).collect())
            .unwrap_or_default()
    };
    let (offsets, counts) = (take(324), take(325));
    if offsets.len() != blocks || counts.len() != blocks {
        return Err(bad("the block tables do not cover the image".into()));
    }
    let scale = tags.get(&33550).cloned().unwrap_or_default();
    let tie = tags.get(&33922).cloned().unwrap_or_default();
    let pixel = 1.0 / PER_DEGREE_G as f64;
    if scale.len() < 2
        || tie.len() < 5
        || (scale[0] - pixel).abs() > 1e-12
        || (scale[1] - pixel).abs() > 1e-12
        || (tie[3] + 180.0).abs() > 1e-9
        || (tie[4] - 90.0).abs() > 1e-9
    {
        return Err(bad(format!(
            "not 1/240° pixels from 180° W, 90° N ({scale:?}, {tie:?})"
        )));
    }
    Ok(Header {
        block,
        across,
        offsets,
        counts,
    })
}

/// One block's heights, `block`² row-major: inflated, the predictor undone (a running sum of the 16-bit values along
/// each row, wrapping).
pub fn decode_block(compressed: &[u8], block: usize) -> Result<Vec<i16>, BakeError> {
    let raw = crate::ntf::inflate(compressed, 2 * block * block)?;
    if raw.len() < 2 * block * block {
        return Err(BakeError::Corrupt(format!(
            "a GEBCO block inflated to {} bytes, not {}",
            raw.len(),
            2 * block * block
        )));
    }
    let mut out = vec![0i16; block * block];
    for r in 0..block {
        let mut sum = 0i16;
        for c in 0..block {
            let at = r * block + c;
            sum = sum.wrapping_add(i16::from_le_bytes([raw[2 * at], raw[2 * at + 1]]));
            out[at] = sum;
        }
    }
    Ok(out)
}

/// Lattice point (`i`, `j`)'s pixel (column, row: the north-west of the four the interpolation reads) and its fraction
/// past it, exactly. A pixel's middle stands at (90° − (r + ½)/240, −180° + (c + ½)/240) and a lattice step is 3/20 of
/// a pixel (240 pixels a degree, 1,600 steps), so the point lies `(3 · (j + 288,000) − 10) / 20` columns and
/// `(3 · (144,000 − i) − 10) / 20` rows past the first middle: integers over 20. The column is not yet wrapped, nor the
/// row held to the image.
#[inline]
pub fn pixel_of(i: i32, j: i32) -> ([i64; 2], [f64; 2]) {
    const NUM: i64 = 3;
    const DEN: i64 = 20;
    const _: () = assert!(PER_DEGREE_G * DEN == NUM * PER_DEGREE as i64);
    let x = NUM * (i64::from(j) + 180 * i64::from(PER_DEGREE)) - DEN / 2;
    let y = NUM * (90 * i64::from(PER_DEGREE) - i64::from(i)) - DEN / 2;
    let (c, u) = (x.div_euclid(DEN), x.rem_euclid(DEN));
    let (r, v) = (y.div_euclid(DEN), y.rem_euclid(DEN));
    ([c, r], [u as f64 / DEN as f64, v as f64 / DEN as f64])
}

/// The pixel the interpolation reads at column `c` (wrapped onto the world) and row `r` (held to the image), and its
/// block and place in the block.
#[inline]
fn locate(header: &Header, c: i64, r: i64) -> (usize, usize) {
    let c = c.rem_euclid(WIDTH as i64) as usize;
    let r = r.clamp(0, HEIGHT as i64 - 1) as usize;
    let b = header.block;
    ((r / b) * header.across + c / b, (r % b) * b + c % b)
}

/// The blocks the heights over `region` read, in index order: those holding the pixels from the north-west one of
/// its first point to the south-east one of its last.
pub fn blocks_for(header: &Header, region: &Region) -> Vec<usize> {
    let ([c0, r0], _) = pixel_of(region.top, region.left);
    let ([c1, r1], _) = pixel_of(region.i(region.rows - 1), region.j(region.columns - 1));
    let b = header.block as i64;
    let mut out = std::collections::BTreeSet::new();
    let rows = (r0.clamp(0, HEIGHT as i64 - 1) / b)..=((r1 + 1).clamp(0, HEIGHT as i64 - 1) / b);
    for br in rows {
        let mut c = c0;
        while c <= c1 + 1 {
            out.insert(locate(header, c, br * b).0);
            // The next block's first column.
            c = (c.div_euclid(b) + 1) * b;
        }
        out.insert(locate(header, c1 + 1, br * b).0);
    }
    out.into_iter().collect()
}

/// The decoded blocks a bake holds, by index.
pub type Blocks = HashMap<usize, Arc<Vec<i16>>>;

/// The height at lattice point (`i`, `j`), metres: the bilinear interpolation of its four pixels, each read from
/// `blocks` (which must hold every block [`blocks_for`] names).
pub fn height_at(header: &Header, blocks: &Blocks, i: i32, j: i32) -> Result<f64, BakeError> {
    let ([c, r], [u, v]) = pixel_of(i, j);
    let at = |dc: i64, dr: i64| -> Result<f64, BakeError> {
        let (b, k) = locate(header, c + dc, r + dr);
        let block = blocks
            .get(&b)
            .ok_or_else(|| BakeError::Missing(format!("GEBCO block {b}")))?;
        Ok(f64::from(block[k]))
    };
    let (a, b, d, e) = (at(0, 0)?, at(1, 0)?, at(0, 1)?, at(1, 1)?);
    Ok(a * (1.0 - u) * (1.0 - v) + b * u * (1.0 - v) + d * (1.0 - u) * v + e * u * v)
}

/// The heights over `region`, by place.
pub fn heights(header: &Header, blocks: &Blocks, region: &Region) -> Result<Plane<f64>, BakeError> {
    let mut out = Plane::filled(*region, 0f64);
    for r in 0..region.rows {
        let i = region.i(r);
        for c in 0..region.columns {
            out.set(r, c, height_at(header, blocks, i, region.j(c))?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lattice_points_pixel_is_exact() {
        // 0° N 0° E: half a pixel past the middle of column 43,199 (its middle 1/480° west) and of row 21,599.
        assert_eq!(pixel_of(0, 0), ([43_199, 21_599], [0.5, 0.5]));
        // The north-west corner is half a pixel before the first middle; seven steps on, 21/20 − 1/2 past it.
        assert_eq!(pixel_of(144_000, -288_000), ([-1, -1], [0.5, 0.5]));
        let ([c, r], [u, v]) = pixel_of(144_000 - 7, -288_000 + 7);
        assert_eq!((c, r), (0, 0));
        assert!((u - 0.55).abs() < 1e-15 && (v - 0.55).abs() < 1e-15);
        // East of 180°: the same pixels a turn on, wrapped when read.
        let ([c, _], [u, _]) = pixel_of(0, 288_000 + 7);
        assert_eq!(c, 86_400);
        assert!((u - 0.55).abs() < 1e-15);
        let header = Header {
            block: 512,
            across: WIDTH.div_ceil(512),
            offsets: vec![],
            counts: vec![],
        };
        assert_eq!(locate(&header, 86_400, 0), locate(&header, 0, 0));
        assert_eq!(locate(&header, -1, 5).0, header.across - 1);
    }

    #[test]
    fn a_block_undoes_the_horizontal_predictor() {
        let block = 16;
        let values: Vec<i16> = (0..block * block)
            .map(|k| ((k * 37) % 400) as i16 - 200)
            .collect();
        let mut diffs = vec![];
        for r in 0..block {
            let mut prev = 0i16;
            for c in 0..block {
                let v = values[r * block + c];
                diffs.extend_from_slice(&v.wrapping_sub(prev).to_le_bytes());
                prev = v;
            }
        }
        let z = crate::ntf::zlib(&diffs);
        assert_eq!(decode_block(&z, block).unwrap(), values);
    }

    #[test]
    fn heights_interpolate_their_four_pixels() {
        let block = 512;
        let header = Header {
            block,
            across: WIDTH.div_ceil(block),
            offsets: vec![],
            counts: vec![],
        };
        // A plane rising 1 m a column east and 10 m a row south about 51° N 1.4° E.
        let region = Region {
            top: (51.01 * 1600.0) as i32,
            left: (1.38 * 1600.0) as i32,
            rows: 20,
            columns: 20,
        };
        let mut blocks = Blocks::new();
        for b in blocks_for(&header, &region) {
            let (br, bc) = (b / header.across, b % header.across);
            let tile: Vec<i16> = (0..block * block)
                .map(|k| {
                    let (r, c) = (br * block + k / block, bc * block + k % block);
                    (c % 1000) as i16 + 10 * (r % 1000) as i16
                })
                .collect();
            blocks.insert(b, Arc::new(tile));
        }
        let h = heights(&header, &blocks, &region).unwrap();
        for r in 0..region.rows {
            for c in 0..region.columns {
                let (i, j) = (region.i(r), region.j(c));
                let ([pc, pr], [u, v]) = pixel_of(i, j);
                let want = (pc % 1000) as f64 + u + 10.0 * ((pr % 1000) as f64 + v);
                assert!((h.at(r, c) - want).abs() < 1e-9, "{i} {j}");
            }
        }
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::release::fixtures::{gebco_header, gebco_pixel, gebco_range};

    #[test]
    fn the_fixture_reads_back_through_the_header_and_its_blocks() {
        let header = parse_header(&gebco_header()).unwrap();
        assert_eq!((header.block, header.across), (512, 169));
        let region = Region {
            top: -34 * 1600 + 3,
            left: 18 * 1600 + 900,
            rows: 7,
            columns: 9,
        };
        let mut blocks = Blocks::new();
        for b in blocks_for(&header, &region) {
            let bytes = gebco_range(header.offsets[b], header.counts[b]);
            blocks.insert(b, Arc::new(decode_block(&bytes, header.block).unwrap()));
        }
        let h = heights(&header, &blocks, &region).unwrap();
        for r in 0..region.rows {
            for c in 0..region.columns {
                let ([pc, pr], [u, v]) = pixel_of(region.i(r), region.j(c));
                let p = |dc: i64, dr: i64| {
                    f64::from(gebco_pixel((pc + dc) as usize, (pr + dr) as usize))
                };
                let want = p(0, 0) * (1.0 - u) * (1.0 - v)
                    + p(1, 0) * u * (1.0 - v)
                    + p(0, 1) * (1.0 - u) * v
                    + p(1, 1) * u * v;
                assert_eq!(h.at(r, c), want);
            }
        }
    }
}
