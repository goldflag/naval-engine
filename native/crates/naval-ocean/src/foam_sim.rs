//! The foam ship wakes and shell splashes leave on the sea, as a layer the water stirs, after Crest 4's foam simulation
//! and the Atlas talk at GDC 2019. A field of [`FoamGrid`] cells about the focus hull (astern of her middle under way,
//! so more of her wake lies in it), stepped at 30 Hz on the GPU (`shaders/foam_sim.wgsl` says how: carried by the drift
//! and the eddies the churn drives, spread, decayed) and fed by each stern's churned water and each splash's water as
//! its column falls back. The surface reads it within its window (`wake.wgsl`'s `sim_read`) and draws the foam itself
//! beyond (`trail_read`, `splash_read`), fading from one to the other over the window's edge and wherever the field has
//! not yet run long enough to hold what lies there (each cell's validity).
//!
//! Addressed toroidally: the field's cell k lies in texel k mod the cells along an edge, so a window that moves clears
//! only the cells it takes in, with no copy. Its cells are laid in the field's own frame, which a theatre re-centre
//! moves as a whole ([`FoamSim::translate`]): the foam stays on its water. A cell the window takes in starts from what
//! the surface draws there (the trails' atlas and the splashes' deposits), so the foam carries on into the field. Its
//! two states are the layers of one texture, which the surface binds as an array.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use naval_render::{
    MakePipelines, PipelineId, PipelineRegistry, RenderContext, ShaderError, compute_recipe,
};

use crate::QualityTier;
use crate::shaders;
use crate::splash_foam::{SplashDeposits, splash_radius};
use crate::trail::{ATLAS_CAPACITY, TileRows};

/// The field's cells along each edge and a cell's edge (m) on a tier (`wake.wgsl` reads both from `Wake.sim_grid`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FoamGrid {
    pub cells: u32,
    pub cell: f64,
}

impl FoamGrid {
    /// Graphics → Ocean: 1,024² cells of 0.75 m (768 m) on High and Ultra; on Medium half the cells over the same
    /// water, as the wake field halves its own there; none on Low, whose foam is all drawn.
    pub fn for_tier(tier: QualityTier) -> Option<Self> {
        match tier {
            QualityTier::Low => None,
            QualityTier::Medium => Some(Self {
                cells: 512,
                cell: 1.5,
            }),
            QualityTier::High | QualityTier::Ultra => Some(Self {
                cells: 1024,
                cell: 0.75,
            }),
        }
    }

    /// The window's edge (m).
    pub fn edge(self) -> f64 {
        f64::from(self.cells) * self.cell
    }

    pub fn half(self) -> f64 {
        0.5 * self.edge()
    }
}

/// The field's fixed step (s), and the most a frame runs: a frame longer than they cover (a time scale above about 3× at
/// 30 fps) runs that many longer steps instead, which share all its time ([`schedule`]).
pub const STEP: f64 = 1.0 / 30.0;
const MAX_STEPS: usize = 3;
/// The longest a step's flow runs (s): a fresh splash's base surge (its radius growing 1.3 times itself a second) then
/// carries its water at most a quarter of its radius in a step, and the eddies 0.4 m. A frame longer than
/// [`MAX_STEPS`] of these (a time scale above 18× at 30 fps, 36× at 60) has each step stand for more game seconds than
/// its flow runs: the foam's lives and the splashes' landing take all of them, so the wakes keep their length.
const LONGEST: f64 = 0.2;
/// The most game seconds a step stands for: a frame longer than three minutes leaves the rest out (the foam is gone
/// either way).
const LONGEST_SPAN: f64 = 60.0;
/// Sterns and splashes a step takes: the sterns in the window, the youngest splashes.
pub const STERN_SLOTS: usize = 8;
pub const SPLASH_SLOTS: usize = 32;
/// A splash's column has fallen back onto the sea this long (s) after it fell (`foam_sim.wgsl`'s landing ramp).
pub(crate) const LANDING: f64 = 10.5;
/// Under way at her full speed, the window's centre lies this share of its edge astern of the focus hull's middle.
const ASTERN: f64 = 0.22;
/// A stern's churned water: the thickness laid down her track's middle at full churn, its width (the Gaussian's,
/// exp(−2 d²/w²), a share of her beam) and her boils' size (a share of her beam, as the relief's `RELIEF_LUMP`).
const STERN_THICKNESS: f64 = 2.2;
const STERN_WIDTH: f64 = 0.45;
const STERN_BOILS: f64 = 0.33;
/// A stern that moved further than this (m) beyond what her speed carries her between the frames that step was put there
/// (the wake field's generators' teleport): no churned water is laid between.
const JUMP: f64 = 100.0;
/// Bytes between the steps' parameters in their buffer (dynamic offsets).
const SLOT: u64 = 4096;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct Params {
    window: [i32; 4],
    counts: [i32; 4],
    clock: [f32; 4],
    drift: [f32; 4],
    grid: [f32; 4],
    seed: [f32; 4],
    sterns: [[f32; 4]; 2 * STERN_SLOTS],
    splashes: [[f32; 4]; 2 * SPLASH_SLOTS],
    tiles: [[f32; 4]; 2 * ATLAS_CAPACITY],
}
const _: () = assert!(std::mem::size_of::<Params>() as u64 <= SLOT);

/// A stern feeding the field ([`stern_source`]): where her churned water comes up (world x, z), her beam (m), her churn
/// (0–1, as `trail.rs` works it out from her speed as far as her wake reaches the surface) and her speed (m/s, what
/// carries her between frames: [`stern_run`]'s jump test).
#[derive(Clone, Debug, PartialEq)]
pub struct SimStern {
    pub id: String,
    pub at: [f64; 2],
    pub beam: f64,
    pub churn: f64,
    pub speed: f64,
}

/// What a frame steps the field with.
pub struct FoamFrame<'a> {
    /// The frame's game seconds.
    pub dt: f64,
    /// Where the window goes ([`place_window`]).
    pub centre: [f64; 2],
    /// The surface's drift (m/s, world x and z).
    pub drift: [f64; 2],
    pub sterns: &'a [SimStern],
    pub splashes: &'a SplashDeposits,
    /// The splashes' clock (s).
    pub now: f64,
    /// The trails the surface draws: their atlas, their tiles' rows and its knobs (foam start, full and cover, and the
    /// slick's full), which a cell the window takes in starts from.
    pub trail_atlas: &'a wgpu::TextureView,
    pub tiles: &'a [TileRows],
    pub shading: [f32; 4],
}

/// The field's pipelines on the renderer's registry ([`FoamSim::register_pipelines`]).
#[derive(Clone, Copy, Debug)]
pub struct FoamSimPipelines {
    advect: PipelineId,
    settle: PipelineId,
}

struct Textures {
    state: wgpu::Texture,
    carried: wgpu::Texture,
    /// Both states as one array: the surface's binding.
    states: wgpu::TextureView,
    /// Advect from state 0, settle state 0 into 1, advect from 1, settle 1 into 0.
    groups: [wgpu::BindGroup; 4],
}

pub struct FoamSim {
    layout: wgpu::BindGroupLayout,
    advect: wgpu::ComputePipeline,
    settle: wgpu::ComputePipeline,
    sampler: wgpu::Sampler,
    params: wgpu::Buffer,
    /// The field's size: none on a tier without one.
    grid: Option<FoamGrid>,
    /// Made at the field's first step.
    textures: Option<Textures>,
    /// The state the last step wrote (its layer).
    newest: usize,
    accumulator: f64,
    clock: f64,
    /// The game seconds since the sterns were last laid (what their speeds carry them since).
    unlaid: f64,
    /// The window's first cell at the last step, in the field's frame.
    window: Option<[i64; 2]>,
    /// Where the field's frame lies in the scene's metres (a scene point p is the field's p − offset): a re-centre moves
    /// it with the scene, so the foam stays on its water and no cell is cleared.
    offset: [f64; 2],
    /// The next step clears every cell (a new field, or one forgotten).
    clear: bool,
    /// How far the drift has carried the eddies' frame (m).
    drifted: [f64; 2],
    /// Where each stern's churned water was laid to at the last step (scene metres).
    laid: HashMap<String, [f64; 2]>,
    pub steps_run: u64,
}

fn layout_entry(binding: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn sampled() -> wgpu::BindingType {
    wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
    }
}

/// A hull as a stern feeding the field: her speed taken as far as her wake reaches the surface
/// ([`crate::ship_wake::surfaced`], as her trail and her waves take it), so a submarine running submerged lays none.
/// None while that speed is under 0.1 m/s.
pub fn stern_source(ship: &crate::ship_wake::WakeShip<'_>) -> Option<SimStern> {
    let speed = ship.speed * crate::ship_wake::surfaced(ship.position[1]);
    (speed.abs() > 0.1).then(|| SimStern {
        id: ship.id.to_owned(),
        at: stern_of(
            ship.position,
            ship.heading,
            ship.speed,
            ship.length,
            [ship.center_x, ship.center_z],
        ),
        beam: ship.beam,
        churn: churn(speed, ship.forward_speed),
        speed: ship.speed,
    })
}

/// Where a hull's churned water comes up: her stern (her bow going astern), as `trail.rs`'s samples lay it.
pub fn stern_of(
    position: [f64; 3],
    heading: f64,
    speed: f64,
    length: f64,
    center: [f64; 2],
) -> [f64; 2] {
    let forward = [heading.sin(), -heading.cos()];
    let right = [-forward[1], forward[0]];
    let middle = [
        position[0] + right[0] * center[0] - forward[0] * center[1],
        position[2] + right[1] * center[0] - forward[1] * center[1],
    ];
    let aft = if speed >= 0.0 { 1.0 } else { -1.0 } * length * 0.468;
    [middle[0] - forward[0] * aft, middle[1] - forward[1] * aft]
}

/// A hull's churn at `speed` (`trail.rs`'s, without the turn): her screws' wash grows with her speed's share of her
/// full speed, 0–1.
pub fn churn(speed: f64, forward_speed: f64) -> f64 {
    let ratio = (speed.abs() / forward_speed.max(0.1)).min(1.0);
    let smooth = ratio * ratio * (3.0 - 2.0 * ratio);
    smooth.powf(1.3)
}

/// The window stays about the focus hull while the view looks at water within this share of its half-edge of where
/// it would lie about her; past that (a camera held over a fall of shot far from her) it goes about that water.
const HULL_REACH: f64 = 0.6;
/// The furthest (m) along the view the water it looks at is taken (a view near the horizon).
const AIM_REACH: f64 = 300.0;

/// Where the window's centre goes: about the focus hull (`hull`, from [`window_centre`]) while the view (`eye`,
/// `forward`) looks at water near it, else about the water the view looks at. `half` is the window's half-edge (m).
pub fn place_window(
    hull: Option<[f64; 2]>,
    eye: [f64; 3],
    forward: [f64; 3],
    half: f64,
) -> Option<[f64; 2]> {
    let across = forward[0].hypot(forward[2]);
    let aim = if across > 1e-6 {
        let run = if forward[1] < -1e-3 {
            (eye[1].max(0.0) / -forward[1] * across).min(AIM_REACH)
        } else {
            AIM_REACH
        };
        [
            eye[0] + forward[0] / across * run,
            eye[2] + forward[2] / across * run,
        ]
    } else {
        [eye[0], eye[2]]
    };
    let reach = HULL_REACH * half;
    match hull {
        Some(h) if (h[0] - aim[0]).hypot(h[1] - aim[1]) <= reach => Some(h),
        _ if aim.iter().all(|v| v.is_finite()) => Some(aim),
        _ => hull,
    }
}

/// Where the window's centre goes for a focus hull at `position` heading `heading` at `speed` of `forward_speed`:
/// astern of her middle by [`ASTERN`] of the window's `edge` (m) at full speed.
pub fn window_centre(
    position: [f64; 3],
    heading: f64,
    speed: f64,
    forward_speed: f64,
    edge: f64,
) -> [f64; 2] {
    let share = (speed / forward_speed.max(0.1)).clamp(-1.0, 1.0);
    let astern = ASTERN * edge * share;
    [
        position[0] - heading.sin() * astern,
        position[2] + heading.cos() * astern,
    ]
}

/// The window's first cell, in the field's frame, for a window about `centre` (scene metres) with the frame at `offset`.
fn first_cell(centre: [f64; 2], offset: [f64; 2], grid: FoamGrid) -> [i64; 2] {
    let half = i64::from(grid.cells / 2);
    [0, 1].map(|i| ((centre[i] - offset[i]) / grid.cell).floor() as i64 - half)
}

/// The window's centre cell: its corner in the frame's metres, and in the scene's.
fn window_origin(low: [i64; 2], offset: [f64; 2], grid: FoamGrid) -> ([f64; 2], [f64; 2]) {
    let half = i64::from(grid.cells / 2);
    let frame = [0, 1].map(|i| (low[i] + half) as f64 * grid.cell);
    (frame, [frame[0] + offset[0], frame[1] + offset[1]])
}

/// `Wake.sim` and `Wake.sim_grid` (but its last two values) for the window from `low`: its centre cell's corner (scene
/// m) and that cell's texel (each axis), then a cell's edge and the cells along an edge.
fn window_rows(low: [i64; 2], offset: [f64; 2], grid: FoamGrid) -> [[f32; 4]; 2] {
    let n = i64::from(grid.cells);
    let (_, corner) = window_origin(low, offset, grid);
    let centre = [0, 1].map(|i| (low[i] + n / 2).rem_euclid(n) as f32);
    [
        [corner[0] as f32, corner[1] as f32, centre[0], centre[1]],
        [grid.cell as f32, grid.cells as f32, 0.0, 0.0],
    ]
}

/// How a frame steps the field ([`schedule`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steps {
    pub count: usize,
    /// Each step's flow (s): [`STEP`], or an equal share of a longer frame, at most [`LONGEST`].
    pub flow: f64,
    /// The game seconds each step stands for (its foam's decay, its splashes' landing): its flow's, or more once a
    /// frame outruns [`MAX_STEPS`] of [`LONGEST`].
    pub span: f64,
    /// What the field's clock owes after them (s, less than a step).
    pub owed: f64,
}

/// The steps covering `owed` seconds (what the last frame left and this frame's time): whole fixed steps while at most
/// [`MAX_STEPS`] of them do, as at game speed; past that, [`MAX_STEPS`] longer steps that share all the whole steps'
/// time, their flow held to [`LONGEST`] and their span the rest (at most [`LONGEST_SPAN`]). No time is dropped.
pub fn schedule(owed: f64) -> Steps {
    let whole = (owed / STEP).floor();
    if whole <= MAX_STEPS as f64 {
        let count = whole.max(0.0) as usize;
        return Steps {
            count,
            flow: STEP,
            span: STEP,
            owed: owed - count as f64 * STEP,
        };
    }
    let span = whole * STEP / MAX_STEPS as f64;
    Steps {
        count: MAX_STEPS,
        flow: span.min(LONGEST),
        span: span.min(LONGEST_SPAN),
        owed: owed - whole * STEP,
    }
}

/// The run a stern's churned water is laid along in step `k` of a frame's `steps`: from where the last step laid it to
/// toward where she is now (`at`), an equal share of the way each step, so the runs join however long they are. A point
/// (nothing laid) where she first appears or has moved more than [`JUMP`] further than `reach` (m, what her speed
/// carries her since the last frame that stepped).
pub fn stern_run(
    laid: Option<[f64; 2]>,
    at: [f64; 2],
    reach: f64,
    k: usize,
    steps: usize,
) -> ([f64; 2], [f64; 2]) {
    let from = laid.unwrap_or(at);
    if (at[0] - from[0]).hypot(at[1] - from[1]) > JUMP + reach.max(0.0) || steps == 0 {
        return (at, at);
    }
    let point = |share: f64| {
        [
            from[0] + (at[0] - from[0]) * share,
            from[1] + (at[1] - from[1]) * share,
        ]
    };
    (
        point(k as f64 / steps as f64),
        point((k + 1) as f64 / steps as f64),
    )
}

/// The splashes a step lays, youngest first and at most [`SPLASH_SLOTS`] (more falling at once leave the oldest of
/// them out, whose foam laid so far stays): those still landing within the window and, when the step takes in cells
/// (`seeding`), the older ones lying there too, which those cells start from. Their rows (`foam_sim.wgsl`'s
/// `params.splashes`, places m from `origin`) at `age_at` on the splashes' clock; returns how many.
fn splash_rows(
    splashes: &SplashDeposits,
    age_at: f64,
    origin: [f64; 2],
    half: f64,
    seeding: bool,
    rows: &mut [[f32; 4]],
) -> usize {
    let mut count = 0;
    for splash in splashes.list.iter().rev() {
        let age = age_at - splash.born;
        if age > LANDING && !seeding {
            break;
        }
        let rel = [splash.x - origin[0], splash.z - origin[1]];
        let scale = splash.shown_scale(age_at);
        let reach = 2.1 * splash_radius(age, scale) + splash.skew[0].hypot(splash.skew[1]);
        if age <= 0.0 || rel[0].abs().max(rel[1].abs()) > half + reach {
            continue;
        }
        rows[2 * count] = [rel[0] as f32, rel[1] as f32, age as f32, scale as f32];
        rows[2 * count + 1] = [
            splash.seed as f32,
            splash.skew[0] as f32,
            splash.skew[1] as f32,
            reach as f32,
        ];
        count += 1;
        if count == SPLASH_SLOTS {
            break;
        }
    }
    count
}

impl FoamSim {
    /// A field of `grid` (none: no field, the foam all drawn).
    pub fn new(device: &wgpu::Device, grid: Option<FoamGrid>) -> Self {
        Self::build(device, grid, MakePipelines::Now)
    }

    pub(crate) fn build(
        device: &wgpu::Device,
        grid: Option<FoamGrid>,
        make: MakePipelines<'_>,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("foam sim"),
            entries: &[
                layout_entry(0, sampled()),
                layout_entry(1, sampled()),
                layout_entry(
                    2,
                    wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                ),
                layout_entry(
                    3,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(
                            std::mem::size_of::<Params>() as u64
                        ),
                    },
                ),
                layout_entry(
                    4,
                    wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                ),
                // The trails' atlas (`crate::stamps`), which a cell the window takes in starts from.
                layout_entry(5, sampled()),
            ],
        });
        let module = make.module(|| shaders::shader_module(device, shaders::FOAM_SIM, &[]));
        let pipeline = |label: &str, entry: &str| {
            make.compute(|| {
                device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(
                        &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                            label: Some(label),
                            bind_group_layouts: &[Some(&layout)],
                            immediate_size: 0,
                        }),
                    ),
                    module: module.as_ref().expect("built now"),
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                })
            })
        };
        let advect = pipeline("foam sim advect", "advect_pass");
        let settle = pipeline("foam sim settle", "settle_pass");
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("foam sim"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("foam sim steps"),
            size: SLOT * MAX_STEPS as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            layout,
            advect,
            settle,
            sampler,
            params,
            grid,
            textures: None,
            newest: 0,
            accumulator: 0.0,
            clock: 0.0,
            unlaid: 0.0,
            window: None,
            offset: [0.0; 2],
            clear: true,
            drifted: [0.0; 2],
            laid: HashMap::new(),
            steps_run: 0,
        }
    }

    /// Builds both passes through the renderer's registry (they then hot-reload). The ocean's files must be registered
    /// first (`crate::shaders::register`).
    pub fn register_pipelines(
        &self,
        ctx: &mut RenderContext<'_>,
    ) -> Result<FoamSimPipelines, ShaderError> {
        let mut add = |label: &str, entry| {
            ctx.add_pipeline(
                label,
                shaders::FOAM_SIM.path,
                Vec::new(),
                compute_recipe(vec![self.layout.clone()], entry),
            )
        };
        Ok(FoamSimPipelines {
            advect: add("foam sim advect", "advect_pass")?,
            settle: add("foam sim settle", "settle_pass")?,
        })
    }

    /// Takes the registry's current pipelines; whether any changed.
    pub fn adopt_pipelines(&mut self, registry: &PipelineRegistry, ids: &FoamSimPipelines) -> bool {
        let mut changed = false;
        for (slot, id) in [
            (&mut self.advect, ids.advect),
            (&mut self.settle, ids.settle),
        ] {
            let new = registry.compute(id);
            if slot != new {
                *slot = new.clone();
                changed = true;
            }
        }
        changed
    }

    pub fn grid(&self) -> Option<FoamGrid> {
        self.grid
    }

    /// Size the field for another tier: its textures go and are made again at the next step, every cell cleared (the
    /// surface draws the foam itself while the new field's cells come to hold it).
    pub fn set_grid(&mut self, grid: Option<FoamGrid>) {
        if self.grid != grid {
            self.grid = grid;
            self.textures = None;
            self.reset();
        }
    }

    /// Both states as one array texture, once the field has stepped.
    pub fn states(&self) -> Option<&wgpu::TextureView> {
        self.textures.as_ref().map(|t| &t.states)
    }

    /// `Wake.foam_state`, `Wake.sim` and `Wake.sim_grid`: the splashes' deposits in use (`count`), the state the surface
    /// reads (its layer) and 1 once there is a field to read; the window ([`window_rows`]).
    pub fn rows(&self, count: usize) -> [[f32; 4]; 3] {
        match (self.grid, self.window, &self.textures) {
            (Some(grid), Some(low), Some(_)) => {
                let [sim, sim_grid] = window_rows(low, self.offset, grid);
                [[count as f32, self.newest as f32, 1.0, 0.0], sim, sim_grid]
            }
            _ => [[count as f32, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4]],
        }
    }

    /// The window's centre (scene metres), once it has stepped.
    pub fn centre(&self) -> Option<[f64; 2]> {
        let (grid, low) = (self.grid?, self.window?);
        Some(window_origin(low, self.offset, grid).1)
    }

    /// Forget the foam (returning to port): the next step clears every cell.
    pub fn reset(&mut self) {
        self.window = None;
        self.accumulator = 0.0;
        self.unlaid = 0.0;
        self.laid.clear();
        self.clear = true;
    }

    /// Move all the field holds by `shift` metres (a theatre re-centre): its frame moves with the scene, so no cell is
    /// cleared and its foam stays where it lay on the sea.
    pub fn translate(&mut self, shift: [f64; 2]) {
        self.offset = [self.offset[0] + shift[0], self.offset[1] + shift[1]];
        for point in self.laid.values_mut() {
            point[0] += shift[0];
            point[1] += shift[1];
        }
    }

    /// Take the field of `old` (from a scene going away), moved `shift` metres as a theatre re-centre moves the scene's
    /// metres: its textures, rebound with this one's pipelines and the new scene's trails, its clock and window. False,
    /// and nothing taken, for a field of another size (the graphics changed) or none yet.
    pub fn take_from(
        &mut self,
        device: &wgpu::Device,
        old: &mut FoamSim,
        shift: [f64; 2],
        trail_atlas: &wgpu::TextureView,
    ) -> bool {
        if old.grid != self.grid || old.textures.is_none() {
            return false;
        }
        let Textures { state, carried, .. } = old.textures.take().expect("checked above");
        self.textures = Some(self.bind(device, state, carried, trail_atlas));
        self.newest = old.newest;
        self.accumulator = old.accumulator;
        self.clock = old.clock;
        self.unlaid = old.unlaid;
        self.window = old.window;
        self.offset = old.offset;
        self.clear = old.clear;
        self.drifted = old.drifted;
        self.laid = std::mem::take(&mut old.laid);
        self.steps_run = old.steps_run;
        self.translate(shift);
        true
    }

    fn make_textures(
        &self,
        device: &wgpu::Device,
        grid: FoamGrid,
        trail_atlas: &wgpu::TextureView,
    ) -> Textures {
        let texture = |label: &str, layers: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: grid.cells,
                    height: grid.cells,
                    depth_or_array_layers: layers,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        self.bind(
            device,
            texture("foam sim states", 2),
            texture("foam sim carried", 1),
            trail_atlas,
        )
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        state: wgpu::Texture,
        carried: wgpu::Texture,
        trail_atlas: &wgpu::TextureView,
    ) -> Textures {
        let layer = |i: u32| {
            state.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: i,
                array_layer_count: Some(1),
                ..Default::default()
            })
        };
        let layers = [layer(0), layer(1)];
        let carried_view = carried.create_view(&wgpu::TextureViewDescriptor::default());
        let states = state.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let params = wgpu::BufferBinding {
            buffer: &self.params,
            offset: 0,
            size: wgpu::BufferSize::new(std::mem::size_of::<Params>() as u64),
        };
        let group =
            |from: &wgpu::TextureView, other: &wgpu::TextureView, to: &wgpu::TextureView| {
                let view = wgpu::BindingResource::TextureView;
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("foam sim"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: view(from),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: view(other),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: view(to),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: wgpu::BindingResource::Buffer(params.clone()),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: wgpu::BindingResource::Sampler(&self.sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 5,
                            resource: view(trail_atlas),
                        },
                    ],
                })
            };
        let groups = [
            group(&layers[0], &layers[1], &carried_view),
            group(&layers[0], &carried_view, &layers[1]),
            group(&layers[1], &layers[0], &carried_view),
            group(&layers[1], &carried_view, &layers[0]),
        ];
        Textures {
            state,
            carried,
            states,
            groups,
        }
    }

    /// Advance by the `frame`'s seconds ([`schedule`]: at most [`MAX_STEPS`] steps, longer ones under a time scale; a
    /// paused frame runs none), each step's compute pass timed by the renderer core's timer as `ocean foam step`. Returns
    /// true when it made its textures (the surface must rebind).
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &mut naval_render::UploadSink<'_>,
        encoder: &mut wgpu::CommandEncoder,
        timer: Option<&naval_render::GpuTimer>,
        frame: &FoamFrame<'_>,
    ) -> bool {
        let Some(grid) = self.grid else {
            return false;
        };
        let made = self.textures.is_none();
        if made {
            self.textures = Some(self.make_textures(device, grid, frame.trail_atlas));
            self.clear = true;
        }
        let dt = if frame.dt.is_finite() {
            frame.dt.max(0.0)
        } else {
            0.0
        };
        let Steps {
            count: steps,
            flow,
            span,
            owed,
        } = schedule(self.accumulator + dt);
        self.accumulator = owed;
        self.unlaid += dt;
        let drift = if frame.drift.iter().all(|v| v.is_finite()) {
            frame.drift
        } else {
            [0.0; 2]
        };
        let half = grid.half();
        let groups = grid.cells.div_ceil(8);
        for k in 0..steps {
            let low = first_cell(frame.centre, self.offset, grid);
            let before = self.window.unwrap_or(low);
            let clear = std::mem::take(&mut self.clear);
            // Cells taken in this step start from what the surface draws there.
            let seeding = clear || low != before;
            let (frame_origin, origin) = window_origin(low, self.offset, grid);
            self.drifted[0] += drift[0] * flow;
            self.drifted[1] += drift[1] * flow;
            let mut params = Params::zeroed();
            params.window = [
                low[0] as i32,
                low[1] as i32,
                before[0] as i32,
                before[1] as i32,
            ];
            params.clock = [
                flow as f32,
                self.clock as f32,
                (frame_origin[0] - self.drifted[0]) as f32,
                (frame_origin[1] - self.drifted[1]) as f32,
            ];
            params.drift = [drift[0] as f32, drift[1] as f32, 0.0, 0.0];
            params.grid = [grid.cell as f32, grid.cells as f32, span as f32, 0.0];
            params.seed = frame.shading;
            // The sterns, each laid along the run her stern made this step.
            let mut count = 0;
            for stern in frame.sterns {
                let reach = stern.speed.abs() * self.unlaid;
                let (from, to) =
                    stern_run(self.laid.get(&stern.id).copied(), stern.at, reach, k, steps);
                let width = (STERN_WIDTH * stern.beam).max(2.0 * grid.cell);
                let from = [from[0] - origin[0], from[1] - origin[1]];
                let to = [to[0] - origin[0], to[1] - origin[1]];
                let mid = [0.5 * (from[0] + to[0]), 0.5 * (from[1] + to[1])];
                let run = (to[0] - from[0]).hypot(to[1] - from[1]);
                if mid[0].abs().max(mid[1].abs()) > half + 2.5 * width + 0.5 * run {
                    continue;
                }
                params.sterns[2 * count] =
                    [from[0] as f32, from[1] as f32, to[0] as f32, to[1] as f32];
                params.sterns[2 * count + 1] = [
                    width as f32,
                    (STERN_THICKNESS * stern.churn) as f32,
                    stern.churn as f32,
                    (STERN_BOILS * stern.beam).max(3.0) as f32,
                ];
                count += 1;
                if count == STERN_SLOTS {
                    break;
                }
            }
            params.counts[0] = count as i32;
            let age_at = frame.now - (steps - 1 - k) as f64 * span;
            params.counts[1] = splash_rows(
                frame.splashes,
                age_at,
                origin,
                half,
                seeding,
                &mut params.splashes,
            ) as i32;
            params.counts[2] = i32::from(clear);
            if seeding {
                let tiles = frame.tiles.iter().take(ATLAS_CAPACITY);
                for (i, row) in tiles.enumerate() {
                    let at = |v: [f32; 4]| {
                        [
                            (f64::from(v[0]) - origin[0]) as f32,
                            (f64::from(v[1]) - origin[1]) as f32,
                            v[2],
                            v[3],
                        ]
                    };
                    params.tiles[2 * i] = at(row.center);
                    params.tiles[2 * i + 1] = at(row.slick_center);
                }
                params.counts[3] = frame.tiles.len().min(ATLAS_CAPACITY) as i32;
            }
            let offset = SLOT * k as u64;
            queue.write_buffer(&self.params, offset, bytemuck::bytes_of(&params));
            let textures = self.textures.as_ref().expect("made above");
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("foam sim"),
                timestamp_writes: timer.and_then(|timer| timer.compute_pass("ocean foam step")),
            });
            pass.set_pipeline(&self.advect);
            pass.set_bind_group(0, &textures.groups[2 * self.newest], &[offset as u32]);
            pass.dispatch_workgroups(groups, groups, 1);
            pass.set_pipeline(&self.settle);
            pass.set_bind_group(0, &textures.groups[2 * self.newest + 1], &[offset as u32]);
            pass.dispatch_workgroups(groups, groups, 1);
            drop(pass);
            self.newest = 1 - self.newest;
            self.window = Some(low);
            self.clock += flow;
            self.steps_run += 1;
        }
        if steps > 0 {
            self.laid = frame.sterns.iter().map(|s| (s.id.clone(), s.at)).collect();
            self.unlaid = 0.0;
            if std::env::var_os("NAVAL_FOAM_SIM_DEBUG").is_some()
                && self.steps_run % 150 < steps as u64
            {
                eprintln!(
                    "[foam sim] step {} about ({:.0}, {:.0}), {} sterns, {} splashes, drift ({:.2}, {:.2})",
                    self.steps_run,
                    frame.centre[0],
                    frame.centre[1],
                    frame.sterns.len(),
                    frame.splashes.list.len(),
                    drift[0],
                    drift[1]
                );
            }
        }
        made
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::splash_foam::SplashDeposit;

    const HIGH: FoamGrid = FoamGrid {
        cells: 1024,
        cell: 0.75,
    };

    /// `foam_sim.wgsl`'s `erf` (Abramowitz and Stegun 7.1.26).
    fn erf(x: f64) -> f64 {
        let t = 1.0 / (1.0 + 0.3275911 * x.abs());
        let y = 1.0
            - ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
                + 0.254829592)
                * t
                * (-x * x).exp();
        y.copysign(x)
    }

    /// `foam_sim.wgsl`'s `swept`: the share of a stern's churned water a run `from`–`to` lays at `p`.
    fn swept(p: [f64; 2], from: [f64; 2], to: [f64; 2], width: f64) -> f64 {
        let run = [to[0] - from[0], to[1] - from[1]];
        let span = run[0].hypot(run[1]);
        let unit = if span > 1e-4 {
            [run[0] / span, run[1] / span]
        } else {
            [1.0, 0.0]
        };
        let off = [p[0] - from[0], p[1] - from[1]];
        let along = off[0] * unit[0] + off[1] * unit[1];
        let side = [off[0] - unit[0] * along, off[1] - unit[1] * along];
        let k = std::f64::consts::SQRT_2 / width;
        (-2.0 * (side[0] * side[0] + side[1] * side[1]) / (width * width)).exp()
            * 0.5
            * (erf(k * along) - erf(k * (along - span)))
    }

    /// A submarine running submerged lays no churned water in the field (her trail and waves take none either); at the
    /// surface she lays all of it, and awash part of it.
    #[test]
    fn a_submerged_stern_lays_nothing() {
        let hull = |y: f64| crate::ship_wake::WakeShip {
            id: "u-boat",
            position: [40.0, y, -25.0],
            heading: 0.4,
            pitch: 0.0,
            roll: 0.0,
            speed: 7.0,
            length: 67.0,
            beam: 6.2,
            draft: 4.7,
            center_x: 0.0,
            center_z: 0.0,
            forward_speed: 9.0,
            reverse_speed: 4.0,
            profile: None,
            oil_leak_m3s: 0.0,
        };
        assert_eq!(stern_source(&hull(-5.0)), None);
        assert_eq!(stern_source(&hull(-3.0)), None);
        let up = stern_source(&hull(0.0)).expect("under way at the surface");
        assert_eq!(up.churn, churn(7.0, 9.0));
        assert_eq!(up.speed, 7.0);
        assert_eq!(
            up.at,
            stern_of([40.0, 0.0, -25.0], 0.4, 7.0, 67.0, [0.0, 0.0])
        );
        let awash = stern_source(&hull(-1.5)).expect("half her wake reaches the surface");
        assert_eq!(awash.churn, churn(3.5, 9.0));
        assert!(awash.churn > 0.0 && awash.churn < up.churn);
        // Her speed still carries her stern between frames, however deep.
        assert_eq!(awash.speed, 7.0);
    }

    #[test]
    fn each_tier_sizes_its_field() {
        assert_eq!(FoamGrid::for_tier(QualityTier::Low), None);
        let medium = FoamGrid::for_tier(QualityTier::Medium).unwrap();
        let high = FoamGrid::for_tier(QualityTier::High).unwrap();
        assert_eq!(FoamGrid::for_tier(QualityTier::Ultra), Some(high));
        assert_eq!(high, HIGH);
        assert!((high.edge() - 768.0).abs() < 1e-9);
        // Medium: a quarter of the cells over the same water, as the wake field keeps its own.
        assert_eq!(medium.cells * 2, high.cells);
        assert!((medium.edge() - high.edge()).abs() < 1e-9);
    }

    #[test]
    fn the_window_lies_astern_of_a_hull_under_way() {
        // Heading 0 is north (−z): astern is +z.
        let still = window_centre([10.0, 0.0, 20.0], 0.0, 0.0, 15.0, HIGH.edge());
        assert_eq!(still, [10.0, 20.0]);
        let under_way = window_centre([10.0, 0.0, 20.0], 0.0, 15.0, 15.0, HIGH.edge());
        assert!((under_way[0] - 10.0).abs() < 1e-9);
        assert!((under_way[1] - (20.0 + ASTERN * 768.0)).abs() < 1e-9);
    }

    #[test]
    fn the_window_goes_where_the_view_looks_when_that_is_far_from_the_hull() {
        let hull = Some([0.0, 0.0]);
        let half = HIGH.half();
        // Looking down at her: about her.
        assert_eq!(
            place_window(hull, [0.0, 300.0, 0.0], [0.0, -1.0, 0.0], half),
            hull
        );
        // From 140 m up at water 2 km off: about that water.
        let down = 140.0 / 125.0f64.hypot(140.0);
        let along = 125.0 / 125.0f64.hypot(140.0);
        let far = place_window(hull, [2000.0, 140.0, 0.0], [along, -down, 0.0], half).unwrap();
        assert!((far[0] - 2125.0).abs() < 1e-6 && far[1].abs() < 1e-9);
        // Water 200 m off her stays about her (within 0.6 of the half-edge); on Medium's field too.
        let near = place_window(hull, [75.0, 140.0, 0.0], [along, -down, 0.0], half);
        assert_eq!(near, hull);
        assert_eq!(
            place_window(None, [5.0, 10.0, 7.0], [0.0, -1.0, 0.0], half),
            Some([5.0, 7.0])
        );
    }

    #[test]
    fn a_re_centre_carries_the_window_without_clearing_it() {
        let offset = [0.0; 2];
        let centre = [1234.4, -567.8];
        let low = first_cell(centre, offset, HIGH);
        let [sim, _] = window_rows(low, offset, HIGH);
        // The scene moves 3,000.3 m (not a whole number of cells): the window's cell in the field's frame is the same,
        // so no cell is taken in, and its corner moves with the scene.
        let shift = [-3000.3, 1999.9];
        let moved = [offset[0] + shift[0], offset[1] + shift[1]];
        let after = first_cell([centre[0] + shift[0], centre[1] + shift[1]], moved, HIGH);
        assert_eq!(after, low);
        let [carried, _] = window_rows(after, moved, HIGH);
        assert!((f64::from(carried[0]) - (f64::from(sim[0]) + shift[0])).abs() < 1e-2);
        assert!((f64::from(carried[1]) - (f64::from(sim[1]) + shift[1])).abs() < 1e-2);
        assert_eq!(carried[2..], sim[2..]);
        // A point on the sea reads the same texel before and after.
        let texel = |p: [f64; 2], row: [f32; 4]| {
            [0, 1].map(|i| {
                ((p[i] - f64::from(row[i])) / HIGH.cell + f64::from(row[2 + i])).floor() as i64
                    & i64::from(HIGH.cells - 1)
            })
        };
        let p = [1100.0, -400.0];
        assert_eq!(
            texel(p, sim),
            texel([p[0] + shift[0], p[1] + shift[1]], carried)
        );
        // A window that moves on by two cells takes in two rows of cells, no more.
        let on = first_cell([centre[0] + 1.5, centre[1]], offset, HIGH);
        assert_eq!([on[0] - low[0], on[1] - low[1]], [2, 0]);
    }

    /// The schedule before time scales took longer steps: whole fixed steps, at most three a frame, the rest of a longer
    /// frame dropped from the field's clock.
    fn old_schedule(accumulator: &mut f64, dt: f64) -> (usize, f64) {
        *accumulator += dt;
        let steps = ((*accumulator / STEP).floor() as usize).min(MAX_STEPS);
        let span = if *accumulator > (MAX_STEPS + 1) as f64 * STEP {
            (dt / steps as f64).max(STEP)
        } else {
            STEP
        };
        if steps == MAX_STEPS {
            *accumulator = accumulator.min(MAX_STEPS as f64 * STEP);
        }
        *accumulator -= steps as f64 * STEP;
        (steps, span)
    }

    #[test]
    fn a_time_scale_takes_longer_steps_rather_than_dropping_time() {
        // At game speed, at 30 to 144 fps with a jittering frame, the steps are the old ones, bit for bit.
        for fps in [30.0, 60.0, 144.0] {
            let (mut old, mut owed) = (0.0, 0.0);
            for i in 0..2000 {
                let dt = (1.0 + 0.2 * (f64::from(i) * 0.7).sin()) / fps;
                let (count, span) = old_schedule(&mut old, dt);
                let steps = schedule(owed + dt);
                owed = steps.owed;
                assert_eq!(
                    (steps.count, steps.flow, steps.span, owed),
                    (count, STEP, span, old),
                    "{fps} fps, frame {i}"
                );
            }
        }
        // Under each speed the theatre runs (to 256×), at 60 and 30 fps and on the slowest frame the client draws
        // (0.1 s), the steps stand for all of the game's time, and their flow for all of it until three steps of
        // LONGEST no longer cover a frame.
        for fps in [60.0, 30.0, 10.0] {
            for scale in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 100.0, 128.0, 256.0] {
                let dt = scale / fps;
                let (mut owed, mut stood, mut flowed) = (0.0, 0.0, 0.0);
                let frames = 300;
                for _ in 0..frames {
                    let steps = schedule(owed + dt);
                    assert!(
                        steps.count <= MAX_STEPS && steps.flow <= steps.span.min(LONGEST) + 1e-12
                    );
                    assert!(steps.owed > -1e-9 && steps.owed < STEP + 1e-9);
                    owed = steps.owed;
                    stood += steps.count as f64 * steps.span;
                    flowed += steps.count as f64 * steps.flow;
                }
                let elapsed = dt * f64::from(frames);
                assert!(
                    (stood + owed - elapsed).abs() < 1e-9 * elapsed,
                    "{scale}x at {fps} fps: {stood} s of {elapsed}"
                );
                if dt <= MAX_STEPS as f64 * LONGEST {
                    assert!((flowed - stood).abs() < 1e-9 * elapsed);
                } else {
                    assert!((flowed - f64::from(frames) * MAX_STEPS as f64 * LONGEST).abs() < 1e-6);
                }
            }
        }
        // A hitch at game speed (0.4 s): three steps share it; none of it is dropped.
        let hitch = schedule(0.4);
        assert_eq!(hitch.count, MAX_STEPS);
        assert!((3.0 * hitch.span + hitch.owed - 0.4).abs() < 1e-12 && hitch.flow == hitch.span);
        // A frame of hours stands for at most LONGEST_SPAN a step.
        assert_eq!(schedule(1.0e4).span, LONGEST_SPAN);
        assert_eq!(schedule(0.0).count, 0);
    }

    #[test]
    fn a_stern_lies_aft_of_her_middle_and_churns_with_her_speed() {
        let stern = stern_of([0.0, 0.0, 0.0], 0.0, 10.0, 250.0, [0.0, 0.0]);
        assert!(stern[0].abs() < 1e-9 && (stern[1] - 117.0).abs() < 1e-9);
        assert_eq!(churn(0.0, 15.0), 0.0);
        assert!((churn(15.0, 15.0) - 1.0).abs() < 1e-12);
        assert!(churn(5.0, 15.0) < churn(10.0, 15.0));
    }

    #[test]
    fn a_stern_lays_one_unbroken_band_however_long_her_runs() {
        // Her runs join: each step's starts where the last ended, and a frame's last ends where she is.
        let (a, b) = stern_run(Some([0.0, 0.0]), [0.0, -30.0], 0.5, 0, 3);
        let (c, d) = stern_run(Some([0.0, 0.0]), [0.0, -30.0], 0.5, 1, 3);
        let (_, f) = stern_run(Some([0.0, 0.0]), [0.0, -30.0], 0.5, 2, 3);
        assert_eq!(
            (a, b, c, d, f),
            (
                [0.0, 0.0],
                [0.0, -10.0],
                [0.0, -10.0],
                [0.0, -20.0],
                [0.0, -30.0]
            )
        );
        // A hull put somewhere else, or one new to the field, lays nothing between.
        assert_eq!(
            stern_run(Some([0.0, 0.0]), [0.0, -500.0], 0.5, 0, 3),
            ([0.0, -500.0], [0.0, -500.0])
        );
        assert_eq!(
            stern_run(None, [4.0, 5.0], 0.5, 1, 2),
            ([4.0, 5.0], [4.0, 5.0])
        );
        // At 256× and 30 fps a hull at 30 kn runs 131 m a frame, which her speed accounts for: she lays it.
        let frame = 256.0 / 30.0;
        let run = 15.4 * frame;
        assert_eq!(
            stern_run(Some([0.0, 0.0]), [0.0, -run], 15.4 * frame, 2, 3).1,
            [0.0, -run]
        );
        assert_eq!(
            stern_run(Some([0.0, 0.0]), [0.0, -run], 15.4 * frame, 0, 3).0,
            [0.0, 0.0]
        );
        // Down her track's middle the runs lay all of her churned water once, whether a run is 0.5 m (at 1×) or 40 m
        // (three capped steps of a long frame): no dashes, no thinning.
        let width = 12.0;
        for run in [0.5, 4.0, 40.0] {
            for i in 0..40 {
                let p = [0.0, -100.0 - f64::from(i) * 1.37];
                let mut total = 0.0;
                let mut at = 0.0;
                while at > -400.0 {
                    total += swept(p, [0.0, at], [0.0, at - run], width);
                    at -= run;
                }
                assert!((total - 1.0).abs() < 1e-5, "run {run} at {p:?}: {total}");
            }
        }
        // Across it, the Gaussian.
        let side = swept([6.0, -200.0], [0.0, 0.0], [0.0, -400.0], width);
        assert!((side - (-0.5f64).exp()).abs() < 1e-5);
    }

    #[test]
    fn the_youngest_splashes_take_the_slots() {
        let mut splashes = SplashDeposits::default();
        let deposit = |x: f64, born: f64| SplashDeposit {
            x,
            z: 0.0,
            born,
            scale: 1.0,
            seed: 0.5,
            skew: [0.0; 2],
            leaving: None,
        };
        // 40 falling within the window over the last 8 s, 3 outside it, and 5 older than their landing.
        for i in 0..5 {
            splashes
                .list
                .push_back(deposit(f64::from(i) * 10.0, 20.0 + f64::from(i)));
        }
        for i in 0..43 {
            let x = if i % 15 == 7 {
                5000.0
            } else {
                f64::from(i) * 5.0 - 100.0
            };
            splashes
                .list
                .push_back(deposit(x, 32.0 + f64::from(i) * 0.2));
        }
        let mut rows = [[0.0f32; 4]; 2 * SPLASH_SLOTS];
        let now = 41.0;
        let count = splash_rows(&splashes, now, [0.0; 2], HIGH.half(), false, &mut rows);
        assert_eq!(count, SPLASH_SLOTS);
        // Youngest first, none outside the window, none past landing.
        let ages: Vec<f32> = rows.chunks(2).take(count).map(|r| r[0][2]).collect();
        assert!(ages.windows(2).all(|w| w[0] <= w[1]));
        assert!((f64::from(ages[0]) - (now - (32.0 + 42.0 * 0.2))).abs() < 1e-4);
        assert!(
            rows.chunks(2)
                .take(count)
                .all(|r| r[0][0].abs() < 1000.0 && f64::from(r[0][2]) <= LANDING)
        );
        // With few falling, a step that takes in cells takes the older ones too (what those cells start from).
        let mut few = SplashDeposits::default();
        few.list.extend(splashes.list.iter().take(8).copied());
        let landing = splash_rows(&few, 36.0, [0.0; 2], HIGH.half(), false, &mut rows);
        let seeding = splash_rows(&few, 36.0, [0.0; 2], HIGH.half(), true, &mut rows);
        assert_eq!((landing, seeding), (3, 8));
    }
}
