// The terrain's bindings at group 1, shared by the land, its trees and its shadow casters.
//
// Ports the uniforms and textures of the web game's `terrain/TerrainMaterial.ts` (`TerrainGrids`, `TerrainMaps`) and the
// scene values `BattleLandscape.ts` and `VisualEnvironment.ts` hand it (the sun, the hemisphere fill, the scene fog, and
// the scene's point lights three hands every lit material).
#define_import_path naval_terrain::bindings


struct Terrain {
    // Chart origin of sample (0, 0) (x, z), field cell (m), level-0 vertex spacing (m).
    field: vec4<f32>,
    // Field columns and rows, attribute-grid columns and rows.
    grid: vec4<f32>,
    // Attribute-grid cell (m), level-0 range (m), time (s), 1 for the detailed surface.
    params: vec4<f32>,
    // World offset of the chart (x, z): world = chart + offset; the trees' range (m); the debug view (0 draws the land).
    offset: vec4<f32>,
    // Unit direction toward the sun (or moon); w: the land style (`STYLE_*`).
    sun_direction: vec4<f32>,
    // The directional light on meshes: colour × intensity × the meshes' share; w: environment intensity.
    sun_radiance: vec4<f32>,
    // The hemisphere fill's sky and ground colours × its intensity; w: 1 where the trees cast into the sun's near map
    // (`fill_sky`) and its wide map (`fill_ground`) this frame (`naval_terrain::forest`).
    fill_sky: vec4<f32>,
    fill_ground: vec4<f32>,
    // Scene fog: colour (linear) and power; start, end, sky blend (m), 1 while on.
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    // The style's palette (linear albedo): shore, lowland, highland; shore's w: 1 where the trees cast into the sun's
    // land maps this frame, lowland's into its aboard map (`naval_terrain::forest`).
    shore: vec4<f32>,
    low: vec4<f32>,
    high: vec4<f32>,
    // Chart direction toward the equator (x, z); z: 1 while the clouds' shadow is bound; w: lightning's flash.
    equator: vec4<f32>,
    // A clearing (a theatre airfield's apron, `TerrainLayer::set_clearing`): chart centre (x, z) and radius (m); w: the
    // width of its feathered edge (m), 0 for none.
    clearing: vec4<f32>,
    // The season and climate (`Climate`): the snowline (m), the thaw below it, the leaves out, a dry summer (0-1 each).
    climate: vec4<f32>,
    // Real-world surface maps (`TerrainLayer::set_surface_maps`): their chart origin (x, z), side (m) and texels a side;
    // all 0 with none bound.
    surface: vec4<f32>,
    // The haze to take out of the maps' colour (linear; dark-object subtraction); w: 1 while real buildings stand on
    // the land (`TerrainLayer::set_buildings_stand`: the built-up class paints no lots of its own). Then each cover
    // class's mean colour in the window with the haze out (w 1 where the window has the class): unknown, 10-100 by
    // tens, mangroves (95) last.
    surface_haze: vec4<f32>,
    cover_means: array<vec4<f32>, 12>,
    // The near tiles (`TerrainLayer::set_near_grid`): the chart corner of tile (0, 0) (x, z), a tile's side (m), and
    // the tiles a side the layer holds (0 with none); then which tile each slot holds, two slots an entry (slot k at
    // entry k / 2, .xy when k is even, .zw when odd; slot (i, j) is k = j * slots + i), i32's least where none.
    near: vec4<f32>,
    near_slots: array<vec4<i32>, 32>,
    // Map features on the ground (`TerrainLayer::set_feature_grid`, `features.wgsl`): level 0's and level 1's texel (m),
    // the slots a side, 1 while bound; the chart corner of page (0, 0) of level 0 (xy) and of level 1 (zw); which page each
    // slot holds, two slots an entry (level l's slot k at entry l * 32 + k / 2, .xy when k is even, .zw when odd).
    features: vec4<f32>,
    feature_origin: vec4<f32>,
    feature_slots: array<vec4<i32>, 64>,
    // Ground materials (`TerrainLayer::set_ground_materials`, `materials.wgsl`): x 1 while bound and shown, y their
    // strength, z and w the pixel footprint (m) over which they fade out; then per material (`GROUND_MATERIALS`) its
    // period (m), its normal's strength, its colour's contrast, and w spare.
    materials: vec4<f32>,
    material_table: array<vec4<f32>, 16>,
    // The cliffs' rock (`ground_materials::CliffRock`, `materials.wgsl`'s `cliff_rock`): the grid about the eye (its
    // first cell's chart corner, a cell (m), 1 with a place; then four bits a cell, cell k = j * 32 + i in
    // `cliff_grid[k / 32][(k / 8) % 4]` at bits 4 (k mod 8)), and per rock three rows: its material (layer, period,
    // normal strength, contrast), its colour (w 1 where it stands in for the photograph's and the style's), its bedding
    // (strength, beds' thickness in m).
    cliff_corner: vec4<f32>,
    cliff_grid: array<vec4<u32>, 32>,
    cliff_rocks: array<vec4<f32>, 30>,
    // The trees near the camera drawn as meshes (`near_trees.wgsl`, `naval_terrain::foliage::near_shares`): the eye
    // (world), and w the largest tree's reach (m; 0 with none, every tree an impostor). Then the reach per metre of a
    // tree's height, the band over which its mesh gives way to its impostor (a share of its reach), the share of the
    // reach drawn at full detail, and the band over which the full detail gives way to the reduced (a share of that).
    tree_near: vec4<f32>,
    tree_lod: vec4<f32>,
    // The wind in the trees (`naval_terrain::foliage::tree_sway`): the way the air moves (world x, z; unit), its speed
    // (m/s), the scene's clock (s; it stands while the picture is held).
    wind: vec4<f32>,
    // The lights on the land at night (`naval_terrain::night`, `night.wgsl`, `lamps.wgsl`): x how dark it is for them
    // (0 by day, 1 at night), y the street lights' strength (the scenario's rule: 1 lit), z the built-up land's glow's,
    // w the floods'; then the beacons' strength, the share of lamps leaking where the rule puts them out, 1 while the
    // street lights are shielded (seen from above only), and the palette (0 sodium, 1 Japan's white).
    night: vec4<f32>,
    night_rule: vec4<f32>,
    // xy: the theatre point at chart (0, 0) modulo the street lights' lattice (`night::LATTICE_M`).
    night_grid: vec4<f32>,
    // Fine relief near the camera (`TerrainLayer::set_relief_grid`, `naval_terrain::relief`): the chart corner of page
    // (0, 0)'s own texels (x, z), a texel (m), the slots a side (0 with none, or hidden); then which page each layer
    // holds, two layers an entry (layer k at entry k / 2, .xy when k is even, .zw when odd; layer (i, j) is
    // k = j * slots + i), i32's least where none.
    relief: vec4<f32>,
    relief_slots: array<vec4<i32>, 32>,
    // Which layers hold a real page (a finer model's, `ReliefPage::real`; not a synthetic or GLO-30's): layer k at bit
    // k mod 32 of component k / 32.
    relief_real: vec4<u32>,
    // The period's country (`TerrainLayer::set_country`, `surface.wgsl`'s "The period's country"): the region's
    // country (`country::Country::index`, 0 for none), the season (months since mid-January, half a year on south of
    // the equator), a test's marks (`TerrainLayer::mark_country_for_tests`: the share of the built-up land taken as
    // marked; 0 none), 1 while the land draws marked texels as the country (else as today's class).
    country: vec4<f32>,
    // The bases' clearings (`TerrainLayer::set_clearings`): the groups held and the edge's width (m); each group's box
    // (chart x, z min and max) and its capsules' first and count (two groups an entry: .xy and .zw); each capsule's ends
    // (chart x, z) and radius (four an entry).
    clearing_head: vec4<f32>,
    clearing_boxes: array<vec4<f32>, 64>,
    clearing_ranges: array<vec4<u32>, 32>,
    clearing_capsules: array<vec4<f32>, 640>,
    clearing_radii: array<vec4<f32>, 160>,
    // The forest floor (`surface.wgsl`'s `floor_at`, `materials.wgsl`'s `floor_read`): the needles' material
    // (`ground_materials::needle_floor`: its layer, period, normal strength and contrast); the trees' biomes about the eye
    // (`species::region_grid`, as `tree_place.wgsl`'s): the grid's first cell's chart corner (x, z), a cell (m), 1 with a
    // grid (a theatre's land), then a byte a cell, cell k = j * 16 + i in `biomes[k / 16][(k / 4) % 4]` at bits 8 (k mod 4).
    needles: vec4<f32>,
    biome_grid: vec4<f32>,
    biomes: array<vec4<u32>, 16>,
    // Where the chart's origin stands on the earth (`Climate::place`, the trees' `TreeView::place`): latitude and
    // longitude (degrees), and 1 with a place (a theatre's land).
    place: vec4<f32>,
}

@group(1) @binding(0) var<uniform> terrain: Terrain;
// Heights in quanta of `terrain.params.z` metres, one texel per field sample, read texel by texel (R16Sint).
@group(1) @binding(1) var heights: texture_2d<i32>;

// Field sample `at`'s height (m).
fn sample_height(at: vec2<i32>) -> f32 {
    return f32(textureLoad(heights, at, 0).r) * terrain.params.z;
}
// Surface gradients (∂h/∂x, ∂h/∂z), mip-mapped (Rg16Float).
@group(1) @binding(2) var gradients: texture_2d<f32>;
// Drainage, broad relief, signed coast distance, fine relief on the attribute grid (Rgba8Unorm, mip-mapped).
@group(1) @binding(3) var attributes: texture_2d<f32>;
// The sun's visibility past the land (R8Unorm, no mips).
@group(1) @binding(4) var sunlight: texture_2d<f32>;
// Open sky on the attribute grid (`SkyBake`, red channel); with trees, the forest's shade in green and blue
// (`tree_shade.wgsl`: the trees standing per tree cell, their mean height over `FOREST_HEIGHT_M`).
@group(1) @binding(5) var open_sky: texture_2d<f32>;
// The noise tile (`TerrainNoise`) and the detail photographs' luminance, repeating.
@group(1) @binding(6) var noise_tile: texture_2d<f32>;
@group(1) @binding(7) var detail_tile: texture_2d<f32>;
// Linear, mip-mapped, anisotropic: clamped for the field's data, repeating for the tiles.
@group(1) @binding(8) var clamp_sampler: sampler;
@group(1) @binding(9) var repeat_sampler: sampler;
// three's split-sum environment BRDF table (`DFG_LUT`).
@group(1) @binding(10) var dfg_lut: texture_2d<f32>;
// The trees' octahedral impostors (`naval_terrain::impostors`): per species a layer of colour (sRGB-encoded,
// premultiplied) and coverage, then per species a layer of normal, depth and ambient occlusion.
@group(1) @binding(11) var tree_atlas: texture_2d_array<f32>;
// The surface maps' satellite colour (sRGB, mip-mapped; colour premultiplied by its alpha in linear light, alpha 1 where
// imagery covers the texel) and land cover (an ESA WorldCover class a texel, read texel by texel), two layers: 0 the
// window's maps (read clamped), 1 the near tiles about the camera (read repeating: tile (i, j) in slot (i mod slots,
// j mod slots)). 1 × 1 and empty with none bound.
@group(1) @binding(12) var surface_albedo: texture_2d_array<f32>;
@group(1) @binding(13) var surface_cover: texture_2d_array<u32>;
// Map features (`features.wgsl`): level l's distances in layer 2 l and its attributes in 2 l + 1 (RGBA8, filtered, read
// repeating at level 0); 1 × 1 with none bound. Fragment and compute stages only, and only on a device allowing
// 17 sampled textures a stage (`FEATURES`).
#ifdef FEATURES
@group(1) @binding(14) var surface_features: texture_2d_array<f32>;
#endif
// The ground materials (`materials.wgsl`): material k's colour ratio and height in layer 2 k, its normal in 2 k + 1
// (filtered, mip-mapped, read repeating); 1 × 1 and neutral with none bound. The fragment stage only, on the High
// setting, on a device allowing one more sampled texture a stage than the land takes without them (`MATERIALS`).
#ifdef MATERIALS
@group(1) @binding(15) var ground_materials: texture_2d_array<f32>;
#endif
// Fine relief near the camera (`naval_terrain::relief`): a layer a page of residual heights (m; R16Float, three levels,
// filtered, read clamped within the layer), and beside them their slopes (Rg8Snorm, `relief::encode_slope`, the land's
// fragments alone); 1 × 1 with none bound. On the High setting, on a device allowing two more sampled textures a stage
// than the land takes without them (`RELIEF`).
#ifdef RELIEF
@group(1) @binding(16) var relief_pages: texture_2d_array<f32>;
@group(1) @binding(17) var relief_slopes: texture_2d_array<f32>;
#endif

const OPEN_SEA_M: f32 = -60.0;
const PI: f32 = 3.141592653589793;

// A test's marks of the period's country (`TerrainLayer::mark_country_for_tests`; a test hook, never a setting:
// `terrain.country.z` is 0 otherwise): with the country drawn, where a smooth noise of 700 m cells fixed to the chart
// lies under the share, a built-up texel centred on chart point `centre` counts as marked (as `cover::PAST_COUNTRY`
// would mark it: `surface.wgsl`'s `country_marked`, `lamps.wgsl`'s `built_about`).
fn country_test_mark(centre: vec2<f32>) -> bool {
    if terrain.country.z <= 0.0 || terrain.country.w <= 0.5 {
        return false;
    }
    let q = centre / 700.0;
    let base = floor(q);
    let f = q - base;
    let s = f * f * (3.0 - 2.0 * f);
    let v = mix(mix(country_test_hash(base), country_test_hash(base + vec2<f32>(1.0, 0.0)), s.x),
        mix(country_test_hash(base + vec2<f32>(0.0, 1.0)), country_test_hash(base + vec2<f32>(1.0, 1.0)), s.x), s.y);
    return v < terrain.country.z;
}

// A hash in 0-1 of a lattice corner (pcg2d's first, as `hex.wgsl`'s `corner_offset`).
fn country_test_hash(corner: vec2<f32>) -> f32 {
    var v = bitcast<vec2<u32>>(vec2<i32>(corner)) * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    return f32(v.x >> 8u) / 16777216.0;
}

fn chart_origin() -> vec2<f32> {
    return terrain.field.xy;
}

fn chart_offset() -> vec2<f32> {
    return terrain.offset.xy;
}

// Value noise for a clearing's ground (0-1, smooth between the lattice's points): arithmetic, so it may be read in any
// branch.
fn clearing_hash(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(0.1031, 0.1030));
    q += dot(q, q.yx + 33.33);
    return fract((q.x + q.y) * q.x);
}

fn clearing_noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = clearing_hash(i);
    let b = clearing_hash(i + vec2<f32>(1.0, 0.0));
    let c = clearing_hash(i + vec2<f32>(0.0, 1.0));
    let d = clearing_hash(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// How far chart point `xz` lies inside a clearing: 1 within one, 0 past its feathered edge or with none. The round
// clearing (`TerrainLayer::set_clearing`), then the bases' capsules (`set_clearings`), their edge ragged: the clearing
// reaches out past a capsule's radius by up to half the edge, so its outline is no capsule's.
fn cleared(xz: vec2<f32>) -> f32 {
    return clearing_at(xz, 0.0).x;
}

// As `cleared`, the clearing reaching `margin` metres further: what a tree standing at `xz` with a crown that wide
// would reach into (the trees' placement, whose slots grow wide with distance).
fn cleared_within(xz: vec2<f32>, margin: f32) -> f32 {
    return clearing_at(xz, margin).x;
}

// A clearing at `xz` reaching `margin` further: x how far inside it (1 within, 0 past its feathered edge or with
// none), y how deep inside the bases' capsules (m from the nearest edge, the deepest capsule's; 0 outside), z how far
// inside one cleared to bare earth (a soft capsule, its radius negative, clears the trees only, its ground the land's
// own worn in patches: what stands among the trees; bare earth wins where they overlap), w how far inside a
// temperate base's (a group flagged in its count's bit 16: its ground mown grass, its trodden ground muddy grass).
fn clearing_at(xz: vec2<f32>, margin: f32) -> vec4<f32> {
    var inside = 0.0;
    var depth = 0.0;
    var hard = 0.0;
    var mild = 0.0;
    let edge = terrain.clearing.w;
    if edge > 0.0 {
        let past = distance(xz, terrain.clearing.xy) - terrain.clearing.z - margin;
        inside = 1.0 - clamp(past / edge, 0.0, 1.0);
        depth = max(-past, 0.0);
        hard = inside;
    }
    let groups = u32(terrain.clearing_head.x);
    if groups == 0u {
        return vec4<f32>(inside, depth, hard, mild);
    }
    let width = terrain.clearing_head.y;
    for (var g = 0u; g < groups; g++) {
        let b = terrain.clearing_boxes[g];
        if xz.x < b.x - margin || xz.y < b.y - margin || xz.x > b.z + margin || xz.y > b.w + margin {
            continue;
        }
        let r = terrain.clearing_ranges[g / 2u];
        let first = select(r.x, r.z, (g & 1u) == 1u);
        let raw = select(r.y, r.w, (g & 1u) == 1u);
        let count = raw & 0xffffu;
        let temperate = f32((raw >> 16u) & 1u);
        // Outward only: within its radius a capsule is wholly clear.
        let wander = (0.5 + 0.5 * sin(xz.x * 0.043 + sin(xz.y * 0.029) * 2.1) * sin(xz.y * 0.051 + sin(xz.x * 0.017) * 1.7)) * 1.2 * width
            + (0.5 + 0.5 * sin(xz.x * 0.137 + sin(xz.y * 0.091) * 1.3) * sin(xz.y * 0.119 + 0.7)) * 0.6 * width
            + (0.5 + 0.5 * sin(xz.x * 0.41 + sin(xz.y * 0.37) * 1.1) * sin(xz.y * 0.43 + 1.9)) * 0.25 * width;
        for (var i = first; i < first + count; i++) {
            let c = terrain.clearing_capsules[i];
            let signed = terrain.clearing_radii[i / 4u][i % 4u];
            let radius = abs(signed);
            let ab = c.zw - c.xy;
            let t = clamp(dot(xz - c.xy, ab) / max(dot(ab, ab), 1e-3), 0.0, 1.0);
            // A small capsule (a camp's cluster, a road) wanders less: it would swell into its neighbours.
            let past = distance(xz, c.xy + ab * t) - radius - wander * clamp(radius / 60.0, 0.2, 1.0) - margin;
            let here = 1.0 - clamp(past / width, 0.0, 1.0);
            inside = max(inside, here);
            depth = max(depth, -past);
            hard = max(hard, select(here, 0.0, signed < 0.0));
            mild = max(mild, here * temperate);
        }
    }
    return vec4<f32>(inside, depth, hard, mild);
}

// The land's height at chart metres as the land draws it: the field's (`field_height`), with the fine relief near the
// camera where its pages are held, read at its level `RELIEF_LOD` (what the trees, the lamps and the casters stand on).
fn height_at(xz: vec2<f32>) -> f32 {
    return relief_height(xz, field_height(xz), RELIEF_LOD);
}

// Bilinear height at chart metres from the height texture, `OPEN_SEA_M` off the field: `terrainHeight` in
// TerrainMaterial.ts, operand for operand (and so `Heightfield.height`, the land the simulation grounds ships on).
fn field_height(xz: vec2<f32>) -> f32 {
    let columns = terrain.grid.x;
    let rows = terrain.grid.y;
    let g = (xz - chart_origin()) / terrain.field.z;
    let i = clamp(floor(g.x), 0.0, columns - 2.0);
    let j = clamp(floor(g.y), 0.0, rows - 2.0);
    let u = g.x - i;
    let v = g.y - j;
    let x = i32(i);
    let z = i32(j);
    let near_left = sample_height(vec2<i32>(x, z));
    let near_right = sample_height(vec2<i32>(x + 1, z));
    let far_left = sample_height(vec2<i32>(x, z + 1));
    let far_right = sample_height(vec2<i32>(x + 1, z + 1));
    let top = near_left * (1.0 - u) + near_right * u;
    let bottom = far_left * (1.0 - u) + far_right * u;
    let inside = g.x >= 0.0 && g.y >= 0.0 && g.x <= columns - 1.0 && g.y <= rows - 1.0;
    return select(OPEN_SEA_M, top * (1.0 - v) + bottom * v, inside);
}

// Texture coordinates of chart metres on the field's grid and on the attribute grid.
fn field_uv(chart: vec2<f32>) -> vec2<f32> {
    return ((chart - chart_origin()) / terrain.field.z + 0.5) / terrain.grid.xy;
}

fn coarse_uv(chart: vec2<f32>) -> vec2<f32> {
    return ((chart - chart_origin()) / terrain.params.x + 0.5) / terrain.grid.zw;
}

// Whether surface maps or near tiles are bound (a uniform: branching on it keeps control flow uniform).
fn surface_bound() -> bool {
    return terrain.surface.z > 0.0 || terrain.near.w > 0.0;
}

// Texture coordinates of chart metres on the surface maps (0-1 across their square; row 0 at the smallest z).
fn surface_uv(chart: vec2<f32>) -> vec2<f32> {
    return (chart - terrain.surface.xy) / max(terrain.surface.z, 1.0);
}

// ---- Fine relief near the camera (`naval_terrain::relief`) ----
//
// Residual heights about the camera from a finer elevation model than the field's: what that model holds and the
// field's grid does not. The land's vertices add them to the field's height (`relief_height`), its fragments their
// slope to the field's (`relief_gradient`); none at or below the sea, so the coast stays where the field draws it.

// A page's texels a side, its own, and its gutter of the neighbours' on each side (`relief::PAGE_TEXELS`, `CONTENT`,
// `GUTTER`); its levels (`LEVELS`).
const RELIEF_PAGE_TEXELS: f32 = 512.0;
const RELIEF_CONTENT: f32 = 504.0;
const RELIEF_GUTTER: f32 = 4.0;
const RELIEF_LEVELS: f32 = 3.0;
// The share of a page over which the relief fades out toward a neighbour not held (`relief::FEATHER`).
const RELIEF_FEATHER: f32 = 0.15;
// The height above the sea (m) from which the land takes all of the relief; below it a share, none at the waterline.
const RELIEF_SHORE_M: f32 = 4.0;
// Land is never lowered below this share of its field height: what the field draws as land stays land.
const RELIEF_FLOOR: f32 = 0.15;
// The level read by what stands on the land (trees, lamps, casters): near what the land's level-0 vertices read, and
// whole, so a read is one bilinear tap.
const RELIEF_LOD: f32 = 1.0;

struct ReliefAt {
    // The layer holding the point's page, -1 for none.
    layer: i32,
    // The point in the layer (0-1, gutters counted).
    uv: vec2<f32>,
    // How much of the relief the point takes: 1 inside the pages held, fading toward an edge whose neighbour is not.
    share: f32,
}

// The layer holding page `page`, -1 where its slot holds another or none.
fn relief_layer(page: vec2<i32>) -> i32 {
    let slots = max(i32(terrain.relief.w), 1);
    let slot = ((page % slots) + slots) % slots;
    let k = slot.y * slots + slot.x;
    let entry = terrain.relief_slots[k / 2];
    let held = select(entry.zw, entry.xy, k % 2 == 0);
    return select(-1, k, all(held == page));
}

fn relief_at(xz: vec2<f32>) -> ReliefAt {
    var at: ReliefAt;
    at.layer = -1;
    at.uv = vec2<f32>(0.0);
    at.share = 0.0;
    if terrain.relief.w <= 0.0 {
        return at;
    }
    let t = (xz - terrain.relief.xy) / (terrain.relief.z * RELIEF_CONTENT);
    let base = floor(t);
    let page = vec2<i32>(base);
    let layer = relief_layer(page);
    if layer < 0 {
        return at;
    }
    let f = t - base;
    // A neighbour is looked up only within the feather toward it.
    var share = 1.0;
    if f.x < RELIEF_FEATHER && relief_layer(page - vec2<i32>(1, 0)) < 0 {
        share = share * smoothstep(0.0, RELIEF_FEATHER, f.x);
    }
    if f.x > 1.0 - RELIEF_FEATHER && relief_layer(page + vec2<i32>(1, 0)) < 0 {
        share = share * smoothstep(0.0, RELIEF_FEATHER, 1.0 - f.x);
    }
    if f.y < RELIEF_FEATHER && relief_layer(page - vec2<i32>(0, 1)) < 0 {
        share = share * smoothstep(0.0, RELIEF_FEATHER, f.y);
    }
    if f.y > 1.0 - RELIEF_FEATHER && relief_layer(page + vec2<i32>(0, 1)) < 0 {
        share = share * smoothstep(0.0, RELIEF_FEATHER, 1.0 - f.y);
    }
    at.layer = layer;
    at.uv = (f * RELIEF_CONTENT + RELIEF_GUTTER) / RELIEF_PAGE_TEXELS;
    at.share = share;
    return at;
}

// How much of a real page's relief point `xz` takes (`relief_at`'s share where its layer holds a finer model's page,
// 0 where a made one: the synthetic's, GLO-30's).
fn relief_real_share(xz: vec2<f32>) -> f32 {
    let at = relief_at(xz);
    if at.layer < 0 {
        return 0.0;
    }
    let bits = select(terrain.relief_real.y, terrain.relief_real.x, at.layer < 32);
    let real = (bits >> u32(at.layer % 32)) & 1u;
    return select(0.0, at.share, real == 1u);
}

// The residual at `uv` of layer `layer`, level `lod` (m).
fn relief_read(layer: i32, uv: vec2<f32>, lod: f32) -> f32 {
#ifdef RELIEF
    return textureSampleLevel(relief_pages, clamp_sampler, uv, layer, lod).r;
#else
    return 0.0;
#endif
}

// How much of the relief land `coarse` metres up takes: none at the waterline, all of it from `RELIEF_SHORE_M`.
fn relief_weight(coarse: f32) -> f32 {
    return smoothstep(0.0, RELIEF_SHORE_M, coarse);
}

// The field's height `coarse` with relief `r` (m) added: none at or below the sea, and land never lowered below
// `RELIEF_FLOOR` of its field height.
fn with_relief(coarse: f32, r: f32) -> f32 {
    let lifted = max(coarse + relief_weight(coarse) * r, coarse * RELIEF_FLOOR);
    return select(coarse, lifted, coarse > 0.0);
}

// The land's height at chart point `xz` whose field height is `coarse`, with the relief read at level `lod`.
fn relief_height(xz: vec2<f32>, coarse: f32, lod: f32) -> f32 {
#ifdef RELIEF
    if coarse > 0.0 {
        let at = relief_at(xz);
        if at.layer >= 0 {
            return with_relief(coarse, at.share * relief_read(at.layer, at.uv, clamp(lod, 0.0, RELIEF_LEVELS - 1.0)));
        }
    }
#endif
    return coarse;
}

// The relief's slope (∂/∂x, ∂/∂z) at chart point `xz` of land `height` metres up, filtered to `footprint` metres (a
// pixel's), as much of it as the land there takes: the page's slopes (one tap of binding 17) on the level that spans
// the footprint, filtered as encoded (`relief::encode_slope`) and decoded.
fn relief_gradient(xz: vec2<f32>, height: f32, footprint: f32) -> vec2<f32> {
#ifdef RELIEF
    if height > 0.0 {
        let at = relief_at(xz);
        if at.layer >= 0 {
            let texel = terrain.relief.z;
            let lod = clamp(log2(max(footprint, texel) / texel), 0.0, RELIEF_LEVELS - 1.0);
            let read = textureSampleLevel(relief_slopes, clamp_sampler, at.uv, at.layer, lod).xy;
            let c = clamp(read, vec2<f32>(-0.98), vec2<f32>(0.98));
            return c / (1.0 - abs(c)) * at.share * relief_weight(height);
        }
    }
#endif
    return vec2<f32>(0.0);
}
