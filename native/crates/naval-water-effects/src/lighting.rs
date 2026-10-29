//! The scene light every spray material reads (the web game's `EffectLighting.ts`): the direction toward the sun (or the moon
//! when it is the brighter light), the direct radiance spray scatters, the sky fill in its shade, the scene's daylight
//! for unlit drops, and the wind that carries spray and mist.
//!
//! The native host feeds it what `VisualEnvironment.syncLighting` fed the removed web game's: the sky's celestial
//! light (the sky environment's `light_direction`, `light_color` and `light_intensity`, as the ocean reads them) and the
//! scene's authored ambient (the map's `ambient`, 1.75 in port), through [`EffectLighting::from_sky`], and the ocean's
//! wind through [`EffectLighting::set_wind`].

use bytemuck::{Pod, Zeroable};
use glam::DVec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectLighting {
    /// Toward the sun, or toward the moon when it is the brighter light (unit).
    pub sun_direction: DVec3,
    /// Direct radiance scattered by spray: celestial colour × intensity, about 1.25 at a clear noon.
    pub direct: DVec3,
    /// Sky fill that reaches shaded folds, about (0.3, 0.35, 0.4) by day.
    pub ambient: DVec3,
    /// 0–1 overall scene brightness, for unlit drops and mist where the sky's environment is not bound.
    pub daylight: f64,
    /// Lightning's flash, a multiple of the sky's light on spray (0 without).
    pub flash: f64,
    /// World drift for spray in m/s: the ocean's wind × 0.35, radians from +X toward +Z.
    pub wind: DVec3,
}

impl Default for EffectLighting {
    fn default() -> Self {
        Self {
            sun_direction: DVec3::new(-0.55, 0.74, -0.39).normalize(),
            direct: DVec3::new(1.25, 1.19, 1.08),
            ambient: DVec3::new(0.3, 0.35, 0.4),
            daylight: 1.0,
            flash: 0.0,
            wind: DVec3::new(2.4, 0.0, 0.9),
        }
    }
}

impl EffectLighting {
    /// `setWind`: `speed` m/s toward `direction` radians from +X toward +Z (the ocean's convention).
    pub fn set_wind(&mut self, speed: f64, direction: f64) {
        self.wind = DVec3::new(direction.cos(), 0.0, direction.sin()) * (speed * 0.35);
    }

    /// `setSun`.
    pub fn set_sun(&mut self, direction: DVec3, daylight: f64) {
        self.sun_direction = direction;
        self.daylight = daylight;
    }

    /// `setIllumination`: the celestial light's linear colour and intensity, and the scene's ambient.
    pub fn set_illumination(&mut self, color: DVec3, intensity: f64, ambient: f64) {
        self.direct = color * (intensity.max(0.0) * 1.25 / 5.8);
        self.ambient = DVec3::new(0.3, 0.35, 0.4) * (ambient.max(0.0) / 1.75);
    }

    /// What `VisualEnvironment.syncLighting` sets from the sky: its light (`direction`, linear `color`, `intensity`,
    /// whether it is the moon) and the scene's authored `ambient` (1.75 in port, the map's in battle; a lightning flash
    /// multiplies it by 1 + flash). In port the daylight is 1.
    pub fn from_sky(
        direction: DVec3,
        color: DVec3,
        intensity: f64,
        night: bool,
        ambient: f64,
        in_port: bool,
    ) -> Self {
        let mut lighting = Self::default();
        let daylight = if in_port {
            1.0
        } else if night {
            (0.18 + ambient * 0.5).min(1.0)
        } else {
            (ambient * 0.45 + intensity * 0.09).min(1.0)
        };
        lighting.set_sun(direction, daylight);
        lighting.set_illumination(color, intensity, ambient);
        lighting
    }

    pub fn uniform(&self) -> LightingUniform {
        let sun = self.sun_direction.as_vec3();
        LightingUniform {
            sun_direction: [sun.x, sun.y, sun.z, self.daylight as f32],
            direct: [
                self.direct.x as f32,
                self.direct.y as f32,
                self.direct.z as f32,
                0.0,
            ],
            ambient: [
                self.ambient.x as f32,
                self.ambient.y as f32,
                self.ambient.z as f32,
                self.flash as f32,
            ],
            column_sampling: [0.0; 4],
        }
    }
}

/// `EffectLighting` in `shaders/spray.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct LightingUniform {
    /// xyz toward the sun, w daylight.
    pub sun_direction: [f32; 4],
    pub direct: [f32; 4],
    pub ambient: [f32; 4],
    /// x the full-resolution pixels per column march texel (1 or 2), set each frame by the layer.
    pub column_sampling: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clear_noon_lights_spray_as_the_web() {
        // The web's noon light: colour ≈ (1, .947, .844), intensity 5.8 gives direct = colour × 1.25.
        let lighting = EffectLighting::from_sky(
            DVec3::Y,
            DVec3::new(1.0, 0.947, 0.844),
            5.8,
            false,
            1.75,
            false,
        );
        assert!((lighting.direct - DVec3::new(1.25, 1.18375, 1.055)).length() < 1e-9);
        assert!((lighting.ambient - DVec3::new(0.3, 0.35, 0.4)).length() < 1e-12);
        assert_eq!(lighting.daylight, 1.0);
    }
}
