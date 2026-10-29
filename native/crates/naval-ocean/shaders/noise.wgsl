// Ports the web game's `ocean/noise.ts`: smooth value noise laid on the world, never repeating. An integer hash of lattice
// indices (no float hash inputs), interpolated with smoothstep weights. Every tiled pattern the ocean draws repeats;
// this is what varies them from one copy to the next (breaking groups, gusts, the windrows' layout).
#define_import_path naval_ocean::noise

// Integer hash of a lattice point to [0, 1). `seed_mix` is Math.imul(seed, 0x9e3779b9) >>> 0, precomputed by
// `noise_seed`.
fn noise_hash(x: i32, y: i32, seed_mix: u32) -> f32 {
    let h0 = (bitcast<u32>(x) * 0x27d4eb2du) ^ (bitcast<u32>(y) * 0x165667b1u) ^ seed_mix;
    let h1 = (h0 ^ (h0 >> 15u)) * 0x85ebca6bu;
    let h2 = (h1 ^ (h1 >> 13u)) * 0xc2b2ae35u;
    return f32(h2 ^ (h2 >> 16u)) * (1.0 / 4294967296.0);
}

fn noise_seed(seed: u32) -> u32 {
    return seed * 0x9e3779b9u;
}

// Gradient noise at `p`, in lattice cells (Perlin's: a unit gradient at a hashed angle at each lattice point, his
// quintic fade), mean 0, standard deviation 0.216. It is 0 at every lattice point and its bumps fall between them at
// no fixed place, where value noise's bumps sit on its lattice points, each the same shape: seen from the air its
// patches lay out as one oval stamped on a grid.
fn gradient_noise(p: vec2<f32>, seed: u32) -> f32 {
    let mixed = noise_seed(seed);
    let cell = floor(p);
    let f = p - cell;
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x = i32(cell.x);
    let y = i32(cell.y);
    let top = mix(gradient_at(x, y, mixed, f), gradient_at(x + 1, y, mixed, f - vec2<f32>(1.0, 0.0)), s.x);
    let bottom = mix(gradient_at(x, y + 1, mixed, f - vec2<f32>(0.0, 1.0)),
        gradient_at(x + 1, y + 1, mixed, f - vec2<f32>(1.0, 1.0)), s.x);
    return mix(top, bottom, s.y);
}

// Lattice point (`x`, `y`)'s gradient along `offset` from it.
fn gradient_at(x: i32, y: i32, mixed: u32, offset: vec2<f32>) -> f32 {
    let angle = noise_hash(x, y, mixed) * 6.2831855;
    return dot(vec2<f32>(cos(angle), sin(angle)), offset);
}

// Value noise at `p`, in lattice cells, 0–1 (mean ½, standard deviation 0.214).
fn value_noise(p: vec2<f32>, seed: u32) -> f32 {
    let mixed = noise_seed(seed);
    let cell = floor(p);
    let f = p - cell;
    let s = f * f * (3.0 - 2.0 * f);
    let x = i32(cell.x);
    let y = i32(cell.y);
    let top = mix(noise_hash(x, y, mixed), noise_hash(x + 1, y, mixed), s.x);
    let bottom = mix(noise_hash(x, y + 1, mixed), noise_hash(x + 1, y + 1, mixed), s.x);
    return mix(top, bottom, s.y);
}
