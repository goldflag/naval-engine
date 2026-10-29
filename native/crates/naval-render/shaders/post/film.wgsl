// The film pass (naval_render::film, native/docs/photo.md "Looks"): what a look's stock and projector add to the
// finished frame, drawn last at the output's resolution so temporal AA's history never holds it. In order: the frame
// shaking in the gate, an autochrome's dyed starch grains, the projector's flicker, a vignette, dust and scratches, and
// grain. The frame is display-encoded; onto an 8- or 10-bit output it is dithered (DITHER_LEVELS), onto an sRGB target
// decoded after (DECODE_SRGB), as the chain's other last stages (`post/output.wgsl`).
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::encode::encode

struct FilmParams {
    // Grain, vignette, the gate's weave and the projector's flicker, 0-1.
    a: vec4<f32>,
    // Dust and scratches and the autochrome's mosaic (0-1), the film's frame (24 a second in play, photo mode's seed),
    // padding.
    b: vec4<f32>,
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
@group(0) @binding(2) var<uniform> film: FilmParams;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

fn hash(p: vec2<f32>) -> f32 {
    let q = fract(p * vec2<f32>(0.1031, 0.1030));
    let r = q + dot(q, q.yx + 33.33);
    return fract((r.x + r.y) * r.x);
}

fn hash1(n: f32) -> f32 {
    return hash(vec2<f32>(n, n * 0.618 + 17.0));
}

// Smooth noise over the film's frames, -1 to 1.
fn wander(t: f32) -> f32 {
    let i = floor(t);
    let f = t - i;
    return mix(hash1(i), hash1(i + 1.0), f * f * (3.0 - 2.0 * f)) * 2.0 - 1.0;
}

@fragment
fn fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(frame));
    let pixel = in.position.xy;
    let grain = film.a.x;
    let vignette = film.a.y;
    let weave = film.a.z;
    let flicker = film.a.w;
    let dust = film.b.x;
    let mosaic = film.b.y;
    let n = film.b.z;

    var c: vec3<f32>;
    if weave > 0.0 {
        // The frame shakes in the gate: a slow wander and a jolt each frame, the picture enlarged a little so its
        // edge never shows.
        let shake = weave * vec2<f32>(
            wander(n * 0.31) * 0.0014 + (hash1(n) - 0.5) * 0.0010,
            wander(n * 0.23 + 40.0) * 0.0020 + (hash1(n + 7.0) - 0.5) * 0.0008,
        );
        let uv = (in.uv - 0.5) / (1.0 + 0.014 * weave) + 0.5 + shake;
        c = textureSampleLevel(frame, bilinear, uv, 0.0).rgb;
    } else {
        c = textureLoad(frame, vec2<i32>(pixel), 0).rgb;
    }

    if mosaic > 0.0 {
        // An autochrome's grains of potato starch, dyed orange, green and violet, in clumps.
        let grain_px = max(1.0, size.y / 560.0);
        let r = fract(hash(floor(pixel / grain_px)) * 0.6 + hash(floor(pixel / (grain_px * 3.0)) + 11.0) * 0.7);
        var dye = vec3<f32>(0.88, 0.84, 1.3);
        if r < 0.333 {
            dye = vec3<f32>(1.25, 0.84, 0.78);
        } else if r < 0.666 {
            dye = vec3<f32>(0.82, 1.18, 0.84);
        }
        c *= mix(vec3<f32>(1.0), dye, 0.3 * mosaic);
    }
    if flicker > 0.0 {
        c *= 1.0 + flicker * 0.10 * (hash1(n * 1.7 + 3.0) * 2.0 - 1.0);
    }
    if vignette > 0.0 {
        // Up to 90 % darker at the corners, from a fifth of the way out.
        let aspect = size.x / size.y;
        let r = length((in.uv - 0.5) * vec2<f32>(aspect, 1.0)) / length(vec2<f32>(aspect, 1.0) * 0.5);
        c *= 1.0 - vignette * 0.9 * smoothstep(0.2, 1.05, r);
    }
    if dust > 0.0 {
        let s = max(1.0, size.y / 900.0);
        // Scratches down the print: up to three, each staying for ten frames and wandering a little.
        let run = floor(n / 10.0);
        var scratch = 0.0;
        for (var i = 0; i < 3; i++) {
            let fi = f32(i);
            let on = step(hash1(run * 3.1 + fi * 17.0), dust * 0.55) * step(0.15, hash1(n * 1.3 + fi * 5.0));
            let x = hash1(run * 5.3 + fi * 9.1) + (hash1(n * 2.1 + fi) - 0.5) * 0.003;
            let d = abs(in.uv.x - x) * size.x / s;
            scratch += on * (1.0 - smoothstep(0.3, 1.3, d)) * (0.55 + 0.45 * sin(in.uv.y * 31.0 + fi * 4.0 + run));
        }
        c = mix(c, vec3<f32>(0.92), clamp(scratch, 0.0, 1.0) * 0.55);
        // Specks of dust, a frame each, mostly dark.
        let cell = vec2<f32>(52.0 * s);
        let id = floor(pixel / cell);
        if hash(id + n * 13.17) < dust * 0.03 {
            let centre = (id + vec2<f32>(hash(id + n * 1.7 + 3.0), hash(id + n * 2.3 + 9.0))) * cell;
            let radius = (0.7 + 2.4 * hash(id + n * 3.1 + 5.0)) * s;
            let d = (pixel - centre) * vec2<f32>(1.0, 0.6 + 0.8 * hash(id + n));
            let speck = 1.0 - smoothstep(radius * 0.5, radius, length(d));
            let tone = select(vec3<f32>(0.95), vec3<f32>(0.03), hash(id + n * 7.0) < 0.75);
            c = mix(c, tone, speck * 0.85);
        }
    }
    if grain > 0.0 {
        // Monochrome, strongest in the mid-tones, a pixel of a 1080-line frame across, its pattern the frame's.
        let cell = floor(pixel / max(1.0, size.y / 1080.0));
        let seed = vec2<f32>(n * 17.0, n * 31.0);
        let g = hash(cell + seed) + hash(cell * 1.37 + seed + 19.0) - 1.0;
        let l = dot(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(0.2126, 0.7152, 0.0722));
        c += vec3<f32>(g * grain * 0.16 * (0.3 + 2.8 * l * (1.0 - l)));
    }
    return encode(max(c, vec3<f32>(0.0)), pixel);
}
