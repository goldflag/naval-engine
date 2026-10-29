//! A heightfield of land: a theatre window's ground (`naval_client`'s theatre land builds one from its land release),
//! sampled as the simulation samples its own (`crates/naval-sim/src/terrain.rs`), so the drawn land is the land ships
//! ground on and shells strike. The battle maps' baked NTF1 files this once decoded were removed on 2026-10-27.

/// Height of the open sea floor beyond a chart (`OPEN_SEA_M`).
pub const OPEN_SEA_M: f64 = -60.0;

/// A heightfield (`Heightfield`): row-major quanta, x along a row, z down the rows.
#[derive(Clone, Debug, PartialEq)]
pub struct Heightfield {
    pub columns: u32,
    pub rows: u32,
    /// Metres between samples.
    pub cell: f32,
    /// Chart position of sample (0, 0), metres.
    pub origin_x: f32,
    pub origin_z: f32,
    /// Metres per quantum.
    pub step: f32,
    pub quanta: Vec<i16>,
}

impl Heightfield {
    /// The stored height of sample (i, j), metres.
    pub fn sample(&self, i: u32, j: u32) -> f64 {
        f64::from(self.quanta[(j * self.columns + i) as usize]) * f64::from(self.step)
    }

    /// Chart extent, metres: [min x, min z, max x, max z].
    pub fn bounds(&self) -> [f64; 4] {
        let (ox, oz, cell) = (
            f64::from(self.origin_x),
            f64::from(self.origin_z),
            f64::from(self.cell),
        );
        [
            ox,
            oz,
            ox + f64::from(self.columns - 1) * cell,
            oz + f64::from(self.rows - 1) * cell,
        ]
    }

    /// Bilinear height at chart metres; [`OPEN_SEA_M`] beyond the grid (`Heightfield.height`, in f64 as JavaScript).
    pub fn height(&self, x: f64, z: f64) -> f64 {
        let cell = f64::from(self.cell);
        let gx = (x - f64::from(self.origin_x)) / cell;
        let gz = (z - f64::from(self.origin_z)) / cell;
        let (w, h) = (f64::from(self.columns), f64::from(self.rows));
        if !(gx >= 0.0 && gz >= 0.0) || gx > w - 1.0 || gz > h - 1.0 {
            return OPEN_SEA_M;
        }
        let i = gx.floor().min(w - 2.0);
        let j = gz.floor().min(h - 2.0);
        let (u, v) = (gx - i, gz - j);
        let at = j as usize * self.columns as usize + i as usize;
        let q = |k: usize| f64::from(self.quanta[k]);
        let c = self.columns as usize;
        let top = q(at) * (1.0 - u) + q(at + 1) * u;
        let bottom = q(at + c) * (1.0 - u) + q(at + c + 1) * u;
        (top * (1.0 - v) + bottom * v) * f64::from(self.step)
    }

    /// Whether any land sample lies within `radius` metres of chart point (x, z) (`Heightfield.landWithin`).
    pub fn land_within(&self, x: f64, z: f64, radius: f64) -> bool {
        if !(x.is_finite() && z.is_finite() && radius.is_finite()) {
            return false;
        }
        let cell = f64::from(self.cell);
        let span = |centre: f64, origin: f64, count: u32| -> Option<(i64, i64)> {
            let low = (((centre - radius - origin) / cell).floor() as i64).max(0);
            let high =
                (((centre + radius - origin) / cell).ceil() as i64).min(i64::from(count) - 1);
            (high >= low).then_some((low, high))
        };
        let (Some((i0, i1)), Some((j0, j1))) = (
            span(x, f64::from(self.origin_x), self.columns),
            span(z, f64::from(self.origin_z), self.rows),
        ) else {
            return false;
        };
        for j in j0..=j1 {
            let dz = f64::from(self.origin_z) + j as f64 * cell - z;
            for i in i0..=i1 {
                let dx = f64::from(self.origin_x) + i as f64 * cell - x;
                if dx * dx + dz * dz <= radius * radius
                    && self.quanta[(j * i64::from(self.columns) + i) as usize] > 0
                {
                    return true;
                }
            }
        }
        false
    }
}

/// A map's heightfield placed in one battle's world frame: world = chart + offset (`PlacedTerrain`).
#[derive(Clone, Copy, Debug)]
pub struct PlacedTerrain<'a> {
    pub field: Option<&'a Heightfield>,
    pub offset: [f64; 2],
}

impl PlacedTerrain<'_> {
    /// Height at world (x, z) (`terrainHeight`).
    pub fn height(&self, x: f64, z: f64) -> f64 {
        self.field.map_or(OPEN_SEA_M, |field| {
            field.height(x - self.offset[0], z - self.offset[1])
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same 3 × 4 field as `terrain::tests` in naval-sim: 40 m cells from (−40, −80), quanta of 0.25 m.
    fn field() -> Heightfield {
        Heightfield {
            columns: 3,
            rows: 4,
            cell: 40.0,
            origin_x: -40.0,
            origin_z: -80.0,
            step: 0.25,
            quanta: vec![-8, -8, -8, -8, 400, -8, -8, 800, 1200, -8, -8, -8],
        }
    }

    #[test]
    fn samples_bilinearly_and_finds_land_like_the_simulation() {
        let field = field();
        assert_eq!(field.sample(1, 1), 100.0);
        assert_eq!(field.bounds(), [-40.0, -80.0, 40.0, 40.0]);
        assert_eq!(field.height(0.0, -40.0), 100.0);
        assert_eq!(
            field.height(20.0, -20.0),
            (100.0 * 0.5 - 2.0 * 0.5) * 0.5 + (200.0 * 0.5 + 300.0 * 0.5) * 0.5
        );
        assert_eq!(field.height(41.0, 0.0), OPEN_SEA_M);
        assert!(field.land_within(0.0, -40.0, 1.0));
        assert!(!field.land_within(-40.0, 40.0, 30.0));
        let placed = PlacedTerrain {
            field: Some(&field),
            offset: [0.0, 40.0],
        };
        assert_eq!(placed.height(0.0, 0.0), 100.0);
        assert_eq!(
            PlacedTerrain {
                field: None,
                offset: [0.0; 2]
            }
            .height(0.0, 0.0),
            OPEN_SEA_M
        );
    }
}
