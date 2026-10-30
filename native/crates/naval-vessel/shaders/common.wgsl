// What every ship pipeline binds (no source port): the frame at group 1 (the light, the DFG table, the instances,
// the lamps), a material at group 2 (its factors, five maps and each one's own sampler), and reading a material's maps through
// their UV sets and KHR_texture_transform. Rust twins in `src/layer.rs` (`FrameUniforms`, `InstanceRecord`,
// `MaterialUniforms`, `LampRecord`); keep them in order.
#define_import_path naval_vessel::common

#import naval::lights::SceneFog

struct Frame {
    // Unit world vector toward the sun (or moon); w unused.
    sun_direction: vec4<f32>,
    // Its colour × intensity.
    sun_radiance: vec4<f32>,
    // Hemisphere fill (a lightning flash's): sky and ground colour × intensity.
    fill_sky: vec4<f32>,
    fill_ground: vec4<f32>,
    // x environment intensity, y 1 to receive the sun's shadow maps, z an emissive map of 1's radiance, w unused.
    lighting: vec4<f32>,
    fog: SceneFog,
    // The main view's eye (xyz): the instances are placed less it, in every pass.
    eye: vec4<f32>,
}

struct Instance {
    // Ship space to the world less the eye, this frame and the one before (the latter less this frame's eye).
    model: mat4x4<f32>,
    previous: mat4x4<f32>,
    // The normals' turn.
    normal: mat3x3<f32>,
    // x 1 when the frame before drew her (her motion has history).
    flags: vec4<f32>,
}

// A lamp as drawn: where (less the eye; w its size, m) and its light at a metre (rgb × candela × the scene's scale).
struct Lamp {
    offset: vec4<f32>,
    light: vec4<f32>,
}

@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var dfg_lut: texture_2d<f32>;
@group(1) @binding(2) var lut_sampler: sampler;
@group(1) @binding(3) var<storage, read> instances: array<Instance>;
@group(1) @binding(4) var<storage, read> lamps: array<Lamp>;

// One map's UV set and transform: uv' = (u_row.xyz · (u, v, 1), v_row.xyz · (u, v, 1)); u_row.w 1 to read UV set 1,
// v_row.w 1 when the material has the map.
struct MapTransform {
    u_row: vec4<f32>,
    v_row: vec4<f32>,
}

struct Material {
    // Linear RGBA.
    base_color: vec4<f32>,
    // rgb emissive (× its strength), w unused.
    emissive: vec4<f32>,
    // x metallic, y roughness, z normal scale, w occlusion strength.
    params: vec4<f32>,
    // x alpha cut-off, y 1 unlit, z 1 glass (its reflections kept whole when blended), w unused.
    flags: vec4<f32>,
    // Base colour, metal-rough, normal, occlusion, emissive.
    maps: array<MapTransform, 5>,
}

@group(2) @binding(0) var<uniform> material: Material;
@group(2) @binding(1) var base_color_map: texture_2d<f32>;
@group(2) @binding(2) var metal_rough_map: texture_2d<f32>;
@group(2) @binding(3) var normal_map: texture_2d<f32>;
@group(2) @binding(4) var occlusion_map: texture_2d<f32>;
@group(2) @binding(5) var emissive_map: texture_2d<f32>;
// Each map's texture's own sampler (its wrapping and filtering).
@group(2) @binding(6) var base_color_sampler: sampler;
@group(2) @binding(7) var metal_rough_sampler: sampler;
@group(2) @binding(8) var normal_sampler: sampler;
@group(2) @binding(9) var occlusion_sampler: sampler;
@group(2) @binding(10) var emissive_sampler: sampler;

const BASE_COLOR: u32 = 0u;
const METAL_ROUGH: u32 = 1u;
const NORMAL: u32 = 2u;
const OCCLUSION: u32 = 3u;
const EMISSIVE: u32 = 4u;

fn map_uv(slot: u32, uv0: vec2<f32>, uv1: vec2<f32>) -> vec2<f32> {
    let m = material.maps[slot];
    let uv = vec3<f32>(select(uv0, uv1, m.u_row.w > 0.5), 1.0);
    return vec2<f32>(dot(m.u_row.xyz, uv), dot(m.v_row.xyz, uv));
}

fn has_map(slot: u32) -> bool {
    return material.maps[slot].v_row.w > 0.5;
}

// The base colour: its factor × its map × the vertex colour.
fn base_color(uv0: vec2<f32>, uv1: vec2<f32>, color: vec4<f32>) -> vec4<f32> {
    return material.base_color * color * textureSample(base_color_map, base_color_sampler, map_uv(BASE_COLOR, uv0, uv1));
}
