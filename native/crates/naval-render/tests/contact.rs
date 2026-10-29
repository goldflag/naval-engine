//! Contact shadows on a real GPU: a post 1.2 m tall on the ground, the sun 45 degrees up, seen from 4 m. The march
//! shades the ground beside its foot on the side away from the light, and nothing on the sunward side, nor further
//! along its shadow than the walk reaches (the maps' work); with no sun the mask is lit, and with contact shadows off
//! nothing is drawn or kept.

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    CONTACT_DEPTH, CONTACT_MASK, Camera, CasterPipeline, DrawContext, FrameInput, FrameReport,
    FrameTime, Phase, PipelineId, RenderContext, RenderOutput, Renderer, SceneLayer,
    ShadowSettings, Sun,
};

const SHADER: &str = r#"
#import naval::camera::camera

// The ground (two triangles, 20 m square) then the post (a box 10 cm square, 1.2 m tall, on the origin).
@vertex fn contact_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    var ground = array<vec2<f32>, 6>(
        vec2<f32>(-10.0, -10.0), vec2<f32>(10.0, -10.0), vec2<f32>(10.0, 10.0),
        vec2<f32>(-10.0, -10.0), vec2<f32>(10.0, 10.0), vec2<f32>(-10.0, 10.0),
    );
    if i < 6u {
        return camera.view_projection * vec4<f32>(ground[i].x, 0.0, ground[i].y, 1.0);
    }
    // A box's 36 corners from its 8: each face two triangles.
    var faces = array<u32, 36>(
        0u, 1u, 3u, 0u, 3u, 2u, 4u, 6u, 7u, 4u, 7u, 5u, 0u, 4u, 5u, 0u, 5u, 1u,
        2u, 3u, 7u, 2u, 7u, 6u, 0u, 2u, 6u, 0u, 6u, 4u, 1u, 5u, 7u, 1u, 7u, 3u,
    );
    let k = faces[i - 6u];
    let corner = vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32((k >> 2u) & 1u));
    let p = vec3<f32>(-0.05, 0.0, -0.05) + corner * vec3<f32>(0.1, 1.2, 0.1);
    return camera.view_projection * vec4<f32>(p, 1.0);
}
"#;

struct Post {
    depth: PipelineId,
}

impl Post {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/contact.wgsl", SHADER).unwrap();
        let depth = ctx
            .add_caster_pipeline(
                "post contact depth",
                "test/contact.wgsl",
                CasterPipeline::new("contact_vertex"),
            )
            .unwrap();
        Self { depth }
    }
}

impl SceneLayer for Post {
    fn label(&self) -> &str {
        "post"
    }

    fn draw(&self, _phase: Phase, _pass: &mut wgpu::RenderPass<'_>, _ctx: &DrawContext<'_>) {}

    fn draws_contact_depth(&self) -> bool {
        true
    }

    fn draw_contact_depth(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        pass.set_pipeline(ctx.pipelines.render(self.depth));
        pass.draw(0..42, 0..1);
    }
}

const SIZE: u32 = 128;

/// The contact mask (0 to 255) at world points, with the frame's report; `None` when no mask was kept.
fn render(
    settings: ShadowSettings,
    sun: Option<Sun>,
    points: &[Vec3],
) -> (Option<Vec<u8>>, FrameReport) {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, SIZE, SIZE, CAPTURE_FORMAT).unwrap();
    let mut post = Post::new(&mut renderer.context());
    renderer.set_shadow_settings(settings);
    renderer.set_sun(sun);
    let camera = Camera::look_at(Vec3::new(0.0, 3.0, 3.0), Vec3::ZERO);
    let capture = Capture::new(&gpu, SIZE, SIZE);
    let report = renderer
        .render(
            FrameInput {
                camera: &camera,
                time: FrameTime::default(),
            },
            &mut [&mut post],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: SIZE,
                height: SIZE,
            },
        )
        .unwrap();
    let Some(mask) = renderer.resources().texture(CONTACT_MASK) else {
        assert!(renderer.resources().texture(CONTACT_DEPTH).is_none());
        return (None, report);
    };
    let bytes = read_bytes(&gpu, &mask.texture);
    let view_projection = camera.projection(1.0) * camera.view();
    let values = points
        .iter()
        .map(|point| {
            let clip = view_projection * point.extend(1.0);
            let ndc = clip.truncate() / clip.w;
            let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32) as u32;
            let y = ((0.5 - ndc.y * 0.5) * SIZE as f32) as u32;
            bytes[(y * SIZE + x) as usize]
        })
        .collect();
    (Some(values), report)
}

/// A one-byte-a-texel texture read back row by row.
fn read_bytes(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let size = texture.size();
    let padded = size.width.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("contact mask read-back"),
        size: u64::from(padded) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size.height),
            },
        },
        size,
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("mapping"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting for the read-back");
    let mapped = slice.get_mapped_range().expect("mapped");
    (0..size.height)
        .flat_map(|y| {
            let row = (y * padded) as usize;
            mapped[row..row + size.width as usize].to_vec()
        })
        .collect()
}

/// The sun 45 degrees up from +x: the post's shadow falls toward −x.
fn sun() -> Sun {
    Sun::new(Vec3::new(1.0, 1.0, 0.0).normalize(), Vec3::ZERO)
}

#[test]
fn the_post_shades_the_ground_beside_its_foot_away_from_the_light() {
    // Beside its foot, away from the light: the walk meets the post 28 and 42 cm on, inside its 0.78 m (24 footprints
    // of 3.3 cm at 4.2 m over 128 rows) and before its last 40 %, where a part's shadow fades.
    let shaded = [Vec3::new(-0.25, 0.0, 0.0), Vec3::new(-0.35, 0.0, 0.02)];
    // On the sunward side; beside the shadow; and in it further than the walk reaches (the post stands 1.3 m along the
    // light from there: the maps' work).
    let lit = [
        Vec3::new(0.3, 0.0, 0.0),
        Vec3::new(-0.3, 0.0, 0.4),
        Vec3::new(-1.0, 0.0, 0.0),
    ];
    let points: Vec<Vec3> = shaded.iter().chain(&lit).copied().collect();
    let (values, report) = render(ShadowSettings::with_map_size(2048), Some(sun()), &points);
    assert!(report.contact, "the march ran");
    let values = values.expect("a mask while contact shadows are on");
    for (point, value) in points.iter().zip(&values) {
        if shaded.contains(point) {
            assert!(*value < 32, "in contact shadow at {point}: {value}");
        } else {
            assert!(*value > 223, "lit at {point}: {value}");
        }
    }
}

#[test]
fn no_light_leaves_the_mask_lit_and_contact_off_keeps_nothing() {
    let points = [Vec3::new(-0.25, 0.0, 0.0), Vec3::new(0.3, 0.0, 0.0)];
    let (values, report) = render(ShadowSettings::with_map_size(2048), None, &points);
    assert!(!report.contact);
    assert_eq!(
        values.expect("a mask while contact shadows are on"),
        [255, 255]
    );
    let off = ShadowSettings {
        contact: false,
        ..ShadowSettings::with_map_size(2048)
    };
    let (values, report) = render(off, Some(sun()), &points);
    assert!(!report.contact);
    assert!(values.is_none());
    // Shadows off take contact shadows with them.
    let (values, report) = render(ShadowSettings::with_map_size(0), Some(sun()), &points);
    assert!(!report.contact);
    assert!(values.is_none());
}
