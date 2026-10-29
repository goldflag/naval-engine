//! GPU smoke test: a heavy splash and a bow at speed draw through the renderer core's transparent phase, and an empty
//! frame draws nothing.

use glam::{DVec3, Vec3};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::CameraView;
use naval_ocean::hull_profile::SimpleHull;
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_water_effects::bow_spray::{CrestShape, EffectShip};
use naval_water_effects::fixture::{HullInfo, ShipPose, WaterEvent, WaterEventKind};
use naval_water_effects::{EffectLighting, EffectsFrame, WaterEffects};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 144;

fn bright_pixels(
    gpu: &Gpu,
    renderer: &mut Renderer,
    effects: &mut WaterEffects,
    camera: &Camera,
    capture: &Capture,
) -> usize {
    let layers: &mut [&mut dyn SceneLayer] = &mut [effects];
    renderer
        .render(
            FrameInput {
                camera,
                time: FrameTime::fixed(0, 1.0 / 60.0),
            },
            layers,
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: WIDTH,
                height: HEIGHT,
            },
        )
        .expect("renders");
    let rgba = capture.read_rgba(gpu).expect("reads back");
    rgba.chunks(4)
        .filter(|p| p[0] > 60 && p[1] > 60 && p[2] > 60)
        .count()
}

#[test]
fn a_splash_and_a_bow_draw_and_an_empty_frame_does_not() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut effects = WaterEffects::new(&mut renderer.context()).expect("pipelines build");
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera {
        eye: Vec3::new(330.0, 30.0, 120.0),
        target: Vec3::new(60.0, 25.0, 20.0),
        fov_y_degrees: 45.0,
        near: 0.5,
        far: 60000.0,
        curvature: naval_render::camera::EARTH_CURVATURE,
    };
    let view = CameraView::from_render(&camera, WIDTH, HEIGHT);
    let lighting = EffectLighting::default();
    assert_eq!(
        bright_pixels(&gpu, &mut renderer, &mut effects, &camera, &capture),
        0
    );
    let event = WaterEvent {
        sequence: 1,
        kind: WaterEventKind::Splash,
        position: [60.0, 0.0, 20.0],
        velocity: Some([-300.0, -450.0, 0.0]),
        caliber_m: Some(0.38),
        shell_id: Some(1_000_001),
    };
    for frame in 0..72 {
        let events = if frame == 0 {
            std::slice::from_ref(&event)
        } else {
            &[]
        };
        effects.update(&EffectsFrame {
            dt: 1.0 / 60.0,
            camera: view,
            ships: &[],
            events,
            sea: None,
            waves: 0.0,
            crest: None,
            lighting,
        });
    }
    let splash = bright_pixels(&gpu, &mut renderer, &mut effects, &camera, &capture);
    assert!(
        splash > 100,
        "a heavy splash at 1.2 s covers {splash} bright pixels"
    );

    effects.reset();
    let hull = HullInfo {
        id: "dd".into(),
        length: 112.5,
        beam: 12.0,
        draft: 4.0,
        forward_speed: 18.5,
        ..Default::default()
    };
    let profile = SimpleHull::warship(112.5, 12.0, 4.0).profile();
    let mut bow_camera = camera;
    for frame in 0..600 {
        let t = frame as f64 / 60.0;
        let pose = ShipPose {
            id: "dd".into(),
            position: [0.0, 0.0, -14.4 * t],
            speed: 14.4,
            ..Default::default()
        };
        let stem = pose.matrix().transform_point3(DVec3::new(0.0, 0.0, -56.0));
        bow_camera = Camera {
            eye: (stem + DVec3::new(45.0, 6.0, -5.0)).as_vec3(),
            target: (stem + DVec3::new(0.0, 2.0, 8.0)).as_vec3(),
            ..camera
        };
        let ships = [EffectShip {
            pose: &pose,
            hull: &hull,
            profile: &profile,
            visible: true,
            lost: false,
            section: None,
        }];
        effects.update(&EffectsFrame {
            dt: 1.0 / 60.0,
            camera: CameraView::from_render(&bow_camera, WIDTH, HEIGHT),
            ships: &ships,
            events: &[],
            sea: None,
            waves: 1.0,
            crest: Some(CrestShape::default()),
            lighting,
        });
    }
    let bow = bright_pixels(&gpu, &mut renderer, &mut effects, &bow_camera, &capture);
    assert!(bow > 50, "a bow at 28 kn covers {bow} bright pixels");
    assert_eq!(effects.counts().sheets, 2);
}

/// The splash columns' noise volume is filled before the first column draws, and each of its channels spreads over
/// 0–1 about 0.5 (`shaders/column_noise.wgsl` stretches them to their percentiles), which the column's shapes count on.
#[test]
fn the_column_noise_fills_and_spreads_over_its_range() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut effects = WaterEffects::new(&mut renderer.context()).expect("pipelines build");
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera {
        eye: Vec3::new(300.0, 30.0, 0.0),
        target: Vec3::new(0.0, 20.0, 0.0),
        fov_y_degrees: 45.0,
        near: 0.5,
        far: 60000.0,
        curvature: 0.0,
    };
    let event = WaterEvent {
        sequence: 1,
        kind: WaterEventKind::Splash,
        position: [0.0, 0.0, 0.0],
        velocity: Some([300.0, -300.0, 0.0]),
        caliber_m: Some(0.38),
        shell_id: Some(1),
    };
    effects.update(&EffectsFrame {
        dt: 0.0,
        camera: CameraView::from_render(&camera, WIDTH, HEIGHT),
        ships: &[],
        events: std::slice::from_ref(&event),
        sea: None,
        waves: 0.0,
        crest: None,
        lighting: EffectLighting::default(),
    });
    bright_pixels(&gpu, &mut renderer, &mut effects, &camera, &capture);
    const SIZE: u32 = 64;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("column noise read-back"),
        size: u64::from(SIZE * SIZE * SIZE * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: effects.column_noise(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: SIZE,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the read-back maps");
    let texels = slice.get_mapped_range().expect("mapped").to_vec();
    for channel in 0..4 {
        let mut values: Vec<u8> = texels.chunks(4).map(|texel| texel[channel]).collect();
        values.sort_unstable();
        let at =
            |share: f64| f64::from(values[((values.len() - 1) as f64 * share) as usize]) / 255.0;
        let (low, middle, high) = (at(0.01), at(0.5), at(0.99));
        assert!(
            low < 0.12 && high > 0.88 && (0.4..0.6).contains(&middle),
            "channel {channel}: 1st percentile {low:.2}, median {middle:.2}, 99th {high:.2}"
        );
    }
}
/// The ocean rejects most scene TAA history. An unchanged column must therefore converge in one frame even with
/// TAA off: changing only the render frame counter used to move its march by up to five metres and change its face.
#[test]
fn a_column_does_not_need_temporal_history_to_hold_its_picture() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    renderer.set_antialiasing(naval_render::Antialiasing::None);
    let mut effects = WaterEffects::new(&mut renderer.context()).expect("pipelines build");
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera {
        eye: Vec3::new(80.0, 50.0, 450.0),
        target: Vec3::new(0.0, 30.0, 0.0),
        fov_y_degrees: 28.0,
        near: 0.5,
        far: 60000.0,
        curvature: 0.0,
    };
    let event = WaterEvent {
        sequence: 1,
        kind: WaterEventKind::Splash,
        position: [0.0; 3],
        velocity: Some([450.0, -210.0, 0.0]),
        caliber_m: Some(0.38),
        shell_id: Some(1),
    };
    for frame in 0..151 {
        effects.update(&EffectsFrame {
            dt: if frame == 0 { 0.0 } else { 1.0 / 60.0 },
            camera: CameraView::from_render(&camera, WIDTH, HEIGHT),
            ships: &[],
            events: if frame == 0 {
                std::slice::from_ref(&event)
            } else {
                &[]
            },
            sea: None,
            waves: 0.0,
            crest: None,
            lighting: EffectLighting::default(),
        });
    }
    let mut previous: Option<Vec<u8>> = None;
    for frame in [1, 2, 17, 63] {
        renderer
            .render(
                FrameInput {
                    camera: &camera,
                    // Fixed presentation time also holds output dithering; only the render frame changes.
                    time: FrameTime {
                        seconds: 2.5,
                        dt: 1.0 / 60.0,
                        frame,
                    },
                },
                &mut [&mut effects],
                RenderOutput {
                    view: &capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .expect("renders");
        let rgba = capture.read_rgba(&gpu).expect("reads back");
        assert!(
            rgba.as_chunks::<4>().0.iter().filter(|p| p[0] > 60).count() > 100,
            "the column is visible"
        );
        if let Some(previous) = &previous {
            let changed = rgba
                .iter()
                .zip(previous.iter())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                changed, 0,
                "frame {frame}: an unchanged column flickers without scene TAA"
            );
        }
        previous = Some(rgba);
    }
}
