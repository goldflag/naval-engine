//! The sphere's mathematics: what the types of `earth` (`WorldPos`, `Calc`, `Carry`, `WorldChart`, `LaunchFrame`)
//! compute with, over plain values, held to a 60-digit reference (`tests/earth_sphere.rs`,
//! `tests/fixtures/earth-sphere-table.json`, written by `scripts/sim/earth-sphere-table.py`) to 1e-6 m and 1e-11 rad
//! out to 2,000 km; step C's long-range forms to 1 km off antipodal and its chart about any place
//! ([`Chart::about_place`]) to 19,000 km. Code comments cite the design's decisions by number (docs/earth.md).
//!
//! **Values.** A [`Place`] is a unit vector from the Earth's centre in the scenario's meridian frame (D1): the
//! Earth's axes turned about the pole so that the scenario origin's meridian is the x–z plane. The origin is
//! `(cos φ0, 0, sin φ0)`, the north pole `(0, 0, 1)`, and `+y` is east at the origin. A height is metres above the
//! sphere of radius [`R`] (D2). A tangent vector is `[east, up, south]` at the place that holds it (D4), a heading
//! radians clockwise from true north there (D3), a chart position `[x east, z south]` metres (D7).
//!
//! **What stands for what.**
//!
//! | The types (`earth::`) | Here |
//! | --- | --- |
//! | `calc(anchor, …)` | [`Frame::at`]: the rigid frame at the anchor (D5) |
//! | `Calc::coords`, `coords3`, `delta`, `delta3` | [`Frame::coords2`], [`Frame::coords`], [`Frame::delta`], [`Frame::delta3`] |
//! | `Calc::position`, `displace`, `displace3` | [`Frame::position`], [`Frame::displace`], [`Frame::point`], [`Frame::displace3`] |
//! | `Calc::tangent3`, `to_tangent3`, `tangent2`, `to_tangent2`, `heading`, `to_heading` | [`Frame::tangent`], [`Frame::to_tangent`], [`Frame::tangent2`], [`Frame::to_tangent2`], [`Frame::heading`], [`Frame::to_heading`] |
//! | `Calc::heading_to`, `heading_to_neg` | [`bearing`], [`bearing_neg`] (from the anchor: [`Frame::bearing`]) |
//! | `Calc::advance`, `step_aloft` | [`advance`], [`step_aloft`] |
//! | `Carry::between`, `heading`, `tangent2`, `tangent3` | [`Convergence::between`], [`Convergence::heading`], [`Convergence::tangent2`], [`Convergence::tangent3`]; the rotation itself, [`transport`] |
//! | `WorldPos::distance`, `distance_sq`, `within` | [`distance`], [`chord_sq`], [`within`] (with [`chord_of`] and [`within_chord`] for a radius held) |
//! | `WorldPoint::distance` | [`slant_range`] |
//! | long constructions (`WorldPos::toward`, a flight's step) | [`destination`], [`toward`] (D6) |
//! | step C, to the antipode guard (nothing calls them yet) | [`bearing_long`], [`toward_long`], [`transport_long`], [`Convergence::between_long`], [`is_antipodal`] |
//! | `WorldChart::from_lat_lon`, `lat_lon`, `grid_xz`, `resolve_xz`, `range_from_origin`, `round_m_on` | [`Chart::from_lat_lon`], [`Chart::lat_lon`], [`Chart::forward`], [`Chart::inverse`], [`Chart::range`], [`Chart::round_m`] |
//! | `Calc::chart_vec_on`, `to_chart_vec_on`, `chart_normal` | [`Chart::displacement`] (J⁻¹), [`Chart::to_displacement`] (J), [`Chart::gradient`] (Jᵀ) |
//! | `Calc::chart_unit_normal` | [`Chart::gradient`], made a unit vector again by its caller |
//! | `WorldChart::course_to`, `course_along`, `grid_heading`, `resolve_heading`, `cover` | the same names on [`Chart`] |
//! | `WorldChart::about_place`, `cover_at`, `earth_fixed`, `from_earth_fixed` (step C) | the same names on [`Chart`] |
//! | `LaunchFrame::of`, `place`, `to_world`, `velocity`, `dir_to_world`, `height` | [`Frame::at`], [`Frame::coords`], [`Frame::point`], [`Frame::tangent`], [`Frame::to_tangent`], [`height_above_sea`] (D8) |
//! | `LaunchFrame::attitude`, `attitude_to_world` | a body's axes through [`Frame::tangent`] and [`Frame::to_tangent`], its heading, pitch and bank read from them |
//! | a place made or read | [`saturate`] (D9), [`check`] (D1) |
//!
//! **Arithmetic.** What runs every tick uses `+ − × ÷ √` only (the platform's transcendental functions carry no promise
//! across targets): the frame, coordinates and their inverses, a step along the surface, a
//! carry, a range test, and the chart inside 1,000 km of its origin (the reach). Angles are the exception, and each
//! function says which calls it makes: an `atan2` gives a bearing or a heading, a `sin` and a `cos` take one. Series
//! stand in for `asin`, `sin` and `cos` on arcs up to 1,000 km and for `ln` on a rise to 49 km, and the platform's
//! functions take over beyond; the tests hold each crossover continuous to the budget.
//!
//! **Singular cases** (D5, D6, D9): coincident places give bearing 0 ([`bearing_neg`]: π), no carry and no move;
//! places more than a quarter of the girth apart ([`is_far`]) give bearing 0, no carry and no move toward; a frame
//! position more than 0.5 R from the anchor is the place at 0.5 R on its bearing ([`Frame::is_near`]); a place made
//! within half a degree of a pole is put on the 89.5° parallel of its meridian.
//!
//! **What is taken as given.** A place is a unit vector to [`UNIT_TOLERANCE`] ([`check`]) and is used as it is
//! stored, never normalized on the way in: the budgets hold for such places, and every place made here is unit to
//! rounding. Lengths, heights and coordinates are metres below 1e150 (their squares are finite). For those, no
//! function returns a NaN.
use crate::geo::{LatLon, wrap_lon};
use crate::vector::Vec3;
use crate::vector::{add, cross, dot, scale, sub};
use std::f64::consts::PI;
use std::fmt;

/// A place on the Earth: the unit vector from its centre, in the scenario's meridian frame (D1).
pub type Place = [f64; 3];

/// The sphere's radius, metres (D2: the chart's, the bake's and `geo.ts`'s).
pub const R: f64 = crate::EARTH_RADIUS_M;

/// A count for the census (`earth/census.rs`, cargo feature `earth-census`); nothing in any other build.
macro_rules! census {
    ($($body:tt)*) => {
        #[cfg(feature = "earth-census")]
        {
            #[allow(unused_imports)]
            use super::census::{Op, count, participant};
            $($body)*
        }
    };
}
const INV_R: f64 = 1.0 / R;
/// Half the Earth's girth, metres: the greatest distance there is.
pub const HALF_GIRTH_M: f64 = PI * R;

/// `sin 89.5°`: no place has a `|z|` above it (D9).
pub const POLE_LIMIT_Z: f64 = 0.9999619230641713;
/// `cos 89.5°`: what `√(x² + y²)` is on the limiting parallel.
pub const POLE_LIMIT_C: f64 = 0.008726535498373935;
/// A stored place is a unit vector to this much: `| |v|² − 1 | ≤ 1e-14`, which is 32 nm of radius. D1 wrote 1e-12,
/// but that admits two vectors naming one direction 3 µm apart, and a frame at one reads the other 3 µm high: over
/// the backend's budget of 1e-6 m before any arithmetic. Every place this module makes is unit to rounding (under
/// 5e-16), twenty times inside this, so a place written and read back passes; nothing else is ever read as one.
pub const UNIT_TOLERANCE: f64 = 1e-14;
/// The chord, metres, up to which a great-circle arc comes from D6's series.
pub const SERIES_CHORD_M: f64 = 1_000_000.0;
/// The arc, metres, up to which `sin` and `cos` of a step come from their series.
pub const SERIES_ARC_M: f64 = 1_000_000.0;
/// A frame position is defined to this fraction of [`R`] from the anchor (D5: 3,186 km).
pub const NEAR_SIDE: f64 = 0.5;
/// The chart's greatest stretch to 1,200 km from its origin, rounded up (`σ / sin σ` is 1.00594 there).
pub const COVER: f64 = 1.006;
/// How far from the chart's origin a disc may reach and keep [`COVER`] ([`Chart::cover_at`]).
pub const COVER_RANGE_M: f64 = 1_200_000.0;
/// How near its origin a place may be and a chart laid about it still be the chart itself ([`Chart::about_place`]): a
/// micrometre, the chart's round trip allowance (`WorldChart::ROUND_TRIP_M`), a thousand times a last bit's 1.4 nm.
pub const SAME_ORIGIN_M: f64 = 1e-6;
/// `(SAME_ORIGIN_M / R)²`.
const SAME_ORIGIN_Q: f64 = (SAME_ORIGIN_M / R) * (SAME_ORIGIN_M / R);

/// `(chord / R)²` at [`SERIES_CHORD_M`].
const SERIES_CHORD_Q: f64 = (SERIES_CHORD_M / R) * (SERIES_CHORD_M / R);
/// `θ²` at [`SERIES_ARC_M`].
const SERIES_ARC_W: f64 = (SERIES_ARC_M / R) * (SERIES_ARC_M / R);
/// `sin² γ` up to which a carry's angle comes from the series (γ ≤ 2.9°: a step's convergence).
const SERIES_TURN_T: f64 = 1.0 / 400.0;

// --- The three series ---

/// `asin(x) / x` from `t = x²`: D6's series, `1 + x²/6 + 3x⁴/40 + 15x⁶/336 + 105x⁸/3456 + 945x¹⁰/42240`. The next
/// term is `0.0174 t⁶`: 9.5e-16 of the arc at 1,000 km of chord (`t = 0.0062`), which is 1e-9 m.
#[inline]
fn asin_ratio(t: f64) -> f64 {
    1.0 + t
        * (1.0 / 6.0
            + t * (3.0 / 40.0 + t * (15.0 / 336.0 + t * (105.0 / 3456.0 + t * (945.0 / 42240.0)))))
}

/// `(sin θ / θ, (1 − cos θ) / θ²)` from `w = θ²`. Up to [`SERIES_ARC_M`] of arc both are series (truncated below
/// 4e-20, and `1 − cos θ` loses nothing to cancellation); beyond, one `sin` and one `cos`.
#[inline]
fn sinc_versc(w: f64) -> (f64, f64) {
    if w <= SERIES_ARC_W {
        let s = 1.0
            + w * (-1.0 / 6.0
                + w * (1.0 / 120.0
                    + w * (-1.0 / 5040.0 + w * (1.0 / 362_880.0 + w * (-1.0 / 39_916_800.0)))));
        let c = 0.5
            + w * (-1.0 / 24.0
                + w * (1.0 / 720.0
                    + w * (-1.0 / 40_320.0
                        + w * (1.0 / 3_628_800.0 + w * (-1.0 / 479_001_600.0)))));
        (s, c)
    } else {
        let theta = w.sqrt();
        let (sin, cos) = theta.sin_cos();
        (sin / theta, (1.0 - cos) / w)
    }
}

// --- A place is valid by construction (D1, D9) ---

/// Why a stored place is none (D1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invalid {
    /// A component is infinite or a NaN.
    NotFinite,
    /// Its length is not 1 to [`UNIT_TOLERANCE`].
    NotUnit,
    /// It is within half a degree of a pole (D9).
    Polar,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "a place has a component that is not finite",
            Self::NotUnit => "a place is not a unit vector",
            Self::Polar => "a place is within half a degree of a pole",
        })
    }
}

/// Whether `p` is a place: finite, a unit vector to [`UNIT_TOLERANCE`] and no nearer a pole than D9 allows. What
/// deserialization checks (D1). It never renormalizes: the bits read are the bits kept, or the read fails.
/// No transcendental call.
#[inline]
pub fn check(p: Place) -> Result<(), Invalid> {
    if !(p[0].is_finite() && p[1].is_finite() && p[2].is_finite()) {
        return Err(Invalid::NotFinite);
    }
    if (dot(p, p) - 1.0).abs() > UNIT_TOLERANCE {
        return Err(Invalid::NotUnit);
    }
    if !within_pole_limit(p) {
        return Err(Invalid::Polar);
    }
    Ok(())
}

/// [`check`] as a yes or no.
#[inline]
pub fn is_valid(p: Place) -> bool {
    check(p).is_ok()
}

/// D9's limit: `|p.z| ≤ sin 89.5°`. East and north do not exist at a pole, so no place is nearer one.
#[inline]
pub fn within_pole_limit(p: Place) -> bool {
    p[2].abs() <= POLE_LIMIT_Z
}

/// `p`, or the place on the 89.5° parallel of its meridian when `p` is nearer a pole than that (D9: every function
/// here that makes a place ends with it, so no frame is ever built inside the cap). A pole itself, which has no
/// meridian, goes to the origin's. No transcendental call; one square root and one division inside the cap only.
#[inline]
pub fn saturate(p: Place) -> Place {
    if within_pole_limit(p) {
        return p;
    }
    census!(count(Op::PoleSaturated););
    let z = POLE_LIMIT_Z.copysign(p[2]);
    let c = (p[0] * p[0] + p[1] * p[1]).sqrt();
    if c > 0.0 {
        let k = POLE_LIMIT_C / c;
        [p[0] * k, p[1] * k, z]
    } else {
        [POLE_LIMIT_C, 0.0, z]
    }
}

/// A vector made as a unit vector, brought back to length 1: one Newton step for `1 / |p|` about 1, which is exact
/// to `3ε²/8` for a length off by `ε` (and every caller's is off by rounding, or by its input's [`UNIT_TOLERANCE`]).
/// Without it a body's place would wander off the sphere over millions of steps.
#[inline]
fn renormalized(p: Vec3) -> Vec3 {
    scale(p, 1.5 - 0.5 * dot(p, p))
}

/// A made place: unit to rounding and outside the polar caps.
#[inline]
fn made(p: Vec3) -> Place {
    saturate(renormalized(p))
}

#[inline]
fn positive(x: f64) -> bool {
    x > 0.0
}

// --- The frame at an anchor (D5), which is also a launch frame (D8) ---

/// The rigid Cartesian frame at an anchor (D5): its origin the anchor at sea level, x east, y up and z south there.
/// A point's coordinates in it are an exact rigid transform of its Earth-centred position, so slant ranges, sight
/// lines and vectors between participants are exact; the horizontal pair `[x, z]` is an orthographic view. A
/// projectile's launch frame is the same frame (D8).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    up: Place,
    east: Vec3,
    north: Vec3,
}

impl Frame {
    /// The frame at `a` (D5): with `c = √(a.x² + a.y²)`, `E = (−a.y, a.x, 0) / c` and
    /// `N = (−a.z·a.x, −a.z·a.y, c²) / c`. One square root and one division; no transcendental call. A vector nearer
    /// a pole than a place may be is taken as [`saturate`] puts it.
    #[inline]
    pub fn at(a: Place) -> Self {
        census!(count(Op::Frame););
        let a = saturate(a);
        let c = (a[0] * a[0] + a[1] * a[1]).sqrt();
        let k = 1.0 / c;
        let east = [-a[1] * k, a[0] * k, 0.0];
        let north = [-a[2] * east[1], a[2] * east[0], c];
        Self { up: a, east, north }
    }
    /// The anchor: the frame's up.
    #[inline]
    pub fn anchor(&self) -> Place {
        self.up
    }
    /// The unit vector east at the anchor, Earth-centred.
    #[inline]
    pub fn east(&self) -> Vec3 {
        self.east
    }
    /// The unit vector north at the anchor, Earth-centred.
    #[inline]
    pub fn north(&self) -> Vec3 {
        self.north
    }

    /// An Earth-centred vector on the frame's axes, `[east, up, south]`. An exact zero comes out `+0`.
    #[inline]
    pub fn components(&self, w: Vec3) -> Vec3 {
        [
            dot(w, self.east) + 0.0,
            dot(w, self.up) + 0.0,
            0.0 - dot(w, self.north),
        ]
    }
    /// A vector of the frame `[east, up, south]`, Earth-centred.
    #[inline]
    pub fn vector(&self, v: Vec3) -> Vec3 {
        let (a, e, n) = (self.up, self.east, self.north);
        [
            v[0] * e[0] + v[1] * a[0] - v[2] * n[0],
            v[0] * e[1] + v[1] * a[1] - v[2] * n[1],
            v[1] * a[2] - v[2] * n[2],
        ]
    }

    // --- In: a participant's coordinates ---

    /// The point at place `p` and height `h`, in the frame: `((R + h)·p − R·a)` on `E`, `a` and `−N` (D5), taken as
    /// `R·(p − a) + h·p` so that a neighbour's coordinates come from a difference of nearby unit vectors and lose
    /// nothing. `[x east, y up, z south]` metres; `y` is the height above the frame's level plane, not above the sea
    /// ([`height_above_sea`]). Also `LaunchFrame::place` (D8). No transcendental call.
    #[inline]
    pub fn coords(&self, p: Place, h: f64) -> Vec3 {
        census!(participant(self.up, p););
        let a = self.up;
        self.components([
            R * (p[0] - a[0]) + h * p[0],
            R * (p[1] - a[1]) + h * p[1],
            R * (p[2] - a[2]) + h * p[2],
        ])
    }
    /// The frame's `[x, z]` of the place `p` at sea level: the orthographic view a `V2` is (D5). The anchor itself is
    /// `[+0, +0]`. No transcendental call.
    #[inline]
    pub fn coords2(&self, p: Place) -> [f64; 2] {
        self.delta(self.up, p)
    }
    /// `to − from` in the frame's `[x, z]`, both at sea level, from the difference of the two unit vectors. Coincident
    /// places give `[+0, +0]`. No transcendental call.
    #[inline]
    pub fn delta(&self, from: Place, to: Place) -> [f64; 2] {
        census!(for p in [from, to] {
            if p != self.up {
                participant(self.up, p);
            }
        });
        let d = sub(to, from);
        [R * dot(d, self.east) + 0.0, 0.0 - R * dot(d, self.north)]
    }
    /// `to − from` in the frame, each a place and a height: the straight vector between two points, exact (their
    /// slant range is its length). No transcendental call.
    #[inline]
    pub fn delta3(&self, from: Place, from_h: f64, to: Place, to_h: f64) -> Vec3 {
        census!(for p in [from, to] {
            if p != self.up {
                participant(self.up, p);
            }
        });
        self.components([
            R * (to[0] - from[0]) + (to_h * to[0] - from_h * from[0]),
            R * (to[1] - from[1]) + (to_h * to[1] - from_h * from[1]),
            R * (to[2] - from[2]) + (to_h * to[2] - from_h * from[2]),
        ])
    }

    // --- Out: the inverses ---

    /// Whether the frame position `[x, z]` is on the anchor's near side: within [`NEAR_SIDE`]` · R` of it (D5). What
    /// the census counts: [`Frame::position`] and [`Frame::displace`] saturate beyond.
    #[inline]
    pub fn is_near(v: [f64; 2]) -> bool {
        let limit = NEAR_SIDE * R;
        v[0] * v[0] + v[1] * v[1] <= limit * limit
    }
    /// The sea-level place whose frame position is `[x, z]`: the inverse of [`Frame::coords2`] (D5), defined on the
    /// near side. With `ρ = hypot(x, z) / R` it is `a·√(1 − ρ²) + (x·E − z·N) / R`, taken as `a` plus a small vector
    /// so that a nearby place loses nothing. Beyond `ρ = 0.5` it is the place at `ρ = 0.5` on the same bearing. Two
    /// divisions and a square root; no transcendental call.
    #[inline]
    pub fn position(&self, v: [f64; 2]) -> Place {
        let (mut ue, mut un) = (v[0] * INV_R, -v[1] * INV_R);
        let mut rho2 = ue * ue + un * un;
        if rho2 > NEAR_SIDE * NEAR_SIDE {
            census!(count(Op::PositionSaturated););
            let k = NEAR_SIDE / rho2.sqrt();
            ue *= k;
            un *= k;
            rho2 = NEAR_SIDE * NEAR_SIDE;
        }
        // √(1 − ρ²) − 1, with no cancellation.
        let w1 = -rho2 / (1.0 + (1.0 - rho2).sqrt());
        let (a, e, n) = (self.up, self.east, self.north);
        made([
            a[0] + (ue * e[0] + un * n[0] + w1 * a[0]),
            a[1] + (ue * e[1] + un * n[1] + w1 * a[1]),
            a[2] + (un * n[2] + w1 * a[2]),
        ])
    }
    /// The place `p` moved by `d` in the frame's `[x, z]`: the place whose frame position is `coords2(p) + d`, at sea
    /// level (D5). `displace(p, delta(p, q))` is `q` to rounding (3 nm) for `q` on the near side. No step is no
    /// move: a `d` of exactly zero returns `p` itself. A body aloft steps with [`step_aloft`], a body moving for a
    /// time with [`advance`]. No transcendental call.
    #[inline]
    pub fn displace(&self, p: Place, d: [f64; 2]) -> Place {
        if d[0] == 0.0 && d[1] == 0.0 {
            return p;
        }
        let c = self.coords2(p);
        self.position([c[0] + d[0], c[1] + d[1]])
    }
    /// The place and the height above the sea of the frame point `v = [x, y, z]`: the inverse of [`Frame::coords`],
    /// exact for any point (a rigid transform has no far side). Also `LaunchFrame::to_world` (D8). One square root
    /// and three divisions; no transcendental call.
    #[inline]
    pub fn point(&self, v: Vec3) -> (Place, f64) {
        let [x, y, z] = v;
        let rho2 = x * x + z * z;
        let ry = R + y;
        let len = (ry * ry + rho2).sqrt();
        let (a, e, n) = (self.up, self.east, self.north);
        let p = if positive(ry) {
            // P / |P| as a plus a small vector: (R + y) − |P| = −ρ² / ((R + y) + |P|).
            let (k, s) = (1.0 / len, -rho2 / (ry + len));
            [
                a[0] + (x * e[0] - z * n[0] + s * a[0]) * k,
                a[1] + (x * e[1] - z * n[1] + s * a[1]) * k,
                a[2] + (s * a[2] - z * n[2]) * k,
            ]
        } else if positive(len) {
            // At or below the level of the Earth's centre: no place is near, and none is lost.
            let k = 1.0 / len;
            [
                (ry * a[0] + x * e[0] - z * n[0]) * k,
                (ry * a[1] + x * e[1] - z * n[1]) * k,
                (ry * a[2] - z * n[2]) * k,
            ]
        } else {
            a
        };
        (made(p), height_above_sea(v))
    }
    /// The point at `p` and `h` moved by the frame vector `d`: where a straight line of the frame reaches (a sample
    /// between two points, an offset about a point). It leaves the sea as the frame's plane does: a body moving for
    /// a time is [`advance`]. No transcendental call.
    #[inline]
    pub fn displace3(&self, p: Place, h: f64, d: Vec3) -> (Place, f64) {
        self.point(add(self.coords(p, h), d))
    }

    // --- Tangents held at another place ---

    /// The tangent vector `v = [east, up, south]` held at `at`, as a vector of this frame: the same Earth-centred
    /// vector on this frame's axes (D5: rigid, so a level velocity 150 km off has an up component here, as it truly
    /// has). Held at the anchor it is `v` itself. Also `LaunchFrame::velocity`. One square root and one division (the
    /// frame at `at`); no transcendental call.
    #[inline]
    pub fn tangent(&self, at: Place, v: Vec3) -> Vec3 {
        if at == self.up {
            return v;
        }
        census!(participant(self.up, at););
        self.components(Frame::at(at).vector(v))
    }
    /// The frame vector `v` as a tangent `[east, up, south]` held at `at`: the inverse of [`Frame::tangent`], exact.
    /// Also `LaunchFrame::dir_to_world`. No transcendental call.
    #[inline]
    pub fn to_tangent(&self, at: Place, v: Vec3) -> Vec3 {
        if at == self.up {
            return v;
        }
        census!(participant(self.up, at););
        Frame::at(at).components(self.vector(v))
    }
    /// The level tangent `v = [east, south]` held at `at`, in the frame's `[x, z]`: the orthographic view of it, which
    /// is the rate the frame position of a body at `at` changes at when it moves at `v`. No transcendental call.
    #[inline]
    pub fn tangent2(&self, at: Place, v: [f64; 2]) -> [f64; 2] {
        let w = self.tangent(at, [v[0], 0.0, v[1]]);
        [w[0], w[2]]
    }
    /// The level tangent held at `at` whose view in the frame is `v = [x, z]`: the inverse of [`Frame::tangent2`], so
    /// a body at `at` moving at the result changes its frame position at `v` ([`Frame::displace`]'s differential).
    /// It is not the frame vector `[x, 0, z]` laid on `at`'s plane, which is short by the square of the angle between
    /// the two verticals (5e-4 at 150 km). Defined on the anchor's near side. Beyond it the foreshortening, which
    /// the view has along the line from the anchor only, is held at its value at 0.5 R; what lies across that line
    /// is not foreshortened and is not touched. No transcendental call.
    #[inline]
    pub fn to_tangent2(&self, at: Place, v: [f64; 2]) -> [f64; 2] {
        if at == self.up {
            return v;
        }
        census!(participant(self.up, at););
        let f = Frame::at(at);
        // B takes [east, south] at `at` to the frame's [x, z]: a rotation, but for the foreshortening by `det` (the
        // cosine between the two ups) along the line from the anchor. Its adjugate is det·B⁻¹.
        let b = [
            [dot(f.east, self.east), -dot(f.north, self.east)],
            [-dot(f.east, self.north), dot(f.north, self.north)],
        ];
        let adjugate = |v: [f64; 2]| {
            [
                b[1][1] * v[0] - b[0][1] * v[1],
                b[0][0] * v[1] - b[1][0] * v[0],
            ]
        };
        let det = dot(f.up, self.up);
        if det >= NEAR_SIDE_COS {
            let w = adjugate(v);
            return [w[0] / det, w[1] / det];
        }
        census!(count(Op::FarSideTangent););
        // The far side. Along the line from the anchor (the frame position of `at`) the adjugate is the rotation,
        // and the inverse divides it by the foreshortening held; across it the transpose is the rotation.
        let radial = self.coords2(f.up);
        let length_sq = radial[0] * radial[0] + radial[1] * radial[1];
        let k = if positive(length_sq) {
            (v[0] * radial[0] + v[1] * radial[1]) / length_sq
        } else {
            0.0
        };
        let along = [k * radial[0], k * radial[1]];
        let across = [v[0] - along[0], v[1] - along[1]];
        let along = adjugate(along);
        [
            b[0][0] * across[0] + b[1][0] * across[1] + along[0] / NEAR_SIDE_COS,
            b[0][1] * across[0] + b[1][1] * across[1] + along[1] / NEAR_SIDE_COS,
        ]
    }
    /// The true heading `h` of a body at `at`, as an angle of this frame (clockwise from its −z): the direction its
    /// bow has in the frame's `[x, z]`. At the anchor it is `h` itself, and no call is made; elsewhere one `sin`, one
    /// `cos` and one `atan2`.
    #[inline]
    pub fn heading(&self, at: Place, h: f64) -> f64 {
        if at == self.up {
            return h;
        }
        let (s, c) = h.sin_cos();
        let v = self.tangent2(at, [s, -c]);
        v[0].atan2(-v[1])
    }
    /// An angle of this frame as the true heading of a body at `at`: the inverse of [`Frame::heading`]. At the anchor
    /// it is the angle itself; elsewhere one `sin`, one `cos` and one `atan2`.
    #[inline]
    pub fn to_heading(&self, at: Place, angle: f64) -> f64 {
        if at == self.up {
            return angle;
        }
        let (s, c) = angle.sin_cos();
        let v = self.to_tangent2(at, [s, -c]);
        v[0].atan2(-v[1])
    }

    /// The true bearing from the anchor toward `to` (D5: exact from the anchor, since `(to − a)` on `E` and `N` is
    /// the great circle's own direction). 0 when they coincide or are more than a quarter of the girth apart (D6).
    /// One `atan2`.
    #[inline]
    pub fn bearing(&self, to: Place) -> f64 {
        if to == self.up || is_far(self.up, to) {
            census!(if to != self.up {
                count(Op::FarAnswer);
            });
            return 0.0;
        }
        let d = sub(to, self.up);
        dot(d, self.east).atan2(dot(d, self.north))
    }
}

/// `cos(asin 0.5)`: the cosine between the anchor's vertical and the vertical at the edge of its near side.
const NEAR_SIDE_COS: f64 = 0.8660254037844386;

/// The height above the sea of the frame point `p = [x, y, z]`, whichever place the frame is at:
/// `|R·a + p| − R` (D8), taken as `(y·(2R + y) + x² + z²) / (|R·a + p| + R)` so that the sea's drop, a few metres
/// in 6,371 km, loses nothing. `LaunchFrame::height`. One square root and one division; no transcendental call.
#[inline]
pub fn height_above_sea(p: Vec3) -> f64 {
    let [x, y, z] = p;
    let rho2 = x * x + z * z;
    let ry = R + y;
    (y * (2.0 * R + y) + rho2) / ((ry * ry + rho2).sqrt() + R)
}

// --- Long-range answers from the sphere (D6) ---

/// Whether two places are more than a quarter of the girth apart (10,000 km): beyond it [`bearing`], [`toward`] and
/// [`Convergence::between`] take their defined answers (D6) and the census reports the site.
#[inline]
pub fn is_far(a: Place, b: Place) -> bool {
    dot(a, b) < 0.0
}

/// The squared chord between two places, metres²: what range tests compare. No transcendental call.
#[inline]
pub fn chord_sq(a: Place, b: Place) -> f64 {
    let d = sub(a, b);
    R * R * dot(d, d)
}

/// The great-circle distance, metres (D6 as amended). To 1,000 km of chord it is the chord `c` times
/// `1 + x²/6 + 3x⁴/40 + 15x⁶/336 + 105x⁸/3456 + 945x¹⁰/42240`, `x = c / 2R` (one square root, no transcendental
/// call, 1e-9 m at the far end); beyond, `R·atan2(|a × b|, a·b)` (one `atan2`).
#[inline]
pub fn distance(a: Place, b: Place) -> f64 {
    let d = sub(a, b);
    let q = dot(d, d);
    if q <= SERIES_CHORD_Q {
        distance_near(q)
    } else {
        census!(count(Op::DistanceFar););
        distance_far(a, b)
    }
}
/// [`distance`]'s series, from `q = (chord / R)²`: for the test of its crossover.
#[inline]
pub fn distance_near(q: f64) -> f64 {
    R * q.sqrt() * asin_ratio(0.25 * q)
}
/// [`distance`]'s far form, `R·atan2(|a × b|, a·b)`: for the test of its crossover.
#[inline]
pub fn distance_far(a: Place, b: Place) -> f64 {
    let c = cross(a, b);
    R * dot(c, c).sqrt().atan2(dot(a, b))
}

/// The straight distance between two points, each a place and a height: the slant range, exact. One square root.
#[inline]
pub fn slant_range(a: Place, a_h: f64, b: Place, b_h: f64) -> f64 {
    let d = [
        R * (b[0] - a[0]) + (b_h * b[0] - a_h * a[0]),
        R * (b[1] - a[1]) + (b_h * b[1] - a_h * a[1]),
        R * (b[2] - a[2]) + (b_h * b[2] - a_h * a[2]),
    ];
    dot(d, d).sqrt()
}

/// The chord, metres, of a great-circle arc of `r` metres: `2R·sin(r / 2R)`, what a range held for many tests is
/// turned into once ([`within_chord`]). An arc of half the girth or more has the diameter. No transcendental call
/// to 2,000 km; one `sin` and one `cos` beyond.
#[inline]
pub fn chord_of(r: f64) -> f64 {
    if r >= HALF_GIRTH_M {
        return 2.0 * R;
    }
    let x = r * (0.5 * INV_R);
    r * sinc_versc(x * x).0
}
/// Whether `b` is within the chord `chord` of `a` (from [`chord_of`]): a comparison of squared chords.
#[inline]
pub fn within_chord(a: Place, b: Place, chord: f64) -> bool {
    chord_sq(a, b) <= chord * chord
}
/// Whether `b` lies within `r` metres of `a` along the great circle (D6): the squared chord against the squared
/// chord of `r`. Inclusive; no negative radius holds anything, and one of half the girth holds everything. No
/// transcendental call to 2,000 km.
#[inline]
pub fn within(a: Place, b: Place, r: f64) -> bool {
    r >= 0.0 && (r >= HALF_GIRTH_M || within_chord(a, b, chord_of(r)))
}

/// The true bearing at `a` toward `b`, radians clockwise from north in (−π, π] (D6). Coincident places give 0 (as
/// `atan2(+0, +0)` does in `Calc::heading_to`); so do places more than a quarter of the girth apart. One square
/// root, one division and one `atan2`.
#[inline]
pub fn bearing(a: Place, b: Place) -> f64 {
    Frame::at(a).bearing(b)
}
/// [`bearing`], but π for coincident places: `Calc::heading_to_neg`'s value there (`atan2(+0, −0)`, D6).
#[inline]
pub fn bearing_neg(a: Place, b: Place) -> f64 {
    if a == b { PI } else { bearing(a, b) }
}

/// `p` moved by `d = [east, south]`, a step given as metres on a sphere of radius `radius`, along the great circle
/// it starts on: `p·cos θ + t·sin θ` with `θ = |d| / radius`, taken as `p` plus a small vector, from `θ²` alone.
#[inline]
fn run(p: Place, d: [f64; 2], radius: f64) -> Place {
    if (d[0] == 0.0 && d[1] == 0.0) || !positive(radius) {
        return p;
    }
    let f = Frame::at(p);
    let inv = 1.0 / radius;
    let (te, ts) = (d[0] * inv, d[1] * inv);
    let w = te * te + ts * ts;
    let (s, c) = sinc_versc(w);
    let (ke, kn, kp) = (te * s, -ts * s, w * c);
    let (a, e, n) = (f.up, f.east, f.north);
    made([
        a[0] + (ke * e[0] + kn * n[0] - kp * a[0]),
        a[1] + (ke * e[1] + kn * n[1] - kp * a[1]),
        a[2] + (kn * n[2] - kp * a[2]),
    ])
}

/// The place `metres` along the great circle that leaves `p` on the true course `course` (D6): a search leg, a
/// patrol corner, an aim reckoned far off. Negative metres run astern. One `sin` and one `cos` of the course, and
/// no other call to 1,000 km (one more `sin` and `cos` beyond).
#[inline]
pub fn destination(p: Place, course: f64, metres: f64) -> Place {
    let (s, c) = course.sin_cos();
    run(p, [metres * s, -metres * c], R)
}

/// The place `metres` from `a` along the great circle toward `b`, flown at the height `h` (D6: a theatre flight's
/// step, which turns the place by `metres / (R + h)`). It does not stop at `b`: more metres than the distance run
/// on past it, and the caller that arrives writes `b`. Coincident places, a step of zero and places more than a
/// quarter of the girth apart give `a` itself (D6: no unique great circle is taken, and nothing moves). One square
/// root and two divisions; no transcendental call to 1,000 km.
#[inline]
pub fn toward(a: Place, b: Place, metres: f64, h: f64) -> Place {
    let radius = R + h;
    if a == b || metres == 0.0 || !positive(radius) || is_far(a, b) {
        census!(if a != b && is_far(a, b) {
            count(Op::FarAnswer);
        });
        return a;
    }
    // The tangent at a toward b: the chord less its part along a. Its length is the sine of the arc between them.
    let d = sub(b, a);
    let along = dot(a, d);
    let t = [
        d[0] - along * a[0],
        d[1] - along * a[1],
        d[2] - along * a[2],
    ];
    let sin2 = dot(t, t);
    if !positive(sin2) {
        return a;
    }
    let theta = metres / radius;
    let w = theta * theta;
    let (s, c) = sinc_versc(w);
    let (k, kp) = (theta * s / sin2.sqrt(), w * c);
    made([
        a[0] + (k * t[0] - kp * a[0]),
        a[1] + (k * t[1] - kp * a[1]),
        a[2] + (k * t[2] - kp * a[2]),
    ])
}

// --- Bodies that move (D5's two operations the frame must not be asked for) ---

/// A body at `p` and height `h` moving at `v = [east, up, south]`, the velocity it holds there, `seconds` on (D5: a
/// track's estimate, a led target). The level part runs along the surface and the height becomes `h + v_up·seconds`:
/// a straight line of a frame would leave the sea (31 m in a 20 km run). The place turns by the integral of
/// `v_level·dt / (R + h(t))` over the heights the body has on the way, which is what the same body stepped tick by
/// tick with [`step_aloft`] comes to: the run over `R + h`, times `ln(1 + x) / x` with `x = v_up·seconds / (R + h)`.
/// The radius at the start alone is off by `x / 2` of the run (299 m in a 318 km descent from 12 km), the mean of
/// the two radii by `x² / 12` of it (9 cm there). The velocity it holds there is [`Convergence::between`] the two
/// places of the one it held. No transcendental call for a run to 1,000 km and a rise to 49 km; one `ln_1p` beyond
/// that rise.
#[inline]
pub fn advance(p: Place, h: f64, v: Vec3, seconds: f64) -> (Place, f64) {
    let (rise, radius) = (v[1] * seconds, R + h);
    let to = h + rise;
    if !positive(radius) || !positive(radius + rise) {
        return (p, to);
    }
    let k = seconds * log_ratio(rise / radius);
    (run(p, [v[0] * k, v[2] * k], radius), to)
}

/// `ln(1 + x) / x`: its series to `x⁶ / 7` for `|x| ≤ 1/128` (truncated below 2.2e-16), one `ln_1p` beyond. 1 at 0.
#[inline]
fn log_ratio(x: f64) -> f64 {
    if x.abs() <= 1.0 / 128.0 {
        1.0 + x
            * (-1.0 / 2.0
                + x * (1.0 / 3.0
                    + x * (-1.0 / 4.0 + x * (1.0 / 5.0 + x * (-1.0 / 6.0 + x * (1.0 / 7.0))))))
    } else {
        x.ln_1p() / x
    }
}

/// The place a body at `p` and height `h` steps to by the level `d = [east, south]` it makes good at its own
/// height: the place turns by `|d| / (R + h)` (D5: an aircraft, a gliding wreck, a theatre flight, a torpedo at its
/// depth; a step taken at sea level overruns 63 m in 100 km flown at 4 km). Its height is its own integration. One
/// square root and two divisions; no transcendental call to 1,000 km.
#[inline]
pub fn step_aloft(p: Place, h: f64, d: [f64; 2]) -> Place {
    run(p, d, R + h)
}

// --- Parallel transport (D4) ---

/// The minimal rotation that takes `a` to `b`, applied to the Earth-centred vector `v` (D4 as revised):
/// `v − ((a + b)·v) / (1 + a·b) · (a + b) + 2(a·v)·b`. Up at `a` goes to up at `b`, a level vector at `a` is
/// parallel-transported along the great circle to `b`, and a vector with both keeps each. The identity for
/// coincident places and for places more than a quarter of the girth apart (D6). One division; no transcendental
/// call. [`Convergence`] is the same rotation on `[east, up, south]` components.
#[inline]
pub fn transport(a: Place, b: Place, v: Vec3) -> Vec3 {
    if a == b || is_far(a, b) {
        return v;
    }
    let sum = add(a, b);
    let (k, j) = (dot(sum, v) / (1.0 + dot(a, b)), 2.0 * dot(a, v));
    [
        v[0] - k * sum[0] + j * b[0],
        v[1] - k * sum[1] + j * b[1],
        v[2] - k * sum[2] + j * b[2],
    ]
}

/// What a move from one place to another does to the directions a body holds there (D4): the minimal rotation
/// between the two places, on `[east, up, south]` components. Up stays up; east and south turn by one angle γ, the
/// convergence of the meridians over the step, which is added to a heading (0.016° per km sailed east at 60°N).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Convergence {
    sin: f64,
    cos: f64,
}

impl Convergence {
    /// No carry: a step that kept the place.
    pub const NONE: Self = Self { sin: 0.0, cos: 1.0 };

    /// The carry from `a` to `b` (D4). With `d = b − a`, `Ñ = c·N` and `Ẽ = c·E` (the frames' axes before their
    /// division), `sin γ` and `cos γ` are in the ratio
    /// `a.z·(a.x·d.y − a.y·d.x)·(1 + a·b) + (d·Ña)(d·Ẽb)` to `(Ña·Ñb)·(1 + a·b) + (d·Ña)(d·Ñb)`: the north at `a`,
    /// turned by the rotation of [`transport`], on east and north at `b`. Every term is a product of small
    /// differences, so a step of a metre gives γ to full precision. [`Convergence::NONE`] for coincident places and
    /// for places more than a quarter of the girth apart (D6). One square root and one division; no transcendental
    /// call.
    #[inline]
    pub fn between(a: Place, b: Place) -> Self {
        census!(count(Op::Carry););
        if a == b || is_far(a, b) {
            census!(if a != b {
                count(Op::FarAnswer);
            });
            return Self::NONE;
        }
        census!(count(Op::CarryTurned););
        let d = sub(b, a);
        let na = [-a[2] * a[0], -a[2] * a[1], a[0] * a[0] + a[1] * a[1]];
        let nb = [-b[2] * b[0], -b[2] * b[1], b[0] * b[0] + b[1] * b[1]];
        let (dna, dnb, deb) = (dot(d, na), dot(d, nb), d[1] * b[0] - d[0] * b[1]);
        let k = 1.0 + dot(a, b);
        let s = a[2] * (a[0] * d[1] - a[1] * d[0]) * k + dna * deb;
        let c = dot(na, nb) * k + dna * dnb;
        let hyp = (s * s + c * c).sqrt();
        if !positive(hyp) {
            return Self::NONE;
        }
        let inv = 1.0 / hyp;
        Self {
            sin: s * inv,
            cos: c * inv,
        }
    }
    /// Whether the carry turns nothing.
    #[inline]
    pub fn is_none(&self) -> bool {
        self.sin == 0.0 && self.cos == 1.0
    }
    /// `(sin γ, cos γ)`.
    #[inline]
    pub fn sin_cos(&self) -> (f64, f64) {
        (self.sin, self.cos)
    }
    /// γ, radians: what the carry adds to a heading. For a step's convergence (to 2.9°) it is `sin γ` times D6's
    /// series, and no transcendental call is made; beyond, one `atan2`.
    #[inline]
    pub fn angle(&self) -> f64 {
        let t = self.sin * self.sin;
        if self.cos > 0.0 && t <= SERIES_TURN_T {
            self.sin * asin_ratio(t)
        } else {
            self.sin.atan2(self.cos)
        }
    }
    /// The heading `h` held at the old place, as it is held at the new one: `h + γ`, not wrapped (the holder wraps,
    /// as `Carry::heading_wrapped` has it). A carry that turns nothing returns `h` itself, bit for bit.
    #[inline]
    pub fn heading(&self, h: f64) -> f64 {
        if self.is_none() { h } else { h + self.angle() }
    }
    /// A level vector `[east, south]` the body held, as it holds it at the new place. No transcendental call.
    #[inline]
    pub fn tangent2(&self, v: [f64; 2]) -> [f64; 2] {
        if self.is_none() {
            return v;
        }
        [
            v[0] * self.cos - v[1] * self.sin,
            v[1] * self.cos + v[0] * self.sin,
        ]
    }
    /// A vector `[east, up, south]` the body held, as it holds it at the new place: its up component unchanged, its
    /// east and south turned as a heading is (D4 as revised). No transcendental call.
    #[inline]
    pub fn tangent3(&self, v: Vec3) -> Vec3 {
        let [e, s] = self.tangent2([v[0], v[2]]);
        [e, v[1], s]
    }
}

// --- Long ranges with no reach (step C, §3.6): everything but the antipode ---
//
// D6's forms above take their defined answers beyond a quarter of the girth. These answer there too, up to
// `ANTIPODE_GUARD_M` of the antipode, where a great circle between two places is no longer unique. Inside a quarter
// of the girth each *is* its D6 form (it calls it), so its answers there are today's, bit for bit. Beyond, each term
// that D6 takes from `b − a` (which cancels as `b` nears `−a`) is taken from `s = a + b`, which does not (its
// components subtract nearly equal numbers exactly), and `1 + a·b` is `½|s|²`: near the antipode `1 + a·b` keeps only
// a part in 1e8 of itself, `½|s|²` all of it. Nothing calls them yet.

/// How near the antipode a pair of places may come and still have one great circle between them, metres (§3.6): the
/// long-range forms take their defined answers within it. There the minimal rotation's divisor `1 + a·b` is at least
/// `(1 km / R)² / 2`, 1.2e-8.
pub const ANTIPODE_GUARD_M: f64 = 1_000.0;
/// `(chord / R)²` of an arc of [`ANTIPODE_GUARD_M`]: `(2 sin(g / 2R))² = (g / R)²·(1 − (g / R)² / 12)`, the next term
/// 1e-25 of it.
const ANTIPODE_Q: f64 = (ANTIPODE_GUARD_M / R)
    * (ANTIPODE_GUARD_M / R)
    * (1.0 - (ANTIPODE_GUARD_M / R) * (ANTIPODE_GUARD_M / R) / 12.0);

/// Whether `b` lies within [`ANTIPODE_GUARD_M`] of `a`'s antipode: the chord `|a + b|` from `−a` to `b` against the
/// guard's. No transcendental call.
#[inline]
pub fn is_antipodal(a: Place, b: Place) -> bool {
    let s = add(a, b);
    dot(s, s) <= ANTIPODE_Q
}

/// [`bearing`] to the antipode guard: the true bearing at `a` toward `b`, 0 for coincident places and within
/// [`ANTIPODE_GUARD_M`] of the antipode. [`bearing`] itself inside a quarter of the girth. One `atan2`.
#[inline]
pub fn bearing_long(a: Place, b: Place) -> f64 {
    Frame::at(a).bearing_long(b)
}

impl Frame {
    /// [`Frame::bearing`] to the antipode guard ([`bearing_long`]). Beyond a quarter of the girth the bearing is the
    /// direction of `s = to + a` on east and north (`a` has none on either), so nothing cancels near the antipode.
    #[inline]
    pub fn bearing_long(&self, to: Place) -> f64 {
        if !is_far(self.up, to) {
            return self.bearing(to);
        }
        if is_antipodal(self.up, to) {
            return 0.0;
        }
        let s = add(to, self.up);
        dot(s, self.east).atan2(dot(s, self.north))
    }
}

/// [`toward`] to the antipode guard: the place `metres` from `a` along the great circle toward `b`, flown at the
/// height `h`. Within [`ANTIPODE_GUARD_M`] of the antipode, as for coincident places and a step of zero, `a` itself.
/// [`toward`] itself inside a quarter of the girth. Beyond, the tangent at `a` toward `b`, `b − (a·b)·a`, is taken as
/// `s − (a·s)·a` with `s = a + b`. No transcendental call to 1,000 km of step; a `sin` and a `cos` beyond.
#[inline]
pub fn toward_long(a: Place, b: Place, metres: f64, h: f64) -> Place {
    if !is_far(a, b) {
        return toward(a, b, metres, h);
    }
    let radius = R + h;
    if metres == 0.0 || !positive(radius) || is_antipodal(a, b) {
        return a;
    }
    // `a·s` is `½|s|²` for unit places; taken as `a·s`, the tangent is square to `a` however far from unit the two
    // places are stored (by 1e-16, which near the antipode would tilt `½|s|²`'s tangent by 1e-16 / |s|).
    let s = add(a, b);
    let along = dot(a, s);
    let t = [
        s[0] - along * a[0],
        s[1] - along * a[1],
        s[2] - along * a[2],
    ];
    let sin2 = dot(t, t);
    if !positive(sin2) {
        return a;
    }
    let theta = metres / radius;
    let w = theta * theta;
    let (sc, vc) = sinc_versc(w);
    let (k, kp) = (theta * sc / sin2.sqrt(), w * vc);
    made([
        a[0] + (k * t[0] - kp * a[0]),
        a[1] + (k * t[1] - kp * a[1]),
        a[2] + (k * t[2] - kp * a[2]),
    ])
}

/// [`transport`] to the antipode guard: the minimal rotation taking `a` to `b`, applied to `v`; the identity within
/// [`ANTIPODE_GUARD_M`] of the antipode. [`transport`] itself inside a quarter of the girth; beyond, its divisor
/// `1 + a·b` is `½|s|²`. One division; no transcendental call.
#[inline]
pub fn transport_long(a: Place, b: Place, v: Vec3) -> Vec3 {
    if !is_far(a, b) {
        return transport(a, b, v);
    }
    if is_antipodal(a, b) {
        return v;
    }
    let sum = add(a, b);
    let (k, j) = (dot(sum, v) / (0.5 * dot(sum, sum)), 2.0 * dot(a, v));
    [
        v[0] - k * sum[0] + j * b[0],
        v[1] - k * sum[1] + j * b[1],
        v[2] - k * sum[2] + j * b[2],
    ]
}

impl Convergence {
    /// [`Convergence::between`] to the antipode guard: [`Convergence::NONE`] within [`ANTIPODE_GUARD_M`] of the
    /// antipode, [`Convergence::between`] itself inside a quarter of the girth. Beyond, each term is taken from
    /// `s = a + b` (`d·Ña = s·Ña`, `d·Ñb = −s·Ñb`, `d.y·b.x − d.x·b.y = s.x·b.y − s.y·b.x`, `a.x·d.y − a.y·d.x =
    /// a.x·s.y − a.y·s.x`) and `1 + a·b` is `½|s|²`, so near the antipode every term keeps its precision. One square
    /// root and one division; no transcendental call.
    #[inline]
    pub fn between_long(a: Place, b: Place) -> Self {
        if !is_far(a, b) {
            return Self::between(a, b);
        }
        if is_antipodal(a, b) {
            return Self::NONE;
        }
        let s = add(a, b);
        let na = [-a[2] * a[0], -a[2] * a[1], a[0] * a[0] + a[1] * a[1]];
        let nb = [-b[2] * b[0], -b[2] * b[1], b[0] * b[0] + b[1] * b[1]];
        let (dna, dnb, deb) = (dot(s, na), -dot(s, nb), s[0] * b[1] - s[1] * b[0]);
        let k = 0.5 * dot(s, s);
        let sn = a[2] * (a[0] * s[1] - a[1] * s[0]) * k + dna * deb;
        let c = dot(na, nb) * k + dna * dnb;
        let hyp = (sn * sn + c * c).sqrt();
        if !positive(hyp) {
            return Self::NONE;
        }
        let inv = 1.0 / hyp;
        Self {
            sin: sn * inv,
            cos: c * inv,
        }
    }
}

// --- The chart (D7): the azimuthal equidistant projection about the scenario's origin ---

/// The chart about a scenario's origin, in the meridian frame (D7): the wire, the keys and the land are laid on it.
/// Its origin is `(cos φ0, 0, sin φ0)`; the origin's longitude is used by [`Chart::lat_lon`] and
/// [`Chart::from_lat_lon`] alone (D1, D10). [`Chart::about_place`] lays the same projection about any other place of
/// the same meridian frame (step C: a scene chart, C7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chart {
    /// The origin's place.
    o: Place,
    sin0: f64,
    cos0: f64,
    at: LatLon,
    /// The meridian frame's longitude, degrees: the scenario origin's (D1), what [`Chart::lat_lon`] adds.
    meridian: f64,
    /// East and north at the origin when it lies off the frame's meridian ([`Chart::about_place`]); `None` on it,
    /// where they are `(0, 1, 0)` and `(−sin φ0, 0, cos φ0)` and [`Chart::about`]'s arithmetic is kept.
    axes: Option<[Vec3; 2]>,
}

/// The chart's Jacobian at a place: `J` takes a displacement `[east, south]` there to the chart displacement
/// `[x, z]` it makes. The chart keeps lengths along the radial from its origin and stretches those across it by
/// `σ / sin σ` (0.41 % at the reach), so displacements and gradients transform differently.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Jacobian(pub [[f64; 2]; 2]);

impl Jacobian {
    /// `J·t`: a displacement or velocity `[east, south]` at the place, on the chart's grid.
    #[inline]
    pub fn apply(&self, t: [f64; 2]) -> [f64; 2] {
        let j = &self.0;
        [
            j[0][0] * t[0] + j[0][1] * t[1],
            j[1][0] * t[0] + j[1][1] * t[1],
        ]
    }
    /// `J⁻¹·v`: a displacement or velocity on the chart's grid, as `[east, south]` at the place. One division.
    #[inline]
    pub fn solve(&self, v: [f64; 2]) -> [f64; 2] {
        let w = self.adjugate(v);
        let det = self.det();
        [w[0] / det, w[1] / det]
    }
    /// `Jᵀ·n`: a gradient or a normal on the chart's grid, as the gradient `[east, south]` at the place. It keeps a
    /// normal square to the level line it is the normal of, which `J⁻¹` does not.
    #[inline]
    pub fn transposed(&self, n: [f64; 2]) -> [f64; 2] {
        let j = &self.0;
        [
            j[0][0] * n[0] + j[1][0] * n[1],
            j[0][1] * n[0] + j[1][1] * n[1],
        ]
    }
    /// The determinant: the chart's areal stretch, `σ / sin σ`, never below 1.
    #[inline]
    pub fn det(&self) -> f64 {
        let j = &self.0;
        j[0][0] * j[1][1] - j[0][1] * j[1][0]
    }
    /// `det(J)·J⁻¹·v`: the direction of `J⁻¹·v` with no division (the determinant is positive).
    #[inline]
    fn adjugate(&self, v: [f64; 2]) -> [f64; 2] {
        let j = &self.0;
        [
            j[1][1] * v[0] - j[0][1] * v[1],
            j[0][0] * v[1] - j[1][0] * v[0],
        ]
    }
}

/// The heading of a level vector `[east, south]` (or of `[x, z]` on the chart): `atan2(x, −z)`.
#[inline]
fn heading_of(v: [f64; 2]) -> f64 {
    v[0].atan2(-v[1])
}

impl Chart {
    /// The chart about `origin`. One `sin` and one `cos`, once.
    #[inline]
    pub fn about(origin: LatLon) -> Self {
        let (sin0, cos0) = origin.lat.to_radians().sin_cos();
        Self {
            o: [cos0, 0.0, sin0],
            sin0,
            cos0,
            at: origin,
            meridian: origin.lon,
            axes: None,
        }
    }
    /// The same projection about the place `o` of this chart's meridian frame (step C: a scene chart about a place
    /// near the camera, C7; a chart about any place). This chart itself, bit for bit, when `o` is its origin, or
    /// within [`SAME_ORIGIN_M`] of it: the origin's place made again from its latitude and longitude may differ from
    /// it in a last bit. On the frame's meridian (`o.y = 0`, `o.x > 0`) it is [`Chart::about`]'s arithmetic with
    /// `sin φ0 = o.z` and `cos φ0 = o.x`, so a chart about such a place computes what the scenario's chart about it
    /// would. Elsewhere east and north at `o` are the frame's ([`Frame::at`]) and every projection is the same algebra
    /// on them. Its places and latitudes and longitudes stay this frame's. Two `atan2` (the origin's latitude and
    /// longitude) when it is not this chart.
    #[inline]
    pub fn about_place(&self, o: Place) -> Self {
        let d = sub(o, self.o);
        if dot(d, d) <= SAME_ORIGIN_Q {
            return *self;
        }
        let at = self.lat_lon(o);
        if o[1] == 0.0 && positive(o[0]) {
            return Self {
                o,
                sin0: o[2],
                cos0: o[0],
                at,
                meridian: self.meridian,
                axes: None,
            };
        }
        let f = Frame::at(o);
        Self {
            o: f.up,
            sin0: f.up[2],
            cos0: f.north[2],
            at,
            meridian: self.meridian,
            axes: Some([f.east, f.north]),
        }
    }
    /// The origin's place: `(cos φ0, 0, sin φ0)` (D1).
    #[inline]
    pub fn origin(&self) -> Place {
        self.o
    }
    /// The origin's latitude and longitude, as given.
    #[inline]
    pub fn origin_lat_lon(&self) -> LatLon {
        self.at
    }

    /// `(e, n, t)` of `p`: its components along east and north at the origin, from `p − o`, and a quarter of the
    /// squared chord from the origin.
    #[inline]
    fn about_origin(&self, p: Place) -> (f64, f64, f64, Vec3) {
        if let Some([east, north]) = self.axes {
            let d = sub(p, self.o);
            return (dot(d, east), dot(d, north), 0.25 * dot(d, d), d);
        }
        let d = [p[0] - self.o[0], p[1], p[2] - self.o[2]];
        (
            p[1],
            d[2] * self.cos0 - d[0] * self.sin0,
            0.25 * dot(d, d),
            d,
        )
    }
    /// `σ / sin σ` and `cos σ` for the place `σ` from the origin whose `(e, n, t)` are given: the chart's scale
    /// `k / R`. `None` at the origin's antipode, where the chart ends.
    #[inline]
    fn scale(&self, p: Place, e: f64, n: f64, t: f64) -> Option<(f64, f64)> {
        if t <= 0.25 * SERIES_CHORD_Q {
            // σ = 2·asin(x) and sin σ = 2x·√(1 − x²) with x half the chord, so the ratio needs no angle.
            return Some((asin_ratio(t) / (1.0 - t).sqrt(), 1.0 - 2.0 * t));
        }
        let s = (e * e + n * n).sqrt();
        let u = match self.axes {
            None => p[0] * self.cos0 + p[2] * self.sin0,
            Some(_) => dot(p, self.o),
        };
        positive(s).then(|| (s.atan2(u) / s, u))
    }

    /// The chart metres `[x east, z south]` of `p` (D7): `e = p.y`, `n = p·N0`, `u = p·o`, then
    /// `k = R·atan2(hypot(e, n), u) / hypot(e, n)`, `x = k·e`, `z = −k·n`, as `theatre::geo::to_local`. Inside
    /// 1,000 km of chord from the origin, which holds the reach, `k` is `R` times D6's series over `√(1 − x²)` with
    /// `x` half the chord: one square root and one division, no transcendental call. Beyond, one `atan2`. The origin
    /// is `[+0, +0]`, its antipode `[0, −πR]`.
    #[inline]
    pub fn forward(&self, p: Place) -> [f64; 2] {
        census!(count(Op::ChartFix););
        let (e, n, t, _) = self.about_origin(p);
        match self.scale(p, e, n, t) {
            Some((m, _)) => {
                let k = R * m;
                [k * e + 0.0, 0.0 - k * n]
            }
            None => [0.0, -HALF_GIRTH_M],
        }
    }
    /// The place whose chart metres are `q` (D7: `theatre::geo::to_lat_lon` without its longitude step):
    /// `o·cos c + (x·E0 − z·N0)·sin c / ρ`, `c = ρ / R`, taken as `o` plus a small vector, from `c²` alone. The
    /// chart's origin gives the origin's place itself. No transcendental call to 1,000 km from the origin; one `sin`
    /// and one `cos` beyond.
    #[inline]
    pub fn inverse(&self, q: [f64; 2]) -> Place {
        census!(count(Op::ChartInverse););
        saturate(self.unprojected(q))
    }
    /// [`Chart::inverse`] before the pole limit: the unit vector the chart metres `q` name.
    #[inline]
    fn unprojected(&self, q: [f64; 2]) -> Vec3 {
        let (x, z) = (q[0] * INV_R, q[1] * INV_R);
        let w = x * x + z * z;
        if w == 0.0 {
            return self.o;
        }
        let (s, c) = sinc_versc(w);
        let (zs, kp) = (z * s, w * c);
        if let Some([e, n]) = self.axes {
            let (xs, o) = (x * s, self.o);
            return renormalized([
                o[0] + (xs * e[0] - zs * n[0] - kp * o[0]),
                o[1] + (xs * e[1] - zs * n[1] - kp * o[1]),
                o[2] + (xs * e[2] - zs * n[2] - kp * o[2]),
            ]);
        }
        renormalized([
            self.o[0] + (zs * self.sin0 - kp * self.o[0]),
            x * s,
            self.o[2] - (zs * self.cos0 + kp * self.o[2]),
        ])
    }
    /// The distance of `p` from the chart's origin, metres: `hypot` of its chart metres, which the projection keeps
    /// exactly (the reach disc, `BattleArea`). As [`distance`] from the origin.
    #[inline]
    pub fn range(&self, p: Place) -> f64 {
        distance(self.o, p)
    }
    /// The place whose chart metres are `p`'s, each rounded to the metre: the identity the theatre keeps a stored
    /// place by. Idempotent in bits: the place of whole metres projects back to within nanometres of them, rounds to
    /// the same whole metres and is made from them again. Against a polar cap the nearest whole metres may lie inside
    /// it, and saturating them would leave the whole metres: it is then the nearest of the eight whole-metre points
    /// about them that is a place (one is always at least half a metre clear of the cap), which keeps both. No
    /// transcendental call inside the reach.
    #[inline]
    pub fn round_m(&self, p: Place) -> Place {
        let q = self.forward(p);
        // A coordinate that rounds to zero from below is −0: taken as +0, as the chart fix of the result is.
        let r = [q[0].round() + 0.0, q[1].round() + 0.0];
        let nearest = self.unprojected(r);
        if within_pole_limit(nearest) {
            return nearest;
        }
        let mut best: Option<(f64, Place)> = None;
        for dx in [-1.0, 0.0, 1.0] {
            for dz in [-1.0, 0.0, 1.0] {
                let metres = [r[0] + dx + 0.0, r[1] + dz + 0.0];
                let place = self.unprojected(metres);
                let off = (metres[0] - q[0]).powi(2) + (metres[1] - q[1]).powi(2);
                if within_pole_limit(place) && best.is_none_or(|(least, _)| off < least) {
                    best = Some((off, place));
                }
            }
        }
        best.map_or_else(|| saturate(nearest), |(_, place)| place)
    }

    /// The chart's Jacobian at `at`. With `m = σ / sin σ` and `q = (m·cos σ − 1) / sin² σ`, a tangent `W` at
    /// `at` goes to `m·(W·E0, −W·N0) + q·(W·o)·(e, −n)`: the projection's differential, whose second term is of the
    /// order of the squared distance from the origin. At the origin it is the identity. One square root and two
    /// divisions beside the frame at `at`; no transcendental call inside the reach.
    #[inline]
    pub fn jacobian(&self, at: Place) -> Jacobian {
        census!(count(Op::Jacobian););
        let f = Frame::at(at);
        let (e, n, t, d) = self.about_origin(f.up);
        let Some((m, u)) = self.scale(f.up, e, n, t) else {
            return Jacobian([[1.0, 0.0], [0.0, 1.0]]);
        };
        let s2 = e * e + n * n;
        // Its limit at the origin is −1/3; elsewhere the cancellation in `m·u − 1` is multiplied back by `s2`.
        let q = if positive(s2) {
            (m * u - 1.0) / s2
        } else {
            -1.0 / 3.0
        };
        let column = |w: Vec3| {
            // W·o = −W·(at − o), since W is level at `at`.
            let wo = -dot(w, d);
            match self.axes {
                None => [
                    m * w[1] + q * wo * e,
                    -(m * (w[2] * self.cos0 - w[0] * self.sin0) + q * wo * n),
                ],
                Some([east0, north0]) => [
                    m * dot(w, east0) + q * wo * e,
                    -(m * dot(w, north0) + q * wo * n),
                ],
            }
        };
        let (east, south) = (column(f.east), column(scale(f.north, -1.0)));
        Jacobian([[east[0], south[0]], [east[1], south[1]]])
    }
    /// A displacement or a velocity `v` given on the chart's grid, as `[east, south]` at `at`: through `J⁻¹` (the
    /// wind's set, a report's error, a homing offset). `Calc::chart_vec_on`.
    #[inline]
    pub fn displacement(&self, at: Place, v: [f64; 2]) -> [f64; 2] {
        self.jacobian(at).solve(v)
    }
    /// The displacement or velocity `t = [east, south]` at `at`, on the chart's grid: through `J`.
    /// `Calc::to_chart_vec_on`, and what the wire and the scene carry (D12).
    #[inline]
    pub fn to_displacement(&self, at: Place, t: [f64; 2]) -> [f64; 2] {
        self.jacobian(at).apply(t)
    }
    /// A gradient or a normal `n` given on the chart's grid, as the gradient `[east, south]` at `at`: through `Jᵀ`
    /// (the way off the land, a wave front's normal). Not made a unit vector again. `Calc::chart_normal`.
    #[inline]
    pub fn gradient(&self, at: Place, n: [f64; 2]) -> [f64; 2] {
        self.jacobian(at).transposed(n)
    }
    /// The true heading a hull at `at` steers along the chart leg to `to`: the grid bearing between their chart
    /// fixes, through `J⁻¹` at `at`. 0 for coincident places. Taken again every step, it keeps her on the chart line
    /// the navigation grid cleared. One `atan2`, and no other call inside the reach.
    #[inline]
    pub fn course_to(&self, at: Place, to: Place) -> f64 {
        self.course_along(at, to, at)
    }
    /// The course of the chart leg from `from` to `to`, as a true heading held at `at`, a place on the leg. 0 for a
    /// leg of no length. One `atan2`.
    #[inline]
    pub fn course_along(&self, from: Place, to: Place, at: Place) -> f64 {
        let (a, b) = (self.forward(from), self.forward(to));
        let leg = [b[0] - a[0], b[1] - a[1]];
        if leg[0] == 0.0 && leg[1] == 0.0 {
            return 0.0;
        }
        heading_of(self.jacobian(at).adjugate(leg))
    }
    /// The true heading `h` of a body at `at`, as a grid heading on the chart (radians clockwise from the chart's
    /// up): what a DTO or a report carries. One `sin`, one `cos` and one `atan2`.
    #[inline]
    pub fn grid_heading(&self, at: Place, h: f64) -> f64 {
        let (s, c) = h.sin_cos();
        heading_of(self.jacobian(at).apply([s, -c]))
    }
    /// A grid heading on the chart as the true heading of a body at `at`: the inverse of [`Chart::grid_heading`].
    /// One `sin`, one `cos` and one `atan2`.
    #[inline]
    pub fn resolve_heading(&self, at: Place, grid: f64) -> f64 {
        let (s, c) = grid.sin_cos();
        heading_of(self.jacobian(at).adjugate([s, -c]))
    }
    /// The radius, in chart metres, that a search keyed on the chart takes to find every place truly within `r`
    /// metres: `r` times [`COVER`], the chart's greatest stretch to 1,200 km. The candidates are confirmed by
    /// [`within`].
    #[inline]
    pub fn cover(r: f64) -> f64 {
        COVER * r
    }
    /// The radius, in chart metres, that a search keyed on this chart takes at `at` to find every place truly within
    /// `r` metres of it (step C, C11): `r · max(COVER, σ / sin σ)` at `σ = (range(at) + r) / R`. The chart's largest
    /// scale over that disc is the tangential `σ / sin σ` at its far side, so no place of it lies farther off on the
    /// chart. Inside [`COVER_RANGE_M`] of the origin it is [`Chart::cover`] itself, bit for bit; a disc that reaches
    /// the origin's antipode takes the chart's whole diameter, `2πR`. The range is [`distance`]'s (an `atan2` past
    /// 1,000 km of chord); past [`COVER_RANGE_M`], one `sin`.
    #[inline]
    pub fn cover_at(&self, r: f64, at: Place) -> f64 {
        let reach = self.range(at) + r;
        if reach <= COVER_RANGE_M {
            return Self::cover(r);
        }
        if reach >= HALF_GIRTH_M {
            return 2.0 * HALF_GIRTH_M;
        }
        let sigma = reach * INV_R;
        let stretch = 1.0 / sinc_versc(sigma * sigma).0;
        (r * COVER.max(stretch)).min(2.0 * HALF_GIRTH_M)
    }
    /// The place `p` of this chart's meridian frame as an Earth-fixed unit vector: x toward 0° N 0° E, y toward
    /// 0° N 90° E, z toward the north pole (step C's lattices are laid on the Earth, not on a scenario). The frame
    /// turned back about the pole by its meridian's longitude. One `sin` and one `cos`.
    #[inline]
    pub fn earth_fixed(&self, p: Place) -> Vec3 {
        let (s, c) = self.meridian.to_radians().sin_cos();
        [p[0] * c - p[1] * s, p[0] * s + p[1] * c, p[2]]
    }
    /// The place of this chart's meridian frame that the Earth-fixed unit vector `v` names: the inverse of
    /// [`Chart::earth_fixed`], no nearer a pole than a place may be. One `sin` and one `cos`.
    #[inline]
    pub fn from_earth_fixed(&self, v: Vec3) -> Place {
        let (s, c) = self.meridian.to_radians().sin_cos();
        made([v[0] * c + v[1] * s, v[1] * c - v[0] * s, v[2]])
    }

    /// The latitude and longitude of `p`, degrees, the longitude wrapped to [−180, 180) (D10): one of the two
    /// functions a longitude exists in. Two `atan2`.
    #[inline]
    pub fn lat_lon(&self, p: Place) -> LatLon {
        LatLon {
            lat: p[2].atan2(p[0].hypot(p[1])).to_degrees(),
            lon: wrap_lon(self.meridian + p[1].atan2(p[0]).to_degrees()),
        }
    }
    /// The place at `ll`, its longitude taken east of the origin's and wrapped first (D10): the other function a
    /// longitude exists in. A latitude within half a degree of a pole is put on the 89.5° parallel (D9). Two `sin`
    /// and two `cos`.
    #[inline]
    pub fn from_lat_lon(&self, ll: LatLon) -> Place {
        let (sin_p, cos_p) = ll.lat.to_radians().sin_cos();
        let (sin_l, cos_l) = wrap_lon(ll.lon - self.meridian).to_radians().sin_cos();
        made([cos_p * cos_l, cos_p * sin_l, sin_p])
    }
}
