// Value noise in 2D, hashed per cell: the spray sprites' tearing (`sprite.wgsl`) and the blast sheets' (`blast.wgsl`).
#define_import_path naval_water_effects::value_noise

fn cell_hash(cell: vec2<f32>) -> f32 {
    return fract(sin(dot(cell, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = p - cell;
    let s = f * f * (3.0 - 2.0 * f);
    let top = mix(cell_hash(cell), cell_hash(cell + vec2<f32>(1.0, 0.0)), s.x);
    let bottom = mix(cell_hash(cell + vec2<f32>(0.0, 1.0)), cell_hash(cell + vec2<f32>(1.0, 1.0)), s.x);
    return mix(top, bottom, s.y);
}
