//! The effects' CPU-made textures, byte for byte the web client's (deterministic, no downloads), for both effect
//! crates: `effectTexture('flash' | 'glow' | 'droplet' | 'smoke')` (`EffectParticles.ts`; the droplet and smoke maps are
//! the water effects' spray's), `smokeBillowTexture` (the funnels' billows and the spray's) and `smokeNoiseTexture`
//! (`SmokeSpriteMaterial.ts`) and `effectVolumeTexture` (`EffectVolume.ts`). Mip chains are 2 × 2 box averages, as
//! three's generated mipmaps are for an exact halving. Texels are raw RGBA8 (three's `NoColorSpace`): the shaders read
//! them as data, never as sRGB.

use crate::math::smooth;

/// An RGBA8 image with its mip chain, level 0 first, rows as the web stores them (row 0 at v = 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Image2d {
    pub size: u32,
    pub levels: Vec<Vec<u8>>,
}

/// A 3D RGBA8 volume without mips (the web samples it linearly, repeating).
#[derive(Clone, Debug)]
pub struct Volume {
    pub size: u32,
    pub texels: Vec<u8>,
}

fn clamp01(n: f64) -> f64 {
    n.clamp(0.0, 1.0)
}

/// `hash` of `EffectParticles.ts`: the sine hash.
fn sine_hash(x: f64, y: f64) -> f64 {
    let n = (x * 127.1 + y * 311.7).sin() * 43_758.545_3;
    n - n.floor()
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// `noise` of `EffectParticles.ts`: bilinear value noise on the sine hash.
fn value_noise(x: f64, y: f64) -> f64 {
    let (ix, iy) = (x.floor(), y.floor());
    let (fx, fy) = (smooth(x - ix), smooth(y - iy));
    lerp(
        lerp(sine_hash(ix, iy), sine_hash(ix + 1.0, iy), fx),
        lerp(sine_hash(ix, iy + 1.0), sine_hash(ix + 1.0, iy + 1.0), fx),
        fy,
    )
}

impl Image2d {
    /// Texel (x, y) of level 0.
    pub fn texel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.size + x) * 4) as usize;
        let p = &self.levels[0];
        [p[i], p[i + 1], p[i + 2], p[i + 3]]
    }
}

/// The sprites' density textures: the combat effects' additive ones and the water effects' spray.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpriteMap {
    /// A muzzle or burst flash: a bright core with a turbulent rim.
    Flash,
    /// A steady, smooth halo (embers, fire glows, shell halos).
    Glow,
    /// A shell's tracer streak: +Y (the last rows) is the shell's tip; the hot centre narrows and fades toward
    /// the tail.
    Tracer,
    /// The spray's round drops, lit a little from above.
    Droplet,
    /// The spray's soft puff with a lit upper edge (a splash's mist).
    Smoke,
}

/// `effectTexture(kind)`: 128², density in alpha; in RGB white light (the colour comes from the sprite), or for the
/// spray's maps (`Droplet`, `Smoke`) the lighting, R = G = B.
pub fn effect_texture(kind: SpriteMap) -> Image2d {
    let size = 128usize;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let radius = u.hypot(v);
            let coarse = value_noise(u * 3.8 + 11.0, v * 3.8 + 31.0);
            let detail = value_noise(u * 12.0 + 71.0, v * 12.0 + 53.0) * 0.65
                + value_noise(u * 29.0 + 9.0, v * 29.0 + 17.0) * 0.35;
            let density =
                smooth((1.0 - radius + (coarse - 0.5) * 0.5) * 2.8) * (0.5 + detail * 0.5);
            let (alpha, light) = match kind {
                SpriteMap::Glow => (
                    (-radius * radius * 7.0).exp() * smooth((1.0 - radius) * 4.0),
                    1.0,
                ),
                SpriteMap::Flash => (
                    (-radius * radius * 5.0).exp()
                        * smooth((1.0 - radius) * 5.0)
                        * (0.55 + density * 0.45),
                    1.0,
                ),
                SpriteMap::Tracer => {
                    let along = (v + 1.0) / 2.0;
                    let width = 0.12 + along * 0.7;
                    (
                        (-(u / width).powi(2) * 4.0).exp()
                            * smooth(along * 1.3)
                            * smooth((1.0 - along) * 18.0),
                        1.0,
                    )
                }
                SpriteMap::Droplet => (
                    smooth((1.0 - radius) * 3.5) * (0.6 + detail * 0.4),
                    clamp01(0.68 + v * 0.16 + coarse * 0.2),
                ),
                SpriteMap::Smoke => (
                    density * smooth((1.0 - radius) * 5.0),
                    clamp01(0.48 + coarse * 0.34 + detail * 0.22 + v * 0.12),
                ),
            };
            let i = (y * size + x) * 4;
            let l = (light * 255.0).round() as u8;
            pixels[i] = l;
            pixels[i + 1] = l;
            pixels[i + 2] = l;
            pixels[i + 3] = (alpha * 255.0).round() as u8;
        }
    }
    with_mips(size as u32, pixels)
}

/// `hash` of `SmokeSpriteMaterial.ts`: an integer lattice hash (no sine, so no banding).
fn lattice_hash(x: i32, y: i32, seed: i32) -> f64 {
    let mut h = x
        .wrapping_mul(374_761_393)
        .wrapping_add(y.wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(1_442_695_041));
    h = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1_274_126_177);
    h ^= ((h as u32) >> 16) as i32;
    f64::from(h as u32) / 4_294_967_296.0
}

/// Periodic value noise: `period` cells across the unit square, so textures tile.
fn periodic_noise(x: f64, y: f64, period: i32, seed: i32) -> f64 {
    let (px, py) = (x * f64::from(period), y * f64::from(period));
    let (ix, iy) = (px.floor() as i32, py.floor() as i32);
    let (fx, fy) = (smooth(px - f64::from(ix)), smooth(py - f64::from(iy)));
    let at = |cx: i32, cy: i32| lattice_hash(cx.rem_euclid(period), cy.rem_euclid(period), seed);
    lerp(
        lerp(at(ix, iy), at(ix + 1, iy), fx),
        lerp(at(ix, iy + 1), at(ix + 1, iy + 1), fx),
        fy,
    )
}

fn fbm(x: f64, y: f64, seed: i32, base: i32) -> f64 {
    periodic_noise(x, y, base, seed) * 0.52
        + periodic_noise(x, y, base * 2, seed + 1) * 0.29
        + periodic_noise(x, y, base * 4, seed + 2) * 0.14
        + periodic_noise(x, y, base * 8, seed + 3) * 0.05
}

/// `smokeBillowTexture(128)`: one cauliflower billow. RGB holds its surface normal in quad space (xy) and its
/// thickness; alpha holds density.
pub fn smoke_billow_texture() -> Image2d {
    let size = 128usize;
    let lobes: [[f64; 3]; 9] = [
        [0.0, -0.04, 0.46],
        [-0.3, 0.1, 0.3],
        [0.29, 0.12, 0.32],
        [-0.08, 0.36, 0.27],
        [0.2, 0.34, 0.22],
        [-0.32, -0.26, 0.24],
        [0.33, -0.22, 0.26],
        [0.04, 0.12, 0.38],
        [-0.02, -0.38, 0.2],
    ];
    let mut heights = vec![0.0f64; size * size];
    let mut alphas = vec![0.0f64; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let mut sum = 0.0;
            for [cx, cy, r] in lobes {
                sum += (-((u - cx).powi(2) + (v - cy).powi(2)) / (r * r) * 2.2).exp() * r;
            }
            let fx = (x as f64 + 0.5) / size as f64;
            let fy = (y as f64 + 0.5) / size as f64;
            let coarse = fbm(fx, fy, 5, 3) - 0.5;
            let fine = fbm(fx, fy, 17, 8) - 0.5;
            let edge = 1.0 - u.hypot(v).min(1.0);
            let h = (sum + coarse * 0.16 + fine * 0.06).max(0.0) * smooth(edge * 3.5);
            heights[y * size + x] = h;
            alphas[y * size + x] = smooth((h - 0.04) / 0.26) * (0.82 + fine * 0.5);
        }
    }
    let at = |x: isize, y: isize| {
        let x = x.clamp(0, size as isize - 1) as usize;
        let y = y.clamp(0, size as isize - 1) as usize;
        heights[y * size + x]
    };
    // Gentle relief: billows read as soft, rounded masses rather than hard spheres.
    let scale = size as f64 / 2.0 * 0.55;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (xi, yi) = (x as isize, y as isize);
            let h = at(xi, yi);
            let dx = (at(xi + 1, yi) - at(xi - 1, yi)) * scale / 2.0;
            let dy = (at(xi, yi + 1) - at(xi, yi - 1)) * scale / 2.0;
            let inv = 1.0 / (dx * dx + dy * dy + 1.0).sqrt();
            let i = (y * size + x) * 4;
            pixels[i] = ((-dx * inv * 0.5 + 0.5) * 255.0).round() as u8;
            pixels[i + 1] = ((-dy * inv * 0.5 + 0.5) * 255.0).round() as u8;
            pixels[i + 2] = (clamp01(h / 0.55) * 255.0).round() as u8;
            pixels[i + 3] = (clamp01(alphas[y * size + x]) * 255.0).round() as u8;
        }
    }
    with_mips(size as u32, pixels)
}

/// `smokeNoiseTexture(128)`: tileable turbulence for ribbons; R and G are two independent fbm fields.
pub fn smoke_noise_texture() -> Image2d {
    let size = 128usize;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f64 / size as f64, y as f64 / size as f64);
            let i = (y * size + x) * 4;
            let r = (clamp01(fbm(fx, fy, 11, 4)) * 255.0).round() as u8;
            pixels[i] = r;
            pixels[i + 1] = (clamp01(fbm(fx, fy, 29, 2)) * 255.0).round() as u8;
            pixels[i + 2] = r;
            pixels[i + 3] = 255;
        }
    }
    with_mips(size as u32, pixels)
}

/// `effectVolumeTexture()`: periodic Worley volumes (32³, RGB at 4, 8 and 12 cells) that animate the flame
/// tongues and erode the fire-smoke puffs.
pub fn effect_volume_texture() -> Volume {
    let size = 32usize;
    let hash = |x: f64, y: f64, z: f64, seed: f64| {
        let n = (x * 127.1 + y * 311.7 + z * 74.7 + seed * 19.19).sin() * 43_758.545_3;
        n - n.floor()
    };
    let worley = |x: f64, y: f64, z: f64, cells: f64| {
        let (x, y, z) = (x * cells, y * cells, z * cells);
        let (ix, iy, iz) = (x.floor(), y.floor(), z.floor());
        let mut distance: f64 = 3.0;
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (cx, cy, cz) = (ix + f64::from(dx), iy + f64::from(dy), iz + f64::from(dz));
                    let wx = (cx + cells) % cells;
                    let wy = (cy + cells) % cells;
                    let wz = (cz + cells) % cells;
                    let px = cx + hash(wx, wy, wz, 1.0) - x;
                    let py = cy + hash(wx, wy, wz, 2.0) - y;
                    let pz = cz + hash(wx, wy, wz, 3.0) - z;
                    distance = distance.min(px * px + py * py + pz * pz);
                }
            }
        }
        (1.0 - distance.sqrt()).max(0.0)
    };
    let mut texels = vec![0u8; size * size * size * 4];
    for z in 0..size {
        for y in 0..size {
            for x in 0..size {
                let i = ((z * size + y) * size + x) * 4;
                let (fx, fy, fz) = (
                    (x as f64 + 0.5) / size as f64,
                    (y as f64 + 0.5) / size as f64,
                    (z as f64 + 0.5) / size as f64,
                );
                for channel in 0..3 {
                    let cells = 4.0 * (channel as f64 + 1.0);
                    texels[i + channel] = (worley(fx, fy, fz, cells) * 255.0).round() as u8;
                }
                texels[i + 3] = 255;
            }
        }
    }
    Volume {
        size: size as u32,
        texels,
    }
}

/// A 2 × 2 box-filtered mip chain down to 1 × 1.
fn with_mips(size: u32, level0: Vec<u8>) -> Image2d {
    let mut levels = vec![level0];
    let mut edge = size as usize;
    while edge > 1 {
        let half = edge / 2;
        let source = levels.last().expect("level 0 is there");
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                for c in 0..4 {
                    let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .iter()
                        .map(|(dx, dy)| {
                            u32::from(source[((y * 2 + dy) * edge + x * 2 + dx) * 4 + c])
                        })
                        .sum();
                    next[(y * half + x) * 4 + c] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
        edge = half;
    }
    Image2d { size, levels }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flash_is_brightest_at_its_centre() {
        let flash = effect_texture(SpriteMap::Flash);
        let alpha = |x: usize, y: usize| flash.levels[0][(y * 128 + x) * 4 + 3];
        assert!(alpha(64, 64) > 100);
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(flash.levels.len(), 8);
        assert_eq!(flash.levels[7].len(), 4);
    }

    #[test]
    fn the_lattice_hash_is_a_stable_unit_value() {
        let h = lattice_hash(0, 0, 5);
        assert!((0.0..1.0).contains(&h));
        assert_eq!(lattice_hash(3, 4, 5), lattice_hash(3, 4, 5));
        assert_ne!(lattice_hash(3, 4, 5), lattice_hash(4, 3, 5));
    }

    /// The spray's maps (`naval-water-effects`), texels computed with the TS formulas in Bun (`effectTexture`,
    /// `smokeBillowTexture`); ±1 for the platform `sin` in the droplet's hash.
    #[test]
    fn the_spray_maps_match_the_web() {
        let close = |a: [u8; 4], b: [u8; 4]| {
            a.iter()
                .zip(b)
                .all(|(x, y)| (i16::from(*x) - i16::from(y)).abs() <= 1)
        };
        let points = [(64, 64), (10, 20), (100, 37), (5, 120), (77, 90)];
        let expected: [(&str, Image2d, [[u8; 4]; 5]); 3] = [
            (
                "droplet",
                effect_texture(SpriteMap::Droplet),
                [
                    [218, 218, 218, 175],
                    [176, 176, 176, 0],
                    [173, 173, 173, 177],
                    [229, 229, 229, 0],
                    [206, 206, 206, 188],
                ],
            ),
            (
                "smoke",
                effect_texture(SpriteMap::Smoke),
                [
                    [210, 210, 210, 155],
                    [194, 194, 194, 0],
                    [150, 150, 150, 94],
                    [204, 204, 204, 0],
                    [181, 181, 181, 172],
                ],
            ),
            (
                "billow",
                smoke_billow_texture(),
                [
                    [103, 70, 255, 252],
                    [128, 128, 0, 0],
                    [141, 118, 16, 0],
                    [128, 128, 0, 0],
                    [162, 230, 141, 206],
                ],
            ),
        ];
        for (name, texture, texels) in &expected {
            assert_eq!(texture.levels.len(), 8, "{name}");
            for (&(x, y), &texel) in points.iter().zip(texels) {
                assert!(
                    close(texture.texel(x, y), texel),
                    "{name} ({x}, {y}): {:?} against {texel:?}",
                    texture.texel(x, y)
                );
            }
        }
    }

    #[test]
    fn the_volume_tiles() {
        let volume = effect_volume_texture();
        assert_eq!(volume.texels.len(), 32 * 32 * 32 * 4);
        assert!(volume.texels.chunks(4).any(|t| t[0] > 200));
    }
}
