// Any glTF ship's surfaces (no source port: the removed web game drew only its own hulls). glTF's metallic-roughness
// material (base colour, metal-rough, normal, occlusion and emissive maps, vertex colours, cut-outs and blending) lit as
// the wildlife and airframes are (naval-wildlife's `wildlife.wgsl`, `reflected_light`, without its water): the sun
// shadowed by the renderer's sun maps, the scene's point lights, a lightning flash's hemisphere fill and the sky's
// environment through three's BSDF, the occlusion map on the indirect light, then the air between her and the eye.
// Normal maps are read on a frame from the screen's derivatives (three's `perturbNormal2Arb`, as three reads a glTF
// without tangents).
//
// Groups: 0 the renderer's camera, shadow maps and lights, 1 the frame (`naval_vessel::common`), 2 a material, 3 the
// sky's environment.

#import naval::camera::eye_to_clip
#import naval::shadow::sun_shadow
#import naval::lights::{LIGHT_WORDS, scene_lights, light_cell, light_word, point_light_at}
#import naval::aerial::{aerial_apply, aerial_apply_additive}
#import naval::motion::{motion_response, surface_motion_from_eye}
#import naval_vessel::bsdf::{Surface, ReflectedLight, direct_light, indirect_irradiance, indirect_environment, apply_ambient_occlusion, ship_roughness}
#import naval_vessel::common::{frame, instances, material, dfg_lut, lut_sampler, metal_rough_map, normal_map, occlusion_map, emissive_map, metal_rough_sampler, normal_sampler, occlusion_sampler, emissive_sampler, map_uv, has_map, base_color, METAL_ROUGH, NORMAL, OCCLUSION, EMISSIVE}
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction}

@group(3) @binding(0) var sky_atlas: texture_2d<f32>;
@group(3) @binding(1) var sky_sampler: sampler;
@group(3) @binding(2) var<uniform> sky_env: SkyEnvironment;

const PI: f32 = 3.141592653589793;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The position less the eye's: its derivatives stay exact far from the world's origin.
    @location(0) offset: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv0: vec2<f32>,
    @location(3) uv1: vec2<f32>,
    @location(4) color: vec4<f32>,
}

@vertex
fn vs_main(in: VertexIn, @builtin(instance_index) index: u32) -> VertexOut {
    let instance = instances[index];
    let offset = (instance.model * vec4<f32>(in.position, 1.0)).xyz;
    var out: VertexOut;
    out.clip = eye_to_clip(offset);
    out.offset = offset;
    out.normal = instance.normal * in.normal;
    out.uv0 = in.uv0;
    out.uv1 = in.uv1;
    out.color = in.color;
    return out;
}

// The light a surface reflects, its diffuse and specular parts apart (glass blends one and keeps the other).
struct Lit {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
}

fn reflected_light(surface: Surface, world: vec3<f32>, geometric: vec3<f32>, occlusion: f32) -> Lit {
    var reflected: ReflectedLight;
    reflected.direct_diffuse = vec3<f32>(0.0);
    reflected.direct_specular = vec3<f32>(0.0);
    reflected.indirect_diffuse = vec3<f32>(0.0);
    reflected.indirect_specular = vec3<f32>(0.0);
    var shadow = 1.0;
    if frame.lighting.y > 0.5 {
        shadow = sun_shadow(world, geometric);
    }
    let intensity = frame.lighting.x;
    direct_light(dfg_lut, lut_sampler, surface, frame.sun_direction.xyz, frame.sun_radiance.rgb * shadow, &reflected);
    // The scene's point lights that reach this pixel's cell (`naval::lights`).
    let cell = light_cell(world);
    for (var word = 0u; word < LIGHT_WORDS; word += 1u) {
        var bits = light_word(cell, word);
        while bits != 0u {
            let index = word * 32u + firstTrailingBit(bits);
            bits &= bits - 1u;
            let light = point_light_at(scene_lights.lights[index], world);
            direct_light(dfg_lut, lut_sampler, surface, light.direction, light.radiance, &reflected);
        }
    }
    let up = dot(surface.normal, vec3<f32>(0.0, 1.0, 0.0)) * 0.5 + 0.5;
    indirect_irradiance(surface, mix(frame.fill_ground.rgb, frame.fill_sky.rgb, up), &reflected);
    let direction = env_reflection_direction(surface.normal, surface.view, surface.roughness);
    let radiance = env_radiance(sky_atlas, sky_sampler, sky_env, direction, surface.roughness) * intensity;
    let irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, surface.normal) * intensity;
    indirect_environment(dfg_lut, lut_sampler, surface, radiance, irradiance, &reflected);
    apply_ambient_occlusion(surface, occlusion, &reflected);
    var lit: Lit;
    lit.diffuse = reflected.direct_diffuse + reflected.indirect_diffuse;
    lit.specular = reflected.direct_specular + reflected.indirect_specular;
    return lit;
}

// KHR_materials_unlit: the colour under the light falling on a level surface (the sun's and the sky's), unshaded, so a
// baked model follows the day and the night.
fn unlit_light(world: vec3<f32>, geometric: vec3<f32>) -> vec3<f32> {
    var shadow = 1.0;
    if frame.lighting.y > 0.5 {
        shadow = sun_shadow(world, geometric);
    }
    let sun = frame.sun_radiance.rgb * max(frame.sun_direction.y, 0.0) * shadow;
    let sky = env_irradiance(sky_atlas, sky_sampler, sky_env, vec3<f32>(0.0, 1.0, 0.0)) * frame.lighting.x;
    return (sun + sky + frame.fill_sky.rgb) / PI;
}

// A shaded fragment: its light (diffuse and specular apart, emission in the diffuse) and its alpha.
struct Shaded {
    lit: Lit,
    alpha: f32,
    world: vec3<f32>,
}

fn shade(in: VertexOut, front_facing: bool) -> Shaded {
    let base = base_color(in.uv0, in.uv1, in.color);
    let metal_rough = textureSample(metal_rough_map, metal_rough_sampler, map_uv(METAL_ROUGH, in.uv0, in.uv1));
    let occlusion_texel = textureSample(occlusion_map, occlusion_sampler, map_uv(OCCLUSION, in.uv0, in.uv1)).r;
    let emissive_texel = textureSample(emissive_map, emissive_sampler, map_uv(EMISSIVE, in.uv0, in.uv1)).rgb;
    let normal_uv = map_uv(NORMAL, in.uv0, in.uv1);
    var mapped = textureSample(normal_map, normal_sampler, normal_uv).xyz * 2.0 - 1.0;
    // The frame the normal map is read on, from the screen's derivatives: done before anything is discarded.
    let dp_x = dpdx(in.offset);
    let dp_y = dpdy(in.offset);
    let duv_x = dpdx(normal_uv);
    let duv_y = dpdy(normal_uv);
    let d_normal_x = dpdx(in.normal);
    let d_normal_y = dpdy(in.normal);

    let world = in.offset + frame.eye.xyz;
    var normal = normalize(in.normal);
    // Both faces of a double-sided material (the other pipelines cull the back).
    normal = select(-normal, normal, front_facing);
    let geometric = normal;
    if has_map(NORMAL) {
        let scale = material.params.z;
        // glTF's +Y is up the image; the derivatives' bitangent runs down it (three flips it the same way).
        mapped = vec3<f32>(mapped.x * scale, -mapped.y * scale, mapped.z);
        let side = select(-1.0, 1.0, front_facing);
        let across_y = cross(dp_y, normal);
        let across_x = cross(normal, dp_x);
        let tangent = (across_y * duv_x.x + across_x * duv_y.x) * side;
        let bitangent = (across_y * duv_x.y + across_x * duv_y.y) * side;
        let size = max(dot(tangent, tangent), dot(bitangent, bitangent));
        if size > 0.0 {
            let inverse = inverseSqrt(size);
            normal = normalize(mat3x3<f32>(tangent * inverse, bitangent * inverse, normal) * mapped);
        }
    }
    var surface: Surface;
    surface.diffuse_color = base.rgb;
    surface.metalness = clamp(material.params.x * metal_rough.b, 0.0, 1.0);
    surface.roughness = ship_roughness(clamp(material.params.y * metal_rough.g, 0.0, 1.0), d_normal_x, d_normal_y);
    surface.normal = normal;
    surface.view = normalize(-in.offset);
    let occlusion = 1.0 + material.params.w * (occlusion_texel - 1.0);
    var out: Shaded;
    if material.flags.y > 0.5 {
        out.lit.diffuse = base.rgb * unlit_light(world, geometric) * occlusion;
        out.lit.specular = vec3<f32>(0.0);
    } else {
        out.lit = reflected_light(surface, world, geometric, occlusion);
    }
    out.lit.diffuse += material.emissive.rgb * emissive_texel * frame.lighting.z;
    out.alpha = base.a;
    out.world = world;
    return out;
}

fn hazed(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if frame.fog.params.w > 0.5 {
        return aerial_apply(color, world);
    }
    return color;
}

@fragment
fn fs_opaque(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shade(in, front_facing);
    return vec4<f32>(hazed(s.lit.diffuse + s.lit.specular, s.world), 1.0);
}

// MASK: cut where the alpha is under the cut-off.
@fragment
fn fs_mask(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shade(in, front_facing);
    if s.alpha < material.flags.x {
        discard;
    }
    return vec4<f32>(hazed(s.lit.diffuse + s.lit.specular, s.world), 1.0);
}

// BLEND, premultiplied: the diffuse part over what is behind by its alpha, the reflections by it too (a decal's clear
// texels show nothing) but whole on glass, which reflects as much as it lets through.
@fragment
fn fs_blend(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let s = shade(in, front_facing);
    let alpha = clamp(s.alpha, 0.0, 1.0);
    let reflected = s.lit.specular * select(alpha, 1.0, material.flags.z > 0.5);
    var color = s.lit.diffuse * alpha + reflected;
    if frame.fog.params.w > 0.5 {
        color = aerial_apply(s.lit.diffuse, s.world) * alpha + aerial_apply_additive(reflected, s.world);
    }
    return vec4<f32>(color, alpha);
}

struct MotionOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) current: vec3<f32>,
    @location(1) previous: vec3<f32>,
    @location(2) @interpolate(flat) kept: f32,
    // A cut-out's coverage, as the scene pass reads it.
    @location(3) uv0: vec2<f32>,
    @location(4) uv1: vec2<f32>,
    @location(5) color: vec4<f32>,
}

// Temporal AA's motion pass: each vertex by her motion since the frame before; a ship new this frame takes no history.
// Its pipelines cull and cut as the scene pass's do, so what it covers is what was drawn.
@vertex
fn vs_motion(in: VertexIn, @builtin(instance_index) index: u32) -> MotionOut {
    let instance = instances[index];
    let offset = (instance.model * vec4<f32>(in.position, 1.0)).xyz;
    var out: MotionOut;
    out.clip = eye_to_clip(offset);
    out.current = offset;
    out.previous = (instance.previous * vec4<f32>(in.position, 1.0)).xyz;
    out.kept = instance.flags.x;
    out.uv0 = in.uv0;
    out.uv1 = in.uv1;
    out.color = in.color;
    return out;
}

fn motion(in: MotionOut) -> vec4<f32> {
    if in.kept < 0.5 {
        return motion_response(1.0);
    }
    return surface_motion_from_eye(in.current, in.previous, 0.0);
}

@fragment
fn fs_motion(in: MotionOut) -> @location(0) vec4<f32> {
    return motion(in);
}

// MASK: none where the scene pass cut it away.
@fragment
fn fs_motion_mask(in: MotionOut) -> @location(0) vec4<f32> {
    if base_color(in.uv0, in.uv1, in.color).a < material.flags.x {
        discard;
    }
    return motion(in);
}
