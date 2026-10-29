//! The camera as the CPU side of the effects sees it: where it stands, how it is turned and what it can see,
//! in `f64` like the web client's `THREE.Camera`, so the sprites are sorted and culled for the camera they are drawn
//! with. One type for both effect crates, built two ways: the combat effects read the renderer's `CameraUniforms`
//! each frame ([`CameraView::from_uniforms`]); the water effects (`naval-water-effects`) place theirs from the
//! renderer's `Camera` in `f64` ([`CameraView::from_render`], [`CameraView::look_at`]), as three's `camera.lookAt`
//! places it. Each crate keeps its own constructor, so neither crate's culls move by a rounding.

use glam::{DMat4, DQuat, DVec3, DVec4};
use naval_render::CameraUniforms;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraView {
    pub position: DVec3,
    /// World to view (three's `matrixWorldInverse`).
    pub view: DMat4,
    /// View to clip, reversed depth (three's `projectionMatrix` with `reversedDepth`).
    pub projection: DMat4,
    pub near: f64,
    /// The vertical field of view: [`CameraView::look_at`]'s, or the projection's from [`CameraView::from_uniforms`].
    pub fov_y_degrees: f64,
    /// Viewport (drawing-buffer) height in pixels.
    pub height: f64,
    /// How the drawn world curves away from the camera (`naval_render::Camera::curvature`).
    pub curvature: f64,
}

/// A plane `normal · p + constant ≥ 0` inside, as three's `Plane`.
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub normal: DVec3,
    pub constant: f64,
}

impl Plane {
    pub fn distance(&self, p: DVec3) -> f64 {
        self.normal.dot(p) + self.constant
    }
}

impl CameraView {
    /// The renderer's camera uniform, as the combat effects' passes are drawn with it.
    pub fn from_uniforms(uniforms: &CameraUniforms) -> Self {
        let mat = |a: &[f32; 16]| DMat4::from_cols_array(&a.map(f64::from));
        let projection = mat(&uniforms.projection);
        Self {
            position: DVec3::from_array(uniforms.position.map(f64::from)),
            view: mat(&uniforms.view),
            projection,
            near: f64::from(uniforms.near),
            fov_y_degrees: (2.0 * (1.0 / projection.y_axis.y).atan()).to_degrees(),
            height: f64::from(uniforms.viewport[1]),
            curvature: f64::from(uniforms.curvature),
        }
    }

    /// A perspective camera at `eye` looking at `target` with world up +Y, as three's `camera.lookAt` places it (the
    /// water effects').
    pub fn look_at(
        eye: DVec3,
        target: DVec3,
        fov_y_degrees: f64,
        near: f64,
        far: f64,
        width: f64,
        height: f64,
    ) -> Self {
        // As naval_render::Camera::view (three's lookAt): a view straight along the up axis nudges its up vector.
        let forward = (target - eye).try_normalize().unwrap_or(DVec3::NEG_Z);
        let up = if forward.cross(DVec3::Y).length_squared() < 1e-10 {
            DVec3::new(0.0, 0.0, forward.y.signum())
        } else {
            DVec3::Y
        };
        let view = glam::dcamera::rh::view::look_to_mat4(eye, forward, up);
        let tan = (fov_y_degrees.to_radians() / 2.0).tan();
        let aspect = width / height.max(1.0);
        // three's makePerspective with reversedDepth: depth 1 at the near plane, 0 at the far plane.
        let projection = DMat4::from_cols(
            DVec4::new(1.0 / (aspect * tan), 0.0, 0.0, 0.0),
            DVec4::new(0.0, 1.0 / tan, 0.0, 0.0),
            DVec4::new(0.0, 0.0, near / (far - near), -1.0),
            DVec4::new(0.0, 0.0, far * near / (far - near), 0.0),
        );
        Self {
            position: eye,
            view,
            projection,
            near,
            fov_y_degrees,
            height,
            curvature: 0.0,
        }
    }

    /// The renderer core's camera at a drawing size, placed in `f64` as [`CameraView::look_at`] places it (the water
    /// effects').
    pub fn from_render(camera: &naval_render::Camera, width: u32, height: u32) -> Self {
        Self {
            curvature: f64::from(camera.curvature),
            ..Self::look_at(
                camera.eye.as_dvec3(),
                camera.target.as_dvec3(),
                f64::from(camera.fov_y_degrees),
                f64::from(camera.near),
                f64::from(camera.far),
                f64::from(width),
                f64::from(height),
            )
        }
    }

    /// A world point where the camera draws it (`naval_render::camera::curved`): what the frustum culls test, since
    /// an effect far off is drawn below where it stands.
    pub fn drawn(&self, p: DVec3) -> DVec3 {
        naval_render::camera::curved_f64(self.position, self.curvature, p)
    }

    /// The camera's orientation (three's `camera.quaternion`): its local −Z looks along the view.
    pub fn rotation(&self) -> DQuat {
        DQuat::from_mat4(&self.view.inverse()).normalize()
    }

    /// The camera's world axes: right (+X), up (+Y) and back (+Z, toward the viewer), three's `matrixWorld` columns.
    pub fn axes(&self) -> (DVec3, DVec3, DVec3) {
        let world = self.view.inverse();
        (
            world.x_axis.truncate().normalize(),
            world.y_axis.truncate().normalize(),
            world.z_axis.truncate().normalize(),
        )
    }

    /// A world direction in camera space (three's `applyQuaternion` of the inverse camera rotation).
    pub fn to_camera_direction(&self, v: DVec3) -> DVec3 {
        self.view.transform_vector3(v)
    }

    /// A world point in view space (camera looking down −Z).
    pub fn to_view(&self, p: DVec3) -> DVec3 {
        self.view.transform_point3(p)
    }

    /// The frustum's planes, normalised: right, left, bottom, top, near, far. The first four pass through the
    /// eye, as three's `Frustum.setFromProjectionMatrix` orders them.
    pub fn frustum(&self) -> [Plane; 6] {
        let m = self.projection * self.view;
        let row = |i: usize| DVec4::new(m.x_axis[i], m.y_axis[i], m.z_axis[i], m.w_axis[i]);
        let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
        let plane = |v: DVec4| {
            let normal = v.truncate();
            let length = normal.length().max(1e-300);
            Plane {
                normal: normal / length,
                constant: v.w / length,
            }
        };
        // Reversed depth: clip z / w runs from 1 at the near plane to 0 at the far plane.
        [
            plane(r3 - r0),
            plane(r3 + r0),
            plane(r3 + r1),
            plane(r3 - r1),
            plane(r3 - r2),
            plane(r2),
        ]
    }

    /// Whether a sphere touches the frustum (`Frustum.intersectsSphere`).
    pub fn sees_sphere(&self, center: DVec3, radius: f64) -> bool {
        self.frustum()
            .iter()
            .all(|plane| plane.distance(center) >= -radius)
    }

    /// `projectionMatrix.elements[0]` and `[5]`: the lens scale across and up.
    pub fn lens(&self) -> (f64, f64) {
        (self.projection.x_axis.x, self.projection.y_axis.y)
    }

    /// Radians per drawing-buffer pixel (`2 tan(fov/2) / height`).
    pub fn pixel_angle(&self) -> f64 {
        2.0 * (self.fov_y_degrees.to_radians() / 2.0).tan() / self.height.max(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_render::{Camera, FrameTime};

    #[test]
    fn frustum_contains_what_the_camera_sees() {
        let camera = Camera::look_at(glam::Vec3::new(0.0, 10.0, 100.0), glam::Vec3::ZERO);
        let view = CameraView::from_uniforms(&camera.uniforms(1600, 900, FrameTime::default()));
        let planes = view.frustum();
        let inside = |p: DVec3| planes.iter().all(|plane| plane.distance(p) >= 0.0);
        assert!(inside(DVec3::ZERO));
        assert!(!inside(DVec3::new(0.0, 0.0, 200.0)), "behind the camera");
        assert!(!inside(DVec3::new(500.0, 0.0, 0.0)), "far to the right");
        let (right, up, back) = view.axes();
        assert!((right - DVec3::X).length() < 1e-6);
        assert!(up.y > 0.99 && back.z > 0.99);
        assert!((view.fov_y_degrees - f64::from(camera.fov_y_degrees)).abs() < 1e-4);
    }

    #[test]
    fn a_placed_camera_sees_what_is_ahead() {
        let camera = CameraView::look_at(
            DVec3::new(0.0, 10.0, 100.0),
            DVec3::new(0.0, 10.0, 0.0),
            50.0,
            0.5,
            60000.0,
            1600.0,
            900.0,
        );
        assert!(camera.sees_sphere(DVec3::new(0.0, 10.0, 0.0), 1.0));
        assert!(!camera.sees_sphere(DVec3::new(0.0, 10.0, 200.0), 1.0));
        assert!(!camera.sees_sphere(DVec3::new(500.0, 10.0, 0.0), 1.0));
        // The camera's −Z looks along the view.
        let ahead = camera.rotation() * DVec3::NEG_Z;
        assert!((ahead - DVec3::NEG_Z).length() < 1e-12);
    }
}
