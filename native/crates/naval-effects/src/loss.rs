//! A ship's loss as the air and the sea keep it: her funnels venting as the sea reaches her boilers and her uptakes,
//! the moment she is gone (all of her under the sea), and what she leaves on the water ([`crate::flotsam`]). Read from
//! what the frames already carry: her pose, her flooding (`ShipFrame::flooded`) and her definition (her boiler rooms, her
//! funnels' bases, her waterplane). Not a port.
//!
//! **Her funnels venting.** No warship's account was found; the merchant ships' are the reference. As Titanic's boiler
//! rooms flooded her safety valves lifted and her funnels roared with steam (Maltin, US Inquiry), and at the break
//! "smoke and sparks rushed from the funnels" in "a big black cloud" (Bride, Dahl); Lusitania's sea, reaching her
//! funnels, blew a passenger back out of one, black with soot (Larson, *Dead Wake*, 2015). So:
//!
//! - When one of her boiler rooms floods (a quarter full, `ShipFrame::flooded`), the funnel over it (the one nearest
//!   along her length) vents steam for [`STEAM_SECONDS`], white with a little of her uptakes' soot: the sea on hot
//!   boilers and her safety valves lifting.
//! - When the sea reaches a funnel's base while its mouth is still clear, the air her sinking hull drives out of her
//!   rushes up it, black with soot: her waterplane, a share of it to each funnel, settling at the rate she sinks there,
//!   pushes [`UPTAKE_SHARE`] of its air out through the funnel's mouth (the rest through her hatches and ventilators). A
//!   battleship settling 0.5 m/s blows some 10 m/s out of each funnel, one going down by the head at 3 m/s some 40. When
//!   the mouth goes under, its last air bursts white on the water.

use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec3;

use crate::flotsam::{Flotsam, Scatter};
use crate::math::{Rgb, ShipPose};
use crate::particles::{ParticlePool, SourceId};
use crate::random::Lcg;
use crate::ships::{ShipEffectsProfile, ShipFrame};

/// Seconds a funnel vents steam once the boiler room under it floods.
pub const STEAM_SECONDS: f64 = 10.0;
/// The share of the air a settling hull drives out that goes up her funnels (the rest through her hatches,
/// ventilators and the holes in her): an estimate.
pub const UPTAKE_SHARE: f64 = 0.25;
/// Seconds of white water and steam where a funnel's mouth goes under.
const MOUTH_SECONDS: f64 = 3.0;
/// A funnel's up axis must stand this high (cos 60°) for her air to come out of it as a jet.
const STANDING: f64 = 0.5;
/// The vented puffs' density at the mouth (the gas shader's: 3 an optical depth of `GAS_DEPTH` 12 at a body's 1):
/// steam stopping some two fifths of the light through its middle and soot half, as a working funnel's smoke at its rim
/// (`funnel.rs`); they thin as they widen (`EffectParticle::thins`). They were 1.6 and 2.6 until 2026-10-21, born as
/// wide as the mouth and never thinning as they grew: from 2 km, solid dark balls on the sea.
const VENT_STEAM_DENSITY: f64 = 0.75;
const VENT_SOOT_DENSITY: f64 = 1.0;
/// The fastest the air comes out of a funnel (m/s): the jets are short and broken.
const FASTEST: f64 = 40.0;
/// Seconds over which a hull's settling rate is smoothed.
const RATE_SECONDS: f64 = 1.5;
/// Seconds after a hull was last drawn that she is forgotten.
const FORGET_SECONDS: f64 = 30.0;
/// A hull still burning above the water within this many seconds of going under takes fire down with her: her own
/// fire is the sustained source her fresh, thick oil needs to light (heavy fuel oil needs "longer heating times and a
/// hotter flame", Fingas, ARPEL Report 28, 2007), as Blücher's burning superstructure set her spilled oil alight as she
/// rolled over (Haarr and Melien, 2023) and Fuso went down "amidst a huge oil fire" (Tully, 2009). A magazine's flash
/// alone lights nothing: it is over before her oil comes up (Barham's and Juneau's survivors swam in thick, unburnt oil).
/// The host lights it only in a sea that holds a burn (`naval_ocean::oil::sea_holds_a_burn`).
pub const FIRE_MEMORY_SECONDS: f64 = 60.0;

/// One funnel's state.
#[derive(Clone, Copy, Debug, Default)]
struct Funnel {
    /// Seconds of steam still to vent, from a flooded boiler room.
    steam: f64,
    /// Its mouth has gone under; seconds of its last burst on the water still to come.
    drowned: bool,
    burst: f64,
    credit: f64,
}

/// What the effects remember of one hull.
#[derive(Clone, Debug)]
struct Hull {
    profile: Arc<ShipEffectsProfile>,
    pose: ShipPose,
    seen: f64,
    /// Each boiler room's flooding last frame (`ShipEffectsProfile::boiler_rooms`' order).
    boilers: Vec<bool>,
    funnels: Vec<Funnel>,
    /// Each funnel base's height over the sea last frame, and how fast it is going down (m/s, smoothed).
    bases: Vec<Option<f64>>,
    sinking: Vec<f64>,
    /// When all of her went under the sea.
    gone: Option<f64>,
    random: Lcg,
    /// When a fire of hers last burned above the water.
    fire_seen: f64,
}

/// A hull that went under this frame: where, for the host (her oil, the sea's boil).
#[derive(Clone, Debug, PartialEq)]
pub struct WentUnder {
    pub ship: SourceId,
    pub position: DVec3,
    pub heading: f64,
    /// Her length (m): her oil comes up, and is lit, over it.
    pub length: f64,
    /// She took fire down with her ([`FIRE_MEMORY_SECONDS`]): her oil burns where it lies thick.
    pub ignites: bool,
}

/// The losses the effects follow.
#[derive(Debug, Default)]
pub struct Losses {
    hulls: HashMap<SourceId, Hull>,
    pub flotsam: Flotsam,
    time: f64,
    /// The hulls that went under in the last update.
    pub went_under: Vec<WentUnder>,
    /// Funnels venting now (steam or air), for tools.
    pub venting: usize,
}

/// Whether all of her is under the sea: every corner of the box about her hull and upperworks (her draft below the
/// water line to her highest structure above it, her length and beam) lies below it.
pub fn all_under(pose: &ShipPose, profile: &ShipEffectsProfile) -> bool {
    // Her ends stand to her deck (her freeboard, at least 2 m), her upperworks over her middle half to their top: an
    // end rising out of the sea as she goes down by the other carries no mast.
    let low = -profile.hull_draft.max(1.0);
    let deck = (profile.hull_depth - profile.hull_draft).max(2.0);
    let top = profile.top_m.max(deck);
    let (beam, length) = (profile.hull_beam * 0.5, profile.hull_length * 0.5);
    for (along, high) in [(length, deck), (0.5 * length, top)] {
        for corner in 0..8 {
            let local = DVec3::new(
                if corner & 1 == 0 { -beam } else { beam },
                if corner & 2 == 0 { low } else { high },
                if corner & 4 == 0 { -along } else { along },
            );
            if pose.local_to_world(local).y > 0.0 {
                return false;
            }
        }
    }
    true
}

impl Losses {
    pub fn new() -> Self {
        Self {
            flotsam: Flotsam::new(),
            ..Self::default()
        }
    }

    pub fn reset(&mut self) {
        self.hulls.clear();
        self.flotsam.reset();
        self.went_under.clear();
    }

    /// Moves what is remembered by `offset` (a theatre scene's re-centre).
    pub fn translate(&mut self, offset: DVec3) {
        self.flotsam.translate(offset);
        for hull in self.hulls.values_mut() {
            hull.pose.x += offset.x;
            hull.pose.z += offset.z;
        }
    }

    /// Whether `ship` has gone under, and since when (the effects' clock).
    pub fn gone(&self, ship: &str) -> Option<f64> {
        self.hulls.get(ship).and_then(|h| h.gone)
    }

    /// Seconds since `ship` went under (none while she is afloat, or one first drawn already under).
    pub fn since_gone(&self, ship: &str) -> Option<f64> {
        self.gone(ship)
            .filter(|gone| gone.is_finite())
            .map(|gone| self.time - gone)
    }

    /// A fire of `ship`'s burns above the water now.
    pub fn note_fire(&mut self, ship: &str) {
        let now = self.time;
        if let Some(hull) = self.hulls.get_mut(ship) {
            hull.fire_seen = now;
        }
    }

    /// A frame: every hull's boilers and funnels watched, their venting emitted into `smoke` (gas; billows of the
    /// smoke's simulation when `flipbook`, the game's look), and a hull that has gone under scatters her flotsam.
    pub fn update(
        &mut self,
        dt: f64,
        wind: DVec3,
        ships: &[ShipFrame],
        smoke: &mut ParticlePool,
        flipbook: bool,
    ) {
        self.went_under.clear();
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt;
        let now = self.time;
        self.venting = 0;
        for ship in ships {
            let profile = &ship.profile;
            let hull = self.hulls.entry(ship.id.clone()).or_insert_with(|| Hull {
                profile: profile.clone(),
                pose: ship.pose,
                seen: now,
                boilers: profile
                    .boiler_rooms
                    .iter()
                    .map(|&room| ship.flooded.get(room).copied().unwrap_or(false))
                    .collect(),
                funnels: vec![Funnel::default(); profile.funnels.len()],
                bases: vec![None; profile.funnels.len()],
                sinking: vec![0.0; profile.funnels.len()],
                // First drawn already under (a wreck in a scene just loaded): long gone, no flotsam now.
                gone: all_under(&ship.pose, profile).then_some(f64::NEG_INFINITY),
                random: Lcg::from_number(id_seed(&ship.id)),
                fire_seen: f64::NEG_INFINITY,
            });
            if !Arc::ptr_eq(&hull.profile, profile) {
                hull.profile = profile.clone();
                hull.funnels = vec![Funnel::default(); profile.funnels.len()];
                hull.bases = vec![None; profile.funnels.len()];
                hull.sinking = vec![0.0; profile.funnels.len()];
                hull.boilers = vec![false; profile.boiler_rooms.len()];
            }
            hull.seen = now;
            hull.pose = ship.pose;
            // A boiler room flooding: the funnel over it vents steam.
            for (k, &room) in profile.boiler_rooms.iter().enumerate() {
                let flooded = ship.flooded.get(room).copied().unwrap_or(false);
                if flooded && !hull.boilers[k] && hull.gone.is_none() {
                    let z = profile.room_z.get(room).copied().unwrap_or(0.0);
                    if let Some(f) = nearest_funnel(profile, z) {
                        hull.funnels[f].steam = STEAM_SECONDS;
                    }
                }
                hull.boilers[k] = flooded;
            }
            // Each funnel: its base and mouth against the sea, its up axis its section's.
            for (f, outlet) in profile.funnels.iter().enumerate() {
                let up = ship.pose_at(outlet.position).rotate(DVec3::Y);
                let base_local = DVec3::new(
                    outlet.position.x,
                    profile
                        .funnel_bases
                        .get(f)
                        .copied()
                        .unwrap_or(outlet.position.y * 0.5),
                    outlet.position.z,
                );
                // Each by the section that holds it, once she has broken.
                let base = ship.place(base_local).y;
                let mouth = ship.place(outlet.position);
                let rate = match hull.bases[f] {
                    Some(last) => (last - base) / dt,
                    None => 0.0,
                };
                hull.bases[f] = Some(base);
                let k = 1.0 - (-dt / RATE_SECONDS).exp();
                hull.sinking[f] += (rate - hull.sinking[f]) * k;
                let funnel = &mut hull.funnels[f];
                // A funnel standing within 60° of upright: one lying along the sea as she rolls over vents no jet (her
                // air bubbles up through the water, the sea's boils).
                let standing = up.y > STANDING;
                if mouth.y <= 0.0 {
                    if !funnel.drowned && hull.gone.is_none() {
                        funnel.drowned = true;
                        funnel.burst = if standing { MOUTH_SECONDS } else { 0.0 };
                    }
                } else {
                    funnel.drowned = false;
                }
                // Air: the sea at its base, its mouth clear, her hull settling there.
                let air =
                    if ship.lost && standing && base <= 0.0 && mouth.y > 0.0 && hull.gone.is_none()
                    {
                        let area = outlet.width.max(1.0) * outlet.length.max(1.0);
                        let waterplane =
                            profile.waterplane_m2 / profile.funnels.len().max(1) as f64;
                        (UPTAKE_SHARE * waterplane / area * hull.sinking[f].max(0.0)).min(FASTEST)
                    } else {
                        0.0
                    };
                let steam = funnel.steam > 0.0 && mouth.y > 0.0;
                let burst = funnel.burst > 0.0;
                if air > 2.0 || steam || burst {
                    self.venting += 1;
                }
                // Puffs a second, laid closer than they are wide (a stream, not a string of balls): a jet's worth, a
                // funnel's steam, the burst at its mouth.
                let rate = if air > 2.0 {
                    8.0 + air * 0.6
                } else if steam || burst {
                    8.0
                } else {
                    0.0
                };
                funnel.credit += rate * dt;
                let random = &mut hull.random;
                while funnel.credit >= 1.0 {
                    funnel.credit -= 1.0;
                    let width = outlet.width.max(outlet.length).max(2.0);
                    let (at, velocity, white) = if air > 2.0 {
                        (
                            mouth,
                            up * air * (0.7 + 0.3 * random.next()) + ship.velocity,
                            0.5,
                        )
                    } else if steam {
                        (
                            mouth,
                            up * (10.0 + 6.0 * random.next()) + ship.velocity,
                            0.85,
                        )
                    } else {
                        (
                            DVec3::new(mouth.x, 0.6, mouth.z),
                            DVec3::new(0.0, 8.0 + 6.0 * random.next(), 0.0),
                            1.0,
                        )
                    };
                    let jitter =
                        DVec3::new(random.next() - 0.5, 0.0, random.next() - 0.5) * width * 0.4;
                    let steamy = random.next() < white;
                    let shade = 0.85 + 0.3 * random.next();
                    // A stream's puff: under half the mouth's width as it leaves it, widening as the jet takes in air
                    // (some two and a half widths in five seconds) and thinning as it widens (`thins`): as dense at the
                    // mouth as a working funnel's smoke (`funnel.rs`, a rim stopping some half of the light), never a
                    // solid ball.
                    let p = smoke.emit(at + jitter, Some(&ship.id));
                    p.velocity = velocity;
                    p.size = width * (0.35 + 0.2 * random.next());
                    p.growth = width * 0.9;
                    p.growth_decay = 0.35;
                    p.diffusion = 0.4;
                    p.drag = 0.6;
                    p.gravity = if steamy { -1.5 } else { -0.8 };
                    p.wind = 0.8;
                    p.life = if steamy { 7.0 } else { 12.0 } + 3.0 * random.next();
                    p.fade_in = 0.1;
                    p.density = if steamy {
                        VENT_STEAM_DENSITY
                    } else {
                        VENT_SOOT_DENSITY
                    };
                    p.thins = true;
                    p.opacity = if steamy { 0.7 } else { 0.85 };
                    p.cooling = 0.0;
                    p.dissipation_time = if steamy { 5.0 } else { 8.0 };
                    p.color = if steamy {
                        Rgb::new(0.86, 0.88, 0.9)
                    } else {
                        Rgb::hex(0x55575a).scale(shade)
                    };
                    p.angle = random.next() * std::f64::consts::TAU;
                    if flipbook {
                        // The game's look: billows of the simulated smoke, as her funnel's own.
                        p.flipbook = crate::flipbook::clip::SMOKE;
                        p.size *= 1.25;
                    }
                }
                funnel.steam = (funnel.steam - dt).max(0.0);
                funnel.burst = (funnel.burst - dt).max(0.0);
            }
            // All of her under: she is gone, and what floated free of her comes up.
            if hull.gone.is_none() && (ship.lost || ship.sunk) && all_under(&ship.pose, profile) {
                hull.gone = Some(now);
                let position = ship.pose.position();
                self.went_under.push(WentUnder {
                    ship: ship.id.clone(),
                    position,
                    heading: ship.pose.heading,
                    length: profile.hull_length,
                    ignites: now - hull.fire_seen < FIRE_MEMORY_SECONDS,
                });
                let mut random = Lcg::from_number(id_seed(&ship.id) + 17.0);
                self.flotsam.scatter(
                    &Scatter {
                        x: position.x,
                        z: position.z,
                        heading: ship.pose.heading,
                        length: profile.hull_length,
                        beam: profile.hull_beam,
                    },
                    &mut random,
                    Some(&ship.id),
                );
            }
        }
        self.flotsam.update(dt, wind);
        self.hulls.retain(|_, h| now - h.seen < FORGET_SECONDS);
    }
}

/// The funnel nearest a point `z` along her length (her frame), if she has any.
fn nearest_funnel(profile: &ShipEffectsProfile, z: f64) -> Option<usize> {
    profile
        .funnels
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (a.1.position.z - z)
                .abs()
                .total_cmp(&(b.1.position.z - z).abs())
        })
        .map(|(i, _)| i)
}

/// A seed from a hull's id, so her venting draws the same on every run.
fn id_seed(id: &str) -> f64 {
    f64::from(id.bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::{EffectParticle, PoolKind};
    use crate::ships::FunnelOutlet;

    /// A 250 m battleship with two funnels over two boiler rooms.
    fn profile() -> Arc<ShipEffectsProfile> {
        Arc::new(ShipEffectsProfile {
            hull_length: 250.0,
            hull_beam: 36.0,
            hull_draft: 9.5,
            top_m: 40.0,
            waterplane_m2: 6500.0,
            funnels: vec![
                FunnelOutlet {
                    id: "funnel".into(),
                    position: DVec3::new(0.0, 28.0, -10.0),
                    width: 8.0,
                    length: 12.0,
                    bearing_rad: None,
                    smoke: 1.0,
                },
                FunnelOutlet {
                    id: "aft-funnel".into(),
                    position: DVec3::new(0.0, 26.0, 30.0),
                    width: 8.0,
                    length: 12.0,
                    bearing_rad: None,
                    smoke: 1.0,
                },
            ],
            funnel_bases: vec![12.0, 12.0],
            boiler_rooms: vec![1, 2],
            room_z: vec![-60.0, -12.0, 32.0],
            ..Default::default()
        })
    }

    fn frame(profile: &Arc<ShipEffectsProfile>, y: f64, pitch: f64, flooded: &[bool]) -> ShipFrame {
        let mut ship = ShipFrame::new(
            "ship",
            profile.clone(),
            ShipPose {
                y,
                pitch,
                ..ShipPose::default()
            },
        );
        ship.flooded = flooded.to_vec();
        ship.lost = true;
        ship
    }

    /// Steam, by the puffs' shade, and soot, emitted since `from`.
    fn steam_and_soot(smoke: &ParticlePool) -> (usize, usize) {
        smoke
            .particles()
            .iter()
            .filter(|p| p.age < p.life)
            .fold((0, 0), |(s, k), p| {
                if p.color.r > 0.6 {
                    (s + 1, k)
                } else {
                    (s, k + 1)
                }
            })
    }

    #[test]
    fn a_flooded_boiler_room_vents_steam_from_the_funnel_over_it() {
        let profile = profile();
        let mut losses = Losses::new();
        let mut smoke = ParticlePool::new(1024, PoolKind::Gas);
        for frame_index in 0..720 {
            let flooded = frame_index >= 60;
            let ship = frame(&profile, 0.0, 0.0, &[false, flooded, false]);
            smoke.advance(1.0 / 60.0, DVec3::ZERO);
            losses.update(1.0 / 60.0, DVec3::ZERO, &[ship], &mut smoke, false);
            if frame_index == 120 {
                assert_eq!(losses.venting, 1, "the fore funnel, over the flooded room");
                let (steam, _) = steam_and_soot(&smoke);
                assert!(steam > 2);
                // From the fore funnel's mouth, 28 m up.
                assert!(
                    smoke
                        .particles()
                        .iter()
                        .filter(|p| p.age < p.life)
                        .all(|p| (p.position.z + 10.0).abs() < 12.0 && p.position.y > 25.0)
                );
            }
        }
        // Ten seconds of it, then no more.
        assert_eq!(losses.venting, 0);
        assert!(losses.went_under.is_empty() && losses.gone("ship").is_none());
    }

    #[test]
    fn a_sinking_hull_blows_her_air_out_of_her_funnels_then_goes_and_leaves_her_flotsam() {
        let profile = profile();
        let mut losses = Losses::new();
        let mut smoke = ParticlePool::new(4096, PoolKind::Gas);
        let mut blowing = 0;
        let mut fastest = 0.0f64;
        let mut gone_at = None;
        // Settling level at 1.5 m/s from her water line to 60 m down.
        for frame_index in 0..(45 * 60) {
            let t = f64::from(frame_index) / 60.0;
            let ship = frame(&profile, -1.5 * t, 0.0, &[false, true, true]);
            smoke.advance(1.0 / 60.0, DVec3::ZERO);
            losses.update(1.0 / 60.0, DVec3::ZERO, &[ship], &mut smoke, false);
            // Her funnels blowing before their mouths go under (18.7 s and 17.3 s), and the fastest soot leaving them.
            if t < 17.0 && losses.venting > 0 {
                blowing += 1;
            }
            fastest = smoke
                .particles()
                .iter()
                .filter(|p| p.age >= 0.0 && p.age < 0.05 && p.color.r < 0.6)
                .map(|p| p.velocity.y)
                .fold(fastest, f64::max);
            if gone_at.is_none() && !losses.went_under.is_empty() {
                gone_at = Some(t);
                assert_eq!(losses.went_under[0].ship.as_ref(), "ship");
                assert!(!losses.went_under[0].ignites, "nothing burned in her");
            }
        }
        // Black air out of her funnels while the sea stood between their bases (12 m up) and their mouths (28 m):
        // some 10 s at 1.5 m/s, at 0.25 × 3,250 m² / 96 m² × 1.5 m/s, about 13 m/s.
        let seconds = f64::from(blowing) / 60.0;
        assert!((6.0..14.0).contains(&seconds), "blew for {seconds:.1} s");
        assert!((9.0..18.0).contains(&fastest), "at {fastest:.1} m/s");
        // Gone once her upperworks (40 m) are under: 27 s in; her flotsam comes up over the next minute.
        let gone_at = gone_at.expect("she went under");
        assert!((26.0..28.5).contains(&gone_at), "gone at {gone_at:.1} s");
        assert!(losses.flotsam.len() > 100);
        assert!(losses.flotsam.pieces().count() > 0);
    }

    /// Her vented smoke and steam leave the mouth no denser than a working funnel's smoke and thin as they widen, so
    /// none stands as a solid ball; a funnel lying along the sea as she rolls over vents no jet.
    #[test]
    fn venting_thins_as_it_widens_and_a_funnel_lying_down_vents_no_jet() {
        let profile = profile();
        let mut losses = Losses::new();
        let mut smoke = ParticlePool::new(4096, PoolKind::Gas);
        let drawn = |p: &EffectParticle| p.density * (p.size / p.current_size()).min(1.0);
        for frame_index in 0..(14 * 60) {
            let t = f64::from(frame_index) / 60.0;
            let ship = frame(&profile, -1.5 * t, 0.0, &[false, true, true]);
            smoke.advance(1.0 / 60.0, DVec3::ZERO);
            losses.update(1.0 / 60.0, DVec3::ZERO, &[ship], &mut smoke, false);
        }
        let puffs: Vec<&EffectParticle> = smoke
            .particles()
            .iter()
            .filter(|p| p.age >= 0.0 && p.age < p.life)
            .collect();
        assert!(!puffs.is_empty(), "no venting");
        for p in &puffs {
            assert!(p.thins && p.density <= VENT_SOOT_DENSITY);
            if p.age > 3.0 {
                assert!(
                    drawn(p) < 0.5 * p.density,
                    "{:.2} at {:.1} s",
                    drawn(p),
                    p.age
                );
            }
        }
        // Over on her beam ends, sinking as fast: her funnels lie along the sea and vent nothing.
        let mut rolled = Losses::new();
        let mut smoke = ParticlePool::new(4096, PoolKind::Gas);
        for frame_index in 0..(14 * 60) {
            let t = f64::from(frame_index) / 60.0;
            let mut ship = frame(&profile, -1.5 * t, 0.0, &[false, true, true]);
            ship.pose.roll = 1.3;
            smoke.advance(1.0 / 60.0, DVec3::ZERO);
            rolled.update(1.0 / 60.0, DVec3::ZERO, &[ship], &mut smoke, false);
            assert_eq!(rolled.venting, 0);
        }
        assert_eq!(smoke.alive(), 0);
    }
}
