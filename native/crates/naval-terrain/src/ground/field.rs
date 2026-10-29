//! CPU preparation of a baked heightfield for drawing (the web game's `terrain/TerrainField.ts`): the arrays the terrain
//! textures upload, the height bounds the level of detail culls with, and the sun's visibility over the land.
//!
//! Arrays are row-major over the field's samples (x along a row, z down the rows). JavaScript computes in f64 and
//! stores into `Float32Array`s; the port does the same, so every array matches the web's byte for byte (the tests pin
//! them to Bun's output).

use crate::heightfield::Heightfield;

/// A regular grid of samples: the field's own, or a coarser one taken from it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub columns: u32,
    pub rows: u32,
    /// Metres between samples (f64, as the web's `cell × factor`).
    pub cell: f64,
}

impl Grid {
    pub fn of(field: &Heightfield) -> Self {
        Self {
            columns: field.columns,
            rows: field.rows,
            cell: f64::from(field.cell),
        }
    }

    fn len(&self) -> usize {
        self.columns as usize * self.rows as usize
    }
}

/// Reach of the signed coast distance stored in the attributes, metres (`COAST_RANGE_M`).
pub const COAST_RANGE_M: f64 = 8000.0;
/// Metres of relative height per unit of the stored topographic positions (`RELIEF_RANGE_M`).
pub const RELIEF_BROAD_M: f64 = 120.0;
pub const RELIEF_FINE_M: f64 = 24.0;
/// Half-widths in metres of the windows the topographic positions average over (`ATTRIBUTE_WINDOWS_M`).
pub const WINDOW_FINE_M: f64 = 240.0;
pub const WINDOW_BROAD_M: f64 = 960.0;
/// Drainage area stored as log2(samples) over this many octaves (`FLOW_OCTAVES`).
pub const FLOW_OCTAVES: f64 = 16.0;

/// The byte value (0–1) that stores a signed coast distance in metres (`coastByte`).
pub fn coast_byte(metres: f64) -> f64 {
    let sign = if metres > 0.0 {
        1.0
    } else if metres < 0.0 {
        -1.0
    } else {
        0.0
    };
    0.5 + 0.5 * sign * (metres.abs().min(COAST_RANGE_M) / COAST_RANGE_M).sqrt()
}

/// Heights in metres, one float per sample: `quantum × step` (`sampleHeights`).
pub fn sample_heights(field: &Heightfield) -> Vec<f32> {
    let step = f64::from(field.step);
    field
        .quanta
        .iter()
        .map(|&q| (f64::from(q) * step) as f32)
        .collect()
}

/// IEEE half float bits of `value` (rounded to f32 first), rounded to nearest as the web's `halfBits` does.
pub fn half_bits(value: f64) -> u16 {
    let bits = (value as f32).to_bits();
    let sign = (bits >> 16) & 0x8000;
    let exponent = ((bits >> 23) & 0xff) as i32 - 112;
    let mut mantissa = bits & 0x7f_ffff;
    if exponent <= 0 {
        if exponent < -10 {
            return sign as u16;
        }
        mantissa = (mantissa | 0x80_0000) >> (1 - exponent) as u32;
        return (sign | ((mantissa + 0x1000) >> 13)) as u16;
    }
    if exponent >= 31 {
        return (sign | 0x7c00) as u16;
    }
    (sign | (((exponent as u32) << 10) + ((mantissa + 0x1000) >> 13))) as u16
}

/// The surface gradient (∂h/∂x, ∂h/∂z) at every sample by central differences, as half-float pairs
/// (`gradientHalves`). One-sided at the border.
pub fn gradient_halves(field: &Heightfield, heights: &[f32]) -> Vec<u16> {
    let (w, h) = (field.columns as usize, field.rows as usize);
    let cell = f64::from(field.cell);
    let mut out = vec![0u16; w * h * 2];
    for j in 0..h {
        let up_j = if j > 0 { j - 1 } else { j };
        let down_j = if j < h - 1 { j + 1 } else { j };
        let (up, down) = (up_j * w, down_j * w);
        let span_z = (down_j - up_j) as f64 * cell;
        for i in 0..w {
            let left = if i > 0 { i - 1 } else { i };
            let right = if i < w - 1 { i + 1 } else { i };
            let row = j * w;
            out[(row + i) * 2] = half_bits(
                (f64::from(heights[row + right]) - f64::from(heights[row + left]))
                    / ((right - left) as f64 * cell),
            );
            out[(row + i) * 2 + 1] =
                half_bits((f64::from(heights[down + i]) - f64::from(heights[up + i])) / span_z);
        }
    }
    out
}

/// Mean over the part of a (2r + 1)² window inside the grid, by separable prefix sums (`boxMean`).
pub fn box_mean(values: &[f32], columns: usize, rows: usize, radius: usize) -> Vec<f32> {
    let mut across = vec![0f32; values.len()];
    let mut out = vec![0f32; values.len()];
    let mut prefix = vec![0f64; columns.max(rows) + 1];
    for j in 0..rows {
        let row = j * columns;
        for i in 0..columns {
            prefix[i + 1] = prefix[i] + f64::from(values[row + i]);
        }
        for i in 0..columns {
            let low = i.saturating_sub(radius);
            let high = (i + radius + 1).min(columns);
            across[row + i] = ((prefix[high] - prefix[low]) / (high - low) as f64) as f32;
        }
    }
    for i in 0..columns {
        for j in 0..rows {
            prefix[j + 1] = prefix[j] + f64::from(across[j * columns + i]);
        }
        for j in 0..rows {
            let low = j.saturating_sub(radius);
            let high = (j + radius + 1).min(rows);
            out[j * columns + i] = ((prefix[high] - prefix[low]) / (high - low) as f64) as f32;
        }
    }
    out
}

/// Every `factor`-th sample of the field's heights along both axes (`coarseGrid`).
pub fn coarse_grid(field: &Heightfield, heights: &[f32], factor: u32) -> (Grid, Vec<f32>) {
    let columns = (field.columns - 1) / factor + 1;
    let rows = (field.rows - 1) / factor + 1;
    let mut out = vec![0f32; columns as usize * rows as usize];
    for j in 0..rows as usize {
        for i in 0..columns as usize {
            out[j * columns as usize + i] =
                heights[j * factor as usize * field.columns as usize + i * factor as usize];
        }
    }
    (
        Grid {
            columns,
            rows,
            cell: f64::from(field.cell) * f64::from(factor),
        },
        out,
    )
}

/// Signed distance in metres to the coastline, positive on land, from a two-pass chamfer transform capped at `cap`
/// (`coastDistance`).
pub fn coast_distance(grid: Grid, heights: &[f32], cap: f64) -> Vec<f32> {
    let (w, h, cell) = (grid.columns as usize, grid.rows as usize, grid.cell);
    let n = w * h;
    let diagonal = cell * std::f64::consts::SQRT_2;
    let mut to_sea = vec![0f32; n];
    let mut to_land = vec![0f32; n];
    for i in 0..n {
        let land = heights[i] > 0.0;
        to_sea[i] = if land { cap as f32 } else { 0.0 };
        to_land[i] = if land { 0.0 } else { cap as f32 };
    }
    chamfer(&mut to_sea, w, h, cell, diagonal);
    chamfer(&mut to_land, w, h, cell, diagonal);
    (0..n)
        .map(|i| {
            if heights[i] > 0.0 {
                (f64::from(to_sea[i]) - cell / 2.0) as f32
            } else {
                (-(f64::from(to_land[i]) - cell / 2.0)) as f32
            }
        })
        .collect()
}

fn chamfer(d: &mut [f32], w: usize, h: usize, cell: f64, diagonal: f64) {
    let at_f = |d: &[f32], k: usize| f64::from(d[k]);
    for j in 0..h {
        let row = j * w;
        for i in 0..w {
            let at = row + i;
            let mut best = at_f(d, at);
            let mut take = |next: f64| {
                if next < best {
                    best = next;
                }
            };
            if i > 0 {
                take(at_f(d, at - 1) + cell);
            }
            if j > 0 {
                take(at_f(d, at - w) + cell);
                if i > 0 {
                    take(at_f(d, at - w - 1) + diagonal);
                }
                if i < w - 1 {
                    take(at_f(d, at - w + 1) + diagonal);
                }
            }
            d[at] = best as f32;
        }
    }
    for j in (0..h).rev() {
        let row = j * w;
        for i in (0..w).rev() {
            let at = row + i;
            let mut best = at_f(d, at);
            let mut take = |next: f64| {
                if next < best {
                    best = next;
                }
            };
            if i < w - 1 {
                take(at_f(d, at + 1) + cell);
            }
            if j < h - 1 {
                take(at_f(d, at + w) + cell);
                if i < w - 1 {
                    take(at_f(d, at + w + 1) + diagonal);
                }
                if i > 0 {
                    take(at_f(d, at + w - 1) + diagonal);
                }
            }
            d[at] = best as f32;
        }
    }
}

/// Area draining through each sample, in samples, by D8 flow routing, highest samples first (`flowAccumulation`).
pub fn flow_accumulation(grid: Grid, heights: &[f32]) -> Vec<f32> {
    let (w, h) = (grid.columns as usize, grid.rows as usize);
    let n = w * h;
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for &value in heights {
        let value = f64::from(value);
        low = low.min(value);
        high = high.max(value);
    }
    let buckets = (((high - low) * 4.0).ceil() as usize + 1).max(1);
    let mut counts = vec![0u32; buckets + 1];
    let bucket = |value: f32| (((f64::from(value) - low) * 4.0).floor() as usize).min(buckets - 1);
    for &value in heights {
        counts[buckets - bucket(value)] += 1;
    }
    for b in 1..=buckets {
        counts[b] += counts[b - 1];
    }
    let mut order = vec![0u32; n];
    for (i, &value) in heights.iter().enumerate() {
        let slot = &mut counts[buckets - 1 - bucket(value)];
        order[*slot as usize] = i as u32;
        *slot += 1;
    }
    let mut area = vec![1f32; n];
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    for &at in &order {
        let at = at as usize;
        let here = f64::from(heights[at]);
        if here <= 0.0 {
            continue;
        }
        let (i, j) = ((at % w) as i64, (at / w) as i64);
        let mut best: Option<usize> = None;
        let mut drop = 0.0;
        for dj in -1i64..=1 {
            for di in -1i64..=1 {
                let (x, y) = (i + di, j + dj);
                if (di == 0 && dj == 0) || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                let to = y as usize * w + x as usize;
                let fall = (here - f64::from(heights[to]))
                    * if di != 0 && dj != 0 { diagonal } else { 1.0 };
                if fall > drop {
                    drop = fall;
                    best = Some(to);
                }
            }
        }
        if let Some(best) = best {
            area[best] += area[at];
        }
    }
    area
}

fn byte(value: f64) -> u8 {
    if value <= 0.0 {
        0
    } else if value >= 1.0 {
        255
    } else {
        (value * 255.0).round() as u8
    }
}

/// Per-sample surface attributes, four bytes each (`surfaceAttributes`): R drainage (log2 samples over
/// [`FLOW_OCTAVES`]), G broad topographic position, B signed coast distance ([`coast_byte`]), A fine topographic
/// position.
pub fn surface_attributes(grid: Grid, heights: &[f32]) -> Vec<u8> {
    let (w, h, cell) = (grid.columns as usize, grid.rows as usize, grid.cell);
    let cells = |metres: f64| ((metres / cell).round() as usize).max(1);
    let fine = box_mean(heights, w, h, cells(WINDOW_FINE_M));
    let broad = box_mean(heights, w, h, cells(WINDOW_BROAD_M));
    let coast = coast_distance(grid, heights, COAST_RANGE_M);
    let flow = flow_accumulation(grid, heights);
    let mut out = vec![0u8; grid.len() * 4];
    for i in 0..grid.len() {
        let height = f64::from(heights[i]);
        out[i * 4] = byte(f64::from(flow[i]).log2() / FLOW_OCTAVES);
        out[i * 4 + 1] = byte(0.5 + (height - f64::from(broad[i])) / (2.0 * RELIEF_BROAD_M));
        out[i * 4 + 2] = byte(coast_byte(f64::from(coast[i])));
        out[i * 4 + 3] = byte(0.5 + (height - f64::from(fine[i])) / (2.0 * RELIEF_FINE_M));
    }
    out
}

/// One level of the bound pyramid: lowest and highest sample over square blocks.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundLevel {
    pub size: usize,
    pub min: Vec<f32>,
    pub max: Vec<f32>,
}

/// Lowest and highest sample over square blocks of `2^level × base` cells, per level up to one block over the whole
/// field (`HeightBounds`).
#[derive(Clone, Debug, PartialEq)]
pub struct HeightBounds {
    pub base: usize,
    pub levels: Vec<BoundLevel>,
}

/// `heightBounds`: a block's bounds include its border samples.
pub fn height_bounds(field: &Heightfield, heights: &[f32], base: usize) -> HeightBounds {
    let (w, h) = (field.columns as usize, field.rows as usize);
    let cells = w.max(h) - 1;
    let mut size = cells.div_ceil(base);
    let mut min = vec![f32::INFINITY; size * size];
    let mut max = vec![f32::NEG_INFINITY; size * size];
    for bj in 0..size {
        for bi in 0..size {
            let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
            for j in bj * base..=((bj + 1) * base).min(h - 1) {
                for i in bi * base..=((bi + 1) * base).min(w - 1) {
                    let value = heights[j * w + i];
                    low = low.min(value);
                    high = high.max(value);
                }
            }
            min[bj * size + bi] = low;
            max[bj * size + bi] = high;
        }
    }
    let mut levels = vec![BoundLevel { size, min, max }];
    while size > 1 {
        let next = size.div_ceil(2);
        let previous = levels.last().unwrap();
        let mut next_min = vec![f32::INFINITY; next * next];
        let mut next_max = vec![f32::NEG_INFINITY; next * next];
        for j in 0..size {
            for i in 0..size {
                let at = (j >> 1) * next + (i >> 1);
                next_min[at] = next_min[at].min(previous.min[j * size + i]);
                next_max[at] = next_max[at].max(previous.max[j * size + i]);
            }
        }
        levels.push(BoundLevel {
            size: next,
            min: next_min,
            max: next_max,
        });
        size = next;
    }
    HeightBounds { base, levels }
}

/// Share of the sun's disc each sample sees past the land toward `direction` (unit, toward the light), 0–255
/// (`sunVisibility`): a line sweep from the sun's side for the sun `penumbra` degrees higher and lower.
pub fn sun_visibility(
    field: Grid,
    heights: &[f32],
    direction: [f64; 3],
    penumbra: f64,
    out: &mut [u8],
) {
    sun_visibility_by(field, |at| heights[at], direction, penumbra, out);
}

/// [`sun_visibility`] with sample `at`'s height (m) from `heights`.
pub fn sun_visibility_by(
    field: Grid,
    heights: impl Fn(usize) -> f32,
    direction: [f64; 3],
    penumbra: f64,
    out: &mut [u8],
) {
    let (w, h, cell) = (field.columns as usize, field.rows as usize, field.cell);
    let [dx, dy, dz] = direction;
    let across = dx.hypot(dz);
    let elevation = dy.atan2(across) * 180.0 / std::f64::consts::PI;
    if elevation + penumbra <= 0.0 {
        out.fill(0);
        return;
    }
    if across < 1e-6 || elevation - penumbra >= 89.0 {
        out.fill(255);
        return;
    }
    let along_x = dx.abs() >= dz.abs();
    let (major, minor) = if along_x { (dx, dz) } else { (dz, dx) };
    let step_major: i64 = if major > 0.0 { 1 } else { -1 };
    let slope = minor / major.abs();
    let run = cell * 1f64.hypot(slope);
    let rad = std::f64::consts::PI / 180.0;
    let drop_high = run * ((elevation - penumbra).max(0.01) * rad).tan();
    let drop_low = run * ((elevation + penumbra).min(89.9) * rad).tan();
    let (lines, length) = if along_x { (h, w) } else { (w, h) };
    let (line_stride, step_stride) = if along_x { (w, 1) } else { (1, w) };
    let mut surface_high = vec![0f32; 2 * lines];
    let mut surface_low = vec![0f32; 2 * lines];
    let offset = slope.floor();
    let t = slope - offset;
    let u = 1.0 - t;
    let offset = offset as i64;
    let first = if step_major > 0 { length - 1 } else { 0 };
    for line in 0..lines {
        let at = first * step_stride + line * line_stride;
        surface_high[line] = heights(at);
        surface_low[line] = heights(at);
        out[at] = 255;
    }
    let floor = -1e9;
    for step in 1..length {
        let previous = ((step - 1) & 1) * lines;
        let current = (step & 1) * lines;
        let k = (first as i64 - step as i64 * step_major) as usize;
        for line in 0..lines {
            let at = k * step_stride + line * line_stride;
            let ground = f64::from(heights(at));
            let lo = line as i64 + offset;
            let (mut high, mut low) = (floor, floor);
            // Beyond the field lies open sea, which casts no shadow.
            if lo >= 0 && lo + 1 < lines as i64 {
                let lo = lo as usize;
                high = f64::from(surface_high[previous + lo]) * u
                    + f64::from(surface_high[previous + lo + 1]) * t
                    - drop_high;
                low = f64::from(surface_low[previous + lo]) * u
                    + f64::from(surface_low[previous + lo + 1]) * t
                    - drop_low;
            } else if lo == lines as i64 - 1 && t == 0.0 {
                let lo = lo as usize;
                high = f64::from(surface_high[previous + lo]) - drop_high;
                low = f64::from(surface_low[previous + lo]) - drop_low;
            }
            surface_high[current + line] = if ground > high { ground } else { high } as f32;
            surface_low[current + line] = if ground > low { ground } else { low } as f32;
            // Lit fully above the surface the lowest part of the disc casts, dark below the one its top casts.
            out[at] = if high <= ground {
                255
            } else if low >= ground {
                0
            } else {
                ((ground - low) / (high - low) * 255.0 + 0.5) as i32 as u8
            };
        }
    }
}

/// Everything the terrain draws from, prepared once per battle from the decoded field (the CPU half of
/// `HeightfieldLandscape`'s constructor).
#[derive(Clone, Debug)]
pub struct PreparedField {
    pub grid: Grid,
    pub origin: [f32; 2],
    /// The field's samples: a height is `quanta · step` metres ([`PreparedField::sample_height`]), what `heights`
    /// holds. All a layer keeps of its heights once they are uploaded.
    pub quanta: Vec<i16>,
    pub step: f32,
    /// Heights (m), [`sample_heights`]. This and the arrays below are what the layer uploads; a built layer's field
    /// has them empty ([`PreparedField::release_uploaded`]).
    pub heights: Vec<f32>,
    /// Half-float (∂h/∂x, ∂h/∂z) pairs on the field's grid.
    pub gradients: Vec<u16>,
    /// The attribute grid (every [`ATTRIBUTE_STRIDE`]-th sample) and its RGBA bytes.
    pub coarse: Grid,
    pub coarse_heights: Vec<f32>,
    pub attributes: Vec<u8>,
    pub bounds: HeightBounds,
}

/// The attributes' grid takes every second sample: they vary over hundreds of metres (`ATTRIBUTE_STRIDE`).
pub const ATTRIBUTE_STRIDE: u32 = 2;
/// Field cells per block at the bound pyramid's base (`heightBounds(field, heights, 8)`).
pub const BOUNDS_BASE: usize = 8;

impl PreparedField {
    pub fn new(field: &Heightfield) -> Self {
        let heights = sample_heights(field);
        let gradients = gradient_halves(field, &heights);
        let (coarse, coarse_heights) = coarse_grid(field, &heights, ATTRIBUTE_STRIDE);
        let attributes = surface_attributes(coarse, &coarse_heights);
        let bounds = height_bounds(field, &heights, BOUNDS_BASE);
        Self {
            grid: Grid::of(field),
            origin: [field.origin_x, field.origin_z],
            quanta: field.quanta.clone(),
            step: field.step,
            heights,
            gradients,
            coarse,
            coarse_heights,
            attributes,
            bounds,
        }
    }

    /// The sun's visibility over the land for `direction` (unit, toward the light), with the web's 1.2° penumbra.
    pub fn sunlight(&self, direction: [f64; 3], out: &mut Vec<u8>) {
        out.resize(self.quanta.len(), 0);
        sun_visibility_by(self.grid, |at| self.sample_height(at), direction, 1.2, out);
    }

    /// Sample `at`'s height (m), as [`sample_heights`] gives it.
    #[inline]
    pub fn sample_height(&self, at: usize) -> f32 {
        (f64::from(self.quanta[at]) * f64::from(self.step)) as f32
    }

    /// The highest sample (m), 0 at the least.
    pub fn top(&self) -> f32 {
        let top = self.quanta.iter().copied().max().unwrap_or(0).max(0);
        (f64::from(top) * f64::from(self.step)) as f32
    }

    /// Bilinear height at chart metres; [`crate::heightfield::OPEN_SEA_M`] beyond the grid: `Heightfield::height` on
    /// the same samples.
    pub fn height(&self, x: f64, z: f64) -> f64 {
        let cell = self.grid.cell;
        let gx = (x - f64::from(self.origin[0])) / cell;
        let gz = (z - f64::from(self.origin[1])) / cell;
        let (w, h) = (f64::from(self.grid.columns), f64::from(self.grid.rows));
        if !(gx >= 0.0 && gz >= 0.0) || gx > w - 1.0 || gz > h - 1.0 {
            return crate::heightfield::OPEN_SEA_M;
        }
        let i = gx.floor().min(w - 2.0);
        let j = gz.floor().min(h - 2.0);
        let (u, v) = (gx - i, gz - j);
        let at = j as usize * self.grid.columns as usize + i as usize;
        let q = |k: usize| f64::from(self.quanta[k]);
        let c = self.grid.columns as usize;
        let top = q(at) * (1.0 - u) + q(at + 1) * u;
        let bottom = q(at + c) * (1.0 - u) + q(at + c + 1) * u;
        (top * (1.0 - v) + bottom * v) * f64::from(self.step)
    }

    /// Lets go of what only the layer's uploads read (the heights in metres, the gradients, the attribute grid's
    /// heights and bytes: about 55 MB on a 2401² field), keeping the samples, the grids and the bounds.
    pub fn release_uploaded(&mut self) {
        self.heights = Vec::new();
        self.gradients = Vec::new();
        self.coarse_heights = Vec::new();
        self.attributes = Vec::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_bits_round_like_the_web() {
        assert_eq!(half_bits(0.0), 0);
        assert_eq!(half_bits(1.0), 0x3c00);
        assert_eq!(half_bits(-2.0), 0xc000);
        assert_eq!(half_bits(65504.0), 0x7bff);
        assert_eq!(half_bits(1e6), 0x7c00);
        assert_eq!(half_bits(1e-5), half::f16::from_f32(1e-5).to_bits());
        for v in [0.1, 0.333, -0.72, 12.5, 1.0e-3] {
            assert_eq!(half_bits(v), half::f16::from_f64(v).to_bits());
        }
    }

    #[test]
    fn coast_bytes_are_signed_square_roots() {
        assert_eq!(coast_byte(0.0), 0.5);
        assert_eq!(coast_byte(8000.0), 1.0);
        assert_eq!(coast_byte(-20000.0), 0.0);
        assert!((coast_byte(2000.0) - 0.75).abs() < 1e-12);
    }
}
