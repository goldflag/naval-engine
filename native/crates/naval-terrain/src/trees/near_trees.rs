//! The trees near the camera drawn as meshes (native/docs/terrain/trees.md, "Near trees"): every species' mesh
//! ([`crate::tree_meshes`]) at its two levels of detail, instanced for the placed trees the binning
//! (`shaders/tree_near_bin.wgsl`) finds in view within their reach, one indirect draw a species and level
//! (`shaders/near_trees.wgsl`). The impostors give way to them by a dither over a band (`shaders/tree_wind.wgsl`).
//! High and Ultra only (the trees' setting); Medium draws no trees.

use bytemuck::{Pod, Zeroable};
use naval_render::{
    HDR_FORMAT, Pipeline, PipelineId, PipelineRecipe, PrepareContext, RenderContext, ScenePipeline,
    ShaderDefs, ShaderError, VertexLayout, compute_recipe, scene_depth_state,
};

use crate::species::models;
use crate::tree_meshes::{MESH_LODS, MESH_VERTEX_BYTES, tree_meshes};
use crate::trees::{TREE_CAPACITY, TREE_INSTANCE_BYTES};

/// The meshes' largest reach (m): a tree as tall as the tallest is a mesh to here, and its impostor's frames past it
/// are about a texel a pixel at 1080 lines (`impostors::FRAME_TEXELS`).
pub const NEAR_REACH_M: f32 = 250.0;
/// The reach a metre of a tree's height (m): a shrub's mesh gives way to its impostor as near as its impostor's
/// texels are as fine on screen as a tall tree's at its reach.
pub const NEAR_REACH_PER_M: f32 = 12.0;
/// The full detail's share of a tree's reach, and the share of each reach the crossfade at its end spans.
pub const FULL_SHARE: f32 = 0.2;
pub const FADE_SHARE: f32 = 0.2;
/// Trees each species' list holds at each level of detail: a tree with no room draws as its impostor.
pub const NEAR_ROOM: u32 = 4096;

/// The two pipelines a tree draw takes (native/docs/terrain/trees.md, "Near trees", "The depth first"): its depth
/// alone,
/// alpha-tested (`depth_entry`, writing no colour), then its light where that depth stands (`desc.fragment_entry`,
/// depth `Equal`, none written, no pixel the prepass stood discarded). Each pixel of the trees is lit once, where a
/// tiled GPU lit every alpha-tested card that passed the depth test as it came, before it knew what lay in front of it.
/// The light's shader keeps a discard no such pixel takes: on Apple's GPUs one that cannot discard, drawn under
/// `Equal`, took unsteady derivatives (`trees.wgsl`, `fs_tree`; "The light's discard").
pub(crate) fn tree_pipelines(
    ctx: &mut RenderContext<'_>,
    label: &str,
    shader: &str,
    desc: ScenePipeline,
    depth_entry: &'static str,
) -> Result<[PipelineId; 2], ShaderError> {
    let depth = ctx.add_scene_pipeline_into(
        &format!("{label} depth"),
        shader,
        ScenePipeline {
            fragment_entry: depth_entry,
            depth_write: true,
            ..desc.clone()
        },
        vec![Some(wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::empty(),
        })],
    )?;
    // As `add_scene_pipeline`'s, but for the depth test.
    let camera_layout = ctx.camera_layout.clone();
    let defs = desc.defs.clone();
    let recipe: PipelineRecipe = Box::new(move |build| {
        let mut groups: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(&camera_layout)];
        groups.extend(desc.bind_group_layouts.iter().map(Some));
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(build.label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            });
        let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = desc
            .vertex_buffers
            .iter()
            .map(|buffer| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: buffer.stride,
                    step_mode: buffer.step_mode,
                    attributes: &buffer.attributes,
                })
            })
            .collect();
        let mut primitive = desc.primitive;
        if build.polygon_mode != wgpu::PolygonMode::Fill {
            primitive.polygon_mode = build.polygon_mode;
        }
        Pipeline::Render(
            build
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: build.module,
                        entry_point: Some(desc.vertex_entry),
                        compilation_options: Default::default(),
                        buffers: &buffers,
                    },
                    primitive,
                    depth_stencil: Some(wgpu::DepthStencilState {
                        depth_write_enabled: Some(false),
                        depth_compare: Some(wgpu::CompareFunction::Equal),
                        ..scene_depth_state(false)
                    }),
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: build.module,
                        entry_point: Some(desc.fragment_entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: HDR_FORMAT,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: build.cache,
                }),
        )
    });
    let lit = ctx.add_pipeline(label, shader, defs, recipe)?;
    ctx.allow_wireframe(lit);
    Ok([depth, lit])
}

/// What the binning reads (`tree_near_bin.wgsl`'s `NearView`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct NearView {
    /// The view's frustum in chart metres (n · p + w ≥ 0 inside).
    pub planes: [[f32; 4]; 6],
    /// The eye in chart metres; w: the species.
    pub eye: [f32; 4],
    /// The largest reach (m), the reach a metre of height, the full detail's share, its band's share.
    pub reach: [f32; 4],
    /// Trees a list, the trees' capacity.
    pub room: [u32; 4],
}

/// The near trees' reach for this lens (m; the largest, and a metre of height): a narrower lens than the game's sees
/// its trees larger, so its meshes reach further, up to 1.6 times.
pub fn near_reach(tan_half_fov: f32, lens_tan: f32) -> [f32; 2] {
    let scale = 1.0 / (tan_half_fov / lens_tan).clamp(0.62, 1.0);
    [NEAR_REACH_M * scale, NEAR_REACH_PER_M * scale]
}

/// The near trees' GPU side: the meshes, the lists and their draws, the binning and the draw's pipelines.
pub struct NearTrees {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// The lists: species × level × [`NEAR_ROOM`] placed trees.
    lists: wgpu::Buffer,
    /// Each list's `DrawIndexedIndirectArgs` (20 bytes), its mesh's and its count.
    args: wgpu::Buffer,
    /// The args as each binning starts: every list empty.
    empty_args: Vec<u32>,
    view: wgpu::Buffer,
    bin: PipelineId,
    bin_group: wgpu::BindGroup,
    /// The depth prepass's pipeline and the light's (`tree_pipelines`).
    pipelines: [PipelineId; 2],
    species: u32,
}

impl NearTrees {
    /// Builds the meshes' buffers and the pipelines. `trees` and `tree_args` are the placement's instances and indirect
    /// draw; `layouts` the scene pipeline's groups 1-3 (the land's, the sky's environment, the cloud shadow).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        trees: &wgpu::Buffer,
        tree_args: &wgpu::Buffer,
        layouts: Vec<wgpu::BindGroupLayout>,
        defs: ShaderDefs,
    ) -> Result<Self, ShaderError> {
        let device = ctx.device;
        let queue = ctx.queue;
        let meshes = tree_meshes();
        let species = models().len() as u32;
        let init = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: contents.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, contents);
            buffer
        };
        let vertices = init(
            "near tree vertices",
            bytemuck::cast_slice(&meshes.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = init(
            "near tree indices",
            bytemuck::cast_slice(&meshes.indices),
            wgpu::BufferUsages::INDEX,
        );
        let mut empty_args = Vec::new();
        for ranges in &meshes.ranges {
            for range in ranges.iter().take(MESH_LODS) {
                empty_args.extend_from_slice(&[
                    range.index_count,
                    0,
                    range.first_index,
                    range.base_vertex as u32,
                    0,
                ]);
            }
        }
        let args = init(
            "near tree draws",
            bytemuck::cast_slice(&empty_args),
            wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC,
        );
        let lists = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("near tree lists"),
            size: u64::from(species)
                * MESH_LODS as u64
                * u64::from(NEAR_ROOM)
                * TREE_INSTANCE_BYTES,
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let view = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("near tree view"),
            size: std::mem::size_of::<NearView>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let storage = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("near tree binning"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, true),
                storage(2, false),
                storage(3, false),
                storage(4, false),
            ],
        });
        let bin_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("near tree binning"),
            layout: &bin_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tree_args.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: trees.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: lists.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: args.as_entire_binding(),
                },
            ],
        });
        let bin = ctx.add_pipeline(
            "terrain near tree binning",
            "naval-terrain/shaders/tree_near_bin.wgsl",
            Vec::new(),
            compute_recipe(vec![bin_layout], "bin"),
        )?;
        let attribute =
            |location: u32, format: wgpu::VertexFormat, offset: u64| wgpu::VertexAttribute {
                format,
                offset,
                shader_location: location,
            };
        use wgpu::VertexFormat as F;
        let buffers = vec![
            VertexLayout {
                stride: MESH_VERTEX_BYTES,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: vec![
                    attribute(0, F::Float32x3, 0),
                    attribute(1, F::Snorm8x4, 12),
                    attribute(2, F::Snorm8x4, 16),
                    attribute(3, F::Unorm16x2, 20),
                    attribute(4, F::Unorm8x4, 24),
                    attribute(5, F::Unorm8x4, 28),
                    attribute(9, F::Snorm8x4, 32),
                ],
            },
            VertexLayout {
                stride: TREE_INSTANCE_BYTES,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: vec![
                    attribute(6, F::Float32x4, 0),
                    attribute(7, F::Float32x4, 16),
                    attribute(8, F::Float32x4, 32),
                ],
            },
        ];
        let pipelines = tree_pipelines(
            ctx,
            "terrain near trees",
            "naval-terrain/shaders/near_trees.wgsl",
            ScenePipeline {
                vertex_buffers: buffers,
                bind_group_layouts: layouts,
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                defs,
                ..ScenePipeline::new("vs_near", "fs_near")
            },
            "fs_near_depth",
        )?;
        Ok(Self {
            vertices,
            indices,
            lists,
            args,
            empty_args,
            view,
            bin,
            bin_group,
            pipelines,
            species,
        })
    }

    /// Bins the placed trees for this view, after the placement (in its own pass, so it reads the placement's count).
    pub fn bin(&self, ctx: &mut PrepareContext<'_>, mut view: NearView) {
        view.eye[3] = self.species as f32;
        view.room = [NEAR_ROOM, TREE_CAPACITY, 0, 0];
        ctx.uploads
            .write_buffer(&self.view, 0, bytemuck::bytes_of(&view));
        ctx.uploads
            .write_buffer(&self.args, 0, bytemuck::cast_slice(&self.empty_args));
        let mut pass = ctx
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("terrain near tree binning"),
                timestamp_writes: ctx.timer.compute_pass("terrain near tree binning"),
            });
        pass.set_pipeline(ctx.pipelines.compute(self.bin));
        pass.set_bind_group(0, &self.bin_group, &[]);
        pass.dispatch_workgroups(TREE_CAPACITY.div_ceil(64), 1, 1);
    }

    /// Draws the lists, the full detail first (nearest, so the reduced detail behind it fails the depth test), each
    /// level where `levels` says so: their depth alone (`light` false), or their light where it stands (true, after
    /// every tree's depth). The caller has bound groups 1-3 as for the impostors.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &naval_render::PipelineRegistry,
        levels: [bool; MESH_LODS],
        light: bool,
    ) {
        pass.set_pipeline(pipelines.render(self.pipelines[usize::from(light)]));
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        let list_bytes = u64::from(NEAR_ROOM) * TREE_INSTANCE_BYTES;
        for lod in 0..MESH_LODS as u32 {
            if !levels[lod as usize] {
                continue;
            }
            for species in 0..self.species {
                let list = species * MESH_LODS as u32 + lod;
                let start = u64::from(list) * list_bytes;
                pass.set_vertex_buffer(1, self.lists.slice(start..start + list_bytes));
                pass.draw_indexed_indirect(&self.args, u64::from(list) * 20);
            }
        }
    }

    /// The lists' indirect draws, for tests.
    pub fn arguments(&self) -> &wgpu::Buffer {
        &self.args
    }

    /// The lists (each [`NEAR_ROOM`] placed trees), for tests.
    pub fn lists(&self) -> &wgpu::Buffer {
        &self.lists
    }

    /// GPU memory the near trees hold (bytes).
    pub fn bytes(&self) -> u64 {
        self.vertices.size() + self.indices.size() + self.lists.size() + self.args.size()
    }
}
