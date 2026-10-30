//! What stands about a house on the ground (native/docs/terrain/buildings.md, "Grounds"): the wall, fence or hedge
//! along its front between it and its road, those down its sides toward its neighbours, and a step under each door
//! (a stoop where the ground falls away below it). [`Shape::Near`] clutter, drawn only in chunks within
//! [`super::clutter::NEAR_M`] of the camera; each piece stands on the ground under it, a long run stepped down a slope.
//!
//! Every choice hashes the building's place, so a chunk made again draws the same grounds.

use std::collections::HashMap;

use super::clutter::{Clutter, Shape};
use super::eaves::place_hash;
use super::geometry::{self, Rect};
use super::mesh::{PART_GABLE, PART_LEAF, style_of};
use super::tags::Road;
use super::{Building, CHUNK_M, Kind, Region, bay_m};

/// A uniform grid of ids over the chunk's metres, for the roads and the buildings near a point.
struct Grid {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<u32>>,
}

impl Grid {
    fn new(cell: f32) -> Self {
        Self {
            cell,
            cells: HashMap::new(),
        }
    }

    fn key(&self, p: [f32; 2]) -> (i32, i32) {
        (
            (p[0] / self.cell).floor() as i32,
            (p[1] / self.cell).floor() as i32,
        )
    }

    fn insert(&mut self, lo: [f32; 2], hi: [f32; 2], id: u32) {
        let (a, b) = (self.key(lo), self.key(hi));
        // A road across the whole chunk spans 40 cells: no more than that.
        if (b.0 - a.0 + 1) * (b.1 - a.1 + 1) > 1600 {
            return;
        }
        for i in a.0..=b.0 {
            for j in a.1..=b.1 {
                self.cells.entry((i, j)).or_default().push(id);
            }
        }
    }

    /// The ids in the cells within `reach` of `p` (each once).
    fn near(&self, p: [f32; 2], reach: f32) -> Vec<u32> {
        let (a, b) = (
            self.key([p[0] - reach, p[1] - reach]),
            self.key([p[0] + reach, p[1] + reach]),
        );
        let mut out = Vec::new();
        for i in a.0..=b.0 {
            for j in a.1..=b.1 {
                if let Some(ids) = self.cells.get(&(i, j)) {
                    out.extend_from_slice(ids);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The chunk's roads and railways, found by where they pass.
pub struct Roads<'a> {
    roads: &'a [Road],
    grid: Grid,
}

/// The nearest road to a point: the nearest point of its middle line, how far that is (m) and its half width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearRoad {
    pub foot: [f32; 2],
    pub distance: f32,
    pub half: f32,
}

impl<'a> Roads<'a> {
    pub fn new(roads: &'a [Road]) -> Self {
        let mut grid = Grid::new(50.0);
        for (k, r) in roads.iter().enumerate() {
            let lo = [r.a[0].min(r.b[0]) - r.half, r.a[1].min(r.b[1]) - r.half];
            let hi = [r.a[0].max(r.b[0]) + r.half, r.a[1].max(r.b[1]) + r.half];
            grid.insert(lo, hi, k as u32);
        }
        Self { roads, grid }
    }

    fn foot(r: &Road, p: [f32; 2]) -> [f32; 2] {
        let d = [r.b[0] - r.a[0], r.b[1] - r.a[1]];
        let l2 = (d[0] * d[0] + d[1] * d[1]).max(1e-6);
        let t = (((p[0] - r.a[0]) * d[0] + (p[1] - r.a[1]) * d[1]) / l2).clamp(0.0, 1.0);
        [r.a[0] + d[0] * t, r.a[1] + d[1] * t]
    }

    /// The road whose edge lies nearest `p`, within `reach` of it.
    pub fn nearest(&self, p: [f32; 2], reach: f32) -> Option<NearRoad> {
        self.grid
            .near(p, reach)
            .into_iter()
            .map(|k| {
                let r = &self.roads[k as usize];
                let foot = Self::foot(r, p);
                NearRoad {
                    foot,
                    distance: geometry::distance(p, foot),
                    half: r.half,
                }
            })
            .filter(|n| n.distance - n.half <= reach)
            .min_by(|a, b| (a.distance - a.half).total_cmp(&(b.distance - b.half)))
    }

    /// Whether `p` stands off every road's paving by `margin`.
    pub fn clear(&self, p: [f32; 2], margin: f32) -> bool {
        self.grid.near(p, 30.0).into_iter().all(|k| {
            let r = &self.roads[k as usize];
            geometry::distance(p, Self::foot(r, p)) > r.half + margin
        })
    }
}

/// The chunk's buildings' outlines, found by where they stand.
#[derive(Default)]
pub struct Footings {
    rings: Vec<Vec<[f32; 2]>>,
    bounds: Vec<[f32; 4]>,
    grid: Option<Grid>,
}

impl Footings {
    /// Adds a building's outline; its number.
    pub fn add(&mut self, ring: &[[f32; 2]]) -> usize {
        let grid = self.grid.get_or_insert_with(|| Grid::new(25.0));
        let b = ring
            .iter()
            .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            });
        let k = self.rings.len();
        grid.insert([b[0], b[1]], [b[2], b[3]], k as u32);
        self.rings.push(ring.to_vec());
        self.bounds.push(b);
        k
    }

    /// Whether `p` lies inside, or within `margin` of, any outline but `except`'s.
    pub fn blocked(&self, p: [f32; 2], margin: f32, except: usize) -> bool {
        let Some(grid) = &self.grid else {
            return false;
        };
        grid.near(p, margin + 1.0).into_iter().any(|k| {
            let k = k as usize;
            let b = self.bounds[k];
            k != except
                && p[0] >= b[0] - margin
                && p[0] <= b[2] + margin
                && p[1] >= b[1] - margin
                && p[1] <= b[3] + margin
                && (geometry::contains(&self.rings[k], p)
                    || geometry::ring_distance(&self.rings[k], p) < margin)
        })
    }

    /// How far from `p` toward `dir` (a unit vector) the nearest other outline stands, looking no further than `most`.
    fn gap(&self, p: [f32; 2], dir: [f32; 2], most: f32, except: usize) -> Option<f32> {
        let mut d = 0.25;
        while d <= most {
            if self.blocked([p[0] + dir[0] * d, p[1] + dir[1] * d], 0.0, except) {
                return Some(d);
            }
            d += 0.5;
        }
        None
    }
}

/// What a boundary is built of.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Make {
    /// Masonry in the walls' palette entry, coped in the roofs' entry.
    Wall { entry: u8, cope: u8 },
    /// Boards or pickets in the walls' entry (timber, a paint).
    Fence { entry: u8 },
    /// A clipped hedge, the colour cube's green.
    Hedge { entry: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Boundary {
    make: Make,
    height: f32,
}

// Hedges' greens and walls' entries (palette.rs's).
const HEDGES: [u8; 3] = [81, 45, 80];
const RED_BRICK: u8 = 4;
const BROWN_BRICK: u8 = 5;
const WHITE_RENDER: u8 = 0;
const CREAM: u8 = 1;
const CONCRETE: u8 = 3;
const DARK_TIMBER: u8 = 12;
const BRIGHT_WHITE: u8 = 13;
const WHITEWASH: u8 = 18;
const FALU: u8 = 24;
const STONE: u8 = 30;
const LIMESTONE: u8 = 31;
// Copes (roofs' entries): concrete, slate, brown tile, white.
const COPE_CONCRETE: u8 = 14;
const COPE_SLATE: u8 = 2;
const COPE_TILE: u8 = 13;

fn weighted(options: &[(f32, Option<Boundary>)], t: f32) -> Option<Boundary> {
    let total: f32 = options.iter().map(|o| o.0).sum();
    let mut at = t * total;
    for (w, v) in options {
        if at < *w {
            return *v;
        }
        at -= w;
    }
    options.last().and_then(|o| o.1)
}

fn wall(entry: u8, cope: u8, height: f32) -> Option<Boundary> {
    Some(Boundary {
        make: Make::Wall { entry, cope },
        height,
    })
}

fn fence(entry: u8, height: f32) -> Option<Boundary> {
    Some(Boundary {
        make: Make::Fence { entry },
        height,
    })
}

fn hedge(entry: u8, height: f32) -> Option<Boundary> {
    Some(Boundary {
        make: Make::Hedge { entry },
        height,
    })
}

/// A house's front and side boundaries in `region` by two hashes (`t` the front's, `u` the sides').
fn boundaries(
    b: &Building,
    region: Region,
    t: f32,
    u: f32,
    v: f32,
) -> (Option<Boundary>, Option<Boundary>) {
    use Region::*;
    let green = HEDGES[(v * 2.99) as usize];
    // The house's own masonry when it is brick or render, so its garden wall matches it.
    let brick = if matches!(b.wall, RED_BRICK | BROWN_BRICK | 6) {
        b.wall
    } else {
        RED_BRICK
    };
    let render = if b.wall < 32 && matches!(b.wall, 0 | 1 | 7..=10 | 13 | 15..=20) {
        b.wall
    } else {
        WHITE_RENDER
    };
    let past = b.period.is_past();
    let front = match region {
        British => weighted(
            &[
                (0.45, wall(brick, COPE_SLATE, 0.9)),
                (0.25, hedge(green, 1.1)),
                (0.15, None),
                (0.15, wall(STONE, COPE_SLATE, 0.8)),
            ],
            t,
        ),
        Celtic => weighted(
            &[
                (0.6, wall(STONE, COPE_SLATE, 1.0)),
                (0.2, hedge(green, 1.1)),
                (0.2, None),
            ],
            t,
        ),
        NorthSea => weighted(
            &[
                (0.5, hedge(green, 0.9)),
                (0.2, wall(brick, COPE_TILE, 0.6)),
                (0.3, None),
            ],
            t,
        ),
        Temperate => weighted(
            &[
                (0.4, wall(render, COPE_CONCRETE, 1.2)),
                (0.3, hedge(green, 1.2)),
                (0.1, fence(DARK_TIMBER, 1.2)),
                (0.2, None),
            ],
            t,
        ),
        Nordic => weighted(
            &[
                (0.35, fence(BRIGHT_WHITE, 1.0)),
                (0.2, fence(FALU, 1.0)),
                (0.45, None),
            ],
            t,
        ),
        Iceland => weighted(
            &[
                (0.3, wall(CONCRETE, COPE_CONCRETE, 0.6)),
                (0.3, fence(BRIGHT_WHITE, 0.9)),
                (0.4, None),
            ],
            t,
        ),
        American => weighted(
            &[
                (0.6, None),
                (0.25, fence(BRIGHT_WHITE, 1.0)),
                (0.15, hedge(green, 0.9)),
            ],
            t,
        ),
        West => weighted(
            &[
                (0.55, None),
                (0.15, wall(render, COPE_CONCRETE, 0.7)),
                (0.2, hedge(green, 0.9)),
                (0.1, fence(BRIGHT_WHITE, 0.9)),
            ],
            t,
        ),
        Florida | Hawaii => weighted(
            &[
                (0.4, hedge(green, 1.0)),
                (0.5, None),
                (0.1, wall(render, COPE_CONCRETE, 0.6)),
            ],
            t,
        ),
        Mediterranean | Malta | Aegean => weighted(
            &[
                (0.5, wall(render, COPE_TILE, 1.8)),
                (0.25, wall(render, COPE_CONCRETE, 1.0)),
                (0.25, None),
            ],
            t,
        ),
        NorthAfrica | Arid | Nile => weighted(
            &[
                (
                    0.7,
                    wall(if t < 0.3 { WHITEWASH } else { render }, COPE_CONCRETE, 2.2),
                ),
                (0.3, None),
            ],
            t,
        ),
        EastAsia if past => weighted(
            &[
                (0.35, hedge(green, 1.6)),
                (0.25, fence(DARK_TIMBER, 1.8)),
                (0.15, wall(STONE, COPE_TILE, 1.0)),
                (0.25, None),
            ],
            t,
        ),
        EastAsia => weighted(
            &[
                (0.3, hedge(green, 1.5)),
                (0.3, wall(CONCRETE, COPE_CONCRETE, 1.4)),
                (0.15, fence(DARK_TIMBER, 1.6)),
                (0.25, None),
            ],
            t,
        ),
        // Taiwan's townhouses at the lane: an iron gate in a low concrete wall before a house's forecourt now and then,
        // no hedges.
        Taiwan => weighted(
            &[
                (0.75, None),
                (0.17, wall(CONCRETE, COPE_CONCRETE, 1.2)),
                (0.08, fence(BRIGHT_WHITE, 1.4)),
            ],
            t,
        ),
        Tropical | Micronesia | Atoll => weighted(
            &[
                (0.7, None),
                (0.2, hedge(green, 1.0)),
                (
                    0.1,
                    wall(if t < 0.5 { LIMESTONE } else { CREAM }, COPE_CONCRETE, 0.6),
                ),
            ],
            t,
        ),
    };
    let sides = match region {
        British => weighted(
            &[
                (0.5, fence(DARK_TIMBER, 1.8)),
                (0.3, hedge(green, 1.7)),
                (0.1, wall(brick, COPE_SLATE, 1.8)),
                (0.1, None),
            ],
            u,
        ),
        Celtic => weighted(
            &[
                (0.5, wall(STONE, COPE_SLATE, 1.2)),
                (0.3, hedge(green, 1.4)),
                (0.2, None),
            ],
            u,
        ),
        NorthSea => weighted(
            &[
                (0.5, hedge(green, 1.6)),
                (0.3, fence(DARK_TIMBER, 1.8)),
                (0.2, None),
            ],
            u,
        ),
        Temperate => weighted(
            &[
                (0.4, hedge(green, 1.8)),
                (0.3, wall(render, COPE_CONCRETE, 1.8)),
                (0.2, fence(DARK_TIMBER, 1.6)),
                (0.1, None),
            ],
            u,
        ),
        Nordic | Iceland => weighted(&[(0.4, fence(BRIGHT_WHITE, 1.0)), (0.6, None)], u),
        American => weighted(
            &[
                (0.4, None),
                (0.3, fence(DARK_TIMBER, 1.8)),
                (0.2, hedge(green, 1.6)),
                (0.1, fence(BRIGHT_WHITE, 1.0)),
            ],
            u,
        ),
        West => weighted(
            &[
                (0.5, fence(DARK_TIMBER, 1.8)),
                (0.2, wall(render, COPE_CONCRETE, 1.8)),
                (0.15, hedge(green, 1.6)),
                (0.15, None),
            ],
            u,
        ),
        Florida | Hawaii => weighted(&[(0.4, hedge(green, 1.8)), (0.6, None)], u),
        Mediterranean | Malta | Aegean => {
            weighted(&[(0.7, wall(render, COPE_TILE, 2.0)), (0.3, None)], u)
        }
        NorthAfrica | Arid | Nile => {
            weighted(&[(0.8, wall(render, COPE_CONCRETE, 2.2)), (0.2, None)], u)
        }
        EastAsia => weighted(
            &[
                (0.4, hedge(green, 1.6)),
                (0.3, fence(DARK_TIMBER, 1.8)),
                (0.3, None),
            ],
            u,
        ),
        Tropical | Micronesia | Atoll => weighted(&[(0.2, hedge(green, 1.2)), (0.8, None)], u),
        Taiwan => weighted(
            &[(0.12, wall(CONCRETE, COPE_CONCRETE, 1.6)), (0.88, None)],
            u,
        ),
    };
    (front, sides)
}

/// The pavement between a front boundary and its road's paving (m).
fn pavement(region: Region) -> f32 {
    use Region::*;
    match region {
        EastAsia | Taiwan => 0.3,
        American | West | Florida | Hawaii => 2.4,
        Tropical | Micronesia | Atoll | Arid | NorthAfrica | Nile => 0.8,
        _ => 1.6,
    }
}

/// `buildings.wgsl`'s `hash13` (Dave Hoskins' hash without sine) in the same f32 steps, so a door's bay is the
/// shader's.
pub fn hash13(p: [f32; 3]) -> f32 {
    let fract = |x: f32| x - x.floor();
    let mut q = [
        fract(p[0] * 0.1031),
        fract(p[1] * 0.1031),
        fract(p[2] * 0.1031),
    ];
    let d = q[0] * (q[2] + 31.32) + q[1] * (q[1] + 31.32) + q[2] * (q[0] + 31.32);
    q = [q[0] + d, q[1] + d, q[2] + d];
    fract((q[0] + q[1]) * q[2])
}

/// `buildings.wgsl`'s `hash12`.
pub fn hash12(p: [f32; 2]) -> f32 {
    let fract = |x: f32| x - x.floor();
    let mut q = [fract(p[0] * 0.1031), fract(p[1] * 0.1030)];
    let d = q[0] * (q[1] + 33.33) + q[1] * (q[0] + 33.33);
    q = [q[0] + d, q[1] + d];
    fract((q[0] + q[1]) * q[0])
}

/// Whether the shader's facade puts a door in bay `bay` of `b`'s ground floor (`window_style`'s and
/// `window_style_past`'s rule: a house's or a terrace's one bay in six or so, but a past period's Japanese house's,
/// which slides; a block's one in eight but where a shop front fills its ground floor; `window_style_taiwan`'s in
/// Taiwan, a block's one in five but where a shop front fills it, on three in four).
pub fn door_at(b: &Building, region: Region, bay: f32) -> bool {
    let seed = f32::from(b.seed);
    let cell = hash13([bay.floor(), 41.0, seed]);
    match b.kind {
        Kind::House | Kind::Terrace => {
            !(b.period.is_past() && region == Region::EastAsia) && cell < 0.16
        }
        Kind::Block | Kind::Tower => {
            // Taiwan's: a shop front on three in four, a door in one bay in five of the rest (`window_style_taiwan`).
            let (shops, doors) = if region == Region::Taiwan {
                (0.75, 0.2)
            } else {
                (0.5, 0.12)
            };
            hash12([seed, 3.0]) >= shops && cell < doors
        }
        _ => false,
    }
}

/// A box piece from `a` to `b` (chunk-local), `height` over the highest ground under it, `thick` deep, its foot a little
/// under the lowest.
fn piece(
    a: [f32; 2],
    c: [f32; 2],
    height: f32,
    thick: f32,
    ground: &dyn Fn([f32; 2]) -> f32,
    style: [u8; 4],
) -> Clutter {
    let m = [(a[0] + c[0]) * 0.5, (a[1] + c[1]) * 0.5];
    let gs = [ground(a), ground(m), ground(c)];
    let lo = gs.iter().copied().fold(f32::MAX, f32::min);
    let hi = gs.iter().copied().fold(f32::MIN, f32::max);
    let foot = lo - 0.15;
    Clutter {
        at: [m[0], foot, m[1]],
        yaw: (c[1] - a[1]).atan2(c[0] - a[0]),
        size: [geometry::distance(a, c), hi + height - foot, thick],
        shape: Shape::Near as u32,
        style,
    }
}

/// Lays a boundary from `a` to `c` in pieces of no more than 6 m, leaving out what would stand on a road or in another
/// building (`me` the house's own outline) or past the chunk's edge (the next chunk's roads and buildings unknown here),
/// and a gap `gate` m wide about `gate_at` (metres from `a`), if any.
#[allow(clippy::too_many_arguments)]
fn run(
    a: [f32; 2],
    c: [f32; 2],
    boundary: Boundary,
    gate: Option<f32>,
    me: usize,
    roads: &Roads,
    footings: &Footings,
    ground: &dyn Fn([f32; 2]) -> f32,
    seed: u8,
    out: &mut Vec<Clutter>,
) {
    let len = geometry::distance(a, c);
    if len < 0.8 {
        return;
    }
    let dir = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
    let at = |s: f32| [a[0] + dir[0] * s, a[1] + dir[1] * s];
    let (style, thick) = match boundary.make {
        Make::Wall { entry, cope } => (
            style_of(entry, cope, seed, Kind::Industrial, PART_GABLE, false),
            0.28,
        ),
        Make::Fence { entry } => (
            style_of(entry, entry, seed, Kind::Shed, PART_GABLE, false),
            0.06,
        ),
        Make::Hedge { entry } => (
            style_of(entry, entry, seed, Kind::Shed, PART_LEAF, false),
            0.7,
        ),
    };
    // Samples every half metre: a run of clear ones is laid, a blocked one ends it.
    let step = 0.5;
    let n = (len / step).ceil() as usize;
    let clear = |s: f32| {
        let p = at(s);
        let gated = gate.is_some_and(|g| (s - g).abs() < 0.55);
        let inside = p.iter().all(|v| (0.0..=CHUNK_M as f32).contains(v));
        !gated
            && inside
            && roads.clear(p, 0.3)
            && !footings.blocked(p, 0.3, me)
            && !in_own(footings, me, p)
    };
    let mut lay = |s0: f32, s1: f32| {
        if s1 - s0 < 0.8 {
            return;
        }
        let pieces = ((s1 - s0) / 6.0).ceil().max(1.0) as usize;
        for k in 0..pieces {
            let u0 = s0 + (s1 - s0) * k as f32 / pieces as f32;
            let u1 = s0 + (s1 - s0) * (k + 1) as f32 / pieces as f32;
            out.push(piece(at(u0), at(u1), boundary.height, thick, ground, style));
        }
    };
    let mut start: Option<f32> = None;
    let mut last = 0.0;
    for k in 0..=n {
        let s = (k as f32 * step).min(len);
        if clear(s) {
            start.get_or_insert(s);
            last = s;
            if k < n {
                continue;
            }
        }
        if let Some(s0) = start.take() {
            lay(s0, last);
        }
    }
}

/// Whether `p` lies inside the house's own outline (a boundary never runs through its own walls).
fn in_own(footings: &Footings, me: usize, p: [f32; 2]) -> bool {
    footings
        .rings
        .get(me)
        .is_some_and(|r| geometry::contains(r, p))
}

/// The grounds of house (or terrace) `b`, outline `me` of `footings`, in `region`: its front boundary toward the road
/// nearest it (with a gate), its sides' toward its neighbours (a detached house's), a step or stoop under each door.
pub fn grounds(
    b: &Building,
    me: usize,
    region: Region,
    roads: &Roads,
    footings: &Footings,
    ground: &dyn Fn([f32; 2]) -> f32,
    out: &mut Vec<Clutter>,
) {
    if !matches!(
        b.kind,
        Kind::House | Kind::Terrace | Kind::Block | Kind::Tower
    ) {
        return;
    }
    steps(b, me, region, roads, footings, ground, out);
    if !matches!(b.kind, Kind::House | Kind::Terrace) {
        return;
    }
    let Some(rect) = geometry::min_area_rect(&b.ring) else {
        return;
    };
    let c = rect.centre;
    let Some(road) = roads.nearest(c, 45.0) else {
        return;
    };
    let to_road = [road.foot[0] - c[0], road.foot[1] - c[1]];
    let tl = to_road[0].hypot(to_road[1]);
    if tl < 1e-3 {
        return;
    }
    let to_road = [to_road[0] / tl, to_road[1] / tl];
    // The rectangle's side facing the road: its outward normal, the way along it, its half length and the house's half
    // depth behind it.
    let Rect { axis, half, .. } = rect;
    let normal = [-axis[1], axis[0]];
    let sides = [
        (axis, normal, half[1], half[0]),
        ([-axis[0], -axis[1]], normal, half[1], half[0]),
        (normal, axis, half[0], half[1]),
        ([-normal[0], -normal[1]], axis, half[0], half[1]),
    ];
    let (f, l, frontage, depth) = sides
        .iter()
        .copied()
        .max_by(|a, b| {
            (a.0[0] * to_road[0] + a.0[1] * to_road[1])
                .total_cmp(&(b.0[0] * to_road[0] + b.0[1] * to_road[1]))
        })
        .unwrap();
    let ahead = (road.foot[0] - c[0]) * f[0] + (road.foot[1] - c[1]) * f[1];
    let gap = ahead - depth - road.half;
    let set = gap - pavement(region);
    let h = |k: i64| place_hash(b, k);
    let (front, side) = boundaries(b, region, h(31), h(32), h(33));
    let detached = b.kind == Kind::House;
    // How far to the neighbours either side (half way, a little off the house).
    let offset = |sign: f32| {
        if !detached {
            return 0.0;
        }
        let from = [c[0] + l[0] * frontage * sign, c[1] + l[1] * frontage * sign];
        match footings.gap(from, [l[0] * sign, l[1] * sign], 8.0, me) {
            Some(g) if g < 1.2 => 0.0,
            Some(g) => (g * 0.5).clamp(0.6, 3.0),
            None => 2.5,
        }
    };
    let (left, right) = (offset(-1.0), offset(1.0));
    let point = |along: f32, out: f32| {
        [
            c[0] + l[0] * along + f[0] * out,
            c[1] + l[1] * along + f[1] * out,
        ]
    };
    // The front: along the road's side of the plot, a gate before the door (or the middle).
    if let Some(boundary) = front
        && (1.0..=30.0).contains(&set)
    {
        let a = point(-frontage - left, depth + set);
        let e = point(frontage + right, depth + set);
        let gate = frontage + left + door_along(b, region, &rect, f, l).unwrap_or(0.0);
        run(
            a,
            e,
            boundary,
            Some(gate),
            me,
            roads,
            footings,
            ground,
            b.seed,
            out,
        );
    }
    // The sides: from the front's line (or the house's front) back past the house.
    if let Some(boundary) = side
        && detached
    {
        let reach = if (1.0..=30.0).contains(&set) {
            depth + set
        } else {
            depth
        };
        let back = -depth - 2.0 - 2.0 * h(34);
        for (sign, off) in [(-1.0, left), (1.0, right)] {
            if off < 0.6 {
                continue;
            }
            let along = (frontage + off) * sign;
            run(
                point(along, reach),
                point(along, back),
                boundary,
                None,
                me,
                roads,
                footings,
                ground,
                b.seed,
                out,
            );
        }
    }
}

/// Where along its front a house's door stands (metres from its middle along `l`), if one does on the wall facing `f`.
fn door_along(b: &Building, region: Region, rect: &Rect, f: [f32; 2], l: [f32; 2]) -> Option<f32> {
    let n = b.ring.len();
    for i in 0..n {
        let (a, c) = (b.ring[i], b.ring[(i + 1) % n]);
        let len = geometry::distance(a, c);
        if len < 0.5 {
            continue;
        }
        let outward = [(c[1] - a[1]) / len, -(c[0] - a[0]) / len];
        if outward[0] * f[0] + outward[1] * f[1] < 0.9 {
            continue;
        }
        let bays = (len / b.bay.max(0.5)).round().max(1.0);
        for k in 0..bays as u32 {
            if door_at(b, region, k as f32) {
                let t = (k as f32 + 0.5) / bays;
                let p = [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t];
                return Some((p[0] - rect.centre[0]) * l[0] + (p[1] - rect.centre[1]) * l[1]);
            }
        }
    }
    None
}

/// A step under each of `b`'s doors (the shader's, [`door_at`]): from the ground before it to the ground floor's sill
/// (the building's mean ground), a stoop of several risers where the ground falls away; none before a door a
/// neighbour's wall stands against (a terrace's party wall), and none that would reach into another building or a road.
fn steps(
    b: &Building,
    me: usize,
    region: Region,
    roads: &Roads,
    footings: &Footings,
    ground: &dyn Fn([f32; 2]) -> f32,
    out: &mut Vec<Clutter>,
) {
    let n = b.ring.len();
    let sill = b.ground[1];
    let width = 0.44 * bay_m(b.kind) + 0.2;
    let style = style_of(
        STONE,
        COPE_CONCRETE,
        b.seed,
        Kind::Industrial,
        PART_GABLE,
        false,
    );
    for i in 0..n {
        let (a, c) = (b.ring[i], b.ring[(i + 1) % n]);
        let len = geometry::distance(a, c);
        if len < 1.5 {
            continue;
        }
        let bays = (len / b.bay.max(0.5)).round().max(1.0);
        if len / bays < width {
            continue;
        }
        let dir = [(c[0] - a[0]) / len, (c[1] - a[1]) / len];
        let outward = [dir[1], -dir[0]];
        for k in 0..bays as u32 {
            if !door_at(b, region, k as f32) {
                continue;
            }
            let t = (k as f32 + 0.5) / bays;
            let door = [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t];
            let before = [door[0] + outward[0] * 0.6, door[1] + outward[1] * 0.6];
            if footings.blocked(before, 0.2, me) {
                continue;
            }
            let fall = sill - ground(before);
            // Buried by the slope: no step.
            if fall < -0.3 {
                continue;
            }
            let risers = (fall.max(0.0) / 0.17).round().clamp(1.0, 8.0);
            let deep = 0.35 + 0.28 * (risers - 1.0);
            let at = [
                door[0] + outward[0] * deep * 0.5,
                door[1] + outward[1] * deep * 0.5,
            ];
            // Its front corners and middle, a little in from its edges.
            let reaches = [-0.45, 0.0, 0.45].iter().any(|side| {
                let p = [
                    door[0] + outward[0] * (deep - 0.05) + dir[0] * width * side,
                    door[1] + outward[1] * (deep - 0.05) + dir[1] * width * side,
                ];
                footings.blocked(p, 0.05, me) || !roads.clear(p, 0.0)
            });
            if reaches {
                continue;
            }
            let foot = ground(at).min(sill) - 0.1;
            out.push(Clutter {
                at: [at[0], foot, at[1]],
                yaw: dir[1].atan2(dir[0]),
                size: [width, (sill + 0.16 - foot).max(0.2), deep],
                shape: Shape::Near as u32,
                style,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::{Period, Roof};

    fn house(x: f32, z: f32, kind: Kind, seed: u8) -> Building {
        Building {
            ring: vec![[x, z], [x + 10.0, z], [x + 10.0, z + 8.0], [x, z + 8.0]],
            kind,
            roof: Roof::Gabled { rise: 3.0 },
            ground: [2.0, 2.0, 2.0],
            height: 5.6,
            base: 0.0,
            storeys: 2,
            storey_m: 2.8,
            wall: 4,
            roof_colour: 2,
            seed,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period: Period::Today,
        }
    }

    #[test]
    fn the_hashes_are_the_shaders() {
        // The shader's values for these (WGSL's hash13 and hash12 run on the GPU: fract(p * 0.1031) and so on).
        let a = hash13([2.0, 41.0, 117.0]);
        assert!((0.0..1.0).contains(&a));
        // By hand for [0, 41, 0]: q = (0, 0.2271, 0); the dot 0.2271 * 31.5471 = 7.16435; q = (7.16435, 7.39145,
        // 7.16435); (q.x + q.y) * q.z = 104.2832, its fraction 0.283.
        let b = hash13([0.0, 41.0, 0.0]);
        assert!((b - 0.283).abs() < 0.005, "{b}");
        // Over many bays and seeds, about one in six a door.
        let doors = (0..256)
            .flat_map(|seed| (0..6).map(move |k| hash13([k as f32, 41.0, seed as f32])))
            .filter(|c| *c < 0.16)
            .count();
        assert!((180..=320).contains(&doors), "{doors}");
    }

    #[test]
    fn taiwans_blocks_take_their_doors_where_its_shader_draws_them() {
        // Seed 2 a shop front in Taiwan (its hash under three in four) but not elsewhere (over one in two); bay 17 a
        // door's hash under either rule.
        let block = house(0.0, 0.0, Kind::Block, 2);
        assert!((0.5..0.75).contains(&hash12([2.0, 3.0])));
        assert!(hash13([17.0, 41.0, 2.0]) < 0.12);
        assert!(door_at(&block, Region::EastAsia, 17.0));
        assert!(!door_at(&block, Region::Taiwan, 17.0));
    }

    #[test]
    fn a_house_gets_a_front_wall_with_a_gate_and_steps_under_its_doors() {
        // A street along z = 30 (its middle), 8 m wide; a British house set 10 m back from it.
        let roads = vec![Road {
            a: [-100.0, 30.0],
            b: [200.0, 30.0],
            half: 4.0,
        }];
        let roads = Roads::new(&roads);
        let mut footings = Footings::default();
        let mut found = 0;
        let ground = |p: [f32; 2]| 2.0 + 0.0 * p[0];
        for seed in 0..60u8 {
            let b = house(seed as f32 * 20.0 - 60.0, 8.0, Kind::House, seed);
            let me = footings.add(&b.ring);
            let mut out = Vec::new();
            grounds(
                &b,
                me,
                Region::British,
                &roads,
                &footings,
                &ground,
                &mut out,
            );
            for p in &out {
                // Nothing on the road's paving, nothing inside the house.
                let at = [p.at[0], p.at[2]];
                assert!(roads.clear(at, 0.0), "{p:?}");
                assert!(!geometry::contains(&b.ring, at), "{p:?}");
                assert!(p.size.iter().all(|s| s.is_finite() && *s > 0.0));
            }
            found += out.len();
        }
        assert!(found > 60, "{found}");
        // A house with no road near it: its steps alone.
        let lonely = house(5000.0, 5000.0, Kind::House, 3);
        let mut out = Vec::new();
        grounds(
            &lonely,
            usize::MAX,
            Region::British,
            &roads,
            &Footings::default(),
            &ground,
            &mut out,
        );
        let doors: usize = (0..4)
            .map(|i| {
                let len = if i % 2 == 0 { 10.0 } else { 8.0 };
                let bays = (len / 3.0f32).round();
                (0..bays as u32)
                    .filter(|k| door_at(&lonely, Region::British, *k as f32))
                    .count()
            })
            .sum();
        assert_eq!(out.len(), doors);
    }

    #[test]
    fn a_stoop_climbs_to_a_door_above_falling_ground() {
        let mut b = house(0.0, 0.0, Kind::House, 0);
        b.ground = [0.0, 1.2, 2.4];
        // A seed with a door somewhere.
        let seed = (0..=255u8)
            .find(|s| {
                b.seed = *s;
                (0..3).any(|k| door_at(&b, Region::British, k as f32))
            })
            .unwrap();
        b.seed = seed;
        let ground = |_: [f32; 2]| 0.0;
        let roads = Roads::new(&[]);
        let mut out = Vec::new();
        steps(
            &b,
            usize::MAX,
            Region::British,
            &roads,
            &Footings::default(),
            &ground,
            &mut out,
        );
        assert!(!out.is_empty());
        for s in &out {
            // From under the ground to just over the sill, several risers deep.
            assert!(s.at[1] < 0.0 && s.at[1] + s.size[1] > 1.2);
            assert!(s.size[2] > 1.0);
        }
        // Neighbours 1.2 m off every wall: the door's step would fit, its stoop would not.
        let mut footings = Footings::default();
        for [x0, z0, x1, z1] in [
            [-6.0, -6.0, 16.0, -1.2],
            [-6.0, 9.2, 16.0, 14.0],
            [-6.0, -1.0, -1.2, 9.0],
            [11.2, -1.0, 16.0, 9.0],
        ] {
            footings.add(&[[x0, z0], [x1, z0], [x1, z1], [x0, z1]]);
        }
        let mut out = Vec::new();
        steps(
            &b,
            usize::MAX,
            Region::British,
            &roads,
            &footings,
            &ground,
            &mut out,
        );
        assert!(out.is_empty());
        b.ground = [0.0, 0.0, 0.0];
        steps(
            &b,
            usize::MAX,
            Region::British,
            &roads,
            &footings,
            &ground,
            &mut out,
        );
        assert!(!out.is_empty());
    }
}
