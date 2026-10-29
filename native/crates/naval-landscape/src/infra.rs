//! Ports and infrastructure over the theatre's land (native/docs/terrain/infra.md; the layer and
//! the meshes are `naval_terrain::infra`'s): container stacks on a port's terminals, boats in a marina's slips and
//! bridges over water. Presentation only, as the rest of the theatre's land: the simulation never knows of any of it.
//!
//! - **Chunks** as the buildings' (1 km, [`CHUNK_M`]): the frame loop's [`Feed`] asks for those within the layer's
//!   range of the camera, near within its near range, nearest first; one worker thread (`theatre-infra`) makes them and
//!   keeps the made meshes ([`MADE_BYTES`]); the feed hands each to the layer, which uploads a few megabytes a frame.
//! - **The map** from OpenStreetMap (`theatre::osm`, zoom 14, the tiles under the chunks with land about them and only
//!   those): the buildings' tags ([`buildings::tags_for`]: the land use, the water, the roads, the outlines, the harbour
//!   points) for the container yards; the pier lines and harbour points for the marinas ([`marina_of`]); the bridges'
//!   ways (`brunnel=bridge`), joined across the tiles within [`BRIDGE_REACH_M`] so every chunk a bridge crosses draws it
//!   alike, with the bridges' outlines (class `bridge`) and the water under them by its kind ([`bridges_of`]).
//! - **The yards' land** is the buildings' too: their infill keeps off it ([`clear_yards`]).
//! - **Power lines** from Overture Maps (`theatre::power`): a chunk made while the cells it needs were being read is
//!   made again once they are.
//!
//! Switched with the buildings (`buildings::enabled`); `NAVAL_INFRA=off` leaves it off.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use glam::Vec2;
use naval_earth::geo::LatLon;
use naval_osm::{Brunnel, Geometry, TileId};
use naval_terrain::StrokeKind;
use naval_terrain::buildings::tags::MapTags;
use naval_terrain::buildings::{
    CHUNK_M, ChunkKey, Footprint, Lod, chunk_corner, chunk_of, geometry,
};
use naval_terrain::infra::airfield::{Airfield, Apron, Runway, Taxiway};
use naval_terrain::infra::containers::YardTest;
use naval_terrain::infra::power::Power;
use naval_terrain::infra::{self, BridgeLine, InfraLayer, InfraMesh, InfraSettings, Marina, Under};

use super::buildings::{self, Context, Ground, SceneLook};
use super::features::period::in_quad;
use super::osm::{self, OsmTile};
use super::period_land;
use super::power;

/// Bytes of made meshes kept.
pub const MADE_BYTES: usize = 192 << 20;
/// The feathered edge of a generated base's clearing (m): its trees thin back to the land's over this.
const BASE_CLEARING_EDGE_M: f32 = 10.0;

/// What the bases near the camera were last handed for: the scene, where the camera stood (theatre x, z, height), the
/// infrastructure's and the land's offsets, the bases laid out and the period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BasesHanded {
    scene: u64,
    camera: [f64; 3],
    offset: [f64; 2],
    land_offset: [f64; 2],
    laid: usize,
    period: u16,
}

impl BasesHanded {
    /// The same but for the camera's moving less than 20 m.
    fn same(&self, other: &Self) -> bool {
        let moved = (self.camera[0] - other.camera[0]).hypot(self.camera[1] - other.camera[1])
            + (self.camera[2] - other.camera[2]).abs();
        moved < 20.0
            && self.scene == other.scene
            && self.offset == other.offset
            && self.land_offset == other.land_offset
            && self.laid == other.laid
            && self.period == other.period
    }
}

static BASES_HANDED: Mutex<Option<BasesHanded>> = Mutex::new(None);

/// What the bases were last handed for: forgotten (set to none), the next frame hands them again.
pub(crate) fn bases_handed() -> std::sync::MutexGuard<'static, Option<BasesHanded>> {
    BASES_HANDED.lock().unwrap_or_else(|e| e.into_inner())
}
/// Chunks made a batch.
const BATCH: usize = 8;
/// How far about a chunk its map's tiles are read (m): a quay's water, a pier reaching in, a marina's point.
const REACH_M: f64 = 150.0;
/// How far about a chunk with a bridge its bridges are read (m): a bridge that long is drawn whole by every chunk.
pub const BRIDGE_REACH_M: f64 = 2500.0;
/// How far a chunk's land may lie from it (m) for its map to be read: a bridge or a marina's slips off the shore.
const LAND_REACH_M: f64 = 300.0;
/// A chunk made while some of its tiles were missing is made again after this.
const RETRY_AFTER: Duration = Duration::from_secs(60);
const ASK_AGAIN: Duration = Duration::from_secs(5);

pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_INFRA").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    buildings::enabled() && osm::enabled() && !off
}

fn log_once(kind: &str, line: &str) {
    static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged
        .get_or_insert_with(HashSet::new)
        .insert(kind.to_owned())
    {
        eprintln!("the theatre's infrastructure: {line}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The map's data about a chunk.

/// The ground at chunk-local point `p` of chunk `index`.
fn ground_at(ground: &Ground, index: [i32; 2], p: [f32; 2]) -> f32 {
    let c = chunk_corner(index);
    ground.height([c[0] + f64::from(p[0]), c[1] + f64::from(p[1])]) as f32
}

/// Whether chunk `index` or the ground within `reach` of it holds land (any of 9 × 9 points more than 0.3 m up).
fn land_within(ground: &Ground, index: [i32; 2], reach: f64) -> bool {
    let c = chunk_corner(index);
    let side = CHUNK_M + 2.0 * reach;
    (0..9).any(|j| {
        (0..9).any(|i| {
            ground.height([
                c[0] - reach + f64::from(i) * side / 8.0,
                c[1] - reach + f64::from(j) * side / 8.0,
            ]) > 0.3
        })
    })
}

/// Whether chunk `index`'s map is worth reading: land within [`LAND_REACH_M`] of it (a quay, a pier, a marina's
/// slips), or within [`BRIDGE_REACH_M`] (the middle of a long bridge may lie over open water that far from its banks).
fn near_land(ground: &Ground, index: [i32; 2]) -> bool {
    land_within(ground, index, LAND_REACH_M) || land_within(ground, index, BRIDGE_REACH_M)
}

/// The zoom-14 tiles under chunk `index` and `reach` about it.
fn tiles_of(geo: LatLon, index: [i32; 2], reach: f64) -> Vec<TileId> {
    let c = chunk_corner(index);
    osm::tiles_over(
        geo,
        [c[0] - reach, c[1] - reach],
        [c[0] + CHUNK_M + reach, c[1] + CHUNK_M + reach],
        osm::ZOOM,
    )
}

/// Tiles `ids`: what [`osm::load`] reads (memory, the disk, the network), else what memory holds.
fn load(geo: LatLon, ids: &[TileId]) -> HashMap<TileId, Arc<OsmTile>> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let area = match osm::load(geo, ids) {
        Ok(area) => area,
        Err(_) => osm::Area {
            tiles: ids.iter().filter_map(|&id| osm::tile(geo, id)).collect(),
            missing: Vec::new(),
        },
    };
    area.tiles.into_iter().map(|t| (t.id, t)).collect()
}

/// Chunk-local coordinates of chunk `index`.
fn local_of(index: [i32; 2]) -> impl Fn(&[f64; 2]) -> [f32; 2] {
    let corner = chunk_corner(index);
    move |p: &[f64; 2]| [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32]
}

/// Whether any pier of `tiles` lies within [`REACH_M`] of chunk `index`.
fn pier_near(index: [i32; 2], tiles: &[Arc<OsmTile>]) -> bool {
    let c = chunk_corner(index);
    let (min, max) = (
        [c[0] - REACH_M, c[1] - REACH_M],
        [c[0] + CHUNK_M + REACH_M, c[1] + CHUNK_M + REACH_M],
    );
    tiles
        .iter()
        .any(|tile| tile.transportation_in(min, max).any(|t| t.class == "pier"))
}

/// A marina's docks about chunk `index`: the pier lines of its `tiles` (each tile's held to the tile proper) as
/// segments, a narrow pier area's long axis likewise, and the harbours' points of `harbour_tiles` (those within the
/// marinas' reach of it; chunk-local).
pub fn marina_of(
    geo: LatLon,
    index: [i32; 2],
    tiles: &[Arc<OsmTile>],
    harbour_tiles: &[Arc<OsmTile>],
) -> Marina {
    let local = local_of(index);
    let corner = chunk_corner(index);
    let reach = f64::from(infra::marina::MARINA_REACH_M);
    let (min, max) = (
        [corner[0] - REACH_M, corner[1] - REACH_M],
        [corner[0] + CHUNK_M + REACH_M, corner[1] + CHUNK_M + REACH_M],
    );
    let mut out = Marina::default();
    for tile in harbour_tiles {
        let quad: Vec<[f32; 2]> = tile.quad(geo).iter().map(&local).collect();
        for p in tile.poi_in(
            [corner[0] - reach, corner[1] - reach],
            [corner[0] + CHUNK_M + reach, corner[1] + CHUNK_M + reach],
        ) {
            if p.class != "harbor" {
                continue;
            }
            for at in p.geometry.points() {
                let at = local(at);
                if geometry::contains(&quad, at) {
                    out.marinas.push(Vec2::from(at));
                }
            }
        }
    }
    for tile in tiles {
        let quad: Vec<[f32; 2]> = tile.quad(geo).iter().map(&local).collect();
        for t in tile.transportation_in(min, max) {
            if t.class != "pier" {
                continue;
            }
            match &t.geometry {
                Geometry::Lines(lines) => {
                    for line in lines {
                        let ring: Vec<[f32; 2]> = line.iter().map(&local).collect();
                        for piece in geometry::clip_polyline(&ring, &quad) {
                            out.docks.extend(
                                piece
                                    .windows(2)
                                    .map(|w| [Vec2::from(w[0]), Vec2::from(w[1])]),
                            );
                        }
                    }
                }
                Geometry::Polygons(ps) => {
                    for p in ps {
                        let ring: Vec<[f32; 2]> = p.exterior.iter().map(&local).collect();
                        let middle = geometry::centroid(&ring);
                        if !geometry::contains(&quad, middle) {
                            continue;
                        }
                        if let Some(rect) = geometry::min_area_rect(&ring) {
                            let (long, short) = (
                                rect.half[0].max(rect.half[1]),
                                rect.half[0].min(rect.half[1]),
                            );
                            if short <= 4.0 && long >= 10.0 {
                                let axis = if rect.half[0] >= rect.half[1] {
                                    Vec2::from(rect.axis)
                                } else {
                                    Vec2::from(rect.axis).perp()
                                };
                                let c = Vec2::from(rect.centre);
                                out.docks.push([c - axis * long, c + axis * long]);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A water area under the bridges (chunk-local): its kind, exterior, holes and bounds (x0, z0, x1, z1).
pub type WaterUnder = (Under, Vec<[f32; 2]>, Vec<Vec<[f32; 2]>>, [f32; 4]);

/// The bridges about a chunk (chunk-local): each whole, as joined from its tiles' pieces; their outlines; and the
/// water under them by its kind, with its bounds.
#[derive(Default)]
pub struct Bridges {
    pub lines: Vec<BridgeLine>,
    pub outlines: Vec<Vec<[f32; 2]>>,
    pub water: Vec<WaterUnder>,
}

impl Bridges {
    /// What lies under `p`: the sea (an ocean's or a dock's water, or ground a metre under the sea where the map
    /// draws none), a river or lake, or dry land.
    pub fn under(&self, p: Vec2, ground: f32) -> Under {
        let mut found = Under::Dry;
        for (kind, exterior, holes, b) in &self.water {
            if *kind <= found || p.x < b[0] || p.y < b[1] || p.x > b[2] || p.y > b[3] {
                continue;
            }
            if geometry::contains_with_holes(exterior, holes, p.to_array()) {
                found = *kind;
            }
        }
        if found == Under::Dry && ground < -1.0 {
            Under::Sea
        } else {
            found
        }
    }
}

/// Whether any bridge's way of `tiles` passes within 60 m of chunk `index`.
fn bridge_near(index: [i32; 2], tiles: &[Arc<OsmTile>]) -> bool {
    let c = chunk_corner(index);
    let (min, max) = (
        [c[0] - 60.0, c[1] - 60.0],
        [c[0] + CHUNK_M + 60.0, c[1] + CHUNK_M + 60.0],
    );
    tiles.iter().any(|tile| {
        tile.transportation_in(min, max)
            .any(|t| t.brunnel == Brunnel::Bridge && matches!(t.geometry, Geometry::Lines(_)))
    })
}

/// One piece of a bridge's or runway's way: its points, its road's half width, whether it carries a railway, a tag (a
/// runway's `ref`'s hash, 0 for a bridge) that only pieces alike join, and whether the way is straight (a runway's:
/// joined only in line).
struct Piece {
    points: Vec<[f32; 2]>,
    half: f32,
    rail: bool,
    tag: u32,
    straight: bool,
}

/// The unit direction along `points` leaving its start (`start`) or arriving at its end.
fn end_direction(points: &[[f32; 2]], start: bool) -> Vec2 {
    let n = points.len();
    if n < 2 {
        return Vec2::ZERO;
    }
    let (a, b) = if start {
        (points[0], points[1])
    } else {
        (points[n - 2], points[n - 1])
    };
    (Vec2::from(b) - Vec2::from(a)).normalize_or_zero()
}

/// Pieces joined end to end (ends within 1.5 m, the same kind of way and tag; a straight way's only in line, within
/// 14°) into whole lines.
fn join(mut pieces: Vec<Piece>) -> Vec<Piece> {
    let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < 1.5;
    let mut out = Vec::new();
    while let Some(mut cur) = pieces.pop() {
        loop {
            let mut grown = false;
            let mut k = 0;
            while k < pieces.len() {
                let o = &pieces[k];
                let (cs, ce) = (cur.points[0], cur.points[cur.points.len() - 1]);
                let (os, oe) = (o.points[0], o.points[o.points.len() - 1]);
                if o.rail != cur.rail || o.tag != cur.tag {
                    k += 1;
                    continue;
                }
                // How they meet (cur's end to o's start or end, or o's to cur's start), and the directions arriving
                // at the joint and leaving it.
                let (c_in, c_out) = (
                    end_direction(&cur.points, false),
                    end_direction(&cur.points, true),
                );
                let (o_in, o_out) = (
                    end_direction(&o.points, false),
                    end_direction(&o.points, true),
                );
                let joint = if near(ce, os) {
                    Some((0, c_in, o_out))
                } else if near(ce, oe) {
                    Some((1, c_in, -o_in))
                } else if near(cs, oe) {
                    Some((2, o_in, c_out))
                } else if near(cs, os) {
                    Some((3, -o_out, c_out))
                } else {
                    None
                };
                // A runway's pieces only in line: two runways meeting at an end stay two.
                let joint =
                    joint.filter(|(_, arrive, leave)| !cur.straight || arrive.dot(*leave) > 0.97);
                match joint {
                    Some((0, ..)) => cur.points.extend_from_slice(&o.points[1..]),
                    Some((1, ..)) => cur.points.extend(o.points.iter().rev().skip(1)),
                    Some((2, ..)) => {
                        let mut points = o.points[..o.points.len() - 1].to_vec();
                        points.extend_from_slice(&cur.points);
                        cur.points = points;
                    }
                    Some(_) => {
                        let mut points: Vec<[f32; 2]> = o.points.iter().rev().copied().collect();
                        points.pop();
                        points.extend_from_slice(&cur.points);
                        cur.points = points;
                    }
                    None => {
                        k += 1;
                        continue;
                    }
                }
                cur.half = cur.half.max(o.half);
                pieces.swap_remove(k);
                grown = true;
            }
            if !grown {
                break;
            }
        }
        out.push(cur);
    }
    out
}

/// The bridges about chunk `index` from `tiles` (those within [`BRIDGE_REACH_M`] of it): the ways with
/// `brunnel=bridge` each tile holds within its proper, joined, those passing within 60 m of the chunk kept whole and
/// each run from its lesser end (by x, then z) so every chunk draws it alike; the outlines (class `bridge`) and the
/// water within the reach. A way's runs are the ones `roads`' period draws on the land (`features::Roads::runs`).
pub fn bridges_of(
    geo: LatLon,
    index: [i32; 2],
    tiles: &[Arc<OsmTile>],
    roads: &super::features::Roads<'_>,
) -> Bridges {
    let local = local_of(index);
    let corner = chunk_corner(index);
    let (min, max) = (
        [corner[0] - BRIDGE_REACH_M, corner[1] - BRIDGE_REACH_M],
        [
            corner[0] + CHUNK_M + BRIDGE_REACH_M,
            corner[1] + CHUNK_M + BRIDGE_REACH_M,
        ],
    );
    let mut out = Bridges::default();
    let mut pieces = Vec::new();
    for tile in tiles {
        let tile_quad = tile.quad(geo);
        let quad: Vec<[f32; 2]> = tile_quad.iter().map(&local).collect();
        for t in tile.transportation_in(min, max) {
            if t.class == "bridge" {
                if let Geometry::Polygons(ps) = &t.geometry {
                    out.outlines.extend(
                        ps.iter()
                            .map(|p| p.exterior.iter().map(&local).collect::<Vec<_>>())
                            .filter(|r| r.len() >= 3),
                    );
                }
                continue;
            }
            if t.brunnel != Brunnel::Bridge {
                continue;
            }
            let Some(style) = roads.style(t) else {
                continue;
            };
            let Geometry::Lines(lines) = &t.geometry else {
                continue;
            };
            for line in lines {
                // The period's runs of it (the line whole today), by the features' rules.
                let (Some(&first), Some(&last)) = (line.first(), line.last()) else {
                    continue;
                };
                let natural = [in_quad(&tile_quad, first), in_quad(&tile_quad, last)];
                let runs = roads.runs(t, line, natural);
                for run in runs {
                    let (kind, half, _) = if run.trunk {
                        super::features::trunk_style()
                    } else {
                        style
                    };
                    let rail = kind == StrokeKind::Rail;
                    let ring: Vec<[f32; 2]> = run.points.iter().map(&local).collect();
                    for piece in geometry::clip_polyline(&ring, &quad) {
                        if piece.len() >= 2 {
                            pieces.push(Piece {
                                points: piece,
                                half: if rail { 2.5 } else { half },
                                rail,
                                tag: 0,
                                straight: false,
                            });
                        }
                    }
                }
            }
        }
        for w in tile.water_in(min, max) {
            let kind = match w.class.as_str() {
                "ocean" | "dock" => Under::Sea,
                "river" | "lake" | "pond" => Under::Inland,
                _ => continue,
            };
            let Geometry::Polygons(ps) = &w.geometry else {
                continue;
            };
            for p in ps {
                let exterior: Vec<[f32; 2]> = p.exterior.iter().map(&local).collect();
                if exterior.len() < 3 {
                    continue;
                }
                let holes = p
                    .holes
                    .iter()
                    .map(|h| h.iter().map(&local).collect::<Vec<_>>())
                    .collect();
                let mut b = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
                for q in &exterior {
                    b = [
                        b[0].min(q[0]),
                        b[1].min(q[1]),
                        b[2].max(q[0]),
                        b[3].max(q[1]),
                    ];
                }
                out.water.push((kind, exterior, holes, b));
            }
        }
    }
    let side = CHUNK_M as f32;
    for mut line in join(pieces) {
        let touches =
            line.points.iter().any(|p| {
                (-60.0..side + 60.0).contains(&p[0]) && (-60.0..side + 60.0).contains(&p[1])
            }) || line.points.windows(2).any(|w| {
                geometry::segment_distance([side * 0.5, side * 0.5], w[0], w[1])
                    < side * 0.71 + 60.0
            });
        if !touches {
            continue;
        }
        let (s, e) = (line.points[0], line.points[line.points.len() - 1]);
        if (e[0], e[1]) < (s[0], s[1]) {
            line.points.reverse();
        }
        let start = line.points[0];
        let key = buildings_key([
            corner[0] + f64::from(start[0]),
            corner[1] + f64::from(start[1]),
        ]);
        out.lines.push(BridgeLine {
            points: line.points.iter().map(|p| Vec2::from(*p)).collect(),
            half: line.half,
            rail: line.rail,
            key,
        });
    }
    out
}

/// A key for a theatre point, rounded to 20 m.
fn buildings_key(p: [f64; 2]) -> i64 {
    let x = (p[0] / 20.0).round() as i64;
    let z = (p[1] / 20.0).round() as i64;
    x.wrapping_mul(73_856_093) ^ z.wrapping_mul(19_349_663)
}

// ---------------------------------------------------------------------------------------------------------------------
// The airfields.

/// How far about a chunk a runway's pieces are gathered and joined (m): the longest runways' length, so a chunk at one
/// end knows the other.
const RUNWAY_REACH_M: f64 = 4500.0;

/// Whether `tiles` hold an aeroway of one of `classes` within [`REACH_M`] of chunk `index`.
fn aeroway_near(index: [i32; 2], tiles: &[Arc<OsmTile>], classes: &[&str]) -> bool {
    let corner = chunk_corner(index);
    let (min, max) = (
        [corner[0] - REACH_M, corner[1] - REACH_M],
        [corner[0] + CHUNK_M + REACH_M, corner[1] + CHUNK_M + REACH_M],
    );
    tiles.iter().any(|t| {
        t.aeroway_in(min, max)
            .any(|a| classes.contains(&a.class.as_str()))
    })
}

/// The aeroways drawn: runways, taxiways and paved areas.
const PAVED: [&str; 4] = ["runway", "taxiway", "apron", "helipad"];

/// Whether a line (chunk-local) passes within 60 m of the chunk (its middle's circle).
fn passes_by(line: &[[f32; 2]]) -> bool {
    let side = CHUNK_M as f32;
    line.windows(2).any(|w| {
        geometry::segment_distance([side * 0.5, side * 0.5], w[0], w[1]) < side * 0.71 + 60.0
    })
}

/// Whether the theatre's runways are painted as the FAA paints them (a single figure without its leading zero): about
/// North America.
fn faa(geo: LatLon) -> bool {
    (15.0..75.0).contains(&geo.lat) && (-170.0..-50.0).contains(&geo.lon)
}

/// A string's hash, for joining only a runway's own pieces.
fn text_tag(text: Option<&str>) -> u32 {
    text.unwrap_or("").bytes().fold(2_166_136_261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    }) | 1
}

/// The airfield about chunk `index`: its runways whole from `wide` (the tiles within [`RUNWAY_REACH_M`]; `near` when
/// none were loaded), their pieces held to each tile proper and joined, those passing within 60 m of the chunk kept and
/// run from their lesser end, each as wide as its outline (class `runway` as an area) where mapped; the taxiways'
/// lines and the aprons' areas (with the taxiways', runways' and helipads' areas) of `near`, each tile's held to it.
pub fn airfield_of(
    geo: LatLon,
    index: [i32; 2],
    near: &[Arc<OsmTile>],
    wide: Option<&[Arc<OsmTile>]>,
) -> Airfield {
    let local = local_of(index);
    let corner = chunk_corner(index);
    let mut field = Airfield {
        leading_zero: !faa(geo),
        ..Airfield::default()
    };
    // The runways.
    let reach = RUNWAY_REACH_M;
    let (min, max) = (
        [corner[0] - reach, corner[1] - reach],
        [corner[0] + CHUNK_M + reach, corner[1] + CHUNK_M + reach],
    );
    let mut pieces = Vec::new();
    let mut references: HashMap<u32, Option<String>> = HashMap::new();
    let mut outlines: Vec<Vec<[f32; 2]>> = Vec::new();
    for tile in wide.unwrap_or(near) {
        let quad: Vec<[f32; 2]> = tile.quad(geo).iter().map(&local).collect();
        for a in tile.aeroway_in(min, max) {
            if a.class != "runway" {
                continue;
            }
            match &a.geometry {
                Geometry::Lines(lines) => {
                    let tag = text_tag(a.reference.as_deref());
                    references.insert(tag, a.reference.clone());
                    for line in lines {
                        let line: Vec<[f32; 2]> = line.iter().map(&local).collect();
                        for piece in geometry::clip_polyline(&line, &quad) {
                            if piece.len() >= 2 {
                                pieces.push(Piece {
                                    points: piece,
                                    half: 0.0,
                                    rail: false,
                                    tag,
                                    straight: true,
                                });
                            }
                        }
                    }
                }
                Geometry::Polygons(ps) => outlines.extend(
                    ps.iter()
                        .map(|p| p.exterior.iter().map(&local).collect::<Vec<_>>())
                        .filter(|r| r.len() >= 3),
                ),
                _ => {}
            }
        }
    }
    for mut line in join(pieces) {
        let (s, e) = (line.points[0], line.points[line.points.len() - 1]);
        // The airport's longest runway sets its taxiways' width, alike in every chunk about it.
        field.longest_runway = field
            .longest_runway
            .max(Vec2::from(s).distance(Vec2::from(e)));
        if !passes_by(&line.points) {
            continue;
        }
        if (e[0], e[1]) < (s[0], s[1]) {
            line.points.reverse();
        }
        let points: Vec<Vec2> = line.points.iter().map(|p| Vec2::from(*p)).collect();
        let (a, b) = (points[0], points[points.len() - 1]);
        let mid = (a + b) * 0.5;
        let half = outlines
            .iter()
            .filter(|o| geometry::contains(o, mid.to_array()))
            .map(|o| geometry::ring_distance(o, mid.to_array()))
            .fold(None, |m: Option<f32>, d| Some(m.map_or(d, |m| m.max(d))))
            .map_or_else(|| Runway::half_for(a.distance(b)), |d| d.clamp(10.0, 40.0));
        field.runways.push(Runway {
            points,
            reference: references.get(&line.tag).cloned().flatten(),
            half,
        });
    }
    // The taxiways and the paved areas, of the tiles about the chunk.
    let (min, max) = (
        [corner[0] - REACH_M, corner[1] - REACH_M],
        [corner[0] + CHUNK_M + REACH_M, corner[1] + CHUNK_M + REACH_M],
    );
    for tile in near {
        let quad: Vec<[f32; 2]> = tile.quad(geo).iter().map(&local).collect();
        for a in tile.aeroway_in(min, max) {
            match (&a.geometry, a.class.as_str()) {
                (Geometry::Lines(lines), "taxiway") => {
                    for line in lines {
                        let line: Vec<[f32; 2]> = line.iter().map(&local).collect();
                        for piece in geometry::clip_polyline(&line, &quad) {
                            if piece.len() >= 2 && passes_by(&piece) {
                                field.taxiways.push(Taxiway {
                                    points: piece.iter().map(|p| Vec2::from(*p)).collect(),
                                    reference: a.reference.clone(),
                                });
                            }
                        }
                    }
                }
                (Geometry::Polygons(ps), "apron" | "taxiway" | "runway" | "helipad") => {
                    for p in ps {
                        let mut ring: Vec<[f32; 2]> = p.exterior.iter().map(&local).collect();
                        if geometry::signed_area2(&ring) < 0.0 {
                            ring.reverse();
                        }
                        let piece = geometry::clip_polygon(&ring, &quad);
                        if piece.len() >= 3 {
                            field.aprons.push(Apron {
                                ring: piece,
                                runway: a.class == "runway",
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
    field
}

// ---------------------------------------------------------------------------------------------------------------------
// The period's (native/docs/terrain/period.md, "The period's ground", "The roads"; the lists are `infra::period`'s).

/// The place of chunk `index`'s local point `p`.
fn lat_lon_of(geo: LatLon, index: [i32; 2], p: [f32; 2]) -> LatLon {
    let c = chunk_corner(index);
    naval_earth::geo::to_lat_lon(geo, [c[0] + f64::from(p[0]), c[1] + f64::from(p[1])])
}

/// Chunk `index`'s airfield as it stood in `year` (`infra::period::field_then`): none of an aerodrome paved after it
/// (its runways judged by their middles, its taxiways and aprons by their first points), the rest's runways cut to
/// their field's grounds then (Mines Field's square mile) and no longer and no wider than the period's (shortened alike
/// from both ends), painted as they were then.
pub fn period_airfield(field: &mut Airfield, geo: LatLon, index: [i32; 2], year: u16) {
    use infra::period::{FieldThen, field_then};
    if field.is_empty() && field.longest_runway == 0.0 {
        return;
    }
    let then = |p: Vec2| {
        let at = lat_lon_of(geo, index, p.to_array());
        (field_then(year, at.lat, at.lon), at)
    };
    let within = |grounds: &Option<[f64; 4]>, at: LatLon| {
        grounds.is_none_or(|b| (b[0]..=b[2]).contains(&at.lat) && (b[1]..=b[3]).contains(&at.lon))
    };
    // Each runway as it was, or none.
    let runways = std::mem::take(&mut field.runways);
    for mut runway in runways {
        let (a, b) = (runway.points[0], runway.points[runway.points.len() - 1]);
        let FieldThen::Stood {
            length_m,
            half_m,
            grounds,
            ..
        } = then((a + b) * 0.5).0
        else {
            continue;
        };
        // Its part on the field's grounds then (sampled every 10 m).
        let (mut a, mut b) = (a, b);
        if grounds.is_some() {
            let n = (a.distance(b) / 10.0).ceil().max(1.0) as usize;
            let inside: Vec<Vec2> = (0..=n)
                .map(|k| a.lerp(b, k as f32 / n as f32))
                .filter(|p| within(&grounds, then(*p).1))
                .collect();
            let (Some(first), Some(last)) = (inside.first(), inside.last()) else {
                continue;
            };
            (a, b) = (*first, *last);
        }
        let length = a.distance(b);
        if length > length_m {
            let (mid, d) = ((a + b) * 0.5, (b - a) / length);
            (a, b) = (mid - d * (length_m * 0.5), mid + d * (length_m * 0.5));
        }
        runway.points = vec![a, b];
        runway.half = runway.half.min(half_m);
        field.runways.push(runway);
    }
    // The taxiways and aprons of a field that stood, on its grounds; no runway's mapped area (today's length and width:
    // its capped line above draws it).
    let kept = |p: Vec2| match then(p) {
        (FieldThen::Stood { grounds, .. }, at) => within(&grounds, at),
        (FieldThen::None, _) => false,
    };
    field
        .taxiways
        .retain(|t| t.points.first().is_none_or(|p| kept(*p)));
    field
        .aprons
        .retain(|a| !a.runway && a.ring.first().is_none_or(|p| kept(Vec2::from(*p))));
    // The airport's longest runway (its taxiways' width) as the field about the chunk had it.
    let middle = Vec2::splat(CHUNK_M as f32 * 0.5);
    field.longest_runway = match then(middle).0 {
        FieldThen::Stood { length_m, .. } => field.longest_runway.min(length_m),
        FieldThen::None => 0.0,
    };
    field.paint = Some(infra::period::runway_paint(year));
}

/// Chunk `index`'s bridges as they stood in `year`: none built after it (`infra::period::bridge_stood`, by the line's
/// middle and its main span over the sea, `bridges::profile`'s).
fn period_bridges(
    bridges: &mut Bridges,
    geo: LatLon,
    index: [i32; 2],
    year: u16,
    ground: &dyn Fn([f32; 2]) -> f32,
) {
    if bridges.lines.is_empty() {
        return;
    }
    let ground_at = |p: Vec2| ground(p.to_array());
    let under = |p: Vec2| bridges.under(p, ground(p.to_array()));
    let input = infra::bridges::BridgeInput {
        lines: &bridges.lines,
        outlines: &bridges.outlines,
        ground: &ground_at,
        under: &under,
    };
    let keep: Vec<bool> = bridges
        .lines
        .iter()
        .map(|line| {
            let sea_m = infra::bridges::profile(line, &input)
                .filter(|p| p.under == Under::Sea)
                .map_or(0.0, |p| f64::from(p.span.1 - p.span.0));
            let mid = (line.points[0] + line.points[line.points.len() - 1]) * 0.5;
            let at = lat_lon_of(geo, index, mid.to_array());
            infra::period::bridge_stood(year, at.lat, at.lon, sea_m)
        })
        .collect();
    let mut k = 0;
    bridges.lines.retain(|_| {
        k += 1;
        keep[k - 1]
    });
}

/// Drops the buildings' infill `lots` of chunk `index` that stand on an airfield (`tiles` about it): within an
/// aerodrome's outline, on an apron or within 60 m of a runway's or taxiway's line. The buildings' own hook
/// (`theatre::buildings`).
pub fn clear_airfields(index: [i32; 2], tiles: &[Arc<OsmTile>], lots: &mut Vec<Footprint>) {
    if !enabled() || lots.is_empty() {
        return;
    }
    let local = local_of(index);
    let corner = chunk_corner(index);
    let (min, max) = (
        [corner[0] - REACH_M, corner[1] - REACH_M],
        [corner[0] + CHUNK_M + REACH_M, corner[1] + CHUNK_M + REACH_M],
    );
    let mut areas: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut lines: Vec<Vec<[f32; 2]>> = Vec::new();
    for tile in tiles {
        for a in tile.aeroway_in(min, max) {
            match &a.geometry {
                Geometry::Polygons(ps) => areas.extend(
                    ps.iter()
                        .map(|p| p.exterior.iter().map(&local).collect::<Vec<_>>())
                        .filter(|r| r.len() >= 3),
                ),
                Geometry::Lines(ls) if matches!(a.class.as_str(), "runway" | "taxiway") => {
                    lines.extend(ls.iter().map(|l| l.iter().map(&local).collect::<Vec<_>>()));
                }
                _ => {}
            }
        }
    }
    if areas.is_empty() && lines.is_empty() {
        return;
    }
    lots.retain(|f| {
        let c = geometry::centroid(&f.ring);
        !areas.iter().any(|r| geometry::contains(r, c))
            && !lines.iter().any(|l| {
                l.windows(2)
                    .any(|w| geometry::segment_distance(c, w[0], w[1]) < 60.0)
            })
    });
}

/// Drops the buildings' infill `lots` of chunk `index` that stand on a container yard (`tags` the chunk's map): the
/// stacks stand there. The buildings' own hook (`theatre::buildings`).
pub fn clear_yards(tags: &MapTags, ground: &Ground, index: [i32; 2], lots: &mut Vec<Footprint>) {
    if !enabled() || lots.is_empty() {
        return;
    }
    let ground = |p: [f32; 2]| ground_at(ground, index, p);
    let yard = YardTest::new(tags, &ground);
    if !yard.port {
        return;
    }
    lots.retain(|f| !yard.holds(geometry::centroid(&f.ring)));
}

/// A chunk's tiles: those about it (within [`REACH_M`]), and those within the marinas' reach when a pier is near, the
/// bridges' when a bridge is and the runways' when a runway is (in their ids' order, so the joins are the same each
/// time); whether an airfield's paving is near; and the power lines about it.
struct ChunkTiles {
    near: Vec<Arc<OsmTile>>,
    harbours: Option<Vec<Arc<OsmTile>>>,
    bridges: Option<Vec<Arc<OsmTile>>>,
    runways: Option<Vec<Arc<OsmTile>>>,
    paved: bool,
    power: Power,
}

/// One chunk's mesh from its tiles, and whether every base it drew was laid out with all its map (else the chunk is
/// made again with the base's next laying out).
fn chunk_mesh(context: &Context, chunk: ChunkKey, tiles: &ChunkTiles) -> (InfraMesh, bool) {
    let index = chunk.index;
    let near = &tiles.near;
    let tags = buildings::tags_for(context.geo, index, near);
    let ground = |p: [f32; 2]| ground_at(&context.ground, index, p);
    let marina = match &tiles.harbours {
        Some(harbours) if chunk.lod == Lod::Full => marina_of(context.geo, index, near, harbours),
        _ => Marina::default(),
    };
    // The period's roads on the bridges, as on the land: the ground's standing as `period_land` has it now (the chunk
    // is made again when that changes, `Made::ground`).
    let (geo, period) = (context.geo, context.period);
    let stood = move |p: [f64; 2]| period_land::stood(geo, period, p);
    let middle = chunk_corner(index).map(|c| c + CHUNK_M * 0.5);
    let roads = super::features::Roads::new(period, geo, middle, &stood);
    let mut bridges = tiles
        .bridges
        .as_deref()
        .map_or_else(Bridges::default, |tiles| {
            bridges_of(context.geo, index, tiles, &roads)
        });
    if let Some(year) = context.period.year() {
        period_bridges(&mut bridges, context.geo, index, year, &ground);
    }
    let mut airfield = if tiles.paved {
        airfield_of(context.geo, index, near, tiles.runways.as_deref())
    } else {
        Airfield::default()
    };
    if let Some(year) = context.period.year() {
        period_airfield(&mut airfield, context.geo, index, year);
    }
    // The generated bases standing in the period (none today), and today's aeroway within their reach left out.
    let bases = super::bases::about_chunk(context, index);
    let bases_complete = bases.iter().all(|m| m.complete);
    super::bases::suppress(&mut airfield, &bases, index);
    let placed = super::bases::placed(&bases, index);
    let water = |p: Vec2| tags.is_water(p.to_array(), &ground);
    let under = |p: Vec2| bridges.under(p, ground(p.to_array()));
    let mesh = infra::build_chunk(&infra::ChunkInput {
        key: chunk,
        tags: &tags,
        ground: &ground,
        marina: &marina,
        water: &water,
        bridges: &bridges.lines,
        bridge_outlines: &bridges.outlines,
        under: &under,
        airfield: &airfield,
        power: &tiles.power,
        period: context.period,
        bases: &placed,
    });
    (mesh, bases_complete)
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// A made mesh's key: the scenario's frame, the heights' spacing (decimetres), the period (`Period::key`) and the
/// chunk.
type MadeKey = ([u64; 2], u32, u16, ChunkKey);

fn made_key(context: &Context, chunk: ChunkKey) -> MadeKey {
    let cell = match &context.ground {
        Ground::Window { field, .. } => field.heights().cell,
        Ground::Battle { field, .. } => f64::from(field.cell),
    };
    (
        [context.geo.lat.to_bits(), context.geo.lon.to_bits()],
        (cell * 10.0).round() as u32,
        context.period.key(),
        chunk,
    )
}

struct Made {
    mesh: Arc<InfraMesh>,
    complete: bool,
    at: Instant,
    /// Made while the power lines' cells were being read: [`power::epoch`] then.
    power_epoch: Option<u64>,
    /// A past period's bridges made from the ground's standing: what they read.
    ground: Option<GroundRead>,
}

impl Made {
    fn stale(&self) -> bool {
        (!self.complete && self.at.elapsed() >= RETRY_AFTER)
            || self.power_epoch.is_some_and(|e| e != power::epoch())
            || self.ground.as_ref().is_some_and(|g| g.stale(self.at))
    }
}

/// The least time between a chunk whose bridges read the ground being made and made again: the buildings' thinning
/// arrives a few chunks a frame about the camera (`period_land::FRAME_CHUNKS`).
const REMAKE_AFTER: Duration = Duration::from_secs(1);

/// The ground a past period's bridges were made from: `period_land::version` then (or when last found unchanged), the
/// scenario's frame and period, and the buildings' chunks (first column, first row, last column, last row) their ways
/// reach.
struct GroundRead {
    version: std::sync::atomic::AtomicU64,
    geo: LatLon,
    period: naval_terrain::buildings::Period,
    chunks: [i32; 4],
}

impl GroundRead {
    fn new(context: &Context, index: [i32; 2]) -> Self {
        let k = ((BRIDGE_REACH_M + super::features::period::ISLAND_M) / CHUNK_M).ceil() as i32;
        Self {
            version: period_land::version().into(),
            geo: context.geo,
            period: context.period,
            chunks: [index[0] - k, index[1] - k, index[0] + k, index[1] + k],
        }
    }

    /// Whether the ground under them changed since, the chunk made at `made` long enough ago to be made again.
    fn stale(&self, made: Instant) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        let version = period_land::version();
        let since = self.version.load(Relaxed);
        if since >= version || made.elapsed() < REMAKE_AFTER {
            return false;
        }
        if period_land::changed_since(self.geo, self.period, since, self.chunks, true) {
            return true;
        }
        self.version.store(version, Relaxed);
        false
    }
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct WorkerStatus {
    pub queued: usize,
    pub working: usize,
    pub made: usize,
    pub made_bytes: usize,
    /// Milliseconds the last batch took, and its chunks.
    pub last_batch_ms: u64,
    pub last_batch: usize,
}

struct State {
    context: Option<Context>,
    wanted: VecDeque<ChunkKey>,
    made: HashMap<MadeKey, Made>,
    order: VecDeque<MadeKey>,
    made_bytes: usize,
    status: WorkerStatus,
    worker: bool,
}

struct Streamer {
    state: Mutex<State>,
    wake: Condvar,
}

fn streamer() -> &'static Streamer {
    static STREAMER: OnceLock<Streamer> = OnceLock::new();
    STREAMER.get_or_init(|| Streamer {
        state: Mutex::new(State {
            context: None,
            wanted: VecDeque::new(),
            made: HashMap::new(),
            order: VecDeque::new(),
            made_bytes: 0,
            status: WorkerStatus::default(),
            worker: false,
        }),
        wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    streamer().state.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        made: state.made.len(),
        made_bytes: state.made_bytes,
        ..state.status
    }
}

/// Asks for `chunks` (nearest first) over `context`, in place of whatever was asked before.
fn want(context: &Context, chunks: &[ChunkKey]) {
    if !enabled() {
        return;
    }
    let mut state = lock();
    let wanted: VecDeque<ChunkKey> = chunks
        .iter()
        .copied()
        .filter(|&c| {
            state
                .made
                .get(&made_key(context, c))
                .is_none_or(Made::stale)
        })
        .collect();
    state.context = Some(context.clone());
    state.status.queued = wanted.len();
    state.wanted = wanted;
    if state.wanted.is_empty() {
        return;
    }
    if !state.worker {
        state.worker = true;
        if let Err(e) = crate::host::spawn_utility("theatre-infra", work) {
            state.worker = false;
            log_once("thread", &format!("no worker thread: {e}"));
        }
    }
    streamer().wake.notify_one();
}

fn work() {
    loop {
        let (context, batch) = {
            let mut state = lock();
            loop {
                if let Some(context) = state.context.clone() {
                    let mut batch = Vec::with_capacity(BATCH);
                    while batch.len() < BATCH
                        && let Some(chunk) = state.wanted.pop_front()
                    {
                        if state
                            .made
                            .get(&made_key(&context, chunk))
                            .is_none_or(Made::stale)
                        {
                            batch.push(chunk);
                        }
                    }
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (context, batch);
                    }
                }
                state.status.working = 0;
                state = streamer()
                    .wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = Instant::now();
        let meshes = make(&context, &batch);
        let mut state = lock();
        state.status.last_batch_ms = started.elapsed().as_millis() as u64;
        state.status.last_batch = batch.len();
        for (chunk, mesh, complete, power_epoch, ground) in meshes {
            let k = made_key(&context, chunk);
            let bytes = mesh.bytes();
            let made = Made {
                mesh,
                complete,
                at: Instant::now(),
                power_epoch,
                ground,
            };
            if let Some(old) = state.made.insert(k, made) {
                state.made_bytes -= old.mesh.bytes();
            } else {
                state.order.push_back(k);
            }
            state.made_bytes += bytes;
        }
        while state.made_bytes > MADE_BYTES
            && let Some(old) = state.order.pop_front()
        {
            if let Some(made) = state.made.remove(&old) {
                state.made_bytes -= made.mesh.bytes();
            }
        }
    }
}

/// A made chunk: its key, its mesh, whether every tile it wanted was had, [`power::epoch`] when it was made while its
/// power lines were being read, and the ground a past period's bridges were made from.
type MadeChunk = (
    ChunkKey,
    Arc<InfraMesh>,
    bool,
    Option<u64>,
    Option<GroundRead>,
);

/// The batch's meshes.
fn make(context: &Context, batch: &[ChunkKey]) -> Vec<MadeChunk> {
    let land: Vec<bool> = batch
        .iter()
        .map(|c| near_land(&context.ground, c.index))
        .collect();
    let mut ids: Vec<TileId> = batch
        .iter()
        .zip(&land)
        .filter(|(_, l)| **l)
        .flat_map(|(c, _)| tiles_of(context.geo, c.index, REACH_M))
        .collect();
    ids.sort_by_key(|t| (t.z, t.x, t.y));
    ids.dedup();
    let tiles = load(context.geo, &ids);
    let mut out = Vec::with_capacity(batch.len());
    for (&chunk, &land) in batch.iter().zip(&land) {
        if !land {
            out.push((chunk, Arc::new(InfraMesh::default()), true, None, None));
            continue;
        }
        let wanted = tiles_of(context.geo, chunk.index, REACH_M);
        let near: Vec<Arc<OsmTile>> = wanted
            .iter()
            .filter_map(|id| tiles.get(id).cloned())
            .collect();
        let mut complete = near.len() == wanted.len();
        // The tiles within `reach`, in the ids' order so the joins (and the mesh) are the same each time.
        let mut within = |reach: f64| {
            let ids = tiles_of(context.geo, chunk.index, reach);
            let had = load(context.geo, &ids);
            complete &= had.len() == ids.len();
            ids.iter()
                .filter_map(|id| had.get(id).cloned())
                .collect::<Vec<_>>()
        };
        let harbours = (chunk.lod == Lod::Full && pier_near(chunk.index, &near))
            .then(|| within(f64::from(infra::marina::MARINA_REACH_M)));
        let bridges = bridge_near(chunk.index, &near).then(|| within(BRIDGE_REACH_M));
        let paved = aeroway_near(chunk.index, &near, &PAVED);
        // The runways about the chunk whole, and the airport's longest for its taxiways' width.
        let runways = paved.then(|| within(RUNWAY_REACH_M));
        // The epoch before the cells are asked for: one had meanwhile moves it past this.
        let epoch = power::epoch();
        let power = power::about(context.geo, chunk.index);
        let power_epoch = power.is_none().then_some(epoch);
        // A cell that could not be read: the chunk is made again after `RETRY_AFTER`, as for a missing tile.
        let power = power.map(|(power, had)| {
            complete &= had;
            power
        });
        let ground = (context.period.is_past() && bridges.is_some())
            .then(|| GroundRead::new(context, chunk.index));
        let tiles = ChunkTiles {
            near,
            harbours,
            bridges,
            runways,
            paved,
            power: power.unwrap_or_default(),
        };
        let (mesh, bases_complete) = chunk_mesh(context, chunk, &tiles);
        complete &= bases_complete;
        out.push((chunk, Arc::new(mesh), complete, power_epoch, ground));
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The scene's side.

/// Where the camera stood at the last frame a scene fed its infrastructure.
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 3])>> = Mutex::new(None);

/// The chunks within `settings`' range of the camera at theatre point (x, z) and height `camera[2]`, nearest first,
/// each at its level (`held` the one a chunk is drawn at now).
pub fn wanted(
    camera: [f64; 3],
    settings: &InfraSettings,
    held: &dyn Fn([i32; 2]) -> Option<Lod>,
) -> Vec<(f32, ChunkKey)> {
    let reach = (f64::from(settings.range_m) / CHUNK_M).ceil() as i32 + 1;
    let at = chunk_of([camera[0], camera[1]]);
    let mut out = Vec::new();
    for dj in -reach..=reach {
        for di in -reach..=reach {
            let index = [at[0] + di, at[1] + dj];
            let c = chunk_corner(index);
            let dx = (c[0] - camera[0])
                .max(camera[0] - (c[0] + CHUNK_M))
                .max(0.0);
            let dz = (c[1] - camera[1])
                .max(camera[1] - (c[1] + CHUNK_M))
                .max(0.0);
            let dy = (camera[2] - 40.0).max(0.0);
            let d = (dx * dx + dz * dz + dy * dy).sqrt() as f32;
            if let Some(lod) = settings.lod_at(d, held(index)) {
                out.push((d, ChunkKey { index, lod }));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The layer for a scene being built (off the frame loop's thread), its theatre at `offset`, with the made chunks
/// about where the camera last stood. None with the infrastructure off.
pub fn layer(
    ctx: &mut naval_render::RenderContext<'_>,
    scene: &SceneLook<'_>,
    quality: &naval_terrain::TerrainQuality,
    context: &Context,
    offset: [f64; 2],
) -> Option<InfraLayer> {
    if !enabled() {
        return None;
    }
    let mut layer =
        match InfraLayer::new(ctx, scene.environment, InfraSettings::for_quality(quality)) {
            Ok(layer) => layer,
            Err(e) => {
                log_once("shaders", &format!("the infrastructure's shaders: {e:?}"));
                return None;
            }
        };
    layer.set_offset(offset);
    layer.set_cloud_shadow(Some(scene.cloud_shadow));
    layer.set_fog(scene.fog);
    let last = *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, camera)) =
        last.filter(|(frame, _)| *frame == [context.geo.lat.to_bits(), context.geo.lon.to_bits()])
    {
        let settings = layer.settings();
        let state = lock();
        for (_, key) in wanted(camera, &settings, &|_| None) {
            if let Some(made) = state.made.get(&made_key(context, key)) {
                layer.queue_chunk(key, made.mesh.clone());
            }
        }
        drop(state);
        layer.flush(ctx);
    }
    Some(layer)
}

/// What a scene on screen does with its infrastructure, from frame to frame.
#[derive(Default)]
pub struct Feed {
    pub scene: u64,
    about: Option<[f64; 3]>,
    made_at: Option<Instant>,
    wanted: Vec<(f32, ChunkKey)>,
    asked: Vec<ChunkKey>,
    asked_at: Option<Instant>,
    pub pending: usize,
    frame: Option<[u64; 2]>,
}

static FEED: Mutex<Option<Feed>> = Mutex::new(None);

impl Feed {
    /// Hands `layer` the made chunks about its camera, drops those out of range and asks the worker for the rest; a
    /// chunk whose own level is not made yet keeps (or takes) the other level's mesh meanwhile.
    pub fn feed(&mut self, layer: &mut InfraLayer, context: &Context) {
        let Some(camera) = layer.camera_theatre() else {
            return;
        };
        let frame = [context.geo.lat.to_bits(), context.geo.lon.to_bits()];
        *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner()) = Some((frame, camera));
        let changed = self.frame != Some(frame);
        self.frame = Some(frame);
        let moved = self.about.is_none_or(|a| {
            (a[0] - camera[0]).hypot(a[1] - camera[1]) > 100.0 || (a[2] - camera[2]).abs() > 100.0
        });
        let stale = self
            .made_at
            .is_none_or(|t| t.elapsed() > Duration::from_millis(500));
        if changed || moved || stale {
            self.about = Some(camera);
            self.made_at = Some(Instant::now());
            let settings = layer.settings();
            self.wanted = wanted(camera, &settings, &|i| layer.held_lod(i));
            let keep: HashSet<[i32; 2]> = self.wanted.iter().map(|(_, k)| k.index).collect();
            for index in layer.held() {
                if !keep.contains(&index) {
                    layer.remove_chunk(index);
                }
            }
        }
        let mut missing = Vec::new();
        {
            let state = lock();
            for &(_, key) in &self.wanted {
                match state.made.get(&made_key(context, key)) {
                    Some(made) => {
                        if !layer.holds_mesh(key, &made.mesh) {
                            layer.queue_chunk(key, made.mesh.clone());
                        }
                        if made.stale() {
                            missing.push(key);
                        }
                    }
                    None => {
                        if layer.holds(key) {
                            continue;
                        }
                        missing.push(key);
                        let other = ChunkKey {
                            lod: match key.lod {
                                Lod::Full => Lod::Box,
                                Lod::Box | Lod::Far => Lod::Full,
                            },
                            ..key
                        };
                        if layer.held_lod(key.index).is_none()
                            && let Some(made) = state.made.get(&made_key(context, other))
                        {
                            layer.queue_chunk(other, made.mesh.clone());
                        }
                    }
                }
            }
        }
        self.pending = missing.len();
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > ASK_AGAIN);
        if missing != self.asked || changed || (!missing.is_empty() && stale) {
            want(context, &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
    }
}

/// Feeds the scene's infrastructure (`scene_id` the driver's swap count) with `context`, the theatre at `offset`.
pub fn feed(scene_id: u64, layer: &mut InfraLayer, context: &Context, offset: [f64; 2]) {
    layer.set_offset(offset);
    let mut feed = FEED.lock().unwrap_or_else(|e| e.into_inner());
    if feed.as_ref().is_none_or(|f| f.scene != scene_id) {
        *feed = Some(Feed {
            scene: scene_id,
            ..Feed::default()
        });
    }
    if let Some(feed) = feed.as_mut() {
        feed.feed(layer, context);
    }
}

/// The infrastructure's state for the RPC (`theatre_state`'s `infra`): `state` `off`, `none`, `waiting` or `drawn`;
/// the layer's chunks, what they hold and their triangles; the worker's.
pub fn status_json(scene_id: u64, layer: Option<&InfraLayer>) -> serde_json::Value {
    if !enabled() {
        return serde_json::json!({"state": "off"});
    }
    let Some(layer) = layer else {
        return serde_json::json!({"state": "none"});
    };
    let s = layer.status();
    let pending = FEED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|f| f.scene == scene_id)
        .map_or(0, |f| f.pending);
    let power = power::status();
    let state = if pending > 0
        || s.queued > 0
        || power.cells_queued > 0
        || layer.camera_theatre().is_none()
    {
        "waiting"
    } else {
        "drawn"
    };
    serde_json::json!({
        "state": state,
        "pending": pending,
        "layer": s,
        "worker": status(),
        "power": power,
    })
}

/// The bases near the camera of a scene's infrastructure (`scene_id` the driver's swap count, the theatre at `offset`):
/// their clearings handed to the scene's land (no tree on them, their ground bare) and the aircraft parked on them
/// returned, for the scene's aircraft to draw. Handed again only when the camera has moved, the land's offset changed
/// or another base was laid out: none returned otherwise, nor before the layer has a camera.
pub fn hand_bases(
    scene_id: u64,
    layer: &InfraLayer,
    land: Option<&mut naval_terrain::TerrainLayer>,
    context: &Context,
    offset: [f64; 2],
) -> Option<Vec<super::bases::ParkedPlane>> {
    let camera = layer.camera_theatre()?;
    let range = f64::from(layer.settings().range_m);
    let land_offset = land.as_ref().map_or([0.0; 2], |l| l.offset());
    let cell = context.ground.cell_dm();
    let laid = super::bases::laid_out(context.geo, cell);
    let key = BasesHanded {
        scene: scene_id,
        camera,
        offset,
        land_offset,
        laid,
        period: context.period.key(),
    };
    let mut handed = BASES_HANDED.lock().unwrap_or_else(|e| e.into_inner());
    if handed.as_ref().is_some_and(|h| h.same(&key)) {
        return None;
    }
    *handed = Some(key);
    drop(handed);
    if let Some(land) = land {
        let to_chart = [offset[0] - land_offset[0], offset[1] - land_offset[1]];
        let groups = super::bases::clearings(
            context.geo,
            cell,
            context.period,
            [camera[0], camera[1]],
            range + 4000.0,
            to_chart,
        );
        land.set_clearings(&groups, BASE_CLEARING_EDGE_M);
    }
    Some(super::bases::parked(
        context.geo,
        cell,
        context.period,
        camera,
        offset,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(points: &[[f32; 2]]) -> Piece {
        Piece {
            points: points.to_vec(),
            half: 4.0,
            rail: false,
            tag: 0,
            straight: false,
        }
    }

    #[test]
    fn pieces_join_end_to_end_whichever_way_they_run() {
        let joined = join(vec![
            piece(&[[0.0, 0.0], [100.0, 0.0]]),
            piece(&[[300.0, 0.0], [200.0, 0.0]]),
            piece(&[[100.5, 0.0], [200.0, 0.5]]),
            piece(&[[0.0, 500.0], [10.0, 500.0]]),
        ]);
        assert_eq!(joined.len(), 2);
        let long = joined.iter().find(|p| p.points.len() > 2).unwrap();
        assert_eq!(long.points.len(), 4);
        let xs: Vec<f32> = long.points.iter().map(|p| p[0]).collect();
        assert!(
            xs.windows(2).all(|w| w[1] > w[0]) || xs.windows(2).all(|w| w[1] < w[0]),
            "{xs:?}"
        );
    }

    #[test]
    fn two_runways_meeting_at_an_end_stay_two() {
        let tagged = |points: &[[f32; 2]], reference: Option<&str>| Piece {
            tag: text_tag(reference),
            straight: true,
            ..piece(points)
        };
        let joined = join(vec![
            tagged(&[[0.0, 0.0], [1500.0, 0.0]], Some("09/27")),
            tagged(&[[1500.0, 0.0], [3000.0, 0.0]], Some("09/27")),
            tagged(&[[3000.0, 0.0], [4000.0, 800.0]], Some("13/31")),
        ]);
        assert_eq!(joined.len(), 2);
        assert!(joined.iter().any(|p| p.points.len() == 3));
        // Without their refs: the pieces in line join, the runway turning off at the end stays its own.
        let joined = join(vec![
            tagged(&[[0.0, 0.0], [1500.0, 0.0]], None),
            tagged(&[[3000.0, 0.0], [1500.0, 0.0]], None),
            tagged(&[[3000.0, 0.0], [4000.0, 800.0]], None),
        ]);
        assert_eq!(joined.len(), 2);
        assert!(joined.iter().any(|p| p.points.len() == 3));
        assert!(!faa(LatLon {
            lat: -9.43,
            lon: 160.05
        }));
        assert!(faa(LatLon {
            lat: 33.94,
            lon: -118.4
        }));
    }

    /// A runway from (`lat0`, `lon0`) to (`lat1`, `lon1`) in the frame about `geo`: the chunk holding its middle and
    /// the airfield it makes there.
    fn field_about(geo: LatLon, ends: [(f64, f64); 2], half: f32) -> ([i32; 2], Airfield) {
        use naval_earth::geo::to_local;
        let [a, b] = ends.map(|(lat, lon)| to_local(geo, LatLon { lat, lon }));
        let index = chunk_of([(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5]);
        let c = chunk_corner(index);
        let local = |p: [f64; 2]| Vec2::new((p[0] - c[0]) as f32, (p[1] - c[1]) as f32);
        let length = local(a).distance(local(b));
        let field = Airfield {
            runways: vec![Runway {
                points: vec![local(a), local(b)],
                reference: None,
                half,
            }],
            longest_runway: length,
            ..Airfield::default()
        };
        (index, field)
    }

    #[test]
    fn a_past_periods_airfield_is_the_field_then() {
        use infra::period::RunwayPaint;
        // Los Angeles' 07L/25R (3,939 m today), in 1942: on Mines Field's square mile, no longer than its runway then.
        let geo = LatLon {
            lat: 33.94,
            lon: -118.40,
        };
        let lax = [(33.9377, -118.4221), (33.9377, -118.3798)];
        let (index, mut field) = field_about(geo, lax, 45.0);
        // Its mapped area (today's length and width) and an apron, both first on Mines Field's grounds (the east end).
        let (a, b) = (field.runways[0].points[0], field.runways[0].points[1]);
        let side = (b - a).normalize().perp() * 45.0;
        let east = a.lerp(b, 0.8);
        let square = |c: Vec2, r: f32| {
            vec![
                [c.x - r, c.y - r],
                [c.x + r, c.y - r],
                [c.x + r, c.y + r],
                [c.x - r, c.y + r],
            ]
        };
        field.aprons = vec![
            Apron {
                ring: [b - side, b + side, a + side, a - side]
                    .map(|p| p.to_array())
                    .to_vec(),
                runway: true,
            },
            Apron {
                ring: square(east + side * 4.0, 60.0),
                runway: false,
            },
        ];
        let mut then = field.clone();
        period_airfield(&mut then, geo, index, 1942);
        assert_eq!(then.runways.len(), 1);
        assert_eq!(then.aprons.len(), 1);
        assert!(!then.aprons[0].runway);
        let runway = &then.runways[0];
        assert!(
            runway.length() <= 1401.0 && runway.length() > 1000.0,
            "{}",
            runway.length()
        );
        assert!(runway.half <= 23.0);
        for p in &runway.points {
            let at = lat_lon_of(geo, index, p.to_array());
            assert!((-118.3966..=-118.3785).contains(&at.lon), "{at:?}");
        }
        assert_eq!(then.paint, Some(RunwayPaint::Plain));
        assert!(then.longest_runway <= 1400.0);
        // Before Mines Field (1929): no runway at all.
        let mut then = field.clone();
        period_airfield(&mut then, geo, index, 1928);
        assert!(then.runways.is_empty() && then.longest_runway == 0.0);
        // After the war's: numbered.
        let mut then = field;
        period_airfield(&mut then, geo, index, 1955);
        assert_eq!(then.paint, Some(RunwayPaint::Numbered));
        // Keflavík: none before 1943, the war's runway after.
        let geo = LatLon {
            lat: 63.985,
            lon: -22.62,
        };
        let keflavik = [(63.9850, -22.6500), (63.9850, -22.5976)];
        let (index, field) = field_about(geo, keflavik, 30.0);
        let mut then = field.clone();
        period_airfield(&mut then, geo, index, 1941);
        assert!(then.runways.is_empty());
        let mut then = field;
        period_airfield(&mut then, geo, index, 1944);
        assert_eq!(then.runways.len(), 1);
        assert!(
            then.runways[0].length() <= 2151.0,
            "{}",
            then.runways[0].length()
        );
    }

    #[test]
    fn a_past_periods_bridges_are_the_ones_that_stood() {
        use naval_earth::geo::to_local;
        // A bridge 700 m long over 500 m of sea (x 100 to 600 m along it).
        let ground = |p: [f32; 2]| {
            if (100.0..600.0).contains(&p[0]) {
                -10.0
            } else {
                5.0
            }
        };
        let bridges_at = |geo: LatLon, at: (f64, f64), length: f32| {
            let p = to_local(
                geo,
                LatLon {
                    lat: at.0,
                    lon: at.1,
                },
            );
            let index = chunk_of(p);
            let c = chunk_corner(index);
            let (x, z) = ((p[0] - c[0]) as f32, (p[1] - c[1]) as f32);
            let bridges = Bridges {
                lines: vec![BridgeLine {
                    points: vec![
                        Vec2::new(x - length * 0.5, z),
                        Vec2::new(x + length * 0.5, z),
                    ],
                    half: 5.0,
                    rail: false,
                    key: 0,
                }],
                ..Bridges::default()
            };
            (index, bridges)
        };
        // An unlisted long span over the sea (Monterey Bay): not before today's.
        let geo = LatLon {
            lat: 36.6,
            lon: -121.9,
        };
        let (index, mut bridges) = bridges_at(geo, (36.6, -121.9), 700.0);
        let middle = |b: &Bridges| {
            b.lines
                .first()
                .map(|l| (l.points[0].x + l.points[1].x) * 0.5)
        };
        let x = middle(&bridges).unwrap_or(0.0);
        let shifted = |p: [f32; 2]| ground([p[0] - (x - 350.0), p[1]]);
        period_bridges(&mut bridges, geo, index, 1942, &shifted);
        assert!(bridges.lines.is_empty());
        // The Vincent Thomas Bridge (1963) over dry ground as far as the test knows: listed, so not in 1942.
        let geo = LatLon {
            lat: 33.75,
            lon: -118.27,
        };
        let dry = |_: [f32; 2]| 5.0;
        let (index, mut bridges) = bridges_at(geo, (33.7497, -118.2716), 200.0);
        period_bridges(&mut bridges, geo, index, 1942, &dry);
        assert!(bridges.lines.is_empty());
        let (index, mut bridges) = bridges_at(geo, (33.7497, -118.2716), 200.0);
        period_bridges(&mut bridges, geo, index, 1964, &dry);
        assert_eq!(bridges.lines.len(), 1);
        // A short bridge elsewhere stood.
        let (index, mut bridges) = bridges_at(geo, (33.80, -118.20), 200.0);
        period_bridges(&mut bridges, geo, index, 1942, &dry);
        assert_eq!(bridges.lines.len(), 1);
    }
}
