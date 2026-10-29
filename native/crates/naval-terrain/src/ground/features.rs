//! Map features drawn on the ground (native/docs/terrain/surface.md, "Map features on the ground"): roads, railways,
//! rivers and
//! canals as signed distance fields on pages of fine texels about the camera, so they stay crisp however close the
//! camera comes, at one texture read a level where they are drawn.
//!
//! - **What a texel holds.** Two RGBA8 layers a level. The first, four distances in metres to the nearest feature of
//!   each kind ([`StrokeKind`]): paved roads' and unpaved roads' edges (the distance to the road's middle less its half
//!   width: negative on the road), railways' middle lines, water's edges (rivers and canals as lines with a half width,
//!   riverbanks as areas: negative inside); each encoded `0.5 + d / (2 R)` over the level's range `R`
//!   ([`RANGE_TEXELS`] texels). Bilinear filtering of a distance field gives its edge back at a fraction of a texel, so a
//!   7 m road on 2 m texels is a sharp-edged band at a metre from it and a filtered line at 20 km. The second layer,
//!   what the shading needs of the nearest feature: the paved road's half width and tone (its class), the unpaved
//!   road's half width, the water's half width (an area's is the most, [`WATER_HALF_WIDTH_MAX_M`]).
//! - **Pages.** [`PAGE_TEXELS`]² texels, fixed squares of the host's metres (page (i, j) of level `l` from (i, j) ×
//!   its side), so one made once serves every scene that comes back to it. A level holds `slots`² pages about the
//!   camera in one texture array's two layers, page (i, j) in slot (i mod slots, j mod slots) read repeating, as the
//!   near tiles are ([`FeatureGrid`]); the shaders draw a slot only where its uniform entry names the page there.
//! - **Made on the host's thread** ([`FeaturePage::rasterize`]: the strokes' segments as capsules, the areas by
//!   scanline), uploaded a few a frame by the layer's `prepare`.

use std::collections::VecDeque;
use std::sync::Arc;

/// Texels on a page's side.
pub const PAGE_TEXELS: u32 = 256;
/// Levels of pages: fine about the camera, coarse farther out.
pub const LEVELS: usize = 2;
/// Most slots a side (the uniforms hold 64 a level).
pub const MAX_SLOTS: u32 = 8;
/// The distances' range either side of a feature, texels: farther is stored as the range.
pub const RANGE_TEXELS: f64 = 8.0;
/// A road's half width as the second layer stores it: 0 to this many metres.
pub const HALF_WIDTH_MAX_M: f32 = 16.0;
/// Water's half width as the second layer stores it (an area: this).
pub const WATER_HALF_WIDTH_MAX_M: f32 = 32.0;
/// Bytes of pages uploaded in a frame at most (at least one page): 512 KB a page, so eight a frame.
pub const UPLOAD_BYTES: usize = 4 << 20;
/// What the shaders read for a slot holding no page.
pub const NO_PAGE: i32 = i32::MIN;

/// What a stroke is drawn as: one distance channel each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StrokeKind {
    /// Asphalt and concrete roads.
    Paved = 0,
    /// Tracks and unpaved roads.
    Unpaved = 1,
    /// Railways (the distance is to the line's middle: the shader lays the ballast and rails about it).
    Rail = 2,
    /// Rivers, streams and canals.
    Water = 3,
}

/// A line feature: its kind, half width (m), tone (0-1, the class's shade, paved roads only) and points in the host's
/// metres (the frame the pages' corners are in).
#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub kind: StrokeKind,
    pub half_width: f32,
    pub tone: f32,
    pub points: Vec<[f64; 2]>,
}

/// A water area (a riverbank, a canal's basin): rings in the host's metres, inside by the even-odd rule (an exterior
/// and its holes).
#[derive(Clone, Debug, PartialEq)]
pub struct WaterArea {
    pub rings: Vec<Vec<[f64; 2]>>,
}

/// The pages' grid on a land layer: each level's chart corner of its page (0, 0), its texel (m), and the slots a side
/// both levels hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeatureGrid {
    pub origin: [[f64; 2]; LEVELS],
    pub texel_m: [f64; LEVELS],
    pub slots: u32,
}

impl FeatureGrid {
    /// A page's side at `level`, metres.
    pub fn page_m(&self, level: usize) -> f64 {
        self.texel_m[level] * f64::from(PAGE_TEXELS)
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

    /// The chart rectangle (x0, z0, x1, z1) whose shading page `index` of `level` can change by coming or going: the
    /// page, grown at level 1 by a page (it feathers into its neighbours, `features.wgsl`'s `FEATURE_FEATHER` of a page,
    /// and their filtered texels reach into it), at level 0 by two texels (the filter alone).
    pub fn reach(&self, level: usize, index: [i32; 2]) -> [f64; 4] {
        let side = self.page_m(level);
        let grow = if level == 0 {
            2.0 * self.texel_m[0]
        } else {
            side
        };
        let [x, z] = [
            self.origin[level][0] + f64::from(index[0]) * side,
            self.origin[level][1] + f64::from(index[1]) * side,
        ];
        [x - grow, z - grow, x + side + grow, z + side + grow]
    }
}

/// One page: its two layers' texels, RGBA8, row 0 at the smallest z.
#[derive(Clone, Debug, PartialEq)]
pub struct FeaturePage {
    pub edges: Vec<u8>,
    pub attrs: Vec<u8>,
    /// Nothing reaches the page (it draws nothing, but is uploaded all the same, so its slot is known drawn).
    pub empty: bool,
    /// The lights on it at night, page-local metres (`night`; level [`crate::night::LAMP_LEVEL`]'s pages only, the
    /// host's to make).
    pub lamps: Vec<crate::night::Lamp>,
}

/// The distance one byte stands for, given the level's range `range` (m).
pub fn decode_distance(byte: u8, range: f64) -> f64 {
    (f64::from(byte) / 255.0 - 0.5) * 2.0 * range
}

fn encode_distance(d: f64, range: f64) -> u8 {
    ((0.5 + d / (2.0 * range)).clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The interval of x on row `y` within `r` of the segment from `a` to `b` (a capsule is convex: one interval), none
/// when the row misses it.
fn capsule_span(a: [f64; 2], b: [f64; 2], r: f64, y: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in [a, b] {
        let dy = y - c[1];
        if dy.abs() <= r {
            let w = (r * r - dy * dy).sqrt();
            lo = lo.min(c[0] - w);
            hi = hi.max(c[0] + w);
        }
    }
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    if l2 > 0.0 {
        let l = l2.sqrt();
        let oy = y - a[1];
        // Along: 0 ≤ (x - a.x) dx + oy dy ≤ l²; across: |(x - a.x) dy - oy dx| ≤ r l. Each an interval of x (or all of
        // x, or none, when its coefficient is 0).
        let interval = |coef: f64, rest: f64, min: f64, max: f64| -> Option<(f64, f64)> {
            if coef.abs() < 1e-12 {
                return (min..=max)
                    .contains(&rest)
                    .then_some((f64::NEG_INFINITY, f64::INFINITY));
            }
            let (p, q) = ((min - rest) / coef, (max - rest) / coef);
            Some((a[0] + p.min(q), a[0] + p.max(q)))
        };
        if let (Some(along), Some(across)) = (
            interval(dx, oy * dy, 0.0, l2),
            interval(dy, -oy * dx, -r * l, r * l),
        ) {
            let (p, q) = (along.0.max(across.0), along.1.min(across.1));
            if p <= q {
                lo = lo.min(p);
                hi = hi.max(q);
            }
        }
    }
    (lo <= hi).then_some((lo, hi))
}

fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// Calls `f(i, j, distance in texels)` for every texel of an `n`² page within `r` texels of the segment from `a` to
/// `b` (texel coordinates: texel (i, j)'s middle at (i, j)).
fn each_near(n: usize, a: [f64; 2], b: [f64; 2], r: f64, mut f: impl FnMut(usize, usize, f64)) {
    let top = (a[1].min(b[1]) - r).ceil().max(0.0);
    let bottom = (a[1].max(b[1]) + r).floor().min(n as f64 - 1.0);
    if top > bottom {
        return;
    }
    for j in top as usize..=bottom as usize {
        let y = j as f64;
        let Some((x0, x1)) = capsule_span(a, b, r, y) else {
            continue;
        };
        let (x0, x1) = (x0.ceil().max(0.0), x1.floor().min(n as f64 - 1.0));
        if x0 > x1 {
            continue;
        }
        for i in x0 as usize..=x1 as usize {
            f(i, j, segment_distance([i as f64, y], a, b));
        }
    }
}

impl FeaturePage {
    pub fn bytes(&self) -> usize {
        self.edges.len() + self.attrs.len()
    }

    /// A page with nothing on it.
    pub fn empty() -> Self {
        let n = (PAGE_TEXELS * PAGE_TEXELS) as usize;
        Self {
            edges: vec![255; n * 4],
            attrs: vec![0; n * 4],
            empty: true,
            lamps: Vec::new(),
        }
    }

    /// The page whose corner is at `origin` (the host's metres), `texel_m` a texel: every stroke and area within the
    /// range of it drawn into its distances.
    pub fn rasterize(
        origin: [f64; 2],
        texel_m: f64,
        strokes: &[Stroke],
        areas: &[WaterArea],
    ) -> Self {
        let n = PAGE_TEXELS as usize;
        let range = RANGE_TEXELS * texel_m;
        let mut d = vec![[range; 4]; n * n];
        let mut half = vec![[0f32; 4]; n * n];
        let mut tone = vec![0f32; n * n];
        let mut touched = false;
        // Texel coordinates of a point: texel (i, j)'s middle is at origin + (i + 0.5, j + 0.5) texels.
        let local = |p: [f64; 2]| {
            [
                (p[0] - origin[0]) / texel_m - 0.5,
                (p[1] - origin[1]) / texel_m - 0.5,
            ]
        };
        for stroke in strokes {
            let k = stroke.kind as usize;
            let h = f64::from(stroke.half_width.max(0.0));
            let reach = (range + h) / texel_m;
            for pair in stroke.points.windows(2) {
                let (a, b) = (local(pair[0]), local(pair[1]));
                each_near(n, a, b, reach, |i, j, dist| {
                    let v = dist * texel_m - h;
                    let t = &mut d[j * n + i];
                    if v < t[k] {
                        t[k] = v;
                        half[j * n + i][k] = h as f32;
                        if k == StrokeKind::Paved as usize {
                            tone[j * n + i] = stroke.tone;
                        }
                        touched = true;
                    }
                });
            }
        }
        let water = StrokeKind::Water as usize;
        for area in areas {
            let r = range / texel_m;
            // The area's box on the page, its edges' reach included.
            let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for p in area.rings.iter().flatten() {
                let q = local(*p);
                lo = [lo[0].min(q[0]), lo[1].min(q[1])];
                hi = [hi[0].max(q[0]), hi[1].max(q[1])];
            }
            let top = (lo[1] - r).ceil().max(0.0);
            let bottom = (hi[1] + r).floor().min(n as f64 - 1.0);
            let left = (lo[0] - r).ceil().max(0.0);
            let right = (hi[0] + r).floor().min(n as f64 - 1.0);
            if top > bottom || left > right {
                continue;
            }
            let (top, bottom, left, right) =
                (top as usize, bottom as usize, left as usize, right as usize);
            let w = right - left + 1;
            // Unsigned distance to the nearest edge (texels), and each row's crossings for the even-odd rule.
            let mut near = vec![r; w * (bottom - top + 1)];
            let mut crossings: Vec<Vec<f64>> = vec![Vec::new(); bottom - top + 1];
            for ring in &area.rings {
                let m = ring.len();
                for e in 0..m {
                    let (a, b) = (local(ring[e]), local(ring[(e + 1) % m]));
                    each_near(n, a, b, r, |i, j, dist| {
                        if (top..=bottom).contains(&j) && (left..=right).contains(&i) {
                            let c = &mut near[(j - top) * w + i - left];
                            *c = c.min(dist);
                        }
                    });
                    let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
                    let first = y0.ceil().max(top as f64);
                    let last = (y1.ceil() - 1.0).min(bottom as f64);
                    if first > last {
                        continue;
                    }
                    // Half-open in y, so a vertex on a row counts once.
                    for j in first as usize..=last as usize {
                        let y = j as f64;
                        if y >= y0 && y < y1 {
                            let x = a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                            crossings[j - top].push(x);
                        }
                    }
                }
            }
            for (row, xs) in crossings.iter_mut().enumerate() {
                xs.sort_by(f64::total_cmp);
                let j = top + row;
                let mut next = 0;
                for i in left..=right {
                    while next < xs.len() && xs[next] < i as f64 {
                        next += 1;
                    }
                    let inside = next % 2 == 1;
                    let dist = near[row * w + i - left] * texel_m;
                    let v = if inside { -dist } else { dist };
                    let t = &mut d[j * n + i];
                    if v < t[water] {
                        t[water] = v;
                        half[j * n + i][water] = WATER_HALF_WIDTH_MAX_M;
                        touched = true;
                    }
                }
            }
        }
        if !touched {
            return Self::empty();
        }
        let mut edges = Vec::with_capacity(n * n * 4);
        let mut attrs = Vec::with_capacity(n * n * 4);
        for k in 0..n * n {
            edges.extend(d[k].map(|v| encode_distance(v, range)));
            let unit = |v: f32, max: f32| ((v / max).clamp(0.0, 1.0) * 255.0).round() as u8;
            attrs.extend([
                unit(half[k][0], HALF_WIDTH_MAX_M),
                unit(tone[k], 1.0),
                unit(half[k][1], HALF_WIDTH_MAX_M),
                unit(half[k][3], WATER_HALF_WIDTH_MAX_M),
            ]);
        }
        Self {
            edges,
            attrs,
            empty: false,
            lamps: Vec::new(),
        }
    }

    /// The distance in channel `kind` at texel (i, j) (metres, for a page of `texel_m`).
    pub fn distance(&self, i: u32, j: u32, kind: StrokeKind, texel_m: f64) -> f64 {
        let k = ((j * PAGE_TEXELS + i) * 4) as usize + kind as usize;
        decode_distance(self.edges[k], RANGE_TEXELS * texel_m)
    }
}

/// The land layer's feature texture: one RGBA8 array of `2 × LEVELS` layers (level `l`'s distances in layer `2 l`, its
/// attributes in `2 l + 1`), bound at the terrain group's binding 14.
pub(crate) struct FeatureTextures {
    placeholder: wgpu::TextureView,
    array: Option<(wgpu::Texture, wgpu::TextureView, u32)>,
    grid: Option<FeatureGrid>,
    held: [Vec<Option<[i32; 2]>>; LEVELS],
    queue: VecDeque<(usize, [i32; 2], Arc<FeaturePage>)>,
    /// Drawn (the default); hidden, the shaders skip them as with none bound (for comparisons).
    pub shown: bool,
}

fn array_texture(device: &wgpu::Device, label: &str, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 2 * LEVELS as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
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

impl FeatureTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        let placeholder = array_texture(device, "terrain features placeholder", 1);
        Self {
            placeholder: array_view(&placeholder),
            array: None,
            grid: None,
            held: std::array::from_fn(|_| Vec::new()),
            queue: VecDeque::new(),
            shown: true,
        }
    }

    /// The view binding 14 takes.
    pub fn view(&self) -> &wgpu::TextureView {
        self.array.as_ref().map_or(&self.placeholder, |(_, v, _)| v)
    }

    /// Takes pages on `grid`; true when the view changed.
    pub fn set_grid(&mut self, device: &wgpu::Device, grid: FeatureGrid) -> Result<bool, String> {
        if grid.slots == 0 || grid.slots > MAX_SLOTS || grid.texel_m.iter().any(|t| *t <= 0.0) {
            return Err(format!(
                "map features: {} a side of {:?} m texels",
                grid.slots, grid.texel_m
            ));
        }
        let largest = device.limits().max_texture_dimension_2d;
        if grid.texels() > largest {
            return Err(format!(
                "map features: {0}² texels, more than this device's {largest}² textures",
                grid.texels()
            ));
        }
        if self.grid == Some(grid) {
            return Ok(false);
        }
        self.grid = Some(grid);
        let slots = (grid.slots * grid.slots) as usize;
        self.held = std::array::from_fn(|_| vec![None; slots]);
        self.queue.clear();
        if self
            .array
            .as_ref()
            .is_some_and(|(_, _, size)| *size == grid.texels())
        {
            return Ok(false);
        }
        let texture = array_texture(device, "terrain features", grid.texels());
        let view = array_view(&texture);
        self.array = Some((texture, view, grid.texels()));
        Ok(true)
    }

    /// No pages: true when the view changed.
    pub fn clear(&mut self) -> bool {
        self.grid = None;
        self.queue.clear();
        self.held = std::array::from_fn(|_| Vec::new());
        self.array.take().is_some()
    }

    pub fn grid(&self) -> Option<FeatureGrid> {
        self.grid
    }

    /// Whether page `index` of `level` is drawn or waiting for its upload.
    pub fn holds(&self, level: usize, index: [i32; 2]) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        let [i, j] = grid.slot(index);
        self.held[level].get((j * grid.slots + i) as usize) == Some(&Some(index))
            || self
                .queue
                .iter()
                .any(|(l, k, _)| *l == level && *k == index)
    }

    /// Pages drawn, and waiting for their upload.
    pub fn counts(&self) -> (usize, usize) {
        (
            self.held.iter().flatten().flatten().count(),
            self.queue.len(),
        )
    }

    /// Queues page `index` of `level` for its upload (a page queued for the same slot before is dropped; one queued for
    /// the same page is replaced in its place: the newer is the one made last).
    pub fn queue(&mut self, level: usize, index: [i32; 2], page: Arc<FeaturePage>) {
        let Some(grid) = self.grid else {
            return;
        };
        let n = (PAGE_TEXELS * PAGE_TEXELS * 4) as usize;
        if level >= LEVELS || page.edges.len() != n || page.attrs.len() != n {
            return;
        }
        let slot = grid.slot(index);
        self.queue.retain(|(l, k, _)| {
            (*l == level && *k == index) || *l != level || grid.slot(*k) != slot
        });
        match self
            .queue
            .iter_mut()
            .find(|(l, k, _)| *l == level && *k == index)
        {
            Some(queued) => queued.2 = page,
            None => self.queue.push_back((level, index, page)),
        }
    }

    /// Uploads queued pages, up to [`UPLOAD_BYTES`] (at least one); how many. `changed` takes the chart rectangle each
    /// page uploaded, and each page it put out of its slot, can change (`FeatureGrid::reach`); `uploaded` each page
    /// uploaded (its level and index) for its lamps.
    pub fn upload(
        &mut self,
        queue: &wgpu::Queue,
        changed: &mut Vec<[f64; 4]>,
        uploaded_pages: &mut Vec<(usize, [i32; 2], Arc<FeaturePage>)>,
    ) -> usize {
        let (Some(grid), Some((texture, _, _))) = (self.grid, &self.array) else {
            return 0;
        };
        let mut bytes = 0;
        let mut uploaded = 0;
        while bytes < UPLOAD_BYTES
            && let Some((level, index, page)) = self.queue.pop_front()
        {
            let [si, sj] = grid.slot(index);
            for (layer, data) in [(2 * level, &page.edges), (2 * level + 1, &page.attrs)] {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d {
                            x: si * PAGE_TEXELS,
                            y: sj * PAGE_TEXELS,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
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
            }
            let held = &mut self.held[level][(sj * grid.slots + si) as usize];
            if let Some(gone) = held.filter(|gone| *gone != index) {
                changed.push(grid.reach(level, gone));
            }
            changed.push(grid.reach(level, index));
            *held = Some(index);
            bytes += page.bytes();
            uploaded += 1;
            uploaded_pages.push((level, index, page));
        }
        uploaded
    }

    /// The uniforms: each level's texel, the slots, whether bound; each level's page (0, 0) corner; the page each slot
    /// holds.
    pub fn uniforms(
        &self,
        features: &mut [f32; 4],
        origin: &mut [f32; 4],
        slots: &mut [[i32; 4]; 64],
    ) {
        for entry in slots.iter_mut() {
            *entry = [NO_PAGE; 4];
        }
        let Some(grid) = self.grid else {
            *features = [0.0; 4];
            *origin = [0.0; 4];
            return;
        };
        *features = [
            grid.texel_m[0] as f32,
            grid.texel_m[1] as f32,
            grid.slots as f32,
            if self.shown { 1.0 } else { 0.0 },
        ];
        *origin = [
            grid.origin[0][0] as f32,
            grid.origin[0][1] as f32,
            grid.origin[1][0] as f32,
            grid.origin[1][1] as f32,
        ];
        for (level, held) in self.held.iter().enumerate() {
            for (k, page) in held.iter().enumerate() {
                if let Some([i, j]) = page {
                    let half = (k % 2) * 2;
                    slots[level * 32 + k / 2][half] = *i;
                    slots[level * 32 + k / 2][half + 1] = *j;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capsules_rows_are_its_own() {
        // Every texel the span says is near is, and every one it leaves out is not.
        let (a, b, r) = ([3.2, 4.1], [40.7, 19.9], 4.5);
        for j in 0..30 {
            let span = capsule_span(a, b, r, f64::from(j));
            for i in -10..60 {
                let p = [f64::from(i), f64::from(j)];
                let near = segment_distance(p, a, b) <= r;
                let inside = span.is_some_and(|(x0, x1)| p[0] >= x0 - 1e-9 && p[0] <= x1 + 1e-9);
                assert_eq!(near, inside, "{p:?} {span:?}");
            }
        }
        // Horizontal, vertical and a point.
        assert_eq!(
            capsule_span([0.0, 0.0], [10.0, 0.0], 1.0, 0.5).map(|s| s.0.round()),
            Some(-1.0)
        );
        assert!(capsule_span([0.0, 0.0], [0.0, 10.0], 1.0, 12.0).is_none());
        assert!(capsule_span([5.0, 5.0], [5.0, 5.0], 2.0, 6.0).is_some());
    }

    #[test]
    fn a_road_is_a_band_its_width_and_a_river_area_is_filled() {
        // A 7 m road across a 2 m page from west to east, 100 m south of its top.
        let t = 2.0;
        let origin = [1000.0, -400.0];
        let road = Stroke {
            kind: StrokeKind::Paved,
            half_width: 3.5,
            tone: 0.6,
            points: vec![[900.0, -300.0], [1700.0, -300.0]],
        };
        let rail = Stroke {
            kind: StrokeKind::Rail,
            half_width: 0.0,
            tone: 0.0,
            points: vec![[1100.0, -500.0], [1100.0, 200.0]],
        };
        // A riverbank: a square 100 m on a side with a 20 m island.
        let bank = WaterArea {
            rings: vec![
                vec![
                    [1300.0, -200.0],
                    [1400.0, -200.0],
                    [1400.0, -100.0],
                    [1300.0, -100.0],
                ],
                vec![
                    [1340.0, -160.0],
                    [1360.0, -160.0],
                    [1360.0, -140.0],
                    [1340.0, -140.0],
                ],
            ],
        };
        let page = FeaturePage::rasterize(origin, t, &[road, rail], &[bank]);
        assert!(!page.empty);
        let texel = |x: f64, z: f64| (((x - origin[0]) / t) as u32, ((z - origin[1]) / t) as u32);
        let paved = |x, z| {
            let (i, j) = texel(x, z);
            page.distance(i, j, StrokeKind::Paved, t)
        };
        // A texel's middle is a metre past its corner: a metre off the road's middle line, 2.5 m inside its edge; 7 m off, 3.5 m
        // outside.
        let tol = 2.0 * RANGE_TEXELS * t / 255.0;
        assert!(
            (paved(1200.0, -301.0) - (-2.5)).abs() < tol,
            "{}",
            paved(1200.0, -301.0)
        );
        assert!(
            (paved(1200.0, -307.0) - 3.5).abs() < tol,
            "{}",
            paved(1200.0, -307.0)
        );
        // Past the range: the range.
        assert!((paved(1200.0, -351.0) - RANGE_TEXELS * t).abs() < tol);
        // The road's half width and tone in the attributes.
        let (i, j) = texel(1200.0, -301.0);
        let k = ((j * PAGE_TEXELS + i) * 4) as usize;
        assert!((f32::from(page.attrs[k]) / 255.0 * HALF_WIDTH_MAX_M - 3.5).abs() < 0.1);
        assert!((f32::from(page.attrs[k + 1]) / 255.0 - 0.6).abs() < 0.01);
        // The railway: its middle line.
        let (i, j) = texel(1103.0, -250.0);
        assert!((page.distance(i, j, StrokeKind::Rail, t) - 3.0).abs() < tol);
        // The riverbank: inside negative, on the island positive, outside positive.
        let water = |x, z| {
            let (i, j) = texel(x, z);
            page.distance(i, j, StrokeKind::Water, t)
        };
        assert!(water(1321.0, -181.0) < -RANGE_TEXELS * t + tol);
        assert!(water(1349.0, -151.0) > 8.0, "{}", water(1349.0, -151.0));
        assert!(
            (water(1295.0, -151.0) - 5.0).abs() < tol,
            "{}",
            water(1295.0, -151.0)
        );
        assert!(
            (water(1305.0, -151.0) + 5.0).abs() < tol,
            "{}",
            water(1305.0, -151.0)
        );
        // Nothing in reach: an empty page, every distance the range.
        let far = FeaturePage::rasterize([5000.0, 5000.0], t, &[], &[]);
        assert!(far.empty && far.edges.iter().all(|b| *b == 255));
    }
}
