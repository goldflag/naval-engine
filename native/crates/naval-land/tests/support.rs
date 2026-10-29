//! What the tests share: a folder of their own, a small release and squares to bake into it.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use naval_land::format::block::Window;
use naval_land::format::lattice::WINDOW;
use naval_land::format::release::{BakeSpec, FileRef, LatticeSpec, ReleaseFile};
use naval_land::format::table::POLE_LIMIT_DEG;
use naval_land::format::{FinalSquare, SquareKey, SquareTable};

/// A folder for one test, empty, removed when it is dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join("naval-land-tests").join(format!(
            "{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const A: SquareKey = SquareKey { lat: 10, lon: 20 };
pub const B: SquareKey = SquareKey { lat: 10, lon: 21 };
pub const C: SquareKey = SquareKey { lat: 11, lon: 20 };
pub const D: SquareKey = SquareKey { lat: -5, lon: 179 };
/// An ocean square of the test release.
pub const OPEN: SquareKey = SquareKey { lat: 40, lon: -30 };

/// A release whose candidates are A, B, C and D: its file and its table.
pub fn release() -> (Vec<u8>, Vec<u8>) {
    let candidates: BTreeSet<SquareKey> = [A, B, C, D].into();
    let table = SquareTable::new(|k| candidates.contains(&k)).encode();
    let file = ReleaseFile {
        format: 1,
        name: "store-test".into(),
        lattice: LatticeSpec::current(),
        bake: BakeSpec {
            algorithm: "test".into(),
            version: 1,
            keep_m: 40_000,
            candidate_m: 65_000,
            pole_limit_deg: POLE_LIMIT_DEG,
        },
        snapshot: None,
        sources: None,
        square_table: FileRef::of(&table),
        synthetic: true,
    }
    .encode();
    (file, table)
}

/// A baked square of `key`: flat at `quantum`, `bake` in its header (another `bake` gives other bytes).
pub fn square(key: SquareKey, quantum: i16, bake: u32) -> Vec<u8> {
    let window = Window {
        quanta: vec![quantum; WINDOW * WINDOW],
    };
    let mut extra = serde_json::Map::new();
    extra.insert("bake".into(), bake.into());
    FinalSquare::encode(key, &window, extra)
}
