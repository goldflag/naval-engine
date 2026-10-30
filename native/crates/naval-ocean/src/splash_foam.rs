//! The foam a shell splash leaves on the sea: a lumpy deposit of its own ([`SplashDeposits`]), laid as the column's
//! water falls back. Within the foam field's window ([`crate::foam_sim`]) the field lays and carries it; beyond the
//! window, and on a tier with no field, the surface draws it from the `Wake` uniform's rows (`wake.wgsl`'s
//! `splash_read`): lobes from its hash, pushed out by the base surge and skewed downwind, white as the column's water
//! falls back (from 1.5 s, densest 4–8 s, held to 12 s), breaking into clumps and streaks by 20 s and leaving a faint
//! scar of pale, stilled water and sparse streaks to two minutes, carried by the surface's drift. The `Wake` uniform
//! holds [`SPLASH_SLOTS`] of them: with more on the water, the least seen gives up its slot, drawing into itself over
//! [`LEAVING`], before the fresh one that needs it shows. A charge going off in the sea (a burster, a torpedo's warhead,
//! a depth charge, a magazine) leaves a deposit too, as wide as its gas bubble grows ([`charge_scale`]); a shell's
//! burster going off in its own splash only widens that splash's ([`SplashDeposits::lay_charge`]).

use std::collections::VecDeque;

use crate::foam_sim::LANDING;
use crate::wake::impulses::bubble_radius;

/// Deposits the `Wake` uniform holds, two rows each.
pub const SPLASH_SLOTS: usize = 64;
pub const SPLASH_ROWS: usize = 2 * SPLASH_SLOTS;
/// A splash's deposit is gone this long (s) after it fell.
pub const SPLASH_LIFETIME: f64 = 120.0;
/// A deposit draws nothing this young (s), its column still up (`wake.wgsl`'s `splash_foam`: its bubbles from 1 s, its
/// white water from 1.3 s): a fresh one waits this long for a slot of the uniform's.
const UNSEEN: f64 = 1.0;
/// A deposit giving up its slot to a fresh one draws into itself over this long (s), shorter than [`UNSEEN`], so the
/// slot is free when the fresh one shows.
pub const LEAVING: f64 = 0.9;
/// How long (s) the wind carries a 38 cm round's falling column: its deposit's skew downwind is the wind's speed times
/// this (scaled with the round, as the column is).
const SKEW_SECONDS: f64 = 1.3;
/// A charge going off within this distance (m) of a deposit this young (s) is taken for that splash's own shell's
/// burster (an HE shell's at once, an AP shell's after its fuze's delay, at most about 30 m on): the deposit takes the
/// larger scale rather than a second laying beside it. Nothing of a deposit is laid or drawn before 0.5 s (the field's
/// bubbles from 0.5 s, its water from 0.8 s; the surface's from 1 s), so its growing never shows.
const JOIN_REACH: f64 = 30.0;
const JOIN_AGE: f64 = 0.5;
/// The deposit's radius (m) at `age` (s) for a round of `scale` ([`SplashDeposit::scale`]): the base surge pushes it out
/// over its first seconds, then it creeps as it spreads. `wake.wgsl`'s `splash_radius` draws the same.
const RADIUS_START: f64 = 7.0;
const RADIUS_SURGE: f64 = 25.0;
const SURGE_SECONDS: f64 = 2.8;
const RADIUS_CREEP: f64 = 0.22;

/// The scale of a round's splash of `caliber_m` (1 for a 38 cm round; an aircraft's crash and a torpedo's entry as
/// rounds of their own calibres).
pub fn splash_scale(caliber_m: f64) -> f64 {
    (caliber_m.max(0.05) / 0.38).powf(0.65)
}

/// The scale of the white water a charge of `kg` TNT going off `depth` m under the sea leaves: its deposit starts as
/// wide as the charge's gas bubble grows ([`bubble_radius`], Cole's 3.4 (W / (z + 10))^⅓ m). A 533 mm torpedo's 350 kg
/// warhead 4 m down lays 1.4 times a 38 cm round's splash, a 610 mm one's 1.6; a depth charge's 130 kg at 10 to 15 m
/// 0.9 to 0.85, its dome's wide white circle; a magazine going up beside the water (2 t at 6 m) 2.4. A shell's
/// burster at the surface lays less than its own splash (a 12.7 cm HE shell's 2 kg 0.28 to its 0.49, a 46 cm one's
/// 117 kg 1.10 to 1.13), so it only widens a bomb's.
pub fn charge_scale(kg: f64, depth: f64) -> f64 {
    bubble_radius(kg, depth) / RADIUS_START
}

pub fn splash_radius(age: f64, scale: f64) -> f64 {
    scale
        * (RADIUS_START
            + RADIUS_SURGE * (1.0 - (-age.max(0.0) / SURGE_SECONDS).exp())
            + RADIUS_CREEP * age.max(0.0))
}

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How white a deposit draws where it lies thickest at `age` (s), 0–1, `wake.wgsl`'s `splash_cover` and its fading
/// with age: its white water (counted from its fall, while that is still to come), then the clumps it breaks into,
/// thinning from 12 s and gone by two minutes.
pub fn splash_showing(age: f64) -> f64 {
    let white = if age < 4.5 {
        1.0
    } else {
        1.0 - smoothstep(9.0, 15.0, age)
    };
    let broken = smoothstep(7.0, 13.0, age)
        * (-(age - 12.0).max(0.0) / 22.0).exp()
        * (1.0 - smoothstep(85.0, 118.0, age));
    (0.92 * white).max(0.4 * broken) * (1.0 - 0.6 * smoothstep(12.0, 50.0, age))
}

/// One splash's deposit: where it lies now (carried by the surface's drift since it fell), when it fell, its round's
/// scale (1 for a 38 cm round), its hash (0–1, from the point it fell at), its skew downwind (m) and when it began to
/// give up its slot (none: it keeps it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplashDeposit {
    pub x: f64,
    pub z: f64,
    pub born: f64,
    pub scale: f64,
    pub seed: f64,
    pub skew: [f64; 2],
    pub leaving: Option<f64>,
}

impl SplashDeposit {
    /// Its scale as drawn at `now`: its own, or, giving up its slot, shrinking to nothing over [`LEAVING`] (its lobes
    /// draw in to where it fell, so it never vanishes at once).
    pub fn shown_scale(&self, now: f64) -> f64 {
        match self.leaving {
            Some(from) => self.scale * (1.0 - smoothstep(0.0, LEAVING, now - from)).max(0.01),
            None => self.scale,
        }
    }

    /// How much of the view it takes at `now`, seen from `eye` (none: by its size alone): how white it draws
    /// ([`splash_showing`]) times the share of the view its radius takes, r² / (d² + r²).
    fn seen(&self, now: f64, eye: Option<[f64; 3]>) -> f64 {
        let age = now - self.born;
        let r = splash_radius(age, self.scale);
        let size = eye.map_or(r * r, |eye| {
            let d = [self.x - eye[0], eye[1], self.z - eye[2]];
            r * r / (d[0] * d[0] + d[1] * d[1] + d[2] * d[2] + r * r)
        });
        splash_showing(age) * size
    }
}

/// The splashes' deposits, oldest first.
#[derive(Clone, Debug, Default)]
pub struct SplashDeposits {
    pub list: VecDeque<SplashDeposit>,
    /// The wind (m/s, world x and z) and the eye of the last [`SplashDeposits::update`], for the charges laid between
    /// updates ([`SplashDeposits::lay_charge`]).
    wind: [f64; 2],
    eye: Option<[f64; 3]>,
}

impl SplashDeposits {
    /// Advance to `now`: carry every deposit `dt` seconds on the surface's `drift` (m/s, world x and z), drop the gone
    /// and lay this frame's `fresh` splashes ((x, z, scale: [`splash_scale`])). Each fresh one past [`SPLASH_SLOTS`] has
    /// the least seen of the others (from `eye`, the camera, when there is one) give up its slot
    /// ([`SplashDeposits::give_up_slot`]).
    pub fn update(
        &mut self,
        now: f64,
        dt: f64,
        drift: [f64; 2],
        fresh: &[(f64, f64, f64)],
        eye: Option<[f64; 3]>,
    ) {
        let drift = if drift.iter().all(|v| v.is_finite()) {
            drift
        } else {
            [0.0; 2]
        };
        for splash in &mut self.list {
            splash.x += drift[0] * dt;
            splash.z += drift[1] * dt;
        }
        self.list.retain(|s| {
            now - s.born <= SPLASH_LIFETIME && s.leaving.is_none_or(|from| now - from < LEAVING)
        });
        self.wind = drift.map(|v| v / crate::oil::DRIFT_SHARE);
        self.eye = eye;
        for &(x, z, scale) in fresh {
            self.lay(now, x, z, scale);
        }
    }

    /// Lay a charge's white water at `now` (on the clock of the last update), where it went off (world `x`, `z`) and of
    /// `scale` ([`charge_scale`]): its own deposit, or, where a deposit under [`JOIN_AGE`] lies within [`JOIN_REACH`]
    /// (the splash of the shell whose burster this is), that deposit grown to `scale` if smaller.
    pub fn lay_charge(&mut self, now: f64, x: f64, z: f64, scale: f64) {
        if !(scale > 0.0 && scale.is_finite() && x.is_finite() && z.is_finite()) {
            return;
        }
        let splash = self
            .list
            .iter_mut()
            .rev()
            .take_while(|s| now - s.born < JOIN_AGE)
            .find(|s| s.leaving.is_none() && (s.x - x).hypot(s.z - z) < JOIN_REACH);
        match splash {
            Some(splash) => {
                if scale > splash.scale {
                    splash.scale = scale;
                    splash.skew = self.wind.map(|w| w * SKEW_SECONDS * scale);
                }
            }
            None => self.lay(now, x, z, scale),
        }
    }

    /// A fresh deposit at `now`; past [`SPLASH_SLOTS`] the least seen of the others gives up its slot.
    fn lay(&mut self, now: f64, x: f64, z: f64, scale: f64) {
        // A hash of the point it fell at: every view of one splash lays the same deposit.
        let seed = ((x * 12.9898 + z * 78.233).sin() * 43758.5453).abs() % 1.0;
        self.list.push_back(SplashDeposit {
            x,
            z,
            born: now,
            scale,
            seed,
            skew: self.wind.map(|w| w * SKEW_SECONDS * scale),
            leaving: None,
        });
        if self.list.iter().filter(|s| s.leaving.is_none()).count() > SPLASH_SLOTS {
            self.give_up_slot(now, self.eye);
        }
    }

    /// The least seen deposit keeping its slot begins to give it up ([`SplashDeposit::seen`]): one that has landed
    /// (the field has laid all its water) before one still landing, and a fresh one that draws nothing yet, at once,
    /// only when every other is fresh too.
    fn give_up_slot(&mut self, now: f64, eye: Option<[f64; 3]>) {
        let rank = |s: &SplashDeposit| {
            let age = now - s.born;
            (
                u8::from(age < LANDING) + u8::from(age < UNSEEN),
                s.seen(now, eye),
            )
        };
        let least = self
            .list
            .iter()
            .enumerate()
            .filter(|(_, s)| s.leaving.is_none())
            .min_by(|(_, a), (_, b)| {
                let (a, b) = (rank(a), rank(b));
                a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
            })
            .map(|(i, s)| (i, now - s.born < UNSEEN));
        match least {
            Some((i, true)) => {
                self.list.remove(i);
            }
            Some((i, false)) => self.list[i].leaving = Some(now),
            None => {}
        }
    }

    pub fn translate(&mut self, offset: [f64; 2]) {
        for splash in &mut self.list {
            splash.x += offset[0];
            splash.z += offset[1];
        }
        if let Some(eye) = &mut self.eye {
            eye[0] += offset[0];
            eye[2] += offset[1];
        }
    }

    /// Carries the deposits into a scene laid on another chart: their places, and their skews as directions.
    pub fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        for splash in &mut self.list {
            let at = [splash.x, 0.0, splash.z];
            splash.skew = relay.direction(at, splash.skew);
            [splash.x, splash.z] = relay.point2([splash.x, splash.z]);
        }
        self.eye = None;
    }

    pub fn clear(&mut self) {
        self.list.clear();
    }

    /// The `Wake.splashes` rows at `now`, youngest first: (x, z, age, scale as drawn) and (seed, skew x, skew z, reach),
    /// the reach (m) a cull radius about (x, z) that holds the deposit's lobes, skew and warp. More deposits than slots
    /// lie on the water only while some give theirs up to fresh ones, which draw nothing yet and wait. Returns the count.
    pub fn rows(&self, now: f64, rows: &mut [[f32; 4]]) -> usize {
        let mut waiting = self.list.len().saturating_sub(SPLASH_SLOTS);
        let mut count = 0;
        for splash in self.list.iter().rev() {
            let age = (now - splash.born).max(0.0);
            if waiting > 0 && age < UNSEEN && splash.leaving.is_none() {
                waiting -= 1;
                continue;
            }
            if count == SPLASH_SLOTS {
                break;
            }
            let scale = splash.shown_scale(now);
            let reach = 2.1 * splash_radius(age, scale) + splash.skew[0].hypot(splash.skew[1]);
            rows[2 * count] = [splash.x as f32, splash.z as f32, age as f32, scale as f32];
            rows[2 * count + 1] = [
                splash.seed as f32,
                splash.skew[0] as f32,
                splash.skew[1] as f32,
                reach as f32,
            ];
            count += 1;
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deposit(x: f64, born: f64) -> SplashDeposit {
        SplashDeposit {
            x,
            z: 0.0,
            born,
            scale: 1.0,
            seed: 0.5,
            skew: [0.0; 2],
            leaving: None,
        }
    }

    #[test]
    fn a_deposit_drifts_and_is_gone_after_its_life() {
        let mut splashes = SplashDeposits::default();
        let drift = [0.25, 0.0];
        splashes.update(0.0, 0.0, drift, &[(100.0, 50.0, 1.0)], None);
        let first = splashes.list[0];
        assert!((first.scale - 1.0).abs() < 1e-9);
        // Skewed downwind: the wind is the drift over its share.
        assert!(first.skew[0] > 5.0 && first.skew[1].abs() < 1e-9);
        let mut now = 0.0;
        for _ in 0..100 {
            now += 0.5;
            splashes.update(now, 0.5, drift, &[], None);
        }
        assert!((splashes.list[0].x - (100.0 + 0.25 * 50.0)).abs() < 1e-6);
        let mut rows = [[0.0_f32; 4]; SPLASH_ROWS];
        assert_eq!(splashes.rows(now, &mut rows), 1);
        assert!((rows[0][2] - 50.0).abs() < 1e-4);
        assert!(f64::from(rows[1][3]) > 2.0 * splash_radius(50.0, 1.0));
        while now < SPLASH_LIFETIME + 1.0 {
            now += 0.5;
            splashes.update(now, 0.5, drift, &[], None);
        }
        assert!(splashes.list.is_empty());
    }

    #[test]
    fn a_full_uniform_gives_up_a_slot_over_a_second_before_the_fresh_splash_shows() {
        let mut splashes = SplashDeposits::default();
        for i in 0..SPLASH_SLOTS {
            let now = i as f64;
            splashes.update(
                now,
                1.0,
                [0.0; 2],
                &[(now * 10.0, 0.0, splash_scale(0.2))],
                None,
            );
        }
        let now = SPLASH_SLOTS as f64;
        splashes.update(now, 1.0, [0.0; 2], &[(-50.0, 0.0, splash_scale(0.2))], None);
        // None goes at once: the oldest, the least seen of these, begins to give up its slot.
        assert_eq!(splashes.list.len(), SPLASH_SLOTS + 1);
        assert_eq!(splashes.list[0].leaving, Some(now));
        assert_eq!(
            splashes.list.iter().filter(|s| s.leaving.is_some()).count(),
            1
        );
        // The uniform holds the leaving one, drawing into itself, while the fresh one (nothing to draw yet) waits.
        let mut rows = [[0.0_f32; 4]; SPLASH_ROWS];
        let mut last = f32::MAX;
        for tenth in 0..=8 {
            let t = now + f64::from(tenth) / 10.0;
            splashes.update(t, 0.1, [0.0; 2], &[], None);
            assert_eq!(splashes.rows(t, &mut rows), SPLASH_SLOTS);
            let pairs: Vec<&[[f32; 4]]> = rows.chunks(2).take(SPLASH_SLOTS).collect();
            assert!(pairs.iter().all(|r| r[0][0] != -50.0));
            let leaving = pairs
                .iter()
                .find(|r| r[0][0] == 0.0)
                .expect("the leaving deposit");
            assert!(
                leaving[0][3] < last || tenth == 0,
                "it shrinks: {} after {last}",
                leaving[0][3]
            );
            last = leaving[0][3];
        }
        assert!(f64::from(last) < 0.1 * splashes.list[0].scale);
        // A second on, it is gone and the fresh one, now showing, has its slot.
        let t = now + 1.0;
        splashes.update(t, 0.2, [0.0; 2], &[], None);
        assert_eq!(splashes.list.len(), SPLASH_SLOTS);
        assert_eq!(splashes.rows(t, &mut rows), SPLASH_SLOTS);
        assert_eq!(rows[0][0], -50.0);
        assert!(rows.chunks(2).take(SPLASH_SLOTS).all(|r| r[0][0] != 0.0));
    }

    #[test]
    fn the_least_seen_deposit_gives_up_its_slot() {
        let leaving = |splashes: &SplashDeposits| -> Vec<f64> {
            splashes
                .list
                .iter()
                .filter(|s| s.leaving.is_some())
                .map(|s| s.x)
                .collect()
        };
        let eye = Some([0.0, 30.0, 0.0]);
        // Of deposits as old, the one far from the camera.
        let mut far = SplashDeposits::default();
        far.list.extend(
            (0..SPLASH_SLOTS)
                .map(|i| deposit(if i == 17 { 5000.0 } else { 100.0 + i as f64 }, 0.0)),
        );
        far.update(30.0, 0.0, [0.0; 2], &[(50.0, 0.0, 1.0)], eye);
        assert_eq!(leaving(&far), vec![5000.0]);
        // Of deposits about one place, the faint scar of an old one before the broken foam of younger ones, larger as
        // it has spread.
        let mut old = SplashDeposits::default();
        old.list.push_back(deposit(7.0, -70.0));
        old.list
            .extend((1..SPLASH_SLOTS).map(|i| deposit(i as f64, 10.0)));
        assert!(splash_showing(100.0) < 0.02 * splash_showing(20.0));
        old.update(30.0, 0.0, [0.0; 2], &[(50.0, 0.0, 1.0)], None);
        assert_eq!(leaving(&old), vec![7.0]);
        // One still landing keeps its slot while one that has landed is there, though it lies far off: the furthest
        // of those landed goes.
        let mut landing = SplashDeposits::default();
        landing
            .list
            .extend((0..SPLASH_SLOTS - 1).map(|i| deposit(100.0 + i as f64, 0.0)));
        landing.list.push_back(deposit(5000.0, 25.0));
        landing.update(30.0, 0.0, [0.0; 2], &[(50.0, 0.0, 1.0)], eye);
        assert_eq!(leaving(&landing), vec![100.0 + (SPLASH_SLOTS - 2) as f64]);
        // Only when every other fell this second too does one go at once: it draws nothing yet.
        let mut burst = SplashDeposits::default();
        burst
            .list
            .extend((0..SPLASH_SLOTS).map(|i| deposit(i as f64, 30.0)));
        burst.update(30.0, 0.0, [0.0; 2], &[(-1.0, 0.0, 1.0)], eye);
        assert_eq!(burst.list.len(), SPLASH_SLOTS);
        assert!(leaving(&burst).is_empty());
    }

    /// A charge's white water is as wide as its gas bubble: a torpedo's warhead and a magazine lay more than a 38 cm
    /// round's splash, a depth charge's dome about as much; a shell's burster less than its own splash.
    #[test]
    fn a_charge_lays_white_water_as_wide_as_its_bubble() {
        let torpedo = charge_scale(350.0, 4.0);
        assert!((1.35..1.5).contains(&torpedo), "533 mm: {torpedo}");
        let long_lance = charge_scale(350.0 * (0.61_f64 / 0.533).powi(3), 4.0);
        assert!((1.55..1.7).contains(&long_lance), "610 mm: {long_lance}");
        for depth in [10.0, 15.0] {
            let dome = charge_scale(130.0, depth);
            assert!(
                (0.8..0.95).contains(&dome),
                "depth charge at {depth} m: {dome}"
            );
        }
        let magazine = charge_scale(2000.0, 6.0);
        assert!((2.3..2.5).contains(&magazine), "magazine: {magazine}");
        // The HE shells' bursters (kg) at the surface, against their splashes (calibre m).
        for (caliber, kg) in [
            (0.127, 2.0),
            (0.15, 3.624),
            (0.2032, 12.156),
            (0.38, 64.0),
            (0.46, 116.8),
        ] {
            assert!(
                charge_scale(kg, 0.0) < splash_scale(caliber),
                "{caliber} m: {} against {}",
                charge_scale(kg, 0.0),
                splash_scale(caliber)
            );
        }
    }

    /// A charge going off in a splash under half a second old (its shell's burster) widens that splash's deposit, never
    /// narrows it, and lays none of its own; one elsewhere, or later, lays its own.
    #[test]
    fn a_charge_widens_its_shells_splash_or_lays_its_own() {
        let mut deposits = SplashDeposits::default();
        let drift = [0.2, 0.0];
        deposits.update(10.0, 0.1, drift, &[(0.0, 0.0, splash_scale(0.38))], None);
        let skew = deposits.list[0].skew;
        // An HE shell's burster at once, an AP shell's 12 m on: its splash is wider already.
        deposits.lay_charge(10.0, 0.0, 0.0, charge_scale(64.0, 0.0));
        deposits.lay_charge(10.04, 12.0, 0.0, charge_scale(20.0, 3.0));
        assert_eq!(deposits.list.len(), 1);
        assert!((deposits.list[0].scale - 1.0).abs() < 1e-9);
        // A bomb's 125 kg: its near miss is wider than its splash.
        let bomb = charge_scale(125.0, 0.0);
        assert!(bomb > 1.1);
        deposits.lay_charge(10.1, 5.0, 3.0, bomb);
        assert_eq!(deposits.list.len(), 1);
        assert!((deposits.list[0].scale - bomb).abs() < 1e-9);
        assert!((deposits.list[0].skew[0] - skew[0] * bomb).abs() < 1e-9);
        assert_eq!([deposits.list[0].x, deposits.list[0].z], [0.0, 0.0]);
        // A torpedo hit far off lays its own, skewed downwind; one in the splash once it has begun to land lays its own.
        deposits.lay_charge(10.2, 300.0, 0.0, charge_scale(350.0, 4.0));
        deposits.lay_charge(10.6, 0.0, 0.0, charge_scale(350.0, 4.0));
        assert_eq!(deposits.list.len(), 3);
        let torpedo = deposits.list[1];
        assert_eq!([torpedo.x, torpedo.z, torpedo.born], [300.0, 0.0, 10.2]);
        assert!((torpedo.scale - charge_scale(350.0, 4.0)).abs() < 1e-9);
        assert!(torpedo.skew[0] > skew[0] && torpedo.skew[1].abs() < 1e-9);
        let mut rows = [[0.0_f32; 4]; SPLASH_ROWS];
        assert_eq!(deposits.rows(10.6, &mut rows), 3);
        // A charge with no size lays nothing.
        deposits.lay_charge(11.0, 50.0, 50.0, f64::NAN);
        deposits.lay_charge(11.0, 50.0, 50.0, 0.0);
        assert_eq!(deposits.list.len(), 3);
    }
}
