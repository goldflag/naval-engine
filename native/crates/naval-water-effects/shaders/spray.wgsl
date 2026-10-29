// Lit white water: ports `sprayRadiance` of the web game's `SprayMaterial.ts` and the uniforms of `EffectLighting.ts`.
// Spray is a cloud of drops and entrained air, so it scatters far more light than smoke: the sunlit face goes white,
// the shaded side stays a light blue-grey from the sky, and thin spray seen toward the sun glows. Native since: the sun's
// shadow maps (`naval::shadow`) take the beam off spray standing in a hull's shadow, or behind a splash column's core.
#define_import_path naval_water_effects::spray

#import naval_sky::environment::{SkyEnvironment, env_radiance}
#import naval::aerial::aerial_apply
#import naval::shadow::{sun_shadow, shadow_light_direction}

struct EffectLighting {
    // xyz toward the sun (or the brighter moon), w the scene's daylight 0–1 (unlit drops and mist take it without the
    // sky's environment).
    sun_direction: vec4<f32>,
    // Direct radiance scattered by spray (rgb), w 1 once the sky's environment is bound.
    direct: vec4<f32>,
    // Sky fill in the shade where the sky's environment is not bound (rgb), w lightning's flash (a multiple of the
    // sky's light).
    ambient: vec4<f32>,
    // x the full-resolution pixels per column march texel (1 or 2).
    column_sampling: vec4<f32>,
}

// The sky's environment (`WaterEffects::set_environment`; a black placeholder until the host binds one).
@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;

// Sky light reaching spray, relative to smoke: multiple scattering between drops fills the shade.
const SKY_GAIN: f32 = 2.2;
// The share of the sky's light smoke scatters back (naval-effects' `SKY_ALBEDO`): the albedo the direct light implies,
// π × 1.25 / 5.8.
const SKY_ALBEDO: f32 = 0.677;
// Luminance of the light on spray under a clear noon sky (direct about 1.19, the sky's about 0.3), where unlit drops
// and mist show their own colour.
const CLEAR_NOON_LIGHT: f32 = 1.4;
// The share of the authored daylight unlit drops and mist keep in a shadow without the sky's environment: the sky's
// share of a clear noon's light on them.
const UNLIT_SHADE: f32 = 0.3;

// The sky's light on spray from above, scaled as smoke's (naval-effects' `sky_fill`): the environment's, so it dims
// with dusk, night and overcast, lifted by lightning's flash; without the environment, the authored stand-in.
fn sky_fill(lighting: EffectLighting) -> vec3<f32> {
    if lighting.direct.w > 0.5 {
        let sky = env_radiance(sky_atlas, sky_sampler, sky_env, vec3<f32>(0.0, 1.0, 0.0), 1.0);
        return sky * SKY_ALBEDO * (1.0 + lighting.ambient.w);
    }
    return lighting.ambient.xyz;
}

// How much of the sun's (or moon's) beam reaches spray at `world`, 0–1, through the sun's shadow maps (near, wide and
// view maps: a hull, her superstructure, a splash column's dense core). The point is lifted `reach` metres up the beam
// first, so only what lies farther up the beam shades it, not spray's own neighbourhood. Spray is small beside the
// maps' texels and filter, so this is read in the vertex stage.
fn particle_sun(world: vec3<f32>, reach: f32) -> f32 {
    let toward = shadow_light_direction();
    return sun_shadow(world + toward * reach, toward);
}

// How bright the scene's light is on unlit drops and mist, 0–1: the sky's and the sun's (or moon's, `shadow` the share
// of its beam that reaches them) against a clear noon's, or the authored daylight without the sky's environment.
fn scene_light(lighting: EffectLighting, shadow: f32) -> f32 {
    if lighting.direct.w > 0.5 {
        let light = sky_fill(lighting) + lighting.direct.xyz * shadow;
        return min(dot(light, vec3<f32>(0.2126, 0.7152, 0.0722)) / CLEAR_NOON_LIGHT, 1.0);
    }
    return lighting.sun_direction.w * mix(UNLIT_SHADE, 1.0, shadow);
}

// The air between the eye and spray at `world` (`naval::aerial`: the atmosphere and the weather's haze, as every
// material takes it). For straight alpha over what is behind; every spray, splash and drop is drawn through it.
fn spray_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    return aerial_apply(color, world);
}

// Spray drawn a few pixels across (a splash 20 km off, even through the binoculars; any column a few kilometres off
// at the bare eye's lens): the tears, clumps and gaps between its jets and billows are finer than a pixel, but the
// water they stand for is a column many drops deep, which the eye sees whole. From `FAR_DETAIL_PX` pixels across a jet
// or a billow down to `FAR_SOLID_PX`, its fill closes (each shader's own) and its shaded side takes the sunlight many
// drops deep scatter round to it, `FAR_SCATTER` of the direct beam, as a cumulus's shaded side keeps half its sunlit
// side's light. Without it a column seen up-sun at range was the horizon's own brightness, and a falling salvo vanished
// (native/docs/water-effects.md, "At range").
const FAR_SOLID_PX: f32 = 3.0;
const FAR_DETAIL_PX: f32 = 16.0;
const FAR_SCATTER: f32 = 0.5;

// How far into the far rendering a strip or sprite is, 0 (resolved) to 1 (a few pixels across), from its own `u`
// across it. Call it where derivatives are defined: unconditionally at the top of a fragment shader.
fn far_share(across_uv: f32) -> f32 {
    let pixels = 1.0 / max(fwidth(across_uv), 1e-6);
    return 1.0 - smoothstep(FAR_SOLID_PX, FAR_DETAIL_PX, pixels);
}

// Radiance scattered by white spray with unit albedo, for world normal `n` seen along `to_camera`. `sunlit` (0–1) is the
// share of sunlight the rest of the column lets through, `shadow` (0–1) the share the sun's shadow maps let reach it
// (`particle_sun`), `sky` (0–1) the share of sky light, `thin` (0–1) how sparse the spray is: sparse spray seen
// toward the sun glows; and `far` (0–1) how few pixels it is drawn in ([`far_share`]).
fn spray_radiance(lighting: EffectLighting, n: vec3<f32>, to_camera: vec3<f32>, sunlit: f32, shadow: f32, sky: f32,
    thin: f32, far: f32) -> vec3<f32> {
    let sun = normalize(lighting.sun_direction.xyz);
    // Drops scatter light well past the terminator: wrap the sunlit face around the side.
    let diffuse = clamp((dot(n, sun) + 0.5) / 1.5, 0.0, 1.0);
    let toward = clamp(dot(-to_camera, sun), 0.0, 1.0);
    let glow = pow(toward, 4.0) * (thin * thin * 1.6 + 0.08) * (sunlit * 0.6 + 0.4);
    // Thick water keeps less of the sky than a thin veil: a column seen against the sun stays grey at its core.
    let fill = sky_fill(lighting) * SKY_GAIN * sky * (thin * 0.4 + 0.6) * (0.82 + n.y * 0.18);
    // Seen whole at range, the column's dense water lights its own shade.
    let lit = max(diffuse * sunlit, FAR_SCATTER * far * (1.0 - thin));
    return fill + lighting.direct.xyz * (lit + glow) * shadow;
}
