//! The buildings layer draws through the renderer core: its pipelines compose (the scene's and the caster's), a chunk
//! handed in is uploaded and drawn where its corner and the layer's offset put it, lit, and gone once removed.

use std::sync::Arc;

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_conditions::MapSelection;
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::TerrainLight;
use naval_terrain::buildings::mesh::ChunkInput;
use naval_terrain::buildings::{
    BuildingSettings, BuildingsLayer, ChunkKey, Footprint, Lod, MapTags, Region, build_chunk,
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
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count()
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
    let mut layer = BuildingsLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        BuildingSettings::HIGH,
    )
    .unwrap();
    layer.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A block, an L and a row of houses in chunk (3, -2), its corner at theatre (3000, -2000); the theatre drawn 2500 m
    // west and 2000 m south, so the chunk's corner stands at the world's (500, 0, 0).
    let square =
        |x: f32, z: f32, w: f32, d: f32| vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]];
    let mut footprints = vec![
        Footprint {
            ring: square(100.0, 100.0, 40.0, 30.0),
            height: Some(24.0),
            kind: None,
            colour: None,
            part: None,
        },
        Footprint {
            ring: vec![
                [160.0, 100.0],
                [200.0, 100.0],
                [200.0, 115.0],
                [175.0, 115.0],
                [175.0, 140.0],
                [160.0, 140.0],
            ],
            height: None,
            kind: None,
            colour: None,
            part: None,
        },
    ];
    for k in 0..6 {
        footprints.push(Footprint {
            ring: square(100.0 + 15.0 * k as f32, 150.0, 9.0, 11.0),
            height: None,
            kind: None,
            colour: None,
            part: None,
        });
    }
    let key = ChunkKey {
        index: [3, -2],
        lod: Lod::Full,
    };
    let mesh = build_chunk(&ChunkInput {
        key,
        footprints: &footprints,
        infill: &[],
        tags: &MapTags::default(),
        ground: &|_| 2.0,
        region: Region::Temperate,
        period: naval_terrain::buildings::Period::Today,
        thinning: None,
    });
    // The block, 30 m deep, stands as its two rows' four buildings (`parcels`); the L and the houses stand whole.
    assert_eq!(mesh.buildings, 11);
    layer.set_offset([-2500.0, 2000.0]);
    layer.queue_chunk(key, Arc::new(mesh));
    assert!(layer.holds(key));
    renderer.context().finish_pipelines().unwrap();

    // From the south-east, 170 m off and 40 m up, looking at the block.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(500.0 + 260.0, 40.0, 240.0),
        target: Vec3::new(500.0 + 140.0, 10.0, 120.0),
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
        .join("../../../.build/native/terrain/buildings-gpu-test.png");
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = image::save_buffer(&path, &drawn, width, height, image::ColorType::Rgba8);
    let counts = layer.counts();
    assert_eq!(counts.chunks, 1);
    assert_eq!(counts.buildings, 11);
    assert_eq!(counts.drawn_chunks, 1);
    let covered = changed(&empty, &drawn);
    assert!(covered > 2000, "the buildings cover {covered} pixels");
    assert!(drawn.chunks(4).all(|p| p[3] == 255));
    // The walls facing the camera and the sun are lit: the buildings' pixels are not black.
    let lit = empty
        .chunks(4)
        .zip(drawn.chunks(4))
        .filter(|(a, b)| a[..3] != b[..3])
        .map(|(_, b)| u32::from(b[0]) + u32::from(b[1]) + u32::from(b[2]))
        .sum::<u32>() as f64
        / covered as f64;
    assert!(lit > 60.0, "mean of the buildings' pixels {lit}");

    // Looking the other way: culled, nothing drawn.
    let away = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        target: Vec3::new(500.0 + 800.0, 60.0, 700.0),
        ..camera
    };
    render(
        &gpu,
        &mut renderer,
        &capture,
        &away,
        &mut [&mut sky, &mut layer],
    );
    assert_eq!(layer.counts().drawn_chunks, 0);
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
fn a_tower_shades_its_neighbours_through_the_sun_maps() {
    use naval_render::{ShadowSettings, Sun};
    let gpu = Gpu::headless().unwrap();
    let scene = MapSelection::battle("north-atlantic").resolve().unwrap();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut layer = BuildingsLayer::new(
        &mut renderer.context(),
        &environment.bind_group,
        BuildingSettings::HIGH,
    )
    .unwrap();
    // A low sun in the west: a 60 m tower throws its shadow east across a long low block.
    let toward_sun = Vec3::new(-1.0, 0.55, 0.15).normalize();
    let mut light = sky.sky.light();
    light.direction = toward_sun;
    layer.set_light(&TerrainLight {
        light,
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    let square =
        |x: f32, z: f32, w: f32, d: f32| vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]];
    let footprints = vec![
        Footprint {
            ring: square(100.0, 100.0, 20.0, 20.0),
            height: Some(60.0),
            kind: None,
            colour: None,
            part: None,
        },
        Footprint {
            ring: square(130.0, 80.0, 90.0, 60.0),
            height: Some(8.0),
            kind: None,
            colour: None,
            part: None,
        },
    ];
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Full,
    };
    let mesh = build_chunk(&ChunkInput {
        key,
        footprints: &footprints,
        infill: &[],
        tags: &MapTags::default(),
        ground: &|_| 2.0,
        region: Region::Temperate,
        period: naval_terrain::buildings::Period::Today,
        thinning: None,
    });
    assert_eq!(mesh.buildings, 2);
    layer.queue_chunk(key, Arc::new(mesh));
    renderer.context().finish_pipelines().unwrap();
    // From above the block's east end, looking down at its roof.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(260.0, 140.0, 110.0),
        target: Vec3::new(170.0, 8.0, 110.0),
        fov_y_degrees: 50.0,
        near: 0.5,
        far: 20000.0,
    };
    renderer.set_sun(Some(Sun::new(toward_sun, Vec3::new(160.0, 0.0, 110.0))));
    renderer.set_shadow_settings(ShadowSettings::for_quality("off").unwrap());
    let unshaded = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
    let shaded = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut layer],
    );
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/terrain/buildings-gpu-shadow.png");
    let _ = image::save_buffer(&path, &shaded, width, height, image::ColorType::Rgba8);
    // The tower's shadow darkens part of the block's roof: many pixels, all darker.
    let mut darker = 0;
    let mut lighter = 0;
    for (a, b) in unshaded.chunks(4).zip(shaded.chunks(4)) {
        let (la, lb) = (
            u32::from(a[0]) + u32::from(a[1]) + u32::from(a[2]),
            u32::from(b[0]) + u32::from(b[1]) + u32::from(b[2]),
        );
        if lb + 24 < la {
            darker += 1;
        } else if la + 24 < lb {
            lighter += 1;
        }
    }
    assert!(darker > 300, "{darker} pixels shaded");
    assert!(lighter < darker / 20, "{lighter} lighter");
}
