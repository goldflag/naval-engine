// Ships into the sun's shadow maps and the lamps' (no source port): depth only, both faces. Group 0 is the map's
// camera, group 1 the frame (the instances, placed less the main view's eye), group 2 a material for the cut-outs.

#import naval::camera::world_to_clip
#import naval_vessel::common::{frame, instances, material, base_color}

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
}

struct CasterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv0: vec2<f32>,
    @location(1) uv1: vec2<f32>,
    @location(2) color: vec4<f32>,
}

@vertex
fn vs_caster(in: VertexIn, @builtin(instance_index) index: u32) -> CasterOut {
    let offset = (instances[index].model * vec4<f32>(in.position, 1.0)).xyz;
    var out: CasterOut;
    out.clip = world_to_clip(offset + frame.eye.xyz);
    out.uv0 = in.uv0;
    out.uv1 = in.uv1;
    out.color = in.color;
    return out;
}

// A cut-out casts only where it is drawn.
@fragment
fn fs_caster_mask(in: CasterOut) {
    if base_color(in.uv0, in.uv1, in.color).a < material.flags.x {
        discard;
    }
}
