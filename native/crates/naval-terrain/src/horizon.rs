//! The land beyond a theatre window: the horizon ring (native/docs/terrain/layer.md, "The horizon ring").
//!
//! A coarse model of the real world about a scene, in two nested squares about one centre ([`HorizonMaps`]: heights
//! and the satellite colour, a near level of [`NEAR_CELL_M`] heights and a far one four times coarser), drawn from where
//! the window's land ends out to the far plane ([`FAR_M`]) as silhouettes: no trees, buildings or clutter, the sun on
//! its slopes and no shadow, the air over it as over everything (`naval::aerial`). Each frame a quadtree of patches
//! about the maps' centre is chosen for the camera (CDLOD, as the land's [`crate::lod`]): left out are the window's
//! square (whose patches sink under the window's land where they reach into it, [`Hole`]), the open sea, and what lies
//! wholly below the sea's horizon or beyond the far plane.
//!
//! The host makes the maps off the frame loop's thread and hands them in ([`HorizonLayer::set_maps`]); they are
//! uploaded a few megabytes a frame, the ring drawn before them (or none) kept until they are all on the GPU. The GPU
//! copy is shared by the layers of every scene drawing the same maps, so a scene rebuilt about a recut window takes
//! the ring as it stands.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use half::f16;
use naval_render::{
    DrawContext, Phase, PipelineId, PrepareContext, RenderContext, SceneLayer, ScenePipeline,
    ShaderError, VertexLayout, shader_file,
};

use crate::layer::{TerrainLight, cloud_shadow_layout, frustum_planes};
use crate::surface_maps::{DARK_LAND, HAZE_MAX};
use crate::textures::{mip_count, mips_r16f, mips_srgb8};
use naval_conditions::mesh_light_shares;

/// Texels on a side of each level's heights.
pub const HEIGHT_TEXELS: u32 = 2048;
/// Texels on a side of each level's colour: half the heights' (the light on the slopes carries the relief).
pub const COLOUR_TEXELS: u32 = 1024;
/// The near level's height cell (m): 2048 of them reach 123 km about the centre, past the windows' edges.
pub const NEAR_CELL_M: f64 = 120.0;
/// The far level's cells are this many times the near level's: 480 m, 491 km about the centre.
pub const LEVEL_SCALE: f64 = 4.0;
/// Half the near level's side (m).
pub const NEAR_HALF_M: f64 = NEAR_CELL_M * HEIGHT_TEXELS as f64 / 2.0;
/// Half the far level's side (m).
pub const FAR_HALF_M: f64 = NEAR_HALF_M * LEVEL_SCALE;
/// The far plane of a view that draws the ring (m): beyond the sea's horizon from 3 km up (195 km) a 4 km peak stands
/// above it to 420 km.
pub const FAR_M: f32 = naval_render::camera::HORIZON_FAR;
/// The ring's floor (m): the open sea's floor the windows settle to (`naval_sim::terrain::OPEN_SEA_M`). The Terrain
/// Tiles hold the ocean's depths (ETOPO1); under the sea they show only as its colour, and the windows' edges sink to
/// this.
pub const SEA_FLOOR_M: f32 = -60.0;
/// Quads on a patch's side.
pub const PATCH_QUADS: u32 = 16;
/// The finest patches' vertex spacing (m): the near level's cell.
pub const SPACING_M: f64 = NEAR_CELL_M;
/// Patches the instance buffer holds.
pub const PATCH_CAPACITY: usize = 4096;
/// The on-screen triangle edge the ranges aim at (pixels): the land's Medium spacing, for a silhouette.
pub const TARGET_PIXELS: f64 = 8.0;
/// Where along a level's range its vertices start and finish sliding onto the next level's ([`crate::lod`]'s).
pub const MORPH_START: f32 = crate::lod::MORPH_START;
pub const MORPH_END: f32 = crate::lod::MORPH_END;
/// The finest quadtree level whose node spans the far level ([`node_size`]).
pub const ROOT_LEVEL: u32 = 8;
/// How far the ring sinks under the window's square (m a metre in from its edge): one vertex in, it lies a vertex's
/// spacing below where it stands, under the window's land and its sea floor.
pub const SINK: f32 = 1.0;
/// Near-level texels at its edge over which the far level takes over (`horizon.wgsl`'s `NEAR_BLEND_TEXELS`).
pub const NEAR_BLEND_TEXELS: f64 = 32.0;
/// Bounds texels on a side: four far texels each, one level-0 patch.
const BOUNDS_TEXELS: usize = (HEIGHT_TEXELS as usize) / 4;
/// Bytes uploaded a frame at most.
pub const UPLOAD_BUDGET: usize = 4 << 20;
/// Rows of one level of one layer uploaded together.
const UPLOAD_ROWS: u32 = 256;

/// Metres a quadtree node spans at `level`: two patches of [`PATCH_QUADS`] quads of [`SPACING_M`] doubled per level.
pub fn node_size(level: u32) -> f64 {
    SPACING_M * f64::from(PATCH_QUADS) * 2.0 * 2f64.powi(level as i32)
}

/// Level 0's range (m) for a view `fov_y` (radians) high over `height_px` pixels: its vertices [`TARGET_PIXELS`]
/// apart at half of it ([`crate::lod::base_range`]'s rule), at least eight of its nodes.
pub fn base_range(fov_y: f64, height_px: f64) -> f64 {
    let pixels_per_radian = height_px / (2.0 * (fov_y / 2.0).tan());
    (2.0 * SPACING_M * pixels_per_radian / TARGET_PIXELS).max(8.0 * node_size(0))
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One level of the maps: its square about the centre, its heights' and colours' mip chains.
pub struct HorizonLevel {
    /// Half the square's side (m).
    pub half: f64,
    /// Heights (m) as half floats, [`HEIGHT_TEXELS`]² at level 0 (row j at z = −half + (j + ½) cell, south down the
    /// rows as the theatre's z), a full mip chain.
    pub heights: Vec<Vec<u16>>,
    /// The satellite colour, sRGB RGBA8, [`COLOUR_TEXELS`]², a full mip chain averaged in linear light.
    pub colours: Vec<Vec<u8>>,
}

impl HorizonLevel {
    /// Its height cell (m).
    pub fn cell(&self) -> f64 {
        2.0 * self.half / f64::from(HEIGHT_TEXELS)
    }

    /// The height at `local` (m about the centre), bilinear between texel centres and held at the edge, as the GPU
    /// samples level 0.
    fn height(&self, local: [f64; 2]) -> f64 {
        let n = HEIGHT_TEXELS as usize;
        let cell = self.cell();
        let fx = ((local[0] + self.half) / cell - 0.5).clamp(0.0, (n - 1) as f64);
        let fz = ((local[1] + self.half) / cell - 0.5).clamp(0.0, (n - 1) as f64);
        let (i, j) = ((fx as usize).min(n - 2), (fz as usize).min(n - 2));
        let (tx, tz) = (fx - i as f64, fz - j as f64);
        let at =
            |a: usize, b: usize| f64::from(f16::from_bits(self.heights[0][b * n + a]).to_f32());
        let top = at(i, j) * (1.0 - tx) + at(i + 1, j) * tx;
        let bottom = at(i, j + 1) * (1.0 - tx) + at(i + 1, j + 1) * tx;
        top * (1.0 - tz) + bottom * tz
    }
}

/// The lowest and highest height (m) of each square of the far level, every level of a pyramid from
/// [`BOUNDS_TEXELS`]² (a level-0 patch each) to one.
struct Bounds {
    levels: Vec<(usize, Vec<f32>, Vec<f32>)>,
}

/// The real world about a scene, as the horizon ring draws it: heights and colour in a near and a far level about one
/// centre, the haze the imagery kept, and whether any of it stands above the sea.
pub struct HorizonMaps {
    /// Unique to these maps: what the GPU copies are shared by.
    pub id: u64,
    /// The centre (theatre metres; the ring's frame is metres about it).
    pub centre: [f64; 2],
    pub levels: [HorizonLevel; 2],
    /// Linear colour taken out of the imagery: the haze its atmospheric correction left ([`DARK_LAND`]'s rule, as a
    /// window's surface maps take it out).
    pub haze: [f32; 3],
    /// Whether any of it stands above the sea.
    pub land: bool,
    bounds: Bounds,
}

impl std::fmt::Debug for HorizonMaps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HorizonMaps")
            .field("id", &self.id)
            .field("centre", &self.centre)
            .field("land", &self.land)
            .finish_non_exhaustive()
    }
}

impl HorizonMaps {
    /// From each level's heights (m, [`HEIGHT_TEXELS`]² row-major, rows down the theatre's z) and colour (sRGB RGBA8,
    /// [`COLOUR_TEXELS`]²), near level first. Slow (a second or so): off the frame loop's thread.
    pub fn new(centre: [f64; 2], heights: [Vec<f32>; 2], colours: [Vec<u8>; 2]) -> Self {
        let n = HEIGHT_TEXELS as usize;
        let c = COLOUR_TEXELS as usize;
        assert!(heights.iter().all(|h| h.len() == n * n));
        assert!(colours.iter().all(|h| h.len() == c * c * 4));
        let bounds = Bounds::new(&heights);
        let land = heights.iter().any(|h| h.iter().any(|&v| v > 0.0));
        let haze = haze_of(&heights, &colours);
        let [near_h, far_h] = heights;
        let [near_c, far_c] = colours;
        let level = |half: f64, h: Vec<f32>, colour: Vec<u8>| HorizonLevel {
            half,
            heights: mips_r16f(&h, HEIGHT_TEXELS, HEIGHT_TEXELS),
            colours: mips_srgb8(&colour, COLOUR_TEXELS, COLOUR_TEXELS),
        };
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            centre,
            levels: [
                level(NEAR_HALF_M, near_h, near_c),
                level(FAR_HALF_M, far_h, far_c),
            ],
            haze,
            land,
            bounds,
        }
    }

    /// The share of the near level at `local` (m about the centre): 1 inside its square, falling to 0 over its outer
    /// [`NEAR_BLEND_TEXELS`] (`horizon.wgsl`'s `near_share`).
    pub fn near_share(local: [f64; 2]) -> f64 {
        let edge = NEAR_HALF_M - local[0].abs().max(local[1].abs());
        (edge / (NEAR_CELL_M * NEAR_BLEND_TEXELS)).clamp(0.0, 1.0)
    }

    /// The height (m) the ring draws at theatre point `p` at its finest (the near level, the far one beyond it), not
    /// held to its floor.
    pub fn height(&self, p: [f64; 2]) -> f64 {
        let local = [p[0] - self.centre[0], p[1] - self.centre[1]];
        let far = self.levels[1].height(local);
        let w = Self::near_share(local);
        if w <= 0.0 {
            return far;
        }
        far + (self.levels[0].height(local) - far) * w
    }

    /// Whether the near level holds the square `half` about theatre point `p` whole.
    pub fn covers(&self, p: [f64; 2], half: f64) -> bool {
        let reach = NEAR_HALF_M - NEAR_CELL_M * NEAR_BLEND_TEXELS;
        (p[0] - self.centre[0]).abs() + half <= reach
            && (p[1] - self.centre[1]).abs() + half <= reach
    }

    /// Bytes on the GPU (and as much in memory).
    pub fn bytes(&self) -> usize {
        self.levels
            .iter()
            .map(|l| {
                l.heights.iter().map(|m| m.len() * 2).sum::<usize>()
                    + l.colours.iter().map(Vec::len).sum::<usize>()
            })
            .sum()
    }

    /// The lowest and highest height (m) in the square `size` at `(x0, z0)` (m about the centre, on the quadtree's
    /// grid), held to the floor; none outside the far level.
    fn bounds(&self, x0: f64, z0: f64, size: f64) -> Option<(f64, f64)> {
        self.bounds.of(x0, z0, size)
    }
}

impl Bounds {
    /// Each bounds texel's lowest and highest height over the far level's texels it covers and the near level's where
    /// it reaches, one texel more each way (bilinear sampling reaches into a neighbour), held to the floor.
    fn new(heights: &[Vec<f32>; 2]) -> Self {
        let n = HEIGHT_TEXELS as usize;
        let b = BOUNDS_TEXELS;
        let (mut low, mut high) = (vec![f32::INFINITY; b * b], vec![f32::NEG_INFINITY; b * b]);
        let mut fold = |i: usize, j: usize, v: f32| {
            let k = j * b + i;
            low[k] = low[k].min(v);
            high[k] = high[k].max(v);
        };
        // Far texels: four to a bounds texel.
        let per = n / b;
        for j in 0..n {
            for i in 0..n {
                fold(i / per, j / per, heights[1][j * n + i]);
            }
        }
        // Near texels: the near square is the far's middle quarter; a bounds texel holds 16 × 16 of them.
        let scale = LEVEL_SCALE as usize;
        let offset = n * (scale - 1) / (2 * scale);
        let near_per = per * scale;
        for j in 0..n {
            for i in 0..n {
                fold(
                    offset / per + i / near_per,
                    offset / per + j / near_per,
                    heights[0][j * n + i],
                );
            }
        }
        // One texel more each way, then the floor.
        let dilate = |values: &[f32], pick: fn(f32, f32) -> f32| -> Vec<f32> {
            let mut out = values.to_vec();
            for j in 0..b {
                for i in 0..b {
                    let mut v = values[j * b + i];
                    for (dj, di) in [
                        (-1i64, 0i64),
                        (1, 0),
                        (0, -1),
                        (0, 1),
                        (-1, -1),
                        (1, 1),
                        (-1, 1),
                        (1, -1),
                    ] {
                        let (a, c) = (i as i64 + di, j as i64 + dj);
                        if a >= 0 && c >= 0 && (a as usize) < b && (c as usize) < b {
                            v = pick(v, values[c as usize * b + a as usize]);
                        }
                    }
                    out[j * b + i] = v.max(SEA_FLOOR_M);
                }
            }
            out
        };
        let mut levels = vec![(b, dilate(&low, f32::min), dilate(&high, f32::max))];
        while levels.last().is_some_and(|l| l.0 > 1) {
            let (size, lo, hi) = levels.last().expect("a level");
            let half = size / 2;
            let (mut l2, mut h2) = (
                vec![f32::INFINITY; half * half],
                vec![f32::NEG_INFINITY; half * half],
            );
            for j in 0..*size {
                for i in 0..*size {
                    let k = (j / 2) * half + i / 2;
                    l2[k] = l2[k].min(lo[j * size + i]);
                    h2[k] = h2[k].max(hi[j * size + i]);
                }
            }
            levels.push((half, l2, h2));
        }
        Self { levels }
    }

    fn of(&self, x0: f64, z0: f64, size: f64) -> Option<(f64, f64)> {
        let texel = 2.0 * FAR_HALF_M / BOUNDS_TEXELS as f64;
        let level = (size / texel).log2().round().max(0.0) as usize;
        let (n, low, high) = self.levels.get(level)?;
        let cell = texel * 2f64.powi(level as i32);
        let i = ((x0 + FAR_HALF_M) / cell).floor();
        let j = ((z0 + FAR_HALF_M) / cell).floor();
        if i < 0.0 || j < 0.0 || i >= *n as f64 || j >= *n as f64 {
            return None;
        }
        let k = j as usize * n + i as usize;
        Some((f64::from(low[k]), f64::from(high[k])))
    }
}

/// The haze the imagery kept (`SurfaceMaps::statistics`' rule): per channel the darkest land's colour (its first
/// percentile, every other texel of every other row) less [`DARK_LAND`], at most [`HAZE_MAX`]; the near level's land,
/// else the far level's, else none.
fn haze_of(heights: &[Vec<f32>; 2], colours: &[Vec<u8>; 2]) -> [f32; 3] {
    let decode: Vec<f32> = (0..256)
        .map(|i| {
            let c = i as f32 / 255.0;
            if c < 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
        .collect();
    let n = HEIGHT_TEXELS as usize;
    let c = COLOUR_TEXELS as usize;
    let per = n / c;
    for level in 0..2 {
        let mut histograms = [[0u64; 256]; 3];
        let mut counted = 0u64;
        for j in (0..c).step_by(2) {
            for i in (0..c).step_by(2) {
                if heights[level][j * per * n + i * per] <= 0.0 {
                    continue;
                }
                let k = (j * c + i) * 4;
                for (ch, histogram) in histograms.iter_mut().enumerate() {
                    histogram[colours[level][k + ch] as usize] += 1;
                }
                counted += 1;
            }
        }
        if counted == 0 {
            continue;
        }
        let mut haze = [0f32; 3];
        for (ch, histogram) in histograms.iter().enumerate() {
            let mut seen = 0u64;
            let byte = histogram
                .iter()
                .position(|&count| {
                    seen += count;
                    seen * 100 >= counted
                })
                .unwrap_or(0);
            haze[ch] = (decode[byte] - DARK_LAND[ch]).clamp(0.0, HAZE_MAX);
        }
        return haze;
    }
    [0.0; 3]
}

/// What the selection reads of the camera, in the ring's frame (metres about the maps' centre).
#[derive(Clone, Debug)]
pub struct HorizonView {
    pub position: [f64; 3],
    /// Frustum planes as [nx, ny, nz, constant], inside where n · p + constant ≥ 0.
    pub planes: Vec<[f64; 4]>,
    pub fov_y: f64,
    pub height_px: f64,
    /// How far the world is drawn below where it stands (`naval_render::Camera::curvature`).
    pub curvature: f64,
    pub far: f64,
}

/// The window's square in the ring's frame: the window draws the land there, and the ring sinks under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hole {
    pub centre: [f64; 2],
    pub half: f64,
}

impl Hole {
    fn holds(&self, x0: f64, z0: f64, size: f64) -> bool {
        x0 >= self.centre[0] - self.half
            && x0 + size <= self.centre[0] + self.half
            && z0 >= self.centre[1] - self.half
            && z0 + size <= self.centre[1] + self.half
    }
}

/// Floats per selected patch: its corner's x and z (m about the centre), its level, and 0.
pub const PATCH_FLOATS: usize = 4;

/// The patches the ring draws for `view` about `maps`, the window's square `hole` left out: CDLOD's selection (as
/// [`crate::lod::TerrainLod`]), leaving out the open sea, what lies wholly below the sea's horizon (a patch whose highest
/// point is hidden by the Earth's curve from the eye) and beyond the far plane. Returns level 0's range.
pub fn select(
    maps: &HorizonMaps,
    view: &HorizonView,
    hole: Option<Hole>,
    out: &mut Vec<[f32; 4]>,
) -> f64 {
    out.clear();
    let range0 = base_range(view.fov_y, view.height_px);
    let ranges: Vec<f64> = (0..=ROOT_LEVEL)
        .map(|l| range0 * 2f64.powi(l as i32))
        .collect();
    let walk = Walk {
        maps,
        view,
        hole,
        ranges: &ranges,
        horizon: if view.curvature > 0.0 {
            (view.position[1].max(0.0) / view.curvature).sqrt()
        } else {
            f64::INFINITY
        },
    };
    walk.node(out, ROOT_LEVEL, 0, 0);
    range0
}

struct Walk<'a> {
    maps: &'a HorizonMaps,
    view: &'a HorizonView,
    hole: Option<Hole>,
    ranges: &'a [f64],
    /// The sea's horizon from the eye (m).
    horizon: f64,
}

impl Walk<'_> {
    /// Whether the node at `level` (`ix`, `iz`) is drawn here or below; false when it is out of its level's range, and
    /// its parent draws it.
    fn node(&self, out: &mut Vec<[f32; 4]>, level: u32, ix: i64, iz: i64) -> bool {
        let size = node_size(level);
        let x0 = -FAR_HALF_M + ix as f64 * size;
        let z0 = -FAR_HALF_M + iz as f64 * size;
        let Some((low, high)) = self.maps.bounds(x0, z0, size) else {
            return true;
        };
        if high <= 0.0 || self.hole.is_some_and(|h| h.holds(x0, z0, size)) {
            return true;
        }
        let d2 = distance2(self.view, x0, z0, size, low, high);
        if d2 > self.ranges[level as usize].powi(2) {
            return false;
        }
        if !self.shown(x0, z0, size, low, high) {
            return true;
        }
        if level == 0 || d2 > self.ranges[level as usize - 1].powi(2) {
            for q in 0..4 {
                self.patch(out, level, ix * 2 + (q & 1), iz * 2 + (q >> 1));
            }
            return true;
        }
        for q in 0..4 {
            let (cx, cz) = (ix * 2 + (q & 1), iz * 2 + (q >> 1));
            if !self.node(out, level - 1, cx, cz) {
                self.patch(out, level, cx, cz);
            }
        }
        true
    }

    fn patch(&self, out: &mut Vec<[f32; 4]>, level: u32, px: i64, pz: i64) {
        let size = node_size(level) / 2.0;
        let x0 = -FAR_HALF_M + px as f64 * size;
        let z0 = -FAR_HALF_M + pz as f64 * size;
        let Some((low, high)) = self.maps.bounds(x0, z0, size) else {
            return;
        };
        if high <= 0.0
            || self.hole.is_some_and(|h| h.holds(x0, z0, size))
            || !self.shown(x0, z0, size, low, high)
            || out.len() >= PATCH_CAPACITY
        {
            return;
        }
        out.push([x0 as f32, z0 as f32, level as f32, 0.0]);
    }

    /// Whether any of the square can show: within the far plane, its highest point above the sea's horizon, and in the
    /// view's frustum as drawn (lowered by the Earth's curve, [`crate::lod`]'s test).
    fn shown(&self, x0: f64, z0: f64, size: f64, low: f64, high: f64) -> bool {
        let view = self.view;
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
        let nearest = near_x.hypot(near_z);
        if nearest > view.far {
            return false;
        }
        if view.curvature > 0.0
            && high > 0.0
            && nearest > (self.horizon + (high / view.curvature).sqrt()) * 1.01
        {
            return false;
        }
        let high = high - view.curvature * (near_x * near_x + near_z * near_z);
        let low = low - view.curvature * (far_x * far_x + far_z * far_z);
        view.planes.iter().all(|&[nx, ny, nz, constant]| {
            let x = if nx > 0.0 { x0 + size } else { x0 };
            let y = if ny > 0.0 { high } else { low };
            let z = if nz > 0.0 { z0 + size } else { z0 };
            nx * x + ny * y + nz * z + constant >= 0.0
        })
    }
}

fn distance2(view: &HorizonView, x0: f64, z0: f64, size: f64, low: f64, high: f64) -> f64 {
    let [px, py, pz] = view.position;
    let along = |p: f64, a: f64, b: f64| {
        if p < a {
            a - p
        } else if p > b {
            p - b
        } else {
            0.0
        }
    };
    let (dx, dy, dz) = (
        along(px, x0, x0 + size),
        along(py, low, high),
        along(pz, z0, z0 + size),
    );
    dx * dx + dy * dy + dz * dz
}

// ---------------------------------------------------------------------------------------------------------------------
// The GPU side.

/// `horizon.wgsl`'s `Horizon`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct HorizonUniforms {
    /// World x and z of the maps' centre, level 0's range (m), the floor.
    origin: [f32; 4],
    /// Half the near and far levels' sides, their height cells (m).
    levels: [f32; 4],
    /// The window's square in the ring's frame (centre, half side), and 1 when there is one.
    hole: [f32; 4],
    /// Toward the sun; 1 where the region's style takes the season's snow.
    sun_direction: [f32; 4],
    /// The sun's radiance on meshes; the environment's intensity.
    sun_radiance: [f32; 4],
    /// The hemisphere fill; 1 while the clouds' shadow is bound.
    fill_sky: [f32; 4],
    fill_ground: [f32; 4],
    /// The haze taken out of the imagery.
    haze: [f32; 4],
    /// The snowline (m), the thaw below it, the direction toward the equator (chart x, z).
    climate: [f32; 4],
    /// The season's melt (the climate's leaves, 0-1), shedding the snow from gentler faces.
    season: [f32; 4],
}

/// The ring's textures on the GPU: the levels' heights and colours as two-layer arrays, uploaded a few megabytes a
/// frame from the maps they were made for. Shared by every layer drawing those maps.
struct RingTextures {
    heights: wgpu::Texture,
    colours: wgpu::Texture,
    heights_view: wgpu::TextureView,
    colours_view: wgpu::TextureView,
    /// Uploads still to make.
    pending: Mutex<Vec<Upload>>,
}

#[derive(Clone, Copy, Debug)]
struct Upload {
    colour: bool,
    layer: u32,
    mip: u32,
    row: u32,
    rows: u32,
}

/// The GPU copies of recent maps, by id and device: a layer of a rebuilt scene takes the ring the last one drew.
/// A process-wide static on native; on wasm32, where wgpu's handles are not `Send` (no crate enables
/// `fragile-send-sync-non-atomic-wasm`) and there is one thread, a thread-local.
type SharedRings = Vec<(u64, wgpu::Device, Weak<RingTextures>)>;
#[cfg(not(target_arch = "wasm32"))]
static SHARED: Mutex<SharedRings> = Mutex::new(Vec::new());
#[cfg(target_arch = "wasm32")]
thread_local! {
    static SHARED: Mutex<SharedRings> = const { Mutex::new(Vec::new()) };
}

/// Runs `f` on [`SHARED`]'s list, locked.
fn with_shared<R>(f: impl FnOnce(&mut SharedRings) -> R) -> R {
    #[cfg(not(target_arch = "wasm32"))]
    {
        f(&mut SHARED.lock().unwrap_or_else(|e| e.into_inner()))
    }
    #[cfg(target_arch = "wasm32")]
    {
        SHARED.with(|shared| f(&mut shared.lock().unwrap_or_else(|e| e.into_inner())))
    }
}

impl RingTextures {
    fn new(device: &wgpu::Device) -> Self {
        let texture = |label: &str, format, size: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 2,
                },
                mip_level_count: mip_count(size, size),
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let heights = texture(
            "horizon heights",
            wgpu::TextureFormat::R16Float,
            HEIGHT_TEXELS,
        );
        let colours = texture(
            "horizon colours",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            COLOUR_TEXELS,
        );
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let mut pending = Vec::new();
        for (colour, size) in [(false, HEIGHT_TEXELS), (true, COLOUR_TEXELS)] {
            for layer in 0..2 {
                for mip in 0..mip_count(size, size) {
                    let h = (size >> mip).max(1);
                    let mut row = 0;
                    while row < h {
                        let rows = UPLOAD_ROWS.min(h - row);
                        pending.push(Upload {
                            colour,
                            layer,
                            mip,
                            row,
                            rows,
                        });
                        row += rows;
                    }
                }
            }
        }
        Self {
            heights_view: array(&heights),
            colours_view: array(&colours),
            heights,
            colours,
            pending: Mutex::new(pending),
        }
    }

    /// Shared with a layer on `device` already drawing `maps`, else new.
    fn for_maps(device: &wgpu::Device, maps: &HorizonMaps) -> Arc<Self> {
        with_shared(|shared| {
            shared.retain(|(_, _, weak)| weak.strong_count() > 0);
            if let Some(found) = shared
                .iter()
                .find(|(id, held, _)| *id == maps.id && held == device)
                .and_then(|(_, _, weak)| weak.upgrade())
            {
                return found;
            }
            let made = Arc::new(Self::new(device));
            shared.push((maps.id, device.clone(), Arc::downgrade(&made)));
            made
        })
    }

    /// Uploads up to `budget` bytes of what is still to come from `maps`; whether everything is in.
    fn upload(&self, queue: &wgpu::Queue, maps: &HorizonMaps, mut budget: usize) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        while budget > 0
            && let Some(u) = pending.pop()
        {
            let (texture, size, bytes_per_texel) = if u.colour {
                (&self.colours, COLOUR_TEXELS, 4)
            } else {
                (&self.heights, HEIGHT_TEXELS, 2)
            };
            let width = (size >> u.mip).max(1);
            let level = &maps.levels[u.layer as usize];
            let row_bytes = (width * bytes_per_texel) as usize;
            let start = u.row as usize * row_bytes;
            let end = start + u.rows as usize * row_bytes;
            let bytes: &[u8] = if u.colour {
                &level.colours[u.mip as usize][start..end]
            } else {
                &bytemuck::cast_slice::<u16, u8>(&level.heights[u.mip as usize])[start..end]
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: u.mip,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: u.row,
                        z: u.layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * bytes_per_texel),
                    rows_per_image: Some(u.rows),
                },
                wgpu::Extent3d {
                    width,
                    height: u.rows,
                    depth_or_array_layers: 1,
                },
            );
            budget = budget.saturating_sub(bytes.len());
        }
        pending.is_empty()
    }

    fn done(&self) -> bool {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }
}

/// A ring drawn or on its way: the maps, their textures once made, and the bind group over them.
struct Ring {
    maps: Arc<HorizonMaps>,
    textures: Option<Arc<RingTextures>>,
    group: Option<wgpu::BindGroup>,
}

/// What the ring drew last frame, for the host's status.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HorizonCounts {
    pub patches: usize,
    /// Whether a ring is drawn, and one on its way (uploading).
    pub drawn: bool,
    pub uploading: bool,
    /// Bytes of its textures on the GPU.
    pub bytes: usize,
}

/// The horizon ring as a renderer-core layer: drawn in the `Opaque` phase after the land, casting nothing.
pub struct HorizonLayer {
    pipeline: PipelineId,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    instances: wgpu::Buffer,
    patch_count: u32,
    environment: wgpu::BindGroup,
    cloud_group: wgpu::BindGroup,
    cloud_placeholder: wgpu::BindGroup,
    cloud_bound: bool,
    uniforms: HorizonUniforms,
    drawn: Option<Ring>,
    next: Option<Ring>,
    /// World x and z of theatre point (0, 0).
    offset: [f64; 2],
    /// The window's square (theatre centre, half side), when the scene has one.
    hole: Option<([f64; 2], f64)>,
    shown: bool,
    patches: Vec<[f32; 4]>,
}

/// The patch mesh: (`PATCH_QUADS` + 1)² vertices a unit apart, every diagonal from (i, j) to (i + 1, j + 1) as the
/// land's patches are cut.
fn patch_mesh() -> (Vec<f32>, Vec<u16>) {
    let n = PATCH_QUADS;
    let mut vertices = Vec::with_capacity(((n + 1) * (n + 1) * 2) as usize);
    for j in 0..=n {
        for i in 0..=n {
            vertices.extend_from_slice(&[i as f32, j as f32]);
        }
    }
    let mut indices = Vec::with_capacity((n * n * 6) as usize);
    for j in 0..n {
        for i in 0..n {
            let a = (j * (n + 1) + i) as u16;
            let (b, c) = (a + 1, a + n as u16 + 1);
            let d = c + 1;
            indices.extend_from_slice(&[a, c, d, a, d, b]);
        }
    }
    (vertices, indices)
}

impl HorizonLayer {
    /// The layer's pipeline and buffers, drawing nothing until maps are handed in ([`Self::set_maps`]). `environment`
    /// is the sky's environment bind group (`naval_sky::EnvironmentMap::bind_group`).
    pub fn new(
        ctx: &mut RenderContext<'_>,
        environment: &wgpu::BindGroup,
    ) -> Result<Self, ShaderError> {
        crate::layer::register_shaders(ctx)?;
        ctx.add_shader(shader_file!("shaders/horizon.wgsl"))?;
        let device = ctx.device.clone();
        let visible = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let array = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: visible,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("horizon"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: visible,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                array(1),
                array(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: visible,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("horizon"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("horizon uniforms"),
            size: std::mem::size_of::<HorizonUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (mesh, index_list) = patch_mesh();
        let buffer = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            ctx.queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let vertices = buffer(
            "horizon patch",
            bytemuck::cast_slice(&mesh),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer(
            "horizon patch indices",
            bytemuck::cast_slice(&index_list),
            wgpu::BufferUsages::INDEX,
        );
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("horizon patches"),
            size: (PATCH_CAPACITY * PATCH_FLOATS * 4) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let environment_layout = naval_sky::environment_bind_group_layout(&device);
        let cloud_layout = cloud_shadow_layout(&device);
        let placeholder = crate::textures::texture_with_levels(
            &device,
            ctx.queue,
            "horizon cloud shadow placeholder",
            wgpu::TextureFormat::Rgba8Unorm,
            1,
            1,
            &[&[255, 255, 255, 255]],
            wgpu::TextureUsages::empty(),
        );
        let placeholder_view = placeholder.create_view(&wgpu::TextureViewDescriptor::default());
        let placeholder_uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("horizon cloud shadow placeholder"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let cloud_placeholder = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("horizon cloud shadow placeholder"),
            layout: &cloud_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&placeholder_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: placeholder_uniforms.as_entire_binding(),
                },
            ],
        });
        let attribute = |format, offset, shader_location| wgpu::VertexAttribute {
            format,
            offset,
            shader_location,
        };
        let pipeline = ctx.add_scene_pipeline(
            "terrain horizon",
            "naval-terrain/shaders/horizon.wgsl",
            ScenePipeline {
                vertex_buffers: vec![
                    VertexLayout {
                        stride: 8,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: vec![attribute(wgpu::VertexFormat::Float32x2, 0, 0)],
                    },
                    VertexLayout {
                        stride: (PATCH_FLOATS * 4) as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: vec![attribute(wgpu::VertexFormat::Float32x4, 0, 1)],
                    },
                ],
                bind_group_layouts: vec![layout.clone(), environment_layout, cloud_layout],
                ..ScenePipeline::new("vs_horizon", "fs_horizon")
            },
        )?;
        let mut layer = Self {
            pipeline,
            layout,
            sampler,
            uniform_buffer,
            vertices,
            indices,
            index_count: index_list.len() as u32,
            instances,
            patch_count: 0,
            environment: environment.clone(),
            cloud_group: cloud_placeholder.clone(),
            cloud_placeholder,
            cloud_bound: false,
            uniforms: HorizonUniforms::zeroed(),
            drawn: None,
            next: None,
            offset: [0.0, 0.0],
            hole: None,
            shown: true,
            patches: Vec::with_capacity(PATCH_CAPACITY),
        };
        layer.set_light(&TerrainLight {
            light: naval_sky::CelestialLight {
                direction: Vec3::new(0.0, 1.0, 0.0),
                color: Vec3::ONE,
                intensity: 5.0,
                night: false,
            },
            ambient: 0.65,
            flash: 0.0,
        });
        layer.set_climate(None, [0.0, 1.0]);
        Ok(layer)
    }

    /// The maps to draw: the ring drawn now (if any) stays until they are all on the GPU. The same maps again change
    /// nothing.
    pub fn set_maps(&mut self, maps: &Arc<HorizonMaps>) {
        let holds = |ring: &Option<Ring>| ring.as_ref().is_some_and(|r| r.maps.id == maps.id);
        if holds(&self.drawn) || holds(&self.next) {
            return;
        }
        self.next = Some(Ring {
            maps: maps.clone(),
            textures: None,
            group: None,
        });
    }

    /// The maps drawn now, or on their way.
    pub fn maps(&self) -> Option<&Arc<HorizonMaps>> {
        self.next.as_ref().or(self.drawn.as_ref()).map(|r| &r.maps)
    }

    /// The maps drawn now.
    pub fn drawn_maps(&self) -> Option<&Arc<HorizonMaps>> {
        self.drawn.as_ref().map(|r| &r.maps)
    }

    /// Where the theatre stands in the world (world x, z = theatre x, z + `offset`), and the window's square (its
    /// centre in the theatre and half its side), where the window draws the land and the ring sinks under it.
    pub fn place(&mut self, offset: [f64; 2], hole: Option<([f64; 2], f64)>) {
        self.offset = offset;
        self.hole = hole;
    }

    /// Draws or hides the ring (the deck's chart over the scene, a before-and-after capture).
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// The clouds' shadow (naval-clouds' `CloudShadow` bind group), or none.
    pub fn set_cloud_shadow(&mut self, bind_group: Option<&wgpu::BindGroup>) {
        self.cloud_bound = bind_group.is_some();
        self.cloud_group = bind_group
            .cloned()
            .unwrap_or_else(|| self.cloud_placeholder.clone());
    }

    /// The season's snow, as the window's land lays it (`surface.wgsl`'s `climate_snow`): the snowline, the thaw
    /// below it and the melt that sheds it from gentler faces (the climate's leaves) where the region's style takes the
    /// season (none: the tropical and volcanic styles), and the direction toward the equator in the chart.
    pub fn set_climate(&mut self, snow: Option<(f32, f32, f32)>, equator: [f32; 2]) {
        let (snowline, thaw, melt) = snow.unwrap_or((1e9, 0.0, 0.0));
        self.uniforms.climate = [snowline, thaw, equator[0], equator[1]];
        self.uniforms.season = [melt, 0.0, 0.0, 0.0];
        self.uniforms.sun_direction[3] = if snow.is_some() { 1.0 } else { 0.0 };
    }

    /// The celestial light this frame, as the land takes it ([`crate::TerrainLayer::set_light`]).
    pub fn set_light(&mut self, light: &TerrainLight) {
        let celestial = light.light;
        let (sun_share, fill_share, sky_share) = mesh_light_shares(
            f64::from(celestial.intensity),
            celestial.night,
            f64::from(light.flash),
        );
        let radiance = celestial.color * celestial.intensity * sun_share as f32;
        let fill = light.ambient * fill_share as f32;
        let (sky, ground) = naval_conditions::MESH_FILL_COLORS;
        let sky = naval_conditions::srgb_hex(sky);
        let ground = naval_conditions::srgb_hex(ground);
        let direction = celestial.direction.normalize_or(Vec3::Y);
        let u = &mut self.uniforms;
        u.sun_direction = [direction.x, direction.y, direction.z, u.sun_direction[3]];
        u.sun_radiance = [radiance.x, radiance.y, radiance.z, sky_share as f32];
        u.fill_sky = [sky[0] * fill, sky[1] * fill, sky[2] * fill, u.fill_sky[3]];
        u.fill_ground = [ground[0] * fill, ground[1] * fill, ground[2] * fill, 0.0];
    }

    /// What it drew last frame.
    pub fn counts(&self) -> HorizonCounts {
        HorizonCounts {
            patches: self.patch_count as usize,
            drawn: self.drawn.as_ref().is_some_and(|r| r.group.is_some()),
            uploading: self.next.is_some(),
            bytes: self.drawn.as_ref().map_or(0, |r| r.maps.bytes()),
        }
    }

    /// Whether a ring with land is drawn: views draw to [`FAR_M`] while one is.
    pub fn draws_land(&self) -> bool {
        self.shown
            && self
                .drawn
                .as_ref()
                .is_some_and(|r| r.group.is_some() && r.maps.land)
    }

    fn bind(&self, device: &wgpu::Device, textures: &RingTextures) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("horizon"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&textures.heights_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&textures.colours_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// The next maps' uploads this frame; once they are all in, they are drawn in place of the last. Each frame's
    /// prepare calls it; a scene's build calls it too, so maps another scene already holds on the GPU (shared, whole)
    /// are drawn from its first frame.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(next) = &mut self.next else {
            return;
        };
        let textures = next
            .textures
            .get_or_insert_with(|| RingTextures::for_maps(device, &next.maps))
            .clone();
        if textures.done() || textures.upload(queue, &next.maps, UPLOAD_BUDGET) {
            let group = self.bind(device, &textures);
            let mut ring = self.next.take().expect("the next ring");
            ring.group = Some(group);
            self.drawn = Some(ring);
        }
    }
}

impl SceneLayer for HorizonLayer {
    fn label(&self) -> &str {
        "horizon"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        self.upload(ctx.device, ctx.queue);
        self.patch_count = 0;
        let camera = ctx.camera;
        let Some(ring) = &self.drawn else {
            return;
        };
        // Not over the chart (a diagram of the sea, drawn flat), nor hidden.
        if !self.shown || camera.curvature <= 0.0 || !ring.maps.land {
            return;
        }
        let maps = &ring.maps;
        let origin = [
            maps.centre[0] + self.offset[0],
            maps.centre[1] + self.offset[1],
        ];
        // The frustum's planes in the ring's frame: world = ring + origin.
        let planes = frustum_planes(Mat4::from_cols_array(&camera.view_projection))
            .into_iter()
            .map(|[nx, ny, nz, c]| [nx, ny, nz, c + nx * origin[0] + nz * origin[1]])
            .collect();
        let eye = camera.position;
        let view = HorizonView {
            position: [
                f64::from(eye[0]) - origin[0],
                f64::from(eye[1]),
                f64::from(eye[2]) - origin[1],
            ],
            planes,
            fov_y: 2.0 * f64::from(camera.tan_half_fov_y).atan(),
            height_px: f64::from(camera.viewport[1]),
            curvature: f64::from(camera.curvature),
            far: f64::from(camera.far),
        };
        let hole = self.hole.map(|(centre, half)| Hole {
            centre: [centre[0] - maps.centre[0], centre[1] - maps.centre[1]],
            half,
        });
        let mut patches = std::mem::take(&mut self.patches);
        let range0 = select(maps, &view, hole, &mut patches);
        self.patch_count = patches.len() as u32;
        if !patches.is_empty() {
            ctx.uploads
                .write_buffer(&self.instances, 0, bytemuck::cast_slice(&patches));
        }
        self.patches = patches;
        let haze = maps.haze;
        let u = &mut self.uniforms;
        u.origin = [
            origin[0] as f32,
            origin[1] as f32,
            range0 as f32,
            SEA_FLOOR_M,
        ];
        u.levels = [
            maps.levels[0].half as f32,
            maps.levels[1].half as f32,
            maps.levels[0].cell() as f32,
            maps.levels[1].cell() as f32,
        ];
        u.hole = match hole {
            Some(h) => [h.centre[0] as f32, h.centre[1] as f32, h.half as f32, 1.0],
            None => [0.0; 4],
        };
        u.fill_sky[3] = if self.cloud_bound { 1.0 } else { 0.0 };
        u.haze = [haze[0], haze[1], haze[2], 0.0];
        ctx.uploads
            .write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&self.uniforms));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase != Phase::Opaque || self.patch_count == 0 {
            return;
        }
        let Some(group) = self.drawn.as_ref().and_then(|r| r.group.as_ref()) else {
            return;
        };
        pass.set_pipeline(ctx.pipelines.render(self.pipeline));
        pass.set_bind_group(1, group, &[]);
        pass.set_bind_group(2, &self.environment, &[]);
        pass.set_bind_group(3, &self.cloud_group, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_vertex_buffer(1, self.instances.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..self.index_count, 0, 0..self.patch_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Maps of a cone `peak` metres high at `at` (m about the centre), the sea elsewhere at −200 m.
    fn cone(peak: f32, at: [f64; 2]) -> HorizonMaps {
        let n = HEIGHT_TEXELS as usize;
        let level = |half: f64| {
            let cell = 2.0 * half / n as f64;
            let mut h = vec![-200f32; n * n];
            for j in 0..n {
                for i in 0..n {
                    let x = -half + (i as f64 + 0.5) * cell;
                    let z = -half + (j as f64 + 0.5) * cell;
                    let d = (x - at[0]).hypot(z - at[1]);
                    h[j * n + i] = (f64::from(peak) * (1.0 - d / 20_000.0)).max(-200.0) as f32;
                }
            }
            h
        };
        let c = COLOUR_TEXELS as usize;
        HorizonMaps::new(
            [1000.0, -2000.0],
            [level(NEAR_HALF_M), level(FAR_HALF_M)],
            [vec![90u8; c * c * 4], vec![90u8; c * c * 4]],
        )
    }

    fn view(position: [f64; 3]) -> HorizonView {
        HorizonView {
            position,
            planes: Vec::new(),
            fov_y: 55f64.to_radians(),
            height_px: 1080.0,
            curvature: 0.5 / 6_371_000.0,
            far: f64::from(FAR_M),
        }
    }

    #[test]
    fn the_levels_meet_their_quadtree_and_the_far_plane() {
        assert_eq!(node_size(ROOT_LEVEL), 2.0 * FAR_HALF_M);
        assert!(FAR_HALF_M > f64::from(FAR_M));
        // The near level reaches past a sea window's edge however its centre lies within the recentring distance.
        const { assert!(NEAR_HALF_M - NEAR_CELL_M * NEAR_BLEND_TEXELS > 48_000.0 + 40_000.0) };
    }

    #[test]
    fn heights_are_read_as_the_gpu_samples_them_and_blend_between_the_levels() {
        let maps = cone(3000.0, [60_000.0, 0.0]);
        let peak = maps.height([1000.0 + 60_000.0, -2000.0]);
        assert!((peak - 3000.0).abs() < 30.0, "{peak}");
        // Far out only the far level holds it.
        let far = cone(3000.0, [300_000.0, 0.0]);
        let h = far.height([1000.0 + 300_000.0, -2000.0]);
        assert!((h - 3000.0).abs() < 120.0, "{h}");
        assert_eq!(HorizonMaps::near_share([0.0, 0.0]), 1.0);
        assert_eq!(HorizonMaps::near_share([NEAR_HALF_M, 0.0]), 0.0);
        assert!(maps.land && maps.bytes() > 30 << 20);
    }

    #[test]
    fn the_open_sea_and_the_windows_square_draw_nothing() {
        let maps = cone(2000.0, [100_000.0, 0.0]);
        let mut out = Vec::new();
        // Seen from the window's middle: patches only about the cone, none in the window's square.
        select(
            &maps,
            &view([0.0, 22.0, 0.0]),
            Some(Hole {
                centre: [0.0, 0.0],
                half: 48_000.0,
            }),
            &mut out,
        );
        assert!(!out.is_empty());
        let hole = Hole {
            centre: [0.0, 0.0],
            half: 48_000.0,
        };
        for p in &out {
            let size = node_size(p[2] as u32) / 2.0;
            assert!(!hole.holds(f64::from(p[0]), f64::from(p[1]), size));
            let cx = f64::from(p[0]) + size / 2.0;
            let cz = f64::from(p[1]) + size / 2.0;
            assert!((cx - 100_000.0).hypot(cz).abs() < 20_000.0 + size, "{p:?}");
        }
        // A cone 2 km high 300 km off is below the horizon from a bridge, and above it from 3 km up.
        let far = cone(2000.0, [300_000.0, 0.0]);
        select(&far, &view([0.0, 22.0, 0.0]), None, &mut out);
        assert!(out.is_empty());
        select(&far, &view([0.0, 3000.0, 0.0]), None, &mut out);
        assert!(!out.is_empty());
    }

    #[test]
    fn levels_coarsen_with_distance() {
        let maps = cone(3000.0, [0.0, 0.0]);
        let mut out = Vec::new();
        select(&maps, &view([0.0, 3000.0, 0.0]), None, &mut out);
        let finest = out.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert_eq!(finest, 0.0);
        select(&maps, &view([-200_000.0, 3000.0, 0.0]), None, &mut out);
        let finest = out.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(finest >= 2.0, "{finest}");
    }
}
