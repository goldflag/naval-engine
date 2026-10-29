// FXAA: edge smoothing on a display-space (tone mapped, sRGB-encoded) frame. Ports three.js
// r185 examples/jsm/tsl/display/FXAANode.js (`ApplyFXAA`, the game's `fxaa(frame)` in
// the web game's `controllers/GraphicsController.ts`) line for line: the same luminance weights,
// contrast and relative thresholds, subpixel blend, six edge steps (1, 1.5, 2, 2, 2, 4) and the
// end guess of 8. `texel` is three's `_invSize` (1 / the input's size); samples are bilinear at
// level 0, as three's `textureNode.bias(-100)`. The texture and sampler are arguments so any
// pass can smooth whatever frame it binds. An HDR frame's highlights are weighed in the
// perceptual domain (`naval::output::perceptual`): its luminance is three's below SDR white.
#define_import_path naval::fxaa

#import naval::output::perceptual

const CONTRAST_THRESHOLD: f32 = 0.0312;
const RELATIVE_THRESHOLD: f32 = 0.063;
const SUBPIXEL_BLENDING: f32 = 1.0;
const EDGE_STEP_COUNT: i32 = 6;
const EDGE_GUESS: f32 = 8.0;

fn fxaa_edge_step(index: i32) -> f32 {
    var steps = array<f32, 6>(1.0, 1.5, 2.0, 2.0, 2.0, 4.0);
    return steps[clamp(index, 0, 5)];
}

fn fxaa_sample(frame: texture_2d<f32>, bilinear: sampler, uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(frame, bilinear, uv, 0.0);
}

fn fxaa_luminance(frame: texture_2d<f32>, bilinear: sampler, uv: vec2<f32>) -> f32 {
    return dot(perceptual(fxaa_sample(frame, bilinear, uv).rgb), vec3<f32>(0.3, 0.59, 0.11));
}

fn fxaa_offset(frame: texture_2d<f32>, bilinear: sampler, uv: vec2<f32>, texel: vec2<f32>, offset: vec2<f32>) -> f32 {
    return fxaa_luminance(frame, bilinear, uv + texel * offset);
}

// The FXAA'd colour at `uv` (alpha as the input's there).
fn fxaa(frame: texture_2d<f32>, bilinear: sampler, uv: vec2<f32>, texel: vec2<f32>) -> vec4<f32> {
    // SampleLuminanceNeighborhood
    let m = fxaa_luminance(frame, bilinear, uv);
    let n = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(0.0, -1.0));
    let e = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(1.0, 0.0));
    let s = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(0.0, 1.0));
    let w = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(-1.0, 0.0));
    let ne = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(1.0, -1.0));
    let nw = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(-1.0, -1.0));
    let se = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(1.0, 1.0));
    let sw = fxaa_offset(frame, bilinear, uv, texel, vec2<f32>(-1.0, 1.0));
    let highest = max(max(max(s, e), max(n, w)), m);
    let lowest = min(min(min(s, e), min(n, w)), m);
    let contrast = highest - lowest;

    var final_uv = uv;
    // ShouldSkipPixel
    if contrast >= max(CONTRAST_THRESHOLD, RELATIVE_THRESHOLD * highest) {
        // DeterminePixelBlendFactor
        var f = 2.0 * (s + e + n + w);
        f += se + sw + ne + nw;
        f *= 1.0 / 12.0;
        f = abs(f - m);
        f = clamp(f / max(contrast, 0.0), 0.0, 1.0);
        let blend_factor = smoothstep(0.0, 1.0, f);
        let pixel_blend = blend_factor * blend_factor * SUBPIXEL_BLENDING;

        // DetermineEdge
        let horizontal = abs(s + n - m * 2.0) * 2.0 + (abs(se + ne - e * 2.0) + abs(sw + nw - w * 2.0));
        let vertical = abs(e + w - m * 2.0) * 2.0 + (abs(se + sw - s * 2.0) + abs(ne + nw - n * 2.0));
        let is_horizontal = horizontal >= vertical;
        let p_luminance = select(e, s, is_horizontal);
        let n_luminance = select(w, n, is_horizontal);
        let p_gradient = abs(p_luminance - m);
        let n_gradient = abs(n_luminance - m);
        var pixel_step = select(texel.x, texel.y, is_horizontal);
        var opposite_luminance: f32;
        var gradient: f32;
        if p_gradient < n_gradient {
            pixel_step = -pixel_step;
            opposite_luminance = n_luminance;
            gradient = n_gradient;
        } else {
            opposite_luminance = p_luminance;
            gradient = p_gradient;
        }

        // DetermineEdgeBlendFactor
        var uv_edge = uv;
        var edge_step: vec2<f32>;
        if is_horizontal {
            uv_edge.y += pixel_step * 0.5;
            edge_step = vec2<f32>(texel.x, 0.0);
        } else {
            uv_edge.x += pixel_step * 0.5;
            edge_step = vec2<f32>(0.0, texel.y);
        }
        let edge_luminance = (m + opposite_luminance) * 0.5;
        let gradient_threshold = gradient * 0.25;

        var puv = uv_edge + edge_step * fxaa_edge_step(0);
        var p_delta = fxaa_luminance(frame, bilinear, puv) - edge_luminance;
        var p_at_end = abs(p_delta) >= gradient_threshold;
        for (var i = 1; i < EDGE_STEP_COUNT; i++) {
            if p_at_end {
                break;
            }
            puv += edge_step * fxaa_edge_step(i);
            p_delta = fxaa_luminance(frame, bilinear, puv) - edge_luminance;
            p_at_end = abs(p_delta) >= gradient_threshold;
        }
        if !p_at_end {
            puv += edge_step * EDGE_GUESS;
        }

        var nuv = uv_edge - edge_step * fxaa_edge_step(0);
        var n_delta = fxaa_luminance(frame, bilinear, nuv) - edge_luminance;
        var n_at_end = abs(n_delta) >= gradient_threshold;
        for (var i = 1; i < EDGE_STEP_COUNT; i++) {
            if n_at_end {
                break;
            }
            nuv -= edge_step * fxaa_edge_step(i);
            n_delta = fxaa_luminance(frame, bilinear, nuv) - edge_luminance;
            n_at_end = abs(n_delta) >= gradient_threshold;
        }
        if !n_at_end {
            nuv -= edge_step * EDGE_GUESS;
        }

        var p_distance: f32;
        var n_distance: f32;
        if is_horizontal {
            p_distance = puv.x - uv.x;
            n_distance = uv.x - nuv.x;
        } else {
            p_distance = puv.y - uv.y;
            n_distance = uv.y - nuv.y;
        }
        var shortest_distance: f32;
        var delta_sign: bool;
        if p_distance <= n_distance {
            shortest_distance = p_distance;
            delta_sign = p_delta >= 0.0;
        } else {
            shortest_distance = n_distance;
            delta_sign = n_delta >= 0.0;
        }
        var edge_blend = 0.0;
        if delta_sign != (m - edge_luminance >= 0.0) {
            edge_blend = 0.5 - shortest_distance / (p_distance + n_distance);
        }

        let final_blend = max(pixel_blend, edge_blend);
        if is_horizontal {
            final_uv.y += pixel_step * final_blend;
        } else {
            final_uv.x += pixel_step * final_blend;
        }
    }
    return fxaa_sample(frame, bilinear, final_uv);
}
