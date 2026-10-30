// The aerial-perspective volume as the sky's own passes read it: a buffer copy of the renderer's volume
// (naval_render::aerial, which every material samples at group 0), made each frame by the pass that fills it
// (naval-clouds' `aerial.wgsl`), with its dome slice after it. The dome takes the haze along each pixel's ray to the end
// of the air; the clouds' march takes the air in front of the clouds' front. Until the volume is filled the copy holds
// clear air. Read by hand (bilinear over a layer, linear between slices), as the hardware filters the volume.
#define_import_path naval_sky::aerial

#import naval::aerial_volume::{AerialSample, AERIAL_SLICES, aerial_slice_of, aerial_near_share}

// The volume's texels, then the dome slice's: two u32 a texel (half floats r | g, b | a), rows of `AERIAL_COLUMNS`,
// layers of `AERIAL_ROWS` rows: the in-scatter's `AERIAL_SLICES` layers, the opacity's (1 − transmittance), then the
// dome's one (rgb in-scatter, a opacity). Zeros are clear air.
@group(1) @binding(12) var<storage, read> aerial_texels: array<vec2<u32>>;

const AERIAL_COLUMNS: u32 = 32u;
const AERIAL_ROWS: u32 = 32u;

// A half float's value (`unpack2x16float` needs a capability not every device grants): subnormals, a few 1e-5 at most,
// read as 0.
fn half_value(bits: u32) -> f32 {
    let exponent = (bits >> 10u) & 0x1fu;
    if exponent == 0u {
        return 0.0;
    }
    return bitcast<f32>(((bits & 0x8000u) << 16u) | ((exponent + 112u) << 23u) | ((bits & 0x3ffu) << 13u));
}

fn aerial_texel(x: u32, y: u32, layer: u32) -> vec4<f32> {
    let packed = aerial_texels[x + AERIAL_COLUMNS * (y + AERIAL_ROWS * layer)];
    return vec4<f32>(
        half_value(packed.x & 0xffffu),
        half_value(packed.x >> 16u),
        half_value(packed.y & 0xffffu),
        half_value(packed.y >> 16u),
    );
}

// One layer at screen `uv` (0–1, y down), bilinear between texel centres, clamped at the edges.
fn aerial_layer(uv: vec2<f32>, layer: u32) -> vec4<f32> {
    let size = vec2<f32>(f32(AERIAL_COLUMNS), f32(AERIAL_ROWS));
    let at = clamp(clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * size - 0.5, vec2<f32>(0.0), size - 1.0);
    let low = vec2<u32>(floor(at));
    let high = min(low + 1u, vec2<u32>(AERIAL_COLUMNS - 1u, AERIAL_ROWS - 1u));
    let f = at - floor(at);
    let top = mix(aerial_texel(low.x, low.y, layer), aerial_texel(high.x, low.y, layer), f.x);
    let bottom = mix(aerial_texel(low.x, high.y, layer), aerial_texel(high.x, high.y, layer), f.x);
    return mix(top, bottom, f.y);
}

// The air between the eye and the point `distance` metres along the ray through screen `uv`.
fn aerial_along(uv: vec2<f32>, distance: f32) -> AerialSample {
    let slices = u32(AERIAL_SLICES);
    let k = aerial_slice_of(distance);
    let near = u32(floor(k));
    let far = min(near + 1u, slices - 1u);
    let f = k - floor(k);
    let inscatter = mix(aerial_layer(uv, near), aerial_layer(uv, far), f).rgb;
    let opacity = mix(aerial_layer(uv, slices + near), aerial_layer(uv, slices + far), f).rgb;
    let share = aerial_near_share(distance);
    return AerialSample(inscatter * share, 1.0 - opacity * share);
}

// The dome's radiance `color` through the haze along the ray through screen `uv`.
fn aerial_over_sky(color: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let haze = aerial_layer(uv, 2u * u32(AERIAL_SLICES));
    return color * (1.0 - haze.a) + haze.rgb;
}

// ---- The towns' light at night ------------------------------------------------------------------------------------
// After the volume and its dome slice the buffer holds the light the lit land sends up through the air
// (`naval_sky::aerial::GLOW_OFFSET`, filled by naval-clouds' `glow_field.wgsl`): a header (the grid's frame, the lit
// land's radiance), then at each point of a grid about the land's window (`GLOW_SIDE`² columns of `GLOW_LEVELS` heights)
// the fluence (x) and the net flux (yzw, world axes) of that light per unit radiance of wholly lit land, the air between
// the land and the point taken out. Raw f32 words, not half floats.

// The first texel pair after the volume's layers and the dome's (AERIAL_COLUMNS × AERIAL_ROWS × (2 × 32 + 1)).
const GLOW_FIRST: u32 = 66560u;
// The header's 256 bytes in texel pairs.
const GLOW_HEADER: u32 = 32u;
const GLOW_SIDE: u32 = 80u;
const GLOW_LEVELS: u32 = 9u;
const GLOW_FLOOR_M: f32 = 31.25;

fn glow_word(i: u32) -> vec4<f32> {
    let a = aerial_texels[i];
    let b = aerial_texels[i + 1u];
    return vec4<f32>(bitcast<f32>(a.x), bitcast<f32>(a.y), bitcast<f32>(b.x), bitcast<f32>(b.y));
}

// The lit land's radiance (linear rgb) and, in w, 1 while the towns' light burns: 0 by day, at sea, on the Medium
// setting and with the lights out, when nothing else here is read.
fn glow_light() -> vec4<f32> {
    return glow_word(GLOW_FIRST + 2u);
}

fn glow_texel(i: u32, j: u32, level: u32) -> vec4<f32> {
    return glow_word(GLOW_FIRST + GLOW_HEADER + 2u * ((level * GLOW_SIDE + j) * GLOW_SIDE + i));
}

fn glow_level(at: vec2<f32>, level: u32) -> vec4<f32> {
    let low = vec2<u32>(floor(at));
    let high = min(low + 1u, vec2<u32>(GLOW_SIDE - 1u));
    let f = at - floor(at);
    let top = mix(glow_texel(low.x, low.y, level), glow_texel(high.x, low.y, level), f.x);
    let bottom = mix(glow_texel(low.x, high.y, level), glow_texel(high.x, high.y, level), f.x);
    return mix(top, bottom, f.y);
}

// The towns' light at world point `xz` (m), `height` m above the sea: its fluence (x) and net flux (yzw: world x, y up,
// z) per unit radiance of wholly lit land, trilinear between the grid's points; nothing off the grid (fading over its
// last column), and above its top level falling as the height's inverse square.
fn glow_field(xz: vec2<f32>, height: f32) -> vec4<f32> {
    let frame = glow_word(GLOW_FIRST);
    if frame.z <= 0.0 {
        return vec4<f32>(0.0);
    }
    let n = frame.w;
    let at = (xz - frame.xy) / frame.z - 0.5;
    let inside = min(min(at.x, at.y), min(n - 1.0 - at.x, n - 1.0 - at.y));
    if inside <= -1.0 {
        return vec4<f32>(0.0);
    }
    let c = clamp(at, vec2<f32>(0.0), vec2<f32>(n - 1.0));
    let h = max(height, GLOW_FLOOR_M);
    let k = min(log2(h / GLOW_FLOOR_M), f32(GLOW_LEVELS - 1u));
    let low = u32(floor(k));
    let high = min(low + 1u, GLOW_LEVELS - 1u);
    var light = mix(glow_level(c, low), glow_level(c, high), k - floor(k));
    let top = GLOW_FLOOR_M * exp2(f32(GLOW_LEVELS - 1u));
    if h > top {
        light *= (top / h) * (top / h);
    }
    return light * clamp(1.0 + inside, 0.0, 1.0);
}
