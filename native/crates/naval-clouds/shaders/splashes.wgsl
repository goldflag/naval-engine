// Rain landing on the sea near the camera (native/docs/clouds/weather.md, "Rain as a camera records it", 2026-10-20): the
// crowns large drops throw up, scattered afresh every life over a square of sea that wraps about the camera, as the
// web game's `sky/weather/splashes.ts` (`SplashField.build`) scattered its rings and crowns. Now at their real size: a
// 3 mm drop's crown stands about 2 cm for a tenth of a second, so it shows within a few tens of metres and is gone,
// below a pixel, by 30 m. The rings every drop sends out are the sea's own (`naval-ocean`'s rain, in its normal where a
// pixel resolves them), and so is the roughening beyond. A crown is a thin sheet of water: it passes most of the sea
// behind it and catches the sky's light (and the sun's where the clouds let it through) at its rim.
// The splashes stand on the ocean's heights about the camera (`naval_ocean::rain_heights`, a square of `sea_span`
// metres repeating over the world), or on the mean sea without them. Premultiplied alpha.

#import naval_sky::common::{sky}
#import naval_sky::environment::{SkyEnvironment, env_radiance}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow_fast}
#import naval_clouds::precipitation::{ramp, phase}

struct SplashUniforms {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    // Per class: its clock in lifetimes (wrapped with the 60 s clock).
    cycles: vec4<f32>,
    // Region phase (x, z) in region units and the region centre relative to the camera (x, z).
    region: vec4<f32>,
    camera_xz: vec2<f32>,
    camera_height: f32,
    opacity: f32,
    // The crowns' size over a 3 mm drop's, and the drawing buffer's height (px).
    size: f32,
    screen_height: f32,
    padding: f32,
    // The span (m) of the sea's heights about the camera, 0 for the flat mean sea.
    sea_span: f32,
}

// The clouds' shadow (`CloudShadow`'s group).
@group(0) @binding(0) var shadow_map: texture_2d<f32>;
@group(0) @binding(1) var shadow_sampler: sampler;
@group(0) @binding(2) var<uniform> shadow: CloudShadow;
@group(2) @binding(0) var<uniform> splashes: SplashUniforms;
@group(2) @binding(1) var sea_heights: texture_2d<f32>;
@group(2) @binding(2) var sea_sampler: sampler;
// The sky's environment (naval-sky's atlas, sampler and uniform).
@group(3) @binding(0) var env_atlas: texture_2d<f32>;
@group(3) @binding(1) var env_sampler: sampler;
@group(3) @binding(2) var<uniform> env: SkyEnvironment;

// The square of sea the splashes are laid on (m; `SPLASH_REGION` in weather.rs).
const REGION: f32 = 24.0;
// Splashes fade out between these distances (m): a crown is a pixel high by 30 m at the bare eye's lens.
const FAR_FROM: f32 = 14.0;
const FAR_TO: f32 = 24.0;
// A 3 mm drop's crown at its height (m), its width (m), and where in its life it stands highest.
const CROWN_HEIGHT: f32 = 0.022;
const CROWN_WIDTH: f32 = 0.03;
const CROWN_PEAK: f32 = 0.3;
// The smallest a crown is drawn (px): a smaller one keeps its area's share of the light (its coverage).
const SMALLEST_PX: f32 = 1.5;
// What the sheet stops of the sea behind it, and the share of the sky's light its rim sends back.
const SHEET: f32 = 0.35;
const RIM_LIGHT: f32 = 0.5;
// Its forward lobe of the sun's (or moon's) light, as a drop's.
const FORWARD_G: f32 = 0.75;

// three's TSL `hash` (PCG).
fn pcg(seed: f32) -> f32 {
    let state = u32(seed) * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return f32((word >> 22u) ^ word) * (1.0 / 4294967296.0);
}

struct SplashVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) age: f32,
    @location(2) fade: f32,
    @location(3) light: vec3<f32>,
}

@vertex
fn splash_vertex(@builtin(vertex_index) index: u32, @location(0) splash: vec4<f32>) -> SplashVertex {
    // A crown standing on the water, facing the camera: x across (−1, 1), y up (0 the water, 1 its top).
    var corners = array<vec2<f32>, 4>(vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0));
    var order = array<u32, 6>(0u, 1u, 2u, 2u, 1u, 3u);
    // Lives per class within the 60 s clock (`SPLASH_PERIODS` in weather.rs).
    var lives = array<f32, 4>(750.0, 600.0, 500.0, 400.0);
    let corner = corners[order[index % 6u]];
    let k = i32(splash.w);
    let seed = fract(splash.w);
    let cycle = splashes.cycles[k] + splash.z;
    let progress = fract(cycle);
    // Every life lands somewhere new, counted within the clock's cycle so a splash keeps its place at the wrap.
    let life = floor(cycle) % lives[k];
    let key = life * 7919.0 + floor(seed * 65536.0);
    let scatter = vec2<f32>(pcg(key), pcg(key + 104729.0));
    let q = fract(splash.xy + scatter + splashes.region.xy) - 0.5;
    let edge = max(abs(q.x), abs(q.y));
    let xz = splashes.region.zw + q * REGION;
    // The drawn sea under the splash, where the ocean gives its heights.
    var sea = 0.0;
    if splashes.sea_span > 0.0 {
        sea = textureSampleLevel(sea_heights, sea_sampler, (splashes.camera_xz + xz) / splashes.sea_span, 0.0).r;
    }
    let base = vec3<f32>(xz.x, sea - splashes.camera_height, xz.y);
    let distance = length(base);
    let view = base / max(distance, 1e-3);
    // Its size: this drop's (a quantile about the mean), rising to its height and falling back into the sea.
    let own = 0.6 + 0.8 * fract(seed * 7.919);
    let rise = select(cos(1.5707963 * (progress - CROWN_PEAK) / (1.0 - CROWN_PEAK)), sin(1.5707963 * progress / CROWN_PEAK),
        progress < CROWN_PEAK);
    let height = CROWN_HEIGHT * splashes.size * own * max(rise, 0.0);
    let width = CROWN_WIDTH * splashes.size * own * (0.6 + 0.4 * sqrt(progress));
    // Held to a pixel and a half on screen, keeping its area's share of the light.
    let focal = splashes.projection[1][1] * splashes.screen_height * 0.5;
    let pixels = max(height, width) * focal / max(distance, 0.1);
    let grow = max(SMALLEST_PX / max(pixels, 1e-3), 1.0);
    let side = normalize(vec3<f32>(-view.z, 0.0, view.x) + vec3<f32>(1e-5, 0.0, 0.0));
    let position = base + side * (corner.x * width * 0.5 * grow) + vec3<f32>(0.0, corner.y * height * grow, 0.0);
    var out: SplashVertex;
    out.local = corner;
    out.age = progress;
    out.fade = (1.0 - ramp(0.4, 0.5, edge)) * (1.0 - ramp(FAR_FROM, FAR_TO, distance)) * splashes.opacity / (grow * grow);
    // The sky's light on its rim and the sun's forward lobe through it where the clouds let the beam through.
    let above = env_radiance(env_atlas, env_sampler, env, vec3<f32>(0.0, 1.0, 0.0), 1.0);
    let toward = env_radiance(env_atlas, env_sampler, env, normalize(vec3<f32>(view.x, 0.2, view.z)), 1.0);
    let beam = cloud_shadow_fast(shadow_map, shadow_sampler, shadow, vec3<f32>(splashes.camera_xz.x + xz.x, sea, splashes.camera_xz.y + xz.y));
    out.light = (0.5 * (above + toward) * RIM_LIGHT) * (sky.flash + 1.0)
        + sky.light_color * beam * phase(dot(view, sky.light_direction), FORWARD_G) * RIM_LIGHT;
    out.position = splashes.projection * vec4<f32>((splashes.view * vec4<f32>(position, 0.0)).xyz, 1.0);
    if height <= 0.0 || out.fade <= 0.0 {
        out.position = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    }
    return out;
}

@fragment
fn splash_fragment(in: SplashVertex) -> @location(0) vec4<f32> {
    // A crown's sheet: thin at the water, flaring to a thicker, brighter rim; softly bounded.
    let across = 1.0 - in.local.x * in.local.x;
    let up = in.local.y;
    let sheet = across * ramp(0.0, 0.25, up) * (1.0 - ramp(0.8, 1.0, up));
    let rim = across * ramp(0.55, 0.85, up) * (1.0 - ramp(0.85, 1.0, up));
    let alpha = (sheet * 0.6 + rim) * SHEET * in.fade;
    return vec4<f32>(in.light * (sheet * 0.4 + rim) * in.fade, alpha);
}
