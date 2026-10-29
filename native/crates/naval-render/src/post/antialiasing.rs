//! The end of the post chain: edge smoothing and the render scale.
//!
//! The Graphics menu offers four kinds of edge smoothing (`antialiasing` in
//! `src/game/settings/graphicsSettings.ts`: off, FXAA, SMAA, TAA), all applied to the display frame (graded,
//! tone mapped and sRGB-encoded), and a render scale (`renderScale`, 50–100 %) that draws the whole
//! frame smaller and brings it up to the output's size. Here:
//!
//! - **[`Antialiasing::None`]** at full scale is the frame as before: the display pass writes the
//!   output directly.
//! - **[`Antialiasing::Fxaa`]**: the display pass writes `display.frame` and one pass of three's
//!   `FXAANode` (`shaders/post/fxaa.wgsl`) writes the output.
//! - **[`Antialiasing::Smaa`]**: three's `SMAANode` (`shaders/post/smaa.wgsl`), 1x Medium with colour
//!   edges: edges into `smaa.edges`, blending weights into `smaa.weights` (with three's area and
//!   search textures), then neighbourhood blending onto the output.
//! - **[`Antialiasing::Taa`]**: temporal AA's resolve (see [`crate::taa`]), then its output pass,
//!   which applies FXAA where the resolve took no history (its alpha ≤ 0: the sea, tracers,
//!   disocclusions). Both run at the output's size.
//!
//! **The output.** Whichever stage writes the output dithers onto an 8- or 10-bit one (`crate::dither`); with an HDR
//! output the frames hold values above SDR white, which the stages smooth in the perceptual domain
//! (`naval::output::perceptual`: FXAA's luminance, SMAA's edges, temporal AA's clipping) and write as they are.
//!
//! **Render scale.** Below 1 the scene, its copies, the post passes and every viewport texture are
//! drawn at the scaled size ([`scene_size`]; layers see it as `ctx.viewport`). With temporal AA the
//! resolve rebuilds the output's resolution from the smaller, jittered frames (temporal upscaling,
//! [`crate::taa`]); otherwise the chain's last stage writes `post.frame` at the scaled size and an
//! `upscale` pass stretches it bilinearly onto the output. Overlays drawn in
//! [`crate::SceneLayer::draw_overlay`] go on after that, at the output's full resolution.

use crate::dither::{Dither, dither_defs};
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::fullscreen_recipe;
use crate::resources::{Resources, TextureDesc};
use crate::shaders::{ShaderDefValue, ShaderError, ShaderFile, ShaderLibrary};
use crate::taa::FRAME_FORMAT;
use crate::timing::GpuTimer;

pub const FXAA_SHADER: ShaderFile = crate::shader_file!("shaders/post/fxaa.wgsl");
pub const SMAA_SHADER: ShaderFile = crate::shader_file!("shaders/post/smaa.wgsl");
pub const OUTPUT_SHADER: ShaderFile = crate::shader_file!("shaders/post/output.wgsl");

/// Registry name of the edge-smoothed frame at the scene's size, which the upscale stretches onto
/// the output (only below full render scale with edge smoothing on).
pub const POST_FRAME: &str = "post.frame";
/// Registry name of SMAA's edges (`Rg16Float`).
pub const SMAA_EDGES: &str = "smaa.edges";
/// Registry name of SMAA's blending weights (`Rgba16Float`).
pub const SMAA_WEIGHTS: &str = "smaa.weights";

/// The lowest scene scale: the Graphics menu's least render scale (`RENDER_SCALE_MIN` in `graphicsSettings.ts`, 50 %)
/// of a base the resolution cap shrinks ([`capped_scale`]), never below a quarter of the output.
pub const RENDER_SCALE_MIN: f32 = 0.25;

/// SMAA's area texture (160 × 560, RG) and search texture (66 × 33, R), the PNGs three r185 embeds
/// in `SMAANode.js`, decoded to raw channels and zlib-compressed (`assets/smaa-textures.ts`).
const SMAA_AREA: &[u8] = include_bytes!("../../assets/smaa_area_160x560_rg8.zlib");
const SMAA_SEARCH: &[u8] = include_bytes!("../../assets/smaa_search_66x33_r8.zlib");
const SMAA_AREA_SIZE: (u32, u32) = (160, 560);
const SMAA_SEARCH_SIZE: (u32, u32) = (66, 33);

/// Edge smoothing, by the Graphics setting's names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Antialiasing {
    /// None (the setting's `off`).
    #[default]
    None,
    /// Fast approximate AA on the display frame (the Low, Medium and High presets).
    Fxaa,
    /// Subpixel morphological AA, 1x Medium (the Ultra preset).
    Smaa,
    /// Temporal AA with FXAA where it takes no history.
    Taa,
}

impl Antialiasing {
    pub const ALL: [Antialiasing; 4] = [Self::None, Self::Fxaa, Self::Smaa, Self::Taa];

    /// `none` (or the setting's `off`), `fxaa`, `smaa` or `taa`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "none" | "off" => Self::None,
            "fxaa" => Self::Fxaa,
            "smaa" => Self::Smaa,
            "taa" => Self::Taa,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fxaa => "fxaa",
            Self::Smaa => "smaa",
            Self::Taa => "taa",
        }
    }
}

/// The scene's size for an output of `output` pixels at render scale `scale` (clamped to
/// [`RENDER_SCALE_MIN`]–1), rounded as three rounds a canvas's pixel size.
pub fn scene_size(output: (u32, u32), scale: f32) -> (u32, u32) {
    let scale = clamp_render_scale(scale);
    if scale >= 1.0 {
        return (output.0.max(1), output.1.max(1));
    }
    let side = |pixels: u32| ((pixels as f32 * scale).round() as u32).max(1);
    (side(output.0), side(output.1))
}

/// `scale` of a base no larger than `cap` pixels in `output`'s shape (the Graphics menu's resolution cap), in
/// [`RENDER_SCALE_MIN`]–1 of the output.
pub fn capped_scale(output: (u32, u32), scale: f32, cap: Option<u64>) -> f32 {
    let pixels = u64::from(output.0.max(1)) * u64::from(output.1.max(1));
    let base = cap.map_or(1.0, |cap| {
        ((cap as f64 / pixels as f64).sqrt()).min(1.0) as f32
    });
    clamp_render_scale(clamp_render_scale(scale) * base)
}

/// A render scale in [`RENDER_SCALE_MIN`]–1; anything else (NaN included) is 1.
pub fn clamp_render_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(RENDER_SCALE_MIN, 1.0)
    } else {
        1.0
    }
}

/// The last passes of the chain, each writing to a format of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputStage {
    Fxaa,
    Taa,
    SmaaBlend,
    Upscale,
}

impl OutputStage {
    fn label(self) -> &'static str {
        match self {
            Self::Fxaa => "fxaa",
            Self::Taa => "taa present",
            Self::SmaaBlend => "smaa blend",
            Self::Upscale => "upscale",
        }
    }
}

/// Where a stage writes.
pub(crate) enum StageTarget<'a> {
    /// The frame's output (a surface or capture).
    Output(&'a wgpu::TextureView),
    /// A texture in the registry by name.
    Texture(&'static str),
}

/// A bind group rebuilt when the versions of what it binds change.
struct CachedGroup {
    versions: Vec<u64>,
    group: wgpu::BindGroup,
}

pub(crate) struct PostChain {
    /// A frame and the bilinear sampler: FXAA, TAA's output, the upscale and SMAA's edges.
    frame_layout: wgpu::BindGroupLayout,
    /// The edges, the sampler, the area and search textures and the nearest sampler.
    weights_layout: wgpu::BindGroupLayout,
    /// The frame, the sampler and the weights.
    blend_layout: wgpu::BindGroupLayout,
    bilinear: wgpu::Sampler,
    nearest: wgpu::Sampler,
    area: wgpu::TextureView,
    search: wgpu::TextureView,
    smaa_edges: PipelineId,
    smaa_weights: PipelineId,
    stages: Vec<((OutputStage, wgpu::TextureFormat), PipelineId)>,
    groups: Vec<(&'static str, CachedGroup)>,
}

fn lookup_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    (width, height): (u32, u32),
    compressed: &[u8],
) -> wgpu::TextureView {
    let texels = decompress(compressed);
    let bytes_per_texel = format.block_copy_size(None).unwrap_or(1);
    assert_eq!(
        texels.len(),
        (width * height * bytes_per_texel) as usize,
        "{label}: the embedded lookup texture has the wrong size"
    );
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * bytes_per_texel),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&Default::default())
}

/// Inflates one of the embedded lookup textures.
pub(crate) fn decompress(compressed: &[u8]) -> Vec<u8> {
    miniz_oxide::inflate::decompress_to_vec_zlib(compressed)
        .expect("the embedded SMAA lookup textures inflate")
}

impl PostChain {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(FXAA_SHADER)?;
        library.add(SMAA_SHADER)?;
        library.add(OUTPUT_SHADER)?;
        let fragment = wgpu::ShaderStages::VERTEX_FRAGMENT;
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
        let sampler = |binding, kind| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Sampler(kind),
            count: None,
        };
        let filtering = wgpu::SamplerBindingType::Filtering;
        // The frame's and the blend's passes may write the output: the dither's mask and phase beside them.
        let [mask, phase] = Dither::layout_entries();
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post frame"),
            entries: &[texture(0), sampler(1, filtering), mask, phase],
        });
        let weights_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa weights"),
            entries: &[
                texture(0),
                sampler(1, filtering),
                texture(2),
                texture(3),
                sampler(4, wgpu::SamplerBindingType::NonFiltering),
            ],
        });
        let blend_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smaa blend"),
            entries: &[texture(0), sampler(1, filtering), texture(2), mask, phase],
        });
        let bilinear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post bilinear"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let nearest = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post nearest"),
            ..Default::default()
        });
        let area = lookup_texture(
            device,
            queue,
            "smaa area",
            wgpu::TextureFormat::Rg8Unorm,
            SMAA_AREA_SIZE,
            SMAA_AREA,
        );
        let search = lookup_texture(
            device,
            queue,
            "smaa search",
            wgpu::TextureFormat::R8Unorm,
            SMAA_SEARCH_SIZE,
            SMAA_SEARCH,
        );
        let smaa_edges = pipelines.add(
            device,
            library,
            "smaa edges",
            SMAA_SHADER.path,
            Vec::new(),
            fullscreen_recipe(
                vec![frame_layout.clone()],
                "edges_vertex",
                "edges",
                EDGES_FORMAT,
                None,
            ),
        )?;
        let smaa_weights = pipelines.add(
            device,
            library,
            "smaa weights",
            SMAA_SHADER.path,
            Vec::new(),
            fullscreen_recipe(
                vec![weights_layout.clone()],
                "weights_vertex",
                "weights",
                WEIGHTS_FORMAT,
                None,
            ),
        )?;
        Ok(Self {
            frame_layout,
            weights_layout,
            blend_layout,
            bilinear,
            nearest,
            area,
            search,
            smaa_edges,
            smaa_weights,
            stages: Vec::new(),
            groups: Vec::new(),
        })
    }

    /// The pipeline for `stage` writing to `format`, built on first use. A target that encodes
    /// sRGB in hardware gets the display values decoded first; an 8- or 10-bit one the dither.
    pub(crate) fn ensure_stage(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        stage: OutputStage,
        format: wgpu::TextureFormat,
    ) -> Result<PipelineId, ShaderError> {
        if let Some((_, id)) = self.stages.iter().find(|(key, _)| *key == (stage, format)) {
            return Ok(*id);
        }
        let mut defs = if format.is_srgb() {
            vec![("DECODE_SRGB".to_owned(), ShaderDefValue::Bool(true))]
        } else {
            Vec::new()
        };
        defs.extend(dither_defs(format));
        let (shader, layout, vertex, fragment) = match stage {
            OutputStage::Fxaa => (OUTPUT_SHADER, &self.frame_layout, "vertex", "fxaa_output"),
            OutputStage::Taa => (OUTPUT_SHADER, &self.frame_layout, "vertex", "taa_output"),
            OutputStage::Upscale => (OUTPUT_SHADER, &self.frame_layout, "vertex", "upscale"),
            OutputStage::SmaaBlend => (SMAA_SHADER, &self.blend_layout, "blend_vertex", "blend"),
        };
        let id = pipelines.add(
            device,
            library,
            // A portable label: no `:` or `/` (see `portable_label`).
            format!("{} to {format:?}", stage.label()),
            shader.path,
            defs,
            fullscreen_recipe(vec![layout.clone()], vertex, fragment, format, None),
        )?;
        self.stages.push(((stage, format), id));
        Ok(id)
    }

    /// Forgets the cached bind groups, so none keeps a texture the registry dropped alive (they are made again
    /// as the stages need them).
    pub(crate) fn forget_groups(&mut self) {
        self.groups.clear();
    }

    /// Registers the textures `stages` write that do not exist yet, and drops the ones no stage
    /// uses this frame. Returns whether it dropped any.
    pub(crate) fn ensure_targets(
        device: &wgpu::Device,
        resources: &mut Resources,
        display_frame: bool,
        post_frame: bool,
        smaa: bool,
    ) -> bool {
        let attachment =
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
        let wanted = [
            (
                crate::taa::DISPLAY_FRAME,
                FRAME_FORMAT,
                attachment | wgpu::TextureUsages::COPY_SRC,
                display_frame,
            ),
            (
                POST_FRAME,
                FRAME_FORMAT,
                attachment | wgpu::TextureUsages::COPY_SRC,
                post_frame,
            ),
            (SMAA_EDGES, EDGES_FORMAT, attachment, smaa),
            (SMAA_WEIGHTS, WEIGHTS_FORMAT, attachment, smaa),
        ];
        let mut dropped = false;
        for (name, format, usage, wanted) in wanted {
            let present = resources.texture(name).is_some();
            if wanted && !present {
                resources.create_texture(device, name, TextureDesc::viewport(format, usage, 1.0));
            } else if !wanted && present {
                // Temporal AA always draws through the display frame (it is never direct), so a frame without one
                // has no use for it either.
                resources.remove(name);
                dropped = true;
            }
        }
        dropped
    }

    fn group(
        &mut self,
        device: &wgpu::Device,
        dither: &Dither,
        key: &'static str,
        layout: Layout,
        views: &[(&wgpu::TextureView, u64)],
    ) -> &wgpu::BindGroup {
        let versions: Vec<u64> = views.iter().map(|(_, version)| *version).collect();
        let index = match self.groups.iter().position(|(name, _)| *name == key) {
            Some(index) if self.groups[index].1.versions == versions => index,
            found => {
                fn texture(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
                    wgpu::BindGroupEntry {
                        binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    }
                }
                let bilinear = wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.bilinear),
                };
                let [mask, phase] = dither.entries();
                let (layout, entries) = match layout {
                    Layout::Frame => (
                        &self.frame_layout,
                        vec![texture(0, views[0].0), bilinear, mask, phase],
                    ),
                    Layout::Weights => (
                        &self.weights_layout,
                        vec![
                            texture(0, views[0].0),
                            bilinear,
                            texture(2, &self.area),
                            texture(3, &self.search),
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(&self.nearest),
                            },
                        ],
                    ),
                    Layout::Blend => (
                        &self.blend_layout,
                        vec![
                            texture(0, views[0].0),
                            bilinear,
                            texture(2, views[1].0),
                            mask,
                            phase,
                        ],
                    ),
                };
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(key),
                    layout,
                    entries: &entries,
                });
                let cached = CachedGroup { versions, group };
                match found {
                    Some(index) => {
                        self.groups[index].1 = cached;
                        index
                    }
                    None => {
                        self.groups.push((key, cached));
                        self.groups.len() - 1
                    }
                }
            }
        };
        &self.groups[index].1.group
    }

    /// SMAA's edge and weight passes on the frame called `source`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_smaa(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        dither: &Dither,
        source: &'static str,
    ) {
        let texture = |name| resources.texture(name).expect("made by ensure_targets");
        let (frame, edges, weights) = (texture(source), texture(SMAA_EDGES), texture(SMAA_WEIGHTS));
        let group = self
            .group(
                device,
                dither,
                "smaa edges",
                Layout::Frame,
                &[(&frame.view, frame.version)],
            )
            .clone();
        fullscreen_pass(
            encoder,
            timer,
            "smaa edges",
            &edges.view,
            pipelines.render(self.smaa_edges),
            &group,
        );
        let group = self
            .group(
                device,
                dither,
                "smaa weights",
                Layout::Weights,
                &[(&edges.view, edges.version)],
            )
            .clone();
        fullscreen_pass(
            encoder,
            timer,
            "smaa weights",
            &weights.view,
            pipelines.render(self.smaa_weights),
            &group,
        );
    }

    /// One output stage: `source` (a registry texture) through `stage` into `target`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_stage(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        dither: &Dither,
        stage: OutputStage,
        pipeline: PipelineId,
        source: &'static str,
        target: StageTarget<'_>,
    ) {
        let frame = resources.texture(source).expect("made by ensure_targets");
        let group = if stage == OutputStage::SmaaBlend {
            let weights = resources
                .texture(SMAA_WEIGHTS)
                .expect("made by ensure_targets");
            self.group(
                device,
                dither,
                "smaa blend",
                Layout::Blend,
                &[
                    (&frame.view, frame.version),
                    (&weights.view, weights.version),
                ],
            )
        } else {
            self.group(
                device,
                dither,
                stage.label(),
                Layout::Frame,
                &[(&frame.view, frame.version)],
            )
        }
        .clone();
        let view = match target {
            StageTarget::Output(view) => view,
            StageTarget::Texture(name) => {
                &resources
                    .texture(name)
                    .expect("made by ensure_targets")
                    .view
            }
        };
        fullscreen_pass(
            encoder,
            timer,
            stage.label(),
            view,
            pipelines.render(pipeline),
            &group,
        );
    }
}

/// SMAA's edges: two channels, 0 or 1, cleared to 0 where no edge is found (8 bits, as Jimenez's reference keeps them).
const EDGES_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
/// SMAA's weights, in 0–1: 8 bits a channel, as Jimenez's reference and its 8-bit area texture (three's half floats
/// held twice the bytes for nothing they show).
const WEIGHTS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

#[derive(Clone, Copy)]
enum Layout {
    Frame,
    Weights,
    Blend,
}

fn fullscreen_pass(
    encoder: &mut wgpu::CommandEncoder,
    timer: &GpuTimer,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
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
        timestamp_writes: timer.render_pass(label),
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
    fn the_names_are_the_webs() {
        for mode in Antialiasing::ALL {
            assert_eq!(Antialiasing::from_name(mode.name()), Some(mode));
        }
        assert_eq!(Antialiasing::from_name("off"), Some(Antialiasing::None));
        assert_eq!(Antialiasing::from_name("msaa"), None);
    }

    #[test]
    fn the_scene_shrinks_by_the_render_scale_and_no_further_than_a_quarter() {
        assert_eq!(scene_size((1920, 1080), 1.0), (1920, 1080));
        assert_eq!(scene_size((1920, 1080), 0.75), (1440, 810));
        assert_eq!(scene_size((1728, 1030), 0.75), (1296, 773));
        assert_eq!(scene_size((1920, 1080), 0.1), (480, 270));
        assert_eq!(scene_size((1920, 1080), 1.5), (1920, 1080));
        assert_eq!(scene_size((1920, 1080), f32::NAN), (1920, 1080));
        assert_eq!(scene_size((1, 1), 0.5), (1, 1));
    }

    #[test]
    fn the_resolution_cap_holds_a_larger_output_to_its_pixels() {
        const QHD: Option<u64> = Some(2560 * 1440);
        // A 16-inch MacBook Pro's Retina screen at High's 70 %: a base of 2560 x 1440's pixels in its shape.
        let retina = (3456, 2234);
        assert_eq!(
            scene_size(retina, capped_scale(retina, 0.7, QHD)),
            (1672, 1081)
        );
        // A 4K screen's base is 2560 x 1440 itself; 1440p and smaller are their own.
        assert_eq!(
            scene_size((3840, 2160), capped_scale((3840, 2160), 1.0, QHD)),
            (2560, 1440)
        );
        assert_eq!(capped_scale((2560, 1440), 0.7, QHD), 0.7);
        assert_eq!(capped_scale((1920, 1080), 0.7, QHD), 0.7);
        assert_eq!(capped_scale(retina, 0.7, None), 0.7);
        // Never below a quarter of the output.
        assert_eq!(capped_scale((15360, 8640), 0.5, QHD), RENDER_SCALE_MIN);
    }

    #[test]
    fn the_embedded_smaa_lookups_are_threes() {
        let area = decompress(SMAA_AREA);
        assert_eq!(area.len(), 160 * 560 * 2);
        let search = decompress(SMAA_SEARCH);
        assert_eq!(search.len(), 66 * 33);
        // The search texture holds lengths 0, 1 and 2 (scaled by 255 in the shader).
        assert!(search.iter().all(|&value| value <= 2));
        assert!(search.contains(&2));
        // The area texture's first subtexture starts with no coverage at distance 0.
        assert_eq!(&area[..2], &[0, 0]);
        assert!(area.iter().any(|&value| value > 200));
    }
}
