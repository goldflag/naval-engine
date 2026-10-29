//! GPU timings on a real device with timestamp queries: every pass the renderer
//! records is timed, results arrive a few submissions late without blocking, and
//! a device without the feature times nothing.

use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneCopies, ShadowSettings, TestScene,
};

fn gpu_with_timestamps() -> Option<Gpu> {
    let instance = native_offscreen::instance();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .ok()?;
    if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
        return None;
    }
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .ok()?;
    Some(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

fn render_frames(gpu: &Gpu, renderer: &mut Renderer, scene: &mut TestScene, frames: u64) {
    let capture = Capture::new(gpu, 64, 36);
    for frame in 0..frames {
        renderer
            .render(
                FrameInput {
                    camera: &Camera::default(),
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                &mut [scene],
                RenderOutput {
                    view: &capture.view,
                    format: CAPTURE_FORMAT,
                    width: 64,
                    height: 36,
                },
            )
            .unwrap();
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
}

#[test]
fn every_pass_is_timed_a_few_frames_late() {
    let Some(gpu) = gpu_with_timestamps() else {
        eprintln!("skipped: the adapter takes no timestamps");
        return;
    };
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 64, 36, CAPTURE_FORMAT).unwrap();
    let mut scene = TestScene::new(&mut renderer.context(), 1).unwrap();
    renderer.set_shadow_settings(ShadowSettings::with_map_size(1024));
    renderer.set_sun(Some(TestScene::sun()));
    renderer.set_extra_scene_copies(SceneCopies::OPAQUE_DEPTH);
    assert!(renderer.gpu_timer().enabled());

    render_frames(&gpu, &mut renderer, &mut scene, 1);
    assert!(
        renderer.gpu_timer().latest().is_none(),
        "a frame's timings never arrive with the frame itself"
    );
    render_frames(&gpu, &mut renderer, &mut scene, 5);
    for _ in 0..3 {
        renderer.drain_gpu_timings();
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }
    renderer.drain_gpu_timings();
    let frames: Vec<_> = renderer.gpu_timer().history().cloned().collect();
    assert_eq!(frames.len(), 6, "every frame's timings arrive: {frames:?}");
    assert!(frames.windows(2).all(|pair| pair[0].frame < pair[1].frame));
    let labels: Vec<&str> = frames[0]
        .passes
        .iter()
        .map(|pass| pass.label.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "shadow near",
            "shadow wide",
            "scene opaque",
            "scene water, sky, transparent",
            "display"
        ]
    );
    for frame in &frames {
        let total = frame.total_ms.expect("a total");
        assert!(total > 0.0 && total < 1000.0, "{frame:?}");
        for pass in &frame.passes {
            let span = pass
                .span_ms
                .expect("resolved after the frame, never zero or stale");
            let serial = pass.serial_ms.expect("a serial cost");
            assert!(
                span >= 0.0 && serial >= 0.0 && serial <= span + 1e-9,
                "{pass:?}"
            );
        }
        let serial: f64 = frame.passes.iter().filter_map(|pass| pass.serial_ms).sum();
        assert!(
            serial <= total + 1e-6,
            "serial costs sum to at most the total: {frame:?}"
        );
    }
}

#[test]
fn a_device_without_timestamps_times_nothing() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    if gpu
        .device
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        return;
    }
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 64, 36, CAPTURE_FORMAT).unwrap();
    let mut scene = TestScene::new(&mut renderer.context(), 1).unwrap();
    assert!(!renderer.gpu_timer().supported());
    render_frames(&gpu, &mut renderer, &mut scene, 3);
    renderer.drain_gpu_timings();
    assert!(renderer.gpu_timer().latest().is_none());
}
