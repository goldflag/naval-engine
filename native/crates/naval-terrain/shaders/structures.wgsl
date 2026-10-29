// The buildings' and the infrastructure's shared colour, hash and light helpers (`buildings.wgsl`, `infra.wgsl`).
#define_import_path naval_terrain::structures

const RECIPROCAL_PI: f32 = 0.3183098861837907;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// sRGB bytes to linear.
fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let v = c / 255.0;
    return select(pow((v + 0.055) / 1.055, vec3<f32>(2.4)), v / 12.92, v <= vec3<f32>(0.04045));
}

fn hash12(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q += dot(q, q.yx + 33.33);
    return fract((q.x + q.y) * q.x);
}

fn hash13(p: vec3<f32>) -> f32 {
    var q = fract(p * 0.1031);
    q += dot(q, q.zyx + 31.32);
    return fract((q.x + q.y) * q.z);
}

// 1 inside [lo, hi] of `t` (a cell's fraction), antialiased over `w` (its pixel footprint).
fn band(t: f32, lo: f32, hi: f32, w: f32) -> f32 {
    let e = max(w, 1e-4);
    return clamp((t - lo) / e + 0.5, 0.0, 1.0) * clamp((hi - t) / e + 0.5, 0.0, 1.0);
}

// The land's tone (`naval_terrain::surface::tone_down`), a little milder: buildings and ports stand on the land's palette.
fn tone(color: vec3<f32>) -> vec3<f32> {
    let l = dot(color, LUMA);
    let brightness = mix(0.9, 1.0, smoothstep(0.35, 0.6, l));
    return max(mix(vec3<f32>(l), color, 0.8), vec3<f32>(0.0)) * brightness;
}

fn f_schlick(f0: vec3<f32>, f90: f32, dot_vh: f32) -> vec3<f32> {
    let fresnel = exp2((dot_vh * -5.55473 - 6.98316) * dot_vh);
    return f0 * (1.0 - fresnel) + f90 * fresnel;
}

fn brdf_ggx(light: vec3<f32>, normal: vec3<f32>, view: vec3<f32>, f0: vec3<f32>, f90: f32, roughness: f32) -> vec3<f32> {
    let alpha = roughness * roughness;
    let half_dir = normalize(light + view);
    let dot_nl = clamp(dot(normal, light), 0.0, 1.0);
    let dot_nv = clamp(dot(normal, view), 1e-4, 1.0);
    let dot_nh = clamp(dot(normal, half_dir), 0.0, 1.0);
    let dot_vh = clamp(dot(view, half_dir), 0.0, 1.0);
    let a2 = alpha * alpha;
    let gv = dot_nl * sqrt(a2 + (1.0 - a2) * dot_nv * dot_nv);
    let gl = dot_nv * sqrt(a2 + (1.0 - a2) * dot_nl * dot_nl);
    let v = 0.5 / max(gv + gl, 1e-6);
    let denom = 1.0 - dot_nh * dot_nh * (1.0 - a2);
    let d = a2 / (denom * denom) * RECIPROCAL_PI;
    return f_schlick(f0, f90, dot_vh) * v * d;
}
