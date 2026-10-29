//! Blast waves on a real GPU, over the renderer's test scene (a ruled sea, a hull-sized box, a sky): with nothing in
//! the air the layer asks for no copy and changes no pixel; a bomb's front changes the picture only inside its own
//! sphere and lays its pale foot on the sea; and under temporal AA its motion pipeline draws and the frame repeats.

use glam::{DVec3, Mat4, Vec3};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::ShockLayer;
use naval_render::{
    Antialiasing, Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneCopies, SceneLayer,
    TestScene,
};

const WIDTH: u32 = 480;
const HEIGHT: u32 = 270;
/// The bomb: 250 kg of TNT 10 m over the sea, clear of the scene's box.
const CENTRE: DVec3 = DVec3::new(150.0, 10.0, 0.0);

struct Rig {
    gpu: Gpu,
    renderer: Renderer,
    scene: TestScene,
    layer: ShockLayer,
    capture: Capture,
}

impl Rig {
    fn new() -> Self {
        let gpu = Gpu::headless().expect("a GPU");
        let mut renderer =
            Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).unwrap();
        let scene = TestScene::new(&mut renderer.context(), 1).unwrap();
        let layer = ShockLayer::new(&mut renderer.context()).expect("the shock shader composes");
        let capture = Capture::new(&gpu, WIDTH, HEIGHT);
        Self {
            gpu,
            renderer,
            scene,
            layer,
            capture,
        }
    }

    /// The frame's pixels and the scene copies it took.
    fn render(&mut self, camera: &Camera) -> (Vec<u8>, SceneCopies) {
        let layers: &mut [&mut dyn SceneLayer] = &mut [&mut self.scene, &mut self.layer];
        let report = self
            .renderer
            .render(
                FrameInput {
                    camera,
                    time: FrameTime::fixed(1, 1.0 / 60.0),
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
        (self.capture.read_rgba(&self.gpu).unwrap(), report.copies)
    }

    /// Writes the last frame to `.build/native/combat-effects/tests/<name>.png` for a look.
    fn save(&self, name: &str) {
        let dir = native_offscreen::build_dir("combat-effects").join("tests");
        std::fs::create_dir_all(&dir).unwrap();
        self.capture
            .save_png(&self.gpu, &dir.join(format!("{name}.png")))
            .unwrap();
    }

    /// The bomb goes off and its front runs for `seconds`.
    fn burst(&mut self, seconds: f64) {
        self.layer.waves.burst(CENTRE, 250.0, 1);
        for _ in 0..(seconds * 240.0).round() as usize {
            self.layer.waves.advance(1.0 / 240.0);
        }
    }
}

/// From 380 m off and 80 m up, the bomb in the middle of the view.
fn camera() -> Camera {
    Camera::look_at(Vec3::new(150.0, 80.0, 380.0), Vec3::new(150.0, 10.0, 0.0))
}

fn luminance(pixel: &[u8]) -> f32 {
    0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2])
}

#[test]
fn nothing_in_the_air_takes_no_copy_and_changes_no_pixel() {
    let mut rig = Rig::new();
    let camera = camera();
    let (quiet, copies) = rig.render(&camera);
    assert!(!copies.contains(SceneCopies::BACKDROP_COLOR), "{copies:?}");
    assert_eq!(rig.layer.drawn(), 0);
    rig.burst(0.12);
    let (struck, copies) = rig.render(&camera);
    assert!(copies.contains(SceneCopies::BACKDROP_COLOR | SceneCopies::BACKDROP_DEPTH));
    assert_eq!(rig.layer.drawn(), 1);
    assert_ne!(struck, quiet);
    // Run out, the front is dropped and the frame is the quiet one again, to the byte.
    rig.layer.waves.advance(1.0);
    let (after, copies) = rig.render(&camera);
    assert!(!copies.contains(SceneCopies::BACKDROP_COLOR), "{copies:?}");
    assert_eq!(after, quiet);
}

#[test]
fn a_front_moves_the_picture_inside_its_sphere_and_pales_the_sea_at_its_foot() {
    let mut rig = Rig::new();
    let camera = camera();
    let (quiet, _) = rig.render(&camera);
    rig.burst(0.12);
    let (struck, _) = rig.render(&camera);
    rig.save("shock-front");
    let radius = rig.layer.waves.shocks()[0].radius as f32;
    assert!(radius > 40.0 && radius < 70.0, "{radius}");
    // Where the sphere lies on the screen: its centre, and the reach of the rays that touch it.
    let uniforms = camera.uniforms(WIDTH, HEIGHT, FrameTime::fixed(1, 1.0 / 60.0));
    let project = |point: Vec3| {
        let clip = Mat4::from_cols_array(&uniforms.view_projection) * point.extend(1.0);
        glam::Vec2::new(
            (clip.x / clip.w * 0.5 + 0.5) * WIDTH as f32,
            (0.5 - clip.y / clip.w * 0.5) * HEIGHT as f32,
        )
    };
    let centre = CENTRE.as_vec3();
    let range = (centre - camera.eye).length();
    let reach = radius * range / (range * range - radius * radius).sqrt();
    let across = (centre - camera.eye).cross(Vec3::Y).normalize();
    let middle = project(centre);
    let outline = (project(centre + across * reach) - middle).length() * 1.1 + 2.0;
    let (mut changed, mut outside, mut paler, mut brighter) = (0usize, 0usize, 0usize, 0.0f32);
    for (at, (now, before)) in struck.chunks(4).zip(quiet.chunks(4)).enumerate() {
        let moved = now.iter().zip(before).any(|(a, b)| a.abs_diff(*b) > 3);
        if !moved {
            continue;
        }
        changed += 1;
        let pixel = glam::Vec2::new(
            (at as u32 % WIDTH) as f32 + 0.5,
            (at as u32 / WIDTH) as f32 + 0.5,
        );
        outside += usize::from((pixel - middle).length() > outline);
        // Under the blast, on the sea.
        if pixel.y > middle.y + 4.0 {
            paler += 1;
            brighter += luminance(now) - luminance(before);
        }
    }
    eprintln!(
        "{changed} pixels changed, {outside} outside the sphere, {paler} on the sea by {brighter}"
    );
    assert!(changed > 300, "the front shows: {changed} pixels");
    assert_eq!(outside, 0, "nothing beyond the front moves");
    assert!(paler > 100, "its foot lies on the sea: {paler} pixels");
    assert!(
        brighter / paler as f32 > 2.0,
        "and pales it: {}",
        brighter / paler as f32
    );
}

#[test]
fn the_front_draws_under_temporal_antialiasing() {
    let mut rig = Rig::new();
    let camera = camera();
    rig.burst(0.12);
    rig.renderer.set_antialiasing(Antialiasing::Taa);
    let (first, _) = rig.render(&camera);
    assert_eq!(rig.layer.drawn(), 1);
    assert_eq!(rig.render(&camera).0, first);
}
