//! Boats moored in a marina's slips (native/docs/terrain/infra.md). OpenStreetMap maps a
//! marina's docks as pier lines (`man_made=pier`; OpenMapTiles' `transportation` class `pier`): long main walkways,
//! and where the mapper went further, the short fingers between the slips. The boats lie in the slips: across each
//! main walkway, both sides, every few metres, between the fingers where they are mapped (a boat never crosses a
//! dock), on water, about four in five slips taken. Their length (8 to 20 m) is the slip's: never more than the open
//! water off the walkway allows (two boats share a fairway between docks). Only within reach of a marina's point.
//!
//! Each boat is a small procedural model: a sailing yacht (a hull, its deck, a cabin with its windows, the mast and
//! boom with its cover) or a motor yacht (a higher hull, a cabin and on the larger ones a flybridge).

use glam::{Vec2, Vec3};

use super::mesh::{Group, Material, MeshBuilder, Paint, hash01};
use crate::buildings::CHUNK_M;
use crate::buildings::geometry;

/// How far from a marina's point its docks get boats (m).
pub const MARINA_REACH_M: f32 = 900.0;
/// A walkway's half width (m): the buildings' pier lines are drawn 4 m wide.
const DOCK_HALF_M: f32 = 2.0;
/// The least walkway that boats lie across (m): shorter lines are fingers.
const WALKWAY_M: f32 = 20.0;
/// Slips taken.
const TAKEN: f32 = 0.82;

/// The marina's docks and points about a chunk (chunk-local metres; reaching a little past it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marina {
    /// Pier lines as segments.
    pub docks: Vec<[Vec2; 2]>,
    /// Marinas' points.
    pub marinas: Vec<Vec2>,
}

/// One boat: its middle, its bow's direction, its length and beam, and what it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Boat {
    pub centre: Vec2,
    pub bow: Vec2,
    pub length: f32,
    pub beam: f32,
    pub sail: bool,
    pub seed: f32,
}

impl Boat {
    fn corners(&self, margin: f32) -> [Vec2; 4] {
        let f = self.bow * (self.length * 0.5 + margin);
        let s = Vec2::new(-self.bow.y, self.bow.x) * (self.beam * 0.5 + margin);
        [
            self.centre - f - s,
            self.centre + f - s,
            self.centre + f + s,
            self.centre - f + s,
        ]
    }
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    geometry::segment_distance(p.to_array(), a.to_array(), b.to_array())
}

/// Whether two convex quads overlap (separating axes).
fn quads_overlap(a: &[Vec2; 4], b: &[Vec2; 4]) -> bool {
    for poly in [a, b] {
        for k in 0..4 {
            let e = poly[(k + 1) % 4] - poly[k];
            let n = Vec2::new(-e.y, e.x);
            let (amin, amax) = a.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                let d = p.dot(n);
                (lo.min(d), hi.max(d))
            });
            let (bmin, bmax) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                let d = p.dot(n);
                (lo.min(d), hi.max(d))
            });
            if amax < bmin || bmax < amin {
                return false;
            }
        }
    }
    true
}

/// Whether a boat's hull crosses a dock (a segment widened by its half width): its outline's edges and its middle
/// line against each dock, `skip` the dock it lies off.
fn crosses_dock(boat: &Boat, docks: &[[Vec2; 2]], skip: usize) -> bool {
    let quad = boat.corners(0.3);
    let lo = quad.iter().fold(Vec2::splat(f32::MAX), |a, p| a.min(*p)) - Vec2::splat(DOCK_HALF_M);
    let hi = quad.iter().fold(Vec2::splat(f32::MIN), |a, p| a.max(*p)) + Vec2::splat(DOCK_HALF_M);
    docks.iter().enumerate().any(|(k, &[a, b])| {
        if k == skip
            || a.x.max(b.x) < lo.x
            || a.x.min(b.x) > hi.x
            || a.y.max(b.y) < lo.y
            || a.y.min(b.y) > hi.y
        {
            return false;
        }
        // The dock as a thin quad.
        let d = (b - a).normalize_or(Vec2::X);
        let n = Vec2::new(-d.y, d.x) * DOCK_HALF_M;
        let dock = [a - n, b - n, b + n, a + n];
        quads_overlap(&quad, &dock)
    })
}

/// The open water off a dock at `from` along `dir`: metres until land, another dock or `limit`.
fn open_water(
    from: Vec2,
    dir: Vec2,
    limit: f32,
    docks: &[[Vec2; 2]],
    skip: usize,
    water: &dyn Fn(Vec2) -> bool,
) -> f32 {
    let step = 1.0;
    let mut r = step;
    while r <= limit {
        let p = from + dir * r;
        if !water(p)
            || docks
                .iter()
                .enumerate()
                .any(|(k, &[a, b])| k != skip && segment_distance(p, a, b) < DOCK_HALF_M)
        {
            return r - step;
        }
        r += step;
    }
    limit
}

/// The boats in the slips of `marina`'s docks whose middles lie in the chunk (chunk-local), `water` telling the open
/// water, `index` the chunk (its hashes theatre-anchored).
pub fn moor(marina: &Marina, water: &dyn Fn(Vec2) -> bool, index: [i32; 2]) -> Vec<Boat> {
    let side = CHUNK_M as f32;
    let corner = Vec2::new(
        (f64::from(index[0]) * CHUNK_M) as f32,
        (f64::from(index[1]) * CHUNK_M) as f32,
    );
    let mut out: Vec<Boat> = Vec::new();
    if marina.marinas.is_empty() {
        return out;
    }
    let near_marina = |p: Vec2| {
        marina
            .marinas
            .iter()
            .any(|m| m.distance(p) < MARINA_REACH_M)
    };
    // Every boat placed, taken or not, kept clear of each other across the docks (a pier's line comes as segments).
    let mut placed: Vec<Boat> = Vec::new();
    for (k, &[a, b]) in marina.docks.iter().enumerate() {
        let len = a.distance(b);
        if len < WALKWAY_M || !near_marina((a + b) * 0.5) {
            continue;
        }
        let d = (b - a) / len;
        for flank in [-1.0f32, 1.0] {
            let out_dir = Vec2::new(-d.y, d.x) * flank;
            let mut s = 1.5;
            while s < len - 1.5 {
                let at = a + d * s;
                let key = |seed: i64| {
                    let g = at + corner;
                    hash01(
                        (g.x * 2.0) as i64,
                        (g.y * 2.0) as i64,
                        seed + (flank as i64) * 7,
                    )
                };
                let sail = key(501) < 0.55;
                let want = if sail {
                    8.0 + 8.0 * key(502).powf(1.5)
                } else {
                    9.0 + 11.0 * key(502).powf(1.8)
                };
                let start = at + out_dir * (DOCK_HALF_M + 0.6);
                let open = open_water(start, out_dir, 50.0, &marina.docks, k, water);
                // Two boats share a fairway between docks.
                let length = if open >= 49.0 {
                    want
                } else {
                    want.min((open - 2.0) * 0.42)
                };
                let beam = length * if sail { 0.32 } else { 0.34 };
                if length < 6.5 {
                    s += 1.0;
                    continue;
                }
                let boat = Boat {
                    centre: start + out_dir * (length * 0.5),
                    // Bows in, toward the walkway, as most berth.
                    bow: -out_dir,
                    length,
                    beam,
                    sail,
                    seed: key(503),
                };
                let clear = boat.corners(0.0).iter().all(|p| water(*p))
                    && water(boat.centre)
                    && !crosses_dock(&boat, &marina.docks, k)
                    && !placed.iter().any(|o| {
                        o.centre.distance(boat.centre) < (o.length + boat.length) * 0.5 + 1.0
                            && quads_overlap(&o.corners(0.35), &boat.corners(0.35))
                    });
                if !clear {
                    s += 0.75;
                    continue;
                }
                let empty = key(504) > TAKEN;
                placed.push(boat);
                if !empty
                    && (0.0..side).contains(&boat.centre.x)
                    && (0.0..side).contains(&boat.centre.y)
                {
                    out.push(boat);
                }
                s += beam + 0.9 + key(505) * 0.6;
            }
        }
    }
    out
}

/// The hull's half beam at `t` along it (−0.5 the stern, 0.5 the bow), as a share of the beam's half.
fn half_beam(t: f32) -> f32 {
    let stations: [(f32, f32); 7] = [
        (-0.5, 0.82),
        (-0.3, 1.0),
        (0.05, 1.0),
        (0.25, 0.86),
        (0.38, 0.62),
        (0.46, 0.32),
        (0.5, 0.0),
    ];
    for w in stations.windows(2) {
        let ((t0, b0), (t1, b1)) = (w[0], w[1]);
        if t <= t1 {
            return b0 + (b1 - b0) * ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
        }
    }
    0.0
}

const HULL_COLOURS: [[u8; 3]; 8] = [
    [236, 236, 230],
    [238, 238, 234],
    [232, 230, 222],
    [240, 240, 238],
    [30, 44, 74],
    [40, 70, 52],
    [150, 36, 32],
    [214, 204, 178],
];
const COVER_COLOURS: [[u8; 3]; 4] = [[34, 56, 104], [26, 40, 70], [150, 140, 120], [60, 90, 120]];

fn pick<T: Copy>(options: &[T], t: f32) -> T {
    options[((t * options.len() as f32) as usize).min(options.len() - 1)]
}

/// The boats' meshes (none for a chunk made far): each in full into [`Group::Near`] (drawn within the close range) and
/// as a stand-in into [`Group::Far`] (past it: the hull's outline, its deck and the cabin, a quarter of the triangles).
pub fn draw(m: &mut MeshBuilder, boats: &[Boat]) {
    if !m.near() {
        return;
    }
    let group = m.group();
    m.set_group(Group::Near);
    for boat in boats {
        draw_boat(m, boat);
    }
    m.set_group(Group::Far);
    for boat in boats {
        draw_stand_in(m, boat);
    }
    m.set_group(group);
    m.counts.boats += boats.len() as u32;
}

fn draw_boat(m: &mut MeshBuilder, b: &Boat) {
    let f = b.bow;
    let s = Vec2::new(-f.y, f.x);
    let at = |t: f32, w: f32, y: f32| {
        let p = b.centre + f * (t * b.length) + s * (w * b.beam * 0.5);
        Vec3::new(p.x, y, p.y)
    };
    let hash = |k: f32| (b.seed * 97.0 + k * 13.37).fract();
    let hull = Paint::new(
        if hash(1.0) < 0.78 {
            pick(&HULL_COLOURS[..4], hash(2.0))
        } else {
            pick(&HULL_COLOURS[4..], hash(2.0))
        },
        Material::Hull,
    );
    let white = Paint::new([236, 236, 232], Material::Hull);
    let deck = Paint::new(
        if b.sail && hash(3.0) < 0.3 {
            [150, 112, 74]
        } else {
            [214, 212, 204]
        },
        Material::Paint,
    );
    let glass = Paint::new([20, 26, 32], Material::Glass);
    let freeboard = if b.sail { 0.95 } else { 1.35 } + b.length * 0.02;
    // The hull: from below the water (the sea hides it) up to the deck's edge, the deck's outline flaring a little.
    let ts: Vec<f32> = vec![-0.5, -0.3, 0.05, 0.25, 0.38, 0.46, 0.5];
    let mut deck_ring = Vec::new();
    let mut keel_ring = Vec::new();
    for &t in &ts {
        deck_ring.push(at(t, half_beam(t), freeboard));
        keel_ring.push(at(t * 0.97, half_beam(t) * 0.8, -0.5));
    }
    for &t in ts.iter().rev().skip(1) {
        deck_ring.push(at(t, -half_beam(t), freeboard));
        keel_ring.push(at(t * 0.97, -half_beam(t) * 0.8, -0.5));
    }
    let centre = at(0.0, 0.0, 0.0);
    m.loft(&keel_ring, &deck_ring, centre, hull);
    let uv: Vec<Vec2> = deck_ring.iter().map(|p| Vec2::new(p.x, p.z)).collect();
    m.face(&deck_ring, &uv, Vec3::Y, deck);
    let block = |m: &mut MeshBuilder, t0: f32, t1: f32, w: f32, y0: f32, y1: f32, paint: Paint| {
        let ring = [
            at(t0, -w, 0.0),
            at(t1, -w, 0.0),
            at(t1, w, 0.0),
            at(t0, w, 0.0),
        ];
        let ring: Vec<Vec2> = ring.iter().map(|p| Vec2::new(p.x, p.z)).collect();
        m.prism(&ring, y0, y1, true, paint);
    };
    if b.sail {
        // The cabin trunk and its windows, the mast and the boom under its cover.
        let top = freeboard + 0.55;
        block(m, -0.22, 0.12, 0.56, freeboard - 0.05, top, white);
        block(
            m,
            -0.18,
            0.08,
            0.565,
            freeboard + 0.2,
            freeboard + 0.38,
            glass,
        );
        let mast = at(0.12, 0.0, top);
        let head = at(0.12, 0.0, freeboard + b.length * 1.25);
        let spar = Paint::new([200, 200, 196], Material::Steel);
        m.beam(mast, head, 0.07 + b.length * 0.004, spar);
        let gooseneck = at(0.11, 0.0, top + 1.0);
        let end = at(-0.32, 0.0, top + 1.15);
        m.beam(
            gooseneck,
            end,
            0.16,
            Paint::new(pick(&COVER_COLOURS, hash(4.0)), Material::Paint),
        );
    } else {
        // The cabin with a band of windows; a flybridge on a larger yacht.
        let top = freeboard + 1.1;
        block(m, -0.32, 0.14, 0.78, freeboard - 0.05, top, white);
        block(
            m,
            -0.26,
            0.1,
            0.785,
            freeboard + 0.45,
            freeboard + 0.85,
            glass,
        );
        if b.length >= 12.0 {
            block(m, -0.24, 0.02, 0.62, top, top + 0.75, white);
            block(m, -0.2, 0.0, 0.625, top + 0.25, top + 0.55, glass);
        }
    }
}

/// A boat as it reads from a kilometre off: the hull's outline at the deck down to the water, the deck, the cabin.
fn draw_stand_in(m: &mut MeshBuilder, b: &Boat) {
    let f = b.bow;
    let s = Vec2::new(-f.y, f.x);
    let at = |t: f32, w: f32| b.centre + f * (t * b.length) + s * (w * b.beam * 0.5);
    let hash = |k: f32| (b.seed * 97.0 + k * 13.37).fract();
    let hull = Paint::new(
        if hash(1.0) < 0.78 {
            pick(&HULL_COLOURS[..4], hash(2.0))
        } else {
            pick(&HULL_COLOURS[4..], hash(2.0))
        },
        Material::Hull,
    );
    let deck = Paint::new([214, 212, 204], Material::Paint);
    let white = Paint::new([236, 236, 232], Material::Hull);
    let freeboard = if b.sail { 0.95 } else { 1.35 } + b.length * 0.02;
    let outline = [
        at(0.5, 0.0),
        at(0.2, 0.95),
        at(-0.5, 0.85),
        at(-0.5, -0.85),
        at(0.2, -0.95),
    ];
    m.prism(&outline, -0.3, freeboard, false, hull);
    let points: Vec<Vec3> = outline
        .iter()
        .map(|p| Vec3::new(p.x, freeboard, p.y))
        .collect();
    m.face(&points, &outline, Vec3::Y, deck);
    let (t0, t1, w, top) = if b.sail {
        (-0.22, 0.12, 0.56, freeboard + 0.55)
    } else {
        (-0.32, 0.14, 0.78, freeboard + 1.1)
    };
    let cabin = [at(t0, -w), at(t1, -w), at(t1, w), at(t0, w)];
    m.prism(&cabin, freeboard - 0.05, top, true, white);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A main walkway along z = 500 from x = 200 to 400 with fingers every 10 m on its north side, open water about.
    fn marina() -> Marina {
        let mut docks = vec![[Vec2::new(200.0, 500.0), Vec2::new(400.0, 500.0)]];
        for k in 0..18 {
            let x = 210.0 + k as f32 * 10.0;
            docks.push([Vec2::new(x, 500.0), Vec2::new(x, 484.0)]);
        }
        Marina {
            docks,
            marinas: vec![Vec2::new(300.0, 450.0)],
        }
    }

    #[test]
    fn boats_lie_in_the_slips_on_the_water_and_off_the_docks() {
        let m = marina();
        let water = |p: Vec2| p.y < 560.0 && p.y > 400.0;
        let boats = moor(&m, &water, [0, 0]);
        assert!(boats.len() > 30, "{} boats", boats.len());
        for b in &boats {
            assert!((8.0 - 1e-3..=20.0).contains(&b.length) || b.length >= 6.5);
            assert!(b.corners(0.0).iter().all(|p| water(*p)));
            assert!(!crosses_dock(b, &m.docks, 0));
            // Across the walkway: bows toward it.
            assert!(b.bow.x.abs() < 1e-4);
        }
        // North of the walkway the boats lie between the fingers (no wider than their gap), south of it in the open.
        let north: Vec<&Boat> = boats.iter().filter(|b| b.centre.y < 500.0).collect();
        assert!(!north.is_empty());
        assert!(north.iter().all(|b| b.beam < 10.0 - 2.0 * DOCK_HALF_M));
        // None overlaps another.
        for (i, a) in boats.iter().enumerate() {
            for b in &boats[i + 1..] {
                assert!(!quads_overlap(&a.corners(0.0), &b.corners(0.0)));
            }
        }
        // Deterministic, and none with no marina about.
        assert_eq!(moor(&m, &water, [0, 0]), boats);
        let none = Marina {
            marinas: vec![],
            ..m.clone()
        };
        assert!(moor(&none, &water, [0, 0]).is_empty());
    }

    #[test]
    fn a_pier_in_segments_moors_no_boat_over_another() {
        let m = Marina {
            docks: vec![
                [Vec2::new(200.0, 500.0), Vec2::new(300.0, 500.0)],
                [Vec2::new(300.0, 500.0), Vec2::new(400.0, 500.0)],
            ],
            marinas: vec![Vec2::new(300.0, 450.0)],
        };
        let boats = moor(&m, &|_| true, [0, 0]);
        assert!(boats.len() > 10, "{} boats", boats.len());
        for (i, a) in boats.iter().enumerate() {
            for b in &boats[i + 1..] {
                assert!(
                    !quads_overlap(&a.corners(0.0), &b.corners(0.0)),
                    "boats at {} and {} overlap",
                    a.centre,
                    b.centre
                );
            }
        }
    }

    #[test]
    fn a_fairway_between_docks_halves_the_boats() {
        // Two walkways 30 m apart: boats between them no longer than (30 - 4 - 1.2 - 2) × 0.42.
        let m = Marina {
            docks: vec![
                [Vec2::new(100.0, 500.0), Vec2::new(300.0, 500.0)],
                [Vec2::new(100.0, 530.0), Vec2::new(300.0, 530.0)],
            ],
            marinas: vec![Vec2::new(200.0, 515.0)],
        };
        let water = |_: Vec2| true;
        let boats = moor(&m, &water, [0, 0]);
        let between: Vec<&Boat> = boats
            .iter()
            .filter(|b| b.centre.y > 500.0 && b.centre.y < 530.0)
            .collect();
        assert!(!between.is_empty());
        assert!(between.iter().all(|b| b.length <= 10.0));
        let mut mesh = MeshBuilder::default();
        draw(&mut mesh, &boats);
        let mesh = mesh.finish();
        assert_eq!(mesh.counts.boats as usize, boats.len());
        assert!(mesh.triangles() < boats.len() * 90);
    }
}
