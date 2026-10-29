//! Each operation held to its kind (the open-ocean plan, step C §3.1, "Dependency units"). A local operation reads
//! within its reach and nothing about where its region starts; a component operation decides each component by the
//! component. Both are held at once by baking a square with more than its halo at every stage: the window is the same
//! bytes. And a halo short of the components' cap is a mutant the edges catch.
use std::sync::Arc;

use naval_land_format::SquareKey;
use naval_region_bake::finish::synthetic::{
    Feature, M_PER_DEGREE, Scene, Shape, SyntheticSources, World,
};
use naval_region_bake::finish::{self, Halo};

use crate::finish_support::{assert_shared_edge, world_across};

fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

#[test]
fn more_halo_at_every_stage_bakes_the_same_window() {
    for (a, east, seed, rows, columns) in [
        (k(10, 20), true, 1, 3, 5),
        (k(61, -8), false, 2, 1, 17),
        (k(-30, 179), true, 3, 6, 2),
    ] {
        let b = if east { a.east() } else { a.north().unwrap() };
        let world = world_across(a, b, seed);
        for key in [a, b] {
            let sources = SyntheticSources::new(world.clone());
            let plain = finish::bake(key, &sources, &sources).unwrap();
            let wide = finish::bake_in(
                key,
                &sources,
                &sources,
                &Halo::with_slack(key, rows, columns),
            )
            .unwrap();
            assert_eq!(
                plain.window, wide.window,
                "{key}: more halo changed the window"
            );
            assert_eq!(plain.report, wide.report, "{key}");
        }
    }
}

#[test]
fn a_component_stage_short_of_the_cap_breaks_the_edges() {
    // A low spit 80 m wide and 700 m long lies east of the edge (10° N, 21° E), from 18 to 28 columns from it: rounding
    // sinks it, and the islands kept restore it, which the shelf off it shows at the edge (1.25 km away). The mutant's
    // component stages hold a sample about each point, not the cap: the west square then holds only part of the spit
    // (its components end 23 samples east of its window), cannot decide it, and leaves it sunk, while the east square
    // restores it. Their edges disagree; with the true halo they do not.
    let (a, b) = (k(10, 20), k(10, 21));
    let lat: f64 = 10.5;
    let m_lon = M_PER_DEGREE * naval_region_bake::finish::lattice::sin_cos(lat.to_radians()).1;
    let column = naval_region_bake::finish::lattice::column_m(16_800);
    let spit = Feature::new(
        [lat, 21.0 + 23.0 * column / m_lon],
        Shape::Spit {
            half_length: 5.0 * column,
            half_width: 40.0,
            bearing_deg: 0.0,
        },
        4.0,
    );
    let world = Arc::new(World::new(vec![
        Scene::open([9.5, 11.5, 19.5, 22.5]).with(spit).seal(),
    ]));
    let bake = |key: SquareKey, halo: &Halo| {
        let sources = SyntheticSources::new(world.clone());
        finish::bake_in(key, &sources, &sources, halo).unwrap()
    };
    let (ta, tb) = (bake(a, &Halo::of(a)), bake(b, &Halo::of(b)));
    assert!(
        tb.report.islands_kept > 0,
        "the spit is restored: {:?}",
        tb.report
    );
    assert_shared_edge(&ta, &tb, true);
    let mutant = |key: SquareKey| {
        let mut halo = Halo::of(key);
        for stage in [2, 3, 4] {
            halo.stages[stage] = (1, 1);
        }
        bake(key, &halo)
    };
    let (ma, mb) = (mutant(a), mutant(b));
    let caught = std::panic::catch_unwind(|| assert_shared_edge(&ma, &mb, true));
    assert!(
        caught.is_err(),
        "a halo short of the cap went unseen at the edge"
    );
}
