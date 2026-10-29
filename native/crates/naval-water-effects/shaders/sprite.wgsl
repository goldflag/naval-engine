// Camera-facing sprites of an effect particle pool: ports the instanced plane of the web game's `EffectParticles.ts`
// (`EffectParticlePool`, its `writeInstancePose` quad) with two materials:
// - `fragment_spray`: `applySpraySprite` of the web game's `SprayMaterial.ts` on the billow map of `SmokeSpriteMaterial.ts`
//   (lit billows of white spray: a whale's blow; `quadAxes` becomes the instance's own axes, exact for these affine
//   quads);
// - `fragment_basic`: the pools' plain MeshBasicNodeMaterial (drops, mist): map × instance colour × material colour (the
//   scene's light, `scene_light`), opacity map alpha × the particle's.
// Straight alpha, blended src·a + dst·(1 − a) as three's NormalBlending; linear HDR radiance.
//
// Native since: lit spray is torn by noise of its own and thins into wisps as it ages (drops and entrained air, not
// cotton), its relief is shallower, every sprite fades where it meets the sea or a hull behind it (soft particles on
// the backdrop's depth, group 3), and the sun's shadow maps take the beam off it at each corner (`particle_sun`).

#import naval::camera::{camera, world_to_clip, curved}
#import naval_water_effects::spray::{EffectLighting, spray_radiance, scene_light, spray_fog, particle_sun, far_share}
#import naval_water_effects::value_noise::value_noise

@group(1) @binding(0) var<uniform> lighting: EffectLighting;
@group(1) @binding(1) var map: texture_2d<f32>;
@group(1) @binding(2) var map_sampler: sampler;
// The backdrop's reversed depth (opaque and water; a far-plane placeholder before the first copy).
@group(3) @binding(0) var scene_depth: texture_depth_2d;

// How much of the billow map's relief the lit spray keeps: a cloud of drops is no solid ball.
const RELIEF: f32 = 0.6;
// Soft particles: the depth over which a sprite fades into what is behind it, as a share of its width (m), and least.
const SOFT_SHARE: f32 = 0.3;
const SOFT_LEAST: f32 = 0.2;
// The tear: the noise's cells across the sprite, how far it moves the billow's density, and how much more of it the
// spray loses by the end of its life.
const TEAR_CELLS: f32 = 5.5;
const TEAR_DEPTH: f32 = 1.3;
const TEAR_AGE: f32 = 0.6;
// The density left in the gaps between clumps of drops, and the share of the sky's light the spray's shade keeps.
const CLUMP_GAPS: f32 = 0.45;
// How far the warp moves the billow map's reading (uv).
const WARP: f32 = 0.3;
const SKY_SHARE: f32 = 0.8;
// The billow's density is raised to this power: a thick middle, a thin veil of drops toward its rim.
const THINNING: f32 = 1.7;

struct Instance {
    // xyz centre (world m), w opacity.
    @location(0) center: vec4<f32>,
    // xyz the quad's +u edge (world m, full width), w the water level the sprite meets the sea at.
    @location(1) axis_u: vec4<f32>,
    // xyz the quad's +v edge, w the share of its life spent.
    @location(2) axis_v: vec4<f32>,
    // rgb linear tint, w the particle's own number (0–1).
    @location(3) color: vec4<f32>,
}

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) color: vec3<f32>,
    @location(3) opacity: f32,
    @location(4) surface: f32,
    @location(5) across: vec3<f32>,
    @location(6) up: vec3<f32>,
    @location(7) @interpolate(flat) aged: f32,
    @location(8) @interpolate(flat) seed: f32,
    // The fragment's depth in view space (m, negative ahead), and the depth it fades over.
    @location(9) view_z: f32,
    @location(10) @interpolate(flat) soft: f32,
    // The share of the sun's beam the shadow maps let through, read at each corner.
    @location(11) sun: f32,
}

// How much of a sprite is left in front of the backdrop: 0 where it meets the sea or a hull, 1 a sprite's `soft`
// metres in front of it.
fn soft_edge(input: Varyings) -> f32 {
    let depth = textureLoad(scene_depth, vec2<i32>(input.clip.xy), 0);
    let behind = -camera.near * camera.far / (depth * (camera.far - camera.near) + camera.near);
    return saturate((input.view_z - behind) / input.soft);
}

// Four vertices a sprite, as a triangle strip: (0, 0), (1, 0), (0, 1), (1, 1) in uv; v runs up the quad, as the web's
// PlaneGeometry, and v = 0 reads the texture's first row.
@vertex
fn vertex(@builtin(vertex_index) index: u32, instance: Instance) -> Varyings {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let local = corner - 0.5;
    let world = instance.center.xyz + instance.axis_u.xyz * local.x + instance.axis_v.xyz * local.y;
    var out: Varyings;
    out.clip = world_to_clip(world);
    out.uv = corner;
    out.world = world;
    out.color = instance.color.rgb;
    out.opacity = instance.center.w;
    out.surface = instance.axis_u.w;
    out.across = instance.axis_u.xyz;
    out.up = instance.axis_v.xyz;
    out.aged = instance.axis_v.w;
    out.seed = instance.color.w;
    out.view_z = (camera.view * vec4<f32>(curved(world), 1.0)).z;
    out.soft = max(length(instance.axis_u.xyz) * SOFT_SHARE, SOFT_LEAST);
    // Lifted half the sprite's width up the beam: a puff is shaded by what stands up-sun of it, not by its own caster.
    out.sun = particle_sun(world, length(instance.axis_u.xyz) * 0.5);
    return out;
}

@fragment
fn fragment_spray(input: Varyings) -> @location(0) vec4<f32> {
    let far = far_share(input.uv.x);
    // The billow map read through a warp of the particle's own: each puff of spray takes a ragged shape of its own
    // instead of the map's one rounded cauliflower.
    let at = input.uv * TEAR_CELLS + vec2<f32>(input.seed * 71.0, input.seed * 37.0);
    let warp = vec2<f32>(value_noise(at * 0.6 + 7.0), value_noise(at * 0.6 + 91.0)) - 0.5;
    let sample = textureSample(map, map_sampler, input.uv + warp * WARP);
    let qx = sample.r * 2.0 - 1.0;
    let qy = sample.g * 2.0 - 1.0;
    let qz = sqrt(max(0.0, 1.0 - qx * qx - qy * qy));
    let to_camera = normalize(camera.position - input.world);
    let facing = normalize(cross(input.across, input.up));
    let front = select(facing, -facing, dot(facing, to_camera) < 0.0);
    let relief = normalize(normalize(input.across) * qx + normalize(input.up) * qy + front * qz);
    let normal = normalize(mix(front, relief, RELIEF));
    // Torn by noise of its own (two scales, placed by the particle's number): the dense middle holds, the rim breaks
    // into wisps, and as it ages it thins away from the inside too.
    let noise = value_noise(at) * 0.5 + value_noise(at * 2.6 + 13.0) * 0.3 + value_noise(at * 7.1 + 29.0) * 0.2;
    let held = sample.a + (noise - 0.5) * TEAR_DEPTH - input.aged * input.aged * TEAR_AGE;
    // Inside, clumps and gaps of drops rather than an even fill.
    let clumps = mix(CLUMP_GAPS, 1.0, smoothstep(0.25, 0.75, value_noise(at * 1.7 + 51.0)));
    // The warp must not carry the billow to the quad's own edge.
    let inside = min(min(input.uv.x, 1.0 - input.uv.x), min(input.uv.y, 1.0 - input.uv.y));
    let edge = smoothstep(0.0, 0.18, inside);
    let near_density = pow(sample.a, THINNING) * smoothstep(0.0, 0.5, held) * clumps * edge;
    // Seen whole (a few pixels across): the billow's mass without its tears and clumps, thinning as it ages.
    let whole = sqrt(sample.a) * smoothstep(0.0, 0.5, sample.a - input.aged * input.aged * TEAR_AGE) * edge;
    let density = mix(near_density, max(near_density, whole), far);
    // The billow's thick core keeps less sky light than its thin rim, and a cloud of drops lit by the sky alone is
    // no brighter than the sky.
    let sky = (1.0 - sample.b * 0.35) * SKY_SHARE;
    let color = spray_radiance(lighting, normal, to_camera, 1.0, input.sun, sky, 1.0 - density, far) * input.color;
    // Spray meets the sea softly at its own water level instead of cutting it with a line.
    let meets_sea = smoothstep(input.surface - 1.0, input.surface + 2.5, input.world.y);
    return vec4<f32>(spray_fog(color, input.world), input.opacity * density * meets_sea * soft_edge(input));
}

@fragment
fn fragment_basic(input: Varyings) -> @location(0) vec4<f32> {
    let sample = textureSample(map, map_sampler, input.uv);
    return vec4<f32>(spray_fog(sample.rgb * input.color * scene_light(lighting, input.sun), input.world),
        sample.a * input.opacity * soft_edge(input));
}
