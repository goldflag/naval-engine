//! The trees in the sun's shadow maps (native/docs/terrain/trees.md, "Shadows"): which maps the placed trees cast
//! into this frame, and the level-0 patches inside those maps that the view left out, whose trees the placement stands
//! only so their shadows fall into the view. The caster draw itself is the layer's (`TerrainLayer::draw_shadow`, with
//! `tree_caster.wgsl`); the shade beyond the maps is `forest.wgsl`'s.

use std::collections::HashSet;

use glam::{Vec2, Vec3};
use naval_render::{ShadowMap, ShadowMapKind};

use crate::field::HeightBounds;
use crate::lod::{PATCH_FLOATS, TerrainLod, node_size};

/// The maps the trees cast into: the near, the wide and the aboard map (the hull past the near map's first metres
/// while the camera stands aboard: a berth's trees throw their shadows on her deck). The view maps (squares about distant hulls the wide map
/// misses) take none: a forest there is kilometres off, where a crown is a few texels and its shadow a few pixels, and
/// the forest's shade stands in.
pub const TREE_CAST_MAPS: [ShadowMapKind; 3] = [
    ShadowMapKind::Near,
    ShadowMapKind::Wide,
    ShadowMapKind::Aboard,
];

/// The farthest (apparent m) a map's target may lie from the camera for the trees to cast into it: the trees' full
/// density distance (`TREE_FULL_M`). Past it the trees thin out, and a 10 m crown's shadow spans about 2 pixels at
/// 1600 × 900 through the game's lens; the forest's shade stands in (through binoculars the reach grows as the trees').
pub const TREE_SHADOW_REACH_M: f32 = crate::trees::TREE_FULL_M;
/// The tallest a tree's crown stands above the ground (m), for the maps' footprint on the land.
const TREE_TOP_M: f32 = 45.0;
/// The most the shadow-only patches reach from a map's centre across the land (m): a sun on the horizon stretches the
/// maps' footprint without bound.
const FOOTPRINT_MAX_M: f32 = 3000.0;

/// Which maps the trees cast into, and those maps' world-to-clip matrices, as the trees were last placed for them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TreeCasts {
    pub casts: [bool; 3],
    pub matrices: [[f32; 16]; 3],
}

impl TreeCasts {
    /// The maps among this frame's `maps` that the trees cast into (`allowed`: trees, shadows on, the land drawn): those
    /// on whose target lies within [`TREE_SHADOW_REACH_M`] of the camera at `eye` (world metres), apparent by the lens's
    /// `zoom` as the trees' level of detail takes it.
    pub fn of(maps: &[ShadowMap], allowed: bool, eye: Vec3, zoom: f32) -> Self {
        let mut out = Self::default();
        if !allowed {
            return out;
        }
        for (k, kind) in TREE_CAST_MAPS.iter().enumerate() {
            if let Some(map) = maps.iter().find(|m| {
                m.kind == *kind && m.on && m.target.distance(eye) * zoom <= TREE_SHADOW_REACH_M
            }) {
                out.casts[k] = true;
                out.matrices[k] = (map.projection * map.view).to_cols_array();
            }
        }
        out
    }

    pub fn any(&self) -> bool {
        self.casts.iter().any(|c| *c)
    }

    /// Whether the trees cast into `kind` this frame.
    pub fn casts_into(&self, kind: ShadowMapKind) -> bool {
        TREE_CAST_MAPS
            .iter()
            .position(|k| *k == kind)
            .is_some_and(|k| self.casts[k])
    }
}

/// The land a map's light box can shade, in world x and z: the bounds of the box's part between the sea and the trees'
/// tops (its corners in that slab, and where its twelve edges cross the slab's two faces), at most
/// [`FOOTPRINT_MAX_M`] from the map's target.
fn footprint(map: &ShadowMap) -> Option<(Vec2, Vec2)> {
    let world = map.view.inverse();
    let (right, up) = (world.x_axis.truncate(), world.y_axis.truncate());
    let forward = -world.z_axis.truncate();
    let corner = |k: usize| -> Vec3 {
        let x = if k & 1 == 0 { -1.0 } else { 1.0 };
        let y = if k & 2 == 0 { -1.0 } else { 1.0 };
        let t = if k & 4 == 0 { map.near } else { map.far };
        map.position + right * (x * map.half_width) + up * (y * map.half_height) + forward * t
    };
    let corners: Vec<Vec3> = (0..8).map(corner).collect();
    let mut points: Vec<Vec3> = corners
        .iter()
        .copied()
        .filter(|c| (0.0..=TREE_TOP_M).contains(&c.y))
        .collect();
    // The edges join corners one bit apart.
    for a in 0..8 {
        for bit in [1, 2, 4] {
            let b = a | bit;
            if b == a {
                continue;
            }
            let (p, q) = (corners[a], corners[b]);
            for height in [0.0, TREE_TOP_M] {
                if (p.y - height) * (q.y - height) <= 0.0 && p.y != q.y {
                    points.push(p + (q - p) * ((height - p.y) / (q.y - p.y)));
                }
            }
        }
    }
    let target = Vec2::new(map.target.x, map.target.z);
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for point in points {
        let flat =
            target + (Vec2::new(point.x, point.z) - target).clamp_length_max(FOOTPRINT_MAX_M);
        min = min.min(flat);
        max = max.max(flat);
    }
    (min.x <= max.x).then_some((min, max))
}

/// The level-0 patches ([`PATCH_FLOATS`] each: corner x, z, level 0, land) with land inside the maps the trees cast
/// into that no drawn patch carrying trees (`drawn`, level 0 or 1) covers, at most `capacity`: the view left them out,
/// but their trees' shadows may fall into it.
pub fn shadow_patches(
    maps: &[ShadowMap],
    casts: &TreeCasts,
    lod: &TerrainLod,
    bounds: &HeightBounds,
    offset: [f64; 2],
    drawn: &[f32],
    capacity: usize,
) -> Vec<f32> {
    let chart = &lod.chart;
    let size = node_size(&lod.settings, 0) / 2.0;
    let index = |v: f32, origin: f64| ((f64::from(v) - origin) / size).round() as i64;
    let mut covered = HashSet::new();
    for patch in drawn.as_chunks::<PATCH_FLOATS>().0 {
        let (i, j) = (
            index(patch[0], chart.origin_x),
            index(patch[1], chart.origin_z),
        );
        let span = 1i64 << (patch[2].max(0.0) as u32);
        for dj in 0..span {
            for di in 0..span {
                covered.insert((i + di, j + dj));
            }
        }
    }
    let base = bounds.base as f64;
    let level = &bounds.levels[0];
    let blocks = (size / (base * chart.cell)) as i64;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (k, kind) in TREE_CAST_MAPS.iter().enumerate() {
        if !casts.casts[k] {
            continue;
        }
        let Some(map) = maps.iter().find(|m| m.kind == *kind) else {
            continue;
        };
        let Some((min, max)) = footprint(map) else {
            continue;
        };
        let first = |v: f32, offset: f64, origin: f64| {
            ((f64::from(v) - offset - origin) / size).floor().max(0.0) as i64
        };
        let (i0, i1) = (
            first(min.x, offset[0], chart.origin_x),
            first(max.x, offset[0], chart.origin_x),
        );
        let (j0, j1) = (
            first(min.y, offset[1], chart.origin_z),
            first(max.y, offset[1], chart.origin_z),
        );
        for j in j0..=j1 {
            for i in i0..=i1 {
                let x0 = chart.origin_x + i as f64 * size;
                let z0 = chart.origin_z + j as f64 * size;
                if x0 >= chart.origin_x + chart.width
                    || z0 >= chart.origin_z + chart.depth
                    || covered.contains(&(i, j))
                    || !seen.insert((i, j))
                {
                    continue;
                }
                // Land in the patch: the bound pyramid's base blocks under it.
                let mut high = f32::NEG_INFINITY;
                for bj in j * blocks..((j + 1) * blocks).min(level.size as i64) {
                    for bi in i * blocks..((i + 1) * blocks).min(level.size as i64) {
                        high = high.max(level.max[bj as usize * level.size + bi as usize]);
                    }
                }
                if high > 1.5 && out.len() < capacity * PATCH_FLOATS {
                    out.extend_from_slice(&[x0 as f32, z0 as f32, 0.0, 1.0]);
                }
            }
        }
    }
    out
}

/// The farthest (m) the trees' placement moves a side of the view out for their shadows ([`tree_shadow_margins`]): a
/// 40 m tree's shadow under a 6° sun.
const TREE_MARGIN_MAX_M: f32 = 400.0;
/// The margins move in steps this long (m), so the sun's slow travel places the trees again only now and then.
const TREE_MARGIN_STEP_M: f32 = 25.0;

/// How far each of the view's four sides (`planes`: left, right, bottom, top, then near and far, inward unit normals,
/// n · p + w ≥ 0 inside) must move out for the trees beyond it to throw their shadows into the view, under a sun
/// `toward` (unit, toward the light): the reach of a `height` m tree's shadow from its culling sphere's middle (half its
/// height up) across that side, in [`TREE_MARGIN_STEP_M`] steps up to [`TREE_MARGIN_MAX_M`]. The land maps take the
/// trees placed for the view, so without it a tree just beyond the view's sunward side threw no shadow into it until
/// the camera turned to show it. A side away from the sun moves not at all: the shadows beyond it fall away.
pub fn tree_shadow_margins(planes: &[[f32; 4]; 6], toward: Vec3, height: f32) -> [f32; 4] {
    if toward.y <= 0.01 {
        return [0.0; 4];
    }
    // From the middle to where the top's shadow falls on the ground beneath the foot.
    let reach = Vec3::Y * (height / 2.0) - toward * (height / toward.y);
    std::array::from_fn(|k| {
        let normal = Vec3::new(planes[k][0], planes[k][1], planes[k][2]);
        let margin = normal.dot(reach).clamp(0.0, TREE_MARGIN_MAX_M);
        (margin / TREE_MARGIN_STEP_M).ceil() * TREE_MARGIN_STEP_M
    })
}

/// `planes` with the four sides moved out by `margins` (m).
pub fn moved_out(planes: [[f32; 4]; 6], margins: [f32; 4]) -> [[f32; 4]; 6] {
    let mut out = planes;
    for (plane, margin) in out.iter_mut().zip(margins) {
        plane[3] += margin;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_moves_out_on_its_sunward_sides_by_a_trees_shadow() {
        // A view looking north (-z) along level ground: its left side faces east (+x inward), its right west.
        let mut planes = [[0.0f32; 4]; 6];
        planes[0] = [1.0, 0.0, 0.0, 500.0];
        planes[1] = [-1.0, 0.0, 0.0, 500.0];
        planes[2] = [0.0, 1.0, 0.0, 0.0];
        planes[3] = [0.0, -1.0, 0.0, 300.0];
        // A morning sun 12° up beyond the view's left side: the trees past it throw their shadows east, into the view,
        // a 30 m tree's top 141 m.
        let elevation = 12f32.to_radians();
        let toward = Vec3::new(-elevation.cos(), elevation.sin(), 0.0);
        let margins = tree_shadow_margins(&planes, toward, 30.0);
        assert_eq!(margins[0], 150.0, "{margins:?}");
        assert_eq!(margins[1], 0.0, "the side away from the sun stays");
        // The bottom (the ground's face, up): the shadow falls down onto it from the middle, within the culling sphere.
        assert_eq!(margins[2], 0.0);
        // The top (facing down): the shadow lies below the middle, 15 m.
        assert_eq!(margins[3], 25.0, "{margins:?}");
        let moved = moved_out(planes, margins);
        assert_eq!(moved[0][3], 650.0);
        assert_eq!(moved[1], planes[1]);
        assert_eq!(moved[4], planes[4]);
        // A sun on the horizon throws no shadow the maps hold; a low one moves a side at most 400 m.
        assert_eq!(
            tree_shadow_margins(&planes, Vec3::new(-1.0, 0.0, 0.0), 30.0),
            [0.0; 4]
        );
        let low = 2f32.to_radians();
        let margins = tree_shadow_margins(&planes, Vec3::new(-low.cos(), low.sin(), 0.0), 40.0);
        assert_eq!(margins[0], 400.0);
    }
}
