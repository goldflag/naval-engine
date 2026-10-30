// Temporal anti-aliasing's resolve. Ports `TemporalResolveNode` of
// the web game's `TemporalAntialiasing.ts` (after three's TRAANode): the display frame (graded,
// tone mapped and sRGB-encoded, as the web resolves it) blended into a reprojected
// history. Every pixel's camera motion comes from the nearest depth of its 3x3
// neighbourhood; moving surfaces add their own motion from the motion pass; history is
// read with a five-tap Catmull-Rom filter and clipped to the neighbourhood's variance;
// a surface that was not where this one reprojects to (disocclusion, by the view depth
// the history keeps in alpha), or that asks for no history (response 1: the sea,
// tracers), takes the current frame alone, drawn back from the jitter (resampled where the
// pixel would be without it), so it holds still as an unjittered frame does instead of
// dancing by a sub-pixel. Alpha out is view depth, negative where the pixel took no history.
// An HDR frame's highlights are clipped and weighed in the perceptual domain
// (`naval::output::perceptual`, the identity below SDR white) and written back out of it.
//
// UPSCALE (the scene drawn smaller than the output, `crate::taa`'s temporal upscaling): the resolve
// runs at the output's size against a history at the output's size. Each output pixel reads the
// scene about the scene pixel whose jittered sample fell nearest its centre, weighs the samples by
// their distance in output pixels, and takes the current frame by how near that nearest sample
// fell (`confidence_scale` keeps the mean the full-scale resolve's), so the history gathers the
// output's detail from the samples the jitter lands on it.
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::color::luminance
#import naval::output::{perceptual, perceptual_inverse}

struct TaaParams {
    // This frame's and the previous frame's unjittered view-projections, for a point given less this frame's eye
    // (the previous moved by the eye's step): reprojecting about the eye keeps a close body's history still far from
    // the world's origin, where world positions step by millimetres.
    current: mat4x4<f32>,
    previous: mat4x4<f32>,
    // The jittered projection's inverse, which the depth was drawn with, and the view's turn back to the world (no
    // translation: to a point less the eye).
    projection_inverse: mat4x4<f32>,
    camera_world: mat4x4<f32>,
    inverse_size: vec2<f32>,
    // 0 restarts: no history this frame.
    history_weight: f32,
    minimum_weight: f32,
    max_velocity_px: f32,
    depth_tolerance: f32,
    // This frame's sub-pixel offset (pixels): the image sits this far left and up.
    jitter: vec2<f32>,
    // The output's texel size: the history's (the scene's at full render scale).
    output_inverse_size: vec2<f32>,
    // UPSCALE: 1 / the nearest sample's mean weight, and output pixels per scene pixel.
    confidence_scale: f32,
    upscale: f32,
}

@group(0) @binding(0) var beauty: texture_2d<f32>;
@group(0) @binding(1) var scene_depth: texture_depth_2d;
@group(0) @binding(2) var motion: texture_2d<f32>;
@group(0) @binding(3) var history: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;
@group(0) @binding(5) var<uniform> params: TaaParams;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

fn size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(beauty));
}

fn clamp_texel(texel: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(clamp(texel, vec2<f32>(0.0), size() - 1.0));
}

// The history's (the output's) size, and a texel of it clamped to it.
fn history_size() -> vec2<f32> {
    return vec2<f32>(textureDimensions(history));
}

fn clamp_history(texel: vec2<f32>) -> vec2<i32> {
    return vec2<i32>(clamp(texel, vec2<f32>(0.0), history_size() - 1.0));
}

// The texel with the nearest depth about `texel`, and that depth: edges take the foreground's
// motion. Reversed depth: the nearest is the largest.
struct Nearest {
    texel: vec2<f32>,
    depth: f32,
}

#ifdef UPSCALE
// Upscaling: over the plus about the texel (the 3x3's diagonals add little at the output's
// finer pixels, and their reads cost a sixth of the resolve).
fn nearest(texel: vec2<f32>) -> Nearest {
    var closest = 2.0;
    var found = Nearest(texel, 0.0);
    let plus = array<vec2<f32>, 5>(
        vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
    );
    for (var i = 0; i < 5; i++) {
        let neighbour = texel + plus[i];
        let depth = textureLoad(scene_depth, clamp_texel(neighbour), 0);
        let value = 1.0 - depth;
        if value < closest {
            closest = value;
            found = Nearest(neighbour, depth);
        }
    }
    return found;
}
#else
// Over the 3x3 neighbourhood.
fn nearest(texel: vec2<f32>) -> Nearest {
    var closest = 2.0;
    var found = Nearest(texel, 0.0);
    for (var x = -1; x <= 1; x++) {
        for (var y = -1; y <= 1; y++) {
            let neighbour = texel + vec2<f32>(f32(x), f32(y));
            let depth = textureLoad(scene_depth, clamp_texel(neighbour), 0);
            let value = 1.0 - depth;
            if value < closest {
                closest = value;
                found = Nearest(neighbour, depth);
            }
        }
    }
    return found;
}
#endif

// three's getViewPosition for WebGPU coordinates, with uv's origin at the top left.
fn view_position(uv: vec2<f32>, depth: f32) -> vec3<f32> {
    let clip = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth, 1.0);
    let view = params.projection_inverse * clip;
    return view.xyz / view.w;
}

// Clip history toward the neighbourhood's colour box (Playdead's AABB clip).
fn clip_aabb(history_color: vec3<f32>, low: vec3<f32>, high: vec3<f32>) -> vec3<f32> {
    let centre = (high + low) * 0.5;
    let extent = (high - low) * 0.5 + 1e-5;
    let offset = history_color - centre;
    let unit = abs(offset / extent);
    let largest = max(unit.x, max(unit.y, unit.z));
    return select(history_color, centre + offset / largest, largest > 1.0);
}

// The current frame's 3x3 neighbourhood: its mean and spread, for variance clipping, and the
// pixel's own colour reconstructed about where its centre falls without the jitter (Karis 2014: a
// Blackman-Harris window over the nine, exp(-2.29 d^2)). One jittered sample swings an edge pixel
// from one side's colour to the other's as the jitter steps, and the history took 6 % of each
// swing, so edges fizzed by 15 levels from frame to frame; the window's estimate barely moves.
struct Neighbourhood {
    current: vec3<f32>,
    mean: vec3<f32>,
    deviation: vec3<f32>,
    // UPSCALE: the nearest sample's weight, 1 where it fell on the output pixel's centre.
    confidence: f32,
}

// `centre`: UPSCALE, where the output pixel's centre lies from `texel`'s, in scene pixels without the jitter.
fn neighbourhood(texel: vec2<f32>, centre: vec2<f32>) -> Neighbourhood {
    var moment_a = vec3<f32>(0.0);
    var moment_b = vec3<f32>(0.0);
    var filtered = vec3<f32>(0.0);
    var total = 0.0;
    var nearest_weight = 0.0;
#ifdef UPSCALE
    // Upscaling: the four samples about the output pixel's centre (the nearest and its neighbours toward the centre)
    // and the plus's other two, six reads where the 3x3 takes nine (a fifth of the resolve); the three left out lie
    // over 1.1 scene pixels from the centre (1.6 output pixels at 70 %), where the window gives them almost nothing.
    let toward = select(vec2<f32>(-1.0), vec2<f32>(1.0), centre - params.jitter >= vec2<f32>(0.0));
    let offsets = array<vec2<f32>, 6>(
        vec2<f32>(0.0), vec2<f32>(toward.x, 0.0), vec2<f32>(0.0, toward.y), toward,
        vec2<f32>(-toward.x, 0.0), vec2<f32>(0.0, -toward.y),
    );
    let count = 6.0;
    for (var i = 0; i < 6; i++) {
        let offset = offsets[i];
        {
#else
    let count = 9.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y));
#endif
            let colour = perceptual(max(textureLoad(beauty, clamp_texel(texel + offset), 0).rgb, vec3<f32>(0.0)));
            moment_a += colour;
            moment_b += colour * colour;
#ifdef UPSCALE
            // The texel `offset` away shows the scene `offset + jitter` from `texel`'s centre: so far from the output
            // pixel's, in output pixels.
            let d = (offset + params.jitter - centre) * params.upscale;
#else
            // The texel `offset` away shows the scene `offset + jitter` from this pixel's centre.
            let d = offset + params.jitter;
#endif
            let w = exp(-2.29 * dot(d, d));
            filtered += colour * w;
            total += w;
            nearest_weight = max(nearest_weight, w);
        }
    }
    var out: Neighbourhood;
    out.current = filtered / total;
    out.mean = moment_a / count;
    out.deviation = sqrt(max(moment_b / count - out.mean * out.mean, vec3<f32>(0.0)));
    out.confidence = nearest_weight;
    return out;
}

// Five bilinear taps of a Catmull-Rom filter: resampling history every frame with a plain
// bilinear tap blurs it within a few frames of camera motion.
fn catmull_rom(source: texture_2d<f32>, coord: vec2<f32>, texture_size: vec2<f32>) -> vec3<f32> {
    let position = coord * texture_size;
    let centre = floor(position - 0.5) + 0.5;
    let f = position - centre;
    let w0 = f * (f * (f * -0.5 + 1.0) - 0.5);
    let w1 = f * f * (f * 1.5 - 2.5) + 1.0;
    let w2 = f * (f * (f * -1.5 + 2.0) + 0.5);
    let w3 = f * f * (f * 0.5 - 0.5);
    let w12 = w1 + w2;
    let t0 = (centre - 1.0) / texture_size;
    let t3 = (centre + 2.0) / texture_size;
    let t12 = (centre + w2 / w12) / texture_size;
    let a = w12.x * w0.y;
    let b = w0.x * w12.y;
    let c = w12.x * w12.y;
    let d = w3.x * w12.y;
    let e = w12.x * w3.y;
    let sum = textureSampleLevel(source, linear_clamp, vec2<f32>(t12.x, t0.y), 0.0).rgb * a
        + textureSampleLevel(source, linear_clamp, vec2<f32>(t0.x, t12.y), 0.0).rgb * b
        + textureSampleLevel(source, linear_clamp, vec2<f32>(t12.x, t12.y), 0.0).rgb * c
        + textureSampleLevel(source, linear_clamp, vec2<f32>(t3.x, t12.y), 0.0).rgb * d
        + textureSampleLevel(source, linear_clamp, vec2<f32>(t12.x, t3.y), 0.0).rgb * e;
    return max(sum / (a + b + c + d + e), vec3<f32>(0.0));
}

@fragment
fn resolve(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let coord = in.uv;
    let texture_size = size();
#ifdef UPSCALE
    // The output pixel's centre in the scene's pixels without the jitter, and the scene pixel whose sample fell
    // nearest it (the texel at k shows the scene at k + jitter).
    let centre = coord * texture_size;
    let texel = floor(centre - params.jitter) + 0.5;
#else
    let texel = coord * texture_size;
#endif
    let closest = nearest(texel);
    let closest_texel = closest.texel;
    // Camera motion of the nearest surface, from its depth in this (jittered) frame.
    let closest_uv = closest_texel * params.inverse_size;
    let view = view_position(closest_uv, closest.depth);
    let world = params.camera_world * vec4<f32>(view, 1.0);
    let now = params.current * world;
    let before = params.previous * world;
    let surface = textureLoad(motion, clamp_texel(closest_texel), 0);
    let velocity = now.xy / now.w - before.xy / before.w + surface.xy;
    // NDC y runs up, uv down.
    let history_uv = coord - velocity * vec2<f32>(0.5, -0.5);
    let inside = all(history_uv >= vec2<f32>(0.0)) && all(history_uv <= vec2<f32>(1.0));
    // A pixel takes the response of the nearest surface about it and its own (rain over a hull adds
    // its own); but one in the ring just outside a nearer surface (the sea around a hull) takes that
    // surface's alone, as it takes its motion: the sea's refusal there left the hull's outline to the
    // jitter, a frame at a time.
    let own_depth = textureLoad(scene_depth, clamp_texel(texel), 0);
    let ring = closest.depth > own_depth * (1.0 + params.depth_tolerance);
    let own_response = select(textureLoad(motion, clamp_texel(texel), 0).z, 0.0, ring);
    let response = saturate(max(surface.z, own_response));
    let rejected = response > 0.99;
    // The history keeps each pixel's nearest view depth in alpha: a surface that was not where
    // this one now reprojects to takes no history.
    let previous_depth = abs(textureLoad(history, clamp_history(history_uv * history_size()), 0).a);
    let expected_depth = before.w;
    let disoccluded = abs(previous_depth - expected_depth) > expected_depth * params.depth_tolerance;
    let valid = inside && !rejected && !disoccluded && params.history_weight > 0.5;

    if !valid {
        // No history: the current frame where the pixel's centre falls without the jitter.
        let unjittered = catmull_rom(beauty, coord - params.jitter * params.inverse_size, texture_size);
        return vec4<f32>(unjittered, view.z);
    }
#ifdef UPSCALE
    let around = neighbourhood(texel, centre - texel);
#else
    let around = neighbourhood(texel, vec2<f32>(0.0));
#endif
    let current = around.current;
    let output_size = history_size();
    let history_color = perceptual(catmull_rom(history, history_uv, output_size));
    // Pixels of the output (the scene's at full scale) the surface moved.
    let motion_factor = saturate(length(velocity * 0.5 * output_size) / params.max_velocity_px);
#ifdef UPSCALE
    // The current frame accumulates by how near its nearest sample fell to this output pixel's centre: a sample on it
    // twice or so the mean, one in a corner a tenth; on average what the full-scale resolve takes. Motion and a
    // surface's response still take the current frame as at full scale.
    let accumulation = params.minimum_weight * around.confidence * params.confidence_scale;
#else
    let accumulation = params.minimum_weight;
#endif
    let weight = saturate(max(accumulation + motion_factor, response));
    let gamma = mix(0.75, 1.25, (1.0 - motion_factor) * (1.0 - motion_factor));
    let spread = around.deviation * gamma;
    let clipped = clip_aabb(history_color, around.mean - spread, around.mean + spread);
    // Luminance weighting keeps bright specks from flickering or smearing.
    let a = weight / (luminance(current) + 1.0);
    let b = (1.0 - weight) / (luminance(clipped) + 1.0);
    let color = (current * a + clipped * b) / max(a + b, 1e-5);
    return vec4<f32>(perceptual_inverse(color), -view.z);
}
