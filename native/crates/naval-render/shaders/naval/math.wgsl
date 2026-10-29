// Small math every crate may need.
#define_import_path naval::math

// GLSL's `mod`: x − y·floor(x / y).
fn modulo(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

// `v` turned by the unit quaternion `q` (xyz the vector part, w the scalar).
fn rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}
