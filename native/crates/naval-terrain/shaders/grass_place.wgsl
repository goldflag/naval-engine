// Grass about the camera (native/docs/terrain/trees.md, "Grass"): a clump of blades on each cell of a grid fixed to the
// chart
// about the eye, every cell's out to `reach.x` and beyond it fewer by the square of the distance (each cell's own rank
// against the share its distance leaves, so a clump grows up out of the ground as the camera nears and none pops),
// none past `reach.y`. A clump stands where the land is grass (its cover's grassland and meadow, a pasture, the ground
// between shrubs, a wetland's reeds; a battle map's style) and its own materials are (`grass_of`), never on a beach,
// snow, a road or a river, a building's clearing, or ground too steep, and takes the ground's own colour there
// (`terrain_surface`), so the grass is the land's and fades into it; it stands as tall as its ground's grass grows (a
// grazed pasture short, a hay meadow and a wetland's reeds tall). Written as instances for `grass.wgsl`, counted into
// its indirect draw.
#import naval_terrain::bindings::{terrain, gradients, attributes, open_sky, sunlight, clamp_sampler, repeat_sampler, surface_albedo, height_at, field_uv, coarse_uv, chart_origin, cleared, surface_uv, surface_bound}
#import naval_terrain::features::{FeatureSite, feature_clearance}
#import naval_terrain::cells::cell_hash
#import naval_terrain::materials::{M_GRASS, M_DRY_GRASS, M_SCRUB, M_DRY_SCRUB, M_MUD}
#import naval_terrain::surface::{SurfaceInputs, CoverShares, Ground, ground_of, ramp, climate_snow, cover_shares, near_cover_shares, mixed_shares, near_bound, near_share, near_uv, surface_inside, terrain_surface, country_field, FIELD_PASTURE, FIELD_HAY, FIELD_ROUGH, FIELD_ORCHARD, FIELD_OLIVES, FIELD_PALMS, FIELD_WOOD, FIELD_YARD}

struct GrassView {
    // The view's frustum in chart metres (n · p + w ≥ 0 inside).
    planes: array<vec4<f32>, 6>,
    // The eye in chart metres; w: a pixel's angle (radians), for the ground's footprint.
    eye: vec4<f32>,
    // The grid's first cell (x, z, in cells from the chart's origin), its cells a side, a cell (m).
    grid: vec4<f32>,
    // Out to here every cell stands its clump (m), none past here (m), the room for clumps, 0.
    reach: vec4<f32>,
}

// A clump: its foot (chart x, the ground, chart z) and the patch its blades stand on (radius, m); its colour (the
// square root of the linear albedo, for the bytes' precision) and open sky packed (`pack4x8unorm`); how far it has
// grown in (0-1), the sun past the land at its foot and the land's slope there (a quarter of it about 0.5) packed; its
// blades' height (m, a float's bits); its seed.
struct Clump {
    foot: vec4<f32>,
    look: vec4<u32>,
}

struct DrawArgs {
    vertex_count: u32,
    instance_count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
}

@group(0) @binding(0) var<uniform> view: GrassView;
@group(0) @binding(1) var<storage, read_write> clumps: array<Clump>;
@group(0) @binding(2) var<storage, read_write> args: DrawArgs;

fn in_view(centre: vec3<f32>, radius: f32) -> bool {
    for (var k = 0; k < 6; k += 1) {
        let plane = view.planes[k];
        if dot(plane.xyz, centre) + plane.w < -radius {
            return false;
        }
    }
    return true;
}

// The surface inputs at a chart point on the land, a pixel there spanning `footprint` m (as `tree_place.wgsl` reads
// them, at the footprint the land is drawn with there).
fn inputs_at(foot: vec2<f32>, ground: f32, gradient: vec2<f32>, footprint: f32) -> SurfaceInputs {
    let cuv = coarse_uv(foot);
    let ground_data = textureSampleLevel(attributes, clamp_sampler, cuv, 0.0);
    var inputs: SurfaceInputs;
    inputs.chart = foot;
    inputs.height = ground;
    inputs.gradient = gradient;
    inputs.footprint = footprint;
    inputs.sky = textureSampleLevel(open_sky, clamp_sampler, cuv, 0.0).r;
    inputs.drainage = ground_data.r;
    inputs.broad = ground_data.g;
    inputs.coast = ground_data.b;
    inputs.fine = ground_data.a;
    inputs.filtered = max(footprint / 8.0, 0.01);
    return inputs;
}

// How much of the ground is grass (0-1) and how tall it grows against a meadow's: the window maps' cover where they know
// it, else the style's.
fn grass_cover(inputs: SurfaceInputs) -> vec2<f32> {
    var grass = vec2<f32>(0.6, 1.0);
#ifdef STYLE_TROPICAL
    grass = vec2<f32>(0.75, 1.2);
#endif
#ifdef STYLE_VOLCANIC
    grass = vec2<f32>(0.45, 0.8);
#endif
#ifdef STYLE_SNOW
    grass = vec2<f32>(0.25, 0.6);
#endif
#ifdef STYLE_CHALK
    grass = vec2<f32>(0.85, 0.7);
#endif
    if surface_bound() {
        let uv = surface_uv(inputs.chart);
        var weight = textureSampleLevel(surface_albedo, clamp_sampler, uv, 0, 0.0).a * surface_inside(uv);
        var s = cover_shares(inputs);
        if near_bound() {
            let near = near_share(inputs.chart)
                * textureSampleLevel(surface_albedo, repeat_sampler, near_uv(inputs.chart), 1, 0.0).a;
            weight = weight * (1.0 - near) + near;
            s = mixed_shares(s, near_cover_shares(inputs), near);
        }
        if weight > 0.0 {
            // Grassland and moss whole, a wetland's reeds, the ground between shrubs, a field's grass, a wood's floor
            // and a town's verges a little.
            // The reeds stand tall, the grass between shrubs and on a wood's floor shorter, a town's verges mown.
            let shares = array<f32, 6>(s.a.w, s.c.y * 0.9, s.a.z * 0.6, s.b.x * 0.45, s.a.x * 0.25 + s.a.y * 0.15,
                (s.b.y + s.b.z) * 0.08);
            let heights = array<f32, 6>(1.0, 1.7, 0.85, 1.0, 0.7, 0.45);
            var mapped = vec2<f32>(0.0);
            for (var k = 0; k < 6; k += 1) {
                mapped += vec2<f32>(shares[k], shares[k] * heights[k]);
            }
            mapped.y /= max(mapped.x, 1e-4);
            // The period's country: its pastures (grazed short), hay (tall before the mowing) and rough grazing grass,
            // an orchard's or a grove's floor half, a ploughed or sown field's soil none.
            if s.past * weight > 0.5 {
                let field = country_field(inputs);
                switch field.kind {
                    case FIELD_PASTURE: {
                        mapped = vec2<f32>(1.0, 0.55);
                    }
                    case FIELD_HAY: {
                        mapped = vec2<f32>(1.0, 1.3);
                    }
                    case FIELD_ROUGH: {
                        mapped = vec2<f32>(1.0, 1.0);
                    }
                    case FIELD_ORCHARD, FIELD_OLIVES, FIELD_PALMS: {
                        mapped = vec2<f32>(0.6, 0.8);
                    }
                    case FIELD_WOOD: {
                        mapped = vec2<f32>(0.25, 0.7);
                    }
                    case FIELD_YARD: {
                        mapped = vec2<f32>(0.1, 0.45);
                    }
                    default: {
                        mapped = vec2<f32>(0.0, 1.0);
                    }
                }
            }
            grass = mix(grass, mapped, weight);
        }
    }
    return grass;
}

// How much of a ground material is grass: grass and dry grass whole, scrub half (the grass between its bushes), mud a
// little more (a marsh's reeds stand in it); a wood's leaf litter, soil, rock, sand and paving none.
fn grass_of(material: f32) -> f32 {
    let id = u32(material + 0.5);
    if id == M_GRASS || id == M_DRY_GRASS {
        return 1.0;
    }
    if id == M_MUD {
        return 0.6;
    }
    if id == M_SCRUB || id == M_DRY_SCRUB {
        return 0.5;
    }
    return 0.0;
}

@compute @workgroup_size(8, 8)
fn place_grass(@builtin(global_invocation_id) id: vec3<u32>) {
    let cells = u32(view.grid.z);
    if any(id.xy >= vec2<u32>(cells)) {
        return;
    }
    let size = view.grid.w;
    let cell = view.grid.xy + vec2<f32>(id.xy);
    // Salted apart from the trees' and the ground cover's cells.
    let key = cell + vec2<f32>(15731.0, 789221.0);
    let foot = chart_origin() + (cell + vec2<f32>(cell_hash(key, 1.0), cell_hash(key, 2.0))) * size;
    let full = view.reach.x;
    let range = view.reach.y;
    let level = length(foot - view.eye.xz);
    let share = min(pow(full / max(level, 1.0), 2.0), 1.0);
    let rank = cell_hash(key, 3.0);
    if level > range || rank >= share {
        return;
    }
    let ground = height_at(foot);
    if ground < 1.5 {
        return;
    }
    let distance = length(view.eye.xyz - vec3<f32>(foot.x, ground, foot.y));
    // The patch a clump stands for: its cell, and as many more as the thinning leaves to it.
    let patch_m = size * 0.75 / sqrt(share);
    if distance > range || !in_view(vec3<f32>(foot.x, ground + 0.3, foot.y), 0.8 + patch_m) {
        return;
    }
    let gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(foot), 0.0).xy;
    if length(gradient) > 0.75 {
        return;
    }
    let footprint = max(distance * view.eye.w, 0.002);
    let inputs = inputs_at(foot, ground, gradient, footprint);
    let g = ground_of(inputs);
    // None on snow, a beach, a clearing, or a road, railway or river (`features.wgsl`).
    if climate_snow(inputs, g) > 0.3 || g.beach > 0.3 || cleared(foot) > 0.0 || feature_clearance(foot) > 0.2 {
        return;
    }
    let cover = grass_cover(inputs);
    // Little under a wood's own shade: the trees standing over it (`forest.wgsl`'s `forest_at`, the open sky's green).
    let canopy = textureSampleLevel(open_sky, clamp_sampler, coarse_uv(foot), 0.0).g;
    let density = cover.x * (1.0 - ramp(0.45, 0.75, length(gradient))) * (1.0 - 0.85 * canopy);
    let kept = share * density;
    if rank >= kept {
        return;
    }
    // The ground there, as the land draws it at this distance (no map features: the grass keeps off them): its colour,
    // and its two heaviest materials, the share of it they make grass taken from the clump's (so a wood's leaf litter
    // carries none, where the few tufts left by the canopy took the litter's colour and stood black on its floor).
    var site: FeatureSite;
    site.distance = vec4<f32>(1.0);
    site.attrs = vec4<f32>(0.0);
    site.px = vec4<f32>(1.0);
    site.share = 0.0;
    site.level = 1;
    let surface = terrain_surface(inputs, site);
    let materials = surface.material;
    let grassy = kept * (grass_of(materials.x) * materials.y + grass_of(materials.z) * materials.w);
    // Grown in over the last quarter of its share's rank, and down into the ground toward the reach's end.
    let growth = clamp((grassy - rank) / max(grassy * 0.25, 1e-4), 0.0, 1.0)
        * (1.0 - ramp(range * 0.65, range, distance));
    if growth < 0.02 {
        return;
    }
    let color = sqrt(clamp(surface.albedo, vec3<f32>(0.0), vec3<f32>(1.0)));
    // Taller on wet low ground, shorter on dry knolls.
    let tall = mix(0.14, 0.34, cell_hash(key, 4.0)) * cover.y * mix(0.85, 1.25, ramp(0.3, 0.7, inputs.drainage))
        * mix(1.0, 0.75, g.knoll);
    let sun = textureSampleLevel(sunlight, clamp_sampler, field_uv(foot), 0.0).r;
    let slope = clamp(gradient * 0.25 + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let index = atomicAdd(&args.instance_count, 1u);
    if index >= arrayLength(&clumps) {
        // Full: take the count back, so the draw never reads past the buffer.
        atomicSub(&args.instance_count, 1u);
        return;
    }
    clumps[index] = Clump(
        vec4<f32>(foot.x, ground, foot.y, patch_m),
        vec4<u32>(
            pack4x8unorm(vec4<f32>(color, inputs.sky)),
            pack4x8unorm(vec4<f32>(growth, sun, slope)),
            bitcast<u32>(tall),
            u32(cell_hash(key, 5.0) * 16777216.0),
        ),
    );
}
