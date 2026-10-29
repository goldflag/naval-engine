//! The local operations of coast finishing, each with a fixed reach in metres (the region bake's steps, from its 70 m
//! samples to the lattice's metres; a "sample" below is [`SAMPLE_M`], a lattice row):
//!
//! - [`slide`] and [`cut`], the coast reconciled with the DEM (`coastline::reconcile`'s): the DEM's coastal profile
//!   slid onto the coastline where it lies a sample or two seaward of the DEM's coast; land the coastline cuts from the
//!   DEM's rising from the new shore at most 1 in 1.
//! - [`mean`] and [`rounded`], the coast rounded (`coast::rounded_coast_outline`'s first half): the coastline's
//!   outline averaged over [`COAST_ROUND_M`] each way, the waterline where that mean is even for low samples by the
//!   coast.
//! - [`heights`], the heights and the shelf (its second half): land near the sea shrunk toward 1 m, the shelf off the
//!   finished coast from the mean's waterline, coast samples lifted so the waterline falls between samples.
use super::coverage::{FULL, HALF};
use super::distance::{distance, half_sample, signed};
use super::lattice::{Plane, ROW_M, Region, Spacing};
/// The steepest the ground rises from a coast OpenStreetMap cut into the DEM's land (1 in 1).
pub const CUT_SLOPE: f64 = 1.0;
/// Samples off the coast within which the DEM's profile is slid whole onto OpenStreetMap's line, and beyond which it is
/// not slid at all (between, the slide fades): the blend is a sample or two.
pub const SLIDE_NEAR: f64 = 1.5;
pub const SLIDE_FAR: f64 = 3.5;
/// The farthest the DEM's profile is slid, samples (fading from [`SLIDE_FADE`]): the DEM's own smear of a cliff or a
/// beach across its 70 m samples. A coast moved farther is not the DEM's smear but land it has wrong (a terrace of
/// noise off Big Sur's bluffs, a lagoon filled, reclaimed land): cut or raised as it stands, the ground behind left
/// where it is.
pub const SLIDE_FADE: f64 = 1.0;
pub const SLIDE_MAX: f64 = 2.0;
use crate::{LAND_MIN_M, LAND_THRESHOLD_M, SEA_MAX_M, SHELF_DEPTH_M, SHELF_SLOPE};

/// The region bake's sample, in metres on the lattice: one row.
pub const SAMPLE_M: f64 = ROW_M;
/// The coasts' distances the slide reads ([`SLIDE_FAR`] + [`SLIDE_MAX`] + 2 samples).
pub const SLIDE_DISTANCE_M: f64 = (SLIDE_FAR + SLIDE_MAX + 2.0) * SAMPLE_M;
/// The farthest the DEM's profile is slid: a slide moves `w · delta / g` with `delta` under [`SLIDE_MAX`] samples
/// and the gradient `g` at least ½.
pub const SLIDE_SHIFT_M: f64 = 2.0 * SLIDE_MAX * SAMPLE_M;
/// The cut's ramp reaches this far behind the new shore (6 samples).
pub const CUT_REACH_M: f64 = 6.0 * SAMPLE_M;
/// The coast is rounded over this each way: the box of the region bake's two passes.
pub const COAST_ROUND_M: f64 = crate::COAST_ROUND_M;
/// The rounding applies to samples standing at most this high (land) or this deep (sea, the shelf's depth by the old
/// coast).
const COAST_BAND_M: f64 = 12.0;
const COAST_SHRINK_SHARPNESS: f64 = 3.0;
const COAST_LIFT_M: f64 = 6.0;
/// The sea's distance to the old coast is read this far (the shelf passes [`COAST_BAND_M`] within it).
pub const OLD_COAST_M: f64 = 2.0 * SAMPLE_M;
/// The shelf's floor is reached this far from the coast: (160 − 2.5) / 0.12 m.
pub const SHELF_REACH_M: f64 = (SHELF_DEPTH_M - 2.5) / SHELF_SLOPE;
/// The waterline is taken from the mean, and the coast lifted, within this of the coast (a sample and a half).
pub const NEAR_COAST_M: f64 = 1.5 * SAMPLE_M;
/// Ground behind a lifted coast sample takes half its lift within this.
pub const BEHIND_M: f64 = SAMPLE_M;

fn shelf(distance: f64) -> f64 {
    SHELF_DEPTH_M.min(2.5 + distance * SHELF_SLOPE)
}

/// Where the bathymetry reads less than this deep (or land) within [`SHELF_REACH_M`] of the coast, its own coast is not
/// the polygons' (GEBCO's pixels are about 460 m): the shelf fills it, wholly where it reads land, not at all from this
/// deep.
pub const UNSURE_M: f64 = 10.0;

/// The second bake's depth at a sea point (metres, positive), from the bathymetry's height there (`floor`, negative
/// below the sea), its distance to the finished coast (`d`, as the shelf reads it) and to the nearest open-sea square
/// (`open`). Each step is continuous in its inputs:
///
/// - the bathymetry's depth, or within [`SHELF_REACH_M`] of the coast the shelf's where the bathymetry's coast is not
///   the polygons' ([`UNSURE_M`]), that fill gone by the reach;
/// - within the reach no deeper than the shelf's slope from 2.5 m at the coast, wholly at the coast and not at all at
///   the reach (where the bathymetry stands alone);
/// - faded to the open sea's floor, [`SHELF_DEPTH_M`], within [`super::OPEN_FADE_M`] of an open-sea square.
pub fn seabed_depth(floor: f64, d: f64, open: f64) -> f64 {
    let t = (d / SHELF_REACH_M).clamp(0.0, 1.0);
    let known = (-floor).max(0.0);
    let unsure = ((floor + UNSURE_M) / UNSURE_M).clamp(0.0, 1.0);
    let depth = known.max(shelf(d) * unsure * (1.0 - t));
    let near = depth.min(2.5 + d * SHELF_SLOPE);
    let depth = near * (1.0 - t) + depth * t;
    let w = smoothstep(0.0, super::OPEN_FADE_M, open);
    SHELF_DEPTH_M + (depth - SHELF_DEPTH_M) * w
}

/// The second bake's sea floor over the window: the bathymetry by place, and each point's distance to open sea.
pub struct SeaFloor {
    pub floor: Plane<f64>,
    pub open: Plane<f32>,
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The coastline's land and its outline over a region, from each point's share of the ocean: land below half sea; the
/// outline the land's share from −[`FULL`] (sea) to +[`FULL`] (land), never 0 and of the land's sign.
pub fn coastline(cover: &Plane<u8>) -> (Plane<u8>, Plane<i8>) {
    let land = cover.map(|c| u8::from(c < HALF));
    let outline = cover.map(|c| {
        let o = FULL as i8 - 2 * c as i8;
        if c < HALF { o.max(1) } else { o.min(-1) }
    });
    (land, outline)
}

/// The region bake's `reconcile`, its slide: where the coastline lies a sample or two seaward of the DEM's coast, each
/// sample takes the DEM's height from where the DEM's own coast is as far as the coastline is from it, along the DEM's
/// coast's normal. `raw` the surveyed heights, `land` and `cover` the coastline's. Returns the heights over `out` and
/// the samples slid. Reach: [`SLIDE_DISTANCE_M`] and a sample (the distances' gradient).
pub fn slide(
    raw: &Plane<f32>,
    land: &Plane<u8>,
    cover: &Plane<u8>,
    spacing: &Spacing,
    out: Region,
) -> (Plane<f32>, usize) {
    let region = raw.region;
    let near = out.grow(1, 1);
    let dem_land = raw.map(|h| u8::from(h > LAND_THRESHOLD_M));
    let d_dem = signed(&dem_land, spacing, SLIDE_DISTANCE_M, near);
    drop(dem_land);
    let d_osm = signed(land, spacing, SLIDE_DISTANCE_M, out);
    let (r0, c0) = out
        .offset_in(&region)
        .expect("the slide's output lies in its input");
    let mut slid = 0;
    let mut heights = raw.crop(out);
    for r in 0..out.rows {
        let i = out.i(r);
        let column = spacing.column_m(i);
        let half = half_sample(spacing, i);
        for c in 0..out.columns {
            let (rr, cc) = (r0 + r, c0 + c);
            // The coastline where it crosses the sample: its land share less a half, along the coast's normal.
            let cv = cover.at(rr, cc);
            let mut d_o = f64::from(d_osm.at(r, c));
            if cv > 0 && cv < FULL {
                let share = 1.0 - f64::from(cv) / f64::from(FULL);
                d_o = if land.at(rr, cc) != 0 {
                    (share - 0.5).max(0.0) * 2.0 * half
                } else {
                    (share - 0.5).min(0.0) * 2.0 * half
                };
            }
            let d_d = f64::from(d_dem.at(r + 1, c + 1));
            let delta = d_o - d_d;
            if delta < 1e-6 {
                continue;
            }
            let w = (1.0 - smoothstep(SLIDE_NEAR * SAMPLE_M, SLIDE_FAR * SAMPLE_M, d_o.abs()))
                * (1.0 - smoothstep(SLIDE_FADE * SAMPLE_M, SLIDE_MAX * SAMPLE_M, delta.abs()));
            if w <= 0.0 {
                continue;
            }
            // The DEM coast's normal: the distance's gradient, metres a metre.
            let gx = (f64::from(d_dem.at(r + 1, c + 2)) - f64::from(d_dem.at(r + 1, c)))
                / (2.0 * column);
            let gz =
                (f64::from(d_dem.at(r + 2, c + 1)) - f64::from(d_dem.at(r, c + 1))) / (2.0 * ROW_M);
            let g = (gx * gx + gz * gz).sqrt();
            if g < 0.5 {
                continue;
            }
            let shift = w * delta / g;
            // Rows run south, as the gradient's z does. The offset is taken apart from the sample's index, so its
            // weights are the same in every region that holds the sample.
            let v = bilinear(raw, rr, cc, gz * shift / ROW_M, gx * shift / column);
            heights.set(r, c, v as f32);
            slid += 1;
        }
    }
    (heights, slid)
}

/// The bilinear value of `plane` `dr` rows and `dc` columns (fractional) from row `r0`, column `c0`: the cell it falls
/// in, with the next row and column, must lie in the plane.
fn bilinear(plane: &Plane<f32>, r0: usize, c0: usize, dr: f64, dc: f64) -> f64 {
    let (fr, fc) = (dr.floor(), dc.floor());
    let (v, u) = (dr - fr, dc - fc);
    let (r, c) = (r0 as i64 + fr as i64, c0 as i64 + fc as i64);
    assert!(
        r >= 0
            && c >= 0
            && (r as usize) + 1 < plane.region.rows
            && (c as usize) + 1 < plane.region.columns,
        "a slide reads past its reach"
    );
    let (r, c) = (r as usize, c as usize);
    let at = |rr: usize, cc: usize| f64::from(plane.at(rr, cc));
    at(r, c) * (1.0 - u) * (1.0 - v)
        + at(r, c + 1) * u * (1.0 - v)
        + at(r + 1, c) * (1.0 - u) * v
        + at(r + 1, c + 1) * u * v
}

/// The region bake's `reconcile`, its cut: land the coastline cut from the DEM's (sea where the slid heights still
/// stand above the land threshold) leaves the ground behind it rising from the new shore at most [`CUT_SLOPE`], so no
/// wall stands at the shore. Returns the heights over `out` and the samples lowered. Reach: [`CUT_REACH_M`].
pub fn cut(
    heights: &Plane<f32>,
    land: &Plane<u8>,
    spacing: &Spacing,
    out: Region,
) -> (Plane<f32>, usize) {
    let region = heights.region;
    let land = land.crop(region);
    let cut = Plane::from_fn(region, |r, c| {
        u8::from(land.at(r, c) == 0 && heights.at(r, c) > LAND_THRESHOLD_M)
    });
    let to_cut = distance(&cut, spacing, CUT_REACH_M, out);
    let (r0, c0) = out
        .offset_in(&region)
        .expect("the cut's output lies in its input");
    let mut lowered = 0;
    let out_heights = Plane::from_fn(out, |r, c| {
        let h = f64::from(heights.at(r0 + r, c0 + c));
        let d = f64::from(to_cut.at(r, c));
        if land.at(r0 + r, c0 + c) == 0 || d >= CUT_REACH_M {
            return h as f32;
        }
        let cap = LAND_MIN_M + (d - half_sample(spacing, out.i(r))).max(0.0) * CUT_SLOPE;
        let capped = h.min(cap.max(LAND_MIN_M));
        if capped < h {
            lowered += 1;
        }
        capped as f32
    });
    (out_heights, lowered)
}

/// Each row's triangle half-width in columns for the rounding: twice the box's (2 · round(140 m / column)).
pub fn round_columns(spacing: &Spacing, i: i32) -> usize {
    (COAST_ROUND_M / spacing.column_m(i))
        .round_ties_even()
        .max(1.0) as usize
}

/// The box's reach in rows: round(140 m / row) = 2.
pub fn round_rows() -> usize {
    (COAST_ROUND_M / ROW_M).round_ties_even().max(1.0) as usize
}

/// The outline's mean over `out`, from −1 (all sea) to 1 (all land): two box passes of [`COAST_ROUND_M`] each way (a
/// triangle of twice that), each row's columns by its own spacing. The sums along a row are integers (prefix sums, the
/// same whatever the row's extent); each row's is normalised, then the rows' are weighted in a fixed order.
/// Reach: two boxes, rows and each row's columns.
pub fn mean(outline: &Plane<i8>, spacing: &Spacing, out: Region) -> Plane<f32> {
    let region = outline.region;
    let (r0, c0) = out
        .offset_in(&region)
        .expect("the mean's output lies in its input");
    let rb = round_rows();
    let rt = 2 * rb;
    assert!(
        r0 >= rt && r0 + out.rows + rt <= region.rows,
        "the outline holds too few rows"
    );
    let weights_rows = (2 * rb + 1) as f64 * (2 * rb + 1) as f64;
    // Each row's triangle sum over the output's columns, normalised.
    let rows: Vec<usize> = (r0 - rt..r0 + out.rows + rt).collect();
    let mut across = vec![0f64; rows.len() * out.columns];
    let mut prefix = vec![0i64; region.columns + 1];
    let mut boxed = vec![0i64; region.columns];
    let mut prefix2 = vec![0i64; region.columns + 1];
    for (k, &r) in rows.iter().enumerate() {
        let cb = round_columns(spacing, region.i(r));
        let ct = 2 * cb;
        assert!(
            c0 >= ct && c0 + out.columns + ct <= region.columns,
            "the outline holds too few columns"
        );
        let row = &outline.values[r * region.columns..(r + 1) * region.columns];
        let (lo, hi) = (c0 - ct, c0 + out.columns + ct);
        prefix[lo] = 0;
        for c in lo..hi {
            prefix[c + 1] = prefix[c] + i64::from(row[c]);
        }
        // The first box over the columns the second reads.
        let (blo, bhi) = (c0 - cb, c0 + out.columns + cb);
        for c in blo..bhi {
            boxed[c] = prefix[c + cb + 1] - prefix[c - cb];
        }
        prefix2[blo] = 0;
        for c in blo..bhi {
            prefix2[c + 1] = prefix2[c] + boxed[c];
        }
        let whole = (2 * cb + 1) as f64 * (2 * cb + 1) as f64 * f64::from(FULL);
        for c in 0..out.columns {
            let x = c0 + c;
            let t = prefix2[x + cb + 1] - prefix2[x - cb];
            across[k * out.columns + c] = t as f64 / whole;
        }
    }
    Plane::from_fn(out, |r, c| {
        let mut sum = 0.0;
        for d in 0..=2 * rt {
            let w = (2 * rb + 1) as f64 - (d as f64 - rt as f64).abs();
            sum += w * across[(r + d) * out.columns + c];
        }
        (sum / weights_rows) as f32
    })
}

/// What rounding leaves at a point.
pub struct Rounding {
    /// The rounded land.
    pub rounded: Plane<u8>,
    /// The level the rounding reads: the ground's height on the coastline's land, the shelf's depth by the old coast at
    /// sea.
    pub level: Plane<f32>,
}

/// The rounded land over `out` (pointwise from its inputs, which must cover it): a sample within the coastal band (land
/// at most 12 m high, sea at most 12 m deep on the old shelf) is land where the mean is positive; any other keeps the
/// coastline's word. With `keep_land` (the second bake) the coastline's land stays land whatever the mean: rounding
/// fills a notch but never cuts a headland's tip back (Cape Point's, narrower than the mean's 280 m).
pub fn rounded(
    land: &Plane<u8>,
    ground: &Plane<f32>,
    old_coast: &Plane<f32>,
    mean: &Plane<f32>,
    out: Region,
    keep_land: bool,
) -> Rounding {
    let level = Plane::from_fn(out, |r, c| {
        let (i, j) = (out.i(r), out.j(c));
        if land.get(i, j) != 0 {
            ground.get(i, j)
        } else {
            shelf(f64::from(old_coast.get(i, j))) as f32
        }
    });
    let rounded = Plane::from_fn(out, |r, c| {
        let (i, j) = (out.i(r), out.j(c));
        if f64::from(level.at(r, c)) <= COAST_BAND_M {
            u8::from(mean.get(i, j) > 0.0 || (keep_land && land.get(i, j) != 0))
        } else {
            land.get(i, j)
        }
    });
    Rounding { rounded, level }
}

/// What the heights read beside the finished coast.
pub struct HeightInputs<'a> {
    /// The finished land (after the islands kept, specks and lakes).
    pub coast: &'a Plane<u8>,
    /// The coastline's land, and the rounded land.
    pub land: &'a Plane<u8>,
    pub rounded: &'a Plane<u8>,
    /// The ground (reconciled heights, at least 1 m), the rounding's level and the mean.
    pub ground: &'a Plane<f32>,
    pub level: &'a Plane<f32>,
    pub mean: &'a Plane<f32>,
    /// The second bake's sea floor over the output (none: the first bake's shelf).
    pub sea: Option<&'a SeaFloor>,
}

/// The finished heights over `out`: land shrunk toward 1 m near the sea by the mean's strength, at least 1 m; the
/// shelf off the finished coast, from the mean's waterline within a sample and a half of it, at least 2 m deep; a coast
/// sample lifted so the contour to its nearest sea sample crosses at the waterline, the ground within a sample behind
/// it to half that. Reach: [`SHELF_REACH_M`] (the sea's distance to the coast), and less for the rest.
pub fn heights(inputs: &HeightInputs, spacing: &Spacing, out: Region) -> Plane<f32> {
    let HeightInputs {
        coast,
        land,
        rounded,
        ground,
        level,
        mean,
        sea: seabed,
    } = *inputs;
    if let Some(f) = seabed {
        assert!(
            f.floor.region == out && f.open.region == out,
            "the sea floor covers the output"
        );
    }
    let off = distance(coast, spacing, SHELF_REACH_M, out);
    // The coast samples' distance to the sea and their lift, over `out` and the ground behind its edge.
    let behind = |i: i32| (BEHIND_M / spacing.column_m(i)).floor() as usize;
    let lifted = out.grow(1, behind(out.poleward()) + 1);
    let sea = coast.map(|v| u8::from(v == 0));
    let ashore = distance(&sea, spacing, NEAR_COAST_M, lifted);
    drop(sea);
    // The mean's waterline: how far its zero lies from the point, from its gradient.
    let waterline = |i: i32, j: i32| -> f64 {
        let column = spacing.column_m(i);
        let m = f64::from(mean.get(i, j));
        let gx = (f64::from(mean.get(i, j + 1)) - f64::from(mean.get(i, j - 1))) / (2.0 * column);
        let gz = (f64::from(mean.get(i - 1, j)) - f64::from(mean.get(i + 1, j))) / (2.0 * ROW_M);
        let slope = (gx * gx + gz * gz).sqrt();
        if slope > 0.0 {
            m.abs() / slope
        } else {
            f64::INFINITY
        }
    };
    let lift = Plane::from_fn(lifted, |r, c| {
        let (i, j) = (lifted.i(r), lifted.j(c));
        let to_land = f64::from(ashore.at(r, c));
        if coast.get(i, j) == 0 || to_land > NEAR_COAST_M || mean.get(i, j) <= 0.0 {
            return 0f32;
        }
        let w = waterline(i, j);
        let to_sea = to_land - w;
        let lift = if to_sea > 0.0 {
            (2.5 + to_sea * SHELF_SLOPE) * w / to_sea
        } else {
            COAST_BAND_M
        };
        lift.min(COAST_LIFT_M) as f32
    });
    Plane::from_fn(out, |r, c| {
        let (i, j) = (out.i(r), out.j(c));
        if coast.get(i, j) == 0 {
            let d = f64::from(off.at(r, c));
            let d = if d <= NEAR_COAST_M && mean.get(i, j) <= 0.0 {
                d.min(waterline(i, j))
            } else {
                d
            };
            let depth = match seabed {
                None => shelf(d),
                Some(f) => seabed_depth(f.floor.at(r, c), d, f64::from(f.open.at(r, c))),
            };
            return (-depth).min(SEA_MAX_M) as f32;
        }
        let m = f64::from(mean.get(i, j));
        let shrink = (m.abs() * COAST_SHRINK_SHARPNESS).min(1.0);
        let lv = f64::from(level.get(i, j));
        let low = lv.min(COAST_BAND_M);
        let h = if land.get(i, j) != 0 {
            lv - low + low * shrink
        } else if rounded.get(i, j) != 0 {
            low * shrink
        } else {
            f64::from(ground.get(i, j))
        }
        .max(LAND_MIN_M);
        let mut raise = f64::from(lift.get(i, j));
        if raise == 0.0 {
            let k = behind(i) as i32;
            for di in -1..=1 {
                for dj in -k..=k {
                    raise = raise.max(f64::from(lift.get(i + di, j + dj)));
                }
            }
            raise *= 0.5;
        }
        h.max(raise) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seabed_meets_the_shelf_at_the_coast_the_bathymetry_past_the_reach_and_open_sea_by_its_fade()
     {
        let far = super::super::OPEN_FADE_M;
        // At the coast: the shelf's 2.5 m whatever the bathymetry says.
        for floor in [5.0, -3.0, -40.0, -900.0] {
            assert!(
                (seabed_depth(floor, 0.0, far) - 2.5).abs() < 1e-9,
                "{floor}"
            );
        }
        // Past the reach: the bathymetry's own depth (the bank, the strait, the canyon); its land is the least depth
        // the heights then hold (2 m, `SEA_MAX_M`).
        for floor in [-6.0, -53.0, -740.0, -4_700.0] {
            assert!((seabed_depth(floor, SHELF_REACH_M, far) + floor).abs() < 1e-9);
            assert!((seabed_depth(floor, 30_000.0, far) + floor).abs() < 1e-9);
        }
        assert_eq!(seabed_depth(12.0, SHELF_REACH_M, far), 0.0);
        // Within it, where the bathymetry reads land the shelf fills in, gone by the reach; never deeper than the
        // shelf's slope from the coast.
        let d = 400.0;
        assert!((seabed_depth(25.0, d, far) - shelf(d) * (1.0 - d / SHELF_REACH_M)).abs() < 1e-9);
        assert!(seabed_depth(-900.0, d, far) < 900.0);
        // Continuous in each input: no step at the reach, at the unsure depth, at the fade's ends.
        for (a, b) in [
            (
                seabed_depth(-900.0, SHELF_REACH_M - 1e-6, far),
                seabed_depth(-900.0, SHELF_REACH_M, far),
            ),
            (
                seabed_depth(-UNSURE_M - 1e-9, d, far),
                seabed_depth(-UNSURE_M + 1e-9, d, far),
            ),
            (
                seabed_depth(0.0 - 1e-9, d, far),
                seabed_depth(0.0 + 1e-9, d, far),
            ),
            (seabed_depth(-60.0, 5_000.0, 0.0), SHELF_DEPTH_M),
            (seabed_depth(-60.0, 5_000.0, far), 60.0),
        ] {
            assert!((a - b).abs() < 1e-6, "{a} {b}");
        }
    }

    #[test]
    fn the_constants_are_the_region_bakes_in_metres() {
        assert_eq!(round_rows(), 2);
        let s = Spacing::of(&Region {
            top: 0,
            left: 0,
            rows: 1,
            columns: 1,
        });
        assert_eq!(round_columns(&s, 0), 2);
        assert!((SHELF_REACH_M - 1312.5).abs() < 1e-9);
        assert!((SLIDE_DISTANCE_M / SAMPLE_M - 7.5).abs() < 1e-12);
    }
}
