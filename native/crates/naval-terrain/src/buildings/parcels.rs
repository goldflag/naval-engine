//! A coarse footprint split into the buildings it holds (native/docs/terrain/buildings.md, "Parcels"). Microsoft's
//! model traces a dense core's roofs as one outline a block (a terrace's row of houses, a street's palazzi, a city
//! block's flats under one ring), so the block stood as one box: one height, one colour, one window grid and one flat
//! roof the size of the block. Here a block or row on residential, commercial or unmapped land is cut back into the
//! buildings a street front holds:
//!
//! - **Frontages.** Cut across its length every few metres by its region's and kind's frontage ([`frontage`]): a
//!   British terrace's houses 4.6-6.4 m wide, a Mediterranean street's palazzi 9-20 m, a Gründerzeit block's houses
//!   13-24 m.
//! - **Rows.** A block deeper than one building ([`ONE_ROW_M`]) is two rows back to back, cut apart a little off its
//!   middle, each row's frontages its own; a block deeper than two ([`COURT_M`]) has a low court between its rows
//!   (one storey: the workshops, stores and yards behind a perimeter block's houses) and a building across each end.
//! - **Each its own.** Every parcel is planned as a building of its block's kind ([`plan_building`]): its own storeys a
//!   step or two either side of the block's ([`steps`]: a Mediterranean street's skyline broken, a terrace's houses
//!   nearly level), its walls and roof from its region's palettes, its roof's shape by its region's rules (a
//!   terrace's houses pitched along the street, some of a Mediterranean block's houses under hipped tile), its bays,
//!   windows and seed by its own hash. A row's houses keep the row's brick or render now and then ([`keeps_row`]).
//!
//! A row is cut only along its length, and only when it is long for its depth and deeper than a house is wide
//! ([`ROW_DEEP_M`]): one deep, narrow terraced house (as OSM maps them) or a pair of semis stands whole. The rule reads
//! the outline alone, so a chunk's whole and boxed levels cut it alike. Every cut is checked to cross the outline once
//! ([`crossings`]: twice its edge), so each piece is the outline's part between the cuts, whole; a cut that would part
//! an L's or a U's arms is left out. A footprint whose pieces would
//! be slivers, or would not all stand (one in the sea, one over a cliff), stands whole as before. Every choice hashes
//! the footprint's place in the theatre, so a chunk made again, at either level of detail, cuts it alike.

use super::geometry::{self, Rect};
use super::{
    Building, Footprint, Kind, Period, Region, Setting, Usage, hash01, plan_building, storey_height,
};

/// A row of houses is at least this deep (m): a narrower footprint is one house, its street at its end.
pub const ROW_DEEP_M: f32 = 7.0;

/// A block deeper than this (m) is two rows of buildings back to back.
pub const ONE_ROW_M: f32 = 24.0;

/// A block deeper than this (m) has a court between its two rows and a building across each end.
pub const COURT_M: f32 = 46.0;

/// The least piece a footprint is cut into (m²): a narrower one leaves the footprint whole.
pub const LEAST_PIECE_M2: f32 = 14.0;

/// Past the outline by this (m), so a band's sides that are not cuts lie outside it.
const MARGIN_M: f32 = 2.0;

/// The frontage a building of `kind` takes along its street in `region` (m): its least and most.
pub fn frontage(region: Region, kind: Kind) -> [f32; 2] {
    use Region::*;
    if kind == Kind::Terrace {
        return match region {
            // Victorian and Edwardian terraces, Dutch and Flemish town houses.
            British | Celtic | NorthSea => [4.6, 6.4],
            // Row houses of 15-20 ft (Philadelphia's, Baltimore's), brownstones.
            American => [4.8, 7.0],
            // Malta's townhouses; the Mediterranean's, the Aegean's and the Maghreb's narrow houses.
            Malta | Mediterranean | Aegean | NorthAfrica | Nile | Arid => [5.5, 9.0],
            // Machiya and nagaya.
            EastAsia => [4.5, 7.5],
            // Townhouses about a suburb's courts.
            West | Florida | Hawaii => [6.5, 9.5],
            _ => [5.5, 9.0],
        };
    }
    match region {
        // Gründerzeit and Nordic blocks: houses of five to seven bays.
        Temperate | Nordic | Iceland => [13.0, 24.0],
        // Georgian and Victorian town houses of three to five storeys, mansion blocks.
        British | Celtic | NorthSea => [7.0, 16.0],
        // Palazzi and casas: a few windows to a street's house.
        Mediterranean => [9.0, 20.0],
        // Polykatoikia on narrow plots; Malta's townhouses of a core.
        Aegean => [9.0, 16.0],
        Malta => [7.0, 13.0],
        NorthAfrica | Nile | Arid => [8.0, 15.0],
        EastAsia => [8.0, 16.0],
        // Downtown lots of 25-50 ft and their doubles.
        American | Florida | Hawaii => [8.0, 22.0],
        West => [12.0, 30.0],
        _ => [9.0, 18.0],
    }
}

/// How a piece's storeys step from its block's: the shares of two down, one down, one up and two up (the rest the
/// block's). A terrace's houses, built as a row, nearly level; a European block's houses within a storey, and
/// Valletta's, a planned town's, little more; the warm countries' streets, built house by house, broken.
pub fn steps(region: Region, kind: Kind) -> [f32; 4] {
    use Region::*;
    let terrace = kind == Kind::Terrace;
    match region {
        British | Celtic | NorthSea | American if terrace => [0.0, 0.05, 0.08, 0.0],
        Malta if terrace => [0.0, 0.1, 0.12, 0.0],
        _ if terrace => [0.0, 0.15, 0.2, 0.03],
        Temperate | Nordic | Iceland | British | Celtic | NorthSea | American => {
            [0.02, 0.2, 0.2, 0.05]
        }
        Malta => [0.03, 0.18, 0.18, 0.04],
        _ => [0.07, 0.23, 0.22, 0.1],
    }
}

/// The share of a row's houses that keep its walls (and roof, when pitched as its): a brick terrace built at once
/// mostly; a warm country's painted houses each their own.
pub fn keeps_row(region: Region) -> f32 {
    use Region::*;
    match region {
        British | Celtic | American => 0.75,
        NorthSea | Temperate | Nordic | Iceland => 0.55,
        _ => 0.3,
    }
}

/// How often the line through `p` along `dir` crosses `ring`'s edges.
pub fn crossings(ring: &[[f32; 2]], p: [f32; 2], dir: [f32; 2]) -> usize {
    // A point's side of the line: its cross product with the direction, a point on the line counted as on the left.
    let side = |q: [f32; 2]| (q[0] - p[0]) * dir[1] - (q[1] - p[1]) * dir[0] >= 0.0;
    let n = ring.len();
    (0..n)
        .filter(|&i| side(ring[i]) != side(ring[(i + 1) % n]))
        .count()
}

/// A band of the footprint's rectangle, in its frame (`u` along its axis, `v` across), cut along `u` (its street runs
/// along the axis) or along `v` (an end's building, its street the cross street).
struct Band {
    u: [f32; 2],
    v: [f32; 2],
    along_u: bool,
}

/// The world point of the rectangle's frame (u, v).
fn at(rect: &Rect, u: f32, v: f32) -> [f32; 2] {
    let a = rect.axis;
    let n = [-a[1], a[0]];
    [
        rect.centre[0] + a[0] * u + n[0] * v,
        rect.centre[1] + a[1] * u + n[1] * v,
    ]
}

/// The counter-clockwise rectangle [u0, u1] × [v0, v1] of the frame, in the world.
fn quad(rect: &Rect, u: [f32; 2], v: [f32; 2]) -> [[f32; 2]; 4] {
    [
        at(rect, u[0], v[0]),
        at(rect, u[1], v[0]),
        at(rect, u[1], v[1]),
        at(rect, u[0], v[1]),
    ]
}

/// Whether the frame's line `u = t` (or `v = t`) crosses `ring` once.
fn cuts_once(ring: &[[f32; 2]], rect: &Rect, t: f32, along_u: bool) -> bool {
    let a = rect.axis;
    let n = [-a[1], a[0]];
    let (p, dir) = if along_u {
        (at(rect, t, 0.0), n)
    } else {
        (at(rect, 0.0, t), a)
    };
    crossings(ring, p, dir) == 2
}

/// `footprint` (planned whole as `whole`) cut into the buildings it holds, each planned; empty when it stands
/// whole: not a block or a row (or an office in a past period) on residential, commercial or unmapped land, shorter
/// than a few frontages, too far from rectangular, a point of interest or a pier on it, a source's lot (the infill's),
/// a row short or narrow for a row of houses, or any piece a sliver or failing to stand.
#[allow(clippy::too_many_arguments)]
pub fn split(
    footprint: &Footprint,
    whole: &Building,
    chunk: [i32; 2],
    density: f32,
    region: Region,
    period: Period,
    setting: &Setting,
    ground: &dyn Fn([f32; 2]) -> f32,
) -> Vec<Building> {
    let kind = whole.kind;
    let eligible = match kind {
        Kind::Terrace | Kind::Block => true,
        // An office of the time is a commercial street's masonry houses.
        Kind::Office => period.is_past(),
        _ => false,
    };
    if !eligible
        || footprint.kind.is_some()
        || setting.poi.is_some()
        || setting.pier.is_some()
        || !matches!(
            setting.usage,
            Usage::Unknown | Usage::Residential | Usage::Commercial
        )
    {
        return Vec::new();
    }
    let Some(ring) = geometry::simplify(&footprint.ring, 0.4) else {
        return Vec::new();
    };
    let Some(rect) = geometry::min_area_rect(&ring) else {
        return Vec::new();
    };
    let area = geometry::area(&ring);
    if area < 0.6 * rect.area() {
        return Vec::new();
    }
    let [least, most] = frontage(region, kind);
    let (long, short) = (2.0 * rect.half[0], 2.0 * rect.half[1]);
    let deep = if kind == Kind::Terrace {
        // A row's houses run along its length, one deep.
        if short < ROW_DEEP_M || long < 2.5 * short || long < 3.0 * least + 1.0 {
            return Vec::new();
        }
        0.0
    } else {
        short
    };
    // Too short for a few frontages and too shallow for two rows: one building.
    if long < 2.5 * least && deep <= ONE_ROW_M {
        return Vec::new();
    }
    // The footprint's hashes, by where it stands in the theatre (decimetres), as `plan_building`'s.
    let centre = geometry::centroid(&ring);
    let hx = i64::from(chunk[0]) * 10_000 + (centre[0] * 10.0) as i64;
    let hz = i64::from(chunk[1]) * 10_000 + (centre[1] * 10.0) as i64;
    let h = |k: i64| hash01(hx, hz, k);

    let (lu, lv) = (rect.half[0], rect.half[1]);
    let (ou, ov) = (lu + MARGIN_M, lv + MARGIN_M);
    let mut bands: Vec<Band> = Vec::new();
    // The court between the rows, when there is one.
    let mut court: Option<([f32; 2], [f32; 2])> = None;
    let row = |v: [f32; 2]| Band {
        u: [-ou, ou],
        v,
        along_u: true,
    };
    if deep <= ONE_ROW_M {
        bands.push(row([-ov, ov]));
    } else if deep <= COURT_M {
        let mid = (h(31) - 0.5) * 0.3 * short;
        if cuts_once(&ring, &rect, mid, false) {
            bands.push(row([-ov, mid]));
            bands.push(row([mid, ov]));
        } else {
            bands.push(row([-ov, ov]));
        }
    } else {
        let front = -lv + 12.0 + 6.0 * h(32);
        let back = lv - 12.0 - 6.0 * h(33);
        let first = -lu + 12.0 + 6.0 * h(34);
        let last = lu - 12.0 - 6.0 * h(35);
        if last - first >= 6.0
            && [(front, false), (back, false), (first, true), (last, true)]
                .iter()
                .all(|&(t, along_u)| cuts_once(&ring, &rect, t, along_u))
        {
            bands.push(row([-ov, front]));
            bands.push(row([back, ov]));
            for u in [[-ou, first], [last, ou]] {
                bands.push(Band {
                    u,
                    v: [front, back],
                    along_u: false,
                });
            }
            court = Some(([first, last], [front, back]));
        } else {
            bands.push(row([-ov, ov]));
        }
    }

    // Each band cut into frontages: from the outline's end, a frontage at a time while a building's width is left.
    let mut pieces: Vec<(Vec<[f32; 2]>, bool)> = Vec::new();
    for (b, band) in bands.iter().enumerate() {
        let (lo, hi) = if band.along_u {
            (band.u[0].max(-lu), band.u[1].min(lu))
        } else {
            (band.v[0].max(-lv), band.v[1].min(lv))
        };
        let mut cuts = Vec::new();
        let mut t = lo;
        for k in 0..64 {
            let w = least + (most - least) * h(40 + 64 * b as i64 + k);
            if t + w + 0.8 * least > hi {
                break;
            }
            t += w;
            if cuts_once(&ring, &rect, t, band.along_u) {
                cuts.push(t);
            }
        }
        let (start, end) = if band.along_u {
            (band.u[0], band.u[1])
        } else {
            (band.v[0], band.v[1])
        };
        let edges: Vec<f32> = std::iter::once(start)
            .chain(cuts)
            .chain(std::iter::once(end))
            .collect();
        for w in edges.windows(2) {
            let (u, v) = if band.along_u {
                ([w[0], w[1]], band.v)
            } else {
                (band.u, [w[0], w[1]])
            };
            pieces.push((geometry::clip_polygon(&ring, &quad(&rect, u, v)), false));
        }
    }
    if let Some((u, v)) = court {
        pieces.push((geometry::clip_polygon(&ring, &quad(&rect, u, v)), true));
    }
    pieces.retain(|(p, _)| !p.is_empty());
    if pieces.len() < 2 {
        return Vec::new();
    }
    // Whole pieces only: no sliver, and the pieces the outline's area between them.
    let mut sum = 0.0;
    for (p, _) in &pieces {
        let a = geometry::area(p);
        if a < LEAST_PIECE_M2 {
            return Vec::new();
        }
        sum += a;
    }
    if (sum - area).abs() > 0.02 * area {
        return Vec::new();
    }

    let storey_m = storey_height(kind);
    // The block's eaves, as its source or estimate gave them.
    let base = whole.height;
    let [down2, down1, up1, up2] = steps(region, kind);
    let least_storeys = if kind == Kind::Terrace { 1.0 } else { 2.0 };
    let mut out = Vec::with_capacity(pieces.len());
    for (ring, court) in pieces {
        let c = geometry::centroid(&ring);
        let px = i64::from(chunk[0]) * 10_000 + (c[0] * 10.0) as i64;
        let pz = i64::from(chunk[1]) * 10_000 + (c[1] * 10.0) as i64;
        let ph = |k: i64| hash01(px, pz, k);
        let height = if court {
            storey_m + 0.4
        } else {
            let t = ph(81);
            let step = if t < down2 {
                -2.0
            } else if t < down2 + down1 {
                -1.0
            } else if t < down2 + down1 + up1 {
                1.0
            } else if t < down2 + down1 + up1 + up2 {
                2.0
            } else {
                0.0
            };
            (base + step * storey_m).max(least_storeys * storey_m + 0.4)
        };
        let piece = Footprint {
            ring,
            height: Some(height),
            kind: Some(kind),
            colour: footprint.colour,
        };
        let Some(mut b) = plan_building(&piece, chunk, density, region, period, setting, ground)
        else {
            return Vec::new();
        };
        // A piece's height is its eaves (the plan takes a given height for a measured roof's mean, its eaves half its
        // rise below and its window rows from the mean): under a pitched roof, eaves over its whole storeys.
        if b.roof.rise() > 0.0 {
            b.height = f32::from(b.storeys) * b.storey_m + 0.4;
        }
        // A row's houses: its brick or render now and then, and its roof's colour under the same roof.
        if kind == Kind::Terrace && ph(82) < keeps_row(region) {
            b.wall = whole.wall;
            if b.roof.rise() > 0.0 && whole.roof.rise() > 0.0 {
                b.roof_colour = whole.roof_colour;
            }
        }
        out.push(b);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::Roof;

    fn setting() -> Setting {
        Setting {
            usage: Usage::Residential,
            ..Setting::default()
        }
    }

    fn rectangle(x: f32, z: f32, long: f32, short: f32) -> Vec<[f32; 2]> {
        vec![[x, z], [x + long, z], [x + long, z + short], [x, z + short]]
    }

    fn whole(footprint: &Footprint, region: Region, period: Period, density: f32) -> Building {
        plan_building(
            footprint,
            [3, 4],
            density,
            region,
            period,
            &setting(),
            &|_| 5.0,
        )
        .expect("the whole stands")
    }

    fn cut(footprint: &Footprint, region: Region, period: Period, density: f32) -> Vec<Building> {
        let b = whole(footprint, region, period, density);
        split(
            footprint,
            &b,
            [3, 4],
            density,
            region,
            period,
            &setting(),
            &|_| 5.0,
        )
    }

    fn total(buildings: &[Building]) -> f32 {
        buildings.iter().map(|b| geometry::area(&b.ring)).sum()
    }

    #[test]
    fn a_mediterranean_block_stands_as_its_houses() {
        // A block of a Borgo's grid, 72 by 20 m, drawn as one.
        let block = Footprint {
            ring: rectangle(100.0, 200.0, 72.0, 20.0),
            ..Footprint::default()
        };
        let b = whole(&block, Region::Mediterranean, Period::Today, 0.6);
        assert_eq!(b.kind, Kind::Block);
        let pieces = cut(&block, Region::Mediterranean, Period::Today, 0.6);
        assert!(
            (4..=8).contains(&pieces.len()),
            "{} pieces of a 72 m block",
            pieces.len()
        );
        let area = geometry::area(&block.ring);
        // Pitched pieces stand on their rectangles; flat ones on their rings: the block's ground either way.
        assert!((total(&pieces) - area).abs() < 0.03 * area);
        let storeys: Vec<u8> = pieces.iter().map(|p| p.storeys).collect();
        assert!(
            storeys.iter().any(|s| *s != storeys[0]),
            "a Mediterranean street's skyline is broken: {storeys:?}"
        );
        for p in &pieces {
            assert_eq!(p.kind, Kind::Block);
            assert!(
                p.storeys.abs_diff(b.storeys) <= 2,
                "{} by {}",
                p.storeys,
                b.storeys
            );
            for q in &p.ring {
                assert!(
                    (99.5..172.5).contains(&q[0]) && (199.5..220.5).contains(&q[1]),
                    "{q:?} within the block"
                );
            }
        }
        let seeds: std::collections::BTreeSet<u8> = pieces.iter().map(|p| p.seed).collect();
        assert!(seeds.len() > 2, "each its own seed");
    }

    #[test]
    fn a_british_row_stands_as_its_houses_under_their_own_roofs() {
        // A row of terraced houses drawn as one: 48 by 9 m.
        let row = Footprint {
            ring: rectangle(10.0, 10.0, 48.0, 9.0),
            ..Footprint::default()
        };
        // Built over a little: a terrace of two or three storeys, not a core's block.
        let b = whole(&row, Region::British, Period::Today, 0.4);
        assert_eq!(b.kind, Kind::Terrace);
        let pieces = cut(&row, Region::British, Period::Today, 0.4);
        assert!(
            (7..=11).contains(&pieces.len()),
            "{} houses in a 48 m row",
            pieces.len()
        );
        for p in &pieces {
            let rect = geometry::min_area_rect(&p.ring).unwrap();
            assert!(
                (4.0..=11.0).contains(&(2.0 * rect.half[1])),
                "a house's frontage: {:?}",
                rect.half
            );
            assert!(p.storeys.abs_diff(b.storeys) <= 1);
        }
        let pitched = pieces
            .iter()
            .filter(|p| matches!(p.roof, Roof::Gabled { .. } | Roof::Hipped { .. }))
            .count();
        assert!(pitched * 2 > pieces.len(), "the row's houses pitched");
        let kept = pieces.iter().filter(|p| p.wall == b.wall).count();
        assert!(kept * 2 >= pieces.len(), "a brick row mostly its brick");
    }

    #[test]
    fn a_deep_block_has_its_court_and_its_ends() {
        let block = Footprint {
            ring: rectangle(0.0, 0.0, 90.0, 60.0),
            ..Footprint::default()
        };
        let b = whole(&block, Region::Temperate, Period::Today, 0.6);
        assert_eq!(b.kind, Kind::Block);
        let pieces = cut(&block, Region::Temperate, Period::Today, 0.6);
        let area = geometry::area(&block.ring);
        assert!((total(&pieces) - area).abs() < 0.03 * area);
        let low: Vec<&Building> = pieces.iter().filter(|p| p.storeys == 1).collect();
        assert_eq!(low.len(), 1, "one court");
        let court = geometry::centroid(&low[0].ring);
        assert!((court[0] - 45.0).abs() < 8.0 && (court[1] - 30.0).abs() < 8.0);
        // The rows and the ends about it.
        assert!(pieces.len() >= 9, "{} pieces", pieces.len());
    }

    #[test]
    fn a_cut_never_parts_an_arm() {
        // A U: two 14 m arms off a 60 m back, their ends open to the street. A cut across the arms would cross the
        // outline four times; the pieces stand whole within the outline.
        let u = Footprint {
            ring: vec![
                [0.0, 0.0],
                [60.0, 0.0],
                [60.0, 40.0],
                [46.0, 40.0],
                [46.0, 14.0],
                [14.0, 14.0],
                [14.0, 40.0],
                [0.0, 40.0],
            ],
            ..Footprint::default()
        };
        let b = whole(&u, Region::Mediterranean, Period::Today, 0.6);
        let pieces = split(
            &u,
            &b,
            [0, 0],
            0.6,
            Region::Mediterranean,
            Period::Today,
            &setting(),
            &|_| 5.0,
        );
        let area = geometry::area(&u.ring);
        if !pieces.is_empty() {
            let flat: f32 = pieces
                .iter()
                .filter(|p| matches!(p.roof, Roof::Flat))
                .map(|p| geometry::area(&p.ring))
                .sum();
            assert!(flat <= area * 1.01);
            for p in &pieces {
                // Nothing over the court between the arms.
                assert!(
                    !geometry::contains(&p.ring, [30.0, 27.0]),
                    "a piece over the court: {:?}",
                    p.ring
                );
            }
        }
    }

    #[test]
    fn crossings_count_the_lines_edges() {
        let square = rectangle(0.0, 0.0, 10.0, 10.0);
        assert_eq!(crossings(&square, [5.0, 5.0], [0.0, 1.0]), 2);
        assert_eq!(crossings(&square, [20.0, 5.0], [0.0, 1.0]), 0);
    }

    #[test]
    fn what_stands_whole_stays_whole() {
        // A house, a short block, a church's land and industrial land.
        let house = Footprint {
            ring: rectangle(0.0, 0.0, 11.0, 8.0),
            ..Footprint::default()
        };
        assert!(cut(&house, Region::British, Period::Today, 0.6).is_empty());
        // One deep terraced house, as OSM maps them, its street at its narrow end.
        let deep = Footprint {
            ring: rectangle(0.0, 0.0, 15.0, 5.2),
            ..Footprint::default()
        };
        let b = whole(&deep, Region::British, Period::Today, 0.6);
        assert_eq!(b.kind, Kind::Terrace);
        assert!(cut(&deep, Region::British, Period::Today, 0.6).is_empty());
        // A pair of semis under one roof.
        let semis = Footprint {
            ring: rectangle(0.0, 0.0, 14.0, 9.0),
            ..Footprint::default()
        };
        assert!(cut(&semis, Region::British, Period::Today, 0.4).is_empty());
        let short = Footprint {
            ring: rectangle(0.0, 0.0, 16.0, 14.0),
            ..Footprint::default()
        };
        assert!(cut(&short, Region::Mediterranean, Period::Today, 0.6).is_empty());
        let block = Footprint {
            ring: rectangle(0.0, 0.0, 72.0, 20.0),
            ..Footprint::default()
        };
        let b = whole(&block, Region::Mediterranean, Period::Today, 0.6);
        for usage in [Usage::Industrial, Usage::Civic, Usage::Military] {
            let setting = Setting {
                usage,
                ..Setting::default()
            };
            assert!(
                split(
                    &block,
                    &b,
                    [3, 4],
                    0.6,
                    Region::Mediterranean,
                    Period::Today,
                    &setting,
                    &|_| 5.0
                )
                .is_empty()
            );
        }
        // The infill's lots are their own buildings already.
        let lot = Footprint {
            kind: Some(Kind::Block),
            height: Some(15.0),
            ..block.clone()
        };
        assert!(cut(&lot, Region::Mediterranean, Period::Today, 0.6).is_empty());
    }

    #[test]
    fn a_block_is_cut_alike_every_time() {
        let block = Footprint {
            ring: rectangle(310.0, 420.0, 66.0, 34.0),
            ..Footprint::default()
        };
        let a = cut(&block, Region::Aegean, Period::Year(1942), 0.6);
        let b = cut(&block, Region::Aegean, Period::Year(1942), 0.6);
        assert!(!a.is_empty());
        assert_eq!(a, b);
    }
}
