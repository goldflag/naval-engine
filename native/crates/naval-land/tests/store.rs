//! The store: the manifest is append-only and a conflicting bake is refused and kept nowhere; pins, leases and a
//! world's requests hold against eviction, the store going over its bound for them; the size bound holds the rest,
//! least recently used first, across a reopen; the synthetic release installs whole, twice the same.
use std::fs;
use std::time::{Duration, SystemTime};

use naval_land::format::manifest::Recorded;
use naval_land::format::{Digest, Entry, Manifest, synthetic};
use naval_land::{Answer, Content, MANIFEST_FILE, Store, StoreError, install_synthetic};

use crate::support::{A, B, C, D, OPEN, Scratch, release, square};

fn land(answer: Answer) -> naval_land::Lease {
    match answer {
        Answer::Land(lease) => lease,
        other => panic!("{other:?}"),
    }
}

fn content(
    release: naval_land::format::ReleaseId,
    key: naval_land::format::SquareKey,
    bytes: &[u8],
) -> Content {
    Content {
        release,
        square: key,
        sha256: Digest::of(bytes),
    }
}

#[test]
fn a_manifest_line_never_changes_and_a_conflicting_bake_is_kept_nowhere() {
    let dir = Scratch::new("manifest");
    let store = Store::open(dir.path(), u64::MAX).unwrap();
    let (file, table) = release();
    let id = store.install(&file, &table, None).unwrap();
    assert_eq!(store.install(&file, &table, None).unwrap(), id);
    let manifest_path = store.release_dir(id).join(MANIFEST_FILE);

    let first = square(A, 40, 1);
    let other = square(A, 40, 2);
    assert!(matches!(
        store.put(id, A, &first).unwrap(),
        Recorded::New(_)
    ));
    // The same bake again: nothing changes.
    assert_eq!(store.put(id, A, &first).unwrap(), Recorded::Already);
    // Another result: refused, its bytes nowhere, the manifest as it was.
    let before = fs::read(&manifest_path).unwrap();
    match store.put(id, A, &other) {
        Err(StoreError::Conflict(c)) => {
            assert_eq!(c.recorded, Entry::Data(Digest::of(&first)));
            assert_eq!(c.offered, Entry::Data(Digest::of(&other)));
        }
        other => panic!("{other:?}"),
    }
    assert!(!store.square_path(&content(id, A, &other)).exists());
    assert!(!store.holds(&content(id, A, &other)));
    assert!(matches!(
        store.put_empty(id, A),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(fs::read(&manifest_path).unwrap(), before);
    // An empty line is as fixed as a digest.
    assert!(matches!(store.put_empty(id, B).unwrap(), Recorded::New(_)));
    assert!(matches!(
        store.put(id, B, &square(B, 40, 1)),
        Err(StoreError::Conflict(_))
    ));
    // Squares the release never bakes, bytes of another square, a release not installed.
    assert!(matches!(
        store.put(id, OPEN, &square(OPEN, 40, 1)),
        Err(StoreError::NotCandidate { .. })
    ));
    assert!(matches!(
        store.put(id, C, &first),
        Err(StoreError::NotItsSquare { .. })
    ));
    let mut broken = square(C, 40, 1);
    let last = broken.len() - 1;
    broken[last] ^= 1;
    assert!(matches!(
        store.put(id, C, &broken),
        Err(StoreError::Refused(_))
    ));
    let stranger = naval_land::format::ReleaseId(Digest::of(b"another"));
    assert!(matches!(
        store.put(stranger, A, &first),
        Err(StoreError::UnknownRelease(_))
    ));

    // The file is its lines in the order written.
    let text = fs::read_to_string(&manifest_path).unwrap();
    assert_eq!(
        text,
        format!("N10E020 {}\nN10E021 empty\n", Digest::of(&first))
    );

    // Reopened, the store answers as before.
    drop(store);
    let store = Store::open(dir.path(), u64::MAX).unwrap();
    assert_eq!(
        &**land(store.read(id, A).unwrap()).bytes(),
        first.as_slice()
    );
    assert!(matches!(store.read(id, B).unwrap(), Answer::Sea));
    assert!(matches!(store.read(id, OPEN).unwrap(), Answer::Sea));
    assert!(matches!(
        store.read(id, C).unwrap(),
        Answer::Missing { recorded: None }
    ));

    // A line a writer did not finish was never written: it is cut off before the next.
    {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&manifest_path)
            .unwrap();
        f.write_all(b"N11E020 0a1b").unwrap();
    }
    drop(store);
    let store = Store::open(dir.path(), u64::MAX).unwrap();
    assert!(matches!(
        store.read(id, C).unwrap(),
        Answer::Missing { recorded: None }
    ));
    assert!(matches!(
        store.put(id, C, &square(C, -8, 1)).unwrap(),
        Recorded::New(_)
    ));
    let text = fs::read_to_string(&manifest_path).unwrap();
    let manifest = Manifest::parse(&text).unwrap();
    assert_eq!(manifest.len(), 3);
    assert_eq!(
        manifest.get(C),
        Some(Entry::Data(Digest::of(&square(C, -8, 1))))
    );
}

#[test]
fn pins_and_leases_hold_against_eviction_and_the_bound_holds_the_rest() {
    let dir = Scratch::new("bound");
    let (file, table) = release();
    let bytes = [
        square(A, 40, 1),
        square(B, 41, 1),
        square(C, 42, 1),
        square(D, 43, 1),
    ];
    let size = |k: usize| bytes[k].len() as u64;
    // Room for two squares.
    let bound = size(0) + size(1).max(size(2)).max(size(3));
    let store = Store::open(dir.path(), bound).unwrap();
    let id = store.install(&file, &table, None).unwrap();
    store.settle();
    let keys = [A, B, C, D];
    let [a, b, c, d]: [Content; 4] = std::array::from_fn(|k| content(id, keys[k], &bytes[k]));

    // A save pins A; three squares then make the store too big: the least recently used one nothing holds goes.
    store.pin("save:one", [a]);
    store.put(id, A, &bytes[0]).unwrap();
    store.put(id, B, &bytes[1]).unwrap();
    store.put(id, C, &bytes[2]).unwrap();
    assert!(store.holds(&a) && !store.holds(&b) && store.holds(&c));
    assert!(store.usage().bytes <= bound);
    // Its line stays: it is missing, not unbaked, and the same bake brings it back.
    assert!(matches!(
        store.read(id, B).unwrap(),
        Answer::Missing { recorded: Some(sha) } if sha == b.sha256
    ));

    // A reader leases C: with A pinned and C leased, the only square that can go is the one just kept.
    let lease = land(store.read(id, C).unwrap());
    let twin = lease.clone();
    assert_eq!(store.usage().leased, 1);
    store.put(id, D, &bytes[3]).unwrap();
    assert!(store.holds(&a) && store.holds(&c) && !store.holds(&d));
    drop(lease);
    assert!(store.evict().is_empty());
    drop(twin);
    assert_eq!(store.usage().leased, 0);

    // Everything pinned: nothing goes, and the store stays over its bound rather than lose a pinned square.
    store.pin("world", [b, c, d]);
    store.put(id, B, &bytes[1]).unwrap();
    store.put(id, D, &bytes[3]).unwrap();
    let usage = store.usage();
    assert_eq!((usage.squares, usage.pinned), (4, 4));
    assert!(usage.bytes > bound);
    assert!(store.evict().is_empty());
    // The world ends: its squares go, least recently used first, down to the bound; the save's A stays.
    store.unpin("world");
    assert!(store.holds(&a));
    assert!(store.usage().bytes <= bound);
    assert_eq!(store.usage().squares, 2);
    assert!(store.holds(&d), "the most recently used of B, C, D stays");
    // A pin replaced: the save now pins nothing, and A is just another square.
    store.pin("save:one", []);
    assert!(!store.is_pinned(&a));
}

#[test]
fn a_requested_square_is_kept_from_its_request_and_the_store_goes_over_its_bound_for_it() {
    let dir = Scratch::new("requested");
    let (file, table) = release();
    let bytes = [square(A, 40, 1), square(B, 41, 1), square(C, 42, 1)];
    // Room for one square, which a save pins.
    let bound = bytes[0].len() as u64;
    let store = Store::open(dir.path(), bound).unwrap();
    let id = store.install(&file, &table, None).unwrap();
    store.settle();
    let keys = [A, B, C];
    let [a, b, c]: [Content; 3] = std::array::from_fn(|k| content(id, keys[k], &bytes[k]));
    store.pin("save:one", [a]);
    store.put(id, A, &bytes[0]).unwrap();
    assert_eq!(store.usage().over, 0);

    // A world asks for B before it is baked: kept from the request, so the bake's bytes stay in a full store, over its
    // bound, and the store says so.
    let requests = store.requests();
    requests.request(id, B);
    assert!(matches!(
        store.read(id, B).unwrap(),
        Answer::Missing { recorded: None }
    ));
    store.put(id, B, &bytes[1]).unwrap();
    assert!(store.holds(&b));
    let usage = store.usage();
    assert!(usage.bytes > bound);
    assert_eq!(usage.requested, 1);
    assert!(usage.over > 0);
    assert!(
        store
            .read(id, B)
            .is_ok_and(|answer| matches!(answer, Answer::Land(_)))
    );
    // A square nothing holds is still the one that goes.
    store.put(id, C, &bytes[2]).unwrap();
    assert!(store.holds(&a) && store.holds(&b) && !store.holds(&c));

    // Asked again, nothing changes; another world's requests are its own.
    requests.request(id, B);
    let other = store.requests();
    other.request(id, B);
    drop(requests);
    assert!(store.evict().is_empty(), "the other world still asks for B");
    // Dropped: B is just another square, and the store comes back within its bound.
    drop(other);
    assert_eq!(store.usage().requested, 0);
    assert_eq!(store.evict(), vec![b]);
    assert!(store.usage().bytes <= bound);
    let over = store.usage().over;
    assert!(store.evict().is_empty());
    assert_eq!(
        store.usage().over,
        over,
        "a pass within the bound is not counted"
    );
}

#[test]
fn a_folder_has_one_store_while_it_or_a_lease_of_it_lives() {
    let dir = Scratch::new("one-store");
    let (file, table) = release();
    let bytes = square(A, 40, 1);
    let store = Store::open(dir.path(), u64::MAX).unwrap();
    let id = store.install(&file, &table, None).unwrap();
    store.put(id, A, &bytes).unwrap();
    let a = content(id, A, &bytes);
    store.pin("save", [a]);
    // A second store would know nothing of the first's pins and leases: it is refused, here as in another process.
    match Store::open(dir.path(), 0) {
        Err(e @ StoreError::InUse { .. }) => {
            assert!(e.to_string().contains("in use by another game"))
        }
        other => panic!("{:?}", other.map(|s| s.usage())),
    }
    // A lease that outlives its store keeps the folder.
    let lease = land(store.read(id, A).unwrap());
    drop(store);
    assert!(matches!(
        Store::open(dir.path(), 0),
        Err(StoreError::InUse { .. })
    ));
    drop(lease);
    let store = Store::open(dir.path(), 0).unwrap();
    assert!(store.holds(&a));
}

#[test]
fn nothing_is_evicted_before_the_store_settles() {
    let dir = Scratch::new("settle");
    let (file, table) = release();
    let bytes = [square(A, 40, 1), square(B, 41, 1), square(C, 42, 1)];
    let bound = bytes[1].len() as u64;
    let store = Store::open(dir.path(), bound).unwrap();
    let id = store.install(&file, &table, None).unwrap();
    for (k, key) in [A, B, C].into_iter().enumerate() {
        store.put(id, key, &bytes[k]).unwrap();
    }
    assert_eq!(store.usage().squares, 3);
    assert!(store.evict().is_empty());
    drop(store);
    // Reopened over its bound, it keeps everything until its owner has read the saves' pins and settles it.
    let store = Store::open(dir.path(), bound).unwrap();
    assert_eq!(store.usage().squares, 3);
    let b = content(id, B, &bytes[1]);
    store.pin("save:b", [b]);
    assert_eq!(store.usage().squares, 3);
    assert_eq!(store.settle().len(), 2);
    assert!(store.holds(&b));
    assert_eq!(store.usage().bytes, bound);
}

#[test]
fn recency_outlives_the_session() {
    let dir = Scratch::new("recency");
    let (file, table) = release();
    let bytes = [square(A, 40, 1), square(B, 41, 1), square(C, 42, 1)];
    let id = {
        let store = Store::open(dir.path(), u64::MAX).unwrap();
        let id = store.install(&file, &table, None).unwrap();
        for (k, key) in [A, B, C].into_iter().enumerate() {
            store.put(id, key, &bytes[k]).unwrap();
        }
        id
    };
    // Last used: B long ago, C an hour ago, A just now.
    let now = SystemTime::now();
    for (k, key, age) in [(0, A, 0), (1, B, 86_400), (2, C, 3_600)] {
        let path = dir
            .path()
            .join(id.to_string())
            .join(key.name())
            .join(format!("{}.nsq", Digest::of(&bytes[k])));
        let f = fs::OpenOptions::new().write(true).open(path).unwrap();
        f.set_modified(now - Duration::from_secs(age)).unwrap();
    }
    let bound = (bytes[0].len() + bytes[2].len()) as u64;
    let store = Store::open(dir.path(), bound).unwrap();
    assert_eq!(store.usage().squares, 3);
    // The save's pins are read; then the store settles to its bound, the longest unused first.
    store.settle();
    assert!(store.holds(&content(id, A, &bytes[0])));
    assert!(!store.holds(&content(id, B, &bytes[1])));
    assert!(store.holds(&content(id, C, &bytes[2])));
}

#[test]
fn the_synthetic_release_installs_whole_and_twice_the_same() {
    let (one, two) = (Scratch::new("synthetic-one"), Scratch::new("synthetic-two"));
    let made = synthetic::release();
    let store = Store::open(one.path(), u64::MAX).unwrap();
    let id = install_synthetic(&store).unwrap();
    assert_eq!(id, made.id);
    // Installing again records nothing new.
    let manifest = fs::read(store.release_dir(id).join(MANIFEST_FILE)).unwrap();
    install_synthetic(&store).unwrap();
    assert_eq!(
        fs::read(store.release_dir(id).join(MANIFEST_FILE)).unwrap(),
        manifest
    );
    assert_eq!(String::from_utf8(manifest).unwrap(), made.manifest);
    for (key, bytes) in &made.squares {
        let lease = land(store.read(id, *key).unwrap());
        assert_eq!(&**lease.bytes(), bytes.as_slice());
        assert_eq!(lease.square().unwrap().key(), *key);
    }
    let empty = Manifest::parse(&made.manifest)
        .unwrap()
        .iter()
        .find(|(_, e)| *e == Entry::Empty)
        .unwrap()
        .0;
    assert!(matches!(store.read(id, empty).unwrap(), Answer::Sea));
    // A second store made from a second run holds the same files, byte for byte.
    let other = Store::open(two.path(), u64::MAX).unwrap();
    install_synthetic(&other).unwrap();
    assert_eq!(listing(one.path()), listing(two.path()));
}

/// Every file under a folder with its bytes, by path.
fn listing(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}
