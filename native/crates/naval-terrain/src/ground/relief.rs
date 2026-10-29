//! Fine relief near the camera (native/docs/terrain/ground.md, "Fine relief near the camera"): what a finer elevation
//! model
//! than the window's holds and the window's grid does not, as pages of residual heights about the camera that the land's
//! vertices add to the window's heights and its fragments to its slope. A host makes the pages; the layer holds them,
//! uploads a few a frame and draws them (`TerrainLayer::set_relief_grid`).
//!
//! - **What a texel holds.** R16Float: the finer model's height there less the window grid's own bilinear surface
//!   through the finer model's heights at the grid's samples (m). Beside it (Rg8Snorm, binding 17) that residual's
//!   slope (∂/∂x, ∂/∂z) at each level, worked out with the page ([`encode_slope`]): the land's fragments read one filtered
//!   texel for the relief's shading, where central differences of the heights took four. The window's heights plus it are the finer model's
//!   wherever the two models agree at the samples, and the window's own heights wherever they do not (no step where the
//!   pages end, no shift of the land's broad shape): the host takes the window's sample lattice for it, so pages are
//!   made again for every window cut.
//! - **Pages.** [`PAGE_TEXELS`]² texels: [`CONTENT`]² of their own about which lies a [`GUTTER`] of the neighbouring
//!   pages' texels, so filtering at a page's edge (and its two coarser levels', [`LEVELS`]) reads what lies beyond it.
//!   Pages are fixed squares of the host's metres (page (i, j) from (i, j) × its side), held `slots`² about the camera
//!   in one array texture, page (i, j) in layer (j mod slots) × slots + (i mod slots); the land reads a layer only where
//!   the uniform's entry names the page there, fading out over the outer [`FEATHER`] of a page whose neighbour is not
//!   held.
//! - **The waterline.** The land adds none of it at or below the sea, all of it from `RELIEF_SHORE_M` up, and never
//!   lowers land below a share of its window height (`bindings.wgsl`'s `with_relief`): the bake's coast stays where it
//!   is drawn.

use std::collections::VecDeque;
use std::sync::Arc;

use half::f16;

/// Texels on a page's side, its gutters included.
pub const PAGE_TEXELS: u32 = 512;
/// Texels of the neighbouring pages about a page's own.
pub const GUTTER: u32 = 4;
/// A page's own texels a side.
pub const CONTENT: u32 = PAGE_TEXELS - 2 * GUTTER;
/// Levels a page holds (its own and two coarser, each half the last's side): the gutter covers a texel of the coarsest.
pub const LEVELS: u32 = 3;
/// Most slots a side (the uniform holds 64 entries).
pub const MAX_SLOTS: u32 = 8;
/// The share of a page over which the relief fades out toward a neighbour not held.
pub const FEATHER: f64 = 0.15;
/// Bytes of pages uploaded in a frame at most (at least one page): 1.4 MB a page, heights and slopes.
pub const UPLOAD_BYTES: usize = 2 << 20;
/// What the land reads for a slot holding no page.
pub const NO_PAGE: i32 = i32::MIN;
/// The heights' format and their slopes'.
const HEIGHTS: wgpu::TextureFormat = wgpu::TextureFormat::R16Float;
const SLOPES: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Snorm;

const _: () = assert!(GUTTER >= 1 << (LEVELS - 1));

/// The pages' grid on a land layer: the chart corner of page (0, 0)'s own texels, a texel (m), and the slots a side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReliefGrid {
    pub origin: [f64; 2],
    pub texel_m: f64,
    pub slots: u32,
}

impl ReliefGrid {
    /// A page's side, metres (its own texels').
    pub fn page_m(&self) -> f64 {
        self.texel_m * f64::from(CONTENT)
    }

    /// The layer page `index` takes.
    pub fn layer(&self, index: [i32; 2]) -> u32 {
        let n = self.slots as i32;
        (index[1].rem_euclid(n) * n + index[0].rem_euclid(n)) as u32
    }

    /// Page `index`'s own texels on the chart (x0, z0, x1, z1).
    pub fn rect(&self, index: [i32; 2]) -> [f64; 4] {
        let side = self.page_m();
        let x0 = self.origin[0] + f64::from(index[0]) * side;
        let z0 = self.origin[1] + f64::from(index[1]) * side;
        [x0, z0, x0 + side, z0 + side]
    }

    /// The chart point at the middle of texel (i, j) of page `index`, its gutters counted (texel `GUTTER` is its first
    /// own).
    pub fn texel_middle(&self, index: [i32; 2], i: u32, j: u32) -> [f64; 2] {
        let [x0, z0, _, _] = self.rect(index);
        [
            x0 + (f64::from(i) - f64::from(GUTTER) + 0.5) * self.texel_m,
            z0 + (f64::from(j) - f64::from(GUTTER) + 0.5) * self.texel_m,
        ]
    }
}

/// One page's levels (f16 bits, row 0 at the smallest z; level `k` of side `PAGE_TEXELS >> k`), each level's slopes
/// (two [`encode_slope`] bytes a texel, ∂/∂x then ∂/∂z), and the largest residual on its own texels (m), which the
/// selection widens a patch's bounds by.
#[derive(Clone, Debug, PartialEq)]
pub struct ReliefPage {
    pub levels: Vec<Vec<u16>>,
    pub slopes: Vec<Vec<u8>>,
    pub peak: f32,
}

/// A slope (m/m) as a signed byte: `g / (1 + |g|)` (fine near level ground, where a step in the shading shows, and up
/// to a cliff's 50), which `bindings.wgsl`'s `relief_gradient` undoes after filtering.
pub fn encode_slope(g: f32) -> u8 {
    let c = if g.is_finite() {
        g / (1.0 + g.abs())
    } else {
        0.0
    };
    ((c * 127.0).round().clamp(-127.0, 127.0) as i8) as u8
}

/// [`encode_slope`]'s byte back to a slope (m/m), as the shader reads it.
pub fn decode_slope(byte: u8) -> f32 {
    let c = (f32::from(byte as i8) / 127.0).clamp(-0.98, 0.98);
    c / (1.0 - c.abs())
}

/// The slopes of a level's heights (`side`², `texel_m` apart): central differences, one-sided at the edges.
fn slopes_of(heights: &[f32], side: usize, texel_m: f32) -> Vec<u8> {
    let at = |i: usize, j: usize| heights[j * side + i];
    let mut out = Vec::with_capacity(side * side * 2);
    for j in 0..side {
        let (j0, j1) = (j.saturating_sub(1), (j + 1).min(side - 1));
        for i in 0..side {
            let (i0, i1) = (i.saturating_sub(1), (i + 1).min(side - 1));
            let gx = (at(i1, j) - at(i0, j)) / ((i1 - i0) as f32 * texel_m);
            let gz = (at(i, j1) - at(i, j0)) / ((j1 - j0) as f32 * texel_m);
            out.push(encode_slope(gx));
            out.push(encode_slope(gz));
        }
    }
    out
}

impl ReliefPage {
    /// The page from its residuals (m; `PAGE_TEXELS`² with its gutters, `texel_m` apart, row 0 at the smallest z; a
    /// value not finite counts as none), its coarser levels each the mean of 2 × 2 of the last's, and each level's
    /// slopes.
    pub fn from_residual(texels: &[f32], texel_m: f32) -> Self {
        let n = PAGE_TEXELS as usize;
        assert_eq!(texels.len(), n * n, "a relief page of the wrong size");
        let mut current: Vec<f32> = texels
            .iter()
            .map(|&v| if v.is_finite() { v } else { 0.0 })
            .collect();
        let g = GUTTER as usize;
        let mut peak = 0f32;
        for j in g..n - g {
            for &v in &current[j * n + g..j * n + n - g] {
                peak = peak.max(v.abs());
            }
        }
        let mut levels = vec![
            current
                .iter()
                .map(|&v| f16::from_f32(v).to_bits())
                .collect(),
        ];
        let mut slopes = vec![slopes_of(&current, n, texel_m)];
        let mut side = n;
        for level in 1..LEVELS {
            let half = side / 2;
            let mut next = vec![0f32; half * half];
            for j in 0..half {
                for i in 0..half {
                    let at = |x: usize, y: usize| current[y * side + x];
                    next[j * half + i] = 0.25
                        * (at(2 * i, 2 * j)
                            + at(2 * i + 1, 2 * j)
                            + at(2 * i, 2 * j + 1)
                            + at(2 * i + 1, 2 * j + 1));
                }
            }
            levels.push(next.iter().map(|&v| f16::from_f32(v).to_bits()).collect());
            slopes.push(slopes_of(&next, half, texel_m * (1 << level) as f32));
            current = next;
            side = half;
        }
        Self {
            levels,
            slopes,
            peak,
        }
    }

    /// A page of no relief.
    pub fn flat() -> Self {
        Self::from_residual(&vec![0.0; (PAGE_TEXELS * PAGE_TEXELS) as usize], 1.0)
    }

    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.len() * 2).sum::<usize>()
            + self.slopes.iter().map(Vec::len).sum::<usize>()
    }

    /// The slope (∂/∂x, ∂/∂z) of level `level` at its texel (i, j) (gutters counted), m/m.
    pub fn slope(&self, level: usize, i: u32, j: u32) -> [f32; 2] {
        let at = 2 * (j * (PAGE_TEXELS >> level) + i) as usize;
        [
            decode_slope(self.slopes[level][at]),
            decode_slope(self.slopes[level][at + 1]),
        ]
    }

    /// Texel (i, j) of level 0 (gutters counted), metres.
    pub fn at(&self, i: u32, j: u32) -> f32 {
        f16::from_bits(self.levels[0][(j * PAGE_TEXELS + i) as usize]).to_f32()
    }
}

/// The land layer's relief: an R16Float array texture of `slots`² layers (the heights, bound at the land's binding 16)
/// and an Rg8Snorm one beside it (their slopes, binding 17).
pub(crate) struct ReliefTextures {
    placeholder: (wgpu::TextureView, wgpu::TextureView),
    texture: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
    slopes: Option<(wgpu::Texture, wgpu::TextureView)>,
    grid: Option<ReliefGrid>,
    /// Per layer, the page it holds (kept for [`ReliefHeights`]: the host's own, shared).
    held: Vec<Option<([i32; 2], Arc<ReliefPage>)>>,
    queue: VecDeque<([i32; 2], Arc<ReliefPage>)>,
    /// Drawn (the default); hidden, the land is drawn with none (for comparisons).
    pub shown: bool,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    size: u32,
    layers: u32,
    levels: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

impl ReliefTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        let heights = texture(device, "terrain relief placeholder", HEIGHTS, 1, 1, 1);
        let slopes = texture(device, "terrain relief slopes placeholder", SLOPES, 1, 1, 1);
        Self {
            placeholder: (array_view(&heights), array_view(&slopes)),
            texture: None,
            slopes: None,
            grid: None,
            held: Vec::new(),
            queue: VecDeque::new(),
            shown: true,
        }
    }

    /// The views bindings 16 (the heights) and 17 (their slopes) take.
    pub fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        match (&self.texture, &self.slopes) {
            (Some((_, heights, _)), Some((_, slopes))) => (heights, slopes),
            _ => (&self.placeholder.0, &self.placeholder.1),
        }
    }

    /// Takes pages on `grid`; true when the view changed.
    pub fn set_grid(&mut self, device: &wgpu::Device, grid: ReliefGrid) -> Result<bool, String> {
        if grid.slots == 0 || grid.slots > MAX_SLOTS || grid.texel_m <= 0.0 {
            return Err(format!(
                "the relief: {} a side of {} m texels",
                grid.slots, grid.texel_m
            ));
        }
        let layers = grid.slots * grid.slots;
        let limit = device.limits().max_texture_array_layers;
        if layers > limit {
            return Err(format!(
                "the relief: {layers} pages, more than this device's {limit} layers"
            ));
        }
        if self.grid == Some(grid) {
            return Ok(false);
        }
        self.grid = Some(grid);
        self.held = vec![None; layers as usize];
        self.queue.clear();
        if self
            .texture
            .as_ref()
            .is_some_and(|(_, _, n)| *n == grid.slots)
        {
            return Ok(false);
        }
        let made = texture(
            device,
            "terrain relief",
            HEIGHTS,
            PAGE_TEXELS,
            layers,
            LEVELS,
        );
        let view = array_view(&made);
        self.texture = Some((made, view, grid.slots));
        let made = texture(
            device,
            "terrain relief slopes",
            SLOPES,
            PAGE_TEXELS,
            layers,
            LEVELS,
        );
        let view = array_view(&made);
        self.slopes = Some((made, view));
        Ok(true)
    }

    /// No pages: true when the view changed.
    pub fn clear(&mut self) -> bool {
        self.grid = None;
        self.queue.clear();
        self.held.clear();
        self.slopes = None;
        self.texture.take().is_some()
    }

    pub fn grid(&self) -> Option<ReliefGrid> {
        self.grid
    }

    /// Whether page `index` is drawn or waiting for its upload.
    pub fn holds(&self, index: [i32; 2]) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        self.held
            .get(grid.layer(index) as usize)
            .is_some_and(|h| h.as_ref().is_some_and(|(k, _)| *k == index))
            || self.queue.iter().any(|(k, _)| *k == index)
    }

    /// Pages drawn, and waiting for their upload.
    pub fn counts(&self) -> (usize, usize) {
        (self.held.iter().flatten().count(), self.queue.len())
    }

    /// Queues page `index` for its upload (a page queued for the same layer before is dropped).
    pub fn queue(&mut self, index: [i32; 2], page: Arc<ReliefPage>) {
        let Some(grid) = self.grid else {
            return;
        };
        let texels = |k: usize| ((PAGE_TEXELS >> k) * (PAGE_TEXELS >> k)) as usize;
        if page.levels.len() != LEVELS as usize
            || page.slopes.len() != LEVELS as usize
            || page
                .levels
                .iter()
                .enumerate()
                .any(|(k, l)| l.len() != texels(k))
            || page
                .slopes
                .iter()
                .enumerate()
                .any(|(k, l)| l.len() != 2 * texels(k))
        {
            return;
        }
        let layer = grid.layer(index);
        self.queue
            .retain(|(k, _)| *k == index || grid.layer(*k) != layer);
        if !self.queue.iter().any(|(k, _)| *k == index) {
            self.queue.push_back((index, page));
        }
    }

    /// Uploads queued pages, up to [`UPLOAD_BYTES`] (at least one); how many. `changed` takes the chart rectangles of
    /// the pages that came into their layers or left them, each with its neighbours' [`FEATHER`] about it (where the
    /// relief's share changes as a neighbour comes or goes).
    pub fn upload(&mut self, queue: &wgpu::Queue, changed: &mut Vec<[f64; 4]>) -> usize {
        let (Some(grid), Some((texture, _, _)), Some((slopes, _))) =
            (self.grid, &self.texture, &self.slopes)
        else {
            return 0;
        };
        let reach = grid.page_m() * FEATHER;
        let grown = |[x0, z0, x1, z1]: [f64; 4]| [x0 - reach, z0 - reach, x1 + reach, z1 + reach];
        let mut bytes = 0;
        let mut uploaded = 0;
        while bytes < UPLOAD_BYTES
            && let Some((index, page)) = self.queue.pop_front()
        {
            let layer = grid.layer(index);
            let levels = page
                .levels
                .iter()
                .map(|l| (texture, bytemuck::cast_slice::<u16, u8>(l)));
            let slope_levels = page.slopes.iter().map(|l| (slopes, l.as_slice()));
            for (k, (texture, texels)) in levels.chain(slope_levels).enumerate() {
                let level = k % LEVELS as usize;
                let side = PAGE_TEXELS >> level;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    texels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * 2),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
            }
            if let Some((old, _)) = &self.held[layer as usize] {
                changed.push(grown(grid.rect(*old)));
            }
            changed.push(grown(grid.rect(index)));
            bytes += page.bytes();
            self.held[layer as usize] = Some((index, page));
            uploaded += 1;
        }
        uploaded
    }

    /// The most any page drawn lifts or lowers the land (m; 0 with none, or hidden).
    pub fn peak(&self) -> f32 {
        if !self.shown {
            return 0.0;
        }
        self.held
            .iter()
            .flatten()
            .fold(0.0, |a: f32, (_, p)| a.max(p.peak))
    }

    /// The chart rectangle (x0, z0, x1, z1) about every page drawn; none with none, or hidden.
    pub fn extent(&self) -> Option<[f64; 4]> {
        let grid = self.grid.filter(|_| self.shown)?;
        self.held.iter().flatten().fold(None, |acc, (index, _)| {
            let r = grid.rect(*index);
            Some(acc.map_or(r, |a: [f64; 4]| {
                [
                    a[0].min(r[0]),
                    a[1].min(r[1]),
                    a[2].max(r[2]),
                    a[3].max(r[3]),
                ]
            }))
        })
    }

    /// The pages drawn, for the land's heights on the CPU ([`ReliefHeights`]); none hidden.
    pub fn heights(&self) -> ReliefHeights {
        match self.grid.filter(|_| self.shown) {
            Some(grid) => ReliefHeights {
                grid: Some(grid),
                held: self.held.clone(),
            },
            None => ReliefHeights::default(),
        }
    }

    /// The land's entries: page (0, 0)'s chart corner, the texel and the slots (0 with none, or hidden); the page each
    /// layer holds, two an entry (layer k at entry k / 2, .xy when k is even, .zw when odd).
    pub fn uniforms(&self, relief: &mut [f32; 4], slots: &mut [[i32; 4]; 32]) {
        for entry in slots.iter_mut() {
            *entry = [NO_PAGE; 4];
        }
        let Some(grid) = self.grid.filter(|_| self.shown) else {
            *relief = [0.0; 4];
            return;
        };
        *relief = [
            grid.origin[0] as f32,
            grid.origin[1] as f32,
            grid.texel_m as f32,
            grid.slots as f32,
        ];
        for (k, page) in self.held.iter().enumerate() {
            if let Some(([i, j], _)) = page {
                let entry = &mut slots[k / 2];
                let at = (k % 2) * 2;
                entry[at] = *i;
                entry[at + 1] = *j;
            }
        }
    }
}

/// The land's heights as drawn with the relief, on the CPU (`bindings.wgsl`'s `relief_height`, read at level 0, the
/// finest any land is drawn at): what keeps a camera clear of a crest the relief lifts. A snapshot of the pages drawn
/// (`TerrainLayer::relief_heights`), cheap to take each frame: it shares them.
#[derive(Clone, Default)]
pub struct ReliefHeights {
    grid: Option<ReliefGrid>,
    held: Vec<Option<([i32; 2], Arc<ReliefPage>)>>,
}

impl ReliefHeights {
    fn page(&self, grid: ReliefGrid, index: [i32; 2]) -> Option<&ReliefPage> {
        match self.held.get(grid.layer(index) as usize) {
            Some(Some((k, page))) if *k == index => Some(page),
            _ => None,
        }
    }

    /// A number for the pages drawn under chart rectangle `lo`-`hi` and a page about it (whose feather reaches in), or
    /// under all of them with none: each page by its place and its contents, so it changes when one comes in, goes or is
    /// made again. 0 where none is drawn.
    pub fn signature(&self, within: Option<([f64; 2], [f64; 2])>) -> u64 {
        let Some(grid) = self.grid else {
            return 0;
        };
        let side = grid.page_m();
        let index = |p: [f64; 2]| {
            [
                ((p[0] - grid.origin[0]) / side).floor() as i32,
                ((p[1] - grid.origin[1]) / side).floor() as i32,
            ]
        };
        let range = within.map(|(lo, hi)| {
            let (a, b) = (index(lo), index(hi));
            ([a[0] - 1, a[1] - 1], [b[0] + 1, b[1] + 1])
        });
        let mut sum = 0u64;
        for (k, page) in self.held.iter().flatten() {
            if range.is_some_and(|(a, b)| k[0] < a[0] || k[1] < a[1] || k[0] > b[0] || k[1] > b[1])
            {
                continue;
            }
            // Order-free: the pages' own numbers summed (a page's address names its contents while it is held).
            let mut h = (u64::from(k[0] as u32) << 32 | u64::from(k[1] as u32))
                ^ (Arc::as_ptr(page) as usize as u64).rotate_left(17);
            h ^= h >> 33;
            h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
            h ^= h >> 33;
            sum = sum.wrapping_add(h | 1);
        }
        sum
    }

    /// The land's height as drawn at chart point `xz` where the window's is `coarse` (m): `coarse` itself at or below
    /// the sea and off the pages.
    pub fn height(&self, xz: [f64; 2], coarse: f64) -> f64 {
        let Some(grid) = self.grid.filter(|_| coarse > 0.0) else {
            return coarse;
        };
        let t = [
            (xz[0] - grid.origin[0]) / grid.page_m(),
            (xz[1] - grid.origin[1]) / grid.page_m(),
        ];
        let index = [t[0].floor() as i32, t[1].floor() as i32];
        let Some(page) = self.page(grid, index) else {
            return coarse;
        };
        let f = [t[0] - t[0].floor(), t[1] - t[1].floor()];
        let smooth = |x: f64, edge: f64| {
            let u = (x / edge).clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        let mut share = 1.0;
        for (a, step) in [(0, [1, 0]), (1, [0, 1])] {
            let missing = |sign: i32| {
                self.page(grid, [index[0] + sign * step[0], index[1] + sign * step[1]])
                    .is_none()
            };
            if f[a] < FEATHER && missing(-1) {
                share *= smooth(f[a], FEATHER);
            }
            if f[a] > 1.0 - FEATHER && missing(1) {
                share *= smooth(1.0 - f[a], FEATHER);
            }
        }
        // Level 0's bilinear filter (texel centres at k + 0.5, the gutters counted), clamped at the page's edge.
        let n = PAGE_TEXELS as usize;
        let g = f.map(|f| f * f64::from(CONTENT) + f64::from(GUTTER) - 0.5);
        let i = g.map(|g| (g.floor().max(0.0) as usize).min(n - 2));
        let u = [0, 1].map(|a| (g[a] - i[a] as f64).clamp(0.0, 1.0));
        let at = |di: usize, dj: usize| f64::from(page.at((i[0] + di) as u32, (i[1] + dj) as u32));
        let r = (at(0, 0) * (1.0 - u[0]) + at(1, 0) * u[0]) * (1.0 - u[1])
            + (at(0, 1) * (1.0 - u[0]) + at(1, 1) * u[0]) * u[1];
        let r = if r.is_finite() { r } else { 0.0 };
        (coarse + smooth(coarse, RELIEF_SHORE_M) * share * r).max(coarse * RELIEF_FLOOR)
    }
}

/// `bindings.wgsl`'s `RELIEF_SHORE_M` and `RELIEF_FLOOR`.
const RELIEF_SHORE_M: f64 = 4.0;
const RELIEF_FLOOR: f64 = 0.15;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_keeps_its_residuals_and_averages_its_levels() {
        let n = PAGE_TEXELS as usize;
        let texels: Vec<f32> = (0..n * n)
            .map(|k| ((k % n) as f32 * 0.25) - ((k / n) as f32 * 0.125))
            .collect();
        let page = ReliefPage::from_residual(&texels, 4.0);
        assert_eq!(page.levels.len(), LEVELS as usize);
        assert_eq!(page.levels[1].len(), (n / 2) * (n / 2));
        assert_eq!(page.levels[2].len(), (n / 4) * (n / 4));
        assert!((page.at(10, 20) - (2.5 - 2.5)).abs() < 1e-3);
        // Level 1's texel (0, 0) is the mean of level 0's first 2 x 2: (0 + 0.25 - 0.125 + 0.125) / 4.
        let first = f16::from_bits(page.levels[1][0]).to_f32();
        assert!((first - 0.0625).abs() < 1e-3, "{first}");
        // The peak counts the page's own texels only.
        let g = GUTTER as usize;
        let texels = &texels;
        let own = (g..n - g)
            .flat_map(|j| (g..n - g).map(move |i| texels[j * n + i].abs()))
            .fold(0f32, f32::max);
        assert!((page.peak - own).abs() < 1e-6);
    }

    #[test]
    fn a_value_not_finite_is_no_relief() {
        let n = PAGE_TEXELS as usize;
        let mut texels = vec![1.0f32; n * n];
        texels[n * 40 + 40] = f32::NAN;
        let page = ReliefPage::from_residual(&texels, 4.0);
        assert_eq!(page.at(40, 40), 0.0);
        assert_eq!(page.at(41, 40), 1.0);
    }

    #[test]
    fn each_level_holds_its_slopes() {
        // A plane rising 0.25 m a texel east (1/16 at 4 m) and falling 0.125 south (-1/32), and a 3 m step across
        // column 200: a cliff of 3/8 (central differences over two texels).
        let n = PAGE_TEXELS as usize;
        let texels: Vec<f32> = (0..n * n)
            .map(|k| {
                let (i, j) = (k % n, k / n);
                i as f32 * 0.25 - j as f32 * 0.125 + if i >= 200 { 3.0 } else { 0.0 }
            })
            .collect();
        let page = ReliefPage::from_residual(&texels, 4.0);
        assert_eq!(page.slopes.len(), LEVELS as usize);
        assert_eq!(page.slopes[2].len(), 2 * (n / 4) * (n / 4));
        let close = |a: [f32; 2], b: [f32; 2], tolerance: f32| {
            (a[0] - b[0]).abs() < tolerance && (a[1] - b[1]).abs() < tolerance
        };
        // Half the byte's step near level ground is under 0.005.
        assert!(close(
            page.slope(0, 100, 100),
            [1.0 / 16.0, -1.0 / 32.0],
            0.005
        ));
        assert!(close(
            page.slope(0, 200, 100),
            [1.0 / 16.0 + 3.0 / 8.0, -1.0 / 32.0],
            0.01
        ));
        // The edges take one-sided differences; the coarser levels the same plane over their wider texels.
        assert!(close(page.slope(0, 0, 0), [1.0 / 16.0, -1.0 / 32.0], 0.005));
        assert!(close(
            page.slope(2, 10, 10),
            [1.0 / 16.0, -1.0 / 32.0],
            0.005
        ));
        // Steep slopes keep their size within a few per cent, up to a cliff's 50.
        for g in [-40.0f32, -3.0, 0.7, 12.0] {
            let back = decode_slope(encode_slope(g));
            assert!((back - g).abs() < 0.12 * g.abs(), "{g} {back}");
        }
        assert_eq!(decode_slope(encode_slope(0.0)), 0.0);
    }

    #[test]
    fn pages_take_their_layers_and_texels_their_places() {
        let grid = ReliefGrid {
            origin: [-1000.0, 500.0],
            texel_m: 4.0,
            slots: 8,
        };
        assert_eq!(grid.page_m(), 2016.0);
        assert_eq!(grid.layer([0, 0]), 0);
        assert_eq!(grid.layer([9, 1]), 9);
        assert_eq!(grid.layer([-1, -1]), 63);
        assert_eq!(grid.rect([1, 0]), [1016.0, 500.0, 3032.0, 2516.0]);
        // Texel GUTTER is the page's first own: its middle half a texel inside the page's corner.
        assert_eq!(grid.texel_middle([0, 0], GUTTER, GUTTER), [-998.0, 502.0]);
        // The gutter's last texel is the neighbour's last own one.
        let left = grid.texel_middle([0, 0], GUTTER - 1, GUTTER);
        let neighbour = grid.texel_middle([-1, 0], PAGE_TEXELS - GUTTER - 1, GUTTER);
        assert_eq!(left, neighbour);
    }

    #[test]
    fn the_heights_on_the_cpu_are_the_lands_as_drawn() {
        let grid = ReliefGrid {
            origin: [0.0, 0.0],
            texel_m: 4.0,
            slots: 2,
        };
        let n = PAGE_TEXELS as usize;
        let lift = Arc::new(ReliefPage::from_residual(&vec![10.0; n * n], 4.0));
        let mut held = vec![None; 4];
        held[grid.layer([0, 0]) as usize] = Some(([0, 0], lift.clone()));
        held[grid.layer([1, 0]) as usize] = Some(([1, 0], lift));
        let heights = ReliefHeights {
            grid: Some(grid),
            held,
        };
        let side = grid.page_m();
        let middle = [side / 2.0, side / 2.0];
        // Inland the whole residual; none at or below the sea, a share on the shore, none off the pages.
        assert!((heights.height(middle, 50.0) - 60.0).abs() < 1e-3);
        assert_eq!(heights.height(middle, 0.0), 0.0);
        assert_eq!(heights.height(middle, -3.0), -3.0);
        assert!((heights.height(middle, 2.0) - 7.0).abs() < 1e-3);
        assert_eq!(heights.height([side / 2.0, -side / 2.0], 50.0), 50.0);
        // Where page (0, 0) meets page (1, 0), both held: all of it. Toward page (0, 1), not held: fading to none.
        assert!((heights.height([side, side / 2.0], 50.0) - 60.0).abs() < 1e-3);
        let edge = heights.height([side / 2.0, side * (1.0 - FEATHER / 2.0)], 50.0);
        assert!((edge - 55.0).abs() < 1e-3, "{edge}");
        assert!((heights.height([side / 2.0, side - 1e-6], 50.0) - 50.0).abs() < 1e-3);
        // Lowered no further than the floor's share of the window's height.
        let sink = Arc::new(ReliefPage::from_residual(&vec![-100.0; n * n], 4.0));
        let mut held = vec![None; 4];
        held[0] = Some(([0, 0], sink));
        let sunk = ReliefHeights {
            grid: Some(grid),
            held,
        };
        assert!((sunk.height(middle, 50.0) - 50.0 * RELIEF_FLOOR).abs() < 1e-3);
        // None held: the window's own.
        assert_eq!(ReliefHeights::default().height(middle, 50.0), 50.0);
    }

    #[test]
    fn the_pages_signature_changes_with_the_pages_under_a_rectangle() {
        let grid = ReliefGrid {
            origin: [0.0, 0.0],
            texel_m: 4.0,
            slots: 8,
        };
        let n = PAGE_TEXELS as usize;
        let page = Arc::new(ReliefPage::from_residual(&vec![1.0; n * n], 4.0));
        let with = |pages: &[([i32; 2], &Arc<ReliefPage>)]| {
            let mut held = vec![None; 64];
            for (k, p) in pages {
                held[grid.layer(*k) as usize] = Some((*k, (*p).clone()));
            }
            ReliefHeights {
                grid: Some(grid),
                held,
            }
        };
        let chunk = Some(([100.0, 100.0], [1100.0, 1100.0]));
        assert_eq!(ReliefHeights::default().signature(chunk), 0);
        assert_eq!(with(&[]).signature(chunk), 0);
        let one = with(&[([0, 0], &page)]);
        assert_ne!(one.signature(chunk), 0);
        // A neighbour coming in changes it (its feather reaches in); a page two away does not, but counts in the whole.
        let near = with(&[([0, 0], &page), ([1, 0], &page)]);
        let far = with(&[([0, 0], &page), ([3, 0], &page)]);
        assert_ne!(near.signature(chunk), one.signature(chunk));
        assert_eq!(far.signature(chunk), one.signature(chunk));
        assert_ne!(far.signature(None), one.signature(None));
        // The same place made again: another page.
        let again = Arc::new(ReliefPage::from_residual(&vec![1.0; n * n], 4.0));
        assert_ne!(
            with(&[([0, 0], &again)]).signature(chunk),
            one.signature(chunk)
        );
    }
}
