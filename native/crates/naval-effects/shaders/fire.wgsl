// Ship-fire quads (`FireBatch`): flame tongues and the puffs of a fire's smoke column. A port of
// `fireFlameMaterial` and `fireSmokeMaterial` (the web game's `FireMaterials.ts`).
//
// Flames: an animated, tapering tongue whose temperature falls from an orange root (soot glowing as a black body,
// `flame_light`) to a dull red tip; the shape comes from a periodic 3D Worley volume scrolled by the particle's age, so
// a paused frame freezes the fire mid-flicker. Smoke: lumpy spheres in the scene's light, with billowing relief,
// self-shadow toward the sun, backlit thin edges and an orange underglow from the fire below; detail follows the
// puff's size on screen so distant columns read as continuous smoke. Both fade behind gun smoke.

#import naval::camera::{camera, world_to_clip, curved}
#import naval_effects::common::{sky_fill, lighting, emitted, emitted_at, flame_light, effect_fog, smoke_visibility, columns_through, quad_corner, sstep}
#import naval_effects::shadow::puff_light
#import naval_effects::warp::{roll_phase, roll_flow, roll_layers}

struct FireQuad {
    center: vec4<f32>,
    sphere: vec4<f32>,
    state: vec4<f32>,
    color: vec4<f32>,
    size: vec4<f32>,
}

struct FireBasis {
    right: vec4<f32>,
    up: vec4<f32>,
    normal: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> quads: array<FireQuad>;
@group(2) @binding(1) var<uniform> basis: FireBasis;
@group(2) @binding(2) var noise: texture_3d<f32>;
@group(2) @binding(3) var noise_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    // World position, distance along the view (m).
    @location(1) world: vec4<f32>,
    // The particle's centre and radius (`fireCenter`).
    @location(2) @interpolate(flat) sphere: vec4<f32>,
    // Seed, life fraction, glow, opacity (`fireState`).
    @location(3) @interpolate(flat) state: vec4<f32>,
    // Albedo and age (`fireColor`).
    @location(4) @interpolate(flat) color: vec4<f32>,
    // The shares of the sun's and the sky's light reaching a smoke puff here (`puff_light`); a flame, its own light,
    // ignores them.
    @location(5) light: vec2<f32>,
    // The warp trial, for the smoke: the way up as the quad sees it (shorter the more it is seen from above or below)
    // and toward the eye, and the puff's place in its cycles (`roll_phase`).
    @location(12) @interpolate(flat) roll: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let quad = quads[instance];
    let corner = quad_corner(vertex_index);
    let world = quad.center.xyz + basis.right.xyz * (corner.x * quad.center.w) + basis.up.xyz * (corner.y * quad.size.x);
    let view = camera.view * vec4<f32>(curved(world), 1.0);
    var out: Varyings;
    out.position = world_to_clip(world);
    out.uv = corner + 0.5;
    out.world = vec4<f32>(world, -view.z);
    out.sphere = quad.sphere;
    out.state = quad.state;
    out.color = quad.color;
    out.light = puff_light(mix(quad.sphere.xyz, world, 0.6), quad.sphere.xyz, quad.sphere.w);
    out.roll = vec4<f32>(basis.right.y, basis.up.y, basis.normal.y, roll_phase(quad.size.y, quad.state.x));
    return out;
}

fn volume(p: vec3<f32>) -> vec4<f32> {
    return textureSampleLevel(noise, noise_sampler, p, 0.0);
}

@fragment
fn flame(in: Varyings) -> @location(0) vec4<f32> {
    let x = (in.uv.x - 0.5) * 2.0;
    let y = in.uv.y;
    let seed = in.state.x;
    let age = in.color.w;
    let n1 = volume(vec3<f32>(x * 0.3 + seed * 0.131, y * 0.5 - age * 1.35, seed * 0.071)).r;
    let n2 = volume(vec3<f32>(x * 0.8 + seed * 0.293, y * 1.2 - age * 2.7, seed * 0.113 + 0.5)).g;
    // The tongue sways more toward its tip; the ragged top tears off as it rises.
    let bend = (n1 - 0.5) * 1.15 * y;
    let width = mix(0.95, 0.08, pow(max(y, 0.0), 0.8));
    let edge = 1.0 - abs(x + bend) / width;
    let top = 1.0 - sstep(0.38, 1.0, y + (n2 - 0.5) * 0.62);
    let body = sstep(0.0, 0.5, edge + (n2 - 0.5) * 0.55) * top * sstep(0.0, 0.1, y);
    let temperature = in.state.z * (1.0 - y * 0.55) * (clamp(edge, 0.0, 1.0) * 0.45 + 0.55);
    let alpha = body * in.state.w * smoke_visibility(in.position.xy, in.world.w, 2.0, false)
        * columns_through(in.position.xy, in.world.w, 2.0);
    // The flames' alpha test rejects their empty margins.
    if alpha < 0.03 {
        discard;
    }
    // Soot glowing as a black body, its root's 1 the core of a fire's flame on the gas sprite's ramp (`flame_light`;
    // until 2026-10-24 a ramp to cream, some four times as bright).
    return vec4<f32>(effect_fog(emitted_at(flame_light(temperature * 1.4), lighting.emission.y), in.world.xyz), alpha);
}

@fragment
fn smoke(in: Varyings) -> @location(0) vec4<f32> {
    let center = in.sphere.xyz;
    let radius = max(in.sphere.w, 0.01);
    let offset = (in.world.xyz - center) / radius;
    let r = length(offset);
    // The puff as a ball about its centre, which the sprite passes through: its near surface bulges toward the camera.
    let near = in.world.w - sqrt(max(1.0 - r * r, 0.0)) * radius;
    let to_eye = normalize(camera.position - center);
    let sun = lighting.sun_direction.xyz;
    // Radius in framebuffer pixels, binoculars included.
    let pixels = camera.viewport.y * 0.5 * camera.projection[1][1] * radius / max(length(camera.position - center), radius);
    // Fine erosion cells are about a sixth of the radius: full strength once they span about ten pixels.
    let detail = sstep(14.0, 70.0, pixels);
    let broad = sstep(3.0, 18.0, pixels);
    // Billows roll slowly with the puff's own age, not wall time: a paused frame holds still.
    let age = in.color.w;
    let seed = vec3<f32>(in.state.x * 0.137, in.state.x * 0.071, in.state.x * 0.043);
    let p = offset + seed + vec3<f32>(age * 0.011, age * -0.017, age * 0.007);
    var coarse: f32;
    var fine: f32;
    var toward: f32;
    if lighting.trial.x > 0.5 {
        // The warp trial: the billows ride the puff's thermal over its face in two layers a half cycle apart, each
        // cycle on another patch of the noise, so its lumps roll up the middle and back down the rim for good. The
        // mix of two patches is lifted back to one patch's contrast.
        let s = in.uv * 2.0 - 1.0;
        let rolling = roll_flow(s, in.roll.xy, in.roll.z, fract(in.state.x * 0.3779));
        let roll = roll_layers(s, rolling.flow, in.roll.w + rolling.stagger.x);
        let right = camera.inverse_view[0].xyz;
        let up = camera.inverse_view[1].xyz;
        let pulled = roll.first - s;
        let other = roll.second - s;
        let first = p + right * pulled.x + up * pulled.y + roll.cycles.x * vec3<f32>(0.37, 0.61, 0.23);
        let second = p + right * other.x + up * other.y + roll.cycles.y * vec3<f32>(0.37, 0.61, 0.23) + 0.5;
        let lift = inverseSqrt(1.0 - 2.0 * roll.blend * (1.0 - roll.blend));
        coarse = clamp((mix(volume(first * 0.38).r, volume(second * 0.38).r, roll.blend) - 0.5) * lift + 0.5, 0.0, 1.0);
        fine = clamp((mix(volume(first * 0.72 + 0.31).g, volume(second * 0.72 + 0.31).g, roll.blend) - 0.5) * lift + 0.5, 0.0, 1.0);
        toward = clamp((mix(volume((first + sun * 0.42) * 0.38).r, volume((second + sun * 0.42) * 0.38).r, roll.blend) - 0.5) * lift + 0.5, 0.0, 1.0);
    } else {
        coarse = volume(p * 0.38).r;
        fine = volume(p * 0.72 + 0.31).g;
        // Self-shadow: billows that are denser a step toward the light shade this one.
        toward = volume((p + sun * 0.42) * 0.38).r;
    }
    // Fine erosion only where it spans enough pixels to read as texture rather than grain.
    let n = (coarse - 0.5) * mix(0.35, 0.62, broad) + (fine - 0.5) * (detail * 0.32);
    // Older puffs erode into thinner, ragged wisps as they mix with air.
    let shape = 1.0 - r + n * 1.05 - in.state.y * 0.2;
    // The edge ramp never narrows below about four pixels, and every puff reaches zero well inside its quad.
    let ramp = max(mix(0.8, 0.45, detail), 4.0 / max(pixels, 1.0));
    let density = sstep(0.0, ramp, shape) * (1.0 - sstep(0.72, 0.98, r));
    let r2 = r * r;
    let occluded = clamp((toward - coarse) * mix(0.8, 2.4, broad) + max(1.0 - r2, 0.0) * 0.25, 0.0, 0.8);
    let nz = sqrt(max(1.0 - r2, 0.0));
    let normal = normalize(offset + to_eye * nz);
    let wrap = clamp(dot(normal, sun) * 0.5 + 0.5, 0.0, 1.0);
    let thin = 1.0 - density;
    let backlight = pow(clamp(dot(-to_eye, sun), 0.0, 1.0), 6.0) * thin * 0.9;
    let sky = normal.y * 0.3 + 0.7;
    let relief = mix(1.0, fine * 0.35 + 0.82, detail);
    // The sun's light through hulls' shadows and the smoke between the puff and the sun (`puff_light`); the fire's
    // glow is not shadowed.
    let light = (sky_fill() * (sky * in.light.y) + lighting.direct.rgb * ((wrap * wrap * (1.0 - occluded) * 0.95 + backlight) * in.light.x)) * relief;
    let underside = clamp(-normal.y * 0.5 + 0.5, 0.0, 1.0);
    let glow = emitted_at(vec3<f32>(1.0, 0.36, 0.08) * in.state.z * (underside * underside * 0.85 + 0.15) * (density * 0.5 + 0.5), lighting.emission.z);
    let alpha = density * in.state.w * smoke_visibility(in.position.xy, near, radius * 0.5, false);
    return vec4<f32>(effect_fog(in.color.rgb * light + glow, in.world.xyz), alpha);
}
