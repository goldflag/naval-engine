//! The land in the sun's shadow maps (native/docs/terrain/shadows.md, "The land's shadow"): the level-0 patches that
//! can throw a
//! shadow into each map this frame, so cliffs, fjord walls and hills shade the ships, the sea, the trees and the
//! buildings. The caster draw is the layer's (`TerrainLayer::draw_shadow`, with `caster.wgsl`); the land itself keeps
//! its baked shadow (`sunlight`), which `terrain.wgsl` takes with the maps' without shading a slope twice.
//!
//! A map's casters are the patches whose bounds meet its box: its square across the light, from its far plane toward
//! the sun without end, since the caster pass clamps what lies nearer the light than its near plane onto that plane
//! (`caster.wgsl`). A cliff far outside the camera's view, kilometres sunward of a ship under a low sun, still shades
//! her.

use glam::{Mat4, Vec3};
use naval_render::{ShadowMap, ShadowMapKind};

use crate::field::HeightBounds;
use crate::lod::{PATCH_FLOATS, TerrainLod, node_size};

/// The most patches one map takes.
pub const LAND_MAP_PATCHES: usize = 1024;
/// The farthest (m, along either chart axis) beyond a map's square that land is drawn into it: a sun on the horizon
/// throws a shadow without end (a 300 m cliff under a 3° sun throws 5.7 km).
pub const LAND_THROW_MAX_M: f32 = 8000.0;
/// The bound pyramid's level the selection culls first: blocks 2.56 km across on a battle map (8 samples of 40 m,
/// doubled three times), so a footprint kilometres long costs tens of block tests before its patches'.
const COARSE_LEVEL: usize = 3;

/// How far (m) the land's caster stands back along the light beyond its slope's share (`caster.wgsl`), in each map: the
/// land's own surface must never shade itself through the maps (its baked shadow does), whose texels and filter spread
/// a slope across several metres of depth. The view maps (squares about hulls kilometres off) are pushed further: the
/// land there may be drawn at a coarser level of detail than the caster's level 0. The land maps (the camera's view of
/// the land out to 4 km, in cascades of texels from tens of centimetres to a few metres) are pushed further still: past
/// the near and wide maps the land's own baked shadow darkens it, and their part is the land's shadow on what stands
/// on it and on the sea, which a few metres more along the light does not move visibly. The farthest land map's
/// caster is a level coarser ([`caster_level`]), so it stands back further again, past where its larger triangles
/// bridge a gully above the drawn land.
pub fn base_push(kind: ShadowMapKind) -> f32 {
    match kind {
        ShadowMapKind::Near
        | ShadowMapKind::Aboard
        | ShadowMapKind::Wide
        | ShadowMapKind::Light { .. } => 2.0,
        ShadowMapKind::View(_) => 6.0,
        ShadowMapKind::Land(_) if caster_level(kind) > 0 => 20.0,
        ShadowMapKind::Land(_) => 10.0,
    }
}

/// The level of detail the land casts into `kind` at: level 0 (20 m triangles on the High terrain) in every map but the
/// farthest land map, whose texels are some 4 m and whose square reaches 8 km across: there level 1 (40 m), a quarter
/// of the triangles (some 600-780 level-0 patches a frame from the air over Los Angeles and Kamakura, 300-400 k
/// triangles, in one map).
pub fn caster_level(kind: ShadowMapKind) -> u32 {
    match kind {
        ShadowMapKind::Land(i) if i + 1 == naval_render::shadows::LAND_CASCADES => 1,
        _ => 0,
    }
}

/// The patches ([`PATCH_FLOATS`] each: chart corner x, z, the map's [`caster_level`], and its [`base_push`]) whose land
/// above the sea can throw a shadow into `map`, at most `capacity`. `offset`: world = chart + offset. The land's bounds
/// are widened by the fine relief's [`TerrainLod::relief_margin`], as the view's selection widens them: the caster
/// draws the relief, and a crest it lifts may reach a map the window's heights do not.
pub fn land_patches(
    map: &ShadowMap,
    lod: &TerrainLod,
    bounds: &HeightBounds,
    offset: [f64; 2],
    capacity: usize,
) -> Vec<f32> {
    let mut out = Vec::new();
    let world = map.view.inverse();
    let toward = world.z_axis.truncate().normalize_or_zero();
    // A light at or below the horizon shades everything, and the maps then show nothing.
    if toward.y <= 0.01 || bounds.levels.is_empty() {
        return out;
    }
    let margin = lod.relief_margin as f32;
    let top = bounds.levels.last().map_or(0.0, |l| l.max[0]);
    if top <= 0.0 {
        return out;
    }
    let Some((min, max)) = footprint(map, toward, top + margin) else {
        return out;
    };
    let chart = &lod.chart;
    let caster = caster_level(map.kind);
    let size = node_size(&lod.settings, caster) / 2.0;
    let origin = [chart.origin_x, chart.origin_z];
    let extent = [chart.width, chart.depth];
    // The cells of `step` metres from the chart's origin that the footprint covers along axis `a`, of `count`.
    let span = |a: usize, step: f64, count: usize| {
        let first = ((f64::from(min[a]) - offset[a] - origin[a]) / step)
            .floor()
            .max(0.0) as usize;
        let last = ((f64::from(max[a]) - offset[a] - origin[a]) / step)
            .floor()
            .max(-1.0);
        first..((last + 1.0) as usize).min(count)
    };
    // A chart box of the land between `low` and `high` (m) in world metres.
    let world_box = |x0: f64, z0: f64, side: f64, low: f32, high: f32| {
        (
            Vec3::new(
                (x0 + offset[0]) as f32,
                low.max(0.0),
                (z0 + offset[1]) as f32,
            ),
            Vec3::new(
                (x0 + side + offset[0]) as f32,
                high,
                (z0 + side + offset[1]) as f32,
            ),
        )
    };
    // The bound pyramid's blocks kilometres across first, then the patches inside those that meet the box: under a low
    // sun the footprint runs kilometres toward the sun, most of it sea or land too low to reach the map.
    let coarse = COARSE_LEVEL.min(bounds.levels.len() - 1);
    let level = &bounds.levels[coarse];
    let block = bounds.base as f64 * chart.cell * f64::from(1u32 << coarse);
    let patches = [
        (extent[0] / size).ceil() as usize,
        (extent[1] / size).ceil() as usize,
    ];
    let mut candidates = Vec::new();
    for bj in span(1, block, level.size) {
        for bi in span(0, block, level.size) {
            let (low, high) = (
                level.min[bj * level.size + bi],
                level.max[bj * level.size + bi],
            );
            let (x0, z0) = (origin[0] + bi as f64 * block, origin[1] + bj as f64 * block);
            let (lo, hi) = world_box(x0, z0, block, low - margin, high + margin);
            if high <= 0.0 || !meets_box(map, &map.view, lo, hi) {
                continue;
            }
            let inside = |a: usize, start: f64| {
                let first = ((start - origin[a]) / size).floor().max(0.0) as usize;
                let last = (((start + block - origin[a]) / size).ceil() as usize).min(patches[a]);
                first..last.max(first)
            };
            let (across, down) = (span(0, size, patches[0]), span(1, size, patches[1]));
            for j in inside(1, z0).filter(|j| down.contains(j)) {
                for i in inside(0, x0).filter(|i| across.contains(i)) {
                    candidates.push((j, i));
                }
            }
        }
    }
    candidates.sort_unstable();
    candidates.dedup();
    // Nearest the map's target first, so a selection cut short at `capacity` (a sun a few degrees up over hills, its
    // footprint kilometres long) keeps the land about the square and gives up the farthest throws.
    let centre = [
        f64::from(map.target.x) - offset[0] - origin[0] - size / 2.0,
        f64::from(map.target.z) - offset[1] - origin[1] - size / 2.0,
    ];
    let distance = |&(j, i): &(usize, usize)| {
        let (dx, dz) = (i as f64 * size - centre[0], j as f64 * size - centre[1]);
        dx * dx + dz * dz
    };
    candidates.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    let base = &bounds.levels[0];
    let base_block = bounds.base as f64 * chart.cell;
    let push = base_push(map.kind);
    for (j, i) in candidates {
        let (x0, z0) = (origin[0] + i as f64 * size, origin[1] + j as f64 * size);
        // The patch's heights: the bound pyramid's base blocks under it.
        let blocks = |start: f64, a: usize| {
            let first = ((start - origin[a]) / base_block).floor().max(0.0) as usize;
            let last = (((start + size - origin[a]) / base_block).ceil() as usize).min(base.size);
            first..last.max(first)
        };
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for bj in blocks(z0, 1) {
            for bi in blocks(x0, 0) {
                low = low.min(base.min[bj * base.size + bi]);
                high = high.max(base.max[bj * base.size + bi]);
            }
        }
        // Land below the sea shades only the seabed, which its own baked shadow darkens.
        if high <= 0.0 {
            continue;
        }
        let (lo, hi) = world_box(x0, z0, size, low - margin, high + margin);
        if !meets_box(map, &map.view, lo, hi) {
            continue;
        }
        if out.len() / PATCH_FLOATS >= capacity {
            break;
        }
        out.extend_from_slice(&[x0 as f32, z0 as f32, caster as f32, push]);
    }
    out
}

/// Whether the world box `lo`–`hi` meets `map`'s box: inside its square across the light, and not wholly past its far
/// plane (nearer the light than its near plane counts: the caster pass clamps it there).
fn meets_box(map: &ShadowMap, view: &Mat4, lo: Vec3, hi: Vec3) -> bool {
    let (mut min, mut max) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
    for k in 0..8 {
        let corner = Vec3::new(
            if k & 1 == 0 { lo.x } else { hi.x },
            if k & 2 == 0 { lo.y } else { hi.y },
            if k & 4 == 0 { lo.z } else { hi.z },
        );
        let p = view.transform_point3(corner);
        min = min.min(p);
        max = max.max(p);
    }
    // The light looks down its −z: depth is −z.
    max.x >= -map.half_width
        && min.x <= map.half_width
        && max.y >= -map.half_height
        && min.y <= map.half_height
        && -max.z <= map.far
}

/// The chart can shade `map` from within these world x and z: the bounds of its square's four corner lines, along the
/// light from where they cross the sea to where they cross the land's highest point `top`, at most
/// [`LAND_THROW_MAX_M`] beyond its square (its half-diagonal from its target) along either axis: a land map through a
/// lens holds a square kilometres across (11.6 km at ×6), all of whose land casts.
fn footprint(map: &ShadowMap, toward: Vec3, top: f32) -> Option<([f32; 2], [f32; 2])> {
    let world = map.view.inverse();
    let (right, up) = (world.x_axis.truncate(), world.y_axis.truncate());
    let (mut min, mut max) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for k in 0..4 {
        let x = if k & 1 == 0 {
            -map.half_width
        } else {
            map.half_width
        };
        let y = if k & 2 == 0 {
            -map.half_height
        } else {
            map.half_height
        };
        let corner = map.target + right * x + up * y;
        for height in [0.0, top] {
            let along = (height - corner.y) / toward.y;
            let point = corner + toward * along;
            for (a, v) in [point.x, point.z].into_iter().enumerate() {
                min[a] = min[a].min(v);
                max[a] = max[a].max(v);
            }
        }
    }
    let target = [map.target.x, map.target.z];
    let reach = LAND_THROW_MAX_M + map.half_width.max(map.half_height) * std::f32::consts::SQRT_2;
    for a in 0..2 {
        min[a] = min[a].max(target[a] - reach);
        max[a] = max[a].min(target[a] + reach);
    }
    (min[0] <= max[0] && min[1] <= max[1]).then_some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::height_bounds;
    use crate::heightfield::Heightfield;
    use crate::lod::{TerrainChart, TerrainQuality};
    use naval_render::shadows::ShadowFit;
    use naval_render::{Camera, ShadowSettings, Sun};

    /// A field `side` metres square about the origin at 40 m samples, its heights `height(x, z)`.
    fn field_of(side: f32, height: impl Fn(f32, f32) -> f32) -> (TerrainLod, HeightBounds) {
        let cell = 40.0;
        let samples = (side / cell) as u32 + 1;
        let half = side / 2.0;
        let mut heights = Vec::with_capacity((samples * samples) as usize);
        for j in 0..samples {
            for i in 0..samples {
                heights.push(height(-half + i as f32 * cell, -half + j as f32 * cell).round());
            }
        }
        let field = Heightfield {
            columns: samples,
            rows: samples,
            cell,
            origin_x: -half,
            origin_z: -half,
            step: 1.0,
            quanta: heights.iter().map(|h| *h as i16).collect(),
        };
        let bounds = height_bounds(&field, &heights, crate::field::BOUNDS_BASE);
        let chart = TerrainChart {
            origin_x: f64::from(-half),
            origin_z: f64::from(-half),
            width: f64::from(side),
            depth: f64::from(side),
            bounds: bounds.clone(),
            cell: f64::from(cell),
        };
        let lod = TerrainLod::new(chart, TerrainQuality::HIGH.lod).unwrap();
        (lod, bounds)
    }

    /// A flat sea 40 m deep, 8 km square about the origin, with a 300 m wall standing along x = `wall` (20 m thick).
    fn walled(wall: f32) -> ((), TerrainLod, HeightBounds) {
        let (lod, bounds) = field_of(8000.0, |x, _| {
            if (x - wall).abs() <= 20.0 {
                300.0
            } else {
                -40.0
            }
        });
        ((), lod, bounds)
    }

    /// Every patch of the chart tested alone (the selection without its coarse pass), for comparison.
    fn every_patch(map: &ShadowMap, lod: &TerrainLod, bounds: &HeightBounds) -> Vec<(i64, i64)> {
        let size = node_size(&lod.settings, 0) / 2.0;
        let chart = &lod.chart;
        let base = &bounds.levels[0];
        let blocks = (size / (bounds.base as f64 * chart.cell)) as usize;
        let count = (chart.width / size).ceil() as usize;
        let toward = map.view.inverse().z_axis.truncate().normalize();
        let footprint = footprint(map, toward, bounds.levels.last().unwrap().max[0]);
        let mut out = Vec::new();
        for j in 0..count {
            for i in 0..count {
                let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
                for bj in j * blocks..((j + 1) * blocks).min(base.size) {
                    for bi in i * blocks..((i + 1) * blocks).min(base.size) {
                        low = low.min(base.min[bj * base.size + bi]);
                        high = high.max(base.max[bj * base.size + bi]);
                    }
                }
                let (x0, z0) = (
                    (chart.origin_x + i as f64 * size) as f32,
                    (chart.origin_z + j as f64 * size) as f32,
                );
                let lo = Vec3::new(x0, low.max(0.0), z0);
                let hi = Vec3::new(x0 + size as f32, high, z0 + size as f32);
                // Within the land the box's prism holds (its footprint), and the box test.
                let within = footprint.is_some_and(|(min, max)| {
                    hi.x > min[0] && lo.x < max[0] && hi.z > min[1] && lo.z < max[1]
                });
                if high > 0.0 && within && meets_box(map, &map.view, lo, hi) {
                    out.push((x0 as i64, z0 as i64));
                }
            }
        }
        out
    }

    fn maps(toward: Vec3) -> [ShadowMap; naval_render::shadows::MAP_COUNT] {
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        let camera = Camera::look_at(Vec3::new(0.0, 30.0, 300.0), Vec3::ZERO);
        fit.update(&camera, 16.0 / 9.0, Some(&Sun::new(toward, Vec3::ZERO)));
        fit.maps
    }

    #[test]
    fn a_cliff_far_sunward_of_a_low_sun_casts_into_the_maps_and_land_beyond_them_does_not() {
        // A sun 10° up from +x: a 300 m wall 1.5 km sunward throws its shadow 1.7 km, past the target, from beyond the
        // near and wide boxes' near planes.
        let elevation = 10f32.to_radians();
        let toward = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let maps = maps(toward);
        let (_, lod, bounds) = walled(1500.0);
        for map in [&maps[0], &maps[1]] {
            let patches = land_patches(map, &lod, &bounds, [0.0, 0.0], LAND_MAP_PATCHES);
            assert!(!patches.is_empty(), "{:?} takes the wall", map.kind);
            for patch in patches.chunks(PATCH_FLOATS) {
                // The wall's samples lie in the patches from x = 1440 m (320 m patches from -4 km).
                assert_eq!(patch[0], 1440.0, "{:?}: {patch:?}", map.kind);
                assert_eq!(patch[3], base_push(map.kind));
            }
        }
        // The same wall 3 km down-sun lies past the boxes' far planes and outside their squares.
        let (_, lod, bounds) = walled(-3000.0);
        for map in [&maps[0], &maps[1]] {
            assert!(land_patches(map, &lod, &bounds, [0.0, 0.0], LAND_MAP_PATCHES).is_empty());
        }
    }

    #[test]
    fn the_land_maps_take_the_land_and_the_farthest_a_level_coarser() {
        // A plane 400 m over rolling hills, looking out over them under a 30° sun: every land map takes patches, the
        // farthest at level 1 on its own grid, pushed back further.
        use naval_render::LandView;
        use naval_render::shadows::{LAND_CASCADES, LAND_FIRST};
        let (lod, bounds) = field_of(24_000.0, |x, z| {
            let hills =
                (x / 1700.0).sin() * (z / 2300.0).cos() + 0.5 * (x / 640.0 + z / 910.0).sin();
            hills * 300.0 + 50.0
        });
        let elevation = 30f32.to_radians();
        let mut settings = ShadowSettings::with_map_size(2048);
        settings.land_maps = true;
        let mut fit = ShadowFit::new(settings);
        let camera = Camera::look_at(Vec3::new(0.0, 800.0, 0.0), Vec3::new(2500.0, 0.0, 1000.0));
        let mut sun = Sun::new(
            Vec3::new(elevation.cos(), elevation.sin(), 0.3).normalize(),
            Vec3::new(0.0, 0.0, 30_000.0),
        );
        sun.land = Some(LandView {
            top: 400.0,
            nearest: 0.0,
        });
        fit.update(&camera, 16.0 / 9.0, Some(&sun));
        for i in 0..LAND_CASCADES {
            let map = &fit.maps[LAND_FIRST + i];
            assert!(map.on && map.drawn, "land {i}");
            let level = caster_level(map.kind);
            assert_eq!(level, u32::from(i + 1 == LAND_CASCADES), "land {i}");
            let patches = land_patches(map, &lod, &bounds, [0.0, 0.0], usize::MAX);
            assert!(!patches.is_empty(), "land {i}");
            let size = node_size(&lod.settings, level) / 2.0;
            for patch in patches.chunks(PATCH_FLOATS) {
                assert_eq!((patch[2], patch[3]), (level as f32, base_push(map.kind)));
                for (corner, origin) in [
                    (patch[0], lod.chart.origin_x),
                    (patch[1], lod.chart.origin_z),
                ] {
                    let along = (f64::from(corner) - origin) / size;
                    assert!((along - along.round()).abs() < 1e-6, "land {i}: {patch:?}");
                }
            }
        }
        assert!(
            base_push(fit.maps[LAND_FIRST + LAND_CASCADES - 1].kind)
                > base_push(fit.maps[LAND_FIRST].kind)
        );
    }

    #[test]
    fn a_land_map_through_a_lens_takes_the_land_across_its_whole_square() {
        // Through a ×6 lens, looking level out over the land: the farthest land map's square is wider than the 8 km
        // throw about its target, and the land may cast from all of it.
        use naval_render::LandView;
        use naval_render::shadows::{LAND_CASCADES, LAND_FIRST};
        let mut settings = ShadowSettings::with_map_size(2048);
        settings.land_maps = true;
        let mut fit = ShadowFit::new(settings);
        let mut camera = Camera::look_at(Vec3::new(0.0, 30.0, 0.0), Vec3::new(20_000.0, 0.0, 0.0));
        camera.fov_y_degrees = 2.0 * (26f32.to_radians().tan() / 6.0).atan().to_degrees();
        let elevation = 20f32.to_radians();
        let mut sun = Sun::new(
            Vec3::new(0.0, elevation.sin(), elevation.cos()),
            Vec3::new(0.0, 0.0, -30_000.0),
        );
        sun.lens = 6.0;
        sun.land = Some(LandView {
            top: 300.0,
            nearest: 0.0,
        });
        fit.update(&camera, 16.0 / 9.0, Some(&sun));
        let map = &fit.maps[LAND_FIRST + LAND_CASCADES - 1];
        assert!(
            map.on && map.half_width > LAND_THROW_MAX_M,
            "{}",
            map.half_width
        );
        let toward = map.view.inverse().z_axis.truncate().normalize();
        let (min, max) = footprint(map, toward, 300.0).expect("a footprint");
        // Every point of the sea the camera sees out to 24 km that the map holds lies in the footprint.
        let (tan_v, aspect) = ((camera.fov_y_degrees.to_radians() / 2.0).tan(), 16.0 / 9.0);
        let matrix = map.projection * map.view;
        let mut held = 0;
        for k in 0..=60 {
            let distance = 1000.0 + k as f32 * 400.0;
            for s in -10..=10 {
                let p = Vec3::new(distance, 0.0, s as f32 / 10.0 * distance * tan_v * aspect);
                let clip = matrix.project_point3(p);
                if clip.x.abs() > 1.0 || clip.y.abs() > 1.0 || !(0.0..=1.0).contains(&clip.z) {
                    continue;
                }
                held += 1;
                assert!(
                    (min[0]..=max[0]).contains(&p.x) && (min[1]..=max[1]).contains(&p.z),
                    "{p} outside {min:?}-{max:?}"
                );
            }
        }
        assert!(held > 100, "{held}");
    }

    #[test]
    fn the_chart_offset_moves_the_patches() {
        let elevation = 10f32.to_radians();
        let toward = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let maps = maps(toward);
        let (_, lod, bounds) = walled(1500.0);
        // The chart drawn 3 km further sunward: the wall stands at 4.5 km, its shadow ending 2.8 km out, short of the
        // wide square's footprint on the sea (2.2 km).
        assert!(land_patches(&maps[1], &lod, &bounds, [3000.0, 0.0], LAND_MAP_PATCHES).is_empty());
        // Drawn 400 m back: at 1.1 km, casting.
        assert!(!land_patches(&maps[1], &lod, &bounds, [-400.0, 0.0], LAND_MAP_PATCHES).is_empty());
    }

    #[test]
    fn the_coarse_pass_keeps_every_patch_that_meets_a_map() {
        // Rolling hills to 900 m with sea between, 24 km square, under suns from 3° to 60° on four bearings.
        let (lod, bounds) = field_of(24_000.0, |x, z| {
            let hills =
                (x / 1700.0).sin() * (z / 2300.0).cos() + 0.5 * (x / 640.0 + z / 910.0).sin();
            hills * 600.0 - 150.0
        });
        let mut slowest = 0.0f64;
        for elevation in [3.0f32, 8.0, 20.0, 60.0] {
            for bearing in [0.0f32, 70.0, 135.0, 250.0] {
                let (e, b) = (elevation.to_radians(), bearing.to_radians());
                let toward = Vec3::new(b.sin() * e.cos(), e.sin(), b.cos() * e.cos());
                for map in &maps(toward)[..2] {
                    let started = std::time::Instant::now();
                    let patches = land_patches(map, &lod, &bounds, [0.0, 0.0], usize::MAX);
                    slowest = slowest.max(started.elapsed().as_secs_f64() * 1e3);
                    let chosen: std::collections::HashSet<(i64, i64)> = patches
                        .chunks(PATCH_FLOATS)
                        .map(|p| (p[0] as i64, p[1] as i64))
                        .collect();
                    for patch in every_patch(map, &lod, &bounds) {
                        assert!(
                            chosen.contains(&patch),
                            "{elevation}° {bearing}°: {patch:?} missed"
                        );
                    }
                }
            }
        }
        println!("slowest map's selection: {slowest:.3} ms");
    }
}
