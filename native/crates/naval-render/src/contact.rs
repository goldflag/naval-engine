//! Contact shadows (native/docs/render-core/frame.md, "Contact shadows"): the sun's shadow from the parts close above a
//! surface, which the sun's maps leave out (their receiver offsets lift a surface 20 to 50 cm off itself on a
//! battleship's near map, and their filter spreads an edge over six texels).
//!
//! While [`crate::ShadowSettings::contact`] is on with the maps, each frame:
//!
//! 1. **The contact depth.** The layers that take contact shadows draw their surfaces depth-only for the frame's own
//!    camera (jitter and all) into [`CONTACT_DEPTH`], at the scene's size, through their caster pipelines
//!    ([`crate::SceneLayer::draw_contact_depth`]: group 0 is the frame's camera in the caster layout). The hulls do;
//!    nothing else yet.
//! 2. **The march** (`shaders/contact.wgsl`): each pixel holding a surface walks toward the light through that
//!    depth, a pixel a step, over a few dozen of its own footprints (at most a metre), and is in shadow where a
//!    surface stands in front of the walk by less than a thickness. The result is [`CONTACT_MASK`], one channel,
//!    1 lit and 0 shadowed, the shadows' intensity applied.
//!
//! A receiver binds the mask (a viewport texture, found by name; rebind when its `version` changes), reads it at its
//! own pixel and takes the darker of it and the maps' shadow for the sun alone. With no layer drawing contact depth,
//! or no sun, the mask is cleared to lit and the depth to the far plane (the hulls also march the fires' light through
//! it after dark: `naval-ships`' `fire_seen`). Its textures exist only while contact shadows are on.

use crate::frame::DEPTH_FORMAT;
use crate::pipelines::{PipelineId, PipelineRegistry};
use crate::post::fullscreen_recipe;
use crate::resources::{Resources, TextureDesc, TextureEntry};
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};

pub const CONTACT_SHADER: ShaderFile = crate::shader_file!("shaders/contact.wgsl");

/// Registry name of the depth contact shadows march through: the surfaces that take them, drawn for the frame's camera
/// before the scene (`Depth32Float`, reversed, the scene's size).
pub const CONTACT_DEPTH: &str = "contact.depth";
/// Registry name of the contact shadows' mask (`R8Unorm`, the scene's size): 1 lit, 0 in a contact shadow.
pub const CONTACT_MASK: &str = "contact.mask";
/// The mask's format.
pub const CONTACT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// The longest walk toward the light (m): `REACH_M` in `contact.wgsl`.
pub const REACH_M: f32 = 1.0;
/// A walk spanning fewer pixels than this finds nothing the maps miss and is not taken: `MIN_PIXELS` in `contact.wgsl`.
pub const MIN_PIXELS: f32 = 3.0;

/// How far from the camera (m along the view) contact shadows still show, for a camera whose lens draws a metre at a
/// metre's distance `focal_px` pixels tall (its projection's y scale times half the viewport's height): beyond it
/// the walk spans under [`MIN_PIXELS`]. A layer leaves what lies wholly beyond it out of the contact depth.
pub fn reach_distance(focal_px: f32) -> f32 {
    focal_px * REACH_M / MIN_PIXELS
}

/// The march's pipeline and groups.
pub(crate) struct ContactShadows {
    pipeline: PipelineId,
    layout: wgpu::BindGroupLayout,
    /// Group 1 of the march (the contact depth), by the depth texture's version.
    group: Option<(u64, wgpu::BindGroup)>,
    /// Group 0 of the layers' caster pipelines holding the frame's camera (the scene camera's own buffer).
    camera: wgpu::BindGroup,
    /// The mask's version while it holds nothing but lit (cleared, and not marched since): a frame without a march
    /// clears it only when this is not the mask's current version (a new texture starts at zero, all shadow).
    cleared: Option<u64>,
}

impl ContactShadows {
    /// Registers the march's pipeline (group 0 the scene's camera layout, group 1 the contact depth) and makes the
    /// casters' camera group over `camera_buffer`, the scene camera's uniforms.
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        camera_layout: &wgpu::BindGroupLayout,
        caster_layout: &wgpu::BindGroupLayout,
        camera_buffer: &wgpu::Buffer,
    ) -> Result<Self, ShaderError> {
        library.add(CONTACT_SHADER)?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("contact depth"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline = pipelines.add(
            device,
            library,
            "contact shadows",
            CONTACT_SHADER.path,
            Vec::new(),
            fullscreen_recipe(
                vec![camera_layout.clone(), layout.clone()],
                "vs_contact",
                "fs_contact",
                CONTACT_FORMAT,
                None,
            ),
        )?;
        let camera = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("contact depth camera"),
            layout: caster_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });
        Ok(Self {
            pipeline,
            layout,
            group: None,
            camera,
            cleared: None,
        })
    }

    /// The mask now holds a march.
    pub(crate) fn mark_marched(&mut self) {
        self.cleared = None;
    }

    /// Whether a frame without a march must clear the mask of `version` to lit; marks it cleared.
    pub(crate) fn needs_clear(&mut self, version: u64) -> bool {
        let needs = self.cleared != Some(version);
        self.cleared = Some(version);
        needs
    }

    /// Makes the contact depth and the mask at the viewport's size while `on`, and lets them go while off.
    pub(crate) fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        resources: &mut Resources,
        on: bool,
    ) {
        let have = resources.texture(CONTACT_MASK).is_some();
        if on && !have {
            let attach =
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
            resources.create_texture(
                device,
                CONTACT_DEPTH,
                TextureDesc::viewport(DEPTH_FORMAT, attach, 1.0),
            );
            // Read back by tests and captures.
            resources.create_texture(
                device,
                CONTACT_MASK,
                TextureDesc::viewport(CONTACT_FORMAT, attach | wgpu::TextureUsages::COPY_SRC, 1.0),
            );
        } else if !on && have {
            resources.remove(CONTACT_DEPTH);
            resources.remove(CONTACT_MASK);
            self.group = None;
            self.cleared = None;
        }
    }

    /// Group 0 of the layers' caster pipelines in the contact depth pass: the frame's camera.
    pub(crate) fn camera_group(&self) -> &wgpu::BindGroup {
        &self.camera
    }

    pub(crate) fn pipeline(&self) -> PipelineId {
        self.pipeline
    }

    /// Group 1 of the march: the contact depth, remade when its texture is.
    pub(crate) fn group(
        &mut self,
        device: &wgpu::Device,
        depth: &TextureEntry,
    ) -> &wgpu::BindGroup {
        if self
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != depth.version)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("contact depth"),
                layout: &self.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth.view),
                }],
            });
            self.group = Some((depth.version, group));
        }
        &self.group.as_ref().expect("made above").1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The constants the hulls are culled by are the shader's.
    #[test]
    fn the_reach_is_the_shaders() {
        let constant = |name: &str| -> f32 {
            let line = CONTACT_SHADER
                .source
                .lines()
                .find(|line| line.starts_with(&format!("const {name}: f32 = ")))
                .unwrap_or_else(|| panic!("contact.wgsl has no f32 {name}"));
            let value = line.split('=').nth(1).expect("a value");
            value
                .trim()
                .trim_end_matches(';')
                .parse()
                .expect("a number")
        };
        assert_eq!(constant("REACH_M"), REACH_M);
        assert_eq!(constant("MIN_PIXELS"), MIN_PIXELS);
    }

    #[test]
    fn contact_shadows_reach_further_through_a_longer_lens() {
        // The battle's 52 degree lens over 1081 rows: about 370 m; through binoculars at x8, eight times that.
        let focal = 1081.0 * 0.5 / 26f32.to_radians().tan();
        assert!(
            (reach_distance(focal) - 369.4).abs() < 1.0,
            "{}",
            reach_distance(focal)
        );
        assert!((reach_distance(focal * 8.0) / reach_distance(focal) - 8.0).abs() < 1e-4);
    }
}
