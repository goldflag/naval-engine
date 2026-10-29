//! The component operations of coast finishing: each labels a connected region (4-connected, as the region bake's
//! `grid::components`) and decides it **by the component**, over its own window: its bounding box grown by
//! [`COMPONENT_MARGIN_M`] each side, capped at [`COMPONENT_CAP_M`] each way. A component whose window is larger is
//! kept as it is. A component touching the edge of the region a bake holds is undecided there; a point at least the
//! cap and a sample inside the region always sees its component whole when the component is under the cap, so the
//! answer is the component's, not the asking square's ([`reach`]).
//!
//! - [`keep_islands`]: an island of the coastline that rounding would sink whole keeps its samples.
//! - [`drop_specks`]: land smaller than [`SPECK_AREA`] (two and a half samples at the equator) is dropped.
//! - [`fill_lakes`]: water the land encloses is filled.
//!
//! A component's extent and area are integers (rows, columns, and [`area_units`](super::lattice::area_units)), so its
//! decision is the same whichever sample it is found from.
use super::lattice::{AREA_ONE, Plane, ROW_M, Region, Spacing};

/// A component's window may be at most this, metres, north-south and east-west; past it the component is kept as it
/// is.
pub const COMPONENT_CAP_M: f64 = 1_000.0;
/// The window is the component's bounding box grown by this each side: its shore.
pub const COMPONENT_MARGIN_M: f64 = ROW_M;
/// Land components smaller than this are specks: two and a half samples at the equator (the region bake drops
/// components under three of its 70 m samples), in [`AREA_ONE`]ths of a sample at the equator.
pub const SPECK_AREA: u64 = 5 * AREA_ONE / 2;

/// The rows and columns a component operation needs about a point: the cap (rows; columns at the narrowest column of
/// the region, `column_min_m`) and a sample.
pub fn reach(column_min_m: f64) -> (usize, usize) {
    (
        (COMPONENT_CAP_M / ROW_M).ceil() as usize + 1,
        (COMPONENT_CAP_M / column_min_m).ceil() as usize + 1,
    )
}

/// One component as found in a region.
pub struct Component<'a> {
    /// Its samples, row-major indices into the region.
    pub samples: &'a [u32],
    /// Its bounding box: first and last row, first and last column.
    pub rows: [usize; 2],
    pub columns: [usize; 2],
    /// Its area in [`AREA_ONE`]ths of a sample at the equator.
    pub area: u64,
    /// Whether it touches the region's edge (it may go on beyond).
    pub border: bool,
}

impl Component<'_> {
    /// Whether it is decided: wholly inside the region, its window within the cap each way (east-west measured at its
    /// box's row nearest the equator, the widest).
    pub fn decided(&self, region: &Region, spacing: &Spacing) -> bool {
        if self.border {
            return false;
        }
        let ns = (self.rows[1] - self.rows[0]) as f64 * ROW_M + 2.0 * COMPONENT_MARGIN_M;
        let bbox = Region {
            top: region.i(self.rows[0]),
            left: region.j(self.columns[0]),
            rows: self.rows[1] - self.rows[0] + 1,
            columns: self.columns[1] - self.columns[0] + 1,
        };
        let ew = (self.columns[1] - self.columns[0]) as f64 * spacing.column_m(bbox.equatorward())
            + 2.0 * COMPONENT_MARGIN_M;
        ns <= COMPONENT_CAP_M && ew <= COMPONENT_CAP_M
    }
}

/// Every component of the samples of `mask` equal to `of`, each handed to `f` once.
pub fn each(mask: &Plane<u8>, of: u8, spacing: &Spacing, mut f: impl FnMut(&Component)) {
    let region = mask.region;
    let (rows, columns) = (region.rows, region.columns);
    let mut seen = vec![false; region.len()];
    let mut queue: Vec<u32> = Vec::new();
    for start in 0..region.len() {
        if mask.values[start] != of || seen[start] {
            continue;
        }
        queue.clear();
        seen[start] = true;
        queue.push(start as u32);
        let mut head = 0;
        let (mut r0, mut r1, mut c0, mut c1) = (usize::MAX, 0, usize::MAX, 0);
        let mut area = 0u64;
        while head < queue.len() {
            let at = queue[head] as usize;
            head += 1;
            let (r, c) = (at / columns, at % columns);
            (r0, r1, c0, c1) = (r0.min(r), r1.max(r), c0.min(c), c1.max(c));
            area += spacing.area(region.i(r));
            let mut visit = |n: usize| {
                if mask.values[n] == of && !seen[n] {
                    seen[n] = true;
                    queue.push(n as u32);
                }
            };
            if c > 0 {
                visit(at - 1);
            }
            if c + 1 < columns {
                visit(at + 1);
            }
            if r > 0 {
                visit(at - columns);
            }
            if r + 1 < rows {
                visit(at + columns);
            }
        }
        let border = r0 == 0 || c0 == 0 || r1 + 1 == rows || c1 + 1 == columns;
        f(&Component {
            samples: &queue,
            rows: [r0, r1],
            columns: [c0, c1],
            area,
            border,
        });
    }
}

/// An island of the coastline (`land`) that `rounded` sinks whole keeps its samples: the rounded land with every such
/// decided island restored, over `out`.
pub fn keep_islands(
    land: &Plane<u8>,
    rounded: &Plane<u8>,
    spacing: &Spacing,
    out: Region,
) -> Plane<u8> {
    let region = land.region;
    let rounded = rounded.crop(region);
    let mut kept = rounded.clone();
    each(land, 1, spacing, |island| {
        if island.decided(&region, spacing)
            && island
                .samples
                .iter()
                .all(|&k| rounded.values[k as usize] == 0)
        {
            for &k in island.samples {
                kept.values[k as usize] = 1;
            }
        }
    });
    kept.crop(out)
}

/// `mask` without its decided land components smaller than [`SPECK_AREA`], over `out`.
pub fn drop_specks(mask: &Plane<u8>, spacing: &Spacing, out: Region) -> Plane<u8> {
    let region = mask.region;
    let mut kept = mask.clone();
    each(mask, 1, spacing, |land| {
        if land.area < SPECK_AREA && land.decided(&region, spacing) {
            for &k in land.samples {
                kept.values[k as usize] = 0;
            }
        }
    });
    kept.crop(out)
}

/// `mask` with its decided water filled: water the land encloses within the cap, over `out`. Water larger than the
/// cap (a lagoon, a harbour behind a narrow mouth) is kept: the water polygons say it is the sea.
pub fn fill_lakes(mask: &Plane<u8>, spacing: &Spacing, out: Region) -> Plane<u8> {
    let region = mask.region;
    let mut filled = mask.clone();
    each(mask, 0, spacing, |water| {
        if water.decided(&region, spacing) {
            for &k in water.samples {
                filled.values[k as usize] = 1;
            }
        }
    });
    filled.crop(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plane(region: Region, text: &[&str]) -> Plane<u8> {
        assert_eq!(text.len(), region.rows);
        Plane {
            region,
            values: text
                .iter()
                .flat_map(|row| row.bytes().map(|b| u8::from(b == b'#')))
                .collect(),
        }
    }

    #[test]
    fn specks_go_lakes_fill_and_anything_at_the_edge_waits() {
        let region = Region {
            top: 100,
            left: 0,
            rows: 7,
            columns: 10,
        };
        let spacing = Spacing::of(&region);
        #[rustfmt::skip]
        let mask = plane(region, &[
            "..........",
            ".###....#.",
            ".#.#..#...",
            ".###......",
            "......##..",
            "#.....##..",
            "#.........",
        ]);
        let all = region.shrink(0, 0);
        let specks = drop_specks(&mask, &spacing, all);
        // The lone samples go; the 2 × 2 block (4 samples) and the ring stay; the edge piece is undecided.
        assert_eq!(specks.at(1, 8), 0);
        assert_eq!(specks.at(2, 6), 0);
        assert_eq!(specks.at(4, 6), 1);
        assert_eq!(specks.at(1, 1), 1);
        assert_eq!(
            specks.at(5, 0),
            1,
            "a piece on the edge may go on beyond it"
        );
        let filled = fill_lakes(&specks, &spacing, all);
        assert_eq!(filled.at(2, 2), 1, "the ring's water fills");
        assert_eq!(filled.at(0, 0), 0, "the open water touches the edge");
    }

    #[test]
    fn a_component_past_the_cap_is_kept_as_it_is() {
        // A ring of land enclosing water 14 rows across (13 steps, 903 m, and 2 × 69 m of margin: over 1 km) is kept;
        // 13 rows across (834 m and the margin) fills.
        for (inside, fills) in [(13usize, true), (14, false)] {
            let n = inside + 2;
            let region = Region {
                top: 0,
                left: 0,
                rows: n + 2,
                columns: n + 2,
            };
            let spacing = Spacing::of(&region);
            let mask = Plane::from_fn(region, |r, c| {
                let ring = (1..=n).contains(&r)
                    && (1..=n).contains(&c)
                    && (r == 1 || r == n || c == 1 || c == n);
                u8::from(ring)
            });
            let filled = fill_lakes(&mask, &spacing, region.shrink(0, 0));
            assert_eq!(filled.at(n / 2 + 1, n / 2 + 1) == 1, fills, "{inside}");
        }
    }
}
