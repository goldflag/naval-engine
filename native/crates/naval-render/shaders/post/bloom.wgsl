// Bloom on HDR highlights ahead of tone mapping. Ports three r185's BloomNode
// (examples/jsm/tsl/display/BloomNode.js: the luminosity high pass, five mips of
// separable Gaussian blur with kernel radii 6, 10, 14, 18 and 22 at half resolution
// and down, and the composite with `lerpBloomFactor`) with the game's settings from
// the web game's `DisplayTransform.ts` (`BLOOM`: strength 0.3, radius 0.5, threshold 2,
// knee 1). The input is the scene graded with the display look (`displayGrade`), as
// the web's `bloom(vec4(displayGrade(scene.rgb, exposure), 1), ...)`.
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::display::{display_grade, LOOK_EXPOSURE}
#import naval::color::luminance

struct BloomParams {
    // The blur's step direction (1, 0) or (0, 1), and 1 / the target's size.
    direction: vec2<f32>,
    inv_size: vec2<f32>,
    kernel_radius: f32,
    threshold: f32,
    knee: f32,
    strength: f32,
    // The composite's weight per mip, `lerpBloomFactor(factor, radius)`.
    factors: vec4<f32>,
    factor_last: f32,
    // The scene's exposure ahead of the look (the eye's is read from `eye`).
    exposure: f32,
    // The flashes on the screen (`BloomGlare`): x the view's width over its height, y how many.
    glare_view: vec2<f32>,
    // Each: xy its place (UV), z its reach as a share of the view's height, w how hot (0 to 1).
    glares: array<vec4<f32>, 8>,
    // The sun's glare in the eye (`naval_render::sun_glare`): xyz the sun's direction in the camera's frame (looking
    // down -Z), w its drawn disc's angular radius (radians).
    sun: vec4<f32>,
    // The light of its veil (the sun's irradiance, the share of the eye's veil shown and the fade past the frame's
    // edge), and 1 with a sun.
    sun_light: vec4<f32>,
    // A pixel's ray: the tangent of half the view across (x) and up (y).
    sun_view: vec4<f32>,
    // x the most a pixel brings to the bloom after dark (the graded luminance, `NIGHT_BLOOM_LIMIT`), 0 for no limit.
    limit: vec4<f32>,
}

// At the heart of a flash the bloom is this many times stronger again, and flame-coloured light blooms from this
// share of the threshold: a muzzle flash, a burst and their fireballs glare, and the sea's glitter away from them
// blooms as it did.
const GLARE_GAIN: f32 = 4.0;
const GLARE_THRESHOLD: f32 = 0.3;

// How hot a flash stands over this place in the view, 0 to 1.
fn glare_heat(uv: vec2<f32>) -> f32 {
    var heat = 0.0;
    let count = i32(params.glare_view.y);
    for (var i = 0; i < count; i++) {
        let glare = params.glares[i];
        let off = (uv - glare.xy) * vec2<f32>(params.glare_view.x, 1.0);
        heat += glare.w * exp(-dot(off, off) / max(glare.z * glare.z, 1e-6));
    }
    return saturate(heat);
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_clamp: sampler;
@group(0) @binding(2) var<uniform> params: BloomParams;
// The eye adaptation's state: stops open in [0].
@group(0) @binding(3) var<storage, read> eye: array<f32, 4>;

// The composite reads the five vertical blurs.
@group(0) @binding(4) var level_a: texture_2d<f32>;
@group(0) @binding(5) var level_b: texture_2d<f32>;
@group(0) @binding(6) var level_c: texture_2d<f32>;
@group(0) @binding(7) var level_d: texture_2d<f32>;
@group(0) @binding(8) var level_e: texture_2d<f32>;
// What the frame shows of the sun's disc, 0 to 1 in [0] (`sun_glare.wgsl`).
@group(0) @binding(9) var<storage, read> sun_seen: array<f32, 4>;

// The CIE's disability-glare function for a young eye, per unit of the light reaching it: 10 / a^3 + 5 / a^2 with the
// angle from the source in degrees, here per radian (`CORE` and `TAIL` in sun_glare.rs).
const SUN_CORE: f32 = 5.3165e-5;
const SUN_TAIL: f32 = 1.5231e-3;

// The veil the eye's own scatter lays about the sun, per unit of its irradiance, `angle` radians from its centre: flat
// across the disc the sky draws.
fn sun_veil(angle: f32) -> f32 {
    let a = max(angle, params.sun.w);
    return SUN_CORE / (a * a * a) + SUN_TAIL / (a * a);
}

// The sun's glare along the pixel's ray, graded as the bloom is added: by the exposure, without the look's contrast.
fn sun_glare(uv: vec2<f32>) -> vec3<f32> {
    if params.sun_light.w < 0.5 {
        return vec3<f32>(0.0);
    }
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let ray = normalize(vec3<f32>(ndc * params.sun_view.xy, -1.0));
    // The chord between two unit vectors: the angle between them, near enough, out to a right angle.
    let angle = distance(ray, params.sun.xyz);
    let exposure = params.exposure * exp2(eye[0]) * LOOK_EXPOSURE;
    return params.sun_light.rgb * (sun_veil(angle) * sun_seen[0] * exposure);
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// `luminosityHighPass`: the graded scene faded in over the knee above the threshold.
@fragment
fn high_pass(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let exposure = params.exposure * exp2(eye[0]);
    // Held under a half float's top: glows added over one another may have run past it.
    let radiance = min(textureSampleLevel(source, linear_clamp, in.uv, 0.0).rgb, vec3<f32>(65504.0));
    let graded = display_grade(radiance, exposure);
    // About a flash: flame's colour (far redder than blue) blooms from a lower threshold, and all of it stronger.
    let heat = glare_heat(in.uv);
    let flame = smoothstep(0.45, 0.75, (graded.r - graded.b) / max(graded.r, 1e-4));
    let lowered = mix(1.0, GLARE_THRESHOLD, heat * flame);
    let threshold = params.threshold * lowered;
    let bright = luminance(graded);
    let alpha = smoothstep(threshold, threshold + params.knee * lowered, bright);
    // After dark a flash's heart and what it lights bring the bloom no more than the limit: no white veil over the view.
    let held = select(1.0, min(1.0, params.limit.x / max(bright, 1e-6)), params.limit.x > 0.0);
    return mix(vec4<f32>(0.0), vec4<f32>(graded * held, 1.0), alpha) * vec4<f32>(vec3<f32>(1.0 + GLARE_GAIN * heat), 1.0);
}

// three's separable blur: Gaussian coefficients 0.39894 exp(-i^2 / 2 sigma^2) / sigma for
// i < kernel radius, sigma = radius / 3, unnormalised, as three builds them.
fn gaussian(i: f32, sigma: f32) -> f32 {
    return 0.39894 * exp(-0.5 * i * i / (sigma * sigma)) / sigma;
}

@fragment
fn blur(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let sigma = params.kernel_radius / 3.0;
    var sum = textureSampleLevel(source, linear_clamp, in.uv, 0.0).rgb * gaussian(0.0, sigma);
    let radius = i32(params.kernel_radius);
    for (var i = 1; i < radius; i++) {
        let x = f32(i);
        let offset = params.direction * params.inv_size * x;
        let a = textureSampleLevel(source, linear_clamp, in.uv + offset, 0.0).rgb;
        let b = textureSampleLevel(source, linear_clamp, in.uv - offset, 0.0).rgb;
        sum += (a + b) * gaussian(x, sigma);
    }
    return vec4<f32>(sum, 1.0);
}

@fragment
fn composite(in: FullscreenVertex) -> @location(0) vec4<f32> {
    var sum = textureSampleLevel(level_a, linear_clamp, in.uv, 0.0) * params.factors.x;
    sum += textureSampleLevel(level_b, linear_clamp, in.uv, 0.0) * params.factors.y;
    sum += textureSampleLevel(level_c, linear_clamp, in.uv, 0.0) * params.factors.z;
    sum += textureSampleLevel(level_d, linear_clamp, in.uv, 0.0) * params.factors.w;
    sum += textureSampleLevel(level_e, linear_clamp, in.uv, 0.0) * params.factor_last;
    return sum * params.strength + vec4<f32>(sun_glare(in.uv), 0.0);
}
