// Look `merge` (`Trial::merge`): the merged smoke laid over the scene where the gas batch draws (`EffectsLayer`'s merged
// smoke). The gas's and the soot's puffs were added up beforehand in a pass of their own at the backdrop's copy point
// (`merged_smoke` in common.wgsl): their light and opacity, each weighed by what the gas in front of it lets through,
// and what they leave of the scene. Here their light is the weighed mean and their opacity what they cover together,
// blended over the scene as one puff of all of them, so no puff's outline cuts across another's and no order can change
// at once. No TypeScript source: the web game layered its puffs.

#import naval::fullscreen::fullscreen_vertex

@group(2) @binding(0) var merged_sum: texture_2d<f32>;
@group(2) @binding(1) var merged_clear: texture_2d<f32>;

struct Vertex {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    var out: Vertex;
    // On the near plane (reversed depth: 1), so no depth hides it: the smoke was tested against the scene as it added up.
    out.position = fullscreen_vertex(index, 1.0).position;
    return out;
}

@fragment
fn fragment(in: Vertex) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let sum = textureLoad(merged_sum, pixel, 0);
    if sum.a <= 0.0 {
        discard;
    }
    let clear = textureLoad(merged_clear, pixel, 0).r;
    return vec4<f32>(sum.rgb / sum.a, 1.0 - clear);
}
