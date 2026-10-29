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
//!
//! Every choice hashes the run's place in the theatre, so a chunk made again holds the same masses.

use std::collections::BTreeMap;

use half::f16;

use super::clutter::ShapeVertex;
use super::infill::{self, BUILT_UP, DISTRICT_M, Grid, MAX_SLOPE};
use super::mesh::{ChunkMesh, SINK_M};
use super::palette::{colours, roof_rules};
use super::thinning::period_storeys;
use super::{CHUNK_M, Kind, Period, Region, hash01, storey_height};

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
}

/// The chunk's masses as a mesh: no vertices, its [`ChunkMesh::masses`], its bounds and region.
pub fn build_far(input: &FarInput) -> ChunkMesh {
    let index = input.index;
    let corner = [f64::from(index[0]) * CHUNK_M, f64::from(index[1]) * CHUNK_M];
    let mut mesh = ChunkMesh {
        region: input.chunk_region.index(),
        period: input.period,
        ..ChunkMesh::default()
    };
    // A quick look: no built-up land anywhere about the chunk, nothing to do.
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
            district(input, dx, dz, corner, &mut mesh.masses, &mut blocks);
        }
    }
    mesh.far_runs = mesh.masses.len() as u32;
    mesh.buildings = mesh.far_runs;
    mesh.masses.extend(blocks);
    bounds(&mut mesh);
    mesh
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
    blocks.extend(
        groups
            .values()
            .filter_map(|runs| block_mass(runs, centre, [u, v], corner)),
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

fn run(input: &FarInput, r: Run) -> Option<RunMass> {
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
    let rules = roof_rules(r.region, b.kind, input.period);
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
    let (wall, roof) = colours(r.region, b.kind, input.period, pitched, hash(11), hash(12));
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
}
