//! The film pass (`native/docs/photo.md`, "Looks"): what a look's stock and projector add to the finished frame. That is
//! grain, a vignette, the frame shaking in the gate, the projector's flicker, dust and scratches, and an autochrome's
//! dyed starch grains. It draws last, after edge smoothing and the upscale and at the output's resolution, so temporal
//! AA's history never holds the grain and the grain is the output's own pixels. A look's grade, halation and diffusion
//! stay in the display pass (`shaders/display_pass.wgsl`).
//!
//! With a film on, the chain's last stage writes [`FILM_FRAME`] (the output's size, display-encoded) and this pass puts
//! it on the output; with none, the chain is as it was: no pass, no target. In play the film runs at [`FILM_FPS`] of
//! the frame's clock; in photo mode it holds at the photo's seed, so a still keeps its grain, dust and scratches.

use bytemuck::{Pod, Zeroable};

use crate::dither::{Dither, dither_defs};
use crate::photo::{Look, PhotoPost};
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::fullscreen_recipe;
use crate::resources::{Lifetime, Resources, TextureDesc};
use crate::shaders::{ShaderDefValue, ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;
use crate::upload::UploadSink;

pub const FILM_SHADER: ShaderFile = crate::shader_file!("shaders/post/film.wgsl");
/// The chain's last frame while a film is on: the output's size, display-encoded.
pub const FILM_FRAME: &str = "film.frame";
/// Frames a second the film runs at in play: its grain, flicker, shake and dust change this often.
pub const FILM_FPS: f64 = 24.0;
/// Film frames before the count wraps, so the shader's hashes keep their precision.
const FILM_FRAMES: f64 = 1024.0;

/// The film pass's settings for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FilmFinish {
    pub grain: f32,
    pub vignette: f32,
    pub weave: f32,
    pub flicker: f32,
    pub dust: f32,
    pub mosaic: f32,
    /// The film's frame: its grain's pattern, its dust and its shake.
    pub frame: f32,
}

impl FilmFinish {
    /// `look`'s film (`photo`'s look replaces the game's while photo mode is up, so the caller passes that one), with
    /// the photographer's grain and vignette where they are more, at the frame's clock `seconds`.
    pub fn new(look: Look, photo: Option<&PhotoPost>, seconds: f64) -> Self {
        let film = look.film();
        let frame = match photo {
            Some(photo) => photo.grain_seed,
            None => (seconds.max(0.0) * FILM_FPS)
                .floor()
                .rem_euclid(FILM_FRAMES) as f32,
        };
        Self {
            grain: photo
                .map_or(0.0, |photo| photo.grain.clamp(0.0, 1.0))
                .max(film.grain),
            vignette: photo
                .map_or(0.0, |photo| photo.vignette.clamp(0.0, 1.0))
                .max(film.vignette),
            weave: film.weave,
            flicker: film.flicker,
            dust: film.dust,
            mosaic: film.mosaic,
            frame,
        }
    }

    /// Whether the pass draws: anything of the film is on.
    pub fn is_on(&self) -> bool {
        [
            self.grain,
            self.vignette,
            self.weave,
            self.flicker,
            self.dust,
            self.mosaic,
        ]
        .iter()
        .any(|v| *v > 0.0)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FilmUniforms {
    /// Grain, vignette, weave, flicker.
    a: [f32; 4],
    /// Dust, mosaic, the film's frame, padding.
    b: [f32; 4],
}

pub(crate) struct FilmPass {
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    sampler: wgpu::Sampler,
    pipelines: Vec<(wgpu::TextureFormat, PipelineId)>,
    group: Option<(u64, wgpu::BindGroup)>,
}

impl FilmPass {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
    ) -> Result<Self, ShaderError> {
        library.add(FILM_SHADER)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let [mask, phase] = Dither::layout_entries();
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("film"),
            entries: &[
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
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
                mask,
                phase,
            ],
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("film uniforms"),
            size: std::mem::size_of::<FilmUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("film"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            layout,
            uniforms,
            sampler,
            pipelines: Vec::new(),
            group: None,
        })
    }

    /// The pipeline writing to `format`, built on first use: decoded after for a target that encodes sRGB in
    /// hardware, dithered onto an 8- or 10-bit one, as the chain's other last stages.
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
            vec![("DECODE_SRGB".to_owned(), ShaderDefValue::Bool(true))]
        } else {
            Vec::new()
        };
        defs.extend(dither_defs(format));
        let id = pipelines.add(
            device,
            library,
            // A portable label (no `:` or `/`, see `portable_label`).
            format!("film to {format:?}"),
            FILM_SHADER.path,
            defs,
            fullscreen_recipe(
                vec![self.layout.clone()],
                "vertex",
                "fragment",
                format,
                None,
            ),
        )?;
        self.pipelines.push((format, id));
        Ok(id)
    }

    /// Registers [`FILM_FRAME`] at the output's size while a film is on, and drops it when none is, with the bind
    /// group holding its view, so its memory goes.
    pub(crate) fn ensure_target(
        &mut self,
        device: &wgpu::Device,
        resources: &mut Resources,
        output: (u32, u32),
        on: bool,
    ) {
        if !on {
            self.group = None;
            if resources.texture(FILM_FRAME).is_some() {
                resources.remove(FILM_FRAME);
            }
            return;
        }
        let output = (output.0.max(1), output.1.max(1));
        let fits = resources.texture(FILM_FRAME).is_some_and(|entry| {
            let size = entry.texture.size();
            (size.width, size.height) == output
        });
        if !fits {
            resources.create_texture(
                device,
                FILM_FRAME,
                TextureDesc::fixed(
                    crate::taa::FRAME_FORMAT,
                    wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    output.0,
                    output.1,
                    Lifetime::Persistent,
                ),
            );
        }
    }

    /// [`FILM_FRAME`] through `finish` onto `output`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut crate::upload::Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        resources: &Resources,
        timer: &GpuTimer,
        dither: &Dither,
        finish: &FilmFinish,
        output: &wgpu::TextureView,
    ) {
        self.write(&mut UploadSink::Belt(uploads), finish);
        let frame = resources
            .texture(FILM_FRAME)
            .expect("registered by ensure_target");
        if self
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != frame.version)
        {
            let group = self.bind(device, dither, &frame.view);
            self.group = Some((frame.version, group));
        }
        let (_, group) = self.group.as_ref().expect("made above");
        draw(encoder, pipeline, group, output, timer.render_pass("film"));
    }

    /// One of the Look tab's previews (`crate::look_preview`): `input`, a display-encoded tile, through `finish` onto
    /// `output`. It writes the pass's uniforms, so the caller submits it before the next is written.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_preview(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        dither: &Dither,
        finish: &FilmFinish,
        input: &wgpu::TextureView,
        output: &wgpu::TextureView,
    ) {
        self.write(&mut UploadSink::Queue(queue), finish);
        let group = self.bind(device, dither, input);
        draw(encoder, pipeline, &group, output, None);
    }

    fn write(&self, uploads: &mut UploadSink<'_>, finish: &FilmFinish) {
        uploads.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&FilmUniforms {
                a: [finish.grain, finish.vignette, finish.weave, finish.flicker],
                b: [finish.dust, finish.mosaic, finish.frame, 0.0],
            }),
        );
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        dither: &Dither,
        input: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let [mask, phase] = dither.entries();
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("film"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(input),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms.as_entire_binding(),
                },
                mask,
                phase,
            ],
        })
    }
}

fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    output: &wgpu::TextureView,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("film"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_has_no_film_and_the_rest_do() {
        assert!(!FilmFinish::new(Look::Natural, None, 3.0).is_on());
        for look in Look::ALL.into_iter().skip(1) {
            assert!(FilmFinish::new(look, None, 3.0).is_on(), "{look:?}");
        }
    }

    #[test]
    fn play_runs_at_24_frames_a_second_and_photo_mode_holds_its_seed() {
        let at = |seconds| FilmFinish::new(Look::GunCamera, None, seconds).frame;
        assert_eq!(at(0.0), 0.0);
        assert_eq!(at(1.0 / 48.0), 0.0);
        assert_eq!(at(1.0), 24.0);
        assert_eq!(at(FILM_FRAMES / FILM_FPS + 1.0), 24.0, "wraps");
        let photo = PhotoPost {
            look: Look::GunCamera,
            grain_seed: 7.0,
            ..Default::default()
        };
        for seconds in [0.0, 5.0, 100.0] {
            assert_eq!(
                FilmFinish::new(photo.look, Some(&photo), seconds).frame,
                7.0
            );
        }
    }

    #[test]
    fn the_photographers_grain_and_vignette_add_to_the_looks() {
        let look = Look::Press.film();
        let mut photo = PhotoPost {
            look: Look::Press,
            ..Default::default()
        };
        let film = FilmFinish::new(photo.look, Some(&photo), 0.0);
        assert_eq!((film.grain, film.vignette), (look.grain, look.vignette));
        photo.grain = 0.9;
        photo.vignette = 0.05;
        let film = FilmFinish::new(photo.look, Some(&photo), 0.0);
        assert_eq!((film.grain, film.vignette), (0.9, look.vignette));
        // Natural, the photographer's alone.
        photo.look = Look::Natural;
        let film = FilmFinish::new(photo.look, Some(&photo), 0.0);
        assert_eq!((film.grain, film.vignette), (0.9, 0.05));
        assert_eq!((film.weave, film.dust), (0.0, 0.0));
    }
}
