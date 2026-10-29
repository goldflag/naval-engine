//! The forest's shade (`tree_shade.wgsl`; native/docs/terrain/trees.md): the trees standing per tree cell and their
//! mean height, baked on the attribute grid into the open sky's spare green and blue. Baked whole on the layer's first
//! frame (and in the next frame when the surface maps or a grid change), and after that only over the rectangles whose
//! inputs changed: a map-feature page or a near tile coming into its slot, or leaving it. A whole window's bake is 8–15
//! ms of GPU; a page's rectangles under 1.5 ms. Rectangles asked for within [`SHADE_FRAMES`] frames of the last bake wait
//! and go together (at most [`SHADE_RECTS`], merged where they touch), into one storage texture kept for the layer, each
//! rectangle then copied over the open sky.

use naval_render::{
    PipelineId, PrepareContext, RenderContext, ShaderDefs, ShaderError, compute_recipe,
};

/// Most rectangles one bake takes (`tree_shade.wgsl`'s `SHADE_RECTS`): more are merged.
pub const SHADE_RECTS: usize = 8;
/// Frames at least from one bake to the next after the first, so pages arriving a few a frame bake together.
pub const SHADE_FRAMES: u32 = 8;

/// A rectangle of the attribute grid in texels: the first column and row, then one past the last.
pub type TexelRect = [u32; 4];

fn is_empty(r: TexelRect) -> bool {
    r[0] >= r[2] || r[1] >= r[3]
}

fn area(r: TexelRect) -> u64 {
    u64::from(r[2] - r[0]) * u64::from(r[3] - r[1])
}

fn union(a: TexelRect, b: TexelRect) -> TexelRect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Overlapping or sharing an edge.
fn touches(a: TexelRect, b: TexelRect) -> bool {
    a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
}

/// The rectangles waiting for a bake: none touching another, at most [`SHADE_RECTS`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DirtyRects {
    rects: Vec<TexelRect>,
}

impl DirtyRects {
    pub fn add(&mut self, rect: TexelRect) {
        if is_empty(rect) {
            return;
        }
        let mut rect = rect;
        while let Some(k) = self.rects.iter().position(|r| touches(*r, rect)) {
            rect = union(self.rects.swap_remove(k), rect);
        }
        self.rects.push(rect);
        if self.rects.len() > SHADE_RECTS {
            // The pair whose union adds the least area goes together.
            let n = self.rects.len();
            let mut best = (0, 1, u64::MAX);
            for i in 0..n {
                for j in i + 1..n {
                    let (a, b) = (self.rects[i], self.rects[j]);
                    let grows = area(union(a, b)).saturating_sub(area(a) + area(b));
                    if grows < best.2 {
                        best = (i, j, grows);
                    }
                }
            }
            let b = self.rects.swap_remove(best.1);
            let a = self.rects.swap_remove(best.0);
            self.add(union(a, b));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }

    pub fn take(&mut self) -> Vec<TexelRect> {
        std::mem::take(&mut self.rects)
    }

    pub fn rects(&self) -> &[TexelRect] {
        &self.rects
    }
}

/// The texels of an attribute grid `size` texels a side (texel (i, j) at `origin` + (i, j) × `cell` in chart metres)
/// whose shade reads anything inside chart rectangle `chart` (x0, z0, x1, z1): a texel's four points lie within a
/// quarter texel of it, and the heights and attributes it reads are filtered, a texel more.
pub fn texels_of(chart: [f64; 4], origin: [f32; 2], cell: f64, size: [u32; 2]) -> TexelRect {
    let at = |v: f64, o: f32| (v - f64::from(o)) / cell;
    let lo = |v: f64, o: f32, n: u32| (at(v, o).floor() - 1.0).clamp(0.0, f64::from(n)) as u32;
    let hi = |v: f64, o: f32, n: u32| (at(v, o).ceil() + 2.0).clamp(0.0, f64::from(n)) as u32;
    [
        lo(chart[0], origin[0], size[0]),
        lo(chart[1], origin[1], size[1]),
        hi(chart[2], origin[0], size[0]),
        hi(chart[3], origin[1], size[1]),
    ]
}

/// The bake: its pipeline, its storage texture and the rectangles waiting.
pub(crate) struct ForestShade {
    bake: PipelineId,
    layout: wgpu::BindGroupLayout,
    rects_buffer: wgpu::Buffer,
    /// The bake's target, the open sky's size, made at the first bake and kept.
    target: Option<(wgpu::Texture, wgpu::BindGroup)>,
    dirty: DirtyRects,
    /// The whole grid waits (`mark_all`): baked at the next frame, whenever the last bake was.
    whole: bool,
    /// Frames since the last bake; none before the first.
    since: Option<u32>,
}

impl ForestShade {
    /// The bake's pipeline (group 1 the terrain group, `terrain_layout`) with the land's `defs`.
    pub fn new(
        ctx: &mut RenderContext<'_>,
        terrain_layout: &wgpu::BindGroupLayout,
        defs: ShaderDefs,
        format: wgpu::TextureFormat,
    ) -> Result<Self, ShaderError> {
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("terrain forest shade bake"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::StorageTexture {
                            access: wgpu::StorageTextureAccess::WriteOnly,
                            format,
                            view_dimension: wgpu::TextureViewDimension::D2,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let bake = ctx.add_pipeline(
            "terrain forest shade bake",
            "naval-terrain/shaders/tree_shade.wgsl",
            defs,
            compute_recipe(vec![layout.clone(), terrain_layout.clone()], "bake"),
        )?;
        let rects_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("terrain forest shade rectangles"),
            size: (SHADE_RECTS * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            bake,
            layout,
            rects_buffer,
            target: None,
            dirty: DirtyRects::default(),
            whole: false,
            since: None,
        })
    }

    /// Bake the whole grid (`size` texels) in the next frame: the surface maps or a grid changed, rarely and all at once,
    /// and the land must not draw the old shade with them for the frames a page's rectangle may wait.
    pub fn mark_all(&mut self, size: [u32; 2]) {
        self.dirty.take();
        self.dirty.add([0, 0, size[0], size[1]]);
        self.whole = true;
    }

    /// Bake `rect` again at the next bake.
    pub fn mark(&mut self, rect: TexelRect) {
        self.dirty.add(rect);
    }

    /// Rectangles waiting for a bake, for tests and probes.
    pub fn waiting(&self) -> &[TexelRect] {
        self.dirty.rects()
    }

    /// Bakes the rectangles waiting, unless the last bake was fewer than [`SHADE_FRAMES`] frames ago (the whole grid
    /// waits for nothing), after the open sky whose red they keep (`open_sky`, read through `terrain_group`).
    pub fn bake(
        &mut self,
        ctx: &mut PrepareContext<'_>,
        terrain_group: &wgpu::BindGroup,
        open_sky: &wgpu::Texture,
    ) {
        if let Some(since) = &mut self.since {
            *since = since.saturating_add(1);
        }
        if self.dirty.is_empty() || (!self.whole && self.since.is_some_and(|s| s < SHADE_FRAMES)) {
            return;
        }
        self.since = Some(0);
        self.whole = false;
        let rects = self.dirty.take();
        let size = open_sky.size();
        let layout = &self.layout;
        let buffer = &self.rects_buffer;
        let (target, group) = self.target.get_or_insert_with(|| {
            let target = ctx.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("terrain forest shade bake"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: open_sky.format(),
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor::default());
            let group = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("terrain forest shade bake"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            });
            (target, group)
        });
        // Each rectangle's corner and size, texels; the dispatch covers the largest, one layer of workgroups each.
        let mut table = [[0u32; 4]; SHADE_RECTS];
        let (mut width, mut height) = (0, 0);
        for (entry, r) in table.iter_mut().zip(&rects) {
            *entry = [r[0], r[1], r[2] - r[0], r[3] - r[1]];
            width = width.max(entry[2]);
            height = height.max(entry[3]);
        }
        ctx.queue
            .write_buffer(&self.rects_buffer, 0, bytemuck::cast_slice(&table));
        {
            let mut pass = ctx
                .encoder
                .begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("terrain forest shade"),
                    timestamp_writes: ctx.timer.compute_pass("terrain forest shade"),
                });
            pass.set_pipeline(ctx.pipelines.compute(self.bake));
            pass.set_bind_group(0, &*group, &[]);
            pass.set_bind_group(1, terrain_group, &[]);
            pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), rects.len() as u32);
        }
        for r in &rects {
            let origin = wgpu::Origin3d {
                x: r[0],
                y: r[1],
                z: 0,
            };
            ctx.encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: target,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: open_sky,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: r[2] - r[0],
                    height: r[3] - r[1],
                    depth_or_array_layers: 1,
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_holds_the_table() {
        let shader = include_str!("../../shaders/tree_shade.wgsl");
        assert!(shader.contains(&format!("const SHADE_RECTS: u32 = {SHADE_RECTS}u;")));
    }

    #[test]
    fn touching_rectangles_merge_and_the_rest_stay_apart() {
        let mut d = DirtyRects::default();
        d.add([0, 0, 10, 10]);
        d.add([20, 20, 30, 30]);
        assert_eq!(d.rects().len(), 2);
        // Sharing an edge with the first: one rectangle.
        d.add([10, 0, 15, 5]);
        assert_eq!(d.rects().len(), 2);
        assert!(d.rects().contains(&[0, 0, 15, 10]));
        // Bridging the two: everything in one.
        d.add([12, 8, 22, 22]);
        assert_eq!(d.rects(), &[[0, 0, 30, 30]]);
        // Empty ones are no work.
        d.add([5, 5, 5, 9]);
        assert_eq!(d.rects().len(), 1);
        assert_eq!(d.take(), vec![[0, 0, 30, 30]]);
        assert!(d.is_empty());
    }

    #[test]
    fn past_the_table_the_closest_pair_goes_together() {
        let mut d = DirtyRects::default();
        // A full table in a row, 10 texels wide every 20, then one far off: two neighbours go together.
        for k in 0..SHADE_RECTS as u32 {
            d.add([k * 20, 0, k * 20 + 10, 10]);
        }
        assert_eq!(d.rects().len(), SHADE_RECTS);
        d.add([1000, 0, 1010, 10]);
        assert_eq!(d.rects().len(), SHADE_RECTS);
        // Every texel asked for is still in some rectangle, and none overlap.
        for x in (0..SHADE_RECTS as u32).map(|k| k * 20).chain([1000]) {
            assert!(
                d.rects()
                    .iter()
                    .any(|r| r[0] <= x && x + 10 <= r[2] && r[1] == 0 && r[3] == 10)
            );
        }
        for (i, a) in d.rects().iter().enumerate() {
            for b in &d.rects()[i + 1..] {
                assert!(!touches(*a, *b), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_chart_rectangle_reaches_the_texels_that_read_it() {
        // 40 m texels from (-1000, -2000), 100 × 50 of them.
        let size = [100, 50];
        let r = texels_of([0.0, 0.0, 400.0, 400.0], [-1000.0, -2000.0], 40.0, size);
        // x: 1000 / 40 = 25 → from 24; 1400 / 40 = 35 → to 37 (exclusive). z: 2000 / 40 = 50 → clamped to the grid.
        assert_eq!(r, [24, 49, 37, 50]);
        // Off the grid: nothing.
        assert!(is_empty(texels_of(
            [9000.0, 0.0, 9100.0, 10.0],
            [-1000.0, -2000.0],
            40.0,
            size
        )));
    }
}
