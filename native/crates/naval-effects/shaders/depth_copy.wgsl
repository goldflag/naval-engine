// The effects' own copy of the scene's depth, for soft particles, until render core II's scene copies land
// (`scene.backdrop.depth`, native/docs/render-core/frame.md "Scene copies"). Run in `prepare`, before the scene pass,
// it copies what `scene.depth` holds then: the previous frame's opaque and sea depth. No TypeScript source: the
// web takes a viewport copy of the depth buffer when the first effect draws (`EffectLighting.sceneDepth`).

#import naval::fullscreen::fullscreen_vertex

@group(0) @binding(0) var source: texture_depth_2d;

struct Vertex {
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    var out: Vertex;
    out.position = fullscreen_vertex(index, 0.0).position;
    return out;
}

@fragment
fn fragment(in: Vertex) -> @builtin(frag_depth) f32 {
    return textureLoad(source, vec2<i32>(in.position.xy), 0);
}
