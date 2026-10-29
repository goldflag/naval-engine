//! The underwater view (the web game's `ocean/screen/underwater.ts` and the camera probe of `Ocean.update`).
//!
//! - [`UnderwaterProbe`] (the ocean's, run in its prepare while the near plane may reach the water): on a 128 × 72 grid
//!   over the screen, the depth below the waves and the wake of each pixel's near-plane point, and the camera's own depth
//!   below the waves. The surface reads the camera's to shade the back faces it sees from below as the sea's underside;
//!   the composite reads the grid for each pixel.
//! - [`UnderwaterLayer`] (a scene layer of its own, after every transparent but the aim overlay): two blended full-
//!   screen draws over the scene, the scene times the water's transmittance to the first surface behind each pixel (the
//!   renderer's backdrop depth copy, which holds the sea's surface), then the light the water scatters toward the camera
//!   and the film of water where the lens meets the surface; then the motes (`shaders/underwater_motes.wgsl`): a
//!   sparse cloud of suspended matter standing still in the world within a few metres of the lens, which streams past
//!   as the camera moves, so a boat's speed and turn read from under the sea; then the bubbles the running torpedoes
//!   leave ([`BubbleTrail`], `shaders/underwater_bubbles.wgsl`).
//!
//! The water under the camera is the water the sea shows from above: the maps' absorption swatch is its beam
//! attenuation, stretched by `UNDERWATER_CLARITY` in `underwater.wgsl` so a hull reads from 100–150 m; the daylight
//! fades with depth by the diffuse attenuation, less the forward scattering the beam loses (`underwater_ray.wgsl`). The
//! web eased the absorption to a twentieth for a camera going under, which left the water clear for a kilometre and its
//! colour unchanged with depth.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Vec3, Vec4};
use naval_render::{
    CameraUniforms, CopyPoint, DrawContext, MotionPipeline, Phase, PipelineId, PipelineRegistry,
    PrepareContext, RenderContext, SCENE_BACKDROP_DEPTH, SceneCopies, SceneLayer, ScenePipeline,
    ShaderError, compute_recipe,
};

use crate::sea_read::SeaRead;
use crate::ship_wake::WakeBindings;
use crate::{WaterColors, shaders};

/// The probe's grid over the screen.
pub const PROBE_SIZE: (u32, u32) = (128, 72);
/// The probe's texel format: depth below the surface in r, the camera's in g.
pub const PROBE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Wake heights are bounded to ±8 m; 10 cm more covers interpolation (`WAKE_BOUND`).
pub const WAKE_BOUND: f64 = 8.1;
/// Crests above twice the significant height do not occur in practice (`CREST_BOUND`).
pub const CREST_BOUND: f64 = 2.0;
/// How many metres of the map's water each metre under the camera counts as. The maps' absorption swatch is the water
/// as the sea shows it from above (visibility about 30 m in blue, 10 m in red on the North Atlantic); under the waves
/// the game wants a hull's bottom readable from 100–150 m, so the water there is clearer than the swatch: the same
/// colour, red still going first, so a distant hull turns blue-green rather than reading through air. The composite
/// dims and lights by it (`underwater.wgsl`), and the hulls light their submerged plating through the same water.
pub const UNDERWATER_CLARITY: f32 = 0.15;
/// Motes in the cube that repeats about the camera (`underwater_motes.wgsl`, its `CUBE`): about sixty in a view,
/// most of them faint.
pub const MOTES: u32 = 768;
/// The torpedoes whose bubbles are drawn at once (`underwater_bubbles.wgsl`, its `Trails`): the nearest the camera.
pub const MAX_TRAILS: usize = 16;
/// One trail's bubbles (`PER_TRAIL`), one released each [`BUBBLE_EMISSION_S`], the last [`BUBBLE_LIFETIME_S`] old: a
/// few each 10 cm of a torpedo's run, and gone by the time most have risen to the surface.
pub const BUBBLES_PER_TRAIL: u32 = 1080;
pub const BUBBLE_EMISSION_S: f64 = 1.0 / 360.0;
pub const BUBBLE_LIFETIME_S: f64 = 3.0;
/// How far from the camera a trail's bubbles are drawn (m, `FAR`), and the room their spreading and rising take
/// beside the course.
const BUBBLE_REACH: f64 = 110.0;
/// The share of an air torpedo's bubbles an oxygen torpedo's trail shows (`underwater_bubbles.wgsl`, its `Trail`'s
/// `seed.y`): its exhaust dissolves on the way up (`crate::torpedo_tracks::OXYGEN_TRACK`), leaving the propellers'
/// wash a few bubbles.
pub const OXYGEN_BUBBLES: f32 = 0.12;
const BUBBLE_ROOM: f64 = 10.0;
/// Metres more than the highest crest at which the composite asks for its depth copy a frame ahead: the copies are
/// chosen before a frame's prepare, so the margin covers a frame of the camera's travel.
const COPY_MARGIN: f64 = 25.0;

/// The highest the drawn surface can reach this frame (`Ocean.update`): the field's bound, at most twice the
/// significant height, plus the wake's.
pub fn surface_bound(max_height: f64, significant_height: f64) -> f64 {
    max_height.min(CREST_BOUND * significant_height) + WAKE_BOUND
}

/// The CPU side of the pass's uniform branch (`nearPlaneMayBeSubmerged`): whether any near-plane corner (or the camera)
/// may lie below `max_height`. False: the whole view starts above the water.
pub fn near_plane_may_be_submerged(camera: &CameraUniforms, max_height: f64) -> bool {
    if !max_height.is_finite() {
        return true;
    }
    let world = Mat4::from_cols_array(&camera.inverse_view);
    let inverse_projection = Mat4::from_cols_array(&camera.inverse_projection);
    let eye = world.w_axis.truncate();
    let forward = -world.z_axis.truncate().normalize_or_zero();
    let mut lowest = f64::from(eye.y);
    for (x, y) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
        // Any depth inside the frustum lies on the corner's ray, which then runs to the near plane.
        let on_ray = inverse_projection * Vec4::new(x, y, 0.5, 1.0);
        let corner = (world * (on_ray / on_ray.w)).truncate() - eye;
        let along = corner.dot(forward);
        if along > 0.0 {
            lowest = lowest.min(f64::from(eye.y + corner.y * camera.near / along));
        }
    }
    lowest <= max_height
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct ProbeParams {
    inverse_projection: [f32; 16],
    camera_world: [f32; 16],
    camera: [f32; 4],
}

/// The camera's height probe (see the module docs). Owned by the ocean, which runs it in its prepare.
pub struct UnderwaterProbe {
    /// Groups 0–2: the camera, the wake and the wave field `height_at` and `wake_height` read.
    sea: SeaRead,
    probe_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    probe_group: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    id: Option<PipelineId>,
    /// Whether the near plane may reach the water this frame (the probe ran and its texels are this frame's).
    pub near_surface: bool,
}

impl UnderwaterProbe {
    /// The probe writing into `probe` (the surface's, [`PROBE_SIZE`] in [`PROBE_FORMAT`]); `waves` is the wave field's
    /// group layout.
    pub(crate) fn new(
        device: &wgpu::Device,
        probe: &wgpu::TextureView,
        waves: &wgpu::BindGroupLayout,
        make: naval_render::MakePipelines<'_>,
    ) -> Self {
        let compute = wgpu::ShaderStages::COMPUTE;
        let sea = SeaRead::new(device, "ocean underwater probe");
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ocean underwater probe"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: compute,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: PROBE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean underwater probe"),
            size: std::mem::size_of::<ProbeParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let probe_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean underwater probe"),
            layout: &probe_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(probe),
                },
            ],
        });
        let layouts = sea.layouts(waves, &probe_layout);
        let pipeline = make.compute(|| {
            let module = shaders::shader_module(device, shaders::UNDERWATER_PROBE, &[]);
            let groups: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("ocean underwater probe"),
                layout: Some(
                    &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("ocean underwater probe"),
                        bind_group_layouts: &groups,
                        immediate_size: 0,
                    }),
                ),
                module: &module,
                entry_point: Some("probe_main"),
                compilation_options: Default::default(),
                cache: None,
            })
        });
        Self {
            sea,
            probe_layout,
            params,
            probe_group,
            pipeline,
            id: None,
            near_surface: false,
        }
    }

    /// Builds the probe's pipeline through the renderer's registry (hot reload).
    pub(crate) fn register(
        &mut self,
        ctx: &mut RenderContext<'_>,
        waves: &wgpu::BindGroupLayout,
    ) -> Result<(), ShaderError> {
        self.id = Some(ctx.add_pipeline(
            "ocean underwater probe",
            shaders::UNDERWATER_PROBE.path,
            Vec::new(),
            compute_recipe(self.sea.layouts(waves, &self.probe_layout), "probe_main"),
        )?);
        Ok(())
    }

    /// Takes the registry's current pipeline; whether it changed.
    pub(crate) fn adopt(&mut self, registry: &PipelineRegistry) -> bool {
        let Some(id) = self.id else {
            return false;
        };
        let pipeline = registry.compute(id);
        let changed = *pipeline != self.pipeline;
        if changed {
            self.pipeline = pipeline.clone();
        }
        changed
    }

    /// Reads `wake` (or a calm sea's) from now on.
    pub(crate) fn bind_wake(&mut self, device: &wgpu::Device, wake: Option<&WakeBindings<'_>>) {
        self.sea.bind_wake(device, wake);
    }

    /// Probes the surface under the `camera` when the near plane may reach it (below `max_height`); the field's group
    /// is this frame's. Whether it ran. The frame the near plane leaves the water's reach the probe is cleared to above
    /// the water instead, so readers that do not know whether it ran (the clouds) can always trust it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &mut naval_render::UploadSink<'_>,
        encoder: &mut wgpu::CommandEncoder,
        camera: &CameraUniforms,
        field: &wgpu::BindGroup,
        max_height: f64,
        timer: Option<&naval_render::GpuTimer>,
    ) -> bool {
        let was_near = self.near_surface;
        self.near_surface = near_plane_may_be_submerged(camera, max_height);
        if !self.near_surface && !was_near {
            return false;
        }
        // A near plane of 0 asks the shader to clear the probe.
        let near = if self.near_surface { camera.near } else { 0.0 };
        let params = ProbeParams {
            inverse_projection: camera.inverse_projection,
            camera_world: camera.inverse_view,
            camera: [
                camera.position[0],
                camera.position[1],
                camera.position[2],
                near,
            ],
        };
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&params));
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ocean underwater probe"),
            timestamp_writes: timer.and_then(|timer| timer.compute_pass("ocean underwater probe")),
        });
        pass.set_pipeline(&self.pipeline);
        self.sea.bind(device, queue, &mut pass, camera, field);
        pass.set_bind_group(3, &self.probe_group, &[]);
        pass.dispatch_workgroups(PROBE_SIZE.0.div_ceil(8), PROBE_SIZE.1.div_ceil(8), 1);
        self.near_surface
    }
}

/// The probe's texture (the surface owns it and binds it).
pub(crate) fn probe_texture(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ocean underwater probe"),
        size: wgpu::Extent3d {
            width: PROBE_SIZE.0,
            height: PROBE_SIZE.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: PROBE_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// A torpedo running in the water, for its bubbles: its id, where its tail is (between the packets, as its model is
/// drawn), its velocity, the seconds it has run (its trail goes no further back than its launch) and whether it runs on
/// oxygen ([`OXYGEN_BUBBLES`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BubbleTrail {
    pub id: i64,
    pub tail: DVec3,
    pub velocity: DVec3,
    pub run: f64,
    pub oxygen: bool,
}

/// A trail as the layer keeps it: as last seen, and when (the presentation clock). One that ended (no longer handed
/// in) is kept until its last bubbles are gone, so they rise on after a hit or a miss.
#[derive(Clone, Copy, Debug)]
struct KeptTrail {
    trail: BubbleTrail,
    seen: f64,
}

/// `Trail` in `underwater_bubbles.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct TrailUniform {
    tail: [f32; 4],
    velocity: [f32; 4],
    seed: [u32; 4],
}

/// `Trails` in `underwater_bubbles.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct TrailsUniform {
    count: [u32; 4],
    clock: [f32; 4],
    trails: [TrailUniform; MAX_TRAILS],
}

/// The trails kept at `now`: `running` as handed in, those that ended kept until their last bubbles are gone, and
/// none from before a clock that went back (another battle's).
fn keep_trails(
    kept: &mut Vec<KeptTrail>,
    now: f64,
    running: impl IntoIterator<Item = BubbleTrail>,
) {
    if kept.iter().any(|k| k.seen > now) {
        kept.clear();
    }
    for trail in running {
        match kept.iter_mut().find(|k| k.trail.id == trail.id) {
            Some(k) => *k = KeptTrail { trail, seen: now },
            None => kept.push(KeptTrail { trail, seen: now }),
        }
    }
    kept.retain(|k| now - k.seen < BUBBLE_LIFETIME_S);
}

/// The uniform for the kept trails nearest `eye` at `now` (at most [`MAX_TRAILS`], within reach), and how many.
fn trails_uniform(kept: &[KeptTrail], now: f64, eye: DVec3) -> TrailsUniform {
    let mut near: Vec<(f64, &KeptTrail)> = kept
        .iter()
        .filter_map(|k| {
            let since = now - k.seen;
            // Its bubbles lie between where its tail was `since` and `since + run` seconds back, as far as the oldest
            // still lives.
            let back = k.trail.run.min(BUBBLE_LIFETIME_S - since).max(0.0);
            let start = k.trail.tail;
            let end = start - k.trail.velocity * back;
            let along = end - start;
            let t = if along.length_squared() > 1e-9 {
                ((eye - start).dot(along) / along.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let distance = eye.distance(start + along * t);
            (distance < BUBBLE_REACH + BUBBLE_ROOM).then_some((distance, k))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut uniform = TrailsUniform::default();
    for (slot, (_, k)) in uniform.trails.iter_mut().zip(&near) {
        let t = k.trail;
        *slot = TrailUniform {
            tail: t.tail.as_vec3().extend((now - k.seen) as f32).to_array(),
            velocity: t.velocity.as_vec3().extend(t.run as f32).to_array(),
            // Its own pattern: a salvo's trails do not repeat each other; and the share of its bubbles shown.
            seed: [
                (t.id as u64)
                    .wrapping_mul(0x9e37_79b9_7f4a_7c15)
                    .rotate_right(29) as u32,
                if t.oxygen { OXYGEN_BUBBLES } else { 1.0 }.to_bits(),
                0,
                0,
            ],
        };
    }
    let ticks = now / BUBBLE_EMISSION_S;
    let newest = ticks.floor();
    uniform.count = [
        newest as i64 as u32,
        near.len().min(MAX_TRAILS) as u32,
        0,
        0,
    ];
    uniform.clock = [(ticks - newest) as f32, 0.0, 0.0, 0.0];
    uniform
}

/// `Underwater` in `underwater.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct UnderwaterUniforms {
    pigment: [f32; 4],
    absorption: [f32; 4],
    transmission: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    water: crate::optics::WaterField,
}

/// The underwater composite (see the module docs): a scene layer drawn in the transparent phase, after every effect.
pub struct UnderwaterLayer {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    probe: wgpu::TextureView,
    sampler: wgpu::Sampler,
    group: Option<wgpu::BindGroup>,
    depth_version: Option<u64>,
    dim: PipelineId,
    light: PipelineId,
    motes: PipelineId,
    bubbles: PipelineId,
    bubbles_motion: PipelineId,
    /// Whether the motes draw (on by default).
    pub show_motes: bool,
    /// The running torpedoes' trails ([`Self::set_bubble_trails`]), the clock they were handed in at, and the buffer
    /// and count of those drawn this frame.
    trails: Vec<KeptTrail>,
    trails_now: f64,
    trails_buffer: wgpu::Buffer,
    trails_drawn: u32,
    /// The water's colours (`OceanParameters::colors`, the absorption as the map sets it: the layer eases it for a
    /// camera under water itself) and the celestial light's direction and colour (brightest channel 1). Set them each
    /// frame.
    pub colors: WaterColors,
    pub water: crate::optics::WaterField,
    pub sun_direction: Vec3,
    pub sun_color: Vec3,
    /// The highest the drawn surface can reach this frame ([`surface_bound`]; the ocean's
    /// [`crate::Ocean::surface_bound`]). Set it each frame.
    pub max_height: f64,
    /// Whether the composite draws this frame, and asked for the depth copy for the next.
    active: bool,
    wants_copy: bool,
}

impl UnderwaterLayer {
    /// The composite over the ocean's probe (`Ocean::underwater_probe`).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        probe: &wgpu::TextureView,
    ) -> Result<Self, ShaderError> {
        crate::shaders::register(ctx)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        // The motes read the uniforms and the probe in their vertex stage.
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ocean underwater"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: both,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    naval_render::copies::depth_copy_layout_entry(1, fragment),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: both,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: both,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // The torpedoes' trails, read by the bubbles' vertex stage.
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline = |blend: wgpu::BlendState| ScenePipeline {
            bind_group_layouts: vec![layout.clone()],
            primitive: wgpu::PrimitiveState::default(),
            depth_write: false,
            blend: Some(blend),
            ..ScenePipeline::new("vertex", "underwater_dim")
        };
        let dim = ctx.add_scene_pipeline(
            "ocean underwater dim",
            shaders::UNDERWATER.path,
            ScenePipeline {
                fragment_entry: "underwater_dim",
                ..pipeline(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::Src,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                })
            },
        )?;
        let light = ctx.add_scene_pipeline(
            "ocean underwater light",
            shaders::UNDERWATER.path,
            ScenePipeline {
                fragment_entry: "underwater_light",
                ..pipeline(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                })
            },
        )?;
        // The motes: dots added to the scene, hidden by what stands nearer.
        let motes = ctx.add_scene_pipeline(
            "ocean underwater motes",
            shaders::UNDERWATER_MOTES.path,
            ScenePipeline {
                bind_group_layouts: vec![layout.clone()],
                primitive: wgpu::PrimitiveState::default(),
                depth_write: false,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                ..ScenePipeline::new("mote_vertex", "mote_fragment")
            },
        )?;
        // The bubbles: added as the motes are.
        let bubbles = ctx.add_scene_pipeline(
            "ocean underwater bubbles",
            shaders::UNDERWATER_BUBBLES.path,
            ScenePipeline {
                bind_group_layouts: vec![layout.clone()],
                primitive: wgpu::PrimitiveState::default(),
                depth_write: false,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                ..ScenePipeline::new("bubble_vertex", "bubble_fragment")
            },
        )?;
        // What they cover takes no history (temporal anti-aliasing's motion pass).
        let bubbles_motion = ctx.add_motion_pipeline(
            "ocean underwater bubbles motion",
            shaders::UNDERWATER_BUBBLES.path,
            MotionPipeline {
                bind_group_layouts: vec![layout.clone()],
                primitive: wgpu::PrimitiveState::default(),
                ..MotionPipeline::new("bubble_vertex", "bubble_motion")
            },
        )?;
        let trails_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean underwater bubbles"),
            size: std::mem::size_of::<TrailsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean underwater"),
            size: std::mem::size_of::<UnderwaterUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ocean underwater probe"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layout,
            uniforms,
            probe: probe.clone(),
            sampler,
            group: None,
            depth_version: None,
            dim,
            light,
            motes,
            bubbles,
            bubbles_motion,
            show_motes: true,
            trails: Vec::new(),
            trails_now: 0.0,
            trails_buffer,
            trails_drawn: 0,
            water: crate::optics::WaterField::default(),
            colors: crate::calibration::scene_parameters(
                &crate::calibration::NORTH_ATLANTIC,
                9.0,
                true,
            )
            .colors,
            sun_direction: Vec3::Y,
            sun_color: Vec3::ONE,
            max_height: f64::INFINITY,
            active: false,
            wants_copy: false,
        })
    }

    /// Whether the composite drew in the last frame.
    pub fn active(&self) -> bool {
        self.active
    }

    /// The torpedoes running in the water this frame, at `now` (the presentation clock, simulation seconds): each
    /// leaves its bubbles, and one no longer handed in leaves its last ones to rise.
    pub fn set_bubble_trails(&mut self, now: f64, running: impl IntoIterator<Item = BubbleTrail>) {
        keep_trails(&mut self.trails, now, running);
        self.trails_now = now;
    }

    /// No battle: no trails.
    pub fn clear_bubble_trails(&mut self) {
        self.trails.clear();
    }
}

impl SceneLayer for UnderwaterLayer {
    fn label(&self) -> &str {
        "underwater"
    }

    fn scene_copies(&self) -> SceneCopies {
        if self.wants_copy {
            SceneCopies::BACKDROP_DEPTH
        } else {
            SceneCopies::NONE
        }
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let camera = ctx.camera;
        self.wants_copy = near_plane_may_be_submerged(camera, self.max_height + COPY_MARGIN);
        let depth = ctx.resources.texture(SCENE_BACKDROP_DEPTH);
        self.active = near_plane_may_be_submerged(camera, self.max_height) && depth.is_some();
        let Some(depth) = depth else {
            return;
        };
        if self.depth_version != Some(depth.version) || self.group.is_none() {
            self.group = Some(ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ocean underwater"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.probe),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.trails_buffer.as_entire_binding(),
                    },
                ],
            }));
            self.depth_version = Some(depth.version);
        }
        self.trails_drawn = 0;
        if !self.active {
            return;
        }
        if !self.trails.is_empty() {
            let eye = Mat4::from_cols_array(&camera.inverse_view)
                .w_axis
                .truncate()
                .as_dvec3();
            let trails = trails_uniform(&self.trails, self.trails_now, eye);
            self.trails_drawn = trails.count[1];
            ctx.uploads
                .write_buffer(&self.trails_buffer, 0, bytemuck::bytes_of(&trails));
        }
        let colors = &self.colors;
        let rgba = |c: [f32; 3]| [c[0], c[1], c[2], 0.0];
        let uniforms = UnderwaterUniforms {
            pigment: rgba(colors.water),
            absorption: [
                colors.absorption[0],
                colors.absorption[1],
                colors.absorption[2],
                UNDERWATER_CLARITY,
            ],
            transmission: rgba(colors.transmission),
            sun_direction: self.sun_direction.extend(0.0).to_array(),
            sun_color: self.sun_color.extend(0.0).to_array(),
            water: self.water,
        };
        ctx.uploads
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
    }

    fn after_copies(&mut self, _point: CopyPoint, _ctx: &mut PrepareContext<'_>) {}

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase != Phase::Transparent || !self.active {
            return;
        }
        let Some(group) = &self.group else {
            return;
        };
        pass.set_bind_group(1, group, &[]);
        pass.set_pipeline(ctx.pipelines.render(self.dim));
        pass.draw(0..3, 0..1);
        pass.set_pipeline(ctx.pipelines.render(self.light));
        pass.draw(0..3, 0..1);
        if self.show_motes {
            pass.set_pipeline(ctx.pipelines.render(self.motes));
            pass.draw(0..6, 0..MOTES);
        }
        if self.trails_drawn > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.bubbles));
            pass.draw(0..6, 0..self.trails_drawn * BUBBLES_PER_TRAIL);
        }
    }

    /// After every layer's surface motion: the bubbles' response stands over the torpedo and the hulls behind them.
    fn draw_motion_over(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.active || self.trails_drawn == 0 {
            return;
        }
        let Some(group) = &self.group else {
            return;
        };
        pass.set_pipeline(ctx.pipelines.render(self.bubbles_motion));
        pass.set_bind_group(1, group, &[]);
        pass.draw(0..6, 0..self.trails_drawn * BUBBLES_PER_TRAIL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trail(id: i64, x: f64) -> BubbleTrail {
        BubbleTrail {
            id,
            tail: DVec3::new(x, -3.0, 0.0),
            velocity: DVec3::new(0.0, 0.0, -23.0),
            run: 10.0,
            oxygen: false,
        }
    }

    #[test]
    fn an_oxygen_torpedo_shows_a_few_of_the_bubbles() {
        let mut kept = Vec::new();
        let oxygen = BubbleTrail {
            oxygen: true,
            ..trail(2, 4.0)
        };
        keep_trails(&mut kept, 100.0, [trail(1, 0.0), oxygen]);
        let uniform = trails_uniform(&kept, 100.0, DVec3::ZERO);
        // Nearest first: the air torpedo's trail, then the oxygen torpedo's 4 m beyond it.
        assert_eq!(uniform.count[1], 2);
        assert_eq!(f32::from_bits(uniform.trails[0].seed[1]), 1.0);
        assert_eq!(f32::from_bits(uniform.trails[1].seed[1]), OXYGEN_BUBBLES);
    }

    #[test]
    fn a_trail_runs_on_its_torpedo_then_its_last_bubbles_rise_and_go() {
        let mut kept = Vec::new();
        keep_trails(&mut kept, 100.0, [trail(1, 0.0)]);
        keep_trails(&mut kept, 101.0, [trail(1, 0.0)]);
        let running = trails_uniform(&kept, 101.0, DVec3::ZERO);
        assert_eq!(running.count[1], 1);
        assert_eq!(running.trails[0].tail[3], 0.0);
        assert_eq!(running.trails[0].velocity[3], 10.0);
        // It ends: kept where it was last seen, its bubbles released no later.
        keep_trails(&mut kept, 103.0, []);
        let ended = trails_uniform(&kept, 103.0, DVec3::ZERO);
        assert_eq!(ended.count[1], 1);
        assert_eq!(ended.trails[0].tail[3], 2.0);
        // Its last bubbles are gone: so is it.
        keep_trails(&mut kept, 101.0 + BUBBLE_LIFETIME_S, []);
        assert!(kept.is_empty());
        // Another battle's clock starts again: nothing kept from the last.
        keep_trails(&mut kept, 50.0, [trail(2, 0.0)]);
        keep_trails(&mut kept, 10.0, []);
        assert!(kept.is_empty());
    }

    #[test]
    fn the_trails_drawn_are_the_nearest_within_reach() {
        let mut kept = Vec::new();
        let far = (0..MAX_TRAILS as i64 + 4).map(|id| trail(id, 20.0 * id as f64));
        keep_trails(&mut kept, 5.0, far.chain([trail(99, 5000.0)]));
        let uniform = trails_uniform(&kept, 5.0, DVec3::new(30.0, 0.0, 0.0));
        let drawn = uniform.count[1] as usize;
        assert!(drawn <= MAX_TRAILS);
        let xs: Vec<f32> = uniform.trails[..drawn].iter().map(|t| t.tail[0]).collect();
        // Nearest first, the one 5 km off never; the seeds differ.
        assert_eq!(&xs[..2], &[20.0, 40.0]);
        assert!(xs.iter().all(|&x| x < 200.0), "{xs:?}");
        assert_ne!(uniform.trails[0].seed[0], uniform.trails[1].seed[0]);
    }

    #[test]
    fn the_bubbles_clock_counts_emissions() {
        let uniform = trails_uniform(&[], 10.0 + 0.5 * BUBBLE_EMISSION_S, DVec3::ZERO);
        assert_eq!(uniform.count[0], 3600);
        assert!((uniform.clock[0] - 0.5).abs() < 1e-3);
        assert_eq!(uniform.count[1], 0);
    }

    #[test]
    fn the_near_plane_reaches_the_water_from_a_low_camera_only() {
        let camera = |y: f32, pitch: f32| {
            let eye = Vec3::new(0.0, y, 0.0);
            let target = eye + Vec3::new(0.0, pitch.sin(), -pitch.cos());
            naval_render::Camera {
                eye,
                target,
                fov_y_degrees: 55.0,
                near: 1.0,
                far: 30_000.0,
                curvature: naval_render::camera::EARTH_CURVATURE,
            }
            .uniforms(1600, 1000, naval_render::FrameTime::default())
        };
        assert!(!near_plane_may_be_submerged(&camera(30.0, 0.0), 10.0));
        assert!(near_plane_may_be_submerged(&camera(9.5, 0.0), 10.0));
        assert!(near_plane_may_be_submerged(&camera(-3.0, 0.3), 10.0));
        assert!(near_plane_may_be_submerged(
            &camera(30.0, 0.0),
            f64::INFINITY
        ));
    }
}
