//! The land's work kept off the frame (native/docs/terrain/shadows.md, "The land's own shadow";
//! native/docs/terrain/trees.md, "Shadows"): its own shadow computed on a host's worker lands as the render thread's
//! would, and the forest's shade baked again over only the rectangles a map feature page reaches comes out as a whole
//! bake with the page there from the start.

use crate::support;
use naval_conditions::{DEFAULT_MAP, LandStyle, MapScene, MapSelection};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::features::PAGE_TEXELS;
use naval_terrain::forest_shade::SHADE_FRAMES;
use naval_terrain::{
    FeatureGrid, FeaturePage, Heightfield, Stroke, StrokeKind, TerrainLayer, TerrainLight,
    TerrainSettings,
};

struct Bench {
    gpu: Gpu,
    scene: MapScene,
    field: Heightfield,
    capture: Capture,
    renderer: Renderer,
    sky: SkyLayer,
    camera: Camera,
}

impl Bench {
    /// The test land in the tropical style (forest from the shore up) in a low morning sun, 320 × 180, looking over
    /// the scarp from 3 km up.
    fn new() -> Self {
        let gpu = Gpu::headless().unwrap();
        let mut selection = MapSelection::battle(DEFAULT_MAP);
        selection.conditions.sun = Some(naval_conditions::Sun {
            elevation: 15.0,
            azimuth: 100.0,
            morning: true,
        });
        let scene = support::styled(selection.resolve().unwrap(), LandStyle::Tropical);
        let field = support::land();
        let (width, height) = (320, 180);
        let capture = Capture::new(&gpu, width, height);
        let renderer =
            Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
        let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
        sky.sky.apply(&scene.sky.cloudless());
        let camera = Camera {
            curvature: naval_render::camera::EARTH_CURVATURE,
            eye: support::over_the_scarp(&scene, 3000.0),
            target: support::world(&scene, support::SCARP, 0.0),
            fov_y_degrees: 52.0,
            near: 0.5,
            far: 60000.0,
        };
        Self {
            gpu,
            scene,
            field,
            capture,
            renderer,
            sky,
            camera,
        }
    }

    fn land(&mut self) -> TerrainLayer {
        let mut terrain = TerrainLayer::new(
            &mut self.renderer.context(),
            &self.field,
            &self.scene,
            TerrainSettings::default(),
            &self.sky.sky.environment().bind_group,
        )
        .unwrap();
        self.light(&mut terrain, self.sky.sky.light().direction);
        terrain
    }

    fn light(&self, terrain: &mut TerrainLayer, direction: Vec3) {
        let mut light = self.sky.sky.light();
        light.direction = direction;
        terrain.set_light(&TerrainLight {
            light,
            ambient: self.scene.ambient as f32,
            flash: 0.0,
        });
    }

    fn frames(&mut self, terrain: &mut TerrainLayer, frames: u64) {
        for frame in 0..frames {
            let layers: &mut [&mut dyn SceneLayer] = &mut [&mut self.sky, &mut *terrain];
            self.renderer
                .render(
                    FrameInput {
                        camera: &self.camera,
                        time: FrameTime {
                            seconds: 0.0,
                            dt: 0.0,
                            frame,
                        },
                    },
                    layers,
                    RenderOutput {
                        view: &self.capture.view,
                        format: CAPTURE_FORMAT,
                        width: self.capture.width,
                        height: self.capture.height,
                    },
                )
                .unwrap();
        }
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
    }

    /// A texture's texels, `texel` bytes each, rows packed.
    fn read(&self, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
        let (width, height) = (texture.width(), texture.height());
        let row = (width * texel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("texture read-back"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.gpu.queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        self.gpu
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
        bytes
            .chunks(row as usize)
            .flat_map(|r| r[..(width * texel) as usize].to_vec())
            .collect()
    }
}

#[test]
fn the_land_shadow_moves_on_the_worker_and_lands_as_the_render_threads() {
    let mut bench = Bench::new();
    let mut terrain = bench.land();
    let spawned = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = spawned.clone();
    terrain.set_worker(Some(Arc::new(move |job: Box<dyn FnOnce() + Send>| {
        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::thread::spawn(job);
    })));
    // Made without a shadow: the first is computed in the frame, not on the worker.
    bench.frames(&mut terrain, 2);
    assert!(!terrain.sun_pending());
    assert_eq!(spawned.load(std::sync::atomic::Ordering::Relaxed), 0);
    let first = bench.read(terrain.sunlight_texture(), 1);

    // The sun 20 degrees on: the worker takes it, and the old shadow is drawn until it lands.
    let light = bench.sky.sky.light().direction;
    let moved = glam::Quat::from_rotation_y(20f32.to_radians()) * light;
    bench.light(&mut terrain, moved);
    bench.frames(&mut terrain, 1);
    assert!(terrain.sun_pending());
    assert_eq!(spawned.load(std::sync::atomic::Ordering::Relaxed), 1);
    let started = Instant::now();
    while terrain.sun_pending() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the worker's shadow never landed"
        );
        std::thread::sleep(Duration::from_millis(5));
        bench.frames(&mut terrain, 1);
    }
    let drawn = terrain.sun_light().unwrap();
    assert!(drawn.angle_between(moved.normalize()) < 1e-6);
    let mut expected = Vec::new();
    terrain.field().sunlight(
        [f64::from(drawn.x), f64::from(drawn.y), f64::from(drawn.z)],
        &mut expected,
    );
    let landed = bench.read(terrain.sunlight_texture(), 1);
    assert_eq!(landed, expected, "the worker's shadow is the sweep's");
    assert_ne!(landed, first, "the shadow moved with the sun");

    // Background work off (a manual clock): the render thread computes it in the frame, as before.
    terrain.set_background_work(false);
    bench.light(&mut terrain, light);
    bench.frames(&mut terrain, 1);
    assert!(!terrain.sun_pending());
    assert_eq!(spawned.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert_eq!(bench.read(terrain.sunlight_texture(), 1), first);

    // A job still out when background work goes off: let go, and the frame computes the light itself; the job running
    // late changes nothing.
    type Job = Box<dyn FnOnce() + Send>;
    let held: Arc<std::sync::Mutex<Vec<Job>>> = Arc::default();
    let hold = held.clone();
    terrain.set_worker(Some(Arc::new(move |job: Job| {
        hold.lock().unwrap().push(job);
    })));
    terrain.set_background_work(true);
    bench.light(&mut terrain, moved);
    bench.frames(&mut terrain, 1);
    assert!(terrain.sun_pending());
    terrain.set_background_work(false);
    bench.frames(&mut terrain, 1);
    assert!(!terrain.sun_pending());
    assert_eq!(bench.read(terrain.sunlight_texture(), 1), landed);
    for job in held.lock().unwrap().drain(..) {
        job();
    }
    bench.light(&mut terrain, moved);
    bench.frames(&mut terrain, 2);
    assert_eq!(bench.read(terrain.sunlight_texture(), 1), landed);
}

/// Paved roads every 48 m both ways over chart square `corner` + `side` (the trees keep off them).
fn roads(corner: [f64; 2], side: f64) -> Vec<Stroke> {
    let mut strokes = Vec::new();
    let mut k = 0.0;
    while k < side {
        for points in [
            vec![
                [corner[0] + k, corner[1]],
                [corner[0] + k, corner[1] + side],
            ],
            vec![
                [corner[0], corner[1] + k],
                [corner[0] + side, corner[1] + k],
            ],
        ] {
            strokes.push(Stroke {
                kind: StrokeKind::Paved,
                half_width: 6.0,
                tone: 0.5,
                points,
            });
        }
        k += 48.0;
    }
    strokes
}

#[test]
fn a_pages_forest_shade_is_baked_over_its_reach_as_a_whole_bake_would() {
    let mut bench = Bench::new();
    // The deepest forest, from a whole bake with no features.
    let mut plain = bench.land();
    bench.frames(&mut plain, 2);
    let open_sky = plain.open_sky_texture();
    let (columns, rows) = (open_sky.width(), open_sky.height());
    let texels = bench.read(open_sky, 4);
    let deepest = (0..columns * rows)
        .max_by_key(|k| texels[*k as usize * 4 + 1])
        .unwrap();
    assert!(texels[deepest as usize * 4 + 1] > 128, "a forest to clear");
    let p = plain.field().clone();
    let at = [
        f64::from(p.origin[0]) + f64::from(deepest % columns) * p.coarse.cell,
        f64::from(p.origin[1]) + f64::from(deepest / columns) * p.coarse.cell,
    ];
    drop(plain);
    // Level 0 on 4 m texels (1 km pages), level 1 on 16 m texels (4 km pages), from the chart's corner.
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let grid = FeatureGrid {
        origin: [origin, origin],
        texel_m: [4.0, 16.0],
        slots: 4,
    };
    let page = |level: usize, at: [f64; 2]| {
        let side = grid.page_m(level);
        let index = [
            ((at[0] - origin[0]) / side).floor() as i32,
            ((at[1] - origin[1]) / side).floor() as i32,
        ];
        let corner = [
            origin[0] + f64::from(index[0]) * side,
            origin[1] + f64::from(index[1]) * side,
        ];
        let made = FeaturePage::rasterize(corner, grid.texel_m[level], &roads(corner, side), &[]);
        (level, index, Arc::new(made))
    };
    let near = [page(0, at), page(0, [at[0] + grid.page_m(0), at[1]])];
    let far = page(1, at);
    assert_eq!(PAGE_TEXELS, 256);

    // Pages arriving after the first frames: the shade waits for rectangles about them, then bakes those alone.
    let mut late = bench.land();
    late.set_feature_grid(&mut bench.renderer.context(), grid)
        .unwrap();
    bench.frames(&mut late, 2);
    assert!(late.forest_shade_waiting().is_empty());
    for (level, index, made) in &near {
        late.queue_feature_page(*level, *index, made.clone());
    }
    bench.frames(&mut late, 1);
    let waiting: u64 = late
        .forest_shade_waiting()
        .iter()
        .map(|r| u64::from(r[2] - r[0]) * u64::from(r[3] - r[1]))
        .sum();
    assert!(waiting > 0, "the pages' reach waits for a bake");
    assert!(
        waiting * 50 < u64::from(columns * rows),
        "two level-0 pages reach {waiting} of {} texels",
        columns * rows
    );
    bench.frames(&mut late, u64::from(SHADE_FRAMES) + 1);
    assert!(late.forest_shade_waiting().is_empty());

    // The same pages there before the first frame: one whole bake.
    let mut early = bench.land();
    early
        .set_feature_grid(&mut bench.renderer.context(), grid)
        .unwrap();
    for (level, index, made) in &near {
        early.queue_feature_page(*level, *index, made.clone());
    }
    early.flush_feature_pages(&mut bench.renderer.context());
    bench.frames(&mut early, 2);
    let late_sky = bench.read(late.open_sky_texture(), 4);
    let early_sky = bench.read(early.open_sky_texture(), 4);
    assert_ne!(early_sky, texels, "the roads clear trees");
    assert_eq!(
        late_sky, early_sky,
        "level 0's pages baked over their reach"
    );

    // A level-1 page, which feathers into its neighbours: the same again.
    late.queue_feature_page(far.0, far.1, far.2.clone());
    bench.frames(&mut late, u64::from(SHADE_FRAMES) + 2);
    let mut whole = bench.land();
    whole
        .set_feature_grid(&mut bench.renderer.context(), grid)
        .unwrap();
    for (level, index, made) in near.iter().chain([&far]) {
        whole.queue_feature_page(*level, *index, made.clone());
    }
    whole.flush_feature_pages(&mut bench.renderer.context());
    bench.frames(&mut whole, 2);
    assert_eq!(
        bench.read(late.open_sky_texture(), 4),
        bench.read(whole.open_sky_texture(), 4),
        "level 1's page baked over its reach"
    );
}
