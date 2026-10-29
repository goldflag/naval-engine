// The splash columns' tiling noise volume (native, `src/columns.rs`): 64³ texels, every channel tiling at the volume's
// edge, filled once on the GPU before the first column is drawn. The lattices are hashed with integers, so every GPU
// fills the same texels.
// - r: billows (Perlin–Worley, cells of 4, 8 and 16 a period): round lumps of spray parted by creases, as a cloud's.
// - g: small billows (Worley, cells of 8, 16 and 32).
// - b: smooth noise (gradient noise, cells of 4, 8 and 16), read stretched along the water's flight for its streaks.
// - a: fine smooth noise (gradient noise, cells of 8, 16 and 32).

@group(0) @binding(0) var volume: texture_storage_3d<rgba8unorm, write>;

const SIZE: u32 = 64u;

fn scramble(x: u32) -> u32 {
    var h = x;
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    return h ^ (h >> 16u);
}

// A lattice cell's hash, the lattice wrapped at `cells` so the volume tiles.
fn cell_hash(cell: vec3<i32>, cells: i32, salt: u32) -> u32 {
    let c = vec3<u32>((cell % cells + cells) % cells);
    return scramble(c.x ^ scramble(c.y ^ scramble(c.z ^ salt)));
}

// Three fractions 0–1 from a hash.
fn unit3(h: u32) -> vec3<f32> {
    return vec3<f32>(f32(h & 1023u), f32((h >> 10u) & 1023u), f32((h >> 20u) & 1023u)) / 1023.0;
}

// Worley noise: 1 at a cell's point, falling to 0 a cell away: round bumps.
fn worley(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let q = p * f32(cells);
    let base = vec3<i32>(floor(q));
    let f = q - floor(q);
    var nearest = 9.0;
    for (var z = -1; z <= 1; z++) {
        for (var y = -1; y <= 1; y++) {
            for (var x = -1; x <= 1; x++) {
                let o = vec3<i32>(x, y, z);
                let point = vec3<f32>(o) + unit3(cell_hash(base + o, cells, salt));
                let d = point - f;
                nearest = min(nearest, dot(d, d));
            }
        }
    }
    return 1.0 - clamp(sqrt(nearest), 0.0, 1.0);
}

fn corner(base: vec3<i32>, o: vec3<i32>, f: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let g = normalize(unit3(cell_hash(base + o, cells, salt)) * 2.0 - 1.0 + vec3<f32>(1e-3, 0.0, 0.0));
    return dot(g, f - vec3<f32>(o));
}

// Gradient noise, about −0.8 to 0.8.
fn gradient_noise(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let q = p * f32(cells);
    let base = vec3<i32>(floor(q));
    let f = q - floor(q);
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let c000 = corner(base, vec3<i32>(0, 0, 0), f, cells, salt);
    let c100 = corner(base, vec3<i32>(1, 0, 0), f, cells, salt);
    let c010 = corner(base, vec3<i32>(0, 1, 0), f, cells, salt);
    let c110 = corner(base, vec3<i32>(1, 1, 0), f, cells, salt);
    let c001 = corner(base, vec3<i32>(0, 0, 1), f, cells, salt);
    let c101 = corner(base, vec3<i32>(1, 0, 1), f, cells, salt);
    let c011 = corner(base, vec3<i32>(0, 1, 1), f, cells, salt);
    let c111 = corner(base, vec3<i32>(1, 1, 1), f, cells, salt);
    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// Three octaves of gradient noise from `cells`, 0–1 about 0.5.
fn smooth_octaves(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    let n = gradient_noise(p, cells, salt) * 0.57 + gradient_noise(p, cells * 2, salt + 1u) * 0.29
        + gradient_noise(p, cells * 4, salt + 2u) * 0.14;
    return clamp(n * 0.75 + 0.5, 0.0, 1.0);
}

// Three octaves of Worley noise from `cells`, 0–1.
fn worley_octaves(p: vec3<f32>, cells: i32, salt: u32) -> f32 {
    return worley(p, cells, salt) * 0.625 + worley(p, cells * 2, salt + 1u) * 0.25 + worley(p, cells * 4, salt + 2u) * 0.125;
}

@compute @workgroup_size(4, 4, 4)
fn fill(@builtin(global_invocation_id) id: vec3<u32>) {
    if any(id >= vec3<u32>(SIZE)) {
        return;
    }
    let p = (vec3<f32>(id) + 0.5) / f32(SIZE);
    // Perlin–Worley (Schneider's): the gradient noise remapped into the Worley cells' range, so the billows keep the
    // Worley's round tops and the gradient noise's connected creases.
    let cells = worley_octaves(p, 4, 11u);
    let gradient = smooth_octaves(p, 4, 23u);
    let billows = (gradient - (cells - 1.0)) / (2.0 - cells);
    let small = worley_octaves(p, 8, 37u);
    let streaks = smooth_octaves(p, 4, 53u);
    let fine = smooth_octaves(p, 8, 71u);
    // Each channel stretched over 0–1: its 1st and 99th percentiles (as filled, read back on 2026-10-16) to 0.02 and
    // 0.98, so the shapes that read it can count on its range.
    let low = vec4<f32>(0.494, 0.22, 0.286, 0.294);
    let span = vec4<f32>(0.333, 0.53, 0.427, 0.42);
    let stretched = (vec4<f32>(billows, small, streaks, fine) - low) / span * 0.96 + 0.02;
    textureStore(volume, vec3<i32>(id), clamp(stretched, vec4<f32>(0.0), vec4<f32>(1.0)));
}
