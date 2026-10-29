use super::*;
use naval_osm::{Brunnel, Features, Landuse, TileId, Transportation};

use crate::features::page_of;
use crate::osm;

const MONTEREY_BAY: LatLon = LatLon {
    lat: 36.936204,
    lon: -122.418494,
};

fn way(class: &str, line: Vec<[f64; 2]>) -> Transportation<[f64; 2]> {
    Transportation {
        class: class.into(),
        subclass: None,
        brunnel: Brunnel::None,
        surface: None,
        layer: 0,
        ramp: false,
        oneway: 0,
        expressway: false,
        service: None,
        network: None,
        geometry: Geometry::Lines(vec![line]),
    }
}

fn tile(id: TileId, features: &Features<naval_osm::TilePoint>) -> Arc<OsmTile> {
    Arc::new(OsmTile::new(MONTEREY_BAY, id, features))
}

#[test]
fn a_lit_road_has_its_lamps_at_its_spacing_once_across_two_tiles() {
    // A primary road west to east across the middle of two neighbouring tiles, each holding it past its edge (the
    // tiles' buffer), and a track (unlit) beside it.
    let west = TileId::of(36.97, -122.03, 14);
    let east = TileId::new(west.z, west.x + 1, west.y);
    let mut a: Features<naval_osm::TilePoint> = Features::default();
    a.transportation
        .push(way("primary", vec![[-0.1, 0.5], [1.1, 0.5]]));
    a.transportation
        .push(way("track", vec![[-0.1, 0.6], [1.1, 0.6]]));
    let mut b: Features<naval_osm::TilePoint> = Features::default();
    b.transportation
        .push(way("primary", vec![[-0.1, 0.5], [1.1, 0.5]]));
    let tiles = [tile(west, &a), tile(east, &b)];
    // The page under the tiles' shared edge.
    let edge = osm::project(MONTEREY_BAY, west, [1.0, 0.5]);
    let index = page_of(LAMP_LEVEL, edge);
    let lamps = page_lamps(MONTEREY_BAY, index, &tiles, &Roads::today());
    assert!(!lamps.is_empty());
    assert!(lamps.iter().all(|l| l.kind() == LampKind::Street as u32));
    assert!(lamps.iter().all(|l| l.class() == road_class("primary")));
    // Every lamp on the page, and none two in one place (each tile keeps only its own half).
    let side = crate::features::page_m(LAMP_LEVEL) as f32;
    assert!(
        lamps
            .iter()
            .all(|l| (0.0..side).contains(&l.at[0]) && (0.0..side).contains(&l.at[1]))
    );
    for (i, l) in lamps.iter().enumerate() {
        for m in &lamps[i + 1..] {
            assert!(
                (l.at[0] - m.at[0]).hypot(l.at[1] - m.at[1]) > 30.0,
                "{l:?} {m:?}"
            );
        }
    }
    // The page's 2 km of road at 40 m: about fifty, staggered either side of it.
    assert!((45..=55).contains(&lamps.len()), "{}", lamps.len());
}

#[test]
fn industrial_land_is_floodlit_with_a_pool_under_each_flood() {
    let id = TileId::of(36.97, -122.03, 14);
    let mut f: Features<naval_osm::TilePoint> = Features::default();
    f.landuse.push(Landuse {
        class: "industrial".into(),
        geometry: Geometry::Polygons(vec![naval_osm::Polygon {
            exterior: vec![[0.4, 0.4], [0.6, 0.4], [0.6, 0.6], [0.4, 0.6]],
            holes: vec![],
        }]),
    });
    f.landuse.push(Landuse {
        class: "residential".into(),
        geometry: Geometry::Polygons(vec![naval_osm::Polygon {
            exterior: vec![[0.1, 0.1], [0.3, 0.1], [0.3, 0.3], [0.1, 0.3]],
            holes: vec![],
        }]),
    });
    let tiles = [tile(id, &f)];
    let middle = osm::project(MONTEREY_BAY, id, [0.5, 0.5]);
    let lamps = page_lamps(
        MONTEREY_BAY,
        page_of(LAMP_LEVEL, middle),
        &tiles,
        &Roads::today(),
    );
    let floods: Vec<_> = lamps
        .iter()
        .filter(|l| l.kind() == LampKind::Flood as u32)
        .collect();
    let pools: Vec<_> = lamps
        .iter()
        .filter(|l| l.kind() == LampKind::Pool as u32)
        .collect();
    assert_eq!(floods.len(), pools.len());
    // In a past period, only over the ground that stood: all where it all stood, none where it went to the country.
    let page = page_of(LAMP_LEVEL, middle);
    let in_period = |stood: &(dyn Fn([f64; 2]) -> bool + Sync)| {
        let roads = Roads::new(
            naval_terrain::buildings::Period::Year(1942),
            MONTEREY_BAY,
            middle,
            stood,
        );
        page_lamps(MONTEREY_BAY, page, &tiles, &roads)
            .iter()
            .filter(|l| l.kind() == LampKind::Flood as u32)
            .count()
    };
    assert_eq!(in_period(&|_| true), floods.len());
    assert_eq!(in_period(&|_| false), 0);
    assert_eq!(
        in_period(&|p| p[0] < middle[0]) * 3 / floods.len(),
        1,
        "about half"
    );
    // A zoom-14 tile is about 2 km across here: the square a fifth of it, about 400 m a side, takes a dozen or so floods
    // on the 110 m grid (fewer if the page's edge cuts it), none on a pier.
    assert!(!floods.is_empty() && floods.len() < 30, "{}", floods.len());
    assert!(floods.iter().all(|l| l.class() == 0));
    for (flood, pool) in floods.iter().zip(&pools) {
        assert_eq!(flood.at, pool.at);
        assert_eq!(flood.seed(), pool.seed());
    }
}

#[test]
fn each_region_its_palette_and_each_road_its_lamp() {
    assert_eq!(
        palette_at(LatLon {
            lat: 35.3,
            lon: 139.6
        }),
        LampPalette::White
    );
    assert_eq!(
        palette_at(LatLon {
            lat: 33.7,
            lon: -118.4
        }),
        LampPalette::Sodium
    );
    assert_eq!(
        palette_at(LatLon {
            lat: -9.4,
            lon: 160.0
        }),
        LampPalette::Sodium
    );
    // A driveway is not lit; a motorway brighter and taller than a minor road.
    let tone_half = |class: &str, service: Option<&str>| {
        let mut t = way(class, vec![[0.0, 0.0], [1.0, 0.0]]);
        t.service = service.map(Into::into);
        let (_, half, tone) = crate::features::style_of(&t).unwrap();
        street_light(tone, half)
    };
    assert_eq!(tone_half("service", Some("driveway")), None);
    assert!(tone_half("service", None).is_some());
    let (_, high, bright) = tone_half("motorway", None).unwrap();
    let (_, low, dim) = tone_half("minor", None).unwrap();
    assert!(high > low && bright > dim);
}
