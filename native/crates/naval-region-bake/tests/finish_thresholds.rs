//! (c) The thresholds (the open-ocean plan, step C §7.2, "The bake"; C1, "Which squares hold land"): a square is a
//! candidate when the coastline's land lies within 65 km of it, and a candidate's manifest line is data when final land
//! lies within 40 km of it, else `empty`. Each threshold has a case either side of it, a lattice column or two away.
use std::sync::Arc;

use naval_land_format::lattice::PER_DEGREE;
use naval_land_format::{Digest, Entry, Manifest, SquareKey};
use naval_region_bake::finish::keep::{self, CANDIDATE_M, KEEP_M};
use naval_region_bake::finish::synthetic::{
    Feature, M_PER_DEGREE, Scene, Shape, SyntheticSources, World,
};
use naval_region_bake::finish::{self, lattice};

fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

/// Open sea about `key` (two degrees each way) with one high round island, its west point `metres` east of the square's
/// east edge on the square's middle parallel.
fn island_east_of(key: SquareKey, metres: f64) -> Arc<World> {
    let radius = 3000.0;
    let lat = f64::from(key.lat) + 0.5;
    let m_lon = M_PER_DEGREE * lattice::sin_cos(lat.to_radians()).1;
    let centre = [lat, f64::from(key.lon + 1) + (metres + radius) / m_lon];
    let mut island = Feature::new(
        centre,
        Shape::Blob {
            radius,
            waves: [(0.0, 0.0); 3],
        },
        200.0,
    );
    island.cliff = true;
    let bounds = [
        f64::from(key.lat) - 2.0,
        f64::from(key.lat) + 3.0,
        f64::from(key.lon) - 2.0,
        f64::from(key.lon) + 3.0,
    ];
    Arc::new(World::new(vec![Scene::open(bounds).with(island).seal()]))
}

/// The least distance from square `key` to the final land of square `of`'s window, metres.
fn nearest_land(key: SquareKey, of: SquareKey, window: &naval_land_format::block::Window) -> f64 {
    let w = lattice::Region::window(of);
    let mut best = f64::INFINITY;
    for r in 0..w.rows {
        for c in 0..w.columns {
            if window.at(r, c) > 0 {
                best = best.min(keep::to_square_m(key, w.i(r), w.j(c)));
            }
        }
    }
    best
}

#[test]
fn the_40_km_keep_each_side() {
    let s = k(-30, -141);
    let n = s.east();
    let mut lines = Manifest::default();
    let mut nearest = vec![];
    for (metres, data) in [(KEEP_M - 90.0, true), (KEEP_M + 80.0, false)] {
        let world = island_east_of(s, metres);
        let sources = SyntheticSources::new(world.clone());
        let own = finish::bake(s, &sources, &sources).unwrap();
        assert_eq!(own.report.land, 0, "the square itself is open sea");
        let theirs = finish::bake(n, &sources, &sources).unwrap();
        let d = nearest_land(s, n, &theirs.window);
        nearest.push(d);
        // The island is the only land: every other square near the square is open sea.
        let kept = keep::keep(s, &own.window, &mut |q| {
            Ok((q == n).then(|| theirs.window.clone()))
        })
        .unwrap();
        assert_eq!(kept, data, "final land {d} m from the square");
        assert_eq!(d <= KEEP_M, data, "{d}");
        let entry = if kept {
            Entry::Data(Digest::of(&own.bytes()))
        } else {
            Entry::Empty
        };
        let mut one = Manifest::default();
        one.record(s, entry).unwrap();
        lines = one;
    }
    // Both sides within a few lattice columns of the threshold (60.5 m a column here).
    assert!(
        nearest[0] <= KEEP_M && nearest[0] > KEEP_M - 250.0,
        "{nearest:?}"
    );
    assert!(
        nearest[1] > KEEP_M && nearest[1] < KEEP_M + 250.0,
        "{nearest:?}"
    );
    assert_eq!(lines.text(), format!("{} empty\n", s.name()));
}

#[test]
fn the_65_km_candidate_each_side() {
    let s = k(-30, -141);
    for (metres, candidate) in [(CANDIDATE_M - 80.0, true), (CANDIDATE_M + 80.0, false)] {
        let world = island_east_of(s, metres);
        assert_eq!(
            keep::coastline_land_within(s, world.as_ref(), CANDIDATE_M).unwrap(),
            candidate,
            "an island {metres} m off"
        );
        // A candidate this far from land bakes to open sea, and its line is empty.
        if candidate {
            let sources = SyntheticSources::new(world.clone());
            let own = finish::bake(s, &sources, &sources).unwrap();
            assert!(!keep::keep(s, &own.window, &mut |_| Ok(None)).unwrap());
        }
    }
    // The threshold in lattice columns at the square's middle parallel: the cases are a column or two either side.
    let column = lattice::column_m(-29 * PER_DEGREE - PER_DEGREE / 2);
    assert!(80.0 < 2.0 * column && 80.0 > column, "{column}");
}
