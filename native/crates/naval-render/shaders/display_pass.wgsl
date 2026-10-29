// The display pass: scene radiance, graded, with bloom, through AgX, onto the output.
// Ports the path of the web game's `DisplayTransform.ts` `DisplayTransform.build` without the
// armor overlay: renderOutput(vec4(displayGrade(radiance, exposure) + bloom, a),
// AgXToneMapping, SRGBColorSpace), with renderer.toneMappingExposure as
// `tone_mapping_exposure` (1 in Game.ts) and the exposure the scene's (`sky.exposure`)
// times the eye adaptation's, `exp2(stops)` (the web game's `EyeAdaptation.ts`). An sRGB output
// target encodes in hardware; ENCODE_SRGB applies three's sRGB transfer for one that does
// not (and for the post chain's frame, which edge smoothing works on in display space).
// An HDR output (`headroom` above 1, native only) takes AgX with its top
// raised to the headroom (`naval::display::agx_hdr_tone_mapping`), the sRGB transfer
// carried on above white. Writing an 8- or 10-bit output (DITHER_LEVELS) it dithers
// (`naval::output`).
//
// A look (native/docs/photo.md "Looks", `naval_render::photo::Look`; no TypeScript source), the game's or photo mode's,
// rides along: its diffusion and halation on the radiance, its grade on the display-referred colour. Its grain,
// vignette, dust and the like come after edge smoothing, in the film pass (`post/film.wgsl`). Photo mode adds the frame
// turned by the camera's roll and its lens. All off (look 0, no roll), the pass is the game's. A look works on SDR's
// range: with one on, an HDR frame's highlights stop at SDR white.
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::display::{display_grade, agx_tone_mapping, agx_hdr_tone_mapping, night_nearness}
#import naval::color::{srgb_oetf, srgb_eotf, luminance}
#import naval::output::dither_encoded

struct DisplayParams {
    // The scene's exposure ahead of the look (`DisplaySources.exposure` without the eye; photo mode's compensation
    // multiplies in).
    exposure: f32,
    tone_mapping_exposure: f32,
    // 1 to add the bloom texture, 0 without bloom.
    bloom: f32,
    // The look (`photo_look`'s case, 0 natural), its halation and diffusion 0-1, a Look tab preview's scale (scene
    // pixels to each pixel drawn, the whole frame drawn small; 0 for the frame itself), and photo mode's: the camera's
    // roll (radians, clockwise) and how much the turned frame is enlarged.
    look: f32,
    halation: f32,
    diffusion: f32,
    preview: f32,
    roll: f32,
    roll_scale: f32,
    // The output's brightest over SDR white: 1 for SDR, more for an HDR output.
    headroom: f32,
    // Photo mode's lens: its distortion (`k` in r_d = r (1 + k r²), r over the half-diagonal) and its lateral
    // chromatic aberration (red and blue shifted by this at the corners, over the half-diagonal).
    distortion: f32,
    chromatic: f32,
    // x: the host supplies a sky mean; yz: camera near/far planes; w: the grade's weight by the host's sun
    // (`naval_render::post::night_weight`), or −1 to follow the clear sky's luminance.
    night_readability: vec4<f32>,
    // x: the dark-adapted eye's grey by the host's sun (`naval_render::post::night_vision_weight`); yzw unused.
    night_vision: vec4<f32>,
    // The air's shimmer (`naval_render::shimmer::Shimmer`): the view's forward and the sea's curvature
    // (`Camera::curvature`); its right over the frame's half-width and the eye's height; its up over the half-height
    // and pixels a radian (0: no shimmer); the air's Cn² at 10 m, the tilt's gain through the glass (2.84 D^-1/3) and
    // its aperture D, metres; each octave's reach, (D / cell)^1/3; each octave's drift across and evolution, in cells.
    shimmer_forward: vec4<f32>,
    shimmer_right: vec4<f32>,
    shimmer_up: vec4<f32>,
    shimmer_air: vec4<f32>,
    shimmer_reach: vec4<f32>,
    shimmer_drift: vec4<f32>,
    shimmer_evolve: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: DisplayParams;
@group(0) @binding(2) var bloom: texture_2d<f32>;
@group(0) @binding(3) var linear_clamp: sampler;
// The eye adaptation's state: stops open in [0] (0 when it never ran).
@group(0) @binding(4) var<storage, read> eye: array<f32, 4>;
@group(0) @binding(5) var scene_depth: texture_depth_2d;
@group(0) @binding(6) var<storage, read> sky_mean: vec4<f32>;
// Metres under water where each pixel's view ray starts (r, positive under), over the screen; black without a sea.
@group(0) @binding(7) var underwater: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// A restrained gameplay grade at night: open the world's shadows by at most 1.25 stops and close the sky's by
// a quarter stop. The host's sun fades it in as it sets (`night_readability.w`, through civil twilight), or without
// one the clear sky over the dusk range in which the eye stops opening: never the cloud cover or the current camera.
// Multiplying existing light keeps black black and retains shading, fog and damage;
// the lift fades out above shadow luminances, so firelight, flashes, stars and the moon keep their highlights, and
// short of the sky's own light (`under_sky`), so the sea stays darker than the sky it mirrors.
// Read before photo mode's filtering so a rotated or distorted view follows the same world/sky boundary. Smoke, spray
// and fire write no depth: what they cover of a pixel (the scene's alpha, `transparent_cover.wgsl`) takes the near
// world's grade whatever lies behind, so a column crossing the horizon or a hull stays one smoke. A far ship's smoke,
// as light as the haze about it, takes little of the lift (`under_sky`): graded as the near world, it had glowed over
// the night sky beside her dark hull.
fn scene_radiance(pixel: vec2<i32>) -> vec3<f32> {
    let texel = textureLoad(scene, pixel, 0);
    // Held under a half float's top: glows added over one another may have run past it.
    let radiance = min(texel.rgb, vec3<f32>(65504.0));
    if params.night_readability.x == 0.0 {
        return radiance;
    }
    let clear = select(luminance(sky_mean.rgb), sky_mean.w, sky_mean.w > 0.0);
    if clear <= 0.0 {
        return radiance;
    }
    let night = select(1.0 - smoothstep(-5.5, -3.0, log2(clear)), params.night_readability.w,
        params.night_readability.w >= 0.0);
    if night <= 0.0 {
        return radiance;
    }
    let shadow = 1.0 - smoothstep(0.02, 0.2, luminance(radiance));
    // The lift stops short of the sky that is there (its mean light, clouds and all): full 2.25 stops under it, none
    // from the quarter stop under it the sky is closed to, so nothing lifted passes the sky. The whole lift carried a
    // night sea of half the sky's light to the horizon over the sky's, its quarter stop closed: 1.4 times it. Spread over 2 stops, the lift falls by under a stop a stop
    // (1.25 × 1.5 / 2 at a smoothstep's steepest), and it takes the lesser of this and the highlights' fade (their
    // product fell faster where a moonlit sky's met them), so a brighter pixel never comes out darker than a dimmer one.
    let sky_light = max(luminance(sky_mean.rgb), 1e-12);
    let under_sky = 1.0 - smoothstep(-2.25, -0.25, log2(max(luminance(radiance), 1e-12) / sky_light));
    // Keep the distant haze and sea joined to the sky (`night_nearness`). A depth of zero is always sky; the camera's
    // own planes recover metres for every lens and scene.
    let depth = textureLoad(scene_depth, pixel, 0);
    let near = params.night_readability.y;
    let far = params.night_readability.z;
    let distance = near * far / (depth * (far - near) + near);
    // A pixel seen through water shows the water near the eye, whatever the depth holds: the open deep has none and
    // is not sky (graded as sky, it parted at eye level from the water under the sea's surface, which has a depth).
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / vec2<f32>(textureDimensions(scene));
    let through_water = textureSampleLevel(underwater, linear_clamp, uv, 0.0).r > 0.0;
    let surface = select(select(0.0, night_nearness(distance), depth > 0.0), 1.0, through_water);
    let nearness = mix(surface, 1.0, clamp(texel.a, 0.0, 1.0));
    let stops = 1.25 * nearness * min(under_sky, shadow) - 0.25 * (1.0 - nearness) * shadow;
    return radiance * exp2(stops * night);
}

// The dark-adapted eye (2026-10-26): by the moon and the stars the rods see, and they see no colour, so what the night
// leaves dim comes out a faint cool grey (`NIGHT_GREY`, the moonlit sky and sea of a night photograph Bill gave, where the
// game's night had been a saturated blue), while light bright enough for the cones keeps its colour: fires, flashes,
// lamps, a star shell's light close by (`CONES`, log2 of the scene's luminance, over which the grey gives way to colour;
// the night's lift sets fires and flashes far over the moonlit world). The host's sun brings it in as the twilight
// falls under a full moon's light (`night_vision.x`), so dusk keeps its colour. Luminance is kept, so the night's lift,
// the highlights and the eye see the same light.
const NIGHT_GREY: vec3<f32> = vec3<f32>(0.82, 1.03, 1.2);
const CONES: vec2<f32> = vec2<f32>(-5.5, 1.0);

fn night_vision(radiance: vec3<f32>) -> vec3<f32> {
    let rods = params.night_vision.x;
    if rods <= 0.0 {
        return radiance;
    }
    let light = luminance(radiance);
    let grey = rods * (1.0 - smoothstep(CONES.x, CONES.y, log2(max(light, 1e-12))));
    return mix(radiance, light * NIGHT_GREY, grey);
}

// The scene's radiance at `p` (pixels, continuous), filtered between its four nearest pixels.
fn radiance_at(p: vec2<f32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(scene)) - vec2<i32>(1);
    let q = p - vec2<f32>(0.5);
    let base = floor(q);
    let f = q - base;
    let i = vec2<i32>(base);
    let a = scene_radiance(clamp(i, vec2<i32>(0), last));
    let b = scene_radiance(clamp(i + vec2<i32>(1, 0), vec2<i32>(0), last));
    let c = scene_radiance(clamp(i + vec2<i32>(0, 1), vec2<i32>(0), last));
    let d = scene_radiance(clamp(i + vec2<i32>(1, 1), vec2<i32>(0), last));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The pixel of the frame drawn a photo pixel `d` from the centre shows: turned by the roll and brought in by the frame
// drawn's widening (`roll_scale`, the roll's and a barrel's).
fn lens_source(d: vec2<f32>, centre: vec2<f32>) -> vec2<f32> {
    let c = cos(params.roll);
    let s = sin(params.roll);
    return centre + vec2<f32>(d.x * c - d.y * s, d.x * s + d.y * c) / max(params.roll_scale, 1.0);
}

// The undistorted radius r with r (1 + k r²) = r_d (`naval_render::photo_optics::undistort`).
fn undistort(r_d: f32, k: f32) -> f32 {
    var r = r_d;
    for (var i = 0; i < 4; i++) {
        r -= (r * (1.0 + k * r * r) - r_d) / (1.0 + 3.0 * k * r * r);
    }
    return r;
}

// The air's shimmer through a long glass (`naval_render::shimmer`, native/docs/haze.md "Shimmer"): each pixel shows the
// scene a little off where it lies, by four octaves of value noise in the view's bearing and elevation (fixed in the
// air as the glass turns, carried across by the wind), each as much as the turbulence along the pixel's own ray shares
// over its cells, and blurred by what is finer.

// The octaves' cell heights, radians (`naval_render::shimmer::OCTAVES`), and how much wider a cell is.
const SHIMMER_CELLS: vec4<f32> = vec4<f32>(1.6e-3, 6e-4, 2.5e-4, 1e-4);
const SHIMMER_CELL_ASPECT: f32 = 1.5;
// An octave fades in between these cell heights in pixels (`FADE_PX`).
const SHIMMER_FADE: vec2<f32> = vec2<f32>(2.0, 5.0);
// The noise's rms (values uniform over −1 … 1, quintic fade), measured over 400,000 points.
const SHIMMER_NOISE_RMS: f32 = 0.4;
// Across, the waver is this share of the waver up and down: the layer's eddies lie flattened on the water.
const SHIMMER_ACROSS: f32 = 0.8;
// Cn²'s reference height, the least height it is read at and the marine layer's fall, metres (`Z_REF_M`, `Z_LEAST_M`,
// `LAYER_TOP_M`); the sky's path (`SKY_PATH_M`); light's wavelength (`WAVELENGTH_M`).
const SHIMMER_Z_REF: f32 = 10.0;
const SHIMMER_Z_LEAST: f32 = 2.0;
const SHIMMER_LAYER_TOP: f32 = 600.0;
const SHIMMER_SKY: f32 = 40000.0;
const SHIMMER_WAVELENGTH: f32 = 5.5e-7;
// The samples along a ray (`naval_render::shimmer::along`), at the squares of their share of it: the share, the step
// over the ray's length, a point's weight there, (1 − share)^5/3, and the share's −1/3 power.
const SHIMMER_ALONG: array<vec4<f32>, 8> = array<vec4<f32>, 8>(
    vec4<f32>(0.00390625, 0.015625, 0.9934981, 6.349604),
    vec4<f32>(0.03515625, 0.046875, 0.9420956, 3.052571),
    vec4<f32>(0.09765625, 0.078125, 0.8425972, 2.171534),
    vec4<f32>(0.19140625, 0.109375, 0.7018064, 1.735194),
    vec4<f32>(0.31640625, 0.140625, 0.5304732, 1.467523),
    vec4<f32>(0.47265625, 0.171875, 0.3442096, 1.283763),
    vec4<f32>(0.66015625, 0.203125, 0.1655003, 1.148465),
    vec4<f32>(0.87890625, 0.234375, 0.02963941, 1.043965),
);
// Under this blur (a Gaussian's sigma, pixels) the pass draws none (`LEAST_BLUR_PX`).
const SHIMMER_LEAST_BLUR: f32 = 0.5;
// The blur's taps: two rings of four (the outer turned by half a step), their radii over the blur's sigma and each
// tap's weight; the centre takes the rest, 0.3. A Gaussian's spread on each axis to 1 %.
const SHIMMER_BLUR_RINGS: vec2<f32> = vec2<f32>(1.2, 2.4);
const SHIMMER_BLUR_WEIGHTS: vec2<f32> = vec2<f32>(0.12, 0.055);
// The farthest a blur tap reaches, pixels.
const SHIMMER_BLUR_MOST: f32 = 8.0;

// A lattice corner's hash from its coordinates' own (each multiplied by its odd constant, xored): one round of mixing.
fn shimmer_mix(h: u32) -> u32 {
    var x = h ^ (h >> 16u);
    x *= 0x7feb352du;
    return x ^ (x >> 15u);
}

// A corner's two values, −1 … 1, from its hash's low and high halves.
fn shimmer_values(h: u32) -> vec2<f32> {
    return vec2<f32>(vec2<u32>(h & 0xffffu, h >> 16u)) / 32767.5 - 1.0;
}

// Two value noises at `p` (one for each axis of the offset, uncorrelated), each of rms SHIMMER_NOISE_RMS, repeating over
// `wrap` cells in x (the bearing's full turn; `p.x` within 0 … wrap). Value noise rather than gradient noise: one hash
// and no gradients a corner, a third of the cost, and at a few pixels' waver the two look alike.
fn shimmer_noise(p: vec3<f32>, wrap: i32) -> vec2<f32> {
    let base = floor(p);
    let f = p - base;
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let i = vec3<i32>(base);
    let x1 = select(i.x + 1, 0, i.x + 1 >= wrap);
    let hx = vec2<u32>(bitcast<u32>(i.x), bitcast<u32>(x1)) * 0x8da6b343u;
    let hy = vec2<u32>(bitcast<u32>(i.y), bitcast<u32>(i.y + 1)) * 0xd8163841u;
    let hz = vec2<u32>(bitcast<u32>(i.z), bitcast<u32>(i.z + 1)) * 0xcb1ab31fu;
    let n000 = shimmer_values(shimmer_mix(hx.x ^ hy.x ^ hz.x));
    let n100 = shimmer_values(shimmer_mix(hx.y ^ hy.x ^ hz.x));
    let n010 = shimmer_values(shimmer_mix(hx.x ^ hy.y ^ hz.x));
    let n110 = shimmer_values(shimmer_mix(hx.y ^ hy.y ^ hz.x));
    let n001 = shimmer_values(shimmer_mix(hx.x ^ hy.x ^ hz.y));
    let n101 = shimmer_values(shimmer_mix(hx.y ^ hy.x ^ hz.y));
    let n011 = shimmer_values(shimmer_mix(hx.x ^ hy.y ^ hz.y));
    let n111 = shimmer_values(shimmer_mix(hx.y ^ hy.y ^ hz.y));
    let near = mix(mix(n000, n100, s.x), mix(n010, n110, s.x), s.y);
    let far = mix(mix(n001, n101, s.x), mix(n011, n111, s.x), s.y);
    return mix(near, far, s.z);
}

// What a ray sees of the air (`naval_render::shimmer::Tilt`): its image's wander, the variance on one axis, and of it
// what is shared over each octave's cells, rad²; the short-exposure seeing blur, radians.
struct ShimmerTilt {
    total: f32,
    held: vec4<f32>,
    seeing: f32,
}

// What a ray `distance` long sees (of a point there, or with `plane` of the sky), climbing `rise` (its sine of
// elevation) from the eye over the curved sea (`naval_render::shimmer::tilt`).
fn shimmer_tilt(distance: f32, rise: f32, plane: bool) -> ShimmerTilt {
    let curvature = params.shimmer_forward.w;
    let eye_height = params.shimmer_right.w;
    let cn2 = params.shimmer_air.x;
    // Each octave's reach over the distance's cube root: (D / cell)^1/3 x^-1/3 is this by the sample's own share.
    let reach = params.shimmer_reach * pow(distance, -1.0 / 3.0);
    var sum = 0.0;
    var held = vec4<f32>(0.0);
    for (var i = 0; i < 8; i++) {
        let along = SHIMMER_ALONG[i];
        let x = distance * along.x;
        let z = eye_height + x * rise + curvature * x * x;
        // Cn² (z / Z_REF)^-2/3 exp(−z / LAYER_TOP) in one exp.
        let strength = cn2 * exp(-2.0 / 3.0 * log(max(z, SHIMMER_Z_LEAST) / SHIMMER_Z_REF)
            - max(z, 0.0) / SHIMMER_LAYER_TOP);
        let part = strength * select(along.z, 1.0, plane) * distance * along.y;
        sum += part;
        held += part * min(vec4<f32>(1.0), reach * along.w);
    }
    var seeing = 0.0;
    if sum > 0.0 {
        let k = 6.2831853 / SHIMMER_WAVELENGTH;
        let r0 = pow(0.423 * k * k * sum, -0.6);
        let width = 0.98 * SHIMMER_WAVELENGTH / r0 * max(1.0 - 0.37 * pow(r0 / params.shimmer_air.z, 1.0 / 3.0), 0.0);
        seeing = width / 2.3548;
    }
    let gain = params.shimmer_air.y;
    return ShimmerTilt(gain * sum, gain * held, seeing);
}

// The metres from the eye to what scene pixel `p` shows along a ray `stretch` times the view's forward depth (the sky's
// path where it shows sky).
fn shimmer_distance(p: vec2<f32>, stretch: f32) -> f32 {
    let last = vec2<i32>(textureDimensions(scene_depth)) - vec2<i32>(1);
    let depth = textureLoad(scene_depth, clamp(vec2<i32>(p), vec2<i32>(0), last), 0);
    let near = params.night_readability.y;
    let far = params.night_readability.z;
    if depth <= 0.0 {
        return SHIMMER_SKY;
    }
    return min(near * far / (depth * (far - near) + near) * stretch, SHIMMER_SKY);
}

// What the air does to scene pixel `p`: the pixels it moves what the pixel shows by, the blur (a Gaussian's sigma,
// pixels), and the metres to what it shows along a ray `stretch` times the view's forward depth. All zero without a
// shimmer.
struct ShimmerAir {
    offset: vec2<f32>,
    blur: f32,
    distance: f32,
    stretch: f32,
}

fn shimmer_air(p: vec2<f32>, size: vec2<f32>) -> ShimmerAir {
    var air = ShimmerAir(vec2<f32>(0.0), 0.0, 0.0, 1.0);
    let px_per_rad = params.shimmer_up.w;
    if px_per_rad <= 0.0 {
        return air;
    }
    let ndc = vec2<f32>(p.x / size.x * 2.0 - 1.0, 1.0 - p.y / size.y * 2.0);
    let along = params.shimmer_forward.xyz + ndc.x * params.shimmer_right.xyz + ndc.y * params.shimmer_up.xyz;
    air.stretch = length(along);
    let ray = along / air.stretch;
    air.distance = shimmer_distance(p, air.stretch);
    let tilt = shimmer_tilt(air.distance, ray.y, air.distance >= SHIMMER_SKY);
    if sqrt(tilt.total) * px_per_rad < 0.02 {
        return air;
    }
    // The ray's bearing (a full turn over 0 … 1) and elevation, radians.
    let bearing = atan2(ray.x, -ray.z) / 6.2831853 + 0.5;
    let elevation = asin(clamp(ray.y, -1.0, 1.0));
    // Each octave draws the wander shared over its cells and not over the coarser's; what it fades out of blurs.
    var field = vec2<f32>(0.0);
    var faded = 0.0;
    var coarser = 0.0;
    for (var k = 0; k < 4; k++) {
        let own = max(tilt.held[k] - coarser, 0.0);
        coarser = tilt.held[k];
        let cell = SHIMMER_CELLS[k];
        let shown = smoothstep(SHIMMER_FADE.x, SHIMMER_FADE.y, cell * px_per_rad);
        faded += own * (1.0 - shown * shown);
        if shown <= 0.0 {
            continue;
        }
        let wrap = f32(max(i32(round(6.2831853 / (SHIMMER_CELL_ASPECT * cell))), 1));
        let across = bearing * wrap - params.shimmer_drift[k];
        let q = vec3<f32>(across - floor(across / wrap) * wrap, elevation / cell + f32(k) * 17.0,
            params.shimmer_evolve[k]);
        field += shimmer_noise(q, i32(wrap)) * sqrt(own) * shown;
    }
    // Bearing to the right and elevation up, onto the frame's x right and y down.
    air.offset = vec2<f32>(field.x * SHIMMER_ACROSS, -field.y) * (px_per_rad / SHIMMER_NOISE_RMS);
    air.blur = sqrt(max(tilt.total - coarser, 0.0) + faded + tilt.seeing * tilt.seeing) * px_per_rad;
    // Nothing nearer comes forward: landing on something nearer, the pixel moves only by about what that nearer thing's
    // own path gives it (the wander grows as the root of the path).
    let there = shimmer_distance(p + air.offset, air.stretch);
    if there < air.distance {
        air.offset *= sqrt(there / air.distance);
    }
    return air;
}

// `radiance`, what the scene shows at `p` where `air` moved the pixel's view, blurred by the air's finer eddies; a tap
// landing on something nearer takes the centre's.
fn shimmer_blurred(p: vec2<f32>, air: ShimmerAir, radiance: vec3<f32>) -> vec3<f32> {
    if air.blur < SHIMMER_LEAST_BLUR {
        return radiance;
    }
    let sigma = min(air.blur, SHIMMER_BLUR_MOST / SHIMMER_BLUR_RINGS.y);
    let last = vec2<i32>(textureDimensions(scene)) - vec2<i32>(1);
    var sum = radiance * (1.0 - 4.0 * (SHIMMER_BLUR_WEIGHTS.x + SHIMMER_BLUR_WEIGHTS.y));
    for (var i = 0; i < 8; i++) {
        let ring = i / 4;
        let angle = 0.4636476 + f32(i) * 1.5707963 + f32(ring) * 0.7853982;
        let tap = p + vec2<f32>(cos(angle), sin(angle)) * (SHIMMER_BLUR_RINGS[ring] * sigma);
        let nearer = shimmer_distance(tap, air.stretch) < 0.9 * air.distance;
        // A tap's own pixel: the blur smooths what bilinear filtering would.
        let there = scene_radiance(clamp(vec2<i32>(tap), vec2<i32>(0), last));
        sum += select(there, radiance, nearer) * SHIMMER_BLUR_WEIGHTS[ring];
    }
    return sum;
}

// An S-curve about the middle, `k` of the way from straight to smoothstep.
fn s_curve(x: vec3<f32>, k: f32) -> vec3<f32> {
    let c = clamp(x, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(c, c * c * (3.0 - 2.0 * c), k);
}

fn saturate_by(c: vec3<f32>, s: f32) -> vec3<f32> {
    return mix(vec3<f32>(dot(c, vec3<f32>(0.2126, 0.7152, 0.0722))), c, s);
}

// Infrared from colour: blue goes dark, and green brighter than red and blue (living leaves) glows.
fn infrared(c: vec3<f32>) -> f32 {
    let leaves = max(c.g - max(c.r, c.b), 0.0);
    return clamp((1.2 * c.r + 0.2 * c.g - 0.75 * c.b) / 0.65 + leaves * 4.0, 0.0, 1.0);
}

// A look's grade on display-referred colour (linear, 0-1), graded on its gamma-encoded values. Returns them encoded.
// Cases 6 on were tuned in the 2026-10-09 film looks study on the game's frames.
fn photo_look(linear: vec3<f32>, look: i32) -> vec3<f32> {
    var c = pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    switch look {
        // Press print: black and white, a firmer contrast, ink on warm paper.
        case 1: {
            let g = s_curve(vec3<f32>(l), 0.5).x;
            c = mix(vec3<f32>(0.06, 0.055, 0.05), vec3<f32>(0.98, 0.95, 0.87), g);
        }
        // Kodachrome: rich, saturated and warm, its reds deep and its shadows dense.
        case 2: {
            c = max(mix(vec3<f32>(l), c, 1.4), vec3<f32>(0.0));
            c = c * vec3<f32>(1.06, 1.0, 0.87);
            c = s_curve(c, 0.35);
            c.r = pow(c.r, 0.94);
        }
        // Newsreel: hard black and white, darker, the blacks crushed.
        case 3: {
            let g = clamp(l * 0.92 - 0.04, 0.0, 1.0);
            c = s_curve(vec3<f32>(g), 1.0) * vec3<f32>(0.97, 0.98, 1.0);
            c = s_curve(c, 0.4);
        }
        // Night glass: the blue-green of a low-light binocular, its colour mostly gone.
        case 4: {
            let glass = vec3<f32>(0.40, 0.80, 0.72) * pow(l, 0.85) * 1.2;
            c = mix(c * vec3<f32>(0.55, 0.85, 0.95), glass, 0.75) * 0.92 + vec3<f32>(0.0, 0.015, 0.02);
        }
        // Faded: little contrast or colour, the blacks lifted and the whites dulled.
        case 5: {
            c = mix(vec3<f32>(l), c, 0.6);
            c = c * 0.8 + vec3<f32>(0.11, 0.10, 0.09);
            c = c * vec3<f32>(1.02, 1.0, 0.95);
        }
        // Three-strip Technicolor: each record printed in its own dye, so a colour loses what the other two records
        // hold. Dense primaries, clean whites, deep blacks; greys stay grey.
        case 6: {
            c = max(saturate_by(c, 1.3), vec3<f32>(0.0));
            let m = 1.0 - (c - (c.gbr + c.brg) * 0.5);
            let t = vec3<f32>(m.g * m.b * c.r, m.r * m.b * c.g, m.r * m.g * c.b);
            c = mix(c, clamp(t, vec3<f32>(0.0), vec3<f32>(1.0)), 0.85);
            c = s_curve(c, 0.3);
            c = max(c - 0.025, vec3<f32>(0.0)) / 0.975 * vec3<f32>(1.03, 1.0, 0.96);
        }
        // Agfacolor: softer and cooler, olive shadows, reds toward brick, yellow-green highlights.
        case 7: {
            let red = max(c.r - max(c.g, c.b), 0.0);
            c = c + vec3<f32>(-0.3, 0.08, 0.0) * red;
            c = saturate_by(c, 0.8) * 0.9 + vec3<f32>(0.03, 0.055, 0.02);
            c *= vec3<f32>(0.98, 1.02, 0.9);
            c = mix(c, c * vec3<f32>(1.0, 1.02, 0.94), smoothstep(0.4, 1.0, l));
            c = s_curve(c, -0.15);
        }
        // Autochrome: pastel, dim and warm (the film pass lays the starch grains over it).
        case 8: {
            c = s_curve(saturate_by(c, 0.72), -0.2);
            c = (c * 0.82 + vec3<f32>(0.07, 0.055, 0.03)) * vec3<f32>(1.04, 1.0, 0.9);
        }
        // Bleach bypass: the silver left in the print over the colour, half the colour, cool shadows.
        case 9: {
            let shade = 2.0 * c * l;
            let light = 1.0 - 2.0 * (1.0 - l) * (1.0 - c);
            c = mix(c, mix(shade, light, clamp(10.0 * (l - 0.45), 0.0, 1.0)), 0.85);
            c = saturate_by(c, 0.5);
            c = mix(c * vec3<f32>(0.97, 1.0, 1.0), c * vec3<f32>(0.9, 0.99, 1.03), 1.0 - smoothstep(0.0, 0.5, l));
        }
        // Black-and-white infrared: the sea and the blue sky dark, clouds, wakes and leaves white.
        case 10: {
            c = s_curve(vec3<f32>(infrared(c)), 0.5) * vec3<f32>(1.0, 0.99, 0.97);
        }
        // Aerochrome (colour infrared): infrared shown red, red shown green, green shown blue. Leaves turn red, the sea
        // deep blue, red paint yellow.
        case 11: {
            c = s_curve(saturate_by(vec3<f32>(infrared(c), c.r, c.g), 1.25), 0.3);
        }
        // Gun camera: hard 16 mm black and white.
        case 12: {
            let g = clamp(l * 1.1 - 0.05, 0.0, 1.0);
            c = s_curve(vec3<f32>(g), 0.8) * vec3<f32>(0.98, 0.98, 1.0);
        }
        default: {}
    }
    return c;
}

// The halation's colour (light through the emulsion bounces off the film's base back into the red layer), how fast the
// bloom's light fills it (squared, so a fire's or a glint's bloom fills it and the faint tail of a wide bloom does not
// tint), and its most, on the display-referred frame, at a look's full halation (the study's 1.8).
const HALATION_TINT: vec3<f32> = vec3<f32>(1.0, 0.3, 0.1);
const HALATION_RESPONSE: f32 = 0.5;
const HALATION_MAX: f32 = 1.8;

// A look's diffusion (a soft lens or a diffusion filter): the frame about `p` blurred over 0.8 % of its height on a
// spiral of twelve samples, lightened in.
fn diffused(p: vec2<f32>, size: vec2<f32>, radiance: vec3<f32>) -> vec3<f32> {
    let radius = size.y * 0.008;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 12; i++) {
        let t = (f32(i) + 0.5) / 12.0;
        let angle = f32(i) * 2.39996323;
        let offset = vec2<f32>(cos(angle), sin(angle)) * radius * sqrt(t);
        sum += textureSampleLevel(scene, linear_clamp, (p + offset) / size, 0.0).rgb;
    }
    return mix(radiance, max(radiance, sum / 12.0), params.diffusion * 0.6);
}

@fragment
fn fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(scene));
    var radiance: vec3<f32>;
    var bloom_uv = in.uv;
    var source = in.position.xy;
    if (params.distortion != 0.0 || params.chromatic > 0.0) {
        // Photo mode's lens: the pixel's radius out of the distortion, then each colour's own magnification (red
        // and blue apart toward the corners), then the roll's turn as below.
        let centre = size * 0.5;
        let d = in.position.xy - centre;
        let half_diagonal = length(centre);
        let r_d = length(d) / half_diagonal;
        let r = undistort(r_d, params.distortion);
        let du = d * select(1.0, r / r_d, r_d > 1e-6);
        let spread = params.chromatic * (0.5 + 0.5 * r * r);
        let lensed = lens_source(du, centre);
        let air = shimmer_air(lensed, size);
        let green = lensed + air.offset;
        radiance = shimmer_blurred(green, air, vec3<f32>(
            radiance_at(lens_source(du * (1.0 - spread), centre) + air.offset).r,
            radiance_at(green).g,
            radiance_at(lens_source(du * (1.0 + spread), centre) + air.offset).b,
        ));
        bloom_uv = green / size;
        source = green;
    } else if (params.roll != 0.0 || params.roll_scale > 1.0) {
        // The camera turned clockwise turns the picture the other way: each pixel shows the frame drawn (a wider view,
        // `roll_scale` larger) turned about the centre.
        let centre = size * 0.5;
        let turned = lens_source(in.position.xy - centre, centre);
        let air = shimmer_air(turned, size);
        let source_px = turned + air.offset;
        radiance = shimmer_blurred(source_px, air, radiance_at(source_px));
        bloom_uv = source_px / size;
        source = source_px;
    } else if params.preview > 0.0 {
        // A Look tab preview (`naval_render::look_preview`): the whole frame drawn small, each pixel the mean of four
        // filtered samples over the scene pixels it covers.
        let p = in.position.xy * params.preview;
        let q = params.preview * 0.25;
        radiance = (radiance_at(p + vec2<f32>(-q, -q)) + radiance_at(p + vec2<f32>(q, -q))
            + radiance_at(p + vec2<f32>(-q, q)) + radiance_at(p + vec2<f32>(q, q))) * 0.25;
        source = p;
    } else {
        let air = shimmer_air(in.position.xy, size);
        if any(air.offset != vec2<f32>(0.0)) || air.blur >= SHIMMER_LEAST_BLUR {
            source = in.position.xy + air.offset;
            radiance = shimmer_blurred(source, air, radiance_at(source));
            bloom_uv = source / size;
        } else {
            radiance = scene_radiance(vec2<i32>(in.position.xy));
        }
    }
    if params.diffusion > 0.0 {
        radiance = diffused(source, size, radiance);
    }
    radiance = night_vision(radiance);
    var graded = display_grade(radiance, params.exposure * exp2(eye[0]));
    let bloom_light = textureSampleLevel(bloom, linear_clamp, bloom_uv, 0.0).rgb;
    graded += bloom_light * params.bloom;
    var color: vec3<f32>;
    if params.headroom > 1.0 {
        color = agx_hdr_tone_mapping(graded, params.tone_mapping_exposure, params.headroom);
    } else {
        color = agx_tone_mapping(graded, params.tone_mapping_exposure);
    }
    if params.halation > 0.0 {
        // A look's halation: the bloom's blur of the brightest lights (none without bloom), bounded as a print's
        // highlights are, tinted.
        let halo = 1.0 - exp(-bloom_light * HALATION_RESPONSE);
        color += halo * halo * HALATION_TINT * params.halation * HALATION_MAX;
    }
    let look = i32(params.look + 0.5);
    if look > 0 {
        color = pow(clamp(photo_look(color, look), vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(2.2));
    }
#ifdef DITHER_LEVELS
    // Onto 8 or 10 bits: dithered in the encoded domain, decoded again where the target
    // encodes in hardware (its encoding then undoes the decoding).
    color = dither_encoded(srgb_oetf(color), in.position.xy, f32(#{DITHER_LEVELS}));
#ifndef ENCODE_SRGB
    color = srgb_eotf(color);
#endif
#else
#ifdef ENCODE_SRGB
    color = srgb_oetf(color);
#endif
#endif
    return vec4<f32>(color, 1.0);
}
