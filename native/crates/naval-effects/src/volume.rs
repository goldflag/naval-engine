//! The trial look "volume" ([`crate::Trial::volume`]): the largest, longest-lived bodies as ray-marched volumes in
//! place of their sprites. This is the CPU half; `volume_pass.rs` and `shaders/volume.wgsl` draw what it publishes.
//!
//! The bodies are the nearest burning ships' smoke (every column of a ship is one body: they merge into one pall) and
//! the newest magazine detonation. Their particles keep the game's choreography (the buoyant climb, the wind's lean,
//! the lobes thrown out of a detonation); each frame they are splatted as soft balls into two grids laid over the
//! body: a coarse one over all of it for its large balls, and a fine one about the fires for the small young ones
//! (a ball goes into one or the other by its size, and the shader joins them). A voxel holds the union of the balls
//! there (their shape, whatever their strength), and their heat, albedo and age. On the coarse lattice a third grid
//! holds the light: the smoke between each voxel and the sun and the sky above it (one sweep through the grid along
//! the light, as GPU Gems 3 chapter 30 lights its smoke), the balls' strength, and whether there is smoke at all.
//! The shader erodes the shape into billows with tiling noise and marches it. The lattices are fixed in the world
//! (an origin is a whole number of voxels, a voxel one of a few sizes), so a puff that holds still gives the same
//! voxels every frame.
//!
//! Everything else stays sprites: the ships beyond the nearest [`COLUMNS`], older detonations, gun smoke, flames.

use glam::DVec3;

use crate::camera::{CameraView, Plane};
use crate::fire_batch::FireBatch;
use crate::math::{Rgb, smooth};
use crate::particles::{ParticlePool, SourceId};

/// Ships whose smoke is marched at once.
pub const COLUMNS: usize = 2;
/// Bodies marched at once: the columns and one detonation.
pub const BODIES: usize = COLUMNS + 1;
/// Voxels along a grid's edge at most.
pub const MAX_DIM: usize = 128;
/// Voxels the coarse and the fine grid may hold: what bounds the splat, the upload and the march.
const FAR_BUDGET: usize = 36_000;
const NEAR_BUDGET: usize = 96_000;
/// The voxel's edge (m): the smallest that fits the balls in the budget.
const LADDER: [f64; 14] = [
    1.0, 1.25, 1.5, 2.0, 2.5, 3.0, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0, 24.0,
];
/// A ball is splatted at least this many voxels in radius, so it does not pulse as it crosses the lattice.
const MIN_KERNEL: f64 = 1.6;
/// A ball's kernel reaches this multiple of its particle's radius (the kernel is thin toward its rim).
const KERNEL_SCALE: f64 = 1.15;
/// Balls whose kernel is smaller than this (m) go into the fine grid.
const SMALL_KERNEL: f64 = 17.0;
/// What the grids' channels stand for at 1.0: the shape (stored as its square root), heat and age (s).
pub const SHAPE_MAX: f32 = 2.0;
pub const HEAT_MAX: f32 = 2.5;
pub const AGE_MAX: f32 = 64.0;
/// The light grid stores `exp(−optical depth / TAU_SCALE)`.
pub const TAU_SCALE: f32 = 4.0;
/// Optical depth across a ball of full strength for the light sweep, per radius.
const COLUMN_TAU: f64 = 2.6;
const BLAST_TAU: f64 = 3.4;
/// A particle fainter than this is left out: an old puff is at its largest, and costs the most to splat, as it
/// fades to nothing.
const FAINT: f64 = 0.03;
/// A ship's smoke is marched only this near the lens (m, squared).
const COLUMN_REACH_SQ: f64 = 6000.0 * 6000.0;

/// What kind of body a grid holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BodyKind {
    /// A burning ship's smoke: columns that rise, lean with the wind and merge.
    #[default]
    Column,
    /// A magazine detonation: the fireball, the column and its cap.
    Detonation,
}

/// A grid of balls: RGBA8, x fastest then y then z, holding √(shape ÷ [`SHAPE_MAX`]), heat ÷ [`HEAT_MAX`], albedo and
/// age ÷ [`AGE_MAX`] (a detonation's balls: their radius in metres ÷ [`AGE_MAX`]).
#[derive(Clone, Debug, Default)]
pub struct Grid {
    /// The grid's minimum corner (world m) and its voxel's edge (m).
    pub origin: DVec3,
    pub voxel: f64,
    /// Voxels along x, y and z (none: the grid is empty).
    pub dims: [usize; 3],
    pub texels: Vec<u8>,
}

impl Grid {
    pub fn voxels(&self) -> usize {
        self.dims.iter().product()
    }
}

/// One body's grids for a frame.
#[derive(Clone, Debug, Default)]
pub struct VolumeBody {
    /// Whether there is anything to march.
    pub live: bool,
    pub kind: BodyKind,
    /// The whole body's box and its large balls.
    pub far: Grid,
    /// The small balls about the fires, inside the body's box.
    pub near: Grid,
    /// On the coarse lattice, RGBA8: `exp(−τ ÷ TAU_SCALE)` toward the sun and toward the sky above, the balls' strength,
    /// and 1 where any ball reaches.
    pub light: Vec<u8>,
    /// Where the noise is anchored: the detonation's seat (its noise swells about it), nothing for a column.
    pub anchor: DVec3,
    /// A detonation's size (m): the noise's cells swell with it.
    pub size: f64,
    /// The wind the smoke leans with (m/s).
    pub wind: DVec3,
    /// The body's clock (s), and how long a detonation has burned (s).
    pub time: f64,
    pub age: f64,
    /// Balls splatted.
    pub blobs: usize,
}

/// Every body for a frame, in slot order (a body keeps its slot while it is marched).
#[derive(Clone, Debug, Default)]
pub struct VolumeFrame {
    pub bodies: [VolumeBody; BODIES],
}

/// One particle as the grids take it.
#[derive(Clone, Copy, Debug)]
struct Blob {
    center: DVec3,
    /// The kernel's reach (m).
    radius: f64,
    /// 0–1: how much of the particle is there.
    strength: f32,
    /// Extinction at its centre (1/m), for the light sweep.
    sigma: f32,
    heat: f32,
    shade: f32,
    age: f32,
}

/// A detonation being marched.
#[derive(Clone, Copy, Debug)]
struct Detonation {
    tag: u32,
    seat: DVec3,
    /// When it went up, on the volumes' clock.
    born: f64,
}

/// What the last publish cost and held, for the cost notes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VolumeStats {
    pub bodies: usize,
    pub blobs: usize,
    pub voxels: usize,
    /// Each slot's coarse and fine lattice: the voxel (m; 0 when idle) and the grid.
    pub lattices: [[(f64, [usize; 3]); 2]; BODIES],
    /// Milliseconds the grids took on the CPU (native only), and of them the splat, the packing and the light sweep.
    pub cpu_ms: f64,
    pub phases_ms: [f64; 3],
}

/// The volumes' state between frames: which bodies are marched, their lattices, and working room.
#[derive(Debug, Default)]
pub struct Volumes {
    time: f64,
    columns: [Option<SourceId>; COLUMNS],
    /// Each slot's coarse and fine voxel last frame (0: none), so a body changes lattice only when it must.
    voxels: [[f64; 2]; BODIES],
    next_tag: u32,
    detonation: Option<Detonation>,
    blobs: Vec<Blob>,
    /// Per ship: nearest live puff (squared distance), its smoke's bounds.
    ships: Vec<(SourceId, f64, DVec3, DVec3)>,
    /// Per voxel of the grid being filled, summed over the balls by the fourth power of their kernel there: 1, heat,
    /// albedo, age.
    cells: Vec<[f32; 4]>,
    /// Per voxel of the light grid: the balls' extinction squared, their strength by the same weight, the weight.
    lights: Vec<[f32; 4]>,
    /// A ball's offsets along a row of voxels (`splat`'s working room).
    along: Vec<f32>,
    sigma: Vec<f32>,
    toward: Vec<f32>,
    slice: Vec<f32>,
    pub stats: VolumeStats,
}

fn luminance(color: Rgb) -> f64 {
    color.r * 0.2126 + color.g * 0.7152 + color.b * 0.0722
}

/// Whether the box `min`–`max` reaches into the frustum.
fn in_view(planes: &[Plane; 6], min: DVec3, max: DVec3) -> bool {
    planes.iter().all(|plane| {
        // The corner furthest along the plane's normal.
        let corner = DVec3::select(plane.normal.cmpge(DVec3::ZERO), max, min);
        plane.distance(corner) >= 0.0
    })
}

fn dims_for(extent: DVec3, voxel: f64) -> [usize; 3] {
    // The balls' reach, a clear voxel on every side and the slack of snapping the origin to the lattice.
    extent.to_array().map(|e| (e / voxel).ceil() as usize + 3)
}

fn fits(extent: DVec3, voxel: f64, budget: usize) -> bool {
    let dims = dims_for(extent, voxel);
    dims.iter().all(|&n| n <= MAX_DIM) && dims[0] * dims[1] * dims[2] <= budget
}

/// The balls' bounds, each reaching at least `floor` metres.
fn bounds(blobs: impl Iterator<Item = Blob>, floor: f64) -> (DVec3, DVec3) {
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for blob in blobs {
        let reach = DVec3::splat(blob.radius.max(floor));
        low = low.min(blob.center - reach);
        high = high.max(blob.center + reach);
    }
    (low, high)
}

/// A lattice over `blobs` within `budget` voxels: the smallest voxel that fits (a body that still fits `previous`
/// keeps it until the next one down fits with room to spare, so the lattice changes seldom), the origin a whole number
/// of voxels with a clear voxel on every side. `None` without balls.
fn lattice(
    blobs: impl Iterator<Item = Blob> + Clone,
    budget: usize,
    previous: f64,
) -> Option<(DVec3, f64, [usize; 3])> {
    let (low, high) = bounds(blobs.clone(), 0.0);
    if !low.is_finite() {
        return None;
    }
    let extent = high - low;
    let mut rung = LADDER
        .iter()
        .position(|&voxel| fits(extent, voxel, budget))
        .unwrap_or(LADDER.len() - 1);
    if let Some(held) = LADDER.iter().position(|&voxel| voxel == previous)
        && held > rung
        && fits(extent, previous, budget)
    {
        rung = if fits(extent * 1.2, LADDER[held - 1], budget) {
            held - 1
        } else {
            held
        };
    }
    // The smallest balls are widened to the kernel's floor, which can ask for one voxel size more.
    let (voxel, low, high) = loop {
        let voxel = LADDER[rung];
        let (low, high) = bounds(blobs.clone(), MIN_KERNEL * voxel);
        if fits(high - low, voxel, budget) || rung + 1 == LADDER.len() {
            break (voxel, low, high);
        }
        rung += 1;
    };
    let origin = (low / voxel).floor() * voxel - DVec3::splat(voxel);
    let dims = ((high - origin) / voxel)
        .to_array()
        .map(|n| (n.ceil() as usize + 1).min(MAX_DIM));
    Some((origin, voxel, dims))
}

/// A lattice: its minimum corner (world m), its voxel's edge (m) and its voxels along x, y and z.
type Lattice = (DVec3, f64, [usize; 3]);

/// What a voxel takes of a ball `offset` from its centre (squared, in kernel radii): the fourth power of the kernel
/// `(1 − d²)²`. Summed by it, balls that overlap join as a union (the root of the squares' sum) rather than piling up,
/// so a column of many puffs is no wider than they are.
fn weight(offset: f32) -> f32 {
    let k = (1.0 - offset).max(0.0);
    let k = k * k;
    k * k
}

/// Packs the sums a lattice's voxels hold (the weights, and heat, albedo and age by them) into `grid`.
fn pack(grid: &mut Grid, (origin, voxel, dims): Lattice, cells: &[[f32; 4]]) {
    grid.origin = origin;
    grid.voxel = voxel;
    grid.dims = dims;
    grid.texels.clear();
    grid.texels.resize(cells.len() * 4, 0);
    for (texel, cell) in grid.texels.as_chunks_mut::<4>().0.iter_mut().zip(cells) {
        if cell[0] <= 0.0 {
            continue;
        }
        let inverse = 1.0 / cell[0];
        texel[0] = unit((cell[0].sqrt() / SHAPE_MAX).sqrt());
        texel[1] = unit(cell[1] * inverse / HEAT_MAX);
        texel[2] = unit(cell[2] * inverse);
        texel[3] = unit(cell[3] * inverse / AGE_MAX);
    }
}

/// Calls `row` for every row of voxels along x that a ball's kernel reaches on a lattice: the voxels' indices, how far
/// each lies from the ball's centre along x and how far the row does across it (both squared, in kernel radii: their
/// sum is [`weight`]'s offset), and what the ball's weight is scaled by (a ball below the kernel's floor is widened,
/// and thinned by as much). `along` is working room.
fn splat(
    blob: &Blob,
    (origin, voxel, dims): Lattice,
    along: &mut Vec<f32>,
    mut row: impl FnMut(std::ops::Range<usize>, &[f32], f32, f32),
) {
    let [nx, ny, nz] = dims;
    let radius = blob.radius.max(MIN_KERNEL * voxel);
    let thinned = (blob.radius / radius).powi(2) as f32;
    let reach = (radius / voxel) as f32;
    // Voxel centres sit at whole numbers plus a half.
    let c = ((blob.center - origin) / voxel).as_vec3() - 0.5;
    let span = |centre: f32, n: usize| {
        let first = (centre - reach).ceil().max(0.0) as usize;
        let last = ((centre + reach).floor() as isize).min(n as isize - 1);
        (first, last)
    };
    let (x0, x1) = span(c.x, nx);
    let (y0, y1) = span(c.y, ny);
    let (z0, z1) = span(c.z, nz);
    if x1 < x0 as isize {
        return;
    }
    let inverse = 1.0 / (reach * reach);
    along.clear();
    along.extend((x0..=x1 as usize).map(|x| (x as f32 - c.x).powi(2) * inverse));
    for z in z0 as isize..=z1 {
        let dz = (z as f32 - c.z).powi(2) * inverse;
        for y in y0 as isize..=y1 {
            let across = dz + (y as f32 - c.y).powi(2) * inverse;
            if across >= 1.0 {
                continue;
            }
            let start = (z as usize * ny + y as usize) * nx + x0;
            row(start..start + along.len(), along, across, thinned);
        }
    }
}

fn unit(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

impl Volumes {
    pub fn advance(&mut self, dt: f64) {
        self.time += dt;
    }

    /// A magazine goes up at `seat`: the tag to stamp on its gas (`ParticlePool::tag`). The newest is the one marched.
    pub fn detonate(&mut self, seat: DVec3) -> u32 {
        self.next_tag += 1;
        self.detonation = Some(Detonation {
            tag: self.next_tag,
            seat,
            born: self.time,
        });
        self.voxels[COLUMNS] = [0.0; 2];
        self.next_tag
    }

    pub fn reset(&mut self) {
        self.time = 0.0;
        self.columns = Default::default();
        self.voxels = [[0.0; 2]; BODIES];
        self.detonation = None;
        self.stats = VolumeStats::default();
    }

    /// Chooses the bodies for `camera`, takes their particles out of the sprites (`smoke.volume_sources`,
    /// `gas.hidden_tag`) and fills their grids into `frame`. `sun` points toward the light.
    #[allow(clippy::too_many_arguments)]
    pub fn publish(
        &mut self,
        camera: &CameraView,
        hidden: Option<&str>,
        smoke: &mut FireBatch,
        gas: &mut ParticlePool,
        sun: DVec3,
        wind: DVec3,
        frame: &mut VolumeFrame,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        let started = std::time::Instant::now();
        self.stats = VolumeStats::default();
        let planes = camera.frustum();
        self.choose_columns(camera, &planes, hidden, smoke);
        smoke.volume_sources.clear();
        for slot in 0..COLUMNS {
            let body = &mut frame.bodies[slot];
            body.live = false;
            let Some(ship) = self.columns[slot].clone() else {
                self.voxels[slot] = [0.0; 2];
                continue;
            };
            self.blobs.clear();
            for p in smoke.particles() {
                if p.age < 0.0 || p.age >= p.life || p.source_id.as_ref() != Some(&ship) {
                    continue;
                }
                let t = p.age / p.life;
                let extent = p.extent();
                // The sprite's fade (`FireBatch::publish_into`), less its clearing about the lens: a volume can be
                // looked through from inside.
                let fade = (if p.fade_in > 0.0 {
                    smooth(p.age / p.fade_in)
                } else {
                    1.0
                }) * (1.0 - smooth((t - 0.45) / 0.55))
                    * if p.thin > 0.0 {
                        (p.size / extent).powf(p.thin).clamp(0.35, 1.0)
                    } else {
                        1.0
                    };
                let strength = p.opacity * fade;
                if strength < FAINT {
                    continue;
                }
                self.blobs.push(Blob {
                    center: p.position,
                    radius: extent * KERNEL_SCALE,
                    strength: strength as f32,
                    sigma: (strength * COLUMN_TAU / extent) as f32,
                    heat: (p.glow * (-p.age / p.cool).exp()) as f32,
                    shade: luminance(p.albedo) as f32,
                    age: p.age as f32,
                });
            }
            body.kind = BodyKind::Column;
            body.anchor = DVec3::ZERO;
            body.size = 0.0;
            body.wind = wind;
            body.time = self.time;
            if self.build(slot, sun, body) {
                // The ship's smoke is the volume's from here: its sprites are left out.
                smoke.volume_sources.push(ship);
            }
        }

        gas.hidden_tag = 0;
        let body = &mut frame.bodies[COLUMNS];
        body.live = false;
        if let Some(detonation) = self.detonation {
            self.blobs.clear();
            let (mut weight, mut spread) = (0.0, 0.0);
            let mut alive = false;
            for p in gas.particles() {
                if p.tag != detonation.tag || p.age >= p.life {
                    continue;
                }
                // A secondary explosion still to come keeps the body alive.
                alive = true;
                if p.age < 0.0 {
                    continue;
                }
                if let (Some(hidden), Some(source)) = (hidden, &p.source_id)
                    && &**source == hidden
                {
                    continue;
                }
                let t = p.age / p.life;
                // The pool's fade (`EffectParticle::fade`) and the gas shader's thinning with dispersal.
                let fade = (1.0 - smooth((t - 0.18) / 0.82))
                    * if p.fade_in > 0.0 {
                        smooth(p.age / p.fade_in)
                    } else {
                        1.0
                    };
                let dispersal = if p.dissipation_time > 0.0 {
                    (p.age - p.cooling).max(0.0) / p.dissipation_time
                } else {
                    0.0
                };
                let thin = 1.0 - (1.0 - (-dispersal * dispersal).exp()) * 0.8;
                let strength = p.opacity * fade * thin * (p.density / 3.0).min(1.0);
                if strength < FAINT {
                    continue;
                }
                let radius = p.current_size() / 2.0;
                let mass = strength * radius * radius * radius;
                weight += mass;
                spread +=
                    mass * ((p.position - detonation.seat).length_squared() + radius * radius);
                self.blobs.push(Blob {
                    center: p.position,
                    radius: radius * KERNEL_SCALE,
                    strength: strength as f32,
                    sigma: (strength * BLAST_TAU / radius) as f32,
                    heat: (p.heat * (1.0 - smooth(p.age / p.cooling))) as f32,
                    shade: luminance(p.color) as f32,
                    // A detonation's lobes are all one age: the channel carries their radius (m) instead, which sets
                    // how thick the gas is and how finely it is marched.
                    age: radius as f32,
                });
            }
            if !alive {
                self.detonation = None;
                self.voxels[COLUMNS] = [0.0; 2];
            } else {
                // Its sprites stay out while the body lives, drawn or not.
                gas.hidden_tag = detonation.tag;
                body.kind = BodyKind::Detonation;
                body.anchor = detonation.seat;
                body.age = self.time - detonation.born;
                body.size = if weight > 0.0 {
                    (spread / weight).sqrt()
                } else {
                    1.0
                };
                body.wind = wind;
                body.time = self.time;
                self.build(COLUMNS, sun, body);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.stats.cpu_ms = started.elapsed().as_secs_f64() * 1000.0;
        }
    }

    /// The ships whose smoke is marched: the nearest in view, a ship already marched keeping its place unless another
    /// is clearly nearer, each in the slot it had.
    fn choose_columns(
        &mut self,
        camera: &CameraView,
        planes: &[Plane; 6],
        hidden: Option<&str>,
        smoke: &FireBatch,
    ) {
        self.ships.clear();
        for p in smoke.particles() {
            if p.age < 0.0 || p.age >= p.life {
                continue;
            }
            let Some(source) = &p.source_id else {
                continue;
            };
            if hidden.is_some_and(|hidden| &**source == hidden) {
                continue;
            }
            let near = (p.position - camera.position).length_squared();
            let reach = DVec3::splat(p.extent());
            let (low, high) = (p.position - reach, p.position + reach);
            match self.ships.iter_mut().find(|ship| ship.0 == *source) {
                Some(ship) => {
                    ship.1 = ship.1.min(near);
                    ship.2 = ship.2.min(low);
                    ship.3 = ship.3.max(high);
                }
                None => self.ships.push((source.clone(), near, low, high)),
            }
        }
        let held = |columns: &[Option<SourceId>; COLUMNS], ship: &SourceId| {
            columns.iter().flatten().any(|column| column == ship)
        };
        let columns = &self.columns;
        self.ships.retain(|ship| {
            // A ship already marched stays until it is well out of reach or sight.
            let slack = if held(columns, &ship.0) { 1.3 } else { 1.0 };
            ship.1 < COLUMN_REACH_SQ * slack && in_view(planes, ship.2, ship.3)
        });
        for ship in &mut self.ships {
            if held(columns, &ship.0) {
                ship.1 *= 0.64;
            }
        }
        self.ships.sort_by(|a, b| a.1.total_cmp(&b.1));
        self.ships.truncate(COLUMNS);
        for slot in 0..COLUMNS {
            if let Some(ship) = &self.columns[slot]
                && !self.ships.iter().any(|chosen| chosen.0 == *ship)
            {
                self.columns[slot] = None;
            }
        }
        for index in 0..self.ships.len() {
            if held(&self.columns, &self.ships[index].0) {
                continue;
            }
            if let Some(slot) = self.columns.iter().position(Option::is_none) {
                self.columns[slot] = Some(self.ships[index].0.clone());
                self.voxels[slot] = [0.0; 2];
            }
        }
    }

    /// Lays the lattices over `self.blobs`, splats them and sweeps the light. False when there is nothing to march.
    fn build(&mut self, slot: usize, sun: DVec3, body: &mut VolumeBody) -> bool {
        body.live = false;
        body.blobs = self.blobs.len();
        let all = self.blobs.iter().copied();
        let Some((origin, voxel, dims)) = lattice(all, FAR_BUDGET, self.voxels[slot][0]) else {
            return false;
        };
        // The fine grid takes the small balls, when a voxel finer than the coarse one holds them.
        let small = |blob: &Blob| blob.radius < SMALL_KERNEL;
        let near = lattice(
            self.blobs.iter().copied().filter(small),
            NEAR_BUDGET,
            self.voxels[slot][1],
        )
        .filter(|near| near.1 < voxel);
        self.voxels[slot] = [voxel, near.map_or(0.0, |near| near.1)];
        self.stats.bodies += 1;
        self.stats.blobs += self.blobs.len();
        #[cfg(not(target_arch = "wasm32"))]
        let mut mark = std::time::Instant::now();
        #[cfg(not(target_arch = "wasm32"))]
        let mut lap = |stats: &mut VolumeStats, phase: usize| {
            stats.phases_ms[phase] += mark.elapsed().as_secs_f64() * 1000.0;
            mark = std::time::Instant::now();
        };
        #[cfg(target_arch = "wasm32")]
        let lap = |_: &mut VolumeStats, _: usize| {};

        // The coarse lattice: the large balls' shapes, and from every ball the light grid's sums (its extinction,
        // joined as the shapes are, its strength, and that it is there).
        let in_near = near.is_some();
        let far = (origin, voxel, dims);
        let count = dims.iter().product::<usize>();
        self.cells.clear();
        self.cells.resize(count, [0.0; 4]);
        self.lights.clear();
        self.lights.resize(count, [0.0; 4]);
        for blob in &self.blobs {
            let carried = [1.0, blob.heat, blob.shade, blob.age];
            let fine = in_near && small(blob);
            let (cells, lights) = (&mut self.cells, &mut self.lights);
            splat(
                blob,
                far,
                &mut self.along,
                |voxels, along, across, thinned| {
                    let lit = [(blob.sigma * thinned).powi(2), blob.strength, 1.0, 0.0];
                    let lights = lights[voxels.clone()].iter_mut();
                    if fine {
                        for (light, offset) in lights.zip(along) {
                            let w = weight(across + offset);
                            for (held, lit) in light.iter_mut().zip(lit) {
                                *held += w * lit;
                            }
                        }
                    } else {
                        for ((cell, light), offset) in
                            cells[voxels].iter_mut().zip(lights).zip(along)
                        {
                            let w = weight(across + offset);
                            for (held, carried) in cell.iter_mut().zip(carried) {
                                *held += w * thinned * carried;
                            }
                            for (held, lit) in light.iter_mut().zip(lit) {
                                *held += w * lit;
                            }
                        }
                    }
                },
            );
        }
        pack(&mut body.far, far, &self.cells);
        match near {
            Some(lattice) => {
                self.cells.clear();
                self.cells
                    .resize(lattice.2.iter().product::<usize>(), [0.0; 4]);
                for blob in self.blobs.iter().filter(|blob| small(blob)) {
                    let carried = [1.0, blob.heat, blob.shade, blob.age];
                    let cells = &mut self.cells;
                    splat(
                        blob,
                        lattice,
                        &mut self.along,
                        |voxels, along, across, thinned| {
                            for (cell, offset) in cells[voxels].iter_mut().zip(along) {
                                let w = weight(across + offset) * thinned;
                                for (held, carried) in cell.iter_mut().zip(carried) {
                                    *held += w * carried;
                                }
                            }
                        },
                    );
                }
                pack(&mut body.near, lattice, &self.cells);
            }
            None => {
                body.near.dims = [0; 3];
                body.near.texels.clear();
            }
        }
        self.stats.voxels += body.far.voxels() + body.near.voxels();
        self.stats.lattices[slot] = [
            (body.far.voxel, body.far.dims),
            (near.map_or(0.0, |near| near.1), body.near.dims),
        ];
        self.sigma.clear();
        self.sigma
            .extend(self.lights.iter().map(|light| light[0].sqrt()));
        lap(&mut self.stats, 0);
        body.light.clear();
        body.light.resize(count * 4, 255);
        let texels = body.light.as_chunks_mut::<4>().0;
        for (texel, light) in texels.iter_mut().zip(&self.lights) {
            let there = light[2] > 0.0;
            texel[2] = if there { unit(light[1] / light[2]) } else { 0 };
            texel[3] = if there { 255 } else { 0 };
        }
        lap(&mut self.stats, 1);
        self.sweep(dims, voxel as f32, sun, &mut body.light);
        lap(&mut self.stats, 2);
        body.live = true;
        true
    }

    /// The smoke between every voxel of the light grid and the sun, and the sky above it, into the first two of
    /// `out`'s four channels: one pass through the grid slice by slice away from the light, each slice reading the
    /// one before it where the light came through.
    fn sweep(&mut self, dims: [usize; 3], voxel: f32, sun: DVec3, out: &mut [u8]) {
        let [nx, ny, nz] = dims;
        let count = nx * ny * nz;
        let sun = sun.normalize_or(DVec3::Y).as_vec3().to_array();
        let strides = [1, nx, nx * ny];
        // Slices along the axis the light runs most along; `b` and `c` are the other two.
        let a = (0..3)
            .max_by(|&i, &j| sun[i].abs().total_cmp(&sun[j].abs()))
            .expect("three axes");
        let (b, c) = ((a + 1) % 3, (a + 2) % 3);
        let (na, nb, nc) = (dims[a], dims[b], dims[c]);
        // One slice toward the light: this far along it, and this many voxels across.
        let length = voxel / sun[a].abs().max(1e-3);
        let (shift_b, shift_c) = (
            sun[b] / sun[a].abs().max(1e-3),
            sun[c] / sun[a].abs().max(1e-3),
        );
        self.toward.clear();
        self.toward.resize(count, 0.0);
        // The slice nearer the light: its optical depth to the sun plus its own.
        self.slice.clear();
        self.slice.resize(nb * nc, 0.0);
        let from_high = sun[a] > 0.0;
        for step in 0..na {
            let i = if from_high { na - 1 - step } else { step };
            for jc in 0..nc {
                for jb in 0..nb {
                    let index = i * strides[a] + jb * strides[b] + jc * strides[c];
                    let tau = if step == 0 {
                        0.0
                    } else {
                        // Where the light crossed the slice before: between four of its voxels (none outside it).
                        let (fb, fc) = (jb as f32 + shift_b, jc as f32 + shift_c);
                        let (b0, c0) = (fb.floor(), fc.floor());
                        let (wb, wc) = (fb - b0, fc - c0);
                        let at = |db: isize, dc: isize| {
                            let (pb, pc) = (b0 as isize + db, c0 as isize + dc);
                            if pb < 0 || pc < 0 || pb >= nb as isize || pc >= nc as isize {
                                0.0
                            } else {
                                self.slice[pc as usize * nb + pb as usize]
                            }
                        };
                        (at(0, 0) * (1.0 - wb) + at(1, 0) * wb) * (1.0 - wc)
                            + (at(0, 1) * (1.0 - wb) + at(1, 1) * wb) * wc
                    };
                    self.toward[index] = tau;
                }
            }
            for jc in 0..nc {
                for jb in 0..nb {
                    let index = i * strides[a] + jb * strides[b] + jc * strides[c];
                    self.slice[jc * nb + jb] = self.toward[index] + self.sigma[index] * length;
                }
            }
        }
        let encode = |tau: f32| ((-tau / TAU_SCALE).exp() * 255.0 + 0.5) as u8;
        // The sky above: straight up through the grid.
        for z in 0..nz {
            for x in 0..nx {
                let mut above = 0.0;
                for y in (0..ny).rev() {
                    let index = (z * ny + y) * nx + x;
                    out[index * 4] = encode(self.toward[index]);
                    out[index * 4 + 1] = encode(above);
                    above += self.sigma[index] * voxel;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fire_batch::FireMode;

    fn camera() -> CameraView {
        let camera = naval_render::Camera::look_at(
            glam::Vec3::new(0.0, 30.0, 200.0),
            glam::Vec3::new(0.0, 60.0, 0.0),
        );
        CameraView::from_uniforms(&camera.uniforms(1280, 720, naval_render::FrameTime::default()))
    }

    /// A column of puffs climbing from the origin, as a ship's fire leaves them.
    fn column(ship: &SourceId, puffs: usize) -> FireBatch {
        let mut smoke = FireBatch::new(1600, FireMode::Smoke);
        for i in 0..puffs {
            let p = smoke.emit(DVec3::new(0.0, 12.0, 0.0), Some(ship));
            p.size = 4.5;
            p.growth = 1.3;
            p.growth_decay = 0.05;
            p.diffusion = 0.05;
            p.thin = 0.55;
            p.life = 40.0;
            p.rise = 9.0;
            p.rise_time = 8.0;
            p.lift = 2.6;
            p.shear = 1.0;
            p.shear_height = 70.0;
            p.opacity = 0.8;
            p.fade_in = 0.5;
            p.albedo = Rgb::new(0.11, 0.1, 0.086);
            p.age = -(i as f64) * 0.4;
        }
        smoke.advance(puffs as f64 * 0.4, DVec3::new(2.4, 0.0, 0.9));
        smoke
    }

    /// The outermost layer of a grid holds nothing: a clear voxel on every side.
    fn rim_is_clear(grid: &Grid, channel: usize, stride: usize, texels: &[u8]) {
        let [nx, ny, nz] = grid.dims;
        for z in 0..nz {
            for y in 0..ny {
                for x in 0..nx {
                    let rim =
                        x == 0 || y == 0 || z == 0 || x == nx - 1 || y == ny - 1 || z == nz - 1;
                    if rim {
                        let texel = texels[((z * ny + y) * nx + x) * stride + channel];
                        assert_eq!(texel, 0, "{x} {y} {z}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_ships_smoke_becomes_one_body_and_leaves_the_sprites() {
        let ship: SourceId = "enemy-1".into();
        let mut smoke = column(&ship, 90);
        let mut gas = ParticlePool::new(64, crate::particles::PoolKind::Gas);
        let camera = camera();
        assert!(!smoke.publish(&camera, None).quads.is_empty());
        let mut volumes = Volumes::default();
        let mut frame = VolumeFrame::default();
        let sun = DVec3::new(-0.55, 0.74, -0.39);
        let wind = DVec3::ZERO;
        volumes.publish(&camera, None, &mut smoke, &mut gas, sun, wind, &mut frame);
        let body = &frame.bodies[0];
        assert!(body.live && !frame.bodies[1].live && !frame.bodies[2].live);
        // The large puffs in the coarse grid, the young ones about the fire in a finer one inside it.
        let (far, near) = (&body.far, &body.near);
        assert_eq!(far.texels.len(), far.voxels() * 4);
        assert_eq!(near.texels.len(), near.voxels() * 4);
        assert_eq!(body.light.len(), far.voxels() * 4);
        assert!(near.voxels() > 0 && near.voxel < far.voxel);
        assert!(far.voxels() <= FAR_BUDGET && near.voxels() <= NEAR_BUDGET);
        for grid in [far, near] {
            assert!(grid.dims.iter().all(|&n| n <= MAX_DIM));
            // The lattice is fixed in the world.
            let steps = grid.origin / grid.voxel;
            assert!((steps - steps.round()).abs().max_element() < 1e-9);
            rim_is_clear(grid, 0, 4, &grid.texels);
            assert!(grid.texels.chunks(4).any(|t| t[0] > 100));
        }
        // The light grid marks where the smoke is, short of its rim.
        rim_is_clear(far, 3, 4, &body.light);
        // Smoke shades what lies under it from the sky, and what lies beyond it from the sun.
        assert!(body.light.chunks(4).any(|l| l[0] < 128));
        assert!(body.light.chunks(4).any(|l| l[1] < 128));
        assert!(body.light.chunks(4).any(|l| l[0] == 255));
        // A puff's strength, where there is one.
        assert!(body.light.chunks(4).any(|l| l[3] == 255 && l[2] > 100));
        // The ship's sprites are gone; another ship's would stay.
        assert!(smoke.publish(&camera, None).quads.is_empty());
        assert!(volumes.stats.blobs > 40);
        // The same puffs give the same grids.
        let first = (far.texels.clone(), near.texels.clone());
        volumes.publish(&camera, None, &mut smoke, &mut gas, sun, wind, &mut frame);
        assert!(first.0 == frame.bodies[0].far.texels && first.1 == frame.bodies[0].near.texels);
    }

    #[test]
    fn only_the_newest_detonation_is_marched() {
        let mut smoke = FireBatch::new(16, FireMode::Smoke);
        let mut gas = ParticlePool::new(64, crate::particles::PoolKind::Gas);
        let mut volumes = Volumes::default();
        let seat = DVec3::new(0.0, 6.0, 0.0);
        for _ in 0..2 {
            gas.tag = volumes.detonate(seat);
            for i in 0..8 {
                let p = gas.emit(seat + DVec3::new(i as f64 * 6.0, 20.0, 0.0), None);
                p.size = 30.0;
                p.life = 60.0;
                p.heat = 2.0;
                p.cooling = 2.0;
                p.density = 3.0;
            }
            gas.tag = 0;
        }
        gas.advance(0.5, DVec3::ZERO);
        let camera = camera();
        let mut frame = VolumeFrame::default();
        let (sun, wind) = (DVec3::Y, DVec3::ZERO);
        volumes.publish(&camera, None, &mut smoke, &mut gas, sun, wind, &mut frame);
        let body = &frame.bodies[COLUMNS];
        assert!(body.live && body.kind == BodyKind::Detonation);
        assert_eq!(body.blobs, 8);
        assert!(
            body.far.texels.chunks(4).any(|t| t[1] > 100),
            "the fireball is hot"
        );
        // The older detonation's eight puffs are still sprites.
        assert_eq!(gas.publish_gas(&camera, None).len(), 8);
    }
}
