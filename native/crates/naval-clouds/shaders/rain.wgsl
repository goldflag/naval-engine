// Near-camera rain as a camera records it (native/docs/clouds/weather.md, "Rain as a camera records it", 2026-10-20). The drops
// live in nested boxes that wrap about the camera, as the web game's `sky/weather/rain.ts` (`RainField.build`) laid
// them; how each is drawn is new:
//
// - The drops drawn are the rain's own large drops, Marshall–Palmer's exponential sizes from `drops.y` millimetres up at
//   the weather's rain rate, each falling at its terminal speed (Atlas, Srivastava & Sekhon 1973) and carried with the
//   wind, so they share one slant.
// - Each is the streak its motion relative to the camera paints over one exposure, a sixtieth of a second, as faint as
//   the time it spends on a pixel makes it: the drop's area spread along its streak (a 3 mm drop 3 m off covers a
//   pixel it crosses for a hundredth of the exposure). A camera records thousands of them a few metres off; the tier
//   draws a few per cent, each standing for the rest about it (`weight`, `weather::drop_weight`), so together they
//   take the rain's share of the light, more coarsely grained. Farther, they thin with depth into the haze's rain
//   (`aerial.wgsl`).
// - A lens focused on the scene blurs the nearest drops (`drops.w` its aperture), so they draw wide and fainter still;
//   farther ones thin below a pixel and fade.
// - A drop shows the scene behind it refracted, inverted, from about 165° (Garg & Nayar 2004, "Photometric model of a
//   rain drop"): the environment's mean light about the view, clouds and sea included, with the few per cent its face
//   reflects of the sky. So drops are dark under a dark deck, show against a dark hull or sea rather than the sky, and
//   shine only when the sun (or moon) stands behind them: their forward lobe of its light, through the clouds' shadow.
//
// Premultiplied alpha.

#import naval_sky::common::{sky}
#import naval_sky::environment::{SkyEnvironment, env_radiance}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow_fast}
#import naval::motion::motion_response
#import naval_clouds::precipitation::{ramp, phase}

struct RainUniforms {
    // Per class k: [3k] (phase x, y, z, box edge), [3k + 1] (box centre from the camera, the drops' top fall speed),
    // [3k + 2] (fall phase).
    boxes: array<vec4<f32>, 12>,
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    camera_velocity: vec3<f32>,
    opacity: f32,
    wind: vec3<f32>,
    // The drawn share of the rain's drops: each drop drawn stands for this many (1 when the tier draws them all).
    weight: f32,
    // The drops' sizes: x Marshall–Palmer's slope Λ (per mm), y the smallest drawn (mm), z 1 − exp(−Λ (largest −
    // smallest)) (the truncation), w the lens's aperture (m) at this view's magnification.
    drops: vec4<f32>,
    bolt_direction: vec3<f32>,
    bolt_irradiance: f32,
    // The camera's world position (m), for the clouds' shadow over the drops.
    camera: vec3<f32>,
    padding: f32,
    screen: vec2<f32>,
    padding_b: vec2<f32>,
}

// The clouds' shadow (`CloudShadow`'s group, bound by the colour draw only: the motion pass keeps the camera at group 0).
@group(0) @binding(0) var shadow_map: texture_2d<f32>;
@group(0) @binding(1) var shadow_sampler: sampler;
@group(0) @binding(2) var<uniform> shadow: CloudShadow;
@group(2) @binding(0) var<uniform> rain: RainUniforms;
// The sky's environment (naval-sky's atlas, sampler and uniform).
@group(3) @binding(0) var env_atlas: texture_2d<f32>;
@group(3) @binding(1) var env_sampler: sampler;
@group(3) @binding(2) var<uniform> env: SkyEnvironment;

// One exposure: a sixtieth of a second (a film camera's 180° shutter at 30 frames a second).
const EXPOSURE: f32 = 0.016666668;
// A drop nearer than this (m) is not drawn, fully from NEAR_FULL: the lens's own front.
const NEAR_CULL: f32 = 0.3;
const NEAR_FULL: f32 = 0.6;
// The share of a box's edge over which a drop fades out toward it (the box wraps there).
const EDGE_FADE: f32 = 0.08;
// A pixel's own footprint (px): the box filter's standard deviation, a little wider for the edge.
const PIXEL: f32 = 0.35;
// Streaks fainter than this at their middle are not drawn.
const ALPHA_CULL: f32 = 0.0008;
// The longest streak drawn (px): a camera flying through rain paints longer ones, too faint to see.
const MAX_PIXELS: f32 = 400.0;
// The light a drop passes on from behind it (two refractions lose about 6 %), and the sky's light its face reflects.
const TRANSMITTED: f32 = 0.94;
const REFLECTED: f32 = 0.06;
// The asymmetry of the light a drop refracts forward (its geometric optics: the diffraction peak stays in the beam).
const FORWARD_G: f32 = 0.75;
// Lightning seen through the drops: its forward lobe and its floor, as the web's.
const BOLT_G: f32 = 0.7;
const BOLT_FLOOR: f32 = 0.01;
const BOLT_GAIN: f32 = 0.08;
const SQRT_TAU: f32 = 2.5066282746310002;

// The standard normal's distribution function (a tanh form, within 2e-4). The argument is held within ±6, where the
// function is 0 or 1 to the float: Metal's fast tanh of a far larger one is not a number.
fn normal_cdf(x: f32) -> f32 {
    let y = clamp(x, -6.0, 6.0);
    return 0.5 + 0.5 * tanh(0.7978846 * (y + 0.044715 * y * y * y));
}

struct RainVertex {
    @builtin(position) position: vec4<f32>,
    // Pixels across the streak from its axis, and along it from where the drop was as the exposure opened.
    @location(0) across: f32,
    @location(1) along: f32,
    // The streak's length (px) and its profile's spread (px).
    @location(2) pixels: f32,
    @location(3) sigma: f32,
    // The drop's area (px²) times what it stands for: the coverage its streak spreads.
    @location(4) ink: f32,
    @location(5) radiance: vec3<f32>,
}

// A drop's streak, without its light.
struct Streak {
    out: RainVertex,
    // From the eye to the drop (camera-relative world m), and whether it is drawn.
    head: vec3<f32>,
    visible: bool,
}

fn streak_geometry(index: u32, drop: vec4<f32>) -> Streak {
    // Corners: x across the streak (−1, 1), y along it (0 tail, 1 head); two triangles (0 1 2, 2 1 3).
    var corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    var order = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    let corner = corners[order[index % 6u]];
    let k = i32(drop.w);
    let seed = fract(drop.w);
    let box = rain.boxes[k * 3];
    let place = rain.boxes[k * 3 + 1];
    let fall = rain.boxes[k * 3 + 2];
    // The drop's diameter (mm): its own quantile of the truncated exponential above the smallest drawn.
    let quantile = fract(seed * 7.919);
    let diameter = rain.drops.y - log(1.0 - quantile * rain.drops.z) / rain.drops.x;
    // Its terminal speed (m/s) in whole tenths of the top speed, so its fall wraps with the box's (every tenth's phase
    // wraps at the box's 10).
    let terminal = 9.65 - 10.3 * exp(-0.6 * diameter);
    let tenths = clamp(round(terminal / place.w * 10.0), 1.0, 10.0);
    let speed = place.w * tenths * 0.1;
    let q = fract(drop.xyz + box.xyz - vec3<f32>(0.0, fall.x * tenths * 0.1, 0.0)) - 0.5;
    let edge = max(abs(q.x), max(abs(q.y), abs(q.z)));
    let head = place.xyz + q * box.w;
    // Where the drop was as the exposure opened: its motion relative to the camera, the wind's and its fall.
    let velocity = vec3<f32>(rain.wind.x, -speed, rain.wind.z) - rain.camera_velocity;
    let tail = head - velocity * EXPOSURE;
    let view_head = (rain.view * vec4<f32>(head, 0.0)).xyz;
    let view_tail = (rain.view * vec4<f32>(tail, 0.0)).xyz;
    let depth = -view_head.z;
    let clip_head = rain.projection * vec4<f32>(view_head, 1.0);
    let clip_full = rain.projection * vec4<f32>(view_tail, 1.0);
    let half_screen = rain.screen * 0.5;
    let pixel_head = clip_head.xy / clip_head.w * half_screen;
    let cut = min(1.0, MAX_PIXELS / max(length(pixel_head - clip_full.xy / clip_full.w * half_screen), 1e-3));
    let clip_tail = mix(clip_head, clip_full, cut);
    let tail_depth = clip_tail.w;
    let pixel_tail = clip_tail.xy / clip_tail.w * half_screen;
    let delta = pixel_head - pixel_tail;
    let pixels = length(delta);
    let direction = select(vec2<f32>(0.0, 1.0), delta / max(pixels, 1e-3), pixels > 1e-3);
    let normal = vec2<f32>(-direction.y, direction.x);
    // The drop's width on screen, the lens's blur of it (focused far: the aperture over the distance), and the
    // profile they and the pixel spread its streak over.
    let focal = rain.projection[1][1] * rain.screen.y * 0.5;
    let distance = max(depth, NEAR_CULL);
    let width = diameter * 0.001 * focal / distance;
    let blur = rain.drops.w * focal / distance;
    let sigma = sqrt((width * width + blur * blur) / 16.0 + PIXEL * PIXEL);
    // Its area (px²) and what it stands for, faded in from the lens and out toward its box's edge (it wraps there).
    let ink = 0.7853982 * width * width * rain.weight * rain.opacity * ramp(NEAR_CULL, NEAR_FULL, depth)
        * (1.0 - ramp(0.5 - EDGE_FADE / 2.0, 0.5, edge));
    // At the middle of the streak: the area spread along its length and across its profile.
    let middle = ink / (max(pixels, SQRT_TAU * sigma) * SQRT_TAU * sigma);
    let reach = 3.0 * sigma;
    var out: RainVertex;
    out.across = corner.x * reach;
    out.along = corner.y * pixels + (corner.y * 2.0 - 1.0) * reach;
    out.pixels = pixels;
    out.sigma = sigma;
    out.ink = ink;
    out.radiance = vec3<f32>(0.0);
    let visible = depth > NEAR_CULL && tail_depth > NEAR_CULL * 0.5 && middle > ALPHA_CULL;
    let pixel = mix(pixel_tail, pixel_head, corner.y) + normal * (corner.x * reach) + direction * ((corner.y * 2.0 - 1.0) * reach);
    let end = mix(clip_tail, clip_head, corner.y);
    out.position = select(vec4<f32>(0.0, 0.0, 2.0, 1.0), vec4<f32>(pixel / half_screen * end.w, end.z, end.w), visible);
    return Streak(out, head, visible);
}

@vertex
fn rain_vertex(@builtin(vertex_index) index: u32, @location(0) drop: vec4<f32>) -> RainVertex {
    let streak = streak_geometry(index, drop);
    var out = streak.out;
    if !streak.visible {
        return out;
    }
    // The light: the scene behind the drop through it, the sky's off its face, the sun's (or moon's) forward lobe
    // where the clouds let its beam through, and lightning's.
    let view = normalize(streak.head);
    let behind = env_radiance(env_atlas, env_sampler, env, view, 1.0);
    let above = env_radiance(env_atlas, env_sampler, env, vec3<f32>(0.0, 1.0, 0.0), 1.0);
    let beam = cloud_shadow_fast(shadow_map, shadow_sampler, shadow, rain.camera + streak.head);
    let flash = sky.flash + 1.0;
    out.radiance = (behind * TRANSMITTED + above * REFLECTED) * flash
        + sky.light_color * beam * phase(dot(view, sky.light_direction), FORWARD_G)
        + vec3<f32>(0.8, 0.87, 1.0) * (rain.bolt_irradiance * (phase(dot(view, rain.bolt_direction), BOLT_G) + BOLT_FLOOR) * BOLT_GAIN);
    return out;
}

// The motion pass's: the streak alone (it binds the camera at group 0, not the clouds' shadow).
@vertex
fn rain_motion_vertex(@builtin(vertex_index) index: u32, @location(0) drop: vec4<f32>) -> RainVertex {
    return streak_geometry(index, drop).out;
}

@fragment
fn rain_fragment(in: RainVertex) -> @location(0) vec4<f32> {
    let alpha = coverage(in);
    return vec4<f32>(in.radiance * alpha, alpha);
}

// Temporal AA's motion pass: a streak falls somewhere new every frame, so where one shows the frame takes no history
// (the web's rain writes a response of 1), or the history averages the streaks away. Its faintest edge keeps some.
@fragment
fn rain_motion(in: RainVertex) -> @location(0) vec4<f32> {
    return motion_response(min(coverage(in) * 50.0, 1.0));
}

// The streak's time-averaged coverage at a pixel: the drop's area spread evenly along the streak and over its
// profile across (a Gaussian of `sigma`), its ends the same profile's.
fn coverage(in: RainVertex) -> f32 {
    let s = in.sigma;
    let across = exp(-0.5 * (in.across * in.across) / (s * s)) / (SQRT_TAU * s);
    var along: f32;
    if in.pixels < 0.05 * s {
        let d = in.along - 0.5 * in.pixels;
        along = exp(-0.5 * d * d / (s * s)) / (SQRT_TAU * s);
    } else {
        along = (normal_cdf(in.along / s) - normal_cdf((in.along - in.pixels) / s)) / in.pixels;
    }
    return min(in.ink * across * along, 1.0);
}
