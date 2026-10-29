//! The window cut from a land release without the simulation (`naval_terrain::release`), on the synthetic release:
//! the squares it names, its chart, and the heights it reads where the synthetic world says they are.

use naval_land_format::synthetic;
use naval_land_format::{SquareKey, lattice};
use naval_terrain::heightfield::OPEN_SEA_M;
use naval_terrain::release::{RELEASE_SEA_M, ReleaseLand, ReleaseWindow, WINDOW_STEP_M};

/// A window 12 km across on Long Point's south coast (local y = −15 km), on the 140° W meridian between two squares.
fn coast() -> ReleaseWindow {
    ReleaseWindow {
        place: synthetic::place([0.0, -15.0]),
        samples: 301,
        cell: 40.0,
        falloff: 2000.0,
    }
}

/// The synthetic world's lattice read bilinearly at `[lat, lon]`, from its generator's quanta.
fn lattice_height([lat, lon]: [f64; 2]) -> f64 {
    let key = SquareKey::at(lat, lon);
    let [row, column] = lattice::position(key, lat, lon);
    let (r, c) = (row.floor(), column.floor());
    let (wr, wc) = (row - r, column - c);
    let h = |dr: usize, dc: usize| {
        f64::from(synthetic::quantum(lattice::window_point(
            key,
            r as usize + dr,
            c as usize + dc,
        ))) * lattice::QUANTUM_M
    };
    (h(0, 0) * (1.0 - wc) + h(0, 1) * wc) * (1.0 - wr) + (h(1, 0) * (1.0 - wc) + h(1, 1) * wc) * wr
}

fn land(window: &ReleaseWindow) -> ReleaseLand {
    ReleaseLand::new(window.squares().into_iter().filter_map(synthetic::square)).unwrap()
}

#[test]
fn the_chart_is_the_theatres_about_the_place() {
    let window = coast();
    let [lat, lon] = window.place;
    let degree = synthetic::KM_PER_DEGREE * 1000.0;
    // North and south along the meridian: a degree of latitude to the kilometre the synthetic world measures.
    let north = window.lat_lon([0.0, -degree]);
    assert!(
        (north[0] - (lat + 1.0)).abs() < 1e-9 && (north[1] - lon).abs() < 1e-9,
        "{north:?}"
    );
    let south = window.lat_lon([0.0, degree / 2.0]);
    assert!((south[0] - (lat - 0.5)).abs() < 1e-9, "{south:?}");
    // East, a short way: the parallel's degree (cos lat of the meridian's).
    let east = window.lat_lon([1000.0, 0.0]);
    let expected = 1000.0 / (degree * lat.to_radians().cos());
    assert!(
        (east[1] - lon - expected).abs() < 1e-6 && (east[0] - lat).abs() < 1e-6,
        "{east:?}"
    );
    assert_eq!(window.lat_lon([0.0, 0.0]), window.place);
}

#[test]
fn a_window_names_the_squares_it_meets() {
    assert_eq!(
        coast().squares(),
        vec![
            SquareKey {
                lat: -31,
                lon: -141
            },
            SquareKey {
                lat: -31,
                lon: -140
            },
        ]
    );
    let middle = ReleaseWindow {
        place: [-30.5, -139.5],
        ..coast()
    };
    assert_eq!(
        middle.squares(),
        vec![SquareKey {
            lat: -31,
            lon: -140
        }]
    );
}

#[test]
fn the_window_reads_the_releases_coast() {
    let window = coast();
    let land = land(&window);
    assert_eq!(land.keys().count(), 2, "both squares hold data");
    let field = window.heightfield(&land).expect("the coast's land");
    assert_eq!((field.columns, field.rows), (301, 301));
    assert_eq!((field.origin_x, field.origin_z), (-6000.0, -6000.0));
    assert!(
        field.quanta.iter().all(|&q| q != 0),
        "nothing on the waterline"
    );
    // Inside the settling band each sample reads the release's lattice at its place, bilinearly between the four
    // lattice points about it (the generator's own quanta, not the square files' blocks), to the window's quantum.
    let mut worst = 0f64;
    for j in (60..=240).step_by(10) {
        for i in (60..=240).step_by(10) {
            let (x, z) = (-6000.0 + f64::from(i) * 40.0, -6000.0 + f64::from(j) * 40.0);
            worst = worst.max((field.sample(i, j) - lattice_height(window.lat_lon([x, z]))).abs());
        }
    }
    assert!(
        worst <= f64::from(WINDOW_STEP_M) / 2.0 + 1e-9,
        "off the lattice's heights by {worst} m"
    );
    // North of the coast the land, 4 km inland about 100 m up; south of it the sea's floor; the corners settled.
    let inland = field.height(0.0, -4000.0);
    assert!((95.0..107.0).contains(&inland), "4 km inland: {inland} m");
    assert!(
        field.height(0.0, 3000.0) < -100.0,
        "the sea south of the coast"
    );
    let corner = field.sample(0, 0);
    assert!(
        (corner - OPEN_SEA_M).abs() <= f64::from(WINDOW_STEP_M),
        "the corner: {corner} m"
    );
}

#[test]
fn a_window_with_no_land_has_no_field() {
    let window = coast();
    // No squares given: every place is the open sea's floor.
    assert_eq!(
        ReleaseLand::default().height(window.place[0], window.place[1]),
        RELEASE_SEA_M
    );
    assert!(window.heightfield(&ReleaseLand::default()).is_none());
}
