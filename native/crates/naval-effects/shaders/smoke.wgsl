// Funnel exhaust: lit billow sprites (`applySmokeSprite`) and the plume ribbons under them
// (`smokeRibbonMaterial`), both from the web game's `SmokeSpriteMaterial.ts`, lit by `smokeRadiance`.
//
// A billow is one cauliflower texture (normal, thickness, density) shaded as a lumpy mound: its quad axes come
// from the sprite itself rather than screen derivatives. A ribbon is a camera-facing strip through the
// plume's released samples, shaded as a lit, turbulent tube cross-section anchored to the released smoke, and
// dissolving into patches as it ages. Both sink into gun smoke in front of them.

#import naval::camera::{camera, world_to_clip, curved}
#import naval_effects::common::{lighting, effect_fog, smoke_visibility, smoke_radiance, quad_corner, sstep}
#import naval_effects::shadow::puff_light
#import naval_effects::warp::{roll_phase, roll_flow, roll_layers, roll_sprite}

struct Sprite {
    position: vec4<f32>,
    size: vec4<f32>,
    color: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> sprites: array<Sprite>;
@group(2) @binding(1) var billow_map: texture_2d<f32>;
@group(2) @binding(2) var noise_map: texture_2d<f32>;
@group(2) @binding(3) var map_sampler: sampler;
@group(2) @binding(4) var noise_sampler: sampler;

struct BillowVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    // World position, distance along the view (m).
    @location(2) world: vec4<f32>,
    // World axes of +u and +v across the quad (m per unit of texture coordinate).
    @location(3) @interpolate(flat) across: vec4<f32>,
    @location(4) @interpolate(flat) up: vec4<f32>,
    // The shares of the sun's and the sky's light reaching the billow here (`puff_light`).
    @location(5) light: vec2<f32>,
    // The warp trial: the way up in the billow's own frame (shorter the more it is seen from above or below) and toward
    // the eye, and the billow's place in its cycles (`roll_phase`); `across.w` carries its seed.
    @location(12) @interpolate(flat) roll: vec4<f32>,
}

@vertex
fn billow_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> BillowVaryings {
    let sprite = sprites[instance];
    let corner = quad_corner(vertex_index);
    let angle = sprite.position.w;
    let c = cos(angle);
    let s = sin(angle);
    let axis_x = camera.inverse_view[0].xyz;
    let axis_y = camera.inverse_view[1].xyz;
    let across = (axis_x * c + axis_y * s) * sprite.size.x;
    let up = (axis_y * c - axis_x * s) * sprite.size.y;
    let world = sprite.position.xyz + across * corner.x + up * corner.y;
    let view = camera.view * vec4<f32>(curved(world), 1.0);
    var out: BillowVaryings;
    out.position = world_to_clip(world);
    out.uv = corner + 0.5;
    out.color = sprite.color;
    out.world = vec4<f32>(world, -view.z);
    // A sprite has no seed of its own: the pool sends one with its roll (`warp::sprite`).
    let rolled = roll_sprite(sprite.size.w);
    out.across = vec4<f32>(across, rolled.y);
    out.up = vec4<f32>(up, 0.0);
    // The renderer's sun shadows and the smoke's own, read per corner a little inside the billow.
    out.light = puff_light(mix(sprite.position.xyz, world, 0.6), sprite.position.xyz, sprite.size.x * 0.5);
    out.roll = vec4<f32>(axis_x.y * c + axis_y.y * s, axis_y.y * c - axis_x.y * s, camera.inverse_view[2].y,
        roll_phase(rolled.x, rolled.y * 100.0));
    return out;
}

@fragment
fn billow_fragment(in: BillowVaryings) -> @location(0) vec4<f32> {
    var sample: vec4<f32>;
    if lighting.trial.x > 0.5 {
        // The warp trial: two layers of the cauliflower riding the billow's thermal a half cycle apart. Their densities
        // mix by their shares; the normal and thickness are the denser layer's by as much as it is the denser.
        let at = in.uv * 2.0 - 1.0;
        let rolling = roll_flow(at, in.roll.xy, in.roll.z, in.across.w);
        let roll = roll_layers(at, rolling.flow, in.roll.w + rolling.stagger.x);
        let first = textureSample(billow_map, map_sampler, roll.first * 0.5 + 0.5);
        let second = textureSample(billow_map, map_sampler, roll.second * 0.5 + 0.5);
        let dense = vec2<f32>(first.a * (1.0 - roll.blend), second.a * roll.blend) + vec2<f32>(1e-5, 0.0);
        sample = vec4<f32>(mix(first.rgb, second.rgb, dense.y / (dense.x + dense.y)), mix(first.a, second.a, roll.blend));
    } else {
        sample = textureSample(billow_map, map_sampler, in.uv);
    }
    let qx = sample.r * 2.0 - 1.0;
    let qy = sample.g * 2.0 - 1.0;
    let qz = sqrt(max(0.0, 1.0 - qx * qx - qy * qy));
    let to_camera = normalize(camera.position - in.world.xyz);
    let facing = normalize(cross(in.across.xyz, in.up.xyz));
    let front = select(facing, -facing, dot(facing, to_camera) < 0.0);
    let normal = normalize(normalize(in.across.xyz) * qx + normalize(in.up.xyz) * qy + front * qz);
    let density = sample.a;
    let radius = length(in.across.xyz) * 0.5;
    let color = smoke_radiance(normal, to_camera, density, sample.b, in.light) * in.color.rgb;
    // Its lumps bulge toward the camera from the sprite, which passes through the billow's centre.
    let near = in.world.w - radius * sample.b;
    let alpha = in.color.a * density * sstep(0.0, 4.0, in.world.y) * smoke_visibility(in.position.xy, near, radius * 0.5, false);
    return vec4<f32>(effect_fog(color, in.world.xyz), alpha);
}

/// Metres of plume per turn of the ribbon turbulence along its length.
const ALONG_SCALE_M: f32 = 45.0;
/// Seconds over which a plume ribbon thickens from nothing at the funnel rim.
const RIBBON_EMERGE_S: f32 = 0.6;

struct RibbonVertex {
    @location(0) position: vec4<f32>,
    @location(1) side: vec4<f32>,
    @location(2) data: vec4<f32>,
    @location(3) tint: vec4<f32>,
}

struct RibbonVaryings {
    @builtin(position) position: vec4<f32>,
    // World position, distance along the view (m).
    @location(0) world: vec4<f32>,
    // Side vector scaled by how squarely the plume faces the camera, fraction of the sample's life spent.
    @location(1) side: vec4<f32>,
    // Across (−1 … 1), metres along the plume, age (s), width (m).
    @location(2) data: vec4<f32>,
    @location(3) tint: vec4<f32>,
    // The shares of the sun's and the sky's light reaching the plume here (`puff_light`).
    @location(4) light: vec2<f32>,
}

@vertex
fn ribbon_vertex(vertex: RibbonVertex) -> RibbonVaryings {
    let view = camera.view * vec4<f32>(curved(vertex.position.xyz), 1.0);
    var out: RibbonVaryings;
    out.position = world_to_clip(vertex.position.xyz);
    out.world = vec4<f32>(vertex.position.xyz, -view.z);
    out.side = vec4<f32>(vertex.side.xyz, vertex.position.w);
    out.data = vertex.data;
    out.tint = vertex.tint;
    // The renderer's sun shadows and the smoke's own, read per vertex a little inside the plume's section, taken as a
    // puff about its middle line.
    let half = max(vertex.data.w, 0.5) * 0.5;
    let side = vertex.side.xyz / max(length(vertex.side.xyz), 0.05);
    let middle = vertex.position.xyz - side * (vertex.data.x * half);
    out.light = puff_light(mix(middle, vertex.position.xyz, 0.6), middle, half);
    return out;
}

@fragment
fn ribbon_fragment(in: RibbonVaryings) -> @location(0) vec4<f32> {
    let facing = length(in.side.xyz);
    let side = in.side.xyz / max(facing, 0.05);
    let spent = in.side.w;
    // A receding plume integrates its turbulence over depth: flatten the noise there.
    let contrast = sstep(0.2, 0.85, facing);
    let across = in.data.x;
    let width = max(in.data.w, 0.5);
    // Anchored to the released smoke; across, feature size follows the plume's width, along it is fixed.
    let scale = width * 1.1 + 10.0;
    let coarse = vec2<f32>(across * width * 0.5 / scale, in.data.y / ALONG_SCALE_M);
    let first = textureSample(noise_map, noise_sampler, coarse + vec2<f32>(0.0, in.data.z * -0.004));
    let fine = textureSample(noise_map, noise_sampler, coarse * 2.7 + vec2<f32>(in.data.z * 0.011, in.data.z * 0.007));
    let turbulence = mix(0.55, first.r * 0.65 + fine.g * 0.35, contrast);
    let profile = exp(across * across * -2.4);
    // Erode the rim more than the core; old smoke dissolves into patches from the rim inward.
    let dissolve = sstep(0.45, 1.0, spent) * 0.55;
    let density = clamp(profile * (turbulence * 1.6 + 0.08) - (1.0 - profile) * 0.3 - dissolve * (1.2 - turbulence), 0.0, 1.0);
    let to_camera = normalize(camera.position - in.world.xyz);
    let bulge = sqrt(max(0.0, 1.0 - across * across));
    // Tube normal, tipped up where the turbulence is dense so billowed crowns catch the sun.
    let normal = normalize(side * across + to_camera * bulge + vec3<f32>(0.0, (turbulence - 0.5) * contrast * 0.8, 0.0));
    let color = smoke_radiance(normal, to_camera, density, profile * 0.8, in.light) * in.tint.rgb;
    // Smoke never hangs below the sea.
    let above_sea = sstep(0.0, 10.0, in.world.y);
    // The column grows out of the funnel smoothly, from zero age at the rim.
    let emerge = sstep(0.0, RIBBON_EMERGE_S, in.data.z);
    let near = in.world.w - width * 0.5 * bulge;
    let alpha = density * in.tint.a * above_sea * emerge * smoke_visibility(in.position.xy, near, width * 0.25, false);
    return vec4<f32>(effect_fog(color, in.world.xyz), alpha);
}
