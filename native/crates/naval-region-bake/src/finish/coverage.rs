//! The coastline on the lattice: each lattice point's share of the ocean, from the release's water polygons (the sea,
//! OpenStreetMap's coastline; `land-v1` reads no rivers, docks or lakes as sea).
//!
//! A point's cell is the square of half a step about it each way; its share is how many of [`SUBSAMPLES`]² points
//! across the cell lie inside the polygons' union ([`FULL`] when every one does). The sub-points are a global lattice
//! of their own (sub-row `v` at latitude (v + ½)/6,400°, sub-column `u` at longitude (u + ½)/6,400°), and each polygon
//! is kept in its own longitude frame: a crossing is computed from the polygon's edge and the sub-row's latitude alone,
//! then taken to a sub-column by an integer shift of whole turns. So a point's share is a function of its place and
//! the polygons, the same in every square whose bake reads it, across ±180° too.
use std::sync::Arc;

use naval_land_format::lattice::PER_DEGREE;

use super::lattice::{Plane, Region};
use crate::BakeError;

/// Sub-points a side of a cell.
pub const SUBSAMPLES: usize = 4;
/// A cell wholly in the sea.
pub const FULL: u8 = (SUBSAMPLES * SUBSAMPLES) as u8;
/// A point is sea when at least this many of its sub-points are.
pub const HALF: u8 = FULL / 2;
/// Sub-points a degree.
const SUB_PER_DEGREE: i64 = PER_DEGREE as i64 * SUBSAMPLES as i64;
/// Sub-columns round the world.
const SUB_AROUND: i64 = 360 * SUB_PER_DEGREE;

/// A water polygon: its exterior then its holes, each a ring of [latitude, longitude] in degrees, the longitude
/// continuous along the polygon (it may run past ±180°, never jumping a turn).
#[derive(Clone, Debug, PartialEq)]
pub struct GeoPolygon {
    pub rings: Vec<Vec<[f64; 2]>>,
    /// south, north, west, east.
    pub bounds: [f64; 4],
}

impl GeoPolygon {
    pub fn new(rings: Vec<Vec<[f64; 2]>>) -> Self {
        let mut b = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
        for p in rings.iter().flatten() {
            b = [
                b[0].min(p[0]),
                b[1].max(p[0]),
                b[2].min(p[1]),
                b[3].max(p[1]),
            ];
        }
        assert!(
            b[3] - b[2] < 180.0,
            "a water polygon spans less than half the world"
        );
        Self { rings, bounds: b }
    }

    /// The whole turns (multiples of 360°) by which the polygon's frame meets the longitudes `west` to `east`.
    fn turns(&self, west: f64, east: f64) -> Vec<i64> {
        let lo = ((west - self.bounds[3]) / 360.0).ceil() as i64;
        let hi = ((east - self.bounds[2]) / 360.0).floor() as i64;
        (lo..=hi).collect()
    }
}

/// Where the bake reads the water polygons.
pub trait Water {
    /// Every polygon that may meet latitudes `south` to `north` and longitudes `west` to `east` (continuous, possibly
    /// past ±180°): a polygon whose box meets the range in any whole turn. More is harmless; fewer is wrong.
    fn ocean(
        &self,
        south: f64,
        north: f64,
        west: f64,
        east: f64,
    ) -> Result<Vec<Arc<GeoPolygon>>, BakeError>;
}

/// An edge of a polygon in one turn's frame, south end first.
struct Edge {
    lat0: f64,
    lat1: f64,
    lon0: f64,
    slope: f64,
    /// The polygon's instance (polygon and turn) and the turn's shift in sub-columns.
    instance: u32,
    shift: i64,
}

/// The sub-row's latitude, degrees.
#[inline]
fn sub_lat(v: i64) -> f64 {
    (2 * v + 1) as f64 / (2 * SUB_PER_DEGREE) as f64
}

/// The first sub-column whose centre lies at or east of longitude `x` (in its polygon's frame).
#[inline]
fn sub_column(x: f64) -> i64 {
    (x * SUB_PER_DEGREE as f64 - 0.5).ceil() as i64
}

/// Each point of `region`'s share of the ocean, in [`FULL`]ths.
pub fn ocean_cover(region: &Region, water: &dyn Water) -> Result<Plane<u8>, BakeError> {
    let s = SUBSAMPLES as i64;
    // Sub-rows from the south: v from 4·i_south − 2 to 4·i_north + 1; sub-columns u from 4·left − 2.
    let i_south = region.i(region.rows - 1);
    let (v0, v1) = (s * i64::from(i_south) - 2, s * i64::from(region.top) + 1);
    let u0 = s * i64::from(region.left) - 2;
    let sub_columns = region.columns * SUBSAMPLES;
    let per = f64::from(PER_DEGREE);
    let (south, north) = (sub_lat(v0), sub_lat(v1));
    let (west, east) = (
        (f64::from(region.left) - 0.5) / per,
        (f64::from(region.j(region.columns - 1)) + 0.5) / per,
    );
    let polygons = water.ocean(south, north, west, east)?;
    let mut edges = vec![];
    let mut instances = 0u32;
    for polygon in &polygons {
        if polygon.bounds[1] < south || polygon.bounds[0] > north {
            continue;
        }
        for turn in polygon.turns(west, east) {
            for ring in &polygon.rings {
                let n = ring.len();
                for e in 0..n {
                    let (a, b) = (ring[e], ring[(e + 1) % n]);
                    if a[0] == b[0] {
                        continue;
                    }
                    let (lo, hi) = if a[0] < b[0] { (a, b) } else { (b, a) };
                    if hi[0] < south || lo[0] > north {
                        continue;
                    }
                    edges.push(Edge {
                        lat0: lo[0],
                        lat1: hi[0],
                        lon0: lo[1],
                        slope: (hi[1] - lo[1]) / (hi[0] - lo[0]),
                        instance: instances,
                        shift: turn * SUB_AROUND,
                    });
                }
            }
            instances += 1;
        }
    }
    edges.sort_by(|a, b| a.lat0.total_cmp(&b.lat0));
    let mut out = Plane::filled(*region, 0u8);
    let mut flags = vec![false; sub_columns];
    let mut active: Vec<usize> = vec![];
    let mut crossings: Vec<(u32, i64)> = vec![];
    let mut next = 0;
    for v in v0..=v1 {
        let lat = sub_lat(v);
        while next < edges.len() && edges[next].lat0 <= lat {
            active.push(next);
            next += 1;
        }
        active.retain(|&e| edges[e].lat1 > lat);
        if active.is_empty() {
            continue;
        }
        crossings.clear();
        crossings.extend(active.iter().map(|&e| {
            let edge = &edges[e];
            let x = edge.lon0 + (lat - edge.lat0) * edge.slope;
            (edge.instance, sub_column(x) + edge.shift - u0)
        }));
        crossings.sort_unstable();
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        let mut k = 0;
        while k < crossings.len() {
            let instance = crossings[k].0;
            let mut end = k;
            while end < crossings.len() && crossings[end].0 == instance {
                end += 1;
            }
            let mut p = k;
            while p + 1 < end {
                let clamp = |u: i64| u.clamp(0, sub_columns as i64) as usize;
                let (c0, c1) = (clamp(crossings[p].1), clamp(crossings[p + 1].1));
                if c1 > c0 {
                    flags[c0..c1].fill(true);
                    lo = lo.min(c0);
                    hi = hi.max(c1);
                }
                p += 2;
            }
            k = end;
        }
        if lo >= hi {
            continue;
        }
        // Sub-row v belongs to lattice row i = floor((v + 2) / 4).
        let i = (v + 2).div_euclid(s) as i32;
        let r = (region.top - i) as usize;
        for (u, flag) in flags.iter_mut().enumerate().take(hi).skip(lo) {
            if *flag {
                let c = u / SUBSAMPLES;
                out.values[r * region.columns + c] += 1;
                *flag = false;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_land_format::SquareKey;

    struct List(Vec<Arc<GeoPolygon>>);
    impl Water for List {
        fn ocean(&self, _: f64, _: f64, _: f64, _: f64) -> Result<Vec<Arc<GeoPolygon>>, BakeError> {
            Ok(self.0.clone())
        }
    }

    fn rect(s: f64, n: f64, w: f64, e: f64) -> GeoPolygon {
        GeoPolygon::new(vec![vec![[s, w], [s, e], [n, e], [n, w]]])
    }

    #[test]
    fn a_cell_counts_its_share_of_the_union_and_holes_are_land() {
        // Ocean west of 10.000625° (a step), over a band of latitudes.
        let per = 1600.0;
        let region = Region {
            top: 16_010,
            left: 15_998,
            rows: 4,
            columns: 6,
        };
        let sea = rect(9.0, 11.0, 9.0, 10.0 + 1.0 / per);
        let c = ocean_cover(&region, &List(vec![Arc::new(sea.clone())])).unwrap();
        // Columns 15,998 to 16,003: the edge at step 16,001, the middle of column 16,001's cell.
        assert_eq!(&c.values[..6], &[16, 16, 16, 8, 0, 0]);
        // Twice is once (two tiles' pieces overlapping).
        let twice =
            ocean_cover(&region, &List(vec![Arc::new(sea.clone()), Arc::new(sea)])).unwrap();
        assert_eq!(twice, c);
        // A hole is land.
        let holed = GeoPolygon::new(vec![
            vec![[9.0, 9.0], [9.0, 11.0], [11.0, 11.0], [11.0, 9.0]],
            vec![[9.5, 9.999], [9.5, 10.0015], [10.5, 10.0015], [10.5, 9.999]],
        ]);
        let h = ocean_cover(&region, &List(vec![Arc::new(holed)])).unwrap();
        assert_eq!(h.at(0, 0), 16);
        assert_eq!(h.at(0, 2), 0);
    }

    #[test]
    fn a_polygon_across_the_date_line_covers_the_same_places_from_either_side() {
        // A polygon from 179.9° to 180.2° (its own frame), read by the squares either side of ±180°.
        let p = GeoPolygon::new(vec![vec![
            [0.1, 179.9],
            [0.1, 180.2],
            [0.4, 180.13],
            [0.35, 179.95],
        ]]);
        let water = List(vec![Arc::new(p)]);
        let west = Region::window(SquareKey { lat: 0, lon: 179 }).grow(0, 400);
        let east = Region::window(SquareKey { lat: 0, lon: -180 }).grow(0, 400);
        let a = ocean_cover(&west, &water).unwrap();
        let b = ocean_cover(&east, &water).unwrap();
        // Region `west` runs 178.75° to 180.25°; `east` −180.25° to −178.75°: they share 179.75° to 180.25°.
        let mut covered = 0;
        for r in 0..west.rows {
            for c in 0..west.columns {
                let j = west.j(c);
                if j < 179 * 1600 + 1200 {
                    continue;
                }
                let k = (j - 360 * 1600 - east.left) as usize;
                assert_eq!(a.at(r, c), b.at(r, k), "row {r}, step {j}");
                covered += usize::from(a.at(r, c) > 0);
            }
        }
        assert!(covered > 10_000, "{covered}");
    }
}
