//! The lights on the theatre's land at night (native/docs/terrain/night.md): street lights along
//! OpenStreetMap's paved roads and floods over its industrial and harbour land and its piers, made with each of the
//! features' 2 km pages (`features`, level [`LAMP_LEVEL`]) and drawn by the land (`naval_terrain::night`), which keeps a
//! street light only on built-up land and most floods only near the water. The scenario's rule (`lights`) and the
//! region's palette are [`apply`]'s, each frame. Presentation only, as the rest of the theatre's land.
//!
//! - **Street lights** (`naval_terrain::night::street_light`, by the road's class as its tone on the pages): a
//!   motorway's every 55 m 12 m up to a service road's every 60 m, brighter on the major roads; where a road crosses
//!   the lines x = k s (or z = k s, whichever it runs along more) of the theatre's metres, so the lamps are fixed to
//!   the world, a road cut at a tile's edge is lit alike on either side, and the land's shading lays each one's pool on
//!   the road beneath it; staggered either side of a wide road, on the left of a narrow one.
//! - **Floods** ([`FLOOD_M`]): on a jittered 110 m grid over industrial, railway, port and harbour land (in a past
//!   period only where its ground stood, `Roads::stood`), every 50 m along
//!   a broad pier, each with its pool of light on the ground beneath (the land drops those over the water, but a
//!   pier's); low, dim lamps every 45 m along a jetty or a marina's pontoon.
//! - **Once each**: a tile's features reach past it (its buffer), so a lamp is kept only in the tile proper (half-open
//!   at its edges) and on its page.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use naval_earth::geo::LatLon;
use naval_osm::Geometry;
use naval_terrain::StrokeKind;
use naval_terrain::infra::RunwaySpan;
use naval_terrain::night::{
    BEACON_LAMPS, LAMP_LEVEL, Lamp, LampKind, LampPalette, NightLights, PAGE_LAMPS, road_class,
    runway_lights, street_light,
};

use super::features::period::in_quad;
use super::features::{Roads, TileRoads, page_rect};
use super::osm::OsmTile;

/// The floods' grid over industrial and harbour land (m; a terminal's high masts stand 100-150 m apart), and how far
/// each strays from its node.
pub const FLOOD_M: f64 = 110.0;
const FLOOD_JITTER_M: f64 = 25.0;
/// A broad pier's floods' spacing (m), and a narrow one's lamps' (a jetty's, a pontoon's).
const PIER_M: f64 = 50.0;
const JETTY_M: f64 = 45.0;
/// A flood mast's height (m), a broad pier's lamp's and a jetty's.
const MAST_M: f32 = 24.0;
const PIER_LAMP_M: f32 = 14.0;
const JETTY_LAMP_M: f32 = 4.0;
/// A flood's class (`Lamp::class`): one on a pier, which the land keeps over the water.
const ON_PIER: u8 = 1;
/// A street light's class on a jetty (`road_class`'s spare): kept over the water and off built-up land.
pub const JETTY: u8 = 8;

/// Whether land use `class` is floodlit at night: industry, railway yards, ports and harbours.
pub fn floodlit(class: &str) -> bool {
    matches!(class, "industrial" | "railway" | "port" | "harbour")
}

fn hash(a: i64, b: i64, salt: u32) -> u32 {
    let mut x = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ u64::from(salt).wrapping_mul(0x1656_67B1_9E37_79F9);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 16) as u32
}

/// Even-odd: whether `p` is inside the rings.
fn in_rings(rings: &[&Vec<[f64; 2]>], p: [f64; 2]) -> bool {
    let mut inside = false;
    for ring in rings {
        let n = ring.len();
        for k in 0..n {
            let (a, b) = (ring[k], ring[(k + 1) % n]);
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
                if p[0] < x {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// Calls `f(point, unit direction, k)` where `line` crosses the lines x = k `spacing` (or z = k `spacing`, whichever
/// axis a segment runs along more), each segment half-open so a vertex on a line counts once.
fn lattice(line: &[[f64; 2]], spacing: f64, mut f: impl FnMut([f64; 2], [f64; 2], i64)) {
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let length = d[0].hypot(d[1]);
        if length < 1e-6 {
            continue;
        }
        let axis = usize::from(d[1].abs() > d[0].abs());
        let (p0, p1) = (a[axis], b[axis]);
        let (lo, hi) = (p0.min(p1), p0.max(p1));
        let mut k = (lo / spacing).ceil() as i64;
        while (k as f64) * spacing < hi {
            let t = ((k as f64) * spacing - p0) / (p1 - p0);
            f(
                [a[0] + t * d[0], a[1] + t * d[1]],
                [d[0] / length, d[1] / length],
                k * 2 + axis as i64,
            );
            k += 1;
        }
    }
}

/// The lamps of page `index` (of [`LAMP_LEVEL`]) from `tiles` about `geo`, page-local metres, the brightest first and no
/// more than [`PAGE_LAMPS`]; the street lights along the roads as `roads`' period draws them (none on a street or
/// motorway it leaves out, nor on a road it leaves unpaved), the floods over land only where its ground stood
/// (`Roads::stood`: a yard the period keeps is lit, a terminal it leaves country is dark).
pub fn page_lamps(
    geo: LatLon,
    index: [i64; 2],
    tiles: &[Arc<OsmTile>],
    roads: &Roads<'_>,
) -> Vec<Lamp> {
    let (min, max) = page_rect(LAMP_LEVEL, index);
    let on_page = |p: [f64; 2]| p[0] >= min[0] && p[0] < max[0] && p[1] >= min[1] && p[1] < max[1];
    let local = |p: [f64; 2]| [(p[0] - min[0]) as f32, (p[1] - min[1]) as f32];
    let grow = 20.0;
    let (lo, hi) = (
        [min[0] - grow, min[1] - grow],
        [max[0] + grow, max[1] + grow],
    );
    let mut lamps = Vec::new();
    for tile in tiles {
        let quad = tile.quad(geo);
        let keep = |p: [f64; 2]| on_page(p) && in_quad(&quad, p);
        let ways = TileRoads::new(roads, tile, lo, hi);
        for t in tile.transportation_in(lo, hi) {
            let Geometry::Lines(lines) = &t.geometry else {
                if t.class == "pier"
                    && let Geometry::Polygons(polys) = &t.geometry
                {
                    for poly in polys {
                        floods_over(poly, PIER_M, &keep, &mut |p, seed| {
                            push_flood(&mut lamps, local(p), seed, PIER_LAMP_M, ON_PIER);
                        });
                    }
                }
                continue;
            };
            if t.class == "pier" {
                // A pier mapped as a line is a narrow one: a jetty, a marina's pontoon, a fishing pier, lit by low
                // lamps along it (a broad pier, mapped as an area, is floodlit as a quay).
                for line in lines {
                    lattice(line, JETTY_M, |p, _, k| {
                        if keep(p) {
                            let seed = hash(k, (p[0] + p[1]).round() as i64, 5);
                            lamps.push(Lamp::new(
                                LampKind::Street,
                                JETTY,
                                0.25,
                                seed,
                                local(p),
                                JETTY_LAMP_M,
                            ));
                        }
                    });
                }
                continue;
            }
            let Some(style @ (StrokeKind::Paved, ..)) = roads.style(t) else {
                continue;
            };
            for ((kind, half, tone), run) in ways.lines(t, style, lines) {
                let Some((spacing, height, brightness)) = street_light(tone, half) else {
                    continue;
                };
                if kind != StrokeKind::Paved {
                    continue;
                }
                let class = road_class(if run.trunk { "trunk" } else { &t.class });
                let wide = half >= 4.0;
                let reach = f64::from((half - 1.5).max(0.0));
                lattice(&run.points, spacing, |p, d, k| {
                    // Staggered either side of a wide road, on the left of a narrow one (as the way runs).
                    let side = if wide && k.rem_euclid(4) >= 2 {
                        -1.0
                    } else {
                        1.0
                    };
                    let at = [p[0] - d[1] * reach * side, p[1] + d[0] * reach * side];
                    if keep(p) {
                        let seed = hash(
                            k,
                            (p[0] * 0.5).round() as i64 ^ (p[1] * 0.5).round() as i64,
                            3,
                        );
                        lamps.push(Lamp::new(
                            LampKind::Street,
                            class,
                            brightness,
                            seed,
                            local(at),
                            height,
                        ));
                    }
                });
            }
        }
        for land in tile.landuse_in(lo, hi) {
            if !floodlit(&land.class) {
                continue;
            }
            let Geometry::Polygons(polys) = &land.geometry else {
                continue;
            };
            let keep = |p: [f64; 2]| keep(p) && roads.stood(p);
            for poly in polys {
                floods_over(poly, FLOOD_M, &keep, &mut |p, seed| {
                    push_flood(&mut lamps, local(p), seed, MAST_M, 0)
                });
            }
        }
    }
    lamps.sort_by(|a, b| b.brightness().total_cmp(&a.brightness()));
    lamps.truncate(PAGE_LAMPS);
    lamps
}

/// A flood at `at` (page-local m) `height` up, and its pool beneath (the same seed and class: the land keeps or drops
/// both).
fn push_flood(lamps: &mut Vec<Lamp>, at: [f32; 2], seed: u32, height: f32, class: u8) {
    lamps.push(Lamp::new(LampKind::Flood, class, 1.0, seed, at, height));
    lamps.push(Lamp::new(LampKind::Pool, class, 1.0, seed, at, 0.0));
}

/// Calls `f(point, seed)` at the nodes of a `spacing` grid fixed to the world (each jittered by its own hash) inside
/// `poly` that `keep` keeps.
fn floods_over(
    poly: &naval_osm::Polygon<[f64; 2]>,
    spacing: f64,
    keep: &impl Fn([f64; 2]) -> bool,
    f: &mut impl FnMut([f64; 2], u32),
) {
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in &poly.exterior {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    if lo[0] > hi[0] {
        return;
    }
    let rings: Vec<&Vec<[f64; 2]>> = std::iter::once(&poly.exterior).chain(&poly.holes).collect();
    let jitter = FLOOD_JITTER_M.min(spacing * 0.25);
    for i in
        ((lo[0] - jitter) / spacing).floor() as i64..=((hi[0] + jitter) / spacing).ceil() as i64
    {
        for j in
            ((lo[1] - jitter) / spacing).floor() as i64..=((hi[1] + jitter) / spacing).ceil() as i64
        {
            let seed = hash(i, j, spacing as u32);
            let jx = (f64::from(seed & 0xff) / 255.0 - 0.5) * 2.0 * jitter;
            let jz = (f64::from((seed >> 8) & 0xff) / 255.0 - 0.5) * 2.0 * jitter;
            let p = [i as f64 * spacing + jx, j as f64 * spacing + jz];
            if keep(p) && in_rings(&rings, p) {
                f(p, seed);
            }
        }
    }
}

/// Whether the lights at night are drawn: unless `NAVAL_NIGHT_LIGHTS` is `off` (or `0`), or `set_shown` hid them.
pub fn shown() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_NIGHT_LIGHTS").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    !off && SHOWN.load(Ordering::Relaxed)
}

static SHOWN: AtomicBool = AtomicBool::new(true);

/// Shows or hides the lights at night on screen (for before-and-after captures and frame measurements): hidden, the
/// land and its windows are drawn as with none. Taken up by the next frame.
pub fn set_shown(shown: bool) {
    SHOWN.store(shown, Ordering::Relaxed);
}

/// The palette of the land about `geo`: Japan's white and Taiwan's (its street lights LED; the buildings' Taiwan, with
/// Kinmen and Matsu and without Fujian's coast, `region::taiwanese`), else sodium.
pub fn palette_at(geo: LatLon) -> LampPalette {
    let japan = (24.0..=46.0).contains(&geo.lat) && (122.5..=146.0).contains(&geo.lon);
    if japan || naval_terrain::buildings::region::taiwanese(geo.lat, geo.lon) {
        LampPalette::White
    } else {
        LampPalette::Sodium
    }
}

/// What the land was last handed (`apply`): the scene (the driver's swap count) with the buildings' beacon version,
/// and with the airfields' runways.
#[derive(Debug, Default)]
pub struct Handed {
    beacons: Option<(u64, u64)>,
    runways: Option<(u64, Vec<RunwaySpan>)>,
}

/// Each frame: the land on screen burns its lights by the scenario's rule in the region's palette, its buildings'
/// windows and its boats' cabins by the rule, and the land takes the runways' lights and the buildings' obstruction
/// lights when they change (`handed`; `scene`, the driver's swap count).
pub fn apply(
    land: Option<&mut naval_terrain::TerrainLayer>,
    buildings: Option<&mut naval_terrain::buildings::BuildingsLayer>,
    infra: Option<&mut naval_terrain::infra::InfraLayer>,
    lights: NightLights,
    scene: u64,
    handed: &mut Handed,
) {
    let Some(land) = land else {
        return;
    };
    let shown = shown();
    land.show_night_lights(shown);
    if land.night_lights() != lights {
        land.set_night_lights(lights);
    }
    let windows = if shown {
        lights.rule.scales().windows
    } else {
        1.0
    };
    if let Some(infra) = infra {
        infra.set_windows(windows);
        // Its runways in theatre metres, their lights in the world's.
        let runways = infra.runways();
        if handed
            .runways
            .as_ref()
            .is_none_or(|(s, r)| *s != scene || *r != runways)
        {
            let offset = infra.offset();
            let world: Vec<([f64; 3], u8)> = runways
                .iter()
                .flat_map(|r| runway_lights(&r.moved(offset)))
                .collect();
            land.set_runway_lights(&world);
            handed.runways = Some((scene, runways));
        }
    }
    let Some(buildings) = buildings else {
        return;
    };
    buildings.set_windows(windows);
    let version = buildings.beacons_version();
    if handed.beacons != Some((scene, version)) {
        let mut world = buildings.beacons();
        world.truncate(BEACON_LAMPS);
        land.set_beacons(&world);
        handed.beacons = Some((scene, version));
    }
}

#[cfg(test)]
#[path = "lamps_tests.rs"]
mod tests;
