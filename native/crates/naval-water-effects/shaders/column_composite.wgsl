// Splash columns laid over the scene (native, `src/layer.rs`). Small distant columns are marched at full scene
// resolution and copied directly. Large columns use half resolution: a compact tent reconstructs premultiplied
// radiance and opacity together, with backdrop-depth weights preserving hulls, wires and the sea intersection.

#import naval::camera::{camera, view_depth}

// The columns at half resolution (premultiplied radiance, opacity), and the backdrop's reversed depth.
@group(1) @binding(0) var marched: texture_2d<f32>;
@group(1) @binding(1) var scene_depth: texture_depth_2d;

// How sharply a texel is weighed down by the depth between where it stopped and this pixel (a share of the distance).
const DEPTH_SHARPNESS: f32 = 30.0;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
}

// The nearest of the backdrop's depths over a texel's 2 × 2 block (reversed: the largest), where its ray stopped
// (`column.wgsl`'s `block_depth`).
fn block_depth(texel: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let at = texel * 2;
    let a = textureLoad(scene_depth, min(at, last), 0);
    let b = textureLoad(scene_depth, min(at + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(scene_depth, min(at + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(scene_depth, min(at + vec2<i32>(1, 1), last), 0);
    return max(max(a, b), max(c, d));
}

// One triangle over the screen, at the near plane (reversed depth: 1), so the scene's depth test passes it everywhere:
// the march stopped at the backdrop already.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Varyings {
    var out: Varyings;
    out.clip = vec4<f32>(f32(i32(index & 1u) * 4 - 1), f32(i32(index >> 1u) * 4 - 1), 1.0, 1.0);
    return out;
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(marched));
    let pixel = vec2<i32>(input.clip.xy);
    // Small distant columns already have one ray per scene pixel; preserve their silhouette exactly.
    if all(size == vec2<i32>(textureDimensions(scene_depth))) {
        let color = textureLoad(marched, pixel, 0);
        if color.a < 0.002 { discard; }
        return color;
    }
    let here = view_depth(textureLoad(scene_depth, pixel, 0));
    // The half-resolution texels about this pixel, in texel-centre coordinates.
    let at = input.clip.xy * 0.5 - 0.5;
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    // Skip reconstruction over empty sea, testing the whole tent footprint so its outer edge is not clipped.
    let nearest = vec2<i32>(floor(at + vec2<f32>(0.5)));
    var coverage = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            coverage += textureLoad(marched, clamp(nearest + vec2<i32>(i, j), vec2<i32>(0), size - 1), 0).a;
        }
    }
    if coverage < 0.002 { discard; }
    var sum = vec4<f32>(0.0);
    var weights = 0.0;
    for (var j = -1; j < 3; j++) {
        for (var i = -1; i < 3; i++) {
            let texel = clamp(base + vec2<i32>(i, j), vec2<i32>(0), size - 1);
            // A tent 1.5 half-res texels wide removes the 2-pixel stair and residual quadrature noise without
            // filling across scene depth edges. Filter radiance and opacity together in premultiplied space.
            let tent = max(vec2<f32>(1.5) - abs(vec2<f32>(f32(i), f32(j)) - f), vec2<f32>(0.0));
            let bilinear = tent.x * tent.y;
            if bilinear <= 0.0 { continue; }
            // The depth that texel's march stopped at.
            let stopped = view_depth(block_depth(texel));
            let near = 1.0 / (1.0 + DEPTH_SHARPNESS * abs(stopped - here) / max(here, 1.0));
            let w = bilinear * near;
            sum += textureLoad(marched, texel, 0) * w;
            weights += w;
        }
    }
    let color = sum / max(weights, 1e-6);
    if color.a < 0.002 {
        discard;
    }
    return color;
}
