//! Fixed-capacity particle pools: the web client's `EffectParticlePool` (`EffectParticles.ts`), the one pool both
//! effect crates draw from (the combat effects' gas and sprites here, the water effects' drops and billows in
//! `naval-water-effects`, [`PoolKind::Water`]).
//!
//! Particles live in world space and move analytically (drag, gravity and wind integrated exactly per step,
//! so a spray's apex does not depend on the frame rate). A full pool gives up the particle nearest the end of
//! its life. `publish` culls to the frustum, sorts back to front (alpha batches) and writes GPU instances.

use std::sync::Arc;

use glam::{DQuat, DVec3};

use crate::camera::CameraView;
use crate::instances::{
    GAS_FIREBALL, GAS_FLAK, GAS_PROPELLANT, GAS_SMOKE, GasPuff, Sprite, WaterSprite,
};
use crate::math::{Rgb, smooth};
use crate::random::golden_seed;

/// How a sprite is turned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Faces the camera, rolled by `angle`.
    #[default]
    Billboard,
    /// Stretched along its velocity as seen on screen.
    Velocity,
    /// Like `Velocity`, but the smear grows with speed so a drop rounds out at its apex.
    Streak,
    /// Lies flat on the water.
    Water,
}

/// Speed (m/s) at which a `Streak` particle reaches its full smear.
const STREAK_SPEED: f64 = 20.0;
/// How much wider than its sphere a burst's soot draws its quad ([`EffectParticle::flak`]): a sphere's outline seen
/// from twice its radius off is 1.15 times as wide as the quad that shows a puff's angular size, and the puff clears
/// before the lens is nearer than that (`gas_sprite.wgsl`'s `near`).
const FLAK_REACH: f64 = 1.2;

/// A ship (or other source) id: optics on that hull leave its particles out.
pub type SourceId = Arc<str>;

/// One particle, with the web's field names in snake case.
#[derive(Clone, Debug)]
pub struct EffectParticle {
    pub source_id: Option<SourceId>,
    pub position: DVec3,
    pub velocity: DVec3,
    pub color: Rgb,
    pub age: f64,
    pub life: f64,
    pub size: f64,
    pub growth: f64,
    pub growth_decay: f64,
    pub diffusion: f64,
    pub heat: f64,
    pub cooling: f64,
    pub density: f64,
    /// Time scale for thinning after cooling; zero keeps the recipe's density.
    pub dissipation_time: f64,
    /// Major/minor radius ratio and fixed world direction of an elongated volume.
    pub volume_aspect: f64,
    pub volume_yaw: f64,
    pub volume_axis_y: f64,
    pub seed: f64,
    pub opacity: f64,
    pub drag: f64,
    pub gravity: f64,
    pub wind: f64,
    pub angle: f64,
    pub spin: f64,
    pub stretch: f64,
    pub fade_in: f64,
    pub align: Align,
    pub waterline: bool,
    pub surface_y: f64,
    /// Trial `flipbook`: the clip of a baked simulation a gas puff plays over its life (`flipbook::clip`); 0, a
    /// still puff of the atlas.
    pub flipbook: f64,
    /// A gun's propellant gas: its flame is drawn as the propellant's (`gas_sprite.wgsl`, `propellant_flame`), and
    /// `heat` is the share of its burning still to come (about 1.3 out of the bore) rather than an explosive's heat.
    pub propellant: bool,
    /// A gun's fireball (with `propellant`): its gas burns white-hot through its first tenth of a second and leaves
    /// fresh soot, dark until it mixes with the air (`gas_sprite.wgsl`, `GAS_FIREBALL`).
    pub fireball: bool,
    /// The body this particle belongs to (the pool's `tag` when it was emitted; 0 for none): the volume trial marches a
    /// tagged body in place of its sprites (`volume.rs`).
    pub tag: u32,
    /// The warp trial: the way the particle was thrown (`warp::axis`), kept from its first step; zero until then.
    pub roll_axis: DVec3,
    /// A puff of a stream (a jet out of a funnel): its gas keeps its soot or steam as it widens, so the optical depth
    /// through its middle falls as its width grows (`density` × first size / size, as a funnel's plume thins,
    /// `funnel.rs`' `puff_optical`). Off, a puff keeps its density however it grows (a burst's billows).
    pub thins: bool,
    /// A heavy AA round's burst of soot (`flak.rs`): one lumpy cloud marched through its sphere (`GAS_FLAK`), whose
    /// soot spreads through it as it grows, so the optical depth across it falls as the square of its width.
    pub flak: bool,
    distance: f64,
}

impl Default for EffectParticle {
    fn default() -> Self {
        Self {
            source_id: None,
            position: DVec3::ZERO,
            velocity: DVec3::ZERO,
            color: Rgb::WHITE,
            age: 0.0,
            life: 0.0,
            size: 1.0,
            growth: 0.0,
            growth_decay: 0.0,
            diffusion: 0.0,
            heat: 0.0,
            cooling: 1.0,
            density: 4.0,
            dissipation_time: 0.0,
            volume_aspect: 1.0,
            volume_yaw: 0.0,
            volume_axis_y: 0.0,
            seed: 0.0,
            opacity: 1.0,
            drag: 0.0,
            gravity: 0.0,
            wind: 0.0,
            angle: 0.0,
            spin: 0.0,
            stretch: 1.0,
            fade_in: 0.0,
            align: Align::Billboard,
            waterline: false,
            surface_y: 0.0,
            flipbook: 0.0,
            propellant: false,
            fireball: false,
            tag: 0,
            roll_axis: DVec3::ZERO,
            thins: false,
            flak: false,
            distance: 0.0,
        }
    }
}

impl EffectParticle {
    fn alive(&self) -> bool {
        self.age < self.life
    }

    /// Size at the current age: the decaying expansion plus steady diffusion.
    pub fn current_size(&self) -> f64 {
        let expansion = if self.growth_decay > 0.0 {
            (1.0 - (-self.age * self.growth_decay).exp()) / self.growth_decay
        } else {
            self.age
        };
        (self.size + self.growth * expansion + self.diffusion * self.age).max(0.01)
    }

    /// Its gas's density at the current age: a stream's thinning as it widens ([`EffectParticle::thins`]), the same
    /// for its drawing and its shadow.
    pub fn density_now(&self) -> f64 {
        if self.flak {
            self.density * (self.size / self.current_size().max(1e-3)).min(1.0).powi(2)
        } else if self.thins {
            self.density * (self.size / self.current_size().max(1e-3)).min(1.0)
        } else {
            self.density
        }
    }

    /// The fade by life and fade-in the pool applies to opacity. A burst's soot fades over the last half of its life
    /// only: its spreading thins it before then ([`EffectParticle::density_now`]).
    fn fade(&self) -> f64 {
        let t = self.age / self.life;
        let from = if self.flak { 0.5 } else { 0.18 };
        (1.0 - smooth((t - from) / (1.0 - from)))
            * if self.fade_in > 0.0 {
                smooth(self.age / self.fade_in)
            } else {
                1.0
            }
    }
}

/// How a slot ranks for reuse: a spent particle first, then the largest share of its life spent.
fn claim_key(p: &EffectParticle) -> f64 {
    if p.age >= p.life {
        return f64::INFINITY;
    }
    let fraction = p.age / p.life;
    if fraction.is_nan() {
        f64::NEG_INFINITY
    } else if fraction == f64::INFINITY {
        f64::MAX
    } else {
        fraction
    }
}

/// What kind of batch a pool publishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoolKind {
    /// Textured sprites blended over the scene, sorted back to front.
    Sprites,
    /// Additive sprites: no sort.
    Additive,
    /// Gas puffs relighting the atlas, sorted back to front.
    Gas,
    /// The water effects' spray ([`ParticlePool::publish_water_into`], `naval-water-effects`' `sprite.wgsl`), sorted
    /// back to front. Its particles take the wind as it is given, not as it blows at their height, and roll no warp.
    /// `fine` leaves out sprites under a pixel or out of view (drops); `offscreen` those whose bounding sphere leaves the
    /// frustum (billows).
    Water { fine: bool, offscreen: bool },
}

/// One instance batch per material, fixed storage, back-to-front sorting.
#[derive(Clone, Debug)]
pub struct ParticlePool {
    pub capacity: usize,
    pub kind: PoolKind,
    particles: Vec<EffectParticle>,
    /// Emitted slots that have not expired, including delayed starts. Slot storage and claim order stay fixed.
    live: Vec<usize>,
    indexed: Vec<bool>,
    scratch: EffectParticle,
    cursor: usize,
    emitted: u64,
    /// Fraction of requested particles that enter the pool (the combat effects setting).
    pub density: f64,
    density_phase: f64,
    drawn: usize,
    /// The last publish's visible particles, back to front: working room kept between frames.
    order: Vec<usize>,
    /// Stamped on every particle emitted while it is set (0: none), and the tag whose particles `publish` leaves out
    /// (0: none) because the volume trial draws that body (`volume.rs`).
    pub tag: u32,
    pub hidden_tag: u32,
}

impl ParticlePool {
    pub fn new(capacity: usize, kind: PoolKind) -> Self {
        Self {
            capacity,
            kind,
            particles: vec![EffectParticle::default(); capacity],
            live: Vec::with_capacity(capacity),
            indexed: vec![false; capacity],
            scratch: EffectParticle::default(),
            cursor: 0,
            emitted: 0,
            density: 1.0,
            density_phase: 0.0,
            drawn: 0,
            order: Vec::new(),
            tag: 0,
            hidden_tag: 0,
        }
    }

    /// A reset particle at `position`. Thinned requests (density below 1) get a detached scratch particle, so
    /// callers keep one emission path at every density.
    pub fn emit(&mut self, position: DVec3, source: Option<&SourceId>) -> &mut EffectParticle {
        if self.density < 1.0 {
            // A fixed-stride sequence thins every effect evenly, without a random stream.
            self.density_phase += self.density;
            if self.density_phase < 1.0 {
                self.scratch = EffectParticle::default();
                return &mut self.scratch;
            }
            self.density_phase -= 1.0;
        }
        let index = self.claim();
        if !self.indexed[index] {
            self.live.push(index);
            self.indexed[index] = true;
        }
        self.emitted += 1;
        let seed = golden_seed(self.emitted);
        let tag = self.tag;
        let p = &mut self.particles[index];
        *p = EffectParticle {
            position,
            life: 1.0,
            seed,
            source_id: source.cloned(),
            tag,
            ..EffectParticle::default()
        };
        p
    }

    /// The next free slot in ring order. A full pool gives up the particle nearest the end of its life, so a
    /// burst of short gunfire never cuts a long-lived plume short. Probing the ring past 32 slots finds what the
    /// web's claim tree finds: the first spent slot in ring order, else the first with the largest share of its
    /// life spent.
    fn claim(&mut self) -> usize {
        let capacity = self.capacity;
        let mut chosen = self.cursor % capacity;
        let mut spent = f64::NEG_INFINITY;
        let mut free = false;
        let probes = capacity.min(32);
        for probe in 0..probes {
            let index = (self.cursor + probe) % capacity;
            let p = &self.particles[index];
            if p.age >= p.life {
                chosen = index;
                free = true;
                break;
            }
            let fraction = p.age / p.life;
            if fraction > spent {
                spent = fraction;
                chosen = index;
            }
        }
        if !free && probes < capacity {
            let start = self.cursor % capacity;
            let mut best = f64::NEG_INFINITY;
            let mut found = start;
            for offset in 0..capacity {
                let index = (start + offset) % capacity;
                let key = claim_key(&self.particles[index]);
                if key > best {
                    best = key;
                    found = index;
                }
            }
            chosen = found;
        }
        self.cursor = chosen + 1;
        chosen
    }

    /// Age and move existing particles; call before this frame's emissions.
    pub fn advance(&mut self, dt: f64, wind: DVec3) {
        let water = matches!(self.kind, PoolKind::Water { .. });
        for &index in &self.live {
            let p = &mut self.particles[index];
            if p.age >= p.life {
                continue;
            }
            let previous_age = p.age;
            p.age += dt;
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            // A delayed particle starts partway through the frame.
            let step = dt.min(p.age);
            if step > 0.0 {
                if !water && p.roll_axis == DVec3::ZERO {
                    p.roll_axis = crate::warp::axis(p.velocity, p.gravity, p.drag);
                }
                let decay = (-p.drag * step).exp();
                let dragged = p.drag > 0.0001;
                let travel = if dragged {
                    (1.0 - decay) / p.drag
                } else {
                    step
                };
                // Gravity with drag integrated analytically: frame-rate independent. The wind at its height.
                let terminal = if dragged { p.gravity / p.drag } else { 0.0 };
                let carried = if water {
                    p.wind
                } else {
                    p.wind * crate::combat::wind_share(p.position.y)
                };
                p.position.x += p.velocity.x * travel + wind.x * carried * step;
                p.position.z += p.velocity.z * travel + wind.z * carried * step;
                p.position.y += if dragged {
                    (p.velocity.y + terminal) * travel - terminal * step
                } else {
                    p.velocity.y * step - 0.5 * p.gravity * step * step
                };
                p.velocity.x *= decay;
                p.velocity.z *= decay;
                p.velocity.y = if dragged {
                    (p.velocity.y + terminal) * decay - terminal
                } else {
                    p.velocity.y - p.gravity * step
                };
                p.angle += p.spin * step;
                if p.waterline && p.position.y < p.surface_y + 0.2 && previous_age >= 0.0 {
                    p.age = p.life;
                }
            }
        }
        self.live.retain(|&index| {
            let expired = self.particles[index].age >= self.particles[index].life;
            self.indexed[index] = !expired;
            !expired
        });
    }

    /// The live particles to draw, back to front for blended batches: frustum culled (a gas puff's sprite is
    /// pulled toward the camera, so only the four side planes reject it), faded puffs dropped, and `hidden`'s
    /// particles left out (they still age and drift).
    /// The visible particles into `self.order`, back to front where the pool sorts.
    fn visible(&mut self, camera: &CameraView, hidden: Option<&str>) {
        let planes = camera.frustum();
        let gas = self.kind == PoolKind::Gas;
        let (checked, fine) = match self.kind {
            PoolKind::Gas => (4, false),
            PoolKind::Sprites | PoolKind::Additive => (6, false),
            PoolKind::Water { fine, offscreen } => (if offscreen { 6 } else { 0 }, fine),
        };
        let (p0, p5) = camera.lens();
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        for &index in &self.live {
            let p = &mut self.particles[index];
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            if let (Some(hidden), Some(source)) = (hidden, &p.source_id)
                && &**source == hidden
            {
                continue;
            }
            if gas && p.opacity * (1.0 - smooth((p.age / p.life - 0.18) / 0.82)) < 0.004 {
                continue;
            }
            if p.tag != 0 && p.tag == self.hidden_tag {
                continue;
            }
            if checked > 0 {
                // Linear growth bounds the decaying expansion; the full rotated, stretched quad is inside.
                let size =
                    (p.size + p.growth.max(0.0) * p.age + p.diffusion.max(0.0) * p.age).max(0.01);
                let reach = -(size * 0.5 * 1f64.hypot(p.stretch));
                let drawn = camera.drawn(p.position);
                if planes[..checked]
                    .iter()
                    .any(|plane| plane.distance(drawn) < reach)
                {
                    continue;
                }
            }
            if fine {
                // Fine water: out of view by its own radius, or under a pixel.
                let v = camera.to_view(p.position);
                let depth = -v.z;
                let radius = p.size + p.growth * p.age;
                if depth + radius < 0.1
                    || v.x.abs() > depth.max(0.0) / p0 + radius
                    || v.y.abs() > depth.max(0.0) / p5 + radius
                    || radius * p5 / depth.max(1.0) < 0.0007
                {
                    continue;
                }
            }
            p.distance = (p.position - camera.position).length_squared();
            order.push(index);
        }
        if self.kind != PoolKind::Additive {
            // Back to front, stable: equal distances keep slot order (the slot breaks the tie, so an unstable sort,
            // which needs no buffer, gives the stable order).
            order.sort_unstable_by(|&a, &b| {
                self.particles[b]
                    .distance
                    .total_cmp(&self.particles[a].distance)
                    .then(a.cmp(&b))
            });
        } else {
            // Additive blending still follows the fixed slots' order, independently of their emission order.
            order.sort_unstable();
        }
        self.drawn = order.len();
        self.order = order;
    }

    /// Gas puffs for a volume batch, as `publish` fills `effectSphere`, `effectVolume`, `effectTint` and
    /// `effectProgress`, with each quad placed half its radius nearer the camera and scaled to keep its angular
    /// size where it is drawn.
    pub fn publish_gas(&mut self, camera: &CameraView, hidden: Option<&str>) -> Vec<GasPuff> {
        let mut puffs = Vec::new();
        self.publish_gas_into(camera, hidden, &mut puffs);
        puffs
    }

    /// [`ParticlePool::publish_gas`] appended to `out` (a list kept between frames allocates nothing).
    pub fn publish_gas_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        out: &mut Vec<GasPuff>,
    ) {
        self.visible(camera, hidden);
        let order = std::mem::take(&mut self.order);
        out.extend(order.iter().map(|&index| {
            let p = &self.particles[index];
            let size = p.current_size();
            let t = p.age / p.life;
            let distance = p.distance.sqrt();
            let radius = size / 2.0;
            // A burst's soot is marched through its sphere: its quad reaches past the sphere's outline as the lens nears it.
            let reach = if p.flak { size * FLAK_REACH } else { size };
            let (placed, edge) = place_facing(camera, p.position, distance, radius, reach);
            let heat = p.heat * (1.0 - smooth(p.age / p.cooling));
            let dispersal = if p.dissipation_time > 0.0 {
                (p.age - p.cooling).max(0.0) / p.dissipation_time
            } else {
                0.0
            };
            GasPuff {
                sphere: [
                    p.position.x as f32,
                    p.position.y as f32,
                    p.position.z as f32,
                    radius as f32,
                ],
                state: [
                    p.age as f32,
                    p.seed as f32,
                    heat as f32,
                    p.density_now() as f32,
                ],
                tint: [
                    p.color.r as f32,
                    p.color.g as f32,
                    p.color.b as f32,
                    (1.0 + (p.volume_aspect - 1.0) * (-p.age * 0.7).exp()) as f32,
                ],
                evolution: [
                    t as f32,
                    (1.0 - (-dispersal * dispersal).exp()) as f32,
                    p.volume_yaw as f32,
                    p.volume_axis_y as f32,
                ],
                quad: [
                    placed.x as f32,
                    placed.y as f32,
                    placed.z as f32,
                    edge as f32,
                ],
                extra: [
                    (p.opacity * p.fade()) as f32,
                    0.0,
                    p.flipbook as f32,
                    if p.flak {
                        GAS_FLAK
                    } else if p.fireball {
                        GAS_FIREBALL
                    } else if p.propellant {
                        GAS_PROPELLANT
                    } else {
                        GAS_SMOKE
                    },
                ],
                warp: crate::warp::gas(
                    crate::warp::roll(size, p.size, p.age),
                    if p.roll_axis == DVec3::ZERO {
                        crate::warp::axis(p.velocity, p.gravity, p.drag)
                    } else {
                        p.roll_axis
                    },
                ),
            }
        }));
        self.order = order;
    }

    /// Textured sprites: billboards rolled by `angle`, velocity-aligned streaks, or quads lying on the water.
    pub fn publish_sprites(&mut self, camera: &CameraView, hidden: Option<&str>) -> Vec<Sprite> {
        let mut sprites = Vec::new();
        self.publish_sprites_into(camera, hidden, &mut sprites);
        sprites
    }

    /// [`ParticlePool::publish_sprites`] appended to `out` (a list kept between frames allocates nothing).
    pub fn publish_sprites_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        out: &mut Vec<Sprite>,
    ) {
        self.visible(camera, hidden);
        let order = std::mem::take(&mut self.order);
        out.extend(order.iter().map(|&index| {
            let p = &self.particles[index];
            let size = p.current_size();
            let mut angle = p.angle;
            let mut stretch = p.stretch;
            if matches!(p.align, Align::Velocity | Align::Streak) {
                let v = camera.to_camera_direction(p.velocity);
                angle = (-v.x).atan2(v.y);
                let speed = p.velocity.length().max(0.01);
                let across = v.x.hypot(v.y) / speed;
                // An exposure smear grows with speed and foreshortens; the drop itself stays round.
                if p.align == Align::Streak {
                    stretch = 1.0 + (stretch - 1.0) * (speed / STREAK_SPEED).min(1.0) * across;
                } else {
                    stretch *= across.max(0.35);
                }
            }
            Sprite {
                position: [
                    p.position.x as f32,
                    p.position.y as f32,
                    p.position.z as f32,
                    angle as f32,
                ],
                size: [
                    size as f32,
                    (size * stretch) as f32,
                    if p.align == Align::Water { 1.0 } else { 0.0 },
                    crate::warp::sprite(crate::warp::roll(size, p.size, p.age), p.seed),
                ],
                color: [
                    p.color.r as f32,
                    p.color.g as f32,
                    p.color.b as f32,
                    (p.opacity * p.fade()) as f32,
                ],
            }
        }));
        self.order = order;
    }

    /// The water effects' sprites ([`PoolKind::Water`]), appended to `out` back to front: each quad built on the CPU
    /// (billboards rolled by `angle` in the camera's turn, velocity-aligned streaks, or lying on the water), as
    /// `naval-water-effects`' `sprite.wgsl` draws them.
    pub fn publish_water_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        out: &mut Vec<WaterSprite>,
    ) {
        self.visible(camera, hidden);
        let order = std::mem::take(&mut self.order);
        let camera_rotation = camera.rotation();
        let inverse = camera_rotation.inverse();
        let water = DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2);
        out.extend(order.iter().map(|&index| {
            let p = &self.particles[index];
            let size = p.current_size();
            let mut angle = p.angle;
            let mut stretch = p.stretch;
            let mut base = camera_rotation;
            match p.align {
                Align::Water => base = water,
                Align::Velocity | Align::Streak => {
                    let d = inverse * p.velocity;
                    angle = (-d.x).atan2(d.y);
                    let speed = p.velocity.length().max(0.01);
                    let across = d.x.hypot(d.y) / speed;
                    if p.align == Align::Streak {
                        stretch = 1.0 + (stretch - 1.0) * (speed / STREAK_SPEED).min(1.0) * across;
                    } else {
                        stretch *= across.max(0.35);
                    }
                }
                Align::Billboard => {}
            }
            let orientation = base * DQuat::from_rotation_z(angle);
            let axis_u = orientation * DVec3::new(size, 0.0, 0.0);
            let axis_v = orientation * DVec3::new(0.0, size * stretch, 0.0);
            let fade_in = if p.fade_in > 0.0 {
                smooth(p.age / p.fade_in)
            } else {
                1.0
            };
            // The water's fade, as its port wrote it: over the last 82 % of its life.
            let opacity = p.opacity * (1.0 - smooth((p.age / p.life - 0.18) / 0.82)) * fade_in;
            WaterSprite {
                center: p.position.as_vec3().to_array(),
                opacity: opacity as f32,
                axis_u: axis_u.as_vec3().to_array(),
                surface: p.surface_y as f32,
                axis_v: axis_v.as_vec3().to_array(),
                aged: (p.age / p.life.max(1e-6)).clamp(0.0, 1.0) as f32,
                color: p.color.to_f32(),
                seed: ((p.life * 12.9898).sin() * 43758.5453).rem_euclid(1.0) as f32,
            }
        }));
        self.order = order;
    }

    /// Particles drawn by the last publish.
    pub fn drawn(&self) -> usize {
        self.drawn
    }

    /// Particles still alive, drawn or not.
    pub fn alive(&self) -> usize {
        self.live
            .iter()
            .filter(|&&index| self.particles[index].alive())
            .count()
    }

    /// Every slot, for tests and diagnostics.
    pub fn particles(&self) -> &[EffectParticle] {
        &self.particles
    }

    pub(crate) fn translate(&mut self, offset: DVec3) {
        for p in &mut self.particles {
            p.position += offset;
        }
    }

    /// Carries the pool into a scene laid on another chart (a theatre's re-lay): each particle's place through its
    /// place, its velocity and the way it was thrown (`roll_axis`, a unit direction) through the two charts' Jacobians
    /// where it is, and the level axis its volume is drawn along (`volume_yaw`, from +x toward +z) likewise.
    pub(crate) fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        for p in &mut self.particles {
            let at = p.position.to_array();
            p.velocity = DVec3::from_array(relay.vector(at, p.velocity.to_array()));
            if p.roll_axis != DVec3::ZERO {
                p.roll_axis = DVec3::from_array(relay.vector(at, p.roll_axis.to_array()))
                    .normalize_or(p.roll_axis);
            }
            let [x, z] = relay.direction(at, [p.volume_yaw.cos(), p.volume_yaw.sin()]);
            p.volume_yaw = z.atan2(x);
            p.position = DVec3::from_array(relay.point(at));
        }
    }

    /// Carries the pool's smoke with the blast waves that passed it this step ([`crate::shock::Shove`]): what was
    /// in the air before a blast went off (its own puffs are thrown by its recipe), and never under the sea.
    pub(crate) fn shove(&mut self, shoves: &[crate::shock::Shove]) {
        for &index in &self.live {
            let p = &mut self.particles[index];
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            for shove in shoves {
                if p.age <= shove.age {
                    continue;
                }
                let by = shove.at(p.position);
                if by != DVec3::ZERO {
                    let floor = p.position.y.min(0.5);
                    p.position += by;
                    p.position.y = p.position.y.max(floor);
                }
            }
        }
    }

    pub fn reset(&mut self) {
        for p in &mut self.particles {
            p.age = 0.0;
            p.life = 0.0;
        }
        self.cursor = 0;
        self.live.clear();
        self.indexed.fill(false);
        self.emitted = 0;
        self.drawn = 0;
        self.density_phase = 0.0;
    }
}

/// A camera-facing sprite for a puff at `center`, `distance` from the camera: drawn half its radius nearer the
/// camera (so the deck it rises from does not cut it through the middle) and scaled to keep its angular size.
/// Returns the placed centre and the quad's edge length.
pub fn place_facing(
    camera: &CameraView,
    center: DVec3,
    distance: f64,
    radius: f64,
    size: f64,
) -> (DVec3, f64) {
    let pull = if distance > 1e-6 {
        (radius * 0.5).min((distance - camera.near * 2.0).max(0.0))
    } else {
        0.0
    };
    let mut placed = center;
    if pull > 0.0 {
        let k = pull / distance;
        placed -= (center - camera.position) * k;
    }
    let edge = size
        * if distance > 1e-6 {
            (distance - pull) / distance
        } else {
            1.0
        };
    (placed, edge)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_wave_carries_the_smoke_that_was_in_the_air_before_it() {
        use crate::shock::Shove;
        let mut pool = ParticlePool::new(8, PoolKind::Gas);
        for height in [10.0, 0.2] {
            pool.emit(DVec3::new(28.0, height, 0.0), None).life = 60.0;
        }
        pool.advance(1.0, DVec3::ZERO);
        // A puff of the blast's own: younger than its front.
        pool.emit(DVec3::new(-28.0, 10.0, 0.0), None).life = 60.0;
        pool.advance(0.05, DVec3::ZERO);
        let places = |pool: &ParticlePool| -> Vec<DVec3> {
            pool.particles[..3].iter().map(|p| p.position).collect()
        };
        let before = places(&pool);
        let shove = Shove {
            centre: DVec3::new(0.0, 20.0, 0.0),
            before: 20.0,
            now: 40.0,
            pulse: 5.0,
            reach: 2.0,
            bore: DVec3::ZERO,
            age: 0.2,
        };
        pool.shove(&[shove]);
        let after = places(&pool);
        // Out from the blast by the pulse's whole reach.
        assert!(((after[0] - before[0]).length() - 2.0).abs() < 1e-9);
        assert!(after[0].x > before[0].x && after[0].y < before[0].y);
        // Low over the sea it is carried along it, not under it.
        assert!(after[1].x > before[1].x + 1.0);
        assert_eq!(after[1].y, before[1].y);
        // The blast's own puff is its recipe's to throw.
        assert_eq!(after[2], before[2]);
        // A front that has not reached the smoke moves nothing.
        pool.shove(&[Shove {
            before: 5.0,
            now: 10.0,
            ..shove
        }]);
        assert_eq!(places(&pool), after);
    }

    #[test]
    fn sparse_slots_match_dense_scan_through_reuse_and_reset() {
        let camera =
            naval_render::Camera::look_at(glam::Vec3::new(0.0, 10.0, 100.0), glam::Vec3::ZERO);
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, Default::default()));
        let sources: [SourceId; 2] = [Arc::from("one"), Arc::from("two")];
        let water = |fine, offscreen| PoolKind::Water { fine, offscreen };
        for kind in [
            PoolKind::Sprites,
            PoolKind::Gas,
            PoolKind::Additive,
            water(false, false),
            water(true, false),
            water(false, true),
        ] {
            let mut sparse = ParticlePool::new(64, kind);
            let mut dense = sparse.clone();
            for frame in 0..110 {
                if frame == 60 {
                    sparse.reset();
                    dense.reset();
                }
                let emits = if frame % 13 == 0 { 90 } else { 3 };
                for pool in [&mut sparse, &mut dense] {
                    pool.density = if frame % 11 == 0 { 0.5 } else { 1.0 };
                    for i in 0..emits {
                        let p =
                            pool.emit(DVec3::new((i % 5) as f64, 3.0, 0.0), Some(&sources[i % 2]));
                        p.life = 0.1 + (i % 9) as f64 * 0.11;
                        p.age = -((i % 3) as f64) * 0.03;
                        p.velocity = DVec3::new(2.0, 1.0, -0.5);
                        p.drag = (i % 2) as f64 * 0.4;
                        p.gravity = 9.81;
                        p.wind = 0.5;
                        p.spin = 0.2;
                        p.size = 3.0;
                        p.waterline = i % 4 == 0;
                        p.align = [
                            Align::Billboard,
                            Align::Velocity,
                            Align::Streak,
                            Align::Water,
                        ][i % 4];
                    }
                }
                // Replay the previous scheduler, which visited every fixed slot in order.
                dense.live.clear();
                dense.live.extend(0..dense.capacity);
                dense.indexed.fill(true);
                let dt = [0.0, 0.02, 0.13, 0.005][frame % 4];
                let wind = DVec3::new(3.0, 0.0, -1.0);
                sparse.advance(dt, wind);
                dense.advance(dt, wind);
                assert_eq!(sparse.cursor, dense.cursor);
                assert_eq!(sparse.alive(), dense.alive());
                for (a, b) in sparse.particles.iter().zip(&dense.particles) {
                    assert_eq!(
                        (a.position, a.velocity, a.age, a.life, a.angle, a.roll_axis),
                        (b.position, b.velocity, b.age, b.life, b.angle, b.roll_axis)
                    );
                }
                dense.live.clear();
                dense.live.extend(0..dense.capacity);
                dense.indexed.fill(true);
                let hidden = (frame % 7 == 0).then_some("one");
                assert_eq!(
                    sparse.publish_sprites(&view, hidden),
                    dense.publish_sprites(&view, hidden)
                );
                assert_eq!(
                    sparse.publish_gas(&view, hidden),
                    dense.publish_gas(&view, hidden)
                );
                let (mut a, mut b) = (Vec::new(), Vec::new());
                sparse.publish_water_into(&view, hidden, &mut a);
                dense.publish_water_into(&view, hidden, &mut b);
                assert_eq!(a, b);
                assert_eq!(sparse.live.capacity(), sparse.capacity);
            }
            sparse.advance(100.0, DVec3::ZERO);
            assert!(sparse.live.is_empty());
            assert!(sparse.indexed.iter().all(|&indexed| !indexed));
        }
    }

    #[test]
    fn drag_and_gravity_integrate_independently_of_the_step() {
        let run = |steps: usize| {
            let mut pool = ParticlePool::new(4, PoolKind::Sprites);
            let p = pool.emit(DVec3::ZERO, None);
            p.velocity = DVec3::new(10.0, 30.0, 0.0);
            p.drag = 0.6;
            p.gravity = 9.81;
            p.life = 10.0;
            for _ in 0..steps {
                pool.advance(2.0 / steps as f64, DVec3::ZERO);
            }
            pool.particles()[0].position
        };
        assert!((run(1) - run(120)).length() < 1e-9);
    }

    #[test]
    fn a_full_pool_gives_up_the_most_spent_particle() {
        let mut pool = ParticlePool::new(40, PoolKind::Sprites);
        for i in 0..40 {
            let p = pool.emit(DVec3::ZERO, None);
            p.life = 10.0 + i as f64;
        }
        pool.advance(5.0, DVec3::ZERO);
        // Slot 0 has spent half its life, the most of any; the next claim takes it.
        let p = pool.emit(DVec3::ONE, None);
        p.life = 99.0;
        assert_eq!(pool.particles()[0].life, 99.0);
    }

    #[test]
    fn falling_water_ends_at_the_waterline() {
        let mut pool = ParticlePool::new(
            2,
            PoolKind::Water {
                fine: true,
                offscreen: false,
            },
        );
        let p = pool.emit(DVec3::new(0.0, 1.0, 0.0), None);
        p.life = 10.0;
        p.gravity = 9.81;
        p.waterline = true;
        pool.advance(0.1, DVec3::ZERO);
        assert_eq!(pool.alive(), 1);
        pool.advance(1.0, DVec3::ZERO);
        assert_eq!(pool.alive(), 0);
    }

    #[test]
    fn thinning_skips_evenly() {
        let mut pool = ParticlePool::new(16, PoolKind::Sprites);
        pool.density = 0.5;
        for _ in 0..8 {
            pool.emit(DVec3::ZERO, None).life = 5.0;
        }
        assert_eq!(pool.alive(), 4);
    }

    #[test]
    fn a_propellants_gas_is_published_as_such() {
        use crate::instances::{GAS_FIREBALL, GAS_PROPELLANT, GAS_SMOKE};
        let camera =
            naval_render::Camera::look_at(glam::Vec3::new(0.0, 10.0, 100.0), glam::Vec3::ZERO);
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, Default::default()));
        let mut pool = ParticlePool::new(4, PoolKind::Gas);
        for (x, propellant, fireball) in
            [(0.0, true, false), (5.0, false, false), (10.0, true, true)]
        {
            let p = pool.emit(DVec3::new(x, 0.0, 0.0), None);
            p.life = 10.0;
            p.size = 10.0;
            p.propellant = propellant;
            p.fireball = fireball;
        }
        let mut kinds: Vec<_> = pool
            .publish_gas(&view, None)
            .iter()
            .map(|p| p.extra[3])
            .collect();
        kinds.sort_by(f32::total_cmp);
        assert_eq!(kinds, [GAS_SMOKE, GAS_PROPELLANT, GAS_FIREBALL]);
    }
}
