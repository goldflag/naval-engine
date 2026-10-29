//! Near tiles on a headless adapter (`TerrainLayer::set_near_grid`, `queue_near_tile`; native/docs/terrain/surface.md,
//! "Near
//! tiles"): uploaded by the frames that follow, drawn over the window's maps where they hold imagery, the window's maps
//! showing through where they hold none; a grid of another size than the maps is refused. The cover rewritten in place
//! (`TerrainLayer::queue_cover_patch`, "The period's ground") draws as maps or a tile holding it.

use crate::support;
use naval_conditions::LandStyle;
use std::sync::Arc;

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::forest_shade::SHADE_FRAMES;
use naval_terrain::surface_maps::{NEAR_TILE_TEXELS, cover};
use naval_terrain::{
    CoverLayer, CoverPatch, NearGrid, NearTile, SurfaceMaps, TerrainLayer, TerrainLight,
    TerrainSettings, review,
};

fn render(
    gpu: &Gpu,
    renderer: &mut Renderer,
    capture: &Capture,
    camera: &Camera,
    layers: &mut [&mut dyn SceneLayer],
) -> Vec<u8> {
    // A run of frames, again until two runs draw the same picture (as tests/gpu.rs: the land takes a few frames more to
    // settle since the trees became impostors). Each run outlasts the forest shade's wait between bakes
    // (`SHADE_FRAMES`): with runs of three, the rectangles the first frames asked for were baked after two runs agreed,
    // and the trees they light (the grown trees' impostors finer than the old) drew a little differently in every
    // picture after the first.
    let mut last: Option<Vec<u8>> = None;
    for _ in 0..8 {
        for frame in 0..=u64::from(SHADE_FRAMES) {
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
/// a few levels (at most 0.05% of the bytes, by at most 8 levels: since the trees draw their depth first and then their
/// light where it stands, `Equal`, the later of two such fragments takes the pixel even where the prepass cut its
/// alpha, as the light no longer tests it; it moved a dozen bytes by up to 7 levels where 4 held before).
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
        differ.len() * 2000 <= a.len() && most <= 8,
        "{what}: {} bytes differ, by up to {most}",
        differ.len()
    );
}

#[test]
fn near_tiles_paint_over_the_window_maps_where_they_are_drawn() {
    near_tiles_paint(&Gpu::headless().unwrap(), false);
}

/// As above on a device sampling BC7: the window's maps and the near tiles' colour in BC7, the tiles made so on their
/// worker (`NearTile::from_maps_bc7`).
#[test]
fn near_tiles_in_bc7_paint_as_rgba8_ones_do() {
    let gpu = Gpu::headless_with(wgpu::Features::TEXTURE_COMPRESSION_BC).unwrap();
    if !naval_terrain::bc7::supported(&gpu.device) {
        eprintln!("no BC7 on this adapter: skipped");
        return;
    }
    near_tiles_paint(&gpu, true);
}

fn near_tiles_paint(gpu: &Gpu, bc7: bool) {
    let tile = |maps: &SurfaceMaps| {
        if bc7 {
            NearTile::from_maps_bc7(maps).unwrap()
        } else {
            NearTile::from_maps(maps).unwrap()
        }
    };
    let scene = support::scene(LandStyle::Tropical);
    let field = support::land();
    let (width, height) = (320, 180);
    let capture = Capture::new(gpu, width, height);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).unwrap();
    let mut sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
    sky.sky.apply(&scene.sky.cloudless());
    let environment = sky.sky.environment();
    // The trees' atlas kept as baked: on a BC device its copy would land between captures.
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
    // From 40 m over the lowland behind the coast, looking up the scarp: land alone, well inside the chart (no seabed,
    // and clear of the tile's feathered edges).
    let camera = Camera {
        curvature: naval_render::camera::EARTH_CURVATURE,
        eye: support::world(&scene, [4000.0, 1200.0], 50.0),
        target: support::world(&scene, [5600.0, 1200.0], 120.0),
        fov_y_degrees: 52.0,
        near: 0.5,
        far: 60000.0,
    };
    macro_rules! draw {
        () => {
            render(
                gpu,
                &mut renderer,
                &capture,
                &camera,
                &mut [&mut sky, &mut terrain],
            )
        };
    }
    let procedural = draw!();
    // Green window maps over the whole chart, a near tile's size, so a grid of one slot over the same chart fits.
    let p = terrain.field();
    let origin = [f64::from(p.origin[0]), f64::from(p.origin[1])];
    let extent = f64::from(p.grid.columns - 1) * p.grid.cell;
    let n = (NEAR_TILE_TEXELS * NEAR_TILE_TEXELS) as usize;
    let filled = |rgba: [u8; 4]| SurfaceMaps {
        origin,
        extent_m: extent,
        size: NEAR_TILE_TEXELS,
        albedo: rgba.repeat(n),
        cover: vec![cover::GRASS; n],
    };
    let grid = NearGrid {
        origin,
        tile_m: extent,
        slots: 1,
    };
    // The window's maps alone, then the same with a near grid set first (the grid comes before the maps, which then
    // share its textures) and no tiles yet: alike.
    terrain
        .set_surface_maps(&mut renderer.context(), &filled([40, 140, 40, 255]))
        .unwrap();
    let window = draw!();
    assert_ne!(window, procedural);
    // A grid after the maps is refused: their textures have no room for tiles.
    assert!(
        terrain
            .set_near_grid(&mut renderer.context(), grid)
            .is_err()
    );
    terrain.clear_surface_maps(&mut renderer.context());
    assert_same(&draw!(), &procedural, "the maps cleared");
    // A grid alone draws the procedural cover.
    terrain
        .set_near_grid(&mut renderer.context(), grid)
        .unwrap();
    assert_eq!(terrain.near_grid(), Some(grid));
    assert_same(&draw!(), &procedural, "a grid with no tiles and no maps");
    terrain
        .set_surface_maps(&mut renderer.context(), &filled([40, 140, 40, 255]))
        .unwrap();
    assert_eq!(
        draw!(),
        window,
        "a grid with no tiles: the window's maps alone"
    );
    // A tile holding no imagery: the window's maps show through.
    let empty = tile(&filled([255, 0, 255, 0]));
    terrain.queue_near_tile([0, 0], Arc::new(empty));
    assert!(terrain.holds_near_tile([0, 0]));
    assert_eq!(terrain.near_tile_counts(), (0, 1));
    assert_same(
        &draw!(),
        &window,
        "no imagery in the tile: the window's maps",
    );
    assert_eq!(terrain.near_tile_counts(), (1, 0));
    // A magenta tile: drawn over the window's green.
    let magenta = Arc::new(tile(&filled([255, 0, 255, 255])));
    terrain.queue_near_tile([0, 0], magenta.clone());
    let near = draw!();
    // The pixels it changes, and how much more purple they are (the colour calibrated and its chroma capped, but
    // there).
    let purple = |p: &[u8]| f64::from(p[0]) + f64::from(p[2]) - 2.0 * f64::from(p[1]);
    let differ = |a: &[u8], b: &[u8]| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8);
    let changed: Vec<(&[u8], &[u8])> = near
        .chunks(4)
        .zip(window.chunks(4))
        .filter(|(a, b)| differ(a, b))
        .collect();
    // (The maps' grass takes the tropical style's trees away, and the tile's too: where a tree stood the tile's ground
    // shows as the window's did.)
    let painted = window
        .chunks(4)
        .zip(procedural.chunks(4))
        .filter(|(a, b)| differ(a, b))
        .count();
    assert!(
        changed.len() * 2 > painted && painted > 500,
        "the near tile shows over the window's maps: {} pixels of the {painted} they paint",
        changed.len()
    );
    let more = changed
        .iter()
        .map(|(a, b)| purple(a) - purple(b))
        .sum::<f64>()
        / changed.len() as f64;
    assert!(more > 8.0, "the tile's colour shows: {more}");
    assert!(near.chunks(4).all(|p| p[3] == 255));
    // Another tile in the same slot: the point is the window's maps' again (the slot holds a tile of elsewhere).
    terrain.queue_near_tile([1, 0], magenta);
    draw!();
    assert!(!terrain.holds_near_tile([0, 0]));
    assert!(terrain.holds_near_tile([1, 0]));
    assert_same(&draw!(), &window, "the slot holds another tile");
    terrain.clear_near_tiles(&mut renderer.context());
    assert_eq!(terrain.near_grid(), None);
    assert_same(&draw!(), &window, "no near tiles: the window's maps");
    // The grid must match the maps' side (they share the textures).
    assert!(
        terrain
            .set_near_grid(&mut renderer.context(), NearGrid { slots: 2, ..grid })
            .is_err()
    );
    // (The maps' textures kept their room for tiles: a grid again is taken.)
    terrain
        .set_near_grid(&mut renderer.context(), grid)
        .unwrap();
    assert_same(&draw!(), &window, "the window's maps again");
}

/// A rewrite of the window's cover, or of a drawn near tile's, draws as maps (or a tile) holding the rewritten classes
/// would: the trees placed again on it. One for a tile the layer does not hold is dropped.
#[test]
fn cover_patches_draw_as_the_classes_they_write() {
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
    let extent = f64::from(p.grid.columns - 1) * p.grid.cell;
    let n = (NEAR_TILE_TEXELS * NEAR_TILE_TEXELS) as usize;
    let maps = |class: u8| SurfaceMaps {
        origin,
        extent_m: extent,
        size: NEAR_TILE_TEXELS,
        albedo: [70, 120, 50, 255].repeat(n),
        cover: vec![class; n],
    };
    let whole = |layer: CoverLayer, class: u8, marks: u64| CoverPatch {
        layer,
        at: [0, 0],
        size: [NEAR_TILE_TEXELS; 2],
        classes: vec![class; n],
        marks,
    };
    terrain
        .set_near_grid(
            &mut renderer.context(),
            NearGrid {
                origin,
                tile_m: extent,
                slots: 1,
            },
        )
        .unwrap();
    // The window's maps: grass, then woods; then grass rewritten as woods.
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::GRASS))
        .unwrap();
    let grass = draw!();
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::TREE))
        .unwrap();
    let woods = draw!();
    let differ = grass
        .chunks(4)
        .zip(woods.chunks(4))
        .filter(|(a, b)| a[..3].iter().zip(&b[..3]).any(|(x, y)| x.abs_diff(*y) > 8))
        .count();
    assert!(
        differ > 500,
        "woods draw otherwise than grass: {differ} pixels"
    );
    terrain
        .set_surface_maps(&mut renderer.context(), &maps(cover::GRASS))
        .unwrap();
    assert_same(&draw!(), &grass, "grass again");
    terrain.queue_cover_patch(whole(CoverLayer::Window, cover::TREE, 0));
    assert_same(&draw!(), &woods, "the window's grass rewritten as woods");
    // A near tile of woods, then one of grass rewritten as woods (its marks' number kept).
    let tile = |class: u8, marks: u64| {
        let mut tile = NearTile::from_maps(&maps(class)).unwrap();
        tile.marks = marks;
        Arc::new(tile)
    };
    terrain.queue_near_tile([0, 0], tile(cover::TREE, 1));
    let near_woods = draw!();
    assert_eq!(terrain.near_tile_marks([0, 0]), Some(1));
    terrain.queue_near_tile([0, 0], tile(cover::GRASS, 2));
    assert_eq!(terrain.near_tile_marks([0, 0]), Some(2));
    let near_grass = draw!();
    assert_ne!(near_grass, near_woods);
    terrain.queue_cover_patch(whole(CoverLayer::Near([0, 0]), cover::TREE, 3));
    assert_eq!(terrain.near_tile_marks([0, 0]), Some(3));
    assert_same(
        &draw!(),
        &near_woods,
        "the near tile's grass rewritten as woods",
    );
    assert_eq!(terrain.near_tile_marks([0, 0]), Some(3));
    // For a tile the slot does not hold, or that does not fit: dropped.
    terrain.queue_cover_patch(whole(CoverLayer::Near([1, 0]), cover::GRASS, 4));
    terrain.queue_cover_patch(CoverPatch {
        size: [NEAR_TILE_TEXELS + 1, 1],
        ..whole(CoverLayer::Near([0, 0]), cover::GRASS, 5)
    });
    assert_eq!(terrain.near_tile_marks([1, 0]), None);
    assert_eq!(terrain.near_tile_marks([0, 0]), Some(3));
    assert_same(&draw!(), &near_woods, "nothing rewritten");
}
