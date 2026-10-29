//! Copernicus DEM GLO-30: the AWS open-data copy, one Cloud-Optimized GeoTIFF per 1 × 1 degree cell named by its
//! south-west corner. Float32 heights above the EGM2008 geoid, pixel-is-point with pixel (0, 0) on the cell's
//! north-west corner, 3600 rows of 1 arcsecond and, by latitude band, 3600 to 360 columns ([`columns_at`]). A cell with
//! no land has no file (403 or 404): it is sea, height 0. Files are tiled 1024², DEFLATE with the floating-point
//! predictor; only the full-resolution image (the first IFD, in the first 64 KB) is read, block by block, by HTTP range.
//! This module only parses and samples (the TypeScript bake's `glo30.ts`, itself a port of `scripts/theatre/glo30.py`):
//! the caller reads the bytes.
//!
//! Licence: free to use with the attribution [`ATTRIBUTION`], which a baked region's `source` carries.

use crate::BakeError;

pub const HOST: &str = "copernicus-dem-30m.s3.amazonaws.com";
pub const ATTRIBUTION: &str = "© DLR e.V. 2010-2014 and © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights reserved";
/// A header's first read: the IFD and its tile tables lie well inside it.
pub const HEADER_BYTES: u64 = 65_536;

/// The file of the cell whose south-west corner is (lat, lon), whole degrees.
pub fn name(lat: i32, lon: i32) -> String {
    format!(
        "Copernicus_DSM_COG_10_{}{:02}_00_{}{:03}_00_DEM",
        if lat >= 0 { 'N' } else { 'S' },
        lat.abs(),
        if lon >= 0 { 'E' } else { 'W' },
        lon.abs()
    )
}

pub fn url(lat: i32, lon: i32) -> String {
    let name = name(lat, lon);
    format!("https://{HOST}/{name}/{name}.tif")
}

/// Columns of a cell whose south edge is at `lat` (the GLO-30 latitude bands).
pub fn columns_at(lat: i32) -> usize {
    let a = if lat < 0 {
        lat.abs().max((lat + 1).abs())
    } else {
        lat.abs()
    };
    for (limit, columns) in [(50, 3600), (60, 2400), (70, 1800), (80, 1200), (85, 720)] {
        if a < limit {
            return columns;
        }
    }
    360
}

/// The full-resolution image of one file: its size, square blocks, their byte ranges, and its georeference.
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub width: usize,
    pub height: usize,
    pub block: usize,
    /// Blocks across a row of blocks.
    pub across: usize,
    pub offsets: Vec<u64>,
    pub counts: Vec<u64>,
    pub dx: f64,
    pub dy: f64,
    pub west: f64,
    pub north: f64,
}

/// What a header parse needs: `Short(n)` when the header reaches byte `n` past those read (read that many and parse
/// again).
#[derive(Debug, PartialEq)]
pub enum HeaderError {
    Short(usize),
    Bad(String),
}

/// The first IFD of a little-endian classic TIFF, refusing anything but GLO-30's layout (Float32, DEFLATE, the
/// floating-point predictor, one plane, square blocks, 1 arcsecond rows across one degree).
pub fn parse_header(bytes: &[u8], label: &str) -> Result<Header, HeaderError> {
    let header = parse_tiled(bytes, label)?;
    let (dx, dy) = (header.dx, header.dy);
    if (dy - 1.0 / 3600.0).abs() > 1e-12 || (dx * header.width as f64 - 1.0).abs() > 1e-9 {
        return Err(HeaderError::Bad(format!(
            "{label}: unexpected pixel size {dx}, {dy}"
        )));
    }
    Ok(header)
}

/// The first IFD of a little-endian classic tiled TIFF of Float32 heights (DEFLATE, the floating-point predictor, one
/// plane, square blocks) and its georeference, whatever its pixel: GLO-30's cells ([`parse_header`]) and NOAA's ETOPO
/// 2022 tiles (`seabed`).
pub fn parse_tiled(bytes: &[u8], label: &str) -> Result<Header, HeaderError> {
    let bad = |why: String| HeaderError::Bad(format!("{label}: {why}"));
    let u16_at = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    if bytes.len() < 8 || bytes[..4] != [0x49, 0x49, 0x2a, 0x00] {
        return Err(bad("not a little-endian TIFF".into()));
    }
    let offset = u32_at(4) as usize;
    if offset + 2 > bytes.len() {
        return Err(HeaderError::Short(offset + 2));
    }
    let count = u16_at(offset) as usize;
    if offset + 2 + 12 * count > bytes.len() {
        return Err(HeaderError::Short(offset + 2 + 12 * count));
    }
    let mut tags: std::collections::HashMap<u16, Vec<f64>> = std::collections::HashMap::new();
    for e in 0..count {
        let at = offset + 2 + 12 * e;
        let (tag, kind, n, value) = (
            u16_at(at),
            u16_at(at + 2),
            u32_at(at + 4) as usize,
            u32_at(at + 8) as usize,
        );
        let size = match kind {
            3 => 2,
            4 => 4,
            12 | 16 => 8,
            _ => continue,
        };
        let total = size * n;
        let start = if total <= 4 { at + 8 } else { value };
        if start + total > bytes.len() {
            return Err(HeaderError::Short(start + total));
        }
        let values = (0..n)
            .map(|k| {
                let p = start + k * size;
                match kind {
                    3 => u16_at(p) as f64,
                    4 => u32_at(p) as f64,
                    12 => f64::from_le_bytes(bytes[p..p + 8].try_into().unwrap()),
                    _ => u64::from_le_bytes(bytes[p..p + 8].try_into().unwrap()) as f64,
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
    let (width, height) = (first(256)? as usize, first(257)? as usize);
    let layout = [
        first(258)?,
        first(259)?,
        first(317)?,
        first(339)?,
        first(284)?,
    ];
    if layout != [32.0, 8.0, 3.0, 3.0, 1.0] {
        return Err(bad(format!(
            "expected Float32, DEFLATE, the floating-point predictor, one plane ({layout:?})"
        )));
    }
    let block = first(322)? as usize;
    if first(323)? as usize != block {
        return Err(bad("blocks are not square".into()));
    }
    let offsets: Vec<u64> = tags
        .get(&324)
        .map(|v| v.iter().map(|&x| x as u64).collect())
        .unwrap_or_default();
    let counts: Vec<u64> = tags
        .get(&325)
        .map(|v| v.iter().map(|&x| x as u64).collect())
        .unwrap_or_default();
    let across = width.div_ceil(block);
    if offsets.len() < across * height.div_ceil(block) || counts.len() != offsets.len() {
        return Err(bad("the block tables do not cover the image".into()));
    }
    let scale = tags.get(&33550).cloned().unwrap_or_default();
    let tie = tags.get(&33922).cloned().unwrap_or_default();
    if scale.len() < 2 || tie.len() < 5 {
        return Err(bad("no georeference".into()));
    }
    let (dx, dy) = (scale[0], scale[1]);
    if !(dx > 0.0 && dy > 0.0) {
        return Err(bad(format!("unexpected pixel size {dx}, {dy}")));
    }
    Ok(Header {
        width,
        height,
        block,
        across,
        offsets,
        counts,
        dx,
        dy,
        west: tie[3],
        north: tie[4],
    })
}

/// One block's heights, `block`² row-major: inflated, the predictor undone (a running byte sum along each row, then the
/// four planes, most significant first, back into floats).
pub fn decode_block(compressed: &[u8], block: usize) -> Result<Vec<f32>, BakeError> {
    let stride = 4 * block;
    let raw = crate::ntf::inflate(compressed, block * stride)?;
    if raw.len() < block * stride {
        return Err(BakeError::Corrupt(format!(
            "a GLO-30 block inflated to {} bytes, not {}",
            raw.len(),
            block * stride
        )));
    }
    let mut out = vec![0f32; block * block];
    let mut row = vec![0u8; stride];
    for r in 0..block {
        let mut sum = 0u8;
        for k in 0..stride {
            sum = sum.wrapping_add(raw[r * stride + k]);
            row[k] = sum;
        }
        for c in 0..block {
            out[r * block + c] = f32::from_be_bytes([
                row[c],
                row[block + c],
                row[2 * block + c],
                row[3 * block + c],
            ]);
        }
    }
    Ok(out)
}

/// The mosaic a set of points samples, as glo30.py `sample` lays it out: the cells two pixels past the points' bounds
/// (`s0..=s1` south edges, `w0..=w1` west edges), at the finest longitude spacing among those bands (`across` columns a
/// degree), rows of 1 arcsecond counted down from `s1 + 1` and columns east from `w0`, cropped to `top_row..=bottom_row`
/// and `left_col..=right_col`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    pub s0: i32,
    pub s1: i32,
    pub w0: i32,
    pub w1: i32,
    pub across: usize,
    pub dy: f64,
    pub dx: f64,
    pub top_row: i64,
    pub bottom_row: i64,
    pub left_col: i64,
    pub right_col: i64,
}

pub fn window(lat_min: f64, lat_max: f64, lon_min: f64, lon_max: f64) -> Window {
    let s0 = (lat_min - 2.0 / 3600.0).floor() as i32;
    let s1 = (lat_max + 2.0 / 3600.0).floor() as i32;
    let w0 = (lon_min - 2.0 / 360.0).floor() as i32;
    let w1 = (lon_max + 2.0 / 360.0).floor() as i32;
    let across = (s0..=s1).map(columns_at).max().unwrap_or(3600);
    let (dy, dx) = (1.0 / 3600.0, 1.0 / across as f64);
    Window {
        s0,
        s1,
        w0,
        w1,
        across,
        dy,
        dx,
        top_row: ((s1 as f64 + 1.0 - lat_max) / dy).floor() as i64 - 1,
        bottom_row: ((s1 as f64 + 1.0 - lat_min) / dy).ceil() as i64 + 1,
        left_col: ((lon_min - w0 as f64) / dx).floor() as i64 - 1,
        right_col: ((lon_max - w0 as f64) / dx).ceil() as i64 + 1,
    }
}

/// One cell's share of a window: the mosaic's rows `r0..r1` and columns `c0..c1` it fills, and the cell's own rows and
/// columns it reads for them (a coarser band's columns are stretched along its rows).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice {
    pub r0: i64,
    pub r1: i64,
    pub c0: i64,
    pub c1: i64,
    pub first: i64,
    pub c_first: i64,
    pub read_r0: usize,
    pub read_r1: usize,
    pub read_c0: usize,
    pub read_c1: usize,
}

/// The cell (s, w)'s slice of `win`, its image `width` columns wide; None when the window does not reach it.
pub fn slice(win: &Window, s: i32, w: i32, width: usize) -> Option<Slice> {
    let first = (win.s1 - s) as i64 * 3600;
    let r0 = win.top_row.max(first);
    let r1 = (win.bottom_row + 1).min(first + 3600);
    let c_first = (w - win.w0) as i64 * win.across as i64;
    let c0 = win.left_col.max(c_first);
    let c1 = (win.right_col + 1).min(c_first + win.across as i64);
    if r0 >= r1 || c0 >= c1 {
        return None;
    }
    let (mut read_c0, mut read_c1) = (c0 - c_first, c1 - c_first);
    if width != win.across {
        let ratio = width as f64 / win.across as f64;
        let a = |c: i64| (((c - c_first) as f64 * ratio).floor() as i64).clamp(0, width as i64 - 1);
        read_c0 = a(c0);
        read_c1 = (a(c1 - 1) + 1).min(width as i64 - 1) + 1;
    }
    Some(Slice {
        r0,
        r1,
        c0,
        c1,
        first,
        c_first,
        read_r0: (r0 - first) as usize,
        read_r1: (r1 - first) as usize,
        read_c0: read_c0 as usize,
        read_c1: read_c1 as usize,
    })
}

/// The blocks of a cell a slice reads, by index.
pub fn slice_blocks(header: &Header, slice: &Slice) -> Vec<usize> {
    let b = header.block;
    let mut out = vec![];
    for by in slice.read_r0 / b..=(slice.read_r1 - 1) / b {
        for bx in slice.read_c0 / b..=(slice.read_c1 - 1) / b {
            out.push(by * header.across + bx);
        }
    }
    out
}

/// A cell as the mosaic reads it: its header and the blocks its slice needs.
pub trait Cell {
    fn header(&self) -> &Header;
    /// Block `k`; every block a slice reads must be there.
    fn block(&self, k: usize) -> Option<&[f32]>;
}

/// A cell held in memory: its header and its decoded blocks by index.
pub struct HeldCell {
    pub header: Header,
    pub blocks: std::collections::HashMap<usize, std::sync::Arc<Vec<f32>>>,
}

impl Cell for HeldCell {
    fn header(&self) -> &Header {
        &self.header
    }
    fn block(&self, k: usize) -> Option<&[f32]> {
        self.blocks.get(&k).map(|b| b.as_slice())
    }
}

/// Rows `r0..r1` and columns `c0..c1` of a cell, row-major.
fn read_cell(
    cell: &dyn Cell,
    r0: usize,
    r1: usize,
    c0: usize,
    c1: usize,
) -> Result<Vec<f32>, BakeError> {
    let header = cell.header();
    let (b, across) = (header.block, header.across);
    let columns = c1 - c0;
    let mut out = vec![0f32; (r1 - r0) * columns];
    for by in r0 / b..=(r1 - 1) / b {
        for bx in c0 / b..=(c1 - 1) / b {
            let data = cell.block(by * across + bx).ok_or_else(|| {
                BakeError::Missing(format!("GLO-30 block {} was not read", by * across + bx))
            })?;
            let (ra, rb) = (r0.max(by * b), r1.min((by + 1) * b));
            let (ca, cb) = (c0.max(bx * b), c1.min((bx + 1) * b));
            for r in ra..rb {
                let src = (r - by * b) * b + (ca - bx * b);
                let dst = (r - r0) * columns + (ca - c0);
                out[dst..dst + (cb - ca)].copy_from_slice(&data[src..src + (cb - ca)]);
            }
        }
    }
    Ok(out)
}

/// The window's mosaic, `outside` (0, GLO-30's sea) where `cell` gives nothing.
pub struct Mosaic {
    pub data: Vec<f32>,
    pub rows: usize,
    pub columns: usize,
}

pub fn mosaic<'a>(
    win: &Window,
    cell: impl Fn(i32, i32) -> Option<&'a dyn Cell>,
    outside: f32,
) -> Result<Mosaic, BakeError> {
    let rows = (win.bottom_row - win.top_row + 1) as usize;
    let columns = (win.right_col - win.left_col + 1) as usize;
    let mut data = vec![outside; rows * columns];
    for s in win.s0..=win.s1 {
        for w in win.w0..=win.w1 {
            let Some(c) = cell(s, w) else {
                continue;
            };
            let width = c.header().width;
            let Some(sl) = slice(win, s, w, width) else {
                continue;
            };
            let span = (sl.c1 - sl.c0) as usize;
            let cells = read_cell(c, sl.read_r0, sl.read_r1, sl.read_c0, sl.read_c1)?;
            let dst_col = (sl.c0 - win.left_col) as usize;
            if width == win.across {
                for r in sl.r0..sl.r1 {
                    let src = (r - sl.r0) as usize * span;
                    let dst = (r - win.top_row) as usize * columns + dst_col;
                    data[dst..dst + span].copy_from_slice(&cells[src..src + span]);
                }
                continue;
            }
            // A coarser band: its columns stretched along the rows, in float32 as numpy does it.
            let ratio = width as f64 / win.across as f64;
            let (lo, stride) = (sl.read_c0, sl.read_c1 - sl.read_c0);
            let mut ia = vec![0usize; span];
            let mut ib = vec![0usize; span];
            let mut t = vec![0f32; span];
            for k in 0..span {
                let pos = (sl.c0 + k as i64 - sl.c_first) as f64 * ratio;
                let a = (pos.floor() as i64).clamp(0, width as i64 - 1) as usize;
                ia[k] = a - lo;
                ib[k] = (a + 1).min(width - 1) - lo;
                t[k] = (pos - a as f64) as f32;
            }
            for r in sl.r0..sl.r1 {
                let src = (r - sl.r0) as usize * stride;
                let dst = (r - win.top_row) as usize * columns + dst_col;
                for k in 0..span {
                    let tk = t[k];
                    data[dst + k] = cells[src + ia[k]] * (1.0 - tk) + cells[src + ib[k]] * tk;
                }
            }
        }
    }
    Ok(Mosaic {
        data,
        rows,
        columns,
    })
}

impl Mosaic {
    /// The bilinear height at (lat, lon), degrees.
    #[inline]
    pub fn bilinear(&self, win: &Window, lat: f64, lon: f64) -> f64 {
        let fy = (win.s1 as f64 + 1.0 - lat) / win.dy - win.top_row as f64;
        let fx = (lon - win.w0 as f64) / win.dx - win.left_col as f64;
        self.at(fx, fy)
    }

    /// The bilinear height at mosaic column `fx`, row `fy`.
    #[inline]
    pub fn at(&self, fx: f64, fy: f64) -> f64 {
        let (rows, columns) = (self.rows, self.columns);
        let iy = (fy.floor()).clamp(0.0, rows as f64 - 2.0) as usize;
        let ix = (fx.floor()).clamp(0.0, columns as f64 - 2.0) as usize;
        let v = (fy - iy as f64).clamp(0.0, 1.0);
        let u = (fx - ix as f64).clamp(0.0, 1.0);
        let k = iy * columns + ix;
        let d = &self.data;
        d[k] as f64 * (1.0 - u) * (1.0 - v)
            + d[k + 1] as f64 * u * (1.0 - v)
            + d[k + columns] as f64 * (1.0 - u) * v
            + d[k + columns + 1] as f64 * u * v
    }
}

/// A cell's decoded blocks by index.
pub type Blocks = HashMap<usize, std::sync::Arc<Vec<f32>>>;

/// GLO-30's cells as a band of samples reads them: each cell's header and only the blocks asked for, decoded
/// (`band`: the 35 m shoreline band reads the blocks along a coast, not whole cells). A cell absent from the bucket is
/// sea (0 m).
#[derive(Default)]
pub struct Sparse {
    /// Each cell read: its header (none: the bucket has no such cell, sea) and its blocks.
    pub cells: HashMap<(i32, i32), Option<(Header, Blocks)>>,
}

/// The cell, row, column and block of the GLO-30 pixel at (lat, lon) and the fractions past it: rows of 1″ from the
/// cell's north edge, columns of 1/`width`° from its west (pixel-is-point), the row past the last one the next cell
/// south's first.
pub fn pixel_of(
    lat: f64,
    lon: f64,
    width_of: impl Fn(i32) -> usize,
) -> ((i32, i32), [usize; 2], [f64; 2]) {
    let s = (lat - 1e-12).floor() as i32;
    let w = ((lon + 180.0).rem_euclid(360.0)).floor() as i32 - 180;
    let width = width_of(s);
    let fy = (s as f64 + 1.0 - lat) * 3600.0;
    let fx = (lon - w as f64).rem_euclid(360.0) * width as f64;
    let (r, c) = (fy.floor(), fx.floor());
    (
        (s, w),
        [(r as usize).min(3599), (c as usize).min(width - 1)],
        [fx - c, fy - r],
    )
}

impl Sparse {
    /// Pixel (row, column) of cell (s, w), the row 3600 being the next cell south's row 0 and the column past the last
    /// the next cell east's first; none where its block was not read.
    fn pixel(&self, s: i32, w: i32, row: usize, column: usize) -> Option<f32> {
        let (s, row) = if row >= 3600 {
            (s - 1, row - 3600)
        } else {
            (s, row)
        };
        let width = columns_at(s);
        let (w, column) = if column >= width {
            ((w + 1 + 180).rem_euclid(360) - 180, column - width)
        } else {
            (w, column)
        };
        match self.cells.get(&(s, w))? {
            None => Some(0.0),
            Some((header, blocks)) => {
                let b = header.block;
                let k = (row / b) * header.across + column / b;
                blocks.get(&k).map(|data| data[(row % b) * b + column % b])
            }
        }
    }

    /// The bilinear height at (lat, lon); none where a block it reads was not read.
    pub fn height(&self, lat: f64, lon: f64) -> Option<f64> {
        let ((s, w), [r, c], [u, v]) = pixel_of(lat, lon, columns_at);
        let p = |dr: usize, dc: usize| self.pixel(s, w, r + dr, c + dc).map(f64::from);
        Some(
            p(0, 0)? * (1.0 - u) * (1.0 - v)
                + p(0, 1)? * u * (1.0 - v)
                + p(1, 0)? * (1.0 - u) * v
                + p(1, 1)? * u * v,
        )
    }

    /// The cells and blocks (by index, each cell's `block`² tiling) a bilinear read at (lat, lon) needs.
    pub fn needs(
        lat: f64,
        lon: f64,
        block: usize,
        out: &mut std::collections::BTreeSet<((i32, i32), usize)>,
    ) {
        let ((s, w), [r, c], _) = pixel_of(lat, lon, columns_at);
        for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let (mut cs, mut row) = (s, r + dr);
            if row >= 3600 {
                cs -= 1;
                row -= 3600;
            }
            let width = columns_at(cs);
            let (mut cw, mut column) = (w, c + dc);
            if column >= width {
                cw = (w + 1 + 180).rem_euclid(360) - 180;
                column -= width;
            }
            let across = width.div_ceil(block);
            out.insert(((cs, cw), (row / block) * across + column / block));
        }
    }
}

impl Sparse {
    /// The cells' pixels over the box [lat_min, lat_max] × [lon_min, lon_max] as one mosaic (glo30.py's layout,
    /// [`window`]), for reading many points fast: a block not read is NaN, a cell not asked for NaN, a cell the bucket
    /// lacks sea (0 m).
    pub fn mosaic(
        &self,
        lat_min: f64,
        lat_max: f64,
        lon_min: f64,
        lon_max: f64,
    ) -> Result<(Window, Mosaic), BakeError> {
        let win = window(lat_min, lat_max, lon_min, lon_max);
        struct Held<'a> {
            header: &'a Header,
            blocks: &'a HashMap<usize, std::sync::Arc<Vec<f32>>>,
        }
        impl Cell for Held<'_> {
            fn header(&self) -> &Header {
                self.header
            }
            fn block(&self, k: usize) -> Option<&[f32]> {
                Some(match self.blocks.get(&k) {
                    Some(b) => b.as_slice(),
                    None => filled(self.header.block, f32::NAN),
                })
            }
        }
        struct Sea(Header);
        impl Cell for Sea {
            fn header(&self) -> &Header {
                &self.0
            }
            fn block(&self, _: usize) -> Option<&[f32]> {
                Some(filled(self.0.block, 0.0))
            }
        }
        let mut cells: HashMap<(i32, i32), Box<dyn Cell + '_>> = HashMap::new();
        for s in win.s0..=win.s1 {
            for w in win.w0..=win.w1 {
                let key = (s, (w + 180).rem_euclid(360) - 180);
                match self.cells.get(&key) {
                    Some(Some((header, blocks))) => {
                        cells.insert((s, w), Box::new(Held { header, blocks }));
                    }
                    Some(None) => {
                        let width = columns_at(s);
                        cells.insert(
                            (s, w),
                            Box::new(Sea(Header {
                                width,
                                height: 3600,
                                block: 1024,
                                across: width.div_ceil(1024),
                                offsets: vec![],
                                counts: vec![],
                                dx: 1.0 / width as f64,
                                dy: 1.0 / 3600.0,
                                west: w as f64,
                                north: s as f64 + 1.0,
                            })),
                        );
                    }
                    None => {}
                }
            }
        }
        let m = mosaic(
            &win,
            |s, w| cells.get(&(s, w)).map(|c| c.as_ref()),
            f32::NAN,
        )?;
        Ok((win, m))
    }
}

/// A block of `block`² values all `value` (NaN or 0), made once.
fn filled(block: usize, value: f32) -> &'static [f32] {
    static NAN: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    static ZERO: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    let slot = if value.is_nan() { &NAN } else { &ZERO };
    let v = slot.get_or_init(|| vec![value; 1024 * 1024]);
    &v[..(block * block).min(v.len())]
}

use std::collections::HashMap;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_bands_follow_the_bucket() {
        assert_eq!(name(64, -23), "Copernicus_DSM_COG_10_N64_00_W023_00_DEM");
        assert_eq!(name(-9, 160), "Copernicus_DSM_COG_10_S09_00_E160_00_DEM");
        assert_eq!(columns_at(64), 1800);
        assert_eq!(columns_at(49), 3600);
        assert_eq!(columns_at(-49), 3600);
        assert_eq!(columns_at(-50), 2400);
        assert_eq!(columns_at(85), 360);
    }
}
