// Textured sprites: flashes, flame jets, sparks and burning grains (additive), fire embers, halos and the glow
// on the sea under a fire. A port of `EffectParticlePool`'s basic material (the web game's `EffectParticles.ts`): the
// instance colour times the map's light, the map's alpha times the particle's opacity. Billboards face the camera
// rolled by their angle (velocity-aligned sprites arrive with that angle and stretch already worked out); water
// sprites lie flat. Fire glows fade behind gun smoke (`lighting.smoke.visibility(viewZ, 2)` on the web).

#import naval::camera::{camera, world_to_clip, curved}
#import naval_effects::common::{lighting, effect_fog_additive, emitted_at, smoke_visibility, columns_through, cloud_visibility, splashes_through, quad_corner}

struct Sprite {
    position: vec4<f32>,
    size: vec4<f32>,
    color: vec4<f32>,
}

struct SpriteBatch {
    // x 1 to fade behind the occlusion map, y alpha test threshold, z 1 for a fire's embers (drawn after the fires'
    // columns, which they fade behind too: `columns_through`), w unused.
    flags: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> sprites: array<Sprite>;
@group(2) @binding(1) var<uniform> batch: SpriteBatch;
@group(2) @binding(2) var map: texture_2d<f32>;
@group(2) @binding(3) var map_sampler: sampler;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    // World position, distance along the view (m).
    @location(2) world: vec4<f32>,
}

// The least a light is drawn across, in pixels. A muzzle flash 20 km off is a third of a pixel at the bare eye's lens:
// drawn at its size it falls between the pixels' samples and is gone (or flickers under the temporal filter). Drawn
// at least this wide, its light spread over the larger quad, it keeps the same light in all and shows as a point.
const LEAST_PIXELS: f32 = 1.5;

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let sprite = sprites[instance];
    let corner = quad_corner(vertex_index);
    // Every sprite here is light added to the scene: grown to the least width, it is dimmed by the area it gained.
    // Water sprites (the sea's glow under a fire) lie flat and keep their size.
    let depth = max(-(camera.view * vec4<f32>(curved(sprite.position.xyz), 1.0)).z, camera.near);
    let pixels = max(sprite.size.x, sprite.size.y) * camera.viewport.y / (2.0 * camera.tan_half_fov_y * depth);
    let grow = select(max(1.0, LEAST_PIXELS / max(pixels, 1e-3)), 1.0, sprite.size.z > 0.5);
    let local = corner * sprite.size.xy * grow;
    let angle = sprite.position.w;
    let c = cos(angle);
    let s = sin(angle);
    let rolled = vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);
    var axis_x = camera.inverse_view[0].xyz;
    var axis_y = camera.inverse_view[1].xyz;
    if sprite.size.z > 0.5 {
        // Lying on the water: the plane turned a quarter about +X, facing up.
        axis_x = vec3<f32>(1.0, 0.0, 0.0);
        axis_y = vec3<f32>(0.0, 0.0, -1.0);
    }
    let world = sprite.position.xyz + axis_x * rolled.x + axis_y * rolled.y;
    let view = camera.view * vec4<f32>(curved(world), 1.0);
    var out: Varyings;
    out.position = world_to_clip(world);
    out.uv = corner + 0.5;
    out.color = vec4<f32>(sprite.color.rgb / (grow * grow), sprite.color.a);
    out.world = vec4<f32>(world, -view.z);
    return out;
}

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    let texel = textureSample(map, map_sampler, in.uv);
    var alpha = texel.a * in.color.a;
    if batch.flags.x > 0.0 {
        alpha *= smoke_visibility(in.position.xy, in.world.w, 2.0, false);
        if batch.flags.z > 0.5 {
            alpha *= columns_through(in.position.xy, in.world.w, 2.0);
        }
    } else {
        // Flashes and flames skip the gun smoke but not the clouds or the splash columns in front (`smoke_visibility`
        // takes those too).
        alpha *= cloud_visibility(in.position.xy, in.world.w) * splashes_through(in.position.xy, in.world.w);
    }
    if alpha < batch.flags.y {
        discard;
    }
    // Every sprite here is light (a flash, a flame, a spark, an ember), in the sun's units: a fire's embers (`flags.z`) at
    // the fires' flames' lift, the rest at the flashes' (`flash_glow.x`).
    let lift = select(lighting.flash_glow.x, lighting.emission.y, batch.flags.z > 0.5);
    return vec4<f32>(effect_fog_additive(emitted_at(in.color.rgb * texel.rgb, lift), in.world.xyz), alpha);
}
