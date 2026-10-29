//! Temporal anti-aliasing on a real GPU: a slanted edge converges to sub-pixel
//! coverage that a single frame lacks, the motion pass writes a moving surface's
//! motion only where it is the nearest thing drawn, water that asks for no
//! history takes none and holds still as the jitter steps, and rendering one frame
//! again gives the same image.

use crate::common;

use common::{read_texture, texel};
use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    Camera, DrawContext, FrameInput, FrameTime, MotionPipeline, Phase, PipelineId, RenderContext,
    RenderOutput, Renderer, SCENE_MOTION, SceneLayer, ScenePipeline, TaaSettings,
};

// A camera at the origin looking down −Z. The background plane (z = −10) is white left of the
// slanted line x = 0.3 y and black right of it; a mover quad at z = −5; a water strip at z = −8
// along the bottom.
const SHADER: &str = r#"
#import naval::camera::camera
#import naval::motion::motion_response

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

fn quad(i: u32, low: vec2<f32>, high: vec2<f32>, z: f32) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let xy = mix(low, high, corners[i]);
    var out: Out;
    out.world = vec3<f32>(xy, z);
    out.clip = camera.view_projection * vec4<f32>(out.world, 1.0);
    return out;
}

@vertex fn background_vertex(@builtin(vertex_index) i: u32) -> Out {
    return quad(i, vec2<f32>(-20.0, -20.0), vec2<f32>(20.0, 20.0), -10.0);
}
@fragment fn background_fragment(in: Out) -> @location(0) vec4<f32> {
    let white = select(0.0, 1.0, in.world.x < 0.3 * in.world.y);
    return vec4<f32>(vec3<f32>(white), 1.0);
}

@vertex fn mover_vertex(@builtin(vertex_index) i: u32) -> Out {
    return quad(i, vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), -5.0);
}
@fragment fn mover_fragment(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(0.2, 0.3, 0.4, 1.0);
}
@fragment fn mover_motion(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(0.1, -0.05, 0.0, 1.0);
}

@vertex fn water_vertex(@builtin(vertex_index) i: u32) -> Out {
    return quad(i, vec2<f32>(-20.0, -5.0), vec2<f32>(20.0, -3.2), -8.0);
}
// Smooth waves, about 17 pixels long.
@fragment fn water_fragment(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(0.25 + 0.2 * sin(in.world.x * 3.0)), 1.0);
}
@fragment fn water_motion(in: Out) -> @location(0) vec4<f32> {
    return motion_response(1.0);
}
"#;

struct Layer {
    background: PipelineId,
    mover: PipelineId,
    water: PipelineId,
    mover_motion: PipelineId,
    water_motion: PipelineId,
}

impl Layer {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/taa.wgsl", SHADER).unwrap();
        let mut scene = |label: &str, vertex, fragment| {
            ctx.add_scene_pipeline(
                label,
                "test/taa.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new(vertex, fragment)
                },
            )
            .unwrap()
        };
        let background = scene("background", "background_vertex", "background_fragment");
        let mover = scene("mover", "mover_vertex", "mover_fragment");
        let water = scene("water", "water_vertex", "water_fragment");
        let mut motion = |label: &str, vertex, fragment| {
            ctx.add_motion_pipeline(
                label,
                "test/taa.wgsl",
                MotionPipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..MotionPipeline::new(vertex, fragment)
                },
            )
            .unwrap()
        };
        let mover_motion = motion("mover motion", "mover_vertex", "mover_motion");
        let water_motion = motion("water motion", "water_vertex", "water_motion");
        Self {
            background,
            mover,
            water,
            mover_motion,
            water_motion,
        }
    }
}

impl SceneLayer for Layer {
    fn label(&self) -> &str {
        "taa test"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let pipelines: &[PipelineId] = match phase {
            Phase::Opaque => &[self.background, self.mover],
            Phase::Water => &[self.water],
            _ => &[],
        };
        for &pipeline in pipelines {
            pass.set_pipeline(ctx.pipelines.render(pipeline));
            pass.draw(0..6, 0..1);
        }
    }

    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        for pipeline in [self.mover_motion, self.water_motion] {
            pass.set_pipeline(ctx.pipelines.render(pipeline));
            pass.draw(0..6, 0..1);
        }
    }
}

const WIDTH: u32 = 96;
const HEIGHT: u32 = 64;

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    layer: Layer,
    capture: Capture,
}

impl Rig {
    fn new(taa: bool) -> Self {
        let gpu = Gpu::headless().expect("a GPU adapter");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let layer = Layer::new(&mut renderer.context());
        renderer.set_taa(taa.then(TaaSettings::default));
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        Self {
            gpu,
            renderer,
            layer,
            capture,
        }
    }

    fn render(&mut self, frame: u64) -> Vec<u8> {
        self.render_from(
            frame,
            &Camera::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0)),
        )
    }

    fn render_from(&mut self, frame: u64, camera: &Camera) -> Vec<u8> {
        self.renderer
            .render(
                FrameInput {
                    camera,
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

/// Pixels of the background's upper band (away from the mover and the water) whose red is
/// strictly between black and the white's level.
fn edge_levels(pixels: &[u8], white: u8) -> usize {
    let mut count = 0;
    for y in 2..16 {
        for x in 0..WIDTH {
            let r = pixels[((y * WIDTH + x) * 4) as usize];
            if r > 8 && r + 8 < white {
                count += 1;
            }
        }
    }
    count
}

#[test]
fn a_slanted_edge_converges_to_sub_pixel_coverage() {
    let mut plain = Rig::new(false);
    let single = plain.render(0);
    let white = single[((5 * WIDTH + 1) * 4) as usize];
    assert!(white > 150, "the white half: {white}");
    let aliased = edge_levels(&single, white);

    let mut rig = Rig::new(true);
    let mut pixels = Vec::new();
    for frame in 0..24 {
        pixels = rig.render(frame);
    }
    assert!(rig.renderer.taa_history_valid());
    let smoothed = edge_levels(&pixels, white);
    assert!(
        smoothed >= 8 && smoothed > aliased * 3,
        "a 14-row edge takes intermediate levels with temporal AA ({smoothed}) where one frame has {aliased}"
    );
    // Away from the edge nothing blurs: the flat halves keep their levels exactly.
    for (x, y) in [(1, 5), (2, 12), (WIDTH - 2, 5), (WIDTH - 3, 12)] {
        let i = ((y * WIDTH + x) * 4) as usize;
        assert_eq!(pixels[i..i + 3], single[i..i + 3], "flat pixel ({x}, {y})");
    }
    // Rendering the last frame again resolves against the same history.
    assert_eq!(
        rig.render(23),
        pixels,
        "a repeated frame gives the same image"
    );
}

/// Turned off, temporal AA leaves none of its targets behind; turned on again it makes them and starts without history.
#[test]
fn temporal_aa_off_frees_its_targets() {
    const TARGETS: [&str; 4] = [
        "scene.motion",
        "display.frame",
        "taa.resolved",
        "taa.history",
    ];
    let mut rig = Rig::new(true);
    for frame in 0..4 {
        rig.render(frame);
    }
    for name in TARGETS {
        assert!(rig.renderer.resources().texture(name).is_some(), "{name}");
    }
    rig.renderer
        .set_antialiasing(naval_render::Antialiasing::None);
    rig.render(4);
    for name in TARGETS {
        assert!(rig.renderer.resources().texture(name).is_none(), "{name}");
    }
    rig.renderer
        .set_antialiasing(naval_render::Antialiasing::Taa);
    rig.render(5);
    assert!(
        !rig.renderer.taa_history_valid(),
        "a new history starts empty"
    );
    for name in TARGETS {
        assert!(rig.renderer.resources().texture(name).is_some(), "{name}");
    }
}

/// A cut on a frame drawn again (a capture from a camera placed elsewhere) drops the history
/// for every render of that frame after it, not only the first: drawn again, the same image.
#[test]
fn a_cut_on_a_frame_drawn_again_keeps_no_history() {
    let mut rig = Rig::new(true);
    for frame in 0..24 {
        rig.render(frame);
    }
    assert!(rig.renderer.taa_history_valid());
    let zoomed = Camera {
        fov_y_degrees: 20.0,
        ..Camera::look_at(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0))
    };
    let cut = rig.render_from(23, &zoomed);
    assert!(
        !rig.renderer.taa_history_valid(),
        "the cut drops the history"
    );
    assert_eq!(
        rig.render_from(23, &zoomed),
        cut,
        "drawn again, the same image"
    );
}

#[test]
fn motion_lands_where_a_surface_is_nearest_and_water_takes_no_history() {
    let mut rig = Rig::new(true);
    for frame in 0..4 {
        rig.render(frame);
    }
    let resources = rig.renderer.resources();
    let motion = read_texture(&rig.gpu, &resources.texture(SCENE_MOTION).unwrap().texture);
    let at = |x, y| texel(&motion, WIDTH, 4, x, y);
    let centre = at(WIDTH / 2, HEIGHT / 2);
    assert!(
        (centre[0] - 0.1).abs() < 1e-3 && (centre[1] + 0.05).abs() < 1e-3 && centre[3] == 1.0,
        "the mover's motion where it is nearest: {centre:?}"
    );
    assert_eq!(at(4, 4), [0.0; 4], "static background writes nothing");
    let water = at(WIDTH / 2, HEIGHT - 3);
    assert_eq!(water, [0.0, 0.0, 1.0, 0.0], "water adds its response only");

    let resolved = read_texture(
        &rig.gpu,
        &resources.texture("taa.resolved").unwrap().texture,
    );
    let alpha = |x, y| texel(&resolved, WIDTH, 4, x, y)[3];
    assert!(
        alpha(4, 4) > 0.0,
        "the still background takes history: {}",
        alpha(4, 4)
    );
    assert!(
        alpha(WIDTH / 2, HEIGHT - 3) < 0.0,
        "water takes none: {}",
        alpha(WIDTH / 2, HEIGHT - 3)
    );
    assert!(
        (alpha(4, 4) - 10.0).abs() < 0.5,
        "alpha is view depth: {}",
        alpha(4, 4)
    );
}

#[test]
fn water_without_history_holds_still_as_the_jitter_steps() {
    let mut rig = Rig::new(true);
    // The frame the resolve reads (jittered) and what it writes, over the water's rows, frame by
    // frame through the jitter's sixteen offsets.
    let mut frames: Vec<(Vec<f32>, Vec<f32>)> = Vec::new();
    for frame in 0..17 {
        rig.render(frame);
        let resources = rig.renderer.resources();
        let read = |name: &str| read_texture(&rig.gpu, &resources.texture(name).unwrap().texture);
        frames.push((read("display.frame"), read("taa.resolved")));
    }
    type Frames = (Vec<f32>, Vec<f32>);
    let change = |pick: fn(&Frames) -> &Vec<f32>| -> f32 {
        let mut total = 0.0;
        for pair in frames.windows(2) {
            let (a, b) = (pick(&pair[0]), pick(&pair[1]));
            for y in HEIGHT - 5..HEIGHT - 1 {
                for x in 4..WIDTH - 4 {
                    total += (texel(a, WIDTH, 4, x, y)[0] - texel(b, WIDTH, 4, x, y)[0]).abs();
                }
            }
        }
        total
    };
    let jittered = change(|f| &f.0);
    let resolved = change(|f| &f.1);
    assert!(jittered > 0.0, "the jitter moves the frame");
    assert!(
        resolved < jittered * 0.2,
        "the water's pixels are drawn back from the jitter: they change {resolved} frame to frame where the \
         jittered frame changes {jittered}"
    );
}

/// Mean difference of the red channel from `reference` over the background's upper band (the slanted edge, away from
/// the mover and the water), in 8-bit levels.
fn band_difference(pixels: &[u8], reference: &[u8]) -> f32 {
    let mut total = 0.0;
    let mut count = 0.0;
    for y in 2..16 {
        for x in 0..WIDTH {
            let i = ((y * WIDTH + x) * 4) as usize;
            total += (f32::from(pixels[i]) - f32::from(reference[i])).abs();
            count += 1.0;
        }
    }
    total / count
}

/// Below full render scale temporal AA resolves at the output's size from the smaller, jittered scene (temporal
/// upscaling): a slanted edge converges nearer the full-scale picture than FXAA's frame stretched bilinearly does, the
/// flat halves keep their levels, the resolve and its history are at the output's size, and a frame drawn again gives
/// the same image.
#[test]
fn an_upscaled_slanted_edge_converges_nearer_the_full_scale_picture() {
    // More frames than the 32 phases the jitter takes at half scale.
    const FRAMES: u64 = 48;
    let mut full = Rig::new(true);
    let mut reference = Vec::new();
    for frame in 0..FRAMES {
        reference = full.render(frame);
    }

    let mut stretched = Rig::new(false);
    stretched
        .renderer
        .set_antialiasing(naval_render::Antialiasing::Fxaa);
    stretched.renderer.set_render_scale(0.5);
    let bilinear = stretched.render(FRAMES - 1);

    let mut rig = Rig::new(true);
    rig.renderer.set_render_scale(0.5);
    let mut pixels = Vec::new();
    for frame in 0..FRAMES {
        pixels = rig.render(frame);
    }
    assert!(rig.renderer.taa_history_valid());
    let resources = rig.renderer.resources();
    assert_eq!(resources.viewport(), (WIDTH / 2, HEIGHT / 2));
    for name in ["taa.resolved", "taa.history"] {
        let size = resources.texture(name).unwrap().texture.size();
        assert_eq!((size.width, size.height), (WIDTH, HEIGHT), "{name}");
    }
    assert!(
        resources.texture(naval_render::POST_FRAME).is_none(),
        "no bilinear stretch after the resolve"
    );

    let upscaled = band_difference(&pixels, &reference);
    let stretched_difference = band_difference(&bilinear, &reference);
    eprintln!(
        "from the full-scale picture: temporal upscale {upscaled:.2} levels, FXAA stretched {stretched_difference:.2}"
    );
    assert!(
        upscaled < stretched_difference * 0.75,
        "the upscaled edge ({upscaled:.2} levels off) lies nearer the full-scale one than the stretch ({stretched_difference:.2})"
    );
    for (x, y) in [(1, 5), (2, 12), (WIDTH - 2, 5), (WIDTH - 3, 12)] {
        let i = ((y * WIDTH + x) * 4) as usize;
        for c in 0..3 {
            assert!(
                pixels[i + c].abs_diff(reference[i + c]) <= 1,
                "flat pixel ({x}, {y}): {:?} against {:?}",
                &pixels[i..i + 3],
                &reference[i..i + 3]
            );
        }
    }
    assert_eq!(
        rig.render(FRAMES - 1),
        pixels,
        "a repeated frame gives the same image"
    );
}
