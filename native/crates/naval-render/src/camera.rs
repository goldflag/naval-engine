//! The camera and its uniform block. It matches the removed web game's
//! `THREE.PerspectiveCamera` under `reversedDepthBuffer: true` (the web game's `Game.ts`):
//! a vertical field of view in degrees, a finite far plane, and depth that runs
//! from 1 at the near plane to 0 at the far plane.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// `Game.ts`: `new THREE.PerspectiveCamera(52, 1, 0.5, 60000)`.
pub const DEFAULT_FOV_Y_DEGREES: f32 = 52.0;
/// The battle near plane (`Game.setInPort`: 0.5 at sea, 3 in port; `CameraRig`
/// pulls the port's in to 0.3 when reading a fitting up close).
pub const DEFAULT_NEAR: f32 = 0.5;
/// The web camera's far plane; the chart raises it for its view.
pub const DEFAULT_FAR: f32 = 60_000.0;
/// The farthest a scene draws a hull from its camera, metres: its far plane ([`DEFAULT_FAR`]). A theatre view that
/// draws the land beyond its window takes its far plane out to [`HORIZON_FAR`], but a hull past this is a few pixels at
/// most from a plane's height and under the horizon from a ship's. The theatre's scene chart bounds its stretch at a
/// drawn hull with it (naval-client `theatre::scene_chart`, step C's C7).
pub const HULL_DRAW_M: f64 = DEFAULT_FAR as f64;
/// The far plane of a theatre view that draws the land beyond its window (`naval_terrain::horizon`): from 3 km up the
/// sea's horizon is 195 km off and a 4 km peak stands above it to 420 km. Depth is reversed in 32-bit floats, so its
/// precision near the eye is the near plane's, whatever the far plane.
pub const HORIZON_FAR: f32 = 450_000.0;
/// The Earth's radius (m): the simulation's (`naval_sim::terrain::EARTH_RADIUS_M`), whose lookouts' horizon and
/// terrain sight lines the drawn world then agrees with.
pub const EARTH_RADIUS: f64 = 6_371_000.0;
/// [`Camera::curvature`] of a camera that sees the Earth's curve: 1 / (2 R), the drop of the sea below the eye's
/// tangent plane per square metre of horizontal distance.
pub const EARTH_CURVATURE: f32 = (0.5 / EARTH_RADIUS) as f32;
/// [`Camera::curvature`] of a camera that draws no curve: the charts, which are diagrams on the chart's plane.
pub const FLAT: f32 = 0.0;

/// A world point as a camera at `eye` with `curvature` draws it (`naval::camera::curved` in WGSL): lowered by the
/// drop of the Earth's surface below the eye's tangent plane, `curvature` times the square of its horizontal distance
/// from the eye. The simulation's world is a plane; only what is drawn curves, so a hull beyond the horizon stands
/// with her waterline, then her decks, behind the sea. Everything the client projects itself (labels, anchors, aim
/// marks, culling) takes the same drop, and a ray from the screen is straight among these lowered points
/// ([`flattened`] takes a point on it back to the plane).
pub fn curved(eye: Vec3, curvature: f32, world: Vec3) -> Vec3 {
    let (dx, dz) = (world.x - eye.x, world.z - eye.z);
    Vec3::new(world.x, world.y - curvature * (dx * dx + dz * dz), world.z)
}

/// [`curved`] in doubles, for the presentation's and the contract's world points.
pub fn curved_f64(eye: glam::DVec3, curvature: f64, world: glam::DVec3) -> glam::DVec3 {
    let (dx, dz) = (world.x - eye.x, world.z - eye.z);
    glam::DVec3::new(world.x, world.y - curvature * (dx * dx + dz * dz), world.z)
}

/// The inverse of [`curved_f64`]: the simulation's point for a drawn one (a point on a ray from the screen).
pub fn flattened_f64(eye: glam::DVec3, curvature: f64, drawn: glam::DVec3) -> glam::DVec3 {
    let (dx, dz) = (drawn.x - eye.x, drawn.z - eye.z);
    glam::DVec3::new(drawn.x, drawn.y + curvature * (dx * dx + dz * dz), drawn.z)
}

/// Where the camera stands and what it sees. World space is three.js's: Y up,
/// right-handed, meters, the camera looking down its own −Z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub fov_y_degrees: f32,
    pub near: f32,
    pub far: f32,
    /// How the world curves away from this camera ([`curved`]): [`EARTH_CURVATURE`] for a view of the sea,
    /// [`FLAT`] for a chart.
    pub curvature: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self::orbit(Vec3::ZERO, 35.0, 12.0, 300.0)
    }
}

impl Camera {
    /// A camera at `eye` looking at `target` with the web client's lens and planes.
    pub fn look_at(eye: Vec3, target: Vec3) -> Self {
        Self {
            eye,
            target,
            fov_y_degrees: DEFAULT_FOV_Y_DEGREES,
            near: DEFAULT_NEAR,
            far: DEFAULT_FAR,
            curvature: EARTH_CURVATURE,
        }
    }

    /// An orbit about `target` in the harness's pose convention
    /// (`scripts/browser/cameraPoses.ts`): azimuth in degrees clockwise from the
    /// bow (−Z; 0 looks back from ahead, 90 from starboard, +X), elevation in
    /// degrees above the horizon, distance in meters.
    pub fn orbit(
        target: Vec3,
        azimuth_degrees: f32,
        elevation_degrees: f32,
        distance: f32,
    ) -> Self {
        let azimuth = azimuth_degrees.to_radians();
        let elevation = elevation_degrees.to_radians();
        let across = distance * elevation.cos();
        let eye = target
            + Vec3::new(
                across * azimuth.sin(),
                distance * elevation.sin(),
                -across * azimuth.cos(),
            );
        Self::look_at(eye, target)
    }

    /// World to view. Like three's `Matrix4.lookAt`, a view straight along the
    /// up axis is nudged rather than left degenerate.
    pub fn view(&self) -> Mat4 {
        let mut forward = self.target - self.eye;
        if forward.length_squared() < 1e-12 {
            forward = Vec3::NEG_Z;
        }
        let forward = forward.normalize();
        let up = if forward.cross(Vec3::Y).length_squared() < 1e-10 {
            // three nudges the eye 1e-4 toward +Z, so looking down the bow (−Z) is
            // at the top of the frame and looking up the stern is.
            Vec3::new(0.0, 0.0, forward.y.signum())
        } else {
            Vec3::Y
        };
        glam::camera::rh::view::look_to_mat4(self.eye, forward, up)
    }

    /// View to clip with reversed depth; see [`reversed_perspective`].
    pub fn projection(&self, aspect: f32) -> Mat4 {
        reversed_perspective(self.fov_y_degrees.to_radians(), aspect, self.near, self.far)
    }

    /// The uniform block for this camera over a `width` × `height` viewport at `time`.
    pub fn uniforms(&self, width: u32, height: u32, time: FrameTime) -> CameraUniforms {
        self.uniforms_jittered(width, height, time, glam::Vec2::ZERO)
    }

    /// The uniform block with the projection shifted by `jitter` pixels (temporal
    /// anti-aliasing's sub-pixel offset), as three's `setViewOffset(w, h, x, y, w, h)`.
    pub fn uniforms_jittered(
        &self,
        width: u32,
        height: u32,
        time: FrameTime,
        jitter: glam::Vec2,
    ) -> CameraUniforms {
        let (w, h) = (width.max(1) as f32, height.max(1) as f32);
        let aspect = w / h;
        let unjittered = self.projection(aspect);
        let projection = jitter_projection(unjittered, jitter, w, h);
        let mut uniforms = CameraUniforms::from_matrices(
            self.view(),
            projection,
            self.eye,
            (width, height),
            (self.near, self.far),
            time,
        );
        uniforms.tan_half_fov_y = (self.fov_y_degrees.to_radians() * 0.5).tan();
        uniforms.curvature = self.curvature;
        uniforms
    }

    /// `world` as this camera draws it: [`curved`] from its eye.
    pub fn curved(&self, world: Vec3) -> Vec3 {
        curved(self.eye, self.curvature, world)
    }

    /// This camera turned onto where it draws its target: a camera placed to look at a point of the simulation's
    /// world (a harness's eye and target, an orbit about a hull seen from far off) has it in the middle of the frame.
    pub fn on_drawn_target(mut self) -> Self {
        self.target = self.curved(self.target);
        self
    }
}

/// `projection` shifted by `jitter` pixels over a `width` × `height` viewport:
/// three's `setViewOffset(width, height, x, y, width, height)` moves the frustum
/// window right by x and down by y pixels, so the image moves left and up.
pub fn jitter_projection(projection: Mat4, jitter: glam::Vec2, width: f32, height: f32) -> Mat4 {
    if jitter == glam::Vec2::ZERO {
        return projection;
    }
    let shift = Mat4::from_translation(Vec3::new(
        -2.0 * jitter.x / width,
        2.0 * jitter.y / height,
        0.0,
    ));
    shift * projection
}

/// three's `Matrix4.makeOrthographic(..., WebGPUCoordinateSystem, reversedDepth = true)`:
/// clip depth 1 at `near` and 0 at `far`, as the sun's shadow cameras use.
pub fn reversed_orthographic(
    left: f32,
    right: f32,
    bottom: f32,
    top: f32,
    near: f32,
    far: f32,
) -> Mat4 {
    let w = right - left;
    let h = top - bottom;
    let d = far - near;
    Mat4::from_cols_array(&[
        2.0 / w,
        0.0,
        0.0,
        0.0, //
        0.0,
        2.0 / h,
        0.0,
        0.0, //
        0.0,
        0.0,
        1.0 / d,
        0.0, //
        -(right + left) / w,
        -(top + bottom) / h,
        far / d,
        1.0,
    ])
}

/// three's `Matrix4.makePerspective(..., WebGPUCoordinateSystem, reversedDepth = true)`
/// for a symmetric frustum: clip depth is 1 at `near` and 0 at `far`, so depth
/// is cleared to 0 and tested `GreaterEqual`.
pub fn reversed_perspective(fov_y_radians: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let y = 1.0 / (fov_y_radians * 0.5).tan();
    let x = y / aspect;
    let c = near / (far - near);
    let d = far * near / (far - near);
    Mat4::from_cols_array(&[
        x, 0.0, 0.0, 0.0, //
        0.0, y, 0.0, 0.0, //
        0.0, 0.0, c, -1.0, //
        0.0, 0.0, d, 0.0,
    ])
}

/// The manual clock a frame is drawn at. Harnesses step it by a fixed `dt`, so
/// a frame depends only on its index, never on wall time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTime {
    /// Seconds since the scene started.
    pub seconds: f64,
    /// Seconds since the previous frame.
    pub dt: f32,
    /// Frames since the scene started.
    pub frame: u64,
}

impl FrameTime {
    /// Frame `frame` of a clock stepping `dt` seconds: `seconds = frame × dt`,
    /// computed from the index so batching steps never drifts the clock.
    pub fn fixed(frame: u64, dt: f64) -> Self {
        Self {
            seconds: frame as f64 * dt,
            dt: dt as f32,
            frame,
        }
    }
}

/// The camera uniform block, bound at group 0 binding 0 of every scene
/// pipeline. Its WGSL twin is `Camera` in `shaders/naval/camera.wgsl`; keep the
/// two in the same order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniforms {
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub view_projection: [f32; 16],
    pub inverse_view: [f32; 16],
    pub inverse_projection: [f32; 16],
    pub inverse_view_projection: [f32; 16],
    /// Camera position in world meters.
    pub position: [f32; 3],
    /// Seconds since the scene started (f32: about 0.25 ms resolution after an hour).
    pub time: f32,
    /// Width, height, 1/width, 1/height in pixels.
    pub viewport: [f32; 4],
    pub near: f32,
    pub far: f32,
    pub tan_half_fov_y: f32,
    pub aspect: f32,
    pub dt: f32,
    pub frame: u32,
    /// [`Camera::curvature`]: 0 for a camera built from matrices (the sun's shadow cameras see the plane).
    pub curvature: f32,
    pub _pad: u32,
}

const _: () = assert!(std::mem::size_of::<CameraUniforms>() == 448);

/// Group 0 binding 5 of scene pipelines, beside the camera: what motion vectors need
/// (`naval::motion`). Its WGSL twin is `CameraMotion` in `shaders/naval/motion.wgsl`.
/// Kept out of [`CameraUniforms`], whose layout other crates build by hand.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraMotion {
    /// This frame's view-projection without temporal anti-aliasing's jitter.
    pub unjittered_view_projection: [f32; 16],
    /// The previous frame's unjittered view-projection (this frame's when there is no
    /// history, and always without temporal AA).
    pub previous_view_projection: [f32; 16],
    /// The sub-pixel offset the camera's projection carries, in pixels (0 without temporal AA).
    pub jitter: [f32; 2],
    pub padding: [f32; 2],
}

const _: () = assert!(std::mem::size_of::<CameraMotion>() == 144);

impl CameraUniforms {
    /// A block from explicit matrices (the sun's orthographic shadow cameras, a
    /// jittered projection). `tan_half_fov_y` and `curvature` are left 0 and `aspect` is the
    /// viewport's; set them for a perspective lens.
    pub fn from_matrices(
        view: Mat4,
        projection: Mat4,
        position: Vec3,
        viewport: (u32, u32),
        (near, far): (f32, f32),
        time: FrameTime,
    ) -> Self {
        let width = viewport.0.max(1) as f32;
        let height = viewport.1.max(1) as f32;
        let view_projection = projection * view;
        Self {
            view: view.to_cols_array(),
            projection: projection.to_cols_array(),
            view_projection: view_projection.to_cols_array(),
            inverse_view: view.inverse().to_cols_array(),
            inverse_projection: projection.inverse().to_cols_array(),
            inverse_view_projection: view_projection.inverse().to_cols_array(),
            position: position.to_array(),
            time: time.seconds as f32,
            viewport: [width, height, 1.0 / width, 1.0 / height],
            near,
            far,
            tan_half_fov_y: 0.0,
            aspect: width / height,
            dt: time.dt,
            frame: time.frame as u32,
            curvature: FLAT,
            _pad: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec4;

    /// Degrees a drawn point stands below the eye's level.
    fn dip(eye: glam::DVec3, drawn: glam::DVec3) -> f64 {
        let across = (drawn.x - eye.x).hypot(drawn.z - eye.z);
        ((eye.y - drawn.y) / across).atan().to_degrees()
    }

    #[test]
    fn the_drawn_sea_has_the_earths_horizon_and_a_mast_clears_it_where_the_lookouts_rule_says() {
        let k = f64::from(EARTH_CURVATURE);
        let eye = glam::DVec3::new(100.0, 25.0, -40.0);
        let sea = |d: f64| curved_f64(eye, k, glam::DVec3::new(100.0 + d, 0.0, -40.0));
        // The sea's highest line is where a sphere's horizon lies, √(2 R h) off, at the dip √(2 h / R).
        let horizon = (2.0 * EARTH_RADIUS * eye.y).sqrt();
        let lowest = (0..600)
            .map(|i| f64::from(i) * 100.0 + 100.0)
            .min_by(|a, b| dip(eye, sea(*a)).total_cmp(&dip(eye, sea(*b))))
            .unwrap();
        assert!((lowest - horizon).abs() <= 100.0, "{lowest} {horizon}");
        let horizon_dip = (2.0 * eye.y / EARTH_RADIUS).sqrt().to_degrees();
        assert!((dip(eye, sea(horizon)) - horizon_dip).abs() < 1e-4);
        // `naval-sim`'s lookouts see a feature `top` metres up as far as 3570 (√eye + √top) metres: there its top is
        // drawn on the sea's horizon, above it nearer and behind the sea farther off.
        let top = 36.0_f64;
        let reach = 3570.0 * (eye.y.sqrt() + top.sqrt());
        let mast = |d: f64| curved_f64(eye, k, glam::DVec3::new(100.0 + d, top, -40.0));
        assert!((dip(eye, mast(reach)) - horizon_dip).abs() < 2e-4);
        assert!(dip(eye, mast(reach - 2000.0)) < horizon_dip);
        assert!(dip(eye, mast(reach + 2000.0)) > horizon_dip);
        // A flat camera draws a point where it stands, and a drawn point goes back to the plane.
        let p = glam::DVec3::new(9000.0, 12.0, -22_000.0);
        assert_eq!(curved_f64(eye, f64::from(FLAT), p), p);
        assert!(flattened_f64(eye, k, curved_f64(eye, k, p)).distance(p) < 1e-9);
        assert!((p.y - curved_f64(eye, k, p).y - 44.06).abs() < 0.1);
    }

    #[test]
    fn a_camera_carries_its_curvature_to_the_shaders_and_a_shadow_camera_none() {
        let camera = Camera::look_at(Vec3::new(0.0, 30.0, 0.0), Vec3::new(0.0, 0.0, -1000.0));
        assert_eq!(camera.curvature, EARTH_CURVATURE);
        assert_eq!(
            camera.uniforms(1600, 900, FrameTime::default()).curvature,
            EARTH_CURVATURE
        );
        let light = CameraUniforms::from_matrices(
            Mat4::IDENTITY,
            Mat4::IDENTITY,
            Vec3::ZERO,
            (64, 64),
            (1.0, 100.0),
            FrameTime::default(),
        );
        assert_eq!(light.curvature, FLAT);
        // Turned onto its drawn target, the target is in the middle of the frame.
        let far = Camera::look_at(Vec3::new(0.0, 30.0, 0.0), Vec3::new(0.0, 10.0, -30_000.0))
            .on_drawn_target();
        let drawn = far.curved(Vec3::new(0.0, 10.0, -30_000.0));
        let clip = far.projection(1.6) * far.view() * drawn.extend(1.0);
        assert!((clip.y / clip.w).abs() < 1e-4);
        assert!(drawn.y < -50.0);
    }

    fn depth_at(camera: &Camera, distance: f32) -> f32 {
        let clip = camera.projection(16.0 / 9.0) * Vec4::new(0.0, 0.0, -distance, 1.0);
        clip.z / clip.w
    }

    #[test]
    fn depth_is_reversed_one_at_near_zero_at_far() {
        let camera = Camera::default();
        assert!((depth_at(&camera, DEFAULT_NEAR) - 1.0).abs() < 1e-6);
        assert!(depth_at(&camera, DEFAULT_FAR).abs() < 1e-6);
        // Nearer is greater, which is why the depth test is GreaterEqual.
        assert!(depth_at(&camera, 100.0) > depth_at(&camera, 200.0));
    }

    #[test]
    fn matches_three_make_perspective() {
        // three: top = near·tan(fov/2); x = 2·near/(right−left); c = near/(far−near); d = far·near/(far−near).
        let (fov, aspect, near, far) = (52f32, 1.5f32, 0.5f32, 60_000f32);
        let top = near * (fov.to_radians() / 2.0).tan();
        let width = aspect * 2.0 * top;
        let m = reversed_perspective(fov.to_radians(), aspect, near, far).to_cols_array();
        assert!((m[0] - 2.0 * near / width).abs() < 1e-5);
        assert!((m[5] - near / top).abs() < 1e-5);
        assert!((m[10] - near / (far - near)).abs() < 1e-9);
        assert!((m[14] - far * near / (far - near)).abs() < 1e-5);
        assert_eq!(m[11], -1.0);
    }

    #[test]
    fn orbit_follows_the_pose_convention() {
        // Azimuth 90 stands to starboard (+X), level with the target.
        let camera = Camera::orbit(Vec3::ZERO, 90.0, 0.0, 100.0);
        assert!((camera.eye - Vec3::new(100.0, 0.0, 0.0)).length() < 1e-3);
        // Azimuth 0 stands ahead of the bow, which points down −Z.
        let camera = Camera::orbit(Vec3::ZERO, 0.0, 0.0, 100.0);
        assert!((camera.eye - Vec3::new(0.0, 0.0, -100.0)).length() < 1e-3);
    }

    #[test]
    fn looking_straight_down_is_not_degenerate() {
        let camera = Camera::look_at(Vec3::new(0.0, 100.0, 0.0), Vec3::ZERO);
        assert!(camera.view().is_finite());
        let uniforms = camera.uniforms(64, 64, FrameTime::default());
        assert!(
            uniforms
                .inverse_view_projection
                .iter()
                .all(|v| v.is_finite())
        );
    }
}
