//! Made relief held to the simulation's surface and off the water (native/docs/terrain/ground.md, "Fine relief near
//! the camera"): what a synthetic, GLO-30 or combined page may do to the land that a finer model's page is trusted to.
//!
//! - **The allowance.** The simulation flies its aircraft over the release's heights, a bilinear surface on its 2.25
//!   arc-second lattice (`naval_sim::land::ReleaseLand::height`), keeping 60 m and their pull-out's height above the
//!   highest ground along their track (`aviation::aircraft_terrain`'s `CLEARANCE_M`). The window is that surface
//!   resampled, and a page is a residual over the window's bilinear surface, so a made page may stand at most
//!   [`RAISE_M`] above it at any texel ([`hold`]). Where it would stand higher over a broad stretch (a crest the band
//!   raises), the stretch is lowered whole by a smooth surface, its shape kept; what is left over (the narrow lips and a
//!   crest's top) is eased under the allowance over its last [`KNEE_M`]. Below the window it may carve as before.
//! - **Off the water.** No made detail on WorldCover's water nor on the rivers, canals, riverbanks and docks the
//!   features draw, nor on OpenStreetMap's lakes and ponds ([`Water`]): the detail comes in over [`WATER_FADE_M`]
//!   from them, over [`STREAM_FADE_M`] from a stream, drain or ditch. And all of it is scaled down near the sea, from
//!   none at its level to all of it [`SHORE_M`] up, so a fjord's foot keeps the window's slope.
//! - **Made on a wider grid.** The smoothing reaches [`MARGIN`] texels past a page's own (with its gutters), so a page
//!   makes its residual that much further and two pages agree where they meet.

use naval_earth::geo::LatLon;
use naval_osm::Geometry;
use naval_terrain::relief::{GUTTER, PAGE_TEXELS};
use naval_terrain::surface_maps::cover;

use super::features;
use super::osm;
use super::relief::{self, TEXEL_M};
use super::relief_glo30::GeoMap;
use super::surface::{self, CoverPatch};

/// How far a made page may stand above the window's surface at any texel (m).
pub const RAISE_M: f32 = 4.0;
/// The last metres under [`RAISE_M`] over which what stands higher is eased in (m): below `RAISE_M - KNEE_M` the land
/// is as made.
pub const KNEE_M: f32 = 1.5;
/// An excess over [`RAISE_M`] covering under half of the `2 NARROW + 1` texels about a texel is eased, not lowered: a
/// lip or a spike lowered whole would dig the land about it.
pub const NARROW: usize = 6;
/// How far a broad excess is spread before it is smoothed (texels, each way): the reach of a lowered crest's shape.
pub const SPREAD: usize = 6;
/// The radius (texels) of each of the two box blurs that smooth the spread excess: together they reach [`SPREAD`], so
/// the smooth surface is everywhere at least the broad excess it is taken from.
const BLUR: usize = SPREAD / 2;
/// Texels past a page's grid (with its gutters) a made page's residual is made over, for [`hold`]'s reach.
pub const MARGIN: usize = NARROW + SPREAD + 2 * BLUR;
/// The made detail comes in from none on the water to all of it this far from it (m).
pub const WATER_FADE_M: f32 = 50.0;
/// The same from a stream, a drain or a ditch (a waterway drawn narrower than [`NARROW_WATER_M`]).
pub const STREAM_FADE_M: f32 = 15.0;
/// A waterway drawn with a half width under this (m) is a stream's.
pub const NARROW_WATER_M: f32 = 3.0;
/// The made detail comes in from none at the sea's level to all of it this high (m).
pub const SHORE_M: f32 = 50.0;

/// Texels a side of a grid `extra` texels past a page's own.
pub const fn side(extra: usize) -> usize {
    PAGE_TEXELS as usize + 2 * extra
}

/// The theatre point of texel (i, j)'s middle on page `index`'s grid `extra` texels past its own.
pub fn texel_at(index: [i64; 2], extra: usize, i: usize, j: usize) -> [f64; 2] {
    let side_m = relief::page_m();
    let g = f64::from(GUTTER) + extra as f64;
    [
        index[0] as f64 * side_m + (i as f64 - g + 0.5) * TEXEL_M,
        index[1] as f64 * side_m + (j as f64 - g + 0.5) * TEXEL_M,
    ]
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The water under a made page and about it: WorldCover's classes, and OpenStreetMap's waterways (each with its drawn
/// half width) and water areas (rings, the even-odd rule), in the theatre's metres.
pub struct Water {
    index: [i64; 2],
    map: GeoMap,
    cover: Option<CoverPatch>,
    pub lines: Vec<(Vec<[f64; 2]>, f32)>,
    pub areas: Vec<Vec<Vec<[f64; 2]>>>,
}

/// Texels past [`MARGIN`] the water is read for, so the fade sees water beyond the grid.
const PAD: usize = (WATER_FADE_M as f64 / TEXEL_M) as usize + 2;

impl Water {
    /// No water known about page `index`.
    pub fn none(geo: LatLon, index: [i64; 2]) -> Self {
        Self {
            index,
            map: GeoMap::new(geo, index),
            cover: None,
            lines: Vec::new(),
            areas: Vec::new(),
        }
    }

    /// The water about page `index`: WorldCover's classes and OpenStreetMap's tiles from the caches, the network where
    /// they lack them; whatever cannot be read (offline, the tiles off) is left out.
    pub fn load(geo: LatLon, index: [i64; 2]) -> Self {
        let mut water = Self::none(geo, index);
        let reach = (f64::from(GUTTER) + (MARGIN + PAD) as f64) * TEXEL_M;
        let (lo, hi) = (-reach, relief::page_m() + reach);
        let [s, n, w, e] = water.map.bounds(lo, hi);
        let pad = 1.0 / 3600.0;
        match surface::cover_patch(s - pad, n + pad, w - pad, e + pad) {
            Ok(cover) => water.cover = Some(cover),
            Err(err) => surface::log_once(
                "relief-water-cover",
                &format!("the relief's water cover: {err}"),
            ),
        }
        let side_m = relief::page_m();
        let corner = [index[0] as f64 * side_m, index[1] as f64 * side_m];
        let (min, max) = (
            [corner[0] + lo, corner[1] + lo],
            [corner[0] + hi, corner[1] + hi],
        );
        let ids = osm::tiles_over(geo, min, max, osm::ZOOM);
        let area = match osm::load(geo, &ids) {
            Ok(area) => area,
            Err(_) => osm::Area {
                tiles: ids.iter().filter_map(|&id| osm::tile(geo, id)).collect(),
                missing: Vec::new(),
            },
        };
        water.add_tiles(&area, min, max);
        water
    }

    /// The waterways and water areas of `area` reaching the rectangle from `min` to `max`.
    pub fn add_tiles(&mut self, area: &osm::Area, min: [f64; 2], max: [f64; 2]) {
        for w in area.waterway_in(min, max) {
            if let (Some(half), Geometry::Lines(lines)) = (features::waterway_style(w), &w.geometry)
            {
                self.lines.extend(lines.iter().map(|l| (l.clone(), half)));
            }
        }
        for w in area.water_in(min, max) {
            let still = matches!(w.class.as_str(), "lake" | "pond") && !w.intermittent;
            if !(features::water_area_drawn(&w.class, w.intermittent) || still) {
                continue;
            }
            if let Geometry::Polygons(polys) = &w.geometry {
                self.areas.extend(polys.iter().map(|p| {
                    std::iter::once(p.exterior.clone())
                        .chain(p.holes.iter().cloned())
                        .collect()
                }));
            }
        }
    }

    /// Whether each texel of the page's grid `extra` texels past its own is water, and whether it is a stream's (a
    /// waterway narrower than [`NARROW_WATER_M`]).
    pub fn grid(&self, extra: usize) -> (Vec<bool>, Vec<bool>) {
        let n = side(extra);
        let mut wet = vec![false; n * n];
        let mut stream = vec![false; n * n];
        let first = texel_at(self.index, extra, 0, 0);
        if let Some(cover) = &self.cover {
            let side_m = relief::page_m();
            let corner = [self.index[0] as f64 * side_m, self.index[1] as f64 * side_m];
            // WorldCover's 10 m pixels read every other texel each way (8 m), at the middle of each 2 x 2.
            for j in (0..n).step_by(2) {
                for i in (0..n).step_by(2) {
                    let p = texel_at(self.index, extra, i, j);
                    let m = [
                        p[0] + 0.5 * TEXEL_M - corner[0],
                        p[1] + 0.5 * TEXEL_M - corner[1],
                    ];
                    let [lat, lon] = self.map.at(m);
                    if cover::today(cover.class_at(lat, lon)) == cover::WATER {
                        for (di, dj) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                            if i + di < n && j + dj < n {
                                wet[(j + dj) * n + i + di] = true;
                            }
                        }
                    }
                }
            }
        }
        // Texel coordinates of a theatre point: texel (i, j)'s middle at (i, j).
        let local = |p: [f64; 2]| [(p[0] - first[0]) / TEXEL_M, (p[1] - first[1]) / TEXEL_M];
        for (line, half) in &self.lines {
            let marks = if *half < NARROW_WATER_M {
                &mut stream
            } else {
                &mut wet
            };
            // A thin stream still marks the texels it runs through.
            let reach = f64::from(*half) / TEXEL_M + 0.5;
            for pair in line.windows(2) {
                let (a, b) = (local(pair[0]), local(pair[1]));
                let lo = [0, 1].map(|k| (a[k].min(b[k]) - reach).floor().max(0.0) as usize);
                let hi =
                    [0, 1].map(|k| ((a[k].max(b[k]) + reach).ceil().max(0.0) as usize).min(n - 1));
                let d = [b[0] - a[0], b[1] - a[1]];
                let len2 = (d[0] * d[0] + d[1] * d[1]).max(1e-12);
                for j in lo[1]..=hi[1] {
                    for i in lo[0]..=hi[0] {
                        let q = [i as f64 - a[0], j as f64 - a[1]];
                        let t = ((q[0] * d[0] + q[1] * d[1]) / len2).clamp(0.0, 1.0);
                        let (x, z) = (q[0] - t * d[0], q[1] - t * d[1]);
                        if x * x + z * z <= reach * reach {
                            marks[j * n + i] = true;
                        }
                    }
                }
            }
        }
        // The areas by scanline, each polygon's rings by the even-odd rule.
        for rings in &self.areas {
            let rings: Vec<Vec<[f64; 2]>> = rings
                .iter()
                .map(|r| r.iter().map(|&p| local(p)).collect())
                .collect();
            let (zlo, zhi) = rings
                .iter()
                .flatten()
                .fold((f64::MAX, f64::MIN), |(lo, hi), p| {
                    (lo.min(p[1]), hi.max(p[1]))
                });
            let (j0, j1) = (
                zlo.ceil().max(0.0) as usize,
                (zhi.floor().max(-1.0) as i64).min(n as i64 - 1),
            );
            let mut xs = Vec::new();
            for j in j0 as i64..=j1 {
                let z = j as f64;
                xs.clear();
                for ring in &rings {
                    for k in 0..ring.len() {
                        let (p, q) = (ring[k], ring[(k + 1) % ring.len()]);
                        if (p[1] <= z) != (q[1] <= z) {
                            xs.push(p[0] + (z - p[1]) / (q[1] - p[1]) * (q[0] - p[0]));
                        }
                    }
                }
                xs.sort_by(f64::total_cmp);
                for span in xs.as_chunks::<2>().0 {
                    let i0 = span[0].ceil().max(0.0) as usize;
                    let i1 = span[1].floor().min(n as f64 - 1.0);
                    if i1 < 0.0 {
                        continue;
                    }
                    for i in i0..=i1 as usize {
                        wet[j as usize * n + i] = true;
                    }
                }
            }
        }
        (wet, stream)
    }
}

/// The made detail's share at each texel of page `index`'s grid [`MARGIN`] past its own: none on the water and
/// coming in over [`WATER_FADE_M`] from it ([`STREAM_FADE_M`] from a stream; `water`), times none at the sea's level
/// coming in over [`SHORE_M`] of the window's height (`window`, at a theatre point).
pub fn weights(index: [i64; 2], water: &Water, window: &dyn Fn([f64; 2]) -> f32) -> Vec<f32> {
    let n = side(MARGIN);
    let wide = side(MARGIN + PAD);
    let (wet, stream) = water.grid(MARGIN + PAD);
    let distance = |marks: &[bool]| {
        marks.iter().any(|&w| w).then(|| {
            let mut d: Vec<f32> = marks.iter().map(|&w| if w { 0.0 } else { 1e6 }).collect();
            super::relief_combined::chamfer(&mut d, wide);
            d
        })
    };
    let (wet, stream) = (distance(&wet), distance(&stream));
    // The window's height every other texel each way (a quarter of the reads), bilinear between: the window is itself
    // bilinear on its 40 m lattice.
    let half = n / 2 + 1;
    let mut coarse = vec![0f32; half * half];
    for b in 0..half {
        for a in 0..half {
            coarse[b * half + a] = window(texel_at(index, MARGIN, 2 * a, 2 * b));
        }
    }
    let mut out = vec![0f32; n * n];
    for j in 0..n {
        let (b, v) = (j / 2, 0.5 * (j % 2) as f32);
        for i in 0..n {
            let (a, u) = (i / 2, 0.5 * (i % 2) as f32);
            let at = |a: usize, b: usize| coarse[b.min(half - 1) * half + a.min(half - 1)];
            let h = (at(a, b) * (1.0 - u) + at(a + 1, b) * u) * (1.0 - v)
                + (at(a, b + 1) * (1.0 - u) + at(a + 1, b + 1) * u) * v;
            let k = (j + PAD) * wide + i + PAD;
            let mut share = smoothstep(0.0, SHORE_M, h);
            if let Some(d) = &wet {
                share *= smoothstep(0.0, WATER_FADE_M, d[k]);
            }
            if let Some(d) = &stream {
                share *= smoothstep(0.0, STREAM_FADE_M, d[k]);
            }
            out[j * n + i] = share;
        }
    }
    out
}

/// A made page's residual over its grid [`MARGIN`] past its own (`grid`) held, then scaled by `weight` (the water's
/// and the shore's, [`weights`]): the page's own texels (with its gutters), at most [`RAISE_M`] above the window at
/// any texel. The excess over the allowance where it is broad (most of the texels within [`NARROW`] over it) is spread
/// [`SPREAD`] texels each way, smoothed within that and taken off, so a broad stretch standing too high is lowered
/// whole, its shape kept; what then stands above `RAISE_M - KNEE_M` (a lip, a spike) is eased under `RAISE_M`
/// ([`ease`]).
pub fn hold(grid: &[f32], weight: &[f32]) -> Vec<f32> {
    let s = side(MARGIN);
    assert!(grid.len() == s * s && weight.len() == s * s);
    let excess: Vec<f32> = grid.iter().map(|&r| (r - RAISE_M).max(0.0)).collect();
    let lowered = excess.iter().any(|&e| e > 0.0).then(|| {
        let over: Vec<f32> = excess
            .iter()
            .map(|&e| f32::from(u8::from(e > 0.0)))
            .collect();
        let share = blur(&over, s, NARROW);
        let broad: Vec<f32> = excess
            .iter()
            .zip(&share)
            .map(|(&e, &b)| e * smoothstep(0.4, 0.8, b))
            .collect();
        broad.iter().any(|&e| e > 0.0).then(|| {
            let mut b = dilate(&broad, s, SPREAD);
            for _ in 0..2 {
                b = blur(&b, s, BLUR);
            }
            b
        })
    });
    let lowered = lowered.flatten();
    let n = PAGE_TEXELS as usize;
    let mut out = vec![0f32; n * n];
    for j in 0..n {
        for i in 0..n {
            let k = (j + MARGIN) * s + i + MARGIN;
            let r = grid[k] - lowered.as_ref().map_or(0.0, |l| l[k]);
            out[j * n + i] = weight[k] * ease(r);
        }
    }
    out
}

/// `r` as it is up to `RAISE_M - KNEE_M`, eased above it toward `RAISE_M` and never reaching it (its slope continuous).
#[inline]
pub fn ease(r: f32) -> f32 {
    let knee = RAISE_M - KNEE_M;
    if r <= knee {
        r
    } else {
        RAISE_M - KNEE_M * (-(r - knee) / KNEE_M).exp()
    }
}

/// The largest of `v` within `r` texels each way (a square) over an `n`² grid, the edges clamped: van Herk's and Gil
/// and Werman's runs, three comparisons a texel whatever `r`. A line all 0 stays 0.
fn dilate(v: &[f32], n: usize, r: usize) -> Vec<f32> {
    let ident = f32::NEG_INFINITY;
    let pick = f32::max;
    let w = 2 * r + 1;
    let m = n + 2 * r;
    let (mut line, mut g, mut h) = (vec![ident; m], vec![ident; m], vec![ident; m]);
    let mut pass = |src: &[f32], stride: usize, step: usize| {
        let mut out = vec![0f32; n * n];
        for l in 0..n {
            let mut any = false;
            for k in 0..n {
                let x = src[l * stride + k * step];
                any |= x != 0.0;
                line[r + k] = x;
            }
            if !any {
                continue;
            }
            for k in 0..m {
                g[k] = if k % w == 0 {
                    line[k]
                } else {
                    pick(g[k - 1], line[k])
                };
            }
            for k in (0..m).rev() {
                h[k] = if k % w == w - 1 || k == m - 1 {
                    line[k]
                } else {
                    pick(h[k + 1], line[k])
                };
            }
            for k in 0..n {
                out[l * stride + k * step] = pick(h[k], g[k + 2 * r]);
            }
        }
        out
    };
    let rows = pass(v, n, 1);
    pass(&rows, 1, n)
}

/// A box mean `2 r + 1` texels wide each way over an `n`² grid (the edges clamped), summed in f64.
fn blur(v: &[f32], n: usize, r: usize) -> Vec<f32> {
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

#[cfg(test)]
#[path = "relief_limit_tests.rs"]
mod tests;
