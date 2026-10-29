//! A splash column over a plain sea, frame by frame: `cargo run -p naval-water-effects --example column_sheet
//! --profile dev-client [-- --only side,near --size 640x360]`.
//!
//! Writes `.build/native/water-effects/columns/<view>.png`: a 38 cm splash (the shell falling at 25° from the west) at
//! 0.15, 0.4, 0.8, 1.5, 2.5, 4, 6, 9 and 13 s, from `side` (330 m off, 12 m up, the sun on the right),
//! `near` (140 m off, 4 m up, the sun over the left shoulder), `zoom` (240 m off through a 22° lens, the sun on the right and a little behind the column), `backlit` (the sun behind the column), `above` (from 160 m up), `salvo` (four 38 cm
//! and two 15 cm splashes from 600 m), `straddle` (six 38 cm splashes from a bridge 450 m off) and `offshore` (a
//! straddle 14 km from the origin, 6 km off through a 5° lens), on a contact sheet each. The backdrop is a flat sea and a sky gradient drawn
//! here, not the ocean: this checks the columns alone, quickly. `--times 1.5,2.5,5` selects ages; `--cost 30`
//! records warmed per-pass GPU samples beside the stills; `--out <folder>` keeps before/after runs separately.

use anyhow::Result;
use glam::DVec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu, build_dir};
use naval_effects::CameraView;
use naval_render::{
    Camera, DrawContext, FrameInput, FrameTime, Phase, PipelineId, RenderContext, RenderOutput,
    Renderer, SceneLayer, ScenePipeline, ShaderFile,
};
use naval_water_effects::fixture::{WaterEvent, WaterEventKind};
use naval_water_effects::{EffectLighting, EffectsFrame, WaterEffects};

const BACKDROP: ShaderFile = ShaderFile {
    path: "naval-water-effects/examples/column_backdrop.wgsl",
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
                "column backdrop sea",
                BACKDROP.path,
                open("sea_vertex", "sea_fragment", true),
            )
            .map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let sky = ctx
            .add_scene_pipeline(
                "column backdrop sky",
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

/// The afternoon light: the sun 35° up, from `bearing` (radians from +X toward +Z, the direction toward it).
fn lighting(bearing: f64) -> EffectLighting {
    let elevation = 35f64.to_radians();
    let sun = DVec3::new(
        bearing.cos() * elevation.cos(),
        elevation.sin(),
        bearing.sin() * elevation.cos(),
    );
    let mut lighting =
        EffectLighting::from_sky(sun, DVec3::new(1.0, 0.93, 0.82), 5.8, false, 0.65, false);
    lighting.set_wind(9.0, 60f64.to_radians());
    lighting
}

fn splash(sequence: u64, x: f64, z: f64, caliber: f64) -> WaterEvent {
    WaterEvent {
        sequence,
        kind: WaterEventKind::Splash,
        position: [x, 0.0, z],
        // From the west, falling at 25°.
        velocity: Some([450.0, -210.0, 0.0]),
        caliber_m: Some(caliber),
        shell_id: Some(1_000_000 + sequence as i64),
    }
}

struct View {
    name: &'static str,
    eye: [f32; 3],
    target: [f32; 3],
    fov: f32,
    sun_bearing: f64,
    events: Vec<WaterEvent>,
}

fn main() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    let value = |flag: &str| {
        arguments
            .iter()
            .position(|a| a == flag)
            .and_then(|i| arguments.get(i + 1))
            .cloned()
    };
    let only = value("--only");
    let (width, height) = value("--size")
        .and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((640u32, 360u32));
    let out = value("--out")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| build_dir("water-effects").join("columns"));
    let cost: usize = value("--cost").map(|s| s.parse()).transpose()?.unwrap_or(0);
    std::fs::create_dir_all(&out)?;
    let gpu = Gpu::headless_with(wgpu::Features::TIMESTAMP_QUERY)?;
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT)
        .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut backdrop = Backdrop::new(&mut renderer.context())?;
    let capture = Capture::new(&gpu, width, height);
    let heavy = || vec![splash(1, 0.0, 0.0, 0.38)];
    let views = [
        View {
            name: "side",
            eye: [60.0, 12.0, 330.0],
            target: [0.0, 30.0, 0.0],
            fov: 32.0,
            sun_bearing: 0.35,
            events: heavy(),
        },
        // The effects trial's `splash` scene: 271 m off, 9 m up, a 40° lens.
        View {
            name: "trial",
            eye: [-266.2, 9.0, -53.0],
            target: [0.0, 30.0, 0.0],
            fov: 40.0,
            sun_bearing: 2.2,
            events: heavy(),
        },
        View {
            name: "zoom",
            eye: [40.0, 18.0, 240.0],
            target: [0.0, 32.0, 0.0],
            fov: 22.0,
            sun_bearing: -0.3,
            events: heavy(),
        },
        View {
            name: "near",
            eye: [30.0, 4.0, 140.0],
            target: [0.0, 32.0, 0.0],
            fov: 50.0,
            sun_bearing: 2.2,
            events: heavy(),
        },
        View {
            name: "backlit",
            eye: [60.0, 12.0, 330.0],
            target: [0.0, 30.0, 0.0],
            fov: 32.0,
            sun_bearing: -1.6,
            events: heavy(),
        },
        View {
            name: "above",
            eye: [120.0, 160.0, 140.0],
            target: [0.0, 10.0, 0.0],
            fov: 45.0,
            sun_bearing: 2.2,
            events: heavy(),
        },
        // A straddle seen from a bridge, 450 m off: six 38 cm splashes.
        View {
            name: "straddle",
            eye: [0.0, 15.0, 450.0],
            target: [0.0, 30.0, 0.0],
            fov: 30.0,
            sun_bearing: 2.2,
            events: vec![
                splash(1, -150.0, 30.0, 0.38),
                splash(2, -95.0, -40.0, 0.38),
                splash(3, -35.0, 15.0, 0.38),
                splash(4, 20.0, -25.0, 0.38),
                splash(5, 80.0, 40.0, 0.38),
                splash(6, 140.0, -10.0, 0.38),
            ],
        },
        // A straddle 14 km from the scene's origin (a theatre's render origin re-centres only past 16 km), seen 6 km
        // off through binoculars' 5° lens: rays that kept the eye's f32 rounding drew these as banded slabs.
        View {
            name: "offshore",
            eye: [14_000.0, 20.0, -2_000.0],
            target: [14_000.0, 40.0, -8_000.0],
            fov: 5.0,
            sun_bearing: 2.2,
            events: vec![
                splash(1, 13_850.0, -8_030.0, 0.38),
                splash(2, 13_905.0, -7_960.0, 0.38),
                splash(3, 13_965.0, -8_015.0, 0.38),
                splash(4, 14_020.0, -7_975.0, 0.38),
                splash(5, 14_080.0, -8_040.0, 0.38),
                splash(6, 14_140.0, -7_990.0, 0.38),
            ],
        },
        View {
            name: "salvo",
            eye: [150.0, 20.0, 650.0],
            target: [0.0, 30.0, 0.0],
            fov: 32.0,
            sun_bearing: 2.2,
            events: vec![
                splash(1, -60.0, 20.0, 0.38),
                splash(2, -15.0, -30.0, 0.38),
                splash(3, 35.0, 10.0, 0.38),
                splash(4, 80.0, -20.0, 0.38),
                splash(5, -120.0, 60.0, 0.152),
                splash(6, 130.0, 50.0, 0.152),
            ],
        },
    ];
    let times: Vec<f64> = value("--times")
        .map(|s| s.split(',').map(str::parse).collect())
        .transpose()?
        .unwrap_or_else(|| vec![0.15, 0.4, 0.8, 1.5, 2.5, 4.0, 6.0, 9.0, 13.0]);
    anyhow::ensure!(
        !times.is_empty()
            && times.iter().all(|t| t.is_finite() && *t >= 0.0)
            && times.windows(2).all(|t| t[0] <= t[1]),
        "--times takes increasing nonnegative seconds"
    );
    anyhow::ensure!(
        cost == 0 || renderer.gpu_timer().supported(),
        "GPU timestamps are unavailable"
    );
    let mut frame = 0u64;
    for view in views {
        if let Some(only) = &only
            && !only.split(',').any(|name| name == view.name)
        {
            continue;
        }
        let mut effects =
            WaterEffects::new(&mut renderer.context()).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        let camera = Camera {
            eye: glam::Vec3::from(view.eye),
            target: glam::Vec3::from(view.target),
            fov_y_degrees: view.fov,
            near: 0.5,
            far: 60000.0,
            curvature: naval_render::camera::EARTH_CURVATURE,
        };
        let seen = CameraView::from_render(&camera, width, height);
        let light = lighting(view.sun_bearing);
        let mut stills = Vec::new();
        let mut costs = Vec::new();
        let mut time = 0.0;
        let mut first = true;
        for &at in &times {
            while time < at - 1e-9 || first {
                let fired = if first { &view.events[..] } else { &[] };
                effects.update(&EffectsFrame {
                    dt: if first { 0.0 } else { 1.0 / 60.0 },
                    camera: seen,
                    ships: &[],
                    events: fired,
                    sea: None,
                    waves: 0.0,
                    crest: None,
                    lighting: light,
                });
                if !first {
                    time += 1.0 / 60.0;
                }
                first = false;
                frame += 1;
            }
            let mut samples: std::collections::BTreeMap<String, Vec<f64>> = Default::default();
            for redraw in 0..(if cost == 0 { 1 } else { cost + 6 }) {
                renderer
                    .render(
                        FrameInput {
                            camera: &camera,
                            time: FrameTime {
                                seconds: frame as f64 / 60.0,
                                dt: 1.0 / 60.0,
                                frame,
                            },
                        },
                        &mut [&mut backdrop, &mut effects],
                        RenderOutput {
                            view: &capture.view,
                            format: CAPTURE_FORMAT,
                            width,
                            height,
                        },
                    )
                    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
                if cost > 0 {
                    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                    renderer.drain_gpu_timings();
                    if redraw >= 6 {
                        let timing = renderer.gpu_timer().latest().expect("GPU timestamps");
                        for pass in &timing.passes {
                            if let Some(ms) = pass.serial_ms {
                                samples.entry(pass.label.clone()).or_default().push(ms);
                            }
                        }
                    }
                }
            }
            if cost > 0 {
                costs.push(serde_json::json!({"seconds": at, "passes_ms": samples}));
            }
            let path = out.join(format!("{}-{at:.2}s.png", view.name));
            capture.save_png(&gpu, &path)?;
            stills.push(image::open(&path)?.to_rgba8());
        }
        if cost > 0 {
            std::fs::write(
                out.join(format!("{}-cost.json", view.name)),
                serde_json::to_vec_pretty(
                    &serde_json::json!({"size": [width, height], "redraws": cost, "moments": costs}),
                )?,
            )?;
        }
        // Three across.
        let columns = 3;
        let rows = stills.len().div_ceil(columns);
        let mut sheet = image::RgbaImage::new(width * columns as u32, height * rows as u32);
        for (i, still) in stills.iter().enumerate() {
            let (x, y) = ((i % columns) as u32 * width, (i / columns) as u32 * height);
            image::imageops::overlay(&mut sheet, still, i64::from(x), i64::from(y));
        }
        sheet.save(out.join(format!("{}.png", view.name)))?;
        println!(
            "{}: {}",
            view.name,
            out.join(format!("{}.png", view.name)).display()
        );
    }
    Ok(())
}
