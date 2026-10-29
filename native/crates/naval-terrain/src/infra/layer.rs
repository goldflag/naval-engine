//! The infrastructure as a renderer-core layer, as the buildings are ([`crate::buildings::BuildingsLayer`]): the
//! chunks the host hands in ([`InfraLayer::queue_chunk`]), uploaded a few megabytes a frame from its own `prepare`,
//! each placed by a translation, culled against the view and its faces drawn in the `Opaque` phase; into the sun's
//! near and wide maps when shadows are on. A chunk made near ([`Lod::Full`]) draws its structures and its near detail,
//! one made far ([`Lod::Box`]) its structures and the stand-ins; both draw what lies on the ground ([`Group::Decal`]),
//! which casts nothing. Their lines (the power lines' wires and the towers' members afar, [`InfraMesh::lines`]) are
//! chosen alike and drawn in the `Transparent` phase, blended over whatever stands behind them by the share of the
//! pixel they cover, writing no depth and casting nothing. Temporal anti-aliasing follows a pixel by its depth, here
//! the sky's or the ground's behind the line, so in its motion pass a line moving against the sky as the eye travels
//! takes no history there, or it would trail ([`SceneLayer::draw_motion`]).
//!
//! The layer chooses nothing: which chunks stand about the camera, and at which level, is the host's (`naval-client`'s
//! `theatre::infra`), from [`InfraLayer::camera_theatre`] and [`InfraSettings`].

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
use naval_ocean::FogParameters;
use naval_render::{
    CasterPipeline, DrawContext, MotionPipeline, Phase, PipelineId, PrepareContext, RenderContext,
    SceneLayer, ScenePipeline, ShaderError, ShadowMap, ShadowMapKind, UploadSink, VertexLayout,
    shader_file,
};

use super::airfield::RunwaySpan;
use super::mesh::{Group, InfraCounts, InfraMesh, InfraVertex};
use crate::buildings::{ChunkKey, Lod, chunk_corner};
use crate::layer::{TerrainLight, cloud_shadow_layout, frustum_planes};
use crate::lod::TerrainQuality;
use naval_conditions::mesh_light_shares;

/// Most chunks a layer holds at once (an 8 km range is about 220 chunks of 1 km).
pub const CHUNK_CAPACITY: usize = 512;
/// Bytes of chunks uploaded a frame at most.
pub const UPLOAD_BUDGET: usize = 8 << 20;

/// How far the infrastructure stands, how far its near detail, and whether it casts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfraSettings {
    /// Within this distance (m, camera to the chunk) a chunk is made near: containers one by one, boats, hangers.
    pub near_m: f32,
    /// Nothing is drawn past this (m).
    pub range_m: f32,
    /// Within this of the eye a chunk made near draws its containers one by one; past it, as rows (m).
    pub close_m: f32,
    /// Into the sun's near and wide maps.
    pub cast_shadows: bool,
}

impl InfraSettings {
    /// High and Ultra: near detail (boats, hangers) to 1.5 km, containers one by one to 900 m (past it a container is
    /// a few pixels and a row's run reads as well), everything to 8 km, casting.
    pub const HIGH: Self = Self {
        near_m: 1500.0,
        range_m: 8000.0,
        close_m: 900.0,
        cast_shadows: true,
    };
    /// Medium: no near detail (no boats; the stacks as rows), to 4 km, no shadows.
    pub const MEDIUM: Self = Self {
        near_m: 0.0,
        range_m: 4000.0,
        close_m: 0.0,
        cast_shadows: false,
    };

    /// By the game's terrain setting, as the buildings take it.
    pub fn for_quality(quality: &TerrainQuality) -> Self {
        if quality.trees {
            Self::HIGH
        } else {
            Self::MEDIUM
        }
    }

    /// The level for a chunk `distance` metres off, `held` the one it is drawn at now (a margin keeps it from
    /// flickering at the boundary); none past the range.
    pub fn lod_at(&self, distance: f32, held: Option<Lod>) -> Option<Lod> {
        let margin = 200.0;
        let range = if held.is_some() {
            self.range_m + margin
        } else {
            self.range_m
        };
        if distance > range {
            return None;
        }
        let near = match held {
            Some(Lod::Full) => self.near_m + margin,
            _ => self.near_m,
        };
        Some(if self.near_m > 0.0 && distance <= near {
            Lod::Full
        } else {
            Lod::Box
        })
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
}

/// A chunk on the GPU (none of its buffers when it holds nothing).
struct GpuChunk {
    lod: Lod,
    corner: [f64; 2],
    buffers: Option<(wgpu::Buffer, wgpu::Buffer)>,
    ranges: [u32; super::mesh::GROUPS + 1],
    /// The lines' groups ([`InfraMesh::lines`]).
    lines: [u32; super::mesh::GROUPS + 1],
    min: [f32; 3],
    max: [f32; 3],
    slot: u32,
    counts: InfraCounts,
    /// Its runways (theatre metres).
    runways: Vec<RunwaySpan>,
    bytes: usize,
    source: std::sync::Weak<InfraMesh>,
}

/// The index ranges of `table` (a mesh's faces' groups or its lines') a chunk at `lod` draws (`close`: within the close
/// range): what lies on the ground, and the structures with the near detail (a chunk made near; its containers one by
/// one only when close, else the rows) or with the stand-ins (one made far).
fn chosen(
    table: &[u32; super::mesh::GROUPS + 1],
    lod: Lod,
    close: bool,
) -> [std::ops::Range<u32>; 2] {
    let range = |g: Group| table[g as usize]..table[g as usize + 1];
    match (lod, close) {
        (Lod::Full, true) => [
            range(Group::Decal),
            range(Group::Structure).start..range(Group::Near).end,
        ],
        // Decal, Far, Structure and Detail lie in a row.
        (Lod::Full, false) => [range(Group::Decal).start..range(Group::Detail).end, 0..0],
        (Lod::Box | Lod::Far, _) => [range(Group::Decal).start..range(Group::Structure).end, 0..0],
    }
}

impl GpuChunk {
    fn range(&self, g: Group) -> std::ops::Range<u32> {
        self.ranges[g as usize]..self.ranges[g as usize + 1]
    }

    /// The faces' index ranges the scene's opaque pass draws ([`chosen`]).
    fn drawn(&self, close: bool) -> [std::ops::Range<u32>; 2] {
        chosen(&self.ranges, self.lod, close)
    }

    /// The lines' index ranges the transparent pass draws, chosen as the faces are.
    fn drawn_lines(&self, close: bool) -> [std::ops::Range<u32>; 2] {
        chosen(&self.lines, self.lod, close)
    }

    /// The range the shadow maps draw: as [`Self::drawn`], without what lies on the ground.
    fn cast(&self, close: bool) -> std::ops::Range<u32> {
        match (self.lod, close) {
            (Lod::Full, true) => self.range(Group::Structure).start..self.range(Group::Near).end,
            (Lod::Full, false) => self.range(Group::Far).start..self.range(Group::Detail).end,
            (Lod::Box | Lod::Far, _) => {
                self.range(Group::Far).start..self.range(Group::Structure).end
            }
        }
    }

    fn triangles(&self, close: bool) -> usize {
        self.drawn(close)
            .iter()
            .chain(&self.drawn_lines(close))
            .map(|r| r.len())
            .sum::<usize>()
            / 3
    }
}

/// What the layer holds, for the host's status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InfraStatus {
    /// Chunks uploaded, those holding anything, and those queued.
    pub chunks: usize,
    pub with_infra: usize,
    pub queued: usize,
    /// Triangles drawn by the chunks uploaded, those drawn last frame, and the GPU bytes held.
    pub triangles: usize,
    pub drawn_chunks: usize,
    pub drawn_triangles: usize,
    pub bytes: usize,
    /// What the uploaded chunks hold.
    pub held: InfraCounts,
}

/// The infrastructure of one scene.
pub struct InfraLayer {
    settings: InfraSettings,
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
    /// The lines' pipeline: blended, depth tested but not written, unculled.
    lines: PipelineId,
    /// The lines in the motion pass: how much history each pixel of them takes.
    line_motion: PipelineId,
    caster: PipelineId,
    chunks: HashMap<[i32; 2], GpuChunk>,
    queued: VecDeque<(ChunkKey, Arc<InfraMesh>)>,
    translations: wgpu::Buffer,
    free_slots: Vec<u32>,
    translations_dirty: bool,
    /// The chunks in view, nearest first, each with whether it lies within the close range.
    visible: Vec<([i32; 2], bool)>,
    /// Of those, the chunks whose lines may move against the sky by a quarter of a pixel or more as the eye steps
    /// (`draw_motion`): none while it holds still or turns.
    moving: Vec<([i32; 2], bool)>,
    camera: Option<[f64; 3]>,
    /// The eye at the last frame (world metres).
    eye: Option<Vec3>,
    /// The camera's curvature at the last frame (`naval_render::Camera::curvature`).
    curvature: f32,
    /// The share of the boats' cabins lit at night, as the buildings' windows (`set_windows`).
    windows: f32,
}

const STRIDE: u64 = std::mem::size_of::<InfraVertex>() as u64;

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

/// Whether the box `lo`-`hi` lies at least partly inside `planes` (inside where n · p + w ≥ 0).
fn in_planes(planes: &[[f64; 4]], lo: Vec3, hi: Vec3) -> bool {
    planes.iter().all(|p| {
        let x = if p[0] >= 0.0 { hi.x } else { lo.x };
        let y = if p[1] >= 0.0 { hi.y } else { lo.y };
        let z = if p[2] >= 0.0 { hi.z } else { lo.z };
        p[0] * f64::from(x) + p[1] * f64::from(y) + p[2] * f64::from(z) + p[3] >= 0.0
    })
}

impl InfraLayer {
    /// The layer with no chunks, lit by the sky's `environment` bind group (`naval_sky::EnvironmentMap::bind_group`).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        environment: &wgpu::BindGroup,
        settings: InfraSettings,
    ) -> Result<Self, ShaderError> {
        crate::layer::register_shaders(ctx)?;
        for file in [
            shader_file!("shaders/infra.wgsl"),
            shader_file!("shaders/infra_caster.wgsl"),
        ] {
            ctx.add_shader(file)?;
        }
        let device = ctx.device.clone();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("infrastructure"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("infrastructure uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("infrastructure"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });
        let environment_layout = naval_sky::environment_bind_group_layout(&device);
        let cloud_layout = cloud_shadow_layout(&device);
        let placeholder = crate::textures::texture_with_levels(
            &device,
            ctx.queue,
            "infrastructure cloud shadow placeholder",
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            1,
            &[&[255, 255, 255, 255]],
            wgpu::TextureUsages::empty(),
        );
        let placeholder_view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("infrastructure cloud shadow placeholder"),
            ..Default::default()
        });
        let placeholder_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("infrastructure cloud shadow placeholder"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let cloud_placeholder = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("infrastructure cloud shadow placeholder"),
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
            "terrain infrastructure",
            "naval-terrain/shaders/infra.wgsl",
            ScenePipeline {
                vertex_buffers: vertex_layouts(),
                bind_group_layouts: vec![
                    layout.clone(),
                    environment_layout.clone(),
                    cloud_layout.clone(),
                ],
                ..ScenePipeline::new("vs_infra", "fs_infra")
            },
        )?;
        // The lines (`fs_infra_line`): over the finished scene, each as much of what is behind it as it leaves of its
        // pixel; the target's alpha kept, no depth written (a line's depth would hide the effects drawn after it behind
        // all of its pixel). Each segment is one ribbon, seen from either side.
        let lines = ctx.add_scene_pipeline(
            "terrain infrastructure lines",
            "naval-terrain/shaders/infra.wgsl",
            ScenePipeline {
                vertex_buffers: vertex_layouts(),
                bind_group_layouts: vec![layout, environment_layout, cloud_layout],
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_write: false,
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::Zero,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                ..ScenePipeline::new("vs_infra_line", "fs_infra_line")
            },
        )?;
        // The lines in temporal anti-aliasing's motion pass (`vs_infra_line_motion`): the camera alone.
        let line_motion = ctx.add_motion_pipeline(
            "terrain infrastructure line motion",
            "naval-terrain/shaders/infra.wgsl",
            MotionPipeline {
                vertex_buffers: vertex_layouts(),
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                ..MotionPipeline::new("vs_infra_line_motion", "fs_infra_line_motion")
            },
        )?;
        let caster = ctx.add_caster_pipeline(
            "terrain infrastructure caster",
            "naval-terrain/shaders/infra_caster.wgsl",
            CasterPipeline {
                vertex_buffers: vertex_layouts(),
                ..CasterPipeline::new("vs_infra_caster")
            },
        )?;
        let translations = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("infrastructure chunk translations"),
            size: (CHUNK_CAPACITY * 16) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
            lines,
            line_motion,
            caster,
            chunks: HashMap::new(),
            queued: VecDeque::new(),
            translations,
            free_slots: (0..CHUNK_CAPACITY as u32).rev().collect(),
            translations_dirty: true,
            visible: Vec::new(),
            moving: Vec::new(),
            eye: None,
            curvature: 0.0,
            camera: None,
            windows: 1.0,
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

    pub fn settings(&self) -> InfraSettings {
        self.settings
    }

    /// Where the theatre stands in the world: world x, z = theatre x, z + `offset`.
    pub fn set_offset(&mut self, offset: [f64; 2]) {
        if self.offset != offset {
            self.offset = offset;
            self.translations_dirty = true;
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

    /// The share of the boats' cabins lit at night, 0-1 of the usual (a scenario's dimout or blackout darkens them, as
    /// the buildings' windows: `BuildingsLayer::set_windows`).
    pub fn set_windows(&mut self, share: f32) {
        self.windows = share.clamp(0.0, 1.0);
    }

    /// The celestial light this frame, as the buildings take it.
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
    /// holds, which keeps drawing until then.
    pub fn queue_chunk(&mut self, key: ChunkKey, mesh: Arc<InfraMesh>) {
        self.queued.retain(|(k, _)| k.index != key.index);
        self.queued.push_back((key, mesh));
    }

    /// Whether chunk `key` is drawn or queued from this very mesh.
    pub fn holds_mesh(&self, key: ChunkKey, mesh: &Arc<InfraMesh>) -> bool {
        self.queued
            .iter()
            .any(|(k, m)| *k == key && Arc::ptr_eq(m, mesh))
            || self.chunks.get(&key.index).is_some_and(|c| {
                c.lod == key.lod && std::ptr::eq(c.source.as_ptr(), Arc::as_ptr(mesh))
            })
    }

    /// Whether chunk `key` is drawn or queued at its level.
    pub fn holds(&self, key: ChunkKey) -> bool {
        self.queued.iter().any(|(k, _)| *k == key)
            || self
                .chunks
                .get(&key.index)
                .is_some_and(|c| c.lod == key.lod)
    }

    /// The level chunk `index` is drawn at, if any.
    pub fn held_lod(&self, index: [i32; 2]) -> Option<Lod> {
        self.chunks.get(&index).map(|c| c.lod)
    }

    /// The chunks drawn (uploaded).
    pub fn held(&self) -> Vec<[i32; 2]> {
        self.chunks.keys().copied().collect()
    }

    /// Drops chunk `index` (and anything queued for it).
    pub fn remove_chunk(&mut self, index: [i32; 2]) {
        self.queued.retain(|(k, _)| k.index != index);
        if let Some(chunk) = self.chunks.remove(&index) {
            self.free_slots.push(chunk.slot);
        }
        self.visible.retain(|(i, _)| *i != index);
    }

    /// The runways of the chunks held, each once (theatre metres: x east, z south; y the paint's height), for their
    /// lights. A runway is held while any chunk it crosses is.
    pub fn runways(&self) -> Vec<RunwaySpan> {
        let mut out: Vec<RunwaySpan> = Vec::new();
        for chunk in self.chunks.values() {
            for r in &chunk.runways {
                let same = |o: &RunwaySpan| {
                    o.ends
                        .iter()
                        .zip(&r.ends)
                        .all(|(a, b)| (a[0] - b[0]).abs() < 1.0 && (a[2] - b[2]).abs() < 1.0)
                };
                if !out.iter().any(same) {
                    out.push(*r);
                }
            }
        }
        out.sort_by(|a, b| a.ends[0][0].total_cmp(&b.ends[0][0]));
        out
    }

    /// The camera at the last frame: theatre x and z, and its height (m).
    pub fn camera_theatre(&self) -> Option<[f64; 3]> {
        self.camera
    }

    pub fn status(&self) -> InfraStatus {
        let mut s = InfraStatus {
            chunks: self.chunks.len(),
            queued: self.queued.len(),
            ..Default::default()
        };
        for chunk in self.chunks.values() {
            if chunk.buffers.is_some() {
                s.with_infra += 1;
            }
            s.held += chunk.counts;
            s.triangles += chunk.triangles(true);
            s.bytes += chunk.bytes;
        }
        for (index, close) in &self.visible {
            if let Some(chunk) = self.chunks.get(index) {
                s.drawn_chunks += 1;
                s.drawn_triangles += chunk.triangles(*close);
            }
        }
        s
    }

    /// Uploads everything queued now (a layer being built, off the frame loop's thread).
    pub fn flush(&mut self, ctx: &mut RenderContext<'_>) {
        let mut sink = UploadSink::Queue(ctx.queue);
        while let Some((key, mesh)) = self.queued.pop_front() {
            self.upload(ctx.device, &mut sink, key, &mesh);
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        sink: &mut UploadSink<'_>,
        key: ChunkKey,
        mesh: &Arc<InfraMesh>,
    ) -> usize {
        let slot = match self.chunks.remove(&key.index) {
            Some(old) => old.slot,
            None => match self.free_slots.pop() {
                Some(slot) => slot,
                None => return 0,
            },
        };
        let buffers = (!mesh.is_empty()).then(|| {
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
            (
                make(
                    "infrastructure chunk vertices",
                    bytemuck::cast_slice(&mesh.vertices),
                    wgpu::BufferUsages::VERTEX,
                ),
                make(
                    "infrastructure chunk indices",
                    bytemuck::cast_slice(&mesh.indices),
                    wgpu::BufferUsages::INDEX,
                ),
            )
        });
        self.chunks.insert(
            key.index,
            GpuChunk {
                lod: key.lod,
                corner: chunk_corner(key.index),
                buffers,
                ranges: mesh.ranges,
                lines: mesh.lines,
                min: mesh.min,
                max: mesh.max,
                slot,
                counts: mesh.counts,
                runways: mesh
                    .runways
                    .iter()
                    .map(|r| r.moved(chunk_corner(key.index)))
                    .collect(),
                bytes: mesh.bytes(),
                source: Arc::downgrade(mesh),
            },
        );
        self.translations_dirty = true;
        mesh.bytes().max(1)
    }

    fn translation(&self, chunk: &GpuChunk) -> [f32; 4] {
        [
            (chunk.corner[0] + self.offset[0]) as f32,
            0.0,
            (chunk.corner[1] + self.offset[1]) as f32,
            0.0,
        ]
    }

    fn bounds(&self, chunk: &GpuChunk) -> (Vec3, Vec3) {
        let t = self.translation(chunk);
        let t = Vec3::new(t[0], t[1], t[2]);
        (
            Vec3::from_array(chunk.min) + t,
            Vec3::from_array(chunk.max) + t,
        )
    }

    /// A chunk's bounds as drawn from the last frame's eye: lowered by the Earth's curve (`world_to_clip`), its foot by
    /// the drop at its farthest corner and its top by the drop at its nearest point.
    fn drawn_bounds(&self, chunk: &GpuChunk) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = self.bounds(chunk);
        if let Some(eye) = self.eye {
            let near = Vec2::new(eye.x.clamp(lo.x, hi.x), eye.z.clamp(lo.z, hi.z))
                .distance_squared(Vec2::new(eye.x, eye.z));
            let far = (eye.x - lo.x).abs().max((eye.x - hi.x).abs()).powi(2)
                + (eye.z - lo.z).abs().max((eye.z - hi.z).abs()).powi(2);
            lo.y -= self.curvature * far;
            hi.y -= self.curvature * near;
        }
        (lo, hi)
    }

    fn draw_ranges(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        chunk: &GpuChunk,
        ranges: &[std::ops::Range<u32>],
    ) {
        if let Some((vertices, indices)) = &chunk.buffers {
            if ranges.iter().all(|r| r.is_empty()) {
                return;
            }
            let at = u64::from(chunk.slot) * 16;
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.set_vertex_buffer(1, self.translations.slice(at..at + 16));
            pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
            for r in ranges.iter().filter(|r| !r.is_empty()) {
                pass.draw_indexed(r.clone(), 0, 0..1);
            }
        }
    }
}

impl SceneLayer for InfraLayer {
    fn label(&self) -> &str {
        "infrastructure"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let mut budget = UPLOAD_BUDGET;
        let mut sink = UploadSink::Belt(&mut *ctx.uploads);
        while budget > 0
            && let Some((key, mesh)) = self.queued.pop_front()
        {
            budget = budget.saturating_sub(self.upload(ctx.device, &mut sink, key, &mesh));
        }
        if self.translations_dirty {
            self.translations_dirty = false;
            let mut data = vec![[0f32; 4]; CHUNK_CAPACITY];
            for chunk in self.chunks.values() {
                data[chunk.slot as usize] = self.translation(chunk);
            }
            ctx.uploads
                .write_buffer(&self.translations, 0, bytemuck::cast_slice(&data));
        }
        let eye = Vec3::from_array(ctx.camera.position);
        let step = self.eye.map_or(0.0, |last| last.distance(eye));
        self.eye = Some(eye);
        self.curvature = ctx.camera.curvature;
        self.camera = Some([
            f64::from(eye.x) - self.offset[0],
            f64::from(eye.z) - self.offset[1],
            f64::from(eye.y),
        ]);
        let planes = frustum_planes(Mat4::from_cols_array(&ctx.camera.view_projection));
        let mut visible: Vec<(f32, [i32; 2])> = self
            .chunks
            .iter()
            .filter(|(_, c)| c.buffers.is_some())
            .filter_map(|(index, c)| {
                let (lo, hi) = self.drawn_bounds(c);
                in_planes(&planes, lo, hi).then(|| (eye.clamp(lo, hi).distance(eye), *index))
            })
            .collect();
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));
        let close = self.settings.close_m;
        // A line `d` metres off moves against the sky by at most the eye's step over `d` (radians) as the eye travels,
        // and gives up its history from a quarter of a pixel (`vs_infra_line_motion`).
        let per_radian = ctx.camera.viewport[1] / (2.0 * ctx.camera.tan_half_fov_y).max(1e-6);
        self.moving = visible
            .iter()
            .filter(|(d, _)| step * per_radian >= 0.25 * d.max(1.0))
            .map(|&(d, i)| (i, d <= close))
            .collect();
        self.visible = visible.into_iter().map(|(d, i)| (i, d <= close)).collect();

        let fog = &self.fog;
        let u = &mut self.uniforms;
        u.fill_sky[3] = if self.cloud_bound { 1.0 } else { 0.0 };
        u.fog_color = [fog.color[0], fog.color[1], fog.color[2], fog.power];
        u.fog = [fog.start, fog.end, fog.sky_blend_distance, 1.0];
        u.params = [
            self.settings.near_m,
            self.settings.range_m,
            1.0 - self.windows,
            0.0,
        ];
        ctx.uploads
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if self.visible.is_empty() {
            return;
        }
        let pipeline = match phase {
            Phase::Opaque => self.pipeline,
            Phase::Transparent => self.lines,
            _ => return,
        };
        pass.set_pipeline(ctx.pipelines.render(pipeline));
        pass.set_bind_group(1, &self.group, &[]);
        pass.set_bind_group(2, &self.environment, &[]);
        pass.set_bind_group(3, &self.cloud_group, &[]);
        if phase == Phase::Opaque {
            for (index, close) in &self.visible {
                if let Some(chunk) = self.chunks.get(index) {
                    self.draw_ranges(pass, chunk, &chunk.drawn(*close));
                }
            }
        } else {
            // The lines farthest first, each blended over those behind it.
            for (index, close) in self.visible.iter().rev() {
                if let Some(chunk) = self.chunks.get(index) {
                    self.draw_ranges(pass, chunk, &chunk.drawn_lines(*close));
                }
            }
        }
    }

    /// The lines' response to the history where they move against what lies behind them (`vs_infra_line_motion`),
    /// for the chunks near enough to the travelling eye (`moving`). The faces stand still and write nothing:
    /// the resolve follows them by their depth.
    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if self.moving.is_empty() {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(self.line_motion));
        for (index, close) in &self.moving {
            if let Some(chunk) = self.chunks.get(index) {
                self.draw_ranges(pass, chunk, &chunk.drawn_lines(*close));
            }
        }
    }

    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.settings.cast_shadows
            || !matches!(
                map.kind,
                ShadowMapKind::Near | ShadowMapKind::Aboard | ShadowMapKind::Wide
            )
            || self.chunks.is_empty()
        {
            return;
        }
        let planes = frustum_planes(map.projection * map.view);
        let mut first = true;
        for chunk in self.chunks.values() {
            let (lo, hi) = self.drawn_bounds(chunk);
            let close = self
                .eye
                .is_some_and(|eye| eye.clamp(lo, hi).distance(eye) <= self.settings.close_m);
            let cast = chunk.cast(close);
            if chunk.buffers.is_none() || cast.is_empty() {
                continue;
            }
            if !in_planes(&planes[..4], lo, hi) {
                continue;
            }
            if first {
                pass.set_pipeline(ctx.pipelines.render(self.caster));
                first = false;
            }
            self.draw_ranges(pass, chunk, &[cast]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn near_detail_keeps_a_margin_and_medium_has_none() {
        let s = InfraSettings::HIGH;
        assert_eq!(s.lod_at(1000.0, None), Some(Lod::Full));
        assert_eq!(s.lod_at(1600.0, None), Some(Lod::Box));
        assert_eq!(s.lod_at(1600.0, Some(Lod::Full)), Some(Lod::Full));
        assert_eq!(s.lod_at(8100.0, None), None);
        assert_eq!(s.lod_at(8100.0, Some(Lod::Box)), Some(Lod::Box));
        let m = InfraSettings::MEDIUM;
        assert_eq!(m.lod_at(0.0, None), Some(Lod::Box));
        assert_eq!(m.lod_at(4100.0, None), None);
    }

    #[test]
    fn a_near_chunk_draws_its_containers_one_by_one_only_when_close() {
        // Decal 0..6, Far 6..30, Structure 30..60, Detail 60..90, Near 90..300.
        let chunk = |lod| GpuChunk {
            lod,
            corner: [0.0; 2],
            buffers: None,
            ranges: [0, 6, 30, 60, 90, 300],
            lines: [300, 300, 320, 320, 340, 360],
            min: [0.0; 3],
            max: [0.0; 3],
            slot: 0,
            counts: InfraCounts::default(),
            runways: Vec::new(),
            bytes: 0,
            source: std::sync::Weak::new(),
        };
        let near = chunk(Lod::Full);
        assert_eq!(near.drawn(true), [0..6, 30..300]);
        assert_eq!(near.drawn(false), [0..90, 0..0]);
        assert_eq!((near.cast(true), near.cast(false)), (30..300, 6..90));
        let far = chunk(Lod::Box);
        assert_eq!(far.drawn(true), [0..60, 0..0]);
        assert_eq!(far.cast(false), 6..60);
        // The lines alike, from theirs: Far 300..320 (the towers afar), Detail 320..340 (the wires), Near 340..360.
        assert_eq!(near.drawn_lines(true), [300..300, 320..360]);
        assert_eq!(near.drawn_lines(false), [300..340, 0..0]);
        assert_eq!(far.drawn_lines(false), [300..320, 0..0]);
        assert_eq!(near.triangles(false), (90 + 40) / 3);
    }
}
