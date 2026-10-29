// The land as a sun shadow caster: the patches `land_shadows.rs` chose for this map, placed as `terrain.wgsl` places
// them (without the morph: the caster selection is a single level), so the land shades the ships, the sea, its trees
// and buildings (native/docs/terrain/shadows.md, "The land's shadow"). On with `TerrainSettings::cast_shadows`.
//
// Two departures from a plain caster:
// - Pushed back along the light, by the instance's base (its `w`: metres for this map) and by how far the land's slope
//   runs along the light across the receivers' filter: the land's own surface must never shade itself through the maps,
//   since it keeps its baked shadow (`sunVisibility`), which `terrain.wgsl` takes with them. Only the first metres of a
//   shadow along the light from its caster are lost (the shore's own on the sea beside it).
// - Clamped onto the map's near plane rather than clipped there: a cliff nearer the sun than the box's near plane (a
//   low sun's shadow runs kilometres) still shades what lies behind it in the box.
#import naval::camera::{camera, world_to_clip}
#import naval_terrain::bindings::{terrain, gradients, clamp_sampler, height_at, field_uv, chart_offset}

// Texels of the map across which a slope's depth must stay behind the receivers it holds: the soft filter's reach about
// a sample (a B-spline over 6 × 6 texels) and its receiver offset.
const PUSH_TEXELS: f32 = 3.5;
// The most a slope adds (m): a face nearly along the light takes almost none of the sun, so its own shading hides it.
const PUSH_MAX_M: f32 = 40.0;

struct VertexIn {
    @location(0) grid: vec2<f32>,
    @location(1) corner: vec4<f32>,
}

@vertex
fn vs_caster(in: VertexIn) -> @builtin(position) vec4<f32> {
    let step = terrain.field.w * exp2(in.corner.z);
    let xz = in.corner.xy + in.grid * step;
    let offset = chart_offset();
    let world = vec3<f32>(xz.x + offset.x, height_at(xz), xz.y + offset.y);
    // The shadow camera's z axis points toward the light; its orthographic half-width sets the texel (m).
    let toward = normalize(camera.inverse_view[2].xyz);
    let texel = 2.0 / (camera.projection[0][0] * camera.viewport.x);
    let gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(xz), 0.0).xy;
    let normal = normalize(vec3<f32>(-gradient.x, 1.0, -gradient.y));
    // The depth a slope gains across one texel of the map: the cotangent of its angle to the light.
    let facing = clamp(dot(normal, toward), 0.0, 1.0);
    let run = sqrt(max(1.0 - facing * facing, 0.0)) / max(facing, 1e-3);
    let push = in.corner.w + min(PUSH_TEXELS * texel * run, PUSH_MAX_M);
    var clip = world_to_clip(world - toward * push);
    // Reversed depth: nearer the light than the near plane is above 1 (w is 1 for the orthographic map).
    clip.z = min(clip.z, clip.w);
    return clip;
}
