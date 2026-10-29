//! The land draws through the renderer core: pipelines compose, the land covers the view it should with
//! the web's patches and trees, and nothing comes out non-finite. Plus the cloud-shadow module's copy against the
//! clouds' original.

use crate::support;
use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_conditions::{DEFAULT_MAP, LandStyle, MapSelection};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::forest_shade::SHADE_FRAMES;
use naval_terrain::{TerrainLayer, TerrainLight, TerrainQuality, TerrainSettings, review};

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
    render_at(gpu, renderer, capture, camera, layers, 0.0)
}

/// [`render`] at `seconds` on the scene's clock.
fn render_at(
    gpu: &Gpu,
    renderer: &mut Renderer,
    capture: &Capture,
    camera: &Camera,
    layers: &mut [&mut dyn SceneLayer],
    seconds: f64,
) -> Vec<u8> {
    // A run of frames, again until two runs draw the same picture: the first bakes the trees' impostors, and the land
    // takes a few frames more to settle bit for bit since (three sufficed before the impostors). Each run outlasts the
    // forest shade's wait between bakes (`SHADE_FRAMES`), as tests/near.rs's do. The trees' light, depth first since
    // 2026-10-26, changed in single pixels from frame to frame until 2026-10-27 (up to 12 levels, mostly the right of a
    // 2×2 quad, every input the same, more often with other tests on the GPU): drawn under the depth test `Equal` by a
    // shader that could not discard, it took unsteady derivatives on Apple's GPUs (`trees.wgsl`, `fs_tree`), and this
    // file's and tests/near.rs's exact comparisons failed now and then.
    let mut last: Option<Vec<u8>> = None;
    for _ in 0..8 {
        for frame in 0..=u64::from(SHADE_FRAMES) {
            renderer
                .render(
                    FrameInput {
                        camera,
                        time: FrameTime {
                            seconds,
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
        let picture = capture.read_rgba(gpu).unwrap();
        if last.as_ref() == Some(&picture) {
            return picture;
        }
        last = Some(picture);
    }
    last.unwrap()
}

/// The same picture, but for the few pixels where two trees' fragments meet at one depth: the placement appends the
/// trees in whatever order its threads reach the counter, and which of two equal fragments lands last can move a channel
/// a level or two (at most 0.05% of the bytes, by at most 4 levels).
#[track_caller]
fn assert_same(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len(), "{what}");
    let differ: Vec<i32> = a
        .iter()
        .zip(b)
        .map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs())
        .filter(|d| *d > 0)
        .collect();
    let most = differ.iter().copied().max().unwrap_or(0);
    assert!(
        differ.len() * 2000 <= a.len() && most <= 4,
        "{what}: {} bytes differ, by up to {most}",
        differ.len()
    );
}

#[test]
fn a_map_draws_its_land_and_trees() {
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
        TerrainSettings::default(),
        &environment.bind_group,
    )
    .unwrap();
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    let view = review::preset(&field, scene.offset, "cliffs").unwrap();
    let camera = Camera {
        eye: Vec3::new(view.eye[0] as f32, view.eye[1] as f32, view.eye[2] as f32),
        target: Vec3::new(
            view.target[0] as f32,
            view.target[1] as f32,
            view.target[2] as f32,
        ),
        fov_y_degrees: view.fov as f32,
        near: 0.5,
        far: 60000.0,
        curvature: naval_render::camera::EARTH_CURVATURE,
    };
    let with_land = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let (patches, trees) = terrain.patch_counts();
    assert!(patches > 100, "{patches} patches");
    assert!(trees > 0, "no trees");
    // The placement stood some of the cliffs' woods, within the instance buffer.
    let placed = placed_trees(&gpu, terrain.tree_draw_arguments().expect("trees on High"));
    assert!(
        placed > 100 && placed < naval_terrain::trees::TREE_CAPACITY,
        "{placed} trees placed"
    );
    let without = render(&gpu, &mut renderer, &capture, &camera, &mut [&mut sky]);
    // The cliffs stand across this view, the sea floor before them: the land changes a tenth of the pixels and more.
    let changed = with_land
        .chunks(4)
        .zip(without.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    assert!(
        changed > (width * height / 10) as usize,
        "the land covers {changed} pixels"
    );
    assert!(with_land.chunks(4).all(|p| p[3] == 255));
}

/// The trees the last frame placed: the instance count of the trees' indirect draw (its second word), copied out.
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
        .unwrap();
    let bytes = read.slice(..).get_mapped_range().unwrap().to_vec();
    let words: &[u32] = bytemuck::cast_slice(&bytes);
    assert_eq!(words[0], 6, "the card's six indices");
    words[1]
}

/// An `Rgba8Unorm` texture copied out: its bytes and their row pitch.
fn read_rgba8(gpu: &Gpu, texture: &wgpu::Texture) -> (Vec<u8>, u32) {
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("texture read-back"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
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
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();
    (bytes, row)
}

#[test]
fn the_trees_cast_into_the_sun_maps_and_shade_the_forest() {
    use naval_render::{ShadowSettings, Sun};
    let gpu = Gpu::headless().unwrap();
    let mut selection = MapSelection::battle(DEFAULT_MAP);
    // A low morning sun: long shadows.
    selection.conditions.sun = Some(naval_conditions::Sun {
        elevation: 15.0,
        azimuth: 100.0,
        morning: true,
    });
    let scene = support::styled(selection.resolve().unwrap(), LandStyle::Tropical);
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings::default(),
        &sky.sky.environment().bind_group,
    )
    .unwrap();
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A first frame bakes the forest's shade into the open sky's green and blue: the island is forest.
    let overview = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: support::over_the_scarp(&scene, 3000.0),
        target: support::world(&scene, support::SCARP, 0.0),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    render(
        &gpu,
        &mut renderer,
        &capture,
        &overview,
        &mut [&mut sky, &mut terrain],
    );
    let (bytes, row) = read_rgba8(&gpu, terrain.open_sky_texture());
    let texture = terrain.open_sky_texture();
    let (mut forest, mut tall, mut deepest) = (0, 0, (0u8, 0, 0));
    for z in 0..texture.height() {
        for x in 0..texture.width() {
            let at = (z * row + x * 4) as usize;
            if bytes[at + 1] > 128 {
                forest += 1;
                // Rainforest 20-30 m tall over the 50 m scale.
                if (80..=160).contains(&bytes[at + 2]) {
                    tall += 1;
                }
            }
            if bytes[at + 1] > deepest.0 && support::on_the_downs(terrain.field(), x, z) {
                deepest = (bytes[at + 1], x, z);
            }
        }
    }
    assert!(forest > 1000, "{forest} forest texels");
    assert!(
        tall * 10 > forest * 9,
        "{tall} of {forest} at a forest's height"
    );

    // Over the deepest forest on the downs behind the scarp (the land falling away west of it), 250 m up and 500 m off,
    // the sun's maps about it.
    let prepared = terrain.field();
    let chart = [
        f64::from(prepared.origin[0]) + f64::from(deepest.1) * prepared.coarse.cell,
        f64::from(prepared.origin[1]) + f64::from(deepest.2) * prepared.coarse.cell,
    ];
    let ground = field.height(chart[0], chart[1]) as f32;
    let spot = Vec3::new(
        (chart[0] + scene.offset[0]) as f32,
        ground,
        (chart[1] + scene.offset[1]) as f32,
    );
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: spot + Vec3::new(-500.0, 250.0, 0.0),
        target: spot,
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    renderer.set_sun(Some(Sun::new(sky.sky.light().direction, camera.target)));
    let cast = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    assert_eq!(
        terrain.tree_shadow_maps(),
        [true, true, false],
        "into the near and wide maps (the aboard map is off)"
    );
    // Off, the forest's shade stands in everywhere and the trees cast nothing: the view differs where their shadows
    // fell.
    terrain.set_tree_shadows(false);
    let shaded = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    assert_eq!(terrain.tree_shadow_maps(), [false; 3]);
    let changed = cast
        .chunks(4)
        .zip(shaded.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    assert!(changed > 100, "the trees' shadows change {changed} pixels");
    assert!(cast.chunks(4).all(|p| p[3] == 255));

    // Turned away from the forest, down the scarp to the west, with the maps still on it: the trees there stand only to
    // cast.
    let away = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        target: camera.eye + Vec3::new(-500.0, -150.0, 0.0),
        ..camera
    };
    let in_view = {
        render(
            &gpu,
            &mut renderer,
            &capture,
            &away,
            &mut [&mut sky, &mut terrain],
        );
        placed_trees(&gpu, terrain.tree_draw_arguments().unwrap())
    };
    terrain.set_tree_shadows(true);
    render(
        &gpu,
        &mut renderer,
        &capture,
        &away,
        &mut [&mut sky, &mut terrain],
    );
    let casting = placed_trees(&gpu, terrain.tree_draw_arguments().unwrap());
    assert!(
        casting > in_view + 500,
        "trees out of view stand to cast: {casting} against {in_view} in view"
    );

    // With the sun's maps off nothing casts.
    terrain.set_tree_shadows(true);
    renderer.set_shadow_settings(ShadowSettings::for_quality("off").unwrap());
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    assert_eq!(terrain.tree_shadow_maps(), [false; 3]);
}

/// Copies `size` bytes of `buffer` out.
fn read_buffer(gpu: &Gpu, buffer: &wgpu::Buffer, size: u64) -> Vec<u8> {
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("read-back"),
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &read, 0, size);
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    read.slice(..).get_mapped_range().unwrap().to_vec()
}

/// The species of the trees `terrain` placed in its last frame, with how many of each.
fn placed_species(gpu: &Gpu, terrain: &TerrainLayer) -> std::collections::BTreeMap<u32, usize> {
    let count = u64::from(placed_trees(gpu, terrain.tree_draw_arguments().unwrap()));
    let bytes = read_buffer(
        gpu,
        terrain.tree_instances().unwrap(),
        count * naval_terrain::trees::TREE_INSTANCE_BYTES,
    );
    let floats: &[f32] = bytemuck::cast_slice(&bytes);
    let mut species = std::collections::BTreeMap::new();
    for tree in floats.chunks(12) {
        *species.entry(tree[5] as u32).or_insert(0) += 1;
    }
    species
}

/// The share of each frame of each species' colour layer that the bake covered, at the base level.
fn impostor_coverage(gpu: &Gpu, atlas: &wgpu::Texture) -> Vec<Vec<f32>> {
    use naval_terrain::impostors::{ATLAS_TEXELS, FRAME_TEXELS, IMPOSTOR_FRAMES};
    let models = (atlas.depth_or_array_layers() - naval_terrain::impostors::LEAF_LAYERS) / 2;
    let row = ATLAS_TEXELS * 4;
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("impostor read-back"),
        size: u64::from(row * ATLAS_TEXELS * models),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: atlas,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &read,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(ATLAS_TEXELS),
            },
        },
        wgpu::Extent3d {
            width: ATLAS_TEXELS,
            height: ATLAS_TEXELS,
            depth_or_array_layers: models,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let texels = read.slice(..).get_mapped_range().unwrap().to_vec();
    let area = (FRAME_TEXELS * FRAME_TEXELS) as f32;
    (0..models)
        .map(|model| {
            (0..IMPOSTOR_FRAMES * IMPOSTOR_FRAMES)
                .map(|frame| {
                    let (fx, fy) = (frame % IMPOSTOR_FRAMES, frame / IMPOSTOR_FRAMES);
                    let mut covered = 0;
                    for y in 0..FRAME_TEXELS {
                        for x in 0..FRAME_TEXELS {
                            let (ax, ay) = (fx * FRAME_TEXELS + x, fy * FRAME_TEXELS + y);
                            let at = ((model * ATLAS_TEXELS + ay) * ATLAS_TEXELS + ax) * 4 + 3;
                            if texels[at as usize] > 128 {
                                covered += 1;
                            }
                        }
                    }
                    covered as f32 / area
                })
                .collect()
        })
        .collect()
}

/// The share of each frame of each species' colour layer at or past the impostors' edge (`trees.wgsl`'s `EDGE`), at
/// every level of the atlas (per species, per level, per frame), and the shares plain 2 x 2 averages of the base level
/// would give (the levels before they kept their coverage).
#[allow(clippy::type_complexity)]
fn impostor_coverage_levels(
    gpu: &Gpu,
    atlas: &wgpu::Texture,
) -> (Vec<Vec<Vec<f32>>>, Vec<Vec<Vec<f32>>>) {
    use naval_terrain::impostors::{ATLAS_LEVELS, ATLAS_TEXELS, IMPOSTOR_FRAMES};
    let models = (atlas.depth_or_array_layers() - naval_terrain::impostors::LEAF_LAYERS) / 2;
    let edge = (0.45f32 * 255.0).ceil() as u8;
    let mut out = vec![Vec::new(); models as usize];
    let mut base: Vec<Vec<f32>> = Vec::new();
    for level in 0..ATLAS_LEVELS {
        let side = ATLAS_TEXELS >> level;
        let row = (side * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("impostor level read-back"),
            size: u64::from(row * side * models),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: atlas,
                mip_level: level,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(side),
                },
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: models,
            },
        );
        gpu.queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let texels = read.slice(..).get_mapped_range().unwrap().to_vec();
        if level == 0 {
            base = (0..models)
                .map(|model| {
                    (0..side * side)
                        .map(|i| {
                            let at = (model * side + i / side) * row + (i % side) * 4 + 3;
                            f32::from(texels[at as usize]) / 255.0
                        })
                        .collect()
                })
                .collect();
        }
        let n = side / IMPOSTOR_FRAMES;
        for (model, levels) in out.iter_mut().enumerate() {
            let shares = (0..IMPOSTOR_FRAMES * IMPOSTOR_FRAMES)
                .map(|frame| {
                    let (fx, fy) = (frame % IMPOSTOR_FRAMES, frame / IMPOSTOR_FRAMES);
                    let mut covered = 0;
                    for y in 0..n {
                        for x in 0..n {
                            let at =
                                (model as u32 * side + fy * n + y) * row + (fx * n + x) * 4 + 3;
                            if texels[at as usize] >= edge {
                                covered += 1;
                            }
                        }
                    }
                    covered as f32 / (n * n) as f32
                })
                .collect::<Vec<f32>>();
            levels.push(shares);
        }
    }
    let mut plain = vec![Vec::new(); models as usize];
    for (model, alpha) in base.into_iter().enumerate() {
        let mut level = alpha;
        let mut side = ATLAS_TEXELS;
        for k in 0..ATLAS_LEVELS {
            if k > 0 {
                let next = side / 2;
                level = (0..next * next)
                    .map(|i| {
                        let (x, y) = (i % next * 2, i / next * 2);
                        let at = |dx: u32, dy: u32| level[((y + dy) * side + x + dx) as usize];
                        (at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) * 0.25
                    })
                    .collect();
                side = next;
            }
            let n = side / IMPOSTOR_FRAMES;
            let shares = (0..IMPOSTOR_FRAMES * IMPOSTOR_FRAMES)
                .map(|frame| {
                    let (fx, fy) = (frame % IMPOSTOR_FRAMES * n, frame / IMPOSTOR_FRAMES * n);
                    let covered = (0..n * n)
                        .filter(|i| level[((fy + i / n) * side + fx + i % n) as usize] >= 0.45)
                        .count();
                    covered as f32 / (n * n) as f32
                })
                .collect::<Vec<f32>>();
            plain[model].push(shares);
        }
    }
    (out, plain)
}

/// Every level of the impostor atlas keeps its frames' coverage at the edge the impostors' alpha test cuts at
/// (Castaño's alpha mipmaps): the share of a frame's texels at or past it stays the base level's, so a far crown
/// neither thins into speckle nor swells.
#[test]
fn the_impostor_levels_keep_their_coverage() {
    use naval_terrain::impostors::{ATLAS_LEVELS, FRAME_TEXELS};
    let gpu = Gpu::headless().unwrap();
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    let scene = support::scene(LandStyle::Chalk);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut terrain = TerrainLayer::for_scene_sharing(
        &mut renderer.context(),
        naval_terrain::PreparedLand::new(&field),
        &scene,
        TerrainSettings::default(),
        &environment.bind_group,
        Some(&naval_terrain::TreeAtlas::uncompressed(&gpu.device)),
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
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let (levels, plain) = impostor_coverage_levels(&gpu, terrain.tree_impostors().unwrap());
    let off = |chain: &Vec<Vec<Vec<f32>>>, level: usize| {
        let mut sum = 0.0f32;
        let mut drift = 0.0f32;
        let mut count = 0;
        for shares in chain {
            for (frame, share) in shares[level].iter().enumerate() {
                sum += (share - shares[0][frame]).abs();
                drift += share - shares[0][frame];
                count += 1;
            }
        }
        (sum / count as f32, drift / count as f32)
    };
    for level in 1..ATLAS_LEVELS as usize {
        let (was, was_drift) = off(&plain, level);
        eprintln!(
            "level {level}: plain averages would be off the base by {was:.4} on average (drift {was_drift:+.4})"
        );
        let mut sum = 0.0f32;
        let mut worst = (0.0f32, 0usize, 0usize);
        let mut count = 0;
        let mut drift = 0.0f32;
        for (model, shares) in levels.iter().enumerate() {
            for (frame, share) in shares[level].iter().enumerate() {
                let d = share - shares[0][frame];
                sum += d.abs();
                drift += d;
                count += 1;
                if d.abs() > worst.0 {
                    worst = (d.abs(), model, frame);
                }
            }
        }
        let mean = sum / count as f32;
        // A frame's texels a side at this level: its share moves in steps of one over their number.
        let step = 1.0 / ((FRAME_TEXELS >> level) as f32).powi(2);
        eprintln!(
            "level {level}: coverage off the base by {mean:.4} on average (drift {:+.4}), at worst {:.4} \
             ({:?} frame {}), a texel {step:.4}",
            drift / count as f32,
            worst.0,
            naval_terrain::species::Species::ALL[worst.1],
            worst.2
        );
        assert!(mean < 0.01 + step, "level {level}: {mean} off on average");
        assert!(
            worst.0 < 0.04 + 2.0 * step,
            "level {level}: {} off at worst",
            worst.0
        );
    }
}

/// The impostors bake every species with coverage in every frame, and the placement chooses a styled land's trees
/// from its style but a theatre's from where it stands: the chalk test land drawn as California's grows coast
/// live oak and the coast's pines, not the chalk style's oak; as Malibu's, no redwoods; as Japan's, laurel and sugi; as
/// the Solomons', rainforest.
#[test]
fn the_impostors_bake_and_the_biome_chooses_the_species() {
    use naval_terrain::species::Species;
    let gpu = Gpu::headless().unwrap();
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    let mut species_of = |climate: Option<naval_conditions::Climate>| {
        let mut scene = support::scene(LandStyle::Chalk);
        if let Some(climate) = climate {
            scene.style.climate = climate;
        }
        sky.sky.apply(&scene.sky.cloudless());
        let environment = sky.sky.environment();
        // The atlas kept as baked, so its coverage can be read back.
        let mut terrain = TerrainLayer::for_scene_sharing(
            &mut renderer.context(),
            naval_terrain::PreparedLand::new(&field),
            &scene,
            TerrainSettings::default(),
            &environment.bind_group,
            Some(&naval_terrain::TreeAtlas::uncompressed(&gpu.device)),
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
        render(
            &gpu,
            &mut renderer,
            &capture,
            &camera,
            &mut [&mut sky, &mut terrain],
        );
        let species = placed_species(&gpu, &terrain);
        (species, terrain)
    };
    let index = |s: Species| Species::ALL.iter().position(|x| *x == s).unwrap() as u32;
    let count = |of: &std::collections::BTreeMap<u32, usize>, s: Species| {
        of.get(&index(s)).copied().unwrap_or(0)
    };
    let (authored, terrain) = species_of(None);
    // Kent in February: bare oak, the pines, shrubs about the woods.
    let allowed = [
        Species::Oak,
        Species::Ash,
        Species::BareOak,
        Species::ScotsPine,
        Species::Shrub,
    ]
    .map(index);
    assert!(authored.keys().all(|k| allowed.contains(k)), "{authored:?}");
    assert!(count(&authored, Species::BareOak) > 100, "{authored:?}");

    // Every species' colour layer holds a tree in every frame, never filling it.
    let coverage = impostor_coverage(&gpu, terrain.tree_impostors().unwrap());
    assert_eq!(coverage.len(), Species::ALL.len());
    for (model, frames) in coverage.iter().enumerate() {
        for (frame, share) in frames.iter().enumerate() {
            assert!(
                *share > 0.02 && *share < 0.9,
                "{:?} frame {frame}: {share} covered",
                Species::ALL[model]
            );
        }
    }

    // The same land as California's in summer.
    let (california, _) = species_of(Some(naval_conditions::Climate {
        snowline_m: 3000.0,
        thaw: 1.0,
        leaf: 1.0,
        dry: 1.0,
        place: Some([36.94, -122.42]),
    }));
    assert!(count(&california, Species::LiveOak) > 100, "{california:?}");
    let coast = [
        Species::MontereyPine,
        Species::MontereyCypress,
        Species::Eucalyptus,
        Species::Redwood,
    ];
    assert!(
        coast.iter().any(|s| count(&california, *s) > 0),
        "{california:?}"
    );
    assert_eq!(count(&california, Species::Oak), 0, "{california:?}");
    assert_eq!(count(&california, Species::BareOak), 0, "{california:?}");

    // Southern California (Malibu): no redwoods south of Big Sur.
    let summer = |place: [f32; 2]| naval_conditions::Climate {
        snowline_m: 3000.0,
        thaw: 1.0,
        leaf: 1.0,
        dry: 0.0,
        place: Some(place),
    };
    let (malibu, _) = species_of(Some(naval_conditions::Climate {
        dry: 1.0,
        ..summer([34.03, -118.7])
    }));
    assert!(count(&malibu, Species::LiveOak) > 100, "{malibu:?}");
    assert_eq!(count(&malibu, Species::Redwood), 0, "{malibu:?}");

    // Japan (Kamakura): the laurel forest and the plantations' sugi, none of California's or Kent's trees.
    let (japan, _) = species_of(Some(summer([35.3, 139.55])));
    assert!(
        count(&japan, Species::Laurel) + count(&japan, Species::Sugi) > 100,
        "{japan:?}"
    );
    for foreign in [Species::LiveOak, Species::MontereyPine, Species::ScotsPine] {
        assert_eq!(count(&japan, foreign), 0, "{foreign:?}: {japan:?}");
    }

    // The Solomons: rainforest.
    let (solomons, _) = species_of(Some(summer([-9.43, 160.0])));
    assert!(count(&solomons, Species::Rainforest) > 100, "{solomons:?}");

    // Kent as itself, in summer: oak and ash, nothing of the south's.
    let (kent, _) = species_of(Some(summer([51.1, 1.0])));
    assert!(
        count(&kent, Species::Oak) + count(&kent, Species::Ash) > 100,
        "{kent:?}"
    );
    for foreign in [Species::LiveOak, Species::Olive, Species::OasisPalm] {
        assert_eq!(count(&kent, foreign), 0, "{foreign:?}: {kent:?}");
    }

    // The Nile's delta: date palms; Savannah: pines and live oaks.
    let (delta, _) = species_of(Some(summer([30.8, 31.0])));
    assert!(count(&delta, Species::OasisPalm) > 50, "{delta:?}");
    assert_eq!(count(&delta, Species::Oak), 0, "{delta:?}");
    let (savannah, _) = species_of(Some(summer([32.08, -81.09])));
    assert!(
        count(&savannah, Species::ScotsPine) + count(&savannah, Species::LiveOak) > 100,
        "{savannah:?}"
    );
}

#[test]
fn the_canopy_stands_the_trees_on_its_crowns() {
    use naval_terrain::canopy::PAGE_TEXELS;
    use naval_terrain::species::Species;
    use naval_terrain::{CanopyGrid, CanopyPage};
    let gpu = Gpu::headless().unwrap();
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    let scene = support::scene(LandStyle::Chalk);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings::default(),
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
    // The trees placed (not the ground cover's shrubs): chart x, z and height.
    let trees = |terrain: &TerrainLayer| -> Vec<[f32; 3]> {
        let count = u64::from(placed_trees(&gpu, terrain.tree_draw_arguments().unwrap()));
        let bytes = read_buffer(
            &gpu,
            terrain.tree_instances().unwrap(),
            count * naval_terrain::trees::TREE_INSTANCE_BYTES,
        );
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        let cover = [Species::Shrub, Species::GrassTuft, Species::Garrigue]
            .map(|s| Species::ALL.iter().position(|x| *x == s).unwrap() as f32);
        floats
            .chunks(12)
            .filter(|t| !cover.contains(&t[5]))
            .map(|t| [t[0], t[2], t[3]])
            .collect()
    };
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let before = trees(&terrain);
    assert!(before.len() > 1000, "{} trees", before.len());

    // Pages over 16 km about the view's target, known all over. The western half bare. The eastern half under full
    // canopy, a 17 m crown in every fifth texel each way (40 m apart) in its middle.
    let offset = terrain.offset();
    let target = [view.target[0] - offset[0], view.target[2] - offset[1]];
    let texel_m = 8.0;
    let grid = CanopyGrid {
        origin: [target[0] - 8192.0, target[1] - 8192.0],
        texel_m,
        slots: 8,
    };
    let n = PAGE_TEXELS as usize;
    let page_of = |crowns: bool| {
        let mut texels = vec![0u8; n * n * 4];
        for j in 0..n {
            for i in 0..n {
                let t = &mut texels[(j * n + i) * 4..][..4];
                t[3] = 255;
                if crowns {
                    t[2] = 255;
                    if i % 5 == 2 && j % 5 == 2 {
                        t.copy_from_slice(&[17, 0x88, 255, 255]);
                    }
                }
            }
        }
        std::sync::Arc::new(CanopyPage { texels })
    };
    let (bare, forest) = (page_of(false), page_of(true));
    terrain
        .set_canopy_grid(&mut renderer.context(), grid)
        .unwrap();
    for j in 0..8 {
        for i in 0..8 {
            terrain.queue_canopy_page([i, j], if i < 4 { bare.clone() } else { forest.clone() });
        }
    }
    terrain.flush_canopy_pages(&mut renderer.context());
    assert_eq!(terrain.canopy_page_counts(), (64, 0));
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let after = trees(&terrain);
    let side = grid.page_m() * 8.0;
    // Within a half, 64 m clear of its edges.
    let within = |t: &&[f32; 3], east: bool| {
        let (x, z) = (
            f64::from(t[0]) - grid.origin[0] - if east { side / 2.0 } else { 0.0 },
            f64::from(t[1]) - grid.origin[1],
        );
        (64.0..side / 2.0 - 64.0).contains(&x) && (64.0..side - 64.0).contains(&z)
    };
    // The bare half stands no tree, where the land cover stood some.
    let west = before.iter().filter(|t| within(t, false)).count();
    assert!(west > 100, "{west} trees in the west before");
    assert_eq!(after.iter().filter(|t| within(t, false)).count(), 0);
    // Under the canopy, no fewer trees than the land cover stands: a wood's own, and the crowns' outside the woods.
    let east = |trees: &[[f32; 3]]| trees.iter().filter(|t| within(t, true)).count();
    assert!(
        east(&after) >= east(&before),
        "{} trees in the east against {} before",
        east(&after),
        east(&before)
    );
    // A tree stands on each crown (8.5 sixteenths into its texel, 40 m apart from each page's corner), at least as tall
    // as measured (a 5% jitter; a wood's tree as tall as its own where that is taller), grown with the distance or
    // shrinking away at its range's end.
    let on_crown = |t: &&[f32; 3]| {
        (0..2).all(|k| {
            let local = (f64::from(t[k]) - grid.origin[k]).rem_euclid(grid.page_m());
            let along = local - (2.0 + 8.5 / 16.0) * texel_m;
            (along - (along / 40.0).round() * 40.0).abs() < 0.5
        })
    };
    let measured: Vec<_> = after
        .iter()
        .filter(|t| within(t, true) && on_crown(t))
        .collect();
    assert!(
        measured.len() > 200,
        "{} trees on the crowns",
        measured.len()
    );
    let full = measured.iter().filter(|t| t[2] > 17.0 * 0.94).count();
    assert!(
        full * 5 > measured.len() * 4,
        "{full} of {} at their height",
        measured.len()
    );

    // Ignored, the land cover's trees again.
    terrain.show_canopy(false);
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    assert_eq!(trees(&terrain).len(), before.len());

    // A paved road along a row of crowns where a measured tree stands: the measured trees keep off it, as the land
    // cover's do.
    use naval_terrain::{FeatureGrid, FeaturePage, Stroke, StrokeKind};
    terrain.show_canopy(true);
    let anchor = measured[measured.len() / 2];
    let row = f64::from(anchor[1]);
    let near = [f64::from(anchor[0]) - 512.0, row - 512.0];
    let features = FeatureGrid {
        origin: [near, [target[0] - 8192.0, target[1] - 8192.0]],
        texel_m: [4.0, 64.0],
        slots: 1,
    };
    terrain
        .set_feature_grid(&mut renderer.context(), features)
        .unwrap();
    let road = Stroke {
        kind: StrokeKind::Paved,
        half_width: 4.0,
        tone: 0.8,
        points: vec![[near[0], row], [near[0] + 1024.0, row]],
    };
    let page = FeaturePage::rasterize(near, 4.0, &[road], &[]);
    let on_road = |t: &&[f32; 3]| {
        let x = f64::from(t[0]) - near[0];
        (64.0..960.0).contains(&x) && (f64::from(t[1]) - row).abs() < 1.0
    };
    let beside = after.iter().filter(on_road).count();
    assert!(beside > 0, "no tree on the row before the road");
    terrain.queue_feature_page(0, [0, 0], std::sync::Arc::new(page));
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let kept = trees(&terrain);
    assert_eq!(kept.iter().filter(on_road).count(), 0, "of {beside}");
}

#[test]
fn a_wide_slot_stands_a_tree_on_each_parts_crown() {
    use naval_terrain::canopy::PAGE_TEXELS;
    use naval_terrain::species::Species;
    use naval_terrain::{CanopyGrid, CanopyPage};
    // A theatre's air window has slots of 35 to 70 m; here the battle map's trees in 40 m slots, each split into 2 x 2
    // parts of 20 m over the canopy.
    let gpu = Gpu::headless().unwrap();
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    let scene = support::scene(LandStyle::Chalk);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    let mut settings = TerrainSettings::default();
    settings.quality.lod.spacing = 40.0;
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        settings,
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
    let offset = terrain.offset();
    let target = [view.target[0] - offset[0], view.target[2] - offset[1]];
    let texel_m = 8.0;
    let grid = CanopyGrid {
        origin: [target[0] - 8192.0, target[1] - 8192.0],
        texel_m,
        slots: 8,
    };
    terrain
        .set_canopy_grid(&mut renderer.context(), grid)
        .unwrap();
    let side = grid.page_m() * 8.0;
    // The trees standing on the pages, with a 17 m crown in every `period`th texel each way: chart x, z.
    let mut trees_with = |period: usize| -> Vec<[f32; 2]> {
        let n = PAGE_TEXELS as usize;
        let mut texels = vec![0u8; n * n * 4];
        for j in 0..n {
            for i in 0..n {
                let t = &mut texels[(j * n + i) * 4..][..4];
                t[3] = 255;
                if i % period == 0 && j % period == 0 {
                    t.copy_from_slice(&[17, 0x88, 80, 255]);
                }
            }
        }
        let page = std::sync::Arc::new(CanopyPage { texels });
        for j in 0..8 {
            for i in 0..8 {
                terrain.queue_canopy_page([i, j], page.clone());
            }
        }
        terrain.flush_canopy_pages(&mut renderer.context());
        render(
            &gpu,
            &mut renderer,
            &capture,
            &camera,
            &mut [&mut sky, &mut terrain],
        );
        let count = u64::from(placed_trees(&gpu, terrain.tree_draw_arguments().unwrap()));
        let bytes = read_buffer(
            &gpu,
            terrain.tree_instances().unwrap(),
            count * naval_terrain::trees::TREE_INSTANCE_BYTES,
        );
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        let cover = [Species::Shrub, Species::GrassTuft, Species::Garrigue]
            .map(|s| Species::ALL.iter().position(|x| *x == s).unwrap() as f32);
        floats
            .chunks(12)
            .filter(|t| !cover.contains(&t[5]))
            .map(|t| [t[0], t[2]])
            .filter(|t| {
                let (x, z) = (
                    f64::from(t[0]) - grid.origin[0],
                    f64::from(t[1]) - grid.origin[1],
                );
                (0.0..side).contains(&x) && (0.0..side).contains(&z)
            })
            .collect()
    };
    // A crown every 40 m: one tree a slot. Every 16 m: one in each part of each slot, more than twice as many where
    // the distance thins the slots, each on a crown.
    let sparse = trees_with(5);
    let dense = trees_with(2);
    assert!(sparse.len() > 200, "{} trees", sparse.len());
    assert!(
        dense.len() > sparse.len() * 2,
        "{} trees on parts against {} a slot",
        dense.len(),
        sparse.len()
    );
    for t in &dense {
        for k in 0..2 {
            let local = (f64::from(t[k]) - grid.origin[k]).rem_euclid(grid.page_m());
            let along = local - 8.5 / 16.0 * texel_m;
            let off = along - (along / 16.0).round() * 16.0;
            assert!(off.abs() < 0.5, "a tree {off} m off its crown: {t:?}");
        }
    }
}

#[test]
fn the_near_trees_draw_as_meshes_and_sway_in_the_wind() {
    use naval_terrain::near_trees::{NEAR_REACH_M, NEAR_ROOM};
    use naval_terrain::trees::TREE_INSTANCE_BYTES;
    let gpu = Gpu::headless().unwrap();
    let scene = support::scene(LandStyle::Tropical);
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings::default(),
        &sky.sky.environment().bind_group,
    )
    .unwrap();
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // A first frame bakes the forest's shade into the open sky's green: over the deepest forest on the downs, low, 40 m
    // above the ground 150 m off, looking at it.
    let overview = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: support::over_the_scarp(&scene, 3000.0),
        target: support::world(&scene, support::SCARP, 0.0),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    render(
        &gpu,
        &mut renderer,
        &capture,
        &overview,
        &mut [&mut sky, &mut terrain],
    );
    let (bytes, row) = read_rgba8(&gpu, terrain.open_sky_texture());
    let texture = terrain.open_sky_texture();
    let mut deepest = (0u8, 0, 0);
    for z in 0..texture.height() {
        for x in 0..texture.width() {
            let g = bytes[(z * row + x * 4 + 1) as usize];
            if g > deepest.0 && support::on_the_downs(terrain.field(), x, z) {
                deepest = (g, x, z);
            }
        }
    }
    let prepared = terrain.field();
    let chart = [
        f64::from(prepared.origin[0]) + f64::from(deepest.1) * prepared.coarse.cell,
        f64::from(prepared.origin[1]) + f64::from(deepest.2) * prepared.coarse.cell,
    ];
    let spot = Vec3::new(
        (chart[0] + scene.offset[0]) as f32,
        field.height(chart[0], chart[1]) as f32,
        (chart[1] + scene.offset[1]) as f32,
    );
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: spot + Vec3::new(-150.0, 40.0, 0.0),
        target: spot,
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    terrain.set_wind(Vec3::new(8.0, 0.0, 3.0));
    let with = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    // Every tree in a list is a placed tree in view within its meshes' reach, in its species' list.
    let count = placed_trees(&gpu, terrain.tree_draw_arguments().unwrap()) as u64;
    let trees: Vec<[u32; 12]> = bytemuck::cast_slice::<u8, u32>(&read_buffer(
        &gpu,
        terrain.tree_instances().unwrap(),
        count * TREE_INSTANCE_BYTES,
    ))
    .chunks(12)
    .map(|t| {
        let mut t: [u32; 12] = t.try_into().unwrap();
        t[11] &= 0x7fff_ffff;
        t
    })
    .collect();
    let placed: std::collections::HashSet<[u32; 12]> = trees.iter().copied().collect();
    let (args, lists) = terrain.near_tree_buffers().unwrap();
    let words: Vec<u32> =
        bytemuck::cast_slice::<u8, u32>(&read_buffer(&gpu, args, args.size())).to_vec();
    let lists = read_buffer(&gpu, lists, lists.size());
    let mut binned = [0usize; 2];
    let eye = camera.eye - Vec3::new(scene.offset[0] as f32, 0.0, scene.offset[1] as f32);
    for (list, draw) in words.chunks(5).enumerate() {
        assert!(draw[0] > 0, "list {list} draws no mesh");
        let n = draw[1] as usize;
        assert!(n <= NEAR_ROOM as usize);
        binned[list % 2] += n;
        let start = list * NEAR_ROOM as usize * TREE_INSTANCE_BYTES as usize;
        let floats: &[f32] =
            bytemuck::cast_slice(&lists[start..start + n * TREE_INSTANCE_BYTES as usize]);
        for t in floats.chunks(12) {
            let mut bits: [u32; 12] = bytemuck::cast_slice::<f32, u32>(t).try_into().unwrap();
            bits[11] &= 0x7fff_ffff;
            assert!(
                placed.contains(&bits),
                "list {list} holds a tree not placed: {t:?}"
            );
            assert_eq!(t[5] as usize, list / 2, "in its species' list");
            let middle = Vec3::new(t[0], t[1] + t[3] * 0.5, t[2]);
            assert!(
                middle.distance(eye) < NEAR_REACH_M * 1.7,
                "{t:?} beyond the reach"
            );
        }
    }
    // Both levels draw; the whole meshes only within a fifth of a tree's reach (`FULL_SHARE`), from 40 m up a few.
    assert!(
        binned[0] > 3 && binned[1] > 10,
        "binned {binned:?} of {count}"
    );
    let triangles: u64 = words
        .chunks(5)
        .map(|d| u64::from(d[0]) / 3 * u64::from(d[1]))
        .sum();
    eprintln!("near trees: {binned:?} of {count} placed, {triangles} triangles");
    // Off, the impostors draw every tree: the near trees change the picture.
    terrain.set_near_trees(false);
    let without = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let changed = with
        .chunks(4)
        .zip(without.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(changed > 300, "the near trees change {changed} pixels");
    assert!(with.chunks(4).all(|p| p[3] == 255));
    // The wind moves them a second later; a calm leaves them still.
    terrain.set_near_trees(true);
    let later = render_at(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
        1.0,
    );
    let moved = with
        .chunks(4)
        .zip(later.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert!(moved > 100, "the wind moves {moved} pixels");
    terrain.set_wind(Vec3::ZERO);
    let calm = render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );
    let calm_later = render_at(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
        1.0,
    );
    // (The sky's own clock moves a few pixels a level or two.)
    let still = calm
        .chunks(4)
        .zip(calm_later.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 12))
        .count();
    assert_eq!(still, 0, "a calm leaves the trees still");
}

#[test]
fn the_land_casts_into_the_sun_maps_and_does_not_shade_itself_twice() {
    use naval_render::{ShadowSettings, Sun, TestScene};
    let gpu = Gpu::headless().unwrap();
    let mut selection = MapSelection::battle(DEFAULT_MAP);
    // A low morning sun from the east: the 180 m cliffs throw their shadow 2.5 km over the sea to the west of them.
    selection.conditions.sun = Some(naval_conditions::Sun {
        elevation: 4.0,
        azimuth: 100.0,
        morning: true,
    });
    let scene = support::styled(selection.resolve().unwrap(), LandStyle::Tropical);
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    // The test scene's flat sea at y = 0 takes the maps' shadows as a hull's paint or the sea does.
    let mut sea = TestScene::new(&mut renderer.context(), 1).unwrap();
    sea.set_draws_sky(false);
    // No trees: the land's own shadow alone.
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings {
            quality: TerrainQuality::MEDIUM,
            cast_shadows: true,
            ..TerrainSettings::default()
        },
        &sky.sky.environment().bind_group,
    )
    .unwrap();
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // Each frame also written for review (`.build/native/terrain/land-shadow-<name>.png`).
    let out = native_offscreen::build_dir("terrain");
    let mut frame = |name: &str,
                     terrain: &mut TerrainLayer,
                     renderer: &mut Renderer,
                     eye: Vec3,
                     target: Vec3| {
        let camera = Camera {
            curvature: naval_render::camera::EARTH_CURVATURE,
            eye,
            target,
            fov_y_degrees: 40.0,
            near: 0.5,
            far: 60000.0,
        };
        renderer.set_sun(Some(Sun::new(sky.sky.light().direction, target)));
        let pixels = render(
            &gpu,
            renderer,
            &capture,
            &camera,
            &mut [&mut sky, &mut sea, terrain],
        );
        let _ = capture.save_png(&gpu, &out.join(format!("land-shadow-{name}.png")));
        pixels
    };
    let mean = |pixels: &[u8]| {
        pixels
            .chunks(4)
            .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
            .sum::<f64>()
            / (pixels.len() / 4) as f64
            / 3.0
    };

    // The sea 1.8 km out from the cliffs' steepest face, seen from above: in the land's shadow with it casting. (Past
    // the land's patches, 50 m deep: no seabed drawn over the test scene's sea there.)
    let cliff = naval_terrain::review::steepest_coast(&field);
    let sea_at = [cliff.x + cliff.dx * 1800.0, cliff.z + cliff.dz * 1800.0];
    assert!(field.height(sea_at[0], sea_at[1]) < -45.0);
    let (eye, target) = (
        support::world(&scene, sea_at, 300.0),
        support::world(&scene, [sea_at[0], sea_at[1] - 60.0], 0.0),
    );
    let shaded = frame("sea-cast", &mut terrain, &mut renderer, eye, target);
    let patches = terrain.land_shadow_patches();
    assert!(
        patches[0] > 0 && patches[1] > 0,
        "the land casts into the near and wide maps: {patches:?}"
    );
    terrain.set_cast_shadows(false, false);
    let lit = frame("sea-alone", &mut terrain, &mut renderer, eye, target);
    assert_eq!(
        terrain.land_shadow_patches(),
        [0; naval_render::shadows::MAP_COUNT]
    );
    assert!(
        mean(&shaded) < mean(&lit) * 0.8,
        "the land's shadow falls on the sea: {} against {} lit",
        mean(&shaded),
        mean(&lit)
    );

    // The hill's sunlit east face and the slopes it shades, the maps about them: the same with the land casting as
    // without, its slopes in its own shadow not darker twice, and no texel of a lit slope shading itself.
    let hill = support::HILL;
    let (eye, target) = (
        support::world(&scene, [hill[0] + 2700.0, hill[1] - 200.0], 600.0),
        support::world(&scene, [hill[0] - 200.0, hill[1]], 300.0),
    );
    let alone = frame("land-alone", &mut terrain, &mut renderer, eye, target);
    terrain.set_cast_shadows(true, true);
    let casting = frame("land-cast", &mut terrain, &mut renderer, eye, target);
    assert!(terrain.land_shadow_patches()[1] > 0);
    let changed = casting
        .chunks(4)
        .zip(alone.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    assert!(
        changed * 100 < casting.len() / 4,
        "the land looks as its own baked shadow has it: {changed} pixels changed"
    );
    assert!(
        (mean(&casting) - mean(&alone)).abs() < 1.0,
        "{} against {}",
        mean(&casting),
        mean(&alone)
    );
}

#[test]
fn the_medium_setting_draws_no_trees() {
    let gpu = Gpu::headless().unwrap();
    let scene = support::scene(LandStyle::Tropical);
    let field = support::land();
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 64, 36, CAPTURE_FORMAT).unwrap();
    let sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    let environment = sky.sky.environment();
    let medium = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings {
            quality: TerrainQuality::MEDIUM,
            ..TerrainSettings::default()
        },
        &environment.bind_group,
    )
    .unwrap();
    assert!(medium.tree_draw_arguments().is_none());
}

#[test]
fn surface_maps_paint_the_land_where_their_imagery_lies() {
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
        TerrainSettings::default(),
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
    // Maps over the whole chart: magenta grassland, first with no imagery anywhere (alpha 0), then with imagery.
    let p = terrain.field();
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let extent = f64::from(p.grid.columns - 1) * p.grid.cell;
    let size = 64u32;
    let n = (size * size) as usize;
    let mut maps = naval_terrain::SurfaceMaps {
        origin,
        extent_m: extent,
        size,
        albedo: [255u8, 0, 255, 0].repeat(n),
        cover: vec![naval_terrain::surface_maps::cover::GRASS; n],
    };
    assert!(
        terrain
            .set_surface_maps(&mut renderer.context(), &maps)
            .is_ok()
    );
    assert!(terrain.has_surface_maps());
    assert_same(
        &draw!(),
        &procedural,
        "no imagery: the procedural cover alone",
    );
    for pixel in maps.albedo.chunks_mut(4) {
        pixel[3] = 255;
    }
    terrain
        .set_surface_maps(&mut renderer.context(), &maps)
        .unwrap();
    let painted = draw!();
    // How magenta the pixels the maps touch are (those that differ from the procedural land).
    let magenta = |image: &[u8]| {
        let (sum, count) = image
            .chunks(4)
            .zip(procedural.chunks(4))
            .filter(|(_, b)| b[..3] != [0, 0, 0])
            .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
            .fold((0.0, 0usize), |(sum, count), (p, _)| {
                (
                    sum + (f64::from(p[0]) + f64::from(p[2])) / 2.0 - f64::from(p[1]),
                    count + 1,
                )
            });
        sum / count.max(1) as f64
    };
    // The aerial view over the lowland and the scarp: the photograph shows on the land, not on the cliffs' steep faces,
    // which keep the style's chalk.
    let changed = painted
        .chunks(4)
        .zip(procedural.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    assert!(
        changed > (width * height / 20) as usize,
        "the maps change {changed} pixels"
    );
    assert!(painted.chunks(4).all(|p| p[3] == 255));
    // The photograph's colour shows (calibrated and its chroma capped, but there): magenta imagery against green
    // imagery of the same cover.
    for pixel in maps.albedo.chunks_mut(4) {
        pixel.copy_from_slice(&[40, 140, 40, 255]);
    }
    terrain
        .set_surface_maps(&mut renderer.context(), &maps)
        .unwrap();
    let green = draw!();
    assert!(
        magenta(&painted) > magenta(&green) + 4.0,
        "the photograph's colour shows: {} against {}",
        magenta(&painted),
        magenta(&green)
    );
    terrain.clear_surface_maps(&mut renderer.context());
    assert!(!terrain.has_surface_maps());
    assert_same(&draw!(), &procedural, "cleared: the procedural cover again");
    maps.cover.pop();
    assert!(
        terrain
            .set_surface_maps(&mut renderer.context(), &maps)
            .is_err()
    );
}

/// Real buildings standing on the land (`TerrainLayer::set_buildings_stand`) take the built-up class's own lots, roofs
/// and streets off: close over a town the ground is the photograph with the ground's mottle, far flatter than the
/// lots; the setting outlasts new maps, and every other class draws as before.
#[test]
fn standing_buildings_take_the_built_up_lots_off() {
    use naval_terrain::surface_maps::cover;
    let gpu = Gpu::headless().unwrap();
    let mut scene = support::scene(LandStyle::Chalk);
    // Summer: no snow over the town (the authored February lies in patches well below its snowline).
    scene.style.climate = naval_conditions::Climate {
        snowline_m: 4000.0,
        thaw: 1.0,
        leaf: 1.0,
        ..naval_conditions::Climate::AUTHORED
    };
    let field = support::land();
    // A flat stretch of land: the first point on a 250 m lattice 15-120 m up whose ground 150 m about stays within 2 m.
    let [x0, z0, x1, z1] = field.bounds();
    let around = [
        (150.0, 0.0),
        (-150.0, 0.0),
        (0.0, 150.0),
        (0.0, -150.0),
        (106.0, 106.0),
        (-106.0, -106.0),
        (106.0, -106.0),
        (-106.0, 106.0),
    ];
    let flat = |x: f64, z: f64| {
        let h = field.height(x, z);
        (15.0..120.0).contains(&h)
            && around
                .iter()
                .all(|(dx, dz)| (field.height(x + dx, z + dz) - h).abs() < 2.0)
    };
    let lattice = (0..)
        .map(|j| z0 + 500.0 + f64::from(j) * 250.0)
        .take_while(|z| *z < z1 - 500.0)
        .flat_map(|z| {
            (0..)
                .map(move |i| (x0 + 500.0 + f64::from(i) * 250.0, z))
                .take_while(|(x, _)| *x < x1 - 500.0)
        });
    let (x, z) = lattice
        .into_iter()
        .find(|(x, z)| flat(*x, *z))
        .expect("flat land on the map");
    let h = field.height(x, z);
    let (width, height) = (320, 180);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    // No trees: their placement appends in no fixed order.
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
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
    let world = |x: f64, y: f64, z: f64| {
        Vec3::new(
            (x + scene.offset[0]) as f32,
            y as f32,
            (z + scene.offset[1]) as f32,
        )
    };
    // Looking down from 70 m over the point: lots of 15 m, streets of 9 m, a pixel of a few decimetres.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: world(x - 60.0, h + 70.0, z),
        target: world(x + 20.0, h, z),
        fov_y_degrees: 50.0,
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
    let p = terrain.field();
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let extent_m = f64::from(p.grid.columns - 1) * p.grid.cell;
    let size = 64u32;
    let n = (size * size) as usize;
    let maps = |class: u8| naval_terrain::SurfaceMaps {
        origin,
        extent_m,
        size,
        albedo: [128u8, 124, 118, 255].repeat(n),
        cover: vec![class; n],
    };
    // The share of pixels standing out from the ground about them: more than 6% lighter or darker than the mean of the
    // 13 × 13 pixels about them (a roof, a street, a roof's shaded edge; the ground's mottle stays within it, and the
    // haze's and the light's broad gradients do not count).
    let standing_out = |image: &[u8]| {
        let (w, h) = (width as usize, height as usize);
        let l: Vec<f64> = image
            .chunks(4)
            .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
            .collect();
        let (mut out, mut all) = (0usize, 0usize);
        for y in 6..h - 6 {
            for x in 6..w - 6 {
                let mut sum = 0.0;
                for v in y - 6..=y + 6 {
                    for u in x - 6..=x + 6 {
                        sum += l[v * w + u];
                    }
                }
                let mean = sum / 169.0;
                all += 1;
                if (l[y * w + x] - mean).abs() > mean * 0.06 {
                    out += 1;
                }
            }
        }
        out as f64 / all as f64 * 100.0
    };
    assert!(!terrain.buildings_stand());
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::BUILT))
        .unwrap();
    let lots = draw!();
    terrain.set_buildings_stand(true);
    assert!(terrain.buildings_stand());
    let standing = draw!();
    for (name, image) in [("lots", &lots), ("standing", &standing)] {
        let path = root().join(format!(
            ".build/native/terrain/built-up-{name}-gpu-test.png"
        ));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::save_buffer(&path, image, width, height, image::ExtendedColorType::Rgba8).unwrap();
    }
    // 5.2% and 0.2% (Apple M5 Pro, 2026-10-05).
    let (a, b) = (standing_out(&lots), standing_out(&standing));
    eprintln!("pixels standing out: {a:.1}% with the lots, {b:.1}% with buildings standing");
    assert!(a > 2.0, "the lots show: {a:.1}%");
    assert!(b < a * 0.2, "the lots go: {a:.1}% against {b:.1}%");
    // Kept across new maps.
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::BUILT))
        .unwrap();
    assert!(terrain.buildings_stand());
    assert_same(&draw!(), &standing, "the setting kept with new maps");
    // Grassland draws the same either way.
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::GRASS))
        .unwrap();
    let grass = draw!();
    terrain.set_buildings_stand(false);
    assert_same(&draw!(), &grass, "grassland whether buildings stand or not");
}

/// A past period's country (`TerrainLayer::set_country`) draws the cover's marked texels (`cover::PAST_COUNTRY`) as
/// the region's fields and nothing else: with the country off a mark changes no pixel, with it on and no texel marked
/// the land draws as before bit for bit, and with half the town marked the pixels change where that half lies and
/// nowhere else. A test's marks (`mark_country_for_tests`) mark built-up land as the cover's mark would.
#[test]
fn the_period_country_draws_on_marked_land_alone() {
    use naval_terrain::surface_maps::cover;
    use naval_terrain::{Country, CountryLook};
    let gpu = Gpu::headless().unwrap();
    let mut scene = support::scene(LandStyle::Chalk);
    scene.style.climate = naval_conditions::Climate {
        snowline_m: 4000.0,
        thaw: 1.0,
        leaf: 1.0,
        ..naval_conditions::Climate::AUTHORED
    };
    let field = support::land();
    let (width, height) = (480, 270);
    let capture = Capture::new(&gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    // No trees: their placement appends in no fixed order.
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
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
    let p = terrain.field();
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let extent_m = f64::from(p.grid.columns - 1) * p.grid.cell;
    // Texels of a hundred metres or so, a window's marks' size: the town's edge a strip of the view.
    let size = 1024u32;
    let n = (size * size) as usize;
    let texel = extent_m / f64::from(size);
    // Gentle land 15-120 m up, two kilometres from the sea, on a texel column's west edge: the town marked west of it.
    let [x0, z0, x1, z1] = field.bounds();
    let inland = |x: f64, z: f64| {
        (0..16).all(|k| {
            let a = f64::from(k) * std::f64::consts::TAU / 16.0;
            [600.0, 1200.0, 2000.0]
                .iter()
                .all(|r| field.height(x + r * a.cos(), z + r * a.sin()) > 8.0)
        })
    };
    let (column, z) = (8..size - 8)
        .step_by(4)
        .flat_map(|i| (8..size - 8).step_by(4).map(move |j| (i, j)))
        .map(|(i, j)| (i, origin[1] + (f64::from(j) + 0.5) * texel))
        .find(|(i, z)| {
            let x = origin[0] + f64::from(*i) * texel;
            let h = field.height(x, *z);
            (15.0..120.0).contains(&h)
                && *z > z0 + 2500.0
                && *z < z1 - 2500.0
                && x > x0 + 2500.0
                && x < x1 - 2500.0
                && [(-400.0, 0.0), (400.0, 0.0), (0.0, -400.0), (0.0, 400.0)]
                    .iter()
                    .all(|(dx, dz)| (field.height(x + dx, z + dz) - h).abs() < 6.0)
                && inland(x, *z)
        })
        .expect("gentle land on the map");
    let x = origin[0] + f64::from(column) * texel;
    let h = field.height(x, z);
    let world = |x: f64, y: f64, z: f64| {
        Vec3::new(
            (x + scene.offset[0]) as f32,
            y as f32,
            (z + scene.offset[1]) as f32,
        )
    };
    // From 1600 m over the line, a pixel of 4-6 m: the fields and their edges.
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: world(x, h + 1600.0, z - 800.0),
        target: world(x, h, z),
        fov_y_degrees: 50.0,
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
    // A town, its texels west of column `upto` of the class `west_class`.
    let maps_to = |west_class: u8, upto: u32| naval_terrain::SurfaceMaps {
        origin,
        extent_m,
        size,
        albedo: [128u8, 124, 118, 255].repeat(n),
        cover: (0..n)
            .map(|k| {
                if (k % size as usize) < upto as usize {
                    west_class
                } else {
                    cover::BUILT
                }
            })
            .collect(),
    };
    let maps = |west_class: u8| maps_to(west_class, column);
    let marked_west = cover::BUILT | cover::PAST_COUNTRY;
    let set =
        |terrain: &mut TerrainLayer, renderer: &mut Renderer, maps: naval_terrain::SurfaceMaps| {
            terrain
                .set_surface_maps(&mut renderer.context(), &maps)
                .unwrap();
        };
    set(&mut terrain, &mut renderer, maps(cover::BUILT));
    let town = draw!();
    // Off (`Period::Today`): the mark changes nothing.
    assert!(!terrain.draws_country());
    set(&mut terrain, &mut renderer, maps(marked_west));
    assert!(
        draw!() == town,
        "with the country off a mark draws as its class"
    );
    // On, nothing marked: bit for bit as before.
    let look = CountryLook {
        country: Country::Britain,
        months: 5.0,
    };
    terrain.set_country(Some(look));
    assert!(terrain.draws_country());
    set(&mut terrain, &mut renderer, maps(cover::BUILT));
    assert!(
        draw!() == town,
        "with the country on and nothing marked, as before"
    );
    // Where the west's texels paint the picture: the pixels their cover changes (water, unlike the town anywhere); and
    // where the texels within the country's reach of them do (`surface.wgsl`'s `COUNTRY_REACH`, 1.5 of Britain's
    // 150 m fields, and a texel of the lookup's): a field the country takes is drawn whole, out to there.
    terrain.set_country(None);
    set(&mut terrain, &mut renderer, maps(cover::WATER));
    let grass = draw!();
    let reach_texels = (150.0 * 1.5 / texel).ceil() as u32 + 1;
    set(
        &mut terrain,
        &mut renderer,
        maps_to(cover::WATER, column + reach_texels),
    );
    let within = draw!();
    let differs_by = |a: &[u8], b: &[u8], levels: u8| -> Vec<bool> {
        a.chunks(4)
            .zip(b.chunks(4))
            .map(|(p, q)| {
                p[..3]
                    .iter()
                    .zip(&q[..3])
                    .any(|(x, y)| x.abs_diff(*y) > levels)
            })
            .collect()
    };
    let differs = |a: &[u8], b: &[u8]| differs_by(a, b, 2);
    let painted = differs_by(&grass, &town, 0);
    let reach = differs_by(&within, &town, 0);
    let west_px = painted.iter().filter(|p| **p).count();
    // On, the west marked: its pixels change, the rest do not.
    terrain.set_country(Some(look));
    set(&mut terrain, &mut renderer, maps(marked_west));
    let country = draw!();
    let changed = differs(&country, &town);
    let outside = changed
        .iter()
        .zip(&reach)
        .filter(|(c, r)| **c && !**r)
        .count();
    let inside = changed
        .iter()
        .zip(&painted)
        .filter(|(c, p)| **c && **p)
        .count();
    for (name, image) in [("town", &town), ("country", &country)] {
        let path = root().join(format!(".build/native/terrain/country-{name}-gpu-test.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::save_buffer(&path, image, width, height, image::ExtendedColorType::Rgba8).unwrap();
    }
    let reach_px = reach.iter().filter(|p| **p).count();
    eprintln!(
        "texels of {texel:.0} m: the country changes {inside} of the west's {west_px} pixels, {outside} beyond its \
         reach ({reach_px})"
    );
    assert!(
        west_px > (width * height / 5) as usize && reach_px < (width * height * 4 / 5) as usize,
        "the view takes in both sides: {west_px} pixels west, {reach_px} within reach"
    );
    assert_eq!(
        outside, 0,
        "the country changes no pixel beyond the marked land's reach"
    );
    assert!(
        inside * 10 > west_px * 9,
        "the country changes the marked land: {inside} of {west_px} pixels"
    );
    // A test's marks: the town's built-up land marked by its noise (all of it at 1).
    set(&mut terrain, &mut renderer, maps(cover::BUILT));
    terrain.mark_country_for_tests(1.0);
    let marked = draw!();
    let all = differs(&marked, &town).iter().filter(|c| **c).count();
    assert!(
        all > (width * height * 9 / 10) as usize,
        "a test's marks mark the town: {all} pixels"
    );
    terrain.mark_country_for_tests(0.0);
    assert!(draw!() == town, "no test's marks: as before");
}

/// The GPU bake (`sky_bake.wgsl`) is the CPU model of what the web bakes (`naval_terrain::open_sky`, itself pinned to the
/// web's read-back), over the test land's cliffs and ravine: the attribute grid's texels within rounding, and clearly
/// not the walk `TerrainBake.ts` reads as.
#[test]
fn the_open_sky_bake_is_what_the_web_bakes() {
    use naval_terrain::open_sky::{BakeInput, open_sky, open_sky_as_written};
    let gpu = Gpu::headless().unwrap();
    let scene = support::scene(LandStyle::Snow);
    let field = support::land();
    let capture = Capture::new(&gpu, 64, 64);
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 64, 64, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let mut terrain = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings::default(),
        &sky.sky.environment().bind_group,
    )
    .unwrap();
    let camera = Camera {
        eye: support::world(
            &scene,
            [support::COAST_X - 1000.0, support::RAVINE_Z],
            900.0,
        ),
        target: support::world(&scene, [support::COAST_X + 1000.0, support::RAVINE_Z], 0.0),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
        curvature: naval_render::camera::EARTH_CURVATURE,
    };
    render(
        &gpu,
        &mut renderer,
        &capture,
        &camera,
        &mut [&mut sky, &mut terrain],
    );

    let texture = terrain.open_sky_texture();
    let (width, height) = (texture.width(), texture.height());
    let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("open sky read-back"),
        size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("open sky copy"),
        });
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
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = buffer.slice(..).get_mapped_range().unwrap().to_vec();

    // The layer let its uploaded arrays go; the same field prepared again has them.
    let prepared = &naval_terrain::field::PreparedField::new(&field);
    assert_eq!(prepared.quanta, terrain.field().quanta);
    let input = BakeInput {
        grid: prepared.grid,
        heights: &prepared.heights,
        gradients: &prepared.gradients,
    };
    let (mut checked, mut off, mut shaded, mut as_written_off, mut darkest) = (0, 0, 0, 0, 1.0f32);
    for z in (0..height).step_by(7) {
        for x in (0..width).step_by(7) {
            let baked = f32::from(bytes[(z * row + x * 4) as usize]) / 255.0;
            checked += 1;
            // A step landing on a rounding boundary may pick its neighbour on one side: rare, and allowed for.
            if (baked - open_sky(&input, x * 2, z * 2)).abs() > 1.5 / 255.0 {
                off += 1;
            }
            // Where the land shuts out some sky (most of the map is open sea, the same either way).
            if baked < 0.98 {
                shaded += 1;
                if (baked - open_sky_as_written(&input, x * 2, z * 2)).abs() > 1.5 / 255.0 {
                    as_written_off += 1;
                }
            }
            darkest = darkest.min(baked);
        }
    }
    assert!(
        off * 500 <= checked,
        "{off} of {checked} texels differ from the model"
    );
    assert!(
        as_written_off * 2 > shaded,
        "only {as_written_off} of {shaded} shaded texels differ from the walk as written"
    );
    assert!(
        darkest < 0.5,
        "the ravine's walls should shut out most of the sky: darkest {darkest}"
    );
}

/// The ground materials (`naval_terrain::ground_materials`): handed in, they change the land close up (at this small size
/// a pixel spans more ground, so they fade sooner: a twentieth of the pixels at least) without moving its mean brightness; hidden, or past their fade (a coast 20 km off), the land is drawn exactly as with none; the Medium
/// setting takes none.
#[test]
fn the_ground_materials_add_structure_close_up_and_nothing_far_off() {
    let gpu = Gpu::headless().unwrap();
    let scene = support::scene(LandStyle::Chalk);
    let field = support::land();
    let files: Vec<Vec<u8>> = naval_terrain::ground_materials::material_files()
        .iter()
        .map(|f| std::fs::read(root().join(f)).unwrap())
        .collect();
    let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let materials = naval_terrain::GroundMaterials::from_sources(&refs, 256).unwrap();
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
        TerrainSettings::default(),
        &environment.bind_group,
    )
    .unwrap();
    assert!(terrain.takes_ground_materials());
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // The Medium setting has no room for them.
    let mut medium = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings {
            quality: TerrainQuality::MEDIUM,
            ..TerrainSettings::default()
        },
        &environment.bind_group,
    )
    .unwrap();
    assert!(!medium.takes_ground_materials());
    assert!(
        medium
            .set_ground_materials(&mut renderer.context(), &materials)
            .is_err()
    );
    // 50 m over the main coast looking 450 m ahead, and the review's coast from 20 km off.
    let coast = review::main_coast(&field);
    let [dx, dz] = coast.direction;
    let at = |d: f64, y: f64| {
        Vec3::new(
            (dx * d + scene.offset[0]) as f32,
            y as f32,
            (dz * d + scene.offset[1]) as f32,
        )
    };
    let ground = |d: f64| field.height(dx * d, dz * d).max(0.0);
    let near = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: at(coast.hit - 150.0, ground(coast.hit) + 50.0),
        target: at(coast.hit + 300.0, ground(coast.hit + 300.0)),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    let view = review::preset(&field, scene.offset, "coast20").unwrap();
    let far = Camera {
        eye: Vec3::new(view.eye[0] as f32, view.eye[1] as f32, view.eye[2] as f32),
        target: Vec3::new(
            view.target[0] as f32,
            view.target[1] as f32,
            view.target[2] as f32,
        ),
        fov_y_degrees: view.fov as f32,
        ..near
    };
    macro_rules! draw {
        ($camera:expr) => {
            render(
                &gpu,
                &mut renderer,
                &capture,
                $camera,
                &mut [&mut sky, &mut terrain],
            )
        };
    }
    let (near_without, far_without) = (draw!(&near), draw!(&far));
    terrain
        .set_ground_materials(&mut renderer.context(), &materials)
        .unwrap();
    assert!(terrain.has_ground_materials());
    let (near_with, far_with) = (draw!(&near), draw!(&far));
    let changed = near_with
        .chunks(4)
        .zip(near_without.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 4))
        .count();
    assert!(
        changed > (width * height / 20) as usize,
        "the materials change {changed} pixels close up"
    );
    let mean = |image: &[u8]| {
        image
            .chunks(4)
            .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
            .sum::<f64>()
            / (image.len() / 4) as f64
            / 3.0
    };
    assert!(
        (mean(&near_with) - mean(&near_without)).abs() < 2.0,
        "the mean brightness stays: {} against {}",
        mean(&near_with),
        mean(&near_without)
    );
    assert_same(
        &far_with,
        &far_without,
        "past their fade: the land as before",
    );
    terrain.show_ground_materials(false);
    assert_same(&draw!(&near), &near_without, "hidden: the land as before");
}

#[test]
fn fine_relief_reshapes_the_land_near_the_camera() {
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
        TerrainSettings::default(),
        &environment.bind_group,
    )
    .unwrap();
    assert!(terrain.takes_relief());
    terrain.set_light(&TerrainLight {
        light: sky.sky.light(),
        ambient: scene.ambient as f32,
        flash: 0.0,
    });
    // 2 × 2 pages of 4 m texels about the main coast, 4 km a side.
    let coast = review::main_coast(&field);
    let [dx, dz] = coast.direction;
    let page_m = 4.0 * f64::from(naval_terrain::relief::CONTENT);
    let grid = naval_terrain::ReliefGrid {
        origin: [dx * coast.hit - page_m, dz * coast.hit - page_m],
        texel_m: 4.0,
        slots: 2,
    };
    // The Medium setting has no room for them.
    let mut medium = TerrainLayer::new(
        &mut renderer.context(),
        &field,
        &scene,
        TerrainSettings {
            quality: TerrainQuality::MEDIUM,
            ..TerrainSettings::default()
        },
        &environment.bind_group,
    )
    .unwrap();
    assert!(!medium.takes_relief());
    assert!(
        medium
            .set_relief_grid(&mut renderer.context(), grid)
            .is_err()
    );
    // 50 m over the main coast looking 450 m ahead.
    let at = |d: f64, y: f64| {
        Vec3::new(
            (dx * d + scene.offset[0]) as f32,
            y as f32,
            (dz * d + scene.offset[1]) as f32,
        )
    };
    let ground = |d: f64| field.height(dx * d, dz * d).max(0.0);
    let near = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: at(coast.hit - 150.0, ground(coast.hit) + 50.0),
        target: at(coast.hit + 300.0, ground(coast.hit + 300.0)),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    macro_rules! draw {
        () => {
            render(
                &gpu,
                &mut renderer,
                &capture,
                &near,
                &mut [&mut sky, &mut terrain],
            )
        };
    }
    let without = draw!();
    // Ridges 12 m high and 48 m apart, running north and south, a crest on the first texel: finer than the field's 40 m
    // samples hold.
    terrain
        .set_relief_grid(&mut renderer.context(), grid)
        .unwrap();
    let n = naval_terrain::relief::PAGE_TEXELS;
    for j in 0..2 {
        for i in 0..2 {
            let texels: Vec<f32> = (0..n * n)
                .map(|k| {
                    let x = grid.texel_middle([i, j], k % n, k / n)[0]
                        - grid.texel_middle([0, 0], 0, 0)[0];
                    6.0 * (x * std::f64::consts::TAU / 48.0).cos() as f32
                })
                .collect();
            let page = naval_terrain::ReliefPage::from_residual(&texels, 4.0);
            assert!((page.peak - 6.0).abs() < 0.1, "{}", page.peak);
            terrain.queue_relief_page([i, j], std::sync::Arc::new(page));
        }
    }
    terrain.flush_relief_pages(&mut renderer.context());
    assert_eq!(terrain.relief_counts().0, 4);
    let with = draw!();
    let changed = with
        .chunks(4)
        .zip(without.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 4))
        .count();
    assert!(
        changed > (width * height / 20) as usize,
        "the relief changes {changed} pixels close up"
    );
    assert!(
        terrain.relief_counts().2 > 0,
        "patches drawn dense near the camera"
    );
    terrain.show_relief(false);
    assert_same(&draw!(), &without, "hidden: the land as before");
    assert_eq!(
        terrain.relief_counts().2,
        0,
        "none dense with the relief hidden"
    );
}
