//! `land-v1` on real data (ignored: they read the pinned snapshot and the squares baked on this machine, which the
//! repository does not hold). The open-ocean plan, step C §7.2 (c): each threshold on real squares, either side of it.
//!
//! - `NAVAL_LAND_SNAPSHOT` (the snapshot's zip) and `NAVAL_LAND_TABLE` (the release's square table): the table is
//!   made again from the snapshot and must be the same bytes; every real square whose coastline lies within a
//!   kilometre of the 65 km margin, measured apart (every edge sampled every 25 m), is a candidate inside it and ocean
//!   outside it.
//! - `NAVAL_LAND_SQUARES` (a folder of a release's final squares, `<square>/<sha256>.nsq` as the store keeps them):
//!   for every candidate within 60 km of a data square's final land, the keep rule's reading of that window flips at
//!   the measured distance, a metre either side, and holds at 40 km as the distance says.
//!
//! `bun run rust:test -- --native -p naval-region-bake release_real -- --ignored --nocapture`
use std::collections::BTreeMap;
use std::path::PathBuf;

use naval_land_format::{Class, FinalSquare, SquareKey, SquareTable, table::polar};
use naval_region_bake::finish::keep::{self, CANDIDATE_M, KEEP_M};
use naval_region_bake::finish::lattice::Region;
use naval_region_bake::release::snapshot::{self, WaterRecord};
use naval_region_bake::release::table::{M_PER_DEGREE, TableBuilder, artificial, distance_m};

fn env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The least distance from each square to the coastline, for squares within `reach` of it, every coast edge sampled
/// every `step` metres.
fn coast_distances(records: &[WaterRecord], reach: f64, step: f64) -> BTreeMap<SquareKey, f64> {
    let mut best: BTreeMap<SquareKey, f64> = BTreeMap::new();
    let mut point = |lat: f64, lon: f64| {
        let rows = reach / M_PER_DEGREE;
        for row in ((lat - rows).floor() as i32).max(-90)..=((lat + rows).floor() as i32).min(89) {
            let cos = (lat.clamp(f64::from(row), f64::from(row) + 1.0))
                .to_radians()
                .cos()
                .max(1e-3);
            let across = (reach / (M_PER_DEGREE * cos)).min(180.0);
            for m in (lon - across).floor() as i32..=(lon + across).floor() as i32 {
                let key = SquareKey {
                    lat: row,
                    lon: (m + 180).rem_euclid(360) - 180,
                };
                if polar(key) {
                    continue;
                }
                let d = distance_m(key, lat, lon);
                if d <= reach {
                    let e = best.entry(key).or_insert(f64::INFINITY);
                    *e = e.min(d);
                }
            }
        }
    };
    for r in records {
        for ring in &r.rings {
            for e in 0..ring.len() {
                let (a, b) = (ring[e], ring[(e + 1) % ring.len()]);
                if artificial(a, b) {
                    continue;
                }
                let c = (0.5 * (a[0] + b[0])).to_radians().cos();
                let length =
                    M_PER_DEGREE * ((b[0] - a[0]).powi(2) + ((b[1] - a[1]) * c).powi(2)).sqrt();
                let n = (length / step).ceil().max(1.0) as usize;
                for k in 0..=n {
                    let t = k as f64 / n as f64;
                    point(a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t);
                }
            }
        }
    }
    best
}

#[test]
#[ignore = "reads the pinned snapshot (NAVAL_LAND_SNAPSHOT) and the release's table (NAVAL_LAND_TABLE)"]
fn the_table_is_the_snapshots_and_its_65_km_margin_holds_on_real_squares() {
    let (Some(zip), Some(table)) = (env("NAVAL_LAND_SNAPSHOT"), env("NAVAL_LAND_TABLE")) else {
        eprintln!("NAVAL_LAND_SNAPSHOT and NAVAL_LAND_TABLE are not set: nothing to check");
        return;
    };
    let pinned = SquareTable::decode(&std::fs::read(&table).unwrap()).unwrap();
    let mut file = std::fs::File::open(&zip).unwrap();
    let mut builder = TableBuilder::default();
    let mut records = vec![];
    snapshot::for_each(&mut file, |r| {
        builder.add(&r);
        records.push(r);
        Ok(())
    })
    .unwrap();
    let made = builder.finish();
    assert_eq!(
        made.encode(),
        pinned.encode(),
        "the table made again is not the release's"
    );
    // A finer measure: every square within 66 km of the coastline, its least distance.
    let near = coast_distances(&records, CANDIDATE_M + 1_000.0, 25.0);
    let (mut inside, mut outside) = (vec![], vec![]);
    for (&key, &d) in &near {
        if d <= CANDIDATE_M - 100.0 {
            assert_eq!(
                pinned.class(key),
                Class::Candidate,
                "{key}: coastline {d:.0} m away"
            );
            if d > CANDIDATE_M - 1_000.0 {
                inside.push((key, d));
            }
        } else if d > CANDIDATE_M + 100.0 && builder.middle_covered(key) {
            assert_eq!(
                pinned.class(key),
                Class::Ocean,
                "{key}: coastline {d:.0} m away"
            );
            outside.push((key, d));
        }
    }
    inside.sort_by(|a, b| b.1.total_cmp(&a.1));
    outside.sort_by(|a, b| a.1.total_cmp(&b.1));
    eprintln!(
        "65 km margin: {} squares within a kilometre inside (nearest {:?}), {} within a kilometre outside (nearest {:?})",
        inside.len(),
        inside.first(),
        outside.len(),
        outside.first()
    );
    assert!(
        !inside.is_empty() && !outside.is_empty(),
        "real squares on both sides of the margin"
    );
}

#[test]
#[ignore = "reads final squares baked on this machine (NAVAL_LAND_SQUARES)"]
fn the_40_km_keep_flips_at_the_measured_distance_on_real_windows() {
    let Some(dir) = env("NAVAL_LAND_SQUARES") else {
        eprintln!("NAVAL_LAND_SQUARES is not set: nothing to check");
        return;
    };
    let mut cases = 0;
    let (mut nearest_in, mut nearest_out) = (f64::NEG_INFINITY, f64::INFINITY);
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let Some(of) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(SquareKey::parse)
        else {
            continue;
        };
        for file in std::fs::read_dir(&path).unwrap() {
            let bytes = std::fs::read(file.unwrap().path()).unwrap();
            let square = FinalSquare::read(bytes).unwrap();
            assert_eq!(square.key(), of);
            let window = square.window().unwrap();
            if !window.land() {
                continue;
            }
            let w = Region::window(of);
            for key in keep::squares_within(of, 60_000.0) {
                let mut d = f64::INFINITY;
                for r in 0..w.rows {
                    for c in 0..w.columns {
                        if window.at(r, c) > 0 {
                            d = d.min(keep::to_square_m(key, w.i(r), w.j(c)));
                        }
                    }
                }
                if !d.is_finite() {
                    continue;
                }
                assert!(
                    keep::land_within(key, of, &window, d + 1.0),
                    "{key} from {of}: {d}"
                );
                assert!(
                    !keep::land_within(key, of, &window, d - 1.0),
                    "{key} from {of}: {d}"
                );
                assert_eq!(
                    keep::land_within(key, of, &window, KEEP_M),
                    d <= KEEP_M,
                    "{key} from {of}: {d}"
                );
                if d <= KEEP_M {
                    nearest_in = nearest_in.max(d);
                } else {
                    nearest_out = nearest_out.min(d);
                }
                cases += 1;
            }
        }
    }
    eprintln!(
        "40 km keep: {cases} neighbours of real windows; the farthest kept {nearest_in:.0} m, the nearest not {nearest_out:.0} m"
    );
    assert!(cases > 0);
}

#[test]
fn the_committed_release_is_land_v1() {
    use naval_land_format::ReleaseFile;
    use naval_region_bake::release::{LAND_V1, NAME};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/land");
    let bytes = std::fs::read(root.join(format!("{NAME}.release.json"))).unwrap();
    let release = ReleaseFile::parse(&bytes).unwrap();
    assert_eq!(
        release.id.to_string(),
        LAND_V1,
        "the release file is not land-v1's: a changed release is a new one"
    );
    assert_eq!(
        release.file.bake,
        naval_region_bake::finish::bake_spec_for(1),
        "the bake is not the one land-v1 was made for"
    );
    assert_eq!(
        naval_region_bake::finish::version_of(&release.file.bake),
        Some(1)
    );
    let table = std::fs::read(root.join(format!("{NAME}.squares.nlt"))).unwrap();
    assert!(release.file.square_table.matches(&table));
    let table = SquareTable::decode(&table).unwrap();
    for (key, class) in [
        (SquareKey { lat: 0, lon: -140 }, Class::Ocean),
        (SquareKey { lat: 36, lon: -122 }, Class::Candidate),
        (SquareKey { lat: -17, lon: 179 }, Class::Candidate),
        (
            SquareKey {
                lat: -17,
                lon: -180,
            },
            Class::Candidate,
        ),
        (SquareKey { lat: 25, lon: 10 }, Class::Candidate),
        (SquareKey { lat: 89, lon: 0 }, Class::Polar),
    ] {
        assert_eq!(table.class(key), class, "{key}");
    }
    let snapshot = release.file.snapshot.unwrap();
    assert_eq!(
        (snapshot.date.as_str(), snapshot.bytes),
        ("2026-10-14", 906_312_607)
    );
}

#[test]
fn the_committed_release_is_land_v2_made_for_the_second_bake_on_land_v1s_snapshot_and_table() {
    use naval_land_format::ReleaseFile;
    use naval_region_bake::release::{LAND_V2, name};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../assets/land");
    let read = |file: String| std::fs::read(root.join(file)).unwrap();
    let bytes = read(format!("{}.release.json", name(2)));
    let release = ReleaseFile::parse(&bytes).unwrap();
    assert_eq!(
        release.id.to_string(),
        LAND_V2,
        "the release file is not land-v2's: a changed release is a new one"
    );
    assert_eq!(release.file.name, "land-v2");
    assert_eq!(
        release.file.bake,
        naval_region_bake::finish::bake_spec_for(2)
    );
    assert_eq!(
        naval_region_bake::finish::version_of(&release.file.bake),
        Some(2)
    );
    let table = read(format!("{}.squares.nlt", name(2)));
    assert!(release.file.square_table.matches(&table));
    // Its coast is land-v1's: the same snapshot, so the same square table.
    let first = ReleaseFile::parse(&read("land-v1.release.json".into())).unwrap();
    assert_eq!(release.file.snapshot, first.file.snapshot);
    assert_eq!(table, read("land-v1.squares.nlt".into()));
    assert_ne!(
        release.file.sources, first.file.sources,
        "its manifest pins the sea floor too"
    );
}
