//! Power lines over the theatre's land (native/docs/terrain/infra.md, "Power lines"; the meshes are
//! `naval_terrain::infra::power`'s, the reader `naval_overture`): Overture Maps' overhead lines, power towers and
//! communication masts, read half a degree square at a time from its public bucket on one worker thread
//! (`theatre-power`) and kept under the user data directory's `overture-cache`, a folder a release.
//!
//! - **The release** is the bucket's newest (listed once a run), else the newest kept whole that opens when offline;
//!   its files' footers are kept with it (about 21 MB, read once; [`READY`] once all are), and the folders of older
//!   releases go when a newer one is opened.
//! - **A cell** ([`CELL_DEG`] square) is read once, its row groups by their bounds, and kept as the features it holds
//!   (`cells/<lat>_<lon>.bin`): a scenario's region reads the few under it once, and every scenario there shares them.
//! - **A chunk** asks [`about`] for the lines that pass through it, whole (so every chunk a line crosses places its
//!   towers alike), and the masts within [`MAST_REACH_M`] of it (a span's far tower's mapped height); none while a
//!   cell it needs is being read. [`epoch`] counts the cells had, so a chunk made meanwhile is made again
//!   (`theatre::infra`); one made while a cell could not be read is incomplete, made again as for a missing tile.
//!   Longitudes are unwrapped about the theatre's ([`near_lon`]), so a theatre on the date line reads the cells either
//!   side of it.
//!
//! Switched with the infrastructure (`infra::enabled`); `NAVAL_POWER=off` leaves it off.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use glam::Vec2;
use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_overture::{Fetch, LineClass, PowerCell, Release};
use naval_terrain::buildings::{CHUNK_M, chunk_corner};
use naval_terrain::infra::power::{LineKind, Mast, MastKind, Power, PowerLine};

/// A cell's side (degrees of latitude and longitude).
pub const CELL_DEG: f64 = 0.5;
/// How far about a chunk its masts are taken (m): the mapped height of the far tower of a span it draws.
pub const MAST_REACH_M: f64 = 2500.0;
/// A cell (or the release) that could not be read is tried again after this.
const RETRY_AFTER: Duration = Duration::from_secs(600);
/// Bytes one response may hold (a column chunk of a row group is a few megabytes).
const MAX_BODY: u64 = 96 << 20;
const USER_AGENT: &str = concat!(
    "BlueWater/",
    env!("CARGO_PKG_VERSION"),
    " (theatre power lines; Overture Maps cache on disk)"
);

pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_POWER").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    super::infra::enabled() && !off
}

/// Where the releases are kept: `NAVAL_OVERTURE_DIR` when set, else the user data directory's `overture-cache`.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_OVERTURE_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    crate::host::data_dir().join("overture-cache")
}

fn log_once(kind: &str, line: &str) {
    static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged
        .get_or_insert_with(HashSet::new)
        .insert(kind.to_owned())
    {
        eprintln!("the theatre's power lines: {line}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The bucket over HTTPS.

struct Http;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_body(Some(Duration::from_secs(60)))
            .timeout_global(Some(Duration::from_secs(180)))
            .http_status_as_error(false)
            .max_idle_connections_per_host(8)
            .build()
            .into()
    })
}

impl Fetch for Http {
    fn get(&self, url: &str, range: Option<std::ops::Range<u64>>) -> Result<Vec<u8>, String> {
        let mut last = String::new();
        for attempt in 0..3u32 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_millis(500 << attempt));
            }
            let mut request = agent().get(url);
            if let Some(r) = &range {
                request = request.header("Range", &format!("bytes={}-{}", r.start, r.end - 1));
            }
            let mut response = match request.call() {
                Ok(response) => response,
                Err(e) => {
                    last = e.to_string();
                    continue;
                }
            };
            let status = response.status().as_u16();
            if !matches!(status, 200 | 206) {
                last = format!("HTTP {status}");
                if status < 500 {
                    break;
                }
                continue;
            }
            match response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
            {
                Ok(bytes) => {
                    if let Some(r) = &range
                        && bytes.len() as u64 != r.end - r.start
                    {
                        last = format!("{} bytes of {}", bytes.len(), r.end - r.start);
                        continue;
                    }
                    return Ok(bytes);
                }
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!("{url}: {last}"))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Cells.

/// A line as kept: its kind, key, bounds and points (longitude, latitude).
struct CellLine {
    kind: LineKind,
    key: u64,
    min: [f64; 2],
    max: [f64; 2],
    points: Vec<[f64; 2]>,
}

/// A cell's features as kept: its lines, and the masts that matter (power towers with a height, communication masts;
/// the poles carry nothing the lines' own do not).
struct Cell {
    lines: Vec<CellLine>,
    masts: Vec<naval_overture::Mast>,
}

impl Cell {
    fn of(read: PowerCell) -> Self {
        let lines = read
            .lines
            .into_iter()
            .filter(|l| l.points.len() >= 2)
            .map(|l| {
                let (min, max) =
                    l.points
                        .iter()
                        .fold(([f64::MAX; 2], [f64::MIN; 2]), |(lo, hi), p| {
                            (
                                [lo[0].min(p[0]), lo[1].min(p[1])],
                                [hi[0].max(p[0]), hi[1].max(p[1])],
                            )
                        });
                CellLine {
                    kind: match l.class {
                        LineClass::Power => LineKind::Transmission,
                        LineClass::Minor => LineKind::Distribution,
                    },
                    key: l.key,
                    min,
                    max,
                    points: l.points,
                }
            })
            .collect();
        let masts = read
            .masts
            .into_iter()
            .filter(|m| match m.kind {
                naval_overture::MastKind::PowerTower => m.height > 0.0,
                naval_overture::MastKind::Communication => true,
                naval_overture::MastKind::PowerPole => false,
            })
            .collect();
        Self { lines, masts }
    }
}

enum Slot {
    Queued,
    Had(Arc<Cell>),
    Failed(Instant),
}

/// What the worker has done, for the RPC.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct PowerStatus {
    pub release: Option<String>,
    pub cells_had: usize,
    pub cells_queued: usize,
    pub cells_failed: usize,
    /// Lines and masts the cells hold.
    pub lines: usize,
    pub masts: usize,
    /// The last cell read from the bucket: its row groups, megabytes and milliseconds.
    pub last_read_groups: usize,
    pub last_read_mb: u64,
    pub last_read_ms: u64,
}

struct State {
    cells: HashMap<[i32; 2], Slot>,
    queue: VecDeque<[i32; 2]>,
    worker: bool,
    status: PowerStatus,
}

struct Loader {
    state: Mutex<State>,
    wake: Condvar,
}

static EPOCH: AtomicU64 = AtomicU64::new(0);

fn loader() -> &'static Loader {
    static LOADER: OnceLock<Loader> = OnceLock::new();
    LOADER.get_or_init(|| Loader {
        state: Mutex::new(State {
            cells: HashMap::new(),
            queue: VecDeque::new(),
            worker: false,
            status: PowerStatus::default(),
        }),
        wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    loader().state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cells had (or given up on) so far: a chunk made while one it needed was being read is made again when this moves.
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

pub fn status() -> PowerStatus {
    let state = lock();
    PowerStatus {
        // Those waiting and the one being read.
        cells_queued: state
            .cells
            .values()
            .filter(|s| matches!(s, Slot::Queued))
            .count(),
        ..state.status.clone()
    }
}

/// Cells around the world in longitude.
const CELLS_AROUND: i32 = (360.0 / CELL_DEG) as i32;

/// The cell holding (longitude, latitude) `p`, its longitude's index unwrapped (see [`wrap_cell`]).
fn cell_of(p: [f64; 2]) -> [i32; 2] {
    [
        (p[1] / CELL_DEG).floor() as i32,
        (p[0] / CELL_DEG).floor() as i32,
    ]
}

/// Cell `c` with its longitude's index wrapped into −180° to 180°.
fn wrap_cell(c: [i32; 2]) -> [i32; 2] {
    [
        c[0],
        (c[1] + CELLS_AROUND / 2).rem_euclid(CELLS_AROUND) - CELLS_AROUND / 2,
    ]
}

/// Longitude `lon` unwrapped to within 180° of `about` (so a box across the date line stays small).
fn near_lon(about: f64, lon: f64) -> f64 {
    about + (lon - about + 540.0).rem_euclid(360.0) - 180.0
}

/// The cells under bounds `b` (longitude unwrapped), each wrapped.
fn cells_over(b: ([f64; 2], [f64; 2])) -> Vec<[i32; 2]> {
    let (lo, hi) = (cell_of(b.0), cell_of(b.1));
    (lo[0]..=hi[0])
        .flat_map(|lat| (lo[1]..=hi[1]).map(move |lon| wrap_cell([lat, lon])))
        .collect()
}

/// The bounds (longitude unwrapped about `geo`'s, latitude) of theatre box `min` to `max` (x east, z south) about
/// `geo`.
fn lon_lat_bounds(geo: LatLon, min: [f64; 2], max: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let mut lo = [f64::MAX; 2];
    let mut hi = [f64::MIN; 2];
    for i in 0..3 {
        for j in 0..3 {
            let x = min[0] + (max[0] - min[0]) * f64::from(i) / 2.0;
            let z = min[1] + (max[1] - min[1]) * f64::from(j) / 2.0;
            let mut p = to_lat_lon(geo, [x, z]);
            p.lon = near_lon(geo.lon, p.lon);
            lo = [lo[0].min(p.lon), lo[1].min(p.lat)];
            hi = [hi[0].max(p.lon), hi[1].max(p.lat)];
        }
    }
    // A little over, for the box's sides bowing between the points taken.
    let pad = 1e-4;
    ([lo[0] - pad, lo[1] - pad], [hi[0] + pad, hi[1] + pad])
}

fn meets(a: ([f64; 2], [f64; 2]), b: ([f64; 2], [f64; 2])) -> bool {
    a.0[0] <= b.1[0] && b.0[0] <= a.1[0] && a.0[1] <= b.1[1] && b.0[1] <= a.1[1]
}

/// The power about chunk `index` of the theatre about `geo` (chunk-local): the lines that pass through it whole, in
/// their keys' order, and the masts within [`MAST_REACH_M`], with whether every cell it needs was had (a cell that
/// could not be read is asked for again after [`RETRY_AFTER`]). None while a cell it needs is being read (asked for
/// here).
pub fn about(geo: LatLon, index: [i32; 2]) -> Option<(Power, bool)> {
    if !enabled() {
        return Some((Power::default(), true));
    }
    let corner = chunk_corner(index);
    let chunk = lon_lat_bounds(
        geo,
        [corner[0] - 20.0, corner[1] - 20.0],
        [corner[0] + CHUNK_M + 20.0, corner[1] + CHUNK_M + 20.0],
    );
    let reach = lon_lat_bounds(
        geo,
        [corner[0] - MAST_REACH_M, corner[1] - MAST_REACH_M],
        [
            corner[0] + CHUNK_M + MAST_REACH_M,
            corner[1] + CHUNK_M + MAST_REACH_M,
        ],
    );
    let mut had = Vec::new();
    let mut waiting = false;
    let mut complete = true;
    {
        let mut state = lock();
        let mut asked = false;
        for key in cells_over(reach) {
            match state.cells.get(&key) {
                Some(Slot::Had(cell)) => had.push(cell.clone()),
                Some(Slot::Queued) => waiting = true,
                Some(Slot::Failed(at)) if at.elapsed() < RETRY_AFTER => complete = false,
                _ => {
                    state.cells.insert(key, Slot::Queued);
                    state.queue.push_back(key);
                    waiting = true;
                    asked = true;
                }
            }
        }
        if asked {
            if !state.worker {
                state.worker = true;
                if let Err(e) = crate::host::spawn_utility("theatre-power", work) {
                    state.worker = false;
                    log_once("thread", &format!("no worker thread: {e}"));
                }
            }
            loader().wake.notify_one();
        }
    }
    if waiting {
        return None;
    }
    let local = |p: &[f64; 2]| {
        let q = to_local(
            geo,
            LatLon {
                lat: p[1],
                lon: p[0],
            },
        );
        Vec2::new((q[0] - corner[0]) as f32, (q[1] - corner[1]) as f32)
    };
    let (mut seen, mut seen_masts) = (HashSet::new(), HashSet::new());
    let mut lines: Vec<(u64, PowerLine)> = Vec::new();
    let mut masts: Vec<(u64, Mast)> = Vec::new();
    for cell in &had {
        for l in &cell.lines {
            let shift = near_lon(geo.lon, l.min[0]) - l.min[0];
            let bounds = ([l.min[0] + shift, l.min[1]], [l.max[0] + shift, l.max[1]]);
            if meets(bounds, chunk) && seen.insert(l.key) {
                lines.push((
                    l.key,
                    PowerLine {
                        kind: l.kind,
                        points: l.points.iter().map(local).collect(),
                    },
                ));
            }
        }
        for m in &cell.masts {
            let at = [near_lon(geo.lon, m.at[0]), m.at[1]];
            if meets((at, at), reach) && seen_masts.insert(m.key) {
                masts.push((
                    m.key,
                    Mast {
                        kind: match m.kind {
                            naval_overture::MastKind::PowerTower => MastKind::PowerTower,
                            naval_overture::MastKind::PowerPole => MastKind::PowerPole,
                            naval_overture::MastKind::Communication => MastKind::Communication,
                        },
                        at: local(&m.at),
                        height: m.height,
                    },
                ));
            }
        }
    }
    lines.sort_by_key(|l| l.0);
    masts.sort_by_key(|m| m.0);
    Some((
        Power {
            lines: lines.into_iter().map(|l| l.1).collect(),
            masts: masts.into_iter().map(|m| m.1).collect(),
        },
        complete,
    ))
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// Whether a folder's name is a release's (`2026-09-23.1`).
fn is_release(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
}

/// A kept release's marker: its listing and every footer were kept.
const READY: &str = "ready";

/// The releases kept whole (marked [`READY`]), newest first.
fn kept_releases(root: &Path) -> Vec<String> {
    let mut kept: Vec<String> = std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| is_release(n) && root.join(n).join(READY).is_file())
                .collect()
        })
        .unwrap_or_default();
    kept.sort_unstable_by(|a, b| b.cmp(a));
    kept
}

/// The release to read: the bucket's newest (older releases' folders then go), else the newest kept whole that opens.
fn open_release() -> Result<Release, String> {
    let root = cache_dir();
    let mut why = Vec::new();
    match naval_overture::latest_release(&Http) {
        Ok(name) => match Release::open(&Http, &root.join(&name), &name) {
            Ok(release) => {
                let _ = std::fs::write(root.join(&name).join(READY), b"");
                if let Ok(entries) = std::fs::read_dir(&root) {
                    for e in entries.flatten() {
                        if let Ok(other) = e.file_name().into_string()
                            && is_release(&other)
                            && other < name
                        {
                            let _ = std::fs::remove_dir_all(e.path());
                        }
                    }
                }
                return Ok(release);
            }
            Err(e) => why.push(format!("{name}: {e}")),
        },
        Err(e) => why.push(format!("no release listed: {e}")),
    }
    for name in kept_releases(&root) {
        match Release::open(&Http, &root.join(&name), &name) {
            Ok(release) => return Ok(release),
            Err(e) => why.push(format!("{name} (kept): {e}")),
        }
    }
    Err(why.join("; "))
}

/// Writes `bytes` whole, then moves them into place: a reader never sees half a file.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp = path.to_path_buf().into_os_string();
    temp.push(format!(".{}.part", std::process::id()));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

/// Cell `key`'s features: kept on disk, else read from the bucket and kept.
fn read_cell(release: &Release, key: [i32; 2]) -> Result<PowerCell, String> {
    let path = cache_dir()
        .join(&release.name)
        .join("cells")
        .join(format!("{}_{}.bin", key[0], key[1]));
    if let Some(cell) = std::fs::read(&path)
        .ok()
        .and_then(|b| PowerCell::decode(&b))
    {
        return Ok(cell);
    }
    let min = [f64::from(key[1]) * CELL_DEG, f64::from(key[0]) * CELL_DEG];
    let max = [min[0] + CELL_DEG, min[1] + CELL_DEG];
    let started = Instant::now();
    let read = release
        .read_box(&Http, min, max)
        .map_err(|e| e.to_string())?;
    {
        let mut state = lock();
        state.status.last_read_groups = read.row_groups;
        state.status.last_read_mb = read.bytes >> 20;
        state.status.last_read_ms = started.elapsed().as_millis() as u64;
    }
    if let Err(e) = write_file(&path, &read.cell.encode()) {
        log_once("write", &format!("not kept: {}: {e}", path.display()));
    }
    Ok(read.cell)
}

fn work() {
    let mut release: Option<(Result<Arc<Release>, String>, Instant)> = None;
    loop {
        let key = {
            let mut state = lock();
            loop {
                if let Some(key) = state.queue.pop_front() {
                    break key;
                }
                state = loader().wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        if release
            .as_ref()
            .is_none_or(|(r, at)| r.is_err() && at.elapsed() >= RETRY_AFTER)
        {
            let opened = open_release().map(Arc::new);
            match &opened {
                Ok(r) => lock().status.release = Some(r.name.clone()),
                Err(e) => log_once("release", &format!("no release: {e}")),
            }
            release = Some((opened, Instant::now()));
        }
        let result = match release.as_ref().map(|(r, _)| r) {
            Some(Ok(r)) => read_cell(r, key),
            Some(Err(e)) => Err(e.clone()),
            None => Err("no release".into()),
        };
        let mut state = lock();
        match result {
            Ok(read) => {
                let cell = Cell::of(read);
                state.status.cells_had += 1;
                state.status.lines += cell.lines.len();
                state.status.masts += cell.masts.len();
                state.cells.insert(key, Slot::Had(Arc::new(cell)));
            }
            Err(e) => {
                log_once(
                    &format!("cell {key:?}"),
                    &format!("cell {key:?} not read: {e}"),
                );
                state.status.cells_failed += 1;
                state.cells.insert(key, Slot::Failed(Instant::now()));
            }
        }
        EPOCH.fetch_add(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_half_degrees_and_a_chunks_bounds_hold_its_corners() {
        assert_eq!(cell_of([-118.27, 33.74]), [67, -237]);
        assert_eq!(cell_of([160.05, -9.43]), [-19, 320]);
        let geo = LatLon {
            lat: 33.74,
            lon: -118.27,
        };
        let (lo, hi) = lon_lat_bounds(geo, [1000.0, -2000.0], [2000.0, -1000.0]);
        for p in [[1000.0, -2000.0], [2000.0, -1000.0], [1500.0, -1500.0]] {
            let q = to_lat_lon(geo, p);
            assert!(q.lon > lo[0] && q.lon < hi[0] && q.lat > lo[1] && q.lat < hi[1]);
        }
        // 1 km is about 0.009° of latitude.
        assert!((hi[1] - lo[1] - 0.009).abs() < 0.001, "{lo:?} {hi:?}");
        assert!(is_release("2026-09-23.1") && !is_release("cells") && !is_release(""));
    }

    #[test]
    fn a_chunk_across_the_date_line_reads_the_cells_either_side_only() {
        let geo = LatLon {
            lat: -17.0,
            lon: 179.99,
        };
        let b = lon_lat_bounds(geo, [-2500.0, -2500.0], [3500.0, 3500.0]);
        assert!(b.1[0] - b.0[0] < 0.1, "{b:?}");
        let mut cells = cells_over(b);
        cells.sort_unstable();
        assert_eq!(cells, [[-35, -360], [-35, 359], [-34, -360], [-34, 359]]);
        assert_eq!(wrap_cell([0, 360]), [0, -360]);
        assert!((near_lon(179.99, -179.98) - 180.02).abs() < 1e-9);
        assert!((near_lon(-118.0, -118.25) + 118.25).abs() < 1e-9);
    }
}
