// MaterialX noise's shared helpers, as three.js r185 writes them (`three/src/nodes/materialx/lib/mx_noise.js`:
// `mx_rotl32`, `mx_negate_if`, `mx_gradient_float_1`, `mx_fade`), transpiled from MaterialX's `mx_noise.glsl`: the
// ships' (`naval_ships::noise`), the sky's (`naval_sky::noise`) and the sea's (`naval_ocean::mx_noise`) Perlin noise.
#define_import_path naval::mx_noise

fn mx_rotl32(x: u32, k: u32) -> u32 {
    return (x << k) | (x >> (32u - k));
}

fn mx_negate_if(value: f32, b: bool) -> f32 {
    return select(value, -value, b);
}

fn mx_gradient_float3(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = hash & 15u;
    let u = select(y, x, h < 8u);
    let v = select(select(z, x, h == 12u || h == 14u), y, h < 4u);
    return mx_negate_if(u, (h & 1u) != 0u) + mx_negate_if(v, (h & 2u) != 0u);
}

fn mx_fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}
