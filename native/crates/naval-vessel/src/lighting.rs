//! The scene's light as the ships take it ([`VesselLighting`]): the sun or the moon from the sky, the sky's environment
//! (bound to the layer), a lightning flash's fill, the air's haze, and the scale real candela take in the scene for
//! her lamps and lit windows. naval-ships' `ShipLighting` less its water: the hulls here are lit as in air.

use glam::Vec3;
use naval_render::SceneFog;
use naval_sky::{CelestialLight, Sky};

/// `#dcebf2` and `#65757e` in linear light, the hemisphere fill's sky and ground (naval-ships' `mesh_fill_colors`).
pub fn fill_colors() -> (Vec3, Vec3) {
    let srgb = |c: u32| {
        let f = |v: u32| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        Vec3::new(f(c >> 16), f((c >> 8) & 0xff), f(c & 0xff))
    };
    (srgb(0xdcebf2), srgb(0x65757e))
}

/// The hemisphere fill a lightning flash lends, a multiple of the scene's ambient per unit of flash (naval-ships'
/// `LIGHTNING_FILL`).
pub const LIGHTNING_FILL: f32 = 1.6;

/// What an emissive map of 1 shines at (candela a square metre): a lit window, a lamp's glass.
pub const EMISSIVE_LUMINANCE: f32 = 20.0;

/// The light the layer draws by. A host calls [`VesselLighting::follow`] every frame with the sky's light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VesselLighting {
    /// The sun by day, the moon by night (`Sky::light`).
    pub light: CelestialLight,
    /// Hemisphere fill colours (linear) and its intensity: 0 but under a lightning flash.
    pub fill_sky: Vec3,
    pub fill_ground: Vec3,
    pub fill_intensity: f32,
    /// The sky's environment light's share (1).
    pub environment_intensity: f32,
    /// The air's haze between her and the eye: on when `fog.enabled` (the aerial perspective the sky writes).
    pub fog: SceneFog,
    /// The scene's intensity for one real candela ([`candela_scale`]): her lamps' and emissive surfaces' light.
    pub candela: f32,
    /// What an emissive map of 1 shines at (cd/m²).
    pub emissive_luminance: f32,
}

impl VesselLighting {
    /// The light of `light` as it is, no flash, no haze, a lamp's candela at the sun's scale.
    pub fn daylight(light: CelestialLight) -> Self {
        let (sky, ground) = fill_colors();
        Self {
            light,
            fill_sky: sky,
            fill_ground: ground,
            fill_intensity: 0.0,
            environment_intensity: 1.0,
            fog: SceneFog::default(),
            candela: 1.0 / naval_render::lights::DAY_LUX,
            emissive_luminance: EMISSIVE_LUMINANCE,
        }
    }

    /// This frame's light from the sky: its sun or moon, a lightning `flash`'s fill under the scene's `ambient`, and
    /// the night's lift on lamps.
    pub fn follow(&mut self, sky: &Sky, ambient: f32, flash: f32) {
        self.light = sky.light();
        self.fill_intensity = ambient * LIGHTNING_FILL * flash;
        self.candela = candela_scale(sky);
    }
}

/// The scene's intensity for one real candela under `sky` (`naval_render::lights::candela_scale` with the game's full
/// moon, as the client lights its star shells and searchlights): the sun's scale by day, lifted with the night as the
/// moon is.
pub fn candela_scale(sky: &Sky) -> f32 {
    let elevation = sky.sun_direction().y.clamp(-1.0, 1.0).asin().to_degrees();
    let full_moon = naval_render::lights::full_moon_scene(
        naval_sky::model::moonlight_at(f64::from(elevation)) as f32,
        naval_sky::model::moon_grade::TRANSMITTANCE as f32,
        naval_sky::model::sky_grade::GAIN as f32,
        naval_sky::state::moon_tint().as_vec3(),
    );
    naval_render::lights::candela_scale(elevation, full_moon)
}
