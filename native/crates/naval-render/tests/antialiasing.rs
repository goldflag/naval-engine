//! Edge smoothing and the render scale on a real GPU: FXAA and SMAA give a slanted edge the
//! intermediate levels a plain frame lacks and leave flat regions alone, temporal AA's first
//! frame (no history anywhere) takes FXAA's smoothing, and below full render scale the scene is
//! drawn smaller and stretched onto the output while an overlay stays at full resolution.

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    Antialiasing, Camera, DrawContext, FrameInput, FrameReport, FrameTime, OverlayContext,
    OverlayPipeline, OverlayPipelineId, POST_FRAME, Phase, PipelineId, RenderContext, RenderOutput,
    Renderer, SceneLayer, ScenePipeline,
};

// A camera at the origin looking down −Z at a plane (z = −10), white left of the slanted line
// x = 0.3 y and black right of it. The overlay is one output column (`COLUMN`) of pure green.
const SHADER: &str = r#"
#import naval::camera::camera

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex fn background_vertex(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let xy = mix(vec2<f32>(-20.0), vec2<f32>(20.0), corners[i]);
    var out: Out;
    out.world = vec3<f32>(xy, -10.0);
    out.clip = camera.view_projection * vec4<f32>(out.world, 1.0);
    return out;
}
@fragment fn background_fragment(in: Out) -> @location(0) vec4<f32> {
    let white = select(0.0, 1.0, in.world.x < 0.3 * in.world.y);
    return vec4<f32>(vec3<f32>(white), 1.0);
}

struct Line {
    @builtin(position) clip: vec4<f32>,
}

// Output pixels [left, left + 1) across the whole height, for an output WIDTH wide.
@vertex fn overlay_vertex(@builtin(vertex_index) i: u32) -> Line {
    let left = 20.0 / 96.0 * 2.0 - 1.0;
    let right = 21.0 / 96.0 * 2.0 - 1.0;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(left, -1.0), vec2<f32>(right, -1.0), vec2<f32>(right, 1.0),
        vec2<f32>(left, -1.0), vec2<f32>(right, 1.0), vec2<f32>(left, 1.0),
    );
    var out: Line;
    out.clip = vec4<f32>(corners[i], 0.0, 1.0);
    return out;
}
@fragment fn overlay_fragment(in: Line) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
}
"#;

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;
const COLUMN: u32 = 20;

struct Layer {
    background: PipelineId,
    overlay: OverlayPipelineId,
    draw_overlay: bool,
}

impl Layer {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/aa.wgsl", SHADER).unwrap();
        let background = ctx
            .add_scene_pipeline(
                "background",
                "test/aa.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new("background_vertex", "background_fragment")
                },
            )
            .unwrap();
        let overlay = ctx
            .add_overlay_pipeline(
                "overlay test",
                "test/aa.wgsl",
                OverlayPipeline::new("overlay_vertex", "overlay_fragment"),
            )
            .unwrap();
        Self {
            background,
            overlay,
            draw_overlay: false,
        }
    }
}

impl SceneLayer for Layer {
    fn label(&self) -> &str {
        "antialiasing test"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.background));
            pass.draw(0..6, 0..1);
        }
    }

    fn has_overlay(&self) -> bool {
        self.draw_overlay
    }

    fn draw_overlay(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &OverlayContext<'_>) {
        assert_eq!(
            ctx.viewport,
            (WIDTH, HEIGHT),
            "overlays see the output's size"
        );
        pass.set_pipeline(ctx.pipeline(self.overlay));
        pass.draw(0..6, 0..1);
    }
}

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    layer: Layer,
    capture: Capture,
    report: FrameReport,
}

impl Rig {
    fn new(antialiasing: Antialiasing) -> Self {
        let gpu = Gpu::headless().expect("a GPU adapter");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let layer = Layer::new(&mut renderer.context());
        renderer.set_antialiasing(antialiasing);
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        Self {
            gpu,
            renderer,
            layer,
            capture,
            report: FrameReport::default(),
        }
    }

    fn render(&mut self, frame: u64) -> Vec<u8> {
        let camera = Camera::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
        self.report = self
            .renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                &mut [&mut self.layer],
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
}

fn red(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * WIDTH + x) * 4) as usize]
}

/// Pixels whose red is strictly between black and white, over the whole frame.
fn edge_levels(pixels: &[u8], white: u8) -> usize {
    let mut count = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let r = red(pixels, x, y);
            if r > 8 && r + 8 < white {
                count += 1;
            }
        }
    }
    count
}

const FLAT: [(u32, u32); 4] = [(1, 5), (2, 50), (WIDTH - 2, 5), (WIDTH - 3, 50)];

#[test]
fn fxaa_and_smaa_smooth_a_slanted_edge_and_leave_flat_regions_alone() {
    let plain = Rig::new(Antialiasing::None).render(0);
    let white = red(&plain, 1, 5);
    assert!(white > 150, "the white half: {white}");
    let aliased = edge_levels(&plain, white);
    for mode in [Antialiasing::Fxaa, Antialiasing::Smaa] {
        let mut rig = Rig::new(mode);
        let pixels = rig.render(0);
        let smoothed = edge_levels(&pixels, white);
        eprintln!("{mode:?}: {smoothed} intermediate pixels, {aliased} plain");
        assert!(
            smoothed >= 24 && smoothed > aliased * 3,
            "{mode:?}: the edge takes intermediate levels ({smoothed}) where the plain frame has {aliased}"
        );
        for (x, y) in FLAT {
            let i = ((y * WIDTH + x) * 4) as usize;
            assert_eq!(
                pixels[i..i + 3],
                plain[i..i + 3],
                "{mode:?}: flat pixel ({x}, {y})"
            );
        }
        assert_eq!(rig.report.viewport, (WIDTH, HEIGHT));
        assert!(rig.renderer.resources().texture(POST_FRAME).is_none());
    }
}

#[test]
fn temporal_aa_smooths_pixels_without_history_with_fxaa() {
    let plain = Rig::new(Antialiasing::None).render(0);
    let white = red(&plain, 1, 5);
    let aliased = edge_levels(&plain, white);
    // The first frame has no history anywhere, so the output pass smooths all of it.
    let mut rig = Rig::new(Antialiasing::Taa);
    let first = rig.render(0);
    assert!(!rig.renderer.taa_history_valid());
    let smoothed = edge_levels(&first, white);
    assert!(
        smoothed > aliased * 3,
        "the first temporal frame is FXAA'd ({smoothed} intermediate pixels, {aliased} plain)"
    );
}

#[test]
fn below_full_scale_the_scene_is_drawn_smaller_and_overlays_stay_sharp() {
    for mode in [Antialiasing::None, Antialiasing::Fxaa, Antialiasing::Taa] {
        let mut rig = Rig::new(mode);
        rig.renderer.set_render_scale(0.5);
        rig.layer.draw_overlay = true;
        let pixels = rig.render(0);
        assert_eq!(rig.report.viewport, (WIDTH / 2, HEIGHT / 2), "{mode:?}");
        // Temporal AA upscales in its own resolve, at the output's size.
        assert_eq!(
            rig.renderer.resources().texture(POST_FRAME).is_some(),
            mode == Antialiasing::Fxaa,
            "{mode:?}: only a frame stretched after FXAA or SMAA needs post.frame"
        );
        // The upscaled scene: white far left, black far right, the edge between.
        assert!(red(&pixels, 1, 5) > 150 && red(&pixels, WIDTH - 2, 5) < 8);
        // The overlay column is exactly green and its neighbours are the scene's.
        for y in [0, HEIGHT / 2, HEIGHT - 1] {
            let i = ((y * WIDTH + COLUMN) * 4) as usize;
            assert_eq!(
                pixels[i..i + 3],
                [0, 255, 0],
                "{mode:?}: overlay at row {y}"
            );
            for x in [COLUMN - 1, COLUMN + 1] {
                let i = ((y * WIDTH + x) * 4) as usize;
                assert_ne!(
                    pixels[i + 1],
                    255,
                    "{mode:?}: the overlay stays one pixel wide"
                );
            }
        }
    }
    // Back at full scale the frame is the plain one again.
    let mut rig = Rig::new(Antialiasing::None);
    let plain = rig.render(0);
    rig.renderer.set_render_scale(0.5);
    rig.render(1);
    rig.renderer.set_render_scale(1.0);
    // Within the output dither, which moves with the frame index: up to a level either way.
    let again = rig.render(2);
    assert_eq!(again.len(), plain.len());
    assert!(
        again.iter().zip(&plain).all(|(a, b)| a.abs_diff(*b) <= 2),
        "back at full scale the frame is the plain one, within the dither"
    );
    assert_eq!(rig.report.viewport, (WIDTH, HEIGHT));
}
