//! Generated wartime bases over the theatre's land (native/docs/terrain/infra.md, "Generated bases"; the catalog, the
//! site
//! fitter, the template and the meshes are `naval_terrain::infra::bases`'s). In a past period (Graphics → Buildings
//! Period) each base of the catalog standing in the scenario's year is laid out once for the scenario's frame
//! ([`made`]): its ground the window's heights, its water those heights' sea with OpenStreetMap's rivers and water
//! areas about it, its neighbours the other bases and the famous fields (`infra::period::FIELDS`), today's runway the
//! map's within its search (not a famous field's). Then:
//!
//! - **The infrastructure's chunks** draw the pieces they hold ([`about_chunk`], from `theatre::infra`'s `chunk_mesh`),
//!   and today's aeroway within a base's reach is left out ([`suppress`]).
//! - **The land** keeps its trees off each base and mows its ground ([`clearings`], the nearest bases' capsules handed to
//!   `TerrainLayer::set_clearings` each frame from `theatre::infra`'s feed).
//! - **The buildings** keep their infill and footprints off it ([`clear`], from `theatre::buildings`).
//!
//! `Period::Today` draws none. Presentation only: the simulation knows nothing of it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use glam::Vec2;
use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_osm::Geometry;
use naval_terrain::buildings::{CHUNK_M, Footprint, Period, chunk_corner, chunk_of, geometry};
use naval_terrain::infra::airfield::Airfield;
use naval_terrain::infra::bases::{self, BaseEntry, Placed, Plan, SiteInput, site, strip};
use naval_terrain::infra::period::FIELDS;
use naval_terrain::{ClearingCapsule, ClearingGroup};

use super::buildings::Context;
use super::osm;

/// A base laid out for a scenario's frame: its catalog entry, its point (theatre metres, its plan's origin) and its
/// plan.
#[derive(Debug)]
pub struct Made {
    pub entry: &'static BaseEntry,
    pub origin: [f64; 2],
    pub plan: Plan,
    /// Whether every map tile it wanted was read (else it is laid out again after [`RETRY_AFTER`]).
    pub complete: bool,
    pub at: Instant,
}

impl Made {
    /// A point of the plan in theatre metres.
    pub fn theatre(&self, p: Vec2) -> [f64; 2] {
        [
            self.origin[0] + f64::from(p.x),
            self.origin[1] + f64::from(p.y),
        ]
    }

    /// A theatre point in the plan's metres.
    pub fn local(&self, p: [f64; 2]) -> Vec2 {
        Vec2::new(
            (p[0] - self.origin[0]) as f32,
            (p[1] - self.origin[1]) as f32,
        )
    }
}

/// A base laid out without all its map is laid out again after this.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// The water mask's cell (m).
const MASK_CELL_M: f32 = 20.0;
/// How far other bases and fields are its neighbours (m).
const NEIGHBOURS_M: f64 = 12_000.0;

/// A made base's key: the scenario's frame, the heights' spacing (decimetres) and the catalog's index.
type Key = ([u64; 2], u32, usize);
type Slot = Arc<Mutex<Option<Arc<Made>>>>;

/// The made bases' slots, each locked while its base is laid out.
fn with_slots<R>(f: impl FnOnce(&mut HashMap<Key, Slot>) -> R) -> R {
    static SLOTS: Mutex<Option<HashMap<Key, Slot>>> = Mutex::new(None);
    let mut guard = SLOTS.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

/// The bases of the catalog standing in `period` (none today): each one's index, entry and point (theatre metres).
pub fn standing(geo: LatLon, period: Period) -> Vec<(usize, &'static BaseEntry, [f64; 2])> {
    let Some(year) = period.year() else {
        return Vec::new();
    };
    bases::catalog()
        .iter()
        .enumerate()
        .filter(|(_, b)| b.stands(year))
        .map(|(i, b)| {
            let at = to_local(
                geo,
                LatLon {
                    lat: b.lat,
                    lon: b.lon,
                },
            );
            (i, b, at)
        })
        .filter(|(_, _, at)| at[0].hypot(at[1]) < 2_000_000.0)
        .collect()
}

/// Base `index` laid out for `context`'s frame: made on the first asking (blocking the caller, a worker: some tens of
/// milliseconds and its map's tiles), kept after; none where no site fits.
pub fn made(context: &Context, index: usize) -> Option<Arc<Made>> {
    let entry = bases::catalog().get(index)?;
    let key = (
        [context.geo.lat.to_bits(), context.geo.lon.to_bits()],
        context.ground.cell_dm(),
        index,
    );
    // A base no site fits is not fitted again for a while (each chunk about it asks).
    static FAILED: Mutex<Option<HashMap<Key, Instant>>> = Mutex::new(None);
    let failed = |key: &Key| {
        FAILED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|f| f.get(key))
            .is_some_and(|at| at.elapsed() < RETRY_AFTER)
    };
    if failed(&key) {
        return None;
    }
    let slot = with_slots(|slots| slots.entry(key).or_default().clone());
    let mut held = slot.lock().unwrap_or_else(|e| e.into_inner());
    if failed(&key) {
        return None;
    }
    if let Some(made) = held.as_ref()
        && (made.complete || made.at.elapsed() < RETRY_AFTER)
    {
        return Some(made.clone());
    }
    let origin = to_local(
        context.geo,
        LatLon {
            lat: entry.lat,
            lon: entry.lon,
        },
    );
    let started = Instant::now();
    let made = lay_out(context, index, entry, origin).map(Arc::new);
    if made.is_none() {
        eprintln!(
            "the theatre's bases: {} not laid out ({:.1} s)",
            entry.name,
            started.elapsed().as_secs_f64()
        );
    }
    if let Some(made) = &made {
        eprintln!(
            "the theatre's bases: {} laid out ({} pieces, {} strips, {} capsules, heading {:.0}°{})",
            entry.name,
            made.plan.pieces(),
            made.plan.ribbons.len(),
            made.plan.clearing.len(),
            made.plan.site.map_or(0.0, |s| s.heading()),
            if made.plan.site.is_some_and(|s| s.aligned) {
                ", along today's runway"
            } else {
                ""
            }
        );
    }
    if made.is_none() {
        FAILED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashMap::new)
            .insert(key, Instant::now());
    }
    held.clone_from(&made);
    made
}

/// How many bases have been laid out for `geo`'s frame on heights of spacing `cell` (for the feed: what it hands
/// changes with it).
pub fn laid_out(geo: LatLon, cell: u32) -> usize {
    made_so_far(geo, Some(cell)).len()
}

/// The bases laid out for `geo`'s frame so far (for the land's clearings and the stills), without waiting on one being
/// made: those laid out on heights of spacing `cell` (decimetres, `Ground::cell_dm`) when given, so the clearings and
/// the aircraft follow the plans the chunks drew (a ship's window and an aircraft's lay a base out each).
pub fn made_so_far(geo: LatLon, cell: Option<u32>) -> Vec<Arc<Made>> {
    let frame = [geo.lat.to_bits(), geo.lon.to_bits()];
    let slots: Vec<Slot> = with_slots(|slots| {
        slots
            .iter()
            .filter(|(k, _)| k.0 == frame && cell.is_none_or(|c| k.1 == c))
            .map(|(_, s)| s.clone())
            .collect()
    });
    let mut out: Vec<Arc<Made>> = slots
        .iter()
        .filter_map(|s| s.try_lock().ok().and_then(|g| g.clone()))
        .collect();
    out.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
    out.dedup_by(|a, b| a.entry.id == b.entry.id);
    out
}

/// What base `entry` stands on about its point `origin`, and the plan laid out there.
fn lay_out(
    context: &Context,
    index: usize,
    entry: &'static BaseEntry,
    origin: [f64; 2],
) -> Option<Made> {
    let geo = context.geo;
    let size = entry.size;
    let reach = f64::from(size.reach_m());
    let (min, max) = (
        [origin[0] - reach, origin[1] - reach],
        [origin[0] + reach, origin[1] + reach],
    );
    let ids = osm::tiles_over(geo, min, max, osm::ZOOM);
    let (tiles, complete) = match osm::load(geo, &ids) {
        Ok(area) => (area.tiles, area.missing.is_empty()),
        Err(_) => (
            ids.iter().filter_map(|&id| osm::tile(geo, id)).collect(),
            false,
        ),
    };
    let ground = |p: Vec2| {
        context
            .ground
            .height([origin[0] + f64::from(p.x), origin[1] + f64::from(p.y)]) as f32
    };
    // The sea by its depth in the tropics; in Europe's lowlands (polders under the sea behind their dikes) by the map.
    let sea = if entry.template().temperate() {
        site::LOWLAND_M
    } else {
        0.5
    };
    let mask = WaterMask::new(reach as f32, sea, &ground, &tiles, origin, min, max);
    let water = |p: Vec2| mask.wet(p);
    // Its neighbours: the other bases of the catalog and the famous fields about it (but the one it is: Munda, the
    // catalog's in its Japanese years).
    let mut others: Vec<Vec2> = Vec::new();
    for (i, b) in bases::catalog().iter().enumerate() {
        if i != index {
            others.push(local_of(geo, origin, b.lat, b.lon));
        }
    }
    for f in FIELDS {
        if !f.place.holds(entry.lat, entry.lon) {
            others.push(local_of(geo, origin, f.place.lat, f.place.lon));
        }
    }
    others.retain(|o| f64::from(o.length()) < NEIGHBOURS_M);
    let runway = todays_runway(geo, entry, origin, &tiles, size.search_m());
    let input = SiteInput {
        seed: entry.seed(),
        size,
        template: entry.template(),
        gravel: entry.surface.as_deref() == Some("gravel"),
        heading_hint: entry.heading_deg,
        ground: &ground,
        water: &water,
        others: &others,
        runway,
    };
    let Some(site) = site::fit(&input) else {
        let wet = mask.wet.iter().filter(|w| **w).count() as f32 / mask.wet.len().max(1) as f32;
        eprintln!(
            "the theatre's bases: no site for {}: {:.0}% of its ground wet, its point {:.1} m above the sea",
            entry.name,
            100.0 * wet,
            ground(Vec2::ZERO)
        );
        return None;
    };
    let plan = bases::lay_out(site, &input);
    Some(Made {
        entry,
        origin,
        plan,
        complete,
        at: Instant::now(),
    })
}

fn local_of(geo: LatLon, origin: [f64; 2], lat: f64, lon: f64) -> Vec2 {
    let p = to_local(geo, LatLon { lat, lon });
    Vec2::new((p[0] - origin[0]) as f32, (p[1] - origin[1]) as f32)
}

/// The map's runway nearest the base's point within `search` of it (its middle; not a famous field's, but the one the
/// base is), its ends in the base's metres.
fn todays_runway(
    geo: LatLon,
    entry: &BaseEntry,
    origin: [f64; 2],
    tiles: &[Arc<osm::OsmTile>],
    search: f32,
) -> Option<[Vec2; 2]> {
    let index = chunk_of(origin);
    let corner = chunk_corner(index);
    let field: Airfield = super::infra::airfield_of(geo, index, tiles, Some(tiles));
    let to_base = |p: Vec2| {
        Vec2::new(
            (corner[0] + f64::from(p.x) - origin[0]) as f32,
            (corner[1] + f64::from(p.y) - origin[1]) as f32,
        )
    };
    field
        .runways
        .iter()
        .filter_map(|r| {
            let (a, b) = (to_base(r.points[0]), to_base(*r.points.last()?));
            let mid = (a + b) * 0.5;
            let at = to_lat_lon(
                geo,
                [origin[0] + f64::from(mid.x), origin[1] + f64::from(mid.y)],
            );
            let famous = FIELDS
                .iter()
                .any(|f| f.place.holds(at.lat, at.lon) && !f.place.holds(entry.lat, entry.lon));
            (mid.length() <= search && !famous).then_some((mid.length(), [a, b]))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, ends)| ends)
}

/// Where the water lies about a base, on a grid of [`MASK_CELL_M`]: the window's sea (heights under half a metre),
/// and the map's rivers, streams and water areas.
struct WaterMask {
    half: f32,
    n: usize,
    wet: Vec<bool>,
}

impl WaterMask {
    fn new(
        half: f32,
        sea: f32,
        ground: &dyn Fn(Vec2) -> f32,
        tiles: &[Arc<osm::OsmTile>],
        origin: [f64; 2],
        min: [f64; 2],
        max: [f64; 2],
    ) -> Self {
        let n = (2.0 * half / MASK_CELL_M).ceil() as usize + 1;
        let mut wet = vec![false; n * n];
        let at = |i: usize, j: usize| {
            Vec2::new(
                -half + i as f32 * MASK_CELL_M,
                -half + j as f32 * MASK_CELL_M,
            )
        };
        for j in 0..n {
            for i in 0..n {
                wet[j * n + i] = ground(at(i, j)) < sea;
            }
        }
        let local = |p: &[f64; 2]| [(p[0] - origin[0]) as f32, (p[1] - origin[1]) as f32];
        let cell_of = |v: f32| ((v + half) / MASK_CELL_M).round();
        let span = |lo: f32, hi: f32| {
            let a = cell_of(lo).max(0.0) as usize;
            let b = (cell_of(hi).max(0.0) as usize).min(n - 1);
            a..=b
        };
        for tile in tiles {
            for w in tile.waterway_in(min, max) {
                // A field's drains and ditches were filled in or piped under an airfield: only rivers, canals and
                // streams keep it off.
                if matches!(w.class.as_str(), "drain" | "ditch") {
                    continue;
                }
                let (Some(width), Geometry::Lines(lines)) =
                    (super::features::waterway_style(w), &w.geometry)
                else {
                    continue;
                };
                let reach = width + 10.0;
                for line in lines {
                    for seg in line.windows(2) {
                        let (a, b) = (Vec2::from(local(&seg[0])), Vec2::from(local(&seg[1])));
                        let (lo, hi) =
                            (a.min(b) - Vec2::splat(reach), a.max(b) + Vec2::splat(reach));
                        for j in span(lo.y, hi.y) {
                            for i in span(lo.x, hi.x) {
                                if strip::segment_distance(at(i, j), a, b) < reach {
                                    wet[j * n + i] = true;
                                }
                            }
                        }
                    }
                }
            }
            for w in tile.water_in(min, max) {
                let Geometry::Polygons(polys) = &w.geometry else {
                    continue;
                };
                for poly in polys {
                    let ring: Vec<[f32; 2]> = poly.exterior.iter().map(local).collect();
                    if ring.len() < 3 {
                        continue;
                    }
                    let holes: Vec<Vec<[f32; 2]>> = poly
                        .holes
                        .iter()
                        .map(|h| h.iter().map(local).collect())
                        .collect();
                    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
                    for p in &ring {
                        lo = lo.min(Vec2::from(*p));
                        hi = hi.max(Vec2::from(*p));
                    }
                    for j in span(lo.y, hi.y) {
                        for i in span(lo.x, hi.x) {
                            let p = at(i, j);
                            if geometry::contains_with_holes(&ring, &holes, p.to_array()) {
                                wet[j * n + i] = true;
                            }
                        }
                    }
                }
            }
        }
        Self { half, n, wet }
    }

    /// Whether `p` (the base's metres) is water; anything off the grid is.
    fn wet(&self, p: Vec2) -> bool {
        let i = ((p.x + self.half) / MASK_CELL_M).round();
        let j = ((p.y + self.half) / MASK_CELL_M).round();
        if i < 0.0 || j < 0.0 || i as usize >= self.n || j as usize >= self.n {
            return true;
        }
        self.wet[j as usize * self.n + i as usize]
    }
}

/// The bases standing in `context`'s period whose plans reach chunk `index`, made as needed.
pub fn about_chunk(context: &Context, index: [i32; 2]) -> Vec<Arc<Made>> {
    let corner = chunk_corner(index);
    let middle = [corner[0] + 0.5 * CHUNK_M, corner[1] + 0.5 * CHUNK_M];
    let mut out = Vec::new();
    for (i, entry, at) in standing(context.geo, context.period) {
        let reach = f64::from(entry.size.reach_m()) + CHUNK_M;
        if (at[0] - middle[0]).hypot(at[1] - middle[1]) > reach {
            continue;
        }
        let Some(made) = made(context, i) else {
            continue;
        };
        let (lo, hi) = (made.theatre(made.plan.min), made.theatre(made.plan.max));
        if hi[0] < corner[0]
            || hi[1] < corner[1]
            || lo[0] > corner[0] + CHUNK_M
            || lo[1] > corner[1] + CHUNK_M
        {
            continue;
        }
        out.push(made);
    }
    out
}

/// The bases about chunk `index` as its mesh takes them: each plan with its point in the chunk's metres.
pub fn placed<'a>(made: &'a [Arc<Made>], index: [i32; 2]) -> Vec<Placed<'a>> {
    let corner = chunk_corner(index);
    made.iter()
        .map(|m| Placed {
            plan: &m.plan,
            origin: Vec2::new(
                (m.origin[0] - corner[0]) as f32,
                (m.origin[1] - corner[1]) as f32,
            ),
        })
        .collect()
}

/// Leaves out of chunk `index`'s airfield (today's map) what lies within a standing base's reach or 600 m of its plan:
/// its runway is the base's now.
pub fn suppress(field: &mut Airfield, made: &[Arc<Made>], index: [i32; 2]) {
    if made.is_empty() || field.is_empty() {
        return;
    }
    let corner = chunk_corner(index);
    let theatre = |p: Vec2| [corner[0] + f64::from(p.x), corner[1] + f64::from(p.y)];
    // Within the base's search and runway, or within 600 m of anything of it (today's other runways beside a field
    // laid along one of them: Husum's).
    let within = |p: Vec2| {
        let t = theatre(p);
        made.iter().any(|m| {
            let reach = f64::from(m.entry.size.search_m() + 0.5 * m.entry.size.length());
            let local = m.local(t);
            let (lo, hi) = (
                m.plan.min - Vec2::splat(600.0),
                m.plan.max + Vec2::splat(600.0),
            );
            (t[0] - m.origin[0]).hypot(t[1] - m.origin[1]) < reach
                || (local.cmpge(lo).all() && local.cmple(hi).all())
        })
    };
    field.runways.retain(|r| {
        let (a, b) = (r.points[0], *r.points.last().unwrap_or(&r.points[0]));
        !within((a + b) * 0.5)
    });
    field
        .taxiways
        .retain(|t| t.points.first().is_none_or(|p| !within(*p)));
    field
        .aprons
        .retain(|a| a.ring.first().is_none_or(|p| !within(Vec2::from(*p))));
}

/// Drops the buildings `lots` (infill or footprints) of chunk `index` standing on a base's clearing, in a past
/// period. The buildings' own hook (`theatre::buildings`).
pub fn clear(context: &Context, index: [i32; 2], lots: &mut Vec<Footprint>) {
    if lots.is_empty() || !context.period.is_past() || !super::infra::enabled() {
        return;
    }
    let made = about_chunk(context, index);
    if made.is_empty() {
        return;
    }
    let corner = chunk_corner(index);
    lots.retain(|f| {
        let c = geometry::centroid(&f.ring);
        let p = [corner[0] + f64::from(c[0]), corner[1] + f64::from(c[1])];
        !made.iter().any(|m| m.plan.holds(m.local(p)))
    });
}

/// The tiles a base's clearing is handed to the land in (m): a land fragment tests only the capsules reaching its own.
const CLEARING_TILE_M: f32 = 700.0;
/// How far past a tile a capsule reaching into it is counted (m): the clearing's edge and the trees' widest slot.
const CLEARING_REACH_M: f32 = 70.0;

/// The clearings of the bases laid out for `geo`'s frame in `period` nearest the camera (theatre point `camera`,
/// within `within` metres), nearest first, each base's in tiles of [`CLEARING_TILE_M`] (the nearest the camera first),
/// in theatre metres moved by `to_chart` into the land's chart.
pub fn clearings(
    geo: LatLon,
    cell: u32,
    period: Period,
    camera: [f64; 2],
    within: f64,
    to_chart: [f64; 2],
) -> Vec<ClearingGroup> {
    let Some(year) = period.year() else {
        return Vec::new();
    };
    let mut near: Vec<(f64, Arc<Made>)> = made_so_far(geo, Some(cell))
        .into_iter()
        .filter(|m| m.entry.stands(year))
        .map(|m| ((m.origin[0] - camera[0]).hypot(m.origin[1] - camera[1]), m))
        .filter(|(d, _)| *d < within)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    for (_, m) in near {
        let chart = |p: Vec2| {
            let t = m.theatre(p);
            [(t[0] + to_chart[0]) as f32, (t[1] + to_chart[1]) as f32]
        };
        let (lo, hi) = (m.plan.min, m.plan.max);
        let tiles = ((hi - lo) / CLEARING_TILE_M).ceil().max(Vec2::ONE);
        let eye = m.local(camera);
        let mut groups: Vec<(f32, ClearingGroup)> = Vec::new();
        for j in 0..tiles.y as i32 {
            for i in 0..tiles.x as i32 {
                let a = lo + Vec2::new(i as f32, j as f32) * CLEARING_TILE_M;
                let b = (a + Vec2::splat(CLEARING_TILE_M)).min(hi);
                let capsules: Vec<ClearingCapsule> = m
                    .plan
                    .clearing
                    .iter()
                    .filter(|c| {
                        let r = c.radius + CLEARING_REACH_M;
                        (c.a.min(c.b) - Vec2::splat(r)).cmple(b).all()
                            && (c.a.max(c.b) + Vec2::splat(r)).cmpge(a).all()
                    })
                    .map(|c| ClearingCapsule {
                        a: chart(c.a),
                        b: chart(c.b),
                        radius: c.radius,
                        soft: c.soft,
                    })
                    .collect();
                if capsules.is_empty() {
                    continue;
                }
                let middle = (a + b) * 0.5;
                groups.push((
                    middle.distance(eye),
                    ClearingGroup {
                        min: chart(a),
                        max: chart(b),
                        capsules,
                        temperate: m.plan.temperate,
                    },
                ));
            }
        }
        groups.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.extend(groups.into_iter().map(|(_, g)| g));
    }
    out
}

/// An aircraft parked ashore that is no battle's: its type (an aircraft model's id), the ground under it (world x,
/// height above the sea, world z), how that ground rises about it (metres a metre along x and z), its heading (degrees
/// clockwise from true north) and its side number. The host draws it with its aircraft.
#[derive(Clone, Debug, PartialEq)]
pub struct ParkedPlane {
    pub model: &'static str,
    pub at: glam::DVec3,
    pub slope: [f64; 2],
    pub heading: f64,
    pub number: u16,
}

/// How far from the camera a base's parked aircraft are drawn (m): about the infrastructure's near range; past it an
/// airframe is a few pixels.
pub const PARKED_RANGE_M: f64 = 2000.0;

static PARKED_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether the bases' parked aircraft are drawn (for timing them; on by default).
pub fn set_parked_shown(shown: bool) {
    PARKED_SHOWN.store(shown, std::sync::atomic::Ordering::Relaxed);
    *super::infra::bases_handed() = None;
}

/// The aircraft parked on the bases laid out for `geo`'s frame in `period` within [`PARKED_RANGE_M`] of the camera
/// (theatre x, z and height), in the world (`offset` from theatre to world metres).
pub fn parked(
    geo: LatLon,
    cell: u32,
    period: Period,
    camera: [f64; 3],
    offset: [f64; 2],
) -> Vec<ParkedPlane> {
    let Some(year) = period.year() else {
        return Vec::new();
    };
    if !PARKED_SHOWN.load(std::sync::atomic::Ordering::Relaxed) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for m in made_so_far(geo, Some(cell)) {
        if !m.entry.stands(year) {
            continue;
        }
        for p in &m.plan.parked {
            let t = m.theatre(p.centre);
            let (dx, dz, dy) = (
                t[0] - camera[0],
                t[1] - camera[1],
                f64::from(p.ground) - camera[2],
            );
            if (dx * dx + dz * dz + dy * dy).sqrt() > PARKED_RANGE_M {
                continue;
            }
            out.push(ParkedPlane {
                model: p.model,
                at: glam::DVec3::new(t[0] + offset[0], f64::from(p.ground), t[1] + offset[1]),
                slope: [f64::from(p.slope.x), f64::from(p.slope.y)],
                heading: f64::from(p.heading),
                number: p.number,
            });
        }
    }
    out
}
