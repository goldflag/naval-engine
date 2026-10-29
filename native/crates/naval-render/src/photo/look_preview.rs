//! The Settings menu's Look tab (`native/docs/photo.md`, "Looks"): each look on the frame last drawn, small, so the
//! player compares them without leaving the menu ([`crate::Renderer::look_previews`]). Each tile is the display pass
//! drawing the whole frame at the tile's size (its radiance sampled over the pixels each tile pixel covers, with the
//! frame's bloom and eye) through the look's grade, halation and diffusion, then the film pass through its film, its
//! grain thinned to the tile ([`preview_film`]). The tiles are copied one after another into one buffer for the host
//! to map; nothing waits on the GPU.

use crate::film::FilmFinish;
use crate::photo::Look;

/// The tiles' format: their bytes are display-encoded RGBA, as an image file keeps them.
pub const PREVIEW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// The narrowest and widest tile.
const WIDTHS: (u32, u32) = (64, 1024);

/// The tiles on their way back: each look's in turn in `buffer`, `row_bytes` a row (whole rows, no padding).
pub struct LookPreviews {
    pub buffer: wgpu::Buffer,
    pub looks: Vec<Look>,
    pub width: u32,
    pub height: u32,
    pub row_bytes: u32,
}

impl LookPreviews {
    /// Bytes one tile takes in the buffer.
    pub fn tile_bytes(&self) -> usize {
        self.row_bytes as usize * self.height as usize
    }

    /// Tile `index`'s RGBA rows from the mapped buffer's bytes.
    pub fn tile<'a>(&self, mapped: &'a [u8], index: usize) -> &'a [u8] {
        let size = self.tile_bytes();
        &mapped[index * size..(index + 1) * size]
    }
}

/// A tile about `width` pixels across, at the frame's aspect: whole 64s across, so a row is whole 256 bytes, as a
/// copy into a buffer needs.
pub(crate) fn tile_size(width: u32, frame: (u32, u32)) -> (u32, u32) {
    let width = (width.clamp(WIDTHS.0, WIDTHS.1) + 32) / 64 * 64;
    let aspect = frame.1.max(1) as f32 / frame.0.max(1) as f32;
    (width, ((width as f32 * aspect).round() as u32).max(1))
}

/// `look`'s film on a tile `height` pixels tall: its grain thinned as the frame's would be drawn that small (a 1080-line
/// frame's grain over the square root of the pixels each tile pixel holds, so it still shows), at a fixed film frame.
pub(crate) fn preview_film(look: Look, height: u32) -> FilmFinish {
    let mut film = FilmFinish::new(look, None, 0.0);
    film.grain *= (height as f32 / 1080.0).sqrt().min(1.0);
    film
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_is_whole_64s_across_at_the_frames_aspect() {
        assert_eq!(tile_size(384, (1920, 1080)), (384, 216));
        assert_eq!(tile_size(400, (2560, 1600)), (384, 240));
        assert_eq!(tile_size(10, (1920, 1080)), (64, 36));
        assert_eq!(tile_size(5000, (1920, 1080)), (1024, 576));
        let (width, _) = tile_size(333, (1000, 1000));
        assert_eq!((width * 4) % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    }

    #[test]
    fn a_tiles_grain_is_thinned_and_the_rest_of_its_film_kept() {
        let full = FilmFinish::new(Look::GunCamera, None, 0.0);
        let tile = preview_film(Look::GunCamera, 270);
        assert!((tile.grain - full.grain * 0.5).abs() < 1e-6);
        assert_eq!(
            (tile.vignette, tile.weave, tile.dust),
            (full.vignette, full.weave, full.dust)
        );
        assert!(!preview_film(Look::Natural, 216).is_on());
    }
}
