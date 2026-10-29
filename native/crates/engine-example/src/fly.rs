//! The fly camera: WASD along the view and across it, Q and E down and up, the mouse (a button held) to look, Shift
//! eight times as fast, the wheel to change the speed.

use std::collections::HashSet;

use glam::Vec3;
use naval_render::Camera;
use winit::keyboard::KeyCode;

/// A camera that flies where it looks: its eye, its heading and pitch (radians; heading 0 looks north, −Z, and grows
/// toward the east, +X), and its speed (m/s).
#[derive(Clone, Copy, Debug)]
pub struct Fly {
    pub eye: Vec3,
    pub heading: f32,
    pub pitch: f32,
    pub speed: f32,
}

impl Fly {
    /// A camera at `eye` looking at `target`.
    pub fn looking_at(eye: Vec3, target: Vec3) -> Self {
        let d = (target - eye).normalize_or_zero();
        Self {
            eye,
            heading: d.x.atan2(-d.z),
            pitch: d.y.clamp(-1.0, 1.0).asin(),
            speed: 40.0,
        }
    }

    /// The unit vector it looks along.
    pub fn forward(&self) -> Vec3 {
        let (sh, ch) = self.heading.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(sh * cp, sp, -ch * cp)
    }

    /// Turn by a mouse movement of `dx`, `dy` pixels.
    pub fn look(&mut self, dx: f64, dy: f64) {
        const RADIANS_PER_PIXEL: f32 = 0.003;
        self.heading += dx as f32 * RADIANS_PER_PIXEL;
        self.pitch = (self.pitch - dy as f32 * RADIANS_PER_PIXEL).clamp(-1.55, 1.55);
    }

    /// Fly for `dt` seconds with `held` keys down, the eye kept a metre over the sea.
    pub fn fly(&mut self, held: &HashSet<KeyCode>, dt: f32) {
        let forward = self.forward();
        let right = forward.cross(Vec3::Y).normalize_or_zero();
        let key = |k: KeyCode| if held.contains(&k) { 1.0 } else { 0.0 };
        let along = key(KeyCode::KeyW) - key(KeyCode::KeyS);
        let across = key(KeyCode::KeyD) - key(KeyCode::KeyA);
        let up = key(KeyCode::KeyE) - key(KeyCode::KeyQ);
        let fast = if held.contains(&KeyCode::ShiftLeft) || held.contains(&KeyCode::ShiftRight) {
            8.0
        } else {
            1.0
        };
        let step = forward * along + right * across + Vec3::Y * up;
        self.eye += step.normalize_or_zero() * self.speed * fast * dt;
        self.eye.y = self.eye.y.max(1.0);
    }

    /// The renderer's camera for this view.
    pub fn camera(&self) -> Camera {
        Camera {
            eye: self.eye,
            target: self.eye + self.forward() * 100.0,
            fov_y_degrees: 50.0,
            near: 0.5,
            far: naval_render::camera::DEFAULT_FAR,
            curvature: naval_render::camera::EARTH_CURVATURE,
        }
    }
}
