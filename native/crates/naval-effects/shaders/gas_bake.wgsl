// The gas atlas bake: a port of `GasAtlas.bake` (the web game's `GasAtlas.ts`), three compute kernels.
//
// `fill_noise` fills a tiling 64³ noise volume (cumulus-like billows dilated by gradient noise, finer billows,
// gradient noise). `bake` raymarches each of the atlas's 16 puffs along −Z in its 256² tile: for every texel it
// stores how much light the puff sends toward the viewer when lit from ±X and ±Y (`sides`), from the viewer's
// side and from behind, its optical depth ÷ GAS_DEPTH and the temperature the eye sees (`body`). `bake_detail`
// fills the 256² tiling map of small billows (normal xy, height, coarse variation) the sprites add up close.
// TSL helpers are transcribed as written: `hash` is three's PCG hash (tsl/math/Hash.js), `mod` is
// `x - y * floor(x / y)`.

const NOISE: u32 = 64u;
const GAS_TILE: u32 = 256u;
const GAS_GRID: u32 = 4u;
const GAS_ATLAS: u32 = 1024u;
const LOBES: u32 = 12u;
const DETAIL: u32 = 256u;
const VIEW_STEPS: u32 = 48u;
const SIGMA: f32 = 7.0;
const GAS_DEPTH: f32 = 12.0;

struct BakeParams {
    // The row of puffs this submission bakes (one row per submission, so a slow GPU is never held long).
    row: u32,
    spare_a: u32,
    spare_b: u32,
    spare_c: u32,
}

@group(0) @binding(0) var noise_out: texture_storage_3d<rgba8unorm, write>;
@group(0) @binding(1) var noise_in: texture_3d<f32>;
@group(0) @binding(2) var noise_sampler: sampler;
@group(0) @binding(3) var<uniform> lobes: array<vec4<f32>, 192>;
@group(0) @binding(4) var<uniform> params: BakeParams;
@group(0) @binding(5) var sides_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var body_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(7) var detail_out: texture_storage_2d<rgba8unorm, write>;

fn tsl_mod(x: f32, y: f32) -> f32 {
    return x - y * floor(x / y);
}

fn tsl_mod3(x: vec3<f32>, y: f32) -> vec3<f32> {
    return x - y * floor(x / y);
}

fn tsl_mod2(x: vec2<f32>, y: f32) -> vec2<f32> {
    return x - y * floor(x / y);
}

// three's `hash`: PCG on the seed converted to u32, in [0, 1).
fn hash(seed: f32) -> f32 {
    let state = u32(seed) * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    let result = (word >> 22u) ^ word;
    return f32(result) * (1.0 / 4294967296.0);
}

// Three hashes in [0, 1) of an integer lattice point.
fn hash3(cell: vec3<f32>, period: f32, seed: f32) -> vec3<f32> {
    let n = (cell.x + cell.y * period + cell.z * (period * period)) * 3.0 + seed * 7919.0;
    return vec3<f32>(hash(n), hash(n + 1.0), hash(n + 2.0));
}

// Tiling Worley F1 "billow" at `p` in [0, 1)³: 1 at a feature point, 0 a cell or more away.
fn billow(p: vec3<f32>, cells: f32, seed: f32) -> f32 {
    let q = p * cells;
    let cell = floor(q);
    let f = fract(q);
    var nearest = 9.0;
    for (var i = 0; i < 27; i++) {
        let offset = vec3<f32>(f32(i % 3), f32((i / 3) % 3), f32(i / 9)) - 1.0;
        let point = offset + hash3(tsl_mod3(cell + offset + cells, cells), cells, seed) - f;
        nearest = min(nearest, dot(point, point));
    }
    return max(1.0 - sqrt(nearest), 0.0);
}

// Tiling 2D Worley F1 billow.
fn billow2(p: vec2<f32>, cells: f32, seed: f32) -> f32 {
    let q = p * cells;
    let cell = floor(q);
    let f = fract(q);
    var nearest = 9.0;
    for (var i = 0; i < 9; i++) {
        let offset = vec2<f32>(f32(i % 3), f32(i / 3)) - 1.0;
        let wrapped = tsl_mod2(cell + offset + cells, cells);
        let n = (wrapped.x + wrapped.y * cells) * 2.0 + seed * 7919.0;
        let point = offset + vec2<f32>(hash(n), hash(n + 1.0)) - f;
        nearest = min(nearest, dot(point, point));
    }
    return max(1.0 - sqrt(nearest), 0.0);
}

fn corner(cell: vec3<f32>, f: vec3<f32>, c: vec3<f32>, period: f32, seed: f32) -> f32 {
    let g = normalize(hash3(tsl_mod3(cell + c, period), period, seed) * 2.0 - 1.0);
    return dot(g, f - c);
}

// Tiling gradient noise, about −1 … 1, with `period` cells per repeat.
fn gradient(p: vec3<f32>, period: f32, seed: f32) -> f32 {
    let q = p * period;
    let cell = floor(q);
    let f = fract(q);
    let s = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    let x00 = mix(corner(cell, f, vec3<f32>(0.0, 0.0, 0.0), period, seed), corner(cell, f, vec3<f32>(1.0, 0.0, 0.0), period, seed), s.x);
    let x10 = mix(corner(cell, f, vec3<f32>(0.0, 1.0, 0.0), period, seed), corner(cell, f, vec3<f32>(1.0, 1.0, 0.0), period, seed), s.x);
    let x01 = mix(corner(cell, f, vec3<f32>(0.0, 0.0, 1.0), period, seed), corner(cell, f, vec3<f32>(1.0, 0.0, 1.0), period, seed), s.x);
    let x11 = mix(corner(cell, f, vec3<f32>(0.0, 1.0, 1.0), period, seed), corner(cell, f, vec3<f32>(1.0, 1.0, 1.0), period, seed), s.x);
    return mix(mix(x00, x10, s.y), mix(x01, x11, s.y), s.z) * 1.6;
}

@compute @workgroup_size(64)
fn fill_noise(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x;
    if i >= NOISE * NOISE * NOISE {
        return;
    }
    let x = i % NOISE;
    let y = (i / NOISE) % NOISE;
    let z = i / (NOISE * NOISE);
    let p = (vec3<f32>(f32(x), f32(y), f32(z)) + 0.5) / f32(NOISE);
    // R: cumulus-like billows dilated by gradient noise; G, B: finer billows; A: gradient noise.
    let cells = billow(p, 4.0, 1.0) * 0.625 + billow(p, 8.0, 2.0) * 0.25 + billow(p, 16.0, 3.0) * 0.125;
    let perlin = clamp((gradient(p, 4.0, 4.0) + gradient(p, 8.0, 5.0) * 0.5) * 0.5 + 0.5, 0.0, 1.0);
    let shape = clamp((cells + perlin * (1.0 - cells) - 0.45) / 0.5, 0.0, 1.0);
    textureStore(noise_out, vec3<u32>(x, y, z), vec4<f32>(shape, billow(p, 8.0, 6.0), billow(p, 16.0, 7.0),
        clamp(gradient(p, 4.0, 8.0) * 0.5 + 0.5, 0.0, 1.0)));
}

fn read_noise(q: vec3<f32>) -> vec4<f32> {
    return textureSampleLevel(noise_in, noise_sampler, q, 0.0);
}

// (density, temperature) at `p` in the unit sphere of puff `variant`; `fine` adds the smallest billows.
fn gas(p: vec3<f32>, variant: i32, fine: f32) -> vec2<f32> {
    // Signed distance to the smoothly joined lobes: negative inside.
    var surface = 4.0;
    for (var i = 0u; i < LOBES; i++) {
        let lobe = lobes[u32(variant) * LOBES + i];
        let d = length(p - lobe.xyz) - lobe.w;
        let h = max(0.14 - abs(d - surface), 0.0) / 0.14;
        surface = min(surface, d) - h * h * 0.035;
    }
    let v = f32(variant);
    let offset = fract(vec3<f32>(v * 0.3719, v * 0.6173, v * 0.1301));
    let q = p * 0.42 + offset;
    let n = read_noise(q);
    // Billows swell the lobes into cauliflower; finer ones crowd their rims.
    var shape = -surface + (n.r - 0.5) * 0.34 + (n.g - 0.47) * 0.17 + (n.b - 0.47) * 0.1;
    if fine > 0.5 {
        let small = read_noise(q * 2.1 + 0.37);
        shape += (small.b - 0.47) * 0.08 + (small.g - 0.47) * 0.05;
    }
    let density = smoothstep(0.0, 0.06, shape) * mix(0.5, 1.0, n.b) * (1.0 - smoothstep(0.93, 1.0, length(p)));
    // Hottest deep inside and in cells that burn on while the gas around them has cooled to smoke.
    let temperature = clamp(-surface * 1.3 + (n.g - 0.45) * 0.9 + (n.a - 0.5) * 0.4 + 0.25, 0.0, 1.0);
    return vec2<f32>(density, temperature);
}

// Light reaching `p` from along `toward`: single scattering plus a softer share for light scattered on the way.
fn lit(p: vec3<f32>, toward: vec3<f32>, variant: i32) -> f32 {
    let steps = array<f32, 8>(0.03, 0.05, 0.08, 0.12, 0.17, 0.24, 0.36, 0.5);
    var depth = 0.0;
    var travelled = 0.0;
    for (var i = 0; i < 8; i++) {
        let step = steps[i];
        let at = p + toward * (travelled + step / 2.0);
        depth += gas(at, variant, 0.0).x * step;
        travelled += step;
    }
    let tau = depth * SIGMA;
    return exp(-tau) * 0.8 + exp(-tau * 0.25) * 0.2;
}

@compute @workgroup_size(64)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= GAS_ATLAS * GAS_TILE {
        return;
    }
    let i = id.x + params.row * (GAS_ATLAS * GAS_TILE);
    let texel = vec2<u32>(i % GAS_ATLAS, i / GAS_ATLAS);
    let tile = texel / GAS_TILE;
    let variant = i32(tile.y * GAS_GRID + tile.x);
    // Tile texels to the puff's unit disc, +Y up; the eye looks along −Z.
    let xy = (vec2<f32>(texel - tile * GAS_TILE) + 0.5) / f32(GAS_TILE) * 2.0 - 1.0;
    let r2 = dot(xy, xy);
    var sides = vec4<f32>(0.0);
    var body = vec4<f32>(0.0);
    if r2 < 0.999 {
        let half = sqrt(1.0 - r2);
        let stride = half * 2.0 / f32(VIEW_STEPS);
        var transmittance = 1.0;
        var light = vec4<f32>(0.0);
        var front_back = vec2<f32>(0.0);
        var heat = 0.0;
        var heat_weight = 0.0;
        var depth = 0.0;
        for (var step = 0u; step < VIEW_STEPS; step++) {
            let p = vec3<f32>(xy, half - stride * (f32(step) + 0.5));
            let sample = gas(p, variant, 1.0);
            if sample.x > 0.002 {
                let tau = sample.x * SIGMA * stride;
                // Light matters only while the eye still sees this deep; the optical depth adds up through the whole puff.
                if transmittance > 0.003 {
                    let alpha = 1.0 - exp(-tau);
                    let weight = alpha * transmittance;
                    light += vec4<f32>(lit(p, vec3<f32>(1.0, 0.0, 0.0), variant), lit(p, vec3<f32>(-1.0, 0.0, 0.0), variant),
                        lit(p, vec3<f32>(0.0, 1.0, 0.0), variant), lit(p, vec3<f32>(0.0, -1.0, 0.0), variant)) * weight;
                    // Light from the viewer's side reaches this gas through what the eye looks through.
                    front_back += vec2<f32>(transmittance, lit(p, vec3<f32>(0.0, 0.0, -1.0), variant)) * weight;
                    // Hot gas shows through thin soot: weigh what lies deeper a little more than its light.
                    let seen = alpha * pow(transmittance, 0.4);
                    heat += sample.y * seen;
                    heat_weight += seen;
                }
                depth += tau;
                transmittance *= exp(-tau);
            }
        }
        let opacity = 1.0 - transmittance;
        let inverse = 1.0 / max(opacity, 1e-4);
        sides = clamp(light * inverse, vec4<f32>(0.0), vec4<f32>(1.0));
        body = clamp(vec4<f32>(front_back.x * inverse, front_back.y * inverse * 0.5, depth / GAS_DEPTH,
            heat / max(heat_weight, 1e-4)), vec4<f32>(0.0), vec4<f32>(1.0));
    }
    textureStore(sides_out, texel, sides);
    textureStore(body_out, texel, body);
}

// Small billows at three octaves.
fn detail_height(p: vec2<f32>) -> f32 {
    return billow2(p, 6.0, 21.0) * 0.5 + billow2(p, 12.0, 22.0) * 0.32 + billow2(p, 24.0, 23.0) * 0.18;
}

@compute @workgroup_size(64)
fn bake_detail(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= DETAIL * DETAIL {
        return;
    }
    let x = id.x % DETAIL;
    let y = id.x / DETAIL;
    let p = (vec2<f32>(f32(x), f32(y)) + 0.5) / f32(DETAIL);
    let e = 1.0 / f32(DETAIL);
    let dx = detail_height(p + vec2<f32>(e, 0.0)) - detail_height(p - vec2<f32>(e, 0.0));
    let dy = detail_height(p + vec2<f32>(0.0, e)) - detail_height(p - vec2<f32>(0.0, e));
    let normal = normalize(vec3<f32>(dx * -8.0, dy * -8.0, 1.0));
    textureStore(detail_out, vec2<u32>(x, y), clamp(vec4<f32>(normal.xy * 0.5 + 0.5, detail_height(p), billow2(p, 3.0, 24.0)),
        vec4<f32>(0.0), vec4<f32>(1.0)));
}
