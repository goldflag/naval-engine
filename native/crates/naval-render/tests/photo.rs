//! Photo mode's post on a real GPU (`naval_render::photo`): the finish left at its defaults draws the game's frame, each
//! look, the grain, the vignette, the roll and the exposure compensation change it as they say, and the depth of field
//! blurs what is out of focus while the frame stays whole. The game's own look (`Renderer::set_look`) grades play, its
//! film running on the clock, and photo mode's replaces it.

use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    DepthOfField, FrameInput, FrameTime, PhotoLook, PhotoPost, RenderOutput, Renderer, TestScene,
};

fn render(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &mut TestScene,
    capture: &Capture,
    camera: &naval_render::Camera,
) -> Vec<u8> {
    render_at(gpu, renderer, scene, capture, camera, 1)
}

fn render_at(
    gpu: &Gpu,
    renderer: &mut Renderer,
    scene: &mut TestScene,
    capture: &Capture,
    camera: &naval_render::Camera,
    frame: u64,
) -> Vec<u8> {
    renderer
        .render(
            FrameInput {
                camera,
                time: FrameTime::fixed(frame, 1.0 / 60.0),
            },
            &mut [scene],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: capture.width,
                height: capture.height,
            },
        )
        .unwrap();
    capture.read_rgba(gpu).unwrap()
}

fn mean(pixels: &[u8]) -> f64 {
    pixels
        .chunks(4)
        .map(|p| f64::from(p[0]) + f64::from(p[1]) + f64::from(p[2]))
        .sum::<f64>()
        / (pixels.len() as f64 / 4.0 * 3.0)
}

fn changed(a: &[u8], b: &[u8]) -> f64 {
    let differ = a
        .chunks(4)
        .zip(b.chunks(4))
        .filter(|(x, y)| x[..3].iter().zip(&y[..3]).any(|(p, q)| p.abs_diff(*q) > 6))
        .count();
    differ as f64 / (a.len() / 4) as f64
}

/// The mean difference between neighbouring pixels: what a blur lowers.
fn sharpness(pixels: &[u8], width: usize) -> f64 {
    let at = |i: usize| -> f64 { pixels[i * 4..i * 4 + 3].iter().map(|c| f64::from(*c)).sum() };
    let n = pixels.len() / 4;
    (1..n)
        .filter(|i| i % width != 0)
        .map(|i| (at(i) - at(i - 1)).abs())
        .sum::<f64>()
        / n as f64
}

#[test]
fn photo_mode_finishes_the_frame_as_it_says() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (128, 80);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sun(Some(TestScene::sun()));
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let camera = TestScene::default_camera();
    let game = render(&gpu, &mut renderer, &mut scene, &capture, &camera);

    // At its defaults the finish is the game's frame.
    renderer.set_photo(Some(PhotoPost::default()));
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );

    // Each look grades the frame; the black-and-white films leave no colour but their tint's (their grain, dust and
    // scratches grey too).
    for look in PhotoLook::ALL.into_iter().skip(1) {
        renderer.set_photo(Some(PhotoPost {
            look,
            ..Default::default()
        }));
        let graded = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
        let share = changed(&game, &graded);
        assert!(share > 0.3, "{look:?} changed {share:.2} of the frame");
        if matches!(
            look,
            PhotoLook::Newsreel | PhotoLook::Infrared | PhotoLook::GunCamera
        ) {
            let colourless = graded
                .chunks(4)
                .all(|p| p[0].abs_diff(p[1]) <= 6 && p[1].abs_diff(p[2]) <= 8);
            assert!(colourless, "{look:?} is black and white");
        }
    }

    // A stop more is brighter; the vignette darkens the corners and not the centre.
    renderer.set_photo(Some(PhotoPost {
        ev: 1.0,
        ..Default::default()
    }));
    let brighter = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(mean(&brighter) > mean(&game) + 5.0);
    renderer.set_photo(Some(PhotoPost {
        vignette: 1.0,
        ..Default::default()
    }));
    let vignetted = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    let pixel = |p: &[u8], x: u32, y: u32| -> f64 {
        let i = ((y * width + x) * 4) as usize;
        p[i..i + 3].iter().map(|c| f64::from(*c)).sum()
    };
    assert!(pixel(&vignetted, 0, 0) < pixel(&game, 0, 0) * 0.5);
    let (cx, cy) = (width / 2, height / 2);
    assert!((pixel(&vignetted, cx, cy) - pixel(&game, cx, cy)).abs() < 6.0);

    // Grain is fixed for its seed: the same frame twice, another seed another pattern.
    let grain = |seed: f32| PhotoPost {
        grain: 1.0,
        grain_seed: seed,
        ..Default::default()
    };
    renderer.set_photo(Some(grain(1.0)));
    let a = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        a
    );
    assert!(changed(&game, &a) > 0.1);
    renderer.set_photo(Some(grain(2.0)));
    assert_ne!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        a
    );

    // The game's look grades play, its film running at 24 frames a second; photo mode's look replaces it.
    renderer.set_photo(None);
    renderer.set_look(PhotoLook::GunCamera);
    let mut at = |frame| render_at(&gpu, &mut renderer, &mut scene, &capture, &camera, frame);
    let (first, same_film_frame, next_film_frame) = (at(1), at(2), at(4));
    assert!(changed(&game, &first) > 0.3);
    // Only the dither, a level a pixel, moves within a film frame.
    let held = changed(&first, &same_film_frame);
    assert!(held < 0.01, "frames 1 and 2 share a film frame: {held:.3}");
    let next = changed(&first, &next_film_frame);
    assert!(next > 0.1, "frame 4 is the next film frame: {next:.3}");
    renderer.set_photo(Some(PhotoPost::default()));
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );
    renderer.set_look(PhotoLook::Natural);

    // The roll turns the picture.
    let roll = 15f32.to_radians();
    renderer.set_photo(Some(PhotoPost {
        roll,
        roll_scale: naval_render::photo::roll_scale(roll, width as f32 / height as f32),
        ..Default::default()
    }));
    let rolled = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(changed(&game, &rolled) > 0.2);

    // The depth of field: a long lens wide open, focused close, blurs the scene; stopped right down it barely does.
    let lens = |aperture: f32| DepthOfField {
        focus_m: 2.0,
        aperture,
        focal_mm: 300.0,
        sensor_mm: 24.0,
    };
    renderer.set_photo(Some(PhotoPost {
        dof: Some(lens(1.4)),
        ..Default::default()
    }));
    let blurred = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    let w = width as usize;
    assert!(
        sharpness(&blurred, w) < sharpness(&game, w) * 0.7,
        "the blur softens the frame: {} against {}",
        sharpness(&blurred, w),
        sharpness(&game, w)
    );
    assert!(
        (mean(&blurred) - mean(&game)).abs() < 12.0,
        "a blur keeps the light"
    );
    renderer.set_photo(Some(PhotoPost {
        dof: Some(DepthOfField {
            focus_m: 10_000.0,
            aperture: 22.0,
            focal_mm: 14.0,
            sensor_mm: 24.0,
        }),
        ..Default::default()
    }));
    let sharp = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(
        changed(&game, &sharp) < 0.02,
        "everything in focus is the frame"
    );
    renderer.set_photo(None);
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );

    // The focus probe reads the depth the frame drew: something in the middle, the sky's clear at the corner.
    let probe = |renderer: &mut Renderer, at: (u32, u32)| -> f32 {
        let buffer = renderer.probe_depth(at).expect("a frame drawn");
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.expect("mapped"));
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        let bytes = buffer.slice(..4).get_mapped_range().unwrap();
        f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    };
    assert!(probe(&mut renderer, (width / 2, height / 2)) > 0.0);
    assert_eq!(probe(&mut renderer, (0, 0)), 0.0);
}

/// A lens with nothing but its defaults: coated, round, no fringes or distortion.
#[test]
fn the_look_tabs_previews_draw_the_last_frame_through_each_look() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (256, 160);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sun(Some(TestScene::sun()));
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let camera = TestScene::default_camera();
    assert!(
        renderer.look_previews(&PhotoLook::ALL, 128).is_none(),
        "nothing drawn yet"
    );
    // The game's own look never reaches the previews: each tile is its look's alone.
    renderer.set_look(PhotoLook::GunCamera);
    render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    renderer.set_look(PhotoLook::Natural);
    let game = render(&gpu, &mut renderer, &mut scene, &capture, &camera);

    let previews = renderer
        .look_previews(&PhotoLook::ALL, 128)
        .expect("a frame was drawn");
    assert_eq!((previews.width, previews.height), (128, 80));
    let slice = previews.buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let mapped = slice.get_mapped_range().unwrap().to_vec();
    assert_eq!(mapped.len(), previews.tile_bytes() * PhotoLook::ALL.len());
    let tiles: Vec<&[u8]> = (0..PhotoLook::ALL.len())
        .map(|i| previews.tile(&mapped, i))
        .collect();

    // Natural is the frame drawn small: its light about the frame's.
    let natural = tiles[0];
    assert!(
        (mean(natural) - mean(&game)).abs() < 4.0,
        "natural {:.1}, the frame {:.1}",
        mean(natural),
        mean(&game)
    );
    for (look, tile) in PhotoLook::ALL.iter().zip(&tiles).skip(1) {
        let share = changed(natural, tile);
        assert!(share > 0.3, "{look:?} changed {share:.2} of its tile");
    }
    let newsreel = tiles[PhotoLook::ALL
        .iter()
        .position(|l| *l == PhotoLook::Newsreel)
        .unwrap()];
    assert!(
        newsreel
            .chunks(4)
            .all(|p| p[0].abs_diff(p[1]) <= 6 && p[1].abs_diff(p[2]) <= 8),
        "newsreel's tile is black and white"
    );
    // The frame itself is as it was.
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );
}

fn lens(sun: Option<naval_render::FlareSun>) -> naval_render::PhotoOptics {
    naval_render::PhotoOptics {
        coating: naval_render::PhotoCoating::Coated,
        blades: 0,
        aperture: 4.0,
        focal_mm: 35.0,
        chromatic: 0.0,
        distortion: false,
        sun,
        lights: naval_render::PointLights::NONE,
    }
}

#[test]
fn photo_mode_lens_flares_only_from_bright_light() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (160, 96);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sun(Some(TestScene::sun()));
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let camera = TestScene::default_camera();
    let game = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    let photo = |optics| PhotoPost {
        optics: Some(optics),
        ..Default::default()
    };

    // The default lens over a frame with no bright source draws it untouched.
    renderer.set_photo(Some(photo(lens(None))));
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game,
        "nothing bright: the frame as it was"
    );

    // The sun just above the frame: its light reaches the lens and flares, the uncoated lens far more.
    let forward = (camera.target - camera.eye).normalize();
    let right = forward.cross(glam::Vec3::Y).normalize();
    let up = right.cross(forward);
    let above = (camera.fov_y_degrees * 0.5 + 3.0).to_radians();
    let sun = naval_render::FlareSun {
        direction: forward * above.cos() + up * above.sin(),
        irradiance: glam::Vec3::splat(6.0),
        angular_radius: 0.004675,
        drawn_radius: 0.0122,
        disc_radiance: glam::Vec3::splat(5.0),
    };
    renderer.set_photo(Some(photo(lens(Some(sun)))));
    let coated = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    renderer.set_photo(Some(photo(naval_render::PhotoOptics {
        coating: naval_render::PhotoCoating::Uncoated,
        ..lens(Some(sun))
    })));
    let uncoated = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(
        mean(&coated) > mean(&game),
        "the coated lens glares a little"
    );
    assert!(
        mean(&uncoated) > mean(&coated) + 3.0,
        "the uncoated lens glares more: {} against {}",
        mean(&uncoated),
        mean(&coated)
    );
    assert!(changed(&game, &uncoated) > 0.2);
    // Stopped down with six blades, the sun's starburst reaches into the frame.
    renderer.set_photo(Some(photo(naval_render::PhotoOptics {
        blades: 6,
        aperture: 16.0,
        ..lens(Some(sun))
    })));
    let star = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(changed(&coated, &star) > 0.01, "the spikes show");

    // Colour fringes grow toward the corners; distortion bends the frame.
    renderer.set_photo(Some(photo(naval_render::PhotoOptics {
        chromatic: 1.0,
        focal_mm: 14.0,
        ..lens(None)
    })));
    let fringed = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert_ne!(fringed, game);
    let i = ((height / 2 * width + width / 2) * 4) as usize;
    assert_eq!(
        fringed[i..i + 3],
        game[i..i + 3],
        "the centre has no fringe"
    );
    let k = naval_render::photo_optics::distortion_of(14.0);
    renderer.set_photo(Some(PhotoPost {
        roll_scale: naval_render::photo_optics::distortion_cover(k),
        optics: Some(naval_render::PhotoOptics {
            distortion: true,
            focal_mm: 14.0,
            ..lens(None)
        }),
        ..Default::default()
    }));
    let bent = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(changed(&game, &bent) > 0.05);

    // Five blades shape the bokeh: the blur differs from a round aperture's.
    let dof = DepthOfField {
        focus_m: 2.0,
        aperture: 4.0,
        focal_mm: 300.0,
        sensor_mm: 24.0,
    };
    renderer.set_photo(Some(PhotoPost {
        dof: Some(dof),
        optics: Some(lens(None)),
        ..Default::default()
    }));
    let round = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    renderer.set_photo(Some(PhotoPost {
        dof: Some(dof),
        optics: Some(naval_render::PhotoOptics {
            blades: 5,
            ..lens(None)
        }),
        ..Default::default()
    }));
    let pentagon = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(
        changed(&round, &pentagon) > 0.005,
        "the blades shape the blur"
    );
    renderer.set_photo(None);
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );
}

/// The mean squared difference between neighbouring pixels within a window (x0..x1, y0..y1): a blur spreading an edge
/// over more pixels lowers it (the plain difference would keep an edge's whole step).
fn window_sharpness(pixels: &[u8], width: u32, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> f64 {
    let at = |x: u32, y: u32| -> f64 {
        let i = ((y * width + x) * 4) as usize;
        pixels[i..i + 3].iter().map(|c| f64::from(*c)).sum()
    };
    let mut sum = 0.0;
    let mut n = 0.0;
    for y in y0..y1 {
        for x in x0.max(1)..x1 {
            sum += (at(x, y) - at(x - 1, y)).powi(2) + (at(x, y) - at(x, y.max(1) - 1)).powi(2);
            n += 1.0;
        }
    }
    sum / n
}

/// The largest difference between two frames within a window.
fn window_change(a: &[u8], b: &[u8], width: u32, (x0, x1, y0, y1): (u32, u32, u32, u32)) -> u8 {
    let mut most = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                most = most.max(a[i + c].abs_diff(b[i + c]));
            }
        }
    }
    most
}

#[test]
fn the_shutter_blurs_what_moves_and_panning_keeps_the_subject_sharp() {
    use naval_render::{MovingBody, MovingShape, Shutter};
    let gpu = Gpu::headless().expect("a GPU adapter");
    let (width, height) = (256, 160);
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, width, height, CAPTURE_FORMAT).expect("renderer");
    renderer.set_sun(Some(TestScene::sun()));
    let mut scene = TestScene::new(&mut renderer.context(), 1).expect("test scene");
    let capture = Capture::new(&gpu, width, height);
    let camera = TestScene::default_camera();
    let game = render(&gpu, &mut renderer, &mut scene, &capture, &camera);

    // The box as a hull, crossing the view broadside on at 120 m/s (fast, so a small frame shows it; along her length
    // her long edges would streak along themselves and stay sharp).
    let yaw = scene.box_yaw(FrameTime::fixed(1, 1.0 / 60.0));
    let forward = glam::Quat::from_rotation_y(yaw) * glam::Vec3::NEG_Z;
    let velocity = forward.cross(glam::Vec3::Y) * 120.0;
    let hull = MovingBody {
        center: glam::Vec3::ZERO,
        velocity,
        angular_velocity: glam::Vec3::ZERO,
        shape: MovingShape::Hull {
            forward,
            up: glam::Vec3::Y,
            length: naval_render::test_scene::BOX_SIZE.z,
            beam: naval_render::test_scene::BOX_SIZE.x,
            bottom: -naval_render::test_scene::BOX_DRAFT,
            top: naval_render::test_scene::BOX_SIZE.y,
        },
    };
    let shutter = |camera_velocity: glam::Vec3| PhotoPost {
        shutter: Some(Shutter {
            exposure_s: 1.0 / 15.0,
            camera_velocity,
        }),
        ..Default::default()
    };
    // The box's middle, the middle of her deck, and a band of the ruled sea in front of her, as the default camera
    // frames them. (Panning, the near sea streaking towards her smears a little over her side at the waterline, as
    // depth alone cannot tell it goes under her.)
    let middle = (width / 2 - 12, width / 2 + 12, height / 2 - 12, height / 2);
    let deck = (width / 2, width / 2 + 16, height / 2 - 8, height / 2 - 3);
    let sea = (8, width - 8, height - 30, height - 8);

    if std::env::var_os("SHUTTER_DUMP").is_some() {
        let dir = native_offscreen::build_dir("shutter-test");
        std::fs::create_dir_all(&dir).unwrap();
        renderer.set_photo(None);
        render(&gpu, &mut renderer, &mut scene, &capture, &camera);
        capture.save_png(&gpu, &dir.join("game.png")).unwrap();
        renderer.set_photo_bodies(&[hull]);
        renderer.set_photo(Some(shutter(glam::Vec3::ZERO)));
        render(&gpu, &mut renderer, &mut scene, &capture, &camera);
        capture.save_png(&gpu, &dir.join("still.png")).unwrap();
        renderer.set_photo(Some(shutter(velocity)));
        render(&gpu, &mut renderer, &mut scene, &capture, &camera);
        capture.save_png(&gpu, &dir.join("panned.png")).unwrap();
        eprintln!("dumped to {}", dir.display());
        return;
    }

    // Nothing moving: the frame as it was, to the byte.
    renderer.set_photo(Some(shutter(glam::Vec3::ZERO)));
    renderer.set_photo_bodies(&[]);
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game,
        "a still scene through the shutter is the scene"
    );

    // The camera still: the hull blurs, the sea stays.
    renderer.set_photo_bodies(&[hull]);
    let still = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(
        window_sharpness(&still, width, middle) < window_sharpness(&game, width, middle) * 0.8,
        "the moving hull blurs: {} against {}",
        window_sharpness(&still, width, middle),
        window_sharpness(&game, width, middle)
    );
    assert_eq!(
        window_change(&still, &game, width, sea),
        0,
        "the still sea in front of her stays sharp"
    );
    assert!(
        (mean(&still) - mean(&game)).abs() < 3.0,
        "the blur keeps the light"
    );

    // Panning: the camera rides with her; she stays sharp and the sea streaks past.
    renderer.set_photo(Some(shutter(velocity)));
    let panned = render(&gpu, &mut renderer, &mut scene, &capture, &camera);
    assert!(
        window_change(&panned, &game, width, deck) <= 2,
        "the subject stays sharp: {}",
        window_change(&panned, &game, width, deck)
    );
    assert!(
        window_sharpness(&panned, width, sea) < window_sharpness(&game, width, sea) * 0.9,
        "the sea streaks: {} against {}",
        window_sharpness(&panned, width, sea),
        window_sharpness(&game, width, sea)
    );

    // Off: the game's frame.
    renderer.set_photo(None);
    assert_eq!(
        render(&gpu, &mut renderer, &mut scene, &capture, &camera),
        game
    );
}
