// Buildings into the sun's shadow maps (native/docs/terrain/buildings.md): a chunk's vertices placed by its
// instance's translation, depth only; and the roof clutter (`vs_clutter_caster`, the near map only) placed as
// `buildings.wgsl` places it. Group 0 is the map's shadow camera.
#import naval::camera::{camera, world_to_clip}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(4) chunk: vec4<f32>,
}

@vertex
fn vs_building_caster(in: VertexIn) -> @builtin(position) vec4<f32> {
    return world_to_clip(in.position + in.chunk.xyz);
}

#ifdef CLUTTER_CASTER
// The chunks' translations by slot.
@group(1) @binding(0) var<storage, read> chunk_translations: array<vec4<f32>>;

// `naval_terrain::buildings::clutter::Clutter` over a `ShapeVertex`, as `buildings.wgsl`'s `ClutterIn`.
struct ClutterIn {
    @location(0) position: vec3<f32>,
    @location(2) at_yaw: vec4<f32>,
    @location(3) size: vec3<f32>,
    @location(4) shape: u32,
}

@vertex
fn vs_clutter_caster(in: ClutterIn) -> @builtin(position) vec4<f32> {
    let t = chunk_translations[in.shape >> 8u];
    let c = cos(in.at_yaw.w);
    let s = sin(in.at_yaw.w);
    let p = in.position * in.size;
    let local = in.at_yaw.xyz + vec3<f32>(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
    return world_to_clip(local + t.xyz);
}
#endif
