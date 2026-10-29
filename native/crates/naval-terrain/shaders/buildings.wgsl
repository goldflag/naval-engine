// Buildings over a theatre's land (native/docs/terrain/buildings.md): extruded footprints in chunks, each chunk's
// vertices chunk-local and placed by its instance's translation; walls with windows laid out per wall from the facade
// coordinates (bays along, storeys up) by the building's kind (houses with shutters, blocks with balconies, offices'
// curtain walls, warehouses' doors, churches' tall windows), roofs in their palette colour with tiles or patches,
// banded towers, glazed lanterns, painted steel, smooth spires and domes; lit as the land is (the sun through the
// shadow maps and the clouds, the hemisphere fill, the sky's environment) and fogged as the land is. Walls weather (dirt
// run down from their sills and heads, splashed up their feet, algae on damp north faces, rust, sun-bleached paint) and
// roofs (each tile its shade, rolled tiles' rolls, ridge tiles, moss and lichen, rust); at night their rooms light by
// household and by the hour, under the scenario's rule.
//
// With FACADE_TEXTURES (High and Ultra) a wall takes its surface from the facade textures (`naval_terrain::buildings::
// facades`: render, stucco, brick, siding, boards, sheet, panels, ashlar, tile, by its palette entry and region, laid in
// metres) and each bay and storey its window from them (by the region and the building's kind), with their relief;
// without, the windows are drawn here from rectangles. The roof clutter (`vs_clutter`: air conditioners, housings,
// tanks, chimneys) is shaded as the walls and roofs.
//
// A chunk drawn in a past period (the translation's w over 31: native/docs/terrain/period.md, "Period styling")
// takes `wall_material_past` and `window_style_past` (no panels, tile or vinyl siding; no sliders, curtain walls,
// ribbons, garage or roller doors, jalousies or Japanese aluminium; masonry punched windows, awnings and signboards,
// carriage doors, shoji and amado, wooden louvres), a cornice on a masonry block's or office's head and soot on
// Britain's, Ireland's, the North Sea's and Europe's walls. Today's chunks draw as they did.
//
// A far chunk (`vs_far`: native/docs/terrain/buildings.md, "The far level") is instances of one gabled box, each a
// run of a street's lots or, farther, a whole block (`naval_terrain::buildings::far::Mass`), shaded as the walls and
// roofs are; its masses sink into the ground over the far level's last kilometres. A chunk changing its level fades in over the one it replaces,
// pixel by pixel (BUILDINGS_FADE: the pipeline the fading chunks are drawn with; the others discard nothing).
// No port: the browser game drew no buildings.
#import naval::camera::{camera, world_to_clip}
#import naval::aerial::aerial_apply
#import naval::shadow::{sun_shadow}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#import naval_terrain::structures::{srgb_to_linear, hash12, hash13, band, tone, brdf_ggx}
#import naval_terrain::tree_wind::pixel_dither
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction, env_fog}

struct Buildings {
    // Toward the sun (or moon); w: the share of the windows lit at night, 0-1.
    sun_direction: vec4<f32>,
    // The directional light on meshes; w: the environment's intensity.
    sun_radiance: vec4<f32>,
    // The hemisphere fill's sky and ground; fill_sky.w: 1 while the clouds' shadow is bound.
    fill_sky: vec4<f32>,
    fill_ground: vec4<f32>,
    // Scene fog: colour and power; start, end, sky blend (m), 1 while on.
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    // x: the full-detail distance (m), y: the range (m), z: the share of the windows dark at night beyond the usual two
    // thirds (a scenario's dimout or blackout, `BuildingsLayer::set_windows`), w: the far level's range across the
    // ground (m; 0 none).
    params: vec4<f32>,
    // The far level's blocks (`vs_far`): x where the first block takes over from its runs (m across the ground), y the
    // stretch the blocks' hashes spread that over, z how far past its block's the runs stand on (m).
    far: vec4<f32>,
}

@group(1) @binding(0) var<uniform> buildings: Buildings;
// The facade textures (`facades::Facades`): the walls' materials, the windows' colour and cover, their normal, glass and
// accent; repeating, trilinear and anisotropic.
@group(1) @binding(1) var facade_walls: texture_2d_array<f32>;
@group(1) @binding(2) var facade_windows: texture_2d_array<f32>;
@group(1) @binding(3) var facade_window_surface: texture_2d_array<f32>;
@group(1) @binding(4) var facade_sampler: sampler;
// The chunks' translations by slot (the clutter's instances name their chunk's).
@group(1) @binding(5) var<storage, read> chunk_translations: array<vec4<f32>>;

@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;
@group(2) @binding(3) var sky_fog: texture_2d<f32>;

@group(3) @binding(0) var cloud_map: texture_2d<f32>;
@group(3) @binding(1) var cloud_sampler: sampler;
@group(3) @binding(2) var<uniform> cloud_params: CloudShadow;

const PI: f32 = 3.141592653589793;
const RECIPROCAL_PI: f32 = 0.3183098861837907;
const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// `naval_terrain::buildings::mesh`'s parts (the style byte's bits 4-6).
const PART_WALL: u32 = 0u;
const PART_ROOF: u32 = 1u;
const PART_GABLE: u32 = 2u;
const PART_BANDED: u32 = 3u;
const PART_GLASS: u32 = 4u;
const PART_STEEL: u32 = 5u;
const PART_DOME: u32 = 6u;
const PART_LEAF: u32 = 7u;

// `naval_terrain::buildings::Kind` (the style byte's bits 0-3).
const KIND_SHED: u32 = 0u;
const KIND_HOUSE: u32 = 1u;
const KIND_TERRACE: u32 = 2u;
const KIND_BLOCK: u32 = 3u;
const KIND_TOWER: u32 = 4u;
const KIND_INDUSTRIAL: u32 = 5u;
const KIND_CHURCH: u32 = 6u;
const KIND_OFFICE: u32 = 7u;
const KIND_WAREHOUSE: u32 = 8u;
const KIND_LIGHTHOUSE: u32 = 10u;

// `naval_terrain::buildings::Region::index` (the chunk translation's w).
const REGION_NORDIC: u32 = 0u;
const REGION_TEMPERATE: u32 = 1u;
const REGION_TROPICAL: u32 = 2u;
const REGION_ARID: u32 = 3u;
const REGION_MEDITERRANEAN: u32 = 4u;
const REGION_AMERICAN: u32 = 5u;
const REGION_WEST: u32 = 6u;
const REGION_EAST_ASIA: u32 = 7u;
const REGION_ICELAND: u32 = 8u;
const REGION_BRITISH: u32 = 9u;
const REGION_CELTIC: u32 = 10u;
const REGION_NORTH_SEA: u32 = 11u;
const REGION_FLORIDA: u32 = 12u;
const REGION_MALTA: u32 = 13u;
const REGION_AEGEAN: u32 = 14u;
const REGION_NORTH_AFRICA: u32 = 15u;
const REGION_NILE: u32 = 16u;
const REGION_HAWAII: u32 = 17u;
const REGION_MICRONESIA: u32 = 18u;
const REGION_ATOLL: u32 = 19u;

// `facades::Material`: the walls' layers.
const MATERIAL_RENDER: u32 = 0u;
const MATERIAL_STUCCO: u32 = 1u;
const MATERIAL_BRICK: u32 = 2u;
const MATERIAL_SIDING: u32 = 3u;
const MATERIAL_BOARDS: u32 = 4u;
const MATERIAL_CORRUGATED: u32 = 5u;
const MATERIAL_PANELS: u32 = 6u;
const MATERIAL_ASHLAR: u32 = 7u;
const MATERIAL_TILE: u32 = 8u;
// `facades::WALL_TILE_M`.
const WALL_TILE_M: f32 = 4.0;

// `facades::Window`: the windows' layers.
const WINDOW_SASH: u32 = 0u;
const WINDOW_SASH_SHUTTERED: u32 = 1u;
const WINDOW_SHUTTERED: u32 = 2u;
const WINDOW_CASEMENT: u32 = 3u;
const WINDOW_NORDIC: u32 = 4u;
const WINDOW_SLIDER: u32 = 5u;
const WINDOW_JAPAN_HOUSE: u32 = 6u;
const WINDOW_JALOUSIE: u32 = 7u;
const WINDOW_GRILLE: u32 = 8u;
const WINDOW_DOOR: u32 = 9u;
const WINDOW_GARAGE: u32 = 10u;
const WINDOW_BLOCK: u32 = 11u;
const WINDOW_BALCONY: u32 = 12u;
const WINDOW_JAPAN_BALCONY: u32 = 13u;
const WINDOW_JAPAN_BLOCK: u32 = 14u;
const WINDOW_SHOPFRONT: u32 = 15u;
const WINDOW_CURTAIN: u32 = 16u;
const WINDOW_RIBBON: u32 = 17u;
const WINDOW_HIGH_RIBBON: u32 = 18u;
const WINDOW_ROLLER: u32 = 19u;
const WINDOW_ARCHED: u32 = 20u;
const WINDOW_BELFRY: u32 = 21u;
// A past period's.
const WINDOW_CARRIAGE_DOOR: u32 = 22u;
const WINDOW_SHOJI: u32 = 23u;
const WINDOW_AMADO: u32 = 24u;
const WINDOW_PUNCHED: u32 = 25u;
const WINDOW_AWNING: u32 = 26u;
const WINDOW_SIGNBOARD: u32 = 27u;
const WINDOW_LOUVRES: u32 = 28u;
const WINDOW_NONE: u32 = 255u;
// The windows' count: the surface array's second half holds each one's recess (`facades::Facades::window_surface`).
const WINDOW_COUNT: u32 = 29u;

// The walls' materials by palette entry (0-31; the colour cube's are render).
const WALL_MATERIALS = array<u32, 32>(
    MATERIAL_RENDER,     // white render
    MATERIAL_RENDER,     // cream
    MATERIAL_PANELS,     // light concrete
    MATERIAL_PANELS,     // mid concrete
    MATERIAL_BRICK,      // red brick
    MATERIAL_BRICK,      // brown brick
    MATERIAL_BRICK,      // yellow brick
    MATERIAL_RENDER,     // pastel blue
    MATERIAL_RENDER,     // pastel green
    MATERIAL_RENDER,     // pastel pink
    MATERIAL_RENDER,     // ochre
    MATERIAL_CORRUGATED, // grey sheet metal
    MATERIAL_BOARDS,     // dark timber
    MATERIAL_RENDER,     // bright white
    MATERIAL_PANELS,     // dark glass
    MATERIAL_RENDER,     // sand
    MATERIAL_RENDER,     // salmon render
    MATERIAL_RENDER,     // warm yellow render
    MATERIAL_RENDER,     // whitewash
    MATERIAL_RENDER,     // light ochre
    MATERIAL_RENDER,     // peach
    MATERIAL_SIDING,     // light grey siding
    MATERIAL_SIDING,     // beige siding
    MATERIAL_SIDING,     // blue-grey siding
    MATERIAL_BOARDS,     // Falu red
    MATERIAL_CORRUGATED, // blue sheet metal
    MATERIAL_CORRUGATED, // green sheet metal
    MATERIAL_CORRUGATED, // cream sheet metal
    MATERIAL_PANELS,     // blue glass
    MATERIAL_PANELS,     // green glass
    MATERIAL_ASHLAR,     // stone grey
    MATERIAL_ASHLAR,     // limestone
);

// The walls' palette (sRGB bytes): `naval_terrain::buildings`'s entries 0-31; 32 and up the colour cube.
const WALLS = array<vec3<f32>, 32>(
    vec3<f32>(206.0, 202.0, 190.0), // white render
    vec3<f32>(200.0, 186.0, 156.0), // cream
    vec3<f32>(168.0, 166.0, 158.0), // light concrete
    vec3<f32>(142.0, 141.0, 136.0), // mid concrete
    vec3<f32>(146.0, 78.0, 60.0),   // red brick
    vec3<f32>(116.0, 80.0, 64.0),   // brown brick
    vec3<f32>(190.0, 164.0, 116.0), // yellow brick
    vec3<f32>(148.0, 166.0, 180.0), // pastel blue
    vec3<f32>(154.0, 174.0, 138.0), // pastel green
    vec3<f32>(192.0, 158.0, 148.0), // pastel pink
    vec3<f32>(204.0, 168.0, 96.0),  // ochre
    vec3<f32>(160.0, 165.0, 168.0), // grey sheet metal
    vec3<f32>(96.0, 72.0, 56.0),    // dark timber
    vec3<f32>(216.0, 216.0, 210.0), // bright white
    vec3<f32>(88.0, 98.0, 108.0),   // dark glass
    vec3<f32>(190.0, 174.0, 142.0), // sand
    vec3<f32>(200.0, 140.0, 104.0), // salmon render
    vec3<f32>(214.0, 184.0, 112.0), // warm yellow render
    vec3<f32>(222.0, 218.0, 206.0), // whitewash
    vec3<f32>(210.0, 182.0, 136.0), // light ochre
    vec3<f32>(214.0, 180.0, 150.0), // peach
    vec3<f32>(178.0, 180.0, 176.0), // light grey siding
    vec3<f32>(196.0, 184.0, 160.0), // beige siding
    vec3<f32>(128.0, 142.0, 152.0), // blue-grey siding
    vec3<f32>(128.0, 44.0, 36.0),   // Falu red
    vec3<f32>(74.0, 104.0, 140.0),  // blue sheet metal
    vec3<f32>(88.0, 120.0, 96.0),   // green sheet metal
    vec3<f32>(196.0, 190.0, 170.0), // cream sheet metal
    vec3<f32>(70.0, 92.0, 112.0),   // blue glass
    vec3<f32>(78.0, 104.0, 100.0),  // green glass
    vec3<f32>(150.0, 144.0, 132.0), // stone grey
    vec3<f32>(196.0, 188.0, 166.0), // limestone
);

// The authored entries after the colour cube (`naval_terrain::buildings::palette::EXTRA_FIRST`, 248 and up): the
// walls' materials and colours, and the roofs'.
const EXTRA_FIRST: u32 = 248u;
const WALL_MATERIALS_EXTRA = array<u32, 5>(
    MATERIAL_STUCCO,     // harl
    MATERIAL_BRICK,      // purple-brown klinker
    MATERIAL_ASHLAR,     // red sandstone
    MATERIAL_ASHLAR,     // honey limestone
    MATERIAL_BOARDS,     // charred boards (a past period's)
);
const WALLS_EXTRA = array<vec3<f32>, 5>(
    vec3<f32>(168.0, 164.0, 152.0), // harl
    vec3<f32>(100.0, 62.0, 56.0),   // purple-brown klinker
    vec3<f32>(150.0, 92.0, 74.0),   // red sandstone
    vec3<f32>(212.0, 184.0, 132.0), // honey limestone
    vec3<f32>(54.0, 46.0, 42.0),    // charred boards
);
const ROOFS_EXTRA = array<vec3<f32>, 4>(
    vec3<f32>(122.0, 104.0, 74.0),  // reed thatch
    vec3<f32>(184.0, 168.0, 136.0), // limestone screed
    vec3<f32>(80.0, 83.0, 90.0),    // smoked (ibushi) kawara
    vec3<f32>(46.0, 52.0, 64.0),    // dark glazed kawara
);
const THATCH: u32 = 248u;
const IBUSHI: u32 = 250u;
const GLAZED: u32 = 251u;
// `palette::KAWARA`: the colour cube's (102, 102, 102), weathered grey kawara.
const KAWARA: u32 = 118u;

// The roofs' palette (sRGB bytes): entries 0-31; 32 and up the colour cube.
const ROOFS = array<vec3<f32>, 32>(
    vec3<f32>(160.0, 84.0, 60.0),   // terracotta
    vec3<f32>(124.0, 68.0, 54.0),   // dark terracotta
    vec3<f32>(72.0, 76.0, 82.0),    // slate
    vec3<f32>(66.0, 66.0, 68.0),    // asphalt
    vec3<f32>(148.0, 48.0, 42.0),   // red iron
    vec3<f32>(68.0, 108.0, 84.0),   // green iron
    vec3<f32>(62.0, 86.0, 120.0),   // blue iron
    vec3<f32>(134.0, 138.0, 140.0), // galvanised
    vec3<f32>(128.0, 90.0, 66.0),   // rusted
    vec3<f32>(110.0, 106.0, 98.0),  // gravel
    vec3<f32>(148.0, 148.0, 142.0), // pale membrane
    vec3<f32>(74.0, 74.0, 74.0),    // tar
    vec3<f32>(44.0, 44.0, 46.0),    // black
    vec3<f32>(110.0, 76.0, 58.0),   // brown tile
    vec3<f32>(128.0, 126.0, 118.0), // concrete
    vec3<f32>(176.0, 176.0, 170.0), // white
    vec3<f32>(178.0, 100.0, 64.0),  // orange terracotta
    vec3<f32>(168.0, 112.0, 88.0),  // faded terracotta
    vec3<f32>(96.0, 96.0, 96.0),    // grey shingle
    vec3<f32>(98.0, 82.0, 70.0),    // brown shingle
    vec3<f32>(58.0, 58.0, 62.0),    // dark shingle
    vec3<f32>(98.0, 150.0, 128.0),  // copper patina
    vec3<f32>(110.0, 114.0, 118.0), // lead
    vec3<f32>(138.0, 70.0, 56.0),   // red-brown tile
    vec3<f32>(196.0, 196.0, 190.0), // white membrane
    vec3<f32>(60.0, 66.0, 80.0),    // blue slate
    vec3<f32>(52.0, 54.0, 58.0),    // anthracite
    vec3<f32>(90.0, 104.0, 96.0),   // green-grey
    vec3<f32>(170.0, 130.0, 96.0),  // sand tile
    vec3<f32>(160.0, 164.0, 166.0), // light galvanised
    vec3<f32>(176.0, 140.0, 72.0),  // gilt
    vec3<f32>(120.0, 128.0, 140.0), // blue-grey lead
);

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) facade: vec2<f32>,
    @location(3) style: vec4<u32>,
    // The chunk's translation into the world (x, y, z) and its region (w).
    @location(4) chunk: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) facade: vec2<f32>,
    @location(3) @interpolate(flat) style: vec4<u32>,
    @location(4) @interpolate(flat) rows: f32,
    @location(5) @interpolate(flat) region: u32,
    // Chunk-local (for the walls' materials, laid in metres: small numbers, steady at a city's far side).
    @location(6) local: vec3<f32>,
    // 1 in a past period (the chunk translation's bit 5), 0 today.
    @location(7) @interpolate(flat) past: u32,
    // The chunk's fade (the translation's bits 6 and up, `fade_hidden`): 0 drawn whole.
    @location(8) @interpolate(flat) fade: u32,
    // The wall's own number (for each window's room, dressing and lamp): from its first corner, which every triangle
    // of a face shares as its first vertex (`mesh.rs`' `face` and `parts.rs`' `smooth_face` fan from it), so the
    // number holds over the whole wall; 0 off the building meshes.
    @location(9) @interpolate(flat) wall: f32,
}

@vertex
fn vs_building(in: VertexIn) -> VertexOut {
    let world = in.position + in.chunk.xyz;
    var out: VertexOut;
    out.clip = world_to_clip(world);
    out.world = world;
    out.normal = in.normal.xyz;
    out.facade = in.facade;
    out.style = in.style;
    // snorm8's w: the window rows, 1/127 each.
    out.rows = round(in.normal.w * 127.0);
    let packed = u32(in.chunk.w + 0.5);
    out.region = packed & 31u;
    out.past = (packed >> 5u) & 1u;
    out.fade = packed >> 6u;
    out.local = in.position;
    out.wall = hash13(round(in.position * 4.0)) * 61.0;
    return out;
}

// `naval_terrain::buildings::clutter::Clutter` over a `ShapeVertex`.
struct ClutterIn {
    @location(0) position: vec3<f32>,
    // w: 1 on a top (the roof's colour), 0 on a side.
    @location(1) normal: vec4<f32>,
    @location(2) at_yaw: vec4<f32>,
    @location(3) size: vec3<f32>,
    // The shape (bits 0-7) and the chunk's slot (8-31).
    @location(4) shape: u32,
    @location(5) style: vec4<u32>,
}

// `clutter::CLUTTER_M`, and the far end of the fade (none: a piece stands or does not, a metre at 1.5 km).
const CLUTTER_M: f32 = 1500.0;

fn clutter_world(in: ClutterIn) -> vec4<f32> {
    let t = chunk_translations[in.shape >> 8u];
    let c = cos(in.at_yaw.w);
    let s = sin(in.at_yaw.w);
    let p = in.position * in.size;
    let local = in.at_yaw.xyz + vec3<f32>(p.x * c - p.z * s, p.y, p.x * s + p.z * c);
    return vec4<f32>(local + t.xyz, t.w);
}

@vertex
fn vs_clutter(in: ClutterIn) -> VertexOut {
    let placed = clutter_world(in);
    let world = placed.xyz;
    let t = chunk_translations[in.shape >> 8u];
    var out: VertexOut;
    out.clip = world_to_clip(world);
    // Past its distance the whole piece goes (judged at its foot, so it never tears).
    if distance(camera.position, in.at_yaw.xyz + t.xyz) > CLUTTER_M {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    let c = cos(in.at_yaw.w);
    let s = sin(in.at_yaw.w);
    // The shape's normal through the inverse of its scale (a tower's drum stretched up leans its cone's normal).
    let n = normalize(in.normal.xyz / max(in.size, vec3<f32>(1e-3)));
    out.world = world;
    out.normal = vec3<f32>(n.x * c - n.z * s, n.y, n.x * s + n.z * c);
    // A storey's middle with no rows: no windows, no ground shade.
    out.facade = vec2<f32>(0.0, 0.5);
    var style = in.style;
    if in.normal.w > 0.5 && ((style.w >> 4u) & 7u) != PART_LEAF {
        style.w = (style.w & 0x8fu) | (PART_DOME << 4u);
    }
    out.style = style;
    out.rows = 0.0;
    let packed = u32(t.w + 0.5);
    out.region = packed & 31u;
    out.past = (packed >> 5u) & 1u;
    out.fade = 0u;
    out.local = world - t.xyz;
    return out;
}

// `naval_terrain::buildings::far::Mass` over the far shape's vertex (`far::far_shape`: y 0 at the foot, 1 at the eaves,
// 2 at the ridge; normal.w 1 on the roof, about 0.5 on a gable, 0 on a wall).
struct FarIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) foot: f32,
    // Its middle's chunk-local x and z, its length along its street and its depth.
    @location(3) place: vec4<f32>,
    // Its eaves over the ground, its ridge's rise over them.
    @location(4) height: vec2<f32>,
    @location(5) style: vec4<u32>,
    // Its heading (bits 0-7), the ground over its foot (8-15, quarter metres), its chunk's slot (16-31).
    @location(6) turn: u32,
}

// Over the far level's last stretch (m) its masses sink into the ground, so none stands or goes at its edge.
const FAR_FADE_M: f32 = 3000.0;

// A storey's height and a window bay's width (m) for a mass's kind (`naval_terrain::buildings::storey_height`, `bay_m`).
fn far_storey(kind: u32) -> f32 {
    if kind == KIND_BLOCK || kind == KIND_TOWER {
        return 3.1;
    }
    return 2.8;
}

fn far_bay(kind: u32) -> f32 {
    if kind == KIND_BLOCK || kind == KIND_TOWER {
        return 3.3;
    }
    return 3.0;
}

@vertex
fn vs_far(in: FarIn) -> VertexOut {
    let t = chunk_translations[in.turn >> 16u];
    let ground = in.foot + f32((in.turn >> 8u) & 255u) * 0.25;
    let heading = f32(in.turn & 255u) / 256.0 * PI;
    let c = cos(heading);
    let s = sin(heading);
    // Sunk into the ground as the camera draws away across it, toward the far level's range.
    let middle = vec2<f32>(in.place.x, in.place.y) + t.xz;
    let reach = length(camera.position.xz - middle);
    var k = 1.0 - smoothstep(buildings.params.w - FAR_FADE_M, buildings.params.w, reach);
    // A block's runs nearer, its one mass farther: each block's from its hash's distance on (`far.x` and `far.y`), its
    // runs standing on for `far.z` past it, so no block stands empty between the two.
    let block = (in.style.w & 16u) != 0u;
    let takes_over = buildings.far.x + buildings.far.y * f32(in.style.z) / 255.0;
    if (block && reach <= takes_over) || (!block && reach > takes_over + buildings.far.z) {
        k = 0.0;
    }
    let eaves = in.height.x * k;
    let rise = in.height.y * k;
    var y = in.foot;
    if in.position.y > 1.5 {
        y = ground + eaves + rise;
    } else if in.position.y > 0.5 {
        y = ground + eaves;
    }
    // Along its street (the infill's frame: along (c, s), across (-s, c)).
    let along = in.position.x * in.place.z;
    let across = in.position.z * in.place.w;
    let local = vec3<f32>(in.place.x + along * c - across * s, y, in.place.y + along * s + across * c);
    let world = local + t.xyz;
    var out: VertexOut;
    out.clip = world_to_clip(world);
    if k < 0.01 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    let kind = in.style.w & 15u;
    let storey_m = far_storey(kind);
    var part = PART_WALL;
    var n = in.normal.xyz;
    var facade = vec2<f32>(0.0, (y - ground) / storey_m);
    if in.normal.w > 0.75 {
        // A roof: its slope's normal from the rise over half the depth (straight up when flat); metres down the slope
        // from the ridge and along it (pitched), or across the roof (flat).
        part = PART_ROOF;
        let half_depth = 0.5 * in.place.w;
        n = vec3<f32>(0.0, half_depth, rise * sign(in.normal.z));
        if rise > 0.05 {
            let slope = sqrt(half_depth * half_depth + rise * rise);
            facade = vec2<f32>(abs(in.position.z) * 2.0 * slope, (in.position.x + 0.5) * in.place.z);
        } else {
            facade = local.xz;
        }
    } else if in.normal.w > 0.25 {
        part = PART_GABLE;
    } else {
        // Whole bays from corner to corner along a long side or an end.
        let side = abs(in.normal.z) > 0.5;
        let length = select(in.place.w, in.place.z, side);
        let u = select(in.position.z, in.position.x, side);
        let bays = max(1.0, round(length / far_bay(kind)));
        facade.x = (u + 0.5) * bays;
    }
    n = normalize(n);
    out.world = world;
    out.normal = vec3<f32>(n.x * c - n.z * s, n.y, n.x * s + n.z * c);
    out.facade = facade;
    // Each face its own number (opposite faces count their bays the same way).
    out.wall = 3.0 + round(in.normal.x) * 2.0 + round(in.normal.z) * 5.0;
    out.style = vec4<u32>(in.style.x, in.style.y, in.style.z, kind | (part << 4u));
    out.rows = round(eaves / storey_m);
    let packed = u32(t.w + 0.5);
    out.region = packed & 31u;
    out.past = (packed >> 5u) & 1u;
    out.fade = packed >> 6u;
    out.local = local;
    return out;
}

// Whether a fading chunk leaves `pixel` to the other level: `fade`'s bit 0 says which way it fades (0 in, 1 out), the
// rest its share (in 255ths) still hidden fading in, or already gone fading out. The two levels of a chunk take
// complementary pixels.
fn fade_hidden(fade: u32, pixel: vec2<f32>) -> bool {
    if fade == 0u {
        return false;
    }
    let share = f32(fade >> 1u) / 255.0;
    let u = pixel_dither(pixel);
    if (fade & 1u) == 0u {
        return u >= 1.0 - share;
    }
    return u < share;
}

// The colour cube's entry `i` (32 and up): six levels a channel, `naval_terrain::buildings::cube_entry`.
fn cube_colour(i: u32) -> vec3<f32> {
    let k = min(i - 32u, 215u);
    return vec3<f32>(f32(k / 36u), f32((k / 6u) % 6u), f32(k % 6u)) * 51.0;
}

fn wall_colour(i: u32) -> vec3<f32> {
    if i >= EXTRA_FIRST {
        return srgb_to_linear(WALLS_EXTRA[min(i - EXTRA_FIRST, 4u)]);
    }
    if i >= 32u {
        return srgb_to_linear(cube_colour(i));
    }
    return srgb_to_linear(WALLS[i]);
}

fn roof_colour(i: u32) -> vec3<f32> {
    if i >= EXTRA_FIRST {
        return srgb_to_linear(ROOFS_EXTRA[min(i - EXTRA_FIRST, 3u)]);
    }
    if i >= 32u {
        return srgb_to_linear(cube_colour(i));
    }
    return srgb_to_linear(ROOFS[i]);
}

struct Surface {
    albedo: vec3<f32>,
    roughness: f32,
    // The reflectance at normal incidence over 0.04 (1 a dielectric's, glass about 1.2).
    specular: f32,
    occlusion: f32,
    emissive: vec3<f32>,
    // A wall's relief: its normal's lean along the wall and up it (zero: the face's own).
    bump: vec2<f32>,
    // What the rooms behind a window's glass send back of the light falling on it (`room`; lit in `fs_building`).
    interior: vec3<f32>,
    // The share of the sun a window's reveal keeps off it (0 none).
    sun_shade: f32,
    // The share of the sun on the glass that the room behind it (`interior`) takes.
    interior_sun: f32,
}

// Weathering (native/docs/terrain/buildings.md, "Weathering").

// Value noise along a line, 0-1, smooth between whole steps of `x`.
fn noise1(x: f32, seed: f32) -> f32 {
    let i = floor(x);
    let f = x - i;
    return mix(hash12(vec2<f32>(i, seed)), hash12(vec2<f32>(i + 1.0, seed)), f * f * (3.0 - 2.0 * f));
}

// Value noise over a plane, 0-1.
fn noise2(p: vec2<f32>, seed: f32) -> f32 {
    let i = floor(p) + vec2<f32>(seed, seed * 1.7);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash12(i);
    let b = hash12(i + vec2<f32>(1.0, 0.0));
    let c = hash12(i + vec2<f32>(0.0, 1.0));
    let d = hash12(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// How damp a region keeps its walls and roofs (algae and moss, rising damp, dirt's longer runs), 0-1.
fn region_wet(region: u32) -> f32 {
    switch region {
        case REGION_BRITISH, REGION_CELTIC: {
            return 1.0;
        }
        case REGION_TROPICAL, REGION_MICRONESIA, REGION_ATOLL, REGION_HAWAII: {
            return 0.9;
        }
        case REGION_NORTH_SEA: {
            return 0.85;
        }
        case REGION_EAST_ASIA: {
            return 0.75;
        }
        case REGION_FLORIDA: {
            return 0.65;
        }
        case REGION_NORDIC, REGION_TEMPERATE: {
            return 0.6;
        }
        case REGION_ICELAND, REGION_AMERICAN: {
            return 0.45;
        }
        case REGION_MEDITERRANEAN: {
            return 0.25;
        }
        case REGION_MALTA, REGION_AEGEAN: {
            return 0.15;
        }
        case REGION_WEST, REGION_NORTH_AFRICA: {
            return 0.1;
        }
        default: {
            return 0.0;
        }
    }
}

// How hard a region's sun bleaches paint and render, 0-1.
fn region_sun(region: u32) -> f32 {
    switch region {
        case REGION_ARID, REGION_NILE: {
            return 1.0;
        }
        case REGION_NORTH_AFRICA, REGION_MALTA, REGION_AEGEAN: {
            return 0.9;
        }
        case REGION_WEST, REGION_FLORIDA, REGION_ATOLL: {
            return 0.8;
        }
        case REGION_MEDITERRANEAN, REGION_HAWAII, REGION_MICRONESIA: {
            return 0.7;
        }
        case REGION_TROPICAL: {
            return 0.6;
        }
        default: {
            return 0.0;
        }
    }
}

// The tropics, where the sun stands high over every face and mould grows black.
fn region_tropical(region: u32) -> bool {
    return region == REGION_TROPICAL || region == REGION_MICRONESIA || region == REGION_ATOLL || region == REGION_HAWAII;
}

// A building's weathering, 0-1, by its hash: a past period's older; today's offices and towers kept clean.
fn building_age(in: VertexOut, kind: u32) -> f32 {
    let h = hash12(vec2<f32>(f32(in.style.z), 61.0));
    if in.past == 1u {
        return 0.45 + 0.55 * h;
    }
    return (0.15 + 0.85 * h) * select(1.0, 0.45, kind == KIND_OFFICE || kind == KIND_TOWER);
}

// A wall's weathering.
struct Grime {
    // How far dirt darkens it, 0-1: run down from its sills and its head, splashed up its foot, rising damp, soot.
    dirt: f32,
    // The dirt's share green with algae (black with mould in the tropics) rather than grey.
    green: f32,
    // Rust run down a sheet wall from its laps.
    rust: f32,
    // Paint and render bleached by the sun, 0-1.
    faded: f32,
}

// How much of a wall's dirt its material shows: pale render, stucco, concrete and stone the most, brick and painted
// boards less, a sheet's (its rust apart) and glazed tile's least.
fn takes_dirt(material: u32) -> f32 {
    switch material {
        case MATERIAL_BRICK: {
            return 0.55;
        }
        case MATERIAL_SIDING, MATERIAL_BOARDS: {
            return 0.5;
        }
        case MATERIAL_CORRUGATED: {
            return 0.35;
        }
        case MATERIAL_TILE: {
            return 0.25;
        }
        case MATERIAL_ASHLAR: {
            return 0.85;
        }
        default: {
            return 1.0;
        }
    }
}

// A wall's weathering at its mean: a wall too far off for its streaks, and Medium's walls. `n`: its face's normal;
// `takes`: how much dirt its material shows (`takes_dirt`).
fn grime_mean(n: vec3<f32>, region: u32, takes: f32, age: f32, sooty: bool) -> Grime {
    var g: Grime;
    let wet = region_wet(region);
    let tropics = region_tropical(region);
    // Northern faces (the chart's z runs south) the dampest, southern the sunniest; in the tropics every face alike.
    let north = select(smoothstep(-0.3, 0.9, -n.z), 0.55, tropics);
    let sunny = select(smoothstep(-0.3, 0.9, n.z), 0.6, tropics);
    g.dirt = clamp(age * (0.2 * takes + select(0.0, 0.105, sooty) + 0.085 * wet * north), 0.0, 0.9);
    g.green = clamp(wet * (0.2 + 0.8 * north) * select(1.0, 0.4, sooty), 0.0, 1.0);
    g.faded = region_sun(region) * sunny * age * (0.5 + 0.5 * takes);
    return g;
}

// A wall's weathering at `along` m along it and `up` m over its mean ground, `top` m under its head and `sill` m under
// the sill of the window over it (under 0: none), `under` how far it lies beneath that sill (0-1, darkest under its
// ends); `n` its face's normal; `footprint` a pixel's metres: the streaks and runs fade to their mean as they outgrow
// it (`grime_mean`).
fn wall_grime(along: f32, up: f32, top: f32, sill: f32, under: f32, n: vec3<f32>, region: u32, kind: u32, material: u32,
    age: f32, sooty: bool, seed: f32, footprint: f32) -> Grime {
    let takes = takes_dirt(material);
    var g = grime_mean(n, region, takes, age, sooty);
    let metal = material == MATERIAL_CORRUGATED;
    let wet = region_wet(region);
    let coarse = 1.0 - smoothstep(0.5, 2.5, footprint);
    if coarse <= 0.0 {
        g.rust = select(0.0, 0.15 * age * (0.4 + 0.6 * wet), metal);
        return g;
    }
    // Each 0.2 m column's streak (two octaves: most faint, a few dark), at its mean once it outgrows the pixel; a run's
    // length, longer where the wall stays wet.
    let fine = 1.0 - smoothstep(0.05, 0.22, footprint);
    let c = 0.65 * noise1(along * 5.0, seed) + 0.35 * noise1(along * 13.0 + 5.0, seed + 3.0);
    let streak = mix(0.4, smoothstep(0.3, 0.75, c), fine);
    let run = mix(0.6, 2.6, noise1(along * 1.1 + 9.0, seed + 7.0)) * (0.7 + 0.5 * wet);
    // From the head: a flat roof's parapet sheds down the wall, a pitched roof's eaves mostly clear of it.
    let flat = kind == KIND_BLOCK || kind == KIND_TOWER || kind == KIND_OFFICE || kind == KIND_INDUSTRIAL
        || kind == KIND_WAREHOUSE;
    let t = max(top, 0.0);
    let head = select(0.45, 1.0, flat) * (1.4 * streak * exp(-t / run) + 0.35 * exp(-t / 0.25));
    // Under a sill, its wash.
    var wash = 0.0;
    if sill >= 0.0 {
        wash = under * mix(0.35, 1.0, min(2.5 * streak, 1.0)) * exp(-sill / (0.3 + 0.2 * run));
    }
    // Up the foot: splashback, and in a damp country rising damp to a ragged tide line.
    let splash = exp(-max(up, 0.0) / 0.3) * (0.55 + 0.45 * noise1(along * 2.0 + 21.0, seed));
    let tide = 0.35 + 0.55 * noise1(along * 0.45 + 31.0, seed);
    let damp = 0.6 * wet * (1.0 - smoothstep(tide - 0.12, tide + 0.08, up));
    var dirt = mix(0.2, 0.9 * head + 0.8 * wash + 0.6 * splash + damp, coarse) * takes;
    // A coal town's soot, in the same runs.
    if sooty {
        dirt += 0.3 * mix(0.35, 0.3 + head + wash, coarse);
    }
    // Algae in blotches on a damp north face.
    let north = select(smoothstep(-0.3, 0.9, -n.z), 0.55, region_tropical(region));
    let blotch = smoothstep(0.55, 0.85, noise2(vec2<f32>(along, up) * 0.6, seed + 11.0));
    let algae = 0.7 * wet * north * mix(0.12, blotch, coarse);
    g.dirt = clamp(age * (dirt + algae), 0.0, 0.9);
    // A sheet's rust, run down from each lap (every 2.4 m) and its fixings.
    if metal {
        let lap = (1.0 - fract(up / 2.4)) * 2.4;
        g.rust = age * mix(0.15, 1.5 * streak * exp(-lap / 0.8), coarse) * (0.4 + 0.6 * wet);
    }
    return g;
}

// A wall's colour under its weathering.
fn weathered(albedo: vec3<f32>, g: Grime, region: u32) -> vec3<f32> {
    var c = mix(albedo, vec3<f32>(dot(albedo, LUMA) * 1.08), 0.3 * g.faded);
    let grey = vec3<f32>(0.6, 0.57, 0.52);
    let green = select(vec3<f32>(0.62, 0.7, 0.5), vec3<f32>(0.46, 0.48, 0.44), region_tropical(region));
    c *= mix(vec3<f32>(1.0), mix(grey, green, g.green), g.dirt);
    c *= mix(vec3<f32>(1.0), vec3<f32>(0.86, 0.58, 0.4), clamp(g.rust, 0.0, 1.0));
    return c;
}

// How far a point `off` from its window's middle (in the window's cell, 0-0.5) lies under the sill of a window `half`
// wide: across the sill, darkest under its ends, where the water runs off.
fn under_sill(off: f32, half: f32) -> f32 {
    let reach = half + 0.03;
    let ends = exp(-abs(off - reach + 0.02) * 30.0);
    return (1.0 - smoothstep(reach - 0.015, reach + 0.025, off)) * (0.45 + 0.55 * ends);
}

// Whether a roof's tiles are rolled (kawara, Spanish and Roman barrel tile, the North Sea's pantiles), their rolls
// running down the slope, rather than laid flat in courses.
fn rolled_tiles(entry: u32, region: u32) -> bool {
    if entry == KAWARA || entry == IBUSHI || entry == GLAZED {
        return true;
    }
    let clay = entry == 0u || entry == 1u || entry == 16u || entry == 17u || entry == 23u;
    let south = region == REGION_MEDITERRANEAN || region == REGION_MALTA || region == REGION_AEGEAN
        || region == REGION_NORTH_AFRICA || region == REGION_ARID || region == REGION_NILE || region == REGION_WEST
        || region == REGION_FLORIDA || region == REGION_HAWAII || region == REGION_NORTH_SEA;
    return clay && south;
}

// The windows at night (native/docs/terrain/buildings.md, "Lamps in the windows").

// The share of a town's rooms with a lamp lit `p` hours after 18:00 by the sun (-6 at noon to 18 at the next:
// `buildings.far.w`, `BuildingsLayer::set_sun`): most of the evening, fewer toward midnight, few in the small hours, the
// early risers' before dawn.
fn evening_share(p: f32) -> f32 {
    if p < 3.5 {
        return 0.42;
    }
    if p < 5.0 {
        return mix(0.42, 0.22, (p - 3.5) / 1.5);
    }
    if p < 7.0 {
        return mix(0.22, 0.06, (p - 5.0) / 2.0);
    }
    return mix(0.06, 0.16, smoothstep(10.5, 12.0, p));
}

// The share of shops lit: through the evening, a few windows' displays after they close.
fn shop_share(p: f32) -> f32 {
    return mix(0.75, 0.1, smoothstep(2.0, 4.0, p));
}

// The share of a lit household's rooms lit: downstairs in the evening, upstairs at bedtime, a bungalow's alike.
fn household_room(floor_index: f32, rows: f32, late: f32) -> f32 {
    if rows < 1.5 {
        return 0.55;
    }
    return select(mix(0.75, 0.3, late), mix(0.3, 0.7, late), floor_index >= 1.0);
}

// A block's stair: one bay in ten up a wall over its ground floor, on blocks of three storeys or more.
fn stair_bay(bay: f32, floor_index: f32, rows: f32, wall: f32, seed: f32) -> bool {
    return floor_index >= 1.0 && rows >= 3.0 && hash13(vec3<f32>(floor(bay), wall, seed + 97.0)) < 0.1;
}

// Whether a room's lamp burns tonight, 0-1 (a stair's landing lamp dim): by household, so a house's or a flat's rooms
// light together (a house whole, a terrace's houses two bays wide, a block's flats three bays a storey), the sitting
// rooms downstairs in the evening and the bedrooms upstairs at bedtime; a block's stair windows up one bay; a shop
// through the evening; an office's storey working late now and then; a works on its night shift; a church dark. `bay`:
// the facade's bay coordinate; `wall`: the wall's number.
fn room_lit(kind: u32, shop: bool, bay: f32, floor_index: f32, rows: f32, wall: f32, seed: f32) -> f32 {
    let p = buildings.far.w;
    let share = evening_share(p);
    let late = smoothstep(3.0, 5.5, p);
    let room = hash13(vec3<f32>(floor(bay) + wall * 1.37, floor_index, seed) + 17.0);
    if shop {
        return select(0.0, 1.0, hash12(vec2<f32>(seed + wall * 3.1, 71.0)) < shop_share(p));
    }
    if kind == KIND_HOUSE || kind == KIND_TERRACE {
        var home = seed;
        if kind == KIND_TERRACE {
            home = seed + wall * 7.3 + floor(floor(bay) * 0.5) * 0.61;
        }
        let up = hash12(vec2<f32>(home, 83.0)) < min(share * 1.8, 0.95);
        return select(0.0, 1.0, up && room < household_room(floor_index, rows, late));
    }
    if kind == KIND_BLOCK || kind == KIND_TOWER {
        if stair_bay(bay, floor_index, rows, wall, seed) {
            return select(0.0, 0.45, hash12(vec2<f32>(seed, 101.0)) < mix(0.6, 0.3, late));
        }
        let flat = floor((floor(bay) + floor(hash12(vec2<f32>(seed, 103.0)) * 3.0)) / 3.0);
        let home = hash13(vec3<f32>(flat + wall * 5.1, floor_index, seed + 113.0));
        return select(0.0, 1.0, home < min(share * 1.6, 0.95) && room < 0.62);
    }
    if kind == KIND_OFFICE {
        let working = hash12(vec2<f32>(seed + floor_index * 3.7, 107.0)) < mix(0.18, 0.04, late);
        return select(0.0, 1.0, working || room < 0.03);
    }
    if kind == KIND_INDUSTRIAL || kind == KIND_WAREHOUSE {
        return select(0.0, 1.0, hash12(vec2<f32>(seed, 109.0)) < 0.3);
    }
    if kind == KIND_CHURCH {
        return 0.0;
    }
    return select(0.0, 1.0, room < share);
}

// `room_lit`'s mean over a building's rooms of its kind and storey (for windows too small to show their own).
fn mean_lit(kind: u32, shop: bool, floor_index: f32, rows: f32) -> f32 {
    let p = buildings.far.w;
    let share = evening_share(p);
    let late = smoothstep(3.0, 5.5, p);
    if shop {
        return shop_share(p);
    }
    if kind == KIND_HOUSE || kind == KIND_TERRACE {
        return min(share * 1.8, 0.95) * household_room(floor_index, rows, late);
    }
    if kind == KIND_BLOCK || kind == KIND_TOWER {
        let stairs = select(0.0, 0.1, floor_index >= 1.0 && rows >= 3.0);
        return stairs * 0.45 * mix(0.6, 0.3, late) + (1.0 - stairs) * min(share * 1.6, 0.95) * 0.62;
    }
    if kind == KIND_OFFICE {
        return mix(0.18, 0.04, late) + 0.03;
    }
    if kind == KIND_INDUSTRIAL || kind == KIND_WAREHOUSE {
        return 0.3;
    }
    if kind == KIND_CHURCH {
        return 0.0;
    }
    return share;
}

// A dimout's shades let a little light through, blackout cloth none (`params.z`: 0 lit, 0.85 a dimout, 0.97 a
// blackout).
fn shades_through(z: f32) -> f32 {
    return 0.06 * (1.0 - smoothstep(0.86, 0.95, z));
}

// The share of a pixel `w` wide about `t` that [lo, hi] covers (a box filter: a thin line keeps its light far off).
fn thin(t: f32, lo: f32, hi: f32, w: f32) -> f32 {
    let e = max(w, 1e-4);
    return clamp((min(t + 0.5 * e, hi) - max(t - 0.5 * e, lo)) / e, 0.0, 1.0);
}

// What of a lit room's light the scenario's rule lets out: all of it from the rooms the rule leaves unshaded (the share
// it keeps), from the rest a little through a dimout's shades, none through blackout cloth but a line where its two
// halves meet, in one window in five. `u`: the point across its window's cell, 0-1, `w` the cell's share of the pixel.
fn rule_light(cell: vec3<f32>, u: f32, w: f32) -> f32 {
    let z = buildings.params.z;
    if hash13(cell + 53.0) < 1.0 - z {
        return 1.0;
    }
    let gap = 0.5 + 0.1 * (hash13(cell + 59.0) - 0.5);
    let leaks = hash13(cell + 61.0) < 0.2;
    return shades_through(z) + select(0.0, 0.9 * thin(u, gap - 0.006, gap + 0.006, w), leaks);
}

// `rule_light`'s mean.
fn rule_mean() -> f32 {
    let z = buildings.params.z;
    return (1.0 - z) + z * (shades_through(z) + 0.2 * 0.012 * 0.9);
}

// A roof: its palette colour, tiles down a pitched slope or patches on a flat one, each fading to its mean as the pixel
// outgrows it: each tile its own shade, rolled tiles' rolls with the sun across them, a ridge of capping tiles, moss and
// lichen on a damp country's north slopes, a metal roof's rust, a flat roof's stains. A smooth roof (a spire, a dome, a
// cap) takes the colour alone, a little glossy.
fn roof_surface(in: VertexOut, part: u32, kind: u32, tint: f32, footprint: f32) -> Surface {
    var s: Surface;
    let entry = in.style.y;
    var color = roof_colour(entry) * tint;
    let pitched = abs(in.normal.y) < 0.97;
    let metal = (entry >= 4u && entry <= 8u) || entry == 29u;
    let lead = entry == 21u || entry == 22u || entry == 30u || entry == 31u;
    let thatch = entry == THATCH;
    if part == PART_DOME {
        s.albedo = color;
        s.roughness = select(0.7, 0.42, lead || metal);
        s.specular = select(0.6, 1.0, lead || metal);
        s.occlusion = 1.0;
        s.emissive = vec3<f32>(0.0);
        return s;
    }
    let seed = f32(in.style.z);
    let age = building_age(in, kind);
    let wet = region_wet(in.region);
    let n = normalize(in.normal);
    // Weathered patches metres across, at their mean once they outgrow the pixel.
    let coarse = 1.0 - smoothstep(0.6, 3.0, footprint);
    let patches = noise2(in.facade * 0.45, seed);
    if pitched && thatch {
        // Reed thatch: no courses, soft streaks down the slope, darker under the ridge's cap.
        let streak = hash12(vec2<f32>(floor(in.facade.y * 3.0), f32(in.style.z)));
        let shown = 1.0 - smoothstep(0.25, 0.6, fwidth(in.facade.y * 3.0));
        color *= mix(1.0, 0.9 + 0.16 * streak, shown);
        color *= mix(0.78, 1.0, smoothstep(0.0, 0.5, in.facade.x));
    } else if pitched {
        // Tile courses (shingles a little finer, a metal sheet's ribs coarser) down the slope.
        let shingle = entry >= 18u && entry <= 20u;
        let slate = entry == 2u || entry == 25u;
        let course = in.facade.x / select(select(0.32, 0.24, shingle), 0.6, metal);
        let w = fwidth(course);
        let ribs = band(fract(course), 0.0, 0.18, w);
        let shown = 1.0 - smoothstep(0.25, 0.6, w);
        color *= mix(0.97, 1.0 - 0.18 * ribs, shown);
        if metal {
            // A painted or galvanised sheet's rust, in patches and down its ribs.
            let ribbed = smoothstep(0.6, 0.9, noise1(in.facade.y * 3.0, seed));
            let rust = age * (0.3 + 0.7 * wet) * mix(0.2, smoothstep(0.5, 0.85, patches) + 0.4 * ribbed, coarse);
            if entry != 8u {
                color = mix(color, roof_colour(8u) * tint * 0.85, clamp(rust, 0.0, 0.7));
            }
        } else {
            // Each tile its own shade (the courses broken-bond), a newer one now and then; rolled tiles' rolls down the
            // slope, the sun across them.
            let rolled = rolled_tiles(entry, in.region);
            let width = select(select(0.24, 0.3, shingle), 0.27, rolled);
            let row = floor(course);
            let across = in.facade.y / width + 0.5 * step(0.5, fract(row * 0.5));
            let tile = hash12(vec2<f32>(floor(across) + seed * 3.1, row));
            let tile_shown = shown * (1.0 - smoothstep(0.25, 0.6, fwidth(across)));
            color *= mix(1.0, select(0.93 + 0.14 * tile, 1.18, tile > 0.975), tile_shown);
            if rolled {
                let r = fract(in.facade.y / width);
                let roll_shown = 1.0 - smoothstep(0.25, 0.6, fwidth(in.facade.y / width));
                s.bump = vec2<f32>(0.5 * sin(r * 2.0 * PI), 0.0) * roll_shown;
                color *= mix(1.0, 0.86 + 0.14 * sin(r * PI), roll_shown);
            }
            // Moss and lichen on a damp country's slopes facing north (z runs south), in patches along the courses.
            let north = smoothstep(0.0, 0.8, -n.z / max(length(n.xz), 1e-3));
            let at = smoothstep(0.45, 0.8, noise2(in.facade * 0.9 + 3.0, seed + 5.0)) * (0.6 + 0.4 * ribs * shown);
            let moss = wet * age * (0.25 + 0.75 * north) * mix(0.18, at, coarse);
            let growth = select(vec3<f32>(0.05, 0.065, 0.028), vec3<f32>(0.16, 0.16, 0.12), slate || shingle);
            color = mix(color, growth, clamp(0.55 * moss, 0.0, 0.6));
        }
        // The ridge's row of capping tiles and their joints (a slate roof's in red clay in Britain and northern Europe).
        let fx = fwidth(in.facade.x);
        let cap = band(in.facade.x, -1.0, 0.2, fx);
        let clay_ridge = slate && (in.region == REGION_BRITISH || in.region == REGION_CELTIC
            || in.region == REGION_NORTH_SEA || in.region == REGION_TEMPERATE);
        let ridge = select(color * 0.82, roof_colour(1u) * tint * 0.9, clay_ridge);
        let joints = band(fract(in.facade.y / 0.45), 0.0, 0.06, fwidth(in.facade.y / 0.45)) * (1.0 - smoothstep(0.03, 0.08, fx));
        color = mix(color, ridge * (1.0 - 0.3 * joints), cap);
        color *= mix(1.0, 0.9 + 0.2 * patches, coarse);
    } else {
        // A flat roof's sheets and patches, metres across and faint; its stains, washed and blown, and the dark ones
        // where water ponds.
        let cell = floor(in.facade / 4.0);
        let mottle = hash12(cell + f32(in.style.z));
        let shown = 1.0 - smoothstep(1.0, 3.0, footprint);
        color *= mix(1.0, 0.96 + 0.08 * mottle, shown);
        let stains = 0.6 * noise2(in.facade * 0.25, seed + 4.0) + 0.4 * noise2(in.facade * 0.9, seed + 6.0);
        color *= mix(1.0, 0.84 + 0.32 * stains, coarse);
        let pond = smoothstep(0.62, 0.82, noise2(in.facade * 0.3, seed + 9.0));
        color *= 1.0 - 0.25 * age * mix(0.1, pond, coarse);
    }
    s.albedo = color;
    s.roughness = select(select(0.88, 0.97, thatch), 0.5, metal);
    s.specular = select(select(0.5, 0.25, thatch), 1.0, metal);
    // Smoked kawara's silver sheen, glazed kawara's gloss.
    if entry == IBUSHI {
        s.roughness = 0.68;
        s.specular = 0.7;
    } else if entry == GLAZED {
        s.roughness = 0.3;
        s.specular = 1.0;
    }
    s.occlusion = 1.0;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// A past period's coal-burning town: Britain's, Ireland's, the North Sea's and Europe's.
fn coal_town(in: VertexOut) -> bool {
    let sooty = in.region == REGION_BRITISH || in.region == REGION_CELTIC || in.region == REGION_NORTH_SEA
        || in.region == REGION_TEMPERATE;
    return in.past == 1u && sooty;
}

// A coal town's soot over its walls (more of it in the weathering's runs: `wall_grime`).
fn soot(in: VertexOut) -> f32 {
    return select(1.0, 0.86, coal_town(in));
}

// A past period's masonry block or office: a stone cornice along its top storey's head and the shade under it.
// `w`: a storey's share of the pixel.
fn cornice(in: VertexOut, kind: u32, part: u32, albedo: vec3<f32>, tint: f32, w: f32) -> vec3<f32> {
    let masonry = kind == KIND_OFFICE || kind == KIND_BLOCK || kind == KIND_TOWER;
    if in.past == 0u || part != PART_WALL || !masonry || in.rows < 2.0 {
        return albedo;
    }
    let d = in.facade.y - in.rows;
    let face = band(d, -0.08, 0.04, w);
    let under = band(d, -0.13, -0.08, w);
    let stone = srgb_to_linear(vec3<f32>(186.0, 178.0, 158.0)) * tint;
    return mix(albedo, stone, face) * (1.0 - 0.45 * under);
}

// The room behind a window (native/docs/terrain/buildings.md, "Rooms behind the windows").
struct Room {
    // What it sends back of the light falling on its window from outside (lit in `fs_building`).
    day: vec3<f32>,
    // What it gives off with its lamp lit, at night.
    lamp: vec3<f32>,
    // The share of it the window's dressing covers (a drape, a net, a blind): the sun falls on that whole, on the room
    // only as a patch.
    dressed: f32,
}

// A room's lamp: incandescent light of the time, a little warmer or whiter by the room, brighter or dimmer.
fn lamp_colour(h: f32) -> vec3<f32> {
    let warm = vec3<f32>(1.0, 0.66, 0.36);
    let white = vec3<f32>(1.0, 0.8, 0.56);
    return mix(warm, white, h) * (0.65 + 0.7 * fract(h * 7.31));
}

// The share of a region's windows behind net curtains: most of Britain's, Ireland's and the Low Countries' of the
// time, many of northern Europe's, few elsewhere; none in Japan.
fn net_share(region: u32) -> f32 {
    if region == REGION_BRITISH || region == REGION_CELTIC {
        return 0.6;
    }
    if region == REGION_NORTH_SEA {
        return 0.55;
    }
    if region == REGION_TEMPERATE || region == REGION_NORDIC || region == REGION_ICELAND {
        return 0.35;
    }
    if region == REGION_EAST_ASIA {
        return 0.0;
    }
    return 0.12;
}

// A room seen through a window from outside (interior mapping: Joost van Dongen, "Interior Mapping", CGI 2008): a box a
// bay wide (`size.x`, m), a storey high (`size.y`) and 3.5-6.5 m deep behind the glass, met by the ray from the eye
// (`ray`, along the wall, up, into it), its floor, ceiling, side and back walls in the room's own colours, a doorway in
// its back wall and a sideboard before it, the daylight through the window falling off into its depth; over the glass
// the window's dressing: a blind part down, net curtains, drapes drawn back or closed. `at`: the point on the glass (m
// along the bay from its left, up from the storey's floor); `cell`: the bay, the storey and the building's seed.
fn room(at: vec2<f32>, ray: vec3<f32>, size: vec2<f32>, cell: vec3<f32>, region: u32, kind: u32, lit: f32) -> Room {
    let h0 = hash13(cell + vec3<f32>(3.1, 7.7, 1.3));
    let h1 = hash13(cell + vec3<f32>(5.3, 2.9, 8.1));
    let h2 = hash13(cell + vec3<f32>(9.7, 4.1, 2.3));
    let h3 = hash13(cell + vec3<f32>(1.9, 6.3, 4.7));
    let depth = 3.5 + 3.0 * h0;
    let floor_y = 0.1;
    let ceiling_y = max(size.y - 0.3, 2.2);
    let d = select(ray, vec3<f32>(1e-4), abs(ray) < vec3<f32>(1e-4));
    let tx = (select(0.0, size.x, d.x > 0.0) - at.x) / d.x;
    let ty = (select(floor_y, ceiling_y, d.y > 0.0) - at.y) / d.y;
    let tz = depth / max(d.z, 1e-4);
    let t = max(min(min(tx, ty), tz), 0.0);
    let hit = vec3<f32>(at, 0.0) + d * t;

    // The room's colours: distemper and paper in creams, buffs, greens and greys, now and then dark panelling; a
    // boarded floor or a carpet (tatami in Japan); a white ceiling.
    var wall = vec3<f32>(0.5, 0.43, 0.31);
    if h1 < 0.18 {
        wall = vec3<f32>(0.56, 0.53, 0.44);
    } else if h1 < 0.32 {
        wall = vec3<f32>(0.33, 0.4, 0.3);
    } else if h1 < 0.44 {
        wall = vec3<f32>(0.4, 0.41, 0.4);
    } else if h1 < 0.54 {
        wall = vec3<f32>(0.44, 0.31, 0.24);
    } else if h1 < 0.61 {
        wall = vec3<f32>(0.15, 0.09, 0.05);
    } else if h1 < 0.72 {
        wall = vec3<f32>(0.3, 0.35, 0.42);
    }
    var floor_colour = vec3<f32>(0.13, 0.075, 0.04);
    if h2 < 0.25 {
        floor_colour = vec3<f32>(0.2, 0.06, 0.05);
    } else if h2 < 0.4 {
        floor_colour = vec3<f32>(0.08, 0.1, 0.06);
    }
    if region == REGION_EAST_ASIA && (kind == KIND_HOUSE || kind == KIND_TERRACE) {
        floor_colour = vec3<f32>(0.34, 0.31, 0.17);
        wall = vec3<f32>(0.5, 0.44, 0.32);
    }
    let ceiling = vec3<f32>(0.6, 0.58, 0.54);

    var albedo = wall * 0.8;
    // The daylight's share where the ray meets the room (the floor by the window the most lit).
    var light = 0.75;
    if t >= tz - 1e-3 {
        albedo = wall;
        light = 0.85;
        // A doorway in the back wall, the room beyond it darker; a sideboard or a settee along it.
        let door_x = mix(0.2, size.x - 1.1, h3);
        if hit.x > door_x && hit.x < door_x + 0.9 && hit.y < 2.05 && h3 > 0.35 {
            albedo = wall * 0.22;
        }
    } else if t >= ty - 1e-3 {
        if d.y > 0.0 {
            albedo = ceiling;
            light = 0.5;
        } else {
            albedo = floor_colour;
            light = 1.0;
        }
    }
    // A sideboard or a settee 0.6 m before the back wall, 0.8-1 m high, part of the room's width.
    let tf = (depth - 0.6) / max(d.z, 1e-4);
    let front = vec3<f32>(at, 0.0) + d * tf;
    let x0 = mix(0.05, 0.5, h2) * size.x;
    let x1 = x0 + mix(0.35, 0.8, h0) * size.x;
    if tf < t && front.y < 0.8 + 0.2 * h1 && front.x > x0 && front.x < x1 && fract(h2 * 3.3) < 0.7 {
        albedo = select(vec3<f32>(0.07, 0.045, 0.03), vec3<f32>(0.18, 0.08, 0.06), h3 > 0.7);
        light = 0.8;
    }
    let z = hit.z;
    let fall = light / (1.0 + (z / 1.8) * (z / 1.8));
    var day = albedo * fall * 0.5;
    // The lamp under the ceiling at the room's middle.
    let to_lamp = hit - vec3<f32>(0.5 * size.x, ceiling_y - 0.35, 0.5 * depth);
    let lamp = lamp_colour(h3) * albedo * (1.6 / (1.0 + 0.35 * dot(to_lamp, to_lamp))) * lit;
    var glow = lamp;

    // The dressing, at the glass. Drapes either side of the window drawn back to `open` m from its middle (closed on
    // one in eight), in the room's colour.
    let off = abs(at.x - 0.5 * size.x);
    let hd = hash13(cell + vec3<f32>(7.3, 1.1, 5.9));
    var drape_colour = vec3<f32>(0.32, 0.07, 0.05);
    if hd < 0.2 {
        drape_colour = vec3<f32>(0.1, 0.17, 0.09);
    } else if hd < 0.38 {
        drape_colour = vec3<f32>(0.45, 0.33, 0.13);
    } else if hd < 0.52 {
        drape_colour = vec3<f32>(0.1, 0.13, 0.26);
    } else if hd < 0.72 {
        drape_colour = vec3<f32>(0.55, 0.48, 0.36);
    }
    let drapes = fract(hd * 5.7) < 0.7;
    let open = select(mix(0.25, 0.75, fract(hd * 11.3)), 0.0, fract(hd * 3.1) < 0.12);
    let drape = select(0.0, smoothstep(open - 0.03, open + 0.03, off), drapes);
    // The fabric just behind the glass faces out as the wall does, and takes most of the light the wall does; with
    // the lamp lit it glows through.
    day = mix(day, drape_colour * 0.7, drape);
    glow = mix(glow, lamp_colour(h3) * drape_colour * 0.3 * lit, drape);
    // Net curtains over the glass, or its lower half (a café curtain), veiling what lies behind.
    let hn = hash13(cell + vec3<f32>(2.7, 8.3, 6.1));
    let net = select(0.0, select(0.62, 0.62 * step(at.y, 1.55), fract(hn * 3.7) < 0.3), hn < net_share(region));
    let net_colour = vec3<f32>(0.7, 0.68, 0.62);
    day = mix(day, net_colour * 0.4, net);
    glow = mix(glow, lamp_colour(h3) * 0.45 * lit, net);
    // A blind drawn part down from the head: holland or paper in cream, white or green.
    let hb = hash13(cell + vec3<f32>(4.4, 9.1, 3.3));
    let blind_bottom = mix(1.1, 2.6, fract(hb * 9.1));
    let blind = select(0.0, step(blind_bottom, at.y), hb < 0.35);
    let blind_colour = select(vec3<f32>(0.56, 0.5, 0.36), vec3<f32>(0.08, 0.15, 0.08), fract(hb * 4.3) < 0.2);
    day = mix(day, blind_colour * 0.5, blind);
    glow = mix(glow, lamp_colour(h3) * blind_colour * 0.5 * lit, blind);
    var r: Room;
    r.day = day;
    r.lamp = glow;
    r.dressed = max(max(drape, net), blind);
    return r;
}

// A room's mean, for a window too small on screen to show its room: the room's light (about its middle) under the
// region's share of net curtains.
fn room_mean(region: u32, lit: f32) -> Room {
    var r: Room;
    let share = net_share(region) * 0.75;
    r.day = mix(vec3<f32>(0.06, 0.052, 0.042), vec3<f32>(0.28, 0.272, 0.248), share * 0.62);
    r.lamp = vec3<f32>(1.0, 0.72, 0.44) * 0.3 * lit;
    r.dressed = share * 0.6;
    return r;
}

// A wall: its palette colour with windows by bay and storey, laid out by the building's kind and varied by its hash
// (window shapes and rhythm, shutters, balconies, a curtain wall, doors); lit windows at night. A plain wall, a banded
// tower, glass and painted steel take their colours alone.
fn wall_surface(in: VertexOut, part: u32, kind: u32, detail: bool, tint: f32) -> Surface {
    var s: Surface;
    let entry = in.style.x;
    var wall = wall_colour(entry) * tint;
    if part == PART_WALL || part == PART_GABLE {
        // Its weathering at its mean (no streaks on Medium).
        let brick = entry == 4u || entry == 5u || entry == 6u || entry == 249u;
        let g = grime_mean(normalize(in.normal), in.region, select(1.0, 0.55, brick), building_age(in, kind), coal_town(in));
        wall = weathered(wall, g, in.region);
    }
    let bay = in.facade.x;
    let storey = in.facade.y;
    let floor_index = floor(storey);
    let fx = fract(bay);
    let fy = fract(storey);
    let w = max(fwidth(bay), fwidth(storey));
    // The windows resolved, fading to their mean as they shrink under a pixel.
    let resolved = 1.0 - smoothstep(0.2, 0.55, w);
    let seed = f32(in.style.z);
    let variant = hash12(vec2<f32>(seed, 7.0));
    let night = buildings.sun_direction.w;
    s.occlusion = mix(0.55, 1.0, smoothstep(-0.05, 0.4, storey));
    s.emissive = vec3<f32>(0.0);
    if part == PART_GLASS {
        // A lantern (lit at night, bright), the sky in it by day; or a factory's north lights, wired and dusty glass
        // between pale glazing bars.
        if detail {
            s.albedo = vec3<f32>(0.02, 0.026, 0.03);
            s.roughness = 0.08;
        } else {
            let bars = band(fract(bay * 3.0), 0.0, 0.12, fwidth(bay * 3.0)) * resolved;
            s.albedo = mix(vec3<f32>(0.11, 0.125, 0.135), wall_colour(entry) * 0.8, bars);
            s.roughness = 0.3;
        }
        s.specular = 1.2;
        s.occlusion = 1.0;
        let lamp = select(vec3<f32>(1.0, 0.8, 0.55) * 0.12, vec3<f32>(1.0, 0.92, 0.78) * 9.0, detail);
        // Dimmed under a dimout and all but out under a blackout, as the windows (`params.z`).
        let rule = 1.0 - buildings.params.z;
        s.emissive = lamp * night * rule * rule;
        return s;
    }
    if part == PART_STEEL {
        s.albedo = wall_colour(entry);
        s.roughness = 0.5;
        s.specular = 1.0;
        s.occlusion = 1.0;
        return s;
    }
    if part == PART_BANDED {
        // Below the bands the walls' colour; up them the roof's colour and white by turns.
        let coloured = band(fract(storey * 0.5), 0.0, 0.5, fwidth(storey) * 0.5);
        let white = srgb_to_linear(vec3<f32>(222.0, 222.0, 216.0));
        s.albedo = select(wall, mix(white, roof_colour(in.style.y), coloured), storey >= 0.0);
        s.roughness = 0.8;
        s.specular = 0.6;
        s.occlusion = 1.0;
        return s;
    }
    if part == PART_GABLE {
        s.albedo = wall;
        s.roughness = 0.9;
        s.specular = 0.5;
        return s;
    }
    let valid = select(0.0, 1.0, storey >= 0.0 && floor_index < in.rows);
    var x = vec2<f32>(0.24, 0.76);
    var y = vec2<f32>(0.32, 0.8);
    var dark = vec3<f32>(0.018, 0.022, 0.028);
    var glass_rough = 0.08;
    if kind == KIND_HOUSE || kind == KIND_TERRACE {
        // Tall narrow, wide low or the plain sash.
        if variant < 0.33 {
            x = vec2<f32>(0.33, 0.67);
            y = vec2<f32>(0.24, 0.82);
        } else if variant < 0.55 {
            x = vec2<f32>(0.16, 0.84);
            y = vec2<f32>(0.36, 0.78);
        }
    }
    if kind == KIND_TOWER || kind == KIND_BLOCK {
        x = vec2<f32>(0.14, 0.86);
        y = vec2<f32>(0.26, 0.84);
        if variant < 0.3 {
            x = vec2<f32>(0.24, 0.76);
            y = vec2<f32>(0.2, 0.86);
        }
        // A ground floor of shop windows on half the blocks.
        if floor_index < 1.0 && hash12(vec2<f32>(seed, 3.0)) < 0.5 {
            x = vec2<f32>(0.06, 0.94);
            y = vec2<f32>(0.08, 0.82);
        }
    }
    if kind == KIND_OFFICE {
        if in.past == 1u {
            // A past period's masonry office: punched windows.
            x = vec2<f32>(0.3, 0.7);
            y = vec2<f32>(0.22, 0.8);
        } else if detail {
            // A curtain wall: glass from floor to floor, thin mullions and spandrels.
            x = vec2<f32>(0.04, 0.96);
            y = vec2<f32>(0.08, 0.94);
            dark = vec3<f32>(0.03, 0.045, 0.06);
            glass_rough = 0.05;
        } else {
            // Ribbons of glass across the whole bay.
            x = vec2<f32>(-0.1, 1.1);
            y = vec2<f32>(0.3, 0.82);
        }
    }
    if kind == KIND_INDUSTRIAL {
        // A ribbon of high windows on the top storey only.
        x = vec2<f32>(0.08, 0.92);
        y = vec2<f32>(0.62, 0.84);
        if floor_index < in.rows - 1.0 {
            y = vec2<f32>(2.0, 2.0);
        }
    }
    if kind == KIND_WAREHOUSE {
        // Big doors on the ground floor's bays now and then, a high ribbon on some.
        let door = hash13(vec3<f32>(floor(bay), 5.0, seed)) < 0.45;
        if floor_index < 1.0 && door {
            x = vec2<f32>(0.14, 0.86);
            y = vec2<f32>(-0.2, 0.78);
            dark = wall_colour(entry) * 0.45;
            glass_rough = 0.6;
        } else if floor_index >= in.rows - 1.0 && variant < 0.5 {
            x = vec2<f32>(0.06, 0.94);
            y = vec2<f32>(0.62, 0.82);
        } else {
            y = vec2<f32>(2.0, 2.0);
        }
    }
    if kind == KIND_CHURCH {
        if in.rows <= 1.0 {
            // The nave's tall windows.
            x = vec2<f32>(0.38, 0.62);
            y = vec2<f32>(0.18, 0.84);
        } else if floor_index >= in.rows - 1.0 {
            // The tower's belfry openings.
            x = vec2<f32>(0.28, 0.72);
            y = vec2<f32>(0.25, 0.8);
        } else {
            x = vec2<f32>(0.45, 0.55);
            y = vec2<f32>(0.35, 0.7);
        }
    }
    if kind == KIND_SHED || kind == KIND_LIGHTHOUSE {
        y = vec2<f32>(2.0, 2.0);
    }
    let mean = max(min(x.y, 1.0) - max(x.x, 0.0), 0.0) * max(min(y.y, 1.0) - min(max(y.x, 0.0), 1.0), 0.0);
    let mask = band(fx, x.x, x.y, fwidth(bay)) * band(fy, y.x, y.y, fwidth(storey));
    var glass = mix(mean, mask, resolved) * valid;
    // Each window its own: a blind drawn or a curtain now and then.
    let cell = vec3<f32>(floor(bay) + in.wall * 1.37, floor_index, seed);
    let blind = hash13(cell);
    let glass_albedo = mix(dark, vec3<f32>(0.16, 0.15, 0.13), step(0.82, blind) * resolved);
    s.albedo = mix(wall, glass_albedo, glass);
    // The rooms behind, each its own (`room` draws them with the facade textures).
    s.interior = room_mean(in.region, 0.0).day * (0.6 + 0.8 * hash13(cell + 5.0)) * glass;
    s.roughness = mix(0.9, glass_rough, glass);
    s.specular = mix(0.5, 1.2, glass);
    // Shutters either side of a house's windows (closed over one now and then), in the house's own colour.
    if detail && (kind == KIND_HOUSE || kind == KIND_TERRACE) && valid > 0.5 {
        let pick = hash12(vec2<f32>(seed, 11.0));
        var shutter = vec3<f32>(0.08, 0.16, 0.09);
        if pick < 0.25 {
            shutter = vec3<f32>(0.07, 0.12, 0.2);
        } else if pick < 0.45 {
            shutter = vec3<f32>(0.16, 0.08, 0.04);
        } else if pick < 0.6 {
            shutter = vec3<f32>(0.3, 0.3, 0.28);
        } else if pick < 0.7 {
            shutter = vec3<f32>(0.6, 0.6, 0.56);
        }
        let sw = (x.y - x.x) * 0.42;
        let side = band(fx, x.x - sw, x.x, fwidth(bay)) + band(fx, x.y, x.y + sw, fwidth(bay));
        let closed = step(hash13(cell + 29.0), 0.15);
        let leaf = (side + closed * mask) * band(fy, y.x, y.y, fwidth(storey)) * resolved;
        s.albedo = mix(s.albedo, shutter, clamp(leaf, 0.0, 1.0));
        s.roughness = mix(s.roughness, 0.8, clamp(leaf, 0.0, 1.0));
        glass *= 1.0 - closed;
    }
    // Balconies on a block's upper storeys, stacked bay over bay: a slab, a rail, the shade under it.
    let modern_balcony = in.past == 1u && in.region == REGION_EAST_ASIA;
    if detail && (kind == KIND_BLOCK || kind == KIND_TOWER) && floor_index >= 1.0 && valid > 0.5 && !modern_balcony {
        let stack = hash12(vec2<f32>(floor(bay) + seed * 0.37, 13.0)) < 0.7;
        if stack {
            let across = band(fx, 0.06, 0.94, fwidth(bay));
            let slab = band(fy, 0.0, 0.06, fwidth(storey)) * across;
            let rail = band(fy, 0.06, 0.34, fwidth(storey)) * across;
            let under = band(fy, 0.9, 1.0, fwidth(storey)) * across;
            let solid = hash12(vec2<f32>(seed, 17.0)) < 0.5;
            let rail_colour = select(vec3<f32>(0.05, 0.055, 0.06), wall * 1.05, solid);
            s.albedo = mix(s.albedo, rail_colour, rail * select(0.65, 0.95, solid) * resolved);
            s.albedo = mix(s.albedo, wall * 1.15, slab * resolved);
            s.albedo *= 1.0 - 0.35 * under * resolved;
        }
    }
    // A sill's line under each storey.
    let sill = band(fy, 0.0, 0.05, fwidth(storey)) * resolved * select(0.0, 1.0, floor_index >= 1.0 && floor_index < in.rows);
    s.albedo *= 1.0 - 0.12 * sill * (1.0 - glass);
    // Lamps at night by household and the hour (`room_lit`), under the scenario's rule, warm, their mean far off.
    let shop = (kind == KIND_TOWER || kind == KIND_BLOCK) && floor_index < 1.0 && hash12(vec2<f32>(seed, 3.0)) < 0.5;
    let lit = room_lit(kind, shop, bay, floor_index, in.rows, in.wall, seed) * rule_light(cell, fx, fwidth(bay));
    let lit_mean = mean_lit(kind, shop, floor_index, in.rows) * rule_mean() * mean;
    s.emissive = vec3<f32>(1.0, 0.74, 0.44) * 0.35 * night * mix(lit_mean, mask * lit, resolved) * valid
        * select(1.0, 0.0, kind == KIND_WAREHOUSE && floor_index < 1.0);
    if in.past == 1u {
        s.albedo = cornice(in, kind, part, s.albedo, tint, fwidth(storey));
    }
    return s;
}

#ifdef FACADE_TEXTURES
// Where the facade textures are sampled, with their screen derivatives (taken before any branch): a window's cell in
// bays and storeys (rows down), a wall material's tile in metres along the wall and up it.
struct Grads {
    cell_uv: vec2<f32>,
    cell_dx: vec2<f32>,
    cell_dy: vec2<f32>,
    wall_uv: vec2<f32>,
    wall_dx: vec2<f32>,
    wall_dy: vec2<f32>,
    // A bay's width and a storey's height on this wall (m), from the facade's coordinates against the metres.
    size: vec2<f32>,
    // Which way the bays and storeys count: 1 along the wall's own direction and up it, -1 against (a far box's
    // opposite faces count the same way across the street).
    sense: vec2<f32>,
}

// A window's half width in its cell at a bay of its kind's width (`naval_terrain::buildings::bay_m`), its trim and
// shutters in; 0 for those that run from bay to bay (a balcony's rail, a shop's front, a curtain wall, ribbons).
fn window_half(style: u32) -> f32 {
    switch style {
        case WINDOW_SASH, WINDOW_PUNCHED: {
            return 0.25;
        }
        case WINDOW_SASH_SHUTTERED, WINDOW_SHUTTERED, WINDOW_SLIDER, WINDOW_JAPAN_HOUSE, WINDOW_BLOCK,
            WINDOW_JAPAN_BLOCK: {
            return 0.34;
        }
        case WINDOW_CASEMENT, WINDOW_GRILLE, WINDOW_DOOR, WINDOW_ARCHED, WINDOW_BELFRY, WINDOW_JALOUSIE: {
            return 0.22;
        }
        case WINDOW_NORDIC, WINDOW_LOUVRES: {
            return 0.27;
        }
        case WINDOW_GARAGE, WINDOW_ROLLER, WINDOW_CARRIAGE_DOOR, WINDOW_SHOJI, WINDOW_AMADO: {
            return 0.44;
        }
        default: {
            return 0.0;
        }
    }
}

// A kind's nominal bay (m): `naval_terrain::buildings::bay_m`'s.
fn nominal_bay(kind: u32) -> f32 {
    switch kind {
        case KIND_SHED: {
            return 3.5;
        }
        case KIND_HOUSE, KIND_TERRACE, KIND_OFFICE: {
            return 3.0;
        }
        case KIND_BLOCK, KIND_TOWER: {
            return 3.3;
        }
        case KIND_INDUSTRIAL: {
            return 6.0;
        }
        case KIND_WAREHOUSE: {
            return 7.0;
        }
        case KIND_CHURCH: {
            return 4.5;
        }
        default: {
            return 4.0;
        }
    }
}

// A wall's material: its palette entry's (`WALL_MATERIALS`, `WALL_MATERIALS_EXTRA`; the colour cube's render), render
// as stucco in a warm country and as siding or boards on an American or Nordic house (and on half of East Asia's),
// painted sheet on much of Iceland's, concrete panels as render on a house and as tile on most East Asian blocks.
fn wall_material(entry: u32, region: u32, kind: u32, seed: f32) -> u32 {
    var m = MATERIAL_RENDER;
    if entry >= EXTRA_FIRST {
        m = WALL_MATERIALS_EXTRA[min(entry - EXTRA_FIRST, 3u)];
    } else if entry < 32u {
        m = WALL_MATERIALS[entry];
    }
    let house = kind == KIND_HOUSE || kind == KIND_TERRACE || kind == KIND_SHED;
    if m == MATERIAL_PANELS && house {
        m = MATERIAL_RENDER;
    }
    let pick = hash12(vec2<f32>(seed, 19.0));
    if m == MATERIAL_RENDER {
        let warm = region == REGION_MEDITERRANEAN || region == REGION_WEST || region == REGION_ARID
            || region == REGION_TROPICAL || region == REGION_FLORIDA || region == REGION_MALTA
            || region == REGION_AEGEAN || region == REGION_NORTH_AFRICA || region == REGION_NILE
            || region == REGION_HAWAII || region == REGION_MICRONESIA || region == REGION_ATOLL;
        if house && region == REGION_HAWAII && pick < 0.65 {
            // A plantation house's boards and battens, a newer house's siding.
            m = select(MATERIAL_SIDING, MATERIAL_BOARDS, pick < 0.3);
        } else if house && region == REGION_ATOLL && pick < 0.25 {
            // An atoll's corrugated sheet walls.
            m = MATERIAL_CORRUGATED;
        } else if warm {
            m = MATERIAL_STUCCO;
        } else if house && region == REGION_AMERICAN {
            m = MATERIAL_SIDING;
        } else if house && region == REGION_NORDIC {
            // Norway's and Sweden's weatherboards, boards and battens.
            m = select(MATERIAL_BOARDS, MATERIAL_SIDING, pick < 0.6);
        } else if house && region == REGION_ICELAND {
            // Iceland's painted corrugated sheet, weatherboards, concrete render.
            m = select(select(MATERIAL_RENDER, MATERIAL_SIDING, pick < 0.65), MATERIAL_CORRUGATED, pick < 0.45);
        } else if house && region == REGION_EAST_ASIA && pick < 0.5 {
            m = MATERIAL_SIDING;
        }
    }
    if m == MATERIAL_PANELS && region == REGION_EAST_ASIA && (kind == KIND_BLOCK || kind == KIND_TOWER) && pick < 0.6 {
        m = MATERIAL_TILE;
    }
    return m;
}

// The window in a bay and storey (`WINDOW_NONE`: plain wall) by the region and the building's kind, its hash and the
// bay's: a house's door, and in America a garage, on the ground floor now and then; a shop front on half the blocks'
// ground floors and balconies stacked in seven bays in ten; an office's curtain wall or ribbons; a factory's high
// ribbon; a warehouse's roller doors; a church's tall arches under its belfry.
fn window_style(region: u32, kind: u32, detail: bool, floor_index: f32, rows: f32, bay: f32, seed: f32, variant: f32) -> u32 {
    let ground = floor_index < 1.0;
    let top = floor_index >= rows - 1.0;
    let cell = hash13(vec3<f32>(floor(bay), 41.0, seed));
    if kind == KIND_HOUSE || kind == KIND_TERRACE {
        if ground && cell < 0.16 {
            return WINDOW_DOOR;
        }
        if ground && (region == REGION_WEST || region == REGION_AMERICAN || region == REGION_FLORIDA
            || region == REGION_HAWAII) && cell > 0.86 {
            return WINDOW_GARAGE;
        }
        if region == REGION_NORDIC || region == REGION_ICELAND {
            return WINDOW_NORDIC;
        }
        if region == REGION_NORTH_SEA {
            return WINDOW_CASEMENT;
        }
        if region == REGION_FLORIDA || region == REGION_HAWAII {
            return select(WINDOW_SLIDER, WINDOW_JALOUSIE, detail);
        }
        if region == REGION_MEDITERRANEAN || region == REGION_MALTA || region == REGION_AEGEAN {
            return select(WINDOW_CASEMENT, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_NORTH_AFRICA {
            return select(WINDOW_GRILLE, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_WEST {
            return WINDOW_SLIDER;
        }
        if region == REGION_EAST_ASIA {
            return WINDOW_JAPAN_HOUSE;
        }
        if region == REGION_TROPICAL || region == REGION_MICRONESIA || region == REGION_ATOLL {
            return select(WINDOW_JALOUSIE, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_ARID || region == REGION_NILE {
            return WINDOW_GRILLE;
        }
        return select(WINDOW_SASH, WINDOW_SASH_SHUTTERED, detail);
    }
    if kind == KIND_TOWER || kind == KIND_BLOCK {
        if ground && hash12(vec2<f32>(seed, 3.0)) < 0.5 {
            return WINDOW_SHOPFRONT;
        }
        if ground && cell < 0.12 {
            return WINDOW_DOOR;
        }
        let stack = hash12(vec2<f32>(floor(bay) + seed * 0.37, 13.0)) < 0.7;
        if detail && !ground && stack {
            return select(WINDOW_BALCONY, WINDOW_JAPAN_BALCONY, region == REGION_EAST_ASIA);
        }
        if region == REGION_EAST_ASIA {
            return WINDOW_JAPAN_BLOCK;
        }
        if region == REGION_MEDITERRANEAN || region == REGION_MALTA || region == REGION_AEGEAN
            || region == REGION_NORTH_AFRICA {
            return select(WINDOW_CASEMENT, WINDOW_SHUTTERED, variant < 0.5);
        }
        if region == REGION_ARID || (region == REGION_NILE && variant < 0.5) {
            return WINDOW_GRILLE;
        }
        if (region == REGION_TROPICAL || region == REGION_MICRONESIA || region == REGION_ATOLL) && variant < 0.5 {
            return WINDOW_JALOUSIE;
        }
        return WINDOW_BLOCK;
    }
    if kind == KIND_OFFICE {
        return select(WINDOW_RIBBON, WINDOW_CURTAIN, detail);
    }
    if kind == KIND_INDUSTRIAL {
        return select(WINDOW_NONE, WINDOW_HIGH_RIBBON, top);
    }
    if kind == KIND_WAREHOUSE {
        if ground && hash13(vec3<f32>(floor(bay), 5.0, seed)) < 0.45 {
            return WINDOW_ROLLER;
        }
        return select(WINDOW_NONE, WINDOW_HIGH_RIBBON, top && variant < 0.5);
    }
    if kind == KIND_CHURCH {
        return select(WINDOW_ARCHED, WINDOW_BELFRY, rows > 1.0 && top);
    }
    return WINDOW_NONE;
}

// A past period's wall material: no concrete panels, glazed tile or vinyl siding. Panels and tile become render (stucco
// in a warm country); render becomes clapboard on an American house, a third of the American West's and Florida's,
// boards or weatherboards on most of Hawaii's, Micronesia's, the tropics' and the atolls' timber houses (sheet on a few),
// stays plaster on a Japanese house (whose dark boards lie as weatherboards on half); Nordic and Icelandic houses as now.
fn wall_material_past(entry: u32, region: u32, kind: u32, seed: f32) -> u32 {
    var m = MATERIAL_RENDER;
    if entry >= EXTRA_FIRST {
        m = WALL_MATERIALS_EXTRA[min(entry - EXTRA_FIRST, 4u)];
    } else if entry < 32u {
        m = WALL_MATERIALS[entry];
    }
    let house = kind == KIND_HOUSE || kind == KIND_TERRACE || kind == KIND_SHED;
    if m == MATERIAL_PANELS || m == MATERIAL_TILE {
        m = MATERIAL_RENDER;
    }
    let pick = hash12(vec2<f32>(seed, 19.0));
    if m == MATERIAL_BOARDS && house && region == REGION_EAST_ASIA && pick < 0.5 {
        return MATERIAL_SIDING;
    }
    if m == MATERIAL_RENDER {
        let warm = region == REGION_MEDITERRANEAN || region == REGION_WEST || region == REGION_ARID
            || region == REGION_TROPICAL || region == REGION_FLORIDA || region == REGION_MALTA
            || region == REGION_AEGEAN || region == REGION_NORTH_AFRICA || region == REGION_NILE
            || region == REGION_HAWAII || region == REGION_MICRONESIA || region == REGION_ATOLL;
        if house && (region == REGION_HAWAII || region == REGION_MICRONESIA) && pick < 0.85 {
            m = select(MATERIAL_SIDING, MATERIAL_BOARDS, pick < 0.45);
        } else if house && (region == REGION_TROPICAL || region == REGION_ATOLL) && pick < 0.8 {
            m = select(MATERIAL_BOARDS, MATERIAL_CORRUGATED, pick < 0.15);
        } else if house && (region == REGION_WEST || region == REGION_FLORIDA) && pick < 0.33 {
            m = MATERIAL_SIDING;
        } else if warm {
            m = MATERIAL_STUCCO;
        } else if house && region == REGION_AMERICAN {
            m = MATERIAL_SIDING;
        } else if house && region == REGION_NORDIC {
            m = select(MATERIAL_BOARDS, MATERIAL_SIDING, pick < 0.6);
        } else if house && region == REGION_ICELAND {
            m = select(select(MATERIAL_RENDER, MATERIAL_SIDING, pick < 0.65), MATERIAL_CORRUGATED, pick < 0.45);
        }
    }
    return m;
}

// A past period's window: no aluminium sliders, curtain walls, ribbons, garage or roller doors, jalousies, Japanese
// aluminium windows or balconies. A house's door, and a carriage door in America now and then; a Japanese house's glazed
// sliding doors with their storm shutters closed over one bay in five; sash windows in the American West and Florida,
// wooden louvres in the tropics; Europe's sashes and casements as now. A block's ground floor of shops under awnings or
// signboards on half, iron balconies (none in Japan), and punched windows in stone surrounds, as an office's; a
// warehouse's wooden doors; a factory's high ribbon of steel windows and a church's arches as now.
fn window_style_past(region: u32, kind: u32, detail: bool, floor_index: f32, rows: f32, bay: f32, seed: f32, variant: f32) -> u32 {
    let ground = floor_index < 1.0;
    let top = floor_index >= rows - 1.0;
    let cell = hash13(vec3<f32>(floor(bay), 41.0, seed));
    let tropics = region == REGION_TROPICAL || region == REGION_MICRONESIA || region == REGION_ATOLL;
    let south = region == REGION_MEDITERRANEAN || region == REGION_MALTA || region == REGION_AEGEAN;
    if kind == KIND_HOUSE || kind == KIND_TERRACE {
        if region == REGION_EAST_ASIA {
            return select(WINDOW_SHOJI, WINDOW_AMADO, cell > 0.8);
        }
        if ground && cell < 0.16 {
            return WINDOW_DOOR;
        }
        if ground && (region == REGION_WEST || region == REGION_AMERICAN || region == REGION_FLORIDA
            || region == REGION_HAWAII) && cell > 0.92 {
            return WINDOW_CARRIAGE_DOOR;
        }
        if region == REGION_NORDIC || region == REGION_ICELAND {
            return WINDOW_NORDIC;
        }
        if region == REGION_NORTH_SEA {
            return WINDOW_CASEMENT;
        }
        if region == REGION_FLORIDA || region == REGION_HAWAII {
            return select(WINDOW_SASH, WINDOW_LOUVRES, detail);
        }
        if south {
            return select(WINDOW_CASEMENT, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_NORTH_AFRICA {
            return select(WINDOW_GRILLE, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_WEST {
            return select(WINDOW_SASH, WINDOW_CASEMENT, variant < 0.35);
        }
        if tropics {
            return select(WINDOW_LOUVRES, WINDOW_SHUTTERED, detail);
        }
        if region == REGION_ARID || region == REGION_NILE {
            return WINDOW_GRILLE;
        }
        return select(WINDOW_SASH, WINDOW_SASH_SHUTTERED, detail);
    }
    if kind == KIND_TOWER || kind == KIND_BLOCK {
        if ground && hash12(vec2<f32>(seed, 3.0)) < 0.5 {
            return select(WINDOW_SIGNBOARD, WINDOW_AWNING, hash12(vec2<f32>(seed, 23.0)) < 0.5);
        }
        if ground && cell < 0.12 {
            return WINDOW_DOOR;
        }
        let stack = hash12(vec2<f32>(floor(bay) + seed * 0.37, 13.0)) < 0.7;
        if detail && !ground && stack && region != REGION_EAST_ASIA {
            return WINDOW_BALCONY;
        }
        if south || region == REGION_NORTH_AFRICA {
            return select(WINDOW_CASEMENT, WINDOW_SHUTTERED, variant < 0.5);
        }
        if region == REGION_ARID || (region == REGION_NILE && variant < 0.5) {
            return WINDOW_GRILLE;
        }
        if tropics && variant < 0.5 {
            return WINDOW_LOUVRES;
        }
        if region == REGION_NORTH_SEA && variant < 0.5 {
            return WINDOW_CASEMENT;
        }
        return WINDOW_PUNCHED;
    }
    if kind == KIND_OFFICE {
        return WINDOW_PUNCHED;
    }
    if kind == KIND_INDUSTRIAL {
        return select(WINDOW_NONE, WINDOW_HIGH_RIBBON, top);
    }
    if kind == KIND_WAREHOUSE {
        if ground && hash13(vec3<f32>(floor(bay), 5.0, seed)) < 0.45 {
            return WINDOW_CARRIAGE_DOOR;
        }
        return select(WINDOW_NONE, WINDOW_HIGH_RIBBON, top && variant < 0.5);
    }
    if kind == KIND_CHURCH {
        return select(WINDOW_ARCHED, WINDOW_BELFRY, rows > 1.0 && top);
    }
    return WINDOW_NONE;
}

// The building's accent (shutters, doors, a shop's sign): green, blue, brown, grey or white by its hash, black on half of
// America's, blue on most of the Aegean's and North Africa's, red now and then on Malta's; an office's curtain wall takes
// its walls' glass colour.
fn accent_colour(region: u32, kind: u32, seed: f32, wall: vec3<f32>) -> vec3<f32> {
    if kind == KIND_OFFICE {
        return wall;
    }
    let pick = hash12(vec2<f32>(seed, 11.0));
    var c = vec3<f32>(0.08, 0.16, 0.09);
    if region == REGION_AMERICAN && pick > 0.5 {
        c = vec3<f32>(0.025, 0.025, 0.03);
    } else if (region == REGION_AEGEAN || region == REGION_NORTH_AFRICA) && pick > 0.4 {
        // The Aegean's and Tunisia's blue shutters and doors.
        c = vec3<f32>(0.03, 0.12, 0.36);
    } else if region == REGION_MALTA && pick > 0.7 {
        // A Maltese balcony's red.
        c = vec3<f32>(0.32, 0.05, 0.04);
    } else if pick < 0.25 {
        c = vec3<f32>(0.07, 0.12, 0.2);
    } else if pick < 0.45 {
        c = vec3<f32>(0.16, 0.08, 0.04);
    } else if pick < 0.6 {
        c = vec3<f32>(0.3, 0.3, 0.28);
    } else if pick < 0.7 {
        c = vec3<f32>(0.6, 0.6, 0.56);
    }
    return c;
}

// A wall from the facade textures: its material in its palette colour and, in each bay and storey, the window its region
// and kind call for over it (premultiplied), with their relief; a blind drawn now and then, a third of the windows lit
// at night, each fading to its mean through the mips as the pixel outgrows it.
fn wall_textured(in: VertexOut, part: u32, kind: u32, detail: bool, tint: f32, g: Grads, footprint: f32) -> Surface {
    var s: Surface;
    let entry = in.style.x;
    let wall = wall_colour(entry) * tint;
    let seed = f32(in.style.z);
    let region = in.region;
    var material: u32;
    if in.past == 1u {
        material = wall_material_past(entry, region, kind, seed);
    } else {
        material = wall_material(entry, region, kind, seed);
    }
    let t = textureSampleGrad(facade_walls, facade_sampler, g.wall_uv, material, g.wall_dx, g.wall_dy);
    let masonry = material == MATERIAL_BRICK || material == MATERIAL_ASHLAR || material == MATERIAL_TILE;
    let joint = select(wall * 0.45, srgb_to_linear(vec3<f32>(178.0, 174.0, 164.0)) * tint, masonry);
    var albedo = mix(wall * (t.r * 2.0), joint, t.g);
    var bump = t.ba * 2.0 - 1.0;
    let metal = material == MATERIAL_CORRUGATED;
    var roughness = select(select(0.9, 0.6, material == MATERIAL_TILE), 0.5, metal);
    var specular = select(select(0.5, 0.8, material == MATERIAL_TILE), 1.0, metal);
    let storey = in.facade.y;
    let floor_index = floor(storey);
    s.occlusion = mix(0.55, 1.0, smoothstep(-0.05, 0.4, storey));
    s.emissive = vec3<f32>(0.0);
    let n = normalize(in.normal);
    let fx = in.facade.x;
    let in_rows = part == PART_WALL && storey >= 0.0 && floor_index < in.rows;
    var style = WINDOW_NONE;
    var half = 0.0;
    var uv = g.cell_uv;
    var uv_dx = g.cell_dx;
    var uv_dy = g.cell_dy;
    var scale = 1.0;
    if in_rows {
        let variant = hash12(vec2<f32>(seed, 7.0));
        if in.past == 1u {
            style = window_style_past(region, kind, detail, floor_index, in.rows, in.facade.x, seed, variant);
        } else {
            style = window_style(region, kind, detail, floor_index, in.rows, in.facade.x, seed, variant);
        }
        // A window keeps its size in metres, centred in its bay, whatever the bay's width on this wall (a wall's bays
        // are its length over the building's bay, rounded): squeezed only as far as its bay needs, and left out of a
        // bay it would fill at less than three fifths of its width (a footprint's short jog). One that runs from bay
        // to bay is laid over the bay as it is.
        half = window_half(style);
        if half > 0.0 && style != WINDOW_NONE {
            let k = g.size.x / nominal_bay(kind);
            let fit = 2.0 * half / 0.92;
            scale = max(k, fit);
            // Past its own cell (a bay wider than the window's), the wall: the texture repeats, and would show the
            // neighbouring cell's window at the bay's edges.
            if k < 0.6 * fit || abs(fract(fx) - 0.5) * scale > 0.5 {
                style = WINDOW_NONE;
            }
            uv.x = floor(fx) + 0.5 + (fract(fx) - 0.5) * scale;
            uv_dx.x *= scale;
            uv_dy.x *= scale;
        }
    }
    // Weathering (native/docs/terrain/buildings.md, "Weathering"): dirt run down from the head and from each window's
    // sill, splashed up the foot, on the wall about the windows (their frames and glass drawn over it). A window's sill
    // lies at about three tenths of its storey, its own over a point below it, the next storey's over one above; the
    // window covers a run's start over its own sill.
    {
        var sill = -1.0;
        var under = 0.0;
        if in_rows {
            let fy = fract(storey);
            let off = abs(fract(fx) - 0.5) * scale;
            let sills = half > 0.0 && style != WINDOW_NONE && style != WINDOW_DOOR && style != WINDOW_GARAGE
                && style != WINDOW_ROLLER && style != WINDOW_CARRIAGE_DOOR;
            if fy < 0.31 && sills {
                sill = (0.31 - fy) * g.size.y;
                under = under_sill(off, half);
            } else if fy >= 0.31 && floor_index + 1.0 < in.rows {
                sill = (1.31 - fy) * g.size.y;
                under = under_sill(off, select(0.28, half, sills));
            }
        }
        let top = select(1e3, (in.rows - storey) * g.size.y, part == PART_WALL);
        let grime = wall_grime(g.wall_uv.x * WALL_TILE_M, storey * g.size.y, top, sill, under, n, region, kind, material,
            building_age(in, kind), coal_town(in), seed, footprint);
        albedo = weathered(albedo, grime, region);
    }
    {
        if style != WINDOW_NONE {
            // The windows resolved, or their mean as they shrink under a pixel.
            let width = max(abs(uv_dx.x) + abs(uv_dy.x), abs(uv_dx.y) + abs(uv_dy.y));
            let resolved = 1.0 - smoothstep(0.2, 0.55, width);
            // Set back in its reveal (deeper in masonry and render than in boards): seen at a slant, what lies back in
            // the opening (`facades`' recess: frames, glass) slides behind the opening's edge and the reveal's side
            // shows; in the sun the head and a jamb shade it. Its trim, sill, surround and shutters stand at the wall.
            // A window running from bay to bay is laid on the wall's face; far off, every window is (`resolved`).
            let along_dir = normalize(vec3<f32>(-n.z, 0.0, n.x));
            let eye = normalize(in.world - camera.position);
            let into = max(-dot(eye, n), 0.12);
            let timber = material == MATERIAL_SIDING || material == MATERIAL_BOARDS || material == MATERIAL_CORRUGATED;
            let depth = select(select(0.16, 0.08, timber), 0.0, half == 0.0) * resolved;
            let per_m = vec2<f32>(scale * g.sense.x / g.size.x, -g.sense.y / g.size.y);
            let inner_uv = uv + vec2<f32>(dot(eye, along_dir), eye.y) * per_m * (depth / into);
            let w = textureSampleGrad(facade_windows, facade_sampler, uv, style, uv_dx, uv_dy);
            let ws = textureSampleGrad(facade_window_surface, facade_sampler, uv, style, uv_dx, uv_dy);
            let accent = accent_colour(region, kind, seed, wall);
            let colour = w.rgb * mix(vec3<f32>(1.0), 2.0 * accent, clamp(ws.a / max(w.a, 1e-3), 0.0, 1.0));
            var cover = w.a;
            var glass = min(ws.b, w.a);
            var normal = ws.rg * 2.0 - 1.0;
            var window_albedo = colour;
            if depth > 0.0 {
                let back = style + WINDOW_COUNT;
                let hole = textureSampleGrad(facade_window_surface, facade_sampler, uv, back, uv_dx, uv_dy).r;
                let wi = textureSampleGrad(facade_windows, facade_sampler, inner_uv, style, uv_dx, uv_dy);
                let wsi = textureSampleGrad(facade_window_surface, facade_sampler, inner_uv, style, uv_dx, uv_dy);
                let hole_inner = textureSampleGrad(facade_window_surface, facade_sampler, inner_uv, back, uv_dx, uv_dy).r;
                // The pixel's shares: the parts at the wall's face, what lies back seen through the opening, and the
                // reveal's side where that has slid out from behind the opening's edge.
                let trim = max(w.a - hole, 0.0);
                let seen = min(hole, hole_inner);
                let side = max(hole - hole_inner, 0.0);
                let colour_inner = wi.rgb * mix(vec3<f32>(1.0), 2.0 * accent, clamp(wsi.a / max(wi.a, 1e-3), 0.0, 1.0));
                let k_trim = trim / max(w.a, 1e-3);
                let k_seen = seen / max(wi.a, 1e-3);
                window_albedo = colour * k_trim + colour_inner * k_seen + wall * 0.6 * side;
                glass = min(wsi.b, wi.a) * k_seen;
                normal = (ws.rg * 2.0 - 1.0) * trim + (wsi.rg * 2.0 - 1.0) * seen;
                cover = trim + seen + side;
                let light = buildings.sun_direction.xyz;
                let facing = dot(light, n);
                if facing > 0.02 {
                    let out_uv = inner_uv + vec2<f32>(dot(light, along_dir), light.y) * per_m * (depth / facing);
                    let open = textureSampleGrad(facade_window_surface, facade_sampler, out_uv, back, uv_dx, uv_dy).r;
                    s.sun_shade = max(hole - open, 0.0);
                }
            }
            let cell = vec3<f32>(floor(fx) + in.wall * 1.37, floor_index, seed);
            albedo = albedo * (1.0 - w.a) + window_albedo;
            bump = bump * (1.0 - cover) + normal;
            roughness = roughness * (1.0 - cover) + 0.55 * (cover - glass) + 0.04 * glass;
            specular = specular * (1.0 - cover) + 0.6 * (cover - glass) + 1.2 * glass;
            // Lit at night by household and the hour (`room_lit`), under the scenario's rule (`rule_light`).
            let shop = style == WINDOW_SHOPFRONT || style == WINDOW_AWNING || style == WINDOW_SIGNBOARD;
            let lit = room_lit(kind, shop, fx, floor_index, in.rows, in.wall, seed);
            // The room behind the glass (paper and frosted glass hide it: their lamp's glow alone).
            var r = room_mean(region, mean_lit(kind, shop, floor_index, in.rows) * rule_mean());
            if resolved > 0.01 && glass > 0.0 {
                var detailed: Room;
                if style != WINDOW_SHOJI {
                    let ray = normalize(vec3<f32>(dot(eye, along_dir) * g.sense.x, eye.y * g.sense.y, -dot(eye, n)));
                    let at = vec2<f32>(fract(fx), fract(storey)) * g.size;
                    detailed = room(at, ray, g.size, cell, region, kind, lit);
                } else {
                    detailed.day = vec3<f32>(0.0);
                    detailed.lamp = vec3<f32>(1.0, 0.72, 0.44) * 0.3 * lit;
                }
                detailed.lamp *= rule_light(cell, fract(uv.x), abs(uv_dx.x) + abs(uv_dy.x));
                r.day = mix(r.day, detailed.day, resolved);
                r.lamp = mix(r.lamp, detailed.lamp, resolved);
                r.dressed = mix(r.dressed, detailed.dressed, resolved);
            }
            s.interior = r.day * glass;
            s.interior_sun = mix(0.3, 1.0, r.dressed);
            s.emissive = r.lamp * buildings.sun_direction.w * glass;
            // Each pane set a little out of true, so the sky it shows differs window to window; old glass wavy.
            let tilt = (vec2<f32>(hash13(cell + 31.0), hash13(cell + 43.0)) - 0.5) * 0.07;
            let wave = select(0.0, 0.025, in.past == 1u) * sin(vec2<f32>(fract(fx) * 41.0 + seed, storey * 23.0));
            bump += (tilt + wave) * glass;
        }
    }
    if in.past == 1u {
        albedo = cornice(in, kind, part, albedo, tint, abs(g.cell_dx.y) + abs(g.cell_dy.y));
    }
    s.albedo = albedo;
    s.roughness = roughness;
    s.specular = specular;
    s.bump = bump;
    return s;
}
#endif

// A clipped hedge's leaves (`grounds.rs`, clutter only): its green mottled by sprays about 0.15 m and clumps about
// 0.6 m across, darker in its depths, rough, its relief leaning the light; each fading to its mean as a pixel outgrows
// it (`footprint`, metres a pixel).
fn leaf_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    let p = in.local;
    let spray = floor(p / 0.15);
    let fine = hash13(spray);
    let clump = hash13(floor(p / 0.6) + 17.0);
    let resolved = 1.0 - smoothstep(0.06, 0.25, footprint);
    let lumpy = 1.0 - smoothstep(0.3, 1.0, footprint);
    let green = wall_colour(in.style.x) * (0.85 + 0.3 * f32(in.style.z) / 255.0);
    s.albedo = green * mix(0.78, 0.5 + 0.6 * fine, resolved) * mix(1.0, 0.8 + 0.4 * clump, lumpy);
    s.roughness = 0.95;
    s.specular = 0.4;
    s.occlusion = mix(0.72, 0.45 + 0.5 * fine, resolved);
    s.emissive = vec3<f32>(0.0);
    s.bump = (vec2<f32>(hash13(spray + 3.0), hash13(spray + 7.0)) - 0.5) * 0.9 * resolved;
    return s;
}

@fragment
fn fs_building(in: VertexOut) -> @location(0) vec4<f32> {
#ifdef BUILDINGS_FADE
    if fade_hidden(in.fade, in.clip.xy) {
        discard;
    }
#endif
    let n = normalize(in.normal);
    let part = (in.style.w >> 4u) & 7u;
    let kind = in.style.w & 15u;
    let detail = (in.style.w >> 7u) == 1u;
    let seed = f32(in.style.z) / 255.0;
    // Each building its own shade: 0.88-1.12 (steel and glass as painted).
    let tint = 0.88 + 0.24 * seed;
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let footprint = max(max(length(dx), length(dy)), 0.01);
#ifdef FACADE_TEXTURES
    // Taken here, before any branch.
    var g: Grads;
    g.cell_uv = vec2<f32>(in.facade.x, -in.facade.y);
    g.cell_dx = dpdx(g.cell_uv);
    g.cell_dy = dpdy(g.cell_uv);
    let along = dot(in.local.xz, vec2<f32>(-in.normal.z, in.normal.x));
    g.wall_uv = vec2<f32>(along, -in.local.y) / WALL_TILE_M;
    g.wall_dx = dpdx(g.wall_uv);
    g.wall_dy = dpdy(g.wall_uv);
    // Metres a bay and a storey: the wall's metres along and up against its facade's coordinates, each linear over the
    // face, so the screen derivatives' ratio is exact (a degenerate face is held to sane sizes).
    let bay_grad = vec2<f32>(g.cell_dx.x, g.cell_dy.x);
    let storey_grad = vec2<f32>(g.cell_dx.y, g.cell_dy.y);
    let along_grad = vec2<f32>(g.wall_dx.x, g.wall_dy.x) * WALL_TILE_M;
    let up_grad = vec2<f32>(g.wall_dx.y, g.wall_dy.y) * WALL_TILE_M;
    let metres = vec2<f32>(
        dot(along_grad, bay_grad) / max(dot(bay_grad, bay_grad), 1e-20),
        dot(up_grad, storey_grad) / max(dot(storey_grad, storey_grad), 1e-20),
    );
    g.size = clamp(abs(metres), vec2<f32>(0.5, 2.2), vec2<f32>(12.0, 6.0));
    g.sense = select(vec2<f32>(-1.0), vec2<f32>(1.0), metres >= vec2<f32>(0.0));
#endif
    // The walls' shade: a past period's soot on them where coal burned (1 today).
    let wall_tint = tint * soot(in);
    var s: Surface;
    if part == PART_LEAF {
        s = leaf_surface(in, footprint);
    } else if part == PART_ROOF || part == PART_DOME {
        s = roof_surface(in, part, kind, tint, footprint);
#ifdef FACADE_TEXTURES
    } else if part == PART_WALL || part == PART_GABLE {
        s = wall_textured(in, part, kind, detail, wall_tint, g, footprint);
#endif
    } else {
        s = wall_surface(in, part, kind, detail, wall_tint);
    }
    let albedo = tone(s.albedo);
    // The wall's relief leans the shading normal along the wall and up it (the shadow's lookup keeps the face's).
    var shading = n;
    let along_dir = vec3<f32>(-n.z, 0.0, n.x);
    if dot(s.bump, s.bump) > 1e-6 && dot(along_dir, along_dir) > 0.01 {
        let b = clamp(s.bump, vec2<f32>(-0.9), vec2<f32>(0.9));
        let out_of = sqrt(max(1.0 - dot(b, b), 0.0));
        shading = normalize(normalize(along_dir) * b.x + vec3<f32>(0.0, b.y, 0.0) + n * out_of);
    }

    let view = normalize(camera.position - in.world);
    let light_dir = buildings.sun_direction.xyz;
    var sun = buildings.sun_radiance.rgb * sun_shadow(in.world, n) * (1.0 - s.sun_shade);
    if buildings.fill_sky.w > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, in.world);
    }
    let dot_nl = clamp(dot(shading, light_dir), 0.0, 1.0);
    let dot_nv = clamp(dot(shading, view), 1e-4, 1.0);
    let roughness = clamp(s.roughness, 0.06, 1.0);
    let f0 = vec3<f32>(0.04 * s.specular);
    let f90 = clamp(s.specular, 0.0, 1.0);
    let intensity = buildings.sun_radiance.w;
    let fill = mix(buildings.fill_ground.rgb, buildings.fill_sky.rgb, shading.y * 0.5 + 0.5);
    // The sky's light, and what the sunlit land about the building throws back onto its walls (the land's albedo about
    // 0.12, a little warm, over the half of the world below a wall's horizon): without it a shaded white wall takes the
    // sky's blue alone.
    let bounce = buildings.sun_radiance.rgb * max(light_dir.y, 0.0) * vec3<f32>(0.13, 0.12, 0.1) * (0.5 - 0.5 * shading.y);
    let irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, shading) * intensity + bounce;
    let reflected = env_radiance(sky_atlas, sky_sampler, sky_env, env_reflection_direction(shading, view, roughness), roughness) * intensity;
    let fresnel = f0 + (vec3<f32>(f90) - f0) * pow(1.0 - dot_nv, 5.0) * (1.0 - roughness * 0.7);
    let direct = dot_nl * sun * (albedo * RECIPROCAL_PI + brdf_ggx(light_dir, shading, view, f0, f90, roughness));
    let indirect = (albedo * (1.0 - fresnel) * RECIPROCAL_PI * (irradiance + fill) + reflected * fresnel) * s.occlusion;
    // The rooms behind the windows, lit by what falls on the glass from outside (the sky, the fill and the sun), seen
    // through what the glass does not reflect.
    let window_light = irradiance + fill + sun * max(dot(n, light_dir), 0.0) * s.interior_sun;
    let rooms = tone(s.interior) * window_light * RECIPROCAL_PI * (1.0 - fresnel);
    var color = direct + indirect + rooms + s.emissive;

    // The scene's aerial perspective, as the land takes it (`naval_terrain::lighting::fogged`).
    if buildings.fog.w > 0.5 {
        color = aerial_apply(color, in.world);
    }
    return vec4<f32>(color, 1.0);
}
