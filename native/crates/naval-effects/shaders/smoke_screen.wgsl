// The smoke screens' billows, where the gas sprites draw them (`gas_sprite.wgsl`) and where the beams light them
// (`air_light.wgsl`).
#define_import_path naval_effects::smoke_screen

// A point of a screen's puff in the billows' noise volume.
fn screen_coord(local: vec3<f32>, radii: vec3<f32>, age: f32, seed: f32) -> vec3<f32> {
    let period = clamp(radii.x * 1.5, 64.0, 240.0);
    let offset = vec3<f32>(fract(seed * 0.61803), -age * 0.004, fract(seed * 0.3779));
    return local * radii / period + offset;
}
