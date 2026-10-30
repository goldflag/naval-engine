//! The far level ([`Lod::Far`], native/docs/terrain/buildings.md, "The far level"): a town past the boxes' range as
//! coarse masses, made from the window's land cover alone (no footprints, nothing fetched), so a city reads as a town of
//! the right grain and colour to the far level's range rather than ending where the boxes do.
//!
//! - **The streets.** The infill's own street grid ([`infill`]): the theatre cut into districts of
//!   [`infill::DISTRICT_M`], each with its street grid at its hashed heading and offset, its blocks of the region's
//!   size, two rows of lots back to back. The same districts, blocks and dense cores as the infill's, so where the
//!   infill stands the far level is its own blocks seen from afar.
//! - **The masses.** Each row is cut into runs of a few lots ([`RUN_M`]; longer where the blocks are small, no more than
//!   [`MOST_RUNS_KM2`] to a square kilometre), and each run on built-up land (ESA
//!   WorldCover's class 50 under it) is one mass: as long as the share of its lots built on (fewer where the town thins),
//!   as deep and as high as the lots' buildings (houses, terraces or a core's blocks by the region's rules, the infill's),
//!   set back from the street as they are, under a gabled roof at the region's pitch or flat as the region's are, in the
//!   region's colours ([`palette`]). A past period's stands only on the ground that stood (the host's `stood`), its
//!   storeys and heights capped as the period's are.
//! - **The blocks.** Farther off a block's runs give way to one mass over them all ([`Mass::BLOCK`]; over a group of
//!   East Asia's small blocks, [`GROUP_M`]): flat at their roofs' mean height, in its longest run's colours. The chunk's runs come first in its masses, its blocks after
//!   ([`ChunkMesh::far_runs`]), so the layer draws only the runs, only the blocks or both by the chunk's distance and the
//!   shader picks each block's by its own (`vs_far`).
//! - **Each 24 bytes** ([`Mass`]), a few hundred to a chunk (an instance each of one gabled box, `buildings.wgsl`'s
//!   `vs_far`): a fully built-up chunk about 19-27 KB.
//! - **The towers.** A mapped building standing [`TOWER_M`] or more (OSM's measured height, its parts stacked as they
//!   are mapped: [`FarTowers`]) stands as itself, the boxes' own mesh of it ([`Lod::Box`], `build_chunk`), so a
//!   city's skyline carries on past the boxes and a tower is the same on both sides of the crossover; no mass stands
//!   on its footprint, and a block's mass that would stand over one keeps its runs, flat, in its place.
//!
//! Every choice hashes the run's place in the theatre, so a chunk made again holds the same masses.

use std::collections::BTreeMap;

use half::f16;

use super::clutter::ShapeVertex;
use super::infill::{self, BUILT_UP, DISTRICT_M, Grid, MAX_SLOPE};
use super::mesh::{ChunkInput, ChunkMesh, SINK_M, build_chunk};
use super::palette::{colours, roof_rules};
use super::tags::MapTags;
use super::thinning::{Thinning, period_storeys};
use super::{
    CHUNK_M, ChunkKey, Footprint, Kind, Lod, Period, Region, geometry, hash01, storey_height,
};

/// A run of lots one mass stands for, about this long (m): two or three houses, a terrace's stretch, one or two of a
/// core's blocks.
pub const RUN_M: f64 = 30.0;

/// The least share of the ground built up about a block for its masses (the infill's).
pub const LEAST_SHARE: f32 = 0.12;

/// The most runs to a square kilometre of town: a grid of small blocks (East Asia's) runs whole rows, so its masses
/// cost no more to draw than another town's.
pub const MOST_RUNS_KM2: f64 = 1000.0;

/// A block's mass stands over a group of blocks about this long along their street and deep across it (m): one block
/// in most towns, two by two of East Asia's small ones.
pub const GROUP_M: [f64; 2] = [150.0, 90.0];

/// A mapped building this tall or taller (m, to its top) stands in the far level as itself ([`FarTowers`]): just over
/// the tallest the masses and the infill stand (a dense core's rare tower of 18 storeys, 56 m), so what stands taller
/// is a tower the masses could not have drawn; aviation's lit obstruction (about 200 ft, `night::beacons_of`), so a far
/// tower carries its lights at night. Taipei's far ring (10-30 km from Taipei 101) holds 108 chunks with towers, 710
/// buildings and parts (their lower parts with them), 12,000 triangles: 0.7 MB on the GPU, 30 KB a chunk at most.
pub const TOWER_M: f32 = 60.0;

/// One mass, 24 bytes (`buildings.wgsl`'s `FarIn`): a gabled box (flat when its rise is 0) along its street.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Mass {
    /// Its foot's height above the sea (m): [`SINK_M`] below the lowest ground under it.
    pub foot: f32,
    /// f16: its middle's chunk-local x and z (m), its length along its street and its depth across it (m).
    pub place: [u16; 4],
    /// f16: its eaves over the ground under it (m), its ridge's rise over the eaves (m; 0 a flat roof).
    pub height: [u16; 2],
    /// The walls' palette entry, the roof's, its block's hash (the shader's tint, and where the block's runs give way to
    /// it), its [`Kind`] (bits 0-3) and [`Mass::BLOCK`].
    pub style: [u8; 4],
    /// Its heading (bits 0-7, its street's direction in 256ths of a half turn), how far the ground stands over its foot
    /// (bits 8-15, quarter metres); the layer writes its chunk's slot in bits 16-31.
    pub turn: u32,
}

impl Mass {
    /// Set in `style[3]` on a block's mass (`buildings.wgsl`'s `vs_far` reads bit 4).
    pub const BLOCK: u8 = 16;

    /// Whether it is a block's mass, not a run's.
    pub fn is_block(&self) -> bool {
        self.style[3] & Self::BLOCK != 0
    }

    /// Chunk-local x and z of its middle (m).
    pub fn at(&self) -> [f32; 2] {
        [
            f16::from_bits(self.place[0]).to_f32(),
            f16::from_bits(self.place[1]).to_f32(),
        ]
    }

    /// Its length along its street and depth (m).
    pub fn size(&self) -> [f32; 2] {
        [
            f16::from_bits(self.place[2]).to_f32(),
            f16::from_bits(self.place[3]).to_f32(),
        ]
    }

    /// Its eaves over the ground and its ridge's rise over them (m).
    pub fn heights(&self) -> [f32; 2] {
        [
            f16::from_bits(self.height[0]).to_f32(),
            f16::from_bits(self.height[1]).to_f32(),
        ]
    }

    /// The ground under it (m above the sea).
    pub fn ground(&self) -> f32 {
        self.foot + ((self.turn >> 8) & 255) as f32 * 0.25
    }

    /// Its heading (rad, 0 to π): its street's direction from +x toward +z.
    pub fn heading(&self) -> f32 {
        (self.turn & 255) as f32 / 256.0 * std::f32::consts::PI
    }

    pub fn kind(&self) -> u8 {
        self.style[3] & 15
    }

    /// Its top (m above the sea).
    pub fn top(&self) -> f32 {
        let [eaves, rise] = self.heights();
        self.ground() + eaves + rise
    }
}

/// What [`build_far`] makes a chunk's masses from.
pub struct FarInput<'a> {
    pub index: [i32; 2],
    /// The cover, the share built up about a point and the ground (the infill's).
    pub land: &'a dyn infill::Land,
    /// Whether the ground at a theatre point stood in the period (always, today).
    pub stood: &'a dyn Fn([f64; 2]) -> bool,
    /// The region at a theatre point (asked of each district's middle, as the infill asks).
    pub region: &'a dyn Fn([f64; 2]) -> Region,
    /// The chunk's region (its translation's, for the facades).
    pub chunk_region: Region,
    pub period: Period,
    /// The most a mass stands in a past period (m, `thinning::height_cap` of the chunk's place).
    pub cap_m: f32,
    /// The map's buildings about the chunk, its towers among them (none: masses alone).
    pub towers: Option<FarTowers<'a>>,
}

/// The map's measured buildings about a far chunk ([`FarInput::towers`]): those standing [`TOWER_M`] or more
/// ([`towers`]) stand as their boxes.
pub struct FarTowers<'a> {
    /// Those whose middles lie in the chunk, OSM's outlines with their parts (`tags::merge_outlines`).
    pub footprints: &'a [Footprint],
    /// Those whose middles lie about it (within the map's reach), as their own footprints: no mass of the chunk's
    /// stands under one.
    pub about: &'a [Footprint],
    /// The map about the chunk: the towers' land (an office's on commercial land) and the areas the host draws itself
    /// (`MapTags::cleared`).
    pub tags: &'a MapTags,
    /// The ground at a chunk-local point (m above the sea), as the near levels' meshes stand on it.
    pub ground: &'a dyn Fn([f32; 2]) -> f32,
    /// In a past period, the place's cap and the listed buildings ([`Thinning::cap_for`], [`Thinning::later`]); the
    /// ground that stood is [`FarInput::stood`]'s.
    pub thinning: Option<&'a Thinning>,
}

/// Which of `t`'s footprints, the chunk's own and those about it, stand as towers in the far level (chunk `index`, under
/// `input`'s period and ground): those of a building any of whose parts (a whole footprint its own building) stands
/// [`TOWER_M`] or more on ground that stood and outside the host's areas; in a past period at most its place's cap (or
/// what one of `thinning::TALL` on it stood), none built after it. All of a building's parts or none, as the near levels
/// keep them, judged by the whole building on both sides of the chunk's edge (its parts share their building's middle,
/// [`Part::middle`](super::Part::middle)): a podium here under a shaft whose own middle lies in the next chunk stands
/// with it, as their boxes did.
pub fn towers(input: &FarInput, t: &FarTowers) -> [Vec<bool>; 2] {
    let corner = [
        f64::from(input.index[0]) * CHUNK_M,
        f64::from(input.index[1]) * CHUNK_M,
    ];
    let thin = t.thinning.filter(|_| input.period.is_past());
    let middle = |f: &Footprint| {
        f.part
            .map_or_else(|| geometry::centroid(&f.ring), |p| p.middle)
    };
    let tall: Vec<[f32; 2]> = t
        .footprints
        .iter()
        .chain(t.about)
        .filter(|f| f.ring.len() >= 3)
        .filter_map(|f| {
            let c = middle(f);
            let mut height = f.height?;
            if t.tags.cleared_at(c)
                || !(input.stood)([corner[0] + f64::from(c[0]), corner[1] + f64::from(c[1])])
            {
                return None;
            }
            if let Some(th) = thin {
                if th.later(&f.ring) || th.later_at(c) {
                    return None;
                }
                height = height.min(th.cap_for(&f.ring));
            }
            (height >= TOWER_M).then_some(c)
        })
        .collect();
    [t.footprints, t.about].map(|footprints| {
        footprints
            .iter()
            .map(|f| f.ring.len() >= 3 && tall.contains(&middle(f)))
            .collect()
    })
}

/// A tower's footprint where no mass stands: its box (min x, min z, max x, max z) and ring, chunk-local.
type Kept = ([f32; 4], Vec<[f32; 2]>);

/// The chunk's masses as a mesh: its [`ChunkMesh::masses`], its towers' boxes (its vertices: [`FarTowers`]), its bounds
/// and region.
pub fn build_far(input: &FarInput) -> ChunkMesh {
    let index = input.index;
    let corner = [f64::from(index[0]) * CHUNK_M, f64::from(index[1]) * CHUNK_M];
    let mut mesh = ChunkMesh {
        region: input.chunk_region.in_period(input.period).index(),
        period: input.period,
        ..ChunkMesh::default()
    };
    // The towers: the chunk's own as the boxes make them, and theirs about it, where no mass stands.
    let mut kept: Vec<Kept> = Vec::new();
    let mut boxes = None;
    if let Some(t) = &input.towers {
        let [own_towers, about_towers] = towers(input, t);
        let own: Vec<Footprint> = t
            .footprints
            .iter()
            .zip(own_towers)
            .filter(|(_, tower)| *tower)
            .map(|(f, _)| f.clone())
            .collect();
        let about = t
            .about
            .iter()
            .zip(about_towers)
            .filter(|(_, tower)| *tower)
            .map(|(f, _)| f);
        kept = own
            .iter()
            .chain(about)
            .map(|f| (box_of(&f.ring), f.ring.clone()))
            .collect();
        if !own.is_empty() {
            let made = build_chunk(&ChunkInput {
                key: ChunkKey {
                    index,
                    lod: Lod::Box,
                },
                footprints: &own,
                infill: &[],
                tags: t.tags,
                ground: t.ground,
                region: input.chunk_region,
                period: input.period,
                thinning: t.thinning,
            });
            boxes = (!made.indices.is_empty()).then_some(made);
        }
    }
    // A quick look: no built-up land anywhere about the chunk, no masses.
    let any = (0..5).any(|j| {
        (0..5).any(|i| {
            let p = [
                corner[0] + (f64::from(i) + 0.5) * 200.0,
                corner[1] + (f64::from(j) + 0.5) * 200.0,
            ];
            input.land.built_share(p, 140.0) > 0.03
        })
    });
    if !any {
        with_towers(&mut mesh, boxes);
        return mesh;
    }
    let d0 = [
        (corner[0] / DISTRICT_M).floor() as i64,
        (corner[1] / DISTRICT_M).floor() as i64,
    ];
    let d1 = [
        ((corner[0] + CHUNK_M) / DISTRICT_M).floor() as i64,
        ((corner[1] + CHUNK_M) / DISTRICT_M).floor() as i64,
    ];
    let mut blocks = Vec::new();
    for dz in d0[1]..=d1[1] {
        for dx in d0[0]..=d1[0] {
            district(input, &kept, dx, dz, corner, &mut mesh.masses, &mut blocks);
        }
    }
    mesh.far_runs = mesh.masses.len() as u32;
    mesh.buildings = mesh.far_runs;
    mesh.masses.extend(blocks);
    bounds(&mut mesh);
    with_towers(&mut mesh, boxes);
    mesh
}

/// The towers' boxes into the far chunk's mesh: their vertices, their buildings counted, the bounds over both.
fn with_towers(mesh: &mut ChunkMesh, boxes: Option<ChunkMesh>) {
    let Some(boxes) = boxes else {
        return;
    };
    if mesh.masses.is_empty() {
        mesh.min = boxes.min;
        mesh.max = boxes.max;
    } else {
        for k in 0..3 {
            mesh.min[k] = mesh.min[k].min(boxes.min[k]);
            mesh.max[k] = mesh.max[k].max(boxes.max[k]);
        }
    }
    mesh.buildings += boxes.buildings;
    mesh.vertices = boxes.vertices;
    mesh.indices = boxes.indices;
}

/// A ring's box: min x, min z, max x, max z.
fn box_of(ring: &[[f32; 2]]) -> [f32; 4] {
    ring.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
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

/// Whether the rectangle `corners` (chunk-local, in order about it) meets one of the towers' footprints `kept`: their
/// edges cross ([`geometry::overlaps`]: a mass across a slim tower, neither's corner in the other), one holds the
/// other's corner, or the footprint holds the rectangle's middle.
fn on_tower(kept: &[Kept], corners: &[[f32; 2]; 4]) -> bool {
    if kept.is_empty() {
        return false;
    }
    let b = box_of(corners);
    let middle = [
        corners.iter().map(|c| c[0]).sum::<f32>() * 0.25,
        corners.iter().map(|c| c[1]).sum::<f32>() * 0.25,
    ];
    kept.iter().any(|(k, ring)| {
        k[0] <= b[2]
            && k[2] >= b[0]
            && k[1] <= b[3]
            && k[3] >= b[1]
            && (geometry::contains(ring, middle) || geometry::overlaps(ring, corners))
    })
}

/// A mass's corners (chunk-local), in order about it.
fn corners_of(m: &Mass) -> [[f32; 2]; 4] {
    let [x, z] = m.at();
    let [along, across] = m.size();
    let (s, c) = m.heading().sin_cos();
    let (a, b) = (along * 0.5, across * 0.5);
    [[-a, -b], [a, -b], [a, b], [-a, b]].map(|[u, v]| [x + u * c - v * s, z + u * s + v * c])
}

/// The chunk-local bounds of its masses.
fn bounds(mesh: &mut ChunkMesh) {
    if mesh.masses.is_empty() {
        return;
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for m in &mesh.masses {
        let [x, z] = m.at();
        let [along, across] = m.size();
        let r = 0.5 * along.hypot(across);
        min[0] = min[0].min(x - r);
        max[0] = max[0].max(x + r);
        min[2] = min[2].min(z - r);
        max[2] = max[2].max(z + r);
        min[1] = min[1].min(m.foot);
        max[1] = max[1].max(m.top());
    }
    mesh.min = min;
    mesh.max = max;
}

/// The storeys of a dense core's block in `region` (the infill's), by a hash `t`.
fn core_storeys(region: Region, t: f32) -> f32 {
    match region {
        Region::Tropical
        | Region::Arid
        | Region::American
        | Region::West
        | Region::Florida
        | Region::Iceland
        | Region::NorthAfrica
        | Region::Hawaii
        | Region::Micronesia
        | Region::Atoll => 2.0 + (t * 1.99).floor(),
        Region::British | Region::Celtic | Region::NorthSea | Region::Malta => {
            3.0 + (t * 2.99).floor()
        }
        Region::Aegean | Region::Nile => 4.0 + (t * 3.99).floor(),
        Region::Nordic | Region::Temperate | Region::Mediterranean | Region::EastAsia => {
            3.0 + (t * 3.99).floor()
        }
        // Taiwan's walk-ups of four and five.
        Region::Taiwan => 4.0 + (t * 1.99).floor(),
    }
}

/// Whether a block of `region` is a dense core's, its ground `share` built up and `t` its hash (the infill's rule).
fn dense_block(region: Region, share: f32, t: f32) -> bool {
    match region {
        Region::Tropical
        | Region::Arid
        | Region::NorthAfrica
        | Region::Micronesia
        | Region::Atoll => share > 0.9 && t < 0.25,
        Region::American | Region::Florida | Region::Hawaii => share > 0.85 && t < 0.35,
        Region::West => share > 0.92 && t < 0.12,
        Region::EastAsia => share > 0.8 && t < (share - 0.6) * 1.5,
        Region::Taiwan => share > 0.85 && t < (share - 0.7) * 1.2,
        Region::Nordic
        | Region::Temperate
        | Region::Mediterranean
        | Region::Iceland
        | Region::British
        | Region::Celtic
        | Region::NorthSea
        | Region::Malta
        | Region::Aegean
        | Region::Nile => share > 0.75 && t < (share - 0.6) * 1.6,
    }
}

/// What stands along a run of lots: its kind, its storeys, the share of a lot's width its buildings fill, their depth
/// (a share of the lot's) and their setback from the street (m); by the region's and the infill's rules.
fn run_building(region: Region, dense: bool, local: f32, hash: &dyn Fn(i64) -> f32) -> RunBuilding {
    // Taiwan's rows of townhouses filling their lots at the lane, deep in them, of three to five storeys (four and five
    // where the town is built over), a core's walk-ups of four and five (`infill::taiwanese`).
    if region == Region::Taiwan {
        let (kind, storeys, depth) = if dense {
            (
                Kind::Block,
                core_storeys(region, hash(3)),
                0.75 + 0.2 * f64::from(hash(4)),
            )
        } else if local > 0.6 {
            (
                Kind::Terrace,
                4.0 + (hash(3) * 1.99).floor(),
                0.65 + 0.3 * f64::from(hash(4)),
            )
        } else {
            (
                Kind::Terrace,
                3.0 + (hash(3) * 1.99).floor(),
                0.65 + 0.3 * f64::from(hash(4)),
            )
        };
        return RunBuilding {
            kind,
            storeys,
            fill: 0.97,
            depth,
            setback: 0.25,
        };
    }
    let terrace = match region {
        Region::West => local > 0.8 && hash(5) < 0.2,
        Region::EastAsia => local > 0.6 && hash(5) < 0.3,
        Region::British | Region::NorthSea => local > 0.3 && hash(5) < 0.85,
        Region::Celtic => local > 0.4 && hash(5) < 0.7,
        Region::Florida => local > 0.85 && hash(5) < 0.15,
        _ => local > 0.55 && hash(5) < 0.5,
    };
    if dense {
        let fill = if region == Region::EastAsia {
            0.82
        } else {
            0.92
        };
        return RunBuilding {
            kind: Kind::Block,
            storeys: core_storeys(region, hash(3)),
            fill,
            depth: 0.6 + 0.3 * f64::from(hash(4)),
            setback: 1.0,
        };
    }
    if terrace {
        let storeys = match region {
            Region::British | Region::Celtic | Region::NorthSea => {
                if hash(3) < 0.85 {
                    2.0
                } else {
                    3.0
                }
            }
            _ => 2.0 + (hash(3) * 1.99).floor(),
        };
        return RunBuilding {
            kind: Kind::Terrace,
            storeys,
            fill: 0.95,
            depth: 0.4 + 0.2 * f64::from(hash(4)),
            setback: 1.0,
        };
    }
    let (one, fill, depth, setback) = match region {
        Region::West => (0.65, 0.7, 0.38, 7.5),
        Region::Florida => (0.8, 0.7, 0.38, 7.5),
        Region::EastAsia => (0.15, 0.8, 0.58, 1.5),
        Region::British => (0.12, 0.67, 0.4, 4.5),
        Region::NorthSea => (0.3, 0.67, 0.4, 4.5),
        Region::Celtic => (0.4, 0.67, 0.4, 4.5),
        Region::Hawaii => (0.62, 0.67, 0.4, 4.5),
        Region::Tropical
        | Region::Arid
        | Region::NorthAfrica
        | Region::Micronesia
        | Region::Atoll => (1.0, 0.67, 0.4, 4.5),
        _ => (0.5, 0.67, 0.4, 4.5),
    };
    RunBuilding {
        kind: Kind::House,
        storeys: if hash(3) < one { 1.0 } else { 2.0 },
        fill,
        depth,
        setback,
    }
}

struct RunBuilding {
    kind: Kind,
    storeys: f32,
    fill: f64,
    depth: f64,
    setback: f64,
}

fn half(v: f32) -> u16 {
    f16::from_f32(v).to_bits()
}

/// The far shape's vertices and triangles (`buildings.wgsl`'s `vs_far` sizes, turns and roofs it per mass): a box from
/// -0.5 to 0.5 along (x) and across (z), y 0 at the foot and 1 at the eaves, under a ridge along x at y 2 over z 0. Its
/// four walls (normal.w 0), two roof slopes (normal.w 127, the normal's z their side) and two gable ends (normal.w 64),
/// in that order: 30 vertices, 14 triangles, every one counter-clockwise from outside. A flat mass's rise is 0: its
/// slopes fold to one flat top and its gables to nothing, so a block's (always flat) are drawn without them, the first
/// [`FLAT_INDICES`].
pub fn far_shape() -> (Vec<ShapeVertex>, Vec<u16>) {
    let mut vertices: Vec<ShapeVertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    let mut face = |points: &[[f32; 3]], normal: [i8; 4]| {
        let first = vertices.len() as u16;
        for p in points {
            vertices.push(ShapeVertex {
                position: *p,
                normal,
            });
        }
        for k in 1..points.len() as u16 - 1 {
            indices.extend_from_slice(&[first, first + k, first + k + 1]);
        }
    };
    let (x0, x1, z0, z1) = (-0.5, 0.5, -0.5, 0.5);
    face(
        &[[x0, 0.0, z1], [x1, 0.0, z1], [x1, 1.0, z1], [x0, 1.0, z1]],
        [0, 0, 127, 0],
    );
    face(
        &[[x1, 0.0, z0], [x0, 0.0, z0], [x0, 1.0, z0], [x1, 1.0, z0]],
        [0, 0, -127, 0],
    );
    face(
        &[[x1, 0.0, z1], [x1, 0.0, z0], [x1, 1.0, z0], [x1, 1.0, z1]],
        [127, 0, 0, 0],
    );
    face(
        &[[x0, 0.0, z0], [x0, 0.0, z1], [x0, 1.0, z1], [x0, 1.0, z0]],
        [-127, 0, 0, 0],
    );
    face(
        &[[x0, 1.0, z1], [x1, 1.0, z1], [x1, 2.0, 0.0], [x0, 2.0, 0.0]],
        [0, 90, 90, 127],
    );
    face(
        &[[x1, 1.0, z0], [x0, 1.0, z0], [x0, 2.0, 0.0], [x1, 2.0, 0.0]],
        [0, 90, -90, 127],
    );
    face(
        &[[x1, 1.0, z1], [x1, 1.0, z0], [x1, 2.0, 0.0]],
        [127, 0, 0, 64],
    );
    face(
        &[[x0, 1.0, z0], [x0, 1.0, z1], [x0, 2.0, 0.0]],
        [-127, 0, 0, 64],
    );
    (vertices, indices)
}

/// The far shape's indices without its gables (a flat mass's walls and top).
pub const FLAT_INDICES: u32 = 36;

fn district(
    input: &FarInput,
    kept: &[Kept],
    dx: i64,
    dz: i64,
    corner: [f64; 2],
    out: &mut Vec<Mass>,
    blocks: &mut Vec<Mass>,
) {
    let land = input.land;
    let lo = [dx as f64 * DISTRICT_M, dz as f64 * DISTRICT_M];
    let centre = [lo[0] + DISTRICT_M * 0.5, lo[1] + DISTRICT_M * 0.5];
    let region = (input.region)(centre);
    // The infill's street grid: the same heading, offset and blocks.
    let heading = f64::from(hash01(dx, dz, 101)) * std::f64::consts::FRAC_PI_2;
    let (s, c) = heading.sin_cos();
    let u = [c, s];
    let v = [-s, c];
    let grid = Grid::of(region);
    let shift = [
        f64::from(hash01(dx, dz, 102)) * grid.block[0],
        f64::from(hash01(dx, dz, 103)) * grid.block[1],
    ];
    let pitch = [grid.block[0] + grid.street, grid.block[1] + grid.street];
    let reach = DISTRICT_M * std::f64::consts::FRAC_1_SQRT_2;
    let (na, nb) = (
        (reach / pitch[0]).ceil() as i64 + 1,
        (reach / pitch[1]).ceil() as i64 + 1,
    );
    let turn = ((heading / std::f64::consts::PI * 256.0).round() as u32) & 255;
    let past = input.period.is_past();
    // Blocks to a group along and across, and the most runs to a row.
    let group = [
        (GROUP_M[0] / pitch[0]).round().max(1.0) as i64,
        (GROUP_M[1] / pitch[1]).round().max(1.0) as i64,
    ];
    let most_runs = ((MOST_RUNS_KM2 * pitch[0] * pitch[1] / 2.0e6).floor() as i64).max(1);
    // Each group's runs, from the district's middle along its streets and across them.
    let mut groups: BTreeMap<(i64, i64), Vec<RunMass>> = BTreeMap::new();
    for bj in -nb..=nb {
        for bi in -na..=na {
            let ba = bi as f64 * pitch[0] + shift[0];
            let bb = bj as f64 * pitch[1] + shift[1];
            let block_hash = |k: i64| hash01(dx * 131 + bi, dz * 137 + bj, k);
            let (gi, gj) = (bi.div_euclid(group[0]), bj.div_euclid(group[1]));
            let mid = [
                centre[0] + u[0] * ba + v[0] * bb,
                centre[1] + u[1] * ba + v[1] * bb,
            ];
            if (mid[0] - centre[0]).abs() > DISTRICT_M * 0.5 + pitch[0]
                || (mid[1] - centre[1]).abs() > DISTRICT_M * 0.5 + pitch[0]
            {
                continue;
            }
            let share = land.built_share(mid, 90.0);
            if share < LEAST_SHARE {
                continue;
            }
            let dense = dense_block(region, share, block_hash(1));
            let lot_w = if dense {
                18.0 + 10.0 * f64::from(block_hash(2))
            } else {
                grid.lot[0] + grid.lot[1] * f64::from(block_hash(2))
            };
            let depth = grid.block[1] * 0.5;
            let per_row = (grid.block[0] / lot_w).floor().max(1.0) as i64;
            let lot_w = grid.block[0] / per_row as f64;
            // Lots to a run: about RUN_M, never less than one, nor more runs to the row than `most_runs`.
            let per_run = ((RUN_M / lot_w).round() as i64)
                .max((per_row + most_runs - 1) / most_runs)
                .clamp(1, per_row);
            // The group's hash, shared by its runs and its mass (the shader's tint, and where the runs give way).
            let seed = (hash01(dx * 131 + gi, dz * 137 + gj, 5) * 255.0) as u8;
            let mut made: Vec<RunMass> = Vec::new();
            for row in [-1.0f64, 1.0] {
                let mut k0 = 0;
                while k0 < per_row {
                    let k1 = (k0 + per_run).min(per_row);
                    made.extend(run(
                        input,
                        kept,
                        Run {
                            lots: [k0, k1],
                            lot_w,
                            depth,
                            row,
                            mid,
                            frame: [u, v],
                            district: lo,
                            corner,
                            dense,
                            region,
                            turn,
                            past,
                            block: grid.block[0],
                            seed,
                        },
                    ));
                    k0 = k1;
                }
            }
            out.extend(made.iter().map(|m| m.mass));
            groups
                .entry((gj, gi))
                .or_default()
                .extend(made.into_iter().map(|m| RunMass {
                    along: [m.along[0] + ba, m.along[1] + ba],
                    across: [m.across[0] + bb, m.across[1] + bb],
                    ..m
                }));
        }
    }
    // A block's mass over its runs stands where they may (its middle on ground that stood, outside the host's areas);
    // one that would stand over a tower is its runs, each flat in its place (none stood on the tower).
    let frame = [u, v];
    blocks.extend(
        groups
            .values()
            .flat_map(|runs| match block_mass(runs, centre, frame, corner) {
                Some(m) if on_tower(kept, &corners_of(&m)) => runs
                    .iter()
                    .filter_map(|r| block_mass(std::slice::from_ref(r), centre, frame, corner))
                    .collect(),
                m => Vec::from_iter(m),
            })
            .filter(|m| {
                let [x, z] = m.at();
                (input.stood)([corner[0] + f64::from(x), corner[1] + f64::from(z)])
            }),
    );
}

/// A run's mass, and what its block's mass takes from it: its extent along the block's street and across the block
/// (m from the block's middle; from the district's once handed to its group), its roof's mean height over the ground,
/// and its length.
struct RunMass {
    mass: Mass,
    along: [f64; 2],
    across: [f64; 2],
    top: f32,
    length: f32,
}

/// A group of blocks' one mass (the far level's farther stretch, [`Mass::BLOCK`]): over all its runs in the chunk
/// (their extents from `mid`), as high as their roofs' mean (flat), in the colours of its longest run.
fn block_mass(
    runs: &[RunMass],
    mid: [f64; 2],
    frame: [[f64; 2]; 2],
    corner: [f64; 2],
) -> Option<Mass> {
    let longest = runs.iter().max_by(|a, b| a.length.total_cmp(&b.length))?;
    let [u, v] = frame;
    let a0 = runs
        .iter()
        .map(|r| r.along[0])
        .fold(f64::INFINITY, f64::min);
    let a1 = runs
        .iter()
        .map(|r| r.along[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let b0 = runs
        .iter()
        .map(|r| r.across[0])
        .fold(f64::INFINITY, f64::min);
    let b1 = runs
        .iter()
        .map(|r| r.across[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let weight: f32 = runs.iter().map(|r| r.length).sum();
    let top = runs.iter().map(|r| r.top * r.length).sum::<f32>() / weight;
    let ground = runs.iter().map(|r| r.mass.ground() * r.length).sum::<f32>() / weight;
    let lowest = runs
        .iter()
        .map(|r| r.mass.foot)
        .fold(f32::INFINITY, f32::min);
    let sink = (((ground - lowest) / 0.25).ceil().max(0.0) as u32).min(255);
    let (a, b) = ((a0 + a1) * 0.5, (b0 + b1) * 0.5);
    let centre = [mid[0] + u[0] * a + v[0] * b, mid[1] + u[1] * a + v[1] * b];
    let mut style = longest.mass.style;
    style[3] = (style[3] & 15) | Mass::BLOCK;
    Some(Mass {
        foot: ground - sink as f32 * 0.25,
        place: [
            half((centre[0] - corner[0]) as f32),
            half((centre[1] - corner[1]) as f32),
            half((a1 - a0) as f32),
            half((b1 - b0) as f32),
        ],
        height: [half(top), half(0.0)],
        style,
        turn: (longest.mass.turn & 255) | (sink << 8),
    })
}

/// A run of lots `lots` (first, past the last) of a block's row.
struct Run {
    lots: [i64; 2],
    lot_w: f64,
    depth: f64,
    row: f64,
    mid: [f64; 2],
    frame: [[f64; 2]; 2],
    district: [f64; 2],
    corner: [f64; 2],
    dense: bool,
    region: Region,
    turn: u32,
    past: bool,
    block: f64,
    /// The block's hash byte.
    seed: u8,
}

fn run(input: &FarInput, standing: &[Kept], r: Run) -> Option<RunMass> {
    let land = input.land;
    let [u, v] = r.frame;
    let at_lot = |k: f64| {
        let la = -r.block * 0.5 + (k + 0.5) * r.lot_w;
        let lb = r.row * r.depth * 0.5;
        [
            r.mid[0] + u[0] * la + v[0] * lb,
            r.mid[1] + u[1] * la + v[1] * lb,
        ]
    };
    let inside = |p: [f64; 2]| {
        (r.district[0]..r.district[0] + DISTRICT_M).contains(&p[0])
            && (r.district[1]..r.district[1] + DISTRICT_M).contains(&p[1])
    };
    // The lots of the run inside its district and on built-up land.
    let mut first = None;
    let mut last = 0;
    let mut built = 0;
    for k in r.lots[0]..r.lots[1] {
        let p = at_lot(k as f64);
        if !inside(p) {
            continue;
        }
        if land.cover(p) == BUILT_UP {
            built += 1;
            first.get_or_insert(k);
            last = k;
        }
    }
    let first = first?;
    let lots = (last - first + 1) as f64;
    let middle = at_lot((first + last) as f64 * 0.5);
    // Each run once: in its own chunk.
    if !(r.corner[0]..r.corner[0] + CHUNK_M).contains(&middle[0])
        || !(r.corner[1]..r.corner[1] + CHUNK_M).contains(&middle[1])
    {
        return None;
    }
    if !(input.stood)(middle) {
        return None;
    }
    let hash = |n: i64| {
        hash01(
            (middle[0] * 2.0).round() as i64,
            (middle[1] * 2.0).round() as i64,
            n + 400,
        )
    };
    // Fewer lots built on where the town thins (the infill keeps a lot where its hash is under 0.35 + 0.6 of the share
    // about it), and only those on built-up land.
    let local = land.built_share(middle, 40.0);
    let kept = ((0.35 + 0.6 * f64::from(local)).min(1.0) * built as f64 / lots).min(1.0);
    if kept < 0.2 || (kept < 0.5 && f64::from(hash(1)) > kept * 2.0) {
        return None;
    }
    let b = run_building(r.region, r.dense, local, &hash);
    let mut storeys = b.storeys;
    if r.past {
        storeys = storeys.min(f32::from(period_storeys(r.region)));
    }
    let along = lots * r.lot_w * b.fill * kept.sqrt().max(0.6);
    let deep = match b.kind {
        Kind::House => (r.depth * b.depth).clamp(6.0, 15.0),
        _ => r.depth * b.depth,
    };
    if along < 4.0 {
        return None;
    }
    // Set back from the street, toward its side of the block.
    let toward = r.depth * 0.5 - b.setback - deep * 0.5;
    let centre = [
        middle[0] + v[0] * r.row * toward,
        middle[1] + v[1] * r.row * toward,
    ];
    // None on a tower's footprint: the tower stands there.
    let corners = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].map(|[a, b]: [f64; 2]| {
        [
            (centre[0] + u[0] * along * a + v[0] * deep * b - r.corner[0]) as f32,
            (centre[1] + u[1] * along * a + v[1] * deep * b - r.corner[1]) as f32,
        ]
    });
    if on_tower(standing, &corners) {
        return None;
    }
    // On dry, gentle land: the ground under its ends and middle.
    let ends = [-0.5, 0.0, 0.5].map(|t: f64| {
        let p = [centre[0] + u[0] * along * t, centre[1] + u[1] * along * t];
        land.height(p)
    });
    let low = ends.iter().copied().fold(f32::INFINITY, f32::min);
    let high = ends.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if low <= 1.0 || (high - low) / along as f32 > MAX_SLOPE {
        return None;
    }
    let ground = ends[1].max(low);
    let storey_m = storey_height(b.kind);
    let mut eaves = storeys * storey_m;
    // The roof: pitched as the region's houses and terraces are (a core's low block hipped as its blocks are), its rise
    // from the region's pitch over half the depth.
    // Its look in the period: a past Taiwan's East Asia's (`Region::in_period`), its lots its own.
    let look = r.region.in_period(input.period);
    let rules = roof_rules(look, b.kind, input.period);
    let pitched = match b.kind {
        Kind::Block => storeys <= 6.0 && hash(9) < rules.block,
        _ => hash(9) >= rules.flat,
    };
    let mut rise = if pitched {
        let degrees = if b.kind == Kind::Block {
            18.0 + 8.0 * hash(10)
        } else {
            rules.pitch[0] + rules.pitch[1] * hash(10)
        };
        let rise = (deep as f32 * 0.5) * degrees.to_radians().tan();
        if b.kind == Kind::Terrace {
            rise.min(4.2)
        } else {
            rise
        }
    } else {
        0.0
    };
    if r.past && eaves + rise > input.cap_m {
        let scale = input.cap_m / (eaves + rise);
        eaves *= scale;
        rise *= scale;
    }
    let (wall, roof) = colours(look, b.kind, input.period, pitched, hash(11), hash(12));
    let local_x = (centre[0] - r.corner[0]) as f32;
    let local_z = (centre[1] - r.corner[1]) as f32;
    let foot = low - SINK_M;
    let sink = (((ground - foot) / 0.25).round() as u32).min(255);
    // Where it stands in its block (from the block's middle, along its street and across).
    let at_along = -r.block * 0.5 + ((first + last) as f64 * 0.5 + 0.5) * r.lot_w;
    let at_across = r.row * (r.depth * 0.5 + toward);
    Some(RunMass {
        mass: Mass {
            foot: ground - sink as f32 * 0.25,
            place: [
                half(local_x),
                half(local_z),
                half(along as f32),
                half(deep as f32),
            ],
            height: [half(eaves), half(rise)],
            style: [wall, roof, r.seed, b.kind as u8],
            turn: r.turn | (sink << 8),
        },
        along: [at_along - along * 0.5, at_along + along * 0.5],
        across: [at_across - deep * 0.5, at_across + deep * 0.5],
        top: eaves + rise * 0.5,
        length: along as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::super::Part;
    use super::*;

    /// A synthetic land: built up where `built` says (a theatre point), the ground 10 m up but where `sea` says.
    struct Synthetic<'a> {
        built: &'a dyn Fn([f64; 2]) -> bool,
        sea: &'a dyn Fn([f64; 2]) -> bool,
    }

    impl infill::Land for Synthetic<'_> {
        fn cover(&self, p: [f64; 2]) -> u8 {
            if (self.sea)(p) {
                80
            } else if (self.built)(p) {
                BUILT_UP
            } else {
                30
            }
        }

        fn built_share(&self, p: [f64; 2], radius: f64) -> f32 {
            let mut n = 0;
            let mut b = 0;
            for j in -2..=2 {
                for i in -2..=2 {
                    let q = [
                        p[0] + f64::from(i) * radius / 2.0,
                        p[1] + f64::from(j) * radius / 2.0,
                    ];
                    n += 1;
                    b += i32::from(self.cover(q) == BUILT_UP);
                }
            }
            b as f32 / n as f32
        }

        fn height(&self, p: [f64; 2]) -> f32 {
            if (self.sea)(p) { -5.0 } else { 10.0 }
        }
    }

    fn make(
        index: [i32; 2],
        built: &dyn Fn([f64; 2]) -> bool,
        sea: &dyn Fn([f64; 2]) -> bool,
        stood: &dyn Fn([f64; 2]) -> bool,
        region: Region,
        period: Period,
    ) -> ChunkMesh {
        let land = Synthetic { built, sea };
        build_far(&FarInput {
            index,
            land: &land,
            stood,
            region: &|_| region,
            chunk_region: region,
            period,
            cap_m: 25.0,
            towers: None,
        })
    }

    fn theatre(index: [i32; 2], m: &Mass) -> [f64; 2] {
        let [x, z] = m.at();
        [
            f64::from(index[0]) * CHUNK_M + f64::from(x),
            f64::from(index[1]) * CHUNK_M + f64::from(z),
        ]
    }

    /// A far chunk's runs, and its blocks.
    fn runs(mesh: &ChunkMesh) -> &[Mass] {
        &mesh.masses[..mesh.far_runs as usize]
    }

    fn blocks(mesh: &ChunkMesh) -> &[Mass] {
        &mesh.masses[mesh.far_runs as usize..]
    }

    #[test]
    fn a_blocks_mass_stands_over_its_runs() {
        let built = |_p: [f64; 2]| true;
        for region in [
            Region::Mediterranean,
            Region::West,
            Region::British,
            Region::EastAsia,
        ] {
            let mesh = make([1, 1], &built, &|_| false, &|_| true, region, Period::Today);
            let (runs, blocks) = (runs(&mesh), blocks(&mesh));
            assert!(runs.iter().all(|m| !m.is_block()));
            assert!(blocks.iter().all(Mass::is_block));
            // A few runs to a block: far fewer instances.
            assert!(
                blocks.len() * 3 < runs.len() && blocks.len() * 20 > runs.len(),
                "{region:?}: {} blocks, {} runs",
                blocks.len(),
                runs.len()
            );
            let tops = |m: &Mass| m.heights()[0] + m.heights()[1] * 0.5;
            for block in blocks {
                // Flat, as high as its runs' roofs on the whole.
                assert_eq!(block.heights()[1], 0.0);
                let [x, z] = block.at();
                let [length, depth] = block.size();
                let heading = block.heading();
                let (s, c) = heading.sin_cos();
                // Its runs: the block's hash and heading, their middles within it.
                let inside: Vec<&Mass> = runs
                    .iter()
                    .filter(|r| {
                        r.style[2] == block.style[2] && (r.turn & 255) == (block.turn & 255)
                    })
                    .filter(|r| {
                        let [rx, rz] = r.at();
                        let (dx, dz) = (rx - x, rz - z);
                        let along = dx * c + dz * s;
                        let across = -dx * s + dz * c;
                        along.abs() <= length * 0.5 + 2.0 && across.abs() <= depth * 0.5 + 2.0
                    })
                    .collect();
                assert!(!inside.is_empty(), "{region:?}: a block with no runs");
                let low = inside.iter().map(|r| tops(r)).fold(f32::INFINITY, f32::min);
                let high = inside.iter().map(|r| tops(r)).fold(0.0, f32::max);
                let top = block.heights()[0];
                assert!(top >= low - 0.1 && top <= high + 0.1, "{top} {low} {high}");
                assert!(
                    block.foot
                        <= inside.iter().map(|r| r.foot).fold(f32::INFINITY, f32::min) + 0.01
                );
            }
        }
    }

    #[test]
    fn a_built_up_chunk_holds_about_as_many_masses_in_every_region() {
        // A whole chunk built up: 690-900 runs and 115-215 blocks' masses (a block across the chunk's or a district's
        // edge counted on each side), however small the region's blocks (East Asia's 64 × 34 m: whole rows to a run,
        // two by two blocks to a mass).
        let built = |_p: [f64; 2]| true;
        for region in super::super::region::REGIONS {
            let mesh = make([4, 7], &built, &|_| false, &|_| true, region, Period::Today);
            let (runs, blocks) = (runs(&mesh).len(), blocks(&mesh).len());
            assert!(
                (600..=MOST_RUNS_KM2 as usize).contains(&runs) && (100..=240).contains(&blocks),
                "{region:?}: {runs} runs, {blocks} blocks"
            );
        }
    }

    #[test]
    fn masses_stand_only_on_built_up_dry_land() {
        // Built up west of x = 500 (chunk-local), the sea south of z = 700.
        let built = |p: [f64; 2]| p[0] < 500.0;
        let sea = |p: [f64; 2]| p[1] > 700.0;
        let mesh = make(
            [0, 0],
            &built,
            &sea,
            &|_| true,
            Region::Mediterranean,
            Period::Today,
        );
        assert!(runs(&mesh).len() > 60, "{}", runs(&mesh).len());
        assert!(mesh.vertices.is_empty() && mesh.indices.is_empty());
        assert_eq!(mesh.buildings as usize, runs(&mesh).len());
        for m in blocks(&mesh) {
            let p = theatre([0, 0], m);
            assert!(p[0] < 540.0 && p[1] < 700.0, "{p:?}");
        }
        for m in runs(&mesh) {
            let p = theatre([0, 0], m);
            // Its middle on the built-up side (a run's lots end at the cover's edge; a mass reaches past it by its
            // setback and half its depth at most).
            assert!(p[0] < 540.0, "{p:?}");
            assert!(p[1] < 700.0, "{p:?}");
            assert!(m.ground() > 1.0);
            let [along, deep] = m.size();
            assert!((4.0..70.0).contains(&along), "{along}");
            assert!((4.0..45.0).contains(&deep), "{deep}");
            let [eaves, rise] = m.heights();
            assert!((2.0..40.0).contains(&eaves), "{eaves}");
            assert!((0.0..12.0).contains(&rise), "{rise}");
            assert!(m.foot <= m.ground() - SINK_M + 0.01);
        }
        // Nothing in the chunk beside it, which holds no built-up land.
        let empty = make(
            [3, 0],
            &|p| p[0] < 500.0,
            &|_| false,
            &|_| true,
            Region::Mediterranean,
            Period::Today,
        );
        assert!(empty.masses.is_empty());
        assert_eq!(empty.min, [0.0; 3]);
    }

    #[test]
    fn a_chunk_is_made_the_same_twice_and_runs_are_not_shared() {
        let built = |_p: [f64; 2]| true;
        let a = make(
            [2, -3],
            &built,
            &|_| false,
            &|_| true,
            Region::Temperate,
            Period::Today,
        );
        let b = make(
            [2, -3],
            &built,
            &|_| false,
            &|_| true,
            Region::Temperate,
            Period::Today,
        );
        assert_eq!(a.masses, b.masses);
        // Every mass's middle in its own chunk, and its neighbour's in its own: none made twice.
        let c = make(
            [3, -3],
            &built,
            &|_| false,
            &|_| true,
            Region::Temperate,
            Period::Today,
        );
        let mut seen = std::collections::HashSet::new();
        for (index, mesh) in [([2, -3], &a), ([3, -3], &c)] {
            for m in runs(mesh) {
                let [x, z] = m.at();
                assert!((-40.0..1040.0).contains(&x) && (-40.0..1040.0).contains(&z));
                let p = theatre(index, m);
                assert!(seen.insert(((p[0] * 10.0) as i64, (p[1] * 10.0) as i64)));
            }
        }
        // A whole chunk built up: several hundred masses to a thousand or so, 24 bytes each.
        assert!((300..1400).contains(&runs(&a).len()), "{}", runs(&a).len());
        assert_eq!(std::mem::size_of::<Mass>(), 24);
        assert!(a.bytes() < 24 * 1600);
    }

    #[test]
    fn the_period_keeps_the_ground_that_stood_and_caps_it() {
        let built = |_p: [f64; 2]| true;
        // The ground stood east of x = 500 only.
        let stood = |p: [f64; 2]| p[0] > 500.0;
        let today = make(
            [0, 0],
            &built,
            &|_| false,
            &stood,
            Region::Aegean,
            Period::Today,
        );
        let then = make(
            [0, 0],
            &built,
            &|_| false,
            &stood,
            Region::Aegean,
            Period::Year(1942),
        );
        assert!(!today.masses.is_empty());
        assert!(!then.masses.is_empty());
        let storeys = f32::from(period_storeys(Region::Aegean));
        for m in runs(&then) {
            assert!(theatre([0, 0], m)[0] > 500.0 - 30.0);
            let [eaves, rise] = m.heights();
            assert!(eaves + rise <= 25.0 + 0.05);
            assert!(eaves <= storeys * storey_height(Kind::Block) + 0.05);
        }
        // Nothing where nothing stood.
        let none = make(
            [0, 0],
            &built,
            &|_| false,
            &|_| false,
            Region::Aegean,
            Period::Year(1942),
        );
        assert!(none.masses.is_empty());
        // Today's walls stand taller than the period's storeys allow, here and there (the period's roofs may be pitched
        // where today's are flat).
        let tallest = |m: &ChunkMesh| runs(m).iter().map(|m| m.heights()[0]).fold(0.0, f32::max);
        assert!(
            tallest(&today) > tallest(&then),
            "{} {}",
            tallest(&today),
            tallest(&then)
        );
    }

    /// Where a far shape's vertex lands for a mass `along` × `across` under `rise` (its unit y: 0 foot, 1 eaves 6 m, 2
    /// ridge), as `vs_far` places it (no turn).
    fn placed(v: &ShapeVertex, along: f32, across: f32, rise: f32) -> [f32; 3] {
        let y = match v.position[1] {
            y if y > 1.5 => 6.0 + rise,
            y if y > 0.5 => 6.0,
            _ => 0.0,
        };
        [v.position[0] * along, y, v.position[2] * across]
    }

    #[test]
    fn the_far_shape_is_closed_and_faces_out() {
        let (vertices, indices) = far_shape();
        assert_eq!((vertices.len(), indices.len()), (30, 42));
        // The gables last: a flat mass drawn without them.
        let flat = FLAT_INDICES as usize;
        assert!(
            indices[..flat]
                .iter()
                .all(|&i| vertices[i as usize].normal[3] != 64)
        );
        assert!(
            indices[flat..]
                .iter()
                .all(|&i| vertices[i as usize].normal[3] == 64)
        );
        for (along, across, rise) in [(30.0, 10.0, 3.0), (20.0, 14.0, 0.0)] {
            let mut area = 0.0;
            for t in indices.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| vertices[i as usize]);
                let [pa, pb, pc] = [a, b, c].map(|v| placed(&v, along, across, rise));
                let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
                let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
                let n = [
                    e1[1] * e2[2] - e1[2] * e2[1],
                    e1[2] * e2[0] - e1[0] * e2[2],
                    e1[0] * e2[1] - e1[1] * e2[0],
                ];
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len < 1e-6 {
                    // A flat mass's gable, folded to nothing.
                    assert!(rise == 0.0 && a.normal[3] == 64);
                    continue;
                }
                area += len * 0.5;
                // The face's normal as `vs_far` takes it: a roof's from its rise over half the depth.
                let stored = if a.normal[3] == 127 {
                    let side = f32::from(a.normal[2]).signum();
                    [0.0, across * 0.5, rise * side]
                } else {
                    [
                        f32::from(a.normal[0]),
                        f32::from(a.normal[1]),
                        f32::from(a.normal[2]),
                    ]
                };
                let dot = n[0] * stored[0] + n[1] * stored[1] + n[2] * stored[2];
                assert!(dot > 0.0, "{t:?} faces in");
            }
            // Its walls and top (and a pitched roof's gables and slopes) in all.
            let walls = 2.0 * 6.0 * (along + across);
            let top = if rise > 0.0 {
                along * 2.0 * ((across * 0.5).hypot(rise)) + across * rise
            } else {
                along * across
            };
            assert!((area - walls - top).abs() < 0.01 * (walls + top), "{area}");
        }
    }

    #[test]
    fn a_core_stands_higher_than_its_suburbs() {
        let mean = |share_built: f64| {
            // A share of the ground built up, scattered by a hash of the place.
            let built = move |p: [f64; 2]| {
                f64::from(hash01((p[0] / 23.0) as i64, (p[1] / 23.0) as i64, 7)) < share_built
            };
            let mut sum = 0.0;
            let mut n = 0.0;
            for index in [[0, 0], [1, 0], [0, 1], [1, 1]] {
                let mesh = make(
                    index,
                    &built,
                    &|_| false,
                    &|_| true,
                    Region::Temperate,
                    Period::Today,
                );
                for m in runs(&mesh) {
                    sum += m.heights()[0];
                    n += 1.0;
                }
            }
            (sum / n, n)
        };
        let (core, core_n) = mean(1.0);
        let (thin, thin_n) = mean(0.45);
        assert!(core > thin + 1.0, "{core} {thin}");
        assert!(core_n > thin_n * 1.3, "{core_n} {thin_n}");
    }

    #[test]
    fn pitched_regions_roof_their_masses_and_flat_ones_do_not() {
        let built = |_p: [f64; 2]| true;
        let share_pitched = |region| {
            let mesh = make([0, 0], &built, &|_| false, &|_| true, region, Period::Today);
            let pitched = runs(&mesh).iter().filter(|m| m.heights()[1] > 0.5).count();
            pitched as f32 / runs(&mesh).len() as f32
        };
        assert!(share_pitched(Region::NorthSea) > 0.5);
        assert!(share_pitched(Region::Nile) < 0.1);
    }

    /// A square footprint of side `side` (m) about chunk-local `at`, `height` tall.
    fn square(at: [f32; 2], side: f32, height: Option<f32>) -> Footprint {
        let h = side * 0.5;
        Footprint {
            ring: vec![
                [at[0] - h, at[1] - h],
                [at[0] + h, at[1] - h],
                [at[0] + h, at[1] + h],
                [at[0] - h, at[1] + h],
            ],
            height,
            ..Footprint::default()
        }
    }

    /// A built-up chunk's masses with `footprints` mapped in it (and `about` it), in `period` under `thinning`, on
    /// ground that stood where `stood` says.
    fn towered(
        footprints: &[Footprint],
        about: &[Footprint],
        tags: &MapTags,
        period: Period,
        thinning: Option<&Thinning>,
        stood: &dyn Fn([f64; 2]) -> bool,
    ) -> ChunkMesh {
        let land = Synthetic {
            built: &|_| true,
            sea: &|_| false,
        };
        build_far(&FarInput {
            index: [4, 7],
            land: &land,
            stood,
            region: &|_| Region::Taiwan,
            chunk_region: Region::Taiwan,
            period,
            cap_m: 31.0,
            towers: Some(FarTowers {
                footprints,
                about,
                tags,
                ground: &|_| 10.0,
                thinning,
            }),
        })
    }

    /// The highest of a mesh's vertices within `r` m of chunk-local `at` (none: no vertex there).
    fn top_near(mesh: &ChunkMesh, at: [f32; 2], r: f32) -> Option<f32> {
        mesh.vertices
            .iter()
            .filter(|v| (v.position[0] - at[0]).hypot(v.position[2] - at[1]) < r)
            .map(|v| v.position[1])
            .reduce(f32::max)
    }

    /// Whether any of the mesh's masses (runs or blocks) meets `ring`.
    fn a_mass_on(mesh: &ChunkMesh, ring: &[[f32; 2]]) -> bool {
        let kept = [(box_of(ring), ring.to_vec())];
        mesh.masses.iter().any(|m| on_tower(&kept, &corners_of(m)))
    }

    /// A past period's thinning that keeps everything, at Taiwan's cap.
    fn keeping(cap: f32) -> Thinning {
        let w = 3 * super::super::thinning::CELLS;
        Thinning::new(&vec![1.0; w * w], vec![0.0; 100], cap)
    }

    #[test]
    fn a_mapped_tower_stands_in_its_far_chunk_at_its_height_and_no_mass_on_it() {
        let tags = MapTags::default();
        let tower = square([500.0, 500.0], 40.0, Some(300.0));
        let low = square([200.0, 700.0], 30.0, Some(45.0));
        let estimated = square([700.0, 200.0], 30.0, None);
        let plain = towered(&[], &[], &tags, Period::Today, None, &|_| true);
        let mesh = towered(
            &[tower.clone(), low.clone(), estimated.clone()],
            &[],
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        // The tower its box at its height on its ground (10 m), the lower and the estimated left to the masses.
        let top = top_near(&mesh, [500.0, 500.0], 30.0).expect("the tower");
        assert!((top - 310.0).abs() < 1.0, "{top}");
        assert!(top_near(&mesh, [200.0, 700.0], 25.0).is_none());
        assert!(top_near(&mesh, [700.0, 200.0], 25.0).is_none());
        assert!(mesh.max[1] >= top - 0.01 && mesh.min[1] <= 10.0);
        assert_eq!(mesh.buildings, mesh.far_runs + 1);
        // No mass (run or block) on the tower; the masses elsewhere as they were.
        assert!(a_mass_on(&plain, &tower.ring));
        assert!(!a_mass_on(&mesh, &tower.ring));
        assert!(a_mass_on(&mesh, &low.ring));
        let gone = plain.masses.len() as i64 - mesh.masses.len() as i64;
        assert!((-40..40).contains(&gone), "{gone}");
        // A tower about the chunk (its middle in a neighbour's) keeps the chunk's masses off it too, and is not drawn.
        let edge = square([1005.0, 500.0], 70.0, Some(200.0));
        let beside = towered(
            &[],
            std::slice::from_ref(&edge),
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        assert!(beside.vertices.is_empty());
        assert!(a_mass_on(&plain, &edge.ring) && !a_mass_on(&beside, &edge.ring));
    }

    #[test]
    fn a_towers_parts_stand_together_as_mapped() {
        let tags = MapTags::default();
        let middle = [500.0, 500.0];
        let part = |side: f32, base: f32, height: f32| Footprint {
            part: Some(Part { base, middle }),
            ..square(middle, side, Some(height))
        };
        // A podium of 40 m under a shaft from 40 to 250 m: both stand (the podium below the threshold, its tower's).
        let mesh = towered(
            &[part(60.0, 0.0, 40.0), part(30.0, 40.0, 250.0)],
            &[],
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        let podium = mesh
            .vertices
            .iter()
            .filter(|v| (v.position[0] - 500.0).abs() > 20.0)
            .map(|v| v.position[1])
            .reduce(f32::max);
        assert!(podium.is_some_and(|y| (y - 50.0).abs() < 1.0), "{podium:?}");
        assert!(top_near(&mesh, middle, 25.0).is_some_and(|y| (y - 260.0).abs() < 1.0));
        // Two low parts alone: masses there.
        let low = towered(
            &[part(60.0, 0.0, 30.0), part(30.0, 30.0, 50.0)],
            &[],
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        assert!(low.vertices.is_empty());
    }

    #[test]
    fn a_towers_parts_across_a_chunks_edge_stand_together() {
        let tags = MapTags::default();
        // A podium of 40 m whose middle lies in the chunk, under a shaft to 160 m whose own middle lies in the next
        // (beyond x = 1,000): one building, one middle.
        let middle = [995.0, 500.0];
        let part = |at: [f32; 2], side: f32, base: f32, height: f32| Footprint {
            part: Some(Part { base, middle }),
            ..square(at, side, Some(height))
        };
        let podium = part([975.0, 500.0], 50.0, 0.0, 40.0);
        let shaft = part([1010.0, 500.0], 30.0, 40.0, 160.0);
        let mesh = towered(
            std::slice::from_ref(&podium),
            std::slice::from_ref(&shaft),
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        // The podium stands here at its own height, the shaft's mesh the next chunk's; no mass on either.
        let top = mesh.vertices.iter().map(|v| v.position[1]).reduce(f32::max);
        assert!(top.is_some_and(|y| (y - 50.0).abs() < 1.0), "{top:?}");
        assert!(mesh.vertices.iter().all(|v| v.position[0] < 1001.0));
        assert!(!a_mass_on(&mesh, &podium.ring) && !a_mass_on(&mesh, &shaft.ring));
        // A low building across the edge: the masses' still.
        let low = towered(
            std::slice::from_ref(&podium),
            std::slice::from_ref(&Footprint {
                height: Some(50.0),
                ..shaft.clone()
            }),
            &tags,
            Period::Today,
            None,
            &|_| true,
        );
        assert!(low.vertices.is_empty());
    }

    #[test]
    fn a_mass_across_a_slim_tower_stands_on_it() {
        let rect =
            |x: [f32; 2], z: [f32; 2]| [[x[0], z[0]], [x[1], z[0]], [x[1], z[1]], [x[0], z[1]]];
        // Their edges cross, neither's corner in the other and the mass's middle off the tower.
        let tower = rect([515.0, 525.0], [470.0, 530.0]).to_vec();
        let kept = [(box_of(&tower), tower)];
        assert!(on_tower(&kept, &rect([470.0, 530.0], [490.0, 510.0])));
        // Beside it, or within the one box but not the other's ring: none.
        assert!(!on_tower(&kept, &rect([470.0, 512.0], [490.0, 510.0])));
        let ell = vec![
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 20.0],
            [20.0, 20.0],
            [20.0, 100.0],
            [0.0, 100.0],
        ];
        let kept = [(box_of(&ell), ell)];
        assert!(!on_tower(&kept, &rect([40.0, 90.0], [40.0, 90.0])));
        assert!(on_tower(&kept, &rect([10.0, 90.0], [40.0, 50.0])));
    }

    #[test]
    fn a_far_tower_keeps_the_period_and_the_hosts_areas() {
        let tower = square([500.0, 500.0], 40.0, Some(300.0));
        let tags = MapTags::default();
        let past = Period::Year(1942);
        // Taiwan's 31 m in 1942: no tower, the masses stand capped over its ground.
        let thin = keeping(31.0);
        let capped = towered(
            std::slice::from_ref(&tower),
            &[],
            &tags,
            past,
            Some(&thin),
            &|_| true,
        );
        assert!(capped.vertices.is_empty());
        assert!(a_mass_on(&capped, &tower.ring));
        // One that stood taller (`thinning::TALL`) stands at what it stood, as the boxes stand it.
        let mut tall = keeping(31.0);
        tall.tall.push(([500.0, 500.0], 65.0));
        let slim = square([500.0, 500.0], 30.0, Some(65.0));
        let listed = towered(
            std::slice::from_ref(&slim),
            &[],
            &tags,
            past,
            Some(&tall),
            &|_| true,
        );
        let top = top_near(&listed, [500.0, 500.0], 25.0).expect("the listed tower");
        assert!((top - 75.0).abs() < 1.0, "{top}");
        assert!(!a_mass_on(&listed, &slim.ring));
        let boxes = build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [4, 7],
                lod: Lod::Box,
            },
            footprints: std::slice::from_ref(&slim),
            infill: &[],
            tags: &tags,
            ground: &|_| 10.0,
            region: Region::Taiwan,
            period: past,
            thinning: Some(&tall),
        });
        assert_eq!(listed.vertices, boxes.vertices);
        // Built after the period (`thinning::LATER`), or on ground that did not stand: none.
        let mut later = keeping(320.0);
        later.later.push([500.0, 500.0]);
        assert!(
            towered(
                std::slice::from_ref(&tower),
                &[],
                &tags,
                past,
                Some(&later),
                &|_| true
            )
            .vertices
            .is_empty()
        );
        let not_stood = |p: [f64; 2]| (p[0] - 4500.0).hypot(p[1] - 7500.0) > 100.0;
        assert!(
            towered(
                std::slice::from_ref(&tower),
                &[],
                &tags,
                past,
                Some(&keeping(320.0)),
                &not_stood
            )
            .vertices
            .is_empty()
        );
        // New York's cap keeps it whole.
        let whole = towered(
            std::slice::from_ref(&tower),
            &[],
            &tags,
            past,
            Some(&keeping(320.0)),
            &|_| true,
        );
        assert!(top_near(&whole, [500.0, 500.0], 30.0).is_some_and(|y| (y - 310.0).abs() < 1.0));
        // Today, an area the host draws itself: no tower there.
        let cleared = MapTags {
            cleared: vec![square([500.0, 500.0], 120.0, None).ring],
            ..MapTags::default()
        };
        let in_area = |p: [f64; 2]| (p[0] - 4500.0).abs() > 60.0 || (p[1] - 7500.0).abs() > 60.0;
        let mesh = towered(
            std::slice::from_ref(&tower),
            &[],
            &cleared,
            Period::Today,
            None,
            &in_area,
        );
        assert!(mesh.vertices.is_empty());
        let inner = square([500.0, 500.0], 60.0, None).ring;
        let kept = [(box_of(&inner), inner)];
        assert!(!runs(&mesh).iter().any(|m| on_tower(&kept, &corners_of(m))));
    }

    #[test]
    fn a_far_chunk_with_towers_is_made_the_same_twice() {
        let tags = MapTags::default();
        let footprints = [
            square([500.0, 500.0], 40.0, Some(300.0)),
            square([300.0, 600.0], 35.0, Some(120.0)),
        ];
        let a = towered(&footprints, &[], &tags, Period::Today, None, &|_| true);
        let b = towered(&footprints, &[], &tags, Period::Today, None, &|_| true);
        assert!(!a.vertices.is_empty());
        assert_eq!(a.masses, b.masses);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
        assert_eq!((a.min, a.max, a.far_runs), (b.min, b.max, b.far_runs));
    }
}
