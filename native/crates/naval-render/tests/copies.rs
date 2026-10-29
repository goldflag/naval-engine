//! Scene copies on a real GPU: the opaque copy holds only the opaque phase, the
//! backdrop copy adds water and sky, and a frame nobody asks copies of stays one pass; a frame finished in parts on
//! other threads (`naval_render::finish`) draws the same.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::common;

use common::{assert_near, read_texture, texel};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::copies::{color_copy_layout_entry, depth_copy_layout_entry};
use naval_render::{
    Camera, CopyPoint, DrawContext, Finisher, FrameInput, FrameTime, Part, Phase, PipelineId,
    PrepareContext, RenderContext, RenderOutput, Renderer, SCENE_BACKDROP_COLOR,
    SCENE_BACKDROP_DEPTH, SCENE_HDR, SCENE_OPAQUE_COLOR, SCENE_OPAQUE_DEPTH, SceneCopies,
    SceneLayer, ScenePipeline,
};

// 32 x 16 pixels. Opaque: a red quad over the left half at depth 0.8. Water: the
// bottom half at depth 0.9 (in front), writing the opaque copy's red and depth.
// Sky: blue wherever nothing wrote depth. Transparent: the right quarter at 0.95,
// writing the backdrop copy's blue and depth.
const SHADER: &str = r#"
@group(1) @binding(0) var opaque_color: texture_2d<f32>;
@group(1) @binding(1) var opaque_depth: texture_depth_2d;
@group(1) @binding(2) var backdrop_color: texture_2d<f32>;
@group(1) @binding(3) var backdrop_depth: texture_depth_2d;

fn quad(index: u32, low: vec2<f32>, high: vec2<f32>, depth: f32) -> vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    return vec4<f32>(mix(low, high, corners[index]), depth, 1.0);
}

@vertex fn opaque_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return quad(i, vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, 1.0), 0.8);
}
@fragment fn opaque_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}

@vertex fn water_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return quad(i, vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 0.0), 0.9);
}
@fragment fn water_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    return vec4<f32>(textureLoad(opaque_color, pixel, 0).r, textureLoad(opaque_depth, pixel, 0), 0.5, 1.0);
}

@vertex fn sky_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return quad(i, vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), 0.0);
}
@fragment fn sky_fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 0.0, 1.0, 1.0);
}

@vertex fn transparent_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return quad(i, vec2<f32>(0.5, -1.0), vec2<f32>(1.0, 1.0), 0.95);
}
@fragment fn transparent_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    return vec4<f32>(textureLoad(backdrop_color, pixel, 0).b, textureLoad(backdrop_depth, pixel, 0), 0.25, 1.0);
}
"#;

struct Layer {
    opaque: PipelineId,
    water: PipelineId,
    sky: PipelineId,
    transparent: PipelineId,
    layout: wgpu::BindGroupLayout,
    group: Option<(u64, wgpu::BindGroup)>,
    asks: SceneCopies,
    hooks: Vec<CopyPoint>,
}

impl Layer {
    fn new(ctx: &mut RenderContext<'_>, asks: SceneCopies) -> Self {
        ctx.library.add_source("test/copies.wgsl", SHADER).unwrap();
        let visibility = wgpu::ShaderStages::FRAGMENT;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("copies"),
                entries: &[
                    color_copy_layout_entry(0, visibility),
                    depth_copy_layout_entry(1, visibility),
                    color_copy_layout_entry(2, visibility),
                    depth_copy_layout_entry(3, visibility),
                ],
            });
        let mut pipeline = |label: &str, entry: &'static str, write: bool, groups: bool| {
            let (vertex, fragment) = match entry {
                "opaque" => ("opaque_vertex", "opaque_fragment"),
                "water" => ("water_vertex", "water_fragment"),
                "sky" => ("sky_vertex", "sky_fragment"),
                _ => ("transparent_vertex", "transparent_fragment"),
            };
            ctx.add_scene_pipeline(
                label,
                "test/copies.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    depth_write: write,
                    bind_group_layouts: if groups { vec![layout.clone()] } else { vec![] },
                    ..ScenePipeline::new(vertex, fragment)
                },
            )
            .unwrap()
        };
        let opaque = pipeline("opaque", "opaque", true, false);
        let water = pipeline("water", "water", true, true);
        let sky = pipeline("sky", "sky", false, false);
        let transparent = pipeline("transparent", "transparent", false, true);
        // The layer binds all four copies, so it asks for their textures up front.
        ctx.request_scene_copies(SceneCopies::ALL);
        Self {
            opaque,
            water,
            sky,
            transparent,
            layout,
            group: None,
            asks,
            hooks: Vec::new(),
        }
    }
}

impl SceneLayer for Layer {
    fn label(&self) -> &str {
        "copies"
    }

    fn scene_copies(&self) -> SceneCopies {
        self.asks
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let names = [
            SCENE_OPAQUE_COLOR,
            SCENE_OPAQUE_DEPTH,
            SCENE_BACKDROP_COLOR,
            SCENE_BACKDROP_DEPTH,
        ];
        let entries: Vec<_> = names
            .iter()
            .map(|name| ctx.resources.texture(name).expect("requested"))
            .collect();
        let version = entries.iter().map(|entry| entry.version).max().unwrap();
        if self.group.as_ref().is_none_or(|(v, _)| *v != version) {
            let bindings: Vec<_> = entries
                .iter()
                .enumerate()
                .map(|(binding, entry)| wgpu::BindGroupEntry {
                    binding: binding as u32,
                    resource: wgpu::BindingResource::TextureView(&entry.view),
                })
                .collect();
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("copies"),
                layout: &self.layout,
                entries: &bindings,
            });
            self.group = Some((version, group));
        }
    }

    fn after_copies(&mut self, point: CopyPoint, _ctx: &mut PrepareContext<'_>) {
        self.hooks.push(point);
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        let (_, group) = self.group.as_ref().expect("made in prepare");
        let pipeline = match phase {
            Phase::Opaque => self.opaque,
            Phase::Water => self.water,
            Phase::Sky => self.sky,
            Phase::Transparent => self.transparent,
        };
        pass.set_pipeline(ctx.pipelines.render(pipeline));
        if matches!(phase, Phase::Water | Phase::Transparent) {
            pass.set_bind_group(1, group, &[]);
        }
        pass.draw(0..6, 0..1);
    }
}

const WIDTH: u32 = 32;
const HEIGHT: u32 = 16;

fn render(asks: SceneCopies) -> (Gpu, Renderer, Layer, naval_render::FrameReport) {
    render_finishing(asks, None)
}

fn render_finishing(
    asks: SceneCopies,
    finisher: Option<Arc<dyn Finisher>>,
) -> (Gpu, Renderer, Layer, naval_render::FrameReport) {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
    renderer.set_finisher(finisher);
    let mut layer = Layer::new(&mut renderer.context(), asks);
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let report = renderer
        .render(
            FrameInput {
                camera: &Camera::default(),
                time: FrameTime::default(),
            },
            &mut [&mut layer],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: WIDTH,
                height: HEIGHT,
            },
        )
        .unwrap();
    (gpu, renderer, layer, report)
}

#[test]
fn copies_hold_what_the_phases_before_them_drew() {
    let (gpu, renderer, layer, report) = render(SceneCopies::ALL);
    assert_eq!(report.copies, SceneCopies::ALL);
    assert_eq!(
        report.scene_passes,
        ["scene opaque", "scene water, sky", "scene transparent"]
    );
    assert_eq!(layer.hooks, [CopyPoint::Opaque, CopyPoint::Backdrop]);

    let hdr = read_texture(
        &gpu,
        &renderer.resources().texture(SCENE_HDR).unwrap().texture,
    );
    let at = |x, y| texel(&hdr, WIDTH, 4, x, y)[..3].to_vec();
    let tolerance = 2e-3;
    assert_near(&at(4, 4), &[1.0, 0.0, 0.0], tolerance, "the red quad");
    assert_near(
        &at(4, 12),
        &[1.0, 0.8, 0.5],
        tolerance,
        "water over the quad reads its color and depth from the opaque copy",
    );
    assert_near(&at(20, 4), &[0.0, 0.0, 1.0], tolerance, "sky");
    assert_near(
        &at(20, 12),
        &[0.0, 0.0, 0.5],
        tolerance,
        "open water reads black and the far plane: the sky is not in the opaque copy",
    );
    assert_near(
        &at(28, 4),
        &[1.0, 0.0, 0.25],
        tolerance,
        "a transparent over the sky reads it, at the far plane, from the backdrop copy",
    );
    assert_near(
        &at(28, 12),
        &[0.5, 0.9, 0.25],
        tolerance,
        "a transparent over the water reads the water and its depth",
    );

    let depth = read_texture(
        &gpu,
        &renderer
            .resources()
            .texture(SCENE_OPAQUE_DEPTH)
            .unwrap()
            .texture,
    );
    assert_near(
        &texel(&depth, WIDTH, 1, 4, 12),
        &[0.8],
        1e-6,
        "opaque depth copy",
    );
    assert_near(
        &texel(&depth, WIDTH, 1, 20, 12),
        &[0.0],
        1e-6,
        "opaque depth copy",
    );
}

#[test]
fn a_frame_without_copies_is_one_pass() {
    let (_gpu, _renderer, layer, report) = render(SceneCopies::NONE);
    assert_eq!(report.copies, SceneCopies::NONE);
    assert_eq!(
        report.scene_passes,
        ["scene opaque, water, sky, transparent"]
    );
    assert!(layer.hooks.is_empty());
}

#[test]
fn only_the_asked_for_point_splits_the_pass() {
    let (_gpu, _renderer, layer, report) = render(SceneCopies::BACKDROP_DEPTH);
    assert_eq!(
        report.scene_passes,
        ["scene opaque, water, sky", "scene transparent"]
    );
    assert_eq!(layer.hooks, [CopyPoint::Backdrop]);
}

/// Finishes each encoder on a thread of its own, and counts them.
#[derive(Default)]
struct Threads(AtomicUsize);

impl Finisher for Threads {
    fn finish(&self, part: Part) {
        self.0.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || part.finish());
    }
}

#[test]
fn a_frame_finished_in_parts_on_other_threads_draws_the_same() {
    let (gpu, renderer, _, _) = render(SceneCopies::ALL);
    let whole = read_texture(
        &gpu,
        &renderer.resources().texture(SCENE_HDR).unwrap().texture,
    );
    let threads = Arc::new(Threads::default());
    let (gpu, renderer, layer, _) = render_finishing(SceneCopies::ALL, Some(threads.clone()));
    assert_eq!(layer.hooks, [CopyPoint::Opaque, CopyPoint::Backdrop]);
    // The layers' prepare, the shadow maps and the scene pass's three groups; the post finishes where it is submitted.
    assert_eq!(threads.0.load(Ordering::Relaxed), 5);
    let parts = read_texture(
        &gpu,
        &renderer.resources().texture(SCENE_HDR).unwrap().texture,
    );
    assert!(
        parts == whole,
        "the copies read across the parts as across one encoder"
    );
}

/// A texture write through a pass's uploads (`PassUploads::write_texture`, the text atlas's) lands where it is recorded
/// on the encoder: a copy recorded between two writes reads the first, as when each pass was a submission of its own.
/// `Queue::write_texture` would land both ahead of the encoder, and the copy would read the second.
#[test]
fn a_texture_written_through_pass_uploads_changes_in_encoder_order() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    // 64 texels of 4 bytes: one row of 256, a copy's row alignment.
    let size = wgpu::Extent3d {
        width: 64,
        height: 2,
        depth_or_array_layers: 1,
    };
    let texture = |label| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    };
    let (written, first, second) = (texture("written"), texture("first"), texture("second"));
    let mut uploads = naval_render::PassUploads::new(&gpu.device, 1 << 10);
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for (value, copy) in [(1.0f32, &first), (2.0, &second)] {
        let data: Vec<u8> = std::iter::repeat_n(value.to_le_bytes(), 128)
            .flatten()
            .collect();
        uploads.write_texture(
            &mut encoder,
            written.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(2),
            },
            size,
        );
        encoder.copy_texture_to_texture(written.as_image_copy(), copy.as_image_copy(), size);
    }
    uploads.finish(&encoder);
    gpu.queue.submit([encoder.finish()]);
    assert_eq!(read_texture(&gpu, &first), vec![1.0; 128]);
    assert_eq!(read_texture(&gpu, &second), vec![2.0; 128]);
}
