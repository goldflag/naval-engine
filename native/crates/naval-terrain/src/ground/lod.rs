//! Continuous distance-dependent level of detail over a baked heightfield (CDLOD, Strugar 2010), the selection half
//! of the web game's `terrain/TerrainLod.ts`: a quadtree of square nodes, each drawn as four patches of
//! [`PATCH_QUADS`]²
//! quads, each level owning a band of camera distance twice as far out as the level below. The vertex shader slides a
//! level's odd vertices onto its even ones across the outer part of its band, so neighbouring levels meet without seams.

use crate::field::HeightBounds;

/// Quads along a patch's side: a node is drawn as 2 × 2 patches (`PATCH_QUADS`).
pub const PATCH_QUADS: u32 = 16;
/// Share of a level's range where its vertices start and finish sliding onto the next coarser grid.
pub const MORPH_START: f32 = 0.78;
pub const MORPH_END: f32 = 0.97;
/// Heights below this never show through the water above them (`DEEP_WATER_M`).
pub const DEEP_WATER_M: f32 = -30.0;
/// Floats per selected patch: chart x and z of its corner, its level, and 1 where land rises above the sea (2 for a
/// level-0 patch with land drawn dense, [`DENSE_LAND`]).
pub const PATCH_FLOATS: usize = 4;
/// A patch's last float where it is a level-0 patch with land drawn with twice the vertices a side over the fine relief
/// (`TerrainLod::dense`); the land's draw of the plain patches leaves it out, and draws its copy ([`DENSE_COPY`]) with the
/// dense patch's mesh.
pub const DENSE_LAND: f32 = 2.0;
pub const DENSE_COPY: f32 = 3.0;
/// The share of level 0's range within which a level-0 patch with land is drawn dense (`terrain.wgsl`'s
/// `DENSE_SHARE`): the halves there span 5 pixels and more on the High setting (its 20 m vertices aim at 5 at the range's
/// half). Half the range drew halves of 2.5 pixels at its edge: the land's shading on such small triangles cost up to
/// 0.8 ms more (2026-10-09, M5 Pro, 1080 lines; native/docs/terrain/ground.md, "Fine relief near the camera").
pub const DENSE_SHARE: f64 = 0.25;
/// The farthest a patch is drawn dense (m; `terrain.wgsl`'s `DENSE_MAX_M`): through binoculars a quarter of level 0's
/// range is the whole relief, and 171 patches drawn dense cost 0.75 ms there.
pub const DENSE_MAX_M: f64 = 3000.0;
/// Patches the instance buffer holds (`PATCH_CAPACITY`).
pub const PATCH_CAPACITY: usize = 6144;

/// The chart the selection covers.
#[derive(Clone, Debug)]
pub struct TerrainChart {
    pub origin_x: f64,
    pub origin_z: f64,
    pub width: f64,
    pub depth: f64,
    pub bounds: HeightBounds,
    /// Field cell size, metres.
    pub cell: f64,
}

/// Level-0 vertex spacing and the on-screen triangle edge the ranges aim for (`LodSettings`), and the depth below which
/// a node is left out as never seen through the water above it ([`DEEP_WATER_M`] for a battle's eye level).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LodSettings {
    pub spacing: f64,
    pub target_pixels: f64,
    /// A node whose highest sample lies below this is not drawn. Looking down from a plane the seabed shows deeper than
    /// from a ship, and a node left out there shows as a straight edge in the seabed: a theatre flight's land draws to
    /// its floor.
    pub deep_water_m: f64,
}

/// The terrain settings the game offers (`TERRAIN_LOD`): level-0 spacing, target pixels, trees, detailed surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainQuality {
    pub lod: LodSettings,
    pub trees: bool,
    pub detailed: bool,
    /// The ground materials under the land's colour close up (`ground_materials`; High only).
    pub materials: bool,
    /// Fine relief near the camera (`relief`; High only): the land's heights and slope from a finer elevation model
    /// where a host hands its pages in.
    pub relief: bool,
    /// The far land beyond a theatre window (`horizon`; High only), drawn to [`crate::horizon::FAR_M`].
    pub horizon: bool,
}

impl TerrainQuality {
    /// High: 20 m vertices at 5 px, trees, the finest surface detail.
    pub const HIGH: Self = Self {
        lod: LodSettings {
            spacing: 20.0,
            target_pixels: 5.0,
            deep_water_m: DEEP_WATER_M as f64,
        },
        trees: true,
        detailed: true,
        materials: true,
        relief: true,
        horizon: true,
    };
    /// Medium: 40 m vertices at 9 px, no trees, no leaf clumps, streaks, furrows or froth.
    pub const MEDIUM: Self = Self {
        lod: LodSettings {
            spacing: 40.0,
            target_pixels: 9.0,
            deep_water_m: DEEP_WATER_M as f64,
        },
        trees: false,
        detailed: false,
        materials: false,
        relief: false,
        horizon: false,
    };

    /// By the terrain setting's name (`high`, `medium`).
    pub fn by_name(name: &str) -> Option<Self> {
        match name {
            "high" => Some(Self::HIGH),
            "medium" => Some(Self::MEDIUM),
            _ => None,
        }
    }
}

/// What the selection reads from the camera, in chart metres (`LodView`).
#[derive(Clone, Debug)]
pub struct LodView {
    pub position: [f64; 3],
    /// Frustum planes as [nx, ny, nz, constant], inside where n · p + constant ≥ 0.
    pub planes: Vec<[f64; 4]>,
    pub fov_y: f64,
    pub height_px: f64,
    pub submerged: bool,
    /// The camera's curvature (`naval_render::Camera::curvature`): how far a patch is drawn below where it stands.
    pub curvature: f64,
}

/// Metres a level-0 node spans at `level` (`nodeSize`).
pub fn node_size(settings: &LodSettings, level: u32) -> f64 {
    settings.spacing * f64::from(PATCH_QUADS) * 2.0 * 2f64.powi(level as i32)
}

/// Range of level 0 in metres (`baseRange`).
pub fn base_range(settings: &LodSettings, fov_y: f64, height_px: f64) -> f64 {
    let pixels_per_radian = height_px / (2.0 * (fov_y / 2.0).tan());
    let range = 2.0 * settings.spacing * pixels_per_radian / settings.target_pixels;
    range.max(8.0 * node_size(settings, 0))
}

/// The selection (`TerrainLod`).
pub struct TerrainLod {
    pub chart: TerrainChart,
    pub settings: LodSettings,
    pub root_level: u32,
    /// Ranges of each level for the last selection, metres.
    pub ranges: Vec<f64>,
    /// Level-0 patches with land at the front of the last selection that carry trees.
    pub near_land: usize,
    /// Distance to the nearest level-0 patch with land left without trees; infinite when every one has them.
    pub tree_reach: f64,
    /// Most level-0 patches with land that carry trees.
    pub tree_patches: usize,
    /// Where the level-0 patches with land within [`DENSE_SHARE`] of level 0's range are drawn dense (chart x0, z0, x1,
    /// z1: the fine relief's pages), marked [`DENSE_LAND`]; none, none are.
    pub dense: Option<[f64; 4]>,
    /// How far the fine relief lifts or lowers the land at most (m): a patch is kept in view by its bounds widened by it.
    pub relief_margin: f64,
    blocks_per_patch: f64,
}

struct Walk<'a> {
    out: &'a mut [f32],
    count: usize,
    capacity: usize,
    view: &'a LodView,
}

impl TerrainLod {
    pub fn new(chart: TerrainChart, settings: LodSettings) -> Result<Self, String> {
        let extent = chart.width.max(chart.depth);
        let root_level = (extent / node_size(&settings, 0)).log2().ceil().max(0.0) as u32;
        let patch_cells = settings.spacing * f64::from(PATCH_QUADS) / chart.cell;
        let blocks_per_patch = patch_cells / chart.bounds.base as f64;
        if blocks_per_patch.fract() != 0.0 || blocks_per_patch < 1.0 {
            return Err("Terrain bounds must tile a level-0 patch.".into());
        }
        Ok(Self {
            chart,
            settings,
            root_level,
            ranges: Vec::new(),
            near_land: 0,
            tree_reach: f64::INFINITY,
            tree_patches: 256,
            dense: None,
            relief_margin: 0.0,
            blocks_per_patch,
        })
    }

    /// Fills `out` with the patches to draw for `view` ([`PATCH_FLOATS`] each) and returns how many, up to the
    /// buffer's capacity. The level-0 patches with land come first; [`TerrainLod::near_land`] says how many.
    pub fn select(&mut self, view: &LodView, out: &mut [f32]) -> usize {
        let range0 = base_range(&self.settings, view.fov_y, view.height_px);
        self.ranges.clear();
        for level in 0..=self.root_level {
            self.ranges.push(range0 * 2f64.powi(level as i32));
        }
        let capacity = out.len() / PATCH_FLOATS;
        let mut walk = Walk {
            out,
            count: 0,
            capacity,
            view,
        };
        self.node(&mut walk, self.root_level, 0, 0);
        let count = walk.count;
        let out = walk.out;
        // Move the level-0 patches with land to the front, keeping their order.
        let mut front = 0;
        for k in 0..count {
            let at = k * PATCH_FLOATS;
            if out[at + 2] != 0.0 || out[at + 3] == 0.0 {
                continue;
            }
            if k != front {
                for f in 0..PATCH_FLOATS {
                    out.swap(front * PATCH_FLOATS + f, at + f);
                }
            }
            front += 1;
        }
        self.near_land = front;
        self.tree_reach = f64::INFINITY;
        if front > self.tree_patches {
            self.nearest_first(out, view, front);
        }
        count
    }

    fn nearest_first(&mut self, out: &mut [f32], view: &LodView, front: usize) {
        let [px, py, pz] = view.position;
        let half = node_size(&self.settings, 0) / 4.0;
        let distance: Vec<f64> = (0..front)
            .map(|k| {
                let at = k * PATCH_FLOATS;
                let dx = f64::from(out[at]) + half - px;
                let dz = f64::from(out[at + 1]) + half - pz;
                (dx * dx + dz * dz + py * py).sqrt()
            })
            .collect();
        let mut order: Vec<usize> = (0..front).collect();
        // Array.prototype.sort is stable, as sort_by is.
        order.sort_by(|&a, &b| distance[a].total_cmp(&distance[b]));
        let copy = out[..front * PATCH_FLOATS].to_vec();
        for (to, &from) in order.iter().enumerate() {
            out[to * PATCH_FLOATS..(to + 1) * PATCH_FLOATS]
                .copy_from_slice(&copy[from * PATCH_FLOATS..(from + 1) * PATCH_FLOATS]);
        }
        self.near_land = self.tree_patches;
        self.tree_reach = distance[order[self.tree_patches]] - half * std::f64::consts::SQRT_2;
    }

    fn bounds_box(&self, level: u32, ix: i64, iz: i64, patch: bool) -> Option<(f64, f64)> {
        let levels = &self.chart.bounds.levels;
        let pyramid =
            self.blocks_per_patch.log2() as i64 + i64::from(level) + if patch { 0 } else { 1 };
        let bounds = &levels[(pyramid as usize).min(levels.len() - 1)];
        let scale: i64 = if pyramid >= levels.len() as i64 {
            1 << (pyramid - levels.len() as i64 + 1)
        } else {
            1
        };
        let (bx, bz) = (ix * scale, iz * scale);
        let size = bounds.size as i64;
        if bx >= size || bz >= size {
            return None;
        }
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for j in bz..(bz + scale).min(size) {
            for i in bx..(bx + scale).min(size) {
                let k = (j * size + i) as usize;
                low = low.min(f64::from(bounds.min[k]));
                high = high.max(f64::from(bounds.max[k]));
            }
        }
        Some((low, high))
    }

    fn distance2(view: &LodView, x0: f64, z0: f64, size: f64, low: f64, high: f64) -> f64 {
        let [px, py, pz] = view.position;
        let dx = if px < x0 {
            x0 - px
        } else if px > x0 + size {
            px - x0 - size
        } else {
            0.0
        };
        let dy = if py < low {
            low - py
        } else if py > high {
            py - high
        } else {
            0.0
        };
        let dz = if pz < z0 {
            z0 - pz
        } else if pz > z0 + size {
            pz - z0 - size
        } else {
            0.0
        };
        dx * dx + dy * dy + dz * dz
    }

    fn visible(&self, view: &LodView, x0: f64, z0: f64, size: f64, low: f64, high: f64) -> bool {
        let (low, high) = (low - self.relief_margin, high + self.relief_margin);
        // The patch is drawn lowered by the Earth's curve (`naval_render::camera::curved`): its top by at least the
        // drop at its nearest point to the eye, its bottom by at most the drop at its farthest.
        let [px, _, pz] = view.position;
        let span = |p: f64, from: f64| {
            let (a, b) = ((p - from).abs(), (p - from - size).abs());
            let near = if p >= from && p <= from + size {
                0.0
            } else {
                a.min(b)
            };
            (near, a.max(b))
        };
        let ((near_x, far_x), (near_z, far_z)) = (span(px, x0), span(pz, z0));
        let high = high - view.curvature * (near_x * near_x + near_z * near_z);
        let low = low - view.curvature * (far_x * far_x + far_z * far_z);
        view.planes.iter().all(|&[nx, ny, nz, constant]| {
            let x = if nx > 0.0 { x0 + size } else { x0 };
            let y = if ny > 0.0 { high } else { low };
            let z = if nz > 0.0 { z0 + size } else { z0 };
            nx * x + ny * y + nz * z + constant >= 0.0
        })
    }

    fn node(&self, walk: &mut Walk<'_>, level: u32, ix: i64, iz: i64) -> bool {
        let size = node_size(&self.settings, level);
        let x0 = self.chart.origin_x + ix as f64 * size;
        let z0 = self.chart.origin_z + iz as f64 * size;
        if x0 >= self.chart.origin_x + self.chart.width
            || z0 >= self.chart.origin_z + self.chart.depth
        {
            return true;
        }
        let Some((low, high)) = self.bounds_box(level, ix, iz, false) else {
            return true;
        };
        let range = self.ranges[level as usize];
        let d2 = Self::distance2(walk.view, x0, z0, size, low, high);
        if d2 > range * range {
            return false;
        }
        if !self.visible(walk.view, x0, z0, size, low, high) {
            return true;
        }
        if level == 0 || d2 > self.ranges[level as usize - 1].powi(2) {
            for q in 0..4 {
                self.patch(walk, level, ix * 2 + (q & 1), iz * 2 + (q >> 1));
            }
            return true;
        }
        for q in 0..4 {
            let (cx, cz) = (ix * 2 + (q & 1), iz * 2 + (q >> 1));
            if !self.node(walk, level - 1, cx, cz) {
                self.patch(walk, level, cx, cz);
            }
        }
        true
    }

    fn patch(&self, walk: &mut Walk<'_>, level: u32, px: i64, pz: i64) {
        let size = node_size(&self.settings, level) / 2.0;
        let x0 = self.chart.origin_x + px as f64 * size;
        let z0 = self.chart.origin_z + pz as f64 * size;
        if x0 >= self.chart.origin_x + self.chart.width
            || z0 >= self.chart.origin_z + self.chart.depth
        {
            return;
        }
        let Some((low, high)) = self.bounds_box(level, px, pz, true) else {
            return;
        };
        if high < self.settings.deep_water_m
            && !(walk.view.submerged
                && Self::distance2(walk.view, x0, z0, size, low, high) < 4000f64.powi(2))
        {
            return;
        }
        if !self.visible(walk.view, x0, z0, size, low, high) || walk.count >= walk.capacity {
            return;
        }
        let at = walk.count * PATCH_FLOATS;
        walk.count += 1;
        walk.out[at] = x0 as f32;
        walk.out[at + 1] = z0 as f32;
        walk.out[at + 2] = level as f32;
        walk.out[at + 3] = if high <= 0.0 {
            0.0
        } else if level == 0 && self.dense_at(walk.view, x0, z0, size, low, high) {
            DENSE_LAND
        } else {
            1.0
        };
    }

    /// Whether a level-0 patch with land is drawn dense: inside the dense rectangle and within [`DENSE_SHARE`] of level
    /// 0's range (at most [`DENSE_MAX_M`]), where its vertices between level 0's have finished sliding onto them by the
    /// patch's far edge (as a level's vertices onto the next's: `terrain.wgsl`'s `place_vertex`).
    fn dense_at(&self, view: &LodView, x0: f64, z0: f64, size: f64, low: f64, high: f64) -> bool {
        let Some([dx0, dz0, dx1, dz1]) = self.dense else {
            return false;
        };
        let reach = (self.ranges.first().copied().unwrap_or(0.0) * DENSE_SHARE).min(DENSE_MAX_M);
        x0 < dx1
            && x0 + size > dx0
            && z0 < dz1
            && z0 + size > dz0
            && Self::distance2(view, x0, z0, size, low, high) < reach * reach
    }
}
