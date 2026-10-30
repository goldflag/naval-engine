//! The combined fine relief (`NAVAL_RELIEF_SOURCE=combined`, [`super::relief::Source::Combined`]): GLO-30's band with
//! its fills kept (`relief_glo30`: Copernicus's 30 m in place of the window's heights at wavelengths under about 330
//! m) for the land's shapes from about 60 m up, and the synthetic's gullies (`relief_synth`) under 60 m on top, made
//! on the heights the band leaves and down its slopes. Where GLO-30 is masked (the canopy, roofs, its voids) or absent
//! the page is the synthetic's whole, the two blended over the mask's fade.
//!
//! - *The band* moves the window's samples (by metres, tens of metres on some crests): what the simulation does not
//!   see. [`Options::cap_m`] holds the samples within it of the window's heights, smoothly ([`hold_samples`]): what
//!   the band would move a sample further is taken off as a bilinear surface through the samples (as a finer model's
//!   residual is), so the band's shape between them stays.
//! - *The fills' edges*: where GLO-30's filled heights meet its own, the band is blurred over [`Options::seam_m`]
//!   either side, so a step between two models does not show.
//!
//! `NAVAL_RELIEF_COMBINED` (`cap=35,seam=60`, [`Options::parse`]) overrides them for tuning.

use std::sync::OnceLock;

use naval_earth::geo::LatLon;
use naval_terrain::ReliefPage;
use naval_terrain::relief::{GUTTER, PAGE_TEXELS};

use super::relief::{self, Coarse, Lattice, TEXEL_M};
use super::relief_glo30::{self, EXTRA, Inputs};
use super::relief_limit::{self, Water};
use super::relief_synth::{self, Params};

/// GLO-30 as the combined source reads it: its band, its fills kept.
pub const GLO30: relief_glo30::Options = relief_glo30::Options {
    keep_fills: true,
    band: true,
};

/// A page whose largest residual is under this holds nothing (m).
const NONE_BELOW_M: f32 = 0.05;

/// How the band is held to the window and blended across the fills' edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// How far the band may move the land at the window's samples (m; 0 unbounded): a soft limit, a sample moved near
    /// as far as the band says up to about half of it.
    pub cap_m: f32,
    /// How far either side of an edge between GLO-30's own heights and its fills the band is blurred (m; 0 not).
    pub seam_m: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            cap_m: 35.0,
            seam_m: 60.0,
        }
    }
}

impl Options {
    /// Neither cap nor seams: GLO-30's band as it is.
    pub const RAW: Self = Self {
        cap_m: 0.0,
        seam_m: 0.0,
    };
}

impl Options {
    /// The defaults with `spec`'s changes (`cap=35,seam=60`).
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut o = Self::default();
        for pair in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let (key, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("{pair}: not key=value"))?;
            let v = value
                .trim()
                .parse::<f32>()
                .map_err(|e| format!("{pair}: {e}"))?;
            match key.trim() {
                "cap" | "cap_m" => o.cap_m = v.max(0.0),
                "seam" | "seam_m" => o.seam_m = v.max(0.0),
                other => return Err(format!("{other}: cap or seam")),
            }
        }
        Ok(o)
    }

    /// The options the client makes its pages with: the defaults, or `NAVAL_RELIEF_COMBINED`'s.
    pub fn current() -> Self {
        static CURRENT: OnceLock<Options> = OnceLock::new();
        *CURRENT.get_or_init(|| match std::env::var("NAVAL_RELIEF_COMBINED") {
            Ok(spec) => Self::parse(&spec).unwrap_or_else(|e| {
                eprintln!("NAVAL_RELIEF_COMBINED: {e}; the defaults");
                Self::default()
            }),
            Err(_) => Self::default(),
        })
    }
}

/// The band's value held within `cap` (m) of the window: near unchanged under half of it, never past it.
#[inline]
pub fn capped(b: f32, cap: f32) -> f32 {
    if cap <= 0.0 {
        return b;
    }
    let t = b.abs() / cap;
    b / (1.0 + t * t * t * t).powf(0.25)
}

/// GLO-30's shape under a page, on the page's texels and [`EXTRA`] more each way: the mask's weight (open ground,
/// GLO-30 held and no void), the band (capped and blurred across the fills' edges), and its slopes.
pub struct Base {
    /// The theatre point of the grid's texel (0, 0)'s middle, and its texels a side.
    first: [f64; 2],
    side: usize,
    pub weight: Vec<f32>,
    pub band: Vec<f32>,
    slope: Vec<[f32; 2]>,
    /// Whether each texel's GLO-30 height is filled from another model.
    pub filled: Vec<bool>,
    /// [`Options::cap_m`].
    pub cap_m: f32,
}

impl Base {
    /// Page `index`'s base from GLO-30's `inputs` (read with [`GLO30`] and the filling mask) and the window's heights.
    pub fn new(index: [i64; 2], inputs: &Inputs, coarse: &Coarse, options: &Options) -> Self {
        let side_m = relief::page_m();
        let corner = [index[0] as f64 * side_m, index[1] as f64 * side_m];
        let window = |m: [f64; 2]| coarse.height([corner[0] + m[0], corner[1] + m[1]]);
        let fine = |m: [f64; 2]| {
            let h = inputs.height(m);
            if h.is_finite() {
                h.max(-5.0)
            } else {
                window(m)
            }
        };
        let fill = |m: [f64; 2]| {
            let [lat, lon] = inputs.map.at(m);
            inputs.fill.value_at(lat, lon)
        };
        let open = |m: [f64; 2]| {
            let [lat, lon] = inputs.map.at(m);
            relief_glo30::open(inputs.cover.class_at(lat, lon))
                && inputs.height(m).is_finite()
                && fill(m) != Some(0)
        };
        let n = PAGE_TEXELS as usize + 2 * EXTRA;
        let g = f64::from(GUTTER) + EXTRA as f64;
        let at = |i: usize, j: usize| {
            [
                (i as f64 - g + 0.5) * TEXEL_M,
                (j as f64 - g + 0.5) * TEXEL_M,
            ]
        };
        let weight = relief_glo30::weights_grid(&open, EXTRA);
        let mut band = relief_glo30::band_grid(&fine, &window, EXTRA);
        let filled: Vec<bool> = (0..n * n)
            .map(|k| fill(at(k % n, k / n)).is_some_and(relief_glo30::filled))
            .collect();
        if options.seam_m > 0.0 {
            band = across_seams(&band, &filled, n, options.seam_m);
        }
        Self {
            cap_m: options.cap_m,
            ..Self::from_grids(index, weight, band, filled)
        }
    }

    /// Page `index`'s base from its grids ([`EXTRA`] past its texels each way): the mask's weight, the band, the fills.
    pub fn from_grids(
        index: [i64; 2],
        weight: Vec<f32>,
        band: Vec<f32>,
        filled: Vec<bool>,
    ) -> Self {
        let n = PAGE_TEXELS as usize + 2 * EXTRA;
        assert!(weight.len() == n * n && band.len() == n * n && filled.len() == n * n);
        let side_m = relief::page_m();
        let corner = [index[0] as f64 * side_m, index[1] as f64 * side_m];
        let g = f64::from(GUTTER) + EXTRA as f64;
        let t = TEXEL_M as f32;
        let mut slope = vec![[0f32; 2]; n * n];
        for j in 0..n {
            let (j0, j1) = (j.saturating_sub(1), (j + 1).min(n - 1));
            for i in 0..n {
                let (i0, i1) = (i.saturating_sub(1), (i + 1).min(n - 1));
                slope[j * n + i] = [
                    (band[j * n + i1] - band[j * n + i0]) / (t * (i1 - i0) as f32),
                    (band[j1 * n + i] - band[j0 * n + i]) / (t * (j1 - j0) as f32),
                ];
            }
        }
        let m0 = (0.5 - g) * TEXEL_M;
        Self {
            first: [corner[0] + m0, corner[1] + m0],
            side: n,
            weight,
            band,
            slope,
            filled,
            cap_m: 0.0,
        }
    }

    /// At theatre point `p`: the mask's weight, the band (m) and its slope (m/m), bilinear between the texels.
    pub fn at(&self, p: [f64; 2]) -> (f32, f32, [f32; 2]) {
        let n = self.side;
        let t = [0, 1].map(|k| ((p[k] - self.first[k]) / TEXEL_M).clamp(0.0, (n - 1) as f64));
        let a = t.map(|v| (v.floor() as usize).min(n - 2));
        let [u, v] = [0, 1].map(|k| (t[k] - a[k] as f64) as f32);
        let k = a[1] * n + a[0];
        let lerp = |x: f32, y: f32, z: f32, w: f32| {
            (x * (1.0 - u) + y * u) * (1.0 - v) + (z * (1.0 - u) + w * u) * v
        };
        let of = |s: &[f32]| lerp(s[k], s[k + 1], s[k + n], s[k + n + 1]);
        let sl = |c: usize| {
            lerp(
                self.slope[k][c],
                self.slope[k + 1][c],
                self.slope[k + n][c],
                self.slope[k + n + 1][c],
            )
        };
        (of(&self.weight), of(&self.band), [sl(0), sl(1)])
    }
}

/// The band blurred near the edges between GLO-30's own heights and its fills: within `seam_m` of an edge it eases
/// to a box mean `seam_m` wide, so a step between the two models becomes a slope.
pub fn across_seams(band: &[f32], filled: &[bool], n: usize, seam_m: f32) -> Vec<f32> {
    // The distance to the nearest edge (a texel whose neighbour is on the other side), by an 8-neighbour chamfer.
    let far = 1e6f32;
    let mut d = vec![far; n * n];
    let mut any = false;
    for j in 0..n {
        for i in 0..n {
            let k = j * n + i;
            let f = filled[k];
            if (i + 1 < n && filled[k + 1] != f) || (j + 1 < n && filled[k + n] != f) {
                d[k] = 0.0;
                any = true;
            }
        }
    }
    if !any {
        return band.to_vec();
    }
    chamfer(&mut d, n);
    let r = ((seam_m / TEXEL_M as f32) / 2.0).round().max(1.0) as usize;
    let blurred = box_blur(band, n, r);
    band.iter()
        .zip(&blurred)
        .zip(&d)
        .map(|((&b, &s), &d)| {
            let t = (d / seam_m).clamp(0.0, 1.0);
            let keep = t * t * (3.0 - 2.0 * t);
            s + keep * (b - s)
        })
        .collect()
}

/// An 8-neighbour chamfer distance (m) over an `n`² grid of texels, from the texels at 0.
pub fn chamfer(d: &mut [f32], n: usize) {
    let (a, b) = (TEXEL_M as f32, TEXEL_M as f32 * std::f32::consts::SQRT_2);
    for j in 0..n {
        for i in 0..n {
            let k = j * n + i;
            let mut v = d[k];
            if i > 0 {
                v = v.min(d[k - 1] + a);
            }
            if j > 0 {
                v = v.min(d[k - n] + a);
                if i > 0 {
                    v = v.min(d[k - n - 1] + b);
                }
                if i + 1 < n {
                    v = v.min(d[k - n + 1] + b);
                }
            }
            d[k] = v;
        }
    }
    for j in (0..n).rev() {
        for i in (0..n).rev() {
            let k = j * n + i;
            let mut v = d[k];
            if i + 1 < n {
                v = v.min(d[k + 1] + a);
            }
            if j + 1 < n {
                v = v.min(d[k + n] + a);
                if i + 1 < n {
                    v = v.min(d[k + n + 1] + b);
                }
                if i > 0 {
                    v = v.min(d[k + n - 1] + b);
                }
            }
            d[k] = v;
        }
    }
}

/// A box mean `2 r + 1` texels wide each way over an `n`² grid (the edges clamped).
fn box_blur(v: &[f32], n: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], stride: usize, step: usize| {
        let mut out = vec![0f32; n * n];
        for line in 0..n {
            let at =
                |k: i64| f64::from(src[line * stride + (k.clamp(0, n as i64 - 1) as usize) * step]);
            let mut sum: f64 = (-(r as i64)..=r as i64).map(at).sum();
            for k in 0..n as i64 {
                out[line * stride + k as usize * step] = (sum / (2 * r + 1) as f64) as f32;
                sum += at(k + r as i64 + 1) - at(k - r as i64);
            }
        }
        out
    };
    let rows = pass(v, n, 1);
    pass(&rows, 1, n)
}

/// Holds page `index`'s samples on `lattice` within `base.cap_m` of the window's heights: at each sample the excess of
/// the band's move over its capped value ([`capped`]), taken off the texels (the page's and `extra` more each way) as
/// the bilinear surface through the samples. The band's value at a sample is read from `base` (a function of the
/// theatre's metres), so two pages agree in their gutters.
pub fn hold_samples(
    texels: &mut [f32],
    lattice: &Lattice,
    index: [i64; 2],
    base: &Base,
    extra: usize,
) {
    let cap = base.cap_m;
    if cap <= 0.0 {
        return;
    }
    let (lo, hi) = relief::reach(lattice);
    let wide = extra as f64 * TEXEL_M;
    let (lo, hi) = (lo - wide, hi + wide);
    let side = relief::page_m();
    let corner = [index[0] as f64 * side, index[1] as f64 * side];
    let first = [0, 1].map(|k| ((corner[k] + lo - lattice.node[k]) / lattice.cell).floor() as i64);
    let last = [0, 1].map(|k| ((corner[k] + hi - lattice.node[k]) / lattice.cell).ceil() as i64);
    let count = [0, 1].map(|k| (last[k] - first[k] + 1) as usize);
    let mut excess = vec![0f32; count[0] * count[1]];
    let mut any = false;
    for j in 0..count[1] {
        for i in 0..count[0] {
            let p = [
                lattice.node[0] + (first[0] + i as i64) as f64 * lattice.cell,
                lattice.node[1] + (first[1] + j as i64) as f64 * lattice.cell,
            ];
            let (w, b, _) = base.at(p);
            let moved = w * b;
            let e = moved - capped(moved, cap);
            any |= e != 0.0;
            excess[j * count[0] + i] = e;
        }
    }
    if !any {
        return;
    }
    let n = PAGE_TEXELS as usize + 2 * extra;
    let g = f64::from(GUTTER) + extra as f64;
    for j in 0..n {
        for i in 0..n {
            let p = [
                corner[0] + (i as f64 - g + 0.5) * TEXEL_M,
                corner[1] + (j as f64 - g + 0.5) * TEXEL_M,
            ];
            let t = [0, 1].map(|k| (p[k] - lattice.node[k]) / lattice.cell - first[k] as f64);
            let (a, b) = (t[0].floor(), t[1].floor());
            let (u, v) = ((t[0] - a) as f32, (t[1] - b) as f32);
            let (a, b) = (
                (a as usize).min(count[0] - 2),
                (b as usize).min(count[1] - 2),
            );
            let at = |i: usize, j: usize| excess[j * count[0] + i];
            texels[j * n + i] -= (at(a, b) * (1.0 - u) + at(a + 1, b) * u) * (1.0 - v)
                + (at(a, b + 1) * (1.0 - u) + at(a + 1, b + 1) * u) * v;
        }
    }
}

/// Page `index`'s residuals on `lattice`: the synthetic over GLO-30's base where `inputs` are held (its samples held
/// within the base's cap), the synthetic alone where not. Neither held to the window nor off the water ([`held`]).
pub fn residuals(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    base: Option<&Base>,
    coarse: &Coarse,
    params: &Params,
) -> Vec<f32> {
    residuals_grid(geo, lattice, index, base, coarse, params, 0)
}

/// [`residuals`] over the page's texels and `extra` more each way.
pub fn residuals_grid(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    base: Option<&Base>,
    coarse: &Coarse,
    params: &Params,
    extra: usize,
) -> Vec<f32> {
    let height = |p: [f64; 2]| coarse.height(p);
    let style = relief_synth::Style::of(geo, index, params);
    let under = |p: [f64; 2]| base.map_or((0.0, 0.0, [0.0; 2]), |b| b.at(p));
    let mut texels = relief_synth::residuals_on(
        lattice,
        index,
        &height,
        params,
        &style,
        base.is_some().then_some(&under as relief_synth::Under),
        extra,
    );
    if let Some(base) = base {
        hold_samples(&mut texels, lattice, index, base, extra);
    }
    texels
}

/// Page `index`'s residuals as [`make_page`] takes them: [`residuals`] made [`relief_limit::MARGIN`] past its texels,
/// held to the window and off `water` (`relief_limit::hold`).
pub fn held(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    base: Option<&Base>,
    coarse: &Coarse,
    params: &Params,
    water: &Water,
) -> Vec<f32> {
    let grid = residuals_grid(
        geo,
        lattice,
        index,
        base,
        coarse,
        params,
        relief_limit::MARGIN,
    );
    let window = |p: [f64; 2]| coarse.height(p);
    relief_limit::hold(&grid, &relief_limit::weights(index, water, &window))
}

/// Page `index` about `geo` on `lattice`: GLO-30's band and the synthetic's fine detail where GLO-30 is held
/// (`inputs`), the synthetic alone where not, held to the window and off `water`; none where it holds under 5 cm.
pub fn make_page(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    inputs: Option<&Inputs>,
    coarse: &Coarse,
    water: &Water,
) -> Option<ReliefPage> {
    let base = inputs.map(|i| Base::new(index, i, coarse, &Options::current()));
    let texels = held(
        geo,
        lattice,
        index,
        base.as_ref(),
        coarse,
        &Params::current(),
        water,
    );
    let mut page = ReliefPage::from_residual(&texels, TEXEL_M as f32);
    page.real = false;
    (page.peak >= NONE_BELOW_M).then_some(page)
}

#[cfg(test)]
#[path = "relief_combined_tests.rs"]
mod tests;
