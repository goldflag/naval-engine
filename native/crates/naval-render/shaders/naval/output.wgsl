// The end of the frame's chain, for every pass that writes the output (the display pass when it writes the output
// itself, edge smoothing's last stage, the upscale): the dither onto an 8- or 10-bit output, and the perceptual domain
// edge smoothing decides in once the frame holds HDR highlights. No web source: the browser game wrote 8-bit sRGB
// without a dither, and had no HDR output.
//
// **Dither.** A display-encoded value about to be stored in `levels` steps (255 for 8 bits) takes up to one step of
// triangular-distributed blue noise first (Gjøl, "Banding in Games", GDC 2016: the uniform mask remapped through the
// triangular distribution's inverse, so the noise's power does not follow the signal), so a sky or fog gradient
// shallower than a step a pixel shows as fine grain rather than bands. The mask is naval-render's own 64 × 64
// void-and-cluster one (`examples/blue_noise.rs`), offset by the golden ratio every sixtieth of a second of the
// presentation clock: a frame drawn again at the same time (a capture, a paused battle) gets the same noise, and the
// manual clock's frames are deterministic. Black and white stay as they are: the noise fades out within half a step of
// either end.
//
// **Perceptual domain.** The post chain's frames are display-encoded: sRGB-encoded values from 0 to SDR white (1), and
// with an HDR output the sRGB curve carried on above white to the display's headroom (1.83 at a headroom of 4). Edge
// smoothing's decisions (FXAA's and SMAA's edges, temporal AA's clipping and blend weights) are tuned for 0 to 1, so
// they are taken on `perceptual(x)`: the value itself up to white, and a highlight above it compressed toward 2
// (`2 - 1/x`, which meets the value with the same slope at white). An SDR frame never reaches past 1 and is decided
// on exactly as before.
#define_import_path naval::output

struct DitherParams {
    // x: this frame's offset into the mask (its presentation time's golden-ratio phase).
    phase: vec4<f32>,
}

// Group 0 of every pass that dithers: the mask (`R8Unorm`, 64²) and this frame's phase.
@group(0) @binding(8) var dither_mask: texture_2d<f32>;
@group(0) @binding(9) var<uniform> dither: DitherParams;

// Triangular noise in (-1, 1) at a pixel this frame.
fn dither_noise(pixel: vec2<f32>) -> f32 {
    let size = textureDimensions(dither_mask);
    let level = textureLoad(dither_mask, vec2<i32>(vec2<u32>(pixel) % size), 0).r;
    let u = fract((level * 255.0 + 0.5) / 256.0 + dither.phase.x);
    return select(1.0 - sqrt(2.0 - 2.0 * u), sqrt(2.0 * u) - 1.0, u < 0.5);
}

// `encoded` with up to a step of noise for an output of `levels` steps; none at black or white.
fn dither_encoded(encoded: vec3<f32>, pixel: vec2<f32>, levels: f32) -> vec3<f32> {
    let room = clamp(min(encoded, 1.0 - encoded) * (2.0 * levels), vec3<f32>(0.0), vec3<f32>(1.0));
    return encoded + dither_noise(pixel) * room / levels;
}

// A display-encoded colour as edge smoothing weighs it: as it is up to SDR white, an HDR highlight compressed toward 2.
fn perceptual(color: vec3<f32>) -> vec3<f32> {
    return select(color, 2.0 - 1.0 / color, color > vec3<f32>(1.0));
}

// `perceptual`'s inverse.
fn perceptual_inverse(value: vec3<f32>) -> vec3<f32> {
    return select(value, 1.0 / (2.0 - min(value, vec3<f32>(1.999))), value > vec3<f32>(1.0));
}
