//! The trees' impostor atlas shared between lands (`naval_terrain::TreeAtlas`, `TerrainLayer::for_scene_sharing`): the
//! first land to draw bakes it, every land made after registers no bake and draws the same texture (its BC7 copy once
//! that is made), and a land given none keeps its own.

use crate::support;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_conditions::LandStyle;
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};
use naval_sky::{SkyLayer, SkyQuality};
use naval_terrain::{PreparedLand, TerrainLayer, TerrainLight, TerrainSettings, TreeAtlas};

#[test]
fn every_land_given_the_atlas_shares_its_one_bake() {
    shares_one_bake(&Gpu::headless().unwrap());
}

/// As above on a device sampling BC7: the bake is copied to BC7, and the lands made after draw the copy.
#[test]
fn every_land_shares_the_bakes_bc7_copy() {
    let gpu = Gpu::headless_with(wgpu::Features::TEXTURE_COMPRESSION_BC).unwrap();
    if !naval_terrain::bc7::supported(&gpu.device) {
        eprintln!("no BC7 on this adapter: skipped");
        return;
    }
    shares_one_bake(&gpu);
}

fn shares_one_bake(gpu: &Gpu) {
    let scene = support::scene(LandStyle::Tropical);
    let field = support::land();
    let capture = Capture::new(gpu, 64, 36);
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
    let atlas = TreeAtlas::new(&gpu.device);
    // Each land in a renderer of its own, as each theatre scene has; drawn one frame when `draw`.
    let land = |sky: &mut SkyLayer, atlas: Option<&TreeAtlas>, draw: bool| {
        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, 64, 36, CAPTURE_FORMAT).unwrap();
        let mut terrain = TerrainLayer::for_scene_sharing(
            &mut renderer.context(),
            PreparedLand::new(&field),
            &scene,
            TerrainSettings::default(),
            &sky.sky.environment().bind_group,
            atlas,
        )
        .unwrap();
        terrain.set_light(&TerrainLight {
            light: sky.sky.light(),
            ambient: scene.ambient as f32,
            flash: 0.0,
        });
        let bakes = terrain.bakes_tree_impostors();
        if draw {
            let layers: &mut [&mut dyn SceneLayer] = &mut [sky, &mut terrain];
            renderer
                .render(
                    FrameInput {
                        camera: &camera,
                        time: FrameTime {
                            seconds: 0.0,
                            dt: 0.0,
                            frame: 0,
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
        (terrain, bakes)
    };
    // Two made before either draws: each holds the bake, and both draw the one atlas.
    let (first, first_bakes) = land(&mut sky, Some(&atlas), false);
    let (second, second_bakes) = land(&mut sky, Some(&atlas), false);
    assert!(first_bakes && second_bakes);
    assert_eq!(first.tree_impostors(), Some(&atlas.texture()));
    assert_eq!(second.tree_impostors(), Some(&atlas.texture()));
    drop((first, second));
    assert!(!atlas.baked());
    // One draws, and bakes it in its first frame.
    let (third, third_bakes) = land(&mut sky, Some(&atlas), true);
    assert!(third_bakes);
    assert!(atlas.baked());
    assert!(!third.bakes_tree_impostors());
    // On a device sampling BC7 the bake is copied to BC7 on a thread of its own, and the lands draw the copy.
    atlas.wait_compressed();
    if naval_terrain::bc7::supported(&gpu.device) {
        assert_eq!(
            atlas.texture().format(),
            naval_terrain::impostors::COMPRESSED_FORMAT
        );
    }
    // Every land after holds no bake and draws the same atlas.
    let (fourth, fourth_bakes) = land(&mut sky, Some(&atlas), false);
    assert!(!fourth_bakes, "a land made after the bake registers none");
    assert_eq!(fourth.tree_impostors(), Some(&atlas.texture()));
    // A land given none makes and bakes its own.
    let (own, own_bakes) = land(&mut sky, None, false);
    assert!(own_bakes);
    assert_ne!(own.tree_impostors(), Some(&atlas.texture()));
}
