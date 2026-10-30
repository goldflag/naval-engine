//! Generic buildings where the footprints have none: lots on a street grid over built-up land (ESA WorldCover's class
//! 50 in the window's maps) that no footprint of the source stands on, as flight simulators fill towns their data
//! misses.
//!
//! - **The grid.** The theatre is cut into districts of [`DISTRICT_M`], each with its own street grid at its own
//!   heading (a hash of the district), and each street grid into blocks of [`BLOCK_M`] between streets
//!   [`STREET_M`] wide. A block holds two rows of lots, back to back, facing the streets on its long sides. Lots
//!   whose middle falls outside their district are dropped, so districts meet along a gap, as along a main road.
//! - **Density.** The share of the cover built-up about a lot (a few cover texels each way) picks what stands on it:
//!   houses of one or two storeys on 14 m lots where the town thins out, terraces and small blocks where it thickens,
//!   blocks of four to seven storeys filling their lots in a dense core.
//! - **Where.** Only on land (heights above 1 m), not on slopes steeper than [`MAX_SLOPE`], and never in a 100 m cell
//!   of the chunk holding any footprint of the source (`occupied`): where the source has a town, the town is its.
//! - **Deterministic**: every choice is a hash of the lot's place in the theatre, so the same lot gets the same building
//!   whichever window or chunk asks.

use super::{CHUNK_M, Footprint, Kind, Region, geometry, hash01};

/// A district's side (m): one street grid's heading.
pub const DISTRICT_M: f64 = 420.0;
/// A block between streets (m), along and across.
pub const BLOCK_M: [f64; 2] = [96.0, 52.0];
/// A street's width (m).
pub const STREET_M: f64 = 12.0;

/// A region's street grid and lots: its blocks (m, along and across) and streets, and its houses' lots (the least
/// width and the spread above it, m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grid {
    pub block: [f64; 2],
    pub street: f64,
    pub lot: [f64; 2],
}

impl Grid {
    pub fn of(region: Region) -> Self {
        match region {
            // Long blocks of 15 m lots 38 m deep (Los Angeles' 50 by 125 ft) between wide streets.
            Region::West => Self {
                block: [150.0, 76.0],
                street: 14.0,
                lot: [14.0, 4.0],
            },
            // Small blocks of narrow lots between lanes.
            Region::EastAsia => Self {
                block: [64.0, 34.0],
                street: 6.0,
                lot: [9.0, 3.0],
            },
            // Taiwan's blocks between narrow lanes (巷, 弄): rows of townhouses on lots 4-6 m wide and 18 m deep.
            Region::Taiwan => Self {
                block: [72.0, 36.0],
                street: 6.0,
                lot: [4.0, 2.0],
            },
            // Florida's subdivisions: long blocks of 23 m (75 ft) lots.
            Region::Florida => Self {
                block: [160.0, 80.0],
                street: 14.0,
                lot: [18.0, 6.0],
            },
            // Honolulu's blocks of 15 m (50 ft) lots 30 m deep.
            Region::Hawaii => Self {
                block: [120.0, 62.0],
                street: 12.0,
                lot: [14.0, 4.0],
            },
            // A British or Dutch town's streets of terraces, 5-6 m to a house, gardens back to back.
            Region::British | Region::Celtic | Region::NorthSea => Self {
                block: [110.0, 58.0],
                street: 10.0,
                lot: [8.0, 4.0],
            },
            _ => Self {
                block: BLOCK_M,
                street: STREET_M,
                lot: [12.0, 5.0],
            },
        }
    }
}
/// The steepest ground a lot is built on (rise over run across it).
pub const MAX_SLOPE: f32 = 0.22;
/// The cover's built-up class (ESA WorldCover).
pub const BUILT_UP: u8 = 50;

/// What the infill reads of the land: the cover's class and the share built-up about a theatre point, and the ground.
pub trait Land {
    /// The cover's class at theatre point `p` (0 unknown).
    fn cover(&self, p: [f64; 2]) -> u8;
    /// The share of the cover built-up within about `radius` metres of `p`, 0-1.
    fn built_share(&self, p: [f64; 2], radius: f64) -> f32;
    /// The ground at theatre point `p`, metres above the sea.
    fn height(&self, p: [f64; 2]) -> f32;
}

/// Which of a chunk's 100 m cells hold a footprint of the source (chunk-local rings).
pub fn occupied(footprints: &[Footprint]) -> [bool; 100] {
    let mut cells = [false; 100];
    for f in footprints {
        for p in &f.ring {
            let (i, j) = ((p[0] / 100.0).floor(), (p[1] / 100.0).floor());
            if (0.0..10.0).contains(&i) && (0.0..10.0).contains(&j) {
                cells[j as usize * 10 + i as usize] = true;
            }
        }
    }
    cells
}

/// A lot's building: its footprint (theatre metres, counter-clockwise in (x, z)), height and kind.
struct Lot {
    centre: [f64; 2],
    axis: [f64; 2],
    half: [f64; 2],
    height: f32,
    kind: Kind,
}

/// The infill's buildings in chunk `index` (chunk-local footprints with their heights and kinds), leaving the 100 m
/// cells `occupied` marks; `region` the region at a theatre point (x, z m), asked of each district's middle so a
/// district's streets and lots are the same from either chunk it spans.
pub fn lots(
    index: [i32; 2],
    land: &dyn Land,
    occupied: &[bool; 100],
    region: &dyn Fn([f64; 2]) -> Region,
) -> Vec<Footprint> {
    let corner = [f64::from(index[0]) * CHUNK_M, f64::from(index[1]) * CHUNK_M];
    // A quick look: no built-up cover anywhere in the chunk, nothing to do.
    let mut any = false;
    for j in 0..10 {
        for i in 0..10 {
            if occupied[j * 10 + i] {
                continue;
            }
            let p = [
                corner[0] + (f64::from(i as u8) + 0.5) * 100.0,
                corner[1] + (f64::from(j as u8) + 0.5) * 100.0,
            ];
            if land.built_share(p, 70.0) > 0.05 {
                any = true;
            }
        }
    }
    if !any {
        return Vec::new();
    }
    let mut out = Vec::new();
    // The districts overlapping the chunk.
    let d0 = [
        (corner[0] / DISTRICT_M).floor() as i64,
        (corner[1] / DISTRICT_M).floor() as i64,
    ];
    let d1 = [
        ((corner[0] + CHUNK_M) / DISTRICT_M).floor() as i64,
        ((corner[1] + CHUNK_M) / DISTRICT_M).floor() as i64,
    ];
    for dz in d0[1]..=d1[1] {
        for dx in d0[0]..=d1[0] {
            district(dx, dz, corner, land, occupied, region, &mut out);
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn district(
    dx: i64,
    dz: i64,
    corner: [f64; 2],
    land: &dyn Land,
    occupied: &[bool; 100],
    region: &dyn Fn([f64; 2]) -> Region,
    out: &mut Vec<Footprint>,
) {
    let lo = [dx as f64 * DISTRICT_M, dz as f64 * DISTRICT_M];
    let centre = [lo[0] + DISTRICT_M * 0.5, lo[1] + DISTRICT_M * 0.5];
    let region = region(centre);
    let heading = f64::from(hash01(dx, dz, 101)) * std::f64::consts::FRAC_PI_2;
    let (s, c) = heading.sin_cos();
    let u = [c, s];
    let v = [-s, c];
    // The grid's offset within the district, so neighbouring districts' streets do not line up.
    let grid = Grid::of(region);
    let shift = [
        f64::from(hash01(dx, dz, 102)) * grid.block[0],
        f64::from(hash01(dx, dz, 103)) * grid.block[1],
    ];
    let pitch = [grid.block[0] + grid.street, grid.block[1] + grid.street];
    // Blocks reaching the district's corners (its half-diagonal).
    let reach = DISTRICT_M * std::f64::consts::FRAC_1_SQRT_2;
    let (na, nb) = (
        (reach / pitch[0]).ceil() as i64 + 1,
        (reach / pitch[1]).ceil() as i64 + 1,
    );
    for bj in -nb..=nb {
        for bi in -na..=na {
            // The block's middle in the district's street frame.
            let ba = bi as f64 * pitch[0] + shift[0];
            let bb = bj as f64 * pitch[1] + shift[1];
            let block_hash = |k: i64| hash01(dx * 131 + bi, dz * 137 + bj, k);
            let mid = [
                centre[0] + u[0] * ba + v[0] * bb,
                centre[1] + u[1] * ba + v[1] * bb,
            ];
            if (mid[0] - centre[0]).abs() > DISTRICT_M * 0.5 + pitch[0]
                || (mid[1] - centre[1]).abs() > DISTRICT_M * 0.5 + pitch[0]
            {
                continue;
            }
            let share = land.built_share(mid, 90.0);
            if share < 0.12 {
                continue;
            }
            // Dense cores fill their blocks with fewer, larger buildings: in a European or Nordic town's middle, an
            // East Asian centre's mid-rise blocks, and only now and then in the tropics, a dry country or America,
            // whose towns are mostly low (the American West's suburbs houses almost to the end).
            let dense = match region {
                Region::Tropical
                | Region::Arid
                | Region::NorthAfrica
                | Region::Micronesia
                | Region::Atoll => share > 0.9 && block_hash(1) < 0.25,
                Region::American | Region::Florida | Region::Hawaii => {
                    share > 0.85 && block_hash(1) < 0.35
                }
                Region::West => share > 0.92 && block_hash(1) < 0.12,
                Region::EastAsia => share > 0.8 && block_hash(1) < (share - 0.6) * 1.5,
                // Walk-ups (公寓) among the townhouses where the ground is thickest.
                Region::Taiwan => share > 0.85 && block_hash(1) < (share - 0.7) * 1.2,
                Region::Nordic
                | Region::Temperate
                | Region::Mediterranean
                | Region::Iceland
                | Region::British
                | Region::Celtic
                | Region::NorthSea
                | Region::Malta
                | Region::Aegean
                | Region::Nile => share > 0.75 && block_hash(1) < (share - 0.6) * 1.6,
            };
            let lot_w = if dense {
                18.0 + 10.0 * f64::from(block_hash(2))
            } else {
                grid.lot[0] + grid.lot[1] * f64::from(block_hash(2))
            };
            let depth = grid.block[1] * 0.5;
            let per_row = (grid.block[0] / lot_w).floor().max(1.0) as i64;
            let lot_w = grid.block[0] / per_row as f64;
            for row in [-1.0f64, 1.0] {
                for k in 0..per_row {
                    let la = -grid.block[0] * 0.5 + (k as f64 + 0.5) * lot_w;
                    let lb = row * depth * 0.5;
                    let at = [
                        mid[0] + u[0] * la + v[0] * lb,
                        mid[1] + u[1] * la + v[1] * lb,
                    ];
                    // Each lot once: in its own chunk (and, below, wholly in its own district).
                    if !(corner[0]..corner[0] + CHUNK_M).contains(&at[0])
                        || !(corner[1]..corner[1] + CHUNK_M).contains(&at[1])
                    {
                        continue;
                    }
                    let (ci, cj) = (
                        ((at[0] - corner[0]) / 100.0) as usize,
                        ((at[1] - corner[1]) / 100.0) as usize,
                    );
                    if occupied[cj.min(9) * 10 + ci.min(9)] {
                        continue;
                    }
                    let lot_hash = |n: i64| {
                        hash01(
                            (at[0] * 2.0).round() as i64,
                            (at[1] * 2.0).round() as i64,
                            n,
                        )
                    };
                    if land.cover(at) != BUILT_UP {
                        continue;
                    }
                    // Thinner where the town thins: an empty lot now and then.
                    let local = land.built_share(at, 40.0);
                    if lot_hash(1) > 0.35 + 0.6 * local {
                        continue;
                    }
                    if let Some(lot) = lot_building(
                        at,
                        [u, v],
                        row,
                        [lot_w, depth],
                        dense,
                        local,
                        &lot_hash,
                        region,
                    ) && corners(&lot).iter().all(|p| {
                        (lo[0]..lo[0] + DISTRICT_M).contains(&p[0])
                            && (lo[1]..lo[1] + DISTRICT_M).contains(&p[1])
                    }) && buildable(&lot, land)
                    {
                        out.push(footprint_of(&lot, corner));
                    }
                }
            }
        }
    }
}

/// The building on a lot of `size` (along the street, depth) at `at`, facing the street on side `row` of its block.
#[allow(clippy::too_many_arguments)]
fn lot_building(
    at: [f64; 2],
    frame: [[f64; 2]; 2],
    row: f64,
    size: [f64; 2],
    dense: bool,
    local: f32,
    hash: &dyn Fn(i64) -> f32,
    region: Region,
) -> Option<Lot> {
    let [u, v] = frame;
    if region == Region::Taiwan {
        return Some(taiwanese(at, frame, row, size, dense, local, hash));
    }
    // Terraces where the town thickens: in the American West a row of townhouses only where it is thick, in East Asia
    // a small apartment house.
    let terrace = match region {
        Region::West => local > 0.8 && hash(5) < 0.2,
        Region::EastAsia => local > 0.6 && hash(5) < 0.3,
        // Terraced streets through a British, Irish or Dutch town, semis and detached houses only at its edge.
        Region::British | Region::NorthSea => local > 0.3 && hash(5) < 0.85,
        Region::Celtic => local > 0.4 && hash(5) < 0.7,
        Region::Florida => local > 0.85 && hash(5) < 0.15,
        _ => local > 0.55 && hash(5) < 0.5,
    };
    let (kind, along, deep, storeys) = if dense {
        let storeys = match region {
            Region::Tropical
            | Region::Arid
            | Region::American
            | Region::West
            | Region::Florida
            | Region::Iceland
            | Region::NorthAfrica
            | Region::Hawaii
            | Region::Micronesia
            | Region::Atoll => 2.0 + (hash(3) * 1.99).floor(),
            // Three to five: a high street's shops and flats, a tenement, a Dutch canal house, Valletta's and
            // Sliema's houses.
            Region::British | Region::Celtic | Region::NorthSea | Region::Malta => {
                3.0 + (hash(3) * 2.99).floor()
            }
            // Four to seven: Athens' polykatoikia, Cairo's and Alexandria's blocks.
            Region::Aegean | Region::Nile => 4.0 + (hash(3) * 3.99).floor(),
            // (Taiwan's lots are its own: `taiwanese`.)
            Region::Nordic
            | Region::Temperate
            | Region::Mediterranean
            | Region::EastAsia
            | Region::Taiwan => 3.0 + (hash(3) * 3.99).floor(),
        };
        // East Asia's mid-rise blocks stand apart.
        let fill = if region == Region::EastAsia {
            0.82
        } else {
            0.92
        };
        (
            Kind::Block,
            size[0] * fill,
            size[1] * (0.6 + 0.3 * f64::from(hash(4))),
            storeys,
        )
    } else if terrace {
        // Two storeys, three now and then (a British terrace's two the most of all).
        let storeys = match region {
            Region::British | Region::Celtic | Region::NorthSea => {
                if hash(3) < 0.85 {
                    2.0
                } else {
                    3.0
                }
            }
            _ => 2.0 + (hash(3) * 1.99).floor(),
        };
        (
            Kind::Terrace,
            size[0] * 0.95,
            size[1] * (0.4 + 0.2 * f64::from(hash(4))),
            storeys,
        )
    } else {
        match region {
            // Ranch houses, wide and low (two storeys on a third, on a fifth in Florida), deep in their lots.
            Region::West | Region::Florida => {
                let one = if region == Region::Florida { 0.8 } else { 0.65 };
                let storeys = if hash(3) < one { 1.0 } else { 2.0 };
                let along = (size[0] * (0.62 + 0.22 * f64::from(hash(6)))).clamp(9.0, 17.0);
                let deep = (size[1] * (0.3 + 0.15 * f64::from(hash(4)))).clamp(9.0, 15.0);
                (Kind::House, along, deep, storeys)
            }
            // Small two-storey houses filling narrow lots, a metre or two apart.
            Region::EastAsia => {
                let storeys = if hash(3) < 0.15 { 1.0 } else { 2.0 };
                let along = (size[0] * (0.72 + 0.16 * f64::from(hash(6)))).clamp(6.0, 10.5);
                let deep = (size[1] * (0.45 + 0.25 * f64::from(hash(4)))).clamp(6.5, 10.5);
                (Kind::House, along, deep, storeys)
            }
            _ => {
                // Two storeys to a British, Dutch or Danish house; a cottage of one in Scotland or Ireland now and
                // then, half the houses elsewhere.
                let low = match region {
                    Region::British => 0.12,
                    Region::NorthSea => 0.3,
                    Region::Celtic => 0.4,
                    Region::Hawaii => 0.62,
                    _ => 0.5,
                };
                let one = matches!(
                    region,
                    Region::Tropical
                        | Region::Arid
                        | Region::NorthAfrica
                        | Region::Micronesia
                        | Region::Atoll
                ) || hash(3) < low;
                let storeys = if one { 1.0 } else { 2.0 };
                let along = (size[0] * (0.55 + 0.25 * f64::from(hash(6)))).clamp(6.0, 13.0);
                let deep = (size[1] * (0.3 + 0.2 * f64::from(hash(4)))).clamp(6.0, 12.0);
                (Kind::House, along, deep, storeys)
            }
        }
    };
    // Set back from the street: the building toward the lot's street side (the American West's front yards deep, East
    // Asia's houses at the lane).
    let setback = match (kind, region) {
        (Kind::House, Region::West | Region::Florida) => 6.0 + 3.0 * f64::from(hash(7)),
        (Kind::House, Region::EastAsia) => 1.0 + 1.0 * f64::from(hash(7)),
        (Kind::House, _) => 3.0 + 3.0 * f64::from(hash(7)),
        _ => 1.0,
    };
    let toward = size[1] * 0.5 - setback - deep * 0.5;
    let centre = [at[0] + v[0] * row * toward, at[1] + v[1] * row * toward];
    // A house turned a little on its lot now and then.
    let turn = if kind == Kind::House {
        (f64::from(hash(8)) - 0.5) * 0.12
    } else {
        0.0
    };
    let (s, c) = turn.sin_cos();
    let axis = [u[0] * c - u[1] * s, u[0] * s + u[1] * c];
    let storey_m = super::storey_height(kind);
    Some(Lot {
        centre,
        axis,
        half: [along * 0.5, deep * 0.5],
        height: storeys * storey_m + 0.4,
        kind,
    })
}

/// Taiwan's lot: a townhouse (透天厝, 街屋) filling its narrow lot's width at the lane, deep in it, flat-roofed, of three
/// to five storeys (four and five where the town is built over); in a dense core a walk-up (公寓) of four or five
/// across its wider lot. No detached houses.
fn taiwanese(
    at: [f64; 2],
    frame: [[f64; 2]; 2],
    row: f64,
    size: [f64; 2],
    dense: bool,
    local: f32,
    hash: &dyn Fn(i64) -> f32,
) -> Lot {
    let [u, v] = frame;
    let (kind, fill, deep, storeys) = if dense {
        (
            Kind::Block,
            0.97,
            size[1] * (0.75 + 0.2 * f64::from(hash(4))),
            4.0 + (hash(3) * 1.99).floor(),
        )
    } else {
        let storeys = if local > 0.6 {
            4.0 + (hash(3) * 1.99).floor()
        } else {
            3.0 + (hash(3) * 1.99).floor()
        };
        (
            Kind::Terrace,
            0.97,
            size[1] * (0.65 + 0.3 * f64::from(hash(4))),
            storeys,
        )
    };
    // At the lane: half a metre back at most.
    let setback = 0.5 * f64::from(hash(7));
    let toward = size[1] * 0.5 - setback - deep * 0.5;
    Lot {
        centre: [at[0] + v[0] * row * toward, at[1] + v[1] * row * toward],
        axis: u,
        half: [size[0] * fill * 0.5, deep * 0.5],
        height: storeys * super::storey_height(kind) + 0.4,
        kind,
    }
}

/// Whether a lot's building stands on dry, gentle land.
fn buildable(lot: &Lot, land: &dyn Land) -> bool {
    let corners = corners(lot);
    let mut lo = f32::INFINITY;
    let mut hi = f32::NEG_INFINITY;
    for p in corners.iter().chain(std::iter::once(&lot.centre)) {
        let h = land.height(*p);
        lo = lo.min(h);
        hi = hi.max(h);
    }
    let span = (2.0 * lot.half[0].max(lot.half[1])) as f32;
    lo > 1.0 && (hi - lo) / span <= MAX_SLOPE
}

fn corners(lot: &Lot) -> [[f64; 2]; 4] {
    let a = lot.axis;
    let n = [-a[1], a[0]];
    let [l, s] = lot.half;
    let c = lot.centre;
    let at = |x: f64, y: f64| [c[0] + a[0] * x + n[0] * y, c[1] + a[1] * x + n[1] * y];
    [at(-l, -s), at(l, -s), at(l, s), at(-l, s)]
}

fn footprint_of(lot: &Lot, corner: [f64; 2]) -> Footprint {
    let ring: Vec<[f32; 2]> = corners(lot)
        .iter()
        .map(|p| [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32])
        .collect();
    debug_assert!(geometry::signed_area(&ring) > 0.0);
    Footprint {
        ring,
        height: Some(lot.height),
        kind: Some(lot.kind),
        colour: None,
        part: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::storey_height;

    /// A town: built-up within 1.5 km of the theatre's (500, 500), denser toward its middle; flat land 10 m up, the sea
    /// west of x = -400.
    struct Town;

    impl Land for Town {
        fn cover(&self, p: [f64; 2]) -> u8 {
            if (p[0] - 500.0).hypot(p[1] - 500.0) < 1500.0 {
                BUILT_UP
            } else {
                30
            }
        }
        fn built_share(&self, p: [f64; 2], _: f64) -> f32 {
            let d = (p[0] - 500.0).hypot(p[1] - 500.0);
            (1.0 - d / 1500.0).clamp(0.0, 1.0) as f32 * 1.2
        }
        fn height(&self, p: [f64; 2]) -> f32 {
            if p[0] < -400.0 { -5.0 } else { 10.0 }
        }
    }

    #[test]
    fn lots_are_deterministic_and_keep_off_the_sea_and_the_sources_towns() {
        let free = [false; 100];
        let a = lots([0, 0], &Town, &free, &|_| Region::Temperate);
        let b = lots([0, 0], &Town, &free, &|_| Region::Temperate);
        assert!(a.len() > 300, "{} lots", a.len());
        assert_eq!(a, b);
        // Every footprint inside its chunk, counter-clockwise, with a height and a kind; denser kinds in the middle.
        for f in &a {
            let c = geometry::centroid(&f.ring);
            assert!((0.0..1000.0).contains(&c[0]) && (0.0..1000.0).contains(&c[1]));
            assert!(geometry::signed_area(&f.ring) > 20.0);
            assert!(f.height.unwrap() >= 2.5 && f.kind.is_some());
        }
        assert!(a.iter().any(|f| f.kind == Some(Kind::Block)));
        assert!(a.iter().any(|f| f.kind == Some(Kind::House)));
        // No two overlap: their middles stay apart by at least a street's half.
        for (i, f) in a.iter().enumerate() {
            let c = geometry::centroid(&f.ring);
            for g in &a[i + 1..] {
                assert!(!geometry::contains(&g.ring, c));
            }
        }
        // A chunk at sea: none. A cell the source holds: none there.
        assert!(lots([-2, 0], &Town, &free, &|_| Region::Temperate).is_empty());
        let mut held = [false; 100];
        held[5 * 10 + 5] = true;
        let c = lots([0, 0], &Town, &held, &|_| Region::Temperate);
        assert!(c.iter().all(|f| {
            let m = geometry::centroid(&f.ring);
            !((500.0..600.0).contains(&m[0]) && (500.0..600.0).contains(&m[1]))
        }));
        assert!(c.len() < a.len());
        // The same lots from a neighbouring chunk's view: chunk (1, 0) holds lots near x = 1000 that chunk (0, 0) does
        // not, never the same.
        let d = lots([1, 0], &Town, &free, &|_| Region::Temperate);
        assert!(!d.is_empty());
    }

    #[test]
    fn suburbs_follow_the_region() {
        let free = [false; 100];
        let houses = |region: Region| -> Vec<Footprint> {
            lots([0, 0], &Town, &free, &|_| region)
                .into_iter()
                .filter(|f| f.kind == Some(Kind::House))
                .collect()
        };
        let mean = |fs: &[Footprint], f: &dyn Fn(&Footprint) -> f32| {
            fs.iter().map(f).sum::<f32>() / fs.len().max(1) as f32
        };
        let area = |f: &Footprint| geometry::signed_area(&f.ring);
        let one_storey = |fs: &[Footprint]| {
            fs.iter().filter(|f| f.height.unwrap() < 4.5).count() as f32 / fs.len() as f32
        };
        let temperate = houses(Region::Temperate);
        let west = houses(Region::West);
        let east = houses(Region::EastAsia);
        assert!(west.len() > 100 && east.len() > 100 && temperate.len() > 100);
        // The American West's ranch houses: bigger and mostly of one storey, on wide lots.
        assert!(mean(&west, &area) > mean(&temperate, &area) * 1.3);
        assert!(one_storey(&west) > 0.5, "{}", one_storey(&west));
        // East Asia's: small, of two storeys, close together (more of them on the same ground).
        assert!(mean(&east, &area) < mean(&temperate, &area));
        assert!(one_storey(&east) < 0.3, "{}", one_storey(&east));
        assert!(east.len() as f32 > temperate.len() as f32 * 1.3);
        // East Asia's town middle: mid-rise blocks of three storeys and more.
        let blocks = lots([0, 0], &Town, &free, &|_| Region::EastAsia)
            .into_iter()
            .filter(|f| f.kind == Some(Kind::Block))
            .count();
        assert!(blocks > 10, "{blocks} blocks");
    }

    #[test]
    fn taiwans_lots_are_rows_of_narrow_deep_townhouses() {
        let free = [false; 100];
        let all = lots([0, 0], &Town, &free, &|_| Region::Taiwan);
        let east = lots([0, 0], &Town, &free, &|_| Region::EastAsia);
        assert!(
            all.len() as f32 > east.len() as f32 * 1.5,
            "{} {}",
            all.len(),
            east.len()
        );
        // No detached houses: townhouses, and a core's walk-ups.
        assert!(
            all.iter()
                .all(|f| matches!(f.kind, Some(Kind::Terrace | Kind::Block)))
        );
        let rows: Vec<&Footprint> = all
            .iter()
            .filter(|f| f.kind == Some(Kind::Terrace))
            .collect();
        let walk_ups = all.len() - rows.len();
        assert!(
            rows.len() > 1000 && walk_ups > 10,
            "{} rows, {walk_ups} walk-ups",
            rows.len()
        );
        // Each 4-6 m wide at the lane and two to four times as deep; three to five storeys, four and five the most.
        let storeys = |f: &Footprint| {
            ((f.height.unwrap() - 0.4) / storey_height(Kind::Terrace)).round() as u8
        };
        let mut tall = 0;
        for f in &rows {
            let r = geometry::min_area_rect(&f.ring).unwrap();
            let (wide, deep) = (
                2.0 * r.half[0].min(r.half[1]),
                2.0 * r.half[0].max(r.half[1]),
            );
            assert!((3.8..6.2).contains(&wide), "{wide}");
            assert!((11.0..18.0).contains(&deep), "{deep}");
            let s = storeys(f);
            assert!((3..=5).contains(&s), "{s}");
            tall += usize::from(s >= 4);
        }
        assert!(tall > rows.len() / 2, "{tall} of {}", rows.len());
        // A row's houses stand side by side: most have a neighbour within a metre of their width.
        let centre = |f: &Footprint| geometry::centroid(&f.ring);
        let close = rows
            .iter()
            .take(300)
            .filter(|f| {
                let c = centre(f);
                rows.iter().any(|g| {
                    let d = geometry::distance(c, centre(g));
                    d > 1.0 && d < 7.0
                })
            })
            .count();
        assert!(close > 250, "{close}");
    }

    #[test]
    fn districts_across_a_regions_edge_are_one_layout() {
        // The American West north of z = 1000, the Mediterranean south of it: the chunks either side of the line agree
        // on the districts that cross it, so no two buildings overlap.
        let free = [false; 100];
        let region = |p: [f64; 2]| {
            if p[1] < 1000.0 {
                Region::West
            } else {
                Region::Mediterranean
            }
        };
        let theatre = |index: [i32; 2]| -> Vec<Vec<[f32; 2]>> {
            lots(index, &Town, &free, &region)
                .into_iter()
                .map(|f| {
                    f.ring
                        .iter()
                        .map(|p| {
                            [
                                p[0] + 1000.0 * index[0] as f32,
                                p[1] + 1000.0 * index[1] as f32,
                            ]
                        })
                        .collect()
                })
                .collect()
        };
        let (a, b) = (theatre([0, 0]), theatre([0, 1]));
        assert!(!a.is_empty() && !b.is_empty());
        for f in &a {
            for g in &b {
                for p in g {
                    assert!(!geometry::contains(f, *p), "overlapping buildings at {p:?}");
                }
            }
        }
    }
}
