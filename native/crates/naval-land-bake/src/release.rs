//! Land releases on this machine (step C, BAKE-2; the open-ocean plan's C1 and §3.1): **making** one from pinned
//! sources ([`make_release`]), and **baking a missing square** of one into the store ([`bake_square`]). Nothing in play
//! reads a release yet (LAND wires it in); `naval-land-bake` and `naval-client land-release` ([`super::cli`]) drive
//! both.
//!
//! - **Where.** The store is `naval_land::Store` at [`store_dir`] (`<data>/land`). The sources are at [`sources_dir`]
//!   (`<data>/land-sources`): the snapshot, pinned and never evicted (`snapshot/water-polygons-split-4326.zip`; fetched
//!   at the first bake that needs it when the game's sources say where from, [`ReleaseSources::with_host`]), and
//!   [`SourceCache`] (`cache/`), bounded by size and evicted least recently used first: the DEM's objects as read
//!   (`dem/`, each under its ETag), each square's water pieces (`pieces/<snapshot>/`) and each surveyed square
//!   (`surveyed/<source manifest>-v<bake version>/`). Everything in the cache is a function of the release's pinned sources and
//!   is made again when gone.
//! - **Pinned reads** ([`PinnedDem`]). Every object comes from the release's source manifest: a Terrarium tile or a
//!   GLO-30 object it does not list is refused, a GLO-30 cell it lists absent is sea and never asked for, and every
//!   request sends the listed ETag (`If-Match`). An object that changed (412), is gone, or comes back another length
//!   fails the bake. There is no fallback: a GLO-30 cell that cannot be read is never baked from Terrarium.
//! - **A square** ([`bake_square`]): the square's pieces cut from the snapshot (whose SHA-256 is checked against the
//!   release first), the surveyed squares its halo reads (a candidate's from the DEM by place, an ocean's or polar's the
//!   shelf's floor), the finish ([`naval_region_bake::finish`]), then its manifest line: its bytes when final land lies
//!   within 40 km (its own window, or a candidate neighbour's, from the store or baked for the purpose), else `empty`.
//!   A line once written never changes: a bake that gives other bytes is refused by the store.
//! - **A release** ([`make_release`]): the snapshot hashed and dated, the square table from one pass over it, then the
//!   source manifest from the buckets' listings (S3 `ListObjectsV2`: no object is downloaded), then the release file
//!   naming the three. Its id is published only after.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use naval_land::format::lattice::PER_DEGREE;
use naval_land::format::release::{FORMAT as RELEASE_FORMAT, FileRef, LatticeSpec, Snapshot};
use naval_land::format::{
    Class, Digest, Entry, Release, ReleaseFile, ReleaseId, SquareKey, SquareTable,
};
use naval_land::{Answer, ReleaseRecord, Store};
use naval_region_bake::finish::lattice::{Region, wrap_j};
use naval_region_bake::finish::surveyed::Surveyed;
use naval_region_bake::finish::{self, Finished, Halo, keep};
use naval_region_bake::glo30::{self, HeldCell};
use naval_region_bake::release::gebco;
use naval_region_bake::release::pieces::{Pieces, Splitter};
use naval_region_bake::release::sources::{
    CellKey, Object, Reads, SourceManifest, TileKey, glo30_url, terrarium_url,
};
use naval_region_bake::release::table::TableBuilder;
use naval_region_bake::release::{SNAPSHOT_FILE, snapshot, surveyed as by_place};
use naval_region_bake::square::Square;
use naval_region_bake::terrarium::{self, DemTile};
use naval_region_bake::{BakeError, glo30::Cell};
use sha2::Digest as _;

use super::dem::{DemError, Got, MAX_READ_BYTES, Transport, write_whole};
use super::{BakeFailure, FETCHERS, parallel};

/// The source cache's bound when its caller sets none.
pub const DEFAULT_SOURCE_BOUND: u64 = 16 << 30;
/// Surveyed squares held decoded (about 10 MB each): the owners of four bakes at once (nine each) and a few more.
pub const SURVEYED_HELD: usize = 40;
/// Squares' pieces held decoded.
pub const PIECES_HELD: usize = 64;
/// The most squares one pass over the snapshot cuts pieces for (about 0.15 MB of pieces each, at most a few MB).
pub const PASS_SQUARES: usize = 128;
/// How long a pass over the snapshot waits before it takes the squares wanted: bakes asked for together (a boundary's
/// need set) all name theirs by then, and one pass of about three seconds serves them all.
pub const PASS_SETTLE: Duration = Duration::from_millis(30);

/// Where land releases' sources are kept: `NAVAL_LAND_SOURCES_DIR` when set, else the data directory's `land-sources`
/// (beside the store, `land`). The game's data directory is the user's (`naval-client`'s `assets::user_data_dir`).
pub fn sources_dir(data: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_LAND_SOURCES_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    data.join("land-sources")
}

/// The store's folder: `NAVAL_LAND_DIR` when set, else the data directory's `land`.
pub fn store_dir(data: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_LAND_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    data.join("land")
}

/// The pinned snapshot under a sources folder.
pub fn snapshot_path(sources: &Path) -> PathBuf {
    sources.join("snapshot").join(SNAPSHOT_FILE)
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> BakeFailure + '_ {
    move |e| BakeFailure::Io(format!("{}: {e}", path.display()))
}

/// The SHA-256 and length of a file, read once.
pub fn hash_file(path: &Path) -> Result<(Digest, u64), BakeFailure> {
    hash_open(&mut File::open(path).map_err(io(path))?, path)
}

/// The SHA-256 and length of an open file, read from its start.
fn hash_open(file: &mut File, path: &Path) -> Result<(Digest, u64), BakeFailure> {
    file.seek(SeekFrom::Start(0)).map_err(io(path))?;
    let mut file = BufReader::with_capacity(1 << 20, file);
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut length = 0u64;
    loop {
        let n = file.read(&mut buffer).map_err(io(path))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        length += n as u64;
    }
    Ok((Digest(hasher.finalize().into()), length))
}

/// The snapshot, opened once and hashed through that handle. Every pass reads the same open file, so a snapshot
/// replaced on disk is never read in its place, and a pass over a file written to since (its length or modification
/// time moved) fails before anything it read is kept.
pub struct HeldSnapshot {
    path: PathBuf,
    file: Mutex<File>,
    stamp: (u64, Option<SystemTime>),
    pub sha256: Digest,
    pub bytes: u64,
}

impl HeldSnapshot {
    pub fn open(path: &Path) -> Result<Self, BakeFailure> {
        let mut file = File::open(path).map_err(io(path))?;
        let stamp = Self::stamp_of(&file, path)?;
        let (sha256, bytes) = hash_open(&mut file, path)?;
        let held = Self {
            path: path.to_path_buf(),
            file: Mutex::new(file),
            stamp,
            sha256,
            bytes,
        };
        held.unchanged(&held.file.lock().unwrap())?;
        Ok(held)
    }

    fn stamp_of(file: &File, path: &Path) -> Result<(u64, Option<SystemTime>), BakeFailure> {
        let meta = file.metadata().map_err(io(path))?;
        Ok((meta.len(), meta.modified().ok()))
    }

    fn unchanged(&self, file: &File) -> Result<(), BakeFailure> {
        let stamp = Self::stamp_of(file, &self.path)?;
        if stamp != self.stamp || stamp.0 != self.bytes {
            return Err(BakeFailure::Bake(format!(
                "{} changed on disk while it was read",
                self.path.display()
            )));
        }
        Ok(())
    }

    /// One pass over the held file, from its start.
    pub fn pass<T>(
        &self,
        read: impl FnOnce(&mut File) -> Result<T, BakeFailure>,
    ) -> Result<T, BakeFailure> {
        let mut file = self.file.lock().unwrap();
        self.unchanged(&file)?;
        file.seek(SeekFrom::Start(0)).map_err(io(&self.path))?;
        let out = read(&mut file)?;
        self.unchanged(&file)?;
        Ok(out)
    }
}

// ---------------------------------------------------------------- the source cache

/// A folder of source files bounded by size: past the bound the least recently used go first ([`Self::trim`], called
/// between bakes, never during one). Across sessions the order is the files' modification times, which a read
/// refreshes.
pub struct SourceCache {
    root: PathBuf,
    bound: u64,
    state: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    files: HashMap<PathBuf, (u64, u64)>,
    bytes: u64,
    clock: u64,
}

impl CacheState {
    fn touch(&mut self, rel: &Path, size: u64) {
        self.clock += 1;
        let clock = self.clock;
        match self.files.get_mut(rel) {
            Some(entry) => {
                self.bytes = self.bytes - entry.0 + size;
                *entry = (size, clock);
            }
            None => {
                self.bytes += size;
                self.files.insert(rel.to_path_buf(), (size, clock));
            }
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, u64, SystemTime)>) -> std::io::Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let meta = entry.metadata()?;
        if meta.is_dir() {
            walk(root, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "part") {
            // A write a crash cut short.
            let _ = std::fs::remove_file(&path);
        } else if let Ok(rel) = path.strip_prefix(root) {
            out.push((rel.to_path_buf(), meta.len(), meta.modified()?));
        }
    }
    Ok(())
}

impl SourceCache {
    pub fn open(root: impl Into<PathBuf>, bound: u64) -> Result<Self, BakeFailure> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(io(&root))?;
        let mut found = vec![];
        walk(&root, &root, &mut found).map_err(io(&root))?;
        found.sort_by_key(|f| f.2);
        let mut state = CacheState::default();
        for (rel, size, _) in found {
            state.touch(&rel, size);
        }
        Ok(Self {
            root,
            bound,
            state: Mutex::new(state),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A file's bytes, when kept.
    pub fn read(&self, rel: &Path) -> Option<Vec<u8>> {
        let path = self.root.join(rel);
        let bytes = std::fs::read(&path).ok()?;
        self.state.lock().unwrap().touch(rel, bytes.len() as u64);
        let _ = File::options()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(SystemTime::now()));
        Some(bytes)
    }

    /// Keeps a file (whole: written aside, then moved in).
    pub fn write(&self, rel: &Path, bytes: &[u8]) {
        let path = self.root.join(rel);
        match write_whole(&path, bytes) {
            Ok(()) => self.state.lock().unwrap().touch(rel, bytes.len() as u64),
            Err(e) => eprintln!("land release: could not keep {}: {e}", path.display()),
        }
    }

    /// Files and bytes held.
    pub fn usage(&self) -> (usize, u64) {
        let s = self.state.lock().unwrap();
        (s.files.len(), s.bytes)
    }

    /// Evicts the least recently used files until the cache is within its bound; the files and bytes evicted.
    pub fn trim(&self) -> (usize, u64) {
        let mut s = self.state.lock().unwrap();
        if s.bytes <= self.bound {
            return (0, 0);
        }
        let mut order: Vec<(u64, PathBuf, u64)> = s
            .files
            .iter()
            .map(|(rel, &(size, used))| (used, rel.clone(), size))
            .collect();
        order.sort();
        let (mut files, mut bytes) = (0, 0);
        for (_, rel, size) in order {
            if s.bytes <= self.bound {
                break;
            }
            let _ = std::fs::remove_file(self.root.join(&rel));
            s.files.remove(&rel);
            s.bytes -= size;
            files += 1;
            bytes += size;
        }
        (files, bytes)
    }
}

// ---------------------------------------------------------------- pinned reads

/// An ETag as a folder name: hex digits, letters and dashes (S3's MD5s and multipart ETags).
fn safe_etag(etag: &str) -> Result<&str, BakeError> {
    if !etag.is_empty()
        && etag.len() <= 80
        && etag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Ok(etag)
    } else {
        Err(BakeError::Refused(format!(
            "the source manifest's ETag {etag:?} is not one"
        )))
    }
}

/// The DEM a release's bakes read: only objects its source manifest lists, each by its frozen ETag.
pub struct PinnedDem {
    manifest: Arc<SourceManifest>,
    cache: Arc<SourceCache>,
    net: Arc<dyn Transport>,
    /// Bytes fetched over the network, and requests sent.
    pub downloaded: AtomicU64,
    pub requests: AtomicU64,
}

fn read_failed(e: DemError) -> BakeError {
    BakeError::Missing(format!("a pinned source could not be read ({e})"))
}

impl PinnedDem {
    pub fn new(
        manifest: Arc<SourceManifest>,
        cache: Arc<SourceCache>,
        net: Arc<dyn Transport>,
    ) -> Self {
        Self {
            manifest,
            cache,
            net,
            downloaded: AtomicU64::new(0),
            requests: AtomicU64::new(0),
        }
    }

    /// One pinned GET, a few tries for a transient failure; never one for a changed or missing object.
    fn fetch(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        object: &Object,
    ) -> Result<Vec<u8>, BakeError> {
        let mut last = None;
        for attempt in 0..3 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(400 << attempt));
            }
            self.requests.fetch_add(1, Ordering::Relaxed);
            match self.net.get_matching(url, range, &object.etag) {
                Ok(Got::Bytes(bytes)) => {
                    self.downloaded
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    return Ok(bytes);
                }
                Ok(Got::Changed) => {
                    return Err(BakeError::Changed(format!(
                        "{url} changed since the release was made: its ETag is no longer {}",
                        object.etag
                    )));
                }
                Ok(Got::Absent) => {
                    return Err(BakeError::Changed(format!(
                        "{url} is gone from its bucket since the release was made"
                    )));
                }
                Err(e) if e.code == "offline" => return Err(read_failed(e)),
                Err(e) => last = Some(e),
            }
        }
        Err(read_failed(last.unwrap_or_else(|| DemError::failed(url))))
    }

    /// A Terrarium tile's PNG, as the manifest lists it.
    pub fn terrarium_png(&self, tile: TileKey) -> Result<Vec<u8>, BakeError> {
        let object = self.manifest.tile(tile)?;
        let etag = safe_etag(&object.etag)?;
        let rel = PathBuf::from(format!(
            "dem/terrarium/{}/{}/{}-{etag}.png",
            tile.0, tile.1, tile.2
        ));
        if let Some(bytes) = self
            .cache
            .read(&rel)
            .filter(|b| b.len() as u64 == object.bytes)
        {
            return Ok(bytes);
        }
        let url = terrarium_url(tile);
        let bytes = self.fetch(&url, None, object)?;
        if bytes.len() as u64 != object.bytes {
            return Err(BakeError::Refused(format!(
                "{url}: {} bytes, where the source manifest lists {}",
                bytes.len(),
                object.bytes
            )));
        }
        self.cache.write(&rel, &bytes);
        Ok(bytes)
    }

    /// A Terrarium tile, decoded.
    pub fn terrarium(&self, tile: TileKey) -> Result<Arc<DemTile>, BakeError> {
        Ok(Arc::new(terrarium::decode(&self.terrarium_png(tile)?)?))
    }

    /// Bytes `offset..offset + length` of a GLO-30 object the manifest lists.
    pub fn glo30_range(
        &self,
        cell: CellKey,
        object: &Object,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, BakeError> {
        let name = glo30::name(cell.0, cell.1);
        if length == 0
            || length > MAX_READ_BYTES
            || offset
                .checked_add(length)
                .is_none_or(|end| end > object.bytes)
        {
            return Err(BakeError::Refused(format!(
                "{name}: a read of {length} bytes at {offset} in an object of {}",
                object.bytes
            )));
        }
        let etag = safe_etag(&object.etag)?;
        let rel = PathBuf::from(format!("dem/glo30/{name}/{etag}/{offset}-{length}.bin"));
        if let Some(bytes) = self.cache.read(&rel).filter(|b| b.len() as u64 == length) {
            return Ok(bytes);
        }
        let bytes = self.fetch(&glo30_url(cell), Some((offset, length)), object)?;
        if bytes.len() as u64 != length {
            return Err(BakeError::Refused(format!(
                "{name}: {} bytes for a range of {length}",
                bytes.len()
            )));
        }
        self.cache.write(&rel, &bytes);
        Ok(bytes)
    }

    /// A GLO-30 cell with every block `reads` takes of it, or none when the manifest lists it absent (open sea, never
    /// asked for).
    pub fn glo30_cell(
        &self,
        reads: &Reads,
        cell: CellKey,
    ) -> Result<Option<Arc<dyn Cell + Send + Sync>>, BakeError> {
        let Some(object) = self.manifest.cell(cell)? else {
            return Ok(None);
        };
        let name = glo30::name(cell.0, cell.1);
        let mut length = glo30::HEADER_BYTES.min(object.bytes);
        let mut header = None;
        for _ in 0..3 {
            let bytes = self.glo30_range(cell, object, 0, length)?;
            match glo30::parse_header(&bytes, &name) {
                Ok(h) => {
                    header = Some(h);
                    break;
                }
                Err(glo30::HeaderError::Short(need))
                    if (need as u64) > length && length < object.bytes =>
                {
                    length = ((need as u64).div_ceil(4096) * 4096).min(object.bytes);
                }
                Err(glo30::HeaderError::Short(_)) => {
                    return Err(BakeError::Corrupt(format!("{name}: a short header")));
                }
                Err(glo30::HeaderError::Bad(why)) => return Err(BakeError::Corrupt(why)),
            }
        }
        let header =
            header.ok_or_else(|| BakeError::Corrupt(format!("{name}: the header kept growing")))?;
        let mut blocks = HashMap::new();
        for k in reads.glo30_blocks(cell, &header) {
            let (at, count) = (header.offsets[k], header.counts[k]);
            let bytes = self.glo30_range(cell, object, at, count)?;
            blocks.insert(k, Arc::new(glo30::decode_block(&bytes, header.block)?));
        }
        Ok(Some(Arc::new(HeldCell { header, blocks })))
    }
}

impl PinnedDem {
    /// Bytes `offset..offset + length` of the sea floor's object, as the manifest lists it.
    pub fn gebco_range(&self, offset: u64, length: u64) -> Result<Vec<u8>, BakeError> {
        let object = self.manifest.seabed()?;
        if length == 0
            || length > MAX_READ_BYTES
            || offset
                .checked_add(length)
                .is_none_or(|end| end > object.bytes)
        {
            return Err(BakeError::Refused(format!(
                "GEBCO: a read of {length} bytes at {offset} in an object of {}",
                object.bytes
            )));
        }
        let etag = safe_etag(&object.etag)?;
        let rel = PathBuf::from(format!("dem/gebco/{etag}/{offset}-{length}.bin"));
        if let Some(bytes) = self.cache.read(&rel).filter(|b| b.len() as u64 == length) {
            return Ok(bytes);
        }
        let bytes = self.fetch(&gebco::url(), Some((offset, length)), object)?;
        if bytes.len() as u64 != length {
            return Err(BakeError::Refused(format!(
                "GEBCO: {} bytes for a range of {length}",
                bytes.len()
            )));
        }
        self.cache.write(&rel, &bytes);
        Ok(bytes)
    }

    /// The sea floor's header (its first IFD and block tables).
    pub fn gebco_header(&self) -> Result<gebco::Header, BakeError> {
        let object = self.manifest.seabed()?;
        let mut length = gebco::HEADER_BYTES.min(object.bytes);
        for _ in 0..3 {
            let bytes = self.gebco_range(0, length)?;
            match gebco::parse_header(&bytes) {
                Ok(h) => return Ok(h),
                Err(gebco::HeaderError::Short(need))
                    if (need as u64) > length && length < object.bytes =>
                {
                    length = ((need as u64).div_ceil(4096) * 4096).min(object.bytes);
                }
                Err(gebco::HeaderError::Short(_)) => {
                    return Err(BakeError::Corrupt("GEBCO: a short header".into()));
                }
                Err(gebco::HeaderError::Bad(why)) => return Err(BakeError::Corrupt(why)),
            }
        }
        Err(BakeError::Corrupt("GEBCO: the header kept growing".into()))
    }
}

/// GEBCO blocks held decoded (half a megabyte each): a few bakes' worth.
pub const GEBCO_HELD: usize = 64;

/// The sea floor a release's bakes read ([`PinnedDem::gebco_range`]): its header once, its blocks decoded as asked.
struct SeaFloorSource {
    header: OnceLock<Result<Arc<gebco::Header>, BakeError>>,
    blocks: Mutex<Lru<usize, Arc<Vec<i16>>>>,
}

// ---------------------------------------------------------------- a release's sources

/// A small map that forgets the least recently used past its size.
struct Lru<K, V> {
    held: HashMap<K, (V, u64)>,
    clock: u64,
    size: usize,
}

impl<K: std::hash::Hash + Eq + Copy, V: Clone> Lru<K, V> {
    fn new(size: usize) -> Self {
        Self {
            held: HashMap::new(),
            clock: 0,
            size,
        }
    }
    fn get(&mut self, k: K) -> Option<V> {
        self.clock += 1;
        let clock = self.clock;
        self.held.get_mut(&k).map(|e| {
            e.1 = clock;
            e.0.clone()
        })
    }
    fn put(&mut self, k: K, v: V) {
        self.clock += 1;
        self.held.insert(k, (v, self.clock));
        while self.held.len() > self.size {
            let Some(oldest) = self.held.iter().min_by_key(|e| e.1.1).map(|e| *e.0) else {
                break;
            };
            self.held.remove(&oldest);
        }
    }
    fn remove(&mut self, k: K) {
        self.held.remove(&k);
    }
}

type Slot = Arc<OnceLock<Result<Arc<Square>, BakeError>>>;

/// The passes over the snapshot: the squares the next one cuts besides those a bake asks for, and whether one is under
/// way.
#[derive(Default)]
struct Passes {
    wanted: Vec<SquareKey>,
    running: bool,
}

/// A count of things under way, at most a bound: each held by a [`Permit`].
struct Gate {
    under_way: Mutex<usize>,
    freed: Condvar,
    bound: usize,
}

impl Gate {
    fn new(bound: usize) -> Self {
        Self {
            under_way: Mutex::new(0),
            freed: Condvar::new(),
            bound: bound.max(1),
        }
    }

    /// Waits for room, and takes it until the permit is dropped.
    fn enter(&self) -> Permit<'_> {
        let mut n = self.under_way.lock().unwrap();
        while *n >= self.bound {
            n = self.freed.wait(n).unwrap();
        }
        *n += 1;
        Permit(self)
    }
}

struct Permit<'a>(&'a Gate);

/// A pass over the snapshot under way: when it is dropped the pass is over, and the bakes waiting for it go on.
struct Passing<'a>(&'a ReleaseSources);

impl Drop for Passing<'_> {
    fn drop(&mut self) {
        self.0
            .passes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .running = false;
        self.0.passed.notify_all();
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.under_way.lock().unwrap() -= 1;
        self.0.freed.notify_one();
    }
}

/// What the sources did, for a bake's report.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStats {
    pub snapshot_passes: usize,
    pub pieces_cut: usize,
    pub surveyed_baked: usize,
    pub surveyed_from_disk: usize,
}

/// A release's sources on this machine: its record, the pinned snapshot, the DEM by its manifest, and the caches.
pub struct ReleaseSources {
    pub id: ReleaseId,
    /// The bake the release was made for (`finish::VERSIONS`).
    pub version: u32,
    pub release: Release,
    pub table: SquareTable,
    pub manifest: Arc<SourceManifest>,
    pub dem: PinnedDem,
    pub cache: Arc<SourceCache>,
    snapshot_path: PathBuf,
    /// The snapshot, once held and checked.
    snapshot: Mutex<Option<Arc<HeldSnapshot>>>,
    /// Where a missing snapshot is fetched from (`hosted`): none, and a missing snapshot refuses the bake.
    host: Option<Arc<dyn super::hosted::Host>>,
    surveyed: Mutex<Lru<SquareKey, Slot>>,
    pieces: Mutex<Lru<SquareKey, Arc<Pieces>>>,
    passes: Mutex<Passes>,
    passed: Condvar,
    /// Surveyed squares read and baked from the DEM at once, however many bakes ask for them: each holds its decoded
    /// DEM tiles and cells (up to about 100 MB) until it is done.
    loads: Gate,
    /// The second bake's sea floor.
    seabed: SeaFloorSource,
    pub stats: Mutex<SourceStats>,
}

impl ReleaseSources {
    /// The sources of the release `record` names, its snapshot at `snapshot`.
    pub fn open(
        record: &ReleaseRecord,
        snapshot: PathBuf,
        cache: Arc<SourceCache>,
        net: Arc<dyn Transport>,
    ) -> Result<Self, BakeFailure> {
        let refused = |why: String| BakeFailure::Bake(why);
        let release = ReleaseFile::parse(&record.release).map_err(|e| refused(e.to_string()))?;
        let (Some(_), Some(named), Some(bytes)) = (
            &release.file.snapshot,
            &release.file.sources,
            &record.sources,
        ) else {
            return Err(refused(format!(
                "{} pins no snapshot or source manifest: it is not a release made from sources",
                release.file.name
            )));
        };
        if !named.matches(bytes) || !release.file.square_table.matches(&record.table) {
            return Err(refused(
                "the source manifest or square table is not the one the release names".into(),
            ));
        }
        let Some(version) = finish::version_of(&release.file.bake) else {
            return Err(refused(format!(
                "{} was made for another bake ({:?}): this one bakes {:?}",
                release.file.name,
                release.file.bake,
                finish::VERSIONS.map(finish::bake_spec_for)
            )));
        };
        let manifest = Arc::new(SourceManifest::parse(bytes)?);
        // The second bake reads its sea floor by the manifest's pin, the first none: a manifest that lists another
        // than the bake reads is not this release's.
        if (version >= 2) != manifest.seabed.is_some() {
            return Err(refused(format!(
                "{}'s source manifest pins {} sea floor, and its bake {version} reads {}",
                release.file.name,
                if manifest.seabed.is_some() { "a" } else { "no" },
                if version >= 2 { "one" } else { "none" }
            )));
        }
        let table = SquareTable::decode(&record.table).map_err(|e| refused(e.to_string()))?;
        Ok(Self {
            id: release.id,
            version,
            dem: PinnedDem::new(manifest.clone(), cache.clone(), net),
            release,
            table,
            manifest,
            cache,
            snapshot_path: snapshot,
            snapshot: Mutex::new(None),
            host: None,
            surveyed: Mutex::new(Lru::new(SURVEYED_HELD)),
            pieces: Mutex::new(Lru::new(PIECES_HELD)),
            passes: Mutex::default(),
            passed: Condvar::new(),
            loads: Gate::new(super::pool_threads()),
            seabed: SeaFloorSource {
                header: OnceLock::new(),
                blocks: Mutex::new(Lru::new(GEBCO_HELD)),
            },
            stats: Mutex::default(),
        })
    }

    /// These sources with a missing snapshot fetched from `host` the first time a bake needs it.
    pub fn with_host(mut self, host: Arc<dyn super::hosted::Host>) -> Self {
        self.host = Some(host);
        self
    }

    /// These sources with at most `loads` surveyed squares read from the DEM at once (the machine's pool by default).
    pub fn with_loads(mut self, loads: usize) -> Self {
        self.loads = Gate::new(loads);
        self
    }

    pub fn class(&self, key: SquareKey) -> Class {
        self.table.class(key)
    }

    /// The snapshot on disk, held open: the release's (its length and SHA-256), checked once a process.
    /// Not there, it is fetched first from the sources' host (`with_host`), checked against the release's pin before it
    /// is kept: one fetch, however many bakes wait for it. A fetch that fails is tried again by the next bake.
    pub fn check_snapshot(&self) -> Result<Arc<HeldSnapshot>, BakeFailure> {
        let mut held = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(snapshot) = held.as_ref() {
            return Ok(snapshot.clone());
        }
        let pinned = self
            .release
            .file
            .snapshot
            .as_ref()
            .expect("a sourced release");
        if let Some(host) = &self.host {
            super::hosted::fetch_pinned(
                host.as_ref(),
                &super::hosted::hosted_name(&pinned.sha256, "zip"),
                &pinned.sha256,
                pinned.bytes,
                &self.snapshot_path,
            )
            .map_err(|e| BakeFailure::Bake(e.to_string()))?;
        }
        let opened = HeldSnapshot::open(&self.snapshot_path)
            .map_err(|e| BakeFailure::Bake(e.to_string()))?;
        if opened.sha256 != pinned.sha256 || opened.bytes != pinned.bytes {
            return Err(BakeFailure::Bake(format!(
                "{} is not the snapshot {} pins ({} of {}): it cannot be baked on this computer",
                self.snapshot_path.display(),
                self.release.file.name,
                pinned.date,
                pinned.sha256
            )));
        }
        let opened = Arc::new(opened);
        *held = Some(opened.clone());
        Ok(opened)
    }

    fn pieces_rel(&self, key: SquareKey) -> PathBuf {
        let sha = self
            .release
            .file
            .snapshot
            .as_ref()
            .expect("a sourced release")
            .sha256;
        PathBuf::from(format!("pieces/{}/{}.nwp", &sha.hex()[..16], key.name()))
    }

    fn snapshot_sha(&self) -> Digest {
        self.release
            .file
            .snapshot
            .as_ref()
            .expect("a sourced release")
            .sha256
    }

    /// Square `key`'s pieces when held or kept in the cache.
    fn pieces_at_hand(&self, key: SquareKey) -> Option<Arc<Pieces>> {
        if let Some(p) = self.pieces.lock().unwrap().get(key) {
            return Some(p);
        }
        let p = Arc::new(
            Pieces::decode(
                &self.cache.read(&self.pieces_rel(key))?,
                key,
                self.snapshot_sha(),
            )
            .ok()?,
        );
        self.pieces.lock().unwrap().put(key, p.clone());
        Some(p)
    }

    /// Whether square `key`'s pieces are held or kept, without reading them.
    fn pieces_kept(&self, key: SquareKey) -> bool {
        self.pieces.lock().unwrap().held.contains_key(&key)
            || self.cache.root().join(self.pieces_rel(key)).is_file()
    }

    /// The squares whose pieces coming bakes will want, most wanted first: the next pass over the snapshot cuts as many
    /// of them as it may ([`PASS_SQUARES`]) beside the pieces a bake asks for, so one pass serves many bakes. Named
    /// before those wanted already, which the pass takes after them.
    pub fn want_pieces(&self, keys: Vec<SquareKey>) {
        let mut passes = self.passes.lock().unwrap();
        let mut wanted = keys;
        let named: std::collections::HashSet<SquareKey> = wanted.iter().copied().collect();
        wanted.extend(passes.wanted.iter().filter(|k| !named.contains(k)));
        wanted.truncate(4 * PASS_SQUARES);
        passes.wanted = wanted;
    }

    /// The squares' water pieces: held, from the cache, or cut in a pass over the snapshot. One pass at a time: a bake
    /// that finds one under way waits for it (its squares then wanted by the next), and the pass cuts the squares
    /// [`Self::want_pieces`] names too. A square's pieces are the same whichever pass cuts them.
    pub fn pieces(
        &self,
        keys: &[SquareKey],
    ) -> Result<HashMap<SquareKey, Arc<Pieces>>, BakeFailure> {
        let mut out = HashMap::new();
        loop {
            let mut missing = vec![];
            for &key in keys {
                if out.contains_key(&key) {
                    continue;
                }
                match self.pieces_at_hand(key) {
                    Some(p) => {
                        out.insert(key, p);
                    }
                    None => missing.push(key),
                }
            }
            if missing.is_empty() {
                return Ok(out);
            }
            let mut passes = self.passes.lock().unwrap();
            if passes.running {
                for (n, key) in missing.iter().enumerate() {
                    if !passes.wanted.contains(key) {
                        let at = n.min(passes.wanted.len());
                        passes.wanted.insert(at, *key);
                    }
                }
                while passes.running {
                    passes = self.passed.wait(passes).unwrap();
                }
                continue;
            }
            passes.running = true;
            drop(passes);
            // However the pass ends (an error, a panic), the next may begin.
            let _passing = Passing(self);
            // The bakes asked for with this one name their squares meanwhile.
            std::thread::sleep(PASS_SETTLE);
            let wanted = std::mem::take(&mut self.passes.lock().unwrap().wanted);
            let mut batch = missing.clone();
            for key in wanted {
                if batch.len() >= PASS_SQUARES.max(missing.len()) {
                    break;
                }
                if !batch.contains(&key) && !self.pieces_kept(key) {
                    batch.push(key);
                }
            }
            let cut = self.cut(&batch);
            drop(_passing);
            for p in cut? {
                if missing.contains(&p.key) {
                    out.insert(p.key, p);
                }
            }
        }
    }

    /// One pass over the snapshot: the pieces of `keys`, kept in the cache and held.
    fn cut(&self, keys: &[SquareKey]) -> Result<Vec<Arc<Pieces>>, BakeFailure> {
        let sha = self.snapshot_sha();
        let split = self.check_snapshot()?.pass(|file| {
            let mut split = Splitter::new(keys.iter().copied());
            snapshot::for_each(file, |record| {
                split.add(&record);
                Ok(())
            })?;
            Ok(split)
        })?;
        {
            let mut stats = self.stats.lock().unwrap();
            stats.snapshot_passes += 1;
            stats.pieces_cut += keys.len();
        }
        Ok(split
            .finish(sha)
            .into_iter()
            .map(|p| {
                self.cache.write(&self.pieces_rel(p.key), &p.encode());
                let p = Arc::new(p);
                self.pieces.lock().unwrap().put(p.key, p.clone());
                p
            })
            .collect())
    }

    fn surveyed_rel(&self, key: SquareKey) -> PathBuf {
        let sources = self.release.file.sources.expect("a sourced release").sha256;
        PathBuf::from(format!(
            "surveyed/{}-v{}/{}",
            &sources.hex()[..16],
            self.version,
            key.file_name()
        ))
    }

    /// Square `key`'s surveyed square: a candidate's from the DEM by place (held, from the cache, or baked from the
    /// pinned sources), an ocean or polar square's the shelf's floor.
    pub fn surveyed(&self, key: SquareKey) -> Result<Arc<Square>, BakeError> {
        let slot = {
            let mut held = self.surveyed.lock().unwrap();
            match held.get(key) {
                Some(slot) => slot,
                None => {
                    let slot = Slot::default();
                    held.put(key, slot.clone());
                    slot
                }
            }
        };
        let got = slot.get_or_init(|| self.load_surveyed(key)).clone();
        if got.is_err() {
            self.surveyed.lock().unwrap().remove(key);
        }
        got
    }

    fn load_surveyed(&self, key: SquareKey) -> Result<Arc<Square>, BakeError> {
        if self.class(key) != Class::Candidate {
            return Ok(Arc::new(by_place::sea(key)));
        }
        let rel = self.surveyed_rel(key);
        if let Some(Ok(square)) = self.cache.read(&rel).map(|b| Square::decode(&b)) {
            self.stats.lock().unwrap().surveyed_from_disk += 1;
            return Ok(Arc::new(square));
        }
        let reads = Reads::of(key);
        self.manifest.covers(&reads)?;
        let _loading = self.loads.enter();
        let never = AtomicBool::new(false);
        let tiles = reads.tiles();
        let fetched = parallel(&tiles, FETCHERS, &never, |&t| self.dem.terrarium(t));
        let fetched = settle(fetched.into_iter().map(|g| g.expect("not cancelled")))?;
        let tile_of: HashMap<TileKey, Arc<DemTile>> = tiles.iter().copied().zip(fetched).collect();
        let cells = reads.cells();
        let fetched = parallel(&cells, FETCHERS.min(4), &never, |&c| {
            self.dem.glo30_cell(&reads, c)
        });
        let fetched = settle(fetched.into_iter().map(|g| g.expect("not cancelled")))?;
        let cell_of: HashMap<CellKey, Option<Arc<dyn Cell + Send + Sync>>> =
            cells.iter().copied().zip(fetched).collect();
        let square = by_place::bake(
            &reads,
            &|t| {
                tile_of
                    .get(&t)
                    .cloned()
                    .ok_or_else(|| BakeError::Missing(format!("tile {t:?}")))
            },
            &|c| {
                cell_of
                    .get(&c)
                    .cloned()
                    .ok_or_else(|| BakeError::Missing(format!("cell {c:?}")))
            },
        )?;
        self.cache.write(&rel, &square.encode());
        self.stats.lock().unwrap().surveyed_baked += 1;
        Ok(Arc::new(square))
    }

    /// Makes ready the surveyed squares of `keys`, on the pool.
    pub fn prefetch(
        &self,
        keys: &[SquareKey],
        threads: usize,
        cancel: &AtomicBool,
    ) -> Result<(), BakeFailure> {
        let got = parallel(keys, threads, cancel, |&k| self.surveyed(k).map(|_| ()));
        if got.iter().any(Option::is_none) {
            return Err(BakeFailure::Cancelled);
        }
        settle(got.into_iter().flatten())?;
        Ok(())
    }

    /// Square `key`'s final window, baked from the sources.
    pub fn window(
        &self,
        key: SquareKey,
        threads: usize,
        cancel: &AtomicBool,
    ) -> Result<Finished, BakeFailure> {
        let pieces = self.pieces(&[key])?.remove(&key).expect("cut above");
        self.prefetch(&owners(key), threads, cancel)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(BakeFailure::Cancelled);
        }
        let seabed: Option<&dyn finish::Seabed> = (self.version >= 2).then_some(self as _);
        Ok(finish::bake_version(
            self.version,
            key,
            self,
            pieces.as_ref(),
            seabed,
        )?)
    }

    /// The sea floor's header, read once a process.
    fn gebco_header(&self) -> Result<Arc<gebco::Header>, BakeError> {
        self.seabed
            .header
            .get_or_init(|| self.dem.gebco_header().map(Arc::new))
            .clone()
    }

    /// GEBCO block `b`, held or read by the manifest's pin.
    fn gebco_block(&self, header: &gebco::Header, b: usize) -> Result<Arc<Vec<i16>>, BakeError> {
        if let Some(block) = self.seabed.blocks.lock().unwrap().get(b) {
            return Ok(block);
        }
        let (offset, length) = (header.offsets[b], header.counts[b]);
        let bytes = self.dem.gebco_range(offset, length)?;
        let block = Arc::new(gebco::decode_block(&bytes, header.block)?);
        self.seabed.blocks.lock().unwrap().put(b, block.clone());
        Ok(block)
    }
}

impl Surveyed for ReleaseSources {
    fn square(&self, key: SquareKey) -> Result<Arc<Square>, BakeError> {
        self.surveyed(key)
    }
}

impl finish::Seabed for ReleaseSources {
    fn floor(&self, region: &Region) -> Result<finish::lattice::Plane<f64>, BakeError> {
        let header = self.gebco_header()?;
        let mut blocks = gebco::Blocks::new();
        for b in gebco::blocks_for(&header, region) {
            blocks.insert(b, self.gebco_block(&header, b)?);
        }
        gebco::heights(&header, &blocks, region)
    }

    fn open_sea(&self, key: SquareKey) -> bool {
        self.class(key) != Class::Candidate
    }
}

/// Reads done side by side, settled: their values, or the first changed source among their errors (never tried again,
/// so never hidden behind a failure that would be), else the first error.
fn settle<T>(results: impl IntoIterator<Item = Result<T, BakeError>>) -> Result<Vec<T>, BakeError> {
    let results: Vec<Result<T, BakeError>> = results.into_iter().collect();
    if let Some(changed) = results.iter().find_map(|r| match r {
        Err(e @ BakeError::Changed(_)) => Some(e.clone()),
        _ => None,
    }) {
        return Err(changed);
    }
    results.into_iter().collect()
}

/// The squares whose surveyed heights square `key`'s bake reads: every owner its held region meets, none polar.
pub fn owners(key: SquareKey) -> Vec<SquareKey> {
    let halo = Halo::of(key);
    let r = Region::window(key).grow(halo.rows(), halo.columns());
    let (south, north) = (
        r.i(r.rows - 1).div_euclid(PER_DEGREE),
        r.top.div_euclid(PER_DEGREE),
    );
    let (west, east) = (
        r.left.div_euclid(PER_DEGREE),
        r.j(r.columns - 1).div_euclid(PER_DEGREE),
    );
    let mut out = BTreeSet::new();
    for lat in south.max(-90)..=north.min(89) {
        for lon in west..=east {
            let k = SquareKey {
                lat,
                lon: wrap_j(lon * PER_DEGREE).div_euclid(PER_DEGREE),
            };
            if !naval_land::format::table::polar(k) {
                out.insert(k);
            }
        }
    }
    out.into_iter().collect()
}

// ---------------------------------------------------------------- baking a square into the store

/// A square baked into the store.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SquareBaked {
    pub square: String,
    /// Its manifest line.
    pub line: String,
    /// Whether the line was new (else the store held it already, and the bake gave its bytes again).
    pub new: bool,
    /// The square's file, bytes (none for `empty`).
    pub bytes: Option<usize>,
    pub seconds: f64,
    /// Neighbours baked for the keep rule (their windows: the square's own held no land).
    pub neighbours: Vec<String>,
    pub report: finish::Report,
}

/// Bakes candidate square `key` of the release into `store` (which must have it installed): its window from the
/// sources, its manifest line by the keep rule. A square the store already records is baked again and must give the
/// recorded bytes: the store refuses others.
pub fn bake_square(
    store: &Store,
    sources: &ReleaseSources,
    key: SquareKey,
    threads: usize,
    cancel: &AtomicBool,
) -> Result<SquareBaked, BakeFailure> {
    let class = sources.class(key);
    if class != Class::Candidate {
        return Err(BakeFailure::Bake(format!(
            "{key} is {} in {}: never baked",
            class.name(),
            sources.release.file.name
        )));
    }
    let started = Instant::now();
    let finished = sources.window(key, threads, cancel)?;
    let mut neighbours = vec![];
    // The second bake keeps every candidate: its sea floor meets the open sea's only at open-sea squares.
    let data = finish::keeps_every_candidate(sources.version)
        || keep::keep(key, &finished.window, &mut |n| {
            if sources.class(n) != Class::Candidate {
                return Ok(None);
            }
            match store.read(sources.id, n) {
                Ok(Answer::Land(lease)) => {
                    let square = lease
                        .square()
                        .map_err(|e| BakeError::Corrupt(e.to_string()))?;
                    return Ok(Some(square.window()?));
                }
                Ok(Answer::Sea) => return Ok(None),
                _ => {}
            }
            neighbours.push(n.name());
            match sources.window(n, threads, cancel) {
                Ok(f) => Ok(Some(f.window)),
                Err(BakeFailure::Bake(m)) => Err(BakeError::Refused(m)),
                Err(BakeFailure::Changed(m)) => Err(BakeError::Changed(m)),
                Err(BakeFailure::Cancelled) => Err(BakeError::Cancelled),
                Err(e) => Err(BakeError::Missing(e.to_string())),
            }
        })?;
    let bytes = finished.bytes();
    let recorded = if data {
        store.put(sources.id, key, &bytes)
    } else {
        store.put_empty(sources.id, key)
    }
    .map_err(|e| BakeFailure::Bake(e.to_string()))?;
    let entry = if data {
        Entry::Data(Digest::of(&bytes))
    } else {
        Entry::Empty
    };
    Ok(SquareBaked {
        square: key.name(),
        line: naval_land::format::Manifest::line(key, entry)
            .trim_end()
            .to_string(),
        new: matches!(recorded, naval_land::format::manifest::Recorded::New(_)),
        bytes: data.then_some(bytes.len()),
        seconds: started.elapsed().as_secs_f64(),
        neighbours,
        report: finished.report,
    })
}

// ---------------------------------------------------------------- making a release

/// The buckets' listing endpoints.
pub const TERRARIUM_BUCKET: &str = "https://s3.amazonaws.com/elevation-tiles-prod";
pub const TERRARIUM_PREFIX: &str = "terrarium";

/// What making a release measured.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MakeStats {
    pub snapshot_bytes: u64,
    pub snapshot_date: String,
    pub records: usize,
    pub edges: u64,
    pub coast_edges: u64,
    pub candidates: usize,
    pub ocean: usize,
    pub polar: usize,
    pub tiles: usize,
    pub cells: usize,
    pub absent_cells: usize,
    pub list_requests: usize,
    pub list_bytes: u64,
    pub sources_bytes: usize,
    pub table_bytes: usize,
    pub seconds_hash: f64,
    pub seconds_table: f64,
    pub seconds_listing: f64,
}

/// A release made: its files and id.
pub struct Made {
    pub id: ReleaseId,
    pub release: Vec<u8>,
    pub table: Vec<u8>,
    pub sources: Vec<u8>,
    pub stats: MakeStats,
}

/// The table from the whole snapshot, and its counts.
pub fn table_of(
    snapshot: &HeldSnapshot,
    stats: &mut MakeStats,
) -> Result<SquareTable, BakeFailure> {
    let started = Instant::now();
    let builder = snapshot.pass(|file| {
        let mut builder = TableBuilder::default();
        snapshot::for_each(file, |record| {
            builder.add(&record);
            Ok(())
        })?;
        Ok(builder)
    })?;
    let table = builder.finish();
    stats.records = builder.records;
    stats.edges = builder.edges;
    stats.coast_edges = builder.coast_edges;
    for key in SquareKey::all() {
        match table.class(key) {
            Class::Candidate => stats.candidates += 1,
            Class::Ocean => stats.ocean += 1,
            Class::Polar => stats.polar += 1,
        }
    }
    stats.seconds_table = started.elapsed().as_secs_f64();
    Ok(table)
}

/// Makes the release of bake `version` (`land-v<version>`) from the snapshot at `snapshot_path`: its square table, its
/// source manifest listed from the buckets over `net` (from the second bake on, the sea floor's object too), and its
/// file.
pub fn make_release(
    snapshot_path: &Path,
    net: &dyn Transport,
    threads: usize,
    report: &(dyn Fn(&str) + Sync),
    version: u32,
) -> Result<Made, BakeFailure> {
    if !finish::VERSIONS.contains(&version) {
        return Err(BakeFailure::Bake(format!("no bake {version}")));
    }
    let mut stats = MakeStats::default();
    let started = Instant::now();
    // The hash, the date and the table are all read through one open handle.
    let held = HeldSnapshot::open(snapshot_path)?;
    let (sha256, bytes) = (held.sha256, held.bytes);
    let date = held.pass(|file| Ok(snapshot::date(file)?))?;
    stats.seconds_hash = started.elapsed().as_secs_f64();
    stats.snapshot_bytes = bytes;
    stats.snapshot_date = date.clone();
    report(&format!("snapshot {date}, {bytes} bytes, sha256 {sha256}"));
    let table = table_of(&held, &mut stats)?;
    report(&format!(
        "square table: {} candidate, {} ocean, {} polar ({:.0} s)",
        stats.candidates, stats.ocean, stats.polar, stats.seconds_table
    ));
    let started = Instant::now();
    let mut tiles = BTreeSet::new();
    let mut cells = BTreeSet::new();
    for key in table.candidates() {
        let reads = Reads::of(key);
        tiles.extend(reads.tiles());
        cells.extend(reads.cells());
    }
    report(&format!(
        "listing {} Terrarium tiles and {} GLO-30 cells",
        tiles.len(),
        cells.len()
    ));
    let listing = Listing::default();
    let seabed = if version >= 2 {
        let object = list(net, gebco::BUCKET, gebco::KEY, &listing)?
            .into_iter()
            .find(|(k, _)| k == gebco::KEY)
            .map(|o| o.1)
            .ok_or_else(|| {
                BakeFailure::Bake(format!("{} is not in its bucket's listing", gebco::url()))
            })?;
        report(&format!(
            "sea floor {}: ETag {}, {} bytes",
            gebco::url(),
            object.etag,
            object.bytes
        ));
        Some(object)
    } else {
        None
    };
    let manifest = SourceManifest {
        terrarium: list_terrarium(net, &tiles, threads, &listing, report)?,
        glo30: list_glo30(net, &cells, threads, &listing)?,
        seabed,
    };
    stats.seconds_listing = started.elapsed().as_secs_f64();
    stats.tiles = manifest.terrarium.len();
    stats.cells = manifest.glo30.values().filter(|o| o.is_some()).count();
    stats.absent_cells = manifest.glo30.values().filter(|o| o.is_none()).count();
    stats.list_requests = listing.requests.load(Ordering::Relaxed);
    stats.list_bytes = listing.bytes.load(Ordering::Relaxed);
    let sources = manifest.encode();
    let table_bytes = table.encode();
    stats.sources_bytes = sources.len();
    stats.table_bytes = table_bytes.len();
    let file = ReleaseFile {
        format: RELEASE_FORMAT,
        name: naval_region_bake::release::name(version),
        lattice: LatticeSpec::current(),
        bake: finish::bake_spec_for(version),
        snapshot: Some(Snapshot {
            source: SNAPSHOT_FILE.into(),
            date,
            sha256,
            bytes,
        }),
        sources: Some(FileRef::of(&sources)),
        square_table: FileRef::of(&table_bytes),
        synthetic: false,
    };
    let release = file.encode();
    Ok(Made {
        id: ReleaseId::of(&release),
        release,
        table: table_bytes,
        sources,
        stats,
    })
}

/// What the listing cost.
#[derive(Default)]
struct Listing {
    requests: AtomicUsize,
    bytes: AtomicU64,
}

/// A query value, percent-encoded.
fn encode(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let at = xml.find(&open)? + open.len();
    let end = xml[at..].find(&format!("</{name}>"))?;
    Some(&xml[at..at + end])
}

/// One page of a bucket's listing: each object's key with its ETag (unquoted) and size, and the next page's token.
pub struct ListingPage {
    pub objects: Vec<(String, Object)>,
    pub next: Option<String>,
}

/// One page of a bucket's listing.
pub fn parse_listing(xml: &str) -> Result<ListingPage, String> {
    if !xml.contains("<ListBucketResult") {
        return Err(format!(
            "not a bucket listing: {}",
            &xml[..xml.len().min(200)]
        ));
    }
    let mut out = vec![];
    let mut rest = xml;
    while let Some(at) = rest.find("<Contents>") {
        let end = rest[at..].find("</Contents>").ok_or("a cut listing")? + at;
        let item = &rest[at..end];
        let key = unescape(tag(item, "Key").ok_or("an object with no key")?);
        let etag = unescape(tag(item, "ETag").ok_or("an object with no ETag")?);
        let size = tag(item, "Size")
            .and_then(|s| s.parse().ok())
            .ok_or("an object with no size")?;
        out.push((
            key,
            Object {
                etag: etag.trim_matches('"').to_string(),
                bytes: size,
            },
        ));
        rest = &rest[end..];
    }
    // A cut page with no token to go on is a failure, never the end of the listing.
    let next = match tag(xml, "IsTruncated") {
        Some("true") => Some(unescape(
            tag(xml, "NextContinuationToken").ok_or("a cut listing with no continuation token")?,
        )),
        Some("false") => None,
        _ => return Err("a listing that does not say whether it is cut".into()),
    };
    Ok(ListingPage { objects: out, next })
}

/// Every object under `prefix` in `bucket` (all pages).
fn list(
    net: &dyn Transport,
    bucket: &str,
    prefix: &str,
    listing: &Listing,
) -> Result<Vec<(String, Object)>, BakeFailure> {
    let mut out = vec![];
    let mut token: Option<String> = None;
    loop {
        let mut url = format!(
            "{bucket}/?list-type=2&max-keys=1000&prefix={}",
            encode(prefix)
        );
        if let Some(t) = &token {
            url.push_str(&format!("&continuation-token={}", encode(t)));
        }
        let mut last = None;
        let mut page = None;
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 << attempt));
            }
            listing.requests.fetch_add(1, Ordering::Relaxed);
            match net.get(&url, None) {
                Ok(Got::Bytes(bytes)) => {
                    listing
                        .bytes
                        .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                    page = Some(bytes);
                    break;
                }
                Ok(_) => last = Some(DemError::failed(format!("{url}: no listing"))),
                Err(e) if e.code == "offline" => return Err(BakeFailure::Dem(e)),
                Err(e) => last = Some(e),
            }
        }
        let page = page.ok_or_else(|| BakeFailure::Dem(last.expect("a failure")))?;
        let ListingPage { objects, next } = parse_listing(&String::from_utf8_lossy(&page))
            .map_err(|e| BakeFailure::Bake(format!("{url}: {e}")))?;
        out.extend(objects);
        match next {
            Some(t) => token = Some(t),
            None => return Ok(out),
        }
    }
}

/// The decimal prefix a Terrarium row is listed under: all but its last two digits (at least one), so one listing
/// holds a hundred or so rows.
fn row_prefix(y: i64) -> String {
    let digits = y.to_string();
    digits[..digits.len().saturating_sub(2).max(1)].to_string()
}

fn list_terrarium(
    net: &dyn Transport,
    tiles: &BTreeSet<TileKey>,
    threads: usize,
    listing: &Listing,
    report: &(dyn Fn(&str) + Sync),
) -> Result<BTreeMap<TileKey, Object>, BakeFailure> {
    let prefixes: Vec<(u32, i64, String)> = tiles
        .iter()
        .map(|&(z, x, y)| (z, x, row_prefix(y)))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let never = AtomicBool::new(false);
    let done = AtomicUsize::new(0);
    let pages = parallel(&prefixes, threads, &never, |(z, x, p)| {
        let got = list(
            net,
            TERRARIUM_BUCKET,
            &format!("{TERRARIUM_PREFIX}/{z}/{x}/{p}"),
            listing,
        );
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        if n.is_multiple_of(500) {
            report(&format!("Terrarium listings: {n} of {}", prefixes.len()));
        }
        got
    });
    let mut out = BTreeMap::new();
    for page in pages {
        for (key, object) in page.expect("not cancelled")? {
            let Some(path) = key
                .strip_prefix(&format!("{TERRARIUM_PREFIX}/"))
                .and_then(|k| k.strip_suffix(".png"))
            else {
                continue;
            };
            let parts: Vec<&str> = path.split('/').collect();
            let parsed = match parts[..] {
                [z, x, y] => z
                    .parse()
                    .ok()
                    .zip(x.parse().ok())
                    .zip(y.parse().ok())
                    .map(|((z, x), y)| (z, x, y)),
                _ => None,
            };
            if let Some(tile) = parsed.filter(|t| tiles.contains(t)) {
                out.insert(tile, object);
            }
        }
    }
    if let Some(missing) = tiles.iter().find(|t| !out.contains_key(t)) {
        return Err(BakeFailure::Bake(format!(
            "Terrarium tile {}/{}/{} is not in its bucket's listing",
            missing.0, missing.1, missing.2
        )));
    }
    Ok(out)
}

fn list_glo30(
    net: &dyn Transport,
    cells: &BTreeSet<CellKey>,
    threads: usize,
    listing: &Listing,
) -> Result<BTreeMap<CellKey, Option<Object>>, BakeFailure> {
    let cells: Vec<CellKey> = cells.iter().copied().collect();
    let never = AtomicBool::new(false);
    let bucket = SourceManifest::glo30_base();
    let got = parallel(&cells, threads, &never, |&(lat, lon)| {
        let name = glo30::name(lat, lon);
        let key = format!("{name}/{name}.tif");
        list(net, &bucket, &key, listing)
            .map(|objects| objects.into_iter().find(|(k, _)| *k == key).map(|o| o.1))
    });
    let mut out = BTreeMap::new();
    for (cell, object) in cells.iter().zip(got) {
        out.insert(*cell, object.expect("not cancelled")?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_listing_page_reads_its_objects_and_token() {
        let xml = r#"<?xml version="1.0"?><ListBucketResult><Name>b</Name><NextContinuationToken>a+b/c=</NextContinuationToken><IsTruncated>true</IsTruncated><Contents><Key>terrarium/11/1000/0.png</Key><ETag>&quot;6620d2&quot;</ETag><Size>32692</Size></Contents><Contents><Key>terrarium/11/1000/0.png.aux.xml</Key><ETag>&quot;2fd5&quot;</ETag><Size>1087</Size></Contents></ListBucketResult>"#;
        let ListingPage { objects, next } = parse_listing(xml).unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].0, "terrarium/11/1000/0.png");
        assert_eq!(
            objects[0].1,
            Object {
                etag: "6620d2".into(),
                bytes: 32692
            }
        );
        assert_eq!(next.as_deref(), Some("a+b/c="));
        // A cut page with no token, or one that does not say, is never the listing's end.
        assert!(
            parse_listing("<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>")
                .is_err()
        );
        assert!(parse_listing("<ListBucketResult></ListBucketResult>").is_err());
        let last =
            parse_listing("<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>")
                .unwrap();
        assert!(last.objects.is_empty() && last.next.is_none());
        assert_eq!(encode("a+b/c="), "a%2Bb/c%3D");
        assert_eq!(row_prefix(7), "7");
        assert_eq!(row_prefix(70), "7");
        assert_eq!(row_prefix(700), "7");
        assert_eq!(row_prefix(1234), "12");
    }

    #[test]
    fn a_squares_owners_are_its_halos_squares_wrapped_and_never_polar() {
        let o = owners(SquareKey { lat: 10, lon: 179 });
        assert_eq!(o.len(), 9);
        assert!(o.contains(&SquareKey { lat: 11, lon: -180 }));
        let o = owners(SquareKey { lat: 88, lon: 0 });
        assert!(o.iter().all(|k| k.lat <= 88), "{o:?}");
        assert!(
            o.len() > 9,
            "the halo spans squares east and west far north"
        );
    }
}
