//! How the cloud anchors' weather is blended where more than one anchor holds a place (step C's C13): so a band keeps
//! one texture's coverage. A linear cross-fade of the anchors' textures averages them toward their mean, a thinner and
//! wider cloud over every band than anywhere else (and more of it to march). Instead each anchor's value goes through
//! the texture's Gaussianizing table, the Gaussian values are blended keeping their mean and variance,
//! `μ + Σ wᵢ (gᵢ − μ) / √Σ wᵢ²` (`μ` the mean Gaussian value over the texture's texels: not quite 0, since a level many
//! texels share, the bare sky's 0, takes one value), and the sum goes back through the inverse table: Heitz and Neyret's
//! histogram-preserving blending ("High-Performance By-Example Noise using a Histogram-Preserving Blending Operator",
//! 2018). It stays a pure function of the anchors' values and weights, continuous in both: one anchor reads its own
//! value exactly, and the tables' round trip is taken off in the weights' proportion, so a weight going to nothing
//! leaves the other anchors' blend exactly.
//!
//! The tables are built once from the texture ([`tables`]); the clouds on the GPU read the same numbers
//! (`naval_clouds::laying`'s uniforms, `shaders/field.wgsl`).

use std::sync::OnceLock;

/// The forward table's entries: one a level of the texture's eight bits, a value between two read linearly.
pub const LEVELS: usize = 256;
/// The inverse table's entries, evenly over the Gaussian values [`-G_MAX`, `G_MAX`].
pub const INVERSE: usize = 1024;
/// The Gaussian values the inverse table spans: past it a value is the texture's least or greatest.
pub const G_MAX: f64 = 5.0;

/// The texture's Gaussianizing table and its inverse, a column a channel.
pub struct Tables {
    /// The Gaussian value of each eight-bit level (`k / 255`): the standard normal quantile of the level's mid-rank
    /// among the texture's bilinear reads ([`Tables::of`]), kept within half a read's rank of either end.
    pub forward: [[f64; 4]; LEVELS],
    /// The value of each Gaussian value `-G_MAX + j · 2 G_MAX / (INVERSE − 1)`: the forward table's piecewise-linear
    /// map inverted (0 below its least, 1 above its greatest).
    pub inverse: Vec<[f64; 4]>,
    /// The mean Gaussian value over the texture's bilinear reads, a channel each.
    pub mean: [f64; 4],
}

impl Tables {
    /// Builds the tables of a 512² texture's texels (`texels`: four channels a texel, eight bits each), as its bilinear
    /// reads find it: the ranks are those of four reads within each texel's square, at its quarter points (sixteenths
    /// of a level apart, counted exactly), so the Gaussian values of reads between texels are Gaussian too.
    pub fn of(texels: &[u8]) -> Self {
        const SIZE: usize = 512;
        const SUB: usize = 16;
        let bins = (LEVELS - 1) * SUB + 1;
        let mut forward = [[0.0; 4]; LEVELS];
        let mut mean = [0.0; 4];
        for c in 0..4 {
            let at =
                |x: usize, y: usize| usize::from(texels[((y % SIZE) * SIZE + x % SIZE) * 4 + c]);
            let mut counts = vec![0usize; bins];
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let (a, b, d, e) = (at(x, y), at(x + 1, y), at(x, y + 1), at(x + 1, y + 1));
                    // The quarter points' weights, in sixteenths: 9, 3, 3, 1 toward the nearest texel.
                    counts[9 * a + 3 * b + 3 * d + e] += 1;
                    counts[3 * a + 9 * b + d + 3 * e] += 1;
                    counts[3 * a + b + 9 * d + 3 * e] += 1;
                    counts[a + 3 * b + 3 * d + 9 * e] += 1;
                }
            }
            let n = (4 * SIZE * SIZE) as f64;
            let half = 0.5 / n;
            let mut below = 0usize;
            let mut ranks = vec![0.0; bins];
            for (bin, count) in counts.iter().enumerate() {
                ranks[bin] = ((below as f64 + 0.5 * *count as f64) / n).clamp(half, 1.0 - half);
                below += count;
            }
            for (k, row) in forward.iter_mut().enumerate() {
                row[c] = normal_quantile(ranks[k * SUB]);
            }
            for (bin, count) in counts.iter().enumerate() {
                let k = bin / SUB;
                let f = (bin % SUB) as f64 / SUB as f64;
                let g = if k + 1 < LEVELS {
                    forward[k][c] + (forward[k + 1][c] - forward[k][c]) * f
                } else {
                    forward[k][c]
                };
                mean[c] += g * *count as f64 / n;
            }
        }
        let mut inverse = vec![[0.0; 4]; INVERSE];
        for (j, row) in inverse.iter_mut().enumerate() {
            let g = -G_MAX + j as f64 * 2.0 * G_MAX / (INVERSE - 1) as f64;
            for c in 0..4 {
                row[c] = if g <= forward[0][c] {
                    0.0
                } else if g >= forward[LEVELS - 1][c] {
                    1.0
                } else {
                    // The level below which the table first passes `g`, and the share of the way to the next.
                    let k = (0..LEVELS - 1)
                        .find(|&k| forward[k + 1][c] > g)
                        .expect("within the table");
                    let (a, b) = (forward[k][c], forward[k + 1][c]);
                    (k as f64 + ((g - a) / (b - a)).clamp(0.0, 1.0)) / (LEVELS - 1) as f64
                };
            }
        }
        Self {
            forward,
            inverse,
            mean,
        }
    }

    /// The Gaussian values of a texel's four channels.
    pub fn gauss(&self, v: [f64; 4]) -> [f64; 4] {
        std::array::from_fn(|c| {
            let x = v[c].clamp(0.0, 1.0) * (LEVELS - 1) as f64;
            let k = (x.floor() as usize).min(LEVELS - 2);
            let f = x - k as f64;
            self.forward[k][c] + (self.forward[k + 1][c] - self.forward[k][c]) * f
        })
    }

    /// The texel values of four channels' Gaussian values.
    pub fn value(&self, g: [f64; 4]) -> [f64; 4] {
        std::array::from_fn(|c| {
            let y = ((g[c] + G_MAX) / (2.0 * G_MAX)).clamp(0.0, 1.0) * (INVERSE - 1) as f64;
            let j = (y.floor() as usize).min(INVERSE - 2);
            let f = y - j as f64;
            self.inverse[j][c] + (self.inverse[j + 1][c] - self.inverse[j][c]) * f
        })
    }

    /// The anchors' texels `parts` (each with its weight, any scale) blended: one alone is itself; more keep the
    /// texture's statistics (the module's notes).
    pub fn blend(&self, parts: impl IntoIterator<Item = ([f64; 4], f64)>) -> [f64; 4] {
        let mut total = 0.0;
        let mut squares = 0.0;
        let mut sum = [0.0; 4];
        let mut round_trip = [0.0; 4];
        let mut count = 0;
        let mut first = [0.0; 4];
        for (v, w) in parts {
            if count == 0 {
                first = v;
            }
            count += 1;
            let g = self.gauss(v);
            let back = self.value(g);
            for c in 0..4 {
                sum[c] += w * (g[c] - self.mean[c]);
                round_trip[c] += w * (v[c] - back[c]);
            }
            total += w;
            squares += w * w;
        }
        if count == 1 {
            return first;
        }
        let root = squares.sqrt();
        let blended = self.value(std::array::from_fn(|c| self.mean[c] + sum[c] / root));
        std::array::from_fn(|c| (blended[c] + round_trip[c] / total).clamp(0.0, 1.0))
    }
}

/// The tables of the clouds' weather texture (`super::weather_bytes`), built once.
pub fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables::of(super::weather_bytes()))
}

/// The standard normal quantile of `p` in (0, 1) (Acklam's rational approximation, refined once by Halley's step on
/// the complementary error function's series: about 1e-15 relative).
pub fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    let low = 0.02425;
    let x = if p < low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= 1.0 - low {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    // One Halley step against the normal CDF.
    let e = 0.5 * erfc(-x / std::f64::consts::SQRT_2) - p;
    let u = e * (2.0 * std::f64::consts::PI).sqrt() * (x * x / 2.0).exp();
    x - u / (1.0 + x * u / 2.0)
}

/// The complementary error function (Numerical Recipes' `erfcc`, about 1.2e-7 relative): enough for one Halley step.
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.265_512_23
            + t * (1.000_023_68
                + t * (0.374_091_96
                    + t * (0.096_784_18
                        + t * (-0.186_288_06
                            + t * (0.278_868_07
                                + t * (-1.135_203_98
                                    + t * (1.488_515_87
                                        + t * (-0.822_152_23 + t * 0.170_872_77)))))))))
            .exp();
    if x >= 0.0 { r } else { 2.0 - r }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quantile_inverts_the_normal_distribution() {
        for (p, x) in [
            (0.5, 0.0),
            (0.975, 1.959_963_984_540_054),
            (0.001, -3.090_232_306_167_813),
        ] {
            assert!(
                (normal_quantile(p) - x).abs() < 1e-6,
                "{p}: {}",
                normal_quantile(p)
            );
        }
    }

    /// One anchor reads its own value; a weight going to nothing leaves the rest's blend; the round trip of a value
    /// through the tables is near it.
    #[test]
    fn the_blend_is_continuous_in_its_weights() {
        let t = tables();
        let a = [0.31, 0.0, 0.62, 0.4];
        let b = [0.8, 0.45, 0.2, 0.55];
        assert_eq!(t.blend([(a, 1.0)]), a);
        let nearly = t.blend([(a, 1.0), (b, 1e-12)]);
        for c in 0..4 {
            assert!((nearly[c] - a[c]).abs() < 1e-9, "{nearly:?} against {a:?}");
            let back = t.value(t.gauss(a))[c];
            assert!(
                (back - a[c]).abs() < 0.01,
                "channel {c}: {back} against {}",
                a[c]
            );
        }
        let half = t.blend([(a, 1.0), (b, 1.0)]);
        assert!(half.iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
