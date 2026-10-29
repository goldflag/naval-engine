//! The sun's glare in the eye (`native/docs/render-core/post.md`, "The sun's glare"): looking toward the sun, the light
//! the eye's own cornea, lens and fundus scatter lays a veil about it, steep at its edge and reaching tens of degrees.
//! The disc the sky draws is dimmed for the display and stands for none of that, so without it the sun is a yellow
//! circle one can look straight at.
//!
//! - **The veil** is the CIE's general disability-glare function for a young eye (Vos 1999): a veiling luminance of
//!   `E (10 / θ³ + 5 / θ²)` cd/m² at θ degrees from a source giving the eye `E` lux, so about 7 % of the sun's light
//!   comes back as veil. [`CORE`] and [`TAIL`] are its two terms per radian, [`veil`] their sum; `E` is the sun's own
//!   irradiance through the air ([`FlareSun::irradiance`]: not the readability floor the world's light keeps as it sets,
//!   so a low sun in thick air glares as dimly as it shines). The game shows [`SHARE`] of it: in full it is 2.5 times the
//!   sun's irradiance 2° off and 0.06 of it at 10°, which is true to the eye and too much of a display.
//! - **What is seen of the sun** is measured where its drawn disc falls in the frame (`shaders/post/sun_glare.wgsl`,
//!   as photo mode's lens does it): the disc's radiance over the sky about it against the radiance it has unobstructed,
//!   so a cloud, smoke, a hull or the horizon in front dims the glare by what it hides. Eased over [`EASE_S`], so rigging
//!   crossing the disc does not flicker; with the clock stopped (a pause, photo mode) it settles at once, so the glare
//!   still follows a camera that moves.
//! - **Out of the frame** the disc cannot be measured: the glare keeps what was last seen of it and fades out over
//!   [`MARGIN`] of the view's height past the frame's edge. A capture, which settles at once, takes none from a sun
//!   it cannot measure, so a still does not depend on the frames before it.
//!
//! The bloom's composite adds it (`sun_glare` in `shaders/post/bloom.wgsl`), so it follows the bloom's setting, the
//! eye's exposure and a look's halation as the bloom does.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use crate::camera::Camera;
use crate::photo_optics::FlareSun;
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::compute_recipe;
use crate::resources::Resources;
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;
use crate::upload::Uploads;
use crate::{SCENE_DEPTH, SCENE_HDR};

pub const SUN_GLARE_SHADER: ShaderFile = crate::shader_file!("shaders/post/sun_glare.wgsl");

/// Registry name of the glare's state buffer (`array<f32, 4>`: the share of the sun's disc seen, eased; this frame's
/// measure; its samples inside the frame; unused), `STORAGE | COPY_SRC`.
pub const SUN_GLARE_BUFFER: &str = "exposure.sun_glare";

/// The veil's steep term: `10 / θ³` with θ in degrees, per radian cubed (`10 (π / 180)³`).
pub const CORE: f32 = 5.3165e-5;
/// The veil's wide term: `5 / θ²` with θ in degrees, per radian squared (`5 (π / 180)²`).
pub const TAIL: f32 = 1.5231e-3;
/// The share of the eye's veil the game shows.
pub const SHARE: f32 = 0.5;
/// The time constant (s) what is seen of the disc eases with.
pub const EASE_S: f32 = 0.12;
/// A sun this far past the frame's edge (shares of the view's height) glares no more.
pub const MARGIN: f32 = 0.3;

/// The veil's radiance per unit of the sun's irradiance, `angle` radians from its centre (`sun_veil` in bloom.wgsl,
/// which holds it flat across the drawn disc).
pub fn veil(angle: f32) -> f32 {
    let angle = angle.max(1e-4);
    CORE / (angle * angle * angle) + TAIL / (angle * angle)
}

/// What is left of a sun's glare with its centre `outside` of the frame (shares of the view's height past the nearest
/// edge; 0 inside).
pub fn frame_fade(outside: f32) -> f32 {
    let t = (outside / MARGIN).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
}

/// The sun's terms in the bloom's uniforms (`BloomParams::sun`, `sun_light` and `sun_view` in bloom.wgsl).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct SunTerms {
    /// Its direction in the camera's frame (looking down −Z), and its drawn disc's angular radius (radians).
    pub sun: [f32; 4],
    /// The light of its veil (its irradiance times [`SHARE`] and the fade past the frame's edge), and 1 with a sun.
    pub light: [f32; 4],
    /// A pixel's ray: the tangent of half the view across and up.
    pub view: [f32; 4],
}

/// The sun before `camera`, drawing at `aspect`: the bloom's terms, its light faded by how far its centre stands past
/// the frame's edge (none behind the camera).
pub(crate) fn place(sun: &FlareSun, camera: &Camera, aspect: f32) -> SunTerms {
    let d = camera
        .view()
        .transform_vector3(sun.direction.normalize_or_zero());
    let tan = (camera.fov_y_degrees.to_radians() * 0.5).tan().max(1e-6);
    let fade = if d.z < -1e-4 {
        // On the image plane, in units of its half-height.
        let (x, y) = (d.x / -d.z / tan, d.y / -d.z / tan);
        frame_fade((x.abs() - aspect).max(y.abs() - 1.0).max(0.0) * 0.5)
    } else {
        0.0
    };
    let light = sun.irradiance.max(Vec3::ZERO) * (SHARE * fade);
    SunTerms {
        sun: [d.x, d.y, d.z, sun.drawn_radius],
        light: [light.x, light.y, light.z, 1.0],
        view: [tan * aspect, tan, 0.0, 0.0],
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MeasureUniforms {
    /// The sun's direction in the camera's frame, and its drawn disc's angular radius ([`SunTerms::sun`]).
    sun: [f32; 4],
    /// The tangent of half the view across and up, a pixel's angle at the frame's centre, 1 with a sun.
    view: [f32; 4],
    /// The drawn disc's unobstructed luminance, the share of the way the state moves to this frame's measure, 1 for a
    /// capture, 1 where the scene's alpha holds the transparents' cover.
    terms: [f32; 4],
}

/// The measure of the sun's disc in the frame: one small compute pass ahead of the bloom, a state buffer its composite
/// reads.
pub(crate) struct SunGlare {
    /// The sun the eye glares at, when it is up ([`crate::frame::Renderer::set_sun_glare`]).
    pub sun: Option<FlareSun>,
    measure: PipelineId,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// By `scene.hdr`'s and `scene.depth`'s versions.
    group: Option<((u64, u64), wgpu::BindGroup)>,
    /// The next measure is a capture's: it settles at once and takes nothing from the frames before it.
    snap: bool,
}

impl SunGlare {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        resources: &mut Resources,
    ) -> Result<Self, ShaderError> {
        library.add(SUN_GLARE_SHADER)?;
        let visibility = wgpu::ShaderStages::COMPUTE;
        let texture = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let buffer = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun glare"),
            entries: &[
                texture(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture(1, wgpu::TextureSampleType::Depth),
                buffer(2, wgpu::BufferBindingType::Uniform),
                buffer(3, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let measure = pipelines.add(
            device,
            library,
            "sun glare measure",
            SUN_GLARE_SHADER.path,
            Vec::new(),
            compute_recipe(vec![layout.clone()], "measure"),
        )?;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun glare"),
            size: std::mem::size_of::<MeasureUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        resources.create_buffer(
            device,
            SUN_GLARE_BUFFER,
            16,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            crate::Lifetime::Persistent,
        );
        Ok(Self {
            sun: None,
            measure,
            layout,
            uniforms,
            group: None,
            snap: false,
        })
    }

    pub(crate) fn snap(&mut self) {
        self.snap = true;
    }

    /// Measures the sun's disc in `scene.hdr` as `camera` draws it and steps what is seen of it by `dt` (at once with
    /// the clock stopped: a pause, photo mode, a frame drawn again); the bloom's terms for this frame (none without a
    /// sun). `cover`: the scene's alpha holds what the transparents cover of each pixel (the night grade's cover pass
    /// ran), and a sample under more than half of it shows no sky.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        camera: &Camera,
        dt: f32,
        cover: bool,
    ) -> SunTerms {
        let viewport = resources.viewport();
        let aspect = viewport.0.max(1) as f32 / viewport.1.max(1) as f32;
        let sun = self.sun.filter(|sun| {
            sun.irradiance.max_element() > 0.0 && sun.disc_radiance.max_element() > 0.0
        });
        let placed = sun.map(|sun| (sun, place(&sun, camera, aspect)));
        let snap = std::mem::take(&mut self.snap);
        let rate = if snap || dt <= 0.0 {
            1.0
        } else {
            1.0 - (-dt / EASE_S).exp()
        };
        let uniforms = match placed {
            Some((sun, terms)) => MeasureUniforms {
                sun: terms.sun,
                view: [
                    terms.view[0],
                    terms.view[1],
                    2.0 * terms.view[1] / viewport.1.max(1) as f32,
                    1.0,
                ],
                terms: [
                    sun.disc_radiance.dot(Vec3::new(0.2126, 0.7152, 0.0722)),
                    rate,
                    f32::from(u8::from(snap)),
                    f32::from(u8::from(cover)),
                ],
            },
            None => MeasureUniforms::zeroed(),
        };
        uploads.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let hdr = resources.texture(SCENE_HDR).expect("registered");
        let depth = resources.texture(SCENE_DEPTH).expect("registered");
        let versions = (hdr.version, depth.version);
        if self.group.as_ref().is_none_or(|(v, _)| *v != versions) {
            let state = resources.buffer(SUN_GLARE_BUFFER).expect("registered");
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("sun glare"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&hdr.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&depth.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: state.buffer.as_entire_binding(),
                    },
                ],
            });
            self.group = Some((versions, group));
        }
        let (_, group) = self.group.as_ref().expect("just made");
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sun glare"),
            timestamp_writes: timer.compute_pass("sun glare"),
        });
        pass.set_bind_group(0, group, &[]);
        pass.set_pipeline(pipelines.compute(self.measure));
        pass.dispatch_workgroups(1, 1, 1);
        placed.map_or(SunTerms::default(), |(_, terms)| terms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun(direction: Vec3) -> FlareSun {
        FlareSun {
            direction,
            irradiance: Vec3::splat(6.0),
            angular_radius: 0.004675,
            drawn_radius: 0.012247,
            disc_radiance: Vec3::splat(8.0),
        }
    }

    #[test]
    fn the_veil_is_the_cies_glare_function() {
        // 10 / θ³ + 5 / θ² at θ degrees: 0.06 at 10°, 15 at 1°.
        for (degrees, expected) in [(10.0f32, 0.06), (1.0, 15.0), (30.0, 0.005_926)] {
            let got = veil(degrees.to_radians());
            assert!((got - expected).abs() < expected * 2e-3, "{degrees}: {got}");
        }
    }

    #[test]
    fn the_sun_is_placed_in_the_frame_and_fades_past_its_edge() {
        let camera = Camera {
            eye: Vec3::ZERO,
            target: Vec3::NEG_Z,
            fov_y_degrees: 60.0,
            ..Camera::default()
        };
        let aspect = 2.0;
        // Dead ahead: the frame's centre, in full.
        let terms = place(&sun(Vec3::NEG_Z), &camera, aspect);
        let tan = 30f32.to_radians().tan();
        assert_eq!(terms.sun[3], 0.012247);
        assert!((terms.light[0] - 6.0 * SHARE).abs() < 1e-6 && terms.light[3] == 1.0);
        assert!((terms.view[0] - tan * aspect).abs() < 1e-6 && (terms.view[1] - tan).abs() < 1e-6);
        assert!((Vec3::from_slice(&terms.sun[..3]) - Vec3::NEG_Z).length() < 1e-6);
        // At the top edge it still glares in full; half a margin above it, by half; past the margin, not at all.
        let up = |outside: f32| {
            let y = (1.0 + 2.0 * outside) * tan;
            place(&sun(Vec3::new(0.0, y, -1.0)), &camera, aspect)
        };
        assert!((up(0.0).light[0] - 6.0 * SHARE).abs() < 1e-4);
        assert!((up(MARGIN * 0.5).light[0] - 3.0 * SHARE).abs() < 1e-3);
        assert_eq!(up(MARGIN).light[0], 0.0);
        // Behind the camera: no light.
        assert_eq!(place(&sun(Vec3::Z), &camera, aspect).light[0], 0.0);
    }
}
