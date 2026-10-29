// Smoke shaded by the smoke between it and the sun (the `shadows` look, on in the game while the sun's shadow maps are:
// `src/shadow.rs`). What the smoke materials read, per vertex, of the opacity shadow map `src/shadow.rs` draws each
// frame (`shadow_cast.wgsl`): a deep opacity map
// (Yuksel and Keyser 2008). Along the light, per texel, `shadow_front` holds where the first smoke begins and
// `shadow_slices` the optical depth gathered by each of eight depths behind that front, close together at the front
// (where the light is still strong and an error shows) and wider apart behind it.
//
// A puff already shades itself through the gas atlas (or its own material), so it reads the map on the plane that
// touches its sun side: what lies between that plane and the sun is other smoke, never the puff itself, and the sun
// side of a bank is lit as it was.
//
// Not a port: the browser game had no smoke shadows. Bound in the effects layer's scene reads (group 3, 40 to 43).
#define_import_path naval_effects::shadow

#import naval_effects::common::puff_sunlight

struct SmokeShadow {
    // World to the map: u and v (0–1) are `dot(p, axis.xyz) + axis.w`.
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    // Metres along the light (away from it) from the map's depth origin.
    axis_d: vec4<f32>,
    // x the depth the front map counts back from (it holds `far − front`, zero where there is no smoke), y and z the
    // slices' spacing (slice i lies `y·i + z·i²` metres behind the front), w 1 while the map holds smoke.
    params: vec4<f32>,
    // The casters' own: the map's width (m), how far the front map's discs reach past a puff (m), one over the
    // slices' width in texels; and w, how far depth along the light stands for depth under the sky (1 with the light
    // high, 0 with it on the horizon).
    casting: vec4<f32>,
}

@group(3) @binding(40) var<uniform> smoke_shadow: SmokeShadow;
@group(3) @binding(41) var shadow_front: texture_2d<f32>;
@group(3) @binding(42) var shadow_slices: texture_2d_array<f32>;
@group(3) @binding(43) var shadow_sampler: sampler;

const SHADOW_SLICES: f32 = 8.0;
// The sun's light on smoke under the deepest smoke: what reaches it round the bank off the sea, the sky and the lit
// smoke about it. Through one bank the light passes ten or thirty puffs, so without it the far side is black.
const DIRECT_DEEP: f32 = 0.18;
// Share of the rest that arrives by scattering through the smoke rather than straight, and how much slower that share
// dies with optical depth.
const SCATTERED: f32 = 0.22;
const SCATTERED_REACH: f32 = 0.25;
// The sky's light on smoke under the deepest smoke (the sky is all round it, so it never goes dark).
const SKY_DEEP: f32 = 0.8;

// Whether the look is on and the map holds smoke this frame.
fn shadows_on() -> bool {
    return smoke_shadow.params.w > 0.5;
}

// Optical depth of the smoke between the light and the plane `depth` metres along it, at map `uv`.
fn optical_before(uv: vec2<f32>, depth: f32) -> f32 {
    let front = smoke_shadow.params.x - textureSampleLevel(shadow_front, shadow_sampler, uv, 0.0).r;
    let behind = max(depth - front, 0.0);
    // The slice at or before `behind`: `a·i + b·i² = behind`.
    let a = smoke_shadow.params.y;
    let b = smoke_shadow.params.z;
    let i = floor((sqrt(a * a + 4.0 * b * behind) - a) / (2.0 * b));
    // Straight between the two slices in depth, as the former volume's third-axis filter did. Permanent 2D array
    // attachment views avoid wgpu 30's DX12 leak of temporary render-target descriptors for 3D depth slices.
    let along = (behind - (a * i + b * i * i)) / (a + b * (2.0 * i + 1.0));
    let slice = min(i + along, SHADOW_SLICES - 1.0);
    let first = i32(floor(slice));
    let next = min(first + 1, i32(SHADOW_SLICES) - 1);
    let before = textureSampleLevel(shadow_slices, shadow_sampler, uv, first, 0.0).r;
    let after = textureSampleLevel(shadow_slices, shadow_sampler, uv, next, 0.0).r;
    let optical = mix(before, after, fract(slice));
    // Out of the map's sides the shadows fade away rather than end.
    let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    return optical * clamp(edge * 16.0, 0.0, 1.0);
}

// What other smoke leaves of the light on a puff (or a plume's section) centred at `centre`, `radius` metres across
// its half, for the fragment at `surface` on it: x the share of the sun's (or moon's) light, y of the sky's.
fn puff_shade(surface: vec3<f32>, centre: vec3<f32>, radius: f32) -> vec2<f32> {
    if !shadows_on() {
        return vec2<f32>(1.0);
    }
    let uv = vec2<f32>(dot(surface, smoke_shadow.axis_u.xyz) + smoke_shadow.axis_u.w,
        dot(surface, smoke_shadow.axis_v.xyz) + smoke_shadow.axis_v.w);
    let plane = dot(centre, smoke_shadow.axis_d.xyz) + smoke_shadow.axis_d.w - radius;
    let optical = optical_before(uv, plane);
    let through = (1.0 - SCATTERED) * exp(-optical) + SCATTERED * exp(-optical * SCATTERED_REACH);
    let direct = mix(DIRECT_DEEP, 1.0, through);
    // Depth along a high light is depth under the sky too; along a light near the horizon it is not.
    let sky = mix(1.0, mix(SKY_DEEP, 1.0, exp(-optical * 0.5)), smoke_shadow.casting.w);
    return vec2<f32>(direct, sky);
}

// The light on a puff of smoke centred at `centre`, `radius` metres across its half, at the point `surface` on it, read
// per vertex: x the share of the sun's (or moon's) light, through the renderer's sun shadow maps (hulls, aircraft,
// land: `puff_sunlight`) and the smoke between it and the light (`puff_shade`); y the share of the sky's. (1, 1) where
// the pipeline is built without `EFFECT_SUN_SHADOW` (the occlusion pass, whose groups hold neither map).
fn puff_light(surface: vec3<f32>, centre: vec3<f32>, radius: f32) -> vec2<f32> {
#ifdef EFFECT_SUN_SHADOW
    let shade = puff_shade(surface, centre, radius);
    return vec2<f32>(puff_sunlight(surface, radius) * shade.x, shade.y);
#else
    return vec2<f32>(1.0);
#endif
}
