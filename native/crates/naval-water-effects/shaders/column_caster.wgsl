// A shell splash's shadow (native, `src/columns.rs`): each column's box drawn into the sun's shadow maps, each texel of
// its face toward the light marching the light's way through the water's envelope (`column_field.wgsl`), kept as a
// screen door as dense as the water stops the sun (interleaved gradient noise on the map's texels, which the maps'
// soft filter averages into a partial shadow) and written at the depth where the water it crosses lies on average, so
// what stands inside the column is shaded by the water up-sun of it. The thin mist about it casts nothing: the eye sees
// through it, and its shadow would lie on the sea as a grey stain. A column stops at most `MOST` of the sun (0.7
// until 2026-10-24, when its dense foot stops nearly all of it: spray's shadow, but never quite a hull's).
//
// Group 0 is the map's shadow camera (`naval::camera`, its orthographic matrices).

#import naval::camera::{camera, world_to_clip}
#import naval_water_effects::column_field::{Column, columns, splash_of, is_splash, fan_box, cast_through}

// How much of the sun a column stops at most, and how its optical depth is read as the sun stopped (the march is
// through the envelope, without the tears and gaps the eye sees through).
const MOST: f32 = 0.9;
const DEPTH_SCALE: f32 = 0.35;

struct CasterVaryings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) index: u32,
}

const CORNERS = array<u32, 36>(
    0u, 4u, 6u, 0u, 6u, 2u, 1u, 3u, 7u, 1u, 7u, 5u, 0u, 1u, 5u, 0u, 5u, 4u,
    2u, 6u, 7u, 2u, 7u, 3u, 0u, 2u, 3u, 0u, 3u, 1u, 4u, 5u, 7u, 4u, 7u, 6u);

// Only the column itself casts: its box (not the splash's, which holds the drifting mist), while it stands. A bow's
// burst is too thin and too brief beside her hull to cast, and her sheet too thin and too close along it.
@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> CasterVaryings {
    let column = columns[instance];
    let size = column.origin.w;
    let fan = fan_box(column);
    var out: CasterVaryings;
    out.index = instance;
    if !is_splash(column) || fan.high.y <= 0.0 {
        // Its water is back in the sea: outside the map, no fragments.
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let low = fan.low * size;
    let high = fan.high * size;
    var corners = CORNERS;
    let corner = corners[index];
    let unit = vec3<f32>(f32(corner & 1u), f32((corner >> 1u) & 1u), f32((corner >> 2u) & 1u));
    out.world = column.origin.xyz + mix(low, high, unit);
    out.clip = world_to_clip(out.world);
    return out;
}

// Interleaved gradient noise (Jimenez 2014) on the map's texels, as the other spray casters' (`caster.wgsl`).
fn screen_door(pixel: vec2<f32>) -> f32 {
    let p = floor(pixel);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

struct CasterOut {
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fragment(input: CasterVaryings, @builtin(front_facing) front: bool) -> CasterOut {
    if !front {
        discard;
    }
    let column = columns[input.index];
    let s = splash_of(column, input.index);
    let size = column.origin.w;
    // The light's way, from the box's face toward the light through it.
    let way = normalize(-camera.inverse_view[2].xyz);
    let start = (input.world - column.origin.xyz) / size;
    // The water along the light's way, and where it lies on average: the depth the shadow is written at.
    let through = cast_through(start, way, column, s);
    let stopped = min(1.0 - exp(-through.total * DEPTH_SCALE), MOST) * column.tint.w;
    if stopped <= screen_door(input.clip.xy) {
        discard;
    }
    let at = column.origin.xyz + (start + way * max(through.half, 0.0)) * size;
    let clip = world_to_clip(at);
    var out: CasterOut;
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
