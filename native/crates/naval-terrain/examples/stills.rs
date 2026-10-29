//! The test land (`tests/support.rs`: cliffs, a wooded scarp, lowland and a hill, built in code since the battle maps'
//! land was removed on 2026-10-27) in each land style from the terrain review's fixed cameras, drawn natively in the
//! game frame (the North Atlantic's sky with its clouds, the sea, the land and its trees, bloom and the eye
//! adaptation) (native only):
//!
//!     cargo run -p naval-terrain --example stills --release [-- --styles chalk,tropical] [--presets coast3,aerial]
//!         [--size 1600x900] [--frames 50] [--web web-native-compare] [--debug sun] [--quality medium] [--casters]
//!         [--tree-cost] [--shadow-cost] [--part-cost near-full] [--passes] [--pairs 24] [--tree-shadows off]
//!         [--sun 12,100] [--weather clear]
//!         [--out <dir>] [--atlas] [--climate <lat>,<lon>,<leaf>,<dry>[,<snowline>]] [--materials off]
//!         [--material-cost] [--material-side 1024]
//!
//! The cameras are found from the field as the removed web game's terrain review found them (`naval_terrain::review`).
//! Writes `.build/native/terrain/native/<style>-<preset>.png`, and with captures of the same names in
//! `.build/reviews/terrain/<web>/`, the capture-left / native-right pairs in `.build/native/terrain/side-by-side/` and
//! `compare.json`; `timings.json` holds the land's
//! GPU cost (frames with and without it, serialised; with the device's timestamps where it takes them, each side's tenth
//! percentile of the frames' GPU busy time, since another process's work on the GPU only adds time, with the median of
//! the pairs' differences beside it; else the wall clock). `--tree-cost` times the land with its trees against the same land
//! without them instead (`tree-timings.json`), `--shadow-cost` the trees casting into the sun's maps against the forest's
//! shade standing in (`tree-shadow-timings.json`), `--material-cost` the land with its ground materials shown against the
//! same land with them hidden (`material-timings.json`), `--part-cost` one of the trees' parts in the scene shown against
//! hidden (`near-full`, `near-reduced`, `impostors`, `grass`: `TreePart`; `part-timings.json`). `--passes` also prints
//! each timed pass's tenth percentile with and without, and their difference (the shadow maps, the scene, each compute
//! pass apart). `--moving` moves the eye a millimetre a frame, so the land chooses its patches and places its trees and
//! grass every frame, as under a moving camera (a still camera places them once). Besides the review's presets, `air300`, `air1500` (an aircraft off the
//! main coast at 300 m and 1,500 m), `ship6` (a ship's bridge 6 km off it), `inland` (500 m over the land, along the
//! coast's bearing), `canopy`, `grove` and `shore` are this example's own views, for the trees. `--sun` sets the sun
//! (elevation and azimuth, degrees). `--atlas` writes the trees' impostor atlas as the first style's land baked it
//! (`tree-atlas.png`, `tree-atlas-normals.png`) and the wall time of that land's first two frames (the first bakes it).
//! `--climate` draws the land where and when a theatre's would be, its trees chosen by that place's biome. The device
//! takes BC where the adapter has it, as the game's does, so the trees draw from the impostor atlas's BC7 copy (made
//! after the first frame, waited for); `--rgba8` keeps the RGBA8 bake, to compare.

#[path = "common/scene.rs"]
mod scene;
#[path = "../tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::{Context, Result};
use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_clouds::{GameSky, Targets};
use naval_conditions::MapSelection;
use naval_ocean::{Ocean, QualityTier};
use naval_render::{
    BloomSettings, Camera, EyeAdaptationSettings, FrameInput, FrameTime, RenderOutput, Renderer,
    SceneLayer, ShadowSettings, SkyMean, Sun,
};
use naval_sky::SkyQuality;
use naval_terrain::review::{self, PRESETS};
use naval_terrain::textures::DETAIL_PHOTOS;
use naval_terrain::{
    Heightfield, TerrainDebug, TerrainLayer, TerrainLight, TerrainQuality, TerrainSettings,
    TreePart,
};
use scene::{SeaInWater, SkyMean as SkyMeanLayer, SkyWithPost, side_by_side};

/// The review page's sea and sky clock (`SEA_TIME`).
const SEA_TIME: f64 = 60.0;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let list =
        |name: &str| option(name).map(|v| v.split(',').map(str::to_owned).collect::<Vec<_>>());
    let styles = list("--styles").unwrap_or_else(|| {
        ["chalk", "tropical", "snow", "volcanic", "rock"]
            .map(str::to_owned)
            .to_vec()
    });
    // `overview` (30 km up) is left out by default: the removed web game drew neither land nor sea from there
    // (see docs/archive/terrain-history.md, "How it matched the web").
    let presets = list("--presets").unwrap_or_else(|| {
        PRESETS
            .iter()
            .filter(|p| **p != "overview")
            .map(|p| (*p).to_owned())
            .collect()
    });
    let (width, height) = option("--size")
        .and_then(|s| {
            s.split_once('x')
                .map(|(w, h)| (w.parse().unwrap(), h.parse().unwrap()))
        })
        .unwrap_or((1600u32, 900u32));
    let frames: u64 = option("--frames")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let web_tag = option("--web").unwrap_or_else(|| "web-native-compare".into());
    let debug = option("--debug")
        .and_then(|d| TerrainDebug::by_name(&d))
        .unwrap_or_default();
    let quality = option("--quality")
        .and_then(|q| TerrainQuality::by_name(&q))
        .unwrap_or(TerrainQuality::HIGH);
    let casters = args.iter().any(|a| a == "--casters");
    let tree_cost = args.iter().any(|a| a == "--tree-cost");
    // `--shadow-cost` times the land with its trees casting into the sun's maps against the same land with the
    // forest's shade standing in everywhere (`set_tree_shadows(false)`), which `--tree-shadows off` draws.
    let shadow_cost = args.iter().any(|a| a == "--shadow-cost");
    let tree_shadows = option("--tree-shadows").is_none_or(|v| v != "off");
    // The ground materials (`naval_terrain::ground_materials`), RGBA8 at `--material-side` (1024: the client's BC7 is
    // the same texels at a quarter of the bytes); `--materials off` draws the land without them, and `--material-cost`
    // times the land with them shown against the same land with them hidden (`material-timings.json`).
    let materials_on = option("--materials").is_none_or(|v| v != "off");
    let material_cost = args.iter().any(|a| a == "--material-cost");
    // `--part-cost <part>`: one of the trees' parts in the scene shown against hidden (`TreePart::by_name`).
    let part_cost = option("--part-cost").map(|name| {
        TreePart::by_name(&name).unwrap_or_else(|| panic!("--part-cost: no part {name}"))
    });
    let passes_shown = args.iter().any(|a| a == "--passes");
    let moving = args.iter().any(|a| a == "--moving");
    let material_side: u32 = option("--material-side")
        .and_then(|v| v.parse().ok())
        .unwrap_or(naval_terrain::ground_materials::MATERIAL_SIDE);
    // Alternating pairs of timed frames (`--pairs`, 24 by default).
    let pairs: u64 = option("--pairs").and_then(|v| v.parse().ok()).unwrap_or(24);

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let build = native_offscreen::build_dir("terrain");
    let native_dir = option("--out").map_or_else(|| build.join("native"), |d| build.join(d));
    let pairs_dir = build.join("side-by-side");
    std::fs::create_dir_all(&native_dir)?;
    std::fs::create_dir_all(&pairs_dir)?;
    let web_dir = root.join(".build/reviews/terrain").join(&web_tag);
    let photos: Vec<Vec<u8>> = DETAIL_PHOTOS
        .iter()
        .map(|path| std::fs::read(root.join(path)).with_context(|| path.to_string()))
        .collect::<Result<_>>()?;
    let materials = if materials_on || material_cost {
        let files: Vec<Vec<u8>> = naval_terrain::ground_materials::material_files()
            .iter()
            .map(|path| std::fs::read(root.join(path)).with_context(|| path.to_string()))
            .collect::<Result<_>>()?;
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let started = Instant::now();
        let materials = naval_terrain::GroundMaterials::from_sources(&refs, material_side)
            .map_err(anyhow::Error::msg)?;
        println!(
            "ground materials: {} layers of {material_side}², {:.1} MB, built in {:.0} ms",
            materials.layers,
            materials.bytes() as f64 / 1e6,
            started.elapsed().as_secs_f64() * 1000.0
        );
        Some(materials)
    } else {
        None
    };

    // `--atlas`: the trees' impostor atlas as the first style's land baked it, then stop.
    let atlas = args.iter().any(|a| a == "--atlas");
    let rgba8 = args.iter().any(|a| a == "--rgba8");
    let gpu = timed_gpu()?;
    let capture = Capture::new(&gpu, width, height);
    let mut compare = BTreeMap::new();
    let mut timings = BTreeMap::new();
    for style_name in &styles {
        let style: naval_conditions::LandStyle =
            serde_json::from_value(serde_json::json!(style_name)).with_context(|| {
                format!("--styles takes chalk, tropical, snow, volcanic and rock; got {style_name}")
            })?;
        let mut selection = MapSelection::battle(naval_conditions::DEFAULT_MAP);
        // `--sun <elevation>,<azimuth>` (degrees; the sky's azimuth before the map's bearing turns it) and `--weather`.
        if let Some((elevation, azimuth)) =
            option("--sun").as_deref().and_then(|s| s.split_once(','))
        {
            selection.conditions.sun = Some(naval_conditions::Sun {
                elevation: elevation.parse()?,
                azimuth: azimuth.parse()?,
                morning: true,
            });
        }
        if let Some(weather) = option("--weather") {
            selection.weather = weather;
        }
        let mut selection = support::styled(selection.resolve().context("map")?, style);
        // `--climate <lat>,<lon>,<leaf>,<dry>[,<snowline>]`: the land drawn where and when a theatre's would be, its
        // trees chosen by that place's biome (`species::Biome`) rather than the style's own.
        if let Some(values) = option("--climate") {
            let v: Vec<f32> = values
                .split(',')
                .map(|x| x.parse())
                .collect::<Result<_, _>>()?;
            selection.style.climate = naval_conditions::Climate {
                snowline_m: v.get(4).copied().unwrap_or(1500.0),
                thaw: 1.0,
                leaf: v[2],
                dry: v[3],
                place: Some([v[0], v[1]]),
            };
        }
        let field = support::land();

        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT)?;
        renderer.set_bloom(Some(BloomSettings::default()));
        renderer.set_eye_adaptation(EyeAdaptationSettings {
            enabled: true,
            ..Default::default()
        });
        renderer.set_sky_mean(SkyMean::Gpu);
        // The High preset: medium shadows (land and trees receive; the land with --casters casts).
        renderer
            .set_shadow_settings(ShadowSettings::for_quality("medium").expect("a shadow preset"));
        let mut sky = GameSky::new(
            &gpu.device,
            &gpu.queue,
            SkyQuality::Medium,
            Targets::default(),
        );
        sky.apply(&gpu.queue, &selection.sky);
        sky.hold(Some(SEA_TIME));
        let environment = sky.sky.environment();
        let mut ctx = renderer.context();
        let mean = SkyMeanLayer::new(&mut ctx, &sky)?;
        let mut ocean = Ocean::new(
            &gpu.device,
            &gpu.queue,
            QualityTier::High,
            ctx.camera_layout,
            &environment.bind_group_layout,
            &environment.bind_group,
            60000.0,
        );
        ocean.parameters = selection.ocean;
        let started = Instant::now();
        // `--atlas` reads the impostor atlas back, and `--rgba8` draws it as baked: neither copies it to BC7.
        let trees = if atlas || rgba8 {
            naval_terrain::TreeAtlas::uncompressed(&gpu.device)
        } else {
            naval_terrain::TreeAtlas::new(&gpu.device)
        };
        let mut terrain = TerrainLayer::for_scene_sharing(
            &mut ctx,
            naval_terrain::PreparedLand::new(&field),
            &selection,
            TerrainSettings {
                quality,
                cast_shadows: casters,
                debug,
                ..TerrainSettings::default()
            },
            &environment.bind_group,
            Some(&trees),
        )?;
        let built = started.elapsed().as_secs_f64() * 1000.0;
        terrain
            .set_detail_photos(&gpu.queue, [&photos[0], &photos[1], &photos[2], &photos[3]])
            .map_err(anyhow::Error::msg)?;
        terrain.set_cloud_shadow(Some(&sky.clouds.shadow().bind_group));
        terrain.set_tree_shadows(tree_shadows);
        if let Some(materials) = &materials
            && terrain.takes_ground_materials()
        {
            terrain
                .set_ground_materials(&mut ctx, materials)
                .map_err(anyhow::Error::msg)?;
            terrain.show_ground_materials(materials_on);
            if let Some(strength) = option("--material-strength").and_then(|v| v.parse().ok()) {
                terrain.set_ground_material_strength(strength);
            }
        }
        // The same land without its trees, for `--tree-cost`.
        let mut bare = if tree_cost {
            let mut bare = TerrainLayer::new(
                &mut ctx,
                &field,
                &selection,
                TerrainSettings {
                    quality: TerrainQuality {
                        trees: false,
                        ..quality
                    },
                    cast_shadows: casters,
                    debug,
                    ..TerrainSettings::default()
                },
                &environment.bind_group,
            )?;
            bare.set_detail_photos(&gpu.queue, [&photos[0], &photos[1], &photos[2], &photos[3]])
                .map_err(anyhow::Error::msg)?;
            bare.set_cloud_shadow(Some(&sky.clouds.shadow().bind_group));
            // The same ground materials, so the trees are all that differs.
            if let Some(materials) = &materials
                && bare.takes_ground_materials()
            {
                bare.set_ground_materials(&mut ctx, materials)
                    .map_err(anyhow::Error::msg)?;
                bare.show_ground_materials(materials_on);
                if let Some(strength) = option("--material-strength").and_then(|v| v.parse().ok()) {
                    bare.set_ground_material_strength(strength);
                }
            }
            Some(bare)
        } else {
            None
        };
        // The sea's foam grown as the ocean lane's stills grow it: wiped at t = 50, then 300 updates to t = 60.
        {
            let mut encoder = gpu.device.create_command_encoder(&Default::default());
            ocean.time = SEA_TIME - 10.0 - 1.0 / 30.0;
            ocean.update(&gpu.queue, &mut encoder, 1000.0);
            ocean.time = SEA_TIME - 10.0;
            for _ in 0..300 {
                ocean.update(&gpu.queue, &mut encoder, 1.0 / 30.0);
            }
            ocean.time = SEA_TIME;
            gpu.queue.submit([encoder.finish()]);
        }
        let mut sea = SeaInWater::new(ocean);
        let mut sky = SkyWithPost(sky);
        let mut mean = mean;
        println!("{style_name}: land built in {built:.0} ms");

        for preset in &presets {
            let Some(view) = review::preset(&field, selection.offset, preset)
                .or_else(|| foliage_view(&field, selection.offset, preset))
            else {
                continue;
            };
            let camera = Camera {
                eye: Vec3::new(view.eye[0] as f32, view.eye[1] as f32, view.eye[2] as f32),
                target: Vec3::new(
                    view.target[0] as f32,
                    view.target[1] as f32,
                    view.target[2] as f32,
                ),
                fov_y_degrees: view.fov as f32,
                // The close views stand an eye among the grass.
                near: if preset.starts_with("at:") { 0.1 } else { 0.5 },
                far: view.far.unwrap_or(60000.0) as f32,
                curvature: naval_render::camera::EARTH_CURVATURE,
            };
            sky.0.reset_history();
            let render = |renderer: &mut Renderer,
                          frame: u64,
                          layers: &mut [&mut dyn SceneLayer]|
             -> Result<()> {
                // `--moving`: a millimetre a frame along the view.
                let step = if moving {
                    (camera.target - camera.eye).normalize_or_zero() * (frame as f32 * 0.001)
                } else {
                    Vec3::ZERO
                };
                let camera = Camera {
                    eye: camera.eye + step,
                    target: camera.target + step,
                    ..camera
                };
                renderer.render(
                    FrameInput {
                        camera: &camera,
                        time: FrameTime {
                            seconds: 0.0,
                            dt: 0.0,
                            frame,
                        },
                    },
                    layers,
                    RenderOutput {
                        view: &capture.view,
                        format: CAPTURE_FORMAT,
                        width,
                        height,
                    },
                )?;
                Ok(())
            };
            for frame in 0..frames {
                // The sun's maps centre where the camera looks.
                let light = sky.0.light();
                renderer.set_sun(Some(Sun::new(light.direction, camera.target)));
                let light = TerrainLight {
                    light: sky.0.light(),
                    ambient: selection.ambient as f32,
                    flash: sky.0.lightning().flash,
                };
                terrain.set_light(&light);
                if let Some(bare) = bare.as_mut() {
                    bare.set_light(&light);
                }
                if frame + 1 == frames {
                    renderer.snap_eye_adaptation();
                }
                let mut layers: [&mut dyn SceneLayer; 4] =
                    [&mut sky, &mut mean, &mut terrain, &mut sea];
                gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                let start = Instant::now();
                render(&mut renderer, frame, &mut layers)?;
                gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                // The first frame bakes the atlas: the frames after draw its BC7 copy, as the game's do a second on.
                if frame == 0 {
                    trees.wait_compressed();
                }
                // The layer's first frame bakes the trees' impostors: its wall time against the next one's.
                if atlas && frame < 2 {
                    println!(
                        "frame {frame}: {:.1} ms",
                        start.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
            let name = format!(
                "{style_name}-{}{}",
                preset.replace(':', "_"),
                if debug == TerrainDebug::Off {
                    String::new()
                } else {
                    format!("-{debug:?}").to_lowercase()
                }
            );
            let path = native_dir.join(format!("{name}.png"));
            capture.save_png(&gpu, &path)?;
            if atlas {
                if let Some(texture) = terrain.tree_impostors() {
                    for (half, file) in [(0, "tree-atlas.png"), (1, "tree-atlas-normals.png")] {
                        let out = build.join(file);
                        save_atlas(&gpu, texture, half, &out)?;
                        println!("{}", out.display());
                    }
                }
                return Ok(());
            }
            let (patches, trees) = terrain.patch_counts();
            let placed = terrain
                .tree_draw_arguments()
                .map_or(0, |args| placed_trees(&gpu, args));

            // The land's cost: frames with it and without, serialised, alternating (as the web's `bench`).
            let mut with = Vec::new();
            let mut without = Vec::new();
            // The frames' own GPU busy time where the device takes timestamps (frame number, with or without).
            let mut timed_frames: Vec<(u64, bool)> = Vec::new();
            renderer.gpu_timer_mut().clear_history();
            for pair in 0..pairs {
                for on in [pair % 2 == 0, pair % 2 != 0] {
                    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                    timed_frames.push((renderer.frames_submitted(), on));
                    let start = Instant::now();
                    if shadow_cost || material_cost || part_cost.is_some() {
                        if shadow_cost {
                            terrain.set_tree_shadows(on);
                        } else if let Some(part) = part_cost {
                            terrain.show_tree_part(part, on);
                        } else {
                            terrain.show_ground_materials(on);
                        }
                        let mut layers: [&mut dyn SceneLayer; 4] =
                            [&mut sky, &mut mean, &mut terrain, &mut sea];
                        render(&mut renderer, frames + pair, &mut layers)?;
                    } else if on {
                        let mut layers: [&mut dyn SceneLayer; 4] =
                            [&mut sky, &mut mean, &mut terrain, &mut sea];
                        render(&mut renderer, frames + pair, &mut layers)?;
                    } else if let Some(bare) = bare.as_mut() {
                        let mut layers: [&mut dyn SceneLayer; 4] =
                            [&mut sky, &mut mean, bare, &mut sea];
                        render(&mut renderer, frames + pair, &mut layers)?;
                    } else {
                        let mut layers: [&mut dyn SceneLayer; 3] = [&mut sky, &mut mean, &mut sea];
                        render(&mut renderer, frames + pair, &mut layers)?;
                    }
                    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                    let ms = start.elapsed().as_secs_f64() * 1000.0;
                    if on { with.push(ms) } else { without.push(ms) }
                }
            }
            let median = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 2]
            };
            // GPU busy time (the passes' serial costs summed) replaces the wall clock where it arrived: a loaded
            // machine's CPU swamps the wall clock.
            for _ in 0..4 {
                gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
                renderer.drain_gpu_timings();
            }
            let busy: BTreeMap<u64, f64> = renderer
                .gpu_timer()
                .history()
                .filter_map(|t| {
                    let serial: Option<f64> = t.passes.iter().map(|p| p.serial_ms).sum();
                    serial.map(|ms| (t.frame, ms))
                })
                .collect();
            let gpu_with: Vec<f64> = timed_frames
                .iter()
                .filter(|(_, on)| *on)
                .filter_map(|(f, _)| busy.get(f).copied())
                .collect();
            let gpu_without: Vec<f64> = timed_frames
                .iter()
                .filter(|(_, on)| !*on)
                .filter_map(|(f, _)| busy.get(f).copied())
                .collect();
            // Each pair's own difference (its two frames are adjacent, so a drift in the GPU's clock or another
            // process's load cancels), the median of those.
            let mut differences: Vec<f64> = timed_frames
                .chunks(2)
                .filter_map(|pair| {
                    let (a, b) = (busy.get(&pair[0].0)?, busy.get(&pair[1].0)?);
                    Some(if pair[0].1 { a - b } else { b - a })
                })
                .collect();
            let mut paired = None;
            if !gpu_with.is_empty()
                && gpu_with.len() * 2 >= with.len()
                && gpu_without.len() * 2 >= without.len()
            {
                with = gpu_with;
                without = gpu_without;
                if !differences.is_empty() {
                    differences.sort_by(f64::total_cmp);
                    paired = Some(differences[differences.len() / 2]);
                }
            } else {
                eprintln!("(no GPU timestamps: wall-clock frames)");
            }
            // Each timed pass apart (`--passes`): its tenth percentile with and without, and their difference; then its
            // span with (a pass that overlaps another's work, as a compute pass may, has little time of its own).
            if passes_shown {
                let mut by_pass: BTreeMap<String, [Vec<f64>; 3]> = BTreeMap::new();
                let on_of: BTreeMap<u64, bool> = timed_frames.iter().copied().collect();
                for t in renderer.gpu_timer().history() {
                    let Some(&on) = on_of.get(&t.frame) else {
                        continue;
                    };
                    for p in &t.passes {
                        let entry = by_pass.entry(p.label.clone()).or_default();
                        if let Some(ms) = p.serial_ms {
                            entry[usize::from(!on)].push(ms);
                        }
                        if let (Some(ms), true) = (p.span_ms, on) {
                            entry[2].push(ms);
                        }
                    }
                }
                for (label, [mut a, mut b, mut span]) in by_pass {
                    let tenth = |v: &mut Vec<f64>| {
                        v.sort_by(f64::total_cmp);
                        v.get(v.len() / 10).copied().unwrap_or(0.0)
                    };
                    let (a_low, b_low) = (tenth(&mut a), tenth(&mut b));
                    println!(
                        "  pass {label}: {a_low:.3} with ({}), {b_low:.3} without ({}), {:+.3} ms, span {:.3}",
                        a.len(),
                        b.len(),
                        a_low - b_low,
                        tenth(&mut span)
                    );
                }
            }
            terrain.set_tree_shadows(tree_shadows);
            if let Some(part) = part_cost {
                terrain.show_tree_part(part, true);
            }
            terrain.show_ground_materials(materials_on);
            // Other processes' work on the GPU only ever adds time: the tenth percentile of each side is the frame
            // with the least of it, and their difference the cost (the paired median beside it).
            let low = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                v[v.len() / 10]
            };
            let (low_with, low_without) = (low(&mut with.clone()), low(&mut without.clone()));
            let (with, without) = (median(&mut with), median(&mut without));
            let cost = if paired.is_some() {
                low_with - low_without
            } else {
                with - without
            };
            let (with_key, without_key, cost_key) = if shadow_cost {
                (
                    "frame_with_tree_shadows_ms",
                    "frame_without_tree_shadows_ms",
                    "tree_shadows_ms",
                )
            } else if material_cost {
                (
                    "frame_with_materials_ms",
                    "frame_without_materials_ms",
                    "materials_ms",
                )
            } else if part_cost.is_some() {
                ("frame_with_part_ms", "frame_without_part_ms", "part_ms")
            } else if tree_cost {
                ("frame_with_trees_ms", "frame_without_trees_ms", "trees_ms")
            } else {
                ("frame_with_land_ms", "frame_without_land_ms", "land_ms")
            };
            timings.insert(
                name.clone(),
                serde_json::json!({
                    with_key: with, without_key: without, cost_key: cost, "paired_median_ms": paired,
                    "low_with_ms": low_with, "low_without_ms": low_without,
                    "patches": patches, "tree_patches": trees, "trees": placed, "built_ms": built,
                }),
            );
            let web = web_dir.join(format!("{style_name}-{preset}.png"));
            if web.exists() && debug == TerrainDebug::Off {
                let (mean_diff, far) =
                    side_by_side(&web, &path, &pairs_dir.join(format!("{name}.png")))?;
                compare.insert(
                    name.clone(),
                    serde_json::json!({ "mean_abs_diff": mean_diff, "over_24": far }),
                );
                println!(
                    "{name}: {patches} patches ({trees} with trees), land {:.2} ms, vs web {mean_diff:.1} levels, {:.1}% > 24",
                    with - without,
                    far * 100.0
                );
            } else {
                println!(
                    "{name}: {patches} patches ({trees} with trees, {placed} trees), {} {:.2} ms (frame {with:.2} ms)",
                    if shadow_cost {
                        "tree shadows"
                    } else if material_cost {
                        "materials"
                    } else if part_cost.is_some() {
                        "part"
                    } else if tree_cost {
                        "trees"
                    } else {
                        "land"
                    },
                    cost
                );
            }
        }
    }
    let timings_name = if shadow_cost {
        "tree-shadow-timings.json"
    } else if material_cost {
        "material-timings.json"
    } else if part_cost.is_some() {
        "part-timings.json"
    } else if tree_cost {
        "tree-timings.json"
    } else {
        "timings.json"
    };
    let load = std::process::Command::new("sysctl")
        .args(["-n", "vm.loadavg"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    std::fs::write(
        build.join("compare.json"),
        serde_json::to_string_pretty(&compare)?,
    )?;
    std::fs::write(
        build.join(timings_name),
        serde_json::to_string_pretty(&serde_json::json!({
            "adapter": gpu.adapter.get_info().name, "size": [width, height], "load": load, "views": timings,
        }))?,
    )?;
    Ok(())
}

/// This example's own views for the trees, off the review's main coast: `air300` and `air1500` an aircraft 2 and 3 km
/// out at 300 m and 1,500 m looking 3 and 6 km inland, `ship6` a ship's bridge (25 m) 6 km out looking at the coast,
/// `inland` 500 m over the land 2 km in from the coast, looking along the coast's bearing a little down; `canopy` 150 m
/// over the land 1.5 km in, looking down 600 m ahead, `grove` 60 m over it looking 250 m ahead, and `shore` 30 m up
/// 500 m off the coast, for the trees' shadows; `beach`, a ship's bridge (20 m) 350 m off the coast looking at the
/// shore, and `air50`, `over300` and `over1500`, an aircraft over the shore 50 m, 300 m and 1,500 m up, for the ground
/// materials.
fn foliage_view(field: &Heightfield, offset: [f64; 2], name: &str) -> Option<review::ReviewCamera> {
    let coast = review::main_coast(field);
    let [dx, dz] = coast.direction;
    let world = |d: f64, y: f64| [dx * d + offset[0], y, dz * d + offset[1]];
    let ground = |d: f64| field.height(dx * d, dz * d).max(0.0);
    let look = |eye: [f64; 3], target: [f64; 3]| review::ReviewCamera {
        eye,
        target,
        fov: 52.0,
        far: None,
    };
    let hit = coast.hit;
    // `at:<d>:<h>:<l>[:<turn>]`: an eye `h` m over the ground `d` m inland of the main coast, looking `l` m ahead
    // along the coast's bearing turned by `turn` degrees, at a point half the eye's height over the ground there: the
    // trees and the grass close up.
    if let Some(spec) = name.strip_prefix("at:") {
        let v: Vec<f64> = spec.split(':').filter_map(|x| x.parse().ok()).collect();
        let (d, h, l) = (*v.first()?, *v.get(1)?, *v.get(2)?);
        let turn = v.get(3).copied().unwrap_or(0.0).to_radians();
        let (tx, tz) = (
            dx * turn.cos() - dz * turn.sin(),
            dx * turn.sin() + dz * turn.cos(),
        );
        let (ex, ez) = (dx * (hit + d), dz * (hit + d));
        let (gx, gz) = (ex + tx * l, ez + tz * l);
        let eye_ground = field.height(ex, ez).max(0.0);
        let target_ground = field.height(gx, gz).max(0.0);
        return Some(look(
            [ex + offset[0], eye_ground + h, ez + offset[1]],
            [gx + offset[0], target_ground + h * 0.5, gz + offset[1]],
        ));
    }
    Some(match name {
        "air300" => look(
            world(hit - 2000.0, 300.0),
            world(hit + 3000.0, ground(hit + 3000.0)),
        ),
        "air1500" => look(
            world(hit - 3000.0, 1500.0),
            world(hit + 6000.0, ground(hit + 6000.0)),
        ),
        "ship6" => look(
            world(hit - 6000.0, 25.0),
            world(hit + 1500.0, ground(hit + 1500.0) + 25.0),
        ),
        "inland" => look(
            world(hit + 2000.0, ground(hit + 2000.0) + 500.0),
            world(hit + 7000.0, ground(hit + 7000.0)),
        ),
        "canopy" => look(
            world(hit + 1500.0, ground(hit + 1500.0) + 150.0),
            world(hit + 2100.0, ground(hit + 2100.0)),
        ),
        "grove" => look(
            world(hit + 1500.0, ground(hit + 1500.0) + 60.0),
            world(hit + 1750.0, ground(hit + 1750.0)),
        ),
        "shore" => look(
            world(hit - 500.0, 30.0),
            world(hit + 300.0, ground(hit + 300.0) + 10.0),
        ),
        // The ground close up (native/docs/terrain/ground.md, "Ground materials"): a ship's bridge 350 m off the beach,
        // and an
        // aircraft over the shore 50 m up looking 450 m ahead, 300 m up looking 1 km ahead and 1,500 m up looking 3.5 km
        // ahead.
        "beach" => look(
            world(hit - 350.0, 20.0),
            world(hit + 60.0, ground(hit + 60.0) + 1.0),
        ),
        "air50" => look(
            world(hit - 150.0, ground(hit).max(0.0) + 50.0),
            world(hit + 300.0, ground(hit + 300.0)),
        ),
        "over300" => look(
            world(hit - 200.0, ground(hit).max(0.0) + 300.0),
            world(hit + 800.0, ground(hit + 800.0)),
        ),
        "over1500" => look(
            world(hit - 1000.0, ground(hit).max(0.0) + 1500.0),
            world(hit + 2500.0, ground(hit + 2500.0)),
        ),
        _ => return None,
    })
}

/// The headless device, with timestamps where the adapter takes them (the cost runs read the GPU's own busy time).
fn timed_gpu() -> Result<Gpu> {
    let gpu = Gpu::headless_with(wgpu::Features::TEXTURE_COMPRESSION_BC)?;
    if !gpu
        .adapter
        .features()
        .contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        return Ok(gpu);
    }
    let Gpu {
        instance, adapter, ..
    } = gpu;
    let (device, queue) = pollster::block_on(
        adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stills"),
            required_features: wgpu::Features::TIMESTAMP_QUERY
                | (adapter.features() & wgpu::Features::TEXTURE_COMPRESSION_BC),
            required_limits: wgpu::Limits {
                max_sampled_textures_per_shader_stage: adapter
                    .limits()
                    .max_sampled_textures_per_shader_stage
                    .min(24),
                ..Default::default()
            },
            ..Default::default()
        }),
    )?;
    Ok(Gpu {
        instance,
        adapter,
        device,
        queue,
    })
}

/// Half `half` of the impostor atlas's layers (0 the colours, 1 the normals, depth and occlusion) at its base level,
/// six species a row, the colour over a grey ground.
fn save_atlas(gpu: &Gpu, texture: &wgpu::Texture, half: u32, path: &std::path::Path) -> Result<()> {
    let side = texture.width();
    let models = (texture.depth_or_array_layers() - naval_terrain::impostors::LEAF_LAYERS) / 2;
    let row = side * 4;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("impostor read-back"),
        size: u64::from(row * side * models),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    for k in 0..models {
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: half * models + k,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: u64::from(row * side * k),
                    bytes_per_row: Some(row),
                    rows_per_image: Some(side),
                },
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
    }
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    let bytes = read.slice(..).get_mapped_range()?.to_vec();
    let columns = 6.min(models);
    let rows = models.div_ceil(columns);
    let (width, height) = (columns * side, rows * side);
    let mut image = vec![0u8; (width * height * 4) as usize];
    for k in 0..models {
        let (cx, cy) = (k % columns, k / columns);
        for y in 0..side {
            for x in 0..side {
                let at = ((k * side * side + y * side + x) * 4) as usize;
                let to = (((cy * side + y) * width + cx * side + x) * 4) as usize;
                let p = &bytes[at..at + 4];
                let pixel = if half == 0 {
                    // Premultiplied over grey.
                    let a = f32::from(p[3]) / 255.0;
                    let over = |c: u8| (f32::from(c) + 96.0 * (1.0 - a)).round().min(255.0) as u8;
                    [over(p[0]), over(p[1]), over(p[2]), 255]
                } else {
                    [p[0], p[1], p[2], 255]
                };
                image[to..to + 4].copy_from_slice(&pixel);
            }
        }
    }
    image::save_buffer(path, &image, width, height, image::ColorType::Rgba8)?;
    Ok(())
}

/// The trees the last frame placed: the instance count of the trees' indirect draw, copied out.
fn placed_trees(gpu: &Gpu, args: &wgpu::Buffer) -> u32 {
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tree count read-back"),
        size: 20,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(args, 0, &read, 0, 20);
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the read-back");
    let bytes = read
        .slice(..)
        .get_mapped_range()
        .expect("the mapped count")
        .to_vec();
    let words: &[u32] = bytemuck::cast_slice(&bytes);
    words[1]
}
