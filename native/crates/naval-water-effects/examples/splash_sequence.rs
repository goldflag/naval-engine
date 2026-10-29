//! Offscreen checks of the splashes and bow spray over a plain sea: `cargo run -p naval-water-effects --example
//! splash_sequence --release [-- --frames 120]`.
//!
//! Writes to `.build/native/water-effects/particles/`:
//! - `splash-<t>.png`: a 38 cm and a 20 cm splash (the web's `splash` review scene: camera 330 m off, 30 m up) at
//!   t = 0.2, 0.6, 1.2, 2, 3.5 and 6 s;
//! - `bow-<view>.png`: a destroyer-sized proxy bow (112.5 m, 12 m beam) at 28 kn after 20 s, from ahead and abeam of the
//!   stem (the web's `bow-spray` cameras), and `bow-plunge.png` the same bow in a 3 m sea;
//! - `timings.json`: CPU time of the effects' update for a salvo of nine heavy splashes at their peak and for four bows at
//!   28 kn, and serialised wall-clock frames with and without the effects layer (`--frames N`, default 120).
//!
//! The backdrop is a flat sea and a sky gradient drawn here, not the ocean: this checks the particles alone.

use naval_effects::CameraView;
use std::time::Instant;

use anyhow::Result;
use glam::DVec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu, build_dir};
use naval_ocean::hull_profile::SimpleHull;
use naval_render::{
    Camera, DrawContext, FrameInput, FrameTime, Phase, PipelineId, RenderContext, RenderOutput,
    Renderer, SceneLayer, ScenePipeline, ShaderFile,
};
use naval_water_effects::bow_spray::{CrestShape, EffectShip};
use naval_water_effects::fixture::{HullInfo, SeaWaves, ShipPose, WaterEvent, WaterEventKind};
use naval_water_effects::{EffectLighting, EffectsFrame, WaterEffects};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

const BACKDROP: ShaderFile = ShaderFile {
    path: "naval-water-effects/examples/backdrop.wgsl",
    source: r#"
#import naval::camera::{camera, view_ray, ndc_from_pixel}

struct Out { @builtin(position) clip: vec4<f32>, @location(0) world: vec3<f32> }

// A 40 km square of flat sea at y = 0, depth written.
@vertex
fn sea_vertex(@builtin(vertex_index) index: u32) -> Out {
    var corners = array<vec2<f32>, 6>(vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    let c = corners[index] * 20000.0 + camera.position.xz;
    var out: Out;
    out.world = vec3<f32>(c.x, 0.0, c.y);
    out.clip = camera.view_projection * vec4<f32>(out.world, 1.0);
    return out;
}

@fragment
fn sea_fragment(input: Out) -> @location(0) vec4<f32> {
    let distance = length(input.world - camera.position);
    let haze = 1.0 - exp(-distance / 6000.0);
    return vec4<f32>(mix(vec3<f32>(0.012, 0.035, 0.06), vec3<f32>(0.35, 0.42, 0.5), haze), 1.0);
}

@vertex
fn sky_vertex(@builtin(vertex_index) index: u32) -> Out {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Out;
    out.clip = vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
    out.world = vec3<f32>(0.0);
    return out;
}

@fragment
fn sky_fragment(input: Out) -> @location(0) vec4<f32> {
    let ray = view_ray(ndc_from_pixel(input.clip.xy));
    let up = clamp(ray.y, 0.0, 1.0);
    return vec4<f32>(mix(vec3<f32>(0.55, 0.62, 0.7), vec3<f32>(0.16, 0.28, 0.55), sqrt(up)), 1.0);
}
"#,
    disk: None,
};

struct Backdrop {
    sea: PipelineId,
    sky: PipelineId,
}

impl Backdrop {
    fn new(ctx: &mut RenderContext<'_>) -> Result<Self> {
        ctx.add_shader(BACKDROP)
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let open = |vertex, fragment, depth_write| ScenePipeline {
            primitive: wgpu::PrimitiveState {
                cull_mode: None,
                ..Default::default()
            },
            depth_write,
            ..ScenePipeline::new(vertex, fragment)
        };
        let sea = ctx
            .add_scene_pipeline(
                "backdrop sea",
                BACKDROP.path,
                open("sea_vertex", "sea_fragment", true),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let sky = ctx
            .add_scene_pipeline(
                "backdrop sky",
                BACKDROP.path,
                open("sky_vertex", "sky_fragment", false),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(Self { sea, sky })
    }
}

impl SceneLayer for Backdrop {
    fn label(&self) -> &str {
        "backdrop"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        match phase {
            Phase::Opaque => {
                pass.set_pipeline(ctx.pipelines.render(self.sea));
                pass.draw(0..6, 0..1);
            }
            Phase::Sky => {
                pass.set_pipeline(ctx.pipelines.render(self.sky));
                pass.draw(0..3, 0..1);
            }
            Phase::Water | Phase::Transparent => {}
        }
    }
}

/// The afternoon light of the web's review scenes (hours 15): the sun about 40° up in the south-west.
fn lighting() -> EffectLighting {
    let sun = DVec3::new(0.55, 0.64, 0.54).normalize();
    let mut lighting =
        EffectLighting::from_sky(sun, DVec3::new(1.0, 0.93, 0.82), 5.8, false, 0.65, false);
    lighting.set_wind(9.0, 60f64.to_radians());
    lighting
}

fn render(
    renderer: &mut Renderer,
    layers: &mut [&mut dyn SceneLayer],
    camera: &Camera,
    capture: &Capture,
    frame: u64,
) -> Result<()> {
    renderer
        .render(
            FrameInput {
                camera,
                time: FrameTime {
                    seconds: frame as f64 / 60.0,
                    dt: 1.0 / 60.0,
                    frame,
                },
            },
            layers,
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: WIDTH,
                height: HEIGHT,
            },
        )
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    Ok(())
}

fn splash(sequence: u64, x: f64, z: f64, caliber: f64) -> WaterEvent {
    WaterEvent {
        sequence,
        kind: WaterEventKind::Splash,
        position: [x, 0.0, z],
        velocity: Some([-300.0, -450.0, 0.0]),
        caliber_m: Some(caliber),
        shell_id: Some(1_000_000 + sequence as i64),
    }
}

fn destroyer(id: &str) -> HullInfo {
    HullInfo {
        id: id.into(),
        preset: "fletcher".into(),
        length: 112.5,
        beam: 12.0,
        draft: 4.0,
        forward_speed: 18.5,
        reverse_speed: 6.0,
        ..Default::default()
    }
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    let frames: usize = arguments
        .iter()
        .position(|a| a == "--frames")
        .and_then(|i| arguments.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    let out = build_dir("water-effects").join("particles");
    std::fs::create_dir_all(&out)?;
    let gpu = Gpu::headless()?;
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut backdrop = Backdrop::new(&mut renderer.context())?;
    let mut effects =
        WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let light = lighting();
    let mut timings = serde_json::Map::new();

    // Splashes: the web's `splash` scene, the target at the origin heading north, the camera off her starboard side.
    let camera = Camera {
        eye: glam::Vec3::new(330.0, 30.0, 120.0),
        target: glam::Vec3::new(60.0, 25.0, 20.0),
        fov_y_degrees: 45.0,
        near: 0.5,
        far: 60000.0,
        curvature: naval_render::camera::EARTH_CURVATURE,
    };
    let view = CameraView::from_render(&camera, WIDTH, HEIGHT);
    let events = [splash(1, 60.0, 20.0, 0.38), splash(2, 95.0, -40.0, 0.2)];
    let mut time = 0.0;
    let mut frame = 0u64;
    for at in [0.2, 0.6, 1.2, 2.0, 3.5, 6.0] {
        while time < at - 1e-9 {
            let fired = if frame == 0 { &events[..] } else { &[] };
            effects.update(&EffectsFrame {
                dt: if frame == 0 { 0.0 } else { 1.0 / 60.0 },
                camera: view,
                ships: &[],
                events: fired,
                sea: None,
                waves: 0.0,
                crest: None,
                lighting: light,
            });
            if frame > 0 {
                time += 1.0 / 60.0;
            }
            frame += 1;
        }
        render(
            &mut renderer,
            &mut [&mut backdrop, &mut effects],
            &camera,
            &capture,
            frame,
        )?;
        capture.save_png(&gpu, &out.join(format!("splash-{at:.1}s.png")))?;
        println!("splash {at} s: {:?}", effects.counts());
    }

    // A destroyer-sized proxy bow at 28 kn, 20 s after reaching it, from the web's `bow-spray` cameras.
    let hull = destroyer("fletcher");
    let profile =
        SimpleHull::warship(hull.length as f32, hull.beam as f32, hull.draft as f32).profile();
    let length = hull.length;
    let stem = -length / 2.0;
    let pose_at = |t: f64| ShipPose {
        id: hull.id.clone(),
        position: [0.0, 0.0, -14.4 * t],
        speed: 14.4,
        ..Default::default()
    };
    let bow_camera = |pose: &ShipPose, offset: [f64; 3], look: [f64; 3], fov: f32| {
        let m = pose.matrix();
        let eye = m.transform_point3(DVec3::from(offset)).as_vec3();
        let target = m.transform_point3(DVec3::from(look)).as_vec3();
        Camera {
            eye,
            target,
            fov_y_degrees: fov,
            near: 0.5,
            far: 60000.0,
            curvature: naval_render::camera::EARTH_CURVATURE,
        }
    };
    let views = [
        (
            "ahead",
            [length * 0.28, length * 0.06, stem - length * 0.75],
            [0.0, 2.0, stem + length * 0.12],
            40.0,
        ),
        (
            "abeam",
            [length * 0.42, length * 0.05, stem - length * 0.05],
            [0.0, 3.0, stem + length * 0.08],
            40.0,
        ),
        (
            "abeam-port",
            [-length * 0.42, length * 0.05, stem - length * 0.05],
            [0.0, 3.0, stem + length * 0.08],
            40.0,
        ),
    ];
    for (name, offset, look, fov) in views {
        let mut run =
            WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let steps = 20 * 60;
        let mut camera = bow_camera(&pose_at(0.0), offset, look, fov);
        for step in 0..=steps {
            let t = step as f64 / 60.0;
            let pose = pose_at(t);
            camera = bow_camera(&pose, offset, look, fov);
            let ships = [EffectShip {
                pose: &pose,
                hull: &hull,
                profile: &profile,
                visible: true,
                lost: false,
                section: None,
            }];
            run.update(&EffectsFrame {
                dt: 1.0 / 60.0,
                camera: CameraView::from_render(&camera, WIDTH, HEIGHT),
                ships: &ships,
                events: &[],
                sea: None,
                waves: 1.0,
                crest: Some(CrestShape::default()),
                lighting: light,
            });
        }
        render(
            &mut renderer,
            &mut [&mut backdrop, &mut run],
            &camera,
            &capture,
            frame,
        )?;
        capture.save_png(&gpu, &out.join(format!("bow-{name}.png")))?;
        println!("bow {name}: {:?}", run.counts());
    }

    // The same bow in a 3 m head sea: the plunges throw spray over the forecastle.
    let sea = SeaWaves {
        amplitude_m: 3.0,
        wavelength_m: 90.0,
        direction: std::f64::consts::FRAC_PI_2,
        wind_mps: 22.0,
        phase: 0.0,
    };
    {
        let mut run =
            WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let (_, offset, look, fov) = views[1];
        let mut camera = bow_camera(&pose_at(0.0), offset, look, fov);
        let mut slams = 0;
        for step in 0..=(12 * 60) {
            let t = step as f64 / 60.0;
            let mut pose = pose_at(t);
            // Ride the sea at the bow and the stern as a rough heave and pitch.
            let bow_height = sea.height(0.0, pose.position[2] + stem, t);
            let stern_height = sea.height(0.0, pose.position[2] - stem, t);
            pose.position[1] = (bow_height + stern_height) * 0.25;
            pose.pitch = (bow_height - stern_height) / length * 0.5;
            camera = bow_camera(&pose, offset, look, fov);
            let ships = [EffectShip {
                pose: &pose,
                hull: &hull,
                profile: &profile,
                visible: true,
                lost: false,
                section: None,
            }];
            run.update(&EffectsFrame {
                dt: 1.0 / 60.0,
                camera: CameraView::from_render(&camera, WIDTH, HEIGHT),
                ships: &ships,
                events: &[],
                sea: Some((sea, t)),
                waves: 5.0,
                crest: Some(CrestShape::default()),
                lighting: light,
            });
            slams += run.bow_spray.slamming();
            // Stop at the height of a plunge.
            if step > 300 && run.bow_spray.slamming() > 0 && run.counts().bursts > 0 {
                break;
            }
        }
        render(
            &mut renderer,
            &mut [&mut backdrop, &mut run],
            &camera,
            &capture,
            frame,
        )?;
        capture.save_png(&gpu, &out.join("bow-plunge.png"))?;
        println!("bow plunge: {:?}, plunging frames {slams}", run.counts());
    }

    // CPU cost: a salvo of nine heavy splashes at their peak (1.2 s), and four bows at 28 kn.
    {
        let mut run =
            WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let salvo: Vec<WaterEvent> = (0..9)
            .map(|i| {
                splash(
                    10 + i,
                    40.0 + (i % 3) as f64 * 35.0,
                    -60.0 + (i / 3) as f64 * 45.0,
                    0.38,
                )
            })
            .collect();
        let mut samples = Vec::new();
        for step in 0..(3 * 60) {
            let started = Instant::now();
            run.update(&EffectsFrame {
                dt: 1.0 / 60.0,
                camera: view,
                ships: &[],
                events: if step == 0 { &salvo } else { &[] },
                sea: None,
                waves: 0.0,
                crest: None,
                lighting: light,
            });
            if step >= 60 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        timings.insert(
            "salvo9UpdateMsMedian".into(),
            samples[samples.len() / 2].into(),
        );
        timings.insert("salvo9Counts".into(), format!("{:?}", run.counts()).into());
        // GPU: serialised frames of the salvo at its peak, with the effects and without.
        run.reset();
        for step in 0..72 {
            run.update(&EffectsFrame {
                dt: 1.0 / 60.0,
                camera: view,
                ships: &[],
                events: if step == 0 { &salvo } else { &[] },
                sea: None,
                waves: 0.0,
                crest: None,
                lighting: light,
            });
        }
        let mut serial = |layers: &mut [&mut dyn SceneLayer]| -> Result<f64> {
            let mut times = Vec::new();
            for i in 0..frames + 10 {
                let started = Instant::now();
                render(&mut renderer, layers, &camera, &capture, frame)?;
                let _ = gpu.device.poll(wgpu::PollType::wait_indefinitely());
                if i >= 10 {
                    times.push(started.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            Ok(times[times.len() / 2])
        };
        let with = serial(&mut [&mut backdrop, &mut run])?;
        let without = serial(&mut [&mut backdrop])?;
        timings.insert("salvo9FrameMsWith".into(), with.into());
        timings.insert("salvo9FrameMsWithout".into(), without.into());
        render(
            &mut renderer,
            &mut [&mut backdrop, &mut run],
            &camera,
            &capture,
            frame,
        )?;
        capture.save_png(&gpu, &out.join("salvo-peak.png"))?;
        println!("salvo of nine at 1.2 s: {:?}", run.counts());
    }
    {
        let mut run =
            WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let hulls: Vec<HullInfo> = (0..4).map(|i| destroyer(&format!("dd{i}"))).collect();
        let mut samples = Vec::new();
        let chase = Camera {
            eye: glam::Vec3::new(-80.0, 40.0, 150.0),
            target: glam::Vec3::new(40.0, 5.0, -60.0),
            fov_y_degrees: 50.0,
            near: 0.5,
            far: 60000.0,
            curvature: naval_render::camera::EARTH_CURVATURE,
        };
        for step in 0..(6 * 60) {
            let t = step as f64 / 60.0;
            let poses: Vec<ShipPose> = hulls
                .iter()
                .enumerate()
                .map(|(i, h)| ShipPose {
                    id: h.id.clone(),
                    position: [i as f64 * 40.0, 0.0, -14.4 * t + i as f64 * 30.0],
                    speed: 14.4,
                    ..Default::default()
                })
                .collect();
            let ships: Vec<EffectShip> = poses
                .iter()
                .zip(&hulls)
                .map(|(pose, hull)| EffectShip {
                    pose,
                    hull,
                    profile: &profile,
                    visible: true,
                    lost: false,
                    section: None,
                })
                .collect();
            let mut moved = chase;
            moved.eye.z += poses[0].position[2] as f32;
            moved.target.z += poses[0].position[2] as f32;
            let started = Instant::now();
            run.update(&EffectsFrame {
                dt: 1.0 / 60.0,
                camera: CameraView::from_render(&moved, WIDTH, HEIGHT),
                ships: &ships,
                events: &[],
                sea: None,
                waves: 1.0,
                crest: Some(CrestShape::default()),
                lighting: light,
            });
            if step >= 120 {
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        samples.sort_by(f64::total_cmp);
        timings.insert(
            "bows4UpdateMsMedian".into(),
            samples[samples.len() / 2].into(),
        );
        timings.insert("bows4Counts".into(), format!("{:?}", run.counts()).into());
    }
    let load = std::process::Command::new("sysctl")
        .arg("-n")
        .arg("vm.loadavg")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    timings.insert("loadAverage".into(), load.into());
    timings.insert(
        "adapter".into(),
        format!("{:?}", gpu.adapter.get_info().name).into(),
    );
    std::fs::write(
        out.join("timings.json"),
        serde_json::to_string_pretty(&serde_json::Value::Object(timings.clone()))?,
    )?;
    println!("{}", serde_json::Value::Object(timings));
    Ok(())
}
