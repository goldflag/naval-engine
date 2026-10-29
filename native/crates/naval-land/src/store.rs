//! The store: releases' records, their manifests and their squares by content, with pins, leases and a size bound.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use naval_land_format::manifest::{Conflict, Recorded};
use naval_land_format::{
    Class, Digest, Entry, FinalSquare, Manifest, Release, ReleaseFile, ReleaseId, SquareKey,
    SquareTable,
};

/// A release's file in its folder.
pub const RELEASE_FILE: &str = "release.json";
/// Its square table.
pub const TABLE_FILE: &str = "squares.nlt";
/// Its source manifest, when it has one.
pub const SOURCES_FILE: &str = "sources.json";
/// Its manifest.
pub const MANIFEST_FILE: &str = "manifest.txt";
/// The store's lock, in its folder.
pub const LOCK_FILE: &str = "store.lock";
/// A store's size bound when its caller sets none: a few thousand land squares.
pub const DEFAULT_BOUND_BYTES: u64 = 4 << 30;

/// A square's bytes in the store: its release, its key and their SHA-256.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Content {
    pub release: ReleaseId,
    pub square: SquareKey,
    pub sha256: Digest,
}

/// Why the store refused.
#[derive(Debug)]
pub enum StoreError {
    Io {
        path: PathBuf,
        error: io::Error,
    },
    /// Bytes that are not what they claim.
    Refused(naval_land_format::Error),
    /// A bake whose bytes are not the ones the manifest records: not stored.
    Conflict(Conflict),
    /// No such release is installed.
    UnknownRelease(ReleaseId),
    /// The release's table says the square is never baked.
    NotCandidate {
        square: SquareKey,
        class: Class,
    },
    /// The bytes are another square's.
    NotItsSquare {
        square: SquareKey,
        holds: SquareKey,
    },
    /// A square a closure needs that this computer does not hold: not baked, or its bytes gone.
    Missing {
        square: SquareKey,
    },
    /// Another store holds the folder: its pins and leases are its own, so a second could evict what it holds.
    InUse {
        root: PathBuf,
    },
}

/// A square's place in words: `31°S 142°W`.
pub(crate) fn near(key: SquareKey) -> String {
    format!(
        "{}°{} {}°{}",
        key.lat.abs(),
        if key.lat >= 0 { 'N' } else { 'S' },
        key.lon.abs(),
        if key.lon >= 0 { 'E' } else { 'W' }
    )
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io { path, error } => write!(f, "{}: {error}", path.display()),
            StoreError::Refused(e) => e.fmt(f),
            StoreError::Conflict(c) => c.fmt(f),
            StoreError::InUse { root } => {
                write!(
                    f,
                    "the land store at {} is in use by another game",
                    root.display()
                )
            }
            StoreError::UnknownRelease(id) => {
                write!(f, "land release {id} is not on this computer")
            }
            StoreError::NotCandidate { square, class } => {
                write!(
                    f,
                    "{square} is {} in its release: it is never baked",
                    class.name()
                )
            }
            StoreError::NotItsSquare { square, holds } => {
                write!(f, "the bytes given for {square} are {holds}'s")
            }
            StoreError::Missing { square } => write!(
                f,
                "the coast near {} is not on this computer",
                near(*square)
            ),
        }
    }
}

impl std::error::Error for StoreError {}

pub(crate) fn at(path: &Path) -> impl FnOnce(io::Error) -> StoreError + '_ {
    move |error| StoreError::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// What the store answers for a square.
#[derive(Debug)]
pub enum Answer {
    /// Open sea with no file: the table calls the square ocean or polar, or its manifest line is `empty`.
    Sea,
    /// Its final square, leased while the lease lives.
    Land(Lease),
    /// Not on this computer: a candidate with no manifest line yet (`recorded` none: bake it), or one whose recorded
    /// bytes are not here (bake it again, which must give them, or import them).
    Missing { recorded: Option<Digest> },
}

/// A release's record: its file, its square table and its source manifest (none for a synthetic release).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseRecord {
    pub release: Vec<u8>,
    pub table: Vec<u8>,
    pub sources: Option<Vec<u8>>,
}

/// What the store holds, and its bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Usage {
    /// The squares' bytes on disk.
    pub bytes: u64,
    pub bound: u64,
    pub squares: usize,
    /// Of them, those a holder pins and those a reader leases.
    pub pinned: usize,
    pub leased: usize,
    /// Of them, those a world's [`Requests`] keep.
    pub requested: usize,
    /// How many times eviction stopped over the bound because every square left was pinned, leased or requested.
    pub over: u64,
}

/// An installed release: its record and its manifest as last read.
struct Held {
    release: Release,
    table: SquareTable,
    manifest: Manifest,
}

/// A square on disk: its size and when it was last used (the store's own clock).
struct Kept {
    size: u64,
    used: u64,
}

#[derive(Default)]
struct Inner {
    releases: BTreeMap<ReleaseId, Held>,
    files: BTreeMap<Content, Kept>,
    clock: u64,
    bytes: u64,
    /// Whether its owner has pinned every reference it holds on disk: nothing is evicted before.
    settled: bool,
    pins: BTreeMap<String, BTreeSet<Content>>,
    leases: BTreeMap<Content, usize>,
    /// Each live [`Requests`]' squares, by its number: kept by key, whatever bytes they come to have.
    requests: BTreeMap<u64, BTreeSet<(ReleaseId, SquareKey)>>,
    next_requests: u64,
    /// Eviction passes that stopped over the bound ([`Usage::over`]), and whether the last one did.
    over: u64,
    is_over: bool,
    /// Squares recorded since the store was opened ([`Store::changes`]).
    changes: u64,
    /// The folder's lock, held while the store or any lease or request of it lives: no other store opens it meanwhile.
    _owner: Option<File>,
}

impl Inner {
    fn held(&mut self, id: ReleaseId) -> Result<&mut Held, StoreError> {
        self.releases
            .get_mut(&id)
            .ok_or(StoreError::UnknownRelease(id))
    }

    fn pinned(&self, c: &Content) -> bool {
        self.pins.values().any(|set| set.contains(c))
    }

    fn requested(&self, c: &Content) -> bool {
        let key = (c.release, c.square);
        self.requests.values().any(|set| set.contains(&key))
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn keep(&mut self, content: Content, size: u64) {
        let used = self.tick();
        match self.files.get_mut(&content) {
            Some(kept) => kept.used = used,
            None => {
                self.files.insert(content, Kept { size, used });
                self.bytes += size;
            }
        }
    }

    fn forget(&mut self, content: &Content) {
        if let Some(kept) = self.files.remove(content) {
            self.bytes -= kept.size;
        }
    }
}

/// A reader's hold on a square: its bytes, and the square kept on disk while any lease of it lives.
pub struct Lease {
    content: Content,
    bytes: Arc<[u8]>,
    inner: Arc<Mutex<Inner>>,
}

impl Lease {
    pub fn content(&self) -> Content {
        self.content
    }
    /// The square's file, its digest checked when it was read.
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }
    /// The square's final layer, no block decoded yet.
    pub fn square(&self) -> Result<FinalSquare, StoreError> {
        FinalSquare::read(self.bytes.clone()).map_err(StoreError::Refused)
    }
}

impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lease")
            .field("content", &self.content)
            .finish()
    }
}

impl Clone for Lease {
    fn clone(&self) -> Self {
        *lock(&self.inner).leases.entry(self.content).or_default() += 1;
        Self {
            content: self.content,
            bytes: self.bytes.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        unlease(&self.inner, &self.content);
    }
}

fn unlease(inner: &Mutex<Inner>, content: &Content) {
    let mut inner = lock(inner);
    if let Some(n) = inner.leases.get_mut(content) {
        *n -= 1;
        if *n == 0 {
            inner.leases.remove(content);
        }
    }
}

/// A world's requests ([`Store::requests`]): the squares it has asked the store for, each leased by its key (whatever
/// bytes the store holds for it or comes to hold: a square still to be baked included) from the request until the
/// requests are dropped. A square a pending admission needs is requested before it is read or asked of a baker, so a
/// full store never evicts it before the boundary admits it: past its bound the store keeps it and goes over.
pub struct Requests {
    number: u64,
    inner: Arc<Mutex<Inner>>,
}

impl Requests {
    /// Square `square` of release `release` requested, kept from now on. Again: nothing changes.
    pub fn request(&self, release: ReleaseId, square: SquareKey) {
        if let Some(set) = lock(&self.inner).requests.get_mut(&self.number) {
            set.insert((release, square));
        }
    }
}

impl std::fmt::Debug for Requests {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Requests")
            .field("number", &self.number)
            .finish()
    }
}

impl Drop for Requests {
    /// Its squares are no longer kept for it: the next eviction may take them.
    fn drop(&mut self) {
        lock(&self.inner).requests.remove(&self.number);
    }
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Land releases' squares on this machine.
pub struct Store {
    root: PathBuf,
    bound: u64,
    inner: Arc<Mutex<Inner>>,
}

static WRITES: AtomicU64 = AtomicU64::new(0);

/// `bytes` at `path`, whole or not at all: written beside it, synced and renamed over it.
pub(crate) fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let dir = path.parent().expect("a store path has a folder");
    fs::create_dir_all(dir).map_err(at(dir))?;
    let name = path.file_name().unwrap().to_string_lossy();
    let temporary = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(at(path))
}

fn read_manifest(path: &Path) -> Result<Manifest, StoreError> {
    match fs::read_to_string(path) {
        Ok(text) => {
            // A line a writer did not finish was never written.
            let whole = text.rfind('\n').map_or(0, |n| n + 1);
            Manifest::parse(&text[..whole]).map_err(StoreError::Refused)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(at(path)(e)),
    }
}

impl Store {
    /// The store at `root` (a game's `<data>/land`), made if absent, to be kept within `bound` bytes of squares once
    /// it is [settled](Self::settle). One store a folder: it locks the folder ([`LOCK_FILE`]) until it and every lease
    /// and request of it are dropped, and another open meanwhile, in this process or another, is refused
    /// ([`StoreError::InUse`]), since pins and leases are a store's own and a second store could evict what the first
    /// holds. Every installed release is read (its file against its folder's id, its table against the file); square
    /// files are counted, oldest use first, and a file its manifest does not record is removed.
    pub fn open(root: impl Into<PathBuf>, bound: u64) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(at(&root))?;
        let lock_path = root.join(LOCK_FILE);
        let owner = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(at(&lock_path))?;
        match owner.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(StoreError::InUse { root }),
            Err(std::fs::TryLockError::Error(e)) => return Err(at(&lock_path)(e)),
        }
        let mut inner = Inner {
            _owner: Some(owner),
            ..Inner::default()
        };
        let mut found: Vec<(SystemTime, Content, u64)> = vec![];
        for entry in fs::read_dir(&root).map_err(at(&root))? {
            let entry = entry.map_err(at(&root))?;
            let Some(id) = entry.file_name().to_str().and_then(ReleaseId::parse) else {
                continue;
            };
            let dir = entry.path();
            // The release file is written last: a folder without it is an install that did not finish.
            let release = match fs::read(dir.join(RELEASE_FILE)) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(at(&dir.join(RELEASE_FILE))(e)),
            };
            let release = ReleaseFile::parse(&release).map_err(StoreError::Refused)?;
            if release.id != id {
                return Err(StoreError::Refused(naval_land_format::Error::Corrupt(
                    format!("{}'s release file is release {}", dir.display(), release.id),
                )));
            }
            let table_path = dir.join(TABLE_FILE);
            let table = fs::read(&table_path).map_err(at(&table_path))?;
            if !release.file.square_table.matches(&table) {
                return Err(StoreError::Refused(naval_land_format::Error::Corrupt(
                    format!("{} is not its release's square table", table_path.display()),
                )));
            }
            let table = SquareTable::decode(&table).map_err(StoreError::Refused)?;
            // The squares on disk first, then the manifest: a line is written before its bytes, so every file seen
            // here is recorded in the manifest read after.
            let mut files = vec![];
            for square in fs::read_dir(&dir).map_err(at(&dir))? {
                let square = square.map_err(at(&dir))?;
                let Some(key) = square.file_name().to_str().and_then(SquareKey::parse) else {
                    continue;
                };
                let folder = square.path();
                for file in fs::read_dir(&folder).map_err(at(&folder))? {
                    let file = file.map_err(at(&folder))?;
                    let name = file.file_name().to_string_lossy().into_owned();
                    let digest = name.strip_suffix(".nsq").and_then(Digest::parse);
                    let meta = file.metadata().map_err(at(&file.path()))?;
                    files.push((file.path(), name, key, digest, meta));
                }
            }
            let manifest = read_manifest(&dir.join(MANIFEST_FILE))?;
            for (path, name, key, digest, meta) in files {
                let recorded = digest.filter(|d| manifest.get(key) == Some(Entry::Data(*d)));
                match recorded {
                    Some(sha256) => found.push((
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                        Content {
                            release: id,
                            square: key,
                            sha256,
                        },
                        meta.len(),
                    )),
                    // Bytes no line records.
                    None if name.ends_with(".nsq") => {
                        let _ = fs::remove_file(path);
                    }
                    // A write that did not finish (an hour old: not another process's, still writing).
                    None if name.ends_with(".tmp")
                        && meta
                            .modified()
                            .ok()
                            .and_then(|t| t.elapsed().ok())
                            .is_some_and(|age| age.as_secs() > 3600) =>
                    {
                        let _ = fs::remove_file(path);
                    }
                    None => {}
                }
            }
            inner.releases.insert(
                id,
                Held {
                    release,
                    table,
                    manifest,
                },
            );
        }
        found.sort_by_key(|f| (f.0, f.1));
        for (_, content, size) in found {
            inner.keep(content, size);
        }
        Ok(Self {
            root,
            bound,
            inner: Arc::new(Mutex::new(inner)),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// A release's folder.
    pub fn release_dir(&self, id: ReleaseId) -> PathBuf {
        self.root.join(id.to_string())
    }

    /// Where a square's bytes are kept.
    pub fn square_path(&self, c: &Content) -> PathBuf {
        self.release_dir(c.release)
            .join(c.square.name())
            .join(format!("{}.nsq", c.sha256))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock(&self.inner)
    }

    /// Installs a release from its record: its file, its square table and (when it names one) its source manifest,
    /// each checked against the file. Installing it again changes nothing.
    pub fn install(
        &self,
        release: &[u8],
        table: &[u8],
        sources: Option<&[u8]>,
    ) -> Result<ReleaseId, StoreError> {
        let refuse = |why: &str| StoreError::Refused(naval_land_format::Error::Corrupt(why.into()));
        let parsed = ReleaseFile::parse(release).map_err(StoreError::Refused)?;
        if !parsed.file.square_table.matches(table) {
            return Err(refuse("the square table is not the one its release names"));
        }
        let decoded = SquareTable::decode(table).map_err(StoreError::Refused)?;
        match (parsed.file.sources, sources) {
            (None, None) => {}
            (Some(named), Some(bytes)) if named.matches(bytes) => {}
            (Some(_), None) => return Err(refuse("the release's source manifest is missing")),
            _ => {
                return Err(refuse(
                    "the source manifest is not the one its release names",
                ));
            }
        }
        let id = parsed.id;
        let mut inner = self.lock();
        if inner.releases.contains_key(&id) {
            return Ok(id);
        }
        let dir = self.release_dir(id);
        write_whole(&dir.join(TABLE_FILE), table)?;
        if let Some(sources) = sources {
            write_whole(&dir.join(SOURCES_FILE), sources)?;
        }
        write_whole(&dir.join(RELEASE_FILE), release)?;
        let manifest = read_manifest(&dir.join(MANIFEST_FILE))?;
        inner.releases.insert(
            id,
            Held {
                release: parsed,
                table: decoded,
                manifest,
            },
        );
        Ok(id)
    }

    /// The installed releases.
    pub fn releases(&self) -> Vec<ReleaseId> {
        self.lock().releases.keys().copied().collect()
    }

    /// A release's record as installed: its file, its square table and its source manifest.
    pub fn record(&self, id: ReleaseId) -> Result<ReleaseRecord, StoreError> {
        let named = self.lock().held(id)?.release.file.sources.is_some();
        let dir = self.release_dir(id);
        let read = |name: &str| fs::read(dir.join(name)).map_err(at(&dir.join(name)));
        Ok(ReleaseRecord {
            release: read(RELEASE_FILE)?,
            table: read(TABLE_FILE)?,
            sources: if named {
                Some(read(SOURCES_FILE)?)
            } else {
                None
            },
        })
    }

    /// What the release's table says a square is.
    pub fn class(&self, id: ReleaseId, key: SquareKey) -> Result<Class, StoreError> {
        Ok(self.lock().held(id)?.table.class(key))
    }

    /// The square's manifest line, read again from disk when the store has none for it (another process may have
    /// baked it since).
    pub fn recorded(&self, id: ReleaseId, key: SquareKey) -> Result<Option<Entry>, StoreError> {
        let path = self.release_dir(id).join(MANIFEST_FILE);
        let mut inner = self.lock();
        let held = inner.held(id)?;
        if held.manifest.get(key).is_none() {
            held.manifest = read_manifest(&path)?;
        }
        Ok(held.manifest.get(key))
    }

    /// Records `entry` for `key` in the release's manifest, under the manifest file's lock and against its lines as
    /// they are on disk then: a new line appended and synced, nothing when the line already says it, a conflict (and
    /// nothing written) when it says otherwise. A torn last line is cut off first.
    fn append(
        &self,
        held: &mut Held,
        id: ReleaseId,
        key: SquareKey,
        entry: Entry,
    ) -> Result<Recorded, StoreError> {
        let path = self.release_dir(id).join(MANIFEST_FILE);
        let io = at(&path);
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(&path)
            .map_err(at(&path))?;
        let written = (|| -> io::Result<Result<Recorded, StoreError>> {
            file.lock()?;
            let mut text = String::new();
            file.read_to_string(&mut text)?;
            if !text.ends_with('\n') && !text.is_empty() {
                let whole = text.rfind('\n').map_or(0, |n| n + 1);
                file.set_len(whole as u64)?;
                text.truncate(whole);
            }
            held.manifest = match Manifest::parse(&text) {
                Ok(manifest) => manifest,
                Err(e) => return Ok(Err(StoreError::Refused(e))),
            };
            Ok(match held.manifest.check(key, entry) {
                Err(conflict) => Err(StoreError::Conflict(conflict)),
                Ok(Recorded::Already) => Ok(Recorded::Already),
                Ok(Recorded::New(line)) => {
                    file.write_all(line.as_bytes())?;
                    file.sync_data()?;
                    held.manifest
                        .record(key, entry)
                        .expect("checked under the lock");
                    Ok(Recorded::New(line))
                }
            })
        })();
        let _ = file.unlock();
        written.map_err(io)?
    }

    /// Keeps a baked square: its manifest line recorded (or found already recorded), then its bytes. Refused when the
    /// release never bakes the square, when the bytes are not a whole final square of `key`, and when the manifest
    /// records other bytes or `empty` (the bake's result is then kept nowhere).
    pub fn put(&self, id: ReleaseId, key: SquareKey, bytes: &[u8]) -> Result<Recorded, StoreError> {
        let square = FinalSquare::read(bytes.to_vec()).map_err(StoreError::Refused)?;
        if square.key() != key {
            return Err(StoreError::NotItsSquare {
                square: key,
                holds: square.key(),
            });
        }
        if !square.verify() {
            return Err(StoreError::Refused(naval_land_format::Error::Corrupt(
                format!("{key}'s digests do not match its bytes"),
            )));
        }
        let content = Content {
            release: id,
            square: key,
            sha256: Digest::of(bytes),
        };
        let mut inner = self.lock();
        let held = inner.held(id)?;
        let class = held.table.class(key);
        if class != Class::Candidate {
            return Err(StoreError::NotCandidate { square: key, class });
        }
        let recorded = self.append(held, id, key, Entry::Data(content.sha256))?;
        let path = self.square_path(&content);
        if !inner.files.contains_key(&content) && !path.exists() {
            write_whole(&path, bytes)?;
        }
        inner.keep(content, bytes.len() as u64);
        inner.changes += 1;
        self.evict_locked(&mut inner);
        Ok(recorded)
    }

    /// Records that a candidate square holds no land within the keep distance.
    pub fn put_empty(&self, id: ReleaseId, key: SquareKey) -> Result<Recorded, StoreError> {
        let mut inner = self.lock();
        let held = inner.held(id)?;
        let class = held.table.class(key);
        if class != Class::Candidate {
            return Err(StoreError::NotCandidate { square: key, class });
        }
        let recorded = self.append(held, id, key, Entry::Empty)?;
        inner.changes += 1;
        Ok(recorded)
    }

    /// A square as the world reads it: open sea, its bytes (leased, their digest checked), or missing.
    pub fn read(&self, id: ReleaseId, key: SquareKey) -> Result<Answer, StoreError> {
        let content = {
            let path = self.release_dir(id).join(MANIFEST_FILE);
            let mut inner = self.lock();
            let held = inner.held(id)?;
            if held.table.class(key) != Class::Candidate {
                return Ok(Answer::Sea);
            }
            if held.manifest.get(key).is_none() {
                held.manifest = read_manifest(&path)?;
            }
            let sha256 = match held.manifest.get(key) {
                None => return Ok(Answer::Missing { recorded: None }),
                Some(Entry::Empty) => return Ok(Answer::Sea),
                Some(Entry::Data(sha256)) => sha256,
            };
            let content = Content {
                release: id,
                square: key,
                sha256,
            };
            if !inner.files.contains_key(&content) {
                // Not counted: perhaps another process kept it since.
                match fs::metadata(self.square_path(&content)) {
                    Ok(meta) => inner.keep(content, meta.len()),
                    Err(_) => {
                        return Ok(Answer::Missing {
                            recorded: Some(sha256),
                        });
                    }
                }
            }
            *inner.leases.entry(content).or_default() += 1;
            content
        };
        // Leased: nothing evicts it while it is read.
        let path = self.square_path(&content);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                let mut inner = self.lock();
                if e.kind() == io::ErrorKind::NotFound {
                    inner.forget(&content);
                }
                drop(inner);
                unlease(&self.inner, &content);
                return match e.kind() {
                    io::ErrorKind::NotFound => Ok(Answer::Missing {
                        recorded: Some(content.sha256),
                    }),
                    _ => Err(at(&path)(e)),
                };
            }
        };
        if Digest::of(&bytes) != content.sha256 {
            let _ = fs::remove_file(&path);
            self.lock().forget(&content);
            unlease(&self.inner, &content);
            return Err(StoreError::Refused(naval_land_format::Error::Corrupt(
                format!(
                    "{} did not match its digest and was removed",
                    path.display()
                ),
            )));
        }
        self.lock().keep(content, bytes.len() as u64);
        // Its use, for the next session's order: best effort.
        if let Ok(file) = OpenOptions::new().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Ok(Answer::Land(Lease {
            content,
            bytes: bytes.into(),
            inner: self.inner.clone(),
        }))
    }

    /// Pins `contents` for `holder` (a world, a save, a checkpoint, a clip), replacing what it pinned before.
    pub fn pin(&self, holder: &str, contents: impl IntoIterator<Item = Content>) {
        let set: BTreeSet<Content> = contents.into_iter().collect();
        let mut inner = self.lock();
        if set.is_empty() {
            inner.pins.remove(holder);
        } else {
            inner.pins.insert(holder.to_string(), set);
        }
        self.evict_locked(&mut inner);
    }

    /// Drops everything `holder` pinned.
    pub fn unpin(&self, holder: &str) {
        self.pin(holder, []);
    }

    pub fn is_pinned(&self, content: &Content) -> bool {
        self.lock().pinned(content)
    }

    /// New requests, holding nothing yet: a world's, for the squares its admissions need ([`Requests`]).
    pub fn requests(&self) -> Requests {
        let mut inner = self.lock();
        let number = inner.next_requests;
        inner.next_requests += 1;
        inner.requests.insert(number, BTreeSet::new());
        Requests {
            number,
            inner: self.inner.clone(),
        }
    }

    /// A count that grows each time a square is recorded here ([`Self::put`], [`Self::put_empty`]): a reader waiting for
    /// squares looks again only once it has changed.
    pub fn changes(&self) -> u64 {
        self.lock().changes
    }

    /// Whether the store holds a square's bytes.
    pub fn holds(&self, content: &Content) -> bool {
        self.lock().files.contains_key(content)
    }

    pub fn usage(&self) -> Usage {
        let inner = self.lock();
        Usage {
            bytes: inner.bytes,
            bound: self.bound,
            squares: inner.files.len(),
            pinned: inner.files.keys().filter(|c| inner.pinned(c)).count(),
            leased: inner.leases.len(),
            requested: inner.files.keys().filter(|c| inner.requested(c)).count(),
            over: inner.over,
        }
    }

    /// Says that every reference on disk (saves, autosaves, checkpoints, clips) is pinned, and evicts down to the
    /// bound: what it removed. Until then the store evicts nothing, so a square a save pins is never lost to a store
    /// opened before its pins were read.
    pub fn settle(&self) -> Vec<Content> {
        let mut inner = self.lock();
        inner.settled = true;
        self.evict_locked(&mut inner)
    }

    /// Evicts down to the bound, least recently used first, never a pinned, leased or requested square, and nothing
    /// before the store is settled: what it removed. When only those are left it stays over the bound and says so.
    pub fn evict(&self) -> Vec<Content> {
        let mut inner = self.lock();
        self.evict_locked(&mut inner)
    }

    fn evict_locked(&self, inner: &mut Inner) -> Vec<Content> {
        let mut gone = vec![];
        if !inner.settled {
            return gone;
        }
        while inner.bytes > self.bound {
            let victim = inner
                .files
                .iter()
                .filter(|(c, _)| {
                    !inner.leases.contains_key(c) && !inner.pinned(c) && !inner.requested(c)
                })
                .min_by_key(|(c, kept)| (kept.used, **c))
                .map(|(c, _)| *c);
            let Some(content) = victim else {
                break;
            };
            inner.forget(&content);
            // A file that will not go stays on disk uncounted; the next open counts it again.
            let _ = fs::remove_file(self.square_path(&content));
            gone.push(content);
        }
        // Over the bound with nothing left to evict: kept rather than lost, counted, and said each time it goes over.
        let over = inner.bytes > self.bound;
        if over {
            inner.over += 1;
            if !inner.is_over {
                eprintln!(
                    "naval-land: the land store at {} holds {} MB over its bound of {} MB: every square left is \
                     pinned, leased or requested",
                    self.root.display(),
                    (inner.bytes - self.bound) >> 20,
                    self.bound >> 20
                );
            }
        }
        inner.is_over = over;
        gone
    }

    /// A release's manifest as the store last read it.
    pub fn manifest(&self, id: ReleaseId) -> Result<Manifest, StoreError> {
        Ok(self.lock().held(id)?.manifest.clone())
    }
}
