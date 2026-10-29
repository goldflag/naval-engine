//! The scenario editor's globe (native/docs/ui-contract/maps.md, "The editor's globe"): the Earth seen from straight
//! above a view centre, orthographic and north up, and the region frame's metres on it. The page's twin is
//! `src/ui/theatre/map/globe.ts`; both follow the shared brief's maths, degrees in the contract and radians inside.
//!
//! - unit(lat, lon) = (cos φ cos λ, cos φ sin λ, sin φ).
//! - At the view centre C = (φ0, λ0): east e = (−sin λ0, cos λ0, 0), north n = (−sin φ0 cos λ0, −sin φ0 sin λ0, cos φ0),
//!   up u = unit(C).
//! - Forward: v = unit(P); x = R (v·e), y = R (v·n); seen while v·u > 0. Scene pixels: W/2 + pxPerM·x, H/2 − pxPerM·y.
//! - Inverse: ρ² = x² + y² (metres off the centre); off the globe past R; v = (x e + y n + sqrt(R² − ρ²) u) / R.
//! - The region frame (azimuthal equidistant about its origin, x east, z south) to and from unit vectors, as
//!   `naval_earth::geo` (`to_local`, `to_lat_lon`), the land bake's `naval_region_bake::geo::Frame` and
//!   `src/theatre/geo.ts` (`toLocal`, `toLatLon`) do.

pub use naval_earth::EARTH_RADIUS_M;

const RAD: f64 = std::f64::consts::PI / 180.0;
/// A point this close to the limb (v·u) is behind it: its projection is foreshortened to nothing.
pub const LIMB: f64 = 1e-9;
/// The view centre's latitude is held within this many degrees of the equator (the basis is undefined at a pole).
pub const MAX_LAT: f64 = 89.0;

pub type V3 = [f64; 3];

#[inline]
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn scaled(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
#[inline]
fn sum(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn normalized(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    if n > 0.0 { scaled(a, 1.0 / n) } else { a }
}

/// The unit vector of `[lat, lon]` degrees.
#[inline]
pub fn unit(lat: f64, lon: f64) -> V3 {
    let (sp, cp) = (lat * RAD).sin_cos();
    let (sl, cl) = (lon * RAD).sin_cos();
    [cp * cl, cp * sl, sp]
}

/// `[lat, lon]` degrees of a unit vector, longitude in (−180, 180].
#[inline]
pub fn lat_lon(v: V3) -> [f64; 2] {
    [v[2].clamp(-1.0, 1.0).asin() / RAD, v[1].atan2(v[0]) / RAD]
}

/// East, north and up at `[lat, lon]` degrees.
pub fn basis([lat, lon]: [f64; 2]) -> [V3; 3] {
    let (sp, cp) = (lat * RAD).sin_cos();
    let (sl, cl) = (lon * RAD).sin_cos();
    [
        [-sl, cl, 0.0],
        [-sp * cl, -sp * sl, cp],
        [cp * cl, cp * sl, sp],
    ]
}

/// The region frame about an origin: its metres (x east, z south, azimuthal equidistant) to and from unit vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoFrame {
    pub origin: [f64; 2],
    east: V3,
    north: V3,
    up: V3,
}

impl GeoFrame {
    pub fn new(origin: [f64; 2]) -> Self {
        let [east, north, up] = basis(origin);
        Self {
            origin,
            east,
            north,
            up,
        }
    }

    /// The unit vector of frame point (x east, z south): the point `hypot(x, z)` along the great circle from the origin
    /// at that bearing (`Frame::geographic`).
    #[inline]
    pub fn unit(&self, [x, z]: [f64; 2]) -> V3 {
        let rho = x.hypot(z);
        if rho == 0.0 {
            return self.up;
        }
        let (s, c) = (rho / EARTH_RADIUS_M).sin_cos();
        let k = s / rho;
        sum(
            scaled(self.up, c),
            sum(scaled(self.east, x * k), scaled(self.north, -z * k)),
        )
    }

    /// The frame point of a unit vector (`Frame::to_local`, `toLocal`); the origin's antipode is due north at πR.
    #[inline]
    pub fn local(&self, v: V3) -> [f64; 2] {
        let (e, n, u) = (dot(v, self.east), dot(v, self.north), dot(v, self.up));
        let s = e.hypot(n);
        if s == 0.0 {
            return if u >= 0.0 {
                [0.0, 0.0]
            } else {
                [0.0, -std::f64::consts::PI * EARTH_RADIUS_M]
            };
        }
        let k = EARTH_RADIUS_M * s.atan2(u) / s;
        [k * e, -k * n]
    }

    /// The frame's east, north and up.
    pub fn axes(&self) -> [V3; 3] {
        [self.east, self.north, self.up]
    }
}

/// The globe's camera over a scene of `width` by `height` pixels: its centre and scale, and the basis there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Globe {
    /// The view's centre, `[lat, lon]` degrees.
    pub centre: [f64; 2],
    /// Scene pixels per metre at the centre.
    pub px_per_m: f64,
    pub width: f64,
    pub height: f64,
    pub east: V3,
    pub north: V3,
    pub up: V3,
}

impl Globe {
    pub fn new(centre: [f64; 2], px_per_m: f64, width: f64, height: f64) -> Self {
        let centre = [centre[0].clamp(-MAX_LAT, MAX_LAT), wrap(centre[1])];
        let [east, north, up] = basis(centre);
        Self {
            centre,
            px_per_m,
            width,
            height,
            east,
            north,
            up,
        }
    }

    /// The same view about another centre.
    pub fn at(&self, centre: [f64; 2]) -> Self {
        Self::new(centre, self.px_per_m, self.width, self.height)
    }

    /// The globe's radius, scene pixels.
    pub fn radius_px(&self) -> f64 {
        EARTH_RADIUS_M * self.px_per_m
    }

    /// Scene pixels of a unit vector and its height toward the eye (v·u): where it would stand, seen or not.
    #[inline]
    pub fn place(&self, v: V3) -> ([f64; 2], f64) {
        let k = EARTH_RADIUS_M * self.px_per_m;
        (
            [
                self.width / 2.0 + k * dot(v, self.east),
                self.height / 2.0 - k * dot(v, self.north),
            ],
            dot(v, self.up),
        )
    }

    /// Scene pixels of a unit vector on the near side; none behind the limb.
    #[inline]
    pub fn forward(&self, v: V3) -> Option<[f64; 2]> {
        let (p, up) = self.place(v);
        (up > LIMB).then_some(p)
    }

    /// The unit vector under scene pixel `p`; none off the Earth's disc.
    pub fn inverse(&self, [sx, sy]: [f64; 2]) -> Option<V3> {
        let x = (sx - self.width / 2.0) / self.px_per_m;
        let y = (self.height / 2.0 - sy) / self.px_per_m;
        let r2 = EARTH_RADIUS_M * EARTH_RADIUS_M;
        let rho2 = x * x + y * y;
        if rho2.is_nan() || rho2 > r2 {
            return None;
        }
        let w = (r2 - rho2).sqrt();
        Some(normalized(scaled(
            sum(
                sum(scaled(self.east, x), scaled(self.north, y)),
                scaled(self.up, w),
            ),
            1.0 / EARTH_RADIUS_M,
        )))
    }

    /// The point a tangent offset of `east` and `north` metres from the centre, as the centre a camera moves to.
    fn offset(&self, east: f64, north: f64) -> [f64; 2] {
        lat_lon(GeoFrame::new(self.centre).unit([east, -north]))
    }

    /// Turns the globe so `v` stands under scene pixel `s` (the grabbed point of a drag, the point a zoom is about),
    /// within 0.05 px. A pixel off the Earth's disc leaves the camera where it is; the centre's latitude is held within
    /// [`MAX_LAT`].
    ///
    /// The centre is solved for in closed form where a north-up view can put `v` there: with `v`'s view coordinates
    /// (x, y, w) wanted, `v·e = cos φ sin(λ − λ0) = x` gives λ0, and turning (cos φ cos(λ − λ0), sin φ) by φ0 onto
    /// (w, y) gives φ0. The shared brief's step (move the centre by the miss, at most four times) then settles what is
    /// left, as where the latitude was held.
    pub fn grab(&self, v: V3, s: [f64; 2]) -> Self {
        let k = self.radius_px();
        let x = (s[0] - self.width / 2.0) / k;
        let y = (self.height / 2.0 - s[1]) / k;
        let rr = x * x + y * y;
        if rr.is_nan() || rr > 1.0 {
            return *self;
        }
        let w = (1.0 - rr).sqrt();
        let [lat, lon] = lat_lon(v);
        let (sp, cp) = (lat * RAD).sin_cos();
        let mut camera = *self;
        if cp > 1e-9 && x.abs() <= cp {
            let turn = (x / cp).asin();
            let nearest = [turn, std::f64::consts::PI - turn]
                .into_iter()
                .map(|t| {
                    let phi = sp.atan2(cp * t.cos()) - y.atan2(w);
                    [phi / RAD, wrap(lon - t / RAD)]
                })
                .filter(|c| c[0].abs() <= 90.0)
                .max_by(|a, b| {
                    dot(unit(a[0], a[1]), self.up).total_cmp(&dot(unit(b[0], b[1]), self.up))
                });
            if let Some(centre) = nearest {
                camera = self.at(centre);
            }
        }
        for _ in 0..4 {
            let (s0, _) = camera.place(v);
            let d = [
                (s[0] - s0[0]) / camera.px_per_m,
                (s0[1] - s[1]) / camera.px_per_m,
            ];
            if (s[0] - s0[0]).hypot(s[1] - s0[1]) < 0.05 {
                break;
            }
            camera = camera.at(camera.offset(-d[0], -d[1]));
        }
        camera
    }

    /// The view's angular radius about its centre, radians: to its farthest corner, or the hemisphere when the limb is
    /// in view.
    pub fn reach(&self) -> f64 {
        let rho = (self.width / 2.0).hypot(self.height / 2.0) / self.px_per_m;
        if rho >= EARTH_RADIUS_M {
            std::f64::consts::FRAC_PI_2
        } else {
            (rho / EARTH_RADIUS_M).asin()
        }
    }
}

/// A longitude in (−180, 180].
pub fn wrap(lon: f64) -> f64 {
    let l = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if l == -180.0 { 180.0 } else { l }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_earth::geo::{LatLon, to_lat_lon, to_local};
    use naval_region_bake::geo::Frame;

    #[test]
    fn the_frame_matches_the_bakes_and_the_pages() {
        for origin in [[-9.5, 160.0], [35.0, 25.0], [64.0, -28.0], [0.0, 179.5]] {
            let ours = GeoFrame::new(origin);
            let bake = Frame::new(origin[0], origin[1]);
            let theatre = LatLon {
                lat: origin[0],
                lon: origin[1],
            };
            for p in [
                [0.0, 0.0],
                [120_000.0, -40_000.0],
                [-900_000.0, 1_500_000.0],
                [3_000_000.0, 2_000_000.0],
            ] {
                let [lat, lon] = bake.geographic(p[0], p[1]);
                let v = ours.unit(p);
                let [la, lo] = lat_lon(v);
                assert!((la - lat).abs() < 1e-9, "{origin:?} {p:?}");
                assert!((wrap(lo) - wrap(lon)).abs() < 1e-9, "{origin:?} {p:?}");
                let back = ours.local(v);
                let theirs = bake.to_local(lat, lon);
                assert!(
                    (back[0] - p[0]).hypot(back[1] - p[1]) < 1e-4,
                    "{back:?} {p:?}"
                );
                assert!((theirs[0] - p[0]).hypot(theirs[1] - p[1]) < 1e-4);
                // The theatre's frame (the simulation's), the same projection.
                let at = to_lat_lon(theatre, p);
                assert!((at.lat - lat).abs() < 1e-9, "{origin:?} {p:?}");
                assert!((wrap(at.lon) - wrap(lon)).abs() < 1e-9, "{origin:?} {p:?}");
                let local = to_local(theatre, at);
                assert!((local[0] - p[0]).hypot(local[1] - p[1]) < 1e-4);
            }
        }
    }

    #[test]
    fn forward_and_inverse_round_trip() {
        let globe = Globe::new([35.0, 25.0], 0.0004, 1200.0, 800.0);
        for (lat, lon) in [(35.0, 25.0), (40.0, 30.0), (10.0, -20.0), (-30.0, 80.0)] {
            let v = unit(lat, lon);
            let Some(p) = globe.forward(v) else {
                assert!(dot(v, globe.up) <= 0.0);
                continue;
            };
            let back = globe.inverse(p).unwrap();
            let [la, lo] = lat_lon(back);
            assert!(
                (la - lat).abs() < 1e-9 && (lo - lon).abs() < 1e-9,
                "{lat} {lon}"
            );
        }
        // The centre is the middle of the scene; off the disc is nothing; the far side is not drawn.
        let mid = globe.forward(unit(35.0, 25.0)).unwrap();
        assert!((mid[0] - 600.0).abs() < 1e-9 && (mid[1] - 400.0).abs() < 1e-9);
        assert!(
            globe
                .inverse([600.0 + globe.radius_px() + 1.0, 400.0])
                .is_none()
        );
        assert!(globe.forward(unit(-35.0, -155.0)).is_none());
        // North is up and east is right.
        let north = globe.forward(unit(36.0, 25.0)).unwrap();
        let east = globe.forward(unit(35.0, 26.0)).unwrap();
        assert!(north[1] < mid[1] && (north[0] - mid[0]).abs() < 1e-6);
        assert!(east[0] > mid[0]);
    }

    #[test]
    fn a_grab_keeps_its_point_under_the_pointer() {
        let globe = Globe::new([-9.5, 160.0], 0.0002, 1200.0, 800.0);
        let grabbed = globe.inverse([700.0, 350.0]).unwrap();
        for target in [
            [760.0, 330.0],
            [400.0, 600.0],
            [1100.0, 100.0],
            [610.0, 410.0],
        ] {
            let moved = globe.grab(grabbed, target);
            let at = moved.forward(grabbed).unwrap();
            assert!(
                (at[0] - target[0]).hypot(at[1] - target[1]) < 0.05,
                "{target:?}: {at:?}"
            );
        }
        // Far zoomed in, a long way in one frame.
        let close = Globe::new([35.0, 25.0], 0.05, 1600.0, 900.0);
        let grabbed = close.inverse([800.0, 450.0]).unwrap();
        let moved = close.grab(grabbed, [100.0, 850.0]);
        let at = moved.forward(grabbed).unwrap();
        assert!((at[0] - 100.0).hypot(at[1] - 850.0) < 0.05);
        // Off the disc the camera stays.
        let whole = Globe::new([0.0, 0.0], 1.0e-5, 1200.0, 800.0);
        let v = whole.inverse([600.0, 400.0]).unwrap();
        assert_eq!(whole.grab(v, [1190.0, 10.0]), whole);
        // The centre's latitude is held off the pole.
        let high = Globe::new([88.0, 0.0], 0.0001, 1200.0, 800.0);
        let v = high.inverse([600.0, 400.0]).unwrap();
        assert!(high.grab(v, [600.0, 700.0]).centre[0] <= MAX_LAT);
    }
}
