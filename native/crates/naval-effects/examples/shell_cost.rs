//! What the shells cost: `cargo run -p naval-effects --release --example shell_cost [-- rounds 1920x1080]`.
//!
//! A fleet action's worth of rounds in flight (by default 64: eight 8-gun salvos of 15 cm rounds fired a second apart,
//! 12 km shots, each with its tracer: the costliest rounds) seen from 1.5 km off the line of fire, drawn over an empty scene at the given size. Reports the CPU
//! (`update` and `publish`) and the GPU: the scene pass's serial cost by pass timestamps with and without the
//! shells, 120 frames each, interleaved, each frame waited for.

use std::time::Instant;

use glam::{DVec3, Vec3};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::{ShellLayer, ShellState};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};

fn gpu() -> Gpu {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .expect("a GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("shell cost"),
        required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("a device");
    Gpu {
        instance,
        adapter,
        device,
        queue,
    }
}

/// Round `i` of `rounds` at battle time `now`: salvos of eight fired a second apart from 12 km off along +X.
fn round(i: usize, now: f64) -> Option<ShellState> {
    let salvo = i / 8;
    let gun = i % 8;
    let age = now - salvo as f64;
    if age <= 0.0 {
        return None;
    }
    let muzzle = DVec3::new(-6000.0, 12.0, f64::from(gun as u32) * 12.0 - 40.0);
    let velocity0 = DVec3::new(760.0, 190.0, 0.0);
    let position = muzzle + velocity0 * age + DVec3::new(0.0, -4.905 * age * age, 0.0);
    let velocity = velocity0 + DVec3::new(0.0, -9.81 * age, 0.0);
    Some(ShellState {
        id: i as i64,
        owner: 1,
        position,
        velocity,
        caliber_m: 0.152,
        age,
        airborne: position.y > 0.0,
        helm: false,
    })
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rounds: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(64);
    let (width, height) = args
        .get(1)
        .and_then(|s| s.split_once('x'))
        .map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap()))
        .unwrap_or((1920u32, 1080u32));
    let gpu = gpu();
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut layer = ShellLayer::new(&mut renderer.context()).unwrap();
    let capture = Capture::new(&gpu, width, height);
    let camera = Camera::look_at(Vec3::new(0.0, 60.0, 1800.0), Vec3::new(0.0, 900.0, 0.0));
    // Fly to 9 s: every salvo in the air, trails at full length.
    let mut now = 0.0;
    let mut cpu_update = Vec::new();
    while now < 9.0 {
        now += 1.0 / 60.0;
        let shells: Vec<ShellState> = (0..rounds).filter_map(|i| round(i, now)).collect();
        let started = Instant::now();
        layer.visuals.update(&shells, 1.0 / 60.0);
        cpu_update.push(started.elapsed().as_secs_f64() * 1e3);
    }
    let mut frame = 0u64;
    let mut render = |layers: &mut [&mut dyn SceneLayer], renderer: &mut Renderer| {
        frame += 1;
        let started = Instant::now();
        renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                layers,
                RenderOutput {
                    view: &capture.view,
                    format: CAPTURE_FORMAT,
                    width,
                    height,
                },
            )
            .unwrap();
        let cpu = started.elapsed().as_secs_f64() * 1e3;
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        renderer.drain_gpu_timings();
        let scene = renderer.gpu_timer().latest().and_then(|t| {
            t.passes
                .iter()
                .find(|p| p.label.starts_with("scene"))
                .and_then(|p| p.serial_ms)
        });
        (started.elapsed().as_secs_f64() * 1e3, scene, cpu)
    };
    let (mut with, mut without, mut scene_with, mut scene_without, mut publish) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..120 {
        let (wall, scene, cpu) = render(&mut [&mut layer], &mut renderer);
        with.push(wall);
        publish.push(cpu);
        scene_with.extend(scene);
        let (wall, scene, _) = render(&mut [], &mut renderer);
        without.push(wall);
        scene_without.extend(scene);
    }
    let stats = layer.stats();
    println!(
        "{rounds} rounds at {width}x{height}: {} bodies, {} glows, {} streaks, {} trail segments",
        stats.bodies + stats.detailed,
        stats.glows,
        stats.streaks,
        stats.trail_segments
    );
    println!(
        "CPU: update {:.3} ms, whole frame record (publish included) {:.3} ms (medians)",
        median(&mut cpu_update),
        median(&mut publish)
    );
    println!(
        "GPU wall: {:.3} ms with shells, {:.3} ms without; scene pass {:.3} ms with, {:.3} ms without",
        median(&mut with),
        median(&mut without),
        if scene_with.is_empty() {
            f64::NAN
        } else {
            median(&mut scene_with)
        },
        if scene_without.is_empty() {
            f64::NAN
        } else {
            median(&mut scene_without)
        },
    );
    let dir = native_offscreen::build_dir("aa-tracers");
    std::fs::create_dir_all(&dir).unwrap();
    render(&mut [&mut layer], &mut renderer);
    capture.save_png(&gpu, &dir.join("shell-cost.png")).unwrap();
    println!("{}", dir.join("shell-cost.png").display());
}
