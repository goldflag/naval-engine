//! What a layer is to the frame: the [`SceneLayer`] trait every drawing crate implements, the phases of the scene
//! pass it draws in, the contexts its calls get, and the pipeline descriptions it registers through a
//! [`super::RenderContext`].

use crate::aerial::AerialVolume;
use crate::camera::{CameraUniforms, FrameTime};
use crate::copies::{CopyPoint, SceneCopies};
use crate::pipelines::{Pipeline, PipelineId, PipelineRecipe, PipelineRegistry};
use crate::resources::Resources;
use crate::shaders::{ShaderDefs, ShaderError, ShaderLibrary};
use crate::shadows::ShadowMap;
use crate::timing::GpuTimer;
use crate::upload::Uploads;

use super::{DEPTH_COMPARE, DEPTH_FORMAT};

/// When in the scene pass a layer's draws run. Every layer's `Opaque` draws
/// run before any `Water` draw, and so on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    /// Depth-writing geometry: hulls, terrain, islands.
    Opaque,
    /// Depth-writing surfaces that see the opaque scene through themselves: the
    /// sea. Drawn after the opaque copies ([`CopyPoint::Opaque`]), which it may
    /// sample, and before the sky, so the sky fills only what neither covered.
    /// The removed web game drew its sea first in the transparent queue for the same reason.
    Water,
    /// Backgrounds at the far plane (depth 0), drawn where opaque geometry and
    /// water left the clear: `GreaterEqual` against 0 passes only there.
    Sky,
    /// Blended geometry tested against, but not writing, depth: smoke, spray,
    /// tracers. Drawn after the backdrop copies ([`CopyPoint::Backdrop`]).
    Transparent,
}

impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Opaque, Phase::Water, Phase::Sky, Phase::Transparent];

    pub fn name(self) -> &'static str {
        match self {
            Phase::Opaque => "opaque",
            Phase::Water => "water",
            Phase::Sky => "sky",
            Phase::Transparent => "transparent",
        }
    }

    /// The copy point that follows this phase, if any.
    pub fn copy_point_after(self) -> Option<CopyPoint> {
        match self {
            Phase::Opaque => Some(CopyPoint::Opaque),
            Phase::Sky => Some(CopyPoint::Backdrop),
            Phase::Water | Phase::Transparent => None,
        }
    }
}

/// What a layer's `prepare` gets.
pub struct PrepareContext<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    /// The frame's encoder, ahead of the scene pass: a layer may record its own
    /// compute or render passes here.
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub pipelines: &'a PipelineRegistry,
    /// Mutable so a layer can make [`crate::resources::Lifetime::Frame`] resources.
    pub resources: &'a mut Resources,
    pub camera: &'a CameraUniforms,
    /// The eye at full precision ([`crate::Renderer::set_eye`]; `camera.position` rounds it). A layer that draws close
    /// bodies kilometres from the world's origin keeps their positions less this eye, in doubles until the subtraction,
    /// and draws them with `naval::camera::eye_to_clip` and `naval::motion::surface_motion_from_eye`.
    pub eye: glam::DVec3,
    /// Group 0 of the scene pass (the camera, the sun's shadow maps, the aerial volume), for a layer's own pass drawn
    /// with pipelines from [`crate::RenderContext::add_scene_pipeline_into`]. Before the scene pass it holds this
    /// frame's camera, though the aerial volume is written only by the sky's `prepare`: read it at a copy point.
    pub camera_group: &'a wgpu::BindGroup,
    pub time: FrameTime,
    /// The frame temporal AA's motion pass moves to from the one before: `time.frame`, but a held frame repeating the
    /// last (a pause under a still camera) resolves the last's again against its history, so it keeps the last's. A
    /// layer that keeps poses for its `draw_motion` advances them when this changes, not with `time.frame`.
    pub motion_frame: u64,
    pub viewport: (u32, u32),
    /// Timestamps for a layer's own passes: put `timer.compute_pass("ocean field")`
    /// (or `render_pass`) in the pass descriptor's `timestamp_writes`, and the pass
    /// shows in the frame's GPU timings.
    pub timer: &'a GpuTimer,
    /// Buffer uploads through the renderer's staging belt: `queue.write_buffer`'s
    /// ordering (in place before the frame's passes) without a fresh staging buffer
    /// per call, so a frame's large uploads touch no fresh pages ([`crate::upload`]).
    pub uploads: &'a mut Uploads,
    /// The sun's shadow maps as fitted for this frame (near, wide, view 1, view 2: `on` where sampled, `drawn` where
    /// their casters are drawn after `prepare`), so a layer can place casters for them (the land's trees).
    pub shadow_maps: &'a [ShadowMap],
    /// The point lights' maps this frame (`ShadowMapKind::Light`, [`crate::lights::light_shadow_maps`]), each drawn after
    /// `prepare` through [`SceneLayer::draw_light_shadow`]: a layer that casts into them culls its casters against these.
    pub light_maps: &'a [ShadowMap],
    /// The aerial-perspective volume every material reads at group 0 ([`crate::aerial`]): the sky's layer writes
    /// it in its `prepare`, for this frame's camera.
    pub aerial: &'a AerialVolume,
    /// Soft occluders in the sun's light (smoke): zeroed before the layers prepare, written by a layer that draws
    /// them ([`crate::shadows::SunOccluders`]).
    pub sun_occluders: &'a crate::shadows::SunOccluders,
}

/// What a layer's `draw` gets. The camera is already bound at group 0.
pub struct DrawContext<'a> {
    pub pipelines: &'a PipelineRegistry,
    pub resources: &'a Resources,
    pub camera: &'a CameraUniforms,
    pub time: FrameTime,
    pub viewport: (u32, u32),
}

/// What a layer's `draw_overlay` gets. Group 0 is bound as in the scene pass (the camera, and
/// `naval::motion::camera_motion` beside it: project with its `unjittered_view_projection`, since
/// the camera block carries temporal AA's jitter).
pub struct OverlayContext<'a> {
    pub pipelines: &'a PipelineRegistry,
    pub resources: &'a Resources,
    pub camera: &'a CameraUniforms,
    pub time: FrameTime,
    /// The output's size: overlays draw at full resolution whatever the render scale.
    pub viewport: (u32, u32),
    /// The scene's size (the output's times the render scale).
    pub scene_viewport: (u32, u32),
    /// The output's format.
    pub format: wgpu::TextureFormat,
    pub(super) overlays: &'a [OverlayEntry],
}

impl OverlayContext<'_> {
    /// The overlay pipeline `id` built for this frame's output format.
    pub fn pipeline(&self, id: OverlayPipelineId) -> &wgpu::RenderPipeline {
        let built = self.overlays[id.0]
            .built
            .iter()
            .find(|(format, _)| *format == self.format)
            .expect("built for the output's format before the overlay pass")
            .1;
        self.pipelines.render(built)
    }
}

/// A pipeline for the overlay pass, built with [`crate::RenderContext::add_overlay_pipeline`]: the
/// camera at group 0 and the layer's groups after it, no depth, onto the output. It is built for
/// each output format it meets; for a format that does not encode sRGB in hardware it gets the
/// `ENCODE_SRGB` define (apply `naval::color::srgb_oetf` to the display-space colour there).
#[derive(Clone, Debug)]
pub struct OverlayPipeline {
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub vertex_buffers: Vec<VertexLayout>,
    /// Groups 1, 2, ...
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    pub primitive: wgpu::PrimitiveState,
    /// Premultiplied alpha over the frame by default.
    pub blend: Option<wgpu::BlendState>,
    pub defs: ShaderDefs,
}

impl OverlayPipeline {
    /// A triangle list with no vertex buffers, blended premultiplied over the frame.
    pub fn new(vertex_entry: &'static str, fragment_entry: &'static str) -> Self {
        Self {
            vertex_entry,
            fragment_entry,
            vertex_buffers: Vec::new(),
            bind_group_layouts: Vec::new(),
            primitive: wgpu::PrimitiveState::default(),
            blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            defs: Vec::new(),
        }
    }
}

/// An overlay pipeline registered with [`crate::RenderContext::add_overlay_pipeline`]; look it up with
/// [`OverlayContext::pipeline`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OverlayPipelineId(pub(super) usize);

pub(crate) struct OverlayEntry {
    pub(super) label: String,
    pub(super) shader: String,
    pub(super) desc: OverlayPipeline,
    pub(super) built: Vec<(wgpu::TextureFormat, PipelineId)>,
}

impl OverlayEntry {
    pub(super) fn ensure(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        camera_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> Result<PipelineId, ShaderError> {
        if let Some((_, id)) = self.built.iter().find(|(built, _)| *built == format) {
            return Ok(*id);
        }
        let mut defs = self.desc.defs.clone();
        if !format.is_srgb() {
            defs.push(("ENCODE_SRGB".to_owned(), crate::ShaderDefValue::Bool(true)));
        }
        let desc = self.desc.clone();
        let camera_layout = camera_layout.clone();
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
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some(desc.fragment_entry),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: desc.blend,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            });
        let id = pipelines.add(
            device,
            library,
            format!("{} to {format:?}", self.label),
            &self.shader,
            defs,
            recipe,
        )?;
        self.built.push((format, id));
        Ok(id)
    }
}

/// Which view of the scene a layer prepares and draws for: the main one, or a second view drawn after it in the same
/// frame ([`crate::Renderer::render_view`]), whose camera stands elsewhere and whose targets are its own size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SceneView {
    #[default]
    Main,
    Second,
}

/// Something that draws into the HDR scene with depth.
///
/// Rules for a layer:
/// - Build pipelines with [`crate::RenderContext::add_scene_pipeline`] (or match
///   [`scene_depth_state`], [`crate::HDR_FORMAT`] and the camera at group 0 in a
///   custom recipe), so they fit the scene pass and hot reload rebuilds them.
/// - Set bind groups from 1 up; group 0 is the camera and is rebound before
///   each layer draws.
/// - Write radiance in linear HDR; never tone map. The display pass does that.
pub trait SceneLayer {
    /// A short name for GPU debug groups and telemetry.
    fn label(&self) -> &str;

    /// Work before the scene pass. The default does nothing.
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let _ = ctx;
    }

    /// Records this layer's draws for `phase` into the scene pass.
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>);

    /// The scene copies this layer reads this frame (see [`crate::copies`]). Asked
    /// every frame before `prepare`; a copy is taken when any layer asks for it.
    /// The default asks for none.
    fn scene_copies(&self) -> SceneCopies {
        SceneCopies::NONE
    }

    /// Records this layer's shadow casters into one sun shadow map's depth-only
    /// pass. Group 0 is bound to that map's shadow camera (`naval::camera::camera`
    /// holds its orthographic matrices; `ctx.camera` is the same block), so a
    /// layer's usual vertex code places casters. Use pipelines from
    /// [`crate::RenderContext::add_caster_pipeline`]. Called for every map drawn this
    /// frame (`map.kind` says which; `map.view` and `map.projection` bound it for
    /// culling). The default casts nothing.
    fn draw_shadow(&self, map: &ShadowMap, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let _ = (map, pass, ctx);
    }

    /// Records this layer's shadow casters into a point light's map (`ShadowMapKind::Light`: a searchlight's along its
    /// beam, a star shell's looking down; [`crate::lights::light_shadow_maps`]), a depth-only pass as the sun's maps
    /// have, group 0 bound to the light's perspective camera. Called for each of [`PrepareContext::light_maps`] (the
    /// hulls cast into them; land, aircraft and effects do not). The default casts nothing.
    fn draw_light_shadow(
        &self,
        map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        let _ = (map, pass, ctx);
    }

    /// Whether this layer draws into the contact depth this frame ([`SceneLayer::draw_contact_depth`]). Asked after
    /// `prepare`, while contact shadows are on; the pass and the march run only when some layer says so. The default
    /// is false.
    fn draws_contact_depth(&self) -> bool {
        false
    }

    /// Records the surfaces of this layer that take contact shadows ([`crate::contact`]) into the contact depth, a
    /// depth-only pass at the scene's size drawn before the scene pass: what this layer will draw opaque this frame,
    /// as the scene pass draws it. Group 0 is bound to the frame's own camera in the caster layout (`ctx.camera` is the
    /// same block, jitter and all), so a layer's caster pipelines
    /// ([`crate::RenderContext::add_caster_pipeline`]) place its surfaces where the scene pass will. A layer that draws
    /// here reads [`crate::contact::CONTACT_MASK`] at its own pixels; one that does not must not read it. Called only
    /// when [`SceneLayer::draws_contact_depth`] is true. The default draws nothing.
    fn draw_contact_depth(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let _ = (pass, ctx);
    }

    /// Records this layer's moving surfaces into the motion pass, drawn after the
    /// scene when temporal anti-aliasing is on (see [`crate::taa`]): object motion
    /// of what moves relative to the world (`naval::motion::surface_motion`), or a
    /// response for surfaces whose pixels should take less history or none
    /// (`motion_response(1.0)` for the sea and tracers). Use pipelines from
    /// [`crate::RenderContext::add_motion_pipeline`]. Static geometry draws nothing here:
    /// the resolve reconstructs camera motion from depth. The default draws nothing.
    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let _ = (pass, ctx);
    }

    /// Records responses into the motion pass after every layer's [`SceneLayer::draw_motion`]:
    /// for a layer drawn early whose transparent surfaces lie in front of later layers' (the
    /// sky's rain in front of a hull, whose motion replaces what is below it). Use
    /// `motion_response`, which adds. The default draws nothing.
    fn draw_motion_over(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let _ = (pass, ctx);
    }

    /// Work at a copy point, after that point's copies are taken and before the
    /// scene pass resumes: a layer's own passes on the copies (a depth pyramid for
    /// screen-space reflections, a downsampled depth for half-resolution
    /// particles). Runs only at points where copies were taken this frame. The
    /// default does nothing.
    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        let _ = (point, ctx);
    }

    /// Whether this layer draws in the overlay pass this frame. The pass is recorded only when
    /// some layer says so. The default is false.
    fn has_overlay(&self) -> bool {
        false
    }

    /// Records this layer's overlay: drawn after the post chain onto the output, at its full
    /// resolution whatever the render scale, without depth, temporal AA or edge smoothing, as
    /// the page sits over the scene. Colours are display space (after tone
    /// mapping). Use pipelines from [`crate::RenderContext::add_overlay_pipeline`], looked up with
    /// [`OverlayContext::pipeline`]. Called only when [`SceneLayer::has_overlay`] is true.
    fn draw_overlay(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &OverlayContext<'_>) {
        let _ = (pass, ctx);
    }

    /// The view the next `prepare` and draws are for ([`SceneView`]), told before each view of a frame that draws a
    /// second one. A layer that keeps state for one camera or one viewport size (screen targets, history across frames,
    /// what it culled against the camera) keeps a copy for the second view, or draws nothing there. The default does
    /// nothing.
    fn set_view(&mut self, view: SceneView) {
        let _ = view;
    }
}

/// A vertex buffer layout that owns its attributes, for [`ScenePipeline`].
#[derive(Clone, Debug)]
pub struct VertexLayout {
    pub stride: u64,
    pub step_mode: wgpu::VertexStepMode,
    pub attributes: Vec<wgpu::VertexAttribute>,
}

/// A pipeline drawn in the scene pass: HDR color, reversed depth, the camera
/// at group 0 and the layer's own bind groups after it.
#[derive(Clone, Debug)]
pub struct ScenePipeline {
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub vertex_buffers: Vec<VertexLayout>,
    /// Groups 1, 2, …; group 0 is always the camera.
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    pub primitive: wgpu::PrimitiveState,
    /// `true` for opaque geometry, `false` for the sky and transparents.
    pub depth_write: bool,
    pub blend: Option<wgpu::BlendState>,
    pub defs: ShaderDefs,
}

impl ScenePipeline {
    /// An opaque, back-face-culled triangle list with no vertex buffers.
    pub fn new(vertex_entry: &'static str, fragment_entry: &'static str) -> Self {
        Self {
            vertex_entry,
            fragment_entry,
            vertex_buffers: Vec::new(),
            bind_group_layouts: Vec::new(),
            primitive: wgpu::PrimitiveState {
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_write: true,
            blend: None,
            defs: Vec::new(),
        }
    }
}

/// The scene pass's depth state: reversed depth, `GreaterEqual`.
pub fn scene_depth_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(write),
        depth_compare: Some(DEPTH_COMPARE),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}
