//! The ships as a renderer-core layer ([`VesselLayer`]): each model uploaded once ([`VesselLayer::add_model`]), each
//! frame the host's ships ([`VesselInstance`]: a model and a full transform) drawn as instances of it, one indexed draw
//! per material. Opaque and cut-out materials draw in the `Opaque` phase, cast into the sun's maps and the lamps' and
//! write their motion for temporal AA; blended ones draw in the `Transparent` phase, back to front ship by ship, and
//! cast nothing; the navigation lamps glow there after them. Instances are placed less the eye, in doubles until the
//! subtraction, so a ship kilometres from the world's origin draws without shaking. Each map is read through its own
//! texture's sampler; normals turn by the instance's inverse transpose; an instance that mirrors or flattens her is
//! refused ([`VesselInstance::check`]).

use std::collections::HashMap;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{DMat3, DMat4, DVec3, Vec3};
use naval_render::lights::FogUniform;
use naval_render::{
    CasterPipeline, DrawContext, MotionPipeline, Phase, PipelineId, PrepareContext, RenderContext,
    SceneLayer, ScenePipeline, SceneView, ShaderError, ShaderFile, ShadowMap, UploadSink,
    VertexLayout,
};
use naval_sky::EnvironmentMap;
use wgpu::util::DeviceExt;

use crate::images::TextureImage;
use crate::lighting::VesselLighting;
use crate::model::{Vertex, VesselModel};
use crate::read::{Alpha, Material, Sampling, Slot};
use crate::vessel::{Lamp, normal_turn};

const COMMON: ShaderFile = naval_render::shader_file!("shaders/common.wgsl");
const BSDF: ShaderFile = naval_render::shader_file!("shaders/bsdf.wgsl");
const VESSEL: ShaderFile = naval_render::shader_file!("shaders/vessel.wgsl");
const CASTER: ShaderFile = naval_render::shader_file!("shaders/vessel_caster.wgsl");
const LAMPS: ShaderFile = naval_render::shader_file!("shaders/lamps.wgsl");
const ENVIRONMENT: ShaderFile = naval_sky::shaders::ENVIRONMENT_FILE;

/// A lamp's glow's own size (m): a lantern's lens.
const LAMP_SIZE: f32 = 0.25;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniforms {
    sun_direction: [f32; 4],
    sun_radiance: [f32; 4],
    fill_sky: [f32; 4],
    fill_ground: [f32; 4],
    lighting: [f32; 4],
    fog: FogUniform,
    eye: [f32; 4],
}

/// One ship as the shaders read it (`Instance` in `shaders/common.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct InstanceRecord {
    model: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
    normal: [[f32; 4]; 3],
    flags: [f32; 4],
}

/// One lamp as the shaders read it (`Lamp` in `shaders/common.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct LampRecord {
    offset: [f32; 4],
    light: [f32; 4],
}

/// A material as the shaders read it (`Material` in `shaders/common.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
struct MaterialUniforms {
    base_color: [f32; 4],
    emissive: [f32; 4],
    params: [f32; 4],
    flags: [f32; 4],
    maps: [[[f32; 4]; 2]; 5],
}

impl MaterialUniforms {
    fn new(m: &Material) -> Self {
        let mut maps = [[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]]; 5];
        for slot in Slot::ALL {
            if let Some(map) = m.maps[slot as usize] {
                let [a, b] = map.transform;
                maps[slot as usize] = [[a[0], a[1], a[2], map.set as f32], [b[0], b[1], b[2], 1.0]];
            }
        }
        let cutoff = match m.alpha {
            Alpha::Mask(cutoff) => cutoff,
            _ => 0.0,
        };
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        Self {
            base_color: m.base_color,
            emissive: [m.emissive[0], m.emissive[1], m.emissive[2], 0.0],
            params: [
                m.metallic,
                m.roughness,
                m.normal_scale,
                m.occlusion_strength,
            ],
            flags: [cutoff, flag(m.unlit), flag(m.transmission > 0.0), 0.0],
            maps,
        }
    }
}

/// How a material draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Opaque = 0,
    Mask = 1,
    Blend = 2,
}

struct GpuDraw {
    indices: Range<u32>,
    kind: Kind,
    two_sided: bool,
    material: usize,
}

struct GpuModel {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    draws: Vec<GpuDraw>,
    materials: Vec<wgpu::BindGroup>,
    lamps: Vec<Lamp>,
    triangles: u64,
}

/// A model uploaded to a layer ([`VesselLayer::add_model`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ModelId(pub usize);

/// One ship to draw.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VesselInstance {
    pub model: ModelId,
    /// Ship space to the world (`VesselPose::transform`, or the host's own with its motion in it): a turn, a move and
    /// any scale, but no mirror ([`VesselInstance::check`]).
    pub transform: DMat4,
    /// The same for her from frame to frame: her motion's history is kept by it.
    pub key: u64,
    /// Her navigation lamps lit.
    pub lamps: bool,
}

impl VesselInstance {
    /// Whether the layer can draw her: a finite transform that neither flattens her (no normals' turn) nor mirrors
    /// her. A mirror (a negative determinant) would turn her faces inside out under the back faces' culling; it is
    /// refused rather than drawn with its own pipelines, as a ship's pose never mirrors her (mirror the model in its
    /// file instead, where loading rewinds its faces). A refused ship is not drawn ([`VesselStats::refused`]).
    pub fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.transform.is_finite(), "her transform is not finite");
        let det = DMat3::from_mat4(self.transform).determinant();
        anyhow::ensure!(
            det > 0.0 && normal_turn(&self.transform).is_some(),
            "her transform {} (determinant {det:.3e}): give her a turn, a move and a scale only",
            if det < 0.0 {
                "mirrors her"
            } else {
                "flattens her"
            }
        );
        Ok(())
    }
}

/// What the layer drew in its last frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
pub struct VesselStats {
    pub models: usize,
    pub ships: usize,
    pub triangles: u64,
    pub lamps: usize,
    /// Instances not drawn: an unknown model, or a transform [`VesselInstance::check`] refuses.
    pub refused: usize,
}

/// A storage buffer that grows to what it is asked to hold; `version` counts its replacements.
struct Storage {
    buffer: wgpu::Buffer,
    label: &'static str,
    version: u64,
}

impl Storage {
    fn new(device: &wgpu::Device, label: &'static str, size: u64) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            label,
            version: 0,
        }
    }

    fn write(&mut self, device: &wgpu::Device, uploads: &mut UploadSink<'_>, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if bytes.len() as u64 > self.buffer.size() {
            *self = Self {
                version: self.version + 1,
                ..Self::new(device, self.label, (bytes.len() as u64).next_power_of_two())
            };
        }
        uploads.write_buffer(&self.buffer, 0, bytes);
    }
}

/// The ships and their drawing.
pub struct VesselLayer {
    /// The scene's light: the host follows the sky with it every frame.
    pub lighting: VesselLighting,
    /// The ships to draw this frame, in any order.
    pub instances: Vec<VesselInstance>,
    /// Draw them (off for an A/B; nothing else changes).
    pub drawn: bool,
    /// Receive the sun's shadow maps, and cast into them and the lamps' maps.
    pub shadows: bool,
    /// Write object motion for temporal AA (on when the renderer's TAA is).
    pub motion: bool,
    models: Vec<GpuModel>,
    frame_buffer: wgpu::Buffer,
    instance_buffer: Storage,
    lamp_buffer: Storage,
    frame_layout: wgpu::BindGroupLayout,
    material_layout: wgpu::BindGroupLayout,
    frame_group: Option<(wgpu::BindGroup, u64, u64)>,
    environment: wgpu::BindGroup,
    dfg: wgpu::TextureView,
    lut_sampler: wgpu::Sampler,
    defaults: [wgpu::TextureView; 3],
    /// One a way of sampling, the maps' that read it.
    samplers: HashMap<Sampling, wgpu::Sampler>,
    /// By [`Kind`] and two-sidedness.
    scene_pipelines: [[PipelineId; 2]; 3],
    casters: [PipelineId; 2],
    /// The opaque and cut-out surfaces' (as the scene pass cuts them), by two-sidedness.
    motion_pipelines: [[PipelineId; 2]; 2],
    lamp_pipeline: PipelineId,
    records: Vec<InstanceRecord>,
    lamp_records: Vec<LampRecord>,
    /// This frame's instances by model: the model and its records' range.
    ranges: Vec<(usize, Range<u32>)>,
    /// The records that draw blended surfaces, farthest first, with their models.
    blended: Vec<(usize, u32)>,
    current: HashMap<u64, DMat4>,
    history: HashMap<u64, DMat4>,
    motion_frame: Option<u64>,
    second_view: bool,
    stats: VesselStats,
}

fn vertex_layout() -> Vec<VertexLayout> {
    let attribute = |format, offset, location| wgpu::VertexAttribute {
        format,
        offset,
        shader_location: location,
    };
    vec![VertexLayout {
        stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: vec![
            attribute(wgpu::VertexFormat::Float32x3, 0, 0),
            attribute(wgpu::VertexFormat::Float32x3, 12, 1),
            attribute(wgpu::VertexFormat::Float32x2, 24, 2),
            attribute(wgpu::VertexFormat::Float32x2, 32, 3),
            attribute(wgpu::VertexFormat::Unorm8x4, 40, 4),
        ],
    }]
}

fn entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    ty: wgpu::BindingType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

fn texture_type() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

fn buffer_type(ty: wgpu::BufferBindingType) -> wgpu::BindingType {
    wgpu::BindingType::Buffer {
        ty,
        has_dynamic_offset: false,
        min_binding_size: None,
    }
}

/// A texture of `levels` (RGBA8) on the GPU.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    name: &str,
    srgb: bool,
    levels: &[crate::images::Level],
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(&label(name)),
        size: wgpu::Extent3d {
            width: levels[0].width,
            height: levels[0].height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, map) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &map.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(map.width * 4),
                rows_per_image: Some(map.height),
            },
            wgpu::Extent3d {
                width: map.width,
                height: map.height,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

fn texel(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    name: &str,
    srgb: bool,
    rgba: [u8; 4],
) -> wgpu::TextureView {
    upload_texture(
        device,
        queue,
        name,
        srgb,
        &[crate::images::Level {
            width: 1,
            height: 1,
            pixels: rgba.to_vec(),
        }],
    )
}

/// A GPU label from a model's own names: ASCII, without what DX12 cannot take in one (`LABEL_FORBIDDEN`).
fn label(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii() && !naval_render::LABEL_FORBIDDEN.contains(&c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `transform` less `eye`, in doubles until the subtraction, as the shaders' columns.
fn columns(transform: &DMat4, eye: DVec3) -> [[f32; 4]; 4] {
    let mut m = *transform;
    m.w_axis -= eye.extend(0.0);
    m.as_mat4().to_cols_array_2d()
}

impl VesselLayer {
    /// Registers the shaders and builds the pipelines; no model yet.
    pub fn new(
        ctx: &mut RenderContext<'_>,
        environment: &EnvironmentMap,
        lighting: VesselLighting,
    ) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        for file in [ENVIRONMENT, BSDF, COMMON, VESSEL, CASTER, LAMPS] {
            if ctx.library.text(file.path).is_none() {
                ctx.add_shader(file)?;
            }
        }
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let storage = buffer_type(wgpu::BufferBindingType::Storage { read_only: true });
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vessel frame"),
            entries: &[
                entry(0, vf, buffer_type(wgpu::BufferBindingType::Uniform)),
                entry(1, fragment, texture_type()),
                entry(
                    2,
                    fragment,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                entry(3, wgpu::ShaderStages::VERTEX, storage),
                entry(4, wgpu::ShaderStages::VERTEX, storage),
            ],
        });
        let mut material_entries =
            vec![entry(0, vf, buffer_type(wgpu::BufferBindingType::Uniform))];
        for binding in 1..=5 {
            material_entries.push(entry(binding, fragment, texture_type()));
        }
        // Each map's own sampler.
        for binding in 6..=10 {
            material_entries.push(entry(
                binding,
                fragment,
                wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            ));
        }
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("vessel material"),
            entries: &material_entries,
        });

        let scene_groups = vec![
            frame_layout.clone(),
            material_layout.clone(),
            environment.bind_group_layout.clone(),
        ];
        let cull = |two_sided: bool| wgpu::PrimitiveState {
            cull_mode: if two_sided {
                None
            } else {
                Some(wgpu::Face::Back)
            },
            ..Default::default()
        };
        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let mut scene = |name: &str, fragment: &'static str, two_sided: bool, blend: bool| {
            ctx.add_scene_pipeline(
                name,
                VESSEL.path,
                ScenePipeline {
                    vertex_buffers: vertex_layout(),
                    bind_group_layouts: scene_groups.clone(),
                    primitive: cull(two_sided),
                    depth_write: !blend,
                    blend: blend.then_some(premultiplied),
                    ..ScenePipeline::new("vs_main", fragment)
                },
            )
        };
        let scene_pipelines = [
            [
                scene("vessel opaque", "fs_opaque", false, false)?,
                scene("vessel opaque two-sided", "fs_opaque", true, false)?,
            ],
            [
                scene("vessel cut-out", "fs_mask", false, false)?,
                scene("vessel cut-out two-sided", "fs_mask", true, false)?,
            ],
            [
                scene("vessel blended", "fs_blend", false, true)?,
                scene("vessel blended two-sided", "fs_blend", true, true)?,
            ],
        ];
        let caster = |ctx: &mut RenderContext<'_>, name: &str, fragment: Option<&'static str>| {
            ctx.add_caster_pipeline(
                name,
                CASTER.path,
                CasterPipeline {
                    vertex_buffers: vertex_layout(),
                    bind_group_layouts: vec![frame_layout.clone(), material_layout.clone()],
                    fragment_entry: fragment,
                    ..CasterPipeline::new("vs_caster")
                },
            )
        };
        let casters = [
            caster(ctx, "vessel casters", None)?,
            caster(ctx, "vessel cut-out casters", Some("fs_caster_mask"))?,
        ];
        // Motion covers what the scene pass drew: a cut-out's holes and a one-sided surface's back write none.
        let mut motion = |name: &str, fragment: &'static str, two_sided: bool| {
            ctx.add_motion_pipeline(
                name,
                VESSEL.path,
                MotionPipeline {
                    vertex_buffers: vertex_layout(),
                    bind_group_layouts: vec![frame_layout.clone(), material_layout.clone()],
                    primitive: cull(two_sided),
                    ..MotionPipeline::new("vs_motion", fragment)
                },
            )
        };
        let motion_pipelines = [
            [
                motion("vessel motion", "fs_motion", false)?,
                motion("vessel motion two-sided", "fs_motion", true)?,
            ],
            [
                motion("vessel cut-out motion", "fs_motion_mask", false)?,
                motion("vessel cut-out motion two-sided", "fs_motion_mask", true)?,
            ],
        ];
        let additive = wgpu::BlendState {
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
        };
        let lamp_pipeline = ctx.add_scene_pipeline(
            "vessel lamps",
            LAMPS.path,
            ScenePipeline {
                bind_group_layouts: vec![frame_layout.clone()],
                primitive: cull(true),
                depth_write: false,
                blend: Some(additive),
                ..ScenePipeline::new("vs_lamp", "fs_lamp")
            },
        )?;

        let dfg_texture = device.create_texture_with_data(
            &queue,
            &wgpu::TextureDescriptor {
                label: Some("vessel DFG table"),
                size: wgpu::Extent3d {
                    width: crate::dfg::DFG_SIZE,
                    height: crate::dfg::DFG_SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rg16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            bytemuck::cast_slice(&crate::dfg::DFG_LUT),
        );
        let lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("vessel DFG"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let defaults = [
            texel(&device, &queue, "vessel white sRGB", true, [255; 4]),
            texel(&device, &queue, "vessel white", false, [255; 4]),
            texel(
                &device,
                &queue,
                "vessel flat normal",
                false,
                [128, 128, 255, 255],
            ),
        ];
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vessel frame uniforms"),
            size: std::mem::size_of::<FrameUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            lighting,
            instances: Vec::new(),
            drawn: true,
            shadows: true,
            motion: false,
            models: Vec::new(),
            frame_buffer,
            instance_buffer: Storage::new(
                &device,
                "vessel instances",
                16 * std::mem::size_of::<InstanceRecord>() as u64,
            ),
            lamp_buffer: Storage::new(
                &device,
                "vessel lamps",
                64 * std::mem::size_of::<LampRecord>() as u64,
            ),
            frame_layout,
            material_layout,
            frame_group: None,
            environment: environment.bind_group.clone(),
            dfg: dfg_texture.create_view(&wgpu::TextureViewDescriptor::default()),
            lut_sampler,
            defaults,
            samplers: HashMap::new(),
            scene_pipelines,
            casters,
            motion_pipelines,
            lamp_pipeline,
            records: Vec::new(),
            lamp_records: Vec::new(),
            ranges: Vec::new(),
            blended: Vec::new(),
            current: HashMap::new(),
            history: HashMap::new(),
            motion_frame: None,
            second_view: false,
            stats: VesselStats::default(),
        })
    }

    /// Uploads `model`: its vertices, indices, textures and materials. Instances name it by the id returned.
    pub fn add_model(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        model: &VesselModel,
    ) -> ModelId {
        let name = &model.vessel.name;
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&label(&format!("vessel {name} vertices"))),
            contents: bytemuck::cast_slice(&model.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&label(&format!("vessel {name} indices"))),
            contents: bytemuck::cast_slice(&model.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let views: Vec<wgpu::TextureView> = model
            .textures
            .iter()
            .map(|t: &TextureImage| {
                upload_texture(
                    device,
                    queue,
                    &format!("vessel {name} {}", t.name),
                    t.srgb,
                    &t.levels,
                )
            })
            .collect();
        let materials = model
            .materials
            .iter()
            .map(|m| self.material_group(device, name, m, &views))
            .collect();
        let draws = model
            .draws
            .iter()
            .map(|d| {
                let m = &model.materials[d.material];
                GpuDraw {
                    indices: d.indices.clone(),
                    kind: match m.alpha {
                        Alpha::Opaque => Kind::Opaque,
                        Alpha::Mask(_) => Kind::Mask,
                        Alpha::Blend => Kind::Blend,
                    },
                    two_sided: m.double_sided,
                    material: d.material,
                }
            })
            .collect();
        self.models.push(GpuModel {
            vertices,
            indices,
            draws,
            materials,
            lamps: model.vessel.lamps.clone(),
            triangles: model.triangles() as u64,
        });
        ModelId(self.models.len() - 1)
    }

    fn material_group(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        material: &Material,
        views: &[wgpu::TextureView],
    ) -> wgpu::BindGroup {
        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("vessel material"),
            contents: bytemuck::bytes_of(&MaterialUniforms::new(material)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        // Each map its own texture's sampler (a missing map glTF's default), one a way of sampling across the layer.
        let sampling = Slot::ALL.map(|slot| {
            material.maps[slot as usize].map_or_else(Sampling::default, |m| m.sampling)
        });
        for s in sampling {
            self.samplers
                .entry(s)
                .or_insert_with(|| device.create_sampler(&s.descriptor()));
        }
        let view = |slot: Slot| -> &wgpu::TextureView {
            match material.maps[slot as usize] {
                Some(map) => &views[map.texture],
                None if slot == Slot::Normal => &self.defaults[2],
                None if slot.srgb() => &self.defaults[0],
                None => &self.defaults[1],
            }
        };
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }];
        for (k, slot) in Slot::ALL.into_iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: k as u32 + 1,
                resource: wgpu::BindingResource::TextureView(view(slot)),
            });
        }
        for (k, s) in sampling.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry {
                binding: k as u32 + 6,
                resource: wgpu::BindingResource::Sampler(&self.samplers[s]),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label(&format!("vessel {name} {}", material.name))),
            layout: &self.material_layout,
            entries: &entries,
        })
    }

    /// Rebinds the sky's environment after it rebuilds its atlas.
    pub fn set_environment(&mut self, environment: &EnvironmentMap) {
        self.environment = environment.bind_group.clone();
    }

    pub fn stats(&self) -> VesselStats {
        self.stats
    }

    fn frame_uniforms(&self, eye: DVec3) -> FrameUniforms {
        let l = &self.lighting;
        let radiance = l.light.color * l.light.intensity;
        FrameUniforms {
            sun_direction: l
                .light
                .direction
                .normalize_or(Vec3::Y)
                .extend(0.0)
                .to_array(),
            sun_radiance: radiance.extend(0.0).to_array(),
            fill_sky: (l.fill_sky * l.fill_intensity).extend(0.0).to_array(),
            fill_ground: (l.fill_ground * l.fill_intensity).extend(0.0).to_array(),
            lighting: [
                l.environment_intensity,
                if self.shadows { 1.0 } else { 0.0 },
                l.emissive_luminance * l.candela,
                0.0,
            ],
            fog: l.fog.uniform(),
            eye: eye.as_vec3().extend(0.0).to_array(),
        }
    }

    /// This frame's records from the host's instances: by model, each less the eye, with the pose of the frame before
    /// and her lamps as the eye sees them.
    fn gather(&mut self, eye: DVec3) {
        self.records.clear();
        self.lamp_records.clear();
        self.ranges.clear();
        self.blended.clear();
        self.current.clear();
        let mut order: Vec<&VesselInstance> = self
            .instances
            .iter()
            .filter(|i| i.model.0 < self.models.len() && i.check().is_ok())
            .collect();
        let refused = self.instances.len() - order.len();
        order.sort_by_key(|i| i.model.0);
        let mut triangles = 0;
        let mut distances = Vec::new();
        for instance in order {
            let model = &self.models[instance.model.0];
            let index = self.records.len() as u32;
            match self.ranges.last_mut() {
                Some((m, range)) if *m == instance.model.0 => range.end = index + 1,
                _ => self.ranges.push((instance.model.0, index..index + 1)),
            }
            let previous = self.history.get(&instance.key);
            // Checked: invertible.
            let normal = normal_turn(&instance.transform)
                .unwrap_or(DMat3::IDENTITY)
                .as_mat3();
            self.records.push(InstanceRecord {
                model: columns(&instance.transform, eye),
                previous: columns(previous.unwrap_or(&instance.transform), eye),
                normal: [normal.x_axis, normal.y_axis, normal.z_axis]
                    .map(|c| c.extend(0.0).to_array()),
                flags: [if previous.is_some() { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
            });
            self.current.insert(instance.key, instance.transform);
            triangles += model.triangles;
            if model.draws.iter().any(|d| d.kind == Kind::Blend) {
                let at = instance.transform.w_axis.truncate();
                distances.push((at.distance_squared(eye), instance.model.0, index));
            }
            if instance.lamps {
                // The eye in ship space, whatever her scale: the bearing her lamps' arcs take.
                let inverse = DMat3::from_mat4(instance.transform).inverse();
                for lamp in &model.lamps {
                    let world = instance
                        .transform
                        .transform_point3(lamp.position.as_dvec3());
                    let seen = inverse * (eye - world);
                    let bearing = seen.x.atan2(-seen.z) as f32;
                    let light =
                        lamp.color * lamp.intensity * self.lighting.candela * lamp.arc(bearing);
                    if light.max_element() <= 0.0 {
                        continue;
                    }
                    self.lamp_records.push(LampRecord {
                        offset: (world - eye).as_vec3().extend(LAMP_SIZE).to_array(),
                        light: light.extend(0.0).to_array(),
                    });
                }
            }
        }
        distances.sort_by(|a, b| b.0.total_cmp(&a.0));
        self.blended = distances.into_iter().map(|(_, m, i)| (m, i)).collect();
        self.stats = VesselStats {
            models: self.models.len(),
            ships: self.records.len(),
            triangles,
            lamps: self.lamp_records.len(),
            refused,
        };
    }

    fn bind_model<'p>(&self, pass: &mut wgpu::RenderPass<'p>, model: &GpuModel) {
        pass.set_vertex_buffer(0, model.vertices.slice(..));
        pass.set_index_buffer(model.indices.slice(..), wgpu::IndexFormat::Uint32);
    }

    /// The casters of every ship (opaque and cut-out surfaces) into a shadow map's pass.
    fn draw_casters(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let Some((frame, ..)) = &self.frame_group else {
            return;
        };
        pass.set_bind_group(1, frame, &[]);
        for (m, range) in &self.ranges {
            let model = &self.models[*m];
            self.bind_model(pass, model);
            for draw in model.draws.iter().filter(|d| d.kind != Kind::Blend) {
                pass.set_pipeline(
                    ctx.pipelines
                        .render(self.casters[(draw.kind == Kind::Mask) as usize]),
                );
                pass.set_bind_group(2, &model.materials[draw.material], &[]);
                pass.draw_indexed(draw.indices.clone(), 0, range.clone());
            }
        }
    }

    fn drawing(&self) -> bool {
        self.drawn && !self.second_view && !self.records.is_empty() && self.frame_group.is_some()
    }
}

impl SceneLayer for VesselLayer {
    fn label(&self) -> &str {
        "vessels"
    }

    fn set_view(&mut self, view: SceneView) {
        // One camera's records: the second view draws no ships.
        self.second_view = view == SceneView::Second;
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        if self.second_view {
            return;
        }
        // The motion frame moved on: this frame's ships take their history from the last one's.
        if self.motion_frame != Some(ctx.motion_frame) {
            std::mem::swap(&mut self.history, &mut self.current);
            self.motion_frame = Some(ctx.motion_frame);
        }
        if !self.drawn {
            self.records.clear();
            self.current.clear();
            return;
        }
        self.gather(ctx.eye);
        if self.records.is_empty() {
            return;
        }
        let device = ctx.device;
        let mut uploads = UploadSink::Belt(ctx.uploads);
        self.instance_buffer
            .write(device, &mut uploads, bytemuck::cast_slice(&self.records));
        self.lamp_buffer.write(
            device,
            &mut uploads,
            bytemuck::cast_slice(&self.lamp_records),
        );
        let uniforms = self.frame_uniforms(ctx.eye);
        uploads.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&uniforms));
        let versions = (self.instance_buffer.version, self.lamp_buffer.version);
        if self
            .frame_group
            .as_ref()
            .is_none_or(|(_, i, l)| (*i, *l) != versions)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("vessel frame"),
                layout: &self.frame_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.frame_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.dfg),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&self.lut_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: self.instance_buffer.buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.lamp_buffer.buffer.as_entire_binding(),
                    },
                ],
            });
            self.frame_group = Some((group, versions.0, versions.1));
        }
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.drawing() || !matches!(phase, Phase::Opaque | Phase::Transparent) {
            return;
        }
        let Some((frame, ..)) = &self.frame_group else {
            return;
        };
        pass.set_bind_group(1, frame, &[]);
        pass.set_bind_group(3, &self.environment, &[]);
        let pipeline = |draw: &GpuDraw| {
            ctx.pipelines
                .render(self.scene_pipelines[draw.kind as usize][draw.two_sided as usize])
        };
        if phase == Phase::Opaque {
            for (m, range) in &self.ranges {
                let model = &self.models[*m];
                self.bind_model(pass, model);
                for draw in model.draws.iter().filter(|d| d.kind != Kind::Blend) {
                    pass.set_pipeline(pipeline(draw));
                    pass.set_bind_group(2, &model.materials[draw.material], &[]);
                    pass.draw_indexed(draw.indices.clone(), 0, range.clone());
                }
            }
            return;
        }
        for &(m, index) in &self.blended {
            let model = &self.models[m];
            self.bind_model(pass, model);
            for draw in model.draws.iter().filter(|d| d.kind == Kind::Blend) {
                pass.set_pipeline(pipeline(draw));
                pass.set_bind_group(2, &model.materials[draw.material], &[]);
                pass.draw_indexed(draw.indices.clone(), 0, index..index + 1);
            }
        }
        if !self.lamp_records.is_empty() {
            pass.set_pipeline(ctx.pipelines.render(self.lamp_pipeline));
            pass.draw(0..6, 0..self.lamp_records.len() as u32);
        }
    }

    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        // The land maps are the land's alone.
        if !self.shadows || !self.drawing() || map.kind.is_land() {
            return;
        }
        self.draw_casters(pass, ctx);
    }

    fn draw_light_shadow(
        &self,
        _map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        if !self.shadows || !self.drawing() {
            return;
        }
        self.draw_casters(pass, ctx);
    }

    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.motion || !self.drawing() {
            return;
        }
        let Some((frame, ..)) = &self.frame_group else {
            return;
        };
        pass.set_bind_group(1, frame, &[]);
        for (m, range) in &self.ranges {
            let model = &self.models[*m];
            self.bind_model(pass, model);
            for draw in model.draws.iter().filter(|d| d.kind != Kind::Blend) {
                let pipeline = self.motion_pipelines[(draw.kind == Kind::Mask) as usize]
                    [draw.two_sided as usize];
                pass.set_pipeline(ctx.pipelines.render(pipeline));
                pass.set_bind_group(2, &model.materials[draw.material], &[]);
                pass.draw_indexed(draw.indices.clone(), 0, range.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_records_are_the_shaders_size() {
        // `Instance`: two mat4x4, a mat3x3 (three padded columns) and a vec4.
        assert_eq!(std::mem::size_of::<InstanceRecord>(), 12 * 16);
        assert_eq!(std::mem::size_of::<LampRecord>(), 2 * 16);
        // `Material`: four vec4s and five maps of two.
        assert_eq!(std::mem::size_of::<MaterialUniforms>(), 14 * 16);
        assert_eq!(std::mem::size_of::<Vertex>(), 44);
        // `Frame`: five vec4s, the fog's two and the eye.
        assert_eq!(std::mem::size_of::<FrameUniforms>(), 8 * 16);
    }

    #[test]
    fn instances_are_placed_less_the_eye_in_doubles() {
        let far = DMat4::from_translation(DVec3::new(40_000.123_456, 0.0, -25_000.5));
        let eye = DVec3::new(40_000.0, 3.0, -25_000.0);
        let m = columns(&far, eye);
        assert!((m[3][0] - 0.123_456).abs() < 1e-6, "{m:?}");
        assert!((m[3][1] + 3.0).abs() < 1e-6 && (m[3][2] + 0.5).abs() < 1e-6);
    }

    #[test]
    fn normals_follow_a_stretch_and_a_mirror_is_refused() {
        // Stretched twice her beam, a face sloped 45° out to starboard leans less: its normal (1, 1)/√2 goes to (½, 1).
        let wide = DMat4::from_scale(DVec3::new(2.0, 1.0, 1.0));
        let n = (normal_turn(&wide).unwrap() * DVec3::new(1.0, 1.0, 0.0)).normalize();
        assert!(
            n.abs_diff_eq(DVec3::new(0.5, 1.0, 0.0).normalize(), 1e-9),
            "{n}"
        );
        // A turn and an even scale turn normals as the turn alone.
        let turned = DMat4::from_scale_rotation_translation(
            DVec3::splat(3.0),
            glam::DQuat::from_rotation_y(0.7),
            DVec3::new(10.0, 0.0, 5.0),
        );
        let n = normal_turn(&turned).unwrap() * DVec3::X;
        assert!(n.abs_diff_eq(glam::DQuat::from_rotation_y(0.7) * DVec3::X, 1e-9));
        let ship = |transform| VesselInstance {
            model: ModelId(0),
            transform,
            key: 0,
            lamps: false,
        };
        assert!(ship(turned).check().is_ok() && ship(wide).check().is_ok());
        let mirrored = ship(DMat4::from_scale(DVec3::new(-1.0, 1.0, 1.0)));
        let reason = mirrored.check().unwrap_err().to_string();
        assert!(reason.contains("mirrors her"), "{reason}");
        assert!(
            ship(DMat4::from_scale(DVec3::new(1.0, 0.0, 1.0)))
                .check()
                .is_err()
        );
    }
}
