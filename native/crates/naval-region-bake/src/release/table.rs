//! A release's square table from its snapshot (C1, "Which squares hold land"): every square `polar` past the pole
//! limit, else `candidate` when **land lies within 65 km of it** ([`CANDIDATE_M`], the bake's consideration margin),
//! else `ocean` (the water polygons cover it and its margin: open sea, authoritatively, never baked). Computed once,
//! when the release is made, in one pass over the whole snapshot ([`TableBuilder`]); the only place a release reads the
//! snapshot in bulk.
//!
//! - **Land is what the water polygons do not cover.** Its edge is the coastline: every polygon edge except those the
//!   split made ([`artificial`]: on a tile's grown edge, or on ±180° or ±90°), which lie inside the sea, the
//!   neighbouring tile's piece reaching past them.
//! - **Within 65 km**: a point of the coastline within [`CANDIDATE_M`] of the square ([`distance_m`]), each edge of the
//!   coastline sampled every [`SAMPLE_M`] or less, its ends included. Or no coastline at all within the margin and the
//!   square's middle uncovered: the square and its margin are then land throughout (a square inland). With no
//!   coastline within the margin the square and its margin are one thing, sea or land, so the middle decides.
//! - **Conservative for the bake.** An `ocean` square has no land within 65 km, to within half a sample, so finishing
//!   (which makes land only within a component's cap of the coastline's) gives it none: the keep rule may read it as
//!   having no final land. A `candidate` bakes, and its own rule writes `empty` or data.
//!
//! The distance is in the bake's metres: a degree of latitude [`M_PER_DEGREE`], a degree of longitude that times the
//! cosine of the place's latitude clamped to the square's (the lattice's own `to_square_m`, at any place), the cosine
//! in `+ − × ÷` alone ([`sin_cos`]), so every machine makes the same table from the same snapshot.
use naval_land_format::{SquareKey, SquareTable, table::polar};

use super::snapshot::WaterRecord;
use crate::finish::keep::CANDIDATE_M;
use crate::finish::lattice::{ROW_M, sin_cos};

/// Metres a degree of latitude: the lattice's.
pub const M_PER_DEGREE: f64 = ROW_M * 1600.0;
/// The coastline is sampled at least this often.
pub const SAMPLE_M: f64 = 100.0;
/// A split edge's grown tile edge: 0.0005° of latitude, and 0.0005°/cos φ of longitude at the tile's middle latitude.
pub const OVERLAP_DEG: f64 = 0.0005;
/// How near a coordinate must lie to a split line to be on it (osmdata's coordinates are 1e-7° apart).
const ON_LINE: f64 = 1e-9;

/// The distance from (lat, lon) to square `key` in the bake's metres: 0 inside or on its edge; the longitude taken
/// within half a world of the square.
pub fn distance_m(key: SquareKey, lat: f64, lon: f64) -> f64 {
    let (s, w) = (f64::from(key.lat), f64::from(key.lon));
    let lon = lon + ((w + 0.5 - lon) / 360.0).round() * 360.0;
    let dlat = (s - lat).max(lat - (s + 1.0)).max(0.0);
    let dlon = (w - lon).max(lon - (w + 1.0)).max(0.0);
    let ns = dlat * M_PER_DEGREE;
    let ew = if dlon > 0.0 {
        let clamped = lat.clamp(s, s + 1.0);
        dlon * M_PER_DEGREE * sin_cos(clamped * std::f64::consts::PI / 180.0).1
    } else {
        0.0
    };
    (ns * ns + ew * ew).sqrt()
}

fn on(v: f64, line: f64) -> bool {
    (v - line).abs() <= ON_LINE
}

/// Whether edge `a`–`b` ([lat, lon]) is one the split made, not coastline: on a tile's grown edge (latitude k ± 0.0005°,
/// longitude m ± 0.0005°/cos φ with φ the middle latitude of the tile the edge lies in), on ±180° (where the snapshot
/// cuts the world) or on ±90°.
pub fn artificial(a: [f64; 2], b: [f64; 2]) -> bool {
    if a[0] == b[0] {
        let lat = a[0];
        return on(lat.abs(), 90.0)
            || [OVERLAP_DEG, -OVERLAP_DEG]
                .iter()
                .any(|o| on(lat, (lat - o).round() + o));
    }
    if a[1] == b[1] {
        let lon = a[1];
        if on(lon.abs(), 180.0) {
            return true;
        }
        let (lo, hi) = (a[0].min(b[0]), a[0].max(b[0]));
        let middle = 0.5 * (lo + hi);
        // The tiles whose grown rows hold the edge.
        for k in [middle.floor() - 1.0, middle.floor(), middle.floor() + 1.0] {
            if lo < k - OVERLAP_DEG - ON_LINE || hi > k + 1.0 + OVERLAP_DEG + ON_LINE {
                continue;
            }
            let grown = OVERLAP_DEG / ((k + 0.5) * std::f64::consts::PI / 180.0).cos();
            if [grown, -grown]
                .iter()
                .any(|o| on(lon, (lon - o).round() + o))
            {
                return true;
            }
        }
    }
    false
}

/// Whether (lat, lon) lies inside the polygon's rings (even–odd).
fn inside(rings: &[Vec<[f64; 2]>], lat: f64, lon: f64) -> bool {
    let mut odd = false;
    for ring in rings {
        let n = ring.len();
        for e in 0..n {
            let (a, b) = (ring[e], ring[(e + 1) % n]);
            if (a[0] > lat) != (b[0] > lat) {
                let x = a[1] + (lat - a[0]) / (b[0] - a[0]) * (b[1] - a[1]);
                if x > lon {
                    odd = !odd;
                }
            }
        }
    }
    odd
}

/// The table, made one polygon at a time.
pub struct TableBuilder {
    /// Squares with coastline within [`CANDIDATE_M`].
    coast: Vec<bool>,
    /// Squares whose middle the water covers.
    covered: Vec<bool>,
    pub records: usize,
    pub edges: u64,
    pub coast_edges: u64,
}

impl Default for TableBuilder {
    fn default() -> Self {
        Self {
            coast: vec![false; SquareKey::COUNT],
            covered: vec![false; SquareKey::COUNT],
            records: 0,
            edges: 0,
            coast_edges: 0,
        }
    }
}

impl TableBuilder {
    pub fn add(&mut self, record: &WaterRecord) {
        self.records += 1;
        let [south, north, west, east] = record.bounds;
        // The middles the polygon's box holds.
        let lat0 = (south - 0.5).ceil() as i32;
        let lat1 = (north - 0.5).floor() as i32;
        let lon0 = (west - 0.5).ceil() as i32;
        let lon1 = (east - 0.5).floor() as i32;
        for lat in lat0.max(-90)..=lat1.min(89) {
            for lon in lon0..=lon1 {
                let key = SquareKey::at(f64::from(lat) + 0.5, f64::from(lon) + 0.5);
                let k = key.index();
                if !self.covered[k]
                    && inside(&record.rings, f64::from(lat) + 0.5, f64::from(lon) + 0.5)
                {
                    self.covered[k] = true;
                }
            }
        }
        for ring in &record.rings {
            let n = ring.len();
            for e in 0..n {
                let (a, b) = (ring[e], ring[(e + 1) % n]);
                self.edges += 1;
                if artificial(a, b) {
                    continue;
                }
                self.coast_edges += 1;
                self.coastline(a, b);
            }
        }
    }

    /// Marks the squares within [`CANDIDATE_M`] of a sample of edge `a`–`b`.
    fn coastline(&mut self, a: [f64; 2], b: [f64; 2]) {
        let c = sin_cos(0.5 * (a[0] + b[0]) * std::f64::consts::PI / 180.0).1;
        let length = M_PER_DEGREE * ((b[0] - a[0]).powi(2) + ((b[1] - a[1]) * c).powi(2)).sqrt();
        let steps = (length / SAMPLE_M).ceil().max(1.0) as usize;
        for k in 0..=steps {
            let t = k as f64 / steps as f64;
            self.near(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
        }
    }

    /// Marks the squares within [`CANDIDATE_M`] of (lat, lon).
    fn near(&mut self, lat: f64, lon: f64) {
        let reach = CANDIDATE_M / M_PER_DEGREE;
        let rows = ((lat - reach).floor() as i32).max(-90)..=((lat + reach).floor() as i32).min(89);
        for row in rows {
            let first = SquareKey { lat: row, lon: 0 };
            if polar(first) {
                continue;
            }
            let (s, n) = (f64::from(row), f64::from(row) + 1.0);
            let ns = (s - lat).max(lat - n).max(0.0) * M_PER_DEGREE;
            if ns > CANDIDATE_M {
                continue;
            }
            let cos = sin_cos(lat.clamp(s, n) * std::f64::consts::PI / 180.0).1;
            let across =
                ((CANDIDATE_M * CANDIDATE_M - ns * ns).sqrt() / (M_PER_DEGREE * cos)).min(180.0);
            let (w, e) = ((lon - across).floor() as i32, (lon + across).floor() as i32);
            for m in w..=e.min(w + 359) {
                let key = SquareKey {
                    lat: row,
                    lon: (m + 180).rem_euclid(360) - 180,
                };
                let k = key.index();
                if !self.coast[k] && distance_m(key, lat, lon) <= CANDIDATE_M {
                    self.coast[k] = true;
                }
            }
        }
    }

    /// The squares with coastline within the margin, so far.
    pub fn coast_near(&self, key: SquareKey) -> bool {
        self.coast[key.index()]
    }

    /// Whether the water covers the square's middle.
    pub fn middle_covered(&self, key: SquareKey) -> bool {
        self.covered[key.index()]
    }

    pub fn finish(&self) -> SquareTable {
        SquareTable::new(|key| self.coast[key.index()] || !self.covered[key.index()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_land_format::Class;

    fn record(rings: Vec<Vec<[f64; 2]>>) -> WaterRecord {
        let mut b = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
        for p in rings.iter().flatten() {
            b = [
                b[0].min(p[0]),
                b[1].max(p[0]),
                b[2].min(p[1]),
                b[3].max(p[1]),
            ];
        }
        WaterRecord {
            index: 0,
            rings,
            bounds: b,
        }
    }

    /// Tile (lat, lon)'s piece of sea, grown as osmdata grows it, with `holes`.
    fn tile(lat: i32, lon: i32, holes: Vec<Vec<[f64; 2]>>) -> WaterRecord {
        let (s, w) = (f64::from(lat), f64::from(lon));
        let gx = OVERLAP_DEG / ((s + 0.5) * std::f64::consts::PI / 180.0).cos();
        let (s0, n0) = (
            (s - OVERLAP_DEG).max(-90.0),
            (s + 1.0 + OVERLAP_DEG).min(90.0),
        );
        let (w0, e0) = ((w - gx).max(-180.0), (w + 1.0 + gx).min(180.0));
        let mut rings = vec![vec![[s0, w0], [n0, w0], [n0, e0], [s0, e0]]];
        rings.extend(holes);
        record(rings)
    }

    /// An open ocean of tiles over latitudes `s..n` and longitudes `w..e`, with an island (a small square hole of side
    /// `side` degrees) at (lat, lon) in the tile holding it.
    fn ocean_with_island(
        s: i32,
        n: i32,
        w: i32,
        e: i32,
        island: [f64; 2],
        side: f64,
    ) -> TableBuilder {
        let mut t = TableBuilder::default();
        for lat in s..n {
            for lon in w..e {
                let hole =
                    side > 0.0 && SquareKey::at(island[0], island[1]) == SquareKey { lat, lon };
                let holes = if hole {
                    let [y, x] = island;
                    vec![vec![
                        [y, x],
                        [y + side, x],
                        [y + side, x + side],
                        [y, x + side],
                    ]]
                } else {
                    vec![]
                };
                t.add(&tile(lat, lon, holes));
            }
        }
        t
    }

    #[test]
    fn split_edges_are_not_coastline_and_coast_is() {
        // A tile's grown edges, at 52.5° (0.000821°) and at the date line.
        let t = tile(-53, -75, vec![]);
        let ring = &t.rings[0];
        for e in 0..4 {
            assert!(artificial(ring[e], ring[(e + 1) % 4]), "{:?}", ring[e]);
        }
        assert!(artificial([10.2, 180.0], [10.4, 180.0]));
        assert!(artificial([10.2, -180.0], [10.4, -180.0]));
        assert!(artificial([90.0, 3.0], [90.0, 4.0]));
        // Coastline: slanted, and straight but off the lines.
        assert!(!artificial([10.2, 20.3], [10.25, 20.31]));
        assert!(!artificial([10.2, 20.3], [10.2, 20.31]));
        assert!(!artificial([10.2, 20.0], [10.4, 20.0]));
        assert!(!artificial([10.0006, 20.3], [10.0006, 20.4]));
        // A grown longitude belongs to the tile's own latitude: 52.5°'s is not 10.5°'s.
        let gx = OVERLAP_DEG / (52.5f64.to_radians()).cos();
        assert!(!artificial([10.2, 20.0 + gx], [10.4, 20.0 + gx]));
        assert!(artificial([52.2, 20.0 + gx], [52.4, 20.0 + gx]));
    }

    #[test]
    fn an_open_ocean_is_ocean_inland_is_candidate_and_poles_are_polar() {
        let t = ocean_with_island(10, 14, 30, 34, [12.5, 32.5], 0.0);
        let table = t.finish();
        assert_eq!(t.coast_edges, 0, "the split's edges only");
        assert_eq!(table.class(SquareKey { lat: 11, lon: 31 }), Class::Ocean);
        assert_eq!(table.class(SquareKey { lat: 12, lon: 32 }), Class::Ocean);
        // Where no polygon lies is land: a square inland is a candidate with no coastline near it.
        assert_eq!(
            table.class(SquareKey { lat: 20, lon: 50 }),
            Class::Candidate
        );
        assert_eq!(table.class(SquareKey { lat: 89, lon: 31 }), Class::Polar);
    }

    #[test]
    fn the_65_km_candidate_margin_each_side() {
        // An island 100 m across at 12.5° N 32.5° E in an ocean six degrees wide: squares round it at about 65 km.
        let island = [12.5, 32.5];
        let t = ocean_with_island(9, 16, 29, 36, island, 0.0009);
        let table = t.finish();
        // Every square's class is whether the island's nearest corner lies within 65 km.
        let mut near = (f64::MAX, f64::MIN);
        let mut far = (f64::MAX, f64::MIN);
        for lat in 10..15 {
            for lon in 30..35 {
                let key = SquareKey { lat, lon };
                let d = [[0.0, 0.0], [0.0009, 0.0], [0.0, 0.0009], [0.0009, 0.0009]]
                    .iter()
                    .map(|o| distance_m(key, island[0] + o[0], island[1] + o[1]))
                    .fold(f64::MAX, f64::min);
                let class = table.class(key);
                assert_eq!(class == Class::Candidate, d <= CANDIDATE_M, "{key}: {d} m");
                if d <= CANDIDATE_M {
                    near = (near.0.min(d), near.1.max(d));
                } else {
                    far = (far.0.min(d), far.1.max(d));
                }
            }
        }
        // Squares on both sides of the margin, the nearest within a few kilometres of it.
        assert!(near.1 > 50_000.0 && far.0 < 80_000.0, "{near:?} {far:?}");
        // The threshold itself: a square whose distance is 65 km less and more than a sample.
        let key = SquareKey { lat: 12, lon: 33 };
        let mut inside = TableBuilder::default();
        let mut outside = TableBuilder::default();
        let at = |metres: f64| {
            [
                12.5,
                34.0 + metres / (M_PER_DEGREE * (12.5f64.to_radians()).cos()),
            ]
        };
        inside.near(at(CANDIDATE_M - 30.0)[0], at(CANDIDATE_M - 30.0)[1]);
        outside.near(at(CANDIDATE_M + 30.0)[0], at(CANDIDATE_M + 30.0)[1]);
        assert!(inside.coast_near(key) && !outside.coast_near(key));
    }

    #[test]
    fn coastline_across_the_date_line_reaches_squares_either_side() {
        let mut t = TableBuilder::default();
        // A coast edge just west of 180° reaches the square east of it.
        t.coastline([-16.5, 179.95], [-16.4, 179.97]);
        assert!(t.coast_near(SquareKey {
            lat: -17,
            lon: -180
        }));
        assert!(t.coast_near(SquareKey { lat: -17, lon: 179 }));
        assert!(!t.coast_near(SquareKey {
            lat: -17,
            lon: -179
        }));
        assert!(
            distance_m(
                SquareKey {
                    lat: -17,
                    lon: -180
                },
                -16.5,
                179.95
            ) < 6_000.0
        );
    }
}
