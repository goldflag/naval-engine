//! Container stacks on a port's terminals (native/docs/terrain/infra.md). OpenMapTiles maps no
//! terminal and no container: a terminal's yard is industrial land of a working port ([`MapTags::port`], or a
//! harbour's point in the chunk's tiles) that stands back from its quay's apron but within reach of deep water
//! ([`YardTest::holds`]). Its stacks stand in blocks on a grid laid along the land's quay ([`quay_axis`]), anchored in
//! the theatre's metres so blocks run on across chunks: rows of 40 ft slots (a quarter of them two 20 ft boxes) one to
//! five high, lanes between the blocks, the colours of the world's lines.
//!
//! Each slot is one column ([`Material::Stacks`]: the shader colours each box in it by a hash of the row's seed, the
//! slot and the tier, so a column's faces carry every box up it), its sides drawn only where a neighbour does not hide
//! them; beyond the near range a block's rows stand in as one box each, coloured by the same hash.
//!
//! The yard's test is the buildings' too: their generic infill keeps off it (`naval-client`'s `theatre::buildings`).

use glam::{Vec2, Vec3};

use super::mesh::{Block, Group, Material, MeshBuilder, Paint, hash01};
use crate::buildings::tags::{MapTags, PoiKind, Usage, Zone};
use crate::buildings::{CHUNK_M, geometry};

/// A 40 ft box's length and width, the slot's pitch along and across a block, and a tier's height (m).
pub const BOX_LONG_M: f32 = 12.19;
pub const BOX_WIDE_M: f32 = 2.44;
pub const SLOT_ALONG_M: f32 = 12.75;
pub const SLOT_ACROSS_M: f32 = 2.75;
pub const TIER_M: f32 = 2.6;
/// The quay's apron, kept clear (m from the water): the gantries' rails and the lanes under them.
pub const APRON_M: f32 = 55.0;
/// How far back from deep water a yard reaches (m).
pub const YARD_REACH_M: f32 = 600.0;
/// The depth that tells a container berth (m below the sea): a marina's, a fishing harbour's or a beach's water is
/// shallower.
pub const DEEP_M: f32 = 8.0;
/// The most of the land within [`BUILT_REACH_M`] that OSM's building outlines may cover about a yard: a warehouse
/// district's or a town's industrial blocks are built over, a terminal is open.
pub const BUILT_SHARE: f32 = 0.18;
pub const BUILT_REACH_M: f32 = 120.0;
/// The building outlines' grid (m a cell), from 200 m before the chunk's corner.
const GRID_M: f32 = 40.0;
const GRID_FROM: f32 = -200.0;
const GRID_N: usize = 35;
/// Lanes between blocks: along their sides (a truck lane and a gantry's wheels) and across their ends (m).
const SIDE_LANE_M: f32 = 9.0;
const END_LANE_M: f32 = 22.0;

/// Whether a chunk's map says a working port is about: a ferry terminal in a tile about it, or a harbour's point.
pub fn port_about(tags: &MapTags) -> bool {
    tags.port || tags.pois.iter().any(|p| p.kind == PoiKind::Harbour)
}

/// The test of a yard's ground over one chunk's map (chunk-local metres).
pub struct YardTest<'a> {
    pub tags: &'a MapTags,
    pub ground: &'a dyn Fn([f32; 2]) -> f32,
    pub port: bool,
    /// OSM's building outlines' area in each cell of a grid over the chunk and 200 m about it (m²).
    built: Vec<f32>,
}

impl<'a> YardTest<'a> {
    pub fn new(tags: &'a MapTags, ground: &'a dyn Fn([f32; 2]) -> f32) -> Self {
        let mut built = vec![0.0; GRID_N * GRID_N];
        for o in &tags.outlines {
            let c = geometry::centroid(&o.ring);
            let (i, j) = (
                ((c[0] - GRID_FROM) / GRID_M).floor(),
                ((c[1] - GRID_FROM) / GRID_M).floor(),
            );
            if (0.0..GRID_N as f32).contains(&i) && (0.0..GRID_N as f32).contains(&j) {
                built[j as usize * GRID_N + i as usize] += geometry::area(&o.ring);
            }
        }
        Self {
            tags,
            ground,
            port: port_about(tags),
            built,
        }
    }

    /// The share of the land within [`BUILT_REACH_M`] of `p` that OSM's outlines cover (by their cells' middles).
    fn built_share(&self, p: [f32; 2]) -> f32 {
        let reach = BUILT_REACH_M;
        let cells = (reach / GRID_M).ceil() as i32;
        let (ci, cj) = (
            ((p[0] - GRID_FROM) / GRID_M).floor() as i32,
            ((p[1] - GRID_FROM) / GRID_M).floor() as i32,
        );
        let mut sum = 0.0;
        for j in cj - cells..=cj + cells {
            for i in ci - cells..=ci + cells {
                if !(0..GRID_N as i32).contains(&i) || !(0..GRID_N as i32).contains(&j) {
                    continue;
                }
                let mid = [
                    GRID_FROM + (i as f32 + 0.5) * GRID_M,
                    GRID_FROM + (j as f32 + 0.5) * GRID_M,
                ];
                if (mid[0] - p[0]).hypot(mid[1] - p[1]) <= reach {
                    sum += self.built[j as usize * GRID_N + i as usize];
                }
            }
        }
        sum / (std::f32::consts::PI * reach * reach)
    }

    /// Whether `p` is container yard: industrial land of a port, dry, at least [`APRON_M`] from the water the map knows,
    /// within [`YARD_REACH_M`] of water deeper than [`DEEP_M`] (by the ground's heights), amid industrial land or water
    /// (13 of 16 points 150 and 300 m about it: not a town's industrial block) and open (OSM's outlines covering less than
    /// [`BUILT_SHARE`] about it: not a warehouse district).
    pub fn holds(&self, p: [f32; 2]) -> bool {
        if !self.port || self.tags.zones.is_empty() {
            return false;
        }
        if self.tags.usage_at(p) != Usage::Industrial {
            return false;
        }
        let ground = self.ground;
        if ground(p) < 0.5 || self.tags.is_water(p, ground) {
            return false;
        }
        let dirs: [[f32; 2]; 8] = std::array::from_fn(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            [a.cos(), a.sin()]
        });
        let at = |d: [f32; 2], r: f32| [p[0] + d[0] * r, p[1] + d[1] * r];
        for d in dirs {
            for r in [APRON_M * 0.5, APRON_M] {
                if self.tags.is_water(at(d, r), ground) {
                    return false;
                }
            }
        }
        let steps = (YARD_REACH_M / 75.0) as i32;
        let deep = dirs
            .iter()
            .any(|d| (1..=steps).any(|k| ground(at(*d, k as f32 * 75.0)) < -DEEP_M));
        if !deep {
            return false;
        }
        let amid = dirs
            .iter()
            .flat_map(|d| [150.0, 300.0].map(|r| at(*d, r)))
            .filter(|q| ground(*q) < 0.0 || self.tags.usage_at(*q) == Usage::Industrial)
            .count();
        amid >= 13 && self.built_share(p) < BUILT_SHARE
    }
}

/// The direction a zone's stacks run along: its longest stretch of edge with the water beyond (its quay), or its
/// smallest rectangle's long side where the map shows it no water; a unit vector in (x, z), its sign fixed (x ≥ 0) so
/// a zone's two tiles' pieces agree.
pub fn quay_axis(zone: &Zone, tags: &MapTags, ground: &dyn Fn([f32; 2]) -> f32) -> Option<Vec2> {
    let ring = &zone.exterior;
    let n = ring.len();
    if n < 3 {
        return None;
    }
    let flip = if geometry::signed_area2(ring) < 0.0 {
        -1.0
    } else {
        1.0
    };
    let mut best: Option<(f32, Vec2)> = None;
    for i in 0..n {
        let (a, b) = (Vec2::from(ring[i]), Vec2::from(ring[(i + 1) % n]));
        let len = a.distance(b);
        if len < 40.0 {
            continue;
        }
        let d = (b - a) / len;
        let out = Vec2::new(d.y, -d.x) * flip;
        let mid = (a + b) * 0.5 + out * 30.0;
        if tags.is_water(mid.to_array(), ground) && best.is_none_or(|(l, _)| len > l) {
            best = Some((len, d));
        }
    }
    let axis = match best {
        Some((_, d)) => d,
        None => Vec2::from(geometry::min_area_rect(ring)?.axis),
    };
    Some(if axis.x < 0.0 || (axis.x == 0.0 && axis.y < 0.0) {
        -axis
    } else {
        axis
    })
}

/// A yard's layout: its blocks' slots along their long axis (`along`, unit) and across, the block's bays and rows.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Layout {
    along: Vec2,
    across: Vec2,
    bays: i32,
    rows: i32,
    seed: i64,
}

impl Layout {
    fn of(axis: Vec2) -> Self {
        // Keyed by the axis's heading (tenths of a degree), so two pieces of one terminal lay out alike.
        let key = (axis.y.atan2(axis.x).to_degrees() * 10.0).round() as i64;
        let turned = hash01(key, 0, 401) < 0.4;
        let along = if turned {
            Vec2::new(-axis.y, axis.x)
        } else {
            axis
        };
        Self {
            along,
            across: Vec2::new(-along.y, along.x),
            bays: 10 + (hash01(key, 1, 402) * 9.0) as i32,
            rows: 5 + (hash01(key, 2, 403) * 3.0) as i32,
            seed: key,
        }
    }

    fn cell_along(&self) -> f32 {
        self.bays as f32 * SLOT_ALONG_M + END_LANE_M
    }

    fn cell_across(&self) -> f32 {
        self.rows as f32 * SLOT_ACROSS_M + SIDE_LANE_M
    }

    /// The theatre point of slot (bay `k`, row `r`)'s middle in cell (`i`, `j`).
    fn slot(&self, i: i32, j: i32, k: i32, r: i32) -> Vec2 {
        let u = i as f32 * self.cell_along() + END_LANE_M * 0.5 + (k as f32 + 0.5) * SLOT_ALONG_M;
        let v =
            j as f32 * self.cell_across() + SIDE_LANE_M * 0.5 + (r as f32 + 0.5) * SLOT_ACROSS_M;
        self.along * u + self.across * v
    }
}

/// What keeps stacks off a slot (its middle `p`, `layout`'s axes): the roads and railways (4 m clear of the box), OSM's
/// building outlines (3 m), the piers (2 m), tested at the box's corners and middle.
fn blocked(tags: &MapTags, layout: &Layout, p: Vec2) -> bool {
    let a = layout.along * (BOX_LONG_M * 0.5);
    let c = layout.across * (BOX_WIDE_M * 0.5);
    let points = [p, p - a - c, p + a - c, p + a + c, p - a + c];
    let reach = 4.0;
    let road = |q: Vec2| {
        tags.roads.iter().any(|r| {
            q.x >= r.a[0].min(r.b[0]) - r.half - reach
                && q.x <= r.a[0].max(r.b[0]) + r.half + reach
                && q.y >= r.a[1].min(r.b[1]) - r.half - reach
                && q.y <= r.a[1].max(r.b[1]) + r.half + reach
                && geometry::segment_distance(q.to_array(), r.a, r.b) < r.half + reach
        })
    };
    let near = |ring: &[[f32; 2]], m: f32, q: Vec2| {
        ring.len() >= 3
            && (geometry::contains(ring, q.to_array())
                || geometry::ring_distance(ring, q.to_array()) < m)
    };
    points.iter().any(|&q| {
        road(q)
            || tags.outlines.iter().any(|o| near(&o.ring, 3.0, q))
            || tags.piers.iter().any(|w| near(&w.ring, 2.0, q))
    })
}

/// Stacks a block's slot holds, 0 (empty) to 5: a block filled to its own level (two to five high; one in twelve
/// emptied), each bay across its rows mostly at that level (a quarter of them a tier lower, one in ten two: being
/// worked), and now and then one slot a tier lower or empty, so the blocks read as the ordered rows they are.
fn tiers(layout: &Layout, i: i32, j: i32, k: i32, r: i32) -> i32 {
    let block = hash01(i64::from(i), i64::from(j), layout.seed * 7 + 411);
    if block < 0.08 {
        return 0;
    }
    let level = (2.0 + (block - 0.08) / 0.92 * 3.4).floor().min(5.0);
    let bay = hash01(
        i64::from(i) * 64 + i64::from(k),
        i64::from(j),
        layout.seed * 7 + 414,
    );
    let base = if bay < 0.1 {
        level - 2.0
    } else if bay < 0.35 {
        level - 1.0
    } else {
        level
    };
    let slot = hash01(
        i64::from(i) * 64 + i64::from(k),
        i64::from(j) * 16 + i64::from(r),
        layout.seed * 7 + 412,
    );
    if slot < 0.03 {
        return 0;
    }
    let t = if slot < 0.12 { base - 1.0 } else { base };
    t.clamp(1.0, 5.0) as i32
}

/// The row's seed: three bytes the shader hashes each box's colour from.
fn row_seed(layout: &Layout, i: i32, j: i32, r: i32) -> [u8; 3] {
    let h = |s: i64| {
        (hash01(
            i64::from(i) * 16 + i64::from(r),
            i64::from(j),
            layout.seed * 13 + s,
        ) * 255.0) as u8
    };
    [h(421), h(422), h(423)]
}

/// Whether slot (`k`, `r`) of a block holds two 20 ft boxes rather than one 40 ft.
fn paired(layout: &Layout, i: i32, j: i32, k: i32, r: i32) -> bool {
    hash01(
        i64::from(i) * 64 + i64::from(k),
        i64::from(j) * 16 + i64::from(r),
        layout.seed * 7 + 413,
    ) < 0.25
}

/// The stacks of chunk `index` over its map `tags` and `ground` (chunk-local): a column a slot into [`Group::Near`] (for
/// a chunk drawn near), a box a block's row into [`Group::Far`]. Slots belong to the chunk holding their middles. The
/// containers counted.
pub fn stack(
    m: &mut MeshBuilder,
    tags: &MapTags,
    ground: &dyn Fn([f32; 2]) -> f32,
    index: [i32; 2],
) -> u32 {
    let test = YardTest::new(tags, ground);
    if !test.port {
        return 0;
    }
    let corner = Vec2::new(
        (f64::from(index[0]) * CHUNK_M) as f32,
        (f64::from(index[1]) * CHUNK_M) as f32,
    );
    let side = CHUNK_M as f32;
    let mut count = 0u32;
    let mut done: Vec<(i64, i32, i32)> = Vec::new();
    // The industrial zones about the chunk with their layouts. A terminal cut by the map's tiles comes in pieces: those
    // laid out alike fill a block together.
    let pieces: Vec<(&Zone, Layout)> = tags
        .zones
        .iter()
        .filter(|z| z.usage == Usage::Industrial)
        .filter(|z| {
            let b = z.bounds;
            !(b[2] < 0.0 || b[3] < 0.0 || b[0] > side || b[1] > side)
        })
        .filter_map(|z| quay_axis(z, tags, ground).map(|axis| (z, Layout::of(axis))))
        .collect();
    for &(zone, layout) in &pieces {
        let b = zone.bounds;
        let alike: Vec<&Zone> = pieces
            .iter()
            .filter(|(_, l)| *l == layout)
            .map(|(z, _)| *z)
            .collect();
        // The cells over the zone's box within the chunk, in the layout's frame (theatre metres).
        let lo = Vec2::new(b[0].max(-40.0), b[1].max(-40.0)) + corner;
        let hi = Vec2::new(b[2].min(side + 40.0), b[3].min(side + 40.0)) + corner;
        let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for c in corners {
            let (u, v) = (c.dot(layout.along), c.dot(layout.across));
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let (ca, cc) = (layout.cell_along(), layout.cell_across());
        for i in (u0 / ca).floor() as i32..=(u1 / ca).floor() as i32 {
            for j in (v0 / cc).floor() as i32..=(v1 / cc).floor() as i32 {
                // A block once, whichever zone piece meets it first.
                if done.contains(&(layout.seed, i, j)) {
                    continue;
                }
                let n = block(m, &layout, i, j, &alike, &test, corner);
                if n > 0 {
                    done.push((layout.seed, i, j));
                }
                count += n;
            }
        }
    }
    count
}

/// One block's stacks (cell `i`, `j`): its slots' heights where its yard and the zones laid out alike (`zones`, the
/// pieces of one terminal) hold them, then the columns and rows whose middles lie in the chunk. The containers drawn.
#[allow(clippy::too_many_arguments)]
fn block(
    m: &mut MeshBuilder,
    layout: &Layout,
    i: i32,
    j: i32,
    zones: &[&Zone],
    test: &YardTest<'_>,
    corner: Vec2,
) -> u32 {
    let inside = |p: Vec2| {
        zones
            .iter()
            .any(|z| geometry::contains_with_holes(&z.exterior, &z.holes, p.to_array()))
    };
    let (bays, rows) = (layout.bays, layout.rows);
    let local = |p: Vec2| p - corner;
    let side = CHUNK_M as f32;
    let owned = |p: Vec2| (0.0..side).contains(&p.x) && (0.0..side).contains(&p.y);
    // Nothing of the block in the chunk: skipped before any test.
    let middle = local(layout.slot(i, j, bays / 2, rows / 2));
    let reach = layout.cell_along().max(layout.cell_across());
    if middle.x < -reach || middle.y < -reach || middle.x > side + reach || middle.y > side + reach
    {
        return 0;
    }
    let mut height = vec![0i32; (bays * rows) as usize];
    let mut any = false;
    for k in 0..bays {
        let bay = local(layout.slot(i, j, k, rows / 2));
        if !inside(bay) || !test.holds(bay.to_array()) {
            continue;
        }
        for r in 0..rows {
            let p = local(layout.slot(i, j, k, r));
            if !inside(p) || blocked(test.tags, layout, p) {
                continue;
            }
            let t = tiers(layout, i, j, k, r);
            height[(r * bays + k) as usize] = t;
            any |= t > 0;
        }
    }
    if !any {
        return 0;
    }
    let h = |k: i32, r: i32| {
        if (0..bays).contains(&k) && (0..rows).contains(&r) {
            height[(r * bays + k) as usize]
        } else {
            0
        }
    };
    let ground = test.ground;
    let mut count = 0u32;
    // Near: a column a slot, its sides where a neighbour leaves them open.
    m.set_group(Group::Near);
    for r in 0..rows {
        let seed = row_seed(layout, i, j, r);
        for k in 0..bays {
            let t = h(k, r);
            if t == 0 {
                continue;
            }
            let p = local(layout.slot(i, j, k, r));
            if !owned(p) {
                continue;
            }
            count += t as u32 * if paired(layout, i, j, k, r) { 2 } else { 1 };
            if !m.near() {
                continue;
            }
            let foot = ground(p.to_array()) - 0.4;
            let top = foot + 0.4 + t as f32 * TIER_M;
            let pair = paired(layout, i, j, k, r);
            column(
                m,
                layout,
                p,
                foot,
                top,
                k,
                pair,
                seed,
                [h(k + 1, r), h(k - 1, r), h(k, r + 1), h(k, r - 1)],
                t,
            );
        }
    }
    // Far: each row's runs of filled slots as one box at their mean height.
    m.set_group(Group::Far);
    for r in 0..rows {
        let seed = row_seed(layout, i, j, r);
        let mut k = 0;
        while k < bays {
            if h(k, r) == 0 {
                k += 1;
                continue;
            }
            let start = k;
            let mut sum = 0;
            while k < bays && h(k, r) > 0 {
                sum += h(k, r);
                k += 1;
            }
            let (a, b) = (
                local(layout.slot(i, j, start, r)),
                local(layout.slot(i, j, k - 1, r)),
            );
            let mid = (a + b) * 0.5;
            if !owned(mid) {
                continue;
            }
            let mean = sum as f32 / (k - start) as f32;
            let foot = ground(mid.to_array()) - 0.4;
            let top = foot + 0.4 + mean * TIER_M;
            let run = Block {
                centre: mid,
                axis: layout.along,
                half_a: a.distance(b) * 0.5 + BOX_LONG_M * 0.5,
                half_c: BOX_WIDE_M * 0.5,
                y0: foot,
                y1: top,
            };
            stacks_box(m, &run, start, mean, seed);
        }
    }
    count
}

/// One slot's column from `foot` to `top` (`t` tiers): its top, and each side above the neighbour beside it (+a, −a,
/// +c, −c: `beside`'s tiers). `uv`: metres along the slot from its −a end and up from the foot; the normal's w the slot's
/// bay (bits 0-5) and whether it holds two 20 ft boxes (bit 6).
#[allow(clippy::too_many_arguments)]
fn column(
    m: &mut MeshBuilder,
    layout: &Layout,
    p: Vec2,
    foot: f32,
    top: f32,
    bay: i32,
    pair: bool,
    seed: [u8; 3],
    beside: [i32; 4],
    t: i32,
) {
    let param = (bay.clamp(0, 63) | if pair { 64 } else { 0 }) as i8;
    let paint = Paint {
        colour: seed,
        material: Material::Stacks,
        param,
    };
    let a = layout.along * (BOX_LONG_M * 0.5);
    let c = layout.across * (BOX_WIDE_M * 0.5);
    let at = |q: Vec2, y: f32| Vec3::new(q.x, y, q.y);
    let base = foot + 0.4;
    let la = BOX_LONG_M;
    // The top: uv.y the top box's tier, as metres up its middle.
    let tier_mark = (t as f32 - 0.5) * TIER_M;
    let [c0, c1, c2, c3] = [p - a - c, p + a - c, p + a + c, p - a + c];
    m.quad(
        [at(c0, top), at(c1, top), at(c2, top), at(c3, top)],
        [
            Vec2::new(0.0, tier_mark),
            Vec2::new(la, tier_mark),
            Vec2::new(la, tier_mark),
            Vec2::new(0.0, tier_mark),
        ],
        Vec3::Y,
        paint,
    );
    // Each side from the neighbour's top (or the foot) to the column's: (from, to, outward, uv.x at each, neighbour).
    // The ends keep one uv.x, the box they close (a pair's second at +a).
    let sides: [(Vec2, Vec2, Vec2, f32, f32, i32); 4] = [
        (c1, c2, layout.along, la, la, beside[0]),
        (c3, c0, -layout.along, 0.0, 0.0, beside[1]),
        (c2, c3, layout.across, la, 0.0, beside[2]),
        (c0, c1, -layout.across, 0.0, la, beside[3]),
    ];
    for (q0, q1, out, u0, u1, other) in sides {
        if other >= t {
            continue;
        }
        let from = if other > 0 {
            base + other as f32 * TIER_M
        } else {
            foot
        };
        let (v0, v1) = (from - base, top - base);
        m.quad(
            [at(q0, from), at(q1, from), at(q1, top), at(q0, top)],
            [
                Vec2::new(u0, v0),
                Vec2::new(u1, v0),
                Vec2::new(u1, v1),
                Vec2::new(u0, v1),
            ],
            Vec3::new(out.x, 0.0, out.y),
            paint,
        );
    }
}

/// A row's run of slots as one box (beyond the near range): uv metres along the row from the run's first slot's −a end
/// (plus that slot's place in the block) and up from the foot; the normal's w −1 (a run).
fn stacks_box(m: &mut MeshBuilder, run: &Block, first_bay: i32, mean: f32, seed: [u8; 3]) {
    let paint = Paint {
        colour: seed,
        material: Material::Stacks,
        param: -1,
    };
    let start = first_bay as f32 * SLOT_ALONG_M;
    let len = run.half_a * 2.0;
    let base = run.y0 + 0.4;
    let [c0, c1, c2, c3] = run.corners();
    let at = |q: Vec2, y: f32| Vec3::new(q.x, y, q.y);
    let mark = (mean - 0.5).max(0.5) * TIER_M;
    m.quad(
        [
            at(c0, run.y1),
            at(c1, run.y1),
            at(c2, run.y1),
            at(c3, run.y1),
        ],
        [
            Vec2::new(start, mark),
            Vec2::new(start + len, mark),
            Vec2::new(start + len, mark),
            Vec2::new(start, mark),
        ],
        Vec3::Y,
        paint,
    );
    let (v0, v1) = (run.y0 - base, run.y1 - base);
    let a = Vec3::new(run.axis.x, 0.0, run.axis.y);
    let c = run.across();
    let c3v = Vec3::new(c.x, 0.0, c.y);
    for (q0, q1, out, u0, u1) in [
        (c0, c1, -c3v, start, start + len),
        (c2, c3, c3v, start + len, start),
        (c1, c2, a, start + len, start + len),
        (c3, c0, -a, start, start),
    ] {
        m.quad(
            [
                at(q0, run.y0),
                at(q1, run.y0),
                at(q1, run.y1),
                at(q0, run.y1),
            ],
            [
                Vec2::new(u0, v0),
                Vec2::new(u1, v0),
                Vec2::new(u1, v1),
                Vec2::new(u0, v1),
            ],
            out,
            paint,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::tags::{Poi, WaterArea};

    fn square(x0: f32, z0: f32, x1: f32, z1: f32) -> Vec<[f32; 2]> {
        vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]]
    }

    /// A terminal: industrial land west of x = 700 from z = 300 to 1400, the sea (15 m deep) east of x = 700.
    fn terminal() -> (MapTags, impl Fn([f32; 2]) -> f32) {
        let tags = MapTags {
            zones: vec![Zone::new(
                Usage::Industrial,
                square(-400.0, 300.0, 700.0, 1400.0),
                vec![],
            )],
            water: vec![WaterArea::new(
                square(700.0, -200.0, 1300.0, 1200.0),
                vec![],
            )],
            pois: vec![Poi {
                at: [600.0, 700.0],
                kind: PoiKind::Harbour,
            }],
            ..MapTags::default()
        };
        let ground = |p: [f32; 2]| if p[0] > 700.0 { -15.0 } else { 3.0 };
        (tags, ground)
    }

    #[test]
    fn a_yard_is_port_land_back_from_its_apron_near_deep_water() {
        let (tags, ground) = terminal();
        let test = YardTest::new(&tags, &ground);
        assert!(test.holds([500.0, 700.0]));
        // The apron, the water, land beyond the zone, land beyond the reach of deep water.
        assert!(!test.holds([670.0, 700.0]));
        assert!(!test.holds([750.0, 700.0]));
        assert!(!test.holds([500.0, 200.0]));
        let far = MapTags {
            zones: vec![Zone::new(
                Usage::Industrial,
                square(-2000.0, 0.0, 700.0, 1000.0),
                vec![],
            )],
            ..tags.clone()
        };
        assert!(!YardTest::new(&far, &ground).holds([-500.0, 700.0]));
        // No port about: no yard.
        let quiet = MapTags {
            pois: vec![],
            ..tags.clone()
        };
        assert!(!YardTest::new(&quiet, &ground).holds([500.0, 700.0]));
        // A town's industrial block (residential about it) and a warehouse district (built over) are no yard.
        let mut town = tags.clone();
        town.zones = vec![
            Zone::new(
                Usage::Residential,
                square(-400.0, 300.0, 700.0, 1400.0),
                vec![],
            ),
            Zone::new(
                Usage::Industrial,
                square(420.0, 620.0, 580.0, 780.0),
                vec![],
            ),
        ];
        assert!(!YardTest::new(&town, &ground).holds([500.0, 700.0]));
        let mut warehouses = tags.clone();
        for k in 0..25 {
            let (x, z) = (400.0 + (k % 5) as f32 * 40.0, 600.0 + (k / 5) as f32 * 40.0);
            warehouses.outlines.push(crate::buildings::tags::Outline {
                ring: square(x, z, x + 30.0, z + 30.0),
                height: None,
                min_height: 0.0,
                colour: None,
                hide_3d: false,
            });
        }
        assert!(!YardTest::new(&warehouses, &ground).holds([500.0, 700.0]));
    }

    #[test]
    fn the_stacks_run_along_the_quay_in_the_yard_alone() {
        let (tags, ground) = terminal();
        let zone = &tags.zones[0];
        // The quay is the zone's east side, north and south.
        let axis = quay_axis(zone, &tags, &ground).unwrap();
        assert!(
            axis.x.abs() < 1e-3 && (axis.y.abs() - 1.0).abs() < 1e-3,
            "{axis}"
        );
        let mut m = MeshBuilder::default();
        let n = stack(&mut m, &tags, &ground, [0, 0]);
        let mesh = m.finish();
        assert!(n > 2000, "{n} containers");
        assert!(!mesh.range(Group::Near).is_empty() && !mesh.range(Group::Far).is_empty());
        // Every column within the zone, off the apron and inside the chunk (a far row by its middle), no taller than five
        // tiers.
        for v in &mesh.vertices {
            let [x, y, z] = v.position;
            assert!((-8.0..=700.0 - APRON_M + 8.0).contains(&x), "x {x}");
            assert!((292.0..=1008.0 + 120.0).contains(&z), "z {z}");
            assert!((2.5..=3.0 + 5.0 * TIER_M + 0.01).contains(&y), "y {y}");
            assert_eq!(v.colour[3], Material::Stacks as u8);
        }
        // Deterministic.
        let mut again = MeshBuilder::default();
        stack(&mut again, &tags, &ground, [0, 0]);
        assert_eq!(again.finish(), mesh);
    }

    #[test]
    fn a_terminal_cut_by_the_tiles_stacks_as_one() {
        let (tags, ground) = terminal();
        let mut m = MeshBuilder::default();
        let whole = stack(&mut m, &tags, &ground, [0, 0]);
        let piece =
            |z0: f32, z1: f32| Zone::new(Usage::Industrial, square(-400.0, z0, 700.0, z1), vec![]);
        for zones in [
            vec![piece(300.0, 850.0), piece(850.0, 1400.0)],
            vec![piece(850.0, 1400.0), piece(300.0, 850.0)],
        ] {
            let cut = MapTags {
                zones,
                ..tags.clone()
            };
            let mut m = MeshBuilder::default();
            assert_eq!(stack(&mut m, &cut, &ground, [0, 0]), whole);
        }
    }

    #[test]
    fn roads_and_buildings_keep_the_stacks_off() {
        let (mut tags, ground) = terminal();
        tags.roads.push(crate::buildings::tags::Road {
            a: [0.0, 750.0],
            b: [700.0, 750.0],
            half: 5.0,
            oneway: false,
        });
        let mut m = MeshBuilder::default();
        stack(&mut m, &tags, &ground, [0, 0]);
        let mesh = m.finish();
        let near = mesh.range(Group::Near);
        for t in mesh.indices[near.start as usize..near.end as usize].chunks(3) {
            let z: f32 = t
                .iter()
                .map(|&i| mesh.vertices[i as usize].position[2])
                .sum::<f32>()
                / 3.0;
            assert!(
                (z - 750.0).abs() > 5.0 + 4.0 - 1.0,
                "a column on the road at z {z}"
            );
        }
    }
}
