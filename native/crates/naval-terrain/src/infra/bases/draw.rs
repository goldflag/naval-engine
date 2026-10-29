//! A generated base's pieces in a chunk's mesh ([`draw`]): each standing piece by the chunk holding its middle, each
//! strip's stretch by the chunk holding the stretch's middle and each hardstand by its middle's, so a base split across
//! chunks is drawn once. What lies on the ground (coral, the mat) goes into [`Group::Decal`]; what stands (berms, huts,
//! tents, the tower, drums, pits, vehicles) into [`Group::Structure`], at every range: a base is a few tens of thousands
//! of triangles.

use std::f32::consts::{PI, TAU};

use glam::{Vec2, Vec3};

use super::strip::{
    Bank, Board, Crates, DRUM_HEIGHT_M, DRUM_PITCH_M, Drums, Gun, Hut, HutKind, Look, Net, Pad,
    Pillbox, Pit, Plan, PoleLine, Revetment, Ribbon, Surface, Tank, Tent, Tower, Vehicle,
    VehicleKind, Wall,
};
use crate::buildings::CHUNK_M;
use crate::infra::mesh::{Block, Group, Material, MeshBuilder, Paint, faces};

const SIDE: f32 = CHUNK_M as f32;
/// A straight strip's stretches (m): the mat's planks repeat in 16 m (42 of 15 in), so every stretch starts on one.
const STRAIGHT_STEP: f32 = 16.0;
/// A curving strip's stretches at most (m).
const CURVE_STEP: f32 = 10.0;

/// A base about a chunk: its plan and its point in the chunk's metres.
#[derive(Clone, Copy, Debug)]
pub struct Placed<'a> {
    pub plan: &'a Plan,
    pub origin: Vec2,
}

fn owned(p: Vec2) -> bool {
    (0.0..SIDE).contains(&p.x) && (0.0..SIDE).contains(&p.y)
}

/// Draws the pieces of `base` the chunk holds; `ground` is the chunk's (chunk-local metres). Returns the standing
/// pieces drawn.
pub fn draw(m: &mut MeshBuilder, base: &Placed<'_>, ground: &dyn Fn(Vec2) -> f32) -> u32 {
    let plan = base.plan;
    let o = base.origin;
    // Nothing of it near the chunk.
    let (lo, hi) = (plan.min + o, plan.max + o);
    if hi.x < 0.0 || hi.y < 0.0 || lo.x >= SIDE || lo.y >= SIDE {
        return 0;
    }
    let group = m.group();
    m.set_group(Group::Decal);
    for (k, ribbon) in plan.ribbons.iter().enumerate() {
        draw_ribbon(m, ribbon, o, k as u32, ground);
    }
    for pad in &plan.pads {
        if owned(pad.centre + o) {
            draw_pad(m, pad, o, ground);
        }
    }
    m.set_group(Group::Structure);
    let mut pieces = 0;
    let mut each = |at: Vec2, m: &mut MeshBuilder, f: &mut dyn FnMut(&mut MeshBuilder)| {
        if owned(at + o) {
            f(m);
            pieces += 1;
        }
    };
    for r in &plan.revetments {
        each(r.centre, m, &mut |m| revetment(m, r, o, ground));
    }
    for h in &plan.huts {
        each(h.centre, m, &mut |m| hut(m, h, o, ground));
    }
    for t in &plan.tents {
        each(t.centre, m, &mut |m| tent(m, t, o, ground));
    }
    for t in &plan.towers {
        each(t.centre, m, &mut |m| tower(m, t, o, ground));
    }
    for d in &plan.drums {
        each(d.centre, m, &mut |m| drums(m, d, o, ground));
    }
    for c in &plan.crates {
        each(c.centre, m, &mut |m| crates(m, c, o, ground));
    }
    for w in &plan.walls {
        each((w.a + w.b) * 0.5, m, &mut |m| wall(m, w, o, ground));
    }
    for b in &plan.boards {
        each((b.a + b.b) * 0.5, m, &mut |m| board(m, b, o, ground));
    }
    for t in &plan.tanks {
        each(t.centre, m, &mut |m| tank(m, t, o, ground));
    }
    for line in &plan.poles {
        for (k, (p, _)) in line.poles.iter().enumerate() {
            each(*p, m, &mut |m| pole(m, line, k, o, ground));
        }
    }
    for n in &plan.nets {
        each(n.centre, m, &mut |m| net(m, n, o, ground));
    }
    for b in &plan.banks {
        each((b.a + b.b) * 0.5, m, &mut |m| bank(m, b, o, ground));
    }
    for b in &plan.pillboxes {
        each(b.centre, m, &mut |m| pillbox(m, b, o, ground));
    }
    for p in &plan.pits {
        each(p.centre, m, &mut |m| pit(m, p, o, ground));
    }
    for v in &plan.vehicles {
        each(v.centre, m, &mut |m| vehicle(m, v, o, ground));
    }
    m.set_group(group);
    m.counts.base_pieces += pieces;
    pieces
}

fn perp(v: Vec2) -> Vec2 {
    Vec2::new(-v.y, v.x)
}

fn up(p: Vec2, y: f32) -> Vec3 {
    Vec3::new(p.x, y, p.y)
}

/// A strip's line resampled: a straight one every [`STRAIGHT_STEP`] from its start, a curving one at most every
/// [`CURVE_STEP`]; with each point's distance along.
fn resample(r: &Ribbon) -> Vec<(Vec2, f32)> {
    let straight = r.points.len() == 2;
    let mut out: Vec<(Vec2, f32)> = Vec::new();
    let mut along = 0.0;
    for w in r.points.windows(2) {
        let length = w[0].distance(w[1]);
        if length < 1e-3 {
            continue;
        }
        let n = if straight {
            (length / STRAIGHT_STEP).ceil() as usize
        } else {
            (length / CURVE_STEP).ceil().max(1.0) as usize
        };
        for k in 0..n {
            let s = if straight {
                k as f32 * STRAIGHT_STEP
            } else {
                length * k as f32 / n as f32
            };
            out.push((w[0] + (w[1] - w[0]) * (s / length), along + s));
        }
        along += length;
    }
    if let Some(last) = r.points.last() {
        out.push((*last, along));
    }
    out
}

/// A strip's stretches the chunk holds, each cut into `across` strips that follow the ground, mitred at the bends.
fn draw_ribbon(m: &mut MeshBuilder, r: &Ribbon, o: Vec2, index: u32, ground: &dyn Fn(Vec2) -> f32) {
    let line = resample(r);
    if line.len() < 2 {
        return;
    }
    let dirs: Vec<Vec2> = line
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).normalize_or_zero())
        .collect();
    let offsets: Vec<Vec2> = (0..line.len())
        .map(|i| {
            let before = dirs[i.saturating_sub(1).min(dirs.len() - 1)];
            let after = dirs[i.min(dirs.len() - 1)];
            let mitre = (perp(before) + perp(after)).normalize_or(perp(after));
            let cos = mitre.dot(perp(after)).max(0.5);
            mitre / cos
        })
        .collect();
    // A runway's third byte its length in 16 m (where its touchdown areas lie from either end), another strip's a
    // seed.
    let length = (r.length() / 16.0).round().clamp(0.0, 255.0) as u8;
    let paint = match r.surface {
        Surface::Coral => Paint {
            colour: [
                r.look as u8,
                r.half.round().clamp(0.0, 255.0) as u8,
                if r.look == Look::Runway {
                    length
                } else {
                    (index.wrapping_mul(37) % 251) as u8
                },
            ],
            material: Material::Coral,
            param: r.paved.round().clamp(0.0, 127.0) as i8,
        },
        Surface::Marston => Paint {
            colour: [96, 0, length],
            material: Material::Marston,
            param: r.half.round().clamp(0.0, 127.0) as i8,
        },
    };
    let n = usize::from(r.across.max(1));
    let cut: Vec<f32> = (0..=n)
        .map(|j| -r.half + 2.0 * r.half * j as f32 / n as f32)
        .collect();
    for i in 0..line.len() - 1 {
        let (p0, s0) = line[i];
        let (p1, s1) = line[i + 1];
        if !owned((p0 + p1) * 0.5 + o) {
            continue;
        }
        let at = |p: Vec2, off: Vec2, c: f32| {
            let q = p + off * c + o;
            Vec3::new(q.x, ground(q) + r.lift, q.y)
        };
        for j in 0..n {
            let (c0, c1) = (cut[j], cut[j + 1]);
            m.quad(
                [
                    at(p0, offsets[i], c0),
                    at(p0, offsets[i], c1),
                    at(p1, offsets[i + 1], c1),
                    at(p1, offsets[i + 1], c0),
                ],
                [
                    Vec2::new(s0, c0),
                    Vec2::new(s0, c1),
                    Vec2::new(s1, c1),
                    Vec2::new(s1, c0),
                ],
                Vec3::Y,
                paint,
            );
        }
    }
}

/// A hardstand (a fan of triangles from its middle, so it follows the ground) or a lot (a grid of cells).
fn draw_pad(m: &mut MeshBuilder, pad: &Pad, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let at = |p: Vec2| up(p + o, ground(p + o) + pad.lift);
    let c = pad.centre;
    let reach = pad.reach();
    // `uv.y` how far in from the rim (the rim 0, the middle its reach), for the edge's fraying; the colour's second byte
    // the reach.
    let paint = Paint {
        colour: [
            pad.look as u8,
            reach.round().clamp(1.0, 255.0) as u8,
            (c.x.abs() as u32 % 251) as u8,
        ],
        material: Material::Coral,
        param: 0,
    };
    let n = pad.ring.len();
    if n < 3 {
        return;
    }
    // A ring halfway in, so a wide lot follows the ground.
    let inner: Vec<Vec2> = pad.ring.iter().map(|p| *p * 0.5).collect();
    let inset = |s: f32| Vec2::new(0.0, reach * (1.0 - s));
    for k in 0..n {
        let j = (k + 1) % n;
        m.face(
            &[at(c), at(c + inner[k]), at(c + inner[j])],
            &[inset(0.0), inset(0.5), inset(0.5)],
            Vec3::Y,
            paint,
        );
        m.quad(
            [
                at(c + inner[k]),
                at(c + pad.ring[k]),
                at(c + pad.ring[j]),
                at(c + inner[j]),
            ],
            [inset(0.5), inset(1.0), inset(1.0), inset(0.5)],
            Vec3::Y,
            paint,
        );
    }
}

/// The lowest and highest ground under `points` (chunk-local).
fn under(points: &[Vec2], ground: &dyn Fn(Vec2) -> f32) -> (f32, f32) {
    points
        .iter()
        .map(|p| ground(*p))
        .fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(h), b.max(h)))
}

/// A U of earth about a hardstand: a bank bulldozed up along its back and two arms, its corners cut off, its section a
/// trapezoid (sloped flanks, a narrow flat crest), each arm's end ramping down into the ground. Cut every 2.5 m along
/// and shaded smooth along it, so its flanks do not facet.
fn revetment(m: &mut MeshBuilder, r: &Revetment, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let f = r.facing;
    let c = perp(f);
    let reach = r.inner + r.berm;
    let back = 0.9 * r.inner + r.berm;
    let chamfer = 0.35 * reach;
    let jitter = |k: u32, s: u32| {
        crate::buildings::hash01(i64::from(r.seed), i64::from(k), i64::from(s)) * 2.0 - 1.0
    };
    // A horseshoe dug by hand is the rougher.
    let rough = if r.horseshoe { 2.2 } else { 1.0 };
    let [right, left] = r.arms.map(|a| a * r.inner);
    // The crest's line in (across, forward) about the hardstand: the right arm's end, its corner, the back, the left
    // corner, the left arm's end.
    let corners: Vec<Vec2> = if r.horseshoe {
        // A horseshoe: its arms straight, its back a half round about the middle.
        let mut c = vec![Vec2::new(reach, right), Vec2::new(reach, 0.0)];
        c.extend((1..10).map(|k| {
            let phi = PI * k as f32 / 10.0;
            Vec2::new(reach * phi.cos(), -reach * phi.sin())
        }));
        c.extend([Vec2::new(-reach, 0.0), Vec2::new(-reach, left)]);
        c
    } else {
        vec![
            Vec2::new(reach, right),
            Vec2::new(reach, -back + chamfer),
            Vec2::new(reach - chamfer, -back),
            Vec2::new(-(reach - chamfer), -back),
            Vec2::new(-reach, -back + chamfer),
            Vec2::new(-reach, left),
        ]
    };
    let mut line: Vec<Vec2> = Vec::new();
    for w in corners.windows(2) {
        let n = (w[0].distance(w[1]) / 2.5).ceil().max(1.0) as usize;
        line.extend((0..n).map(|k| w[0].lerp(w[1], k as f32 / n as f32)));
    }
    line.extend(corners.last());
    let centre = r.centre + o;
    let points: Vec<Vec2> = line
        .iter()
        .enumerate()
        .map(|(k, q)| {
            centre
                + c * q.x
                + f * q.y
                + Vec2::new(jitter(k as u32, 1), jitter(k as u32, 2)) * (0.25 * rough)
        })
        .collect();
    earthwork(
        m,
        &points,
        (r.height, r.berm, rough),
        r.colour,
        jitter(0, 7) * 3.0,
        ground,
    );
}

/// A straight bank of earth: a revetment's section along its line, its ends ramping down.
fn bank(m: &mut MeshBuilder, b: &Bank, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let (a, z) = (b.a + o, b.b + o);
    let jitter = |k: u32, s: u32| {
        crate::buildings::hash01(i64::from(b.seed), i64::from(k), i64::from(s)) * 2.0 - 1.0
    };
    let n = (a.distance(z) / 2.5).ceil().max(2.0) as usize;
    let points: Vec<Vec2> = (0..=n)
        .map(|k| {
            let t = k as f32 / n as f32;
            a.lerp(z, t) + Vec2::new(jitter(k as u32, 1), jitter(k as u32, 2)) * 0.2
        })
        .collect();
    earthwork(
        m,
        &points,
        (b.height, b.berm, 1.0),
        b.colour,
        jitter(0, 7) * 3.0,
        ground,
    );
}

/// Earth heaped along `points` (chunk-local): a trapezoid's section (sloped flanks, a crest 1.1 m wide), `shape` its
/// height, its half width at the foot and how rough it is, its height waving along it and each end ramping down
/// into the ground.
fn earthwork(
    m: &mut MeshBuilder,
    points: &[Vec2],
    (bank_height, berm, rough): (f32, f32, f32),
    colour: [u8; 3],
    phase: f32,
    ground: &dyn Fn(Vec2) -> f32,
) {
    let n = points.len();
    if n < 2 {
        return;
    }
    let mut along = Vec::with_capacity(n);
    let mut run = 0.0;
    for k in 0..n {
        if k > 0 {
            run += points[k].distance(points[k - 1]);
        }
        along.push(run);
    }
    let total = run;
    // Each end ramps down into the ground over this.
    let ramp = (1.8 * bank_height).min(0.3 * total);
    let paint = Paint::new(colour, Material::Earth);
    let top = 0.55;
    let mut rows: [Vec<Vec3>; 4] = Default::default();
    let mut normals: [Vec<Vec3>; 3] = Default::default();
    let mut heights = Vec::with_capacity(n);
    for k in 0..n {
        let p = points[k];
        let tangent =
            (points[(k + 1).min(n - 1)] - points[k.saturating_sub(1)]).normalize_or_zero();
        // Either way across: the section is the same both sides.
        let out = perp(tangent);
        let s = along[k];
        let taper = (s.min(total - s) / ramp).clamp(0.0, 1.0);
        let wave =
            1.0 + rough * (0.1 * (s * 0.21 + phase).sin() + 0.05 * (s * 0.57 + 2.0 * phase).sin());
        let height = (bank_height * wave * taper).max(0.05);
        let base = berm * (1.0 + rough * 0.08 * (s * 0.33 + phase).sin());
        let g = ground(p);
        let foot = |q: Vec2| up(q, ground(q) - 0.3);
        rows[0].push(foot(p + out * base));
        rows[1].push(up(p + out * top, g + height));
        rows[2].push(up(p - out * top, g + height));
        rows[3].push(foot(p - out * base));
        let run = (base - top).max(0.1);
        let o3 = Vec3::new(out.x, 0.0, out.y);
        normals[0].push((o3 * height + Vec3::Y * run).normalize());
        normals[1].push(Vec3::Y);
        normals[2].push((-o3 * height + Vec3::Y * run).normalize());
        heights.push(height);
    }
    for j in 0..3 {
        let (lo, hi) = (&rows[j], &rows[j + 1]);
        let uv = |row: &Vec<Vec3>, k: usize| {
            Vec2::new(along[k], row[k].y - rows[0][k].y.min(rows[3][k].y))
        };
        let uv_lo: Vec<Vec2> = (0..n).map(|k| uv(lo, k)).collect();
        let uv_hi: Vec<Vec2> = (0..n).map(|k| uv(hi, k)).collect();
        m.strip(lo, hi, &uv_lo, &uv_hi, &normals[j], paint);
    }
}

/// A Quonset hut (a half-round of corrugated iron on a plinth, plywood ends with a door and windows) or a frame
/// building (plank walls screened along their tops, a corrugated gable roof).
fn hut(m: &mut MeshBuilder, h: &Hut, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let a = h.axis;
    let c = perp(a);
    let centre = h.centre + o;
    let half_a = 0.5 * h.length;
    let corners = [
        centre - a * half_a - c * h.half,
        centre + a * half_a - c * h.half,
        centre + a * half_a + c * h.half,
        centre - a * half_a + c * h.half,
    ];
    let (lo, hi) = under(&corners, ground);
    let sides = faces::END_POS | faces::END_NEG | faces::SIDE_POS | faces::SIDE_NEG;
    // A hut on a plinth, a barracks on a skirt of planks, a thatched hut on stilts; a tent, a latrine and a hangar on
    // the ground.
    let floor = match h.kind {
        HutKind::Quonset
        | HutKind::Frame
        | HutKind::Barracks
        | HutKind::Brick
        | HutKind::Farmhouse
        | HutKind::WatchOffice
        | HutKind::Control => {
            let barracks = h.kind == HutKind::Barracks;
            let floor = hi + if barracks { 0.45 } else { 0.2 };
            let plinth = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: lo - 0.3,
                y1: floor,
            };
            let paint = if barracks {
                Paint::new([48, 44, 38], Material::Wood)
            } else {
                Paint::new([132, 128, 118], Material::Concrete)
            };
            m.block(&plinth, sides, paint);
            floor
        }
        HutKind::Thatch => {
            let floor = hi + 0.9;
            for (sa, sc) in [
                (-1.0f32, -1.0f32),
                (1.0, -1.0),
                (1.0, 1.0),
                (-1.0, 1.0),
                (0.0, -1.0),
                (0.0, 1.0),
            ] {
                let foot = centre + a * (sa * (half_a - 0.3)) + c * (sc * (h.half - 0.3));
                m.beam(
                    up(foot, ground(foot) - 0.2),
                    up(foot, floor),
                    0.09,
                    Paint::new([92, 80, 62], Material::Wood),
                );
            }
            let deck = Block {
                centre,
                axis: a,
                half_a: half_a + 0.2,
                half_c: h.half + 0.2,
                y0: floor - 0.15,
                y1: floor,
            };
            m.block(&deck, faces::ALL, Paint::new([96, 84, 64], Material::Wood));
            floor
        }
        _ => 0.5 * (lo + hi),
    };
    let a3 = Vec3::new(a.x, 0.0, a.y);
    let c3 = Vec3::new(c.x, 0.0, c.y);
    let wood = Paint::new(h.ends, Material::Wood);
    let door = Paint::new([62, 54, 42], Material::Wood);
    let glass = Paint::new([10, 12, 14], Material::Glass);
    // A flat piece on an end or side wall: its middle, the wall's outward and across directions, width and height.
    let panel =
        |m: &mut MeshBuilder, mid: Vec3, out: Vec3, across: Vec3, w: f32, ht: f32, paint: Paint| {
            let base = mid + out * 0.04;
            let (x, y) = (across * (0.5 * w), Vec3::Y * ht);
            m.quad(
                [base - x, base + x, base + x + y, base - x + y],
                [
                    Vec2::ZERO,
                    Vec2::new(w, 0.0),
                    Vec2::new(w, ht),
                    Vec2::new(0.0, ht),
                ],
                out,
                paint,
            );
        };
    match h.kind {
        HutKind::Quonset => {
            let iron = Paint::new(h.colour, Material::Iron);
            let segments = 9;
            let arc: Vec<(f32, f32)> = (0..=segments)
                .map(|j| {
                    let phi = PI * j as f32 / segments as f32;
                    (h.half * phi.cos(), h.height * phi.sin())
                })
                .collect();
            let at =
                |end: f32, (x, y): (f32, f32)| up(centre + a * (end * half_a) + c * x, floor + y);
            let mut s = 0.0;
            for j in 0..segments {
                let (p, q) = (arc[j], arc[j + 1]);
                let ds = Vec2::new(q.0 - p.0, q.1 - p.1).length();
                let phi = PI * (j as f32 + 0.5) / segments as f32;
                let out = c3 * phi.cos() + Vec3::Y * phi.sin();
                m.quad(
                    [at(-1.0, p), at(1.0, p), at(1.0, q), at(-1.0, q)],
                    [
                        Vec2::new(0.0, s),
                        Vec2::new(h.length, s),
                        Vec2::new(h.length, s + ds),
                        Vec2::new(0.0, s + ds),
                    ],
                    out,
                    iron,
                );
                s += ds;
            }
            for end in [-1.0f32, 1.0] {
                let points: Vec<Vec3> = arc.iter().map(|p| at(end, *p)).collect();
                let uv: Vec<Vec2> = arc.iter().map(|(x, y)| Vec2::new(*x, *y)).collect();
                m.face(&points, &uv, a3 * end, wood);
                let mid = up(centre + a * (end * half_a), floor);
                panel(m, mid, a3 * end, c3, 1.3, 2.1, door);
                for side in [-1.0f32, 1.0] {
                    let w = mid + c3 * (side * 1.9) + Vec3::Y * 1.0;
                    panel(m, w, a3 * end, c3, 0.8, 0.7, glass);
                }
            }
        }
        HutKind::Barracks | HutKind::Thatch => {
            let thatch = h.kind == HutKind::Thatch;
            let wall = if thatch { 1.45 } else { 2.3 };
            let walls_paint = if thatch {
                Paint::new(h.ends, Material::Canvas)
            } else {
                Paint::new(h.ends, Material::Wood)
            };
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: floor,
                y1: floor + wall,
            };
            m.block(&walls, sides, walls_paint);
            let dark = Paint::new([26, 24, 21], Material::Paint);
            if thatch {
                // A doorway on a long side.
                panel(m, up(centre + c * h.half, floor), c3, a3, 0.9, 1.35, dark);
            } else {
                // Shutters propped open down its sides, the dark of the room behind; a door at each end.
                let n = ((h.length - 2.0) / 3.2).floor().max(1.0) as i32;
                for side in [-1.0f32, 1.0] {
                    for k in 0..n {
                        let u = (k as f32 - 0.5 * (n - 1) as f32) * 3.2;
                        let mid = up(centre + a * u + c * (side * h.half), floor + 0.95);
                        panel(m, mid, c3 * side, a3, 1.1, 0.8, dark);
                    }
                }
                for end in [-1.0f32, 1.0] {
                    let mid = up(centre + a * (end * half_a), floor);
                    panel(m, mid, a3 * end, c3, 1.2, 1.9, door);
                }
            }
            let eave = floor + wall - 0.05;
            let (ridge, over) = if thatch {
                (eave + 1.2 * h.half, Vec2::new(0.6, 0.85))
            } else {
                (eave + 0.5 * h.half, Vec2::new(0.45, 0.6))
            };
            let roof = if thatch {
                Paint::new(h.colour, Material::Canvas)
            } else {
                Paint::new(h.colour, Material::Wood)
            };
            gable_roof(
                m,
                (centre, a),
                (half_a, h.half),
                (eave, ridge),
                over,
                (roof, walls_paint),
                [true, true],
            );
        }
        HutKind::Hangar => {
            // Open at its end toward its axis: the dark inside there, a gable over the opening.
            let eave = lo + 0.62 * h.height;
            let walls_paint = Paint::new(h.ends, Material::Wood);
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: lo - 0.2,
                y1: eave,
            };
            m.block(
                &walls,
                faces::END_NEG | faces::SIDE_POS | faces::SIDE_NEG,
                walls_paint,
            );
            let inside = up(centre + a * (half_a - 0.6), lo - 0.1);
            panel(
                m,
                inside,
                a3,
                c3,
                2.0 * h.half - 0.8,
                eave - lo - 0.3,
                Paint::new([14, 14, 12], Material::Paint),
            );
            gable_roof(
                m,
                (centre, a),
                (half_a, h.half),
                (eave, lo + h.height),
                Vec2::new(0.8, 0.6),
                (Paint::new(h.colour, Material::Iron), walls_paint),
                [true, true],
            );
        }
        HutKind::T2 | HutKind::Hall => {
            // A T2: steel walls, doors across both ends, a low gable. A Luftwaffe hall: rendered or brick walls, its
            // doors across its long side toward `c`, a broader gable.
            let t2 = h.kind == HutKind::T2;
            let eave = lo + if t2 { 0.78 } else { 0.72 } * h.height;
            let ridge = lo + h.height;
            let (wall_paint, roof_paint) = if t2 {
                (
                    Paint::new(h.ends, Material::Iron),
                    Paint::new(h.colour, Material::Iron),
                )
            } else {
                (
                    Paint::new(h.ends, Material::Concrete),
                    Paint::new(h.colour, Material::Wood),
                )
            };
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: lo - 0.2,
                y1: eave,
            };
            m.block(&walls, sides, wall_paint);
            let doors = Paint::new([54, 56, 54], Material::Iron);
            if t2 {
                for end in [-1.0f32, 1.0] {
                    let mid = up(centre + a * (end * half_a), lo - 0.1);
                    panel(
                        m,
                        mid,
                        a3 * end,
                        c3,
                        2.0 * h.half - 2.0,
                        eave - lo - 0.6,
                        doors,
                    );
                }
            } else {
                let mid = up(centre + c * h.half, lo - 0.1);
                panel(m, mid, c3, a3, h.length - 4.0, eave - lo - 1.2, doors);
                // A row of windows high along its back.
                panel(
                    m,
                    up(centre - c * h.half, eave - 2.2),
                    -c3,
                    a3,
                    h.length - 3.0,
                    1.2,
                    Paint::new([20, 22, 24], Material::Glass),
                );
            }
            gable_roof(
                m,
                (centre, a),
                (half_a, h.half),
                (eave, ridge),
                Vec2::new(0.5, 0.6),
                (roof_paint, wall_paint),
                [true, true],
            );
        }
        HutKind::Blister => {
            // An arch of steel to the ground along its axis, open at both ends, the dark inside there.
            let iron = Paint::new(h.colour, Material::Iron);
            let segments = 10;
            let arc: Vec<(f32, f32)> = (0..=segments)
                .map(|j| {
                    let phi = PI * j as f32 / segments as f32;
                    (h.half * phi.cos(), h.height * phi.sin())
                })
                .collect();
            let at = |end: f32, (x, y): (f32, f32)| {
                up(centre + a * (end * half_a) + c * x, lo - 0.2 + y)
            };
            let mut s = 0.0;
            for j in 0..segments {
                let (p, q) = (arc[j], arc[j + 1]);
                let ds = Vec2::new(q.0 - p.0, q.1 - p.1).length();
                let phi = PI * (j as f32 + 0.5) / segments as f32;
                let out = c3 * phi.cos() + Vec3::Y * phi.sin();
                m.quad(
                    [at(-1.0, p), at(1.0, p), at(1.0, q), at(-1.0, q)],
                    [
                        Vec2::new(0.0, s),
                        Vec2::new(h.length, s),
                        Vec2::new(h.length, s + ds),
                        Vec2::new(0.0, s + ds),
                    ],
                    out,
                    iron,
                );
                s += ds;
            }
            let dark = Paint::new([16, 16, 14], Material::Paint);
            for end in [-1.0f32, 1.0] {
                let inset = end * (half_a - 1.5);
                let points: Vec<Vec3> = arc
                    .iter()
                    .map(|(x, y)| up(centre + a * inset + c * (*x * 0.96), lo - 0.2 + y * 0.96))
                    .collect();
                let uv: Vec<Vec2> = arc.iter().map(|(x, y)| Vec2::new(*x, *y)).collect();
                m.face(&points, &uv, a3 * end, dark);
            }
        }
        HutKind::Brick | HutKind::Farmhouse => {
            // Brick or rendered walls, small windows down its sides, a door; a pitched roof (a farmhouse's steep,
            // of red tiles).
            let farm = h.kind == HutKind::Farmhouse;
            let wall = if farm { 3.0 } else { 3.4 };
            let walls_paint = Paint::new(h.ends, Material::Concrete);
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: floor,
                y1: floor + wall,
            };
            m.block(&walls, sides, walls_paint);
            let glass = Paint::new([22, 24, 26], Material::Glass);
            let n = ((h.length - 2.0) / 3.0).floor().max(1.0) as i32;
            for side in [-1.0f32, 1.0] {
                for k in 0..n {
                    let u = (k as f32 - 0.5 * (n - 1) as f32) * 3.0;
                    let mid = up(centre + a * u + c * (side * h.half), floor + 1.0);
                    panel(
                        m,
                        mid,
                        c3 * side,
                        a3,
                        if farm { 0.8 } else { 1.4 },
                        1.1,
                        glass,
                    );
                }
            }
            panel(m, up(centre + a * half_a, floor), a3, c3, 1.2, 2.1, door);
            let eave = floor + wall - 0.05;
            let ridge = eave + if farm { 1.15 } else { 0.6 } * h.half;
            gable_roof(
                m,
                (centre, a),
                (half_a, h.half),
                (eave, ridge),
                Vec2::new(0.35, 0.5),
                (Paint::new(h.colour, Material::Wood), walls_paint),
                [true, true],
            );
        }
        HutKind::WatchOffice | HutKind::Control => {
            // Two storeys under a flat roof: a watch office rendered, its windows wide, a railed balcony across its
            // front and a rail round its roof; a control building of brick with a glazed room on its roof.
            let watch = h.kind == HutKind::WatchOffice;
            let walls_paint = Paint::new(h.ends, Material::Concrete);
            let top = floor + 6.4;
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: floor,
                y1: top,
            };
            m.block(&walls, sides | faces::TOP, walls_paint);
            let glass = Paint::new([20, 22, 26], Material::Glass);
            for storey in [0.0f32, 3.2] {
                for (out, across, w) in [
                    (a3, c3, 2.0 * h.half),
                    (-a3, c3, 2.0 * h.half),
                    (c3, a3, h.length),
                    (-c3, a3, h.length),
                ] {
                    let face = up(centre, floor)
                        + out
                            * if out == a3 || out == -a3 {
                                half_a
                            } else {
                                h.half
                            };
                    let wide = if watch { 0.7 } else { 0.45 };
                    panel(
                        m,
                        face + Vec3::Y * (storey + 1.0),
                        out,
                        across,
                        w * wide,
                        1.4,
                        glass,
                    );
                }
            }
            let rail = Paint::new([60, 62, 60], Material::Steel);
            // The rails: posts and a top rail along each side of a rectangle at `y`, `reach` out from the middle.
            let railing =
                |m: &mut MeshBuilder, y: f32, ra: f32, rc: f32, sides: &[(Vec2, Vec2)]| {
                    for &(p, q) in sides {
                        let length = p.distance(q);
                        let posts = (length / 1.6).ceil().max(1.0) as usize;
                        for k in 0..=posts {
                            let at = p.lerp(q, k as f32 / posts as f32);
                            m.beam(up(at, y), up(at, y + 1.05), 0.03, rail);
                        }
                        m.beam(up(p, y + 1.05), up(q, y + 1.05), 0.03, rail);
                        m.beam(up(p, y + 0.5), up(q, y + 0.5), 0.02, rail);
                    }
                    let _ = (ra, rc);
                };
            let corner =
                |sa: f32, sc: f32, ra: f32, rc: f32| centre + a * (sa * ra) + c * (sc * rc);
            if watch {
                let (ra, rc) = (half_a - 0.1, h.half - 0.1);
                let ring = [
                    (corner(-1.0, -1.0, ra, rc), corner(1.0, -1.0, ra, rc)),
                    (corner(1.0, -1.0, ra, rc), corner(1.0, 1.0, ra, rc)),
                    (corner(1.0, 1.0, ra, rc), corner(-1.0, 1.0, ra, rc)),
                    (corner(-1.0, 1.0, ra, rc), corner(-1.0, -1.0, ra, rc)),
                ];
                railing(m, top, ra, rc, &ring);
                // The balcony across its front (toward `axis`), at the first floor.
                let slab = Block {
                    centre: centre + a * (half_a + 0.75),
                    axis: a,
                    half_a: 0.75,
                    half_c: h.half,
                    y0: floor + 3.0,
                    y1: floor + 3.2,
                };
                m.block(&slab, faces::ALL, walls_paint);
                let front = [(
                    corner(1.0, -1.0, half_a + 1.45, h.half),
                    corner(1.0, 1.0, half_a + 1.45, h.half),
                )];
                railing(m, floor + 3.2, half_a, h.half, &front);
            } else {
                let room = Block {
                    centre,
                    axis: a,
                    half_a: 0.55 * half_a,
                    half_c: 0.55 * h.half,
                    y0: top,
                    y1: top + 2.6,
                };
                m.block(&room, faces::SHELL, glass);
                let roof = Block {
                    centre,
                    axis: a,
                    half_a: 0.55 * half_a + 0.4,
                    half_c: 0.55 * h.half + 0.4,
                    y0: top + 2.6,
                    y1: top + 2.85,
                };
                m.block(&roof, faces::ALL, walls_paint);
            }
        }
        HutKind::Latrine => {
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: lo - 0.1,
                y1: floor + 2.0,
            };
            m.block(&walls, sides, Paint::new(h.ends, Material::Wood));
            panel(m, up(centre + c * h.half, floor), c3, a3, 0.8, 1.9, door);
            // Its lean-to roof, high over the door's side.
            let p = |x: f32, z: f32, y: f32| up(centre + a * x + c * z, y);
            let (ra, rc) = (half_a + 0.25, h.half + 0.3);
            m.quad(
                [
                    p(-ra, rc, floor + 2.3),
                    p(ra, rc, floor + 2.3),
                    p(ra, -rc, floor + 1.95),
                    p(-ra, -rc, floor + 1.95),
                ],
                [
                    Vec2::ZERO,
                    Vec2::new(2.0 * ra, 0.0),
                    Vec2::new(2.0 * ra, 2.0 * rc),
                    Vec2::new(0.0, 2.0 * rc),
                ],
                Vec3::Y,
                Paint::new(h.colour, Material::Iron),
            );
        }
        HutKind::Frame | HutKind::Ward => {
            let ward = h.kind == HutKind::Ward;
            let wall = if ward { 1.7 } else { 2.6 };
            let wall_paint = if ward {
                Paint::new(h.colour, Material::Canvas)
            } else {
                Paint::new(h.ends, Material::Wood)
            };
            let walls = Block {
                centre,
                axis: a,
                half_a,
                half_c: h.half,
                y0: if ward { lo - 0.1 } else { floor },
                y1: floor + wall,
            };
            m.block(&walls, sides, wall_paint);
            // A frame building's screens along its long walls' tops.
            for side in [-1.0f32, 1.0] {
                if ward {
                    break;
                }
                let mid = up(centre + c * (side * h.half), floor + 1.2);
                panel(
                    m,
                    mid,
                    c3 * side,
                    a3,
                    h.length - 1.2,
                    1.1,
                    Paint::new([40, 44, 40], Material::Paint),
                );
            }
            for end in [-1.0f32, 1.0] {
                let mid = up(centre + a * (end * half_a), floor);
                panel(m, mid, a3 * end, c3, 1.4, 2.1, door);
            }
            // The roof: two planes from the eaves (overhanging) to the ridge, the gables' triangles.
            let eave = floor + wall - 0.05;
            let ridge = if ward {
                floor + h.height
            } else {
                floor + wall + 0.45 * h.half
            };
            let (ra, rc) = if ward {
                (half_a + 0.1, h.half + 0.15)
            } else {
                (half_a + 0.45, h.half + 0.5)
            };
            let iron = if ward {
                Paint::new(h.ends, Material::Canvas)
            } else {
                Paint::new(h.colour, Material::Iron)
            };
            let p = |x: f32, z: f32, y: f32| up(centre + a * x + c * z, y);
            let slope = (rc * rc + (ridge - eave) * (ridge - eave)).sqrt();
            for side in [-1.0f32, 1.0] {
                m.quad(
                    [
                        p(-ra, side * rc, eave),
                        p(ra, side * rc, eave),
                        p(ra, 0.0, ridge),
                        p(-ra, 0.0, ridge),
                    ],
                    [
                        Vec2::new(0.0, 0.0),
                        Vec2::new(2.0 * ra, 0.0),
                        Vec2::new(2.0 * ra, slope),
                        Vec2::new(0.0, slope),
                    ],
                    c3 * side + Vec3::Y,
                    iron,
                );
                // The roof's underside at the eaves (seen from below the overhang).
                let gable = [
                    p(side * half_a, -h.half, eave),
                    p(side * half_a, h.half, eave),
                    p(side * half_a, 0.0, ridge - 0.1),
                ];
                m.face(
                    &gable,
                    &[
                        Vec2::new(0.0, 0.0),
                        Vec2::new(2.0 * h.half, 0.0),
                        Vec2::new(h.half, ridge - eave),
                    ],
                    a3 * side,
                    wall_paint,
                );
            }
        }
    }
}

/// A gabled roof over a box about `centre` along `a` (its half length and half width): two planes from the eaves to the
/// ridge (`heights`), overhanging by `over` (along, across), and under it at each end asked for (`ends`: −a, +a) the
/// gable's triangle (`paints`: the roof's, the gables').
fn gable_roof(
    m: &mut MeshBuilder,
    (centre, a): (Vec2, Vec2),
    (half_a, half): (f32, f32),
    (eave, ridge): (f32, f32),
    over: Vec2,
    (roof, gable): (Paint, Paint),
    ends: [bool; 2],
) {
    let c = perp(a);
    let a3 = Vec3::new(a.x, 0.0, a.y);
    let c3 = Vec3::new(c.x, 0.0, c.y);
    let p = |x: f32, z: f32, y: f32| up(centre + a * x + c * z, y);
    let (ra, rc) = (half_a + over.x, half + over.y);
    // The eaves drop with the overhang, the roof's pitch carried on.
    let pitch = (ridge - eave) / half.max(0.1);
    let low = eave - over.y * pitch;
    let slope = (rc * rc + (ridge - low) * (ridge - low)).sqrt();
    for side in [-1.0f32, 1.0] {
        m.quad(
            [
                p(-ra, side * rc, low),
                p(ra, side * rc, low),
                p(ra, 0.0, ridge),
                p(-ra, 0.0, ridge),
            ],
            [
                Vec2::new(0.0, 0.0),
                Vec2::new(2.0 * ra, 0.0),
                Vec2::new(2.0 * ra, slope),
                Vec2::new(0.0, slope),
            ],
            c3 * side * pitch.max(0.2) + Vec3::Y,
            roof,
        );
    }
    for (k, end) in [-1.0f32, 1.0].into_iter().enumerate() {
        if !ends[k] {
            continue;
        }
        m.face(
            &[
                p(end * half_a, -half, eave),
                p(end * half_a, half, eave),
                p(end * half_a, 0.0, ridge - 0.08),
            ],
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(2.0 * half, 0.0),
                Vec2::new(half, ridge - eave),
            ],
            a3 * end,
            gable,
        );
    }
}

/// A pyramidal tent: canvas walls (dark where rolled up) and a canvas pyramid on its pole.
fn tent(m: &mut MeshBuilder, t: &Tent, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let a = t.axis;
    let c = perp(a);
    let centre = t.centre + o;
    let corners = [
        centre - a * t.half - c * t.half,
        centre + a * t.half - c * t.half,
        centre + a * t.half + c * t.half,
        centre - a * t.half + c * t.half,
    ];
    let (lo, hi) = under(&corners, ground);
    let mean = 0.5 * (lo + hi);
    let eave = mean + t.wall;
    let canvas = Paint::new(t.colour, Material::Canvas);
    let wall = if t.rolled {
        Paint::new([34, 34, 28], Material::Paint)
    } else {
        canvas
    };
    let block = Block {
        centre,
        axis: a,
        half_a: t.half,
        half_c: t.half,
        y0: lo - 0.15,
        y1: eave,
    };
    let sides = faces::END_POS | faces::END_NEG | faces::SIDE_POS | faces::SIDE_NEG;
    m.block(&block, sides, wall);
    // The roof from the eaves (a hand's overhang) to the peak.
    let over = t.half + 0.2;
    let rim = [
        centre - a * over - c * over,
        centre + a * over - c * over,
        centre + a * over + c * over,
        centre - a * over + c * over,
    ];
    let peak = up(centre, mean + t.peak);
    let slant = (over * over + (t.peak - t.wall).powi(2)).sqrt();
    for k in 0..4 {
        let (p, q) = (rim[k], rim[(k + 1) % 4]);
        let mid = (p + q) * 0.5 - centre;
        m.face(
            &[up(p, eave - 0.1), up(q, eave - 0.1), peak],
            &[
                Vec2::new(0.0, 0.0),
                Vec2::new(2.0 * over, 0.0),
                Vec2::new(over, slant),
            ],
            Vec3::new(mid.x, over, mid.y),
            canvas,
        );
    }
}

/// A wooden control tower: four legs braced across each face, a glazed cab under a flat roof, a stair up one side.
fn tower(m: &mut MeshBuilder, t: &Tower, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    if t.lookout {
        lookout(m, t, o, ground);
        return;
    }
    let a = t.axis;
    let c = perp(a);
    let centre = t.centre + o;
    let g = ground(centre);
    let top = g + t.height;
    let wood = Paint::new([128, 114, 92], Material::Wood);
    let leg = |sa: f32, sc: f32, w: f32| centre + a * (sa * w) + c * (sc * w);
    let feet = [
        leg(-1.0, -1.0, 2.0),
        leg(1.0, -1.0, 2.0),
        leg(1.0, 1.0, 2.0),
        leg(-1.0, 1.0, 2.0),
    ];
    let heads = [
        leg(-1.0, -1.0, 1.7),
        leg(1.0, -1.0, 1.7),
        leg(1.0, 1.0, 1.7),
        leg(-1.0, 1.0, 1.7),
    ];
    let at = |k: usize, s: f32| {
        let p = feet[k].lerp(heads[k], s);
        up(
            p,
            (ground(feet[k]) - 0.3) + (top - ground(feet[k]) + 0.3) * s,
        )
    };
    for k in 0..4 {
        m.beam(at(k, 0.0), at(k, 1.0), 0.13, wood);
        let j = (k + 1) % 4;
        // An X on each face in two storeys, a girt between.
        for (s0, s1) in [(0.08, 0.5), (0.5, 0.92)] {
            m.beam(at(k, s0), at(j, s1), 0.06, wood);
            m.beam(at(j, s0), at(k, s1), 0.06, wood);
            m.beam(at(k, s1), at(j, s1), 0.07, wood);
        }
    }
    let cab = Block {
        centre,
        axis: a,
        half_a: 2.2,
        half_c: 2.2,
        y0: top,
        y1: top + 2.6,
    };
    m.block(&cab, faces::ALL & !faces::TOP, wood);
    let a3 = Vec3::new(a.x, 0.0, a.y);
    let c3 = Vec3::new(c.x, 0.0, c.y);
    let glass = Paint::new([10, 12, 14], Material::Glass);
    for (out, across) in [(a3, c3), (-a3, c3), (c3, a3), (-c3, a3)] {
        let base = up(centre, top + 1.1) + out * 2.24;
        let (x, y) = (across * 1.9, Vec3::Y * 1.2);
        m.quad(
            [base - x, base + x, base + x + y, base - x + y],
            [
                Vec2::ZERO,
                Vec2::new(3.8, 0.0),
                Vec2::new(3.8, 1.2),
                Vec2::new(0.0, 1.2),
            ],
            out,
            glass,
        );
    }
    let roof = Block {
        centre,
        axis: a,
        half_a: 2.9,
        half_c: 2.9,
        y0: top + 2.6,
        y1: top + 2.8,
    };
    m.block(
        &roof,
        faces::ALL,
        Paint::new([120, 122, 118], Material::Iron),
    );
    // The stair: a flight from the ground to the cab's floor along one side.
    let foot = centre + c * 2.6 - a * 4.5;
    let head = centre + c * 2.6 + a * 1.2;
    m.beam(up(foot, ground(foot) - 0.1), up(head, top), 0.45, wood);
}

/// A lookout: four slim legs braced in three storeys, an open platform with a rail, a little hipped roof on posts over
/// it, a ladder up one side.
fn lookout(m: &mut MeshBuilder, t: &Tower, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let a = t.axis;
    let c = perp(a);
    let centre = t.centre + o;
    let g = ground(centre);
    let top = g + t.height;
    let wood = Paint::new([104, 92, 72], Material::Wood);
    let leg = |sa: f32, sc: f32, w: f32| centre + a * (sa * w) + c * (sc * w);
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    let feet = corners.map(|(sa, sc)| leg(sa, sc, 1.9));
    let heads = corners.map(|(sa, sc)| leg(sa, sc, 1.05));
    let at = |k: usize, s: f32| {
        let p = feet[k].lerp(heads[k], s);
        up(
            p,
            (ground(feet[k]) - 0.3) + (top - ground(feet[k]) + 0.3) * s,
        )
    };
    for k in 0..4 {
        m.beam(at(k, 0.0), at(k, 1.0), 0.1, wood);
        let j = (k + 1) % 4;
        for (s0, s1) in [(0.05, 0.36), (0.36, 0.68), (0.68, 0.97)] {
            m.beam(at(k, s0), at(j, s1), 0.045, wood);
            m.beam(at(k, s1), at(j, s1), 0.05, wood);
        }
    }
    let deck = Block {
        centre,
        axis: a,
        half_a: 1.5,
        half_c: 1.5,
        y0: top,
        y1: top + 0.15,
    };
    m.block(&deck, faces::ALL, wood);
    for (dir, across) in [(a, c), (-a, c), (c, a), (-c, a)] {
        let rail = Block {
            centre: centre + dir * 1.45,
            axis: across,
            half_a: 1.5,
            half_c: 0.04,
            y0: top + 0.15,
            y1: top + 1.05,
        };
        m.block(&rail, faces::SHELL, wood);
    }
    let roof_y = top + 2.3;
    for (sa, sc) in corners {
        let p = leg(sa, sc, 1.35);
        m.beam(up(p, top + 0.1), up(p, roof_y), 0.05, wood);
    }
    let rim = corners.map(|(sa, sc)| leg(sa, sc, 1.9));
    let peak = up(centre, roof_y + 0.9);
    let thatch = Paint::new([120, 104, 70], Material::Canvas);
    for k in 0..4 {
        let (p, q) = (rim[k], rim[(k + 1) % 4]);
        let mid = (p + q) * 0.5 - centre;
        m.face(
            &[up(p, roof_y - 0.1), up(q, roof_y - 0.1), peak],
            &[Vec2::ZERO, Vec2::new(3.8, 0.0), Vec2::new(1.9, 2.1)],
            Vec3::new(mid.x, 1.9, mid.y),
            thatch,
        );
    }
    // The ladder up the face toward its axis.
    let foot = centre + a * 2.6;
    m.beam(
        up(foot, ground(foot) - 0.1),
        up(centre + a * 1.5, top),
        0.22,
        wood,
    );
}

/// Camouflage over a revetment: netting sagging between its poles and draped to the ground at its edges, or palm
/// fronds laid every way over the plane.
fn net(m: &mut MeshBuilder, n: &Net, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let a = n.axis.normalize_or(Vec2::X);
    let c = perp(a);
    let centre = n.centre + o;
    let g = ground(centre);
    let rand = |k: u32| crate::buildings::hash01(i64::from(n.seed), i64::from(k), 23) * 2.0 - 1.0;
    if n.fronds {
        for k in 0..11u32 {
            let at =
                centre + a * (rand(k) * n.half.x * 0.85) + c * (rand(k + 40) * n.half.y * 0.85);
            let d = Vec2::from_angle(rand(k + 80) * PI);
            let length = 3.4 + rand(k + 120) * 0.9;
            let h = g + n.height + rand(k + 160) * 0.45;
            let side = Vec3::new(-d.y, 0.0, d.x) * 0.5;
            let base = up(at - d * (0.5 * length), h - 0.45);
            let mid = up(at + d * (0.05 * length), h + 0.05);
            let tip = up(at + d * (0.5 * length), h - 0.6);
            let dry = 0.5 + 0.5 * rand(k + 200);
            let colour = [
                (n.colour[0] as f32 + 40.0 * dry) as u8,
                (n.colour[1] as f32 + 18.0 * dry) as u8,
                (n.colour[2] as f32 + 12.0 * dry) as u8,
            ];
            let leaf = Paint::new(colour, Material::Canvas);
            m.quad(
                [base - side * 0.3, base + side * 0.3, mid + side, mid - side],
                [
                    Vec2::ZERO,
                    Vec2::new(1.0, 0.0),
                    Vec2::new(1.0, 2.0),
                    Vec2::new(0.0, 2.0),
                ],
                Vec3::Y,
                leaf,
            );
            m.face(
                &[mid - side, mid + side, tip],
                &[Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(0.5, 2.0)],
                Vec3::Y,
                leaf,
            );
        }
        return;
    }
    // Netting garnished with leaves: the bank's earth shading (mottled, green where it faces up), not cloth.
    let paint = Paint::new(n.colour, Material::Earth);
    let steps = 6;
    let point = |i: usize, j: usize| {
        let u = i as f32 / steps as f32 * 2.0 - 1.0;
        let v = j as f32 / steps as f32 * 2.0 - 1.0;
        let k = (i * 13 + j) as u32;
        let p = centre
            + a * (u * n.half.x)
            + c * (v * n.half.y)
            + Vec2::new(rand(k), rand(k + 300)) * 0.35;
        // Held up on its poles, sagging between them, draped down at its rim.
        let rim = u.abs().max(v.abs()).powi(4);
        let sag = 0.35 * (1.0 - (u * PI).cos().abs()) * (1.0 - (v * PI).cos().abs());
        let y = g + n.height * (1.0 - 0.7 * rim) - sag + rand(k + 600) * 0.12;
        up(p, y.max(ground(p) + 0.2))
    };
    for i in 0..steps {
        for j in 0..steps {
            let q = [
                point(i, j),
                point(i + 1, j),
                point(i + 1, j + 1),
                point(i, j + 1),
            ];
            // `uv.y` high: no foot of trodden earth.
            let uv = [
                Vec2::new(i as f32, 5.0),
                Vec2::new(i as f32 + 1.0, 5.0),
                Vec2::new(i as f32 + 1.0, 5.0),
                Vec2::new(i as f32, 5.0),
            ];
            m.quad(q, uv, Vec3::Y, paint);
        }
    }
    // Its poles: one in the middle, four about it.
    let wood = Paint::new([96, 84, 64], Material::Wood);
    for (u, v) in [
        (0.0f32, 0.0f32),
        (0.5, 0.5),
        (-0.5, 0.5),
        (0.5, -0.5),
        (-0.5, -0.5),
    ] {
        let p = centre + a * (u * n.half.x) + c * (v * n.half.y);
        let top = point(
            ((u + 1.0) * 0.5 * steps as f32).round() as usize,
            ((v + 1.0) * 0.5 * steps as f32).round() as usize,
        );
        m.beam(up(p, ground(p) - 0.2), top, 0.06, wood);
    }
}

/// A block of standing fuel drums.
fn drums(m: &mut MeshBuilder, d: &Drums, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let centre = d.centre + o;
    let (ha, hc) = (
        0.5 * f32::from(d.along) * DRUM_PITCH_M,
        0.5 * f32::from(d.across) * DRUM_PITCH_M,
    );
    let c = perp(d.axis);
    let corners = [
        centre - d.axis * ha - c * hc,
        centre + d.axis * ha - c * hc,
        centre + d.axis * ha + c * hc,
        centre - d.axis * ha + c * hc,
    ];
    let (lo, _) = under(&corners, ground);
    let mean = corners.iter().map(|p| ground(*p)).sum::<f32>() / 4.0;
    let block = Block {
        centre,
        axis: d.axis,
        half_a: ha,
        half_c: hc,
        y0: lo - 0.05,
        y1: mean + f32::from(d.tiers) * DRUM_HEIGHT_M,
    };
    m.block(
        &block,
        faces::SHELL,
        Paint::new([d.seed, d.tiers, 0], Material::Drums),
    );
}

/// A stack of crates: a box of planks, its boards across it.
fn crates(m: &mut MeshBuilder, c: &Crates, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let centre = c.centre + o;
    let side = perp(c.axis);
    let corners = [
        centre - c.axis * c.half.x - side * c.half.y,
        centre + c.axis * c.half.x - side * c.half.y,
        centre + c.axis * c.half.x + side * c.half.y,
        centre - c.axis * c.half.x + side * c.half.y,
    ];
    let (lo, hi) = under(&corners, ground);
    let block = Block {
        centre,
        axis: c.axis,
        half_a: c.half.x,
        half_c: c.half.y,
        y0: lo - 0.05,
        y1: hi + c.height,
    };
    m.block(&block, faces::SHELL, Paint::new(c.colour, Material::Wood));
}

/// A sandbag wall: courses of bags along its line.
fn wall(m: &mut MeshBuilder, w: &Wall, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let (a, b) = (w.a + o, w.b + o);
    let axis = (b - a).normalize_or(Vec2::X);
    let (lo, hi) = under(&[a, b, (a + b) * 0.5], ground);
    let block = Block {
        centre: (a + b) * 0.5,
        axis,
        half_a: 0.5 * a.distance(b),
        half_c: 0.4,
        y0: lo - 0.2,
        y1: 0.5 * (lo + hi) + w.height,
    };
    let bags = Paint {
        colour: [150, 132, 98],
        material: Material::Earth,
        param: 1,
    };
    m.block(&block, faces::SHELL, bags);
}

/// A duckboard walk: slats across it, on the ground in lengths of 6 m.
fn board(m: &mut MeshBuilder, b: &Board, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let (a, z) = (b.a + o, b.b + o);
    let along = (z - a).normalize_or(Vec2::X);
    let n = (a.distance(z) / 6.0).ceil().max(1.0) as usize;
    for k in 0..n {
        let (p, q) = (
            a.lerp(z, k as f32 / n as f32),
            a.lerp(z, (k + 1) as f32 / n as f32),
        );
        let (lo, hi) = under(&[p, q], ground);
        // Its axis across the walk, so the planks' boards run across it.
        let block = Block {
            centre: (p + q) * 0.5,
            axis: perp(along),
            half_a: b.half,
            half_c: 0.5 * p.distance(q),
            y0: lo - 0.05,
            y1: hi + 0.1,
        };
        m.block(
            &block,
            faces::SHELL,
            Paint::new([118, 102, 80], Material::Wood),
        );
    }
}

/// A water tank: timber staves (iron on a low stand) under a shallow cone of iron, on a braced tower or low legs.
fn tank(m: &mut MeshBuilder, t: &Tank, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let centre = t.centre + o;
    let g = ground(centre);
    let wood = Paint::new([112, 98, 78], Material::Wood);
    let base = g + t.stand;
    let leg = t.radius * if t.tower { 0.85 } else { 0.7 };
    let splay = if t.tower { 1.25 } else { 1.0 };
    let corner = |k: usize, r: f32| centre + Vec2::from_angle(k as f32 * TAU / 4.0 + 0.785) * r;
    let feet: Vec<Vec2> = (0..4).map(|k| corner(k, leg * splay)).collect();
    let heads: Vec<Vec2> = (0..4).map(|k| corner(k, leg)).collect();
    for k in 0..4 {
        let half = if t.tower { 0.14 } else { 0.1 };
        m.beam(
            up(feet[k], ground(feet[k]) - 0.2),
            up(heads[k], base),
            half,
            wood,
        );
        if t.tower {
            let j = (k + 1) % 4;
            let at = |i: usize, s: f32| {
                let p = feet[i].lerp(heads[i], s);
                up(p, ground(feet[i]) + (base - ground(feet[i])) * s)
            };
            for (s0, s1) in [(0.1f32, 0.55f32), (0.55, 0.95)] {
                m.beam(at(k, s0), at(j, s1), 0.05, wood);
                m.beam(at(j, s0), at(k, s1), 0.05, wood);
            }
        }
    }
    let ring: Vec<Vec2> = (0..12)
        .map(|k| centre + Vec2::from_angle(k as f32 * TAU / 12.0) * t.radius)
        .collect();
    let staves = if t.tower {
        wood
    } else {
        Paint::new([140, 142, 138], Material::Iron)
    };
    m.prism(&ring, base, base + t.height, false, staves);
    let peak = up(centre, base + t.height + 0.45 * t.radius);
    let roof = Paint::new([128, 128, 122], Material::Iron);
    for k in 0..12 {
        let (p, q) = (ring[k], ring[(k + 1) % 12]);
        let mid = (p + q) * 0.5 - centre;
        m.face(
            &[up(p, base + t.height), up(q, base + t.height), peak],
            &[
                Vec2::ZERO,
                Vec2::new(p.distance(q), 0.0),
                Vec2::new(0.5, t.radius),
            ],
            Vec3::new(mid.x, t.radius, mid.y),
            roof,
        );
    }
}

/// A telephone pole, its crossarm, and the pair of wires sagging to the next pole of its line (drawn by this pole's
/// chunk).
fn pole(m: &mut MeshBuilder, line: &PoleLine, k: usize, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let (p, h) = line.poles[k];
    let at = p + o;
    let g = ground(at);
    let wood = Paint::new([92, 80, 64], Material::Wood);
    let neighbour = line
        .poles
        .get(k + 1)
        .or_else(|| k.checked_sub(1).and_then(|j| line.poles.get(j)));
    let dir = neighbour.map_or(Vec2::X, |(q, _)| (*q - p).normalize_or(Vec2::X));
    let arm = perp(dir);
    m.beam(up(at, g - 0.4), up(at, g + h), 0.12, wood);
    let top = g + h - 0.45;
    m.beam(up(at - arm * 0.8, top), up(at + arm * 0.8, top), 0.06, wood);
    if let Some((q, qh)) = line.poles.get(k + 1) {
        let to = *q + o;
        let qg = ground(to);
        let qdir = line
            .poles
            .get(k + 2)
            .map_or(dir, |(r, _)| (*r - *q).normalize_or(dir));
        let qarm = perp((dir + qdir).normalize_or(dir));
        for side in [-0.65f32, 0.65] {
            let a = up(at + arm * side, top + 0.05);
            let b = up(to + qarm * side, qg + qh - 0.4);
            let span = a.distance(b);
            let points: Vec<Vec3> = (0..=10)
                .map(|i| {
                    let t = i as f32 / 10.0;
                    a.lerp(b, t) - Vec3::Y * (4.0 * t * (1.0 - t) * 0.02 * span)
                })
                .collect();
            m.wire(&points, 1, [38, 38, 36]);
        }
    }
}

/// A pillbox: a hexagon of concrete, its roof slab overhanging a little, a dark loophole in each face but its back's.
fn pillbox(m: &mut MeshBuilder, b: &Pillbox, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let centre = b.centre + o;
    let concrete = Paint::new([150, 148, 140], Material::Concrete);
    let turn = b.axis.to_angle();
    let ring: Vec<Vec2> = (0..6)
        .map(|k| centre + Vec2::from_angle(turn + k as f32 * TAU / 6.0) * b.radius)
        .collect();
    let (lo, hi) = under(&ring, ground);
    m.prism(&ring, lo - 0.3, hi + b.height, false, concrete);
    let roof: Vec<Vec2> = (0..6)
        .map(|k| centre + Vec2::from_angle(turn + k as f32 * TAU / 6.0) * (b.radius + 0.25))
        .collect();
    m.prism(&roof, hi + b.height, hi + b.height + 0.3, true, concrete);
    let dark = Paint::new([14, 14, 14], Material::Paint);
    for k in 0..5 {
        let (p, q) = (ring[k], ring[(k + 1) % 6]);
        let mid = (p + q) * 0.5;
        let out = (mid - centre).normalize_or(Vec2::X);
        let along = (q - p).normalize_or(Vec2::X);
        let base = up(mid + out * 0.03, hi + 0.8);
        let (x, y) = (Vec3::new(along.x, 0.0, along.y) * 0.4, Vec3::Y * 0.22);
        m.quad(
            [base - x, base + x, base + x + y, base - x + y],
            [
                Vec2::ZERO,
                Vec2::new(0.8, 0.0),
                Vec2::new(0.8, 0.22),
                Vec2::new(0.0, 0.22),
            ],
            Vec3::new(out.x, 0.0, out.y),
            dark,
        );
    }
}

/// A gun pit: a ring of sandbags with its gap (a Japanese gun's a bank of earth, sloped outside), and its gun.
fn pit(m: &mut MeshBuilder, p: &Pit, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let centre = p.centre + o;
    let g = ground(centre);
    let earth = matches!(p.gun, Gun::Triple | Gun::Navy | Gun::Flak20 | Gun::Flak88);
    let bank = if earth {
        Paint::new([102, 92, 74], Material::Earth)
    } else {
        Paint {
            colour: [150, 132, 98],
            material: Material::Earth,
            param: 1,
        }
    };
    let n = 18;
    let gap = p.opening.to_angle();
    let (inner, outer) = (p.radius - 0.45, p.radius + 0.45);
    // A bank of earth's outside slopes down to its foot.
    let foot_out = if earth { p.radius + 1.6 } else { outer };
    let height = match p.gun {
        Gun::Heavy => 1.2,
        Gun::Navy | Gun::Flak88 => 1.4,
        Gun::Light | Gun::Triple | Gun::Flak20 => 1.0,
    };
    let foot = g - 0.25;
    let open = |phi: f32| {
        let d = (phi - gap + PI).rem_euclid(TAU) - PI;
        d.abs() < 0.32
    };
    let ring = |phi: f32, r: f32| centre + Vec2::from_angle(phi) * r;
    for k in 0..n {
        let (p0, p1) = (k as f32 * TAU / n as f32, (k + 1) as f32 * TAU / n as f32);
        if open(0.5 * (p0 + p1)) {
            continue;
        }
        let (s0, s1) = (p0 * p.radius, p1 * p.radius);
        let h0 = ground(ring(p0, p.radius)) + height;
        let h1 = ground(ring(p1, p.radius)) + height;
        let (o0, o1, i0, i1) = (
            ring(p0, outer),
            ring(p1, outer),
            ring(p0, inner),
            ring(p1, inner),
        );
        let (f0, f1) = (ring(p0, foot_out), ring(p1, foot_out));
        let mid = Vec2::from_angle(0.5 * (p0 + p1));
        let out = Vec3::new(mid.x, 0.0, mid.y);
        let uv = |s: f32, y: f32| Vec2::new(s, y - foot);
        m.quad(
            [up(f0, foot), up(f1, foot), up(o1, h1), up(o0, h0)],
            [uv(s0, foot), uv(s1, foot), uv(s1, h1), uv(s0, h0)],
            out + Vec3::Y * if earth { 0.8 } else { 0.0 },
            bank,
        );
        m.quad(
            [up(o0, h0), up(o1, h1), up(i1, h1 - 0.05), up(i0, h0 - 0.05)],
            [uv(s0, h0), uv(s1, h1), uv(s1, h1 + 0.9), uv(s0, h0 + 0.9)],
            Vec3::Y,
            bank,
        );
        m.quad(
            [
                up(i1, foot),
                up(i0, foot),
                up(i0, h0 - 0.05),
                up(i1, h1 - 0.05),
            ],
            [uv(s1, foot), uv(s0, foot), uv(s0, h0), uv(s1, h1)],
            -out,
            bank,
        );
        // The wall's ends at the gap.
        for (phi, next, h) in [(p0, -1.0f32, h0), (p1, 1.0, h1)] {
            let beyond = phi + next * TAU / n as f32;
            if open(0.5 * (phi + beyond)) {
                let side = Vec2::from_angle(phi + next * PI * 0.5);
                m.face(
                    &[
                        up(ring(phi, inner), foot),
                        up(ring(phi, foot_out), foot),
                        up(ring(phi, outer), h),
                        up(ring(phi, inner), h - 0.05),
                    ],
                    &[uv(0.0, foot), uv(0.9, foot), uv(0.9, h), uv(0.0, h)],
                    Vec3::new(side.x, 0.0, side.y),
                    bank,
                );
            }
        }
    }
    // The gun on its mount, laid toward its aim: the American drab, the Japanese navy's grey-green.
    let colour = match p.gun {
        Gun::Triple | Gun::Navy => [88, 94, 84],
        Gun::Flak20 | Gun::Flak88 => [80, 82, 80],
        _ => [74, 76, 54],
    };
    let drab = Paint::new(colour, Material::Paint);
    let (mount, cradle, barrel, elevation, bore) = match p.gun {
        Gun::Heavy => (0.55, 1.6, 4.6, 35f32, 0.09),
        Gun::Light => (0.45, 1.25, 2.4, 25f32, 0.05),
        Gun::Triple => (0.5, 1.15, 2.3, 30f32, 0.045),
        Gun::Navy => (0.6, 1.75, 4.9, 40f32, 0.1),
        Gun::Flak20 => (0.35, 1.0, 2.2, 35f32, 0.04),
        Gun::Flak88 => (0.5, 2.0, 6.4, 30f32, 0.1),
    };
    if p.gun == Gun::Flak88 {
        // Its cruciform mount's four outriggers.
        for k in 0..4 {
            let d = Vec2::from_angle(k as f32 * TAU / 4.0 + 0.785);
            m.beam(
                up(centre, g + 0.2),
                up(centre + d * 3.0, g + 0.15),
                0.12,
                drab,
            );
        }
    }
    let ring8: Vec<Vec2> = (0..8)
        .map(|k| centre + Vec2::from_angle(k as f32 * TAU / 8.0) * mount)
        .collect();
    m.prism(&ring8, g - 0.1, g + cradle - 0.6, true, drab);
    let aim = p.aim.normalize_or(Vec2::X);
    let body = Block {
        centre,
        axis: aim,
        half_a: 0.75,
        half_c: if p.gun == Gun::Triple { 0.7 } else { 0.55 },
        y0: g + cradle - 0.6,
        y1: g + cradle,
    };
    m.block(&body, faces::SHELL, drab);
    let e = elevation.to_radians();
    let lay = Vec3::new(aim.x * e.cos(), e.sin(), aim.y * e.cos());
    let across = perp(aim);
    let barrels: &[f32] = if p.gun == Gun::Triple {
        &[-0.3, 0.0, 0.3]
    } else {
        &[0.0]
    };
    for off in barrels {
        let breech = up(centre - aim * 0.5 + across * *off, g + cradle - 0.1);
        m.beam(breech, breech + lay * barrel, bore, drab);
    }
    if matches!(p.gun, Gun::Navy | Gun::Flak88 | Gun::Flak20) {
        // Its shield, open behind.
        let shield = Block {
            centre: centre + aim * 0.55,
            axis: across,
            half_a: 1.05,
            half_c: 0.05,
            y0: g + 0.8,
            y1: g + cradle + 0.9,
        };
        m.block(&shield, faces::SHELL, drab);
    }
}

/// A truck (its running gear, body, cab, hood and the canvas over its bed) or a weapons carrier.
fn vehicle(m: &mut MeshBuilder, v: &Vehicle, o: Vec2, ground: &dyn Fn(Vec2) -> f32) {
    let a = v.axis.normalize_or(Vec2::X);
    let c = perp(a);
    let centre = v.centre + o;
    let (len, half) = match v.kind {
        VehicleKind::Truck => (3.4, 1.12),
        VehicleKind::Carrier => (2.25, 0.98),
        VehicleKind::Jeep => (1.6, 0.78),
        VehicleKind::Bowser => (3.0, 1.0),
        VehicleKind::Cart => (0.9, 0.6),
    };
    let wheels = [
        centre - a * len - c * half,
        centre + a * len - c * half,
        centre + a * len + c * half,
        centre - a * len + c * half,
    ];
    let g = wheels.iter().map(|p| ground(*p)).sum::<f32>() / 4.0;
    let drab = Paint::new(v.colour, Material::Paint);
    let dark = Paint::new([30, 30, 27], Material::Paint);
    let piece =
        |m: &mut MeshBuilder, along: f32, ha: f32, hc: f32, y0: f32, y1: f32, paint: Paint| {
            let block = Block {
                centre: centre + a * along,
                axis: a,
                half_a: ha,
                half_c: hc,
                y0: g + y0,
                y1: g + y1,
            };
            m.block(&block, faces::SHELL, paint);
        };
    if v.kind != VehicleKind::Cart {
        piece(m, 0.0, len - 0.4, half - 0.12, -0.15, 0.55, dark);
    }
    match v.kind {
        VehicleKind::Truck => {
            piece(m, -0.3, len - 0.2, half, 0.55, 1.25, drab);
            piece(m, 1.75, 0.75, half - 0.05, 1.25, 2.25, drab);
            piece(m, 2.85, 0.55, 0.78, 1.25, 1.7, drab);
            piece(
                m,
                -1.25,
                1.95,
                half,
                1.25,
                2.6,
                Paint::new([100, 98, 70], Material::Canvas),
            );
        }
        VehicleKind::Carrier => {
            piece(m, -0.2, len - 0.2, half, 0.5, 1.15, drab);
            piece(
                m,
                0.7,
                0.08,
                half - 0.1,
                1.15,
                1.75,
                Paint::new([24, 26, 28], Material::Glass),
            );
        }
        VehicleKind::Jeep => {
            // A tub, its windscreen, a spare on its back.
            let glass = Paint::new([24, 26, 28], Material::Glass);
            piece(m, 0.0, len - 0.1, half, 0.35, 0.95, drab);
            piece(m, 0.45, 0.05, half - 0.08, 0.95, 1.35, glass);
            piece(m, -len, 0.12, 0.3, 0.45, 1.05, dark);
        }
        VehicleKind::Bowser => {
            // The cab and hood ahead, the tank behind.
            piece(m, 1.7, 0.7, half - 0.05, 0.55, 2.1, drab);
            piece(m, 2.65, 0.4, 0.7, 0.55, 1.45, drab);
            piece(
                m,
                -0.85,
                1.95,
                0.82,
                0.7,
                2.05,
                Paint::new([86, 88, 74], Material::Paint),
            );
        }
        VehicleKind::Cart => {
            // Its bed of planks on two wheels, its handles.
            let wood = Paint::new(v.colour, Material::Wood);
            piece(m, 0.0, len, half, 0.5, 0.62, wood);
            for side in [-1.0f32, 1.0] {
                let wheel = Block {
                    centre: centre + c * (side * (half + 0.06)),
                    axis: a,
                    half_a: 0.33,
                    half_c: 0.04,
                    y0: g - 0.05,
                    y1: g + 0.62,
                };
                m.block(&wheel, faces::SHELL, dark);
                let from = centre + a * len + c * (side * (half - 0.1));
                m.beam(up(from, g + 0.6), up(from + a * 1.1, g + 0.85), 0.03, wood);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::SizeClass;
    use super::super::site::{SiteInput, fit, tests::coast, tests::creek};
    use super::super::strip::us_pacific_strip;
    use super::*;
    use crate::infra::mesh::InfraMesh;

    fn plan() -> Plan {
        plan_by(super::super::Template::UsPacificStrip)
    }

    fn plan_by(template: super::super::Template) -> Plan {
        let input = SiteInput {
            seed: 9,
            size: SizeClass::BomberField,
            template,
            gravel: false,
            heading_hint: None,
            ground: &coast,
            water: &creek,
            others: &[],
            runway: None,
        };
        let site = fit(&input).expect("a site");
        match template {
            super::super::Template::UsPacificStrip => us_pacific_strip(site, &input),
            _ => super::super::lay_out(site, &input),
        }
    }

    #[test]
    fn a_japanese_field_split_across_chunks_is_drawn_once() {
        let plan = plan_by(super::super::Template::IjnSolomonsField);
        assert!(!plan.nets.is_empty() && plan.huts.iter().any(|h| h.kind == HutKind::Hangar));
        let meshes = chunks(&plan);
        let drawn: u32 = meshes.iter().map(|m| m.counts.base_pieces).sum();
        assert_eq!(drawn as usize, plan.pieces(), "every standing piece once");
        // Every kind of Japanese piece draws something standing.
        for hut in &plan.huts {
            let mut m = MeshBuilder::new(true);
            super::hut(&mut m, hut, Vec2::ZERO, &|p| coast(p));
            assert!(m.finish().triangles() > 10, "{:?}", hut.kind);
        }
    }

    /// The base drawn by every chunk about it: each chunk's mesh, made from its own corner.
    fn chunks(plan: &Plan) -> Vec<InfraMesh> {
        let (lo, hi) = (plan.min, plan.max);
        let (i0, i1) = (
            (lo.x / SIDE).floor() as i32 - 1,
            (hi.x / SIDE).ceil() as i32 + 1,
        );
        let (j0, j1) = (
            (lo.y / SIDE).floor() as i32 - 1,
            (hi.y / SIDE).ceil() as i32 + 1,
        );
        let mut out = Vec::new();
        for j in j0..j1 {
            for i in i0..i1 {
                let corner = Vec2::new(i as f32, j as f32) * SIDE;
                let mut m = MeshBuilder::new(true);
                let ground = |p: Vec2| coast(p + corner);
                draw(
                    &mut m,
                    &Placed {
                        plan,
                        origin: -corner,
                    },
                    &ground,
                );
                out.push(m.finish());
            }
        }
        out
    }

    #[test]
    fn a_base_split_across_chunks_is_drawn_once() {
        let plan = plan();
        let meshes = chunks(&plan);
        let drawn: u32 = meshes.iter().map(|m| m.counts.base_pieces).sum();
        assert_eq!(drawn as usize, plan.pieces(), "every standing piece once");
        assert!(
            meshes.iter().filter(|m| !m.is_empty()).count() >= 4,
            "split across chunks"
        );
        // The runway's coral laid once: its area, overruns and shoulders, within a percent.
        let runway = &plan.ribbons[0];
        let length = runway.points[0].distance(runway.points[1]);
        let mut area = 0.0;
        for mesh in &meshes {
            for t in mesh.indices
                [mesh.range(Group::Decal).start as usize..mesh.range(Group::Decal).end as usize]
                .chunks(3)
            {
                let v = [0, 1, 2].map(|k| mesh.vertices[t[k] as usize]);
                if v[0].colour[3] != Material::Coral as u8 || v[0].colour[0] != 0 {
                    continue;
                }
                let p = v.map(|v| Vec3::from(v.position));
                area += 0.5 * (p[1] - p[0]).cross(p[2] - p[0]).length();
            }
        }
        let whole = length * 2.0 * runway.half;
        assert!((area - whole).abs() < whole * 0.01, "{area} of {whole}");
        // Nothing floats: every standing piece's lowest vertex at or under the ground about it.
        for mesh in &meshes {
            let r = mesh.range(Group::Structure);
            assert!(mesh.range(Group::Decal).end <= r.start);
        }
    }

    #[test]
    fn pieces_face_out_and_stand_on_the_ground() {
        let plan = plan();
        let tent = plan.tents[0];
        let mut m = MeshBuilder::new(true);
        let ground = |p: Vec2| coast(p);
        tent_alone(&mut m, &tent, &ground);
        let mesh = m.finish();
        let g = coast(tent.centre);
        assert!(mesh.min[1] < g, "{} over {g}", mesh.min[1]);
        assert!((mesh.max[1] - (g + tent.peak)).abs() < 0.5);
        // A Quonset hut: 6.1 m wide, its length, its ends closed.
        let hut = plan
            .huts
            .iter()
            .find(|h| h.kind == HutKind::Quonset)
            .copied()
            .expect("a Quonset");
        let mut m = MeshBuilder::new(true);
        super::hut(&mut m, &hut, Vec2::ZERO, &ground);
        let mesh = m.finish();
        let size = Vec3::from(mesh.max) - Vec3::from(mesh.min);
        let footprint = size.x.max(size.z);
        assert!((14.0..=19.0).contains(&footprint), "{size}");
        // Every face's normal points away from the hut's middle line, up or out (the plinth, shell and ends).
        let mid = Vec3::new(hut.centre.x, 0.0, hut.centre.y);
        for t in mesh.indices.chunks(3) {
            let v = mesh.vertices[t[0] as usize];
            let n = Vec3::new(
                f32::from(v.normal[0]),
                f32::from(v.normal[1]),
                f32::from(v.normal[2]),
            );
            let p = Vec3::from(v.position);
            let d = Vec3::new(p.x - mid.x, 0.0, p.z - mid.z);
            assert!(n.y > 0.0 || d.dot(n) > 0.0, "a face inward at {p}");
        }
    }

    fn tent_alone(m: &mut MeshBuilder, t: &Tent, ground: &dyn Fn(Vec2) -> f32) {
        super::tent(m, t, Vec2::ZERO, ground);
    }
}
