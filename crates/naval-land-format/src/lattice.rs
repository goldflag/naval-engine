//! The one global final lattice (step C, §3.1): a point every 1/[`PER_DEGREE`]° of latitude and of longitude, at every
//! latitude. A square's final layer is its window of it, [`WINDOW`] × [`WINDOW`] points with both edges, row 0 the
//! north edge and column 0 the west edge. Two squares' shared edge is therefore the same lattice points, across a band
//! change of the surveyed layer, the DEM's change at 60° N and 56° S, and ±180° (180° E and 180° W are one longitude);
//! a bilinear interpolation inside the owning square is one continuous function across every square edge.
//!
//! The cost is samples poleward (twice the ground's need at 60°, 2.9 times at 70°); a local operation takes its radius
//! in metres and each row's column spacing from its latitude.
use crate::SquareKey;

/// Lattice points a degree, of latitude and of longitude: 2.25″ (69.5 m of latitude).
pub const PER_DEGREE: i32 = 1600;
/// Points on a side of a square's window, both edges included.
pub const WINDOW: usize = PER_DEGREE as usize + 1;
/// Blocks on a side of a square: 0.25° each.
pub const BLOCKS: usize = 4;
/// Lattice steps on a side of a block.
pub const BLOCK_STEPS: usize = PER_DEGREE as usize / BLOCKS;
/// Points on a side of a block, both edges included: a block shares its edge points with the next block, so a cell
/// of the lattice lies whole in one block and a reader decodes only the block a place falls in.
pub const BLOCK_SIDE: usize = BLOCK_STEPS + 1;
/// A final height's quantum, metres: land is at least one quantum above the sea and sea one below, never 0.
pub const QUANTUM_M: f64 = 0.25;

/// Longitude steps round the world.
pub const AROUND: i32 = 360 * PER_DEGREE;
/// Latitude steps from the equator to a pole.
pub const POLE: i32 = 90 * PER_DEGREE;

/// A point of the global lattice: latitude `i`/1,600°, longitude `j`/1,600°. Made only by [`Point::new`], which takes
/// `j` into [−180°, 180°) and sets it to 0 at either pole, where every longitude is one place: one place, one point.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    i: i32,
    j: i32,
}

impl Point {
    /// The lattice point at latitude step `i` (−144,000 to 144,000) and longitude step `j` (any: wrapped).
    pub fn new(i: i32, j: i32) -> Self {
        assert!(i.abs() <= POLE, "latitude step {i} is past a pole");
        let j = if i.abs() == POLE {
            0
        } else {
            (j + AROUND / 2).rem_euclid(AROUND) - AROUND / 2
        };
        Self { i, j }
    }
    /// Its latitude step.
    pub fn i(self) -> i32 {
        self.i
    }
    /// Its longitude step, in [−288,000, 288,000).
    pub fn j(self) -> i32 {
        self.j
    }
    /// Its latitude, degrees: exact to the f64's rounding of `i / 1600`.
    pub fn lat(self) -> f64 {
        f64::from(self.i) / f64::from(PER_DEGREE)
    }
    /// Its longitude, degrees, in [−180, 180).
    pub fn lon(self) -> f64 {
        f64::from(self.j) / f64::from(PER_DEGREE)
    }
}

/// Square `key`'s window point at row `r` (from the north edge) and column `c` (from the west edge), each 0 to 1,600.
pub fn window_point(key: SquareKey, r: usize, c: usize) -> Point {
    debug_assert!(r < WINDOW && c < WINDOW);
    Point::new(
        (key.lat + 1) * PER_DEGREE - r as i32,
        key.lon * PER_DEGREE + c as i32,
    )
}

/// A place's fractional (row, column) in square `key`'s window, longitude taken within half a world of the square. For
/// the square that owns the place ([`SquareKey::at`]) the row is in (0, 1,600] and the column in [0, 1,600).
pub fn position(key: SquareKey, lat: f64, lon: f64) -> [f64; 2] {
    let mut d = lon - f64::from(key.lon);
    d -= ((d + 180.0) / 360.0).floor() * 360.0;
    let per = f64::from(PER_DEGREE);
    [(f64::from(key.lat + 1) - lat) * per, d * per]
}

/// Where a fractional window position is read: the block holding its lattice cell, and the position within that block
/// (each in [0, [`BLOCK_STEPS`]]). A position on a block's edge belongs to the block its cell starts in, the last
/// block keeping its far edge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// Block row and column, each 0 to 3.
    pub block: [usize; 2],
    /// Row and column within the block.
    pub at: [f64; 2],
}

/// The block and in-block position of window position `[row, column]` (clamped onto the window).
pub fn cell(position: [f64; 2]) -> Cell {
    let last = (WINDOW - 2) as f64;
    let mut block = [0; 2];
    let mut at = [0.0; 2];
    for k in 0..2 {
        let p = position[k].clamp(0.0, (WINDOW - 1) as f64);
        let b = (p.floor().min(last) as usize) / BLOCK_STEPS;
        block[k] = b;
        at[k] = p - (b * BLOCK_STEPS) as f64;
    }
    Cell { block, at }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_share_their_edge_points() {
        // Within a band, across each of the surveyed layer's band changes, across the DEM's change at 60° N and 56° S,
        // and across ±180°: east and north neighbours, every point of the shared edge.
        for lat in [-57, -56, -31, -1, 0, 29, 30, 44, 45, 59, 60, 84, 85, 88] {
            for lon in [-180, -1, 0, 36, 178, 179] {
                let key = SquareKey { lat, lon };
                let east = key.east();
                for r in 0..WINDOW {
                    assert_eq!(
                        window_point(key, r, WINDOW - 1),
                        window_point(east, r, 0),
                        "{key} {r}"
                    );
                }
                let north = key.north().unwrap();
                for c in 0..WINDOW {
                    assert_eq!(
                        window_point(key, 0, c),
                        window_point(north, WINDOW - 1, c),
                        "{key} {c}"
                    );
                }
            }
        }
        // 180° E is 180° W; every longitude at a pole is one point.
        assert_eq!(Point::new(0, 288_000), Point::new(0, -288_000));
        assert_eq!(Point::new(POLE, 5), Point::new(POLE, -77));
        assert_eq!(Point::new(16_000, 288_000).lon(), -180.0);
    }

    #[test]
    fn a_place_reads_one_cell_of_one_block() {
        let key = SquareKey { lat: 36, lon: -123 };
        let [r, c] = position(key, 36.75, -122.75);
        assert_eq!([r, c], [400.0, 400.0]);
        // On a block's edge the cell that starts there is read.
        assert_eq!(
            cell([400.0, 400.0]),
            Cell {
                block: [1, 1],
                at: [0.0, 0.0]
            }
        );
        // The south and east edges belong to the last block.
        assert_eq!(
            cell([1600.0, 1600.0]),
            Cell {
                block: [3, 3],
                at: [400.0, 400.0]
            }
        );
        assert_eq!(
            cell([399.5, 0.25]),
            Cell {
                block: [0, 0],
                at: [399.5, 0.25]
            }
        );
        // Longitude is taken within half a world of the square.
        let [_, c] = position(SquareKey { lat: 0, lon: 179 }, 0.5, -179.75);
        assert_eq!(c, 2000.0);
        let [_, c] = position(SquareKey { lat: 0, lon: -180 }, 0.5, 179.75);
        assert_eq!(c, -400.0);
    }
}
