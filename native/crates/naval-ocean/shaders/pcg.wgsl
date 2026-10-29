// The underwater particles' hash (`underwater_motes.wgsl`, `underwater_bubbles.wgsl`).
#define_import_path naval_ocean::pcg

// PCG, three draws in [0, 1).
fn hash3(index: u32) -> vec3<f32> {
    var state = index * 747796405u + 2891336453u;
    var out = vec3<f32>(0.0);
    for (var i = 0; i < 3; i++) {
        state = state * 747796405u + 2891336453u;
        let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
        out[i] = f32((word >> 22u) ^ word) / 4294967296.0;
    }
    return out;
}
