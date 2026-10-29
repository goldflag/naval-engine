//! The world chart under the scenario editor's globe (native/docs/ui-contract/maps.md, "The editor's globe"): the
//! bundled `public/theatre/world/world-v1.bin` the page sends once (`map-world`), decoded as
//! `src/theatre/editor/worldData.ts` does, and its layers made into meshes on the sphere once, on a thread: the depth
//! bands and the land filled, the coasts and reefs as segments, a graticule.
//!
//! A fill is cut into cells of [`CELL`] degrees and triangulated in each (lyon, even-odd), so no triangle is longer
//! than a cell's diagonal: its flat chord stays within a third of a pixel of the sphere wherever the limb is in view, and
//! shared cell edges keep the fill free of cracks. The GPU projects the unit vectors per frame ([`crate::pass`]), so a
//! camera change costs no tessellation; the meshes are grouped by tiles of [`TILE`] degrees for culling.
//!
//! Two levels of detail: near, Natural Earth's 1:10m land, coasts, reefs and depth bands; far, the 1:50m overview land
//! and coast and the depth bands thinned to [`FAR_STEP`] degrees.

use std::io::Read;
use std::sync::{Arc, mpsc};

use anyhow::{Context, Result, bail, ensure};
use lyon_path::{Path, math::point};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, VertexBuffers,
};
use serde::Deserialize;

use crate::globe::{V3, dot, unit, wrap};

/// The file's magic and the version this reader takes (`WORLD_MAGIC`, `WORLD_VERSION`).
const MAGIC: &[u8; 4] = b"NWD1";
const VERSION: u32 = 1;
/// A fill's cells, degrees.
pub const CELL: f64 = 2.5;
/// The culling tiles, degrees (a whole number of cells).
pub const TILE: f64 = 10.0;
pub const TILE_COLUMNS: usize = 36;
pub const TILE_ROWS: usize = 18;
/// The far depth bands keep a point at least this far (degrees, longitude by its parallel) from the last.
pub const FAR_STEP: f64 = 0.03;
/// A coast or reef edge longer than this (degrees) is cut into pieces this long, so it bends with the sphere.
const SEGMENT: f64 = 1.0;

/// The layers in file order (`WORLD_LAYERS`).
pub const LAYERS: [&str; 7] = [
    "land",
    "reefs",
    "bathymetry-200",
    "bathymetry-1000",
    "bathymetry-3000",
    "bathymetry-4000",
    "overview",
];

#[derive(Deserialize)]
struct Header {
    version: u32,
    quantum: f64,
    layers: Vec<HeaderLayer>,
}
#[derive(Deserialize)]
struct HeaderLayer {
    name: String,
    kind: String,
    count: usize,
}

/// One ring or line: longitude, latitude degrees.
#[derive(Clone, Debug)]
pub struct WorldRing {
    pub coords: Vec<[f64; 2]>,
    pub outer: bool,
}

/// The decoded file: each of [`LAYERS`]' rings (or lines, for the reefs).
#[derive(Clone, Debug)]
pub struct World {
    pub layers: Vec<Vec<WorldRing>>,
    pub lines: Vec<bool>,
}

impl World {
    pub fn layer(&self, name: &str) -> &[WorldRing] {
        LAYERS
            .iter()
            .position(|l| *l == name)
            .map_or(&[], |i| &self.layers[i])
    }

    pub fn points(&self) -> usize {
        self.layers.iter().flatten().map(|r| r.coords.len()).sum()
    }
}

/// Standard base64 (RFC 4648), padding optional, whitespace skipped.
pub fn unbase64(text: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0;
    for &c in text.as_bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => bail!("the world chart is not base64"),
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// The file as fetched (gzip, inflated here when it starts `1f 8b`) as a world.
pub fn decode(bytes: &[u8]) -> Result<World> {
    let inflated;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::with_capacity(bytes.len() * 4);
        flate2::read::MultiGzDecoder::new(bytes)
            .read_to_end(&mut out)
            .context("inflating the world chart")?;
        inflated = out;
        &inflated[..]
    } else {
        bytes
    };
    ensure!(
        bytes.len() >= 8 && &bytes[..4] == MAGIC,
        "not a world chart file"
    );
    let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let header_end = 8usize
        .checked_add(length)
        .filter(|&end| end <= bytes.len())
        .context("the world chart's header is cut short")?;
    let header: Header =
        serde_json::from_slice(&bytes[8..header_end]).context("the world chart's header")?;
    ensure!(
        header.version == VERSION,
        "world chart version {} is not {VERSION}",
        header.version
    );
    ensure!(
        header.quantum.is_finite() && header.quantum > 0.0,
        "the world chart's quantum"
    );
    let mut at = header_end;
    let mut varint = || -> Result<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let b = *bytes.get(at).context("the world chart is cut short")?;
            at += 1;
            ensure!(shift < 63, "a world chart number is too long");
            v |= u64::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
    };
    let unzig = |v: u64| -> i64 {
        if v % 2 == 1 {
            -((v as i64 + 1) / 2)
        } else {
            (v / 2) as i64
        }
    };
    let mut layers = vec![Vec::new(); LAYERS.len()];
    let mut lines = vec![false; LAYERS.len()];
    for layer in &header.layers {
        let mut rings = Vec::with_capacity(layer.count);
        for _ in 0..layer.count {
            let head = varint()?;
            let n = (head / 2) as usize;
            ensure!(n <= bytes.len(), "a world chart ring is too long");
            let (mut x, mut y) = (0i64, 0i64);
            let mut coords = Vec::with_capacity(n);
            for _ in 0..n {
                x += unzig(varint()?);
                y += unzig(varint()?);
                coords.push([x as f64 * header.quantum, y as f64 * header.quantum]);
            }
            rings.push(WorldRing {
                coords,
                outer: head % 2 == 1,
            });
        }
        if let Some(i) = LAYERS.iter().position(|l| *l == layer.name) {
            layers[i] = rings;
            lines[i] = layer.kind == "lines";
        }
    }
    ensure!(at == bytes.len(), "the world chart file has trailing bytes");
    Ok(World { layers, lines })
}

/// A tile's index from a longitude and latitude.
pub fn tile_of([lon, lat]: [f64; 2]) -> u16 {
    let col = (((lon + 180.0) / TILE).floor() as isize).clamp(0, TILE_COLUMNS as isize - 1);
    let row = (((lat + 90.0) / TILE).floor() as isize).clamp(0, TILE_ROWS as isize - 1);
    (row as usize * TILE_COLUMNS + col as usize) as u16
}

/// A tile's centre on the sphere and the angle (radians) from it to its farthest corner.
pub fn tile_cap(tile: u16) -> (V3, f64) {
    let (row, col) = (tile as usize / TILE_COLUMNS, tile as usize % TILE_COLUMNS);
    let lon0 = -180.0 + col as f64 * TILE;
    let lat0 = -90.0 + row as f64 * TILE;
    let centre = unit(lat0 + TILE / 2.0, lon0 + TILE / 2.0);
    let radius = [
        (lat0, lon0),
        (lat0, lon0 + TILE),
        (lat0 + TILE, lon0),
        (lat0 + TILE, lon0 + TILE),
        (lat0 + TILE / 2.0, lon0),
    ]
    .iter()
    .map(|&(la, lo)| dot(centre, unit(la, lo)).clamp(-1.0, 1.0).acos())
    .fold(0.0, f64::max);
    (centre, radius)
}

/// A run of a mesh's indices (or a line mesh's segments) in one tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRange {
    pub tile: u16,
    pub start: u32,
    pub end: u32,
}

/// Triangles on the sphere: unit vectors and indices, sorted by tile.
#[derive(Clone, Debug, Default)]
pub struct FillMesh {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub tiles: Vec<TileRange>,
}

/// Segments on the sphere (two unit vectors each), sorted by tile.
#[derive(Clone, Debug, Default)]
pub struct LineMesh {
    pub segments: Vec<[f32; 6]>,
    pub tiles: Vec<TileRange>,
}

/// One level of detail's layers.
#[derive(Clone, Debug, Default)]
pub struct Lod {
    /// Deeper than 200, 1,000, 3,000 and 4,000 m, shallow first.
    pub depth: [FillMesh; 4],
    pub land: FillMesh,
    pub coast: LineMesh,
    pub reefs: LineMesh,
}

/// The world's meshes.
#[derive(Clone, Debug, Default)]
pub struct WorldMeshes {
    pub near: Lod,
    pub far: Lod,
    /// Every 10°, and every 30°.
    pub graticule: [LineMesh; 2],
}

fn f32s(v: V3) -> [f32; 3] {
    [v[0] as f32, v[1] as f32, v[2] as f32]
}

/// A closed ring cut to one side of `x[axis] = at`: `below` keeps the side under it (Sutherland–Hodgman).
fn clip_ring(ring: &[[f64; 2]], axis: usize, at: f64, below: bool) -> Vec<[f64; 2]> {
    let inside = |p: &[f64; 2]| if below { p[axis] <= at } else { p[axis] >= at };
    let mut out = Vec::with_capacity(ring.len() / 2 + 4);
    let Some(&last) = ring.last() else {
        return out;
    };
    let mut a = last;
    for &b in ring {
        let (ia, ib) = (inside(&a), inside(&b));
        if ia != ib {
            let t = (at - a[axis]) / (b[axis] - a[axis]);
            let mut p = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            p[axis] = at;
            out.push(p);
        }
        if ib {
            out.push(b);
        }
        a = b;
    }
    out
}

fn bounds(ring: &[[f64; 2]]) -> [f64; 4] {
    ring.iter().fold(
        [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ],
        |b, p| {
            [
                b[0].min(p[0]),
                b[1].min(p[1]),
                b[2].max(p[0]),
                b[3].max(p[1]),
            ]
        },
    )
}

/// Rings of points, as cut.
pub type Rings = [Vec<[f64; 2]>];

/// Cuts `rings` (inside `rect`: x0, y0, x1, y1 on the grid of `cell`) into cells, halving along grid lines; each cell
/// with anything in it goes to `out`. The world's are degrees ([`CELL`]); a globe scene's fills are metres
/// (`maps.rs`).
pub fn cut(
    rings: Vec<Vec<[f64; 2]>>,
    rect: [f64; 4],
    cell: f64,
    out: &mut dyn FnMut([f64; 4], &Rings),
) {
    if rings.is_empty() {
        return;
    }
    let (w, h) = (rect[2] - rect[0], rect[3] - rect[1]);
    let slack = cell * 1e-9;
    let (wide, tall) = (w > cell + slack, h > cell + slack);
    if !wide && !tall {
        out(rect, &rings);
        return;
    }
    let axis = if wide && (w >= h || !tall) { 0 } else { 1 };
    let n = ((rect[axis + 2] - rect[axis]) / cell).round();
    let at = rect[axis] + (n / 2.0).floor() * cell;
    let (mut low, mut high) = (Vec::new(), Vec::new());
    for ring in rings {
        let b = bounds(&ring);
        if b[axis + 2] <= at {
            low.push(ring);
        } else if b[axis] >= at {
            high.push(ring);
        } else {
            let l = clip_ring(&ring, axis, at, true);
            if l.len() >= 3 {
                low.push(l);
            }
            let r = clip_ring(&ring, axis, at, false);
            if r.len() >= 3 {
                high.push(r);
            }
        }
    }
    let (mut lo_rect, mut hi_rect) = (rect, rect);
    lo_rect[axis + 2] = at;
    hi_rect[axis] = at;
    cut(low, lo_rect, cell, out);
    cut(high, hi_rect, cell, out);
}

/// A layer's closed rings filled (even-odd) as a mesh on the sphere.
pub fn fill(rings: &[Vec<[f64; 2]>]) -> FillMesh {
    let mut tiles: Vec<(u16, Vec<[f32; 3]>, Vec<u32>)> = Vec::new();
    let mut tessellator = FillTessellator::new();
    let mut geometry = VertexBuffers::<[f32; 2], u32>::new();
    let input: Vec<Vec<[f64; 2]>> = rings.iter().filter(|r| r.len() >= 3).cloned().collect();
    cut(
        input,
        [-180.0, -90.0, 180.0, 90.0],
        CELL,
        &mut |rect, cell| {
            // In the cell's own degrees, so 32-bit floats keep centimetres.
            let origin = [rect[0], rect[1]];
            let mut b = Path::builder();
            for ring in cell {
                b.begin(point(
                    (ring[0][0] - origin[0]) as f32,
                    (ring[0][1] - origin[1]) as f32,
                ));
                for p in &ring[1..] {
                    b.line_to(point((p[0] - origin[0]) as f32, (p[1] - origin[1]) as f32));
                }
                b.end(true);
            }
            geometry.vertices.clear();
            geometry.indices.clear();
            if tessellator
                .tessellate_path(
                    &b.build(),
                    &FillOptions::default().with_fill_rule(FillRule::EvenOdd),
                    &mut BuffersBuilder::new(&mut geometry, |v: FillVertex<'_>| {
                        v.position().to_array()
                    }),
                )
                .is_err()
                || geometry.indices.is_empty()
            {
                return;
            }
            let tile = tile_of([(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0]);
            let at = match tiles.iter().position(|t| t.0 == tile) {
                Some(at) => at,
                None => {
                    tiles.push((tile, Vec::new(), Vec::new()));
                    tiles.len() - 1
                }
            };
            let (_, vertices, indices) = &mut tiles[at];
            let base = vertices.len() as u32;
            vertices.extend(geometry.vertices.iter().map(|p| {
                f32s(unit(
                    f64::from(p[1]) + origin[1],
                    f64::from(p[0]) + origin[0],
                ))
            }));
            indices.extend(geometry.indices.iter().map(|i| base + i));
        },
    );
    tiles.sort_by_key(|t| t.0);
    let mut mesh = FillMesh::default();
    for (tile, vertices, indices) in tiles {
        let base = mesh.vertices.len() as u32;
        let start = mesh.indices.len() as u32;
        mesh.vertices.extend(vertices);
        mesh.indices.extend(indices.iter().map(|i| base + i));
        mesh.tiles.push(TileRange {
            tile,
            start,
            end: mesh.indices.len() as u32,
        });
    }
    mesh
}

/// Whether an edge is a seam of the source rather than a coast: along the antimeridian (±180° are one meridian, so an
/// edge from one to the other is the source's cut too, as Antarctica's `[180, -90] → [-180, -89.5]`) or the South
/// Pole's parallel.
fn seam(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0].abs() >= 179.9999 && b[0].abs() >= 179.9999) || (a[1] <= -89.99 && b[1] <= -89.99)
}

/// Lines (each a run of edges; `closed` adds the last to the first, seams left out) as segments on the sphere.
pub fn lines<'a>(runs: impl Iterator<Item = &'a [[f64; 2]]>, closed: bool) -> LineMesh {
    let mut tiles: Vec<Vec<[f32; 6]>> = vec![Vec::new(); TILE_COLUMNS * TILE_ROWS];
    for run in runs {
        let n = run.len();
        if n < 2 {
            continue;
        }
        let edges = if closed { n } else { n - 1 };
        for i in 0..edges {
            let (a, b) = (run[i], run[(i + 1) % n]);
            if closed && seam(a, b) {
                continue;
            }
            // The short way round: an edge across the antimeridian does not sweep the other 340°.
            let d = [wrap(b[0] - a[0]), b[1] - a[1]];
            let pieces = (d[0].hypot(d[1]) / SEGMENT).ceil().max(1.0) as usize;
            let mut from = unit(a[1], a[0]);
            for k in 1..=pieces {
                let t = k as f64 / pieces as f64;
                let p = [a[0] + d[0] * t, a[1] + d[1] * t];
                let to = unit(p[1], p[0]);
                let mid = [
                    wrap(a[0] + d[0] * (t - 0.5 / pieces as f64)),
                    a[1] + d[1] * (t - 0.5 / pieces as f64),
                ];
                let [f, g] = [f32s(from), f32s(to)];
                tiles[tile_of(mid) as usize].push([f[0], f[1], f[2], g[0], g[1], g[2]]);
                from = to;
            }
        }
    }
    let mut mesh = LineMesh::default();
    for (tile, segments) in tiles.into_iter().enumerate() {
        if segments.is_empty() {
            continue;
        }
        let start = mesh.segments.len() as u32;
        mesh.segments.extend(segments);
        mesh.tiles.push(TileRange {
            tile: tile as u16,
            start,
            end: mesh.segments.len() as u32,
        });
    }
    mesh
}

/// A ring thinned to points at least `step` degrees apart (longitude by its parallel's length); none left under four.
fn thinned(ring: &[[f64; 2]], step: f64) -> Option<Vec<[f64; 2]>> {
    let mut out = Vec::with_capacity(ring.len() / 4 + 4);
    for &p in ring {
        if let Some(&last) = out.last() {
            let [lon, lat]: [f64; 2] = last;
            let dx = (p[0] - lon) * (lat.to_radians().cos());
            if dx.hypot(p[1] - lat) < step {
                continue;
            }
        }
        out.push(p);
    }
    (out.len() >= 4).then_some(out)
}

/// Parallels and meridians every `step` degrees, a degree a segment: parallels to ±80°, meridians from −80° to 80°.
fn graticule(step: f64) -> LineMesh {
    let mut runs: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut lat = -90.0 + step;
    while lat < 90.0 - 1e-9 {
        if lat.abs() <= 80.0 + 1e-9 {
            runs.push((0..=360).map(|i| [-180.0 + f64::from(i), lat]).collect());
        }
        lat += step;
    }
    let reach = if step >= 30.0 { 60.0 } else { 80.0 };
    let mut lon = -180.0;
    while lon < 180.0 - 1e-9 {
        let n = (2.0 * reach) as i32;
        runs.push((0..=n).map(|i| [lon, -reach + f64::from(i)]).collect());
        lon += step;
    }
    lines(runs.iter().map(|r| r.as_slice()), false)
}

/// Every mesh of the world.
pub fn meshes(world: &World) -> WorldMeshes {
    let closed = |name: &str| -> Vec<Vec<[f64; 2]>> {
        world.layer(name).iter().map(|r| r.coords.clone()).collect()
    };
    let depth_names = [
        "bathymetry-200",
        "bathymetry-1000",
        "bathymetry-3000",
        "bathymetry-4000",
    ];
    let near_depth = depth_names.map(|name| fill(&closed(name)));
    let far_depth = depth_names.map(|name| {
        let rings: Vec<_> = world
            .layer(name)
            .iter()
            .filter_map(|r| thinned(&r.coords, FAR_STEP))
            .collect();
        fill(&rings)
    });
    let land = world.layer("land");
    let overview = world.layer("overview");
    let reefs = world.layer("reefs");
    let reefs_closed = !world.lines[1];
    WorldMeshes {
        near: Lod {
            depth: near_depth,
            land: fill(&closed("land")),
            coast: lines(land.iter().map(|r| r.coords.as_slice()), true),
            reefs: lines(reefs.iter().map(|r| r.coords.as_slice()), reefs_closed),
        },
        far: Lod {
            depth: far_depth,
            land: fill(&closed("overview")),
            coast: lines(overview.iter().map(|r| r.coords.as_slice()), true),
            reefs: LineMesh::default(),
        },
        graticule: [graticule(10.0), graticule(30.0)],
    }
}

/// The world as the page sent it, and its meshes once they are made.
pub struct WorldStore {
    pub version: String,
    pending: Option<mpsc::Receiver<WorldMeshes>>,
    meshes: Option<Arc<WorldMeshes>>,
    /// A number for each set of meshes, which the GPU keys its buffers by.
    pub id: u64,
}

impl WorldStore {
    /// Decodes `data` (base64 of the file as fetched) now, so a file that does not decode is refused with the reason,
    /// and makes its meshes on a thread.
    pub fn new(version: String, data: &str) -> Result<Self> {
        Self::from_bytes(version, &unbase64(data)?)
    }

    /// [`WorldStore::new`] of the file's bytes as read (gzip or not).
    pub fn from_bytes(version: String, bytes: &[u8]) -> Result<Self> {
        let world = decode(bytes)?;
        ensure!(world.points() > 0, "the world chart is empty");
        let (send, receive) = mpsc::channel();
        std::thread::Builder::new()
            .name("world chart meshes".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let made = meshes(&world);
                eprintln!(
                    "naval-globe: world chart: {} points made into meshes in {:.0} ms",
                    world.points(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
                let _ = send.send(made);
            })
            .context("starting the world chart's thread")?;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Ok(Self {
            version,
            pending: Some(receive),
            meshes: None,
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        })
    }

    /// The meshes, once made.
    pub fn meshes(&mut self) -> Option<Arc<WorldMeshes>> {
        if let Some(receiver) = &self.pending {
            match receiver.try_recv() {
                Ok(meshes) => {
                    self.meshes = Some(Arc::new(meshes));
                    self.pending = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.pending = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.meshes.clone()
    }

    /// Waits for the meshes (tests and the harness).
    pub fn settle(&mut self) -> Option<Arc<WorldMeshes>> {
        for _ in 0..6000 {
            if self.pending.is_none() {
                break;
            }
            if let Some(meshes) = self.meshes() {
                return Some(meshes);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.meshes()
    }
}

/// A layer to encode: its name in [`LAYERS`], its kind (`rings` or `lines`), its rings or lines (longitude, latitude
/// degrees) and whether each ring is outer.
pub type Layer<'a> = (&'a str, &'a str, Vec<(Vec<[f64; 2]>, bool)>);

/// A world file in the page's format (`encodeWorld`), uncompressed, from each layer's rings at a quantum of 1e-5°
/// (a layer not given is empty): a host's own chart, or a test's.
pub fn encode(layers: &[Layer<'_>]) -> Vec<u8> {
    let quantum = 1e-5;
    let mut out = Vec::new();
    let varint = |out: &mut Vec<u8>, mut v: u64| {
        while v >= 0x80 {
            out.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    };
    let zigzag = |v: i64| {
        if v >= 0 {
            (v * 2) as u64
        } else {
            (-v * 2 - 1) as u64
        }
    };
    let mut described = Vec::new();
    let none = Vec::new();
    for name in LAYERS {
        let (kind, rings) = layers
            .iter()
            .find(|l| l.0 == name)
            .map_or(("rings", &none), |l| (l.1, &l.2));
        for (ring, outer) in rings {
            varint(&mut out, ring.len() as u64 * 2 + u64::from(*outer));
            let (mut px, mut py) = (0i64, 0i64);
            for &[lon, lat] in ring {
                let (x, y) = (
                    (lon / quantum).round() as i64,
                    (lat / quantum).round() as i64,
                );
                varint(&mut out, zigzag(x - px));
                varint(&mut out, zigzag(y - py));
                px = x;
                py = y;
            }
        }
        described.push(
            serde_json::json!({"name": name, "kind": kind, "count": rings.len(),
            "points": rings.iter().map(|r| r.0.len()).sum::<usize>()}),
        );
    }
    let header = serde_json::json!({"version": VERSION, "quantum": quantum, "layers": described,
        "places": [], "source": "naval-globe"});
    let json = serde_json::to_vec(&header).expect("a JSON value");
    let mut bytes = MAGIC.to_vec();
    bytes.extend((json.len() as u32).to_le_bytes());
    bytes.extend(json);
    bytes.extend(out);
    bytes
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A ring of `half` degrees about a longitude and latitude.
    pub fn square(lon: f64, lat: f64, half: f64) -> Vec<[f64; 2]> {
        vec![
            [lon - half, lat - half],
            [lon + half, lat - half],
            [lon + half, lat + half],
            [lon - half, lat + half],
        ]
    }

    #[test]
    fn the_pages_format_decodes_gzipped_or_not() {
        let bytes = encode(&[
            ("land", "rings", vec![(square(20.0, 40.0, 3.0), true)]),
            (
                "reefs",
                "lines",
                vec![(vec![[150.0, -10.0], [151.0, -10.5]], false)],
            ),
        ]);
        let world = decode(&bytes).unwrap();
        assert_eq!(world.layer("land").len(), 1);
        assert!(world.layer("land")[0].outer);
        assert!((world.layer("land")[0].coords[2][0] - 23.0).abs() < 1e-9);
        assert!(world.lines[1] && !world.lines[0]);
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &bytes).unwrap();
        let zipped = gz.finish().unwrap();
        assert_eq!(decode(&zipped).unwrap().points(), world.points());
        // Base64 as the page sends it.
        let text = naval_region_bake::ntf::base64(&zipped);
        assert_eq!(unbase64(&text).unwrap(), zipped);
        // Damaged files are refused with the reason.
        assert!(decode(b"NOPE1234").is_err());
        let mut long = bytes.clone();
        long.push(0);
        assert!(decode(&long).unwrap_err().to_string().contains("trailing"));
        assert!(decode(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn fills_are_cut_into_cells_with_no_long_triangle() {
        // A ring across many cells with a hole: every triangle within a cell, the hole left out.
        let mut outer = square(10.0, 20.0, 12.0);
        outer.reverse();
        let hole = square(10.0, 20.0, 2.0);
        let mesh = fill(&[outer, hole]);
        assert!(!mesh.indices.is_empty());
        let lat_lon = |v: [f32; 3]| {
            crate::globe::lat_lon([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])])
        };
        let mut area = 0.0_f64;
        for t in mesh.indices.chunks(3) {
            let p: [[f64; 2]; 3] = std::array::from_fn(|k| lat_lon(mesh.vertices[t[k] as usize]));
            for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                // Within a cell's diagonal, give or take the 32-bit unit vectors' rounding.
                let edge = (a[0] - b[0]).hypot(a[1] - b[1]);
                assert!(edge <= CELL * 2f64.sqrt() + 1e-4, "{edge}");
            }
            area += ((p[1][1] - p[0][1]) * (p[2][0] - p[0][0])
                - (p[2][1] - p[0][1]) * (p[1][0] - p[0][0]))
                .abs()
                / 2.0;
            // Nothing inside the hole.
            let c = [
                (p[0][0] + p[1][0] + p[2][0]) / 3.0,
                (p[0][1] + p[1][1] + p[2][1]) / 3.0,
            ];
            assert!(!((18.0..22.0).contains(&c[0]) && (8.0..12.0).contains(&c[1])));
        }
        assert!((area - (24.0 * 24.0 - 16.0)).abs() < 0.01, "{area}");
        // Sorted by tile, every index in its tile's range.
        assert!(
            mesh.tiles
                .windows(2)
                .all(|w| w[0].tile < w[1].tile && w[0].end == w[1].start)
        );
    }

    #[test]
    fn coasts_leave_out_the_sources_seams() {
        // A ring cut at the antimeridian and the South Pole's parallel, as Natural Earth cuts Antarctica.
        let ring = vec![
            [170.0, -70.0],
            [180.0, -72.0],
            [180.0, -90.0],
            [170.0, -90.0],
        ];
        let mesh = lines(std::iter::once(ring.as_slice()), true);
        // 170,-70 → 180,-72 (11 pieces of a degree or less) and 170,-90 → 170,-70 (20); the seams none.
        assert_eq!(mesh.segments.len(), 11 + 20);
        // ±180° are one meridian: the bundled Antarctica's 180,-90 → -180,-89.5 is a seam, not a 360° spiral, and an
        // edge across the antimeridian goes the short way (-170,-70 → 170,-70 is 20°, 20 pieces).
        let ring = vec![
            [170.0, -70.0],
            [180.0, -72.0],
            [180.0, -90.0],
            [-180.0, -89.5],
            [-180.0, -72.0],
            [-170.0, -70.0],
        ];
        let mesh = lines(std::iter::once(ring.as_slice()), true);
        assert_eq!(mesh.segments.len(), 11 + 11 + 20);
        assert!(
            mesh.segments
                .iter()
                .all(|s| (s[0] - s[3]).hypot(s[1] - s[4]).hypot(s[2] - s[5]) < 0.02),
            "no piece longer than about a degree"
        );
    }
}
