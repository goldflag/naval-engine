//! Taiwan's streets today (native/docs/terrain/buildings.md, "Taiwan's streets"): the arcades (騎樓) its rows stand
//! over along the main roads, their ground floors set back behind a row of columns under the floors above; the shop
//! signs (招牌) crowding a shop's front, boards standing out from the facade and along the arcade's top; and the air
//! conditioners (冷氣室外機) on its homes' walls. The arcade is the building's own mesh (`MeshBuilder::arcade_walls`);
//! the signs and the air conditioners are clutter ([`Shape::Sign`], [`Shape::Unit`]).
//!
//! Every choice hashes the building's place and seed, and an arcade reads only its frontage against the road beside
//! it, so a row's neighbours along one road take it alike and a chunk made again holds the same.

use super::clutter::{Clutter, Shape};
use super::geometry;
use super::grounds::Roads;
use super::mesh::PART_GABLE;
use super::{Building, Kind, Region, hash01};

/// A main road's half width (m), least and most: OpenMapTiles' secondary (4), primary (4.5) and trunk (5) roads, not
/// a tertiary road or a ramp (3.5) nor a motorway's carriageway (6.5).
pub const MAIN_HALF_M: [f32; 2] = [3.9, 5.5];
/// A street a shop fronts: a minor road (3) or wider, not a service lane (2.5) or a railway (2.5).
pub const STREET_HALF_M: f32 = 2.9;
/// How far a frontage may stand back from its road's edge (m), over clear ground: a main road's (its stroke is its
/// class's width, a wide avenue's carriageways and pavements are not) and a street's.
pub const MAIN_SETBACK_M: f32 = 16.0;
pub const STREET_SETBACK_M: f32 = 8.0;
/// The walkway's depth behind the frontage (m): the old rule's 12 feet on a secondary road, 4 m on a wider one.
pub const ARCADE_DEPTH_M: [f32; 2] = [3.6, 4.0];
/// The air conditioners on one building, at most: a townhouse's, a walk-up's or a tower's.
const UNITS_MOST: [usize; 3] = [6, 10, 10];

/// A building's arcade: its frontage from `a` to `c` (chunk-local x, z, counter-clockwise along its ring) facing `out`
/// (a unit vector) onto its main road, the walkway `depth` m deep behind it under a ceiling at `top` (m above the sea),
/// its floor at `floor`; `half` the road's half width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arcade {
    /// The frontage's edge of the ring (from `a` to `c`).
    pub edge: usize,
    pub a: [f32; 2],
    pub c: [f32; 2],
    pub out: [f32; 2],
    pub depth: f32,
    pub top: f32,
    pub floor: f32,
    pub half: f32,
}

impl Arcade {
    /// How far `p` lies in front of the walkway's back wall (m): over 0 under the walkway (or out past the frontage).
    pub fn ahead(&self, p: [f32; 2]) -> f32 {
        (p[0] - self.a[0]) * self.out[0] + (p[1] - self.a[1]) * self.out[1] + self.depth
    }

    /// Whether `other`'s walkway runs on from this one's: the same back wall's line, so the two open into each other.
    pub fn runs_on(&self, other: &Arcade) -> bool {
        self.out[0] * other.out[0] + self.out[1] * other.out[1] > 0.97
            && self
                .ahead([
                    other.a[0] - other.out[0] * other.depth,
                    other.a[1] - other.out[1] * other.depth,
                ])
                .abs()
                < 0.3
    }

    /// `ring` cut along the back wall's line: what stands behind it (the shops), or the walkway's floor in front of it
    /// (`front`). Counter-clockwise as the ring is; empty when nothing is left.
    pub fn cut(&self, ring: &[[f32; 2]], front: bool) -> Vec<[f32; 2]> {
        let side = |p: [f32; 2]| if front { self.ahead(p) } else { -self.ahead(p) };
        let mut out = Vec::new();
        let n = ring.len();
        for k in 0..n {
            let (p, q) = (ring[k], ring[(k + 1) % n]);
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                let t = sp / (sp - sq);
                out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        if out.len() < 3 { Vec::new() } else { out }
    }

    /// Whether the stretch from `p` to `q` lies along the back wall's line.
    pub fn on_back(&self, p: [f32; 2], q: [f32; 2]) -> bool {
        self.ahead(p).abs() < 1e-3 && self.ahead(q).abs() < 1e-3
    }

    /// Where the columns stand along the frontage (m from `a`), and how wide each is: one inside each end (a row's
    /// neighbours' pair make one pier over their party wall), and between them no span over 6.5 m.
    pub fn columns(&self) -> (Vec<f32>, f32) {
        const WIDE: f32 = 0.42;
        let len = geometry::distance(self.a, self.c);
        let spans = ((len - WIDE) / 6.5).ceil().max(1.0) as usize;
        let at = (0..=spans)
            .map(|k| 0.5 * WIDE + (len - WIDE) * k as f32 / spans as f32)
            .collect();
        (at, WIDE)
    }
}

/// A building's frontage onto a road: its ring's edge `edge` facing the road, and that road's half width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frontage {
    pub edge: usize,
    pub half: f32,
}

/// The edge of `b` that fronts a road whose half width lies within `half` (m): at least 3 m long, along the road
/// within about 20°, the road before it, its edge within `setback` over ground `clear` says no other building stands
/// on; the nearest of them.
pub fn frontage(
    b: &Building,
    roads: &Roads,
    half: [f32; 2],
    setback: f32,
    clear: &dyn Fn([f32; 2]) -> bool,
) -> Option<Frontage> {
    frontages(b, roads, half, setback, clear)
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, f)| f)
}

/// Every edge of `b` fronting such a road ([`frontage`]), each with its gap to the road's edge (m).
fn frontages(
    b: &Building,
    roads: &Roads,
    half: [f32; 2],
    setback: f32,
    clear: &dyn Fn([f32; 2]) -> bool,
) -> Vec<(f32, Frontage)> {
    let ring = &b.ring;
    let n = ring.len();
    let mut all: Vec<(f32, Frontage)> = Vec::new();
    for i in 0..n {
        let (a, c) = (ring[i], ring[(i + 1) % n]);
        let len = geometry::distance(a, c);
        if len < 3.0 {
            continue;
        }
        let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
        let out = [d[1], -d[0]];
        let mid = [0.5 * (a[0] + c[0]), 0.5 * (a[1] + c[1])];
        let mut best: Option<(f32, Frontage)> = None;
        for r in roads.around(mid, half[1] + setback) {
            if r.half < half[0] || r.half > half[1] {
                continue;
            }
            let rl = geometry::distance(r.a, r.b);
            if rl < 1.0 {
                continue;
            }
            let rd = [(r.b[0] - r.a[0]) / rl, (r.b[1] - r.a[1]) / rl];
            if (rd[0] * d[0] + rd[1] * d[1]).abs() < 0.94 {
                continue;
            }
            let t = ((mid[0] - r.a[0]) * rd[0] + (mid[1] - r.a[1]) * rd[1]).clamp(0.0, rl);
            let foot = [r.a[0] + rd[0] * t, r.a[1] + rd[1] * t];
            let to = [foot[0] - mid[0], foot[1] - mid[1]];
            let dist = to[0].hypot(to[1]);
            let gap = dist - r.half;
            if gap > setback || to[0] * out[0] + to[1] * out[1] < 0.8 * dist {
                continue;
            }
            // Nothing stands between: the ground clear from just before the edge's middle to the road.
            let steps = (gap / 1.5).ceil().max(1.0) as usize;
            if !(0..=steps).all(|k| {
                let d = 0.5 + (gap - 0.5).max(0.0) * k as f32 / steps as f32;
                clear([mid[0] + out[0] * d, mid[1] + out[1] * d])
            }) {
                continue;
            }
            if best.is_none_or(|(g, _)| gap < g) {
                best = Some((
                    gap,
                    Frontage {
                        edge: i,
                        half: r.half,
                    },
                ));
            }
        }
        all.extend(best);
    }
    all
}

/// The arcade of `b` in `region` (Taiwan's, today: [`Region::in_period`] makes a past Taiwan East Asia): a townhouse's,
/// a walk-up's or a tower's of two storeys or more along a main road ([`MAIN_HALF_M`]), deep enough to stand back
/// [`ARCADE_DEPTH_M`] and still hold its shops, on fairly level ground, its walkway's back wall crossing the outline
/// twice (a plain cut). `ground` the ground at a chunk-local point; `clear` whether no other building stands on one.
pub fn arcade_of(
    b: &Building,
    region: Region,
    roads: &Roads,
    ground: &dyn Fn([f32; 2]) -> f32,
    clear: &dyn Fn([f32; 2]) -> bool,
) -> Option<Arcade> {
    if region != Region::Taiwan
        || b.period.is_past()
        || b.base > 0.0
        || b.storeys < 2
        || !matches!(b.kind, Kind::Terrace | Kind::Block | Kind::Tower)
    {
        return None;
    }
    let f = frontage(b, roads, MAIN_HALF_M, MAIN_SETBACK_M, clear)?;
    let n = b.ring.len();
    let (a, c) = (b.ring[f.edge], b.ring[(f.edge + 1) % n]);
    let len = geometry::distance(a, c);
    let out = [(c[1] - a[1]) / len, -(c[0] - a[0]) / len];
    let depth = if f.half >= 4.4 {
        ARCADE_DEPTH_M[1]
    } else {
        ARCADE_DEPTH_M[0]
    };
    let mut arcade = Arcade {
        edge: f.edge,
        a,
        c,
        out,
        depth,
        top: b.ground[1] + b.storey_m,
        floor: 0.0,
        half: f.half,
    };
    // Deep enough behind the walkway for its shops.
    let deep = b
        .ring
        .iter()
        .map(|p| -((p[0] - a[0]) * out[0] + (p[1] - a[1]) * out[1]))
        .fold(0.0, f32::max);
    if deep < depth + 4.0 {
        return None;
    }
    // The back wall's line crossing the outline twice.
    let crossings = (0..n)
        .filter(|&k| (arcade.ahead(b.ring[k]) >= 0.0) != (arcade.ahead(b.ring[(k + 1) % n]) >= 0.0))
        .count();
    if crossings != 2 {
        return None;
    }
    // Its floor a step over the highest ground under it; none on a slope steeper than a metre over the walkway.
    let back = |p: [f32; 2]| [p[0] - out[0] * depth, p[1] - out[1] * depth];
    let heights = [ground(a), ground(c), ground(back(a)), ground(back(c))];
    let (lo, hi) = heights
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), g| (l.min(*g), h.max(*g)));
    if hi - lo > 1.0 {
        return None;
    }
    arcade.floor = hi + 0.15;
    let eave = b.ground[1] + b.height;
    if arcade.top < arcade.floor + 2.6 || arcade.top > eave - 2.0 {
        return None;
    }
    Some(arcade)
}

/// The shop signs' colours (`buildings.wgsl`'s `SIGN_COLOURS`): the boards' and their marks', light and dark.
const SIGN_LIGHT: [u8; 5] = [0, 1, 2, 3, 4];
const SIGN_DARK: [u8; 7] = [5, 6, 7, 8, 9, 10, 11];

fn sign_style(seed: u8, h: f32, k: f32) -> [u8; 4] {
    // Light boards with red, blue, green or black marks; dark ones (red, blue, green, orange, purple, black) with white
    // or yellow.
    let (bg, ink) = if h < 0.45 {
        let bg = SIGN_LIGHT[(k * 4.99) as usize];
        (bg, [5u8, 6, 7, 10, 11][(fract(k * 7.3) * 4.99) as usize])
    } else {
        let bg = SIGN_DARK[(k * 6.99) as usize];
        (bg, [0u8, 1, 0, 1][(fract(k * 5.1) * 3.99) as usize])
    };
    [bg, ink, seed, Kind::Shed as u8 | (PART_GABLE << 4)]
}

fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// The shop signs on `b`'s frontage `f` (its arcade's, or a shop's onto a street): a board along the arcade's top (or
/// over the ground floor, under the eaves) for each shop, and boards standing out from the facade above it, over the
/// walkway; each where no other building stands (`open`: whether none does at a point). Whether it carries any.
pub fn signs(
    b: &Building,
    arcade: Option<&Arcade>,
    f: Frontage,
    open: &dyn Fn([f32; 2]) -> bool,
    out: &mut Vec<Clutter>,
) -> bool {
    let n = b.ring.len();
    let (a, c) = (b.ring[f.edge], b.ring[(f.edge + 1) % n]);
    let len = geometry::distance(a, c);
    let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
    let o = [d[1], -d[0]];
    let mid = geometry::centroid(&b.ring);
    let h = |k: i64| {
        hash01(
            (mid[0] * 8.0) as i64 + i64::from(b.seed) * 7919,
            (mid[1] * 8.0) as i64,
            900 + k,
        )
    };
    // Most shops along a main road carry them; fewer along a side street's.
    let share = match (arcade.is_some(), b.kind) {
        (true, _) => 0.92,
        (false, Kind::Terrace) => 0.5,
        (false, Kind::Block) => 0.55,
        _ => 0.3,
    };
    if h(0) >= share {
        return false;
    }
    let start = out.len();
    let ground = b.ground[1];
    let storey = b.storey_m;
    let first = arcade.map_or(ground + storey, |a| a.top);
    let eave = ground + b.height;
    // A townhouse one shop; a walk-up's or a tower's front a shop every 4-6 m.
    let shops = if b.kind == Kind::Terrace {
        1
    } else {
        (len / 5.0).round().max(1.0) as usize
    };
    let along_yaw = d[1].atan2(d[0]);
    let out_yaw = o[1].atan2(o[0]);
    let at = |t: f32, off: f32| [a[0] + d[0] * t + o[0] * off, a[1] + d[1] * t + o[1] * off];
    // A board's footprint, from `t0` to `t1` along the frontage and from `off[0]` to `off[1]` out from it, clear of every
    // other building: a neighbour may stand before an outer shop though the frontage's middle is clear.
    let clear = |t0: f32, t1: f32, off: [f32; 2]| {
        [t0, 0.5 * (t0 + t1), t1]
            .iter()
            .all(|&t| off.iter().all(|&o| open(at(t, o))))
    };
    for s in 0..shops {
        let k = s as i64 * 16;
        let (t0, t1) = (
            len * s as f32 / shops as f32,
            len * (s + 1) as f32 / shops as f32,
        );
        // Along the arcade's top: the shop's width but a little, 0.9-1.3 m high; on a low building under its eaves,
        // over the door, or none.
        let high = (0.9 + 0.4 * h(k + 2)).min(0.45 * storey);
        let wide = (t1 - t0 - 0.3).max(1.0);
        let y = (first + 0.08).min(eave - 0.05 - high);
        let middle = 0.5 * (t0 + t1);
        if h(k + 1) < 0.88
            && y >= ground + 2.2
            && clear(middle - 0.5 * wide, middle + 0.5 * wide, [0.01, 0.21])
        {
            let p = at(middle, 0.11);
            out.push(Clutter {
                at: [p[0], y, p[1]],
                yaw: along_yaw,
                size: [wide, high, 0.2],
                shape: Shape::Sign as u32,
                style: sign_style(b.seed, h(k + 3), h(k + 4)),
            });
        }
        // Standing out over the walkway: one or two, at the shop's ends, from over the board up a storey or two.
        let standing = if h(k + 5) < 0.3 { 2 } else { 1 };
        for j in 0..standing {
            if h(k + 6 + j) >= if arcade.is_some() { 0.85 } else { 0.6 } {
                continue;
            }
            let bottom = first + 1.45 + storey * (j as f32) * 0.15;
            let room = eave - 0.4 - bottom;
            if room < 1.6 {
                continue;
            }
            let storeys = if b.kind == Kind::Tower { 3.0 } else { 2.0 };
            let high = (storey * (1.0 + (storeys - 1.0) * h(k + 8 + j))).min(room);
            let wide = 0.75 + 0.55 * h(k + 10 + j);
            let end = if (j == 0) == (h(k + 12) < 0.5) {
                t0 + 0.35
            } else {
                t1 - 0.35
            };
            if !clear(end - 0.11, end + 0.11, [0.06, 0.06 + wide]) {
                continue;
            }
            let p = at(end, 0.06 + 0.5 * wide);
            out.push(Clutter {
                at: [p[0], bottom, p[1]],
                yaw: out_yaw,
                size: [wide, high, 0.22],
                shape: Shape::Sign as u32,
                style: sign_style(b.seed, h(k + 13 + j), h(k + 14 + j)),
            });
        }
    }
    out.len() > start
}

/// The air conditioners on `b`'s walls: on a townhouse's, a walk-up's or a tower's storeys over its ground floor, under
/// a window or beside it, a split unit's box on its brackets or a window unit standing out of the wall, white, grey or
/// beige, now and then rusted: crowding the walls that front a street (`roads`), a few on its other walls but none on
/// one against a neighbour's nor where one stands over a unit's own span (`open`: whether no other building stands at
/// a point), nor over the signs on the frontage `sign_front` (an edge of its ring); [`UNITS_MOST`] at most, spread
/// over its walls.
pub fn units(
    b: &Building,
    roads: &Roads,
    sign_front: Option<usize>,
    open: &dyn Fn([f32; 2]) -> bool,
    out: &mut Vec<Clutter>,
) {
    let (share, most): (f32, usize) = match b.kind {
        Kind::Terrace => (0.45, UNITS_MOST[0]),
        Kind::Block => (0.36, UNITS_MOST[1]),
        Kind::Tower => (0.1, UNITS_MOST[2]),
        _ => return,
    };
    let fronts: Vec<usize> = frontages(b, roads, [1.7, 7.0], STREET_SETBACK_M, open)
        .into_iter()
        .map(|(_, f)| f.edge)
        .collect();
    let ring = &b.ring;
    let n = ring.len();
    let mid = geometry::centroid(ring);
    let h = |k: i64| {
        hash01(
            (mid[0] * 8.0) as i64 + i64::from(b.seed) * 7919,
            (mid[1] * 8.0) as i64,
            1100 + k,
        )
    };
    let storey = b.storey_m;
    let top = b.storeys.min(16);
    // The walls clear of a neighbour's, each with its bays, and the share that keeps the building to its most.
    let walls: Vec<(usize, f32)> = (0..n)
        .filter_map(|i| {
            let (a, c) = (ring[i], ring[(i + 1) % n]);
            let len = geometry::distance(a, c);
            if len < 2.5 {
                return None;
            }
            let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
            let outside = |t: f32| {
                [
                    a[0] + d[0] * len * t + d[1] * 0.8,
                    a[1] + d[1] * len * t - d[0] * 0.8,
                ]
            };
            [0.2, 0.5, 0.8]
                .iter()
                .all(|t| open(outside(*t)))
                .then(|| (i, (len / b.bay.max(0.5)).round().max(1.0)))
        })
        .collect();
    let lowest = |i: usize| if Some(i) == sign_front { 2 } else { 1 };
    // A street's front its full share, the building's other walls a fifth of it.
    let weight = |i: usize| if fronts.contains(&i) { 1.0 } else { 0.2 };
    let slots: f32 = walls
        .iter()
        .map(|&(i, bays)| weight(i) * bays * f32::from(top.saturating_sub(lowest(i))))
        .sum();
    let share = share.min(1.2 * most as f32 / slots.max(1.0));
    let start = out.len();
    for &(i, bays) in &walls {
        let (a, c) = (ring[i], ring[(i + 1) % n]);
        let len = geometry::distance(a, c);
        let d = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
        let o = [d[1], -d[0]];
        let bay = len / bays;
        let yaw = o[1].atan2(o[0]);
        for k in lowest(i)..top {
            for j in 0..bays as usize {
                let key = (i as i64 * 64 + j as i64) * 32 + i64::from(k);
                if h(key * 4) >= share * weight(i) {
                    continue;
                }
                // A split unit (most) or a window unit; under the window or beside it.
                let split = h(key * 4 + 1) < 0.65;
                let (wide, high, deep) = if split {
                    (0.78 + 0.1 * h(key * 4 + 2), 0.55, 0.3)
                } else {
                    (0.6, 0.42, 0.5)
                };
                let beside = h(key * 4 + 3) < 0.45 && bay > 2.4;
                let t = if beside {
                    let edge = if h(key * 4 + 2) < 0.5 {
                        0.5 * wide + 0.15
                    } else {
                        bay - 0.5 * wide - 0.15
                    };
                    j as f32 * bay + edge
                } else {
                    (j as f32 + 0.5) * bay + 0.3 * (h(key * 4 + 3) - 0.5)
                };
                let y = b.ground[1]
                    + f32::from(k) * storey
                    + if beside { 0.32 * storey } else { 0.06 * storey };
                let gap: f32 = if split { 0.12 } else { 0.0 };
                // Its own span clear of every other building: a wall may stand against a neighbour's over a stretch
                // the wall's own test missed.
                let off =
                    |s: f32, out: f32| [a[0] + d[0] * s + o[0] * out, a[1] + d[1] * s + o[1] * out];
                if ![t - 0.5 * wide, t, t + 0.5 * wide].iter().all(|&s| {
                    [gap.max(0.05), gap + deep]
                        .iter()
                        .all(|&out| open(off(s, out)))
                }) {
                    continue;
                }
                let p = off(t, gap + 0.5 * deep);
                // White (most), light grey, beige or grey; rusted on one in four.
                let colour = [0u8, 0, 0, 1, 1, 2, 3][(h(key * 4 + 2) * 6.99) as usize];
                let rust = if fract(h(key * 4 + 1) * 3.7) < 0.25 {
                    1 + (h(key) * 2.99) as u8
                } else {
                    0
                };
                out.push(Clutter {
                    at: [p[0], y, p[1]],
                    yaw,
                    size: [deep, high, wide],
                    shape: Shape::Unit as u32,
                    style: [colour, rust, b.seed, b.kind as u8 | (PART_GABLE << 4)],
                });
            }
        }
    }
    out.truncate(start + most);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::mesh::{
        BuildingVertex, ChunkInput, MeshBuilder, PART_WALL, arcade_parties, build_chunk,
    };
    use crate::buildings::tags::{MapTags, Road};
    use crate::buildings::{ChunkKey, Footprint, Lod, Period, Roof};

    fn townhouse(x: f32, storeys: u8, period: Period) -> Building {
        // 5 m wide along z = 20 (its front), 18 m deep behind it, counter-clockwise.
        Building {
            ring: vec![[x, 2.0], [x, 20.0], [x + 5.0, 20.0], [x + 5.0, 2.0]]
                .into_iter()
                .rev()
                .collect(),
            kind: Kind::Terrace,
            roof: Roof::Flat,
            ground: [3.0, 3.0, 3.0],
            height: f32::from(storeys) * 3.2,
            base: 0.0,
            storeys,
            storey_m: 3.2,
            wall: 1,
            roof_colour: 9,
            seed: 77,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period,
        }
    }

    fn road(z: f32, half: f32) -> Vec<Road> {
        vec![Road {
            a: [-200.0, z],
            b: [200.0, z],
            half,
            oneway: false,
        }]
    }

    fn flat(_: [f32; 2]) -> f32 {
        3.0
    }

    #[test]
    fn arcades_stand_only_on_taiwans_main_road_frontages() {
        let b = townhouse(0.0, 4, Period::Today);
        // A primary road 3 m off its front: the front's edge (z = 20), 4 m deep under the first floor.
        let main = road(27.5, 4.5);
        let roads = Roads::new(&main);
        let arcade = arcade_of(&b, Region::Taiwan, &roads, &flat, &|_| true).expect("an arcade");
        assert_eq!((arcade.a[1], arcade.c[1]), (20.0, 20.0));
        assert!((arcade.out[1] - 1.0).abs() < 1e-6);
        assert_eq!(arcade.depth, ARCADE_DEPTH_M[1]);
        assert!((arcade.top - 6.2).abs() < 1e-5 && (arcade.floor - 3.15).abs() < 1e-5);
        // A secondary road's 3.6 m.
        let secondary = road(27.0, 4.0);
        assert_eq!(
            arcade_of(&b, Region::Taiwan, &Roads::new(&secondary), &flat, &|_| {
                true
            })
            .map(|a| a.depth),
            Some(ARCADE_DEPTH_M[0])
        );
        // Not elsewhere, nor in a past period, nor along a lane or a motorway, nor across a road.
        assert_eq!(
            arcade_of(&b, Region::EastAsia, &roads, &flat, &|_| true),
            None
        );
        let past = townhouse(0.0, 4, Period::Year(1942));
        assert_eq!(
            arcade_of(&past, Region::Taiwan, &roads, &flat, &|_| true),
            None
        );
        for (z, half) in [(26.0, 3.0), (29.0, 6.5)] {
            let other = road(z, half);
            assert_eq!(
                arcade_of(&b, Region::Taiwan, &Roads::new(&other), &flat, &|_| true),
                None,
                "{z} {half}"
            );
        }
        let across = vec![Road {
            a: [2.5, 24.0],
            b: [2.5, 80.0],
            half: 4.5,
            oneway: false,
        }];
        assert_eq!(
            arcade_of(&b, Region::Taiwan, &Roads::new(&across), &flat, &|_| true),
            None
        );
        // Back from a wide road's stroke over clear ground (its pavement, its other lanes), but not behind another
        // building nor too far back; too shallow, a single storey, a shed.
        let wide = road(34.0, 4.5);
        let back = Roads::new(&wide);
        assert!(arcade_of(&b, Region::Taiwan, &back, &flat, &|_| true).is_some());
        let between = |q: [f32; 2]| !(22.0..26.0).contains(&q[1]);
        assert_eq!(arcade_of(&b, Region::Taiwan, &back, &flat, &between), None);
        assert_eq!(
            arcade_of(
                &b,
                Region::Taiwan,
                &Roads::new(&road(41.5, 4.5)),
                &flat,
                &|_| true
            ),
            None
        );
        let mut shallow = b.clone();
        shallow.ring = vec![[0.0, 13.0], [5.0, 13.0], [5.0, 20.0], [0.0, 20.0]];
        assert_eq!(
            arcade_of(&shallow, Region::Taiwan, &roads, &flat, &|_| true),
            None
        );
        let low = townhouse(0.0, 1, Period::Today);
        assert_eq!(
            arcade_of(&low, Region::Taiwan, &roads, &flat, &|_| true),
            None
        );
        let shed = Building {
            kind: Kind::Shed,
            ..b.clone()
        };
        assert_eq!(
            arcade_of(&shed, Region::Taiwan, &roads, &flat, &|_| true),
            None
        );

        // Its mesh: no wall of the ground storey on the frontage, the walkway's back wall 4 m behind it (its window rows
        // marked), the ceiling and floor over the walkway, two columns.
        let mut m = MeshBuilder::default().with_region(Region::Taiwan);
        m.set_arcade(Some(arcade));
        m.building(&b, Lod::Full);
        let mesh = m.finish();
        let part = |v: &BuildingVertex| (v.style[3] >> 4) & 7;
        let on_front = |v: &BuildingVertex| (v.position[2] - 20.0).abs() < 1e-3;
        assert!(
            mesh.vertices
                .iter()
                .filter(|v| part(v) == PART_WALL && on_front(v))
                .all(|v| v.position[1] >= arcade.top - 1e-3)
        );
        let back: Vec<&BuildingVertex> = mesh
            .vertices
            .iter()
            .filter(|v| {
                (v.position[2] - 16.0).abs() < 1e-3 && part(v) == PART_WALL && v.normal[2] > 100
            })
            .collect();
        assert!(!back.is_empty() && back.iter().all(|v| v.normal[3] == -5));
        let level = |y: f32, up: bool| {
            mesh.vertices.iter().any(|v| {
                (v.position[1] - y).abs() < 1e-4 && (v.normal[1] > 100) == up && v.normal[3] == -1
            })
        };
        assert!(level(arcade.top, false) && level(arcade.floor, true));
        let columns = mesh
            .vertices
            .iter()
            .filter(|v| {
                part(v) == PART_GABLE
                    && v.normal[3] == 0
                    && v.position[2] > 19.4
                    && v.position[1] <= arcade.top + 1e-3
            })
            .count();
        assert_eq!(columns, 2 * 4 * 4);
    }

    #[test]
    fn a_rows_arcades_run_on_and_a_neighbour_without_one_closes_its_end() {
        let roads_list = road(27.5, 4.5);
        let roads = Roads::new(&roads_list);
        let a = arcade_of(
            &townhouse(0.0, 4, Period::Today),
            Region::Taiwan,
            &roads,
            &flat,
            &|_| true,
        )
        .unwrap();
        let b = arcade_of(
            &townhouse(5.0, 5, Period::Today),
            Region::Taiwan,
            &roads,
            &flat,
            &|_| true,
        )
        .unwrap();
        assert!(a.runs_on(&b) && b.runs_on(&a));
        // A party wall from the back to the front at x = 5: drawn only behind the neighbour's walkway.
        let party = crate::buildings::mesh::Party {
            from: [5.0, 2.0],
            to: [5.0, 20.0],
            top: 20.0,
            of: 1,
        };
        let kept = party.outside(&a).unwrap();
        assert_eq!((kept.from, kept.to), ([5.0, 2.0], [5.0, 16.0]));
    }

    fn chunk(region: Region, period: Period) -> crate::buildings::mesh::ChunkMesh {
        // A row of townhouses along a primary road, and a walk-up on a lane behind them.
        let mut footprints: Vec<Footprint> = (0..8)
            .map(|k| {
                let x = 100.0 + 5.0 * k as f32;
                Footprint {
                    ring: vec![[x, 102.0], [x + 5.0, 102.0], [x + 5.0, 120.0], [x, 120.0]],
                    kind: Some(Kind::Terrace),
                    height: Some(13.0),
                    ..Default::default()
                }
            })
            .collect();
        footprints.push(Footprint {
            ring: vec![[100.0, 60.0], [120.0, 60.0], [120.0, 75.0], [100.0, 75.0]],
            kind: Some(Kind::Block),
            height: Some(16.0),
            ..Default::default()
        });
        let tags = MapTags {
            roads: vec![
                Road {
                    a: [0.0, 127.5],
                    b: [400.0, 127.5],
                    half: 4.5,
                    oneway: false,
                },
                Road {
                    a: [0.0, 80.0],
                    b: [400.0, 80.0],
                    half: 3.0,
                    oneway: false,
                },
            ],
            ..Default::default()
        };
        build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [5, 7],
                lod: Lod::Full,
            },
            footprints: &[],
            infill: &footprints,
            tags: &tags,
            ground: &flat,
            region,
            period,
            thinning: None,
        })
    }

    #[test]
    fn signs_and_air_conditioners_only_today_and_in_taiwan() {
        let count = |m: &crate::buildings::mesh::ChunkMesh, shape: Shape| {
            m.clutter
                .iter()
                .filter(|c| c.shape & 0xff == shape as u32)
                .count()
        };
        let now = chunk(Region::Taiwan, Period::Today);
        assert_eq!(now.buildings, 9);
        let (signs, units) = (count(&now, Shape::Sign), count(&now, Shape::Unit));
        assert!(signs >= 8, "{signs}");
        assert!(
            units >= 8 && units <= 8 * UNITS_MOST[0] + UNITS_MOST[1],
            "{units}"
        );
        // Every sign on the row's front over the walkway's ceiling or the walk-up's onto its lane, standing out over
        // the road; the boards bright.
        for c in now
            .clutter
            .iter()
            .filter(|c| c.shape & 0xff == Shape::Sign as u32)
        {
            let row = c.at[2] >= 120.0 && c.at[2] < 122.0;
            let walk_up = c.at[2] >= 75.0 && c.at[2] < 77.0;
            assert!(row || walk_up, "{:?}", c.at);
            assert!(c.at[1] > 3.0 + 2.4, "{:?}", c.at);
            assert!(c.style[0] < 12 && c.style[1] < 12 && c.style[0] != c.style[1]);
        }
        // No air conditioner on a wall against a neighbour's: every one off the row's front, back or ends, or the
        // walk-up's walls.
        for c in now
            .clutter
            .iter()
            .filter(|c| c.shape & 0xff == Shape::Unit as u32)
        {
            let [x, _, z] = c.at;
            let row =
                (z - 120.0).abs() < 0.7 || (z - 102.0).abs() < 0.7 || !(100.0..=140.0).contains(&x);
            let walk_up = (60.0 - 0.7..=75.7).contains(&z);
            assert!(row || walk_up, "{:?}", c.at);
        }
        // The row's townhouses on their arcades, the walk-up on a lane without one.
        assert!(now.vertices.iter().filter(|v| v.normal[3] < 0).count() > 8 * 10);
        assert!(
            now.vertices
                .iter()
                .all(|v| v.normal[3] >= 0 || v.position[2] > 100.0)
        );
        for (region, period) in [
            (Region::Taiwan, Period::Year(1942)),
            (Region::EastAsia, Period::Today),
            (Region::Tropical, Period::Today),
        ] {
            let m = chunk(region, period);
            assert_eq!(
                count(&m, Shape::Sign) + count(&m, Shape::Unit),
                0,
                "{region:?}"
            );
            assert!(m.vertices.iter().all(|v| v.normal[3] >= 0), "{region:?}");
        }
        // Deterministic.
        let again = chunk(Region::Taiwan, Period::Today);
        assert_eq!(now.clutter, again.clutter);
        assert_eq!(now.vertices, again.vertices);
    }

    #[test]
    fn a_rows_arcades_stepping_up_a_slope_close_the_step_between_their_ceilings() {
        // Two pieces of one row along a primary road, the second's ground (and so its walkway's ceiling) 0.5 m higher.
        let roads_list = road(27.5, 4.5);
        let roads = Roads::new(&roads_list);
        let low = townhouse(0.0, 4, Period::Today);
        let mut high = townhouse(5.0, 4, Period::Today);
        high.ground = [3.5, 3.5, 3.5];
        let arcades: Vec<Option<Arcade>> = [&low, &high]
            .iter()
            .map(|b| arcade_of(b, Region::Taiwan, &roads, &flat, &|_| true))
            .collect();
        let (a, b) = (arcades[0].unwrap(), arcades[1].unwrap());
        assert!(a.runs_on(&b) && b.runs_on(&a) && (b.top - a.top - 0.5).abs() < 1e-5);
        let mut m = MeshBuilder::default().with_region(Region::Taiwan);
        let pieces = [low.clone(), high];
        let mut parties = m.parties(&pieces, Lod::Full);
        let steps = arcade_parties(&mut parties, &arcades);
        // The walkways run on (both party walls kept whole); the lower one's wall at x = 5 under the higher one's
        // walkway, from its ceiling up to theirs, is its step.
        assert_eq!((parties[0].len(), parties[1].len()), (1, 1));
        assert_eq!(
            (parties[0][0].from, parties[0][0].to),
            ([5.0, 2.0], [5.0, 20.0])
        );
        assert!(steps[1].is_empty() && steps[0].len() == 1);
        let s = steps[0][0];
        assert!((s.from[0] - 5.0).abs() < 1e-5 && (s.from[1] - 16.0).abs() < 1e-4);
        assert_eq!((s.to, s.top), ([5.0, 20.0], b.top));
        m.set_party(parties[0].clone());
        m.set_arcade(Some(a));
        m.set_steps(steps[0].clone());
        m.building(&low, Lod::Full);
        let mesh = m.finish();
        // Its face on x = 5 toward the higher walkway, under cover, from the one ceiling to the other.
        let step: Vec<&BuildingVertex> = mesh
            .vertices
            .iter()
            .filter(|v| {
                (v.position[0] - 5.0).abs() < 1e-3
                    && v.normal[0] > 100
                    && v.position[2] > 16.0 - 1e-3
            })
            .collect();
        let at = |y: f32| {
            step.iter()
                .any(|v| (v.position[1] - y).abs() < 1e-3 && v.normal[3] == -1)
        };
        assert!(at(a.top) && at(b.top));
    }

    #[test]
    fn a_tapered_footprints_end_columns_stand_within_it() {
        // Narrowing from a 28 m frontage at z = 20 to 8 m at its back: both ends slant in behind the frontage.
        let mut b = townhouse(0.0, 4, Period::Today);
        b.ring = vec![[0.0, 0.0], [8.0, 0.0], [18.0, 20.0], [-10.0, 20.0]];
        let main = road(27.5, 4.5);
        let arcade =
            arcade_of(&b, Region::Taiwan, &Roads::new(&main), &flat, &|_| true).expect("an arcade");
        let mut m = MeshBuilder::default().with_region(Region::Taiwan);
        m.set_arcade(Some(arcade));
        m.building(&b, Lod::Full);
        let mesh = m.finish();
        let part = |v: &BuildingVertex| (v.style[3] >> 4) & 7;
        let columns: Vec<[f32; 3]> = mesh
            .vertices
            .iter()
            .filter(|v| {
                part(v) == PART_GABLE
                    && v.normal[3] == 0
                    && v.position[2] > 19.4
                    && v.position[1] <= arcade.top + 1e-3
            })
            .map(|v| v.position)
            .collect();
        assert!(!columns.is_empty());
        for [x, _, z] in columns {
            // The ends: x = 8 + z / 2 and x = -z / 2.
            assert!(
                x <= 8.0 + 0.5 * z + 1e-3 && x >= -0.5 * z - 1e-3,
                "a column's corner at ({x}, {z}) outside the footprint"
            );
        }
    }

    #[test]
    fn an_air_conditioner_keeps_off_a_neighbour_against_part_of_its_wall() {
        // A walk-up's 20 m front (z = 0) onto a lane, a neighbour against its first 3 m (x 0-3): the wall's own test
        // (at 4, 10 and 16 m) passes, so each unit's span decides.
        let mut b = townhouse(0.0, 6, Period::Today);
        b.kind = Kind::Block;
        b.ring = vec![[0.0, 0.0], [20.0, 0.0], [20.0, 12.0], [0.0, 12.0]];
        b.storey_m = 3.0;
        b.height = 18.4;
        let lane = road(-6.0, 3.0);
        let roads = Roads::new(&lane);
        let neighbour = |p: [f32; 2]| (0.0..3.0).contains(&p[0]) && (-10.0..0.0).contains(&p[1]);
        let in_it = |out: &[Clutter]| {
            out.iter()
                .any(|c| c.at[2] < 0.0 && c.at[0] - 0.5 * c.size[2] < 3.0)
        };
        let mut seen = 0;
        for seed in 0..32 {
            b.seed = seed;
            let mut all = Vec::new();
            units(&b, &roads, None, &|_| true, &mut all);
            let mut kept = Vec::new();
            units(&b, &roads, None, &|p| !neighbour(p), &mut kept);
            seen += usize::from(in_it(&all));
            assert!(!in_it(&kept), "seed {seed}");
            assert!(
                kept.iter().any(|c| c.at[2] < 0.0),
                "seed {seed}: none on the front"
            );
        }
        assert!(seen > 0);
    }

    #[test]
    fn a_neighbour_across_the_chunks_edge_keeps_signs_and_air_conditioners_off() {
        // A walk-up against the chunk's east edge, its east wall fronting a lane 7 m off; a neighbour 5 m deep between
        // them, its middle in the next chunk (`MapTags::outlines_about`).
        use crate::buildings::tags::Outline;
        let side = crate::buildings::CHUNK_M as f32;
        let walk_up = Footprint {
            ring: vec![
                [side - 20.0, 60.0],
                [side, 60.0],
                [side, 75.0],
                [side - 20.0, 75.0],
            ],
            kind: Some(Kind::Block),
            height: Some(16.0),
            ..Default::default()
        };
        let next_door = Outline {
            ring: vec![
                [side, 60.0],
                [side + 5.0, 60.0],
                [side + 5.0, 75.0],
                [side, 75.0],
            ],
            height: Some(16.0),
            min_height: 0.0,
            colour: None,
            hide_3d: false,
        };
        let east = |about: Vec<Outline>| {
            let tags = MapTags {
                roads: vec![Road {
                    a: [side + 10.0, 0.0],
                    b: [side + 10.0, 200.0],
                    half: 3.0,
                    oneway: false,
                }],
                outlines_about: about,
                ..Default::default()
            };
            let m = build_chunk(&ChunkInput {
                key: ChunkKey {
                    index: [5, 7],
                    lod: Lod::Full,
                },
                footprints: &[],
                infill: std::slice::from_ref(&walk_up),
                tags: &tags,
                ground: &flat,
                region: Region::Taiwan,
                period: Period::Today,
                thinning: None,
            });
            assert_eq!(m.buildings, 1);
            m.clutter
                .iter()
                .filter(|c| {
                    let shape = c.shape & 0xff;
                    (shape == Shape::Unit as u32 || shape == Shape::Sign as u32) && c.at[0] > side
                })
                .count()
        };
        assert!(east(Vec::new()) > 0);
        assert_eq!(east(vec![next_door.clone()]), 0);
        // A raised part's outline stands over another's: not a neighbour on the ground.
        let raised = Outline {
            min_height: 6.0,
            ..next_door
        };
        assert!(east(vec![raised]) > 0);
    }

    #[test]
    fn a_low_shops_board_stands_under_its_eaves_or_not_at_all() {
        // A flat 2.5 m terrace (a 2.8 m storey) onto a street: no board over its roof; a 3.6 m one's under its eaves,
        // over the door.
        let lane = road(25.0, 3.0);
        let roads = Roads::new(&lane);
        let mut boards = 0;
        for (height, storey) in [(2.5, 2.8), (3.6, 3.2)] {
            let mut b = townhouse(0.0, 1, Period::Today);
            b.height = height;
            b.storey_m = storey;
            let f = frontage(
                &b,
                &roads,
                [STREET_HALF_M, MAIN_HALF_M[1]],
                STREET_SETBACK_M,
                &|_| true,
            )
            .expect("a street's front");
            let eave = b.ground[1] + height;
            for seed in 0..64 {
                b.seed = seed;
                let mut out = Vec::new();
                signs(&b, None, f, &|_| true, &mut out);
                for c in &out {
                    assert!(
                        c.at[1] + c.size[1] <= eave + 1e-4 && c.at[1] >= b.ground[1] + 2.2 - 1e-4,
                        "{height} m, seed {seed}: {c:?}"
                    );
                }
                if height > 3.0 {
                    boards += out.len();
                }
            }
        }
        assert!(boards > 0);
    }

    #[test]
    fn a_shops_signs_keep_off_a_neighbour_standing_before_it() {
        // A walk-up's 20 m frontage at z = 20 along a primary road at z = 36; a neighbour 0.25 m before its outer shop
        // (x 15-20) leaves the frontage's middle clear, so it takes its arcade.
        let mut b = townhouse(0.0, 4, Period::Today);
        b.kind = Kind::Block;
        b.ring = vec![[0.0, 2.0], [20.0, 2.0], [20.0, 20.0], [0.0, 20.0]];
        let main = road(36.0, 4.5);
        let roads = Roads::new(&main);
        let neighbour =
            |p: [f32; 2]| (15.0..=20.0).contains(&p[0]) && (20.25..=30.0).contains(&p[1]);
        let open = |p: [f32; 2]| !neighbour(p);
        let arcade = arcade_of(&b, Region::Taiwan, &roads, &flat, &open).expect("an arcade");
        let f = Frontage {
            edge: arcade.edge,
            half: arcade.half,
        };
        // Whether a board's footprint (its middle, `size[0]` along its yaw and `size[2]` across) reaches the neighbour.
        let into = |out: &[Clutter]| {
            out.iter().any(|c| {
                let (dir, across) = ([c.yaw.cos(), c.yaw.sin()], [-c.yaw.sin(), c.yaw.cos()]);
                [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)]
                    .iter()
                    .any(|&(u, v)| {
                        let (u, v) = (0.5 * u * c.size[0], 0.5 * v * c.size[2]);
                        neighbour([
                            c.at[0] + dir[0] * u + across[0] * v,
                            c.at[2] + dir[1] * u + across[1] * v,
                        ])
                    })
            })
        };
        let mut seen = 0;
        for seed in 0..32 {
            b.seed = seed;
            let mut all = Vec::new();
            signs(&b, Some(&arcade), f, &|_| true, &mut all);
            let mut kept = Vec::new();
            signs(&b, Some(&arcade), f, &open, &mut kept);
            seen += usize::from(into(&all));
            assert!(!into(&kept), "seed {seed}");
        }
        assert!(seen > 0);
    }

    #[test]
    fn the_signs_burn_by_the_lights_rule() {
        use crate::night::LightsRule;
        let signs = |r: LightsRule| r.scales().signs;
        assert_eq!(signs(LightsRule::Lit), 1.0);
        assert!(signs(LightsRule::Dimout) > 0.0 && signs(LightsRule::Dimout) < 0.2);
        assert_eq!(signs(LightsRule::Blackout), 0.0);
        // The shader lights a sign by night under that share (`BuildingsLayer::set_signs`), its palette as `sign_style`
        // picks it.
        let wgsl = include_str!("../../shaders/buildings.wgsl");
        assert!(wgsl.contains(
            "s.emissive = s.albedo * SIGN_GLOW * buildings.sun_direction.w * rule * sign_lit(piece);"
        ));
        assert!(wgsl.contains("let rule = buildings.fill_ground.w;"));
        assert!(wgsl.contains("const SIGN_COLOURS = array<vec3<f32>, 12>("));
    }
}
