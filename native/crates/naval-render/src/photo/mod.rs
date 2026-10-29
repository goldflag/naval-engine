//! Photo mode's post (`native/docs/photo.md`): a thin lens's depth of field on the scene's radiance, and the finish the
//! display pass and the film pass give the frame (exposure compensation, a look, film grain, a vignette, the camera's
//! roll).
//!
//! - **Depth of field** ([`DepthOfField`]): the circle of confusion of a lens of `focal_mm` at f/`aperture` focused at
//!   `focus_m`, on a sensor `sensor_mm` tall (a full-frame 36 × 24 mm camera's 24, more when the roll widens the frame
//!   drawn), mapped to the frame's pixels. Four passes after the eye adaptation and the bloom: a prefilter to half
//!   resolution (the radiance and each pixel's signed circle, its nearest depth's), the largest circle in each 8 × 8
//!   tile, a gather at half resolution and a composite at full. The gather is Dennis Gustafsson's single-pass bokeh
//!   ("Bokeh depth of field in a single pass", 2018): a spiral of samples out to the largest circle in reach, each
//!   counted where its own circle covers the pixel, a sample behind the pixel no larger than twice the pixel's own (so
//!   a sharp ship's edge never takes the blurred sea behind her), its spiral's spacing grown so no pixel takes more
//!   than [`GATHER_SAMPLES`]. The composite keeps the full-resolution pixel where both its own circle and the near
//!   blur reaching it are under a pixel. Off, nothing is drawn or allocated.
//! - **The shutter** ([`crate::shutter`]): the scene blurred by the frozen moment's velocities, in place, before the
//!   eye, the bloom and the depth of field read it.
//! - **The finish** (in the display pass, `shaders/display_pass.wgsl`): `ev` stops over the eye's exposure (the bloom
//!   takes them too); a look graded on the display-referred frame (photo mode's replaces the game's, [`Look`]); and the
//!   roll: the frame drawn turned about its centre and enlarged by [`roll_scale`], whose field of view the host widens
//!   by the same factor so the photo keeps its focal length. Grain (fixed per seed, so a still frame keeps it) and the
//!   vignette, each the larger of the photographer's and the look's, go on in the film pass ([`crate::film`]) with
//!   the rest of the look's film, held at the seed's frame.

pub mod film;
pub mod look_preview;
pub mod photo_optics;
pub mod play_camera;
pub mod shutter;

use bytemuck::{Pod, Zeroable};

use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::{compute_recipe, fullscreen_recipe};
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;

pub const PHOTO_DOF_SHADER: ShaderFile = crate::shader_file!("shaders/post/photo_dof.wgsl");
pub const PHOTO_PROBE_SHADER: ShaderFile = crate::shader_file!("shaders/post/photo_probe.wgsl");

/// The depth of field's targets' format.
const DOF_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Half-resolution pixels a side of the gather's tiles.
const TILE: u32 = 8;
/// The most samples one pixel's gather takes.
pub const GATHER_SAMPLES: f32 = 160.0;
/// The largest circle of confusion drawn: its radius over the frame's height.
pub const MAX_RADIUS_OF_HEIGHT: f32 = 0.04;

/// A look: a colour grade on the display-referred frame, and the film it comes with ([`Look::film`]). The game's
/// (the Graphics menu's Look, [`crate::Renderer::set_look`]) or photo mode's, which replaces it while photo mode is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    /// As the game draws it.
    #[default]
    Natural,
    /// Black and white, a firmer contrast, printed on warm paper.
    Press,
    /// Rich, saturated and warm.
    Kodachrome,
    /// Hard black and white, darker.
    Newsreel,
    /// A blue-green low-light glass.
    Night,
    /// Low contrast and saturation, lifted blacks.
    Faded,
    /// Three-strip Technicolor: each record printed in its own dye; dense primaries, clean whites, greys kept grey.
    Technicolor,
    /// Agfacolor: softer and cooler, olive shadows, brick reds.
    Agfacolor,
    /// An autochrome plate: pastel, dim and warm, seen through dyed starch grains.
    Autochrome,
    /// Bleach bypass: silver left in the print, half the colour, hard contrast.
    BleachBypass,
    /// Black-and-white infrared: the sea and the blue sky dark, leaves and clouds white.
    Infrared,
    /// Colour infrared: leaves red, the sea deep blue, red paint yellow.
    Aerochrome,
    /// A fighter's 16 mm gun camera: hard black and white, soft, shaken and scratched.
    GunCamera,
}

/// What a look's stock and projector add to its grade, each 0–1 (`native/docs/photo.md`, "Looks"). Halation and
/// diffusion go on in the display pass; the rest in the film pass after edge smoothing ([`crate::film`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Film {
    /// A red glow around the brightest lights: the bloom's blur, tinted.
    pub halation: f32,
    /// A soft lens's glow, the frame blurred and lightened in.
    pub diffusion: f32,
    pub grain: f32,
    pub vignette: f32,
    /// The frame shaking in the gate.
    pub weave: f32,
    /// The projector's flicker.
    pub flicker: f32,
    /// Dust and scratches.
    pub dust: f32,
    /// An autochrome's dyed starch grains.
    pub mosaic: f32,
}

impl Look {
    pub const ALL: [Look; 13] = [
        Look::Natural,
        Look::Press,
        Look::Kodachrome,
        Look::Newsreel,
        Look::Night,
        Look::Faded,
        Look::Technicolor,
        Look::Agfacolor,
        Look::Autochrome,
        Look::BleachBypass,
        Look::Infrared,
        Look::Aerochrome,
        Look::GunCamera,
    ];

    /// The grade's case in `photo_look` (`shaders/display_pass.wgsl`).
    fn index(self) -> f32 {
        match self {
            Look::Natural => 0.0,
            Look::Press => 1.0,
            Look::Kodachrome => 2.0,
            Look::Newsreel => 3.0,
            Look::Night => 4.0,
            Look::Faded => 5.0,
            Look::Technicolor => 6.0,
            Look::Agfacolor => 7.0,
            Look::Autochrome => 8.0,
            Look::BleachBypass => 9.0,
            Look::Infrared => 10.0,
            Look::Aerochrome => 11.0,
            Look::GunCamera => 12.0,
        }
    }

    /// The film the look comes with (tuned in the 2026-10-09 film looks study on the game's frames).
    pub fn film(self) -> Film {
        let film = |halation, diffusion, grain, vignette| Film {
            halation,
            diffusion,
            grain,
            vignette,
            ..Film::default()
        };
        match self {
            Look::Natural => Film::default(),
            Look::Press => film(0.0, 0.0, 0.2, 0.15),
            Look::Kodachrome => film(0.15, 0.0, 0.15, 0.0),
            Look::Newsreel => Film {
                weave: 0.4,
                flicker: 0.4,
                dust: 0.3,
                ..film(0.0, 0.15, 0.5, 0.35)
            },
            Look::Night => film(0.0, 0.0, 0.35, 0.6),
            Look::Faded => Film {
                dust: 0.15,
                ..film(0.1, 0.0, 0.25, 0.0)
            },
            Look::Technicolor => film(0.3, 0.1, 0.15, 0.15),
            Look::Agfacolor => film(0.2, 0.1, 0.25, 0.15),
            Look::Autochrome => Film {
                mosaic: 1.0,
                ..film(0.0, 0.45, 0.0, 0.5)
            },
            Look::BleachBypass => film(0.05, 0.0, 0.35, 0.2),
            Look::Infrared => film(0.6, 0.3, 0.55, 0.25),
            Look::Aerochrome => film(0.1, 0.0, 0.25, 0.0),
            Look::GunCamera => Film {
                weave: 0.7,
                flicker: 0.6,
                dust: 0.5,
                ..film(0.0, 0.45, 0.7, 0.75)
            },
        }
    }
}

/// A thin lens's depth of field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthOfField {
    /// Metres to the plane in focus.
    pub focus_m: f32,
    /// The f-number.
    pub aperture: f32,
    /// The focal length, millimetres.
    pub focal_mm: f32,
    /// The height of the sensor the drawn frame spans, millimetres (24 on a full-frame camera; times [`roll_scale`]
    /// while the roll widens the frame drawn).
    pub sensor_mm: f32,
}

impl DepthOfField {
    /// The circle of confusion's diameter, in pixels of a frame `height` pixels tall, of a point at infinity: a point
    /// `z` metres away has `scale × (1 − focus / z)`, negative nearer than the focus.
    pub fn coc_scale(&self, height: f32) -> f32 {
        let f = (self.focal_mm.max(1.0)) * 1e-3;
        let focus = self.focus_m.max(2.0 * f);
        let n = self.aperture.max(0.5);
        let sensor = (self.sensor_mm.max(1.0)) * 1e-3;
        f * f / (n * (focus - f)) / sensor * height
    }

    /// The signed diameter, in pixels of a frame `height` tall, of the circle a point `z` metres away blurs to.
    pub fn circle_of_confusion(&self, z: f32, height: f32) -> f32 {
        self.coc_scale(height) * (1.0 - self.focus_m.max(1e-3) / z.max(1e-3))
    }
}

/// Photo mode's post for the renderer ([`crate::Renderer::set_photo`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhotoPost {
    /// Stops over the eye's exposure.
    pub ev: f32,
    /// Replaces the game's look while photo mode is up.
    pub look: Look,
    /// 0–1; the look's own grain when that is more.
    pub grain: f32,
    /// 0–1; the look's own vignette when that is more.
    pub vignette: f32,
    /// The grain's pattern, and the frame the look's film is held at (its dust, scratches, flicker and shake).
    pub grain_seed: f32,
    /// The camera's roll, radians, positive clockwise: the picture turns the other way.
    pub roll: f32,
    /// The frame drawn over the photo's ([`roll_scale`]; 1 without a roll; times [`crate::photo_optics::distortion_cover`]
    /// with a barrel's distortion).
    pub roll_scale: f32,
    pub dof: Option<DepthOfField>,
    /// The lens's character and its flare (`crate::photo_optics`); none draws nothing of it.
    pub optics: Option<crate::photo_optics::Optics>,
    /// The shutter's motion blur (`crate::shutter`); `None` is the frozen instant. The moving bodies go to
    /// [`crate::Renderer::set_photo_bodies`].
    pub shutter: Option<crate::shutter::Shutter>,
}

impl Default for PhotoPost {
    fn default() -> Self {
        Self {
            ev: 0.0,
            look: Look::Natural,
            grain: 0.0,
            vignette: 0.0,
            grain_seed: 0.0,
            roll: 0.0,
            roll_scale: 1.0,
            dof: None,
            optics: None,
            shutter: None,
        }
    }
}

/// How much larger than the photo the frame drawn must be for the photo, turned `roll` radians, to fit inside it: the
/// field of view widens by this, and the display enlarges the turned frame by it.
pub fn roll_scale(roll: f32, aspect: f32) -> f32 {
    let (sin, cos) = roll.abs().sin_cos();
    let wide = aspect.max(1.0 / aspect.max(1e-3));
    cos + wide * sin
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DofUniforms {
    focus: f32,
    coc_scale: f32,
    near: f32,
    far: f32,
    max_radius: f32,
    max_samples: f32,
    full_size: [f32; 2],
    half_size: [f32; 2],
    padding: [f32; 2],
    /// The aperture's shape ([`crate::photo_optics::aperture_shape`]): blades (0 round), the blades' angle in the
    /// frame drawn, the polygon's share, 0.
    aperture: [f32; 4],
}

struct Target {
    view: wgpu::TextureView,
    version: u64,
}

struct DofTargets {
    viewport: (u32, u32),
    half: Target,
    tiles: Target,
    blur: Target,
    out: Target,
    /// The bind groups, per pass, and the scene views' versions they were built on.
    groups: Option<((u64, u64), [wgpu::BindGroup; 4])>,
}

/// The depth of field's passes and targets.
pub(crate) struct DepthOfFieldPass {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    prefilter: PipelineId,
    tiles: PipelineId,
    gather: PipelineId,
    composite: PipelineId,
    targets: Option<DofTargets>,
    next_version: u64,
    /// The focus probe: one texel of the scene's depth into a small buffer.
    probe: PipelineId,
    probe_layout: wgpu::BindGroupLayout,
    probe_at: wgpu::Buffer,
    probe_out: wgpu::Buffer,
}

impl DepthOfFieldPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(PHOTO_DOF_SHADER)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo dof"),
            entries: &[
                texture(0),
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
                texture(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: fragment,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let mut add = |label: &str, entry: &'static str| {
            pipelines.add(
                device,
                library,
                label,
                PHOTO_DOF_SHADER.path,
                Vec::new(),
                fullscreen_recipe(vec![layout.clone()], "vertex", entry, DOF_FORMAT, None),
            )
        };
        let prefilter = add("photo dof prefilter", "prefilter")?;
        let tiles = add("photo dof tiles", "tiles")?;
        let gather = add("photo dof gather", "gather")?;
        let composite = add("photo dof composite", "composite")?;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo dof uniforms"),
            size: std::mem::size_of::<DofUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        library.add(PHOTO_PROBE_SHADER)?;
        let probe_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("photo probe"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let probe = pipelines.add(
            device,
            library,
            "photo focus probe",
            PHOTO_PROBE_SHADER.path,
            Vec::new(),
            compute_recipe(vec![probe_layout.clone()], "probe"),
        )?;
        let probe_at = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo probe at"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let probe_out = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo probe out"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("photo dof"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layout,
            uniforms,
            sampler,
            prefilter,
            tiles,
            gather,
            composite,
            targets: None,
            // Apart from the frame's resources' versions: the display caches its bind group by its source's.
            next_version: 1 << 40,
            probe,
            probe_layout,
            probe_at,
            probe_out,
        })
    }

    /// The reversed depth at texel `at` of `depth`, copied into a new read-back buffer (16 bytes, the depth first) in
    /// a submission of its own: the caller maps it.
    pub(crate) fn probe_depth(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pipelines: &PipelineRegistry,
        depth: &wgpu::TextureView,
        at: (u32, u32),
    ) -> wgpu::Buffer {
        queue.write_buffer(&self.probe_at, 0, bytemuck::cast_slice(&[at.0, at.1, 0, 0]));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("photo probe"),
            layout: &self.probe_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.probe_at.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.probe_out.as_entire_binding(),
                },
            ],
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("photo probe read-back"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("photo focus probe"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("photo focus probe"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipelines.compute(self.probe));
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&self.probe_out, 0, &readback, 0, 16);
        queue.submit([encoder.finish()]);
        readback
    }

    /// Lets the targets go (photo mode left, or its depth of field turned off).
    pub(crate) fn release(&mut self) {
        self.targets = None;
    }

    fn target(&mut self, device: &wgpu::Device, label: &str, size: (u32, u32)) -> Target {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DOF_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.next_version += 1;
        Target {
            view: texture.create_view(&Default::default()),
            version: self.next_version,
        }
    }

    fn ensure(&mut self, device: &wgpu::Device, viewport: (u32, u32)) {
        if self
            .targets
            .as_ref()
            .is_some_and(|targets| targets.viewport == viewport)
        {
            return;
        }
        let half = (viewport.0.div_ceil(2), viewport.1.div_ceil(2));
        let tiles = (half.0.div_ceil(TILE), half.1.div_ceil(TILE));
        let targets = DofTargets {
            viewport,
            half: self.target(device, "photo dof half", half),
            tiles: self.target(device, "photo dof tiles", tiles),
            blur: self.target(device, "photo dof blur", half),
            out: self.target(device, "photo dof out", viewport),
            groups: None,
        };
        self.targets = Some(targets);
    }

    /// Draws the depth of field of `scene` (`scene.hdr` and `scene.depth` with their versions) under a camera with
    /// planes `near` and `far`; returns the result's view and version for the display.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        timer: &GpuTimer,
        scene: ((&wgpu::TextureView, u64), (&wgpu::TextureView, u64)),
        viewport: (u32, u32),
        planes: (f32, f32),
        dof: &DepthOfField,
        aperture: [f32; 4],
    ) -> (&wgpu::TextureView, u64) {
        self.ensure(device, viewport);
        let ((hdr, hdr_version), (depth, depth_version)) = scene;
        let height = viewport.1.max(1) as f32;
        let half = (viewport.0.div_ceil(2), viewport.1.div_ceil(2));
        let uniforms = DofUniforms {
            focus: dof.focus_m.max(1e-3),
            coc_scale: dof.coc_scale(height),
            near: planes.0,
            far: planes.1,
            max_radius: MAX_RADIUS_OF_HEIGHT * height * 0.5,
            max_samples: GATHER_SAMPLES,
            full_size: [viewport.0 as f32, viewport.1 as f32],
            half_size: [half.0 as f32, half.1 as f32],
            padding: [0.0; 2],
            aperture,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let targets = self.targets.as_mut().expect("just made");
        let versions = (hdr_version, depth_version);
        if targets
            .groups
            .as_ref()
            .is_none_or(|(built, _)| *built != versions)
        {
            let group = |source: &wgpu::TextureView, aux: &wgpu::TextureView| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("photo dof"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(depth),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: self.uniforms.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::TextureView(aux),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                    ],
                })
            };
            // Each pass reads what it does not write: the aux binding of the passes without one is another target.
            let groups = [
                group(hdr, &targets.tiles.view),
                group(&targets.half.view, &targets.blur.view),
                group(&targets.half.view, &targets.tiles.view),
                group(hdr, &targets.blur.view),
            ];
            targets.groups = Some((versions, groups));
        }
        let (_, groups) = targets.groups.as_ref().expect("just made");
        let passes = [
            (self.prefilter, &targets.half.view, "photo dof prefilter"),
            (self.tiles, &targets.tiles.view, "photo dof tiles"),
            (self.gather, &targets.blur.view, "photo dof gather"),
            (self.composite, &targets.out.view, "photo dof composite"),
        ];
        // One timestamp pair for the four.
        let chain = timer.render_pass("photo dof");
        let last = passes.len() - 1;
        for (index, ((pipeline, target, label), group)) in
            passes.into_iter().zip(groups).enumerate()
        {
            let timestamp_writes = chain
                .as_ref()
                .map(|writes| wgpu::RenderPassTimestampWrites {
                    query_set: writes.query_set,
                    beginning_of_pass_write_index: if index == 0 {
                        writes.beginning_of_pass_write_index
                    } else {
                        None
                    },
                    end_of_pass_write_index: if index == last {
                        writes.end_of_pass_write_index
                    } else {
                        None
                    },
                });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timestamp_writes.filter(|writes| {
                    writes.beginning_of_pass_write_index.is_some()
                        || writes.end_of_pass_write_index.is_some()
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(pipeline));
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        (&targets.out.view, targets.out.version)
    }
}

/// The finish's part of the display pass's uniforms (`DisplayParams` after `bloom`): the look shown (photo mode's
/// while it is up, else the game's), its halation and diffusion, and the camera's roll.
pub(crate) fn display_finish(look: Look, photo: Option<&PhotoPost>) -> [f32; 6] {
    let film = look.film();
    let (roll, roll_scale) = photo.map_or((0.0, 1.0), |p| (p.roll, p.roll_scale.max(1.0)));
    [
        look.index(),
        film.halation,
        film.diffusion,
        0.0,
        roll,
        roll_scale,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(focal_mm: f32, aperture: f32, focus_m: f32) -> DepthOfField {
        DepthOfField {
            focus_m,
            aperture,
            focal_mm,
            sensor_mm: 24.0,
        }
    }

    #[test]
    fn the_circle_of_confusion_is_a_thin_lens_on_a_full_frame_sensor() {
        // 50 mm at f/2 focused at 5 m: a point at infinity blurs to f² / (N (S − f)) = 0.2525 mm on the sensor, 1.05 %
        // of its 24 mm height.
        let dof = lens(50.0, 2.0, 5.0);
        let height = 1000.0;
        assert!((dof.coc_scale(height) - 0.0025 / (2.0 * 4.95) / 0.024 * 1000.0).abs() < 1e-3);
        assert!(
            dof.circle_of_confusion(5.0, height).abs() < 1e-4,
            "the focus is sharp"
        );
        assert!(
            dof.circle_of_confusion(2.5, height) < 0.0,
            "nearer is negative"
        );
        assert!((dof.circle_of_confusion(2.5, height) + dof.coc_scale(height)).abs() < 1e-3);
        // At the sea's distances a normal lens holds everything sharp; a long one wide open does not.
        assert!(
            lens(35.0, 4.0, 800.0)
                .circle_of_confusion(20_000.0, 1440.0)
                .abs()
                < 0.1
        );
        assert!(lens(300.0, 2.8, 60.0).circle_of_confusion(5_000.0, 1440.0) > 20.0);
        // Stopping down shrinks it in proportion.
        let open = lens(85.0, 1.4, 30.0).coc_scale(height);
        let shut = lens(85.0, 14.0, 30.0).coc_scale(height);
        assert!((open / shut - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_rolled_photo_fits_inside_the_frame_drawn() {
        assert_eq!(roll_scale(0.0, 16.0 / 9.0), 1.0);
        let roll = 15f32.to_radians();
        let aspect = 16.0 / 9.0;
        let s = roll_scale(roll, aspect);
        // The photo's corners, turned and shrunk by the scale, stay within the frame.
        let (w, h) = (aspect, 1.0);
        for (x, y) in [(w, h), (w, -h), (-w, h), (-w, -h)] {
            let (sin, cos) = roll.sin_cos();
            let (u, v) = ((x * cos - y * sin) / s, (x * sin + y * cos) / s);
            assert!(u.abs() <= w + 1e-5 && v.abs() <= h + 1e-5, "{u} {v}");
        }
        assert_eq!(roll_scale(-roll, aspect), s);
    }
}
