//! Renderer tests on a real (headless) GPU: the display transform against its
//! CPU twin, the reversed depth convention, and hot reload keeping the last
//! good pipeline.

use half::f16;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::display::reference;
use naval_render::{
    Camera, DisplaySettings, DrawContext, FrameInput, FrameTime, Phase, RenderContext,
    RenderOutput, Renderer, SceneLayer, ScenePipeline, TestScene,
};

/// Draws a full-screen triangle whose pixels read radiance from a storage
/// buffer, row-major, one value per pixel.
struct Radiances {
    pipeline: naval_render::PipelineId,
    group: wgpu::BindGroup,
}

const RADIANCE_SHADER: &str = r#"
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::camera::camera

@group(1) @binding(0) var<storage, read> values: array<vec4<f32>>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.5);
}

@fragment
fn fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let pixel = vec2<u32>(in.position.xy);
    return values[pixel.y * u32(camera.viewport.x) + pixel.x];
}
"#;

impl Radiances {
    fn new(ctx: &mut RenderContext<'_>, values: &[[f32; 4]]) -> Self {
        ctx.library
            .add_source("test/radiances.wgsl", RADIANCE_SHADER)
            .unwrap();
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("radiances"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let buffer = {
            use wgpu::util::DeviceExt;
            ctx.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("radiances"),
                    contents: bytemuck::cast_slice(values),
                    usage: wgpu::BufferUsages::STORAGE,
                })
        };
        let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("radiances"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        let pipeline = ctx
            .add_scene_pipeline(
                "radiances",
                "test/radiances.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    bind_group_layouts: vec![layout],
                    ..ScenePipeline::new("vertex", "fragment")
                },
            )
            .expect("radiance pipeline");
        Self { pipeline, group }
    }
}

impl SceneLayer for Radiances {
    fn label(&self) -> &str {
        "radiances"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.set_bind_group(1, &self.group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

const WIDTH: u32 = 8;
const HEIGHT: u32 = 4;

fn radiances() -> Vec<[f32; 4]> {
    let mut values: Vec<[f32; 4]> = [
        0.0, 0.001, 0.01, 0.05, 0.18, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 100.0,
    ]
    .iter()
    .map(|&v| [v, v, v, 1.0])
    .collect();
    values.extend([
        [1.0, 0.0, 0.0, 1.0],
        [0.0, 1.0, 0.0, 1.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.8, 0.3, 0.1, 1.0],
        [0.1, 0.4, 0.9, 1.0],
        [20.0, 18.0, 15.0, 1.0],
        [0.38, 0.46, 0.58, 1.0],
        [0.006, 0.022, 0.04, 1.0],
        [0.42, 0.42, 0.45, 1.0],
        [3.0, 0.2, 0.05, 1.0],
    ]);
    while values.len() < (WIDTH * HEIGHT) as usize {
        let i = values.len() as f32;
        values.push([0.03 * i, 0.5 / i, (i * 0.37).fract() * 2.0, 1.0]);
    }
    values
}

/// Renders the radiances through the display pass into `format` and returns
/// the read-back RGBA8.
fn display(
    gpu: &Gpu,
    format: wgpu::TextureFormat,
    settings: DisplaySettings,
    values: &[[f32; 4]],
) -> Vec<u8> {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, format).expect("renderer");
    renderer.set_display_settings(settings);
    let mut layer = Radiances::new(&mut renderer.context(), values);
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("display test"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let capture = Capture {
        view: texture.create_view(&Default::default()),
        texture,
        width: WIDTH,
        height: HEIGHT,
    };
    let camera = Camera::default();
    renderer
        .render(
            FrameInput {
                camera: &camera,
                time: FrameTime::default(),
            },
            &mut [&mut layer],
            RenderOutput {
                view: &capture.view,
                format,
                width: WIDTH,
                height: HEIGHT,
            },
        )
        .expect("frame");
    capture.read_rgba(gpu).expect("read-back")
}

fn assert_matches_reference(pixels: &[u8], values: &[[f32; 4]], settings: DisplaySettings) {
    let mut worst = 0;
    for (i, value) in values.iter().enumerate() {
        // The scene target is half float; the reference sees what it stored.
        let stored = [value[0], value[1], value[2]].map(|c| f16::from_f32(c).to_f32());
        let expected = reference::display_srgb8(stored, settings);
        let got = &pixels[i * 4..i * 4 + 3];
        for channel in 0..3 {
            let diff = (i32::from(got[channel]) - i32::from(expected[channel])).abs();
            worst = worst.max(diff);
            assert!(
                diff <= 1,
                "radiance {value:?}: GPU {got:?}, reference {expected:?} (channel {channel})"
            );
        }
        assert_eq!(pixels[i * 4 + 3], 255, "opaque output");
    }
    eprintln!("worst channel difference: {worst}");
}

#[test]
fn display_pass_matches_the_cpu_agx_reference() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let values = radiances();
    let settings = DisplaySettings::default();
    let pixels = display(&gpu, CAPTURE_FORMAT, settings, &values);
    assert_matches_reference(&pixels, &values, settings);
}

#[test]
fn display_pass_encodes_srgb_itself_for_a_linear_target() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let values = radiances();
    let settings = DisplaySettings {
        exposure: 0.5,
        tone_mapping_exposure: 1.0,
    };
    let pixels = display(&gpu, wgpu::TextureFormat::Rgba8Unorm, settings, &values);
    assert_matches_reference(&pixels, &values, settings);
}

#[test]
fn agx_reference_holds_its_known_points() {
    // Black stays near black, middle gray lands mid-scale, a huge radiance
    // rolls off to white rather than clipping a channel alone.
    let settings = DisplaySettings::default();
    assert!(
        reference::display_srgb8([0.0; 3], settings)
            .iter()
            .all(|&c| c < 8)
    );
    let gray = reference::display_srgb8([0.18; 3], settings);
    assert!((90..170).contains(&gray[1]), "middle gray {gray:?}");
    assert_eq!(gray[0], gray[1]);
    let white = reference::display_srgb8([1000.0; 3], settings);
    assert!(white.iter().all(|&c| c >= 250), "white {white:?}");
    let sun = reference::display_srgb8([20.0, 18.0, 15.0], settings);
    assert!(sun[0] >= sun[2], "a warm highlight stays warm {sun:?}");
}

/// Two quads in the opposite of depth order: the nearer must win under
/// reversed depth, and the sky must fill only what neither covers.
struct DepthOrder {
    pipeline: naval_render::PipelineId,
    sky: naval_render::PipelineId,
}

const DEPTH_SHADER: &str = r#"
struct Out {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

// Instance 0: near (clip depth 0.8), red, the left half of the middle.
// Instance 1: far (0.2), green, the whole middle. Near is drawn first, so only
// the depth test keeps red in front.
@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) quad: u32) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[index];
    var out: Out;
    if quad == 0u {
        out.position = vec4<f32>(mix(-0.5, 0.0, c.x), mix(-0.5, 0.5, c.y), 0.8, 1.0);
        out.color = vec3<f32>(1.0, 0.0, 0.0);
    } else {
        out.position = vec4<f32>(mix(-0.5, 0.5, c.x), mix(-0.5, 0.5, c.y), 0.2, 1.0);
        out.color = vec3<f32>(0.0, 1.0, 0.0);
    }
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}

@vertex
fn sky_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn sky_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 1.0, 1.0);
}
"#;

impl SceneLayer for DepthOrder {
    fn label(&self) -> &str {
        "depth order"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        match phase {
            Phase::Opaque => {
                pass.set_pipeline(ctx.pipelines.render(self.pipeline));
                pass.draw(0..6, 0..1); // near red first
                pass.draw(0..6, 1..2); // then far green
            }
            Phase::Sky => {
                pass.set_pipeline(ctx.pipelines.render(self.sky));
                pass.draw(0..3, 0..1);
            }
            Phase::Water | Phase::Transparent => {}
        }
    }
}

#[test]
fn reversed_depth_keeps_the_nearer_surface_and_the_sky_behind() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (32, 16);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    let mut layer = {
        let mut ctx = renderer.context();
        ctx.library
            .add_source("test/depth.wgsl", DEPTH_SHADER)
            .unwrap();
        let pipeline = ctx
            .add_scene_pipeline(
                "depth order",
                "test/depth.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new("vertex", "fragment")
                },
            )
            .unwrap();
        let sky = ctx
            .add_scene_pipeline(
                "depth sky",
                "test/depth.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    depth_write: false,
                    ..ScenePipeline::new("sky_vertex", "sky_fragment")
                },
            )
            .unwrap();
        DepthOrder { pipeline, sky }
    };
    let capture = Capture::new(&gpu, width, height);
    renderer
        .render(
            FrameInput {
                camera: &Camera::default(),
                time: FrameTime::default(),
            },
            &mut [&mut layer],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width,
                height,
            },
        )
        .unwrap();
    let pixels = capture.read_rgba(&gpu).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * width + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    let dominant = |p: [u8; 3]| (0..3).max_by_key(|&c| p[c]).unwrap();
    assert_eq!(
        dominant(at(10, 8)),
        0,
        "near red in front of far green: {:?}",
        at(10, 8)
    );
    assert_eq!(
        dominant(at(20, 8)),
        1,
        "far green where red is not: {:?}",
        at(20, 8)
    );
    assert_eq!(
        dominant(at(2, 2)),
        2,
        "sky where nothing is: {:?}",
        at(2, 2)
    );
}

fn render_test_scene(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &mut TestScene,
    capture: &Capture,
) -> Vec<u8> {
    renderer
        .render(
            FrameInput {
                camera: &TestScene::default_camera(),
                time: FrameTime::fixed(60, 1.0 / 60.0),
            },
            &mut [scene],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: capture.width,
                height: capture.height,
            },
        )
        .unwrap();
    capture.read_rgba(gpu).unwrap()
}

#[test]
fn hot_reload_rebuilds_affected_pipelines_and_keeps_the_last_good_one() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (64, 36);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let before = render_test_scene(&gpu, &mut renderer, &mut scene, &capture);
    let sky_path = naval_render::test_scene::files::SKY.path;

    // A broken edit: the sky keeps its last good pipeline and the frame is unchanged.
    let report = renderer.apply_shader_changes(vec![(
        sky_path.to_owned(),
        "@fragment fn fragment( {".to_owned(),
    )]);
    assert_eq!(report.changed, vec![sky_path.to_owned()]);
    assert!(report.pipelines.rebuilt.is_empty());
    assert_eq!(report.pipelines.failed.len(), 1, "{report:?}");
    assert_eq!(report.pipelines.failed[0].0, "test scene sky");
    let status = renderer.pipeline_status();
    assert!(
        status
            .iter()
            .any(|s| s.label == "test scene sky" && s.error.is_some())
    );
    assert_eq!(
        render_test_scene(&gpu, &mut renderer, &mut scene, &capture),
        before
    );

    // A good edit: only the sky rebuilds, and the top of the frame turns red.
    let red_sky = naval_render::test_scene::files::SKY.source.replace(
        "return vec4<f32>(sky_radiance(",
        "return vec4<f32>(vec3<f32>(0.8, 0.0, 0.0) + 0.0 * sky_radiance(",
    );
    let report = renderer.apply_shader_changes(vec![(sky_path.to_owned(), red_sky)]);
    assert_eq!(
        report.pipelines.rebuilt,
        vec!["test scene sky".to_owned()],
        "{report:?}"
    );
    assert!(report.pipelines.failed.is_empty());
    let after = render_test_scene(&gpu, &mut renderer, &mut scene, &capture);
    let top = &after[(width as usize * 2 + width as usize / 2) * 4..][..3];
    assert!(
        top[0] > top[1] + 80 && top[0] > top[2] + 80,
        "red sky at the top: {top:?}"
    );

    // An edit to the shared module rebuilds every pipeline that imports it.
    let common_path = naval_render::test_scene::files::COMMON.path;
    let brighter = naval_render::test_scene::files::COMMON.source.replace(
        "SUN_RADIANCE: vec3<f32> = vec3<f32>(2.6, 2.4, 2.1)",
        "SUN_RADIANCE: vec3<f32> = vec3<f32>(3.0, 2.4, 2.1)",
    );
    let report = renderer.apply_shader_changes(vec![(common_path.to_owned(), brighter)]);
    let mut rebuilt = report.pipelines.rebuilt.clone();
    rebuilt.sort();
    assert_eq!(
        rebuilt,
        [
            "test scene box",
            "test scene box caster",
            "test scene box motion",
            "test scene grid",
            "test scene sky"
        ],
        "{report:?}"
    );
}

#[test]
fn scene_targets_follow_the_output_size() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, 64, 36, CAPTURE_FORMAT).expect("renderer");
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let hdr_version = renderer
        .resources()
        .texture(naval_render::SCENE_HDR)
        .unwrap()
        .version;
    let capture = Capture::new(&gpu, 40, 30);
    render_test_scene(&gpu, &mut renderer, &mut scene, &capture);
    let hdr = renderer
        .resources()
        .texture(naval_render::SCENE_HDR)
        .unwrap();
    assert_eq!(hdr.desc.size, (40, 30));
    assert_ne!(hdr.version, hdr_version, "remade at the new size");
    assert_eq!(
        renderer
            .resources()
            .texture(naval_render::SCENE_DEPTH)
            .unwrap()
            .desc
            .size,
        (40, 30)
    );
}
