// The bubbles a running torpedo leaves in the water, seen from under the sea (native only): the exhaust and the
// propellers' wash stream from its tail as a churning white tube that spreads into a loose cloud and rises, as the
// plume the surface's torpedo track is made of (`torpedo_tracks.rs`: about 1.5 m/s, surfacing well astern). Each
// bubble is the one released at its moment: an emission index counted on the presentation clock, so it stands where
// its torpedo's tail was then (back along the course from where the tail is now) and only rises and spreads after.
// No particle lives on the CPU: the instance index names the trail and the moment. Most are soft puffs of the cloud,
// some single bubbles that catch the light, both added to the scene after the underwater composite, lit as the water
// at their depth and dimmed by the water between them and the eye (`underwater.wgsl` dims only what lies behind),
// hidden by whatever stands nearer and drawn only where their pixel's ray starts under the waves. They stream past a
// camera riding a torpedo faster than temporal anti-aliasing's history can follow, so what they cover takes none.
// An oxygen torpedo's exhaust dissolves on the way up, so its trail shows a few of them (`OXYGEN_BUBBLES`).

#import naval::camera::{camera, world_to_clip}
#import naval::motion::motion_response
#import naval_ocean::underwater_ray::{water_light, diffuse_attenuation}
#import naval_ocean::water_properties::{WaterField, sample_water, water_beam, water_pigment}
#import naval_ocean::pcg::hash3

// `Underwater` in underwater.wgsl.
struct Underwater {
    pigment: vec4<f32>,
    absorption: vec4<f32>,
    transmission: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    water: WaterField,
}

// `naval_ocean::underwater::TrailUniform`: xyz where the tail was at the trail's anchor (now while it runs, where it
// ended after), w the seconds since that anchor; xyz its velocity (m/s), w the seconds it had run by its anchor; x its
// seed, y the share of its bubbles shown (an f32's bits: 1, an oxygen torpedo's `OXYGEN_BUBBLES`).
struct Trail {
    tail: vec4<f32>,
    velocity: vec4<f32>,
    seed: vec4<u32>,
}

// `naval_ocean::underwater::TrailsUniform`: x the newest bubble's emission index, y the trails written; x the share of
// an emission interval since the newest was released.
struct Trails {
    count: vec4<u32>,
    clock: vec4<f32>,
    trails: array<Trail, 16>,
}

@group(1) @binding(0) var<uniform> water: Underwater;
@group(1) @binding(2) var depth_probe: texture_2d<f32>;
@group(1) @binding(3) var probe_sampler: sampler;
@group(1) @binding(4) var<uniform> trails: Trails;

// One trail's bubbles (`naval_ocean::underwater::BUBBLES_PER_TRAIL`), one released each EMISSION seconds, the last
// LIFETIME seconds old.
const PER_TRAIL: u32 = 1080u;
const EMISSION: f32 = 1.0 / 360.0;
const LIFETIME: f32 = 3.0;
// The share of them that are single bubbles (the rest the cloud's puffs).
const SINGLES: f32 = 0.3;
// Released over the propellers' disc (radius, m), spreading as √age (m/√s), flattened a little in the vertical.
const DISC: f32 = 0.24;
const SPREAD: f32 = 0.42;
// The wash carries each astern of where it left (m), most of the way within JET_TIME seconds.
const JET: f32 = 1.4;
const JET_TIME: f32 = 0.35;
// Rising (m/s): the cloud's puffs and the single bubbles, from rest at release to full speed over RISE_TIME seconds.
const PUFF_RISE: vec2<f32> = vec2<f32>(0.8, 1.7);
const SINGLE_RISE: vec2<f32> = vec2<f32>(1.2, 2.4);
const RISE_TIME: f32 = 0.5;
// Sizes (m): a single bubble, and a puff at release and how it grows with √age.
const SINGLE_RADIUS: vec2<f32> = vec2<f32>(0.006, 0.03);
const PUFF_RADIUS: f32 = 0.13;
const PUFF_GROWTH: f32 = 0.5;
// A single bubble's smallest and largest size on screen (pixels); a puff's least.
const SMALLEST: f32 = 1.1;
const LARGEST: f32 = 4.0;
const PUFF_SMALLEST: f32 = 1.5;
// Light as a multiple of what the open water at its depth shows, and the opacity at the centre: a single bubble's,
// a puff's. A bubble returns the daylight that reaches it from the surface, many times the water's own faint glow,
// and leans towards white by WHITEN: not the water's colour.
const SINGLE_LIGHT: f32 = 24.0;
const SINGLE_OPACITY: f32 = 0.9;
const PUFF_LIGHT: f32 = 8.0;
const PUFF_OPACITY: f32 = 0.17;
const WHITEN: f32 = 0.45;
// Ranges (m) over which it fades in before the lens and out into the water; how near the surface it is lost into the
// track's foam there.
const NEAR: vec2<f32> = vec2<f32>(0.4, 1.2);
const FAR: vec2<f32> = vec2<f32>(70.0, 110.0);
const SURFACE: vec2<f32> = vec2<f32>(0.08, 0.45);

struct BubbleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) offset: vec2<f32>,
    // rgb its light, a its opacity.
    @location(1) light: vec4<f32>,
    // 1 for a puff's gaussian fall-off, 0 for a bubble's disc.
    @location(2) puff: f32,
}

fn hidden() -> BubbleOut {
    var out: BubbleOut;
    out.clip = vec4<f32>(0.0, 0.0, -2.0, 1.0);
    out.offset = vec2<f32>(0.0);
    out.light = vec4<f32>(0.0);
    out.puff = 0.0;
    return out;
}

@vertex
fn bubble_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> BubbleOut {
    let index = instance / PER_TRAIL;
    if index >= trails.count.y {
        return hidden();
    }
    let trail = trails.trails[index];
    let j = instance % PER_TRAIL;
    let age = (f32(j) + trails.clock.x) * EMISSION;
    // Released this long before the trail's anchor: none after it ended, none before it ran.
    let back = age - trail.tail.w;
    if back < 0.0 || back > trail.velocity.w {
        return hidden();
    }
    let moment = ((trails.count.x - j) * 2654435761u) ^ trail.seed.x;
    if hash3(moment ^ 0x85ebca6bu).x >= bitcast<f32>(trail.seed.y) {
        return hidden();
    }
    let a = hash3(moment);
    let b = hash3(moment ^ 0x9e3779b9u);
    let single = a.x < SINGLES;
    // The course and the axes across it.
    let forward = normalize(select(trail.velocity.xyz, vec3<f32>(0.0, 0.0, -1.0), dot(trail.velocity.xyz, trail.velocity.xyz) < 1e-6));
    let right = normalize(select(cross(forward, vec3<f32>(0.0, 1.0, 0.0)), vec3<f32>(1.0, 0.0, 0.0), abs(forward.y) > 0.999));
    let up = cross(right, forward);
    let released = trail.tail.xyz - trail.velocity.xyz * back;
    if released.y > -SURFACE.x {
        return hidden();
    }
    let angle = a.y * 6.2831853 + age * (b.z - 0.5) * 1.5;
    let spread = sqrt(a.z) * DISC + SPREAD * sqrt(age) * (0.5 + b.x);
    let rise_speed = select(mix(PUFF_RISE.x, PUFF_RISE.y, b.y), mix(SINGLE_RISE.x, SINGLE_RISE.y, b.y), single);
    let rise = rise_speed * (age - RISE_TIME * (1.0 - exp(-age / RISE_TIME)));
    let wobble = select(0.0, 0.04 * sin(age * 9.0 + b.z * 6.2831853), single);
    let world = released
        - forward * (JET * (1.0 - exp(-age / JET_TIME)))
        + right * ((cos(angle) * spread) + wobble)
        + up * (sin(angle) * spread * 0.75)
        + vec3<f32>(0.0, rise, 0.0);
    let below = -world.y;
    let eye = camera.position;
    let range = length(world - eye);
    if below < SURFACE.x || range > FAR.y || range < NEAR.x {
        return hidden();
    }
    let centre = world_to_clip(world);
    if centre.w < 1e-3 {
        return hidden();
    }
    let per_metre = camera.viewport.y / (2.0 * camera.tan_half_fov_y * max(range, 0.1));
    let size = select(PUFF_RADIUS + PUFF_GROWTH * sqrt(age) * (0.6 + b.x), mix(SINGLE_RADIUS.x, SINGLE_RADIUS.y, b.z * b.z), single);
    let pixels = size * per_metre;
    let radius = select(max(pixels, PUFF_SMALLEST), clamp(pixels, SMALLEST, LARGEST), single);
    // Its pixel's ray starts under the waves, or it is not drawn.
    let uv = centre.xy / centre.w * vec2<f32>(0.5, -0.5) + 0.5;
    let under = smoothstep(0.0, 0.4, textureSampleLevel(depth_probe, probe_sampler, uv, 0.0).r);
    // Fresh at release, gone by its lifetime; a puff thins as it spreads.
    let life = age / LIFETIME;
    let fading = select(pow(max(1.0 - life, 0.0), 1.6) * smoothstep(0.0, 0.06, age), 1.0 - smoothstep(0.55, 1.0, life), single);
    let shown = fading
        * smoothstep(SURFACE.x, SURFACE.y, below)
        * smoothstep(NEAR.x, NEAR.y, range)
        * (1.0 - smoothstep(FAR.x, FAR.y, range))
        * under
        * select(1.0, min(pixels / SMALLEST, 1.0), single);
    // Lit as the water at its depth (the daylight left there, the sun's forward lobe included), whiter than the water,
    // and dimmed by the water between it and the eye.
    let clarity = water.absorption.w;
    let view = (world - eye) / max(range, 1e-3);
    let optics = sample_water(water.water, world.xz, water.absorption.rgb);
    let beam = select(water.absorption.rgb, water_beam(optics), water.water.absorption.w > 0.5);
    let pigment = select(water.pigment.rgb, water_pigment(optics, water.absorption.rgb, water.pigment.rgb), water.water.absorption.w > 0.5);
    let glow = water_light(view, pigment, water.sun_direction.xyz, water.sun_color.rgb)
        * exp(-diffuse_attenuation(beam) * below * clarity);
    let white = mix(glow, vec3<f32>(max(glow.r, max(glow.g, glow.b))), WHITEN);
    let light = white * select(PUFF_LIGHT, SINGLE_LIGHT, single) * exp(-beam * range * clarity);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex];
    var out: BubbleOut;
    out.clip = centre + vec4<f32>(corner * (radius * 2.0) * camera.viewport.zw * centre.w, 0.0, 0.0);
    out.offset = corner;
    out.light = vec4<f32>(light, shown * select(PUFF_OPACITY, SINGLE_OPACITY, single));
    out.puff = select(1.0, 0.0, single);
    return out;
}

// How much of its quad a bubble covers here: a single bubble's soft disc, a puff's gaussian.
fn coverage(in: BubbleOut) -> f32 {
    let r = length(in.offset);
    let disc = 1.0 - smoothstep(0.35, 1.0, r);
    let cloud = exp(-r * r * 3.5) * (1.0 - smoothstep(0.85, 1.0, r));
    return in.light.a * mix(disc, cloud, in.puff);
}

// Added to the scene (blended: one, one).
@fragment
fn bubble_fragment(in: BubbleOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.light.rgb * coverage(in), 0.0);
}

// The motion pass: what a bubble covers takes no history, a faint puff's edge some.
@fragment
fn bubble_motion(in: BubbleOut) -> @location(0) vec4<f32> {
    let response = saturate(coverage(in) * 4.0);
    if response < 0.02 {
        discard;
    }
    return motion_response(response);
}
