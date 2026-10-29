//! The synthetic release: the same bytes every time, a table, manifest and squares that agree, and the world §7.3 asks
//! for (a 300 km peninsula, a 2 km strait, a bay whose mouth is in another square, a closed lagoon, an island chain, a
//! legacy region whose disc edge crosses an island).
use std::sync::OnceLock;

use naval_land_format::block::Window;
use naval_land_format::lattice::{self, BLOCKS};
use naval_land_format::synthetic::{self, LAND, Shape, Synthetic};
use naval_land_format::{
    Class, Digest, Entry, FinalSquare, Manifest, ReleaseFile, SquareKey, SquareTable,
};

fn made() -> &'static Synthetic {
    static MADE: OnceLock<Synthetic> = OnceLock::new();
    MADE.get_or_init(synthetic::release)
}

/// Land at a place on the local map (km), as the release's quanta say there.
fn land(km: [f64; 2]) -> bool {
    let [lat, lon] = synthetic::place(km);
    synthetic::height_m(lat, lon) > 0.0
}

fn square_at(km: [f64; 2]) -> SquareKey {
    let [lat, lon] = synthetic::place(km);
    SquareKey::at(lat, lon)
}

#[test]
fn the_release_is_the_same_bytes_every_time() {
    let again = synthetic::release();
    assert_eq!(&again, made());
    let first = made();
    for (key, bytes) in &first.squares {
        assert_eq!(Digest::of(bytes), Digest::of(&again.squares[key]), "{key}");
    }
    // The release names its table; its id is its file's digest.
    let release = ReleaseFile::parse(&first.release).unwrap();
    assert_eq!(release.id, first.id);
    assert!(release.file.synthetic);
    assert!(release.file.square_table.matches(&first.table));
}

#[test]
fn its_table_manifest_and_squares_agree() {
    let made = made();
    let table = SquareTable::decode(&made.table).unwrap();
    let manifest = Manifest::parse(&made.manifest).unwrap();
    let candidates: Vec<SquareKey> = table.candidates().collect();
    assert_eq!(
        manifest.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        candidates
    );
    let mut data = 0;
    for (key, entry) in manifest.iter() {
        match entry {
            Entry::Data(digest) => {
                let bytes = &made.squares[&key];
                assert_eq!(Digest::of(bytes), digest);
                assert_eq!(FinalSquare::read(bytes.clone()).unwrap().key(), key);
                data += 1;
            }
            Entry::Empty => assert!(!made.squares.contains_key(&key)),
        }
    }
    assert_eq!(data, made.squares.len());
    // The world lies about its centre: every candidate within a few degrees, the rest of the Earth ocean or polar.
    assert!(
        candidates
            .iter()
            .all(|k| (-34..=-27).contains(&k.lat) && (-145..=-134).contains(&k.lon))
    );
    assert!(
        manifest.iter().any(|(_, e)| e == Entry::Empty),
        "some candidate is empty"
    );
    assert_eq!(table.class(SquareKey { lat: 0, lon: 0 }), Class::Ocean);
    assert_eq!(table.class(SquareKey { lat: 89, lon: 0 }), Class::Polar);
    println!(
        "synthetic release: {} candidates, {} data squares",
        candidates.len(),
        data
    );
}

#[test]
fn a_block_asks_only_the_shapes_near_it_and_gives_every_shapes_quanta() {
    // The bay's mouth and the peninsula's tip.
    for key in [square_at([-140.0, -105.0]), square_at([160.0, 0.0])] {
        assert_eq!(
            synthetic::window(key),
            Window::of(key, synthetic::quantum),
            "{key}"
        );
    }
}

#[test]
fn neighbouring_squares_meet_on_their_edges() {
    let squares = &made().squares;
    let read = |key: &SquareKey| FinalSquare::read(squares[key].clone()).unwrap();
    let mut pairs = 0;
    for key in squares.keys() {
        let here = read(key);
        if let Some(east) = squares.get(&key.east()).map(|_| read(&key.east())) {
            for b in 0..BLOCKS {
                let (a, z) = (
                    here.block(b, BLOCKS - 1).unwrap(),
                    east.block(b, 0).unwrap(),
                );
                for r in 0..=lattice::BLOCK_STEPS {
                    assert_eq!(a.at(r, lattice::BLOCK_STEPS), z.at(r, 0), "{key} east");
                }
            }
            pairs += 1;
        }
        if let Some(north) = key.north().filter(|n| squares.contains_key(n)) {
            let north = read(&north);
            for b in 0..BLOCKS {
                let (a, z) = (
                    here.block(0, b).unwrap(),
                    north.block(BLOCKS - 1, b).unwrap(),
                );
                for c in 0..=lattice::BLOCK_STEPS {
                    assert_eq!(a.at(0, c), z.at(lattice::BLOCK_STEPS, c), "{key} north");
                }
            }
            pairs += 1;
        }
    }
    assert!(pairs > 10, "{pairs} pairs");
}

#[test]
fn the_world_has_its_features() {
    // Long Point: land 300 km from Westland's coast to its tip, sea either side and past the tip.
    for x in (-139..160).step_by(1) {
        assert!(land([x as f64, 0.0]), "Long Point at {x} km");
    }
    assert!(!land([160.5, 0.0]));
    for x in (-130..=160).step_by(5) {
        assert!(
            !land([x as f64, 15.5]) && !land([x as f64, -15.5]),
            "beside Long Point at {x} km"
        );
    }
    assert!(
        !land([-139.0, 40.0]) && !land([-139.0, -40.0]),
        "Westland's coast is at x = -140"
    );
    // A route from 40 km north of it to 40 km south, near x = 0, must round the tip 160 km east: past the 100 km its
    // first search regions grow by, inside the 200 km of the next.
    assert!((100.0..200.0).contains(&(160.0 - 0.0)));

    // The Gate: 2 km of sea between West Gate (to x = -40) and East Gate (from x = -38), 60 km long.
    for y in (120..=180).step_by(10) {
        let y = y as f64;
        assert!(
            land([-40.1, y]) && land([-37.9, y]),
            "the Gate's shores at {y}"
        );
        for x in [-39.9, -39.0, -38.1] {
            assert!(!land([x, y]), "the Gate at ({x}, {y})");
        }
    }

    // Deep Bay: its basin in one square, its mouth in the next, open to the sea.
    let (basin, mouth) = ([-170.0, -135.0], [-140.0, -105.0]);
    assert!(!land(basin) && !land(mouth) && !land([-155.0, -120.0]));
    assert!(
        land([-150.0, -135.0]) && land([-185.0, -100.0]),
        "Westland round the bay"
    );
    assert_eq!(square_at(basin).name(), "S32W142");
    assert_eq!(square_at(mouth).name(), "S31W142");

    // Ring Reef: every way out of the lagoon crosses land.
    let c = [110.0, -150.0];
    assert!(!land(c));
    for k in 0..64 {
        let a = k as f64 / 64.0 * std::f64::consts::TAU;
        let (s, co) = (a.sin(), a.cos());
        assert!(
            (0..=160).any(|r| land([c[0] + co * r as f64 / 10.0, c[1] + s * r as f64 / 10.0])),
            "lagoon open at {k}"
        );
        assert!(!land([c[0] + co * 17.0, c[1] + s * 17.0]));
    }

    // The Stepping Stones: seven islands with sea between them.
    let stones: Vec<Shape> = LAND
        .iter()
        .filter(|(n, _)| n.starts_with("Stepping"))
        .map(|(_, s)| *s)
        .collect();
    assert_eq!(stones.len(), 7);
    for pair in stones.windows(2) {
        let (Shape::Disc { c: a, .. }, Shape::Disc { c: b, .. }) = (pair[0], pair[1]) else {
            unreachable!()
        };
        assert!(land(a) && land(b) && !land([(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0]));
    }
}
