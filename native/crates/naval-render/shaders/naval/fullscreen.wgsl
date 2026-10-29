// One triangle covering the viewport, for full-screen passes and far-plane
// backgrounds. No TypeScript source: three draws a quad for the same job.
#define_import_path naval::fullscreen

struct FullscreenVertex {
    @builtin(position) position: vec4<f32>,
    // 0,0 at the top left, 1,1 at the bottom right.
    @location(0) uv: vec2<f32>,
}

// Vertex `index` (0, 1, 2) of the triangle at clip depth `depth`. With reversed
// depth, 0 is the far plane: a sky drawn there after opaque geometry passes
// GreaterEqual only where the clear is left.
fn fullscreen_vertex(index: u32, depth: f32) -> FullscreenVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: FullscreenVertex;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    out.uv = uv;
    return out;
}
