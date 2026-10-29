//! The air's shimmer over the sea: what a long glass shows of the turbulence in the air near the water, a far hull,
//! her masts and the horizon wavering, boiling and going soft, by the weather (native/docs/haze.md, "Shimmer"). Drawn
//! in the display pass (`shaders/display_pass.wgsl`, `shimmer_air`): each pixel shows the scene a little off where it
//! lies, by a field of value noise in the view's bearing and elevation (fixed in the air as the glass turns) carried
//! across the view by the wind, and what the field is too fine to draw (eddies under a few pixels) goes into a blur.
//!
//! - **How turbulent the air is** ([`SeaAir::of_weather`]): the refractive-index structure parameter Cn² at 10 m comes
//!   from the air–sea temperature contrast by Monin–Obukhov similarity ([`CN2_PER_K2`] a kelvin squared, neutral).
//!   The contrast is the open ocean's (air a kelvin under the sea), a calm sunny sea's warm skin on top (up to 2 K, gone
//!   in a fresh breeze) and a shower's rain-cooled air; light winds let it convect (up to three times), fog's warm air
//!   over cold water lies stable (a fifth). An overcast day in a breeze is about 1.6e-15 m^-2/3, a calm sunny noon
//!   1.2e-14, thick fog 5e-16. Cn² falls with height as z^-2/3 and over the marine layer's [`LAYER_TOP_M`].
//! - **What a ray sees of it** ([`tilt`]): a point's image wanders (G-tilt, one axis) by 2.84 D^-1/3 ∫ Cn² w dx through
//!   an aperture D, w = (1 − x/L)^5/3 for a point at L (a spherical wave: the air near the eye counts most) and 1 for
//!   the sky (a plane wave), at the ray's own height over the curved sea. The aperture is the glass's: 6 mm of
//!   objective a power, from the eye's 7 mm to a big glass's 150 mm ([`aperture_m`]). Fried's r0 of the same integral
//!   gives the short-exposure seeing blur.
//! - **At which scales**: two points an angle θ apart see an eddy x off through beams θx apart; their wander is shared
//!   while θx is under D and parts as (θx / D)^-1/3 past it. Each octave of the field ([`OCTAVES`], cells of 1.6 mrad
//!   to 0.1 mrad) draws the wander shared over its cells but not over the next coarser's; what parts under the finest
//!   cell is drawn as blur. An octave fades in as its cells grow past a few pixels ([`FADE_PX`]) and what it would
//!   draw blurs instead, so the normal lens draws none of it and the display pass skips it there.
//! - **How it moves** ([`ShimmerClock`]): frozen turbulence carried by the wind across the line of sight, as it would
//!   be seen [`DRIFT_DISTANCE_M`] off, and boiling at the wind's and convection's pace over each cell.
//! - **Nothing nearer comes forward**: an offset or a blur tap landing on something nearer moves only by what that
//!   nearer thing's own path gives it, or takes the pixel's own colour.
//!
//! On a typical day at sea a hull 15 km off wanders a quarter of a pixel at the binoculars' 32× on a 1440-pixel frame,
//! and blurs by a third: the shimmer is a calm sunny day's sight, or a low bridge's.

use glam::{Vec2, Vec3};

use crate::camera::{Camera, DEFAULT_FOV_Y_DEGREES, FLAT};

/// Cn² at 10 m a kelvin of air–sea contrast squared, neutral, m^-2/3 K^-2: T* = κ ΔT / ln(z / z0T) with z0T = 1e-4 m,
/// C_T² = 4.9 T*² z^-2/3, Cn² = (79e-6 P / T²)² C_T² at 1013 hPa and 288 K.
pub const CN2_PER_K2: f32 = 1.2e-15;
/// The open ocean's air under the sea, kelvin (its mean).
pub const OCEAN_CONTRAST_K: f32 = 1.0;
/// The most a calm sea's sunlit warm skin adds, kelvin (the sun overhead, no cloud, no wind).
pub const WARM_SKIN_K: f32 = 2.0;
/// Rain's cooling of the air under it, kelvin at a downpour.
pub const RAIN_COOLING_K: f32 = 1.0;
/// Humidity's share, as a contrast added in quadrature: a neutral sea's floor.
pub const HUMIDITY_K: f32 = 0.3;
/// Cn²'s reference height and the least height it is read at, metres.
pub const Z_REF_M: f32 = 10.0;
pub const Z_LEAST_M: f32 = 2.0;
/// The marine layer's fall, metres: Cn² falls by e over this on top of z^-2/3.
pub const LAYER_TOP_M: f32 = 600.0;
/// How far a ray to the sky is taken to run, metres: past the horizon of any eye aboard.
pub const SKY_PATH_M: f32 = 40_000.0;
/// Light's wavelength for the seeing, metres.
pub const WAVELENGTH_M: f32 = 0.55e-6;
/// The samples along a ray, at the squares of their share of it (closer near the eye, where the air counts most).
pub const SAMPLES: u32 = 8;
/// Where the turbulence that moves the image is taken to drift, metres off.
pub const DRIFT_DISTANCE_M: f32 = 4_000.0;
/// Convection's own stirring, m/s: the boil in a calm.
pub const CONVECTION_MPS: f32 = 0.5;

/// The field's octaves' cell heights, radians; a cell is [`CELL_ASPECT`] times as wide.
pub const OCTAVES: [f32; 4] = [1.6e-3, 6e-4, 2.5e-4, 1e-4];
pub const CELL_ASPECT: f32 = 1.5;
/// An octave fades in between these cell heights in pixels.
pub const FADE_PX: [f32; 2] = [2.0, 5.0];
/// Under this many pixels of waver and of blur at the horizon the display pass skips the shimmer.
const LEAST_PX: f32 = 0.05;
const LEAST_BLUR_PX: f32 = 0.5;
/// The evolution's phase wraps at this many cells (a jump every few hours).
const EVOLVE_WRAP: f32 = 65_536.0;

/// The weather the air's turbulence is read from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirWeather {
    /// Degrees above the horizon.
    pub sun_elevation_deg: f32,
    /// 0 clear … 1 overcast.
    pub cloud_cover: f32,
    /// Fog on the sea, 0 … 1.
    pub fog: f32,
    /// Rain, 0 … 1 (a downpour).
    pub rain: f32,
    /// The wind over the sea, world x and z, m/s (where the air goes).
    pub wind: Vec2,
}

/// The air over the sea: its turbulence and its wind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SeaAir {
    /// Cn² at 10 m, m^-2/3.
    pub cn2: f32,
    /// World x and z, m/s.
    pub wind: Vec2,
}

impl SeaAir {
    /// An overcast day at sea in a fresh breeze, for a scene that gives no weather.
    pub const TYPICAL: SeaAir = SeaAir {
        cn2: 1.6e-15,
        wind: Vec2::ZERO,
    };

    /// The air `weather` makes (the module docs).
    pub fn of_weather(weather: &AirWeather) -> Self {
        let wind = weather.wind.length();
        let contrast = contrast_k(weather);
        let convection = 1.0 + 2.0 / (1.0 + wind);
        let stable = 1.0 - 0.8 * weather.fog.clamp(0.0, 1.0);
        Self {
            cn2: CN2_PER_K2 * (contrast * contrast + HUMIDITY_K * HUMIDITY_K) * convection * stable,
            wind: weather.wind,
        }
    }
}

/// How much colder the air is than the sea, kelvin: the ocean's, a calm sunny sea's warm skin and rain's cooling.
pub fn contrast_k(weather: &AirWeather) -> f32 {
    let sunshine = weather.sun_elevation_deg.to_radians().sin().max(0.0)
        * (1.0 - 0.8 * weather.cloud_cover.clamp(0.0, 1.0));
    let warm_skin = WARM_SKIN_K * sunshine * (-weather.wind.length() / 3.0).exp();
    OCEAN_CONTRAST_K + warm_skin + RAIN_COOLING_K * weather.rain.clamp(0.0, 1.0)
}

/// The objective `camera`'s field makes, metres: 6 mm a power over the normal lens, from the eye's pupil to a big
/// glass's.
pub fn aperture_m(camera: &Camera) -> f32 {
    let normal = (DEFAULT_FOV_Y_DEGREES.to_radians() * 0.5).tan();
    let half = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-6);
    (0.006 * normal / half).clamp(0.007, 0.15)
}

/// What a ray sees of the air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tilt {
    /// The image's wander, its variance on one axis, rad².
    pub total: f32,
    /// Of it, what is shared over each octave's cells, rad².
    pub held: [f32; 4],
    /// The short-exposure seeing blur, a Gaussian's sigma, radians.
    pub seeing: f32,
}

/// Sample `i` of [`SAMPLES`] along a ray, at the square of its share of it: the share, the step over the ray's length,
/// a point's weight there, (1 − share)^5/3, and the share's −1/3 power (the shader's `SHIMMER_ALONG`).
pub fn along(i: u32) -> [f32; 4] {
    let u = (i as f32 + 0.5) / SAMPLES as f32;
    let share = u * u;
    [
        share,
        2.0 * u / SAMPLES as f32,
        (1.0 - share).powf(5.0 / 3.0),
        share.powf(-1.0 / 3.0),
    ]
}

/// What a ray `distance` long sees (of a point there, or with `plane` of the sky), climbing `rise` (its sine of
/// elevation) from an eye `eye_height` up over a sea curving away by `curvature` (`Camera::curvature`), through
/// `aperture` in `air` (the shader's `shimmer_tilt`).
pub fn tilt(
    air: &SeaAir,
    aperture: f32,
    distance: f32,
    rise: f32,
    eye_height: f32,
    curvature: f32,
    plane: bool,
) -> Tilt {
    // Each octave's reach over the distance's cube root: (D / cell)^1/3 x^-1/3 is this by the sample's own share.
    let reach = OCTAVES.map(|cell| (aperture / cell).cbrt() * distance.powf(-1.0 / 3.0));
    let mut sum = 0.0;
    let mut held = [0.0; 4];
    for i in 0..SAMPLES {
        let [share, step, weight, near] = along(i);
        let x = distance * share;
        let z = eye_height + x * rise + curvature * x * x;
        let strength = air.cn2
            * (-2.0 / 3.0 * (z.max(Z_LEAST_M) / Z_REF_M).ln() - z.max(0.0) / LAYER_TOP_M).exp();
        let part = strength * if plane { 1.0 } else { weight } * distance * step;
        sum += part;
        for (h, r) in held.iter_mut().zip(reach) {
            *h += part * (r * near).min(1.0);
        }
    }
    let gain = 2.84 * aperture.powf(-1.0 / 3.0);
    Tilt {
        total: gain * sum,
        held: held.map(|s| gain * s),
        seeing: seeing_rad(sum, aperture),
    }
}

/// The short-exposure seeing blur (a Gaussian's sigma, radians) of a path whose weighted ∫ Cn² is `sum`, through
/// `aperture`: Fried's r0, the long exposure's width less the wander's share.
fn seeing_rad(sum: f32, aperture: f32) -> f32 {
    if sum <= 0.0 {
        return 0.0;
    }
    let k = std::f32::consts::TAU / WAVELENGTH_M;
    let r0 = (0.423 * k * k * sum).powf(-0.6);
    let width = 0.98 * WAVELENGTH_M / r0 * (1.0 - 0.37 * (r0 / aperture).cbrt()).max(0.0);
    width / 2.3548
}

/// An octave's share at cells `cell_px` pixels tall: none under [`FADE_PX`]'s first, all over its second.
fn fade(cell_px: f32) -> f32 {
    let t = ((cell_px - FADE_PX[0]) / (FADE_PX[1] - FADE_PX[0])).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What `tilt` draws at `px_per_rad`: the field's waver (rms, one axis) and the blur (a Gaussian's sigma), pixels.
pub fn drawn_px(tilt: &Tilt, px_per_rad: f32) -> (f32, f32) {
    let mut drawn = 0.0;
    let mut faded = 0.0;
    let mut coarser = 0.0;
    for (cell, held) in OCTAVES.into_iter().zip(tilt.held) {
        let own = (held - coarser).max(0.0);
        let shown = fade(cell * px_per_rad);
        drawn += own * shown * shown;
        faded += own * (1.0 - shown * shown);
        coarser = held;
    }
    let blur = (tilt.total - coarser).max(0.0) + faded + tilt.seeing * tilt.seeing;
    (drawn.sqrt() * px_per_rad, blur.sqrt() * px_per_rad)
}

/// The field's phases, in cells: each octave's drift across with the wind and its evolution, moved frame by frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShimmerClock {
    pub drift: [f32; 4],
    pub evolve: [f32; 4],
}

impl ShimmerClock {
    /// `dt` seconds of `air` across `camera`'s view: the wind across the line of sight carries the field as it would
    /// be seen [`DRIFT_DISTANCE_M`] off, and it boils at the wind's and convection's pace over each cell.
    pub fn advance(&mut self, camera: &Camera, air: &SeaAir, dt: f32) {
        if dt <= 0.0 {
            return;
        }
        let right = camera.view().row(0).truncate();
        let across = Vec2::new(right.x, right.z)
            .normalize_or_zero()
            .dot(air.wind);
        let turn = across / DRIFT_DISTANCE_M;
        let stir = (CONVECTION_MPS + 0.25 * air.wind.length()) / DRIFT_DISTANCE_M;
        for (k, cell) in OCTAVES.into_iter().enumerate() {
            let width = CELL_ASPECT * cell;
            self.drift[k] = (self.drift[k] + turn / width * dt).rem_euclid(cells_around(cell));
            self.evolve[k] = (self.evolve[k] + stir / cell * dt).rem_euclid(EVOLVE_WRAP);
        }
    }
}

/// The cells across a full turn of bearing at an octave's cell height (the shader's `wrap`).
fn cells_around(cell: f32) -> f32 {
    (std::f32::consts::TAU / (CELL_ASPECT * cell))
        .round()
        .max(1.0)
}

/// The display pass's shimmer: the view's frame, the eye, the air and the field's phases (`DisplayParams.shimmer`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shimmer {
    rows: [[f32; 4]; 7],
}

impl Shimmer {
    /// The shimmer `camera` sees over a `width` × `height` frame in `air` with the field at `clock`, or `None` where it
    /// would move nothing by [`LEAST_PX`] nor blur by [`LEAST_BLUR_PX`] at the horizon: the normal lens, a chart, an eye
    /// under water.
    pub fn new(
        camera: &Camera,
        width: u32,
        height: u32,
        air: &SeaAir,
        clock: &ShimmerClock,
    ) -> Option<Self> {
        if camera.curvature == FLAT || camera.eye.y <= 0.0 || width == 0 || height == 0 {
            return None;
        }
        let tan_y = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-6);
        let px_per_rad = height as f32 / (2.0 * tan_y);
        let aperture = aperture_m(camera);
        let horizon = tilt(
            air,
            aperture,
            SKY_PATH_M,
            0.0,
            camera.eye.y,
            camera.curvature,
            true,
        );
        let (waver, blur) = drawn_px(&horizon, px_per_rad);
        if waver < LEAST_PX && blur < LEAST_BLUR_PX {
            return None;
        }
        let view = camera.view();
        // The view's rows: right, up and back (`look_to_mat4`).
        let right = view.row(0).truncate() * (tan_y * width as f32 / height as f32);
        let up = view.row(1).truncate() * tan_y;
        let forward: Vec3 = -view.row(2).truncate();
        Some(Self {
            rows: [
                [forward.x, forward.y, forward.z, camera.curvature],
                [right.x, right.y, right.z, camera.eye.y],
                [up.x, up.y, up.z, px_per_rad],
                [air.cn2, 2.84 * aperture.powf(-1.0 / 3.0), aperture, 0.0],
                OCTAVES.map(|cell| (aperture / cell).cbrt()),
                clock.drift,
                clock.evolve,
            ],
        })
    }

    /// The uniform block's rows, or zeros for none (the shader's `px_per_rad` of 0 skips it).
    pub(crate) fn rows(shimmer: Option<Self>) -> [[f32; 4]; 7] {
        shimmer.map_or([[0.0; 4]; 7], |s| s.rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::EARTH_CURVATURE;

    /// Binoculars at `magnification` from an eye `height` up, looking level along −z.
    fn glass(magnification: f32, height: f32) -> Camera {
        let normal = (DEFAULT_FOV_Y_DEGREES.to_radians() * 0.5).tan();
        let mut camera =
            Camera::look_at(Vec3::new(0.0, height, 0.0), Vec3::new(0.0, height, -1000.0));
        camera.fov_y_degrees = (2.0 * (normal / magnification).atan()).to_degrees();
        camera
    }

    fn weather(sun: f32, cover: f32, wind: f32) -> AirWeather {
        AirWeather {
            sun_elevation_deg: sun,
            cloud_cover: cover,
            fog: 0.0,
            rain: 0.0,
            wind: Vec2::new(wind, 0.0),
        }
    }

    fn calm_noon() -> SeaAir {
        SeaAir::of_weather(&weather(60.0, 0.0, 1.0))
    }

    /// The waver and blur in pixels at 32× on a 1440-pixel frame of a hull `range` off from an eye `height` up.
    fn hull_px(air: &SeaAir, range: f32, height: f32) -> (f32, f32) {
        let camera = glass(32.0, height);
        let px_per_rad = 1440.0 / (2.0 * (camera.fov_y_degrees.to_radians() * 0.5).tan());
        let drop = range * range * EARTH_CURVATURE;
        let t = tilt(
            air,
            aperture_m(&camera),
            range,
            -(height + drop) / range,
            height,
            EARTH_CURVATURE,
            false,
        );
        drawn_px(&t, px_per_rad)
    }

    #[test]
    fn the_weather_sets_the_air() {
        let overcast = SeaAir::of_weather(&weather(40.0, 1.0, 8.0)).cn2;
        let calm = calm_noon().cn2;
        let breezy = SeaAir::of_weather(&weather(60.0, 0.0, 9.0)).cn2;
        let night = SeaAir::of_weather(&weather(-20.0, 0.0, 1.0)).cn2;
        let fog = SeaAir::of_weather(&AirWeather {
            fog: 1.0,
            ..weather(40.0, 1.0, 3.0)
        })
        .cn2;
        let rain = SeaAir::of_weather(&AirWeather {
            rain: 1.0,
            ..weather(30.0, 1.0, 10.0)
        })
        .cn2;
        assert!((1.0e-15..2.5e-15).contains(&overcast), "{overcast}");
        assert!((0.8e-14..1.6e-14).contains(&calm), "{calm}");
        assert!(breezy < 0.3 * calm && night < 0.4 * calm);
        assert!(fog < 0.6e-15, "{fog}");
        assert!(rain > 2.0 * overcast, "{rain}");
    }

    #[test]
    fn a_typical_day_barely_moves_a_far_hull_and_a_calm_noon_does() {
        let (typical, typical_blur) = hull_px(&SeaAir::TYPICAL, 15_000.0, 34.0);
        assert!((0.1..0.5).contains(&typical), "{typical} px");
        assert!(typical_blur < 0.6, "{typical_blur} px");
        let (waver, blur) = hull_px(&calm_noon(), 15_000.0, 34.0);
        assert!(waver > 2.0 * typical && blur > 0.8, "{waver} px, {blur} px");
        // A destroyer's bridge, 12 m up, looks through lower, rougher air.
        let (low, _) = hull_px(&calm_noon(), 15_000.0, 12.0);
        assert!(low > waver, "{low} against {waver}");
    }

    #[test]
    fn the_air_near_the_eye_counts_most_and_fine_cells_share_more() {
        let air = SeaAir::TYPICAL;
        let point = tilt(&air, 0.15, 15_000.0, -0.003, 34.0, EARTH_CURVATURE, false);
        let plane = tilt(&air, 0.15, 15_000.0, -0.003, 34.0, EARTH_CURVATURE, true);
        assert!(point.total < 0.6 * plane.total);
        assert!(point.held[0] < point.held[3] && point.held[3] <= point.total);
    }

    #[test]
    fn a_ray_climbing_out_of_the_layer_barely_wavers() {
        let air = SeaAir::TYPICAL;
        let level = tilt(&air, 0.15, SKY_PATH_M, 0.0, 34.0, EARTH_CURVATURE, true);
        // An aircraft 5 km off, 20° up.
        let rise = 20f32.to_radians().sin();
        let climbing = tilt(&air, 0.15, 5_000.0, rise, 34.0, EARTH_CURVATURE, false);
        assert!(climbing.total < 0.05 * level.total);
    }

    #[test]
    fn the_normal_lens_and_a_chart_draw_none() {
        let clock = ShimmerClock::default();
        let worst = SeaAir::of_weather(&AirWeather {
            rain: 1.0,
            ..weather(90.0, 0.0, 0.0)
        });
        for height in [6.0, 34.0] {
            assert!(Shimmer::new(&glass(1.0, height), 2560, 1440, &worst, &clock).is_none());
        }
        let mut chart = glass(32.0, 25.0);
        chart.curvature = FLAT;
        assert!(Shimmer::new(&chart, 2560, 1440, &worst, &clock).is_none());
        assert!(Shimmer::new(&glass(32.0, -2.0), 2560, 1440, &worst, &clock).is_none());
        assert!(Shimmer::new(&glass(32.0, 25.0), 2560, 1440, &SeaAir::TYPICAL, &clock).is_some());
    }

    #[test]
    fn the_wind_carries_the_field_across() {
        let camera = glass(32.0, 25.0);
        let air = SeaAir {
            wind: Vec2::new(7.0, 0.0),
            ..SeaAir::TYPICAL
        };
        let mut clock = ShimmerClock::default();
        clock.advance(&camera, &air, 0.5);
        // Looking along −z, +x is to the right: every octave drifts right by the same angle.
        for (cell, drift) in OCTAVES.into_iter().zip(clock.drift) {
            let turned = drift * CELL_ASPECT * cell;
            assert!(
                (turned - 7.0 / DRIFT_DISTANCE_M * 0.5).abs() < 1e-6,
                "{turned}"
            );
        }
        let before = clock;
        clock.advance(&camera, &air, 0.0);
        assert_eq!(clock, before);
    }

    #[test]
    fn the_shader_draws_these_octaves_and_constants() {
        let shader = include_str!("../../shaders/display_pass.wgsl");
        let cells = OCTAVES.map(|c| format!("{c:e}")).join(", ");
        let lines = [
            format!("const SHIMMER_CELLS: vec4<f32> = vec4<f32>({cells});"),
            format!(
                "const SHIMMER_FADE: vec2<f32> = vec2<f32>({:?}, {:?});",
                FADE_PX[0], FADE_PX[1]
            ),
            format!("const SHIMMER_CELL_ASPECT: f32 = {CELL_ASPECT:?};"),
            format!("const SHIMMER_Z_REF: f32 = {Z_REF_M:?};"),
            format!("const SHIMMER_Z_LEAST: f32 = {Z_LEAST_M:?};"),
            format!("const SHIMMER_LAYER_TOP: f32 = {LAYER_TOP_M:?};"),
            format!("const SHIMMER_SKY: f32 = {SKY_PATH_M:?};"),
            format!("const SHIMMER_WAVELENGTH: f32 = {WAVELENGTH_M:e};"),
            format!("const SHIMMER_LEAST_BLUR: f32 = {LEAST_BLUR_PX:?};"),
        ];
        for line in lines {
            assert!(shader.contains(&line), "{line}");
        }
        let table = shader
            .split("const SHIMMER_ALONG")
            .nth(1)
            .and_then(|rest| rest.split(");\n").next())
            .expect("the samples' table");
        let rows: Vec<Vec<f32>> = table
            .split("vec4<f32>(")
            .skip(1)
            .map(|row| {
                let values = row.split(')').next().unwrap_or_default();
                values
                    .split(',')
                    .map(|v| v.trim().parse().expect(v))
                    .collect()
            })
            .collect();
        assert_eq!(rows.len(), SAMPLES as usize);
        for (i, row) in rows.iter().enumerate() {
            for (drawn, want) in row.iter().zip(along(i as u32)) {
                assert!(
                    (drawn - want).abs() <= 2e-6 * want.abs(),
                    "sample {i}: {drawn} against {want}"
                );
            }
        }
    }
}
