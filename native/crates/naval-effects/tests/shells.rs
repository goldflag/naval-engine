//! Shells on a real GPU: every shell pipeline builds (motion included, under temporal AA), a salvo of tracers in
//! flight draws its red sparks, streaks and trails over a black scene while a heavy salvo draws its trails and nothing
//! that glows, a tracer seen from behind burns in its base, a thin trail stays one unbroken line (the coverage floor),
//! and the player's heavy salvo carries small orange markers.

use glam::{DVec3, Vec3};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::{ShellLayer, ShellState};
use naval_render::{
    Antialiasing, Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    layer: ShellLayer,
    capture: Capture,
}

impl Rig {
    fn new() -> Self {
        let gpu = Gpu::headless().expect("a GPU");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let layer = ShellLayer::new(&mut renderer.context()).expect("the shell shaders compose");
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        Self {
            gpu,
            renderer,
            layer,
            capture,
        }
    }

    fn render(&mut self, camera: &Camera, frame: u64) -> Vec<u8> {
        let layers: &mut [&mut dyn SceneLayer] = &mut [&mut self.layer];
        self.renderer
            .render(
                FrameInput {
                    camera,
                    time: FrameTime::fixed(frame, 1.0 / 60.0),
                },
                layers,
                RenderOutput {
                    view: &self.capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .unwrap();
        self.capture.read_rgba(&self.gpu).unwrap()
    }

    /// Writes the last frame to `.build/native/aa-tracers/tests/<name>.png` for a look.
    fn save(&self, name: &str) {
        let dir = native_offscreen::build_dir("aa-tracers").join("tests");
        std::fs::create_dir_all(&dir).unwrap();
        self.capture
            .save_png(&self.gpu, &dir.join(format!("{name}.png")))
            .unwrap();
    }
}

/// Three rounds of `caliber` (m) flying +X at 800 m/s, 40 m apart, stepped `seconds` at 60 Hz; `helm`: fired by the
/// ship the player commands.
fn salvo(layer: &mut ShellLayer, caliber: f64, seconds: f64, helm: bool) {
    let steps = (seconds * 60.0).round() as usize;
    for step in 1..=steps {
        let age = step as f64 / 60.0;
        let shells: Vec<ShellState> = (0..3)
            .map(|i| ShellState {
                id: i,
                owner: 1,
                position: DVec3::new(800.0 * age, 30.0 + 60.0 * age, f64::from(i as i32) * 40.0),
                velocity: DVec3::new(800.0, 60.0, 0.0),
                caliber_m: caliber,
                age,
                airborne: true,
                helm,
            })
            .collect();
        layer.visuals.update(&shells, 1.0 / 60.0);
    }
}

/// A destroyer's 5-inch, which carries a tracer, and a battleship's 38 cm, which does not.
const TRACER: f64 = 0.127;
const HEAVY: f64 = 0.38;

fn lit(pixels: &[u8], level: u8) -> usize {
    pixels
        .chunks(4)
        .filter(|p| p[0].max(p[1]).max(p[2]) > level)
        .count()
}

/// Pixels burning red: bright, and far redder than they are green.
fn red(pixels: &[u8]) -> usize {
    pixels
        .chunks(4)
        .filter(|p| p[0] > 120 && f32::from(p[0]) > 1.6 * f32::from(p[1]))
        .count()
}

#[test]
fn a_salvo_of_tracers_draws_red_sparks_streaks_and_trails() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, TRACER, 1.0, false);
    // From above and aside, 120 m off the heads.
    let camera = Camera::look_at(Vec3::new(740.0, 120.0, 130.0), Vec3::new(780.0, 88.0, 40.0));
    let pixels = rig.render(&camera, 1);
    rig.save("salvo");
    let stats = rig.layer.stats();
    assert_eq!(stats.glows, 3);
    assert_eq!(stats.streaks, 3);
    assert!(stats.trail_segments >= 6, "{stats:?}");
    let (faint, burning) = (lit(&pixels, 6), red(&pixels));
    eprintln!("{faint} faint, {burning} burning red pixels");
    assert!(burning >= 3, "the sparks burn red: {burning} pixels");
    assert!(
        faint > burning + 40,
        "the trails show behind them: {faint} pixels"
    );
    // Under temporal AA the motion pipelines draw too, and the frame renders again the same.
    rig.renderer.set_antialiasing(Antialiasing::Taa);
    let first = rig.render(&camera, 2);
    assert_eq!(rig.render(&camera, 2), first);
}

#[test]
fn a_heavy_salvo_draws_trails_but_nothing_that_glows() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, HEAVY, 1.0, false);
    let camera = Camera::look_at(Vec3::new(700.0, 200.0, 260.0), Vec3::new(650.0, 80.0, 40.0));
    let pixels = rig.render(&camera, 1);
    rig.save("heavy-salvo");
    let stats = rig.layer.stats();
    assert_eq!((stats.glows, stats.streaks), (0, 0), "{stats:?}");
    assert!(stats.trail_segments >= 6, "{stats:?}");
    assert!(lit(&pixels, 6) > 40, "the trails show");
    assert_eq!(red(&pixels), 0, "no round glows");
}

#[test]
fn a_tracer_seen_from_behind_burns_in_its_base() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, TRACER, 0.5, false);
    let head = Vec3::new(400.0, 60.0, 0.0);
    // A metre and a half behind the round and a little aside, looking up its flight.
    let camera = Camera::look_at(head + Vec3::new(-1.5, 0.2, 0.4), head);
    let pixels = rig.render(&camera, 1);
    rig.save("behind");
    assert!(rig.layer.stats().detailed >= 1, "{:?}", rig.layer.stats());
    let burning = red(&pixels);
    eprintln!("{burning} burning red pixels");
    assert!(burning > 200, "the base burns: {burning} pixels");
}

#[test]
fn a_heavy_round_seen_close_is_cold_steel() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, HEAVY, 0.5, false);
    let head = Vec3::new(400.0, 60.0, 0.0);
    let camera = Camera::look_at(head + Vec3::new(-0.2, 0.6, 3.0), head);
    let pixels = rig.render(&camera, 1);
    rig.save("close");
    assert!(rig.layer.stats().detailed >= 1, "{:?}", rig.layer.stats());
    let (body, burning) = (lit(&pixels, 12), red(&pixels));
    eprintln!("{body} body pixels, {burning} burning red");
    assert!(
        body > 1500,
        "the body fills a good part of the view: {body} pixels"
    );
    assert!(
        burning < body / 50,
        "and does not glow: {burning} red of {body}"
    );
}

#[test]
fn a_thin_trail_is_one_unbroken_line() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, TRACER, 1.0, false);
    // Behind the heads, 80 to 160 m off: a 5-inch round's trail is under two pixels wide (the floor) on these rows, and
    // faint at its old end, in the display transform's toe, where a line split over two pixel rows shows darker than
    // on one.
    let camera = Camera::look_at(Vec3::new(740.0, 80.0, 160.0), Vec3::new(740.0, 70.0, 0.0));
    let pixels = rig.render(&camera, 1);
    rig.save("distant");
    // Each column the trail crosses has a lit pixel: no gaps along it. (Without the floor, most would be dark.)
    let columns: Vec<bool> = (0..WIDTH)
        .map(|x| (0..HEIGHT).any(|y| pixels[((y * WIDTH + x) * 4) as usize] > 1))
        .collect();
    let first = columns.iter().position(|&c| c).expect("something drew");
    let last = columns.iter().rposition(|&c| c).unwrap();
    let gaps = columns[first..=last].iter().filter(|&&c| !c).count();
    eprintln!("columns {first} … {last}, {gaps} gaps");
    assert!(
        last - first > 20,
        "the trail spans {} columns",
        last - first
    );
    assert_eq!(gaps, 0, "no column between the first and last is dark");
}

#[test]
fn the_helms_heavy_salvo_shows_small_orange_dots_far_off() {
    let mut rig = Rig::new();
    salvo(&mut rig.layer, HEAVY, 1.0, true);
    // 2 km behind the heads, the salvo spread across the view; the rounds themselves are well under a pixel.
    let camera = Camera::look_at(
        Vec3::new(-1200.0, 150.0, 40.0),
        Vec3::new(800.0, 90.0, 40.0),
    );
    let pixels = rig.render(&camera, 1);
    rig.save("markers");
    let stats = rig.layer.stats();
    assert_eq!((stats.markers, stats.glows), (3, 0), "{stats:?}");
    // Orange, not the trails' pale grey.
    let orange = pixels
        .chunks(4)
        .filter(|p| p[0] > 20 && f32::from(p[0]) > 1.3 * f32::from(p[1]) && p[1] > p[2])
        .count();
    eprintln!("{orange} orange pixels");
    assert!(
        (3..=60).contains(&orange),
        "three dots a few pixels across: {orange} pixels"
    );
    // Under temporal AA the motion pipeline draws too.
    rig.renderer.set_antialiasing(Antialiasing::Taa);
    let first = rig.render(&camera, 2);
    assert_eq!(rig.render(&camera, 2), first);
}
