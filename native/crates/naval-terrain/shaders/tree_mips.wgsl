// The impostor atlas's mip levels (`tree_bake.wgsl` writes the base): each texel of level k + 1 from the 2 x 2 of
// level k beneath it, one workgroup a frame of a species' layer. The colour layers are premultiplied (sRGB-encoded),
// so they average in premultiplied linear light and are encoded again; the normal and depth average over the covered
// texels only (weighted by the colour layer's coverage), so an empty texel's envelope never bends a crown's edge, and
// keep the envelope where nothing is covered; the material (premultiplied) keeps the one more of the coverage is. A frame's texels halve whole at every level made (96
// down to 3), so no level mixes two frames. The leaf sprites' layers (the last) come with their levels from the CPU.
//
// Each level keeps its frame's coverage at the edge the impostors' alpha test cuts at (Castaño, "Computing alpha
// mipmaps", 2010): averaging thins a crown's coverage, so far crowns shrank and broke into speckle. A level's coverage
// is scaled so the share of the frame's texels at or past `EDGE` is the base level's, the scale found by bisection
// (the base's share counted as the first level is made, and kept in `targets` for the levels after it).
#import naval_terrain::impostor::{encode_normal, decode_normal, LEAF_LAYERS, IMPOSTOR_FRAMES, FRAME_TEXELS}

// `trees.wgsl`'s edge, and the least coverage a texel is written with that passes it as a byte.
const EDGE: f32 = 0.45;
const EDGE_WRITTEN: f32 = 114.5 / 255.0;
// How far each texel's coverage is spread about its own (a hash of its place): texels of one coverage (most of a
// level's, the base holding only quarters) would cross the edge all at once, and no scale could match the base.
const TIE_SPREAD: f32 = 0.04;
const LANES: u32 = 256u;
// A frame's texels a lane takes at most (the first level made: 48² over 256 lanes).
const MOST_TEXELS: u32 = 9u;
// The bisection's steps, and the scales it searches between.
const STEPS: u32 = 12u;
const LEAST_SCALE: f32 = 0.5;
const MOST_SCALE: f32 = 4.0;

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((max(c, vec3<f32>(0.0)) + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@group(0) @binding(0) var source: texture_2d_array<f32>;
@group(0) @binding(1) var level_out: texture_storage_2d_array<rgba8unorm, write>;
// Each species' frames' share of texels at or past the edge at the base level.
@group(0) @binding(2) var<storage, read_write> targets: array<f32>;

var<workgroup> base_count: atomic<u32>;
var<workgroup> counts: array<atomic<u32>, STEPS>;
var<workgroup> ends: array<atomic<u32>, 2>;

// A texel's own spread of its coverage (1 about `TIE_SPREAD`).
fn spread(layer: i32, at: vec2<i32>) -> f32 {
    var h = (bitcast<u32>(at.x) * 73856093u) ^ (bitcast<u32>(at.y) * 19349663u) ^ (bitcast<u32>(layer) * 83492791u);
    h = (h ^ (h >> 13u)) * 1274126177u;
    h = h ^ (h >> 16u);
    return 1.0 + TIE_SPREAD * (f32(h & 1023u) / 1023.0 - 0.5);
}

// A texel's coverage at the level made, spread: the mean of the four beneath it.
fn coverage_at(layer: i32, at: vec2<i32>) -> f32 {
    let base = at * 2;
    var sum = 0.0;
    for (var k = 0; k < 4; k += 1) {
        sum += textureLoad(source, base + vec2<i32>(k & 1, k >> 1u), layer, 0).a;
    }
    return sum * 0.25 * spread(layer, at);
}

// A texel of the level made, its coverage spread and scaled by `scale`: the colour layer's (premultiplied, encoded)
// and the normal layer's.
fn write_texel(layer: i32, models: i32, at: vec2<i32>, scale: f32) {
    let base = at * 2;
    var color = vec4<f32>(0.0);
    var normal = vec3<f32>(0.0);
    var envelope = vec3<f32>(0.0);
    var depth = 0.0;
    var plain = 0.0;
    var weight = 0.0;
    var leaf_weight = 0.0;
    var leaf_packed = 0.0;
    var bark_packed = 0.0;
    for (var k = 0; k < 4; k += 1) {
        let beneath = base + vec2<i32>(k & 1, k >> 1u);
        let c = textureLoad(source, beneath, layer, 0);
        let n = textureLoad(source, beneath, models + layer, 0);
        // Premultiplied linear light: the encoded colour over its coverage, decoded, times the coverage.
        color += vec4<f32>(srgb_to_linear(c.rgb / max(c.a, 1e-4)) * c.a, c.a) * 0.25;
        let decoded = decode_normal(n.xy);
        normal += decoded * c.a;
        depth += n.z * c.a;
        weight += c.a;
        envelope += decoded;
        plain += n.z * 0.25;
        let leaf = n.w > 0.5 * c.a;
        leaf_weight += select(0.0, c.a, leaf);
        leaf_packed += select(0.0, n.w, leaf);
        bark_packed += select(n.w, 0.0, leaf);
    }
    let coverage = min(color.a * spread(layer, at) * scale, 1.0);
    let encoded = linear_to_srgb(color.rgb / max(color.a, 1e-4)) * coverage;
    textureStore(level_out, at, layer, vec4<f32>(encoded, coverage));
    let bark_weight = weight - leaf_weight;
    let packed = select(bark_packed / max(bark_weight, 1e-4), leaf_packed / max(leaf_weight, 1e-4), leaf_weight >= bark_weight);
    if weight > 1e-4 {
        let n = encode_normal(normalize(normal + vec3<f32>(0.0, 1e-4, 0.0)));
        textureStore(level_out, at, models + layer, vec4<f32>(n, depth / weight, packed * coverage));
    } else {
        let n = encode_normal(normalize(envelope + vec3<f32>(0.0, 1e-4, 0.0)));
        textureStore(level_out, at, models + layer, vec4<f32>(n, plain, 0.0));
    }
}

@compute @workgroup_size(256)
fn downsample(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let frames = u32(IMPOSTOR_FRAMES);
    let n = textureDimensions(level_out).x / frames;
    let texels = n * n;
    let origin = vec2<u32>(group.x % frames, group.x / frames) * n;
    let layer = i32(group.z);
    let models = i32((textureNumLayers(source) - LEAF_LAYERS) / 2u);
    let slot = group.z * frames * frames + group.x;
    // The first level made reads the base: its share at the edge, counted here.
    let first = textureDimensions(source).x == u32(IMPOSTOR_FRAMES * FRAME_TEXELS);
    if first {
        let m = n * 2u;
        var mine = 0u;
        for (var i = lane; i < m * m; i += LANES) {
            let at = vec2<i32>(origin * 2u + vec2<u32>(i % m, i / m));
            mine += select(0u, 1u, textureLoad(source, at, layer, 0).a >= EDGE);
        }
        atomicAdd(&base_count, mine);
    }
    workgroupBarrier();
    let goal = select(targets[slot], f32(atomicLoad(&base_count)) / f32(4u * texels), first);
    // This lane's texels' coverage at this level.
    var coverage: array<f32, MOST_TEXELS>;
    var held = 0u;
    for (var i = lane; i < texels; i += LANES) {
        coverage[held] = coverage_at(layer, vec2<i32>(origin + vec2<u32>(i % n, i / n)));
        held += 1u;
    }
    // The least scale whose share at the edge reaches the goal.
    var low = LEAST_SCALE;
    var high = MOST_SCALE;
    for (var step = 0u; step < STEPS; step += 1u) {
        let mid = 0.5 * (low + high);
        var mine = 0u;
        for (var j = 0u; j < held; j += 1u) {
            mine += select(0u, 1u, coverage[j] * mid >= EDGE_WRITTEN);
        }
        atomicAdd(&counts[step], mine);
        workgroupBarrier();
        let short = f32(atomicLoad(&counts[step])) < goal * f32(texels);
        low = select(low, mid, short);
        high = select(mid, high, short);
    }
    // Texels of one coverage cross the edge together: the scale either side of the jump the bisection closed on,
    // whichever comes nearer the goal.
    var below = 0u;
    var above = 0u;
    for (var j = 0u; j < held; j += 1u) {
        below += select(0u, 1u, coverage[j] * low >= EDGE_WRITTEN);
        above += select(0u, 1u, coverage[j] * high >= EDGE_WRITTEN);
    }
    atomicAdd(&ends[0], below);
    atomicAdd(&ends[1], above);
    workgroupBarrier();
    let want = goal * f32(texels);
    let nearer = select(high, low, abs(f32(atomicLoad(&ends[0])) - want) < abs(f32(atomicLoad(&ends[1])) - want));
    let scale = select(nearer, 1.0, goal <= 0.0);
    for (var i = lane; i < texels; i += LANES) {
        write_texel(layer, models, vec2<i32>(origin + vec2<u32>(i % n, i / n)), scale);
    }
    if first && lane == 0u {
        targets[slot] = goal;
    }
}
