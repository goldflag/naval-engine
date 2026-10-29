//! Power lines and masts (native/docs/terrain/infra.md, "Power lines"): Overture Maps' overhead lines (the host's
//! `theatre::power`, chunk-local), each transmission line on lattice towers and each distribution line on wooden poles,
//! one at each of its vertices, their wires hung between them in catenaries; and the communication towers, triangular
//! lattice masts.
//!
//! - **Supports** stand at a line's vertices (those closer than [`SPAN_MIN_M`] to the last one are bends, not
//!   supports), turned across the bisector of their spans, drawn by the chunk holding them, once where lines meet. A
//!   tower is as tall as the map's power tower there says, else [`TOWER_M`]; within the close range a lattice of legs,
//!   bracing, two crossarms and insulators ([`Group::Near`]), past it its legs, a cross on each face and its arms as
//!   lattice members ([`MeshBuilder::member`], [`Group::Far`]): lines opened to a pixel and blended over the scene by
//!   the share of it they cover, so a tower thins to nothing with distance.
//! - **Wires** hang from the crossarms' insulators and the earth wire's peak, five a transmission span and three a
//!   distribution span, sagging 3% (2%) of the span, drawn by the chunk holding the span's middle, on chunks within the
//!   near range only ([`Group::Detail`]; the distribution lines' with their poles, within the close range), lines
//!   blended as the members afar are, a little stronger so a span across the sky stays just seen.
//! - **Communication towers** are as tall as mapped (else [`MAST_M`]), banded red and white from 60 m.

use glam::{Vec2, Vec3};

use super::mesh::{Group, Material, MeshBuilder, Paint};
use crate::buildings::CHUNK_M;

const SIDE: f32 = CHUNK_M as f32;

/// What carries a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    /// A transmission line on lattice towers.
    Transmission,
    /// A distribution line on wooden poles.
    Distribution,
}

/// An overhead line, whole (chunk-local).
#[derive(Clone, Debug, PartialEq)]
pub struct PowerLine {
    pub kind: LineKind,
    pub points: Vec<Vec2>,
}

/// A mast's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MastKind {
    PowerTower,
    PowerPole,
    Communication,
}

/// A mast where the map has one (chunk-local), its height where mapped (m, else 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mast {
    pub kind: MastKind,
    pub at: Vec2,
    pub height: f32,
}

/// The lines and masts about a chunk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Power {
    pub lines: Vec<PowerLine>,
    pub masts: Vec<Mast>,
}

impl Power {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.masts.is_empty()
    }
}

/// A lattice tower's height where the map gives none (m): a 230 kV double circuit's.
pub const TOWER_M: f32 = 36.0;
/// A wooden pole's height (m).
pub const POLE_M: f32 = 11.0;
/// A communication tower's height where the map gives none (m).
pub const MAST_M: f32 = 60.0;
/// A vertex nearer its last support than this is a bend in the line, not a support (m).
pub const SPAN_MIN_M: f32 = 25.0;

const STEEL: Paint = Paint::new([150, 154, 152], Material::Steel);
const WOOD: Paint = Paint::new([92, 72, 54], Material::Paint);
const INSULATOR: Paint = Paint::new([96, 112, 104], Material::Hull);
const WIRE_COLOUR: [u8; 3] = [92, 94, 96];
const RED: Paint = Paint::new([176, 52, 40], Material::Paint);
const WHITE: Paint = Paint::new([214, 214, 208], Material::Paint);

fn owned(p: Vec2) -> bool {
    (0.0..SIDE).contains(&p.x) && (0.0..SIDE).contains(&p.y)
}

/// A support along a line: where, its foot's height, its height, the unit direction across the line (the left of the
/// line's way, so its neighbours' match) and what it carries.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Support {
    at: Vec2,
    foot: f32,
    height: f32,
    across: Vec2,
    kind: LineKind,
}

impl Support {
    /// Where its wires hang (from the across side's left): a tower's two crossarms' insulators a side and its peak's
    /// earth wire, a pole's crossarm ends and its top.
    fn attachments(&self) -> Vec<Vec3> {
        let at = |across: f32, up: f32| {
            let p = self.at + self.across * across;
            Vec3::new(p.x, self.foot + up, p.y)
        };
        let h = self.height;
        match self.kind {
            LineKind::Transmission => vec![
                at(-0.24 * h, 0.70 * h - 2.4),
                at(-0.18 * h, 0.84 * h - 2.4),
                at(0.0, h),
                at(0.18 * h, 0.84 * h - 2.4),
                at(0.24 * h, 0.70 * h - 2.4),
            ],
            LineKind::Distribution => vec![at(-1.1, h - 0.3), at(0.0, h + 0.25), at(1.1, h - 0.3)],
        }
    }
}

/// A line's supports: at its first and last points and every vertex at least [`SPAN_MIN_M`] on from the last.
fn supports(line: &PowerLine, masts: &[Mast], ground: &dyn Fn(Vec2) -> f32) -> Vec<Support> {
    let mut at: Vec<Vec2> = Vec::new();
    for (i, p) in line.points.iter().enumerate() {
        let last = i + 1 == line.points.len();
        match at.last() {
            Some(q) if q.distance(*p) < SPAN_MIN_M => {
                if last {
                    *at.last_mut().expect("a support") = *p;
                }
            }
            _ => at.push(*p),
        }
    }
    let n = at.len();
    (0..n)
        .map(|i| {
            let before = if i > 0 {
                (at[i] - at[i - 1]).normalize_or_zero()
            } else {
                Vec2::ZERO
            };
            let after = if i + 1 < n {
                (at[i + 1] - at[i]).normalize_or_zero()
            } else {
                Vec2::ZERO
            };
            let way = (before + after).normalize_or(Vec2::X);
            let height = match line.kind {
                LineKind::Transmission => masts
                    .iter()
                    .filter(|m| {
                        m.kind == MastKind::PowerTower
                            && m.height > 0.0
                            && m.at.distance(at[i]) < 12.0
                    })
                    .map(|m| m.height.clamp(15.0, 90.0))
                    .next()
                    .unwrap_or(TOWER_M),
                LineKind::Distribution => POLE_M,
            };
            Support {
                at: at[i],
                foot: ground(at[i]).max(0.0),
                height,
                // The way turned a quarter anticlockwise seen from above (x east, z south): its left.
                across: Vec2::new(way.y, -way.x),
                kind: line.kind,
            }
        })
        .collect()
}

/// A wire's points from `a` to `b`, sagging `sag` at its middle.
fn catenary(a: Vec3, b: Vec3, sag: f32) -> Vec<Vec3> {
    let span = a.distance(b);
    let n = (span / 30.0).ceil().clamp(4.0, 16.0) as usize;
    (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            a.lerp(b, t) - Vec3::Y * (4.0 * sag * t * (1.0 - t))
        })
        .collect()
}

/// A lattice tower `h` high at `foot` (x, ground, z), its arms along `across`: within the close range (`full`) its
/// legs, bracing, girts, crossarms and insulators; else its legs, a cross on each face below the waist and its arms as
/// lattice members (lines at least a pixel wide, blended by the share of it they cover, never more), so a tower afar is
/// a faint lattice that thins to nothing rather than a solid shape or a few pixels flickering.
fn lattice_tower(m: &mut MeshBuilder, foot: Vec3, across: Vec2, h: f32, full: bool) {
    let a = Vec3::new(across.x, 0.0, across.y);
    let l = Vec3::new(-across.y, 0.0, across.x);
    let (w0, w1, w2) = (
        (0.16 * h).clamp(3.0, 8.0),
        (0.05 * h).max(1.2),
        (0.035 * h).max(0.8),
    );
    let waist = 0.62 * h;
    let corners = |y: f32, w: f32| -> [Vec3; 4] {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(i, j)| foot + a * (i * w) + l * (j * w) + Vec3::Y * y)
    };
    let width_at = |y: f32| {
        if y <= waist {
            w0 + (w1 - w0) * (y / waist)
        } else {
            w1 + (w2 - w1) * ((y - waist) / (h - waist))
        }
    };
    let (base, mid, top) = (corners(0.0, w0), corners(waist, w1), corners(h, w2));
    let arms = [(0.70 * h, 0.24 * h), (0.84 * h, 0.18 * h)];
    if !full {
        // The legs, a cross on each face of the body below the waist, and the arms.
        for k in 0..4 {
            m.member(&[base[k], mid[k], top[k]], 25, STEEL.colour);
            let n = (k + 1) % 4;
            m.member(&[base[k], mid[n]], 12, STEEL.colour);
            m.member(&[base[n], mid[k]], 12, STEEL.colour);
        }
        for (y, reach) in arms {
            let c = foot + Vec3::Y * y;
            m.member(&[c - a * reach, c + a * reach], 30, STEEL.colour);
        }
        return;
    }
    for k in 0..4 {
        m.beam(base[k], mid[k], 0.16, STEEL);
        m.beam(mid[k], top[k], 0.12, STEEL);
    }
    for (y, reach) in arms {
        let c = foot + Vec3::Y * y;
        m.beam(c - a * reach, c + a * reach, 0.22, STEEL);
    }
    // Bracing: each panel's faces crossed, a girt at its top.
    let levels = [0.0, 0.2 * h, 0.4 * h, waist, 0.81 * h, h];
    for w in levels.windows(2) {
        let (lo, hi) = (corners(w[0], width_at(w[0])), corners(w[1], width_at(w[1])));
        for k in 0..4 {
            let n = (k + 1) % 4;
            m.beam(lo[k], hi[n], 0.06, STEEL);
            m.beam(lo[n], hi[k], 0.06, STEEL);
            m.beam(hi[k], hi[n], 0.06, STEEL);
        }
    }
    // The arms' lower chords, back to the body, and the insulators hanging from their ends.
    for (y, reach) in arms {
        let c = foot + Vec3::Y * y;
        let body = width_at(y);
        for s in [-1.0, 1.0] {
            m.beam(
                c + a * (s * reach),
                c + a * (s * body) - Vec3::Y * 2.0,
                0.08,
                STEEL,
            );
            let tip = c + a * (s * reach);
            m.beam(tip, tip - Vec3::Y * 2.4, 0.14, INSULATOR);
        }
    }
}

/// A wooden pole `h` high, its crossarm along `across`.
fn pole(m: &mut MeshBuilder, foot: Vec3, across: Vec2, h: f32) {
    let a = Vec3::new(across.x, 0.0, across.y);
    m.beam(foot, foot + Vec3::Y * (h + 0.4), 0.14, WOOD);
    let arm = foot + Vec3::Y * (h - 0.45);
    m.beam(arm - a * 1.3, arm + a * 1.3, 0.06, WOOD);
}

/// A triangular lattice mast `h` high, banded red and white from 60 m: within the close range (`full`) its legs and
/// bracing, else its legs as lines (as a tower's afar).
fn communication_tower(m: &mut MeshBuilder, foot: Vec3, h: f32, full: bool) {
    let r = (h * 0.012).clamp(0.9, 2.5);
    let legs: [Vec3; 3] = [0.0f32, 2.094, 4.189].map(|t| Vec3::new(t.cos() * r, 0.0, t.sin() * r));
    let bands = if h >= 60.0 { 7 } else { 1 };
    for b in 0..bands {
        let (y0, y1) = (
            h * b as f32 / bands as f32,
            h * (b + 1) as f32 / bands as f32,
        );
        let paint = if bands == 1 {
            STEEL
        } else if b % 2 == 0 {
            RED
        } else {
            WHITE
        };
        for leg in &legs {
            let (p, q) = (foot + *leg + Vec3::Y * y0, foot + *leg + Vec3::Y * y1);
            if full {
                m.beam(p, q, 0.1, paint);
            } else {
                m.member(&[p, q], 30, paint.colour);
            }
        }
        if full {
            let panels = ((y1 - y0) / (2.5 * r)).ceil().max(1.0) as usize;
            for p in 0..panels {
                let (ya, yb) = (
                    y0 + (y1 - y0) * p as f32 / panels as f32,
                    y0 + (y1 - y0) * (p + 1) as f32 / panels as f32,
                );
                for k in 0..3 {
                    let (s, e) = (legs[k], legs[(k + 1) % 3]);
                    m.beam(
                        foot + s + Vec3::Y * ya,
                        foot + e + Vec3::Y * yb,
                        0.04,
                        paint,
                    );
                    m.beam(
                        foot + s + Vec3::Y * yb,
                        foot + e + Vec3::Y * yb,
                        0.04,
                        paint,
                    );
                }
            }
        }
    }
}

/// Draws the chunk's supports, masts and wires; returns the supports and masts it drew.
pub fn draw(m: &mut MeshBuilder, power: &Power, ground: &dyn Fn(Vec2) -> f32) -> u32 {
    if power.is_empty() {
        return 0;
    }
    let group = m.group();
    let near = m.near();
    let mut drawn: Vec<Vec2> = Vec::new();
    let mut count = 0;
    for line in &power.lines {
        let supports = supports(line, &power.masts, ground);
        for s in &supports {
            if !owned(s.at) || drawn.iter().any(|d| d.distance(s.at) < 3.0) {
                continue;
            }
            drawn.push(s.at);
            count += 1;
            let foot = Vec3::new(s.at.x, s.foot, s.at.y);
            match s.kind {
                LineKind::Transmission => {
                    m.set_group(Group::Far);
                    lattice_tower(m, foot, s.across, s.height, false);
                    if near {
                        m.set_group(Group::Near);
                        lattice_tower(m, foot, s.across, s.height, true);
                    }
                }
                LineKind::Distribution if near => {
                    m.set_group(Group::Near);
                    pole(m, foot, s.across, s.height);
                }
                LineKind::Distribution => {}
            }
        }
        if !near {
            continue;
        }
        let (sag, radius, group) = match line.kind {
            LineKind::Transmission => (0.03, 2, Group::Detail),
            LineKind::Distribution => (0.02, 1, Group::Near),
        };
        m.set_group(group);
        for w in supports.windows(2) {
            if !owned((w[0].at + w[1].at) * 0.5) {
                continue;
            }
            let span = w[0].at.distance(w[1].at);
            for (a, b) in w[0].attachments().into_iter().zip(w[1].attachments()) {
                m.wire(&catenary(a, b, sag * span), radius, WIRE_COLOUR);
            }
        }
    }
    for mast in &power.masts {
        if mast.kind != MastKind::Communication || !owned(mast.at) {
            continue;
        }
        count += 1;
        let h = if mast.height > 0.0 {
            mast.height.clamp(15.0, 400.0)
        } else {
            MAST_M
        };
        let foot = Vec3::new(mast.at.x, ground(mast.at).max(0.0), mast.at.y);
        m.set_group(Group::Far);
        communication_tower(m, foot, h, false);
        if near {
            m.set_group(Group::Near);
            communication_tower(m, foot, h, true);
        }
    }
    m.set_group(group);
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::mesh::InfraMesh;

    fn line(kind: LineKind, points: &[[f32; 2]]) -> PowerLine {
        PowerLine {
            kind,
            points: points.iter().map(|p| Vec2::from(*p)).collect(),
        }
    }

    fn wires(mesh: &InfraMesh, g: Group) -> Vec<Vec3> {
        mesh.indices[mesh.line_range(g).start as usize..mesh.line_range(g).end as usize]
            .iter()
            .map(|&i| mesh.vertices[i as usize])
            .filter(|v| v.colour[3] == Material::Wire as u8)
            .map(|v| Vec3::from(v.position))
            .collect()
    }

    #[test]
    fn a_transmission_line_stands_on_towers_with_its_wires_sagging_between() {
        // Across the chunk and beyond it, a bend 10 m after the second vertex that is no support.
        let power = Power {
            lines: vec![line(
                LineKind::Transmission,
                &[
                    [-200.0, 500.0],
                    [150.0, 500.0],
                    [160.0, 501.0],
                    [500.0, 520.0],
                    [900.0, 560.0],
                    [1300.0, 600.0],
                ],
            )],
            masts: vec![Mast {
                kind: MastKind::PowerTower,
                at: Vec2::new(500.0, 521.0),
                height: 48.0,
            }],
        };
        let ground = |_: Vec2| 20.0;
        let mut m = MeshBuilder::new(true);
        assert_eq!(
            draw(&mut m, &power, &ground),
            3,
            "the supports the chunk holds"
        );
        let mesh = m.finish();
        assert!(!mesh.range(Group::Near).is_empty() && !mesh.line_range(Group::Far).is_empty());
        // The near lattice is faces; the towers afar and the wires are lines, none of them among the faces.
        assert!(mesh.range(Group::Far).is_empty() && mesh.range(Group::Detail).is_empty());
        // The mapped tower's height, else the default.
        assert!((mesh.max[1] - (20.0 + 48.0)).abs() < 0.5, "{:?}", mesh.max);
        let w = wires(&mesh, Group::Detail);
        assert!(!w.is_empty());
        // Every wire sags below its ends and stays well over the ground.
        let lowest = w.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(
            lowest > 20.0 + 10.0 && lowest < 20.0 + 0.7 * 48.0,
            "{lowest}"
        );
        // The spans whose middles the chunk holds: (150..500), (500..900), and the first's from −200 is another's.
        assert!(w.iter().all(|p| p.x >= 150.0 - 10.0 && p.x <= 900.0 + 10.0));

        // Made far: the towers' legs and arms only.
        let mut far = MeshBuilder::new(false);
        draw(&mut far, &power, &ground);
        let far = far.finish();
        assert!(far.range(Group::Near).is_empty() && far.line_range(Group::Detail).is_empty());
        assert!(far.range(Group::Far).is_empty() && !far.line_range(Group::Far).is_empty());
        // Every line afar is a lattice's member, no wire.
        let lattice = far.indices[far.line_range(Group::Far).start as usize..]
            .iter()
            .all(|&i| far.vertices[i as usize].colour[3] == Material::Lattice as u8);
        assert!(lattice);
        assert!(
            far.triangles() * 4 < mesh.triangles(),
            "{} {}",
            far.triangles(),
            mesh.triangles()
        );
    }

    #[test]
    fn neighbouring_towers_hang_their_wires_alike() {
        let l = line(
            LineKind::Transmission,
            &[[0.0, 0.0], [300.0, 0.0], [600.0, 100.0]],
        );
        let s = supports(&l, &[], &|_| 0.0);
        assert_eq!(s.len(), 3);
        // Each attachment's partner on the next support lies on the same side of the line.
        for w in s.windows(2) {
            let (a, b) = (w[0].attachments(), w[1].attachments());
            for (p, q) in a.iter().zip(&b) {
                let side = |x: &Vec3, s: &Support| (Vec2::new(x.x, x.z) - s.at).dot(s.across);
                assert!(side(p, &w[0]) * side(q, &w[1]) >= -1e-3);
            }
        }
        // Where lines meet, one tower.
        let power = Power {
            lines: vec![
                l.clone(),
                line(LineKind::Transmission, &[[300.0, 0.0], [300.0, 400.0]]),
            ],
            masts: vec![],
        };
        assert_eq!(draw(&mut MeshBuilder::new(true), &power, &|_| 0.0), 4);
    }

    #[test]
    fn a_communication_tower_stands_as_tall_as_mapped() {
        let power = Power {
            lines: vec![],
            masts: vec![Mast {
                kind: MastKind::Communication,
                at: Vec2::new(400.0, 400.0),
                height: 150.0,
            }],
        };
        let mut m = MeshBuilder::new(true);
        assert_eq!(draw(&mut m, &power, &|_| 100.0), 1);
        let mesh = m.finish();
        assert!((mesh.max[1] - 250.0).abs() < 0.5);
    }
}
