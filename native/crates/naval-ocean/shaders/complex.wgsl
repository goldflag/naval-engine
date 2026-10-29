// Complex arithmetic of the spectrum's passes (`fft.wgsl`, `resolve.wgsl`).
#define_import_path naval_ocean::complex

// e^{iθ}-rotation of two packed complex numbers (xy, zw).
fn rotate(value: vec4<f32>, c: f32, s: f32) -> vec4<f32> {
    return value * c + vec4<f32>(-value.y, value.x, -value.w, value.z) * s;
}
