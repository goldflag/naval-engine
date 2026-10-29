//! Solid debris a strike throws: torn plating, splinters of the shell, deck planking and fittings, tumbling on
//! ballistic arcs. A hot fragment glows and burns as it flies and cools to a dark chunk; the larger pieces trail
//! smoke; whatever reaches the sea throws a puff of spray, and a plank of timber floats there (`Debris::take_landed`,
//! the flotsam's). Not a port: the browser game's strikes threw only additive sparks.
//!
//! A magazine's wreckage is torn plating and girders (`DebrisBurst::plating`): plates folded and crumpled, angle
//! sections kinked part way, as the photographs and accounts show it (Queen Mary's "turret roofs … blown 100 feet into
//! the air", Proceedings, April 1925; Mount Hood's largest pieces "no bigger than 16 by 10 feet", DANFS), each
//! trailing an unbroken streak of smoke: its puffs laid closer than they are wide and drawn out along its flight.
//!
//! The pieces are drawn as small lit, faceted solids (`shaders/debris.wgsl`) in the opaque phase, so the sea hides
//! what has sunk and the smoke drawn after sorts against them. Their trails, flames and splashes go into the
//! effects' own pools (soot and fire), so they light, fade and drift as every other puff.

use glam::{DQuat, DVec3};

use crate::camera::CameraView;
use crate::math::{Rgb, clamp, normalize};
use crate::particles::{Align, ParticlePool, SourceId};
use crate::random::Lcg;

/// Pieces in the air at once: a full set gives up the piece nearest the end of its flight.
pub const CAPACITY: usize = 640;
/// Vertices a piece is drawn with (`shaders/debris.wgsl`, `PIECE_VERTICES`): up to four boxes of twelve triangles,
/// as its shape needs (`crate::flotsam::shape`).
pub const PIECE_VERTICES: u32 = 144;
/// Height of the sea a piece splashes into (the effects' flat waterline, as the particle pools' `waterline`).
const SEA_Y: f64 = 0.15;
/// A strike's trails: grey smoke of burning paint and cork.
fn trail_smoke() -> Rgb {
    Rgb::hex(0x707275)
}

/// One piece in flight.
#[derive(Clone, Debug)]
struct Piece {
    position: DVec3,
    velocity: DVec3,
    /// Tumbling: about a fixed axis at a fixed rate, from a random starting turn.
    axis: DVec3,
    spin: f64,
    phase: f64,
    /// Half extents (m): length, width and thickness of the shard.
    extents: DVec3,
    albedo: Rgb,
    /// Incandescence at birth (0 cold, 1 yellow-hot) and how fast it falls (1/s).
    heat: f64,
    cooling: f64,
    /// Its shape (`crate::flotsam::shape`): a shard, a torn plate, a girder or a plank.
    shape: f32,
    /// Smoke trail: puffs per metre flown while it lasts, and the puffs' size.
    trail: f64,
    trail_size: f64,
    trail_life: f64,
    trail_strength: f64,
    trail_colour: Rgb,
    credit: f64,
    drag: f64,
    age: f64,
    life: f64,
    seed: f32,
    source: Option<SourceId>,
}

impl Piece {
    fn glow(&self) -> f64 {
        if self.age < 0.0 {
            return 0.0;
        }
        self.heat * (-self.age * self.cooling).exp()
    }
}

/// How a burst of debris is thrown.
#[derive(Clone, Copy, Debug)]
pub struct DebrisBurst {
    /// Where the pieces start and the cone they are thrown in (axis, half angle in radians).
    pub at: DVec3,
    pub axis: DVec3,
    pub spread: f64,
    pub count: usize,
    /// Throw speed range (m/s) and piece size range (largest half extent, m).
    pub speed: (f64, f64),
    pub size: (f64, f64),
    /// Share of pieces thrown hot (glowing, a flame licking off the hottest), and of pieces trailing smoke.
    pub hot: f64,
    pub smoking: f64,
    /// How thick the smoking pieces' trails are: 0 none, about 0.5 for plating torn by a shell, 1.5 for burning
    /// wreckage hurled by a magazine.
    pub trail: f64,
    /// The trails' smoke: grey for a strike's scraps (burning paint and cork), paler for a magazine's wreckage.
    pub trail_colour: Rgb,
    /// Paint and steel (0) or a share of deck planking (pale timber).
    pub timber: f64,
    /// The share of its steel thrown as torn plates and girders rather than shards (a magazine's wreckage).
    pub plating: f64,
    /// The share of it that is clods of earth and stones rather than steel (a shell bursting on the land).
    pub earth: f64,
    /// A start delay, for a burst that follows its strike (s).
    pub delay: f64,
    /// The velocity of what the pieces come off (m/s), added to their throw: an airframe's way, which the air soon
    /// takes off them.
    pub carry: DVec3,
}

impl Default for DebrisBurst {
    fn default() -> Self {
        Self {
            at: DVec3::ZERO,
            axis: DVec3::Y,
            spread: 0.6,
            count: 0,
            speed: (10.0, 40.0),
            size: (0.08, 0.4),
            hot: 0.3,
            smoking: 0.0,
            trail: 0.0,
            trail_colour: trail_smoke(),
            timber: 0.0,
            plating: 0.0,
            earth: 0.0,
            delay: 0.0,
            carry: DVec3::ZERO,
        }
    }
}

/// Debris instance as `shaders/debris.wgsl` reads it (twin `DebrisPiece`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct DebrisInstance {
    /// Centre (m) and the shard's seed.
    pub position: [f32; 4],
    /// Orientation (quaternion x, y, z, w).
    pub rotation: [f32; 4],
    /// Half extents (m) and its shape (`crate::flotsam::shape`).
    pub extents: [f32; 4],
    /// Albedo and glow (incandescence, 0 … 1).
    pub color: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<DebrisInstance>() == 64);

/// The pieces in flight.
#[derive(Clone, Debug)]
pub struct Debris {
    pieces: Vec<Piece>,
    cursor: usize,
    /// Fraction of requested pieces thrown (the combat effects setting).
    pub density: f64,
    thinning: f64,
    splashes: u64,
    /// Planks that came down on the water since the last [`Debris::take_landed`]: where, how long (m), a seed and
    /// whose.
    landed: Vec<(DVec3, f64, f32, Option<SourceId>)>,
}

impl Default for Debris {
    fn default() -> Self {
        Self::new()
    }
}

/// A unit direction inside the cone about `axis` of half angle `spread`, uniform over the cap.
fn in_cone(axis: DVec3, spread: f64, random: &mut Lcg) -> DVec3 {
    let axis = normalize(axis);
    let mut across = axis.cross(DVec3::Y);
    if across.length_squared() < 1e-4 {
        across = DVec3::X;
    }
    let across = normalize(across);
    let up = axis.cross(across);
    let cos = 1.0 - random.next() * (1.0 - spread.cos());
    let sin = (1.0 - cos * cos).max(0.0).sqrt();
    let turn = random.next() * std::f64::consts::TAU;
    axis * cos + (across * turn.cos() + up * turn.sin()) * sin
}

impl Debris {
    pub fn new() -> Self {
        Self {
            pieces: Vec::with_capacity(CAPACITY),
            cursor: 0,
            density: 1.0,
            thinning: 0.0,
            splashes: 0,
            landed: Vec::new(),
        }
    }

    /// Pieces in flight.
    pub fn alive(&self) -> usize {
        self.pieces.iter().filter(|p| p.age < p.life).count()
    }

    /// Splashes thrown so far.
    pub fn splashes(&self) -> u64 {
        self.splashes
    }

    pub fn reset(&mut self) {
        self.pieces.clear();
        self.cursor = 0;
        self.thinning = 0.0;
        self.landed.clear();
    }

    /// The planks that came down on the water since the last call, to float there (`crate::flotsam`).
    pub fn take_landed(&mut self) -> Vec<(DVec3, f64, f32, Option<SourceId>)> {
        std::mem::take(&mut self.landed)
    }

    fn slot(&mut self) -> usize {
        if self.pieces.len() < CAPACITY {
            self.pieces.push(Piece {
                position: DVec3::ZERO,
                velocity: DVec3::ZERO,
                axis: DVec3::Y,
                spin: 0.0,
                phase: 0.0,
                extents: DVec3::ONE,
                albedo: Rgb::WHITE,
                shape: 0.0,
                heat: 0.0,
                cooling: 1.0,
                trail: 0.0,
                trail_size: 0.0,
                trail_life: 0.0,
                trail_strength: 0.0,
                trail_colour: trail_smoke(),
                credit: 0.0,
                drag: 0.0,
                age: 0.0,
                life: 0.0,
                seed: 0.0,
                source: None,
            });
            return self.pieces.len() - 1;
        }
        // A spent piece, else the one furthest through its flight, scanning from the cursor.
        let mut best = self.cursor % CAPACITY;
        let mut best_key = f64::NEG_INFINITY;
        for k in 0..CAPACITY {
            let i = (self.cursor + k) % CAPACITY;
            let p = &self.pieces[i];
            let key = if p.age >= p.life {
                f64::INFINITY
            } else {
                p.age / p.life
            };
            if key > best_key {
                best_key = key;
                best = i;
                if key == f64::INFINITY {
                    break;
                }
            }
        }
        self.cursor = best + 1;
        best
    }

    /// Throws a burst. Randomness comes from the event's own stream, so a replay throws the same pieces.
    pub fn throw(&mut self, burst: &DebrisBurst, random: &mut Lcg, source: Option<&SourceId>) {
        for _ in 0..burst.count {
            // Every draw is taken whether or not the piece is thinned out, so density does not reshuffle the rest.
            let direction = in_cone(burst.axis, burst.spread, random);
            let speed = burst.speed.0 + (burst.speed.1 - burst.speed.0) * random.next().powf(1.6);
            // Many small pieces, few large ones.
            let size = burst.size.0 + (burst.size.1 - burst.size.0) * random.next().powi(3);
            let hot = random.next() < burst.hot;
            let smoking = random.next() < burst.smoking;
            let timber = random.next() < burst.timber;
            let flat = 0.12 + random.next() * 0.25;
            let width = 0.35 + random.next() * 0.5;
            let axis = normalize(DVec3::new(
                random.next() - 0.5,
                random.next() - 0.5,
                random.next() - 0.5,
            ));
            let spin = (4.0 + random.next() * 14.0) * if random.next() < 0.5 { -1.0 } else { 1.0 };
            let phase = random.next() * std::f64::consts::TAU;
            let shade = 0.7 + random.next() * 0.6;
            let seed = random.next() as f32;
            let jitter = DVec3::new(
                random.next() - 0.5,
                random.next() - 0.5,
                random.next() - 0.5,
            );
            let cooling = 0.6 + random.next() * 1.2;
            let life = 3.0 + random.next() * 4.0;
            // Its shape, drawn after the rest so a burst without plating draws as it did.
            let form = if burst.plating > 0.0 {
                random.next()
            } else {
                1.0
            };
            let girder = burst.plating > 0.0 && random.next() < 0.35;
            // Earth, drawn after the rest likewise.
            let earth = burst.earth > 0.0 && random.next() < burst.earth;
            self.thinning += self.density;
            if self.thinning < 1.0 {
                continue;
            }
            self.thinning -= 1.0;
            let i = self.slot();
            let p = &mut self.pieces[i];
            p.position = burst.at + jitter * (size * 2.0);
            p.velocity = direction * speed + burst.carry;
            p.axis = axis;
            p.spin = spin;
            p.phase = phase;
            p.extents = DVec3::new(size, size * width, size * flat);
            p.shape = crate::flotsam::shape::SHARD;
            if earth {
                // A clod or a stone: chunkier than a shard of steel.
                p.extents = DVec3::new(
                    size,
                    size * (0.55 + width * 0.5),
                    size * (0.45 + flat * 1.4),
                );
            } else if timber && size > 0.25 {
                // A plank of her deck: 4 to 6 in wide, about 3 in thick.
                p.extents = DVec3::new(size, 0.035 + 0.02 * flat, 0.05 + 0.03 * width);
                p.shape = crate::flotsam::shape::PLANK;
            } else if !timber && form < burst.plating && size > 0.3 {
                if girder {
                    // An angle section, legs a sixth of its length (at most 0.35 m), its steel an inch thick.
                    p.extents = DVec3::new(size * 1.2, (size * 0.18).clamp(0.08, 0.35), 0.025);
                    p.shape = crate::flotsam::shape::GIRDER;
                } else {
                    // Torn plating: plate of 15 to 25 mm, drawn no thinner than 3 cm.
                    p.extents = DVec3::new(size, (size * 0.02).max(0.03), size * width);
                    p.shape = crate::flotsam::shape::PLATE;
                }
            }
            // Grey paint and dark steel, pale timber, or brown earth and grey-brown stone; the hot ones are scaled by
            // their glow in the shader.
            p.albedo = if earth {
                if shade > 1.15 {
                    Rgb::hex(0x6a6358).scale(shade * 0.85)
                } else {
                    Rgb::hex(0x4a3e31).scale(shade)
                }
            } else if timber {
                Rgb::hex(0x8a7356).scale(shade)
            } else if hot {
                Rgb::hex(0x3a3836).scale(shade)
            } else {
                Rgb::hex(0x6d7075).scale(shade)
            };
            p.heat = if hot && !earth {
                0.7 + 0.3 * shade.min(1.0)
            } else {
                0.0
            };
            p.cooling = cooling;
            // A larger piece is slowed less by the air.
            p.drag = clamp(0.09 / size.max(0.03), 0.12, 2.2);
            // Thin grey wisps behind the larger pieces, a few puffs a metre; a magazine's burning wreckage streams an
            // unbroken trail, its puffs laid at half their width (drawn out along its flight, `update`).
            // A magazine's burning wreckage trails a plume of a few metres (its fire's smoke), a strike's a wisp.
            p.trail_size = if burst.plating > 0.0 {
                (0.3 + size * 1.2).min(2.5)
            } else {
                (0.12 + size * 0.9).min(1.6)
            };
            let spacing = if burst.plating > 0.0 { 0.5 } else { 1.1 };
            p.trail = if smoking && burst.trail > 0.0 && size > 0.1 {
                1.0 / (spacing * p.trail_size)
            } else {
                0.0
            };
            p.trail_life = 0.5 + size * 1.5 * burst.trail.min(1.5);
            p.trail_strength = burst.trail;
            p.trail_colour = burst.trail_colour;
            p.credit = 0.0;
            p.age = -burst.delay;
            // Long enough to come down (without the air's drag, which brings it down sooner and splashes it).
            let rise = p.velocity.y.max(0.0);
            let fall = (rise + (rise * rise + 2.0 * 9.81 * p.position.y.max(0.0)).sqrt()) / 9.81;
            p.life = life.max(fall + 0.5).min(24.0);
            p.seed = seed;
            p.source = source.cloned();
        }
    }

    /// The shape of the pieces in flight that are of `shape`, for tests.
    pub fn count_shaped(&self, shape: f32) -> usize {
        self.pieces
            .iter()
            .filter(|p| p.age < p.life && p.shape == shape)
            .count()
    }

    /// Moves every piece by `dt`: drag and gravity, its trail and flame into `soot` and `fire`, a splash where it
    /// meets the sea.
    pub fn update(
        &mut self,
        dt: f64,
        wind: DVec3,
        soot: &mut ParticlePool,
        fire: &mut ParticlePool,
    ) {
        if dt <= 0.0 {
            return;
        }
        for p in &mut self.pieces {
            if p.age >= p.life {
                continue;
            }
            p.age += dt;
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            let step = dt.min(p.age);
            let start = p.position;
            // Linear drag with gravity, integrated exactly over the step.
            let decay = (-p.drag * step).exp();
            let travel = (1.0 - decay) / p.drag;
            let terminal = 9.81 / p.drag;
            p.position.x += p.velocity.x * travel;
            p.position.z += p.velocity.z * travel;
            p.position.y += (p.velocity.y + terminal) * travel - terminal * step;
            p.velocity.x *= decay;
            p.velocity.z *= decay;
            p.velocity.y = (p.velocity.y + terminal) * decay - terminal;
            let glow = p.glow();
            // A burning fragment: a short flame licking off it.
            if glow > 0.25 {
                let flame = fire.emit(p.position, None);
                flame.velocity = p.velocity * 0.15;
                flame.size = (p.extents.x * 1.6 + 0.12) * (0.6 + glow * 0.6);
                flame.stretch = 1.0 + (p.velocity.length() * 0.05).min(3.0);
                flame.align = Align::Velocity;
                flame.life = 0.12 + glow * 0.12;
                flame.growth = -flame.size * 2.0;
                flame.color = Rgb::new(2.6, 1.1, 0.32).scale(glow);
                flame.opacity = 0.8;
            }
            // Smoke left along the path, thinning out as the trail ages.
            if p.trail > 0.0 && p.age < p.trail_life {
                let fade = 1.0 - p.age / p.trail_life;
                p.credit += start.distance(p.position) * p.trail * fade.max(0.5);
                let mut at: f64 = 0.0;
                while p.credit >= 1.0 {
                    p.credit -= 1.0;
                    at += 0.37;
                    let along = start.lerp(p.position, at % 1.0);
                    let puff = soot.emit(along, p.source.as_ref());
                    puff.size = p.trail_size * (0.6 + 0.4 * fade);
                    puff.growth = p.trail_size * 1.2;
                    puff.growth_decay = 1.4;
                    puff.diffusion = 0.12;
                    puff.life = 1.2 + 1.8 * fade * p.trail_strength.min(1.5);
                    puff.fade_in = 0.05;
                    puff.velocity = p.velocity * 0.04 + DVec3::new(0.0, 0.5, 0.0);
                    puff.drag = 0.8;
                    puff.gravity = -0.3;
                    puff.wind = 0.9;
                    puff.density = 1.3 + 0.5 * p.trail_strength.min(1.5);
                    puff.heat = (glow * 0.8).min(0.7);
                    puff.cooling = 1.5;
                    puff.opacity = (0.22 + 0.3 * fade) * p.trail_strength.clamp(0.4, 1.5);
                    puff.color = p.trail_colour.scale(0.9 + 0.2 * fade);
                    // Drawn out along its flight, so the trail is a streak rather than a string of beads.
                    let way = p.velocity.normalize_or(DVec3::Y);
                    puff.volume_aspect = 1.0 + (p.velocity.length() / 40.0).min(0.8);
                    puff.volume_yaw = way.z.atan2(way.x);
                    puff.volume_axis_y = way.y.clamp(-1.0, 1.0);
                }
            }
            if p.position.y < SEA_Y && start.y >= SEA_Y {
                // Into the sea: a puff of spray sized to the piece and its fall, then gone.
                // A narrow column, taller the harder it came down.
                let fall = (-p.velocity.y).max(0.0);
                let size = (p.extents.x * 1.1 + 0.15) * (0.6 + (fall / 30.0).min(1.0));
                let at = DVec3::new(p.position.x, SEA_Y + size, p.position.z);
                let spray = soot.emit(at, None);
                spray.size = size;
                spray.growth = size * 1.4;
                spray.growth_decay = 2.2;
                spray.diffusion = 0.05;
                spray.life = 0.9 + size * 0.4;
                spray.velocity = DVec3::new(0.0, 2.0 + fall * 0.15, 0.0);
                spray.drag = 2.4;
                spray.gravity = 3.0;
                spray.wind = 0.5;
                spray.density = 1.3;
                spray.opacity = 0.75;
                spray.volume_aspect = 2.2;
                spray.volume_axis_y = 1.0;
                spray.color = Rgb::hex(0xe4e9ee);
                self.splashes += 1;
                // A plank floats where it came down.
                if p.shape == crate::flotsam::shape::PLANK {
                    let at = DVec3::new(p.position.x, 0.0, p.position.z);
                    self.landed
                        .push((at, p.extents.x, p.seed, p.source.clone()));
                }
                p.age = p.life;
            }
        }
        let _ = wind;
    }

    /// The pieces in `camera`'s view as instances.
    pub fn publish_into(&self, camera: &CameraView, out: &mut Vec<DebrisInstance>) {
        let planes = camera.frustum();
        self.publish_where(out, |p| {
            let radius = p.extents.x * 1.5;
            !planes
                .iter()
                .any(|plane| plane.distance(camera.drawn(p.position)) < -radius)
        });
    }

    /// Every piece in flight as instances, in view or not: the sun's shadow casters (a piece off the screen can
    /// shade what is on it).
    pub fn publish_casters_into(&self, out: &mut Vec<DebrisInstance>) {
        self.publish_where(out, |_| true);
    }

    fn publish_where(&self, out: &mut Vec<DebrisInstance>, keep: impl Fn(&Piece) -> bool) {
        out.clear();
        for p in &self.pieces {
            if p.age < 0.0 || p.age >= p.life || !keep(p) {
                continue;
            }
            let turn = DQuat::from_axis_angle(p.axis, p.phase + p.spin * p.age);
            out.push(DebrisInstance {
                position: [
                    p.position.x as f32,
                    p.position.y as f32,
                    p.position.z as f32,
                    p.seed,
                ],
                rotation: [turn.x as f32, turn.y as f32, turn.z as f32, turn.w as f32],
                extents: [
                    p.extents.x as f32,
                    p.extents.y as f32,
                    p.extents.z as f32,
                    p.shape,
                ],
                color: [
                    p.albedo.r as f32,
                    p.albedo.g as f32,
                    p.albedo.b as f32,
                    p.glow() as f32,
                ],
            });
        }
    }

    /// A digest of every piece in flight, for determinism tests.
    pub fn digest(&self) -> f64 {
        self.pieces
            .iter()
            .filter(|p| p.age >= 0.0 && p.age < p.life)
            .map(|p| p.position.x * 3.0 + p.position.y * 5.0 + p.position.z * 7.0 + p.age)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::PoolKind;

    fn burst(count: usize) -> DebrisBurst {
        DebrisBurst {
            at: DVec3::new(0.0, 8.0, 0.0),
            axis: DVec3::new(1.0, 0.3, 0.0),
            spread: 0.7,
            count,
            speed: (15.0, 40.0),
            size: (0.1, 0.6),
            hot: 0.5,
            smoking: 0.6,
            trail: 0.8,
            ..Default::default()
        }
    }

    #[test]
    fn pieces_fly_out_fall_and_splash() {
        let mut debris = Debris::new();
        let mut soot = ParticlePool::new(2048, PoolKind::Gas);
        let mut fire = ParticlePool::new(1024, PoolKind::Additive);
        debris.throw(&burst(40), &mut Lcg::new(7), None);
        assert_eq!(debris.alive(), 40);
        for _ in 0..30 {
            soot.advance(1.0 / 60.0, DVec3::ZERO);
            fire.advance(1.0 / 60.0, DVec3::ZERO);
            debris.update(1.0 / 60.0, DVec3::ZERO, &mut soot, &mut fire);
        }
        // Thrown along +X: after half a second every piece is out that way, trailing smoke and flame.
        assert!(
            debris.pieces.iter().all(|p| p.position.x > 0.5),
            "pieces went the wrong way"
        );
        assert!(soot.alive() > 0 && fire.alive() > 0);
        for _ in 0..600 {
            soot.advance(1.0 / 60.0, DVec3::ZERO);
            fire.advance(1.0 / 60.0, DVec3::ZERO);
            debris.update(1.0 / 60.0, DVec3::ZERO, &mut soot, &mut fire);
        }
        assert_eq!(debris.alive(), 0);
        assert!(debris.splashes() > 20, "{} splashes", debris.splashes());
    }

    #[test]
    fn a_throw_replays_identically_and_thins_evenly() {
        let mut a = Debris::new();
        let mut b = Debris::new();
        a.throw(&burst(30), &mut Lcg::new(3), None);
        b.throw(&burst(30), &mut Lcg::new(3), None);
        assert_eq!(a.digest(), b.digest());
        let mut thin = Debris::new();
        thin.density = 0.5;
        thin.throw(&burst(30), &mut Lcg::new(3), None);
        assert_eq!(thin.alive(), 15);
    }

    #[test]
    fn delayed_pieces_wait_unseen() {
        let mut debris = Debris::new();
        debris.throw(
            &DebrisBurst {
                delay: 0.2,
                ..burst(5)
            },
            &mut Lcg::new(1),
            None,
        );
        let eye = naval_render::Camera::look_at(
            glam::Vec3::new(-40.0, 10.0, 0.0),
            glam::Vec3::new(0.0, 8.0, 0.0),
        );
        let camera =
            CameraView::from_uniforms(&eye.uniforms(1280, 720, naval_render::FrameTime::default()));
        let mut out = Vec::new();
        debris.publish_into(&camera, &mut out);
        assert!(out.is_empty());
        let mut soot = ParticlePool::new(64, PoolKind::Gas);
        let mut fire = ParticlePool::new(64, PoolKind::Additive);
        debris.update(0.25, DVec3::ZERO, &mut soot, &mut fire);
        debris.publish_into(&camera, &mut out);
        assert_eq!(out.len(), 5);
    }
}
