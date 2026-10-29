//! The mocked pair of buckets the bake's tests read (tests/release.rs), serving the fixtures' DEM
//! (`naval_region_bake::release::fixtures`), and the releases made over them. The client's tests of its baker include
//! this file too (`naval-client`'s tests/land_release.rs).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use naval_land::ReleaseRecord;
use naval_land::format::release::{FORMAT, FileRef, LatticeSpec, Snapshot};
use naval_land::format::{Digest, ReleaseFile, SquareKey, SquareTable};
use naval_land_bake::dem::{DemError, Got, Transport};
use naval_land_bake::release::{ReleaseSources, SourceCache, snapshot_path};
use naval_region_bake::release::fixtures;
use naval_region_bake::release::gebco;
use naval_region_bake::release::sources::{CellKey, Object, Reads, SourceManifest, TileKey};

pub fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

pub fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("naval-land-release-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn etag(bytes: &[u8]) -> String {
    Digest::of(bytes).hex()[..32].to_string()
}

/// The fixtures' objects, made once a process.
pub fn png(tile: TileKey) -> Arc<Vec<u8>> {
    static HELD: OnceLock<Mutex<HashMap<TileKey, Arc<Vec<u8>>>>> = OnceLock::new();
    let held = HELD.get_or_init(Mutex::default);
    if let Some(b) = held.lock().unwrap().get(&tile) {
        return b.clone();
    }
    let b = Arc::new(fixtures::terrarium_png(tile.0, tile.1, tile.2));
    held.lock().unwrap().insert(tile, b.clone());
    b
}

pub fn cog(cell: CellKey) -> Arc<Vec<u8>> {
    static HELD: OnceLock<Mutex<HashMap<CellKey, Arc<Vec<u8>>>>> = OnceLock::new();
    let held = HELD.get_or_init(Mutex::default);
    if let Some(b) = held.lock().unwrap().get(&cell) {
        return b.clone();
    }
    let b = Arc::new(fixtures::glo30_cog(cell.0, cell.1));
    held.lock().unwrap().insert(cell, b.clone());
    b
}

/// What the buckets were asked: the URL, the range and the ETag sent.
pub type Asked = (String, Option<(u64, u64)>, Option<String>);

/// The two buckets, serving the fixtures. `changed` answers 412 for one URL; `failing` fails one URL's every read (a
/// server error); `broken_glo30` fails every GLO-30 read.
#[derive(Default)]
pub struct Buckets {
    pub asked: Mutex<Vec<Asked>>,
    pub changed: Option<String>,
    pub failing: Option<String>,
    pub broken_glo30: bool,
}

pub fn parse_tile(url: &str) -> Option<TileKey> {
    let path = url.strip_prefix(&format!("{}/", naval_region_bake::terrarium::TERRARIUM_URL))?;
    let mut p = path.strip_suffix(".png")?.split('/');
    Some((
        p.next()?.parse().ok()?,
        p.next()?.parse().ok()?,
        p.next()?.parse().ok()?,
    ))
}

pub fn parse_cell(url: &str) -> Option<CellKey> {
    let name = url.rsplit('/').next()?.strip_suffix(".tif")?;
    let b = name.as_bytes();
    // Copernicus_DSM_COG_10_N86_00_E179_00_DEM
    let lat: i32 = name.get(23..25)?.parse().ok()?;
    let lon: i32 = name.get(30..33)?.parse().ok()?;
    let lat = if b[22] == b'S' { -lat } else { lat };
    let lon = if b[29] == b'W' { -lon } else { lon };
    Some((lat, lon))
}

impl Transport for Buckets {
    fn get(&self, url: &str, range: Option<(u64, u64)>) -> Result<Got, DemError> {
        self.asked.lock().unwrap().push((url.into(), range, None));
        Err(DemError::failed(format!("{url}: an unpinned read")))
    }

    fn get_matching(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        tag: &str,
    ) -> Result<Got, DemError> {
        self.asked
            .lock()
            .unwrap()
            .push((url.into(), range, Some(tag.into())));
        if self.changed.as_deref() == Some(url) {
            return Ok(Got::Changed);
        }
        if self.failing.as_deref() == Some(url) {
            return Err(DemError::failed(format!("{url}: HTTP 500")));
        }
        if url == gebco::url() {
            if tag != GEBCO_ETAG {
                return Ok(Got::Changed);
            }
            let (at, n) = range.expect("GEBCO is read by range");
            return Ok(Got::Bytes(fixtures::gebco_range(at, n)));
        }
        let bytes = if let Some(tile) = parse_tile(url) {
            png(tile)
        } else if let Some(cell) = parse_cell(url) {
            if self.broken_glo30 {
                return Err(DemError::failed(format!("{url}: HTTP 500")));
            }
            cog(cell)
        } else {
            return Ok(Got::Absent);
        };
        if etag(&bytes) != tag {
            return Ok(Got::Changed);
        }
        Ok(Got::Bytes(match range {
            None => bytes.to_vec(),
            Some((at, n)) => bytes[at as usize..(at + n) as usize].to_vec(),
        }))
    }
}

/// The GEBCO fixture's ETag.
pub const GEBCO_ETAG: &str = "6e1f5ac7fixture-1";

/// A manifest listing what `squares` read, every cell present unless in `absent`.
pub fn manifest(squares: &[SquareKey], absent: &[CellKey]) -> SourceManifest {
    let mut m = SourceManifest::default();
    for &key in squares {
        let reads = Reads::of(key);
        for tile in reads.tiles() {
            let b = png(tile);
            m.terrarium.insert(
                tile,
                Object {
                    etag: etag(&b),
                    bytes: b.len() as u64,
                },
            );
        }
        for cell in reads.cells() {
            let object = (!absent.contains(&cell)).then(|| {
                let b = cog(cell);
                Object {
                    etag: etag(&b),
                    bytes: b.len() as u64,
                }
            });
            m.glo30.insert(cell, object);
        }
    }
    m
}

/// A release for the first bake over `candidates`, its manifest `m`, its snapshot `snapshot`.
pub fn record(candidates: &[SquareKey], m: &SourceManifest, snapshot: &[u8]) -> ReleaseRecord {
    record_for(1, candidates, m, snapshot)
}

/// A release for bake `version`.
pub fn record_for(
    version: u32,
    candidates: &[SquareKey],
    m: &SourceManifest,
    snapshot: &[u8],
) -> ReleaseRecord {
    let table = SquareTable::new(|key| candidates.contains(&key)).encode();
    let sources = m.encode();
    let file = ReleaseFile {
        format: FORMAT,
        name: "land-test".into(),
        lattice: LatticeSpec::current(),
        bake: naval_region_bake::finish::bake_spec_for(version),
        snapshot: Some(Snapshot {
            source: "water-polygons-split-4326.zip".into(),
            date: "2026-10-14".into(),
            sha256: Digest::of(snapshot),
            bytes: snapshot.len() as u64,
        }),
        sources: Some(FileRef::of(&sources)),
        square_table: FileRef::of(&table),
        synthetic: false,
    };
    ReleaseRecord {
        release: file.encode(),
        table,
        sources: Some(sources),
    }
}

pub fn sources(
    dir: &std::path::Path,
    record: &ReleaseRecord,
    buckets: Arc<Buckets>,
    snapshot: &[u8],
) -> ReleaseSources {
    let path = snapshot_path(dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, snapshot).unwrap();
    let cache = Arc::new(SourceCache::open(dir.join("cache"), u64::MAX).unwrap());
    ReleaseSources::open(record, path, cache, buckets).unwrap()
}

/// The island across ±180° at 16.5° S and the nine candidates about it, the snapshot's way (split at ±180°).
pub fn island() -> (Vec<u8>, Vec<SquareKey>) {
    let ring =
        |lon0: f64, lon1: f64| vec![[-18.6, lon0], [-18.6, lon1], [-14.4, lon1], [-14.4, lon0]];
    let island =
        |lon0: f64, lon1: f64| vec![[-16.6, lon0], [-16.6, lon1], [-16.4, lon1], [-16.4, lon0]];
    let snapshot = fixtures::snapshot(&[
        vec![ring(177.5, 180.0), island(179.9, 180.0)],
        vec![ring(-180.0, -177.5), island(-180.0, -179.95)],
    ]);
    let candidates = (-18..=-16)
        .flat_map(|lat| [178, 179, -180].map(|lon| k(lat, lon)))
        .collect();
    (snapshot, candidates)
}
