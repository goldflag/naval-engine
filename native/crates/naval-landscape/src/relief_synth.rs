//! Synthetic fine relief (native/docs/terrain/ground.md, "Fine relief near the camera"): where no finer elevation model
//! covers the land (`relief::source_at` none: the Alps, Taiwan), a page's detail is made from the window's own heights.
//!
//! - **The base**: the window's samples drawn smooth (Catmull-Rom between them) rather than bilinear, so a crest or a
//!   valley bends between its samples (`Params::curvature`).
//! - **Gullies** run down the fall lines, octave over octave from `Params::wavelength_m` halving to a few metres: each
//!   octave's stripes (phacelle noise: four cells' waves across the slope, their phases blended and made unit, so the
//!   stripes stay whole and fork where the cells disagree) turned by the slope the coarser octaves leave, so the finer
//!   run off the sides of the coarser and branch. After Rune Skovbo Johansen's erosion filter (2024-25).
//! - **Their shape**: a narrow notch at each stripe's trough (`notch`: V-shaped, a quarter of the wavelength wide), the
//!   ground between them a little rounded (`round`): gullies cut into the slope rather than corrugating it.
//! - **How deep**: each octave a share of its wavelength (`Params::amplitude`, `gain` from one to the next), times how
//!   steep the slope is where it falls (`slope`) and how much the land about rises (its range within `radius_m`,
//!   `relief_m`); none on flats, valley floors, lakes and the shore (`shore_m`).
//! - **Beds**: the smooth heights stepped into benches and risers along their contours (`terrace_m` apart, two sets of
//!   beds out of step, their surfaces undulating), strongly where GLiM's rock is sedimentary (sandstone, limestone: the
//!   canyons' cliff bands), lightly elsewhere (ledges); the rock read at the page's corners, so it blends between pages.
//! - **Through the same residual** as a finer model's page (`relief::make_page`): its heights less the lattice's
//!   bilinear surface through its own heights at the samples, so a page is zero at the samples and never moves the
//!   land's broad shape. The noise is a function of the theatre's metres: neighbouring pages agree in their gutters.
//!
//! `NAVAL_RELIEF_SYNTH` (`key=value,...`, [`Params::parse`]) overrides the parameters for tuning.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use naval_earth::geo::{LatLon, to_lat_lon};
use naval_terrain::ReliefPage;
use naval_terrain::earth::{KoppenGroup, Lithology};
use naval_terrain::relief::{GUTTER, PAGE_TEXELS};

use super::relief::{Lattice, TEXEL_M, page_m, reach};
use super::relief_limit::{self, Water};

/// What the generator makes of the window's heights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Octaves of gullies, the first `wavelength_m` apart (m), each next half as far.
    pub octaves: u32,
    pub wavelength_m: f32,
    /// An octave's amplitude as a share of its wavelength, at full strength.
    pub amplitude: f32,
    /// Each octave's amplitude over the one before (its wavelength is half).
    pub gain: f32,
    /// How much an octave's own slope turns the finer octaves' stripes (0: all run down the window's fall line).
    pub branch: f32,
    /// The gradient (m/m) where gullies start, and where they are full.
    pub slope: [f32; 2],
    /// The land's range (m) within `radius_m` where gullies start, and where they are full.
    pub relief_m: [f32; 2],
    pub radius_m: f32,
    /// The height (m) above the sea where gullies start, and where they are full.
    pub shore_m: [f32; 2],
    /// The smooth base's share: 0 the window's bilinear surface, 1 Catmull-Rom between its samples.
    pub curvature: f32,
    /// The gullies' notch: its half-width as |cos(phase / 2)| at its edge (0: a plain cosine, 0.4: a quarter of the
    /// wavelength wide), and the cosine's share added to round the ground between.
    pub notch: f32,
    pub round: f32,
    /// How much each octave's depth varies from place to place (0 even, 1 from none to full).
    pub patchiness: f32,
    /// The beds' spacing (m; 0 none), how far they step the land (0 not at all, 1 flat benches and risers) in rock
    /// that is not sedimentary and in rock that is, and how steep a riser stands over the slope's mean.
    pub terrace_m: f32,
    pub terrace: f32,
    pub terrace_sediment: f32,
    pub riser: f32,
    /// Over a finer model's shape (`residuals_on`, the combined source): the octaves longer than `under_m` and the beds
    /// kept where it is used (`under_kept`; 0 none: its own shape stands for them).
    pub under_m: f32,
    pub under_kept: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            octaves: 4,
            wavelength_m: 240.0,
            amplitude: 0.1,
            gain: 0.42,
            branch: 0.5,
            slope: [0.1, 0.6],
            relief_m: [15.0, 200.0],
            radius_m: 320.0,
            shore_m: [4.0, 40.0],
            curvature: 1.0,
            notch: 0.4,
            round: 0.25,
            patchiness: 0.8,
            terrace_m: 90.0,
            terrace: 0.05,
            terrace_sediment: 1.0,
            riser: 3.0,
            under_m: 60.0,
            under_kept: 0.0,
        }
    }
}

impl Params {
    /// The defaults with `spec`'s changes: `key=value` pairs, commas between, a range as `low:high`
    /// (`amplitude=0.1,slope=0.15:0.7`).
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut p = Self::default();
        for pair in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("{pair}: not key=value"))?;
            let num = |v: &str| v.trim().parse::<f32>().map_err(|e| format!("{pair}: {e}"));
            let range = |v: &str| -> Result<[f32; 2], String> {
                let (a, b) = v
                    .split_once(':')
                    .ok_or_else(|| format!("{pair}: a range is low:high"))?;
                Ok([num(a)?, num(b)?])
            };
            match key.trim() {
                "octaves" => p.octaves = num(value)?.max(0.0) as u32,
                "wavelength" | "wavelength_m" => p.wavelength_m = num(value)?,
                "amplitude" => p.amplitude = num(value)?,
                "gain" => p.gain = num(value)?,
                "branch" => p.branch = num(value)?,
                "slope" => p.slope = range(value)?,
                "relief" | "relief_m" => p.relief_m = range(value)?,
                "radius" | "radius_m" => p.radius_m = num(value)?,
                "shore" | "shore_m" => p.shore_m = range(value)?,
                "curvature" => p.curvature = num(value)?,
                "notch" => p.notch = num(value)?,
                "round" => p.round = num(value)?,
                "patchiness" => p.patchiness = num(value)?,
                "terrace_m" | "beds" => p.terrace_m = num(value)?,
                "terrace" => p.terrace = num(value)?,
                "terrace_sediment" | "sediment" => p.terrace_sediment = num(value)?,
                "riser" => p.riser = num(value)?,
                "under_m" | "under" => p.under_m = num(value)?,
                "under_kept" | "kept" => p.under_kept = num(value)?,
                other => return Err(format!("{other}: no such parameter")),
            }
        }
        Ok(p)
    }

    /// The parameters the client makes its pages with: the defaults, or `NAVAL_RELIEF_SYNTH`'s.
    pub fn current() -> Self {
        static CURRENT: OnceLock<Params> = OnceLock::new();
        *CURRENT.get_or_init(|| match std::env::var("NAVAL_RELIEF_SYNTH") {
            Ok(spec) => Self::parse(&spec).unwrap_or_else(|e| {
                eprintln!("NAVAL_RELIEF_SYNTH: {e}; the defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        })
    }
}

/// The page's own threshold: a page whose residual stays under this everywhere (m) is none.
const NONE_BELOW_M: f32 = 0.05;
/// The octaves' seeds, and how far each octave's frame is turned from the last (radians): no grid shared by two.
const SEED: u32 = 0x5EED_0000;
const TURN: f64 = 1.234;

fn smoothstep([lo, hi]: [f32; 2], x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A well-mixed 32 bits of a cell and a seed.
#[inline]
fn hash(x: i64, z: i64, seed: u32) -> u32 {
    let mut h = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ u64::from(seed).wrapping_mul(0x1656_67B1_9E37_79F9);
    h ^= h >> 31;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 29;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 32;
    h as u32
}

/// `h` as [0, 1).
#[inline]
fn unit(h: u32) -> f32 {
    (h >> 8) as f32 * (1.0 / 16_777_216.0)
}

/// sin(2π x), to about 0.001: a parabola refined (plenty for a terrain's noise, a few times cheaper than `sin`).
#[inline]
fn sin_turns(x: f32) -> f32 {
    let r = x - (x + 0.5).floor();
    let y = 8.0 * r - 16.0 * r * r.abs();
    0.225 * (y * y.abs() - y) + y
}

/// One octave's stripes at `p` (metres, in the octave's turned frame), `wavelength` apart across `t` (unit, in the same
/// frame): the cosine and sine of the four nearest cells' phases blended (each cell's wave offset by its own random
/// phase, weighted by a smoothed bilinear tent) and made unit, and the blend's length before (near 0 where the cells'
/// phases cancel, where the stripes fork).
#[inline]
fn stripes(p: [f64; 2], wavelength: f64, t: [f32; 2], seed: u32) -> (f32, f32, f32) {
    let q = [p[0] / wavelength - 0.5, p[1] / wavelength - 0.5];
    let b = [q[0].floor(), q[1].floor()];
    let f = [(q[0] - b[0]) as f32, (q[1] - b[1]) as f32];
    let (bx, bz) = (b[0] as i64, b[1] as i64);
    let w = f.map(|v| v * v * (3.0 - 2.0 * v));
    let (mut c, mut s) = (0f32, 0f32);
    for (dx, dz) in [(0i64, 0i64), (1, 0), (0, 1), (1, 1)] {
        let wx = if dx == 0 { 1.0 - w[0] } else { w[0] };
        let wz = if dz == 0 { 1.0 - w[1] } else { w[1] };
        let phase = (f[0] - dx as f32) * t[0]
            + (f[1] - dz as f32) * t[1]
            + unit(hash(bx + dx, bz + dz, seed));
        c += wx * wz * sin_turns(phase + 0.25);
        s += wx * wz * sin_turns(phase);
    }
    let len = c.hypot(s);
    if len < 1e-6 {
        return (0.0, 0.0, 0.0);
    }
    (c / len, s / len, len)
}

/// The window's samples about a page: their heights and the land's range about each.
struct Patch {
    node: [f64; 2],
    cell: f64,
    /// The lattice index of the patch's sample (0, 0), and its samples a side.
    first: [i64; 2],
    count: [usize; 2],
    heights: Vec<f32>,
    /// The range (highest less lowest) of the samples within the relief's radius of each (m).
    range: Vec<f32>,
}

impl Patch {
    fn new(
        lattice: &Lattice,
        first: [i64; 2],
        count: [usize; 2],
        radius: usize,
        height: &(dyn Fn([f64; 2]) -> f32 + Sync),
    ) -> Self {
        let mut heights = vec![0f32; count[0] * count[1]];
        for j in 0..count[1] {
            for i in 0..count[0] {
                heights[j * count[0] + i] = height([
                    lattice.node[0] + (first[0] + i as i64) as f64 * lattice.cell,
                    lattice.node[1] + (first[1] + j as i64) as f64 * lattice.cell,
                ]);
            }
        }
        // The range over the square of `radius` samples each way: the rows' extremes, then the columns'.
        let (w, h) = (count[0], count[1]);
        let mut row_max = vec![0f32; w * h];
        let mut row_min = vec![0f32; w * h];
        for j in 0..h {
            for i in 0..w {
                let (a, b) = (i.saturating_sub(radius), (i + radius).min(w - 1));
                let row = &heights[j * w + a..=j * w + b];
                row_max[j * w + i] = row.iter().copied().fold(f32::MIN, f32::max);
                row_min[j * w + i] = row.iter().copied().fold(f32::MAX, f32::min);
            }
        }
        let mut range = vec![0f32; w * h];
        for j in 0..h {
            let (a, b) = (j.saturating_sub(radius), (j + radius).min(h - 1));
            for i in 0..w {
                let (mut most, mut least) = (f32::MIN, f32::MAX);
                for k in a..=b {
                    most = most.max(row_max[k * w + i]);
                    least = least.min(row_min[k * w + i]);
                }
                range[j * w + i] = most - least;
            }
        }
        Self {
            node: lattice.node,
            cell: lattice.cell,
            first,
            count,
            heights,
            range,
        }
    }

    /// Theatre point `p` in the patch's samples: the sample below and left of it, and how far on to the next.
    #[inline]
    fn locate(&self, p: [f64; 2]) -> ([usize; 2], [f32; 2]) {
        let t = [0, 1].map(|k| (p[k] - self.node[k]) / self.cell - self.first[k] as f64);
        let a = t.map(f64::floor);
        let at = [0, 1].map(|k| (a[k].max(1.0) as usize).min(self.count[k] - 3));
        ([at[0], at[1]], [(t[0] - a[0]) as f32, (t[1] - a[1]) as f32])
    }

    /// At `p`: Catmull-Rom's height and its gradient (m/m), the bilinear surface's height, and the samples' range
    /// there (bilinear).
    #[inline]
    fn sample(&self, p: [f64; 2]) -> (f32, [f32; 2], f32, f32) {
        let (at, f) = self.locate(p);
        let weights = |t: f32| {
            let (t2, t3) = (t * t, t * t * t);
            (
                [
                    0.5 * (-t3 + 2.0 * t2 - t),
                    0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
                    0.5 * (-3.0 * t3 + 4.0 * t2 + t),
                    0.5 * (t3 - t2),
                ],
                [
                    0.5 * (-3.0 * t2 + 4.0 * t - 1.0),
                    0.5 * (9.0 * t2 - 10.0 * t),
                    0.5 * (-9.0 * t2 + 8.0 * t + 1.0),
                    0.5 * (3.0 * t2 - 2.0 * t),
                ],
            )
        };
        let (wx, dx) = weights(f[0]);
        let (wz, dz) = weights(f[1]);
        let w = self.count[0];
        let (mut h, mut gx, mut gz) = (0f32, 0f32, 0f32);
        for (b, (&wzb, &dzb)) in wz.iter().zip(&dz).enumerate() {
            let row = (at[1] + b - 1) * w + at[0] - 1;
            let s = &self.heights[row..row + 4];
            let along = s[0] * wx[0] + s[1] * wx[1] + s[2] * wx[2] + s[3] * wx[3];
            let slope = s[0] * dx[0] + s[1] * dx[1] + s[2] * dx[2] + s[3] * dx[3];
            h += wzb * along;
            gx += wzb * slope;
            gz += dzb * along;
        }
        let inv = 1.0 / self.cell as f32;
        let [u, v] = f;
        let k = at[1] * w + at[0];
        let lerp = |a: &[f32]| {
            (a[k] * (1.0 - u) + a[k + 1] * u) * (1.0 - v)
                + (a[k + w] * (1.0 - u) + a[k + w + 1] * u) * v
        };
        (
            h,
            [gx * inv, gz * inv],
            lerp(&self.heights),
            lerp(&self.range),
        )
    }
}

/// The detail at theatre point `p` over land `h` high there, falling `grad` (m/m), with `range` about it.
struct Gullies {
    params: Params,
    /// Each octave's turn (cos, sin).
    turns: Vec<(f64, f64)>,
}

impl Gullies {
    fn new(params: Params) -> Self {
        let turns = (0..params.octaves)
            .map(|k| {
                let a = TURN * f64::from(k);
                (a.cos(), a.sin())
            })
            .collect();
        Self { params, turns }
    }

    /// `terrace`: how far the beds step the land here (0 to 1, the rock's); `coarse`: the share of the octaves longer
    /// than `Params::under_m` and of the beds (1 but over a finer model's shape).
    #[inline]
    fn at(
        &self,
        p: [f64; 2],
        h: f32,
        grad: [f32; 2],
        range: f32,
        terrace: f32,
        coarse: f32,
    ) -> f32 {
        let params = &self.params;
        let mask = smoothstep(params.shore_m, h) * smoothstep(params.relief_m, range);
        if mask <= 0.0 {
            return 0.0;
        }
        let mut g = grad;
        let mut out = 0f32;
        let mut wavelength = f64::from(params.wavelength_m);
        let mut amplitude = params.amplitude * params.wavelength_m * mask;
        let (notch, round) = (params.notch, params.round);
        for (k, &(c, s)) in self.turns.iter().enumerate() {
            let steep = g[0].hypot(g[1]);
            let share = if wavelength > f64::from(params.under_m) {
                coarse
            } else {
                1.0
            };
            let fade = smoothstep(params.slope, steep) * share;
            if fade > 0.0 {
                // Across the slope: the stripes' crests and troughs run down it.
                let t = [-g[1] / steep, g[0] / steep];
                let (cf, sf) = (c as f32, s as f32);
                let turned = [p[0] * c + p[1] * s, p[1] * c - p[0] * s];
                let across = [t[0] * cf + t[1] * sf, t[1] * cf - t[0] * sf];
                let (cos, sin, len) = stripes(turned, wavelength, across, SEED + k as u32);
                // Patchy: each octave deep in some places and faint in others, a few wavelengths across, so the
                // gullies do not flute a whole slope evenly.
                let patch = value_noise(p, 3.0 * wavelength, SEED ^ (0xA11 + k as u32));
                let patchy = 1.0 - params.patchiness * (1.0 - smoothstep([0.25, 0.75], patch));
                let a = amplitude * fade * patchy * (len * 3.0).min(1.0);
                // The shape, and its change a turn of the phase: the notch from |cos(phase / 2)| (0 at the trough,
                // falling linearly from it), the rounding a cosine.
                let (mut v, mut dv) = (round * cos, -round * sin * TAU);
                if notch > 0.0 {
                    let q = (0.5 * (1.0 + cos)).max(0.0).sqrt();
                    if q < notch {
                        v -= 1.0 - q / notch;
                        // d|cos(θ/2)|/dθ = -sin θ / (4 |cos(θ/2)|), a turn 2π.
                        dv += sin / (4.0 * q.max(1e-3) * notch) * TAU;
                    }
                } else {
                    v += (1.0 - round) * cos;
                    dv -= (1.0 - round) * sin * TAU;
                }
                out += a * v;
                let turn = a * dv / wavelength as f32 * params.branch;
                g = [g[0] + turn * t[0], g[1] + turn * t[1]];
            }
            wavelength *= 0.5;
            amplitude *= params.gain;
        }
        let beds = params.terrace_m;
        if beds > 0.0 && terrace > 0.0 {
            let steep = grad[0].hypot(grad[1]);
            // In bands and patches, not everywhere: the harder beds stand out as cliffs where noise a few hundred
            // metres across says so.
            let hard = smoothstep([0.35, 0.7], value_noise(p, 450.0, SEED ^ 0xCA5));
            let strength = terrace * mask * hard * smoothstep([0.1, 0.4], steep) * coarse;
            if strength > 0.0 {
                // The beds' surfaces undulate by up to a bed's thickness over a kilometre or so.
                let wobble = beds * (value_noise(p, 700.0, SEED ^ 0xBED) - 0.5) * 2.0;
                let stepped = |spacing: f32, seed: u32| {
                    let x = (h + wobble) / spacing + unit(hash(0, 0, seed));
                    let f = x - x.floor();
                    spacing * (riser(f, params.riser.round() as i32) - f)
                };
                out += strength
                    * (0.65 * stepped(beds, SEED ^ 1) + 0.35 * stepped(beds * 1.73, SEED ^ 2));
            }
        }
        out
    }
}

/// A bed's profile through its thickness: flat at its foot and its top (the benches), steepest halfway (the riser),
/// `steep` times its mean there; C¹ from one bed to the next.
#[inline]
fn riser(f: f32, steep: i32) -> f32 {
    let n = steep.max(1);
    if f < 0.5 {
        0.5 * (2.0 * f).powi(n)
    } else {
        1.0 - 0.5 * (2.0 * (1.0 - f)).powi(n)
    }
}

/// Smooth value noise in [0, 1) on a grid `cell` metres apart.
#[inline]
fn value_noise(p: [f64; 2], cell: f64, seed: u32) -> f32 {
    let q = [p[0] / cell, p[1] / cell];
    let b = [q[0].floor(), q[1].floor()];
    let f = [(q[0] - b[0]) as f32, (q[1] - b[1]) as f32].map(|v| v * v * (3.0 - 2.0 * v));
    let (x, z) = (b[0] as i64, b[1] as i64);
    let at = |dx: i64, dz: i64| unit(hash(x + dx, z + dz, seed));
    (at(0, 0) * (1.0 - f[0]) + at(1, 0) * f[0]) * (1.0 - f[1])
        + (at(0, 1) * (1.0 - f[0]) + at(1, 1) * f[0]) * f[1]
}

/// How far the beds step the land at a page's corners (west-north, east-north, west-south, east-south: the order of
/// its texels), from GLiM's rock there: sedimentary rock's `terrace_sediment` in a dry climate (Köppen's B), a quarter
/// of it in a wet one, the rest's `terrace`.
pub fn style(geo: LatLon, index: [i64; 2], params: &Params) -> [f32; 4] {
    let s = Style::of(geo, index, params);
    [
        s.corners[1][1],
        s.corners[1][2],
        s.corners[2][1],
        s.corners[2][2],
    ]
}

/// How far the beds step the land about a page: [`style`]'s value at the corners of the page and its eight
/// neighbours, bilinear within the page holding a point, so two pages reading the same point agree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    index: [i64; 2],
    /// `corners[b][a]`: the corner `a - 1` pages east and `b - 1` south of the page's west-north one.
    corners: [[f32; 4]; 4],
}

impl Style {
    /// About page `index` from the rock and the climate.
    pub fn of(geo: LatLon, index: [i64; 2], params: &Params) -> Self {
        let side = page_m();
        let corners = std::array::from_fn(|b| {
            std::array::from_fn(|a| {
                let p = to_lat_lon(
                    geo,
                    [
                        (index[0] + a as i64 - 1) as f64 * side,
                        (index[1] + b as i64 - 1) as f64 * side,
                    ],
                );
                corner_style(p, params)
            })
        });
        Self { index, corners }
    }

    /// Page `index`'s own corners (west-north, east-north, west-south, east-south), held at its edges beyond it.
    pub fn page(index: [i64; 2], own: [f32; 4]) -> Self {
        let [a, b, c, d] = own;
        let north = [a, a, b, b];
        let south = [c, c, d, d];
        Self {
            index,
            corners: [north, north, south, south],
        }
    }

    /// At theatre point `p`: bilinear between the corners of the page holding it (one of the nine).
    #[inline]
    pub fn at(&self, p: [f64; 2]) -> f32 {
        let side = page_m();
        let t = [0, 1].map(|k| (p[k] / side - self.index[k] as f64).clamp(-1.0, 2.0 - 1e-9));
        let a = t.map(|v| v.floor());
        let [u, v] = [0, 1].map(|k| (t[k] - a[k]) as f32);
        let [i, j] = [0, 1].map(|k| (a[k] + 1.0) as usize);
        let c = &self.corners;
        (c[j][i] * (1.0 - u) + c[j][i + 1] * u) * (1.0 - v)
            + (c[j + 1][i] * (1.0 - u) + c[j + 1][i + 1] * u) * v
    }
}

fn corner_style(p: LatLon, params: &Params) -> f32 {
    let sediment = match naval_terrain::earth::rock_near(p.lat, p.lon) {
        Some(
            Lithology::SiliciclasticSediments
            | Lithology::MixedSediments
            | Lithology::CarbonateSediments
            | Lithology::Evaporites,
        ) => 1.0,
        Some(Lithology::Pyroclastics | Lithology::BasicVolcanics) => 0.5,
        _ => 0.0,
    };
    // Bare beds stand out where the land is dry; under a wet climate's soil and forest, a quarter of them.
    let arid =
        naval_terrain::earth::koppen(p.lat, p.lon).is_some_and(|k| k.group() == KoppenGroup::Arid);
    let sediment = if arid { sediment } else { 0.25 * sediment };
    params.terrace + sediment * (params.terrace_sediment - params.terrace)
}

/// Page `index`'s residuals (`PAGE_TEXELS`² with their gutters, row 0 at the smallest z) on `lattice`, the window's
/// height at a theatre point from `height` (read at the lattice's samples only).
pub fn residuals(
    lattice: &Lattice,
    index: [i64; 2],
    height: &(dyn Fn([f64; 2]) -> f32 + Sync),
    params: &Params,
    style: [f32; 4],
) -> Vec<f32> {
    residuals_on(
        lattice,
        index,
        height,
        params,
        &Style::page(index, style),
        None,
        0,
    )
}

/// A finer model's shape under the synthetic detail, at a theatre point: the share of it used there (0 to 1), its
/// residual over the window's bilinear surface (m) and that residual's slope (m/m).
pub type Under<'a> = &'a (dyn Fn([f64; 2]) -> (f32, f32, [f32; 2]) + Sync);

/// [`residuals`] over a finer model's shape (`under`, the combined source's GLO-30 band) where one is given: there the
/// smooth base is its residual rather than Catmull-Rom's, the gullies run down its slopes and follow its heights, and
/// the octaves longer than `Params::under_m` and the beds fade by its share (its own shape stands for them); elsewhere,
/// and where its share is 0, the synthetic as without it. Over the page's texels and `extra` more each way
/// (`PAGE_TEXELS + 2 extra` a side), its beds as `style` says.
pub fn residuals_on(
    lattice: &Lattice,
    index: [i64; 2],
    height: &(dyn Fn([f64; 2]) -> f32 + Sync),
    params: &Params,
    style: &Style,
    under: Option<Under>,
    extra: usize,
) -> Vec<f32> {
    let side = page_m();
    let corner = [index[0] as f64 * side, index[1] as f64 * side];
    let (lo, hi) = reach(lattice);
    let wide = extra as f64 * TEXEL_M;
    let (lo, hi) = (lo - wide, hi + wide);
    // The samples the residual's surface stands on (as `relief::make_page`), and the patch about them: the relief's
    // radius and Catmull-Rom's reach beyond.
    let first = [0, 1].map(|k| ((corner[k] + lo - lattice.node[k]) / lattice.cell).floor() as i64);
    let last = [0, 1].map(|k| ((corner[k] + hi - lattice.node[k]) / lattice.cell).ceil() as i64);
    let count = [0, 1].map(|k| (last[k] - first[k] + 1) as usize);
    let radius = (f64::from(params.radius_m) / lattice.cell).ceil().max(1.0) as usize;
    let pad = radius + 2;
    let patch = Patch::new(
        lattice,
        first.map(|f| f - pad as i64),
        count.map(|c| c + 2 * pad),
        radius,
        height,
    );
    let gullies = Gullies::new(*params);
    let curvature = params.curvature;
    // The smooth part of the residual (Catmull-Rom's over the bilinear surface, or the finer model's) and the detail.
    let detail = |p: [f64; 2]| {
        let (h, grad, flat, range) = patch.sample(p);
        let terrace = style.at(p);
        let base = curvature * smoothstep(params.shore_m, flat) * (h - flat);
        let Some((w, b, slope)) = under.map(|f| f(p)).filter(|u| u.0 > 0.0) else {
            return (base, gullies.at(p, h, grad, range, terrace, 1.0));
        };
        // The heights and slopes the detail follows: the finer model's where it is used, Catmull-Rom's elsewhere.
        let h = h + w * (flat + b - h);
        let grad = [grad[0] + w * slope[0], grad[1] + w * slope[1]];
        let coarse = 1.0 - w * (1.0 - params.under_kept);
        (
            base + w * (b - base),
            gullies.at(p, h, grad, range, terrace, coarse),
        )
    };
    // The detail at the samples: the bilinear surface through them is what the residual leaves out.
    let mut nodes = vec![0f32; count[0] * count[1]];
    for j in 0..count[1] {
        for i in 0..count[0] {
            let p = [
                lattice.node[0] + (first[0] + i as i64) as f64 * lattice.cell,
                lattice.node[1] + (first[1] + j as i64) as f64 * lattice.cell,
            ];
            nodes[j * count[0] + i] = detail(p).1;
        }
    }
    let n = PAGE_TEXELS as usize + 2 * extra;
    let g = f64::from(GUTTER) + extra as f64;
    let mut texels = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            let p = [
                corner[0] + (i as f64 - g + 0.5) * TEXEL_M,
                corner[1] + (j as f64 - g + 0.5) * TEXEL_M,
            ];
            let (base, d) = detail(p);
            let t = [0, 1].map(|k| (p[k] - lattice.node[k]) / lattice.cell - first[k] as f64);
            let (a, b) = (t[0].floor(), t[1].floor());
            let (u, v) = ((t[0] - a) as f32, (t[1] - b) as f32);
            let (a, b) = (
                (a as usize).min(count[0] - 2),
                (b as usize).min(count[1] - 2),
            );
            let at = |i: usize, j: usize| nodes[j * count[0] + i];
            let surface = (at(a, b) * (1.0 - u) + at(a + 1, b) * u) * (1.0 - v)
                + (at(a, b + 1) * (1.0 - u) + at(a + 1, b + 1) * u) * v;
            texels[j * n + i] = base + d - surface;
        }
    }
    texels
}

/// Page `index` about `geo` on `lattice` from the window's heights (`height` at a theatre point), held to the window
/// and off `water` (`relief_limit::hold`): none where it would hold no relief (the sea, flats).
pub fn make_page(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    height: &(dyn Fn([f64; 2]) -> f32 + Sync),
    water: &Water,
) -> Option<ReliefPage> {
    let params = Params::current();
    let texels = held(geo, lattice, index, height, &params, water);
    let mut page = ReliefPage::from_residual(&texels, TEXEL_M as f32);
    page.real = false;
    (page.peak >= NONE_BELOW_M).then_some(page)
}

/// Page `index`'s residuals (as [`make_page`] takes them): made [`relief_limit::MARGIN`] past its texels, held to the
/// window and off `water`.
pub fn held(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    height: &(dyn Fn([f64; 2]) -> f32 + Sync),
    params: &Params,
    water: &Water,
) -> Vec<f32> {
    let style = Style::of(geo, index, params);
    let grid = residuals_on(
        lattice,
        index,
        height,
        params,
        &style,
        None,
        relief_limit::MARGIN,
    );
    relief_limit::hold(&grid, &relief_limit::weights(index, water, height))
}

#[cfg(test)]
#[path = "relief_synth_tests.rs"]
mod tests;
