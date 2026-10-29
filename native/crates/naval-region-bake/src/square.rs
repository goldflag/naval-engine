//! The world's squares (stage 1 of the world cache, 2026-10-04): 1° × 1° squares georeferenced in latitude and
//! longitude at about 70 m, each baked once from its DEM and kept, then resampled into any scenario's frame. A second
//! scenario over the same coast reads the squares the first baked instead of the DEM.
//!
//! - **The grid.** Square (lat, lon) is named by its south-west corner (`N36W123`). Its samples stand every
//!   1/[`ROWS_PER_DEGREE`]° of latitude (69.5 m) and every 1/[`columns_per_degree`]° of longitude, the columns set by
//!   latitude band so a sample is never more than about 70 m apart either way; row 0 is the north edge, column 0 the
//!   west edge, both edges inclusive, and [`MARGIN`] samples past every edge, so a resample never reads a neighbour.
//! - **What a square holds.** The surveyed heights: each sample the mean of 2 × 2 bilinear DEM points a quarter of its
//!   spacing either side (the region bake's supersampling, in the square's own grid), despiked with a halo of
//!   [`BAKE_HALO`] samples so a square's samples are exactly those of the despiked world and neighbours meet without a
//!   seam. Terrarium (NED's sea fill at sea level) where SRTM covers the square, Copernicus GLO-30 past 60° N and 56° S
//!   ([`source_for`]). Nothing about land or coast: those are decided after the resample, in the scenario's frame, so a
//!   coastline is resampled once (resampling a rounded coast would blur it twice).
//! - **Layers.** A square is a list of named layers ([`Square::layers`]); `height` is the only one stage 1 bakes. Lane
//!   B adds its own (a coastline mask from OpenStreetMap, a drawn sea floor for the renderer, lake masks with their
//!   levels in the header's `extra`) without a new format: a reader skips layers it does not know.
//! - **The final layer** ([`FINAL`], [`Layer::Final`]; step C): the heights the simulation reads, after coast
//!   finishing, on **one global lattice** (1/1,600° of latitude and of longitude at every latitude, not this grid's
//!   bands), the square's window of 1,601 × 1,601 i16 quanta of 0.25 m with both edges, so neighbours share their edge
//!   points whatever the band. It is kept as sixteen 0.25° blocks, each compressed and hashed apart, so a reader
//!   decodes only the blocks it reads. Its lattice, blocks and reader are `naval-land-format`'s (the root workspace's,
//!   so the simulation decodes it on every target); a release's square holds it alone ([`final_square`]). The surveyed
//!   `height` layer stays as it is, read by owner: a place's surveyed height is the one square holding it.
//! - **The file** ([`Square::encode`], NSQ1, [`FORMAT`] 1): `NSQ1`, a u32 header length, the header as JSON (the
//!   square, its grid, its source and attribution, each layer's encoding, byte range and SHA-256), then the layers'
//!   payloads. `height` is i32 quanta of 1/256 m (Terrarium's own precision), the NTF1 planar prediction, its bytes
//!   split into four planes, zlib. A file is the same wherever it is kept, so stage 2 can serve the same files from a
//!   static host (`squares-v1/N36W123.nsq`) beside the local cache.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

pub use naval_land_format::SquareKey;
pub use naval_land_format::block::Window;
pub use naval_land_format::nsq::{FINAL, FORMAT, LayerInfo, MAGIC};

use crate::geo::{catmull_rom, mercator_x, mercator_y};
use crate::glo30;
use crate::terrarium::{self, DemTile};
use crate::{BakeError, DEEP_M, Dem, SRTM_NORTH, SRTM_SOUTH, SURVEY_CELL_M};

/// A Terrarium tile key: x continuous about the origin (not wrapped), y.
pub type DemKey = (i64, i64);
/// A GLO-30 cell key: its south-west corner, whole degrees.
pub type CellKey = (i32, i32);

/// The bilinear value of a row-major grid at column `fx`, row `fy`, clamped to its edge cells.
#[inline]
pub fn bilinear(grid: &[f32], columns: usize, rows: usize, fx: f64, fy: f64) -> f64 {
    let ix = fx.floor().clamp(0.0, columns as f64 - 2.0) as usize;
    let iy = fy.floor().clamp(0.0, rows as f64 - 2.0) as usize;
    let u = (fx - ix as f64).clamp(0.0, 1.0);
    let v = (fy - iy as f64).clamp(0.0, 1.0);
    let k = iy * columns + ix;
    grid[k] as f64 * (1.0 - u) * (1.0 - v)
        + grid[k + 1] as f64 * u * (1.0 - v)
        + grid[k + columns] as f64 * (1.0 - u) * v
        + grid[k + columns + 1] as f64 * u * v
}

/// The folder squares of this format are kept in, locally or on a host.
pub const FOLDER: &str = "squares-v1";
/// Rows a degree of latitude: 2.25″, 69.5 m.
pub const ROWS_PER_DEGREE: usize = 1600;
/// Samples kept past each edge, so a bicubic resample near an edge reads only its own square.
pub const MARGIN: usize = 2;
/// Samples baked past the margin for the despike (two passes of a 9 × 9 window reach 8), then dropped.
pub const BAKE_HALO: usize = 8;
/// Heights are kept in steps of this, metres.
pub const HEIGHT_QUANTUM: f64 = 1.0 / 256.0;
/// The zlib level squares are written at: measured on N36W122, level 1 writes 4.1 MB where 6 writes 3.7 MB in ten
/// times the time and 9 3.6 MB in forty (a host serving them may pack them tighter).
pub const ZLIB_LEVEL: u32 = 1;

/// Longitude columns a degree in the band of a square whose south edge is `lat`: by the latitude of its edge nearer
/// the equator, so no sample of the square is more than about 70 m from the next.
pub fn columns_per_degree(lat: i32) -> usize {
    let a = if lat >= 0 { lat } else { -(lat + 1) };
    for (limit, columns) in [
        (30, 1600),
        (45, 1400),
        (55, 1150),
        (62, 920),
        (68, 760),
        (74, 600),
        (80, 448),
        (85, 280),
    ] {
        if a < limit {
            return columns;
        }
    }
    140
}

/// The elevation a square is baked from: Terrarium where SRTM covers it, Copernicus GLO-30 past 60° N and 56° S, where
/// Terrarium falls back to coarse sources.
pub fn source_for(key: SquareKey) -> Dem {
    if key.lat as f64 >= SRTM_NORTH || (key.lat + 1) as f64 <= SRTM_SOUTH {
        Dem::Glo30
    } else {
        Dem::Terrarium
    }
}

/// A square's grid, margin included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareGrid {
    pub key: SquareKey,
    /// Samples down and across, margin included.
    pub rows: usize,
    pub columns: usize,
    /// Degrees between samples.
    pub dlat: f64,
    pub dlon: f64,
    /// Samples past each edge.
    pub margin: usize,
}

impl SquareGrid {
    pub fn new(key: SquareKey, margin: usize) -> Self {
        let across = columns_per_degree(key.lat);
        Self {
            key,
            rows: ROWS_PER_DEGREE + 1 + 2 * margin,
            columns: across + 1 + 2 * margin,
            dlat: 1.0 / ROWS_PER_DEGREE as f64,
            dlon: 1.0 / across as f64,
            margin,
        }
    }

    /// Row `r`'s latitude.
    #[inline]
    pub fn lat(&self, r: usize) -> f64 {
        self.key.lat as f64 + 1.0 - (r as f64 - self.margin as f64) * self.dlat
    }

    /// Column `c`'s longitude.
    #[inline]
    pub fn lon(&self, c: usize) -> f64 {
        self.key.lon as f64 + (c as f64 - self.margin as f64) * self.dlon
    }

    /// The fractional (column, row) of (lat, lon), longitude taken within half a world of the square.
    #[inline]
    pub fn position(&self, lat: f64, lon: f64) -> [f64; 2] {
        let mut d = lon - self.key.lon as f64;
        d -= ((d + 180.0) / 360.0).floor() * 360.0;
        [
            d / self.dlon + self.margin as f64,
            (self.key.lat as f64 + 1.0 - lat) / self.dlat + self.margin as f64,
        ]
    }

    /// The latitudes and longitudes the grid spans: [lat_min, lat_max, lon_min, lon_max].
    pub fn bounds(&self) -> [f64; 4] {
        [
            self.lat(self.rows - 1),
            self.lat(0),
            self.lon(0),
            self.lon(self.columns - 1),
        ]
    }
}

/// One named layer of a square.
#[derive(Clone, Debug, PartialEq)]
pub enum Layer {
    /// Heights (or any signed value) in metres, f32 in memory, kept as i32 steps of `quantum`.
    Height { quantum: f64, values: Vec<f32> },
    /// One byte a sample (a mask, a coverage, an id), kept as is.
    Bytes(Vec<u8>),
    /// The final heights on the global lattice, in blocks (`naval_land_format::block`): not on this square's grid.
    Final(Window),
}

/// Where a square's heights came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquareSource {
    pub dem: Dem,
    /// Terrarium's zoom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoom: Option<u32>,
    /// Baked from Terrarium where GLO-30 was wanted and could not be read: kept for the session, never in the cache.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fallback: bool,
    pub attribution: String,
}

/// A square's header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SquareHeader {
    pub format: u32,
    pub lat: i32,
    pub lon: i32,
    pub rows: usize,
    pub columns: usize,
    pub margin: usize,
    pub rows_per_degree: usize,
    pub columns_per_degree: usize,
    pub source: SquareSource,
    /// Whether any sample stands above the land threshold.
    pub land: bool,
    pub layers: Vec<LayerInfo>,
    /// Anything a later layer needs in the header (lane B's lake levels).
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A square in memory.
#[derive(Clone, Debug, PartialEq)]
pub struct Square {
    pub grid: SquareGrid,
    pub source: SquareSource,
    pub land: bool,
    pub layers: BTreeMap<String, Layer>,
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The `height` layer's name.
pub const HEIGHT: &str = "height";

impl Square {
    /// The surveyed heights, row-major over the grid.
    pub fn heights(&self) -> &[f32] {
        match self.layers.get(HEIGHT) {
            Some(Layer::Height { values, .. }) => values,
            _ => &[],
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        self.encode_at(ZLIB_LEVEL)
    }

    /// The file at zlib `level` (0 to 9).
    pub fn encode_at(&self, level: u32) -> Vec<u8> {
        let level = flate2::Compression::new(level);
        let mut payloads: Vec<u8> = vec![];
        let mut infos = vec![];
        for (name, layer) in &self.layers {
            if let Layer::Final(window) = layer {
                let (mut info, bytes) = naval_land_format::nsq::final_layer(window);
                info.name = name.clone();
                info.offset = payloads.len();
                infos.push(info);
                payloads.extend_from_slice(&bytes);
                continue;
            }
            let (encoding, quantum, bytes) = match layer {
                Layer::Height { quantum, values } => {
                    let q: Vec<i32> = values
                        .iter()
                        .map(|&v| (v as f64 / quantum).round_ties_even() as i32)
                        .collect();
                    (
                        "i32-planar-zlib",
                        Some(*quantum),
                        crate::ntf::zlib_at(
                            &planar_i32(&q, self.grid.columns, self.grid.rows),
                            level,
                        ),
                    )
                }
                Layer::Bytes(values) => ("u8-zlib", None, crate::ntf::zlib_at(values, level)),
                Layer::Final(_) => unreachable!("written above"),
            };
            infos.push(LayerInfo {
                name: name.clone(),
                encoding: encoding.into(),
                quantum,
                offset: payloads.len(),
                length: bytes.len(),
                sha256: crate::ntf::sha256_hex(&bytes),
                blocks: None,
            });
            payloads.extend_from_slice(&bytes);
        }
        let header = SquareHeader {
            format: FORMAT,
            lat: self.grid.key.lat,
            lon: self.grid.key.lon,
            rows: self.grid.rows,
            columns: self.grid.columns,
            margin: self.grid.margin,
            rows_per_degree: ROWS_PER_DEGREE,
            columns_per_degree: columns_per_degree(self.grid.key.lat),
            source: self.source.clone(),
            land: self.land,
            layers: infos,
            extra: self.extra.clone(),
        };
        let json = serde_json::to_vec(&header).expect("a square's header serialises");
        let mut out = Vec::with_capacity(8 + json.len() + payloads.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&payloads);
        out
    }

    /// A square's header alone, from its file's first bytes.
    pub fn header(bytes: &[u8]) -> Result<SquareHeader, BakeError> {
        let bad = |why: String| BakeError::Corrupt(format!("not a world square: {why}"));
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(bad("no NSQ1 magic".into()));
        }
        let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let header: SquareHeader = serde_json::from_slice(
            bytes
                .get(8..8 + length)
                .ok_or_else(|| bad("short header".into()))?,
        )
        .map_err(|e| bad(e.to_string()))?;
        if header.format != FORMAT {
            return Err(bad(format!("format {}, not {FORMAT}", header.format)));
        }
        Ok(header)
    }

    /// A square from its file, its layers checked against their digests; layers of unknown encodings are skipped.
    pub fn decode(bytes: &[u8]) -> Result<Self, BakeError> {
        let bad = |why: String| BakeError::Corrupt(format!("not a world square: {why}"));
        let header = Self::header(bytes)?;
        let key = SquareKey {
            lat: header.lat,
            lon: header.lon,
        };
        let grid = SquareGrid::new(key, header.margin);
        if (grid.rows, grid.columns) != (header.rows, header.columns)
            || header.rows_per_degree != ROWS_PER_DEGREE
        {
            return Err(bad(format!(
                "its grid {} × {} is not {}'s",
                header.rows,
                header.columns,
                key.name()
            )));
        }
        let start = 8 + u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let samples = grid.rows * grid.columns;
        let mut layers = BTreeMap::new();
        for info in &header.layers {
            let payload = bytes
                .get(start + info.offset..start + info.offset + info.length)
                .ok_or_else(|| bad(format!("layer {} runs past the file", info.name)))?;
            if crate::ntf::sha256_hex(payload) != info.sha256 {
                return Err(bad(format!(
                    "layer {} does not match its SHA-256",
                    info.name
                )));
            }
            let layer = match (info.encoding.as_str(), info.quantum) {
                ("i32-planar-zlib", Some(quantum)) => {
                    let raw = crate::ntf::inflate(payload, samples * 4)?;
                    if raw.len() != samples * 4 {
                        return Err(bad(format!(
                            "layer {} is not one value a sample",
                            info.name
                        )));
                    }
                    let q = unplanar_i32(&raw, grid.columns, grid.rows);
                    Layer::Height {
                        quantum,
                        values: q.iter().map(|&v| (v as f64 * quantum) as f32).collect(),
                    }
                }
                (naval_land_format::block::ENCODING, _) => {
                    let blocks = info
                        .blocks
                        .as_deref()
                        .ok_or_else(|| bad(format!("layer {} has no block table", info.name)))?;
                    Layer::Final(naval_land_format::block::decode_window(payload, blocks)?)
                }
                ("u8-zlib", _) => {
                    let raw = crate::ntf::inflate(payload, samples)?;
                    if raw.len() != samples {
                        return Err(bad(format!("layer {} is not one byte a sample", info.name)));
                    }
                    Layer::Bytes(raw)
                }
                _ => continue,
            };
            layers.insert(info.name.clone(), layer);
        }
        Ok(Self {
            grid,
            source: header.source,
            land: header.land,
            layers,
            extra: header.extra,
        })
    }

    /// The square's `layer` at fractional (column, row), Catmull-Rom bicubic (a `Bytes` layer bilinear); None where the
    /// square has no such layer.
    #[inline]
    pub fn sample(&self, values: &[f32], fx: f64, fy: f64) -> f64 {
        bicubic(values, self.grid.columns, self.grid.rows, fx, fy)
    }
}

/// A release's square (step C): its final window alone, as the release's manifest hashes it and the store keeps it.
/// `extra` is what the bake records beside it.
pub fn final_square(
    key: SquareKey,
    window: &Window,
    extra: serde_json::Map<String, serde_json::Value>,
) -> Vec<u8> {
    naval_land_format::FinalSquare::encode(key, window, extra)
}

/// The NTF1 planar prediction over i32, its little-endian bytes split into four planes (each value's lowest bytes
/// first): small residuals leave three planes nearly constant, which zlib packs to almost nothing.
fn planar_i32(q: &[i32], columns: usize, rows: usize) -> Vec<u8> {
    let n = q.len();
    let mut out = vec![0u8; 4 * n];
    for j in 0..rows {
        for i in 0..columns {
            let at = j * columns + i;
            let left = if i > 0 { q[at - 1] } else { 0 };
            let up = if j > 0 { q[at - columns] } else { 0 };
            let up_left = if i > 0 && j > 0 {
                q[at - columns - 1]
            } else {
                0
            };
            let r = q[at]
                .wrapping_sub(left.wrapping_add(up).wrapping_sub(up_left))
                .to_le_bytes();
            for (p, byte) in r.iter().enumerate() {
                out[p * n + at] = *byte;
            }
        }
    }
    out
}

fn unplanar_i32(raw: &[u8], columns: usize, rows: usize) -> Vec<i32> {
    let n = columns * rows;
    let mut q = vec![0i32; n];
    for j in 0..rows {
        for i in 0..columns {
            let at = j * columns + i;
            let r = i32::from_le_bytes([raw[at], raw[n + at], raw[2 * n + at], raw[3 * n + at]]);
            let left = if i > 0 { q[at - 1] } else { 0 };
            let up = if j > 0 { q[at - columns] } else { 0 };
            let up_left = if i > 0 && j > 0 {
                q[at - columns - 1]
            } else {
                0
            };
            q[at] = r.wrapping_add(left.wrapping_add(up).wrapping_sub(up_left));
        }
    }
    q
}

/// The Catmull-Rom bicubic value of a row-major grid at column `fx`, row `fy`, its edge samples repeated beyond it,
/// held between the least and greatest of the four samples about the point: the cubic keeps the slopes the bilinear
/// blurs, and the clamp stops it ringing past a step (beside a bathymetry step it rang a metre or two above the sea,
/// which came out as land).
#[inline]
pub fn bicubic(grid: &[f32], columns: usize, rows: usize, fx: f64, fy: f64) -> f64 {
    let (x0, y0) = (fx.floor(), fy.floor());
    let (wx, wy) = (catmull_rom(fx - x0), catmull_rom(fy - y0));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let mut sum = 0.0;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for (dy, wy) in wy.iter().enumerate() {
        let r = (y0 + dy as i64 - 1).clamp(0, rows as i64 - 1) as usize * columns;
        let mut across = 0.0;
        for (dx, wx) in wx.iter().enumerate() {
            let c = (x0 + dx as i64 - 1).clamp(0, columns as i64 - 1) as usize;
            let v = grid[r + c] as f64;
            across += v * wx;
            if (dx == 1 || dx == 2) && (dy == 1 || dy == 2) {
                low = low.min(v);
                high = high.max(v);
            }
        }
        sum += across * wy;
    }
    sum.clamp(low, high)
}

// ---------------------------------------------------------------- what a square reads

/// The grid a square is baked on: its own, with the despike's halo past the margin.
pub fn bake_grid(key: SquareKey) -> SquareGrid {
    SquareGrid::new(key, MARGIN + BAKE_HALO)
}

/// The Terrarium zoom a square reads: the region bake's, by the square's middle latitude.
pub fn terrarium_zoom(key: SquareKey) -> u32 {
    terrarium::zoom_for(key.lat as f64 + 0.5, SURVEY_CELL_M)
}

/// The supersampled points' extent past the bake grid: a quarter of a sample either way.
fn supersampled_bounds(grid: &SquareGrid) -> [f64; 4] {
    let [lat_min, lat_max, lon_min, lon_max] = grid.bounds();
    [
        lat_min - grid.dlat / 4.0,
        lat_max + grid.dlat / 4.0,
        lon_min - grid.dlon / 4.0,
        lon_max + grid.dlon / 4.0,
    ]
}

/// The Terrarium tiles a square reads at its zoom, x continuous about the square: [x0, x1] by [y0, y1].
pub fn terrarium_tiles(key: SquareKey) -> (u32, Vec<DemKey>) {
    let zoom = terrarium_zoom(key);
    let [x0, x1, y0, y1] = terrarium_span(&bake_grid(key), zoom);
    let mut out = vec![];
    for y in y0..=y1 {
        for x in x0..=x1 {
            out.push((x, y));
        }
    }
    (zoom, out)
}

fn terrarium_span(grid: &SquareGrid, zoom: u32) -> [i64; 4] {
    let scale = (1u64 << zoom) as f64 * 256.0;
    let [lat_min, lat_max, lon_min, lon_max] = supersampled_bounds(grid);
    let (px0, px1) = (
        mercator_x(lon_min, scale) - 0.5,
        mercator_x(lon_max, scale) - 0.5,
    );
    let (py0, py1) = (
        mercator_y(lat_max, scale) - 0.5,
        mercator_y(lat_min, scale) - 0.5,
    );
    [
        (px0.floor() / 256.0).floor() as i64,
        ((px1.floor() + 1.0) / 256.0).floor() as i64,
        (py0.floor() / 256.0).floor() as i64,
        ((py1.floor() + 1.0) / 256.0).floor() as i64,
    ]
}

/// The GLO-30 window a square reads (its cell and the edges of its neighbours').
pub fn glo30_window(key: SquareKey) -> glo30::Window {
    let [lat_min, lat_max, lon_min, lon_max] = supersampled_bounds(&bake_grid(key));
    glo30::window(lat_min, lat_max, lon_min, lon_max)
}

/// The GLO-30 cells a square's window reaches, south-west corners.
pub fn glo30_cells(key: SquareKey) -> Vec<CellKey> {
    let win = glo30_window(key);
    let mut out = vec![];
    for s in win.s0..=win.s1 {
        for w in win.w0..=win.w1 {
            if glo30::slice(&win, s, w, win.across).is_some() {
                out.push((s, w));
            }
        }
    }
    out
}

// ---------------------------------------------------------------- baking a square

/// The supersampled mean of `value` at every sample of the bake grid: 2 × 2 points a quarter of a sample either way,
/// `px(lon)` and `py(lat)` separable (both DEMs are), then the despike and the halo dropped.
pub(crate) fn bake_heights(
    key: SquareKey,
    px: impl Fn(f64) -> f64,
    py: impl Fn(f64) -> f64,
    value: impl Fn(f64, f64) -> f64,
) -> Vec<f32> {
    bake_heights_halo(key, BAKE_HALO, px, py, value)
}

/// [`bake_heights`] with `halo` samples baked past the margin: any halo at least the despike's reach gives the same
/// samples (the despike is local), which its test holds.
fn bake_heights_halo(
    key: SquareKey,
    halo: usize,
    px: impl Fn(f64) -> f64,
    py: impl Fn(f64) -> f64,
    value: impl Fn(f64, f64) -> f64,
) -> Vec<f32> {
    let grid = SquareGrid::new(key, MARGIN + halo);
    let (rows, columns) = (grid.rows, grid.columns);
    let quarter = [-0.25, 0.25];
    let xs: Vec<[f64; 2]> = (0..columns)
        .map(|c| quarter.map(|o| px(grid.lon(c) + o * grid.dlon)))
        .collect();
    let mut heights = vec![0f32; rows * columns];
    for r in 0..rows {
        let ys = quarter.map(|o| py(grid.lat(r) - o * grid.dlat));
        for c in 0..columns {
            let mut sum = 0.0;
            for y in ys {
                for x in xs[c] {
                    sum += value(x, y);
                }
            }
            heights[r * columns + c] = (sum / 4.0) as f32;
        }
    }
    despiked_core(key, halo, &heights)
}

/// A bake grid's heights (the square's grid with `halo` samples past the margin) despiked, the halo dropped.
fn despiked_core(key: SquareKey, halo: usize, heights: &[f32]) -> Vec<f32> {
    let grid = SquareGrid::new(key, MARGIN + halo);
    let (despiked, _) = crate::despike::despike(heights, grid.columns, grid.rows, 2);
    let kept = SquareGrid::new(key, MARGIN);
    let mut out = Vec::with_capacity(kept.rows * kept.columns);
    for r in halo..halo + kept.rows {
        out.extend_from_slice(
            &despiked[r * grid.columns + halo..r * grid.columns + halo + kept.columns],
        );
    }
    out
}

/// A square sampled straight from `height` (latitude, longitude) at each sample of its bake grid, despiked with its
/// halo as a DEM's square is: a generated surveyed layer's (`finish::synthetic`), which has no DEM to supersample.
pub fn bake_sampled(
    key: SquareKey,
    height: &dyn Fn(f64, f64) -> f64,
    source: SquareSource,
) -> Square {
    let grid = bake_grid(key);
    let mut heights = vec![0f32; grid.rows * grid.columns];
    for r in 0..grid.rows {
        let lat = grid.lat(r);
        for c in 0..grid.columns {
            heights[r * grid.columns + c] = height(lat, grid.lon(c)) as f32;
        }
    }
    square_of(key, source, despiked_core(key, BAKE_HALO, &heights))
}

pub(crate) fn square_of(key: SquareKey, source: SquareSource, heights: Vec<f32>) -> Square {
    let land = heights.iter().any(|&h| h > crate::LAND_THRESHOLD_M);
    let mut layers = BTreeMap::new();
    layers.insert(
        HEIGHT.to_string(),
        Layer::Height {
            quantum: HEIGHT_QUANTUM,
            // As the file keeps them, so a square baked and one read back resample alike.
            values: heights
                .into_iter()
                .map(|h| ((h as f64 / HEIGHT_QUANTUM).round_ties_even() * HEIGHT_QUANTUM) as f32)
                .collect(),
        },
    );
    Square {
        grid: SquareGrid::new(key, MARGIN),
        source,
        land,
        layers,
        extra: serde_json::Map::new(),
    }
}

/// A square from Terrarium: `tile` gives every tile [`terrarium_tiles`] lists (decoded, NED's fill at sea level).
pub fn bake_terrarium(
    key: SquareKey,
    tile: &dyn Fn(DemKey) -> Option<Arc<DemTile>>,
    fallback: bool,
) -> Result<Square, BakeError> {
    let zoom = terrarium_zoom(key);
    let [x0, x1, y0, y1] = terrarium_span(&bake_grid(key), zoom);
    let mw = ((x1 - x0 + 1) * 256) as usize;
    let mh = ((y1 - y0 + 1) * 256) as usize;
    let mut mosaic = vec![DEEP_M as f32; mw * mh];
    for ty in y0..=y1 {
        for tx in x0..=x1 {
            let t = tile((tx, ty)).ok_or_else(|| {
                BakeError::Missing(format!("Terrarium tile {zoom}/{tx}/{ty} was not fetched"))
            })?;
            let (ox, oy) = (((tx - x0) * 256) as usize, ((ty - y0) * 256) as usize);
            for r in 0..256 {
                mosaic[(oy + r) * mw + ox..(oy + r) * mw + ox + 256]
                    .copy_from_slice(&t[r * 256..(r + 1) * 256]);
            }
        }
    }
    let scale = (1u64 << zoom) as f64 * 256.0;
    let (bx, by) = ((x0 * 256) as f64, (y0 * 256) as f64);
    let heights = bake_heights(
        key,
        |lon| mercator_x(lon, scale) - 0.5 - bx,
        |lat| mercator_y(lat, scale) - 0.5 - by,
        |fx, fy| bilinear(&mosaic, mw, mh, fx, fy),
    );
    Ok(square_of(
        key,
        SquareSource {
            dem: Dem::Terrarium,
            zoom: Some(zoom),
            fallback,
            attribution: TERRARIUM_ATTRIBUTION.into(),
        },
        heights,
    ))
}

/// Terrarium's sources, as the tiles' own attribution names them (tilezen/joerd).
pub const TERRARIUM_ATTRIBUTION: &str = "AWS Terrain Tiles (Terrarium): SRTM and GMTED2010 courtesy of the U.S. Geological Survey, ETOPO1 (NOAA), EU-DEM (Copernicus), ArcticDEM (NSF), USGS NED";

/// A square from GLO-30: `cell` gives each cell [`glo30_cells`] lists that the bucket has, with every block its slice
/// of [`glo30_window`] reads; a cell it does not give is sea level.
pub fn bake_glo30(
    key: SquareKey,
    cell: &dyn Fn(CellKey) -> Option<Arc<dyn glo30::Cell + Send + Sync>>,
) -> Result<Square, BakeError> {
    let win = glo30_window(key);
    let held: BTreeMap<CellKey, Arc<dyn glo30::Cell + Send + Sync>> = glo30_cells(key)
        .into_iter()
        .filter_map(|k| cell(k).map(|c| (k, c)))
        .collect();
    let mosaic = glo30::mosaic(
        &win,
        |s, w| held.get(&(s, w)).map(|c| c.as_ref() as &dyn glo30::Cell),
        0.0,
    )?;
    let heights = bake_heights(
        key,
        |lon| (lon - win.w0 as f64) / win.dx - win.left_col as f64,
        |lat| (win.s1 as f64 + 1.0 - lat) / win.dy - win.top_row as f64,
        |fx, fy| mosaic.at(fx, fy),
    );
    Ok(square_of(
        key,
        SquareSource {
            dem: Dem::Glo30,
            zoom: None,
            fallback: false,
            attribution: format!("Copernicus DEM GLO-30, {}", glo30::ATTRIBUTION),
        },
        heights,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_name_and_wrap_the_world() {
        assert_eq!(
            SquareKey::at(36.9, -122.4),
            SquareKey { lat: 36, lon: -123 }
        );
        assert_eq!(SquareKey::at(-9.2, 160.1).name(), "S10E160");
        assert_eq!(SquareKey::at(10.0, 181.5), SquareKey { lat: 10, lon: -179 });
        assert_eq!(
            SquareKey::at(10.0, -180.0),
            SquareKey { lat: 10, lon: -180 }
        );
        assert_eq!(SquareKey { lat: 36, lon: -123 }.name(), "N36W123");
        assert_eq!(source_for(SquareKey { lat: 60, lon: 0 }), Dem::Glo30);
        assert_eq!(source_for(SquareKey { lat: 59, lon: 0 }), Dem::Terrarium);
        assert_eq!(source_for(SquareKey { lat: -56, lon: 0 }), Dem::Terrarium);
        assert_eq!(source_for(SquareKey { lat: -57, lon: 0 }), Dem::Glo30);
    }

    #[test]
    fn no_sample_is_more_than_about_seventy_metres_from_the_next() {
        for lat in -85..85 {
            let across = columns_per_degree(lat) as f64;
            let equatorward = if lat >= 0 { lat } else { lat + 1 } as f64;
            let m = 111_195.0 * equatorward.to_radians().cos() / across;
            assert!(m <= 70.0, "{lat}: {m} m");
        }
    }

    #[test]
    fn a_square_round_trips_its_file_and_resamples_its_own_samples() {
        let key = SquareKey { lat: 36, lon: -123 };
        let grid = SquareGrid::new(key, MARGIN);
        let values: Vec<f32> = (0..grid.rows * grid.columns)
            .map(|k| {
                (((k % grid.columns) as f32 * 0.01).sin() * 300.0 - 20.0
                    + (k / grid.columns) as f32 * 0.05)
                    .round()
                    / 4.0
            })
            .collect();
        let square = square_of(
            key,
            SquareSource {
                dem: Dem::Terrarium,
                zoom: Some(11),
                fallback: false,
                attribution: "test".into(),
            },
            values.clone(),
        );
        let bytes = square.encode();
        let back = Square::decode(&bytes).unwrap();
        assert_eq!(back, square);
        assert_eq!(back.heights(), values.as_slice());
        // At a sample's own position the bicubic gives the sample.
        let (r, c) = (700, 333);
        let [fx, fy] = grid.position(grid.lat(r), grid.lon(c));
        assert!((fx - c as f64).abs() < 1e-6 && (fy - r as f64).abs() < 1e-6);
        let v = back.sample(back.heights(), c as f64, r as f64);
        assert!((v - values[r * grid.columns + c] as f64).abs() < 1e-4);
        // A corrupt layer is refused.
        let mut broken = bytes.clone();
        let last = broken.len() - 1;
        broken[last] ^= 1;
        assert!(Square::decode(&broken).is_err());
        // A surveyed square's header is as it was before the final layer: no block table.
        let header = Square::header(&bytes).unwrap();
        assert!(header.layers.iter().all(|l| l.blocks.is_none()));
    }

    #[test]
    fn a_squares_supersampled_and_despiked_samples_are_local() {
        // A DEM of ridges with spikes in a grid: the square baked with its own halo and with twice it gives the same
        // samples, so the halo holds the supersampling's and the despike's reach (two 9 × 9 passes, 8 samples).
        let key = SquareKey { lat: 47, lon: -3 };
        let dem = |lon: f64, lat: f64| {
            let base = ((lon * 37.0).sin() * 40.0 + (lat * 23.0).cos() * 30.0).max(-5.0);
            let (u, v) = ((lon * 40.0).fract(), (lat * 40.0).fract());
            if u.abs() < 0.02 && v.abs() < 0.02 {
                base + 500.0
            } else {
                base
            }
        };
        let own = bake_heights(key, |lon| lon, |lat| lat, dem);
        let wider = bake_heights_halo(key, 2 * BAKE_HALO, |lon| lon, |lat| lat, dem);
        assert_eq!(own, wider);
        // The despike acted: no sample stands a spike's height over the ridges.
        assert!(own.iter().all(|&h| h < 200.0));
    }

    #[test]
    fn a_square_carries_the_final_layer_and_a_reader_takes_one_block() {
        let key = SquareKey { lat: 59, lon: 179 };
        let grid = SquareGrid::new(key, MARGIN);
        let heights = vec![-3.0f32; grid.rows * grid.columns];
        // A height that is a function of the lattice point (the edge at 180° is the lattice's −180°).
        let window = Window::of(key, |p| {
            let q = ((p.i() % 211) - (p.j() % 97) * 3) as i16;
            if q == 0 { -1 } else { q }
        });
        let mut square = square_of(
            key,
            SquareSource {
                dem: Dem::Glo30,
                zoom: None,
                fallback: false,
                attribution: "test".into(),
            },
            heights,
        );
        square
            .layers
            .insert(FINAL.into(), Layer::Final(window.clone()));
        let bytes = square.encode();
        assert_eq!(Square::decode(&bytes).unwrap(), square);
        // The simulation's reader finds the final layer in the surveyed square and decodes one block alone.
        let read = naval_land_format::FinalSquare::read(bytes.clone()).unwrap();
        assert_eq!(read.key(), key);
        assert_eq!(read.block(2, 3).unwrap(), window.block(2, 3));
        // A release's square holds it alone, and the same window gives the same bytes.
        let release = final_square(key, &window, Default::default());
        assert_eq!(release, final_square(key, &window, Default::default()));
        let read = naval_land_format::FinalSquare::read(release).unwrap();
        assert_eq!(read.window().unwrap(), window);
        // A corrupt byte is refused when the bake reads the square back.
        let at = bytes.len() - 10;
        let mut broken = bytes;
        broken[at] ^= 0x40;
        assert!(Square::decode(&broken).is_err());
    }
}
