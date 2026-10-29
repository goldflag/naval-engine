//! The output's end on a real GPU, against the CPU twin: an HDR (float) output drawn for a headroom shows highlights
//! above SDR white and AgX below the knee, through edge smoothing too; an 8-bit output ignores the headroom and is
//! dithered (a flat field keeps its mean, black stays black, the same frame gets the same noise); and the linear
//! present decodes an encoded frame onto a linear surface with SDR white at the display's level.

use crate::common;

use common::read_texture;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::display::reference;
use naval_render::{
    Antialiasing, Camera, DrawContext, FrameInput, FrameTime, LinearPresent, Phase, PipelineId,
    RenderContext, RenderOutput, Renderer, SceneLayer, ScenePipeline,
};

const SHADER: &str = r#"
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
@group(1) @binding(0) var<uniform> radiance: vec4<f32>;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> FullscreenVertex {
    return fullscreen_vertex(i, 0.5);
}
@fragment fn fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    return radiance;
}
"#;

/// A scene of one radiance everywhere.
struct Flat {
    pipeline: PipelineId,
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl Flat {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/flat.wgsl", SHADER).unwrap();
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let pipeline = ctx
            .add_scene_pipeline(
                "flat",
                "test/flat.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout],
                    ..ScenePipeline::new("vertex", "fragment")
                },
            )
            .unwrap();
        Self {
            pipeline,
            buffer,
            group,
        }
    }
}

impl SceneLayer for Flat {
    fn label(&self) -> &str {
        "flat"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const FLOAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    scene: Flat,
    capture: Capture,
    float: wgpu::Texture,
}

impl Rig {
    fn new() -> Self {
        let gpu = Gpu::headless().expect("a GPU adapter");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let scene = Flat::new(&mut renderer.context());
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        let float = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("hdr output"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FLOAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self {
            gpu,
            renderer,
            scene,
            capture,
            float,
        }
    }

    fn draw(&mut self, radiance: f32, frame: u64, float: bool) {
        self.gpu.queue.write_buffer(
            &self.scene.buffer,
            0,
            bytemuck::cast_slice(&[radiance, radiance, radiance, 1.0]),
        );
        let float_view = self.float.create_view(&Default::default());
        let output = if float {
            RenderOutput {
                view: &float_view,
                format: FLOAT,
                width: WIDTH,
                height: HEIGHT,
            }
        } else {
            RenderOutput {
                view: &self.capture.view,
                format: CAPTURE_FORMAT,
                width: WIDTH,
                height: HEIGHT,
            }
        };
        self.renderer
            .render(
                FrameInput {
                    camera: &Camera::default(),
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                &mut [&mut self.scene],
                output,
            )
            .unwrap();
    }

    /// The float output's centre, decoded from its extended sRGB to linear (SDR white 1).
    fn hdr(&mut self, radiance: f32) -> [f32; 3] {
        self.draw(radiance, 0, true);
        let values = read_texture(&self.gpu, &self.float);
        let i = (((HEIGHT / 2) * WIDTH + WIDTH / 2) * 4) as usize;
        std::array::from_fn(|c| decode(values[i + c]))
    }

    /// The 8-bit output's pixels.
    fn sdr(&mut self, radiance: f32, frame: u64) -> Vec<u8> {
        self.draw(radiance, frame, false);
        self.capture.read_rgba(&self.gpu).unwrap()
    }
}

fn decode(encoded: f32) -> f32 {
    let c = encoded.max(0.0);
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// What the CPU twin shows for a grey radiance at `headroom`.
fn expected(radiance: f32, headroom: f32) -> [f32; 3] {
    reference::agx_hdr_tone_mapping(reference::display_grade([radiance; 3], 1.0), 1.0, headroom)
}

#[test]
fn an_hdr_output_shows_highlights_above_white_and_agx_below_the_knee() {
    let mut rig = Rig::new();
    for antialiasing in Antialiasing::ALL {
        rig.renderer.set_antialiasing(antialiasing);
        for headroom in [1.0, 4.0] {
            rig.renderer.set_headroom(headroom);
            for radiance in [0.02, 0.3, 2.0, 6.0, 40.0] {
                let shown = rig.hdr(radiance);
                let want = expected(radiance, headroom);
                for c in 0..3 {
                    // Half floats and the GPU's pow: within a few thousandths, relative.
                    assert!(
                        (shown[c] - want[c]).abs() <= want[c] * 4e-3 + 2e-4,
                        "{antialiasing:?} headroom {headroom} radiance {radiance}: {shown:?} vs {want:?}"
                    );
                }
            }
        }
    }
    // A highlight goes above white only with headroom; below the knee nothing moves.
    rig.renderer.set_antialiasing(Antialiasing::None);
    rig.renderer.set_headroom(4.0);
    assert!(rig.hdr(40.0)[1] > 3.0);
    let low = rig.hdr(0.3)[1];
    rig.renderer.set_headroom(1.0);
    assert!(rig.hdr(40.0)[1] <= 1.0);
    assert!((rig.hdr(0.3)[1] - low).abs() < 1e-3);
}

#[test]
fn an_eight_bit_output_is_drawn_for_sdr_and_dithered() {
    let mut rig = Rig::new();
    rig.renderer.set_headroom(4.0);
    let mean = |pixels: &[u8]| {
        pixels.chunks(4).map(|p| f64::from(p[1])).sum::<f64>() / (WIDTH * HEIGHT) as f64
    };
    for radiance in [0.05, 0.3, 2.0, 40.0] {
        let pixels = rig.sdr(radiance, 0);
        // The headroom never reaches an 8-bit output: the field is SDR's, its mean within a tenth of a level.
        let exact = f64::from(reference::srgb_encode(expected(radiance, 1.0)[1])) * 255.0;
        assert!(
            (mean(&pixels) - exact).abs() < 0.1,
            "{radiance}: mean {} against {exact}",
            mean(&pixels)
        );
        // Spread over the neighbouring levels, never further than one.
        let levels: std::collections::BTreeSet<u8> = pixels.chunks(4).map(|p| p[1]).collect();
        if exact < 254.5 {
            assert!(levels.len() >= 2, "{radiance}: {levels:?}");
        }
        assert!(
            levels.iter().all(|&l| (f64::from(l) - exact).abs() <= 1.5),
            "{radiance}: {levels:?} about {exact}"
        );
    }
    // Black stays black.
    assert!(rig.sdr(0.0, 0).chunks(4).all(|p| p[..3] == [0, 0, 0]));
    // The same presentation time gets the same noise (a capture, a paused battle); the next frame's other noise.
    let first = rig.sdr(0.3, 7);
    assert_eq!(rig.sdr(0.3, 7), first);
    assert_ne!(rig.sdr(0.3, 8), first);
}

#[test]
fn the_linear_present_decodes_onto_a_linear_surface_at_its_sdr_white() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let make = |label, usage| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 4,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: naval_render::present::FRAME_FORMAT,
            usage,
            view_formats: &[],
        })
    };
    let frame = make(
        "encoded frame",
        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    );
    let surface = make(
        "linear surface",
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    // Black, a mid grey, SDR white and a highlight at 4x white, encoded.
    let encoded = [0.0f32, 0.5, 1.0, 1.055 * 4f32.powf(1.0 / 2.4) - 0.055];
    let texels: Vec<u8> = encoded
        .iter()
        .flat_map(|&e| [e, e, e, 1.0])
        .flat_map(|c| half::f16::from_f32(c).to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        frame.as_image_copy(),
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * 8),
            rows_per_image: Some(1),
        },
        frame.size(),
    );
    let present = LinearPresent::new(&gpu.device);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    // Windows at an SDR white of 200 nits: scRGB's 1 is 80.
    present.encode(
        &gpu.device,
        &gpu.queue,
        &mut encoder,
        &frame.create_view(&Default::default()),
        &surface.create_view(&Default::default()),
        2.5,
    );
    gpu.queue.submit([encoder.finish()]);
    let values = read_texture(&gpu, &surface);
    for (i, want) in [0.0f32, 0.214 * 2.5, 2.5, 10.0].into_iter().enumerate() {
        let got = values[i * 4 + 1];
        assert!(
            (got - want).abs() <= want * 5e-3 + 1e-3,
            "texel {i}: {got} vs {want}"
        );
    }
}
