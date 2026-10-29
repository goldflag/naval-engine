//! The final lattice as the bake measures it: each row's spacing in metres, a rectangle of the lattice ([`Region`])
//! and values over one ([`Plane`]).
//!
//! - **Metres.** A row is [`ROW_M`] north to south everywhere; a column is [`column_m`] east to west, `ROW_M · cos φ`
//!   at the row's latitude. The cosine is a polynomial of `+ − × ÷` alone ([`cos_step`]), never the platform's libm, so
//!   every target measures the same metres and makes the same quanta.
//! - **A region** is rows from a latitude step `top` southward and columns from a longitude step `left` eastward, the
//!   longitude continuous about the square being baked (it runs past ±180° where the square's halo does). A column's
//!   place is its step wrapped ([`Region::point`]): one place, one lattice point.
use naval_land_format::SquareKey;
use naval_land_format::lattice::{AROUND, PER_DEGREE, POLE, Point, WINDOW};

/// Metres a lattice row, north to south: 1/1,600° of the theatre's sphere (69.49 m).
pub const ROW_M: f64 = crate::EARTH_RADIUS_M * std::f64::consts::PI / 180.0 / PER_DEGREE as f64;
/// Radians a lattice step.
const RAD_PER_STEP: f64 = std::f64::consts::PI / 180.0 / PER_DEGREE as f64;
/// A component's area is counted in these parts of a sample at the equator ([`area_units`]): exact integers, so the
/// sum is the same in any order.
pub const AREA_ONE: u64 = 1 << 32;

/// The cosine of latitude step `i` (`i`/1,600°, |i| ≤ 144,000): reduced exactly in steps to [0°, 45°], then Taylor
/// polynomials through x²² (error below 10⁻¹⁷ there), in `+ − × ÷` alone.
pub fn cos_step(i: i32) -> f64 {
    let a = i.unsigned_abs() as i32;
    assert!(a <= POLE, "latitude step {i} is past a pole");
    if a <= POLE / 2 {
        cos_poly(f64::from(a) * RAD_PER_STEP)
    } else {
        sin_poly(f64::from(POLE - a) * RAD_PER_STEP)
    }
}

/// The sine and cosine of `x` radians, in `+ − × ÷` alone: reduced to [−π/4, π/4] by quarter turns, then the same
/// polynomials. For generated sources, which must be the same on every target too.
pub fn sin_cos(x: f64) -> (f64, f64) {
    use std::f64::consts::{FRAC_PI_2, TAU};
    let r = x - (x / TAU).round() * TAU;
    let q = (r / FRAC_PI_2).round();
    let y = r - q * FRAC_PI_2;
    let (s, c) = (sin_poly(y), cos_poly(y));
    match (q as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

fn cos_poly(x: f64) -> f64 {
    let x2 = x * x;
    // 1 − x²/(1·2)(1 − x²/(3·4)(1 − …)), innermost first.
    let mut t = 1.0;
    for n in (1..=11).rev() {
        let (a, b) = (f64::from(2 * n - 1), f64::from(2 * n));
        t = 1.0 - x2 / (a * b) * t;
    }
    t
}

fn sin_poly(x: f64) -> f64 {
    let x2 = x * x;
    let mut t = 1.0;
    for n in (1..=11).rev() {
        let (a, b) = (f64::from(2 * n), f64::from(2 * n + 1));
        t = 1.0 - x2 / (a * b) * t;
    }
    x * t
}

/// Metres a lattice column at latitude step `i`, east to west.
#[inline]
pub fn column_m(i: i32) -> f64 {
    ROW_M * cos_step(i)
}

/// A sample's area at latitude step `i` in [`AREA_ONE`]ths of one at the equator (`ROW_M`²): an integer, so a
/// component's area is the same whichever sample it is counted from.
pub fn area_units(i: i32) -> u64 {
    (cos_step(i) * AREA_ONE as f64).round() as u64
}

/// A rectangle of the global lattice: `rows` from latitude step `top` southward, `columns` from longitude step `left`
/// eastward, longitude continuous.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Region {
    pub top: i32,
    pub left: i32,
    pub rows: usize,
    pub columns: usize,
}

impl Region {
    /// Square `key`'s final window: its 1,601 × 1,601 points, both edges included.
    pub fn window(key: SquareKey) -> Self {
        Self {
            top: (key.lat + 1) * PER_DEGREE,
            left: key.lon * PER_DEGREE,
            rows: WINDOW,
            columns: WINDOW,
        }
    }

    pub fn len(&self) -> usize {
        self.rows * self.columns
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Row `r`'s latitude step.
    #[inline]
    pub fn i(&self, r: usize) -> i32 {
        self.top - r as i32
    }

    /// Column `c`'s longitude step, continuous (not wrapped).
    #[inline]
    pub fn j(&self, c: usize) -> i32 {
        self.left + c as i32
    }

    /// The place of row `r`, column `c`: its lattice point, longitude wrapped.
    #[inline]
    pub fn point(&self, r: usize, c: usize) -> Point {
        Point::new(self.i(r), self.j(c))
    }

    /// The region `rows` and `columns` larger on every side.
    pub fn grow(&self, rows: usize, columns: usize) -> Self {
        Self {
            top: self.top + rows as i32,
            left: self.left - columns as i32,
            rows: self.rows + 2 * rows,
            columns: self.columns + 2 * columns,
        }
    }

    /// The region `rows` and `columns` smaller on every side.
    pub fn shrink(&self, rows: usize, columns: usize) -> Self {
        assert!(
            self.rows > 2 * rows && self.columns > 2 * columns,
            "{self:?} cannot shrink by {rows} rows and {columns} columns"
        );
        Self {
            top: self.top - rows as i32,
            left: self.left + columns as i32,
            rows: self.rows - 2 * rows,
            columns: self.columns - 2 * columns,
        }
    }

    /// Where `self` starts in `outer` (row, column), when it lies wholly inside it.
    pub fn offset_in(&self, outer: &Region) -> Option<(usize, usize)> {
        let r = outer.top - self.top;
        let c = self.left - outer.left;
        (r >= 0
            && c >= 0
            && r as usize + self.rows <= outer.rows
            && c as usize + self.columns <= outer.columns)
            .then_some((r as usize, c as usize))
    }

    /// The row farthest from the equator (the narrowest columns), as a latitude step.
    pub fn poleward(&self) -> i32 {
        let bottom = self.top - self.rows as i32 + 1;
        if self.top.abs() >= bottom.abs() {
            self.top
        } else {
            bottom
        }
    }

    /// The row of `self` nearest the equator (the widest columns), as a latitude step.
    pub fn equatorward(&self) -> i32 {
        let bottom = self.top - self.rows as i32 + 1;
        if bottom <= 0 && self.top >= 0 {
            0
        } else if self.top < 0 {
            self.top
        } else {
            bottom
        }
    }
}

/// The row of the polar squares' near edges (89° N and 89° S): the square table answers a `polar` square as open sea
/// and holds no heights for it.
pub const POLAR_ROW: i32 = 89 * PER_DEGREE;

/// Whether row `i` is a polar place, on a polar square's edge or inside one: open sea, whatever the sources say, so that
/// the edge a baked square shares with a polar square is sea at every point, as the polar square is.
#[inline]
pub fn polar_row(i: i32) -> bool {
    i.abs() >= POLAR_ROW
}

/// The longitude step `j` taken onto the world, [−180°, 180°).
#[inline]
pub fn wrap_j(j: i32) -> i32 {
    (j + AROUND / 2).rem_euclid(AROUND) - AROUND / 2
}

/// Each row's spacing over a region: what every operation reads its metres from.
#[derive(Clone, Debug)]
pub struct Spacing {
    top: i32,
    column_m: Vec<f64>,
    area: Vec<u64>,
}

impl Spacing {
    pub fn of(region: &Region) -> Self {
        Self {
            top: region.top,
            column_m: (0..region.rows).map(|r| column_m(region.i(r))).collect(),
            area: (0..region.rows).map(|r| area_units(region.i(r))).collect(),
        }
    }

    /// Metres a column at latitude step `i`.
    #[inline]
    pub fn column_m(&self, i: i32) -> f64 {
        self.column_m[(self.top - i) as usize]
    }

    /// A sample's area at latitude step `i`, [`area_units`].
    #[inline]
    pub fn area(&self, i: i32) -> u64 {
        self.area[(self.top - i) as usize]
    }
}

/// Values over a region, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Plane<T> {
    pub region: Region,
    pub values: Vec<T>,
}

impl<T: Copy> Plane<T> {
    pub fn filled(region: Region, value: T) -> Self {
        Self {
            region,
            values: vec![value; region.len()],
        }
    }

    /// The plane whose value at row `r`, column `c` is `f(r, c)`.
    pub fn from_fn(region: Region, mut f: impl FnMut(usize, usize) -> T) -> Self {
        let mut values = Vec::with_capacity(region.len());
        for r in 0..region.rows {
            for c in 0..region.columns {
                values.push(f(r, c));
            }
        }
        Self { region, values }
    }

    #[inline]
    pub fn at(&self, r: usize, c: usize) -> T {
        self.values[r * self.region.columns + c]
    }

    #[inline]
    pub fn set(&mut self, r: usize, c: usize, v: T) {
        self.values[r * self.region.columns + c] = v;
    }

    /// The value at latitude step `i` and continuous longitude step `j`, which must lie in the region.
    #[inline]
    pub fn get(&self, i: i32, j: i32) -> T {
        let (r, c) = (
            (self.region.top - i) as usize,
            (j - self.region.left) as usize,
        );
        debug_assert!(r < self.region.rows && c < self.region.columns);
        self.at(r, c)
    }

    /// The same values over `inner`, which must lie in the region.
    pub fn crop(&self, inner: Region) -> Self {
        let (r0, c0) = inner
            .offset_in(&self.region)
            .unwrap_or_else(|| panic!("{inner:?} is not inside {:?}", self.region));
        let mut values = Vec::with_capacity(inner.len());
        for r in 0..inner.rows {
            let at = (r0 + r) * self.region.columns + c0;
            values.extend_from_slice(&self.values[at..at + inner.columns]);
        }
        Self {
            region: inner,
            values,
        }
    }

    pub fn map<U: Copy>(&self, f: impl Fn(T) -> U) -> Plane<U> {
        Plane {
            region: self.region,
            values: self.values.iter().map(|&v| f(v)).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_polar_rows_are_the_polar_squares_and_their_near_edges() {
        use naval_land_format::table::polar;
        let key = |lat| SquareKey { lat, lon: 0 };
        // 89° N is the south edge of N89 (polar) and the north edge of N88 (baked); 89° S the north edge of S90 (polar)
        // and the south edge of S89 (baked).
        assert!(polar(key(89)) && !polar(key(88)) && polar(key(-90)) && !polar(key(-89)));
        let n88 = Region::window(key(88));
        let s89 = Region::window(key(-89));
        assert!(polar_row(n88.top) && !polar_row(n88.top - 1));
        let s89_south = s89.i(s89.rows - 1);
        assert!(polar_row(s89_south) && !polar_row(s89_south + 1));
    }

    #[test]
    fn the_polynomial_cosine_is_libms_to_the_last_bits() {
        for i in (0..=POLE).step_by(7) {
            let exact = (f64::from(i) / 1600.0).to_radians().cos();
            assert!((cos_step(i) - exact).abs() < 1e-15, "{i}");
            assert_eq!(cos_step(i), cos_step(-i));
        }
        assert_eq!(cos_step(0), 1.0);
        assert!((ROW_M - 69.4968).abs() < 1e-4, "{ROW_M}");
        assert!(cos_step(POLE).abs() < 1e-16);
        for k in -400..400 {
            let x = f64::from(k) * 0.0371;
            let (s, c) = sin_cos(x);
            assert!(
                (s - x.sin()).abs() < 1e-14 && (c - x.cos()).abs() < 1e-14,
                "{x}"
            );
        }
    }

    #[test]
    fn regions_grow_shrink_and_crop() {
        let key = SquareKey { lat: -1, lon: 179 };
        let w = Region::window(key);
        assert_eq!((w.top, w.left, w.rows, w.columns), (0, 286_400, 1601, 1601));
        let g = w.grow(10, 20);
        assert_eq!(g.shrink(10, 20), w);
        assert_eq!(w.offset_in(&g), Some((10, 20)));
        assert_eq!(g.equatorward(), 0);
        assert_eq!(g.poleward(), -1610);
        // The east edge at 180° is the world's −180°.
        assert_eq!(w.point(0, 1600), Point::new(0, -288_000));
        assert_eq!(wrap_j(288_000 + 5), -288_000 + 5);
        let p = Plane::from_fn(g, |r, c| (r * 1000 + c) as u32);
        let q = p.crop(w);
        assert_eq!(q.at(0, 0), 10 * 1000 + 20);
        assert_eq!(q.get(w.top, w.left), p.get(w.top, w.left));
    }
}
