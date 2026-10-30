//! The far level's mapped towers (native/docs/terrain/buildings.md, "The far level"): OpenStreetMap's buildings of
//! [`far::TOWER_M`] and more, from the map's zoom-14 tiles, the near levels' own ([`osm`]'s cache on disk), so a far
//! chunk stands the same towers as its boxes did.
//!
//! - **Read for the towers alone.** A tile's `building` and `landuse` layers are decoded (not its roads, water or
//!   points), and only its towers are kept ([`reduce`]): a building with a measured height of [`far::TOWER_M`] or
//!   more, the outlines and parts holding its middle (a podium under a shaft), the parts within an outline drawn by its
//!   parts (`hide_3d`), and the land use under them (an office on commercial land, as the boxes judge it): 0.3-0.9 ms
//!   a tile against the map's whole decode's 1.3-2.8. Kept in memory as tiles of their own ([`KEPT_BYTES`]; Taipei's
//!   347 tiles within 30 km 1.6 MB), apart from the map's 192 MB, so the far ring's 400-900 tiles do not push the near
//!   levels' out.
//! - **Fetched behind.** A tile not on disk is fetched by a thread of its own (`theatre-towers`, [`osm::FETCHERS`] at a
//!   time, nearest first as asked), never by the far chunks' worker, whose masses stand at once; [`epoch`] counts the
//!   batches landed, and a far chunk made without its tiles is made again once it moves on. The tiles are the map's
//!   about what is drawn (the far chunks on land within 30 km), never a region whole, as its terms ask: a scene's
//!   requests are dropped once another asks, and any not asked again for [`FORGET_AFTER`] (its chunk no longer drawn),
//!   [`MOST_QUEUED`] at most. A tile whose fetch failed, or that will not decode (thrown away, as the map's own reader
//!   does), is not asked again for [`RETRY_AFTER`], and its far chunks are made again once it is had.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_earth::geo::{LatLon, to_local};
use naval_osm::{Building, Features, Geometry, Polygon, TileId, TilePoint, mvt};
use naval_terrain::buildings::far;
use naval_terrain::buildings::thinning::{self, CELLS, LATER, TALL, Thinning};
use naval_terrain::buildings::{CHUNK_M, Period, Region, Usage};

use super::super::osm::{self, OsmTile};

/// The towers' tiles kept in memory, bytes (roughly: `OsmTile::bytes`); the oldest go first.
pub const KEPT_BYTES: usize = 16 << 20;
/// A tile fetched in one go of the thread, at most.
const FETCH_BATCH: usize = 24;
/// A tile whose fetch failed, or that would not decode, is not asked for again for this long.
pub const RETRY_AFTER: Duration = Duration::from_secs(60);
/// A tile asked for and not asked again for this long is no longer wanted (a far chunk drawn without its tiles is made
/// again, and asks again, at least once a minute): its request is dropped.
pub const FORGET_AFTER: Duration = Duration::from_secs(120);
/// Tiles waiting to be fetched, at most (a far ring wants 400-900): past it the first asked go first, asked again when
/// their chunks are made again.
pub const MOST_QUEUED: usize = 2048;
/// OpenMapTiles' default height for a building with no measurement (m): never a tower.
const DEFAULT_HEIGHT_M: f64 = 5.0;

/// A scenario's frame (its origin's bits): the towers are projected about it, and its requests are its own.
type Scene = [u64; 2];

type Key = (Scene, TileId);

#[derive(Default)]
struct State {
    kept: HashMap<Key, Arc<OsmTile>>,
    order: VecDeque<Key>,
    bytes: usize,
    asked: Asked,
    fetcher: bool,
    /// Tiles read from the disk this session, and fetched.
    read: usize,
    fetched: usize,
}

impl State {
    /// Keeps the tiles read from the disk for `scene`, the oldest going past [`KEPT_BYTES`]: those had, and those that
    /// would not decode (none: thrown away, failed at `now`, never kept as an empty tile).
    fn keep(
        &mut self,
        scene: Scene,
        decoded: Vec<(TileId, Option<OsmTile>)>,
        now: Instant,
    ) -> (Vec<Arc<OsmTile>>, Vec<TileId>) {
        let mut had = Vec::with_capacity(decoded.len());
        let mut bad = Vec::new();
        for (id, tile) in decoded {
            let Some(t) = tile else {
                self.asked.failed.insert(id, now);
                bad.push(id);
                continue;
            };
            self.read += 1;
            self.asked.failed.remove(&id);
            let k = (scene, id);
            let t = Arc::new(t);
            if let Some(old) = self.kept.insert(k, t.clone()) {
                self.bytes -= old.bytes;
            } else {
                self.order.push_back(k);
            }
            self.bytes += t.bytes;
            had.push(t);
        }
        while self.bytes > KEPT_BYTES
            && let Some(old) = self.order.pop_front()
        {
            if let Some(t) = self.kept.remove(&old) {
                self.bytes -= t.bytes;
            }
        }
        (had, bad)
    }
}

/// When a scene asked for a tile, last.
#[derive(Clone, Copy, Debug)]
struct Ask {
    scene: Scene,
    at: Instant,
}

/// The tiles asked of the fetching thread: those to fetch in the order first asked (nearest first), each with the scene
/// that asked and when it last did; those in its hands; and those that failed lately. Bounded: another scene's and
/// those not asked again lately dropped ([`FORGET_AFTER`]), [`MOST_QUEUED`] at most, a failure kept [`RETRY_AFTER`].
#[derive(Default)]
struct Asked {
    queue: VecDeque<TileId>,
    queued: HashMap<TileId, Ask>,
    in_hand: HashSet<TileId>,
    failed: HashMap<TileId, Instant>,
}

impl Asked {
    /// Asks for `ids` for `scene` at `now`, after dropping what is no longer wanted ([`Asked::forget`]): those failed
    /// lately or in hand left out, those queued asked anew in their place. Whether any was queued anew.
    fn ask(&mut self, scene: Scene, ids: impl IntoIterator<Item = TileId>, now: Instant) -> bool {
        self.forget(Some(scene), now);
        let mut any = false;
        for id in ids {
            let failed = self
                .failed
                .get(&id)
                .is_some_and(|at| now.duration_since(*at) < RETRY_AFTER);
            if failed || self.in_hand.contains(&id) {
                continue;
            }
            match self.queued.entry(id) {
                Entry::Occupied(mut e) => e.get_mut().at = now,
                Entry::Vacant(e) => {
                    e.insert(Ask { scene, at: now });
                    self.queue.push_back(id);
                    any = true;
                }
            }
        }
        while self.queue.len() > MOST_QUEUED
            && let Some(old) = self.queue.pop_front()
        {
            self.queued.remove(&old);
        }
        any
    }

    /// Drops the requests no longer wanted at `now`: another scene's than `scene` (none: any scene's kept), and those not
    /// asked again for [`FORGET_AFTER`]; and the failures older than [`RETRY_AFTER`], which hold nothing back.
    fn forget(&mut self, scene: Option<Scene>, now: Instant) {
        let queued = &mut self.queued;
        self.queue.retain(|id| {
            let wanted = queued.get(id).is_some_and(|a| {
                scene.is_none_or(|s| a.scene == s) && now.duration_since(a.at) < FORGET_AFTER
            });
            if !wanted {
                queued.remove(id);
            }
            wanted
        });
        self.failed
            .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    }

    /// Up to `n` of the tiles queued into the thread's hands, the first asked first, those no longer wanted dropped.
    fn take(&mut self, n: usize, now: Instant) -> Vec<TileId> {
        self.forget(None, now);
        let n = self.queue.len().min(n);
        let batch: Vec<TileId> = self.queue.drain(..n).collect();
        for id in &batch {
            self.queued.remove(id);
            self.in_hand.insert(*id);
        }
        batch
    }

    /// The thread's batch `ids` done at `now`: fetched (`ok`, no longer failed) or failed.
    fn done(&mut self, ids: &[TileId], ok: bool, now: Instant) {
        for id in ids {
            self.in_hand.remove(id);
            if ok {
                self.failed.remove(id);
            } else {
                self.failed.insert(*id, now);
            }
        }
    }

    /// Tiles to fetch and in hand.
    fn len(&self) -> usize {
        self.queue.len() + self.in_hand.len()
    }
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

/// Batches of tiles fetched since the start: a far chunk made without some of its tiles is made again once it moves.
static EPOCH: AtomicU64 = AtomicU64::new(0);

fn shared() -> &'static Shared {
    static SHARED: OnceLock<Shared> = OnceLock::new();
    SHARED.get_or_init(|| Shared {
        state: Mutex::new(State::default()),
        wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    shared().state.lock().unwrap_or_else(|e| e.into_inner())
}

/// Batches of the towers' tiles fetched since the start.
pub fn epoch() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

/// A batch of the towers' tiles landed (or failed): the far chunks made without some of them are made again.
pub(super) fn landed() {
    EPOCH.fetch_add(1, Ordering::Release);
}

/// What the towers' reader holds and does, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// Tiles to fetch and in hand.
    pub fetching: usize,
    /// Tiles held, their bytes, and their towers' outlines.
    pub tiles: usize,
    pub bytes: usize,
    pub outlines: usize,
    /// Tiles read from the disk, and fetched, this session.
    pub read: usize,
    pub fetched: usize,
}

pub fn status() -> Status {
    let state = lock();
    Status {
        fetching: state.asked.len(),
        tiles: state.kept.len(),
        bytes: state.bytes,
        outlines: state.kept.values().map(|t| t.features.building.len()).sum(),
        read: state.read,
        fetched: state.fetched,
    }
}

/// Tiles `ids`' towers about `geo`: from memory, else the disk (decoded on the caller's thread, a few at a time, and
/// kept); a tile not on disk, or whose bytes will not decode (thrown away), is asked of the fetching thread and left out
/// meanwhile. With whether every tile was had.
pub fn load(geo: LatLon, ids: &[TileId]) -> (Vec<Arc<OsmTile>>, bool) {
    let scene: Scene = [geo.lat.to_bits(), geo.lon.to_bits()];
    let key = |id: TileId| (scene, id);
    let mut out = Vec::with_capacity(ids.len());
    let mut absent = Vec::new();
    {
        let state = lock();
        for &id in ids {
            match state.kept.get(&key(id)) {
                Some(t) => out.push(t.clone()),
                None => absent.push(id),
            }
        }
    }
    if absent.is_empty() {
        return (out, true);
    }
    let mut on_disk = Vec::new();
    let mut missing = Vec::new();
    for id in absent {
        match osm::cached(id) {
            Some(bytes) => on_disk.push((id, bytes.unwrap_or_default())),
            None => missing.push(id),
        }
    }
    let decoded = decode_all(geo, &on_disk);
    for (id, tile) in &decoded {
        if tile.is_none() {
            // As the map's own reader does: thrown away, fetched again when next asked for.
            osm::discard(*id);
        }
    }
    let now = Instant::now();
    let mut state = lock();
    let (had, bad) = state.keep(scene, decoded, now);
    out.extend(had);
    let all = missing.is_empty() && bad.is_empty();
    if !missing.is_empty() && osm::enabled() && state.asked.ask(scene, missing, now) {
        if !state.fetcher {
            state.fetcher = true;
            // At the asking worker's class (`host::Inherit`).
            let qos = crate::host::Inherit::here();
            if let Err(e) = std::thread::Builder::new()
                .name("theatre-towers".into())
                .spawn(move || {
                    qos.apply();
                    fetch_work();
                })
            {
                state.fetcher = false;
                eprintln!("the far level's towers: no thread to fetch them ({e})");
            }
        }
        shared().wake.notify_one();
    }
    (out, all)
}

/// The tiles read from the disk (empty: nothing in it, or none on the server) decoded for their towers, up to four at
/// a time (none: it will not decode).
fn decode_all(geo: LatLon, tiles: &[(TileId, Vec<u8>)]) -> Vec<(TileId, Option<OsmTile>)> {
    let one = |(id, bytes): &(TileId, Vec<u8>)| (*id, decode(geo, *id, bytes));
    if tiles.len() <= 2 {
        return tiles.iter().map(one).collect();
    }
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 4)
        .min(tiles.len());
    let per = tiles.len().div_ceil(threads);
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        let handles: Vec<_> = tiles
            .chunks(per)
            .map(|part| {
                scope.spawn(move || {
                    qos.apply();
                    part.iter().map(one).collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// Tile `id`'s towers from its bytes (empty: nothing in it), projected about `geo`; none when they will not decode
/// (not a tile with no towers: [`load`] throws it away and asks for it again).
pub fn decode(geo: LatLon, id: TileId, bytes: &[u8]) -> Option<OsmTile> {
    let features = if bytes.is_empty() {
        Features::default()
    } else {
        let tile = mvt::decode_layers(bytes, Some(&["building", "landuse"])).ok()?;
        reduce(&Features::from_tile(&tile))
    };
    Some(OsmTile::new(geo, id, &features))
}

/// A building's measured height (m): OpenMapTiles' `render_height` but its default.
fn measured(b: &Building<TilePoint>) -> Option<f64> {
    b.render_height
        .filter(|h| *h > 0.0 && (*h - DEFAULT_HEIGHT_M).abs() > 1e-3)
}

/// A tile's features cut to its towers: each building (each of a feature's polygons, the tiles merging a few
/// buildings into one) measured [`far::TOWER_M`] or more, those of its outlines and parts that hold its middle, the
/// parts within an outline drawn by its parts that is kept, and the land use holding any of them (an office's
/// commercial land). Nothing when it has no tower.
pub fn reduce(features: &Features<TilePoint>) -> Features<TilePoint> {
    let tall = f64::from(far::TOWER_M);
    // Every building's polygons, their boxes and middles, and whether the building is a tower.
    struct Shape<'a> {
        building: usize,
        polygon: &'a Polygon<TilePoint>,
        bounds: [f64; 4],
        middle: [f64; 2],
    }
    let shapes: Vec<Shape> = features
        .building
        .iter()
        .enumerate()
        .flat_map(|(k, b)| match &b.geometry {
            Geometry::Polygons(ps) => ps
                .iter()
                .filter(|p| p.exterior.len() >= 3)
                .map(|p| Shape {
                    building: k,
                    polygon: p,
                    bounds: box_of(&p.exterior),
                    middle: centroid(&p.exterior),
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    let towers: Vec<bool> = features
        .building
        .iter()
        .map(|b| measured(b).is_some_and(|h| h >= tall))
        .collect();
    let seeds: Vec<[f64; 2]> = shapes
        .iter()
        .filter(|s| towers[s.building])
        .map(|s| s.middle)
        .collect();
    if seeds.is_empty() {
        return Features::default();
    }
    let holds = |s: &Shape, p: [f64; 2]| in_box(&s.bounds, p) && contains(&s.polygon.exterior, p);
    let mut keep: Vec<bool> = shapes
        .iter()
        .map(|s| towers[s.building] || seeds.iter().any(|&p| holds(s, p)))
        .collect();
    // The parts within a kept outline drawn by its parts: the whole building.
    let drawn_by_parts: Vec<usize> = (0..shapes.len())
        .filter(|&k| keep[k] && features.building[shapes[k].building].hide_3d)
        .collect();
    if !drawn_by_parts.is_empty() {
        for k in 0..shapes.len() {
            if !keep[k]
                && drawn_by_parts
                    .iter()
                    .any(|&o| holds(&shapes[o], shapes[k].middle))
            {
                keep[k] = true;
            }
        }
    }
    // Each building with its kept polygons.
    let mut kept: Vec<Vec<Polygon<TilePoint>>> = vec![Vec::new(); features.building.len()];
    for (s, _) in shapes.iter().zip(&keep).filter(|(_, k)| **k) {
        kept[s.building].push(s.polygon.clone());
    }
    let building: Vec<Building<TilePoint>> = features
        .building
        .iter()
        .zip(kept)
        .filter(|(_, ps)| !ps.is_empty())
        .map(|(b, ps)| Building {
            geometry: Geometry::Polygons(ps),
            ..b.clone()
        })
        .collect();
    let middles: Vec<[f64; 2]> = shapes
        .iter()
        .zip(&keep)
        .filter(|(_, k)| **k)
        .map(|(s, _)| s.middle)
        .collect();
    let landuse = features
        .landuse
        .iter()
        .filter(|l| Usage::of_landuse(&l.class).is_some())
        .filter(|l| match &l.geometry {
            Geometry::Polygons(ps) => ps.iter().any(|p| {
                let b = box_of(&p.exterior);
                middles
                    .iter()
                    .any(|&m| in_box(&b, m) && contains(&p.exterior, m))
            }),
            _ => false,
        })
        .cloned()
        .collect();
    Features {
        building,
        landuse,
        ..Features::default()
    }
}

fn box_of(ring: &[[f64; 2]]) -> [f64; 4] {
    ring.iter()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        })
}

fn in_box(b: &[f64; 4], p: [f64; 2]) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// Whether `ring` holds `p` (even-odd).
fn contains(ring: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = ring.len() - 1;
    for i in 0..ring.len() {
        let (a, b) = (ring[i], ring[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A ring's middle: its area's centroid (its points' mean where it has no area).
fn centroid(ring: &[[f64; 2]]) -> [f64; 2] {
    let (mut a, mut cx, mut cy) = (0.0, 0.0, 0.0);
    let o = ring[0];
    for i in 0..ring.len() {
        let p = [ring[i][0] - o[0], ring[i][1] - o[1]];
        let q = ring[(i + 1) % ring.len()];
        let q = [q[0] - o[0], q[1] - o[1]];
        let cross = p[0] * q[1] - q[0] * p[1];
        a += cross;
        cx += (p[0] + q[0]) * cross;
        cy += (p[1] + q[1]) * cross;
    }
    if a.abs() < 1e-18 {
        let n = ring.len() as f64;
        return [
            ring.iter().map(|p| p[0]).sum::<f64>() / n,
            ring.iter().map(|p| p[1]).sum::<f64>() / n,
        ];
    }
    [o[0] + cx / (3.0 * a), o[1] + cy / (3.0 * a)]
}

/// The fetching thread: the asked tiles still wanted, [`FETCH_BATCH`] at a time, into the map's cache; each batch
/// landed moves [`epoch`] on.
fn fetch_work() {
    loop {
        let batch: Vec<TileId> = {
            let mut state = lock();
            loop {
                let batch = state.asked.take(FETCH_BATCH, Instant::now());
                if !batch.is_empty() {
                    break batch;
                }
                state = shared().wake.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        };
        let result = osm::fetch_missing(&batch);
        let mut state = lock();
        // Offline or a server's error: tried again in a minute (what arrived is on disk, read from there).
        state.asked.done(&batch, result.is_ok(), Instant::now());
        state.fetched += result.unwrap_or(0);
        drop(state);
        landed();
    }
}

/// A past period's thinning for far chunk `index`'s towers about `geo` in `year`: everything kept (the ground that stood
/// is the far level's own, `period_land`), the place's cap, the buildings that stood taller ([`TALL`]) and those built
/// after ([`LATER`]) about the chunk, as `period::prepare` lists them for the near levels.
pub fn thinning(geo: LatLon, index: [i32; 2], region: Region, year: u16) -> Thinning {
    let corner = [f64::from(index[0]) * CHUNK_M, f64::from(index[1]) * CHUNK_M];
    let middle =
        naval_earth::geo::to_lat_lon(geo, [corner[0] + CHUNK_M * 0.5, corner[1] + CHUNK_M * 0.5]);
    let w = 3 * CELLS;
    let mut t = Thinning::new(
        &vec![1.0; w * w],
        vec![0.0; CELLS * CELLS],
        thinning::height_cap(region, middle.lat, middle.lon),
    );
    let local = |lat: f64, lon: f64| {
        let p = to_local(geo, LatLon { lat, lon });
        [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32]
    };
    let near = |p: [f32; 2]| {
        (-100.0..CHUNK_M as f32 + 100.0).contains(&p[0])
            && (-100.0..CHUNK_M as f32 + 100.0).contains(&p[1])
    };
    for &(lat, lon, height) in TALL {
        let p = local(lat, lon);
        if near(p) {
            t.tall.push((p, height));
        }
    }
    for &(lat, lon, from) in LATER {
        let p = local(lat, lon);
        if year < from && near(p) {
            t.later.push(p);
        }
    }
    t
}

/// The year a past `period` is drawn in.
pub fn year_of(period: Period) -> Option<u16> {
    period.year().filter(|_| period.is_past())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(x: u32) -> TileId {
        TileId { z: 14, x, y: 7000 }
    }

    const TAIPEI: Scene = [1, 1];
    const ATHENS: Scene = [2, 2];

    /// A tile whose bytes will not decode is none, not an empty tile; one with nothing in it is an empty tile. Kept, the
    /// first is not held but failed (not asked again for a minute, then asked), and the far chunk wanting it is not
    /// complete.
    #[test]
    fn a_tile_that_will_not_decode_is_asked_again_not_kept_empty() {
        let geo = LatLon {
            lat: 25.03,
            lon: 121.56,
        };
        assert!(decode(geo, tile(1), &[0x1f, 0x8b, 0, 0, 7]).is_none());
        let empty = decode(geo, tile(2), &[]).expect("an empty tile");
        assert!(empty.features.building.is_empty());
        let now = Instant::now();
        let mut state = State::default();
        let (had, bad) = state.keep(TAIPEI, vec![(tile(1), None), (tile(2), Some(empty))], now);
        assert_eq!((had.len(), bad), (1, vec![tile(1)]));
        assert!(!state.kept.contains_key(&(TAIPEI, tile(1))));
        assert!(state.kept.contains_key(&(TAIPEI, tile(2))));
        assert!(!state.asked.ask(TAIPEI, [tile(1)], now));
        assert!(state.asked.ask(TAIPEI, [tile(1)], now + RETRY_AFTER));
    }

    /// Another scene's requests are dropped once one asks, and those not asked again lately; a tile asked again keeps its
    /// place; past the bound the first asked go first.
    #[test]
    fn requests_no_longer_wanted_are_dropped_and_bounded() {
        let now = Instant::now();
        let mut asked = Asked::default();
        assert!(asked.ask(TAIPEI, (0..10).map(tile), now));
        assert!(!asked.ask(TAIPEI, [tile(3)], now));
        assert_eq!(asked.len(), 10);
        // Athens asks: Taipei's are gone, Athens' fetched first.
        assert!(asked.ask(ATHENS, [tile(20), tile(21)], now));
        assert_eq!(asked.take(24, now), vec![tile(20), tile(21)]);
        assert_eq!(asked.len(), 2);
        asked.done(&[tile(20), tile(21)], true, now);
        assert_eq!(asked.len(), 0);
        // Not asked again for a while: dropped; asked again meanwhile: kept, in its place.
        asked.ask(ATHENS, [tile(30), tile(31)], now);
        asked.ask(ATHENS, [tile(31)], now + FORGET_AFTER / 2);
        assert_eq!(asked.take(24, now + FORGET_AFTER), vec![tile(31)]);
        asked.done(&[tile(31)], true, now);
        asked.ask(ATHENS, [tile(40), tile(41)], now);
        asked.ask(ATHENS, [tile(40)], now + FORGET_AFTER / 2);
        asked.ask(ATHENS, [tile(42)], now + FORGET_AFTER / 2);
        assert_eq!(asked.take(24, now + FORGET_AFTER), vec![tile(40), tile(42)]);
        // Bounded: the first asked go first.
        let mut asked = Asked::default();
        asked.ask(TAIPEI, (0..MOST_QUEUED as u32 + 10).map(tile), now);
        assert_eq!(asked.len(), MOST_QUEUED);
        assert_eq!(asked.queued.len(), MOST_QUEUED);
        assert_eq!(asked.take(1, now), vec![tile(10)]);
    }

    /// A failure is forgotten once the tile is had, and after a minute however it went: the record does not grow with
    /// every tile ever tried.
    #[test]
    fn failed_tiles_are_forgotten() {
        let now = Instant::now();
        let mut asked = Asked::default();
        asked.ask(TAIPEI, (0..4).map(tile), now);
        let batch = asked.take(24, now);
        asked.done(&batch, false, now);
        assert_eq!(asked.failed.len(), 4);
        assert!(!asked.ask(TAIPEI, (0..4).map(tile), now));
        // Fetched by a retry, or read from the disk: no longer failed.
        asked.done(&[tile(0)], true, now);
        let mut state = State {
            asked,
            ..State::default()
        };
        let empty = OsmTile::new(LatLon { lat: 0.0, lon: 0.0 }, tile(1), &Features::default());
        state.keep(TAIPEI, vec![(tile(1), Some(empty))], now);
        assert_eq!(state.asked.failed.len(), 2);
        // A minute on, the rest are gone too, asked or not.
        state.asked.forget(None, now + RETRY_AFTER);
        assert!(state.asked.failed.is_empty());
    }
}
