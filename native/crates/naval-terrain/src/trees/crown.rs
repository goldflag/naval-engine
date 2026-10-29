//! The light inside the trees' crowns (native/docs/terrain/trees.md, "Grown trees"). Each species' leaves become a
//! density
//! over a grid about its model, from its design's clumps (a soft ball of leaves each, denser where balls overlap, a
//! clump of bare twigs a fraction as dense), and from that density, for every point of its mesh, the share of the sky
//! it sees through the crown and which way more of it lies (its slope). A leaf deep in a crown is darker
//! than one at its rim and a trunk under its crown darker than the ground beside it: the leaves take the light on its
//! way in, about exponentially with their depth.
//!
//! The sky is gathered along [`SKY_RAYS`] directions spread over the upper hemisphere by its cosine: each direction's
//! optical depth from every node to the grid's edge is swept across the grid a slice at a time, each node from the
//! slice beyond it toward the sky, so the whole grid costs one interpolated read a node a direction; a point between
//! nodes takes their transmittances trilinearly.

use glam::Vec3;

use crate::species::{KIND_BARK, KIND_TWIGS, Model};

/// Directions the sky is gathered along.
pub const SKY_RAYS: usize = 16;
/// Nodes along the grid's longest side (its cells about cubic), and the fewest along any.
const LONGEST_NODES: f32 = 32.0;
const LEAST_NODES: usize = 6;
/// A clump's leaves take the light as their area does: its sprays' cards' area, the share of a card its sprite covers
/// with leaves (or bare twigs), half of that facing any one way (leaves facing every way), over the clump's volume.
/// A bare crown lets most of the sky through (a winter wood passes half to two thirds of it straight down): its twigs
/// take about a tenth of the light a metre (at 0.12, half a metre's worth, its limbs under the twigs drew dark).
const LEAF_COVER: f32 = 0.4;
const TWIG_COVER: f32 = 0.03;
const FACING_SHARE: f32 = 0.5;
/// The light takes the leaves the eye sees: a crown's cards as drawn, whose gaps show its inner leaves, at their share
/// (at twice it, three quarters of the rainforest's sprays saw under a tenth of the sky, and the inner leaves an eye
/// saw through the gaps drew black; at 3.5 the oak's heart three metres in saw none).
const CROWN_DENSITY: f32 = 1.0;
/// A clump with no sprays of its own (a palm's frond, a blade of grass: one ribbon down a run of clumps) takes the
/// light a closed crown's metre of leaves takes (a leaf area density of about two square metres a cubic metre), less
/// where its rim is open.
const RIBBON_EXTINCTION_PER_M: f32 = 1.0;
const OPEN_RIM_LESS: f32 = 0.6;
/// A clump buried in its crown carries no sprays in the mesh (`tree_meshes::BURIED_LEAST`): it stands for the
/// crown's sparser heart, half as dense.
const BURIED_SHARE: f32 = 0.5;
/// A clump's leaves are dense to this share of its radius and thin to nothing a little past it.
const RIM_INNER: f32 = 0.8;
const RIM_OUTER: f32 = 1.05;

/// The directions the sky is gathered along: a Fibonacci spiral over the unit disc lifted onto the upper hemisphere,
/// so they spread by the cosine from the zenith to about 10 degrees above the horizon (the land's own open sky stands
/// for what lies lower).
pub fn sky_rays() -> [Vec3; SKY_RAYS] {
    std::array::from_fn(|i| {
        let r = ((i as f32 + 0.5) / SKY_RAYS as f32).sqrt();
        let azimuth = i as f32 * 2.399_963;
        Vec3::new(r * azimuth.cos(), (1.0 - r * r).sqrt(), r * azimuth.sin())
    })
}

/// One species' crown as light reaches into it: the transmittance toward each of [`sky_rays`] at every node of a grid
/// about its model (model space: the foot at 0, the top at 1).
pub struct CrownLight {
    origin: Vec3,
    cell: Vec3,
    dims: [usize; 3],
    rays: [Vec3; SKY_RAYS],
    fit: [Vec3; SKY_RAYS],
    through: Vec<[f32; SKY_RAYS]>,
}

/// Each of the model's shapes' extinction (per model unit; bark none): its sprays' leaf area over its volume, or a
/// ribbon's clump's by its openness; a buried clump's halved.
fn clump_extinction(model: &Model, height: f32) -> Vec<f32> {
    let mut area = vec![0.0f32; model.primitives.len()];
    for spray in &model.sprays {
        area[spray.clump as usize] += spray.length * spray.width;
    }
    model
        .primitives
        .iter()
        .zip(&area)
        .map(|(p, &a)| {
            if p.b[3] == KIND_BARK {
                return 0.0;
            }
            let k = if a > 0.0 {
                let cover = if p.b[3] == KIND_TWIGS {
                    TWIG_COVER
                } else {
                    LEAF_COVER
                };
                let r = p.a[3] * 0.5 * (RIM_INNER + RIM_OUTER);
                CROWN_DENSITY * FACING_SHARE * cover * a
                    / (4.0 / 3.0 * std::f32::consts::PI * r * r * r)
            } else {
                RIBBON_EXTINCTION_PER_M * (1.0 - OPEN_RIM_LESS * p.c[3]) * height
            };
            if p.d[0] < crate::tree_meshes::BURIED_LEAST {
                k * BURIED_SHARE
            } else {
                k
            }
        })
        .collect()
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl CrownLight {
    pub fn new(model: &Model) -> Self {
        let height = model.species.height_m();
        // The grid's box: the model's shapes and its reach, a cell clear of them each way.
        let mut lo = Vec3::new(-model.radius, -0.02, -model.radius);
        let mut hi = Vec3::new(model.radius, 1.0, model.radius);
        for p in &model.primitives {
            let r = Vec3::splat(p.a[3]);
            let a = Vec3::new(p.a[0], p.a[1], p.a[2]);
            lo = lo.min(a - r);
            hi = hi.max(a + r);
            if p.b[3] == KIND_BARK {
                let b = Vec3::new(p.b[0], p.b[1], p.b[2]);
                lo = lo.min(b - r);
                hi = hi.max(b + r);
            }
        }
        let size = (hi - lo).max_element() / LONGEST_NODES;
        lo -= Vec3::splat(size);
        hi += Vec3::splat(size);
        let extent = hi - lo;
        let dims = [0, 1, 2].map(|i| ((extent[i] / size).ceil() as usize + 1).max(LEAST_NODES));
        let cell = Vec3::new(
            extent.x / (dims[0] - 1) as f32,
            extent.y / (dims[1] - 1) as f32,
            extent.z / (dims[2] - 1) as f32,
        );
        let rays = sky_rays();
        let mut light = Self {
            origin: lo,
            cell,
            dims,
            rays,
            fit: slope_fit(&rays),
            through: Vec::new(),
        };
        let sigma = light.density(model, height);
        light.through = vec![[1.0; SKY_RAYS]; sigma.len()];
        let mut tau = vec![0.0f32; sigma.len()];
        for i in 0..SKY_RAYS {
            light.sweep(&sigma, light.rays[i], &mut tau);
            for (node, t) in light.through.iter_mut().zip(&tau) {
                node[i] = (-t).exp();
            }
        }
        light
    }

    fn index(&self, at: [usize; 3]) -> usize {
        at[0] + self.dims[0] * (at[1] + self.dims[1] * at[2])
    }

    /// The leaves' extinction at every node (per model unit): each clump a soft ball of its leaves, overlapping balls'
    /// leaves adding up.
    fn density(&self, model: &Model, height: f32) -> Vec<f32> {
        let n = self.dims[0] * self.dims[1] * self.dims[2];
        let mut dense = vec![0.0f32; n];
        let least = self.cell.max_element() * 0.75;
        let extinction = clump_extinction(model, height);
        for (p, &k) in model.primitives.iter().zip(&extinction) {
            if p.b[3] == KIND_BARK {
                continue;
            }
            let centre = Vec3::new(p.a[0], p.a[1], p.a[2]);
            let mut k = k;
            // A clump smaller than the grid's cells spreads over a cell, as thin as its leaves are few.
            let r = p.a[3].max(least);
            k *= (p.a[3] / r).powi(3);
            let reach = Vec3::splat(r * RIM_OUTER);
            let first = ((centre - reach - self.origin) / self.cell)
                .floor()
                .max(Vec3::ZERO);
            let last = ((centre + reach - self.origin) / self.cell).ceil();
            let last = [0, 1, 2].map(|i| (last[i] as usize).min(self.dims[i] - 1));
            for z in first.z as usize..=last[2] {
                for y in first.y as usize..=last[1] {
                    for x in first.x as usize..=last[0] {
                        let at = self.origin + Vec3::new(x as f32, y as f32, z as f32) * self.cell;
                        let w = 1.0 - smoothstep(r * RIM_INNER, r * RIM_OUTER, at.distance(centre));
                        if w > 0.0 {
                            dense[self.index([x, y, z])] += w * k;
                        }
                    }
                }
            }
        }
        dense
    }

    /// Each node's optical depth along `ray` to the grid's edge, into `tau`: one cell a step along the axis the ray
    /// crosses cells fastest on, from the slice beyond (read between its nodes) back toward the ray's start.
    fn sweep(&self, sigma: &[f32], ray: Vec3, tau: &mut [f32]) {
        let per_cell = ray / self.cell;
        let a = (0..3)
            .max_by(|&i, &j| per_cell[i].abs().total_cmp(&per_cell[j].abs()))
            .unwrap();
        let (b, c) = ((a + 1) % 3, (a + 2) % 3);
        let step = per_cell / per_cell[a].abs();
        let length = 1.0 / per_cell[a].abs();
        let forward = step[a] > 0.0;
        let (nb, nc) = (self.dims[b], self.dims[c]);
        let at = |s: usize, jb: usize, jc: usize| {
            let mut p = [0usize; 3];
            p[a] = s;
            p[b] = jb;
            p[c] = jc;
            self.index(p)
        };
        for k in 0..self.dims[a] {
            let s = if forward { self.dims[a] - 1 - k } else { k };
            let next = if forward { s + 1 } else { s.wrapping_sub(1) };
            let inside = next < self.dims[a];
            for jc in 0..nc {
                for jb in 0..nb {
                    let here = at(s, jb, jc);
                    let (sigma_next, tau_next) = if inside {
                        let fb = jb as f32 + step[b];
                        let fc = jc as f32 + step[c];
                        if fb < 0.0 || fc < 0.0 || fb > (nb - 1) as f32 || fc > (nc - 1) as f32 {
                            (0.0, 0.0)
                        } else {
                            let ib = (fb as usize).min(nb - 2);
                            let ic = (fc as usize).min(nc - 2);
                            let (ub, uc) = (fb - ib as f32, fc - ic as f32);
                            let corners = [
                                (at(next, ib, ic), (1.0 - ub) * (1.0 - uc)),
                                (at(next, ib + 1, ic), ub * (1.0 - uc)),
                                (at(next, ib, ic + 1), (1.0 - ub) * uc),
                                (at(next, ib + 1, ic + 1), ub * uc),
                            ];
                            corners.iter().fold((0.0, 0.0), |(s0, t0), &(i, w)| {
                                (s0 + sigma[i] * w, t0 + tau[i] * w)
                            })
                        }
                    } else {
                        (0.0, 0.0)
                    };
                    tau[here] = 0.5 * length * (sigma[here] + sigma_next) + tau_next;
                }
            }
        }
    }

    /// The transmittance toward each ray at `p`, from the nodes about it.
    fn through_at(&self, p: Vec3) -> [f32; SKY_RAYS] {
        let top = Vec3::new(
            (self.dims[0] - 1) as f32,
            (self.dims[1] - 1) as f32,
            (self.dims[2] - 1) as f32,
        );
        let g = ((p - self.origin) / self.cell).clamp(Vec3::ZERO, top);
        let i = [0, 1, 2].map(|k| (g[k] as usize).min(self.dims[k] - 2));
        let f = g - Vec3::new(i[0] as f32, i[1] as f32, i[2] as f32);
        let mut out = [0.0f32; SKY_RAYS];
        for corner in 0..8 {
            let d = [corner & 1, (corner >> 1) & 1, corner >> 2];
            let w = [0, 1, 2]
                .map(|k| if d[k] == 1 { f[k] } else { 1.0 - f[k] })
                .iter()
                .product::<f32>();
            if w <= 0.0 {
                continue;
            }
            let node = &self.through[self.index([i[0] + d[0], i[1] + d[1], i[2] + d[2]])];
            for (o, t) in out.iter_mut().zip(node) {
                *o += w * t;
            }
        }
        out
    }

    /// The share of the sky `p` sees through the crown (0-1, by the cosine) and its slope: the open sky toward a
    /// direction `d` of the upper hemisphere is about `sky + slope · (d - SKY_MEAN)`, the least-squares line through
    /// the rays' transmittances (a trunk under its crown sees out under the crown's skirt: open low, shut high).
    pub fn at(&self, p: Vec3) -> (f32, Vec3) {
        let through = self.through_at(p);
        let sky = through.iter().sum::<f32>() / SKY_RAYS as f32;
        let slope = self
            .fit
            .iter()
            .zip(through)
            .fold(Vec3::ZERO, |sum, (w, t)| sum + *w * t);
        (sky.clamp(0.0, 1.0), slope)
    }
}

/// The mean of the rays' directions over the upper hemisphere by its cosine (`shaders/foliage.wgsl`'s `SKY_MEAN`).
pub const SKY_MEAN: Vec3 = Vec3::new(0.0, 2.0 / 3.0, 0.0);
/// A slope's largest component, as a vertex carries it (snorm over this).
pub const SLOPE_RANGE: f32 = 4.0;

/// Each ray's weight in the slope's least-squares fit: the rays' covariance inverted, times the ray's offset from
/// their mean, over their count.
fn slope_fit(rays: &[Vec3; SKY_RAYS]) -> [Vec3; SKY_RAYS] {
    let mean = rays.iter().copied().sum::<Vec3>() / SKY_RAYS as f32;
    let mut cov = glam::Mat3::ZERO;
    for r in rays {
        let d = *r - mean;
        cov += glam::Mat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    let inverse = (cov * (1.0 / SKY_RAYS as f32)).inverse();
    std::array::from_fn(|i| inverse * (rays[i] - mean) / SKY_RAYS as f32)
}

/// How much of the sun bark at a point sees past its crown, from its sky and slope (`shaders/foliage.wgsl`'s
/// `bark_sun`, which this mirrors for the tests).
pub fn bark_sun(sky: f32, slope: Vec3, to_sun: Vec3) -> f32 {
    smoothstep(0.1, 0.9, sky + slope.dot(to_sun - SKY_MEAN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::{Species, models};

    fn model(species: Species) -> &'static Model {
        models().iter().find(|m| m.species == species).unwrap()
    }

    /// The top of the crown's leaves over its axis: the highest clump's top within a third of its reach of it.
    fn crown_top(m: &Model) -> Vec3 {
        let y = m
            .primitives
            .iter()
            .filter(|p| p.b[3] != KIND_BARK && p.a[0].hypot(p.a[2]) < m.radius * 0.3)
            .map(|p| p.a[1] + p.a[3])
            .fold(f32::MIN, f32::max);
        Vec3::new(0.0, y, 0.0)
    }

    /// The trunk's bark at `metres` up, on its +x side, and that side's normal.
    fn trunk_bark(m: &Model, metres: f32) -> Vec3 {
        let trunk = &m.limbs[0];
        let y = metres / m.species.height_m();
        let k = trunk
            .points
            .windows(2)
            .position(|w| w[1].y >= y)
            .unwrap_or(0);
        let (p0, p1) = (trunk.points[k], trunk.points[k + 1]);
        let t = ((y - p0.y) / (p1.y - p0.y).max(1e-6)).clamp(0.0, 1.0);
        let r = trunk.radii[k] + (trunk.radii[k + 1] - trunk.radii[k]) * t;
        p0.lerp(p1, t) + Vec3::X * r
    }

    #[test]
    fn the_shaders_read_the_slope_as_built() {
        assert!(
            include_str!("../../shaders/near_trees.wgsl")
                .contains(&format!("const SLOPE_RANGE: f32 = {SLOPE_RANGE:?};"))
        );
        assert!(
            include_str!("../../shaders/foliage.wgsl").contains(&format!(
                "const SKY_MEAN: vec3<f32> = vec3<f32>(0.0, {}, 0.0);",
                SKY_MEAN.y
            ))
        );
    }

    #[test]
    fn the_rays_spread_over_the_upper_hemisphere_by_its_cosine() {
        let rays = sky_rays();
        let mean = rays.iter().copied().sum::<Vec3>() / SKY_RAYS as f32;
        // A cosine-weighted hemisphere's mean direction is straight up, two thirds of a unit long.
        assert!(mean.x.abs() < 0.05 && mean.z.abs() < 0.05, "{mean:?}");
        assert!((mean.y - 2.0 / 3.0).abs() < 0.03, "{mean:?}");
        assert!(
            rays.iter()
                .all(|r| r.y > 0.15 && (r.length() - 1.0).abs() < 1e-5)
        );
    }

    /// The sky a crown leaves: at its sunny top most of it, a metre into a dense broadleaf crown a quarter or so, a
    /// trunk under an open-grown tree's crown less than a third (what it sees under the crown's skirt), and the sun
    /// on that trunk's bark low under the skirt but never high.
    #[test]
    fn the_crown_takes_the_sky_by_depth() {
        let mut rows = Vec::new();
        for m in models() {
            if m.sprays.is_empty() || m.limbs.is_empty() {
                continue;
            }
            let height = m.species.height_m();
            let light = CrownLight::new(m);
            let top = crown_top(m);
            let sky = |p: Vec3| light.at(p).0;
            let (trunk, slope) = light.at(trunk_bark(m, 1.5));
            let sun = |elevation: f32| {
                let e = elevation.to_radians();
                bark_sun(trunk, slope, Vec3::new(e.cos(), e.sin(), 0.0))
            };
            let extinction = clump_extinction(m, height);
            let leafy: Vec<f32> = extinction.iter().copied().filter(|k| *k > 0.0).collect();
            let per_m = leafy.iter().sum::<f32>() / leafy.len().max(1) as f32 / height;
            eprintln!(
                "{:>16}: {per_m:.2}/m; top {:.2}, 1 m in {:.2}, 3 m in {:.2}; trunk at 1.5 m {:.2}, sun at 12 deg {:.2}, at 60 deg {:.2}",
                format!("{:?}", m.species),
                sky(top),
                sky(top - Vec3::Y / height),
                sky(top - Vec3::Y * 3.0 / height),
                trunk,
                sun(12.0),
                sun(60.0),
            );
            rows.push((
                m.species,
                sky(top),
                sky(top - Vec3::Y / height),
                trunk,
                sun(12.0),
                sun(60.0),
            ));
        }
        let row = |s: Species| *rows.iter().find(|r| r.0 == s).unwrap();
        // A metre in: the rainforest 0.73, the live oak 0.71, the oak's separate masses 0.63, the laurel's deep dome
        // of overlapping clumps 0.48; three metres in 0.29, 0.09, 0.08 and 0.18 (2026-10-26, the light taking the
        // leaves as drawn).
        for (species, least, most) in [
            (Species::Rainforest, 0.5, 0.85),
            (Species::LiveOak, 0.5, 0.85),
            (Species::Oak, 0.5, 0.85),
            (Species::Laurel, 0.3, 0.6),
        ] {
            let (_, top, inside, _, _, _) = row(species);
            assert!((0.8..=1.0).contains(&top), "{species:?}: top {top}");
            assert!(
                (least..=most).contains(&inside),
                "{species:?}: a metre in {inside}"
            );
        }
        // Under a crown from a third of the way up, the sky below its skirt (about 40 degrees up from the trunk, the
        // sine squared of that, 0.4, of the sky by the cosine) is most of what the trunk sees; a high sun reaches it
        // only through the crown (the olive's open one lets a quarter through).
        for species in [
            Species::Oak,
            Species::LiveOak,
            Species::Olive,
            Species::Laurel,
        ] {
            let (_, _, _, trunk, low, high) = row(species);
            assert!((0.15..=0.6).contains(&trunk), "{species:?}: trunk {trunk}");
            assert!(
                low > 0.5 && high < 0.3,
                "{species:?}: sun low {low}, high {high}"
            );
        }
        // A bare crown lets most of the sky and the sun through, onto its trunk too.
        let (_, top, inside, trunk, _, high) = row(Species::BareOak);
        assert!(
            top > 0.9 && inside > 0.8 && trunk > 0.7 && high > 0.7,
            "bare oak: top {top}, a metre in {inside}, trunk {trunk}, a high sun on it {high}"
        );
        // In the open above a crown, the sun anywhere above the horizon.
        let light = CrownLight::new(model(Species::Oak));
        let (sky, slope) = light.at(Vec3::new(0.0, 1.05, 0.0));
        assert!(
            bark_sun(sky, slope, Vec3::new(0.8, 0.6, 0.0)) > 0.95,
            "{sky} {slope:?}"
        );
    }
}
