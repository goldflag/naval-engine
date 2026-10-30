// three.js r185 physical shading, as `MeshStandardNodeMaterial` lights a surface: copied from naval-ships' `bsdf.wgsl`
// (this crate builds without it), its import path renamed.
//
// Ports `three/src/nodes/functions/BSDF/` (`F_Schlick`, `V_GGX_SmithCorrelated`, `D_GGX`, `BRDF_GGX`,
// `BRDF_GGX_Multiscatter`, `DFGLUT`, `BRDF_Lambert`), `PhysicalLightingModel.js` (`direct`,
// `indirectDiffuse`, `indirectSpecular` with `computeMultiscattering`) and
// `functions/material/getRoughness.js` + `getGeometryRoughness.js`. Everything is evaluated in world
// space except the geometry roughness, which three measures on the view-space normal.
#define_import_path naval_vessel::bsdf

#import naval::pbr::{f_schlick, v_ggx_smith_correlated, d_ggx}

const PI: f32 = 3.141592653589793;
const RECIPROCAL_PI: f32 = 0.3183098861837907;
// three's `EPSILON` (MathNode.js).
const EPSILON: f32 = 1e-6;

// The ship's surface as the lighting sees it (three's PropertyNodes).
struct Surface {
    diffuse_color: vec3<f32>,
    metalness: f32,
    // After `getRoughness` (clamped to 0.0525, plus geometry roughness, at most 1).
    roughness: f32,
    // World-space shading normal (three's `normalWorld`; `normalView` rotated by the camera).
    normal: vec3<f32>,
    // World-space unit vector from the surface to the eye (three's `positionViewDirection`).
    view: vec3<f32>,
}

struct ReflectedLight {
    direct_diffuse: vec3<f32>,
    direct_specular: vec3<f32>,
    indirect_diffuse: vec3<f32>,
    indirect_specular: vec3<f32>,
}

fn brdf_ggx(light: vec3<f32>, normal: vec3<f32>, view: vec3<f32>, f0: vec3<f32>, f90: f32, roughness: f32) -> vec3<f32> {
    let alpha = roughness * roughness;
    let half_dir = normalize(light + view);
    let dot_nl = clamp(dot(normal, light), 0.0, 1.0);
    let dot_nv = clamp(dot(normal, view), 0.0, 1.0);
    let dot_nh = clamp(dot(normal, half_dir), 0.0, 1.0);
    let dot_vh = clamp(dot(view, half_dir), 0.0, 1.0);
    let f = f_schlick(f0, f90, dot_vh);
    let v = v_ggx_smith_correlated(alpha, dot_nl, dot_nv);
    let d = d_ggx(alpha, dot_nh);
    return f * v * d;
}

// Scale and bias to F0 of the split-sum environment BRDF (three's 16 × 16 `DFG_LUT`).
fn dfg(lut: texture_2d<f32>, lut_sampler: sampler, roughness: f32, dot_nv: f32) -> vec2<f32> {
    return textureSampleLevel(lut, lut_sampler, vec2<f32>(roughness, dot_nv), 0.0).rg;
}

fn brdf_ggx_multiscatter(
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    light: vec3<f32>,
    normal: vec3<f32>,
    view: vec3<f32>,
    f0: vec3<f32>,
    f90: f32,
    roughness: f32,
) -> vec3<f32> {
    let single_scatter = brdf_ggx(light, normal, view, f0, f90, roughness);
    let dot_nl = clamp(dot(normal, light), 0.0, 1.0);
    let dot_nv = clamp(dot(normal, view), 0.0, 1.0);
    let dfg_v = dfg(lut, lut_sampler, roughness, dot_nv);
    let dfg_l = dfg(lut, lut_sampler, roughness, dot_nl);
    let fss_ess_v = f0 * dfg_v.x + f90 * dfg_v.y;
    let fss_ess_l = f0 * dfg_l.x + f90 * dfg_l.y;
    let ess_v = dfg_v.x + dfg_v.y;
    let ess_l = dfg_l.x + dfg_l.y;
    let ems_v = 1.0 - ess_v;
    let ems_l = 1.0 - ess_l;
    let f_avg = f0 + (1.0 - f0) * 0.047619;
    let f_ms = fss_ess_v * fss_ess_l * f_avg / (1.0 - ems_v * ems_l * f_avg * f_avg + EPSILON);
    return single_scatter + f_ms * (ems_v * ems_l);
}

// three's `getRoughness`: the material roughness, no lower than the base mip of a 256 cube, plus
// the view-space normal's screen-space variation (`getGeometryRoughness`), at most 1.
fn ship_roughness(roughness: f32, d_normal_x: vec3<f32>, d_normal_y: vec3<f32>) -> f32 {
    let dxy = max(abs(d_normal_x), abs(d_normal_y));
    let geometry_roughness = max(max(dxy.x, dxy.y), dxy.z);
    return min(max(roughness, 0.0525) + geometry_roughness, 1.0);
}

// `PhysicalLightingModel.direct` for one directional light of radiance `light_radiance`
// (colour × intensity) toward `light`.
fn direct_light(
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    surface: Surface,
    light: vec3<f32>,
    light_radiance: vec3<f32>,
    reflected: ptr<function, ReflectedLight>,
) {
    let dot_nl = clamp(dot(surface.normal, light), 0.0, 1.0);
    let irradiance = dot_nl * light_radiance;
    let diffuse_contribution = surface.diffuse_color * (1.0 - surface.metalness);
    let specular_blended = mix(vec3<f32>(0.04), surface.diffuse_color, surface.metalness);
    (*reflected).direct_diffuse += irradiance * diffuse_contribution * RECIPROCAL_PI;
    (*reflected).direct_specular += irradiance
        * brdf_ggx_multiscatter(lut, lut_sampler, light, surface.normal, surface.view, specular_blended, 1.0, surface.roughness);
}

// `PhysicalLightingModel.indirectDiffuse` for analytic irradiance (the hemisphere fill).
fn indirect_irradiance(surface: Surface, irradiance: vec3<f32>, reflected: ptr<function, ReflectedLight>) {
    let diffuse_contribution = surface.diffuse_color * (1.0 - surface.metalness);
    (*reflected).indirect_diffuse += irradiance * diffuse_contribution * RECIPROCAL_PI;
}

// `PhysicalLightingModel.indirectSpecular`: the environment's prefiltered `radiance` along the
// reflection and its `ibl_irradiance` (π × the roughest level along the normal), each split by
// Fdez-Agüera's multiple-scattering approximation for the dielectric and metallic paths.
fn indirect_environment(
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    surface: Surface,
    radiance: vec3<f32>,
    ibl_irradiance: vec3<f32>,
    reflected: ptr<function, ReflectedLight>,
) {
    let dot_nv = clamp(dot(surface.normal, surface.view), 0.0, 1.0);
    let fab = dfg(lut, lut_sampler, surface.roughness, dot_nv);
    let ess = fab.x + fab.y;
    let ems = 1.0 - ess;

    let f0_dielectric = vec3<f32>(0.04);
    let fss_ess_dielectric = f0_dielectric * fab.x + fab.y;
    let f_avg_dielectric = f0_dielectric + (1.0 - f0_dielectric) * 0.047619;
    let f_ms_dielectric = fss_ess_dielectric * f_avg_dielectric / (1.0 - ems * f_avg_dielectric);
    let single_dielectric = fss_ess_dielectric;
    let multi_dielectric = f_ms_dielectric * ems;

    let f0_metallic = surface.diffuse_color;
    let fss_ess_metallic = f0_metallic * fab.x + fab.y;
    let f_avg_metallic = f0_metallic + (1.0 - f0_metallic) * 0.047619;
    let f_ms_metallic = fss_ess_metallic * f_avg_metallic / (1.0 - ems * f_avg_metallic);
    let single_metallic = fss_ess_metallic;
    let multi_metallic = f_ms_metallic * ems;

    let single_scattering = mix(single_dielectric, single_metallic, surface.metalness);
    let multi_scattering = mix(multi_dielectric, multi_metallic, surface.metalness);
    let total_dielectric = single_dielectric + multi_dielectric;
    let diffuse_contribution = surface.diffuse_color * (1.0 - surface.metalness);
    let diffuse = diffuse_contribution * (1.0 - total_dielectric);
    let cosine_weighted = ibl_irradiance * RECIPROCAL_PI;
    (*reflected).indirect_specular += radiance * single_scattering + multi_scattering * cosine_weighted;
    (*reflected).indirect_diffuse += diffuse * cosine_weighted;
}

// `PhysicalLightingModel.ambientOcclusion` for an occlusion term `ao` (1 = open sky). The ship
// occlusion pass (the web game's `ShipOcclusion.ts`) is not ported yet, so callers pass 1.
fn apply_ambient_occlusion(surface: Surface, ao: f32, reflected: ptr<function, ReflectedLight>) {
    let dot_nv = clamp(dot(surface.normal, surface.view), 0.0, 1.0);
    let ao_nv = dot_nv + ao;
    let ao_exp = exp2(-(1.0 + 16.0 * surface.roughness));
    let ao_specular = clamp(ao - (1.0 - pow(ao_nv, ao_exp)), 0.0, 1.0);
    (*reflected).indirect_diffuse *= ao;
    (*reflected).indirect_specular *= ao_specular;
}
