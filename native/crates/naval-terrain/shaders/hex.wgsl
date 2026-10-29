// Hex tiling: a repeating tile drawn with no period (shared by the land's noise and photographs, `surface.wgsl`, and its
// ground materials, `materials.wgsl`).
//
// A tile repeated every few tens of metres shows its period from the air as a lattice over kilometres (the speck's
// 47 m, the grain's 210 m, the detail photographs'). Mikkelsen's hex tiling (JCGT 2022, "Practical Real-Time
// Hex-Tiling"): the plane cut into triangles about `1 / HEX_PER_TILE` of a tile across, each corner reading the tile at
// an offset of its own (a hash of the corner), the three readings blended by the point's place in its triangle. A
// value blended so keeps its contrast (divided by the weights' root sum of squares about the tile's mean, 0.5), and
// nothing repeats. Gradients come from the tile's own coordinates, so no seam shows where the offsets change.
#define_import_path naval_terrain::hex

const HEX_PER_TILE: f32 = 1.7;

// Two hashes in 0-1 of a lattice corner (pcg2d, Jarzynski and Olano 2020).
fn corner_offset(corner: vec2<f32>) -> vec2<f32> {
    var v = bitcast<vec2<u32>>(vec2<i32>(corner)) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return vec2<f32>(v >> vec2<u32>(8u)) / 16777216.0;
}

struct HexTaps {
    weights: vec3<f32>,
    a: vec2<f32>,
    b: vec2<f32>,
    c: vec2<f32>,
}

// The triangle `uv` (tile units) lies in: its three corners' offsets and the point's weights toward them.
fn hex_taps(uv: vec2<f32>) -> HexTaps {
    // The plane skewed so the triangles of the lattice are half cells of the unit grid.
    let p = uv * (HEX_PER_TILE * 3.4641016);
    let skewed = vec2<f32>(p.x - 0.57735027 * p.y, 1.15470054 * p.y);
    let base = floor(skewed);
    let f = fract(skewed);
    let z = 1.0 - f.x - f.y;
    var t: HexTaps;
    if z > 0.0 {
        t.weights = vec3<f32>(z, f.y, f.x);
        t.a = corner_offset(base);
        t.b = corner_offset(base + vec2<f32>(0.0, 1.0));
        t.c = corner_offset(base + vec2<f32>(1.0, 0.0));
    } else {
        t.weights = vec3<f32>(-z, 1.0 - f.y, 1.0 - f.x);
        t.a = corner_offset(base + vec2<f32>(1.0, 1.0));
        t.b = corner_offset(base + vec2<f32>(1.0, 0.0));
        t.c = corner_offset(base + vec2<f32>(0.0, 1.0));
    }
    // Sharpened toward the nearest corner, so most of a triangle reads one tile.
    let w = t.weights * t.weights * t.weights;
    t.weights = w / (w.x + w.y + w.z);
    return t;
}

// Three readings blended about the tiles' mean (0.5) with their contrast kept.
fn hex_blend(t: HexTaps, a: vec4<f32>, b: vec4<f32>, c: vec4<f32>) -> vec4<f32> {
    let w = t.weights;
    let spread = sqrt(dot(w, w));
    return (w.x * (a - 0.5) + w.y * (b - 0.5) + w.z * (c - 0.5)) / spread + 0.5;
}

// One round of the hash (half of `corner_offset`'s work): enough for the ground materials' offsets.
fn corner_offset_fast(corner: vec2<f32>) -> vec2<f32> {
    var v = bitcast<vec2<u32>>(vec2<i32>(corner)) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    return vec2<f32>(v >> vec2<u32>(8u)) / 16777216.0;
}

// `hex_taps` with the cheaper hash and no branch (`materials.wgsl`).
fn hex_taps_fast(uv: vec2<f32>) -> HexTaps {
    let p = uv * (HEX_PER_TILE * 3.4641016);
    let skewed = vec2<f32>(p.x - 0.57735027 * p.y, 1.15470054 * p.y);
    let base = floor(skewed);
    let f = skewed - base;
    let z = 1.0 - f.x - f.y;
    let upper = z <= 0.0;
    var t: HexTaps;
    t.weights = select(vec3<f32>(z, f.y, f.x), vec3<f32>(-z, 1.0 - f.y, 1.0 - f.x), upper);
    let o = select(vec2<f32>(0.0), vec2<f32>(1.0), upper);
    t.a = corner_offset_fast(base + o);
    t.b = corner_offset_fast(base + select(vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), upper));
    t.c = corner_offset_fast(base + select(vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), upper));
    let w = t.weights * t.weights * t.weights;
    t.weights = w / (w.x + w.y + w.z);
    return t;
}
