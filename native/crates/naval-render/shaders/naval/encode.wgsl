// A display-encoded colour as the chain's last stages write it to the output (`post/output.wgsl`, `post/film.wgsl`):
// dithered onto an 8- or 10-bit output (`DITHER_LEVELS`, `naval::output`), decoded after for an sRGB target, which
// encodes in hardware (`DECODE_SRGB`).
#define_import_path naval::encode

#import naval::color::srgb_eotf
#import naval::output::dither_encoded

fn encode(color: vec3<f32>, pixel: vec2<f32>) -> vec4<f32> {
    var out = color;
#ifdef DITHER_LEVELS
    out = dither_encoded(out, pixel, f32(#{DITHER_LEVELS}));
#endif
#ifdef DECODE_SRGB
    out = srgb_eotf(out);
#endif
    return vec4<f32>(out, 1.0);
}
