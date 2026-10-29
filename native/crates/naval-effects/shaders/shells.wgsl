// Shells in flight (naval_effects::shells): the round's body, lit by the scene, its pale trail and a tracer's spark
// and streak. First ported from the web game's `CombatEffects.ts` (`projectileMaterial`, the `shellGlows` and `streaks`
// basic materials: additive, depth-writing, alpha tested; the trail writes no depth) and `ShellTrails.ts` (`TrailMaterial`: a ribbon whose width
// and opacity run from tail to head, a Gaussian cross-section); since 2026-10-10 nothing glows from heat, only a
// tracer burning in the base, and every size is the world's but the trail's, a few pixels at any range
// (native/docs/combat-effects/shells-and-strikes.md, "Shells in flight").
// Every fragment takes the ocean's distance fog, as three's basic and standard materials take `scene.fogNode`.
//
// Narrower than a few pixels, a spark, streak or trail segment is drawn that wide, keeping what it adds to the frame
// (a spark's light by the area it gained, a line's by the width it gained, the trail's Gaussian broadened to about a
// pixel); a spark or streak is alpha tested as the small one would be. So a tracer dims with its distance as a point of light does, and
// a thin line stays whole where it would break into dashes (native/docs/aa-tracers.md).
//
// Groups: 0 the camera, 1 the effect lighting (naval_effects::common), 2 the batch.

#import naval::camera::{camera, world_to_clip, curved}
#import naval::motion::motion_response
#import naval_effects::common::{lighting, emitted, effect_fog, effect_fog_additive, quad_corner, sky_fill, scene_light, surface_sunlight}
#import naval::math::rotate

const MIN_LINE_PX: f32 = 2.0;
const MIN_SPARK_PX: f32 = 3.0;
// A marker on the helm's heavy rounds: the share of the view's height it spans, at least MARKER_MIN_PX; and the
// round's length (calibres, the lathe's) and the lengths on screen (px) over which it gives way to the round itself.
const MARKER_VIEW: f32 = 0.0018;
const MARKER_MIN_PX: f32 = 2.0;
const ROUND_CALIBERS: f32 = 4.4;
const MARKER_FADE_PX: vec2<f32> = vec2<f32>(6.0, 12.0);
const PI: f32 = 3.14159265;

struct Body {
    // Position (m) and its tracer (1 burning, 0 none).
    position: vec4<f32>,
    // Rotation taking +Y to the flight direction (xyzw).
    rotation: vec4<f32>,
    // Calibre (m).
    scale: vec4<f32>,
}

struct Glow {
    // Centre (m) and edge length (m), drawn at least MIN_SPARK_PX wide; a marker's centre and its round's calibre (m).
    position: vec4<f32>,
}

struct Strip {
    // Centre (m), a streak's width (m).
    center: vec4<f32>,
    // Unit across axis, a trail's tail width (m).
    across: vec4<f32>,
    // Tail to head (m), a trail's head width (m).
    along: vec4<f32>,
    // A trail's opacity at the tail and the head.
    opacity: vec4<f32>,
}

// Linear colours (naval_effects::shell_layer::material).
struct ShellMaterial {
    // rgb spark colour × intensity, w opacity.
    glow: vec4<f32>,
    // rgb streak colour × intensity, w opacity.
    streak: vec4<f32>,
    // rgb trail colour, w unused.
    trail: vec4<f32>,
    // rgb the tracer's burning face × intensity, w unused.
    face: vec4<f32>,
    // x spark alpha test, y streak alpha test, zw unused.
    alpha_test: vec4<f32>,
    // rgb a marker's colour in daylight, w the share of it kept at night.
    marker: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> bodies: array<Body>;
@group(2) @binding(1) var<storage, read> glows: array<Glow>;
@group(2) @binding(2) var<storage, read> strips: array<Strip>;
@group(2) @binding(3) var<uniform> material: ShellMaterial;
@group(2) @binding(4) var tracer_map: texture_2d<f32>;
@group(2) @binding(5) var map_sampler: sampler;

// World metres a pixel spans at `depth` metres in front of the camera.
fn pixel_meters(world: vec3<f32>) -> f32 {
    let view = camera.view * vec4<f32>(curved(world), 1.0);
    let depth = max(-view.z, camera.near);
    return 2.0 * depth / (camera.projection[1][1] * camera.viewport.y);
}

// ---- Bodies ----

struct BodyVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // The lathe's own position (calibres) and the tracer.
    @location(3) local_tracer: vec4<f32>,
}

@vertex
fn body_vertex(
    @location(0) local: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    @builtin(instance_index) instance: u32,
) -> BodyVaryings {
    let body = bodies[instance];
    let world = body.position.xyz + rotate(body.rotation, local * body.scale.x);
    var out: BodyVaryings;
    out.position = world_to_clip(world);
    out.world = world;
    out.normal = rotate(body.rotation, normal);
    out.color = color;
    out.local_tracer = vec4<f32>(local, body.position.w);
    return out;
}

@fragment
fn body_fragment(in: BodyVaryings) -> @location(0) vec4<f32> {
    // MeshStandardMaterial with vertex colours, metalness 0.45, roughness 0.36, under the effects' sun and sky
    // (a Lambert and Blinn-Phong stand-in for three's physical lights and environment).
    let n = normalize(in.normal);
    let v = normalize(camera.position - in.world);
    let l = normalize(lighting.sun_direction.xyz);
    let metalness = 0.45;
    let albedo = in.color;
    // The effects' `direct` is the sun scaled for smoke (1.25 at a clear noon for three's 5.8): back to irradiance,
    // through the renderer's sun shadow maps (a round falling into a hull's shadow loses the sun).
    let sun = lighting.direct.rgb * (5.8 / 1.25) * surface_sunlight(in.world, n);
    let sky = sky_fill() * PI;
    let n_dot_l = max(dot(n, l), 0.0);
    let diffuse = albedo * (1.0 - metalness) * (sun * n_dot_l + sky) / PI;
    let f0 = mix(vec3<f32>(0.04), albedo, metalness);
    let h = normalize(l + v);
    let shininess = 118.0;
    let specular = f0 * sun * n_dot_l * pow(max(dot(n, h), 0.0), shininess) * (shininess + 8.0) / (8.0 * PI);
    let reflection = f0 * sky / PI;
    // A tracer burns in the cup in the base (the lathe's recess, 0.17 calibres across, behind -1.87); the rest of the
    // round is cold steel: 800 m/s does not heat a shell to a glow.
    let local = in.local_tracer.xyz;
    let cup = (1.0 - smoothstep(0.175, 0.2, length(local.xz))) * (1.0 - smoothstep(-1.87, -1.84, local.y));
    let emissive = emitted(material.face.rgb * cup * in.local_tracer.w);
    return vec4<f32>(effect_fog(diffuse + specular + reflection + emissive, in.world), 1.0);
}

@fragment
fn body_motion(in: BodyVaryings) -> @location(0) vec4<f32> {
    return motion_response(1.0);
}

// ---- Sparks ----

struct SpriteVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    // Opacity from the vertex: the share of its light a widened sprite keeps per pixel.
    @location(2) opacity: f32,
}

@vertex
fn glow_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> SpriteVaryings {
    let glow = glows[instance];
    let corner = quad_corner(vertex);
    // A point of light: under MIN_SPARK_PX it spreads the same light over that many pixels, so its brightness falls
    // with the square of its distance.
    let size = glow.position.w;
    let drawn = max(size, MIN_SPARK_PX * pixel_meters(glow.position.xyz));
    // A plane turned with the camera: facing it, upright in its frame.
    let world = glow.position.xyz
        + (camera.inverse_view[0].xyz * corner.x + camera.inverse_view[1].xyz * corner.y) * drawn;
    var out: SpriteVaryings;
    out.position = world_to_clip(world);
    out.uv = corner + 0.5;
    out.world = world;
    let share = size / max(drawn, 1e-6);
    out.opacity = share * share;
    return out;
}

// The spark's glare, exp(−12 r²) over the sprite, computed rather than sampled: at a tracer's intensity an 8-bit
// texture's last step is still bright, and ended the glare in a hard rim seen close.
fn glow_color(in: SpriteVaryings) -> vec4<f32> {
    let r = length(in.uv * 2.0 - 1.0);
    return vec4<f32>(material.glow.rgb, exp(-12.0 * r * r) * material.glow.w);
}

@fragment
fn glow_fragment(in: SpriteVaryings) -> @location(0) vec4<f32> {
    let color = glow_color(in);
    if color.a < material.alpha_test.x {
        discard;
    }
    return vec4<f32>(effect_fog_additive(emitted(color.rgb), in.world), color.a * in.opacity);
}

@fragment
fn glow_motion(in: SpriteVaryings) -> @location(0) vec4<f32> {
    if glow_color(in).a < material.alpha_test.x {
        discard;
    }
    return motion_response(1.0);
}

// ---- Markers: the helm's heavy rounds, an aid to play rather than anything seen ----

@vertex
fn marker_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> SpriteVaryings {
    let marker = glows[instance];
    let center = marker.position.xyz;
    let corner = quad_corner(vertex);
    // The same few pixels near and far.
    let pixel = pixel_meters(center);
    let drawn = max(MARKER_MIN_PX, MARKER_VIEW * camera.viewport.y) * pixel;
    let world = center + (camera.inverse_view[0].xyz * corner.x + camera.inverse_view[1].xyz * corner.y) * drawn;
    var out: SpriteVaryings;
    out.position = world_to_clip(world);
    out.uv = corner + 0.5;
    out.world = world;
    // Close enough to show the round itself, it gives way to it.
    let round = marker.position.w * ROUND_CALIBERS / pixel;
    out.opacity = 1.0 - smoothstep(MARKER_FADE_PX.x, MARKER_FADE_PX.y, round);
    return out;
}

// A round dot, soft at its edge.
fn marker_alpha(in: SpriteVaryings) -> f32 {
    return (1.0 - smoothstep(0.6, 1.0, length(in.uv * 2.0 - 1.0))) * in.opacity;
}

@fragment
fn marker_fragment(in: SpriteVaryings) -> @location(0) vec4<f32> {
    let alpha = marker_alpha(in);
    if alpha <= 0.0 {
        discard;
    }
    // Painted over the scene rather than added to it, so it shows on a bright sky as on a dark sea; dimmer at night.
    let light = mix(material.marker.w, 1.0, scene_light());
    return vec4<f32>(material.marker.rgb * light, alpha);
}

@fragment
fn marker_motion(in: SpriteVaryings) -> @location(0) vec4<f32> {
    if marker_alpha(in) < 0.5 {
        discard;
    }
    return motion_response(1.0);
}

// ---- Streaks and trails: quads along a flight ----

// Corner `vertex` of a strip whose width runs from `tail_width` to `head_width` (m), floored at MIN_LINE_PX.
// `coverage` is the true width over the drawn one (1 when not widened).
struct StripVaryings {
    @builtin(position) position: vec4<f32>,
    // Across (0 … 1 over the drawn width), along (0 tail … 1 head).
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) coverage: f32,
    // A trail's opacity along the segment (1 for a streak).
    @location(3) opacity: f32,
}

fn strip_corner(strip: Strip, vertex: u32, tail_width: f32, head_width: f32) -> StripVaryings {
    let corner = quad_corner(vertex);
    let along = corner.y + 0.5;
    let spine = strip.center.xyz + strip.along.xyz * corner.y;
    let width = mix(tail_width, head_width, along);
    let drawn = max(width, MIN_LINE_PX * pixel_meters(spine));
    let world = spine + strip.across.xyz * (corner.x * drawn);
    var out: StripVaryings;
    out.position = world_to_clip(world);
    out.uv = vec2<f32>(corner.x + 0.5, along);
    out.world = world;
    out.coverage = width / max(drawn, 1e-6);
    out.opacity = 1.0;
    return out;
}

@vertex
fn streak_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> StripVaryings {
    let strip = strips[instance];
    return strip_corner(strip, vertex, strip.center.w, strip.center.w);
}

// The streak's colour and its alpha before widening scaled it down (for the alpha test).
fn streak_color(in: StripVaryings) -> vec4<f32> {
    let texel = textureSample(tracer_map, map_sampler, in.uv);
    return vec4<f32>(material.streak.rgb * texel.rgb, texel.a * material.streak.w);
}

@fragment
fn streak_fragment(in: StripVaryings) -> @location(0) vec4<f32> {
    let color = streak_color(in);
    if color.a < material.alpha_test.y {
        discard;
    }
    return vec4<f32>(effect_fog_additive(color.rgb, in.world), color.a * in.coverage);
}

@fragment
fn streak_motion(in: StripVaryings) -> @location(0) vec4<f32> {
    if streak_color(in).a < material.alpha_test.y {
        discard;
    }
    return motion_response(1.0);
}

@vertex
fn trail_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> StripVaryings {
    let strip = strips[instance];
    var out = strip_corner(strip, vertex, strip.across.w, strip.along.w);
    out.opacity = mix(strip.opacity.x, strip.opacity.y, out.uv.y);
    return out;
}

// ∫ exp(−a x²) over −1 … 1: sqrt(π / a) · erf(sqrt(a)), with erf by Abramowitz and Stegun 7.1.26.
fn gaussian_area(a: f32) -> f32 {
    let x = sqrt(a);
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0 - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t * exp(-a);
    return sqrt(PI / a) * erf;
}

@fragment
fn trail_fragment(in: StripVaryings) -> @location(0) vec4<f32> {
    // The web's cross-section exp(−7 x²) over the ribbon. Widened, the same vapour spreads over the drawn width in
    // a broader Gaussian (about a pixel wide at MIN_LINE_PX), so the line stays whole across pixel rows.
    let x = (in.uv.x - 0.5) * 2.0;
    let k = clamp(in.coverage, 0.0, 1.0);
    let a = mix(2.8, 7.0, k);
    let scale = k * gaussian_area(7.0) / gaussian_area(a);
    // Not alpha tested: it writes no depth, and a test drops the half-covered rows of a faint line into dashes.
    let alpha = in.opacity * exp(-a * x * x) * scale;
    // Vapour scatters the scene's light: drawn in its daylight colour, it dims with dusk and night.
    return vec4<f32>(effect_fog(material.trail.rgb * scene_light(), in.world), alpha);
}
