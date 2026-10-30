// The towns' light in the air at night (native/docs/terrain/night.md, "The towns' light in the sky"): the light the lit
// land sends up, at each point of a grid about the land's window (`naval_sky::aerial::GLOW_SIDE`² columns at
// `GLOW_LEVELS` heights from 31 m to 8 km), the air between the land and the point taken out. One thread a point sums
// every lit cell of the land's map (`naval_clouds::glow`), each a source at its middle whose light leaves it by
// Garstang's emission (Garstang 1986, "Model for artificial night-sky illumination", PASP 98: a share G(1 − F) of the
// lamps' light reflected by the ground as a Lambertian surface, a share F sent up directly, strongest toward the
// horizon, 0.554 F ψ⁴ of the zenith angle ψ). Written per unit radiance of wholly lit land: the fluence (x) and the net
// flux (yzw, world x, y up, z), which the aerial perspective scatters toward the eye and the clouds' march takes from
// below. Run when the map or the air changes, not each frame.

struct GlowParams {
    // The grid in the land's chart: x, z of its corner (m), a column's side (m), columns a side.
    frame: vec4<f32>,
    // The air the light crosses: the haze's extinction at the sea (per m) and its scale height (m), the atmosphere's
    // extinction at the sea as a view sees it (per m) and its scale height (m).
    air: vec4<f32>,
    // x the sources, y the grid's points (columns² × levels).
    counts: vec4<u32>,
}

@group(0) @binding(0) var<uniform> params: GlowParams;
// Each lit cell of the land's map: chart x, z of its middle (m), its light sent up per unit radiance of wholly lit
// land (π × its lit share × its area, m²), and the mean square distance across the cell (m²), which softens it near.
@group(0) @binding(1) var<storage, read> sources: array<vec4<f32>>;
// The sky's aerial buffer past the towns' light's header (`naval_sky::aerial::GLOW_OFFSET` + 256 bytes).
@group(0) @binding(2) var<storage, read_write> field: array<vec4<f32>>;

const PI: f32 = 3.14159265;
const FLOOR_M: f32 = 31.25;
const LEVELS: u32 = 9u;
// Garstang's ground reflectance G and the share F of the lamps' light sent up directly (his 0.15 and 0.1).
const GROUND_REFLECTANCE: f32 = 0.15;
const DIRECT_UP: f32 = 0.1;
// Sources a workgroup reads together.
const TILE: u32 = 64u;

var<workgroup> tile: array<vec4<f32>, 64>;

// The share of a source's light sent up toward zenith angle acos(`cos_psi`), per steradian (1 over the upper hemisphere).
fn emission(cos_psi: f32) -> f32 {
    let psi = acos(clamp(cos_psi, 0.0, 1.0));
    let psi2 = psi * psi;
    let reflected = 2.0 * GROUND_REFLECTANCE * (1.0 - DIRECT_UP);
    let up = GROUND_REFLECTANCE * (1.0 - DIRECT_UP) + DIRECT_UP;
    return (reflected * cos_psi + 0.554 * DIRECT_UP * psi2 * psi2) / (2.0 * PI * up);
}

// The mean over the heights 0 to `h` of an exponential profile of scale height `scale`, over its value at the sea.
fn thinned(h: f32, scale: f32) -> f32 {
    let x = h / max(scale, 1.0);
    return select((1.0 - exp(-x)) / x, 1.0 - 0.5 * x, x < 1e-3);
}

@compute @workgroup_size(64)
fn glow_main(@builtin(global_invocation_id) id: vec3<u32>, @builtin(local_invocation_index) local: u32) {
    let n = u32(params.frame.w);
    let index = id.x;
    let column = index % (n * n);
    let level = min(index / (n * n), LEVELS - 1u);
    let point = params.frame.xy + (vec2<f32>(f32(column % n), f32(column / n)) + 0.5) * params.frame.z;
    let h = FLOOR_M * exp2(f32(level));
    // The air's mean extinction along a path from the land up to the point (per m), slant or level alike.
    let extinction = params.air.x * thinned(h, params.air.y) + params.air.z * thinned(h, params.air.w);
    var fluence = 0.0;
    var flux = vec3<f32>(0.0);
    let count = params.counts.x;
    for (var base = 0u; base < count; base += TILE) {
        let k = base + local;
        tile[local] = select(vec4<f32>(0.0), sources[min(k, count - 1u)], k < count);
        workgroupBarrier();
        let last = min(TILE, count - base);
        for (var t = 0u; t < last; t++) {
            let s = tile[t];
            let d = point - s.xy;
            let r2 = dot(d, d) + h * h;
            let r = sqrt(r2);
            let depth = extinction * r;
            if depth < 16.0 {
                let e = s.z * emission(h / r) * exp(-depth) / (r2 + s.w);
                fluence += e;
                flux += e * vec3<f32>(d.x, h, d.y) / r;
            }
        }
        workgroupBarrier();
    }
    if index < params.counts.y {
        field[index] = vec4<f32>(fluence, flux);
    }
}
