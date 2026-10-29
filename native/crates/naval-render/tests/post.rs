//! Bloom and eye adaptation on a real GPU, against CPU twins: a uniform scene
//! blooms by exactly what three's chain gives a constant image, a dim scene opens
//! the eye as `EyeAdaptation.ts` would, a snap settles at once, and rendering a
//! frame again does not move the eye.

use crate::common;

use common::{read_texture, texel};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::display::reference;
use naval_render::post::{
    Hour, NOON_CLEAR_SKY, night_vision_weight, reference_adapt, reference_adapt_in,
};
use naval_render::sun_glare::{self, SUN_GLARE_BUFFER};
use naval_render::{
    BloomSettings, Camera, DisplaySettings, DrawContext, EYE_STATE_BUFFER, EyeAdaptationSettings,
    FlareSun, FrameInput, FrameTime, Phase, PipelineId, RenderContext, RenderOutput, Renderer,
    SKY_MEAN_BUFFER, SceneLayer, ScenePipeline, SecondView, SkyMean,
};

const SHADER: &str = r#"
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
@group(1) @binding(0) var<uniform> radiance: vec4<f32>;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> FullscreenVertex {
    return fullscreen_vertex(i, 0.5);
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn fragment(in: FullscreenVertex) -> Fragment {
    var out: Fragment;
    out.color = vec4<f32>(radiance.rgb, 1.0);
    // The night tests put a sky at clear depth on the left and a surface on the right, at the same radiance.
    out.depth = select(0.5, -radiance.w, radiance.w < 0.0);
    if radiance.w < 0.0 && in.uv.x < 0.5 {
        out.depth = 0.0;
    }
    return out;
}
// A transparent sheet of the same radiance over the whole frame, nearer than both, covering `smoke.x` of it.
@group(1) @binding(1) var<uniform> smoke: vec4<f32>;
@vertex fn smoke_vertex(@builtin(vertex_index) i: u32) -> FullscreenVertex {
    return fullscreen_vertex(i, 0.9);
}
@fragment fn smoke_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(radiance.rgb, smoke.x);
}
"#;

/// A scene of one radiance everywhere.
struct Uniform {
    pipeline: PipelineId,
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// The transparent sheet's pipeline and its cover (0: not drawn).
    smoke: PipelineId,
    smoke_buffer: wgpu::Buffer,
    cover: std::cell::Cell<f32>,
}

impl Uniform {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/uniform.wgsl", SHADER).unwrap();
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }),
            });
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let smoke_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: smoke_buffer.as_entire_binding(),
                },
            ],
        });
        let pipeline = ctx
            .add_scene_pipeline(
                "uniform",
                "test/uniform.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout.clone()],
                    ..ScenePipeline::new("vertex", "fragment")
                },
            )
            .unwrap();
        // Blended as the effects' smoke is: straight alpha on the colour, the cover left in the alpha.
        let smoke = ctx
            .add_scene_pipeline(
                "uniform smoke",
                "test/uniform.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout],
                    depth_write: false,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    ..ScenePipeline::new("smoke_vertex", "smoke_fragment")
                },
            )
            .unwrap();
        Self {
            pipeline,
            buffer,
            group,
            smoke,
            smoke_buffer,
            cover: std::cell::Cell::new(0.0),
        }
    }

    fn set(&self, queue: &wgpu::Queue, radiance: f32) {
        queue.write_buffer(
            &self.buffer,
            0,
            bytemuck::cast_slice(&[radiance, radiance, radiance, 1.0]),
        );
    }
}

impl SceneLayer for Uniform {
    fn label(&self) -> &str {
        "uniform"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
        if phase == Phase::Transparent && self.cover.get() > 0.0 {
            pass.set_pipeline(ctx.pipelines.render(self.smoke));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

const WIDTH: u32 = 64;
const HEIGHT: u32 = 48;

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    scene: Uniform,
    capture: Capture,
}

impl Rig {
    fn new() -> Self {
        let gpu = Gpu::headless().expect("a GPU adapter");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let scene = Uniform::new(&mut renderer.context());
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        Self {
            gpu,
            renderer,
            scene,
            capture,
        }
    }

    /// Renders frame `frame` of radiance `value` and returns the centre pixel's sRGB.
    fn render(&mut self, value: f32, frame: u64, dt: f64) -> [u8; 3] {
        self.scene.set(&self.gpu.queue, value);
        let pixels = self.draw(frame, dt);
        // The frame is flat: its mean level, which the output dither (a level either way on single pixels) keeps.
        let count = (WIDTH * HEIGHT) as f64;
        std::array::from_fn(|channel| {
            let sum: f64 = pixels
                .chunks(4)
                .map(|texel| f64::from(texel[channel]))
                .sum();
            (sum / count).round() as u8
        })
    }

    /// Draws a second view of radiance `value` at `seconds` after the main frame, and returns its mean sRGB.
    fn render_second(&mut self, view: &mut SecondView, value: f32, seconds: f64) -> [u8; 3] {
        self.scene.set(&self.gpu.queue, value);
        self.renderer
            .render_view(
                view,
                FrameInput {
                    camera: &Camera::default(),
                    time: FrameTime {
                        seconds,
                        ..Default::default()
                    },
                },
                &mut [&mut self.scene],
                RenderOutput {
                    view: &self.capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .unwrap();
        let pixels = self.capture.read_rgba(&self.gpu).unwrap();
        let count = (WIDTH * HEIGHT) as f64;
        std::array::from_fn(|channel| {
            let sum: f64 = pixels
                .chunks(4)
                .map(|texel| f64::from(texel[channel]))
                .sum();
            (sum / count).round() as u8
        })
    }

    fn draw(&mut self, frame: u64, dt: f64) -> Vec<u8> {
        self.renderer
            .render(
                FrameInput {
                    camera: &Camera::default(),
                    time: FrameTime::fixed(frame, dt),
                },
                &mut [&mut self.scene],
                RenderOutput {
                    view: &self.capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .unwrap();
        self.capture.read_rgba(&self.gpu).unwrap()
    }

    /// The frame's mean over a scene of one colour everywhere, a surface as `render`'s.
    fn render_colour(&mut self, colour: [f32; 3], frame: u64) -> [u8; 3] {
        self.gpu.queue.write_buffer(
            &self.scene.buffer,
            0,
            bytemuck::cast_slice(&[colour[0], colour[1], colour[2], 1.0]),
        );
        let pixels = self.draw(frame, 1.0 / 60.0);
        let count = (WIDTH * HEIGHT) as f64;
        std::array::from_fn(|channel| {
            let sum: f64 = pixels
                .chunks(4)
                .map(|texel| f64::from(texel[channel]))
                .sum();
            (sum / count).round() as u8
        })
    }

    fn split(&mut self, value: f32, frame: u64) -> [[u8; 3]; 2] {
        self.split_at_depth(value, frame, 0.5)
    }

    fn split_at_depth(&mut self, value: f32, frame: u64, depth: f32) -> [[u8; 3]; 2] {
        self.gpu.queue.write_buffer(
            &self.scene.buffer,
            0,
            bytemuck::cast_slice(&[value, value, value, -depth]),
        );
        let pixels = self.draw(frame, 1.0 / 60.0);
        std::array::from_fn(|side| {
            std::array::from_fn(|channel| {
                let sum: f64 = pixels
                    .chunks(4)
                    .enumerate()
                    .filter(|(i, _)| i % WIDTH as usize / (WIDTH as usize / 2) == side)
                    .map(|(_, texel)| f64::from(texel[channel]))
                    .sum();
                (sum / f64::from(WIDTH * HEIGHT / 2)).round() as u8
            })
        })
    }

    /// A transparent sheet covering `cover` of every pixel from the next frame on (0: none).
    fn smoke(&mut self, cover: f32) {
        self.scene.cover.set(cover);
        self.gpu.queue.write_buffer(
            &self.scene.smoke_buffer,
            0,
            bytemuck::cast_slice(&[cover, 0.0, 0.0, 0.0]),
        );
    }

    fn sky(&mut self, radiance: f32, clear: f32) {
        self.renderer.set_sky_mean(SkyMean::Gpu);
        self.gpu.queue.write_buffer(
            &self
                .renderer
                .resources()
                .buffer(SKY_MEAN_BUFFER)
                .unwrap()
                .buffer,
            0,
            bytemuck::cast_slice(&[radiance, radiance, radiance, clear]),
        );
    }

    fn eye_state(&self) -> [f32; 4] {
        self.state(EYE_STATE_BUFFER)
    }

    /// A four-float state buffer of the renderer's, read back.
    fn state(&self, name: &str) -> [f32; 4] {
        let buffer = &self.renderer.resources().buffer(name).unwrap().buffer;
        let readback = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, 0, &readback, 0, 16);
        self.gpu.queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range().unwrap();
        let values: &[f32] = bytemuck::cast_slice(&bytes);
        [values[0], values[1], values[2], values[3]]
    }
}

/// The night tests' sky mean (its light, clouds and all): three stops over the 0.02 they draw the world at, so the lift
/// reaches it whole (it stops short of the sky's light); the clear sky's mean beside it sets the dusk fade.
const NIGHT_SKY: f32 = 0.16;

#[test]
fn night_lifts_world_shadows_and_dims_the_sky_without_lifting_black_or_highlights() {
    let mut rig = Rig::new();
    let dark = 0.02;
    let baseline = rig.split(dark, 0);
    assert_eq!(
        baseline[0], baseline[1],
        "no sky mean leaves the display alone"
    );

    rig.sky(NIGHT_SKY, 0.01);
    let night = rig.split(dark, 1);
    for channel in 0..3 {
        assert!(
            night[0][channel] < baseline[0][channel],
            "sky: {night:?}, {baseline:?}"
        );
        assert!(
            night[1][channel] > baseline[1][channel] + 5,
            "world: {night:?}, {baseline:?}"
        );
        assert!(
            night[1][channel] < 60,
            "the world still reads as night: {night:?}"
        );
    }
    assert_eq!(rig.split(0.0, 2), [[0; 3]; 2], "there is no added glow");
    let highlight = rig.split(1.0, 3);
    let expected = reference::display_srgb8([1.0; 3], DisplaySettings::default());
    for side in highlight {
        for (shown, expected) in side.into_iter().zip(expected) {
            assert!(
                shown.abs_diff(expected) <= 1,
                "highlights retain their brightness"
            );
        }
    }

    // A missing sky must disable the grade even if a previous scene left a night mean in the buffer.
    rig.renderer.set_sky_mean(SkyMean::None);
    assert_eq!(rig.split(dark, 4), baseline);
}

#[test]
fn night_readability_fades_with_the_clear_sky_and_clouds_do_not_turn_day_into_night() {
    let mut rig = Rig::new();
    let value = 0.02;
    let baseline = rig.split(value, 0);
    rig.sky(0.001, 0.5);
    assert_eq!(
        rig.split(value, 1),
        baseline,
        "an overcast day is still day"
    );

    rig.sky(NIGHT_SKY, 0.01);
    let night = rig.split(value, 2);
    rig.sky(NIGHT_SKY * 0.75, 0.01);
    assert_eq!(
        rig.split(value, 3),
        night,
        "cloud cover does not change the grade of a world well under the sky's light"
    );
    rig.sky(NIGHT_SKY, 0.05);
    let dusk = rig.split(value, 4);
    for channel in 0..3 {
        assert!(night[0][channel] <= dusk[0][channel] && dusk[0][channel] <= baseline[0][channel]);
        assert!(night[1][channel] > dusk[1][channel] && dusk[1][channel] > baseline[1][channel]);
    }
}

#[test]
fn night_readability_grades_what_smoke_covers_alike_over_the_sky_and_the_world() {
    let mut rig = Rig::new();
    rig.sky(NIGHT_SKY, 0.01);
    let bare = rig.split(0.02, 0);
    // Smoke writes no depth: all of a pixel it covers takes the near world's grade, whatever lies behind.
    rig.smoke(1.0);
    let covered = rig.split(0.02, 1);
    assert_eq!(
        covered[0], covered[1],
        "one smoke over the sky and the world"
    );
    assert_eq!(covered[1], bare[1], "graded as the near world");
    // Thin smoke leaves the sky behind it most of its own grade.
    rig.smoke(0.5);
    let thin = rig.split(0.02, 2);
    assert_eq!(thin[1], bare[1]);
    for channel in 0..3 {
        assert!(
            bare[0][channel] < thin[0][channel] && thin[0][channel] < covered[0][channel],
            "{bare:?}, {thin:?}, {covered:?}"
        );
    }
}

#[test]
fn night_readability_lifts_the_world_short_of_the_skys_light() {
    let mut rig = Rig::new();
    let value = 0.02;
    let baseline = rig.split(value, 0);
    rig.sky(NIGHT_SKY, 0.01);
    let under = rig.split(value, 1);
    // Within the quarter stop the sky is closed by, the world takes none of the lift: a sea mirroring the sky stays
    // under it.
    rig.sky(value * 1.15, 0.01);
    let beside = rig.split(value, 2);
    assert_eq!(beside[1], baseline[1], "{beside:?} {baseline:?}");
    for channel in 0..3 {
        assert!(
            under[1][channel] > beside[1][channel] + 5,
            "{under:?} {beside:?}"
        );
        assert!(
            beside[0][channel] < beside[1][channel],
            "the sky still closes: {beside:?}"
        );
    }
    // A brighter world never comes out darker than a dimmer one as the lift falls off.
    rig.sky(0.08, 0.01);
    let mut last = [0u8; 3];
    for (frame, value) in [0.004, 0.008, 0.012, 0.016, 0.02, 0.028, 0.04, 0.056, 0.08]
        .into_iter()
        .enumerate()
    {
        let shown = rig.split(value, 3 + frame as u64)[1];
        for channel in 0..3 {
            assert!(
                shown[channel] >= last[channel],
                "{value}: {shown:?} after {last:?}"
            );
        }
        last = shown;
    }
}

#[test]
fn night_readability_joins_the_distant_world_to_the_sky() {
    let mut rig = Rig::new();
    rig.sky(NIGHT_SKY, 0.01);
    let near = rig.split(0.02, 0);
    let camera = Camera::default();
    let depth_at = |metres| camera.near * (camera.far / metres - 1.0) / (camera.far - camera.near);
    let middle = rig.split_at_depth(0.02, 1, depth_at(6000.0));
    let far = rig.split_at_depth(0.02, 2, depth_at(15000.0));
    for channel in 0..3 {
        assert!(near[1][channel] > middle[1][channel] && middle[1][channel] > far[1][channel]);
        assert!(
            far[0][channel].abs_diff(far[1][channel]) <= 1,
            "the far sea shares the sky's grade: {far:?}"
        );
    }
}

#[test]
fn night_readability_takes_what_is_seen_through_water_as_near_whatever_its_depth() {
    let mut rig = Rig::new();
    rig.sky(NIGHT_SKY, 0.01);
    let above = rig.split(0.02, 0);
    // A probe one metre under everywhere: r in half floats.
    let probe = rig.gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("underwater probe"),
        size: wgpu::Extent3d::default(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    rig.gpu.queue.write_texture(
        probe.as_image_copy(),
        &[0x00, 0x3c, 0, 0, 0, 0, 0, 0],
        wgpu::TexelCopyBufferLayout::default(),
        wgpu::Extent3d::default(),
    );
    rig.renderer
        .set_underwater(Some(&probe.create_view(&Default::default())));
    let under = rig.split(0.02, 1);
    assert_eq!(
        under[1], above[1],
        "water over the near world changes nothing"
    );
    for channel in 0..3 {
        assert!(
            under[0][channel].abs_diff(under[1][channel]) <= 1,
            "the open deep shares the near water's grade: {under:?}"
        );
    }
    rig.renderer.set_underwater(None);
    assert_eq!(rig.split(0.02, 2), above);
}

fn luminance(value: f32) -> f32 {
    // A grey radiance: Rec. 709 weights sum to 1.
    value
}

/// three's bloom chain on a constant image: each blur keeps a constant times the sum of its
/// kernel (clamped edges), squared for the two directions, and each mip blurs the last.
fn constant_bloom(graded: f32, settings: BloomSettings) -> f32 {
    let alpha = {
        let t = ((luminance(graded) - settings.threshold) / settings.knee).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut level = graded * alpha;
    let mut sum = 0.0;
    for (radius, factor) in [6.0f32, 10.0, 14.0, 18.0, 22.0]
        .into_iter()
        .zip([1.0f32, 0.8, 0.6, 0.4, 0.2])
    {
        let sigma = radius / 3.0;
        let c = |i: f32| 0.39894 * (-0.5 * i * i / (sigma * sigma)).exp() / sigma;
        let kernel: f32 = c(0.0) + (1..radius as i32).map(|i| 2.0 * c(i as f32)).sum::<f32>();
        level *= kernel * kernel;
        sum += (factor + (1.2 - 2.0 * factor) * settings.radius) * level;
    }
    sum * settings.strength
}

#[test]
fn bloom_adds_what_threes_chain_gives_a_constant_image() {
    let mut rig = Rig::new();
    let settings = BloomSettings::default();
    rig.renderer.set_bloom(Some(settings));
    let radiance = 6.0;
    let shown = rig.render(radiance, 0, 1.0 / 60.0);

    let graded = reference::display_grade([radiance; 3], 1.0)[0];
    let expected = constant_bloom(graded, settings);
    assert!(
        expected > 0.1,
        "a radiance of 6 grades above the threshold: {graded}"
    );
    let bloom = read_texture(&rig.gpu, rig.renderer.bloom_texture().unwrap());
    let size = rig.renderer.bloom_texture().unwrap().size();
    let centre = texel(&bloom, size.width, 4, size.width / 2, size.height / 2);
    assert!(
        (centre[0] - expected).abs() < expected * 0.01 + 1e-3,
        "bloom {centre:?}, expected {expected}"
    );
    // The display adds it to the graded radiance ahead of AgX.
    let tone = reference::agx_tone_mapping([graded + expected; 3], 1.0).map(reference::srgb8);
    for channel in 0..3 {
        assert!(
            (i32::from(shown[channel]) - i32::from(tone[channel])).abs() <= 1,
            "shown {shown:?}, expected {tone:?}"
        );
    }

    // Below the threshold nothing blooms (alpha carries the blurs' coverage, unused).
    rig.render(0.5, 1, 1.0 / 60.0);
    let bloom = read_texture(&rig.gpu, rig.renderer.bloom_texture().unwrap());
    assert!(
        bloom
            .chunks(4)
            .all(|texel| texel[..3].iter().all(|v| *v == 0.0))
    );
}

#[test]
fn a_dim_view_opens_the_eye_and_a_snap_settles_it() {
    let mut rig = Rig::new();
    let settings = EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    };
    rig.renderer.set_eye_adaptation(settings);
    let sky = 0.5;
    rig.renderer.set_sky_mean(SkyMean::Rgb([sky; 3]));
    // A view 4 stops under the sky (no clear sky given: the sky's own): metered −4, goal min((−0.37 + 4) × 1, 2) = 2.
    let radiance = sky / 16.0;
    let cells = [radiance.log2(); 256];
    rig.renderer.snap_eye_adaptation();
    let shown = rig.render(radiance, 0, 1.0 / 60.0);
    let state = rig.eye_state();
    let (stops, metered, goal) =
        reference_adapt(&settings, &cells, sky, 0.0, 0.0, 1.0 / 60.0, true);
    assert!(
        (state[1] - metered).abs() < 1e-3,
        "metered {state:?}, {metered}"
    );
    assert!((state[2] - goal).abs() < 1e-3 && (goal - settings.max_stops).abs() < 1e-3);
    assert!((state[0] - stops).abs() < 1e-4, "a snap settles: {state:?}");
    let exposure = 2f32.powf(state[0]);
    let expected = reference::display_srgb8(
        [radiance; 3],
        DisplaySettings {
            exposure,
            ..Default::default()
        },
    );
    for channel in 0..3 {
        assert!(
            (i32::from(shown[channel]) - i32::from(expected[channel])).abs() <= 1,
            "shown {shown:?}, expected {expected:?} at {exposure}x"
        );
    }

    // Back to a bright view, brighter than the reference: the eye closes, past the fixed exposure, with the 0.4 s
    // time constant, frame by frame.
    let bright = sky * 2.0;
    let mut stops = state[0];
    for frame in 1..=6 {
        rig.render(bright, frame, 0.1);
        stops = reference_adapt(
            &settings,
            &[bright.log2(); 256],
            sky,
            0.0,
            stops,
            0.1,
            false,
        )
        .0;
    }
    let state = rig.eye_state();
    assert!((state[0] - stops).abs() < 1e-3, "{state:?} vs {stops}");
    assert!(
        state[2] < 0.0,
        "a view brighter than the reference aims past the fixed exposure: {state:?}"
    );
    // Rendering the same frame again does not move the eye.
    rig.render(bright, 6, 0.1);
    assert!((rig.eye_state()[0] - state[0]).abs() < 1e-7);
}

#[test]
fn a_second_view_meters_its_own_scene_whatever_the_main_eye() {
    let settings = EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    };
    let sky: f32 = 0.5;
    // The fall of shot inset's sea, two stops under the sky: its eye opens as far as its own view asks.
    let inset = sky / 4.0;
    let (stops, _, _) = reference_adapt(&settings, &[inset.log2(); 256], sky, 0.0, 0.0, 0.0, true);
    let expected = reference::display_srgb8(
        [inset; 3],
        DisplaySettings {
            exposure: 2f32.powf(stops),
            ..Default::default()
        },
    );
    // The main camera in shadow (its eye wide open) and then looking into a bright sky (its eye closed): the inset, a
    // view of somewhere else, is shown alike under both.
    for main in [sky / 16.0, sky * 4.0] {
        let mut rig = Rig::new();
        rig.renderer.set_eye_adaptation(settings);
        rig.renderer.set_sky_mean(SkyMean::Rgb([sky; 3]));
        rig.renderer.snap_eye_adaptation();
        rig.render(main, 0, 1.0 / 60.0);
        let mut view = SecondView::default();
        let shown = rig.render_second(&mut view, inset, 0.0);
        for channel in 0..3 {
            assert!(
                (i32::from(shown[channel]) - i32::from(expected[channel])).abs() <= 1,
                "main {main}: shown {shown:?}, expected {expected:?}"
            );
        }
        // The main eye is the main view's alone.
        let (main_stops, _, _) = reference_adapt(
            &settings,
            &[main.log2(); 256],
            sky,
            0.0,
            0.0,
            1.0 / 60.0,
            true,
        );
        assert!((rig.eye_state()[0] - main_stops).abs() < 1e-3);
    }
}

#[test]
fn without_a_sky_mean_the_eye_holds_at_the_fixed_exposure() {
    let mut rig = Rig::new();
    rig.renderer.set_eye_adaptation(EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    });
    let shown = rig.render(0.01, 0, 1.0 / 60.0);
    assert_eq!(rig.eye_state(), [0.0; 4]);
    assert_eq!(
        shown,
        reference::display_srgb8([0.01; 3], DisplaySettings::default())
    );
}

const SUN_SHADER: &str = r#"
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
// xy the disc's centre (pixels), z its radius (pixels), w the sky's radiance.
@group(1) @binding(0) var<uniform> disc: vec4<f32>;
// x what the disc adds to the sky, y the depth of a surface in front (0: none), z the column it starts at.
@group(1) @binding(1) var<uniform> front: vec4<f32>;
@vertex fn vertex(@builtin(vertex_index) i: u32) -> FullscreenVertex {
    return fullscreen_vertex(i, 0.5);
}
struct Fragment {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}
@fragment fn fragment(@builtin(position) at: vec4<f32>) -> Fragment {
    var out: Fragment;
    let lit = select(0.0, front.x, distance(at.xy, disc.xy) < disc.z);
    out.color = vec4<f32>(vec3<f32>(disc.w + lit), 1.0);
    out.depth = 0.0;
    // The surface is as bright as the sky, and hides the disc.
    if front.y > 0.0 && at.x >= front.z {
        out.color = vec4<f32>(vec3<f32>(disc.w), 1.0);
        out.depth = front.y;
    }
    return out;
}
// A transparent over the disc alone (a rain streak's core, thick smoke), covering `front.w` of it, as bright as a full
// disc's excess: straight alpha on the colour, the cover left in the alpha, as the effects blend.
@vertex fn sheet_vertex(@builtin(vertex_index) i: u32) -> FullscreenVertex {
    return fullscreen_vertex(i, 0.9);
}
@fragment fn sheet(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    if distance(at.xy, disc.xy) >= disc.z * 1.2 {
        discard;
    }
    return vec4<f32>(vec3<f32>(disc.w + 1.0), front.w);
}
"#;

/// A sky at clear depth with a bright disc on it, a surface that may stand in front from some column on, and a
/// transparent that may cover the disc.
struct SunSky {
    pipeline: PipelineId,
    sheet: PipelineId,
    disc: wgpu::Buffer,
    front: wgpu::Buffer,
    group: wgpu::BindGroup,
    /// The transparent's cover over the disc (0: not drawn).
    cover: std::cell::Cell<f32>,
}

impl SunSky {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library
            .add_source("test/sun_sky.wgsl", SUN_SHADER)
            .unwrap();
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &[0, 1].map(|binding| wgpu::BindGroupLayoutEntry {
                    binding,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }),
            });
        let [disc, front] = [(); 2].map(|()| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: disc.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: front.as_entire_binding(),
                },
            ],
        });
        let pipeline = ctx
            .add_scene_pipeline(
                "sun sky",
                "test/sun_sky.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout.clone()],
                    ..ScenePipeline::new("vertex", "fragment")
                },
            )
            .unwrap();
        let sheet = ctx
            .add_scene_pipeline(
                "sun sheet",
                "test/sun_sky.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout],
                    depth_write: false,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    ..ScenePipeline::new("sheet_vertex", "sheet")
                },
            )
            .unwrap();
        Self {
            pipeline,
            sheet,
            disc,
            front,
            group,
            cover: std::cell::Cell::new(0.0),
        }
    }
}

impl SceneLayer for SunSky {
    fn label(&self) -> &str {
        "sun sky"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
        if phase == Phase::Transparent && self.cover.get() > 0.0 {
            pass.set_pipeline(ctx.pipelines.render(self.sheet));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[test]
fn the_sun_glares_by_what_the_frame_shows_of_its_disc() {
    let mut rig = Rig::new();
    rig.renderer.set_bloom(Some(BloomSettings::default()));
    let mut sky = SunSky::new(&mut rig.renderer.context());
    // The camera looks straight at the sun: its disc at the frame's centre, six pixels in radius, one unit of radiance
    // over a sky of a quarter (neither blooms: both grade under the threshold).
    let camera = Camera::default();
    let forward = (camera.target - camera.eye).normalize();
    let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan();
    let pixel = 2.0 * tan / HEIGHT as f32;
    let radius_px = 6.0;
    let irradiance = 6.0;
    let sun = FlareSun {
        direction: forward,
        irradiance: glam::Vec3::splat(irradiance),
        angular_radius: 0.004675,
        drawn_radius: radius_px * pixel,
        disc_radiance: glam::Vec3::splat(1.0),
    };
    let centre = [WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5];
    rig.gpu.queue.write_buffer(
        &sky.disc,
        0,
        bytemuck::cast_slice(&[centre[0], centre[1], radius_px, 0.25]),
    );
    let mut frame = 0;
    let mut draw =
        |rig: &mut Rig, sky: &mut SunSky, camera: &Camera, front: [f32; 2], dt: f64, snap: bool| {
            rig.gpu.queue.write_buffer(
                &sky.front,
                0,
                bytemuck::cast_slice(&[1.0f32, front[0], front[1], 0.0]),
            );
            if snap {
                rig.renderer.snap_eye_adaptation();
            }
            frame += 1;
            rig.renderer
                .render(
                    FrameInput {
                        camera,
                        time: FrameTime::fixed(frame, dt),
                    },
                    &mut [sky],
                    RenderOutput {
                        view: &rig.capture.view,
                        format: CAPTURE_FORMAT,
                        width: WIDTH,
                        height: HEIGHT,
                    },
                )
                .unwrap();
            let texture = rig.renderer.bloom_texture().unwrap();
            (read_texture(&rig.gpu, texture), texture.size())
        };
    // The veil at a texel of the bloom's composite, for a sun seen in full.
    let expected = |size: wgpu::Extent3d, x: u32, y: u32| {
        let ndc = [
            (x as f32 + 0.5) / size.width as f32 * 2.0 - 1.0,
            1.0 - (y as f32 + 0.5) / size.height as f32 * 2.0,
        ];
        let aspect = WIDTH as f32 / HEIGHT as f32;
        let ray = glam::Vec3::new(ndc[0] * tan * aspect, ndc[1] * tan, -1.0).normalize();
        let angle = ray.distance(glam::Vec3::NEG_Z).max(sun.drawn_radius);
        // The look's exposure (0.9) over the display's (1), the eye at rest.
        irradiance * sun_glare::SHARE * sun_glare::veil(angle) * 0.9
    };

    // No sun set: the bloom is the plain one, nothing here.
    let (bloom, _) = draw(&mut rig, &mut sky, &camera, [0.0, 0.0], 1.0 / 60.0, true);
    assert!(bloom.chunks(4).all(|t| t[..3].iter().all(|v| *v == 0.0)));

    // The sun in full: the veil of the CIE's function about it, flat across the disc.
    rig.renderer.set_sun_glare(Some(sun));
    let (bloom, size) = draw(&mut rig, &mut sky, &camera, [0.0, 0.0], 1.0 / 60.0, true);
    let state = rig.state(SUN_GLARE_BUFFER);
    assert!(
        (state[0] - 1.0).abs() < 1e-3 && state[2] == 48.0,
        "{state:?}"
    );
    for (x, y) in [
        (size.width / 2, size.height / 2),
        (size.width / 2 + 8, size.height / 2),
        (1, 1),
    ] {
        let got = texel(&bloom, size.width, 4, x, y)[0];
        let want = expected(size, x, y);
        assert!(
            (got - want).abs() < want * 0.01,
            "({x}, {y}): {got}, {want}"
        );
    }
    let full = texel(&bloom, size.width, 4, 1, 1)[0];
    assert!(
        texel(&bloom, size.width, 4, size.width / 2, size.height / 2)[0] > 20.0 * full,
        "white hot at the disc, a faint veil in the corner"
    );

    // A surface in front of half the disc: half the glare. In front of all of it: none.
    let (bloom, _) = draw(
        &mut rig,
        &mut sky,
        &camera,
        [0.5, centre[0]],
        1.0 / 60.0,
        true,
    );
    let half = texel(&bloom, size.width, 4, 1, 1)[0] / full;
    assert!((half - 0.5).abs() < 0.08, "{half}");
    let (bloom, _) = draw(&mut rig, &mut sky, &camera, [0.5, 0.0], 1.0 / 60.0, true);
    assert!(bloom.chunks(4).all(|t| t[..3].iter().all(|v| *v == 0.0)));

    // Out from behind it the glare eases in: after one time constant, 1 - 1/e of it.
    let (bloom, _) = draw(
        &mut rig,
        &mut sky,
        &camera,
        [0.0, 0.0],
        f64::from(sun_glare::EASE_S),
        false,
    );
    let eased = texel(&bloom, size.width, 4, 1, 1)[0] / full;
    assert!((eased - (1.0 - (-1.0f32).exp())).abs() < 0.01, "{eased}");
    // With the clock stopped (a pause, photo mode) it settles at once: hidden, then out again.
    let (bloom, _) = draw(&mut rig, &mut sky, &camera, [0.5, 0.0], 0.0, false);
    assert!(bloom.chunks(4).all(|t| t[..3].iter().all(|v| *v == 0.0)));
    let (bloom, _) = draw(&mut rig, &mut sky, &camera, [0.0, 0.0], 0.0, false);
    let stopped = texel(&bloom, size.width, 4, 1, 1)[0] / full;
    assert!((stopped - 1.0).abs() < 1e-3, "{stopped}");

    // The camera turned down until the sun stands half a margin over the frame: its disc is no longer measured, and
    // the glare keeps what was last seen of it, at half its strength.
    let down = ((1.0 + sun_glare::MARGIN) * tan).atan();
    let turned = Camera {
        target: camera.eye
            + glam::Quat::from_axis_angle(forward.cross(glam::Vec3::Y).normalize(), -down)
                * forward,
        ..camera
    };
    let (bloom, _) = draw(&mut rig, &mut sky, &turned, [0.0, 0.0], 1.0 / 60.0, false);
    let state = rig.state(SUN_GLARE_BUFFER);
    assert!(
        (state[0] - 1.0).abs() < 1e-3 && state[2] == 0.0,
        "{state:?}"
    );
    let top = texel(&bloom, size.width, 4, size.width / 2, 0)[0];
    let placed = turned.view().transform_vector3(forward);
    let ray = glam::Vec3::new(
        0.5 / size.width as f32 * 2.0 * tan * WIDTH as f32 / HEIGHT as f32,
        (1.0 - 0.5 / size.height as f32 * 2.0) * tan,
        -1.0,
    )
    .normalize();
    let want = irradiance * sun_glare::SHARE * 0.5 * sun_glare::veil(ray.distance(placed)) * 0.9;
    assert!((top - want).abs() < want * 0.02, "{top}, {want}");
    // A capture of that view settles at once: it takes no glare from a sun it cannot measure.
    let (bloom, _) = draw(&mut rig, &mut sky, &turned, [0.0, 0.0], 1.0 / 60.0, true);
    assert!(bloom.chunks(4).all(|t| t[..3].iter().all(|v| *v == 0.0)));
}

#[test]
fn night_readability_follows_the_hosts_sun_whatever_the_sky() {
    let mut rig = Rig::new();
    let value = 0.02;
    let baseline = rig.split(value, 0);
    // A storm's dusk: under the deck the clear sky's mean reads as dark as a night's, so by the sky's luminance the
    // grade came full on; with the sun 3° up it is the dusk's half, as under a clear dusk's sky.
    rig.sky(NIGHT_SKY, 0.02);
    let by_sky = rig.split(value, 1);
    rig.renderer.set_sun_elevation(Some(3.0));
    let storm_dusk = rig.split(value, 2);
    rig.sky(NIGHT_SKY * 2.0, 0.05);
    assert_eq!(
        rig.split(value, 3),
        storm_dusk,
        "the sun, not the sky, sets the grade"
    );
    for channel in 0..3 {
        assert!(
            baseline[1][channel] < storm_dusk[1][channel]
                && storm_dusk[1][channel] < by_sky[1][channel],
            "{baseline:?} {storm_dusk:?} {by_sky:?}"
        );
    }
    // Night is the full grade whatever the sky's mean (the moon brightens a clear night's).
    rig.renderer.set_sun_elevation(Some(-28.0));
    let night = rig.split(value, 4);
    rig.sky(NIGHT_SKY, 0.01);
    assert_eq!(rig.split(value, 5), night);
    // By day there is none, even under a sky as dark as a night's.
    rig.renderer.set_sun_elevation(Some(30.0));
    rig.sky(0.001, 0.01);
    assert_eq!(rig.split(value, 6), baseline);
}

#[test]
fn by_moonlight_the_eye_sees_grey_and_fires_keep_their_colour() {
    let mut rig = Rig::new();
    rig.sky(NIGHT_SKY, 0.01);
    // The moonlit sea's blue, dim enough for the rods alone, and a fire's orange, bright enough for the cones.
    let (moonlit, fire) = ([0.004, 0.01, 0.03], [5.0, 1.6, 0.3]);
    rig.renderer.set_sun_elevation(Some(30.0));
    let (day_moonlit, day_fire) = (rig.render_colour(moonlit, 0), rig.render_colour(fire, 1));
    rig.renderer.set_sun_elevation(Some(-28.0));
    let (night_moonlit, night_fire) = (rig.render_colour(moonlit, 2), rig.render_colour(fire, 3));
    // The blue turns a faint cool grey (lifted by the night's grade, so brighter than by day): the photograph's sky,
    // (46, 52, 60), is 0.23 saturated.
    let saturation = |c: [u8; 3]| {
        let (low, high) = (c.iter().min().copied(), c.iter().max().copied());
        f32::from(high.unwrap() - low.unwrap()) / f32::from(high.unwrap().max(1))
    };
    assert!(saturation(day_moonlit) > 0.8, "{day_moonlit:?}");
    assert!(
        saturation(night_moonlit) < 0.4,
        "{night_moonlit:?} against {day_moonlit:?}"
    );
    assert!(
        night_moonlit[0] < night_moonlit[1] && night_moonlit[1] < night_moonlit[2],
        "{night_moonlit:?}"
    );
    for (night, day) in night_fire.into_iter().zip(day_fire) {
        assert!(
            night.abs_diff(day) <= 1,
            "the fire keeps its colour: {night_fire:?}, {day_fire:?}"
        );
    }
    // Dusk keeps its colour: the blue hour is the cones'.
    rig.renderer.set_sun_elevation(Some(-4.0));
    let dusk = rig.render_colour(moonlit, 4);
    assert!(saturation(dusk) > 0.8, "{dusk:?} against {day_moonlit:?}");
}

#[test]
fn the_rods_take_over_as_the_twilight_falls_under_a_full_moons_light() {
    assert_eq!(
        night_vision_weight(-4.0),
        0.0,
        "the blue hour is the cones'"
    );
    assert_eq!(night_vision_weight(30.0), 0.0);
    assert_eq!(
        night_vision_weight(-12.0),
        1.0,
        "nautical twilight's end is the rods'"
    );
    // The twilight a full moon's light, 0.27 lux, the sun 8.4° under.
    let full_moon = night_vision_weight(-8.4);
    assert!((0.8..0.95).contains(&full_moon), "{full_moon}");
    let mut last = 0.0;
    for tenth in 0..=200 {
        let weight = night_vision_weight(-tenth as f32 / 10.0);
        assert!(weight >= last, "it never falls as the sun sets");
        last = weight;
    }
}

#[test]
fn with_the_hosts_sun_the_eye_opens_at_a_storms_dusk() {
    let mut rig = Rig::new();
    let settings = EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    };
    rig.renderer.set_eye_adaptation(settings);
    // Under a storm's deck at dusk the clear sky's mean reads as dark as a night's (log2 −5.6): by its luminance the
    // eye stays shut over a view two stops under the sky.
    let (sky, clear) = (0.005, 0.02);
    rig.sky(sky, clear);
    let radiance = sky / 4.0;
    let cells = [radiance.log2(); 256];
    rig.renderer.snap_eye_adaptation();
    rig.render(radiance, 0, 1.0 / 60.0);
    assert_eq!(rig.eye_state()[2], 0.0, "{:?}", rig.eye_state());
    // With the sun 3° up it opens as far as it may, as the CPU twin has it.
    rig.renderer.set_sun_elevation(Some(3.0));
    rig.renderer.snap_eye_adaptation();
    rig.render(radiance, 1, 1.0 / 60.0);
    let state = rig.eye_state();
    let (stops, _, goal) = reference_adapt_in(
        &settings,
        &cells,
        sky,
        clear,
        Hour {
            elevation: Some(3.0),
            lift: 1.0,
            light: 0.0,
            night: 0.0,
            sun_away: 0.0,
        },
        0.0,
        1.0 / 60.0,
        true,
    );
    assert!(goal > 1.5, "{goal}");
    assert!((state[2] - goal).abs() < 1e-3, "{state:?} {goal}");
    assert!((state[0] - stops).abs() < 1e-3, "{state:?} {stops}");
}

/// After dark the eye closes for a view the lights fill as the CPU twin has it: a salvo beside the eye lights the view
/// 12 stops over a clear noon's highlights, and the eye closes by as much, the night's lift letting it.
#[test]
fn at_night_the_eye_closes_for_a_view_the_lights_fill_as_the_twin_does() {
    let mut rig = Rig::new();
    let settings = EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    };
    rig.renderer.set_eye_adaptation(settings);
    let (sky, clear) = (0.03, 0.03);
    rig.sky(sky, clear);
    let hour = Hour {
        elevation: Some(-28.0),
        lift: 16_700.0,
        light: naval_render::post::NIGHT_ADAPT_FROM,
        night: 1.0,
        sun_away: 0.0,
    };
    rig.renderer.set_sun_elevation(hour.elevation);
    rig.renderer.set_night_lift(hour.lift);
    rig.renderer.set_night_share(hour.night);
    let radiance = (NOON_CLEAR_SKY.log2() + settings.night_highlight + 12.0).exp2();
    let cells = [radiance.log2(); 256];
    rig.renderer.snap_eye_adaptation();
    rig.render(radiance, 0, 1.0 / 60.0);
    let state = rig.eye_state();
    let (stops, _, goal) =
        reference_adapt_in(&settings, &cells, sky, clear, hour, 0.0, 1.0 / 60.0, true);
    assert!(goal < -11.5, "{goal}");
    assert!((state[2] - goal).abs() < 1e-3, "{state:?} {goal}");
    assert!((state[0] - stops).abs() < 1e-3, "{state:?} {stops}");
    // Without the lift (a host that gives none) it closes by the day's stop at most.
    rig.renderer.set_night_lift(1.0);
    rig.renderer.snap_eye_adaptation();
    rig.render(radiance, 1, 1.0 / 60.0);
    assert!(
        (rig.eye_state()[2] + settings.max_down).abs() < 1e-3,
        "{:?}",
        rig.eye_state()
    );
    // A moonless night's dark-adapted eye opens a dim view by Weber's law with the dark light, as the twin does.
    rig.renderer.set_night_lift(hour.lift);
    rig.renderer.set_night_light(0.005);
    let dim = clear / 16.0;
    rig.renderer.snap_eye_adaptation();
    rig.render(dim, 2, 1.0 / 60.0);
    let (_, _, opened) = reference_adapt_in(
        &settings,
        &[dim.log2(); 256],
        sky,
        clear,
        Hour {
            light: 0.005,
            ..hour
        },
        0.0,
        1.0 / 60.0,
        true,
    );
    assert!(
        (opened - naval_render::post::night_open(0.005)).abs() < 1e-5,
        "{opened}"
    );
    assert!(
        (rig.eye_state()[2] - opened).abs() < 1e-3,
        "{:?}",
        rig.eye_state()
    );
    // Two stops closed (as after a lamp) over a darker view, and closed by a flash, it opens at the night's pace, as the
    // twin does: back from the flash, more than ten stops in a frame. The darker view, its sea alone, opens no further.
    let dark_hour = Hour {
        light: 0.005,
        ..hour
    };
    let darker = clear / 1024.0;
    let frame = 1.0 / 60.0;
    for (index, (from, view)) in [(opened - 2.0, darker), (-12.0, dim)]
        .into_iter()
        .enumerate()
    {
        rig.renderer
            .set_exposure_control(naval_render::post::ExposureControl::Fixed(from));
        rig.render(view, 3 + 2 * index as u64, frame);
        rig.renderer
            .set_exposure_control(naval_render::post::ExposureControl::Auto);
        rig.render(view, 4 + 2 * index as u64, frame);
        let (step, _, _) = reference_adapt_in(
            &settings,
            &[view.log2(); 256],
            sky,
            clear,
            dark_hour,
            from,
            frame as f32,
            false,
        );
        let state = rig.eye_state();
        assert!((state[0] - step).abs() < 1e-3, "{index}: {state:?} {step}");
        if index == 0 {
            assert!(step > from, "{step}");
            assert!((state[2] - opened).abs() < 1e-3, "{state:?} {opened}");
        } else {
            assert!(step > from + 10.0 && step <= 0.0, "{step}");
        }
    }
    // The eye off closes for a salvo's flash filling the view only toward the fixed exposure, as the twin has it.
    let off = EyeAdaptationSettings {
        enabled: false,
        dark_adapt: true,
        ..settings
    };
    rig.renderer.set_eye_adaptation(off);
    rig.renderer
        .set_exposure_control(naval_render::post::ExposureControl::Fixed(opened));
    rig.render(radiance, 7, frame);
    rig.renderer
        .set_exposure_control(naval_render::post::ExposureControl::Auto);
    rig.render(radiance, 8, frame);
    let (step, _, rest) = reference_adapt_in(
        &off,
        &cells,
        sky,
        clear,
        dark_hour,
        opened,
        frame as f32,
        false,
    );
    let state = rig.eye_state();
    assert_eq!(rest, 0.0);
    assert!((state[0] - step).abs() < 1e-3, "{state:?} {step}");
    assert!(step < opened && step > 0.0, "{step}");
}

/// What a transparent covers of the disc more than half (a rain streak's core, thick smoke) shows the transparent's
/// light, not the sky's: a sun hidden behind a deck under a bright streak no longer reads as a disc seen in full.
#[test]
fn a_transparent_over_the_disc_shows_no_sun() {
    let mut rig = Rig::new();
    rig.renderer.set_bloom(Some(BloomSettings::default()));
    // With a sky mean the night grade's cover pass runs: the scene's alpha holds what the transparents cover. A quarter
    // of a unit (log2 −2) is day: no night grade.
    rig.renderer.set_sky_mean(SkyMean::Rgb([0.25; 3]));
    let mut sky = SunSky::new(&mut rig.renderer.context());
    let camera = Camera::default();
    let forward = (camera.target - camera.eye).normalize();
    let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan();
    let radius_px = 6.0;
    rig.renderer.set_sun_glare(Some(FlareSun {
        direction: forward,
        irradiance: glam::Vec3::splat(6.0),
        angular_radius: 0.004675,
        drawn_radius: radius_px * 2.0 * tan / HEIGHT as f32,
        disc_radiance: glam::Vec3::splat(1.0),
    }));
    rig.gpu.queue.write_buffer(
        &sky.disc,
        0,
        bytemuck::cast_slice(&[WIDTH as f32 * 0.5, HEIGHT as f32 * 0.5, radius_px, 0.25]),
    );
    // The disc behind a deck (it adds nothing to the sky), a sheet over it covering `cover`: what the glare sees of it.
    let mut seen = |frame: u64, cover: f32| {
        rig.gpu.queue.write_buffer(
            &sky.front,
            0,
            bytemuck::cast_slice(&[0.0f32, 0.0, 0.0, cover]),
        );
        sky.cover.set(cover);
        rig.renderer.snap_eye_adaptation();
        rig.renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                &mut [&mut sky],
                RenderOutput {
                    view: &rig.capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .unwrap();
        rig.state(SUN_GLARE_BUFFER)[0]
    };
    assert!(seen(1, 0.0).abs() < 1e-3, "a hidden disc glares not");
    // Thin cover counts as the sky it lets through: its light, a quarter of the disc's, reads as a quarter of a disc.
    assert!((seen(2, 0.25) - 0.25).abs() < 0.02);
    // Over half: no sky is seen there, so no disc.
    assert!(seen(3, 0.75).abs() < 1e-3);
}
