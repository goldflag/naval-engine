//! The second bake's sea floor (step C, BAKE-3: `land-v2`): the release's bathymetry by place wherever the water
//! polygons say sea, on the synthetic world with a synthetic floor of its own (`finish::synthetic::SyntheticSeabed`:
//! the DEM's land where it stands above the sea, else banks from 3 m and deeps to 400 m).
//!
//! - Every edge two baked squares share is the same bytes and the same limiting heights, within a band, across the
//!   date line and at the pole limit, as the first bake's are.
//! - The edge a square shares with an open-sea square (the table's `ocean`, or `polar`) is the open sea's floor at
//!   every point, which is what the simulation reads past it: one continuous floor.
//! - The land is the first bake's, and what the first bake's rounding cut from the polygons' land besides (a headland's
//!   tip): every first-bake land point is land, and every one standing above the coast's lift (6 m) holds its height to
//!   the quantum. The sea's points move where the floor is not the shelf's.
//! - More halo at every stage bakes the same window: the floor is a local operation like the rest.
use std::collections::BTreeSet;
use std::sync::Arc;

use naval_land_format::SquareKey;
use naval_land_format::lattice::WINDOW;
use naval_region_bake::finish::synthetic::{
    Feature, Scene, Shape, SyntheticSeabed, SyntheticSources, World,
};
use naval_region_bake::finish::{self, Finished, Halo};

use crate::finish_support::{assert_shared_edge, world_across};

fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

fn seabed(world: &Arc<World>, open: &[SquareKey]) -> SyntheticSeabed {
    SyntheticSeabed {
        world: world.clone(),
        open: open.iter().copied().collect::<BTreeSet<_>>(),
    }
}

/// Square `key` by the second bake, on sources of its own.
fn bake_v2(key: SquareKey, world: &Arc<World>, open: &[SquareKey]) -> Finished {
    let sources = SyntheticSources::new(world.clone());
    let floor = seabed(world, open);
    finish::bake_version(2, key, &sources, &sources, Some(&floor)).unwrap()
}

fn edge(a: SquareKey, east: bool, seed: u64) {
    let b = if east { a.east() } else { a.north().unwrap() };
    let world = world_across(a, b, seed);
    let (fa, fb) = (bake_v2(a, &world, &[]), bake_v2(b, &world, &[]));
    assert!(
        fa.report.floor > 0 && fb.report.floor > 0,
        "{:?}",
        fa.report
    );
    let land = assert_shared_edge(&fa, &fb, east);
    assert!(
        land > 50,
        "{a} and {b}: only {land} land points on their edge"
    );
}

#[test]
fn shared_edges_are_one_floor_within_a_band_across_the_date_line_and_at_the_pole_limit() {
    edge(k(10, 20), true, 1);
    edge(k(-41, -71), false, 3);
    edge(k(5, 179), true, 40);
    edge(k(88, 10), true, 50);
}

#[test]
fn an_open_sea_squares_edge_is_the_open_floor_and_the_floor_fades_to_it() {
    // Open sea about 10° N 20° E with an island in it; the square north of it is open sea.
    let (a, east) = (k(10, 20), k(10, 21));
    let open = k(11, 20);
    let island = Feature::new(
        [10.4, 20.6],
        Shape::Blob {
            radius: 9000.0,
            waves: [(0.1, 0.3), (0.0, 0.0), (0.05, 1.0)],
        },
        300.0,
    );
    let world = Arc::new(World::new(vec![
        Scene::open([9.2, 11.8, 19.2, 21.8]).with(island).seal(),
    ]));
    let f = bake_v2(a, &world, &[open]);
    let open_floor = (-naval_region_bake::SHELF_DEPTH_M / 0.25) as i16;
    for c in 0..WINDOW {
        let q = f.window.at(0, c);
        assert!(
            q == open_floor || q > 0,
            "column {c} of the north edge: {q}"
        );
    }
    let sea_on_edge = (0..WINDOW)
        .filter(|&c| f.window.at(0, c) == open_floor)
        .count();
    assert_eq!(sea_on_edge, WINDOW, "the north edge is open sea");
    // The fade: within its 20 km of the edge the floor goes to the open floor; a row 25 km in is the floor's own.
    let row_25km = (25_000.0 / naval_region_bake::finish::lattice::ROW_M) as usize;
    assert!((0..WINDOW).any(|c| f.window.at(row_25km, c) != open_floor));
    // Its own sea away from open sea is the floor's, from banks a few metres deep to deeps past the shelf's 160 m.
    let mid: Vec<i16> = (WINDOW / 2..WINDOW)
        .flat_map(|r| (0..WINDOW).map(move |c| (r, c)))
        .map(|(r, c)| f.window.at(r, c))
        .filter(|&q| q < 0)
        .collect();
    assert!(mid.iter().any(|&q| q > -40), "a bank shallower than 10 m");
    assert!(mid.iter().any(|&q| q < 4 * -250), "a deep past 250 m");
    // The east neighbour's edge with it is still one floor.
    let fe = bake_v2(east, &world, &[open]);
    assert_shared_edge(&f, &fe, true);
}

#[test]
fn the_land_is_the_first_bakes_and_the_tips_its_rounding_cut() {
    for (a, east, seed) in [
        (k(10, 20), true, 1),
        (k(61, -8), false, 2),
        (k(-30, 179), true, 3),
    ] {
        let b = if east { a.east() } else { a.north().unwrap() };
        let world = world_across(a, b, seed);
        let sources = SyntheticSources::new(world.clone());
        let one = finish::bake(a, &sources, &sources).unwrap();
        let two = bake_v2(a, &world, &[]);
        let (mut moved, mut kept) = (0, 0);
        for (p, q) in one.window.quanta.iter().zip(&two.window.quanta) {
            if *p > 0 {
                assert!(*q > 0, "{a}: the first bake's land is land");
                // Above the coast's lift (6 m, 24 quanta) nothing the rounding kept reaches a height.
                if *p > 24 {
                    assert_eq!(p, q, "{a}: a land height moved");
                }
            } else if *q > 0 {
                kept += 1;
            } else if p != q {
                moved += 1;
            }
        }
        assert!(moved > 1000, "{a}: the floor moved only {moved} sea points");
        let (r1, r2) = (one.report, two.report);
        // The rounding cuts nothing: what it cut in the first bake is land, each point of it.
        assert_eq!(r2.rounded_down, 0, "{a}");
        // Land only where the first bake's rounding cut it (or dropped what it cut as a speck).
        assert_eq!(kept, r2.land - r1.land, "{a}");
        assert!(
            kept <= r1.rounded_down + r1.specks,
            "{a}: {kept} points kept, {} cut, {} specks",
            r1.rounded_down,
            r1.specks
        );
        assert_eq!(
            (r1.slid, r1.lowered, r1.rounded_up, r1.filled),
            (r2.slid, r2.lowered, r2.rounded_up, r2.filled),
            "{a}"
        );
    }
}

#[test]
fn more_halo_bakes_the_same_floor() {
    let (a, b) = (k(-30, 179), k(-30, -180));
    let world = world_across(a, b, 3);
    for key in [a, b] {
        let sources = SyntheticSources::new(world.clone());
        let floor = seabed(&world, &[k(-31, 179)]);
        let plain =
            finish::bake_with(key, &sources, &sources, &Halo::of(key), Some(&floor)).unwrap();
        let wide = finish::bake_with(
            key,
            &sources,
            &sources,
            &Halo::with_slack(key, 6, 2),
            Some(&floor),
        )
        .unwrap();
        assert_eq!(
            plain.window, wide.window,
            "{key}: more halo changed the floor"
        );
    }
}

#[test]
fn the_second_bake_needs_its_floor_and_names_itself() {
    let world = world_across(k(10, 20), k(10, 21), 1);
    let sources = SyntheticSources::new(world);
    assert!(finish::bake_version(2, k(10, 20), &sources, &sources, None).is_err());
    assert!(finish::bake_version(3, k(10, 20), &sources, &sources, None).is_err());
    let (one, two) = (finish::bake_spec_for(1), finish::bake_spec_for(2));
    assert_eq!((one.version, one.keep_m), (1, 40_000));
    assert_eq!((two.version, two.keep_m), (2, 0));
    assert_eq!(finish::version_of(&one), Some(1));
    assert_eq!(finish::version_of(&two), Some(2));
    assert_eq!(finish::bake_spec(), two);
    assert!(finish::keeps_every_candidate(2) && !finish::keeps_every_candidate(1));
}
