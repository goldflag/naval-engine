//! Which candidate squares hold data (C1): a square's manifest line is its bytes when **final land** lies within
//! [`KEEP_M`] of it (the region bake's keep distance, [`crate::LAND_REACH_M`]), else `empty`. Final land is the
//! finished quanta above the sea, in the square or in a neighbour's window; a square the table calls `ocean` has none
//! (it has no coastline land within [`CANDIDATE_M`], and finishing makes land only within a component's cap of the
//! coastline's).
//!
//! Distances are on the lattice ([`to_square_m`]): north-south [`ROW_M`] a row, east-west the column of the square's
//! row nearest the point. The same arithmetic as the bake's, so a threshold case is the same on every machine.
use naval_land_format::SquareKey;
use naval_land_format::block::Window;
use naval_land_format::lattice::{AROUND, PER_DEGREE, WINDOW};

use super::coverage::{HALF, Water, ocean_cover};
use super::lattice::{ROW_M, Region, column_m, wrap_j};
use crate::BakeError;

/// Final land within this of a square makes its line data.
pub const KEEP_M: f64 = crate::LAND_REACH_M;
/// Coastline land within this of a square makes it a candidate.
pub const CANDIDATE_M: f64 = crate::CANDIDATE_REACH_M;

/// The distance from lattice point (`i`, `j`) (any longitude step: taken within half a world of the square) to square
/// `key`'s rectangle, edges included, metres: 0 inside.
pub fn to_square_m(key: SquareKey, i: i32, j: i32) -> f64 {
    let (i0, j0) = (key.lat * PER_DEGREE, key.lon * PER_DEGREE);
    let (i1, j1) = (i0 + PER_DEGREE, j0 + PER_DEGREE);
    let middle = j0 + PER_DEGREE / 2;
    let j = middle + wrap_j(j - middle);
    let di = (i0 - i).max(i - i1).max(0);
    let dj = (j0 - j).max(j - j1).max(0);
    let ns = f64::from(di) * ROW_M;
    let ew = f64::from(dj) * column_m(i.clamp(i0, i1));
    (ns * ns + ew * ew).sqrt()
}

/// The squares other than `key` (never polar) any of whose points lie within `reach_m` of it, in key order.
pub fn squares_within(key: SquareKey, reach_m: f64) -> Vec<SquareKey> {
    let mut out = vec![];
    for lat in key.lat - 1..=key.lat + 1 {
        if !(-90..=89).contains(&lat) {
            continue;
        }
        // East and west until a square's nearest edge lies past the reach.
        let mut add = |n: SquareKey| {
            if n != key
                && !naval_land_format::table::polar(n)
                && !out.contains(&n)
                && rectangle_within(key, n, reach_m)
            {
                out.push(n);
                true
            } else {
                n == key
            }
        };
        let row = SquareKey { lat, lon: key.lon };
        add(row);
        let mut any = true;
        let mut k = 1;
        while any && k < 180 {
            any = false;
            for lon in [key.lon + k, key.lon - k] {
                let n = SquareKey {
                    lat,
                    lon: (lon + 180).rem_euclid(360) - 180,
                };
                any |= add(n);
            }
            k += 1;
        }
    }
    out.sort();
    out
}

/// Whether any point of square `n` lies within `reach_m` of square `key`: the least is on `n`'s edges.
fn rectangle_within(key: SquareKey, n: SquareKey, reach_m: f64) -> bool {
    let w = Region::window(n);
    (0..WINDOW).any(|k| {
        [(0, k), (WINDOW - 1, k), (k, 0), (k, WINDOW - 1)]
            .into_iter()
            .any(|(r, c)| to_square_m(key, w.i(r), w.j(c)) <= reach_m)
    })
}

/// Whether square `of`'s final window holds land within `reach_m` of square `key`.
pub fn land_within(key: SquareKey, of: SquareKey, window: &Window, reach_m: f64) -> bool {
    let w = Region::window(of);
    (0..WINDOW).any(|r| {
        (0..WINDOW).any(|c| window.at(r, c) > 0 && to_square_m(key, w.i(r), w.j(c)) <= reach_m)
    })
}

/// The bake's keep rule: whether square `key`, whose final window is `own`, holds data. `neighbour` gives a nearby
/// square's final window, or none where the table calls it ocean (or polar): no final land.
pub fn keep(
    key: SquareKey,
    own: &Window,
    neighbour: &mut dyn FnMut(SquareKey) -> Result<Option<Window>, BakeError>,
) -> Result<bool, BakeError> {
    if own.land() {
        return Ok(true);
    }
    for n in squares_within(key, KEEP_M) {
        if let Some(window) = neighbour(n)?
            && land_within(key, n, &window, KEEP_M)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The candidate rule over the water polygons: whether the coastline's land (a lattice point less than half sea) lies
/// within `reach_m` of square `key`. Exact on the lattice, and so the synthetic sources' square table; it rasterises
/// the square and the reach about it, which only a mid-latitude test can afford (a release's table is made from its
/// snapshot when the release is made).
pub fn coastline_land_within(
    key: SquareKey,
    water: &dyn Water,
    reach_m: f64,
) -> Result<bool, BakeError> {
    let window = Region::window(key);
    let rows = (reach_m / ROW_M).ceil() as usize + 1;
    let poleward = window.grow(rows, 0).poleward();
    let columns = (reach_m / column_m(poleward)).ceil() as usize + 1;
    assert!(
        columns < AROUND as usize / 4,
        "the reach spans too much of a parallel"
    );
    let region = window.grow(rows, columns);
    let cover = ocean_cover(&region, water)?;
    for r in 0..region.rows {
        for c in 0..region.columns {
            if cover.at(r, c) < HALF && to_square_m(key, region.i(r), region.j(c)) <= reach_m {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_squares_distance_is_zero_inside_and_wraps_the_world() {
        let key = SquareKey { lat: 10, lon: 179 };
        assert_eq!(to_square_m(key, 16_400, 179 * 1600 + 3), 0.0);
        // One step east of 180° (−180° + a step) lies a column away.
        let d = to_square_m(key, 16_400, -288_000 + 1);
        assert!((d - column_m(16_400)).abs() < 1e-9, "{d}");
        // North of the square: rows only.
        let d = to_square_m(key, 17_600 + 10, 179 * 1600 + 5);
        assert!((d - 10.0 * ROW_M).abs() < 1e-9);
        let near = squares_within(key, KEEP_M);
        assert!(near.contains(&SquareKey { lat: 10, lon: -180 }));
        assert!(near.contains(&SquareKey { lat: 11, lon: 178 }));
        assert_eq!(near.len(), 8);
        // Far north a degree of longitude is a few kilometres: 40 km reaches many squares east and west.
        let north = squares_within(SquareKey { lat: 85, lon: 0 }, KEEP_M);
        assert!(north.len() > 8, "{}", north.len());
        assert!(north.iter().all(|n| (84..=86).contains(&n.lat)));
    }
}
