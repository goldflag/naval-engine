// A finished frame onto a linear extended-range surface (scRGB: Windows' HDR swap chains, which take linear light with
// 1 at 80 nits). The frame comes display-encoded, the sRGB curve carried on above SDR white: the post chain's space,
// which the page, the labels and the aim marks are composited in. It is decoded here and scaled by `white`, SDR white
// in the surface's units (the display's SDR white level over 80 nits), so SDR white lands where the desktop's does and
// the highlights above it go up to the display's peak. No web source: the browser game had no HDR output.

struct Present {
    // x: SDR white in the surface's units.
    white: vec4<f32>,
}

@group(0) @binding(0) var frame: texture_2d<f32>;
@group(0) @binding(1) var<uniform> present: Present;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
}

// The sRGB transfer's inverse carried on above 1 (IEC 61966-2-1's curve, as `naval::color::srgb_eotf`).
fn decode(encoded: vec3<f32>) -> vec3<f32> {
    let c = max(encoded, vec3<f32>(0.0));
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let encoded = textureLoad(frame, vec2<i32>(position.xy), 0).rgb;
    return vec4<f32>(decode(encoded) * present.white.x, 1.0);
}
