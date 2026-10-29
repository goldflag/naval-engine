// The trial look "volume" (`src/volume.rs`, `src/volume_pass.rs`): a burning ship's smoke and a magazine's
// detonation marched through grids of their particles instead of drawn as sprites. Not a port. The technique: smoke
// lit through a grid swept along the light (GPU Gems 3, chapter 30), the energy-conserving integration of Frostbite's
// volumetrics (Hillaire), the jittered steps of naval-clouds' march, and noise carried along a flow in two phases
// (flow maps).
//
// A body is a box over three grids. Two hold what the particles' soft balls leave in each voxel (the union of their
// shapes, their heat, albedo and age): a coarse one over the whole body for its large balls and a fine one about the
// fires for the small young ones. The light grid, on the coarse lattice, holds the smoke between each voxel and the
// sun and the sky above it, the balls' strength, and whether any ball is there. The march erodes the shape into
// billows with the fire shaders' tiling Worley volume (a column's billows carried up and downwind at the smoke's own
// speed, a detonation's swelling about its seat) and lights each step with the sun through the light grid and past
// the billows toward it, the sky, the flashes, and its own heat.
//
// Groups: 0 the camera, 1 the effects' lighting, 2 the body, 3 the scene reads (depth, occlusion map, sky, clouds).

#import naval::camera::{camera, world_to_clip, view_ray, ndc_from_pixel, view_depth}
#import naval::motion::motion_response
#import naval_effects::common::{sky_fill, lighting, emitted, emitted_at, effect_fog, smoke_visibility, sstep, scene_depth, nearness_of, distance_of, occlusion_fronts, occlusion_depths, occlusion_sampler}

// `BodyUniform` in volume_pass.rs.
struct VolumeBody {
    // xyz the coarse grid's minimum corner (world m), w its voxel's edge (m).
    origin: vec4<f32>,
    // xyz its voxels in use, w the kind: 0 a column, 1 a detonation.
    dims: vec4<f32>,
    // xyz the coarse grid's and the light grid's textures' size in voxels, w a detonation's size (m).
    grid: vec4<f32>,
    // The fine grid: its corner and voxel, its voxels in use (none without one), its texture's size.
    near_origin: vec4<f32>,
    near_dims: vec4<f32>,
    near_grid: vec4<f32>,
    // xyz where the noise is anchored (world m), w the body's clock (s).
    anchor: vec4<f32>,
    // x and z the wind the smoke leans with (m/s), y how far a column's broad billows have climbed (m).
    flow: vec4<f32>,
    // x 1 with the lens inside the box, y the short step (of a puff's size), z the long one (coarse voxels), w the
    // most steps.
    march: vec4<f32>,
}

@group(2) @binding(0) var<uniform> body: VolumeBody;
@group(2) @binding(1) var far_grid: texture_3d<f32>;
@group(2) @binding(2) var near_grid: texture_3d<f32>;
@group(2) @binding(3) var light_grid: texture_3d<f32>;
@group(2) @binding(4) var noise: texture_3d<f32>;
@group(2) @binding(5) var grid_sampler: sampler;
@group(2) @binding(6) var noise_sampler: sampler;

// What the grids' channels stand for at 1.0 (`volume.rs`).
const SHAPE_MAX: f32 = 2.0;
const HEAT_MAX: f32 = 2.5;
const AGE_MAX: f32 = 64.0;
const TAU_SCALE: f32 = 4.0;
// A column's billows: the period (m) of the small ones, carried in two phases of this many seconds, and of the broad
// ones its older, larger puffs break into.
const FINE_PERIOD: f32 = 22.0;
const BROAD_PERIOD: f32 = 64.0;
const PHASE_SECONDS: f32 = 3.0;
// The period (m) of a detonation's small billows once it has stopped swelling.
const BLAST_PERIOD: f32 = 44.0;
// Optical depth across a puff of a sprite's usual opacity, per radius.
const COLUMN_TAU: f32 = 2.6;
const BLAST_TAU: f32 = 3.4;
// The shape below which no billow is left, and how much more the folds between billows need.
const RIM: f32 = 0.18;
const EROSION: f32 = 0.5;
// The sun's and the sky's light on smoke against the sprites'.
const DIRECT_GAIN: f32 = 0.75;
const SKY_GAIN: f32 = 0.8;
// How much of the frame temporal anti-aliasing takes over its history where the smoke is thick.
const RESPONSE: f32 = 0.4;

struct Varyings {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    if body.march.x > 0.5 {
        // The lens is in the box: one triangle over the screen at the near plane.
        if index < 3u {
            out.position = vec4<f32>(f32(i32(index & 1u) * 4 - 1), f32(i32(index >> 1u) * 4 - 1), 1.0, 1.0);
        } else {
            out.position = vec4<f32>(-2.0, -2.0, 1.0, 1.0);
        }
        return out;
    }
    // The box's faces, wound counter-clockwise seen from outside (the pipeline culls the far ones).
    let corners = array<u32, 36>(
        0u, 4u, 6u, 0u, 6u, 2u, 1u, 3u, 7u, 1u, 7u, 5u, 0u, 1u, 5u, 0u, 5u, 4u,
        2u, 6u, 7u, 2u, 7u, 3u, 0u, 2u, 3u, 0u, 3u, 1u, 4u, 5u, 7u, 4u, 7u, 6u);
    let corner = corners[index];
    let unit = vec3<f32>(f32(corner & 1u), f32((corner >> 1u) & 1u), f32((corner >> 2u) & 1u));
    out.position = world_to_clip(body.origin.xyz + unit * body.dims.xyz * body.origin.w);
    return out;
}

// Interleaved gradient noise (Jimenez), stepped each frame so temporal anti-aliasing averages the march's steps.
fn jitter_at(pixel: vec2<f32>) -> f32 {
    let p = pixel + 5.588238 * f32(camera.frame % 64u);
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

// Billows, 0–1, high at their hearts: the Worley volume's three cell sizes (4, 8 and 12 to the period).
fn billows(p: vec3<f32>) -> f32 {
    let w = textureSampleLevel(noise, noise_sampler, p, 0.0).rgb;
    return w.r * 0.58 + w.g * 0.28 + w.b * 0.14;
}

// Henyey-Greenstein times 4π: 1 for an even scatterer.
fn henyey(cosine: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(1.0 + g2 - 2.0 * g * cosine, 1.5);
}

// The balls at a world point from both grids: x the union of their shapes (1 at a ball's centre), then their heat,
// albedo and age, each 0–1 as the grids store them.
fn balls_at(world: vec3<f32>) -> vec4<f32> {
    let local = (world - body.origin.xyz) / body.origin.w;
    // The outermost half voxel is never read: beyond it a texture holds another frame's voxels.
    if any(local < vec3<f32>(0.5)) || any(local > body.dims.xyz - 0.5) {
        return vec4<f32>(0.0);
    }
    let far = textureSampleLevel(far_grid, grid_sampler, local / body.grid.xyz, 0.0);
    // A grid stores the root of a shape, itself the root of the balls' squares summed: the grids join by that sum.
    let far_shape = far.r * far.r * SHAPE_MAX;
    var sum = far_shape * far_shape;
    var carried = far.gba * sum;
    let near_local = (world - body.near_origin.xyz) / body.near_origin.w;
    if all(near_local > vec3<f32>(0.5)) && all(near_local < body.near_dims.xyz - 0.5) {
        let near = textureSampleLevel(near_grid, grid_sampler, near_local / body.near_grid.xyz, 0.0);
        let near_shape = near.r * near.r * SHAPE_MAX;
        let weight = near_shape * near_shape;
        sum += weight;
        carried += near.gba * weight;
    }
    return vec4<f32>(sqrt(sum), carried / max(sum, 1e-6));
}

// The light grid at a world point inside the box: the sun's and the sky's way here, the balls' strength, and above 0
// where any ball reaches.
fn light_at(world: vec3<f32>) -> vec4<f32> {
    let local = (world - body.origin.xyz) / body.origin.w;
    return textureSampleLevel(light_grid, grid_sampler, local / body.grid.xyz, 0.0);
}

// A column puff's radius at an age (`fires.rs`: its size and its growth, slowing).
fn column_radius(age: f32) -> f32 {
    return 4.5 + 26.0 * (1.0 - exp(-age * 0.05));
}

// Where the noise is read for the smoke at a point: the small billows in two phases, each carried along the smoke's
// motion for a few seconds and then begun again while the other shows (a flow map's two phases), and the broad ones.
struct Flow {
    first: vec3<f32>,
    second: vec3<f32>,
    // The second phase's share.
    blend: f32,
    broad: vec3<f32>,
}

// A column: smoke climbs fast off the fire and slower aloft, and leans with the wind, more of it the higher.
fn column_flow(world: vec3<f32>, age: f32) -> Flow {
    let at = vec3<f32>(world.x - body.anchor.x, world.y, world.z - body.anchor.z);
    let carry = 0.35 + 0.65 * sstep(0.0, 12.0, age);
    let velocity = vec3<f32>(body.flow.x * carry, 8.5 * exp(-age / 8.0) + 2.7, body.flow.z * carry);
    let cycle = body.anchor.w / PHASE_SECONDS;
    let lead = fract(cycle);
    let lag = fract(cycle + 0.5);
    // Each run of a phase reads another part of the noise.
    let run = vec2<f32>(floor(cycle), floor(cycle + 0.5));
    var flow: Flow;
    flow.first = (at - velocity * ((lead - 0.5) * PHASE_SECONDS)) / FINE_PERIOD + vec3<f32>(0.37, 0.71, 0.13) * run.x;
    flow.second = (at - velocity * ((lag - 0.5) * PHASE_SECONDS)) / FINE_PERIOD + vec3<f32>(0.59, 0.23, 0.83) * run.y + 0.5;
    flow.blend = abs(lead * 2.0 - 1.0);
    flow.broad = (at - vec3<f32>(body.flow.x * body.anchor.w, body.flow.y, body.flow.z * body.anchor.w)) / BROAD_PERIOD;
    return flow;
}

// A detonation: the noise swells about its seat with the cloud, and boils slowly. Once the burst has slowed, the
// small billows stop swelling with it and stay the size turbulence leaves them, climbing with the cloud.
fn blast_flow(world: vec3<f32>) -> Flow {
    let age = body.flow.w;
    let boil = age * 0.012;
    let from_seat = world - body.anchor.xyz;
    let p = from_seat / (max(body.grid.w, 1.0) * 0.9);
    var flow: Flow;
    flow.first = p * 2.7 + vec3<f32>(0.71, 0.29 - boil * 2.2, 0.05);
    flow.second = (from_seat - vec3<f32>(body.flow.x, 2.2, body.flow.z) * age) / BLAST_PERIOD;
    flow.blend = sstep(0.8, 3.5, age);
    flow.broad = p * 1.4 + vec3<f32>(0.13, -boil, 0.57);
    return flow;
}

// The small billows at a flow, `shift` noise periods aside.
fn small_billows(flow: Flow, shift: vec3<f32>) -> f32 {
    if flow.blend >= 1.0 {
        return billows(flow.second + shift);
    }
    let first = billows(flow.first + shift);
    if flow.blend <= 0.0 {
        return first;
    }
    let second = billows(flow.second + shift);
    let w = flow.blend;
    // Two patterns averaged are flatter than either: the blend is stretched back about their mean.
    return 0.45 + ((first - 0.45) * (1.0 - w) + (second - 0.45) * w) / sqrt((1.0 - w) * (1.0 - w) + w * w);
}

// The smoke at a point.
struct Medium {
    // How much of the balls' shape the billows leave here, 0–1.
    solid: f32,
    // The balls' shape before the erosion.
    shape: f32,
    // Heat (a column's underglow, a detonation's burning gas), albedo, age (s).
    heat: f32,
    shade: f32,
    age: f32,
    // The billows here, 0 between them to 1 at their hearts; the small and the broad ones as the noise gives them.
    billow: f32,
    small: f32,
    broad: f32,
    // The size of a puff here (m), and how much the broad billows shape it.
    scale: f32,
    wide: f32,
}

// The smoke where the grids hold `s` and the noise is read at `flow`.
fn medium_of(s: vec4<f32>, flow: Flow) -> Medium {
    var m = Medium(0.0, s.x, s.y * HEAT_MAX, s.z, s.w * AGE_MAX, 0.0, 0.0, 0.0, 1.0, 0.6);
    if body.dims.w < 0.5 {
        m.scale = column_radius(m.age);
        // Little of the broad billows about a column's young, small puffs.
        m.wide = 0.15 + 0.5 * sstep(6.0, 18.0, m.scale);
    } else {
        // A detonation's grid carries its lobes' radius in the age.
        m.scale = max(m.age, 4.0);
        m.wide = 0.5;
    }
    // Under the thinnest shape any billow keeps, there is nothing: no noise is read.
    if m.shape < RIM {
        return m;
    }
    m.small = small_billows(flow, vec3<f32>(0.0));
    m.broad = billows(flow.broad);
    m.billow = sstep(0.2, 0.78, mix(m.small, m.broad, m.wide));
    // Deep in the body the smoke is whole; toward its rim only the billows' hearts are left, each with a firm edge.
    // More ragged in some reaches of the broad billows than in others.
    // A detonation's lobes overlap deeply: its billows cut further in, or the cloud is one round mass.
    let erosion = EROSION * mix(0.65, 1.3, m.broad) * select(1.0, 1.7, body.dims.w > 0.5);
    m.solid = sstep(0.0, 0.09, m.shape - (RIM + 0.02 + erosion * (1.0 - m.billow)));
    return m;
}

// Burning gas: how much of the gas here burns (a), and what it emits (rgb), as the gas sprites' heat ramp gives it
// (`gas_sprite.wgsl`): as the gas cools only its hottest hearts still burn.
fn fire_at(m: Medium, world: vec3<f32>) -> vec4<f32> {
    if m.heat < 0.03 {
        return vec4<f32>(0.0);
    }
    // Flames a few metres across whatever the cloud's size, climbing: a small burst is not one even ball of fire.
    let licks = billows(world / 22.0 - vec3<f32>(0.0, body.anchor.w * 0.25, 0.0));
    let temperature = clamp(m.shape * 0.9, 0.0, 1.0) * mix(0.5, 1.3, m.small) * mix(0.6, 1.25, licks);
    let threshold = clamp(1.0 - m.heat * 0.38, 0.1, 0.96);
    let burning = sstep(threshold - 0.05, threshold + 0.2, temperature);
    let heat = burning * min(m.heat, 1.7) * (0.45 + temperature * 0.5);
    let ember = mix(vec3<f32>(0.45, 0.03, 0.004), vec3<f32>(2.4, 0.45, 0.04), sstep(0.12, 0.55, heat));
    let flame = mix(ember, vec3<f32>(4.2, 1.55, 0.3), sstep(0.55, 1.0, heat));
    return vec4<f32>(flame * (heat * heat) + vec3<f32>(3.5, 2.8, 1.5) * sstep(1.25, 1.6, heat), burning);
}

// Fuel-oil smoke is near black with a warm brown cast; steam and pale smoke are neutral.
fn smoke_albedo(shade: f32) -> vec3<f32> {
    return shade * mix(vec3<f32>(1.1, 1.0, 0.86), vec3<f32>(1.0, 1.01, 1.03), sstep(0.3, 0.6, shade));
}

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    let dir = view_ray(ndc_from_pixel(pixel));
    let eye = camera.position;
    let voxel = body.origin.w;
    // The ray's run through the box, less the grid's outermost half voxel, up to the scene behind.
    let low = body.origin.xyz + voxel * 0.5;
    let high = body.origin.xyz + (body.dims.xyz - 0.5) * voxel;
    let safe = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let a = (low - eye) / safe;
    let b = (high - eye) / safe;
    let entry = min(a, b);
    let exit = max(a, b);
    let t0 = max(max(entry.x, entry.y), max(entry.z, 0.0));
    let forward = -camera.inverse_view[2].xyz;
    let along = max(dot(dir, forward), 1e-4);
    let scene = view_depth(textureLoad(scene_depth, vec2<i32>(pixel), 0)) / along;
    let t1 = min(min(min(exit.x, exit.y), exit.z), scene);
    if t1 <= t0 {
        discard;
    }

    let detonation = body.dims.w > 0.5;
    let sun = normalize(lighting.sun_direction.xyz);
    let cosine = dot(dir, sun);
    // Smoke scatters a little back and more on toward the light: thin edges against the sun glow.
    let phase = mix(henyey(cosine, -0.25), henyey(cosine, 0.45), 0.5);
    // The sky's light on smoke comes from all of it, the pale horizon and the clouds with the blue overhead: less
    // blue than the zenith's.
    let overhead = sky_fill() * SKY_GAIN;
    let sky = mix(overhead, vec3<f32>(dot(overhead, vec3<f32>(0.2126, 0.7152, 0.0722))), 0.8);
    let direct = lighting.direct.rgb * DIRECT_GAIN;
    var flashing = false;
    for (var i = 0; i < 4; i++) {
        flashing = flashing || lighting.flashes[i].w > 0.0;
    }
    // How far toward the sun the billows' relief is read, in periods of the small and of the broad billows.
    let small_reach = sun * select(3.0 / FINE_PERIOD, 0.12, detonation);
    let broad_reach = sun * select(8.0 / BROAD_PERIOD, 0.1, detonation);

    // Long steps where the light grid holds no ball, short ones through the smoke: a fraction of a puff's size, and
    // longer with distance (a step no finer than the pixel).
    let coarse = voxel * body.march.z;
    let jitter = jitter_at(pixel);
    var t = t0 + coarse * jitter;
    var transmittance = 1.0;
    var radiance = vec3<f32>(0.0);
    // Where the smoke the eye sees lies: its nearest, and the mean of what it scatters.
    var front = -1.0;
    var weight = 0.0;
    var mean = 0.0;
    // The march's gait: 0 in long steps where no ball reaches, 1 in hops within a ball's reach but outside its
    // billows, 2 in short steps through them. A longer step that lands on something goes back over its own length in
    // the next gait down, so nothing thinner than a step is skipped or met at the step's phase (which would show as
    // the jitter's pattern).
    var gait = 0;
    // The last step taken, how much of a long step is still to go back over, and the short steps that found nothing.
    var last = coarse;
    var search = 0.0;
    var empty = 0;
    // A hop is no longer than the smallest billows.
    let hop_most = select(2.5, clamp(body.grid.w * 0.03, 1.5, 4.0), detonation);
    let steps = i32(body.march.w);
    for (var i = 0; i < steps; i++) {
        if t >= t1 || transmittance < 0.02 {
            break;
        }
        let world = eye + dir * t;
        let lit = light_at(world);
        if lit.a <= 0.0 {
            if search > 0.0 {
                t += hop_most;
                search -= hop_most;
                last = hop_most;
            } else {
                t += coarse;
                last = coarse;
                gait = 0;
            }
            continue;
        }
        if gait == 0 {
            gait = 1;
            if last > hop_most * 1.5 {
                search = last;
                t = max(t - last, t0) + hop_most * jitter;
                last = hop_most;
                continue;
            }
        }
        let s = balls_at(world);
        var flow: Flow;
        if detonation {
            flow = blast_flow(world);
        } else {
            flow = column_flow(world, s.w * AGE_MAX);
        }
        let m = medium_of(s, flow);
        // A step no finer than a few pixels, and longer the more steps the ray has taken: a ray through a long run of
        // thin smoke reaches its end rather than stopping short.
        var stride = max(max(0.3, t * 0.006), m.scale * body.march.y);
        if detonation && m.heat > 0.25 {
            // Burning gas is thin and its flames fine: shorter steps through it, or they show the jitter's pattern.
            stride = min(stride, max(1.4, t * 0.006));
        }
        stride *= 1.0 + f32(i) * 0.012;
        if m.solid <= 0.0 {
            // Outside the billows: on by a hop, or, just out of the smoke, by the short step until a few have found
            // nothing.
            var hop = stride;
            if gait == 2 {
                empty += 1;
                if empty > 4 {
                    gait = 1;
                }
            } else {
                hop = clamp(m.scale * 0.2, stride, max(hop_most, stride));
            }
            t += hop;
            last = hop;
            search = max(search - hop, 0.0);
            continue;
        }
        empty = 0;
        if gait != 2 {
            gait = 2;
            if last > stride * 1.5 {
                t = max(t - last + stride * jitter, t0);
                last = stride;
                continue;
            }
        }
        last = stride;
        search = max(search - stride, 0.0);
        var fire = vec4<f32>(0.0);
        if detonation {
            fire = fire_at(m, world - body.anchor.xyz);
        }
        // As thick as its balls' strength makes it, a sprite's opacity read as an optical depth. Burning gas is thin
        // and bright; it thickens into soot as it cools. The smoke ends softly on the scene.
        let strength = -log(1.0 - min(lit.b, 0.95)) / 1.6;
        let thickness = select(COLUMN_TAU, BLAST_TAU, detonation) / m.scale;
        let sigma = m.solid * strength * thickness * mix(1.0, 0.3, fire.a) * sstep(0.0, 2.5, scene - t);
        // The sun through the body (the light grid), and past the billows toward it: a billow whose heart lies
        // toward the sun from here shades this.
        var relief = 0.0;
        if m.wide < 0.6 {
            relief += (small_billows(flow, small_reach) - m.small) * (1.0 - m.wide);
        }
        if m.wide > 0.05 {
            relief += (billows(flow.broad + broad_reach) - m.broad) * m.wide;
        }
        let tau_sun = max(-log(max(lit.r, 0.004)) * TAU_SCALE + relief * 8.0, 0.0);
        let tau_sky = -log(max(lit.g, 0.004)) * TAU_SCALE;
        let beam = exp(-tau_sun);
        // Light scattered more than once reaches deeper than the beam.
        let scattered = exp(-tau_sun * 0.3) * 0.2;
        // A detonation's soot is blacker than its particles' tint (the sprites take that tint twice).
        let albedo = smoke_albedo(m.shade) * (1.0 - fire.a * 0.85) * select(1.0, 0.42, detonation);
        // The sky reaches the body's flanks from the side as well as its top from above: less of it the deeper in,
        // and in the folds between billows.
        let open = (0.4 + 0.6 * exp(-tau_sky * 0.3)) * mix(1.0, 0.5, sstep(0.5, 1.2, m.shape)) * mix(0.35, 1.0, m.billow);
        var light = direct * (beam * phase + scattered) + sky * open;
        if flashing {
            var flash = 0.0;
            for (var f = 0; f < 4; f++) {
                let offset = lighting.flashes[f].xyz - world;
                flash += lighting.flashes[f].w / (dot(offset, offset) + 64.0);
            }
            light += lighting.flash_color.rgb * min(flash, 2.5 * lighting.emission.w);
        }
        var source = albedo * light;
        var emission = vec3<f32>(0.0);
        if detonation {
            // What a metre of burning gas emits: a fireball's depth of it is white-hot.
            emission = emitted_at(fire.rgb * m.solid * (2.2 / m.scale), lighting.flash_glow.x);
        } else {
            // The fire below lights the youngest smoke from under.
            source += emitted_at(vec3<f32>(1.0, 0.36, 0.08) * m.heat * (0.1 + 0.3 * (1.0 - exp(-tau_sky))), lighting.emission.z);
        }
        let through = exp(-sigma * stride);
        let held = transmittance * (1.0 - through);
        radiance += held * source + transmittance * emission * stride;
        weight += held;
        mean += held * t;
        if front < 0.0 {
            front = t;
        }
        transmittance *= through;
        t += stride;
    }
    let alpha = 1.0 - transmittance;
    if front < 0.0 || alpha < 0.002 {
        discard;
    }
    let depth = mean / max(weight, 1e-5);
    // Behind gun smoke and behind the clouds, as the sprites are. The occlusion map holds this body too, marched more
    // coarsely: its own face is asked for well in front of where that march may have put it.
    let visible = smoke_visibility(pixel, front * along - (voxel + 8.0), max(depth - front, 4.0), false);
    return vec4<f32>(effect_fog(radiance / alpha, eye + dir * depth), alpha * visible);
}

// The body in the motion pass, for temporal anti-aliasing: its billows move and have no motion vectors, so the history
// would smear them. Where the occlusion map holds smoke in front of the scene, the pixel takes less history.
@fragment
fn response(in: Varyings) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    let uv = pixel * camera.viewport.zw;
    let optical = textureSampleLevel(occlusion_depths, occlusion_sampler, uv, 0.0).r;
    let front = distance_of(textureSampleLevel(occlusion_fronts, occlusion_sampler, uv, 0.0).x);
    if lighting.direct.w <= 0.0 || optical < 0.03 || front > view_depth(textureLoad(scene_depth, vec2<i32>(pixel), 0)) {
        discard;
    }
    return motion_response(RESPONSE * sstep(0.03, 0.6, optical));
}

struct Occlusion {
    // Nearness of the nearest front per coverage level (max blending).
    @location(0) fronts: vec4<f32>,
    // Optical depth and optical depth × nearness of its middle (added).
    @location(1) depths: vec4<f32>,
}

// The same body for the smoke occlusion map (`gas_sprite.wgsl`'s `occluder`), drawn with the gas into its quarter
// resolution targets: sprites behind the body (debris trails, flames, gun smoke) sink into it as they do into gas. A
// coarser march without light, and without the scene's depth (the map is drawn before the scene).
@fragment
fn occluder(in: Varyings) -> Occlusion {
    let pixel = in.position.xy;
    let dir = view_ray(ndc_from_pixel(pixel));
    let eye = camera.position;
    let voxel = body.origin.w;
    let low = body.origin.xyz + voxel * 0.5;
    let high = body.origin.xyz + (body.dims.xyz - 0.5) * voxel;
    let safe = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let a = (low - eye) / safe;
    let b = (high - eye) / safe;
    let entry = min(a, b);
    let exit = max(a, b);
    let start = max(max(entry.x, entry.y), max(entry.z, 0.0));
    let end = min(min(exit.x, exit.y), exit.z);
    if end <= start {
        discard;
    }
    let along = max(dot(dir, -camera.inverse_view[2].xyz), 1e-4);
    let detonation = body.dims.w > 0.5;
    let levels = vec4<f32>(0.25, 0.5, 0.8, 0.95);
    var t = start + voxel * 0.5;
    var optical = 0.0;
    var mean = 0.0;
    var fronts = vec4<f32>(0.0);
    for (var i = 0; i < 96; i++) {
        if t >= end || optical > 4.0 {
            break;
        }
        let world = eye + dir * t;
        let lit = light_at(world);
        if lit.a <= 0.0 {
            t += voxel;
            continue;
        }
        let s = balls_at(world);
        var flow: Flow;
        if detonation {
            flow = blast_flow(world);
        } else {
            flow = column_flow(world, s.w * AGE_MAX);
        }
        let m = medium_of(s, flow);
        let stride = max(max(1.0, t * 0.01), m.scale * 0.2);
        if m.solid > 0.0 {
            let strength = -log(1.0 - min(lit.b, 0.95)) / 1.6;
            let deeper = m.solid * strength * select(COLUMN_TAU, BLAST_TAU, detonation) / m.scale * stride;
            optical += deeper;
            mean += deeper * t;
            // Each level's front is where the smoke so far first covers that much.
            let covered = 1.0 - exp(-optical);
            let near = nearness_of(t * along);
            fronts = max(fronts, near * vec4<f32>(
                sstep(levels.x - 0.1, levels.x + 0.02, covered),
                sstep(levels.y - 0.1, levels.y + 0.02, covered),
                sstep(levels.z - 0.1, levels.z + 0.02, covered),
                sstep(levels.w - 0.1, levels.w + 0.02, covered)));
        }
        t += stride;
    }
    if optical < 0.01 {
        discard;
    }
    var out: Occlusion;
    out.fronts = fronts;
    out.depths = vec4<f32>(optical, optical * nearness_of(mean / optical * along), 0.0, 0.0);
    return out;
}
