//! GPU tests: every shader composes and every pipeline builds, the gas atlas bakes puffs where they should be,
//! a turret blast draws over a black scene, smoke behind a splash column is hidden by it, and an AA burst's soot seen
//! through thin smoke stays black when the smoke is merged, but a dense screen in front of it hides it.

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::events::{AirburstInfo, AircraftInfo, ShellInfo};
use naval_effects::{CameraView, CombatEvent, EffectsLayer};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

fn shot(sequence: u64, x: f64) -> CombatEvent {
    CombatEvent {
        sequence,
        tick: 10,
        kind: "shot".into(),
        ship_id: "player".into(),
        position: [x, 14.0, -60.0],
        shell: Some(ShellInfo {
            id: 1000 + sequence as i64,
            caliber_m: 0.38,
            velocity: [820.0, 30.0, 0.0],
            shell_type: Some("AP".into()),
            explosive_kg: None,
        }),
        ..Default::default()
    }
}

fn render(
    renderer: &mut Renderer,
    layer: &mut EffectsLayer,
    camera: &Camera,
    capture: &Capture,
    frame: u64,
) {
    let layers: &mut [&mut dyn SceneLayer] = &mut [layer];
    renderer
        .render(
            FrameInput {
                camera,
                time: FrameTime::fixed(frame, 1.0 / 60.0),
            },
            layers,
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: WIDTH,
                height: HEIGHT,
            },
        )
        .expect("renders");
}

#[test]
fn a_turret_blast_draws() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera::look_at(Vec3::new(-150.0, 40.0, 60.0), Vec3::new(40.0, 14.0, -60.0));
    let view = CameraView::from_uniforms(&camera.uniforms(WIDTH, HEIGHT, FrameTime::default()));

    // Nothing yet: black.
    render(&mut renderer, &mut layer, &camera, &capture, 0);
    let empty = capture.read_rgba(&gpu).unwrap();
    assert!(empty.chunks(4).all(|p| p[0] < 3 && p[1] < 3 && p[2] < 3));

    // Two barrels of one turret, 0.6 s on: flash gone, fireball and gun smoke in the air.
    let events = [shot(1, 0.0), shot(2, 1.2)];
    layer
        .effects
        .update(1.0 / 60.0, 10.0 / 60.0, &view, &events, &[]);
    for _ in 0..35 {
        layer
            .effects
            .update(1.0 / 60.0, 10.0 / 60.0, &view, &[], &[]);
    }
    let counts = layer.effects.counts();
    assert!(counts.gas > 10, "{counts:?}");
    render(&mut renderer, &mut layer, &camera, &capture, 36);
    let pixels = capture.read_rgba(&gpu).unwrap();
    let lit = pixels
        .chunks(4)
        .filter(|p| p[0] > 20 || p[1] > 20 || p[2] > 20)
        .count();
    assert!(lit > 500, "only {lit} lit pixels");
    let stats = layer.stats();
    assert!(stats.gas > 0 && stats.occlusion, "{stats:?}");
}

#[test]
fn a_camera_inside_a_tactical_screen_still_sees_smoke() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
    let mut layer =
        EffectsLayer::new(&mut renderer.context()).expect("the tactical smoke shader composes");
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera::look_at(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, 10.0, -100.0));
    layer
        .effects
        .tactical
        .smoke
        .push(naval_effects::tactical::ScreenSmoke {
            id: 1,
            position: glam::DVec3::new(0.0, 10.0, 0.0),
            radius: 120.0,
            height: 60.0,
            density: 1.0,
            age: 20.0,
        });
    render(&mut renderer, &mut layer, &camera, &capture, 1);
    let pixels = capture.read_rgba(&gpu).unwrap();
    let smoked = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[0] > 12 && pixel[1] > 12 && pixel[2] > 12)
        .count();
    assert!(
        smoked > (WIDTH * HEIGHT * 3 / 4) as usize,
        "screen cleared around camera: {smoked} pixels"
    );
    assert_eq!(layer.stats().gas, 1);
}

#[test]
fn a_tactical_screen_has_billows_instead_of_a_flat_capsule() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
    let mut layer = EffectsLayer::new(&mut renderer.context()).unwrap();
    let capture = Capture::new(&gpu, WIDTH, HEIGHT);
    let camera = Camera::look_at(Vec3::new(0.0, 50.0, 300.0), Vec3::new(0.0, 30.0, 0.0));
    layer
        .effects
        .tactical
        .smoke
        .push(naval_effects::tactical::ScreenSmoke {
            id: 7,
            position: glam::DVec3::new(0.0, 30.0, 0.0),
            radius: 120.0,
            height: 60.0,
            density: 1.0,
            age: 20.0,
        });
    render(&mut renderer, &mut layer, &camera, &capture, 1);
    let pixels = capture.read_rgba(&gpu).unwrap();
    let dir = native_offscreen::build_dir("combat-effects").join("tests");
    std::fs::create_dir_all(&dir).unwrap();
    capture
        .save_png(&gpu, &dir.join("tactical-screen.png"))
        .unwrap();

    // The heart of the screen must have local light/dark folds. A uniform ellipsoid's chord is nearly opaque here,
    // giving a flat grey patch; the second difference rejects its broad, smooth falloff as well as output dither.
    let luminance = |x: usize, y: usize| {
        let pixel = &pixels[(y * WIDTH as usize + x) * 4..];
        (f64::from(pixel[0]) + f64::from(pixel[1]) + f64::from(pixel[2])) / 3.0
    };
    let mut detail = 0.0;
    let mut count = 0;
    for y in 78..102 {
        for x in 108..212 {
            detail += (2.0 * luminance(x, y) - luminance(x - 6, y) - luminance(x + 6, y)).abs();
            count += 1;
        }
    }
    let detail = detail / f64::from(count);
    assert!(
        detail > 2.0,
        "screen is a smooth capsule: local contrast {detail:.2}"
    );
    let smoked = pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 12)
        .count();
    assert!(
        smoked > 3000,
        "screen lost its concealment: {smoked} pixels"
    );

    // Redrawing the same moment must not move the billows or change their opacity.
    render(&mut renderer, &mut layer, &camera, &capture, 1);
    assert_eq!(pixels, capture.read_rgba(&gpu).unwrap());
}

/// The fireball's books and its side book from the checkout, or none where Git LFS has not fetched them.
fn fireball_books() -> Option<Vec<naval_effects::flipbook::FlipbookData>> {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/effects/flipbook");
    ["fireball", "fireball-side"]
        .into_iter()
        .map(|name| {
            let bytes = std::fs::read(dir.join(format!("{name}.flipbook"))).ok()?;
            naval_effects::flipbook::FlipbookData::from_bytes(name, &bytes).ok()
        })
        .collect()
}

/// One puff of the fireball's smoke, still, seen level from 70 m at each eighth of a turn round it: which pixels its gas
/// covers, a row at a time. `NAVAL_EFFECTS_TEST_PNG` names a directory to write the frames to.
fn smoke_round(
    gpu: &Gpu,
    books: &[naval_effects::flipbook::FlipbookData],
    trial: naval_effects::Trial,
) -> Vec<Vec<bool>> {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer
        .set_flipbooks(&gpu.device, &gpu.queue, books)
        .expect("the fireball's books load");
    layer.effects.trial = trial;
    let capture = Capture::new(gpu, WIDTH, HEIGHT);
    let centre = Vec3::new(0.0, 30.0, 0.0);
    let start = Camera::look_at(Vec3::new(70.0, 30.0, 0.0), centre);
    let view = CameraView::from_uniforms(&start.uniforms(WIDTH, HEIGHT, FrameTime::default()));
    layer.effects.update(1.0 / 60.0, 0.0, &view, &[], &[]);
    let (smoke, _) = layer.effects.aircraft_pools();
    let p = smoke.emit(centre.as_dvec3(), None);
    p.seed = 0.37;
    p.size = 40.0;
    p.life = 20.0;
    // A third of its life on: well into the clip's smoke, where it shows the side the eye is on.
    p.age = 6.0;
    p.density = 5.0;
    p.opacity = 1.0;
    p.dissipation_time = 1000.0;
    p.color = naval_effects::Rgb::new(0.5, 0.5, 0.5);
    p.flipbook = naval_effects::flipbook::clip::SMOKE;
    (0..8)
        .map(|eighth| {
            let angle = std::f32::consts::FRAC_PI_4 * eighth as f32;
            let camera = Camera::look_at(
                centre + Vec3::new(angle.cos(), 0.0, angle.sin()) * 70.0,
                centre,
            );
            render(
                &mut renderer,
                &mut layer,
                &camera,
                &capture,
                1 + eighth as u64,
            );
            if let Some(dir) = std::env::var_os("NAVAL_EFFECTS_TEST_PNG") {
                let name = format!(
                    "smoke-{}-{eighth}.png",
                    if trial.views { "views" } else { "card" }
                );
                capture
                    .save_png(gpu, &std::path::Path::new(&dir).join(name))
                    .unwrap();
            }
            capture
                .read_rgba(gpu)
                .unwrap()
                .chunks(4)
                .map(|p| p[0].max(p[1]).max(p[2]) > 6)
                .collect()
        })
        .collect()
}

/// The share of the pixels either mask covers that both do (1: the same outline), `b` read right to left when `mirror`.
fn overlap(a: &[bool], b: &[bool], mirror: bool) -> f64 {
    let (mut both, mut either) = (0usize, 0usize);
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let other = if mirror { WIDTH as usize - 1 - x } else { x };
            let (p, q) = (a[y * WIDTH as usize + x], b[y * WIDTH as usize + other]);
            both += usize::from(p && q);
            either += usize::from(p || q);
        }
    }
    both as f64 / either.max(1) as f64
}

#[test]
fn a_smoke_puff_turns_its_sides_as_the_eye_goes_round_it() {
    let Some(books) = fireball_books() else {
        eprintln!("the fireball's flipbooks are not in the checkout (git lfs pull); skipped");
        return;
    };
    let gpu = Gpu::headless().expect("a GPU");
    let game = naval_effects::Trial::GAME;
    let round = smoke_round(&gpu, &books, game);
    let covered = round[0].iter().filter(|p| **p).count();
    assert!(covered > 2000, "the puff covers only {covered} pixels");
    // A quarter of the way round it shows another side; half way round, the side it showed mirrored.
    let quarter = overlap(&round[0], &round[2], false);
    let behind = overlap(&round[0], &round[4], true);
    assert!(
        quarter < 0.8,
        "a quarter round, the outline overlaps the first by {quarter:.2}"
    );
    assert!(
        behind > 0.95,
        "from behind, the mirrored outline overlaps the first by {behind:.2}"
    );
    // One side only: the same outline from every side, as a camera-facing card shows it.
    let card = smoke_round(
        &gpu,
        &books,
        naval_effects::Trial {
            views: false,
            ..game
        },
    );
    let same = overlap(&card[0], &card[2], false);
    assert!(
        same > 0.95,
        "a card a quarter round overlaps itself by {same:.2}"
    );
}

/// A grey puff and a light one 8 m apart across the view, overlapping on the screen, from 80 m with the camera 0.25 m
/// to one side of the line where they lie equally far and then 0.25 m to the other: the drawn frames (RGBA), as the
/// smoke is layered or merged (look `merge`). The camera's step swaps which puff is nearer.
fn two_puffs_across_their_swap(gpu: &Gpu, merge: bool) -> [Vec<u8>; 2] {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer.effects.trial = naval_effects::Trial {
        merge,
        ..naval_effects::Trial::GAME
    };
    // Merged smoke adds up at the backdrop's copy point, as the client draws it.
    layer.depth_source = naval_effects::layer::DepthSource::Backdrop;
    let capture = Capture::new(gpu, WIDTH, HEIGHT);
    let centre = Vec3::new(0.0, 30.0, 0.0);
    let camera = |side: f32| Camera::look_at(Vec3::new(80.0, 30.0, side), centre);
    let view =
        CameraView::from_uniforms(&camera(0.0).uniforms(WIDTH, HEIGHT, FrameTime::default()));
    layer.effects.update(1.0 / 60.0, 0.0, &view, &[], &[]);
    let (smoke, _) = layer.effects.aircraft_pools();
    for (z, grey, seed) in [(-4.0, 0.3, 0.21), (4.0, 0.9, 0.64)] {
        let p = smoke.emit(glam::DVec3::new(0.0, 30.0, z), None);
        p.seed = seed;
        p.size = 40.0;
        p.life = 20.0;
        p.age = 2.0;
        p.density = 5.0;
        p.opacity = 1.0;
        p.dissipation_time = 1000.0;
        p.color = naval_effects::Rgb::new(grey, grey, grey);
    }
    [-0.25, 0.25].map(|side| {
        render(&mut renderer, &mut layer, &camera(side), &capture, 1);
        assert_eq!(layer.stats().merged, merge, "{:?}", layer.stats());
        if let Some(dir) = std::env::var_os("NAVAL_EFFECTS_TEST_PNG") {
            let name = format!(
                "swap-{}-{side}.png",
                if merge { "merged" } else { "layered" }
            );
            capture
                .save_png(gpu, &std::path::Path::new(&dir).join(name))
                .unwrap();
        }
        capture.read_rgba(gpu).unwrap()
    })
}

/// The mean change of the pixels either frame shows smoke in (0–255), and how many those are.
fn change(a: &[u8], b: &[u8]) -> (f64, usize) {
    let (mut sum, mut count) = (0u64, 0usize);
    for (p, q) in a.chunks(4).zip(b.chunks(4)) {
        if p[..3].iter().chain(&q[..3]).all(|&c| c <= 6) {
            continue;
        }
        sum += (0..3).map(|i| u64::from(p[i].abs_diff(q[i]))).sum::<u64>();
        count += 1;
    }
    (sum as f64 / (3 * count.max(1)) as f64, count)
}

#[test]
fn merged_smoke_does_not_jump_when_two_puffs_swap_places() {
    let gpu = Gpu::headless().expect("a GPU");
    let [before, after] = two_puffs_across_their_swap(&gpu, false);
    let (layered, covered) = change(&before, &after);
    assert!(covered > 3000, "the puffs cover only {covered} pixels");
    let [before, after] = two_puffs_across_their_swap(&gpu, true);
    let (merged, _) = change(&before, &after);
    eprintln!(
        "a 0.5 m step across the swap: layered {layered:.2}, merged {merged:.2} (mean change, 0-255)"
    );
    // Layered, the overlap turns from one puff to the other at once; merged, the step changes almost nothing.
    assert!(
        layered > 4.0,
        "layered, the swap changes the smoke by only {layered:.2}"
    );
    assert!(
        merged < layered / 5.0,
        "merged, the swap changes the smoke by {merged:.2} (layered {layered:.2})"
    );
}

/// A cover as the splash columns' layer publishes it (`naval_render::Cover`): opaque over the whole view at half its
/// size, its water `lies` metres along the view.
fn opaque_cover(gpu: &Gpu, lies: f64) -> naval_render::Cover {
    let target = |label: &str, format: wgpu::TextureFormat| {
        gpu.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: WIDTH / 2,
                    height: HEIGHT / 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default())
    };
    let color = target("test splash cover", wgpu::TextureFormat::Rgba16Float);
    let depth = target("test splash cover lies", wgpu::TextureFormat::R16Float);
    let cleared = |view, clear| {
        Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(clear),
                store: wgpu::StoreOp::Store,
            },
        })
    };
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("test splash cover"),
        color_attachments: &[
            cleared(&color, wgpu::Color::BLACK),
            cleared(
                &depth,
                wgpu::Color {
                    r: lies,
                    ..wgpu::Color::BLACK
                },
            ),
        ],
        ..Default::default()
    });
    gpu.queue.submit([encoder.finish()]);
    let cover = naval_render::Cover::default();
    cover.publish(Some(naval_render::CoverTarget {
        color,
        depth,
        version: 1,
    }));
    cover
}

/// The pixels a puff 80 m off shows in (any channel over 6), merged or layered, with `cover` laid before it.
fn puff_with_cover(gpu: &Gpu, merge: bool, cover: Option<naval_render::Cover>) -> usize {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer.effects.trial = naval_effects::Trial {
        merge,
        ..naval_effects::Trial::GAME
    };
    // The cover is read at the backdrop's copy point, as the client draws it.
    layer.depth_source = naval_effects::layer::DepthSource::Backdrop;
    layer.set_splash_cover(cover);
    let capture = Capture::new(gpu, WIDTH, HEIGHT);
    let centre = Vec3::new(0.0, 30.0, 0.0);
    let camera = Camera::look_at(Vec3::new(80.0, 30.0, 0.0), centre);
    let view = CameraView::from_uniforms(&camera.uniforms(WIDTH, HEIGHT, FrameTime::default()));
    layer.effects.update(1.0 / 60.0, 0.0, &view, &[], &[]);
    let (smoke, _) = layer.effects.aircraft_pools();
    let p = smoke.emit(glam::DVec3::new(0.0, 30.0, 0.0), None);
    p.seed = 0.4;
    p.size = 40.0;
    p.life = 20.0;
    p.age = 2.0;
    p.density = 5.0;
    p.opacity = 1.0;
    p.dissipation_time = 1000.0;
    p.color = naval_effects::Rgb::new(0.7, 0.7, 0.7);
    render(&mut renderer, &mut layer, &camera, &capture, 1);
    assert_eq!(layer.stats().merged, merge, "{:?}", layer.stats());
    capture
        .read_rgba(gpu)
        .unwrap()
        .chunks(4)
        .filter(|p| p[..3].iter().any(|&c| c > 6))
        .count()
}

#[test]
fn smoke_behind_a_splash_column_is_hidden_and_smoke_before_it_is_not() {
    let gpu = Gpu::headless().expect("a GPU");
    for merge in [true, false] {
        let open = puff_with_cover(&gpu, merge, None);
        let hidden = puff_with_cover(&gpu, merge, Some(opaque_cover(&gpu, 40.0)));
        let before = puff_with_cover(&gpu, merge, Some(opaque_cover(&gpu, 200.0)));
        eprintln!(
            "merged {merge}: open {open}, a column before it {hidden}, behind it {before} pixels"
        );
        assert!(open > 2000, "the puff covers only {open} pixels");
        // The column's composite drew before the smoke: a column 40 m off stands in front of the puff and hides it, one
        // 200 m off stands behind it and the puff draws over it whole.
        assert!(
            hidden < open / 50,
            "merged {merge}: {hidden} of {open} pixels show through an opaque column in front"
        );
        assert!(
            before * 100 >= open * 98,
            "merged {merge}: a column behind the puff hides {} of its {open} pixels",
            open - before.min(open)
        );
    }
}

/// A 5-inch AA round's burst 120 m off, 2 s after it burst, with a pale puff 40 m off in front of it of `front`'s density
/// and opacity (a ship's funnel haze over her guns, or a smoke screen) and a dense light one 720 m off behind it: the
/// drawn frame (RGBA), layered or merged.
fn burst_behind_smoke(gpu: &Gpu, merge: bool, front: (f64, f64)) -> Vec<u8> {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer.effects.trial = naval_effects::Trial {
        merge,
        ..naval_effects::Trial::GAME
    };
    // Merged smoke adds up at the backdrop's copy point, as the client draws it.
    layer.depth_source = naval_effects::layer::DepthSource::Backdrop;
    let capture = Capture::new(gpu, WIDTH, HEIGHT);
    let camera = Camera::look_at(Vec3::new(120.0, 300.0, 0.0), Vec3::new(0.0, 300.0, 0.0));
    let view = CameraView::from_uniforms(&camera.uniforms(WIDTH, HEIGHT, FrameTime::default()));
    let fired = CombatEvent {
        sequence: 1,
        tick: 0,
        kind: "aircraft-fire".into(),
        ship_id: "player".into(),
        aircraft: Some(AircraftInfo {
            target: Some([0.0, 300.0, 0.0]),
            airburst: Some(AirburstInfo {
                flight_time: 0.0,
                caliber_m: 0.127,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    layer.effects.update(1.0 / 60.0, 2.0, &view, &[fired], &[]);
    let (smoke, _) = layer.effects.aircraft_pools();
    for (x, size, density, opacity, grey) in [
        (80.0, 60.0, front.0, front.1, 0.95),
        (-600.0, 500.0, 5.0, 1.0, 0.8),
    ] {
        let p = smoke.emit(glam::DVec3::new(x, 300.0, 0.0), None);
        p.seed = 0.3;
        p.size = size;
        p.life = 20.0;
        p.age = 2.0;
        p.density = density;
        p.opacity = opacity;
        p.dissipation_time = 1000.0;
        p.color = naval_effects::Rgb::new(grey, grey, grey);
    }
    render(&mut renderer, &mut layer, &camera, &capture, 1);
    assert_eq!(layer.stats().merged, merge, "{:?}", layer.stats());
    if let Some(dir) = std::env::var_os("NAVAL_EFFECTS_TEST_PNG") {
        let name = format!(
            "burst-{}-{}.png",
            front.1,
            if merge { "merged" } else { "layered" }
        );
        capture
            .save_png(gpu, &std::path::Path::new(&dir).join(name))
            .unwrap();
    }
    capture.read_rgba(gpu).unwrap()
}

/// The mean of a frame's channels (0–255) over the pixels within `reach` of (`x`, `y`).
fn mean_about(frame: &[u8], x: i32, y: i32, reach: i32) -> f64 {
    let (mut sum, mut count) = (0u64, 0u64);
    for py in y - reach..=y + reach {
        for px in x - reach..=x + reach {
            let at = (py as usize * WIDTH as usize + px as usize) * 4;
            sum += frame[at..at + 3].iter().map(|&c| u64::from(c)).sum::<u64>();
            count += 3;
        }
    }
    sum as f64 / count as f64
}

#[test]
fn an_aa_bursts_soot_seen_through_thin_smoke_stays_black_merged() {
    let gpu = Gpu::headless().expect("a GPU");
    let (x, y) = (WIDTH as i32 / 2, HEIGHT as i32 / 2);
    for merge in [false, true] {
        let frame = burst_behind_smoke(&gpu, merge, (1.0, 0.5));
        let middle = mean_about(&frame, x, y, 3);
        let beside = (mean_about(&frame, x - 40, y, 3) + mean_about(&frame, x + 40, y, 3)) / 2.0;
        eprintln!(
            "merged {merge}: the burst's middle {middle:.1}, the smoke beside it {beside:.1} (0-255)"
        );
        // Black soot against the light puff behind it, greyed a little by the haze in front. Merged, the haze's optical
        // depth drew the map's middle of its gas in front of the burst, which then lay behind its own soot at the least
        // weight and took the haze's colour: a pale disc, 166 against 193 beside it.
        assert!(
            middle < beside * 0.6,
            "merged {merge}: the burst's middle {middle:.1} against {beside:.1} beside it"
        );
    }
}

#[test]
fn an_aa_bursts_soot_behind_a_dense_screen_is_mostly_hidden_merged() {
    let gpu = Gpu::headless().expect("a GPU");
    let (x, y) = (WIDTH as i32 / 2, HEIGHT as i32 / 2);
    let frame = burst_behind_smoke(&gpu, true, (5.0, 1.0));
    let middle = mean_about(&frame, x, y, 3);
    let beside = (mean_about(&frame, x - 40, y, 3) + mean_about(&frame, x + 40, y, 3)) / 2.0;
    eprintln!("the burst's middle {middle:.1}, the screen beside it {beside:.1} (0-255)");
    // The screen lets a fifth through where the burst lies behind it (layered, over a light puff and a black one), so
    // the burst darkens it only a little: 152 against 188 beside it, where the screen over nothing shows 150. Weighed by
    // the densest gas in front only, the soot passed under all but the screen's densest level: 129, a dark disc.
    assert!(
        middle > beside * 0.75,
        "the burst's middle {middle:.1} against {beside:.1} beside it"
    );
}
