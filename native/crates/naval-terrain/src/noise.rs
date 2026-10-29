//! The tileable noise tile the terrain shader samples at several scales (the web game's `terrain/TerrainNoise.ts`), built
//! once on the CPU, byte for byte the web's: R fractal value noise, G distance to the nearest Worley point, B F2 − F1,
//! A a random value per Worley cell.

/// Texels along a side of the tile (`NOISE_SIZE`).
pub const NOISE_SIZE: usize = 512;
/// Worley cells along a side of the tile (`WORLEY_CELLS`).
pub const WORLEY_CELLS: i64 = 16;

/// The web's integer hash in 0–1 (`Math.imul` on 32-bit lanes).
fn hash(x: i64, y: i64, seed: i64) -> f64 {
    let (x, y, seed) = (x as u32, y as u32, seed as u32);
    let mut h =
        x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1) ^ seed.wrapping_mul(0x9e37_79b9);
    h = (h ^ (h >> 15)).wrapping_mul(0x85eb_ca6b);
    h = (h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35);
    f64::from(h ^ (h >> 16)) / 4_294_967_296.0
}

fn value_noise(x: f64, y: f64, period: i64, seed: i64) -> f64 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (x - ix, y - iy);
    let u = fx * fx * (3.0 - 2.0 * fx);
    let v = fy * fy * (3.0 - 2.0 * fy);
    let (ix, iy) = (ix as i64, iy as i64);
    let x0 = ix.rem_euclid(period);
    let y0 = iy.rem_euclid(period);
    let x1 = (x0 + 1) % period;
    let y1 = (y0 + 1) % period;
    let a = hash(x0, y0, seed);
    let b = hash(x1, y0, seed);
    let c = hash(x0, y1, seed);
    let d = hash(x1, y1, seed);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

fn round_byte(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// The tile's RGBA bytes, `NOISE_SIZE`² texels, row-major (`terrainNoise`).
pub fn terrain_noise() -> Vec<u8> {
    let size = NOISE_SIZE;
    let mut out = vec![0u8; size * size * 4];
    let cells = WORLEY_CELLS as usize;
    let mut points = vec![0f32; cells * cells * 2];
    for j in 0..cells {
        for i in 0..cells {
            points[(j * cells + i) * 2] = (0.1 + 0.8 * hash(i as i64, j as i64, 11)) as f32;
            points[(j * cells + i) * 2 + 1] = (0.1 + 0.8 * hash(i as i64, j as i64, 12)) as f32;
        }
    }
    let cell_texels = (size / cells) as f64;
    for y in 0..size {
        for x in 0..size {
            let (mut fbm, mut amplitude, mut total) = (0.0, 0.5, 0.0);
            for octave in 0..5 {
                let period = 8i64 << octave;
                fbm += value_noise(
                    x as f64 / size as f64 * period as f64,
                    y as f64 / size as f64 * period as f64,
                    period,
                    octave + 1,
                ) * amplitude;
                total += amplitude;
                amplitude *= 0.5;
            }
            fbm /= total;
            let (cx, cy) = (x as f64 / cell_texels, y as f64 / cell_texels);
            let (ix, iy) = (cx.floor() as i64, cy.floor() as i64);
            let (mut f1, mut f2, mut owner) = (f64::INFINITY, f64::INFINITY, 0i64);
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (nx, ny) = (ix + dx, iy + dy);
                    let wx = nx.rem_euclid(WORLEY_CELLS);
                    let wy = ny.rem_euclid(WORLEY_CELLS);
                    let k = (wy * WORLEY_CELLS + wx) as usize;
                    let px = nx as f64 + f64::from(points[k * 2]);
                    let py = ny as f64 + f64::from(points[k * 2 + 1]);
                    let distance = (px - cx).hypot(py - cy);
                    if distance < f1 {
                        f2 = f1;
                        f1 = distance;
                        owner = wy * WORLEY_CELLS + wx;
                    } else if distance < f2 {
                        f2 = distance;
                    }
                }
            }
            let at = (y * size + x) * 4;
            // Stretch the value noise's narrow middle-heavy spread to use the byte range.
            out[at] = round_byte((0.5 + (fbm - 0.5) * 1.8) * 255.0);
            out[at + 1] = round_byte((f1 / 0.5).min(1.0) * 255.0);
            out[at + 2] = round_byte(((f2 - f1) / 0.5).min(1.0) * 255.0);
            out[at + 3] = round_byte(hash(owner, 7, 13) * 255.0);
        }
    }
    out
}
