// three.js r185's physical shading as the terrain uses it: `TerrainLightingModel` (the web game's `terrain/TerrainMaterial.ts`)
// for the sun, with a single-scattering GGX whose Fresnel runs from 0.04 × `specular` to `specular` at grazing angles,
// and three's `PhysicalLightingModel` for the hemisphere fill, the environment (`indirectSpecular` with
// `computeMultiscattering` over the overridden `specularColor` and `specularF90`) and `ambientOcclusion`.
//
// Ports `three/src/nodes/functions/BSDF/` (`F_Schlick`, `V_GGX_SmithCorrelated`, `D_GGX`, `BRDF_GGX`, `DFGLUT`) as
// naval-ships' `bsdf.wgsl` does. Everything is in world space; the patches' geometry normal is constant, so three's
// geometry roughness is zero here.
#define_import_path naval_terrain::lighting

#import naval_terrain::bindings::{terrain, dfg_lut, clamp_sampler}
#import naval::lights::{LIGHT_WORDS, scene_lights, light_cell, light_word, shadowed_point_light}
#import naval_terrain::surface::ramp
#import naval::aerial::aerial_apply
#import naval::pbr::{f_schlick, v_ggx_smith_correlated, d_ggx}

const RECIPROCAL_PI: f32 = 0.3183098861837907;

fn brdf_ggx(light: vec3<f32>, normal: vec3<f32>, view: vec3<f32>, f0: vec3<f32>, f90: f32, roughness: f32) -> vec3<f32> {
    let alpha = roughness * roughness;
    let half_dir = normalize(light + view);
    let dot_nl = clamp(dot(normal, light), 0.0, 1.0);
    let dot_nv = clamp(dot(normal, view), 0.0, 1.0);
    let dot_nh = clamp(dot(normal, half_dir), 0.0, 1.0);
    let dot_vh = clamp(dot(view, half_dir), 0.0, 1.0);
    return f_schlick(f0, f90, dot_vh) * v_ggx_smith_correlated(alpha, dot_nl, dot_nv) * d_ggx(alpha, dot_nh);
}

// three's `getRoughness` for a constant geometry normal: no lower than the base mip of a 256 cube, at most 1.
fn land_roughness(roughness: f32) -> f32 {
    return min(max(roughness, 0.0525), 1.0);
}

struct LandLight {
    // Unit direction toward the sun and its radiance on meshes, already shaded by the land, the shadow maps and the
    // clouds.
    sun_direction: vec3<f32>,
    sun_radiance: vec3<f32>,
    // The hemisphere fill's irradiance at this normal.
    fill: vec3<f32>,
    // The environment's prefiltered radiance along the (bent) reflection, and its irradiance (π × the roughest level
    // along the normal).
    radiance: vec3<f32>,
    irradiance: vec3<f32>,
    // World position (m), where the scene's point lights are measured from.
    world: vec3<f32>,
}

// The lit land: `TerrainLightingModel.direct`, then three's indirect diffuse, indirect specular and ambient occlusion.
fn shade_land(
    albedo: vec3<f32>,
    roughness: f32,
    specular: f32,
    occlusion: f32,
    normal: vec3<f32>,
    view: vec3<f32>,
    light: LandLight,
) -> vec3<f32> {
    let f0 = vec3<f32>(0.04) * specular;
    let f90 = specular;
    let dot_nv = clamp(dot(normal, view), 0.0, 1.0);
    // Direct: Lambert and a single-scattering GGX, for the sun and the point lights that reach this point's cell of the
    // renderer's index (`naval::lights`; "other lights pass as they are": only the sun takes the land's own shadow,
    // already in its radiance, and the hulls shade the brightest searchlights and star shells through their maps).
    let irradiance = clamp(dot(normal, light.sun_direction), 0.0, 1.0) * light.sun_radiance;
    var direct_diffuse = irradiance * albedo * RECIPROCAL_PI;
    var direct_specular = irradiance * brdf_ggx(light.sun_direction, normal, view, f0, f90, roughness);
    let cell = light_cell(light.world);
    for (var word = 0u; word < LIGHT_WORDS; word += 1u) {
        var bits = light_word(cell, word);
        while bits != 0u {
            let index = word * 32u + firstTrailingBit(bits);
            bits &= bits - 1u;
            let point = shadowed_point_light(scene_lights.lights[index], light.world, normal);
            let lit = clamp(dot(normal, point.direction), 0.0, 1.0) * point.radiance;
            direct_diffuse += lit * albedo * RECIPROCAL_PI;
            direct_specular += lit * brdf_ggx(point.direction, normal, view, f0, f90, roughness);
        }
    }
    // Indirect diffuse from the hemisphere fill.
    var indirect_diffuse = light.fill * albedo * RECIPROCAL_PI;
    // Indirect specular and the environment's diffuse, split by Fdez-Agüera's multiple scattering (dielectric).
    let fab = textureSampleLevel(dfg_lut, clamp_sampler, vec2<f32>(roughness, dot_nv), 0.0).rg;
    let fss_ess = f0 * fab.x + f90 * fab.y;
    let ess = fab.x + fab.y;
    let ems = 1.0 - ess;
    let f_avg = f0 + (1.0 - f0) * 0.047619;
    let f_ms = fss_ess * f_avg / (1.0 - ems * f_avg);
    let single_scattering = fss_ess;
    let multi_scattering = f_ms * ems;
    let cosine_weighted = light.irradiance * RECIPROCAL_PI;
    var indirect_specular = light.radiance * single_scattering + multi_scattering * cosine_weighted;
    indirect_diffuse += albedo * (1.0 - (single_scattering + multi_scattering)) * cosine_weighted;
    // Ambient occlusion: all of the indirect diffuse, and the specular by three's view-dependent term.
    let ao_nv = dot_nv + occlusion;
    let ao_exp = exp2(-(1.0 + 16.0 * roughness));
    let ao_specular = clamp(occlusion - (1.0 - pow(ao_nv, ao_exp)), 0.0, 1.0);
    indirect_diffuse *= occlusion;
    indirect_specular *= ao_specular;
    return direct_diffuse + direct_specular + indirect_diffuse + indirect_specular;
}

// The air between the eye and the land at `world` (`naval::aerial`: the atmosphere and the weather's haze, as every
// material takes it), while the scene's fog is on.
fn fogged(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if terrain.fog.w < 0.5 {
        return color;
    }
    return aerial_apply(color, world);
}
