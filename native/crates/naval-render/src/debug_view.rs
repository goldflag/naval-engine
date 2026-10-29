//! Full-screen debug views for the developer panel (`Renderer::set_debug_view`).
//!
//! - [`DebugView::Wireframe`]: the opaque scene geometry that allows it (hulls, land, aircraft) drawn as
//!   lines through each pipeline's line twin ([`crate::PipelineRegistry::allow_wireframe`]); the sea, sky and effects
//!   draw as they are. Needs `Features::POLYGON_MODE_LINE`; the twins compile on the build threads when the view is
//!   first chosen, and until each is ready its pipeline draws filled.
//! - [`DebugView::Normals`], [`DebugView::Depth`], [`DebugView::ShadowCascades`]: a pass after the post chain, onto
//!   the output, from the scene's depth and the camera block (`shaders/debug_view.wgsl`). The normals are face normals
//!   rebuilt from depth (no layer writes its shading normals anywhere the renderer can read); the cascades are the sun
//!   map each pixel samples, multiplied over the finished frame.
//!
//! Nothing is built or drawn while the view is [`DebugView::Off`].

use crate::frame::{OverlayPipeline, OverlayPipelineId, RenderContext};
use crate::shaders::{ShaderError, ShaderFile};

pub const DEBUG_VIEW_SHADER: ShaderFile = crate::shader_file!("shaders/debug_view.wgsl");

/// What the frame shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DebugView {
    /// The frame as the game draws it.
    #[default]
    Off,
    Wireframe,
    Normals,
    Depth,
    ShadowCascades,
}

impl DebugView {
    pub const ALL: [DebugView; 5] = [
        DebugView::Off,
        DebugView::Wireframe,
        DebugView::Normals,
        DebugView::Depth,
        DebugView::ShadowCascades,
    ];

    /// The view's name in the UI contract (`render-debug`'s `view`).
    pub fn name(self) -> &'static str {
        match self {
            DebugView::Off => "off",
            DebugView::Wireframe => "wireframe",
            DebugView::Normals => "normals",
            DebugView::Depth => "depth",
            DebugView::ShadowCascades => "shadow-cascades",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|view| view.name() == name)
    }

    /// The views a device can show: every one but the wireframe without `Features::POLYGON_MODE_LINE`.
    pub fn offered(features: wgpu::Features) -> Vec<DebugView> {
        Self::ALL
            .into_iter()
            .filter(|view| {
                *view != DebugView::Wireframe
                    || features.contains(wgpu::Features::POLYGON_MODE_LINE)
            })
            .collect()
    }

    /// The full-screen pass's fragment entry, for the views drawn after the post chain.
    fn entry(self) -> Option<&'static str> {
        match self {
            DebugView::Normals => Some("fs_normals"),
            DebugView::Depth => Some("fs_depth"),
            DebugView::ShadowCascades => Some("fs_cascades"),
            DebugView::Off | DebugView::Wireframe => None,
        }
    }
}

/// The full-screen views' pipelines (made the first time one is chosen) and their depth binding.
#[derive(Default)]
pub(crate) struct DebugViews {
    pub view: DebugView,
    built: Option<Built>,
    /// The depth binding and the depth texture's version it was made on.
    group: Option<(u64, wgpu::BindGroup)>,
}

struct Built {
    layout: wgpu::BindGroupLayout,
    /// Normals, depth, cascades.
    pipelines: [OverlayPipelineId; 3],
}

impl DebugViews {
    /// Registers the full-screen views' pipelines, once.
    pub(crate) fn build(&mut self, ctx: &mut RenderContext<'_>) -> Result<(), ShaderError> {
        if self.built.is_some() {
            return Ok(());
        }
        ctx.add_shader(DEBUG_VIEW_SHADER)?;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("debug view depth"),
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
        let mut add = |label: &str, entry: &'static str, blend: wgpu::BlendState| {
            let mut desc = OverlayPipeline::new("vs_debug", entry);
            desc.bind_group_layouts = vec![layout.clone()];
            desc.blend = Some(blend);
            ctx.add_overlay_pipeline(label, DEBUG_VIEW_SHADER.path, desc)
        };
        let multiply = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Dst,
            dst_factor: wgpu::BlendFactor::Zero,
            operation: wgpu::BlendOperation::Add,
        };
        let pipelines = [
            add(
                "debug view normals",
                "fs_normals",
                wgpu::BlendState::REPLACE,
            )?,
            add("debug view depth", "fs_depth", wgpu::BlendState::REPLACE)?,
            add(
                "debug view shadow cascades",
                "fs_cascades",
                wgpu::BlendState {
                    color: multiply,
                    alpha: wgpu::BlendComponent::OVER,
                },
            )?,
        ];
        self.built = Some(Built { layout, pipelines });
        Ok(())
    }

    /// The full-screen pass this frame, if the view has one and it is built.
    pub(crate) fn pipeline(&self) -> Option<OverlayPipelineId> {
        let built = self.built.as_ref()?;
        Some(match self.view.entry()? {
            "fs_normals" => built.pipelines[0],
            "fs_depth" => built.pipelines[1],
            _ => built.pipelines[2],
        })
    }

    /// Group 1 of the pass: the scene's depth, remade when the depth texture is.
    pub(crate) fn group(
        &mut self,
        device: &wgpu::Device,
        depth: &crate::resources::TextureEntry,
    ) -> Option<&wgpu::BindGroup> {
        let built = self.built.as_ref()?;
        if self
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != depth.version)
        {
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("debug view depth"),
                layout: &built.layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&depth.view),
                }],
            });
            self.group = Some((depth.version, group));
        }
        self.group.as_ref().map(|(_, group)| group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_by_name_and_the_wireframe_needs_line_polygons() {
        for view in DebugView::ALL {
            assert_eq!(DebugView::from_name(view.name()), Some(view));
        }
        assert_eq!(DebugView::from_name("overdraw"), None);
        assert!(!DebugView::offered(wgpu::Features::empty()).contains(&DebugView::Wireframe));
        assert_eq!(
            DebugView::offered(wgpu::Features::POLYGON_MODE_LINE).len(),
            DebugView::ALL.len()
        );
    }
}
