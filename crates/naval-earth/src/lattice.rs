//! The Earth's lattices (globe stage 2, step C: C9, C10, C13): fixed on the Earth, not on a scenario, so a weather
//! system, a wave or a cloud is where it is whatever chart a run is laid on. Inert: nothing calls them yet (ENV-SIM).
//! Each takes an Earth-fixed unit vector (`WorldChart::earth_fixed`: x toward 0° N 0° E, y toward 0° N 90° E, z
//! north) or a [`Geo`].
//!
//! - **The cube-sphere** ([`CubeCell`], C9): the 216 cells of an equiangular cube-sphere. A cube about the Earth's
//!   centre, its faces square to the axes, each face cut 6 × 6 by planes through the centre 15° apart, so a cell's
//!   four sides are great-circle arcs. Every point of the sphere is in one cell ([`CubeCell::at`], half-open); a cell
//!   has its centre (the middle of its two angles), its corners and its circumradius (the farthest corner from its
//!   centre, 1,140 to 1,258 km). [`cells_within`] lists, by id, every cell whose centre lies within a distance plus
//!   that cell's circumradius of a place: every cell a weather system born in it and reaching that far can come from.
//! - **Sea anchors** ([`SeaAnchor`], C10): a place every whole degree of latitude and longitude. A place reads the
//!   four at the corners of its lattice cell ([`SeaAnchor::about`]); their direction bins are offset by four colours
//!   ([`SeaAnchor::colour`]), all four different about every place.
//! - **Cloud anchors** (C13): the cube-sphere cells' centres. A place reads every anchor whose cell, grown by a band,
//!   holds it ([`cloud_anchors`]): one inside a cell, two along an edge, three at a cube corner, four where four cells
//!   meet.
//!
//! The cells' boundaries are tested against exact cuts (`tan 15°`, `tan 30°`), so a point's cell is the same on every
//! target; the centres, corners and circumradii are made once ([`std::sync::OnceLock`]), with one `atan2` for each
//! corner's distance.
use super::sphere::{R, distance, within};
use super::squares::{Geo, angle};
use crate::vector::{cross, dot, scale};
use std::sync::OnceLock;

type V3 = [f64; 3];

const TAN15: f64 = 0.2679491924311227;
const TAN30: f64 = 0.5773502691896257;
const TAN7_5: f64 = 0.13165249758739586;
const TAN22_5: f64 = 0.41421356237309503;
const TAN37_5: f64 = 0.7673269879789604;
/// A face's cuts, as tangents of the angle from its middle: −45° to 45° every 15°.
const CUTS: [f64; 7] = [-1.0, -TAN30, -TAN15, 0.0, TAN15, TAN30, 1.0];
/// The middles of a face's six bands.
const MIDDLES: [f64; 6] = [-TAN37_5, -TAN22_5, -TAN7_5, TAN7_5, TAN22_5, TAN37_5];
/// Each face's outward axis `N` and its two in-face axes `U` and `V`: +x (0° N 0° E), +y (0° N 90° E), −x (0° N
/// 180°), −y (0° N 90° W), +z (the north pole), −z (the south pole). A point of face `f` is `N + a·U + b·V` on the
/// cube, `a = (v·U) / (v·N)` and `b = (v·V) / (v·N)` in [−1, 1].
const FACES: [[V3; 3]; 6] = [
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[-1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
    [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]],
    [[0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
];

/// A cell of the equiangular cube-sphere: its id is `face · 36 + i · 6 + j`, `i` the band along the face's `U` and
/// `j` along its `V`, each 0 to 5. Ordered by id: the canonical order of [`cells_within`] and [`cloud_anchors`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CubeCell(u8);

/// A cell's centre, corners (in turn round it) and circumradius, metres.
struct Shape {
    centre: V3,
    corners: [V3; 4],
    circumradius: f64,
}

fn shapes() -> &'static [Shape; CubeCell::COUNT] {
    static SHAPES: OnceLock<[Shape; CubeCell::COUNT]> = OnceLock::new();
    SHAPES.get_or_init(|| {
        std::array::from_fn(|id| {
            let cell = CubeCell(id as u8);
            let [n, u, v] = FACES[cell.face()];
            let on = |a: f64, b: f64| {
                unit([
                    n[0] + a * u[0] + b * v[0],
                    n[1] + a * u[1] + b * v[1],
                    n[2] + a * u[2] + b * v[2],
                ])
            };
            let (i, j) = (cell.i(), cell.j());
            let centre = on(MIDDLES[i], MIDDLES[j]);
            let corners = [
                on(CUTS[i], CUTS[j]),
                on(CUTS[i + 1], CUTS[j]),
                on(CUTS[i + 1], CUTS[j + 1]),
                on(CUTS[i], CUTS[j + 1]),
            ];
            let circumradius = corners
                .iter()
                .map(|k| distance(centre, *k))
                .fold(0.0, f64::max);
            Shape {
                centre,
                corners,
                circumradius,
            }
        })
    })
}

fn unit(v: V3) -> V3 {
    scale(v, 1.0 / dot(v, v).sqrt())
}

/// The band of `a` (a face coordinate, −1 to 1): half-open between the cuts, the last band closed at 1.
#[inline]
fn band(a: f64) -> usize {
    CUTS[1..6].iter().filter(|c| a >= **c).count()
}

impl CubeCell {
    /// Cells on the sphere.
    pub const COUNT: usize = 216;

    /// The cell `i`, `j` of face `face` (each `i`, `j` 0 to 5, `face` 0 to 5).
    #[inline]
    pub fn new(face: usize, i: usize, j: usize) -> Self {
        assert!(face < 6 && i < 6 && j < 6, "no cell {face} {i} {j}");
        Self((face * 36 + i * 6 + j) as u8)
    }
    /// The cell with id `id`, if there is one.
    #[inline]
    pub fn from_id(id: usize) -> Option<Self> {
        (id < Self::COUNT).then_some(Self(id as u8))
    }
    /// Every cell, by id.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).map(|id| Self(id as u8))
    }
    #[inline]
    pub fn id(self) -> usize {
        usize::from(self.0)
    }
    #[inline]
    pub fn face(self) -> usize {
        self.id() / 36
    }
    #[inline]
    pub fn i(self) -> usize {
        self.id() / 6 % 6
    }
    #[inline]
    pub fn j(self) -> usize {
        self.id() % 6
    }

    /// The cell holding the point the Earth-fixed vector `v` names: the face of `v`'s largest component (on a tie,
    /// z before x before y), then the bands of `a` and `b`. Every point is in one cell. No transcendental call.
    #[inline]
    pub fn at(v: V3) -> Self {
        let [ax, ay, az] = v.map(f64::abs);
        let face = if az >= ax && az >= ay {
            if v[2] >= 0.0 { 4 } else { 5 }
        } else if ax >= ay {
            if v[0] >= 0.0 { 0 } else { 2 }
        } else if v[1] >= 0.0 {
            1
        } else {
            3
        };
        let [n, u, w] = FACES[face];
        let up = dot(v, n);
        Self::new(face, band(dot(v, u) / up), band(dot(v, w) / up))
    }
    /// Its centre: the middle of its two angles, an Earth-fixed unit vector. Its cloud anchor (C13).
    #[inline]
    pub fn centre(self) -> V3 {
        shapes()[self.id()].centre
    }
    /// Its four corners, in turn round it.
    #[inline]
    pub fn corners(self) -> [V3; 4] {
        shapes()[self.id()].corners
    }
    /// The distance from its centre to its farthest corner, metres: every point of it lies within this of its centre.
    #[inline]
    pub fn circumradius_m(self) -> f64 {
        shapes()[self.id()].circumradius
    }
    /// Whether the point `v` lies in it or on its boundary: the closed cell, between its four great circles.
    #[inline]
    pub fn contains(self, v: V3) -> bool {
        let [n, u, w] = FACES[self.face()];
        let up = dot(v, n);
        let (a, b) = (dot(v, u), dot(v, w));
        let (i, j) = (self.i(), self.j());
        up > 0.0
            && a >= CUTS[i] * up
            && a <= CUTS[i + 1] * up
            && b >= CUTS[j] * up
            && b <= CUTS[j + 1] * up
    }
    /// How far the point `v` lies outside it, metres along the sphere: 0 in it, else to the nearest of its four
    /// sides.
    pub fn outside_m(self, v: V3) -> f64 {
        if self.contains(v) {
            return 0.0;
        }
        let k = self.corners();
        (0..4)
            .map(|e| to_arc(v, k[e], k[(e + 1) % 4]))
            .fold(f64::INFINITY, f64::min)
            * R
    }
}

/// The angle from `p` to the great-circle arc from `a` to `b` (shorter than a half circle), radians: across to it when
/// the foot of the perpendicular lies on it, else to its nearer end.
fn to_arc(p: V3, a: V3, b: V3) -> f64 {
    let n = unit(cross(a, b));
    let across = dot(p, n);
    let foot = [
        p[0] - across * n[0],
        p[1] - across * n[1],
        p[2] - across * n[2],
    ];
    if dot(foot, foot) > 0.0 && dot(cross(a, foot), n) >= 0.0 && dot(cross(foot, b), n) >= 0.0 {
        across.abs().atan2(dot(foot, foot).sqrt())
    } else {
        angle(p, a).min(angle(p, b))
    }
}

/// Every cell whose centre lies within `distance_m` plus that cell's circumradius of the point `v`, by id (C9: with
/// `R_MAX + V_MAX · L_MAX`, 4,456 km, the cells whose systems can reach `v`). A comparison of squared chords each.
pub fn cells_within(v: V3, distance_m: f64) -> Vec<CubeCell> {
    shapes()
        .iter()
        .enumerate()
        .filter(|(_, s)| within(v, s.centre, distance_m + s.circumradius))
        .map(|(id, _)| CubeCell(id as u8))
        .collect()
}

/// How far a cloud anchor's cell is grown, metres (C13).
pub const CLOUD_BAND_M: f64 = 200_000.0;

/// The cloud anchors the point `v` reads (C13): every cell whose region, grown by `band_m`, holds `v`, with how far
/// `v` lies outside it (0 inside), by id. Each anchor is its cell's [`CubeCell::centre`].
pub fn cloud_anchors(v: V3, band_m: f64) -> Vec<(CubeCell, f64)> {
    shapes()
        .iter()
        .enumerate()
        .filter(|(_, s)| within(v, s.centre, band_m + s.circumradius))
        .map(|(id, _)| CubeCell(id as u8))
        .map(|cell| (cell, cell.outside_m(v)))
        .filter(|(_, d)| *d <= band_m)
        .collect()
}

/// A sea anchor (C10): the place at whole degrees of latitude (−90 to 90; ±90 is the pole) and longitude (−180 to
/// 179). Ordered by latitude, then longitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SeaAnchor {
    pub lat: i32,
    pub lon: i32,
}

impl SeaAnchor {
    /// Whether it names an anchor.
    #[inline]
    pub fn valid(self) -> bool {
        (-90..=90).contains(&self.lat) && (-180..=179).contains(&self.lon)
    }
    /// The four anchors at the corners of the lattice cell holding `g`, `[lat0, lat0 + 1] × [lon0, lon0 + 1]`, in
    /// the order south-west, south-east, north-west, north-east (east of 179 is −180), and where `g` lies in it:
    /// `[east, north]`, each in [0, 1]. A place on the northernmost parallel a place may have (89.5°) reads the pole's
    /// anchors at its north corners.
    pub fn about(g: &Geo) -> ([SeaAnchor; 4], [f64; 2]) {
        let lat0 = g.lat().floor().clamp(-90.0, 89.0);
        let lon0 = g.lon().floor();
        let east_of = (g.lon() - lon0).clamp(0.0, 1.0);
        let north_of = (g.lat() - lat0).clamp(0.0, 1.0);
        let (lat, lon) = (lat0 as i32, column(lon0 as i32));
        let lon1 = column(lon + 1);
        (
            [
                SeaAnchor { lat, lon },
                SeaAnchor { lat, lon: lon1 },
                SeaAnchor { lat: lat + 1, lon },
                SeaAnchor {
                    lat: lat + 1,
                    lon: lon1,
                },
            ],
            [east_of, north_of],
        )
    }
    /// Its place on the Earth (the pole for ±90).
    #[inline]
    pub fn geo(self) -> Geo {
        Geo::degrees(f64::from(self.lat), f64::from(self.lon))
    }
    /// Its colour, 0 to 3, from the parity of its latitude and of its longitude: the four anchors about a place have
    /// four (360 longitudes are an even count, so ±180° keeps the parity's turn).
    #[inline]
    pub fn colour(self) -> u8 {
        (self.lat.rem_euclid(2) * 2 + self.lon.rem_euclid(2)) as u8
    }
    /// How far its direction bins are turned, degrees: 0, 1.25, 2.5 or 3.75 by its colour (C10), so the four anchors a
    /// place reads never share a mode's direction.
    #[inline]
    pub fn direction_offset_deg(self) -> f64 {
        1.25 * f64::from(self.colour())
    }
}

/// A column's longitude, −180 to 179.
#[inline]
fn column(lon: i32) -> i32 {
    (lon + 180).rem_euclid(360) - 180
}
