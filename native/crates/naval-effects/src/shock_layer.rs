//! The blast waves as a `naval_render::SceneLayer`: [`ShockWaves`] published for the frame's camera and drawn in
//! `shaders/shock.wgsl`, the fronts' bend of the backdrop and the cloud behind them.
//!
//! The layer asks for the backdrop's colour and depth only while a front is in the air (nothing otherwise: no copy,
//! no draw), reads them at the backdrop copy point and draws in the transparent phase. Pass it before every other
//! layer that draws transparents: it moves the backdrop, so what is drawn before it would be drawn over.
//!
//! ```ignore
//! let mut shock = ShockLayer::new(&mut renderer.context())?;
//! // each frame, after the combat effects' update:
//! shock.follow(&effects);
//! renderer.render(input, &mut [/* sea, ships, sky, */ &mut shock, &mut effects], output)?;
//! ```

use bytemuck::{Pod, Zeroable, bytes_of};
use naval_render::copies::{color_copy_layout_entry, depth_copy_layout_entry};
use naval_render::{
    CopyPoint, DrawContext, MotionPipeline, Phase, PipelineId, PrepareContext, RenderContext,
    SCENE_BACKDROP_COLOR, SCENE_BACKDROP_DEPTH, SceneCopies, SceneLayer, ScenePipeline,
    ShaderError,
};

use crate::camera::CameraView;
use crate::combat::EffectLighting;
use crate::gpu::{sampler, sampler_entry, uniform_entry};
use crate::layer::EffectsLayer;
use crate::shock::{MAX_SHOCKS, ShockInstance, ShockWaves};

const SHOCK: naval_render::ShaderFile = naval_render::shader_file!("shaders/shock.wgsl");
/// The mist's radiance from the scene's light: the share of the sun's (or moon's) radiance on smoke it scatters
/// toward the eye, and of the sky's fill.
const VEIL_SUN: f64 = 0.6;
const VEIL_SKY: f64 = 1.2;
/// Pixels of shift that take a whole frame's history from temporal AA.
const RESPONSE_PX: f32 = 2.0;
/// A camera this far under the sea's level draws none: the fronts are the air's.
const SUBMERGED_M: f64 = -0.5;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShockUniform {
    params: [f32; 4],
    radiance: [f32; 4],
    items: [ShockInstance; MAX_SHOCKS],
}

pub struct ShockLayer {
    /// The fronts drawn: the combat effects' ([`ShockLayer::follow`]), or a harness's own.
    pub waves: ShockWaves,
    pub lighting: EffectLighting,
    instances: Vec<ShockInstance>,
    uniform: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// The bind group and the backdrop's versions (depth, colour) it was built on.
    group: Option<((u64, u64), wgpu::BindGroup)>,
    pipeline: PipelineId,
    motion: PipelineId,
    /// Fronts drawn this frame.
    drawn: u32,
}

impl ShockLayer {
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        ctx.add_shader(SHOCK)?;
        let device = ctx.device.clone();
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shock"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                depth_copy_layout_entry(1, fragment),
                color_copy_layout_entry(2, fragment),
                sampler_entry(3, fragment),
            ],
        });
        // Over what is there: the backdrop's shift added, the mist laid over (premultiplied).
        let over = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = ctx.add_scene_pipeline(
            "shock fronts",
            SHOCK.path,
            ScenePipeline {
                bind_group_layouts: vec![layout.clone()],
                primitive: wgpu::PrimitiveState::default(),
                depth_write: false,
                blend: Some(wgpu::BlendState {
                    color: over,
                    alpha: over,
                }),
                ..ScenePipeline::new("shock_vertex", "shock_fragment")
            },
        )?;
        let motion = ctx.add_motion_pipeline(
            "shock fronts motion",
            SHOCK.path,
            MotionPipeline {
                bind_group_layouts: vec![layout.clone()],
                primitive: wgpu::PrimitiveState::default(),
                ..MotionPipeline::new("shock_vertex", "shock_motion")
            },
        )?;
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shock fronts"),
            size: std::mem::size_of::<ShockUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            waves: ShockWaves::default(),
            lighting: EffectLighting::default(),
            instances: Vec::with_capacity(MAX_SHOCKS),
            uniform,
            layout,
            sampler: sampler(
                &device,
                "shock backdrop",
                wgpu::AddressMode::ClampToEdge,
                false,
                1,
            ),
            group: None,
            pipeline,
            motion,
            drawn: 0,
        })
    }

    /// Takes the combat effects' fronts and light.
    pub fn follow(&mut self, effects: &EffectsLayer) {
        self.waves.follow(&effects.effects.shocks);
        self.lighting = effects.effects.lighting;
    }

    /// Fronts drawn in the last frame.
    pub fn drawn(&self) -> u32 {
        self.drawn
    }

    fn radiance(&self) -> [f32; 4] {
        let light = &self.lighting;
        let radiance = light.direct * VEIL_SUN + light.ambient * (VEIL_SKY * (1.0 + light.flash));
        [radiance.x as f32, radiance.y as f32, radiance.z as f32, 0.0]
    }
}

impl SceneLayer for ShockLayer {
    fn label(&self) -> &str {
        "shock"
    }

    fn scene_copies(&self) -> SceneCopies {
        if self.waves.is_empty() {
            SceneCopies::NONE
        } else {
            SceneCopies::BACKDROP_COLOR | SceneCopies::BACKDROP_DEPTH
        }
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        self.drawn = 0;
        self.instances.clear();
        if self.waves.is_empty() {
            return;
        }
        let camera = CameraView::from_uniforms(ctx.camera);
        if camera.position.y < SUBMERGED_M {
            return;
        }
        self.waves.publish(camera.position, &mut self.instances);
        if self.instances.is_empty() {
            return;
        }
        let mut uniform = ShockUniform::zeroed();
        uniform.params = [
            2.0 * ctx.camera.tan_half_fov_y / ctx.viewport.1.max(1) as f32,
            RESPONSE_PX,
            0.0,
            0.0,
        ];
        uniform.radiance = self.radiance();
        uniform.items[..self.instances.len()].copy_from_slice(&self.instances);
        ctx.uploads
            .write_buffer(&self.uniform, 0, bytes_of(&uniform));
    }

    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        if point != CopyPoint::Backdrop || self.instances.is_empty() {
            return;
        }
        let (Some(depth), Some(color)) = (
            ctx.resources.texture(SCENE_BACKDROP_DEPTH),
            ctx.resources.texture(SCENE_BACKDROP_COLOR),
        ) else {
            return;
        };
        let versions = (depth.version, color.version);
        if self
            .group
            .as_ref()
            .is_none_or(|(built, _)| *built != versions)
        {
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shock"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&color.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            self.group = Some((versions, group));
        }
        self.drawn = self.instances.len() as u32;
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase != Phase::Transparent || self.drawn == 0 {
            return;
        }
        if let Some((_, group)) = &self.group {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..6, 0..self.drawn);
        }
    }

    /// After every layer's surface motion: the fronts' response stands over the hulls and the sea behind them.
    fn draw_motion_over(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if self.drawn == 0 {
            return;
        }
        if let Some((_, group)) = &self.group {
            pass.set_pipeline(ctx.pipelines.render(self.motion));
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..6, 0..self.drawn);
        }
    }
}
