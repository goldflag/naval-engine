// The end of the post chain: a display-space frame (graded, tone mapped, sRGB-encoded) onto the
// output, with the game's edge smoothing (the web game's `controllers/GraphicsController.ts`
// `buildPipeline`) and the render scale's upscale (Game.ts `setPixelRatio(... * renderScale /
// 100)`, which the browser's compositor stretches bilinearly onto the page):
//
// - `fxaa_output`: `fxaa(frame)` (three r185 FXAANode; see naval::fxaa).
// - `taa_output`: temporal AA's resolve with FXAA where it took no history (alpha <= 0: the sea,
//   tracers, disocclusions), `mix(resolved, fxaa(resolved), step(resolved.a, 0))`, and a light
//   sharpen where it took history (`TAA_SHARPEN`).
// - `upscale`: the frame stretched bilinearly to the output (render scale below 100 %).
//
// The first two run at the frame's own size. Onto an 8- or 10-bit output (DITHER_LEVELS) the
// values are dithered (`naval::output`); an sRGB target (DECODE_SRGB), which encodes in hardware,
// gets them decoded after. An HDR output's frame holds values above SDR white, written as they are.
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::fxaa::fxaa
#import naval::encode::encode

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

fn texel() -> vec2<f32> {
    return 1.0 / vec2<f32>(textureDimensions(frame));
}

@fragment
fn fxaa_output(in: FullscreenVertex) -> @location(0) vec4<f32> {
    // Pixel centres exactly: this pass draws at the frame's size.
    let size = texel();
    return encode(fxaa(frame, bilinear, in.position.xy * size, size).rgb, in.position.xy);
}

// Temporal AA's history and its filtered sample soften an edge a little against FXAA. Against a 2x
// supersampled frame (superstructure and rigging, `native/docs/aa-tracers.md`), this unsharp mask on
// the four neighbours brings the gradient back to FXAA's and the picture nearer the supersampled
// one; clamped to the neighbourhood's range, it adds no halo. Here, not in the resolve, so the
// history it reads next frame is never sharpened twice.
const TAA_SHARPEN: f32 = 0.3;

@fragment
fn taa_output(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let resolved = textureLoad(frame, pixel, 0);
    if resolved.a > 0.0 {
        let last = vec2<i32>(textureDimensions(frame)) - 1;
        let c = resolved.rgb;
        let n = textureLoad(frame, clamp(pixel + vec2<i32>(0, -1), vec2<i32>(0), last), 0).rgb;
        let s = textureLoad(frame, clamp(pixel + vec2<i32>(0, 1), vec2<i32>(0), last), 0).rgb;
        let e = textureLoad(frame, clamp(pixel + vec2<i32>(1, 0), vec2<i32>(0), last), 0).rgb;
        let w = textureLoad(frame, clamp(pixel + vec2<i32>(-1, 0), vec2<i32>(0), last), 0).rgb;
        let low = min(c, min(min(n, s), min(e, w)));
        let high = max(c, max(max(n, s), max(e, w)));
        let sharpened = c + TAA_SHARPEN * (c - 0.25 * (n + s + e + w));
        return encode(clamp(sharpened, low, high), in.position.xy);
    }
    let size = texel();
    return encode(fxaa(frame, bilinear, in.position.xy * size, size).rgb, in.position.xy);
}

@fragment
fn upscale(in: FullscreenVertex) -> @location(0) vec4<f32> {
    return encode(textureSampleLevel(frame, bilinear, in.uv, 0.0).rgb, in.position.xy);
}
