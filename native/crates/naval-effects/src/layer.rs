//! The effects as a `naval_render::SceneLayer`.
//!
//! `prepare` publishes the CPU simulation for the frame's camera (cull, sort, pack), uploads the instances, draws
//! the quarter-resolution smoke occlusion map from the occluding gas (before the scene pass) and refreshes the
//! soft-particle depth. `draw` draws every batch in the transparent phase, in a fixed order. With the look `merge`
//! the gas and the soot are added up at the backdrop's copy point (`after_copies`) into the merged smoke, which `draw`
//! lays over the scene where the gas batch draws (`shaders/merge.wgsl`).
//!
//! Soft-particle depth sits behind a seam ([`DepthSource`]): [`DepthSource::Backdrop`] reads render core II's
//! `scene.backdrop.depth` (the same frame's opaque and sea depth, asked for through `scene_copies`); the older
//! [`DepthSource::OwnCopy`] copies `scene.depth` in `prepare`, which then still holds the previous frame (right for a
//! still camera, one frame late for a moving one). The shaders bind a `texture_depth_2d` either way.

use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use naval_render::{
    CopyPoint, DrawContext, Phase, Pipeline, PipelineId, PrepareContext, RenderContext,
    SCENE_BACKDROP_DEPTH, SCENE_DEPTH, SceneCopies, SceneLayer, ScenePipeline, SceneView,
    ShaderError, ShadowMap, UploadSink, VertexLayout,
};

use crate::camera::CameraView;
use crate::combat::{CombatEffects, EffectsFrame};
use crate::gas_atlas::GasAtlas;
use crate::gpu::{
    GrowBuffer, depth_entry, sampler, sampler_entry, storage_entry, texture_entry, uniform_entry,
    upload_image, upload_volume,
};
use crate::instances::{FireBasis, RibbonVertex};
use crate::textures::{
    SpriteMap, effect_texture, effect_volume_texture, smoke_billow_texture, smoke_noise_texture,
};

const COMMON: naval_render::ShaderFile = naval_render::shader_file!("shaders/common.wgsl");
const WARP: naval_render::ShaderFile = naval_render::shader_file!("shaders/warp.wgsl");
/// The smoke screens' billow coordinates, which the gas sprite and the air light import.
const SMOKE_SCREEN: naval_render::ShaderFile =
    naval_render::shader_file!("shaders/smoke_screen.wgsl");
const GAS: naval_render::ShaderFile = naval_render::shader_file!("shaders/gas_sprite.wgsl");
/// Trial `flipbook`: the module the gas sprite reads a simulation's frames through.
const FLIPBOOK: naval_render::ShaderFile = naval_render::shader_file!("shaders/flipbook.wgsl");
const SPRITE: naval_render::ShaderFile = naval_render::shader_file!("shaders/sprite.wgsl");
const FIRE: naval_render::ShaderFile = naval_render::shader_file!("shaders/fire.wgsl");
const SMOKE: naval_render::ShaderFile = naval_render::shader_file!("shaders/smoke.wgsl");
const DEPTH_COPY: naval_render::ShaderFile = naval_render::shader_file!("shaders/depth_copy.wgsl");
const DEBRIS: naval_render::ShaderFile = naval_render::shader_file!("shaders/debris.wgsl");
/// Look `merge`: the merged smoke laid over the scene.
const MERGE: naval_render::ShaderFile = naval_render::shader_file!("shaders/merge.wgsl");
/// Where naval-sky's environment module is registered (the key every crate uses).
const ENVIRONMENT_PATH: &str = "naval-sky/shaders/environment.wgsl";

/// Registers naval-sky's environment module and naval-clouds' shadow module, unless another layer has:
/// `shaders/common.wgsl` imports them (under `EFFECT_SKY` and `EFFECT_CLOUD_SHADOW`, but the library resolves every
/// import when it registers the module), so register them before `common`.
pub(crate) fn register_environment(ctx: &mut RenderContext<'_>) -> Result<(), ShaderError> {
    if ctx.library.text(ENVIRONMENT_PATH).is_none() {
        // With its disk path, so it hot-reloads.
        ctx.add_shader(naval_sky::shaders::ENVIRONMENT_FILE)?;
    }
    if ctx
        .library
        .text(naval_clouds::shaders::SHADOW_FILE.path)
        .is_none()
    {
        ctx.add_shader(naval_clouds::shaders::SHADOW_FILE)?;
    }
    Ok(())
}

/// Framebuffer pixels per occlusion-map texel along each axis (`SmokeOcclusion`'s SCALE).
pub const OCCLUSION_SCALE: u32 = 4;
const OCCLUSION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const COPY_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Look `merge`: the merged smoke's sum of weighed light and opacity, and what it leaves of the scene.
const MERGED_SUM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MERGED_CLEAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// The ocean's distance fog as the effects are drawn with it (`scene.fogNode` on the web).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectFog {
    /// Linear RGB tint.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub power: f32,
    /// Distance over which the fog colour turns into the sky behind it.
    pub sky_blend_distance: f32,
    /// The sky behind the fragment: a stand-in for the sky's fog sampler (linear RGB).
    pub sky: [f32; 3],
    pub enabled: bool,
}

impl Default for EffectFog {
    fn default() -> Self {
        Self {
            color: [0.26, 0.27, 0.29],
            start: 2800.0,
            end: 17000.0,
            power: 1.4,
            sky_blend_distance: 9900.0,
            sky: [0.45, 0.55, 0.65],
            enabled: false,
        }
    }
}

impl From<naval_render::SceneFog> for EffectFog {
    /// The scene fog as the effects take it; the fog's own colour stands in for the sky behind until
    /// [`EffectsLayer::set_environment`] binds the sky's.
    fn from(fog: naval_render::SceneFog) -> Self {
        Self {
            color: fog.color,
            start: fog.start,
            end: fog.end,
            power: fog.power,
            sky_blend_distance: fog.sky_blend_distance,
            sky: fog.color,
            enabled: fog.enabled,
        }
    }
}

/// Where the soft particles' scene depth comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DepthSource {
    /// The layer's own copy of `scene.depth`, taken in `prepare` (the previous frame's).
    OwnCopy,
    /// Render core II's `scene.backdrop.depth`: this frame's opaque and sea depth, copied by the renderer before the
    /// transparents draw (the layer asks for it in `scene_copies`).
    Backdrop,
    /// No soft edges: a far-plane depth (every puff fully in front).
    None,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightingUniform {
    sun_direction: [f32; 4],
    direct: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    fog_sky: [f32; 4],
    flashes: [[f32; 4]; 4],
    /// The clouds the composite drew before the effects: the lowest layer's bottom, the highest's top (m), the sea
    /// sphere's radius (m), 1 while they are drawn.
    clouds: [f32; 4],
    /// The trial looks switched on (`Trial::uniform`).
    trial: [f32; 4],
    /// x the night's lift on what glows (`EffectLighting::lift`): the gas's heat, the embers, sparks and flashes, the hot
    /// fragments; y the fires' flames' (`EffectLighting::flames`), z the fires' light on their smoke
    /// (`EffectLighting::firelight`).
    emission: [f32; 4],
    /// rgb the colour of the flashes' light on the smoke (`EffectsFrame::flash_color`), w 1 while the occlusion map
    /// holds the fires' columns (`columns_through` in `shaders/common.wgsl`).
    flash_color: [f32; 4],
    /// x the flashes' and blasts' own glow's lift (`EffectLighting::flash_glow`).
    flash_glow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GasBatchUniform {
    /// rgb the colour of a fire's light on the batch's gas; w 1 where the flashes light it, in their own colour (the
    /// lighting's `flash_color`).
    flash_color: [f32; 4],
    flags: [f32; 4],
}

/// A gas batch the flashes light ([`GasBatchUniform::flash_color`]): the combat gas, the soot, the funnels' smoke. Until
/// 2026-10-21 these carried the flash's old colour, `#ffd29a`, which their light on the smoke kept after the flash went to
/// 2,500 K everywhere else (`native/docs/combat-effects/gun-blasts.md`, "The flash's colour on the smoke").
const FLASH_LIT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SpriteBatchUniform {
    flags: [f32; 4],
}

/// A batch of instances in a storage buffer and the bind group over it.
struct Batch {
    buffer: GrowBuffer,
    uniform: wgpu::Buffer,
    group: Option<(u64, wgpu::BindGroup)>,
    count: u32,
}

impl Batch {
    fn new(device: &wgpu::Device, label: &'static str, uniform: &[u8]) -> Self {
        let buffer = GrowBuffer::new(device, label, wgpu::BufferUsages::STORAGE, 4096);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniform.len().max(16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            uniform: uniform_buffer,
            group: None,
            count: 0,
        }
    }

    fn write<T: Pod>(&mut self, device: &wgpu::Device, queue: &mut UploadSink<'_>, items: &[T]) {
        self.buffer.write_to(device, queue, cast_slice(items));
        self.count = items.len() as u32;
    }

    /// The bind group, rebuilt when the buffer was remade.
    fn bind(&mut self, build: impl FnOnce(&wgpu::Buffer, &wgpu::Buffer) -> wgpu::BindGroup) {
        if self
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != self.buffer.version)
        {
            self.group = Some((
                self.buffer.version,
                build(&self.buffer.buffer, &self.uniform),
            ));
        }
    }

    fn group(&self) -> &wgpu::BindGroup {
        &self.group.as_ref().expect("bound in prepare").1
    }
}

struct Targets {
    size: (u32, u32),
    /// The occlusion map's size (a quarter of the viewport, rounded up).
    quarter: (u32, u32),
    /// The depth source the scene-reads group was built with.
    depth_source: DepthSource,
    fronts: wgpu::TextureView,
    depths: wgpu::TextureView,
    /// The layer's own depth copy: [`DepthSource::OwnCopy`] only (the backdrop's is the renderer's).
    depth: Option<wgpu::TextureView>,
    /// `scene.depth`'s version the copy's bind group was built on.
    source_version: u64,
    copy_group: Option<wgpu::BindGroup>,
    /// The depth view the scene reads bind (binding 3).
    reads_depth: wgpu::TextureView,
    /// The scene reads over each of the clouds' two histories (binding 7), drawn with the one the composite drew.
    reads: [wgpu::BindGroup; 2],
    /// The splash columns' cover the scene reads bind (bindings 8 and 9), as this view's columns published it.
    splash: Option<naval_render::CoverTarget>,
    /// Look `merge`: the merged smoke's targets at the viewport's size, made on the first frame that merges smoke.
    merged: Option<MergedTargets>,
}

/// Look `merge`: the merged smoke's two targets and the composite's group reading them.
struct MergedTargets {
    sum: wgpu::TextureView,
    clear: wgpu::TextureView,
    group: wgpu::BindGroup,
}

/// What the last frame drew, for telemetry and cost notes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DrawStats {
    pub gas: u32,
    pub soot: u32,
    pub fire: u32,
    pub fire_smoke: u32,
    pub fire_bodies: u32,
    pub fire_flames: u32,
    pub fire_embers: u32,
    pub funnel_billows: u32,
    /// Strike debris pieces drawn.
    pub debris: u32,
    pub funnel_ribbon_triangles: u32,
    /// Whether the occlusion map held gas this frame.
    pub occlusion: bool,
    /// Whether the smoke was merged this frame (look `merge`).
    pub merged: bool,
    /// Screens' worth of fire-smoke and flame quads drawn (the fill that sets their cost).
    pub fire_fill: f64,
}

pub struct EffectsLayer {
    /// The CPU simulation: feed it events and ships with `effects.update(…)` before rendering.
    pub effects: CombatEffects,
    /// The published frame, kept so the next is written over it.
    frame: EffectsFrame,
    pub fog: EffectFog,
    /// The sky's environment the fog reads the sky behind from ([`EffectsLayer::set_environment`]): atlas, sampler,
    /// uniform, and whether it is the sky's (a black placeholder otherwise, with `fog.sky` standing in).
    environment: (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, bool),
    /// Bumps with each [`EffectsLayer::set_environment`], so the shells know to rebind it.
    environment_version: u64,
    /// The clouds' shadow the smoke's sunlight passes through ([`EffectsLayer::set_cloud_shadow`]): map, sampler and
    /// uniforms, or a placeholder whose uniforms leave the light whole.
    cloud_shadow: (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer),
    /// Off, the occlusion map stays empty and smoke draws in the batches' order alone (for comparisons).
    pub occlusion_enabled: bool,
    /// Off, the air holds no light from the beams and star shells (`air_light.rs`; for comparisons).
    pub air_light_enabled: bool,
    pub depth_source: DepthSource,
    gas_pipeline: PipelineId,
    /// Trial `flipbook`: the fire columns' links, on the gas sprite's lean path.
    link_pipeline: PipelineId,
    /// Look `merge`: the gas and the soot into the merged smoke, and the merged smoke over the scene.
    merged_gas_pipeline: PipelineId,
    merge_pipeline: PipelineId,
    merge_layout: wgpu::BindGroupLayout,
    /// Look `merge`: whether this frame's smoke went into the merged smoke (`after_copies`), which `draw` lays over the
    /// scene in place of the batches it holds.
    merged: bool,
    occluder_pipeline: PipelineId,
    /// The fires' columns into the occlusion map's free `depths` channels (`column_occluder` in `gas_sprite.wgsl`).
    column_occluder_pipeline: PipelineId,
    sprite_pipeline: PipelineId,
    flame_pipeline: PipelineId,
    fire_smoke_pipeline: PipelineId,
    billow_pipeline: PipelineId,
    ribbon_pipeline: PipelineId,
    depth_copy_pipeline: PipelineId,
    debris_pipeline: PipelineId,
    gas_layout: wgpu::BindGroupLayout,
    sprite_layout: wgpu::BindGroupLayout,
    fire_layout: wgpu::BindGroupLayout,
    smoke_layout: wgpu::BindGroupLayout,
    debris_layout: wgpu::BindGroupLayout,
    reads_layout: wgpu::BindGroupLayout,
    copy_layout: wgpu::BindGroupLayout,
    atlas: GasAtlas,
    /// Trial `flipbook`: the simulations' frames (an empty stand-in until [`EffectsLayer::set_flipbooks`]).
    flipbooks: crate::flipbook::Flipbooks,
    flash_map: wgpu::TextureView,
    glow_map: wgpu::TextureView,
    billow_map: wgpu::TextureView,
    noise_map: wgpu::TextureView,
    volume: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    repeat_sampler: wgpu::Sampler,
    sprite_sampler: wgpu::Sampler,
    volume_sampler: wgpu::Sampler,
    noise_sampler: wgpu::Sampler,
    occlusion_sampler: wgpu::Sampler,
    lighting_buffer: wgpu::Buffer,
    lighting_group: wgpu::BindGroup,
    occlusion_camera: wgpu::Buffer,
    occlusion_camera_group: wgpu::BindGroup,
    far_depth: wgpu::TextureView,
    /// The clouds' published cover ([`EffectsLayer::set_cloud_cover`]), the histories the scene reads bind (with
    /// their version), a clear 1 × 1 standing in for them, and this frame's: the history the composite drew and the
    /// lighting's `clouds`.
    cloud_cover: Option<naval_sky::CloudCover>,
    cloud_histories: Option<(u64, [wgpu::TextureView; 2])>,
    no_clouds: wgpu::TextureView,
    cloud_frame: (usize, [f32; 4]),
    /// The splash columns' published cover ([`EffectsLayer::set_splash_cover`]); the clear 1 × 1 stands in for it.
    splash_cover: Option<naval_render::Cover>,
    targets: Option<Targets>,
    /// Preparing and drawing a second view of the frame ([`naval_render::Renderer::render_view`]), and the other
    /// view's targets (each at its own viewport's size), kept while this one's are in place.
    second: bool,
    other_targets: Option<Targets>,
    fire: Batch,
    gas: Batch,
    soot: Batch,
    fire_smoke: Batch,
    bodies: Batch,
    column: Batch,
    /// Trial `flipbook`: the funnels' exhaust as plume links.
    funnel_links: Batch,
    /// The game's look: the funnels' puffs of simulated smoke, occluding and merged as the gas is (untinted: their
    /// albedo is the plume's own).
    funnel_gas: Batch,
    flames: Batch,
    embers: Batch,
    billows: Batch,
    debris: Batch,
    /// Every piece in flight, for the sun's shadow maps.
    debris_casters: Batch,
    ribbon_vertices: GrowBuffer,
    ribbon_indices: GrowBuffer,
    ribbon_count: u32,
    stats: DrawStats,
    /// The smoke's opacity shadow map and the renderer's soft occluders, drawn in `prepare` while the `shadows` look and
    /// the sun's shadow maps are on (`shadow.rs`).
    shadows: crate::shadow::SmokeShadows,
    /// The volume trial's pass (`volume_pass.rs`): idle unless a frame holds a body.
    volumes: crate::volume_pass::VolumePass,
    /// The strike debris in the renderer's sun shadow maps (`sun_casters.rs`).
    sun_casters: crate::sun_casters::SunCasters,
    /// The searchlights' beams and the star shells in the air (`air_light.rs`): idle while none burns.
    air_light: crate::air_light::AirLight,
}

fn normal_blend() -> wgpu::BlendState {
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

fn additive_blend() -> wgpu::BlendState {
    wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::SrcAlpha,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
        alpha: wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        },
    }
}

/// The scene pipelines' shader defs: the fog reads the sky's environment bound in the scene reads (group 3, bindings 4
/// to 6), and the light reads the renderer's sun shadow maps (group 0; `puff_sunlight` and `surface_sunlight` in
/// `shaders/common.wgsl`). The occlusion pass, whose group 0 is a camera alone, is built without them.
pub(crate) fn scene_defs() -> naval_render::ShaderDefs {
    ["EFFECT_SKY", "EFFECT_SUN_SHADOW"]
        .map(|name| (name.to_owned(), naval_render::ShaderDefValue::Bool(true)))
        .to_vec()
}

/// This layer's scene pipelines' defs: [`scene_defs`], and the smoke's sunlight through the clouds' shadow its scene
/// reads bind (bindings 44 to 46, `EFFECT_CLOUD_SHADOW`; [`EffectsLayer::set_cloud_shadow`]).
fn layer_defs() -> naval_render::ShaderDefs {
    let mut defs = scene_defs();
    defs.push((
        "EFFECT_CLOUD_SHADOW".to_owned(),
        naval_render::ShaderDefValue::Bool(true),
    ));
    defs
}

/// A pipeline drawing into the smoke-occlusion map (`fronts` by the nearest of each coverage level, `depths` added up),
/// from `entries` (vertex, fragment) of the gas sprite's shader, writing the channels `writes` names in each.
fn occluder_recipe(
    layouts: [wgpu::BindGroupLayout; 3],
    entries: (&'static str, &'static str),
    writes: [wgpu::ColorWrites; 2],
) -> naval_render::PipelineRecipe {
    Box::new(move |build| {
        let groups: Vec<Option<&wgpu::BindGroupLayout>> = layouts.iter().map(Some).collect();
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(build.label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            });
        let max = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Max,
        };
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        Pipeline::Render(
            build
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: build.module,
                        entry_point: Some(entries.0),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: build.module,
                        entry_point: Some(entries.1),
                        compilation_options: Default::default(),
                        targets: &[
                            Some(wgpu::ColorTargetState {
                                format: OCCLUSION_FORMAT,
                                blend: Some(wgpu::BlendState {
                                    color: max,
                                    alpha: max,
                                }),
                                write_mask: writes[0],
                            }),
                            Some(wgpu::ColorTargetState {
                                format: OCCLUSION_FORMAT,
                                blend: Some(wgpu::BlendState {
                                    color: add,
                                    alpha: add,
                                }),
                                write_mask: writes[1],
                            }),
                        ],
                    }),
                    multiview_mask: None,
                    cache: build.cache,
                }),
        )
    })
}

fn transparent(
    vertex: &'static str,
    fragment: &'static str,
    layouts: Vec<wgpu::BindGroupLayout>,
    blend: wgpu::BlendState,
) -> ScenePipeline {
    ScenePipeline {
        bind_group_layouts: layouts,
        primitive: wgpu::PrimitiveState::default(),
        depth_write: false,
        blend: Some(blend),
        defs: layer_defs(),
        ..ScenePipeline::new(vertex, fragment)
    }
}

impl EffectsLayer {
    /// Registers the shaders, bakes the gas atlas (submitted, not waited for) and builds every pipeline.
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        Self::with_atlas(ctx, None)
    }

    /// As [`EffectsLayer::new`], with the gas atlas from a pack's bake when given (uploaded, so the layer neither
    /// builds the bake's pipelines nor waits for them); baked here when absent or malformed.
    pub fn with_atlas(
        ctx: &mut RenderContext<'_>,
        baked: Option<&crate::gas_atlas::BakedGasAtlas>,
    ) -> Result<Self, ShaderError> {
        let uploaded = baked.and_then(|b| GasAtlas::from_baked(ctx.device, ctx.queue, b));
        Self::with_gas_atlas(ctx, uploaded)
    }

    /// As [`EffectsLayer::new`], drawing from `atlas` when given (one already on this device, which layers may share:
    /// a host uploads the pack's once a process), baked here when absent.
    pub fn with_gas_atlas(
        ctx: &mut RenderContext<'_>,
        atlas: Option<GasAtlas>,
    ) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        // The gas material binds every look's textures: a device made with the default limit (16) cannot build it.
        let sampled = device.limits().max_sampled_textures_per_shader_stage;
        if sampled < crate::trial::SAMPLED_TEXTURES {
            return Err(ShaderError {
                path: "naval-effects gas material".into(),
                message: format!(
                    "the device allows {sampled} sampled textures a shader stage and the effects bind {}: request                      the device with naval_effects::trial::limits(&adapter)",
                    crate::trial::SAMPLED_TEXTURES
                ),
            });
        }
        let atlas = match atlas {
            Some(atlas) => atlas,
            None => GasAtlas::bake(ctx)?,
        };
        register_environment(ctx)?;
        // The shared module, then the smoke shadows' (which imports it) before the materials that import both.
        ctx.add_shader(COMMON)?;
        ctx.add_shader(SMOKE_SCREEN)?;
        let shadows = crate::shadow::SmokeShadows::new(ctx)?;
        let shadow_reads = crate::shadow::reads_entries();
        for file in [
            WARP, FLIPBOOK, GAS, SPRITE, FIRE, SMOKE, DEPTH_COPY, DEBRIS, MERGE,
        ] {
            ctx.add_shader(file)?;
        }
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let d2 = wgpu::TextureViewDimension::D2;
        let lighting_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect lighting"),
            entries: &[
                uniform_entry(0, vf),
                // The warp trial's eddies (`shaders/warp.wgsl`).
                texture_entry(20, fragment, d2, true),
                sampler_entry(21, fragment),
            ],
        });
        let gas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gas batch"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                uniform_entry(1, vf),
                texture_entry(2, fragment, d2, true),
                texture_entry(3, fragment, d2, true),
                texture_entry(4, fragment, d2, true),
                sampler_entry(5, fragment),
                sampler_entry(6, fragment),
                // Tactical screens use the fire's existing Worley volume for their billows.
                texture_entry(7, fragment, wgpu::TextureViewDimension::D3, true),
                sampler_entry(8, fragment),
                // Trial `flipbook` (bindings 30 to 39): a simulation's frames and the clip table.
                texture_entry(30, fragment, wgpu::TextureViewDimension::D2Array, true),
                texture_entry(31, fragment, wgpu::TextureViewDimension::D2Array, true),
                texture_entry(32, fragment, wgpu::TextureViewDimension::D2Array, true),
                sampler_entry(33, fragment),
                uniform_entry(34, vf),
            ],
        });
        let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sprite batch"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                uniform_entry(1, fragment),
                texture_entry(2, fragment, d2, true),
                sampler_entry(3, fragment),
            ],
        });
        let fire_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fire batch"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                uniform_entry(1, wgpu::ShaderStages::VERTEX),
                texture_entry(2, fragment, wgpu::TextureViewDimension::D3, true),
                sampler_entry(3, fragment),
            ],
        });
        let smoke_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("funnel smoke"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                texture_entry(1, fragment, d2, true),
                texture_entry(2, fragment, d2, true),
                sampler_entry(3, fragment),
                sampler_entry(4, fragment),
            ],
        });
        let debris_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("strike debris"),
            entries: &[storage_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let reads_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect scene reads"),
            entries: &[
                texture_entry(0, fragment, d2, true),
                texture_entry(1, fragment, d2, true),
                sampler_entry(2, fragment),
                depth_entry(3, fragment),
                // The sky's environment: a column link takes the sky's light per vertex (`link_vertex`).
                texture_entry(4, vf, d2, true),
                sampler_entry(5, vf),
                uniform_entry(6, vf),
                texture_entry(7, fragment, d2, true),
                // The splash columns' cover (`common.wgsl`'s `splashes_through`).
                texture_entry(8, fragment, d2, true),
                texture_entry(9, fragment, d2, true),
                // The smoke's opacity shadow map (`shadow.wgsl`, bindings 40 to 43), read per vertex.
                shadow_reads[0],
                shadow_reads[1],
                shadow_reads[2],
                shadow_reads[3],
                // The clouds' shadow (`common.wgsl`, `EFFECT_CLOUD_SHADOW`), read per vertex.
                texture_entry(44, wgpu::ShaderStages::VERTEX, d2, true),
                sampler_entry(45, wgpu::ShaderStages::VERTEX),
                uniform_entry(46, wgpu::ShaderStages::VERTEX),
            ],
        });
        let copy_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("effect depth copy"),
            entries: &[depth_entry(0, fragment)],
        });
        // The occlusion pass binds a camera of its own at group 0 (it runs in `prepare`, outside the scene
        // pass, where the renderer's camera group is not bound); the camera module only reads binding 0.
        let occlusion_camera_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("smoke occlusion camera"),
                entries: &[uniform_entry(0, vf)],
            });

        let gas_pipeline = ctx.add_scene_pipeline(
            "effect gas",
            GAS.path,
            transparent(
                "vertex",
                "fragment",
                vec![
                    lighting_layout.clone(),
                    gas_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        let link_pipeline = ctx.add_scene_pipeline(
            "effect column links",
            GAS.path,
            transparent(
                "link_vertex",
                "link_fragment",
                vec![
                    lighting_layout.clone(),
                    gas_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        // Look `merge`: the gas and the soot added up into the merged smoke (`merged_smoke` in common.wgsl), tested
        // against the scene's depth as the layered draws are, then laid over the scene by `merge.wgsl`.
        let add = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let multiply = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::OneMinusSrc,
            operation: wgpu::BlendOperation::Add,
        };
        let merged_targets = vec![
            Some(wgpu::ColorTargetState {
                format: MERGED_SUM_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: add,
                    alpha: add,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            }),
            Some(wgpu::ColorTargetState {
                format: MERGED_CLEAR_FORMAT,
                blend: Some(wgpu::BlendState {
                    color: multiply,
                    alpha: multiply,
                }),
                write_mask: wgpu::ColorWrites::ALL,
            }),
        ];
        let merged_gas_pipeline = ctx.add_scene_pipeline_into(
            "effect gas merged",
            GAS.path,
            transparent(
                "vertex",
                "merged",
                vec![
                    lighting_layout.clone(),
                    gas_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
            merged_targets,
        )?;
        let merge_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("merged smoke"),
            entries: &[
                texture_entry(0, fragment, d2, false),
                texture_entry(1, fragment, d2, false),
            ],
        });
        // Groups 1 and 3 as every effect pipeline has them, so the groups `draw` binds once stay bound across it.
        let merge_pipeline = ctx.add_scene_pipeline(
            "effect merged smoke",
            MERGE.path,
            transparent(
                "vertex",
                "fragment",
                vec![
                    lighting_layout.clone(),
                    merge_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        let sprite_pipeline = ctx.add_scene_pipeline(
            "effect sprites",
            SPRITE.path,
            transparent(
                "vertex",
                "fragment",
                vec![
                    lighting_layout.clone(),
                    sprite_layout.clone(),
                    reads_layout.clone(),
                ],
                additive_blend(),
            ),
        )?;
        let flame_pipeline = ctx.add_scene_pipeline(
            "fire flames",
            FIRE.path,
            transparent(
                "vertex",
                "flame",
                vec![
                    lighting_layout.clone(),
                    fire_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        let fire_smoke_pipeline = ctx.add_scene_pipeline(
            "fire smoke",
            FIRE.path,
            transparent(
                "vertex",
                "smoke",
                vec![
                    lighting_layout.clone(),
                    fire_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        let billow_pipeline = ctx.add_scene_pipeline(
            "funnel billows",
            SMOKE.path,
            transparent(
                "billow_vertex",
                "billow_fragment",
                vec![
                    lighting_layout.clone(),
                    smoke_layout.clone(),
                    reads_layout.clone(),
                ],
                normal_blend(),
            ),
        )?;
        // Debris is solid: drawn in the opaque phase, writing depth, both faces (the fragment turns the facet's normal
        // toward the camera).
        let debris_pipeline = ctx.add_scene_pipeline(
            "strike debris",
            DEBRIS.path,
            ScenePipeline {
                bind_group_layouts: vec![
                    lighting_layout.clone(),
                    debris_layout.clone(),
                    reads_layout.clone(),
                ],
                primitive: wgpu::PrimitiveState::default(),
                defs: layer_defs(),
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        // The debris into the sun's shadow maps.
        let sun_casters = crate::sun_casters::SunCasters::new(
            ctx,
            DEBRIS.path,
            &lighting_layout,
            &debris_layout,
        )?;
        let mut ribbon = transparent(
            "ribbon_vertex",
            "ribbon_fragment",
            vec![
                lighting_layout.clone(),
                smoke_layout.clone(),
                reads_layout.clone(),
            ],
            normal_blend(),
        );
        ribbon.vertex_buffers = vec![VertexLayout {
            stride: std::mem::size_of::<RibbonVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4].to_vec(),
        }];
        let ribbon_pipeline = ctx.add_scene_pipeline("funnel ribbons", SMOKE.path, ribbon)?;

        let occluder_layouts = [
            occlusion_camera_layout.clone(),
            lighting_layout.clone(),
            gas_layout.clone(),
        ];
        let occluder_pipeline = ctx.add_pipeline(
            "smoke occlusion",
            GAS.path,
            Vec::new(),
            occluder_recipe(
                occluder_layouts.clone(),
                ("vertex", "occluder"),
                [wgpu::ColorWrites::ALL; 2],
            ),
        )?;
        // The fires' columns, into the map's free `depths` channels alone (`column_occluder`): what draws after them
        // (the fires' bodies, tongues and embers) fades behind them; the gas and the columns' own links do not.
        let column_occluder_pipeline = ctx.add_pipeline(
            "fire column occlusion",
            GAS.path,
            Vec::new(),
            occluder_recipe(
                occluder_layouts,
                ("column_occluder_vertex", "column_occluder"),
                [
                    wgpu::ColorWrites::empty(),
                    wgpu::ColorWrites::BLUE | wgpu::ColorWrites::ALPHA,
                ],
            ),
        )?;
        let copy_layouts = copy_layout.clone();
        let depth_copy_pipeline = ctx.add_pipeline(
            "effect depth copy",
            DEPTH_COPY.path,
            Vec::new(),
            Box::new(move |build| {
                let layout = build
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(build.label),
                        bind_group_layouts: &[Some(&copy_layouts)],
                        immediate_size: 0,
                    });
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some("vertex"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: Some(wgpu::DepthStencilState {
                            format: COPY_FORMAT,
                            depth_write_enabled: Some(true),
                            depth_compare: Some(wgpu::CompareFunction::Always),
                            stencil: wgpu::StencilState::default(),
                            bias: wgpu::DepthBiasState::default(),
                        }),
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some("fragment"),
                            compilation_options: Default::default(),
                            targets: &[],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            }),
        )?;

        let lighting_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("effect lighting"),
            size: std::mem::size_of::<LightingUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The warp trial's eddies, bound with the lighting: every effect pipeline binds that group, the occlusion
        // pass included.
        let warp_flow = upload_image(
            &device,
            &queue,
            "effect warp flow",
            &crate::warp::flow_texture(),
        );
        let warp_sampler = sampler(
            &device,
            "effect warp flow",
            wgpu::AddressMode::Repeat,
            false,
            1,
        );
        let lighting_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect lighting"),
            layout: &lighting_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lighting_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 20,
                    resource: wgpu::BindingResource::TextureView(&warp_flow),
                },
                wgpu::BindGroupEntry {
                    binding: 21,
                    resource: wgpu::BindingResource::Sampler(&warp_sampler),
                },
            ],
        });
        let occlusion_camera = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke occlusion camera"),
            size: std::mem::size_of::<naval_render::CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let occlusion_camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke occlusion camera"),
            layout: &occlusion_camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: occlusion_camera.as_entire_binding(),
            }],
        });
        let far_depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("effect far depth"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COPY_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        // No clouds: a new texture is cleared, opacity 0.
        let no_clouds = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("effect no clouds"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        let gas_uniform = |flash: [f32; 4], soft: bool, occluder: bool, tinted: bool| {
            bytes_of(&GasBatchUniform {
                flash_color: flash,
                flags: [f32::from(soft), f32::from(occluder), 1.0, f32::from(tinted)],
            })
            .to_vec()
        };
        let sprite_uniform = |visibility: bool, alpha_test: f32, fire: bool| {
            bytes_of(&SpriteBatchUniform {
                flags: [f32::from(visibility), alpha_test, f32::from(fire), 0.0],
            })
            .to_vec()
        };
        let batch = |label: &'static str, uniform: Vec<u8>| {
            let batch = Batch::new(&device, label, &uniform);
            queue.write_buffer(&batch.uniform, 0, &uniform);
            batch
        };
        let fire_basis = vec![0u8; std::mem::size_of::<FireBasis>()];
        let environment = placeholder_environment(&device);
        let volumes = crate::volume_pass::VolumePass::new(
            ctx,
            &lighting_layout,
            &reads_layout,
            &occlusion_camera_layout,
        )?;
        let air_light = crate::air_light::AirLight::new(ctx, &lighting_layout, &reads_layout)?;
        Ok(Self {
            effects: CombatEffects::new(),
            frame: EffectsFrame::default(),
            fog: EffectFog::default(),
            environment,
            environment_version: 0,
            cloud_shadow: placeholder_cloud_shadow(&device),
            occlusion_enabled: true,
            air_light_enabled: true,
            depth_source: DepthSource::OwnCopy,
            gas_pipeline,
            occluder_pipeline,
            column_occluder_pipeline,
            link_pipeline,
            merged_gas_pipeline,
            merge_pipeline,
            merge_layout,
            merged: false,
            sprite_pipeline,
            flame_pipeline,
            fire_smoke_pipeline,
            billow_pipeline,
            ribbon_pipeline,
            depth_copy_pipeline,
            debris_pipeline,
            gas_layout,
            sprite_layout,
            fire_layout,
            smoke_layout,
            debris_layout,
            reads_layout,
            copy_layout,
            flash_map: upload_image(
                &device,
                &queue,
                "effect flash",
                &effect_texture(SpriteMap::Flash),
            ),
            glow_map: upload_image(
                &device,
                &queue,
                "effect glow",
                &effect_texture(SpriteMap::Glow),
            ),
            billow_map: upload_image(&device, &queue, "smoke billow", &smoke_billow_texture()),
            noise_map: upload_image(&device, &queue, "smoke noise", &smoke_noise_texture()),
            volume: upload_volume(&device, &queue, "fire noise", &effect_volume_texture()),
            atlas,
            flipbooks: crate::flipbook::Flipbooks::none(&device, &queue),
            atlas_sampler: sampler(
                &device,
                "gas atlas",
                wgpu::AddressMode::ClampToEdge,
                true,
                1,
            ),
            repeat_sampler: sampler(&device, "gas detail", wgpu::AddressMode::Repeat, true, 1),
            sprite_sampler: sampler(
                &device,
                "effect sprite",
                wgpu::AddressMode::ClampToEdge,
                true,
                1,
            ),
            volume_sampler: sampler(&device, "fire noise", wgpu::AddressMode::Repeat, false, 1),
            noise_sampler: sampler(&device, "smoke noise", wgpu::AddressMode::Repeat, true, 8),
            occlusion_sampler: sampler(
                &device,
                "smoke occlusion",
                wgpu::AddressMode::ClampToEdge,
                false,
                1,
            ),
            lighting_buffer,
            lighting_group,
            occlusion_camera,
            occlusion_camera_group,
            far_depth,
            cloud_cover: None,
            cloud_histories: None,
            no_clouds,
            splash_cover: None,
            cloud_frame: (0, [0.0; 4]),
            targets: None,
            second: false,
            other_targets: None,
            fire: batch("combat fire", sprite_uniform(false, 0.0, false)),
            gas: batch("combat gas", gas_uniform(FLASH_LIT, true, true, true)),
            soot: batch(
                "smouldering soot",
                gas_uniform(FLASH_LIT, true, false, true),
            ),
            fire_smoke: batch("fire smoke", fire_basis.clone()),
            // Trial `flipbook`: the fire columns as links of a simulated plume, lit from below by their fires.
            column: batch(
                "fire column",
                gas_uniform([1.0, 0.42, 0.12, 0.0], true, false, false),
            ),
            // Firelight on the bodies' soot is the colour of burning oil, deeper than a muzzle flash. Soft against the
            // scene: a flame's sprite is a flat card, and where it passes into the deck or plating it stands on it
            // fades out over its last few metres instead of ending in a straight cut. Drawn after the fires' columns,
            // it fades behind their smoke too (flags' z 2: `columns_through`).
            bodies: batch(
                "fire bodies",
                bytes_of(&GasBatchUniform {
                    flash_color: [1.0, 0.42, 0.12, 0.0],
                    flags: [1.0, 0.0, 2.0, 0.0],
                })
                .to_vec(),
            ),
            flames: batch("fire flames", fire_basis),
            embers: batch("fire embers", sprite_uniform(true, 0.01, true)),
            billows: batch("funnel billows", Vec::new()),
            funnel_links: batch(
                "funnel links",
                gas_uniform([1.0, 0.42, 0.12, 0.0], true, false, false),
            ),
            funnel_gas: batch("funnel smoke", gas_uniform(FLASH_LIT, true, true, false)),
            debris: batch("strike debris", Vec::new()),
            debris_casters: batch("strike debris casters", Vec::new()),
            ribbon_vertices: GrowBuffer::new(
                &device,
                "funnel ribbon vertices",
                wgpu::BufferUsages::VERTEX,
                65536,
            ),
            ribbon_indices: GrowBuffer::new(
                &device,
                "funnel ribbon indices",
                wgpu::BufferUsages::INDEX,
                65536,
            ),
            ribbon_count: 0,
            stats: DrawStats::default(),
            shadows,
            volumes,
            sun_casters,
            air_light,
        })
    }

    /// The air the searchlights and star shells shine through this frame (the weather's haze and rain, as the aerial
    /// perspective takes them): what their beams and glow in the air scatter (`air_light.rs`).
    pub fn set_air(&mut self, medium: crate::air_light::AirMedium) {
        self.air_light.medium = medium;
    }

    /// What the last frame drew.
    pub fn stats(&self) -> DrawStats {
        self.stats
    }

    /// What the last frame's smoke shadow map held (nothing with it off).
    pub fn shadow_stats(&self) -> crate::shadow::ShadowStats {
        self.shadows.stats
    }

    fn upload(&mut self, device: &wgpu::Device, sink: &mut UploadSink<'_>, frame: &EffectsFrame) {
        self.fire.write(device, sink, &frame.fire);
        self.gas.write(device, sink, &frame.gas);
        self.soot.write(device, sink, &frame.soot);
        self.fire_smoke.write(device, sink, &frame.fire_smoke.quads);
        self.column.write(device, sink, &frame.fire_smoke.gas);
        sink.write_buffer(
            &self.fire_smoke.uniform,
            0,
            bytes_of(&frame.fire_smoke.basis),
        );
        self.bodies.write(device, sink, &frame.fire_bodies);
        self.flames.write(device, sink, &frame.fire_flames.quads);
        sink.write_buffer(&self.flames.uniform, 0, bytes_of(&frame.fire_flames.basis));
        self.embers.write(device, sink, &frame.fire_embers);
        self.billows.write(device, sink, &frame.funnel_billows);
        self.funnel_links.write(device, sink, &frame.funnel_links);
        self.funnel_gas.write(device, sink, &frame.funnel_gas);
        self.debris.write(device, sink, &frame.debris);
        self.debris_casters
            .write(device, sink, &frame.debris_casters);
        self.ribbon_vertices
            .write_to(device, sink, cast_slice(&frame.funnel_ribbon_vertices));
        self.ribbon_indices
            .write_to(device, sink, cast_slice(&frame.funnel_ribbon_indices));
        self.ribbon_count = frame.funnel_ribbon_indices.len() as u32;

        let sprite = |batch: &mut Batch,
                      map: &wgpu::TextureView,
                      sampler: &wgpu::Sampler,
                      layout: &wgpu::BindGroupLayout| {
            batch.bind(|buffer, uniform| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("sprite batch"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(map),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                    ],
                })
            });
        };
        sprite(
            &mut self.fire,
            &self.flash_map,
            &self.sprite_sampler,
            &self.sprite_layout,
        );
        sprite(
            &mut self.embers,
            &self.glow_map,
            &self.sprite_sampler,
            &self.sprite_layout,
        );
        let atlas = &self.atlas;
        let flipbooks = &self.flipbooks;
        let (atlas_sampler, repeat_sampler, gas_layout) =
            (&self.atlas_sampler, &self.repeat_sampler, &self.gas_layout);
        let (volume, volume_sampler) = (&self.volume, &self.volume_sampler);
        for batch in [
            &mut self.gas,
            &mut self.soot,
            &mut self.bodies,
            &mut self.column,
            &mut self.funnel_links,
            &mut self.funnel_gas,
        ] {
            batch.bind(|buffer, uniform| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gas batch"),
                    layout: gas_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&atlas.sides_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(&atlas.body_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::TextureView(&atlas.detail_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: wgpu::BindingResource::Sampler(atlas_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 6,
                            resource: wgpu::BindingResource::Sampler(repeat_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 7,
                            resource: wgpu::BindingResource::TextureView(volume),
                        },
                        wgpu::BindGroupEntry {
                            binding: 8,
                            resource: wgpu::BindingResource::Sampler(volume_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 30,
                            resource: wgpu::BindingResource::TextureView(&flipbooks.sides),
                        },
                        wgpu::BindGroupEntry {
                            binding: 31,
                            resource: wgpu::BindingResource::TextureView(&flipbooks.body),
                        },
                        wgpu::BindGroupEntry {
                            binding: 32,
                            resource: wgpu::BindingResource::TextureView(&flipbooks.motion),
                        },
                        wgpu::BindGroupEntry {
                            binding: 33,
                            resource: wgpu::BindingResource::Sampler(&flipbooks.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 34,
                            resource: flipbooks.uniform.as_entire_binding(),
                        },
                    ],
                })
            });
        }
        let (volume, volume_sampler, fire_layout) =
            (&self.volume, &self.volume_sampler, &self.fire_layout);
        for batch in [&mut self.fire_smoke, &mut self.flames] {
            batch.bind(|buffer, uniform| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("fire batch"),
                    layout: fire_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: uniform.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(volume),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Sampler(volume_sampler),
                        },
                    ],
                })
            });
        }
        let debris_layout = &self.debris_layout;
        for batch in [&mut self.debris, &mut self.debris_casters] {
            batch.bind(|buffer, _| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("strike debris"),
                    layout: debris_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            });
        }
        let (billow_map, noise_map, sprite_sampler, noise_sampler, smoke_layout) = (
            &self.billow_map,
            &self.noise_map,
            &self.sprite_sampler,
            &self.noise_sampler,
            &self.smoke_layout,
        );
        self.billows.bind(|buffer, _| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("funnel smoke"),
                layout: smoke_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(billow_map),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(noise_map),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(sprite_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(noise_sampler),
                    },
                ],
            })
        });
    }

    /// Trial `flipbook`: the baked simulations its puffs are drawn from (`.flipbook` files' contents, read by the
    /// host), uploaded with their mips. Clips whose simulation is not among them draw still puffs. Returns the
    /// bytes of texture held; the error names what does not fit, and the layer keeps what it had.
    pub fn set_flipbooks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        books: &[crate::flipbook::FlipbookData],
    ) -> Result<u64, String> {
        let flipbooks = crate::flipbook::Flipbooks::new(device, queue, books)?;
        Ok(self.use_flipbooks(flipbooks))
    }

    /// Trial `flipbook`: simulations already on this device ([`crate::flipbook::Flipbooks::new`]), which layers may
    /// share (a host uploads them once a process). Returns the bytes of texture they hold.
    pub fn use_flipbooks(&mut self, flipbooks: crate::flipbook::Flipbooks) -> u64 {
        self.flipbooks = flipbooks;
        self.effects.flipbook_fire = self.flipbooks.fire;
        self.effects.flipbook_plume = self.flipbooks.plume;
        self.effects.flipbook_sizes = self.flipbooks.sizes.clone();
        // The gas batches bind them: rebuilt on the next upload.
        for batch in [
            &mut self.gas,
            &mut self.soot,
            &mut self.bodies,
            &mut self.column,
            &mut self.funnel_links,
            &mut self.funnel_gas,
        ] {
            batch.group = None;
        }
        self.flipbooks.bytes
    }

    /// Fog toward the sky's environment along each fragment's ray (the web's fog sampler; `fog.sky` stands in
    /// until this is set). Call again when the sky remakes its atlas.
    pub fn set_environment(&mut self, environment: &naval_sky::EnvironmentMap) {
        self.environment = (
            environment.view.clone(),
            environment.sampler.clone(),
            environment.uniforms.clone(),
            true,
        );
        self.environment_version += 1;
        // The scene reads are rebuilt over it with the targets on the next prepare.
        self.targets = None;
    }

    /// The clouds' shadow (naval-clouds' `CloudShadow`: its map, sampler and uniforms), which the smoke's sunlight
    /// passes through as the hulls' and the sea's does, or none (the light whole). Call again when the clouds remake it.
    pub fn set_cloud_shadow(
        &mut self,
        device: &wgpu::Device,
        shadow: Option<(&wgpu::TextureView, &wgpu::Sampler, &wgpu::Buffer)>,
    ) {
        self.cloud_shadow = match shadow {
            Some((map, sampler, uniforms)) => (map.clone(), sampler.clone(), uniforms.clone()),
            None => placeholder_cloud_shadow(device),
        };
        // The scene reads are rebuilt over it with the targets on the next prepare.
        self.targets = None;
        self.other_targets = None;
    }

    /// The sky's environment as the effects read it (atlas, sampler, uniforms, whether it is the sky's) and its version.
    pub(crate) fn environment(
        &self,
    ) -> (u64, &(wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, bool)) {
        (self.environment_version, &self.environment)
    }

    /// The occlusion map and depth copy at the viewport's size, and the scene-reads bind group over them.
    fn ensure_targets(&mut self, device: &wgpu::Device, viewport: (u32, u32)) {
        // A changed depth source remakes them too: only the own copy holds a depth texture.
        let fresh = self
            .targets
            .as_ref()
            .is_none_or(|t| t.size != viewport || t.depth_source != self.depth_source);
        if fresh {
            let (w, h) = (viewport.0.max(1), viewport.1.max(1));
            let quarter = (w.div_ceil(OCCLUSION_SCALE), h.div_ceil(OCCLUSION_SCALE));
            let target = |label: &str, size: (u32, u32), format: wgpu::TextureFormat| {
                device
                    .create_texture(&wgpu::TextureDescriptor {
                        label: Some(label),
                        size: wgpu::Extent3d {
                            width: size.0,
                            height: size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            let fronts = target("smoke occlusion fronts", quarter, OCCLUSION_FORMAT);
            let depths = target("smoke occlusion depths", quarter, OCCLUSION_FORMAT);
            let depth = (self.depth_source == DepthSource::OwnCopy)
                .then(|| target("effect depth copy", (w, h), COPY_FORMAT));
            let reads_depth = self.reads_depth(depth.as_ref()).clone();
            let reads = self.reads_group(device, &fronts, &depths, &reads_depth, None);
            self.targets = Some(Targets {
                size: viewport,
                quarter,
                depth_source: self.depth_source,
                fronts,
                depths,
                depth,
                source_version: u64::MAX,
                copy_group: None,
                reads_depth,
                reads,
                splash: None,
                merged: None,
            });
        }
    }

    /// Look `merge`: whether this frame's gas and soot go into the merged smoke. It needs the backdrop's copy point,
    /// where the merged smoke is added up against this frame's depth (`DepthSource::Backdrop`, the client's); elsewhere
    /// the smoke is layered.
    fn merging(&self) -> bool {
        self.effects.trial.merge
            && self.depth_source == DepthSource::Backdrop
            && self.gas.count + self.soot.count + self.funnel_gas.count > 0
    }

    /// Look `merge`: the merged smoke's targets, made at the viewport's size on the first frame that merges smoke.
    fn ensure_merged(&mut self, device: &wgpu::Device) {
        let Some(targets) = self.targets.as_mut() else {
            return;
        };
        if targets.merged.is_some() {
            return;
        }
        let (w, h) = (targets.size.0.max(1), targets.size.1.max(1));
        let target = |label: &str, format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let sum = target("merged smoke sum", MERGED_SUM_FORMAT);
        let clear = target("merged smoke clear", MERGED_CLEAR_FORMAT);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("merged smoke"),
            layout: &self.merge_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&sum),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&clear),
                },
            ],
        });
        targets.merged = Some(MergedTargets { sum, clear, group });
    }

    /// The depth the scene reads bind for the depth source: the own copy `depth`, else the far plane (without soft
    /// edges, or until the renderer's backdrop copy is bound in `prepare`).
    fn reads_depth<'a>(&'a self, depth: Option<&'a wgpu::TextureView>) -> &'a wgpu::TextureView {
        match (self.depth_source, depth) {
            (DepthSource::OwnCopy, Some(depth)) => depth,
            _ => &self.far_depth,
        }
    }

    /// The scene reads over `depth` and the splash columns' cover, one over each of the clouds' histories (the clear
    /// stand-in for both without, and for the cover).
    fn reads_group(
        &self,
        device: &wgpu::Device,
        fronts: &wgpu::TextureView,
        depths: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        splash: Option<&naval_render::CoverTarget>,
    ) -> [wgpu::BindGroup; 2] {
        let clouds = |index: usize| match &self.cloud_histories {
            Some((_, histories)) => &histories[index],
            None => &self.no_clouds,
        };
        let splash = splash.map_or((&self.no_clouds, &self.no_clouds), |s| (&s.color, &s.depth));
        [0, 1].map(|index| {
            self.reads_group_over(device, fronts, depths, depth, clouds(index), splash)
        })
    }

    fn reads_group_over(
        &self,
        device: &wgpu::Device,
        fronts: &wgpu::TextureView,
        depths: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        clouds: &wgpu::TextureView,
        splash: (&wgpu::TextureView, &wgpu::TextureView),
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("effect scene reads"),
            layout: &self.reads_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(fronts),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depths),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.occlusion_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.environment.0),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.environment.1),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: self.environment.2.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(clouds),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: wgpu::BindingResource::TextureView(splash.0),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: wgpu::BindingResource::TextureView(splash.1),
                },
                // The smoke's opacity shadow map (stand-ins until it is first drawn).
                wgpu::BindGroupEntry {
                    binding: 40,
                    resource: self.shadows.uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 41,
                    resource: wgpu::BindingResource::TextureView(self.shadows.views().0),
                },
                wgpu::BindGroupEntry {
                    binding: 42,
                    resource: wgpu::BindingResource::TextureView(self.shadows.views().1),
                },
                wgpu::BindGroupEntry {
                    binding: 43,
                    resource: wgpu::BindingResource::Sampler(&self.shadows.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 44,
                    resource: wgpu::BindingResource::TextureView(&self.cloud_shadow.0),
                },
                wgpu::BindGroupEntry {
                    binding: 45,
                    resource: wgpu::BindingResource::Sampler(&self.cloud_shadow.1),
                },
                wgpu::BindGroupEntry {
                    binding: 46,
                    resource: self.cloud_shadow.2.as_entire_binding(),
                },
            ],
        })
    }

    /// The clouds' composite this layer's smoke, fire and glows draw after: a fragment the clouds stand in front of
    /// (a funnel's plume seen from above a deck of cloud) is faded by them. Without it they draw over the clouds.
    pub fn set_cloud_cover(&mut self, cover: Option<naval_sky::CloudCover>) {
        self.cloud_cover = cover;
    }

    /// The splash columns' cover (naval-water-effects' `WaterEffects::cover`), which their composite lays over the
    /// scene before this layer's smoke, fire and glows draw, and which the scene's depth does not hold: a fragment
    /// behind a column is faded by it. Without it they draw over the columns (a burning ship's smoke over a splash
    /// before her).
    pub fn set_splash_cover(&mut self, cover: Option<naval_render::Cover>) {
        self.splash_cover = cover;
    }

    /// The splash columns' cover as their layer published it at this copy point (it draws before this one), unless the
    /// look `splash-cover` is off: the scene reads rebound over it when it is new.
    fn read_splash_cover(&mut self, device: &wgpu::Device) {
        let splash = self
            .splash_cover
            .as_ref()
            .filter(|_| self.effects.trial.splash_cover)
            .and_then(naval_render::Cover::read);
        let Some(targets) = self.targets.as_ref() else {
            return;
        };
        if targets.splash.as_ref().map(|s| s.version) == splash.as_ref().map(|s| s.version) {
            return;
        }
        let reads = self.reads_group(
            device,
            &targets.fronts,
            &targets.depths,
            &targets.reads_depth,
            splash.as_ref(),
        );
        let targets = self.targets.as_mut().expect("present");
        targets.reads = reads;
        targets.splash = splash;
    }

    /// This frame's clouds from the published cover: binds their histories when they are new, and keeps the one the
    /// composite draws and the lighting's `clouds`.
    fn read_cloud_cover(&mut self, device: &wgpu::Device, frame: u64) {
        let cover = self.cloud_cover.as_ref().map(naval_sky::CloudCover::read);
        let histories = cover
            .as_ref()
            .and_then(|c| c.histories.clone().map(|h| (c.version, h)));
        let bound = self.cloud_histories.as_ref().map(|(version, _)| *version);
        if histories.as_ref().map(|(version, _)| *version) != bound {
            self.cloud_histories = histories;
            if let Some(targets) = self.targets.as_ref() {
                let reads = self.reads_group(
                    device,
                    &targets.fronts,
                    &targets.depths,
                    &targets.reads_depth,
                    targets.splash.as_ref(),
                );
                self.targets.as_mut().expect("present").reads = reads;
            }
        }
        self.cloud_frame = match cover {
            Some(cover) if cover.frame == Some(frame) && self.cloud_histories.is_some() => {
                match cover.current {
                    Some(current) => (current, [cover.bottom, cover.top, cover.planet_radius, 1.0]),
                    None => (0, [0.0; 4]),
                }
            }
            _ => (0, [0.0; 4]),
        };
    }

    fn lighting_uniform(
        &self,
        frame: &EffectsFrame,
        occlusion: bool,
        columns: bool,
    ) -> LightingUniform {
        let lighting = &self.effects.lighting;
        let v = |x: glam::DVec3, w: f32| [x.x as f32, x.y as f32, x.z as f32, w];
        let fog = &self.fog;
        LightingUniform {
            sun_direction: v(lighting.sun_direction, lighting.daylight as f32),
            direct: v(lighting.direct, f32::from(occlusion)),
            ambient: v(lighting.ambient, lighting.flash as f32),
            fog_color: [fog.color[0], fog.color[1], fog.color[2], fog.power],
            fog: [
                fog.start,
                fog.end,
                fog.sky_blend_distance,
                f32::from(fog.enabled),
            ],
            fog_sky: [
                fog.sky[0],
                fog.sky[1],
                fog.sky[2],
                f32::from(u8::from(self.environment.3)),
            ],
            flashes: frame.flashes,
            clouds: self.cloud_frame.1,
            trial: self.effects.trial.uniform(),
            emission: [
                lighting.lift as f32,
                lighting.flames as f32,
                lighting.glow as f32,
                lighting.flash_smoke as f32,
            ],
            flash_color: [
                frame.flash_color[0],
                frame.flash_color[1],
                frame.flash_color[2],
                f32::from(columns),
            ],
            flash_glow: [lighting.flash_glow as f32, 0.0, 0.0, 0.0],
        }
    }
}

impl SceneLayer for EffectsLayer {
    fn label(&self) -> &str {
        "combat effects"
    }

    fn scene_copies(&self) -> SceneCopies {
        if self.depth_source == DepthSource::Backdrop {
            SceneCopies::BACKDROP_DEPTH
        } else {
            SceneCopies::NONE
        }
    }

    fn set_view(&mut self, view: SceneView) {
        let second = view == SceneView::Second;
        if second != self.second {
            self.second = second;
            std::mem::swap(&mut self.targets, &mut self.other_targets);
        }
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let camera = CameraView::from_uniforms(ctx.camera);
        // Written over last frame's, its lists keeping their room.
        let mut frame = std::mem::take(&mut self.frame);
        self.effects.publish_into(&camera, &mut frame);
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        // The instances and the frame's uniforms through the frame's staging belt (no fresh staging buffer per write).
        self.upload(&device, &mut UploadSink::Belt(ctx.uploads), &frame);
        self.volumes.prepare(
            &device,
            &queue,
            ctx.camera,
            &frame.volumes,
            &self.volume,
            &self.volume_sampler,
        );
        if !self.second {
            self.read_cloud_cover(&device, ctx.time.frame);
        }
        // The volume trial's bodies occlude as the gas does.
        let occlusion = self.occlusion_enabled
            && (!frame.gas.is_empty() || !frame.funnel_gas.is_empty() || self.volumes.live());
        // The fires' columns, in the map's channels of their own: what draws after them (a fire's flames, its bodies
        // of burning gas and its embers) fades behind their smoke.
        let columns = self.occlusion_enabled
            && self.effects.trial.column_cover
            && !frame.fire_smoke.gas.is_empty();
        ctx.uploads.write_buffer(
            &self.lighting_buffer,
            0,
            bytes_of(&self.lighting_uniform(&frame, occlusion, columns)),
        );
        self.stats = DrawStats {
            gas: frame.gas.len() as u32,
            soot: frame.soot.len() as u32,
            fire: frame.fire.len() as u32,
            fire_smoke: (frame.fire_smoke.quads.len() + frame.fire_smoke.gas.len()) as u32,
            fire_bodies: frame.fire_bodies.len() as u32,
            fire_flames: frame.fire_flames.quads.len() as u32,
            fire_embers: frame.fire_embers.len() as u32,
            funnel_billows: (frame.funnel_billows.len()
                + frame.funnel_links.len()
                + frame.funnel_gas.len()) as u32,
            debris: frame.debris.len() as u32,
            funnel_ribbon_triangles: frame.funnel_ribbon_indices.len() as u32 / 3,
            occlusion,
            merged: false,
            fire_fill: frame.fire_smoke.fill + frame.fire_flames.fill,
        };
        self.frame = frame;
        self.air_light.prepare(
            (self.air_light_enabled, self.second),
            &device,
            &mut UploadSink::Belt(ctx.uploads),
            &camera,
            (
                2.0 * f64::from(ctx.camera.tan_half_fov_y) / f64::from(ctx.viewport.1.max(1)),
                ctx.viewport,
            ),
            (&self.effects.tactical, &self.effects.rules),
            (&self.volume, &self.volume_sampler),
        );

        // The smoke's opacity shadow map along the light (the `shadows` look, while the sun's maps are drawn), and the
        // renderer's soft occluders from it. Its maps are made on its first frame, and the scene reads rebuilt over
        // them.
        // A second view casts no smoke shadow of its own: the sun's maps are the main view's.
        if !self.second && self.shadows.prepare(ctx, &self.effects, &camera) {
            self.targets = None;
            self.other_targets = None;
        }

        let source = ctx
            .resources
            .texture(SCENE_DEPTH)
            .map(|entry| (entry.version, entry.view.clone()));
        let backdrop = ctx
            .resources
            .texture(SCENE_BACKDROP_DEPTH)
            .map(|entry| (entry.version, entry.view.clone()));
        self.ensure_targets(&device, ctx.viewport);
        self.merged = false;
        if self.merging() {
            self.ensure_merged(&device);
        }
        // Render core II's backdrop depth: rebind the scene reads when the renderer made or remade it.
        if self.depth_source == DepthSource::Backdrop
            && let Some((version, view)) = &backdrop
            && let Some(targets) = self.targets.as_ref()
            && targets.source_version != *version
        {
            let reads = self.reads_group(
                &device,
                &targets.fronts,
                &targets.depths,
                view,
                targets.splash.as_ref(),
            );
            let targets = self.targets.as_mut().expect("present");
            targets.reads = reads;
            targets.reads_depth = view.clone();
            targets.source_version = *version;
        }
        {
            // The occlusion pass draws into its quarter-resolution target: its camera's viewport is that target's,
            // as three's `screenSize` is the bound render target's (the puffs' detail strength reads it).
            let quarter = self.targets.as_ref().expect("ensured").quarter;
            let mut camera = *ctx.camera;
            let (w, h) = (quarter.0 as f32, quarter.1 as f32);
            camera.viewport = [w, h, 1.0 / w, 1.0 / h];
            ctx.uploads
                .write_buffer(&self.occlusion_camera, 0, bytes_of(&camera));
        }
        let targets = self.targets.as_mut().expect("ensured");

        // The occlusion map: cleared every frame, then the occluding gas drawn alone into it.
        {
            let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("smoke occlusion"),
                timestamp_writes: ctx.timer.render_pass("smoke occlusion"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.fronts,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        view: &targets.depths,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                ..Default::default()
            });
            if occlusion || columns {
                pass.set_bind_group(0, &self.occlusion_camera_group, &[]);
                pass.set_bind_group(1, &self.lighting_group, &[]);
            }
            if occlusion {
                pass.set_pipeline(ctx.pipelines.render(self.occluder_pipeline));
                for batch in [&self.gas, &self.funnel_gas] {
                    if batch.count > 0 {
                        pass.set_bind_group(2, batch.group(), &[]);
                        pass.draw(0..6, 0..batch.count);
                    }
                }
                self.volumes.draw_occluders(&mut pass, ctx.pipelines);
            }
            if columns && self.column.count > 0 {
                pass.set_pipeline(ctx.pipelines.render(self.column_occluder_pipeline));
                pass.set_bind_group(2, self.column.group(), &[]);
                pass.draw(0..6, 0..self.column.count);
            }
        }

        // The soft particles' depth: this crate's copy of what `scene.depth` holds now (the previous frame).
        if self.depth_source == DepthSource::OwnCopy
            && let Some((version, view)) = source
            && let Some(depth) = targets.depth.as_ref()
        {
            if targets.source_version != version || targets.copy_group.is_none() {
                targets.copy_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("effect depth copy"),
                    layout: &self.copy_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    }],
                }));
                targets.source_version = version;
            }
            let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("effect depth copy"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_pipeline(ctx.pipelines.render(self.depth_copy_pipeline));
            pass.set_bind_group(0, targets.copy_group.as_ref().expect("built above"), &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// At the backdrop's copy point (this frame's opaque and sea depth taken): the star shells' halo in the air at half
    /// the resolution (`air_light.rs`), and in look `merge` the gas and the soot added up into the merged smoke against
    /// the scene's depth, which `draw` then lays over the scene.
    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        if point != CopyPoint::Backdrop {
            return;
        }
        self.read_splash_cover(ctx.device);
        if let Some(targets) = self.targets.as_ref() {
            self.air_light.draw_halo(
                ctx,
                &self.lighting_group,
                &targets.reads[self.cloud_frame.0],
            );
        }
        if !self.merging() {
            return;
        }
        let (Some(targets), Some(depth)) =
            (self.targets.as_ref(), ctx.resources.texture(SCENE_DEPTH))
        else {
            return;
        };
        let Some(merged) = targets.merged.as_ref() else {
            return;
        };
        let cleared = |view, clear| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("merged smoke"),
            color_attachments: &[
                cleared(&merged.sum, wgpu::Color::TRANSPARENT),
                cleared(&merged.clear, wgpu::Color::WHITE),
            ],
            // Tested against, never written: the scene pass goes on with it.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth.view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: ctx.timer.render_pass("merged smoke"),
            ..Default::default()
        });
        pass.set_bind_group(0, ctx.camera_group, &[]);
        pass.set_bind_group(1, &self.lighting_group, &[]);
        pass.set_bind_group(3, &targets.reads[self.cloud_frame.0], &[]);
        pass.set_pipeline(ctx.pipelines.render(self.merged_gas_pipeline));
        for batch in [&self.gas, &self.soot, &self.funnel_gas] {
            if batch.count == 0 {
                continue;
            }
            pass.set_bind_group(2, batch.group(), &[]);
            pass.draw(0..6, 0..batch.count);
        }
        drop(pass);
        self.merged = true;
        self.stats.merged = true;
    }

    /// The strike debris into each of the sun's shadow maps but the land maps, which only the land casts into (the
    /// smoke's shadow is the soft occluders' map, `shadow.rs`).
    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if map.kind.is_land() {
            return;
        }
        if self.debris_casters.count > 0 {
            self.sun_casters.draw(
                pass,
                ctx.pipelines,
                &self.lighting_group,
                self.debris_casters.group(),
                self.debris_casters.count,
            );
        }
    }

    /// The volume trial's bodies ask temporal anti-aliasing for less history (nothing without one).
    fn draw_motion_over(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if let Some(targets) = &self.targets {
            let reads = &targets.reads[self.cloud_frame.0];
            self.volumes
                .draw_responses(pass, ctx, &self.lighting_group, reads);
        }
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let Some(targets) = &self.targets else {
            return;
        };
        if phase == Phase::Opaque && self.debris.count > 0 {
            // Each piece up to four boxes of 12 triangles, as its shape needs.
            pass.set_pipeline(ctx.pipelines.render(self.debris_pipeline));
            pass.set_bind_group(1, &self.lighting_group, &[]);
            pass.set_bind_group(2, self.debris.group(), &[]);
            pass.set_bind_group(3, &targets.reads[self.cloud_frame.0], &[]);
            pass.draw(0..crate::debris::PIECE_VERTICES, 0..self.debris.count);
        }
        if phase != Phase::Transparent {
            return;
        }
        pass.set_bind_group(1, &self.lighting_group, &[]);
        pass.set_bind_group(3, &targets.reads[self.cloud_frame.0], &[]);
        let instanced = |pass: &mut wgpu::RenderPass<'_>, pipeline: PipelineId, batch: &Batch| {
            if batch.count == 0 {
                return;
            }
            pass.set_pipeline(ctx.pipelines.render(pipeline));
            pass.set_bind_group(2, batch.group(), &[]);
            pass.draw(0..6, 0..batch.count);
        };
        // The order: render order 0 by creation (combat fire, gas, the fire columns, soot, funnel ribbon and
        // billows), then 1 (flame bodies), 2 (flames) and 3 (embers), so the flames and embers blend over the smoke.
        instanced(pass, self.sprite_pipeline, &self.fire);
        // Look `merge`: the gas, the funnels' smoke and the soot merged (`after_copies`), laid over the scene where the
        // gas draws.
        let merged = targets.merged.as_ref().filter(|_| self.merged);
        if let Some(merged) = merged {
            pass.set_pipeline(ctx.pipelines.render(self.merge_pipeline));
            pass.set_bind_group(2, &merged.group, &[]);
            pass.draw(0..3, 0..1);
        } else {
            instanced(pass, self.gas_pipeline, &self.gas);
            instanced(pass, self.gas_pipeline, &self.funnel_gas);
        }
        instanced(pass, self.fire_smoke_pipeline, &self.fire_smoke);
        // The volume trial's bodies, where the smoke columns' sprites draw.
        self.volumes.draw(pass, ctx);
        instanced(pass, self.link_pipeline, &self.column);
        if merged.is_none() {
            instanced(pass, self.gas_pipeline, &self.soot);
        }
        if self.ribbon_count > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.ribbon_pipeline));
            pass.set_bind_group(2, self.billows.group(), &[]);
            pass.set_vertex_buffer(0, self.ribbon_vertices.buffer.slice(..));
            pass.set_index_buffer(
                self.ribbon_indices.buffer.slice(..),
                wgpu::IndexFormat::Uint32,
            );
            pass.draw_indexed(0..self.ribbon_count, 0, 0..1);
        }
        instanced(pass, self.billow_pipeline, &self.billows);
        instanced(pass, self.link_pipeline, &self.funnel_links);
        instanced(pass, self.gas_pipeline, &self.bodies);
        instanced(pass, self.flame_pipeline, &self.flames);
        instanced(pass, self.sprite_pipeline, &self.embers);
        // Last, added over everything the air holds: the light it scatters from the beams and the star shells.
        self.air_light.draw(pass, ctx.pipelines);
    }
}

const _: () = assert!(std::mem::size_of::<LightingUniform>() == 240);

/// No sky yet: a black 1×1 atlas, a sampler and a zeroed `SkyEnvironment` (never read while the fog's flag is 0).
pub(crate) fn placeholder_environment(
    device: &wgpu::Device,
) -> (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, bool) {
    let view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("effect placeholder environment"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("effect placeholder environment"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("effect placeholder environment"),
        size: std::mem::size_of::<naval_sky::environment::EnvironmentUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    (view, sampler, uniforms, false)
}

/// No clouds' shadow: a 1 × 1 map under zeroed uniforms, which leave the light whole (`enabled` 0).
fn placeholder_cloud_shadow(
    device: &wgpu::Device,
) -> (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer) {
    let view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("effect placeholder cloud shadow"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("effect placeholder cloud shadow"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("effect placeholder cloud shadow"),
        size: std::mem::size_of::<naval_clouds::CloudShadowUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    (view, sampler, uniforms)
}
