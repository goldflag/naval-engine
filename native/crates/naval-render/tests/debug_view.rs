//! The developer panel's debug views and exposure control on a real GPU: each full-screen view draws over the test
//! scene and changes the frame, the wireframe draws the box as lines once its twin is built (where the adapter has
//! line polygons), and a set exposure brightens the frame by what it says and reads back.

use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    DebugView, ExposureControl, EyeAdaptationSettings, FrameInput, FrameTime, RenderOutput,
    Renderer, SkyMean, TestScene,
};

/// A headless device with line polygons where the adapter has them.
fn gpu() -> Gpu {
    pollster::block_on(async {
        let instance = native_offscreen::instance();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .expect("a GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("debug view test"),
                required_features: adapter.features() & wgpu::Features::POLYGON_MODE_LINE,
                ..Default::default()
            })
            .await
            .expect("a device");
        Gpu {
            instance,
            adapter,
            device,
            queue,
        }
    })
}

fn render(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &mut TestScene,
    capture: &Capture,
    frame: u64,
) -> Vec<u8> {
    renderer
        .render(
            FrameInput {
                camera: &TestScene::default_camera(),
                time: FrameTime::fixed(frame, 1.0 / 60.0),
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

fn mean(pixels: &[u8]) -> f64 {
    pixels
        .chunks(4)
        .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
        .sum::<f64>()
        / (pixels.len() as f64 / 4.0 * 3.0)
}

fn changed(a: &[u8], b: &[u8]) -> f64 {
    let differ = a
        .chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x[..3].iter().zip(&y[..3]).any(|(p, q)| p.abs_diff(*q) > 8))
        .count();
    differ as f64 / (a.len() / 4) as f64
}

#[test]
fn each_debug_view_draws_and_off_restores_the_frame() {
    let gpu = gpu();
    let (width, height) = (96, 64);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sun(Some(TestScene::sun()));
    renderer.set_shadow_settings(naval_render::ShadowSettings::with_map_size(2048));
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let normal = render(&gpu, &mut renderer, &mut scene, &capture, 1);
    let views = renderer.debug_views();
    assert_eq!(
        views.contains(&DebugView::Wireframe),
        gpu.device
            .features()
            .contains(wgpu::Features::POLYGON_MODE_LINE)
    );
    for view in [
        DebugView::Normals,
        DebugView::Depth,
        DebugView::ShadowCascades,
    ] {
        renderer.set_debug_view(view).unwrap();
        let shown = render(&gpu, &mut renderer, &mut scene, &capture, 1);
        let share = changed(&normal, &shown);
        assert!(share > 0.2, "{view:?} changed {share:.2} of the frame");
    }
    // The normals: the box's top faces up (green), the sky is black.
    renderer.set_debug_view(DebugView::Normals).unwrap();
    let shown = render(&gpu, &mut renderer, &mut scene, &capture, 1);
    let corner = &shown[..3];
    assert!(corner.iter().all(|c| *c < 8), "sky is black: {corner:?}");
    renderer.set_debug_view(DebugView::Off).unwrap();
    assert_eq!(render(&gpu, &mut renderer, &mut scene, &capture, 1), normal);

    if views.contains(&DebugView::Wireframe) {
        renderer.set_debug_view(DebugView::Wireframe).unwrap();
        // The twins build on threads; frames until then draw filled.
        let mut lines = normal.clone();
        for _ in 0..200 {
            lines = render(&gpu, &mut renderer, &mut scene, &capture, 1);
            if renderer.pipelines().wireframe_twins() > 0 && lines != normal {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(renderer.pipelines().wireframe_twins() > 0);
        assert!(changed(&normal, &lines) > 0.02, "the box is drawn as lines");
        renderer.set_debug_view(DebugView::Off).unwrap();
        assert_eq!(render(&gpu, &mut renderer, &mut scene, &capture, 1), normal);
    } else {
        assert!(renderer.set_debug_view(DebugView::Wireframe).is_err());
    }
}

#[test]
fn a_set_exposure_brightens_by_its_stops_and_reads_back() {
    let gpu = gpu();
    let (width, height) = (64, 36);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sky_mean(SkyMean::Rgb(TestScene::sky_mean()));
    renderer.set_eye_adaptation(EyeAdaptationSettings {
        enabled: true,
        ..Default::default()
    });
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    renderer.snap_eye_adaptation();
    let auto = render(&gpu, &mut renderer, &mut scene, &capture, 1);
    renderer.set_exposure_control(ExposureControl::Fixed(1.5));
    let mut frame = 2;
    let brighter = render(&gpu, &mut renderer, &mut scene, &capture, frame);
    assert!(
        mean(&brighter) > mean(&auto) + 10.0,
        "{} {}",
        mean(&brighter),
        mean(&auto)
    );
    // Read back a few frames later, without waiting on the GPU in between.
    let mut reading = None;
    for _ in 0..20 {
        renderer.request_eye_reading();
        frame += 1;
        render(&gpu, &mut renderer, &mut scene, &capture, frame);
        reading = renderer.eye_reading();
        if reading.is_some() {
            break;
        }
    }
    assert_eq!(reading.expect("a reading")[0], 1.5);
    // Held, the stops stay where they were however the view meters.
    renderer.set_exposure_control(ExposureControl::Hold);
    for _ in 0..5 {
        frame += 1;
        render(&gpu, &mut renderer, &mut scene, &capture, frame);
    }
    renderer.request_eye_reading();
    for _ in 0..20 {
        frame += 1;
        render(&gpu, &mut renderer, &mut scene, &capture, frame);
        renderer.request_eye_reading();
    }
    assert_eq!(renderer.eye_reading().expect("a reading")[0], 1.5);
}
