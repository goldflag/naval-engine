//! The layer through the renderer core, under the real sky: a made-up ship draws, her side facing the sun is lit and the
//! other is not, her deckhouse shadows her deck on the side away from the sun, and by night her masthead lamp glows
//! ahead of her. Skips without a GPU. Writes `.build/native/vessel/test-*.png`.

use glam::{DMat4, DVec3, DVec4, Mat4, Vec3, Vec4Swizzles};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu, build_dir};
use naval_render::{Camera, FrameInput, FrameTime, RenderOutput, Renderer, ShadowSettings, Sun};
use naval_sky::{SkyLayer, SkyQuality, SkyScene};
use naval_vessel::setup::{LampKind, LampSetup};
use naval_vessel::{
    ModelId, Setup, VesselInstance, VesselLayer, VesselLighting, VesselModel, no_files,
};

use crate::fixture::{self, BEAM, DRAFT, FREEBOARD, HOUSE};

const W: u32 = 480;
const H: u32 = 270;

struct Bench {
    gpu: Gpu,
    renderer: Renderer,
    sky: SkyLayer,
    layer: VesselLayer,
    capture: Capture,
    model: ModelId,
}

impl Bench {
    fn new() -> Option<Self> {
        let Ok(gpu) = Gpu::headless() else {
            eprintln!("no GPU adapter: skipped");
            return None;
        };
        let mut renderer = Renderer::new(&gpu.device, &gpu.queue, W, H, CAPTURE_FORMAT).unwrap();
        renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
        let sky = SkyLayer::new(&gpu.device, &gpu.queue, SkyQuality::Low);
        let mut layer = VesselLayer::new(
            &mut renderer.context(),
            sky.sky.environment(),
            VesselLighting::daylight(sky.sky.light()),
        )
        .unwrap();
        let setup = Setup {
            draft: Some(DRAFT),
            lights: vec![LampSetup {
                name: "masthead".into(),
                at: [0.0, FREEBOARD + HOUSE + 15.0, -3.7],
                kind: LampKind::Masthead,
                color: None,
                intensity: None,
            }],
            ..Setup::new("plain.glb")
        };
        let model =
            VesselModel::load(&setup, "plain", &fixture::plain().bytes(), &mut no_files).unwrap();
        let model = layer.add_model(&gpu.device, &gpu.queue, &model);
        let capture = Capture::new(&gpu, W, H);
        Some(Self {
            gpu,
            renderer,
            sky,
            layer,
            capture,
            model,
        })
    }

    /// The sun at `elevation` and `azimuth` (degrees), cloudless.
    fn sun(&mut self, elevation: f64, azimuth: f64) -> Vec3 {
        self.sky
            .sky
            .apply(&SkyScene::north_atlantic(elevation, azimuth).cloudless());
        self.sky.sky.light().direction
    }

    /// One ship whose starboard side faces `starboard` (level), at the origin.
    fn place(&mut self, starboard: Vec3) {
        let x = DVec3::new(f64::from(starboard.x), 0.0, f64::from(starboard.z)).normalize();
        let z = x.cross(DVec3::Y);
        self.layer.instances = vec![VesselInstance {
            model: self.model,
            transform: DMat4::from_cols(x.extend(0.0), DVec4::Y, z.extend(0.0), DVec4::W),
            key: 1,
            lamps: true,
        }];
    }

    /// Renders a few frames from `camera` (the sky's tables settle) and reads the last back.
    fn render(&mut self, camera: &Camera, name: &str) -> Vec<u8> {
        for frame in 0..4 {
            self.layer.lighting.follow(&self.sky.sky, 0.0, 0.0);
            self.renderer.set_sun(Some(Sun::new(
                self.layer.lighting.light.direction,
                Vec3::ZERO,
            )));
            self.renderer
                .render(
                    FrameInput {
                        camera,
                        time: FrameTime::fixed(frame, 1.0 / 60.0),
                    },
                    &mut [&mut self.sky, &mut self.layer],
                    RenderOutput {
                        view: &self.capture.view,
                        format: CAPTURE_FORMAT,
                        width: W,
                        height: H,
                    },
                )
                .unwrap();
        }
        self.capture
            .save_png(
                &self.gpu,
                &build_dir("vessel").join(format!("test-{name}.png")),
            )
            .unwrap();
        self.capture.read_rgba(&self.gpu).unwrap()
    }
}

fn camera(eye: Vec3, target: Vec3) -> Camera {
    Camera {
        curvature: 0.0,
        ..Camera::look_at(eye, target)
    }
}

/// Where `world` lands in the capture (pixels), if in front of the camera.
fn pixel(camera: &Camera, world: Vec3) -> Option<(usize, usize)> {
    let clip: Mat4 = camera.projection(W as f32 / H as f32) * camera.view();
    let p = clip * world.extend(1.0);
    if p.w <= 0.0 {
        return None;
    }
    let ndc = p.xy() / p.w;
    let (x, y) = (
        (ndc.x * 0.5 + 0.5) * W as f32,
        (0.5 - ndc.y * 0.5) * H as f32,
    );
    ((0.0..W as f32).contains(&x) && (0.0..H as f32).contains(&y))
        .then_some((x as usize, y as usize))
}

/// The mean display brightness (0–255, the channels' mean) of a 5 × 5 window about each of `points`.
fn brightness(rgba: &[u8], camera: &Camera, points: &[Vec3]) -> f64 {
    let mut sum = 0.0;
    let mut count = 0.0;
    for &p in points {
        let (x, y) = pixel(camera, p).expect("the point is in view");
        for dy in -2i64..=2 {
            for dx in -2i64..=2 {
                let (px, py) = (
                    (x as i64 + dx).clamp(0, W as i64 - 1),
                    (y as i64 + dy).clamp(0, H as i64 - 1),
                );
                let at = (py as usize * W as usize + px as usize) * 4;
                sum +=
                    (f64::from(rgba[at]) + f64::from(rgba[at + 1]) + f64::from(rgba[at + 2])) / 3.0;
                count += 1.0;
            }
        }
    }
    sum / count
}

fn differing(a: &[u8], b: &[u8]) -> usize {
    a.chunks(4)
        .zip(b.chunks(4))
        .filter(|(p, q)| (0..3).any(|c| p[c].abs_diff(q[c]) > 6))
        .count()
}

#[test]
fn a_ship_draws_lit_from_the_suns_side_and_shadows_her_deck() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    let toward_sun = bench.sun(40.0, 90.0);
    let level = Vec3::new(toward_sun.x, 0.0, toward_sun.z).normalize();
    let ahead = level.cross(Vec3::Y).normalize() * -1.0;

    // 120 m off her beam: from the sun's side (her side to the sun), then from the other (her side in her own shade).
    bench.place(level);
    let plating = |side: f32| -> Vec<Vec3> {
        (-3..=3)
            .map(|k| level * (side * BEAM / 2.0) + Vec3::Y * 1.5 + ahead * (8.0 * k as f32))
            .collect()
    };
    let sun_side = camera(level * 120.0 + Vec3::Y * 8.0, Vec3::Y * 1.0);
    let sunlit = bench.render(&sun_side, "sun-side");
    bench.layer.drawn = false;
    let empty = bench.render(&sun_side, "sun-side-empty");
    bench.layer.drawn = true;
    let covered = differing(&sunlit, &empty);
    assert!(
        covered > (W * H) as usize / 40,
        "she covers {covered} pixels"
    );
    let shade_side = camera(level * -120.0 + Vec3::Y * 8.0, Vec3::Y * 1.0);
    let shaded = bench.render(&shade_side, "shade-side");
    let (lit, dark) = (
        brightness(&sunlit, &sun_side, &plating(1.0)),
        brightness(&shaded, &shade_side, &plating(-1.0)),
    );
    eprintln!("her side: {lit:.1} to the sun, {dark:.1} away from it");
    assert!(lit > dark * 1.4 + 10.0, "{lit} against {dark}");

    // From above: the deckhouse's shadow falls across her deck away from the sun.
    let above = camera(Vec3::Y * 90.0 + ahead * 1.0, Vec3::ZERO);
    bench.place(level);
    let shadowed = bench.render(&above, "deck");
    let deck = |across: f32| -> Vec<Vec3> {
        (-1..=1)
            .map(|k| level * across + Vec3::Y * FREEBOARD + ahead * (4.0 * k as f32))
            .collect()
    };
    let (in_sun, in_shadow) = (
        brightness(&shadowed, &above, &deck(6.0)),
        brightness(&shadowed, &above, &deck(-5.5)),
    );
    bench.layer.shadows = false;
    let unshadowed = bench.render(&above, "deck-no-shadows");
    let open = brightness(&unshadowed, &above, &deck(-5.5));
    eprintln!(
        "her deck: {in_sun:.1} in the sun, {in_shadow:.1} in the deckhouse's shadow, {open:.1} with no shadows"
    );
    assert!(in_shadow < in_sun * 0.75, "{in_shadow} against {in_sun}");
    assert!(open > in_shadow * 1.25, "{open} against {in_shadow}");
}

#[test]
fn by_night_her_masthead_lamp_shows_ahead_of_her() {
    let Some(mut bench) = Bench::new() else {
        return;
    };
    bench.sun(-25.0, 270.0);
    bench.place(Vec3::X);
    // From 600 m ahead (her bow −Z), 6 m up.
    let lamp = Vec3::new(0.0, FREEBOARD + HOUSE + 15.0, -3.7);
    let ahead = camera(Vec3::new(0.0, 6.0, -600.0), Vec3::new(0.0, 10.0, 0.0));
    let lit = bench.render(&ahead, "night-ahead");
    let glow = brightness(&lit, &ahead, &[lamp]);
    bench.layer.instances[0].lamps = false;
    let dark = brightness(&bench.render(&ahead, "night-ahead-dark"), &ahead, &[lamp]);
    eprintln!("the masthead lamp from ahead: {glow:.1}, unlit {dark:.1}");
    assert!(glow > dark + 25.0, "{glow} against {dark}");
    // From astern it is hidden: its arc ends 22.5° abaft her beams.
    bench.layer.instances[0].lamps = true;
    let astern = camera(Vec3::new(0.0, 6.0, 600.0), Vec3::new(0.0, 10.0, 0.0));
    let behind = brightness(&bench.render(&astern, "night-astern"), &astern, &[lamp]);
    assert!(behind < dark + 10.0, "{behind} from astern");
}
