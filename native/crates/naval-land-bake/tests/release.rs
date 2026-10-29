//! A land release's pinned sources and its bake into the store (`naval_land_bake::release`; the open-ocean plan,
//! step C, C1 and §7.2 (d)), over a mocked pair of buckets serving the fixtures' DEM (`naval_region_bake::release::
//! fixtures`): Terrarium PNGs and GLO-30 tiled TIFFs, each object with an ETag.
//!
//! - Every read is pinned: it sends the manifest's ETag, an object that changed fails the bake, an unreadable GLO-30
//!   cell fails it with no Terrarium in its place, an absent cell is sea and never asked for, and an object the
//!   manifest does not list is refused before any request.
//! - The date line: squares at 179° E and 180° W, Terrarium and GLO-30, through the whole fetch and interpolation path,
//!   equal the reference resampled in unwrapped longitude.
//! - A square baked end to end into a store, and again: the same manifest line, the bytes the store holds.
//! - A release made for another bake is refused; the snapshot is read through the handle that was hashed, and a pass
//!   over one written to fails.
//! - The second bake (`land-v2`): its sea floor read from the pinned GEBCO object (the fixture's, every read by its
//!   ETag; a changed one fails the bake), every candidate kept, its land holding the first bake's, its edge with open
//!   sea the open floor; a release for it whose manifest pins no sea floor is refused.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use naval_land::Store;
use naval_land::format::{Digest, ReleaseFile, SquareKey};
use naval_land_bake::release::{ReleaseSources, SourceCache, bake_square, snapshot_path};
use naval_region_bake::glo30;
use naval_region_bake::release::fixtures;
use naval_region_bake::release::gebco;
use naval_region_bake::release::pieces::Splitter;
use naval_region_bake::release::sources::{Object, Reads, SourceManifest};

use crate::support::*;

#[test]
fn every_read_is_pinned_and_a_changed_or_unread_object_fails_the_bake() {
    // Terrarium: one tile changed in its bucket.
    let key = k(-17, 179);
    let m = manifest(&[key], &[]);
    let snapshot = fixtures::snapshot(&[]);
    let rec = record(&[key], &m, &snapshot);
    let first = *m.terrarium.keys().nth(3).unwrap();
    let changed = naval_region_bake::release::sources::terrarium_url(first);
    let buckets = Arc::new(Buckets {
        changed: Some(changed.clone()),
        ..Default::default()
    });
    let dir = temp("changed");
    let at = sources(&dir, &rec, buckets.clone(), &snapshot);
    let e = at.surveyed(key).unwrap_err().to_string();
    assert!(e.contains("changed since the release was made"), "{e}");
    for (url, _, tag) in buckets.asked.lock().unwrap().iter() {
        let tile = parse_tile(url).expect("Terrarium only");
        assert_eq!(
            tag.as_deref(),
            Some(m.terrarium[&tile].etag.as_str()),
            "{url}"
        );
    }

    // GLO-30: unreadable, and no Terrarium asked for in its place.
    let key = k(86, 10);
    let m = manifest(&[key], &[]);
    let rec = record(&[key], &m, &snapshot);
    let buckets = Arc::new(Buckets {
        broken_glo30: true,
        ..Default::default()
    });
    let dir = temp("broken");
    let at = sources(&dir, &rec, buckets.clone(), &snapshot);
    assert!(at.surveyed(key).is_err());
    let asked = buckets.asked.lock().unwrap();
    assert!(!asked.is_empty());
    assert!(
        asked
            .iter()
            .all(|(url, _, tag)| parse_cell(url).is_some() && tag.is_some()),
        "{asked:?}"
    );

    // An absent cell is sea and never asked for; an object the manifest lacks is refused before any request.
    let key = k(86, 20);
    let reads = Reads::of(key);
    let gone = reads.cells()[0];
    let m = manifest(&[key], &[gone]);
    let rec = record(&[key], &m, &snapshot);
    let buckets = Arc::new(Buckets::default());
    let dir = temp("absent");
    let at = sources(&dir, &rec, buckets.clone(), &snapshot);
    at.surveyed(key).unwrap();
    assert!(
        buckets
            .asked
            .lock()
            .unwrap()
            .iter()
            .all(|(url, ..)| parse_cell(url) != Some(gone))
    );
    let mut short = manifest(&[key], &[]);
    short.glo30.remove(&gone);
    let rec = record(&[key], &short, &snapshot);
    let buckets = Arc::new(Buckets::default());
    let at = sources(&temp("short"), &rec, buckets.clone(), &snapshot);
    let e = at.surveyed(key).unwrap_err().to_string();
    assert!(e.contains("not in the release's source manifest"), "{e}");
    assert!(buckets.asked.lock().unwrap().is_empty());
}

#[test]
fn the_date_line_path_is_the_unwrapped_resample() {
    let keys = [k(-17, 179), k(-17, -180), k(86, 179), k(86, -180)];
    let m = manifest(&keys, &[]);
    let snapshot = fixtures::snapshot(&[]);
    let rec = record(&keys, &m, &snapshot);
    let buckets = Arc::new(Buckets::default());
    let at = sources(&temp("dateline"), &rec, buckets.clone(), &snapshot);
    let q = naval_region_bake::square::HEIGHT_QUANTUM as f32;
    for key in keys {
        let got = at.surveyed(key).unwrap();
        let want = fixtures::reference(&Reads::of(key));
        let worst = got
            .heights()
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst <= q, "{key}: {worst} m");
    }
    // Every object was asked for by its wrapped name, on both sides of ±180°.
    let asked = buckets.asked.lock().unwrap();
    let (mut tiles, mut cells) = (BTreeMap::new(), BTreeMap::new());
    for (url, ..) in asked.iter() {
        if let Some((z, x, _)) = parse_tile(url) {
            assert!((0..1 << z).contains(&x), "{url}");
            *tiles.entry(x == (1 << z) - 1 || x == 0).or_insert(0) += 1;
        } else if let Some((_, lon)) = parse_cell(url) {
            assert!((-180..180).contains(&lon), "{url}");
            *cells.entry(lon).or_insert(0) += 1;
        }
    }
    assert!(tiles.get(&true).copied().unwrap_or(0) > 0, "{tiles:?}");
    assert!(
        cells.contains_key(&-180) && cells.contains_key(&179),
        "{cells:?}"
    );
    let _ = glo30::HOST;
}

#[test]
fn a_square_bakes_into_the_store_and_again_to_the_same_line() {
    // An island across ±180° at 16.5° S in a sea that covers the squares about it, the snapshot's way: split at ±180°.
    let ring =
        |lon0: f64, lon1: f64| vec![[-18.6, lon0], [-18.6, lon1], [-14.4, lon1], [-14.4, lon0]];
    let island =
        |lon0: f64, lon1: f64| vec![[-16.6, lon0], [-16.6, lon1], [-16.4, lon1], [-16.4, lon0]];
    let snapshot = fixtures::snapshot(&[
        vec![ring(177.5, 180.0), island(179.9, 180.0)],
        vec![ring(-180.0, -177.5), island(-180.0, -179.95)],
    ]);
    let key = k(-17, 179);
    let candidates: Vec<SquareKey> = (-18..=-16)
        .flat_map(|lat| [178, 179, -180].map(|lon| k(lat, lon)))
        .collect();
    let m = manifest(&candidates, &[]);
    let rec = record(&candidates, &m, &snapshot);
    let dir = temp("store");
    let buckets = Arc::new(Buckets::default());
    let at = sources(&dir, &rec, buckets.clone(), &snapshot);
    let store = Store::open(dir.join("land"), u64::MAX).unwrap();
    let id = store
        .install(&rec.release, &rec.table, rec.sources.as_deref())
        .unwrap();
    assert_eq!(id, at.id);
    let cancel = AtomicBool::new(false);
    let first = bake_square(&store, &at, key, 3, &cancel).unwrap();
    assert!(first.new && first.bytes.is_some(), "{first:?}");
    assert!(
        first.report.land > 0,
        "the island is land: {:?}",
        first.report
    );
    // Again, from a cold process's caches and from the warm ones: the line the store holds.
    let warm = bake_square(&store, &at, key, 3, &cancel).unwrap();
    assert_eq!((warm.line.as_str(), warm.new), (first.line.as_str(), false));
    let fresh = sources(
        &temp("store-cold"),
        &rec,
        Arc::new(Buckets::default()),
        &snapshot,
    );
    let cold = bake_square(&store, &fresh, key, 1, &cancel).unwrap();
    assert_eq!(cold.line, first.line);
    // The ocean square is never baked; a read of every object sent its ETag.
    assert!(bake_square(&store, &at, k(-20, 179), 1, &cancel).is_err());
    assert!(
        buckets
            .asked
            .lock()
            .unwrap()
            .iter()
            .all(|(_, _, tag)| tag.is_some())
    );
    let stats = at.stats.lock().unwrap().clone();
    assert_eq!(stats.snapshot_passes, 1, "{stats:?}");
}

#[test]
fn a_release_for_another_bake_or_a_snapshot_written_to_is_refused() {
    let key = k(-17, 179);
    let m = manifest(&[key, k(-17, 178)], &[]);
    let snapshot = fixtures::snapshot(&[]);
    // A release made for another bake is never baked by this one.
    let mut other = record(&[key], &m, &snapshot);
    let mut file = ReleaseFile::parse(&other.release).unwrap().file;
    file.bake.version += 1;
    other.release = file.encode();
    let dir = temp("other-bake");
    let path = snapshot_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &snapshot).unwrap();
    let cache = Arc::new(SourceCache::open(dir.join("cache"), u64::MAX).unwrap());
    let e = ReleaseSources::open(&other, path, cache, Arc::new(Buckets::default()))
        .err()
        .expect("refused")
        .to_string();
    assert!(e.contains("another bake"), "{e}");
    // The snapshot is read through the handle that was hashed: one replaced on disk is not read in its place, and one
    // written to fails the next pass.
    let rec = record(&[key, k(-17, 178)], &m, &snapshot);
    let dir = temp("snapshot-held");
    let at = sources(&dir, &rec, Arc::new(Buckets::default()), &snapshot);
    at.check_snapshot().unwrap();
    let path = snapshot_path(&dir);
    let other = path.with_extension("other");
    std::fs::write(
        &other,
        fixtures::snapshot(&[vec![vec![
            [-16.6, 179.2],
            [-16.6, 179.8],
            [-16.4, 179.8],
            [-16.4, 179.2],
        ]]]),
    )
    .unwrap();
    std::fs::rename(&other, &path).unwrap();
    // The held snapshot has no water; the one now on disk has a sea at the square.
    assert!(at.pieces(&[key]).unwrap()[&key].polygons.is_empty());
    let mut fresh = Splitter::new([key]);
    naval_region_bake::release::snapshot::for_each(&mut std::fs::File::open(&path).unwrap(), |r| {
        fresh.add(&r);
        Ok(())
    })
    .unwrap();
    assert!(
        !fresh.finish(Digest::of(b"")).remove(0).polygons.is_empty(),
        "the replacement has water at the square"
    );
    std::fs::write(&path, b"").unwrap();
    let held = at.check_snapshot().unwrap();
    assert_eq!(held.bytes, snapshot.len() as u64);
    // Written to in place: the held file itself moved.
    let dir = temp("snapshot-written");
    let at = sources(&dir, &rec, Arc::new(Buckets::default()), &snapshot);
    at.check_snapshot().unwrap();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(snapshot_path(&dir))
            .unwrap();
        f.write_all(b"more").unwrap();
    }
    let Err(e) = at.pieces(&[key]) else {
        panic!("a pass over a snapshot written to")
    };
    let e = e.to_string();
    assert!(e.contains("changed on disk"), "{e}");
}

/// A host holding files by name, counting what it is asked for.
struct Shelf(Vec<(String, Vec<u8>)>, std::sync::atomic::AtomicUsize);
impl naval_land_bake::hosted::Host for Shelf {
    fn get(
        &self,
        name: &str,
        out: &mut dyn std::io::Write,
    ) -> Result<(), naval_land_bake::hosted::Unfetched> {
        use naval_land_bake::hosted::Unfetched;
        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (_, bytes) = self
            .0
            .iter()
            .find(|(n, _)| n == name)
            .ok_or_else(|| Unfetched::Refused(format!("{name} is not hosted")))?;
        out.write_all(bytes)
            .map_err(|e| Unfetched::Refused(e.to_string()))
    }
}

/// A fresh install has no snapshot: the first bake fetches it from where the sources are hosted, by its pin, once,
/// and keeps it where the bakes read it. A file that is not the one pinned is refused and not kept; without a host the
/// bake is refused as before.
#[test]
fn a_missing_snapshot_is_fetched_by_its_pin_at_the_first_bake() {
    use naval_land_bake::hosted::{self, REFUSED};
    let key = k(-17, 179);
    let m = manifest(&[key], &[]);
    let snapshot = fixtures::snapshot(&[]);
    let rec = record(&[key], &m, &snapshot);
    let name = hosted::hosted_name(&Digest::of(&snapshot), "zip");
    let open = |dir: &std::path::Path| {
        let cache = Arc::new(SourceCache::open(dir.join("cache"), u64::MAX).unwrap());
        ReleaseSources::open(
            &rec,
            snapshot_path(dir),
            cache,
            Arc::new(Buckets::default()),
        )
        .unwrap()
    };
    // No host: refused, as on a machine that never had the snapshot.
    let dir = temp("snapshot-unhosted");
    assert!(open(&dir).check_snapshot().is_err());
    // Another file under the pinned name: refused, nothing kept, and tried again by the next bake.
    let forged = Arc::new(Shelf(
        vec![(name.clone(), b"PK".to_vec())],
        Default::default(),
    ));
    let at = open(&dir).with_host(forged.clone());
    let e = at.check_snapshot().err().expect("refused").to_string();
    assert_eq!(e, REFUSED);
    assert!(!snapshot_path(&dir).exists());
    assert!(at.check_snapshot().is_err());
    assert_eq!(forged.1.load(std::sync::atomic::Ordering::Relaxed), 2);
    // The file pinned: fetched once, kept, and baked from.
    let shelf = Arc::new(Shelf(vec![(name, snapshot.clone())], Default::default()));
    let at = open(&dir).with_host(shelf.clone());
    let held = at.check_snapshot().unwrap();
    assert_eq!(held.bytes, snapshot.len() as u64);
    at.check_snapshot().unwrap();
    assert_eq!(std::fs::read(snapshot_path(&dir)).unwrap(), snapshot);
    assert!(at.pieces(&[key]).is_ok());
    let again = open(&dir).with_host(shelf.clone());
    again.check_snapshot().unwrap();
    assert_eq!(shelf.1.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn the_second_bake_reads_its_pinned_floor_keeps_every_candidate_and_keeps_the_first_bakes_land() {
    let (snapshot, candidates) = island();
    let mut m = manifest(&candidates, &[]);
    m.seabed = Some(Object {
        etag: GEBCO_ETAG.into(),
        bytes: fixtures::gebco_bytes(),
    });
    let two = record_for(2, &candidates, &m, &snapshot);
    let cancel = AtomicBool::new(false);
    let dir = temp("second-bake");
    let buckets = Arc::new(Buckets::default());
    let at = sources(&dir, &two, buckets.clone(), &snapshot);
    assert_eq!(at.version, 2);
    let store = Store::open(dir.join("land"), u64::MAX).unwrap();
    store
        .install(&two.release, &two.table, two.sources.as_deref())
        .unwrap();
    // The island's square: its floor read from GEBCO, every read by its ETag.
    let key = k(-17, 179);
    let first = bake_square(&store, &at, key, 3, &cancel).unwrap();
    assert!(first.report.floor > 0 && first.bytes.is_some(), "{first:?}");
    let asked = buckets.asked.lock().unwrap().clone();
    let floor_reads: Vec<&Asked> = asked.iter().filter(|a| a.0 == gebco::url()).collect();
    assert!(!floor_reads.is_empty());
    assert!(
        floor_reads
            .iter()
            .all(|a| a.2.as_deref() == Some(GEBCO_ETAG))
    );
    // Its land holds the first bake's (each height above the coast's 6 m lift to the quantum); its sea is the floor's,
    // deeper than the shelf in places.
    let one = record(&candidates, &m_without_floor(&m), &snapshot);
    let at1 = sources(
        &temp("second-bake-v1"),
        &one,
        Arc::new(Buckets::default()),
        &snapshot,
    );
    let (w1, w2) = (
        at1.window(key, 1, &cancel).unwrap().window,
        at.window(key, 1, &cancel).unwrap().window,
    );
    for (p, q) in w1.quanta.iter().zip(&w2.quanta) {
        if *p > 0 {
            assert!(*q > 0);
        }
        if *p > 24 {
            assert_eq!(p, q);
        }
    }
    assert!(
        w2.quanta.iter().any(|&q| q < -640),
        "a floor deeper than the shelf's 160 m"
    );
    // A square with no land within 40 km is data (the first bake would write it empty), and its edge with the ocean
    // square south of it is the open floor.
    let bare = k(-18, 178);
    let baked = bake_square(&store, &at, bare, 1, &cancel).unwrap();
    assert!(baked.bytes.is_some(), "{baked:?}");
    let empty_in_one = {
        let s1 = Store::open(temp("second-bake-v1-store").join("land"), u64::MAX).unwrap();
        s1.install(&one.release, &one.table, one.sources.as_deref())
            .unwrap();
        bake_square(&s1, &at1, bare, 1, &cancel).unwrap()
    };
    assert!(
        empty_in_one.bytes.is_none(),
        "the first bake writes it empty"
    );
    let window = at.window(bare, 1, &cancel).unwrap().window;
    let last = naval_land::format::lattice::WINDOW - 1;
    assert!(
        (0..=last).all(|c| window.at(last, c) == -640),
        "the south edge is the open floor"
    );
    // Baked again cold: the same line.
    let fresh = sources(
        &temp("second-bake-cold"),
        &two,
        Arc::new(Buckets::default()),
        &snapshot,
    );
    assert_eq!(
        bake_square(&store, &fresh, key, 1, &cancel).unwrap().line,
        first.line
    );
    // A changed GEBCO object fails the bake; a release for the second bake whose manifest pins no floor is refused.
    let changed = Arc::new(Buckets {
        changed: Some(gebco::url()),
        ..Default::default()
    });
    let at = sources(&temp("second-bake-changed"), &two, changed, &snapshot);
    let e = at.window(key, 1, &cancel).expect_err("refused").to_string();
    assert!(e.contains("changed since the release was made"), "{e}");
    let bare_two = record_for(2, &candidates, &m_without_floor(&m), &snapshot);
    let dir = temp("second-bake-no-floor");
    let path = snapshot_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &snapshot).unwrap();
    let cache = Arc::new(SourceCache::open(dir.join("cache"), u64::MAX).unwrap());
    let e = ReleaseSources::open(&bare_two, path, cache, Arc::new(Buckets::default()))
        .err()
        .expect("refused")
        .to_string();
    assert!(e.contains("pins no sea floor"), "{e}");
}

fn m_without_floor(m: &SourceManifest) -> SourceManifest {
    SourceManifest {
        seabed: None,
        ..m.clone()
    }
}
