// The sea's height about each hull, for the hulls' wet band: the web game's `HullWetBand.ts` reads, per hull fragment, the sea
// Game.ts hands it (`ocean.waveField.heightAt(xz)` with the sea around hulls of the web game's `ocean/waves/hullSea.ts`, plus
// `shipWake.surfaceHeight(x, z)`, the web game's `ShipWake.ts`: the wake field's height and the bow waves' at the vertex stage's
// feature size). The ship pipeline has no group free for the wave field (and the field's arrays change every frame), so
// the ocean evaluates the same heights here, once a frame, on a patch about each hull it couples and the camera sees:
// one layer per patch, its texels (as many as its distance needs) in the layer's corner from stern to stem (x) and port
// to starboard (y), the outer ones on the patch's edges. naval-ships' ship.wgsl reads them back bilinearly
// (`sea_height`).
//
// The heights are the sea as drawn against the hull (`drawn_height_at` at the water mesh's spacing there), not the wave
// field's full detail: a hull far from the mesh's centre (a ship on the horizon through binoculars) stands in water
// drawn flat, and plating the full waves would cover was lit as under water, black strips above the sea drawn.
//
// `sides_main` keeps the water's memory against each hull's sides (`naval_ocean::hull_sides`, native only): per hull,
// side and station from her stem aft, how high the water stands up her side now and has lately climbed (her frame,
// metres over her design waterline), and how hard it rushes up her side (m/s). It writes the readers' copy into the
// heights' last layer, a row per channel.

#import naval_ocean::waves::drawn_height_at
#import naval_ocean::wake::{wake_height, mesh_spacing}

// `naval_ocean::hull_heights::HullHeightsUniforms`: per patch (centre x, z, bow axis x, z) and (half length, half
// breadth, texels along, texels across); then (patches in use, ·, ·, ·).
struct HullPatches {
    patches: array<vec4<f32>, 32>,
    count: vec4<f32>,
}

@group(3) @binding(0) var<uniform> hull_patches: HullPatches;
@group(3) @binding(1) var heights_out: texture_storage_2d_array<r32float, write>;

// `naval_ocean::hull_sides::SideHullUniform`.
struct SideHull {
    // World from hull: her x (starboard), y (up) and z (aft) axes, each with that component of her origin in w.
    axes: array<vec4<f32>, 3>,
    // x 1 in use, y 1 to start her history afresh, z her stem's z on her design waterline (her frame, m), w a station's
    // span (m).
    stations: vec4<f32>,
    // Her profile's stations: x the first one's z, y 1 / their spacing, z their count.
    profile: vec4<f32>,
    // Her half-breadths on her design waterline per profile station: starboard, port, then the next station's (m).
    breadths: array<vec4<f32>, 16>,
}

// `naval_ocean::hull_sides::HullSidesUniforms`.
struct HullSides {
    hulls: array<SideHull, 16>,
    // x the frame's presentation step (s; 0 holds), y the drying (m/s), z the slap's fading and w the rise's easing (s).
    step: vec4<f32>,
}

@group(3) @binding(2) var<uniform> hull_sides: HullSides;
// Per slot, side (port, starboard) and station from her stem aft: (now, climb, slap, rise), the pass's own.
@group(3) @binding(3) var<storage, read_write> side_state: array<vec4<f32>>;

// `naval_ocean::hull_sides::SIDE_STATIONS`, `SIDES_LAYER` and `SideChannel`'s rows.
const SIDE_STATIONS: u32 = 128u;
const SIDE_SLOTS: u32 = 16u;
const SIDES_LAYER: i32 = 16;
const CHANNEL_CLIMB: i32 = 0;
const CHANNEL_SLAP: i32 = 1;
const CHANNEL_NOW: i32 = 2;
const CHANNEL_RISE: i32 = 3;
// Of her world up, the least her own up keeps toward it: a hull on her beam ends reads her sides as if at this tilt.
const SIDE_UPRIGHT: f32 = 0.2;

@compute @workgroup_size(8, 8)
fn heights_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let slot = i32(id.z);
    if slot >= i32(hull_patches.count.x) {
        return;
    }
    let centre = hull_patches.patches[slot * 2];
    let reach = hull_patches.patches[slot * 2 + 1];
    let size = min(vec2<u32>(reach.zw), textureDimensions(heights_out));
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let along = (f32(id.x) / f32(size.x - 1u) * 2.0 - 1.0) * reach.x;
    let across = (f32(id.y) / f32(size.y - 1u) * 2.0 - 1.0) * reach.y;
    let xz = centre.xy + centre.zw * along + vec2<f32>(-centre.w, centre.z) * across;
    // A calm wake (none bound yet) names no mesh: the finest detail, as near the mesh's centre.
    let height = drawn_height_at(xz, max(mesh_spacing(xz), 0.01)) + wake_height(xz);
    textureStore(heights_out, vec2<i32>(id.xy), slot, vec4<f32>(height, 0.0, 0.0, 1.0));
}

// Her half-breadth on her design waterline at `z` (her frame) on `side` (0 port, 1 starboard), between her profile's
// stations.
fn side_breadth(hull: SideHull, z: f32, side: u32) -> f32 {
    let last = hull.profile.z - 1.0;
    let u = clamp((z - hull.profile.x) * hull.profile.y, 0.0, last);
    let k = u32(min(floor(u), last - 1.0));
    let a = hull.breadths[k / 2u];
    let b = hull.breadths[(k + 1u) / 2u];
    // Starboard in x and z, port in y and w; an even station in xy, an odd one in zw.
    let here = select(select(a.y, a.x, side == 1u), select(a.w, a.z, side == 1u), k % 2u == 1u);
    let next = select(select(b.y, b.x, side == 1u), select(b.w, b.z, side == 1u), (k + 1u) % 2u == 1u);
    return mix(here, next, u - f32(k));
}

@compute @workgroup_size(64)
fn sides_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let slot = id.y;
    let side = id.x / SIDE_STATIONS;
    let station = id.x % SIDE_STATIONS;
    if slot >= SIDE_SLOTS || side > 1u {
        return;
    }
    let hull = hull_sides.hulls[slot];
    if hull.stations.x < 0.5 {
        return;
    }
    // Where her side meets her design waterline at the station, in her frame and in the world.
    let z = hull.stations.z + (f32(station) + 0.5) * hull.stations.w;
    let breadth = side_breadth(hull, z, side);
    let p = vec3<f32>(select(-breadth, breadth, side == 1u), 0.0, z);
    let axes = mat3x3<f32>(hull.axes[0].xyz, hull.axes[1].xyz, hull.axes[2].xyz);
    let origin = vec3<f32>(hull.axes[0].w, hull.axes[1].w, hull.axes[2].w);
    let world = origin + axes * p;
    let sea = drawn_height_at(world.xz, max(mesh_spacing(world.xz), 0.01)) + wake_height(world.xz);
    // The water's height up her side in her frame: where her side, upright in her frame, meets the level sea.
    let now = (sea - world.y) / max(hull.axes[1].y, SIDE_UPRIGHT);
    let index = (slot * 2u + side) * SIDE_STATIONS + station;
    let step = hull_sides.step.x;
    var state = side_state[index];
    if hull.stations.y > 0.5 {
        // A hull new to the slot, or one that jumped: dry above the water, nothing rushing.
        state = vec4<f32>(now, now, 0.0, 0.0);
    } else if step > 0.0 {
        let rise = mix(state.w, (now - state.x) / step, 1.0 - exp(-step / hull_sides.step.w));
        let climb = max(now, state.y - hull_sides.step.y * step);
        let slap = max(max(rise, 0.0), state.z * exp(-step / hull_sides.step.z));
        state = vec4<f32>(now, climb, slap, rise);
    }
    side_state[index] = state;
    let texel = vec2<i32>(i32(station + side * SIDE_STATIONS), i32(slot) * 4);
    textureStore(heights_out, texel + vec2<i32>(0, CHANNEL_CLIMB), SIDES_LAYER, vec4<f32>(state.y, 0.0, 0.0, 1.0));
    textureStore(heights_out, texel + vec2<i32>(0, CHANNEL_SLAP), SIDES_LAYER, vec4<f32>(state.z, 0.0, 0.0, 1.0));
    textureStore(heights_out, texel + vec2<i32>(0, CHANNEL_NOW), SIDES_LAYER, vec4<f32>(state.x, 0.0, 0.0, 1.0));
    textureStore(heights_out, texel + vec2<i32>(0, CHANNEL_RISE), SIDES_LAYER, vec4<f32>(state.w, 0.0, 0.0, 1.0));
}
