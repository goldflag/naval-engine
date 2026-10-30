//! OpenStreetMap's features over the theatre (`native/docs/terrain/surface.md`, "Map features on the ground"): vector
//! tiles
//! from OpenFreeMap, decoded by `naval_osm` and projected into a scenario's frame, for whatever draws from them (the
//! roads, railways and waterways on the ground, `theatre::features`; buildings' tags).
//! Presentation only, as the rest of the theatre's land.
//!
//! - **Source.** OpenFreeMap's public instance (https://openfreemap.org): the OpenMapTiles schema as Mapbox Vector
//!   Tiles to zoom 14, no key, no account, no request limit, commercial use allowed; the tile URL comes from its
//!   TileJSON ([`TILEJSON_URL`], kept a week). Its terms ask for the attribution ([`ATTRIBUTION`]) and forbid
//!   collecting its data in bulk: only the tiles about what is drawn are fetched, never a region whole.
//! - **Cache.** Every tile as it came under the user data directory's `osm-cache` ([`cache_dir`]; `NAVAL_OSM_DIR` moves
//!   it), `openfreemap/{z}/{x}/{y}.pbf`, written whole and moved into place, kept for good (an empty file where the
//!   tile is empty, an `.absent` marker where the server has none). Decoded and projected tiles stay in memory
//!   ([`KEPT_BYTES`]).
//! - **Two ways in.** From the frame loop: [`tile`] (memory only, never waits) and [`want`] (the worker thread,
//!   `theatre-osm`, fetches and decodes what a client asks for, nearest first, [`FETCHERS`] requests at a time). From a
//!   thread of one's own (a scene build, another worker): [`load`], which reads the disk and fetches what is missing on
//!   the caller's thread. Both share the cache, the memory and `surface`'s offline back-off: after a fetch finds no
//!   network nothing is tried for a minute, and failures are logged once.
//! - **Switched** with the surface's streaming ([`super::surface::enabled`]); `NAVAL_OSM=off` turns it off alone.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_osm::{
    Aeroway, Building, Features, Geometry, Landcover, Landuse, Poi, TileId, Transportation, Water,
    Waterway,
};

use super::surface::{self, FetchError};

/// OpenFreeMap's TileJSON for its planet: the URL of the current tiles (a dated path that changes weekly).
pub const TILEJSON_URL: &str = "https://tiles.openfreemap.org/planet";
/// What the game must credit wherever it shows something drawn from these tiles.
pub const ATTRIBUTION: &str = naval_osm::ATTRIBUTION;
/// The deepest zoom fetched (the tiles' own: OpenFreeMap answers deeper zooms empty).
pub const ZOOM: u8 = naval_osm::tiles::MAX_ZOOM;
/// Requests in flight at once.
pub const FETCHERS: usize = 6;
/// Decoded tiles kept in memory, bytes (roughly: their points and strings).
pub const KEPT_BYTES: usize = 192 << 20;
/// A tile's body is never more than this (a dense city's zoom-14 tile is about 1 MB).
const MAX_TILE_BYTES: u64 = 16 << 20;
/// The TileJSON is fetched again after this long.
const TILEJSON_FRESH: Duration = Duration::from_secs(7 * 24 * 3600);
/// A tile whose fetch failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
const USER_AGENT: &str = concat!(
    "naval-native/",
    env!("CARGO_PKG_VERSION"),
    " (theatre map features; tile cache on disk)"
);

/// Whether the tiles are fetched and drawn: with the surface's streaming on, unless `NAVAL_OSM` is `off` (or `0`).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_OSM").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

/// Where the tiles are kept: `NAVAL_OSM_DIR` when set, else the user data directory's `osm-cache` (shared by every
/// checkout and the packaged game).
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_OSM_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    crate::host::data_dir().join("osm-cache")
}

fn log_once(kind: &str, line: &str) {
    static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged
        .get_or_insert_with(HashSet::new)
        .insert(kind.to_owned())
    {
        eprintln!("the theatre's map features: {line}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Tiles in the theatre's frame.

/// The tiles at zoom `z` under the theatre's rectangle from `min` to `max` (metres, x east and z south) about `geo`,
/// nearest the rectangle's middle first. The frame's azimuthal projection is not mercator's, so the rectangle's
/// latitude and longitude box is taken from its edges, eight points a side.
pub fn tiles_over(geo: LatLon, min: [f64; 2], max: [f64; 2], z: u8) -> Vec<TileId> {
    let (mut south, mut north, mut west, mut east) = (90.0f64, -90.0f64, f64::MAX, f64::MIN);
    let steps = 8;
    for k in 0..=steps {
        let t = f64::from(k) / f64::from(steps);
        for p in [
            [min[0] + (max[0] - min[0]) * t, min[1]],
            [min[0] + (max[0] - min[0]) * t, max[1]],
            [min[0], min[1] + (max[1] - min[1]) * t],
            [max[0], min[1] + (max[1] - min[1]) * t],
        ] {
            let ll = to_lat_lon(geo, p);
            // Longitudes unwrapped about the origin's, so a frame across the antimeridian stays one box.
            let lon = geo.lon + naval_earth::geo::wrap_lon(ll.lon - geo.lon);
            south = south.min(ll.lat);
            north = north.max(ll.lat);
            west = west.min(lon);
            east = east.max(lon);
        }
    }
    let wrap = |lon: f64| naval_earth::geo::wrap_lon(lon);
    let mut tiles = naval_osm::tiles::covering([south, wrap(west)], [north, wrap(east)], z);
    let middle = to_lat_lon(geo, [(min[0] + max[0]) / 2.0, (min[1] + max[1]) / 2.0]);
    let centre = naval_osm::tiles::fraction(middle.lat, middle.lon);
    let n = f64::from(TileId::side(z));
    let distance = |t: &TileId| {
        let dx = (f64::from(t.x) + 0.5) / n - centre[0];
        let dx = dx - dx.round();
        let dy = (f64::from(t.y) + 0.5) / n - centre[1];
        dx.hypot(dy)
    };
    tiles.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    tiles
}

/// The theatre point of a tile's point `(u, v)` (fractions of the tile) about `geo`.
pub fn project(geo: LatLon, id: TileId, uv: [f64; 2]) -> [f64; 2] {
    let (lat, lon) = id.lat_lon(uv[0], uv[1], 1.0);
    to_local(geo, LatLon { lat, lon })
}

/// A feature's box in the theatre's metres: min x, min z, max x, max z.
pub type Bounds = [f64; 4];

fn bounds_of(g: &Geometry<[f64; 2]>) -> Bounds {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for p in g.points() {
        b = [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ];
    }
    b
}

fn overlaps(b: &Bounds, min: [f64; 2], max: [f64; 2]) -> bool {
    b[0] <= max[0] && b[2] >= min[0] && b[1] <= max[1] && b[3] >= min[1]
}

/// One tile's features in a scenario's frame (theatre metres, x east and z south), each layer's boxes beside it. A
/// tile's features reach a little past it (the tile's buffer, about 30 m at zoom 14) and a polygon is cut at that
/// buffer's edge: a feature crossing tiles comes in each, and a union of the tiles' polygons is the whole.
#[derive(Debug)]
pub struct OsmTile {
    pub id: TileId,
    pub features: Features<[f64; 2]>,
    /// The tile proper (without its buffer) in the frame: its four corners' box.
    pub bounds: Bounds,
    boxes: [Vec<Bounds>; 8],
    /// Roughly its memory, bytes.
    pub bytes: usize,
}

impl OsmTile {
    /// The tile `id` decoded from `bytes` (empty: a tile with nothing in it) and projected about `geo`.
    pub fn decode(geo: LatLon, id: TileId, bytes: &[u8]) -> Result<Self, naval_osm::DecodeError> {
        let features = if bytes.is_empty() {
            Features::default()
        } else {
            naval_osm::features(bytes)?
        };
        Ok(Self::new(geo, id, &features))
    }

    pub fn new(geo: LatLon, id: TileId, features: &Features<naval_osm::TilePoint>) -> Self {
        let features = features.map(&mut |p| project(geo, id, *p));
        let boxes = [
            features
                .transportation
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .waterway
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .water
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .landuse
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .landcover
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .building
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .poi
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
            features
                .aeroway
                .iter()
                .map(|f| bounds_of(&f.geometry))
                .collect(),
        ];
        let corners = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]].map(|c| project(geo, id, c));
        let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for p in corners {
            bounds = [
                bounds[0].min(p[0]),
                bounds[1].min(p[1]),
                bounds[2].max(p[0]),
                bounds[3].max(p[1]),
            ];
        }
        let points = features.point_count();
        let features_count: usize = boxes.iter().map(Vec::len).sum();
        Self {
            id,
            features,
            bounds,
            boxes,
            bytes: points * 16 + features_count * 160 + 256,
        }
    }

    /// Roads, railways and the rest of `transportation` whose box meets the rectangle from `min` to `max`.
    pub fn transportation_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Transportation<[f64; 2]>> {
        within(&self.features.transportation, &self.boxes[0], min, max)
    }

    pub fn waterway_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Waterway<[f64; 2]>> {
        within(&self.features.waterway, &self.boxes[1], min, max)
    }

    pub fn water_in(&self, min: [f64; 2], max: [f64; 2]) -> impl Iterator<Item = &Water<[f64; 2]>> {
        within(&self.features.water, &self.boxes[2], min, max)
    }

    pub fn landuse_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Landuse<[f64; 2]>> {
        within(&self.features.landuse, &self.boxes[3], min, max)
    }

    pub fn landcover_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Landcover<[f64; 2]>> {
        within(&self.features.landcover, &self.boxes[4], min, max)
    }

    pub fn building_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Building<[f64; 2]>> {
        within(&self.features.building, &self.boxes[5], min, max)
    }

    /// Points of interest within the rectangle.
    pub fn poi_in(&self, min: [f64; 2], max: [f64; 2]) -> impl Iterator<Item = &Poi<[f64; 2]>> {
        within(&self.features.poi, &self.boxes[6], min, max)
    }

    /// An aerodrome's runways, taxiways, aprons and the rest within the rectangle.
    pub fn aeroway_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Aeroway<[f64; 2]>> {
        within(&self.features.aeroway, &self.boxes[7], min, max)
    }

    /// The tile proper's four corners in the frame (north-west, north-east, south-east, south-west: counter-clockwise
    /// in (x, z)): neighbouring tiles share theirs, so features cut to them meet without overlapping.
    pub fn quad(&self, geo: LatLon) -> [[f64; 2]; 4] {
        [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].map(|c| project(geo, self.id, c))
    }
}

fn within<'a, T>(
    items: &'a [T],
    boxes: &'a [Bounds],
    min: [f64; 2],
    max: [f64; 2],
) -> impl Iterator<Item = &'a T> {
    items
        .iter()
        .zip(boxes)
        .filter(move |(_, b)| overlaps(b, min, max))
        .map(|(t, _)| t)
}

/// Several tiles' features over an area, as [`load`] or [`area`] gives them.
#[derive(Clone, Debug, Default)]
pub struct Area {
    pub tiles: Vec<Arc<OsmTile>>,
    /// Tiles not yet in memory (asked of the worker by [`area`]; failed for [`load`]).
    pub missing: Vec<TileId>,
}

impl Area {
    pub fn transportation_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Transportation<[f64; 2]>> {
        self.tiles
            .iter()
            .flat_map(move |t| t.transportation_in(min, max))
    }

    pub fn waterway_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Waterway<[f64; 2]>> {
        self.tiles.iter().flat_map(move |t| t.waterway_in(min, max))
    }

    pub fn water_in(&self, min: [f64; 2], max: [f64; 2]) -> impl Iterator<Item = &Water<[f64; 2]>> {
        self.tiles.iter().flat_map(move |t| t.water_in(min, max))
    }

    pub fn landuse_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Landuse<[f64; 2]>> {
        self.tiles.iter().flat_map(move |t| t.landuse_in(min, max))
    }

    pub fn landcover_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Landcover<[f64; 2]>> {
        self.tiles
            .iter()
            .flat_map(move |t| t.landcover_in(min, max))
    }

    pub fn building_in(
        &self,
        min: [f64; 2],
        max: [f64; 2],
    ) -> impl Iterator<Item = &Building<[f64; 2]>> {
        self.tiles.iter().flat_map(move |t| t.building_in(min, max))
    }

    pub fn poi_in(&self, min: [f64; 2], max: [f64; 2]) -> impl Iterator<Item = &Poi<[f64; 2]>> {
        self.tiles.iter().flat_map(move |t| t.poi_in(min, max))
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The disk cache and the network.

/// The tile cache on disk.
pub struct Cache {
    pub dir: PathBuf,
}

impl Cache {
    pub fn tile_path(&self, id: TileId) -> PathBuf {
        self.dir
            .join("openfreemap")
            .join(id.z.to_string())
            .join(id.x.to_string())
            .join(format!("{}.pbf", id.y))
    }

    fn absent_path(&self, id: TileId) -> PathBuf {
        let mut p = self.tile_path(id).into_os_string();
        p.push(".absent");
        p.into()
    }

    fn tilejson_path(&self) -> PathBuf {
        self.dir.join("openfreemap").join("tilejson.json")
    }

    /// The tile's bytes on disk: `Some(Some(bytes))` (empty for an empty tile), `Some(None)` where the server has
    /// none, `None` when not fetched.
    pub fn read(&self, id: TileId) -> Option<Option<Vec<u8>>> {
        match std::fs::read(self.tile_path(id)) {
            Ok(bytes) => Some(Some(bytes)),
            Err(_) if self.absent_path(id).exists() => Some(None),
            Err(_) => None,
        }
    }

    /// Whether the tile is on disk (or known absent), without reading it.
    pub fn holds(&self, id: TileId) -> bool {
        self.tile_path(id).exists() || self.absent_path(id).exists()
    }

    /// Keeps the tile's bytes (none: the server has no such tile), written whole and moved into place.
    pub fn write(&self, id: TileId, bytes: Option<&[u8]>) -> std::io::Result<()> {
        let path = match bytes {
            Some(_) => self.tile_path(id),
            None => self.absent_path(id),
        };
        write_file(&path, bytes.unwrap_or_default())
    }
}

/// Writes `bytes` whole, then moves them into place: a reader never sees half a file.
fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let mut temp = path.to_path_buf().into_os_string();
    temp.push(format!(
        ".{}-{}.part",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .max_idle_connections_per_host(FETCHERS)
            .build()
            .into()
    })
}

/// A whole body; none where the server has none (404, 403, 204).
fn get(url: &str) -> Result<Option<Vec<u8>>, FetchError> {
    let mut response = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => FetchError::Offline(format!("{url}: {e}")),
        other => FetchError::Server(format!("{url}: {other}")),
    })?;
    match response.status().as_u16() {
        200 => {
            let mut bytes = Vec::new();
            response
                .body_mut()
                .with_config()
                .limit(MAX_TILE_BYTES)
                .reader()
                .read_to_end(&mut bytes)
                .map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
            Ok(Some(bytes))
        }
        204 => Ok(Some(Vec::new())),
        403 | 404 => Ok(None),
        status => Err(FetchError::Server(format!("{url}: HTTP {status}"))),
    }
}

/// The tile URL template from a TileJSON document (its first `tiles` entry, with `{z}`, `{x}` and `{y}`).
pub fn template_of(tilejson: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(tilejson).ok()?;
    let t = v.get("tiles")?.get(0)?.as_str()?;
    (t.contains("{z}") && t.contains("{x}") && t.contains("{y}") && t.starts_with("https://"))
        .then(|| t.to_owned())
}

/// The template filled in for `id`.
pub fn tile_url(template: &str, id: TileId) -> String {
    template
        .replace("{z}", &id.z.to_string())
        .replace("{x}", &id.x.to_string())
        .replace("{y}", &id.y.to_string())
}

/// The tile URL template: in memory, else the TileJSON on disk while fresh (or whenever the network is out), else
/// fetched (and kept). `refresh` fetches it again whatever its age (a tile the old template named was missing).
fn template(cache: &Cache, refresh: bool) -> Result<String, FetchError> {
    static TEMPLATE: Mutex<Option<String>> = Mutex::new(None);
    if !refresh && let Some(t) = TEMPLATE.lock().unwrap_or_else(|e| e.into_inner()).clone() {
        return Ok(t);
    }
    let path = cache.tilejson_path();
    let on_disk = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| template_of(&text));
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < TILEJSON_FRESH);
    let keep = |t: &str| *TEMPLATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(t.to_owned());
    if let Some(t) = &on_disk
        && (fresh || surface::offline())
        && !refresh
    {
        keep(t);
        return Ok(t.clone());
    }
    if surface::offline() {
        return Err(FetchError::Offline("tried lately".into()));
    }
    match get(TILEJSON_URL) {
        Ok(Some(bytes)) => {
            let text = String::from_utf8_lossy(&bytes);
            let t = template_of(&text)
                .ok_or_else(|| FetchError::Server(format!("{TILEJSON_URL}: no tile URL in it")))?;
            if let Err(e) = write_file(&path, text.as_bytes()) {
                log_once("write", &format!("keeping the TileJSON: {e}"));
            }
            keep(&t);
            Ok(t)
        }
        // The server is out but an old template is on disk: it still names tiles OpenFreeMap serves for a while.
        Ok(None) | Err(FetchError::Server(_)) if on_disk.is_some() => {
            let t = on_disk.expect("on disk");
            keep(&t);
            Ok(t)
        }
        Ok(None) => Err(FetchError::Server(format!("{TILEJSON_URL}: none"))),
        Err(e) => Err(e),
    }
}

/// Fetches tile `id` into the cache.
fn fetch(cache: &Cache, id: TileId) -> Result<(), FetchError> {
    let mut url = tile_url(&template(cache, false)?, id);
    let mut body = get(&url)?;
    if body.is_none() {
        // The template may have aged out (OpenFreeMap's dated paths): once more with a fresh one.
        url = tile_url(&template(cache, true)?, id);
        body = get(&url)?;
    }
    cache
        .write(id, body.as_deref())
        .map_err(|e| FetchError::Server(format!("the OSM cache: {e}")))
}

// ---------------------------------------------------------------------------------------------------------------------
// Memory, the worker and the two ways in.

type Key = ([u64; 2], TileId);

fn key(geo: LatLon, id: TileId) -> Key {
    ([geo.lat.to_bits(), geo.lon.to_bits()], id)
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerStatus {
    /// Tiles asked for and not begun, over every client.
    pub queued: usize,
    /// Tiles of the batch in hand.
    pub working: usize,
    /// Tiles in memory, and their bytes.
    pub kept: usize,
    pub kept_bytes: usize,
    /// Tiles fetched from the network this session.
    pub fetched: usize,
    /// Tiles that failed in the last minute.
    pub failed: usize,
}

struct State {
    geo: Option<LatLon>,
    /// Each client's tiles, nearest first, in place of what it asked before.
    wanted: Vec<(&'static str, VecDeque<TileId>)>,
    turn: usize,
    kept: HashMap<Key, Arc<OsmTile>>,
    order: VecDeque<Key>,
    kept_bytes: usize,
    failed: HashMap<Key, Instant>,
    working: usize,
    fetched: usize,
    worker: bool,
}

struct Store {
    cache: Cache,
    state: Mutex<State>,
    wake: Condvar,
}

fn store() -> &'static Store {
    static STORE: OnceLock<Store> = OnceLock::new();
    STORE.get_or_init(|| Store {
        cache: Cache { dir: cache_dir() },
        state: Mutex::new(State {
            geo: None,
            wanted: Vec::new(),
            turn: 0,
            kept: HashMap::new(),
            order: VecDeque::new(),
            kept_bytes: 0,
            failed: HashMap::new(),
            working: 0,
            fetched: 0,
            worker: false,
        }),
        wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    store().state.lock().unwrap_or_else(|e| e.into_inner())
}

impl State {
    fn keep(&mut self, k: Key, tile: Arc<OsmTile>) {
        self.kept_bytes += tile.bytes;
        if let Some(old) = self.kept.insert(k, tile) {
            self.kept_bytes -= old.bytes;
        } else {
            self.order.push_back(k);
        }
        while self.kept_bytes > KEPT_BYTES && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front()
                && let Some(t) = self.kept.remove(&old)
            {
                self.kept_bytes -= t.bytes;
            }
        }
    }
}

/// Tile `id` about `geo` if it is in memory now. Never waits: callable from the frame loop.
pub fn tile(geo: LatLon, id: TileId) -> Option<Arc<OsmTile>> {
    lock().kept.get(&key(geo, id)).cloned()
}

/// The tiles under the theatre's rectangle from `min` to `max` about `geo` at zoom `z` that are in memory now, and
/// the rest asked of the worker for `client` (in place of what `client` asked before). Never waits.
pub fn area(client: &'static str, geo: LatLon, min: [f64; 2], max: [f64; 2], z: u8) -> Area {
    let ids = tiles_over(geo, min, max, z);
    let mut out = Area::default();
    {
        let state = lock();
        for &id in &ids {
            match state.kept.get(&key(geo, id)) {
                Some(t) => out.tiles.push(t.clone()),
                None => out.missing.push(id),
            }
        }
    }
    want(client, geo, &out.missing);
    out
}

/// Asks the worker for `ids` about `geo` (nearest first) for `client`, in place of whatever `client` asked before; a
/// tile in memory, or failed in the last minute, is left out. A client is any short name (`"features"`,
/// `"buildings"`): the worker takes the clients' tiles in turn. Nothing with the tiles off.
pub fn want(client: &'static str, geo: LatLon, ids: &[TileId]) {
    if !enabled() {
        return;
    }
    let s = store();
    let mut state = lock();
    if state.geo != Some(geo) {
        // Another scenario: what was asked for the last one is moot.
        state.wanted.clear();
        state.geo = Some(geo);
    }
    let now = Instant::now();
    state
        .failed
        .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    let list: VecDeque<TileId> = ids
        .iter()
        .copied()
        .filter(|&id| {
            let k = key(geo, id);
            !state.kept.contains_key(&k) && !state.failed.contains_key(&k)
        })
        .collect();
    match state.wanted.iter_mut().find(|(c, _)| *c == client) {
        Some((_, l)) => *l = list,
        None => state.wanted.push((client, list)),
    }
    if state.wanted.iter().all(|(_, l)| l.is_empty()) {
        return;
    }
    if !state.worker {
        state.worker = true;
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-osm", work);
        if let Err(e) = spawned {
            state.worker = false;
            log_once("thread", &format!("no worker thread: {e}"));
        }
    }
    s.wake.notify_one();
}

/// What the worker and the memory hold.
pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        queued: state.wanted.iter().map(|(_, l)| l.len()).sum(),
        working: state.working,
        kept: state.kept.len(),
        kept_bytes: state.kept_bytes,
        fetched: state.fetched,
        failed: state.failed.len(),
    }
}

fn work() {
    loop {
        let (geo, batch) = {
            let mut state = lock();
            loop {
                if let Some(geo) = state.geo {
                    // The clients in turn, up to FETCHERS tiles a batch.
                    let mut batch = Vec::with_capacity(FETCHERS);
                    let clients = state.wanted.len();
                    let mut idle = 0;
                    while batch.len() < FETCHERS && clients > 0 && idle < clients {
                        let turn = state.turn % clients;
                        state.turn = state.turn.wrapping_add(1);
                        match state.wanted[turn].1.pop_front() {
                            Some(id) => {
                                idle = 0;
                                if !state.kept.contains_key(&key(geo, id)) && !batch.contains(&id) {
                                    batch.push(id);
                                }
                            }
                            None => idle += 1,
                        }
                    }
                    if !batch.is_empty() {
                        state.working = batch.len();
                        break (geo, batch);
                    }
                }
                state.working = 0;
                state = store().wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let result = load(geo, &batch);
        let mut state = lock();
        state.working = 0;
        if let Err(e) = result {
            let now = Instant::now();
            for &id in &batch {
                if !state.kept.contains_key(&key(geo, id)) {
                    state.failed.insert(key(geo, id), now);
                }
            }
            if matches!(e, FetchError::Offline(_)) {
                // Everything else asked would fail the same way: asked again by the next frame that wants it.
                for (_, l) in &mut state.wanted {
                    l.clear();
                }
            }
        }
    }
}

/// Tiles `ids` about `geo`: from memory, else the disk, else fetched ([`FETCHERS`] at a time) on the caller's thread,
/// decoded and projected (and kept). For a caller off the frame loop (a scene build, a worker). With the tiles off,
/// or offline, only what memory and the disk hold; the first failure is returned once every tile that could be read
/// is kept (`Err` then means some tiles are missing: [`tile`] has the rest).
pub fn load(geo: LatLon, ids: &[TileId]) -> Result<Area, FetchError> {
    let s = store();
    let mut out = Area::default();
    // Memory under the lock; the disk after it (the frame loop's `tile` and `want` take the same lock).
    let mut absent_from_memory = Vec::new();
    {
        let state = lock();
        for &id in ids {
            match state.kept.get(&key(geo, id)) {
                Some(t) => out.tiles.push(t.clone()),
                None => absent_from_memory.push(id),
            }
        }
    }
    let (on_disk, mut fetch_list): (Vec<TileId>, Vec<TileId>) = absent_from_memory
        .into_iter()
        .partition(|&id| s.cache.holds(id));
    let mut failure = None;
    if !fetch_list.is_empty() {
        if !enabled() {
            out.missing.extend(&fetch_list);
            fetch_list.clear();
        } else if surface::offline() {
            out.missing.extend(&fetch_list);
            fetch_list.clear();
            failure = Some(FetchError::Offline("tried lately".into()));
        }
    }
    let fetched = fetch_all(&s.cache, &fetch_list);
    match fetched {
        Ok(n) => lock().fetched += n,
        Err((n, e)) => {
            lock().fetched += n;
            if matches!(e, FetchError::Offline(_)) {
                surface::mark_offline();
            }
            log_once(
                match e {
                    FetchError::Offline(_) => "offline",
                    FetchError::Server(_) => "server",
                },
                &format!("{e}; the land is drawn without them"),
            );
            failure = Some(e);
        }
    }
    // Decode what is now on disk, the tiles in parallel (a city's tile decodes and projects in a few milliseconds).
    let ready: Vec<TileId> = on_disk
        .into_iter()
        .chain(fetch_list.into_iter().filter(|&id| s.cache.holds(id)))
        .collect();
    let decoded = decode_all(&s.cache, geo, &ready);
    let mut state = lock();
    for (id, tile) in ready.iter().zip(decoded) {
        match tile {
            Some(t) => {
                let t = Arc::new(t);
                state.keep(key(geo, *id), t.clone());
                out.tiles.push(t);
            }
            None => out.missing.push(*id),
        }
    }
    for &id in ids {
        if !ready.contains(&id)
            && !out.tiles.iter().any(|t| t.id == id)
            && !out.missing.contains(&id)
        {
            out.missing.push(id);
        }
    }
    match failure {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

/// Tile `id`'s bytes in the cache on disk: `Some(Some(bytes))` (empty for an empty tile), `Some(None)` where the
/// server has none, `None` when not fetched (for a reader of its own, the far level's towers: `buildings::towers`).
pub fn cached(id: TileId) -> Option<Option<Vec<u8>>> {
    store().cache.read(id)
}

/// Throws away tile `id`'s bytes in the cache on disk (bytes that will not decode, for a reader of its own): fetched
/// again when next asked for, as the map's own reader does with one ([`decode_all`]).
pub fn discard(id: TileId) {
    let _ = std::fs::remove_file(store().cache.tile_path(id));
}

/// Fetches into the cache those of `ids` it does not hold, [`FETCHERS`] at a time, on the caller's thread (decoding
/// nothing): how many were fetched. With the tiles off, none; offline (lately), the failure.
pub fn fetch_missing(ids: &[TileId]) -> Result<usize, FetchError> {
    let s = store();
    let list: Vec<TileId> = ids
        .iter()
        .copied()
        .filter(|&id| !s.cache.holds(id))
        .collect();
    if list.is_empty() || !enabled() {
        return Ok(0);
    }
    if surface::offline() {
        return Err(FetchError::Offline("tried lately".into()));
    }
    match fetch_all(&s.cache, &list) {
        Ok(n) => {
            lock().fetched += n;
            Ok(n)
        }
        Err((n, e)) => {
            lock().fetched += n;
            if matches!(e, FetchError::Offline(_)) {
                surface::mark_offline();
            }
            log_once(
                match e {
                    FetchError::Offline(_) => "offline",
                    FetchError::Server(_) => "server",
                },
                &format!("{e}; the land is drawn without them"),
            );
            Err(e)
        }
    }
}

/// Fetches every tile of `list` into the cache, [`FETCHERS`] at a time; the first failure ends it (what arrived stays
/// cached). How many were fetched, with the failure.
fn fetch_all(cache: &Cache, list: &[TileId]) -> Result<usize, (usize, FetchError)> {
    if list.is_empty() {
        return Ok(0);
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<FetchError>> = Mutex::new(None);
    // At the worker's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..FETCHERS.min(list.len()) {
            scope.spawn(|| {
                qos.apply();
                while !stop.load(Ordering::Relaxed) {
                    let Some(&id) = list.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    match fetch(cache, id) {
                        Ok(()) => {
                            done.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            stop.store(true, Ordering::Relaxed);
                            failure
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .get_or_insert(e);
                        }
                    }
                }
            });
        }
    });
    let n = done.into_inner();
    match failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
        Some(e) => Err((n, e)),
        None => Ok(n),
    }
}

/// Decodes and projects tiles from the cache, a few threads at a time; none for a tile the server has none of is an
/// empty tile, a tile that will not decode is deleted (fetched again next time) and comes back none.
fn decode_all(cache: &Cache, geo: LatLon, ids: &[TileId]) -> Vec<Option<OsmTile>> {
    let one = |id: TileId| -> Option<OsmTile> {
        let bytes = cache.read(id)?.unwrap_or_default();
        match OsmTile::decode(geo, id, &bytes) {
            Ok(t) => Some(t),
            Err(e) => {
                log_once("decode", &format!("tile {}/{}/{}: {e}", id.z, id.x, id.y));
                let _ = std::fs::remove_file(cache.tile_path(id));
                None
            }
        }
    };
    if ids.len() <= 1 {
        return ids.iter().map(|&id| one(id)).collect();
    }
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 4)
        .min(ids.len());
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<OsmTile>>> = Mutex::new((0..ids.len()).map(|_| None).collect());
    // At the worker's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                qos.apply();
                loop {
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&id) = ids.get(k) else {
                        break;
                    };
                    let t = one(id);
                    results.lock().unwrap_or_else(|e| e.into_inner())[k] = t;
                }
            });
        }
    });
    results.into_inner().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
#[path = "osm_tests.rs"]
mod tests;
