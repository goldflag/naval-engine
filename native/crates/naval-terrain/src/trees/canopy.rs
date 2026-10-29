//! Trees where real trees stand (native/docs/terrain/trees.md, "Trees where real trees stand"): a canopy height map
//! about the
//! camera, as pages of texels each holding the tallest crown within it and the share of its ground under canopy, which
//! the trees' placement reads (`tree_place.wgsl`'s `canopy_crown`). A slot whose cell a page covers stands its tree on
//! the tallest crown in the cell at its measured height, as wide as the cell's canopy, or stands none where the map has
//! no crown; past the pages the land cover's density places the trees as before.
//!
//! - **What a texel holds.** RGBA8: R the tallest crown's apex height (m; 0, none of [`MIN_CROWN_M`]); G where in the
//!   texel it stands, x in the high nibble and z in the low, in sixteenths of the texel ((v + 0.5) / 16); B the share of
//!   the texel's ground under canopy [`COVER_MIN_M`] or taller (0–255); A 255 where the map covers the texel, 0 where it
//!   does not (the land cover's trees there).
//! - **Crowns** ([`CanopyPage::from_heights`]): from a grid of the map's heights brought to cells of a few metres (the
//!   host's: the tallest of the source pixels in each), a crown is a cell at least [`MIN_CROWN_M`] tall that no cell
//!   within [`CROWN_REACH_CELLS`] tops, nor a crown as tall found before it in row order (so a closed canopy's flat
//!   top, the map's whole metres, has a crown every few cells, as its trees stand).
//! - **Pages.** [`PAGE_TEXELS`]² texels, fixed squares of the host's metres (page (i, j) from (i, j) × its side), held
//!   `slots`² about the camera in one texture, page (i, j) in slot (i mod slots, j mod slots), as the map features' are
//!   (`features`); the placement reads a slot only where the uniform's entry names the page there.

use std::collections::VecDeque;
use std::sync::Arc;

/// Texels on a page's side.
pub const PAGE_TEXELS: u32 = 256;
/// Most slots a side (the trees' view holds 64 entries).
pub const MAX_SLOTS: u32 = 8;
/// The shortest crown that stands a tree (m): lower is scrub, the land cover's.
pub const MIN_CROWN_M: u8 = 3;
/// The height counted as canopy cover (m).
pub const COVER_MIN_M: u8 = 2;
/// A crown tops every cell within this many cells of it (Chebyshev): its eight neighbours. A reach of 2 left a 20 m
/// square of a closed forest without a crown one time in five, a taller tree's just over its edge topping all of its.
pub const CROWN_REACH_CELLS: usize = 1;
/// A height the host's grid marks as not covered by the map.
pub const UNKNOWN: u8 = 255;
/// Bytes of pages uploaded in a frame at most (at least one page): 256 KB a page.
pub const UPLOAD_BYTES: usize = 2 << 20;
/// What the placement reads for a slot holding no page.
pub const NO_PAGE: i32 = i32::MIN;

/// The pages' grid on a land layer: the chart corner of its page (0, 0), a texel (m), and the slots a side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanopyGrid {
    pub origin: [f64; 2],
    pub texel_m: f64,
    pub slots: u32,
}

impl CanopyGrid {
    /// A page's side, metres.
    pub fn page_m(&self) -> f64 {
        self.texel_m * f64::from(PAGE_TEXELS)
    }

    /// The texture's side, texels.
    pub fn texels(&self) -> u32 {
        self.slots * PAGE_TEXELS
    }

    /// The slot page `index` takes.
    pub fn slot(&self, index: [i32; 2]) -> [u32; 2] {
        let n = self.slots as i32;
        [index[0].rem_euclid(n) as u32, index[1].rem_euclid(n) as u32]
    }
}

/// One page's texels, RGBA8, row 0 at the smallest z.
#[derive(Clone, Debug, PartialEq)]
pub struct CanopyPage {
    pub texels: Vec<u8>,
}

/// A crown found on a page (`CanopyPage::crowns`): where it stands (page metres) and its height (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crown {
    pub at: [f64; 2],
    pub height: u8,
}

impl CanopyPage {
    /// A page the map does not cover: the land cover's trees all over it.
    pub fn unknown() -> Self {
        Self {
            texels: vec![0; (PAGE_TEXELS * PAGE_TEXELS * 4) as usize],
        }
    }

    pub fn bytes(&self) -> usize {
        self.texels.len()
    }

    /// The page from a grid of heights (m, [`UNKNOWN`] where the map does not cover a cell), `cells` a texel's side in
    /// cells: the grid is `side`² cells (`side` = [`PAGE_TEXELS`] × `cells`) with `margin` cells more about it (the
    /// neighbours' cells, so a crown at the page's edge is found as it would be in the middle), row 0 at the smallest z.
    pub fn from_heights(heights: &[u8], cells: usize, margin: usize) -> Self {
        let side = PAGE_TEXELS as usize * cells;
        let width = side + 2 * margin;
        assert_eq!(
            heights.len(),
            width * width,
            "a canopy grid of the wrong size"
        );
        assert!(
            (1..=16).contains(&cells),
            "a texel of {cells} cells: its position nibbles need at most 16"
        );
        let n = PAGE_TEXELS as usize;
        let mut texels = vec![0u8; n * n * 4];
        let height = |x: usize, y: usize| heights[y * width + x];
        let crowns = crown_mask(heights, width, CROWN_REACH_CELLS.min(margin));
        for ty in 0..n {
            for tx in 0..n {
                let (mut known, mut covered) = (0usize, 0usize);
                let mut best: Option<(u8, usize, usize)> = None;
                for cy in 0..cells {
                    for cx in 0..cells {
                        let (x, y) = (margin + tx * cells + cx, margin + ty * cells + cy);
                        let h = height(x, y);
                        if h == UNKNOWN {
                            continue;
                        }
                        known += 1;
                        if h >= COVER_MIN_M {
                            covered += 1;
                        }
                        if h < MIN_CROWN_M || best.is_some_and(|(b, _, _)| b >= h) {
                            continue;
                        }
                        if crowns[y * width + x] {
                            best = Some((h, cx, cy));
                        }
                    }
                }
                let texel = &mut texels[(ty * n + tx) * 4..][..4];
                // Half the texel's cells or more known: the map covers it.
                if known * 2 < cells * cells {
                    continue;
                }
                if let Some((h, cx, cy)) = best {
                    // The cell's middle in sixteenths of the texel, as the shader reads it back ((v + 0.5) / 16).
                    let nibble = |c: usize| {
                        ((c as f64 + 0.5) / cells as f64 * 16.0 - 0.5)
                            .round()
                            .clamp(0.0, 15.0) as u8
                    };
                    texel[0] = h;
                    texel[1] = (nibble(cx) << 4) | nibble(cy);
                }
                texel[2] = ((covered * 255 + known / 2) / known) as u8;
                texel[3] = 255;
            }
        }
        Self { texels }
    }

    /// The texel (i, j): its crown's height (0 none), the crown's place in it (0–1 each way), the canopy's share
    /// (0–1), whether the map covers it.
    pub fn texel(&self, i: u32, j: u32) -> (u8, [f64; 2], f64, bool) {
        let t = &self.texels[((j * PAGE_TEXELS + i) * 4) as usize..][..4];
        let at = [
            (f64::from(t[1] >> 4) + 0.5) / 16.0,
            (f64::from(t[1] & 15) + 0.5) / 16.0,
        ];
        (t[0], at, f64::from(t[2]) / 255.0, t[3] > 0)
    }

    /// Every crown on the page, `texel_m` a texel: where it stands (page metres) and its height.
    pub fn crowns(&self, texel_m: f64) -> Vec<Crown> {
        let mut out = Vec::new();
        for j in 0..PAGE_TEXELS {
            for i in 0..PAGE_TEXELS {
                let (height, at, _, known) = self.texel(i, j);
                if known && height > 0 {
                    out.push(Crown {
                        at: [
                            (f64::from(i) + at[0]) * texel_m,
                            (f64::from(j) + at[1]) * texel_m,
                        ],
                        height,
                    });
                }
            }
        }
        out
    }

    /// Whether the map covers any of the page.
    pub fn known(&self) -> bool {
        self.texels.as_chunks::<4>().0.iter().any(|t| t[3] > 0)
    }
}

/// Which cells of a `width`² grid are crowns: at least [`MIN_CROWN_M`] tall, topped by no cell within `reach` (an
/// unknown cell is none), and no crown as tall within it found before them in row order. A flat top of a few cells has
/// one crown, a wide one a crown every `reach + 1` cells.
fn crown_mask(heights: &[u8], width: usize, reach: usize) -> Vec<bool> {
    let mut crowns = vec![false; heights.len()];
    for y in 0..width {
        for x in 0..width {
            let h = heights[y * width + x];
            if h == UNKNOWN || h < MIN_CROWN_M {
                continue;
            }
            let rows = y.saturating_sub(reach)..=(y + reach).min(width - 1);
            let topped = rows.into_iter().any(|ny| {
                (x.saturating_sub(reach)..=(x + reach).min(width - 1)).any(|nx| {
                    let k = ny * width + nx;
                    let other = heights[k];
                    other != UNKNOWN && (other > h || (other == h && crowns[k]))
                })
            });
            crowns[y * width + x] = !topped;
        }
    }
    crowns
}

/// The land layer's canopy texture: one RGBA8 2D texture of `slots`² pages, bound at the trees' placement's binding 4.
pub(crate) struct CanopyTextures {
    placeholder: wgpu::TextureView,
    texture: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
    grid: Option<CanopyGrid>,
    held: Vec<Option<[i32; 2]>>,
    queue: VecDeque<([i32; 2], Arc<CanopyPage>)>,
    /// Read (the default); hidden, the placement reads none (for comparisons).
    pub shown: bool,
}

fn texture(device: &wgpu::Device, label: &str, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

impl CanopyTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        let placeholder = texture(device, "terrain canopy placeholder", 1);
        Self {
            placeholder: placeholder.create_view(&Default::default()),
            texture: None,
            grid: None,
            held: Vec::new(),
            queue: VecDeque::new(),
            shown: true,
        }
    }

    /// The view binding 4 takes.
    pub fn view(&self) -> &wgpu::TextureView {
        self.texture
            .as_ref()
            .map_or(&self.placeholder, |(_, v, _)| v)
    }

    /// Takes pages on `grid`; true when the view changed.
    pub fn set_grid(&mut self, device: &wgpu::Device, grid: CanopyGrid) -> Result<bool, String> {
        if grid.slots == 0 || grid.slots > MAX_SLOTS || grid.texel_m <= 0.0 {
            return Err(format!(
                "the canopy: {} a side of {} m texels",
                grid.slots, grid.texel_m
            ));
        }
        let largest = device.limits().max_texture_dimension_2d;
        if grid.texels() > largest {
            return Err(format!(
                "the canopy: {0}² texels, more than this device's {largest}² textures",
                grid.texels()
            ));
        }
        if self.grid == Some(grid) {
            return Ok(false);
        }
        self.grid = Some(grid);
        self.held = vec![None; (grid.slots * grid.slots) as usize];
        self.queue.clear();
        if self
            .texture
            .as_ref()
            .is_some_and(|(_, _, size)| *size == grid.texels())
        {
            return Ok(false);
        }
        let made = texture(device, "terrain canopy", grid.texels());
        let view = made.create_view(&Default::default());
        self.texture = Some((made, view, grid.texels()));
        Ok(true)
    }

    /// No pages: true when the view changed.
    pub fn clear(&mut self) -> bool {
        self.grid = None;
        self.queue.clear();
        self.held.clear();
        self.texture.take().is_some()
    }

    pub fn grid(&self) -> Option<CanopyGrid> {
        self.grid
    }

    /// Whether page `index` is read or waiting for its upload.
    pub fn holds(&self, index: [i32; 2]) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        let [i, j] = grid.slot(index);
        self.held.get((j * grid.slots + i) as usize) == Some(&Some(index))
            || self.queue.iter().any(|(k, _)| *k == index)
    }

    /// Pages read, and waiting for their upload.
    pub fn counts(&self) -> (usize, usize) {
        (self.held.iter().flatten().count(), self.queue.len())
    }

    /// Queues page `index` for its upload (a page queued for the same slot before is dropped).
    pub fn queue(&mut self, index: [i32; 2], page: Arc<CanopyPage>) {
        let Some(grid) = self.grid else {
            return;
        };
        if page.texels.len() != (PAGE_TEXELS * PAGE_TEXELS * 4) as usize {
            return;
        }
        let slot = grid.slot(index);
        self.queue
            .retain(|(k, _)| *k == index || grid.slot(*k) != slot);
        if !self.queue.iter().any(|(k, _)| *k == index) {
            self.queue.push_back((index, page));
        }
    }

    /// Uploads queued pages, up to [`UPLOAD_BYTES`] (at least one); how many.
    pub fn upload(&mut self, queue: &wgpu::Queue) -> usize {
        let (Some(grid), Some((texture, _, _))) = (self.grid, &self.texture) else {
            return 0;
        };
        let mut bytes = 0;
        let mut uploaded = 0;
        while bytes < UPLOAD_BYTES
            && let Some((index, page)) = self.queue.pop_front()
        {
            let [si, sj] = grid.slot(index);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: si * PAGE_TEXELS,
                        y: sj * PAGE_TEXELS,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &page.texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(PAGE_TEXELS * 4),
                    rows_per_image: Some(PAGE_TEXELS),
                },
                wgpu::Extent3d {
                    width: PAGE_TEXELS,
                    height: PAGE_TEXELS,
                    depth_or_array_layers: 1,
                },
            );
            self.held[(sj * grid.slots + si) as usize] = Some(index);
            bytes += page.bytes();
            uploaded += 1;
        }
        uploaded
    }

    /// The trees' view's entries: page (0, 0)'s chart corner, the texel and the slots (0 with none, or hidden); the
    /// page each slot holds.
    pub fn uniforms(&self, canopy: &mut [f32; 4], slots: &mut [[i32; 4]; 32]) {
        for entry in slots.iter_mut() {
            *entry = [NO_PAGE; 4];
        }
        let Some(grid) = self.grid.filter(|_| self.shown) else {
            *canopy = [0.0; 4];
            return;
        };
        *canopy = [
            grid.origin[0] as f32,
            grid.origin[1] as f32,
            grid.texel_m as f32,
            grid.slots as f32,
        ];
        for (k, page) in self.held.iter().enumerate() {
            if let Some([i, j]) = page {
                let half = (k % 2) * 2;
                slots[k / 2][half] = *i;
                slots[k / 2][half + 1] = *j;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of `cells`-cell texels with a margin of 2, every cell `ground` but those `put`.
    fn grid(cells: usize, ground: u8, put: &[(usize, usize, u8)]) -> Vec<u8> {
        let width = PAGE_TEXELS as usize * cells + 4;
        let mut g = vec![ground; width * width];
        for &(x, y, h) in put {
            g[(y + 2) * width + x + 2] = h;
        }
        g
    }

    #[test]
    fn a_crown_is_the_tallest_cell_about_it() {
        // Two crowns of a tree: the 12 m apex and a 9 m shoulder a cell off it, then a 7 m tree 5 cells away, and a
        // 2 m shrub. 4-cell (8 m with 2 m cells) texels.
        let page = CanopyPage::from_heights(
            &grid(
                4,
                0,
                &[(9, 6, 12), (10, 6, 9), (8, 7, 10), (15, 6, 7), (20, 20, 2)],
            ),
            4,
            2,
        );
        let crowns = page.crowns(8.0);
        assert_eq!(crowns.len(), 2, "{crowns:?}");
        // The apex in cell (9, 6): texel (2, 1), its cell (1, 2) there, whose middle is (1.5, 2.5) cells, 3 m and 5 m in.
        assert_eq!(crowns[0].height, 12);
        assert!((crowns[0].at[0] - 19.0).abs() < 0.3, "{:?}", crowns[0]);
        assert!((crowns[0].at[1] - 13.0).abs() < 0.3, "{:?}", crowns[0]);
        assert_eq!(crowns[1].height, 7);
        // The shrub covers its texel's ground but stands no tree; the bare ground is known, with no cover.
        let (h, _, cover, known) = page.texel(5, 5);
        assert_eq!((h, known), (0, true));
        assert!((cover - 1.0 / 16.0).abs() < 0.01, "{cover}");
        let (h, _, cover, known) = page.texel(100, 100);
        assert_eq!((h, cover, known), (0, 0.0, true));
    }

    #[test]
    fn a_plateau_has_one_crown_and_the_unknown_is_left_to_the_cover() {
        // A flat-topped crown of 3 x 3 cells at 15 m.
        let mut put = Vec::new();
        for y in 40..43 {
            for x in 40..43 {
                put.push((x, y, 15));
            }
        }
        let page = CanopyPage::from_heights(&grid(4, 1, &put), 4, 2);
        let crowns = page.crowns(8.0);
        assert_eq!(crowns.len(), 1, "{crowns:?}");
        assert_eq!(crowns[0].height, 15);
        // Unknown all over: no texel known, so the land cover's trees stand.
        let page = CanopyPage::from_heights(&grid(4, UNKNOWN, &[]), 4, 2);
        assert!(!page.known());
        assert_eq!(page, CanopyPage::unknown());
    }

    #[test]
    fn a_closed_canopys_flat_top_has_a_crown_every_few_cells() {
        // 24 x 24 cells at 15 m, in whole texels of 4 cells: a crown every other cell, in every texel.
        let mut put = Vec::new();
        for y in 8..32 {
            for x in 8..32 {
                put.push((x, y, 15));
            }
        }
        let page = CanopyPage::from_heights(&grid(4, 0, &put), 4, 2);
        let crowns = page.crowns(8.0);
        assert_eq!(crowns.len(), 36, "{crowns:?}");
        for ty in 2..8 {
            for tx in 2..8 {
                assert_eq!(page.texel(tx, ty).0, 15, "texel ({tx}, {ty})");
            }
        }
    }

    #[test]
    fn a_crown_at_the_pages_edge_sees_its_neighbours_cells() {
        // A 10 m cell on the page's west edge, beside a taller one in the margin (the next page's): no crown here.
        let width = PAGE_TEXELS as usize * 2 + 4;
        let mut g = vec![0u8; width * width];
        g[(2 + 50) * width + 2] = 10;
        g[(2 + 50) * width + 1] = 14;
        let page = CanopyPage::from_heights(&g, 2, 2);
        assert!(page.crowns(4.0).is_empty());
        g[(2 + 50) * width + 1] = 6;
        let page = CanopyPage::from_heights(&g, 2, 2);
        assert_eq!(page.crowns(4.0).len(), 1);
    }
}
