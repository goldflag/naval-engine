//! The blast wave of a gun or an explosion in the air: the shock front racing out from it, seen by the bend it puts in
//! the light that crosses it and, in damp sea air, by the cloud that condenses behind it for a moment.
//!
//! **The front.** A charge of `W` kg of TNT sends out a shock whose strength depends only on the scaled distance
//! `Z = r / ∛W` (Hopkinson–Cranz), so every blast here is one curve stretched by its [`Shock::scale`], ∛W. The peak
//! overpressure is [`overpressure_kpa`] (Mills's fit to the free-air data: 31 kPa at Z = 5, 11 at Z = 10, 5 at
//! Z = 20), the front's speed the sound's times [`mach`] (Rankine–Hugoniot: 4 at Z = 1, 1.12 at Z = 5, 1.02 at
//! Z = 20), so it leaves the fireball several times faster than sound and settles to it within a few tens of metres.
//! A gun's blast is the propellant gas leaving the muzzle: [`gun_charge_kg`] of TNT a barrel, the barrels of a turret
//! together, leaning along the bore as the sheet it tears off the sea does (naval-water-effects' `blast_sheet`,
//! whose reach of 95 calibres is this charge's Z = 8.8).
//!
//! **What is seen of it.** The air behind the front is denser by [`density_jump`], and denser air bends light. A
//! ray that only grazes the sphere of the front crosses that step at a glancing angle and turns by [`limb_bend`] at
//! each crossing (the critical angle's complement, √(2 δn): 0.011 rad at Z = 5); a ray through its middle crosses it
//! square and does not turn. So the front shows as a thin line of bent background at the sphere's outline and
//! nothing inside it. The true line is centimetres wide, too thin to see past a few tens of metres; the game draws
//! the same turn across the whole pressure pulse behind the front ([`band_m`], a few metres), which is what makes it
//! a visible effect, [`SHOWN`] times over. It fades between [`FADE_Z`] and [`END_Z`], where the front is a sound wave.
//!
//! **On the sea.** From above, nothing grazes the front: what is seen is its foot running over the water. The air
//! behind the front moves with it ([`gust_mps`]: 66 m/s at Z = 5, 26 at Z = 10, 12 at Z = 20, for a few hundredths of
//! a second), which ruffles the sea into a pale band behind the front's foot, and nearer in tears the spray off it
//! (the blast sheet). The game draws that band ([`RUFFLE`]; a light gun's gust is too brief to raise one) and, with
//! it, the pulse as a ripple in what lies under it: the scene there read [`PUSH`] scales further out, a bend life
//! does not show.
//!
//! **The cloud.** Behind the pulse the air is drawn below the surrounding pressure for a few hundredths of a second
//! and cools as it expands; where that takes it below its dew point a mist forms, and goes as the pressure comes
//! back (a Wilson cloud). The underpressure is about 35 / Z kPa, and air [`ShockWaves::humidity`] damp needs
//! [`veil_underpressure_kpa`] of it, so the cloud stands only between [`VEIL_INNER_Z`] (nearer, the shock has heated
//! the air) and [`veil_reach`] (Z = 6.5 in the open ocean's 78 %, 14 at 90 %, nothing under 60 %), in a shell
//! [`VEIL_LENGTH`] scales thick trailing the pulse. Its mist is thin, so its opacity is its depth: a five-inch gun's
//! is nothing, a heavy turret's a pale flicker about the fireball, a magazine's a white dome over the ship.
//!
//! **What it does to smoke.** The air behind the front moves with it for as long as the pulse lasts, and carries
//! the smoke it passes that far ([`shove_m`], the gust times the pulse's duration: a metre at Z = 5 from a heavy
//! turret, four at Z = 2, three times that from a magazine, a hand's breadth from a five-inch gun).
//! [`ShockWaves::shoves`] gives each front's step for the smoke pools to take ([`Shove::at`]).
//!
//! **When it arrives.** A blast is heard and felt when its front arrives: [`arrival_seconds`], which the client's
//! camera shake and sound wait for.
//!
//! [`ShockWaves`] keeps the fronts and [`ShockWaves::publish`] packs what a camera can see of them for
//! `shaders/shock.wgsl` ([`crate::ShockLayer`]).

use bytemuck::{Pod, Zeroable};
use glam::DVec3;

use crate::blasts::TurretBlast;
use crate::math::smoothstep;

/// Sound in the sea-level air (m/s) and its pressure (kPa).
pub const SOUND: f64 = 340.0;
pub const AMBIENT_KPA: f64 = 101.325;
/// Air's refractivity n − 1 at sea level (550 nm).
pub const REFRACTIVITY: f64 = 2.78e-4;
/// A front is born this far out (scaled), inside any fireball, and is never taken as faster than this.
const START_Z: f64 = 0.3;
const MAX_MACH: f64 = 20.0;
/// Where the drawn bend starts to fade and where the front is dropped (scaled distance): a heavy turret's is gone
/// 180 m out and half a second on, a five-inch mount's 40 m out, a magazine's 600 m.
pub const FADE_Z: f64 = 10.0;
pub const END_Z: f64 = 28.0;
/// The grazing ray's turn is drawn this many times life's.
pub const SHOWN: f64 = 1.5;
/// The ripple under the pulse: the scene there is read this far further out from the blast (m per ∛kg) at Z = 5,
/// less as the front weakens (as the root of its density step).
pub const PUSH: f64 = 0.09;
/// The palest the ruffled sea behind the front's foot is drawn (its opacity under a gust far over
/// [`RUFFLE_GUST`] m/s), and the scales (∛kg) over which it comes in: the gust lasts as long as the pulse, a few
/// thousandths of a second from a five-inch gun, which stirs nothing, and hundredths from a heavy turret or a bomb.
pub const RUFFLE: f64 = 0.2;
const RUFFLE_GUST: f64 = 30.0;
pub const RUFFLE_SCALES: [f64; 2] = [1.0, 5.0];
/// The pressure pulse's length behind the front as a share of the scale: it lengthens as it runs (m per ∛kg).
const BAND: f64 = 0.6;
const BAND_PER_Z: f64 = 0.045;
/// The TNT a barrel's muzzle blast is worth per cubic metre of calibre (kg): a charge of about 4,200 kg/m³ of calibre
/// (297 kg in a 16-inch gun, 6.9 in a five-inch), three tenths of it left in the gas that leaves the muzzle.
pub const GUN_TNT_PER_CALIBRE3: f64 = 1250.0;
/// The blast's centre lies this many calibres ahead of the muzzles.
const GUN_CENTRE_CALIBRES: f64 = 12.0;
/// Guns below this calibre (m) draw no front: their blast is a crack a few metres across.
pub const MIN_GUN_CALIBRE: f64 = 0.10;
/// No cloud forms nearer than this (scaled): the shock leaves the air there too warm.
pub const VEIL_INNER_Z: f64 = 3.0;
/// The underpressure behind the pulse, kPa at Z = 1, falling as 1 / Z (Brode).
const UNDERPRESSURE: f64 = 35.0;
/// The farthest a cloud is drawn (scaled), however damp the air.
const VEIL_MAX_Z: f64 = 18.0;
/// The cloud's shell behind the pulse, in scales (m per ∛kg): the underpressure lasts about three times the pulse.
pub const VEIL_LENGTH: f64 = 4.0;
/// The cloud's shell is drawn from no further out than this share of its outer radius.
const VEIL_DRAWN_INNER: f64 = 0.5;
/// A gun's cloud against an explosion's of the same worth: its blast is gas let go over hundredths of a second, not
/// a detonation, and its underpressure is shallower.
pub const GUN_VEIL: f64 = 0.4;
/// The mist's extinction (1/m): a haze, a kilometre of visibility.
pub const VEIL_EXTINCTION: f64 = 0.005;
/// A cloud whose deepest line of sight is thinner than this optical depth is not drawn: a 15 cm shell's burst and
/// anything lighter makes none.
const MIN_VEIL_DEPTH: f64 = 0.08;
/// A front bending light by less than this (rad) and ruffling the sea by less than this opacity is not drawn.
const MIN_BEND: f64 = 2e-4;
const MIN_RUFFLE: f64 = 0.01;
/// Seen from inside, the band reaches this many of its widths in from the front.
const BAND_REACH: f64 = 4.0;
/// A camera within this share of the radius is given the whole view rather than a quad about the sphere.
const WHOLE_VIEW: f64 = 1.08;
/// Fronts kept at once; the one furthest through its run goes first.
pub const MAX_SHOCKS: usize = 24;

/// The peak overpressure of a free-air burst at scaled distance `z` (m/∛kg), kPa (Mills 1987).
pub fn overpressure_kpa(z: f64) -> f64 {
    let z = z.max(1e-3);
    1772.0 / z.powi(3) - 114.0 / z.powi(2) + 108.0 / z
}

/// The front's Mach number at scaled distance `z`: √(1 + 6 Δp / 7 p₀) for air.
pub fn mach(z: f64) -> f64 {
    (1.0 + 6.0 * overpressure_kpa(z).max(0.0) / (7.0 * AMBIENT_KPA))
        .sqrt()
        .min(MAX_MACH)
}

/// How much denser the air just behind a front of `mach` is than the air ahead, as a share of it.
pub fn density_jump(mach: f64) -> f64 {
    let m2 = mach * mach;
    2.4 * m2 / (0.4 * m2 + 2.0) - 1.0
}

/// The turn (rad) of a ray grazing the front at scaled distance `z`, at one crossing: √(2 δn), the complement of the
/// critical angle for the step in refractive index.
pub fn limb_bend(z: f64) -> f64 {
    (2.0 * REFRACTIVITY * density_jump(mach(z))).sqrt()
}

/// The speed (m/s) of the air just behind the front at scaled distance `z`: 5/6 c (M − 1/M).
pub fn gust_mps(z: f64) -> f64 {
    let mach = mach(z);
    5.0 / 6.0 * SOUND * (mach - 1.0 / mach)
}

/// The pressure pulse's length behind the front (m) at scaled distance `z` for a blast of `scale`.
pub fn band_m(z: f64, scale: f64) -> f64 {
    scale * (BAND + BAND_PER_Z * z)
}

/// How far (m) the air behind the front carries what floats in it as the pulse passes, at scaled distance `z` from
/// a blast of `scale`: the gust's speed for the pulse's duration.
pub fn shove_m(z: f64, scale: f64) -> f64 {
    gust_mps(z) * band_m(z, scale) / SOUND
}

/// A gun's blast toward `outward` (unit, from its centre) against its strength along `bore`: its overpressure as
/// (1/2 + 1/2 cos)^1.3, never below 0.18 (naval-water-effects' blast sheet; `shaders/shock.wgsl`'s `lean`).
fn lean(outward: DVec3, bore: DVec3) -> f64 {
    if bore == DVec3::ZERO {
        return 1.0;
    }
    (0.5 + 0.5 * outward.dot(bore)).max(0.18).powf(1.3)
}

/// The seconds the front of a blast of `scale` (∛kg of TNT; 0 for a sound with no front) takes to reach `distance`
/// m from it: faster than sound near the blast, at sound's pace from there on.
pub fn arrival_seconds(scale: f64, distance: f64) -> f64 {
    let distance = distance.max(0.0);
    if !(scale.is_finite() && scale > 0.0) {
        return distance / SOUND;
    }
    let (mut radius, mut seconds) = (START_Z * scale, 0.0);
    while radius < distance {
        let z = radius / scale;
        // Past the front's run it is a sound wave.
        if z >= END_Z {
            return seconds + (distance - radius) / SOUND;
        }
        let step = (0.08 * radius).min(distance - radius);
        seconds += step / (SOUND * mach(z));
        radius += step;
    }
    seconds
}

/// The underpressure (kPa) that takes air of relative `humidity` (0–1) to its dew point: its dew-point depression,
/// about a fifth of a kelvin per percent short of saturation, cooled adiabatically (1.23 kPa a kelvin at sea level).
pub fn veil_underpressure_kpa(humidity: f64) -> f64 {
    let depression_k = (1.0 - humidity.clamp(0.0, 1.0)) * 20.0;
    1.231 * depression_k
}

/// The scaled distance out to which a cloud condenses behind the pulse in air of `humidity`; None where it never
/// does.
pub fn veil_reach(humidity: f64) -> Option<f64> {
    let needed = veil_underpressure_kpa(humidity).max(1e-6);
    let reach = (UNDERPRESSURE / needed).min(VEIL_MAX_Z);
    (reach > VEIL_INNER_Z).then_some(reach)
}

/// The TNT (kg) the muzzle blast of `barrels` guns of `caliber_m` fired together is worth.
pub fn gun_charge_kg(caliber_m: f64, barrels: usize) -> f64 {
    GUN_TNT_PER_CALIBRE3 * caliber_m.max(0.0).powi(3) * barrels.max(1) as f64
}

/// One blast's front.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shock {
    /// Where it went off (scene m).
    pub centre: DVec3,
    /// ∛ of its TNT (kg): metres per unit of scaled distance.
    pub scale: f64,
    /// The front's radius now (m), a step ago, and the seconds since it went off.
    pub radius: f64,
    pub before: f64,
    pub age: f64,
    /// A gun's line of fire (unit); zero for a burst, which is the same all round.
    pub bore: DVec3,
    /// Its own pattern in the cloud.
    pub seed: f32,
}

impl Shock {
    /// The front's scaled distance.
    pub fn scaled(&self) -> f64 {
        self.radius / self.scale
    }

    /// The front `dt` seconds on: its radius carried at the shock's speed, in steps short against the radius.
    fn advance(&mut self, dt: f64) {
        self.age += dt;
        self.before = self.radius;
        let mut left = dt;
        while left > 0.0 {
            let speed = SOUND * mach(self.scaled());
            let step = left.min((0.08 * self.radius / speed).max(2e-5));
            self.radius += speed * step;
            left -= step;
        }
    }
}

/// A front's last step as the smoke it passed takes it: what lay in the pulse during the step is carried outward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shove {
    pub centre: DVec3,
    /// The front's radius before the step and after it (m), and the pulse's length behind it.
    pub before: f64,
    pub now: f64,
    pub pulse: f64,
    /// How far the whole pulse carries smoke here (m).
    pub reach: f64,
    /// A gun's line of fire; zero for a burst.
    pub bore: DVec3,
    /// Seconds since the blast: smoke younger than this is the blast's own, thrown by its own recipe.
    pub age: f64,
}

/// A shove under this (m) is not worth walking the smoke for.
const MIN_SHOVE: f64 = 0.05;

impl Shove {
    /// How far `point` is carried by this step: outward from the blast, by the share of the pulse that passed over
    /// it, so the steps of a front's whole passage add up to [`Shove::reach`] at any frame rate.
    pub fn at(&self, point: DVec3) -> DVec3 {
        let spoke = point - self.centre;
        let range = spoke.length();
        if range <= self.before - self.pulse || range > self.now || range < 1e-3 {
            return DVec3::ZERO;
        }
        let passed =
            (self.now.min(range + self.pulse) - self.before.max(range)).max(0.0) / self.pulse;
        let outward = spoke / range;
        outward * (self.reach * passed * lean(outward, self.bore))
    }
}

/// A front as `shaders/shock.wgsl` takes it (its `Shock`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ShockInstance {
    /// xyz the centre (scene m), w the front's radius (m).
    pub centre: [f32; 4],
    /// x the pulse's length (m), y the turn at one crossing (rad), z the lean along the bore (0 a burst, 1 a gun),
    /// w 1 where the camera is given the whole view.
    pub band: [f32; 4],
    /// xyz the line of fire, w the seed.
    pub bore: [f32; 4],
    /// The cloud: x its inner and y its outer radius (m), z its extinction (1/m), w the blast's scale (m).
    pub veil: [f32; 4],
    /// The pulse on what lies under it: x how far further out the scene there is read (m), y the ruffled sea's
    /// opacity.
    pub surface: [f32; 4],
}

/// The fronts in the air.
#[derive(Clone, Debug, PartialEq)]
pub struct ShockWaves {
    /// The surface air's relative humidity, 0–1: how far a cloud condenses behind a front.
    pub humidity: f64,
    /// Off: nothing is kept.
    pub enabled: bool,
    shocks: Vec<Shock>,
}

impl Default for ShockWaves {
    fn default() -> Self {
        Self {
            humidity: 0.78,
            enabled: true,
            shocks: Vec::new(),
        }
    }
}

impl ShockWaves {
    pub fn shocks(&self) -> &[Shock] {
        &self.shocks
    }

    pub fn is_empty(&self) -> bool {
        self.shocks.is_empty()
    }

    pub fn clear(&mut self) {
        self.shocks.clear();
    }

    /// Takes `other`'s fronts and air, keeping this list's room.
    pub fn follow(&mut self, other: &Self) {
        self.humidity = other.humidity;
        self.enabled = other.enabled;
        self.shocks.clone_from(&other.shocks);
    }

    /// A turret's muzzle blast.
    pub fn gun(&mut self, blast: &TurretBlast) {
        if blast.caliber_m < MIN_GUN_CALIBRE {
            return;
        }
        let bore = blast.bore.try_normalize().unwrap_or(DVec3::X);
        self.spawn(
            blast.muzzle + bore * (GUN_CENTRE_CALIBRES * blast.caliber_m),
            gun_charge_kg(blast.caliber_m, blast.barrels),
            bore,
            blast.sequence,
        );
    }

    /// An explosion of `charge_kg` of TNT in the air at `at`.
    pub fn burst(&mut self, at: DVec3, charge_kg: f64, sequence: u64) {
        self.spawn(at, charge_kg, DVec3::ZERO, sequence);
    }

    fn spawn(&mut self, centre: DVec3, charge_kg: f64, bore: DVec3, sequence: u64) {
        if !self.enabled || !centre.is_finite() || !charge_kg.is_finite() || charge_kg <= 0.0 {
            return;
        }
        if self.shocks.len() == MAX_SHOCKS {
            let spent = self
                .shocks
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.scaled().total_cmp(&b.1.scaled()))
                .map(|(at, _)| at)
                .expect("full");
            self.shocks.swap_remove(spent);
        }
        let scale = charge_kg.cbrt();
        self.shocks.push(Shock {
            centre,
            scale,
            radius: START_Z * scale,
            before: START_Z * scale,
            age: 0.0,
            bore,
            seed: (sequence % 4096) as f32 * 0.618_034,
        });
    }

    /// Carries every front `dt` seconds on and drops those run out.
    pub fn advance(&mut self, dt: f64) {
        if !self.enabled {
            self.shocks.clear();
            return;
        }
        if !(dt.is_finite() && dt > 0.0) {
            // A held frame (a pause, a redraw) is no step: nothing is carried again.
            for shock in &mut self.shocks {
                shock.before = shock.radius;
            }
            return;
        }
        for shock in &mut self.shocks {
            shock.advance(dt);
        }
        self.shocks.retain(|shock| shock.scaled() < END_Z);
    }

    /// Each front's last step, for the smoke it passed, into `out`: none from a front too weak to move any.
    pub fn shoves(&self, out: &mut Vec<Shove>) {
        out.clear();
        out.extend(self.shocks.iter().filter_map(|shock| {
            let z = shock.scaled();
            let reach = shove_m(z, shock.scale);
            (reach >= MIN_SHOVE && shock.radius > shock.before).then(|| Shove {
                centre: shock.centre,
                before: shock.before,
                now: shock.radius,
                pulse: band_m(z, shock.scale),
                reach,
                bore: shock.bore,
                age: shock.age,
            })
        }));
    }

    /// What `eye` (scene m) can see of the fronts, farthest first, into `out`.
    pub fn publish(&self, eye: DVec3, out: &mut Vec<ShockInstance>) {
        out.clear();
        let reach = veil_reach(self.humidity);
        let mut seen: Vec<(f64, ShockInstance)> = Vec::with_capacity(self.shocks.len());
        for shock in &self.shocks {
            let z = shock.scaled();
            let band = band_m(z, shock.scale);
            let fade = 1.0 - smoothstep(z, FADE_Z, END_Z);
            let bend = SHOWN * limb_bend(z) * fade;
            let push = PUSH
                * shock.scale
                * (density_jump(mach(z)) / density_jump(mach(5.0)))
                    .sqrt()
                    .min(1.5)
                * fade;
            let ruffle = RUFFLE
                * (1.0 - (-gust_mps(z) / RUFFLE_GUST).exp())
                * smoothstep(shock.scale, RUFFLE_SCALES[0], RUFFLE_SCALES[1])
                * fade;
            let (inner, outer, extinction) = reach.map_or((0.0, 0.0, 0.0), |reach| {
                let thick = VEIL_LENGTH * shock.scale;
                let outer = (shock.radius - band).min(reach * shock.scale);
                let inner = (shock.radius - band - thick).max(VEIL_INNER_Z * shock.scale);
                let deep = outer - inner;
                if deep <= 0.0 {
                    return (0.0, 0.0, 0.0);
                }
                // It comes and goes as the shell enters and leaves the air it condenses in.
                let gun = if shock.bore == DVec3::ZERO {
                    1.0
                } else {
                    GUN_VEIL
                };
                let extinction = VEIL_EXTINCTION * gun * smoothstep(deep, 0.0, 0.4 * thick);
                let deepest = 2.0 * (outer * outer - inner * inner).sqrt();
                if extinction * deepest < MIN_VEIL_DEPTH {
                    (0.0, 0.0, 0.0)
                } else {
                    // Drawn at least half its radius deep: a thinner shell reads as a pane of glass, not as mist.
                    (inner.min(VEIL_DRAWN_INNER * outer), outer, extinction)
                }
            });
            let distance = eye.distance(shock.centre);
            let inside = distance < shock.radius * WHOLE_VIEW;
            // From well inside the front nothing grazes it: only its foot on the sea and a cloud show.
            let grazing = !inside || shock.radius - distance < band * BAND_REACH;
            let bend = if grazing { bend } else { 0.0 };
            if bend < MIN_BEND && extinction == 0.0 && ruffle < MIN_RUFFLE {
                continue;
            }
            seen.push((
                distance,
                ShockInstance {
                    centre: [
                        shock.centre.x as f32,
                        shock.centre.y as f32,
                        shock.centre.z as f32,
                        shock.radius as f32,
                    ],
                    band: [
                        band as f32,
                        bend as f32,
                        f32::from(u8::from(shock.bore != DVec3::ZERO)),
                        f32::from(u8::from(inside)),
                    ],
                    bore: [
                        shock.bore.x as f32,
                        shock.bore.y as f32,
                        shock.bore.z as f32,
                        shock.seed,
                    ],
                    veil: [
                        inner as f32,
                        outer as f32,
                        extinction as f32,
                        shock.scale as f32,
                    ],
                    surface: [push as f32, ruffle as f32, 0.0, 0.0],
                },
            ));
        }
        seen.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.extend(seen.into_iter().map(|(_, instance)| instance));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A triple 40.6 cm turret, fired level along +X from 12 m over the sea.
    fn turret() -> TurretBlast {
        TurretBlast {
            ship_id: "iowa".into(),
            muzzle: DVec3::new(0.0, 12.0, 0.0),
            bore: DVec3::X,
            caliber_m: 0.406,
            barrels: 3,
            sequence: 7,
        }
    }

    fn run(waves: &mut ShockWaves, seconds: f64) {
        let steps = (seconds * 240.0).round() as usize;
        for _ in 0..steps {
            waves.advance(1.0 / 240.0);
        }
    }

    #[test]
    fn the_front_leaves_faster_than_sound_and_settles_to_it() {
        assert!(mach(1.0) > 3.0 && mach(1.0) < 5.0);
        assert!((mach(5.0) - 1.12).abs() < 0.02);
        assert!(mach(20.0) < 1.03 && mach(20.0) > 1.0);
        let mut waves = ShockWaves::default();
        waves.gun(&turret());
        let scale = waves.shocks()[0].scale;
        // 250 kg of TNT: the blast sheet's reach of 56 m is its Z = 8.8.
        assert!((scale - 6.3).abs() < 0.2, "{scale}");
        run(&mut waves, 0.1);
        let radius = waves.shocks()[0].radius;
        // Sound alone would be 34 m out; the front has been supersonic all the way.
        assert!(radius > 40.0 && radius < 60.0, "{radius}");
        let before = radius;
        run(&mut waves, 0.1);
        let speed = (waves.shocks()[0].radius - before) / 0.1;
        assert!(speed > SOUND && speed < SOUND * 1.08, "{speed}");
    }

    #[test]
    fn a_blast_runs_out_within_its_scale() {
        let mut waves = ShockWaves::default();
        waves.gun(&turret());
        run(&mut waves, 0.4);
        assert_eq!(waves.shocks().len(), 1);
        run(&mut waves, 0.25);
        assert!(waves.is_empty(), "a turret's front is gone in 0.6 s");
        // A five-inch twin's is gone within two tenths of a second and 50 m.
        waves.gun(&TurretBlast {
            caliber_m: 0.127,
            barrels: 2,
            ..turret()
        });
        run(&mut waves, 0.1);
        assert!(waves.shocks()[0].radius < 50.0);
        run(&mut waves, 0.1);
        assert!(waves.is_empty());
        // An anti-aircraft gun's is not kept at all.
        waves.gun(&TurretBlast {
            caliber_m: 0.04,
            ..turret()
        });
        assert!(waves.is_empty());
    }

    #[test]
    fn the_bend_is_the_grazing_rays_and_fades_with_the_front() {
        assert!(
            (limb_bend(5.0) - 0.0108).abs() < 0.001,
            "{}",
            limb_bend(5.0)
        );
        assert!((gust_mps(5.0) - 66.0).abs() < 4.0, "{}", gust_mps(5.0));
        assert!(limb_bend(2.0) > limb_bend(5.0) && limb_bend(5.0) > limb_bend(20.0));
        let mut waves = ShockWaves::default();
        waves.burst(DVec3::new(0.0, 10.0, 0.0), 250.0, 1);
        let eye = DVec3::new(0.0, 30.0, 400.0);
        let mut out = Vec::new();
        run(&mut waves, 0.1);
        waves.publish(eye, &mut out);
        let early = out[0];
        run(&mut waves, 0.25);
        waves.publish(eye, &mut out);
        let late = out[0];
        assert!(early.band[1] > 2.0 * late.band[1], "{early:?} {late:?}");
        assert!(
            late.band[0] > early.band[0],
            "the pulse lengthens as it runs"
        );
        assert_eq!(
            early.band[3], 0.0,
            "seen from outside: a quad about the sphere"
        );
        // Its foot on the sea: a ripple of under a metre and a pale band, both fading with it.
        assert!(
            early.surface[0] > 0.25 && early.surface[0] < 1.0,
            "{early:?}"
        );
        assert!(early.surface[1] > 0.08 && early.surface[1] > 3.0 * late.surface[1]);
        // The front has passed a camera 40 m off and left it far behind: nothing grazes it from there, and only its
        // foot on the sea is left to see, over the whole view.
        waves.publish(DVec3::new(0.0, 10.0, 40.0), &mut out);
        assert_eq!((out[0].band[1], out[0].band[3]), (0.0, 1.0), "{out:?}");
        // Run out, it is dropped.
        run(&mut waves, 0.2);
        waves.publish(eye, &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_cloud_condenses_behind_big_blasts_in_damp_air_only() {
        assert!(veil_reach(0.5).is_none());
        let open_ocean = veil_reach(0.78).unwrap();
        assert!((open_ocean - 6.5).abs() < 0.3, "{open_ocean}");
        assert!(veil_reach(0.9).unwrap() > 2.0 * open_ocean);
        let eye = DVec3::new(0.0, 30.0, 600.0);
        let cloud = |charge: f64, humidity: f64, seconds: f64| {
            let mut waves = ShockWaves {
                humidity,
                ..Default::default()
            };
            waves.burst(DVec3::new(0.0, 10.0, 0.0), charge, 1);
            run(&mut waves, seconds);
            let mut out = Vec::new();
            waves.publish(eye, &mut out);
            // Its optical depth straight through the middle of the shell.
            out.first().map_or(0.0, |shock| {
                let [inner, outer, extinction, _] = shock.veil;
                extinction * 2.0 * (outer - inner)
            })
        };
        // A bomb of a heavy turret's worth: a pale cloud a tenth of a second on, gone a quarter of a second later.
        let turret = cloud(250.0, 0.78, 0.1);
        assert!(turret > 0.12 && turret < 0.5, "{turret}");
        assert_eq!(cloud(250.0, 0.78, 0.35), 0.0);
        // A five-inch shell's is nothing; a magazine's hides what is behind it; dry air makes none.
        assert_eq!(cloud(3.0, 0.78, 0.03), 0.0);
        assert!(cloud(8000.0, 0.78, 0.3) > 0.4);
        assert_eq!(cloud(8000.0, 0.5, 0.3), 0.0);
    }

    #[test]
    fn smoke_is_carried_as_far_as_the_gust_blows_while_the_pulse_passes() {
        // A metre at Z = 5 from a heavy turret, several nearer in, a hand's breadth from a five-inch gun.
        assert!(
            (shove_m(5.0, 6.3) - 1.0).abs() < 0.2,
            "{}",
            shove_m(5.0, 6.3)
        );
        assert!(shove_m(2.0, 6.3) > 3.0 && shove_m(5.0, 20.0) > 3.0);
        assert!(shove_m(5.0, 1.7) < 0.35);
        // A puff 60 m from a bomb's burst, carried by the front's whole passage at two frame rates.
        let carried = |hz: f64| {
            let mut waves = ShockWaves::default();
            waves.burst(DVec3::new(0.0, 10.0, 0.0), 250.0, 1);
            let mut puff = DVec3::new(60.0, 10.0, 0.0);
            let mut shoves = Vec::new();
            for _ in 0..(0.4 * hz) as usize {
                waves.advance(1.0 / hz);
                waves.shoves(&mut shoves);
                for shove in &shoves {
                    puff += shove.at(puff);
                }
            }
            puff.x - 60.0
        };
        // A held frame carries nothing again.
        let mut held = ShockWaves::default();
        held.burst(DVec3::ZERO, 250.0, 1);
        held.advance(0.1);
        let mut shoves = Vec::new();
        held.shoves(&mut shoves);
        assert_eq!(shoves.len(), 1);
        held.advance(0.0);
        held.shoves(&mut shoves);
        assert!(shoves.is_empty());
        let (smooth, coarse) = (carried(240.0), carried(30.0));
        let expected = shove_m(60.0 / 250f64.cbrt(), 250f64.cbrt());
        assert!(
            (smooth - expected).abs() < 0.2 * expected,
            "{smooth} {expected}"
        );
        assert!((coarse - smooth).abs() < 0.25 * smooth, "{coarse} {smooth}");
        // A gun's shove leans along its bore: less abeam, least behind.
        let shove = Shove {
            centre: DVec3::ZERO,
            before: 0.0,
            now: 30.0,
            pulse: 5.0,
            reach: 1.0,
            bore: DVec3::X,
            age: 0.1,
        };
        let ahead = shove.at(DVec3::new(28.0, 0.0, 0.0)).length();
        let abeam = shove.at(DVec3::new(0.0, 0.0, 28.0)).length();
        let astern = shove.at(DVec3::new(-28.0, 0.0, 0.0)).length();
        assert!(
            ahead > 2.0 * abeam && abeam > 2.0 * astern,
            "{ahead} {abeam} {astern}"
        );
        assert_eq!(
            shove.at(DVec3::new(40.0, 0.0, 0.0)),
            DVec3::ZERO,
            "ahead of the front"
        );
    }

    #[test]
    fn a_blast_arrives_sooner_than_its_sound_would_near_it() {
        // With no front, sound's pace.
        assert!((arrival_seconds(0.0, 340.0) - 1.0).abs() < 1e-9);
        // A heavy turret's front is 15 m ahead of sound a tenth of a second on and keeps that lead.
        let near = arrival_seconds(6.3, 49.0);
        assert!((near - 0.1).abs() < 0.01, "{near}");
        let far = arrival_seconds(6.3, 1000.0);
        assert!(far < 1000.0 / SOUND && far > 1000.0 / SOUND - 0.08, "{far}");
        // It agrees with the front as it is carried frame by frame.
        let mut waves = ShockWaves::default();
        waves.burst(DVec3::ZERO, 250.0, 1);
        run(&mut waves, 0.3);
        let radius = waves.shocks()[0].radius;
        assert!((arrival_seconds(250f64.cbrt(), radius) - 0.3).abs() < 0.01);
    }

    #[test]
    fn the_list_is_bounded_and_switches_off() {
        let mut waves = ShockWaves::default();
        for i in 0..40 {
            waves.burst(DVec3::new(f64::from(i), 5.0, 0.0), 50.0, i as u64);
            waves.advance(0.004);
        }
        assert_eq!(waves.shocks().len(), MAX_SHOCKS);
        // The oldest went first.
        assert!(waves.shocks().iter().all(|shock| shock.centre.x >= 16.0));
        waves.enabled = false;
        waves.advance(0.004);
        waves.burst(DVec3::ZERO, 50.0, 99);
        assert!(waves.is_empty());
    }
}
