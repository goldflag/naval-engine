//! Fixed-capacity batches for ship fires: the web client's `FireBatch` (`FireBatch.ts`).
//!
//! Particles live in world space; vertical motion is analytic from the spawn point (a buoyant climb easing from
//! `rise` to `lift`), horizontal drift decays with drag and wind shear grows with the climb. Flames stand upright
//! and turn only in azimuth toward the camera (full billboards looking straight down); smoke puffs face the
//! camera; gas puffs face the camera from their centres, the frame the gas atlas is relit in.
//!
//! Trial `flipbook` ([`FireBatch::plume`]): a smoke batch's puffs publish as links of a simulated plume instead
//! (`link`), gas puffs laid along the column they climb, which `fires.rs` thins to a chain (`link_life`).

use std::sync::Arc;

use glam::DVec3;

use crate::camera::CameraView;
use crate::flipbook::{PLUME_ASPECT, PLUME_CLIMB, clip};
use crate::instances::{FireBasis, FireQuad, GAS_COLUMN, GasPuff};
use crate::math::{Rgb, smooth};
use crate::particles::{SourceId, place_facing};
use crate::random::golden_seed;

/// One particle of a ship fire: a flame tongue, a puff of its smoke column or a rolling flame body.
#[derive(Clone, Debug)]
pub struct FireParticle {
    pub source_id: Option<SourceId>,
    /// Spawn point; vertical motion is analytic from here.
    pub origin: DVec3,
    pub position: DVec3,
    /// Horizontal launch velocity (inherited hull motion and scatter), decaying with `drag`.
    pub drift: DVec3,
    pub drag: f64,
    pub age: f64,
    pub life: f64,
    /// Buoyant climb: `rise` m/s at the source easing over `rise_time` s to `lift` m/s.
    pub rise: f64,
    pub rise_time: f64,
    pub lift: f64,
    /// Metres over its origin where its climb levels off (a fire's column at the height its buoyancy takes it to,
    /// `fires.rs`' `column_top`), easing into it ([`FireParticle::climb_at`]); infinite for none.
    pub top: f64,
    /// Wind carried at the source; it grows to the full multiple by `shear_height` metres of climb, of the wind at its
    /// height ([`crate::combat::wind_share`]).
    pub shear: f64,
    pub shear_height: f64,
    /// Smoke radius or flame height in metres, and how it grows.
    pub size: f64,
    pub growth: f64,
    pub growth_decay: f64,
    pub diffusion: f64,
    /// Flame width as a fraction of its height.
    pub aspect: f64,
    pub seed: f64,
    pub opacity: f64,
    /// Thinning as the puff spreads: opacity × (initial ÷ current size)^thin, floored at 35 %.
    pub thin: f64,
    pub fade_in: f64,
    pub albedo: Rgb,
    /// Flame temperature, or smoke underglow from the fire below; cools over `cool` seconds.
    pub glow: f64,
    pub cool: f64,
    /// Gas only: burning gas (0 none … 2 white-hot) that cools to smoke over `cooling` seconds.
    pub heat: f64,
    pub cooling: f64,
    /// Trial `flipbook`, a link of a smoke column ([`FireBatch::plume`]): its source's velocity when it left (the
    /// column runs along the puff's motion as its source sees it), the way the column runs past it, and its place
    /// in the plume's loop.
    pub wake: DVec3,
    pub axis: DVec3,
    pub phase: f64,
    /// Trial `flipbook`, a link of a smoke column: how far off the column's line it lies, horizontal metres per metre
    /// along it, so the column's billows stand out of it at their own slants rather than stack into a tube.
    pub lean: DVec3,
    /// Trial `flipbook`, a puff of a smoke column drawn as a round billow among its links (`billow`) rather than as
    /// a link.
    pub billow: bool,
    /// Which of its ship's fires it came from (`fires.rs`, the source's place; 0 for none), so a column cut off at its
    /// fire can be let go ([`FireBatch::release`]).
    pub tag: u32,
}

/// The least time (s) a puff of a column cut off at its fire lives on ([`FireBatch::release`]).
pub const RELEASE_SECONDS: f64 = 4.0;

/// Trial `flipbook`: half the length of a column link's sprite, in puff radii. The tile's smoke fills about four
/// fifths of its width ([`PLUME_ASPECT`] of its length), which is the puff's.
pub(crate) const LINK_REACH: f64 = 1.0 / (0.8 * PLUME_ASPECT);

/// Trial `flipbook`: the radius of a column's billow's sprite, in puff radii: its smoke stands a little wider than a
/// link's, so it bulges out of the column.
pub(crate) const BILLOW_REACH: f64 = 1.4;

impl Default for FireParticle {
    fn default() -> Self {
        Self {
            source_id: None,
            origin: DVec3::ZERO,
            position: DVec3::ZERO,
            drift: DVec3::ZERO,
            drag: 0.0,
            age: 0.0,
            life: 0.0,
            rise: 0.0,
            rise_time: 1.0,
            lift: 0.0,
            top: f64::INFINITY,
            shear: 0.0,
            shear_height: 1.0,
            size: 1.0,
            growth: 0.0,
            growth_decay: 0.0,
            diffusion: 0.0,
            aspect: 1.0,
            seed: 0.0,
            opacity: 1.0,
            thin: 0.0,
            fade_in: 0.0,
            albedo: Rgb::WHITE,
            glow: 0.0,
            cool: 1.0,
            heat: 0.0,
            cooling: 1.0,
            wake: DVec3::ZERO,
            axis: DVec3::Y,
            phase: 0.0,
            lean: DVec3::ZERO,
            billow: false,
            tag: 0,
        }
    }
}

impl FireParticle {
    /// Radius of a smoke puff, or height of a flame tongue, at its current age.
    pub fn extent(&self) -> f64 {
        let expansion = if self.growth_decay > 0.0 {
            (1.0 - (-self.age * self.growth_decay).exp()) / self.growth_decay
        } else {
            self.age
        };
        (self.size + self.growth * expansion + self.diffusion * self.age).max(0.01)
    }

    /// How far it has climbed over its origin (m) at `age`, and how fast it climbs then (m/s): `rise` easing over
    /// `rise_time` to `lift`, levelling off at `top` as `top`·tanh(climb ÷ `top`), as near the fire as it climbs freely.
    pub fn climb_at(&self, age: f64) -> (f64, f64) {
        let easing = (-age / self.rise_time).exp();
        let free = self.rise * self.rise_time * (1.0 - easing) + self.lift * age;
        let rising = self.rise * easing + self.lift;
        if self.top.is_finite() && self.top > 0.0 {
            let level = (free / self.top).tanh();
            (self.top * level, rising * (1.0 - level * level))
        } else {
            (free, rising)
        }
    }

    /// The share of the wind ten metres up it drifts with `climb` metres over its origin: from 0.35 of `shear` at
    /// the source to all of it by `shear_height` of climb, of the wind at its height.
    pub fn carried_at(&self, climb: f64) -> f64 {
        self.shear
            * (0.35 + 0.65 * (climb.max(0.0) / self.shear_height).min(1.0))
            * crate::combat::wind_share(self.origin.y + climb)
    }
}

/// `Flame` tongues, `Smoke` puffs shaded as spheres by their own material, or `Gas` puffs relighting the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireMode {
    Flame,
    Smoke,
    Gas,
}

/// What a fire batch publishes for one frame.
#[derive(Clone, Debug, Default)]
pub struct FirePublish {
    pub quads: Vec<FireQuad>,
    pub gas: Vec<GasPuff>,
    pub basis: FireBasis,
    /// Screens' worth of quad area drawn (1 = the whole viewport once): the overdraw that sets the fill cost.
    pub fill: f64,
}

#[derive(Clone, Debug)]
pub struct FireBatch {
    pub capacity: usize,
    pub mode: FireMode,
    particles: Vec<FireParticle>,
    scratch: FireParticle,
    cursor: usize,
    emitted: u64,
    /// Fraction of requested emissions that enter the batch (the combat effects setting).
    pub density: f64,
    density_phase: f64,
    drawn: usize,
    /// The last publish's visible particles (index, squared distance, extent): room kept between frames.
    order: Vec<(usize, f64, f64)>,
    /// Sources whose particles `publish` leaves out because the volume trial draws them (`volume.rs`); empty otherwise.
    pub volume_sources: Vec<SourceId>,
    /// Trial `flipbook` (a `Smoke` batch): its puffs publish as links of a simulated plume (`flipbook::clip::PLUME`),
    /// gas puffs laid along the column, in place of the smoke quads.
    pub plume: bool,
}

const UP: DVec3 = DVec3::Y;

/// The height (m) over which a puff of a fire's column stands clear of the flames it rose from: where it left them,
/// raised by the radius it left with. Above it, its smoke hides those flames in the occlusion map (`gas_sprite.wgsl`'s
/// `column_occluder`); below it, it is the column's foot, which the flames stand in, and dims them not at all.
fn clear_of_flames(p: &FireParticle) -> f32 {
    (p.origin.y + p.size) as f32
}

/// Trial `flipbook`: how a puff of a fire's smoke column shows, drawn as a link or a billow: its fade (in, and over
/// the last of its life, a link's neighbours covering for it: `fires.rs` gives every other link a shorter one), the
/// optical depth it thins to as it spreads, its albedo and the firelight under it.
fn column_look(p: &FireParticle, extent: f64) -> (f64, f64, Rgb, f64) {
    let t = p.age / p.life;
    let fade = (if p.fade_in > 0.0 {
        smooth(p.age / p.fade_in)
    } else {
        1.0
    }) * (1.0 - smooth((t - 0.7) / 0.3));
    let glow = p.glow * (-p.age / p.cool).exp();
    // Thinner as it spreads, as the column's quads are (`thin`), by the optical depth rather than the opacity: the
    // puff's rim clears first and its heart stays dark. Its pall, hundreds of metres across, thins to a fifth.
    // A wisp or a puff of steam is thin throughout.
    let density = 4.0 * (p.size / extent).powf(0.4).clamp(0.2, 1.0) * p.opacity.min(1.0);
    // The six baked lights find more of the sun than the column's own shading did (which left most of a puff to the
    // sky's light): soot is darker to match. It keeps its brown cast (until 2026-10-24 a third of it): the sun gives
    // nine tenths of a sunlit column's light, and the sky and the air between take it back toward grey. Steam and the
    // pale wisps of a fire that is out keep their own shade.
    let grey = (p.albedo.r + p.albedo.g + p.albedo.b) / 3.0;
    let albedo = p
        .albedo
        .scale(0.5 + 0.4 * crate::math::smoothstep(grey, 0.2, 0.6));
    (fade, density, albedo, glow)
}

/// How a neutral plume disperses `x` metres downwind of its source (Pasquill–Gifford class D, Briggs' open-country
/// fit): its spread across the wind, σy, and up and down, σz (m). At 4 km they are 270 and 91 m.
pub(crate) fn dispersal(x: f64) -> (f64, f64) {
    let x = x.max(0.0);
    (
        0.08 * x / (1.0 + 0.0001 * x).sqrt(),
        0.06 * x / (1.0 + 0.0015 * x).sqrt(),
    )
}

/// Trial `flipbook`: a link's cross-section as it drifts off downwind, in multiples of its own: across the wind and up
/// and down, each [`dispersal`]'s σ at its distance from the fire where that is wider than the link's own. A link's
/// smoke reaches about its radius (`extent`) either side of its line, some two σ of a plume's, so its own σ is half its
/// radius. That alone has grown with the column's diffusion about as a neutral plume spreads up and down (a big fire's
/// pall, 4 km off, some 220 m in radius, 110 m a σ, against σz's 91 m), but it is round: a real pall spreads three
/// times as wide across the wind (σy, 270 m there). Near the fire the column's own growth is the wider and it stays
/// round; a link standing up the column (no wind) spreads alike all round. Its distance downwind is from where its
/// source is now, had she held her way (`wake`): a ship under way trails her pall in calm air as the wind carries it
/// off one lying still. The sprite shader draws the link as wide as that ellipse is seen along the view and its smoke
/// as much thinner, so the soot the column carries is the same.
pub(crate) fn pall_spread(p: &FireParticle, extent: f64) -> (f64, f64) {
    let off = p.position - p.origin - p.wake * p.age.max(0.0);
    let (across, deep) = dispersal(off.x.hypot(off.z));
    let own = 0.5 * extent;
    let deep = (deep / own).max(1.0);
    let wide = (across / own).max(deep);
    let lying = 1.0 - crate::math::smoothstep(p.axis.y.abs(), 0.5, 0.9);
    (deep + (wide - deep) * lying, deep)
}

/// Trial `flipbook`: a puff of a fire's smoke column as a link of the simulated plume. It lies along the column, a
/// puff's radius wide and [`LINK_REACH`] of it long each way, spread across the wind and deepened as it drifts off
/// ([`pall_spread`]); the smoke in it thins as the puff spreads and goes over the last of its life.
fn link(p: &FireParticle, extent: f64) -> GasPuff {
    let t = p.age / p.life;
    let (fade, density, albedo, glow) = column_look(p, extent);
    let (wide, deep) = pall_spread(p, extent);
    GasPuff {
        sphere: [
            p.position.x as f32,
            p.position.y as f32,
            p.position.z as f32,
            (extent * LINK_REACH) as f32,
        ],
        state: [p.age as f32, p.seed as f32, 0.0, density as f32],
        tint: [albedo.r as f32, albedo.g as f32, albedo.b as f32, 1.0],
        evolution: [
            (p.phase + p.seed * 0.0731).fract() as f32,
            (0.12 * crate::math::smoothstep(t, 0.5, 1.0)) as f32,
            wide as f32,
            deep as f32,
        ],
        quad: [
            p.axis.x as f32,
            p.axis.y as f32,
            p.axis.z as f32,
            clear_of_flames(p),
        ],
        extra: [
            (p.opacity * fade) as f32,
            (glow * 6.0) as f32,
            clip::PLUME as f32,
            0.0,
        ],
        // The warp trial does not roll a link of the plume.
        warp: [0.0; 4],
    }
}

/// Trial `flipbook`: a puff of a fire's smoke column as a billow among its links, a heap of simulated smoke
/// ([`clip::SMOKE`]) rolling and thinning over the puff's life, [`BILLOW_REACH`] of its radius across, upright. The
/// links alone lie end to end into an even tube; the billows give the column the rounded heads that stand out of a
/// burning ship's smoke.
fn billow(p: &FireParticle, extent: f64) -> GasPuff {
    let t = p.age / p.life;
    let (fade, density, albedo, glow) = column_look(p, extent);
    GasPuff {
        sphere: [
            p.position.x as f32,
            p.position.y as f32,
            p.position.z as f32,
            (extent * BILLOW_REACH) as f32,
        ],
        state: [p.age as f32, p.seed as f32, 0.0, density as f32],
        tint: [albedo.r as f32, albedo.g as f32, albedo.b as f32, 1.0],
        evolution: [
            t as f32,
            (0.12 * crate::math::smoothstep(t, 0.5, 1.0)) as f32,
            0.0,
            0.0,
        ],
        // The sprite shader places a flipbook puff's quad itself.
        quad: [0.0, 0.0, 0.0, clear_of_flames(p)],
        extra: [
            (p.opacity * fade) as f32,
            (glow * 6.0) as f32,
            clip::SMOKE as f32,
            GAS_COLUMN,
        ],
        warp: [0.0; 4],
    }
}

impl FireBatch {
    pub fn new(capacity: usize, mode: FireMode) -> Self {
        Self {
            capacity,
            mode,
            particles: vec![FireParticle::default(); capacity],
            scratch: FireParticle::default(),
            cursor: 0,
            emitted: 0,
            density: 1.0,
            density_phase: 0.0,
            drawn: 0,
            order: Vec::new(),
            volume_sources: Vec::new(),
            plume: false,
        }
    }

    /// A reset particle at `position`. A full batch gives up the most-spent particle of the next 96 slots, so a
    /// burst of short flames never cuts a long smoke column short.
    pub fn emit(&mut self, position: DVec3, source: Option<&SourceId>) -> &mut FireParticle {
        if self.density < 1.0 {
            self.density_phase += self.density;
            if self.density_phase < 1.0 {
                self.scratch = FireParticle::default();
                return &mut self.scratch;
            }
            self.density_phase -= 1.0;
        }
        let mut chosen = self.cursor % self.capacity;
        let mut spent = f64::NEG_INFINITY;
        for probe in 0..self.capacity.min(96) {
            let index = (self.cursor + probe) % self.capacity;
            let candidate = &self.particles[index];
            if candidate.age >= candidate.life {
                chosen = index;
                break;
            }
            let fraction = candidate.age / candidate.life;
            if fraction > spent {
                spent = fraction;
                chosen = index;
            }
        }
        self.cursor = chosen + 1;
        self.emitted += 1;
        let p = &mut self.particles[chosen];
        *p = FireParticle {
            source_id: source.map(Arc::clone),
            origin: position,
            position,
            life: 1.0,
            seed: golden_seed(self.emitted),
            ..FireParticle::default()
        };
        p
    }

    /// Age and move every particle; the same dt at any frame rate gives the same climb.
    pub fn advance(&mut self, dt: f64, wind: DVec3) {
        if dt <= 0.0 {
            return;
        }
        for p in &mut self.particles {
            if p.age >= p.life {
                continue;
            }
            p.age += dt;
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            let step = dt.min(p.age);
            let (climb, rising) = p.climb_at(p.age);
            p.position.y = p.origin.y + climb;
            let decay = (-p.drag * p.age).exp();
            let carried = p.carried_at(climb);
            let across = DVec3::new(
                p.drift.x * decay + wind.x * carried,
                0.0,
                p.drift.z * decay + wind.z * carried,
            );
            p.position.x += across.x * step;
            p.position.z += across.z * step;
            if self.plume {
                // The column runs along the puff's motion as its source saw it leave, and its smoke rolls up the
                // link a share as fast as the puff climbs: quick over the fire, slow where the pall hangs.
                p.axis = (across - p.wake + (DVec3::Y + p.lean) * rising).normalize_or(DVec3::Y);
                let length = 2.0 * LINK_REACH * p.extent();
                p.phase += step * (0.35 * rising).max(1.2) / (PLUME_CLIMB * length);
            }
        }
    }

    /// Sort the visible particles back to front and build this frame's quads (or gas puffs).
    pub fn publish(&mut self, camera: &CameraView, hidden: Option<&str>) -> FirePublish {
        let mut publish = FirePublish::default();
        self.publish_into(camera, hidden, &mut publish);
        publish
    }

    /// [`FireBatch::publish`] written over `publish`, whose lists keep their room.
    pub fn publish_into(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        publish: &mut FirePublish,
    ) {
        let planes = camera.frustum();
        let (camera_right, camera_up, camera_back) = camera.axes();
        let (lens_x, lens_y) = camera.lens();
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        let mut fill = 0.0;
        for (index, p) in self.particles.iter().enumerate() {
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            if let (Some(hidden), Some(source)) = (hidden, &p.source_id)
                && &**source == hidden
            {
                continue;
            }
            if !self.volume_sources.is_empty()
                && let Some(source) = &p.source_id
                && self.volume_sources.contains(source)
            {
                continue;
            }
            // A puff faded to nothing would still fill its whole quad.
            if self.mode != FireMode::Flame
                && p.opacity * (1.0 - smooth((p.age / p.life - 0.45) / 0.55)) < 0.01
            {
                continue;
            }
            let extent = p.extent();
            let mut center = p.position;
            if self.mode == FireMode::Flame {
                center.y += extent * 0.5;
            }
            // A link spread across the wind in its pall reaches as far again across it.
            let spread = if self.plume && !p.billow {
                pall_spread(p, extent).0
            } else {
                1.0
            };
            let radius = extent
                * if self.mode == FireMode::Flame {
                    0.75
                } else if self.plume {
                    LINK_REACH * 1.3 * spread
                } else {
                    1.45
                };
            let drawn = camera.drawn(center);
            if planes.iter().any(|plane| plane.distance(drawn) < -radius) {
                continue;
            }
            let distance_sq = (p.position - camera.position).length_squared();
            let area = if self.mode == FireMode::Flame {
                extent * extent * p.aspect
            } else if self.plume {
                4.0 * extent * extent * LINK_REACH * LINK_REACH * PLUME_ASPECT * spread
            } else {
                4.0 * extent * extent
            };
            fill +=
                (area * lens_x * lens_y / (4.0 * distance_sq.max(extent * extent * 0.25))).min(4.0);
            order.push((index, distance_sq, extent));
        }
        // Back to front; equal distances keep slot order (the slot breaks the tie: no sort buffer needed).
        order.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        self.drawn = order.len();

        // One shared basis per frame. Flames: upright on the world axis, facing the camera in azimuth, easing
        // toward a full billboard when looking steeply down so no tongue collapses to a line.
        let (mut right, mut up, mut normal) =
            (camera_right, camera_up, camera_right.cross(camera_up));
        if self.mode == FireMode::Flame {
            let to_camera = camera_back;
            let steep = smooth((to_camera.y.abs() - 0.72) / 0.24);
            normal = DVec3::new(to_camera.x, 0.0, to_camera.z);
            if normal.length_squared() < 1e-6 {
                normal = -camera_up;
                normal.y = 0.0;
            }
            normal = normal.normalize().lerp(to_camera, steep).normalize();
            up = UP.lerp(camera_up, steep).normalize();
            right = up.cross(normal).normalize();
            up = normal.cross(right);
        }
        let basis = FireBasis {
            right: [right.x as f32, right.y as f32, right.z as f32, 0.0],
            up: [up.x as f32, up.y as f32, up.z as f32, 0.0],
            normal: [normal.x as f32, normal.y as f32, normal.z as f32, 0.0],
        };

        publish.quads.clear();
        publish.gas.clear();
        publish.basis = basis;
        publish.fill = fill;
        for &(index, distance_sq, extent) in &order {
            let p = &self.particles[index];
            let t = p.age / p.life;
            if self.plume && self.mode == FireMode::Smoke {
                publish.gas.push(if p.billow {
                    billow(p, extent)
                } else {
                    link(p, extent)
                });
                continue;
            }
            let fade = (if p.fade_in > 0.0 {
                smooth(p.age / p.fade_in)
            } else {
                1.0
            }) * (1.0 - smooth((t - 0.45) / 0.55))
                * if p.thin > 0.0 {
                    (p.size / extent).powf(p.thin).clamp(0.35, 1.0)
                } else {
                    1.0
                }
                // A puff around the camera would fill the screen with one flat layer: clear it as the lens enters.
                * if self.mode == FireMode::Smoke {
                    smooth((distance_sq.sqrt() - extent * 0.7) / (extent * 1.6))
                } else {
                    1.0
                };
            let glow = p.glow * (-p.age / p.cool).exp();
            let opacity = p.opacity * fade;
            let heat = if p.heat > 0.0 {
                p.heat * (1.0 - smooth(p.age / p.cooling))
            } else {
                0.0
            };
            let state = [p.seed as f32, t as f32, glow as f32, opacity as f32];
            let color = [
                p.albedo.r as f32,
                p.albedo.g as f32,
                p.albedo.b as f32,
                p.age as f32,
            ];
            let sphere = |w: f64| {
                [
                    p.position.x as f32,
                    p.position.y as f32,
                    p.position.z as f32,
                    w as f32,
                ]
            };
            match self.mode {
                FireMode::Flame => {
                    let height = extent * (0.55 + 0.9 * t) * (1.0 - t * t);
                    let sy = height;
                    let sx = height * p.aspect * (1.0 - 0.45 * t);
                    let c = p.position + up * sy * 0.5;
                    publish.quads.push(FireQuad {
                        center: [c.x as f32, c.y as f32, c.z as f32, sx as f32],
                        sphere: sphere(sy * 0.5),
                        state,
                        color,
                        size: [sy as f32, 0.0, 0.0, 0.0],
                    });
                }
                FireMode::Smoke => {
                    let edge = extent * 2.0;
                    publish.quads.push(FireQuad {
                        center: sphere(edge),
                        sphere: sphere(extent),
                        state,
                        color,
                        size: [
                            edge as f32,
                            crate::warp::roll(extent, p.size, p.age),
                            0.0,
                            0.0,
                        ],
                    });
                }
                FireMode::Gas => {
                    // `fireBodyInputs`: burning oil is dense (density 5); a body thins into wisps late in its climb.
                    let distance = distance_sq.sqrt();
                    let (placed, edge) =
                        place_facing(camera, p.position, distance, extent, extent * 2.0);
                    publish.gas.push(GasPuff {
                        sphere: sphere(extent),
                        state: [p.age as f32, p.seed as f32, (heat + glow * 0.3) as f32, 5.0],
                        tint: [color[0], color[1], color[2], 1.0],
                        evolution: [
                            t as f32,
                            crate::math::smoothstep(t, 0.6, 1.0) as f32,
                            0.0,
                            0.0,
                        ],
                        quad: [
                            placed.x as f32,
                            placed.y as f32,
                            placed.z as f32,
                            edge as f32,
                        ],
                        extra: [
                            opacity as f32,
                            (glow * 1.6) as f32,
                            0.0,
                            crate::instances::GAS_FIRE,
                        ],
                        // A flame body climbs: its thermal rolls about the way up.
                        warp: crate::warp::gas(crate::warp::roll(extent, p.size, p.age), UP),
                    });
                }
            }
        }
        self.order = order;
    }

    /// Particles drawn by the last publish.
    pub fn drawn(&self) -> usize {
        self.drawn
    }

    /// Particles still alive, drawn or not.
    pub fn alive(&self) -> usize {
        self.particles.iter().filter(|p| p.age < p.life).count()
    }

    pub fn particles(&self) -> &[FireParticle] {
        &self.particles
    }

    pub(crate) fn translate(&mut self, offset: DVec3) {
        for p in &mut self.particles {
            p.position += offset;
            // With its spawn point: a link's spread in its pall goes by how far it has drifted from it.
            p.origin += offset;
        }
    }

    /// Carries the batch into a scene laid on another chart (a theatre's re-lay): each particle's spawn point and
    /// place through their places; its drift, a column link's source velocity (`wake`) and lean through the two
    /// charts' Jacobians where it is, and its axis (a unit direction) likewise.
    pub(crate) fn relay(&mut self, relay: &dyn naval_render::relay::SceneRelay) {
        for p in &mut self.particles {
            let at = p.position.to_array();
            let vector = |v: DVec3| DVec3::from_array(relay.vector(at, v.to_array()));
            p.drift = vector(p.drift);
            p.wake = vector(p.wake);
            p.lean = vector(p.lean);
            p.axis = vector(p.axis).normalize_or(DVec3::Y);
            p.origin = DVec3::from_array(relay.point(p.origin.to_array()));
            p.position = DVec3::from_array(relay.point(at));
        }
    }

    /// Carries the batch's smoke with the blast waves that passed it this step ([`crate::shock::Shove`]), never
    /// under the sea; its climb goes on from where it is carried to.
    pub(crate) fn shove(&mut self, shoves: &[crate::shock::Shove]) {
        for p in &mut self.particles {
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            for shove in shoves {
                if p.age <= shove.age {
                    continue;
                }
                let from = p.position.y;
                p.position += shove.at(p.position);
                p.position.y = p.position.y.max(from.min(0.5));
                p.origin.y += p.position.y - from;
            }
        }
    }

    /// Lets go of the smoke `ship`'s fire `tag` has sent up, cut off at the fire (her place gone under the sea, the
    /// fire drowned): nothing feeds it from below any more, so it thins from its foot. Each puff lives on for as long
    /// again as it has been climbing and a quarter more (at least [`RELEASE_SECONDS`]), never longer than it would
    /// have: the youngest smoke, low over the fire, clears within seconds, the oldest, aloft, goes last, and the whole
    /// drifts on with the air as it did. The fades start no earlier than they would have (a puff stays under 0.45 of
    /// its life, where every fade of the batch begins), so nothing goes at once.
    pub fn release(&mut self, ship: &SourceId, tag: u32) -> usize {
        let mut released = 0;
        for p in &mut self.particles {
            if p.age < 0.0 || p.age >= p.life || p.tag != tag {
                continue;
            }
            if p.source_id.as_deref() != Some(&**ship) {
                continue;
            }
            p.life = p.life.min(p.age + (1.25 * p.age).max(RELEASE_SECONDS));
            released += 1;
        }
        released
    }

    pub fn reset(&mut self) {
        for p in &mut self.particles {
            p.age = 0.0;
            p.life = 0.0;
        }
        self.cursor = 0;
        self.emitted = 0;
        self.drawn = 0;
        self.density_phase = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pall_spreads_across_the_wind_as_a_neutral_plume_does() {
        let (across, deep) = dispersal(4000.0);
        assert!(
            (across - 270.0).abs() < 3.0 && (deep - 91.0).abs() < 1.0,
            "{across:.0} × {deep:.0} m"
        );
        let mut p = FireParticle {
            origin: DVec3::new(0.0, 20.0, 0.0),
            axis: DVec3::X,
            ..FireParticle::default()
        };
        // Over the fire the column's own growth is the wider: round, as drawn before.
        p.position = DVec3::new(60.0, 120.0, 0.0);
        assert_eq!(pall_spread(&p, 12.0), (1.0, 1.0));
        // Four kilometres downwind, a big fire's 220 m link lying along the wind keeps its depth (110 m a σ, more than
        // σz) and spreads across it to σy's 270 m.
        p.position = DVec3::new(4000.0, 620.0, 0.0);
        let (wide, deep) = pall_spread(&p, 220.0);
        assert!(
            (wide * 110.0 - 270.0).abs() < 3.0 && deep == 1.0,
            "{wide:.2} × {deep:.2}"
        );
        // A smaller fire's thinner link spreads up and down too.
        let (wide, deep) = pall_spread(&p, 120.0);
        assert!(
            (wide * 60.0 - 270.0).abs() < 3.0 && (deep * 60.0 - 91.0).abs() < 1.0,
            "{wide:.2} × {deep:.2}"
        );
        // And standing up the column, alike all round.
        p.axis = DVec3::Y;
        assert_eq!(pall_spread(&p, 120.0), (deep, deep));
        // A ship under way at 15 m/s in calm air leaves it four minutes old 3.6 km astern of her: spread as far.
        p.axis = DVec3::X;
        p.position = DVec3::new(19.0, 620.0, 0.0);
        p.wake = DVec3::new(-15.0, 0.0, 0.0);
        p.age = 240.0;
        let (wide, _) = pall_spread(&p, 120.0);
        assert!((wide * 60.0 - dispersal(3619.0).0).abs() < 1.0, "{wide:.2}");
    }

    #[test]
    fn the_climb_is_analytic() {
        let run = |steps: usize| {
            let mut batch = FireBatch::new(4, FireMode::Smoke);
            let p = batch.emit(DVec3::new(0.0, 10.0, 0.0), None);
            p.rise = 8.0;
            p.rise_time = 2.0;
            p.lift = 2.0;
            p.life = 20.0;
            for _ in 0..steps {
                batch.advance(3.0 / steps as f64, DVec3::ZERO);
            }
            batch.particles()[0].position.y
        };
        let expected = 10.0 + 8.0 * 2.0 * (1.0 - (-1.5f64).exp()) + 6.0;
        assert!((run(1) - expected).abs() < 1e-9);
        assert!((run(180) - expected).abs() < 1e-9);
    }

    #[test]
    fn a_blast_wave_above_low_smoke_carries_it_along_the_sea_not_under_it() {
        use crate::shock::Shove;
        let mut batch = FireBatch::new(4, FireMode::Smoke);
        for height in [10.0, 0.2] {
            batch.emit(DVec3::new(28.0, height, 0.0), None).life = 60.0;
        }
        batch.advance(1.0, DVec3::ZERO);
        let before: Vec<(DVec3, f64)> = batch.particles()[..2]
            .iter()
            .map(|p| (p.position, p.origin.y))
            .collect();
        batch.shove(&[Shove {
            centre: DVec3::new(0.0, 20.0, 0.0),
            before: 20.0,
            now: 40.0,
            pulse: 5.0,
            reach: 2.0,
            bore: DVec3::ZERO,
            age: 0.2,
        }]);
        let after = batch.particles();
        // Well over the sea it goes out and down with the front, and its climb goes on from there.
        let fell = after[0].position.y - before[0].0.y;
        assert!(fell < -0.1 && after[0].position.x > before[0].0.x);
        assert!((after[0].origin.y - before[0].1 - fell).abs() < 1e-9);
        // Low over the sea it is carried along it: neither it nor its climb's foot goes under.
        assert!(after[1].position.x > before[1].0.x + 1.0);
        assert_eq!(after[1].position.y, before[1].0.y);
        assert_eq!(after[1].origin.y, before[1].1);
    }
}
