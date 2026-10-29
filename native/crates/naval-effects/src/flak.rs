//! A heavy AA round's burst in the open air: an `aircraft-fire` event's round at its fuze's time
//! (`CombatEffects::update_airbursts`).
//!
//! As the war's photographs show a barrage (the 5-inch/38's over Yorktown at Midway and the fleet off Okinawa, the 8.8
//! and 10.5 cm flak about the bombers over Germany): the round's few kilograms of TNT or Explosive D burst in a flash
//! that films catch in a frame or two, a small yellow-white ball in an orange fringe. What it leaves is its soot, for the
//! filling burns short of oxygen: a cloud black from the first frames, a dozen metres across for a 5-inch round within
//! a fraction of a second, never a smooth ball but lumps and billows heaped on one another, crisp at its edge, some with
//! a tail or streamers torn off it where the casing broke. It hangs where it burst and drifts with the wind at its
//! height, swelling to twice its width in half a minute as the air mixes into it, so it greys and thins from its edges,
//! a grey smudge still marking the place most of a minute on; a barrage leaves the sky dotted with the salvos before
//! and, low over the sea, a haze of them. The fragments fly too fast and too cool to be seen by day.
//!
//! The cloud is one puff marched through its sphere (`EffectParticle::flak`, `gas_sprite.wgsl`'s `flak_soot`), its
//! lobes, streamers and billows drawn from its seed, so no two bursts are alike from any side.

use glam::DVec3;

use crate::blasts::FlashRequest;
use crate::combat::wind_share;
use crate::math::{Rgb, clamp};
use crate::particles::ParticlePool;
use crate::random::Lcg;

/// The calibre the sizes are for: the 5-inch/38's (m), with some 3.2 kg of explosive. The cloud's width goes with the
/// cube root of the charge, and the charge with the cube of the calibre: an 8.8 cm round's 0.87 kg.
const REFERENCE_CALIBRE: f64 = 0.127;
/// The soot's width (m) a third of a second after a 5-inch round bursts, as its lobes and billows stand out from the
/// sphere's middle (`gas_sprite.wgsl`'s `flak_body`): some eleven metres of black seen across.
const CLOUD: f64 = 15.0;
/// The puff's sphere over the cloud's width: its lobes, streamers and billows reach into the margin
/// (`gas_sprite.wgsl`'s `FLAK_CLOUD`, the cloud's share of the sphere, 0.7).
const SPHERE: f64 = 1.0 / 0.7;
/// How much the cloud swells in its first seconds as the burst's gas slows (a share of its first width), and how fast
/// (1/s): a fifth wider two seconds on, a third five seconds on.
const SWELL: f64 = 0.3;
const SWELL_RATE: f64 = 0.4;
/// How fast it widens after, as the air mixes into it, a share of its first width a second: nearly twice as wide half a
/// minute on, nearly three times a minute and a half on.
const SPREAD: f64 = 0.018;
/// How long the soot lasts (s), at least and at most: a grey smudge most of a minute on, thinned to nothing by the end
/// (`EffectParticle::fade`).
const LINGER: [f64; 2] = [80.0, 110.0];
/// The optical depth through a fresh cloud's middle over `gas_sprite.wgsl`'s `FLAK_DEPTH` (`EffectParticle::density`
/// is three times this): it falls as the square of the cloud's width as the soot spreads (`EffectParticle::flak`).
const THICKNESS: f64 = 1.0;
/// Its dispersal's time (s, `EffectParticle::dissipation_time`): its thin parts tear away and it greys as it goes.
const DISPERSES: f64 = 50.0;
/// How long its filling burns orange inside the soot after the flash (s, `EffectParticle::cooling`).
const AFTERBURN: f64 = 0.2;
/// How long the flash's white-yellow ball burns (s), and its orange fringe.
const FLASH: f64 = 0.05;
const FRINGE: f64 = 0.12;
/// How long its light shines on the scene (s): the aircraft beside it, the smoke about it.
const LIGHT: f64 = 0.09;

/// Fresh soot's reflectance: black, faintly brown (`gas_sprite.wgsl` greys it as it disperses).
fn fresh_soot() -> Rgb {
    Rgb::new(0.05, 0.046, 0.043)
}

/// One heavy AA round's burst: where, the round's calibre, how long ago (s; a frame that comes late draws it at its age,
/// one older than the soot lasts nothing) and the wind ten metres up (the pools', carried at the burst's height).
pub(crate) struct Airburst {
    pub at: DVec3,
    pub caliber_m: f64,
    pub age: f64,
    pub wind: DVec3,
}

/// The pools a burst emits into: its soot's, its flash's sprites and its flash's light.
pub(crate) struct FlakPools<'a> {
    pub soot: &'a mut ParticlePool,
    pub fire: &'a mut ParticlePool,
    pub flashes: &'a mut Vec<FlashRequest>,
}

/// The burst's size against a 5-inch round's: a 3-inch round's 0.6, an 8.8 cm's 0.7, a 10.5 cm's 0.83.
pub(crate) fn scale(caliber_m: f64) -> f64 {
    clamp(caliber_m / REFERENCE_CALIBRE, 0.5, 1.5)
}

/// Emits the burst.
pub(crate) fn burst(burst: &Airburst, random: &mut Lcg, pools: FlakPools<'_>) {
    let s = scale(burst.caliber_m);
    // Every draw is taken whatever the burst's age, so a late frame draws the burst a timely one would.
    let width = CLOUD * s * (0.85 + random.next() * 0.3);
    let life = LINGER[0] + random.next() * (LINGER[1] - LINGER[0]);
    let seed = random.next() * 100.0;
    let shade = 0.9 + random.next() * 0.2;
    if burst.age >= life {
        return;
    }
    let FlakPools {
        soot,
        fire,
        flashes,
    } = pools;
    if burst.age < LIGHT {
        flashes.push(FlashRequest {
            position: burst.at,
            power: 900.0 * s * s,
            duration: LIGHT - burst.age,
            distance: 60.0 * s,
            ..Default::default()
        });
    }
    if burst.age < FLASH {
        let ball = fire.emit(burst.at, None);
        ball.size = 2.5 * s;
        ball.growth = 120.0 * s;
        ball.growth_decay = 30.0;
        ball.life = FLASH;
        ball.age = burst.age;
        ball.color = Rgb::new(6.0, 5.2, 3.6);
    }
    if burst.age < FRINGE {
        let fringe = fire.emit(burst.at, None);
        fringe.size = 4.0 * s;
        fringe.growth = 50.0 * s;
        fringe.growth_decay = 12.0;
        fringe.life = FRINGE;
        fringe.age = burst.age;
        fringe.opacity = 0.85;
        fringe.color = Rgb::new(3.2, 1.3, 0.35);
    }
    let p = soot.emit(burst.at, None);
    p.flak = true;
    p.size = width * SPHERE;
    p.growth = width * SPHERE * SWELL * SWELL_RATE;
    p.growth_decay = SWELL_RATE;
    p.diffusion = width * SPHERE * SPREAD;
    p.life = life;
    p.age = burst.age;
    p.heat = 1.0;
    p.cooling = AFTERBURN;
    p.density = 3.0 * THICKNESS;
    p.dissipation_time = DISPERSES;
    p.opacity = 1.0;
    p.wind = 1.0;
    p.seed = seed;
    p.color = fresh_soot().scale(shade);
    // Carried where the wind at its height would have taken it by a late frame.
    p.position +=
        DVec3::new(burst.wind.x, 0.0, burst.wind.z) * (wind_share(burst.at.y) * burst.age);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::PoolKind;

    fn pools() -> (ParticlePool, ParticlePool, Vec<FlashRequest>) {
        (
            ParticlePool::new(16, PoolKind::Gas),
            ParticlePool::new(16, PoolKind::Additive),
            Vec::new(),
        )
    }

    fn burst_at(
        at: DVec3,
        age: f64,
        wind: DVec3,
        seed: f64,
    ) -> (ParticlePool, ParticlePool, Vec<FlashRequest>) {
        let (mut soot, mut fire, mut flashes) = pools();
        burst(
            &Airburst {
                at,
                caliber_m: 0.127,
                age,
                wind,
            },
            &mut Lcg::from_number(seed),
            FlakPools {
                soot: &mut soot,
                fire: &mut fire,
                flashes: &mut flashes,
            },
        );
        (soot, fire, flashes)
    }

    #[test]
    fn a_burst_flashes_then_leaves_one_cloud_of_soot_that_spreads_and_thins() {
        let (soot, fire, flashes) = burst_at(DVec3::new(0.0, 900.0, 0.0), 0.0, DVec3::ZERO, 7.0);
        assert_eq!(flashes.len(), 1);
        assert_eq!(fire.alive(), 2, "the white ball and its orange fringe");
        assert_eq!(soot.alive(), 1);
        let p = &soot.particles()[0];
        assert!(p.flak);
        // A 5-inch round's cloud some 13 to 17 m across in a sphere 1/0.7 as wide.
        let width = p.current_size() / SPHERE;
        assert!((12.5..17.5).contains(&width), "{width}");
        let mut aged = p.clone();
        aged.age = 2.0;
        let swollen = aged.current_size() / p.current_size();
        assert!(
            (1.15..1.25).contains(&swollen),
            "a fifth wider two seconds on: {swollen}"
        );
        aged.age = 30.0;
        let grown = aged.current_size() / p.current_size();
        assert!(
            (1.8..2.0).contains(&grown),
            "nearly twice as wide half a minute on: {grown}"
        );
        // Its soot spread through four times the area across.
        assert!((aged.density_now() * grown * grown - p.density_now()).abs() < 1e-9);
    }

    #[test]
    fn a_late_frame_draws_the_burst_at_its_age_and_a_stale_one_nothing() {
        let wind = DVec3::new(9.0, 0.0, 0.0);
        let (soot, fire, flashes) = burst_at(DVec3::new(0.0, 900.0, 0.0), 2.0, wind, 7.0);
        assert!(
            flashes.is_empty() && fire.alive() == 0,
            "the flash is long out"
        );
        let p = &soot.particles()[0];
        assert_eq!(p.age, 2.0);
        assert!(
            (p.position.x - 2.0 * 9.0 * wind_share(900.0)).abs() < 1e-9,
            "carried by the wind aloft"
        );
        let (timely, ..) = burst_at(DVec3::new(0.0, 900.0, 0.0), 0.0, wind, 7.0);
        assert_eq!(p.size, timely.particles()[0].size, "the same burst");
        let (stale, ..) = burst_at(DVec3::new(0.0, 900.0, 0.0), 120.0, wind, 7.0);
        assert_eq!(stale.alive(), 0);
    }

    #[test]
    fn calibres_scale_the_cloud() {
        assert!((scale(0.088) - 0.693).abs() < 1e-3);
        assert_eq!(scale(0.02), 0.5);
        assert_eq!(scale(0.203), 1.5);
    }
}
