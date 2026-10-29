//! Shells in flight: every round's body and pale trail and, for the lighter rounds that carry one, its tracer: a red
//! spark burning in the base and the streak that spark draws across a frame. Driven by the render packet's
//! projectiles; ballistics stay the simulation's. First a port of `CombatEffects.updateShells` (the web game's
//! `CombatEffects.ts`) and `ShellTrails.ts`; since 2026-10-10 the rounds are drawn as they were seen at sea (see
//! `native/docs/combat-effects/shells-and-strikes.md`, "Shells in flight"): nothing glows from heat and a heavy round is a dark body that
//! is seldom seen. Two aids to play are not what was seen. Every round above the water leaves a short pale trail a few
//! pixels wide at any range (since 2026-10-17: a screen has far fewer pixels than the eye, and without it a salvo was lost
//! between the flash and the splashes), and the heavy rounds of the ship the player commands carry a small marker, a
//! dot a few pixels across, at the head.
//!
//! Two steps a frame, history then geometry:
//! - [`ShellVisuals::update`] takes the frame's shells and `dt`: each airborne round's trail history (a sample every
//!   0.1 s, the corner of a ricochet, 0.6 s of life, at most 16 samples) and what is gone.
//! - [`ShellVisuals::publish`] builds the instances for a camera: bodies (a detailed lathe when the round is big on
//!   screen), for tracers, sparks and streaks, trail segments clipped at the near plane and culled, and the player's
//!   heavy rounds' markers.
//!
//! The bodies, sparks and streaks are the world's size; a trail is sized to the screen. A spark narrower than a few
//! pixels, or a streak or trail narrower than [`MIN_LINE_PX`], is drawn that wide with the light or density it would
//! have spread over it (`shaders/shells.wgsl`): a tracer dims with distance as a point of light does, and neither
//! line breaks into dashes.

use std::collections::BTreeMap;

use bytemuck::{Pod, Zeroable};
use glam::{DMat3, DQuat, DVec3};

use crate::camera::CameraView;
use crate::math::smoothstep;

const UP: DVec3 = DVec3::Y;
/// Rounds lighter than this calibre (m) carry a tracer in the base: destroyers' and light cruisers' guns, which
/// fought at night at ranges a tracer could mark. Heavier rounds marked their splashes with dye instead: a tracer
/// would have had to burn through a 30 s flight.
pub const TRACER_CALIBER: f64 = 0.2;
/// A tracer's spark: the sprite's edge, in calibres (its glow map's bright core is about a third of that), and where
/// it burns, in the cup in the base, behind the round's centre (the lathe's base, `shell_layer::shell_geometry`).
const SPARK_CALIBERS: f64 = 3.0;
const BASE_CALIBERS: f64 = 1.95;
/// The streak: the spark's light across one frame at 60 Hz (s), and its width (calibres).
pub const STREAK_EXPOSURE: f64 = 1.0 / 60.0;
const STREAK_CALIBERS: f64 = 0.6;
/// The trail: its life (s, about 500 m behind a heavy round; the web game's was 1.25 s), its sample interval (s) and
/// the samples one trail keeps.
const LIFE: f64 = 0.6;
const SAMPLE_INTERVAL: f64 = 0.1;
const MAX_SAMPLES: usize = 16;
/// The trail's width, a share of the view's height at its depth (3.5 px at 1080p), and its opacity at the round,
/// both for a 38 cm round and scaled by √(calibre / 0.38): the web game's width without its cap of 10 m, which thinned
/// a trail past 3 km to a faint hair, and a little fainter than its 0.48, so it marks a salvo without drawing over the
/// sky.
const TRAIL_VIEW: f64 = 0.0032;
const TRAIL_OPACITY: f64 = 0.4;
/// Cosmetic work stays bounded in dense salvos; rounds themselves are never dropped.
const MAX_SEGMENTS: usize = 8192;
/// The narrowest streak or trail drawn, in pixels; narrower ones fade by the width they lost instead.
pub const MIN_LINE_PX: f32 = 2.0;
/// The narrowest spark drawn, in pixels; a smaller one spreads its light over that.
pub const MIN_SPARK_PX: f32 = 3.0;

/// A round in flight, from the render packet (`naval_sim_host::ShellRender`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShellState {
    /// Stable across packets.
    pub id: i64,
    /// Who fired it, hashed (the web keys a trail by id and owner; 0 when unknown).
    pub owner: u64,
    pub position: DVec3,
    pub velocity: DVec3,
    pub caliber_m: f64,
    /// Seconds since it left the muzzle.
    pub age: f64,
    /// Above the water (the web's `waterDragPerSecond === undefined`): a tracer burns and a trail is laid only there.
    /// Bombs are not shells here.
    pub airborne: bool,
    /// Fired by the ship the local seat holds the helm of.
    pub helm: bool,
}

impl ShellState {
    /// Whether its tracer shows: a round lighter than [`TRACER_CALIBER`], above the water.
    pub fn tracer(&self) -> bool {
        self.airborne && self.caliber_m < TRACER_CALIBER
    }

    /// Whether it carries a marker: a heavy round of the helm's, above the water (a light one has its tracer).
    pub fn marker(&self) -> bool {
        self.helm && self.airborne && self.caliber_m >= TRACER_CALIBER
    }

    /// Its spark's sprite edge (m), before the shader's floor of [`MIN_SPARK_PX`].
    pub fn spark_size(&self) -> f64 {
        self.caliber_m * SPARK_CALIBERS
    }

    /// Where its tracer burns: the base, `direction` (its unit flight) back from its centre.
    pub fn base(&self, direction: DVec3) -> DVec3 {
        self.position - direction * (self.caliber_m * BASE_CALIBERS)
    }
}

/// A shell body: the lathe placed and scaled (`shaders/shells.wgsl`, twin `Body`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct BodyInstance {
    /// Position (m) and its tracer (1 burning in the base, 0 none or out under water).
    pub position: [f32; 4],
    /// Rotation taking +Y to the flight direction (quaternion xyzw).
    pub rotation: [f32; 4],
    /// Calibre (m, the lathe is in calibres), unused ×3.
    pub scale: [f32; 4],
}

/// A tracer's spark, a camera-facing sprite: centre and edge length (twin `Glow`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GlowInstance {
    /// Centre (m) and edge length (m; the shader draws it at least [`MIN_SPARK_PX`] wide).
    pub position: [f32; 4],
}

/// A tracer streak or one trail segment: a quad on `across` × `along` at `center` (twin `Strip`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct StripInstance {
    /// Centre (m), and for a streak its width (m).
    pub center: [f32; 4],
    /// Unit across axis, and the width at the tail (m) for a trail.
    pub across: [f32; 4],
    /// Tail-to-head vector (m: its length is the quad's), and the width at the head (m) for a trail.
    pub along: [f32; 4],
    /// Opacity at the tail and at the head (a trail), unused ×2.
    pub opacity: [f32; 4],
}

/// What the shells draw this frame, for a camera.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShellFrame {
    pub bodies: Vec<BodyInstance>,
    pub detailed: Vec<BodyInstance>,
    pub glows: Vec<GlowInstance>,
    pub streaks: Vec<StripInstance>,
    pub trails: Vec<StripInstance>,
    /// The helm's heavy rounds' markers: their centres and calibres (the edge is the shader's, a few pixels).
    pub markers: Vec<GlowInstance>,
}

#[derive(Clone, Copy, Debug)]
struct Sample {
    position: DVec3,
    age: f64,
}

#[derive(Clone, Debug)]
struct Trail {
    owner: u64,
    caliber_m: f64,
    samples: Vec<Sample>,
    head: DVec3,
    velocity: DVec3,
    head_age: f64,
    age: f64,
}

impl Trail {
    fn append(&mut self, position: DVec3, age: f64) {
        if self.samples.len() == MAX_SAMPLES {
            self.samples.remove(0);
        }
        self.samples.push(Sample { position, age });
    }

    /// Its width and opacity against a 38 cm round's.
    fn scale(&self) -> f64 {
        (self.caliber_m / 0.38).sqrt()
    }

    /// Its width (m) at a point `view_height` metres of the view tall: a few pixels, never narrower than the round.
    fn width(&self, view_height: f64) -> f64 {
        (TRAIL_VIEW * view_height * self.scale()).max(self.caliber_m)
    }

    /// Its opacity where it left the round at `at` (the round's age then), fading out over its life.
    fn opacity(&self, at: f64) -> f64 {
        TRAIL_OPACITY * self.scale() * smoothstep(1.0 - (self.age - at) / LIFE, 0.0, 1.0)
    }
}

/// Counts for telemetry: rounds, trail histories and the segments drawn last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellCounts {
    pub shells: usize,
    pub histories: usize,
    pub segments: usize,
}

/// The shells' CPU side (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct ShellVisuals {
    shells: Vec<ShellState>,
    trails: BTreeMap<i64, Trail>,
    segments: usize,
    /// The trails are left out of the frame (Graphics → Shell trails off, photo mode, films). Their histories still
    /// step, so they show again whole.
    pub hide_trails: bool,
}

fn normalize_or(v: DVec3, fallback: DVec3) -> DVec3 {
    let length = v.length();
    if length > 0.0 { v / length } else { fallback }
}

impl ShellVisuals {
    pub fn new() -> Self {
        Self::default()
    }

    /// The frame's shells (bombs excluded) and the time since the last frame: steps every airborne round's trail history
    /// (`ShellTrails.update`'s first half, and its pruning).
    pub fn update(&mut self, shells: &[ShellState], dt: f64) {
        self.shells.clear();
        self.shells.extend_from_slice(shells);
        for trail in self.trails.values_mut() {
            trail.age += dt;
        }
        for shell in shells.iter().filter(|shell| shell.airborne) {
            let direction = normalize_or(shell.velocity, DVec3::ZERO);
            let replaced = self
                .trails
                .get(&shell.id)
                .is_none_or(|trail| trail.owner != shell.owner || shell.age < trail.head_age);
            if replaced {
                // Backfill only the launch tick; never invent a long straight path for a round first seen in flight.
                let age = shell.age.min(1.0 / 60.0);
                let mut trail = Trail {
                    owner: shell.owner,
                    caliber_m: shell.caliber_m,
                    samples: Vec::with_capacity(MAX_SAMPLES),
                    head: shell.position,
                    velocity: direction,
                    head_age: shell.age,
                    age: shell.age,
                };
                trail.append(shell.position - shell.velocity * age, shell.age - age);
                self.trails.insert(shell.id, trail);
            }
            let trail = self.trails.get_mut(&shell.id).expect("inserted above");
            // Keep the last observed corner of a ricochet even between sample times.
            let last_age = trail.samples.last().map_or(f64::NEG_INFINITY, |s| s.age);
            if trail.velocity.dot(direction) < 0.96 && trail.head_age > last_age {
                let (head, head_age) = (trail.head, trail.head_age);
                trail.append(head, head_age);
            }
            trail.velocity = direction;
            trail.age = shell.age;
            trail.head_age = shell.age;
            trail.head = shell.position;
            let last_age = trail.samples.last().map_or(f64::NEG_INFINITY, |s| s.age);
            if shell.age - last_age >= SAMPLE_INTERVAL {
                trail.append(shell.position, shell.age);
            }
        }
        // Drop samples older than the life (keeping one to start the oldest segment) and spent trails.
        self.trails.retain(|_, trail| {
            let cutoff = trail.age - LIFE;
            while trail.samples.len() > 1 && trail.samples[1].age <= cutoff {
                trail.samples.remove(0);
            }
            trail.head_age > cutoff
        });
    }

    /// The instances for `camera` (`CombatEffects.updateShells` and `ShellTrails.update`'s second half).
    pub fn publish(&mut self, camera: &CameraView) -> ShellFrame {
        let mut frame = ShellFrame::default();
        self.publish_into(camera, &mut frame);
        frame
    }

    /// [`ShellVisuals::publish`] written over `frame`, whose lists keep their room (a steady frame allocates none).
    pub fn publish_into(&mut self, camera: &CameraView, frame: &mut ShellFrame) {
        frame.bodies.clear();
        frame.detailed.clear();
        frame.glows.clear();
        frame.streaks.clear();
        frame.trails.clear();
        frame.markers.clear();
        let perspective = camera.projection.z_axis.w == -1.0;
        let lens_y = camera.projection.y_axis.y;
        for shell in &self.shells {
            let tracer = shell.tracer();
            let position = shell.position;
            let direction = normalize_or(shell.velocity, UP);
            let pose = DQuat::from_rotation_arc(UP, direction);
            // Physical size; the projection follows binocular zoom.
            let depth = if perspective {
                (-camera.to_view(position).z).max(0.1)
            } else {
                1.0
            };
            let view_height = 2.0 * depth / lens_y;
            let body = BodyInstance {
                position: [
                    position.x as f32,
                    position.y as f32,
                    position.z as f32,
                    if tracer { 1.0 } else { 0.0 },
                ],
                rotation: pose.as_quat().to_array(),
                scale: [shell.caliber_m as f32, 0.0, 0.0, 0.0],
            };
            // Spend the engraved lathe only when the projected round can show it.
            if shell.caliber_m / view_height > 0.002 {
                frame.detailed.push(body);
            } else {
                frame.bodies.push(body);
            }
            if shell.marker() {
                frame.markers.push(GlowInstance {
                    position: [
                        position.x as f32,
                        position.y as f32,
                        position.z as f32,
                        shell.caliber_m as f32,
                    ],
                });
            }
            if !tracer {
                continue;
            }
            let base = shell.base(direction);
            frame.glows.push(GlowInstance {
                position: [
                    base.x as f32,
                    base.y as f32,
                    base.z as f32,
                    shell.spark_size() as f32,
                ],
            });
            // The spark's light across one frame, ending at it, and not behind a fresh muzzle. A camera that close
            // rides along with the round (the follow camera), so the streak recedes there; the spark stays.
            let tracking = smoothstep(camera.position.distance(position), 65.0, 150.0);
            let speed = shell.velocity.length();
            let length = speed * STREAK_EXPOSURE.min(shell.age) * (0.04 + 0.96 * tracking);
            let center = base - direction * (length / 2.0);
            let (across, _) = strip_axes(direction, camera.position - center);
            let width = shell.caliber_m * STREAK_CALIBERS;
            let along = direction * length;
            frame.streaks.push(StripInstance {
                center: [
                    center.x as f32,
                    center.y as f32,
                    center.z as f32,
                    width as f32,
                ],
                across: [across.x as f32, across.y as f32, across.z as f32, 0.0],
                along: [along.x as f32, along.y as f32, along.z as f32, 0.0],
                opacity: [1.0, 1.0, 0.0, 0.0],
            });
        }
        if self.hide_trails {
            self.segments = 0;
        } else {
            self.publish_trails(camera, frame);
        }
    }

    fn publish_trails(&mut self, camera: &CameraView, frame: &mut ShellFrame) {
        let perspective = camera.projection.z_axis.w == -1.0;
        let near = if perspective { camera.near } else { 0.0 };
        let frustum = camera.frustum();
        // Metres the view spans at a depth, and a pixel of it (the shader's `pixel_meters`).
        let view = 2.0 / camera.projection.y_axis.y;
        let pixel = view / camera.height.max(1.0);
        let depth_of = |p: DVec3| {
            if perspective {
                -camera.to_view(p).z
            } else {
                1.0
            }
        };
        let mut count = 0;
        'trails: for trail in self.trails.values() {
            let cutoff = trail.age - LIFE;
            let end_age = trail.head_age;
            for i in 0..trail.samples.len() {
                if count >= MAX_SEGMENTS {
                    break 'trails;
                }
                let a = trail.samples[i];
                let b = trail.samples.get(i + 1);
                let end = b.map_or(trail.head, |b| b.position);
                let mut age = b.map_or(end_age, |b| b.age);
                if age <= cutoff {
                    continue;
                }
                let mut start_age = a.age.max(cutoff);
                let t = if age > a.age {
                    (start_age - a.age) / (age - a.age)
                } else {
                    0.0
                };
                let mut start = a.position.lerp(end, t);
                let mut end = end;
                let mut tail_depth = depth_of(start);
                let mut head_depth = depth_of(end);
                if tail_depth < near && head_depth < near {
                    continue;
                }
                // A followed trail passes beside and behind the camera: clip before widening it.
                if tail_depth < near {
                    let t = (near - tail_depth) / (head_depth - tail_depth);
                    start = start.lerp(end, t);
                    start_age += (age - start_age) * t;
                    tail_depth = near;
                } else if head_depth < near {
                    let t = (near - head_depth) / (tail_depth - head_depth);
                    end = end.lerp(start, t);
                    age += (start_age - age) * t;
                    head_depth = near;
                }
                let delta = end - start;
                let length = delta.length();
                if length < 0.001 {
                    continue;
                }
                let center = start + delta * 0.5;
                let tail_width = trail.width(view * tail_depth);
                let head_width = trail.width(view * head_depth);
                // Offscreen history is kept but not drawn; the radius holds the ribbon as drawn at either end.
                let floor = f64::from(MIN_LINE_PX) * pixel * tail_depth.max(head_depth);
                let radius = length / 2.0 + tail_width.max(head_width).max(floor);
                let drawn = camera.drawn(center);
                if frustum.iter().any(|plane| plane.distance(drawn) < -radius) {
                    continue;
                }
                let direction = delta / length;
                let (across, _) = strip_axes(direction, camera.position - center);
                frame.trails.push(StripInstance {
                    center: [center.x as f32, center.y as f32, center.z as f32, 0.0],
                    across: [
                        across.x as f32,
                        across.y as f32,
                        across.z as f32,
                        tail_width as f32,
                    ],
                    along: [
                        delta.x as f32,
                        delta.y as f32,
                        delta.z as f32,
                        head_width as f32,
                    ],
                    opacity: [
                        trail.opacity(start_age) as f32,
                        trail.opacity(age) as f32,
                        0.0,
                        0.0,
                    ],
                });
                count += 1;
            }
        }
        self.segments = count;
    }

    pub fn counts(&self) -> ShellCounts {
        ShellCounts {
            shells: self.shells.len(),
            histories: self.trails.len(),
            segments: self.segments,
        }
    }

    /// Forgets every round and trail (a new battle).
    pub fn reset(&mut self) {
        self.shells.clear();
        self.trails.clear();
        self.segments = 0;
    }
}

/// The across and normal axes of a strip along `direction` turned to face `toward_camera`: three's
/// `makeBasis(across, direction, normal)` with the web's end-on fallback.
fn strip_axes(direction: DVec3, toward_camera: DVec3) -> (DVec3, DVec3) {
    let mut normal = normalize_or(toward_camera, DVec3::Z);
    let mut across = direction.cross(normal);
    if across.length_squared() < 1e-8 {
        normal = if direction.x.abs() > 0.9 {
            DVec3::Y
        } else {
            DVec3::X
        };
        across = direction.cross(normal);
    }
    let across = across.normalize();
    normal = across.cross(direction).normalize();
    // The basis the web writes (columns across, direction, normal) is right-handed by construction.
    debug_assert!(DMat3::from_cols(across, direction, normal).determinant() > 0.0);
    (across, normal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_render::{Camera, FrameTime};

    fn camera(eye: DVec3, target: DVec3) -> CameraView {
        let camera = Camera::look_at(eye.as_vec3(), target.as_vec3());
        CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::fixed(0, 1.0 / 60.0)))
    }

    /// A 12.7 cm round (a destroyer's 5-inch, which carries a tracer).
    fn shell(id: i64, position: DVec3, velocity: DVec3, age: f64) -> ShellState {
        ShellState {
            id,
            owner: 7,
            position,
            velocity,
            caliber_m: 0.127,
            age,
            airborne: true,
            helm: false,
        }
    }

    /// A 12.7 cm round flying +X at 800 m/s from (0, 20, 0), stepped at 60 Hz from `from` to `to` seconds of flight.
    fn fly_between(visuals: &mut ShellVisuals, from: f64, to: f64) -> DVec3 {
        let velocity = DVec3::new(800.0, 0.0, 0.0);
        let mut position = DVec3::new(0.0, 20.0, 0.0);
        let first = (from * 60.0).round() as usize + 1;
        for step in first..=(to * 60.0).round() as usize {
            let age = step as f64 / 60.0;
            position = DVec3::new(0.0, 20.0, 0.0) + velocity * age;
            visuals.update(&[shell(1, position, velocity, age)], 1.0 / 60.0);
        }
        position
    }

    fn fly(visuals: &mut ShellVisuals, seconds: f64) -> DVec3 {
        fly_between(visuals, 0.0, seconds)
    }

    #[test]
    fn a_trail_samples_every_tenth_of_a_second_and_lives_six_tenths() {
        let mut visuals = ShellVisuals::new();
        fly(&mut visuals, 0.5);
        let trail = &visuals.trails[&1];
        // The launch backfill, then a sample once 0.1 s has passed (a frame later when the sum rounds under).
        assert!(
            (5..=7).contains(&trail.samples.len()),
            "{:?}",
            trail.samples
        );
        fly_between(&mut visuals, 0.5, 3.0);
        let trail = &visuals.trails[&1];
        let oldest = trail.samples[1].age;
        assert!(
            trail.age - oldest < LIFE && trail.samples.len() <= 8,
            "only the last 0.6 s is kept: {} samples, oldest after the first at {oldest}",
            trail.samples.len()
        );
        // The round lands: its trail fades and is dropped after its life.
        for _ in 0..40 {
            visuals.update(&[], 1.0 / 60.0);
        }
        assert!(visuals.trails.is_empty());
    }

    #[test]
    fn a_tracer_draws_a_body_a_spark_a_one_frame_streak_and_a_trail_facing_the_camera() {
        let mut visuals = ShellVisuals::new();
        fly(&mut visuals, 1.0);
        // The last 480 m of its flight in view, 2 km off its beam.
        let view = camera(
            DVec3::new(600.0, 70.0, 2000.0),
            DVec3::new(600.0, 20.0, 0.0),
        );
        let frame = visuals.publish(&view);
        assert_eq!((frame.bodies.len(), frame.detailed.len()), (1, 0));
        assert_eq!(frame.bodies[0].position[3], 1.0, "its tracer burns");
        assert_eq!(frame.glows.len(), 1);
        assert!(
            (frame.glows[0].position[3] - 0.381).abs() < 1e-4,
            "a spark three calibres across"
        );
        assert_eq!(frame.streaks.len(), 1);
        assert!(frame.trails.len() >= 5, "{} segments", frame.trails.len());
        let streak = frame.streaks[0];
        // 800 m/s for a sixtieth of a second, the full exposure beyond 150 m, ending at the round.
        let length =
            DVec3::from_array([streak.along[0], streak.along[1], streak.along[2]].map(f64::from))
                .length();
        assert!(
            (length - 800.0 / 60.0).abs() < 0.01,
            "streak length {length}"
        );
        assert!(
            (streak.center[3] - 0.0762).abs() < 1e-4,
            "the streak's own width, not the screen's"
        );
        let across = DVec3::from_array(
            [streak.across[0], streak.across[1], streak.across[2]].map(f64::from),
        );
        assert!(
            across.dot(DVec3::X).abs() < 1e-6,
            "the streak's across axis is square to its flight"
        );
        // The trail's newest segment is the most opaque, its oldest the faintest; a 12.7 cm round's is √(1 / 3) of a
        // 38 cm round's.
        let (first, last) = (frame.trails.first().unwrap(), frame.trails.last().unwrap());
        let scale = (0.127f64 / 0.38).sqrt();
        assert!(
            last.opacity[1] > first.opacity[0]
                && (f64::from(last.opacity[1]) - TRAIL_OPACITY * scale).abs() < 1e-3,
            "{} … {}",
            first.opacity[0],
            last.opacity[1]
        );
        // A few pixels wide on the 720-row screen, near and far along it.
        for segment in [first, last] {
            let center = DVec3::from_array(
                [segment.center[0], segment.center[1], segment.center[2]].map(f64::from),
            );
            let pixel = 2.0 * -view.to_view(center).z / (view.projection.y_axis.y * 720.0);
            let px = f64::from(segment.across[3]) / pixel;
            assert!(
                (px - TRAIL_VIEW * 720.0 * scale).abs() < 0.1,
                "{px} px wide"
            );
        }
    }

    #[test]
    fn a_trail_far_off_keeps_its_few_pixels() {
        let mut visuals = ShellVisuals::new();
        let mut round = shell(
            2,
            DVec3::new(0.0, 900.0, 0.0),
            DVec3::new(700.0, 0.0, 0.0),
            10.0,
        );
        round.caliber_m = 0.38;
        for _ in 0..60 {
            round.position.x += 700.0 / 60.0;
            round.age += 1.0 / 60.0;
            visuals.update(&[round], 1.0 / 60.0);
        }
        // 18 km off its beam.
        let view = camera(
            DVec3::new(350.0, 50.0, 18_000.0),
            DVec3::new(350.0, 900.0, 0.0),
        );
        let frame = visuals.publish(&view);
        let segment = frame.trails.last().expect("a trail far off");
        let pixel = 2.0 * 18_000.0 / (view.projection.y_axis.y * 720.0);
        let px = f64::from(segment.along[3]) / pixel;
        assert!(
            (px - TRAIL_VIEW * 720.0).abs() < 0.1,
            "a 38 cm round's trail is 2.3 px wide at 720 rows, as close by: {px}"
        );
    }

    #[test]
    fn a_heavy_round_is_a_dark_body_with_a_trail_but_no_tracer() {
        let mut visuals = ShellVisuals::new();
        let mut round = shell(
            2,
            DVec3::new(400.0, 300.0, 0.0),
            DVec3::new(760.0, 150.0, 0.0),
            0.5,
        );
        round.caliber_m = 0.38;
        visuals.update(&[round], 1.0 / 60.0);
        let frame = visuals.publish(&camera(
            DVec3::new(400.0, 320.0, 1500.0),
            DVec3::new(400.0, 300.0, 0.0),
        ));
        assert_eq!(frame.bodies.len() + frame.detailed.len(), 1);
        assert_eq!(frame.bodies[0].position[3], 0.0, "nothing burns in it");
        assert!(frame.glows.is_empty() && frame.streaks.is_empty());
        assert!(!frame.trails.is_empty(), "it leaves a trail");
        assert!(
            frame.markers.is_empty(),
            "another ship's round has no marker"
        );
    }

    #[test]
    fn hidden_trails_draw_nothing_and_show_again_whole() {
        let mut visuals = ShellVisuals::new();
        visuals.hide_trails = true;
        fly(&mut visuals, 1.0);
        let view = camera(
            DVec3::new(600.0, 70.0, 2000.0),
            DVec3::new(600.0, 20.0, 0.0),
        );
        let hidden = visuals.publish(&view);
        assert!(hidden.trails.is_empty() && visuals.counts().segments == 0);
        assert_eq!(hidden.bodies.len(), 1, "the round itself is drawn");
        assert_eq!(hidden.streaks.len(), 1, "and its tracer's streak");
        visuals.hide_trails = false;
        let shown = visuals.publish(&view);
        assert!(shown.trails.len() >= 5, "{} segments", shown.trails.len());
    }

    #[test]
    fn only_the_helms_heavy_rounds_above_the_water_carry_a_marker() {
        let mut visuals = ShellVisuals::new();
        let at = |id, z| {
            shell(
                id,
                DVec3::new(400.0, 300.0, z),
                DVec3::new(760.0, 150.0, 0.0),
                0.5,
            )
        };
        let heavy = |round: ShellState| ShellState {
            caliber_m: 0.38,
            ..round
        };
        let ours = ShellState {
            helm: true,
            ..heavy(at(1, 0.0))
        };
        let theirs = heavy(at(2, 20.0));
        let light = ShellState {
            helm: true,
            ..at(3, 40.0)
        };
        let sunk = ShellState {
            airborne: false,
            ..ours
        };
        visuals.update(
            &[ours, theirs, light, ShellState { id: 4, ..sunk }],
            1.0 / 60.0,
        );
        let frame = visuals.publish(&camera(
            DVec3::new(400.0, 320.0, 1500.0),
            DVec3::new(400.0, 300.0, 0.0),
        ));
        assert_eq!(frame.markers.len(), 1, "{:?}", frame.markers);
        assert_eq!(
            frame.markers[0].position,
            [400.0, 300.0, 0.0, 0.38],
            "at the round"
        );
        assert_eq!(
            frame.glows.len(),
            1,
            "the light round has its tracer instead"
        );
    }

    #[test]
    fn a_tracer_close_to_the_camera_gets_the_detailed_body_and_a_short_exposure() {
        let mut visuals = ShellVisuals::new();
        let head = fly(&mut visuals, 0.5);
        let view = camera(head + DVec3::new(-3.0, 0.5, 4.0), head);
        let frame = visuals.publish(&view);
        assert_eq!((frame.bodies.len(), frame.detailed.len()), (0, 1));
        let along = frame.streaks[0].along;
        let length = (along[0] * along[0] + along[1] * along[1] + along[2] * along[2]).sqrt();
        assert!(
            length < 1.0,
            "a close round's streak shrinks to 4 %: {length}"
        );
    }

    #[test]
    fn a_round_under_water_is_a_cold_body_without_light() {
        let mut visuals = ShellVisuals::new();
        let mut round = shell(
            3,
            DVec3::new(0.0, -2.0, 0.0),
            DVec3::new(0.0, -100.0, 50.0),
            4.0,
        );
        round.airborne = false;
        visuals.update(&[round], 1.0 / 60.0);
        let frame = visuals.publish(&camera(DVec3::new(0.0, 10.0, 300.0), DVec3::ZERO));
        assert_eq!(frame.bodies.len() + frame.detailed.len(), 1);
        assert_eq!(
            frame
                .bodies
                .first()
                .or(frame.detailed.first())
                .unwrap()
                .position[3],
            0.0
        );
        assert!(frame.glows.is_empty() && frame.streaks.is_empty() && frame.trails.is_empty());
        assert!(visuals.trails.is_empty());
    }

    #[test]
    fn a_ricochet_keeps_its_corner_and_a_new_shot_under_the_same_id_starts_over() {
        let mut visuals = ShellVisuals::new();
        let corner = fly(&mut visuals, 0.55);
        let bounce = DVec3::new(400.0, 300.0, 0.0);
        let after = corner + bounce / 60.0;
        visuals.update(&[shell(1, after, bounce, 0.55 + 1.0 / 60.0)], 1.0 / 60.0);
        assert!(
            visuals.trails[&1]
                .samples
                .iter()
                .any(|s| s.position.distance(corner) < 1e-9),
            "the corner is kept"
        );
        visuals.update(
            &[shell(1, DVec3::new(0.0, 20.0, 0.0), DVec3::X * 800.0, 0.01)],
            1.0 / 60.0,
        );
        assert_eq!(
            visuals.trails[&1].samples.len(),
            1,
            "a younger round under the id is a new shot"
        );
    }
}
