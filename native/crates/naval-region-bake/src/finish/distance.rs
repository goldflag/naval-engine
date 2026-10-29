//! Distances on the lattice in metres, capped at a reach: a local operation of that radius. North-south a row is
//! [`ROW_M`]; east-west a column is the spacing of the row it lies in. Each row's nearest set column is found exactly
//! (integers), then each point takes the least over the rows within the reach of
//! `(Δrow · ROW_M)² + (Δcolumn · column_m(row))²`, in a fixed order, so a point's distance is a function of the mask
//! within the reach of it, the same in every region that holds that neighbourhood.
use super::lattice::{Plane, ROW_M, Region, Spacing};

/// Rows a reach spans.
#[inline]
pub fn reach_rows(reach_m: f64) -> usize {
    (reach_m / ROW_M).floor() as usize
}

/// Each point of `out`'s distance in metres to the nearest set point of `mask` (centre to centre), `reach_m` where
/// none lies within it. `out` must lie in the mask's region at least the reach from its edge (rows: [`reach_rows`];
/// columns: the reach over the narrowest column of the mask's rows).
pub fn distance(mask: &Plane<u8>, spacing: &Spacing, reach_m: f64, out: Region) -> Plane<f32> {
    let region = mask.region;
    let (r0, c0) = out
        .offset_in(&region)
        .unwrap_or_else(|| panic!("{out:?} is not inside {region:?}"));
    let rr = reach_rows(reach_m);
    assert!(
        r0 >= rr && r0 + out.rows + rr <= region.rows,
        "the mask holds too few rows for a reach of {reach_m} m"
    );
    // Each row's nearest set column within the reach (u32::MAX when none), over the output's columns.
    const NONE: u32 = u32::MAX;
    let (first, last) = (r0 - rr, r0 + out.rows + rr);
    let mut near = vec![NONE; (last - first) * out.columns];
    for (k, r) in (first..last).enumerate() {
        let i = region.i(r);
        let limit = (reach_m / spacing.column_m(i)).floor() as usize;
        assert!(
            c0 >= limit && c0 + out.columns + limit <= region.columns,
            "the mask holds too few columns for a reach of {reach_m} m"
        );
        let row = &mask.values[r * region.columns..(r + 1) * region.columns];
        let into = &mut near[k * out.columns..(k + 1) * out.columns];
        // West to east, then east to west, over the columns the output's can reach.
        let (lo, hi) = (c0 - limit, c0 + out.columns + limit);
        let mut last_set: Option<usize> = None;
        for c in lo..c0 + out.columns {
            if row[c] != 0 {
                last_set = Some(c);
            }
            if c >= c0
                && let Some(s) = last_set
                && c - s <= limit
            {
                into[c - c0] = (c - s) as u32;
            }
        }
        let mut next_set: Option<usize> = None;
        for c in (c0..hi).rev() {
            if row[c] != 0 {
                next_set = Some(c);
            }
            if c < c0 + out.columns
                && let Some(s) = next_set
                && s - c <= limit
            {
                let d = (s - c) as u32;
                let slot = &mut into[c - c0];
                if d < *slot {
                    *slot = d;
                }
            }
        }
    }
    let cap = reach_m * reach_m;
    let column_m: Vec<f64> = (first..last)
        .map(|r| spacing.column_m(region.i(r)))
        .collect();
    Plane::from_fn(out, |r, c| {
        let mut best = cap;
        for dr in 0..=2 * rr {
            let g = near[(r + dr) * out.columns + c];
            if g == NONE {
                continue;
            }
            let ns = (dr as f64 - rr as f64) * ROW_M;
            let ew = f64::from(g) * column_m[r + dr];
            let d2 = ns * ns + ew * ew;
            if d2 < best {
                best = d2;
            }
        }
        best.sqrt() as f32
    })
}

/// Half a sample at latitude step `i`, metres: half the narrower of its row and its column. A coast lies this far
/// before the first sample past it.
#[inline]
pub fn half_sample(spacing: &Spacing, i: i32) -> f64 {
    0.5 * ROW_M.min(spacing.column_m(i))
}

/// The signed distance to the edge of `inside`, metres: positive inside, the edge half a sample
/// ([`half_sample`]) before the nearest sample across it, capped at the reach.
pub fn signed(inside: &Plane<u8>, spacing: &Spacing, reach_m: f64, out: Region) -> Plane<f32> {
    let outside = inside.map(|v| u8::from(v == 0));
    let to_out = distance(&outside, spacing, reach_m, out);
    let to_in = distance(inside, spacing, reach_m, out);
    let (r0, c0) = out.offset_in(&inside.region).expect("checked by distance");
    Plane::from_fn(out, |r, c| {
        let half = half_sample(spacing, out.i(r));
        if inside.at(r0 + r, c0 + c) != 0 {
            (f64::from(to_out.at(r, c)) - half) as f32
        } else {
            -(f64::from(to_in.at(r, c)) - half) as f32
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The brute-force distance: every set point within the region.
    fn brute(mask: &Plane<u8>, spacing: &Spacing, reach: f64, r: usize, c: usize) -> f64 {
        let region = mask.region;
        let mut best = reach * reach;
        for rr in 0..region.rows {
            for cc in 0..region.columns {
                if mask.at(rr, cc) == 0 {
                    continue;
                }
                let ns = (rr as f64 - r as f64) * ROW_M;
                let ew = (cc as f64 - c as f64) * spacing.column_m(region.i(rr));
                best = best.min(ns * ns + ew * ew);
            }
        }
        best.sqrt()
    }

    fn speckled(region: Region, seed: u64) -> Plane<u8> {
        let mut s = seed;
        Plane::from_fn(region, |_, _| {
            s = s
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            u8::from((s >> 33).is_multiple_of(97))
        })
    }

    #[test]
    fn the_distance_is_the_brute_forces_and_the_regions_origin_does_not_matter() {
        // At 60° N a column is half a row: the reach spans twice the columns it spans rows.
        let region = Region {
            top: 96_000,
            left: 1000,
            rows: 60,
            columns: 90,
        };
        let spacing = Spacing::of(&region);
        let mask = speckled(region, 7);
        let reach = 400.0;
        let out = region.shrink(8, 15);
        let d = distance(&mask, &spacing, reach, out);
        for r in 0..out.rows {
            for c in 0..out.columns {
                let want = brute(&mask, &spacing, reach, r + 8, c + 15);
                assert!((f64::from(d.at(r, c)) - want).abs() < 1e-3, "{r} {c}");
            }
        }
        // A smaller mask about the same output gives the same bits.
        let inner = region.shrink(1, 2);
        let d2 = distance(&mask.crop(inner), &spacing, reach, out);
        assert_eq!(d2, d);
    }
}
