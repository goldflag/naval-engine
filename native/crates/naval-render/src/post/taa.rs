//! Temporal anti-aliasing, ported from the web game's `TemporalAntialiasing.ts`.
//!
//! With it on, the camera takes a Halton (2, 3) sub-pixel offset each frame, layers write their
//! surfaces' own motion in a **motion pass** after the scene ([`crate::SceneLayer::draw_motion`]),
//! the display pass writes its frame (graded, tone mapped, sRGB-encoded) into `display.frame`
//! instead of the output, and a resolve blends it into a reprojected history in `taa.resolved`;
//! the post chain's output pass then writes that to the output, with FXAA where the resolve took
//! no history (see [`crate::antialiasing`]). Every pixel's camera motion is reconstructed
//! from depth; only moving surfaces write motion, and surfaces whose pixels the history cannot
//! follow (the sea's waves, tracers) write a response of 1 to take the current frame alone.
//!
//! **The motion pass** draws into `scene.motion` (`Rgba16Float`: object motion in the previous
//! frame's NDC in xy, temporal response in z, coverage in w; cleared to 0) with `scene.depth`
//! bound read-only and tested `GreaterEqual` with a small bias toward the camera, so a surface's
//! motion lands exactly on the pixels where it is the nearest thing drawn. The blend is
//! (One, OneMinusSrcAlpha): a surface writing alpha 1 replaces what is below, one writing alpha 0
//! adds its response. Static geometry draws nothing there. Unlike the web game's multiple render target
//! this leaves every scene pipeline as it was (one color target), at the cost of drawing moving
//! geometry a second time, position only.
//!
//! **Determinism.** The jitter and the history step with the frame index: rendering one frame
//! again (a capture, `measure_frames`) resolves against the same history and gives the same image.
//!
//! **Temporal upscaling.** The resolve and its history are at the output's size. Below full render
//! scale the scene, its motion and the display frame are drawn smaller, jittered in the scene's
//! pixels over more phases ([`jitter_phases`]: about eight samples per output pixel a scene pixel
//! covers, as FSR 2 takes), and the resolve reconstructs each output pixel from the scene's samples
//! about it and the reprojected history at the output's size (TAAU): a sample that falls near the
//! output pixel's centre counts for more, one that falls far from it for less ([`confidence_scale`]
//! keeps the mean weight the full-scale resolve's). Overlays stay at the output's size, as they are
//! drawn after the chain.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Vec2};

use crate::Camera;
use crate::frame::{DEPTH_FORMAT, SCENE_DEPTH};
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::fullscreen_recipe;
use crate::resources::{Lifetime, Resources, TextureDesc};
use crate::shaders::{ShaderDefValue, ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;

pub const TAA_SHADER: ShaderFile = crate::shader_file!("shaders/post/taa.wgsl");

/// Registry name of the motion target (`Rgba16Float`), written by the motion pass.
pub const SCENE_MOTION: &str = "scene.motion";
/// Registry name of the display frame temporal AA resolves (`Rgba16Float`, display space).
pub const DISPLAY_FRAME: &str = "display.frame";
/// Registry name of the resolved frame (display space; alpha is view depth, negative where the
/// pixel took no history), which the post chain's output pass reads.
pub const TAA_RESOLVED: &str = "taa.resolved";
const RESOLVED: &str = TAA_RESOLVED;
const HISTORY: &str = "taa.history";

/// The motion target's format.
pub const MOTION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// The display frame's format with temporal AA (display-space values, sRGB-encoded).
pub const FRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// `TemporalAntialiasing`'s tunables.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaaSettings {
    /// Weight of the current frame with a still camera; 1/20 converges in about a third of a second.
    pub minimum_weight: f32,
    /// Pixel speed at which the current frame fully replaces history.
    pub max_velocity_px: f32,
    /// Relative change of view depth beyond which history belongs to another surface.
    pub depth_tolerance: f32,
}

impl Default for TaaSettings {
    fn default() -> Self {
        Self {
            minimum_weight: 0.06,
            max_velocity_px: 96.0,
            depth_tolerance: 0.05,
        }
    }
}

/// The web's 16 Halton (2, 3) offsets, in pixels about the pixel centre.
pub fn jitter(frame: u64) -> Vec2 {
    jitter_of(frame, FULL_SCALE_PHASES)
}

/// The full-scale jitter's phases (the web's).
const FULL_SCALE_PHASES: u64 = 16;

/// The Halton (2, 3) offset of `frame` in a cycle of `phases`, in the scene's pixels about the pixel centre.
pub fn jitter_of(frame: u64, phases: u64) -> Vec2 {
    let halton = |index: u64, base: u64| {
        let (mut fraction, mut result, mut i) = (1.0f64, 0.0f64, index);
        while i > 0 {
            fraction /= base as f64;
            result += fraction * (i % base) as f64;
            i /= base;
        }
        result
    };
    let index = frame % phases.max(1) + 1;
    Vec2::new(
        (halton(index, 2) - 0.5) as f32,
        (halton(index, 3) - 0.5) as f32,
    )
}

/// The jitter's phases for a scene drawn at `scale` of the output's size: the web's 16 at full scale, and below it
/// enough that each output pixel a scene pixel covers sees about eight samples a cycle (FSR 2's `8 / scale²`).
pub fn jitter_phases(scale: f32) -> u64 {
    let scale = clamp_scale(scale);
    FULL_SCALE_PHASES.max((8.0 / (scale * scale)).ceil() as u64)
}

/// The weight the resolve gives a scene sample `d` output pixels from an output pixel's centre: Karis's
/// Blackman-Harris window, `exp(-2.29 d²)` (as `naval::taa`'s filtered sample).
const WINDOW: f64 = 2.29;

/// 1 / the mean weight of the nearest scene sample to an output pixel's centre when the scene is drawn at `scale` of
/// the output's size: the upscaling resolve scales the current frame's share by the nearest sample's weight over this
/// mean, so the history takes on average what it takes at full scale. The nearest sample lies uniformly within half a
/// scene pixel each way, `1 / scale` output pixels wide.
pub fn confidence_scale(scale: f32) -> f32 {
    let wide = 1.0 / f64::from(clamp_scale(scale));
    // The mean over one axis, by the midpoint rule (the window is smooth), squared for the two axes.
    const STEPS: usize = 256;
    let axis = (0..STEPS)
        .map(|i| {
            let u = ((i as f64 + 0.5) / STEPS as f64 - 0.5) * wide;
            (-WINDOW * u * u).exp()
        })
        .sum::<f64>()
        / STEPS as f64;
    (1.0 / (axis * axis)) as f32
}

fn clamp_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(0.25, 1.0)
    } else {
        1.0
    }
}

/// A pipeline for the motion pass (see the module docs), built with
/// [`crate::RenderContext::add_motion_pipeline`]. Group 0 is the camera, as in the scene pass;
/// `naval::motion` gives the fragment's output.
#[derive(Clone, Debug)]
pub struct MotionPipeline {
    pub vertex_entry: &'static str,
    pub fragment_entry: &'static str,
    pub vertex_buffers: Vec<crate::VertexLayout>,
    /// Groups 1, 2, …
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    pub primitive: wgpu::PrimitiveState,
    pub defs: crate::ShaderDefs,
}

impl MotionPipeline {
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
            defs: Vec::new(),
        }
    }
}

/// The motion pass's color target and its blend.
pub fn motion_target() -> wgpu::ColorTargetState {
    let component = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::ColorTargetState {
        format: MOTION_FORMAT,
        blend: Some(wgpu::BlendState {
            color: component,
            alpha: component,
        }),
        write_mask: wgpu::ColorWrites::ALL,
    }
}

/// The motion pass's depth state: `scene.depth` read-only, `GreaterEqual`, nudged toward the
/// camera so a surface passes against its own depth.
pub fn motion_depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH_FORMAT,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState {
            constant: 4,
            slope_scale: 1.0,
            clamp: 0.0,
        },
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TaaUniforms {
    current: [f32; 16],
    previous: [f32; 16],
    projection_inverse: [f32; 16],
    camera_world: [f32; 16],
    inverse_size: [f32; 2],
    history_weight: f32,
    minimum_weight: f32,
    max_velocity_px: f32,
    depth_tolerance: f32,
    /// This frame's jitter in pixels, which a pixel without history is drawn back from.
    jitter: [f32; 2],
    /// The output's (the resolve's and the history's) texel size: the scene's at full render scale.
    output_inverse_size: [f32; 2],
    /// Upscaling: [`confidence_scale`] of the render scale.
    confidence_scale: f32,
    /// Upscaling: output pixels per scene pixel (1 / the render scale).
    upscale: f32,
}

/// A lens that changes by more than this factor in one frame is a cut: zooming in magnifies the
/// history into a blur at the middle of the view, where nothing moves fast enough to refuse it
/// (the binoculars' snap in and out, the first frames of their glide, a film's change of lens).
const LENS_CUT: f32 = 1.25;
/// A view that turns further than this in one frame (cosine of 30°) is a cut.
const TURN_CUT: f32 = 0.866;
/// A camera that moves farther than this in one frame (m) has cut (the clouds' `CUT_DISTANCE`: the free camera at
/// its fastest flies 200 m a frame).
const JUMP_CUT: f32 = 250.0;
/// A camera whose view moves less than this (pixels) is still.
const STILL_PX: f32 = 0.01;

/// This frame's camera history, from [`Taa::begin`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct TaaFrame {
    pub jitter: Vec2,
    pub previous_view_projection: Mat4,
    /// This frame's unjittered view-projection about its eye (`Camera::view_about_eye`), and the previous frame's for
    /// a point given less this frame's eye: the previous projection and turn, moved by the eye's step since, taken in
    /// doubles. The eye is the camera's own (f32), which the layers that draw world positions draw about, so the
    /// resolve reprojects their surfaces exactly however far from the world's origin.
    pub current_from_eye: Mat4,
    pub previous_from_eye: Mat4,
    /// The full-precision eye's step since the previous frame less the camera's own eye's (metres; millimetres at
    /// most): a layer that draws about the full-precision eye (`PrepareContext::eye`) adds it to its previous
    /// positions (`naval::motion::surface_motion_from_eye`), so its history lines up with the resolve's.
    pub precise_step: glam::Vec3,
    /// The first render of a new frame (not a capture of it again, nor a held frame repeating it).
    pub new_frame: bool,
    /// The clock frame this render resolves: the last new frame's, which a capture or a held frame draws again.
    pub frame: u64,
    /// The camera cut away from the last render's: its history shows something else.
    pub cut: bool,
    /// The view is the last new frame's (a new frame, or one drawn again from where that one stood: a capture, a
    /// pause with the camera still), so the motion since the previous frame is this view's; false for a camera moved
    /// while time is held (the film camera's shutter then draws nothing, having no interval to scale by).
    pub motion_holds: bool,
}

/// The camera over frame indices: the last index begun, its latest unjittered view-projection
/// and the previous index's, and the camera they were drawn with.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TaaHistory {
    frame: Option<u64>,
    current: Mat4,
    previous: Mat4,
    /// The same about each one's eye, the camera's own eye and the full-precision one: [`TaaFrame::previous_from_eye`].
    current_about: (Mat4, DVec3, DVec3),
    previous_about: (Mat4, DVec3, DVec3),
    camera: Option<Camera>,
    /// The unjittered view-projection the last new frame was drawn with, which a held picture
    /// keeps repeating while the camera stays on it.
    anchor: Mat4,
    /// The jitter's phases ([`jitter_phases`]; 0 is the full scale's 16).
    pub(crate) phases: u64,
}

impl TaaHistory {
    /// Steps the camera history for `frame` and returns the jitter and the previous unjittered
    /// view-projection. A repeated index keeps the previous camera, since it resolves against the
    /// same history, but the current one follows every render (a paused viewer orbiting): the
    /// latest render's resolve is what becomes the next frame's history.
    ///
    /// A `held` frame (the presentation paused: no time passed) under a camera that has not moved
    /// is the last frame drawn again, its jitter and history, so a paused picture stands to the
    /// byte as it does without temporal AA (the jitter stepping on had the sea, which takes no
    /// history, dance by a sub-pixel). A camera that turns, zooms or jumps away from the last
    /// frame's is a cut ([`TaaFrame::cut`]). `eye` is the camera's eye at full precision (its own rounds to it).
    pub(crate) fn begin(
        &mut self,
        frame: u64,
        camera: &Camera,
        unjittered_view_projection: Mat4,
        viewport: (u32, u32),
        held: bool,
        eye: DVec3,
    ) -> TaaFrame {
        let about = (
            camera.projection(viewport.0 as f32 / viewport.1.max(1) as f32)
                * camera.view_about_eye(),
            camera.eye.as_dvec3(),
            eye,
        );
        let begun = self.frame.is_some();
        let still_view = begun && still(self.anchor, unjittered_view_projection, viewport);
        let new_frame = self.frame != Some(frame) && !(held && still_view);
        // A camera that zoomed, turned or jumped away from the last render's has cut, a new frame
        // or one drawn again (a capture from a camera placed elsewhere) alike. A smaller move
        // reprojects: drawn again, the frame resolves against the same history from where the
        // camera stands now.
        let cut = self.camera.is_some_and(|last| cuts(&last, camera));
        if new_frame {
            (self.previous, self.previous_about) = if begun && !cut {
                (self.current, self.current_about)
            } else {
                (unjittered_view_projection, about)
            };
            self.frame = Some(frame);
            self.anchor = unjittered_view_projection;
        }
        self.current = unjittered_view_projection;
        self.current_about = about;
        self.camera = Some(*camera);
        let phases = if self.phases == 0 {
            FULL_SCALE_PHASES
        } else {
            self.phases
        };
        TaaFrame {
            jitter: jitter_of(self.frame.unwrap_or(frame), phases),
            previous_view_projection: self.previous,
            current_from_eye: about.0,
            previous_from_eye: previous_from_eye(self.previous_about, about.1),
            precise_step: ((about.2 - self.previous_about.2) - (about.1 - self.previous_about.1))
                .as_vec3(),
            new_frame,
            frame: self.frame.unwrap_or(frame),
            cut,
            motion_holds: new_frame || still_view,
        }
    }
}

/// A view-projection about its eye (`about`: it, that eye) for a point given less `eye`: moved by the step between
/// them, taken in doubles, so a camera kilometres from the world's origin keeps its millimetres.
pub(crate) fn previous_from_eye(
    (view_projection, from, _): (Mat4, DVec3, DVec3),
    eye: DVec3,
) -> Mat4 {
    view_projection * Mat4::from_translation((eye - from).as_vec3())
}

/// The camera cut from `last` to `now`: its lens changed by more than [`LENS_CUT`], its view
/// turned by more than 30° or it moved more than [`JUMP_CUT`] in one frame.
fn cuts(last: &Camera, now: &Camera) -> bool {
    let tan = |c: &Camera| (c.fov_y_degrees.to_radians() * 0.5).tan();
    let lens = tan(last) / tan(now).max(1e-6);
    let forward = |c: &Camera| (c.target - c.eye).normalize_or_zero();
    !(1.0 / LENS_CUT..=LENS_CUT).contains(&lens)
        || forward(last).dot(forward(now)) < TURN_CUT
        || last.eye.distance(now.eye) > JUMP_CUT
}

/// `now` shows the view `then` showed, to [`STILL_PX`]: points at the middle and corners of the
/// view, near (reversed depth 0.5: twice the near plane's distance) and far (1e-3: a thousand
/// times it), land within that many pixels of where `then` drew them.
fn still(then: Mat4, now: Mat4, viewport: (u32, u32)) -> bool {
    if then == now {
        return true;
    }
    // In double precision: the round trip through a far-reaching perspective's inverse loses
    // hundredths of a pixel in single.
    let wide = |m: Mat4| glam::DMat4::from_cols_array(&m.to_cols_array().map(f64::from));
    let (then, inverse) = (wide(then), wide(now).inverse());
    let half = glam::DVec2::new(f64::from(viewport.0), f64::from(viewport.1)) * 0.5;
    [0.5, 1.0e-3].iter().all(|&depth| {
        [
            (0.0, 0.0),
            (-1.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (1.0, 1.0),
        ]
        .iter()
        .all(|&(x, y)| {
            let world = inverse * glam::DVec4::new(x, y, depth, 1.0);
            let before = then * world;
            if before.w.abs() < 1e-12 {
                return false;
            }
            let moved = (before.truncate().truncate() / before.w - glam::DVec2::new(x, y)) * half;
            moved.length() <= f64::from(STILL_PX)
        })
    })
}

pub(crate) struct Taa {
    pub settings: TaaSettings,
    resolve: PipelineId,
    /// The resolve for a scene drawn smaller than the output (`UPSCALE`).
    resolve_upscale: PipelineId,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    resolve_group: Option<(Vec<u64>, wgpu::BindGroup)>,
    /// The camera's history over frame indices.
    history: TaaHistory,
    /// `taa.resolved` holds the last frame's resolve, to become the history.
    resolved_valid: bool,
    /// `taa.history` holds the frame before this one.
    history_valid: bool,
    /// The textures' version the validity refers to.
    texture_version: u64,
}

impl Taa {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(TAA_SHADER)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let texture = |binding, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("taa resolve"),
            entries: &[
                texture(0, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                texture(2, false),
                texture(3, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let resolve = pipelines.add(
            device,
            library,
            "taa resolve",
            TAA_SHADER.path,
            Vec::new(),
            fullscreen_recipe(
                vec![layout.clone()],
                "vertex",
                "resolve",
                FRAME_FORMAT,
                None,
            ),
        )?;
        let resolve_upscale = pipelines.add(
            device,
            library,
            "taa resolve upscale",
            TAA_SHADER.path,
            vec![("UPSCALE".to_owned(), ShaderDefValue::Bool(true))],
            fullscreen_recipe(
                vec![layout.clone()],
                "vertex",
                "resolve",
                FRAME_FORMAT,
                None,
            ),
        )?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("taa"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("taa"),
            size: std::mem::size_of::<TaaUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            settings: TaaSettings::default(),
            resolve,
            resolve_upscale,
            layout,
            sampler,
            uniforms,
            resolve_group: None,
            history: TaaHistory::default(),
            resolved_valid: false,
            history_valid: false,
            texture_version: 0,
        })
    }

    /// Forgets history: the next frame shows the current frame alone.
    pub(crate) fn reset(&mut self) {
        self.history = TaaHistory::default();
        self.resolved_valid = false;
        self.history_valid = false;
    }

    /// Registers the frame and motion targets at the scene's size and the resolve and history
    /// targets at the `output`'s (the scene's at full render scale), if they do not exist at those
    /// sizes.
    pub(crate) fn ensure_targets(
        device: &wgpu::Device,
        resources: &mut Resources,
        output: (u32, u32),
    ) {
        let attachment = wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC;
        for (name, format) in [(SCENE_MOTION, MOTION_FORMAT), (DISPLAY_FRAME, FRAME_FORMAT)] {
            if resources.texture(name).is_none() {
                resources.create_texture(
                    device,
                    name,
                    TextureDesc::viewport(format, attachment, 1.0),
                );
            }
        }
        let output = (output.0.max(1), output.1.max(1));
        for (name, usage) in [
            (RESOLVED, attachment),
            (
                HISTORY,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            ),
        ] {
            let fits = resources.texture(name).is_some_and(|entry| {
                let size = entry.texture.size();
                (size.width, size.height) == output
            });
            if !fits {
                resources.create_texture(
                    device,
                    name,
                    TextureDesc::fixed(
                        FRAME_FORMAT,
                        usage,
                        output.0,
                        output.1,
                        Lifetime::Persistent,
                    ),
                );
            }
        }
    }

    /// Drops the motion, resolve and history targets while temporal AA is off (at 3840 × 1600 they hold about
    /// 150 MB), and the resolve's bind group, which would keep them alive; [`Taa::ensure_targets`] makes them again,
    /// and their new versions forget the history. The display frame is the post chain's to keep or drop
    /// ([`crate::antialiasing`]). Returns whether anything was dropped.
    pub(crate) fn release_targets(&mut self, resources: &mut Resources) -> bool {
        let mut dropped = false;
        for name in [SCENE_MOTION, RESOLVED, HISTORY] {
            dropped |= resources.texture(name).is_some();
            resources.remove(name);
        }
        if dropped {
            self.resolve_group = None;
        }
        dropped
    }

    /// See [`TaaHistory::begin`]; the jitter takes `phases` ([`jitter_phases`]).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin(
        &mut self,
        frame: u64,
        camera: &Camera,
        unjittered_view_projection: Mat4,
        viewport: (u32, u32),
        held: bool,
        phases: u64,
        eye: DVec3,
    ) -> TaaFrame {
        self.history.phases = phases;
        self.history.begin(
            frame,
            camera,
            unjittered_view_projection,
            viewport,
            held,
            eye,
        )
    }

    /// The resolve: `display.frame` (this frame) against the history into `taa.resolved`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        frame: &TaaFrame,
        camera: &crate::CameraUniforms,
    ) {
        let texture = |name| resources.texture(name).expect("made by ensure_targets");
        let (beauty, depth, motion, resolved, history) = (
            texture(DISPLAY_FRAME),
            resources.texture(SCENE_DEPTH).expect("registered"),
            texture(SCENE_MOTION),
            texture(RESOLVED),
            texture(HISTORY),
        );
        // New textures (a resize) hold nothing to reproject.
        let version = resolved.version.max(history.version);
        if version != self.texture_version {
            self.texture_version = version;
            self.resolved_valid = false;
            self.history_valid = false;
        }
        // A new frame index: last frame's resolve becomes the history. A repeated one resolves
        // against the same history again.
        if frame.new_frame {
            self.history_valid = self.resolved_valid && !frame.cut;
            if self.history_valid {
                encoder.copy_texture_to_texture(
                    resolved.texture.as_image_copy(),
                    history.texture.as_image_copy(),
                    resolved.texture.size(),
                );
            }
        }
        // A cut on a frame drawn again (a capture from a camera placed elsewhere) drops the
        // history too: the renders of that frame after it come from the cut camera, not a cut.
        if frame.cut {
            self.history_valid = false;
        }
        let (width, height) = resources.viewport();
        let output = resolved.texture.size();
        // Below full render scale the resolve reconstructs the output from the smaller scene.
        let upscaling = (output.width, output.height) != (width, height);
        let scale = width as f32 / output.width.max(1) as f32;
        // The camera's motion is reprojected about its own eye, which the world's layers draw about: the view's turn
        // back to the world without the eye's position, and the view-projections for a point given less it
        // (`TaaFrame::previous_from_eye`).
        let mut turn_back = Mat4::from_cols_array(&camera.inverse_view);
        turn_back.w_axis = glam::Vec4::W;
        let uniforms = TaaUniforms {
            current: frame.current_from_eye.to_cols_array(),
            previous: frame.previous_from_eye.to_cols_array(),
            projection_inverse: camera.inverse_projection,
            camera_world: turn_back.to_cols_array(),
            inverse_size: [1.0 / width as f32, 1.0 / height as f32],
            history_weight: if self.history_valid && !frame.cut {
                1.0
            } else {
                0.0
            },
            minimum_weight: self.settings.minimum_weight,
            max_velocity_px: self.settings.max_velocity_px,
            depth_tolerance: self.settings.depth_tolerance,
            jitter: frame.jitter.to_array(),
            output_inverse_size: [
                1.0 / output.width.max(1) as f32,
                1.0 / output.height.max(1) as f32,
            ],
            confidence_scale: confidence_scale(scale),
            upscale: 1.0 / clamp_scale(scale),
        };
        uploads.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let versions = vec![
            beauty.version,
            depth.version,
            motion.version,
            history.version,
        ];
        if self
            .resolve_group
            .as_ref()
            .is_none_or(|(built, _)| *built != versions)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("taa resolve"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&beauty.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&motion.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&history.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: self.uniforms.as_entire_binding(),
                    },
                ],
            });
            self.resolve_group = Some((versions, group));
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("taa resolve"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &resolved.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timer.render_pass("taa resolve"),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(if upscaling {
                self.resolve_upscale
            } else {
                self.resolve
            }));
            pass.set_bind_group(0, &self.resolve_group.as_ref().expect("just made").1, &[]);
            pass.draw(0..3, 0..1);
        }
        self.resolved_valid = true;
    }

    /// Whether the history was used this frame (for telemetry).
    pub(crate) fn history_valid(&self) -> bool {
        self.history_valid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: (u32, u32) = (1600, 900);

    fn view_projection(camera: &Camera) -> Mat4 {
        camera.projection(16.0 / 9.0) * camera.view()
    }

    #[test]
    fn a_repeated_frame_keeps_the_previous_camera_and_follows_the_current_one() {
        let (a, b, c) = (
            Mat4::from_scale(glam::Vec3::splat(1.0)),
            Mat4::from_scale(glam::Vec3::splat(2.0)),
            Mat4::from_scale(glam::Vec3::splat(3.0)),
        );
        let camera = Camera::default();
        let mut history = TaaHistory::default();
        let first = history.begin(5, &camera, a, VIEWPORT, false, camera.eye.as_dvec3());
        assert!(first.new_frame);
        assert_eq!(
            first.previous_view_projection, a,
            "no history: its own camera"
        );
        let again = history.begin(5, &camera, b, VIEWPORT, false, camera.eye.as_dvec3());
        assert!(!again.new_frame);
        assert_eq!(again.previous_view_projection, a);
        let next = history.begin(6, &camera, c, VIEWPORT, false, camera.eye.as_dvec3());
        assert_eq!(
            next.previous_view_projection, b,
            "the latest render of the last frame is the one its history holds"
        );
    }

    #[test]
    fn a_held_frame_under_a_still_camera_repeats_the_last_one() {
        let camera = Camera::look_at(glam::Vec3::new(0.0, 20.0, 300.0), glam::Vec3::ZERO);
        let vp = view_projection(&camera);
        let mut history = TaaHistory::default();
        history.begin(10, &camera, vp, VIEWPORT, false, camera.eye.as_dvec3());
        let running = history.begin(11, &camera, vp, VIEWPORT, false, camera.eye.as_dvec3());
        assert!(
            running.new_frame,
            "a still camera in a running scene steps on"
        );
        // Paused: frames 12, 13, … repeat frame 11, its jitter and its history.
        for frame in 12..40 {
            let held = history.begin(frame, &camera, vp, VIEWPORT, true, camera.eye.as_dvec3());
            assert!(!held.new_frame && !held.cut);
            assert_eq!(held.jitter, running.jitter);
            assert_eq!(
                held.previous_view_projection,
                running.previous_view_projection
            );
        }
        // The camera turned a little under the pause: the picture follows it, the jitter steps on.
        let turned = Camera::look_at(camera.eye, glam::Vec3::new(1.0, 0.0, 0.0));
        let moved = history.begin(
            40,
            &turned,
            view_projection(&turned),
            VIEWPORT,
            true,
            turned.eye.as_dvec3(),
        );
        assert!(moved.new_frame && !moved.cut);
        assert_eq!(moved.jitter, jitter(40));
        // It settles: held frames repeat the new picture.
        let settled = history.begin(
            41,
            &turned,
            view_projection(&turned),
            VIEWPORT,
            true,
            turned.eye.as_dvec3(),
        );
        assert!(!settled.new_frame);
        assert_eq!(settled.jitter, jitter(40));
        // Running again: every frame is new.
        assert!(
            history
                .begin(
                    42,
                    &turned,
                    view_projection(&turned),
                    VIEWPORT,
                    false,
                    turned.eye.as_dvec3()
                )
                .new_frame
        );
    }

    #[test]
    fn a_lens_change_or_a_sharp_turn_is_a_cut() {
        let camera = Camera::look_at(glam::Vec3::new(0.0, 20.0, 300.0), glam::Vec3::ZERO);
        let step = |history: &mut TaaHistory, frame, to: &Camera| {
            history.begin(
                frame,
                to,
                view_projection(to),
                VIEWPORT,
                false,
                to.eye.as_dvec3(),
            )
        };
        let mut history = TaaHistory::default();
        assert!(
            !step(&mut history, 1, &camera).cut,
            "the first frame has nothing to cut from"
        );
        // A zoom that settles (a tenth a frame) keeps its history; the binoculars' snap does not.
        let lens = |fov| Camera {
            fov_y_degrees: fov,
            ..camera
        };
        assert!(!step(&mut history, 2, &lens(47.0)).cut);
        assert!(step(&mut history, 3, &lens(4.655)).cut, "x12 at once");
        assert!(step(&mut history, 4, &lens(52.0)).cut, "and back");
        // Turning 10° in a frame is a fast pan; 45° is another view.
        let toward = |degrees: f32| {
            let (sin, cos) = degrees.to_radians().sin_cos();
            Camera::look_at(camera.eye, camera.eye + glam::Vec3::new(-sin, 0.0, -cos))
        };
        assert!(!step(&mut history, 5, &toward(0.0)).cut);
        assert!(!step(&mut history, 6, &toward(10.0)).cut);
        assert!(step(&mut history, 7, &toward(55.0)).cut);
        // A spectated hull a kilometre off is another view; the free camera at its fastest is not.
        let moved = |by: f32| Camera::look_at(camera.eye + glam::Vec3::X * by, glam::Vec3::X * by);
        step(&mut history, 69, &moved(0.0));
        assert!(!step(&mut history, 70, &moved(0.0)).cut);
        assert!(!step(&mut history, 71, &moved(200.0)).cut);
        assert!(step(&mut history, 72, &moved(1200.0)).cut);
        // A cut frame's previous camera is its own: nothing reprojects from the old view.
        let cut = step(&mut history, 8, &lens(4.655));
        assert!(cut.cut);
        assert_eq!(cut.previous_view_projection, view_projection(&lens(4.655)));
        // Frame 8 drawn again as it was is no cut; drawn again from a camera placed elsewhere
        // (a capture after the RPC moved it), it is.
        assert!(!step(&mut history, 8, &lens(4.655)).cut);
        let elsewhere = Camera::look_at(glam::Vec3::new(50.0, 20.0, 300.0), glam::Vec3::ZERO);
        let again = step(&mut history, 8, &elsewhere);
        assert!(!again.new_frame && again.cut);
    }

    #[test]
    fn far_from_the_origin_a_close_body_reprojects_about_the_eye() {
        // A camera 30 km out riding 2 m off a plane flying at 82 m/s, sixty frames a second, on a 3300 × 1856 screen (the
        // gallery's close look), its eye given at full precision. A still surface beside the plane, drawn about the
        // camera's own eye as the world's layers draw, is reprojected into the frame before as the resolve does: about
        // the eye it lands within a thousandth of a pixel of the doubles' answer; through world positions (the resolve
        // before) it missed by pixels. A point on the plane, drawn about the full-precision eye, comes back to where the
        // frame before drew it with the precise step added to its previous position.
        let mut history = TaaHistory::default();
        let pixels = |ndc: glam::DVec2| ndc * glam::DVec2::new(1650.0, 928.0);
        let project = |m: Mat4, p: glam::Vec3| {
            let clip = m * p.extend(1.0);
            pixels((clip.truncate().truncate() / clip.w).as_dvec2())
        };
        let (mut still, mut world, mut plane_point) = (0.0_f64, 0.0_f64, 0.0_f64);
        let mut last: Option<(Camera, DVec3, DVec3)> = None;
        for frame in 0..60_u64 {
            let t = frame as f64 / 60.0;
            let plane = DVec3::new(-29_650.0 + 82.0 * t, 90.0, 12_310.0 + 9.0 * t);
            let eye = plane + DVec3::new(-1.4, 0.5, 1.3);
            let camera = Camera::look_at(eye.as_vec3(), plane.as_vec3());
            let begun = history.begin(
                frame,
                &camera,
                view_projection(&camera),
                VIEWPORT,
                false,
                eye,
            );
            let wall = plane + DVec3::new(0.4, 0.3, -0.6);
            let on_plane = DVec3::new(0.2, 0.9, -1.1);
            if let Some((before, before_eye, before_plane)) = last {
                // The doubles' answers: by the frame before's turn and lens, the surface less the eye it was drawn about.
                let lens = (before.projection(16.0 / 9.0) * before.view_about_eye()).as_dmat4();
                let truth = |offset: DVec3| {
                    let clip = lens * offset.extend(1.0);
                    pixels(clip.truncate().truncate() / clip.w)
                };
                let rounded = camera.eye.as_dvec3();
                let resolved = project(begun.previous_from_eye, (wall - rounded).as_vec3());
                still = still.max(resolved.distance(truth(wall - before.eye.as_dvec3())));
                let drawn = project(begun.previous_view_projection, wall.as_vec3());
                world = world.max(drawn.distance(truth(wall - before.eye.as_dvec3())));
                // The plane's point a frame ago, less this frame's precise eye, moved by the precise step.
                let previous = (before_plane + on_plane - eye).as_vec3() + begun.precise_step;
                let resolved = project(begun.previous_from_eye, previous);
                plane_point =
                    plane_point.max(resolved.distance(truth(before_plane + on_plane - before_eye)));
            }
            last = Some((camera, eye, plane));
        }
        assert!(still < 1e-3, "a still surface about the eye: {still} px");
        assert!(
            plane_point < 1e-3,
            "the plane about the precise eye: {plane_point} px"
        );
        assert!(world > 1.0, "through world positions: {world} px");
    }

    #[test]
    fn below_full_scale_the_jitter_takes_more_phases() {
        assert_eq!(jitter_phases(1.0), 16);
        assert_eq!(jitter_phases(0.75), 16);
        assert_eq!(jitter_phases(0.7), 17);
        assert_eq!(jitter_phases(0.5), 32);
        assert_eq!(jitter_phases(f32::NAN), 16);
        // A longer cycle starts with the web's 16 offsets, and its every phase is its own, within half a pixel.
        for frame in 0..16 {
            assert_eq!(jitter_of(frame, 32), jitter(frame));
        }
        let cycle: Vec<Vec2> = (0..32).map(|frame| jitter_of(frame, 32)).collect();
        for (i, a) in cycle.iter().enumerate() {
            assert!(a.x.abs() <= 0.5 && a.y.abs() <= 0.5);
            assert!(cycle[i + 1..].iter().all(|b| (*a - *b).length() > 1e-3));
        }
        assert_eq!(jitter_of(32, 32), jitter_of(0, 32));
    }

    #[test]
    fn the_confidence_scale_keeps_the_mean_weight() {
        // The nearest sample's mean weight: the window's mean over a pixel each way, squared. At full scale
        // (∫ exp(-2.29 u²) over ±0.5)² = 0.702; at half scale the samples lie twice as far: 0.321.
        assert!((confidence_scale(1.0) - 1.0 / 0.702).abs() < 0.005);
        assert!((confidence_scale(0.5) - 1.0 / 0.3212).abs() < 0.01);
        assert!(confidence_scale(0.5) > confidence_scale(0.7));
        assert!(confidence_scale(0.7) > confidence_scale(1.0));
        assert_eq!(confidence_scale(f32::NAN), confidence_scale(1.0));
    }

    #[test]
    fn the_jitter_is_the_webs_halton_sequence() {
        // halton(1, 2) = 0.5, halton(1, 3) = 1/3; halton(2, 2) = 0.25, halton(2, 3) = 2/3.
        assert!((jitter(0) - Vec2::new(0.0, 1.0 / 3.0 - 0.5)).length() < 1e-6);
        assert!((jitter(1) - Vec2::new(-0.25, 2.0 / 3.0 - 0.5)).length() < 1e-6);
        assert_eq!(jitter(16), jitter(0));
        for frame in 0..16 {
            let j = jitter(frame);
            assert!(j.x.abs() <= 0.5 && j.y.abs() <= 0.5);
        }
    }
}
