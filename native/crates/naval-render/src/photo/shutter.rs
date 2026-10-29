//! Photo mode's shutter (`native/docs/photo.md`): the scene blurred by what would move while the shutter is open.
//!
//! Photo mode stops time, so nothing moves between frames and temporal AA's motion is zero. The shutter takes the
//! velocities of the frozen moment instead: the host hands the renderer the [`MovingBody`]s in view (hulls with their
//! turn, airframes, rounds and their tracers, splash columns) and the camera's own velocity ([`Shutter`]: still, or the
//! subject's when panning), and five steps after the scene and the motion pass (`shaders/post/photo_shutter.wgsl`):
//!
//! 1. **velocity** (full resolution): each pixel's world point from the scene's depth, the velocity of the body holding
//!    it (else the still world's), relative to the camera, projected at the shutter's opening and closing: half the
//!    line it draws on the sensor, in pixels, at most [`MAX_RADIUS_OF_HEIGHT`] of the frame's height, and its depth. A
//!    splash column the view ray enters in front of the scene takes the pixel instead (its water writes no depth).
//! 2. **tiles**: the longest travel in each 16 × 16 tile, in two steps of 4 × 4;
//! 3. **neighbours**: the longest travel among the tiles near enough to reach a tile;
//! 4. **gather** (full): McGuire's reconstruction filter (2012) in Jimenez's mirrored pairs (2014): samples along the
//!    neighbourhood's longest travel, one in front of the pixel counted where its own travel reaches the pixel, one
//!    behind where the pixel's reaches it, so a sharp hull never smears the streaking sea behind her into herself;
//! 5. the result copied back over `scene.hdr`, so the eye, the bloom, the depth of field and the finish all take the
//!    blurred radiance (scene-referred, before any of them).
//!
//! A tile whose neighbourhood travels under half a pixel is the scene as it was, so a still scene comes out unchanged.
//! Off (no [`Shutter`]), nothing is drawn or allocated.
//!
//! In play the film camera (`crate::play_camera`) takes the same passes from another velocity ([`Velocity::Motion`]):
//! temporal AA's, each pixel's travel since the last frame (the camera's from depth and the last frame's
//! view-projection, a moving surface's own from the motion pass), scaled to the exposure.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

use crate::Camera;
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::fullscreen_recipe;
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;

pub const PHOTO_SHUTTER_SHADER: ShaderFile = crate::shader_file!("shaders/post/photo_shutter.wgsl");

/// The shutter's targets' format.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Pixels a side of the tiles the longest travels are kept in.
const TILE: u32 = 16;
/// The longest half-travel drawn: its share of the frame's height (a streak at most a sixth of the frame long).
pub const MAX_RADIUS_OF_HEIGHT: f32 = 0.08;
/// The most sample pairs one pixel's gather takes, and the fewest.
pub const MAX_PAIRS: f32 = 24.0;
pub const MIN_PAIRS: f32 = 4.0;
/// The pixels between a gather's sample pairs it aims for (fewer pairs for a shorter travel).
pub const PAIR_SPACING: f32 = 1.5;
/// The bodies one frame takes at most (the nearest first beyond them).
pub const MAX_BODIES: usize = 1024;

/// Photo mode's shutter: how long it is open and how the camera moves meanwhile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shutter {
    /// Seconds the shutter is open (1/1000 to 1/15 in the viewfinder).
    pub exposure_s: f32,
    /// The camera's world velocity while it is open, m/s: zero for a camera held still; the subject's when panning,
    /// the camera riding with her, so she stays sharp and what moves past her streaks.
    pub camera_velocity: Vec3,
}

/// What a moving body takes up, for the pixels it gives its velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MovingShape {
    /// A hull about her waterline centre: `forward` her bow and `up` her deck's up (unit, world), `length` × `beam`,
    /// from `bottom` (below the waterline, negative) to `top` metres along `up`. Within 1.5 m of her waterline only her
    /// waterplane is hers (a superellipse, finer forward than aft), so the sea about her bow and quarters stays the
    /// sea's; above it a box a little longer and wider than she is.
    Hull {
        forward: Vec3,
        up: Vec3,
        length: f32,
        beam: f32,
        bottom: f32,
        top: f32,
    },
    /// A sphere about the centre (an airframe).
    Sphere { radius: f32 },
    /// A capsule from the centre back `length` metres against `direction` (unit), `radius` round: a round, its halo and
    /// its tracer, which write depth.
    Capsule {
        direction: Vec3,
        length: f32,
        radius: f32,
    },
    /// A splash: a column `height` tall and `radius` round standing on the centre (the sea), its water thrown up from
    /// there `age_s` seconds ago, so the water at height h rises at `h / age − g age / 2` and spreads out from the
    /// column's axis at `spread` m/s. Its water writes no depth: the view ray is tested against it.
    Column {
        height: f32,
        radius: f32,
        age_s: f32,
        spread: f32,
    },
}

/// A moving body at the frozen moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MovingBody {
    /// World metres (a hull's waterline centre, a column's foot, a round's head).
    pub center: Vec3,
    /// World m/s.
    pub velocity: Vec3,
    /// World rad/s about the centre (a hull's turn: `−heading rate` about +Y).
    pub angular_velocity: Vec3,
    pub shape: MovingShape,
}

const KIND_HULL: f32 = 0.0;
const KIND_SPHERE: f32 = 1.0;
const KIND_CAPSULE: f32 = 2.0;
const KIND_COLUMN: f32 = 3.0;
/// A hull's box past her length and beam (`HULL_MARGIN` in the shader), and her waterline band's top.
const HULL_MARGIN: f32 = 1.5;
const WATERLINE_BAND: f32 = 1.5;

/// A body as the shader reads it (`Body` in `photo_shutter.wgsl`): view space, its velocity relative to the camera.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct GpuBody {
    center: [f32; 4],
    axis: [f32; 4],
    up: [f32; 4],
    extent: [f32; 4],
    velocity: [f32; 4],
    spin: [f32; 4],
}

/// Where the shutter's velocities come from.
pub(crate) enum Velocity<'a> {
    /// Photo mode's frozen moment: the bodies in view and the camera's own velocity.
    Bodies {
        shutter: &'a Shutter,
        bodies: &'a [MovingBody],
    },
    /// Play: temporal AA's motion. `motion` is the motion pass's target (`scene.motion`, its view and version: the
    /// moving surfaces' own travel in the last frame's NDC), `reproject` takes view space to the last frame's clip
    /// space (its unjittered view-projection times this frame's inverse view), `share` is the part of a frame's travel
    /// the shutter draws (its exposure over the frames' interval).
    Motion {
        motion: (&'a wgpu::TextureView, u64),
        reproject: Mat4,
        share: f32,
    },
}

/// `ShutterParams` in `photo_shutter.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShutterUniforms {
    inverse_projection: [f32; 16],
    projection: [f32; 16],
    world_velocity: [f32; 4],
    size: [f32; 2],
    tiles: [f32; 2],
    exposure: f32,
    max_radius: f32,
    body_count: u32,
    reach: i32,
    max_pairs: f32,
    pair_spacing: f32,
    near: f32,
    min_pairs: f32,
    /// [`Velocity::Motion`]'s view space to the last frame's clip space, and its share in x.
    reproject: [f32; 16],
    motion: [f32; 4],
}

/// The view-space bodies in sight of a camera with view `view`, projection half-tangents `tan` (x, y) and near plane
/// `near`, their velocities relative to a camera moving at `camera_velocity`; nearest first, at most [`MAX_BODIES`].
fn view_bodies(
    bodies: &[MovingBody],
    view: Mat4,
    tan: Vec2,
    near: f32,
    camera_velocity: Vec3,
    out: &mut Vec<GpuBody>,
) {
    out.clear();
    let to_view = |v: Vec3| view.transform_vector3(v);
    // A sphere wholly outside one of the frustum's sides (or behind its near plane) is out of sight.
    let side = |n: Vec3| n.normalize();
    let planes = [
        side(Vec3::new(1.0, 0.0, tan.x)),
        side(Vec3::new(-1.0, 0.0, tan.x)),
        side(Vec3::new(0.0, 1.0, tan.y)),
        side(Vec3::new(0.0, -1.0, tan.y)),
    ];
    let mut kept: Vec<(f32, GpuBody)> = Vec::with_capacity(bodies.len().min(MAX_BODIES));
    for body in bodies {
        let center = view.transform_point3(body.center);
        let velocity = to_view(body.velocity - camera_velocity);
        let spin = to_view(body.angular_velocity);
        let (axis, up, extent, bound) = match body.shape {
            MovingShape::Hull {
                forward,
                up,
                length,
                beam,
                bottom,
                top,
            } => {
                let reach = Vec3::new(
                    length * 0.5 + HULL_MARGIN,
                    beam * 0.5 + HULL_MARGIN,
                    bottom.abs().max(top),
                );
                (
                    to_view(forward).extend(length * 0.5),
                    to_view(up).extend(beam * 0.5),
                    [bottom, WATERLINE_BAND, top, KIND_HULL],
                    reach.length(),
                )
            }
            MovingShape::Sphere { radius } => (
                glam::Vec4::ZERO,
                glam::Vec4::new(0.0, 0.0, 0.0, radius),
                [0.0, 0.0, 0.0, KIND_SPHERE],
                radius,
            ),
            MovingShape::Capsule {
                direction,
                length,
                radius,
            } => (
                to_view(-direction).extend(length),
                glam::Vec4::new(0.0, 0.0, 0.0, radius),
                [0.0, 0.0, 0.0, KIND_CAPSULE],
                length + radius,
            ),
            MovingShape::Column {
                height,
                radius,
                age_s,
                spread,
            } => (
                to_view(Vec3::Y).extend(height),
                glam::Vec4::new(0.0, 0.0, 0.0, radius),
                [age_s, spread, 0.0, KIND_COLUMN],
                (height * height + radius * radius).sqrt(),
            ),
        };
        if center.z - bound > -near || planes.iter().any(|n| n.dot(center) > bound) {
            continue;
        }
        kept.push((
            (center.length() - bound).max(0.0),
            GpuBody {
                center: center.extend(bound).to_array(),
                axis: axis.to_array(),
                up: up.to_array(),
                extent,
                velocity: velocity.extend(0.0).to_array(),
                spin: spin.extend(0.0).to_array(),
            },
        ));
    }
    kept.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.extend(kept.into_iter().take(MAX_BODIES).map(|(_, body)| body));
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// The scene's colour and depth, the bodies buffer's and the motion target's versions a set of bind groups was built on.
type GroupVersions = (u64, u64, u64, u64);

struct ShutterTargets {
    viewport: (u32, u32),
    velocity: Target,
    /// The longest travels at a quarter of the resolution, then in each tile.
    quarter: Target,
    tiles: Target,
    neighbours: Target,
    out: Target,
    /// The bind groups, per pass, and the scene views', the bodies buffer's and the motion target's versions they were
    /// built on (the motion's 0 for photo mode's).
    groups: Option<(GroupVersions, [wgpu::BindGroup; 5])>,
}

/// The shutter's passes, targets and bodies.
pub(crate) struct ShutterPass {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    velocity: PipelineId,
    /// The velocities from temporal AA's motion ([`Velocity::Motion`]).
    motion_velocity: PipelineId,
    reduce: PipelineId,
    neighbours: PipelineId,
    gather: PipelineId,
    targets: Option<ShutterTargets>,
    /// The bodies (grown as needed) and its version.
    bodies: Option<(wgpu::Buffer, u64)>,
    next_version: u64,
    /// This frame's bodies in view space, kept for their room.
    scratch: Vec<GpuBody>,
}

impl ShutterPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(PHOTO_SHUTTER_SHADER)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo shutter"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(3),
                texture(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mut add = |label: &str, entry: &'static str| {
            pipelines.add(
                device,
                library,
                label,
                PHOTO_SHUTTER_SHADER.path,
                Vec::new(),
                fullscreen_recipe(vec![layout.clone()], "vertex", entry, FORMAT, None),
            )
        };
        let velocity = add("photo shutter velocity", "velocity")?;
        let motion_velocity = add("film camera velocity", "motion_velocity")?;
        let reduce = add("photo shutter tiles", "reduce")?;
        let neighbours = add("photo shutter neighbours", "neighbours")?;
        let gather = add("photo shutter gather", "gather")?;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo shutter uniforms"),
            size: std::mem::size_of::<ShutterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photo shutter"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layout,
            uniforms,
            sampler,
            velocity,
            motion_velocity,
            reduce,
            neighbours,
            gather,
            targets: None,
            bodies: None,
            next_version: 1,
            scratch: Vec::new(),
        })
    }

    /// Lets the targets and the bodies go (photo mode left, or its shutter closed to the frozen instant, and no film
    /// camera in play).
    pub(crate) fn release(&mut self) {
        self.targets = None;
        self.bodies = None;
    }

    fn target(device: &wgpu::Device, label: &str, size: (u32, u32), copy: bool) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | if copy {
                    wgpu::TextureUsages::COPY_SRC
                } else {
                    wgpu::TextureUsages::empty()
                },
            view_formats: &[],
        });
        Target {
            view: texture.create_view(&Default::default()),
            texture,
        }
    }

    fn ensure(&mut self, device: &wgpu::Device, viewport: (u32, u32)) {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.viewport == viewport)
        {
            return;
        }
        let quarter = (viewport.0.div_ceil(4), viewport.1.div_ceil(4));
        let tiles = (quarter.0.div_ceil(4), quarter.1.div_ceil(4));
        self.targets = Some(ShutterTargets {
            viewport,
            velocity: Self::target(device, "photo shutter velocity", viewport, false),
            quarter: Self::target(device, "photo shutter quarter", quarter, false),
            tiles: Self::target(device, "photo shutter tiles", tiles, false),
            neighbours: Self::target(device, "photo shutter neighbours", tiles, false),
            out: Self::target(device, "photo shutter out", viewport, true),
            groups: None,
        });
    }

    /// Writes the bodies in view, growing their buffer as needed.
    fn upload_bodies(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // At least one, so the binding is never empty.
        let count = self.scratch.len().max(1);
        let bytes = (count * std::mem::size_of::<GpuBody>()) as u64;
        if self
            .bodies
            .as_ref()
            .is_none_or(|(buffer, _)| buffer.size() < bytes)
        {
            self.next_version += 1;
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("photo shutter bodies"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.bodies = Some((buffer, self.next_version));
        }
        if !self.scratch.is_empty() {
            let (buffer, _) = self.bodies.as_ref().expect("just made");
            queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.scratch));
        }
    }

    /// Blurs `scene.hdr` (its texture, view and version) in place by the shutter, from `scene.depth` (view and
    /// version), under `camera` drawn with temporal AA's `jitter` (pixels) over `viewport`, with the velocities of
    /// `velocity`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        timer: &GpuTimer,
        hdr: (&wgpu::Texture, &wgpu::TextureView, u64),
        depth: (&wgpu::TextureView, u64),
        viewport: (u32, u32),
        camera: &Camera,
        jitter: Vec2,
        velocity: Velocity<'_>,
    ) {
        self.ensure(device, viewport);
        let (width, height) = (viewport.0.max(1) as f32, viewport.1.max(1) as f32);
        let aspect = width / height;
        let projection = camera.projection(aspect);
        let jittered = crate::camera::jitter_projection(projection, jitter, width, height);
        let tan_y = (camera.fov_y_degrees.to_radians() * 0.5).tan();
        let mut scratch = std::mem::take(&mut self.scratch);
        let (exposure, camera_velocity) = match &velocity {
            Velocity::Bodies { shutter, bodies } => {
                view_bodies(
                    bodies,
                    camera.view(),
                    Vec2::new(tan_y * aspect, tan_y),
                    camera.near,
                    shutter.camera_velocity,
                    &mut scratch,
                );
                (shutter.exposure_s.max(0.0), shutter.camera_velocity)
            }
            Velocity::Motion { .. } => {
                scratch.clear();
                (0.0, Vec3::ZERO)
            }
        };
        self.scratch = scratch;
        self.upload_bodies(device, queue);
        let (reproject, motion, motion_version) = match &velocity {
            Velocity::Motion {
                motion,
                reproject,
                share,
            } => (*reproject, [*share, 0.0, 0.0, 0.0], Some(*motion)),
            Velocity::Bodies { .. } => (Mat4::IDENTITY, [0.0; 4], None),
        };
        let max_radius = MAX_RADIUS_OF_HEIGHT * height;
        let tiles = (viewport.0.div_ceil(TILE), viewport.1.div_ceil(TILE));
        let uniforms = ShutterUniforms {
            inverse_projection: jittered.inverse().to_cols_array(),
            projection: projection.to_cols_array(),
            world_velocity: camera
                .view()
                .transform_vector3(-camera_velocity)
                .extend(0.0)
                .to_array(),
            size: [width, height],
            tiles: [tiles.0 as f32, tiles.1 as f32],
            exposure,
            max_radius,
            body_count: self.scratch.len() as u32,
            reach: (max_radius / TILE as f32).floor() as i32 + 1,
            max_pairs: MAX_PAIRS,
            pair_spacing: PAIR_SPACING,
            near: camera.near,
            min_pairs: MIN_PAIRS,
            reproject: reproject.to_cols_array(),
            motion,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let (hdr_texture, hdr_view, hdr_version) = hdr;
        let (depth_view, depth_version) = depth;
        let (bodies_buffer, bodies_version) = self.bodies.as_ref().expect("uploaded above");
        let targets = self.targets.as_mut().expect("just made");
        let versions = (
            hdr_version,
            depth_version,
            *bodies_version,
            motion_version.map_or(0, |(_, version)| version),
        );
        if targets
            .groups
            .as_ref()
            .is_none_or(|(built, _)| *built != versions)
        {
            let group = |aux: &wgpu::TextureView, aux2: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("photo shutter"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(hdr_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.uniforms.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(aux),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(aux2),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: bodies_buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            };
            // Each pass reads what it does not write; the velocities from the motion read it.
            let groups = [
                group(
                    motion_version.map_or(&targets.tiles.view, |(view, _)| view),
                    &targets.neighbours.view,
                ),
                group(&targets.velocity.view, &targets.neighbours.view),
                group(&targets.quarter.view, &targets.neighbours.view),
                group(&targets.velocity.view, &targets.tiles.view),
                group(&targets.velocity.view, &targets.neighbours.view),
            ];
            targets.groups = Some((versions, groups));
        }
        let (_, groups) = targets.groups.as_ref().expect("just made");
        let passes = [
            if motion_version.is_some() {
                (
                    self.motion_velocity,
                    &targets.velocity.view,
                    "film camera velocity",
                )
            } else {
                (
                    self.velocity,
                    &targets.velocity.view,
                    "photo shutter velocity",
                )
            },
            (self.reduce, &targets.quarter.view, "photo shutter quarter"),
            (self.reduce, &targets.tiles.view, "photo shutter tiles"),
            (
                self.neighbours,
                &targets.neighbours.view,
                "photo shutter neighbours",
            ),
            (self.gather, &targets.out.view, "photo shutter gather"),
        ];
        // A timestamp pair each: what the velocities, the tiles and the gather cost apart.
        for ((pipeline, target, label), group) in passes.into_iter().zip(groups) {
            let timestamp_writes = timer.render_pass(label);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes,
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(pipeline));
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        // The blurred radiance in the scene's place, for everything after.
        encoder.copy_texture_to_texture(
            targets.out.texture.as_image_copy(),
            hdr_texture.as_image_copy(),
            wgpu::Extent3d {
                width: viewport.0.max(1),
                height: viewport.1.max(1),
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> Camera {
        let mut camera = Camera::look_at(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 10.0, -100.0));
        camera.fov_y_degrees = 40.0;
        camera
    }

    fn sphere(center: Vec3) -> MovingBody {
        MovingBody {
            center,
            velocity: Vec3::new(10.0, 0.0, 0.0),
            angular_velocity: Vec3::ZERO,
            shape: MovingShape::Sphere { radius: 5.0 },
        }
    }

    #[test]
    fn bodies_out_of_sight_are_left_out_and_the_rest_go_nearest_first() {
        let camera = camera();
        let tan = Vec2::new(0.364 * 16.0 / 9.0, 0.364);
        let bodies = [
            sphere(Vec3::new(0.0, 10.0, -500.0)),
            sphere(Vec3::new(0.0, 10.0, -100.0)),
            // Behind the camera, and far off to one side.
            sphere(Vec3::new(0.0, 10.0, 100.0)),
            sphere(Vec3::new(2000.0, 10.0, -100.0)),
        ];
        let mut out = Vec::new();
        view_bodies(
            &bodies,
            camera.view(),
            tan,
            camera.near,
            Vec3::ZERO,
            &mut out,
        );
        assert_eq!(out.len(), 2);
        assert!((out[0].center[2] + 100.0).abs() < 1e-3, "{:?}", out[0]);
        assert!((out[1].center[2] + 500.0).abs() < 1e-3, "{:?}", out[1]);
        // Its velocity relative to a camera riding at the same velocity is nothing.
        view_bodies(
            &bodies,
            camera.view(),
            tan,
            camera.near,
            Vec3::new(10.0, 0.0, 0.0),
            &mut out,
        );
        assert!(
            out.iter()
                .all(|b| b.velocity[..3].iter().all(|v| v.abs() < 1e-5))
        );
    }

    #[test]
    fn a_hull_turns_into_view_space_with_her_extent() {
        let camera = camera();
        let tan = Vec2::new(0.7, 0.4);
        let hull = MovingBody {
            center: Vec3::new(0.0, 0.0, -300.0),
            velocity: Vec3::new(0.0, 0.0, -15.0),
            angular_velocity: Vec3::new(0.0, 0.02, 0.0),
            shape: MovingShape::Hull {
                forward: Vec3::NEG_Z,
                up: Vec3::Y,
                length: 114.7,
                beam: 12.0,
                bottom: -4.8,
                top: 40.0,
            },
        };
        let mut out = Vec::new();
        view_bodies(
            &[hull],
            camera.view(),
            tan,
            camera.near,
            Vec3::ZERO,
            &mut out,
        );
        let body = out[0];
        assert_eq!(body.extent, [-4.8, WATERLINE_BAND, 40.0, KIND_HULL]);
        // The camera looks along −Z, level: the bow is ahead (−Z in view space too), half her length in w.
        assert!((body.axis[2] + 1.0).abs() < 1e-5 && (body.axis[3] - 57.35).abs() < 1e-3);
        assert!((body.up[1] - 1.0).abs() < 1e-5 && (body.up[3] - 6.0).abs() < 1e-5);
        assert!((body.velocity[2] + 15.0).abs() < 1e-4);
        assert!((body.spin[1] - 0.02).abs() < 1e-6);
    }
}
