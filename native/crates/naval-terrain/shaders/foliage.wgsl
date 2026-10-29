// The trees' light, which their impostors (`trees.wgsl`) and the near trees' meshes (`near_trees.wgsl`) share, so the
// crossfade between them shows no seam (moved here from `trees.wgsl`).
//
// The light is the impostors' since 2026-10-04: the sun wrapped round a crown past its terminator and shining through
// its leaves toward an eye that looks toward the sun, the sky and the fill about the normal bent up, the other trees'
// shadows read in front of the tree's own caster card (`tree_caster.wgsl`, a card facing the light through the tree's
// middle, so a crown never shades itself through the map), the forest's stand-in beyond the maps. Since 2026-10-26 a
// tree shades itself by its crown's light (`naval_terrain::crown`): the share of the sky each point sees through the
// crown takes its sky and fill whole (no floor), its leaves' sun by their depth in the crown, its bark's sun by where
// the open sky lies (a trunk under its crown sees a low sun under the crown's skirt, never a high one). The grass's
// blades (`grass.wgsl`) take the ground's light instead (`light_grass`).
#define_import_path naval_terrain::foliage

#import naval::camera::{camera}
#import naval::shadow::{occluder_transmittance, shadows, shadow_cascades, shadow_wide, shadow_sampler, shadow_cover, shadow_coord, shadows_enabled, shadow_land_on, shadow_land, SHADOW_ORDER, SHADOW_HELD}
#import naval_terrain::bindings::{terrain, chart_offset}
#import naval_terrain::surface::{ramp}
#import naval_terrain::lighting::{LandLight, shade_land, land_roughness, fogged}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#import naval_terrain::forest::{forest_at, forest_floor, tree_map_share, crown_shade, under_leaves}
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction}

@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;

@group(3) @binding(0) var cloud_map: texture_2d<f32>;
@group(3) @binding(1) var cloud_sampler: sampler;
@group(3) @binding(2) var<uniform> cloud_params: CloudShadow;

const RECIPROCAL_PI: f32 = 0.3183098861837907;
// How far the sun's light wraps round a crown past its terminator (its leaves face every way).
const WRAP: f32 = 0.5;
// How much of the sun another crown's shadow takes from a crown (its foliage lets some through, and the light inside a
// canopy scatters into a shadow).
const CROWN_SHADOW: f32 = 0.7;
// The sky through its crown under which a leaf's sun falls off (smoothly to none in the dark heart: a metre into a closed
// crown, a third of the sky, about half the sun), and the share of the sun a crown's leaves let through onto bark in
// their shade.
const LEAF_SUN_SKY: f32 = 0.45;
// The light a crown's leaves scatter on (they pass on about a third of the green they take) reaches deeper into it than
// the sky seen straight through: its leaves' sky and fill fall off as their sky to this power (Goudriaan's diffuse
// extinction, k times the root of 1 - 0.3).
const LEAF_SCATTER: f32 = 0.84;
const BARK_THROUGH: f32 = 0.07;
// A leaf passes on through it about as much green as it sends back (about a tenth each), less of the red and far less
// of the blue: the light through a leaf is yellower than the leaf seen lit.
const LEAF_TRANSMIT: vec3<f32> = vec3<f32>(1.1, 1.25, 0.55);
// The share of the sun on a clump's lit side that reaches the leaves on its far side, a leaf or two of it between
// (about e^-1.5): they take it on their inner faces, and an eye that sees their outer faces sees it passed through.
const CLUMP_THROUGH: f32 = 0.22;
// The light a crown's leaves scatter fills it: its sunlit outer leaves pass on inward about a tenth of the green on
// them (and send back as much of what comes from inside), so inside a sunlit crown a leaf takes about a fifteenth of
// the sun in green from every side, a deep green (each leaf passes on and sends back about a fifth of the green on it,
// a tenth of the red, a twentieth of the blue); the sky's light twice as much, as it lights the whole crown and the sun
// its one side. None at the open rim, growing with the depth as the sun and the sky seen straight through fall away:
// the heart takes a dark green from it, not black, in the land's shadow too. Its colour is the crown's leaves' (a
// typical leaf's share of each colour, `LEAF_OMEGA`, whatever this leaf's own brightness), and the limbs inside the
// crown take it as its leaves do: deep in the crown only, so a trunk under its skirt keeps its shade. What the leaves
// pass on the leaves about them take and pass on again, and so on: every order of it together, w / (1 - w) of the
// light for a leaf's share w (a fifth of the green, a tenth of the red, a twentieth of the blue), a quarter more green
// than the first order alone (with the first alone a wood's undersides drew a fifth darker than the old trees').
const CROWN_SCATTER: f32 = 0.7;
const LEAF_OMEGA: vec3<f32> = vec3<f32>(0.1236, 0.25, 0.0526);
// A bare crown's twigs are a mass of thin round twigs, not a surface. The sun reaches through the thin mass to its far
// side (about a third of it: a winter crown passes most of the light), and of the sun on its twigs the eye sees their
// lit sides less as it looks toward the sun (a round twig's phase, as a sphere's about ((1 + cos) / 2)^1.6, softened
// toward the first power for the lit land the gaps between its twigs show behind them).
const TWIG_THROUGH: f32 = 0.3;
const TWIG_PHASE: f32 = 1.3;
// In a wood the neighbours close the sky low about a trunk under the canopy's top as its own crown closes it above:
// the open ring a trunk sees under its crown's skirt is their crowns and trunks. At a full wood's foot this share of
// its sky is gone, none at the canopy's top. (A crown's leaves seen at all look out through the gaps the neighbours
// leave, and keep their own crown's sky.)
const NEIGHBOUR_SKY: f32 = 0.7;
// The mean direction of the crown's sky rays (`crown::SKY_MEAN`): a point's open sky toward a direction `d` is about
// its sky plus its slope dotted with `d` less this.
const SKY_MEAN: vec3<f32> = vec3<f32>(0.0, 0.6666667, 0.0);
// How far a grass blade's shading normal is bent from its own toward the land's, its roughness and its waxy face's
// specular share (`shade_land`), and the share of the sun on one face a blade lets through to the other.
const GRASS_GROUND_NORMAL: f32 = 0.75;
const GRASS_ROUGHNESS: f32 = 0.7;
const GRASS_SPECULAR: f32 = 0.3;
const GRASS_TRANSMIT: f32 = 0.45;

// A tree's own tint on a point of it: the leaves take it whole (a eucalyptus' blue-grey, a cypress' dark green, each
// tree's hue), its bark half its brightness only (a blue gum's trunk is pale by its own bark, not blue; a tint of
// 1.2 made pale bark glow).
fn tree_tint(tint: vec3<f32>, leafy: f32) -> vec3<f32> {
    let brightness = dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(mix(1.0, brightness, 0.5)), tint, leafy);
}

// A point of a tree to light.
struct Foliage {
    // Linear albedo, toned and tinted.
    albedo: vec3<f32>,
    // The point (world) and its shading normal.
    world: vec3<f32>,
    normal: vec3<f32>,
    // The tree's middle (world).
    middle: vec3<f32>,
    // How much of it is leaves (0 bark), how much a bare crown's twigs (lit as the crown's mass, as its leaves would be,
    // but neither passing light through nor scattering it), the share of the sky it sees through its crown (0 none,
    // 1 open: `naval_terrain::crown`), its height up the tree (0 at the foot, 1 at the top).
    leafy: f32,
    twig: f32,
    sky: f32,
    up_tree: f32,
    // The slope of that sky (world; w 1 where the point carries one, a mesh's, 0 an impostor's): toward a direction
    // `d` above the horizon it sees about `sky + dot(slope, d - SKY_MEAN)` of it.
    slope: vec4<f32>,
    // 1 for a trunk's or a limb's bark on a mesh, 0 for a card's leaves or twigs and for an impostor.
    tube: f32,
    // Its height (m).
    height: f32,
    // The open sky at the tree's foot, and the sun past the land there.
    open: f32,
    sun_at_foot: f32,
}

// How much of the sun bark sees past its own crown: by where its open sky lies where it carries the sky's slope (a
// mesh's point: `crown::bark_sun`), else (an impostor's) by its sky squared.
fn bark_sees_sun(f: Foliage, to_light: vec3<f32>) -> f32 {
    if f.slope.w > 0.5 {
        return smoothstep(0.1, 0.9, f.sky + dot(f.slope.xyz, to_light - SKY_MEAN));
    }
    return f.sky * f.sky;
}

// The light a point of a tree sends the eye, fogged.
fn light_foliage(f: Foliage) -> vec3<f32> {
    let view = normalize(camera.position - f.world);
    let roughness = land_roughness(0.9);
    // The sun past the land's own shadow, and past the other trees' crowns: their shadows in the sun's maps, read in
    // front of the tree's own caster card, so a crown never shades itself through the map (its own crown's light is
    // its sky and normal); and where the maps carry no tree, the neighbours toward the sun a forest's crowns stand in
    // for its lower crowns on average. The maps are read through the coarser filter (`coarse_sun_shadow_maps`): a crown's
    // leaves break a shadow's edge up finer than the soft filter's kernel (it took a fifth to a third of the trees' cost
    // once each pixel was lit once, 2026-10-26).
    let to_light = terrain.sun_direction.xyz;
    let behind = dot(f.world - f.middle, to_light);
    let lookup = f.world + to_light * (max(-behind, 0.0) + 1.5);
    let shadowed = coarse_sun_shadow_maps(lookup, to_light) * occluder_transmittance(lookup);
    let forest = forest_at(f.world.xz - chart_offset());
    let neighbours = crown_shade(forest, f.up_tree, to_light.y) * (1.0 - tree_map_share(f.world));
    // Near the ground another crown's shadow is the whole canopy's, as on the floor about it: a shrub under the pines
    // took the light a crown's upper leaves let through and stood out pale against the shaded floor. A trunk or a limb
    // takes another crown's shadow as it takes its own's, but for the little the leaves let through (a trunk in a wood
    // took a third of the sun at every height above 5 m and stood pale among the shaded leaves); a bare crown's twigs
    // keep the leaves' third (whole, a wood of bare oaks drew black).
    let crown_shadow = mix(mix(0.95, CROWN_SHADOW, ramp(1.5, 5.0, f.up_tree * f.height)), 1.0 - BARK_THROUGH, f.tube);
    var sun = terrain.sun_radiance.rgb * f.sun_at_foot * mix(1.0, shadowed, crown_shadow) * (1.0 - crown_shadow * neighbours);
    if terrain.equator.z > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, f.world);
    }
    // The sky and the fill reach a crown from above as much as from the side it faces (its leaves and twigs face every
    // way): taken about a normal bent up. Bark is a surface, and takes them about its own normal (bent up too, a trunk
    // in the open took the sky of a face turned up to it, and a beach's palms stood pale from the sea).
    let mass = max(f.leafy, f.twig);
    let sky_normal = normalize(f.normal + vec3<f32>(0.0, 0.45 * mass, 0.0));
    var light: LandLight;
    light.sun_direction = to_light;
    light.sun_radiance = vec3<f32>(0.0);
    light.fill = mix(terrain.fill_ground.rgb, terrain.fill_sky.rgb, sky_normal.y * 0.5 + 0.5);
    let intensity = terrain.sun_radiance.w;
    light.irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, sky_normal) * intensity;
    light.world = f.world;
    // What the sunlit ground about the tree throws back up onto its trunk and its crown's underside (the land's albedo
    // about 0.12, a little warm, as the buildings take it), past the land's own shadow at its foot: without it a
    // shaded trunk took the sky's blue alone and read navy.
    let below = 0.5 - 0.5 * f.normal.y;
    light.irradiance += terrain.sun_radiance.rgb * max(to_light.y, 0.0) * f.sun_at_foot * vec3<f32>(0.13, 0.12, 0.1)
        * below;
    // Leaves are as rough as a surface gets: the sky's reflection off them is its cosine-weighted light (a second
    // lookup of the environment a pixel saved).
    light.radiance = light.irradiance * RECIPROCAL_PI;
    // All of it, the ground's light too (the ground under a crown lies in its shade), as much as the crown leaves the
    // point and the land around leaves the tree's foot, with no floor: a leaf in the crown's heart and a trunk under it
    // see little of either. The neighbours close the sky about bark only (a crown's leaves and twigs look out through
    // the gaps they leave).
    let wood = 1.0 - NEIGHBOUR_SKY * forest.cover * (1.0 - f.up_tree) * (1.0 - mass);
    let occlusion = f.open * f.open * mix(f.sky, pow(max(f.sky, 0.0), LEAF_SCATTER), f.leafy) * wood;
    let indirect = shade_land(f.albedo, roughness, 0.2, occlusion, sky_normal, view, light);
    // The sun. On a crown's mass wrapped round the clump (its leaves and twigs face every way), falling off with the
    // depth in the crown its sky gives (the light inside a crown falls about exponentially with the leaves it has
    // passed). On bark as a surface takes it, as much as it sees past its crown, and a little through the leaves.
    let facing = dot(f.normal, to_light);
    let reach = smoothstep(0.0, LEAF_SUN_SKY, f.sky);
    let sunward = clamp((facing + WRAP) / (1.0 + WRAP), 0.0, 1.0);
    let lit = sunward * reach;
    let twigs_lit = mix(TWIG_THROUGH, 1.0, sunward) * reach * pow(0.5 + 0.5 * dot(view, to_light), TWIG_PHASE);
    let seen = bark_sees_sun(f, to_light);
    let bark_sun = max(facing, 0.0) * (seen + (1.0 - seen) * BARK_THROUGH);
    let wrapped = mix(bark_sun, mix(lit, twigs_lit, f.twig), mass);
    // Leaves on the far side of their clump from the sun take the sun a leaf or two of it let through, and pass it on
    // to the eye that sees them from that side, yellow-green: a crown glows against the sun, and from under it.
    let back = clamp((-facing + WRAP) / (1.0 + WRAP), 0.0, 1.0) * reach * CLUMP_THROUGH * f.leafy;
    // And toward an eye that looks toward the sun, at the crown's open rim, the light the leaves pass on forward: more at
    // the thin edge than through the middle.
    let forward = clamp(dot(-view, to_light), 0.0, 1.0);
    let thin = ramp(0.45, 1.0, 1.0 - abs(dot(f.normal, view)));
    let rim = smoothstep(0.0, 0.6, f.sky);
    let through = pow(forward, 4.0) * 0.45 * mix(0.3, 1.0, thin) * rim * (f.up_tree * 0.4 + 0.6) * f.leafy;
    let passed = f.albedo * LEAF_TRANSMIT;
    // The light the crown's leaves scatter, on the leaves and limbs inside it: a share of the sun and the sky on the
    // crown that the leaves have coloured once already (their reflectance and transmittance together), so a deep green.
    let inside = mix(1.0 - smoothstep(0.0, 0.6, f.sky), 1.0, f.leafy) * (1.0 - f.twig);
    let scattered = CROWN_SCATTER * (1.0 - f.sky) * inside * LEAF_OMEGA;
    let on_crown = sun * 0.5 + light.irradiance * (f.open * f.open);
    let direct = (sun * (f.albedo * wrapped + passed * (back + through)) + on_crown * f.albedo * scattered)
        * RECIPROCAL_PI;
    return fogged(indirect + direct, f.world);
}

// One square map's shadow at `coord` (1 lit) through four bilinear comparisons half a texel either side, a kernel three
// texels across whose weights change linearly as a receiver moves (the land maps' filter,
// `naval::shadow::shadow_land_compare`): about a third of the soft filter's nine gathers. One comparison while the soft
// filter is off.
fn coarse_compare(index: u32, coord: vec3<f32>) -> f32 {
    let d = 0.5 * shadows.maps[index].params.y * shadows.settings.y;
    let z = coord.z;
    let a = coord.xy + vec2<f32>(-d, -d);
    let b = coord.xy + vec2<f32>(d, -d);
    let c = coord.xy + vec2<f32>(-d, d);
    let e = coord.xy + vec2<f32>(d, d);
    if index == 1u {
        return 0.25 * (textureSampleCompareLevel(shadow_wide, shadow_sampler, a, z)
            + textureSampleCompareLevel(shadow_wide, shadow_sampler, b, z)
            + textureSampleCompareLevel(shadow_wide, shadow_sampler, c, z)
            + textureSampleCompareLevel(shadow_wide, shadow_sampler, e, z));
    }
    let layer = select(i32(index) - 1, 0, index == 0u);
    return 0.25 * (textureSampleCompareLevel(shadow_cascades, shadow_sampler, a, layer, z)
        + textureSampleCompareLevel(shadow_cascades, shadow_sampler, b, layer, z)
        + textureSampleCompareLevel(shadow_cascades, shadow_sampler, c, layer, z)
        + textureSampleCompareLevel(shadow_cascades, shadow_sampler, e, layer, z));
}

// The sun past the depth maps as `naval::shadow::sun_shadow_maps` takes it (the near, aboard and wide maps, the view
// maps, the land maps beyond them), each square map read through `coarse_compare` rather than the soft filter: for a
// receiver whose own fine detail breaks a shadow's edge up anyway (a crown's leaves, a grass blade; the images moved by
// about a thousandth). The soft filter took two thirds of the grass's cost (0.6 of 0.9 ms in a pasture at an eye's
// height) and 0.5-0.6 ms of the trees' in Kawah Lagoon's views, 2026-10-26.
fn coarse_sun_shadow_maps(position: vec3<f32>, normal: vec3<f32>) -> f32 {
    if !shadows_enabled() {
        return 1.0;
    }
    var weights = array<f32, 5>(0.0, 0.0, 0.0, 0.0, 0.0);
    var rest = 1.0;
    var beyond = 1.0;
    for (var k = 0u; k < 5u; k++) {
        let i = SHADOW_ORDER[k];
        let share = shadow_cover(i, position) * shadows.maps[i].state.x;
        weights[k] = rest * share;
        rest *= 1.0 - share;
        if k + 1u == SHADOW_HELD {
            beyond = rest;
        }
    }
    var held = 0.0;
    var outside = rest;
    for (var k = 0u; k < 5u; k++) {
        if weights[k] > 0.0 {
            let i = SHADOW_ORDER[k];
            let coord = shadow_coord(i, position, normal);
            var seen = 1.0;
            if all(coord.xy >= vec2<f32>(0.0)) && all(coord.xy <= vec2<f32>(1.0)) && coord.z <= 1.0 {
                seen = mix(1.0, coarse_compare(i, coord), shadows.settings.x);
            }
            if k < SHADOW_HELD {
                held += weights[k] * seen;
            } else {
                outside += weights[k] * seen;
            }
        }
    }
    if beyond > 0.0 && shadow_land_on() {
        outside = min(outside, beyond * shadow_land(position, normal));
    }
    return held + outside;
}

// A point of a grass blade to light (`grass.wgsl`).
struct Blade {
    // Linear albedo.
    albedo: vec3<f32>,
    // The point (world), the blade's normal (turned toward the eye) and the land's under it.
    world: vec3<f32>,
    normal: vec3<f32>,
    ground: vec3<f32>,
    // How much of the sky and the sun the blades about it leave it (least down among them at a root, 1 at a tip).
    occluded: f32,
    // The open sky at the blade's foot, and the sun past the land there.
    open: f32,
    sun_at_foot: f32,
}

// The light a point of a grass blade sends the eye, fogged: lit as the ground under it is (`terrain.wgsl`: the sun past
// the land's shadow and the maps', the forest floor's shade and sky, the land's own shading) about a normal bent most of
// the way to the land's, so a field seen from afar is its ground lit as the ground is; darker down among the blades,
// with a sheen off a blade's waxy face, and the sun shining through it: a face turned from the sun glows with what
// the blade lets through (without it the blades the eye saw turned from the sun stood black on the lit ground), more
// toward an eye that looks toward the sun.
fn light_grass(b: Blade) -> vec3<f32> {
    let view = normalize(camera.position - b.world);
    let to_light = terrain.sun_direction.xyz;
    let floor = forest_floor(forest_at(b.world.xz - chart_offset()), b.world);
    var sun = terrain.sun_radiance.rgb * min(b.sun_at_foot, coarse_sun_shadow_maps(b.world, b.ground)) * occluder_transmittance(b.world)
        * floor.x;
    if terrain.equator.z > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, b.world);
    }
    let normal = normalize(mix(b.normal, b.ground, GRASS_GROUND_NORMAL));
    let roughness = land_roughness(GRASS_ROUGHNESS);
    var light: LandLight;
    light.sun_direction = to_light;
    // The blades about it take the sun from its lower part.
    light.sun_radiance = sun * mix(0.35, 1.0, b.occluded);
    light.fill = mix(terrain.fill_ground.rgb, terrain.fill_sky.rgb, normal.y * 0.5 + 0.5);
    let intensity = terrain.sun_radiance.w;
    let direction = env_reflection_direction(normal, view, roughness);
    light.radiance = env_radiance(sky_atlas, sky_sampler, sky_env, direction, roughness) * intensity;
    light.irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, normal) * intensity;
    // Under the leaves, as the ground's light is (`terrain.wgsl`).
    let daylight = terrain.sun_radiance.rgb * max(to_light.y, 0.0) + light.irradiance;
    light.fill = under_leaves(light.fill, daylight, floor.z);
    light.radiance = under_leaves(light.radiance, daylight, floor.z);
    light.irradiance = under_leaves(light.irradiance, daylight, floor.z);
    light.world = b.world;
    let occlusion = b.open * b.open * floor.y * b.occluded;
    let lit = shade_land(b.albedo, roughness, GRASS_SPECULAR, occlusion, normal, view, light);
    // Through the blade, yellower than the blade seen lit.
    let forward = clamp(dot(-view, to_light), 0.0, 1.0);
    let through = GRASS_TRANSMIT * max(-dot(b.normal, to_light), 0.0) + pow(forward, 4.0) * 0.45 * b.occluded;
    let shine = light.sun_radiance * b.albedo * vec3<f32>(1.0, 1.15, 0.6) * through * RECIPROCAL_PI;
    return fogged(lit + shine, b.world);
}
