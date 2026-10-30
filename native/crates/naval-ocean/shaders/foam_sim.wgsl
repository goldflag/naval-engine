// The foam ship wakes and shell splashes leave on the sea (naval_ocean::foam_sim, src/foam_sim.rs): a layer the water
// stirs, after Crest 4's foam simulation (wave-harmonic/crest, MIT: a persistent foam buffer stepped at a fixed rate,
// fed by its inputs, decayed and carried by a flow) and the Atlas talk at GDC 2019 (foam injected and then evolved with
// the water's own motion). Written here from their descriptions; no code copied.
//
// A field of n² cells `params.grid.x` metres across about the focus hull, addressed toroidally: the field's cell k lies
// in texel k mod n, so a window that moves clears only the cells it takes in. Texels: (thickness, turbulence, residue,
// validity). Thickness is the foam (about 1 dense white water), turbulence the churn that stirs it (1 behind a stern or
// under a falling column), residue the scum and bubbles foam leaves as it dies, validity how long the cell has been in
// the window (0 as it is taken in, 1 after VALID_RISE seconds): the surface draws the foam itself where it is low, so a
// cleared cell never shows. A cell taken in starts from what the surface draws there (`seeded`: the trails' churned
// water as the field's own foam of its age, the splashes' deposits), so the foam carries on into the field. Each 30 Hz step `advect_pass`
// carries the field back along the flow (semi-Lagrangian); `settle_pass` corrects that (MacCormack, clamped to the
// cells it was read between, so the foam stays sharp), spreads it where the water is churned (eddy diffusion), decays
// it (thin foam in seconds, thick in tens of seconds, the residue over minutes) and adds the step's sources: each
// stern's churn along the run her stern made, laid on the boils it raises, and each splash's water as the column falls
// back onto the sea.
//
// The flow: the surface's drift; eddies 6 and 18 m across that the turbulence drives (their stream function times the
// turbulence is differentiated for the velocity, so the stirring never gathers or thins the foam, and runs along the
// churned water's edges, drawing it out into streaks); each splash's base surge, pushing its water out with its
// deposit; and the wind's Langmuir cells, rolls along the wind whose surface water meets in lines LANGMUIR_SPACING
// apart, gathering the thin foam left on the sea into windrows (the one part of the flow that gathers foam: it
// thickens where the water meets, while it is thin, and thins where it parts).

#import naval_ocean::mx_noise::mx_noise_float
#import naval_ocean::noise::{gradient_noise, noise_seed}
#import naval_ocean::pcg::hash3
#import naval_ocean::wake::{splash_deposit, splash_radius, splash_laid, splash_white, splash_cover, splash_still, track_coordinate, inside_square, square_edge, WAKE_EXTENT, TILES, TRACK_RANGE, YOUNG_START, YOUNG_FULL, SIM_THIN, SIM_FULL, SIM_YOUNG, SIM_STILL}

struct Params {
    // xy: the window's first cell (in the field's frame), zw: the previous step's (cells new to the window are taken in).
    window: vec4<i32>,
    // x: sterns, y: splashes this step, z: 1 when every cell is cleared (a new field, or one forgotten), w: the trails'
    // tiles a cell taken in starts from (none when no cell is taken in).
    counts: vec4<i32>,
    // x: the step (s), y: the field's clock (s), zw: where the eddies' frame lies (m from the window's centre: they ride
    // the drift).
    clock: vec4<f32>,
    // xy: the surface's drift (m/s).
    drift: vec4<f32>,
    // x: a cell's edge (m), y: the cells along an edge, z: the game seconds the step stands for (its flow's, `clock.x`,
    // or more under a time scale its steps' flow cannot keep up with): the foam's lives and the splashes' landing.
    grid: vec4<f32>,
    // The surface's knobs for the trails' foam (`Wake.shading`): its start, full and cover, and the slick's full.
    seed: vec4<f32>,
    // Per stern, two rows: the run her stern made this step (m from the window's centre, from xy to zw), then her
    // churned water's width (m, the Gaussian's), the thickness laid down its middle once she has passed, its turbulence
    // and her boils' size (m).
    sterns: array<vec4<f32>, 16>,
    // Per splash, `Wake.splashes`' two rows (src/splash_foam.rs), its place m from the window's centre and its age at
    // this step: those still landing and, when cells are taken in, older ones those cells start from.
    splashes: array<vec4<f32>, 64>,
    // Per trail tile, `Wake.tiles`' first two rows, their places m from the window's centre: its square's centre and
    // atlas tile, its slick square's centre and her beam.
    tiles: array<vec4<f32>, 128>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var carried: texture_2d<f32>;
@group(0) @binding(2) var written: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var field_sampler: sampler;
// The trails' atlas (src/stamps.rs), which a cell taken in starts from.
@group(0) @binding(5) var trails: texture_2d<f32>;

// The eddies: sizes (m), shares of the stirring and turnover times (s), and the speed at full turbulence (m/s).
const EDDY_SIZES: vec2<f32> = vec2<f32>(6.0, 18.0);
const EDDY_SHARES: vec2<f32> = vec2<f32>(0.55, 0.45);
const EDDY_TURNOVER: vec2<f32> = vec2<f32>(8.0, 22.0);
const EDDY_SPEED: f32 = 2.0;
// The Langmuir cells: their lines' mean spacing (m), the speed (m/s) at which the surface water meets in them, and how
// far the lines wander (m, over LANGMUIR_WANDER_LENGTH along the wind and LANGMUIR_WANDER_WIDTH across it, and again at
// 2.3 times those with 1.5 times the wander), so they bend and lie unevenly apart. The water meets in stretches about
// LANGMUIR_BREAKS m long and lies slack between, so a line breaks off and starts again. Where it meets it sinks, and
// takes down the foam that would gather past WINDROW (thickness): a windrow holds thin, old foam, never a white bar.
const LANGMUIR_SPACING: f32 = 17.0;
const LANGMUIR_SPEED: f32 = 0.04;
const LANGMUIR_WANDER: f32 = 9.0;
const LANGMUIR_WANDER_LENGTH: f32 = 70.0;
const LANGMUIR_WANDER_WIDTH: f32 = 40.0;
const LANGMUIR_BREAKS: f32 = 45.0;
const WINDROW: vec2<f32> = vec2<f32>(0.2, 0.6);
// A base surge's speed (m/s) per metre of its radius's growth rate: 1, the water moving with the deposit's edge.
const SURGE: f32 = 1.0;
// Scales on what the sterns lay (their thickness and width), for tuning without a build.
const STERN_GAIN: f32 = 1.0;
const STERN_SPREAD: f32 = 1.3;
// Diffusion (m²/s): a little everywhere, more where the water is churned (its unresolved eddies).
const DIFFUSION: f32 = 0.01;
const EDDY_DIFFUSION: f32 = 0.6;
// Thickness's life (s): thin foam's and thick's, and the thicknesses between which it goes from one to the other; the
// patches (PATCH_SIZE m) where it dies sooner or later (PATCHINESS), and a floor (per s) so the last of it goes.
const THIN_LIFE: f32 = 3.5;
const THICK_LIFE: f32 = 34.0;
const THIN: f32 = 0.06;
const THICK: f32 = 0.9;
const PATCH_SIZE: f32 = 8.0;
const PATCHINESS: f32 = 0.7;
const FADE: f32 = 0.004;
// Turbulence's life (s); the share of the foam that dies left as residue, and the residue's life (s).
const TURBULENCE_LIFE: f32 = 5.0;
const RESIDUE_SHARE: f32 = 0.35;
const RESIDUE_LIFE: f32 = 75.0;
const MOST: f32 = 4.0;
// A splash's water: thickness per second (at the middle of its deposit) as the column falls back, and its lumps.
const SPLASH_RATE: f32 = 0.25;
const SPLASH_LUMPS: f32 = 0.6;
const SPLASH_FRAY: f32 = 0.3;
// A stern's churned water comes up in boils: how much the thickness she lays varies with them.
const STERN_LUMPS: f32 = 0.7;
// A splash's column has fallen back by this age (s); older splashes only seed the cells taken in.
const LANDING: f32 = 10.5;
// A cell's validity rises from 0 as it is taken in to 1 over this (s), while the surface fades from its own foam to
// the field's: sooner in some patches VALID_PATCH m across, later in others (VALID_SPREAD: the rise's shortest and
// longest as shares of VALID_RISE), so the field takes over a patch at a time, never everywhere at once.
const VALID_RISE: f32 = 3.0;
const VALID_PATCH: f32 = 24.0;
const VALID_SPREAD: vec2<f32> = vec2<f32>(0.4, 2.2);
// A cell taken in on a trail starts as the field's own foam of the trail's age there: the cover the surface draws, in
// the lumps a stern lays (SEED_LUMPS of her beam across, as STERN_BOILS lays them in src/foam_sim.rs) and thinned
// where the field's foam dies sooner (the decay's patches), the more the older the trail: its age read off its churn,
// which fades over SEED_TRAIL_LIFE s (src/trail.rs, `WakeTuning::churn_life`).
const SEED_LUMPS: f32 = 0.33;
const SEED_TRAIL_LIFE: f32 = 30.0;
// The residue it starts with: a little where the churned water lay (none under the slick beyond it, which the trail
// draws itself), and under a splash's scar this share of the scar's.
const SEED_RESIDUE: f32 = 0.15;

// The field is kept at half precision, whose step is about 0.001 at a thickness of 1 to 2: a 30 Hz step's decay of
// thick foam is about one such step, so rounding it to the nearest would drain the foam (or hold it) by that share every
// step. Each stored value is dithered by up to half a step either way, from a hash fresh each texel and step, so its
// rounding is unbiased and the decay keeps its rate.
fn dithered(value: vec4<f32>, cell: vec2<i32>) -> vec4<f32> {
    let index = u32(cell.x) * 1973u + u32(cell.y) * 9277u + bitcast<u32>(params.clock.y) * 26699u;
    let noise = vec4<f32>(hash3(index) - 0.5, 0.0);
    // Half precision's step at the value (its subnormals' below 2^-14).
    let step = exp2(floor(log2(max(abs(value), vec4<f32>(6.1035156e-5)))) - 10.0);
    return select(value, value + noise * step, value > vec4<f32>(0.0));
}

fn wrap(cell: vec2<i32>, n: i32) -> vec2<i32> {
    return ((cell % n) + n) % n;
}

fn texel_at(texture: texture_2d<f32>, cell: vec2<i32>, n: i32) -> vec4<f32> {
    return textureLoad(texture, wrap(cell, n), 0);
}

// Where texel `cell` lies, metres from the window's centre: its world cell is the one in the window it stands for.
fn place(cell: vec2<i32>, n: i32) -> vec2<f32> {
    let m = wrap(cell - params.window.xy, n);
    return (vec2<f32>(m - n / 2) + 0.5) * params.grid.x;
}

// Whether texel `cell` stood for another world cell before this step's window (it then holds nothing of this one's).
fn taken_in(cell: vec2<i32>, n: i32) -> bool {
    let now = params.window.xy + wrap(cell - params.window.xy, n);
    let before = params.window.zw + wrap(cell - params.window.zw, n);
    return any(now != before);
}

// The splashes whose reach (`params.splashes`' second row's w, more than the 1.6 radii a surge reaches) overlaps this
// workgroup's 8 × 8 cells, as bits (bit i: splash i; SPLASH_SLOTS in src/foam_sim.rs is 32): each cell loops over these
// alone, in the rows' order, so it adds up the same as over all of them.
var<workgroup> near_splashes: atomic<u32>;

// Fills `near_splashes` for the workgroup whose first cell is `first`: each of its invocations tests one splash against
// the box of its cells' places (a cell's edge to spare; the whole window across where its cells straddle the window's
// seam). Call it in uniform control flow, before any invocation returns. With no splashes it lists none and waits for
// no other invocation.
fn splashes_near(first: vec2<i32>, local: u32, n: i32) -> u32 {
    if params.counts.y == 0 {
        return 0u;
    }
    if local == 0u {
        atomicStore(&near_splashes, 0u);
    }
    workgroupBarrier();
    let i = i32(local);
    if i < params.counts.y {
        let start = wrap(first - params.window.xy, n);
        let whole = start + 7 >= vec2<i32>(n);
        let low = select(start, vec2<i32>(0), whole);
        let high = select(start + 7, vec2<i32>(n - 1), whole);
        let h = params.grid.x;
        let lo = (vec2<f32>(low - n / 2) - 0.5) * h;
        let hi = (vec2<f32>(high - n / 2) + 1.5) * h;
        let centre = params.splashes[2 * i].xy;
        let off = centre - clamp(centre, lo, hi);
        let reach = params.splashes[2 * i + 1].w;
        if dot(off, off) <= reach * reach {
            atomicOr(&near_splashes, 1u << u32(i));
        }
    }
    workgroupBarrier();
    return atomicLoad(&near_splashes);
}

// The eddies' stream function at `p` (m from the window's centre), in the frame the drift carries.
fn stream(p: vec2<f32>) -> f32 {
    let q = p + params.clock.zw;
    let t = params.clock.y;
    return mx_noise_float(vec3<f32>(q / EDDY_SIZES.x, t / EDDY_TURNOVER.x)) * (EDDY_SIZES.x * EDDY_SHARES.x)
        + mx_noise_float(vec3<f32>(q / EDDY_SIZES.y + 31.7, t / EDDY_TURNOVER.y)) * (EDDY_SIZES.y * EDDY_SHARES.y);
}

// The Langmuir cells at `p`: the across-wind velocity (m/s) and its divergence (1/s, below 0 where the water meets).
fn langmuir(p: vec2<f32>) -> vec3<f32> {
    let drift = params.drift.xy;
    if dot(drift, drift) < 1e-8 {
        return vec3<f32>(0.0);
    }
    let along = normalize(drift);
    let across = vec2<f32>(-along.y, along.x);
    let q = p + params.clock.zw;
    let lines = vec2<f32>(dot(q, along) / LANGMUIR_WANDER_LENGTH, dot(q, across) / LANGMUIR_WANDER_WIDTH);
    let k = 6.2831853 / LANGMUIR_SPACING;
    let wander = gradient_noise(lines + vec2<f32>(0.0, 3.7), 891u) + gradient_noise(lines / 2.3 + 8.1, 894u) * 1.5;
    let y = dot(q, across) + wander * LANGMUIR_WANDER;
    let breaks = vec2<f32>(lines.x * (LANGMUIR_WANDER_LENGTH / LANGMUIR_BREAKS), lines.y + 7.9);
    let speed = LANGMUIR_SPEED * smoothstep(-0.2, 0.2, gradient_noise(breaks, 892u));
    return vec3<f32>(across * (speed * sin(k * y)), speed * k * cos(k * y));
}

// Each landing splash's base surge at `p` (m/s): its water carried out with its deposit, at the speed its radius grows
// times how far out it lies (so the deposit spreads as a whole, its lobes kept), dying away past its edge.
fn surge(p: vec2<f32>, near: u32) -> vec2<f32> {
    var u = vec2<f32>(0.0);
    var left = near;
    while left != 0u {
        let i = i32(countTrailingZeros(left));
        left &= left - 1u;
        let row = params.splashes[2 * i];
        if row.z > LANDING {
            continue;
        }
        let q = p - row.xy;
        let radius = splash_radius(row.z, row.w);
        let r = length(q) / radius;
        if r < 1.6 {
            let growth = row.w * (25.0 / 2.8 * exp(-row.z / 2.8) + 0.22);
            u += q * (SURGE * growth / radius) * (1.0 - smoothstep(1.0, 1.6, r));
        }
    }
    return u;
}

// The water's velocity at texel `cell` (at `p`), m/s: the drift, the Langmuir cells, the splashes' surges and the curl
// of the eddies' stream function times the turbulence there (central differences over a cell).
fn velocity(cell: vec2<i32>, p: vec2<f32>, n: i32, near: u32) -> vec2<f32> {
    let h = params.grid.x;
    let carried = params.drift.xy + langmuir(p).xy + surge(p, near);
    let west = texel_at(source, cell - vec2<i32>(1, 0), n).g;
    let east = texel_at(source, cell + vec2<i32>(1, 0), n).g;
    let north = texel_at(source, cell - vec2<i32>(0, 1), n).g;
    let south = texel_at(source, cell + vec2<i32>(0, 1), n).g;
    if max(max(west, east), max(north, south)) < 1e-3 {
        return carried;
    }
    let dx = (stream(p + vec2<f32>(h, 0.0)) * east - stream(p - vec2<f32>(h, 0.0)) * west) / (2.0 * h);
    let dz = (stream(p + vec2<f32>(0.0, h)) * south - stream(p - vec2<f32>(0.0, h)) * north) / (2.0 * h);
    return carried + vec2<f32>(dz, -dx) * EDDY_SPEED;
}

// The field read at texel coordinates `at` (texel centres at whole numbers + ½), wrapped.
fn read(texture: texture_2d<f32>, at: vec2<f32>, n: i32) -> vec4<f32> {
    return textureSampleLevel(texture, field_sampler, at / f32(n), 0.0);
}

// Semi-Lagrangian: each cell takes what lay where its water was a step ago.
@compute @workgroup_size(8, 8)
fn advect_pass(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let n = i32(textureDimensions(source).x);
    let near = splashes_near(vec2<i32>(group.xy) * 8, local, n);
    let cell = vec2<i32>(id.xy);
    if any(cell >= vec2<i32>(n)) {
        return;
    }
    let shift = velocity(cell, place(cell, n), n, near) * (params.clock.x / params.grid.x);
    textureStore(written, cell, read(source, vec2<f32>(cell) + 0.5 - shift, n));
}

// Lumps about 1 (0 to 2) of `size` metres at `q`, `amount` of them.
fn lumps(q: vec2<f32>, size: f32, amount: f32, seed: u32) -> f32 {
    let n = gradient_noise(q / size, seed) + gradient_noise(q / (size * 0.43) + 17.3, seed + 1u) * 0.5;
    return clamp(1.0 + n * (amount * 1.6), 0.0, 2.0);
}

// erf (Abramowitz and Stegun 7.1.26, |error| < 1.5e-7).
fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.3275911 * abs(x));
    let y = 1.0 - ((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t
        * exp(-x * x);
    return select(-y, y, x >= 0.0);
}

// A stern's churned water laid along the run `start`–`end` her stern made this step, at `p`: a Gaussian `width` across
// (exp(−2 d²/w²)) swept along the run, the share of it this run lays (summed over her runs, 1 down the middle of her
// track once she has passed, however long each run: no dashes), and how near the run `p` lies (1 on it).
fn swept(p: vec2<f32>, start: vec2<f32>, end: vec2<f32>, width: f32) -> vec2<f32> {
    let run = end - start;
    let span = length(run);
    let unit = select(vec2<f32>(1.0, 0.0), run / max(span, 1e-6), span > 1e-4);
    let off = p - start;
    let along = dot(off, unit);
    let nearest = off - unit * clamp(along, 0.0, span);
    let near = dot(nearest, nearest) / (width * width);
    if near > 6.0 {
        return vec2<f32>(0.0);
    }
    let side = off - unit * along;
    let k = 1.41421356 / width;
    let laid = exp(-2.0 * dot(side, side) / (width * width)) * 0.5 * (erf(k * along) - erf(k * (along - span)));
    return vec2<f32>(max(laid, 0.0), exp(-2.0 * near));
}

// What the surface draws at `p` (m from the window's centre), as the field's (thickness, turbulence, residue): the
// trails' churned water (the atlas as `trail_read` reads it, without its warp and lace) as the field's own foam of its
// age (SEED_LUMPS: in lumps, thinned in patches the more the older it is), and the splashes' deposits
// (`splash_foam`'s cover). Thickness is taken so the field draws about that cover there (`sim_read`'s
// SIM_THIN–SIM_FULL), turbulence so it is as young, and little residue (SEED_RESIDUE). A cell taken in starts from it.
fn seeded(p: vec2<f32>, near: u32) -> vec3<f32> {
    var turbulence = 0.0;
    var across = TRACK_RANGE;
    var beam = 10.0;
    for (var i = 0; i < params.counts.w; i++) {
        let bounds = params.tiles[2 * i];
        let uv = (p - bounds.xy) / WAKE_EXTENT + 0.5;
        if inside_square(uv) {
            let tile = textureSampleLevel(trails, field_sampler, (uv + bounds.zw) / TILES, 0.0);
            let value = tile.r * square_edge(uv);
            if value > turbulence {
                turbulence = value;
                let band = track_coordinate(tile.b);
                across = select(TRACK_RANGE, band, band >= 0.0);
                beam = max(params.tiles[2 * i + 1].z, 1.0);
            }
        }
    }
    let knobs = params.seed;
    var young = smoothstep(YOUNG_START, YOUNG_FULL, turbulence);
    // Old water's foam lies in streaks: about half the young water's cover.
    let banded = smoothstep(knobs.x, knobs.y, turbulence) * knobs.z * mix(0.5, 1.0, young) * (1.0 - smoothstep(0.8, 1.1, across));
    var thickness = drawn_as(banded);
    if thickness > 0.0 {
        let age = -SEED_TRAIL_LIFE * log(clamp(turbulence, 0.02, 1.0));
        let drifting = p + params.clock.zw;
        let patches = max(1.0 + gradient_noise(drifting / PATCH_SIZE + 5.1, 861u) * PATCHINESS, 0.3);
        let dying = clamp(exp(-age / THICK_LIFE * (1.0 / patches - 1.0)), 0.0, 2.0);
        thickness *= lumps(drifting, beam * SEED_LUMPS, STERN_LUMPS, 871u) * dying;
    }
    var residue = smoothstep(0.02, 0.4, turbulence) * (1.0 - young) * SEED_RESIDUE;
    // The splashes' deposits, where one draws more than the trail's foam.
    var splash = 0.0;
    var left = near;
    while left != 0u {
        let i = i32(countTrailingZeros(left));
        left &= left - 1u;
        let row = params.splashes[2 * i];
        let more = params.splashes[2 * i + 1];
        let q = p - row.xy;
        if dot(q, q) > more.w * more.w {
            continue;
        }
        let age = row.z;
        let deposit = splash_laid(q, age, row.w, noise_seed(u32(more.x * 16777215.0)), more.yz);
        let drawn = splash_cover(deposit, age) * mix(1.0, 0.4, smoothstep(12.0, 50.0, age));
        if drawn > max(banded, splash) {
            splash = drawn;
            young = smoothstep(0.1, 0.7, splash_white(age));
        }
        residue = max(residue, splash_still(deposit, age) / SIM_STILL * SEED_RESIDUE);
    }
    thickness = max(thickness, drawn_as(splash));
    let churned = select(0.0, mix(SIM_YOUNG.x, SIM_YOUNG.y, young), young > 1e-3);
    return vec3<f32>(thickness, churned, residue);
}

// The thickness the field draws as `cover` (`sim_read`'s SIM_THIN–SIM_FULL): smoothstep's inverse.
fn drawn_as(cover: f32) -> f32 {
    let c = clamp(cover, 0.0, 1.0);
    return select(0.0, SIM_THIN + (SIM_FULL - SIM_THIN) * (0.5 - sin(asin(1.0 - 2.0 * c) / 3.0)), c > 1e-3);
}

// MacCormack's correction, diffusion, decay and the step's sources.
@compute @workgroup_size(8, 8)
fn settle_pass(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let n = i32(textureDimensions(source).x);
    let near = splashes_near(vec2<i32>(group.xy) * 8, local, n);
    let cell = vec2<i32>(id.xy);
    if any(cell >= vec2<i32>(n)) {
        return;
    }
    let dt = params.clock.x;
    let span = params.grid.z;
    let h = params.grid.x;
    let p = place(cell, n);
    let shift = velocity(cell, p, n, near) * (dt / h);
    let here = textureLoad(carried, cell, 0);
    let now = textureLoad(source, cell, 0);
    // Carried forward again, the carried field should give back what is here now: half the difference is the
    // semi-Lagrangian step's own error, put back.
    let again = read(carried, vec2<f32>(cell) + 0.5 + shift, n);
    var next = here + (now - again) * 0.5;
    // Never past what the four cells it was read between hold (Selle et al. 2008).
    let back = vec2<f32>(cell) - shift;
    let low = vec2<i32>(floor(back));
    let a = texel_at(source, low, n);
    let b = texel_at(source, low + vec2<i32>(1, 0), n);
    let c = texel_at(source, low + vec2<i32>(0, 1), n);
    let d = texel_at(source, low + vec2<i32>(1, 1), n);
    next = clamp(next, min(min(a, b), min(c, d)), max(max(a, b), max(c, d)));
    // Spread where the water is churned.
    let around = texel_at(carried, cell + vec2<i32>(1, 0), n) + texel_at(carried, cell - vec2<i32>(1, 0), n)
        + texel_at(carried, cell + vec2<i32>(0, 1), n) + texel_at(carried, cell - vec2<i32>(0, 1), n);
    let spread = min((DIFFUSION + EDDY_DIFFUSION * now.g) * dt / (h * h), 0.2);
    next += (around - here * 4.0) * spread;
    // Decay over the game seconds the step stands for: thin foam dies fast, thick slowly, sooner in some patches than
    // others; what dies leaves a residue.
    let drifting = p + params.clock.zw;
    let patches = 1.0 + gradient_noise(drifting / PATCH_SIZE + 5.1, 861u) * PATCHINESS;
    let thickness = max(next.r, 0.0);
    let life = mix(THIN_LIFE, THICK_LIFE, smoothstep(THIN, THICK, thickness)) * max(patches, 0.3);
    let lost = min(thickness, thickness * (1.0 - exp(-span / life)) + FADE * span);
    // The Langmuir cells gather the foam and its residue where the water meets, the foam only while it is thin (WINDROW).
    let meets = clamp(1.0 - langmuir(p).z * dt, 0.9, 1.1);
    let gathered = select(meets, mix(meets, 1.0, smoothstep(WINDROW.x, WINDROW.y, thickness)), meets > 1.0);
    next.r = (thickness - lost) * gathered;
    next.g = max(next.g * exp(-span / TURBULENCE_LIFE) - FADE * span, 0.0);
    next.b = max(next.b * exp(-span / RESIDUE_LIFE) + lost * RESIDUE_SHARE - FADE * 0.25 * span, 0.0) * meets;
    // A cell taken in this step starts from what the surface draws there; one kept grows surer.
    if params.counts.z != 0 || taken_in(cell, n) {
        next = vec4<f32>(seeded(p, near), 0.0);
    } else {
        let rise = mix(VALID_SPREAD.x, VALID_SPREAD.y, smoothstep(-0.35, 0.35, gradient_noise(p / VALID_PATCH + 2.3, 857u)));
        next.a = min(now.a + dt / (VALID_RISE * rise), 1.0);
    }
    // Each stern lays her churned water along the run her stern made, on the boils her screws raise.
    for (var i = 0; i < params.counts.x; i++) {
        let run = params.sterns[2 * i];
        let shape = params.sterns[2 * i + 1];
        let laid = swept(p, run.xy, run.zw, shape.x * STERN_SPREAD);
        if laid.y > 0.0 {
            next.r += shape.y * STERN_GAIN * laid.x * lumps(drifting, shape.w, STERN_LUMPS, 871u);
            next.g = max(next.g, shape.z * laid.y);
        }
    }
    // Each splash's water as its column falls back onto the sea (from about 1 s, most of it 2.5–6.5 s, the last by
    // 10 s), in lumps, over its deposit as the base surge spreads it; laid over the game seconds the step stands for.
    var left = near;
    while left != 0u {
        let i = i32(countTrailingZeros(left));
        left &= left - 1u;
        let row = params.splashes[2 * i];
        let more = params.splashes[2 * i + 1];
        let q = p - row.xy;
        if dot(q, q) > more.w * more.w {
            continue;
        }
        let age = row.z;
        let landing = smoothstep(0.8, 2.5, age) * (1.0 - smoothstep(6.5, 10.0, age));
        if landing <= 0.0 {
            continue;
        }
        let seed = noise_seed(u32(more.x * 16777215.0));
        let radius = splash_radius(age, row.w);
        // Its outline frayed (SPLASH_FRAY of its radius), in its deposit's own frame so the surge keeps it.
        let unit = q / radius + vec2<f32>(more.x * 37.0, more.x * 91.0);
        let fray = vec2<f32>(gradient_noise(unit * 2.6, 885u), gradient_noise(unit * 2.6 + 9.7, 886u));
        let deposit = splash_deposit(q + fray * (radius * SPLASH_FRAY), radius, seed, more.yz, smoothstep(1.0, 6.0, age));
        // Its lumps spread with it (laid in its deposit's own frame, as the surge carries what lies there).
        let lumpy = lumps(q * (22.0 / radius) + vec2<f32>(more.x * 517.0, more.x * 211.0), 3.2, SPLASH_LUMPS, 881u);
        next.r += SPLASH_RATE * landing * deposit * lumpy * span;
        next.g = max(next.g, deposit * smoothstep(0.5, 2.0, age) * (1.0 - smoothstep(8.0, 13.0, age)));
    }
    next = clamp(dithered(next, cell), vec4<f32>(0.0), vec4<f32>(MOST, 1.0, MOST, 1.0));
    textureStore(written, cell, next);
}
