//! Sampleable copies of the scene, taken between phases of the scene pass.
//!
//! The removed web game drew the sea first in three's transparent queue so that its viewport copies
//! (`viewportTexture()`, `ViewportDepthTextureNode`) held the opaque scene, and the effects' own
//! depth copy (`EffectLighting.sceneDepth`) was taken when the first smoke sprite drew, after the
//! sea had written its depth. A render attachment cannot be sampled in the pass that draws into it,
//! so the renderer ends the scene pass at two points, copies what layers asked for, and carries on:
//!
//! | Copy point | Taken | Holds | Web counterpart |
//! | --- | --- | --- | --- |
//! | [`CopyPoint::Opaque`] | after every `Opaque` draw, before `Water` | the opaque scene: black and far (depth 0) where water and sky will be | the ocean's `viewportTexture()` and `EffectDepthTextureNode` |
//! | [`CopyPoint::Backdrop`] | after `Water` and `Sky`, before `Transparent` | everything transparents draw over: opaque, sea and sky (the sky writes no depth) | `EffectLighting.sceneDepth` (soft particles) |
//!
//! Copies are taken only when a layer asks for them ([`crate::SceneLayer::scene_copies`]); a frame
//! nobody asks of stays one render pass.

use std::ops::{BitAnd, BitOr, BitOrAssign};

/// Registry name of the opaque scene's color copy (`Rgba16Float`, linear HDR radiance).
pub const SCENE_OPAQUE_COLOR: &str = "scene.opaque.color";
/// Registry name of the opaque scene's depth copy (`Depth32Float`, reversed).
pub const SCENE_OPAQUE_DEPTH: &str = "scene.opaque.depth";
/// Registry name of the backdrop's color copy (opaque, water and sky).
pub const SCENE_BACKDROP_COLOR: &str = "scene.backdrop.color";
/// Registry name of the backdrop's depth copy (opaque and water).
pub const SCENE_BACKDROP_DEPTH: &str = "scene.backdrop.depth";

/// Where in the scene pass copies are taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyPoint {
    /// After the `Opaque` phase, before `Water`.
    Opaque,
    /// After the `Water` and `Sky` phases, before `Transparent`.
    Backdrop,
}

impl CopyPoint {
    pub const ALL: [CopyPoint; 2] = [CopyPoint::Opaque, CopyPoint::Backdrop];

    /// The copies taken at this point.
    pub fn copies(self) -> SceneCopies {
        match self {
            CopyPoint::Opaque => SceneCopies::OPAQUE_COLOR | SceneCopies::OPAQUE_DEPTH,
            CopyPoint::Backdrop => SceneCopies::BACKDROP_COLOR | SceneCopies::BACKDROP_DEPTH,
        }
    }
}

/// A set of scene copies. Combine with `|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SceneCopies(u8);

impl SceneCopies {
    pub const NONE: Self = Self(0);
    /// [`SCENE_OPAQUE_COLOR`]: the sea's straight-through transmission and screen-space reflections.
    pub const OPAQUE_COLOR: Self = Self(1);
    /// [`SCENE_OPAQUE_DEPTH`]: the water column behind the sea, shoreline and waterline foam.
    pub const OPAQUE_DEPTH: Self = Self(2);
    /// [`SCENE_BACKDROP_COLOR`]: refraction or distortion by transparents.
    pub const BACKDROP_COLOR: Self = Self(4);
    /// [`SCENE_BACKDROP_DEPTH`]: soft particles fading into hulls and the sea.
    pub const BACKDROP_DEPTH: Self = Self(8);
    pub const ALL: Self = Self(15);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Each copy in the set with its registry name and whether it is depth.
    pub fn entries(self) -> impl Iterator<Item = (SceneCopies, &'static str, bool)> {
        [
            (Self::OPAQUE_COLOR, SCENE_OPAQUE_COLOR, false),
            (Self::OPAQUE_DEPTH, SCENE_OPAQUE_DEPTH, true),
            (Self::BACKDROP_COLOR, SCENE_BACKDROP_COLOR, false),
            (Self::BACKDROP_DEPTH, SCENE_BACKDROP_DEPTH, true),
        ]
        .into_iter()
        .filter(move |(copy, ..)| self.contains(*copy))
    }
}

impl BitOr for SceneCopies {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for SceneCopies {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitAnd for SceneCopies {
    type Output = Self;
    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// A bind group layout entry for a color copy: `texture_2d<f32>`, filterable (`Rgba16Float` is),
/// so it can be read with `textureLoad` or sampled.
pub fn color_copy_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A bind group layout entry for a depth copy: `texture_depth_2d`, read with `textureLoad`
/// (texel exact) or a comparison sampler. Reversed depth: 1 at the near plane, 0 at the far
/// plane and wherever nothing wrote depth; `naval::camera::view_depth` turns it into meters.
pub fn depth_copy_layout_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_combine_and_name_their_textures() {
        let set = SceneCopies::OPAQUE_COLOR | SceneCopies::BACKDROP_DEPTH;
        assert!(set.contains(SceneCopies::OPAQUE_COLOR));
        assert!(!set.contains(SceneCopies::OPAQUE_DEPTH));
        assert!(set.intersects(CopyPoint::Opaque.copies()));
        assert!(set.intersects(CopyPoint::Backdrop.copies()));
        let names: Vec<_> = set
            .entries()
            .map(|(_, name, depth)| (name, depth))
            .collect();
        assert_eq!(
            names,
            [(SCENE_OPAQUE_COLOR, false), (SCENE_BACKDROP_DEPTH, true)]
        );
        assert!(SceneCopies::NONE.is_empty());
        assert_eq!(
            CopyPoint::Opaque.copies() | CopyPoint::Backdrop.copies(),
            SceneCopies::ALL
        );
    }
}
