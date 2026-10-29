//! Map features on a headless adapter (`TerrainLayer::set_feature_grid`, `queue_feature_page`;
//! native/docs/terrain/surface.md,
//! "Map features on the ground"): a grid with no pages, or a page with nothing on it, draws the land as before (but for
//! a few pixels a level or two off, `assert_same`); a page of roads paints them; clearing the grid restores the land.

use crate::support;
use naval_conditions::LandStyle;
use std::sync::Arc;

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::features::PAGE_TEXELS;
use naval_terrain::{
    FeatureGrid, FeaturePage, Stroke, StrokeKind, TerrainLayer, TerrainLight, TerrainQuality,
    TerrainSettings, review,
};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

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

/// The land as before, but for a few pixels a level or two off. On Apple's GPUs the comparison with no pages failed in 2
/// of 46 runs of the crate's tests, 51 pixels of the 57,600 a level off, 49 of them in the right column of a 2 × 2 quad:
/// the mark of the unsteady derivatives the trees' light had (native/docs/terrain/trees.md, "The light's discard"),
/// here in
/// the land's shader with the features' binding. The trees' cure, a discard, would cost the whole land's shader. So at
/// most 0.2% of the pixels, by at most 2 levels: the roads below move more than 500 pixels by more than 8.
#[track_caller]
fn assert_same(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}");
    let (mut pixels, mut most) = (0, 0);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        let d = p
            .iter()
            .zip(q)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        if d > 0 {
            pixels += 1;
            most = most.max(d);
        }
    }
    assert!(
        pixels * 500 <= a.len() / 4 && most <= 2,
        "{what}: {pixels} pixels differ, by up to {most} levels"
    );
}

#[test]
fn a_page_of_roads_paints_them_and_nothing_else_changes() {
    let gpu = Gpu::headless().unwrap();
    let scene = support::scene(LandStyle::Chalk);
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        // No trees: their placement appends in no fixed order, so two placements of the same view differ by a few
        // overlapping cards (the near tiles' test looks at the cliffs, where few stand).
        TerrainSettings {
            quality: TerrainQuality::MEDIUM,
            ..TerrainSettings::default()
        },
        &environment.bind_group,
    )
    .unwrap();
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    let view = review::preset(&field, scene.offset, "aerial").unwrap();
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(view.eye[0] as f32, view.eye[1] as f32, view.eye[2] as f32),
        target: Vec3::new(
            view.target[0] as f32,
            view.target[1] as f32,
            view.target[2] as f32,
        ),
        fov_y_degrees: view.fov as f32,
        near: 0.5,
        far: 60000.0,
    };
    macro_rules! draw {
        () => {
            render(
                &gpu,
                &mut renderer,
                &capture,
                &camera,
                &mut [&mut sky, &mut terrain],
            )
        };
    }
    let procedural = draw!();
    // One slot a level; level 1's page spans the whole chart (its texel the chart's extent over a page).
    let p = terrain.field();
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let extent = f64::from(p.grid.columns - 1) * p.grid.cell;
    let far_texel = extent / f64::from(PAGE_TEXELS);
    let grid = FeatureGrid {
        origin: [origin, origin],
        texel_m: [2.0, far_texel],
        slots: 1,
    };
    terrain
        .set_feature_grid(&mut renderer.context(), grid)
        .unwrap();
    assert_eq!(terrain.feature_grid(), Some(grid));
    assert_same(&draw!(), &procedural, "a grid with no pages");
    // A page with nothing on it.
    terrain.queue_feature_page(1, [0, 0], Arc::new(FeaturePage::empty()));
    assert!(terrain.holds_feature_page(1, [0, 0]));
    assert_same(&draw!(), &procedural, "an empty page");
    assert_eq!(terrain.feature_page_counts(), (1, 0));
    // Wide paved roads every eight texels both ways across the chart (the page's metres are the chart's).
    let mut strokes = Vec::new();
    let mut k = 0.0;
    while k < extent {
        for points in [
            vec![
                [origin[0] + k, origin[1]],
                [origin[0] + k, origin[1] + extent],
            ],
            vec![
                [origin[0], origin[1] + k],
                [origin[0] + extent, origin[1] + k],
            ],
        ] {
            strokes.push(Stroke {
                kind: StrokeKind::Paved,
                half_width: far_texel as f32,
                tone: 0.8,
                points,
            });
        }
        k += 8.0 * far_texel;
    }
    let page = FeaturePage::rasterize(origin, far_texel, &strokes, &[]);
    assert!(!page.empty);
    terrain.queue_feature_page(1, [0, 0], Arc::new(page));
    let roads = draw!();
    let chroma = |p: &[u8]| {
        let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
        (r - g).abs() + (g - b).abs()
    };
    let changed: Vec<(&[u8], &[u8])> = roads
        .chunks(4)
        .zip(procedural.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .collect();
    assert!(
        changed.len() > 500,
        "the roads show: {} pixels",
        changed.len()
    );
    let greyer = changed
        .iter()
        .filter(|(a, b)| chroma(a) <= chroma(b))
        .count();
    assert!(
        greyer * 10 > changed.len() * 8,
        "asphalt is grey: {greyer} of {}",
        changed.len()
    );
    let path = root().join(".build/native/terrain/features-gpu-test.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::save_buffer(
        &path,
        &roads,
        width,
        height,
        image::ExtendedColorType::Rgba8,
    )
    .unwrap();
    terrain.clear_features(&mut renderer.context());
    assert_eq!(terrain.feature_grid(), None);
    assert_same(&draw!(), &procedural, "no map features");
}

/// On a device allowing only WebGPU's default 16 sampled textures a stage, the land is built without the features'
/// binding (and draws); a grid is refused.
#[test]
fn the_land_draws_on_a_device_with_no_room_for_the_features() {
    let gpu = pollster::block_on(async {
        let instance = native_offscreen::instance();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("sixteen textures"),
                required_limits: wgpu::Limits::default(),
                ..Default::default()
            })
            .await
            .unwrap();
        Gpu {
            instance,
            adapter,
            device,
            queue,
        }
    });
    assert_eq!(
        gpu.device.limits().max_sampled_textures_per_shader_stage,
        16
    );
    let scene = support::scene(LandStyle::Chalk);
    let field = support::land();
    let capture = Capture::new(&gpu, 160, 90);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 160, 90, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings::default(),
        &environment.bind_group,
    )
    .expect("the land without the features' binding");
    let view = review::preset(&field, scene.offset, "aerial").unwrap();
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: Vec3::new(view.eye[0] as f32, view.eye[1] as f32, view.eye[2] as f32),
        target: Vec3::new(
            view.target[0] as f32,
            view.target[1] as f32,
            view.target[2] as f32,
        ),
        fov_y_degrees: view.fov as f32,
        near: 0.5,
        far: 60000.0,
    };
    let pixels = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    assert!(pixels.chunks(4).all(|p| p[3] == 255));
    let grid = FeatureGrid {
        origin: [[0.0, 0.0]; 2],
        texel_m: [2.0, 8.0],
        slots: 1,
    };
    assert!(
        terrain
            .set_feature_grid(&mut renderer.context(), grid)
            .is_err()
    );
}
