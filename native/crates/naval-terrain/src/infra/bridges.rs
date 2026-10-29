//! Bridges over water as structures (native/docs/terrain/infra.md). OpenMapTiles carries a road's
//! or railway's crossing (`transportation` `brunnel=bridge`) and, where the mapper drew it, the bridge's outline
//! (`man_made=bridge`, class `bridge`); neither says how high it stands. Over water the roads on the ground stop at the
//! shore, so a bridge there is drawn whole: its deck along the line (as wide as its outline, or its road), the parapets,
//! the piers down to the ground or the seabed, and over a long navigable span a suspension bridge's towers, cables and
//! hangers. Drawn only: the simulation never knows of it, and a ship sails under where the clearance allows.
//!
//! **Clearance.** The deck rises from the road at either end at a grade of at most 5.5 percent (8 over a stream) to a
//! clearance over its main span (the longest stretch over water) that grows with the span: over the sea 6 m plus a
//! ninth of the span, to 62 m (Los Angeles' Vincent Thomas Bridge, a 457 m main span, stands 56 m over the channel);
//! over a river or canal 2.5 m plus 3 percent, to 12 m. A bridge too short to climb that high stands lower, as real
//! ones do.

use glam::{Vec2, Vec3};

use super::mesh::{Group, Material, MeshBuilder, Paint, hash01};
use crate::buildings::{CHUNK_M, geometry};

/// What lies under a point of a bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Under {
    Dry,
    /// A river, canal or lake.
    Inland,
    /// The sea, a harbour or a dock.
    Sea,
}

/// A bridge's line (chunk-local, the whole of it as the host joined it across tiles), its road's half width, whether it
/// carries a railway, and a key (its place, hashed) for its look.
#[derive(Clone, Debug, PartialEq)]
pub struct BridgeLine {
    pub points: Vec<Vec2>,
    pub half: f32,
    pub rail: bool,
    pub key: i64,
}

/// What the bridges of a chunk are made from: the lines, the bridges' outlines (chunk-local rings), the ground and
/// what lies under a point.
pub struct BridgeInput<'a> {
    pub lines: &'a [BridgeLine],
    pub outlines: &'a [Vec<[f32; 2]>],
    pub ground: &'a dyn Fn(Vec2) -> f32,
    pub under: &'a dyn Fn(Vec2) -> Under,
}

/// A bridge's samples along its line every few metres, and how it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub points: Vec<Vec2>,
    /// Metres along the line.
    pub along: Vec<f32>,
    /// The deck's top (m above the sea).
    pub deck: Vec<f32>,
    /// The ground under each sample (the seabed under the sea).
    pub ground: Vec<f32>,
    /// The deck's half widths left and right of the line (left: −r, r the direction turned a quarter clockwise).
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// The main span: from and to (m along), what it crosses, and the water's level under it.
    pub span: (f32, f32),
    pub under: Under,
    pub water: f32,
    /// The deck's depth (m).
    pub depth: f32,
    /// A suspension bridge's: its towers' places along the line.
    pub towers: Option<(f32, f32)>,
}

impl Profile {
    /// Clearance over the main span's middle (its navigation channel): the deck's bottom there, less the water's level.
    pub fn clearance(&self) -> f32 {
        let mid = (self.span.0 + self.span.1) * 0.5;
        let k = self
            .along
            .partition_point(|s| *s < mid)
            .min(self.deck.len() - 1);
        self.deck[k] - self.depth - self.water
    }

    /// Whether `s` (m along) lies in the main span's channel, kept clear of piers: the middle 60 percent of a span over
    /// the sea (up to 200 m of it), the middle half of one inland; all of a suspension bridge's main span.
    pub fn in_channel(&self, s: f32) -> bool {
        if let Some((a, b)) = self.towers {
            return s > a && s < b;
        }
        let (w, mid) = (self.span.1 - self.span.0, (self.span.0 + self.span.1) * 0.5);
        let half = match self.under {
            Under::Sea => (w * 0.3).min(100.0),
            _ => w * 0.25,
        };
        (s - mid).abs() < half
    }
}

/// The least main span drawn (m): a culvert's stream is the road's own.
const LEAST_SPAN_M: f32 = 12.0;
/// The longest step between samples (m).
const STEP_M: f32 = 8.0;

fn resample(points: &[Vec2]) -> (Vec<Vec2>, Vec<f32>) {
    let mut out = Vec::new();
    let mut along = Vec::new();
    let mut s = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.distance(b);
        if len < 1e-3 {
            continue;
        }
        let n = (len / STEP_M).ceil().max(1.0) as usize;
        for k in 0..n {
            let t = k as f32 / n as f32;
            out.push(a.lerp(b, t));
            along.push(s + len * t);
        }
        s += len;
    }
    if let Some(&last) = points.last() {
        out.push(last);
        along.push(s);
    }
    (out, along)
}

/// The direction to the right of the line at sample `k` (the mean of its segments').
fn right_at(points: &[Vec2], k: usize) -> Vec2 {
    let n = points.len();
    let a = points[k.saturating_sub(1)];
    let b = points[(k + 1).min(n - 1)];
    let d = (b - a).normalize_or(Vec2::X);
    Vec2::new(-d.y, d.x)
}

/// How far from `p` along `dir` a ring's edge lies (the nearest crossing), if `p` is inside it.
fn reach_in(ring: &[[f32; 2]], p: Vec2, dir: Vec2) -> Option<f32> {
    if !geometry::contains(ring, p.to_array()) {
        return None;
    }
    let n = ring.len();
    let mut best = f32::MAX;
    for i in 0..n {
        let (a, b) = (Vec2::from(ring[i]), Vec2::from(ring[(i + 1) % n]));
        let e = b - a;
        let denom = dir.perp_dot(e);
        if denom.abs() < 1e-6 {
            continue;
        }
        let w = a - p;
        let t = w.perp_dot(e) / denom;
        let u = w.perp_dot(dir) / denom;
        if t >= 0.0 && (0.0..=1.0).contains(&u) {
            best = best.min(t);
        }
    }
    (best < f32::MAX).then_some(best)
}

/// The profile of `line` over `input`'s ground and water; none where it crosses no water.
pub fn profile(line: &BridgeLine, input: &BridgeInput<'_>) -> Option<Profile> {
    let (points, along) = resample(&line.points);
    let n = points.len();
    if n < 2 {
        return None;
    }
    let length = *along.last()?;
    let ground: Vec<f32> = points.iter().map(|p| (input.ground)(*p)).collect();
    let under: Vec<Under> = points.iter().map(|p| (input.under)(*p)).collect();
    // The longest run over water.
    let mut best: Option<(usize, usize)> = None;
    let mut k = 0;
    while k < n {
        if under[k] == Under::Dry {
            k += 1;
            continue;
        }
        let start = k;
        while k + 1 < n && under[k + 1] != Under::Dry {
            k += 1;
        }
        if best.is_none_or(|(a, b)| along[k] - along[start] > along[b] - along[a]) {
            best = Some((start, k));
        }
        k += 1;
    }
    let (a, b) = best?;
    // A run's ends lie a sample inside the water: the span reaches half a step past each.
    let span = (
        (along[a] - STEP_M * 0.5).max(0.0),
        (along[b] + STEP_M * 0.5).min(length),
    );
    let width = span.1 - span.0;
    if width < LEAST_SPAN_M {
        return None;
    }
    let kind = under[a..=b].iter().copied().max().unwrap_or(Under::Inland);
    let water = if kind == Under::Sea {
        0.0
    } else {
        ground[a..=b]
            .iter()
            .copied()
            .fold(f32::MAX, f32::min)
            .max(0.0)
    };
    let (clearance, grade) = match kind {
        Under::Sea => ((6.0 + width / 9.0).min(62.0), 0.055),
        _ => ((2.5 + width * 0.03).min(12.0), 0.08),
    };
    let depth = if width < 60.0 {
        1.3
    } else if width < 250.0 {
        2.4
    } else {
        3.4
    };
    let top = water + clearance + depth;
    let end = |g: f32| g.max(0.0) + 0.4;
    let (h0, h1) = (end(ground[0]), end(ground[n - 1]));
    let mut deck: Vec<f32> = along
        .iter()
        .map(|&s| top.min(h0 + grade * s).min(h1 + grade * (length - s)))
        .collect();
    // Smoothed (the ramps' joins), never under the ground.
    for _ in 0..3 {
        let prev = deck.clone();
        for k in 1..n - 1 {
            deck[k] = (prev[k - 1] + 2.0 * prev[k] + prev[k + 1]) * 0.25;
        }
    }
    for k in 0..n {
        if under[k] == Under::Dry {
            deck[k] = deck[k].max(ground[k] + 0.4);
        }
    }
    // Half widths: the outlines' where the line lies in them (the widest reach of those holding it: a tile's piece of
    // an outline is cut a little past the tile), else the road's and a margin.
    let mut left = vec![line.half + 1.0; n];
    let mut right = vec![line.half + 1.0; n];
    for k in 0..n {
        let r = right_at(&points, k);
        let mut found: Option<(f32, f32)> = None;
        for ring in input.outlines {
            if let (Some(l), Some(rr)) =
                (reach_in(ring, points[k], -r), reach_in(ring, points[k], r))
            {
                found = Some(found.map_or((l, rr), |(a, b)| (a.max(l), b.max(rr))));
            }
        }
        if let Some((l, rr)) = found {
            left[k] = l.clamp(2.0, 40.0);
            right[k] = rr.clamp(2.0, 40.0);
        }
    }
    let towers = (kind == Under::Sea && width >= 300.0 && top >= 35.0)
        .then(|| ((span.0 - 6.0).max(0.0), (span.1 + 6.0).min(length)));
    Some(Profile {
        points,
        along,
        deck,
        ground,
        left,
        right,
        span,
        under: kind,
        water,
        depth,
        towers,
    })
}

/// The lines whose bridges are drawn: of lines whose middles lie in one bridge outline (a dual carriageway's two ways on
/// one deck), the longest alone, its deck as wide as the outline.
pub fn spines<'a>(lines: &'a [BridgeLine], outlines: &[Vec<[f32; 2]>]) -> Vec<&'a BridgeLine> {
    let length = |line: &BridgeLine| {
        line.points
            .windows(2)
            .map(|w| w[0].distance(w[1]))
            .sum::<f32>()
    };
    let middle = |line: &BridgeLine| {
        let (points, along) = resample(&line.points);
        let half = along.last().copied().unwrap_or(0.0) * 0.5;
        let k = along
            .partition_point(|s| *s < half)
            .min(points.len().saturating_sub(1));
        points.get(k).copied().unwrap_or(Vec2::ZERO).to_array()
    };
    let middles: Vec<[f32; 2]> = lines.iter().map(middle).collect();
    let mut out = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let beaten = outlines
            .iter()
            .filter(|ring| geometry::contains(ring, middles[k]))
            .any(|ring| {
                lines.iter().enumerate().any(|(j, other)| {
                    j != k
                        && geometry::contains(ring, middles[j])
                        && (length(other), -(j as i64)) > (length(line), -(k as i64))
                })
            });
        if !beaten {
            out.push(line);
        }
    }
    out
}

const STEEL_COLOURS: [[u8; 3]; 4] = [
    [150, 168, 160],
    [176, 178, 176],
    [146, 64, 48],
    [214, 214, 206],
];
const CONCRETE: [u8; 3] = [178, 174, 166];

fn pick<T: Copy>(options: &[T], t: f32) -> T {
    options[((t * options.len() as f32) as usize).min(options.len() - 1)]
}

/// Chunk `index`'s parts of the bridges of `input`: each segment whose middle lies in the chunk, each pier, tower and
/// cable piece likewise. The bridges counted (those with a part here).
pub fn build(m: &mut MeshBuilder, input: &BridgeInput<'_>) -> u32 {
    let side = CHUNK_M as f32;
    let owned = |p: Vec2| (0.0..side).contains(&p.x) && (0.0..side).contains(&p.y);
    let mut count = 0;
    for line in spines(input.lines, input.outlines) {
        let Some(profile) = profile(line, input) else {
            continue;
        };
        let before = m.vertex_count();
        draw(m, line, &profile, &owned);
        if m.vertex_count() > before {
            count += 1;
        }
    }
    m.counts.bridges += count;
    count
}

fn draw(m: &mut MeshBuilder, line: &BridgeLine, p: &Profile, owned: &dyn Fn(Vec2) -> bool) {
    let n = p.points.len();
    let look = hash01(line.key, 0, 601);
    let steel = Paint::new(pick(&STEEL_COLOURS, look), Material::Steel);
    let concrete = Paint::new(CONCRETE, Material::Concrete);
    let road = Paint {
        colour: if line.rail {
            [112, 104, 96]
        } else {
            [70, 70, 72]
        },
        material: if line.rail {
            Material::Concrete
        } else {
            Material::Deck
        },
        param: 0,
    };
    let in_span = |s: f32| p.towers.is_some_and(|(a, b)| s > a && s < b);
    m.set_group(Group::Structure);
    // The deck: its cross-section (across, up from the road) swept along the line, segment by segment.
    for k in 0..n - 1 {
        let mid = (p.points[k] + p.points[k + 1]) * 0.5;
        if !owned(mid) {
            continue;
        }
        let parapet = if line.rail { 0.5 } else { 0.9 };
        let section = |j: usize| -> [Vec3; 8] {
            let (l, r, d) = (p.left[j], p.right[j], p.depth);
            let inset = (d * 0.6).min(l.min(r) * 0.4);
            let rv = right_at(&p.points, j);
            let at = |across: f32, up: f32| {
                let q = p.points[j] + rv * across;
                Vec3::new(q.x, p.deck[j] + up, q.y)
            };
            [
                at(-l + inset, -d),
                at(r - inset, -d),
                at(r, parapet),
                at(r - 0.3, parapet),
                at(r - 0.3, 0.0),
                at(-l + 0.3, 0.0),
                at(-l + 0.3, parapet),
                at(-l, parapet),
            ]
        };
        let (s0, s1) = (section(k), section(k + 1));
        let centre = |s: &[Vec3; 8]| s.iter().copied().sum::<Vec3>() / 8.0;
        let c0 = centre(&s0);
        let spanned = in_span(p.along[k]);
        for e in 0..8 {
            let f = (e + 1) % 8;
            let quad = [s0[e], s0[f], s1[f], s1[e]];
            // Outward: from the section's middle (the road's top faces up, the parapets' insides in).
            let mid_edge = (s0[e] + s0[f]) * 0.5;
            let mut out = mid_edge - c0;
            if e == 4 {
                out = Vec3::Y;
            } else if e == 3 || e == 5 {
                // The parapets' inner faces look over the road.
                out = (c0 - mid_edge).with_y(0.0);
            }
            let paint = if e == 4 {
                road
            } else if spanned && (e == 0 || e == 1 || e == 7) {
                steel
            } else {
                concrete
            };
            let along = [p.along[k], p.along[k], p.along[k + 1], p.along[k + 1]];
            let across =
                |v: Vec3, j: usize| (Vec2::new(v.x, v.z) - p.points[j]).dot(right_at(&p.points, j));
            let uv = if e == 4 {
                let param = ((p.left[k] + p.right[k]) * 0.5 - 0.3)
                    .round()
                    .clamp(1.0, 60.0) as i8;
                m.quad(
                    quad,
                    [
                        Vec2::new(
                            along[0],
                            across(quad[0], k) + (p.left[k] - p.right[k]) * 0.5,
                        ),
                        Vec2::new(
                            along[1],
                            across(quad[1], k) + (p.left[k] - p.right[k]) * 0.5,
                        ),
                        Vec2::new(
                            along[2],
                            across(quad[2], k + 1) + (p.left[k + 1] - p.right[k + 1]) * 0.5,
                        ),
                        Vec2::new(
                            along[3],
                            across(quad[3], k + 1) + (p.left[k + 1] - p.right[k + 1]) * 0.5,
                        ),
                    ],
                    out,
                    Paint { param, ..paint },
                );
                continue;
            } else {
                [
                    Vec2::new(along[0], quad[0].y),
                    Vec2::new(along[1], quad[1].y),
                    Vec2::new(along[2], quad[2].y),
                    Vec2::new(along[3], quad[3].y),
                ]
            };
            m.quad(quad, uv, out, paint);
        }
        // The deck's ends, where the bridge ends: the slab's trapezoid (the parapets' ends are left open).
        let dir = (p.points[k + 1] - p.points[k]).normalize_or(Vec2::X);
        let dir = Vec3::new(dir.x, 0.0, dir.y);
        for (s, out, end) in [(&s0, -dir, k == 0), (&s1, dir, k + 2 == n)] {
            if end {
                let slab = [s[0], s[1], s[4], s[5]];
                let uv = slab.map(|v| Vec2::new(v.x + v.z, v.y));
                m.face(&slab, &uv, out, concrete);
            }
        }
    }
    // Piers where the deck stands clear of the ground, none within a suspension span.
    let width = |k: usize| p.left[k] + p.right[k];
    let spacing = if p.depth > 3.0 {
        90.0
    } else if p.depth > 2.0 {
        55.0
    } else {
        32.0
    };
    let length = *p.along.last().unwrap_or(&0.0);
    let mut s = spacing * 0.5 + (length % spacing) * 0.5;
    while s < length - 4.0 {
        let k = p.along.partition_point(|a| *a < s).min(n - 1);
        let at = p.points[k];
        let bottom = p.deck[k] - p.depth;
        // Into the ground or the seabed (no deeper than 40 m: the sea hides the rest).
        let foot = p.ground[k].max(-40.0) - 2.0;
        if owned(at) && bottom - p.ground[k].max(p.water) > 2.5 && !p.in_channel(s) {
            let rv = right_at(&p.points, k);
            let dir = Vec2::new(rv.y, -rv.x);
            let offset = (p.right[k] - p.left[k]) * 0.5;
            let centre = at + rv * offset;
            let tall = bottom - foot > 25.0;
            if tall {
                for side in [-1.0f32, 1.0] {
                    let c = centre + rv * (side * width(k) * 0.3);
                    let ring = square(c, dir, 1.4, 1.4);
                    m.prism(&ring, foot, bottom - 1.2, false, concrete);
                }
                let cap = square(centre, dir, 1.6, width(k) * 0.42);
                m.prism(&cap, bottom - 1.2, bottom, false, concrete);
            } else {
                let ring = square(centre, dir, 1.0, width(k) * 0.32);
                m.prism(&ring, foot, bottom, false, concrete);
            }
        }
        s += spacing;
    }
    if let Some((ta, tb)) = p.towers {
        suspension(m, p, ta, tb, steel, owned);
    }
}

/// A square ring about `c`, `dir` its first axis, half extents along it and across.
fn square(c: Vec2, dir: Vec2, half_along: f32, half_across: f32) -> Vec<Vec2> {
    let a = dir * half_along;
    let x = Vec2::new(-dir.y, dir.x) * half_across;
    vec![c - a - x, c + a - x, c + a + x, c - a + x]
}

/// A suspension bridge's two towers (at `ta` and `tb` along the line), its two main cables over the main span and
/// down the side spans to the deck, and the hangers (near only).
fn suspension(
    m: &mut MeshBuilder,
    p: &Profile,
    ta: f32,
    tb: f32,
    steel: Paint,
    owned: &dyn Fn(Vec2) -> bool,
) {
    let n = p.points.len();
    let sample = |s: f32| p.along.partition_point(|a| *a < s).min(n - 1);
    let span = tb - ta;
    // A tenth of the span's sag, the cable's lowest 3 m over the deck.
    let rise = span * 0.1 + 3.0;
    let deck_at = |s: f32| p.deck[sample(s)];
    let top = deck_at(ta).max(deck_at(tb)) + rise;
    // Each cable's place across the line: just outside the deck's edges.
    let across = |k: usize, side: f32| {
        if side < 0.0 {
            -(p.left[k] + 0.8)
        } else {
            p.right[k] + 0.8
        }
    };
    let point = |s: f32, side: f32, y: f32| {
        let k = sample(s);
        let q = p.points[k] + right_at(&p.points, k) * across(k, side);
        Vec3::new(q.x, y, q.y)
    };
    m.set_group(Group::Structure);
    for t in [ta, tb] {
        let k = sample(t);
        if !owned(p.points[k]) {
            continue;
        }
        let foot = p.ground[k].min(0.0) - 2.0;
        let legs = [point(t, -1.0, foot), point(t, 1.0, foot)];
        for leg in legs {
            m.beam(leg, Vec3::new(leg.x, top, leg.z), 2.0, steel);
        }
        for y in [
            p.deck[k] - p.depth - 1.5,
            p.deck[k] + rise * 0.55,
            top - 2.0,
        ] {
            m.beam(
                Vec3::new(legs[0].x, y, legs[0].z),
                Vec3::new(legs[1].x, y, legs[1].z),
                1.4,
                steel,
            );
        }
    }
    // The cables: a parabola over the main span from tower top to tower top, its lowest 3 m over the deck; straight
    // down each side span to the deck a third of the span beyond the tower.
    let low = deck_at((ta + tb) * 0.5) + 3.0;
    let sag = top - low;
    let main = |s: f32| {
        let t = (s - ta) / span;
        top - sag * 4.0 * t * (1.0 - t)
    };
    let back = span * 0.35;
    let length = *p.along.last().unwrap_or(&0.0);
    let (a0, b1) = ((ta - back).max(0.0), (tb + back).min(length));
    let pieces = |m: &mut MeshBuilder, from: f32, to: f32, height: &dyn Fn(f32) -> f32| {
        let steps = ((to - from) / 15.0).ceil().max(1.0) as usize;
        for k in 0..steps {
            let (s0, s1) = (
                from + (to - from) * k as f32 / steps as f32,
                from + (to - from) * (k + 1) as f32 / steps as f32,
            );
            if !owned(p.points[sample((s0 + s1) * 0.5)]) {
                continue;
            }
            for side in [-1.0, 1.0] {
                m.beam(
                    point(s0, side, height(s0)),
                    point(s1, side, height(s1)),
                    0.45,
                    steel,
                );
            }
        }
    };
    pieces(m, ta, tb, &main);
    let deck_a = deck_at(a0) + 1.0;
    pieces(m, a0, ta, &|s| {
        deck_a + (top - deck_a) * (s - a0) / (ta - a0).max(1.0)
    });
    let deck_b = deck_at(b1) + 1.0;
    pieces(m, tb, b1, &|s| {
        top + (deck_b - top) * (s - tb) / (b1 - tb).max(1.0)
    });
    // The hangers, every 15 m of the main span (near only).
    if !m.near() {
        return;
    }
    m.set_group(Group::Detail);
    let mut s = ta + 15.0;
    while s < tb - 7.0 {
        if owned(p.points[sample(s)]) {
            for side in [-1.0, 1.0] {
                let y = main(s);
                let d = deck_at(s);
                if y - d > 1.5 {
                    m.beam(point(s, side, d + 0.9), point(s, side, y), 0.1, steel);
                }
            }
        }
        s += 15.0;
    }
    m.set_group(Group::Structure);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight road from x = 0 to 2400 along z = 500: land to x = 900 and from x = 1500, the sea (12 m deep) between.
    fn channel() -> (BridgeLine, impl Fn(Vec2) -> f32, impl Fn(Vec2) -> Under) {
        let line = BridgeLine {
            points: vec![
                Vec2::new(0.0, 500.0),
                Vec2::new(1200.0, 500.0),
                Vec2::new(2400.0, 500.0),
            ],
            half: 7.0,
            rail: false,
            key: 42,
        };
        let ground = |p: Vec2| {
            if (900.0..1500.0).contains(&p.x) {
                -12.0
            } else {
                4.0
            }
        };
        let under = |p: Vec2| {
            if (900.0..1500.0).contains(&p.x) {
                Under::Sea
            } else {
                Under::Dry
            }
        };
        (line, ground, under)
    }

    #[test]
    fn a_long_sea_span_stands_high_on_towers() {
        let (line, ground, under) = channel();
        let input = BridgeInput {
            lines: std::slice::from_ref(&line),
            outlines: &[],
            ground: &ground,
            under: &under,
        };
        let p = profile(&line, &input).unwrap();
        assert_eq!(p.under, Under::Sea);
        let (a, b) = p.span;
        assert!(
            (a - 896.0).abs() < 8.0 && (b - 1504.0).abs() < 8.0,
            "{a} {b}"
        );
        // A 600 m span: 6 + 600 / 9 = 72.7, held to 62 m.
        let c = p.clearance();
        assert!((60.0..=62.5).contains(&c), "clearance {c}");
        // No pier within the span; piers on the approaches.
        assert!(p.in_channel(1200.0) && !p.in_channel(400.0));
        // The ends meet the road on the ground, and no grade is steeper than 6 percent.
        assert!((p.deck[0] - 4.4).abs() < 0.5);
        assert!((p.deck.last().unwrap() - 4.4).abs() < 0.5);
        for k in 1..p.deck.len() {
            let grade = (p.deck[k] - p.deck[k - 1]).abs() / (p.along[k] - p.along[k - 1]);
            assert!(grade < 0.06, "grade {grade} at {}", p.along[k]);
        }
        assert!(p.towers.is_some());
        let mut m = MeshBuilder::default();
        assert_eq!(build(&mut m, &input), 1);
        let mesh = m.finish();
        // The towers rise a tenth of the span above the deck; hangers drawn near only.
        assert!(mesh.max[1] > 62.0 + 55.0, "top {}", mesh.max[1]);
        assert!(!mesh.range(Group::Detail).is_empty());
        assert_eq!(mesh.counts.bridges, 1);
    }

    #[test]
    fn a_short_creek_bridge_stands_low_and_dry_bridges_not_at_all() {
        let line = BridgeLine {
            points: vec![Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0)],
            half: 4.0,
            rail: false,
            key: 1,
        };
        let ground = |_: Vec2| 3.0;
        let under = |p: Vec2| {
            if (120.0..140.0).contains(&p.x) {
                Under::Inland
            } else {
                Under::Dry
            }
        };
        let input = BridgeInput {
            lines: std::slice::from_ref(&line),
            outlines: &[],
            ground: &ground,
            under: &under,
        };
        let p = profile(&line, &input).unwrap();
        assert_eq!(p.under, Under::Inland);
        assert!(p.towers.is_none());
        let c = p.clearance();
        assert!((0.5..=4.0).contains(&c), "clearance {c}");
        let dry = |_: Vec2| Under::Dry;
        let input = BridgeInput {
            under: &dry,
            ..input
        };
        assert!(profile(&line, &input).is_none());
    }

    #[test]
    fn an_outline_widens_the_deck_and_keeps_one_of_its_ways() {
        let (line, ground, under) = channel();
        let mut other = line.clone();
        other.points = other
            .points
            .iter()
            .map(|p| *p + Vec2::new(0.0, 12.0))
            .collect();
        other.points[0].x += 30.0;
        let ring = vec![
            [-10.0, 485.0],
            [1610.0, 485.0],
            [1610.0, 525.0],
            [-10.0, 525.0],
        ];
        let lines = [line.clone(), other];
        let outlines = [ring];
        let kept = spines(&lines, &outlines);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0], &lines[0]);
        let input = BridgeInput {
            lines: &lines,
            outlines: &outlines,
            ground: &ground,
            under: &under,
        };
        let p = profile(&line, &input).unwrap();
        let k = p.points.len() / 2;
        assert!(
            (p.left[k] - 15.0).abs() < 0.5 && (p.right[k] - 25.0).abs() < 0.5,
            "{} {}",
            p.left[k],
            p.right[k]
        );
    }
}
