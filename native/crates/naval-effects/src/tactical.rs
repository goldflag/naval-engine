//! Draw the simulation's smoke volumes and illumination without running a second smoke simulation.

use glam::DVec3;
use naval_render::PointLight;

use crate::{
    CameraView,
    instances::{GAS_SCREEN, GasPuff},
    particles::place_facing,
};

#[derive(Clone, Copy, Debug)]
pub struct ScreenSmoke {
    pub id: i64,
    pub position: DVec3,
    pub radius: f64,
    pub height: f64,
    pub density: f64,
    pub age: f64,
}

/// A star shell's candle hangs this far (m) under the canopy's centre: the shroud lines of a parachute of about a metre
/// and a half across. Its pendulum's period is 2π √(L / g), 3.5 s.
pub const SHROUD_M: f64 = 3.0;
/// The swing's amplitude (rad, about 10°): an estimate for a small canopy in the gusts aloft, where nothing damps it.
pub const SWING_RAD: f64 = 0.17;
/// The burning candle's flame (m, its radius): what the flare's light comes from, the size its glare is drawn at.
pub const FLAME_RADIUS_M: f64 = 0.25;
/// The canopy's glide (m/s): a round canopy slides off its sink as it swings, a fifth of it or so, and turns as it
/// slides, a circle in 10 to 18 s (by the flare's id, either way), so the candle traces a loose helix through the air
/// and the smoke it leaves a corkscrew some metres across, as the photographs of parachute flares show it.
pub const GLIDE_M_S: f64 = 0.8;
const GLIDE_TURN_S: (f64, f64) = (10.0, 18.0);
/// The air's eddies a few hundred metres up on a quiet night: how fast they carry the smoke off the line the candle left
/// (m/s), and how long one lasts (s), past which the smoke's wander grows as a random walk's, √t. Estimates: they bend
/// a trail some metres off true a few tens of metres up, a trail's width at the top, which is how it looks.
const EDDY_M_S: f64 = 0.25;
const EDDY_S: f64 = 15.0;
/// The eddies' periods along the trail (s of the candle's burning, a few tens of metres apart at its sink) and their
/// weights (the sum's root mean square about 1).
const EDDIES: [(f64, f64); 3] = [(11.0, 0.6), (19.0, 0.8), (33.0, 0.9)];
/// How fast the candle's swing goes out of the smoke it gives off (s): the canopy's wake and the plume's own heat mix
/// it out within a second or two, so the trail leaves the swinging candle and straightens above it.
const SWING_MIXED_S: f64 = 1.5;
/// A searchlight's mirror (m, its radius): a 36-inch lamp's, the size of the source seen down the beam, and its near
/// field, where the beam is no wider than the mirror and its light no longer falls as the square of the distance.
pub const LAMP_APERTURE_M: f64 = 0.457;

#[derive(Clone, Copy, Debug)]
pub struct Searchlight {
    pub origin: DVec3,
    pub direction: DVec3,
    pub range: f64,
    pub half_angle: f64,
    pub intensity: f64,
    /// The scene's intensity for a real candela this frame (`naval_render::lights::candela_scale`: the sun's scale by
    /// day, the moon's lift after dark), which the host sets as it fills the lamps.
    pub candela: f64,
}

/// A star shell (`FlareRender`): flying to its burst, then burning under its parachute. The simulation moves it (its
/// flight, then the wind's drift and its sink); the swing under the canopy is drawn only ([`Flare::candle`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Flare {
    /// The simulation's stable id: it sets the swing's phase.
    pub id: i64,
    /// Where the simulation holds it: the candle hanging still under its canopy.
    pub position: DVec3,
    /// World velocity (m/s): the wind's drift and the sink.
    pub velocity: DVec3,
    /// Seconds since it lit (0 before).
    pub burning: f64,
    pub ignited: bool,
    pub range: f64,
    /// Its candle fresh, by the bore of the gun that fired it (the host's: the simulation's, by its calibre,
    /// `naval_effects_sim::star_shell_candle`), faded as its reach fades (`illumination::star_shell`).
    pub fresh: crate::illumination::StarShellCandle,
    /// The scene's intensity for a real candela this frame, as [`Searchlight::candela`].
    pub candela: f64,
}

#[derive(Default)]
pub struct TacticalEffects {
    pub smoke: Vec<ScreenSmoke>,
    pub searchlights: Vec<Searchlight>,
    pub flares: Vec<Flare>,
}

impl Searchlight {
    /// The lamp as the lit layers take it, and the light the air scatters from its beam (`air_light`): one light, a
    /// 36-inch arc's real candela at its power (`illumination::searchlight`) in the scene's units, reaching the
    /// simulation's range.
    pub fn light(&self) -> PointLight {
        let lamp = crate::illumination::searchlight(
            self.origin,
            self.direction,
            self.half_angle,
            self.intensity,
        );
        PointLight {
            intensity: lamp.intensity * self.candela as f32,
            distance: self.range as f32,
            ..lamp
        }
    }
}

impl Flare {
    /// The burning candle as it swings under its canopy: a spherical pendulum of the shroud's length about the point
    /// [`SHROUD_M`] over where the simulation holds it, glided off it ([`Flare::glide`]), its ellipse and phase from the
    /// flare's id. Drawn only: the simulation's light and sight take its place.
    pub fn candle(&self) -> DVec3 {
        if !self.ignited {
            return self.position;
        }
        self.position + self.glide(self.burning) + self.swing(self.burning)
    }

    /// The candle's swing under the canopy `seconds` after it lit (m, from the canopy's centre less the shroud).
    fn swing(&self, seconds: f64) -> DVec3 {
        let omega = (9.81 / SHROUD_M).sqrt();
        let seed = self.seed();
        let phase = omega * seconds + seed * std::f64::consts::TAU;
        // The ellipse's axes turned by the seed, its minor axis 0.3 to 0.9 of its major.
        let (turn_sin, turn_cos) = (seed * 7.3).sin_cos();
        let (major, minor) = (
            SWING_RAD * phase.sin(),
            SWING_RAD * (0.3 + 0.6 * (seed * 3.1).fract()) * phase.cos(),
        );
        let (x, z) = (
            major * turn_cos - minor * turn_sin,
            major * turn_sin + minor * turn_cos,
        );
        // Leaning `lean` from the vertical toward (x, z), on the sphere of the shroud's length about the canopy.
        let lean = (x * x + z * z).sqrt();
        let across = if lean > 1e-12 { lean.sin() / lean } else { 1.0 };
        DVec3::new(
            SHROUD_M * x * across,
            SHROUD_M * (1.0 - lean.cos()),
            SHROUD_M * z * across,
        )
    }

    fn seed(&self) -> f64 {
        (self.id as f64 * 0.618_034).fract().abs()
    }

    /// How fast it sinks (m/s): the simulation's, never less than half a metre a second.
    pub fn sink(&self) -> f64 {
        (-self.velocity.y).max(0.5)
    }

    /// How far the canopy has glided through the air `seconds` after it lit (m, horizontal; [`GLIDE_M_S`]): a circle
    /// begun where the simulation holds it.
    pub fn glide(&self, seconds: f64) -> DVec3 {
        let seed = self.seed();
        let period = GLIDE_TURN_S.0 + (GLIDE_TURN_S.1 - GLIDE_TURN_S.0) * (seed * 5.7).fract();
        let turn = std::f64::consts::TAU / period
            * if (seed * 11.3).fract() < 0.5 {
                1.0
            } else {
                -1.0
            };
        let heading = seed * 13.1;
        let radius = GLIDE_M_S / turn;
        let (now_sin, now_cos) = (heading + turn * seconds).sin_cos();
        let (lit_sin, lit_cos) = heading.sin_cos();
        DVec3::new(
            radius * (now_sin - lit_sin),
            0.0,
            radius * (lit_cos - now_cos),
        )
    }

    /// Where the smoke the candle gave off `age` seconds ago is now: where the candle was in the air then (the wind
    /// carries smoke and flare alike, so only the glide and the sink part them; its swing mixed out as it ages,
    /// [`SWING_MIXED_S`]), risen by the flare's sink since, and carried off that line by the eddies ([`EDDY_M_S`]) as
    /// far as its age lets them. The newest is the candle.
    pub fn smoke(&self, age: f64) -> DVec3 {
        let age = age.clamp(0.0, self.burning);
        let released = self.burning - age;
        let seed = self.seed();
        let reach = EDDY_M_S * age / (1.0 + age / EDDY_S).sqrt();
        let mut eddies = DVec3::ZERO;
        for (k, &(period, weight)) in EDDIES.iter().enumerate() {
            // Each eddy's phase by the smoke's release, so neighbouring smoke goes the same way, turning slowly as the
            // smoke ages.
            let phase = std::f64::consts::TAU * (released / period + age / (3.0 * period))
                + seed * (k as f64 + 1.0) * 7.9;
            eddies += DVec3::new(phase.sin(), 0.0, (phase * 1.3 + seed * 4.1).sin()) * weight;
        }
        self.position
            + self.glide(released)
            + self.swing(released) * (-age / SWING_MIXED_S).exp()
            + DVec3::Y * (self.sink() * age)
            + eddies * reach
    }

    /// The smoke trailing above the burning candle, newest first, each with its age (s): the candle, the smoke it gave
    /// off at whole multiples of `step` seconds since it lit (so each point stays with the same smoke as it ages and
    /// the line holds still in the air), and the oldest within `longest` metres of the candle.
    pub fn trail(&self, longest: f64, step: f64) -> Vec<(DVec3, f64)> {
        if !self.ignited || self.burning <= 0.0 {
            return Vec::new();
        }
        let oldest = self.burning.min(longest / self.sink());
        let mut points = vec![(self.candle(), 0.0)];
        let mut released = (self.burning / step).floor() * step;
        if self.burning - released < 1e-3 {
            released -= step;
        }
        while self.burning - released < oldest && released > 0.0 {
            let age = self.burning - released;
            points.push((self.smoke(age), age));
            released -= step;
        }
        points.push((self.smoke(oldest), oldest));
        points
    }

    /// The burning flare as the lit layers take it, at its swinging candle, and the light the air scatters about it
    /// (`air_light`): one light, its candle's real candela by its calibre (`illumination::star_shell`, faded as the
    /// simulation fades its reach) in the scene's units, reaching the simulation's reach.
    pub fn light(&self) -> PointLight {
        let candle = crate::illumination::star_shell(self.candle(), self.fresh, self.range);
        PointLight {
            intensity: candle.intensity * self.candela as f32,
            distance: self.range as f32,
            ..candle
        }
    }
}

impl TacticalEffects {
    /// Ordinary pool smoke fades near the eye; a tactical screen must keep obscuring a ship inside it.
    pub fn publish_smoke(&self, camera: &CameraView, out: &mut Vec<GasPuff>) {
        let frustum = camera.frustum();
        for smoke in &self.smoke {
            if smoke.density <= 0.0 || smoke.radius <= 0.0 || smoke.height <= 0.0 {
                continue;
            }
            let radius = smoke.radius.max(smoke.height * 0.5);
            if !frustum
                .iter()
                .all(|plane| plane.distance(camera.drawn(smoke.position)) >= -radius)
            {
                continue;
            }
            let distance = smoke.position.distance(camera.position);
            // A near-plane quad covers the view when the lens enters the volume. Its shader integrates only the
            // actual ellipsoid in front of the eye, truncated at the scene depth (including our own hull).
            let (placed, edge) = if distance < radius * 1.25 {
                let (_, _, back) = camera.axes();
                let near = camera.near.max(0.1) * 2.0;
                let lens = camera.lens();
                (
                    camera.position - back * near,
                    near * 4.0 / lens.0.min(lens.1).max(0.01),
                )
            } else {
                place_facing(camera, smoke.position, distance, radius, radius * 2.4)
            };
            out.push(GasPuff {
                sphere: smoke
                    .position
                    .as_vec3()
                    .extend(smoke.radius as f32)
                    .to_array(),
                state: [
                    smoke.age as f32,
                    smoke.id as f32,
                    0.0,
                    smoke.density.clamp(0.0, 1.0) as f32,
                ],
                tint: [0.68, 0.70, 0.72, 1.0],
                evolution: [0.0, 0.0, 0.0, (smoke.height * 0.5) as f32],
                quad: placed.as_vec3().extend(edge as f32).to_array(),
                // w selects the analytical tactical-volume path in both the gas and occlusion shaders.
                extra: [1.0, 0.0, 0.0, GAS_SCREEN],
                warp: [0.0; 4],
            });
        }
    }

    /// A bounded renderer light list; the simulation's detection never depends on this budget.
    pub fn lights(&self, camera: DVec3) -> Vec<PointLight> {
        let mut lights = Vec::with_capacity(self.searchlights.len() + self.flares.len());
        lights.extend(self.searchlights.iter().map(Searchlight::light));
        lights.extend(
            self.flares
                .iter()
                .filter(|flare| flare.ignited)
                .map(Flare::light),
        );
        lights.sort_by(|a, b| {
            a.position
                .as_dvec3()
                .distance_squared(camera)
                .total_cmp(&b.position.as_dvec3().distance_squared(camera))
        });
        lights.truncate(4);
        lights
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn unlit_shells_do_not_illuminate_and_searchlights_are_directional() {
        let effects = TacticalEffects {
            searchlights: vec![Searchlight {
                origin: DVec3::ZERO,
                direction: DVec3::Z,
                range: 5000.0,
                half_angle: 0.1,
                intensity: 1.0,
                candela: 1.0,
            }],
            flares: vec![Flare {
                position: DVec3::Y * 300.0,
                ignited: false,
                range: 1500.0,
                ..Default::default()
            }],
            ..Default::default()
        };
        let lights = effects.lights(DVec3::ZERO);
        assert_eq!(lights.len(), 1);
        assert_eq!(lights[0].angular_attenuation(Vec3::Z), 1.0);
        assert_eq!(lights[0].angular_attenuation(Vec3::X), 0.0);
    }

    #[test]
    fn a_burning_flare_swings_on_its_shroud_and_its_light_swings_with_it() {
        let hung = DVec3::new(100.0, 500.0, -200.0);
        let flare = |burning: f64| Flare {
            id: 7,
            position: hung,
            ignited: true,
            burning,
            range: 3000.0,
            ..Default::default()
        };
        // Before it lights, the shell is where the simulation flies it.
        let unlit = Flare {
            ignited: false,
            ..flare(0.0)
        };
        assert_eq!(unlit.candle(), hung);
        let period = std::f64::consts::TAU * (SHROUD_M / 9.81).sqrt();
        let mut widest: f64 = 0.0;
        for step in 0..40 {
            let burning = step as f64 * period / 40.0;
            let candle = flare(burning).candle() - flare(burning).glide(burning);
            // On the sphere of the shroud's length about the canopy, and never above the canopy's height.
            let pivot = hung + DVec3::Y * SHROUD_M;
            assert!((candle.distance(pivot) - SHROUD_M).abs() < 1e-9);
            assert!(candle.y < pivot.y);
            widest = widest.max((candle - hung).length());
            assert_eq!(
                flare(burning).light().position,
                flare(burning).candle().as_vec3()
            );
        }
        // It swings by about the amplitude, and comes back after a period.
        assert!(widest > SHROUD_M * SWING_RAD * 0.5 && widest < SHROUD_M * SWING_RAD * 1.1);
        let swung = |burning: f64| flare(burning).candle() - flare(burning).glide(burning);
        assert!(swung(3.0).distance(swung(3.0 + period)) < 1e-6);
    }

    #[test]
    fn a_flare_leaves_its_smoke_in_the_air_wandering_off_true_as_it_ages() {
        let flare = |burning: f64| Flare {
            id: 11,
            position: DVec3::new(0.0, 600.0 - 4.0 * burning, 0.0),
            velocity: DVec3::new(0.0, -4.0, 0.0),
            ignited: true,
            burning,
            range: 3000.0,
            ..Default::default()
        };
        // The canopy glides a circle at its glide speed, begun where the simulation holds it.
        let still = flare(0.0);
        assert_eq!(still.glide(0.0), DVec3::ZERO);
        let speed = (still.glide(1.0005) - still.glide(0.9995)).length() / 0.001;
        assert!((speed - GLIDE_M_S).abs() < 1e-3, "{speed}");
        // The trail runs from the candle up, a point every step of release, never longer than asked.
        let trail = flare(30.0).trail(200.0, 2.0);
        assert_eq!(trail[0], (flare(30.0).candle(), 0.0));
        assert_eq!(trail.last().unwrap().1, 30.0);
        assert!(trail.windows(2).all(|w| w[1].1 > w[0].1));
        // Each risen by the sink since, but for what is left of the candle's swing (a few centimetres up at most).
        for &(point, age) in &trail[1..] {
            assert!((point.y - (flare(30.0).position.y + 4.0 * age)).abs() < 0.05);
        }
        // The newest smoke is the candle, so a point coming in next to it does not jump.
        assert!(flare(30.0).smoke(0.0).distance(flare(30.0).candle()) < 1e-9);
        let capped = flare(90.0).trail(200.0, 2.0);
        assert!((capped.last().unwrap().1 - 50.0).abs() < 1e-9);
        assert!(capped.len() <= 2 + 25);
        // Each point stays with its smoke: half a second on, the smoke given off at 20 s is where it was, but for the
        // eddies' half a second (some tenths of a metre a second).
        let at = |burning: f64| {
            flare(burning)
                .trail(200.0, 2.0)
                .into_iter()
                .find(|&(_, age)| (burning - age - 20.0).abs() < 1e-9)
                .expect("a point at 20 s")
                .0
        };
        let moved = at(30.5) - at(30.0);
        assert!(moved.y.abs() < 1e-3 && moved.length() < 0.4, "{moved}");
        // Old smoke has wandered further off the candle's line than young.
        let off = |burning: f64, age: f64| {
            let f = flare(burning);
            let swing = f.swing(burning - age) * (-age / SWING_MIXED_S).exp();
            (f.smoke(age) - f.position - f.glide(burning - age) - swing - DVec3::Y * 4.0 * age)
                .length()
        };
        let mean = |age: f64| (0..20).map(|k| off(60.0 + k as f64, age)).sum::<f64>() / 20.0;
        assert!(
            mean(2.0) < 1.0 && mean(45.0) > 3.0 * mean(5.0),
            "{} {} {}",
            mean(2.0),
            mean(5.0),
            mean(45.0)
        );
    }
}
