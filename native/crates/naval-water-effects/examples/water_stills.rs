//! Offscreen stills of the native water effects beside the web game's (`native/docs/water-effects.md`).
//!
//! `cargo run -p naval-water-effects --example water_stills --release [-- --only wake-stern,splash] [--frames 60]
//! [--uncoupled]`
//!
//! The sea around hulls follows each capture's `hullSeaFlag` (the web's `--hullsea`, on by default); `--uncoupled` draws
//! it unblended whatever the web did, into `native-uncoupled/`, `side-by-side-uncoupled/`, `diff-uncoupled/` and
//! `compare-uncoupled.json`.
//!
//! For every scene the web driver wrote to `.build/native/water-effects/web/` (`bun
//! native/crates/naval-water-effects/web/water-stills.ts`) it reads the recording (every hull pose, water event and torpedo
//! the web's wakes consumed, frame by frame, with each frame's dt and camera), the capture cameras, the sun and the web
//! sky's equirectangular bake. It replays the recording through the ocean's `ShipWake` (and the splash and spray
//! particles) exactly as the web stepped them, freezes the waves as the ocean stills do (foam wiped at t = 50 − 1/30,
//! 300 updates to t = 60), and renders each capture through `naval_render::Renderer`: proxy hulls, the ocean with its
//! wakes, the stand-in sky. It writes `.build/native/water-effects/native/<capture>.png`, a web | native pair in
//! `side-by-side/`, a difference image in `diff/`, and `compare.json`. `--frames N` also times the wakes (serialised
//! whole frames with and without them, and the wake field's steps alone) into `timings.json`.

use naval_effects::CameraView;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu, build_dir};
use naval_ocean::backdrop::StandInSky;
use naval_ocean::calibration::{NORTH_ATLANTIC, scene_parameters};
use naval_ocean::environment::{CelestialLight, CubeUvLayout, Equirect, StandInEnvironment};
use naval_ocean::hull_sea::{HULL_SEA_SLOTS, HullSeaWave, SeaHull, hull_footprints};
use naval_ocean::ship_wake::{WakeCamera, WakeInputs};
use naval_ocean::{Ocean, QualityTier};
use naval_render::{
    Camera, DisplaySettings, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer,
};
use naval_water_effects::drive::{self, Hulls};
use naval_water_effects::fixture::Recording;
use naval_water_effects::hulls::{HullDraw, HullLayer, HullLighting};
use naval_water_effects::{CrestShape, EffectLighting, EffectShip, EffectsFrame, WaterEffects};
use serde_json::Value;

const WIDTH: u32 = 1600;
const HEIGHT: u32 = 900;

fn vec3(value: &Value) -> Vec3 {
    Vec3::new(
        value[0].as_f64().unwrap_or(0.0) as f32,
        value[1].as_f64().unwrap_or(0.0) as f32,
        value[2].as_f64().unwrap_or(0.0) as f32,
    )
}

fn vec3_of(position: [f64; 3]) -> Vec3 {
    Vec3::new(position[0] as f32, position[1] as f32, position[2] as f32)
}

/// A headless device.
fn device() -> Result<Gpu> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    }))
    .context("no GPU adapter")?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("water stills"),
        ..Default::default()
    }))?;
    Ok(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

/// Cosine-weighted irradiance from an equirectangular map (rows from the nadir up) on a face with normal `n`.
fn irradiance(width: u32, height: u32, rgba: &[f32], n: Vec3) -> Vec3 {
    let mut sum = Vec3::ZERO;
    for row in 0..height {
        let elevation = ((row as f32 + 0.5) / height as f32 - 0.5) * std::f32::consts::PI;
        let solid = (std::f32::consts::TAU / width as f32)
            * (std::f32::consts::PI / height as f32)
            * elevation.cos();
        for col in 0..width {
            let azimuth = ((col as f32 + 0.5) / width as f32 - 0.5) * std::f32::consts::TAU;
            let direction = Vec3::new(
                elevation.cos() * azimuth.cos(),
                elevation.sin(),
                elevation.cos() * azimuth.sin(),
            );
            let facing = direction.dot(n);
            if facing <= 0.0 {
                continue;
            }
            let i = ((row * width + col) * 4) as usize;
            sum += Vec3::new(rgba[i], rgba[i + 1], rgba[i + 2]) * facing * solid;
        }
    }
    sum
}

struct Scene {
    name: String,
    meta: Value,
    recording: Recording,
    sky: (u32, u32, Vec<f32>),
}

fn scenes(web: &Path, only: &Option<Vec<String>>) -> Result<Vec<Scene>> {
    let mut names: Vec<String> = std::fs::read_dir(web)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                .filter_map(|n| n.strip_suffix(".recording.json").map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    if let Some(only) = only {
        names.retain(|name| only.contains(name));
    }
    names
        .into_iter()
        .map(|name| {
            let meta: Value =
                serde_json::from_str(&std::fs::read_to_string(web.join(format!("{name}.json")))?)?;
            let recording = Recording::from_json(&std::fs::read_to_string(
                web.join(format!("{name}.recording.json")),
            )?)?;
            let bytes = std::fs::read(web.join(format!("{name}-sky.bin")))
                .with_context(|| format!("{name}-sky.bin"))?;
            let floats: Vec<f32> = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect();
            let sky = (
                meta["sky"]["width"].as_u64().unwrap_or(384) as u32,
                meta["sky"]["height"].as_u64().unwrap_or(192) as u32,
                floats,
            );
            Ok(Scene {
                name,
                meta,
                recording,
                sky,
            })
        })
        .collect()
}

/// The frame's camera as the wakes read it, from the web's `frameCameras` entry `[x, y, z, fov°, proj[5], aspect]`.
fn wake_camera(entry: &Value) -> WakeCamera {
    let get = |i: usize| entry[i].as_f64().unwrap_or(0.0);
    WakeCamera {
        eye: [get(0), get(1), get(2)],
        forward: [0.0, 0.0, -1.0],
        fov_y: get(3).to_radians(),
        zoom: get(4),
        drawing_height: f64::from(HEIGHT),
    }
}

struct Stage<'a> {
    gpu: &'a Gpu,
    renderer: Renderer,
    capture: Capture,
}

impl Stage<'_> {
    fn render(
        &mut self,
        layers: &mut [&mut dyn SceneLayer],
        camera: &Camera,
        frame: u64,
        dt: f32,
    ) -> Result<()> {
        self.renderer
            .render(
                FrameInput {
                    camera,
                    time: FrameTime {
                        seconds: frame as f64 / 60.0,
                        dt,
                        frame,
                    },
                },
                layers,
                RenderOutput {
                    view: &self.capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        Ok(())
    }
}

/// Freeze the waves as the web driver did: foam wiped, ten seconds of waves, then paused frames at t = 60.
fn freeze_waves(gpu: &Gpu, ocean: &mut Ocean) {
    let step = |ocean: &mut Ocean, time: f64, dt: f64| {
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        ocean
            .field
            .update(&gpu.queue, &mut encoder, &ocean.parameters, time, dt);
        gpu.queue.submit([encoder.finish()]);
    };
    step(ocean, 50.0 - 1.0 / 30.0, 1000.0);
    for i in 0..300 {
        step(ocean, 50.0 + f64::from(i) / 30.0, 1.0 / 30.0);
    }
    ocean.time = 60.0;
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    let value_of = |flag: &str| {
        arguments
            .iter()
            .position(|a| a == flag)
            .and_then(|i| arguments.get(i + 1))
            .cloned()
    };
    let only =
        value_of("--only").map(|list| list.split(',').map(str::to_owned).collect::<Vec<_>>());
    let frames: u32 = value_of("--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let uncoupled = arguments.iter().any(|a| a == "--uncoupled");
    let suffix = if uncoupled { "-uncoupled" } else { "" };
    let root = build_dir("water-effects");
    let web = root.join("web");
    let gpu = device()?;
    let info = gpu.adapter.get_info();
    println!("adapter: {} ({:?})", info.name, info.backend);
    let renderer = Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let mut stage = Stage {
        gpu: &gpu,
        renderer,
        capture,
    };
    let camera_layout = stage.renderer.context().camera_layout.clone();
    let mut comparisons = Vec::new();
    let mut timings = Vec::new();
    let scene_list = scenes(&web, &only)?;
    if scene_list.is_empty() {
        println!(
            "no web scenes in {}: run `bun native/crates/naval-water-effects/web/water-stills.ts` first",
            web.display()
        );
    }
    for scene in scene_list {
        println!("{}", scene.name);
        let meta = &scene.meta;
        let sky_light = &meta["skyLight"];
        let light = CelestialLight {
            direction: vec3(&sky_light["direction"]).normalize(),
            color: vec3(&sky_light["color"]),
            intensity: sky_light["intensity"].as_f64().unwrap_or(6.6) as f32,
            night: false,
        };
        let (width, height, rgba) = (&scene.sky.0, &scene.sky.1, &scene.sky.2);
        let environment = StandInEnvironment::new(
            &gpu.device,
            &gpu.queue,
            CubeUvLayout::from_bake_width(*width),
            &Equirect {
                width: *width,
                height: *height,
                rgba,
            },
            &light,
        );
        let far = meta["captures"][0]["camera"]["far"]
            .as_f64()
            .unwrap_or(60000.0);
        let mut ocean = Ocean::new(
            &gpu.device,
            &gpu.queue,
            QualityTier::High,
            &camera_layout,
            &environment.bind_group_layout,
            &environment.bind_group,
            far,
        );
        let wind = meta["wind"].as_f64().unwrap_or(9.0);
        ocean.parameters = scene_parameters(&NORTH_ATLANTIC, wind, false);
        ocean.enable_wake(&gpu.device);
        let mut hulls =
            HullLayer::new(&mut stage.renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        // The sea reads the renderer's copies of the opaque scene (the proxy hulls), as the slice's does.
        ocean.read_scene_copies = true;
        let mut sky = StandInSky::new(
            &gpu.device,
            &camera_layout,
            &environment.bind_group_layout,
            &environment.bind_group,
        );
        let mut effects = WaterEffects::new(&mut stage.renderer.context())
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        // The effect light exactly as the page set it (`EffectLighting` after `VisualEnvironment.syncLighting`).
        let lit = &meta["effectLighting"];
        let d3 = |v: &Value| vec3(v).as_dvec3();
        let lighting = EffectLighting {
            sun_direction: d3(&lit["sunDirection"]).normalize(),
            direct: d3(&lit["direct"]),
            ambient: d3(&lit["ambient"]),
            daylight: lit["daylight"].as_f64().unwrap_or(1.0),
            flash: 0.0,
            wind: d3(&lit["wind"]),
        };
        // Capture cameras by position: a frame whose eye is a capture's (a staged camera) takes its orientation.
        let orientations: Vec<(Vec3, glam::Quat)> = meta["captures"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|c| {
                        let q = &c["camera"]["quaternion"];
                        let f = |i: usize| {
                            q[i].as_f64().unwrap_or(if i == 3 { 1.0 } else { 0.0 }) as f32
                        };
                        (
                            vec3(&c["camera"]["position"]),
                            glam::Quat::from_xyzw(f(0), f(1), f(2), f(3)),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let sun = light.color * light.intensity;
        let hull_lighting = HullLighting {
            sun_direction: light.direction,
            sun,
            sky_up: irradiance(*width, *height, rgba, Vec3::Y),
            sky_side: irradiance(*width, *height, rgba, Vec3::X),
        };
        let exposure = meta["display"]["skyExposure"].as_f64().unwrap_or(1.0) as f32;
        stage.renderer.set_display_settings(DisplaySettings {
            exposure,
            tone_mapping_exposure: 1.0,
        });
        let recording = &scene.recording;
        let hull_set = Hulls::new(recording);
        let frame_dts: Vec<f64> = meta["frameDts"]
            .as_array()
            .map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect())
            .unwrap_or_default();
        let frame_cameras = meta["frameCameras"].as_array().cloned().unwrap_or_default();
        let sea_height = meta["shipWake"]["seaHeight"].as_f64().unwrap_or(0.0);
        let mut captures: Vec<&Value> = meta["captures"]
            .as_array()
            .map(|a| a.iter().collect())
            .unwrap_or_default();
        captures.sort_by_key(|c| c["recordingFrames"].as_u64().unwrap_or(0));
        let mut cursor = 0usize;
        let started = Instant::now();
        let mut last_capture: Option<(Camera, String)> = None;
        for capture in captures {
            let until = capture["recordingFrames"].as_u64().unwrap_or(0) as usize;
            while cursor < until.min(recording.frames.len()) {
                let frame = &recording.frames[cursor];
                let dt = frame_dts.get(cursor).copied().unwrap_or(recording.dt);
                let ships = hull_set.wake_ships(frame);
                let splashes = drive::splashes(frame);
                let torpedoes = drive::torpedoes(frame);
                let camera = frame_cameras
                    .get(cursor)
                    .map(wake_camera)
                    .unwrap_or(WakeCamera {
                        eye: [0.0, 50.0, 0.0],
                        forward: [0.0, 0.0, -1.0],
                        fov_y: 52f64.to_radians(),
                        zoom: 2.05,
                        drawing_height: f64::from(HEIGHT),
                    });
                // The particles' view of the frame: a staged camera's own orientation, else the chase rig's aim at the
                // focus hull (the page recorded where its camera was, not where it looked).
                let entry = frame_cameras.get(cursor);
                let get =
                    |i: usize, default: f64| entry.and_then(|e| e[i].as_f64()).unwrap_or(default);
                let eye = Vec3::new(get(0, 0.0) as f32, get(1, 50.0) as f32, get(2, 0.0) as f32);
                let fov = get(3, 52.0) as f32;
                let target = match orientations
                    .iter()
                    .find(|(position, _)| position.distance(eye) < 0.01)
                {
                    Some((_, rotation)) => eye + *rotation * Vec3::new(0.0, 0.0, -100.0),
                    None => frame.ships.first().map_or(Vec3::ZERO, |s| {
                        vec3_of(s.position) + Vec3::new(0.0, 10.0, 0.0)
                    }),
                };
                let effects_camera = CameraView::from_render(
                    &Camera {
                        eye,
                        target,
                        fov_y_degrees: fov,
                        near: 0.5,
                        far: far as f32,
                        curvature: naval_render::camera::EARTH_CURVATURE,
                    },
                    WIDTH,
                    HEIGHT,
                );
                let effect_ships: Vec<EffectShip<'_>> = frame
                    .ships
                    .iter()
                    .filter_map(|pose| {
                        let (hull, _, profile) = hull_set.get(&pose.id)?;
                        Some(EffectShip {
                            pose,
                            hull,
                            profile,
                            visible: true,
                            lost: false,
                            section: None,
                        })
                    })
                    .collect();
                let crest = ocean
                    .wake
                    .as_ref()
                    .map(|w| CrestShape::from(&w.bow_waves.tuning));
                effects.update(&EffectsFrame {
                    dt,
                    camera: effects_camera,
                    ships: &effect_ships,
                    events: &frame.events,
                    sea: recording.sea.map(|sea| (sea, frame.time)),
                    waves: sea_height,
                    crest,
                    lighting,
                });
                if let Some(wake) = ocean.wake.as_mut() {
                    wake.update(&WakeInputs {
                        ships: &ships,
                        dt,
                        splashes: &splashes,
                        torpedoes: &torpedoes,
                        camera,
                        sea_height,
                    });
                }
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                ocean.prepare_wake(&gpu.device, &gpu.queue, &mut encoder);
                gpu.queue.submit([encoder.finish()]);
                cursor += 1;
            }
            freeze_waves(&gpu, &mut ocean);
            // The hulls as the last replayed frame posed them.
            let last = &recording.frames[cursor.saturating_sub(1).min(recording.frames.len() - 1)];
            let draws: Vec<HullDraw> = last
                .ships
                .iter()
                .filter_map(|pose| {
                    let (_, shape, _) = hull_set.get(&pose.id)?;
                    let matrix = pose.matrix();
                    let model = Mat4::from_cols_array(&matrix.to_cols_array().map(|v| v as f32));
                    Some(HullDraw {
                        shape: *shape,
                        model,
                    })
                })
                .collect();
            hulls.set_hulls(&gpu.device, &gpu.queue, &draws, hull_lighting);
            let c = &capture["camera"];
            let eye = vec3(&c["position"]);
            // The sea around the hulls as the page drew it (`hullSeaFlag`: `on` is the game as it ships, `off` left the
            // sea unblended): combat's sea at the last replayed frame's time about the camera (`Game.coupleHullSea`).
            let coupled = meta["hullSeaFlag"].as_str() == Some("on") && !uncoupled;
            match recording.sea.filter(|_| coupled) {
                Some(sea) => {
                    let hulls: Vec<SeaHull> = last
                        .ships
                        .iter()
                        .filter_map(|pose| {
                            let (hull, _, _) = hull_set.get(&pose.id)?;
                            Some(SeaHull {
                                x: pose.position[0],
                                z: pose.position[2],
                                heading: pose.heading,
                                length: hull.length,
                                beam: hull.beam,
                                center_x: hull.center_x,
                                center_z: hull.center_z,
                                contact: 1.0,
                            })
                        })
                        .collect();
                    let origin = (f64::from(eye.x), f64::from(eye.z));
                    let footprints = hull_footprints(&hulls, origin, HULL_SEA_SLOTS);
                    let waves = [
                        HullSeaWave {
                            amplitude: sea.amplitude_m * 0.7,
                            wavelength: sea.wavelength_m,
                            direction: sea.direction,
                            phase: sea.phase,
                        },
                        HullSeaWave {
                            amplitude: sea.amplitude_m * 0.3,
                            wavelength: sea.wavelength_m * 0.57,
                            direction: sea.direction + 0.8,
                            phase: sea.phase + 2.0,
                        },
                    ];
                    // The page couples at the simulation's presentation time, which the recording's last frame rounds.
                    let time = meta["simulation"]["presentationTime"]
                        .as_f64()
                        .unwrap_or(last.time);
                    ocean.set_hull_sea(&waves, time, &footprints, origin);
                }
                None => ocean.set_hull_sea(&[], 0.0, &[], (0.0, 0.0)),
            }
            // The camera's own orientation (a staged camera has no target): three looks down its local −Z.
            let q = &c["quaternion"];
            let rotation = glam::Quat::from_xyzw(
                q[0].as_f64().unwrap_or(0.0) as f32,
                q[1].as_f64().unwrap_or(0.0) as f32,
                q[2].as_f64().unwrap_or(0.0) as f32,
                q[3].as_f64().unwrap_or(1.0) as f32,
            );
            let camera = Camera {
                eye,
                target: eye + rotation * Vec3::new(0.0, 0.0, -100.0),
                fov_y_degrees: c["fov"].as_f64().unwrap_or(50.0) as f32,
                near: c["near"].as_f64().unwrap_or(0.5) as f32,
                far: c["far"].as_f64().unwrap_or(60000.0) as f32,
                curvature: naval_render::camera::EARTH_CURVATURE,
            };
            {
                let layers: &mut [&mut dyn SceneLayer] =
                    &mut [&mut hulls, &mut ocean, &mut sky, &mut effects];
                stage.render(layers, &camera, 0, 0.0)?;
            }
            let file = capture["file"].as_str().unwrap_or("capture.png");
            let native_path = root.join(format!("native{suffix}")).join(file);
            stage.capture.save_png(&gpu, &native_path)?;
            let web_path = web.join(file);
            if web_path.exists() {
                side_by_side(
                    &web_path,
                    &native_path,
                    &root.join(format!("side-by-side{suffix}")).join(file),
                )?;
                let result = compare(
                    &web_path,
                    &native_path,
                    &camera,
                    file,
                    &root.join(format!("diff{suffix}")).join(file),
                )?;
                println!("  {result}");
                comparisons.push(result);
            }
            last_capture = Some((camera, file.to_owned()));
        }
        println!(
            "  replayed {cursor} frames in {:.1} s",
            started.elapsed().as_secs_f64()
        );
        // Timings step the wake field and repaint the atlas beyond the recording, so they run once every capture of the
        // scene is written, from its last camera.
        if frames > 0
            && let Some((camera, file)) = &last_capture
        {
            timings.push(time_frames(
                &mut stage,
                &mut ocean,
                &mut hulls,
                &mut sky,
                &mut effects,
                camera,
                frames,
                file,
            )?);
        }
    }
    if !comparisons.is_empty() {
        std::fs::write(
            root.join(format!("compare{suffix}.json")),
            serde_json::to_string_pretty(&comparisons)?,
        )?;
    }
    if !timings.is_empty() {
        std::fs::write(
            root.join("timings.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "adapter": info.name, "backend": format!("{:?}", info.backend), "size": [WIDTH, HEIGHT], "tier": "high",
                "captures": timings,
            }))?,
        )?;
    }
    println!("stills in {}", root.display());
    Ok(())
}

/// Serialised wall-clock frames through the renderer with the wakes shaded and without (the surface bound to a calm
/// wake), and the wake's GPU half alone (field steps, atlas repaints, uploads) for frames that step.
#[allow(clippy::too_many_arguments)]
fn time_frames(
    stage: &mut Stage<'_>,
    ocean: &mut Ocean,
    hulls: &mut HullLayer,
    sky: &mut StandInSky,
    effects: &mut WaterEffects,
    camera: &Camera,
    frames: u32,
    name: &str,
) -> Result<Value> {
    let gpu = stage.gpu;
    let wait = || gpu.device.poll(wgpu::PollType::wait_indefinitely());
    let median = |values: &mut Vec<f64>| {
        values.sort_by(|a, b| a.total_cmp(b));
        values.get(values.len() / 2).copied()
    };
    let mut with = Vec::new();
    let mut without = Vec::new();
    let saved = ocean.wake.take();
    let mut wake = saved;
    // Alternate with and without, so load from other work on the machine falls on both.
    for i in 0..(frames + 10) * 2 {
        let shaded = i % 2 == 0;
        if shaded {
            ocean.wake = wake.take();
        } else {
            wake = ocean.wake.take();
        }
        ocean.prepare_wake(
            &gpu.device,
            &gpu.queue,
            &mut gpu.device.create_command_encoder(&Default::default()),
        );
        let started = Instant::now();
        let layers: &mut [&mut dyn SceneLayer] =
            &mut [&mut *hulls, &mut *ocean, &mut *sky, &mut *effects];
        stage.render(layers, camera, u64::from(i), 0.0)?;
        wait()?;
        if i >= 20 {
            if shaded { &mut with } else { &mut without }
                .push(started.elapsed().as_secs_f64() * 1e3);
        }
    }
    if ocean.wake.is_none() {
        ocean.wake = wake;
    }
    ocean.prepare_wake(
        &gpu.device,
        &gpu.queue,
        &mut gpu.device.create_command_encoder(&Default::default()),
    );
    // The wake field's steps and the trail atlas's repaints alone, as the web's ocean-wake diagnostic measures its steps:
    // a batch of 30 submitted at once and timed to completion, the median of nine batches.
    let (mut steps, mut repaints) = (Vec::new(), Vec::new());
    if let Some(wake) = ocean.wake.as_mut() {
        for _ in 0..9 {
            let started = Instant::now();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            if let Some(field) = wake.field.as_mut() {
                for _ in 0..30 {
                    field.update(&gpu.queue, &mut encoder, naval_ocean::wake::STEP);
                }
            }
            gpu.queue.submit([encoder.finish()]);
            wait()?;
            steps.push(started.elapsed().as_secs_f64() * 1e3 / 30.0);
            let started = Instant::now();
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            for _ in 0..30 {
                wake.repaint_trails(&gpu.device, &gpu.queue, &mut encoder);
            }
            gpu.queue.submit([encoder.finish()]);
            wait()?;
            repaints.push(started.elapsed().as_secs_f64() * 1e3 / 30.0);
        }
    }
    let stamps = ocean.wake.as_ref().map_or(0, |w| w.diagnostics().0);
    let result = serde_json::json!({
        "capture": name,
        "frames": frames,
        "serialisedFrameMs": median(&mut with),
        "serialisedFrameWithoutWakesMs": median(&mut without),
        "wakeFieldMsPerStep": median(&mut steps),
        "trailRepaintMs": median(&mut repaints),
        "trailStamps": stamps,
    });
    println!("  {result}");
    Ok(result)
}

/// How far the native still is from the web's below the horizon: each side's mean sRGB colour, the mean absolute
/// difference per channel and the share of pixels more than 24 levels apart; a difference image (red where native is
/// brighter, blue where darker, ×4) to `diff`.
fn compare(web: &Path, native: &Path, camera: &Camera, name: &str, diff: &Path) -> Result<Value> {
    let web = image::open(web)?.to_rgb8();
    let native = image::open(native)?.to_rgb8();
    let (width, height) = web.dimensions();
    let direction = camera.target - camera.eye;
    let pitch = direction.y.atan2(direction.x.hypot(direction.z));
    let focal = 1.0 / (camera.fov_y_degrees.to_radians() / 2.0).tan();
    let horizon = ((height as f32 / 2.0 + pitch.tan() * focal * height as f32 / 2.0) as i64 + 3)
        .clamp(0, i64::from(height) - 1) as u32;
    let mut sums = [[0f64; 3]; 2];
    let (mut absolute, mut far_apart, mut count) = (0f64, 0u64, 0u64);
    let mut picture = image::RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let a = web.get_pixel(x, y).0;
            let b = native.get_pixel(x, y).0;
            let signed = (0..3)
                .map(|c| f32::from(b[c]) - f32::from(a[c]))
                .sum::<f32>()
                / 3.0;
            picture.put_pixel(
                x,
                y,
                image::Rgb([
                    (signed * 4.0).clamp(0.0, 255.0) as u8,
                    signed.abs().min(255.0) as u8,
                    (-signed * 4.0).clamp(0.0, 255.0) as u8,
                ]),
            );
            if y < horizon {
                continue;
            }
            let mut largest = 0u8;
            for c in 0..3 {
                sums[0][c] += f64::from(a[c]);
                sums[1][c] += f64::from(b[c]);
                let d = a[c].abs_diff(b[c]);
                absolute += f64::from(d);
                largest = largest.max(d);
            }
            far_apart += u64::from(largest > 24);
            count += 1;
        }
    }
    if let Some(parent) = diff.parent() {
        std::fs::create_dir_all(parent)?;
    }
    picture.save(diff)?;
    let n = count.max(1) as f64;
    let mean = |side: usize| sums[side].map(|s| (s / n * 10.0).round() / 10.0);
    Ok(serde_json::json!({
        "capture": name,
        "seaFromRow": horizon,
        "webMeanSrgb": mean(0),
        "nativeMeanSrgb": mean(1),
        "meanAbsoluteDifference": (absolute / (n * 3.0) * 100.0).round() / 100.0,
        "shareOver24Levels": (far_apart as f64 / n * 1e4).round() / 1e4,
    }))
}

/// Web on the left, native on the right, with a thin gap.
fn side_by_side(web: &Path, native: &Path, out: &Path) -> Result<()> {
    let left = image::open(web)?.to_rgba8();
    let right = image::open(native)?.to_rgba8();
    let gap = 8;
    let height = left.height().max(right.height());
    let mut canvas = image::RgbaImage::from_pixel(
        left.width() + gap + right.width(),
        height,
        image::Rgba([255, 255, 255, 255]),
    );
    image::imageops::overlay(&mut canvas, &left, 0, 0);
    image::imageops::overlay(&mut canvas, &right, i64::from(left.width() + gap), 0);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    canvas.save(out)?;
    Ok(())
}
