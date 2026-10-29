//! The infrastructure layer draws through the renderer core: its shaders translate for every backend, its pipelines
//! compose, a chunk handed in (a terminal's stacks, a marina's boats, a bridge) is uploaded and drawn where its corner
//! and the layer's offset put it, lit, and gone once removed; a power line's towers and wires stand against the sky,
//! and towers afar over the ground are faint and take its colour, never the sky's.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use naga::back::{hlsl, msl, spv};
use naga::valid::{Capabilities, ValidationFlags, Validator};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_conditions::MapSelection;
use naval_render::{
    Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer, ShaderLibrary,
};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::TerrainLight;
use naval_terrain::buildings::tags::{MapTags, Poi, PoiKind, Usage, WaterArea, Zone};
use naval_terrain::infra::airfield::{Apron, Runway, Taxiway};
use naval_terrain::infra::power::{LineKind, PowerLine};
use naval_terrain::infra::{
    Airfield, BridgeLine, ChunkInput, ChunkKey, InfraLayer, InfraSettings, Lod, Marina, Power,
    Under, build_chunk,
};

fn render(
    gpu: &Gpu,
    renderer: &mut Renderer,
    capture: &Capture,
    camera: &Camera,
    layers: &mut [&mut dyn SceneLayer],
) -> Vec<u8> {
    for frame in 0..3 {
        renderer
            .render(
                FrameInput {
                    camera,
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
                    width: capture.width,
                    height: capture.height,
                },
            )
            .unwrap();
    }
    capture.read_rgba(gpu).unwrap()
}

fn changed(a: &[u8], b: &[u8]) -> usize {
    changed_by(a, b, 8)
}

/// Pixels with a channel changed by more than `by`.
fn changed_by(a: &[u8], b: &[u8], by: u8) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > by))
        .count()
}

fn square(x0: f32, z0: f32, x1: f32, z1: f32) -> Vec<[f32; 2]> {
    vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]]
}

#[test]
fn the_infrastructures_shaders_translate_for_metal_directx_and_vulkan() {
    let mut library = ShaderLibrary::new();
    library
        .add(naval_sky::shaders::ENVIRONMENT_FILE)
        .expect("the environment module");
    library
        .add(naval_clouds::shaders::SHADOW_FILE)
        .expect("the clouds' shadow module");
    for file in [
        naval_render::shader_file!("shaders/bindings.wgsl"),
        naval_render::shader_file!("shaders/structures.wgsl"),
        naval_render::shader_file!("shaders/infra.wgsl"),
        naval_render::shader_file!("shaders/infra_caster.wgsl"),
    ] {
        library.add(file).expect("the shaders register");
    }
    for path in ["infra.wgsl", "infra_caster.wgsl"] {
        let module = library
            .compose(
                &format!("naval-terrain/shaders/{path}"),
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{path}: {}", e.message));
        let info = Validator::new(ValidationFlags::all(), Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{path}: naga's validation: {e:?}"));
        let options = msl::Options {
            lang_version: (2, 4),
            ..msl::Options::default()
        };
        msl::write_string(&module, &info, &options, &msl::PipelineOptions::default())
            .unwrap_or_else(|e| panic!("{path}: MSL: {e:?}"));
        let mut text = String::new();
        let options = hlsl::Options {
            shader_model: hlsl::ShaderModel::V6_0,
            ..hlsl::Options::default()
        };
        hlsl::Writer::new(&mut text, &options, &hlsl::PipelineOptions::default())
            .write(&module, &info, None)
            .unwrap_or_else(|e| panic!("{path}: HLSL: {e:?}"));
        spv::write_vec(&module, &info, &spv::Options::default(), None)
            .unwrap_or_else(|e| panic!("{path}: SPIR-V: {e:?}"));
    }
}

#[test]
fn a_chunk_handed_in_is_drawn_where_it_stands() {
    let gpu = Gpu::headless().unwrap();
    let scene = MapSelection::battle("north-atlantic").resolve().unwrap();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut layer = InfraLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        InfraSettings::HIGH,
    )
    .unwrap();
    layer.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A terminal west of x = 700 in the chunk's south half, the sea (15 m deep) east of it with a marina's walkway, and
    // a bridge over the sea from x = 600 to 1000 along z = 300.
    let tags = MapTags {
        zones: vec![Zone::new(
            Usage::Industrial,
            square(-400.0, 300.0, 700.0, 1400.0),
            vec![],
        )],
        water: vec![WaterArea::new(
            square(700.0, -200.0, 1300.0, 1200.0),
            vec![],
        )],
        pois: vec![Poi {
            at: [600.0, 700.0],
            kind: PoiKind::Harbour,
        }],
        ..MapTags::default()
    };
    let ground = |p: [f32; 2]| if p[0] > 700.0 { -15.0 } else { 3.0 };
    let marina = Marina {
        docks: vec![[Vec2::new(760.0, 800.0), Vec2::new(900.0, 800.0)]],
        marinas: vec![Vec2::new(800.0, 760.0)],
    };
    let bridge = BridgeLine {
        points: vec![Vec2::new(400.0, 300.0), Vec2::new(990.0, 300.0)],
        half: 6.0,
        rail: false,
        key: 7,
    };
    let water = |p: Vec2| p.x > 700.0;
    let under = |p: Vec2| if p.x > 700.0 { Under::Sea } else { Under::Dry };
    let key = ChunkKey {
        index: [3, -2],
        lod: Lod::Full,
    };
    let mesh = build_chunk(&ChunkInput {
        key,
        tags: &tags,
        ground: &ground,
        marina: &marina,
        water: &water,
        bridges: std::slice::from_ref(&bridge),
        bridge_outlines: &[],
        under: &under,
        airfield: &Airfield::default(),
        power: &Power::default(),
        period: naval_terrain::buildings::Period::Today,
        bases: &[],
    });
    assert!(mesh.counts.containers > 1000, "{:?}", mesh.counts);
    assert!(mesh.counts.boats > 10, "{:?}", mesh.counts);
    assert_eq!(mesh.counts.bridges, 1);
    layer.set_offset([-2500.0, 2000.0]);
    layer.queue_chunk(key, Arc::new(mesh));
    renderer.context().finish_pipelines().unwrap();

    // From the south-east over the sea, 60 m up, looking west over the marina at the stacks.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(500.0 + 1000.0, 60.0, 980.0),
        target: Vec3::new(500.0 + 600.0, 5.0, 700.0),
        fov_y_degrees: 50.0,
        near: 0.5,
        far: 20000.0,
    };
    let empty = render(&gpu, &mut renderer, &capture, &camera, &mut [&mut sky]);
    let drawn = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/terrain/infra-gpu-test.png");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = image::save_buffer(&path, &drawn, width, height, image::ColorType::Rgba8);
    let status = layer.status();
    assert_eq!(status.chunks, 1);
    assert_eq!(status.drawn_chunks, 1);
    assert_eq!(status.held.bridges, 1);
    let covered = changed(&empty, &drawn);
    assert!(covered > 3000, "the infrastructure covers {covered} pixels");
    let lit = empty
        .chunks(4)
        .zip(drawn.chunks(4))
        .filter(|(a, b)| a[..3] != b[..3])
        .map(|(_, b)| u32::from(b[0]) + u32::from(b[1]) + u32::from(b[2]))
        .sum::<u32>() as f64
        / covered as f64;
    assert!(lit > 60.0, "mean of the infrastructure's pixels {lit}");

    // Made far: the stacks as rows, no boats; still drawn.
    let far = ChunkKey {
        lod: Lod::Box,
        ..key
    };
    let far_mesh = build_chunk(&ChunkInput {
        key: far,
        tags: &tags,
        ground: &ground,
        marina: &marina,
        water: &water,
        bridges: std::slice::from_ref(&bridge),
        bridge_outlines: &[],
        under: &under,
        airfield: &Airfield::default(),
        power: &Power::default(),
        period: naval_terrain::buildings::Period::Today,
        bases: &[],
    });
    assert_eq!(far_mesh.counts.boats, 0);
    layer.queue_chunk(far, Arc::new(far_mesh));
    let rows = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    assert!(changed(&empty, &rows) > 2000);
    assert!(layer.status().triangles < status.triangles);

    // Removed: the view as it was without.
    layer.remove_chunk([3, -2]);
    let gone = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    assert_eq!(changed(&empty, &gone), 0);
}

#[test]
fn a_runway_is_painted_on_the_ground_and_handed_to_its_lights() {
    let gpu = Gpu::headless().unwrap();
    let scene = MapSelection::battle("north-atlantic").resolve().unwrap();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut layer = InfraLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        InfraSettings::HIGH,
    )
    .unwrap();
    layer.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // Runway 09/27, 2 km east from x = 100 along z = 500, a taxiway beside it; flat ground 5 m up.
    let airfield = Airfield {
        runways: vec![Runway {
            points: vec![Vec2::new(100.0, 500.0), Vec2::new(2100.0, 500.0)],
            reference: Some("09/27".into()),
            half: 22.5,
        }],
        taxiways: vec![Taxiway {
            points: vec![Vec2::new(100.0, 640.0), Vec2::new(1000.0, 640.0)],
            reference: Some("A".into()),
        }],
        ..Airfield::default()
    };
    let ground = |_: [f32; 2]| 5.0;
    let key = ChunkKey {
        index: [3, -2],
        lod: Lod::Box,
    };
    let mesh = build_chunk(&ChunkInput {
        key,
        tags: &MapTags::default(),
        ground: &ground,
        marina: &Marina::default(),
        water: &|_| false,
        bridges: &[],
        bridge_outlines: &[],
        under: &|_| Under::Dry,
        airfield: &airfield,
        power: &Power::default(),
        period: naval_terrain::buildings::Period::Today,
        bases: &[],
    });
    assert_eq!(mesh.counts.runways, 1);
    layer.queue_chunk(key, Arc::new(mesh));
    renderer.context().finish_pipelines().unwrap();

    // 40 m up, 100 m short of the threshold, looking east down the runway.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(3000.0, 40.0, -2000.0 + 510.0),
        target: Vec3::new(3000.0 + 220.0, 5.0, -2000.0 + 510.0),
        fov_y_degrees: 50.0,
        near: 0.5,
        far: 20000.0,
    };
    let empty = render(&gpu, &mut renderer, &capture, &camera, &mut [&mut sky]);
    let drawn = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/terrain/infra-runway-test.png");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = image::save_buffer(&path, &drawn, width, height, image::ColorType::Rgba8);
    let covered = changed(&empty, &drawn);
    assert!(covered > 2000, "the runway covers {covered} pixels");
    // The paint stands out of the asphalt: many of the runway's pixels well over its median.
    let mut sums: Vec<u32> = empty
        .chunks(4)
        .zip(drawn.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .map(|(_, b)| u32::from(b[0]) + u32::from(b[1]) + u32::from(b[2]))
        .collect();
    sums.sort_unstable();
    let median = sums[sums.len() / 2];
    let painted = sums.iter().filter(|&&s| s * 10 > median * 14).count();
    assert!(
        painted > 150,
        "{painted} painted pixels over a median of {median}"
    );

    // The runway's ends for its lights, in theatre metres.
    let runways = layer.runways();
    assert_eq!(runways.len(), 1);
    let r = runways[0];
    assert!((r.ends[0][0] - 3100.0).abs() < 0.01 && (r.ends[0][2] + 1500.0).abs() < 0.01);
    assert!((r.ends[1][0] - 5100.0).abs() < 0.01 && (r.ends[0][1] - 5.0).abs() < 0.5);
    assert_eq!(r.half, 22.5);
    assert_eq!(r.designators, [(9, None), (27, None)]);
}

#[test]
fn a_power_line_stands_on_its_towers_its_wires_seen_against_the_sky() {
    let gpu = Gpu::headless().unwrap();
    let scene = MapSelection::battle("north-atlantic").resolve().unwrap();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut layer = InfraLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        InfraSettings::HIGH,
    )
    .unwrap();
    layer.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A transmission line east along z = 500, a tower every 300 m.
    let power = Power {
        lines: vec![PowerLine {
            kind: LineKind::Transmission,
            points: [-100.0, 200.0, 500.0, 800.0, 1100.0]
                .map(|x| Vec2::new(x, 500.0))
                .to_vec(),
        }],
        masts: vec![],
    };
    let ground = |_: [f32; 2]| 0.0;
    let key = ChunkKey {
        index: [3, -2],
        lod: Lod::Full,
    };
    let mesh = build_chunk(&ChunkInput {
        key,
        tags: &MapTags::default(),
        ground: &ground,
        marina: &Marina::default(),
        water: &|_| false,
        bridges: &[],
        bridge_outlines: &[],
        under: &|_| Under::Dry,
        airfield: &Airfield::default(),
        power: &power,
        period: naval_terrain::buildings::Period::Today,
        bases: &[],
    });
    assert_eq!(mesh.counts.towers, 3);
    layer.queue_chunk(key, Arc::new(mesh));
    renderer.context().finish_pipelines().unwrap();

    // 250 m south of the line, looking north up at its wires between the towers at 200 and 500 m.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(3000.0 + 350.0, 8.0, -2000.0 + 750.0),
        target: Vec3::new(3000.0 + 350.0, 26.0, -2000.0 + 500.0),
        fov_y_degrees: 50.0,
        near: 0.5,
        far: 20000.0,
    };
    let empty = render(&gpu, &mut renderer, &capture, &camera, &mut [&mut sky]);
    let drawn = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/terrain/infra-power-test.png");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = image::save_buffer(&path, &drawn, width, height, image::ColorType::Rgba8);
    // The towers at the frame's sides and the wires across its middle, sagging together: a few columns halfway between
    // them cross them.
    let middle = (0..height)
        .filter(|y| {
            (width / 2 - 2..width / 2 + 3).any(|x| {
                let i = ((y * width + x) * 4) as usize;
                empty[i..i + 3]
                    .iter()
                    .zip(&drawn[i..i + 3])
                    .any(|(a, b)| a.abs_diff(*b) > 4)
            })
        })
        .count();
    assert!(
        (2..40).contains(&middle),
        "{middle} of the middle column's pixels changed"
    );
    // The wires a pixel (1.3 m here) wide and faint, as a span a kilometre off is at 720 lines: seen across the frame.
    let covered = changed_by(&empty, &drawn, 2);
    assert!(covered > 1000, "the line covers {covered} pixels");
    // Darker than the sky behind them, never paler.
    let (mut before, mut after) = (0u64, 0u64);
    for (a, b) in empty.chunks(4).zip(drawn.chunks(4)) {
        if a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 2) {
            before += a[..3].iter().map(|&v| u64::from(v)).sum::<u64>();
            after += b[..3].iter().map(|&v| u64::from(v)).sum::<u64>();
        }
    }
    assert!(
        after < before,
        "the line's pixels {after} against the sky's {before}"
    );
}

/// The mean colour (sRGB bytes) of the pixels in `b` changed from `a` by more than `by`, and of the same pixels in `a`,
/// and how many.
fn changed_means(a: &[u8], b: &[u8], by: u8) -> ([f64; 3], [f64; 3], usize) {
    let (mut was, mut now, mut n) = ([0.0; 3], [0.0; 3], 0);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        if p[..3].iter().zip(&q[..3]).any(|(x, y)| x.abs_diff(*y) > by) {
            for c in 0..3 {
                was[c] += f64::from(p[c]);
                now[c] += f64::from(q[c]);
            }
            n += 1;
        }
    }
    let k = n.max(1) as f64;
    (was.map(|v| v / k), now.map(|v| v / k), n)
}

#[test]
fn towers_afar_over_the_ground_are_faint_lattices_never_the_skys_colour() {
    let gpu = Gpu::headless().unwrap();
    let scene = MapSelection::battle("north-atlantic").resolve().unwrap();
    let (width, height) = (640, 360);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut layer = InfraLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        InfraSettings::HIGH,
    )
    .unwrap();
    layer.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A chunk made far, paved over (an apron the chunk's size) with a transmission line across it, east along z = 700,
    // a tower every 200 m; and the same chunk without the line.
    let airfield = Airfield {
        aprons: vec![Apron {
            ring: vec![[0.0, 0.0], [1000.0, 0.0], [1000.0, 1000.0], [0.0, 1000.0]],
            runway: false,
        }],
        ..Airfield::default()
    };
    let power = Power {
        lines: vec![PowerLine {
            kind: LineKind::Transmission,
            points: [100.0, 300.0, 500.0, 700.0, 900.0]
                .map(|x| Vec2::new(x, 700.0))
                .to_vec(),
        }],
        masts: vec![],
    };
    let key = ChunkKey {
        index: [3, -2],
        lod: Lod::Box,
    };
    let mesh = |power: &Power| {
        build_chunk(&ChunkInput {
            key,
            tags: &MapTags::default(),
            ground: &|_| 0.0,
            marina: &Marina::default(),
            water: &|_| false,
            bridges: &[],
            bridge_outlines: &[],
            under: &|_| Under::Dry,
            airfield: &airfield,
            power,
            period: naval_terrain::buildings::Period::Today,
            bases: &[],
        })
    };
    let (paved, lined) = (mesh(&Power::default()), mesh(&power));
    assert_eq!(lined.counts.towers, 5);
    // 1.5 km south of the line and 120 m up, looking a little down at it (as from a ship's bridge or a low aircraft at
    // a town's power lines): the towers stand against the apron behind them, the view under the horizon.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(3500.0, 120.0, -2000.0 + 700.0 + 1500.0),
        target: Vec3::new(3500.0, 0.0, -2000.0 + 700.0),
        fov_y_degrees: 50.0,
        near: 0.5,
        far: 20000.0,
    };
    layer.queue_chunk(key, Arc::new(paved));
    renderer.context().finish_pipelines().unwrap();
    let ground = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    layer.queue_chunk(key, Arc::new(lined));
    let drawn = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/terrain/infra-towers-afar-test.png");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = image::save_buffer(&path, &drawn, width, height, image::ColorType::Rgba8);
    let (was, now, n) = changed_means(&ground, &drawn, 2);
    eprintln!("{n} pixels: the apron {was:?}, the towers over it {now:?}");
    assert!(n > 60, "the towers cover {n} pixels");
    // Steel over the ground, not the sky's colour: no bluer than the concrete behind (within two levels under the North
    // Atlantic's sky), and faint (lines over it, not a solid shape).
    assert!(
        (now[2] - now[0]) - (was[2] - was[0]) < 2.0,
        "bluer than the apron: {now:?} over {was:?}"
    );
    let luma = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    assert!(
        (luma(now) - luma(was)).abs() < 0.15 * luma(was),
        "{now:?} over {was:?}"
    );
}
