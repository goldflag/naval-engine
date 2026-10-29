//! The sky's renderer-free state: the scene description, where the sun, moon and stars stand, the CPU
//! values the shaders read, and the celestial light the scene shares.
//!
//! Ports the web game's `sky/contracts.ts` (`SkyScene`), `celestialModel.ts`, `state.ts`, `celestial/bodies.ts`
//! and `celestial/galactic.ts`.

use glam::{DMat3, DVec3};

use crate::model::{AuthoredAtmosphere, smoothstep, srgb_to_linear};

pub use naval_sky_types::{CloudGenus, HighCloudGenus, HighClouds};

/// `SkyScene.clouds`: the volumetric layer and the high sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CloudScene {
    /// 0 clear … 1 overcast. A shape control, not a measured sky fraction.
    pub coverage: f64,
    /// Cloud base above the sea (m).
    pub altitude: f64,
    /// Depth of the cloud layer (m).
    pub thickness: f64,
    /// Extra coverage toward the horizon.
    pub horizon_coverage: f64,
    /// Gain on the sky light filling the clouds (authored against another model: 1.1 is a gain of 1).
    pub ambient: f64,
    /// Darkening of cloud bases, 0–1.
    pub base_shadow: f64,
    /// Drift speed of the cloud field (m/s).
    pub wind_speed: f64,
    /// Drift heading, compass degrees the clouds move toward.
    pub wind_heading: f64,
    /// The layer's genus; none lets the cover decide.
    pub genus: Option<CloudGenus>,
    /// The high sheet; none is the sky's own cirrus, which grows with the cover.
    pub high: Option<HighClouds>,
    /// Fog banks lying on the sea: 0 none … 1 thick banks (the fog weather).
    pub fog: f64,
    /// Whether the scene chose the base and the depth (a genus, the battle rail's base, the console): the clouds then
    /// stand there; otherwise the weather puts its own layers where its clouds stand.
    pub base_chosen: bool,
    pub depth_chosen: bool,
}

/// `SkyScene.weather`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeatherScene {
    /// Rain from 0 (none) to 1 (a downpour under storm cells).
    pub precipitation: f64,
    /// Lightning strikes per minute within sight; 0 for none.
    pub lightning: f64,
}

/// A place in the sky: degrees above the horizon (negative below it) and compass degrees, 0 toward +Z, 90 toward +X.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyAngles {
    pub elevation: f64,
    pub azimuth: f64,
}

/// What a scene asks of the sky (`SkyScene`). The sky itself draws the atmosphere and the bodies and
/// drifts the clouds with `clouds.wind_*`; the clouds and weather (`naval-clouds`) read the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyScene {
    /// Degrees above the horizon; negative below it.
    pub sun_elevation: f64,
    /// Compass degrees: 0 toward +Z, 90 toward +X.
    pub sun_azimuth: f64,
    /// Peak daytime radiance scale the sea and scene light share (about 6 in daylight).
    pub sun_intensity: f64,
    /// 0 new, 0.5 full, 1 new again.
    pub moon_phase: f64,
    /// Where the moon stands, for a scene that knows (a theatre's, by its date and place). None puts it along the
    /// sun's arc by its phase: opposite the sun when full.
    pub moon: Option<SkyAngles>,
    pub atmosphere: AuthoredAtmosphere,
    pub clouds: CloudScene,
    pub weather: WeatherScene,
    /// The port: sheltered daylight.
    pub port: bool,
}

/// The port's afternoon (`DEFAULT_SKY_SCENE`).
pub const DEFAULT_SKY_SCENE: SkyScene = SkyScene {
    sun_elevation: 36.0,
    sun_azimuth: 58.0,
    sun_intensity: 5.8,
    moon_phase: 0.5,
    moon: None,
    atmosphere: AuthoredAtmosphere {
        rayleigh: 0.42,
        turbidity: 3.2,
        mie: 0.25,
        mie_g: 0.6,
        multiple: 1.4,
    },
    clouds: CloudScene {
        coverage: 0.38,
        altitude: 1700.0,
        thickness: 2400.0,
        horizon_coverage: 0.06,
        ambient: 1.1,
        base_shadow: 0.2,
        wind_speed: 12.0,
        wind_heading: 55.0,
        genus: None,
        high: None,
        fog: 0.0,
        base_chosen: false,
        depth_chosen: false,
    },
    weather: WeatherScene {
        precipitation: 0.0,
        lightning: 0.0,
    },
    port: true,
};

impl SkyScene {
    /// The North Atlantic map's sky in a battle (`assets/maps/environments.v1.json`, weather "map"), as
    /// `VisualEnvironment` hands it to the sky: the authored aureole at 0.45 of the map's.
    pub fn north_atlantic(sun_elevation: f64, sun_azimuth: f64) -> Self {
        Self {
            sun_elevation,
            sun_azimuth,
            sun_intensity: 6.6,
            moon_phase: 0.5,
            moon: None,
            atmosphere: AuthoredAtmosphere {
                rayleigh: 0.38,
                turbidity: 2.2,
                mie: 0.5 * 0.45,
                mie_g: 0.72,
                multiple: 1.0,
            },
            // The map's fair-weather cumulus, as the game's sky review applies it (read back 2026-09-30).
            clouds: CloudScene {
                coverage: 0.4,
                ..DEFAULT_SKY_SCENE.clouds
            },
            weather: DEFAULT_SKY_SCENE.weather,
            port: false,
        }
    }

    /// The scene `t` of the way from `self` to `other` (a weather changing): every quantity between the two, angles the
    /// short way round; what has no between (the air, whose tables take a precompute; a genus, the high sheet's kind,
    /// the port) is `self`'s for the first half and `other`'s for the second, so the air changes once.
    pub fn lerp(&self, other: &SkyScene, t: f64) -> SkyScene {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f64, b: f64| a + (b - a) * t;
        let angle = |a: f64, b: f64| a + ((b - a + 540.0).rem_euclid(360.0) - 180.0) * t;
        let late = t >= 0.5;
        let (a, b) = (&self.clouds, &other.clouds);
        let high = match (a.high, b.high) {
            (Some(x), Some(y)) if x.genus == y.genus => Some(HighClouds {
                genus: x.genus,
                amount: mix(x.amount, y.amount),
                altitude: mix(x.altitude, y.altitude),
            }),
            (x, y) => {
                if late {
                    y
                } else {
                    x
                }
            }
        };
        SkyScene {
            sun_elevation: mix(self.sun_elevation, other.sun_elevation),
            sun_azimuth: angle(self.sun_azimuth, other.sun_azimuth).rem_euclid(360.0),
            sun_intensity: mix(self.sun_intensity, other.sun_intensity),
            moon_phase: mix(self.moon_phase, other.moon_phase),
            moon: match (self.moon, other.moon) {
                (Some(a), Some(b)) => Some(SkyAngles {
                    elevation: mix(a.elevation, b.elevation),
                    azimuth: angle(a.azimuth, b.azimuth).rem_euclid(360.0),
                }),
                (a, b) => {
                    if late {
                        b
                    } else {
                        a
                    }
                }
            },
            atmosphere: if late {
                other.atmosphere
            } else {
                self.atmosphere
            },
            clouds: CloudScene {
                coverage: mix(a.coverage, b.coverage),
                altitude: mix(a.altitude, b.altitude),
                thickness: mix(a.thickness, b.thickness),
                horizon_coverage: mix(a.horizon_coverage, b.horizon_coverage),
                ambient: mix(a.ambient, b.ambient),
                base_shadow: mix(a.base_shadow, b.base_shadow),
                wind_speed: mix(a.wind_speed, b.wind_speed),
                wind_heading: angle(a.wind_heading, b.wind_heading).rem_euclid(360.0),
                genus: if late { b.genus } else { a.genus },
                high,
                fog: mix(a.fog, b.fog),
                base_chosen: if late { b.base_chosen } else { a.base_chosen },
                depth_chosen: if late { b.depth_chosen } else { a.depth_chosen },
            },
            weather: WeatherScene {
                precipitation: mix(self.weather.precipitation, other.weather.precipitation),
                lightning: mix(self.weather.lightning, other.weather.lightning),
            },
            port: if late { other.port } else { self.port },
        }
    }

    /// The same scene with no cloud at all: the sky lane's stills and parity tests, drawn before clouds.
    pub fn cloudless(mut self) -> Self {
        self.clouds.coverage = 0.0;
        self.clouds.horizon_coverage = 0.0;
        self.clouds.fog = 0.0;
        // A genus with cells (cumulonimbus) towers and rains whatever the cover: none.
        self.clouds.genus = None;
        self.clouds.high = Some(HighClouds {
            genus: HighCloudGenus::Cirrus,
            amount: 0.0,
            altitude: 9000.0,
        });
        self.weather = WeatherScene {
            precipitation: 0.0,
            lightning: 0.0,
        };
        self
    }
}

/// Local sidereal angle the sky shows (radians) where a scene has no place on the Earth.
pub const SIDEREAL: f64 = 55.0 * std::f64::consts::PI / 180.0;

/// Where on the Earth a scene is and the sidereal time there. With them the stars stand as they did over that sea
/// that night: the Southern Cross over Savo, Orion over a winter Atlantic. The scene's axes are the theatre's: x east,
/// z south.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// Degrees north.
    pub latitude: f64,
    /// Local sidereal time, degrees: the right ascension on the meridian
    /// (`naval_sim::theatre::sun::sidereal_deg`).
    pub sidereal: f64,
    /// Where true north lies on the scene's grid there, degrees clockwise from −Z: 0 where the scene's grid is true, as
    /// it is not far from the place a theatre scene's chart is laid about (the meridians converge).
    pub north_deg: f64,
}

impl Place {
    /// True north and true east on the horizon, in the scene's axes.
    fn north_east(&self) -> (DVec3, DVec3) {
        let (sin, cos) = self.north_deg.to_radians().sin_cos();
        (DVec3::new(sin, 0.0, -cos), DVec3::new(cos, 0.0, sin))
    }
}
/// Inclination of the moon's path to the sun's arc.
pub const MOON_INCLINATION: f64 = 28.0 * std::f64::consts::PI / 180.0;

/// Unit vector for compass angles in degrees: elevation above the horizon, azimuth 0 toward +Z, 90 toward +X.
pub fn direction_from_angles(elevation_deg: f64, azimuth_deg: f64) -> DVec3 {
    let (e, a) = (elevation_deg.to_radians(), azimuth_deg.to_radians());
    DVec3::new(a.sin() * e.cos(), e.sin(), a.cos() * e.cos())
}

/// Elevation and azimuth in degrees of a unit vector; azimuth in (-180, 180].
pub fn angles_of(direction: DVec3) -> (f64, f64) {
    (
        direction.y.clamp(-1.0, 1.0).asin().to_degrees(),
        direction.x.atan2(direction.z).to_degrees(),
    )
}

/// Lit share of the moon's disc for a phase (0 new, 0.5 full).
pub fn moon_illumination(phase: f64) -> f64 {
    (1.0 - (2.0 * std::f64::consts::PI * phase).cos()) / 2.0
}

/// The moon's light at a phase (0 new, 0.5 full) over a full moon's: Krisciunas and Schaefer's fit to its magnitude by
/// the phase angle α (degrees), 0.026 |α| + 4 × 10⁻⁹ α⁴ fainter than full ("A model of the brightness of moonlight",
/// PASP 103, 1991). The lit share alone overstates a part moon: the shadows of its rough ground fill the lit part as the
/// sun lights it from the side, so a quarter moon gives 9 % of a full moon's light, not half, and a crescent three days
/// old under 1 %.
pub fn moon_brightness(phase: f64) -> f64 {
    let alpha = (180.0 - 360.0 * phase.rem_euclid(1.0)).abs();
    10f64.powf(-0.4 * (0.026 * alpha + 4e-9 * alpha.powi(4)))
}

/// Where the sun, moon and stars stand for an authored sun direction (`CelestialModel`).
#[derive(Clone, Copy, Debug)]
pub struct CelestialModel {
    pub sun: DVec3,
    pub moon: DVec3,
    pub pole: DVec3,
    /// World → celestial (equatorial) frame; its rows are the celestial axes in world space.
    pub star_rotation: DMat3,
    pub phase: f64,
    pub sidereal: f64,
    /// The scene's place on the Earth, when it has one: the stars then stand by it, not by the authored sun.
    pub place: Option<Place>,
    meridian: DVec3,
    west: DVec3,
}

impl Default for CelestialModel {
    fn default() -> Self {
        Self {
            sun: DVec3::Y,
            moon: -DVec3::Y,
            pole: DVec3::Y,
            star_rotation: DMat3::IDENTITY,
            phase: 0.5,
            sidereal: SIDEREAL,
            place: None,
            meridian: DVec3::ZERO,
            west: DVec3::ZERO,
        }
    }
}

impl CelestialModel {
    pub fn set(&mut self, elevation_deg: f64, azimuth_deg: f64, phase: f64) {
        self.phase = ((phase % 1.0) + 1.0) % 1.0;
        let night = elevation_deg < 0.0;
        self.meridian = direction_from_angles(
            elevation_deg.abs(),
            if night {
                azimuth_deg + 180.0
            } else {
                azimuth_deg
            },
        );
        self.pole = direction_from_angles(
            90.0 - elevation_deg.abs(),
            if night {
                azimuth_deg
            } else {
                azimuth_deg + 180.0
            },
        );
        self.west = self.pole.cross(self.meridian).normalize();
        let sun_hour = if night { std::f64::consts::PI } else { 0.0 };
        self.sun = self.on_arc(sun_hour);
        self.moon = self.on_arc(sun_hour - 2.0 * std::f64::consts::PI * self.phase);
        let (cos, sin) = (MOON_INCLINATION.cos(), MOON_INCLINATION.sin());
        let along = self.sun.dot(self.moon);
        let across = self.sun.cross(self.moon);
        self.moon = (self.moon * cos - across * sin + self.sun * (along * (1.0 - cos))).normalize();
        self.update_stars();
    }

    fn on_arc(&self, hour: f64) -> DVec3 {
        (self.meridian * hour.cos() + self.west * hour.sin()).normalize()
    }

    /// The moon where it stands (a theatre's, by its date), in place of its place along the sun's arc. The next
    /// [`CelestialModel::set`] puts it back on the arc.
    pub fn stand_moon(&mut self, moon: DVec3) {
        self.moon = moon.normalize();
    }

    /// The celestial pole: the place's, or the authored sun's.
    pub fn north(&self) -> DVec3 {
        match self.place {
            Some(place) => {
                let latitude = place.latitude.to_radians();
                let (north, _) = place.north_east();
                DVec3::Y * latitude.sin() + north * latitude.cos()
            }
            None => self.pole,
        }
    }

    /// The scene's place on the Earth (none: the stars turn about the authored sun's pole).
    pub fn set_place(&mut self, place: Option<Place>) {
        self.place = place;
        if place.is_some() || self.meridian != DVec3::ZERO {
            self.update_stars();
        }
    }

    fn update_stars(&mut self) {
        let (x, z) = match self.place {
            // The true sky. The pole stands the latitude above the north point (−Z) and the equator crosses the
            // meridian to the south; right ascension zero lies the sidereal time west (−X) of the meridian.
            Some(place) => {
                let (latitude, sidereal) =
                    (place.latitude.to_radians(), place.sidereal.to_radians());
                let (north, east) = place.north_east();
                let pole = DVec3::Y * latitude.sin() + north * latitude.cos();
                let meridian = DVec3::Y * latitude.cos() - north * latitude.sin();
                (meridian * sidereal.cos() - east * sidereal.sin(), pole)
            }
            None => {
                let (c, s) = (self.sidereal.cos(), self.sidereal.sin());
                (self.meridian * c + self.west * s, self.pole)
            }
        };
        let y = z.cross(x);
        // Rows are the celestial axes: glam matrices are column-major, so transpose the columns.
        self.star_rotation = DMat3::from_cols(x, y, z).transpose();
    }

    pub fn illumination(&self) -> f64 {
        moon_illumination(self.phase)
    }
}

/// Moonlight above the air at full moon, in the sea's units: through the air (`model::moon_grade::TRANSMITTANCE`), on the
/// dome's scale (`model::sky_grade::GAIN`) and in its tint (`moon_tint`) it lights the sea at 0.27 units, the real full
/// moon's 0.27 lux high in a clear sky (`naval_render::lights::FULL_MOON_LUX`), so that after dark a unit of the scene's
/// light is a lux. It was 0.8 until 2026-10-21, a full moon of 0.52 lifted a stop for readability, which dusk keeps
/// ([`MOONLIGHT_DUSK`], `model::moonlight_at`).
pub const MOONLIGHT: f64 = 0.4173;
/// The moon's light through dusk: the game's moon the blue hour was graded with (a full moon of 0.52 at the sea).
pub const MOONLIGHT_DUSK: f64 = 0.8;
/// The moon's cool tint `#b4c9f0`, linear.
pub fn moon_tint() -> DVec3 {
    DVec3::new(
        srgb_to_linear(180.0 / 255.0),
        srgb_to_linear(201.0 / 255.0),
        srgb_to_linear(240.0 / 255.0),
    )
}

/// The light of the sun or moon at the sea (the CPU side of [`crate::CelestialLight`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub direction: DVec3,
    pub color: DVec3,
    pub intensity: f64,
    pub night: bool,
}

/// The active body's light from the sea-level sun and moon (colour × intensity): the moon once it
/// outshines the sun (`SkyState.chooseLight`).
pub fn choose_light(sun: DVec3, moon: DVec3, sea_sun: DVec3, sea_moon: DVec3) -> Light {
    let solar = sea_sun.max_element();
    let lunar = sea_moon.max_element();
    let night = lunar > solar;
    let intensity = if night { lunar } else { solar };
    let color = if intensity > 0.0 {
        (if night { sea_moon } else { sea_sun }) / intensity
    } else {
        DVec3::ONE
    };
    Light {
        direction: if night { moon } else { sun },
        color,
        intensity,
        night,
    }
}

// --- celestial/bodies.ts ------------------------------------------------------------------------

const STAR_LIMIT_AT: f64 = -2.0;
const STAR_LIMIT_MAGNITUDE: f64 = -1.5;
const STAR_LIMIT_PER_DEGREE: f64 = 0.55;
const MILKY_WAY_TWILIGHT: [f64; 2] = [-16.0, -9.0];
const MOONLIGHT_WASHOUT_MILKY_WAY: f64 = 0.45;
// The night sky for the faintest star: a natural sky's brightness (V magnitudes a square arcsecond), the moon's light
// over it as multiples of it (a full moon's three magnitudes brighter), and the margin of an eye that is not a practised
// observer's at the zenith (2026-10-25).
const DARK_SKY: f64 = 21.7;
const FULL_MOON_SKY: f64 = 14.85;
const UNPRACTISED_EYE: f64 = 0.5;
const NORMAL_FIELD: f64 = 52.0;
const ZOOM_GAIN: f64 = 3.0;
const EARTH_RADIUS: f64 = 6.36e6;
const EARTHSHINE: f64 = 0.016;
const EARTHSHINE_TINT: DVec3 = DVec3::new(0.8, 0.87, 1.0);
const PHASE_FLOOR: f64 = 0.38;

fn moon_disc_tint() -> DVec3 {
    DVec3::new(1.0, 0.975, 0.93) * (1.0 / (0.2126 + 0.7152 * 0.975 + 0.0722 * 0.93))
}

/// Share of a full moon's light on the sky: its light at the phase ([`moon_brightness`]), fading as it sets.
pub fn moonlight(phase: f64, moon: DVec3) -> f64 {
    moon_brightness(phase) * smoothstep(-0.05, 0.2, moon.y)
}

/// Faintest star magnitude visible with the sun at `sun_elevation` degrees under a sky lit by `moonlight`: twilight's
/// ramp, never past what the night sky leaves. That is the sky's brightness `b` (`DARK_SKY` and the moon's light over
/// it) by the usual conversion of a sky-quality reading to the naked eye's limit, 7.93 − 5 log₁₀(10^(4.316 − b/5) + 1),
/// less `UNPRACTISED_EYE`: 6.0 on a moonless night, 5.5 under a quarter moon, 4.0 under a full one. It was the ramp
/// less 1.4 magnitudes under a full moon, past the catalogue's faintest stars once the sun was 20° down, so the moon
/// put out none of them (2026-10-25).
pub fn star_limit(sun_elevation: f64, moonlight: f64) -> f64 {
    let twilight = STAR_LIMIT_MAGNITUDE + STAR_LIMIT_PER_DEGREE * (STAR_LIMIT_AT - sun_elevation);
    let sky = DARK_SKY - 2.5 * (1.0 + FULL_MOON_SKY * moonlight).log10();
    let night = 7.93 - 5.0 * (10f64.powf(4.316 - sky / 5.0) + 1.0).log10() - UNPRACTISED_EYE;
    twilight.min(night)
}

/// Share of the Milky Way's radiance shown.
pub fn milky_way_share(sun_elevation: f64, moonlight: f64) -> f64 {
    (1.0 - smoothstep(MILKY_WAY_TWILIGHT[0], MILKY_WAY_TWILIGHT[1], sun_elevation))
        * (1.0 - MOONLIGHT_WASHOUT_MILKY_WAY * moonlight)
}

/// Radians a pixel spans at the centre of a view `height` pixels tall whose half field has tangent
/// `tan_half_field`, and the gain on point sources its zoom gives (`viewScale`).
pub fn view_scale(tan_half_field: f64, height: f64) -> (f64, f64) {
    let magnification = (NORMAL_FIELD.to_radians() / 2.0).tan() / tan_half_field;
    (
        2.0 * tan_half_field / height.max(1.0),
        magnification.max(1.0).sqrt().min(ZOOM_GAIN),
    )
}

/// Lowest direction (its y) worth drawing stars in from a camera at `altitude` metres.
pub fn horizon_cut(altitude: f64) -> f64 {
    -(2.0 * altitude.max(0.0) / EARTH_RADIUS).sqrt() - 0.02
}

/// The moon's disc frame and lighting (`moonLighting`).
#[derive(Clone, Copy, Debug, Default)]
pub struct MoonLighting {
    pub lit: f64,
    pub opposition: f64,
    pub right: DVec3,
    pub up: DVec3,
    pub sun: DVec3,
    pub surface: DVec3,
    pub earthshine: DVec3,
}

/// The disc's surface is a full moon's (`full`, its irradiance above the air, times `disc`): the shader shades its lit
/// part by the sun's angle on it (Lommel–Seeliger) and the phase's darkening (`opposition`).
pub fn moon_lighting(
    sun: DVec3,
    moon: DVec3,
    pole: DVec3,
    phase: f64,
    full: DVec3,
    disc: f64,
) -> MoonLighting {
    let mut up = pole - moon * moon.dot(pole);
    if up.length_squared() < 1e-8 {
        up = DVec3::Y - moon * moon.y;
    }
    if up.length_squared() < 1e-8 {
        up = DVec3::X;
    }
    let up = up.normalize();
    let right = moon.cross(up).normalize();
    let lit = moon_illumination(phase);
    let luminance = 0.2126 * full.x + 0.7152 * full.y + 0.0722 * full.z;
    MoonLighting {
        lit,
        opposition: PHASE_FLOOR + (1.0 - PHASE_FLOOR) * ((1.0 - sun.dot(moon)) / 2.0).powf(1.5),
        right,
        up,
        sun: DVec3::new(sun.dot(right), sun.dot(up), -sun.dot(moon)),
        surface: moon_disc_tint() * if lit > 1e-4 { luminance * disc } else { 0.0 },
        earthshine: EARTHSHINE_TINT * (EARTHSHINE * (1.0 - lit)),
    }
}

/// Equatorial (J2000) to galactic coordinates; rows are the galactic axes (`galactic.ts`).
pub(crate) const EQUATORIAL_TO_GALACTIC: [[f64; 3]; 3] = [
    [-0.0548755604, -0.8734370902, -0.4838350155],
    [0.4941094279, -0.44482963, 0.7469822445],
    [-0.867666149, -0.1980763734, 0.4559837762],
];

/// World → galactic rotation for the sky's star frame (world → celestial).
pub fn galactic_rotation(star_rotation: DMat3) -> DMat3 {
    let rows = EQUATORIAL_TO_GALACTIC;
    let e2g = DMat3::from_cols(
        DVec3::from(rows[0]),
        DVec3::from(rows[1]),
        DVec3::from(rows[2]),
    )
    .transpose();
    e2g * star_rotation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_stands_where_the_scene_asks() {
        let mut model = CelestialModel::default();
        for (elevation, azimuth) in [(36.0, 58.0), (-6.0, 270.0), (0.0, 90.0), (60.0, 200.0)] {
            model.set(elevation, azimuth, 0.5);
            let (e, a) = angles_of(model.sun);
            assert!((e - elevation).abs() < 1e-9, "{e} vs {elevation}");
            let a = (a - azimuth).rem_euclid(360.0);
            assert!(a < 1e-6 || (360.0 - a) < 1e-6);
            // A full moon stands opposite the sun.
            assert!((model.moon + model.sun).length() < 1e-9);
        }
    }

    /// Where a star of right ascension `ra` and declination `dec` (degrees) stands over `place`: degrees above the
    /// horizon, and its world direction (x east, z south).
    fn star_over(place: Place, ra: f64, dec: f64) -> (f64, DVec3) {
        let mut model = CelestialModel::default();
        model.set(-30.0, 40.0, 0.5);
        model.set_place(Some(place));
        let (ra, dec) = (ra.to_radians(), dec.to_radians());
        let equatorial = DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin());
        let world = model.star_rotation.transpose() * equatorial;
        assert!((world.length() - 1.0).abs() < 1e-12);
        (world.y.asin().to_degrees(), world)
    }

    /// A scene whose grid is turned off true north at the place (a theatre scene away from the place its chart is laid
    /// about, where the meridians converge): every star turns with true north about the vertical, its elevation held,
    /// and the pole stands over true north.
    #[test]
    fn stars_turn_with_true_north_on_the_scenes_grid() {
        let at = |north_deg| Place {
            latitude: 50.0,
            sidereal: 40.0,
            north_deg,
        };
        let (s, c) = 30f64.to_radians().sin_cos();
        for (ra, dec) in [(37.95, 89.264), (88.79, 7.41), (186.65, -63.10)] {
            let (e0, w0) = star_over(at(0.0), ra, dec);
            let (e1, w1) = star_over(at(30.0), ra, dec);
            assert!((e0 - e1).abs() < 1e-9, "{e0} {e1}");
            // Turned 30° clockwise seen from above: −Z (north) toward +X (east).
            let turned = DVec3::new(w0.x * c - w0.z * s, w0.y, w0.x * s + w0.z * c);
            assert!((w1 - turned).length() < 1e-12, "{w1} {turned}");
        }
        let mut model = CelestialModel::default();
        model.set_place(Some(at(30.0)));
        let north = model.north();
        assert!((north.x.atan2(-north.z).to_degrees() - 30.0).abs() < 1e-9);
    }

    #[test]
    fn a_place_stands_the_pole_star_at_its_latitude_in_the_north() {
        for sidereal in [0.0, 95.0, 260.0] {
            let place = Place {
                latitude: 60.0,
                sidereal,
                north_deg: 0.0,
            };
            // Polaris, three quarters of a degree from the pole.
            let (elevation, world) = star_over(place, 37.95, 89.264);
            assert!((elevation - 60.0).abs() < 0.8, "{elevation}");
            assert!(world.z < 0.0 && world.x.abs() < 0.02, "{world:?}");
        }
    }

    #[test]
    fn a_star_rises_in_the_east_and_sets_in_the_west() {
        // A star on the equator from the equator: overhead when the sidereal time is its right ascension, in the east
        // three hours before and in the west three hours after.
        let at = |sidereal: f64| {
            star_over(
                Place {
                    latitude: 0.0,
                    sidereal,
                    north_deg: 0.0,
                },
                120.0,
                0.0,
            )
        };
        assert!((at(120.0).0 - 90.0).abs() < 1e-6);
        let (before, after) = (at(75.0), at(165.0));
        assert!((before.0 - 45.0).abs() < 1e-6 && before.1.x > 0.0);
        assert!((after.0 - 45.0).abs() < 1e-6 && after.1.x < 0.0);
    }

    #[test]
    fn a_moon_stood_where_it_is_stays_until_the_next_scene() {
        let mut model = CelestialModel::default();
        model.set(-30.0, 40.0, 0.9);
        let arc = model.moon;
        model.stand_moon(direction_from_angles(25.0, 80.0));
        assert!((angles_of(model.moon).0 - 25.0).abs() < 1e-9);
        assert!((angles_of(model.moon).1 - 80.0).abs() < 1e-9);
        // Its phase is the scene's, whatever its place.
        assert!((model.illumination() - moon_illumination(0.9)).abs() < 1e-12);
        model.set(-30.0, 40.0, 0.9);
        assert_eq!(model.moon, arc);
        // The pole is the place's once there is one.
        assert_eq!(model.north(), model.pole);
        model.set_place(Some(Place {
            latitude: -9.0,
            sidereal: 0.0,
            north_deg: 0.0,
        }));
        assert!(model.north().y < 0.0 && model.north().z < 0.0);
    }

    #[test]
    fn a_scene_between_two_moves_the_moon_between_them() {
        let at = |elevation, azimuth| SkyScene {
            moon: Some(SkyAngles { elevation, azimuth }),
            ..DEFAULT_SKY_SCENE
        };
        let half = at(10.0, 350.0).lerp(&at(30.0, 20.0), 0.5).moon.unwrap();
        assert!((half.elevation - 20.0).abs() < 1e-9 && (half.azimuth - 5.0).abs() < 1e-9);
        // A scene with no place for it keeps its own until halfway.
        assert_eq!(DEFAULT_SKY_SCENE.lerp(&at(30.0, 20.0), 0.4).moon, None);
        assert!(DEFAULT_SKY_SCENE.lerp(&at(30.0, 20.0), 0.6).moon.is_some());
    }

    #[test]
    fn without_a_place_the_stars_turn_about_the_authored_pole() {
        let mut model = CelestialModel::default();
        model.set(20.0, 250.0, 0.3);
        let authored = model.star_rotation;
        model.set_place(Some(Place {
            latitude: 35.0,
            sidereal: 10.0,
            north_deg: 0.0,
        }));
        assert_ne!(model.star_rotation, authored);
        // The frame stays a rotation, and a new sun does not move a placed sky.
        let placed = model.star_rotation;
        assert!((placed.determinant() - 1.0).abs() < 1e-12);
        model.set(-40.0, 100.0, 0.3);
        assert_eq!(model.star_rotation, placed);
        model.set_place(None);
        model.set(20.0, 250.0, 0.3);
        assert_eq!(model.star_rotation, authored);
    }

    #[test]
    fn star_rotation_takes_the_pole_to_celestial_z() {
        let mut model = CelestialModel::default();
        model.set(20.0, 250.0, 0.3);
        let z = model.star_rotation * model.pole;
        assert!((z - DVec3::Z).length() < 1e-9);
    }

    #[test]
    fn a_full_moon_is_the_night_light() {
        let light = choose_light(
            DVec3::Y,
            -DVec3::Y,
            DVec3::ZERO,
            DVec3::new(0.292, 0.374, 0.558),
        );
        assert!(light.night);
        assert!((light.intensity - 0.558).abs() < 1e-12);
    }

    /// A moonless night shows stars to the sixth magnitude, never the catalogue's faintest however deep the sun; a quarter
    /// moon puts out half a magnitude of them, a full one two; twilight's ramp still rules the dusk.
    #[test]
    fn the_moon_puts_out_the_faint_stars() {
        let dark = star_limit(-40.0, 0.0);
        assert!((dark - 6.0).abs() < 0.05, "{dark}");
        assert_eq!(star_limit(-18.0, 0.0), dark);
        let quarter = star_limit(-40.0, moon_brightness(0.25));
        assert!((quarter - 5.5).abs() < 0.05, "{quarter}");
        let full = star_limit(-40.0, 1.0);
        assert!((full - 4.0).abs() < 0.05, "{full}");
        assert!((star_limit(-6.0, 0.0) - 0.7).abs() < 1e-12);
        assert_eq!(star_limit(-6.0, 1.0), star_limit(-6.0, 0.0));
    }

    /// The full moon lights the sea at the real 0.27 lux, a unit a lux; a part moon by Krisciunas and Schaefer's law.
    #[test]
    fn the_moon_gives_its_real_light_by_its_phase() {
        let full = naval_render::lights::full_moon_scene(
            MOONLIGHT as f32,
            crate::model::moon_grade::TRANSMITTANCE as f32,
            crate::model::sky_grade::GAIN as f32,
            moon_tint().as_vec3(),
        );
        assert!(
            (full - naval_render::lights::FULL_MOON_LUX).abs() < 1e-3,
            "{full}"
        );
        assert!((moon_brightness(0.5) - 1.0).abs() < 1e-12);
        for quarter in [0.25, 0.75] {
            assert!(
                (moon_brightness(quarter) - 0.091).abs() < 0.002,
                "{quarter}"
            );
        }
        // A thin crescent (two days old) under a quarter of a percent, the lit share's tenfold.
        assert!(moon_brightness(0.06) < 0.0025);
        assert!(moon_illumination(0.06) > 0.03);
        assert!((moon_brightness(0.0) - moon_brightness(1.0)).abs() < 1e-12);
    }
}
