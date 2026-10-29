//! Web-mercator tiles (the XYZ scheme: x east from the antimeridian, y south from 85.05° N) and a tile's own units to
//! latitude and longitude.

use std::f64::consts::PI;

/// The deepest zoom OpenMapTiles' vector tiles go to (OpenFreeMap's planet stops there; deeper zooms overzoom it).
pub const MAX_ZOOM: u8 = 14;
/// The web-mercator sphere's radius, metres.
pub const MERCATOR_RADIUS_M: f64 = 6_378_137.0;
/// Web mercator's latitude limit, degrees.
pub const MAX_LAT: f64 = 85.051_128_779_806_59;

/// A tile: zoom, column and row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    pub z: u8,
    pub x: u32,
    pub y: u32,
}

impl TileId {
    pub fn new(z: u8, x: u32, y: u32) -> Self {
        Self { z, x, y }
    }

    /// Tiles a side at this zoom.
    pub fn side(z: u8) -> u32 {
        1 << z
    }

    /// The tile at zoom `z` holding `lat`, `lon` (longitude wrapped, latitude clamped to mercator's limit).
    pub fn of(lat: f64, lon: f64, z: u8) -> Self {
        let [fx, fy] = fraction(lat, lon);
        let n = f64::from(Self::side(z));
        let max = Self::side(z) - 1;
        Self {
            z,
            x: ((fx * n).floor() as i64).clamp(0, i64::from(max)) as u32,
            y: ((fy * n).floor() as i64).clamp(0, i64::from(max)) as u32,
        }
    }

    /// The latitude and longitude of the point `(u, v)` of the tile in units of `extent` across it (0 at its north-west
    /// corner, `extent` at the next tile's; a buffer's points lie past either end).
    pub fn lat_lon(&self, u: f64, v: f64, extent: f64) -> (f64, f64) {
        let n = f64::from(Self::side(self.z));
        let fx = (f64::from(self.x) + u / extent) / n;
        let fy = (f64::from(self.y) + v / extent) / n;
        let lon = fx * 360.0 - 180.0;
        let lat = (PI * (1.0 - 2.0 * fy)).sinh().atan().to_degrees();
        (lat, lon)
    }

    /// The tile's corners: (south, west) and (north, east), degrees.
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        let (north, west) = self.lat_lon(0.0, 0.0, 1.0);
        let (south, east) = self.lat_lon(1.0, 1.0, 1.0);
        ([south, west], [north, east])
    }

    /// Metres across the tile on the ground at its middle's latitude.
    pub fn ground_side_m(&self) -> f64 {
        let (lat, _) = self.lat_lon(0.5, 0.5, 1.0);
        2.0 * PI * MERCATOR_RADIUS_M * lat.to_radians().cos() / f64::from(Self::side(self.z))
    }

    /// The tile at zoom `z` (no deeper than this one's) that holds this one.
    pub fn ancestor(&self, z: u8) -> Self {
        let shift = self.z.saturating_sub(z);
        Self {
            z: self.z - shift,
            x: self.x >> shift,
            y: self.y >> shift,
        }
    }
}

/// Mercator's fractions of the world, x east from the antimeridian and y south from the top, 0-1.
pub fn fraction(lat: f64, lon: f64) -> [f64; 2] {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    let lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
    let fx = (lon + 180.0) / 360.0;
    let fy = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0;
    [fx, fy]
}

/// The tiles at zoom `z` covering the box from (south, west) to (north, east), degrees, row by row. A box across the
/// antimeridian (west east of east) takes the tiles on both sides.
pub fn covering(south_west: [f64; 2], north_east: [f64; 2], z: u8) -> Vec<TileId> {
    let a = TileId::of(north_east[0], south_west[1], z);
    let b = TileId::of(south_west[0], north_east[1], z);
    let columns: Vec<u32> = if a.x <= b.x {
        (a.x..=b.x).collect()
    } else {
        (a.x..TileId::side(z)).chain(0..=b.x).collect()
    };
    (a.y.min(b.y)..=a.y.max(b.y))
        .flat_map(|y| columns.iter().map(move |&x| TileId::new(z, x, y)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_are_named_as_the_servers_name_them() {
        // Santa Cruz, California at zoom 14.
        assert_eq!(TileId::of(37.0, -122.03, 14), TileId::new(14, 2638, 6377));
        assert_eq!(TileId::of(0.0, 0.0, 1), TileId::new(1, 1, 1));
        assert_eq!(TileId::of(-33.86, 151.21, 10), TileId::new(10, 942, 614));
        // The poles clamp, the antimeridian wraps.
        assert_eq!(TileId::of(89.9, 180.0, 3), TileId::new(3, 0, 0));
        assert_eq!(TileId::of(-89.9, 179.999, 3), TileId::new(3, 7, 7));
    }

    #[test]
    fn a_tiles_units_map_back_to_its_corners() {
        let t = TileId::of(36.936204, -122.418494, 14);
        let ([s, w], [n, e]) = t.bounds();
        assert!(s < 36.936204 && 36.936204 < n && w < -122.418494 && -122.418494 < e);
        // Every point of the tile comes back to the tile; the next tile's corner is this one's far corner.
        for (u, v) in [(0.0, 0.0), (4095.0, 17.0), (2048.0, 4095.9)] {
            let (lat, lon) = t.lat_lon(u, v, 4096.0);
            assert_eq!(TileId::of(lat, lon, 14), t, "{u} {v}");
        }
        let next = TileId::new(14, t.x + 1, t.y + 1);
        let (lat, lon) = next.lat_lon(0.0, 0.0, 4096.0);
        let (lat2, lon2) = t.lat_lon(4096.0, 4096.0, 4096.0);
        assert!((lat - lat2).abs() < 1e-12 && (lon - lon2).abs() < 1e-12);
        // About 1.95 km a side at 37 degrees.
        assert!(
            (t.ground_side_m() - 1955.0).abs() < 10.0,
            "{}",
            t.ground_side_m()
        );
        assert_eq!(t.ancestor(12), TileId::new(12, t.x / 4, t.y / 4));
        assert_eq!(t.ancestor(16), t);
    }

    #[test]
    fn a_box_is_covered_whole() {
        let tiles = covering([36.9, -122.5], [37.0, -122.4], 14);
        let a = TileId::of(37.0, -122.5, 14);
        let b = TileId::of(36.9, -122.4, 14);
        assert_eq!(
            tiles.len() as u32,
            (b.x - a.x + 1) * (b.y - a.y + 1),
            "{tiles:?}"
        );
        assert!(tiles.contains(&TileId::of(36.95, -122.45, 14)));
        // Across the antimeridian: both edges' columns.
        let across = covering([-10.0, 179.5], [-9.0, -179.5], 8);
        assert!(across.iter().any(|t| t.x == 255) && across.iter().any(|t| t.x == 0));
    }
}
