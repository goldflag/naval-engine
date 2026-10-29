//! The display pass: linear HDR scene radiance to the output, through the
//! game's look, bloom and three.js's AgX (the web game's `DisplayTransform.ts`).
//!
//! **HDR output.** With an output that holds values above SDR white (a float target: an
//! HDR surface, the HDR capture) and the display's headroom above 1
//! ([`crate::Renderer::set_headroom`]), AgX's top is raised to the headroom over the
//! highlights (`agx_hdr_tone_mapping` in `shaders/naval/display.wgsl`): the picture is AgX's
//! up to a graded 1 (sea, sky, sunlit hulls), and the four stops above it, which SDR packs
//! into its last 0.8 stop, spread over the headroom. Written sRGB-encoded with the curve
//! carried on above white (extended sRGB), as the post chain's frame always is. At a headroom
//! of 1 it is AgX exactly.

use bytemuck::{Pod, Zeroable};

use crate::dither::{Dither, dither_defs};
use crate::pipelines::{Pipeline, PipelineId, PipelineRegistry};
use crate::shaders::{ShaderDefValue, ShaderError, ShaderLibrary, builtin};
use crate::upload::UploadSink;

/// Exposures ahead of the look. `Game.ts` sets `toneMappingExposure = 1`; the
/// scene exposure is the web's `sky.exposure` (1 in the game's sky). The eye
/// adaptation's factor multiplies it on the GPU (see [`crate::post`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplaySettings {
    pub exposure: f32,
    pub tone_mapping_exposure: f32,
}

impl Default for DisplaySettings {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            tone_mapping_exposure: 1.0,
        }
    }
}

/// A camera's exposure against the game's fixed one: a sun-facing grey card with the sun 61° up reads 2.7 times its
/// albedo through the game's display and 1.45 times through a camera's at sunny 16 (EV 15 at ISO 100; middle grey at
/// about 4.1 kcd/m² where the game puts it at 2.3: `native/docs/lighting/daylight.md`, "Sun and sky at the sea"). Since
/// 2026-10-20 the game draws the ships' catalogue paints at their measured reflectance (`naval_ships::ShipPreset::
/// paint_gains`: each swatch over the catalogue's `albedoScale`, 0.54, which this ratio had cancelled), and the camera's
/// exposure enters through the eye, a camera's meter over sunny 16 (`crate::post::METER_REFERENCE`); the fixed
/// exposure stays the game's, where the eye rests when it does not meter (night, eye adaptation off, under water, a
/// view brighter than a meter's middle grey).
pub const CAMERA_EXPOSURE: f32 = 1.45 / 2.7;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DisplayUniforms {
    exposure: f32,
    tone_mapping_exposure: f32,
    bloom: f32,
    /// The look and photo mode's roll (`crate::photo::display_finish`): look, halation, diffusion, the preview's
    /// scale (scene pixels a pixel drawn, 0 for the frame itself), roll, roll scale.
    finish: [f32; 6],
    /// The output's brightest over SDR white (1 for SDR).
    headroom: f32,
    /// Photo mode's lens (`crate::photo_optics::display_lens`): its distortion's `k` and its chromatic aberration's
    /// shift at the corners.
    lens: [f32; 2],
    /// Night readability: enabled (x), camera near/far planes (yz), and its weight by the host's sun (w,
    /// `post::night_weight`), or −1 to follow the clear sky's luminance.
    night_readability: [f32; 4],
    /// The dark-adapted eye's grey by the host's sun (x, `post::night_vision_weight`); yzw unused.
    night_vision: [f32; 4],
    /// The air's shimmer (`crate::shimmer::Shimmer`'s rows), zeros for none.
    shimmer: [[f32; 4]; 7],
}

/// What the display pass reads this frame. Each view comes with a version that
/// changes when it is remade, so the bind group is rebuilt only then.
pub(crate) struct DisplayInputs<'a> {
    /// Linear HDR radiance: `scene.hdr`.
    pub source: (&'a wgpu::TextureView, u64),
    /// Reversed scene depth: zero for sky, positive for the sea and solid geometry.
    pub depth: (&'a wgpu::TextureView, u64),
    pub clip_planes: [f32; 2],
    /// The bloom's composite, or a black texture without bloom.
    pub bloom: (&'a wgpu::TextureView, u64),
    pub bloom_on: bool,
    pub sampler: &'a wgpu::Sampler,
    /// The eye adaptation's state (`exposure.eye_state`, or a second view's own).
    pub eye: &'a wgpu::Buffer,
    /// Sky radiance (rgb) and clear-sky luminance (w), independent of the view being shown.
    pub sky_mean: &'a wgpu::Buffer,
    pub night_readability: bool,
    /// The night grade's weight by the host's sun (`crate::post::night_weight`); `None` follows the clear sky's
    /// luminance.
    pub night: Option<f32>,
    /// The dark-adapted eye's grey by the host's sun (`crate::post::night_vision_weight`); none without a sun.
    pub night_vision: f32,
    /// Metres under water where each pixel's view ray starts (r, positive under; the host's probe over the screen), or
    /// a black texture: what the night grade takes as water near the eye, whatever the depth holds.
    pub underwater: (&'a wgpu::TextureView, u64),
    /// The output's headroom over SDR white this frame (1 for an SDR output).
    pub headroom: f32,
    /// The dither's mask and phase.
    pub dither: &'a Dither,
    /// Photo mode's finish, if it is up (`crate::photo`).
    pub photo: Option<&'a crate::photo::PhotoPost>,
    /// The look shown: photo mode's while it is up, else the game's.
    pub look: crate::photo::Look,
    /// The film camera's colour fringes in play (`crate::play_camera::PlayCamera::fringe_shift`), when photo mode's
    /// lens is not drawing its own; 0 for none.
    pub fringes: f32,
    /// A Look tab's preview (`crate::look_preview`): scene pixels to each pixel drawn, the whole frame drawn small; 0
    /// for the frame itself.
    pub preview: f32,
    /// The air's shimmer through a long lens (`crate::shimmer`), if the view shows any.
    pub shimmer: Option<crate::shimmer::Shimmer>,
}

/// The cover's blending: the smaller of what it writes and what the scene holds (`min` takes no factors).
const HELD_UNDER: wgpu::BlendComponent = wgpu::BlendComponent {
    src_factor: wgpu::BlendFactor::One,
    dst_factor: wgpu::BlendFactor::One,
    operation: wgpu::BlendOperation::Min,
};

/// What the display's bind group was built for: the source, bloom, depth and underwater views' versions, and the eye.
type BuiltFor = ((u64, u64, u64, u64), wgpu::Buffer);

pub(crate) struct DisplayPass {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    pipelines: Vec<(wgpu::TextureFormat, PipelineId)>,
    /// Zeroes the scene's alpha ahead of the transparents, so it holds their cover (`transparent_cover.wgsl`).
    cover: Option<PipelineId>,
    /// Holds the scene's colour under the half floats' range before a scene copy (`transparent_cover.wgsl`).
    hold: Option<PipelineId>,
    /// Built for these views' versions and this eye (the main view's, or a second view's own).
    bind_group: Option<(BuiltFor, wgpu::BindGroup)>,
    pub(crate) settings: DisplaySettings,
}

impl DisplayPass {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let [mask, phase] = Dither::layout_entries();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("display"),
            entries: &[
                // Filtered by a look's diffusion (`diffused`); read texel by texel otherwise.
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                crate::copies::depth_copy_layout_entry(5, fragment),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: fragment,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                mask,
                phase,
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: fragment,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("display uniforms"),
            size: std::mem::size_of::<DisplayUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            uniforms,
            pipelines: Vec::new(),
            cover: None,
            hold: None,
            bind_group: None,
            settings: DisplaySettings::default(),
        }
    }

    /// The transparents' cover pipeline and the scene's hold: full-screen triangles into the scene's colour inside the
    /// scene pass, past every depth test, blended by `min` (`transparent_cover.wgsl`). The cover zeroes the alpha and
    /// holds the colour under the scene's half floats' range; the hold holds the colour alone.
    pub(crate) fn ensure_cover(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<(), ShaderError> {
        if self.cover.is_some() {
            return Ok(());
        }
        let mut add = |label: &str, write_mask: wgpu::ColorWrites| {
            pipelines.add(
                device,
                library,
                label,
                builtin::TRANSPARENT_COVER.path,
                Vec::new(),
                Box::new(move |build| {
                    let pipeline_layout =
                        build
                            .device
                            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                                label: Some(build.label),
                                bind_group_layouts: &[],
                                immediate_size: 0,
                            });
                    Pipeline::Render(build.device.create_render_pipeline(
                        &wgpu::RenderPipelineDescriptor {
                            label: Some(build.label),
                            layout: Some(&pipeline_layout),
                            vertex: wgpu::VertexState {
                                module: build.module,
                                entry_point: Some("vertex"),
                                compilation_options: Default::default(),
                                buffers: &[],
                            },
                            primitive: wgpu::PrimitiveState::default(),
                            depth_stencil: Some(wgpu::DepthStencilState {
                                format: crate::frame::DEPTH_FORMAT,
                                depth_write_enabled: Some(false),
                                depth_compare: Some(wgpu::CompareFunction::Always),
                                stencil: wgpu::StencilState::default(),
                                bias: wgpu::DepthBiasState::default(),
                            }),
                            multisample: wgpu::MultisampleState::default(),
                            fragment: Some(wgpu::FragmentState {
                                module: build.module,
                                entry_point: Some("fragment"),
                                compilation_options: Default::default(),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format: crate::frame::HDR_FORMAT,
                                    blend: Some(wgpu::BlendState {
                                        color: HELD_UNDER,
                                        alpha: HELD_UNDER,
                                    }),
                                    write_mask,
                                })],
                            }),
                            multiview_mask: None,
                            cache: build.cache,
                        },
                    ))
                }),
            )
        };
        let cover = add("transparent cover", wgpu::ColorWrites::ALL)?;
        let hold = add("scene hold", wgpu::ColorWrites::COLOR)?;
        self.cover = Some(cover);
        self.hold = Some(hold);
        Ok(())
    }

    /// Zeroes the scene's alpha: the start of the scene pass's `Transparent` phase while the night is graded.
    pub(crate) fn draw_cover(&self, pass: &mut wgpu::RenderPass<'_>, pipelines: &PipelineRegistry) {
        if let Some(id) = self.cover {
            pass.set_pipeline(pipelines.render(id));
            pass.draw(0..3, 0..1);
        }
    }

    /// Holds the scene's light under the half floats' range, its alpha untouched: the end of a scene pass whose copy
    /// (the opaque scene, the backdrop) other layers read, so they read what the frame holds.
    pub(crate) fn draw_hold(&self, pass: &mut wgpu::RenderPass<'_>, pipelines: &PipelineRegistry) {
        if let Some(id) = self.hold {
            pass.set_pipeline(pipelines.render(id));
            pass.draw(0..3, 0..1);
        }
    }

    /// The pipeline writing to `format`, built on first use. A format without
    /// hardware sRGB encoding gets three's transfer in the shader; an 8- or 10-bit
    /// one the output dither (`crate::dither`).
    pub(crate) fn ensure_format(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        format: wgpu::TextureFormat,
    ) -> Result<PipelineId, ShaderError> {
        if let Some((_, id)) = self.pipelines.iter().find(|(f, _)| *f == format) {
            return Ok(*id);
        }
        let mut defs = if format.is_srgb() {
            Vec::new()
        } else {
            vec![("ENCODE_SRGB".to_owned(), ShaderDefValue::Bool(true))]
        };
        defs.extend(dither_defs(format));
        let layout = self.layout.clone();
        let id = pipelines.add(
            device,
            library,
            // A portable label (`portable_label`: ASCII, no `: < > " / \ | ? *`):
            // wgpu's DX12 backend hands it to DXC as the shader's file name,
            // and DXC fails to read one Windows cannot open ("error reading
            // 'display → Rgba8UnormSrgb'"), so the pipeline is never built on
            // Windows. See native/docs/packaging.md.
            format!("display to {format:?}"),
            builtin::DISPLAY_PASS.path,
            defs,
            Box::new(move |build| {
                let pipeline_layout =
                    build
                        .device
                        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some(build.label),
                            bind_group_layouts: &[Some(&layout)],
                            immediate_size: 0,
                        });
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some("vertex"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some("fragment"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            }),
        )?;
        self.pipelines.push((format, id));
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut UploadSink<'_>,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        inputs: &DisplayInputs<'_>,
        output: &wgpu::TextureView,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        uploads.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&DisplayUniforms {
                exposure: self.settings.exposure
                    * inputs.photo.map_or(1.0, |photo| photo.ev.exp2()),
                tone_mapping_exposure: self.settings.tone_mapping_exposure,
                bloom: if inputs.bloom_on { 1.0 } else { 0.0 },
                finish: {
                    let mut finish = crate::photo::display_finish(inputs.look, inputs.photo);
                    finish[3] = inputs.preview;
                    finish
                },
                headroom: inputs.headroom.max(1.0),
                lens: match inputs.photo.and_then(|photo| photo.optics.as_ref()) {
                    Some(optics) => crate::photo_optics::display_lens(Some(optics)),
                    None => [0.0, inputs.fringes],
                },
                night_readability: [
                    f32::from(u8::from(inputs.night_readability)),
                    inputs.clip_planes[0],
                    inputs.clip_planes[1],
                    inputs.night.unwrap_or(-1.0),
                ],
                night_vision: [inputs.night_vision, 0.0, 0.0, 0.0],
                shimmer: crate::shimmer::Shimmer::rows(inputs.shimmer),
            }),
        );
        let versions = (
            inputs.source.1,
            inputs.bloom.1,
            inputs.depth.1,
            inputs.underwater.1,
        );
        if self
            .bind_group
            .as_ref()
            .is_none_or(|((built, eye), _)| *built != versions || eye != inputs.eye)
        {
            let [mask, phase] = inputs.dither.entries();
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("display"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(inputs.source.0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(inputs.bloom.0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: inputs.eye.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(inputs.depth.0),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: inputs.sky_mean.as_entire_binding(),
                    },
                    mask,
                    phase,
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(inputs.underwater.0),
                    },
                ],
            });
            self.bind_group = Some(((versions, inputs.eye.clone()), group));
        }
        let (_, group) = self.bind_group.as_ref().expect("just made");
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("display"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// A CPU twin of `shaders/naval/display.wgsl`, transcribed from the same
/// three.js and `DisplayTransform.ts` sources, for tests and for tools that
/// need to know what a radiance will look like.
pub mod reference {
    type Vec3 = [f32; 3];
    type Mat3 = [Vec3; 3]; // columns, as three and WGSL build them

    const LOOK_EXPOSURE: f32 = 0.9;
    const LOOK_CONTRAST: f32 = 1.25;
    const LOOK_SATURATION: f32 = 1.0;
    const MIDDLE_GRAY: f32 = 0.18;

    const LINEAR_REC2020_TO_LINEAR_SRGB: Mat3 = [
        [1.6605, -0.1246, -0.0182],
        [-0.5876, 1.1329, -0.1006],
        [-0.0728, -0.0083, 1.1187],
    ];
    const LINEAR_SRGB_TO_LINEAR_REC2020: Mat3 = [
        [0.6274, 0.0691, 0.0164],
        [0.3293, 0.9195, 0.0880],
        [0.0433, 0.0113, 0.8956],
    ];
    const AGX_INSET: Mat3 = [
        [0.856_627_2, 0.137_318_98, 0.111_898_21],
        [0.095_121_24, 0.761_242, 0.076_799_42],
        [0.048_251_606, 0.101_439_04, 0.811_302_4],
    ];
    const AGX_OUTSET: Mat3 = [
        [1.127_100_6, -0.141_329_76, -0.141_329_76],
        [-0.110_606_64, 1.157_823_7, -0.110_606_64],
        [-0.016_493_939, -0.016_493_939, 1.251_936_4],
    ];
    const AGX_MIN_EV: f32 = -12.47393;
    const AGX_MAX_EV: f32 = 4.026069;

    fn mul(m: &Mat3, v: Vec3) -> Vec3 {
        std::array::from_fn(|row| m[0][row] * v[0] + m[1][row] * v[1] + m[2][row] * v[2])
    }

    fn map(v: Vec3, f: impl Fn(f32) -> f32) -> Vec3 {
        v.map(f)
    }

    /// `displayGrade(radiance, exposure)`.
    pub fn display_grade(radiance: Vec3, exposure: f32) -> Vec3 {
        let exposed = map(radiance, |c| c * (exposure * LOOK_EXPOSURE));
        let lum = (0.2126 * exposed[0] + 0.7152 * exposed[1] + 0.0722 * exposed[2]).max(1e-5);
        let contrast = (lum / MIDDLE_GRAY).powf(LOOK_CONTRAST - 1.0);
        map(exposed, |c| {
            (lum + (c - lum) * LOOK_SATURATION).max(0.0) * contrast
        })
    }

    /// three's `agxDefaultContrastApprox`.
    fn sigmoid(x: f32) -> f32 {
        let x2 = x * x;
        let x4 = x2 * x2;
        15.5 * (x4 * x2) - 40.14 * (x4 * x)
            + (31.96 * x4 - 6.868 * (x2 * x) + (0.4298 * x2 + (0.1191 * x - 0.00232)))
    }

    /// `agx_log_open`: AgX up to its sigmoid, not clamped.
    fn agx_log_open(color: Vec3, exposure: f32) -> Vec3 {
        let mut tone = map(color, |c| c * exposure);
        tone = mul(&LINEAR_SRGB_TO_LINEAR_REC2020, tone);
        tone = mul(&AGX_INSET, tone);
        tone = map(tone, |c| c.max(1e-10).log2());
        map(tone, |c| (c - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV))
    }

    /// `agx_log`: AgX up to its sigmoid.
    fn agx_log(color: Vec3, exposure: f32) -> Vec3 {
        map(agx_log_open(color, exposure), |c| c.clamp(0.0, 1.0))
    }

    /// `agx_display`: AgX after its sigmoid, not yet clamped.
    fn agx_display(sigmoid: Vec3) -> Vec3 {
        let mut tone = mul(&AGX_OUTSET, sigmoid);
        tone = map(tone, |c| c.max(0.0).powf(2.2));
        mul(&LINEAR_REC2020_TO_LINEAR_SRGB, tone)
    }

    /// three's `agxToneMapping(color, exposure)`.
    pub fn agx_tone_mapping(color: Vec3, exposure: f32) -> Vec3 {
        let sigmoid = map(agx_log(color, exposure), sigmoid);
        map(agx_display(sigmoid), |c| c.clamp(0.0, 1.0))
    }

    /// `AGX_TOP`, `HDR_KNEE` and `HDR_CURVATURE` in `display.wgsl`.
    pub const AGX_TOP: f32 = 0.99858;
    pub const HDR_KNEE: f32 = 0.7560;
    pub const HDR_CURVATURE: f32 = 20.0;

    /// `agx_hdr_tone_mapping(color, exposure, headroom)`: AgX with its top raised to `headroom`.
    pub fn agx_hdr_tone_mapping(color: Vec3, exposure: f32, headroom: f32) -> Vec3 {
        let range = headroom.max(1.0);
        let lift = AGX_TOP * (range.powf(1.0 / 2.2) - 1.0);
        let span = (lift / HDR_CURVATURE).sqrt().max(1e-4);
        let sigmoid = map(agx_log_open(color, exposure), |t| {
            let u = ((t - HDR_KNEE) / span).clamp(0.0, 1.0);
            sigmoid(t.clamp(0.0, 1.0)) + lift * u * u * (3.0 - 2.0 * u)
        });
        map(agx_display(sigmoid), |c| c.clamp(0.0, range))
    }

    /// three's sRGB transfer (carried on above 1, as an HDR output's encoding is).
    pub fn srgb_encode(linear: f32) -> f32 {
        if linear <= 0.003_130_8 {
            linear * 12.92
        } else {
            linear.max(0.0).powf(0.41666) * 1.055 - 0.055
        }
    }

    /// three's sRGB transfer, to 8 bits.
    pub fn srgb8(linear: f32) -> u8 {
        (srgb_encode(linear).clamp(0.0, 1.0) * 255.0).round() as u8
    }

    /// What the display pass writes for `radiance` under `settings`, as 8-bit sRGB.
    pub fn display_srgb8(radiance: Vec3, settings: super::DisplaySettings) -> [u8; 3] {
        agx_tone_mapping(
            display_grade(radiance, settings.exposure),
            settings.tone_mapping_exposure,
        )
        .map(srgb8)
    }
}

#[cfg(test)]
mod tests {
    use super::reference::*;

    /// Neutral greys from a sixteenth of middle grey to far past AgX's top, a twentieth of a stop apart.
    fn greys() -> impl Iterator<Item = f32> {
        (-200..240).map(|step| 0.18 * 2f32.powf(step as f32 / 20.0))
    }

    fn luminance(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn at_a_headroom_of_one_hdr_is_agx() {
        for v in greys() {
            for colour in [[v; 3], [v, v * 0.5, v * 0.1], [v * 0.2, v * 0.6, v]] {
                assert_eq!(
                    agx_hdr_tone_mapping(colour, 1.0, 1.0),
                    agx_tone_mapping(colour, 1.0),
                    "{colour:?}"
                );
            }
        }
    }

    #[test]
    fn below_the_knee_every_headroom_shows_agx() {
        // A graded 1 is the knee; just under it nothing moves.
        for v in greys().filter(|v| *v < 0.98) {
            let sdr = agx_tone_mapping([v; 3], 1.0);
            for headroom in [1.5, 4.0, 16.0] {
                let hdr = agx_hdr_tone_mapping([v; 3], 1.0, headroom);
                for c in 0..3 {
                    assert!((hdr[c] - sdr[c]).abs() < 1e-6, "{v}: {hdr:?} vs {sdr:?}");
                }
            }
        }
    }

    #[test]
    fn highlights_rise_steadily_and_more_headroom_never_dims_one() {
        let headrooms = [1.0, 1.05, 1.5, 2.0, 4.0, 8.0, 16.0];
        let mut last: Vec<f32> = vec![0.0; headrooms.len()];
        for v in greys() {
            let shown: Vec<f32> = headrooms
                .iter()
                .map(|&h| luminance(agx_hdr_tone_mapping([v; 3], 1.0, h)))
                .collect();
            for (i, (&now, &before)) in shown.iter().zip(&last).enumerate() {
                assert!(
                    now >= before - 1e-6,
                    "{v} at {}: {now} < {before}",
                    headrooms[i]
                );
            }
            for pair in shown.windows(2) {
                assert!(pair[1] >= pair[0] - 1e-6, "{v}: {shown:?}");
            }
            last = shown;
        }
    }

    #[test]
    fn the_brightest_highlights_reach_the_headroom() {
        for headroom in [2.0f32, 4.0, 16.0] {
            let top = luminance(agx_hdr_tone_mapping([1e4; 3], 1.0, headroom));
            // AgX's top is 0.9969 of white: the headroom's share of it.
            assert!((top / headroom - 0.9969).abs() < 2e-3, "{headroom}: {top}");
        }
        // A graded 11 (sun glitter, flashes) is within a fifth of a stop of it at a headroom of 4.
        let glitter = luminance(agx_hdr_tone_mapping([11.0; 3], 1.0, 4.0));
        assert!(glitter > 4.0 * 0.87, "{glitter}");
        // SDR's top stays SDR's.
        assert!(luminance(agx_hdr_tone_mapping([1e4; 3], 1.0, 1.0)) <= 1.0);
    }
}
