// The horizon ring (`naval_terrain::horizon`): the real world's land beyond a theatre window, as silhouettes.
//
// Patches of a quadtree about the maps' centre (CDLOD as `terrain.wgsl`'s: a level's odd vertices slide onto its even
// ones across the outer part of its range), displaced by the near level's heights where it reaches and the far level's
// beyond, held to the open sea's floor and sunk under the window's square. Shaded as the window's land is from its
// photograph (`surface.wgsl`'s calibration, the photograph's own relief shading lessened on slopes, the season's snow
// from the climate's snowline) and lit by the sun, the sky's environment and the clouds' shadow, with no shadow of its
// own; the air over it is every material's (`naval::aerial`).
#import naval::camera::{camera, world_to_clip}
#import naval::aerial::aerial_at
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#import naval_terrain::surface::{ramp, calibrated, tone_down, srgb}
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction}

// `naval_terrain::horizon::HorizonUniforms`.
struct Horizon {
    // World x and z of the maps' centre, level 0's range (m), the floor.
    origin: vec4<f32>,
    // Half the near and far levels' sides, their height cells (m).
    levels: vec4<f32>,
    // The window's square in the ring's frame (centre, half side), and 1 when there is one.
    hole: vec4<f32>,
    // Toward the sun; 1 where the region's style takes the season's snow.
    sun_direction: vec4<f32>,
    // The sun's radiance on meshes; the environment's intensity.
    sun_radiance: vec4<f32>,
    // The hemisphere fill; 1 while the clouds' shadow is bound.
    fill_sky: vec4<f32>,
    fill_ground: vec4<f32>,
    // The haze taken out of the photograph.
    haze: vec4<f32>,
    // The snowline (m), the thaw below it, the direction toward the equator (chart x, z).
    climate: vec4<f32>,
    // The season's melt (the leaves', `surface.wgsl`'s `snow_melt`), 0-1.
    season: vec4<f32>,
}

@group(1) @binding(0) var<uniform> horizon: Horizon;
// The levels' heights (m) and colours, near level in layer 0.
@group(1) @binding(1) var heights: texture_2d_array<f32>;
@group(1) @binding(2) var colours: texture_2d_array<f32>;
@group(1) @binding(3) var ring_sampler: sampler;

@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;

@group(3) @binding(0) var cloud_map: texture_2d<f32>;
@group(3) @binding(1) var cloud_sampler: sampler;
@group(3) @binding(2) var<uniform> cloud_params: CloudShadow;

// `horizon::MORPH_START`, `MORPH_END` (the land's).
const MORPH_START: f32 = 0.78;
const MORPH_END: f32 = 0.97;
// `horizon::NEAR_BLEND_TEXELS`: near texels at its edge over which the far level takes over.
const NEAR_BLEND_TEXELS: f32 = 32.0;
// `horizon::SINK`: metres the ring sinks a metre inside the window's square.
const SINK: f32 = 1.0;
// log2 of `horizon::LEVEL_SCALE`.
const LEVEL_STEP: f32 = 2.0;
// The colour's texels are twice the heights'.
const COLOUR_STEP: f32 = 1.0;
const PI: f32 = 3.141592653589793;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// `surface.wgsl`'s photograph calibration.
const PHOTO_DETAIL: f32 = 0.12;

struct VertexIn {
    // The patch vertex (i, j), 0-16.
    @location(0) grid: vec2<f32>,
    // The patch: its corner (m in the ring's frame), its level.
    @location(1) instance: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The ring's frame (m) and the height there, before the window's square sinks it.
    @location(0) local: vec3<f32>,
    @location(1) world: vec3<f32>,
}

// The share of the near level at `local`: 1 inside its square, falling to 0 over its outer texels.
fn near_share(local: vec2<f32>) -> f32 {
    let edge = horizon.levels.x - max(abs(local.x), abs(local.y));
    return clamp(edge / (horizon.levels.z * NEAR_BLEND_TEXELS), 0.0, 1.0);
}

fn level_uv(local: vec2<f32>, half: f32) -> vec2<f32> {
    return (local + vec2<f32>(half)) / (2.0 * half);
}

// The height (m) at `local`, read at `lod` of the near level's texels (the far level's that much coarser).
fn height_at(local: vec2<f32>, lod: f32) -> f32 {
    let w = near_share(local);
    var h = textureSampleLevel(heights, ring_sampler, level_uv(local, horizon.levels.y), 1, max(lod - LEVEL_STEP, 0.0)).r;
    if w > 0.0 {
        let near = textureSampleLevel(heights, ring_sampler, level_uv(local, horizon.levels.x), 0, max(lod, 0.0)).r;
        h = mix(h, near, w);
    }
    return h;
}

// The camera in the ring's frame.
fn camera_local() -> vec3<f32> {
    return vec3<f32>(camera.position.x - horizon.origin.x, camera.position.y, camera.position.z - horizon.origin.y);
}

@vertex
fn vs_horizon(in: VertexIn) -> VertexOut {
    let level = in.instance.z;
    let step = horizon.levels.z * exp2(level);
    let at = in.instance.xy + in.grid * step;
    // The morph by the vertex's distance from the eye (the selection's bounds are the heights'), read a level coarser.
    let reach = length(vec3<f32>(at.x, max(height_at(at, level + 1.0), horizon.origin.w), at.y) - camera_local());
    let range = horizon.origin.z * exp2(level);
    let morph = clamp((reach - range * MORPH_START) / (range * (MORPH_END - MORPH_START)), 0.0, 1.0);
    let xz = in.instance.xy + in.grid * step - fract(in.grid * 0.5) * 2.0 * step * morph;
    // Read at the level the vertices' spacing spans (its own and its morph's: two levels meeting read the same).
    let height = max(height_at(xz, level + morph), horizon.origin.w);
    var drawn = height;
    if horizon.hole.w > 0.5 {
        let inside = horizon.hole.z - max(abs(xz.x - horizon.hole.x), abs(xz.y - horizon.hole.y));
        drawn -= max(inside, 0.0) * SINK;
    }
    let world = vec3<f32>(xz.x + horizon.origin.x, drawn, xz.y + horizon.origin.y);
    var out: VertexOut;
    out.clip = world_to_clip(world);
    out.local = vec3<f32>(xz.x, height, xz.y);
    out.world = world;
    return out;
}

// The slope (dh/dx, dh/dz) of one level at `uv` by central differences `lod` levels coarse.
fn level_gradient(uv: vec2<f32>, layer: i32, lod: f32, cell: f32) -> vec2<f32> {
    let texel = exp2(floor(lod)) / f32(textureDimensions(heights).x);
    let l = floor(lod);
    let e = textureSampleLevel(heights, ring_sampler, uv + vec2<f32>(texel, 0.0), layer, l).r;
    let w = textureSampleLevel(heights, ring_sampler, uv - vec2<f32>(texel, 0.0), layer, l).r;
    let s = textureSampleLevel(heights, ring_sampler, uv + vec2<f32>(0.0, texel), layer, l).r;
    let n = textureSampleLevel(heights, ring_sampler, uv - vec2<f32>(0.0, texel), layer, l).r;
    let run = 2.0 * cell * exp2(l);
    return vec2<f32>(e - w, s - n) / run;
}

// One level's colour at `local`, `lod` of its own texels.
fn level_colour(local: vec2<f32>, layer: i32, half: f32, lod: f32) -> vec3<f32> {
    return textureSampleLevel(colours, ring_sampler, level_uv(local, half), layer, max(lod, 0.0)).rgb;
}

// Karis's fit of the split-sum's scale and bias (the land's DFG table, which the ring does not bind).
fn env_brdf(roughness: f32, dot_nv: f32) -> vec2<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * dot_nv)) * r.x + r.y;
    return vec2<f32>(-1.04, 1.04) * a004 + r.zw;
}

@fragment
fn fs_horizon(in: VertexOut) -> @location(0) vec4<f32> {
    let local = in.local.xz;
    let height = in.local.y;
    // Metres a pixel spans: the heights and colour are read no finer.
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let footprint = max(max(length(dx), length(dy)), 1.0);
    // The air between: where it lets through nothing of the land (fog, thick haze), the air alone, which is what the
    // land's light under it comes to within a 2048th.
    let air = aerial_at(in.world);
    if max(air.transmittance.r, max(air.transmittance.g, air.transmittance.b)) < 1.0 / 2048.0 {
        return vec4<f32>(air.inscatter, 1.0);
    }
    let near = near_share(local);
    let near_cell = horizon.levels.z;
    let far_cell = horizon.levels.w;
    let lod_near = max(log2(footprint / near_cell), 0.0);
    let lod_far = max(log2(footprint / far_cell), 0.0);
    var gradient = level_gradient(level_uv(local, horizon.levels.y), 1, lod_far, far_cell);
    var colour = level_colour(local, 1, horizon.levels.y, lod_far - COLOUR_STEP);
    var broad = level_colour(local, 1, horizon.levels.y, lod_far - COLOUR_STEP + 2.0);
    if near > 0.0 {
        let uv = level_uv(local, horizon.levels.x);
        gradient = mix(gradient, level_gradient(uv, 0, lod_near, near_cell), near);
        colour = mix(colour, level_colour(local, 0, horizon.levels.x, lod_near - COLOUR_STEP), near);
        broad = mix(broad, level_colour(local, 0, horizon.levels.x, lod_near - COLOUR_STEP + 2.0), near);
    }
    let slope = length(gradient);
    let normal = normalize(vec3<f32>(-gradient.x, 1.0, -gradient.y));

    // The photograph as the window's land takes it (`real_surface`): its haze out, its own relief shading lessened
    // where the land slopes (ours shades it), calibrated, a detail's contrast against its surroundings given back.
    let haze = horizon.haze.rgb;
    let raw = max(colour - haze, vec3<f32>(0.0));
    let raw_broad = max(broad - haze, vec3<f32>(0.0));
    let ratio = max(dot(raw_broad, LUMA), 1e-4) / max(dot(raw, LUMA), 1e-4);
    let relief = ramp(0.05, 0.35, slope) * 0.6;
    var albedo = calibrated(raw * pow(ratio, relief)) * pow(clamp(1.0 / ratio, 0.25, 4.0), PHOTO_DETAIL * (1.0 - relief / 0.6));
    var roughness = 0.93;
    var specular = 0.35;
    // The season's snow above the climate's snowline (`climate_snow`, its hollows and knolls the heights' own against
    // their surroundings four texels about): higher on slopes facing the sun and on knolls, lower in hollows, lying in
    // patches below the line until the thaw, shed by steep faces.
    if horizon.sun_direction.w > 0.5 {
        let snowline_m = horizon.climate.x;
        let sunny = dot(-gradient, horizon.climate.zw) / sqrt(slope * slope + 1.0);
        let around = height_at(local, lod_near + 2.0) - height_at(local, lod_near);
        let hollow = ramp(0.0, 30.0, around);
        let knoll = ramp(0.0, 30.0, -around);
        let snowline = snowline_m + sunny * 220.0 - hollow * 60.0 + knoll * 25.0;
        let above = ramp(snowline - 20.0, snowline + 20.0, height);
        let lying = ramp(snowline - 260.0, snowline, height) * 0.35 + 0.5 + hollow * 0.25 - knoll * 0.2 - sunny * 0.9
            - horizon.climate.y * 0.65;
        let near_line = 1.0 - ramp(300.0, 700.0, snowline - height);
        // Shed by steep faces as `surface.wgsl`'s `snow_shed`: from 35-45 degrees in winter, 30-40 in the melt, the
        // melt reaching 300-900 m over the snowline.
        let melt = horizon.season.x * (1.0 - ramp(snowline + 300.0, snowline + 900.0, height));
        let shed = ramp(mix(0.7, 0.58, melt), mix(1.0, 0.84, melt), slope);
        let snow = max(above, ramp(0.47, 0.53, lying) * near_line) * (1.0 - shed);
        albedo = mix(albedo, tone_down(srgb(232.0, 237.0, 241.0)), snow);
        roughness = mix(roughness, 0.75, snow);
        specular = mix(specular, 0.45, snow);
    }

    // The sun through the clouds, the sky's environment, and its reflection off the rough ground.
    let view = normalize(camera.position.xyz - in.world);
    var sun = horizon.sun_radiance.rgb;
    if horizon.fill_sky.w > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, in.world);
    }
    let intensity = horizon.sun_radiance.w;
    let dot_nl = clamp(dot(normal, horizon.sun_direction.xyz), 0.0, 1.0);
    let dot_nv = clamp(dot(normal, view), 1e-3, 1.0);
    let fill = mix(horizon.fill_ground.rgb, horizon.fill_sky.rgb, normal.y * 0.5 + 0.5);
    let irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, normal) * intensity;
    let radiance = env_radiance(sky_atlas, sky_sampler, sky_env, env_reflection_direction(normal, view, roughness), roughness)
        * intensity;
    let fab = env_brdf(roughness, dot_nv);
    let reflected = 0.04 * specular * fab.x + specular * fab.y;
    let lit = albedo * (dot_nl * sun + fill + irradiance * (1.0 - reflected)) / PI + radiance * reflected;
    return vec4<f32>(lit * air.transmittance + air.inscatter, 1.0);
}
