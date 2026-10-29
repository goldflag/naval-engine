//! The scene pass's groups of phases, ended where a layer asked for a copy of the scene, and the copies.

use crate::copies::{CopyPoint, SceneCopies};
use crate::resources::{Resources, TextureDesc};

use super::layer::Phase;
use super::{DEPTH_CLEAR, DEPTH_FORMAT, HDR_FORMAT, SCENE_DEPTH, SCENE_HDR};

/// The phases each render pass of the scene draws, and the copy point that ends
/// it: a pass ends only where `copies` asks for a copy.
pub(crate) fn scene_pass_groups(copies: SceneCopies) -> Vec<(Vec<Phase>, Option<CopyPoint>)> {
    let mut groups = Vec::new();
    let mut phases = Vec::new();
    for phase in Phase::ALL {
        phases.push(phase);
        if let Some(point) = phase
            .copy_point_after()
            .filter(|point| copies.intersects(point.copies()))
        {
            groups.push((std::mem::take(&mut phases), Some(point)));
        }
    }
    if !phases.is_empty() {
        groups.push((phases, None));
    }
    groups
}

/// Registers the textures for `copies` that do not exist yet: full-size viewport
/// textures in the scene targets' formats.
pub(super) fn ensure_scene_copies(
    device: &wgpu::Device,
    resources: &mut Resources,
    copies: SceneCopies,
) {
    for (_, name, depth) in copies.entries() {
        if resources.texture(name).is_some() {
            continue;
        }
        let format = if depth { DEPTH_FORMAT } else { HDR_FORMAT };
        resources.create_texture(
            device,
            name,
            TextureDesc::viewport(
                format,
                // COPY_SRC for captures and tests that read a copy back.
                wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                1.0,
            ),
        );
    }
}

/// Copies the scene targets into each of `copies` (whole textures, as a depth
/// copy must be).
pub(super) fn copy_scene(
    encoder: &mut wgpu::CommandEncoder,
    resources: &Resources,
    copies: SceneCopies,
) {
    for (_, name, depth) in copies.entries() {
        let source = resources
            .texture(if depth { SCENE_DEPTH } else { SCENE_HDR })
            .expect("registered in new");
        let target = resources
            .texture(name)
            .expect("made by ensure_scene_copies");
        encoder.copy_texture_to_texture(
            source.texture.as_image_copy(),
            target.texture.as_image_copy(),
            source.texture.size(),
        );
    }
}

/// One render pass of the scene: clears the targets for the first, loads them after.
pub(super) fn begin_scene_pass<'e>(
    encoder: &'e mut wgpu::CommandEncoder,
    resources: &Resources,
    label: &str,
    load: bool,
    timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
) -> wgpu::RenderPass<'e> {
    let hdr = resources.texture(SCENE_HDR).expect("registered in new");
    let depth = resources.texture(SCENE_DEPTH).expect("registered in new");
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: &hdr.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if load {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                },
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: &depth.view,
            depth_ops: Some(wgpu::Operations {
                load: if load {
                    wgpu::LoadOp::Load
                } else {
                    wgpu::LoadOp::Clear(DEPTH_CLEAR)
                },
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes,
        ..Default::default()
    })
}

/// A shadow map pass's label (`shadow <map>`), named once.
pub(super) fn shadow_label(kind: crate::shadows::ShadowMapKind) -> &'static str {
    use crate::shadows::ShadowMapKind;
    match kind {
        ShadowMapKind::Near => "shadow near",
        ShadowMapKind::Wide => "shadow wide",
        ShadowMapKind::Aboard => "shadow aboard",
        ShadowMapKind::View(0) => "shadow view 1",
        ShadowMapKind::View(_) => "shadow view 2",
        ShadowMapKind::Land(0) => "shadow land 1",
        ShadowMapKind::Land(1) => "shadow land 2",
        ShadowMapKind::Land(_) => "shadow land 3",
        ShadowMapKind::Light { .. } => "shadow light",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scene_pass_ends_only_where_a_copy_is_asked_for() {
        let groups = scene_pass_groups(SceneCopies::NONE);
        assert_eq!(groups, [(Phase::ALL.to_vec(), None)]);
        let groups = scene_pass_groups(SceneCopies::OPAQUE_DEPTH);
        assert_eq!(
            groups,
            [
                (vec![Phase::Opaque], Some(CopyPoint::Opaque)),
                (vec![Phase::Water, Phase::Sky, Phase::Transparent], None),
            ]
        );
        let groups = scene_pass_groups(SceneCopies::ALL);
        assert_eq!(
            groups,
            [
                (vec![Phase::Opaque], Some(CopyPoint::Opaque)),
                (vec![Phase::Water, Phase::Sky], Some(CopyPoint::Backdrop)),
                (vec![Phase::Transparent], None),
            ]
        );
        let groups = scene_pass_groups(SceneCopies::BACKDROP_COLOR);
        assert_eq!(
            groups,
            [
                (
                    vec![Phase::Opaque, Phase::Water, Phase::Sky],
                    Some(CopyPoint::Backdrop)
                ),
                (vec![Phase::Transparent], None),
            ]
        );
    }
}
