//! The theatre's local frame: metres, `x` east and `z` south, on an azimuthal-equidistant projection about the
//! scenario's origin on a sphere of radius [`EARTH_RADIUS_M`]. `src/theatre/geo.ts` implements the same formulas
//! term for term; both are held to `src/theatre/fixtures/geo-table.json` to the millimetre
//! (`tests/theatre_geo.rs` and `geo.test.ts`).
//!
//! Forward (`to_local`), with `φ0, λ0` the origin and `Δλ = λ − λ0`:
//! `e = cos φ sin Δλ`, `n = cos φ0 sin φ − sin φ0 cos φ cos Δλ`, `u = sin φ0 sin φ + cos φ0 cos φ cos Δλ`,
//! the angular distance `c = atan2(hypot(e, n), u)`, `k = R c / hypot(e, n)`, `x = k e`, `z = −k n`.
//!
//! Inverse (`to_lat_lon`), with `ρ = hypot(x, z)`, `c = ρ / R`, `sin θ = x / ρ`, `cos θ = −z / ρ`:
//! `px = cos c cos φ0 − sin c cos θ sin φ0`, `py = sin c sin θ`, `pz = cos c sin φ0 + sin c cos θ cos φ0`,
//! `φ = atan2(pz, hypot(px, py))`, `λ = λ0 + atan2(py, px)`, wrapped to [−180, 180).
//!
//! Directions on the chart are grid bearings ([`bearing_deg`]), chart numbers; the theatre steers on true headings at
//! its places (`earth`). Bearings a side is told (a contact's bearing, a report's text) are true bearings
//! ([`true_bearing_deg`]), the initial great-circle course, which differs from the grid bearing away from the origin's
//! meridian (1.9° at 64.6°N, 100 km east of it).
pub use crate::EARTH_RADIUS_M;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Degrees north and east.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(rename = "TheatreLatLon")]
pub struct LatLon {
    pub lat: f64,
    pub lon: f64,
}
impl LatLon {
    pub fn valid(&self) -> bool {
        self.lat.is_finite()
            && self.lon.is_finite()
            && (-89.0..=89.0).contains(&self.lat)
            && (-180.0..=180.0).contains(&self.lon)
    }
}

/// Longitude wrapped to [−180, 180).
pub fn wrap_lon(lon: f64) -> f64 {
    if (-180.0..180.0).contains(&lon) {
        return lon;
    }
    let wrapped = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if wrapped >= 180.0 { -180.0 } else { wrapped }
}

/// `[x east, z south]` metres of `p` in the frame about `origin`.
pub fn to_local(origin: LatLon, p: LatLon) -> [f64; 2] {
    let (phi0, phi) = (origin.lat.to_radians(), p.lat.to_radians());
    let dl = (p.lon - origin.lon).to_radians();
    let (sin0, cos0) = phi0.sin_cos();
    let (sin_p, cos_p) = phi.sin_cos();
    let cos_d = dl.cos();
    let e = cos_p * dl.sin();
    let n = cos0 * sin_p - sin0 * cos_p * cos_d;
    let u = sin0 * sin_p + cos0 * cos_p * cos_d;
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

/// The latitude and longitude (longitude in [−180, 180)) of the local point `[x, z]` about `origin`.
pub fn to_lat_lon(origin: LatLon, p: [f64; 2]) -> LatLon {
    let (x, z) = (p[0], p[1]);
    let rho = x.hypot(z);
    if rho == 0.0 {
        return LatLon {
            lat: origin.lat,
            lon: wrap_lon(origin.lon),
        };
    }
    let phi0 = origin.lat.to_radians();
    let c = rho / EARTH_RADIUS_M;
    let (sin0, cos0) = phi0.sin_cos();
    let (sin_c, cos_c) = c.sin_cos();
    let (sin_t, cos_t) = (x / rho, -z / rho);
    let px = cos_c * cos0 - sin_c * cos_t * sin0;
    let py = sin_c * sin_t;
    let pz = cos_c * sin0 + sin_c * cos_t * cos0;
    LatLon {
        lat: pz.atan2(px.hypot(py)).to_degrees(),
        lon: wrap_lon(origin.lon + py.atan2(px).to_degrees()),
    }
}

/// True bearing in degrees [0, 360): the initial great-circle course from `a` to `b` (0 when they coincide).
/// The great-circle distance between two points, in metres (haversine, on the frame's sphere).
pub fn great_circle_m(a: LatLon, b: LatLon) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dp = p2 - p1;
    let dl = (b.lon - a.lon).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().min(1.0).asin()
}

pub fn initial_bearing_deg(a: LatLon, b: LatLon) -> f64 {
    let (phi1, phi2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let y = dl.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dl.cos();
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    let deg = y.atan2(x).to_degrees().rem_euclid(360.0);
    if deg >= 360.0 { 0.0 } else { deg }
}
/// True bearing in degrees [0, 360) from local point `from` to `to` in the frame about `origin`: the initial
/// great-circle course, what a ship steers and a report gives. `bearingDeg` in `geo.ts`.
pub fn true_bearing_deg(origin: LatLon, from: [f64; 2], to: [f64; 2]) -> f64 {
    initial_bearing_deg(to_lat_lon(origin, from), to_lat_lon(origin, to))
}

/// The point `metres` along the great circle leaving `from` on true course `course_deg`.
pub fn destination(from: LatLon, course_deg: f64, metres: f64) -> LatLon {
    let d = metres / EARTH_RADIUS_M;
    let (phi1, lam1) = (from.lat.to_radians(), from.lon.to_radians());
    let theta = course_deg.to_radians();
    let phi2 = (phi1.sin() * d.cos() + phi1.cos() * d.sin() * theta.cos())
        .clamp(-1.0, 1.0)
        .asin();
    let lam2 = lam1 + (theta.sin() * d.sin() * phi1.cos()).atan2(d.cos() - phi1.sin() * phi2.sin());
    LatLon {
        lat: phi2.to_degrees(),
        lon: wrap_lon(lam2.to_degrees()),
    }
}
/// The grid course, degrees [0, 360), at local point `at` (in the frame about `origin`) of the true course `true_deg`:
/// the direction on the chart a ship steering `true_deg` from there first makes good. The inverse of
/// [`true_bearing_deg`] at that point: the two differ away from the origin's meridian (the meridians converge) and,
/// slightly, with distance from the origin (the projection keeps distances from the origin, not angles).
pub fn grid_course_deg(origin: LatLon, at: [f64; 2], true_deg: f64) -> f64 {
    // Far enough for the frame's metres, near enough that the great circle is straight on the chart.
    const STEP_M: f64 = 100.0;
    let ahead = destination(to_lat_lon(origin, at), true_deg, STEP_M);
    bearing_deg(at, to_local(origin, ahead))
}
/// Grid bearing in degrees [0, 360) from `from` toward `to`, both local: the direction on the flat chart, clockwise
/// from the chart's up (−z). The theatre steers by it; a bearing a side is told is [`true_bearing_deg`].
pub fn bearing_deg(from: [f64; 2], to: [f64; 2]) -> f64 {
    let d = [to[0] - from[0], to[1] - from[1]];
    (d[0].atan2(-d[1]).to_degrees()).rem_euclid(360.0)
}
/// Distance in metres between two local points.
pub fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
/// The unit vector of a true heading in radians (the battle's heading convention: 0 north, clockwise).
pub fn heading_vector(heading: f64) -> [f64; 2] {
    [heading.sin(), -heading.cos()]
}
/// Round a local point to the metre.
pub fn round_m(p: [f64; 2]) -> [f64; 2] {
    [p[0].round(), p[1].round()]
}

#[cfg(test)]
mod tests {
    use super::*;
    const ORIGIN: LatLon = LatLon {
        lat: 64.6,
        lon: -28.4,
    };
    /// The grid course of a true course is its inverse: the true bearing along it is the course steered, wherever on
    /// the chart; on the origin's meridian they agree, and far east of it at a high latitude they differ widely.
    #[test]
    fn a_grid_course_steers_the_true_course() {
        for at in [
            [0.0, 0.0],
            [0.0, 400_000.0],
            [700_000.0, -600_000.0],
            [-900_000.0, 300_000.0],
        ] {
            for course in [0.0, 45.0, 90.0, 200.0, 315.0] {
                let grid = grid_course_deg(ORIGIN, at, course);
                let v = heading_vector(grid.to_radians());
                let ahead = [at[0] + v[0] * 1_000.0, at[1] + v[1] * 1_000.0];
                let made = true_bearing_deg(ORIGIN, at, ahead);
                let off = (made - course + 540.0).rem_euclid(360.0) - 180.0;
                assert!(off.abs() < 0.01, "{at:?} {course}: {made}");
            }
        }
        assert!((grid_course_deg(ORIGIN, [0.0, 0.0], 90.0) - 90.0).abs() < 1e-6);
        assert!((grid_course_deg(ORIGIN, [700_000.0, -600_000.0], 90.0) - 90.0).abs() > 10.0);
    }
    #[test]
    fn the_origin_is_zero_and_axes_point_east_and_south() {
        assert_eq!(to_local(ORIGIN, ORIGIN), [0.0, 0.0]);
        let north = to_local(
            ORIGIN,
            LatLon {
                lat: 65.6,
                lon: -28.4,
            },
        );
        assert!(north[0].abs() < 1e-6 && north[1] < 0.0);
        // One degree of latitude along the meridian is R·π/180.
        assert!((north[1] + EARTH_RADIUS_M * std::f64::consts::PI / 180.0).abs() < 1e-6);
        let east = to_local(
            ORIGIN,
            LatLon {
                lat: 64.6,
                lon: -27.4,
            },
        );
        assert!(east[0] > 0.0);
    }
    #[test]
    fn local_and_geographic_round_trip_to_the_millimetre() {
        for (lat, lon) in [
            (62.9, -24.5),
            (66.6, -25.5),
            (60.5, -36.0),
            (64.6, -28.4),
            (-8.0, 161.5),
        ] {
            let p = LatLon { lat, lon };
            let local = to_local(ORIGIN, p);
            let back = to_local(ORIGIN, to_lat_lon(ORIGIN, local));
            assert!(distance(local, back) < 1e-3, "{p:?}: {local:?} vs {back:?}");
        }
    }
    #[test]
    fn grid_bearings_run_clockwise_from_the_charts_up() {
        assert_eq!(bearing_deg([0.0, 0.0], [0.0, -1.0]), 0.0);
        assert_eq!(bearing_deg([0.0, 0.0], [1.0, 0.0]), 90.0);
        assert_eq!(bearing_deg([0.0, 0.0], [0.0, 1.0]), 180.0);
        assert_eq!(bearing_deg([0.0, 0.0], [-1.0, 0.0]), 270.0);
    }
    #[test]
    fn true_bearings_part_from_grid_bearings_away_from_the_origins_meridian() {
        // Along the origin's meridian the chart's up is true north.
        assert!(true_bearing_deg(ORIGIN, [0.0, 50_000.0], [0.0, -50_000.0]).abs() < 1e-9);
        // 100 km east of it at 64.6°N, the chart's up is 1.9° east of true north.
        let up = true_bearing_deg(ORIGIN, [100_000.0, 0.0], [100_000.0, -10_000.0]);
        assert!((up - 1.9).abs() < 0.1, "{up}");
        assert_eq!(bearing_deg([100_000.0, 0.0], [100_000.0, -10_000.0]), 0.0);
    }
    #[test]
    fn longitudes_wrap_to_the_half_open_range() {
        assert_eq!(wrap_lon(180.0), -180.0);
        assert_eq!(wrap_lon(-180.0), -180.0);
        assert_eq!(wrap_lon(190.0), -170.0);
        assert_eq!(wrap_lon(-190.0), 170.0);
        assert_eq!(wrap_lon(540.0), -180.0);
    }
}
