// The land: level-of-detail patches displaced onto the heightfield and shaded with the map's cover.
//
// Ports the web game's `terrain/TerrainMaterial.ts`: the vertex stage (each patch instance's corner and level, the odd
// vertices sliding onto the even ones across the outer part of the level's range, the height read texel by texel with
// `Heightfield.height`'s bilinear expression), the fragment's inputs to `terrainSurface`, the small relief as a normal
// (`bumped`, Mikkelsen's surface gradient), `TerrainLightingModel` and three's indirect light, and the debug views
// (`TERRAIN_DEBUG`). The scene fog is the ocean's `scene.fogNode` (the web game's `ocean/screen/fog.ts`) with the sky behind
// read from the environment's sharpest level, as naval-ocean's surface stands it in. The seabed more than
// `SEABED_PLAIN_M` down, seen only through the sea, takes `seabed_surface` in place of the land's whole surface.
#import naval::camera::{camera, world_to_clip}
#import naval::shadow::{sun_shadow_maps, occluder_transmittance}
#import naval_terrain::bindings::{terrain, gradients, attributes, sunlight, open_sky, clamp_sampler, field_height, relief_height, relief_gradient, field_uv, coarse_uv, chart_offset, cleared, clearing_at, clearing_noise}
#import naval_terrain::features::feature_site
#import naval_terrain::night::night_land
#import naval_terrain::surface::{SurfaceInputs, Surface, terrain_surface, seabed_surface, SEABED_PLAIN_M, SEABED_RELIEF_M, ramp, srgb, tone_down}
#import naval_terrain::lighting::{LandLight, shade_land, land_roughness, fogged}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#ifdef TREES
#import naval_terrain::forest::{forest_at, forest_floor, floor_seen, cards_standing, under_leaves}
#endif
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction}

@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;

@group(3) @binding(0) var cloud_map: texture_2d<f32>;
@group(3) @binding(1) var cloud_sampler: sampler;
@group(3) @binding(2) var<uniform> cloud_params: CloudShadow;

// `MORPH_START`, `MORPH_END` (TerrainLod.ts).
const MORPH_START: f32 = 0.78;
const MORPH_END: f32 = 0.97;

struct VertexIn {
    // The patch vertex (i, j), 0–16; on the dense patch's mesh in halves (0, 0.5, … 16).
    @location(0) grid: vec2<f32>,
    // The instance: chart x and z of the patch's corner, its level, 1 where land rises above the sea (2 for a level-0
    // patch drawn dense, which the plain mesh leaves out, 3 for its copy the dense mesh draws: `lod::DENSE_LAND`).
    @location(1) corner: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Chart metres and height (the web's `positionLocal`: the mesh stands at the chart's offset).
    @location(0) local: vec3<f32>,
    @location(1) world: vec3<f32>,
    @location(2) @interpolate(flat) level: f32,
}

// The camera in chart metres.
fn camera_chart() -> vec3<f32> {
    let offset = chart_offset();
    return camera.position.xyz - vec3<f32>(offset.x, 0.0, offset.y);
}

// The share of level 0's range within which a level-0 patch with land over the fine relief is drawn dense, its
// vertices in halves (`lod::DENSE_SHARE`): there they span 5 pixels and more on the High setting.
const DENSE_SHARE: f32 = 0.25;
// The farthest a patch is drawn dense, m (`lod::DENSE_MAX_M`): through binoculars a quarter of the range is all of
// the relief.
const DENSE_MAX_M: f32 = 3000.0;

// The share of `range` along which a vertex `reach` from the eye has slid onto the coarser grid.
fn morph_of(reach: f32, range: f32) -> f32 {
    return clamp((reach - range * MORPH_START) / (range * (MORPH_END - MORPH_START)), 0.0, 1.0);
}

// Where patch vertex `grid` of `corner` stands, in chart metres, with the morph toward the coarser level. The reach
// that sets the morph is the field's (as the selection's bounds are), so neighbouring levels meet with the relief on.
// A dense patch's vertices between level 0's slide onto them over the outer part of `DENSE_SHARE` of level 0's range
// (at most `DENSE_MAX_M`) first, as a level's onto the next: no dense patch meets a plain one before they have. The
// fine relief is read at a level the vertices' spacing spans (a level's vertices and its morph's, so a vertex where two levels meet reads the same).
fn place_vertex(grid: vec2<f32>, corner: vec4<f32>, morphing: bool) -> vec3<f32> {
    let level = corner.z;
    let step = terrain.field.w * exp2(level);
    let at = corner.xy + grid * step;
    var xz = at;
    var spacing = level;
    if morphing {
        let reach = length(vec3<f32>(at.x, field_height(at), at.y) - camera_chart());
        let range = terrain.params.y * exp2(level);
        let morph = morph_of(reach, range);
        var g = grid;
        if corner.w > 2.5 {
            let fine = morph_of(reach, min(range * DENSE_SHARE, DENSE_MAX_M));
            g = grid - fract(grid) * fine;
            spacing = spacing - 1.0 + fine;
        }
        xz = corner.xy + g * step - fract(g * 0.5) * 2.0 * step * morph;
        spacing = spacing + morph;
    }
    let lod = spacing + log2(terrain.field.w / max(terrain.relief.z, 1e-3)) - 1.0;
    return vec3<f32>(xz.x, relief_height(xz, field_height(xz), lod), xz.y);
}

@vertex
fn vs_land(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    // A patch drawn dense is left out of the plain patches' draw (its copy is drawn with the dense mesh).
    if in.corner.w > 1.5 && in.corner.w < 2.5 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        out.local = vec3<f32>(0.0);
        out.world = vec3<f32>(0.0);
        out.level = 0.0;
        return out;
    }
    let local = place_vertex(in.grid, in.corner, true);
    let offset = chart_offset();
    let world = local + vec3<f32>(offset.x, 0.0, offset.y);
    out.clip = world_to_clip(world);
    out.local = local;
    out.world = world;
    out.level = in.corner.z;
    return out;
}

// `normal` tilted by the screen-space gradient of `bump` (metres of height): Mikkelsen's surface gradient.
fn bumped(normal: vec3<f32>, bump: f32, dx: vec3<f32>, dy: vec3<f32>, bump_dx: f32, bump_dy: f32) -> vec3<f32> {
    let r1 = cross(dy, normal);
    let r2 = cross(normal, dx);
    let det = dot(dx, r1);
    let surface_gradient = (r1 * bump_dx + r2 * bump_dy) * sign(det);
    return normalize(normal * abs(det) - surface_gradient);
}


// A colour for each ground material (the `material` debug view): its id's hue, none black.
fn material_hue(id: f32) -> vec3<f32> {
    if id > 14.5 {
        return vec3<f32>(0.0);
    }
    let h = id / 15.0 * 6.0;
    return clamp(vec3<f32>(abs(h - 3.0) - 1.0, 2.0 - abs(h - 2.0), 2.0 - abs(h - 4.0)), vec3<f32>(0.0), vec3<f32>(1.0))
        * (0.55 + 0.45 * f32(u32(id) % 2u));
}

@fragment
fn fs_land(in: VertexOut) -> @location(0) vec4<f32> {
    let chart = in.local.xz;
    let height = in.local.y;
    let fuv = field_uv(chart);
    let cuv = coarse_uv(chart);
    let ground_data = textureSample(attributes, clamp_sampler, cuv);
    let sky = textureSample(open_sky, clamp_sampler, cuv).r;
    let land_sun = textureSample(sunlight, clamp_sampler, fuv).r;
    // Metres a pixel spans on the land: procedural detail fades to its mean before it would alias.
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let footprint = max(max(length(dx), length(dy)), 0.01);
    // The field's slope, and the fine relief's near the camera (`relief_gradient`: none without its pages).
    let gradient = textureSample(gradients, clamp_sampler, fuv).xy + relief_gradient(chart, height, footprint);
    let macro_normal = normalize(vec3<f32>(-gradient.x, 1.0, -gradient.y));

    var inputs: SurfaceInputs;
    inputs.chart = chart;
    inputs.height = height;
    inputs.gradient = gradient;
    inputs.footprint = footprint;
    inputs.sky = sky;
    inputs.drainage = ground_data.r;
    inputs.broad = ground_data.g;
    inputs.coast = ground_data.b;
    inputs.fine = ground_data.a;
    inputs.chart_dx = dx.xz;
    inputs.chart_dy = dy.xz;
    // What a texture filtered with the samplers' 8× anisotropy averages over (the ground materials' fade).
    inputs.filtered = max(footprint / 8.0, max(min(length(dx), length(dy)), 0.01));
#ifdef TREES
    // The forest here (the trees standing per cell and their height), for its floor and its shade below.
    let trees_here = forest_at(chart);
    inputs.floor = floor_seen(trees_here, in.world);
    inputs.standing = cards_standing(trees_here, in.world);
#endif
    // The map features' screen derivatives here, in uniform control flow; nothing below takes any until the relief's.
    let features = feature_site(chart, footprint);
    // The seabed under a few metres of sea, seen only through it, is the seabed alone: the land's whole surface there
    // (its cover worked out and then painted over with the seabed) cost as much as the land. Above it the cover's
    // relief fades out toward it, so no seam shows along the depth line.
    var surface: Surface;
    if height < SEABED_PLAIN_M {
        surface = seabed_surface(inputs);
    } else {
        surface = terrain_surface(inputs, features);
        surface.bump = surface.bump * ramp(SEABED_PLAIN_M, SEABED_RELIEF_M, height);
    }
    // A clearing (a theatre airfield's apron): mown grass on the land, mottled by its fine relief, with no canopy's shade
    // or relief; the shore and the sea as they are.
    let clearing = clearing_at(chart, 0.0);
    let clear = clearing.x * ramp(0.3, 1.5, height);
    if clear > 0.0 {
        // A base's ground: bare earth and coral dust worn by wheels and boots, oil-dark here and there, kunai left only
        // toward the clearing's edge (deeper the more it wanders); over the land's own colour, with no canopy's shade.
        let broad = clearing_noise(chart / 41.0);
        let mid = clearing_noise(chart / 9.0 + 7.1);
        let fine = clearing_noise(chart / 2.1 + 1.7);
        let earth = mix(srgb(100.0, 84.0, 62.0), srgb(122.0, 104.0, 78.0), mid);
        var ground = mix(earth, srgb(144.0, 134.0, 112.0), smoothstep(0.5, 0.85, broad) * 0.6);
        ground = mix(ground, srgb(84.0, 74.0, 58.0), smoothstep(0.74, 0.95, clearing_noise(chart / 12.0 + 3.9)) * 0.3);
        ground *= 0.84 + 0.3 * fine;
        let kunai = mix(srgb(82.0, 98.0, 48.0), srgb(116.0, 114.0, 66.0), broad) * (0.86 + 0.26 * fine);
        // Grass toward the edge, and in tufts and patches where nothing drives.
        let tufts = smoothstep(0.62, 0.8, clearing_noise(chart / 13.0 + 11.3)) * 0.6;
        let grassy = max(1.0 - smoothstep(4.0, 14.0 + 16.0 * mid, clearing.y), tufts);
        ground = mix(ground, kunai, grassy * (0.55 + 0.4 * mid));
        // What stands among the trees (a soft capsule: a camp's huts, tents, streets and roads, a dump, a revetment, a
        // taxiway in the jungle): the land's own ground at its edge, worn through to earth in patches, trodden and
        // driven to earth with grass in tufts some metres in (so a road's or a revetment's edge meets earth).
        let hardness = clamp(clearing.z / max(clearing.x, 1e-3), 0.0, 1.0);
        let worn = smoothstep(0.3, 0.75, mid * 0.5 + fine * 0.25 + broad * 0.25);
        let core = smoothstep(0.5, 5.0, clearing.y);
        let soft = clear * (1.0 - hardness) * mix(0.15 + 0.35 * worn, 0.88 + 0.1 * fine, core);
        let trodden = mix(earth * (0.9 + 0.2 * fine), kunai, 0.3 * smoothstep(0.4, 0.8, broad) + 0.5 * tufts);
        // A temperate base (Europe's): its field mown grass over the land's own colour, so its fields' and hedges'
        // marks still show faintly through and its edge melts into the farmland; trodden ground muddy grass.
        let mild = clamp(clearing.w / max(clearing.x, 1e-3), 0.0, 1.0);
        let mown = mix(srgb(76.0, 98.0, 48.0), srgb(98.0, 114.0, 58.0), broad) * (0.88 + 0.22 * fine);
        let muddy = mix(mown, earth * 0.8, 0.1 + 0.25 * worn);
        // Mown grass: the land's own colour, a touch greener, evened toward a mown green by an eighth, so a field keeps
        // its farmland's tone (Kaldaðarnes's at 45%, and at a quarter, read as a bright green disc on Iceland's grass).
        let evened = mix(surface.albedo * vec3<f32>(0.98, 1.02, 0.97), tone_down(mown), 0.12);
        let bare = mix(tone_down(ground), evened, mild);
        let walked = tone_down(mix(trodden, muddy, mild));
        surface.albedo = mix(surface.albedo, bare, clear * 0.9 * hardness);
        surface.albedo = mix(surface.albedo, walked, soft * mix(1.0, 0.6, mild));
        // A temperate field keeps most of its relief and shading: flattened, Kaldaðarnes's read as a bright disc.
        let flat = clear * mix(0.3, 1.0, hardness) * mix(1.0, 0.2, mild);
        surface.roughness = mix(surface.roughness, 0.95, flat);
        surface.occlusion = mix(surface.occlusion, 1.0, flat);
        surface.bump = surface.bump * (1.0 - flat);
        surface.slope = surface.slope * (1.0 - flat);
    }
    let bump_dx = dpdx(surface.bump);
    let bump_dy = dpdy(surface.bump);
#ifdef DETAILED
    // The ground materials' relief tilts the land's own surface (a slope added to its gradient), then the small relief.
    let detailed_normal = normalize(vec3<f32>(-gradient.x - surface.slope.x, 1.0, -gradient.y - surface.slope.y));
    let normal = bumped(detailed_normal, surface.bump, dx, dy, bump_dx, bump_dy);
#else
    let normal = macro_normal;
#endif

    // Debug views (`TERRAIN_DEBUG`) replace the albedo with black and add one input as emission, in the display's
    // middle range; the land's specular light stays.
    let debug = i32(terrain.offset.w + 0.5);
    var emissive = vec3<f32>(0.0);
    var albedo = surface.albedo;
    if debug != 0 {
        var shown = surface.albedo;
        switch debug {
            case 2: { shown = normal * 0.5 + 0.5; }
            case 3: { shown = vec3<f32>(land_sun); }
            case 4: { shown = vec3<f32>(sky); }
            case 5: { shown = vec3<f32>(surface.occlusion); }
            case 6: { shown = vec3<f32>(fract(in.level * 0.37), fract(in.level * 0.61 + 0.3), fract(in.level * 0.83 + 0.6)); }
            case 7: { shown = vec3<f32>(ground_data.b, ramp(0.49, 0.51, ground_data.b), 0.0); }
            case 8: { shown = vec3<f32>(ground_data.g, ground_data.a, 0.5); }
            case 9: { shown = vec3<f32>(length(gradient) / 1.5); }
            case 10: { shown = material_hue(surface.material.x) * surface.material.y + material_hue(surface.material.z) * surface.material.w; }
            default: {}
        }
        emissive = shown * shown * 0.45;
        if debug == 1 {
            // The albedo, measurable: linear and with no specular light, beside a strip of known albedos along the
            // view's bottom edge (0 to 0.6 left to right, in sixteen steps of 0.04) through the same display, so a
            // still's pixels read back as albedo (each against the strip's step of the same displayed luminance).
            var known = surface.albedo;
            if in.clip.y > camera.viewport.y - 48.0 {
                known = vec3<f32>(floor(in.clip.x * 16.0 / camera.viewport.x) * 0.04);
            }
            emissive = known * 0.45;
            surface.specular = 0.0;
        }
        albedo = vec3<f32>(0.0);
    }

    let view = normalize(camera.position.xyz - in.world);
    let roughness = land_roughness(surface.roughness);
    // The sun, shaded by the land's own shadow, the shadow maps and the clouds. The maps hold the ships', the trees' and
    // the buildings', and where the land casts into them its own again (native/docs/terrain/shadows.md, "The land's
    // shadow"):
    // the darker of the baked shadow and the maps', so a slope the land shades is not shaded twice where a map holds
    // the same shadow, and beyond the maps (which fade to lit) the baked shadow alone, with no seam where they end.
    // Then the smoke's (soft occluders), as `sun_shadow` takes it.
    var sun = terrain.sun_radiance.rgb * min(land_sun, sun_shadow_maps(in.world, normal)) * occluder_transmittance(in.world);
    if terrain.equator.z > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, in.world);
    }
#ifdef TREES
    // Under the trees: the forest floor's sun past the trees' shadows (beyond the maps they cast into) and its sky.
    let forest = forest_floor(trees_here, in.world);
    sun *= forest.x;
    surface.occlusion *= forest.y;
#endif
    var light: LandLight;
    light.sun_direction = terrain.sun_direction.xyz;
    light.sun_radiance = sun;
    light.fill = mix(terrain.fill_ground.rgb, terrain.fill_sky.rgb, dot(normal, vec3<f32>(0.0, 1.0, 0.0)) * 0.5 + 0.5);
    let intensity = terrain.sun_radiance.w;
    let direction = env_reflection_direction(normal, view, roughness);
    light.radiance = env_radiance(sky_atlas, sky_sampler, sky_env, direction, roughness) * intensity;
    light.irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, normal) * intensity;
#ifdef TREES
    // Under the leaves the light, and what the ground reflects, is the leaves' (`under_leaves`).
    let daylight = terrain.sun_radiance.rgb * max(terrain.sun_direction.y, 0.0) + light.irradiance;
    light.fill = under_leaves(light.fill, daylight, forest.z);
    light.radiance = under_leaves(light.radiance, daylight, forest.z);
    light.irradiance = under_leaves(light.irradiance, daylight, forest.z);
#endif
    light.world = in.world;
    var lit = shade_land(albedo, roughness, surface.specular, surface.occlusion, normal, view, light) + emissive;
    // The ground under the street lights and the towns' glow at night (`night.wgsl`).
    if debug == 0 {
        lit += night_land(features, surface.built, surface.photo_detail, albedo, chart, height, footprint);
    }
    return vec4<f32>(fogged(lit, in.world), 1.0);
}
