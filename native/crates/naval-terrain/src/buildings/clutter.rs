//! Roof clutter (native/docs/terrain/buildings.md, "Roof clutter"): what stands on a whole building's roof, as
//! instances of a few shapes the layer draws within [`CLUTTER_M`] of the camera on High and Ultra. On a flat roof: air
//! conditioners and vents, a block's stair and lift housing, a Japanese block's water tank on its stand, a New York
//! block's timber water tower, a Taiwanese roof's sheet-metal shed and steel water tanks on their stands; on a pitched
//! roof: chimneys (a row of them along a British terrace's ridge).
//!
//! In a past period (`Building::period`): no air conditioners, coolers, solar heaters or tanks, fewer housings, New
//! York's water towers kept; chimneys on most pitched houses where coal and wood heated them, and stacks on Europe's
//! blocks, pitched or flat.
//!
//! Every choice hashes the building's place and seed, so a chunk made again holds the same clutter.

use super::geometry;
use super::mesh::{PART_DOME, PART_GABLE, PART_STEEL};
use super::{Building, Kind, Region, Roof, hash01};

/// Clutter is drawn within this distance of the camera (m).
pub const CLUTTER_M: f32 = 1500.0;
/// What stands about a building on the ground and its small parts ([`Shape::Near`]: walls, fences and hedges, steps,
/// downpipes) are drawn only in chunks within this distance of the camera (m).
pub const NEAR_M: f32 = 800.0;

/// One piece: an instance of a [`Shape`], 36 bytes (`buildings.wgsl`'s `ClutterIn`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Clutter {
    /// Chunk-local x, its foot's height above the sea, chunk-local z (m).
    pub at: [f32; 3],
    /// Its turn about the vertical (rad).
    pub yaw: f32,
    /// Its width, height and depth (m).
    pub size: [f32; 3],
    /// Its [`Shape`] (bits 0-7); the layer writes its chunk's slot in bits 8-31.
    pub shape: u32,
    /// The sides' wall palette entry, the top's roof entry, a hash, and its kind (bits 0-3) and the sides' part (bits
    /// 4-6) as a building vertex's.
    pub style: [u8; 4],
}

/// A clutter shape: unit-sized, its foot's middle at the origin (`shape_mesh`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Shape {
    /// A box from 0 to 1 high, -0.5 to 0.5 across and deep.
    Box = 0,
    /// A water tower: a round tank (0.42 to 0.9 high) under a shallow cone, on four legs.
    Tower = 1,
    /// A box as [`Shape::Box`], small or low: drawn only in chunks within [`NEAR_M`].
    Near = 2,
}

pub const SHAPES: usize = 3;

/// How far from the camera a chunk's pieces of each shape are drawn (m).
pub const SHAPE_REACH: [f32; SHAPES] = [CLUTTER_M, CLUTTER_M, NEAR_M];

/// A shape's vertex, 16 bytes: its position, and its normal (snorm8) with w 127 on a top (the roof's colour) and 0 on
/// a side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ShapeVertex {
    pub position: [f32; 3],
    pub normal: [i8; 4],
}

/// The shapes' vertices and triangles, one after the other, and each shape's first index and index count.
pub struct ShapeMesh {
    pub vertices: Vec<ShapeVertex>,
    pub indices: Vec<u16>,
    pub ranges: [(u32, u32); SHAPES],
}

fn q(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

struct Shaper {
    vertices: Vec<ShapeVertex>,
    indices: Vec<u16>,
}

impl Shaper {
    fn quad(&mut self, p: [[f32; 3]; 4], n: [f32; 3], top: bool) {
        let first = self.vertices.len() as u16;
        for v in p {
            self.vertices.push(ShapeVertex {
                position: v,
                normal: [q(n[0]), q(n[1]), q(n[2]), if top { 127 } else { 0 }],
            });
        }
        self.indices
            .extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    /// An axis-aligned box from `lo` to `hi`, its four sides and top (no bottom).
    fn cuboid(&mut self, lo: [f32; 3], hi: [f32; 3], top: bool) {
        let [x0, y0, z0] = lo;
        let [x1, y1, z1] = hi;
        // Counter-clockwise seen from outside (wgpu's front face).
        self.quad(
            [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
            [0.0, 0.0, 1.0],
            false,
        );
        self.quad(
            [[x1, y0, z0], [x0, y0, z0], [x0, y1, z0], [x1, y1, z0]],
            [0.0, 0.0, -1.0],
            false,
        );
        self.quad(
            [[x1, y0, z1], [x1, y0, z0], [x1, y1, z0], [x1, y1, z1]],
            [1.0, 0.0, 0.0],
            false,
        );
        self.quad(
            [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
            [-1.0, 0.0, 0.0],
            false,
        );
        self.quad(
            [[x0, y1, z1], [x1, y1, z1], [x1, y1, z0], [x0, y1, z0]],
            [0.0, 1.0, 0.0],
            top,
        );
    }
}

/// The shapes' mesh.
pub fn shape_mesh() -> ShapeMesh {
    let mut s = Shaper {
        vertices: Vec::new(),
        indices: Vec::new(),
    };
    let mut ranges = [(0, 0); SHAPES];
    // The box.
    s.cuboid([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5], true);
    ranges[Shape::Box as usize] = (0, s.indices.len() as u32);
    // The water tower: legs, a smooth drum, a cone.
    let start = s.indices.len() as u32;
    for (x, z) in [(-0.33, -0.33), (0.33, -0.33), (0.33, 0.33), (-0.33, 0.33)] {
        s.cuboid(
            [x - 0.035, 0.0, z - 0.035],
            [x + 0.035, 0.43, z + 0.035],
            false,
        );
    }
    let sides = 12;
    let ring = |k: u32| {
        let a = k as f32 / sides as f32 * std::f32::consts::TAU;
        (a.cos(), a.sin())
    };
    for k in 0..sides {
        let ((c0, s0), (c1, s1)) = (ring(k), ring(k + 1));
        let first = s.vertices.len() as u16;
        for (c, sn, y) in [(c0, s0, 0.42), (c1, s1, 0.42), (c1, s1, 0.9), (c0, s0, 0.9)] {
            s.vertices.push(ShapeVertex {
                position: [0.5 * c, y, 0.5 * sn],
                normal: [q(c), 0, q(sn), 0],
            });
        }
        // Counter-clockwise from outside: the angle runs from +x toward +z, so the quad turns the other way.
        s.indices
            .extend_from_slice(&[first, first + 2, first + 1, first, first + 3, first + 2]);
        // The cone over it, rising to the middle.
        let first = s.vertices.len() as u16;
        let slope = 0.1f32.atan2(0.5);
        let (ny, nr) = (slope.cos(), slope.sin());
        let mid = {
            let a = (k as f32 + 0.5) / sides as f32 * std::f32::consts::TAU;
            (a.cos(), a.sin())
        };
        for (c, sn, y, r) in [
            (c0, s0, 0.9, 0.5),
            (c1, s1, 0.9, 0.5),
            (mid.0, mid.1, 1.0, 0.0),
        ] {
            s.vertices.push(ShapeVertex {
                position: [r * c, y, r * sn],
                normal: [q(mid.0 * nr), q(ny), q(mid.1 * nr), 127],
            });
        }
        s.indices.extend_from_slice(&[first, first + 2, first + 1]);
    }
    ranges[Shape::Tower as usize] = (start, s.indices.len() as u32 - start);
    ranges[Shape::Near as usize] = ranges[Shape::Box as usize];
    ShapeMesh {
        vertices: s.vertices,
        indices: s.indices,
        ranges,
    }
}

/// The building's roof clutter in `region`, its eaves at `eave` (m above the sea), appended to `out`.
pub fn roof_clutter(b: &Building, region: Region, eave: f32, out: &mut Vec<Clutter>) {
    if b.ring.len() < 3 {
        return;
    }
    let c = geometry::centroid(&b.ring);
    let h = |k: i64| {
        hash01(
            (c[0] * 8.0) as i64 + i64::from(b.seed) * 7919,
            (c[1] * 8.0) as i64,
            400 + k,
        )
    };
    match b.roof {
        Roof::Flat => flat_roof(b, region, eave, c, &h, out),
        Roof::Gabled { rise } | Roof::Hipped { rise } if b.ring.len() == 4 => {
            chimneys(b, region, eave, rise, &h, out)
        }
        _ => {}
    }
}

fn style(wall: u8, top: u8, seed: u8, kind: Kind, part: u8) -> [u8; 4] {
    [wall, top, seed, (kind as u8 & 15) | ((part & 7) << 4)]
}

/// The direction of the ring's longest side (rad, from +x toward +z), which the clutter lines up with.
fn long_axis(ring: &[[f32; 2]]) -> f32 {
    let mut best = (0.0f32, 0.0f32);
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
        let l = geometry::distance(a, b);
        if l > best.0 {
            best = (l, (b[1] - a[1]).atan2(b[0] - a[0]));
        }
    }
    best.1
}

#[allow(clippy::too_many_arguments)]
fn place(
    ring: &[[f32; 2]],
    lo: [f32; 2],
    hi: [f32; 2],
    half: f32,
    taken: &mut Vec<([f32; 2], f32)>,
    first: Option<[f32; 2]>,
    h: &dyn Fn(i64) -> f32,
    key: i64,
) -> Option<[f32; 2]> {
    for t in 0..12i64 {
        let p = match (t, first) {
            (0, Some(p)) => p,
            _ => [
                lo[0] + (hi[0] - lo[0]) * h(key * 31 + t * 2),
                lo[1] + (hi[1] - lo[1]) * h(key * 31 + t * 2 + 1),
            ],
        };
        if geometry::contains(ring, p)
            && geometry::ring_distance(ring, p) >= half + 0.6
            && taken
                .iter()
                .all(|(q, r)| geometry::distance(p, *q) >= r + half + 0.4)
        {
            taken.push((p, half));
            return Some(p);
        }
    }
    None
}

fn flat_roof(
    b: &Building,
    region: Region,
    eave: f32,
    c: [f32; 2],
    h: &dyn Fn(i64) -> f32,
    out: &mut Vec<Clutter>,
) {
    let area = geometry::area(&b.ring);
    let residential_small = matches!(b.kind, Kind::House | Kind::Shed);
    let served = matches!(
        b.kind,
        Kind::Block
            | Kind::Tower
            | Kind::Office
            | Kind::Industrial
            | Kind::Warehouse
            | Kind::Terrace
            | Kind::House
    );
    if !served || area < 30.0 {
        return;
    }
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for p in &b.ring {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let yaw = long_axis(&b.ring);
    let mut taken = Vec::new();
    let push = |p: [f32; 2],
                y: f32,
                size: [f32; 3],
                shape: Shape,
                style: [u8; 4],
                out: &mut Vec<Clutter>| {
        out.push(Clutter {
            at: [p[0], y, p[1]],
            yaw,
            size,
            shape: shape as u32,
            style,
        });
    };
    let past = b.period.is_past();
    // Taiwan's sheet-metal rooftop shed (頂樓加蓋) over much of a townhouse's or a walk-up's roof, its stair's head in
    // it, and steel water tanks on their stands on its roof or on the building's.
    let taiwan = region == Region::Taiwan && !past;
    let shed = if taiwan { shed_of(b, h) } else { None };
    if let Some(s) = &shed {
        let walls = [11u8, 25, 27, 13, 11, 25, 2][(h(133) * 6.99) as usize];
        let top = [6u8, 7, 8, 15, 29, 6, 8, 7][(h(134) * 7.99) as usize];
        out.push(Clutter {
            at: [s.centre[0], eave, s.centre[1]],
            yaw: s.axis[1].atan2(s.axis[0]),
            size: [s.long, s.height, s.wide],
            shape: Shape::Box as u32,
            style: style(walls, top, b.seed, b.kind, PART_GABLE),
        });
        taken.push((s.centre, 0.5 * s.long.max(s.wide)));
    }
    if taiwan {
        taiwan_tanks(b, shed.as_ref(), eave, (lo, hi), &mut taken, h, out);
    }
    // A block's stair and lift housing, in its walls' colour, near its middle (in a past period on two in five of the
    // taller blocks; none under a Taiwanese roof's shed).
    let storeys = b.storeys;
    let housed = (!past || (storeys >= 5 && h(120) < 0.4)) && shed.is_none();
    if matches!(b.kind, Kind::Block | Kind::Tower | Kind::Office)
        && storeys >= 4
        && area > 250.0
        && housed
    {
        let size = [3.0 + 1.5 * h(1), 2.6 + 0.4 * h(2), 4.0 + 2.0 * h(3)];
        if let Some(p) = place(&b.ring, lo, hi, size[2] * 0.5, &mut taken, Some(c), h, 1) {
            push(
                p,
                eave,
                size,
                Shape::Box,
                style(b.wall, b.roof_colour, b.seed, b.kind, PART_GABLE),
                out,
            );
        }
    }
    // Water: a Japanese block's tank on its stand, a New York block's timber tower.
    if matches!(b.kind, Kind::Block | Kind::Tower) {
        if region == Region::EastAsia && !past && storeys >= 3 && h(4) < 0.55 {
            if let Some(p) = place(&b.ring, lo, hi, 1.6, &mut taken, None, h, 2) {
                let tank = [13u8, 27, 7, 13][(h(5) * 3.99) as usize];
                push(
                    p,
                    eave,
                    [2.4, 0.9, 2.0],
                    Shape::Box,
                    style(11, 7, b.seed, b.kind, PART_STEEL),
                    out,
                );
                push(
                    p,
                    eave + 0.9,
                    [2.2, 1.6, 1.8],
                    Shape::Box,
                    style(tank, 29, b.seed, b.kind, PART_STEEL),
                    out,
                );
            }
        } else if region == Region::American && storeys >= 6 && h(6) < 0.5 {
            let d = 3.4 + 1.2 * h(7);
            if let Some(p) = place(&b.ring, lo, hi, d * 0.5, &mut taken, None, h, 3) {
                push(
                    p,
                    eave,
                    [d, d * 1.8, d],
                    Shape::Tower,
                    style(12, 11, b.seed, b.kind, PART_GABLE),
                    out,
                );
            }
        }
    }
    // A past period's European block: brick stacks on its flat roof.
    if past && matches!(b.kind, Kind::Block | Kind::Tower) && european(region) {
        let n = ((area / 120.0).sqrt() * (0.7 + 0.6 * h(121)))
            .round()
            .clamp(2.0, 8.0) as u32;
        for k in 0..n {
            let k = i64::from(k);
            let size = [
                1.2 + 1.2 * h(122 + k),
                1.4 + 0.8 * h(140 + k),
                0.6 + 0.2 * h(160),
            ];
            if let Some(p) = place(&b.ring, lo, hi, size[0] * 0.5, &mut taken, None, h, 90 + k) {
                push(
                    p,
                    eave,
                    size,
                    Shape::Box,
                    style(stack_brick(region, h(103)), 14, b.seed, b.kind, PART_GABLE),
                    out,
                );
            }
        }
    }
    // Air conditioners (an evaporative cooler on a dry country's or the American West's flat house) and vents; in a
    // past period vents alone.
    let (units, vents) = if residential_small || b.kind == Kind::Terrace {
        let cooler = matches!(region, Region::West | Region::Arid) && h(8) < 0.6;
        (u32::from(cooler), 0)
    } else {
        (
            ((area / 140.0) * (0.5 + h(9))).round().clamp(1.0, 16.0) as u32,
            (area / 260.0).round().clamp(0.0, 10.0) as u32,
        )
    };
    let units = if past { 0 } else { units };
    for k in 0..units {
        let size = [
            1.0 + 0.5 * h(20 + i64::from(k)),
            0.9 + 0.3 * h(40),
            0.8 + 0.3 * h(41),
        ];
        if let Some(p) = place(
            &b.ring,
            lo,
            hi,
            size[0] * 0.6,
            &mut taken,
            None,
            h,
            10 + i64::from(k),
        ) {
            let wall = [13u8, 2, 11, 13][(h(60 + i64::from(k)) * 3.99) as usize];
            push(
                p,
                eave,
                size,
                Shape::Box,
                style(wall, 7, b.seed, b.kind, PART_STEEL),
                out,
            );
        }
    }
    for k in 0..vents {
        let size = [0.45, 0.5 + 0.4 * h(80 + i64::from(k)), 0.45];
        if let Some(p) = place(&b.ring, lo, hi, 0.3, &mut taken, None, h, 40 + i64::from(k)) {
            push(
                p,
                eave,
                size,
                Shape::Box,
                style(11, 22, b.seed, b.kind, PART_STEEL),
                out,
            );
        }
    }
    // Water tanks on a warm, dry country's flat roofs: a solar heater's white tank over its dark panel in Greece,
    // Cyprus, Malta and the Mediterranean, white and black tanks in North Africa, Egypt and the Middle East.
    let warm = matches!(
        region,
        Region::Aegean
            | Region::Malta
            | Region::Mediterranean
            | Region::NorthAfrica
            | Region::Nile
            | Region::Arid
    );
    let tanks = if past
        || !warm
        || !matches!(
            b.kind,
            Kind::House | Kind::Terrace | Kind::Block | Kind::Tower
        ) {
        0
    } else if residential_small || b.kind == Kind::Terrace {
        u32::from(h(110) < 0.7)
    } else {
        ((area / 120.0) * (0.4 + 0.6 * h(111)))
            .round()
            .clamp(1.0, 8.0) as u32
    };
    let solar = matches!(
        region,
        Region::Aegean | Region::Malta | Region::Mediterranean
    );
    let across = [-yaw.sin(), yaw.cos()];
    for k in 0..tanks {
        let k = i64::from(k);
        let Some(p) = place(&b.ring, lo, hi, 1.1, &mut taken, None, h, 70 + k) else {
            continue;
        };
        let (side, top) = if solar || h(112 + k) < 0.5 {
            (13u8, 15u8)
        } else {
            (14, 12)
        };
        push(
            p,
            eave,
            [1.5, 0.7, 0.7],
            Shape::Box,
            style(side, top, b.seed, b.kind, PART_STEEL),
            out,
        );
        if solar {
            let at = [p[0] + across[0] * 0.9, p[1] + across[1] * 0.9];
            push(
                at,
                eave + 0.3,
                [1.8, 0.12, 1.1],
                Shape::Box,
                style(28, 25, b.seed, b.kind, PART_STEEL),
                out,
            );
        }
    }
}

/// A Taiwanese roof's shed: its middle and long axis on the roof (chunk-local), its length along that axis, width and
/// height (m).
struct Shed {
    centre: [f32; 2],
    axis: [f32; 2],
    long: f32,
    wide: f32,
    height: f32,
}

/// The shed (頂樓加蓋) on a Taiwanese townhouse's (three in five), walk-up's (half) or tower's (one in six) flat roof:
/// its whole width but a parapet's margin each side, 45-90% of its length from one end, 2.3-2.9 m high; none on a roof
/// too small or not near its rectangle.
fn shed_of(b: &Building, h: &dyn Fn(i64) -> f32) -> Option<Shed> {
    let share = match b.kind {
        Kind::House | Kind::Terrace => 0.6,
        Kind::Block => 0.5,
        Kind::Tower => 0.16,
        _ => 0.0,
    };
    if h(130) >= share {
        return None;
    }
    let rect = geometry::min_area_rect(&b.ring)?;
    if rect.area() > 1.15 * geometry::area(&b.ring) || rect.area() < 25.0 {
        return None;
    }
    let (along, across) = (rect.half[0], rect.half[1]);
    let margin = 0.35;
    let long = (2.0 * along * (0.45 + 0.45 * h(131)) - margin).min(2.0 * (along - margin));
    let wide = 2.0 * (across - margin);
    if long < 2.5 || wide < 2.5 {
        return None;
    }
    // From one end or the other.
    let end = if h(132) < 0.5 { -1.0 } else { 1.0 };
    let shift = end * (along - margin - long * 0.5);
    let a = rect.axis;
    let centre = [rect.centre[0] + a[0] * shift, rect.centre[1] + a[1] * shift];
    let n = [-a[1], a[0]];
    let corner = |x: f32, y: f32| {
        [
            centre[0] + a[0] * x * long * 0.5 + n[0] * y * wide * 0.5,
            centre[1] + a[1] * x * long * 0.5 + n[1] * y * wide * 0.5,
        ]
    };
    // Its corners on the roof and no corner of the roof within it (a recess between its corners).
    let within = |p: [f32; 2]| {
        let d = [p[0] - centre[0], p[1] - centre[1]];
        (d[0] * a[0] + d[1] * a[1]).abs() < long * 0.5
            && (d[0] * n[0] + d[1] * n[1]).abs() < wide * 0.5
    };
    let inside = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .all(|&(x, y)| geometry::contains(&b.ring, corner(x, y)))
        && !b.ring.iter().any(|&p| within(p));
    inside.then_some(Shed {
        centre,
        axis: a,
        long,
        wide,
        height: 2.3 + 0.6 * h(135),
    })
}

/// Taiwan's water tanks: stainless (now and then white) cylinders on steel stands, one on a townhouse (two on one in
/// four), one to three on a walk-up, two to four larger on a tower; on the shed's roof from its far end when there is
/// one, else about the roof.
#[allow(clippy::too_many_arguments)]
fn taiwan_tanks(
    b: &Building,
    shed: Option<&Shed>,
    eave: f32,
    (lo, hi): ([f32; 2], [f32; 2]),
    taken: &mut Vec<([f32; 2], f32)>,
    h: &dyn Fn(i64) -> f32,
    out: &mut Vec<Clutter>,
) {
    let n = match b.kind {
        Kind::House | Kind::Terrace => 1 + u32::from(h(141) < 0.25),
        Kind::Block => 1 + (h(141) * 2.99) as u32,
        Kind::Tower => 2 + (h(141) * 2.99) as u32,
        _ => 0,
    };
    let d = if b.kind == Kind::Tower {
        1.8 + 0.8 * h(142)
    } else {
        1.0 + 0.45 * h(142)
    };
    let wall = [11u8, 11, 2, 13][(h(144) * 3.99) as usize];
    for k in 0..n {
        let k64 = i64::from(k);
        let (p, foot) = if let Some(s) = shed {
            let along = s.long * 0.5 - d * 0.5 - 0.3 - k as f32 * (d + 0.35);
            if along < d * 0.5 - s.long * 0.5 {
                break;
            }
            let across = (h(145 + k64) - 0.5) * (s.wide - d - 0.4).max(0.0);
            let [a0, a1] = s.axis;
            (
                [
                    s.centre[0] + a0 * along - a1 * across,
                    s.centre[1] + a1 * along + a0 * across,
                ],
                eave + s.height,
            )
        } else {
            match place(&b.ring, lo, hi, d * 0.5, taken, None, h, 150 + k64) {
                Some(p) => (p, eave),
                None => continue,
            }
        };
        out.push(Clutter {
            at: [p[0], foot, p[1]],
            yaw: 0.0,
            size: [d, 2.2 * d, d],
            shape: Shape::Tower as u32,
            style: style(wall, 29, b.seed, b.kind, PART_STEEL),
        });
    }
}

fn chimneys(
    b: &Building,
    region: Region,
    eave: f32,
    rise: f32,
    h: &dyn Fn(i64) -> f32,
    out: &mut Vec<Clutter>,
) {
    let past = b.period.is_past();
    let block = past && b.kind == Kind::Block && european(region);
    if !(matches!(b.kind, Kind::House | Kind::Terrace) || block) || rise < 0.6 {
        return;
    }
    let share = if past {
        past_chimney_share(region)
    } else {
        todays_chimney_share(region)
    };
    if h(100) >= share {
        return;
    }
    let r = &b.ring;
    let (p0, p1, p2, p3) = (r[0], r[1], r[2], r[3]);
    let long = geometry::distance(p0, p1);
    let short = geometry::distance(p1, p2);
    if long < 3.0 || short < 3.0 {
        return;
    }
    if block {
        // A past period's block: stacks along its ridge every 8-10 m, clear of the hips.
        let mid = |a: [f32; 2], c: [f32; 2]| [(a[0] + c[0]) * 0.5, (a[1] + c[1]) * 0.5];
        let (ra, rb) = (mid(p3, p0), mid(p1, p2));
        let yaw = (p1[1] - p0[1]).atan2(p1[0] - p0[0]);
        let margin = if matches!(b.roof, Roof::Hipped { .. }) {
            (short * 0.5 / long).min(0.45) + 0.05
        } else {
            0.08
        };
        let n = (long * (1.0 - 2.0 * margin) / (8.0 + 2.0 * h(106)))
            .round()
            .clamp(1.0, 6.0) as u32;
        let wall = stack_brick(region, h(103));
        for k in 0..n {
            let t = margin + (1.0 - 2.0 * margin) * (k as f32 + 0.5) / n as f32;
            let p = [ra[0] + (rb[0] - ra[0]) * t, ra[1] + (rb[1] - ra[1]) * t];
            let foot = eave + rise * 0.45;
            let top = eave + rise + 0.9 + 0.6 * h(105);
            out.push(Clutter {
                at: [p[0], foot, p[1]],
                yaw,
                size: [1.3 + 0.6 * h(107), top - foot, 0.7],
                shape: Shape::Box as u32,
                style: style(wall, 14, b.seed, b.kind, PART_GABLE),
            });
        }
        return;
    }
    chimney_row(b, region, eave, rise, h, out);
}

/// The regions whose old blocks carry chimney stacks (coal heat): Europe's.
fn european(region: Region) -> bool {
    matches!(
        region,
        Region::Temperate | Region::British | Region::Celtic | Region::NorthSea | Region::Nordic
    )
}

/// A chimney stack's brick, sooty and plain (red, brown, yellow; Holland's klinker).
fn stack_brick(region: Region, t: f32) -> u8 {
    if region == Region::NorthSea && t < 0.3 {
        super::palette::KLINKER
    } else {
        [4u8, 5, 5, 6][(t * 3.99) as usize]
    }
}

/// The share of pitched houses with a chimney in a past period: most where coal and wood heated them.
fn past_chimney_share(region: Region) -> f32 {
    match region {
        Region::British | Region::Celtic => 0.97,
        Region::Temperate => 0.9,
        Region::NorthSea | Region::American | Region::Nordic => 0.85,
        Region::Iceland | Region::West => 0.6,
        Region::Florida => 0.2,
        _ => todays_chimney_share(region),
    }
}

fn todays_chimney_share(region: Region) -> f32 {
    match region {
        Region::British | Region::Celtic => 0.95,
        Region::Temperate => 0.8,
        Region::NorthSea => 0.6,
        Region::American => 0.6,
        Region::Nordic => 0.55,
        Region::Iceland => 0.3,
        Region::Florida => 0.1,
        Region::Aegean => 0.05,
        Region::Malta | Region::NorthAfrica => 0.02,
        Region::Nile => 0.0,
        Region::West => 0.45,
        Region::Mediterranean => 0.35,
        Region::Arid => 0.1,
        Region::Tropical => 0.05,
        Region::EastAsia | Region::Taiwan | Region::Hawaii | Region::Micronesia | Region::Atoll => {
            0.0
        }
    }
}

/// A house's or terrace's chimneys along its ridge.
fn chimney_row(
    b: &Building,
    region: Region,
    eave: f32,
    rise: f32,
    h: &dyn Fn(i64) -> f32,
    out: &mut Vec<Clutter>,
) {
    let r = &b.ring;
    let (p0, p1, p2, p3) = (r[0], r[1], r[2], r[3]);
    let long = geometry::distance(p0, p1);
    let short = geometry::distance(p1, p2);
    let mid = |a: [f32; 2], c: [f32; 2]| [(a[0] + c[0]) * 0.5, (a[1] + c[1]) * 0.5];
    let (ra, rb) = (mid(p3, p0), mid(p1, p2));
    let yaw = (p1[1] - p0[1]).atan2(p1[0] - p0[0]);
    // Along the ridge, clear of a hipped roof's hips; a terrace's stacks on its party walls every 5-6 m (in the
    // British way, temperate, Scottish and Irish).
    let hipped = matches!(b.roof, Roof::Hipped { .. });
    let margin = if hipped {
        (short * 0.5 / long).min(0.45) + 0.05
    } else {
        0.12
    };
    let along: Vec<f32> = if b.kind == Kind::Terrace
        && matches!(region, Region::Temperate | Region::British | Region::Celtic)
    {
        // A terraced house's own ridge (across its width): its stack on one party wall.
        let n = (long / 5.5).round().max(1.0) as u32;
        if n <= 1 {
            vec![0.0]
        } else {
            (1..n).map(|k| k as f32 / n as f32).collect()
        }
    } else if h(101) < 0.5 {
        vec![margin + (1.0 - 2.0 * margin) * 0.2 * h(102)]
    } else {
        vec![1.0 - margin - (1.0 - 2.0 * margin) * 0.2 * h(102)]
    };
    let brick = matches!(
        region,
        Region::Temperate | Region::American | Region::British | Region::NorthSea
    );
    let wall = if brick {
        [4u8, 5, 4, 6][(h(103) * 3.99) as usize]
    } else if matches!(region, Region::Nordic | Region::Iceland) {
        [2u8, 13, 3][(h(103) * 2.99) as usize]
    } else {
        b.wall
    };
    let wide = 0.6 + 0.35 * h(104);
    for t in along {
        let t = t.clamp(margin, 1.0 - margin);
        let p = [ra[0] + (rb[0] - ra[0]) * t, ra[1] + (rb[1] - ra[1]) * t];
        let foot = eave + rise * 0.45;
        let top = eave + rise + 0.7 + 0.5 * h(105);
        out.push(Clutter {
            at: [p[0], foot, p[1]],
            yaw,
            size: [
                if b.kind == Kind::Terrace {
                    wide * 1.6
                } else {
                    wide
                },
                top - foot,
                0.55,
            ],
            shape: Shape::Box as u32,
            style: style(wall, 14, b.seed, b.kind, PART_GABLE),
        });
    }
}

/// Sorts a chunk's clutter by shape, and counts each.
pub fn sort_by_shape(clutter: &mut [Clutter]) -> [u32; SHAPES] {
    clutter.sort_by_key(|c| c.shape & 0xff);
    let mut counts = [0; SHAPES];
    for c in clutter.iter() {
        counts[(c.shape & 0xff) as usize] += 1;
    }
    counts
}

/// The top's part for a shape vertex's top (the shader's): a smooth roof in the top's colour.
pub const TOP_PART: u8 = PART_DOME;

#[cfg(test)]
mod tests {
    use super::*;

    fn building(ring: Vec<[f32; 2]>, kind: Kind, roof: Roof, storeys: u8) -> Building {
        Building {
            ring,
            kind,
            roof,
            ground: [4.0, 5.0, 6.0],
            height: f32::from(storeys) * 3.0,
            base: 0.0,
            storeys,
            storey_m: 3.0,
            wall: 1,
            roof_colour: 9,
            seed: 7,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period: crate::buildings::Period::Today,
        }
    }

    fn rect(x: f32, z: f32, w: f32, d: f32) -> Vec<[f32; 2]> {
        vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]]
    }

    #[test]
    fn a_taiwanese_shed_keeps_off_a_recess_in_its_roof() {
        let h = |_: i64| 0.5;
        let whole = building(rect(0.0, 0.0, 20.0, 10.0), Kind::Terrace, Roof::Flat, 4);
        assert!(shed_of(&whole, &h).is_some());
        // A recess 2 m wide and 4 m deep into one side, between the shed's corners.
        let notched = vec![
            [0.0, 0.0],
            [20.0, 0.0],
            [20.0, 10.0],
            [11.0, 10.0],
            [11.0, 6.0],
            [9.0, 6.0],
            [9.0, 10.0],
            [0.0, 10.0],
        ];
        let notched = building(notched, Kind::Terrace, Roof::Flat, 4);
        assert!(shed_of(&notched, &h).is_none());
    }

    #[test]
    fn the_shapes_close_with_their_normals_out() {
        let m = shape_mesh();
        for (first, count) in m.ranges {
            assert!(count > 0 && count % 3 == 0);
            for t in m.indices[first as usize..(first + count) as usize].chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
                let e1 = [
                    b.position[0] - a.position[0],
                    b.position[1] - a.position[1],
                    b.position[2] - a.position[2],
                ];
                let e2 = [
                    c.position[0] - a.position[0],
                    c.position[1] - a.position[1],
                    c.position[2] - a.position[2],
                ];
                let n = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let v = a.normal;
                let d = n[0] * f32::from(v[0]) + n[1] * f32::from(v[1]) + n[2] * f32::from(v[2]);
                assert!(d > 0.0, "a triangle wound against its normal");
            }
        }
        assert!(m.vertices.len() < u16::MAX as usize);
    }

    /// Every region's clutter on a few buildings, many seeds each.
    fn sample(period: crate::buildings::Period) -> Vec<(Region, Kind, Vec<Clutter>)> {
        use crate::buildings::region::REGIONS;
        let shapes = [
            (Kind::Block, Roof::Flat, 6, rect(0.0, 0.0, 40.0, 25.0)),
            (Kind::Block, Roof::Flat, 3, rect(0.0, 0.0, 22.0, 14.0)),
            (Kind::Tower, Roof::Flat, 12, rect(0.0, 0.0, 30.0, 30.0)),
            (Kind::Office, Roof::Flat, 8, rect(0.0, 0.0, 50.0, 30.0)),
            (Kind::Warehouse, Roof::Flat, 2, rect(0.0, 0.0, 60.0, 30.0)),
            (Kind::House, Roof::Flat, 1, rect(0.0, 0.0, 12.0, 9.0)),
            (Kind::Terrace, Roof::Flat, 2, rect(0.0, 0.0, 20.0, 9.0)),
            (
                Kind::House,
                Roof::Gabled { rise: 2.5 },
                2,
                rect(0.0, 0.0, 10.0, 7.0),
            ),
            (
                Kind::House,
                Roof::Hipped { rise: 2.0 },
                1,
                rect(0.0, 0.0, 14.0, 9.0),
            ),
            (
                Kind::Terrace,
                Roof::Gabled { rise: 3.0 },
                2,
                rect(0.0, 0.0, 30.0, 8.0),
            ),
            (
                Kind::Block,
                Roof::Hipped { rise: 3.5 },
                4,
                rect(0.0, 0.0, 30.0, 14.0),
            ),
        ];
        let mut out = Vec::new();
        for region in REGIONS {
            for (kind, roof, storeys, ring) in &shapes {
                let mut pieces = Vec::new();
                for seed in 0..24u8 {
                    let mut b = building(
                        ring.iter()
                            .map(|p| [p[0] + f32::from(seed) * 70.0, p[1]])
                            .collect(),
                        *kind,
                        *roof,
                        *storeys,
                    );
                    b.seed = seed.wrapping_mul(37);
                    b.period = period;
                    roof_clutter(&b, region, 3.0 * f32::from(*storeys), &mut pieces);
                }
                out.push((region, *kind, pieces));
            }
        }
        out
    }

    #[test]
    fn a_past_periods_roofs_carry_no_modern_plant() {
        let then = sample(crate::buildings::Period::Year(1942));
        let now = sample(crate::buildings::Period::Today);
        let part = |c: &Clutter| (c.style[3] >> 4) & 7;
        for (region, kind, pieces) in &then {
            for c in pieces {
                // No air conditioners, coolers, solar heaters or tanks: the only steel left is a vent.
                if part(c) == PART_STEEL {
                    assert!(c.size[0] < 0.5, "{region:?} {kind:?} {c:?}");
                }
            }
        }
        let count = |set: &[(Region, Kind, Vec<Clutter>)],
                     region: Region,
                     kind: Kind,
                     f: &dyn Fn(&Clutter) -> bool| {
            set.iter()
                .filter(|(r, k, _)| *r == region && *k == kind)
                .map(|(_, _, p)| p.iter().filter(|c| f(c)).count())
                .sum::<usize>()
        };
        let tall = |c: &Clutter| part(c) == PART_GABLE && c.size[2] > 3.0;
        // Fewer housings; New York's water towers kept.
        assert!(
            count(&then, Region::Temperate, Kind::Block, &tall)
                < count(&now, Region::Temperate, Kind::Block, &tall)
        );
        let tower = |c: &Clutter| c.shape == Shape::Tower as u32;
        let towers = count(&then, Region::American, Kind::Tower, &tower);
        assert!(towers > 0 && towers + 2 >= count(&now, Region::American, Kind::Tower, &tower));
        // Chimneys on most pitched houses where coal and wood heated them, none on a Japanese house; stacks on Europe's
        // blocks, flat or hipped.
        let stack = |c: &Clutter| part(c) == PART_GABLE;
        let houses = |set: &[(Region, Kind, Vec<Clutter>)], region| {
            set.iter()
                .filter(|(r, k, p)| *r == region && *k == Kind::House && !p.is_empty())
                .map(|(_, _, p)| p.iter().filter(|c| stack(c)).count())
                .sum::<usize>()
        };
        for region in [
            Region::British,
            Region::Temperate,
            Region::NorthSea,
            Region::American,
            Region::Nordic,
        ] {
            // Two pitched house shapes of 24 seeds each: most carry one.
            assert!(
                houses(&then, region) >= 34,
                "{region:?} {}",
                houses(&then, region)
            );
            assert!(houses(&then, region) >= houses(&now, region), "{region:?}");
        }
        assert_eq!(houses(&then, Region::EastAsia), 0);
        for region in [Region::British, Region::NorthSea] {
            let blocks: usize = then
                .iter()
                .filter(|(r, k, _)| *r == region && *k == Kind::Block)
                .map(|(_, _, p)| {
                    p.iter()
                        .filter(|c| stack(c) && c.size[1] < 2.5 && c.size[2] < 0.9)
                        .count()
                })
                .sum();
            assert!(blocks > 40, "{region:?} {blocks}");
        }
        // Where no chimneys or stacks are added, a past roof carries no more than today's (no vents where none stand).
        for ((region, kind, past), (_, _, today)) in then.iter().zip(&now) {
            if !european(*region) && past_chimney_share(*region) == todays_chimney_share(*region) {
                assert!(past.len() <= today.len(), "{region:?} {kind:?}");
            }
        }
    }

    #[test]
    fn taiwans_roofs_carry_sheds_and_steel_tanks_today_only() {
        let now = sample(crate::buildings::Period::Today);
        let then = sample(crate::buildings::Period::Year(1942));
        let part = |c: &Clutter| (c.style[3] >> 4) & 7;
        // A shed's top is sheet (a stair housing's the building's roof).
        let shed = |c: &Clutter| {
            c.shape == Shape::Box as u32
                && c.size[0] >= 2.5
                && c.size[2] >= 2.5
                && c.size[1] >= 2.3
                && [6, 7, 8, 15, 29].contains(&c.style[1])
        };
        let tank = |c: &Clutter| c.shape == Shape::Tower as u32 && part(c) == PART_STEEL;
        let count = |set: &[(Region, Kind, Vec<Clutter>)],
                     region: Region,
                     kind: Kind,
                     f: &dyn Fn(&Clutter) -> bool| {
            set.iter()
                .filter(|(r, k, _)| *r == region && *k == kind)
                .map(|(_, _, p)| p.iter().filter(|c| f(c)).count())
                .sum::<usize>()
        };
        // Of 24 flat roofs each: sheds on about three townhouses in five and half the walk-ups, a tank on every
        // townhouse and walk-up, two or more on a tower.
        let terraces = count(&now, Region::Taiwan, Kind::Terrace, &shed);
        assert!((8..=20).contains(&terraces), "{terraces}");
        assert!(count(&now, Region::Taiwan, Kind::Block, &shed) >= 6);
        assert!(count(&now, Region::Taiwan, Kind::Terrace, &tank) >= 24);
        assert!(count(&now, Region::Taiwan, Kind::House, &tank) >= 24);
        assert!(count(&now, Region::Taiwan, Kind::Tower, &tank) >= 48);
        // A shed within its roof, its sides sheet (grey, blue, cream, white or concrete) under a sheet roof; a tank on
        // a shed stands on its roof.
        for (region, kind, pieces) in &now {
            if *region != Region::Taiwan || *kind != Kind::Terrace {
                continue;
            }
            for c in pieces.iter().filter(|c| shed(c)) {
                assert!(c.size[0] <= 20.0 && c.size[2] <= 9.0, "{c:?}");
                assert!(
                    [11, 25, 27, 13, 2].contains(&c.style[0])
                        && [6, 7, 8, 15, 29].contains(&c.style[1])
                );
                assert_eq!(part(c), PART_GABLE);
            }
            if pieces.iter().any(shed) {
                assert!(pieces.iter().any(|c| tank(c) && c.at[1] > 6.0 + 2.2));
            }
        }
        // None elsewhere, none in the period; Japan's blocks keep their box tanks.
        for set in [&now, &then] {
            for region in [Region::EastAsia, Region::Temperate, Region::Tropical] {
                for kind in [Kind::Terrace, Kind::Block, Kind::House] {
                    assert_eq!(count(set, region, kind, &tank), 0, "{region:?} {kind:?}");
                }
            }
        }
        for kind in [Kind::Terrace, Kind::Block, Kind::Tower, Kind::House] {
            assert_eq!(count(&then, Region::Taiwan, kind, &tank), 0);
            assert_eq!(count(&then, Region::Taiwan, kind, &shed), 0);
        }
    }

    #[test]
    fn todays_clutter_is_as_it_was() {
        // Every region's as it was before Taiwan's own (2026-10-29).
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for (_, _, pieces) in sample(crate::buildings::Period::Today)
            .into_iter()
            .filter(|(region, _, _)| *region != Region::Taiwan)
        {
            for b in bytemuck::cast_slice::<Clutter, u8>(&pieces) {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        assert_eq!(h, 4_835_397_365_008_730_658);
    }

    #[test]
    fn flat_roofs_carry_plant_within_them_and_pitched_ones_chimneys() {
        // A 40 by 25 m block of six storeys: a housing, units and vents, all on the roof inside the walls.
        let block = building(rect(0.0, 0.0, 40.0, 25.0), Kind::Block, Roof::Flat, 6);
        let mut out = Vec::new();
        roof_clutter(&block, Region::Temperate, 23.0, &mut out);
        assert!(out.len() >= 5, "{}", out.len());
        for c in &out {
            assert!(geometry::contains(&block.ring, [c.at[0], c.at[2]]));
            assert_eq!(c.at[1], 23.0);
            assert!(c.size.iter().all(|s| *s > 0.2 && *s < 7.0));
        }
        assert!(out.iter().any(|c| c.size[1] > 2.0), "a stair housing");
        // Deterministic.
        let mut again = Vec::new();
        roof_clutter(&block, Region::Temperate, 23.0, &mut again);
        assert_eq!(out, again);
        // Japanese blocks often carry a tank on a stand, New York ones a timber tower.
        let towers = (0..40)
            .filter(|k| {
                let mut b = building(
                    rect(100.0 * *k as f32, 0.0, 30.0, 20.0),
                    Kind::Block,
                    Roof::Flat,
                    8,
                );
                b.seed = *k as u8;
                let mut out = Vec::new();
                roof_clutter(&b, Region::American, 25.0, &mut out);
                out.iter().any(|c| c.shape == Shape::Tower as u32)
            })
            .count();
        assert!((8..32).contains(&towers), "{towers}");
        let tanks = (0..40)
            .filter(|k| {
                let mut b = building(
                    rect(100.0 * *k as f32, 0.0, 30.0, 20.0),
                    Kind::Block,
                    Roof::Flat,
                    5,
                );
                b.seed = *k as u8;
                let mut out = Vec::new();
                roof_clutter(&b, Region::EastAsia, 16.0, &mut out);
                out.iter().filter(|c| c.at[1] > 16.5).count() == 1
            })
            .count();
        assert!((8..34).contains(&tanks), "{tanks}");
        // A British terrace's stacks along its ridge; no chimney on a Japanese house.
        let terrace = building(
            rect(0.0, 0.0, 30.0, 8.0),
            Kind::Terrace,
            Roof::Gabled { rise: 3.0 },
            2,
        );
        let mut stacks = Vec::new();
        for seed in 0..10u8 {
            let mut t = terrace.clone();
            t.seed = seed;
            roof_clutter(&t, Region::Temperate, 11.0, &mut stacks);
        }
        assert!(stacks.len() >= 20, "{}", stacks.len());
        for c in &stacks {
            assert!((c.at[2] - 4.0).abs() < 0.01, "on the ridge");
            assert!(c.at[1] + c.size[1] > 14.0, "over the ridge");
        }
        let mut none = Vec::new();
        for seed in 0..20u8 {
            let mut house = building(
                rect(0.0, 0.0, 9.0, 7.0),
                Kind::House,
                Roof::Hipped { rise: 2.0 },
                2,
            );
            house.seed = seed;
            roof_clutter(&house, Region::EastAsia, 11.0, &mut none);
        }
        assert!(none.is_empty());
    }
}
