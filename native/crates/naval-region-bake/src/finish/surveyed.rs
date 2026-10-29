//! The surveyed layer, read **by owner** (§3.1): the surveyed height at a place is the interpolation of the one surveyed
//! square that owns it (half-open, `[lat, lat + 1) × [lon, lon + 1)`), never a neighbour's margin, so a band change
//! never gives one place two values. The surveyed squares are the world cache's (`square`): each sample supersampled
//! from the DEM and despiked with its halo on its band's grid (the despike: local, two 9 × 9 passes,
//! [`crate::square::BAKE_HALO`] samples). Their rows are the final lattice's rows, so a lattice point reads its owner's row and interpolates
//! linearly between two of its columns.
use std::sync::Arc;

use naval_land_format::SquareKey;
use naval_land_format::lattice::PER_DEGREE;

use super::lattice::{Plane, Region, polar_row, wrap_j};
use crate::BakeError;

/// The surveyed height of a polar place: the shelf's floor.
pub const POLAR_SEA_M: f32 = -(crate::SHELF_DEPTH_M as f32);
use crate::square::{MARGIN, ROWS_PER_DEGREE, Square, columns_per_degree};

/// Where the bake reads surveyed squares: the world cache's, or a generated layer's.
pub trait Surveyed {
    /// Square `key`'s surveyed heights (its `height` layer on its band's grid, with [`MARGIN`]).
    fn square(&self, key: SquareKey) -> Result<Arc<Square>, BakeError>;
}

/// The surveyed height at lattice point (`i`, wrapped `j`) in its owner `square`: the owner's row, linear between its
/// two columns about the place.
#[inline]
fn read(square: &Square, i: i32, j: i32) -> f64 {
    let key = square.grid.key;
    let across = columns_per_degree(key.lat) as i64;
    let r = ((key.lat + 1) * PER_DEGREE - i) as usize + MARGIN;
    // The column, exactly: (j − west) · across / 1,600, its whole part and its remainder.
    let n = i64::from(j - key.lon * PER_DEGREE) * across;
    let (whole, rest) = (
        n.div_euclid(i64::from(PER_DEGREE)),
        n.rem_euclid(i64::from(PER_DEGREE)),
    );
    let c = whole as usize + MARGIN;
    let columns = square.grid.columns;
    let values = square.heights();
    let v0 = f64::from(values[r * columns + c]);
    if rest == 0 {
        return v0;
    }
    let v1 = f64::from(values[r * columns + c + 1]);
    let t = rest as f64 / f64::from(PER_DEGREE);
    v0 + (v1 - v0) * t
}

/// The owner of lattice point (`i`, wrapped `j`).
#[inline]
pub fn owner(i: i32, j: i32) -> SquareKey {
    SquareKey {
        lat: i.div_euclid(PER_DEGREE),
        lon: j.div_euclid(PER_DEGREE),
    }
}

/// The surveyed height at a lattice point, read by owner (for tests and tools; the bake reads a region at once).
pub fn height_at(source: &dyn Surveyed, i: i32, j: i32) -> Result<f64, BakeError> {
    let j = wrap_j(j);
    let square = source.square(owner(i, j))?;
    Ok(read(&square, i, j))
}

/// The surveyed heights over `region`, each read by owner; a polar place ([`polar_row`]) the open sea's floor,
/// [`POLAR_SEA_M`], its square never read.
pub fn heights(region: &Region, source: &dyn Surveyed) -> Result<Plane<f32>, BakeError> {
    let mut out = Plane::filled(*region, 0f32);
    let mut held: Option<Arc<Square>> = None;
    for r in 0..region.rows {
        let i = region.i(r);
        if polar_row(i) {
            // Open sea: a polar square's surveyed heights are never read.
            for c in 0..region.columns {
                out.set(r, c, POLAR_SEA_M);
            }
            continue;
        }
        for c in 0..region.columns {
            let j = wrap_j(region.j(c));
            let key = owner(i, j);
            if held.as_ref().is_none_or(|s| s.grid.key != key) {
                let square = source.square(key)?;
                check(&square, key)?;
                held = Some(square);
            }
            let square = held.as_ref().expect("held above");
            out.set(r, c, read(square, i, j) as f32);
        }
    }
    Ok(out)
}

fn check(square: &Square, key: SquareKey) -> Result<(), BakeError> {
    let g = &square.grid;
    let ok = g.key == key
        && g.margin == MARGIN
        && g.rows == ROWS_PER_DEGREE + 1 + 2 * MARGIN
        && g.columns == columns_per_degree(key.lat) + 1 + 2 * MARGIN
        && square.heights().len() == g.rows * g.columns;
    if ok {
        Ok(())
    } else {
        Err(BakeError::Corrupt(format!(
            "the surveyed square handed for {key} is not {key}'s grid"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::square::{Layer, SquareGrid, SquareSource};
    use std::collections::BTreeMap;

    fn square(key: SquareKey, f: impl Fn(f64, f64) -> f32) -> Arc<Square> {
        let grid = SquareGrid::new(key, MARGIN);
        let mut values = vec![0f32; grid.rows * grid.columns];
        for r in 0..grid.rows {
            for c in 0..grid.columns {
                values[r * grid.columns + c] = f(grid.lat(r), grid.lon(c));
            }
        }
        let mut layers = BTreeMap::new();
        layers.insert(
            crate::square::HEIGHT.to_string(),
            Layer::Height {
                quantum: 1.0 / 256.0,
                values,
            },
        );
        Arc::new(Square {
            grid,
            source: SquareSource {
                dem: crate::Dem::Terrarium,
                zoom: None,
                fallback: false,
                attribution: "test".into(),
            },
            land: true,
            layers,
            extra: Default::default(),
        })
    }

    struct Two(Arc<Square>, Arc<Square>);
    impl Surveyed for Two {
        fn square(&self, key: SquareKey) -> Result<Arc<Square>, BakeError> {
            [&self.0, &self.1]
                .into_iter()
                .find(|s| s.grid.key == key)
                .cloned()
                .ok_or_else(|| BakeError::Missing(key.name()))
        }
    }

    #[test]
    fn a_place_reads_its_owner_across_a_band_change() {
        // 29° N has 1,600 columns a degree, 30° N 1,400: each square's own heights differ (its index), so a read from
        // the wrong one shows.
        let (south, north) = (
            SquareKey { lat: 29, lon: 10 },
            SquareKey { lat: 30, lon: 10 },
        );
        let source = Two(
            square(south, |_, lon| lon as f32 * 2.0),
            square(north, |_, lon| 1000.0 + lon as f32),
        );
        // On the shared edge (30° N exactly) the owner is the north square.
        let i = 30 * PER_DEGREE;
        for j in [10 * PER_DEGREE, 10 * PER_DEGREE + 7, 10 * PER_DEGREE + 1599] {
            let h = height_at(&source, i, j).unwrap();
            let lon = f64::from(j) / 1600.0;
            assert!((h - (1000.0 + lon)).abs() < 1e-3, "{j}: {h}");
        }
        // One row south it is the south square's.
        let h = height_at(&source, i - 1, 10 * PER_DEGREE + 7).unwrap();
        assert!((h - 2.0 * (10.0 + 7.0 / 1600.0)).abs() < 1e-3, "{h}");
        assert_eq!(owner(i, 16_000), north);
        assert_eq!(owner(i - 1, 16_000), south);
        assert_eq!(owner(0, -1), SquareKey { lat: 0, lon: -1 });
        assert_eq!(owner(-1, 0), SquareKey { lat: -1, lon: 0 });
    }

    #[test]
    fn a_column_between_samples_is_linear_in_its_owners_row() {
        let key = SquareKey { lat: 50, lon: -3 };
        let s = square(key, |lat, lon| (lat * 100.0 + lon * 10.0) as f32);
        let across = columns_per_degree(50) as f64;
        let j = -3 * PER_DEGREE + 333;
        let i = 50 * PER_DEGREE + 800;
        let got = read(&s, i, j);
        let lon = -3.0 + 333.0 / 1600.0;
        let c = (lon + 3.0) * across;
        let (c0, t) = (c.floor(), c - c.floor());
        let lon0 = -3.0 + c0 / across;
        let lon1 = -3.0 + (c0 + 1.0) / across;
        let lat = 50.5;
        let v0 = (lat * 100.0 + lon0 * 10.0) as f32 as f64;
        let v1 = (lat * 100.0 + lon1 * 10.0) as f32 as f64;
        assert!((got - (v0 + (v1 - v0) * t)).abs() < 1e-3, "{got}");
    }
}
