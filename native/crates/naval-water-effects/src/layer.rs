//! [`WaterEffects`]: the splashes and bow spray as one `naval_render::SceneLayer`. The host calls
//! [`WaterEffects::update`] once a frame with what the simulation shows ([`EffectsFrame`]); `prepare` uploads the
//! frame's columns and sprites, and `draw` draws them in `Phase::Transparent` (depth tested, not written, blended on
//! straight alpha, linear HDR): first the sheets of spray the heavy guns' blasts tear off the sea under them
//! ([`crate::blast_sheet`], `shaders/blast.wgsl`), then the splashes' columns of water and the bows' sheets and bursts of
//! spray, marched through back to front ([`crate::columns`], [`crate::bow_sheets`], [`crate::bow_bursts`],
//! `shaders/column.wgsl`), then the drops the splashes throw and the billows of a whale's blow. Every
//! column and sprite takes the sun's shadow maps, and a splash's column casts into them (`draw_shadow`,
//! `shaders/column_caster.wgsl`).

use bytemuck::cast_slice;
use glam::DVec3;
use naval_render::{
    CasterPipeline, CopyPoint, DrawContext, Phase, PipelineId, PrepareContext, RenderContext,
    SCENE_BACKDROP_DEPTH, SceneCopies, SceneLayer, ScenePipeline, SceneView, ShaderError,
    ShaderFile, ShadowMap, VertexLayout, compute_recipe,
};

use crate::blast_sheet::{BlastSheets, GunBlast, MAX_SHEETS, SheetInstance};
use crate::bow_spray::{BowSpray, BowSprayFrame, CrestShape, EffectShip};
use crate::columns::ColumnGpu;
use crate::fixture::{SeaWaves, WaterEvent};
use crate::lighting::{EffectLighting, LightingUniform};
use crate::splash::Splashes;
use naval_effects::CameraView;
use naval_effects::instances::WaterSprite;
use naval_effects::particles::ParticlePool;
use naval_effects::textures::{Image2d, SpriteMap, effect_texture, smoke_billow_texture};

pub const SPRAY_MODULE: ShaderFile = naval_render::shader_file!("shaders/spray.wgsl");
pub const COLUMN_FIELD_MODULE: ShaderFile = naval_render::shader_file!("shaders/column_field.wgsl");
pub const VALUE_NOISE_MODULE: ShaderFile = naval_render::shader_file!("shaders/value_noise.wgsl");
pub const SPRITE_SHADER: ShaderFile = naval_render::shader_file!("shaders/sprite.wgsl");
pub const COLUMN_SHADER: ShaderFile = naval_render::shader_file!("shaders/column.wgsl");
pub const COLUMN_CASTER_SHADER: ShaderFile =
    naval_render::shader_file!("shaders/column_caster.wgsl");
pub const COLUMN_NOISE_SHADER: ShaderFile = naval_render::shader_file!("shaders/column_noise.wgsl");
pub const COLUMN_COMPOSITE_SHADER: ShaderFile =
    naval_render::shader_file!("shaders/column_composite.wgsl");
pub const BLAST_SHADER: ShaderFile = naval_render::shader_file!("shaders/blast.wgsl");
/// Every WGSL file, modules first.
pub const SHADERS: [ShaderFile; 9] = [
    SPRAY_MODULE,
    COLUMN_FIELD_MODULE,
    VALUE_NOISE_MODULE,
    SPRITE_SHADER,
    COLUMN_SHADER,
    COLUMN_CASTER_SHADER,
    COLUMN_NOISE_SHADER,
    COLUMN_COMPOSITE_SHADER,
    BLAST_SHADER,
];
/// Texels on each edge of the columns' tiling noise volume (`shaders/column_noise.wgsl`'s `SIZE`).
const NOISE_SIZE: u32 = 64;
/// The columns' target: premultiplied linear HDR radiance and opacity.
const COLUMN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Beside it, the view depth (m) times the opacity, added up as the colour is: where the columns' water lies along the
/// view, for the combat effects drawn after the composite ([`WaterEffects::cover`]).
const COLUMN_DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;

/// What the effects read in one frame.
pub struct EffectsFrame<'a> {
    /// Presentation seconds since the last frame; 0 holds everything where it is.
    pub dt: f64,
    pub camera: CameraView,
    /// Every drawn hull (the bow spray takes the nearest twelve it sees largest).
    pub ships: &'a [EffectShip<'a>],
    /// One-shot events since the last frame; events already seen are skipped by sequence.
    pub events: &'a [WaterEvent],
    /// The combat sea and the presentation time it is drawn at.
    pub sea: Option<(SeaWaves, f64)>,
    /// Significant wave height of the sea on show (m).
    pub waves: f64,
    /// The bow waves' crest shape the spray rises from; `None` while they are off.
    pub crest: Option<CrestShape>,
    pub lighting: EffectLighting,
}

/// Sprites in one draw: lit billows of spray on the billow map, or drops on the droplet map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Material {
    Billows,
    Drops,
}

struct Batch {
    material: Material,
    buffer: wgpu::Buffer,
    /// Instances (sprites).
    count: u32,
}

/// The target the columns are marched into. Large columns use half resolution; small distant columns use full
/// resolution within a projected pixel budget (`columns::march_divisor`).
struct ColumnTarget {
    /// The march target's size.
    size: (u32, u32),
    color: wgpu::TextureView,
    /// The columns' depth over the same texels ([`COLUMN_DEPTH_FORMAT`]).
    lies: wgpu::TextureView,
    /// Made anew with the target, for the readers of the cover.
    version: u64,
    /// Whether the target still holds a march: cleared once when no column stands, so the cover reads clear.
    holds: bool,
    /// Cleared each frame and never written: the scene pipelines' depth state wants an attachment; the march stops at
    /// the backdrop's depth itself.
    depth: wgpu::TextureView,
    /// The composite's group (the target and the backdrop's depth) and the depth copy's version it was made with.
    composite: Option<(u64, wgpu::BindGroup)>,
}

/// The splashes' columns: their pipelines, the noise volume they read and the frame's columns.
struct ColumnDraw {
    /// The march into its adaptive-resolution target, and the composite over the scene.
    march: PipelineId,
    composite: PipelineId,
    composite_layout: wgpu::BindGroupLayout,
    target: Option<ColumnTarget>,
    /// Whether the columns were marched this frame (at the backdrop's copy point).
    marched: bool,
    divisor: u32,
    caster: PipelineId,
    /// The noise volume's fill, its group, and whether the volume holds it yet.
    fill: PipelineId,
    fill_group: wgpu::BindGroup,
    filled: bool,
    noise: wgpu::Texture,
    /// Group 1 of the march and the caster: the lighting, the noise volume, its sampler and the columns.
    group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    count: u32,
}

/// The splashes and bow spray, simulated on the CPU and drawn as one layer.
/// What one view of the frame keeps that the other must not take from it: the columns' march target and divisor, and
/// the backdrop depth group (each at its own viewport's size).
struct OtherView {
    target: Option<ColumnTarget>,
    divisor: u32,
    depth_group: (Option<u64>, wgpu::BindGroup),
}

pub struct WaterEffects {
    pub lighting: EffectLighting,
    pub splashes: Splashes,
    pub bow_spray: BowSpray,
    /// The water lost hulls pour off as they come out of the sea (`crate::runoff`), thrown as the bow spray's bursts.
    pub runoff: crate::runoff::HullRunoff,
    /// The sheets of spray the heavy guns' blasts tear off the sea ([`WaterEffects::gun_blasts`]).
    pub blasts: BlastSheets,
    lighting_buffer: wgpu::Buffer,
    /// The sprites' group 1 for the billows and the drops: the lighting, the map and its sampler.
    groups: [wgpu::BindGroup; 2],
    /// The sky's environment the spray takes its sky light from (group 2; [`WaterEffects::set_environment`]), and
    /// whether it is the sky's rather than the black placeholder.
    sky_layout: wgpu::BindGroupLayout,
    sky_group: wgpu::BindGroup,
    sky_bound: bool,
    /// The backdrop's depth the sprites fade into and the columns stop at (group 3: where they meet the sea or a
    /// hull), and the copy's version it holds; a 1 × 1 far-plane placeholder until a copy is taken.
    depth_layout: wgpu::BindGroupLayout,
    depth_group: (Option<u64>, wgpu::BindGroup),
    /// Preparing and drawing a second view of the frame ([`naval_render::Renderer::render_view`]), and the other view's
    /// column target, march divisor and depth group (each at its own viewport's size), kept while this one's are in place.
    second: bool,
    other: Option<OtherView>,
    pipelines: [PipelineId; 2],
    columns: ColumnDraw,
    /// Whether a splash's column casts into the sun's shadow maps (on by default).
    pub cast_shadows: bool,
    /// The columns' target as the layers drawn after the composite read it ([`WaterEffects::cover`]), and the last
    /// version given a target (each view's its own).
    cover: naval_render::Cover,
    cover_version: u64,
    /// The frame's splash columns, bow sheets and bursts in view, farthest first: what the march draws.
    marched: Vec<ColumnGpu>,
    /// Splash drops, billows (a whale's blow).
    batches: Vec<Batch>,
    /// The blast sheets' pipeline, its group 1 (the lighting; the point lights are the renderer's, at group 0), and
    /// the sheets drawn this frame.
    blast_pipeline: PipelineId,
    blast_group: wgpu::BindGroup,
    blast_buffer: wgpu::Buffer,
    blast_count: u32,
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    data: &Image2d,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: data.size,
            height: data.size,
            depth_or_array_layers: 1,
        },
        mip_level_count: data.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let (mut width, mut height) = (data.size, data.size);
    for (level, pixels) in data.levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn alpha_blend() -> wgpu::BlendState {
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

/// `count` vec4 attributes at locations 0.., packed in a vertex of `stride` bytes.
fn vec4_attributes(step_mode: wgpu::VertexStepMode, stride: u64, count: u64) -> VertexLayout {
    VertexLayout {
        stride,
        step_mode,
        attributes: (0..count)
            .map(|i| wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: i * 16,
                shader_location: i as u32,
            })
            .collect(),
    }
}

impl WaterEffects {
    /// Build the layer's pipelines through the renderer core (so hot reload reaches them) and its textures.
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        // The sky's environment module, which `shaders/spray.wgsl` imports, with its disk path so it hot-reloads (unless
        // the sky or the ocean has registered it already).
        if ctx
            .library
            .text(naval_sky::shaders::ENVIRONMENT_FILE.path)
            .is_none()
        {
            ctx.add_shader(naval_sky::shaders::ENVIRONMENT_FILE)?;
        }
        for file in SHADERS {
            ctx.add_shader(file)?;
        }
        let device = ctx.device;
        let queue = ctx.queue;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water effects"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sky_layout = sky_layout(device);
        let sky_group = placeholder_sky(device, &sky_layout);
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water effects backdrop depth"),
            entries: &[naval_render::copies::depth_copy_layout_entry(
                0,
                wgpu::ShaderStages::FRAGMENT,
            )],
        });
        let depth_group = (None, placeholder_depth(device, &depth_layout));
        let lighting_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("water effects lighting"),
            size: std::mem::size_of::<LightingUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let clamp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("water effects sprites"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let group = |label: &str, data: &Image2d| {
            let view = upload(device, queue, label, data);
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: lighting_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&clamp),
                    },
                ],
            })
        };
        let groups = [
            group("spray billow map", &smoke_billow_texture()),
            group("spray droplet map", &effect_texture(SpriteMap::Droplet)),
        ];
        let sprite_vertex = vec4_attributes(
            wgpu::VertexStepMode::Instance,
            std::mem::size_of::<WaterSprite>() as u64,
            4,
        );
        let transparent = |fragment: &'static str| ScenePipeline {
            vertex_buffers: vec![sprite_vertex.clone()],
            bind_group_layouts: vec![layout.clone(), sky_layout.clone(), depth_layout.clone()],
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_write: false,
            blend: Some(alpha_blend()),
            ..ScenePipeline::new("vertex", fragment)
        };
        let pipelines = [
            ctx.add_scene_pipeline(
                "spray billows",
                SPRITE_SHADER.path,
                transparent("fragment_spray"),
            )?,
            ctx.add_scene_pipeline(
                "spray drops",
                SPRITE_SHADER.path,
                transparent("fragment_basic"),
            )?,
        ];
        let splashes = Splashes::new();
        let columns =
            ColumnDraw::new(ctx, &lighting_buffer, &sky_layout, &depth_layout, &splashes)?;
        // The blast sheets: the lighting and the scene's point lights in group 1.
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let blast_layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blast sheets"),
                entries: &[uniform(0)],
            });
        let blast_pipeline = ctx.add_scene_pipeline(
            "blast sheets",
            BLAST_SHADER.path,
            ScenePipeline {
                vertex_buffers: vec![vec4_attributes(
                    wgpu::VertexStepMode::Instance,
                    std::mem::size_of::<SheetInstance>() as u64,
                    5,
                )],
                bind_group_layouts: vec![
                    blast_layout.clone(),
                    sky_layout.clone(),
                    depth_layout.clone(),
                ],
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_write: false,
                blend: Some(alpha_blend()),
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let blast_group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("blast sheets"),
            layout: &blast_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: lighting_buffer.as_entire_binding(),
            }],
        });
        let bow_spray = BowSpray::new();
        let device = ctx.device;
        let buffer = |label: &str, bytes: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let sprites = |label: &str, pool: &ParticlePool| {
            buffer(label, pool.capacity * std::mem::size_of::<WaterSprite>())
        };
        let batches = vec![
            Batch {
                material: Material::Drops,
                buffer: sprites("splash drops", &splashes.spray),
                count: 0,
            },
            Batch {
                material: Material::Billows,
                buffer: sprites("spray billows", &bow_spray.billows),
                count: 0,
            },
        ];
        let blast_buffer = buffer(
            "blast sheets",
            MAX_SHEETS * std::mem::size_of::<SheetInstance>(),
        );
        Ok(Self {
            lighting: EffectLighting::default(),
            splashes,
            bow_spray,
            runoff: crate::runoff::HullRunoff::new(),
            blasts: BlastSheets::new(),
            lighting_buffer,
            groups,
            sky_layout,
            sky_group,
            sky_bound: false,
            depth_layout,
            depth_group,
            second: false,
            other: None,
            pipelines,
            columns,
            cast_shadows: true,
            cover: naval_render::Cover::default(),
            cover_version: 0,
            marched: Vec::new(),
            batches,
            blast_pipeline,
            blast_group,
            blast_buffer,
            blast_count: 0,
        })
    }

    /// Turrets fired since the last update (naval-effects' `CombatEffects::turret_blasts`): the next update lays the
    /// sheets of spray their blasts tear off the sea.
    pub fn gun_blasts(&mut self, blasts: &[GunBlast]) {
        self.blasts.queue(blasts);
    }

    /// The splash columns' target, published at the backdrop's copy point, as a layer drawn after their composite
    /// reads it (the combat effects' `set_splash_cover`): the scene's depth does not hold the columns, so without it a
    /// plume behind a column draws over it.
    pub fn cover(&self) -> naval_render::Cover {
        self.cover.clone()
    }

    /// Light the spray's shade and its unlit drops and mist from the sky's environment (naval-sky's `EnvironmentMap`:
    /// its atlas, sampler and uniforms), as the ships and the combat effects are; until this is called they take the
    /// lighting's authored ambient and daylight. Call again when the sky remakes its atlas.
    pub fn set_environment(
        &mut self,
        device: &wgpu::Device,
        atlas: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
        uniforms: &wgpu::Buffer,
    ) {
        self.sky_group = sky_group(device, &self.sky_layout, atlas, sampler, uniforms);
        self.sky_bound = true;
    }

    /// Advance the effects one frame (`CombatEffects.update`'s splash part, then `BowSpray.update`).
    pub fn update(&mut self, frame: &EffectsFrame<'_>) {
        self.lighting = frame.lighting;
        let wind = self.lighting.wind;
        // Advance before emitting: a slow frame still shows the first instant of a splash.
        self.splashes.advance(frame.dt, wind);
        self.splashes.emit(frame.events);
        self.splashes.publish(&frame.camera);
        self.blasts.update(frame.dt, wind, frame.sea, frame.waves);
        self.blasts.publish(&frame.camera);
        self.bow_spray.update(
            &BowSprayFrame {
                ships: frame.ships,
                dt: frame.dt,
                camera: &frame.camera,
                sea: frame.sea,
                crest: frame.crest,
                waves: frame.waves,
            },
            wind,
        );
        // What lost hulls lift out of the sea pours off them: the sea's amplitude is the combat sea's (what heaves her).
        let amplitude = frame.sea.as_ref().map_or(0.0, |(sea, _)| sea.amplitude_m);
        self.runoff.update(
            frame.ships,
            frame.dt,
            &frame.sea,
            amplitude,
            frame.camera.position,
            &mut self.bow_spray.bursts,
        );
    }

    /// The splashes, the bows' sheets and bursts and the blast sheets placed for `camera` again, without moving them
    /// on: a second view of the frame draws what its own camera sees ([`naval_render::Renderer::render_view`]), and the
    /// main view's camera is given back after it.
    pub fn publish(&mut self, camera: &CameraView) {
        self.splashes.publish(camera);
        self.bow_spray.publish_marched(camera);
        self.blasts.publish(camera);
    }

    /// Columns and particles drawn last frame.
    pub fn counts(&self) -> EffectCounts {
        EffectCounts {
            columns: self.splashes.columns.count(),
            bursts: self.bow_spray.bursts.instances().len(),
            billows: self.bow_spray.billow_sprites.len(),
            drops: self.splashes.drops.len(),
            sheets: self.bow_spray.sheets.instances().len(),
            bows: self.bow_spray.slots(),
            blasts: self.blasts.count(),
        }
    }

    /// The splash columns' tiling noise volume (64³ `Rgba8Unorm`), filled on the GPU before the first column draws:
    /// for tools that read it back.
    pub fn column_noise(&self) -> &wgpu::Texture {
        &self.columns.noise
    }

    /// Clear every splash and all spray.
    pub fn reset(&mut self) {
        self.splashes.reset();
        self.bow_spray.reset();
        self.runoff.reset();
        self.blasts.reset();
    }
}

impl ColumnDraw {
    fn new(
        ctx: &mut RenderContext<'_>,
        lighting: &wgpu::Buffer,
        sky_layout: &wgpu::BindGroupLayout,
        depth_layout: &wgpu::BindGroupLayout,
        splashes: &Splashes,
    ) -> Result<Self, ShaderError> {
        let device = ctx.device;
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splash columns"),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: both,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("splash column noise"),
            size: wgpu::Extent3d {
                width: NOISE_SIZE,
                height: NOISE_SIZE,
                depth_or_array_layers: NOISE_SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let noise_view = noise.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("splash column noise"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("splash columns"),
            size: ((splashes.columns.capacity()
                + crate::bow_spray::BURST_CAPACITY
                + crate::bow_spray::SHEET_CAPACITY)
                * std::mem::size_of::<ColumnGpu>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splash columns"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lighting.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&noise_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: buffer.as_entire_binding(),
                },
            ],
        });
        let fill_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splash column noise fill"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D3,
                },
                count: None,
            }],
        });
        let fill_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("splash column noise fill"),
            layout: &fill_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&noise_view),
            }],
        });
        let fill = ctx.add_pipeline(
            "splash column noise",
            COLUMN_NOISE_SHADER.path,
            Vec::new(),
            compute_recipe(vec![fill_layout], "fill"),
        )?;
        // The front faces of each box (counter-clockwise from outside), or the triangle over the screen, into the
        // adaptive-resolution target.
        let march = ctx.add_scene_pipeline_into(
            "splash columns",
            COLUMN_SHADER.path,
            ScenePipeline {
                bind_group_layouts: vec![layout.clone(), sky_layout.clone(), depth_layout.clone()],
                depth_write: false,
                ..ScenePipeline::new("vertex", "fragment")
            },
            vec![
                Some(wgpu::ColorTargetState {
                    format: COLUMN_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
                Some(wgpu::ColorTargetState {
                    format: COLUMN_DEPTH_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                }),
            ],
        )?;
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("splash columns composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                naval_render::copies::depth_copy_layout_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let composite = ctx.add_scene_pipeline(
            "splash columns composite",
            COLUMN_COMPOSITE_SHADER.path,
            ScenePipeline {
                bind_group_layouts: vec![composite_layout.clone()],
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_write: false,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                ..ScenePipeline::new("vertex", "fragment")
            },
        )?;
        let caster = ctx.add_caster_pipeline(
            "splash column casters",
            COLUMN_CASTER_SHADER.path,
            CasterPipeline {
                bind_group_layouts: vec![layout],
                fragment_entry: Some("fragment"),
                ..CasterPipeline::new("vertex")
            },
        )?;
        Ok(Self {
            march,
            composite,
            composite_layout,
            target: None,
            marched: false,
            divisor: 2,
            caster,
            fill,
            fill_group,
            filled: false,
            noise,
            group,
            buffer,
            count: 0,
        })
    }
}

/// What a frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EffectCounts {
    /// Splash columns in view, and the bows' bursts and sheets of spray.
    pub columns: usize,
    pub bursts: usize,
    /// Lit billows (a whale's blow).
    pub billows: usize,
    pub drops: usize,
    pub sheets: usize,
    pub bows: usize,
    /// Blast sheets on the sea.
    pub blasts: usize,
}

impl SceneLayer for WaterEffects {
    fn label(&self) -> &str {
        "water effects"
    }

    fn scene_copies(&self) -> SceneCopies {
        // The columns stop at the sea or a hull behind them, the sprites fade into it, and a blast sheet's columns of
        // mist run down to it. A target still holding the last march is cleared at its copy point (the cover).
        if self.blasts.count()
            + self.splashes.columns.count()
            + self.bow_spray.bursts.instances().len()
            + self.bow_spray.sheets.instances().len()
            + self.splashes.drops.len()
            + self.bow_spray.billow_sprites.len()
            > 0
            || self.columns.target.as_ref().is_some_and(|t| t.holds)
        {
            SceneCopies::BACKDROP_DEPTH
        } else {
            SceneCopies::NONE
        }
    }

    fn set_view(&mut self, view: SceneView) {
        let second = view == SceneView::Second;
        if second == self.second {
            return;
        }
        self.second = second;
        let other = self.other.get_or_insert_with(|| OtherView {
            target: None,
            divisor: 1,
            depth_group: (None, self.depth_group.1.clone()),
        });
        std::mem::swap(&mut self.columns.target, &mut other.target);
        std::mem::swap(&mut self.columns.divisor, &mut other.divisor);
        std::mem::swap(&mut self.depth_group, &mut other.depth_group);
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        if let Some(depth) = ctx.resources.texture(SCENE_BACKDROP_DEPTH)
            && self.depth_group.0 != Some(depth.version)
        {
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water effects backdrop depth"),
                layout: &self.depth_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth.view),
                }],
            });
            self.depth_group = (Some(depth.version), group);
        }
        // The splashes' columns and the bows' sheets and bursts, farthest first: one march blends them back to front.
        let eye = DVec3::from(ctx.camera.position.map(f64::from));
        self.marched.clear();
        self.marched
            .extend_from_slice(self.splashes.columns.instances());
        for (_, column) in self
            .bow_spray
            .bursts
            .instances()
            .iter()
            .chain(self.bow_spray.sheets.instances())
        {
            self.marched.push(*column);
        }
        let away = |column: &ColumnGpu| (column.bounds().0 - eye).length_squared();
        self.marched.sort_by(|a, b| away(b).total_cmp(&away(a)));
        let columns = self.marched.as_slice();
        self.columns.divisor = crate::columns::march_divisor(
            columns,
            glam::Vec3::from(ctx.camera.position),
            -glam::Vec3::from_slice(&ctx.camera.inverse_view[8..11]),
            ctx.viewport.1 as f32 / (2.0 * ctx.camera.tan_half_fov_y.max(1e-4)),
            self.columns.divisor == 1,
        );
        let mut uniform = self.lighting.uniform();
        uniform.direct[3] = f32::from(u8::from(self.sky_bound));
        uniform.column_sampling[0] = self.columns.divisor as f32;
        ctx.uploads
            .write_buffer(&self.lighting_buffer, 0, bytemuck::bytes_of(&uniform));
        let sheets = self.blasts.instances();
        self.blast_count = sheets.len() as u32;
        if !sheets.is_empty() {
            ctx.uploads
                .write_buffer(&self.blast_buffer, 0, cast_slice(sheets));
        }
        self.columns.count = columns.len() as u32;
        // Marched again at this frame's backdrop copy point, if any column stands.
        self.columns.marched = false;
        if !columns.is_empty() {
            if !self.columns.filled {
                let mut pass = ctx
                    .encoder
                    .begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("splash column noise"),
                        timestamp_writes: None,
                    });
                pass.set_pipeline(ctx.pipelines.compute(self.columns.fill));
                pass.set_bind_group(0, &self.columns.fill_group, &[]);
                let groups = NOISE_SIZE.div_ceil(4);
                pass.dispatch_workgroups(groups, groups, groups);
                self.columns.filled = true;
            }
            ctx.uploads
                .write_buffer(&self.columns.buffer, 0, cast_slice(columns));
        }
        let sprites: [&[WaterSprite]; 2] = [&self.splashes.drops, &self.bow_spray.billow_sprites];
        for (batch, data) in self.batches.iter_mut().zip(sprites) {
            batch.count = data.len() as u32;
            // Through the frame's staging belt: no fresh staging buffer per frame.
            ctx.uploads.write_buffer(&batch.buffer, 0, cast_slice(data));
        }
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase != Phase::Transparent {
            return;
        }
        // The sheets lie on the sea, under every splash.
        if self.blast_count > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.blast_pipeline));
            pass.set_bind_group(1, &self.blast_group, &[]);
            pass.set_bind_group(2, &self.sky_group, &[]);
            pass.set_bind_group(3, &self.depth_group.1, &[]);
            pass.set_vertex_buffer(0, self.blast_buffer.slice(..));
            pass.draw(0..4, 0..self.blast_count);
        }
        // The columns, marched at the backdrop's copy point, laid over the scene.
        if self.columns.marched
            && let Some((_, group)) = self
                .columns
                .target
                .as_ref()
                .and_then(|h| h.composite.as_ref())
        {
            pass.set_pipeline(ctx.pipelines.render(self.columns.composite));
            pass.set_bind_group(1, group, &[]);
            pass.draw(0..3, 0..1);
        }
        for batch in &self.batches {
            if batch.count == 0 {
                continue;
            }
            let which = match batch.material {
                Material::Billows => 0,
                Material::Drops => 1,
            };
            pass.set_pipeline(ctx.pipelines.render(self.pipelines[which]));
            pass.set_bind_group(1, &self.groups[which], &[]);
            pass.set_bind_group(2, &self.sky_group, &[]);
            pass.set_bind_group(3, &self.depth_group.1, &[]);
            pass.set_vertex_buffer(0, batch.buffer.slice(..));
            pass.draw(0..4, 0..batch.count);
        }
    }

    /// The columns marched into their target, farthest first (36 corners of each one's box), once this
    /// frame's backdrop depth (the hulls and the sea) is taken: the march stops at it. The target is then published
    /// as the cover ([`WaterEffects::cover`]) for the combat effects, which read it at this copy point after this
    /// layer.
    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        if point != CopyPoint::Backdrop {
            return;
        }
        self.columns.marched = false;
        self.march(ctx);
        self.cover.publish(
            self.columns
                .target
                .as_ref()
                .map(|t| naval_render::CoverTarget {
                    color: t.color.clone(),
                    depth: t.lies.clone(),
                    version: t.version,
                }),
        );
    }

    /// A splash's column shades the sea, the hulls and the other spray: each column in view draws into every map but
    /// the land maps (which only the land casts into) as a screen door of the sun it stops
    /// (`shaders/column_caster.wgsl`). The bow's spray is too thin to cast.
    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.cast_shadows
            || map.kind.is_land()
            || self.columns.count == 0
            || !self.columns.filled
        {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(self.columns.caster));
        pass.set_bind_group(1, &self.columns.group, &[]);
        pass.draw(0..36, 0..self.columns.count);
    }
}

impl WaterEffects {
    /// The columns into their target ([`SceneLayer::after_copies`]); without a column the target, if it still holds
    /// the last march, is cleared once.
    fn march(&mut self, ctx: &mut PrepareContext<'_>) {
        if self.columns.count == 0 || !self.columns.filled {
            if let Some(target) = self.columns.target.as_mut().filter(|t| t.holds) {
                let cleared = |view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })
                };
                ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("splash columns clear"),
                    color_attachments: &[cleared(&target.color), cleared(&target.lies)],
                    ..Default::default()
                });
                target.holds = false;
            }
            return;
        }
        let Some(depth) = ctx.resources.texture(SCENE_BACKDROP_DEPTH) else {
            return;
        };
        if self.depth_group.0 != Some(depth.version) {
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("water effects backdrop depth"),
                layout: &self.depth_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth.view),
                }],
            });
            self.depth_group = (Some(depth.version), group);
        }
        let full = depth.desc.size;
        let divisor = self.columns.divisor;
        let size = (
            full.0.div_ceil(divisor).max(1),
            full.1.div_ceil(divisor).max(1),
        );
        if self
            .columns
            .target
            .as_ref()
            .is_none_or(|half| half.size != size)
        {
            let texture = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
                ctx.device
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
                        usage,
                        view_formats: &[],
                    })
                    .create_view(&wgpu::TextureViewDescriptor::default())
            };
            self.cover_version += 1;
            self.columns.target = Some(ColumnTarget {
                size,
                color: texture(
                    "splash columns target",
                    COLUMN_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                ),
                lies: texture(
                    "splash columns target lies",
                    COLUMN_DEPTH_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                ),
                version: self.cover_version,
                holds: false,
                depth: texture(
                    "splash columns target depth",
                    naval_render::DEPTH_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT,
                ),
                composite: None,
            });
        }
        let half = self.columns.target.as_mut().expect("made above");
        if half.composite.as_ref().map(|(version, _)| *version) != Some(depth.version) {
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("splash columns composite"),
                layout: &self.columns.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&half.color),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth.view),
                    },
                ],
            });
            half.composite = Some((depth.version, group));
        }
        half.holds = true;
        let cleared = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("splash columns"),
            color_attachments: &[cleared(&half.color), cleared(&half.lies)],
            // Reversed depth: cleared to the far plane, so every box's face passes.
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &half.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: ctx.timer.render_pass("splash columns"),
            ..Default::default()
        });
        pass.set_pipeline(ctx.pipelines.render(self.columns.march));
        pass.set_bind_group(0, ctx.camera_group, &[]);
        pass.set_bind_group(1, &self.columns.group, &[]);
        pass.set_bind_group(2, &self.sky_group, &[]);
        pass.set_bind_group(3, &self.depth_group.1, &[]);
        pass.draw(0..36, 0..self.columns.count);
        drop(pass);
        self.columns.marched = true;
    }
}

/// A 1 × 1 depth at the far plane (reversed depth: 0), which fades no sprite, until the backdrop's copy is taken.
fn placeholder_depth(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("water effects placeholder depth"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water effects placeholder depth"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    })
}

/// The sun at noon for tests and stills: toward the south-east, high.
pub fn default_sun() -> DVec3 {
    DVec3::new(-0.55, 0.74, -0.39).normalize()
}

/// Group 2 of the spray's pipelines: the sky's environment atlas, its sampler and its uniforms.
fn sky_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let fragment = wgpu::ShaderStages::FRAGMENT;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("water effects sky"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: fragment,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: fragment,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
        ],
    })
}

fn sky_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    atlas: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniforms: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("water effects sky"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(atlas),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniforms.as_entire_binding(),
            },
        ],
    })
}

/// No sky yet: a black 1×1 atlas, a sampler and zeroed uniforms (never read while the lighting's flag is 0).
fn placeholder_sky(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> wgpu::BindGroup {
    let atlas = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("water effects placeholder sky"),
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
        label: Some("water effects placeholder sky"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("water effects placeholder sky"),
        size: std::mem::size_of::<naval_ocean::environment::EnvironmentUniforms>() as u64,
        usage: wgpu::BufferUsages::UNIFORM,
        mapped_at_creation: false,
    });
    sky_group(device, layout, &atlas, &sampler, &uniforms)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shader composes through the renderer core's library and passes naga's validation, without a GPU.
    #[test]
    fn every_shader_composes_and_validates() {
        let mut library = naval_render::ShaderLibrary::new();
        library
            .add(naval_sky::shaders::ENVIRONMENT_FILE)
            .unwrap_or_else(|error| panic!("{error:?}"));
        for file in SHADERS {
            library
                .add(file)
                .unwrap_or_else(|error| panic!("{error:?}"));
        }
        for entry in [
            SPRITE_SHADER,
            COLUMN_SHADER,
            COLUMN_CASTER_SHADER,
            COLUMN_NOISE_SHADER,
            COLUMN_COMPOSITE_SHADER,
            BLAST_SHADER,
        ] {
            let module = library
                .compose(entry.path, &Vec::new())
                .unwrap_or_else(|error| panic!("{error:?}"));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::default(),
            )
            .validate(&module)
            .unwrap_or_else(|error| panic!("{}: {error:?}", entry.path));
        }
    }
}
