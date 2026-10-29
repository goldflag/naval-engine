//! The star shells' and searchlights' light on the scene: their real candela and colours, as `naval_render::PointLight`s
//! in real candela, which `tactical.rs`'s `Searchlight::light` and `Flare::light` scale into the scene's units by the
//! night (`naval_render::lights::candela_scale`, the host's `candela` on each).
//! The simulation's flares and lamps (`naval_sim::battle_effects`) say where they burn and how strongly (a lamp's power,
//! a candle's last seconds, a star shell's candle by its bore: the host's [`StarShellCandle`]); what they look like is
//! here. Sources are in `native/docs/lighting/lights.md`, "Lights at night".

use glam::{DVec3, Vec3};
use naval_render::lights::{LightCaster, PointLight};

/// A star shell's light, linear (brightest channel 1): a sodium nitrate and magnesium candle burns yellow-white, its
/// sodium lines over the white continuum of its burning magnesium and hot magnesia (Conkling and Mocella, "Chemistry of
/// Pyrotechnics", on illuminating compositions), taken as a 4,000 K source.
pub const STAR_SHELL_COLOR: Vec3 = Vec3::new(1.0, 0.659, 0.379);
/// A searchlight's beam, candela on its axis at full power: a 36-inch (91 cm) high-intensity arc lamp, the US and Royal
/// Navies' and the Germans' 90 cm. By its mirror's area between the German 60 cm lamp's 135 million candela and the
/// 150 cm's 990 million (Wikipedia, "German searchlights of World War II": 375–440 million a square metre), 340 million.
pub const SEARCHLIGHT_CANDELA: f64 = 3.4e8;
/// A searchlight's light, linear (brightest channel 1): the high-intensity carbon arc's crater and flame, about 5,500 K.
pub const SEARCHLIGHT_COLOR: Vec3 = Vec3::new(1.0, 0.851, 0.755);

/// A fresh star shell's candle, by the bore of the gun that fired it: the simulation's (`naval_sim::battle_effects`'
/// `star_shell_candela` and `star_shell_reach_m`), which the host gives each flare (`naval_effects_sim::star_shell_candle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StarShellCandle {
    /// Its light (cd).
    pub candela: f64,
    /// How far it lights (m), the reach the simulation fades as it burns out.
    pub reach_m: f64,
}

impl Default for StarShellCandle {
    /// A 5-inch's, the simulation's candle when the bore is unknown (6,500 cd a millimetre, lighting 3,000 m).
    fn default() -> Self {
        Self {
            candela: 825_500.0,
            reach_m: 3000.0,
        }
    }
}

/// A burning star shell at `position` whose candle burns fresh as `fresh`, its light faded as the simulation fades its
/// reach (`light_radius_m` against its full reach: the same inverse square at its last seconds), in real candela. It
/// casts down through a map of its own when it is among the brightest at the camera.
pub fn star_shell(position: DVec3, fresh: StarShellCandle, light_radius_m: f64) -> PointLight {
    let fade = (light_radius_m / fresh.reach_m).clamp(0.0, 1.0).powi(2);
    PointLight {
        position: position.as_vec3(),
        color: unit_luminance(STAR_SHELL_COLOR),
        intensity: (fresh.candela * fade) as f32,
        distance: 0.0,
        decay: 2.0,
        caster: LightCaster::Overhead,
        ..Default::default()
    }
}

/// A searchlight at `origin` trained along `direction` with the simulation's cone (`half_angle` radians) at `power`
/// (its share of full output, 0.2–1), in real candela on its axis. It casts along its beam through a map of its own when
/// it is among the brightest at the camera.
pub fn searchlight(origin: DVec3, direction: DVec3, half_angle: f64, power: f64) -> PointLight {
    PointLight {
        position: origin.as_vec3(),
        color: unit_luminance(SEARCHLIGHT_COLOR),
        intensity: (SEARCHLIGHT_CANDELA * power.clamp(0.0, 1.0)) as f32,
        distance: 0.0,
        decay: 2.0,
        caster: LightCaster::Beam,
        ..Default::default()
    }
    .with_cone(direction.as_vec3(), half_angle as f32)
}

/// `color` scaled to unit luminance, so a light's intensity is its luminous intensity (candela).
fn unit_luminance(color: Vec3) -> Vec3 {
    color / naval_render::lights::luminance(color).max(1e-6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_star_shell_burns_by_its_candle_and_fades_with_its_reach() {
        // A 5-inch's (its calibre's candle is the host's, `naval-effects-sim`'s tests).
        let candle = StarShellCandle {
            candela: 825_500.0,
            reach_m: 3000.0,
        };
        let five_inch = star_shell(DVec3::new(0.0, 500.0, 0.0), candle, 3000.0);
        assert!(
            (five_inch.power() - 825_500.0).abs() < 1.0,
            "{}",
            five_inch.power()
        );
        // Its reach halved (a quarter of its last 12 s left), a quarter of its light.
        let fading = star_shell(DVec3::ZERO, candle, 1500.0);
        assert!((fading.power() * 4.0 - five_inch.power()).abs() < 1.0);
        assert_eq!(five_inch.caster, LightCaster::Overhead);
    }

    #[test]
    fn a_searchlight_is_a_cone_at_its_lamp_power() {
        let lamp = searchlight(DVec3::ZERO, DVec3::Z, 0.026, 0.5);
        assert!((lamp.power() - 1.7e8).abs() < 1e3, "{}", lamp.power());
        assert_eq!(lamp.angular_attenuation(Vec3::Z), 1.0);
        assert_eq!(lamp.angular_attenuation(Vec3::X), 0.0);
        assert_eq!(lamp.caster, LightCaster::Beam);
    }
}
