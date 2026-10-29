//! Exhaust from every funnel in the fleet: the web client's `ShipFunnelSmoke` and `SmokeRibbons`
//! (the web game's `ShipFunnelSmoke.ts`, `SmokeRibbons.ts`).
//!
//! Lit near-field billows boil off the rim over one continuous ribbon that rises from the same rim and carries
//! the plume downwind for about half a kilometre underway, so a ship trails a visible streak at range. Billows
//! leave the rim spaced by distance through the air, so a ship underway streams one overlapping column rather
//! than a string of puffs. Colour and density follow the machinery: light haze at rest, heavier working smoke
//! at speed, a sooty burst when working up, black smoke from damaged boilers, nothing once the plant is dead or
//! the rim is under water.
//!
//! Trial `flipbook` ([`FunnelSmoke::plume`]): the billows and the ribbon give way to one chain of links of the
//! simulated plume (`flipbook::clip::PLUME`) laid along the same released samples from the rim (`draw_links`), so
//! the trail's length, its bend astern and its colour are the ribbon's, and nothing hands over from one to the other.
//!
//! The game's look since 2026-10-20 ([`FunnelSmoke::puffs`], `Trial::funnel_puffs`): the billows and the ribbon give
//! way to puffs of the gun smoke's simulated smoke (`flipbook::clip::SMOKE`) laid along the same released samples
//! (`draw_puffs`), drawn and merged with the other gas. The ribbon was a strip turned to face the lens: from above it
//! was a flat band of one opacity with hard edges, and seen end on it twisted into thin wedges. The puffs are round
//! from every side, thin at their rims, widen and thin downwind as the ribbon did, and their gas shades the sea.

use std::collections::{HashMap, VecDeque};

use glam::DVec3;

use crate::camera::CameraView;
use crate::flipbook::{ClipSizes, clip};
use crate::instances::{GAS_SMOKE, GasPuff, RibbonVertex, Sprite};
use crate::math::{Rgb, clamp, smoothstep};
use crate::particles::{ParticlePool, PoolKind, SourceId};
use crate::random::Lcg;
use crate::ships::{FunnelOutlet, ShipFrame};

const SPRITES: usize = 6144;
/// Plume samples per funnel, one every `TRAIL_INTERVAL` seconds, for at most `TRAIL_LIFE_S` of drifting trail.
const TRAIL_INTERVAL: f64 = 0.5;
const TRAIL_LIFE_S: f64 = 60.0;
const TRAIL_POINTS: usize = (TRAIL_LIFE_S / TRAIL_INTERVAL) as usize;
/// Underway, the trail ends about this far from where its smoke was released, measured through the air.
const TRAIL_LENGTH_M: f64 = 500.0;
const TRAIL_MIN_S: f64 = 15.0;
/// The most simulated time one update steps: a stalled frame's. A theatre's battle runs its smoke at its running
/// speed, up to 256×, about 4 s a frame at 60 frames a second; a lower cap ages the smoke slower than the hulls move
/// and the trail runs that many times its length astern.
const MAX_STEP_S: f64 = 5.0;
/// Beyond this camera distance near-field billows thin out and the ribbon carries the plume.
const SPRITE_LOD_M: f64 = 1400.0;
/// Billows leave the rim at least this fraction of the mouth's width apart through the air.
const BILLOW_SPACING: f64 = 0.4;
const MAX_BILLOW_RATE: f64 = 30.0;
// Exhaust leaves the rim fast and hot, loses its momentum within a couple of seconds and keeps only a slow
// buoyant rise, so the column bends over downwind.
const DRAG: f64 = 0.45;
const BUOYANCY: f64 = 0.12;
const INHERIT: f64 = 0.3;
/// Ribbon vertices across the fleet (`SmokeRibbons` capacity).
const RIBBON_CAPACITY: usize = 32768;

fn cruise() -> Rgb {
    Rgb::hex(0xbdbdba)
}
fn working() -> Rgb {
    Rgb::hex(0x8f8f8d)
}
fn sooty() -> Rgb {
    Rgb::hex(0x2b2a29)
}

/// A released plume sample: the ribbon's spine. It moves with the air once it leaves the rim.
#[derive(Clone, Copy, Debug, Default)]
struct TrailPoint {
    position: DVec3,
    velocity: DVec3,
    color: Rgb,
    width: f64,
    width0: f64,
    opacity: f64,
    opacity0: f64,
    along: f64,
    age: f64,
    spent: f64,
    life: f64,
    /// Its number among the funnel's released samples (trial `flipbook`: what its links are known by).
    seq: u64,
}

#[derive(Clone, Debug)]
struct Emitter {
    outlet: FunnelOutlet,
    previous: DVec3,
    credit: f64,
    initialized: bool,
    /// Released samples, oldest first.
    trail: VecDeque<TrailPoint>,
    trail_credit: f64,
    along: f64,
    /// Samples released so far.
    released: u64,
    /// Mixed into the numbers its puffs' places draw (the hull's and the funnel's ids), so no two funnels' puffs
    /// stand alike.
    salt: u64,
    /// Whether the funnel smoked at the last update (its rim is `previous`).
    smoking: bool,
}

#[derive(Clone, Debug)]
struct ShipState {
    funnels: Vec<Emitter>,
    speed: f64,
    soot: f64,
}

/// A funnel's plume as its ribbon follows it ([`FunnelSmoke::spines`]): the rim where it stood at the last update
/// and the released samples, oldest first, each with its age (s) and current width (m). For tools that aim a camera
/// at a plume, which bends with the ship's way and the wind.
#[derive(Clone, Debug)]
pub struct FunnelSpine {
    pub ship: String,
    pub funnel: String,
    pub rim: DVec3,
    pub samples: Vec<SpineSample>,
}

/// A released plume sample (see [`FunnelSpine`]).
#[derive(Clone, Copy, Debug)]
pub struct SpineSample {
    pub position: DVec3,
    pub age: f64,
    pub width: f64,
}

/// What the funnels publish for one frame.
#[derive(Clone, Debug, Default)]
pub struct FunnelPublish {
    pub billows: Vec<Sprite>,
    pub ribbon_vertices: Vec<RibbonVertex>,
    pub ribbon_indices: Vec<u32>,
}

pub struct FunnelSmoke {
    pool: ParticlePool,
    emitters: HashMap<SourceId, ShipState>,
    random: Lcg,
    /// Emission rate multiplier (the combat effects setting).
    pub density: f64,
    ribbons: FunnelPublishRibbons,
    /// A trail's samples as drawn, working room kept between frames.
    ordered: Vec<TrailPoint>,
    /// Trial `flipbook`: the exhaust drawn as a chain of plume links (`draw_links`) in place of billows and ribbon.
    pub(crate) plume: bool,
    /// Seconds of smoke stepped (the links' clock), and this update's links with their squared distance.
    clock: f64,
    links: Vec<(f64, GasPuff)>,
    /// The scene's 0–1 brightness (`EffectLighting::daylight`), for the links' paleness.
    pub(crate) daylight: f64,
    /// The game's look: the exhaust drawn as puffs of simulated smoke (`draw_puffs`) in place of billows and ribbon.
    /// `plume` wins over it.
    pub(crate) puffs: bool,
    /// The smoke clip's frames' sizes over a puff's life (`ClipSizes::frame`, [`PUFF_SIZES`] + 1 of them): a puff is
    /// drawn as large as its frame's gas, which grows through the clip.
    smoke_sizes: [f32; PUFF_SIZES + 1],
    /// This update's puffs with their squared distance.
    puff_list: Vec<(f64, GasPuff)>,
    /// Working room for the puffs' drawing order: each one's distance, tie-break and place in `puff_list` in one key
    /// (`order_key`).
    puff_order: Vec<u128>,
}

#[derive(Clone, Debug, Default)]
struct FunnelPublishRibbons {
    vertices: Vec<RibbonVertex>,
    indices: Vec<u32>,
}

impl Default for FunnelSmoke {
    fn default() -> Self {
        Self::new()
    }
}

impl FunnelSmoke {
    pub fn new() -> Self {
        Self {
            pool: ParticlePool::new(SPRITES, PoolKind::Sprites),
            emitters: HashMap::new(),
            random: Lcg::new(1),
            density: 1.0,
            ribbons: FunnelPublishRibbons::default(),
            ordered: Vec::new(),
            plume: false,
            clock: 0.0,
            links: Vec::new(),
            daylight: 1.0,
            puffs: false,
            smoke_sizes: [1.0; PUFF_SIZES + 1],
            puff_list: Vec::new(),
            puff_order: Vec::new(),
        }
    }

    /// The smoke clip's frame sizes, from the flipbook's table (`ClipSizes::frame`): its gas grows through the clip.
    pub(crate) fn set_smoke_sizes(&mut self, sizes: &ClipSizes) {
        for (i, size) in self.smoke_sizes.iter_mut().enumerate() {
            *size = sizes.frame(clip::SMOKE, i as f64 / PUFF_SIZES as f64).0 as f32;
        }
    }

    fn next(&mut self) -> f64 {
        self.random.next()
    }

    /// Step every funnel and rebuild the plume ribbons for `camera`.
    pub fn update(
        &mut self,
        ships: &[ShipFrame],
        dt: f64,
        camera: &CameraView,
        wind: DVec3,
        hidden: Option<&str>,
    ) {
        // A departing hull stops emitting; the other hulls and already released smoke sail on.
        self.emitters
            .retain(|id, _| ships.iter().any(|s| &s.id == id));
        let step = dt.clamp(0.0, MAX_STEP_S);
        self.pool.advance(step, wind);
        self.ribbons.vertices.clear();
        self.ribbons.indices.clear();
        self.links.clear();
        self.clock += step;
        // Links first, then puffs, then billows and ribbon.
        let puffs = self.puffs && !self.plume;
        let planes = camera.frustum();
        for ship in ships {
            let profile = &ship.profile;
            let state = self
                .emitters
                .entry(ship.id.clone())
                .or_insert_with(|| ShipState {
                    speed: ship.speed,
                    soot: 0.0,
                    funnels: profile
                        .funnels
                        .iter()
                        .map(|outlet| Emitter {
                            outlet: outlet.clone(),
                            previous: DVec3::ZERO,
                            credit: 0.0,
                            initialized: false,
                            trail: VecDeque::with_capacity(TRAIL_POINTS),
                            trail_credit: 0.0,
                            along: 0.0,
                            released: 0,
                            salt: fnv(&[ship.id.as_bytes(), outlet.id.as_bytes()]),
                            smoking: false,
                        })
                        .collect(),
                });
            if state.funnels.is_empty() {
                continue;
            }
            let power = if ship.sunk { 0.0 } else { ship.engine_power };
            let (forward, reverse) = (profile.forward_speed, profile.reverse_speed);
            let load =
                (ship.speed.abs() / if ship.speed < 0.0 { reverse } else { forward }).min(1.0);
            // Working up to speed makes smoke; so do damaged boilers, however slow the ship.
            if step > 0.0 {
                let target = (((ship.speed.abs() - state.speed.abs()) / step).max(0.0)
                    / (forward * 0.015))
                    .min(1.0);
                let tau = if target > state.soot { 0.8 } else { 5.0 };
                state.soot += (target - state.soot) * (1.0 - (-step / tau).exp());
                state.speed = ship.speed;
            }
            let soot = (state.soot * 0.9).max(1.0 - smoothstep(power, 0.35, 0.92));
            let albedo = cruise().lerp(working(), load).lerp(sooty(), soot);
            let thickness = (0.5 + load * 0.22 + soot * 0.28) * power.max(0.0).sqrt();
            let velocity = ship.velocity;
            // How fast the ship leaves its released smoke behind: billows keep some of its way, the trail none.
            let shed = (velocity.x * (1.0 - INHERIT) - wind.x)
                .hypot(velocity.z * (1.0 - INHERIT) - wind.z);
            let airspeed = (velocity.x - wind.x).hypot(velocity.z - wind.z);
            let hidden_ship = hidden == Some(&*ship.id);
            let mut funnels =
                std::mem::take(&mut self.emitters.get_mut(&ship.id).expect("inserted").funnels);
            for emitter in &mut funnels {
                let outlet = &emitter.outlet;
                let origin = ship.place(outlet.position);
                let travel = velocity.length() * step + 100.0;
                if !emitter.initialized
                    || emitter.previous.distance_squared(origin) > travel * travel
                {
                    emitter.previous = origin;
                    emitter.credit = 0.0;
                    emitter.trail_credit = TRAIL_INTERVAL;
                    emitter.trail.clear();
                    emitter.initialized = true;
                }
                let alive = power > 0.01 && origin.y > 0.3;
                let lod = clamp(
                    SPRITE_LOD_M / (camera.position - origin).length().max(1.0),
                    0.12,
                    1.0,
                );
                let mouth = (outlet.width * 0.9).max(1.5);
                if alive && step > 0.0 && !self.plume && !puffs {
                    // Streamed out underway, the same smoke is spread over more billows, each proportionally thinner.
                    let base = 3.8 + load * 4.5 + soot * 5.0;
                    let spaced = shed / (mouth * BILLOW_SPACING);
                    let spaced = base.max(spaced).min(MAX_BILLOW_RATE);
                    let share = base / spaced;
                    let rate = spaced * power.sqrt() * self.density * lod;
                    let interval = 1.0 / rate;
                    emitter.credit += step;
                    while emitter.credit + 1e-9 >= interval {
                        emitter.credit = (emitter.credit - interval).max(0.0);
                        let mut position =
                            emitter.previous.lerp(origin, 1.0 - emitter.credit / step);
                        // Spread within the uptake, including a carrier's long funnel mouth.
                        let across = (self.next() - 0.5) * outlet.width * 0.4;
                        let along = (self.next() - 0.5) * outlet.length * 0.5;
                        let heading = ship.pose.heading + outlet.bearing_rad.unwrap_or(0.0);
                        let (sin, cos) = heading.sin_cos();
                        position.x += across * cos - along * sin;
                        position.z += across * sin + along * cos;
                        let size = mouth * (0.6 + self.next() * 0.7) / lod.sqrt();
                        let stretch = 0.85 + self.next() * 0.35;
                        let life = 5.5 + load * 1.5 + self.next() * 2.0;
                        let vx = velocity.x * INHERIT + (self.next() - 0.5) * 1.4;
                        let vy = 4.0 + load * 3.0 + soot * 2.0 + self.next() * 1.5;
                        let vz = velocity.z * INHERIT + (self.next() - 0.5) * 1.4;
                        let opacity =
                            1.0 - (1.0 - thickness * (0.62 + self.next() * 0.25)).powf(share);
                        let color = albedo.scale(0.9 + self.next() * 0.18);
                        let angle = self.next() * std::f64::consts::TAU;
                        let spin = (self.next() - 0.5) * 0.22;
                        let p = self.pool.emit(position, Some(&ship.id));
                        // Distant funnels shed fewer, larger billows; the ribbon carries the plume there.
                        p.size = size;
                        p.stretch = stretch;
                        p.growth = mouth * (1.1 + load * 0.5 + soot * 0.6);
                        p.growth_decay = 0.32;
                        p.diffusion = 0.45 + soot * 0.2;
                        p.life = life;
                        p.velocity = DVec3::new(vx, vy, vz);
                        p.drag = DRAG;
                        p.gravity = -BUOYANCY;
                        p.wind = 1.0;
                        p.opacity = opacity;
                        p.fade_in = 0.3;
                        p.color = color;
                        p.angle = angle;
                        p.spin = spin;
                    }
                } else if !alive {
                    emitter.credit = 0.0;
                }
                advance_trail(emitter, step, wind);
                if alive && step > 0.0 {
                    emitter.trail_credit += step;
                    while emitter.trail_credit + 1e-9 >= TRAIL_INTERVAL {
                        emitter.trail_credit -= TRAIL_INTERVAL;
                        let age = emitter.trail_credit.max(0.0);
                        let position = emitter.previous.lerp(origin, 1.0 - age / step);
                        // The released smoke's own spacing, so turbulence keeps its world scale.
                        if let Some(newest) = emitter.trail.back() {
                            emitter.along += (newest.position - position).length().max(0.3);
                        }
                        if emitter.trail.len() == TRAIL_POINTS {
                            emitter.trail.pop_front();
                        }
                        let mut released = TrailPoint {
                            position,
                            velocity: DVec3::new(
                                velocity.x * INHERIT,
                                4.0 + load * 3.0 + soot * 2.0,
                                velocity.z * INHERIT,
                            ),
                            width0: mouth,
                            opacity0: thickness,
                            color: albedo,
                            along: emitter.along,
                            seq: emitter.released,
                            life: clamp(
                                TRAIL_LENGTH_M / airspeed.max(1.0),
                                TRAIL_MIN_S,
                                TRAIL_LIFE_S,
                            ),
                            ..TrailPoint::default()
                        };
                        integrate(&mut released, age, wind);
                        emitter.trail.push_back(released);
                        emitter.released += 1;
                    }
                } else if !alive {
                    emitter.trail_credit = TRAIL_INTERVAL;
                }
                if !hidden_ship && self.plume {
                    // The six baked lights find less of the sun than the ribbon's tube does: by day the exhaust is
                    // paler to match, after dark (where the sky's light is most of it) as it is.
                    let pale = 1.0 + (LINK_PALE - 1.0) * smoothstep(self.daylight, 0.3, 0.9);
                    draw_links(
                        pale,
                        emitter,
                        alive.then_some(origin),
                        mouth,
                        self.clock,
                        camera,
                        &planes,
                        &mut self.links,
                    );
                } else if puffs {
                    // Drawn for each view as it publishes (`publish_puffs_into`): a second view sees what the first
                    // does not.
                } else if !hidden_ship {
                    draw_trail(
                        emitter,
                        alive.then_some(origin),
                        mouth,
                        camera,
                        &planes,
                        &mut self.ribbons,
                        &mut self.ordered,
                    );
                }
                emitter.previous = origin;
                emitter.smoking = alive;
            }
            self.emitters.get_mut(&ship.id).expect("inserted").funnels = funnels;
        }
    }

    /// Trial `flipbook`: the chains' links back to front, written over `out`.
    pub fn publish_links_into(&mut self, out: &mut Vec<GasPuff>) {
        self.links.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.clear();
        out.extend(self.links.iter().map(|(_, link)| *link));
    }

    /// The game's look: every plume's puffs for `camera`, back to front, written over `out` (none with the look off).
    /// `hidden`'s funnels are left out, as optics on that hull leave out her smoke.
    pub fn publish_puffs_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        out: &mut Vec<GasPuff>,
    ) {
        out.clear();
        self.puff_list.clear();
        if !self.puffs || self.plume {
            return;
        }
        let planes = camera.frustum();
        let look = PuffLook {
            apart: PUFF_APART / self.density.clamp(0.25, 1.0).sqrt(),
            sizes: &self.smoke_sizes,
        };
        for (ship, state) in &self.emitters {
            if hidden == Some(&**ship) {
                continue;
            }
            for emitter in state.funnels.iter().filter(|e| e.initialized) {
                let mouth = (emitter.outlet.width * 0.9).max(1.5);
                let rim = emitter.smoking.then_some(emitter.previous);
                if plume_unseen(&look, emitter, rim, mouth, camera, &planes) {
                    continue;
                }
                draw_puffs(
                    &look,
                    emitter,
                    rim,
                    mouth,
                    camera,
                    &planes,
                    &mut self.ordered,
                    &mut self.puff_list,
                );
            }
        }
        // Back to front; puffs equally far keep a fixed order whatever the map's. The keys are sorted, not the puffs
        // (seven times their size), their place last as the stable sort kept it.
        self.puff_order.clear();
        self.puff_order.extend(
            self.puff_list
                .iter()
                .enumerate()
                .map(|(i, (distance, puff))| order_key(*distance, puff.state[1], i as u32)),
        );
        self.puff_order.sort_unstable();
        out.extend(
            self.puff_order
                .iter()
                .map(|&key| self.puff_list[key as u32 as usize].1),
        );
    }

    pub fn publish(&mut self, camera: &CameraView, hidden: Option<&str>) -> FunnelPublish {
        let mut publish = FunnelPublish::default();
        self.publish_into(
            camera,
            hidden,
            &mut publish.billows,
            &mut publish.ribbon_vertices,
            &mut publish.ribbon_indices,
        );
        publish
    }

    /// [`FunnelSmoke::publish`] written over the caller's lists, which keep their room.
    pub fn publish_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        billows: &mut Vec<Sprite>,
        ribbon_vertices: &mut Vec<RibbonVertex>,
        ribbon_indices: &mut Vec<u32>,
    ) {
        billows.clear();
        self.pool.publish_sprites_into(camera, hidden, billows);
        ribbon_vertices.clone_from(&self.ribbons.vertices);
        ribbon_indices.clone_from(&self.ribbons.indices);
    }

    /// Billows alive (or the puffs the last publish drew, in the game's look) and ribbon segments built by the last
    /// update.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.pool.alive() + self.puff_list.len(),
            self.ribbons.indices.len() / 6,
        )
    }

    /// Every funnel's plume spine, by ship id and funnel order.
    pub fn spines(&self) -> Vec<FunnelSpine> {
        let mut ships: Vec<_> = self.emitters.iter().collect();
        ships.sort_by(|a, b| a.0.cmp(b.0));
        ships
            .into_iter()
            .flat_map(|(ship, state)| {
                state
                    .funnels
                    .iter()
                    .filter(|e| e.initialized)
                    .map(|emitter| FunnelSpine {
                        ship: ship.to_string(),
                        funnel: emitter.outlet.id.clone(),
                        rim: emitter.previous,
                        samples: emitter
                            .trail
                            .iter()
                            .map(|p| SpineSample {
                                position: p.position,
                                age: p.age,
                                width: spread(p),
                            })
                            .collect(),
                    })
            })
            .collect()
    }

    /// Each funnel's mouth in world space, for diagnostics (the web's `diagnostics().outlets`).
    pub fn outlets(ships: &[ShipFrame]) -> Vec<(String, String, DVec3)> {
        ships
            .iter()
            .flat_map(|ship| {
                ship.profile.funnels.iter().map(|outlet| {
                    (
                        ship.id.to_string(),
                        outlet.id.clone(),
                        ship.place(outlet.position),
                    )
                })
            })
            .collect()
    }

    /// Trial (shadows): the billows and the plumes' released smoke as the shadow map's casters (`shadow.rs`): a ball
    /// at every quarter-width of trail or more, each a share of what the ribbon's section stops, so the balls
    /// together stop what the section does.
    pub(crate) fn shadow_blobs(&self, hidden: Option<&str>, out: &mut Vec<crate::shadow::Blob>) {
        crate::shadow::billow_blobs(&self.pool, hidden, out);
        // The game's look draws the plume as wide as its puffs spread, and as dark (`draw_puffs`).
        let puffs = self.puffs && !self.plume;
        let width_of = if puffs { puff_spread } else { spread };
        for (ship, state) in &self.emitters {
            if hidden == Some(&**ship) {
                continue;
            }
            for emitter in &state.funnels {
                let mut kept: Option<DVec3> = None;
                for p in &emitter.trail {
                    let width = width_of(p);
                    let radius = width * 0.5;
                    let spacing = kept.map_or(radius, |kept| p.position.distance(kept));
                    if kept.is_some() && spacing < radius * 0.5 {
                        continue;
                    }
                    kept = Some(p.position);
                    // The ribbon's opacity through its middle (`draw_trail`, `ribbon_fragment`), or the puffs'.
                    let spent = (p.age / p.life).min(1.0);
                    let section = if puffs {
                        1.0 - (-puff_optical(p, width)).exp()
                    } else {
                        p.opacity0
                            * (p.width0 / width).powf(0.35)
                            * (1.0 - smoothstep(spent, 0.45, 1.0))
                            * smoothstep(p.age, 0.0, 0.6)
                            * 0.9
                    }
                    .min(0.97);
                    let share = (spacing / radius * 0.75).min(1.0);
                    out.extend(crate::shadow::Blob::new(
                        p.position,
                        radius,
                        1.0 - (1.0 - section).powf(share),
                    ));
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.pool.reset();
        self.emitters.clear();
        self.ribbons.vertices.clear();
        self.ribbons.indices.clear();
        self.random = Lcg::new(1);
        self.links.clear();
        self.puff_list.clear();
        self.clock = 0.0;
    }

    pub(crate) fn translate(&mut self, offset: DVec3) {
        self.pool.translate(offset);
        for state in self.emitters.values_mut() {
            for emitter in &mut state.funnels {
                emitter.previous += offset;
                for point in &mut emitter.trail {
                    point.position += offset;
                }
            }
        }
    }

    /// Carries the funnels into a scene laid on another chart (a theatre's re-lay): the smoke as its pool carries it, an
    /// outlet's last place and each released sample's through their places, a sample's velocity through the two
    /// charts' Jacobians where it is.
    pub(crate) fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        self.pool.relay(relay);
        for state in self.emitters.values_mut() {
            for emitter in &mut state.funnels {
                emitter.previous = DVec3::from_array(relay.point(emitter.previous.to_array()));
                for point in &mut emitter.trail {
                    let at = point.position.to_array();
                    point.velocity = DVec3::from_array(relay.vector(at, point.velocity.to_array()));
                    point.position = DVec3::from_array(relay.point(at));
                }
            }
        }
    }

    /// Carries the funnels' smoke, billows and released trail, with the blast waves that passed it this step,
    /// never under the sea.
    pub(crate) fn shove(&mut self, shoves: &[crate::shock::Shove]) {
        self.pool.shove(shoves);
        for state in self.emitters.values_mut() {
            for emitter in &mut state.funnels {
                for point in &mut emitter.trail {
                    for shove in shoves {
                        if point.age > shove.age {
                            let floor = point.position.y.min(0.5);
                            point.position += shove.at(point.position);
                            point.position.y = point.position.y.max(floor);
                        }
                    }
                }
            }
        }
    }
}

/// A released sample's width (m) at its age: turbulent spreading, fast at first, then steady diffusion that keeps
/// widening downwind.
fn spread(p: &TrailPoint) -> f64 {
    p.width0 * (1.0 + 1.9 * (1.0 - (-p.age / 5.0).exp())) + 0.8 * p.age
}

/// The game's look: a released sample's width (m) as its puffs draw it. The hot exhaust boils out of the rim and takes
/// in air fastest in its first seconds, as the billows it replaces grew (to about three and a half mouths across in five
/// seconds), then keeps widening downwind half as fast again as the ribbon did: some 45 m across ten seconds out, 70 m
/// half a minute out, for an 8 m mouth.
fn puff_spread(p: &TrailPoint) -> f64 {
    p.width0 * (1.0 + 3.0 * (1.0 - (-p.age / 3.0).exp())) + 1.2 * p.age
}

/// The game's look: the optical depth through the middle of a released sample's plume `width` metres wide. It left the
/// rim stopping `opacity0` of the light; its soot then spreads over the plume's section as it widens, so the path
/// through its middle stops as much less as the plume is wider, and the last of it clears over the end of its life
/// (the ribbon's opacity fell only as the width's 0.35th power: from above, a band of one opacity).
fn puff_optical(p: &TrailPoint, width: f64) -> f64 {
    let spent = (p.age / p.life).min(1.0);
    -(1.0 - p.opacity0.min(0.97)).ln()
        * (p.width0 / width.max(p.width0))
        * (1.0 - smoothstep(spent, 0.45, 1.0))
}

fn advance_trail(emitter: &mut Emitter, step: f64, wind: DVec3) {
    if step <= 0.0 {
        return;
    }
    for point in &mut emitter.trail {
        integrate(point, step, wind);
    }
    while emitter.trail.front().is_some_and(|p| p.age >= p.life) {
        emitter.trail.pop_front();
    }
}

/// Drag toward the moving air (the wind at its height) plus a slow buoyant rise, integrated exactly (frame-rate
/// independent).
fn integrate(p: &mut TrailPoint, step: f64, wind: DVec3) {
    if step <= 0.0 {
        return;
    }
    let decay = (-DRAG * step).exp();
    let travel = (1.0 - decay) / DRAG;
    let terminal = -BUOYANCY / DRAG;
    let wind = wind * crate::combat::wind_share(p.position.y);
    p.position.x += p.velocity.x * travel + wind.x * step;
    p.position.z += p.velocity.z * travel + wind.z * step;
    p.position.y += (p.velocity.y + terminal) * travel - terminal * step;
    p.velocity.x *= decay;
    p.velocity.z *= decay;
    p.velocity.y = (p.velocity.y + terminal) * decay - terminal;
    p.age += step;
}

fn draw_trail(
    emitter: &mut Emitter,
    mouth: Option<DVec3>,
    width: f64,
    camera: &CameraView,
    planes: &[crate::camera::Plane; 6],
    ribbons: &mut FunnelPublishRibbons,
    ordered: &mut Vec<TrailPoint>,
) {
    let size = emitter.trail.len();
    if size == 0 {
        return;
    }
    // Cheap early cull from every eighth sample: offscreen plumes keep drifting but skip all strip work.
    let mut min = DVec3::splat(f64::INFINITY);
    let mut max = DVec3::splat(f64::NEG_INFINITY);
    let mut expand = |p: DVec3| {
        min = min.min(p);
        max = max.max(p);
    };
    if let Some(mouth) = mouth {
        expand(mouth);
    }
    for i in (0..size).step_by(8) {
        expand(emitter.trail[i].position);
    }
    expand(emitter.trail[size - 1].position);
    let oldest = emitter.trail[0];
    let center = (min + max) * 0.5;
    // The widest (oldest) section plus how far eight samples can bow out between those taken.
    let radius = (max - min).length() * 0.5 + oldest.width0 * 3.0 + 0.65 * oldest.age + 60.0;
    let drawn = camera.drawn(center);
    if planes.iter().any(|plane| plane.distance(drawn) < -radius) {
        return;
    }
    ordered.clear();
    ordered.reserve(size + 1);
    for i in 0..size {
        let p = &mut emitter.trail[i];
        let t = p.age;
        p.width = spread(p);
        // Keep samples a quarter-width apart, and about a degree apart as seen from the camera.
        if let Some(kept) = ordered.last()
            && i < size - 1
        {
            let spacing = (p.width.min(kept.width) * 0.25)
                .max((p.position - camera.position).length() * 0.015);
            if p.position.distance_squared(kept.position) < spacing * spacing {
                continue;
            }
        }
        // The ribbon material fades the column in from the rim by age, under the billows.
        p.spent = (t / p.life).min(1.0);
        p.opacity =
            p.opacity0 * (p.width0 / p.width).powf(0.35) * (1.0 - smoothstep(p.spent, 0.45, 1.0));
        ordered.push(*p);
    }
    // A trail younger than its life has an unfaded first sample: taper the oldest end.
    let mut run = 0.0;
    for i in 0..ordered.len() {
        if i > 0 {
            run += (ordered[i].position - ordered[i - 1].position).length();
        }
        let taper = smoothstep(run, 0.0, (ordered[i].width * 1.5).max(40.0));
        ordered[i].opacity *= taper;
    }
    if let (Some(mouth), Some(newest)) = (mouth, ordered.last().copied()) {
        ordered.push(TrailPoint {
            position: mouth,
            width,
            opacity: newest.opacity,
            color: newest.color,
            along: newest.along + (newest.position - mouth).length().max(0.3),
            age: 0.0,
            spent: 0.0,
            ..TrailPoint::default()
        });
    }
    strip(ordered, camera, ribbons);
}

/// Trial `flipbook`: links a sample's stretch of trail is cut into, so the chain is close enough by the rim, where
/// samples leave half a second apart and the plume is narrow.
const LINK_CUTS: u64 = 8;
/// Links wanted along the trail, in link radii between them (a link's smoke thins to nothing at both ends).
const LINK_APART: f64 = 1.6;
/// A link's smoke against the plume's width there (the ribbon's dense core is about this share of it).
const LINK_WIDTH: f64 = 0.45;
/// A link's albedo against the ribbon's colour, by day.
const LINK_PALE: f64 = 1.6;
/// How dense a link's smoke is at the ribbon's opacity of 1.
const LINK_DENSITY: f64 = 2.2;

/// Trial `flipbook`: the trail as a chain of links of the simulated plume. Every stretch between two released samples
/// (and the one between the newest and the rim) is cut into [`LINK_CUTS`] places carried by the air with them; a
/// place is known by its sample's number and its cut, and is drawn as a link only while the chain needs it: places
/// whose number ends in more zero bits stand in for their neighbours as the plume widens, so the links stand between
/// one and two [`LINK_APART`] radii apart all the way along and each fades in or out as it is needed, never popping.
/// A link's opacity and colour are the ribbon's; its smoke rolls up the link away from the rim.
#[allow(clippy::too_many_arguments)]
fn draw_links(
    pale: f64,
    emitter: &Emitter,
    mouth: Option<DVec3>,
    width: f64,
    clock: f64,
    camera: &CameraView,
    planes: &[crate::camera::Plane; 6],
    links: &mut Vec<(f64, GasPuff)>,
) {
    let size = emitter.trail.len();
    if size == 0 {
        return;
    }
    // The samples, oldest first, and the rim as one more just released.
    let mut points: Vec<TrailPoint> = emitter.trail.iter().copied().collect();
    for p in &mut points {
        p.width = spread(p);
        p.spent = (p.age / p.life).min(1.0);
        p.opacity =
            p.opacity0 * (p.width0 / p.width).powf(0.35) * (1.0 - smoothstep(p.spent, 0.45, 1.0));
    }
    if let (Some(mouth), Some(newest)) = (mouth, points.last().copied()) {
        points.push(TrailPoint {
            position: mouth,
            width,
            opacity: newest.opacity0,
            seq: newest.seq + 1,
            ..newest
        });
        points.last_mut().expect("pushed").age = 0.0;
    }
    if points.len() < 2 {
        return;
    }
    // Metres of trail behind each sample, from the oldest.
    let mut run = vec![0.0; points.len()];
    for i in 1..points.len() {
        run[i] = run[i - 1] + (points[i].position - points[i - 1].position).length();
    }
    let total = run[points.len() - 1];
    let far = smoothstep(
        (points[points.len() - 1].position - camera.position).length(),
        1500.0,
        6000.0,
    );
    for i in 0..points.len() - 1 {
        let (old, new) = (&points[i], &points[i + 1]);
        let toward = old.position - new.position;
        let stretch = toward.length();
        let axis = if stretch > 1e-3 {
            toward / stretch
        } else {
            DVec3::Y
        };
        let cut = stretch / LINK_CUTS as f64;
        for j in 0..LINK_CUTS {
            // From the newer end: the cut at 0 is the newer sample's own place.
            let share = j as f64 / LINK_CUTS as f64;
            let id = old.seq * LINK_CUTS
                + (LINK_CUTS - j) % LINK_CUTS
                + if j == 0 { LINK_CUTS } else { 0 };
            let w = new.width + (old.width - new.width) * share;
            let extent = w * LINK_WIDTH * (1.0 + 1.5 * far);
            let half = extent * crate::fire_batch::LINK_REACH;
            // Ranks a place needs to be drawn: the chain at rank r stands cut × 2^r apart.
            let need = (LINK_APART * extent / cut.max(1e-3)).log2() - 1.0;
            let rank = f64::from(id.trailing_zeros().min(20));
            let weight = smoothstep(rank - need, -1.0, 0.0);
            if weight <= 0.01 {
                continue;
            }
            let mut at = new.position + toward * share;
            // Metres of trail behind the place (to the oldest sample) and ahead of it (to the rim).
            let behind = run[i + 1] - stretch * share;
            let ahead = total - behind;
            // By the rim a link stands out of the funnel: its near end, which thins to nothing, at the mouth.
            if mouth.is_some() {
                at += axis * (half * 0.55 - ahead).max(0.0);
            }
            let opacity = (new.opacity + (old.opacity - new.opacity) * share)
                // The oldest end tapers, as the ribbon's does.
                * smoothstep(behind, 0.0, (w * 1.5).max(40.0))
                * weight;
            if opacity < 0.01 {
                continue;
            }
            let drawn = camera.drawn(at);
            if planes
                .iter()
                .any(|plane| plane.distance(drawn) < -half * 1.3)
            {
                continue;
            }
            let seed = crate::random::golden_seed(id);
            let age = new.age + (old.age - new.age) * share;
            let spent = new.spent + (old.spent - new.spent) * share;
            let pace = 0.8 + 0.4 * (seed * 0.731).fract();
            let color = new.color.scale(pale);
            links.push((
                (at - camera.position).length_squared(),
                GasPuff {
                    sphere: [at.x as f32, at.y as f32, at.z as f32, half as f32],
                    state: [
                        age as f32,
                        seed as f32,
                        0.0,
                        (LINK_DENSITY * opacity) as f32,
                    ],
                    tint: [color.r as f32, color.g as f32, color.b as f32, 1.0],
                    evolution: [
                        (seed * 0.0731 + clock * pace / (5.0 + 0.15 * w)).fract() as f32,
                        (0.12 * smoothstep(spent, 0.5, 1.0)) as f32,
                        0.0,
                        0.0,
                    ],
                    quad: [axis.x as f32, axis.y as f32, axis.z as f32, 0.0],
                    extra: [1.0, 0.0, clip::PLUME as f32, 0.0],
                    // The warp trial does not roll a link of the plume.
                    warp: [0.0; 4],
                },
            ));
        }
    }
}

/// The game's look: a puff's radius (the gas it draws) against the plume's width there. The ribbon's width is its whole
/// span, out to where its profile has thinned to a tenth (`ribbon_fragment`); a puff's gas reaches this share of it
/// each way, and its neighbours, a little off the spine to either side, fill out the rest.
const PUFF_WIDTH: f64 = 0.45;
/// Puffs wanted along the trail, in puff radii between them: the places drawn stand between a half and a whole of
/// this apart (`draw_puffs`), so each point of the plume lies in two or three.
const PUFF_APART: f64 = 1.1;
/// The puffs a point of the plume lies in, over [`PUFF_APART`]: what the ribbon's middle stopped is shared among them.
const PUFF_OVERLAP: f64 = 2.5;
/// How far a puff stands off the spine at most, across the plume and up, in plume widths: its billows bulge out of its
/// line by turns, and its rim is ragged, a few puffs' thin edges.
const PUFF_ACROSS: f64 = 0.22;
const PUFF_UP: f64 = 0.12;
/// Steps of the smoke clip's frame sizes kept over a puff's life.
const PUFF_SIZES: usize = 32;
/// The gas shader's optical depth at a body's 1 and a density of 3 (`GAS_DEPTH` in `gas_sprite.wgsl`).
const GAS_DEPTH: f64 = 12.0;
/// The share of a flipbook frame's half-width its gas fills as a ball (`shadow.rs`' `FLIPBOOK_GAS`).
const PUFF_GAS: f64 = 0.8;

/// The mean of a smoke-clip frame's optical depth over its gas, in `GAS_DEPTH`s (`body.b`, stored as its root), at
/// `life` (0–1) of a puff playing the clip: measured over the clip's frames of `fireball.flipbook` (0.28 at its first
/// frame to 0.13 at its last) and `fireball-side.flipbook` (0.17 to 0.19), the two sides a puff shows, on 2026-10-20.
/// Its smoke thins as the clip plays (as the simulation's cloud spread); the puffs' density makes up for it, so the
/// plume thins downwind as the ribbon did and not twice over.
fn smoke_depth(life: f64) -> f64 {
    // The clip's place at `life` (its curve, `flipbook.rs`' `CLIPS`: 1 − e^(−t) over the clip).
    let played = (1.0 - (-life.clamp(0.0, 1.0)).exp()) / (1.0 - (-1.0f64).exp());
    0.23 - 0.07 * played
}

/// How the puffs are drawn this update.
struct PuffLook<'a> {
    /// Puff radii between neighbours: [`PUFF_APART`], more at a lower effects density.
    apart: f64,
    /// The smoke clip's frame sizes over a puff's life.
    sizes: &'a [f32; PUFF_SIZES + 1],
}

impl PuffLook<'_> {
    /// The smoke clip's frame size at `life` (0–1 of the puff's life): its gas as a share of its last frame's.
    fn frame_size(&self, life: f64) -> f64 {
        let at = life.clamp(0.0, 1.0) * PUFF_SIZES as f64;
        let i = (at.floor() as usize).min(PUFF_SIZES - 1);
        let (a, b) = (f64::from(self.sizes[i]), f64::from(self.sizes[i + 1]));
        (a + (b - a) * (at - i as f64)).max(0.05)
    }

    /// No frame is smaller than this ([`PuffLook::frame_size`] between two of the clip's sizes), short of a
    /// millionth for rounding.
    fn smallest_frame(&self) -> f64 {
        let least = self.sizes.iter().copied().fold(f32::INFINITY, f32::min);
        f64::from(least).max(0.05) * (1.0 - 1e-6)
    }
}

/// A puff's place in the drawing order as one number: farther first (`distance` by `f64::total_cmp`, the other way),
/// then by its `tie` (`f32::total_cmp`), then by its `index`.
fn order_key(distance: f64, tie: f32, index: u32) -> u128 {
    // The bits of a float in its total order, as an unsigned number.
    let far = distance.to_bits();
    let far = if far >> 63 == 1 { !far } else { far | 1 << 63 };
    let tie = tie.to_bits();
    let tie = if tie >> 31 == 1 { !tie } else { tie | 1 << 31 };
    u128::from(!far) << 64 | u128::from(tie) << 32 | u128::from(index)
}

/// FNV-1a over `parts`, each followed by a zero byte.
fn fnv(parts: &[&[u8]]) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    for part in parts {
        for byte in part.iter().chain([&0u8]) {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

/// A number in 0–1 from a puff's place `id` and a draw `n`: the same every frame (SplitMix64's finalizer).
fn place_draw(id: u64, n: u64) -> f64 {
    let mut z = id
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(n.wrapping_mul(0xD1B5_4A32_D192_ED03));
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
}

/// The number of the place `j` cuts along the link from sample `seq` to the next, from the newer end; the cut at 0
/// is the newer sample's own place.
fn place_id(seq: u64, j: u64) -> u64 {
    seq * LINK_CUTS + (LINK_CUTS - j) % LINK_CUTS + if j == 0 { LINK_CUTS } else { 0 }
}

/// The game's look: the trail as puffs of the gun smoke's simulated smoke ([`clip::SMOKE`]) along the released samples
/// and the rim, placed as the links are (`draw_links`): every stretch between two samples is cut into [`LINK_CUTS`]
/// places carried by the air with them, each drawn only while the plume needs it (places whose number ends in more
/// zero bits stand in for their neighbours as it widens), so the puffs stand about [`PUFF_APART`] radii apart all the
/// way along and each fades in or out as it is needed, never popping. A puff stands a little off the spine and is of a
/// size of its own, both by its place; it plays the smoke clip over the plume's life, rolling and thinning, and stops
/// a share of what the ribbon's middle stopped there. `points` is working room.
/// Whether none of the puffs [`draw_puffs`] would draw for `emitter` stands in the view's `planes`, as each puff's own
/// test there would find: each stands within its width's reach of a link between two samples (or the rim) and is no
/// larger than the clip's smallest frame lets it be, so a ball about them all wholly outside one plane by that much is
/// out of the view. Most of a fleet's plumes are, and cost no more than this.
fn plume_unseen(
    look: &PuffLook<'_>,
    emitter: &Emitter,
    mouth: Option<DVec3>,
    width: f64,
    camera: &CameraView,
    planes: &[crate::camera::Plane; 6],
) -> bool {
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    let mut widest: f64 = 0.0;
    for p in &emitter.trail {
        low = low.min(p.position);
        high = high.max(p.position);
        // `puff_spread` with its growth's exponential at its limit.
        widest = widest.max(p.width0 * 4.0 + 1.2 * p.age);
    }
    if let Some(mouth) = mouth {
        low = low.min(mouth);
        high = high.max(mouth);
        widest = widest.max(width);
    }
    // A puff's radius at most (`mean` with `far` at its most, drawn 1.2 times), how far it may stand off its link
    // (across, up, and out of the funnel by the rim), and the margin its own test gives it.
    let radius = widest * PUFF_WIDTH * 2.0 * 1.2;
    let off = widest * (PUFF_ACROSS + PUFF_UP) + 0.7 * radius;
    let drawn = radius / (PUFF_GAS * look.smallest_frame());
    let centre = (low + high) * 0.5;
    let reach = (high - low).length() * 0.5 + off;
    // How much more or less the Earth's curve lowers a point within `reach` than the centre (`camera.drawn`).
    let level = (centre - camera.position).with_y(0.0).length();
    let sag = camera.curvature.abs() * reach * (2.0 * level + reach);
    let margin = (reach + sag + 1.6 * drawn) * (1.0 + 1e-6) + 1e-3;
    let seen = camera.drawn(centre);
    planes.iter().any(|plane| plane.distance(seen) < -margin)
}

#[allow(clippy::too_many_arguments)]
fn draw_puffs(
    look: &PuffLook<'_>,
    emitter: &Emitter,
    mouth: Option<DVec3>,
    width: f64,
    camera: &CameraView,
    planes: &[crate::camera::Plane; 6],
    points: &mut Vec<TrailPoint>,
    puffs: &mut Vec<(f64, GasPuff)>,
) {
    if emitter.trail.is_empty() {
        return;
    }
    // The samples, oldest first, and the rim as one more just released; each `along` becomes the metres of trail
    // behind it, from the oldest.
    points.clear();
    points.extend(emitter.trail.iter().copied());
    // Here `opacity` holds the optical depth through the plume's middle (`puff_optical`), worked out only for the
    // samples of links that draw (NaN until then): most links of a wide plume draw none.
    for p in points.iter_mut() {
        p.width = puff_spread(p);
        p.spent = (p.age / p.life).min(1.0);
        p.opacity = f64::NAN;
    }
    if let (Some(mouth), Some(newest)) = (mouth, points.last().copied()) {
        let mut rim = TrailPoint {
            position: mouth,
            width,
            seq: newest.seq + 1,
            age: 0.0,
            spent: 0.0,
            ..newest
        };
        rim.width0 = width;
        rim.opacity = f64::NAN;
        points.push(rim);
    }
    if points.len() < 2 {
        return;
    }
    let mut run = 0.0;
    for i in 0..points.len() {
        if i > 0 {
            run += (points[i].position - points[i - 1].position).length();
        }
        points[i].along = run;
    }
    let total = run;
    // Far off, fewer and larger puffs (each as dark as the plume's smoke spread over its width allows): a puff a
    // pixel or two across would flicker between the pixels.
    let far = smoothstep(
        (points[points.len() - 1].position - camera.position).length(),
        1500.0,
        6000.0,
    );
    for i in 0..points.len() - 1 {
        let toward = points[i].position - points[i + 1].position;
        let stretch = toward.length();
        let cut = stretch / LINK_CUTS as f64;
        // The fewest places a cut of the link needs (`places` below at its narrower end), short of a millionth for
        // rounding. A link whose most-ranked place (the newer sample's own, a multiple of `LINK_CUTS`) falls two ranks
        // short of that draws nothing, and needs neither its frame nor its samples' depth.
        let fewest = look.apart
            * (points[i].width.min(points[i + 1].width) * PUFF_WIDTH * (1.0 + far))
            / cut.max(1e-3)
            * (1.0 - 1e-6);
        let short = |zeros: u32| fewest >= (4u64 << zeros) as f64;
        if short(place_id(points[i].seq, 0).trailing_zeros().min(20)) {
            continue;
        }
        for p in &mut points[i..i + 2] {
            if p.opacity.is_nan() {
                p.opacity = puff_optical(p, p.width);
            }
        }
        let (old, new) = (&points[i], &points[i + 1]);
        let axis = if stretch > 1e-3 {
            toward / stretch
        } else {
            DVec3::Y
        };
        // Across the plume, level, and the third way out of it (up, for a plume trailing astern).
        let across = axis.cross(DVec3::Y);
        let across = if across.length_squared() > 1e-6 {
            across.normalize()
        } else {
            DVec3::X
        };
        let up = across.cross(axis);
        for j in 0..LINK_CUTS {
            let id = place_id(old.seq, j);
            let zeros = id.trailing_zeros().min(20);
            if short(zeros) {
                continue;
            }
            // From the newer end: the cut at 0 is the newer sample's own place.
            let share = j as f64 / LINK_CUTS as f64;
            let w = new.width + (old.width - new.width) * share;
            let mean = w * PUFF_WIDTH * (1.0 + far);
            // Ranks a place needs to be drawn: the places at rank r stand cut × 2^r apart.
            let places = look.apart * mean / cut.max(1e-3);
            // A place two ranks short (`need` a rank or more past its own) has no weight: most places of a wide
            // plume, left out before the logarithm.
            if places >= (4u64 << zeros) as f64 {
                continue;
            }
            let need = places.log2() - 1.0;
            let rank = f64::from(zeros);
            let weight = smoothstep(rank - need, -1.0, 0.0);
            if weight <= 0.01 {
                continue;
            }
            // Metres of trail behind the place (to the oldest sample) and ahead of it (to the rim).
            let behind = new.along - stretch * share;
            let ahead = total - behind;
            let middle = (new.opacity + (old.opacity - new.opacity) * share)
                // The oldest end tapers, as the ribbon's does.
                * smoothstep(behind, 0.0, (w * 1.5).max(40.0));
            if middle < 0.004 {
                continue;
            }
            let draw = |n: u64| place_draw(id ^ emitter.salt, n);
            let radius = mean * (0.8 + 0.4 * draw(0));
            let mut at = new.position
                + toward * share
                + across * ((draw(1) - 0.5) * 2.0 * PUFF_ACROSS * w)
                + up * ((draw(2) - 0.5) * 2.0 * PUFF_UP * w);
            if mouth.is_some() {
                // By the rim a puff stands out of the funnel, its foot about the mouth.
                at += axis * (radius * 0.7 - ahead).max(0.0);
            }
            let life = new.spent + (old.spent - new.spent) * share;
            // The quad the gas is drawn in: the frame's gas fills a share of its tile, and grows through the clip.
            let drawn = radius / (PUFF_GAS * look.frame_size(life));
            // A puff seen from its sides may be drawn up to half as wide again (`flipbook_widen`).
            let seen = camera.drawn(at);
            if planes
                .iter()
                .any(|plane| plane.distance(seen) < -drawn * 1.6)
            {
                continue;
            }
            let age = new.age + (old.age - new.age) * share;
            // What the plume's middle stops here, shared among the puffs a point lies in, and spread over the wider
            // puffs far off: each puff's gas stops that much on the mean over its disc.
            let optical = middle * look.apart / PUFF_OVERLAP / (1.0 + far);
            // Older, it thins and its rim tears into wisps (`puff_at`), which the density makes up for in its middle,
            // as it does for the clip's own thinning (`smoke_depth`).
            let dispersal = 0.3 * smoothstep(life, 0.3, 1.0);
            let density = 3.0 * optical / (GAS_DEPTH * smoke_depth(life)) / (1.0 - 0.8 * dispersal);
            // The plume's own colour: the gas's six baked lights take the sun and the sky as the ribbon's tube did.
            let color = new.color;
            puffs.push((
                (at - camera.position).length_squared(),
                GasPuff {
                    sphere: [at.x as f32, at.y as f32, at.z as f32, drawn as f32],
                    state: [age as f32, (100.0 * draw(3)) as f32, 0.0, density as f32],
                    tint: [color.r as f32, color.g as f32, color.b as f32, 1.0],
                    evolution: [life as f32, dispersal as f32, 0.0, 0.0],
                    // The sprite shader places a flipbook puff's quad itself.
                    quad: [0.0; 4],
                    extra: [weight as f32, 0.0, clip::SMOKE as f32, GAS_SMOKE],
                    warp: [0.0; 4],
                },
            ));
        }
    }
}

/// Append one camera-facing strip through `points` (oldest first): `SmokeRibbons.strip`.
fn strip(points: &[TrailPoint], camera: &CameraView, ribbons: &mut FunnelPublishRibbons) {
    let count = points.len();
    if count < 2 || ribbons.vertices.len() + count * 2 > RIBBON_CAPACITY {
        return;
    }
    let (_, _, back) = camera.axes();
    let forward = -back;
    let first = ribbons.vertices.len() as u32;
    let mut previous_side = DVec3::ZERO;
    for i in 0..count {
        let p = &points[i];
        let previous = &points[i.saturating_sub(1)];
        let next = &points[(i + 1).min(count - 1)];
        let mut tangent = next.position - previous.position;
        tangent = if tangent.length_squared() < 1e-6 {
            DVec3::Y
        } else {
            tangent.normalize()
        };
        let mut to_camera = camera.position - p.position;
        let distance = to_camera.length();
        to_camera /= distance.max(1e-4);
        let mut side = tangent.cross(to_camera);
        let side_length = side.length();
        // End-on, blend toward a stable horizontal side instead of letting the strip twist.
        let mut fallback = DVec3::Y.cross(to_camera);
        if fallback.length_squared() < 1e-6 {
            fallback = DVec3::X;
        }
        fallback = fallback.normalize();
        if side_length > 1e-5 {
            side /= side_length;
        }
        // Either side is as good as the other, but it must stay the same side along the strip.
        if i == 0 {
            if side.dot(fallback) < 0.0 {
                side = -side;
            }
        } else {
            if side.dot(previous_side) < 0.0 {
                side = -side;
            }
            if fallback.dot(previous_side) < 0.0 {
                fallback = -fallback;
            }
        }
        let end_on = 1.0 - smoothstep(side_length, 0.08, 0.45);
        side = side.lerp(fallback, end_on);
        side = if side.length_squared() < 1e-6 {
            if i > 0 { previous_side } else { fallback }
        } else {
            side.normalize()
        };
        previous_side = side;
        let depth = to_camera.dot(forward) * -distance;
        // A plume passing the lens would cover the screen, and one level with it to the side would stretch its
        // camera-facing quads into bars across the view (their corners reach half a width toward the lens); the
        // sprites carry it there.
        let near_fade = smoothstep(distance, p.width * 0.7, p.width * 2.5)
            * smoothstep(depth, p.width * 0.75, p.width * 2.0);
        let opacity = p.opacity * near_fade * (1.0 - end_on * 0.55);
        // The side's length carries how squarely the plume faces the camera (1) or recedes (→0).
        let facing = side_length.clamp(0.05, 1.0);
        let half = p.width / 2.0;
        for across in [-1.0, 1.0] {
            let position = p.position + side * (across * half);
            ribbons.vertices.push(RibbonVertex {
                position: [
                    position.x as f32,
                    position.y as f32,
                    position.z as f32,
                    p.spent as f32,
                ],
                side: [
                    (side.x * facing) as f32,
                    (side.y * facing) as f32,
                    (side.z * facing) as f32,
                    0.0,
                ],
                data: [across as f32, p.along as f32, p.age as f32, p.width as f32],
                tint: [
                    p.color.r as f32,
                    p.color.g as f32,
                    p.color.b as f32,
                    opacity as f32,
                ],
            });
        }
    }
    for i in 0..count as u32 - 1 {
        let a = first + i * 2;
        ribbons
            .indices
            .extend_from_slice(&[a, a + 1, a + 2, a + 2, a + 1, a + 3]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DMat4;

    fn camera(eye: DVec3, target: DVec3) -> CameraView {
        CameraView {
            position: eye,
            view: glam::dcamera::rh::view::look_at_mat4(eye, target, DVec3::Y),
            projection: DMat4::IDENTITY,
            near: 0.5,
            fov_y_degrees: 90.0,
            height: 1000.0,
            curvature: 0.0,
        }
    }

    /// An underway plume as `draw_trail` hands it over, oldest first: released at a funnel 40 m up every half
    /// second, spreading as it ages and carried aft and across by the ship's way and the wind.
    fn underway_trail() -> Vec<TrailPoint> {
        let count = 80;
        (0..count)
            .map(|k| {
                let age = (count - 1 - k) as f64 * TRAIL_INTERVAL;
                let width0 = 8.0;
                TrailPoint {
                    position: DVec3::new(
                        2.5 * age,
                        40.0 + 6.0 * (1.0 - (-age / 4.0).exp()),
                        13.0 * age,
                    ),
                    color: cruise(),
                    width: width0 * (1.0 + 1.9 * (1.0 - (-age / 5.0).exp())) + 0.8 * age,
                    width0,
                    opacity: 0.6,
                    opacity0: 0.6,
                    along: (count - k) as f64 * 6.5,
                    age,
                    spent: age / TRAIL_LIFE_S,
                    life: TRAIL_LIFE_S,
                    ..TrailPoint::default()
                }
            })
            .collect()
    }

    /// A ship steaming along +X at `speed` m/s for 40 s with one funnel 30 m up, the camera 300 m off her beam; the
    /// funnels and what they drew at the end.
    fn steam(plume: bool, speed: f64) -> (FunnelSmoke, Vec<GasPuff>, FunnelPublish, DVec3) {
        let (smoke, links, publish, rim, _) = steam_with(speed, |smoke| smoke.plume = plume);
        (smoke, links, publish, rim)
    }

    /// [`steam`] with the funnels' look set by `look`, and the puffs published for the camera at the end.
    #[allow(clippy::type_complexity)]
    fn steam_with(
        speed: f64,
        look: impl FnOnce(&mut FunnelSmoke),
    ) -> (
        FunnelSmoke,
        Vec<GasPuff>,
        FunnelPublish,
        DVec3,
        Vec<GasPuff>,
    ) {
        let profile = std::sync::Arc::new(crate::ships::ShipEffectsProfile {
            hull_length: 240.0,
            hull_beam: 36.0,
            forward_speed: 15.0,
            reverse_speed: 6.0,
            funnels: vec![FunnelOutlet {
                id: "funnel".into(),
                position: DVec3::new(0.0, 30.0, 0.0),
                width: 8.0,
                length: 10.0,
                bearing_rad: None,
            }],
            ..Default::default()
        });
        let mut smoke = FunnelSmoke::new();
        look(&mut smoke);
        let mut view = camera(DVec3::ZERO, DVec3::X);
        let mut x = 0.0;
        let mut rim = DVec3::ZERO;
        for _ in 0..40 * 30 {
            x += speed / 30.0;
            let pose = crate::math::ShipPose {
                x,
                y: 0.0,
                z: 0.0,
                heading: std::f64::consts::FRAC_PI_2,
                pitch: 0.0,
                roll: 0.0,
            };
            let mut ship = ShipFrame::new("ship", profile.clone(), pose);
            ship.velocity = DVec3::new(speed, 0.0, 0.0);
            ship.speed = speed;
            rim = pose.local_to_world(DVec3::new(0.0, 30.0, 0.0));
            view = camera(
                DVec3::new(x - 150.0, 60.0, 300.0),
                DVec3::new(x - 150.0, 40.0, 0.0),
            );
            view.projection = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                60f64.to_radians(),
                16.0 / 9.0,
                0.5,
            );
            smoke.update(&[ship], 1.0 / 30.0, &view, DVec3::new(2.4, 0.0, 0.9), None);
        }
        let mut links = Vec::new();
        smoke.publish_links_into(&mut links);
        let publish = smoke.publish(&view, None);
        let mut puffs = Vec::new();
        smoke.publish_puffs_into(&view, None, &mut puffs);
        (smoke, links, publish, rim, puffs)
    }

    /// How far astern of the rim (m) the trail reaches after a ship steams along +X at 10 m/s in still air for 200
    /// simulated seconds, stepped `dt` simulated seconds a frame.
    fn trail_reach(dt: f64) -> f64 {
        let profile = std::sync::Arc::new(crate::ships::ShipEffectsProfile {
            hull_length: 240.0,
            hull_beam: 36.0,
            forward_speed: 15.0,
            reverse_speed: 6.0,
            funnels: vec![FunnelOutlet {
                id: "funnel".into(),
                position: DVec3::new(0.0, 30.0, 0.0),
                width: 8.0,
                length: 10.0,
                bearing_rad: None,
            }],
            ..Default::default()
        });
        let speed = 10.0;
        let mut smoke = FunnelSmoke::new();
        let mut x = 0.0;
        for _ in 0..(200.0 / dt).round() as usize {
            x += speed * dt;
            let pose = crate::math::ShipPose {
                x,
                y: 0.0,
                z: 0.0,
                heading: std::f64::consts::FRAC_PI_2,
                pitch: 0.0,
                roll: 0.0,
            };
            let mut ship = ShipFrame::new("ship", profile.clone(), pose);
            ship.velocity = DVec3::new(speed, 0.0, 0.0);
            ship.speed = speed;
            let view = camera(DVec3::new(x, 60.0, 300.0), DVec3::new(x, 40.0, 0.0));
            smoke.update(&[ship], dt, &view, DVec3::ZERO, None);
        }
        let spine = &smoke.spines()[0];
        spine
            .samples
            .iter()
            .map(|s| spine.rim.x - s.position.x)
            .fold(0.0, f64::max)
    }

    #[test]
    fn a_departing_ship_does_not_restart_another_ships_plume() {
        let (mut smoke, _, _, rim) = steam(true, 10.0);
        let profile = std::sync::Arc::new(crate::ships::ShipEffectsProfile {
            forward_speed: 15.0,
            reverse_speed: 6.0,
            funnels: vec![smoke.emitters["ship"].funnels[0].outlet.clone()],
            ..Default::default()
        });
        let mut ship = ShipFrame::new(
            "ship",
            profile.clone(),
            crate::math::ShipPose {
                x: rim.x,
                ..Default::default()
            },
        );
        ship.velocity = DVec3::new(10.0, 0.0, 0.0);
        ship.speed = 10.0;
        let other = ShipFrame::new("departing", profile, crate::math::ShipPose::default());
        let view = camera(rim + DVec3::new(0.0, 60.0, 300.0), rim);
        smoke.update(&[ship.clone(), other], 0.1, &view, DVec3::ZERO, None);
        smoke.update(&[ship], 0.0, &view, DVec3::ZERO, None);
        let oldest = smoke.spines()[0]
            .samples
            .iter()
            .map(|p| p.age)
            .fold(0.0, f64::max);
        assert!(
            oldest > 20.0,
            "a departing ship reset an established plume to {oldest} s"
        );
    }

    #[test]
    fn a_fast_clock_leaves_the_trail_its_length() {
        // A theatre battle at 32× steps its smoke over half a second a frame: aged that fast, the trail ends where it
        // does at 1× (it once ran 5× as far astern, its step held to 0.1 s).
        let real = trail_reach(1.0 / 60.0);
        let fast = trail_reach(32.0 / 60.0);
        assert!((450.0..560.0).contains(&real), "{real}");
        assert!((fast - real).abs() < 0.05 * real, "{fast} against {real}");
    }

    #[test]
    fn a_plume_of_puffs_runs_from_the_rim_and_widens_and_thins_downwind() {
        let centre = |p: &GasPuff| {
            DVec3::new(
                f64::from(p.sphere[0]),
                f64::from(p.sphere[1]),
                f64::from(p.sphere[2]),
            )
        };
        // The gas a puff draws reaches this far from its middle (its frames all of a size here).
        let gas = |p: &GasPuff| f64::from(p.sphere[3]) * PUFF_GAS;
        for speed in [0.0, 14.0] {
            let (mut smoke, links, publish, rim, puffs) =
                steam_with(speed, |smoke| smoke.puffs = true);
            // Each view draws its own: one looking away from the plume draws none, and the plume's are still there for
            // the next (a second view, the fall-of-shot window, sees what the first does not).
            let mut away = camera(
                rim + DVec3::new(0.0, 0.0, 300.0),
                rim + DVec3::new(0.0, 0.0, 600.0),
            );
            away.projection = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                60f64.to_radians(),
                16.0 / 9.0,
                0.5,
            );
            let mut none = Vec::new();
            smoke.publish_puffs_into(&away, None, &mut none);
            assert!(none.is_empty(), "{} puffs behind the lens", none.len());
            // Optics on her own hull leave her smoke out.
            let toward = camera(rim + DVec3::new(-150.0, 30.0, 300.0), rim);
            smoke.publish_puffs_into(&toward, Some("ship"), &mut none);
            assert!(none.is_empty());
            // Puffs in place of the billows, the ribbon and the links.
            assert!(
                links.is_empty() && publish.billows.is_empty() && publish.ribbon_indices.is_empty()
            );
            assert!(
                (8..200).contains(&puffs.len()),
                "{} puffs at {speed} m/s",
                puffs.len()
            );
            assert!(puffs.iter().all(|p| p.extra[2] == clip::SMOKE as f32));
            // One stands at the rim, its gas reaching down to it.
            let first = puffs
                .iter()
                .map(|p| (centre(p) - rim).length() - gas(p))
                .fold(f64::INFINITY, f64::min);
            assert!(
                first < 1.0,
                "at {speed} m/s the nearest puff's gas is {first:.1} m from the rim"
            );
            // Along the trail the gas of the puffs drawn whole overlaps from one to the next.
            let mut byrim: Vec<&GasPuff> = puffs.iter().filter(|p| p.extra[0] > 0.99).collect();
            byrim.sort_by(|a, b| {
                (centre(a) - rim)
                    .length()
                    .total_cmp(&(centre(b) - rim).length())
            });
            for pair in byrim.windows(2) {
                let gap = (centre(pair[1]) - centre(pair[0])).length();
                assert!(
                    gap < gas(pair[0]) + gas(pair[1]),
                    "a gap of {gap:.1} m at {speed} m/s"
                );
            }
            // Downwind it is wider, and its smoke thinner.
            let fifth = byrim.len() / 5;
            let mean = |puffs: &[&GasPuff], of: &dyn Fn(&GasPuff) -> f64| {
                puffs.iter().map(|p| of(p)).sum::<f64>() / puffs.len() as f64
            };
            let (near, far) = (&byrim[..fifth], &byrim[byrim.len() - fifth..]);
            let radius = |p: &GasPuff| gas(p);
            let density = |p: &GasPuff| f64::from(p.state[3]);
            assert!(
                mean(far, &radius) > 2.0 * mean(near, &radius),
                "at {speed} m/s: radius {:.1} m by the rim, {:.1} m downwind",
                mean(near, &radius),
                mean(far, &radius)
            );
            assert!(
                mean(far, &density) < 0.5 * mean(near, &density),
                "at {speed} m/s: density {:.2} by the rim, {:.2} downwind",
                mean(near, &density),
                mean(far, &density)
            );
        }
    }

    #[test]
    fn a_chain_of_links_runs_from_the_rim_without_a_gap() {
        // Switched off, the funnel draws its billows and ribbon and no links.
        let (_, links, publish, _) = steam(false, 14.0);
        assert!(
            links.is_empty() && !publish.billows.is_empty() && !publish.ribbon_indices.is_empty()
        );
        for speed in [0.0, 14.0] {
            let (_, links, publish, rim) = steam(true, speed);
            // Switched on: links in place of both.
            assert!(publish.billows.is_empty() && publish.ribbon_indices.is_empty());
            let centre = |l: &GasPuff| {
                DVec3::new(
                    f64::from(l.sphere[0]),
                    f64::from(l.sphere[1]),
                    f64::from(l.sphere[2]),
                )
            };
            // One stands at the rim, its near end there.
            let first = links
                .iter()
                .map(|l| (centre(l) - rim).length() - f64::from(l.sphere[3]))
                .fold(f64::INFINITY, f64::min);
            assert!(
                first < 2.0,
                "at {speed} m/s the nearest link's end is {first:.1} m from the rim"
            );
            // And along the trail no two neighbours stand further apart than a link is long.
            let mut byrim: Vec<&GasPuff> = links.iter().filter(|l| l.extra[0] > 0.0).collect();
            byrim.sort_by(|a, b| {
                (centre(a) - rim)
                    .length()
                    .total_cmp(&(centre(b) - rim).length())
            });
            for pair in byrim.windows(2) {
                let gap = (centre(pair[1]) - centre(pair[0])).length();
                assert!(
                    gap < f64::from(pair[0].sphere[3].max(pair[1].sphere[3])) * 2.0,
                    "a gap of {gap:.1} m at {speed} m/s"
                );
            }
            println!("at {speed} m/s: {} links", links.len());
            assert!(links.len() < 120);
        }
    }

    /// A camera in or beside the plume looking up it toward the funnel: no part of the strip it can see may lie
    /// nearly level with the lens, where a camera-facing quad projects into a long bar across the screen.
    #[test]
    fn a_plume_beside_the_camera_draws_no_bars_across_the_view() {
        let points = underway_trail();
        for x in [-40.0, -25.0, -10.0, 0.0, 10.0, 25.0, 40.0, 60.0] {
            for z in [30.0, 60.0, 90.0, 150.0, 250.0, 400.0] {
                for y in [35.0, 45.0, 60.0] {
                    let eye = DVec3::new(x, y, z);
                    let camera = camera(eye, DVec3::new(0.0, 40.0, 0.0));
                    let mut ribbons = FunnelPublishRibbons::default();
                    strip(&points, &camera, &mut ribbons);
                    for vertex in &ribbons.vertices {
                        if vertex.tint[3] < 0.005 {
                            continue;
                        }
                        let [px, py, pz, _] = vertex.position.map(f64::from);
                        let depth = -camera.to_view(DVec3::new(px, py, pz)).z;
                        let width = f64::from(vertex.data[3]);
                        assert!(
                            depth > width * 0.2,
                            "from {eye:?} a vertex {depth:.2} m in front of the lens (plume {width:.1} m wide) is \
                             drawn at opacity {:.3}",
                            vertex.tint[3]
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_plume_ruled_out_of_view_draws_no_puff_there() {
        // A ring of views about the plume, near and far, level and from above, flat and curved: plumes in view, out
        // of it and half in it.
        let (smoke, _, _, rim, _) = steam_with(14.0, |smoke| smoke.puffs = true);
        let look = PuffLook {
            apart: PUFF_APART,
            sizes: &smoke.smoke_sizes,
        };
        let emitter = &smoke.emitters.values().next().expect("her plume").funnels[0];
        let mouth = (emitter.outlet.width * 0.9).max(1.5);
        let (mut ruled_out, mut drawn) = (0, 0);
        for k in 0..96 {
            let bearing = k as f64 * 0.37;
            let range = [120.0, 400.0, 1500.0, 5000.0][k % 4];
            let eye = rim
                + DVec3::new(
                    bearing.cos() * range,
                    [5.0, 60.0, 400.0][k % 3],
                    bearing.sin() * range,
                );
            // Toward a place along the plume, away from it, or across it.
            let along =
                rim + DVec3::new(-200.0 * (k % 5) as f64, 0.0, 90.0 * (k % 7) as f64 - 270.0);
            let target = match k % 3 {
                0 => along,
                1 => eye * 2.0 - along,
                _ => eye + (along - eye).cross(DVec3::Y),
            };
            let mut view = camera(eye, target);
            view.projection = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
                60f64.to_radians(),
                16.0 / 9.0,
                0.5,
            );
            view.curvature = if k % 2 == 0 { 0.0 } else { 1.0 / 6.371e6 };
            let planes = view.frustum();
            let (mut points, mut puffs) = (Vec::new(), Vec::new());
            draw_puffs(
                &look,
                emitter,
                Some(emitter.previous),
                mouth,
                &view,
                &planes,
                &mut points,
                &mut puffs,
            );
            if plume_unseen(
                &look,
                emitter,
                Some(emitter.previous),
                mouth,
                &view,
                &planes,
            ) {
                ruled_out += 1;
                assert!(
                    puffs.is_empty(),
                    "view {k}: {} puffs in a plume ruled out",
                    puffs.len()
                );
            } else if !puffs.is_empty() {
                drawn += 1;
            }
        }
        // Both kinds of view are tried.
        assert!(
            ruled_out >= 20 && drawn >= 20,
            "{ruled_out} ruled out, {drawn} drawn"
        );
    }
}
