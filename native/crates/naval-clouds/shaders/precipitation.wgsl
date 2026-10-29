// The rain's and its splashes' shared helpers (`rain.wgsl`, `splashes.wgsl`).
#define_import_path naval_clouds::precipitation

// Smoothstep from `e0` to `e1`.
fn ramp(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Henyey–Greenstein phase for a scattering angle's cosine.
fn phase(cosine: f32, g: f32) -> f32 {
    return (1.0 - g * g) / (4.0 * 3.141592653589793) / pow(max(1.0 + g * g - cosine * 2.0 * g, 1e-4), 1.5);
}
