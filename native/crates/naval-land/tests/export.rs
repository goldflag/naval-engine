//! Export: a closure's land goes whole into one file and comes out whole on another computer; a square not on this
//! computer is named in words; a bundle that disagrees with the store it is imported into is refused whole.
use naval_land::format::{Entry, SquareKey, synthetic};
use naval_land::{Answer, Store, StoreError, install_synthetic};

use crate::support::{A, B, C, D, OPEN, Scratch, release, square};

#[test]
fn a_closure_travels_whole() {
    let (here, there) = (Scratch::new("export-here"), Scratch::new("export-there"));
    let store = Store::open(here.path(), u64::MAX).unwrap();
    let id = install_synthetic(&store).unwrap();
    let made = synthetic::release();
    let manifest = store.manifest(id).unwrap();
    let data: Vec<SquareKey> = made.squares.keys().copied().take(3).collect();
    let empty = manifest.iter().find(|(_, e)| *e == Entry::Empty).unwrap().0;
    let ocean = SquareKey { lat: 0, lon: 0 };
    let closure = store
        .closure(id, data.iter().copied().chain([empty, ocean]))
        .unwrap();
    // The ocean square is the table's: no line.
    assert_eq!(closure.squares.len(), 4);
    assert_eq!(closure.squares[&empty], Entry::Empty);
    assert_eq!(closure.contents().count(), 3);
    let bundle = store.export(&closure).unwrap();
    assert_eq!(
        store.export(&closure).unwrap(),
        bundle,
        "the same closure, the same file"
    );

    let other = Store::open(there.path(), u64::MAX).unwrap();
    assert_eq!(other.import(&bundle).unwrap(), closure);
    for key in &data {
        match other.read(id, *key).unwrap() {
            Answer::Land(lease) => assert_eq!(&**lease.bytes(), made.squares[key].as_slice()),
            answer => panic!("{key}: {answer:?}"),
        }
    }
    assert!(matches!(other.read(id, empty).unwrap(), Answer::Sea));
    assert!(matches!(other.read(id, ocean).unwrap(), Answer::Sea));
    // Squares outside the closure are not there: never baked on that computer.
    let outside = *made.squares.keys().last().unwrap();
    assert!(matches!(
        other.read(id, outside).unwrap(),
        Answer::Missing { recorded: None }
    ));
    // Importing again changes nothing.
    assert_eq!(other.import(&bundle).unwrap(), closure);

    // A bundle with a byte changed is refused.
    let mut tampered = bundle.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    let third = Scratch::new("export-third");
    let third = Store::open(third.path(), u64::MAX).unwrap();
    assert!(matches!(
        third.import(&tampered),
        Err(StoreError::Refused(_))
    ));
    assert!(third.releases().is_empty() || third.manifest(id).unwrap().is_empty());
}

#[test]
fn what_is_not_here_is_named_and_a_disagreeing_bundle_is_refused_whole() {
    let (here, there) = (Scratch::new("refuse-here"), Scratch::new("refuse-there"));
    let (file, table) = release();
    let store = Store::open(here.path(), u64::MAX).unwrap();
    let id = store.install(&file, &table, None).unwrap();
    store.put(id, A, &square(A, 40, 1)).unwrap();
    store.put(id, D, &square(D, 40, 1)).unwrap();
    store.put_empty(id, B).unwrap();
    // A candidate never baked here, and a square whose bytes are gone, are named in words.
    match store.closure(id, [A, C]) {
        Err(e @ StoreError::Missing { square }) => {
            assert_eq!(square, C);
            assert_eq!(
                e.to_string(),
                "the coast near 11°N 20°E is not on this computer"
            );
        }
        other => panic!("{other:?}"),
    }
    let closure = store.closure(id, [A, B, D, OPEN]).unwrap();
    std::fs::remove_file(store.square_path(&closure.contents().find(|c| c.square == D).unwrap()))
        .unwrap();
    match store.export(&closure) {
        Err(e @ StoreError::Missing { square: D }) => {
            assert_eq!(
                e.to_string(),
                "the coast near 5°S 179°E is not on this computer"
            )
        }
        other => panic!("{other:?}"),
    }
    store.put(id, D, &square(D, 40, 1)).unwrap();
    let bundle = store.export(&closure).unwrap();

    // Another computer baked A to other bytes: the bundle is refused, and nothing of it is recorded there.
    let other = Store::open(there.path(), u64::MAX).unwrap();
    other.install(&file, &table, None).unwrap();
    other.put(id, A, &square(A, 40, 2)).unwrap();
    assert!(matches!(
        other.import(&bundle),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(other.manifest(id).unwrap().len(), 1);
    assert_eq!(other.recorded(id, B).unwrap(), None);
}
