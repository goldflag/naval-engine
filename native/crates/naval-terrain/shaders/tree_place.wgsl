// Places the trees: one thread per tree slot of every level-0 and level-1 patch with land (`TREE_SLOTS`² a patch; a
// level-1 slot covers 2 x 2 cells and stands its tree in the one that ranks lowest), each finding its slot's tree from
// the same land-use masks that paint the ground (`tree_cover`), thinning the trees with distance and culling them to
// the view, choosing each tree's species, and appending the standing ones to the instance buffer the impostor draw
// instances (`trees.wgsl`) through an indirect draw whose instance count it counts. A second entry (`place_cover`)
// stands the ground cover, shrubs and tufts, on a fine grid about the camera into the same buffer.
//
// The slot's tree, its place, size and tint are `TerrainTrees.ts`' (`TreeMaterial`'s vertex stage), as the web placed
// them in every card's vertex, but for the tint's own jitter per tree; the level of detail is native
// (native/docs/terrain/trees.md): every slot carries a stratified rank, and a share (full / d)^2 of the slots keeps
// its tree at an apparent distance d (the spacing on screen stays the same), the lowest ranks first, those still
// standing grown by d / full to keep the forest's cover, each shrinking away over the last part of its own range so
// none pops.
//
// The species (`naval_terrain::species`): a battle map's trees are its style's (rainforest and palms, birch and spruce,
// oak and pine), each kind one species as the old cards were; a theatre's are chosen by its biome (`view.biome.x`,
// from where it stands) with the land cover at the foot, the height, the slope, the coast and stands of a few hundred
// metres (`choose`).
//
// The trees cast into the sun's near, wide and aboard maps (`tree_caster.wgsl`): a tree out of view whose foot lies in one of
// them stands too, so its shadow falls into the view, and the patches past the drawn ones (`view.shadow.zw`) are
// level-0 patches inside the maps that the view left out, carrying such trees only.
#import naval_terrain::bindings::{terrain, gradients, attributes, open_sky, clamp_sampler, repeat_sampler, surface_albedo, height_at, field_uv, coarse_uv, chart_origin, chart_offset, cleared, cleared_within, surface_uv, surface_bound}
#import naval_terrain::features::{feature_clearance}
#import naval_terrain::cells::cell_hash
#import naval_terrain::impostor::{LEAN_MOST, PALM_LEAN_MOST, PALM_FORM}
#import naval_terrain::surface::{cover_raw, SurfaceInputs, TreeCover, CoverShares, Ground, Parcels, tree_cover, ground_of, ramp, noise_r, parcels, cover_shares, near_cover_shares, mixed_shares, near_bound, near_share, near_uv, surface_inside, climate_snow, boreal_treeline, birch_belt, spruce_native, boreal_conifers, LUMA, TREE_PALM, TREE_CONIFER, TREE_BARE, CountryField, country_field, country_region, COUNTRY_WEST, COUNTRY_EAST, COUNTRY_BRITAIN, COUNTRY_EUROPE, COUNTRY_JAPAN, COUNTRY_TROPICS, COUNTRY_NORDIC, COUNTRY_MEDITERRANEAN, FIELD_ORCHARD, FIELD_PADDY, FIELD_GARDEN, FIELD_PALMS, FIELD_WOOD, FIELD_OLIVES, FIELD_ROUGH, FIELD_PASTURE, FIELD_YARD}

struct TreeView {
    // The view's frustum in chart metres (n · p + w ≥ 0 inside).
    planes: array<vec4<f32>, 6>,
    // The eye in chart metres; w: the lens's zoom (the square root of tan(fov / 2) over the game's 52° lens's).
    eye: vec4<f32>,
    // The full-density distance (apparent m), the range's end (m), the most a thinned tree grows, level 0's range (m).
    lod: vec4<f32>,
    // The sun's near, wide and aboard maps (world to the light's clip space) the trees cast into this frame.
    maps: array<mat4x4<f32>, 3>,
    // 1 where the trees cast into the near, the wide and the aboard map.
    casts: vec4<f32>,
    // The first of the shadow-only patches in `patches`, and the drawn patches carrying trees before them (the
    // workgroups past those take the shadow-only ones).
    shadow: vec4<f32>,
    // The biome (`species::Biome`; 0 a battle map's own trees), the ground cover's reach (m), its grid's first cell
    // (x, z, in cells from the chart's origin).
    biome: vec4<f32>,
    // The ground cover's grid: cells a side, a cell (m); the culling sphere of a tree about its foot: its middle's
    // height and its radius (m), from the tallest tree the land's species can grow.
    cover: vec4<f32>,
    // Where the chart's origin stands on the earth (`Climate::place`): latitude and longitude (degrees), 1 with a
    // place (a theatre's land; 0 a battle map's).
    place: vec4<f32>,
    // Per species (`species::Species`): the model's radius over its height, its height (m), its winter self (-1 for
    // an evergreen).
    species: array<vec4<f32>, 32>,
    // The canopy height map's pages (`naval_terrain::canopy`): page (0, 0)'s chart corner (x, z), a texel (m), the
    // slots a side (0 with none).
    canopy: vec4<f32>,
    // The page each slot holds, two a vector (slot k in `canopy_slots[k / 2]`, .xy for even k, .zw for odd).
    canopy_slots: array<vec4<i32>, 32>,
    // The biomes about the eye (`species::region_grid`): a byte a cell, four to a word, cell k = j * REGION_CELLS + i
    // in `regions[k / 16][(k / 4) % 4]` at bits 8 (k mod 4).
    regions: array<vec4<u32>, 16>,
    // The biomes' grid: its first cell's chart corner (x, z), a cell (m), 1 with a grid (a theatre's land).
    region_grid: vec4<f32>,
}

// A placed tree: foot (chart x, the model's base height, chart z) and height (m); the model's radius, species, its
// turn and form packed in one float (`packed_form`, `impostor::form_of`) and open sky; tint and width (m a model unit
// across, over the height).
struct Tree {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
}

// `DrawIndexedIndirectArgs`: the card's six indices, the trees counted here.
struct DrawArgs {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

@group(0) @binding(0) var<uniform> view: TreeView;
@group(0) @binding(1) var<storage, read> patches: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read_write> trees: array<Tree>;
@group(0) @binding(3) var<storage, read_write> args: DrawArgs;
// The canopy's pages, RGBA8 (`naval_terrain::canopy`): the tallest crown's height (m / 255), its place in the texel
// (x and z nibbles, sixteenths), the canopy's share of the texel, 1 where the map covers it.
@group(0) @binding(4) var canopy_pages: texture_2d<f32>;

// The species, in `species::Species::ALL`'s order (a Rust test holds these to it).
const SPECIES_RAINFOREST: f32 = 0.0;
const SPECIES_EMERGENT: f32 = 1.0;
const SPECIES_PALM: f32 = 2.0;
const SPECIES_MANGROVE: f32 = 3.0;
const SPECIES_OAK: f32 = 4.0;
const SPECIES_ASH: f32 = 5.0;
const SPECIES_BIRCH: f32 = 6.0;
const SPECIES_BARE_OAK: f32 = 7.0;
const SPECIES_BARE_BIRCH: f32 = 8.0;
const SPECIES_SPRUCE: f32 = 9.0;
const SPECIES_SCOTS_PINE: f32 = 10.0;
const SPECIES_STONE_PINE: f32 = 11.0;
const SPECIES_CYPRESS: f32 = 12.0;
const SPECIES_OLIVE: f32 = 13.0;
const SPECIES_LIVE_OAK: f32 = 14.0;
const SPECIES_MONTEREY_PINE: f32 = 15.0;
const SPECIES_MONTEREY_CYPRESS: f32 = 16.0;
const SPECIES_EUCALYPTUS: f32 = 17.0;
const SPECIES_REDWOOD: f32 = 18.0;
const SPECIES_ACACIA: f32 = 19.0;
const SPECIES_SHRUB: f32 = 20.0;
const SPECIES_GARRIGUE: f32 = 21.0;
const SPECIES_GRASS: f32 = 22.0;
const SPECIES_FAN_PALM: f32 = 23.0;
const SPECIES_DATE_PALM: f32 = 24.0;
const SPECIES_SYCAMORE: f32 = 25.0;
const SPECIES_CHAPARRAL: f32 = 26.0;
const SPECIES_SUGI: f32 = 27.0;
const SPECIES_LAUREL: f32 = 28.0;
const SPECIES_BLACK_PINE: f32 = 29.0;
const SPECIES_BAMBOO: f32 = 30.0;
const SPECIES_OASIS_PALM: f32 = 31.0;

// `species::Biome`.
const BIOME_AUTHORED: f32 = 0.0;
const BIOME_TROPICAL: f32 = 1.0;
const BIOME_ARID: f32 = 2.0;
const BIOME_MEDITERRANEAN: f32 = 3.0;
const BIOME_CALIFORNIA: f32 = 4.0;
const BIOME_SUBTROPICAL: f32 = 5.0;
const BIOME_TEMPERATE: f32 = 6.0;
const BIOME_BOREAL: f32 = 7.0;
const BIOME_SUBARCTIC: f32 = 8.0;
const BIOME_JAPAN: f32 = 9.0;
const BIOME_BRITAIN: f32 = 10.0;
const BIOME_NORTHERN_EUROPE: f32 = 11.0;
const BIOME_NORTH_AFRICA: f32 = 12.0;
const BIOME_EGYPT: f32 = 13.0;
const BIOME_NORTHEAST: f32 = 14.0;
const BIOME_SOUTHEAST: f32 = 15.0;
const BIOME_HAWAII: f32 = 16.0;
const BIOME_INTERIOR_WEST: f32 = 17.0;

// Cells a side of the biomes' grid (`species::REGION_CELLS`).
const REGION_CELLS: i32 = 16;

// Metres of the chart's z (which runs south) a degree of latitude spans.
const METRES_PER_DEGREE: f32 = 111195.0;

// The thinning's last share of a tree's own range, over which it shrinks away.
const SHRINK: f32 = 0.3;
// Texels on a canopy page's side (`canopy::PAGE_TEXELS`).
const CANOPY_PAGE_TEXELS: i32 = 256;
// A crown the canopy map measures this tall (m) stands a tree even where the land cover grows scrub or grass.
const CANOPY_TREE_M: f32 = 5.0;
// Where the canopy map covers a slot wider than this (m; a theatre's air window has slots of up to 70 m), it splits
// into parts, at most CANOPY_PARTS a side, each standing a tree on its own tallest crown: one crown standing for a
// closed canopy's every crown over 70 m would be three times as wide as it is tall.
const CANOPY_PART_M: f32 = 22.0;
const CANOPY_PARTS: i32 = 3;
// A measured tree's crown radius over the radius of a disc of its part's canopy: a crown is drawn about 0.8 of its
// model's widest reach across from above (1.25), and a closed canopy's crowns overlap, as they stand where they were
// measured and not on a grid (up to 0.4 more with the canopy's share).
const CANOPY_DRAWN: f32 = 1.25;
const CANOPY_OVERLAP: f32 = 0.4;
// The widest a measured tree stands over its species' width: a broad crown's (a reach of half its height) 1.8; a
// narrow one's (a spruce's, a palm's, 0.3 of its height or less) and a stand's (the sugi's, the bamboo's, which fill a
// slot as they are) 1.3, near its shape. And no crown reaches further than the tree is tall (CANOPY_ASPECT): a garden's
// 6 m hedge of 100% cover is no 30 m disc.
const CANOPY_WIDEST: vec2<f32> = vec2<f32>(1.3, 1.8);
const CANOPY_ASPECT: f32 = 1.0;
// A wood: where the land cover's chance of a tree is this or more (WorldCover's tree cover and mangroves, a biome's
// closed canopy or plantation), its own trees stand as they did without the canopy map, the map only raising a tree
// where its crown stands taller (Meta's map reads a tall tropical canopy low) and clearing a bare patch.
const CANOPY_WOOD: f32 = 0.5;
// A bare patch, where no tree stands: a square of at least CANOPY_BARE_M (m) about the slot with no crown in it and
// under CANOPY_BARE of it under canopy 2 m or taller (a clearing, a lawn, an airfield's paving and grass).
const CANOPY_BARE: f32 = 0.1;
const CANOPY_BARE_M: f32 = 32.0;
// Ground cover: every cell keeps its shrub or tuft within this share of the reach, (share / d)² of them beyond.
const COVER_FULL: f32 = 0.35;
// Tufts of grass stand only this near (m): further they are a pixel or two.
const GRASS_REACH_M: f32 = 140.0;
// A natural wood's dead trees and its younger ones (shares of its trees), the open land within this of a wood that
// makes its edge (m), and the share of the edge's mantle that stands as the wood's saplings.
const SNAGS: f32 = 0.015;
const YOUNG: f32 = 0.14;
const EDGE_M: f32 = 24.0;
const SAPLINGS: f32 = 0.3;
// The mantle's shrubs at a wood's edge, a share of the edge's cells.
const MANTLE: f32 = 0.45;

// Whether `cell` is the one cell of the 2 × 2 cells of size 2^`level` about it that its block's hash picks.
fn picked(cell: vec2<i32>, level: u32, salt: f32) -> bool {
    let block = cell >> vec2<u32>(level + 1u);
    let pick = min(i32(cell_hash(vec2<f32>(block), salt) * 4.0), 3);
    let sub = (cell >> vec2<u32>(level)) & vec2<i32>(1);
    return sub.x + sub.y * 2 == pick;
}

// The slot's rank in 0–1, stratified so any share of the lowest ranks spreads evenly over the land: one cell in each
// 2 × 2 block (picked by the block's hash) ranks below 1/4, one of those in each 4 × 4 block below 1/16, one in each
// 8 × 8 below 1/64; within its tier a cell's rank is its own hash. A thinned forest keeps one tree a block, not a
// lattice and not random clumps.
fn rank_of(cell: vec2<f32>) -> f32 {
    let c = vec2<i32>(cell);
    let h = cell_hash(cell, 9.0);
    if !picked(c, 0u, 11.0) {
        return (16.0 + 48.0 * h) / 64.0;
    }
    if !picked(c, 1u, 12.0) {
        return (4.0 + 12.0 * h) / 64.0;
    }
    if !picked(c, 2u, 13.0) {
        return (1.0 + 3.0 * h) / 64.0;
    }
    return h / 64.0;
}

fn in_view(centre: vec3<f32>, radius: f32) -> bool {
    for (var k = 0; k < 6; k += 1) {
        let plane = view.planes[k];
        if dot(plane.xyz, centre) + plane.w < -radius {
            return false;
        }
    }
    return true;
}

// Whether a tree standing at chart point `foot` on ground at `ground` lies in a map the trees cast into (its crown's
// middle, with a crown's room at the map's edges), so its shadow is drawn there whether or not it is in view.
fn in_maps(foot: vec2<f32>, ground: f32) -> bool {
    let offset = chart_offset();
    let middle = vec4<f32>(foot.x + offset.x, ground + 10.0, foot.y + offset.y, 1.0);
    for (var k = 0; k < 3; k += 1) {
        if view.casts[k] > 0.5 {
            let clip = view.maps[k] * middle;
            if all(abs(clip.xy) <= vec2<f32>(1.05)) && clip.z >= -0.05 && clip.z <= 1.05 {
                return true;
            }
        }
    }
    return false;
}

// The cell a level-`level` patch's slot `block` (a square of 2^level cells a side) stands its tree in: the one cell of
// the block that ranks below 4^-level, found down the picks `rank_of` makes.
fn survivor(block: vec2<i32>, level: u32) -> vec2<i32> {
    var cell = block;
    for (var l = i32(level) - 1; l >= 0; l -= 1) {
        let pick = min(i32(cell_hash(vec2<f32>(cell), 11.0 + f32(l)) * 4.0), 3);
        cell = cell * 2 + vec2<i32>(pick & 1, pick >> 1u);
    }
    return cell;
}

// The surface inputs at a chart point on the land.
fn inputs_at(foot: vec2<f32>, ground: f32, gradient: vec2<f32>) -> SurfaceInputs {
    let cuv = coarse_uv(foot);
    let ground_data = textureSampleLevel(attributes, clamp_sampler, cuv, 0.0);
    var inputs: SurfaceInputs;
    inputs.chart = foot;
    inputs.height = ground;
    inputs.gradient = gradient;
    inputs.footprint = 0.0;
    inputs.sky = textureSampleLevel(open_sky, clamp_sampler, cuv, 0.0).r;
    inputs.drainage = ground_data.r;
    inputs.broad = ground_data.g;
    inputs.coast = ground_data.b;
    inputs.fine = ground_data.a;
    return inputs;
}

// The land cover at the foot as `tree_cover` reads it, with the share of the point the maps know (0 with none).
struct Cover {
    shares: CoverShares,
    weight: f32,
}

fn cover_at(inputs: SurfaceInputs) -> Cover {
    var c: Cover;
    c.weight = 0.0;
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
        c.shares = s;
        c.weight = weight;
    }
    return c;
}

// A tree's species, its height (m, before its own jitter) and tint, and how it stands: how likely at all, how wide, how
// like its neighbours, and on a plantation's rows or not.
struct Choice {
    species: f32,
    height: f32,
    tint: vec3<f32>,
    // The chance a tree stands at the foot: the masks' (`tree_cover`), raised where the biome closes a canopy or
    // plants a plantation.
    density: f32,
    // Its width over its species' (before its own jitter).
    width: f32,
    // How far it differs from its neighbours in height and shade: 1 a wood's, less in a plantation of one age.
    variety: f32,
    // A plantation's rows: the angle (radians) of the grid its trees stand on, -1 for none.
    rows: f32,
    // How wild a wood it stands in, 0-1: a natural wood's dead and younger trees (`SNAGS`, `YOUNG`), none in a town,
    // a plantation or a field.
    wild: f32,
    // A lean of its own over the tree's own (its top's offset over its height, chart x and z): a wood's edge tree
    // reaching out to the light (`place`).
    lean: vec2<f32>,
}

fn species_height(species: f32) -> f32 {
    return view.species[u32(species)].y;
}

// Its winter self, by the season's leaves (`in_leaf` its own share of them).
fn seasonal(species: f32, in_leaf: bool) -> f32 {
    let winter = view.species[u32(species)].z;
    return select(species, winter, winter >= 0.0 && !in_leaf);
}

// The latitude (degrees) of a chart point: the chart's z runs south from its origin's place.
fn latitude_at(chart: vec2<f32>) -> f32 {
    return view.place.x - chart.y / METRES_PER_DEGREE;
}

// The longitude (degrees) of a chart point (`species::chart_place`): the chart's x runs east.
fn longitude_at(chart: vec2<f32>) -> f32 {
    return view.place.y + chart.x / (METRES_PER_DEGREE * max(cos(radians(view.place.x)), 0.05));
}

// The biome at a chart point (`species::region_grid`; the view's own on a battle map), the lookup moved by up to 0.4
// of a cell for each tree (`key` its cell), so a biome's edge on land is no line of squares.
fn biome_at(chart: vec2<f32>, key: vec2<f32>) -> f32 {
    if view.region_grid.w < 0.5 {
        return view.biome.x;
    }
    let jitter = (vec2<f32>(cell_hash(key, 21.0), cell_hash(key, 22.0)) - 0.5) * 0.8;
    let cell = clamp(vec2<i32>(floor((chart - view.region_grid.xy) / view.region_grid.z + jitter)), vec2<i32>(0),
        vec2<i32>(REGION_CELLS - 1));
    let k = u32(cell.y * REGION_CELLS + cell.x);
    return f32((view.regions[k / 16u][(k / 4u) % 4u] >> (8u * (k % 4u))) & 255u);
}

// The photograph's sun, toward it (chart x east, z south): late morning (Sentinel-2 passes at 10:30), from the south-east
// some 45° up, as it stands north of the tropics, where the biomes that read the photograph's woods lie.
const PHOTO_SUN: vec3<f32> = vec3<f32>(0.455, 0.707, 0.542);

// How bright the photograph shows the land about a point against the window's woods (their mean colour, the haze out:
// `terrain.cover_means[1]`), its slope's own light under the photograph's sun taken out: 1 as bright, less darker. A
// spruce or pine plantation, a fir or hemlock wood, reads at a half to two-thirds of a broadleaf wood in a summer
// mosaic. 1 with no photograph, or no woods in the window.
fn photo_brightness(inputs: SurfaceInputs) -> f32 {
    let mean = terrain.cover_means[1];
    let mean_luminance = dot(mean.rgb, LUMA);
    if !surface_bound() || mean.w < 0.5 || mean_luminance <= 1e-4 {
        return 1.0;
    }
    var sum = vec4<f32>(0.0);
    for (var k = 0; k < 4; k += 1) {
        let at = inputs.chart + (vec2<f32>(f32(k & 1), f32(k >> 1u)) * 2.0 - 1.0) * 15.0;
        let uv = surface_uv(at);
        var sample = textureSampleLevel(surface_albedo, clamp_sampler, uv, 0, 0.0) * surface_inside(uv);
        if near_bound() {
            sample = mix(sample, textureSampleLevel(surface_albedo, repeat_sampler, near_uv(at), 1, 0.0), near_share(at));
        }
        sum += sample;
    }
    if sum.a < 0.5 {
        return 1.0;
    }
    let rgb = max(sum.rgb / sum.a - terrain.surface_haze.rgb, vec3<f32>(0.0));
    let normal = normalize(vec3<f32>(-inputs.gradient.x, 1.0, -inputs.gradient.y));
    let lit = (0.35 + 0.65 * max(dot(normal, PHOTO_SUN), 0.0)) / (0.35 + 0.65 * PHOTO_SUN.y);
    return dot(rgb, LUMA) / (mean_luminance * max(lit, 0.3));
}

// A battle map's tree: its style's kind as one species, as the old cards drew it (a quarter of the broadleaf in the
// old atlas's broad, flat-topped or tall, narrow variants: the emergent, the ash).
fn authored(cover: TreeCover, pick: f32) -> f32 {
    let variant = fract(pick * 3.71) < 0.25;
#ifdef STYLE_SNOW
    return select(select(SPECIES_BIRCH, SPECIES_BARE_BIRCH, cover.kind == TREE_BARE), SPECIES_SPRUCE, cover.kind == TREE_CONIFER);
#else
#ifdef STYLE_CHALK
    let leafy = select(SPECIES_OAK, SPECIES_ASH, variant);
    return select(select(leafy, SPECIES_BARE_OAK, cover.kind == TREE_BARE), SPECIES_SCOTS_PINE, cover.kind == TREE_CONIFER);
#else
    return select(select(SPECIES_RAINFOREST, SPECIES_EMERGENT, variant), SPECIES_PALM, cover.kind == TREE_PALM);
#endif
#endif
}

// A theatre's tree by its biome: the land cover at the foot (shrubland grows shrubs, mangroves mangroves, fields and
// towns their own trees), moist hollows and canyons, exposed headlands by the sea, the height, the latitude, stands
// of one species a few hundred metres across and plantations' compartments.
fn choose(biome: f32, inputs: SurfaceInputs, g: Ground, cover: TreeCover, c: Cover, pick: f32) -> Choice {
    let s = c.shares;
    let w = c.weight;
    let height = inputs.height;
    let u = fract(pick * 5.37);
    let stand = noise_r(inputs, 650.0);
    let grove = noise_r(inputs, 1700.0);
    let moist = max(max(g.gully, g.hollow), ramp(6.5, 9.0, g.wetness));
    let headland = (1.0 - ramp(120.0, 500.0, g.coast)) * (0.4 + 0.6 * max(g.knoll, ramp(0.15, 0.4, g.slope)));
    let woods = s.a.x * w;
    let shrubland = s.a.z * w;
    let mangroves = s.a.y * w;
    let fields = s.b.x * w;
    let built = s.b.y * w;
    let open_land = (s.a.w + s.b.x + s.b.y) * w;
    // A town's or suburb's trees: the gardens' and streets' (WorldCover's tree cover over a leafy suburb is town too,
    // by the built share about it).
    let town = fract(pick * 3.17) < ramp(0.15, 0.6, built);
    let leaf = terrain.climate.z;
    let in_leaf = fract(pick * 7.13) < leaf;
    var choice: Choice;
    choice.density = cover.density;
    choice.width = 1.0;
    choice.variety = 1.0;
    choice.rows = -1.0;
    choice.tint = vec3<f32>(1.0);
    var species = SPECIES_OAK;
    var scale = 1.0;
    // A plantation's clear-felled compartments and the rides between them: none stands, whatever the land cover's
    // density (applied after its floor below).
    var felled = 1.0;
    if biome == BIOME_TROPICAL {
        // Coconut plantations on the coastal plains: estates of palms in rows on a grid of their own, over the woods
        // and fields of the flat land behind the shore (Guadalcanal's north coast was planted from Lunga to Tetere).
        // The Philippines plant coconuts up the hills too (Leyte's and Samar's slopes are palms to 400 m), far inland.
        let lat = latitude_at(inputs.chart);
        let lon = longitude_at(inputs.chart);
        let philippines = lat > 4.5 && lat < 19.0 && lon > 116.5 && lon < 127.0;
        var plain = (1.0 - ramp(1500.0, 4000.0, g.coast)) * (1.0 - ramp(20.0, 40.0, height))
            * (1.0 - ramp(0.06, 0.12, g.slope));
        if philippines {
            plain = max(plain, (1.0 - ramp(300.0, 450.0, height)) * (1.0 - ramp(0.3, 0.45, g.slope)) * 0.9);
        }
        let estates = parcels(inputs, 700.0, 14.0);
        let estate = plain * (woods + fields + 0.5 * s.a.w * w) * (1.0 - estates.boundary)
            * select(0.0, 1.0, fract(estates.id * 3.7) < 0.6);
        // The strand: a low islet or the flat behind a beach, an atoll's every islet (Tarawa's, Kwajalein's): coconut
        // palms over breadfruit, pandanus and the beach's scaevola and heliotrope; no rainforest.
        let strand = (1.0 - ramp(200.0, 450.0, g.coast)) * (1.0 - ramp(5.0, 9.0, height));
        if mangroves > 0.4 || (height < 4.0 && g.coast < 250.0 && s.c.y * w > 0.3) {
            species = SPECIES_MANGROVE;
        } else if strand > 0.5 && !town {
            choice.density = max(choice.density, (woods + 0.6 * s.a.w * w + 0.6 * shrubland) * 0.9);
            if u < 0.62 {
                // An atoll's wooded islet is a coconut grove, the crowns nearly meeting (1.25 under the 9 m crowns
                // before 2026-10-26's palm).
                species = SPECIES_PALM;
                choice.width = 1.1;
            } else if u < 0.78 {
                // Breadfruit.
                species = SPECIES_LAUREL;
                scale = 0.8;
            } else {
                // Pandanus, scaevola and beach heliotrope.
                species = SPECIES_SHRUB;
                scale = 1.6;
                choice.width = 1.3;
            }
        } else if estate > 0.5 {
            species = SPECIES_PALM;
            choice.density = max(choice.density, 0.97);
            choice.rows = estates.id * 1.5707963;
            choice.variety = 0.4;
            // One planting an estate: its palms of an age.
            scale = 0.8 + 0.35 * fract(estates.id * 11.3);
            // The Philippines' smallholdings, planted closer than a slot: crowns that nearly meet (1.3 under the 9 m
            // crowns before 2026-10-26's palm).
            choice.width = select(1.0, 1.15, philippines);
        } else if cover.kind == TREE_PALM || (open_land > 0.5 && u < 0.6) {
            // Coconut palms behind the beaches and about the villages, gardens and grassland.
            species = SPECIES_PALM;
        } else if shrubland > 0.5 {
            species = SPECIES_SHRUB;
        } else {
            // A closed lowland forest: a crown in every slot, uneven (patches of taller and lower canopy, a treefall's
            // regrowth), emergents standing out of it.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.99);
            let closed = select(woods + (1.0 - w), 1.0, w <= 0.0);
            species = select(SPECIES_RAINFOREST, SPECIES_EMERGENT, u < 0.12 && height < 700.0 && closed > 0.6);
            choice.width = select(1.0, 1.35, species == SPECIES_RAINFOREST);
            // Lower near the sea, in the salt wind (a small island's forest is all coastal).
            scale = (1.0 - 0.35 * ramp(900.0, 1800.0, height)) * (0.8 + 0.4 * noise_r(inputs, 170.0))
                * mix(0.7, 1.0, ramp(300.0, 3000.0, g.coast));
        }
    } else if biome == BIOME_ARID {
        // Date palms by the sea, in the towns and the oases (the Sonoran desert's fan palms), acacia, scrub.
        let palm = select(SPECIES_OASIS_PALM, SPECIES_FAN_PALM, longitude_at(inputs.chart) < -30.0);
        species = select(select(SPECIES_ACACIA, palm, (g.coast < 2500.0 && u < 0.5) || ((fields + built) > 0.5 && u < 0.7)), SPECIES_SHRUB,
            shrubland > 0.5);
    } else if biome == BIOME_MEDITERRANEAN || biome == BIOME_NORTH_AFRICA || biome == BIOME_EGYPT {
        let lat = latitude_at(inputs.chart);
        let lon = longitude_at(inputs.chart);
        let africa = biome != BIOME_MEDITERRANEAN;
        let egypt = biome == BIOME_EGYPT;
        let v = fract(pick * 9.71);
        // The Sahara's side of North Africa (Gabès 33.9 N on the sea, Tripoli 32.9, Benghazi 32.1, every oasis): date
        // palms where anything is grown.
        let sahara = select(select(0.0, ramp(34.6, 32.2, lat), africa), 1.0, egypt);
        // The groves: a parcel of trees of one age in rows on the farmland below the mountains: olives most, citrus on
        // the irrigated plains (Valencia's huerta, the Catania plain, Jaffa's, the Mitidja: low and flat), date palms
        // toward the desert.
        let parcel = parcels(inputs, 260.0, 9.0);
        // (A wood on the flat counts: the land cover takes an old grove for one. Grass and scrub alone do not: a
        // dehesa's pasture under scattered oaks is no grove.)
        let farmland = (fields + 0.3 * s.a.w * w + 0.3 * shrubland + 0.6 * woods * (1.0 - ramp(0.04, 0.1, g.slope)))
            * (1.0 - parcel.boundary) * (1.0 - ramp(500.0, 800.0, height)) * (1.0 - ramp(0.2, 0.35, g.slope));
        let irrigated = (1.0 - ramp(120.0, 220.0, height)) * (1.0 - ramp(0.03, 0.07, g.slope));
        // The shares of the farmland's parcels in groves. Egypt's are few among its open fields: citrus and mango, a
        // date grove.
        var olives = select(0.15 + 0.4 * noise_r(inputs, 5000.0), 0.45, africa) * (1.0 - sahara);
        var citrus = select(0.18, 0.2, africa) * irrigated * (1.0 - sahara);
        var dates = select(0.0, 0.45 * sahara, africa);
        if egypt {
            olives = 0.0;
            citrus = 0.08;
            dates = 0.1;
        }
        let id = fract(parcel.id * 5.13);
        let in_grove = farmland > 0.5 && id < olives + citrus + dates;
        // Eucalyptus planted in Portugal and the Algarve, Sardinia and Sicily (not Malta), the Levant and the Maghreb.
        let eucalyptus = select(select(0.0, 1.0, lon < -6.6 || (lon > 8.0 && lon < 15.6 && lat > 36.5 && lat < 41.3)
            || lon > 34.0), 1.0, africa);
        if mangroves > 0.4 {
            species = SPECIES_MANGROVE;
        } else if in_grove {
            choice.density = max(choice.density, 0.92);
            choice.rows = parcel.id * 1.5707963;
            choice.variety = 0.4;
            let age = 0.75 + 0.45 * fract(parcel.id * 11.3);
            if id < dates {
                species = SPECIES_OASIS_PALM;
                scale = age;
            } else if id < dates + citrus {
                // Orange and lemon trees, dark and dense, or Egypt's mango.
                species = SPECIES_LAUREL;
                scale = 0.32 * age;
                choice.width = 1.25;
                choice.tint = vec3<f32>(0.78, 0.9, 0.72);
            } else {
                species = SPECIES_OLIVE;
                scale = age;
                choice.width = 1.3;
                choice.tint = vec3<f32>(1.04, 1.05, 1.0);
            }
        } else if shrubland > 0.5 {
            // Maquis and garrigue: holm oak and kermes oak, lentisk, heather, broom; the desert's tamarisk.
            species = select(SPECIES_SHRUB, SPECIES_GARRIGUE, u < 0.35);
            scale = select(1.0, 0.7, sahara > 0.5);
        } else if town {
            // Stone pines, cypresses and palms, the planes of the squares, ficus and pepper trees, orange trees in the
            // courtyards; Africa's date palms, ficus and eucalyptus, Egypt's casuarinas and flame trees.
            if africa {
                if u < 0.28 {
                    species = SPECIES_OASIS_PALM;
                } else if u < 0.36 {
                    species = select(SPECIES_DATE_PALM, SPECIES_FAN_PALM, v < 0.5);
                } else if u < 0.62 {
                    species = SPECIES_LAUREL;
                    scale = 0.85;
                } else if u < 0.72 {
                    species = SPECIES_EUCALYPTUS;
                    scale = 0.7;
                } else if u < 0.8 {
                    species = select(SPECIES_STONE_PINE, SPECIES_SCOTS_PINE, egypt);
                    choice.tint = select(vec3<f32>(1.0), vec3<f32>(0.85, 0.92, 0.88), egypt);
                } else if u < 0.88 {
                    species = SPECIES_ACACIA;
                    scale = 1.3;
                    choice.width = 1.3;
                } else {
                    species = SPECIES_OLIVE;
                }
            } else {
                if u < 0.22 {
                    species = SPECIES_STONE_PINE;
                } else if u < 0.36 {
                    species = SPECIES_CYPRESS;
                } else if u < 0.48 {
                    species = seasonal(SPECIES_SYCAMORE, in_leaf);
                } else if u < 0.56 {
                    species = select(SPECIES_DATE_PALM, SPECIES_FAN_PALM, v < 0.4);
                } else if u < 0.7 {
                    species = SPECIES_LAUREL;
                    scale = select(0.75, 0.35, v < 0.4);
                } else if u < 0.78 {
                    species = SPECIES_EUCALYPTUS;
                    scale = 0.7;
                } else {
                    species = SPECIES_OLIVE;
                }
            }
        } else if egypt || sahara > 0.5 {
            // The Nile's and the oases' land outside the groves: date palms along the canals and about the villages,
            // casuarina and eucalyptus windbreaks, sycamore figs.
            if u < 0.55 {
                species = SPECIES_OASIS_PALM;
            } else if u < 0.8 {
                species = select(SPECIES_EUCALYPTUS, SPECIES_SCOTS_PINE, v < 0.5);
                choice.tint = select(vec3<f32>(1.0), vec3<f32>(0.85, 0.92, 0.88), v < 0.5);
            } else {
                species = SPECIES_LAUREL;
            }
        } else if moist > 0.65 && height < 700.0 {
            // The streams' oriental planes (Italy and east), alders and ashes (Iberia's and the Maghreb's), poplars and
            // oleanders.
            let gallery = select(SPECIES_SYCAMORE, SPECIES_ASH, lon < 7.0);
            species = select(seasonal(gallery, in_leaf), select(SPECIES_BARE_BIRCH, SPECIES_CYPRESS, in_leaf), u < 0.25);
            choice.tint = select(vec3<f32>(1.0), vec3<f32>(1.15, 1.2, 1.0), species == SPECIES_CYPRESS);
            scale = select(1.0, 1.2, species == SPECIES_CYPRESS);
        } else if fields > 0.5 {
            // The fields' edges: olives, cypress windbreaks, eucalyptus lines, carobs.
            species = select(select(SPECIES_OLIVE, SPECIES_CYPRESS, u < 0.3), SPECIES_EUCALYPTUS, u > 1.0 - 0.15 * eucalyptus);
        } else if height > 1400.0 {
            // The high mountains: black pine and fir, the Atlas' cedars.
            species = select(SPECIES_BLACK_PINE, SPECIES_SPRUCE, u < 0.3);
            scale = select(1.3, 1.0, species == SPECIES_SPRUCE);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.9);
        } else if height > 800.0 {
            // The mountain woods: black pine, deciduous and holm oak, chestnut.
            species = select(seasonal(SPECIES_OAK, in_leaf), SPECIES_BLACK_PINE, stand > 0.5);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.9);
        } else if g.coast < 1500.0 && stand > 0.55 {
            // Stone pines by the sea.
            species = SPECIES_STONE_PINE;
        } else if grove > 0.7 && height < 400.0 && v < eucalyptus {
            species = SPECIES_EUCALYPTUS;
        } else if stand > 0.55 && lon > -4.5 {
            // Aleppo pine, the limestone hills' pine: paler, open, crooked (not on western Andalusia's, Portugal's and
            // Tangier's sandstone hills, the cork oak's).
            species = SPECIES_SCOTS_PINE;
            scale = 0.7;
            choice.tint = vec3<f32>(0.98, 1.02, 0.9);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.85);
        } else {
            // Holm and cork oak, wild olive and carob, a cypress.
            species = select(select(SPECIES_LIVE_OAK, SPECIES_OLIVE, u < 0.3), SPECIES_CYPRESS, u < 0.06);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.9);
        }
    } else if biome == BIOME_CALIFORNIA {
        let lat = latitude_at(inputs.chart);
        // Redwoods from Big Sur north: none south of about 35.8 N (their southernmost groves), thinning out over the
        // last 40 km.
        let redwoods = ramp(35.6, 36.0, lat);
        // Southern California, Point Conception and south: palms in the towns, sycamores in the canyons.
        let south = 1.0 - ramp(34.6, 35.4, lat);
        let v = fract(pick * 9.71);
        if shrubland > 0.5 {
            // Chaparral: a closed cover of evergreen shrubs, a thicket a slot (narrower on steep slopes, where a wide
            // one would stand off the ground).
            species = SPECIES_CHAPARRAL;
            // Wider than a slot on the flat, so their edges meet; on a slope narrower and a little taller, so a
            // thicket neither stands off the ground downhill nor sinks into it uphill.
            choice.density = max(choice.density, shrubland * 0.92);
            scale = 1.0 + 1.5 * g.slope;
            choice.width = 1.3 / ((1.0 + 1.2 * g.slope) * scale);
            choice.variety = 0.7;
        } else if town {
            // Towns and gardens: Mexican fan and Canary Island date palms down the streets of the south, camphor,
            // ficus and magnolia (the laurel's dense dome), blue gums, sycamores and oaks.
            let palms = mix(0.12, 0.5, south);
            if u < palms * 0.78 {
                species = SPECIES_FAN_PALM;
            } else if u < palms {
                species = SPECIES_DATE_PALM;
            } else if u < palms + 0.25 {
                species = SPECIES_LAUREL;
                scale = 0.8;
            } else if u < palms + 0.37 {
                species = SPECIES_EUCALYPTUS;
            } else if u < palms + 0.47 {
                species = select(SPECIES_MONTEREY_PINE, seasonal(SPECIES_SYCAMORE, in_leaf), v < south);
            } else {
                species = SPECIES_LIVE_OAK;
            }
        } else if moist > 0.6 && height > 30.0 && height < 700.0 && g.slope > 0.1 && stand > 0.4 && g.coast > 400.0
            && g.coast < 25000.0 && v < redwoods {
            // Redwoods in the moist canyons of the coast ranges.
            species = SPECIES_REDWOOD;
        } else if moist > 0.5 && height < 1200.0 && v < south {
            // The south's canyon bottoms and creeks: sycamores and coast live oaks.
            species = select(SPECIES_LIVE_OAK, seasonal(SPECIES_SYCAMORE, in_leaf), u < 0.5);
        } else if s.a.w * w > 0.5 && v < south && u > 0.15 {
            // The south's grassy hills: coastal sage scrub, lower, greyer and more open than chaparral, among a few
            // oaks.
            species = SPECIES_CHAPARRAL;
            choice.density = max(choice.density, s.a.w * w * 0.55);
            scale = 1.0 + 1.5 * g.slope;
            choice.width = 1.1 / ((1.0 + 1.2 * g.slope) * scale);
            scale *= 0.75;
            choice.tint = vec3<f32>(1.12, 1.1, 1.02);
            choice.variety = 0.8;
        } else if headland > 0.45 {
            species = select(SPECIES_MONTEREY_PINE, SPECIES_MONTEREY_CYPRESS, u < 0.55);
        } else if grove > 0.66 + 0.12 * (1.0 - (s.b.x + s.b.y) * w) && height < 350.0 {
            // Blue gum groves, planted about the farms and towns.
            species = SPECIES_EUCALYPTUS;
        } else if g.coast < 4000.0 && stand > 0.62 + 0.2 * south {
            species = SPECIES_MONTEREY_PINE;
        } else {
            species = SPECIES_LIVE_OAK;
        }
    } else if biome == BIOME_JAPAN {
        let lat = latitude_at(inputs.chart);
        // The evergreen broadleaf (laurel) forest's upper edge: some 1,200 m in Kyushu and Shikoku, 700 m about Tokyo,
        // the sea by 38.5 N; south of 31 N higher, some 2,400 m in Taiwan's mountains at 24 N (its Machilus-Castanopsis
        // and evergreen oak forests); beech and oak above, fir and spruce higher.
        let laurel_top = clamp(min((38.5 - lat) * 220.0, 1500.0 + max(31.0 - lat, 0.0) * 130.0), 0.0, 2700.0);
        let warm = 1.0 - ramp(laurel_top - 150.0, laurel_top + 150.0, height);
        let subalpine = ramp(laurel_top + 900.0, laurel_top + 1200.0, height) + ramp(42.0, 43.5, lat) * 0.4;
        // Taiwan and the Ryukyus (south of about 26 N): evergreen to the treeline, their oaks the evergreen ring-cupped
        // oaks, their mountains above the laurel forest Taiwan hemlock, spruce and fir with the red and yellow cypress
        // (Chamaecyparis) below them, rhododendron and dwarf bamboo the broadleaves; no crown bare in any month.
        let formosa = 1.0 - ramp(25.5, 27.0, lat);
        // Plantations of sugi and hinoki, a compartment of one species and age, on the slopes of Kyushu, Shikoku and
        // Honshu (four in ten of their woods; not Hokkaido's or the Ryukyus'), with rides between them.
        let compartment = parcels(inputs, 300.0, 8.0);
        let planted = ramp(30.6, 31.4, lat) * (1.0 - ramp(41.0, 41.8, lat)) * (1.0 - compartment.boundary)
            * select(0.0, 1.0, compartment.id < 0.45) * ramp(30.0, 100.0, height) * (1.0 - ramp(1300.0, 1600.0, height))
            * ramp(0.05, 0.12, g.slope);
        let woodland = select(woods, 1.0, w <= 0.0);
        let villages = 1.0 - ramp(150.0, 300.0, height);
        if shrubland > 0.5 {
            // Dwarf bamboo (sasa) and scrub.
            species = SPECIES_SHRUB;
        } else if town {
            // Towns: camphor and the evergreens, zelkova, cherry and ginkgo (bare in winter), the gardens' pines,
            // bamboo.
            if u < 0.4 {
                species = SPECIES_LAUREL;
                scale = 0.8;
            } else if u < 0.72 {
                // Taiwan's banyans and camphors in their place.
                species = select(seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.56), in_leaf), SPECIES_LAUREL, formosa > 0.5);
                scale = 0.85;
            } else if u < 0.92 {
                species = SPECIES_BLACK_PINE;
                scale = 0.7;
            } else {
                species = SPECIES_BAMBOO;
            }
        } else if (g.coast < 350.0 && height < 40.0) || headland > 0.75 {
            // The shore's black pine belts, and the pines of the rocky headlands.
            species = SPECIES_BLACK_PINE;
            choice.density = max(choice.density, woodland * 0.9);
            choice.width = 1.2;
        } else if fields > 0.5 {
            // Among the paddies: farmsteads' bamboo and evergreen windbreaks.
            species = select(select(SPECIES_LAUREL, SPECIES_BLACK_PINE, u < 0.3), SPECIES_BAMBOO, u < 0.25 * villages + 0.05);
        } else if planted > 0.5 && woodland > 0.5 {
            // A plantation: sugi in most compartments, hinoki (a little lower and bluer) in a third, every slot
            // planted, its trees of one height and shade.
            species = SPECIES_SUGI;
            choice.density = max(choice.density, 0.97);
            choice.variety = 0.25;
            scale = 0.72 + 0.4 * fract(compartment.id * 7.31);
            if compartment.id < 0.15 && moist < 0.7 {
                scale *= 0.85;
                choice.tint = vec3<f32>(0.8, 0.9, 0.98);
                choice.width = 1.1;
            }
        } else if subalpine > 0.5 {
            // Subalpine fir and spruce; Hokkaido's Todo fir and Ezo spruce among its broadleaves; Taiwan's fir (narrow)
            // over rhododendron.
            let broadleaf = select(seasonal(select(SPECIES_OAK, SPECIES_BIRCH, u < 0.3), in_leaf), SPECIES_LAUREL, formosa > 0.5);
            species = select(broadleaf, SPECIES_SPRUCE, u < 0.55 + 0.35 * subalpine);
            if formosa > 0.5 {
                scale = select(0.45, 0.9, species == SPECIES_SPRUCE);
                choice.width = select(1.0, 0.8, species == SPECIES_SPRUCE);
            }
        } else if warm > 0.5 {
            // The laurel forest (Castanopsis, Machilus, camphor), closed; red and black pine on the dry ridges, konara
            // oak coppice, bamboo groves about the villages.
            // Closed where the cover is mostly woods (WorldCover's mixed pixels at a wood's edges and clearings read as
            // part grass), its crowns a little wider than a slot, so they meet in one lumpy canopy.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.98);
            // Taiwan's cypress forests among the evergreen oaks, some 1,800 m to the laurel forest's top: the giant
            // red cypress, its thick red trunk (the redwood's model, lower).
            let cypress = formosa * ramp(1700.0, 1900.0, height) * 0.3;
            if villages > 0.5 && noise_r(inputs, 280.0) > 0.74 {
                species = SPECIES_BAMBOO;
            } else if (g.knoll > 0.5 && u < 0.3) || u < 0.06 {
                species = SPECIES_BLACK_PINE;
                choice.width = 1.3;
            } else if fract(pick * 2.39) < cypress {
                species = SPECIES_REDWOOD;
                scale = 0.6;
            } else if u < 0.2 {
                // Konara oak coppice; Taiwan's and the Ryukyus' evergreen oaks.
                species = select(seasonal(SPECIES_OAK, in_leaf), SPECIES_LIVE_OAK, formosa > 0.5);
                choice.width = 1.3;
            } else {
                species = SPECIES_LAUREL;
                choice.width = 1.3;
            }
        } else if formosa > 0.5 {
            // Taiwan's mountains above the laurel forest: hemlock and spruce, the cypress below them, closed; evergreen
            // oaks and rhododendron among them.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.95);
            if u < 0.72 {
                species = SPECIES_SPRUCE;
                choice.tint = vec3<f32>(0.85, 0.95, 0.9);
            } else if u < 0.84 {
                species = SPECIES_REDWOOD;
                scale = 0.55;
            } else {
                species = SPECIES_LAUREL;
                scale = 0.6;
            }
        } else {
            // The cool-temperate forest: beech and oak (bare in winter), birch, a few fir.
            species = select(seasonal(select(SPECIES_OAK, SPECIES_BIRCH, u < 0.25), in_leaf), SPECIES_SPRUCE, u < 0.12);
        }
    } else if biome == BIOME_BRITAIN || biome == BIOME_NORTHERN_EUROPE {
        let lat = latitude_at(inputs.chart);
        let lon = longitude_at(inputs.chart);
        let britain = biome == BIOME_BRITAIN;
        // Britain's windswept north and west, Orkney and Shetland, Caithness's coast, the Outer Hebrides, Tiree and
        // Coll: hardly a tree but the farms' and the towns' shelter, low and wind-cut.
        let isles = select(0.0, max(ramp(58.45, 58.7, lat), select(0.0, 1.0, lon < -6.25 && lat > 56.4)), britain);
        // The Highlands' glens and hills above the farms.
        let highland = select(0.0, ramp(56.0, 56.6, lat), britain);
        let upland = max(ramp(250.0, 400.0, height), highland * ramp(60.0, 200.0, height));
        // The continent's mountains: the Alps, the Pyrenees, the Carpathians, the Dinaric ranges.
        let mountain = select(ramp(800.0, 1100.0, height), 0.0, britain);
        // Conifer woods where the photograph shows a wood dark: spruce and pine plantations, the Highlands' native
        // pinewoods. Not south of the Alps' latitude below the mountains, where a dark wood is oak, beech, hornbeam and
        // chestnut (Thrace's Istranca and Belgrad forests, the Balkans' hills).
        var dark = 0.0;
        let conifer_belt = select(ramp(44.5, 46.5, lat), 1.0, britain);
        if woods > 0.3 && conifer_belt > 0.0 {
            dark = (1.0 - ramp(0.6, 0.8, photo_brightness(inputs))) * conifer_belt;
        }
        // The sandy plain from the Netherlands to the Vistula, where the pine is the wood's tree.
        let sandy = select(ramp(51.3, 52.0, lat) * ramp(6.0, 7.5, lon) * (1.0 - ramp(150.0, 250.0, height)), 0.0, britain);
        // The Landes' maritime pine forest, the largest planted forest in Europe; Galicia's and northern Portugal's
        // eucalyptus and pine.
        let landes = !britain && lat > 43.6 && lat < 45.3 && lon > -1.45 && lon < -0.2 && height < 200.0;
        let galicia = !britain && lat < 43.9 && lon < -6.6 && height < 600.0;
        // The polders and the Po's plain: poplars in rows, willows along the ditches.
        let lowland_plain = !britain && height < 120.0
            && ((lat > 51.6 && lat < 53.6 && lon > 3.5 && lon < 7.2) || (lat > 44.6 && lat < 45.9 && lon > 7.6 && lon < 12.6));
        let compartment = parcels(inputs, 350.0, 10.0);
        if shrubland > 0.5 {
            // Gorse and broom on the commons, the heaths' scrub, the mountains' dwarf pine.
            species = SPECIES_SHRUB;
            choice.tint = vec3<f32>(0.82, 0.92, 0.72);
        } else if isles > 0.5 {
            species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.4), in_leaf);
            scale = 0.5;
        } else if town {
            // Limes, planes, sycamores and horse chestnuts down the streets and in the parks, birches, the gardens'
            // Leyland cypress hedges and pines.
            if u < 0.16 {
                species = seasonal(SPECIES_SYCAMORE, in_leaf);
            } else if u < 0.3 {
                species = seasonal(SPECIES_BIRCH, in_leaf);
                scale = 0.8;
            } else if u < 0.38 {
                species = SPECIES_CYPRESS;
                scale = 0.6;
                choice.tint = vec3<f32>(0.78, 0.88, 0.8);
            } else if u < 0.44 {
                species = SPECIES_SCOTS_PINE;
                scale = 0.8;
            } else {
                species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.7), in_leaf);
                scale = 0.85;
            }
        } else if lowland_plain && woods < 0.5 && u < 0.45 {
            // A row of poplars, tall and narrow (bare in winter), or pollard willows, small and grey.
            if u < 0.3 {
                species = select(SPECIES_BARE_BIRCH, SPECIES_CYPRESS, in_leaf);
                scale = 1.35;
                choice.tint = vec3<f32>(1.15, 1.2, 1.0);
            } else {
                species = seasonal(SPECIES_OAK, in_leaf);
                scale = 0.45;
                choice.tint = vec3<f32>(1.1, 1.1, 1.05);
            }
        } else if fields > 0.5 || (s.a.w * w > 0.5 && woods < 0.3) {
            // Hedgerow and field trees: oak above all, ash.
            species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.3), in_leaf);
        } else if landes || (galicia && woods > 0.3) {
            // Plantations in compartments of one age, some clear-felled and replanted: maritime pine, Galicia's blue
            // gums.
            let age = fract(compartment.id * 7.31);
            species = select(SPECIES_SCOTS_PINE, SPECIES_EUCALYPTUS, galicia && fract(compartment.id * 3.3) < 0.55);
            choice.density = max(choice.density, 0.95);
            felled = select(1.0, 0.0, age < 0.1) * (1.0 - compartment.boundary);
            choice.variety = 0.3;
            scale = 0.35 + 0.75 * age;
        } else if (dark > 0.5 && woods > 0.3) || mountain > 0.5 {
            // Conifer woods: Sitka and Norway spruce in compartments of one age, Scots and Corsican pine on the heaths
            // and the sandy plain, the Highlands' native pinewoods; in the mountains spruce and fir.
            let pine = select(0.3 + 0.6 * sandy, 0.25 + 0.5 * highland, britain);
            species = select(SPECIES_SPRUCE, SPECIES_SCOTS_PINE, fract(compartment.id * 5.17) < pine);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.97);
            if mountain < 0.5 {
                choice.variety = 0.35;
                scale = 0.5 + 0.65 * fract(compartment.id * 7.31);
                choice.width = 1.15;
            } else {
                species = select(SPECIES_SPRUCE, seasonal(SPECIES_BIRCH, in_leaf), u < 0.08);
                scale = 1.0 - 0.4 * ramp(1500.0, 2000.0, height);
            }
        } else if g.coast < 1000.0 && height < 30.0 && stand > 0.5 {
            // The dunes' pines: Corsican and Austrian pine on the Dutch, Belgian and Danish dunes, Scots pine on
            // Britain's.
            species = select(SPECIES_BLACK_PINE, SPECIES_SCOTS_PINE, britain);
        } else if upland > 0.5 && britain {
            // Britain's upland woods: birch, rowan and oak, Scots pine (the continent's hills keep the lowland's oak and
            // beech below the mountains).
            species = select(seasonal(SPECIES_BIRCH, in_leaf), SPECIES_SCOTS_PINE, u < 0.15 + 0.35 * highland);
            scale = 0.8;
        } else {
            // Lowland woods: oak and ash, beech (brighter, a smooth dome) in stands, birch on the heaths, pines on the
            // sand.
            species = seasonal(select(select(SPECIES_OAK, SPECIES_ASH, u < 0.35), SPECIES_BIRCH, u < 0.12 + 0.2 * sandy), in_leaf);
            species = select(species, SPECIES_SCOTS_PINE, u > 1.0 - 0.3 * sandy);
            if stand > 0.58 && species == SPECIES_OAK {
                choice.tint = vec3<f32>(1.02, 1.06, 0.88);
                choice.width = 1.15;
            }
        }
    } else if biome == BIOME_NORTHEAST {
        let lat = latitude_at(inputs.chart);
        // Maine and the Maritimes: the spruce and fir of the north woods among the hardwoods.
        let north = ramp(43.6, 45.2, lat);
        var dark = 0.0;
        if woods > 0.3 {
            dark = 1.0 - ramp(0.6, 0.8, photo_brightness(inputs));
        }
        let v = fract(pick * 9.71);
        if shrubland > 0.5 {
            // Bayberry, scrub oak and beach plum.
            species = SPECIES_SHRUB;
        } else if town {
            // Maples, oaks and lindens down the streets and in the yards, London planes in the cities, a few white
            // pines, spruces and birches.
            if u < 0.12 {
                species = seasonal(SPECIES_SYCAMORE, in_leaf);
            } else if u < 0.22 {
                species = select(SPECIES_SCOTS_PINE, SPECIES_SPRUCE, v < 0.4);
                scale = 0.8;
            } else if u < 0.3 {
                species = seasonal(SPECIES_BIRCH, in_leaf);
                scale = 0.8;
            } else {
                species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.6), in_leaf);
                scale = 0.9;
            }
        } else if fields > 0.5 || (s.a.w * w > 0.5 && woods < 0.3) {
            // Along the stone walls and in the old fields: maples and oaks, red cedars coming up.
            species = select(seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.45), in_leaf), SPECIES_CYPRESS, u > 0.88);
            scale = select(1.0, 0.45, species == SPECIES_CYPRESS);
            choice.tint = select(vec3<f32>(1.0), vec3<f32>(0.78, 0.86, 0.74), species == SPECIES_CYPRESS);
        } else if g.coast < 2500.0 && height < 40.0 && stand > 0.45 && north < 0.5 {
            // The sandy coast's pitch pine and scrub oak (Cape Cod, Plymouth, the islands).
            species = select(seasonal(SPECIES_OAK, in_leaf), SPECIES_SCOTS_PINE, u < 0.65);
            scale = 0.6;
        } else if dark > 0.5 && woods > 0.3 {
            // White pine in tall stands, hemlock in the ravines; in the north spruce and fir.
            if moist > 0.6 || u < 0.35 * north {
                species = SPECIES_SPRUCE;
                choice.tint = select(vec3<f32>(0.85, 0.95, 0.92), vec3<f32>(1.0), north > 0.5);
            } else {
                species = SPECIES_SCOTS_PINE;
                scale = 1.3;
                choice.tint = vec3<f32>(0.9, 1.0, 1.06);
            }
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.95);
        } else {
            // The hardwoods: oaks and hickories, red and sugar maple, beech, birch, a white pine standing over them.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.95);
            if u < 0.1 + 0.3 * north {
                species = select(SPECIES_SCOTS_PINE, SPECIES_SPRUCE, north > 0.5 || v < 0.3);
                scale = select(1.3, 1.0, species == SPECIES_SPRUCE);
            } else if u < 0.2 + 0.35 * north {
                species = seasonal(SPECIES_BIRCH, in_leaf);
            } else {
                species = seasonal(select(SPECIES_OAK, SPECIES_ASH, v < 0.45), in_leaf);
                choice.width = 1.1;
            }
        }
    } else if biome == BIOME_SOUTHEAST {
        let lat = latitude_at(inputs.chart);
        // South Florida and the Keys: coconut and royal palms, mangroves, gumbo-limbo and mahogany hammocks, slash pine
        // on the rock; the cabbage palm north to about Cape Fear.
        let south = 1.0 - ramp(26.4, 27.6, lat);
        let palms = 1.0 - ramp(33.2, 34.6, lat);
        let compartment = parcels(inputs, 380.0, 12.0);
        let wet = s.c.y * w;
        let v = fract(pick * 9.71);
        if mangroves > 0.4 || (lat < 30.5 && height < 3.0 && g.coast < 300.0 && wet > 0.3) {
            // A closed thicket of red and black mangrove, its crowns meeting.
            species = SPECIES_MANGROVE;
            choice.density = max(choice.density, max(mangroves, wet) * 0.9);
            choice.width = 1.5;
        } else if shrubland > 0.5 {
            // Saw palmetto, wax myrtle, yaupon.
            species = SPECIES_SHRUB;
            choice.tint = vec3<f32>(1.0, 1.05, 0.8);
        } else if town {
            if v < south {
                // Key West's and Miami's: a leafy canopy of ficus, mahogany, gumbo-limbo and sea grape (the laurel),
                // royal poinciana, live oaks, coconut and royal palms standing out of it, closed where the land cover
                // has trees.
                choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.9);
                if u < 0.12 {
                    species = SPECIES_PALM;
                } else if u < 0.17 {
                    species = SPECIES_FAN_PALM;
                    scale = 0.8;
                    choice.tint = vec3<f32>(1.1, 1.1, 1.0);
                } else if u < 0.27 {
                    species = SPECIES_ACACIA;
                    scale = 1.4;
                    choice.width = 1.3;
                } else if u < 0.4 {
                    species = SPECIES_LIVE_OAK;
                    choice.width = 1.2;
                } else {
                    species = SPECIES_LAUREL;
                    scale = 0.95;
                    choice.width = 1.3;
                }
            } else {
                // Live oaks over the streets and squares, cabbage palms, magnolias and crape myrtles, loblolly pines,
                // water oaks.
                if u < 0.36 {
                    species = SPECIES_LIVE_OAK;
                    choice.width = 1.2;
                } else if u < 0.36 + 0.18 * palms {
                    species = SPECIES_FAN_PALM;
                    scale = 0.55;
                } else if u < 0.66 {
                    species = select(SPECIES_LAUREL, seasonal(SPECIES_OAK, in_leaf), v < 0.5);
                    scale = 0.8;
                } else {
                    species = SPECIES_SCOTS_PINE;
                    scale = 1.15;
                    choice.tint = vec3<f32>(0.95, 1.05, 0.92);
                }
            }
        } else if wet > 0.3 || (moist > 0.75 && height < 25.0) {
            // The swamps and bottomlands: bald cypress (feathery, rust-brown in winter) and tupelo, gum and water oak.
            if u < 0.5 {
                species = SPECIES_REDWOOD;
                scale = 0.5;
                choice.tint = select(vec3<f32>(1.35, 0.95, 0.65), vec3<f32>(1.08, 1.08, 0.9), in_leaf);
            } else {
                species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.75), in_leaf);
            }
            choice.density = max(choice.density, wet * 0.85);
        } else if south > 0.5 {
            // South Florida's hammocks and pine rockland: gumbo-limbo, mahogany and live oak, slash pine over palmetto,
            // cabbage palms.
            species = select(select(SPECIES_LAUREL, SPECIES_LIVE_OAK, u < 0.35), SPECIES_SCOTS_PINE, stand > 0.55);
            species = select(species, SPECIES_FAN_PALM, u > 0.88);
            scale = select(0.7, 0.55, species == SPECIES_FAN_PALM);
        } else if g.coast < 800.0 && height < 15.0 {
            // The maritime forest behind the dunes: live oaks sheared by the wind, cabbage palms, loblolly pine.
            if u < 0.5 {
                species = SPECIES_LIVE_OAK;
                scale = 0.75;
                choice.width = 1.25;
            } else if u < 0.5 + 0.25 * palms {
                species = SPECIES_FAN_PALM;
                scale = 0.5;
            } else {
                species = SPECIES_SCOTS_PINE;
                choice.tint = vec3<f32>(0.95, 1.05, 0.92);
            }
        } else if fields > 0.5 || (s.a.w * w > 0.5 && woods < 0.3) {
            // The fields' edges and the farmsteads: oaks, pines, pecans.
            species = select(seasonal(SPECIES_OAK, in_leaf), SPECIES_SCOTS_PINE, u < 0.35);
        } else if fract(compartment.id * 3.7) < 0.55 && height < 200.0 {
            // Loblolly and slash pine plantations: compartments of one age, some clear-felled.
            let age = fract(compartment.id * 7.31);
            species = SPECIES_SCOTS_PINE;
            choice.tint = vec3<f32>(0.95, 1.05, 0.92);
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.96);
            felled = select(1.0, 0.0, age < 0.12) * (1.0 - compartment.boundary);
            choice.variety = 0.3;
            scale = 0.4 + 0.85 * age;
        } else {
            // The mixed woods: loblolly pine, oaks, sweetgum and hickory.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.95);
            if u < 0.4 {
                species = SPECIES_SCOTS_PINE;
                scale = 1.15;
                choice.tint = vec3<f32>(0.95, 1.05, 0.92);
            } else {
                species = select(seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.6), in_leaf), SPECIES_LIVE_OAK, u > 0.92);
            }
        }
    } else if biome == BIOME_HAWAII {
        let v = fract(pick * 9.71);
        // The dry leeward lowlands: kiawe and haole koa on the old cane land, the plains and the lower slopes.
        let leeward = (1.0 - ramp(250.0, 450.0, height)) * (1.0 - moist) * select(0.0, 1.0, woods < 0.6);
        if mangroves > 0.4 {
            species = SPECIES_MANGROVE;
        } else if town {
            // Monkeypods' great umbrellas, banyans, coconut and royal palms, shower trees and plumeria, mango.
            if u < 0.24 {
                species = SPECIES_ACACIA;
                scale = 2.0;
                choice.width = 1.35;
            } else if u < 0.38 {
                species = SPECIES_PALM;
            } else if u < 0.43 {
                species = SPECIES_FAN_PALM;
                scale = 0.8;
                choice.tint = vec3<f32>(1.1, 1.1, 1.0);
            } else {
                species = SPECIES_LAUREL;
                scale = select(0.55, 1.1, v < 0.4);
                choice.width = select(1.0, 1.3, v < 0.4);
            }
        } else if g.coast < 300.0 && height < 12.0 {
            // The shore: coconut palms, ironwood (casuarina), kiawe and naupaka.
            if u < 0.5 {
                species = SPECIES_PALM;
            } else if u < 0.8 {
                species = SPECIES_SCOTS_PINE;
                scale = 0.85;
                choice.tint = vec3<f32>(0.85, 0.92, 0.88);
            } else {
                species = SPECIES_ACACIA;
            }
        } else if leeward > 0.5 {
            // Kiawe woodland and haole koa scrub, grey-green in the dry season.
            species = select(SPECIES_SHRUB, SPECIES_ACACIA, u < 0.55 || shrubland < 0.5);
            choice.tint = vec3<f32>(1.06, 1.06, 0.9);
            choice.density = max(choice.density, (shrubland + woods) * 0.6);
        } else if shrubland > 0.5 {
            species = SPECIES_SHRUB;
        } else {
            // The wet windward valleys and the uplands: a closed forest of introduced trees (mango, kukui, albizia,
            // octopus tree), eucalyptus and paperbark groves, Cook pines planted on the ridges; ohia above.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.97);
            if grove > 0.7 && height < 900.0 {
                species = SPECIES_EUCALYPTUS;
            } else if u < 0.04 {
                species = SPECIES_CYPRESS;
                scale = 1.6;
            } else if height > 900.0 {
                species = SPECIES_LAUREL;
                scale = 0.6;
            } else {
                species = select(SPECIES_LAUREL, SPECIES_RAINFOREST, u < 0.55);
                scale = select(1.0, 0.6, species == SPECIES_RAINFOREST);
                choice.width = 1.25;
            }
        }
    } else if biome == BIOME_INTERIOR_WEST {
        // The interior West in belts by height (`west_belt`): desert scrub and sagebrush below the woodland (shrubs,
        // `place_cover`), pinyon pine and Utah juniper, ponderosa pine with Gambel oak, Douglas fir and white fir with
        // aspen, Engelmann spruce and subalpine fir to the treeline; cottonwoods along the rivers and in the towns.
        // WorldCover's tree cover decides where they stand. Pinyon is the Monterey pine's dense dome-cone, small;
        // juniper the olive's crooked open crown, blue-grey; ponderosa the Scots pine's tall bole, taller; the firs
        // and spruces the spruce's cone; aspen the birch.
        let belt = west_belt(inputs, g);
        let v = fract(pick * 9.71);
        // The rivers' bottoms only, where the water gathers on the flat (the plateau's dry washes and gullies keep
        // their pinyon and juniper).
        let riparian = ramp(8.5, 10.5, g.wetness) * (1.0 - ramp(0.12, 0.25, g.slope));
        if (riparian > 0.5 && belt < 1900.0 && woods > 0.3) || (fields > 0.5 && belt < 2300.0) {
            // Fremont and narrowleaf cottonwoods, willows along the rivers and the irrigation ditches.
            species = seasonal(SPECIES_ASH, in_leaf);
            choice.tint = vec3<f32>(1.05, 1.08, 0.86);
            scale = select(0.95, 0.7, fields > 0.5);
        } else if town {
            // Cottonwoods, elms and ash in the yards and streets; ponderosa and blue spruce in the mountain towns.
            if belt > 2000.0 && u < 0.4 {
                species = select(SPECIES_SCOTS_PINE, SPECIES_SPRUCE, u < 0.12);
                scale = select(1.1, 0.7, species == SPECIES_SPRUCE);
            } else {
                species = seasonal(select(SPECIES_OAK, SPECIES_ASH, u < 0.6), in_leaf);
                scale = 0.85;
            }
        } else if belt < 1450.0 || (shrubland > 0.5 && belt < 2000.0) {
            // The desert's few trees and the shrubland's: juniper on the rocks, low, scattered.
            species = select(SPECIES_SHRUB, SPECIES_OLIVE, u < 0.6 || shrubland <= 0.5);
            scale = select(0.8, 0.55, species == SPECIES_OLIVE);
            choice.tint = select(vec3<f32>(0.9, 0.92, 0.82), vec3<f32>(0.86, 0.95, 0.94), species == SPECIES_OLIVE);
        } else if shrubland > 0.5 {
            // Gambel oak and mountain mahogany thickets, manzanita.
            species = select(SPECIES_SHRUB, seasonal(SPECIES_OAK, in_leaf), u < 0.3 && belt < 2700.0);
            scale = select(1.0, 0.35, species != SPECIES_SHRUB);
        } else if belt < 2150.0 {
            // Pinyon-juniper woodland: open, its trees low and round.
            if u < 0.55 {
                species = SPECIES_MONTEREY_PINE;
                scale = 0.28 + 0.08 * v;
                choice.width = 1.25;
                choice.tint = vec3<f32>(0.82, 0.92, 0.82);
            } else {
                species = SPECIES_OLIVE;
                scale = 0.7;
                choice.tint = vec3<f32>(0.86, 0.95, 0.94);
            }
        } else if belt < 2700.0 {
            // Ponderosa pine: tall, open, park-like; Gambel oak under them, pinyon at its lower edge.
            choice.density = max(choice.density, ramp(0.3, 0.8, woods) * 0.7);
            if u < 0.08 {
                species = seasonal(SPECIES_OAK, in_leaf);
                scale = 0.4;
            } else if u < 0.18 && belt < 2300.0 {
                species = SPECIES_MONTEREY_PINE;
                scale = 0.32;
                choice.width = 1.25;
                choice.tint = vec3<f32>(0.82, 0.92, 0.82);
            } else {
                species = SPECIES_SCOTS_PINE;
                scale = 1.15 + 0.25 * v;
                choice.tint = vec3<f32>(1.0, 1.06, 0.9);
            }
        } else {
            // Mixed conifer (Douglas fir, white fir, ponderosa at its foot), then spruce-fir to the treeline; aspen in
            // its groves.
            choice.density = max(choice.density, ramp(0.25, 0.75, woods) * 0.9);
            let spruce_fir = ramp(2950.0, 3100.0, belt);
            if stand > 0.68 && belt < 3300.0 {
                species = seasonal(SPECIES_BIRCH, in_leaf);
                scale = 1.1;
                choice.tint = vec3<f32>(1.02, 1.06, 0.9);
            } else if u < 0.15 * (1.0 - spruce_fir) {
                species = SPECIES_SCOTS_PINE;
                scale = 1.2;
                choice.tint = vec3<f32>(1.0, 1.06, 0.9);
            } else if spruce_fir > 0.5 {
                // Engelmann spruce, blue-green; subalpine fir, a narrow spire; lower toward the treeline.
                species = SPECIES_SPRUCE;
                scale = select(0.95, 0.8, u > 0.6) * (1.0 - 0.45 * ramp(3300.0, 3600.0, belt));
                choice.width = select(1.0, 0.7, u > 0.6);
                choice.tint = vec3<f32>(0.84, 0.95, 1.04);
            } else {
                // Douglas fir, dark; white fir, grey-blue.
                species = SPECIES_SPRUCE;
                scale = select(1.2, 1.0, u > 0.75);
                choice.tint = select(vec3<f32>(0.86, 0.95, 0.84), vec3<f32>(0.88, 0.97, 1.02), u > 0.75);
            }
        }
    } else if biome == BIOME_SUBTROPICAL {
        if g.coast < 600.0 {
            species = select(SPECIES_LIVE_OAK, SPECIES_MONTEREY_PINE, u < 0.6);
        } else if stand > 0.6 {
            species = SPECIES_MONTEREY_PINE;
        } else {
            species = select(seasonal(SPECIES_OAK, fract(pick * 7.13) < max(leaf, 0.3)), SPECIES_LIVE_OAK, u < 0.55);
        }
        species = select(species, SPECIES_SHRUB, shrubland > 0.5);
    } else if biome == BIOME_TEMPERATE {
        if shrubland > 0.5 {
            species = SPECIES_SHRUB;
        } else if height > 800.0 {
            species = select(SPECIES_SCOTS_PINE, SPECIES_SPRUCE, stand < 0.6);
        } else if g.coast < 500.0 && stand > 0.5 {
            species = SPECIES_SCOTS_PINE;
        } else if stand > 0.72 {
            species = select(SPECIES_SCOTS_PINE, SPECIES_SPRUCE, u < 0.6);
        } else {
            species = seasonal(select(select(SPECIES_BIRCH, SPECIES_ASH, u < 0.8), SPECIES_OAK, u < 0.45), in_leaf);
        }
    } else if biome == BIOME_BOREAL {
        // The mountain birch's belt under the treeline (by the latitude, its edge a band up the slope: `birch_belt`),
        // lower toward the line; under it spruce where it is native, Scots pine, birch (`boreal_conifers`: past the
        // spruce's northern limit, Ofoten's and Troms' woods, birch with the pine in stands on the dry ground).
        let lat = latitude_at(inputs.chart);
        let treeline = boreal_treeline(lat);
        if fract(pick * 2.39) < birch_belt(inputs, g, treeline) {
            species = seasonal(SPECIES_BIRCH, in_leaf);
            scale = 0.75 - 0.3 * ramp(treeline - 300.0, treeline, height);
        } else {
            let conifers = boreal_conifers(inputs, moist, spruce_native(lat, longitude_at(inputs.chart)));
            species = select(select(seasonal(SPECIES_BIRCH, in_leaf), SPECIES_SCOTS_PINE, u < conifers.x + conifers.y),
                SPECIES_SPRUCE, u < conifers.x);
        }
    } else {
        // Subarctic: low birch scrub, a few planted spruce (none past the spruce's limit: Finnmark's coast).
        let native = spruce_native(latitude_at(inputs.chart), longitude_at(inputs.chart));
        species = select(seasonal(SPECIES_BIRCH, in_leaf), SPECIES_SPRUCE, u < 0.12 * native);
        scale = select(0.5, 0.7, species == SPECIES_SPRUCE);
    }
    choice.species = species;
    // Lower in fields, towns and on wind-swept headlands than in a wood (a town's palms as tall as they grow).
    let tall_in_town = species == SPECIES_FAN_PALM || species == SPECIES_DATE_PALM;
    let open_scale = mix(1.0, select(0.75, 1.0, tall_in_town), (s.b.x + s.b.y) * w);
    choice.height = species_height(species) * scale * open_scale * (1.0 - 0.3 * headland);
    // A density raised above (a closed canopy, a plantation, a thicket) keeps `tree_cover`'s exclusions: none on a
    // road, a railway or a river, at the water's edge or on a cliff, where the maps cover the point.
    let standing = ramp(1.5, 3.0, height) * (1.0 - ramp(0.75, 1.0, g.slope));
    choice.density = max(cover.density, choice.density * mix(1.0, standing, w) * (1.0 - feature_clearance(inputs.chart)))
        * felled;
    choice.wild = ramp(0.4, 0.8, woods) * select(1.0, 0.0, town || choice.rows >= 0.0) * choice.variety;
    return choice;
}

// The height (m) of the interior West's belts a point stands in, as they stand at 36 N: some 100 m lower a degree north,
// a shaded face's belts lower than a sunny one's (a sunny face's ponderosa as high as a shaded face's firs 300 m above),
// the edges between them wandering.
fn west_belt(inputs: SurfaceInputs, g: Ground) -> f32 {
    return inputs.height + (latitude_at(inputs.chart) - 36.0) * 100.0 - g.sunny * 150.0 + (noise_r(inputs, 300.0) - 0.5) * 220.0;
}

// The land cover's woods about a foot, from its window maps: the share of the four points `EDGE_M` off (east, south,
// west, north) that are woods (WorldCover's tree cover or mangroves), and the way toward those that are open (chart,
// unnormalised; zero with no maps).
fn woods_about(foot: vec2<f32>) -> vec3<f32> {
    if terrain.surface.w <= 0.0 {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let texel = max(terrain.surface.z / terrain.surface.w, 1.0);
    let size = i32(terrain.surface.w);
    var woods = 0.0;
    var open = vec2<f32>(0.0);
    for (var k = 0; k < 4; k += 1) {
        let a = f32(k) * 1.5707963;
        let d = vec2<f32>(cos(a), sin(a));
        let raw = cover_raw(vec2<i32>(floor((foot + d * EDGE_M - terrain.surface.xy) / texel)), size, 0) & 0x7fu;
        let wood = select(0.0, 1.0, raw == 10u || raw == 95u);
        woods += wood * 0.25;
        open += d * (1.0 - wood);
    }
    return vec3<f32>(woods, open);
}

// Conifers, whose dead stand with their needles browned; the evergreen broadleaves, whose dead stand bare as the oak's
// winter self (a palm, a stand of many trunks or a shrub stands no dead).
fn conifer(species: f32) -> bool {
    return species == SPECIES_SPRUCE || species == SPECIES_SCOTS_PINE || species == SPECIES_BLACK_PINE
        || species == SPECIES_MONTEREY_PINE || species == SPECIES_STONE_PINE || species == SPECIES_REDWOOD
        || species == SPECIES_CYPRESS || species == SPECIES_MONTEREY_CYPRESS;
}

fn evergreen_broadleaf(species: f32) -> bool {
    return species == SPECIES_LIVE_OAK || species == SPECIES_LAUREL || species == SPECIES_EUCALYPTUS
        || species == SPECIES_RAINFOREST || species == SPECIES_EMERGENT || species == SPECIES_OLIVE;
}

// A stand of many trunks, a thicket or ground cover: one model a slot, which no lean of one tree fits.
fn many(species: f32) -> bool {
    return species == SPECIES_SUGI || species == SPECIES_BAMBOO || species == SPECIES_CHAPARRAL || shrubby(species);
}

// A tree's turn and form packed in one float of its instance (`impostor::form_of`): the yaw (radians) the fraction of
// a turn, the whole part a code of its top's lean over its height (chart x and z, in fifteenths of `LEAN_MOST`, 15
// upright; a palm's in fifteenths of `PALM_LEAN_MOST`, marked by `PALM_FORM`) and its crown's stretch along its own x
// (0-1, in fifteenths). A tree's code stays under 2^14, so its turn keeps ten bits; a palm's under 2^15, nine.
fn packed_form(yaw: f32, lean: vec2<f32>, stretch: f32, palm: bool) -> f32 {
    let l = clamp(round(lean / select(LEAN_MOST, PALM_LEAN_MOST, palm) * 15.0) + 15.0, vec2<f32>(0.0), vec2<f32>(30.0));
    let code = l.x + l.y * 32.0 + clamp(round(stretch * 15.0), 0.0, 15.0) * 1024.0 + select(0.0, f32(PALM_FORM), palm);
    return code + min(fract(yaw / 6.2831853), 0.998);
}

// The palms (`species::Species::CoconutPalm`, `FanPalm`, `DatePalm`, `OasisPalm`), which lean far and from the foot.
fn palm(species: f32) -> bool {
    return species == SPECIES_PALM || species == SPECIES_FAN_PALM || species == SPECIES_DATE_PALM
        || species == SPECIES_OASIS_PALM;
}

// Scrub and grass: the land cover's, which the canopy map (its crowns 3 m and taller) does not place.
fn shrubby(species: f32) -> bool {
    return species == SPECIES_CHAPARRAL || species == SPECIES_SHRUB || species == SPECIES_GARRIGUE || species == SPECIES_GRASS;
}

// Canopy texel `t` (counted from page (0, 0)'s corner), zero where no slot holds its page.
fn canopy_texel(t: vec2<i32>) -> vec4<f32> {
    let slots = i32(view.canopy.w);
    let page = t >> vec2<u32>(8u);
    let slot = ((page % slots) + slots) % slots;
    let k = slot.y * slots + slot.x;
    let entry = view.canopy_slots[k / 2];
    let held = select(entry.xy, entry.zw, (k & 1) == 1);
    if any(held != page) {
        return vec4<f32>(0.0);
    }
    return textureLoad(canopy_pages, slot * CANOPY_PAGE_TEXELS + (t & vec2<i32>(CANOPY_PAGE_TEXELS - 1)), 0);
}

// Whether a canopy page covers chart point `at`.
fn canopy_known(at: vec2<f32>) -> bool {
    if view.canopy.w < 1.0 {
        return false;
    }
    return canopy_texel(vec2<i32>(floor((at - view.canopy.xy) / view.canopy.z))).a >= 0.5;
}

// What the canopy map says of a square of `side` metres from chart point `lo` (a slot's cell or a part of it, at most
// 5 texels a side): whether a page covers it, the tallest crown standing in it (its apex and height, m; 0 for none) and
// the share of it under canopy.
struct Crown {
    known: bool,
    apex: vec2<f32>,
    height: f32,
    cover: f32,
}

fn canopy_crown(lo: vec2<f32>, side: f32) -> Crown {
    var crown: Crown;
    crown.known = false;
    crown.apex = lo + side * 0.5;
    crown.height = 0.0;
    crown.cover = 0.0;
    // Known by the texel under its middle.
    if !canopy_known(lo + side * 0.5) {
        return crown;
    }
    crown.known = true;
    let origin = view.canopy.xy;
    let texel_m = view.canopy.z;
    let first = vec2<i32>(floor((lo - origin) / texel_m));
    let last = min(vec2<i32>(floor((lo + side - origin) / texel_m - 1e-3)), first + 4);
    var cover = 0.0;
    for (var j = first.y; j <= last.y; j += 1) {
        for (var i = first.x; i <= last.x; i += 1) {
            let t = canopy_texel(vec2<i32>(i, j));
            let corner = origin + vec2<f32>(f32(i), f32(j)) * texel_m;
            let overlap = max(min(lo + side, corner + texel_m) - max(lo, corner), vec2<f32>(0.0));
            cover += t.b * overlap.x * overlap.y;
            let h = round(t.r * 255.0);
            if h > crown.height {
                let g = u32(round(t.g * 255.0));
                let apex = corner + (vec2<f32>(f32(g >> 4u), f32(g & 15u)) + 0.5) / 16.0 * texel_m;
                if all(apex >= lo) && all(apex < lo + side) {
                    crown.height = h;
                    crown.apex = apex;
                }
            }
        }
    }
    crown.cover = cover / (side * side);
    return crown;
}

// Whether the slot at `lo` (`spacing` a side, `cover` the share of it under canopy, no crown in it) lies on a bare
// patch: the slot itself where it is CANOPY_BARE_M or wider, else the square of CANOPY_BARE_M about its middle.
fn canopy_bare(lo: vec2<f32>, spacing: f32, cover: f32) -> bool {
    if cover >= CANOPY_BARE {
        return false;
    }
    if spacing >= CANOPY_BARE_M {
        return true;
    }
    let around = canopy_crown(lo + (spacing - CANOPY_BARE_M) * 0.5, CANOPY_BARE_M);
    return around.known && around.height <= 0.0 && around.cover < CANOPY_BARE;
}

// Stands the choice's tree at `at` (its ground `at_ground`, the slope `gradient` there), `key` its own hash: sunk on
// slopes, so the downhill side of its foot does not float (a thicket wider than it is tall by most of its reach, its
// uphill edge buried; a stand of many trunks, the sugi's or the bamboo's, by its whole footprint, as its outer trunks
// stand far out), each tree its own shade (brighter or darker, a little yellower or bluer) and its own turn about its
// axis.
fn plant(at: vec2<f32>, at_ground: f32, gradient: vec2<f32>, tree_height: f32, width: f32, choice: Choice, open: f32, key: vec2<f32>) {
    var species = choice.species;
    let brightness = 0.96 + (cell_hash(key, 8.0) - 0.5) * 0.48 * choice.variety;
    let hue = (cell_hash(key, 10.0) * 2.0 - 1.0) * choice.variety;
    var tint = choice.tint * brightness * vec3<f32>(1.0 + 0.14 * hue, 1.0, 1.0 - 0.2 * hue);
    // A wild wood's dead: a broadleaf bare (a deciduous one its winter self, an evergreen the oak's), a conifer with its
    // needles browned; greyer than the living.
    if cell_hash(key, 15.0) < SNAGS * choice.wild && !many(species) {
        let winter = view.species[u32(species)].z;
        if winter >= 0.0 {
            species = winter;
        } else if evergreen_broadleaf(species) {
            species = SPECIES_BARE_OAK;
        } else if conifer(species) {
            tint *= vec3<f32>(1.25, 0.8, 0.55);
        }
        tint *= 0.88;
    }
    let radius = view.species[u32(species)].x * tree_height * width;
    let stand = species == SPECIES_SUGI || species == SPECIES_BAMBOO;
    let sunk = select(min(radius, 3.0), radius * 1.5, radius > tree_height * 1.5 || stand);
    let base = at_ground - length(gradient) * sunk * 0.5 - 0.3;
    // Its form: a lean of its own (most within a degree or so of upright, one in seven up to six degrees), a little
    // downhill on a slope, and the choice's (a wood's edge tree reaching out to the light); its crown wider one way than
    // the other, along the choice's lean where it gives one. A plantation's trees stand straighter, a stand of many
    // trunks or a thicket upright.
    var yaw = cell_hash(key, 7.0) * 6.2831853;
    var lean = vec2<f32>(0.0);
    var stretch = cell_hash(key, 14.0) * choice.variety;
    let leans_far = palm(species);
    if leans_far {
        // A palm leans from its foot, and far (measured: a coconut palm mostly 5-30 degrees, rarely upright): a coconut
        // palm 3-25 degrees, half of them under 11, a date or oasis palm up to 16, a fan palm up to 9, a plantation's
        // rows less; out over the open ground at a wild wood's edge (within 70 degrees of it), a little downhill. A
        // coconut palm's model curves (its foot leaning along its own x, its top turned up: `species.rs`), laid along
        // its lean, one in four against it (an S).
        let tilt = cell_hash(key, 12.0);
        var heading = cell_hash(key, 13.0) * 6.2831853;
        if dot(choice.lean, choice.lean) > 1e-4 {
            heading = atan2(choice.lean.y, choice.lean.x) + (cell_hash(key, 13.0) - 0.5) * 2.4;
        }
        let coconut = species == SPECIES_PALM;
        let least = select(1.0, 3.0, coconut);
        let most = select(select(15.0, 8.0, species == SPECIES_FAN_PALM), 22.0, coconut);
        let degrees = (least + most * pow(tilt, 1.5)) * (0.4 + 0.6 * choice.variety);
        lean = vec2<f32>(cos(heading), sin(heading)) * tan(radians(degrees)) - gradient * 0.12;
        lean *= min(1.0, PALM_LEAN_MOST * 0.95 / max(length(lean), 1e-4));
        if coconut {
            let along = heading + select(0.0, 3.1415927, cell_hash(key, 16.0) < 0.25);
            yaw = atan2(-sin(along), cos(along));
        }
    } else if !many(species) {
        let tilt = cell_hash(key, 12.0);
        let heading = cell_hash(key, 13.0) * 6.2831853;
        let own = select(0.02 * tilt / 0.86, 0.035 + 0.07 * (tilt - 0.86) / 0.14, tilt > 0.86) * choice.variety;
        lean = vec2<f32>(cos(heading), sin(heading)) * own - gradient * 0.12 + choice.lean;
        lean *= min(1.0, LEAN_MOST * 0.95 / max(length(lean), 1e-4));
        if dot(choice.lean, choice.lean) > 1e-4 {
            // Its own x (`impostor::turn_by` takes it to (cos yaw, -sin yaw)) toward the light.
            let d = normalize(choice.lean);
            yaw = atan2(-d.y, d.x);
            stretch = max(stretch, 0.8);
        }
    }
    append(at, base, tree_height, species, packed_form(yaw, lean, stretch, leans_far), open, tint, width);
}

fn append(foot: vec2<f32>, base: f32, tree_height: f32, species: f32, form: f32, open: f32, tint: vec3<f32>, width: f32) {
    let index = atomicAdd(&args.instance_count, 1u);
    if index >= arrayLength(&trees) {
        // Full: take the count back, so the draw never reads past the buffer.
        atomicSub(&args.instance_count, 1u);
        return;
    }
    let radius = view.species[u32(species)].x;
    trees[index] = Tree(vec4<f32>(foot.x, base, foot.y, tree_height), vec4<f32>(radius, species, form, open), vec4<f32>(tint, width));
}

// ---- The period's country (`surface.wgsl`'s "The period's country") ----
//
// On the land a past period leaves as country the measured canopy (today's street trees and gardens) stands no tree.
// The country's own stand instead, by its field (`country_field`, as the land paints it): a woodlot's or a grove's
// trees as a wood's; an orchard's, an olive grove's or a plantation's in rows on the crowns the land paints, every one
// close to the camera (`ORCHARD_NEAR_M`) and beyond it those a slot apart; hedgerow trees and windbreaks on the fields'
// edges; a farmstead's trees about its place; elsewhere a lone tree now and then.

// Out to this apparent distance an orchard's every tree stands (beyond it they thin as the slots do from
// `view.lod.x`, but for the rows a slot apart).
const ORCHARD_NEAR_M: f32 = 700.0;
// A farmstead's trees stand within this of its place (m).
const FARMSTEAD_M: f32 = 45.0;

// A tree of the country: its species, scale of the species' height, tint and width (`Choice`'s).
fn country_tree(species: f32, scale: f32, tint: vec3<f32>, width: f32) -> Choice {
    var c: Choice;
    c.species = species;
    c.height = select(0.0, species_height(max(species, 0.0)) * scale, species >= 0.0);
    c.tint = tint;
    c.density = 1.0;
    c.width = width;
    c.variety = 1.0;
    c.rows = -1.0;
    return c;
}

// An orchard's trees in `field`, as the land paints them (`surface.wgsl`'s `country_ground`): their spacing (m, before
// the slot's grid rounds it), whether those a slot apart stand beyond `ORCHARD_NEAR_M`, and the tree (none, a species
// below 0, for Japan's mulberry bushes, which the land's relief alone draws).
struct OrchardTrees {
    desired: f32,
    far: bool,
    tree: Choice,
}

fn orchard_trees(region: u32, field: CountryField, in_leaf: bool) -> OrchardTrees {
    var o: OrchardTrees;
    o.far = true;
    if field.kind == FIELD_PALMS {
        o.desired = 9.0;
        o.tree = country_tree(SPECIES_PALM, 0.85, vec3<f32>(0.95, 0.97, 0.9), 1.0);
    } else if field.kind == FIELD_OLIVES {
        o.desired = 9.0;
        o.tree = country_tree(SPECIES_OLIVE, 0.85, vec3<f32>(1.04, 1.05, 1.0), 1.3);
    } else if region == COUNTRY_WEST || region == COUNTRY_MEDITERRANEAN {
        if fract(field.id * 7.7) < 0.7 {
            // Oranges and lemons: low, dark and close; seen from afar the land's painted rows.
            o.desired = 7.0;
            o.far = false;
            o.tree = country_tree(SPECIES_LAUREL, 0.32, vec3<f32>(0.78, 0.9, 0.72), 1.25);
        } else {
            o.desired = 11.0;
            o.tree = country_tree(seasonal(SPECIES_OAK, in_leaf), 0.6, vec3<f32>(1.08, 1.1, 0.95), 1.2);
        }
    } else if region == COUNTRY_JAPAN {
        o.desired = 3.0;
        o.far = false;
        o.tree = country_tree(-1.0, 0.0, vec3<f32>(1.0), 1.0);
    } else {
        // Apples.
        o.desired = 8.0;
        o.far = false;
        o.tree = country_tree(seasonal(SPECIES_OAK, in_leaf), 0.3, vec3<f32>(0.98, 1.04, 0.92), 1.3);
    }
    o.tree.variety = 0.4;
    return o;
}

// The trees on a field's edges in `region`: the chance one stands on the edge in a slot it crosses, how many along it,
// how far off the edge's middle (m: beside a track), and the tree; a chance of 0 for none. The West's and the
// Mediterranean's windbreaks line some fields' edges only (eucalyptus, cypress), Britain's hedgerows stand oaks and
// ashes in most, the open fields few.
struct EdgeTrees {
    chance: f32,
    count: i32,
    aside: f32,
    tree: Choice,
}

fn edge_trees(region: u32, field: CountryField, pick: f32, in_leaf: bool) -> EdgeTrees {
    var e: EdgeTrees;
    e.chance = 0.0;
    e.count = 1;
    e.aside = 0.0;
    let u = fract(pick * 5.37);
    let lined = fract(field.id * 3.31);
    switch region {
        case COUNTRY_WEST: {
            if lined < 0.35 {
                e.chance = 0.9;
                e.count = 3;
                e.aside = 4.0;
                e.tree = country_tree(SPECIES_EUCALYPTUS, 0.75, vec3<f32>(1.0), 0.75);
            } else {
                e.chance = 0.04;
                e.aside = 3.5;
                e.tree = country_tree(select(SPECIES_LAUREL, SPECIES_FAN_PALM, u < 0.3), select(0.6, 0.8, u < 0.3),
                    vec3<f32>(1.0), 1.0);
            }
        }
        case COUNTRY_EAST: {
            e.chance = 0.35;
            e.tree = country_tree(seasonal(select(SPECIES_ASH, SPECIES_OAK, u < 0.6), in_leaf), 0.85, vec3<f32>(1.0), 1.1);
        }
        case COUNTRY_BRITAIN: {
            e.chance = 0.4;
            e.tree = country_tree(seasonal(select(SPECIES_ASH, SPECIES_OAK, u < 0.65), in_leaf), 0.9, vec3<f32>(1.0), 1.15);
        }
        case COUNTRY_EUROPE: {
            e.chance = select(0.05, 0.6, lined < 0.12);
            e.tree = country_tree(seasonal(SPECIES_ASH, in_leaf), 0.85, vec3<f32>(1.0), 0.9);
        }
        case COUNTRY_JAPAN: {
            e.chance = 0.04;
            e.tree = country_tree(select(SPECIES_LAUREL, SPECIES_BLACK_PINE, u < 0.4), 0.6, vec3<f32>(1.0), 1.0);
        }
        case COUNTRY_MEDITERRANEAN: {
            if lined < 0.2 {
                e.chance = 0.85;
                e.count = 2;
                e.aside = 1.5;
                e.tree = country_tree(SPECIES_CYPRESS, 0.8, vec3<f32>(1.15, 1.2, 1.0), 0.9);
            } else {
                e.chance = 0.06;
                e.tree = country_tree(SPECIES_OLIVE, 0.9, vec3<f32>(1.04, 1.05, 1.0), 1.3);
            }
        }
        default: {}
    }
    return e;
}

// A farmstead's trees in `region` (shade about the house, a shelter belt), the share of fields with one, and a lone
// tree's in a field (`lone` its chance).
struct HomeTrees {
    farms: f32,
    lone: f32,
    tree: Choice,
    lone_tree: Choice,
}

fn home_trees(region: u32, field: CountryField, pick: f32, in_leaf: bool, spruce: f32) -> HomeTrees {
    var h: HomeTrees;
    let u = fract(pick * 5.37);
    h.farms = 0.08;
    h.lone = 0.01;
    h.tree = country_tree(seasonal(SPECIES_OAK, in_leaf), 0.8, vec3<f32>(1.0), 1.1);
    h.lone_tree = h.tree;
    switch region {
        case COUNTRY_WEST: {
            h.farms = 0.12;
            h.lone = 0.006;
            let species = select(select(SPECIES_LAUREL, SPECIES_FAN_PALM, u < 0.25), SPECIES_EUCALYPTUS, u > 0.7);
            h.tree = country_tree(species, select(0.6, 0.7, species != SPECIES_LAUREL), vec3<f32>(1.0), 1.1);
            h.lone_tree = country_tree(SPECIES_LIVE_OAK, 0.8, vec3<f32>(1.0), 1.3);
        }
        case COUNTRY_EAST: {
            h.farms = 0.12;
        }
        case COUNTRY_BRITAIN: {
            // Parkland oaks stand alone in the pasture.
            h.lone = select(0.012, 0.03, field.kind == FIELD_PASTURE);
        }
        case COUNTRY_EUROPE: {
            h.farms = 0.05;
            h.lone = 0.005;
        }
        case COUNTRY_JAPAN: {
            // The villages are the groves (`FIELD_WOOD`); no tree among the paddies.
            h.farms = 0.03;
            h.lone = select(0.004, 0.0, field.kind == FIELD_PADDY);
            h.tree = country_tree(select(SPECIES_LAUREL, SPECIES_BAMBOO, u < 0.35), 0.6, vec3<f32>(1.0), 1.0);
            h.lone_tree = country_tree(SPECIES_BLACK_PINE, 0.6, vec3<f32>(1.0), 1.0);
        }
        case COUNTRY_TROPICS: {
            h.farms = 0.06;
            h.lone = select(select(0.015, 0.025, field.kind == FIELD_ROUGH), 0.12, field.kind == FIELD_GARDEN);
            h.tree = country_tree(select(SPECIES_LAUREL, SPECIES_PALM, u < 0.5), select(0.7, 0.85, u < 0.5),
                vec3<f32>(1.0), 1.1);
            h.lone_tree = h.tree;
        }
        case COUNTRY_NORDIC: {
            // A rowan or birch by the farm, a few planted spruce (none past the spruce's limit, `spruce` its native
            // share); else none.
            h.farms = 0.1;
            h.lone = 0.0;
            h.tree = country_tree(select(seasonal(SPECIES_BIRCH, in_leaf), SPECIES_SPRUCE, u < 0.3 * spruce), 0.45,
                vec3<f32>(1.0), 1.0);
        }
        case COUNTRY_MEDITERRANEAN: {
            h.farms = 0.07;
            h.lone = 0.012;
            h.tree = country_tree(select(select(SPECIES_OLIVE, SPECIES_CYPRESS, u < 0.35), SPECIES_STONE_PINE, u > 0.8),
                0.9, vec3<f32>(1.0), 1.1);
            h.lone_tree = country_tree(SPECIES_OLIVE, 0.9, vec3<f32>(1.04, 1.05, 1.0), 1.3);
        }
        default: {}
    }
    return h;
}

// The inputs at another chart point for a field's lookup (`country_field` reads the chart point alone).
fn moved(inputs: SurfaceInputs, at: vec2<f32>) -> SurfaceInputs {
    var out = inputs;
    out.chart = at;
    return out;
}

// Stands a country tree `tree` at `at` (`key` its hash) by `chance`, off roads and clearings, on land, its size by its
// rank against the level of detail's limit (as the slots' and the measured parts').
fn country_plant(at: vec2<f32>, tree: Choice, key: vec2<f32>, chance: f32, part_rank: f32, limit: f32, grown: f32,
    open: f32) {
    if tree.species < 0.0 || part_rank >= limit {
        return;
    }
    let at_ground = height_at(at);
    if at_ground < 1.5 || cell_hash(key, 4.0) >= chance * (1.0 - cleared(at)) * (1.0 - feature_clearance(at)) {
        return;
    }
    let size = grown * ramp(0.0, SHRINK, (limit - part_rank) / limit);
    let tree_height = tree.height * (1.0 + (cell_hash(key, 5.0) - 0.5) * 0.5 * tree.variety) * size;
    if tree_height < 0.5 {
        return;
    }
    let width = (cell_hash(key, 6.0) * 0.3 + 0.85) * tree.width;
    let gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(at), 0.0).xy;
    plant(at, at_ground, gradient, tree_height, width, tree, open, key);
}

// A slot on the period's country outside a woodlot (`field` its field at the slot's foot `foot`; the slot `cell` from
// `lo`, `spacing` a side; `rank` and `share` the slot's and the level of detail's, `apparent` its distance).
fn country_slot(field: CountryField, cell: vec2<f32>, lo: vec2<f32>, spacing: f32, foot: vec2<f32>,
    inputs: SurfaceInputs, pick: f32, rank: f32, share: f32, apparent: f32) {
    let region = country_region();
    let grown = clamp(apparent / view.lod.x, 1.0, view.lod.z);
    let in_leaf = fract(pick * 7.13) < terrain.climate.z;
    let open = inputs.sky;
    if field.kind == FIELD_ORCHARD || field.kind == FIELD_OLIVES || field.kind == FIELD_PALMS {
        // The rows on the land's painted crowns: a lattice of the slot's spacing over a whole number, on the field's
        // heading, from the chart's origin; every point of it in the slot that lies in the field and off its edge.
        let o = orchard_trees(region, field, in_leaf);
        if o.tree.species < 0.0 {
            return;
        }
        let k = max(round(spacing / o.desired), 1.0);
        let sp = spacing / k;
        let along = field.along;
        let across = vec2<f32>(-along.y, along.x);
        var q_lo = vec2<f32>(1e9);
        var q_hi = vec2<f32>(-1e9);
        for (var c = 0; c < 4; c += 1) {
            let corner = lo + vec2<f32>(f32(c & 1), f32(c >> 1u)) * spacing;
            let q = vec2<f32>(dot(corner, along), dot(corner, across)) / sp;
            q_lo = min(q_lo, q);
            q_hi = max(q_hi, q);
        }
        let first = ceil(q_lo);
        let count = min(vec2<i32>(floor(q_hi) - first) + 1, vec2<i32>(16));
        let near = pow(ORCHARD_NEAR_M / max(apparent, 1.0), 2.0);
        // Off the field's edge: a track's or a hedge's half width and a crown's.
        let margin = 4.0;
        for (var j = 0; j < count.y; j += 1) {
            for (var i = 0; i < count.x; i += 1) {
                let g = first + vec2<f32>(f32(i), f32(j));
                let at = (along * g.x + across * g.y) * sp;
                if any(at < lo) || any(at >= lo + spacing) {
                    continue;
                }
                let key = g + vec2<f32>(floor(field.id * 7919.0), 0.0);
                let coarse = g.x - k * floor(g.x / k) < 0.5 && g.y - k * floor(g.y / k) < 0.5;
                let far = coarse && o.far;
                let part_rank = select(max(rank, cell_hash(key, 11.0)), rank, far);
                let limit = select(min(near, share), share, far);
                if part_rank >= limit {
                    continue;
                }
                // Within the foot's distance of its field's edge the point is in the field; beyond it, looked up.
                if distance(at, foot) > field.edge - margin {
                    let there = country_field(moved(inputs, at));
                    if there.id != field.id || there.edge < margin {
                        continue;
                    }
                }
                let jitter = (vec2<f32>(cell_hash(key, 1.0), cell_hash(key, 2.0)) - 0.5) * 0.6;
                country_plant(at + jitter, o.tree, key, 0.97, part_rank, limit, grown, open);
            }
        }
        return;
    }
    // Hedgerow trees and windbreaks: a slot the edge passes near stands its tree on it (the edge's distance rises away
    // from it either side: the foot moved down its slope by the distance).
    let e = edge_trees(region, field, pick, in_leaf);
    if e.chance > 0.0 && field.edge < spacing * 0.5 && field.kind != FIELD_YARD {
        let dx = country_field(moved(inputs, foot + vec2<f32>(1.0, 0.0))).edge - field.edge;
        let dz = country_field(moved(inputs, foot + vec2<f32>(0.0, 1.0))).edge - field.edge;
        let slope = vec2<f32>(dx, dz);
        if length(slope) > 0.5 {
            let out = slope / length(slope);
            let along = vec2<f32>(-out.y, out.x);
            let on = foot - out * (field.edge - e.aside);
            for (var t = 0; t < e.count; t += 1) {
                let key = cell * 4.0 + vec2<f32>(f32(t), 0.0);
                let at = on + along * ((f32(t) - 0.5 * f32(e.count - 1)) * spacing / f32(e.count));
                let part_rank = select(max(rank, cell_hash(key, 11.0)), rank, t == 0);
                country_plant(at, e.tree, key, e.chance, part_rank, share, grown, open);
            }
            return;
        }
    }
    if field.kind == FIELD_YARD {
        return;
    }
    let h = home_trees(region, field, pick, in_leaf, spruce_native(latitude_at(foot), longitude_at(foot)));
    if fract(field.id * 11.3) < h.farms && field.home < FARMSTEAD_M {
        country_plant(foot, h.tree, cell, 0.6, rank, share, grown, open);
        return;
    }
    country_plant(foot, h.lone_tree, cell, h.lone, rank, share, grown, open);
}

@compute @workgroup_size(16, 16)
fn place(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) local: vec3<u32>) {
    // The drawn patches with trees first, then the shadow-only ones (after every drawn patch).
    let drawn = u32(view.shadow.y);
    let corner = patches[select(group.x, u32(view.shadow.x) + group.x - drawn, group.x >= drawn)];
    let level = u32(corner.z);
    let spacing = terrain.field.w;
    // The slot's cell on the chart, stable wherever the patches fall: a level-0 patch has a slot a cell, a level-1
    // patch a slot for every 2 × 2 cells, standing its tree in the cell of the four that ranks lowest.
    let first = vec2<i32>(round((corner.xy - chart_origin()) / (spacing * exp2(corner.z))));
    let cell = vec2<f32>(survivor(first + vec2<i32>(local.xy), level));
    let jitter = vec2<f32>(cell_hash(cell, 1.0), cell_hash(cell, 2.0));
    var foot = chart_origin() + (cell + jitter) * spacing;
    var ground = height_at(foot);
    if ground < 1.5 {
        return;
    }

    // The level of detail: how many of the slots about here keep their tree at this apparent distance (no more than
    // the patch's slots hold).
    let toward = view.eye.xyz - vec3<f32>(foot.x, ground, foot.y);
    let reach = length(toward);
    let apparent = reach * view.eye.w;
    var share = pow(view.lod.x / max(apparent, 1.0), 2.0) * (1.0 - ramp(view.lod.y * 0.7, view.lod.y, reach));
    // A level-1 patch holds the slots ranking below 1/4, so the share comes down to 1/4 by the end of level 0, where
    // level-1 patches begin: no tree is lost as a patch changes level.
    share = min(share, mix(2.0, 0.25, ramp(view.lod.w * 0.7, view.lod.w, reach)));
    share = min(share, select(1e6, 0.2501, level > 0u));
    let rank = rank_of(cell);
    if rank >= share || !(in_view(vec3<f32>(foot.x, ground + view.cover.z, foot.y), view.cover.w) || in_maps(foot, ground)) {
        return;
    }
    var gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(foot), 0.0).xy;
    // Seen at a grazing angle the rows of trees stand one behind another on screen, so fewer make the same wall of
    // crowns: past two kilometres (apparent) the share falls with the sine of the view over the ground, to a third.
    let ground_normal = normalize(vec3<f32>(-gradient.x, 1.0, -gradient.y));
    let grazing = clamp(abs(dot(ground_normal, toward / max(reach, 1e-3))) / 0.3, 0.35, 1.0);
    share *= mix(1.0, grazing, ramp(1500.0, 3000.0, apparent));
    if rank >= share {
        return;
    }

    // The canopy height map, where its pages cover the cell: its parts' crowns (the cell itself, but in a slot wider
    // than CANOPY_PART_M), the slot's own tree standing on the tallest.
    let lo = chart_origin() + cell * spacing;
    let parts = clamp(i32(ceil(spacing / CANOPY_PART_M - 0.05)), 1, CANOPY_PARTS);
    let part_m = spacing / f32(parts);
    var crowns: array<Crown, 9>;
    var crown = Crown(canopy_known(lo + spacing * 0.5), lo + spacing * 0.5, 0.0, 0.0);
    var slot_cover = 0.0;
    if crown.known {
        for (var k = 0; k < parts * parts; k += 1) {
            crowns[k] = canopy_crown(lo + vec2<f32>(f32(k % parts), f32(k / parts)) * part_m, part_m);
            slot_cover += crowns[k].cover / f32(parts * parts);
            if crowns[k].height > crown.height {
                crown = crowns[k];
            }
        }
    }
    if crown.height > 0.0 {
        foot = crown.apex;
        ground = height_at(foot);
        if ground < 1.5 {
            return;
        }
        gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(foot), 0.0).xy;
    }
    let inputs = inputs_at(foot, ground, gradient);
    let pick = cell_hash(cell, 3.0);
    let cover = tree_cover(inputs, pick);
    var choice: Choice;
    // 0 where the land cover stands no wood, whatever the choice's density says (scrub a measured crown stands a
    // woodland tree on).
    var land_density = 1.0;
    let biome = biome_at(foot, cell);
    // The period's country's share of the foot, and the cover and ground there (`country_slot`).
    var country = 0.0;
    var country_cover: Cover;
    var country_ground: Ground;
    if biome == BIOME_AUTHORED {
        choice.species = authored(cover, pick);
        choice.height = cover.height;
        choice.tint = cover.tint;
        choice.density = cover.density;
        choice.width = 1.0;
        choice.variety = 1.0;
        choice.rows = -1.0;
    } else {
        let g = ground_of(inputs);
        var c = cover_at(inputs);
        choice = choose(biome, inputs, g, cover, c, pick);
        // A tree the canopy map measures where the land cover has scrub or grass: the biome's tree of the woods there
        // (a tree of its own, the scrub no wood).
        if crown.height >= CANOPY_TREE_M && shrubby(choice.species) {
            land_density = 0.0;
            c.shares.a = vec4<f32>(c.shares.a.x + c.shares.a.z + c.shares.a.w, c.shares.a.y, 0.0, 0.0);
            choice = choose(biome, inputs, g, cover, c, pick);
        }
        // At a wild wood's edge (open land within `EDGE_M`) its trees reach out to the light: they lean out over the
        // open ground, their crowns wider that way, and stand a little lower.
        if choice.wild > 0.0 {
            let about = woods_about(foot);
            if about.x < 0.99 && dot(about.yz, about.yz) > 0.0 {
                let edge = 1.0 - about.x;
                choice.lean = normalize(about.yz) * (0.045 + 0.05 * edge) * choice.wild;
                choice.height *= 1.0 - 0.15 * edge;
            }
        }
        country = c.shares.past * c.weight;
        country_cover = c;
        country_ground = g;
    }
    // On the period's country its own trees, today's measured crowns standing none (`country_slot`); a woodlot's or a
    // grove's the biome's wood, whatever the canopy map says of today's ground.
    var woodlot = false;
    if country > 0.5 {
        let field = country_field(inputs);
        if field.kind != FIELD_WOOD {
            country_slot(field, cell, lo, spacing, foot, inputs, pick, rank, share, apparent);
            return;
        }
        woodlot = true;
        var wood = country_cover;
        wood.shares.a = vec4<f32>(1.0, 0.0, 0.0, 0.0);
        wood.shares.b = vec4<f32>(0.0);
        wood.shares.c = vec4<f32>(0.0);
        wood.shares.past = 0.0;
        wood.weight = 1.0;
        choice = choose(biome, inputs, country_ground, cover, wood, pick);
        choice.density = max(choice.density, 0.85 * (1.0 - feature_clearance(foot)));
    }
    // Where the canopy map covers the cell (but for scrub, the land cover's: the map's crowns start at 3 m), a wood the
    // land cover stands keeps its trees, and outside a wood the map's crowns stand trees of their own (a town's
    // gardens and streets, hedgerows, a wood's edge); in either a bare patch stands none. Elsewhere the land cover's
    // density alone. No tree stands in a clearing (a theatre airfield's apron), nor a measured one on a road, railway
    // or river (the land cover's density keeps off them already).
    let mapped = crown.known && !shrubby(choice.species) && !woodlot;
    let measured = mapped && crown.height > 0.0 && min(land_density, choice.density) < CANOPY_WOOD;
    // A clearing reaches half a slot further for a tree: one grown to fill a wide slot would spread its crown into it.
    let chance = select(choice.density, 1.0 - feature_clearance(foot), measured)
        * (1.0 - cleared_within(foot, 0.5 * spacing));
    // (The bare patch read only for a tree that would stand.)
    if cell_hash(cell, 4.0) >= chance || (mapped && crown.height <= 0.0 && canopy_bare(lo, spacing, slot_cover)) {
        return;
    }
    var at = foot;
    var at_ground = ground;
    if choice.rows >= 0.0 && !measured {
        // A plantation's rows: the tree stands on the plantation's grid, a point a cell turned by its angle, or none
        // where the turned grid leaves the cell without one (a gap in a row).
        let along = vec2<f32>(cos(choice.rows), sin(choice.rows));
        let across = vec2<f32>(-along.y, along.x);
        let centre = chart_origin() + (cell + 0.5) * spacing;
        let grid = round(vec2<f32>(dot(centre, along), dot(centre, across)) / spacing);
        at = (along * grid.x + across * grid.y) * spacing + (jitter - 0.5) * 0.6;
        if any(floor((at - chart_origin()) / spacing) != cell) {
            return;
        }
        at_ground = height_at(at);
        if at_ground < 1.5 || cell_hash(cell, 4.0) >= (1.0 - cleared_within(at, 0.5 * spacing)) * (1.0 - feature_clearance(at)) {
            return;
        }
    }
    // Grown with distance as the forest thins, shrinking away over the last of the tree's own range.
    let grown = clamp(apparent / view.lod.x, 1.0, view.lod.z);
    if measured {
        // Each part's tallest crown stands a tree at its measured height, as wide as the part's canopy (one crown
        // standing for every crown in it), of the slot's species; the parts beside the slot's own tree thinned with
        // distance as the slots are.
        for (var k = 0; k < parts * parts; k += 1) {
            let part_crown = crowns[k];
            if part_crown.height <= 0.0 {
                continue;
            }
            let key = cell * f32(parts) + vec2<f32>(f32(k % parts), f32(k / parts));
            let own = all(part_crown.apex == crown.apex);
            let part_rank = select(max(rank, cell_hash(key, 11.0)), rank, own);
            let part_ground = height_at(part_crown.apex);
            let standing = (1.0 - cleared(part_crown.apex)) * (1.0 - feature_clearance(part_crown.apex));
            // The slot's own crown was let stand by the clearing at the slot's foot; a wide slot's crown may lie in
            // a clearing its foot is not in (a generated base's edge), so it is held to the clearing where it stands.
            let in_clearing = own && cell_hash(key, 4.0) >= 1.0 - cleared_within(part_crown.apex, 0.5 * part_m);
            if part_rank >= share || part_ground < 1.5 || (!own && cell_hash(key, 4.0) >= standing) || in_clearing {
                continue;
            }
            let part_size = grown * ramp(0.0, SHRINK, (share - part_rank) / share);
            let tree_height = part_crown.height * (1.0 + (cell_hash(key, 5.0) - 0.5) * 0.1) * part_size;
            let spread = CANOPY_DRAWN * (1.0 + CANOPY_OVERLAP * part_crown.cover);
            let canopy_radius = max(part_m * sqrt(part_crown.cover / 3.14159265) * spread, 1.5);
            let crown_ratio = view.species[u32(choice.species)].x;
            let stand = choice.species == SPECIES_SUGI || choice.species == SPECIES_BAMBOO;
            let broad = select(ramp(0.3, 0.5, crown_ratio), 0.0, stand);
            let widest = min(mix(CANOPY_WIDEST.x, CANOPY_WIDEST.y, broad), CANOPY_ASPECT / crown_ratio);
            let width = clamp(canopy_radius / (crown_ratio * part_crown.height), 0.6, widest);
            if tree_height >= 0.5 {
                let part_gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(part_crown.apex), 0.0).xy;
                plant(part_crown.apex, part_ground, part_gradient, tree_height, width, choice, inputs.sky, key);
            }
        }
        return;
    }
    let size = grown * ramp(0.0, SHRINK, (share - rank) / share);
    var tree_height = choice.height * (1.0 + (cell_hash(cell, 5.0) - 0.5) * 0.5 * choice.variety) * size;
    // A wild wood's younger trees, half to three quarters of its canopy's height (a measured crown stands as measured).
    if cell_hash(cell, 16.0) < YOUNG * choice.wild {
        tree_height *= 0.5 + 0.25 * cell_hash(cell, 17.0);
    }
    if mapped && crown.height > 0.0 {
        // A wood's tree, on the map's crown: as tall as the crown where that stands taller than the biome's tree.
        tree_height = max(tree_height, crown.height * (1.0 + (cell_hash(cell, 5.0) - 0.5) * 0.1) * size);
    }
    let width = (cell_hash(cell, 6.0) * 0.3 + 0.85) * choice.width;
    if tree_height < 0.5 {
        return;
    }
    plant(at, at_ground, gradient, tree_height, width, choice, inputs.sky, cell);
}

// A wood's sapling at its edge, by the biome: a young tree of the wood's commonest kind (-1 for none: a shrub); `spruce`
// how much of the boreal spruce is native there (`spruce_native`: past its limit a young pine in its place).
fn sapling(biome: f32, u: f32, in_leaf: bool, spruce: f32) -> f32 {
    if biome == BIOME_BRITAIN || biome == BIOME_NORTHERN_EUROPE || biome == BIOME_TEMPERATE || biome == BIOME_NORTHEAST {
        return seasonal(select(SPECIES_ASH, SPECIES_BIRCH, u < 0.4), in_leaf);
    }
    if biome == BIOME_BOREAL || biome == BIOME_SUBARCTIC {
        return select(select(seasonal(SPECIES_BIRCH, in_leaf), SPECIES_SCOTS_PINE, u < 0.4 - 0.25 * spruce),
            SPECIES_SPRUCE, u < 0.4 * spruce);
    }
    if biome == BIOME_MEDITERRANEAN || biome == BIOME_NORTH_AFRICA || biome == BIOME_CALIFORNIA || biome == BIOME_SUBTROPICAL {
        return SPECIES_LIVE_OAK;
    }
    if biome == BIOME_SOUTHEAST {
        return select(SPECIES_LIVE_OAK, SPECIES_SCOTS_PINE, u < 0.5);
    }
    if biome == BIOME_JAPAN {
        return SPECIES_LAUREL;
    }
    if biome == BIOME_TROPICAL || biome == BIOME_HAWAII {
        return SPECIES_RAINFOREST;
    }
    if biome == BIOME_INTERIOR_WEST {
        // A young ponderosa or pinyon.
        return select(-1.0, SPECIES_SCOTS_PINE, u < 0.5);
    }
    return -1.0;
}

// The ground cover about the camera, one thread a cell of `view.cover.y` metres over a square `view.cover.x` cells a
// side from cell `view.biome.zw`: shrubs where the land cover is shrubland (chaparral, maquis and garrigue by the
// biome), a few under the woods, a mantle of shrubs and saplings along a wood's edge, tufts of grass on dry or tropical
// grassland near the camera, none on fields, towns, bare ground, snow, water or roads; where the canopy map covers the
// cell, the open land's shrubs where it measures growth under a tree's height (a hedge, a thicket), none on a mown
// field; the period's country's hedgerows along its fields' edges; on a battle map from its style (the tropics' kunai,
// the snow style's heath, the chalk downs). Thinned with distance as the trees are, each shrinking away toward the reach.
@compute @workgroup_size(8, 8)
fn place_cover(@builtin(global_invocation_id) id: vec3<u32>) {
    let cells = u32(view.cover.x);
    if any(id.xy >= vec2<u32>(cells)) {
        return;
    }
    let size = view.cover.y;
    let cell = view.biome.zw + vec2<f32>(id.xy);
    // Salted apart from the tree cells.
    let key = cell + vec2<f32>(7919.0, 104729.0);
    let foot = chart_origin() + (cell + vec2<f32>(cell_hash(key, 1.0), cell_hash(key, 2.0))) * size;
    let ground = height_at(foot);
    if ground < 2.0 {
        return;
    }
    let toward = view.eye.xyz - vec3<f32>(foot.x, ground, foot.y);
    let reach = length(toward);
    let range = view.biome.y;
    // Wide enough for a mantle's sapling (6 m) and a bush moved onto its hedge's line (up to 0.75 of a cell).
    if reach > range || !in_view(vec3<f32>(foot.x, ground + 3.0, foot.y), 3.5 + size * 0.75) {
        return;
    }
    let full = range * COVER_FULL;
    let share = min(pow(full / max(reach, 1.0), 2.0), 1.0) * (1.0 - ramp(range * 0.75, range, reach));
    let rank = rank_of(key);
    if rank >= share {
        return;
    }
    let gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(foot), 0.0).xy;
    if length(gradient) > 0.9 {
        return;
    }
    let inputs = inputs_at(foot, ground, gradient);
    let g = ground_of(inputs);
    // None on snow, a beach, a clearing, or a road, railway or river (`features.wgsl`).
    if climate_snow(inputs, g) > 0.5 || g.beach > 0.5 || cleared(foot) > 0.0 || feature_clearance(foot) > 0.3 {
        return;
    }
    let pick = cell_hash(key, 3.0);
    let wooded = tree_cover(inputs, pick).density;
    let c = cover_at(inputs);
    let biome = biome_at(foot, key);
    let arid = biome == BIOME_ARID || biome == BIOME_EGYPT;
    let dry_summer = biome == BIOME_CALIFORNIA || biome == BIOME_MEDITERRANEAN || biome == BIOME_NORTH_AFRICA;
    let dry = max(terrain.climate.w, select(0.0, 1.0, arid || dry_summer));
    let tropical = biome == BIOME_TROPICAL || biome == BIOME_HAWAII;
    let heath = biome == BIOME_BRITAIN || biome == BIOME_NORTHERN_EUROPE;
    let west = biome == BIOME_INTERIOR_WEST;
    // The style's own ground on a battle map (or where the maps know nothing): shrubs under and about the woods.
    var shrub = wooded * 0.1;
    var grass = 0.0;
    var garrigue = 0.0;
    // A wood's edge's share of the shrubs (its saplings among them), and a hedge's bush moved onto its line.
    var mantle = 0.0;
    var hedge = false;
    var at = foot;
    var at_ground = ground;
#ifdef STYLE_TROPICAL
    grass = (1.0 - wooded) * 0.4;
#endif
#ifdef STYLE_VOLCANIC
    grass = (1.0 - wooded) * 0.25;
#endif
#ifdef STYLE_SNOW
    // Heath: crowberry, juniper and dwarf birch cushions.
    garrigue = (1.0 - wooded) * 0.3;
    grass = (1.0 - wooded) * 0.08;
#endif
    if c.weight > 0.0 {
        let s = c.shares;
        let tufts = select(0.08, 0.45, tropical || dry > 0.5 || west);
        // The open land's scattered shrubs, or where the canopy map covers the cell those it measures: growth 2 m or
        // taller over a share of its texel with no crown (3 m and taller) in it or the texels about it, so a hedge or a
        // thicket and not a tree's shade (a bush can stand on a building's texel in a town: none there).
        var open_shrubs = s.a.w * 0.03 + s.c.y * 0.05;
        if canopy_known(foot) {
            let t = vec2<i32>(floor((foot - view.canopy.xy) / view.canopy.z));
            var crowns = 0.0;
            for (var j = -1; j <= 1; j += 1) {
                for (var i = -1; i <= 1; i += 1) {
                    crowns = max(crowns, canopy_texel(t + vec2<i32>(i, j)).r);
                }
            }
            let growth = select(0.0, ramp(0.12, 0.45, canopy_texel(t).b), round(crowns * 255.0) < 3.0);
            open_shrubs = growth * 0.75 * (1.0 - s.a.x) * (1.0 - s.b.y);
        }
        // A wood's mantle: shrubs and saplings out over the open ground along its edge.
        let about = woods_about(foot);
        mantle = about.x * (1.0 - s.a.x) * (1.0 - s.b.y) * select(1.0, 0.0, arid);
        // The period's country: a field's middle, ploughed, sown or grazed, stands none of the shrubs today's canopy
        // measures there, nor a wood's mantle. Britain's hedgerows and the East's fencerows (the land paints them,
        // `surface.wgsl`'s `country_ground`) stand as bushes along its fields' edges, each cell near one moving its bush
        // onto the edge's line (the edge's distance rises away from it either side), so the hedge closes.
        if s.past * c.weight > 0.5 {
            let field = country_field(inputs);
            if field.kind != FIELD_WOOD && field.kind != FIELD_YARD {
                open_shrubs = 0.0;
                mantle = 0.0;
                let region = country_region();
                if (region == COUNTRY_BRITAIN || region == COUNTRY_EAST) && field.edge < size * 0.75 {
                    let dx = country_field(moved(inputs, foot + vec2<f32>(1.0, 0.0))).edge - field.edge;
                    let dz = country_field(moved(inputs, foot + vec2<f32>(0.0, 1.0))).edge - field.edge;
                    let slope = vec2<f32>(dx, dz);
                    if length(slope) > 0.5 {
                        at = foot - slope / length(slope) * field.edge;
                        at_ground = height_at(at);
                        hedge = at_ground >= 2.0 && cleared(at) <= 0.0 && feature_clearance(at) <= 0.3;
                        if !hedge {
                            return;
                        }
                    }
                }
            }
        }
        var mapped_shrub = s.a.z * 0.55 + s.a.x * 0.1 + open_shrubs + mantle * MANTLE;
        var mapped_garrigue = 0.0;
        if arid || (dry_summer && biome != BIOME_CALIFORNIA) {
            // Garrigue on the dry, stony ground among the maquis.
            mapped_garrigue = s.a.z * 0.3 + s.b.z * 0.1;
            mapped_shrub *= 0.75;
        }
        if biome == BIOME_SUBARCTIC || biome == BIOME_BOREAL {
            mapped_garrigue = (s.a.z + s.a.w) * 0.25;
        }
        if west {
            // Sagebrush over the shrubland, the grassland and the stony ground, to the spruce-fir's belt.
            mapped_garrigue = (s.a.z * 0.35 + s.a.w * 0.15 + s.b.z * 0.08) * (1.0 - ramp(2700.0, 3100.0, ground));
            mapped_shrub *= 0.8;
        }
        if heath {
            // Heather on the heaths and the moors (the uplands' rough grazing), gorse and broom among it.
            mapped_garrigue = s.a.z * 0.4 + s.a.w * 0.12 * ramp(150.0, 300.0, ground);
            mapped_shrub *= 0.8;
        }
        if biome == BIOME_SOUTHEAST {
            // Saw palmetto and gallberry under the pines.
            mapped_shrub += s.a.x * 0.12;
        }
        if biome == BIOME_HAWAII {
            // Haole koa on the dry grassland.
            mapped_shrub += s.a.w * 0.08;
        }
        if biome == BIOME_CALIFORNIA {
            // Chaparral: a closed cover of shrubs.
            mapped_shrub += s.a.z * 0.3;
        }
        shrub = mix(shrub, mapped_shrub, c.weight);
        garrigue = mix(garrigue, mapped_garrigue, c.weight);
        grass = mix(grass, s.a.w * tufts, c.weight);
        mantle *= c.weight;
    }
    grass *= 1.0 - ramp(GRASS_REACH_M * 0.75, GRASS_REACH_M, reach);
    let roll = cell_hash(key, 4.0);
    var species = SPECIES_SHRUB;
    var scale = cell_hash(key, 5.0) * 0.7 + 0.6;
    var width = cell_hash(key, 6.0) * 0.4 + 0.8;
    let in_leaf = fract(pick * 7.13) < terrain.climate.z;
    if hedge {
        // Hawthorn, blackthorn and hazel, laid and trimmed: about 2-3 m in Britain, lower in a fencerow.
        species = SPECIES_SHRUB;
        scale = select(0.6, 0.8, country_region() == COUNTRY_BRITAIN) + 0.35 * cell_hash(key, 5.0);
        width = 1.1 + 0.3 * cell_hash(key, 6.0);
    } else if roll < shrub {
        species = SPECIES_SHRUB;
        // Among a wood's mantle its saplings: a young tree of the wood's kind, 3-6 m.
        let young = sapling(biome, cell_hash(key, 9.0), in_leaf,
            spruce_native(latitude_at(foot), longitude_at(foot)));
        if young >= 0.0 && cell_hash(key, 11.0) < SAPLINGS * MANTLE * mantle / max(shrub, 1e-3) {
            species = young;
            scale = (3.0 + 3.0 * cell_hash(key, 5.0)) / species_height(young);
            width = 0.85;
        }
    } else if roll < shrub + garrigue {
        species = SPECIES_GARRIGUE;
    } else if roll < shrub + garrigue + grass {
        species = SPECIES_GRASS;
    } else {
        return;
    }
    let shrink = ramp(0.0, SHRINK, (share - rank) / share);
    let tree_height = species_height(species) * scale * shrink;
    if tree_height < 0.15 {
        return;
    }
    let radius = view.species[u32(species)].x * tree_height * width;
    let base = at_ground - length(gradient) * radius * 0.5 - 0.1;
    let brightness = cell_hash(key, 8.0) * 0.4 + 0.8;
    let hue = cell_hash(key, 10.0) * 2.0 - 1.0;
    var tint = brightness * vec3<f32>(1.0 + 0.12 * hue, 1.0, 1.0 - 0.18 * hue);
    // Chaparral's dark olive, gorse's dark green, saw palmetto's yellow-green; heather's purple-brown; dry grass in a
    // dry summer.
    if species == SPECIES_SHRUB && biome == BIOME_CALIFORNIA {
        tint *= vec3<f32>(0.8, 0.88, 0.76);
    }
    if species == SPECIES_SHRUB && heath {
        tint *= vec3<f32>(0.8, 0.9, 0.7);
    }
    if species == SPECIES_SHRUB && biome == BIOME_SOUTHEAST {
        tint *= vec3<f32>(1.0, 1.06, 0.78);
    }
    if species == SPECIES_GARRIGUE && heath {
        tint *= vec3<f32>(0.9, 0.72, 0.78);
    }
    // Sagebrush's silver grey-green; blackbrush, cliffrose and rabbitbrush grey-olive.
    if species == SPECIES_GARRIGUE && west {
        tint *= vec3<f32>(1.1, 1.12, 1.04);
    }
    if species == SPECIES_SHRUB && west {
        tint *= vec3<f32>(0.9, 0.92, 0.82);
    }
    if species == SPECIES_GRASS {
        tint *= mix(vec3<f32>(1.0), vec3<f32>(1.25, 1.05, 0.7), max(dry * select(terrain.climate.w, 1.0, arid), 0.0));
    }
    if hedge {
        // Dark in leaf; out of leaf a hedge's twigs, brown-grey.
        tint *= select(vec3<f32>(0.95, 0.78, 0.62), vec3<f32>(0.82, 0.95, 0.74), in_leaf);
    }
    // A bush wider one way than the other; a sapling leaning as a tree's own may.
    let yaw = cell_hash(key, 7.0) * 6.2831853;
    let heading = cell_hash(key, 13.0) * 6.2831853;
    let lean = select(vec2<f32>(0.0), vec2<f32>(cos(heading), sin(heading)) * 0.04 * cell_hash(key, 12.0), !shrubby(species));
    let form = packed_form(yaw, lean, cell_hash(key, 14.0) * 0.8, false);
    append(at, base, tree_height, species, form, inputs.sky, tint, width);
}
