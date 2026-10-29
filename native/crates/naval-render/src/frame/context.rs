//! The registry a layer is built with ([`RenderContext`]): the device, the shader library, the pipeline and
//! resource registries and the camera's layouts, borrowed from the [`Renderer`].

use crate::aerial::AerialVolume;
use crate::copies::SceneCopies;
use crate::pipelines::{
    Pipeline, PipelineId, PipelineRecipe, PipelineRegistry, PlaceholderPipelines,
};
use crate::resources::Resources;
use crate::shaders::{ShaderDefs, ShaderError, ShaderFile, ShaderLibrary};
use crate::shadows::{CasterPipeline, caster_depth_state};
use crate::taa::{MotionPipeline, motion_depth_state, motion_target};

use super::layer::{
    OverlayEntry, OverlayPipeline, OverlayPipelineId, ScenePipeline, scene_depth_state,
};
use super::scene_pass::ensure_scene_copies;
use super::{HDR_FORMAT, Renderer};

/// What a layer is built with: the device, the shader library and the
/// pipeline registry, the resource registry and the camera's layout.
pub struct RenderContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub library: &'a mut ShaderLibrary,
    pub pipelines: &'a mut PipelineRegistry,
    pub resources: &'a mut Resources,
    /// Group 0 of every scene pipeline: the camera at binding 0 and the sun's
    /// shadow maps at bindings 1 to 4, their soft occluders at 8 and 9
    /// (`naval::shadow`). Also usable in compute pipelines.
    pub camera_layout: &'a wgpu::BindGroupLayout,
    /// Group 0 of shadow caster pipelines: a camera block alone, holding the
    /// map's shadow camera.
    pub caster_layout: &'a wgpu::BindGroupLayout,
    /// The output format overlay pipelines are first built for.
    pub output_format: wgpu::TextureFormat,
    /// The aerial-perspective volume group 0 binds (bindings 6 and 7, [`crate::aerial`]).
    pub aerial: &'a AerialVolume,
    overlays: &'a mut Vec<OverlayEntry>,
    placeholders: &'a std::sync::OnceLock<PlaceholderPipelines>,
}

impl RenderContext<'_> {
    /// Stand-ins for pipelines a layer registered and takes from the registry at its first prepare
    /// (see [`PlaceholderPipelines`]); made on first use, once per renderer.
    pub fn placeholders(&self) -> &PlaceholderPipelines {
        self.placeholders
            .get_or_init(|| PlaceholderPipelines::new(self.device))
    }

    /// Waits for every pipeline still compiling on a build thread (see
    /// [`PipelineRegistry::finish`]).
    pub fn finish_pipelines(&mut self) -> Result<(), ShaderError> {
        self.pipelines.finish()
    }

    /// Registers a WGSL file (a module to import or an entry shader).
    pub fn add_shader(&mut self, file: ShaderFile) -> Result<(), ShaderError> {
        self.library.add(file)
    }

    /// Builds a pipeline for the scene pass from the entry shader at `shader`.
    pub fn add_scene_pipeline(
        &mut self,
        label: &str,
        shader: &str,
        desc: ScenePipeline,
    ) -> Result<PipelineId, ShaderError> {
        let target = wgpu::ColorTargetState {
            format: HDR_FORMAT,
            blend: desc.blend,
            write_mask: wgpu::ColorWrites::ALL,
        };
        self.add_scene_pipeline_into(label, shader, desc, vec![Some(target)])
    }

    /// As [`RenderContext::add_scene_pipeline`], drawing into `targets` instead of the scene's color (`desc.blend` is
    /// then unused): for a layer's own pass with the scene's camera and depth, recorded at a copy point
    /// ([`crate::SceneLayer::after_copies`], binding [`crate::PrepareContext::camera_group`] at group 0).
    pub fn add_scene_pipeline_into(
        &mut self,
        label: &str,
        shader: &str,
        desc: ScenePipeline,
        targets: Vec<Option<wgpu::ColorTargetState>>,
    ) -> Result<PipelineId, ShaderError> {
        let camera_layout = self.camera_layout.clone();
        let defs = desc.defs.clone();
        // Opaque triangles may be drawn as lines by the wireframe debug view.
        let wireframe = desc.depth_write
            && desc.primitive.polygon_mode == wgpu::PolygonMode::Fill
            && matches!(
                desc.primitive.topology,
                wgpu::PrimitiveTopology::TriangleList | wgpu::PrimitiveTopology::TriangleStrip
            );
        let recipe: PipelineRecipe =
            Box::new(move |build| {
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
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some(desc.vertex_entry),
                            compilation_options: Default::default(),
                            buffers: &buffers,
                        },
                        primitive,
                        depth_stencil: Some(scene_depth_state(desc.depth_write)),
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some(desc.fragment_entry),
                            compilation_options: Default::default(),
                            targets: &targets,
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            });
        let id = self
            .pipelines
            .add(self.device, self.library, label, shader, defs, recipe)?;
        if wireframe {
            self.pipelines
                .allow_wireframe(self.device, self.library, id);
        }
        Ok(id)
    }

    /// Lets the wireframe debug view draw pipeline `id` as lines (see
    /// [`PipelineRegistry::allow_wireframe`]): for a scene pipeline of opaque triangles built with a recipe of its
    /// own, which puts [`crate::PipelineBuild::polygon_mode`] in its primitive state.
    /// [`RenderContext::add_scene_pipeline`] allows it by itself for opaque triangles.
    pub fn allow_wireframe(&mut self, id: PipelineId) {
        self.pipelines
            .allow_wireframe(self.device, self.library, id);
    }

    /// Builds a pipeline that draws into the sun's shadow maps from the entry
    /// shader at `shader` (see [`crate::shadows::CasterPipeline`]).
    pub fn add_caster_pipeline(
        &mut self,
        label: &str,
        shader: &str,
        desc: CasterPipeline,
    ) -> Result<PipelineId, ShaderError> {
        let caster_layout = self.caster_layout.clone();
        let defs = desc.defs.clone();
        let recipe: PipelineRecipe =
            Box::new(move |build| {
                let mut groups: Vec<Option<&wgpu::BindGroupLayout>> = vec![Some(&caster_layout)];
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
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some(desc.vertex_entry),
                            compilation_options: Default::default(),
                            buffers: &buffers,
                        },
                        primitive: desc.primitive,
                        depth_stencil: Some(caster_depth_state()),
                        multisample: wgpu::MultisampleState::default(),
                        fragment: desc.fragment_entry.map(|entry| wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            targets: &[],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            });
        self.pipelines
            .add(self.device, self.library, label, shader, defs, recipe)
    }

    /// Builds a pipeline for the motion pass from the entry shader at `shader`
    /// (see [`crate::taa::MotionPipeline`]): the camera at group 0, the motion
    /// target with its blend ([`crate::taa::motion_target`]), `scene.depth` read-only.
    pub fn add_motion_pipeline(
        &mut self,
        label: &str,
        shader: &str,
        desc: MotionPipeline,
    ) -> Result<PipelineId, ShaderError> {
        let camera_layout = self.camera_layout.clone();
        let defs = desc.defs.clone();
        let recipe: PipelineRecipe =
            Box::new(move |build| {
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
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some(desc.vertex_entry),
                            compilation_options: Default::default(),
                            buffers: &buffers,
                        },
                        primitive: desc.primitive,
                        depth_stencil: Some(motion_depth_state()),
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some(desc.fragment_entry),
                            compilation_options: Default::default(),
                            targets: &[Some(motion_target())],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            });
        self.pipelines
            .add(self.device, self.library, label, shader, defs, recipe)
    }

    /// Builds a pipeline for the overlay pass from the entry shader at `shader` (see
    /// [`OverlayPipeline`]), for the renderer's output format now and for any other format an
    /// output brings later.
    pub fn add_overlay_pipeline(
        &mut self,
        label: &str,
        shader: &str,
        desc: OverlayPipeline,
    ) -> Result<OverlayPipelineId, ShaderError> {
        let mut entry = OverlayEntry {
            label: label.to_owned(),
            shader: shader.to_owned(),
            desc,
            built: Vec::new(),
        };
        entry.ensure(
            self.device,
            self.library,
            self.pipelines,
            self.camera_layout,
            self.output_format,
        )?;
        self.overlays.push(entry);
        Ok(OverlayPipelineId(self.overlays.len() - 1))
    }

    /// Makes the textures for `copies` now, so a layer can build bind groups on
    /// them when it is created (they are made on first request anyway). They are
    /// viewport textures: check their `version` before reusing a bind group.
    pub fn request_scene_copies(&mut self, copies: SceneCopies) {
        ensure_scene_copies(self.device, self.resources, copies);
    }

    /// Builds any other pipeline (compute, shadow maps, offscreen targets) from
    /// the entry shader at `shader` with a recipe of the caller's own.
    pub fn add_pipeline(
        &mut self,
        label: &str,
        shader: &str,
        defs: ShaderDefs,
        recipe: PipelineRecipe,
    ) -> Result<PipelineId, ShaderError> {
        self.pipelines
            .add(self.device, self.library, label, shader, defs, recipe)
    }
}

impl Renderer {
    /// Borrows what a layer is built with.
    pub fn context(&mut self) -> RenderContext<'_> {
        RenderContext {
            device: &self.device,
            queue: &self.queue,
            library: &mut self.library,
            pipelines: &mut self.pipelines,
            resources: &mut self.resources,
            camera_layout: &self.camera.layout,
            caster_layout: &self.camera.caster_layout,
            output_format: self.output_format,
            aerial: &self.camera.aerial,
            overlays: &mut self.overlays,
            placeholders: &self.placeholders,
        }
    }
}
