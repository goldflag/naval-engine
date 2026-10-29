//! The renderer and the frame it records: explicit passes over owned targets.
//!
//! A frame is, in order:
//!
//! 1. **Prepare.** Each [`SceneLayer`]'s `prepare` runs with the frame's command
//!    encoder: uniform writes, compute work, its own offscreen passes.
//! 2. **Sun shadows.** When shadows are on and a [`Sun`] is set, each shadow map
//!    due this frame is a depth-only pass drawn by every layer's
//!    [`SceneLayer::draw_shadow`] (see [`crate::shadows`]).
//! 3. **Scene pass.** Render passes into `scene.hdr` (`Rgba16Float`, cleared
//!    to black) with `scene.depth` (`Depth32Float`, reversed, cleared to 0).
//!    The camera is bound at group 0, then every layer draws in each [`Phase`]:
//!    all layers' `Opaque`, then all `Water`, `Sky` and `Transparent`. Where a
//!    layer asked for scene copies ([`SceneLayer::scene_copies`]), the pass ends
//!    after `Opaque` or after `Sky`, the copies are taken, layers may record
//!    their own passes on them ([`SceneLayer::after_copies`]), and the scene
//!    carries on in a new pass that loads what was drawn (see [`crate::copies`]).
//! 4. **Post.** The eye adaptation meters `scene.hdr` against the sky's mean
//!    radiance and steps its exposure; bloom blurs the graded highlights (see
//!    [`crate::post`]).
//! 5. **Display pass.** `scene.hdr` through the look (with the eye's exposure),
//!    the bloom and AgX onto the output (see [`crate::display`]). An 8- or 10-bit
//!    output is dithered by whichever pass writes it (see [`crate::dither`]); a float
//!    output takes values above SDR white when the renderer has a headroom above 1
//!    ([`Renderer::set_headroom`]: HDR output).
//!
//! With temporal anti-aliasing on (see [`crate::taa`]), the camera is jittered,
//! a motion pass follows the scene passes ([`SceneLayer::draw_motion`]), the
//! display pass writes `display.frame`, and a resolve and an output pass put the
//! result on the output. FXAA and SMAA also read `display.frame`, and below full
//! render scale the whole frame is drawn smaller and upscaled at the end (see
//! [`crate::antialiasing`]): by temporal AA's resolve at the output's size (temporal
//! upscaling), or bilinearly after FXAA or SMAA. Overlays ([`SceneLayer::draw_overlay`])
//! go on last, at the output's full resolution.
//!
//! External systems (ships, the ocean, the sky) plug in as [`SceneLayer`]s:
//! they build their pipelines through a [`RenderContext`] (so hot reload
//! reaches them) and are handed to [`Renderer::render`] each frame.
//!
//! The files: `layer.rs` (what a layer is: [`SceneLayer`], the phases, the contexts its calls get), `context.rs`
//! ([`RenderContext`]), `render.rs` ([`Renderer::render`], phase by phase), `second_view.rs`
//! ([`Renderer::render_view`]), `scene_pass.rs` (the scene pass's groups and copies), `settings.rs` (what the host sets
//! and reads between frames) and `look_previews.rs` ([`Renderer::look_previews`]).

mod context;
mod layer;
mod look_previews;
mod render;
mod scene_pass;
mod second_view;
mod settings;

pub use context::RenderContext;
pub use layer::{
    DrawContext, OverlayContext, OverlayPipeline, OverlayPipelineId, Phase, PrepareContext,
    SceneLayer, ScenePipeline, SceneView, VertexLayout, scene_depth_state,
};
pub use second_view::SecondView;

use self::layer::OverlayEntry;

use bytemuck::bytes_of;

use crate::aerial::AerialVolume;
use crate::antialiasing::{Antialiasing, PostChain};
use crate::camera::{Camera, CameraMotion, CameraUniforms, FrameTime};
use crate::copies::{CopyPoint, SceneCopies};
use crate::debug_view::DebugViews;
use crate::display::DisplayPass;
use crate::dither::Dither;
use crate::pipelines::{PipelineRegistry, PlaceholderPipelines, RebuildReport};
use crate::post::{Bloom, EyeAdaptation};
use crate::resources::{Resources, TextureDesc};
use crate::shaders::{ShaderError, ShaderLibrary};
use crate::shadows::{ShadowFit, ShadowMap, ShadowTargets, Sun};
use crate::taa::Taa;
use crate::timing::GpuTimer;
use crate::upload::{UploadStats, Uploads};

/// The scene's color target: linear HDR radiance.
pub const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The scene's depth target: reversed floating-point depth.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Reversed depth: nearer is greater.
pub const DEPTH_COMPARE: wgpu::CompareFunction = wgpu::CompareFunction::GreaterEqual;
/// Reversed depth: the far plane.
pub const DEPTH_CLEAR: f32 = 0.0;
/// Registry name of the scene's HDR color target.
pub const SCENE_HDR: &str = "scene.hdr";
/// Registry name of the scene's depth target.
pub const SCENE_DEPTH: &str = "scene.depth";

/// Whether an output of `format` holds values above SDR white: a float format, as an HDR surface
/// (`Rgba16Float` in extended sRGB) or the HDR capture is. Only such an output takes the
/// renderer's headroom ([`Renderer::set_headroom`]); any other is drawn for SDR.
pub fn extended_range(format: wgpu::TextureFormat) -> bool {
    matches!(
        format,
        wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float
    )
}

/// Group 0: the camera's uniform buffer, the sun's shadow maps beside it, and the scene's point lights.
struct CameraBinding {
    /// Group 0 of scene pipelines: the camera (binding 0) and the shadow maps (1 to 4).
    layout: wgpu::BindGroupLayout,
    /// Group 0 of caster pipelines: a camera block alone (the map's shadow camera).
    caster_layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    /// `CameraMotion` at binding 5.
    motion: wgpu::Buffer,
    /// The aerial-perspective volume at bindings 6 and 7.
    aerial: AerialVolume,
    /// The point lights (`naval::lights::SceneLights`) at binding 10, their clustered index at binding 11.
    lights: wgpu::Buffer,
    clusters: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// The shadow targets' version `group` was built on.
    shadows_version: u64,
}

fn camera_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<CameraUniforms>() as u64),
        },
        count: None,
    }
}

impl CameraBinding {
    fn new(device: &wgpu::Device) -> (Self, ShadowTargets) {
        let mut entries = vec![camera_entry()];
        entries.extend(crate::shadows::view_layout_entries());
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 5,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(std::mem::size_of::<CameraMotion>() as u64),
            },
            count: None,
        });
        entries.extend(AerialVolume::layout_entries());
        entries.extend(crate::shadows::occluder_layout_entries());
        // The point lights and their clustered index, read where surfaces are shaded.
        entries.push(crate::lights::scene_lights_entry());
        entries.push(crate::lights::clusters_entry());
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("camera and shadows"),
            entries: &entries,
        });
        let caster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow caster camera"),
            entries: &[camera_entry()],
        });
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let motion = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("camera motion"),
            size: std::mem::size_of::<CameraMotion>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadows = ShadowTargets::new(device, &caster_layout);
        let aerial = AerialVolume::new(device);
        let lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point lights"),
            size: std::mem::size_of::<crate::lights::SceneLightsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let clusters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("point light clusters"),
            size: crate::lights::CLUSTER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = Self::make_group(
            device, &layout, &buffer, &motion, &shadows, &aerial, &lights, &clusters,
        );
        (
            Self {
                layout,
                caster_layout,
                buffer,
                motion,
                aerial,
                lights,
                clusters,
                group,
                shadows_version: shadows.version,
            },
            shadows,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn make_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        buffer: &wgpu::Buffer,
        motion: &wgpu::Buffer,
        shadows: &ShadowTargets,
        aerial: &AerialVolume,
        lights: &wgpu::Buffer,
        clusters: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let [aerial_volume, aerial_sampler] = aerial.group_entries();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera and shadows"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: shadows.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&shadows.cascades_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&shadows.wide_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&shadows.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: motion.as_entire_binding(),
                },
                aerial_volume,
                aerial_sampler,
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: shadows.occluders.texels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
                    resource: shadows.occluders.uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: lights.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: clusters.as_entire_binding(),
                },
            ],
        })
    }

    /// Rebuilds the group when the shadow maps were remade.
    fn follow(&mut self, device: &wgpu::Device, shadows: &ShadowTargets) {
        if self.shadows_version != shadows.version {
            self.group = Self::make_group(
                device,
                &self.layout,
                &self.buffer,
                &self.motion,
                shadows,
                &self.aerial,
                &self.lights,
                &self.clusters,
            );
            self.shadows_version = shadows.version;
        }
    }
}

/// The camera and clock of one frame.
#[derive(Clone, Copy, Debug)]
pub struct FrameInput<'a> {
    pub camera: &'a Camera,
    pub time: FrameTime,
}

/// Where the display pass writes: a surface texture or a capture target.
#[derive(Clone, Copy, Debug)]
pub struct RenderOutput<'a> {
    pub view: &'a wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub width: u32,
    pub height: u32,
}

/// The scene pass's groups: each group's phases, the copy point after it, and its pass's label.
type SceneGroups = Vec<(Vec<Phase>, Option<CopyPoint>, String)>;

/// What a frame did, for telemetry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameReport {
    pub frame: u64,
    pub time: f64,
    pub viewport: (u32, u32),
    /// Layer labels in draw order.
    pub layers: Vec<String>,
    /// The scene copies taken.
    pub copies: SceneCopies,
    /// The scene's render passes, by the phases each drew.
    pub scene_passes: Vec<String>,
    /// The sun shadow maps drawn this frame.
    pub shadow_maps: Vec<&'static str>,
    /// Whether contact shadows were marched this frame ([`crate::contact`]).
    pub contact: bool,
}

/// Files changed by hot reload, and what rebuilding their pipelines did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReloadReport {
    /// Library paths whose text changed and composed.
    pub changed: Vec<String>,
    /// Modules that no longer compose; the library keeps their last good build.
    pub library_errors: Vec<(String, String)>,
    pub pipelines: RebuildReport,
}

impl ReloadReport {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.library_errors.is_empty()
    }
}

/// The renderer: shader library, pipelines, resources, camera, and the scene
/// and display passes.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    library: ShaderLibrary,
    pipelines: PipelineRegistry,
    resources: Resources,
    camera: CameraBinding,
    shadow_fit: ShadowFit,
    shadow_targets: ShadowTargets,
    sun: Option<Sun>,
    /// The frame's point lights ([`Renderer::set_point_lights`]), their clustered index for the view being drawn, and the
    /// maps the brightest casters among them draw this frame (`crate::lights`).
    point_lights: crate::lights::PointLights,
    light_clusters: crate::lights::LightClusters,
    light_maps: Vec<ShadowMap>,
    /// Contact shadows' march and the casters' camera group (`crate::contact`).
    contact: crate::contact::ContactShadows,
    timer: GpuTimer,
    /// The frame's buffer uploads, submitted ahead of it.
    uploads: Uploads,
    /// The scene pass's groups of phases and their labels for the copies last asked for (worked out again only
    /// when the copies change), and a report handed back for reuse ([`Renderer::recycle_report`]).
    scene_groups: (SceneCopies, SceneGroups),
    spare_report: Option<FrameReport>,
    /// What the last frame uploaded through them.
    upload_stats: UploadStats,
    /// Copies taken whatever the layers ask for (measurement and debugging).
    extra_copies: SceneCopies,
    /// Frames submitted, for the timer (a capture renders one clock frame twice).
    submitted: u64,
    /// Where the frame's parts finish as they are recorded (`crate::finish`); `None` finishes the frame in one encoder.
    finisher: Option<std::sync::Arc<dyn crate::finish::Finisher>>,
    /// The parts the frame hands to `finisher`, and where they come back.
    parts: crate::finish::Parts,
    /// Command buffers for the start of the next frame's submission (`Renderer::submit_with_frame`).
    leading: Vec<wgpu::CommandBuffer>,
    display: DisplayPass,
    eye: EyeAdaptation,
    bloom: Bloom,
    bloom_on: bool,
    /// The sun's glare in the eye, which the bloom's composite adds (`crate::sun_glare`).
    sun_glare: crate::sun_glare::SunGlare,
    taa: Taa,
    antialiasing: Antialiasing,
    /// The scene's size over the output's (see [`crate::antialiasing`]).
    render_scale: f32,
    /// The most pixels the scene's base may hold before the render scale: a larger output's base is shrunk to it
    /// ([`Renderer::scene_size`]); `None` draws from the output itself.
    scene_pixel_cap: Option<u64>,
    chain: PostChain,
    overlays: Vec<OverlayEntry>,
    /// The latest output's format, which new overlay pipelines are built for.
    output_format: wgpu::TextureFormat,
    /// A black texture the display reads without bloom.
    black: (wgpu::Texture, wgpu::TextureView),
    /// The host's underwater probe and how often it has changed ([`Renderer::set_underwater`]).
    underwater: (Option<wgpu::TextureView>, u64),
    /// Whether the shimmer draws (Graphics → Heat shimmer, [`Renderer::set_shimmer`]), the air over the sea it draws
    /// ([`Renderer::set_sea_air`]) and its field's phases, moved by the main view's frames.
    shimmer_on: bool,
    sea_air: crate::shimmer::SeaAir,
    shimmer_clock: crate::shimmer::ShimmerClock,
    /// The output dither's mask and phase.
    dither: Dither,
    /// The display's headroom over SDR white, which a float output is drawn for (1: SDR).
    headroom: f32,
    #[cfg(not(target_arch = "wasm32"))]
    watcher: crate::hot_reload::ShaderWatcher,
    placeholders: std::sync::OnceLock<PlaceholderPipelines>,
    /// The developer panel's debug view.
    debug: DebugViews,
    /// Photo mode's post (`crate::photo`): its depth of field's passes, and the finish the display gives the frame.
    dof: crate::photo::DepthOfFieldPass,
    /// Photo mode's lens flare and glare (`crate::photo_optics`).
    optics: crate::photo_optics::OpticsPass,
    photo: Option<crate::photo::PhotoPost>,
    /// Photo mode's shutter (`crate::shutter`) and the moving bodies it blurs.
    shutter: crate::shutter::ShutterPass,
    photo_bodies: Vec<crate::shutter::MovingBody>,
    /// The film camera in play (`crate::play_camera`), and the seconds between the frames temporal AA's motion spans
    /// (the last new frame's step), which its shutter scales by.
    play_camera: Option<crate::play_camera::PlayCamera>,
    motion_interval: f32,
    /// The game's look (the Graphics menu's Look); photo mode's replaces it while photo mode is up.
    look: crate::photo::Look,
    /// The look the last frame was graded with: temporal AA's history holds its colours.
    drawn_look: crate::photo::Look,
    /// The last frame's camera planes, near and far, for the Look tab's previews of it (`crate::look_preview`).
    clip_planes: [f32; 2],
    /// The look's film after edge smoothing (`crate::film`).
    film: crate::film::FilmPass,
}

impl Renderer {
    /// A renderer whose scene targets start at `width` × `height`; they follow
    /// the output's size after that. Builds the display pipeline for
    /// `output_format` up front.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
    ) -> Result<Self, ShaderError> {
        Self::with_pipeline_cache(device, queue, width, height, output_format, None)
    }

    /// [`Renderer::new`] with a pipeline cache every pipeline it and its layers build is handed (Vulkan's
    /// `Features::PIPELINE_CACHE`; see [`crate::pipelines::PipelineBuild::cache`]), its own post passes included.
    pub fn with_pipeline_cache(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        output_format: wgpu::TextureFormat,
        cache: Option<wgpu::PipelineCache>,
    ) -> Result<Self, ShaderError> {
        let mut resources = Resources::new(width, height);
        resources.create_texture(
            device,
            SCENE_HDR,
            TextureDesc::viewport(
                HDR_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                1.0,
            ),
        );
        resources.create_texture(
            device,
            SCENE_DEPTH,
            TextureDesc::viewport(
                DEPTH_FORMAT,
                wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                1.0,
            ),
        );
        let (camera, shadow_targets) = CameraBinding::new(device);
        let mut library = ShaderLibrary::new();
        let mut pipelines = PipelineRegistry::default();
        // Every pipeline from here on compiles on build threads (native), so a load's pipelines
        // compile together; lookups wait for theirs (see `crate::pipelines`).
        pipelines.set_build_threads(crate::pipelines::default_build_threads());
        pipelines.set_cache(cache);
        let eye = EyeAdaptation::new(device, &mut library, &mut pipelines, &mut resources)?;
        let sun_glare =
            crate::sun_glare::SunGlare::new(device, &mut library, &mut pipelines, &mut resources)?;
        let bloom = Bloom::new(device, &mut library, &mut pipelines)?;
        let taa = Taa::new(device, &mut library, &mut pipelines)?;
        let chain = PostChain::new(device, queue, &mut library, &mut pipelines)?;
        let dof = crate::photo::DepthOfFieldPass::new(device, &mut library, &mut pipelines)?;
        let optics = crate::photo_optics::OpticsPass::new(device, &mut library, &mut pipelines)?;
        let shutter = crate::shutter::ShutterPass::new(device, &mut library, &mut pipelines)?;
        let film = crate::film::FilmPass::new(device, &mut library)?;
        let contact = crate::contact::ContactShadows::new(
            device,
            &mut library,
            &mut pipelines,
            &camera.layout,
            &camera.caster_layout,
            &camera.buffer,
        )?;
        let black = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("no bloom"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: HDR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let black_view = black.create_view(&Default::default());
        let mut renderer = Self {
            device: device.clone(),
            queue: queue.clone(),
            library,
            pipelines,
            resources,
            camera,
            shadow_fit: ShadowFit::default(),
            shadow_targets,
            point_lights: crate::lights::PointLights::NONE,
            light_clusters: crate::lights::LightClusters::default(),
            light_maps: Vec::new(),
            sun: None,
            contact,
            timer: GpuTimer::new(device, queue),
            uploads: Uploads::new(device),
            upload_stats: UploadStats::default(),
            scene_groups: (SceneCopies::NONE, Vec::new()),
            spare_report: None,
            extra_copies: SceneCopies::NONE,
            submitted: 0,
            finisher: None,
            parts: crate::finish::Parts::new(),
            leading: Vec::new(),
            display: DisplayPass::new(device),
            eye,
            bloom,
            bloom_on: false,
            sun_glare,
            taa,
            antialiasing: Antialiasing::None,
            render_scale: 1.0,
            scene_pixel_cap: None,
            chain,
            overlays: Vec::new(),
            output_format,
            black: (black, black_view),
            underwater: (None, 0),
            shimmer_on: false,
            sea_air: crate::shimmer::SeaAir::TYPICAL,
            shimmer_clock: crate::shimmer::ShimmerClock::default(),
            dither: Dither::new(device, queue),
            headroom: 1.0,
            #[cfg(not(target_arch = "wasm32"))]
            watcher: crate::hot_reload::ShaderWatcher::default(),
            placeholders: std::sync::OnceLock::new(),
            debug: DebugViews::default(),
            dof,
            optics,
            photo: None,
            shutter,
            photo_bodies: Vec::new(),
            play_camera: None,
            motion_interval: 0.0,
            look: crate::photo::Look::Natural,
            drawn_look: crate::photo::Look::Natural,
            clip_planes: [0.5, 60_000.0],
            film,
        };
        renderer.display.ensure_format(
            &renderer.device,
            &mut renderer.library,
            &mut renderer.pipelines,
            output_format,
        )?;
        renderer.display.ensure_cover(
            &renderer.device,
            &mut renderer.library,
            &mut renderer.pipelines,
        )?;
        Ok(renderer)
    }

    /// How far a camera looks from the sun, by the sun's height (`crate::post::sun_away`): the eye's room for sunlit
    /// cloud tops. None with the sun down: the key light is then the moon once it outshines the sun's last light
    /// (`naval_clouds::key_light`), while the eye's daylight lasts to 4° under the horizon.
    fn sun_away(&self, camera: &Camera) -> f32 {
        if self
            .eye
            .sun_elevation
            .is_some_and(|elevation| elevation <= 0.0)
        {
            return 0.0;
        }
        self.sun.as_ref().map_or(0.0, |sun| {
            let forward = -camera.view().inverse().z_axis.truncate().normalize();
            crate::post::sun_away(forward, sun.toward_light)
        })
    }

    /// Indexes the point lights for `camera` and writes them and their index at group 0 (bindings 10 and 11), with
    /// this frame's light maps when `shadowed` (from layer `first_layer` of the sun's array).
    fn write_lights(&mut self, camera: &Camera, aspect: f32, first_layer: u32, shadowed: bool) {
        self.light_clusters
            .build(&self.point_lights, camera, aspect);
        let maps: &[ShadowMap] = if shadowed { &self.light_maps } else { &[] };
        let uniform = self.point_lights.uniform(maps, first_layer);
        self.uploads
            .write_buffer(&self.camera.lights, 0, bytes_of(&uniform));
        self.uploads.write_buffer(
            &self.camera.clusters,
            0,
            bytemuck::cast_slice(self.light_clusters.words()),
        );
    }
}
