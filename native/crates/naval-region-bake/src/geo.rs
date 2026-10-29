//! The region's frame: azimuthal equidistant about the scenario's origin on the theatre's sphere, x east, z south
//! (src/theatre/geo.ts, `naval_sim::theatre::geo`), in the bake's own arithmetic (the TypeScript bake's and the
//! removed battle maps' bake's `geographic`: latitude by asin, longitude continuous about the origin, never wrapped),
//! and web mercator for Terrarium.

use crate::EARTH_RADIUS_M;

const RAD: f64 = std::f64::consts::PI / 180.0;

/// An origin's sines and cosines, computed once for many points.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub lat: f64,
    pub lon: f64,
    sin_lat0: f64,
    cos_lat0: f64,
    lon0: f64,
}

impl Frame {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self {
            lat,
            lon,
            sin_lat0: (lat * RAD).sin(),
            cos_lat0: (lat * RAD).cos(),
            lon0: lon * RAD,
        }
    }

    /// The inverse projection of frame point (x east, z south): [lat, lon] in degrees, longitude continuous about the
    /// origin. The TypeScript bake's `geographic`, operation for operation.
    #[inline]
    pub fn geographic(&self, x: f64, z: f64) -> [f64; 2] {
        let north = -z;
        let rho = x.hypot(north);
        let c = rho / EARTH_RADIUS_M;
        let (sin_az, cos_az) = if rho > 0.0 {
            (x / rho, north / rho)
        } else {
            (0.0, 1.0)
        };
        let (sin_c, cos_c) = (c.sin(), c.cos());
        let sin_lat = self.sin_lat0 * cos_c + self.cos_lat0 * sin_c * cos_az;
        let lat = sin_lat.asin();
        let lon =
            self.lon0 + (sin_az * sin_c * self.cos_lat0).atan2(cos_c - self.sin_lat0 * lat.sin());
        [lat / RAD, lon / RAD]
    }

    /// The forward projection (src/theatre/geo.ts `toLocal`): [x, z] metres.
    pub fn to_local(&self, lat: f64, lon: f64) -> [f64; 2] {
        let (phi0, phi, d) = (self.lat * RAD, lat * RAD, (lon - self.lon) * RAD);
        let e = phi.cos() * d.sin();
        let n = phi0.cos() * phi.sin() - phi0.sin() * phi.cos() * d.cos();
        let u = phi0.sin() * phi.sin() + phi0.cos() * phi.cos() * d.cos();
        let s = e.hypot(n);
        let k = if s > 0.0 {
            EARTH_RADIUS_M * s.atan2(u) / s
        } else {
            EARTH_RADIUS_M
        };
        [k * e, -k * n]
    }
}

/// Web mercator's x at `scale` pixels round the world.
#[inline]
pub fn mercator_x(lon: f64, scale: f64) -> f64 {
    (lon + 180.0) / 360.0 * scale
}

/// Web mercator's y at `scale` pixels round the world.
#[inline]
pub fn mercator_y(lat: f64, scale: f64) -> f64 {
    let phi = lat * RAD;
    (1.0 - (phi.tan() + 1.0 / phi.cos()).ln() / std::f64::consts::PI) / 2.0 * scale
}

/// The latitude of web-mercator y at `scale` pixels round the world.
pub fn mercator_lat(y: f64, scale: f64) -> f64 {
    (std::f64::consts::PI * (1.0 - 2.0 * y / scale))
        .sinh()
        .atan()
        / RAD
}

/// Catmull-Rom weights for a fraction `t` across an interval.
#[inline]
pub fn catmull_rom(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        0.5 * (-t3 + 2.0 * t2 - t),
        0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
        0.5 * (-3.0 * t3 + 4.0 * t2 + t),
        0.5 * (t3 - t2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_projection_round_trips() {
        let frame = Frame::new(36.936204, -122.418494);
        let [lat, lon] = frame.geographic(123_456.0, -98_765.0);
        let [x, z] = frame.to_local(lat, lon);
        assert!((x - 123_456.0).abs() < 1e-6 && (z + 98_765.0).abs() < 1e-6);
    }
}
