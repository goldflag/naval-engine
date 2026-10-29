//! A scene that exercises the renderer end to end: a ruled sea plane (opaque,
//! depth writing), a lit hull-sized box that turns slowly (opaque, a vertex
//! buffer and a bind group of its own) that casts into the sun's shadow maps,
//! and a sky gradient at the far plane (the Sky phase). It is also the smallest
//! example of a [`SceneLayer`].

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

use crate::camera::{Camera, FrameTime};
use crate::frame::{
    DrawContext, Phase, PrepareContext, RenderContext, SceneLayer, ScenePipeline, VertexLayout,
};
use crate::pipelines::PipelineId;
use crate::shaders::ShaderError;
use crate::shadows::{CasterPipeline, CasterSphere, ShadowMap, Sun};
use crate::taa::MotionPipeline;

/// The scene's WGSL: one module and three entry shaders.
pub mod files {
    use crate::ShaderFile;

    pub const COMMON: ShaderFile = crate::shader_file!("shaders/test_scene/common.wgsl");
    pub const SKY: ShaderFile = crate::shader_file!("shaders/test_scene/sky.wgsl");
    pub const GRID: ShaderFile = crate::shader_file!("shaders/test_scene/grid.wgsl");
    pub const BOX: ShaderFile = crate::shader_file!("shaders/test_scene/box.wgsl");
    pub const ALL: [ShaderFile; 4] = [COMMON, SKY, GRID, BOX];
}

/// Beam, height and length of the box, in meters: about a cruiser's hull.
pub const BOX_SIZE: Vec3 = Vec3::new(18.0, 12.0, 60.0);
/// How far the box sits below the waterline.
pub const BOX_DRAFT: f32 = 3.0;
/// The box's turn rate, radians per second.
pub const BOX_TURN_RATE: f32 = 0.1;
/// Toward the sun, as `SUN_DIRECTION` in `shaders/test_scene/common.wgsl`: 20
/// degrees up, azimuth 100 (clockwise from −Z).
pub const SUN_DIRECTION: Vec3 = Vec3::new(0.925416, 0.342020, 0.163176);

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniforms {
    model: [f32; 16],
    normal: [f32; 16],
    albedo: [f32; 4],
    /// The model matrix a frame earlier, for temporal AA's motion pass.
    previous: [f32; 16],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
}

pub struct TestScene {
    seed: u64,
    sky: PipelineId,
    grid: PipelineId,
    box_pipeline: PipelineId,
    box_caster: PipelineId,
    box_motion: PipelineId,
    draws_sky: bool,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    model: wgpu::Buffer,
    model_group: wgpu::BindGroup,
}

impl TestScene {
    pub const LABEL: &'static str = "test scene";

    pub fn new(ctx: &mut RenderContext<'_>, seed: u64) -> Result<Self, ShaderError> {
        for file in files::ALL {
            ctx.add_shader(file)?;
        }
        let device = ctx.device;

        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("test scene box"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let model = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test scene box"),
            size: std::mem::size_of::<ModelUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let model_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test scene box"),
            layout: &model_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: model.as_entire_binding(),
            }],
        });

        let (vertex_data, index_data) = unit_box();
        let vertices = buffer_with(
            device,
            "test scene box vertices",
            bytemuck::cast_slice(&vertex_data),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer_with(
            device,
            "test scene box indices",
            bytemuck::cast_slice(&index_data),
            wgpu::BufferUsages::INDEX,
        );

        let sky = ctx.add_scene_pipeline(
            "test scene sky",
            files::SKY.path,
            ScenePipeline {
                primitive: wgpu::PrimitiveState::default(),
                depth_write: false,
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let grid = ctx.add_scene_pipeline(
            "test scene grid",
            files::GRID.path,
            ScenePipeline {
                primitive: wgpu::PrimitiveState::default(),
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let box_pipeline = ctx.add_scene_pipeline(
            "test scene box",
            files::BOX.path,
            ScenePipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3].to_vec(),
                }],
                bind_group_layouts: vec![model_layout.clone()],
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let box_motion = ctx.add_motion_pipeline(
            "test scene box motion",
            files::BOX.path,
            MotionPipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: wgpu::vertex_attr_array![0 => Float32x3].to_vec(),
                }],
                bind_group_layouts: vec![model_layout.clone()],
                ..MotionPipeline::new("motion_vertex", "motion_fragment")
            },
        )?;
        let box_caster = ctx.add_caster_pipeline(
            "test scene box caster",
            files::BOX.path,
            CasterPipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: wgpu::vertex_attr_array![0 => Float32x3].to_vec(),
                }],
                bind_group_layouts: vec![model_layout],
                ..CasterPipeline::new("caster_vertex")
            },
        )?;

        Ok(Self {
            seed,
            sky,
            grid,
            box_pipeline,
            box_caster,
            box_motion,
            draws_sky: true,
            vertices,
            indices,
            index_count: index_data.len() as u32,
            model,
            model_group,
        })
    }

    /// The view the harness opens on: the box's bow quarter from a little above.
    pub fn default_camera() -> Camera {
        Camera::orbit(Vec3::new(0.0, 4.0, 0.0), 35.0, 12.0, 110.0)
    }

    /// The box's heading at `time`: a seed-chosen start within ±0.5 rad, turning
    /// at [`BOX_TURN_RATE`].
    pub fn box_yaw(&self, time: FrameTime) -> f32 {
        let unit = (splitmix64(self.seed) >> 40) as f32 / (1u64 << 24) as f32;
        (unit - 0.5) + BOX_TURN_RATE * time.seconds as f32
    }

    /// Whether the scene draws its own gradient sky (on by default); off under
    /// another layer's sky.
    pub fn set_draws_sky(&mut self, draws_sky: bool) {
        self.draws_sky = draws_sky;
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The sky's mean radiance for the eye adaptation, as the web reads it from its
    /// environment's roughest level (`pmremTexture(env, up, 1)`): this scene's sky
    /// averaged over the sphere with the 0.582-radian Gaussian about the zenith that
    /// three's roughest PMREM level applies.
    pub fn sky_mean() -> [f32; 3] {
        const SIGMA: f32 = 0.582;
        let (mut sum, mut weights) = (Vec3::ZERO, 0.0f32);
        let (rings, segments) = (90, 72);
        for ring in 0..rings {
            let theta = (ring as f32 + 0.5) / rings as f32 * std::f32::consts::PI;
            let weight = (-theta * theta / (2.0 * SIGMA * SIGMA)).exp() * theta.sin();
            for segment in 0..segments {
                let phi = (segment as f32 + 0.5) / segments as f32 * std::f32::consts::TAU;
                let ray = Vec3::new(
                    theta.sin() * phi.cos(),
                    theta.cos(),
                    theta.sin() * phi.sin(),
                );
                sum += sky_radiance(ray) * weight;
                weights += weight;
            }
        }
        (sum / weights).to_array()
    }

    /// The scene's sun for the renderer's shadow maps: the direction its shaders
    /// light with, over the box.
    pub fn sun() -> Sun {
        let mut sun = Sun::new(SUN_DIRECTION, Vec3::ZERO);
        sun.casters = vec![CasterSphere {
            center: Vec3::new(0.0, BOX_SIZE.y * 0.5 - BOX_DRAFT, 0.0),
            radius: BOX_SIZE.length() * 0.5,
        }];
        sun
    }
}

impl SceneLayer for TestScene {
    fn label(&self) -> &str {
        Self::LABEL
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let model_at = |time: FrameTime| {
            Mat4::from_translation(Vec3::new(0.0, BOX_SIZE.y * 0.5 - BOX_DRAFT, 0.0))
                * Mat4::from_rotation_y(self.box_yaw(time))
                * Mat4::from_scale(BOX_SIZE)
        };
        let model = model_at(ctx.time);
        // The frame before, on the same clock.
        let previous = model_at(FrameTime {
            seconds: ctx.time.seconds - f64::from(ctx.time.dt),
            ..ctx.time
        });
        let uniforms = ModelUniforms {
            model: model.to_cols_array(),
            normal: model.inverse().transpose().to_cols_array(),
            albedo: [0.42, 0.42, 0.45, 1.0],
            previous: previous.to_cols_array(),
        };
        ctx.queue
            .write_buffer(&self.model, 0, bytemuck::bytes_of(&uniforms));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        match phase {
            Phase::Opaque => {
                pass.set_pipeline(ctx.pipelines.render(self.grid));
                pass.draw(0..6, 0..1);
                pass.set_pipeline(ctx.pipelines.render(self.box_pipeline));
                pass.set_bind_group(1, &self.model_group, &[]);
                pass.set_vertex_buffer(0, self.vertices.slice(..));
                pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..self.index_count, 0, 0..1);
            }
            Phase::Sky if self.draws_sky => {
                pass.set_pipeline(ctx.pipelines.render(self.sky));
                pass.draw(0..3, 0..1);
            }
            Phase::Sky | Phase::Water | Phase::Transparent => {}
        }
    }

    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        pass.set_pipeline(ctx.pipelines.render(self.box_motion));
        pass.set_bind_group(1, &self.model_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }

    fn draw_shadow(
        &self,
        _map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        pass.set_pipeline(ctx.pipelines.render(self.box_caster));
        pass.set_bind_group(1, &self.model_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

fn buffer_with(
    device: &wgpu::Device,
    label: &str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents,
        usage,
    })
}

/// A unit cube about the origin, four vertices per face so each face has its
/// own normal, wound counter-clockwise seen from outside.
fn unit_box() -> (Vec<Vertex>, Vec<u16>) {
    let faces: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
        // normal, u axis, v axis (u × v = normal)
        ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
        ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
        ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
        ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, u, v) in faces {
        let (n, u, v) = (Vec3::from(normal), Vec3::from(u), Vec3::from(v));
        debug_assert!((u.cross(v) - n).length() < 1e-6);
        let base = vertices.len() as u16;
        for (su, sv) in [(-0.5, -0.5), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)] {
            vertices.push(Vertex {
                position: (n * 0.5 + u * su + v * sv).to_array(),
                normal,
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// `sky_radiance` in `shaders/test_scene/common.wgsl`, for the eye adaptation's sky mean.
pub fn sky_radiance(ray: Vec3) -> Vec3 {
    const SUN_RADIANCE: Vec3 = Vec3::new(2.6, 2.4, 2.1);
    const ZENITH: Vec3 = Vec3::new(0.035, 0.09, 0.26);
    const HORIZON: Vec3 = Vec3::new(0.38, 0.46, 0.58);
    let smoothstep = |edge0: f32, edge1: f32, x: f32| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let up = ray.y;
    let mut sky = HORIZON.lerp(ZENITH, up.clamp(0.0, 1.0).powf(0.45));
    sky = sky.lerp(HORIZON * 0.55, smoothstep(0.0, -0.08, up));
    let mu = ray.dot(SUN_DIRECTION).max(0.0);
    sky += SUN_RADIANCE * (0.08 * mu.powf(12.0) + 0.25 * mu.powf(300.0));
    sky += SUN_RADIANCE * 40.0 * smoothstep(0.999962, 0.999976, mu);
    sky
}

/// A small, well-mixed hash so neighbouring seeds look unrelated.
pub fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
