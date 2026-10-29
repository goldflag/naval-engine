// Ports and infrastructure into the sun's shadow maps (native/docs/terrain/infra.md): a chunk's
// vertices placed by its instance's translation, depth only (what lies on the ground is never drawn here). Group 0 is
// the map's shadow camera.
#import naval::camera::{camera, world_to_clip}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(4) chunk: vec4<f32>,
}

@vertex
fn vs_infra_caster(in: VertexIn) -> @builtin(position) vec4<f32> {
    return world_to_clip(in.position + in.chunk.xyz);
}
