//! (a) Shared edges (the open-ocean plan, step C §7.2, "The bake"): two neighbouring squares baked apart, each on
//! sources of its own, hold the same bytes at every point of the edge they share, and their limiting bilinear heights
//! are equal at every metre of it: within a band, across each of the surveyed layer's band changes, across the DEM's
//! change at 60° N and 56° S, across ±180° and at the pole limit. Each edge has a scene of its own laid across it
//! (islands it cuts, specks on it, spits and atolls each side of the component cap, ponds, a mainland's coast).
use std::sync::Arc;

use naval_land_format::SquareKey;
use naval_land_format::lattice::{PER_DEGREE, WINDOW};
use naval_region_bake::BakeError;
use naval_region_bake::finish;
use naval_region_bake::finish::surveyed::{self, Surveyed};
use naval_region_bake::finish::synthetic::{Feature, Scene, Shape, SyntheticSources, World};
use naval_region_bake::square::{MARGIN, columns_per_degree};

use crate::finish_support::{assert_shared_edge, bake_alone, world_across};

fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

/// Bakes `a` and its east (or north) neighbour apart and checks their edge, which must cross land.
fn edge(a: SquareKey, east: bool, seed: u64) {
    let b = if east { a.east() } else { a.north().unwrap() };
    let world = world_across(a, b, seed);
    let (fa, fb) = (bake_alone(a, &world), bake_alone(b, &world));
    let land = assert_shared_edge(&fa, &fb, east);
    assert!(
        land > 50,
        "{a} and {b}: only {land} land points on their edge"
    );
    for f in [&fa, &fb] {
        let r = f.report;
        assert!(
            r.land > 0 && r.rounded_up + r.rounded_down > 0,
            "{}: {r:?}",
            f.key
        );
    }
}

#[test]
fn within_a_band() {
    edge(k(10, 20), true, 1);
    edge(k(10, 20), false, 2);
    edge(k(-41, -71), true, 3);
}

#[test]
fn across_each_band_change_north() {
    // The surveyed layer's columns change between these and the squares north of them (1,600 to 1,400 a degree at
    // 30° N, …, 280 to 140 at 85° N).
    for (n, lat) in [29, 44, 54, 61, 67, 73, 79, 84].into_iter().enumerate() {
        let a = k(lat, 30);
        assert_ne!(columns_per_degree(lat), columns_per_degree(lat + 1));
        edge(a, false, 10 + n as u64);
    }
}

#[test]
fn across_each_band_change_south() {
    for (n, lat) in [-31, -46, -56, -63, -69, -75, -81, -86]
        .into_iter()
        .enumerate()
    {
        let a = k(lat, -50);
        assert_ne!(columns_per_degree(lat), columns_per_degree(lat + 1));
        edge(a, false, 20 + n as u64);
    }
}

#[test]
fn across_the_dems_change_at_60_north_and_56_south() {
    // One band either side: only the DEM (its synthetic stand-ins, by place) changes.
    assert_eq!(columns_per_degree(59), columns_per_degree(60));
    assert_eq!(columns_per_degree(-57), columns_per_degree(-56));
    edge(k(59, 5), false, 30);
    edge(k(-57, 5), false, 31);
}

#[test]
fn across_the_date_line() {
    edge(k(5, 179), true, 40);
    edge(k(-20, 179), true, 41);
    edge(k(70, 179), true, 42);
}

#[test]
fn at_the_pole_limit() {
    // The last squares before the pole limit (89.5°): their halos reach 4,500 columns a side. One test, so they bake
    // one at a time.
    edge(k(88, 10), true, 50);
    edge(k(87, 10), false, 51);
    edge(k(-89, 10), true, 52);
    edge(k(-89, 10), false, 53);
}

#[test]
fn at_a_polar_squares_edge_every_point_is_sea() {
    // N89 and S90 are polar: open sea, never baked. An island stands across each of their near edges (89° N, 89° S);
    // the baked square beside it (N88, S89) keeps its land short of the edge, and every point of the edge is sea.
    for (key, lat, north_edge) in [(k(88, 10), 89.0, true), (k(-89, 10), -89.0, false)] {
        let island = Feature::new(
            [lat, 10.5],
            Shape::Blob {
                radius: 8000.0,
                waves: [(0.1, 0.3), (0.0, 0.0), (0.05, 1.0)],
            },
            300.0,
        );
        let bounds = if north_edge {
            [87.5, 89.9, 3.0, 18.0]
        } else {
            [-89.9, -87.5, 3.0, 18.0]
        };
        let world = Arc::new(World::new(vec![Scene::open(bounds).with(island).seal()]));
        assert!(
            world.height(lat, 10.5) > 100.0,
            "the sources hold land on the edge"
        );
        let f = bake_alone(key, &world);
        let edge = if north_edge { 0 } else { WINDOW - 1 };
        let land_on_edge = (0..WINDOW).filter(|&c| f.window.at(edge, c) > 0).count();
        assert_eq!(land_on_edge, 0, "{key}: land on the polar square's edge");
        // The island stands a few rows short of it: within 120 rows (8.3 km) of the edge.
        let near = |r: usize| (0..WINDOW).filter(|&c| f.window.at(r, c) > 0).count();
        let rows: Vec<usize> = if north_edge {
            (1..120).collect()
        } else {
            (WINDOW - 120..WINDOW - 1).collect()
        };
        let land_near: usize = rows.into_iter().map(near).sum();
        assert!(
            land_near > 1000,
            "{key}: the island is gone ({land_near} samples)"
        );
    }
}

#[test]
fn a_polar_square_is_refused() {
    let world = Arc::new(World::new(vec![]));
    let sources = SyntheticSources::new(world);
    for key in [k(89, 0), k(-90, 179)] {
        assert!(
            matches!(
                finish::bake(key, &sources, &sources),
                Err(BakeError::Refused(_))
            ),
            "{key}"
        );
    }
}

#[test]
fn a_surveyed_height_at_a_band_change_is_its_owners() {
    // 29° N (1,600 columns a degree) and 30° N (1,400): a point on 30° N belongs to the north square, a row south of it
    // to the south square, and the bake reads each from its owner's own samples, never the other's margin.
    let (south, north) = (k(29, 30), k(30, 30));
    let world = world_across(south, north, 60);
    let sources = SyntheticSources::new(world);
    let (sq_s, sq_n) = (
        sources.square(south).unwrap(),
        sources.square(north).unwrap(),
    );
    let edge = 30 * PER_DEGREE;
    let mut differ = 0;
    for j in (30 * PER_DEGREE..31 * PER_DEGREE).step_by(7) {
        // Each square's own reading: its row for the latitude, linear between its columns.
        let own = |square: &naval_region_bake::square::Square, i: i32| {
            let g = square.grid;
            let across = columns_per_degree(g.key.lat) as f64;
            let r = ((g.key.lat + 1) * PER_DEGREE - i) as usize + MARGIN;
            let x = f64::from(j - g.key.lon * PER_DEGREE) * across / f64::from(PER_DEGREE);
            let c = x.floor() as usize + MARGIN;
            let t = x - x.floor();
            let v = |c: usize| f64::from(square.heights()[r * g.columns + c]);
            v(c) + (v(c + 1) - v(c)) * t
        };
        let on = surveyed::height_at(&sources, edge, j).unwrap();
        assert!((on - own(&sq_n, edge)).abs() < 1e-9, "{j}");
        let below = surveyed::height_at(&sources, edge - 1, j).unwrap();
        assert!((below - own(&sq_s, edge - 1)).abs() < 1e-9, "{j}");
        // The south square's margin holds the same row on its own grid: the reading would differ there.
        differ += usize::from((own(&sq_s, edge) - on).abs() > 1e-3);
    }
    assert!(
        differ > 0,
        "the two grids' readings never differ: the test shows nothing"
    );
}
