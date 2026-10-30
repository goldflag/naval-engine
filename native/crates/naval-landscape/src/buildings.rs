//! Buildings over the theatre's land (`native/docs/terrain/buildings.md`; the layer and the meshes are
//! `naval_terrain::buildings`'s, native/docs/terrain/buildings.md). Presentation only, as the rest of the theatre's
//! land.
//!
//! - **Footprints** from Microsoft's Global ML Building Footprints (CDLA Permissive 2.0, [`ATTRIBUTION`]): the dataset's
//!   index ([`INDEX_URLS`], a CSV of `Location,QuadKey,Url,Size,UploadDate`) names, for each level-9 Bing quadkey, one
//!   gzipped file a region (a quadkey on a border has two, `Europe` and `Iceland`), each line a GeoJSON feature: a
//!   polygon in longitude and latitude, `height` (m; -1 where none was estimated) and `confidence` (-1 for older
//!   releases). Only the quadkeys under a chunk with land the camera wants are fetched. Files up to
//!   [`WHOLE_FILE_BYTES`] are fetched whole; larger ones (the densest cities', 100-300 MB) are streamed by a thread of
//!   their own (`theatre-footprints`), inflated a line at a time and split into level-13 tiles ([`split`]), the infill
//!   standing in meanwhile; the chunks made without them are made again once they are in.
//! - **Cache**: the index and every whole file as it came, every big file as its tiles (a `<quadkey>.tiles` folder,
//!   moved into place only once the whole stream was read and checked), under the user data directory's
//!   `buildings-cache` (`NAVAL_BUILDINGS_DIR` moves it), kept for good (the dataset changes yearly); offline, what is
//!   there is drawn, the rest is infill.
//! - **Chunks** of 1 km on the theatre's metres ([`naval_terrain::buildings::CHUNK_M`]): the frame loop's [`Feed`] asks
//!   for those within the layer's range of the camera, whole within its full-detail distance, boxes beyond, the far
//!   level's masses past the boxes (`make_far`, from the window's cover alone), nearest first; one worker thread
//!   (`theatre-buildings`) fetches, parses, projects, plans and meshes them, and keeps the made
//!   meshes ([`MADE_BYTES`]); the feed hands each to the layer, which uploads a few megabytes a frame. No scene rebuild:
//!   the layer is in the scene from its build, empty until its chunks arrive.
//! - **Infill** where the source has none (no file for the quadkey, or a 100 m cell with no footprint) over built-up
//!   cover in the window's real-world maps (`naval_terrain::buildings::infill`).
//! - **Types** from OpenStreetMap (`theatre::osm`, the tiles under the chunks the worker makes and only those, zoom 14
//!   at both near levels, `osm_zoom`): the land use a footprint stands in, the point of interest on it, OSM's
//!   own outlines (their heights where
//!   Microsoft's are missing or capped, their colours, and the buildings Microsoft misses), the water and the piers
//!   ([`tags_for`]), and a few landmarks the tiles cannot carry ([`LANDMARKS`]). A chunk made while some of its tiles
//!   were missing (offline) is drawn as made and made again a minute later.
//!
//! Switched with the surface's streaming (`surface::enabled`); `NAVAL_BUILDINGS=off` leaves it off.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use naval_conditions::SurfaceStyle;
use naval_earth::geo::{LatLon, to_lat_lon, to_local};
use naval_osm::{Geometry, TileId};
use naval_terrain::SurfaceMaps;
use naval_terrain::buildings::mesh::ChunkInput;
use naval_terrain::buildings::tags::{
    self, Landmark, LandmarkKind, MapTags, Outline, Pier, PoiKind, Usage, Zone,
};
use naval_terrain::buildings::{
    BuildingSettings, BuildingsLayer, CHUNK_M, ChunkKey, ChunkMesh, Footprint, Lod, Period, Region,
    build_chunk, chunk_corner, chunk_of, far, geometry, infill, thinning,
};

use super::near::WindowHeights;
use super::osm::{self, OsmTile};
use super::surface::{self, FetchError};

#[path = "buildings_split.rs"]
pub mod split;

#[path = "buildings_period.rs"]
pub mod period;

/// The dataset's index: the README's current one first, then the older host's.
pub const INDEX_URLS: [&str; 2] = [
    "https://bfppub.blob.core.windows.net/%24web/2026-08-13/dataset-links.csv",
    "https://minedbuildings.z5.web.core.windows.net/global-buildings/dataset-links.csv",
];
/// What the game must show where it credits its sources (the data's licence, CDLA Permissive 2.0, asks no more of a
/// picture made from it; the text is kept for the credits).
pub const ATTRIBUTION: &str = "Building footprints: Microsoft Global ML Building Footprints \
     (https://github.com/microsoft/GlobalMLBuildingFootprints), CDLA Permissive 2.0";
/// The index's quadkeys' level.
pub const QUADKEY_LEVEL: u32 = 9;
/// Files up to this size (compressed) are fetched whole and parsed in memory; larger ones (the densest cities') are
/// streamed and split into tiles ([`split`]).
pub const WHOLE_FILE_BYTES: u64 = 80 << 20;
/// Chunks the worker makes together.
pub const BATCH: usize = 16;
/// Far chunks made at once (`make_far`: no files, a few milliseconds each).
pub const FAR_BATCH: usize = 64;
/// Made meshes kept in memory.
pub const MADE_BYTES: usize = 320 << 20;
/// Quadkeys' (and big files' tiles') footprints kept in memory, parsed and projected: at most this many stores...
pub const QUADS_KEPT: usize = 64;
/// ...and, beyond the newest, this many footprints.
pub const FOOTPRINTS_KEPT: usize = 3_000_000;
/// A big file whose streaming failed is not asked for again for this long.
const SPLIT_RETRY_AFTER: Duration = Duration::from_secs(300);
/// A chunk whose making failed is not asked for again for this long.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long the feed waits before asking the worker again for the same chunks.
const ASK_AGAIN: Duration = Duration::from_secs(5);
const USER_AGENT: &str = concat!(
    "naval-native/",
    env!("CARGO_PKG_VERSION"),
    " (theatre buildings; file cache on disk)"
);

/// Landmarks OpenMapTiles cannot carry (it has no `man_made`: no lighthouses, chimneys or cranes), as (latitude,
/// longitude, what stands there): the lighthouses and the power station's stacks of the theatres' coasts the game
/// ships or was checked on (Reykjavík; Monterey Bay, California). Places from OpenStreetMap (ODbL, credited with the
/// tiles) and Wikipedia; heights from the lights' lists. Anything else stands where the map's points name it.
pub const LANDMARKS: &[(f64, f64, LandmarkKind)] = &[
    // Moss Landing Power Plant: units 6 and 7's two 150 m stacks, the combined-cycle plant's four 30 m stacks.
    (
        36.804951,
        -121.781923,
        LandmarkKind::Chimney {
            height: 150.0,
            radius: 7.5,
        },
    ),
    (
        36.804567,
        -121.781954,
        LandmarkKind::Chimney {
            height: 150.0,
            radius: 7.5,
        },
    ),
    (
        36.804183,
        -121.777898,
        LandmarkKind::Chimney {
            height: 30.0,
            radius: 2.6,
        },
    ),
    (
        36.804353,
        -121.777828,
        LandmarkKind::Chimney {
            height: 30.0,
            radius: 2.6,
        },
    ),
    (
        36.804225,
        -121.777303,
        LandmarkKind::Chimney {
            height: 30.0,
            radius: 2.6,
        },
    ),
    (
        36.804059,
        -121.777368,
        LandmarkKind::Chimney {
            height: 30.0,
            radius: 2.6,
        },
    ),
    // Point Pinos: a 13 m tower rising from its keeper's house, white under a black lantern.
    (
        36.633389,
        -121.933737,
        LandmarkKind::Lighthouse {
            height: 15.0,
            bands: 0,
            red: false,
            keeper: true,
        },
    ),
    // Pigeon Point: a 35 m white tower.
    (
        37.181750,
        -122.393917,
        LandmarkKind::Lighthouse {
            height: 36.0,
            bands: 0,
            red: false,
            keeper: false,
        },
    ),
    // Santa Cruz: the Mark Abbott Memorial Lighthouse (12 m), and the west breakwater's light (Walton).
    (
        36.951486,
        -122.026736,
        LandmarkKind::Lighthouse {
            height: 12.0,
            bands: 0,
            red: false,
            keeper: false,
        },
    ),
    (
        36.960703,
        -122.002185,
        LandmarkKind::Lighthouse {
            height: 11.0,
            bands: 0,
            red: false,
            keeper: false,
        },
    ),
    // Point Sur.
    (
        36.306500,
        -121.901600,
        LandmarkKind::Lighthouse {
            height: 15.0,
            bands: 0,
            red: false,
            keeper: true,
        },
    ),
    // Reykjavík: Gróttuviti (24 m), Engeyjarviti, the harbour mouth's green and red lights, two more.
    (
        64.164999,
        -22.021954,
        LandmarkKind::Lighthouse {
            height: 24.0,
            bands: 0,
            red: true,
            keeper: false,
        },
    ),
    (
        64.174973,
        -21.924493,
        LandmarkKind::Lighthouse {
            height: 11.0,
            bands: 0,
            red: true,
            keeper: false,
        },
    ),
    (
        64.152861,
        -21.932094,
        LandmarkKind::Lighthouse {
            height: 7.0,
            bands: 0,
            red: false,
            keeper: false,
        },
    ),
    (
        64.151891,
        -21.931735,
        LandmarkKind::Lighthouse {
            height: 7.0,
            bands: 3,
            red: true,
            keeper: false,
        },
    ),
    (
        64.147459,
        -21.905554,
        LandmarkKind::Lighthouse {
            height: 9.0,
            bands: 0,
            red: true,
            keeper: false,
        },
    ),
    (
        64.116356,
        -22.007432,
        LandmarkKind::Lighthouse {
            height: 9.0,
            bands: 0,
            red: true,
            keeper: false,
        },
    ),
];

/// Whether buildings are drawn: with the surface's streaming on, unless `NAVAL_BUILDINGS` is `off` (or `0`).
/// `NAVAL_BUILDINGS=plain` draws them without the facade textures and roof clutter, as Medium does (to measure them).
pub fn enabled() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    let off = *OFF.get_or_init(|| {
        std::env::var("NAVAL_BUILDINGS").is_ok_and(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "off" | "0" | "false"
            )
        })
    });
    surface::enabled() && !off
}

/// Whether `NAVAL_BUILDINGS` is `plain`: no facade textures or roof clutter on any setting.
fn plain() -> bool {
    static PLAIN: OnceLock<bool> = OnceLock::new();
    *PLAIN.get_or_init(|| {
        std::env::var("NAVAL_BUILDINGS").is_ok_and(|v| v.trim().eq_ignore_ascii_case("plain"))
    })
}

/// The far level's range from `NAVAL_BUILDINGS_FAR` (m): `off` (or `0`) none, a number its kilometres, at most High's
/// 30 ([`MOST_FAR_M`]); unset, the setting's own.
fn far_override() -> Option<f32> {
    static FAR: OnceLock<Option<f32>> = OnceLock::new();
    *FAR.get_or_init(|| {
        let v = std::env::var("NAVAL_BUILDINGS_FAR").ok()?;
        far_range(&v)
    })
}

/// The farthest the far level reaches (m): its chunks (about 2,900 within it) and those fading fit the layer's
/// `CHUNK_CAPACITY` slots.
pub const MOST_FAR_M: f32 = 30_000.0;

/// A far range (m) from `NAVAL_BUILDINGS_FAR`'s value (see [`far_override`]).
fn far_range(v: &str) -> Option<f32> {
    let v = v.trim().to_ascii_lowercase();
    if matches!(v.as_str(), "off" | "false") {
        return Some(0.0);
    }
    v.parse::<f32>()
        .ok()
        .filter(|km| km.is_finite())
        .map(|km| (km.max(0.0) * 1000.0).min(MOST_FAR_M))
}

/// The settings for the game's terrain `quality` (and `NAVAL_BUILDINGS=plain`, `NAVAL_BUILDINGS_FAR`), drawn in
/// `period`.
fn settings_for(quality: &naval_terrain::TerrainQuality, period: Period) -> BuildingSettings {
    let mut settings = BuildingSettings::for_quality(quality).with_period(period);
    if plain() {
        settings.facades = false;
        settings.clutter = false;
    }
    if let Some(far_m) = far_override() {
        settings.far_m = if far_m > settings.range_m { far_m } else { 0.0 };
    }
    settings
}

/// Where the files are kept: `NAVAL_BUILDINGS_DIR` when set, else the user data directory's `buildings-cache`.
pub fn cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("NAVAL_BUILDINGS_DIR").filter(|v| !v.is_empty()) {
        return dir.into();
    }
    crate::host::data_dir().join("buildings-cache")
}

fn log_once(kind: &str, line: &str) {
    static LOGGED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut logged = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if logged
        .get_or_insert_with(HashSet::new)
        .insert(kind.to_owned())
    {
        eprintln!("the theatre's buildings: {line}");
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Quadkeys.

/// The Bing quadkey of `level` holding a latitude and longitude.
pub fn quadkey(lat: f64, lon: f64, level: u32) -> String {
    let n = 1i64 << level;
    let lat = lat.clamp(-85.051_128, 85.051_128);
    let x = ((lon + 180.0) / 360.0 * n as f64).floor() as i64;
    let s = lat.to_radians().sin();
    let y = ((0.5 - ((1.0 + s) / (1.0 - s)).ln() / (4.0 * std::f64::consts::PI)) * n as f64).floor()
        as i64;
    let (x, y) = (x.clamp(0, n - 1), y.clamp(0, n - 1));
    (1..=level)
        .rev()
        .map(|i| {
            let mask = 1i64 << (i - 1);
            let digit = u8::from(x & mask != 0) + 2 * u8::from(y & mask != 0);
            char::from(b'0' + digit)
        })
        .collect()
}

/// A quadkey's tile: south, west, north and east edges in degrees; none for a key that is not one.
pub fn quadkey_bounds(key: &str) -> Option<[f64; 4]> {
    let (mut x, mut y) = (0i64, 0i64);
    for c in key.chars() {
        let d = c.to_digit(4)? as i64;
        x = x * 2 + (d & 1);
        y = y * 2 + (d >> 1);
    }
    let n = (1i64 << key.len()) as f64;
    let lon = |x: f64| x / n * 360.0 - 180.0;
    let lat = |y: f64| {
        let m = std::f64::consts::PI * (1.0 - 2.0 * y / n);
        m.sinh().atan().to_degrees()
    };
    Some([
        lat((y + 1) as f64),
        lon(x as f64),
        lat(y as f64),
        lon((x + 1) as f64),
    ])
}

// ---------------------------------------------------------------------------------------------------------------------
// The index.

/// One file of the dataset.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexFile {
    pub location: String,
    pub url: String,
    pub bytes: u64,
}

/// The dataset's files by quadkey.
#[derive(Debug, Default)]
pub struct Index {
    pub files: HashMap<String, Vec<IndexFile>>,
}

/// "4.5MB", "697B", "74.7KB" in bytes.
fn parse_size(s: &str) -> u64 {
    let s = s.trim();
    let (number, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let v: f64 = number.trim().parse().unwrap_or(0.0);
    let k = match unit.trim().to_ascii_uppercase().as_str() {
        "KB" => 1024.0,
        "MB" => 1024.0 * 1024.0,
        "GB" => 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (v * k) as u64
}

/// The index from its CSV.
pub fn parse_index(text: &str) -> Index {
    let mut index = Index::default();
    for line in text.lines().skip(1) {
        let mut cells = line.split(',');
        let (Some(location), Some(key), Some(url), Some(size)) =
            (cells.next(), cells.next(), cells.next(), cells.next())
        else {
            continue;
        };
        if key.is_empty() || !url.starts_with("http") {
            continue;
        }
        index
            .files
            .entry(key.trim().to_owned())
            .or_default()
            .push(IndexFile {
                location: location.trim().to_owned(),
                url: url.trim().to_owned(),
                bytes: parse_size(size),
            });
    }
    index
}

// ---------------------------------------------------------------------------------------------------------------------
// Footprints.

/// A footprint as the source gives it: its outer ring (longitude, latitude) and its height when estimated.
#[derive(Clone, Debug, PartialEq)]
pub struct RawFootprint {
    pub ring: Vec<[f64; 2]>,
    pub height: Option<f32>,
}

/// The number at the start of `s` (after spaces), and the rest.
fn number(s: &str) -> Option<(f64, &str)> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')))
        .unwrap_or(s.len());
    Some((s[..end].parse().ok()?, &s[end..]))
}

/// One line of a file: a GeoJSON feature with a polygon (or a multipolygon, whose first polygon is taken) and its
/// `height`. Its holes are left out (a courtyard is roofed over).
pub fn parse_line(line: &str) -> Option<RawFootprint> {
    let height = line
        .find("\"height\"")
        .and_then(|at| line[at + 8..].trim_start().strip_prefix(':'))
        .and_then(|rest| number(rest).map(|(v, _)| v as f32))
        .filter(|h| *h > 0.0);
    let at = line.find("\"coordinates\"")?;
    let mut rest = line[at + 13..].trim_start().strip_prefix(':')?;
    // Down the brackets to the first pair.
    let mut depth = 0;
    loop {
        rest = rest.trim_start();
        match rest.strip_prefix('[') {
            Some(r) => {
                rest = r;
                depth += 1;
            }
            None => break,
        }
    }
    if depth < 3 {
        return None;
    }
    let mut ring = Vec::with_capacity(8);
    loop {
        let (lon, r) = number(rest)?;
        let r = r.trim_start().strip_prefix(',')?;
        let (lat, r) = number(r)?;
        ring.push([lon, lat]);
        // A third coordinate, if any, then the pair's end.
        let mut r = r.trim_start();
        if let Some(more) = r.strip_prefix(',') {
            let (_, after) = number(more)?;
            r = after.trim_start();
        }
        r = r.strip_prefix(']')?.trim_start();
        match r.strip_prefix(',') {
            Some(next) => rest = next.trim_start().strip_prefix('[')?,
            None => break,
        }
    }
    (ring.len() >= 3).then_some(RawFootprint { ring, height })
}

/// Each line of a gzip stream (one member or several), inflated as it goes.
pub fn gunzip_lines(bytes: &[u8], mut each: impl FnMut(&str)) -> Result<(), String> {
    use miniz_oxide::inflate::stream::{InflateState, inflate};
    use miniz_oxide::{DataFormat, MZFlush, MZStatus};
    let mut at = 0usize;
    let mut line: Vec<u8> = Vec::with_capacity(4096);
    let mut out = vec![0u8; 1 << 16];
    while at + 10 <= bytes.len() {
        let header = &bytes[at..];
        if header[0] != 0x1f || header[1] != 0x8b || header[2] != 8 {
            if at == 0 {
                return Err("not a gzip stream".into());
            }
            break;
        }
        let flags = header[3];
        let mut p = 10usize;
        if flags & 4 != 0 {
            let extra = usize::from(*header.get(p).ok_or("short header")?)
                | usize::from(*header.get(p + 1).ok_or("short header")?) << 8;
            p += 2 + extra;
        }
        for bit in [8u8, 16] {
            if flags & bit != 0 {
                while *header.get(p).ok_or("short header")? != 0 {
                    p += 1;
                }
                p += 1;
            }
        }
        if flags & 2 != 0 {
            p += 2;
        }
        let mut state = InflateState::new_boxed(DataFormat::Raw);
        let mut input = header.get(p..).ok_or("short header")?;
        let mut consumed = p;
        loop {
            let result = inflate(&mut state, input, &mut out, MZFlush::None);
            input = &input[result.bytes_consumed..];
            consumed += result.bytes_consumed;
            for &b in &out[..result.bytes_written] {
                if b == b'\n' {
                    if let Ok(s) = std::str::from_utf8(&line) {
                        each(s);
                    }
                    line.clear();
                } else {
                    line.push(b);
                }
            }
            match result.status {
                Ok(MZStatus::StreamEnd) => break,
                Ok(_) => {
                    if result.bytes_consumed == 0 && result.bytes_written == 0 {
                        return Err("the stream ended early".into());
                    }
                }
                Err(e) => return Err(format!("inflating: {e:?}")),
            }
        }
        // The member's trailer (CRC and size), then perhaps another member.
        at += consumed + 8;
    }
    if !line.is_empty()
        && let Ok(s) = std::str::from_utf8(&line)
    {
        each(s);
    }
    Ok(())
}

/// A raw footprint in the theatre about `geo`: its chunk and its ring from the chunk's corner.
pub fn project(raw: &RawFootprint, geo: LatLon) -> Option<([i32; 2], Footprint)> {
    let points: Vec<[f64; 2]> = raw
        .ring
        .iter()
        .map(|p| {
            to_local(
                geo,
                LatLon {
                    lat: p[1],
                    lon: p[0],
                },
            )
        })
        .collect();
    let n = points.len() as f64;
    let mean = points
        .iter()
        .fold([0.0, 0.0], |s, p| [s[0] + p[0] / n, s[1] + p[1] / n]);
    let chunk = chunk_of(mean);
    let corner = chunk_corner(chunk);
    let ring = points
        .iter()
        .map(|p| [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32])
        .collect();
    Some((
        chunk,
        Footprint {
            ring,
            height: raw.height,
            ..Footprint::default()
        },
    ))
}

/// One quadkey's footprints in the theatre about a scenario's origin, by chunk.
#[derive(Debug, Default)]
pub struct QuadStore {
    pub chunks: HashMap<[i32; 2], Vec<Footprint>>,
    pub footprints: usize,
    /// Whether the source has the quadkey at all (else the infill stands in everywhere in it).
    pub covered: bool,
}

impl QuadStore {
    /// Adds a file's footprints.
    pub fn add_file(&mut self, gz: &[u8], geo: LatLon) -> Result<(), String> {
        let mut bad = 0usize;
        gunzip_lines(gz, |line| {
            if line.trim().is_empty() {
                return;
            }
            match parse_line(line).and_then(|raw| project(&raw, geo)) {
                Some((chunk, footprint)) => {
                    self.chunks.entry(chunk).or_default().push(footprint);
                    self.footprints += 1;
                }
                None => bad += 1,
            }
        })?;
        if bad > 0 {
            log_once(
                "bad-lines",
                &format!("{bad} lines of a footprint file were not read"),
            );
        }
        self.covered = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The ground and the cover.

/// The heights the buildings stand on: a cruise scene's window (its chart's (0, 0) at theatre point `origin`; the
/// host's field, as the near tiles hold it), or a field in a battle's frame (the tests').
#[derive(Clone)]
pub enum Ground {
    Window {
        field: Arc<dyn WindowHeights>,
        origin: [f64; 2],
    },
    Battle {
        field: Arc<naval_terrain::Heightfield>,
        origin: [f64; 2],
    },
}

impl Ground {
    pub fn origin(&self) -> [f64; 2] {
        match self {
            Self::Window { origin, .. } | Self::Battle { origin, .. } => *origin,
        }
    }

    /// The ground at theatre point `p`, metres above the sea.
    pub fn height(&self, p: [f64; 2]) -> f64 {
        let o = self.origin();
        match self {
            Self::Window { field, .. } => field.height(p[0] - o[0], p[1] - o[1]),
            Self::Battle { field, .. } => field.height(p[0] - o[0], p[1] - o[1]),
        }
    }

    /// The heights' sample spacing in decimetres, for the made meshes' keys.
    pub(crate) fn cell_dm(&self) -> u32 {
        let cell = match self {
            Self::Window { field, .. } => field.heights().cell,
            Self::Battle { field, .. } => f64::from(field.cell),
        };
        (cell * 10.0).round() as u32
    }

    /// Whether chunk `index` holds land (any of 7 × 7 points over it more than 0.3 m up).
    fn has_land(&self, index: [i32; 2]) -> bool {
        let c = chunk_corner(index);
        (0..7).any(|j| {
            (0..7).any(|i| {
                self.height([
                    c[0] + f64::from(i) * CHUNK_M / 6.0,
                    c[1] + f64::from(j) * CHUNK_M / 6.0,
                ]) > 0.3
            })
        })
    }
}

/// The window's real-world maps, their chart's (0, 0) at theatre point `origin`.
#[derive(Clone)]
pub struct Cover {
    pub maps: Arc<SurfaceMaps>,
    pub origin: [f64; 2],
}

impl Cover {
    fn class_at(&self, p: [f64; 2]) -> u8 {
        let m = &self.maps;
        let texel = m.extent_m / f64::from(m.size);
        let i = ((p[0] - self.origin[0] - m.origin[0]) / texel).floor();
        let j = ((p[1] - self.origin[1] - m.origin[1]) / texel).floor();
        let n = f64::from(m.size);
        if !((0.0..n).contains(&i) && (0.0..n).contains(&j)) {
            return 0;
        }
        // Today's class: the buildings' own period decides what stands, not the ground's mark of it.
        naval_terrain::surface_maps::cover::today(
            m.cover[j as usize * m.size as usize + i as usize],
        )
    }

    /// Whether the maps cover all of chunk `index`.
    fn covers(&self, index: [i32; 2]) -> bool {
        let m = &self.maps;
        let c = chunk_corner(index);
        let lo = [self.origin[0] + m.origin[0], self.origin[1] + m.origin[1]];
        (0..2).all(|a| c[a] >= lo[a] && c[a] + CHUNK_M <= lo[a] + m.extent_m)
    }
}

struct Land<'a> {
    ground: &'a Ground,
    cover: Option<&'a Cover>,
}

impl infill::Land for Land<'_> {
    fn cover(&self, p: [f64; 2]) -> u8 {
        self.cover.map_or(0, |c| c.class_at(p))
    }

    fn built_share(&self, p: [f64; 2], radius: f64) -> f32 {
        let Some(cover) = self.cover else {
            return 0.0;
        };
        let texel = cover.maps.extent_m / f64::from(cover.maps.size);
        let steps = (radius / texel).ceil().clamp(1.0, 4.0) as i32;
        let step = radius / f64::from(steps);
        let (mut built, mut all) = (0u32, 0u32);
        for j in -steps..=steps {
            for i in -steps..=steps {
                let class =
                    cover.class_at([p[0] + f64::from(i) * step, p[1] + f64::from(j) * step]);
                if class != 0 {
                    all += 1;
                    built += u32::from(class == infill::BUILT_UP);
                }
            }
        }
        if all == 0 {
            0.0
        } else {
            built as f32 / all as f32
        }
    }

    fn height(&self, p: [f64; 2]) -> f32 {
        self.ground.height(p) as f32
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The worker.

/// What the chunks are made over: the scenario's frame, the ground, the cover (with the window's maps), the land's
/// style (each chunk's palettes are its place's, `Region::at`; `region` the frame's origin's, for the keys) and the
/// age the buildings are drawn in (the layer's, Graphics → Buildings and the scenario's year).
#[derive(Clone)]
pub struct Context {
    pub geo: LatLon,
    pub ground: Ground,
    pub cover: Option<Cover>,
    pub region: Region,
    pub style: SurfaceStyle,
    pub period: Period,
    /// The land's fine relief as drawn (the land layer's pages, `TerrainLayer::relief_heights`), its chart's (0, 0) at
    /// the ground's origin: whole and boxed chunks stand on it, and are made again when the pages under them change.
    /// None without it (Medium, a battle's map, a test).
    pub relief: Option<naval_terrain::ReliefHeights>,
}

impl Context {
    /// The ground as the land draws it at theatre point `p`: the window's heights with the fine relief where drawn.
    pub fn footing(&self, p: [f64; 2]) -> f64 {
        let coarse = self.ground.height(p);
        match &self.relief {
            Some(relief) => {
                let o = self.ground.origin();
                relief.height([p[0] - o[0], p[1] - o[1]], coarse)
            }
            None => coarse,
        }
    }

    /// The relief's pages under chunk `chunk`, by [`naval_terrain::ReliefHeights::signature`] (0 for a far chunk, which
    /// stands on the window's heights).
    fn relief_under(&self, chunk: ChunkKey) -> u64 {
        let Some(relief) = self.relief.as_ref().filter(|_| chunk.lod != Lod::Far) else {
            return 0;
        };
        let c = chunk_corner(chunk.index);
        let o = self.ground.origin();
        let lo = [c[0] - o[0], c[1] - o[1]];
        relief.signature(Some((lo, [lo[0] + CHUNK_M, lo[1] + CHUNK_M])))
    }

    /// The palettes of chunk `index`: its middle's place's.
    pub fn region_of(&self, index: [i32; 2]) -> Region {
        let c = chunk_corner(index);
        self.region_at([c[0] + CHUNK_M * 0.5, c[1] + CHUNK_M * 0.5])
    }

    /// The region at theatre point `p` (x, z m).
    pub fn region_at(&self, p: [f64; 2]) -> Region {
        let p = to_lat_lon(self.geo, p);
        Region::at(p.lat, p.lon, &self.style)
    }
}

/// An area the host draws itself (a landmark of its own), in latitude and longitude: every building whose middle lies
/// in one is left out at every level and from every source (Microsoft's footprints, OSM's outlines and parts, the
/// infill's lots, a block's parcels, the far level's masses, the landmarks, lighthouses, cranes and piers' decks), and
/// the infill leaves it empty ([`set_clear_areas`]).
#[derive(Clone, Debug, PartialEq)]
pub enum ClearArea {
    /// A ring of points, either way round, closed or not.
    Polygon(Vec<LatLon>),
    /// A circle about `middle`, `radius_m` metres across the ground.
    Circle { middle: LatLon, radius_m: f64 },
}

/// The sides of a circle's ring ([`ClearArea::Circle`]): its edge within 0.2% of the radius.
const CIRCLE_SIDES: usize = 48;

impl ClearArea {
    /// Its ring in theatre metres about `geo` (x east, z south).
    fn ring(&self, geo: LatLon) -> Vec<[f64; 2]> {
        match self {
            Self::Polygon(points) => points.iter().map(|p| to_local(geo, *p)).collect(),
            Self::Circle { middle, radius_m } => {
                let c = to_local(geo, *middle);
                (0..CIRCLE_SIDES)
                    .map(|k| {
                        let a = std::f64::consts::TAU * k as f64 / CIRCLE_SIDES as f64;
                        [c[0] + radius_m * a.cos(), c[1] + radius_m * a.sin()]
                    })
                    .collect()
            }
        }
    }

    /// A hash of the area, for the chunks under it to know it changed.
    fn signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut point = |p: &LatLon| {
            p.lat.to_bits().hash(&mut h);
            p.lon.to_bits().hash(&mut h);
        };
        match self {
            Self::Polygon(points) => points.iter().for_each(&mut point),
            Self::Circle { middle, radius_m } => {
                point(middle);
                radius_m.to_bits().hash(&mut h);
            }
        }
        h.finish()
    }
}

/// The areas handed ([`set_clear_areas`]), each with its ring and bounds in a scenario's frame as asked for.
#[derive(Default)]
struct Clearing {
    areas: Vec<(ClearArea, u64)>,
    /// By the frame's origin (its bits): each area's ring in theatre metres and its bounds (least x, z, most x, z).
    framed: Mutex<HashMap<[u64; 2], Arc<Vec<Framed>>>>,
}

struct Framed {
    ring: Vec<[f64; 2]>,
    bounds: [f64; 4],
    signature: u64,
}

impl Clearing {
    fn framed(&self, geo: LatLon) -> Arc<Vec<Framed>> {
        let mut framed = self.framed.lock().unwrap_or_else(|e| e.into_inner());
        framed
            .entry([geo.lat.to_bits(), geo.lon.to_bits()])
            .or_insert_with(|| {
                Arc::new(
                    self.areas
                        .iter()
                        .map(|(area, signature)| {
                            let ring = area.ring(geo);
                            let bounds = ring.iter().fold(
                                [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
                                |b, p| {
                                    [
                                        b[0].min(p[0]),
                                        b[1].min(p[1]),
                                        b[2].max(p[0]),
                                        b[3].max(p[1]),
                                    ]
                                },
                            );
                            Framed {
                                ring,
                                bounds,
                                signature: *signature,
                            }
                        })
                        .collect(),
                )
            })
            .clone()
    }

    /// The areas reaching into chunk `index` of `geo`'s frame.
    fn under(&self, geo: LatLon, index: [i32; 2]) -> Vec<Framed> {
        if self.areas.is_empty() {
            return Vec::new();
        }
        let c = chunk_corner(index);
        self.framed(geo)
            .iter()
            .filter(|a| {
                a.bounds[0] <= c[0] + CHUNK_M
                    && a.bounds[2] >= c[0]
                    && a.bounds[1] <= c[1] + CHUNK_M
                    && a.bounds[3] >= c[1]
            })
            .map(|a| Framed {
                ring: a.ring.clone(),
                bounds: a.bounds,
                signature: a.signature,
            })
            .collect()
    }

    /// The areas over chunk `index` as a hash (0 for none): a chunk made under others is made again.
    fn signature(&self, geo: LatLon, index: [i32; 2]) -> u64 {
        if self.areas.is_empty() {
            return 0;
        }
        let c = chunk_corner(index);
        self.framed(geo)
            .iter()
            .filter(|a| {
                a.bounds[0] <= c[0] + CHUNK_M
                    && a.bounds[2] >= c[0]
                    && a.bounds[1] <= c[1] + CHUNK_M
                    && a.bounds[3] >= c[1]
            })
            .fold(0u64, |h, a| h.rotate_left(5) ^ a.signature)
    }
}

static CLEARING: Mutex<Option<Arc<Clearing>>> = Mutex::new(None);

fn clearing() -> Arc<Clearing> {
    CLEARING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Default::default)
        .clone()
}

/// Hands the buildings the areas the host draws itself ([`ClearArea`]), in place of any handed before (none to clear
/// them): every building whose middle lies in one is left out, and the chunks under an area added, moved or taken away
/// are made again (the old mesh standing until the new one is in). Any thread, any time; a scenario's frame need not be
/// known yet.
pub fn set_clear_areas(areas: Vec<ClearArea>) {
    let areas = areas
        .into_iter()
        .map(|a| {
            let signature = a.signature();
            (a, signature)
        })
        .collect();
    *CLEARING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Clearing {
        areas,
        framed: Mutex::new(HashMap::new()),
    }));
    // The feed asks again (`want`), and the chunks under a change are stale.
    streamer().wake.notify_one();
}

/// The areas handed last ([`set_clear_areas`]).
pub fn clear_areas() -> Vec<ClearArea> {
    clearing().areas.iter().map(|(a, _)| a.clone()).collect()
}

/// The areas reaching into chunk `index` of `geo`'s frame, as chunk-local rings (`MapTags::cleared`).
fn cleared_rings(clearing: &Clearing, geo: LatLon, index: [i32; 2]) -> Vec<Vec<[f32; 2]>> {
    let c = chunk_corner(index);
    clearing
        .under(geo, index)
        .into_iter()
        .map(|a| {
            a.ring
                .iter()
                .map(|p| [(p[0] - c[0]) as f32, (p[1] - c[1]) as f32])
                .collect()
        })
        .collect()
}

/// A made mesh's key: the scenario's frame, the heights' spacing, whether the cover was had, the palettes, the period
/// ([`Period::key`]), the chunk.
type MadeKey = ([u64; 2], u32, bool, u8, u16, ChunkKey);

/// A made mesh, whether everything it wanted was had (every OpenStreetMap tile, every big file's tiles), when it was
/// made, whether a big file under it was still being split, and the splits finished before it was begun.
struct Made {
    mesh: Arc<ChunkMesh>,
    complete: bool,
    at: Instant,
    awaiting: bool,
    epoch: u64,
    /// A far chunk's: the period ground's version it was made after (`period_land::version`); 0 otherwise.
    answered: u64,
    /// The relief's pages it was made on ([`Context::relief_under`]).
    relief: u64,
    /// The areas the host draws itself over it when it was begun ([`Clearing::signature`]).
    cleared: u64,
}

impl Made {
    /// Whether to make it again: made without something it wanted, and a big file has been split since or it was made a
    /// while ago (not while the split it waits for runs).
    fn stale(&self) -> bool {
        !self.complete
            && (self.epoch != split_epoch()
                || (self.at.elapsed() >= RETRY_AFTER && !(self.awaiting && splitting())))
    }

    /// The same, or, for far chunk `chunk` in a past period, its ground answered anew since it was made (the buildings'
    /// thinning about the camera, a window's estimate): its masses stand on the ground that stood.
    fn stale_for(&self, context: &Context, chunk: ChunkKey) -> bool {
        self.stale()
            || self.evicted().stale_for(context, chunk)
            || (chunk.lod == Lod::Far
                && context.period.is_past()
                && super::period_land::changed_since(
                    context.geo,
                    context.period,
                    self.answered,
                    [
                        chunk.index[0],
                        chunk.index[1],
                        chunk.index[0],
                        chunk.index[1],
                    ],
                    true,
                ))
    }

    /// What it was made on, kept past its eviction from the cache ([`State::evicted`]).
    fn evicted(&self) -> Evicted {
        Evicted {
            relief: self.relief,
            cleared: self.cleared,
        }
    }
}

/// What a mesh evicted from the cache was made on: the relief's pages ([`Made::relief`]) and the areas the host draws
/// itself over it ([`Made::cleared`]).
#[derive(Clone, Copy)]
struct Evicted {
    relief: u64,
    cleared: u64,
}

impl Evicted {
    /// Whether chunk `chunk`, still drawn, is to be made again: the relief under it, or the areas over it, changed
    /// since it was made.
    fn stale_for(&self, context: &Context, chunk: ChunkKey) -> bool {
        self.relief != context.relief_under(chunk)
            || self.cleared != clearing().signature(context.geo, chunk.index)
    }
}

impl Context {
    fn made_key(&self, chunk: ChunkKey) -> MadeKey {
        (
            [self.geo.lat.to_bits(), self.geo.lon.to_bits()],
            self.ground.cell_dm(),
            self.cover.is_some(),
            self.region.index(),
            self.period.key(),
            chunk,
        )
    }

    fn same(&self, other: &Context) -> bool {
        self.made_key(ChunkKey {
            index: [0, 0],
            lod: Lod::Full,
        }) == other.made_key(ChunkKey {
            index: [0, 0],
            lod: Lod::Full,
        }) && self.ground.origin() == other.ground.origin()
    }
}

/// What the worker is doing, for the RPC.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct WorkerStatus {
    /// Chunks asked for and not begun, and those of the batch in hand.
    pub queued: usize,
    pub working: usize,
    /// Files fetched and to fetch for the batch.
    pub fetched: usize,
    pub fetching: usize,
    /// Chunks that failed in the last minute.
    pub failed: usize,
    /// Big files queued or being streamed and split, and the megabytes of the one in hand read so far.
    pub streaming: usize,
    pub streamed_mb: u64,
    /// Quadkeys held parsed, and their footprints.
    pub quadkeys: usize,
    pub footprints: usize,
    /// Meshes made and kept, and their bytes.
    pub made: usize,
    pub made_bytes: usize,
}

struct State {
    context: Option<Context>,
    wanted: VecDeque<ChunkKey>,
    made: HashMap<MadeKey, Made>,
    order: VecDeque<MadeKey>,
    made_bytes: usize,
    failed: HashMap<MadeKey, Instant>,
    /// Chunks made without some of their tiles, when, and the splits finished before: kept past their meshes'
    /// eviction, so a drawn chunk whose mesh left the cache is made again only when it lacked something.
    incomplete: HashMap<MadeKey, (Instant, u64)>,
    /// What each mesh evicted from the cache was made on ([`Evicted`]): a drawn chunk whose mesh left the cache is made
    /// again when the relief under it or the areas over it change.
    evicted: HashMap<MadeKey, Evicted>,
    quads: HashMap<([u64; 2], String), Arc<QuadStore>>,
    quad_order: VecDeque<([u64; 2], String)>,
    index: Option<Arc<Index>>,
    status: WorkerStatus,
    worker: bool,
    /// Big files to stream and split (with their quadkeys), the one in hand (its URL), and those that failed lately.
    splits: VecDeque<(IndexFile, String)>,
    splitting: Option<String>,
    split_failed: HashMap<String, Instant>,
    splitter: bool,
}

struct Streamer {
    state: Mutex<State>,
    wake: Condvar,
    split_wake: Condvar,
}

/// Batches of meshes made since the start (the feed looks over its chunks again only when it changes, or every half
/// second).
static MADE_VERSION: AtomicU64 = AtomicU64::new(0);
/// Big files split since the start.
static SPLIT_EPOCH: AtomicU64 = AtomicU64::new(0);
/// Big files queued or being split.
static SPLITS: AtomicUsize = AtomicUsize::new(0);
/// The compressed bytes of the big file in hand read so far.
static SPLIT_BYTES: AtomicU64 = AtomicU64::new(0);

fn split_epoch() -> u64 {
    SPLIT_EPOCH.load(Ordering::Acquire)
}

fn splitting() -> bool {
    SPLITS.load(Ordering::Acquire) > 0
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
            failed: HashMap::new(),
            incomplete: HashMap::new(),
            evicted: HashMap::new(),
            quads: HashMap::new(),
            quad_order: VecDeque::new(),
            index: None,
            status: WorkerStatus::default(),
            worker: false,
            splits: VecDeque::new(),
            splitting: None,
            split_failed: HashMap::new(),
            splitter: false,
        }),
        wake: Condvar::new(),
        split_wake: Condvar::new(),
    })
}

fn lock() -> std::sync::MutexGuard<'static, State> {
    streamer().state.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the worker is doing.
pub fn status() -> WorkerStatus {
    let state = lock();
    WorkerStatus {
        failed: state.failed.len(),
        quadkeys: state.quads.len(),
        footprints: state.quads.values().map(|q| q.footprints).sum(),
        made: state.made.len(),
        made_bytes: state.made_bytes,
        streaming: state.splits.len() + usize::from(state.splitting.is_some()),
        streamed_mb: if state.splitting.is_some() {
            SPLIT_BYTES.load(Ordering::Relaxed) >> 20
        } else {
            0
        },
        ..state.status
    }
}

/// Asks for `chunks` (nearest first) over `context`, in place of whatever was asked before; those made, or failed
/// lately, are left out.
pub fn want(context: &Context, chunks: &[ChunkKey]) {
    if !enabled() {
        return;
    }
    let mut state = lock();
    let now = Instant::now();
    state
        .failed
        .retain(|_, at| now.duration_since(*at) < RETRY_AFTER);
    let wanted: VecDeque<ChunkKey> = chunks
        .iter()
        .copied()
        .filter(|&c| {
            let k = context.made_key(c);
            state.made.get(&k).is_none_or(|m| m.stale_for(context, c))
                && !state.failed.contains_key(&k)
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
        // Below the frame loop it would inherit and the simulation (`host`).
        let spawned = crate::host::spawn_utility("theatre-buildings", work);
        if let Err(e) = spawned {
            state.worker = false;
            log_once("thread", &format!("no worker thread: {e}"));
        }
    }
    streamer().wake.notify_one();
}

/// The mesh made for `chunk` over `context`, if any.
fn made(state: &State, context: &Context, chunk: ChunkKey) -> Option<Arc<ChunkMesh>> {
    state
        .made
        .get(&context.made_key(chunk))
        .map(|m| m.mesh.clone())
}

/// A mesh to draw for `chunk` while its own is made, and the level of detail it was made at: the same chunk made
/// without (or with) the cover, or at the other level of detail.
fn stand_in(state: &State, context: &Context, chunk: ChunkKey) -> Option<(Lod, Arc<ChunkMesh>)> {
    let other = ChunkKey {
        lod: other(chunk.lod),
        ..chunk
    };
    let flipped = |key: ChunkKey| {
        let mut k = context.made_key(key);
        k.2 = !k.2;
        k
    };
    [
        (chunk.lod, flipped(chunk)),
        (other.lod, context.made_key(other)),
        (other.lod, flipped(other)),
    ]
    .iter()
    .find_map(|(lod, k)| state.made.get(k).map(|m| (*lod, m.mesh.clone())))
}

fn work() {
    loop {
        let (context, batch, far) = {
            let mut state = lock();
            loop {
                if let Some(context) = state.context.clone() {
                    // A batch of whole and boxed chunks, or of far ones (many more, made from the window's maps alone).
                    let far = state.wanted.front().is_some_and(|c| c.lod == Lod::Far);
                    let size = if far { FAR_BATCH } else { BATCH };
                    let mut batch = Vec::with_capacity(size);
                    while batch.len() < size
                        && state
                            .wanted
                            .front()
                            .is_some_and(|c| (c.lod == Lod::Far) == far)
                        && let Some(chunk) = state.wanted.pop_front()
                    {
                        if state
                            .made
                            .get(&context.made_key(chunk))
                            .is_none_or(|m| m.stale_for(&context, chunk))
                        {
                            batch.push(chunk);
                        }
                    }
                    state.status.queued = state.wanted.len();
                    if !batch.is_empty() {
                        state.status.working = batch.len();
                        break (context, batch, far);
                    }
                    if !state.wanted.is_empty() {
                        continue;
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
        let epoch = split_epoch();
        // The areas the host draws itself as the batch is begun, the meshes made and marked by them alike: one handed
        // meanwhile makes its chunks again.
        let cleared = clearing();
        // Read before the far chunks' ground is: an answer changing meanwhile makes them again.
        let answered = if far {
            super::period_land::version()
        } else {
            0
        };
        let result = if far {
            Ok((make_far(&context, &batch, &cleared), 0))
        } else {
            make_under(&context, &batch, &cleared)
        };
        let mut state = lock();
        state.status.fetched = 0;
        state.status.fetching = 0;
        match result {
            Ok((meshes, fetched)) => {
                let buildings: u32 = meshes.iter().map(|(_, m, _)| m.buildings).sum();
                if fetched > 0 || started.elapsed() > Duration::from_millis(500) {
                    eprintln!(
                        "the theatre's buildings: {} chunks made ({buildings} buildings, {fetched} files fetched) in \
                         {:.2} s",
                        meshes.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
                for (chunk, mesh, done) in meshes {
                    let k = context.made_key(chunk);
                    let complete = done.tiles && !done.awaiting;
                    if complete {
                        state.incomplete.remove(&k);
                    } else {
                        state.incomplete.insert(k, (Instant::now(), epoch));
                    }
                    let bytes = mesh.bytes();
                    let made = Made {
                        mesh,
                        complete,
                        at: Instant::now(),
                        awaiting: done.awaiting,
                        epoch,
                        answered,
                        relief: context.relief_under(chunk),
                        cleared: cleared.signature(context.geo, chunk.index),
                    };
                    state.evicted.remove(&k);
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
                        state.evicted.insert(old, made.evicted());
                    }
                }
                MADE_VERSION.fetch_add(1, Ordering::Release);
            }
            Err(e) => {
                if matches!(e, FetchError::Offline(_)) {
                    surface::mark_offline();
                }
                log_once(
                    "fetch",
                    &format!(
                        "{e}; the chunks it held wait (a minute) and the infill stands alone meanwhile"
                    ),
                );
                let now = Instant::now();
                for &chunk in &batch {
                    state.failed.insert(context.made_key(chunk), now);
                }
                state.wanted.clear();
                state.status.queued = 0;
            }
        }
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(180)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// A whole file; none where the server has none.
fn get(url: &str, limit: u64) -> Result<Option<Vec<u8>>, FetchError> {
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
                .limit(limit)
                .reader()
                .read_to_end(&mut bytes)
                .map_err(|e| FetchError::Server(format!("{url}: {e}")))?;
            Ok(Some(bytes))
        }
        403 | 404 => Ok(None),
        status => Err(FetchError::Server(format!("{url}: HTTP {status}"))),
    }
}

/// Writes `bytes` whole, then moves them into place.
fn write_file(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp = path.to_path_buf().into_os_string();
    temp.push(format!(".{}.part", std::process::id()));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path)
}

/// The index: in memory, else from disk, else fetched (and kept).
fn index() -> Result<Arc<Index>, FetchError> {
    if let Some(index) = lock().index.clone() {
        return Ok(index);
    }
    let path = cache_dir().join("dataset-links.csv");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            if surface::offline() {
                return Err(FetchError::Offline("tried lately".into()));
            }
            let mut last = None;
            let mut text = None;
            for url in INDEX_URLS {
                match get(url, 64 << 20) {
                    Ok(Some(bytes)) => {
                        text = Some(String::from_utf8_lossy(&bytes).into_owned());
                        break;
                    }
                    Ok(None) => last = Some(FetchError::Server(format!("{url}: none"))),
                    Err(e) => last = Some(e),
                }
            }
            let text =
                text.ok_or_else(|| last.unwrap_or_else(|| FetchError::Server("no index".into())))?;
            if let Err(e) = write_file(&path, text.as_bytes()) {
                log_once("write", &format!("keeping the index: {e}"));
            }
            text
        }
    };
    let index = Arc::new(parse_index(&text));
    if index.files.is_empty() {
        // Not the index (an error page kept): fetched again next time.
        let _ = std::fs::remove_file(&path);
        return Err(FetchError::Server("the index lists no files".into()));
    }
    lock().index = Some(index.clone());
    Ok(index)
}

/// A file's folder in the cache: its region's.
fn location_dir(file: &IndexFile) -> PathBuf {
    let location: String = file
        .location
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    cache_dir().join("footprints").join(location)
}

/// A whole file's place in the cache.
fn file_path(file: &IndexFile, key: &str) -> PathBuf {
    location_dir(file).join(format!("{key}.csv.gz"))
}

/// A big file's tiles' folder in the cache (there only once the whole file was read: [`split::split_into`]).
fn tiles_path(file: &IndexFile, key: &str) -> PathBuf {
    location_dir(file).join(format!("{key}.tiles"))
}

/// Keeps `store` in memory as `id`, dropping the oldest beyond [`QUADS_KEPT`] stores or [`FOOTPRINTS_KEPT`]
/// footprints.
fn keep(id: ([u64; 2], String), store: &Arc<QuadStore>) {
    let mut state = lock();
    if state.quads.insert(id.clone(), store.clone()).is_none() {
        state.quad_order.push_back(id);
    }
    let mut footprints: usize = state.quads.values().map(|q| q.footprints).sum();
    while state.quad_order.len() > 1
        && (state.quad_order.len() > QUADS_KEPT || footprints > FOOTPRINTS_KEPT)
    {
        if let Some(old) = state.quad_order.pop_front()
            && let Some(gone) = state.quads.remove(&old)
        {
            footprints -= gone.footprints;
        }
    }
}

/// Quadkey `key`'s footprints about `geo` from its whole files (those up to [`WHOLE_FILE_BYTES`]): kept, else read
/// from disk or fetched, parsed and projected. Counts the files fetched into `fetched`.
pub fn quad(geo: LatLon, key: &str, fetched: &mut usize) -> Result<Arc<QuadStore>, FetchError> {
    let id = ([geo.lat.to_bits(), geo.lon.to_bits()], key.to_owned());
    if let Some(store) = lock().quads.get(&id).cloned() {
        return Ok(store);
    }
    let index = index()?;
    let mut store = QuadStore::default();
    for file in index.files.get(key).map(Vec::as_slice).unwrap_or_default() {
        if file.bytes > WHOLE_FILE_BYTES {
            continue;
        }
        let path = file_path(file, key);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => {
                if surface::offline() {
                    return Err(FetchError::Offline("tried lately".into()));
                }
                let bytes = get(&file.url, WHOLE_FILE_BYTES + (1 << 20))?.unwrap_or_default();
                *fetched += 1;
                if let Err(e) = write_file(&path, &bytes) {
                    log_once("write", &format!("keeping a file: {e}"));
                }
                bytes
            }
        };
        if bytes.is_empty() {
            continue;
        }
        if let Err(e) = store.add_file(&bytes, geo) {
            log_once("parse", &format!("reading {}: {e}", path.display()));
            // A damaged file is fetched again next time.
            let _ = std::fs::remove_file(&path);
        }
    }
    let store = Arc::new(store);
    keep(id, &store);
    Ok(store)
}

/// A big file's tile `tile` (in its folder `dir`, named `name` in memory) about `geo`: kept, else read and projected.
/// None when the tile is damaged or its folder gone: the folder is thrown away and every tile of it kept in memory
/// forgotten, so the file is streamed again and read anew.
fn tile_store(geo: LatLon, dir: &Path, name: &str, tile: &str) -> Option<Arc<QuadStore>> {
    let id = (
        [geo.lat.to_bits(), geo.lon.to_bits()],
        format!("{name}/{tile}"),
    );
    if let Some(store) = lock().quads.get(&id).cloned() {
        return Some(store);
    }
    let mut store = QuadStore {
        covered: true,
        ..QuadStore::default()
    };
    match split::read_tile(dir, tile) {
        Ok(raws) => {
            for raw in &raws {
                if let Some((chunk, footprint)) = project(raw, geo) {
                    store.chunks.entry(chunk).or_default().push(footprint);
                    store.footprints += 1;
                }
            }
        }
        Err(e) => {
            log_once(
                "tile",
                &format!("{e}; the file's tiles are thrown away and fetched again"),
            );
            let _ = std::fs::remove_dir_all(dir);
            let prefix = format!("{name}/");
            let mut state = lock();
            state.quads.retain(|(_, n), _| !n.starts_with(&prefix));
            state.quad_order.retain(|(_, n)| !n.starts_with(&prefix));
            return None;
        }
    }
    let store = Arc::new(store);
    keep(id, &store);
    Some(store)
}

/// A quadkey's tile column and row.
fn quadkey_xy(key: &str) -> (i64, i64) {
    key.chars().fold((0i64, 0i64), |(x, y), ch| {
        let d = i64::from(ch.to_digit(4).unwrap_or(0));
        (x * 2 + (d & 1), y * 2 + (d >> 1))
    })
}

/// The quadkey of `level` of tile column `x` and row `y`.
fn quadkey_at(x: i64, y: i64, level: u32) -> String {
    (1..=level)
        .rev()
        .map(|i| {
            let mask = 1i64 << (i - 1);
            char::from(b'0' + u8::from(x & mask != 0) + 2 * u8::from(y & mask != 0))
        })
        .collect()
}

/// The quadkeys of `level` under chunk `index`: every tile its latitudes and longitudes (a couple of metres about it)
/// reach, row by row.
pub fn quadkeys_over(geo: LatLon, index: [i32; 2], level: u32) -> Vec<String> {
    let c = chunk_corner(index);
    let pad = 2.0;
    let points: Vec<(i64, i64)> = [
        [c[0] - pad, c[1] - pad],
        [c[0] + CHUNK_M + pad, c[1] - pad],
        [c[0] - pad, c[1] + CHUNK_M + pad],
        [c[0] + CHUNK_M + pad, c[1] + CHUNK_M + pad],
    ]
    .iter()
    .map(|&p| {
        let p = to_lat_lon(geo, p);
        quadkey_xy(&quadkey(p.lat, p.lon, level))
    })
    .collect();
    // Across the date line the columns wrap: those in the western half are counted on past the last one.
    let columns = 1i64 << level;
    let wraps = {
        let x = points.iter().map(|p| p.0);
        x.clone().max().unwrap_or(0) - x.min().unwrap_or(0) > columns / 2
    };
    let points: Vec<(i64, i64)> = points
        .into_iter()
        .map(|(x, y)| {
            (
                if wraps && x < columns / 2 {
                    x + columns
                } else {
                    x
                },
                y,
            )
        })
        .collect();
    let range = |f: fn(&(i64, i64)) -> i64| {
        let v = points.iter().map(f);
        (v.clone().min().unwrap_or(0), v.max().unwrap_or(0))
    };
    let (x0, x1) = range(|p| p.0);
    let (y0, y1) = range(|p| p.1);
    let mut keys = Vec::new();
    for y in y0..=y1 {
        for x in x0..=x1 {
            keys.push(quadkey_at(x.rem_euclid(columns), y, level));
        }
    }
    keys.sort();
    keys
}

/// The dataset's quadkeys under chunk `index`.
fn quadkeys_of(geo: LatLon, index: [i32; 2]) -> Vec<String> {
    quadkeys_over(geo, index, QUADKEY_LEVEL)
}

/// The agent for big files: no limit on the whole transfer but half an hour for the body (a cut connection errors).
fn big_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_recv_body(Some(Duration::from_secs(1800)))
            .http_status_as_error(false)
            .build()
            .into()
    })
}

/// Streams big file `file` (quadkey `key`) and splits it into its tiles' folder.
fn stream_split(file: &IndexFile, key: &str) -> Result<split::Summary, FetchError> {
    let done = tiles_path(file, key);
    if done.is_dir() {
        return Ok(split::Summary::default());
    }
    let url = &file.url;
    let response = big_agent().get(url).call().map_err(|e| match e {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => FetchError::Offline(format!("{url}: {e}")),
        other => FetchError::Server(format!("{url}: {other}")),
    })?;
    match response.status().as_u16() {
        200 => {}
        status => return Err(FetchError::Server(format!("{url}: HTTP {status}"))),
    }
    SPLIT_BYTES.store(0, Ordering::Relaxed);
    let reader = response.into_body().into_reader();
    split::split_into(reader, &done, &SPLIT_BYTES)
        .map_err(|e| FetchError::Server(format!("{url}: {e}")))
}

/// Asks for big file `file` (quadkey `key`) to be streamed and split, unless it is queued or in hand already, failed
/// lately, or the network is down.
fn request_split(file: &IndexFile, key: &str) {
    let mut state = lock();
    if state.splitting.as_deref() == Some(file.url.as_str())
        || state.splits.iter().any(|(f, _)| f.url == file.url)
        || state
            .split_failed
            .get(&file.url)
            .is_some_and(|at| at.elapsed() < SPLIT_RETRY_AFTER)
        || surface::offline()
    {
        return;
    }
    log_once(
        &format!("big-{key}"),
        &format!(
            "{} quadkey {key} is {} MB: streamed and split into tiles, the infill standing in meanwhile",
            file.location,
            file.bytes >> 20
        ),
    );
    state.splits.push_back((file.clone(), key.to_owned()));
    SPLITS.fetch_add(1, Ordering::AcqRel);
    if !state.splitter {
        state.splitter = true;
        if let Err(e) = crate::host::spawn_utility("theatre-footprints", split_work) {
            state.splitter = false;
            SPLITS.fetch_sub(state.splits.len(), Ordering::AcqRel);
            state.splits.clear();
            log_once("split-thread", &format!("no thread for big files: {e}"));
            return;
        }
    }
    streamer().split_wake.notify_one();
}

/// The big files' thread: one file at a time, in the order asked.
fn split_work() {
    loop {
        let (file, key) = {
            let mut state = lock();
            loop {
                if let Some(next) = state.splits.pop_front() {
                    state.splitting = Some(next.0.url.clone());
                    break next;
                }
                state = streamer()
                    .split_wake
                    .wait(state)
                    .unwrap_or_else(|e| e.into_inner());
            }
        };
        let started = Instant::now();
        let result = stream_split(&file, &key);
        let mut state = lock();
        state.splitting = None;
        match result {
            Ok(summary) => {
                state.split_failed.remove(&file.url);
                if summary.footprints > 0 {
                    eprintln!(
                        "the theatre's buildings: {} quadkey {key} streamed ({} MB) into {} tiles of {} footprints in \
                         {:.1} s",
                        file.location,
                        SPLIT_BYTES.load(Ordering::Relaxed) >> 20,
                        summary.tiles,
                        summary.footprints,
                        started.elapsed().as_secs_f64()
                    );
                }
                // The chunks made without it are made again (`Made::stale`).
                SPLIT_EPOCH.fetch_add(1, Ordering::AcqRel);
            }
            Err(e) => {
                if matches!(e, FetchError::Offline(_)) {
                    surface::mark_offline();
                }
                log_once(
                    &format!("split-{key}"),
                    &format!(
                        "streaming {} quadkey {key}: {e}; the infill stands in",
                        file.location
                    ),
                );
                state.split_failed.insert(file.url.clone(), Instant::now());
            }
        }
        SPLITS.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What a chunk was made with besides its mesh: whether every OpenStreetMap tile it wanted was had, and whether a big
/// file under it was still to be split (the infill standing in for it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Done {
    pub tiles: bool,
    pub awaiting: bool,
}

/// Makes `batch`'s meshes over `context`: fetching what their quadkeys need and the OpenStreetMap tiles under them,
/// then planning, filling in and meshing each chunk (a few threads). The meshes (each with what it lacked), and the
/// files fetched.
#[allow(clippy::type_complexity)]
pub fn make(
    context: &Context,
    batch: &[ChunkKey],
) -> Result<(Vec<(ChunkKey, Arc<ChunkMesh>, Done)>, usize), FetchError> {
    make_under(context, batch, &clearing())
}

/// [`make`] under the areas `cleared` (the worker's, read as its batch is begun).
#[allow(clippy::type_complexity)]
fn make_under(
    context: &Context,
    batch: &[ChunkKey],
    cleared: &Clearing,
) -> Result<(Vec<(ChunkKey, Arc<ChunkMesh>, Done)>, usize), FetchError> {
    let ground = &context.ground;
    let land: Vec<bool> = batch.iter().map(|c| ground.has_land(c.index)).collect();
    let chunk_keys: Vec<Vec<String>> = batch
        .iter()
        .zip(&land)
        .map(|(c, l)| {
            if *l {
                quadkeys_of(context.geo, c.index)
            } else {
                Vec::new()
            }
        })
        .collect();
    let mut keys: Vec<String> = chunk_keys.iter().flatten().cloned().collect();
    keys.sort();
    keys.dedup();
    {
        let mut state = lock();
        state.status.fetching = keys.len();
        state.status.fetched = 0;
    }
    let mut fetched = 0;
    let mut stores: HashMap<String, Arc<QuadStore>> = HashMap::new();
    for (k, key) in keys.iter().enumerate() {
        stores.insert(key.clone(), quad(context.geo, key, &mut fetched)?);
        lock().status.fetched = k + 1;
    }
    // Each chunk's footprints' stores in a fixed order: its whole files', then its big files' tiles; a big file not yet
    // split is asked for, and the infill stands in for it meanwhile.
    let index = if keys.is_empty() {
        None
    } else {
        Some(index()?)
    };
    let mut sources: Vec<(Vec<Arc<QuadStore>>, bool)> = Vec::with_capacity(batch.len());
    for (chunk, keys) in batch.iter().zip(&chunk_keys) {
        let mut found: Vec<Arc<QuadStore>> =
            keys.iter().filter_map(|k| stores.get(k).cloned()).collect();
        let mut awaiting = false;
        for key in keys {
            let files = index.as_ref().and_then(|i| i.files.get(key));
            for file in files.map(Vec::as_slice).unwrap_or_default() {
                if file.bytes <= WHOLE_FILE_BYTES {
                    continue;
                }
                let dir = tiles_path(file, key);
                if !dir.is_dir() {
                    request_split(file, key);
                    awaiting = true;
                    continue;
                }
                let name = format!("{}/{key}", file.location);
                for tile in quadkeys_over(context.geo, chunk.index, split::SPLIT_LEVEL) {
                    match tile_store(context.geo, &dir, &name, &tile) {
                        Some(store) => found.push(store),
                        None => {
                            // Damaged: streamed again, and the chunk made again once it is.
                            request_split(file, key);
                            awaiting = true;
                            break;
                        }
                    }
                }
            }
        }
        sources.push((found, awaiting));
    }
    // The map's tiles under the chunks with land (only those: the tiles' terms forbid collecting more).
    let mut ids: Vec<TileId> = batch
        .iter()
        .zip(&land)
        .filter(|(_, l)| **l)
        .flat_map(|(c, _)| osm_tiles_of(context.geo, *c, context.period))
        .collect();
    ids.sort_by_key(|t| (t.z, t.x, t.y));
    ids.dedup();
    let tiles: HashMap<TileId, Arc<OsmTile>> = if osm::enabled() && !ids.is_empty() {
        let area = match osm::load(context.geo, &ids) {
            Ok(area) => area,
            // Offline or a server's error: what memory holds now (`load` kept every tile it could read).
            Err(_) => osm::Area {
                tiles: ids
                    .iter()
                    .filter_map(|&id| osm::tile(context.geo, id))
                    .collect(),
                missing: Vec::new(),
            },
        };
        area.tiles.into_iter().map(|t| (t.id, t)).collect()
    } else {
        HashMap::new()
    };
    // A past period's thinning of each chunk, from its neighbours' footprints and its 5' cells' (here, one at a time:
    // it reads files the chunks share).
    let gathering = Instant::now();
    let (periods, period_fetched) = period::prepare(context, batch, &land);
    fetched += period_fetched;
    if gathering.elapsed() > Duration::from_millis(100) {
        eprintln!(
            "the theatre's buildings: the period's thinning of {} chunks gathered in {:.0} ms",
            periods.iter().flatten().count(),
            gathering.elapsed().as_secs_f64() * 1e3
        );
    }
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 4);
    let jobs: Vec<(usize, ChunkKey)> = batch.iter().copied().enumerate().collect();
    let per = jobs.len().div_ceil(threads);
    // At the worker's class (`host::Inherit`).
    let qos = crate::host::Inherit::here();
    let mut meshes: Vec<(usize, ChunkKey, Arc<ChunkMesh>, Done)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(per.max(1))
            .map(|part| {
                let sources = &sources;
                let land = &land;
                let tiles = &tiles;
                let periods = &periods;
                scope.spawn(move || {
                    qos.apply();
                    part.iter()
                        .map(|&(k, chunk)| {
                            if !land[k] {
                                let done = Done {
                                    tiles: true,
                                    awaiting: false,
                                };
                                return (k, chunk, Arc::new(ChunkMesh::default()), done);
                            }
                            let wanted = if osm::enabled() {
                                osm_tiles_of(context.geo, chunk, context.period)
                            } else {
                                Vec::new()
                            };
                            let have: Vec<Arc<OsmTile>> = wanted
                                .iter()
                                .filter_map(|id| tiles.get(id).cloned())
                                .collect();
                            let (stores, awaiting) = &sources[k];
                            let past = periods[k].as_ref();
                            let lacking = past.map(|p| p.lacking).unwrap_or_default();
                            let done = Done {
                                tiles: have.len() == wanted.len() && !lacking.missing,
                                awaiting: *awaiting || lacking.awaiting,
                            };
                            let mesh =
                                chunk_mesh_under(context, chunk, stores, &have, past, cleared);
                            (k, chunk, Arc::new(mesh), done)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });
    meshes.sort_by_key(|(k, _, _, _)| *k);
    // The period's ground answers as the chunks' buildings stand.
    let kept: Vec<Option<_>> = meshes.iter().map(|(_, _, m, _)| m.kept.clone()).collect();
    period::answer(context, batch, &periods, &kept);
    Ok((
        meshes
            .into_iter()
            .map(|(_, c, m, done)| (c, m, done))
            .collect(),
        fetched,
    ))
}

/// Makes a batch of far chunks over `context` (a few threads): each its town's masses from the window's maps alone
/// (`naval_terrain::buildings::far`), nothing fetched. In a past period only on the ground that stood
/// (`period_land`'s answers: the buildings' thinning where it ran, the cover's estimate beyond); a chunk whose ground is
/// not known yet, or that the window's maps do not wholly cover, is made again a minute later.
fn make_far(
    context: &Context,
    batch: &[ChunkKey],
    cleared: &Clearing,
) -> Vec<(ChunkKey, Arc<ChunkMesh>, Done)> {
    let answers = super::period_land::answers(context.geo, context.period);
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 4);
    let per = batch.len().div_ceil(threads).max(1);
    let qos = crate::host::Inherit::here();
    std::thread::scope(|scope| {
        let handles: Vec<_> = batch
            .chunks(per)
            .map(|part| {
                let answers = answers.as_deref();
                scope.spawn(move || {
                    qos.apply();
                    part.iter()
                        .map(|&chunk| {
                            let (mesh, done) = far_mesh(context, answers, chunk, cleared);
                            (chunk, Arc::new(mesh), done)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// One far chunk's masses ([`make_far`]) under the areas `cleared`, and whether all it wanted was had.
fn far_mesh(
    context: &Context,
    answers: Option<&super::period_land::Answers>,
    chunk: ChunkKey,
    cleared: &Clearing,
) -> (ChunkMesh, Done) {
    let mut done = Done {
        tiles: true,
        awaiting: false,
    };
    // Past the window (its maps and heights), made again once the window has moved over it.
    let Some(cover) = context.cover.as_ref() else {
        done.tiles = false;
        return (ChunkMesh::default(), done);
    };
    done.tiles = cover.covers(chunk.index);
    if !context.ground.has_land(chunk.index) {
        return (ChunkMesh::default(), done);
    }
    let past = context.period.is_past();
    let ground = answers.map(|a| a.ground(chunk.index));
    if past && !ground.is_some_and(|g| g.known()) {
        done.tiles = false;
        return (ChunkMesh::default(), done);
    }
    let corner = chunk_corner(chunk.index);
    // No mass in an area the host draws itself.
    let cleared = cleared_rings(cleared, context.geo, chunk.index);
    let stood = |p: [f64; 2]| {
        let local = [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32];
        (!past || ground.is_some_and(|g| g.stood(local)))
            && !cleared.iter().any(|ring| geometry::contains(ring, local))
    };
    let land = Land {
        ground: &context.ground,
        cover: Some(cover),
    };
    let region = context.region_of(chunk.index);
    let middle = to_lat_lon(
        context.geo,
        [corner[0] + CHUNK_M * 0.5, corner[1] + CHUNK_M * 0.5],
    );
    let mesh = far::build_far(&far::FarInput {
        index: chunk.index,
        land: &land,
        stood: &stood,
        region: &|p| context.region_at(p),
        chunk_region: region,
        period: context.period,
        cap_m: thinning::height_cap(region, middle.lat, middle.lon),
    });
    (mesh, done)
}

/// How far about a chunk its map's tiles are read (m): a quay's water and a pier reaching in from beyond it.
const TAGS_REACH_M: f64 = 150.0;

/// The zoom a chunk's map is read at: the deepest (14) at both near levels, whatever the period. OpenMapTiles' zoom 13
/// holds a tile's buildings as one shape with no heights, parts or colours (a boxed tower 3.5 km off fell to an
/// estimate, and where Microsoft has no footprints, as in Taiwan, a city to blobs), and in a past period what stood is
/// decided from the map (the lifts' places of worship and small railway, military and harbour land, the working
/// waterfront, OSM's outlines merged into the footprints): a chunk keeps the same buildings whichever level draws it.
fn osm_zoom(_lod: Lod, _period: Period) -> u8 {
    osm::ZOOM
}

/// The OpenStreetMap tiles under chunk `chunk` and [`TAGS_REACH_M`] about it, at its level's zoom in `period`
/// ([`osm_zoom`]).
pub fn osm_tiles_of(geo: LatLon, chunk: ChunkKey, period: Period) -> Vec<TileId> {
    let c = chunk_corner(chunk.index);
    osm::tiles_over(
        geo,
        [c[0] - TAGS_REACH_M, c[1] - TAGS_REACH_M],
        [c[0] + CHUNK_M + TAGS_REACH_M, c[1] + CHUNK_M + TAGS_REACH_M],
        osm_zoom(chunk.lod, period),
    )
}

/// One chunk's mesh: the source's footprints in it with OpenStreetMap's outlines merged in, the infill over the
/// built-up cover its footprints leave empty (but on land the map keeps open), typed by the map's tags from `tiles`;
/// in a past period thinned by `past` ([`period::prepare`]).
pub fn chunk_mesh(
    context: &Context,
    chunk: ChunkKey,
    stores: &[Arc<QuadStore>],
    tiles: &[Arc<OsmTile>],
    past: Option<&period::Prepared>,
) -> ChunkMesh {
    chunk_mesh_under(context, chunk, stores, tiles, past, &clearing())
}

/// [`chunk_mesh`] under the areas `cleared` (the worker's, read as its batch is begun, so the mesh is marked by the
/// areas it was made under).
fn chunk_mesh_under(
    context: &Context,
    chunk: ChunkKey,
    stores: &[Arc<QuadStore>],
    tiles: &[Arc<OsmTile>],
    past: Option<&period::Prepared>,
    cleared: &Clearing,
) -> ChunkMesh {
    let mut footprints: Vec<Footprint> = Vec::new();
    for store in stores {
        if let Some(found) = store.chunks.get(&chunk.index) {
            footprints.extend(found.iter().cloned());
        }
    }
    let mut tags = tags_for(context.geo, chunk.index, tiles);
    // The areas the host draws itself: nothing of the chunk's stands there, and the infill leaves them empty (its lots
    // are laid about every footprint, those in them too, and left out with them).
    tags.cleared = cleared_rings(cleared, context.geo, chunk.index);
    tags::merge_outlines(&mut footprints, &tags.outlines, &tags.outlines_about);
    let land = Land {
        ground: &context.ground,
        cover: context.cover.as_ref(),
    };
    let region = context.region_of(chunk.index);
    let lots = if context.cover.is_some() {
        let mut lots = infill::lots(chunk.index, &land, &infill::occupied(&footprints), &|p| {
            context.region_at(p)
        });
        if !tags.zones.is_empty() || !tags.roads.is_empty() {
            lots.retain(|f| {
                tags.usage_at(geometry::centroid(&f.ring)) != Usage::Open && !tags.on_road(&f.ring)
            });
        }
        // A terminal's container yard holds the stacks (`theatre::infra`).
        super::infra::clear_yards(&tags, &context.ground, chunk.index, &mut lots);
        // An airfield's paving and grass (`theatre::infra`), and a generated base's clearing in the period.
        super::infra::clear_airfields(chunk.index, tiles, &mut lots);
        super::bases::clear(context, chunk.index, &mut lots);
        lots
    } else {
        Vec::new()
    };
    let corner = chunk_corner(chunk.index);
    // The buildings stand on the land as drawn: with its fine relief near the camera.
    let ground = |p: [f32; 2]| {
        context.footing([corner[0] + f64::from(p[0]), corner[1] + f64::from(p[1])]) as f32
    };
    // No building of today's stands on a generated base's clearing in the period.
    super::bases::clear(context, chunk.index, &mut footprints);
    let thinning = past.map(|p| p.thinning(chunk.index, footprints.iter().chain(&lots)));
    build_chunk(&ChunkInput {
        key: chunk,
        footprints: &footprints,
        infill: &lots,
        tags: &tags,
        ground: &ground,
        region,
        period: context.period,
        thinning: thinning.as_ref(),
    })
}

/// OpenMapTiles' default height for a building with neither `height` nor `building:levels` (m): no measurement.
const OMT_DEFAULT_HEIGHT_M: f64 = 5.0;

/// A polygon cut to its tile: its exterior and holes, chunk-local.
type Clipped = (Vec<[f32; 2]>, Vec<Vec<[f32; 2]>>);

/// The map's tags for chunk `index` (chunk-local metres) from `tiles` about `geo`: the land use, the points of interest
/// that type a building, OSM's building outlines whose middles lie in the chunk, the water (the sea, docks, rivers) and
/// the piers, each tile's polygons, lines and points held to the tile proper (its four corners) so neighbouring tiles'
/// pieces meet without overlapping; and the [`LANDMARKS`] in the chunk.
pub fn tags_for(geo: LatLon, index: [i32; 2], tiles: &[Arc<OsmTile>]) -> MapTags {
    let corner = chunk_corner(index);
    let local = |p: &[f64; 2]| [(p[0] - corner[0]) as f32, (p[1] - corner[1]) as f32];
    let ring_of = |r: &[[f64; 2]]| r.iter().map(local).collect::<Vec<[f32; 2]>>();
    let min = [corner[0] - TAGS_REACH_M, corner[1] - TAGS_REACH_M];
    let max = [
        corner[0] + CHUNK_M + TAGS_REACH_M,
        corner[1] + CHUNK_M + TAGS_REACH_M,
    ];
    let in_chunk = |p: [f32; 2]| {
        (0.0..CHUNK_M as f32).contains(&p[0]) && (0.0..CHUNK_M as f32).contains(&p[1])
    };
    let mut out = MapTags::default();
    for tile in tiles {
        // A ferry terminal anywhere in a tile about the chunk: a working port (its quays get cranes).
        out.port |= tile
            .features
            .poi
            .iter()
            .any(|p| p.class == "ferry_terminal");
        let quad: Vec<[f32; 2]> = tile.quad(geo).iter().map(local).collect();
        let polygons = |g: &Geometry<[f64; 2]>| -> Vec<Clipped> {
            let Geometry::Polygons(ps) = g else {
                return Vec::new();
            };
            ps.iter()
                .filter_map(|p| {
                    let exterior = geometry::clip_polygon(&ring_of(&p.exterior), &quad);
                    (exterior.len() >= 3).then(|| {
                        let holes = p
                            .holes
                            .iter()
                            .map(|h| geometry::clip_polygon(&ring_of(h), &quad))
                            .filter(|h| h.len() >= 3)
                            .collect();
                        (exterior, holes)
                    })
                })
                .collect()
        };
        for l in tile.landuse_in(min, max) {
            let Some(usage) = Usage::of_landuse(&l.class) else {
                continue;
            };
            for (exterior, holes) in polygons(&l.geometry) {
                let mut zone = Zone::new(usage, exterior, holes);
                zone.railway = l.class == "railway";
                out.zones.push(zone);
            }
        }
        for p in tile.poi_in(min, max) {
            let Some(kind) = PoiKind::of(&p.class, p.subclass.as_deref(), p.name.as_deref()) else {
                continue;
            };
            for at in p.geometry.points() {
                let at = local(at);
                if geometry::contains(&quad, at) {
                    out.pois.push(tags::Poi { at, kind });
                }
            }
        }
        for b in tile.building_in(min, max) {
            let Geometry::Polygons(ps) = &b.geometry else {
                continue;
            };
            for p in ps {
                let ring = ring_of(&p.exterior);
                if ring.len() < 3 {
                    continue;
                }
                let c = geometry::centroid(&ring);
                // Each building once: in the tile whose proper holds its middle, in the chunk that holds it; those
                // about the chunk read for the building a part of its belongs to (`MapTags::outlines_about`).
                let reach = TAGS_REACH_M as f32;
                let near = |v: f32| (-reach..CHUNK_M as f32 + reach).contains(&v);
                if !near(c[0]) || !near(c[1]) || !geometry::contains(&quad, c) {
                    continue;
                }
                let outlines = if in_chunk(c) {
                    &mut out.outlines
                } else {
                    &mut out.outlines_about
                };
                outlines.push(Outline {
                    ring,
                    height: b
                        .render_height
                        .filter(|h| *h > 0.0 && (*h - OMT_DEFAULT_HEIGHT_M).abs() > 1e-3)
                        .map(|h| h as f32),
                    min_height: b.render_min_height.unwrap_or(0.0) as f32,
                    colour: b.colour.as_deref().and_then(tags::parse_colour),
                    hide_3d: b.hide_3d,
                });
            }
        }
        for w in tile.water_in(min, max) {
            if !matches!(w.class.as_str(), "ocean" | "dock" | "river") {
                continue;
            }
            for (exterior, holes) in polygons(&w.geometry) {
                out.water.push(tags::WaterArea::new(exterior, holes));
            }
        }
        for t in tile.transportation_in(min, max) {
            // Roads and railways the infill's lots keep off (not driveways, parking aisles or tracks).
            let narrow = matches!(t.service.as_deref(), Some("driveway" | "parking_aisle"));
            if let Some((kind, half, _)) = super::features::style_of(t)
                && kind != naval_terrain::StrokeKind::Unpaved
                && !narrow
                && let Geometry::Lines(lines) = &t.geometry
            {
                let half = if kind == naval_terrain::StrokeKind::Rail {
                    2.5
                } else {
                    half
                };
                for line in lines {
                    for piece in geometry::clip_polyline(&ring_of(line), &quad) {
                        out.roads.extend(piece.windows(2).map(|w| tags::Road {
                            a: w[0],
                            b: w[1],
                            half,
                        }));
                    }
                }
            }
            if t.class != "pier" {
                continue;
            }
            match &t.geometry {
                Geometry::Polygons(_) => {
                    for (exterior, _) in polygons(&t.geometry) {
                        out.piers.extend(Pier::area(exterior));
                    }
                }
                Geometry::Lines(lines) => {
                    for line in lines {
                        for piece in geometry::clip_polyline(&ring_of(line), &quad) {
                            out.piers.extend(Pier::line(&piece));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    for &(lat, lon, kind) in LANDMARKS {
        let at = local(&to_local(geo, LatLon { lat, lon }));
        if in_chunk(at) {
            out.landmarks.push(Landmark { at, kind });
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------------------------------
// The scene's side.

/// Where the camera stood at the last frame a scene fed its buildings (the scenario's frame, the theatre point).
static LAST_CAMERA: Mutex<Option<([u64; 2], [f64; 3])>> = Mutex::new(None);

/// The chunks within `settings`' reach of the camera at theatre point (x, z) and height `camera[2]`, nearest first,
/// each at its level of detail (`held` the one a chunk is drawn at now): whole and boxes by the distance from the camera
/// (its height over 40 m counted), the far level's by the distance across the ground (`BuildingSettings::lod_at`).
pub fn wanted(
    camera: [f64; 3],
    settings: &BuildingSettings,
    held: &dyn Fn([i32; 2]) -> Option<Lod>,
) -> Vec<(f32, ChunkKey)> {
    let reach = (f64::from(settings.reach_m()) / CHUNK_M).ceil() as i32 + 1;
    let at = chunk_of([camera[0], camera[1]]);
    let dy = (camera[2] - 40.0).max(0.0);
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
            let across = dx * dx + dz * dz;
            let d = (across + dy * dy).sqrt() as f32;
            if let Some(lod) = settings.lod_at(d, across.sqrt() as f32, held(index)) {
                out.push((d, ChunkKey { index, lod }));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// What a scene lends the layers made for it (the buildings', the infrastructure's): the sky's environment they are lit
/// by, the clouds' shadow and the fog.
#[derive(Clone, Copy)]
pub struct SceneLook<'a> {
    pub environment: &'a wgpu::BindGroup,
    pub cloud_shadow: &'a wgpu::BindGroup,
    pub fog: naval_ocean::FogParameters,
}

/// The layer for a scene being built (off the frame loop's thread), with its theatre at `offset` (world x, z = theatre
/// x, z + offset) and the made chunks about where the camera last stood, so a rebuilt scene draws them from its first
/// frame. None with buildings off.
pub fn layer(
    ctx: &mut naval_render::RenderContext<'_>,
    scene: &SceneLook<'_>,
    quality: &naval_terrain::TerrainQuality,
    context: &Context,
    offset: [f64; 2],
) -> Option<BuildingsLayer> {
    if !enabled() {
        return None;
    }
    let mut layer = match BuildingsLayer::new(
        ctx,
        scene.environment,
        settings_for(quality, context.period),
    ) {
        Ok(layer) => layer,
        Err(e) => {
            log_once("shaders", &format!("the buildings' shaders: {e:?}"));
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
            if let Some(mesh) = made(&state, context, key) {
                layer.queue_chunk(key, mesh);
            } else if let Some((lod, mesh)) = stand_in(&state, context, key) {
                layer.queue_chunk(ChunkKey { lod, ..key }, mesh);
            }
        }
        drop(state);
        layer.flush(ctx);
    }
    Some(layer)
}

/// What a scene on screen does with its buildings, from frame to frame.
#[derive(Default)]
pub struct Feed {
    /// The scene it feeds (the driver's swap count).
    pub scene: u64,
    /// The camera the wanted list was made about, and when.
    about: Option<[f64; 3]>,
    made_at: Option<Instant>,
    wanted: Vec<(f32, ChunkKey)>,
    /// The chunks last asked of the worker, and when.
    asked: Vec<ChunkKey>,
    asked_at: Option<Instant>,
    /// The wanted chunks not yet made, those made while a big file under them was still to be split, and the big files
    /// split when that was counted.
    pub pending: usize,
    pub awaiting: usize,
    epoch: u64,
    context: Option<Context>,
    /// The worker's made batches when the feed last looked over its chunks ([`MADE_VERSION`]), and the relief's pages
    /// then: the far level's thousands are looked over again only when something was made, the relief changed, or the
    /// list made again.
    fed: Option<(u64, u64)>,
}

static FEED: Mutex<Option<Feed>> = Mutex::new(None);

impl Feed {
    /// Hands `layer` the made chunks about its camera, drops those out of range and asks the worker for the rest.
    pub fn feed(&mut self, layer: &mut BuildingsLayer, context: &Context) {
        let Some(camera) = layer.camera_theatre() else {
            return;
        };
        *LAST_CAMERA.lock().unwrap_or_else(|e| e.into_inner()) = Some((
            [context.geo.lat.to_bits(), context.geo.lon.to_bits()],
            camera,
        ));
        let changed = self.context.as_ref().is_none_or(|c| !c.same(context));
        if changed {
            self.context = Some(context.clone());
        }
        // Read before the made meshes are: a split finishing meanwhile shows as unfed.
        let epoch = split_epoch();
        let split = epoch != self.epoch;
        self.epoch = epoch;
        let moved = self.about.is_none_or(|a| {
            (a[0] - camera[0]).hypot(a[1] - camera[1]) > 100.0 || (a[2] - camera[2]).abs() > 100.0
        });
        let stale = self
            .made_at
            .is_none_or(|t| t.elapsed() > Duration::from_millis(500));
        let version = (
            MADE_VERSION.load(Ordering::Acquire),
            context.relief.as_ref().map_or(0, |r| r.signature(None)),
        );
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
        } else if self.fed == Some(version) && !split {
            return;
        }
        self.fed = Some(version);
        let mut missing = Vec::new();
        let mut awaiting = 0;
        {
            let state = lock();
            for &(_, key) in &self.wanted {
                match state.made.get(&context.made_key(key)) {
                    Some(made) => {
                        awaiting += usize::from(made.awaiting);
                        // A stand-in held, or a mesh made again (with the map's tiles it lacked): this one in its place.
                        if !layer.holds_mesh(key, &made.mesh) {
                            layer.queue_chunk(key, made.mesh.clone());
                        }
                        if made.stale_for(context, key) {
                            missing.push(key);
                        }
                    }
                    None => {
                        let k = context.made_key(key);
                        // Drawn already and only its mesh gone from the cache: left as it is, unless what is drawn
                        // is a stand-in made without (or with) the cover, it was made lacking some of its tiles a
                        // while ago, or the relief under it or the areas the host draws over it have changed since.
                        let mut flipped = k;
                        flipped.2 = !flipped.2;
                        let stand_in_held = state
                            .made
                            .get(&flipped)
                            .is_some_and(|m| layer.holds_mesh(key, &m.mesh));
                        let retry = state.incomplete.get(&k).is_some_and(|(at, epoch)| {
                            at.elapsed() >= RETRY_AFTER || *epoch != split_epoch()
                        });
                        let changed = state
                            .evicted
                            .get(&k)
                            .is_some_and(|e| e.stale_for(context, key));
                        if layer.holds(key) && !stand_in_held && !retry && !changed {
                            continue;
                        }
                        missing.push(key);
                        // Drawn meanwhile at the level it was made at.
                        if layer.held_lod(key.index).is_none()
                            && let Some((lod, mesh)) = stand_in(&state, context, key)
                        {
                            layer.queue_chunk(ChunkKey { lod, ..key }, mesh);
                        }
                    }
                }
            }
        }
        self.pending = missing.len();
        self.awaiting = awaiting;
        let stale = self.asked_at.is_none_or(|t| t.elapsed() > ASK_AGAIN);
        if missing != self.asked || changed || (!missing.is_empty() && stale) {
            want(context, &missing);
            self.asked = missing;
            self.asked_at = Some(Instant::now());
        }
    }
}

/// The level a chunk's mesh at `lod` stands in for while its own is made, and the other way about: whole and boxes for
/// each other, boxes for the far level.
fn other(lod: Lod) -> Lod {
    match lod {
        Lod::Full => Lod::Box,
        Lod::Box => Lod::Full,
        Lod::Far => Lod::Box,
    }
}

/// Feeds the scene's buildings (`scene_id` the driver's swap count) with `context`, the theatre at `offset`.
pub fn feed(scene_id: u64, layer: &mut BuildingsLayer, context: &Context, offset: [f64; 2]) {
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

/// The buildings' state for the RPC (`theatre_state`'s `buildings`): `state` `off`, `none` (no buildings layer in the
/// scene), `waiting` (chunks still to come) or `drawn`; the layer's chunks, buildings and triangles; the worker's.
pub fn status_json(scene_id: u64, layer: Option<&BuildingsLayer>) -> serde_json::Value {
    if !enabled() {
        return serde_json::json!({"state": "off"});
    }
    let Some(layer) = layer else {
        return serde_json::json!({"state": "none"});
    };
    let counts = layer.counts();
    let (pending, awaiting, fed) = FEED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .filter(|f| f.scene == scene_id)
        .map_or((0, 0, true), |f| {
            (f.pending, f.awaiting, f.epoch == split_epoch())
        });
    let worker = status();
    // A big file under the chunks drawn streaming in (drawn with the infill meanwhile), or split since the feed last
    // looked: waiting for its footprints.
    let state = if pending > 0
        || counts.queued > 0
        || layer.camera_theatre().is_none()
        || (awaiting > 0 && worker.streaming > 0)
        || !fed
    {
        "waiting"
    } else {
        "drawn"
    };
    serde_json::json!({
        "state": state,
        "pending": pending,
        "chunks": counts.chunks,
        "withBuildings": counts.with_buildings,
        "queued": counts.queued,
        "buildings": counts.buildings,
        "triangles": counts.triangles,
        "drawnChunks": counts.drawn_chunks,
        "drawnTriangles": counts.drawn_triangles,
        "drawnClutter": counts.drawn_clutter,
        "farChunks": counts.far_chunks,
        "masses": counts.masses,
        "drawnFarChunks": counts.drawn_far_chunks,
        "drawnMasses": counts.drawn_masses,
        "fading": counts.fading,
        "farM": layer.settings().far_m,
        "gpuBytes": counts.bytes,
        "worker": worker,
    })
}

#[cfg(test)]
#[path = "buildings_tests.rs"]
mod tests;
