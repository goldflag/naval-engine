//! What a layer lays over the scene that the scene's depth does not hold, as the transparents drawn after it read it.
//! The splash columns are marched into a target of their own at the backdrop's copy point and laid over the scene before
//! the combat effects draw, and a burning ship's smoke behind a column drew over it (the smoke is tested against the
//! scene's depth, which holds the hulls and the sea but not the column). The layer that lays the cover publishes here,
//! at the copy point, the target it laid and how far along the view its opacity lies; a layer drawn after it reads that
//! at the same copy point and fades what the cover stands in front of. It lives here because both crates depend on
//! this one.

use std::sync::{Arc, Mutex};

/// A cover's target as its layer published it: clear wherever nothing was laid this frame.
#[derive(Clone, Debug)]
pub struct CoverTarget {
    /// Premultiplied linear radiance, opacity in alpha (`Rgba16Float`), at the scene's size or a fraction of it.
    pub color: wgpu::TextureView,
    /// Over the same texels, the view depth (m) times the opacity, added up as the colour is (`R16Float`): over the
    /// colour's alpha, the depth the cover's opacity lies at.
    pub depth: wgpu::TextureView,
    /// Changes whenever the views are made again, so a reader knows to rebind them.
    pub version: u64,
}

/// A handle on a cover, shared by the layer that lays it and the layers that read it: `None` until it has a target.
#[derive(Clone, Debug, Default)]
pub struct Cover(Arc<Mutex<Option<CoverTarget>>>);

impl Cover {
    /// Publishes the view's target (a second view publishes its own over the main view's, and the main view its own
    /// back, each at its copy point).
    pub fn publish(&self, target: Option<CoverTarget>) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = target;
    }

    /// The target published last.
    pub fn read(&self) -> Option<CoverTarget> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}
