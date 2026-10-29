// Photo mode's focus probe (native/docs/photo.md, `naval_render::photo`): the scene's reversed depth at one texel, for
// the CPU to read back (a depth texture cannot be copied in part). No TypeScript source.

@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var<uniform> at: vec4<u32>;
@group(0) @binding(2) var<storage, read_write> probed: array<f32, 4>;

@compute @workgroup_size(1)
fn probe() {
    let last = textureDimensions(depth) - vec2<u32>(1u);
    probed[0] = textureLoad(depth, vec2<i32>(min(at.xy, last)), 0);
}
