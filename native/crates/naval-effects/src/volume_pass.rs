//! The GPU half of the trial look "volume": each body's grids as 3D textures and the pass that marches them
//! (`shaders/volume.wgsl`) in the transparent phase, where the smoke columns' sprites draw. `volume.rs` is the CPU
//! half. Nothing is allocated, uploaded or drawn until a frame holds a live body, which only the trial publishes.

use bytemuck::{Pod, Zeroable, bytes_of};
use naval_render::{
    CameraUniforms, DrawContext, PipelineId, RenderContext, ScenePipeline, ShaderError,
};

use crate::gpu::{sampler, sampler_entry, texture_entry, uniform_entry};
use crate::volume::{BODIES, BodyKind, Grid, MAX_DIM, VolumeBody, VolumeFrame};

const VOLUME: naval_render::ShaderFile = naval_render::shader_file!("shaders/volume.wgsl");

/// The march's steps: through smoke a fraction of a puff's size, through the box's empty voxels this many coarse
/// voxels, and the most a ray takes.
const FINE_STEP: f32 = 0.08;
const COARSE_STEP: f32 = 1.0;
const MAX_STEPS: f32 = 160.0;
/// Metres a second a column's broad billows climb (its smoke's speed aloft), and the distance the scroll wraps at: a
/// whole number of every noise period.
const RISE_REFERENCE: f64 = 3.2;
const NOISE_WRAP: f64 = 4096.0;

/// `VolumeBody` in `shaders/volume.wgsl`; keep the two in order.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BodyUniform {
    /// The coarse grid's minimum corner (world m) and its voxel's edge (m).
    origin: [f32; 4],
    /// The coarse grid's voxels in use along each axis, and the kind (0 a column, 1 a detonation).
    dims: [f32; 4],
    /// The coarse grid's and the light grid's textures' size in voxels, and a detonation's size (m).
    grid: [f32; 4],
    /// The fine grid: its minimum corner and voxel, its voxels in use (none: no fine grid), its texture's size.
    near_origin: [f32; 4],
    near_dims: [f32; 4],
    near_grid: [f32; 4],
    /// Where the noise is anchored (world m), and the body's clock (s).
    anchor: [f32; 4],
    /// The wind (x, z; m/s) the smoke leans with, in y how far a column's broad billows have climbed (m), in w how long
    /// a detonation has burned (s).
    flow: [f32; 4],
    /// 1 when the lens is inside the box (a full-screen proxy), the short step (of a puff's size), the long step
    /// (coarse voxels), the most steps.
    march: [f32; 4],
}

/// A grid's texture and its size in voxels.
type Held = Option<(wgpu::Texture, [u32; 3])>;

struct Slot {
    uniform: wgpu::Buffer,
    far: Held,
    near: Held,
    light: Held,
    group: Option<wgpu::BindGroup>,
    distance: f32,
}

pub(crate) struct VolumePass {
    pipeline: PipelineId,
    occluder: PipelineId,
    response: PipelineId,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    slots: Vec<Slot>,
    /// The live slots, furthest first.
    order: Vec<usize>,
}

fn blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// Makes `held` hold `dims` voxels, remaking the texture when it has outgrown it (rounded up, so a body growing a
/// little does not remake it every frame); true when it was remade.
fn ensure(device: &wgpu::Device, label: &str, held: &mut Held, dims: [usize; 3]) -> bool {
    let size = dims.map(|n| (n.div_ceil(16) * 16).clamp(16, MAX_DIM) as u32);
    if held
        .as_ref()
        .is_some_and(|(_, has)| (0..3).all(|axis| has[axis] >= size[axis]))
    {
        return false;
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: size[2],
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    *held = Some((texture, size));
    true
}

/// Writes `dims` voxels of RGBA8 into the corner of a grid's texture.
fn write(queue: &wgpu::Queue, held: &Held, dims: [usize; 3], data: &[u8]) {
    let [x, y, z] = dims.map(|n| n as u32);
    if x * y * z == 0 {
        return;
    }
    let (texture, _) = held.as_ref().expect("ensured");
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(x * 4),
            rows_per_image: Some(y),
        },
        wgpu::Extent3d {
            width: x,
            height: y,
            depth_or_array_layers: z,
        },
    );
}

impl VolumePass {
    /// Registers the shader and its pipelines over the effect layer's lighting (group 1) and scene reads (group 3),
    /// and for the occlusion map over its own camera.
    pub(crate) fn new(
        ctx: &mut RenderContext<'_>,
        lighting_layout: &wgpu::BindGroupLayout,
        reads_layout: &wgpu::BindGroupLayout,
        occlusion_camera_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, ShaderError> {
        ctx.add_shader(VOLUME)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let d3 = wgpu::TextureViewDimension::D3;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("effect volume"),
                entries: &[
                    uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                    texture_entry(1, fragment, d3, true),
                    texture_entry(2, fragment, d3, true),
                    texture_entry(3, fragment, d3, true),
                    texture_entry(4, fragment, d3, true),
                    sampler_entry(5, fragment),
                    sampler_entry(6, fragment),
                ],
            });
        let pipeline = ctx.add_scene_pipeline(
            "effect volumes",
            VOLUME.path,
            ScenePipeline {
                bind_group_layouts: vec![
                    lighting_layout.clone(),
                    layout.clone(),
                    reads_layout.clone(),
                ],
                depth_write: false,
                blend: Some(blend()),
                defs: vec![(
                    "EFFECT_SKY".to_owned(),
                    naval_render::ShaderDefValue::Bool(true),
                )],
                // The box's front faces, or a triangle over the screen with the lens inside it.
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        // The bodies in the motion pass: less of temporal anti-aliasing's history where their billows move.
        let response = ctx.add_motion_pipeline(
            "effect volume response",
            VOLUME.path,
            naval_render::taa::MotionPipeline {
                bind_group_layouts: vec![
                    lighting_layout.clone(),
                    layout.clone(),
                    reads_layout.clone(),
                ],
                defs: vec![(
                    "EFFECT_SKY".to_owned(),
                    naval_render::ShaderDefValue::Bool(true),
                )],
                ..naval_render::taa::MotionPipeline::new("vertex", "response")
            },
        )?;
        // The bodies in the smoke occlusion map, beside the gas (`layer.rs`'s occluder pipeline: its camera at group 0,
        // two targets, the fronts kept by max and the depths added).
        let occluder_layouts = [
            occlusion_camera_layout.clone(),
            lighting_layout.clone(),
            layout.clone(),
        ];
        let occluder = ctx.add_pipeline(
            "effect volume occlusion",
            VOLUME.path,
            Vec::new(),
            Box::new(move |build| {
                let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                    occluder_layouts.iter().map(Some).collect();
                let layout = build
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(build.label),
                        bind_group_layouts: &groups,
                        immediate_size: 0,
                    });
                let target = |operation: wgpu::BlendOperation| {
                    let component = wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation,
                    };
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState {
                            color: component,
                            alpha: component,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                };
                naval_render::Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some("vertex"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState {
                            cull_mode: Some(wgpu::Face::Back),
                            ..Default::default()
                        },
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some("occluder"),
                            compilation_options: Default::default(),
                            targets: &[
                                target(wgpu::BlendOperation::Max),
                                target(wgpu::BlendOperation::Add),
                            ],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            }),
        )?;
        let sampler = sampler(
            ctx.device,
            "effect volume",
            wgpu::AddressMode::ClampToEdge,
            false,
            1,
        );
        Ok(Self {
            pipeline,
            occluder,
            response,
            layout,
            sampler,
            slots: Vec::new(),
            order: Vec::new(),
        })
    }

    /// Uploads this frame's bodies. `noise` is the tiling Worley volume the fire shaders read, with its sampler.
    pub(crate) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        camera: &CameraUniforms,
        frame: &VolumeFrame,
        noise: &wgpu::TextureView,
        noise_sampler: &wgpu::Sampler,
    ) {
        self.order.clear();
        if frame.bodies.iter().all(|body| !body.live) {
            return;
        }
        if self.slots.is_empty() {
            self.slots = (0..BODIES)
                .map(|_| Slot {
                    uniform: device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("effect volume"),
                        size: std::mem::size_of::<BodyUniform>() as u64,
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    }),
                    far: None,
                    near: None,
                    light: None,
                    group: None,
                    distance: 0.0,
                })
                .collect();
        }
        let eye = glam::Vec3::from_array(camera.position);
        for (index, body) in frame.bodies.iter().enumerate() {
            if !body.live {
                continue;
            }
            let slot = &mut self.slots[index];
            let remade = ensure(device, "effect volume grid", &mut slot.far, body.far.dims)
                | ensure(
                    device,
                    "effect volume fine grid",
                    &mut slot.near,
                    body.near.dims,
                )
                | ensure(
                    device,
                    "effect volume light",
                    &mut slot.light,
                    body.far.dims,
                );
            write(queue, &slot.far, body.far.dims, &body.far.texels);
            write(queue, &slot.near, body.near.dims, &body.near.texels);
            write(queue, &slot.light, body.far.dims, &body.light);
            if remade || slot.group.is_none() {
                let view = |held: &Held| {
                    held.as_ref()
                        .expect("ensured")
                        .0
                        .create_view(&wgpu::TextureViewDescriptor::default())
                };
                fn texture(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
                    wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    }
                }
                let (far, near, light) = (view(&slot.far), view(&slot.near), view(&slot.light));
                slot.group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("effect volume"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: slot.uniform.as_entire_binding(),
                        },
                        texture(1, &far),
                        texture(2, &near),
                        texture(3, &light),
                        texture(4, noise),
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(noise_sampler),
                        },
                    ],
                }));
            }
            let size = |held: &Held| held.as_ref().expect("ensured").1;
            let uniform = body_uniform(body, size(&slot.far), size(&slot.near), eye, camera.near);
            queue.write_buffer(&slot.uniform, 0, bytes_of(&uniform));
            let far = &body.far;
            let centre = (far.origin
                + glam::DVec3::from_array(far.dims.map(|n| n as f64)) * (far.voxel * 0.5))
                .as_vec3();
            slot.distance = centre.distance(eye);
            self.order.push(index);
        }
        let slots = &self.slots;
        self.order
            .sort_by(|&a, &b| slots[b].distance.total_cmp(&slots[a].distance));
    }

    /// Whether any body is marched this frame.
    pub(crate) fn live(&self) -> bool {
        !self.order.is_empty()
    }

    /// Draws the bodies into the smoke occlusion map's pass. Its camera (group 0) and the lighting (group 1) are bound.
    pub(crate) fn draw_occluders(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &naval_render::PipelineRegistry,
    ) {
        if self.order.is_empty() {
            return;
        }
        pass.set_pipeline(pipelines.render(self.occluder));
        for &index in &self.order {
            if let Some(group) = &self.slots[index].group {
                pass.set_bind_group(2, group, &[]);
                pass.draw(0..36, 0..1);
            }
        }
    }

    /// Draws the bodies' responses into the motion pass, over the effect layer's lighting and scene reads.
    pub(crate) fn draw_responses(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
        lighting: &wgpu::BindGroup,
        reads: &wgpu::BindGroup,
    ) {
        if self.order.is_empty() {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(self.response));
        pass.set_bind_group(1, lighting, &[]);
        pass.set_bind_group(3, reads, &[]);
        for &index in &self.order {
            if let Some(group) = &self.slots[index].group {
                pass.set_bind_group(2, group, &[]);
                pass.draw(0..36, 0..1);
            }
        }
    }

    /// Draws the bodies, furthest first. The lighting (group 1) and the scene reads (group 3) are bound.
    pub(crate) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if self.order.is_empty() {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(self.pipeline));
        for &index in &self.order {
            let Some(group) = &self.slots[index].group else {
                continue;
            };
            pass.set_bind_group(2, group, &[]);
            pass.draw(0..36, 0..1);
        }
    }
}

/// A grid's corner and voxel, its voxels in use and its texture's size as the shader takes them.
fn grid_uniform(grid: &Grid, texture: [u32; 3]) -> ([f32; 4], [f32; 3], [f32; 3]) {
    let origin = grid.origin.as_vec3();
    (
        [origin.x, origin.y, origin.z, grid.voxel as f32],
        grid.dims.map(|n| n as f32),
        texture.map(|n| n as f32),
    )
}

fn body_uniform(
    body: &VolumeBody,
    far_texture: [u32; 3],
    near_texture: [u32; 3],
    eye: glam::Vec3,
    near_plane: f32,
) -> BodyUniform {
    let (origin, dims, grid) = grid_uniform(&body.far, far_texture);
    let (near_origin, near_dims, near_grid) = grid_uniform(&body.near, near_texture);
    let voxel = origin[3];
    let low = glam::Vec3::new(origin[0], origin[1], origin[2]);
    let high = low + glam::Vec3::from_array(dims) * voxel;
    // With the lens in the box, or so near it that the near plane would cut its faces, the proxy is the whole screen.
    let margin = glam::Vec3::splat(near_plane * 4.0 + voxel);
    let inside = eye.cmpgt(low - margin).all() && eye.cmplt(high + margin).all();
    let detonation = body.kind == BodyKind::Detonation;
    // A column's noise is laid in the world: its anchor is a whole number of noise periods.
    let anchor = if detonation {
        body.anchor
    } else {
        let snap = |v: f64| (v / NOISE_WRAP).floor() * NOISE_WRAP;
        glam::DVec3::new(snap(body.far.origin.x), 0.0, snap(body.far.origin.z))
    };
    BodyUniform {
        origin,
        dims: [dims[0], dims[1], dims[2], f32::from(u8::from(detonation))],
        grid: [grid[0], grid[1], grid[2], body.size as f32],
        near_origin,
        near_dims: [near_dims[0], near_dims[1], near_dims[2], 0.0],
        near_grid: [near_grid[0], near_grid[1], near_grid[2], 0.0],
        anchor: [
            anchor.x as f32,
            anchor.y as f32,
            anchor.z as f32,
            (body.time % 3600.0) as f32,
        ],
        flow: [
            body.wind.x as f32,
            ((body.time * RISE_REFERENCE) % NOISE_WRAP) as f32,
            body.wind.z as f32,
            body.age as f32,
        ],
        march: [
            f32::from(u8::from(inside)),
            FINE_STEP,
            COARSE_STEP,
            MAX_STEPS,
        ],
    }
}

const _: () = assert!(std::mem::size_of::<BodyUniform>() == 144);
