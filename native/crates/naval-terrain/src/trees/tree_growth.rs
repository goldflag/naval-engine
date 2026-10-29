//! The trees grown from their designs (native/docs/terrain/trees.md, "Grown trees"): each species' design
//! ([`crate::species`]: the trunk and limbs as capsules, the crown as clumps of leaves in lobes) grown into the tree
//! the near meshes draw and the impostors are baked from. The design's capsules become tapering tubes; each lobe of
//! clumps a branch from the nearest limb, each clump a branch from its lobe's, and each clump's leaves sprays on cards
//! whose stems sit on that branch and fan out to the clump's surface, as a branch's end carries its twigs and leaves.
//! A spray's card shows a branchlet with its leaves, needles or bare twigs at their real size (the leaf sprites,
//! `tree_meshes::leaf_sprites`), so a crown is many small sprays on real branches, not a ball of noise.
//!
//! Lobes on a trunk's axis (a conifer's whorls, a cypress' spiral, a bamboo's culms) grow their clumps from the trunk,
//! each from the nearest branch already grown, so a whorl's run of clumps becomes one branch reaching out. Lobes far
//! from every limb (a garrigue cushion, a chaparral's shrubs) grow their own stems from the ground. The runs of clumps
//! strung along a line (palm fronds, grass blades: `tree_meshes::chains`) stay ribbons and grow nothing here.
//!
//! Everything is in the design's normalised space (foot at 0, top at 1); sizes given in metres are taken over the
//! species' height where it grows well ([`Species::height_m`]), as it is placed. Deterministic: one random sequence
//! per species.

use glam::Vec3;

use crate::species::{KIND_BARK, Model, Primitive, Species};
use crate::tree_meshes::{Foliage, chains, foliage};

/// A tapering tube: the design's trunk or limb, or a branch grown to a lobe or a clump.
#[derive(Clone, Debug, PartialEq)]
pub struct Limb {
    /// Its axis, from where it grows (model space).
    pub points: Vec<Vec3>,
    /// Its radius at each point (model units).
    pub radii: Vec<f32>,
    /// Linear albedo.
    pub color: [f32; 3],
    /// Ambient occlusion (0 hidden, 1 open).
    pub occlusion: f32,
    /// 0 the design's trunk and limbs, 1 a lobe's branch, 2 a clump's.
    pub order: u8,
    /// It stands on the ground: its foot flares into the roots.
    pub flare: bool,
}

/// A spray of leaves, needles or bare twigs on a card: its stem at `base`, reaching `length` along `along`, `width`
/// across `across`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spray {
    pub base: Vec3,
    pub along: Vec3,
    pub across: Vec3,
    pub length: f32,
    pub width: f32,
    /// The design's clump it fills (its colour, openness and shading), an index into the model's shapes.
    pub clump: u32,
    /// Its own brightness (about 1).
    pub tone: f32,
    /// A random value 0-1: the reduced level of detail keeps the sprays below its share.
    pub keep: f32,
    /// Its phase in the wind, 0-1.
    pub phase: f32,
}

/// How a species' crown grows: its sprays' size and number, and how they stand.
#[derive(Clone, Copy, Debug)]
struct Habit {
    /// A spray's length (m) and its width over its length.
    spray_m: f32,
    aspect: f32,
    /// Sprays a clump holds for its size: about this many cards' worth of leaves over its outline.
    fill: f32,
    /// How far a spray turns up from its branch (negative: hangs down).
    rise: f32,
    /// How far its face turns to the sky (0 any way, 1 level).
    level: f32,
    /// How far a spray's direction strays from its branch's (0 along it, 1 any way outward).
    spread: f32,
    /// A clump branch's radius at its end (m).
    twig_m: f32,
}

fn habit(species: Species) -> Habit {
    let base = Habit {
        spray_m: 1.0,
        aspect: 0.8,
        fill: 2.2,
        rise: 0.3,
        level: 0.45,
        spread: 0.85,
        twig_m: 0.012,
    };
    match foliage(species) {
        Foliage::Needles => Habit {
            spray_m: if species == Species::Redwood {
                1.5
            } else {
                1.1
            },
            aspect: 0.75,
            fill: 2.4,
            rise: -0.25,
            level: 0.8,
            spread: 0.6,
            twig_m: 0.015,
        },
        Foliage::PineTuft => Habit {
            spray_m: if species == Species::StonePine {
                1.0
            } else {
                0.9
            },
            aspect: 0.85,
            fill: 2.2,
            rise: 0.55,
            level: 0.25,
            spread: 0.7,
            twig_m: 0.015,
        },
        Foliage::Scale => Habit {
            spray_m: 0.9,
            aspect: 0.7,
            fill: 2.6,
            rise: 0.5,
            level: 0.15,
            spread: 0.75,
            twig_m: 0.012,
        },
        Foliage::Twigs => Habit {
            spray_m: if species == Species::BareBirch {
                1.1
            } else {
                1.5
            },
            aspect: 0.9,
            fill: 4.2,
            rise: 0.45,
            level: 0.0,
            spread: 0.8,
            twig_m: 0.01,
        },
        Foliage::Lanceolate => Habit {
            spray_m: match species {
                Species::Olive => 0.7,
                Species::Bamboo => 0.9,
                _ => 1.0,
            },
            aspect: 0.75,
            fill: 2.0,
            rise: match species {
                Species::Eucalyptus => -0.35,
                Species::Bamboo => -0.3,
                _ => 0.2,
            },
            level: 0.25,
            spread: 0.85,
            twig_m: 0.01,
        },
        Foliage::SmallLeaf => Habit {
            spray_m: match species {
                Species::Garrigue => 0.22,
                Species::Shrub => 0.45,
                Species::Chaparral => 0.55,
                Species::Mangrove | Species::Acacia => 0.7,
                _ => 0.8,
            },
            fill: 2.4,
            ..base
        },
        Foliage::Tropical => Habit {
            spray_m: 1.6,
            aspect: 0.85,
            fill: 2.2,
            rise: 0.2,
            level: 0.55,
            ..base
        },
        Foliage::Lobed => Habit {
            spray_m: if species == Species::Sycamore {
                1.1
            } else {
                1.0
            },
            ..base
        },
        Foliage::Broadleaf => Habit {
            spray_m: match species {
                Species::Birch => 0.7,
                Species::Ash => 1.1,
                _ => 0.95,
            },
            rise: if species == Species::Birch {
                -0.15
            } else {
                0.3
            },
            ..base
        },
        Foliage::Fan => Habit {
            spray_m: 1.5,
            aspect: 1.0,
            fill: 1.0,
            rise: 0.0,
            level: 0.0,
            spread: 0.0,
            twig_m: 0.02,
        },
        Foliage::Frond | Foliage::Grass => Habit {
            spray_m: 0.8,
            ..base
        },
    }
}

/// The web's LCG, as the designs use: deterministic per species.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
    /// A unit vector, uniform over the sphere.
    fn unit(&mut self) -> Vec3 {
        loop {
            let p = Vec3::new(
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
            );
            let l = p.length();
            if l <= 1.0 && l > 1e-3 {
                return p / l;
            }
        }
    }
}

fn centre_of(p: &Primitive) -> Vec3 {
    Vec3::new(p.a[0], p.a[1], p.a[2])
}

fn lobe_of(p: &Primitive) -> Vec3 {
    Vec3::new(p.b[0], p.b[1], p.b[2])
}

/// A piece of the branches grown so far, which a lobe or a clump may grow from.
#[derive(Clone, Copy)]
struct Piece {
    a: Vec3,
    b: Vec3,
    ra: f32,
    rb: f32,
    /// The lobe it belongs to (`usize::MAX` the design's limbs).
    lobe: usize,
}

impl Piece {
    /// The nearest point of its axis to `p`, and its radius there.
    fn nearest(&self, p: Vec3) -> (Vec3, f32) {
        let ab = self.b - self.a;
        let t = ((p - self.a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
        (self.a + ab * t, self.ra + (self.rb - self.ra) * t)
    }
}

/// Where a branch toward `to` grows from among `pieces` (those `allowed`): the nearest point, a little farther for a
/// point above `to` (branches grow up and out, not down from above), and its radius there.
fn attach(
    pieces: &[Piece],
    to: Vec3,
    allowed: impl Fn(&Piece) -> bool,
) -> Option<(Vec3, f32, f32)> {
    let mut best: Option<(Vec3, f32, f32)> = None;
    for piece in pieces.iter().filter(|p| allowed(p)) {
        let (at, r) = piece.nearest(to);
        let d = to - at;
        let length = d.length();
        let above = (-d.y / length.max(1e-6)).max(0.0);
        let cost = length * (1.0 + 0.8 * above);
        if best.is_none_or(|b| cost < b.2) {
            best = Some((at, r, cost));
        }
    }
    best.map(|(at, r, _)| (at, r, (to - at).length()))
}

/// A gentle arc from `a` to `b` in `segments`, its middle lifted by `bow` (model units) and pushed sideways by `side`.
fn arc(a: Vec3, b: Vec3, bow: f32, side: Vec3, segments: usize) -> Vec<Vec3> {
    (0..=segments)
        .map(|k| {
            let t = k as f32 / segments as f32;
            let lift = 4.0 * t * (1.0 - t);
            a.lerp(b, t) + Vec3::Y * (bow * lift) + side * lift
        })
        .collect()
}

/// The design's limbs: its capsules joined end to end into tubes (a limb's segments share their ends).
fn design_limbs(model: &Model) -> Vec<Limb> {
    let mut limbs: Vec<Limb> = Vec::new();
    let mut previous_end: Option<Vec3> = None;
    for p in &model.primitives {
        if p.b[3] != KIND_BARK {
            previous_end = None;
            continue;
        }
        let a = Vec3::new(p.a[0], p.a[1], p.a[2]);
        let b = Vec3::new(p.b[0], p.b[1], p.b[2]);
        let r = p.a[3];
        let color = [p.c[0], p.c[1], p.c[2]];
        let continues = previous_end.is_some_and(|e| e.distance(a) < 1e-5);
        match limbs.last_mut() {
            Some(limb) if continues => {
                // The joint takes the mean of the two segments' radii; the end, this segment's.
                let last = limb.radii.len() - 1;
                limb.radii[last] = 0.5 * (limb.radii[last] + r);
                limb.points.push(b);
                limb.radii.push(r);
                limb.occlusion = limb.occlusion.min(p.d[0]);
                for (c, n) in limb.color.iter_mut().zip(color) {
                    *c = 0.5 * (*c + n);
                }
            }
            _ => limbs.push(Limb {
                points: vec![a, b],
                radii: vec![r, r],
                color,
                occlusion: p.d[0],
                order: 0,
                flare: a.y < 0.01,
            }),
        }
        previous_end = Some(b);
    }
    // A tube tapers along its length: each point's radius toward the mean of its segment's ends.
    for limb in &mut limbs {
        let n = limb.radii.len();
        if n >= 3 {
            let r = limb.radii.clone();
            for k in 1..n - 1 {
                limb.radii[k] = 0.25 * r[k - 1] + 0.5 * r[k] + 0.25 * r[k + 1];
            }
        }
    }
    limbs
}

/// A clump's lobe and its clumps, in the design's order.
struct Lobe {
    centre: Vec3,
    clumps: Vec<usize>,
}

/// The design grown: its limbs as tubes, branches to its lobes and clumps, and its clumps' sprays.
pub fn grow(model: &Model) -> (Vec<Limb>, Vec<Spray>) {
    let species = model.species;
    let habit = habit(species);
    let per_m = 1.0 / species.height_m();
    let mut rng = Rng(0x2545_f491 ^ (species as u32).wrapping_mul(0x9e37_79b9));
    let mut limbs = design_limbs(model);
    let mut pieces: Vec<Piece> = Vec::new();
    for limb in &limbs {
        for k in 1..limb.points.len() {
            pieces.push(Piece {
                a: limb.points[k - 1],
                b: limb.points[k],
                ra: limb.radii[k - 1],
                rb: limb.radii[k],
                lobe: usize::MAX,
            });
        }
    }
    let skeleton_pieces = pieces.len();
    let has_skeleton = skeleton_pieces > 0;

    // The ribbons (fronds, blades) grow nothing here.
    let mut in_run = vec![false; model.primitives.len()];
    for (first, length) in chains(model) {
        for flag in &mut in_run[first..first + length] {
            *flag = true;
        }
    }
    let mut lobes: Vec<Lobe> = Vec::new();
    for (index, p) in model.primitives.iter().enumerate() {
        if p.b[3] == KIND_BARK || in_run[index] {
            continue;
        }
        let centre = lobe_of(p);
        match lobes.iter_mut().find(|l| l.centre.distance(centre) < 1e-6) {
            Some(lobe) => lobe.clumps.push(index),
            None => lobes.push(Lobe {
                centre,
                clumps: vec![index],
            }),
        }
    }

    let twig_r = habit.twig_m * per_m;
    let fan = foliage(species) == Foliage::Fan;
    let mut sprays: Vec<Spray> = Vec::new();
    for (lobe_index, lobe) in lobes.iter().enumerate() {
        let shapes: Vec<&Primitive> = lobe.clumps.iter().map(|&i| &model.primitives[i]).collect();
        let mean = shapes.iter().map(|p| centre_of(p)).sum::<Vec3>() / shapes.len() as f32;
        let reach = shapes
            .iter()
            .map(|p| centre_of(p).distance(mean) + p.a[3])
            .fold(0.0f32, f32::max);
        // A lobe on a limb's axis (a whorl about the trunk) grows its clumps straight from the limbs.
        let on_axis = attach(&pieces[..skeleton_pieces], lobe.centre, |_| true)
            .is_some_and(|(_, r, d)| d < r * 2.0 + 0.6 * per_m);
        let lobe_occlusion = shapes.iter().map(|p| p.d[0]).sum::<f32>() / shapes.len() as f32;
        // The branches darker than the trunk they grow from: younger bark, and shaded among the leaves.
        let bark = limbs
            .first()
            .map_or([0.25, 0.2, 0.15], |l| l.color)
            .map(|c| c * 0.75);
        let thickness = |clumps: usize| twig_r * (clumps as f32).powf(0.45) * 1.15;
        if !on_axis {
            // The lobe's branch: from the nearest limb (or the ground, a shrub's own stem) to the lobe's middle.
            let from = attach(&pieces[..skeleton_pieces], mean, |_| true)
                .filter(|(_, _, d)| *d < (reach * 2.5).max(3.0 * per_m));
            let (start, parent_r, grounded) = match from {
                Some((at, r, _)) => (at, r, false),
                None => (
                    Vec3::new(mean.x * 0.85, 0.0, mean.z * 0.85),
                    thickness(lobe.clumps.len()) * 1.4,
                    true,
                ),
            };
            let r0 = thickness(lobe.clumps.len()).min(parent_r * 0.8);
            let span = mean - start;
            let side = rng.unit() * span.length() * 0.08;
            let points = arc(
                start,
                mean,
                span.length() * if grounded { 0.05 } else { 0.12 },
                Vec3::new(side.x, 0.0, side.z),
                4,
            );
            let n = points.len();
            let radii: Vec<f32> = (0..n)
                .map(|k| r0 * (1.0 - 0.45 * k as f32 / (n - 1) as f32))
                .collect();
            for k in 1..n {
                pieces.push(Piece {
                    a: points[k - 1],
                    b: points[k],
                    ra: radii[k - 1],
                    rb: radii[k],
                    lobe: lobe_index,
                });
            }
            limbs.push(Limb {
                points,
                radii,
                color: bark,
                occlusion: (lobe_occlusion * 0.6).clamp(0.05, 1.0),
                order: 1,
                flare: grounded && has_skeleton,
            });
        }
        // Its clumps, the nearest to where the lobe grows from first, each from the nearest branch already grown.
        let mut order: Vec<usize> = (0..shapes.len()).collect();
        let key = |i: usize| -> f32 {
            let c = centre_of(shapes[i]);
            if on_axis {
                // Out from the axis.
                let axis =
                    attach(&pieces[..skeleton_pieces], c, |_| true).map_or(c, |(at, _, _)| at);
                c.distance(axis)
            } else {
                c.distance(mean)
            }
        };
        order.sort_by(|&a, &b| key(a).total_cmp(&key(b)));
        for i in order {
            let p = shapes[i];
            let index = lobe.clumps[i];
            let c = centre_of(p);
            let r = p.a[3];
            let allowed =
                |piece: &Piece| piece.lobe == lobe_index || (on_axis && piece.lobe == usize::MAX);
            let Some((start, parent_r, _)) =
                attach(&pieces, c, allowed).or_else(|| attach(&pieces, c, |_| true))
            else {
                continue;
            };
            let dir = (c - start).normalize_or(Vec3::Y);
            let span = c.distance(start);
            // Through the clump's middle and on into it, toward its outer side.
            let end = c + dir * r * 0.35;
            let points = arc(start, end, span * 0.06, Vec3::ZERO, 2);
            let r0 = (twig_r * 2.2).min(parent_r * 0.7).max(twig_r);
            let radii = vec![r0, (r0 + twig_r) * 0.5, twig_r * 0.7];
            for k in 1..points.len() {
                pieces.push(Piece {
                    a: points[k - 1],
                    b: points[k],
                    ra: radii[k - 1],
                    rb: radii[k],
                    lobe: lobe_index,
                });
            }
            limbs.push(Limb {
                points,
                radii,
                color: bark,
                occlusion: (p.d[0] * 0.55).clamp(0.05, 1.0),
                order: 2,
                flare: false,
            });
            if fan {
                // A fan palm's fan: one card across its stalk, facing out from the head.
                let normal = (c - lobe.centre).normalize_or(Vec3::Y);
                let across = normal.cross(Vec3::Y).normalize_or(Vec3::X);
                let along = across.cross(normal).normalize_or(Vec3::Y);
                let size = r * 2.3;
                sprays.push(Spray {
                    base: c - along * size * 0.5,
                    along,
                    across,
                    length: size,
                    width: size,
                    clump: index as u32,
                    tone: rng.range(0.9, 1.1),
                    keep: rng.next(),
                    phase: rng.next(),
                });
                continue;
            }
            // Its sprays: their stems on the branch inside the clump, fanning out to its surface.
            let spray = habit.spray_m * per_m;
            let outline = std::f32::consts::PI * r * r;
            let card = spray * spray * habit.aspect * 0.5;
            let open = p.c[3];
            let count =
                ((habit.fill * (1.25 - 0.5 * open) * outline / card).round() as usize).clamp(2, 40);
            let outward = if on_axis {
                let level = Vec3::new(dir.x, 0.0, dir.z);
                level.normalize_or(dir)
            } else {
                (c - lobe.centre).normalize_or(dir)
            };
            let entry = c - dir * r * 0.55;
            for _ in 0..count {
                let t = rng.next();
                let jitter = rng.unit() * r * 0.22;
                let base = entry.lerp(end, t.sqrt()) + jitter;
                // Outward from the branch, turned up (or hanging down) by the habit.
                let wild = rng.unit();
                let mut along = (wild * habit.spread
                    + outward * (1.0 - habit.spread * 0.6)
                    + dir * 0.35
                    + Vec3::Y * habit.rise)
                    .normalize_or(outward);
                if along.dot(outward) < -0.2 {
                    along = (along + outward * 0.8).normalize_or(outward);
                }
                let mut length = spray * rng.range(0.8, 1.2);
                // Its tip no farther than the clump's surface (a little past it).
                let to = base - c;
                let b = to.dot(along);
                let q = to.length_squared() - (r * 1.08) * (r * 1.08);
                let h = b * b - q;
                if h > 0.0 {
                    let exit = -b + h.sqrt();
                    length = length.min(exit.max(spray * 0.45));
                }
                // Its face turned toward the sky by the habit.
                let sky = (Vec3::Y - along * along.y).normalize_or(Vec3::X);
                let any = along.any_orthonormal_vector();
                let turn = rng.next() * std::f32::consts::TAU;
                let random = any * turn.cos() + along.cross(any) * turn.sin();
                let face = (random * (1.0 - habit.level) + sky * habit.level).normalize_or(random);
                let across = along.cross(face).normalize_or(any);
                // Above the ground: a low clump's spray lifted clear of it.
                let half = across * (length * habit.aspect * 0.5);
                let tip = base + along * length;
                let lowest = (base - half)
                    .y
                    .min((base + half).y)
                    .min((tip - half).y)
                    .min((tip + half).y);
                let base = base + Vec3::Y * (0.004 - lowest).max(0.0);
                sprays.push(Spray {
                    base,
                    along,
                    across,
                    length,
                    width: length * habit.aspect,
                    clump: index as u32,
                    tone: rng.range(0.85, 1.15),
                    keep: rng.next(),
                    phase: rng.next(),
                });
            }
        }
    }
    (limbs, sprays)
}

/// The widest reach of a grown tree from its axis (model units): its limbs and its sprays' corners.
pub fn reach(limbs: &[Limb], sprays: &[Spray]) -> f32 {
    let level = |p: Vec3| (p.x * p.x + p.z * p.z).sqrt();
    let mut most: f32 = 0.0;
    for limb in limbs {
        for (p, r) in limb.points.iter().zip(&limb.radii) {
            most = most.max(level(*p) + r);
        }
    }
    for s in sprays {
        for corner in corners(s) {
            most = most.max(level(corner));
        }
    }
    most
}

/// A spray's card's four corners: its stem's two, then its tip's.
pub fn corners(s: &Spray) -> [Vec3; 4] {
    let half = s.across * (s.width * 0.5);
    let tip = s.base + s.along * s.length;
    [s.base - half, s.base + half, tip + half, tip - half]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::models;

    #[test]
    fn every_tree_grows_limbs_and_sprays_within_its_bounds() {
        for model in models() {
            let runs = chains(model).len();
            let palm_or_tuft = matches!(foliage(model.species), Foliage::Frond | Foliage::Grass);
            assert!(
                !model.sprays.is_empty() || palm_or_tuft,
                "{:?}: no sprays",
                model.species
            );
            assert!(
                runs > 0 || !model.sprays.is_empty(),
                "{:?}: neither sprays nor ribbons",
                model.species
            );
            for s in &model.sprays {
                for corner in corners(s) {
                    assert!(
                        corner.y > -0.05 && corner.y < 1.06,
                        "{:?}: a spray at {corner}",
                        model.species
                    );
                }
                assert!((s.along.length() - 1.0).abs() < 1e-3);
                assert!(s.along.dot(s.across).abs() < 1e-3);
                assert!(s.length > 0.0 && s.width > 0.0);
            }
            for limb in &model.limbs {
                assert_eq!(limb.points.len(), limb.radii.len());
                assert!(limb.points.len() >= 2);
                assert!(limb.radii.iter().all(|r| *r > 0.0), "{:?}", model.species);
            }
            assert!(model.radius >= reach(&model.limbs, &model.sprays) - 1e-5);
        }
    }
}
