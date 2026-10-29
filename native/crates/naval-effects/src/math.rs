//! Small numeric helpers shared by the effect systems, written as the web client writes them so ported
//! recipes carry over unchanged. The CPU side works in `f64` like JavaScript; only instance data sent to the
//! GPU is narrowed to `f32`.

use glam::DVec3;

/// `THREE.MathUtils.clamp`.
pub fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.max(min).min(max)
}

/// The pools' `smooth`: Hermite of `n` clamped to 0–1.
pub fn smooth(n: f64) -> f64 {
    let t = clamp(n, 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `THREE.MathUtils.smoothstep(x, min, max)` (note the argument order: value first).
pub fn smoothstep(x: f64, min: f64, max: f64) -> f64 {
    if x <= min {
        return 0.0;
    }
    if x >= max {
        return 1.0;
    }
    let t = (x - min) / (max - min);
    t * t * (3.0 - 2.0 * t)
}

/// A linear RGB colour as three's `Color` holds it.
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const WHITE: Rgb = Rgb::new(1.0, 1.0, 1.0);

    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    /// `new THREE.Color('#rrggbb')`: an sRGB hex colour converted into the linear working space.
    pub fn hex(hex: u32) -> Self {
        let channel = |shift: u32| srgb_to_linear(f64::from((hex >> shift) & 0xff) / 255.0);
        Self::new(channel(16), channel(8), channel(0))
    }

    pub fn scale(self, s: f64) -> Self {
        Self::new(self.r * s, self.g * s, self.b * s)
    }

    /// `Color.lerp`.
    pub fn lerp(self, to: Rgb, alpha: f64) -> Self {
        Self::new(
            self.r + (to.r - self.r) * alpha,
            self.g + (to.g - self.g) * alpha,
            self.b + (to.b - self.b) * alpha,
        )
    }

    pub fn to_f32(self) -> [f32; 3] {
        [self.r as f32, self.g as f32, self.b as f32]
    }

    /// A black body's light at `kelvin` in linear sRGB, in the sun's units (`naval_render::lights::DAY_LUX` cd/m² a
    /// unit by day: sunlit grey paint is about 0.57): Planck's law against the CIE 1931 observer, fitted over the heats
    /// of burning soot within a few per cent: 650 cd/m² at 1,300 K, 7,800 at 1,500 K, 21,500 at 1,600 K, near the old
    /// candela's 600,000 at the platinum point's 2,042 K. Its green rises from a tenth of its red at 1,300 K to 0.17 at
    /// 1,600 K; its blue is next to none at these heats (a little below zero, outside the gamut: none). The twin of
    /// `shaders/common.wgsl`'s `black_body`.
    pub fn black_body(kelvin: f64) -> Self {
        let kelvin = kelvin.max(500.0);
        let luminance = 7777.0 / f64::from(naval_render::lights::DAY_LUX)
            * (24_200.0 * (1.0 / 1500.0 - 1.0 / kelvin)).exp();
        let green = (2.42e-4 * (kelvin - 900.0)).max(0.0);
        Self::new(1.0, green, 0.0).scale(luminance / (0.2126 + 0.7152 * green))
    }
}

/// three's `SRGBToLinear`.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c < 0.04045 {
        c * 0.077_399_380_8
    } else {
        (c * 0.947_867_298_6 + 0.052_132_701_4).powf(2.4)
    }
}

/// A `[x, y, z]` array as a vector.
pub fn v3(a: [f64; 3]) -> DVec3 {
    DVec3::from_array(a)
}

/// `Vector3.normalize()`: a zero vector stays zero (three divides by `length || 1`).
pub fn normalize(v: DVec3) -> DVec3 {
    let length = v.length();
    if length > 0.0 { v / length } else { v }
}

/// A ship pose in the simulation's frame: metres, world Y up, heading clockwise from above with 0 steaming
/// toward −Z. `local_to_world` is `src/game/geometry.ts`'s (three's Euler YXZ with yaw = −heading).
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipPose {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
}

impl ShipPose {
    pub fn position(&self) -> DVec3 {
        DVec3::new(self.x, self.y, self.z)
    }

    /// `rotate(v, pose)`.
    pub fn rotate(&self, v: DVec3) -> DVec3 {
        let (sr, cr) = self.roll.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sh, ch) = self.heading.sin_cos();
        let x = cr * v.x - sr * v.y;
        let y = sr * v.x + cr * v.y;
        let yy = cp * y - sp * v.z;
        let z = sp * y + cp * v.z;
        DVec3::new(ch * x - sh * z, yy, sh * x + ch * z)
    }

    /// `localToWorld(v, pose)`.
    pub fn local_to_world(&self, v: DVec3) -> DVec3 {
        self.rotate(v) + self.position()
    }

    /// `worldToLocal(v, pose)`.
    pub fn world_to_local(&self, v: DVec3) -> DVec3 {
        let p = v - self.position();
        let (sh, ch) = self.heading.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        let x = ch * p.x + sh * p.z;
        let zz = -sh * p.x + ch * p.z;
        let y = cp * p.y + sp * zz;
        let z = -sp * p.y + cp * zz;
        DVec3::new(cr * x + sr * y, -sr * x + cr * y, z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colours_are_linearised_like_three() {
        // new THREE.Color('#9c9fa5') in the browser: 0.33245…, 0.34670…, 0.37626…
        let c = Rgb::hex(0x9c9fa5);
        assert!((c.r - 0.332_451_9).abs() < 1e-6, "{c:?}");
        assert!((c.g - 0.346_704_5).abs() < 1e-6, "{c:?}");
        assert!((c.b - 0.376_262_0).abs() < 1e-6, "{c:?}");
    }

    #[test]
    fn black_bodies_shine_as_planck_has_them() {
        // Planck's law integrated against the CIE 1931 observer: kelvin, cd/m² and green over red in linear sRGB.
        let day = f64::from(naval_render::lights::DAY_LUX);
        for (kelvin, candela, green) in [
            (1300.0, 650.0, 0.10),
            (1400.0, 2450.0, 0.12),
            (1500.0, 7800.0, 0.145),
            (1600.0, 21_500.0, 0.17),
            (1800.0, 118_000.0, 0.22),
            (2042.0, 600_000.0, 0.28),
        ] {
            let c = Rgb::black_body(kelvin);
            let luminance = (0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b) * day;
            assert!(
                (luminance / candela - 1.0).abs() < 0.08,
                "{kelvin} K: {luminance} cd/m²"
            );
            assert!((c.g / c.r - green).abs() < 0.012, "{kelvin} K: {c:?}");
            assert_eq!(c.b, 0.0);
        }
    }

    #[test]
    fn world_and_local_round_trip() {
        let pose = ShipPose {
            x: 12.0,
            y: -0.4,
            z: -300.0,
            heading: 0.7,
            pitch: 0.02,
            roll: -0.05,
        };
        let local = DVec3::new(5.0, 9.0, -60.0);
        let back = pose.world_to_local(pose.local_to_world(local));
        assert!((back - local).length() < 1e-9);
        // Heading 90° (east) turns the bow (−Z) toward +X.
        let east = ShipPose {
            heading: std::f64::consts::FRAC_PI_2,
            ..Default::default()
        };
        assert!((east.rotate(DVec3::new(0.0, 0.0, -1.0)) - DVec3::X).length() < 1e-12);
    }
}
