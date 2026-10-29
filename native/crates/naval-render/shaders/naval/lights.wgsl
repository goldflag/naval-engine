// The scene's lights beyond the sun, and its fog switch, as every lit material takes them.
//
// - Point lights: three.js r185's `PointLightNode` (`directPointLight`) with `getDistanceAttenuation`
//   (src/nodes/lighting/LightUtils.js, Frostbite's windowed inverse square). The host hands the frame's lights to the
//   renderer, which keeps the brightest at the camera in one list at group 0 (`scene_lights`, binding 10) and a
//   clustered index of the view (`light_clusters`, binding 11: a bit for each light in each cell of 16 × 9 tiles and 16
//   slices of depth that its light reaches). A surface loops over its cell's lights:
//
//       let cell = light_cell(world);
//       for (var word = 0u; word < LIGHT_WORDS; word++) {
//           var bits = light_word(cell, word);
//           while bits != 0u {
//               let index = word * 32u + firstTrailingBit(bits);
//               bits &= bits - 1u;
//               let light = shadowed_point_light(scene_lights.lights[index], world, normal);
//               ...
//           }
//       }
//
//   The brightest searchlights and star shells cast shadows through maps of their own, layers of the sun's cascade
//   array (`light_shadow`); after sunset the hulls march toward the fires' on screen (`naval-ships`' `fire_seen`, a
//   light's `shadow.z`); every other light is shadowless as three's are. A light the size of a body of flame
//   (`shadow.w`) lights a surface nearer than that as at that distance.
// - Fog: the air between the eye and a fragment is the aerial perspective (`naval::aerial`: the atmosphere and the
//   weather's haze, from the volume the sky fills each frame); `SceneFog` only switches it on for a layer.
//
// Rust twins in naval_render::lights (`SceneLightsUniform`, `LightClusters`, `light_shadow_maps`).
#define_import_path naval::lights

#import naval::camera::{camera, curved}
#import naval::shadow::{shadow_cascades, shadow_sampler}

struct PointLight {
    // xyz world position (m), w cut-off distance (m; 0 reaches everywhere).
    position: vec4<f32>,
    // rgb linear colour × intensity (candela), w decay exponent.
    color: vec4<f32>,
    // xyz outward searchlight direction (zero for a point light), w outer half-angle cosine.
    cone: vec4<f32>,
    // x its shadow map + 1 (0: none), y the map's texel at a metre's distance, z 1 for a light the hulls march toward
    // on screen (`LightCaster::Screen`), w the size of what gives it (m; 0 a point).
    shadow: vec4<f32>,
}

// `naval_render::lights::{MAX_POINT_LIGHTS, LIGHT_WORDS, MAX_SHADOWED_LIGHTS}`.
const MAX_POINT_LIGHTS: u32 = 64u;
const LIGHT_WORDS: u32 = 2u;
const MAX_SHADOWED_LIGHTS: u32 = 8u;

struct SceneLights {
    // x the lights in use, y the shadowed lights, z their maps' first layer in the sun's cascade array, w unused.
    count: vec4<u32>,
    // The clustered index: columns, rows, slices and words a cell.
    grid: vec4<u32>,
    // x where the first slice ends (m), y slices per natural log of depth past it, zw unused.
    slices: vec4<f32>,
    lights: array<PointLight, MAX_POINT_LIGHTS>,
    // World to each light map's (u, v, reversed depth), v down.
    shadows: array<mat4x4<f32>, MAX_SHADOWED_LIGHTS>,
}

@group(0) @binding(10) var<uniform> scene_lights: SceneLights;
@group(0) @binding(11) var<storage, read> light_clusters: array<u32>;

struct SceneFog {
    // rgb the authored fog colour, a the old ramp's power (no longer drawn).
    color: vec4<f32>,
    // x start, y end (the visibility, m), z sky-blend distance (m), w 1 while the aerial perspective is on: only w is
    // read here; the visibility reaches the sky's haze from the host.
    params: vec4<f32>,
}

// A point light at a surface: the unit direction toward the light and its colour × attenuation.
struct LightSample {
    direction: vec3<f32>,
    radiance: vec3<f32>,
}

// three's `getDistanceAttenuation`: 1 / max(d^decay, 0.01), windowed to 0 at the cut-off when there is one.
fn distance_attenuation(distance: f32, cutoff: f32, decay: f32) -> f32 {
    let falloff = 1.0 / max(pow(distance, decay), 0.01);
    if cutoff > 0.0 {
        let ratio = distance / cutoff;
        let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
        return falloff * window * window;
    }
    return falloff;
}

// `directPointLight`: toward the light from `world`, its colour times the attenuation over the distance, held at the
// light's size nearer it (`shadow.w`): beside a body of flame a surface takes no more than the flame's luminance times
// its albedo, however near it stands.
fn point_light_at(light: PointLight, world: vec3<f32>) -> LightSample {
    let to_light = light.position.xyz - world;
    let distance = length(to_light);
    var out: LightSample;
    out.direction = to_light / max(distance, 1e-4);
    out.radiance = light.color.rgb * distance_attenuation(max(distance, light.shadow.w), light.position.w, light.color.w);
    if dot(light.cone.xyz, light.cone.xyz) > 0.5 {
        let inner = mix(light.cone.w, 1.0, 0.15);
        out.radiance *= smoothstep(light.cone.w, inner, dot(light.cone.xyz, -out.direction));
    }
    return out;
}

// The light's own shadow at `world` (1 lit, 0 shadowed), with the surface's `normal` (zero for none): its map, when it
// has one, sampled as the sun's maps are (reversed depth, `GreaterEqual`), four bilinear comparisons over 3 × 3 texels.
// The receiver steps off its surface by a texel and a half there and toward the light by one, so a surface does not
// shadow itself. Outside the map (behind the light, past its frustum) it is lit.
fn light_shadow(light: PointLight, world: vec3<f32>, normal: vec3<f32>) -> f32 {
    let map = u32(light.shadow.x + 0.5);
    if map == 0u || map > scene_lights.count.y {
        return 1.0;
    }
    let to_light = light.position.xyz - world;
    let distance = length(to_light);
    let texel = light.shadow.y * distance;
    let receiver = world + normal * (1.5 * texel) + to_light * (texel / max(distance, 1e-3));
    let projected = scene_lights.shadows[map - 1u] * vec4<f32>(receiver, 1.0);
    if projected.w <= 0.0 {
        return 1.0;
    }
    let coords = projected.xyz / projected.w;
    if any(coords.xy < vec2<f32>(0.0)) || any(coords.xy > vec2<f32>(1.0)) || coords.z <= 0.0 || coords.z >= 1.0 {
        return 1.0;
    }
    let layer = i32(scene_lights.count.z + map - 1u);
    let step = 0.5 / f32(textureDimensions(shadow_cascades).x);
    var lit = 0.0;
    lit += textureSampleCompareLevel(shadow_cascades, shadow_sampler, coords.xy + vec2<f32>(-step, -step), layer, coords.z);
    lit += textureSampleCompareLevel(shadow_cascades, shadow_sampler, coords.xy + vec2<f32>(step, -step), layer, coords.z);
    lit += textureSampleCompareLevel(shadow_cascades, shadow_sampler, coords.xy + vec2<f32>(-step, step), layer, coords.z);
    lit += textureSampleCompareLevel(shadow_cascades, shadow_sampler, coords.xy + vec2<f32>(step, step), layer, coords.z);
    // A star shell's map fades out toward its edge, where its texels are coarsest, rather than ending in a line.
    let edge = max(abs(coords.x - 0.5), abs(coords.y - 0.5)) * 2.0;
    return mix(lit * 0.25, 1.0, smoothstep(0.9, 1.0, edge));
}

// `point_light_at` shaded by the light's own map (`light_shadow`).
fn shadowed_point_light(light: PointLight, world: vec3<f32>, normal: vec3<f32>) -> LightSample {
    var out = point_light_at(light, world);
    if light.shadow.x > 0.5 && dot(out.radiance, out.radiance) > 0.0 {
        out.radiance *= light_shadow(light, world, normal);
    }
    return out;
}

// The clustered index's cell holding `world` as this view draws it: its tile on screen and its slice of depth.
fn light_cell(world: vec3<f32>) -> u32 {
    let clip = camera.view_projection * vec4<f32>(curved(world), 1.0);
    let grid = scene_lights.grid;
    let depth = clip.w;
    let ndc = clip.xy / max(abs(clip.w), 1e-4);
    let column = u32(clamp(i32((ndc.x * 0.5 + 0.5) * f32(grid.x)), 0, i32(grid.x) - 1));
    let row = u32(clamp(i32((0.5 - ndc.y * 0.5) * f32(grid.y)), 0, i32(grid.y) - 1));
    var slice = 0u;
    if depth > scene_lights.slices.x {
        slice = min(u32(1.0 + log(depth / scene_lights.slices.x) * scene_lights.slices.y), grid.z - 1u);
    }
    return (slice * grid.y + row) * grid.x + column;
}

// Word `word` of cell `cell`: a bit for each of 32 lights (light `word * 32 + bit`).
fn light_word(cell: u32, word: u32) -> u32 {
    return light_clusters[cell * LIGHT_WORDS + word];
}
