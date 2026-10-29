//! Ready-use rounds cooking off in a burning ship's upperworks (`native/docs/combat-effects/fire-and-smoke.md`, "Fire inside her, and
//! cook-offs"). The clipped rounds and lockers beside her secondary and anti-aircraft guns (`naval_sim::fire_zones`: the
//! guns that stand in a zone) go off in its fire in irregular bursts, as the 20 mm and 40 mm did aboard Princeton,
//! Franklin and Bunker Hill (1944–45): each a pop and a flash, a spray of sparks, a small white puff of its propellant,
//! a sharp crack, and one to three of its rounds flung out of the fire, their tracers arcing away. Drawn and heard
//! only: the simulation owns what the fire wrecks. When and where follow from the ship, the gun and the effects' clock
//! alone (hashed), so the same fire pops the same way on every run.

use std::collections::HashMap;
use std::f64::consts::TAU;

use glam::DVec3;

use crate::blasts::{BlastPools, FlashRequest};
use crate::combat::EffectLighting;
use crate::math::{Rgb, smoothstep};
use crate::particles::{Align, SourceId};
use crate::random::Lcg;
use crate::shells::ShellState;
use crate::ships::ShipFrame;

/// A zone's guns start to cook off once it burns this hard (0–1), and do at their full rate from the second.
const BURNING_HARD: [f64; 2] = [0.45, 0.85];
/// The clock is cut into slots this long (s); a gun's burst starts in one by chance, in this share of them at the full
/// rate: about two bursts a minute a gun.
const SLOT_S: f64 = 4.0;
const BURST_SHARE: f64 = 0.14;
/// A burst: one pop to this many, spread over this many seconds.
const BURST_POPS: f64 = 4.0;
const BURST_S: f64 = 1.8;
/// A gun's ready-use rounds last this many seconds of its zone burning at the full rate, its bursts thinning as they
/// are spent.
const READY_USE_S: f64 = 360.0;
/// Where a gun's ready-use rounds lie about its mount (m): its lockers and clips round its tub or gunhouse, up to this
/// far out from its side, a little above its base (a round cooking off inside a gunhouse would not be seen).
const LOCKERS: [f64; 2] = [1.2, 0.8];
/// A round flung out of the fire: its speed (m/s, least and most; a round cooking off outside a barrel leaves slowly,
/// its case split), how long its tracer burns (s), its calibre (m, a 20 mm or 40 mm round's) and the drag it tumbles
/// with (1/s).
const THROWN: [f64; 2] = [18.0, 70.0];
const TRACER_BURN_S: [f64; 2] = [1.5, 4.0];
const TRACER_CALIBRE: f64 = 0.03;
const TUMBLE_DRAG: f64 = 0.35;
/// A round going off: its flash's radius (m, a lone round's to a clip's), how long it burns (s) and its colour (the
/// sun's units, orange-white); its light on the scene (`FlashRequest::power`, under the bloom's glare, which starts at
/// 3.75) and how far it reaches (m: the gun's side and the deck beside it), for how long (s). A few hundred grams of
/// propellant and a small burster: a spark of a flash, not a gun's.
const POP_SIZE: [f64; 2] = [0.25, 0.45];
const POP_S: f64 = 0.035;
const POP_COLOR: [f64; 3] = [3.0, 2.1, 1.0];
const POP_LIGHT: [f64; 2] = [1.0, 3.0];
const POP_REACH: [f64; 2] = [4.0, 6.0];
const POP_LIGHT_S: f64 = 0.03;
/// Rounds in flight at most, and pops waiting their moment.
const MAX_ROUNDS: usize = 96;
const MAX_PENDING: usize = 256;
/// Beyond this range (m) a pop lights nothing about it and throws no sparks: they would be under a pixel.
const LIT_WITHIN: f64 = 2500.0;

/// A round cooking off this frame, for its crack (the host's sound).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pop {
    /// Where (world).
    pub position: DVec3,
    /// 0–1: how large it goes off, one round or a clip of them.
    pub size: f64,
}

/// A round flung out of the fire, its tracer lit.
#[derive(Clone, Copy, Debug)]
struct Round {
    id: i64,
    owner: u64,
    position: DVec3,
    velocity: DVec3,
    age: f64,
    burn: f64,
}

/// A pop of a burst begun: when (the effects' clock), by which gun of which ship, and its draws.
#[derive(Clone, Debug)]
struct Pending {
    at: f64,
    ship: SourceId,
    mount: usize,
    draws: [f64; 4],
}

/// The rounds cooking off in every burning upperworks zone: their pops this frame and their rounds in flight.
#[derive(Debug, Default)]
pub struct CookOffs {
    time: f64,
    /// Seconds each gun's zone has burned hard (by ship and mount), weighted by how hard: its rounds spent.
    spent: HashMap<(SourceId, usize), f64>,
    pending: Vec<Pending>,
    rounds: Vec<Round>,
    pops: Vec<Pop>,
    next_id: i64,
    /// Pops since the start (the probe's).
    pub popped: u64,
}

/// FNV-1a of a ship's id, the seed of her guns' bursts.
fn seed_of(id: &str) -> u32 {
    id.bytes().fold(0x811c_9dc5, |h: u32, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// A slot's seed for one gun of a ship.
fn slot_seed(ship: u32, mount: usize, slot: i64) -> u32 {
    let mut h =
        ship ^ (mount as u32).wrapping_mul(0x9e37_79b9) ^ (slot as u32).wrapping_mul(0x85eb_ca6b);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^ (h >> 16)
}

impl CookOffs {
    /// Steps the rounds in flight `dt` seconds and pops every round due in the step: each burning zone's guns (`ships`'
    /// `zone_fires`, the profile's `zone_mounts`) burst while it burns hard, until their ready-use rounds are spent.
    /// `eye`: the camera's place (a far pop throws no sparks and lights nothing); `lighting` the night's lift, of which a
    /// pop takes the fires' share after dark (its flash and sparks the flames', its light the fires' light's), not a
    /// gun flash's.
    pub fn update(
        &mut self,
        dt: f64,
        eye: DVec3,
        ships: &[ShipFrame],
        pools: &mut BlastPools<'_>,
        lighting: &EffectLighting,
    ) {
        self.pops.clear();
        for round in &mut self.rounds {
            round.velocity.y -= 9.81 * dt;
            round.velocity *= (-TUMBLE_DRAG * dt).exp();
            round.position += round.velocity * dt;
            round.age += dt;
        }
        self.rounds
            .retain(|round| round.age < round.burn && round.position.y > 0.0);
        if dt <= 0.0 {
            return;
        }
        let (from, to) = (self.time, self.time + dt);
        self.time = to;
        for ship in ships.iter().filter(|ship| !ship.sunk) {
            let profile = &ship.profile;
            let seed = seed_of(&ship.id);
            for (k, fire) in ship.zone_fires.iter().enumerate() {
                let rate = smoothstep(fire.intensity, BURNING_HARD[0], BURNING_HARD[1]);
                let Some(mounts) = profile.zone_mounts.get(k).filter(|_| rate > 0.0) else {
                    continue;
                };
                for &mount in mounts.iter().filter(|&&j| j < profile.mounts.len()) {
                    let spent = self.spent.entry((ship.id.clone(), mount)).or_default();
                    *spent += dt * rate;
                    let left = 1.0 - *spent / READY_USE_S;
                    if left <= 0.0 {
                        continue;
                    }
                    // Each slot begun in this step bursts or not, once; its pops wait their moments.
                    let share = BURST_SHARE * rate * left.sqrt();
                    for slot in (from / SLOT_S).floor() as i64 + 1..=(to / SLOT_S).floor() as i64 {
                        let mut random = Lcg::new(slot_seed(seed, mount, slot));
                        if random.next() >= share {
                            continue;
                        }
                        let start = (slot as f64 + random.next()) * SLOT_S;
                        let count = 1 + (random.next() * BURST_POPS) as usize;
                        for n in 0..count {
                            let at = start + if n == 0 { 0.0 } else { random.next() * BURST_S };
                            let draws =
                                [random.next(), random.next(), random.next(), random.next()];
                            if self.pending.len() < MAX_PENDING {
                                self.pending.push(Pending {
                                    at,
                                    ship: ship.id.clone(),
                                    mount,
                                    draws,
                                });
                            }
                        }
                    }
                }
            }
        }
        // The pops due: by their ship as she lies now (one gone or sunk meanwhile pops no more).
        let mut pending = std::mem::take(&mut self.pending);
        pending.retain(|pop| {
            if pop.at > to {
                return true;
            }
            if let Some(ship) = ships.iter().find(|s| s.id == pop.ship && !s.sunk) {
                self.pop(ship, pop.mount, pop.draws, eye, pools, lighting);
            }
            false
        });
        self.pending = pending;
    }

    /// One round going off by `mount` of `ship`: its flash and light, sparks, puff and tracers, from four draws.
    fn pop(
        &mut self,
        ship: &ShipFrame,
        mount: usize,
        [place, size, thrown, seed]: [f64; 4],
        eye: DVec3,
        pools: &mut BlastPools<'_>,
        lighting: &EffectLighting,
    ) {
        let frame = ship.profile.mount_frame(mount, &ship.mount_trains);
        let [width, length, _] = ship.profile.mounts[mount].gunhouse;
        let (yaw, reach) = (
            place * TAU,
            0.5 * width.max(length) + LOCKERS[0] * (0.25 + 0.75 * size),
        );
        let local = DVec3::new(
            frame.x + yaw.cos() * reach,
            frame.y + LOCKERS[1],
            frame.z + yaw.sin() * reach,
        );
        let at = ship.place(local);
        if at.y <= 0.3 {
            return;
        }
        self.popped += 1;
        self.pops.push(Pop { position: at, size });
        let mut random = Lcg::from_number(seed * 4_294_967_296.0);
        let near = at.distance(eye) < LIT_WITHIN;
        // The night's lift the sprites and lights below take after: the fires' share of it, not a gun flash's.
        let (glow, shine) = (
            lighting.flames / lighting.lift.max(1.0),
            lighting.firelight / lighting.lift.max(1.0),
        );
        let by = |[least, most]: [f64; 2]| least + (most - least) * size;
        if near {
            pools.flashes.push(FlashRequest {
                position: at + DVec3::Y * 0.3,
                power: by(POP_LIGHT) * shine,
                duration: POP_LIGHT_S,
                distance: by(POP_REACH),
                ..Default::default()
            });
        }
        // The flash: its charge burning orange-white for a frame or two.
        let flash = pools.fire.emit(at, None);
        flash.size = by(POP_SIZE);
        flash.growth = 3.0;
        flash.growth_decay = 30.0;
        flash.life = POP_S;
        flash.color = Rgb::new(POP_COLOR[0], POP_COLOR[1], POP_COLOR[2]).scale(glow);
        flash.opacity = 1.0;
        if near {
            // Sparks: burning grains of its propellant and bits of its case, sprayed up and out.
            let sparks = 8 + (random.next() * 10.0) as usize;
            for _ in 0..sparks {
                let yaw = random.next() * TAU;
                let up = 0.15 + random.next() * 0.85;
                let flat = (1.0 - up * up).sqrt();
                let p = pools.fire.emit(at, None);
                p.velocity = DVec3::new(yaw.cos() * flat, up, yaw.sin() * flat)
                    * (5.0 + random.next().powf(0.6) * 20.0);
                p.size = 0.05 + random.next() * 0.05;
                p.stretch = 6.0 + random.next() * 6.0;
                p.align = Align::Velocity;
                p.life = 0.35 + random.next() * 0.6;
                p.gravity = 9.81;
                p.drag = 1.5;
                p.opacity = 0.95;
                p.color = Rgb::hex(0xffb35c).scale(2.2 * glow);
            }
        }
        // The puff: its propellant's smoke, white, rising and drifting off in the wind.
        let puff = pools.volumes.emit(at, None);
        puff.size = 0.35 + 0.25 * size;
        puff.growth = 1.6;
        puff.growth_decay = 2.0;
        puff.diffusion = 0.25;
        puff.life = 3.0 + random.next() * 1.5;
        puff.opacity = 0.5;
        puff.fade_in = 0.03;
        puff.density = 0.8;
        puff.dissipation_time = 1.8;
        puff.velocity = DVec3::new(0.0, 1.2, 0.0);
        puff.wind = 1.0;
        puff.seed = random.next() * 100.0;
        puff.color = Rgb::hex(0xcfcfca);
        // One to three of its rounds flung out of the fire, every way but down, their tracers lit.
        let rounds = 1 + (thrown * 3.0) as usize;
        let owner = u64::from(seed_of(&ship.id));
        for _ in 0..rounds {
            if self.rounds.len() >= MAX_ROUNDS {
                break;
            }
            let yaw = random.next() * TAU;
            let up = -0.1 + random.next() * 0.9;
            let flat = (1.0 - up * up).sqrt();
            let speed = THROWN[0] + random.next() * (THROWN[1] - THROWN[0]);
            self.next_id -= 1;
            self.rounds.push(Round {
                id: self.next_id,
                owner,
                position: at,
                velocity: DVec3::new(yaw.cos() * flat, up, yaw.sin() * flat) * speed
                    + ship.velocity,
                age: 0.0,
                burn: TRACER_BURN_S[0] + random.next() * (TRACER_BURN_S[1] - TRACER_BURN_S[0]),
            });
        }
    }

    /// The clock the rounds go off by (s): the effects' own, from the scene's start.
    pub fn clock(&self) -> f64 {
        self.time
    }

    /// This frame's pops, for their cracks: each taken once.
    pub fn take_pops(&mut self) -> std::vec::Drain<'_, Pop> {
        self.pops.drain(..)
    }

    /// The rounds in flight as the tracers' drawing takes them (`ShellVisuals`): ids below zero, never a battle's.
    pub fn rounds(&self) -> impl Iterator<Item = ShellState> + '_ {
        self.rounds.iter().map(|round| ShellState {
            id: round.id,
            owner: round.owner,
            position: round.position,
            velocity: round.velocity,
            caliber_m: TRACER_CALIBRE,
            age: round.age,
            airborne: true,
            helm: false,
        })
    }

    /// Into a rebuilt theatre scene (`CombatEffects::take_ship_history_from`): the rounds in flight move with the world.
    pub fn translate(&mut self, offset: DVec3) {
        for round in &mut self.rounds {
            round.position += offset;
        }
    }

    /// Into a scene laid on another chart: the rounds in flight are let go, what the guns have spent kept.
    pub fn relay(&mut self) {
        self.rounds.clear();
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::math::ShipPose;
    use crate::particles::{ParticlePool, PoolKind};
    use crate::ships::{FireState, MountProfile, ShipEffectsProfile};

    fn burning(intensity: f64) -> ShipFrame {
        let profile = Arc::new(ShipEffectsProfile {
            mounts: vec![MountProfile {
                position: DVec3::new(6.0, 9.0, -20.0),
                bearing_deg: 90.0,
                parent: None,
                gunhouse: [3.0, 3.0, 2.0],
            }],
            zone_mounts: vec![vec![0]],
            ..Default::default()
        });
        let mut ship = ShipFrame::new("ship", profile, ShipPose::default());
        ship.zone_fires = vec![FireState {
            intensity,
            ..Default::default()
        }];
        ship.mount_trains = vec![0.0];
        ship
    }

    fn pools() -> (ParticlePool, ParticlePool) {
        (
            ParticlePool::new(1024, PoolKind::Gas),
            ParticlePool::new(1024, PoolKind::Additive),
        )
    }

    /// Pops over `seconds` of 1/60 s frames, and the most rounds in flight at once.
    fn run(ship: &ShipFrame, seconds: f64) -> (Vec<Pop>, usize) {
        let (pops, most, _, _) = run_lit(ship, seconds, &EffectLighting::default());
        (pops, most)
    }

    /// [`run`] under `lighting`, with the lights the pops asked for and the sprites they left.
    fn run_lit(
        ship: &ShipFrame,
        seconds: f64,
        lighting: &EffectLighting,
    ) -> (Vec<Pop>, usize, Vec<FlashRequest>, ParticlePool) {
        let mut cookoffs = CookOffs::default();
        let (mut volumes, mut fire) = pools();
        let mut flashes = Vec::new();
        let (mut pops, mut most) = (Vec::new(), 0);
        for _ in 0..(seconds * 60.0) as usize {
            let mut pools = BlastPools {
                volumes: &mut volumes,
                fire: &mut fire,
                flashes: &mut flashes,
            };
            cookoffs.update(
                1.0 / 60.0,
                DVec3::ZERO,
                std::slice::from_ref(ship),
                &mut pools,
                lighting,
            );
            pops.extend(cookoffs.take_pops());
            most = most.max(cookoffs.rounds().count());
        }
        (pops, most, flashes, fire)
    }

    #[test]
    fn a_gun_in_a_zone_burning_hard_cooks_off_a_few_times_a_minute_with_tracers() {
        let (pops, most) = run(&burning(1.0), 120.0);
        assert!(
            (4..=40).contains(&pops.len()),
            "{} pops in two minutes",
            pops.len()
        );
        assert!(most >= 1);
        // About her gun's lockers round its 3 m gunhouse, above its base: outside the gunhouse, within its reach.
        for pop in &pops {
            let out = (pop.position - DVec3::new(6.0, 9.8, -20.0)) * DVec3::new(1.0, 0.0, 1.0);
            assert!(
                pop.position.y > 9.0 && (1.5..2.8).contains(&out.length()),
                "{out}"
            );
        }
    }

    /// A round's few hundred grams of propellant: by day and after dark its light stays under the bloom's glare and
    /// reaches a few metres; after dark it glows at the fires' flames' lift and lights at their light's, not a gun's.
    #[test]
    fn a_pop_is_a_spark_not_a_guns_flash_by_day_or_night() {
        let mut night = EffectLighting::default();
        night.set_lift(16_700.0, 1.0);
        for lighting in [EffectLighting::default(), night] {
            let (pops, _, flashes, fire) = run_lit(&burning(1.0), 120.0, &lighting);
            assert!(!pops.is_empty() && flashes.len() == pops.len());
            for flash in &flashes {
                // The glare (`naval-client`'s `bloom_glares`) takes the power without the lift, from 3.75.
                assert!(flash.power < 3.75 && flash.distance <= 6.0 && flash.duration <= 0.04);
                assert!(flash.power * lighting.lift <= 3.0 * lighting.firelight * 1.0001);
            }
            let brightest = fire
                .particles()
                .iter()
                .filter(|p| p.age < p.life)
                .map(|p| p.color.r)
                .fold(0.0, f64::max);
            assert!(
                brightest * lighting.lift <= 3.0 * lighting.flames * 1.0001,
                "{brightest}"
            );
        }
    }

    #[test]
    fn nothing_cooks_off_in_a_zone_burning_lightly() {
        assert!(run(&burning(0.3), 120.0).0.is_empty());
    }

    #[test]
    fn the_same_fire_pops_the_same_way_however_the_frames_fall() {
        let ship = burning(1.0);
        let mut cookoffs = CookOffs::default();
        let (mut volumes, mut fire) = pools();
        let mut flashes = Vec::new();
        let mut pops = Vec::new();
        for _ in 0..(60.0 / 0.05) as usize {
            let mut pools = BlastPools {
                volumes: &mut volumes,
                fire: &mut fire,
                flashes: &mut flashes,
            };
            cookoffs.update(
                0.05,
                DVec3::ZERO,
                std::slice::from_ref(&ship),
                &mut pools,
                &EffectLighting::default(),
            );
            pops.extend(cookoffs.take_pops());
        }
        let (steady, _) = run(&ship, 60.0);
        assert_eq!(pops.len(), steady.len());
        for (a, b) in pops.iter().zip(&steady) {
            assert!(a.position.distance(b.position) < 1e-9 && a.size == b.size);
        }
    }
}
