//! A whole building's roof edges by its region and kind (native/docs/terrain/buildings.md, "Eaves, parapets and
//! dormers"): how far a pitched roof's eaves hold out over its walls, how thick they stand, whether a gutter runs along
//! them and its colour; a flat roof's parapet; the dormers on a pitched house's front slope.
//!
//! Every choice hashes the building's place, so a chunk made again draws the same roofs.

use super::geometry;
use super::{Building, Kind, Region, Roof, cube_entry, hash01};

/// A pitched roof's eaves: the overhang past the walls (m, level all round a hipped roof), the roof's thickness at the
/// walls (m), whether a gutter runs along the eaves (its drop below the soffit, m), and the palette entries (walls')
/// of the fascia and gutter and of the soffit under them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Eaves {
    pub overhang: f32,
    pub thickness: f32,
    pub gutter: f32,
    pub fascia: u8,
    pub soffit: u8,
}

// Trim colours from the colour cube (sRGB, six levels a channel).
const WHITE: [u8; 3] = [255, 255, 255];
const CREAM: [u8; 3] = [255, 255, 204];
const BLACK: [u8; 3] = [51, 51, 51];
const BROWN: [u8; 3] = [102, 51, 0];
const TIMBER: [u8; 3] = [153, 102, 51];
const ZINC: [u8; 3] = [153, 153, 153];
const LIGHT_ZINC: [u8; 3] = [204, 204, 204];
const COPPER: [u8; 3] = [204, 102, 51];
const GREEN: [u8; 3] = [0, 102, 51];
const FALU: [u8; 3] = [153, 51, 0];

/// The building's own hash `k`, by where it stands in its chunk (decimetres).
pub(super) fn place_hash(b: &Building, k: i64) -> f32 {
    let c = geometry::centroid(&b.ring);
    hash01((c[0] * 10.0) as i64, (c[1] * 10.0) as i64, 7_000 + k)
}

fn choose<T: Copy>(options: &[(f32, T)], t: f32) -> T {
    let total: f32 = options.iter().map(|o| o.0).sum();
    let mut at = t * total;
    for (w, v) in options {
        if at < *w {
            return *v;
        }
        at -= w;
    }
    options[options.len() - 1].1
}

/// A pitched roof's eaves on `b` in `region`; none on a roof not pitched or a kind drawn by its parts alone.
pub fn eaves_of(b: &Building, region: Region) -> Option<Eaves> {
    if !matches!(b.roof, Roof::Gabled { .. } | Roof::Hipped { .. }) {
        return None;
    }
    use Region::*;
    let h = |k: i64| place_hash(b, k);
    let past = b.period.is_past();
    let (lo, spread) = match region {
        EastAsia if past => (0.65, 0.3),
        EastAsia | Taiwan => (0.45, 0.25),
        Tropical | Micronesia | Atoll | Hawaii => (0.5, 0.3),
        Florida => (0.45, 0.25),
        West => (0.45, 0.35),
        American => (0.3, 0.2),
        Nordic => (0.35, 0.25),
        Iceland => (0.25, 0.15),
        Temperate => (0.3, 0.25),
        British | Celtic => (0.12, 0.18),
        NorthSea => (0.25, 0.2),
        Mediterranean | Malta | Aegean | NorthAfrica | Arid | Nile => (0.2, 0.2),
    };
    let mut overhang = lo + spread * h(1);
    let mut thickness = 0.18;
    let mut gutter_share = match region {
        Mediterranean | Malta | Aegean | NorthAfrica | Arid | Nile => 0.3,
        Tropical | Micronesia | Atoll => 0.4,
        _ => 0.9,
    };
    match b.kind {
        Kind::House => {}
        // A row's eaves shallow (its gables are party walls: no verge).
        Kind::Terrace => overhang *= 0.8,
        Kind::Shed => {
            overhang *= 0.6;
            thickness = 0.14;
            gutter_share *= 0.35;
        }
        Kind::Block | Kind::Tower | Kind::Office => {
            overhang = overhang.max(0.35) + 0.15 * h(2);
            thickness = 0.25;
        }
        Kind::Industrial | Kind::Warehouse => {
            overhang = 0.3;
            thickness = 0.16;
            gutter_share = 0.7;
        }
        Kind::Church => {
            overhang = 0.3;
            thickness = 0.22;
            gutter_share = 0.6;
        }
        _ => return None,
    }
    let gutter = h(3) < gutter_share;
    let t = h(4);
    let entry = |rgb: [u8; 3]| cube_entry(rgb);
    let (fascia, soffit) = if gutter {
        let fascia = match region {
            British | Celtic => choose(&[(0.6, BLACK), (0.4, WHITE)], t),
            American | West | Florida | Hawaii => {
                choose(&[(0.7, WHITE), (0.15, CREAM), (0.15, BROWN)], t)
            }
            Nordic | Iceland => choose(&[(0.6, WHITE), (0.4, ZINC)], t),
            NorthSea => choose(&[(0.3, BLACK), (0.3, WHITE), (0.2, GREEN), (0.2, ZINC)], t),
            Temperate => choose(&[(0.5, ZINC), (0.2, COPPER), (0.3, WHITE)], t),
            EastAsia if past => choose(&[(0.5, COPPER), (0.5, BROWN)], t),
            EastAsia | Taiwan => choose(&[(0.5, LIGHT_ZINC), (0.3, BROWN), (0.2, WHITE)], t),
            _ => choose(&[(0.5, WHITE), (0.5, ZINC)], t),
        };
        let soffit = match region {
            EastAsia | Taiwan | Nordic => choose(&[(0.7, TIMBER), (0.3, BROWN)], h(5)),
            West if b.kind == Kind::House => choose(&[(0.5, TIMBER), (0.5, WHITE)], h(5)),
            _ => fascia,
        };
        (entry(fascia), entry(soffit))
    } else {
        // No gutter: the rafters' and the tiles' ends, dark timber, or the walls' own render under the eaves.
        let timber = entry(choose(&[(0.6, BROWN), (0.4, TIMBER)], t));
        let soffit = if h(5) < 0.5 { b.wall } else { timber };
        (timber, soffit)
    };
    let fascia = if region == Nordic && h(6) < 0.25 {
        entry(FALU)
    } else {
        fascia
    };
    Some(Eaves {
        overhang,
        thickness,
        gutter: if gutter { 0.06 } else { 0.0 },
        fascia,
        soffit,
    })
}

/// A flat roof's parapet over the eaves (m): most blocks and offices, many terraces, a warm, dry country's houses;
/// none on a shed.
pub fn parapet_of(b: &Building, region: Region) -> f32 {
    if b.roof != Roof::Flat {
        return 0.0;
    }
    use Region::*;
    let h = |k: i64| place_hash(b, k);
    let (share, lo, spread) = match b.kind {
        Kind::Block | Kind::Tower | Kind::Office => (0.85, 0.6, 0.5),
        Kind::Terrace => (0.7, 0.5, 0.4),
        Kind::House => match region {
            Mediterranean | Malta | Aegean | NorthAfrica | Arid | Nile => (0.8, 0.4, 0.5),
            West | Florida | Hawaii => (0.4, 0.3, 0.3),
            _ => (0.3, 0.3, 0.2),
        },
        Kind::Industrial | Kind::Warehouse => (0.4, 0.3, 0.3),
        _ => return 0.0,
    };
    if h(11) < share {
        lo + spread * h(12)
    } else {
        0.0
    }
}

/// How many dormers stand on a pitched house's or terrace's front slope: where attics were lived in (Britain, Scotland
/// and Ireland, the North Sea's and Europe's houses, Scandinavia and Iceland, America's Cape Cods), on a roof steep and
/// deep enough to stand one under its ridge; none elsewhere.
pub fn dormers_of(b: &Building, region: Region, depth: f32, length: f32, rise: f32) -> u32 {
    if !matches!(b.kind, Kind::House | Kind::Terrace) || b.storeys > 2 {
        return 0;
    }
    use Region::*;
    let share = match (region, b.kind) {
        (British, Kind::House) => 0.25,
        (British, _) => 0.1,
        (Celtic, Kind::House) => 0.35,
        (Celtic, _) => 0.2,
        (NorthSea, _) => 0.32,
        (Temperate, Kind::House) => 0.3,
        (Temperate, _) => 0.25,
        (Nordic, _) => 0.2,
        (Iceland, _) => 0.15,
        (American, Kind::House) => 0.3,
        (American, _) => 0.15,
        _ => 0.0,
    };
    // Deep enough (a dormer's ridge under the roof's), steep enough (30 degrees) and long enough for one.
    if depth < 3.0 || rise < 2.4 || rise < depth * 30f32.to_radians().tan() || length < 4.0 {
        return 0;
    }
    if place_hash(b, 21) >= share {
        return 0;
    }
    let room = ((length - 1.6) / 2.8).floor().max(1.0) as u32;
    let wanted = 1 + (place_hash(b, 22) * 2.99) as u32;
    wanted.min(room).min(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::Period;

    fn house(roof: Roof, kind: Kind, at: [f32; 2]) -> Building {
        Building {
            ring: vec![
                [at[0], at[1]],
                [at[0] + 10.0, at[1]],
                [at[0] + 10.0, at[1] + 8.0],
                [at[0], at[1] + 8.0],
            ],
            kind,
            roof,
            ground: [1.0, 1.0, 1.0],
            height: 5.6,
            base: 0.0,
            storeys: 2,
            storey_m: 2.8,
            wall: 4,
            roof_colour: 2,
            seed: 9,
            detail: false,
            bay: 3.0,
            extras: Vec::new(),
            period: Period::Today,
        }
    }

    #[test]
    fn eaves_hold_out_by_region_and_parapets_crown_flat_blocks() {
        let mean = |region: Region| {
            let all: Vec<f32> = (0..200)
                .filter_map(|k| {
                    eaves_of(
                        &house(
                            Roof::Gabled { rise: 3.0 },
                            Kind::House,
                            [k as f32 * 13.0, 0.0],
                        ),
                        region,
                    )
                })
                .map(|e| e.overhang)
                .collect();
            all.iter().sum::<f32>() / all.len() as f32
        };
        // Japan's and California's eaves deep, Britain's tight.
        assert!(mean(Region::EastAsia) > 0.5 && mean(Region::West) > 0.55);
        assert!(mean(Region::British) < 0.25);
        // Flat roofs have none; a flat block mostly a parapet, a shed none.
        assert!(eaves_of(&house(Roof::Flat, Kind::House, [0.0, 0.0]), Region::British).is_none());
        let crowned = (0..200)
            .filter(|k| {
                parapet_of(
                    &house(Roof::Flat, Kind::Block, [*k as f32 * 13.0, 0.0]),
                    Region::Temperate,
                ) > 0.0
            })
            .count();
        assert!((150..=190).contains(&crowned), "{crowned}");
        assert_eq!(
            parapet_of(&house(Roof::Flat, Kind::Shed, [0.0, 0.0]), Region::Malta),
            0.0
        );
        // The same building, the same eaves.
        let b = house(Roof::Hipped { rise: 3.0 }, Kind::House, [3.0, 4.0]);
        assert_eq!(eaves_of(&b, Region::Nordic), eaves_of(&b, Region::Nordic));
    }

    #[test]
    fn dormers_stand_on_steep_deep_roofs_where_attics_were_lived_in() {
        let count = |region: Region, rise: f32| {
            (0..400)
                .map(|k| {
                    dormers_of(
                        &house(Roof::Gabled { rise }, Kind::House, [k as f32 * 13.0, 0.0]),
                        region,
                        4.0,
                        10.0,
                        rise,
                    )
                })
                .filter(|n| *n > 0)
                .count()
        };
        assert!((70..=170).contains(&count(Region::Celtic, 3.2)));
        assert_eq!(count(Region::Mediterranean, 3.2), 0);
        assert_eq!(count(Region::EastAsia, 3.2), 0);
        // A low-pitched roof holds none.
        assert_eq!(count(Region::Celtic, 1.5), 0);
        // At most three, and no more than its length holds.
        let b = house(Roof::Gabled { rise: 3.2 }, Kind::House, [0.0, 0.0]);
        for k in 0..50 {
            let mut b = b.clone();
            b.ring.iter_mut().for_each(|p| p[0] += k as f32 * 11.0);
            assert!(dormers_of(&b, Region::NorthSea, 4.0, 4.5, 3.2) <= 1);
            assert!(dormers_of(&b, Region::NorthSea, 4.0, 30.0, 3.2) <= 3);
        }
    }
}
