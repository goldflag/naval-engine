//! The buildings as a renderer-core layer: the chunks the host hands in ([`BuildingsLayer::queue_chunk`]), uploaded a
//! few megabytes a frame from its own `prepare`, each placed by a translation (its corner in the theatre plus the
//! layer's offset), culled against the view and drawn in the `Opaque` phase after the land; into the sun's near and
//! wide maps when shadows are on. A far chunk ([`Lod::Far`]) is its masses, instances of one shape ([`super::far`]),
//! and casts nothing. A chunk handed in at another level than it is drawn at fades in over the old one for [`FADE_S`]
//! (each pixel to one of the two), and the old one goes once it has.
//!
//! The layer chooses nothing: which chunks stand about the camera, and at which level of detail, is the host's
//! (`naval-client`'s `theatre::buildings`), from [`BuildingsLayer::camera_theatre`] and [`BuildingSettings`].

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use naval_ocean::FogParameters;
use naval_render::{
    CasterPipeline, DrawContext, Phase, PipelineId, PrepareContext, RenderContext, SceneLayer,
    ScenePipeline, ShaderDefValue, ShaderError, ShadowMap, ShadowMapKind, UploadSink, VertexLayout,
    shader_file,
};

use super::clutter::{CLUTTER_M, Clutter, SHAPE_REACH, SHAPES, ShapeVertex, shape_mesh};
use super::facades::{self, Layers};
use super::far::{FLAT_INDICES, Mass, far_shape};
use super::{ChunkKey, ChunkMesh, Lod, Period, chunk_corner};
use crate::layer::{TerrainLight, cloud_shadow_layout, frustum_planes};
use crate::lod::TerrainQuality;
use naval_conditions::mesh_light_shares;

/// Most chunks a layer holds at once, those fading out with them (the boxes' 10 km is about 320 chunks of 1 km, the far
/// level's 30 km about 2,900).
pub const CHUNK_CAPACITY: usize = 4096;
/// How long a chunk changing its level takes to fade from the old level to the new (s).
pub const FADE_S: f32 = 0.6;
/// The margin about each level's distance (m), so a chunk does not flicker between two.
pub const MARGIN_M: f32 = 250.0;
/// Bytes of chunks uploaded a frame at most.
pub const UPLOAD_BUDGET: usize = 8 << 20;

/// How far buildings stand, whether they cast, and how much they carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildingSettings {
    /// Within this distance (m, camera to the chunk) a chunk is drawn whole ([`Lod::Full`]), beyond it as boxes.
    pub full_m: f32,
    /// No boxes past this (m, camera to the chunk).
    pub range_m: f32,
    /// Past the boxes, a chunk within this distance across the ground (m, the camera's height not counted) stands as its
    /// town's masses ([`Lod::Far`]); none past it (the satellite photograph carries the town there). 0: no far level.
    pub far_m: f32,
    /// Into the sun's near and wide maps.
    pub cast_shadows: bool,
    /// Walls and windows from the facade textures ([`super::facades`]); without, the shader's drawn windows.
    pub facades: bool,
    /// The roof clutter ([`super::clutter`]) within [`CLUTTER_M`], into the sun's near map and the nearest land map
    /// when casting.
    pub clutter: bool,
    /// The age the buildings are drawn in ([`Period`]): the game's Graphics menu and the scenario's year.
    pub period: Period,
}

impl BuildingSettings {
    /// High and Ultra: whole buildings to 3.5 km, boxes to 10 km, the far level's masses to 30 km across the ground,
    /// casting, textured, with their roof clutter.
    pub const HIGH: Self = Self {
        full_m: 3500.0,
        range_m: 10_000.0,
        far_m: 30_000.0,
        cast_shadows: true,
        facades: true,
        clutter: true,
        period: Period::Today,
    };
    /// Medium: whole to 1.5 km, boxes to 6 km, no far level (from 4 km up its masses would stand from 4.4 km, large on
    /// the screen: 1-2.7 ms over Istanbul to 16 km), no shadows, the shader's windows, no clutter.
    pub const MEDIUM: Self = Self {
        full_m: 1500.0,
        range_m: 6000.0,
        far_m: 0.0,
        cast_shadows: false,
        facades: false,
        clutter: false,
        period: Period::Today,
    };

    /// By the game's terrain setting: Medium's (no trees) draws fewer.
    pub fn for_quality(quality: &TerrainQuality) -> Self {
        if quality.trees {
            Self::HIGH
        } else {
            Self::MEDIUM
        }
    }

    /// The same, drawn in `period`.
    pub fn with_period(self, period: Period) -> Self {
        Self { period, ..self }
    }

    /// The level of detail for a chunk `distance` metres off (from the camera, its height counted) and `across` metres
    /// off across the ground, `held` the one it is drawn at now (a margin about each line keeps it from flickering
    /// there): whole and boxes by `distance`, as before the far level; then the far level by `across`, so a camera
    /// climbing keeps its ring of masses; none past it.
    pub fn lod_at(&self, distance: f32, across: f32, held: Option<Lod>) -> Option<Lod> {
        let margin = MARGIN_M;
        let range = match held {
            Some(Lod::Far) => self.range_m - margin,
            Some(_) => self.range_m + margin,
            None => self.range_m,
        };
        if distance <= range {
            let full = match held {
                Some(Lod::Full) => self.full_m + margin,
                _ => self.full_m,
            };
            return Some(if distance <= full {
                Lod::Full
            } else {
                Lod::Box
            });
        }
        let far = if held.is_some() {
            self.far_m + margin
        } else {
            self.far_m
        };
        (self.far_m > 0.0 && across <= far).then_some(Lod::Far)
    }

    /// The farthest a chunk is drawn across the ground (m): the far level's range, or the boxes'.
    pub fn reach_m(&self) -> f32 {
        self.range_m.max(self.far_m) + MARGIN_M
    }

    /// Where the far level's blocks take over from their runs (m across the ground): the first block a fortieth of the
    /// way out over the far level, the last a tenth, each by its hash, its runs standing on a fortieth past it (High:
    /// 10.5-12 km, the runs to 12.5 km; with `NAVAL_BUILDINGS_FAR=16` on Medium 6.25-7 km, to 7.25). Drawing the
    /// distant masses is bound by their count, a run's as dear as a block's, and a block stands for six to ten runs.
    pub fn blocks(&self) -> [f32; 3] {
        let stretch = (self.far_m - self.range_m).max(0.0);
        [
            self.range_m + 0.025 * stretch,
            0.075 * stretch,
            0.025 * stretch,
        ]
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Uniforms {
    sun_direction: [f32; 4],
    sun_radiance: [f32; 4],
    fill_sky: [f32; 4],
    fill_ground: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    params: [f32; 4],
    /// The far level's blocks ([`BuildingSettings::blocks`]): where they take over, the stretch, the runs' overlap.
    far: [f32; 4],
}

/// The hours since dusk a layer takes until it is told the sun ([`BuildingsLayer::set_sun`]): 22:00, its windows lit
/// about as they were before the hour counted.
pub const EVENING_HOURS: f32 = 4.0;

/// The hours since 18:00 by the sun toward `sun` about the celestial pole `pole` (world: x east, y up, z south): its
/// hour angle, from the meridian through the zenith, taken from -6 (noon) through 6 (midnight) to 18 (the next noon), so
/// a winter evening before 18:00 is still evening and a dark winter morning after 06:00 still morning. [`EVENING_HOURS`]
/// at a pole, where the sun's hour tells nothing.
pub fn hours_after_dusk(sun: Vec3, pole: Vec3) -> f32 {
    use std::f32::consts::PI;
    let pole = pole.normalize_or(Vec3::Y);
    let up = Vec3::Y - Vec3::Y.dot(pole) * pole;
    if up.length() < 1e-3 {
        return EVENING_HOURS;
    }
    let up = up.normalize();
    let east = pole.cross(up);
    // The hour angle: 0 at noon, positive in the afternoon (the sun west).
    let h = (-sun.dot(east)).atan2(sun.dot(up));
    let hours = h * 12.0 / PI - 6.0;
    if hours < -6.0 { hours + 24.0 } else { hours }
}

/// Added to a chunk translation's w (its region) when it is drawn in a past period (`buildings.wgsl` reads bit 5).
pub const PAST_BIT: u8 = 32;
/// A chunk translation's w over this is its fade (`buildings.wgsl`'s `fade_hidden`): bit 0 whether it fades out, the
/// rest the share (in 255ths) hidden fading in or gone fading out (nine bits).
pub const FADE_UNIT: u32 = 64;
/// Added to a far chunk's translation's w ([`Lod::Far`]): its towers' boxes sink into the ground over the far level's
/// last stretch as its masses do (`buildings.wgsl`'s `vs_building`, bit 15), about the translation's y, their lowest
/// foot.
pub const FAR_BIT: u32 = 1 << 15;

/// A chunk translation's w: its region, plus [`PAST_BIT`] when `past`, plus its `fade` ([`fade_code`]) over
/// [`FADE_UNIT`], plus [`FAR_BIT`] when `far`. A whole number under 2^24, exact in an f32.
fn translation_w(region: u8, past: bool, fade: u32, far: bool) -> f32 {
    f32::from(region + if past { PAST_BIT } else { 0 })
        + (fade * FADE_UNIT + if far { FAR_BIT } else { 0 }) as f32
}

/// A chunk's fade at `t` (0-1) of the way through it: fading in, the share still hidden; fading out, the share gone
/// (`buildings.wgsl`'s `fade_hidden`), the two at one moment complements to the 255th; 0 when it is drawn whole.
pub fn fade_code(out: bool, t: f32) -> u32 {
    let hidden = (255.0 * (1.0 - t.clamp(0.0, 1.0))).round() as u32;
    if out {
        ((255 - hidden) << 1) | 1
    } else {
        hidden << 1
    }
}

/// A chunk on the GPU (none of its buffers when it holds no building).
struct GpuChunk {
    lod: Lod,
    corner: [f64; 2],
    buffers: Option<(wgpu::Buffer, wgpu::Buffer)>,
    index_count: u32,
    min: [f32; 3],
    max: [f32; 3],
    slot: u32,
    /// Its [`super::Region::index`] (the translation's w).
    region: u8,
    /// Drawn in a past period (the translation's w over [`PAST_BIT`]): the shader's period styling.
    past: bool,
    /// Its roof clutter's instances (sorted by shape, the slot written in), and how many of each shape.
    clutter: Option<wgpu::Buffer>,
    clutter_counts: [u32; SHAPES],
    buildings: u32,
    bytes: usize,
    /// The mesh it was uploaded from (for [`BuildingsLayer::holds_mesh`]).
    source: std::sync::Weak<ChunkMesh>,
    /// A far chunk's masses (its slot written in), how many, and how many of them are its runs (its blocks after).
    masses: Option<wgpu::Buffer>,
    mass_count: u32,
    far_runs: u32,
    /// Fading in over the level it replaced, since then.
    fading_in: Option<Instant>,
}

impl GpuChunk {
    /// Whether it draws anything.
    fn draws(&self) -> bool {
        self.buffers.is_some() || self.masses.is_some()
    }
}

/// The far level's shape and pipelines.
struct FarDraw {
    pipeline: PipelineId,
    fading: PipelineId,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
}

/// The clutter's shapes and pipelines (the settings' `clutter`).
struct ClutterDraw {
    pipeline: PipelineId,
    caster: PipelineId,
    /// The caster's group 1: the chunks' translations.
    caster_group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    ranges: [(u32, u32); SHAPES],
}

/// What the layer holds, for the host's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildingCounts {
    /// Chunks uploaded, those with buildings, and those queued.
    pub chunks: usize,
    pub with_buildings: usize,
    pub queued: usize,
    /// Buildings and triangles uploaded, those drawn last frame, and the GPU bytes held.
    pub buildings: usize,
    pub triangles: usize,
    pub drawn_chunks: usize,
    pub drawn_triangles: usize,
    /// Roof clutter in the chunks drawn with it last frame (each piece past [`CLUTTER_M`] dropped on the GPU).
    pub drawn_clutter: usize,
    pub bytes: usize,
    /// Far chunks uploaded and their masses, those drawn last frame, and chunks fading between two levels.
    pub far_chunks: usize,
    pub masses: usize,
    pub drawn_far_chunks: usize,
    pub drawn_masses: usize,
    /// The far chunks' towers' triangles uploaded, and those drawn last frame (in `triangles` and `drawn_triangles`
    /// too).
    pub far_triangles: usize,
    pub drawn_far_triangles: usize,
    pub fading: usize,
}

/// The buildings of one scene.
pub struct BuildingsLayer {
    settings: BuildingSettings,
    /// World x, z = theatre x, z + offset.
    offset: [f64; 2],
    uniforms: Uniforms,
    uniform_buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    environment: wgpu::BindGroup,
    cloud_group: wgpu::BindGroup,
    cloud_placeholder: wgpu::BindGroup,
    cloud_bound: bool,
    fog: FogParameters,
    pipeline: PipelineId,
    /// The same with BUILDINGS_FADE: for the chunks fading between two levels.
    fading_pipeline: PipelineId,
    caster: PipelineId,
    clutter: Option<ClutterDraw>,
    far: Option<FarDraw>,
    /// Whether the far chunks are drawn (`set_far_drawn`, to measure them).
    far_drawn: bool,
    chunks: HashMap<[i32; 2], GpuChunk>,
    /// The levels chunks are fading out of (each with when its fade began).
    fading_out: HashMap<[i32; 2], (GpuChunk, Instant)>,
    /// The chunks queued, oldest first, each with its serial; and the mesh queued for each chunk with its serial (an
    /// entry in the order whose serial is not the chunk's is a mesh replaced since).
    queued: VecDeque<([i32; 2], u64)>,
    pending: HashMap<[i32; 2], (u64, ChunkKey, Arc<ChunkMesh>)>,
    serial: u64,
    translations: wgpu::Buffer,
    free_slots: Vec<u32>,
    translations_dirty: bool,
    /// Chunks in view at the last `prepare`, nearest first, and the levels fading out in view.
    visible: Vec<[i32; 2]>,
    visible_out: Vec<[i32; 2]>,
    /// Those of them with clutter within its distance, and how far each is from the camera (m).
    clutter_visible: Vec<([i32; 2], f32)>,
    /// The camera's frustum's four sides at the last `prepare`: the land maps take only the chunks whose shadows can
    /// fall into the view (`draw_shadow`).
    view_sides: Vec<[f64; 4]>,
    /// The camera at the last `prepare`, theatre x and z and world height.
    camera: Option<[f64; 3]>,
    /// The share of the lit rooms left unshaded (`set_windows`).
    windows: f32,
    /// The share of the shop signs' light let out (`set_signs`).
    signs: f32,
    /// The hours since dusk by the sun (`set_sun`, [`hours_after_dusk`]): how many lamps still burn.
    night_hours: f32,
    /// Each chunk's obstruction lights (`night::beacons_of`, chunk-local), and a count of their changes.
    beacons: HashMap<[i32; 2], Vec<[f32; 3]>>,
    beacons_version: u64,
}

/// The least height a chunk's shadow is taken to run from (m), at the sun's elevation (its own height where taller): past
/// the view's sides by that run, a chunk can still shade what the camera sees.
const SHADOW_RUN_HEIGHT_M: f32 = 150.0;
/// The farthest that run is taken (m): a low sun throws a tower's shadow without end.
const SHADOW_RUN_MAX_M: f32 = 1500.0;

const STRIDE: u64 = std::mem::size_of::<super::BuildingVertex>() as u64;

fn vertex_layouts() -> Vec<VertexLayout> {
    let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
        format,
        offset,
        shader_location,
    };
    vec![
        VertexLayout {
            stride: STRIDE,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: vec![
                attribute(wgpu::VertexFormat::Float32x3, 0, 0),
                attribute(wgpu::VertexFormat::Snorm8x4, 12, 1),
                attribute(wgpu::VertexFormat::Float16x2, 16, 2),
                attribute(wgpu::VertexFormat::Uint8x4, 20, 3),
            ],
        },
        VertexLayout {
            stride: 16,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: vec![attribute(wgpu::VertexFormat::Float32x4, 0, 4)],
        },
    ]
}

/// The clutter's: a [`ShapeVertex`] and, per instance, a [`Clutter`] (`buildings.wgsl`'s `ClutterIn`); the caster's
/// without the normal and style.
fn clutter_layouts(caster: bool) -> Vec<VertexLayout> {
    let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
        format,
        offset,
        shader_location,
    };
    let mut shape = vec![attribute(wgpu::VertexFormat::Float32x3, 0, 0)];
    let mut instance = vec![
        attribute(wgpu::VertexFormat::Float32x4, 0, 2),
        attribute(wgpu::VertexFormat::Float32x3, 16, 3),
        attribute(wgpu::VertexFormat::Uint32, 28, 4),
    ];
    if !caster {
        shape.push(attribute(wgpu::VertexFormat::Snorm8x4, 12, 1));
        instance.push(attribute(wgpu::VertexFormat::Uint8x4, 32, 5));
    }
    vec![
        VertexLayout {
            stride: std::mem::size_of::<ShapeVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: shape,
        },
        VertexLayout {
            stride: std::mem::size_of::<Clutter>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: instance,
        },
    ]
}

/// The far level's: a [`ShapeVertex`] of the far shape and, per instance, a [`Mass`] (`buildings.wgsl`'s `FarIn`).
fn far_layouts() -> Vec<VertexLayout> {
    let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
        format,
        offset,
        shader_location,
    };
    vec![
        VertexLayout {
            stride: std::mem::size_of::<ShapeVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: vec![
                attribute(wgpu::VertexFormat::Float32x3, 0, 0),
                attribute(wgpu::VertexFormat::Snorm8x4, 12, 1),
            ],
        },
        VertexLayout {
            stride: std::mem::size_of::<Mass>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: vec![
                attribute(wgpu::VertexFormat::Float32, 0, 2),
                attribute(wgpu::VertexFormat::Float16x4, 4, 3),
                attribute(wgpu::VertexFormat::Float16x2, 12, 4),
                attribute(wgpu::VertexFormat::Uint8x4, 16, 5),
                attribute(wgpu::VertexFormat::Uint32, 20, 6),
            ],
        },
    ]
}

/// A texture array of `layers` with every level, as the facade textures' (or a 1 × 1 placeholder's).
fn array_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    layers: &Layers,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: layers.size,
            height: layers.size,
            depth_or_array_layers: layers.layers,
        },
        mip_level_count: layers.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut side = layers.size;
    for (level, bytes) in layers.levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: layers.layers,
            },
        );
        side = (side / 2).max(1);
    }
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2Array,
            multisampled: false,
        },
        count: None,
    }
}

fn translations_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

impl BuildingsLayer {
    /// The layer with no chunks, lit by the sky's `environment` bind group (`naval_sky::EnvironmentMap::bind_group`).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        environment: &wgpu::BindGroup,
        settings: BuildingSettings,
    ) -> Result<Self, ShaderError> {
        crate::layer::register_shaders(ctx)?;
        for file in [
            shader_file!("shaders/buildings.wgsl"),
            shader_file!("shaders/buildings_caster.wgsl"),
        ] {
            ctx.add_shader(file)?;
        }
        let device = ctx.device.clone();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("buildings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                translations_entry(5),
            ],
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("buildings uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let translations = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("buildings chunk translations"),
            size: (CHUNK_CAPACITY * 16) as u64,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // The facade textures on High and Ultra (drawn once a run), 1 × 1 placeholders without.
        let blank = Layers {
            size: 1,
            layers: 1,
            levels: vec![vec![128; 4]],
        };
        let (walls, windows, window_surface) = if settings.facades {
            let f = facades::baked();
            (&f.walls, &f.windows, &f.window_surface)
        } else {
            (&blank, &blank, &blank)
        };
        let facade_views = [
            array_texture(&device, ctx.queue, "buildings facade walls", walls),
            array_texture(&device, ctx.queue, "buildings facade windows", windows),
            array_texture(
                &device,
                ctx.queue,
                "buildings facade window surface",
                window_surface,
            ),
        ];
        let facade_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("buildings facades"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("buildings"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&facade_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&facade_views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&facade_views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&facade_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: translations.as_entire_binding(),
                },
            ],
        });
        let defs = if settings.facades {
            vec![("FACADE_TEXTURES".to_owned(), ShaderDefValue::Bool(true))]
        } else {
            Vec::new()
        };
        let environment_layout = naval_sky::environment_bind_group_layout(&device);
        let cloud_layout = cloud_shadow_layout(&device);
        let placeholder = crate::textures::texture_with_levels(
            &device,
            ctx.queue,
            "buildings cloud shadow placeholder",
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            1,
            &[&[255, 255, 255, 255]],
            wgpu::TextureUsages::empty(),
        );
        let placeholder_view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("buildings cloud shadow placeholder"),
            ..Default::default()
        });
        let placeholder_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("buildings cloud shadow placeholder"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let cloud_placeholder = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("buildings cloud shadow placeholder"),
            layout: &cloud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: placeholder_uniforms.as_entire_binding(),
                },
            ],
        });
        let pipeline = ctx.add_scene_pipeline(
            "terrain buildings",
            "naval-terrain/shaders/buildings.wgsl",
            ScenePipeline {
                vertex_buffers: vertex_layouts(),
                bind_group_layouts: vec![
                    layout.clone(),
                    environment_layout.clone(),
                    cloud_layout.clone(),
                ],
                defs: defs.clone(),
                ..ScenePipeline::new("vs_building", "fs_building")
            },
        )?;
        // The chunks fading between two levels: the same, each pixel to one level or the other.
        let mut fading_defs = defs.clone();
        fading_defs.push(("BUILDINGS_FADE".to_owned(), ShaderDefValue::Bool(true)));
        let groups = vec![
            layout.clone(),
            environment_layout.clone(),
            cloud_layout.clone(),
        ];
        let fading_pipeline = ctx.add_scene_pipeline(
            "terrain buildings fading",
            "naval-terrain/shaders/buildings.wgsl",
            ScenePipeline {
                vertex_buffers: vertex_layouts(),
                bind_group_layouts: groups.clone(),
                defs: fading_defs.clone(),
                ..ScenePipeline::new("vs_building", "fs_building")
            },
        )?;
        let far = if settings.far_m > 0.0 {
            let pipeline = ctx.add_scene_pipeline(
                "terrain buildings far",
                "naval-terrain/shaders/buildings.wgsl",
                ScenePipeline {
                    vertex_buffers: far_layouts(),
                    bind_group_layouts: groups.clone(),
                    defs: defs.clone(),
                    ..ScenePipeline::new("vs_far", "fs_building")
                },
            )?;
            let fading = ctx.add_scene_pipeline(
                "terrain buildings far fading",
                "naval-terrain/shaders/buildings.wgsl",
                ScenePipeline {
                    vertex_buffers: far_layouts(),
                    bind_group_layouts: groups,
                    defs: fading_defs,
                    ..ScenePipeline::new("vs_far", "fs_building")
                },
            )?;
            let (vertices, mut indices) = far_shape();
            let index_count = indices.len() as u32;
            // An even count of u16 indices keeps the write a multiple of four bytes.
            if indices.len() % 2 == 1 {
                indices.push(0);
            }
            let buffer = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes.len() as u64,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                ctx.queue.write_buffer(&buffer, 0, bytes);
                buffer
            };
            Some(FarDraw {
                pipeline,
                fading,
                vertices: buffer(
                    "buildings far shape",
                    bytemuck::cast_slice(&vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(
                    "buildings far shape indices",
                    bytemuck::cast_slice(&indices),
                    wgpu::BufferUsages::INDEX,
                ),
                index_count,
            })
        } else {
            None
        };
        let caster = ctx.add_caster_pipeline(
            "terrain buildings caster",
            "naval-terrain/shaders/buildings_caster.wgsl",
            CasterPipeline {
                vertex_buffers: vertex_layouts(),
                ..CasterPipeline::new("vs_building_caster")
            },
        )?;
        let clutter = if settings.clutter {
            let pipeline = ctx.add_scene_pipeline(
                "terrain buildings clutter",
                "naval-terrain/shaders/buildings.wgsl",
                ScenePipeline {
                    vertex_buffers: clutter_layouts(false),
                    bind_group_layouts: vec![layout, environment_layout, cloud_layout],
                    defs,
                    ..ScenePipeline::new("vs_clutter", "fs_building")
                },
            )?;
            let caster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("buildings clutter caster"),
                entries: &[translations_entry(0)],
            });
            let caster_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("buildings clutter caster"),
                layout: &caster_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: translations.as_entire_binding(),
                }],
            });
            let caster = ctx.add_caster_pipeline(
                "terrain buildings clutter caster",
                "naval-terrain/shaders/buildings_caster.wgsl",
                CasterPipeline {
                    vertex_buffers: clutter_layouts(true),
                    bind_group_layouts: vec![caster_layout],
                    defs: vec![("CLUTTER_CASTER".to_owned(), ShaderDefValue::Bool(true))],
                    ..CasterPipeline::new("vs_clutter_caster")
                },
            )?;
            let shapes = shape_mesh();
            let buffer = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(label),
                    size: bytes.len() as u64,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                ctx.queue.write_buffer(&buffer, 0, bytes);
                buffer
            };
            // An even count of u16 indices keeps the write a multiple of four bytes.
            let mut indices = shapes.indices;
            if indices.len() % 2 == 1 {
                indices.push(0);
            }
            Some(ClutterDraw {
                pipeline,
                caster,
                caster_group,
                vertices: buffer(
                    "buildings clutter shapes",
                    bytemuck::cast_slice(&shapes.vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                indices: buffer(
                    "buildings clutter shape indices",
                    bytemuck::cast_slice(&indices),
                    wgpu::BufferUsages::INDEX,
                ),
                ranges: shapes.ranges,
            })
        } else {
            None
        };
        let mut layer = Self {
            settings,
            offset: [0.0, 0.0],
            uniforms: Uniforms::zeroed(),
            uniform_buffer,
            group,
            environment: environment.clone(),
            cloud_group: cloud_placeholder.clone(),
            cloud_placeholder,
            cloud_bound: false,
            fog: FogParameters {
                color: [0.7, 0.75, 0.8],
                start: 0.0,
                end: 1e9,
                power: 1.0,
                sky_blend_distance: 1.0,
            },
            pipeline,
            fading_pipeline,
            caster,
            clutter,
            far,
            far_drawn: true,
            chunks: HashMap::new(),
            fading_out: HashMap::new(),
            queued: VecDeque::new(),
            pending: HashMap::new(),
            serial: 0,
            translations,
            free_slots: (0..CHUNK_CAPACITY as u32).rev().collect(),
            translations_dirty: true,
            visible: Vec::new(),
            visible_out: Vec::new(),
            clutter_visible: Vec::new(),
            view_sides: Vec::new(),
            camera: None,
            windows: 1.0,
            signs: 1.0,
            night_hours: EVENING_HOURS,
            beacons: HashMap::new(),
            beacons_version: 0,
        };
        layer.set_light(&TerrainLight {
            light: naval_sky::CelestialLight {
                direction: Vec3::new(0.0, 1.0, 0.0),
                color: Vec3::ONE,
                intensity: 5.0,
                night: false,
            },
            ambient: 0.65,
            flash: 0.0,
        });
        Ok(layer)
    }

    pub fn settings(&self) -> BuildingSettings {
        self.settings
    }

    /// Where the theatre stands in the world: world x, z = theatre x, z + `offset`.
    pub fn set_offset(&mut self, offset: [f64; 2]) {
        if self.offset != offset {
            self.offset = offset;
            self.translations_dirty = true;
            self.beacons_version += 1;
        }
    }

    pub fn offset(&self) -> [f64; 2] {
        self.offset
    }

    /// The clouds' shadow (naval-clouds' `CloudShadow` bind group), or none.
    pub fn set_cloud_shadow(&mut self, bind_group: Option<&wgpu::BindGroup>) {
        self.cloud_bound = bind_group.is_some();
        self.cloud_group = bind_group
            .cloned()
            .unwrap_or_else(|| self.cloud_placeholder.clone());
    }

    /// The scene fog, as the land takes it.
    pub fn set_fog(&mut self, fog: FogParameters) {
        self.fog = fog;
    }

    /// The celestial light this frame, as the land takes it ([`crate::TerrainLayer::set_light`]); the windows light up
    /// as the sun goes down.
    pub fn set_light(&mut self, light: &TerrainLight) {
        let celestial = light.light;
        let (sun_share, fill_share, sky_share) = mesh_light_shares(
            f64::from(celestial.intensity),
            celestial.night,
            f64::from(light.flash),
        );
        let radiance = celestial.color * celestial.intensity * sun_share as f32;
        let fill = light.ambient * fill_share as f32;
        let (sky, ground) = naval_conditions::MESH_FILL_COLORS;
        let sky = naval_conditions::srgb_hex(sky);
        let ground = naval_conditions::srgb_hex(ground);
        let direction = celestial.direction.normalize_or(Vec3::Y);
        let night = if celestial.night {
            1.0
        } else {
            ((0.10 - direction.y) / 0.14).clamp(0.0, 1.0)
        };
        let u = &mut self.uniforms;
        u.sun_direction = direction.extend(night).to_array();
        u.sun_radiance = [radiance.x, radiance.y, radiance.z, sky_share as f32];
        u.fill_sky = [sky[0] * fill, sky[1] * fill, sky[2] * fill, 0.0];
        u.fill_ground = [ground[0] * fill, ground[1] * fill, ground[2] * fill, 0.0];
    }

    /// Hands in chunk `key`'s mesh (made on any thread), uploaded from a later `prepare` in place of whatever the chunk
    /// holds (at any level of detail), which keeps drawing until then (and, at another level, fades out under it). A
    /// newer mesh for the same chunk queued meanwhile replaces this one, at the back of the queue.
    pub fn queue_chunk(&mut self, key: ChunkKey, mesh: Arc<ChunkMesh>) {
        self.serial += 1;
        self.pending.insert(key.index, (self.serial, key, mesh));
        self.queued.push_back((key.index, self.serial));
    }

    /// The oldest mesh still queued.
    fn next_queued(&mut self) -> Option<(ChunkKey, Arc<ChunkMesh>)> {
        while let Some((index, serial)) = self.queued.pop_front() {
            if self.pending.get(&index).is_some_and(|p| p.0 == serial)
                && let Some((_, key, mesh)) = self.pending.remove(&index)
            {
                return Some((key, mesh));
            }
        }
        None
    }

    /// Whether chunk `key` is drawn or queued at its level of detail.
    pub fn holds(&self, key: ChunkKey) -> bool {
        match self.pending.get(&key.index) {
            Some((_, k, _)) => *k == key,
            None => self
                .chunks
                .get(&key.index)
                .is_some_and(|c| c.lod == key.lod),
        }
    }

    /// Whether chunk `key` is drawn or queued from this very mesh (a remade mesh for the same chunk, say with the map's
    /// tags it lacked, is not held until it is queued).
    pub fn holds_mesh(&self, key: ChunkKey, mesh: &Arc<ChunkMesh>) -> bool {
        match self.pending.get(&key.index) {
            Some((_, k, m)) => *k == key && Arc::ptr_eq(m, mesh),
            None => self.chunks.get(&key.index).is_some_and(|c| {
                c.lod == key.lod && std::ptr::eq(c.source.as_ptr(), Arc::as_ptr(mesh))
            }),
        }
    }

    /// Whether the far chunks are drawn (on, but for measuring them).
    pub fn set_far_drawn(&mut self, drawn: bool) {
        self.far_drawn = drawn;
    }

    /// The level of detail chunk `index` is drawn at, if any.
    pub fn held_lod(&self, index: [i32; 2]) -> Option<Lod> {
        self.chunks.get(&index).map(|c| c.lod)
    }

    /// The chunks drawn (uploaded), any level of detail.
    pub fn held(&self) -> Vec<[i32; 2]> {
        self.chunks.keys().copied().collect()
    }

    /// Drops chunk `index` (and anything queued for it, and a level it fades out of).
    pub fn remove_chunk(&mut self, index: [i32; 2]) {
        self.pending.remove(&index);
        if let Some(chunk) = self.chunks.remove(&index) {
            self.free_slots.push(chunk.slot);
        }
        if let Some((chunk, _)) = self.fading_out.remove(&index) {
            self.free_slots.push(chunk.slot);
        }
        self.visible_out.retain(|i| *i != index);
        if self.beacons.remove(&index).is_some() {
            self.beacons_version += 1;
        }
        self.visible.retain(|i| *i != index);
        self.clutter_visible.retain(|(i, _)| *i != index);
    }

    /// The camera at the last frame: theatre x and z, and its height (m).
    pub fn camera_theatre(&self) -> Option<[f64; 3]> {
        self.camera
    }

    pub fn counts(&self) -> BuildingCounts {
        let mut c = BuildingCounts {
            chunks: self.chunks.len(),
            queued: self.pending.len(),
            fading: self.fading_out.len(),
            ..Default::default()
        };
        for chunk in self.chunks.values() {
            if chunk.draws() {
                c.with_buildings += 1;
            }
            c.buildings += chunk.buildings as usize;
            c.triangles += chunk.index_count as usize / 3;
            c.bytes += chunk.bytes;
            if chunk.lod == Lod::Far {
                c.far_chunks += 1;
                c.masses += chunk.mass_count as usize;
                c.far_triangles += chunk.index_count as usize / 3;
            }
        }
        for (chunk, _) in self.fading_out.values() {
            c.bytes += chunk.bytes;
        }
        for index in &self.visible {
            let Some(chunk) = self.chunks.get(index) else {
                continue;
            };
            if chunk.lod == Lod::Far {
                if self.far_shown() {
                    c.drawn_far_chunks += 1;
                    c.drawn_masses += self.mass_range(chunk).len();
                    c.drawn_far_triangles += chunk.index_count as usize / 3;
                    c.drawn_triangles += chunk.index_count as usize / 3;
                }
            } else {
                c.drawn_chunks += 1;
                c.drawn_triangles += chunk.index_count as usize / 3;
            }
        }
        for (index, d) in &self.clutter_visible {
            if let Some(chunk) = self.chunks.get(index) {
                c.drawn_clutter += chunk
                    .clutter_counts
                    .iter()
                    .zip(SHAPE_REACH)
                    .filter(|(_, reach)| *d <= *reach)
                    .map(|(n, _)| *n)
                    .sum::<u32>() as usize;
            }
        }
        c
    }

    /// Uploads everything queued now (a layer being built, off the frame loop's thread).
    pub fn flush(&mut self, ctx: &mut RenderContext<'_>) {
        let mut sink = UploadSink::Queue(ctx.queue);
        while let Some((key, mesh)) = self.next_queued() {
            self.upload(ctx.device, &mut sink, key, &mesh);
        }
    }

    /// Whether the far chunks are drawn: the level made (its settings' `far_m`) and not turned off.
    fn far_shown(&self) -> bool {
        self.far.is_some() && self.far_drawn
    }

    /// The masses of a far chunk to draw from where the camera stands: its runs while any could stand nearer than
    /// their blocks take over, its blocks once any could have (`buildings.wgsl`'s `vs_far` picks between the two).
    fn mass_range(&self, chunk: &GpuChunk) -> std::ops::Range<u32> {
        match self.camera {
            Some([x, z, _]) => mass_range(
                &self.settings,
                [x, z],
                chunk.corner,
                chunk.far_runs,
                chunk.mass_count,
            ),
            None => 0..chunk.mass_count,
        }
    }

    /// A chunk's buffers made and written through `sink` (the frame's staging belt in `prepare`, so up to
    /// [`UPLOAD_BUDGET`] a frame touches no fresh staging pages); its bytes.
    fn upload(
        &mut self,
        device: &wgpu::Device,
        sink: &mut UploadSink<'_>,
        key: ChunkKey,
        mesh: &Arc<ChunkMesh>,
    ) -> usize {
        // At another level than it is drawn at, the new level fades in over the old, which keeps its slot until it has
        // gone; with no slot to spare, it swaps at once. A remade mesh at the same level carries on any fade under way
        // (as does one replacing a level that drew nothing: the fade out of the level before it goes on).
        let (slot, fading_in) = match self.chunks.remove(&key.index) {
            Some(old) if old.lod != key.lod && old.draws() => {
                if let Some((older, _)) = self.fading_out.remove(&key.index) {
                    self.free_slots.push(older.slot);
                }
                match self.free_slots.pop() {
                    Some(slot) => {
                        let now = Instant::now();
                        self.fading_out.insert(key.index, (old, now));
                        (slot, Some(now))
                    }
                    None => (old.slot, None),
                }
            }
            Some(old) => (old.slot, old.fading_in),
            None => match self.free_slots.pop() {
                Some(slot) => (slot, None),
                None => return 0,
            },
        };
        let mut make = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            sink.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let buffers = (!mesh.indices.is_empty()).then(|| {
            (
                make(
                    "buildings chunk vertices",
                    bytemuck::cast_slice(&mesh.vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                make(
                    "buildings chunk indices",
                    bytemuck::cast_slice(&mesh.indices),
                    wgpu::BufferUsages::INDEX,
                ),
            )
        });
        // The clutter with its chunk's slot written in (none drawn without the setting).
        let clutter = (self.settings.clutter && !mesh.clutter.is_empty()).then(|| {
            let placed: Vec<Clutter> = mesh
                .clutter
                .iter()
                .map(|c| Clutter {
                    shape: (c.shape & 0xff) | (slot << 8),
                    ..*c
                })
                .collect();
            make(
                "buildings chunk clutter",
                bytemuck::cast_slice(&placed),
                wgpu::BufferUsages::VERTEX,
            )
        });
        // A far chunk's masses, the slot written in likewise.
        let masses = (!mesh.masses.is_empty()).then(|| {
            let placed: Vec<Mass> = mesh
                .masses
                .iter()
                .map(|m| Mass {
                    turn: (m.turn & 0xffff) | (slot << 16),
                    ..*m
                })
                .collect();
            make(
                "buildings chunk masses",
                bytemuck::cast_slice(&placed),
                wgpu::BufferUsages::VERTEX,
            )
        });
        // The bytes uploaded (the clutter's only when drawn).
        let bytes = mesh.bytes()
            - if clutter.is_none() {
                std::mem::size_of_val(mesh.clutter.as_slice())
            } else {
                0
            };
        self.chunks.insert(
            key.index,
            GpuChunk {
                lod: key.lod,
                corner: chunk_corner(key.index),
                buffers,
                index_count: mesh.indices.len() as u32,
                min: mesh.min,
                max: mesh.max,
                slot,
                region: mesh.region,
                past: mesh.period.is_past(),
                clutter,
                clutter_counts: mesh.clutter_counts,
                buildings: mesh.buildings,
                bytes,
                source: Arc::downgrade(mesh),
                masses,
                mass_count: mesh.masses.len() as u32,
                far_runs: mesh.far_runs.min(mesh.masses.len() as u32),
                fading_in,
            },
        );
        self.translations_dirty = true;
        let beacons = crate::night::beacons_of(mesh);
        if !(beacons.is_empty() && !self.beacons.contains_key(&key.index)) {
            self.beacons.insert(key.index, beacons);
            self.beacons_version += 1;
        }
        bytes.max(1)
    }

    /// The share of the lit rooms a scenario's rule leaves unshaded, 0-1 (a dimout's or blackout's:
    /// `night::LightScales::windows`; the rest show through their shades or cloth, `buildings.wgsl`'s `rule_light`).
    pub fn set_windows(&mut self, share: f32) {
        self.windows = share.clamp(0.0, 1.0);
    }

    /// The share of Taiwan's shop signs' light a scenario's rule lets out, 0-1 (`night::LightScales::signs`: dimmed
    /// under a dimout, out under a blackout; `buildings.wgsl`'s `sign_surface`).
    pub fn set_signs(&mut self, share: f32) {
        self.signs = share.clamp(0.0, 1.0);
    }

    /// Toward the sun and the celestial pole (world: x east, y up, z south), whichever body lights the scene: the lamps
    /// in the windows go out as the night wears on (`buildings.wgsl`'s `evening_share`).
    pub fn set_sun(&mut self, sun: Vec3, pole: Vec3) {
        self.night_hours = hours_after_dusk(sun, pole);
    }

    /// A count that changes whenever the obstruction lights do (`beacons`).
    pub fn beacons_version(&self) -> u64 {
        self.beacons_version
    }

    /// The red obstruction lights on the chunks drawn (`night::beacons_of`): world x, height over the sea and world z
    /// of each.
    pub fn beacons(&self) -> Vec<[f64; 3]> {
        let mut out = Vec::new();
        for (index, beacons) in &self.beacons {
            let corner = chunk_corner(*index);
            out.extend(beacons.iter().map(|b| {
                [
                    corner[0] + self.offset[0] + f64::from(b[0]),
                    f64::from(b[1]),
                    corner[1] + self.offset[1] + f64::from(b[2]),
                ]
            }));
        }
        out
    }

    /// Its corner in the world, and its region (w; plus [`PAST_BIT`] in a past period, plus its `fade`).
    fn translation(&self, chunk: &GpuChunk, fade: u32) -> [f32; 4] {
        let t = self.corner(chunk);
        let far = chunk.lod == Lod::Far;
        // A far chunk's y: its towers' lowest foot, which they sink to (`vs_building`); `vs_far` reads x and z alone.
        let y = if far { chunk.min[1] } else { t.y };
        [
            t.x,
            y,
            t.z,
            translation_w(chunk.region, chunk.past, fade, far),
        ]
    }

    /// Its corner in the world.
    fn corner(&self, chunk: &GpuChunk) -> Vec3 {
        Vec3::new(
            (chunk.corner[0] + self.offset[0]) as f32,
            0.0,
            (chunk.corner[1] + self.offset[1]) as f32,
        )
    }

    /// The chunks' translations, each with its fade at `now`.
    fn write_translations(&self, sink: &mut UploadSink<'_>, now: Instant) {
        let t = |start: Instant| now.saturating_duration_since(start).as_secs_f32() / FADE_S;
        let mut data = vec![[0f32; 4]; CHUNK_CAPACITY];
        for chunk in self.chunks.values() {
            let fade = chunk
                .fading_in
                .map_or(0, |start| fade_code(false, t(start)));
            data[chunk.slot as usize] = self.translation(chunk, fade);
        }
        for (chunk, start) in self.fading_out.values() {
            data[chunk.slot as usize] = self.translation(chunk, fade_code(true, t(*start)));
        }
        sink.write_buffer(&self.translations, 0, bytemuck::cast_slice(&data));
    }

    /// Ends the fades done by `now`: the levels faded out go, and those that replaced them are drawn whole. (A chunk
    /// fades in only while the level it replaced fades out, from the same moment.)
    fn end_fades(&mut self, now: Instant) {
        let ended: Vec<[i32; 2]> = self
            .fading_out
            .iter()
            .filter(|(_, (_, start))| now.saturating_duration_since(*start).as_secs_f32() >= FADE_S)
            .map(|(i, _)| *i)
            .collect();
        for index in ended {
            if let Some((chunk, _)) = self.fading_out.remove(&index) {
                self.free_slots.push(chunk.slot);
            }
            if let Some(chunk) = self.chunks.get_mut(&index) {
                chunk.fading_in = None;
            }
            self.translations_dirty = true;
        }
    }

    /// The chunk's bounds in the world.
    fn bounds(&self, chunk: &GpuChunk) -> (Vec3, Vec3) {
        let t = self.corner(chunk);
        (
            Vec3::from_array(chunk.min) + t,
            Vec3::from_array(chunk.max) + t,
        )
    }

    fn draw_chunk(&self, pass: &mut wgpu::RenderPass<'_>, chunk: &GpuChunk) {
        if let Some((vertices, indices)) = &chunk.buffers {
            let at = u64::from(chunk.slot) * 16;
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_vertex_buffer(1, self.translations.slice(at..at + 16));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..chunk.index_count, 0, 0..1);
        }
    }

    /// The chunks' clutter, shape by shape, each chunk's shapes within their reach of the camera at its distance (the
    /// pipeline and groups set; the shapes' buffers bound here).
    fn draw_clutter(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draw: &ClutterDraw,
        chunks: &[([i32; 2], f32)],
    ) {
        pass.set_vertex_buffer(0, draw.vertices.slice(..));
        pass.set_index_buffer(draw.indices.slice(..), wgpu::IndexFormat::Uint16);
        for (index, distance) in chunks {
            let Some(chunk) = self.chunks.get(index) else {
                continue;
            };
            let Some(instances) = &chunk.clutter else {
                continue;
            };
            pass.set_vertex_buffer(1, instances.slice(..));
            let mut first = 0;
            for (shape, count) in chunk.clutter_counts.iter().enumerate() {
                if *count > 0 && *distance <= SHAPE_REACH[shape] {
                    let (start, len) = draw.ranges[shape];
                    pass.draw_indexed(start..start + len, 0, first..first + count);
                }
                first += count;
            }
        }
    }

    /// The clutter into the sun's near map and the nearest land map (whole chunks within its distance of the camera in
    /// the map's box); the farther maps would not resolve it.
    fn draw_clutter_shadow(
        &self,
        map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        let Some(draw) = &self.clutter else {
            return;
        };
        if !self.settings.cast_shadows
            || !matches!(
                map.kind,
                ShadowMapKind::Near | ShadowMapKind::Aboard | ShadowMapKind::Land(0)
            )
        {
            return;
        }
        let Some([x, z, y]) = self.camera else {
            return;
        };
        let eye = Vec3::new(
            (x + self.offset[0]) as f32,
            y as f32,
            (z + self.offset[1]) as f32,
        );
        let planes = frustum_planes(map.projection * map.view);
        let chunks: Vec<([i32; 2], f32)> = self
            .chunks
            .iter()
            .filter(|(_, c)| c.clutter.is_some())
            .filter_map(|(i, c)| {
                let (lo, hi) = self.bounds(c);
                let d = eye.clamp(lo, hi).distance(eye);
                (d <= CLUTTER_M && in_planes(&planes[..4], lo, hi)).then_some((*i, d))
            })
            .collect();
        if chunks.is_empty() {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(draw.caster));
        pass.set_bind_group(1, &draw.caster_group, &[]);
        self.draw_clutter(pass, draw, &chunks);
    }
}

/// The masses to draw of a far chunk at theatre `corner` holding `runs` runs then its blocks (`count` in all), seen
/// from theatre point `camera` ([`BuildingsLayer::mass_range`]).
fn mass_range(
    settings: &BuildingSettings,
    camera: [f64; 2],
    corner: [f64; 2],
    runs: u32,
    count: u32,
) -> std::ops::Range<u32> {
    // A mass reaches a little past its chunk (a block's middle, a run's end).
    let pad = 150.0;
    let lo = [corner[0] - pad, corner[1] - pad];
    let hi = [
        corner[0] + super::CHUNK_M + pad,
        corner[1] + super::CHUNK_M + pad,
    ];
    let near = |p: f64, a: f64, b: f64| (a - p).max(p - b).max(0.0);
    let far = |p: f64, a: f64, b: f64| (p - a).abs().max((b - p).abs());
    let [x, z] = camera;
    let nearest = near(x, lo[0], hi[0]).hypot(near(z, lo[1], hi[1])) as f32;
    let farthest = far(x, lo[0], hi[0]).hypot(far(z, lo[1], hi[1])) as f32;
    let [takes_over, stretch, overlap] = settings.blocks();
    let start = if nearest <= takes_over + stretch + overlap {
        0
    } else {
        runs
    };
    let end = if farthest > takes_over { count } else { runs };
    start..end.max(start)
}

/// Whether the box `lo`-`hi` lies at least partly inside `planes` (inside where n · p + w ≥ 0).
fn in_planes(planes: &[[f64; 4]], lo: Vec3, hi: Vec3) -> bool {
    planes.iter().all(|p| {
        // The box's corner farthest along the plane's normal.
        let x = if p[0] >= 0.0 { hi.x } else { lo.x };
        let y = if p[1] >= 0.0 { hi.y } else { lo.y };
        let z = if p[2] >= 0.0 { hi.z } else { lo.z };
        p[0] * f64::from(x) + p[1] * f64::from(y) + p[2] * f64::from(z) + p[3] >= 0.0
    })
}

impl SceneLayer for BuildingsLayer {
    fn label(&self) -> &str {
        "buildings"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let mut budget = UPLOAD_BUDGET;
        let mut sink = UploadSink::Belt(&mut *ctx.uploads);
        while budget > 0
            && let Some((key, mesh)) = self.next_queued()
        {
            budget = budget.saturating_sub(self.upload(ctx.device, &mut sink, key, &mesh));
        }
        // While a chunk fades its translation changes each frame (its fade in w).
        let now = Instant::now();
        self.end_fades(now);
        if self.translations_dirty || !self.fading_out.is_empty() {
            self.translations_dirty = false;
            self.write_translations(&mut sink, now);
        }
        let eye = Vec3::from_array(ctx.camera.position);
        self.camera = Some([
            f64::from(eye.x) - self.offset[0],
            f64::from(eye.z) - self.offset[1],
            f64::from(eye.y),
        ]);
        let planes = frustum_planes(Mat4::from_cols_array(&ctx.camera.view_projection));
        self.view_sides.clear();
        self.view_sides.extend_from_slice(&planes[..4]);
        let far_shown = self.far_shown();
        let shown = |c: &GpuChunk| {
            if c.lod == Lod::Far {
                far_shown && c.draws()
            } else {
                c.buffers.is_some()
            }
        };
        let mut visible: Vec<(f32, [i32; 2])> = self
            .chunks
            .iter()
            .filter(|(_, c)| shown(c))
            .filter_map(|(index, c)| {
                let (lo, hi) = self.bounds(c);
                in_planes(&planes, lo, hi).then(|| (eye.clamp(lo, hi).distance(eye), *index))
            })
            .collect();
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.visible_out = self
            .fading_out
            .iter()
            .filter(|(_, (c, _))| shown(c))
            .filter(|(_, (c, _))| {
                let (lo, hi) = self.bounds(c);
                in_planes(&planes, lo, hi)
            })
            .map(|(i, _)| *i)
            .collect();
        self.clutter_visible = visible
            .iter()
            .filter(|(d, i)| {
                *d <= CLUTTER_M && self.chunks.get(i).is_some_and(|c| c.clutter.is_some())
            })
            .map(|(d, i)| (*i, *d))
            .collect();
        self.visible = visible.into_iter().map(|(_, i)| i).collect();

        let fog = &self.fog;
        let u = &mut self.uniforms;
        u.fill_sky[3] = if self.cloud_bound { 1.0 } else { 0.0 };
        u.fill_ground[3] = self.signs;
        u.fog_color = [fog.color[0], fog.color[1], fog.color[2], fog.power];
        u.fog = [fog.start, fog.end, fog.sky_blend_distance, 1.0];
        u.params = [
            self.settings.full_m,
            self.settings.range_m,
            1.0 - self.windows,
            self.settings.far_m,
        ];
        let [takes_over, stretch, overlap] = self.settings.blocks();
        u.far = [takes_over, stretch, overlap, self.night_hours];
        sink.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase != Phase::Opaque || (self.visible.is_empty() && self.visible_out.is_empty()) {
            return;
        }
        pass.set_bind_group(1, &self.group, &[]);
        pass.set_bind_group(2, &self.environment, &[]);
        pass.set_bind_group(3, &self.cloud_group, &[]);
        // Whole and boxed chunks and the far chunks' towers, nearest first; then those fading (in over a level, and the
        // levels fading out).
        let drawn = self.visible.iter().filter_map(|i| self.chunks.get(i));
        let fading_out = self
            .visible_out
            .iter()
            .filter_map(|i| self.fading_out.get(i).map(|(c, _)| c));
        let mut set = None;
        for (fading, chunk) in drawn
            .clone()
            .filter(|c| c.buffers.is_some() && c.fading_in.is_none())
            .map(|c| (false, c))
            .chain(
                drawn
                    .clone()
                    .filter(|c| c.buffers.is_some() && c.fading_in.is_some())
                    .chain(fading_out.clone().filter(|c| c.buffers.is_some()))
                    .map(|c| (true, c)),
            )
        {
            if set != Some(fading) {
                let pipeline = if fading {
                    self.fading_pipeline
                } else {
                    self.pipeline
                };
                pass.set_pipeline(ctx.pipelines.render(pipeline));
                set = Some(fading);
            }
            self.draw_chunk(pass, chunk);
        }
        if let Some(draw) = &self.clutter
            && !self.clutter_visible.is_empty()
        {
            pass.set_pipeline(ctx.pipelines.render(draw.pipeline));
            self.draw_clutter(pass, draw, &self.clutter_visible);
        }
        // The far chunks, their masses instances of the one shape.
        if let Some(far) = &self.far
            && self.far_drawn
        {
            let mut set = None;
            for (fading, chunk) in drawn
                .clone()
                .filter(|c| c.lod == Lod::Far && c.fading_in.is_none())
                .map(|c| (false, c))
                .chain(
                    drawn
                        .filter(|c| c.lod == Lod::Far && c.fading_in.is_some())
                        .chain(fading_out.filter(|c| c.lod == Lod::Far))
                        .map(|c| (true, c)),
                )
            {
                let Some(masses) = &chunk.masses else {
                    continue;
                };
                let range = self.mass_range(chunk);
                if range.is_empty() {
                    continue;
                }
                if set.is_none() {
                    pass.set_vertex_buffer(0, far.vertices.slice(..));
                    pass.set_index_buffer(far.indices.slice(..), wgpu::IndexFormat::Uint16);
                }
                if set != Some(fading) {
                    let pipeline = if fading { far.fading } else { far.pipeline };
                    pass.set_pipeline(ctx.pipelines.render(pipeline));
                    set = Some(fading);
                }
                pass.set_vertex_buffer(1, masses.slice(..));
                // The runs whole, the blocks (flat) without their gables.
                let runs = range.start..range.end.min(chunk.far_runs);
                let blocks = range.start.max(chunk.far_runs)..range.end;
                if !runs.is_empty() {
                    pass.draw_indexed(0..far.index_count, 0, runs);
                }
                if !blocks.is_empty() {
                    pass.draw_indexed(0..FLAT_INDICES, 0, blocks);
                }
            }
        }
    }

    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.settings.cast_shadows
            || !matches!(
                map.kind,
                ShadowMapKind::Near
                    | ShadowMapKind::Aboard
                    | ShadowMapKind::Wide
                    | ShadowMapKind::Land(_)
            )
            || self.chunks.is_empty()
        {
            return;
        }
        let planes = frustum_planes(map.projection * map.view);
        // The land maps square the camera's view of the land across the light: much of each square lies beside the
        // view. They take a chunk only where its shadow can fall into the view: inside the view's sides moved out by
        // the run of a tall building's shadow: a metre of height's run (the sun's cotangent), taken over the chunk's own
        // height or a 150 m building's, whichever is taller (a 508 m tower's chunk casts from farther off).
        let run = if map.kind.is_land() {
            let toward = map.view.inverse().z_axis.truncate().normalize_or(Vec3::Y);
            let sine = toward.y.max(1e-3);
            Some((1.0 - sine * sine).max(0.0).sqrt() / sine)
        } else {
            None
        };
        // Reversed-depth orthographic maps: the far plane may cut casters standing behind the light box's back; the
        // near plane must not (the casters toward the sun). Both are kept: the render core fits the box about the
        // casters it knows, and buildings stand within the land's heights.
        let mut first = true;
        for chunk in self.chunks.values() {
            // The far level casts nothing (its towers neither: their translation's y is their foot, `translation`).
            if chunk.buffers.is_none() || chunk.lod == Lod::Far {
                continue;
            }
            let (lo, hi) = self.bounds(chunk);
            if !in_planes(&planes[..4], lo, hi) {
                continue;
            }
            if let Some(per_m) = run
                && let run = f64::from(
                    ((hi.y - lo.y).max(SHADOW_RUN_HEIGHT_M) * per_m).min(SHADOW_RUN_MAX_M),
                )
                && !self
                    .view_sides
                    .iter()
                    .all(|p| in_planes(&[[p[0], p[1], p[2], p[3] + run]], lo, hi))
            {
                continue;
            }
            if first {
                pass.set_pipeline(ctx.pipelines.render(self.caster));
                first = false;
            }
            self.draw_chunk(pass, chunk);
        }
        self.draw_clutter_shadow(map, pass, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_night_is_told_by_the_suns_bearing() {
        // The sun at `hour` (sun time) at latitude `lat`, declination `dec` (degrees): world x east, y up, z south.
        let sun = |hour: f32, lat: f32, dec: f32| {
            let (h, p, d) = (
                (hour - 12.0) * std::f32::consts::PI / 12.0,
                lat.to_radians(),
                dec.to_radians(),
            );
            let east = -d.cos() * h.sin();
            let north = p.cos() * d.sin() - p.sin() * d.cos() * h.cos();
            let up = p.sin() * d.sin() + p.cos() * d.cos() * h.cos();
            Vec3::new(east, up, -north)
        };
        for (lat, dec) in [
            (50.0, 23.4),
            (50.0, -23.4),
            (64.0, 21.0),
            (-9.0, 22.0),
            (35.0, 0.0),
        ] {
            let pole = Vec3::new(0.0, f32::to_radians(lat).sin(), -f32::to_radians(lat).cos());
            // A winter's dark 17:00 still evening, a dark 07:00 or 08:00 still morning.
            for hour in [17.0, 19.0, 21.0, 22.5, 24.0, 25.5, 27.0, 29.5, 31.0, 32.0] {
                let got = hours_after_dusk(sun(hour, lat, dec), pole);
                assert!(
                    (got - (hour - 18.0)).abs() < 0.01,
                    "{lat} {dec} {hour}: {got}"
                );
            }
        }
        // At a pole the hour is left at the evening's.
        assert_eq!(hours_after_dusk(Vec3::X, Vec3::Y), EVENING_HOURS);
    }

    #[test]
    fn the_level_of_detail_keeps_a_margin() {
        let s = BuildingSettings::HIGH;
        // On the ground (distance and across the same).
        let at = |d: f32, held| s.lod_at(d, d, held);
        assert_eq!(at(1000.0, None), Some(Lod::Full));
        assert_eq!(at(3600.0, None), Some(Lod::Box));
        // Held whole, it stays whole a little past the line; held as boxes, it turns whole only inside it.
        assert_eq!(at(3600.0, Some(Lod::Full)), Some(Lod::Full));
        assert_eq!(at(3400.0, Some(Lod::Box)), Some(Lod::Full));
        // Past the boxes, the far level; held as boxes, they stay boxes a little past the line, held far, it turns
        // to boxes only a little inside it.
        assert_eq!(at(10_100.0, None), Some(Lod::Far));
        assert_eq!(at(10_100.0, Some(Lod::Box)), Some(Lod::Box));
        assert_eq!(at(10_400.0, Some(Lod::Box)), Some(Lod::Far));
        assert_eq!(at(9900.0, Some(Lod::Far)), Some(Lod::Far));
        assert_eq!(at(9700.0, Some(Lod::Far)), Some(Lod::Box));
        // The far level's own edge, with its margin held.
        assert_eq!(at(30_100.0, None), None);
        assert_eq!(at(30_100.0, Some(Lod::Far)), Some(Lod::Far));
        assert_eq!(at(30_300.0, Some(Lod::Far)), None);
        // Without the far level (as before it), nothing past the boxes.
        let plain = BuildingSettings { far_m: 0.0, ..s };
        assert_eq!(plain.lod_at(10_100.0, 10_100.0, None), None);
        assert_eq!(
            plain.lod_at(10_100.0, 10_100.0, Some(Lod::Box)),
            Some(Lod::Box)
        );
        assert_eq!(plain.lod_at(10_400.0, 10_400.0, Some(Lod::Box)), None);
        assert_eq!(plain.reach_m(), 10_250.0);
        assert_eq!(s.reach_m(), 30_250.0);
    }

    #[test]
    fn a_climbing_camera_keeps_its_far_ring() {
        // From 4 km up, the whole and boxed levels count the height, so the boxes reach about 9.2 km across the
        // ground; the far level counts the ground alone, out to its 30 km whatever the height.
        let s = BuildingSettings::HIGH;
        let height = 4000.0f32;
        let d = |across: f32| (across * across + height * height).sqrt();
        assert_eq!(s.lod_at(d(2000.0), 2000.0, None), Some(Lod::Box));
        assert_eq!(s.lod_at(d(9000.0), 9000.0, None), Some(Lod::Box));
        assert_eq!(s.lod_at(d(9500.0), 9500.0, None), Some(Lod::Far));
        assert_eq!(s.lod_at(d(29_000.0), 29_000.0, None), Some(Lod::Far));
        assert_eq!(s.lod_at(d(31_000.0), 31_000.0, None), None);
        // Higher than the boxes reach, the far level still stands below.
        assert_eq!(s.lod_at(11_000.0, 500.0, None), Some(Lod::Far));
    }

    #[test]
    fn a_far_chunk_draws_its_runs_near_and_its_blocks_far() {
        let s = BuildingSettings::HIGH;
        assert_eq!(s.blocks(), [10_500.0, 1500.0, 500.0]);
        let medium = BuildingSettings {
            far_m: 16_000.0,
            ..BuildingSettings::MEDIUM
        };
        assert_eq!(medium.blocks(), [6250.0, 750.0, 250.0]);
        // Medium draws no far level of its own.
        assert_eq!(BuildingSettings::MEDIUM.reach_m(), 6250.0);
        // 400 runs, then 70 blocks; the chunk (k, 0) km from a camera at the origin.
        let range = |k: f64| mass_range(&s, [0.0, 0.0], [k * 1000.0, 0.0], 400, 470);
        // Nearer than any block takes over: the runs alone.
        assert_eq!(range(9.0), 0..400);
        // Across the stretch: both, the shader choosing each block's.
        assert_eq!(range(10.0), 0..470);
        assert_eq!(range(12.0), 0..470);
        // Past every block's runs: the blocks alone.
        assert_eq!(range(13.0), 400..470);
        assert_eq!(range(28.0), 400..470);
        // No blocks (a chunk of loose runs): the runs while they stand, then nothing.
        assert_eq!(mass_range(&s, [0.0, 0.0], [25_000.0, 0.0], 30, 30), 30..30);
    }

    #[test]
    fn boxes_are_culled_by_planes() {
        // x ≥ 0 and x ≤ 10.
        let planes = [[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 10.0]];
        assert!(in_planes(
            &planes,
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0)
        ));
        assert!(!in_planes(
            &planes,
            Vec3::new(11.0, 0.0, 0.0),
            Vec3::new(12.0, 1.0, 1.0)
        ));
        assert!(!in_planes(
            &planes,
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::new(-1.0, 1.0, 1.0)
        ));
    }

    #[test]
    fn the_translation_carries_the_region_and_the_period() {
        // Today's w is the region alone, as before the period; a past period's sets bit 5 (`buildings.wgsl`).
        for region in super::super::region::REGIONS {
            let r = region.index();
            assert_eq!(translation_w(r, false, 0, false), f32::from(r));
            let packed = (translation_w(r, true, 0, false) + 0.5) as u32;
            assert_eq!((packed & 31, packed >> 5), (u32::from(r), 1));
            // A fade rides over both and a far chunk's bit over it, the largest exact in an f32.
            for fade in [1, fade_code(false, 0.0), fade_code(true, 1.0)] {
                for past in [false, true] {
                    for far in [false, true] {
                        let packed = (translation_w(r, past, fade, far) + 0.5) as u32;
                        assert_eq!(
                            (
                                packed & 31,
                                (packed >> 5) & 1,
                                (packed >> 6) & 511,
                                packed >> 15
                            ),
                            (u32::from(r), u32::from(past), fade, u32::from(far))
                        );
                    }
                }
            }
        }
        assert!(fade_code(true, 1.0) < 512, "the fade's nine bits");
        assert_eq!(FAR_BIT, FADE_UNIT << 9);
        let shader = include_str!("../../shaders/buildings.wgsl");
        assert!(shader.contains("out.region = packed & 31u;"));
        assert!(shader.contains("out.past = (packed >> 5u) & 1u;"));
        assert!(shader.contains("out.fade = (packed >> 6u) & 511u;"));
        assert!(shader.contains("let far = (packed >> 15u) & 1u;"));
        assert_eq!(FADE_UNIT, 64);
    }

    #[test]
    fn a_fade_hands_each_pixel_from_one_level_to_the_other() {
        // Whole when done; fading in, the share still hidden falls from all to none; fading out, the share gone
        // rises from none to all, and the two at the same moment are complements (`fade_hidden`).
        assert_eq!(fade_code(false, 1.0), 0);
        assert_eq!(fade_code(false, 0.0), 255 << 1);
        assert_eq!(fade_code(true, 0.0), 1);
        assert_eq!(fade_code(true, 1.0), (255 << 1) | 1);
        for t in [0.1f32, 0.37, 0.5, 0.92] {
            let hidden_in = fade_code(false, t) >> 1;
            let gone_out = fade_code(true, t) >> 1;
            assert_eq!(hidden_in + gone_out, 255, "{t}");
        }
        // Past the ends it holds.
        assert_eq!(fade_code(false, 1.5), 0);
        assert_eq!(fade_code(true, -0.5), 1);
    }
}
