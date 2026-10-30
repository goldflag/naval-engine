// What covers the land: the albedo, roughness, grazing reflectance, sky occlusion and small relief of every pixel,
// from where it lies (height, slope, relief, open sky, distance to the coast), per map style; and which trees stand
// where, from the same masks.
//
// Ports the web game's `terrain/TerrainSurface.ts` (`terrainSurface`, `treeCover` and their helpers) line for line. The
// style is a shader def (`STYLE_TROPICAL`, `STYLE_SNOW`, `STYLE_VOLCANIC` or `STYLE_CHALK`; `rock` shades as chalk),
// the season and climate a uniform (`terrain.climate`: the snowline, the thaw, the leaves, a dry summer; a battle map's
// draw each style as authored, a theatre's where and when it stands), `DETAILED` is the high terrain
// setting, and `LOD_ZERO` samples every texture at its base level (the trees are placed in a compute pass, as
// the web's `footprint: 0` inputs do). Where a theatre's window has real-world surface maps (satellite colour and land
// cover, `terrain.surface`), they paint the land and place the trees instead ("Real-world surface" below); the styles'
// colours then take the global tone (`tone_down`), the photograph keeping its own.
//
// No read here takes screen derivatives of its own: each filtered read's level comes from the chart point's
// derivatives (`SurfaceInputs::chart_dx`, `chart_dy`, taken once in the land's fragment stage) carried through its
// pattern's scale and turn (`tile_uv`), and the map features' derivatives come in with `FeatureSite`. So any of it may
// run in a branch: only the classes a point holds work out their patterns, the style's own cover only where a
// photograph does not paint the point whole, and the seabed under the sea takes `seabed_surface` alone.
//
// A theatre's land (a place: `rock_faces`) draws its cliffs and steepest faces as rock of the place's own (`rock_face`,
// `materials.wgsl`'s `cliff_rock`): coloured, blotched, streaked and bedded by height, its material read face on.
#define_import_path naval_terrain::surface

#import naval_terrain::bindings::{terrain, gradients, field_uv, relief_real_share, noise_tile, detail_tile, repeat_sampler, clamp_sampler, surface_albedo, surface_cover, surface_uv, surface_bound, country_test_mark}
#import naval_terrain::features::{FeatureSite, feature_cover, feature_clearance}
#import naval_terrain::hex::{HexTaps, hex_taps, hex_blend, corner_offset, corner_offset_fast}
#import naval::math::modulo
#import naval_terrain::materials::{Mats, mats_of, mats_cover, mats_add, mats_scaled, mats_wet, mats_weight, ground_detail, materials_bound, materials_strength, Wall, flat_wall, CliffRock, cliff_rock, wall_slope, M_GRASS, M_DRY_GRASS, M_SOIL, M_SCRUB, M_DRY_SCRUB, M_FOREST, M_ROCK, M_CLIFF, M_SAND, M_WET_SAND, M_SHINGLE, M_SNOW, M_MUD, M_PAVING, M_FINE_SAND, M_NONE}

const COAST_RANGE_M: f32 = 8000.0;
const FLOW_OCTAVES: f32 = 16.0;
const RELIEF_BROAD_M: f32 = 120.0;
const RELIEF_FINE_M: f32 = 24.0;
const LN2: f32 = 0.6931471805599453;
const HALF_PI_EIGHTHS: f32 = 0.39269908169872414;

struct SurfaceInputs {
    // Chart metres and height of the point.
    chart: vec2<f32>,
    height: f32,
    // Surface gradient (∂h/∂x, ∂h/∂z).
    gradient: vec2<f32>,
    // Metres one pixel spans on the land; 0 takes every pattern at full detail.
    footprint: f32,
    // Open sky and the raw attribute bytes (0–1).
    sky: f32,
    drainage: f32,
    broad: f32,
    coast: f32,
    fine: f32,
    // The chart point's screen derivatives, and the metres over which a filtered texture is averaged there (the
    // footprint's longer axis over the sampler's 8× anisotropy, or its shorter, whichever is more): the ground
    // materials' gradients and fade (`materials.wgsl`). 0 off the land's fragment stage.
    chart_dx: vec2<f32>,
    chart_dy: vec2<f32>,
    filtered: f32,
    // How much the eye sees this ground as the floor under standing trees, from beneath their crowns (`forest.wgsl`'s
    // `floor_seen`): there the cover's leaf litter is the forest floor, not the canopy painted for a view from above.
    // 0 off the land's fragment stage and with no trees drawn (Medium).
    floor: f32,
}

struct Surface {
    albedo: vec3<f32>,
    roughness: f32,
    // Fresnel reflectance at grazing angles, 0–1.
    specular: f32,
    occlusion: f32,
    // Small relief in metres, turned into a normal by its screen-space gradient.
    bump: f32,
    // The ground materials' relief as a slope of the ground (dh/dx, dh/dz in chart metres), added to the land's own
    // gradient; and the two materials read with their weights (`mats_top`), for the debug view.
    slope: vec2<f32>,
    material: vec4<f32>,
    // How built up the land is where a theatre window's maps paint it (their cover's built-up share, 0-1): where its
    // lights burn at night (`night.wgsl`); and how much brighter the photograph is there than about it (1 alike; a
    // broad street or a yard's concrete above, a block's trees and roofs below), where they burn brighter.
    built: f32,
    photo_detail: f32,
}

// `smoothstep` by its formula: the web calls it with falling edges too, which Metal's leaves undefined.
fn ramp(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn one(x: f32) -> f32 {
    return 1.0 - x;
}

// 1 while features of `size` metres span several pixels, falling to 0 as a pixel grows to half their size.
fn resolved(footprint: f32, size: f32) -> f32 {
    return one(ramp(size * 0.12, size * 0.5, footprint));
}

// `chart` turned and shifted by an angle and offset of its own for each scale (`turned` in TerrainMaterial.ts).
fn turned(chart: vec2<f32>, metres: f32) -> vec2<f32> {
    let angle = modulo(metres * 2.399963, 2.0 * 3.141592653589793);
    let c = cos(angle);
    let s = sin(angle);
    let shift = modulo(metres * 7.31, 1000.0);
    return vec2<f32>(chart.x * c - chart.y * s + shift, chart.x * s + chart.y * c + shift * 0.61);
}

// A chart direction (one of the chart point's screen derivatives) turned as `turned` turns the chart for `metres`.
fn turned_by(d: vec2<f32>, metres: f32) -> vec2<f32> {
    let angle = modulo(metres * 2.399963, 2.0 * 3.141592653589793);
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(d.x * c - d.y * s, d.x * s + d.y * c);
}

// Texture coordinates and their screen derivatives.
struct Uv {
    at: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
}

// The chart point on a tile repeated every `metres`, turned by the angle of `turn` metres (`turned`), with its
// derivatives (zero off the land's fragment stage, where every read is at level 0).
fn tile_uv(inputs: SurfaceInputs, turn: f32, metres: f32) -> Uv {
    var uv: Uv;
    uv.at = turned(inputs.chart, turn) / metres;
    uv.dx = turned_by(inputs.chart_dx, turn) / metres;
    uv.dy = turned_by(inputs.chart_dy, turn) / metres;
    return uv;
}

fn tile(uv: Uv) -> vec4<f32> {
#ifdef LOD_ZERO
    return textureSampleLevel(noise_tile, repeat_sampler, uv.at, 0.0);
#else
    return textureSampleGrad(noise_tile, repeat_sampler, uv.at, uv.dx, uv.dy);
#endif
}

// The noise tile repeated every `metres`, turned by an angle of its own for the scale. Every channel as the tile holds
// it, for the patterns whose cells must stay whole (the parcels' boundaries and ids, the canopy's crowns); `noise_r`
// for a value with no period.
fn noise(inputs: SurfaceInputs, metres: f32) -> vec4<f32> {
    return tile(tile_uv(inputs, metres, metres));
}

// ---- Hex tiling (`hex.wgsl`) ----

fn hex_noise(uv: Uv) -> vec4<f32> {
    let t = hex_taps(uv.at);
#ifdef LOD_ZERO
    let a = textureSampleLevel(noise_tile, repeat_sampler, uv.at + t.a, 0.0);
    let b = textureSampleLevel(noise_tile, repeat_sampler, uv.at + t.b, 0.0);
    let c = textureSampleLevel(noise_tile, repeat_sampler, uv.at + t.c, 0.0);
#else
    let a = textureSampleGrad(noise_tile, repeat_sampler, uv.at + t.a, uv.dx, uv.dy);
    let b = textureSampleGrad(noise_tile, repeat_sampler, uv.at + t.b, uv.dx, uv.dy);
    let c = textureSampleGrad(noise_tile, repeat_sampler, uv.at + t.c, uv.dx, uv.dy);
#endif
    return hex_blend(t, a, b, c);
}

// The noise tile at the scale of `metres`, each triangle of the hex lattice reading it whole at its nearest corner's
// offset: for the patterns whose cells must stay whole (a parcel's id and boundary, a crown's dome), which a blend would
// smear. The lattice's edges cut through them as a field's edge or a gap in the canopy would.
fn noise_whole(inputs: SurfaceInputs, metres: f32) -> vec4<f32> {
    let uv = tile_uv(inputs, metres, metres);
    let t = hex_taps(uv.at);
    let w = t.weights;
    let offset = select(select(t.c, t.b, w.y >= w.z), t.a, w.x >= w.y && w.x >= w.z);
#ifdef LOD_ZERO
    return textureSampleLevel(noise_tile, repeat_sampler, uv.at + offset, 0.0);
#else
    return textureSampleGrad(noise_tile, repeat_sampler, uv.at + offset, uv.dx, uv.dy);
#endif
}

// The noise tile's value channel at the scale of `metres`, hex-tiled: no period, the same contrast.
fn noise_r(inputs: SurfaceInputs, metres: f32) -> f32 {
    return clamp(hex_noise(tile_uv(inputs, metres, metres)).r, 0.0, 1.0);
}

// The detail photographs' luminance (rock, grass, sand, snow; 0.5 their mean) at the scale of `metres`, hex-tiled.
fn detail(inputs: SurfaceInputs, metres: f32) -> vec4<f32> {
    let uv = tile_uv(inputs, metres * 1.7, metres);
    let t = hex_taps(uv.at);
#ifdef LOD_ZERO
    let a = textureSampleLevel(detail_tile, repeat_sampler, uv.at + t.a, 0.0);
    let b = textureSampleLevel(detail_tile, repeat_sampler, uv.at + t.b, 0.0);
    let c = textureSampleLevel(detail_tile, repeat_sampler, uv.at + t.c, 0.0);
#else
    let a = textureSampleGrad(detail_tile, repeat_sampler, uv.at + t.a, uv.dx, uv.dy);
    let b = textureSampleGrad(detail_tile, repeat_sampler, uv.at + t.b, uv.dx, uv.dy);
    let c = textureSampleGrad(detail_tile, repeat_sampler, uv.at + t.c, uv.dx, uv.dy);
#endif
    return clamp(hex_blend(t, a, b, c), vec4<f32>(0.0), vec4<f32>(1.0));
}

// What every style reads of the ground (`groundOf`).
struct Ground {
    slope: f32,
    coast: f32,
    broad: f32,
    fine: f32,
    drainage: f32,
    gully: f32,
    sunny: f32,
    wetness: f32,
    hollow: f32,
    knoll: f32,
    jag: f32,
    patches: f32,
    grain: f32,
    speck: f32,
    beach: f32,
}

fn ground_of(inputs: SurfaceInputs) -> Ground {
    var g: Ground;
    let height = inputs.height;
    let slope = length(inputs.gradient);
    g.slope = slope;
    // Signed coast distance, metres: stored as the signed square root of its share of the reach.
    let coast_byte = (inputs.coast - 0.5) * 2.0;
    g.coast = sign(coast_byte) * (coast_byte * coast_byte) * COAST_RANGE_M;
    g.patches = noise_r(inputs, 930.0);
    g.grain = noise_r(inputs, 210.0);
    g.speck = noise_r(inputs, 47.0);
    // Beaches: low, gently sloping land by the sea; the top of the beach wanders with the noise.
    let beach_top = 2.4 + (g.patches - 0.5) * 2.4 + (g.grain - 0.5);
    g.beach = one(ramp(beach_top, beach_top + 1.4, height)) * one(ramp(0.16, 0.34, slope)) * one(ramp(90.0, 260.0, g.coast));
    g.fine = (inputs.fine - 0.5) * (2.0 * RELIEF_FINE_M);
    g.broad = (inputs.broad - 0.5) * (2.0 * RELIEF_BROAD_M);
    g.drainage = inputs.drainage;
    g.gully = max(ramp(0.38, 0.55, inputs.drainage), ramp(-3.0, -10.0, g.fine)) * ramp(0.08, 0.2, slope);
    g.sunny = dot(-inputs.gradient, terrain.equator.xy) / sqrt(slope * slope + 1.0);
    g.wetness = inputs.drainage * (FLOW_OCTAVES * LN2) - log(max(slope, 0.005));
    g.jag = g.grain - 0.5 + (g.speck - 0.5) * 0.6;
    g.hollow = ramp(-1.5, -8.0, g.fine);
    g.knoll = ramp(1.5, 8.0, g.fine);
    return g;
}

// Parcels of land about `size` metres across, with their `edge`-metre boundaries (`parcels`).
struct Parcels {
    id: f32,
    boundary: f32,
    near: f32,
    edge_near: f32,
}

fn parcels(inputs: SurfaceInputs, size: f32, edge: f32) -> Parcels {
    let cells = noise_whole(inputs, size * 16.0);
    let warp = vec4<f32>(noise_r(inputs, size * 2.3));
    let width = edge / (size * 16.0) * 2.0;
    var p: Parcels;
    p.boundary = one(ramp(width * 0.5, width * 1.5, cells.b + (warp.r - 0.5) * 0.03));
    p.id = cells.a;
    p.near = resolved(inputs.footprint, size * 0.4);
    p.edge_near = resolved(inputs.footprint, edge * 3.0);
    return p;
}

// Noise stretched down the fall line on one of eight fixed headings (`fallStreaks`).
fn along_heading(inputs: SurfaceInputs, heading: f32, across: f32, along: f32) -> f32 {
    let angle = heading * HALF_PI_EIGHTHS;
    let c = cos(angle);
    let s = sin(angle);
    let u = vec2<f32>(-s, c) / (across * 8.0);
    let v = vec2<f32>(c, s) / (along * 8.0);
    var uv: Uv;
    uv.at = vec2<f32>(dot(inputs.chart, u) + heading * 0.37, dot(inputs.chart, v) + heading * 0.61);
    uv.dx = vec2<f32>(dot(inputs.chart_dx, u), dot(inputs.chart_dx, v));
    uv.dy = vec2<f32>(dot(inputs.chart_dy, u), dot(inputs.chart_dy, v));
    return tile(uv).r;
}

fn fall_streaks(inputs: SurfaceInputs, across: f32, along: f32) -> f32 {
    // Where a pixel spans a streak, the mean: no moiré at a distance (and nothing read).
    let near = resolved(inputs.footprint, across);
    if near <= 0.0 {
        return 0.5;
    }
    return mix(0.5, streaks_read(inputs, inputs.gradient, across, along, 0.0), near);
}

// The streaks as the tile holds them, down the fall line of `gradient` turned by `turn` eighths of a half turn (4: along
// the contour), for a caller that fades them itself (a pattern cut from them fades to its share, not to the cut of the
// mean).
fn streaks_read(inputs: SurfaceInputs, gradient: vec2<f32>, across: f32, along: f32, turn: f32) -> f32 {
    let fall = normalize(gradient + vec2<f32>(1e-4, 0.0));
    // The fall line's heading in eighths of a half turn, 0–8 (a streak has no sense of up and down).
    let turns = modulo(atan2(fall.y, fall.x) / HALF_PI_EIGHTHS + 16.0 + turn, 8.0);
    let first = floor(turns);
    let blend = ramp(0.0, 1.0, turns - first);
    return mix(along_heading(inputs, first, across, along), along_heading(inputs, modulo(first + 1.0, 8.0), across, along), blend);
}

// A tree canopy from above (`canopy`): `shade` multiplies the canopy colour with mean 1, `height` is the crowns'
// relief (m), `tint` a per-crown brightness with mean 1.
struct Canopy {
    shade: f32,
    height: f32,
    tint: f32,
}

fn canopy(inputs: SurfaceInputs, crown: f32, relief: f32) -> Canopy {
    var c: Canopy;
    // Where a pixel spans a crown the canopy has faded to its mean (and the clumps, a third of a crown, long before):
    // nothing to read. Nor beneath the crowns (`SurfaceInputs::floor`), where the ground is their floor: their relief
    // there, metres high, drew hard-edged banks of light and shade across it where a wood's mask ramps (a wood's field's
    // edge), and their pattern blotches.
    let near = resolved(inputs.footprint, crown) * one(inputs.floor);
    if near <= 0.0 {
        c.shade = 1.0;
        c.height = 0.0;
        c.tint = 1.0;
        return c;
    }
    let a = noise_whole(inputs, crown * 16.0);
    let b = noise_whole(inputs, crown * 16.0 * 0.63);
    let size_a = a.a * 0.45 + 0.85;
    let size_b = b.a * 0.45 + 0.8;
    let dome_a = one(pow(min(a.g / size_a, 1.0), 2.0)) * size_a;
    let dome_b = one(pow(min(b.g / size_b, 1.0), 2.0)) * size_b * 0.8;
    var clumps = 0.0;
#ifdef DETAILED
    // Leaf clumps a few metres across break every crown's outline and top.
    let clumps_near = resolved(inputs.footprint, crown * 0.3);
    if clumps_near > 0.0 {
        clumps = (noise_r(inputs, crown * 1.6) - 0.5) * clumps_near;
    }
#endif
    let dome = max(max(dome_a, dome_b) + clumps * 0.55, 0.0);
    // The pattern's mean is 0.84 over the noise tile: divided by it, the canopy keeps its brightness as it fades.
    let raw = (sqrt(min(dome, 1.0)) * 0.58 + 0.42) * mix(0.55, 1.0, ramp(0.0, 0.12, dome)) / 0.84;
    let tint = mix(b.a, a.a, ramp(-0.05, 0.05, dome_a - dome_b)) * 0.34 + 0.83;
    c.shade = mix(1.0, raw, near) * (clumps * 0.5 + 1.0);
    c.height = dome * relief * near;
    c.tint = mix(1.0, tint, near);
    return c;
}

// One of five or six crop colours by a parcel's id, with furrows on the first and the mean far away (`crops`).
struct Crops {
    color: vec3<f32>,
    furrows: f32,
}

fn crops(inputs: SurfaceInputs, land: Parcels, colors: array<vec3<f32>, 6>, count: i32) -> Crops {
    let n = f32(count);
    var palette = colors;
    var crop = palette[0];
    var mean = palette[0];
    for (var k = 1; k < count; k++) {
        let at = f32(k) / n;
        crop = mix(crop, palette[k], ramp(at - 0.005, at + 0.005, land.id));
        mean += palette[k];
    }
    mean /= n;
    let heading = land.id * 37.7;
    let along = dot(inputs.chart, vec2<f32>(cos(heading), sin(heading))) / 4.0 * (2.0 * 3.141592653589793);
    let ploughed = one(ramp(1.0 / n - 0.005, 1.0 / n + 0.005, land.id));
    let furrows = sin(along) * resolved(inputs.footprint, 4.0) * ploughed;
    crop *= furrows * 0.14 + 1.0;
#ifdef DETAILED
    // (Far off the crops are their mean colour: nothing to read.)
    if land.near > 0.0 {
        crop *= (noise_r(inputs, 90.0) - 0.5) * 0.3 + 1.0;
    }
#endif
    var out: Crops;
    out.color = mix(mean, crop, land.near);
    out.furrows = furrows;
    return out;
}

// Rock with the photograph's structure, blotches and streaks toward `stain` down the fall line (`rock`).
struct Rock {
    color: vec3<f32>,
    height: f32,
}

fn rock(inputs: SurfaceInputs, color: vec3<f32>, stain: vec3<f32>) -> Rock {
    // Each structure read only while it shows at the pixel's footprint (the photograph's within 15 m, the relief within
    // 4 m, the streaks within 6 m); the blotches always.
    let structure_near = resolved(inputs.footprint, 30.0);
    let height_near = resolved(inputs.footprint, 8.0);
    var fine_detail = 0.5;
    var structure = 1.0;
    if structure_near > 0.0 {
        fine_detail = detail(inputs, 50.0).r;
        let coarse = detail(inputs, 190.0).r;
        structure = mix(1.0, fine_detail * 1.2 + coarse * 0.8, structure_near);
    }
    let blotch = noise_r(inputs, 140.0);
    let streak_near = resolved(inputs.footprint, 12.0);
    var streak = 0.0;
#ifdef DETAILED
    if streak_near > 0.0 {
        streak = ramp(0.55, 0.8, fall_streaks(inputs, 9.0, 90.0) * 0.7 + noise_r(inputs, 30.0) * 0.3);
    }
#endif
    let tinted = mix(color * (blotch * 0.5 + 0.75), stain, streak * 0.45 * streak_near);
    var r: Rock;
    r.color = tinted * structure;
    r.height = fine_detail * 1.5 * height_near;
    return r;
}

// Cover the land layer by layer, bottom up (`layered`).
struct Layered {
    color: vec3<f32>,
    roughness: f32,
    specular: f32,
    bump: f32,
}

fn cover(below: Layered, color: vec3<f32>, roughness: f32, specular: f32, bump: f32, weight: f32) -> Layered {
    var out: Layered;
    out.color = mix(below.color, color, weight);
    out.roughness = mix(below.roughness, roughness, weight);
    out.specular = mix(below.specular, specular, weight);
    out.bump = mix(below.bump, bump, weight);
    return out;
}

fn base_cover(color: vec3<f32>, roughness: f32, specular: f32, bump: f32) -> Layered {
    var out: Layered;
    out.color = color;
    out.roughness = roughness;
    out.specular = specular;
    out.bump = bump;
    return out;
}

// three's `Color.set(hex)` for the palette constants: sRGB bytes to linear.
fn srgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    let c = vec3<f32>(r, g, b) / 255.0;
    return select(pow(c * 0.9478672986 + 0.0521327014, vec3<f32>(2.4)), c * 0.0773993808, c < vec3<f32>(0.04045));
}

// ---- Rock faces (`materials.wgsl`'s `cliff_rock`) ----

// Whether the land's cliffs are drawn in the rock of their place (a theatre's land: `terrain.cliff_corner.w`, on the High
// setting), which then stands in for the photograph and the style on its faces (`rock_face`). Medium draws them as
// before: the photograph fading to the style's rock on the slopes.
fn rock_faces() -> bool {
#ifdef DETAILED
    return terrain.cliff_corner.w > 0.5;
#else
    return false;
#endif
}

// How much of the photograph a slope keeps: on a theatre's land all of it, its rock faces drawn over it; with no place
// none past about 40 degrees, where a photograph from above sees little of a cliff (the style's rock and chalk there).
fn photo_kept(g: Ground) -> f32 {
    if rock_faces() {
        return 1.0;
    }
    return one(ramp(0.55, 0.95, g.slope + g.jag * 0.1));
}

// A rock face: how much of the point it covers, its colour in the rock's own (`color`: the rock's times its `tone`, the
// blotches, beds and streaks, to lay over the photograph's where it takes that, `photo` 0-1) and its small relief (m).
struct RockFace {
    weight: f32,
    color: vec3<f32>,
    tone: f32,
    photo: f32,
    bump: f32,
}

// The rock's beds where it stands at `level` (m, the height warped as the beds dip and wander), `thickness` metres a
// bed on average: each bed a tone of its own (0-1), from the noise tile's cells along a line through it (a cell's id
// across its width: beds of uneven thickness with sharp edges). `level_dx`, `level_dy`: the level's screen
// derivatives. `shift` moves the line, so two scales never share their beds.
fn beds(level: f32, level_dx: f32, level_dy: f32, thickness: f32, shift: f32) -> f32 {
    let along = vec2<f32>(1.0, 0.618) / (16.0 * thickness);
    var uv: Uv;
    uv.at = along * level + vec2<f32>(shift, shift * 0.37);
    uv.dx = along * level_dx;
    uv.dy = along * level_dy;
    return tile(uv).a;
}

// ---- Stepped walls (`stepped`) ----

// Layered rock weathers into steps: a hard bed stands as a cliff over the soft beds' slope beneath it, and its top is a
// bench where the soft rock above has worn back (the Grand Canyon's Kaibab and Coconino cliffs over the Toroweap and
// Hermit slopes, the Supai's ledges, the Redwall over the Muav and Bright Angel, the Tapeats over the Tonto platform).
// The window's 40 m cells draw such a wall as a smooth slope, so the steps are drawn into its normal alone (no vertex,
// height or collision changes): the height, warped as the beds are (`strata_level`), is cut into packages of
// `TERRACE_UNIT_M` on average (each between about 40 and 200 m), and across each package the slope is a bench and then
// a cliff to the next package's bench: in a hard package (two in three) a cliff of 67-74 degrees over a bench from a
// broad platform to a talus slope, the bench a steep ledge on a steep wall; in a soft one the slope barely stepped.
// Where a pixel spans a cliff band (or the slope is too gentle for one), the steps fade back into the slope.
const TERRACE_UNIT_M: f32 = 120.0;
const TERRACE_JITTER: f32 = 0.33;

// What the steps make of a point: the land's slope there times `k` (over 1 up a cliff band, under it on a bench), the
// height its beds are read at (`level`: packed up a cliff, spread across a bench), how much of a cliff band it is
// (`cliff`, 0-1), of how hard a package (`hard`, 0-1) and its cliff's shade (`shade`, -1 to 1: a formation's rock
// paler or darker than the next); `slope`, the land's own slope before them.
struct Terrace {
    k: f32,
    level: f32,
    cliff: f32,
    hard: f32,
    shade: f32,
    slope: f32,
}

fn flat_steps(level: f32, slope: f32) -> Terrace {
    return Terrace(1.0, level, 0.0, 0.0, 0.0, slope);
}

// Two values 0-1 for a package (its index), from an integer hash (PCG's output step).
fn package_hash(i: f32) -> vec2<f32> {
    var h = bitcast<u32>(i32(i)) * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return vec2<f32>(f32(h & 65535u), f32(h >> 16u)) / 65535.0;
}

// The level a package `i` starts at (m): `TERRACE_UNIT_M` apart on average, each moved up or down by a third of it.
fn package_floor(i: f32) -> f32 {
    return (i + (package_hash(i).x - 0.5) * 2.0 * TERRACE_JITTER) * TERRACE_UNIT_M;
}

// The height `height` warped as the beds dip and wander (`beds`, `stepped`): tilted by `dip` (m, `strata_dip`), by the
// broad noise some tens of metres over a kilometre and by the finer a little, so neither the beds nor the steps follow
// the contours exactly.
fn strata_level(height: f32, g: Ground, bed_m: f32, dip: f32) -> f32 {
    let thickness = max(bed_m, 0.5);
    return height + dip + (g.patches - 0.5) * (thickness * 6.0 + 24.0) + (g.grain - 0.5) * thickness * 1.2;
}

// How far the beds at a point stand above the level they would have flat (m): the strata warped over the country by a
// broad noise, 1,600 m over 40 km and 400 m over 12 km, so they dip a degree or two on average and up to about six,
// their dip's direction wandering (a cone's beds cross it rather than ring it; a plateau's lie nearly flat).
fn strata_dip(inputs: SurfaceInputs) -> f32 {
    return (noise_r(inputs, 40000.0) - 0.5) * 1600.0 + (noise_r(inputs, 12000.0) - 0.5) * 400.0;
}

// The steps at a point whose warped height is `level` (m), `level_px` metres of it to a pixel, on land of slope `slope`
// (before the steps), `lateral` the land's 210 m noise there, `weight` of it stepped (0-1: the rock layered, bare and
// steep enough; `terrain_surface`).
fn stepped(level: f32, level_px: f32, slope: f32, lateral: f32, weight: f32) -> Terrace {
    var t = flat_steps(level, slope);
    if weight <= 0.0 {
        return t;
    }
    // The package the level stands in: between the floors of `i` and `i + 1` (the jitter moves each by under half).
    let i = floor(level / TERRACE_UNIT_M);
    var id = i;
    var low = package_floor(i);
    var high = package_floor(i + 1.0);
    if level < low {
        id = i - 1.0;
        high = low;
        low = package_floor(id);
    } else if level >= high {
        id = i + 1.0;
        low = high;
        high = package_floor(id + 1.0);
    }
    let thickness = high - low;
    let u = (level - low) / thickness;
    let h = package_hash(id + 7919.0);
    let h2 = package_hash(id + 104729.0);
    let hard = ramp(0.28, 0.4, h.x);
    // The cliff's face and its bench's slope: a hard package's cliff at 2.4-3.4 (67-74 degrees) over a bench from a
    // broad platform to a talus slope (0.1-0.6 of the wall's slope), never gentler than a ledge on a steep wall (0.65,
    // 33 degrees, as the wall nears 1.2, 50); a soft package's slope barely stepped (1.3 and 0.7 of it).
    let ledge = 0.65 * ramp(0.5, 1.2, slope);
    let face = mix(slope * 1.3, max(2.4 + h.y, slope * 1.2), hard);
    let bench = mix(slope * 0.7, max(slope * mix(0.1, 0.6, h2.y), ledge), hard);
    // The cliff's share of the slope's width, and the slope's factor on the bench and the cliff (their mean 1, so the
    // wall keeps its slope); `a`, the cliff's share of the package's height.
    let r = clamp((slope - bench) / max(face - bench, 1e-3), 0.03, 0.97);
    let k_bench = bench / max(slope, 1e-3);
    let k_cliff = face / max(slope, 1e-3);
    let a = r * k_cliff;
    // Across the package, of its width a pixel spans; the cliff's foot soft (its talus over a quarter of its width),
    // its rim sharp, both never sharper than the pixel.
    let du = level_px / max(thickness, 1.0);
    let foot = 1.0 - r;
    let soft = max(r * 0.25, du);
    let sharp = max(r * 0.05, du);
    var cliff = ramp(foot - soft, foot + soft * 0.25, u) * (1.0 - 0.5 * ramp(1.0 - sharp, 1.0, u));
    cliff = max(cliff, 0.5 * ramp(sharp, 0.0, u));
    // The steps fade back into the slope as a pixel spans the cliff band; and each package's cliff runs out along the
    // wall here and there (where the land's 210 m noise, `lateral`, is under a cut of the package's own), as a
    // formation thins and a face breaks (the more on a steep wall), so no band rings a whole mountain.
    let cut = mix(0.1, 0.5, h2.x) + 0.25 * ramp(0.8, 1.4, slope);
    let shown = weight * ramp(0.6, 2.0, r / max(du, 1e-5)) * ramp(cut - 0.08, cut + 0.08, lateral);
    t.k = mix(1.0, mix(k_bench, k_cliff, cliff), shown);
    // The stepped height through the package (piecewise: the bench, then the cliff), for the beds.
    let p = select((1.0 - a) + (u - foot) * k_cliff, u * k_bench, u < foot);
    t.level = mix(level, low + thickness * p, shown);
    t.cliff = cliff * shown;
    t.hard = hard;
    t.shade = package_hash(id + 104729.0).x * 2.0 - 1.0;
    return t;
}

// Where a theatre's land stands as a rock face (`rock_faces`), and how it looks. The faces: the sea's cliffs (the land
// within a few hundred metres of the sea and above its beach) from about 23 degrees at the water's edge to about 38 a
// few hundred metres up the slope, faces inland from about 49 (a slope short of that holds soil and growth), either
// steeper where the cover grows (`growth`: its woods and scrub, x, and its grass and fields, y, 0-1; by the sea mostly
// the woods and scrub, grass topping most sea cliffs), the edge sharp and broken by a fine noise where a pixel holds
// it; and where the ground is bare rock (`exposed`, 0-1: the cover's bare ground, its moss and lichen over the
// treeline), from about 33 degrees. The slope is the stepped one (`stepped`): a cliff band counts as steep, its bench
// as gentle. The rock (`rock`, `cliff_rock`): its colour blotched at a few scales, banded by its beds where they show
// (packages of beds and the beds within them, faded as a pixel spans them, a hard package's cliff massive, the bedding
// clearer in some stretches of a wall than others), streaked dark down the fall line; the beds standing out a little
// from the face. Its colour is the photograph's where that saw the slope from above well enough (`photo`: the slope
// before the steps under about 45 degrees), the rock's own past about 52.
fn rock_face(inputs: SurfaceInputs, g: Ground, growth: vec2<f32>, rock: CliffRock, steps: Terrace, exposed: f32)
    -> RockFace {
    var out: RockFace;
    out.weight = 0.0;
    out.color = vec3<f32>(0.0);
    out.tone = 1.0;
    out.photo = 0.0;
    out.bump = 0.0;
    let height = inputs.height;
    let footprint = inputs.footprint;
    let sea = one(ramp(120.0, 450.0, g.coast + g.jag * 250.0)) * ramp(3.0, 9.0, height + g.jag * 4.0);
    let start = mix(1.15 + (growth.x + growth.y * 0.5) * 0.6, 0.42 + growth.x * 0.5 + growth.y * 0.12
        + ramp(60.0, 400.0, g.coast) * 0.35, sea);
    let bare_start = select(9.0, 0.62, exposed > 0.0);
    let rough = g.slope + g.jag * 0.25;
    if rough < min(start, bare_start) - 0.2 {
        return out;
    }
    var edge = 0.0;
    let edge_near = resolved(footprint, 9.0);
    if edge_near > 0.0 {
        edge = (noise_r(inputs, 9.0) - 0.5) * 0.4 * edge_near;
    }
    let wall = ramp(start, start + 0.12, rough + edge);
    let bare = exposed * ramp(bare_start, bare_start + 0.14, rough + edge);
    out.weight = max(wall, bare) * ramp(0.2, 1.5, height);
    if out.weight <= 0.0 {
        return out;
    }
    out.photo = 1.0 - ramp(1.0, 1.3, steps.slope + g.jag * 0.25);
    // Blotches: weathering, lichen and stain, from the land's own noise and a finer one close up.
    var tone = (g.grain - 0.5) * 0.3 + (g.speck - 0.5) * 0.3 + 1.0;
    let fine_near = resolved(footprint, 14.0);
    if fine_near > 0.0 {
        tone *= (noise_r(inputs, 14.0) - 0.5) * 0.35 * fine_near + 1.0;
    }
    // The beds, at the stepped level (`stepped`: packed up a cliff band, spread across its bench), where the bedding
    // shows: a hard package's cliff is massive, and the bedding clearer in some stretches of a wall than others. Each
    // package a shade of its own.
    let thickness = max(rock.bed_m, 0.5);
    let level = steps.level;
    let level_dx = dot(inputs.gradient, inputs.chart_dx);
    let level_dy = dot(inputs.gradient, inputs.chart_dy);
    if rock.strata > 0.0 {
        let clear = rock.strata * mix(1.0, 0.4, steps.cliff * steps.hard) * (ramp(0.4, 0.75, g.patches) * 0.8 + 0.2);
        tone *= steps.shade * 0.14 * steps.cliff + 1.0;
        let packages_near = resolved(footprint, thickness * 6.0);
        if packages_near > 0.0 {
            let p = beds(level, level_dx, level_dy, thickness * 6.0, 0.0) - 0.5;
            tone *= p * 0.7 * clear * packages_near + 1.0;
            out.bump += p * thickness * 0.35 * packages_near;
        }
        let beds_near = resolved(footprint, thickness);
        if beds_near > 0.0 {
            let b = beds(level, level_dx, level_dy, thickness, 0.43) - 0.5;
            tone *= b * 0.45 * clear * beds_near + 1.0;
            out.bump += b * thickness * 0.1 * beds_near;
        }
    }
    // Stains and runnels down the fall line.
    let streak_near = resolved(footprint, 20.0);
    if streak_near > 0.0 {
        let streak = ramp(0.55, 0.85, fall_streaks(inputs, 10.0, 80.0) * 0.75 + noise_r(inputs, 23.0) * 0.25);
        tone *= mix(1.0, 0.78, streak * streak_near);
    }
    // Where the rock keeps the style's own colours (no place), the land's colour shows through.
    out.tone = max(tone, 0.2);
    out.color = rock.colour.rgb * out.tone;
    out.weight *= rock.colour.w;
    return out;
}

// ---- Land use per style, shared by the ground's paint and the trees ----

struct SnowMasks {
    snow: f32,
    bare: f32,
    woods: f32,
    tidal: f32,
    shore: f32,
}

// Late snow below the snowline lies in patches that thin out over a broad band of their measure `lying` (0.5 its
// middle), not to a line or in blots (`climate_snow`, `snow_masks`). The drifts move the measure (a 420 m noise, its
// cells some 50 m across, by a standard deviation of 0.05; its mean, the tile's 0.535, taken out, and 0.05 more so
// that as much snow lies as before), and the share of the ground the patches cover rises from none 0.2 under the
// middle to all of it 0.2 over. The patches stand where a fine noise (40 m: the hummocks and the wind's scour, its
// cells some 5 m across) is over its quantile for that share (as `floor_patches`), edged over a metre or so; as a
// pixel grows past them, the share itself, a thinning cover. So from 1-3 km the thaw reads as a soft edge of finely
// broken patches. (Until 2026-10-27 the patches ended within 0.06 of their measure: at Narvik in late May a hard edge
// ran some 2 km straight across a wooded slope. That morning the drifts' noise and an 85 m one were added to the
// measure with the edge as hard, and Kirkenes' snow broke into hard-edged black blots 100-200 m across.)
fn lying_snow(inputs: SurfaceInputs, lying: f32) -> f32 {
    // (The drifts move the measure by -0.21 to 0.19: past the band by more, their noise is not read.)
    let measure = lying - 0.05;
    if measure < 0.114 {
        return 0.0;
    }
    if measure > 0.914 {
        return 1.0;
    }
    let share = ramp(0.3, 0.7, measure + (noise_r(inputs, 420.0) - 0.535) * 0.4);
    let near = resolved(inputs.filtered, 6.0);
    if share <= 0.0 || share >= 1.0 || near <= 0.0 {
        return share;
    }
    let quantile = 0.535 - 0.07 * log(share / (1.0 - share));
    return mix(share, ramp(quantile - 0.05, quantile + 0.05, noise_r(inputs, 40.0)), near);
}

// Vestfjord in April: the snowline, bare steep faces between couloirs, birch below the treeline, the tidal band. The
// snowline is the climate's (230 m as authored); the thaw clears the snow lying in patches below it.
fn snow_masks(inputs: SurfaceInputs, g: Ground) -> SnowMasks {
    let height = inputs.height;
    let snowline = terrain.climate.x + g.sunny * 220.0 - g.hollow * 60.0 + g.knoll * 25.0 + g.jag * 110.0;
    let above = ramp(snowline - 20.0, snowline + 20.0, height);
    var m: SnowMasks;
    m.shore = one(ramp(3.0, 22.0, height)) * one(ramp(40.0, 350.0, g.coast));
    let lying = ramp(snowline - 260.0, snowline, height) * 0.35 + 0.5 + g.hollow * 0.25 - g.knoll * 0.2 - g.sunny * 0.9
        - m.shore * 0.45 + g.jag * 0.6 - terrain.climate.y * 0.65;
    m.snow = max(above, lying_snow(inputs, lying));
    let chutes = max(ramp(0.56, 0.64, fall_streaks(inputs, 45.0, 420.0)), ramp(-3.0, -9.0, g.fine) * 0.8);
    // The steep faces bare between them, from a gentler slope in the melt (`snow_shed`).
    let melt = snow_melt(height, snowline) * 0.1;
    m.bare = ramp(0.66 - melt, 0.86 - melt, g.slope + g.jag * 0.15) * one(chutes * 0.8);
    let treeline = 320.0 + g.sunny * 150.0 + g.jag * 80.0;
    let shelter = 0.45 + g.hollow * 0.25 - g.knoll * 0.25 + ramp(4.0, 9.0, g.wetness) * 0.2 + g.jag * 0.5;
    m.woods = one(ramp(treeline - 90.0, treeline, height)) * ramp(4.0, 16.0, height) * one(ramp(0.6, 0.85, g.slope))
        * ramp(40.0, 180.0, g.coast) * ramp(0.35, 0.55, shelter);
    m.tidal = one(ramp(1.2, 3.2, height + g.jag * 2.0));
    return m;
}

struct ChalkMasks {
    cliff: f32,
    foot: f32,
    fields: Parcels,
    gentle: f32,
    woods: f32,
    hedges: f32,
}

fn chalk_masks(inputs: SurfaceInputs, g: Ground) -> ChalkMasks {
    let height = inputs.height;
    var m: ChalkMasks;
    // Cliffs: the faces the sea has cut, steep and within a few hundred metres of it.
    m.cliff = ramp(0.3, 0.55, g.slope + g.jag * 0.12) * one(ramp(250.0, 700.0, g.coast)) * ramp(3.0, 10.0, height);
    // The foot of the coast: shingle and fallen chalk within a stone's throw of the sea.
    m.foot = one(ramp(50.0, 130.0, g.coast + g.jag * 50.0)) * one(ramp(14.0, 30.0, height));
    // Smaller fields where the summer is dry: terraces and walled plots, not the open fields of the north.
    m.fields = parcels(inputs, mix(260.0, 130.0, terrain.climate.w), mix(5.0, 2.5, terrain.climate.w));
    m.gentle = one(ramp(0.12, 0.22, g.slope)) * ramp(6.0, 18.0, height);
    let hanger = ramp(0.13, 0.24, g.slope + g.jag * 0.08) * one(ramp(0.4, 0.6, g.slope));
    let copse = ramp(8.0, 10.5, g.wetness + g.jag * 2.0) * (g.hollow * 0.5 + 0.5);
    let wood_field = ramp(0.93, 0.94, m.fields.id) * m.fields.near * one(g.knoll * 0.5);
    m.woods = max(max(hanger, copse * 0.9), wood_field) * one(m.cliff) * ramp(6.0, 20.0, height) * ramp(60.0, 200.0, g.coast);
    m.hedges = m.fields.boundary * m.fields.edge_near * m.gentle * one(m.woods);
    return m;
}

struct VolcanicMasks {
    plain: f32,
    plots: Parcels,
    grove: f32,
    palms: f32,
    steep: f32,
}

fn volcanic_masks(inputs: SurfaceInputs, g: Ground) -> VolcanicMasks {
    let height = inputs.height;
    var m: VolcanicMasks;
    // Paddies: the flat, wet lowland plains and valley floors.
    m.plain = one(ramp(0.03, 0.07, g.slope)) * one(ramp(40.0, 120.0, height + g.jag * 30.0)) * ramp(3.0, 7.0, height)
        * (ramp(4.5, 7.0, g.wetness + g.jag * 1.5) * 0.4 + 0.6);
    m.plots = parcels(inputs, 80.0, 1.5);
    // Village groves fill whole plots among the paddies, more of them on the slightly raised, drier ground.
    m.grove = ramp(0.8, 0.82, m.plots.id + (g.speck - 0.5) * 0.02 + one(g.hollow) * 0.06) * m.plots.near + one(m.plots.near) * 0.2;
    // Coconut groves: the coastal strip behind the beach, in the lowlands.
    m.palms = one(ramp(8.0, 25.0, height + g.jag * 6.0)) * one(ramp(350.0, 1100.0, g.coast + g.jag * 300.0)) * ramp(2.5, 4.0, height);
    m.steep = ramp(1.05, 1.5, g.slope + g.jag * 0.3);
    return m;
}

struct TropicalMasks {
    montane: f32,
    gully: f32,
    kunai: f32,
    plantation: f32,
    scar: f32,
}

fn tropical_masks(inputs: SurfaceInputs, g: Ground) -> TropicalMasks {
    let height = inputs.height;
    var m: TropicalMasks;
    m.montane = ramp(900.0, 1500.0, height + g.jag * 150.0);
    // Gallery forest follows every gully and stream, and the wet feet of slopes.
    m.gully = max(ramp(6.5, 9.0, g.wetness + g.jag * 2.0), g.hollow);
    // Kunai: the dry, open ground of the coastal plain and the crests of the low foothill ridges, never in a gully. A
    // satellite sees the islands almost wholly under forest, the grass in a few broad tracts of the northern plains and
    // foothills: so it stands only where the broad noise (930 m) opens a tract, and there in soft-edged patches, the
    // ridges' grass rarer still. (It once covered most flat ground in hard-edged blobs, half the plain or more.)
    let tract = ramp(0.5, 0.72, g.patches);
    let plain = one(ramp(0.03, 0.08, g.slope)) * one(ramp(25.0, 60.0, height)) * ramp(4.0, 8.0, height);
    let ridges = one(ramp(140.0, 300.0, height + g.jag * 60.0)) * ramp(0.8, 1.15, g.knoll + g.broad / 240.0 + g.jag * 0.4)
        * one(ramp(0.3, 0.5, g.slope)) * one(ramp(2500.0, 5000.0, g.coast));
    m.kunai = clamp(max(plain * ramp(0.35, 0.85, g.jag + 0.55 - g.hollow), ridges) * tract * one(m.gully)
        * ramp(4.0, 10.0, height), 0.0, 1.0);
    // Plantations: rows of palms in whole blocks on the flat strip behind the beaches.
    let blocks = parcels(inputs, 420.0, 6.0);
    m.plantation = one(ramp(8.0, 22.0, height)) * one(ramp(0.04, 0.09, g.slope)) * one(ramp(900.0, 2000.0, g.coast))
        * ramp(0.3, 0.32, blocks.id) * one(m.gully * 0.7);
    m.scar = ramp(1.15, 1.5, g.slope + g.jag * 0.4) * ramp(150.0, 400.0, height);
    return m;
}

// Where the tropical and volcanic styles' trees stand (`tree_cover`), which the ground under them follows: the chance a
// tree stands on a spot (x) and the share of those that are palms (y).
fn tropical_stand(inputs: SurfaceInputs, g: Ground, m: TropicalMasks) -> vec2<f32> {
    let height = inputs.height;
    // The coconut palms along the shore, behind the beach.
    let fringe = one(ramp(120.0, 350.0, g.coast)) * ramp(1.8, 4.0, height) * one(ramp(10.0, 25.0, height));
    let forest = one(m.kunai) * one(m.plantation) * one(m.scar);
    let density = max(forest * 0.95, max(m.plantation * 0.85, max(fringe * 0.75, m.kunai * 0.03))) * ramp(1.5, 3.0, height);
    return vec2<f32>(density, max(m.plantation, fringe));
}

fn volcanic_stand(inputs: SurfaceInputs, g: Ground, m: VolcanicMasks) -> vec2<f32> {
    let height = inputs.height;
    // No tree on the beach, low by the sea or on the steepest slopes.
    let land = ramp(2.5, 5.0, height) * one(g.beach) * one(ramp(0.45, 0.6, g.slope));
    let fringe = one(ramp(150.0, 400.0, g.coast)) * ramp(2.0, 4.0, height) * one(ramp(12.0, 25.0, height));
    let density = max(one(m.plain) * one(m.steep) * 0.95, max(m.plain * m.grove * 0.85, max(m.palms * 0.7, fringe * 0.8))) * land;
    return vec2<f32>(density, max(max(m.palms, fringe), m.plain * m.grove * 0.5));
}

// The ground a wood's leaf litter covers in those styles: where its broadleaf trees stand close (a stand from
// `tropical_stand`, `volcanic_stand`), its edge as soft as their cover's; the palms stand on grass and sand.
fn under_broadleaves(stand: vec2<f32>) -> f32 {
    return ramp(0.3, 0.8, stand.x) * one(stand.y);
}

// ---- The forest floor ----

// What lies under a wood's trees (`litter_color`, and the forest material's read: `materials.wgsl`'s `floor_read`): the
// share of it that is needles (a conifer wood's; the rest a broadleaf wood's leaves), and how dry and dusty (a dry
// summer's woods) and how wet and dark (the rainforest's) it is, 0-1 each.
struct Floor {
    needles: f32,
    dry: f32,
    wet: f32,
}

// The share `share` of a wood's floor that is needles, in patches (under each group of its conifers) rather than all of
// it at that share: where the noise `n` (the tile's: mean 0.535, standard deviation 0.119) falls below its quantile for
// that share, a logistic's stand-in for the normal's, edged over a few metres.
fn floor_patches(share: f32, n: f32) -> f32 {
    if share <= 0.0 || share >= 1.0 {
        return clamp(share, 0.0, 1.0);
    }
    let s = clamp(share, 0.01, 0.99);
    let quantile = 0.535 + 0.07 * log(s / (1.0 - s));
    return 1.0 - smoothstep(quantile - 0.02, quantile + 0.02, n);
}

// The biome at a chart point (`terrain.biomes`, the trees' grid: `tree_place.wgsl`'s `biome_at`), the lookup moved by
// the broad noise so a biome's edge on land is no line of squares; -1 with no grid (a battle map, its style's trees).
fn biome_here(chart: vec2<f32>, g: Ground) -> f32 {
    if terrain.biome_grid.w < 0.5 {
        return -1.0;
    }
    let jitter = vec2<f32>(g.patches - 0.535) * 3.0;
    let cell = clamp(vec2<i32>(floor((chart - terrain.biome_grid.xy) / terrain.biome_grid.z + jitter)), vec2<i32>(0),
        vec2<i32>(BIOME_CELLS - 1));
    let k = u32(cell.y * BIOME_CELLS + cell.x);
    return f32((terrain.biomes[k / 16u][(k / 4u) % 4u] >> (8u * (k % 4u))) & 255u);
}

// Cells a side of the biomes' grid (`species::REGION_CELLS`).
const BIOME_CELLS: i32 = 16;

// ---- The boreal woods' make-up, shared by the trees (`tree_place.wgsl`'s `choose`) and their floor (`floor_at`) ----

// Metres of the chart a degree of latitude spans (`species::chart_place`).
const DEGREE_M: f32 = 111195.0;

// Where chart point `chart` stands on the earth (latitude, longitude; degrees), the chart's origin at `terrain.place`
// (`tree_place.wgsl`'s `latitude_at` and `longitude_at` from the same place).
fn place_at(chart: vec2<f32>) -> vec2<f32> {
    let lat = terrain.place.x - chart.y / DEGREE_M;
    return vec2<f32>(lat, terrain.place.y + chart.x / (DEGREE_M * max(cos(radians(terrain.place.x)), 0.05)));
}

// The boreal forest's upper edge (m) by the latitude alone: about 1,150 m at 61 degrees (Jotunheimen, Dovre), 950 m at
// 63.5 (Trondelag), 550 m at 68.5 (Ofoten, Troms), 430 m at 70, some 350 m under the summer's snowline. (Until
// 2026-10-27 it was the scenario's own snowline, so the birch belt rose and fell with the date: at Narvik it began at
// 134 m in late May and 706 m in August, and in winter there was none.)
fn boreal_treeline(lat: f32) -> f32 {
    return 750.0 + (66.0 - abs(lat)) * 80.0;
}

// The share of a boreal wood that is the mountain birch's belt under the treeline (`ground` the point's): none below
// about 350 m under the line, all from about 150 m under it, the band's middle higher on slopes facing the sun, lower in
// the hollows the cold air fills, and wandering some 60 m of height either way with a broad noise (up the gullies,
// down the spurs), so the conifers give way to the birch over a slope's length rather than at a contour. Each tree
// draws its own chance against it.
fn birch_belt(inputs: SurfaceInputs, g: Ground, treeline: f32) -> f32 {
    let middle = treeline - 250.0 + g.sunny * 100.0 - g.hollow * 40.0 + (noise_r(inputs, 900.0) - 0.535) * 500.0;
    return ramp(middle - 100.0, middle + 100.0, inputs.height);
}

// How much of the boreal forest's spruce is native at (`lat`, `lon`): Norway spruce came into Scandinavia from the east
// and never reached Norway's coast north of Saltfjellet, and it thins out northward across Lapland. Its limit runs from
// 66.6 degrees north on the Norwegian coast (14 degrees east and west of it) to about 67.7 in the Torne valley (20
// east) and 69.2 in Inari, Pasvik and Kola (28 east and on), the share falling over half a degree of latitude across
// it. In the boreal woods outside Europe (the Urals' east, North America) spruce stands to the treeline.
fn spruce_native(lat: f32, lon: f32) -> f32 {
    if lon < -12.0 || lon > 60.0 {
        return 1.0;
    }
    let limit = 66.6 + 2.6 * clamp((lon - 14.0) / 14.0, 0.0, 1.0);
    return 1.0 - ramp(limit - 0.25, limit + 0.25, lat);
}

// A boreal wood's conifers under its birch belt, as shares of its trees: x spruce where it is native (more in the wet
// ground), y Scots pine (a quarter); past the spruce's limit the pine stands in stands on the dry ground (a noise at
// 650 m, as `choose`'s `stand`: some three in ten of the land), a few scattered elsewhere, and birch takes the rest
// and the wet ground (Ofoten's and Troms' woods: the mountain birch, with Scots pine on the dry ground low down).
fn boreal_conifers(inputs: SurfaceInputs, moist: f32, native: f32) -> vec2<f32> {
    let spruce = (0.45 + 0.25 * moist) * native;
    let stands = ramp(0.5, 0.7, noise_r(inputs, 650.0));
    let pine = mix((0.08 + 0.62 * stands) * (1.0 - 0.7 * moist), 0.25, native);
    return vec2<f32>(spruce, pine);
}

// The floor under the trees at a point, by the trees its biome stands there (`tree_place.wgsl`'s `choose`: the share of
// conifers, where they stand in stands), on a battle map by its style's (`tree_cover`): needles under the boreal forest's
// spruce and pine (seven to nine in ten of its trees, birch in its belt under the treeline; past the spruce's northern
// limit the pine's stands among the birch), in the temperate biomes' and
// Britain's and Europe's conifer stands (the stands' noise where the trees find theirs, the mountains, pines on the
// coast), in patches under the snow style's spruce and the warm biomes' pines; leaves under a few scattered conifers
// (the chalk style's, a subarctic birch wood's planted spruce). Dry by the climate's summer (Malta's in August, a dry
// country's chalk), the deserts always; wet in the tropics.
fn floor_at(inputs: SurfaceInputs, g: Ground) -> Floor {
    var f: Floor;
    f.dry = terrain.climate.w;
    f.wet = 0.0;
    var conifers = 0.0;
    let biome = biome_here(inputs.chart, g);
    if biome < 0.0 {
#ifdef STYLE_SNOW
        conifers = g.hollow * 0.3 + 0.15;
#endif
#ifdef STYLE_TROPICAL
        f.wet = 1.0;
#endif
#ifdef STYLE_VOLCANIC
        f.wet = 1.0;
#endif
    } else if biome == 1.0 || biome == 16.0 {
        // The wet tropics and Hawaii.
        f.wet = 1.0;
    } else if biome == 2.0 {
        f.dry = 1.0;
    } else if biome == 3.0 || biome == 12.0 || biome == 13.0 {
        // The Mediterranean's and North Africa's Aleppo and stone pines among the oaks, olives and palms.
        conifers = 0.35;
    } else if biome == 4.0 {
        conifers = 0.4;
    } else if biome == 5.0 || biome == 15.0 {
        // The humid subtropics' coastal pines; the south-eastern United States' pine woods.
        conifers = select(select(ramp(0.55, 0.65, noise_r(inputs, 650.0)), 0.6, g.coast < 600.0), 0.6, biome == 15.0);
        f.wet = 0.3;
    } else if biome == 6.0 || biome == 10.0 || biome == 11.0 || biome == 14.0 {
        // Temperate: spruce and pine above 800 m, pine by the sea and in a share of the stands, as `choose` puts its
        // trees (the same noise). Britain's, Europe's and the Northeast's trees find their conifers in the photograph's
        // dark woods, which the floor does not read: there it takes the temperate share.
        let stand = noise_r(inputs, 650.0);
        conifers = max(max(ramp(780.0, 820.0, inputs.height), ramp(0.7, 0.74, stand)),
            ramp(0.48, 0.52, stand) * one(ramp(450.0, 550.0, g.coast)));
    } else if biome == 7.0 {
        // Boreal: spruce (more where it is moist, where it is native) and pine under the birch belt; past the spruce's
        // limit pine in its stands on the dry ground (`boreal_conifers`, as `choose` stands them).
        let moist = max(max(g.gully, g.hollow), ramp(6.5, 9.0, g.wetness));
        let place = place_at(inputs.chart);
        let shares = boreal_conifers(inputs, moist, spruce_native(place.x, place.y));
        conifers = (shares.x + shares.y) * one(birch_belt(inputs, g, boreal_treeline(place.x)));
    } else if biome == 9.0 {
        // Japan's sugi and hinoki plantations, its mountains' fir.
        conifers = 0.4;
        f.wet = 0.3;
    } else if biome == 17.0 {
        // The interior West's pinyon, juniper and ponderosa on dry ground, its firs and spruces higher.
        conifers = 0.6;
        f.dry = max(f.dry, 0.5);
    }
    f.needles = 0.0;
    if conifers > 0.0 {
        f.needles = floor_patches(conifers, noise_r(inputs, 40.0));
    }
    return f;
}

// Litter on a forest's floor (native/docs/terrain/ground.md, "The ground close up"): a broadleaf wood's leaves dark
// brown (0.093
// of albedo, C*ab 15, where autumn beech leaves stand at 25 or more), the rainforest's wetter and darker (0.077), a
// conifer wood's needles darker and greyer (0.078, C*ab 9), a dry summer's woods pale with dust (0.19), as such floors
// measure (0.05-0.15 in the visible, dusty ground more); lighter where it is fresh, darker in the damp hollows. The
// forest material's ratio lays its leaves or needles over it, and it keeps its colour through the land's tone
// (`tone_down`), which would take 30% of its chroma.
fn litter_color(g: Ground, f: Floor) -> vec3<f32> {
    let leaves = mix(srgb(100.0, 83.0, 63.0), srgb(92.0, 76.0, 58.0), f.wet);
    let floor = mix(leaves, srgb(88.0, 77.0, 65.0), f.needles);
    return mix(floor, srgb(132.0, 118.0, 98.0), f.dry) * mix(0.88, 1.13, g.grain * 0.7 + g.speck * 0.3)
        * mix(1.0, 0.8, g.hollow);
}

// Detail photographs lend structure only where the pixel is small enough to hold it.
fn textured(value: f32, footprint: f32, size: f32, strength: f32) -> f32 {
    return mix(1.0, value, resolved(footprint, size) * strength);
}

// The detail photographs at the scale of `metres` (`detail`) for a `textured` of `size`: their mean (0.5), with nothing
// read, where the pixel is too large for them to show (`textured` gives 1 there whatever they hold).
fn detail_shown(inputs: SurfaceInputs, metres: f32, size: f32) -> vec4<f32> {
    if resolved(inputs.footprint, size) <= 0.0 {
        return vec4<f32>(0.5);
    }
    return detail(inputs, metres);
}

// ---- The land's tone, and the real-world surface ----

const LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// Every procedural land colour (a battle map's, the styles' beyond a theatre window's photograph, the trees' cards) is
// drawn this much less saturated and this much darker than authored (the land read too green and too bright against
// the sea and sky); the photograph keeps its own colour (`calibrated`). Snow and chalk keep their brightness
// (above a luminance of 0.35, toward 0.6). Not the seabed, the wet band or foam.
const LAND_SATURATION: f32 = 0.7;
const LAND_BRIGHTNESS: f32 = 0.86;

fn tone_down(color: vec3<f32>) -> vec3<f32> {
    let l = dot(color, LUMA);
    let brightness = mix(LAND_BRIGHTNESS, 1.0, ramp(0.35, 0.6, l));
    return max(mix(vec3<f32>(l), color, LAND_SATURATION), vec3<f32>(0.0)) * brightness;
}

// Satellite colour to albedo. A Sentinel-2 cloudless mosaic (EOX s2cloudless) is close to the ground's reflectance but
// darker than the procedural styles were authored (Kent: woods 0.029 in linear luminance, grass 0.052, fields 0.069,
// towns 0.095, bare chalk and sand 0.27, the land 0.060 on average, where the chalk style's downs stand at 0.16 and its
// fields at 0.10), contrasty and strongly saturated (chroma 0.75-0.9 of the luminance against the styles' 0.6-0.7). The
// luminance is compressed a little (`PHOTO_GAIN * l^PHOTO_GAMMA`: woods 0.033, grass 0.052, fields 0.065, towns
// 0.084 as before, bare ground 0.19, the land 0.058; the mosaic's range, seven to one between the Los Angeles basin
// and the rainforest, is wider than the ground's) and the colour keeps `PHOTO_SATURATION` of its chroma, and no more
// than `PHOTO_CHROMA` of its luminance (the rainforest of Guadalcanal photographs at 1.1). The photograph then keeps
// its colour: the land's tone (`tone_down`) is the procedural styles' (native/docs/terrain/surface.md, "The photograph
// as albedo").
const PHOTO_GAIN: f32 = 0.552;
const PHOTO_GAMMA: f32 = 0.8;
const PHOTO_SATURATION: f32 = 0.9;
const PHOTO_CHROMA: f32 = 0.9;
// Of the luminance's exponent, what a detail gets back against its surroundings (`real_surface`): 0.8 + 0.12.
const PHOTO_DETAIL: f32 = 0.12;
// The ground's mottling under the open classes, metres (and a finer octave at 0.37 of it).
const MOTTLE_M: f32 = 5.3;

fn calibrated(photo: vec3<f32>) -> vec3<f32> {
    let l = max(dot(photo, LUMA), 1e-4);
    var hue = mix(vec3<f32>(1.0), photo / l, PHOTO_SATURATION);
    let spread = max(max(hue.r, hue.g), hue.b) - min(min(hue.r, hue.g), hue.b);
    hue = max(mix(vec3<f32>(1.0), hue, min(1.0, PHOTO_CHROMA / max(spread, 1e-4))), vec3<f32>(0.0));
    return hue * (PHOTO_GAIN * pow(l, PHOTO_GAMMA));
}

// The share of a point that lies on the window maps' square, feathered over two texels at its edge (0 with none).
fn surface_inside(uv: vec2<f32>) -> f32 {
    let edge = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y)) * terrain.surface.w;
    return ramp(0.0, 2.0, edge);
}

// Whether real buildings stand on the land (`TerrainLayer::set_buildings_stand`): the built-up class then paints no
// lots, roofs or streets of its own.
fn buildings_stand() -> bool {
    return terrain.surface_haze.w > 0.5;
}

// ---- Near tiles: finer squares of the same sources about the camera (`terrain.near`, layer 1) ----
//
// Tile (i, j) of the grid lies in slot (i mod slots, j mod slots) of the near layer, so the layer's texture coordinates
// are the chart's over the slots' whole span, read repeating: continuous across slots (no seam in the filtering's
// derivatives). A slot draws only when its entry names the tile at the point; a tile fades out over `NEAR_FEATHER` of
// its side toward a neighbour not drawn.

const NEAR_TILE_TEXELS: f32 = 512.0;
const NEAR_FEATHER: f32 = 0.2;

fn near_bound() -> bool {
    return terrain.near.w > 0.0;
}

// Metres a near texel.
fn near_texel_m() -> f32 {
    return terrain.near.z / NEAR_TILE_TEXELS;
}

// The near layer's index: 1, or 0 with no near tiles (the textures have no second layer then; nothing near is drawn).
fn near_layer() -> i32 {
    return select(0, 1, terrain.near.w > 0.0);
}

fn near_uv(chart: vec2<f32>) -> vec2<f32> {
    return (chart - terrain.near.xy) / max(terrain.near.z * terrain.near.w, 1.0);
}

// Whether near tile `tile` is drawn (its slot holds it).
fn near_drawn(tile: vec2<i32>) -> bool {
    let slots = max(i32(terrain.near.w), 1);
    let slot = ((tile % slots) + slots) % slots;
    let k = slot.y * slots + slot.x;
    let entry = terrain.near_slots[k / 2];
    let held = select(entry.zw, entry.xy, k % 2 == 0);
    return all(held == tile);
}

// How much of a point the near tiles paint: 1 inside a tile drawn, fading toward an edge whose neighbour is not; 0
// with none.
fn near_share(chart: vec2<f32>) -> f32 {
    if terrain.near.w <= 0.0 {
        return 0.0;
    }
    let t = (chart - terrain.near.xy) / terrain.near.z;
    let base = floor(t);
    let tile = vec2<i32>(base);
    if !near_drawn(tile) {
        return 0.0;
    }
    let f = t - base;
    var share = 1.0;
    if !near_drawn(tile - vec2<i32>(1, 0)) {
        share = share * ramp(0.0, NEAR_FEATHER, f.x);
    }
    if !near_drawn(tile + vec2<i32>(1, 0)) {
        share = share * ramp(0.0, NEAR_FEATHER, 1.0 - f.x);
    }
    if !near_drawn(tile - vec2<i32>(0, 1)) {
        share = share * ramp(0.0, NEAR_FEATHER, f.y);
    }
    if !near_drawn(tile + vec2<i32>(0, 1)) {
        share = share * ramp(0.0, NEAR_FEATHER, 1.0 - f.y);
    }
    return share;
}

// ---- Magnified: the photograph where a texel spans several pixels ----
//
// The imagery's finest pixel is 8 to 19 m: within a few kilometres of the camera a texel spans tens of pixels, and the
// bilinear filter draws it as soft blobs and diamonds. Where the layer is magnified (`magnified_share`), its colour is
// rebuilt from level 0 (`magnified`): a cubic spline, the photograph's finest detail sharpened, and texel meeting texel
// at an edge, the lookup warped by noise so the edges are the land's irregular ones, not the texel grid's.

// How much of the photograph's own finest detail (its level 0 against level 1) is added back.
const MAGNIFIED_SHARPEN: f32 = 0.35;
// How much of the colour is the steepened filter's, and its edge's half width (texels).
const MAGNIFIED_CRISP: f32 = 0.5;
const MAGNIFIED_EDGE: f32 = 0.18;
// How far the lookup is warped, texels at most.
const MAGNIFIED_WARP: f32 = 0.8;

// How magnified a layer of `texel` metres is where a pixel spans `footprint` metres: 1 from a texel over 2.5 pixels, 0
// at a texel a pixel.
fn magnified_share(footprint: f32, texel: f32) -> f32 {
    return 1.0 - ramp(0.4, 1.0, footprint / max(texel, 0.01));
}

// Chart point `chart` moved by up to `MAGNIFIED_WARP` texels of `texel` metres, by `amount`: noise at two scales a few
// texels across (sampled with derivatives: call in uniform control flow).
fn magnified_warp(inputs: SurfaceInputs, texel: f32, amount: f32) -> vec2<f32> {
    let warp = vec2<f32>(noise_r(inputs, texel * 2.3), noise_r(inputs, texel * 3.7)) - 0.5;
    return inputs.chart + warp * (MAGNIFIED_WARP * texel * amount);
}

// Layer `layer`'s colour at level 0 (the layer `size` texels a side, read through `s`), magnified: a Catmull-Rom
// spline through five bilinear taps (Jimenez's approximation, its four corner taps left out) rather than the bilinear
// filter's facets, the photograph's finest detail added back by `MAGNIFIED_SHARPEN`; and `MAGNIFIED_CRISP` of it the
// bilinear filter with its weights steepened about each texel's middle, so one texel meets the next at an edge
// `MAGNIFIED_EDGE` of a texel wide instead of a ramp across both: a roof, a road, a field's edge stay edges. Explicit
// levels only, so it may run per pixel in a branch. Premultiplied, clamped to 0-1.
fn magnified(layer: i32, s: sampler, uv: vec2<f32>, size: f32) -> vec4<f32> {
    let p = uv * size;
    let c = floor(p - 0.5) + 0.5;
    let f = p - c;
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let t0 = (c - 1.0) / size;
    let t3 = (c + 2.0) / size;
    let t12 = (c + w2 / w12) / size;
    let sum = textureSampleLevel(surface_albedo, s, vec2<f32>(t12.x, t0.y), layer, 0.0) * (w12.x * w0.y)
        + textureSampleLevel(surface_albedo, s, vec2<f32>(t0.x, t12.y), layer, 0.0) * (w0.x * w12.y)
        + textureSampleLevel(surface_albedo, s, t12, layer, 0.0) * (w12.x * w12.y)
        + textureSampleLevel(surface_albedo, s, vec2<f32>(t3.x, t12.y), layer, 0.0) * (w3.x * w12.y)
        + textureSampleLevel(surface_albedo, s, vec2<f32>(t12.x, t3.y), layer, 0.0) * (w12.x * w3.y);
    let total = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    let cubic = sum / total;
    let soft = textureSampleLevel(surface_albedo, s, uv, layer, 1.0);
    let q = uv * size - 0.5;
    let cell = floor(q);
    let steep = smoothstep(vec2<f32>(0.5 - MAGNIFIED_EDGE), vec2<f32>(0.5 + MAGNIFIED_EDGE), q - cell);
    let crisp = textureSampleLevel(surface_albedo, s, (cell + steep + 0.5) / size, layer, 0.0);
    let smooth_part = cubic + (cubic - soft) * MAGNIFIED_SHARPEN;
    return clamp(mix(smooth_part, crisp, MAGNIFIED_CRISP), vec4<f32>(0.0), vec4<f32>(1.0));
}

// ---- The period's country: the marks on the cover (`country_shares`) ----
//
// In a past period (`terrain.country.w`, `TerrainLayer::set_country`) a cover texel may carry the mark
// `cover::PAST_COUNTRY` (0x80) over its class where today's built-up land stood empty then. Such land is drawn as the
// region's farmland of the time, field by field (`country_field`, `country_ground` below): its class taken as cropland,
// its colour not the photograph's. Water, snow, wetland and mangroves keep their own class whatever the mark. With the
// country off (`Period::Today`, a battle map), or where no texel about a point is marked, the cover is read exactly as
// before.

const PAST_COUNTRY: u32 = 0x80u;
// How much an unmarked field, wood or grass weighs against the marked land about it (`country_past`): the gardens,
// lawns and street trees among the roofs go with the period's country about them; the kept town's built-up and bare
// land, water and the unknown stand against it whole.
const COUNTRY_GREEN: f32 = 0.15;
// The town's edge: a field is the country where the marked share about its middle passes a level of its own, 0.5 ±
// this; it is drawn whole to this share of the region's field size from a marked texel.
const COUNTRY_EDGE_WANDER: f32 = 0.3;
const COUNTRY_REACH: f32 = 1.5;

fn country_on() -> bool {
    return terrain.country.w > 0.5;
}

// 1 where a cover byte is the period's country (its mark over a land class; a test's mark over built-up land, the
// texel's middle at chart point `centre`), else 0.
fn country_marked(raw: u32, centre: vec2<f32>) -> f32 {
    let id = raw & 0x7fu;
    let keeps = id == 70u || id == 80u || id == 90u || id == 95u;
    var marked = (raw & PAST_COUNTRY) != 0u;
    if !marked && id == 50u {
        marked = country_test_mark(centre);
    }
    return select(0.0, 1.0, marked && !keeps);
}

// How much an unmarked class stands against the marked country about it (1), or goes with it (`COUNTRY_GREEN`): the
// green classes (trees, shrubs, grass, fields, moss) go with it.
fn country_against(raw: u32) -> f32 {
    let id = raw & 0x7fu;
    let green = id == 10u || id == 20u || id == 30u || id == 40u || id == 100u;
    return select(1.0, COUNTRY_GREEN, green);
}

// Shares of the land-cover classes (ESA WorldCover) about a point, summing to 1.
struct CoverShares {
    // Tree cover, mangroves, shrubland, grassland and moss.
    a: vec4<f32>,
    // Cropland, built-up, bare and sparse ground, snow and ice.
    b: vec4<f32>,
    // Open water, herbaceous wetland, unknown.
    c: vec4<f32>,
    // The share of the point that is the period's country (`country_shares`), counted in the cropland's (`b.x`): 0
    // with no texel about the point marked, or with the country off.
    past: f32,
}

fn cover_class(id: u32) -> CoverShares {
    var s: CoverShares;
    s.a = vec4<f32>(f32(id == 10u), f32(id == 95u), f32(id == 20u), f32(id == 30u || id == 100u));
    s.b = vec4<f32>(f32(id == 40u), f32(id == 50u), f32(id == 60u), f32(id == 70u));
    let known = dot(s.a, vec4<f32>(1.0)) + dot(s.b, vec4<f32>(1.0)) + f32(id == 80u) + f32(id == 90u);
    s.c = vec4<f32>(f32(id == 80u), f32(id == 90u), 1.0 - known, 0.0);
    return s;
}

// The cover byte at texel `texel` of a cover layer `size` texels a side: the window's (layer 0) clamped at its edge, the
// near tiles' (layer 1) repeating.
fn cover_raw(texel: vec2<i32>, size: i32, layer: i32) -> u32 {
    let n = max(size, 1);
    let at = select(clamp(texel, vec2<i32>(0), vec2<i32>(n - 1)), ((texel % n) + n) % n, layer == 1);
    return textureLoad(surface_cover, at, layer, 0).r;
}

// The class today of a cover byte (`cover::today`): a past period's mark (`cover::PAST_COUNTRY`, 0x80) taken off.
fn cover_today(raw: u32) -> CoverShares {
    return cover_class(raw & 0x7fu);
}

// The window maps' cover about a point (`cover_shares_on`).
fn cover_shares(inputs: SurfaceInputs) -> CoverShares {
    let texel = max(terrain.surface.z / max(terrain.surface.w, 1.0), 1.0);
    return cover_shares_on(inputs, terrain.surface.xy, texel, i32(terrain.surface.w), 0);
}

// The near tiles' cover about a point.
fn near_cover_shares(inputs: SurfaceInputs) -> CoverShares {
    return cover_shares_on(inputs, terrain.near.xy, near_texel_m(), i32(terrain.near.w * NEAR_TILE_TEXELS), 1);
}

fn mixed_shares(a: CoverShares, b: CoverShares, t: f32) -> CoverShares {
    var s: CoverShares;
    s.a = mix(a.a, b.a, t);
    s.b = mix(a.b, b.b, t);
    s.c = mix(a.c, b.c, t);
    s.past = mix(a.past, b.past, t);
    return s;
}

// The cover about a point on a layer of `texel` metres whose texel (0, 0) has its corner at chart point `origin`: the
// four texels around it blended bilinearly, then sharpened toward the nearest class (cubed and normalised, so a
// boundary stays crisp and a field ends where it ends), the lookup warped by noise about a texel across so the texel
// grid never shows as stairs.
fn cover_shares_on(inputs: SurfaceInputs, origin: vec2<f32>, texel: f32, size: i32, layer: i32) -> CoverShares {
    let warp = vec2<f32>(noise_r(inputs, texel * 1.3), noise_r(inputs, texel * 1.9)) - 0.5;
    let g = (inputs.chart - origin) / texel - 0.5 + warp * 1.2;
    let base = floor(g);
    let f = g - base;
    let i = vec2<i32>(base);
    let r00 = cover_raw(i, size, layer);
    let r10 = cover_raw(i + vec2<i32>(1, 0), size, layer);
    let r01 = cover_raw(i + vec2<i32>(0, 1), size, layer);
    let r11 = cover_raw(i + vec2<i32>(1, 1), size, layer);
    let s00 = cover_today(r00);
    let s10 = cover_today(r10);
    let s01 = cover_today(r01);
    let s11 = cover_today(r11);
    let w = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    let a = s00.a * w.x + s10.a * w.y + s01.a * w.z + s11.a * w.w;
    let b = s00.b * w.x + s10.b * w.y + s01.b * w.z + s11.b * w.w;
    let c = s00.c * w.x + s10.c * w.y + s01.c * w.z + s11.c * w.w;
    let a3 = a * a * a;
    let b3 = b * b * b;
    let c3 = c * c * c;
    let total = max(dot(a3, vec4<f32>(1.0)) + dot(b3, vec4<f32>(1.0)) + dot(c3, vec4<f32>(1.0)), 1e-6);
    var s: CoverShares;
    s.a = a3 / total;
    s.b = b3 / total;
    s.c = c3 / total;
    if country_on() {
        // A texel about the point marked, or one within reach (`country_near`): the period's country there, field by
        // field (`country_shares`).
        let corner = origin + (base + 0.5) * texel;
        let marks = vec4<f32>(country_marked(r00, corner), country_marked(r10, corner + vec2<f32>(texel, 0.0)),
            country_marked(r01, corner + vec2<f32>(0.0, texel)), country_marked(r11, corner + vec2<f32>(texel)));
        if any(marks > vec4<f32>(0.0)) || country_near(inputs) {
            return country_shares(inputs, s);
        }
    }
    return s;
}

// The layer the country is decided on (`country_near`, `country_past`): the window's maps, whole about any point they
// cover; the near tiles' only without them (a near tile's neighbours may not be resident, nor the tile a slot holds be
// the one about a point).
struct CountryLayer {
    origin: vec2<f32>,
    texel: f32,
    size: i32,
    layer: i32,
}

fn country_layer() -> CountryLayer {
    var l: CountryLayer;
    if terrain.surface.w > 0.0 {
        l.origin = terrain.surface.xy;
        l.texel = max(terrain.surface.z / terrain.surface.w, 1.0);
        l.size = i32(terrain.surface.w);
        l.layer = 0;
    } else {
        l.origin = terrain.near.xy;
        l.texel = near_texel_m();
        l.size = i32(terrain.near.w * NEAR_TILE_TEXELS);
        l.layer = 1;
    }
    return l;
}

// Whether a mark lies within the country's reach of a point (1.5 of the region's field length, `COUNTRY_REACH`):
// eight texels about the point's own on the decision's layer (`country_layer`), four on the diagonals at the reach, four
// square to it at half of it; the answer kept for the point (its window and near tiles' lookups ask in turn).
fn country_near(inputs: SurfaceInputs) -> bool {
    if country_ring_set && all(country_ring_at == inputs.chart) {
        return country_ring_found;
    }
    let l = country_layer();
    let reach = country_layout(country_region()).size.x * COUNTRY_REACH;
    let k = max(round(reach / l.texel), 1.0);
    let d = round(k * 0.7);
    let h = ceil(k * 0.5);
    let own = floor((inputs.chart - l.origin) / l.texel);
    var found = false;
    for (var n = 0; n < 8; n += 1) {
        let x = f32((n & 1) * 2 - 1);
        let y = f32((n >> 1u) % 2 * 2 - 1);
        var o = vec2<f32>(x, y) * d;
        if n >= 4 {
            o = select(vec2<f32>(0.0, x * h), vec2<f32>(x * h, 0.0), n < 6);
        }
        let t = own + o;
        if country_marked(cover_raw(vec2<i32>(t), l.size, l.layer), l.origin + (t + 0.5) * l.texel) > 0.0 {
            found = true;
            break;
        }
    }
    country_ring_set = true;
    country_ring_at = inputs.chart;
    country_ring_found = found;
    return found;
}

// Whether a mark lies within reach of `country_ring_at` (`country_near`): the window's and the near tiles' lookups of
// one pixel look about it once between them.
var<private> country_ring_found: bool;
var<private> country_ring_at: vec2<f32>;
var<private> country_ring_set: bool;

// The share of the land about chart point `at` that is the period's country (on the decision's layer, `country_layer`,
// unwarped): the marked texels' bilinear weight against the unmarked ones' (the green classes weighing `COUNTRY_GREEN`,
// so the gardens and street trees among marked roofs go with the country, a kept town's built-up land, water and the
// unknown whole).
fn country_past(at: vec2<f32>) -> f32 {
    let l = country_layer();
    let origin = l.origin;
    let texel = l.texel;
    let size = l.size;
    let layer = l.layer;
    let g = (at - origin) / texel - 0.5;
    let base = floor(g);
    let f = g - base;
    let i = vec2<i32>(base);
    let r00 = cover_raw(i, size, layer);
    let r10 = cover_raw(i + vec2<i32>(1, 0), size, layer);
    let r01 = cover_raw(i + vec2<i32>(0, 1), size, layer);
    let r11 = cover_raw(i + vec2<i32>(1, 1), size, layer);
    let corner = origin + (base + 0.5) * texel;
    let marks = vec4<f32>(country_marked(r00, corner), country_marked(r10, corner + vec2<f32>(texel, 0.0)),
        country_marked(r01, corner + vec2<f32>(0.0, texel)), country_marked(r11, corner + vec2<f32>(texel)));
    let against = vec4<f32>(country_against(r00), country_against(r10), country_against(r01), country_against(r11));
    let w = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    let marked = dot(w, marks);
    return marked / max(marked + dot(w * (vec4<f32>(1.0) - marks), against), 1e-6);
}

// The cover `s` about a point near the period's country (a texel about it marked, or a green patch among marked
// texels): whole fields are the country (`country_field`), where the marked share about the field's middle
// (`country_past`) passes a level of the field's own, so the town's edge runs along the fields' edges; there the cover
// is cropland (`past` its share), but for the water, snow, wetland and mangroves the point holds today. Elsewhere `s`.
fn country_shares(inputs: SurfaceInputs, s: CoverShares) -> CoverShares {
    let field = country_field(inputs);
    let level = 0.5 + (fract(field.id * 5.77) - 0.5) * (2.0 * COUNTRY_EDGE_WANDER);
    if country_past(field.middle) <= level {
        return s;
    }
    let keep = s.a.y + s.b.w + s.c.x + s.c.y;
    var out: CoverShares;
    out.a = vec4<f32>(0.0, s.a.y, 0.0, 0.0);
    out.b = vec4<f32>(1.0 - keep, 0.0, 0.0, s.b.w);
    out.c = vec4<f32>(s.c.x, s.c.y, 0.0, 0.0);
    out.past = 1.0 - keep;
    return out;
}

// The window's mean colour for the cover about a point (`terrain.cover_means`), w the share of it the window knows.
fn typical_color(s: CoverShares) -> vec4<f32> {
    let m = terrain.cover_means;
    return s.a.x * m[1] + s.a.y * m[11] + s.a.z * m[2] + s.a.w * m[3] + s.b.x * m[4] + s.b.y * m[5] + s.b.z * m[6]
        + s.b.w * m[7] + s.c.x * m[8] + s.c.y * m[9] + s.c.z * m[0];
}

// How far the season has stripped the leaves (the seasonal styles only: the tropics stay in leaf whatever the month).
fn leafless() -> f32 {
#ifdef STYLE_CHALK
    return 1.0 - max(terrain.climate.z, terrain.climate.w);
#else
#ifdef STYLE_SNOW
    return 1.0 - terrain.climate.z;
#else
    return 0.0;
#endif
#endif
}

// A summer photograph's green turned toward winter's browns and greys by `amount`.
fn wintered(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let l = dot(color, LUMA);
    return mix(color, vec3<f32>(l * 1.12, l * 1.0, l * 0.8), amount);
}

// Snow from the climate's snowline (as the snow style lays it: ragged with the aspect and the relief, lying in patches
// below the line until the thaw), for a land painted from photographs taken in summer. The seasonal styles only: the
// tropical and volcanic coasts have none to fear at sea level (a high peak's snow is in its photograph and its cover).
fn climate_snow(inputs: SurfaceInputs, g: Ground) -> f32 {
#ifdef STYLE_TROPICAL
    return 0.0;
#else
#ifdef STYLE_VOLCANIC
    return 0.0;
#else
    let height = inputs.height;
    // None lies more than 700 m under the lowest the snowline reaches (a sunless hollow).
    if height < terrain.climate.x - 1100.0 {
        return 0.0;
    }
    let snowline = snowline_at(g);
    let above = ramp(snowline - 20.0, snowline + 20.0, height);
    let shore = one(ramp(3.0, 22.0, height)) * one(ramp(40.0, 350.0, g.coast));
    let lying = ramp(snowline - 260.0, snowline, height) * 0.35 + 0.5 + g.hollow * 0.25 - g.knoll * 0.2 - g.sunny * 0.9
        - shore * 0.45 + g.jag * 0.6 - terrain.climate.y * 0.65;
    let near_line = one(ramp(300.0, 700.0, snowline - height));
    var patches = 0.0;
    if near_line > 0.0 && above < 1.0 {
        patches = lying_snow(inputs, lying) * near_line;
    }
    // Steep faces shed it.
    return max(above, patches) * one(snow_shed(g.slope + g.jag * 0.15, snow_melt(height, snowline)));
#endif
#endif
}

// The climate's snowline (m) where the ground stands: higher on slopes facing the sun and on knolls, lower in hollows,
// ragged with the relief.
fn snowline_at(g: Ground) -> f32 {
    return terrain.climate.x + g.sunny * 220.0 - g.hollow * 60.0 + g.knoll * 25.0 + g.jag * 110.0;
}

// How far the melt has gone at `height` (m) where the snowline stands at `snowline`, 0-1: the season's (the leaves',
// `terrain.climate.z`, which come out as the melt starts in late April and fall as it ends in October; none for
// `Climate::AUTHORED`), up to 300 m over the snowline and gone 900 m over it, where the cold keeps the faces' snow and
// ice as in winter (the north faces' ice fields under the Alps' summits).
fn snow_melt(height: f32, snowline: f32) -> f32 {
    return terrain.climate.z * one(ramp(snowline + 300.0, snowline + 900.0, height));
}

// How much of the season's snow a face sheds by its `slope` (the gradient's length), 0-1. In winter from about 35
// degrees (0.7) to 45 (1.0), as Vestfjord's April has it; in the melt (`snow_melt`) from about 30 (0.58) to 40 (0.84),
// the wet snow sliding off and the dark rock warming through it, so by July the Alps' rock faces stand bare over their
// glaciers (the window's 40 m cells draw a 50-60 degree face at some 40-45). `horizon.wgsl`'s ring sheds it alike.
fn snow_shed(slope: f32, melt: f32) -> f32 {
    return ramp(mix(0.7, 0.58, melt), mix(1.0, 0.84, melt), slope);
}

// The share of the noise tile's values over `cut`: a logistic about the tile's mean 0.535, its scale 0.07 (as
// `lying_snow`'s quantile). What a pattern cut there covers on average, once a pixel outgrows it.
fn share_over(cut: f32) -> f32 {
    return 1.0 / (1.0 + exp((cut - 0.535) / 0.07));
}

// Glacier ice: WorldCover's snow and ice where the season's snow has gone from it (`climate_snow` lays the firn and the
// snow over it above the snowline and in patches below), as the Aletsch's tongue lies in late summer. Blue-grey,
// streaked along its flow by bands, greyer with the dirt the melt leaves on it farther below the snowline; striped by
// medial moraines (the rock a tributary's edges carry down the trunk glacier from where two meet: dark bands some 40-80 m
// wide and a kilometre or more long, about one part in twelve of the ice); crossed by crevasses where the ice steepens
// and stretches (a few on the flat, many past 17 degrees: dark blue slots some 10 m wide and 200 m long across the
// flow). Each fades to its mean as a pixel outgrows it. The tropical and volcanic styles keep `snow`: they lay no
// season's snow over it (a high peak's snow is in its cover).
fn glacier_ice(inputs: SurfaceInputs, g: Ground, snow: Layered) -> Layered {
#ifdef STYLE_TROPICAL
    return snow;
#else
#ifdef STYLE_VOLCANIC
    return snow;
#else
    let footprint = inputs.footprint;
    // The ice flows down the glacier's broad fall line: the gradients' second mip (160 m). The window's own (40 m) turns
    // with every hummock on ice so nearly flat, and its streaks lay at every heading; the fourth (640 m) takes in the
    // valley's walls and turns them across the glacier.
    let flow = textureSampleLevel(gradients, clamp_sampler, field_uv(inputs.chart), 3.0).xy;
    let dirt = ramp(0.0, 900.0, snowline_at(g) - inputs.height);
    var color = srgb(150.0, 168.0, 180.0) * mix(1.0, 0.65, dirt)
        * ((g.grain - 0.5) * 0.16 + (g.speck - 0.5) * 0.1 + 1.0);
    let bands_near = resolved(footprint, 12.0);
    if bands_near > 0.0 {
        color *= (streaks_read(inputs, flow, 12.0, 600.0, 0.0) - 0.5) * 0.35 * bands_near + 1.0;
    }
    // The moraines: a noise in cells 150 m across and 12 km long down the flow, over 0.64-0.70 (bands some 40-80 m
    // wide), drawn while a pixel is under some 30 m (a long band shows when it is no wider than a pixel) and their share
    // from 120 m.
    var moraine = share_over(0.705);
    let moraine_near = resolved(footprint, 240.0);
    if moraine_near > 0.0 {
        moraine = mix(moraine, ramp(0.68, 0.73, streaks_read(inputs, flow, 150.0, 12000.0, 0.0)), moraine_near);
    }
    // The crevasses: a noise in cells 10 m across and 200 m long across the flow (the headings turned a quarter), over a
    // cut falling from 0.86 on the flat to 0.66 past 17 degrees (the ice's own slope); their share from 15 m a pixel.
    let cut = mix(0.86, 0.66, ramp(0.08, 0.3, g.slope));
    var crevasse = share_over(cut + 0.02);
    let crevasse_near = resolved(footprint, 30.0);
    if crevasse_near > 0.0 {
        crevasse = mix(crevasse, ramp(cut, cut + 0.04, streaks_read(inputs, flow, 10.0, 200.0, 4.0)), crevasse_near);
    }
    let debris = srgb(98.0, 93.0, 88.0) * ((g.speck - 0.5) * 0.3 + 1.0);
    var ice = base_cover(color, 0.5, 0.5, 0.0);
    ice = cover(ice, debris, 0.9, 0.35, 2.0, moraine);
    ice = cover(ice, srgb(54.0, 84.0, 110.0), 0.6, 0.4, -1.5, crevasse * one(moraine));
    return ice;
#endif
#endif
}

// ---- The period's country: its fields (native/docs/terrain/period.md, "The period's ground", "The country") ----
//
// The land a past period leaves as country (`country_shares`) is drawn as the region's farmland of the 1940s
// (`terrain.country.x`, `country::Country`): a patchwork of fields (`country_field`), each of a kind the region grew
// (`FIELD_*`, in the region's shares, `country_kind`), its colour made from the window's own rural colours (the cover's
// means: grass, fields, trees, bare ground and shrubs, the haze out) so it sits in the photograph about it, through the
// season (`terrain.country.y`, the months), each field with its own tone, stage and heading; near the camera each
// kind's structure (furrows, strips, rows of trees, paddies' bunds) and the fields' edges (hedgerows, tracks, bunds,
// walls, ditches), each fading to its mean as a pixel outgrows it (`country_ground`). The trees' placement reads the
// same fields (`tree_place.wgsl`'s `country_slot`): orchards' rows, woodlots, hedgerow and windbreak trees.

const COUNTRY_WEST: u32 = 1u;
const COUNTRY_EAST: u32 = 2u;
const COUNTRY_BRITAIN: u32 = 3u;
const COUNTRY_EUROPE: u32 = 4u;
const COUNTRY_JAPAN: u32 = 5u;
const COUNTRY_TROPICS: u32 = 6u;
const COUNTRY_NORDIC: u32 = 7u;
const COUNTRY_MEDITERRANEAN: u32 = 8u;

// The kinds of field.
const FIELD_PASTURE: u32 = 0u;
const FIELD_GRAIN: u32 = 1u;
const FIELD_PLOUGHED: u32 = 2u;
// Citrus, walnuts or apples in rows (`country_orchard`).
const FIELD_ORCHARD: u32 = 3u;
// Vegetables in strips: the West's truck farms, Japan's dry fields, the Mediterranean's vines.
const FIELD_TRUCK: u32 = 4u;
const FIELD_PADDY: u32 = 5u;
// Hay and alfalfa: the Nordic tún, the West's irrigated green.
const FIELD_HAY: u32 = 6u;
// Small mixed plots: the tropics' gardens.
const FIELD_GARDEN: u32 = 7u;
// Coconut palms in rows.
const FIELD_PALMS: u32 = 8u;
// A woodlot, a village's grove.
const FIELD_WOOD: u32 = 9u;
const FIELD_OLIVES: u32 = 10u;
// The unimproved land: kunai, heath and moss, garrigue.
const FIELD_ROUGH: u32 = 11u;
// A yard along the shore: a quay's, a boatyard's or a fishing village's packed earth, cinders and gravel (the docks
// and sheds a past period kept stand on it, `country_field`).
const FIELD_YARD: u32 = 12u;

const TAU: f32 = 6.283185307179586;

fn country_region() -> u32 {
    return u32(terrain.country.x + 0.5);
}

// How a region lays its fields out: their sides before they are split (m), the share of districts whose fields run on
// the chart's own axes (a survey's grid: the West's section lines), how far their lines bend (m), and whether they are
// the irregular fields of old enclosure instead (`parcels`' cells, `size.x` across).
struct CountryLayout {
    size: vec2<f32>,
    aligned: f32,
    bend: f32,
    irregular: bool,
}

fn country_layout(region: u32) -> CountryLayout {
    var l = CountryLayout(vec2<f32>(300.0, 180.0), 0.3, 10.0, false);
    switch region {
        case COUNTRY_WEST: {
            // Ten- to forty-acre ranches and groves on the section lines.
            l = CountryLayout(vec2<f32>(400.0, 200.0), 0.8, 3.0, false);
        }
        case COUNTRY_EAST: {
            l = CountryLayout(vec2<f32>(320.0, 180.0), 0.5, 12.0, false);
        }
        case COUNTRY_BRITAIN: {
            l = CountryLayout(vec2<f32>(150.0), 0.0, 0.0, true);
        }
        case COUNTRY_EUROPE: {
            // Long strips of the open fields.
            l = CountryLayout(vec2<f32>(320.0, 70.0), 0.2, 18.0, false);
        }
        case COUNTRY_JAPAN: {
            // Blocks of paddies a hundred metres long (the old jōri grid's tan), split into plots.
            l = CountryLayout(vec2<f32>(110.0, 60.0), 0.3, 5.0, false);
        }
        case COUNTRY_TROPICS: {
            l = CountryLayout(vec2<f32>(380.0, 260.0), 0.2, 25.0, false);
        }
        case COUNTRY_NORDIC: {
            l = CountryLayout(vec2<f32>(120.0), 0.0, 0.0, true);
        }
        case COUNTRY_MEDITERRANEAN: {
            l = CountryLayout(vec2<f32>(190.0, 110.0), 0.2, 12.0, false);
        }
        default: {}
    }
    return l;
}

// A field of the period's country about a point.
struct CountryField {
    // Its own number (0-1) and kind (`FIELD_*`).
    id: f32,
    kind: u32,
    // The way its rows run (a chart direction), the point in the field's own frame (metres along its rows and across
    // them, from its corner for the laid-out fields), its sides (m), and metres to its nearest side.
    along: vec2<f32>,
    local: vec2<f32>,
    size: vec2<f32>,
    edge: f32,
    // Metres from its farmstead's place: a laid-out field's corner, an old enclosure's middle.
    home: f32,
    // Its middle (a chart point).
    middle: vec2<f32>,
}

// Two smooth value noises (0-1 each) over a lattice of unit squares, `salt` choosing a pair of many.
fn smooth_values(p: vec2<f32>, salt: f32) -> vec2<f32> {
    let base = floor(p);
    let f = p - base;
    let s = f * f * (3.0 - 2.0 * f);
    let k = base + vec2<f32>(salt * 131.0, salt * 71.0);
    return mix(mix(corner_offset_fast(k), corner_offset_fast(k + vec2<f32>(1.0, 0.0)), s.x),
        mix(corner_offset_fast(k + vec2<f32>(0.0, 1.0)), corner_offset_fast(k + vec2<f32>(1.0, 1.0)), s.x), s.y);
}

// Cells about points scattered one to a square of a lattice (`cells_at`): at a point (in squares), its cell's square,
// the cell's point and the distance to the cell's nearest edge (in squares).
struct Cells {
    square: vec2<f32>,
    point: vec2<f32>,
    edge: f32,
}

fn cells_at(p: vec2<f32>, salt: f32) -> Cells {
    let n = floor(p);
    let f = p - n;
    // The points of the nine squares about the point's own (a point lies in the middle 0.7 of its square, so the
    // nearest is among them, and so are the neighbours whose bisectors bound its cell about the point). No array: a
    // second pass hashes them again (cheaper than an array indexed by the loop, which may go to memory).
    var near_g = vec2<f32>(0.0);
    var near_r = vec2<f32>(0.0);
    var best = 8.0;
    for (var k = 0; k < 9; k += 1) {
        let g = vec2<f32>(f32(k % 3 - 1), f32(k / 3 - 1));
        let r = g + 0.15 + 0.7 * corner_offset_fast(n + g + salt) - f;
        let d = dot(r, r);
        if d < best {
            best = d;
            near_g = g;
            near_r = r;
        }
    }
    // The edge: the nearest bisector between the cell's point and the others'.
    var edge = 8.0;
    for (var k = 0; k < 9; k += 1) {
        let g = vec2<f32>(f32(k % 3 - 1), f32(k / 3 - 1));
        let r = g + 0.15 + 0.7 * corner_offset_fast(n + g + salt) - f;
        let between = r - near_r;
        let length2 = dot(between, between);
        if length2 > 1e-6 {
            edge = min(edge, dot(0.5 * (near_r + r), between) * inverseSqrt(length2));
        }
    }
    var c: Cells;
    c.square = n + near_g;
    c.point = p + near_r;
    c.edge = edge;
    return c;
}

// As `cells_at` in one pass, its edge the bisector with the next nearest point's alone (a little long near a cell's
// corners): the districts' (`country_field`), whose edges are only a track's.
fn cells_rough(p: vec2<f32>, salt: f32) -> Cells {
    let n = floor(p);
    let f = p - n;
    var near_g = vec2<f32>(0.0);
    var near_r = vec2<f32>(0.0);
    var next_r = vec2<f32>(8.0);
    var best = 64.0;
    var second = 64.0;
    for (var k = 0; k < 9; k += 1) {
        let g = vec2<f32>(f32(k % 3 - 1), f32(k / 3 - 1));
        let r = g + 0.15 + 0.7 * corner_offset_fast(n + g + salt) - f;
        let d = dot(r, r);
        if d < best {
            second = best;
            next_r = near_r;
            best = d;
            near_g = g;
            near_r = r;
        } else if d < second {
            second = d;
            next_r = r;
        }
    }
    let between = next_r - near_r;
    var c: Cells;
    c.square = n + near_g;
    c.point = p + near_r;
    c.edge = dot(0.5 * (near_r + next_r), between) * inverseSqrt(max(dot(between, between), 1e-6));
    return c;
}

// The kind of field `id` (0-1) is in `region`, its district `district` (0-1): within a district the fields share a
// part of the region's mix (a citrus belt, a flat of truck farms, a valley of paddies), the roll spanning 0.65 of the
// shares from the district's own start.
fn country_kind(region: u32, id: f32, district: f32) -> u32 {
    let roll = fract(district * 7.31 + fract(id * 3.71) * 0.65);
    switch region {
        case COUNTRY_WEST: {
            // Los Angeles' basin and the San Fernando Valley in 1942: citrus and walnut groves, truck farms, dry
            // grain and grass, alfalfa.
            if roll < 0.36 { return FIELD_ORCHARD; }
            if roll < 0.5 { return FIELD_TRUCK; }
            if roll < 0.68 { return FIELD_GRAIN; }
            if roll < 0.84 { return FIELD_PASTURE; }
            if roll < 0.93 { return FIELD_PLOUGHED; }
            return FIELD_HAY;
        }
        case COUNTRY_EAST: {
            if roll < 0.28 { return FIELD_PASTURE; }
            if roll < 0.52 { return FIELD_GRAIN; }
            if roll < 0.66 { return FIELD_HAY; }
            if roll < 0.76 { return FIELD_PLOUGHED; }
            if roll < 0.9 { return FIELD_WOOD; }
            if roll < 0.95 { return FIELD_TRUCK; }
            return FIELD_ORCHARD;
        }
        case COUNTRY_BRITAIN: {
            if roll < 0.5 { return FIELD_PASTURE; }
            if roll < 0.66 { return FIELD_GRAIN; }
            if roll < 0.78 { return FIELD_PLOUGHED; }
            if roll < 0.9 { return FIELD_HAY; }
            if roll < 0.96 { return FIELD_WOOD; }
            return FIELD_ORCHARD;
        }
        case COUNTRY_EUROPE: {
            if roll < 0.3 { return FIELD_GRAIN; }
            if roll < 0.58 { return FIELD_PASTURE; }
            if roll < 0.74 { return FIELD_PLOUGHED; }
            if roll < 0.84 { return FIELD_HAY; }
            if roll < 0.92 { return FIELD_TRUCK; }
            if roll < 0.97 { return FIELD_WOOD; }
            return FIELD_ORCHARD;
        }
        case COUNTRY_JAPAN: {
            // Paddies, the dry fields' strips on the higher ground, a village's grove, mulberry.
            if roll < 0.62 { return FIELD_PADDY; }
            if roll < 0.8 { return FIELD_TRUCK; }
            if roll < 0.86 { return FIELD_GARDEN; }
            if roll < 0.95 { return FIELD_WOOD; }
            return FIELD_ORCHARD;
        }
        case COUNTRY_TROPICS: {
            // Kunai, coconut plantations, gardens, bush.
            if roll < 0.34 { return FIELD_ROUGH; }
            if roll < 0.64 { return FIELD_PALMS; }
            if roll < 0.8 { return FIELD_GARDEN; }
            if roll < 0.94 { return FIELD_WOOD; }
            return FIELD_PASTURE;
        }
        case COUNTRY_NORDIC: {
            // Hayfields, heath and moss, a little pasture and potatoes.
            if roll < 0.48 { return FIELD_HAY; }
            if roll < 0.82 { return FIELD_ROUGH; }
            if roll < 0.94 { return FIELD_PASTURE; }
            return FIELD_PLOUGHED;
        }
        case COUNTRY_MEDITERRANEAN: {
            if roll < 0.3 { return FIELD_OLIVES; }
            if roll < 0.58 { return FIELD_GRAIN; }
            if roll < 0.72 { return FIELD_PLOUGHED; }
            if roll < 0.82 { return FIELD_TRUCK; }
            if roll < 0.94 { return FIELD_ROUGH; }
            return FIELD_ORCHARD;
        }
        default: {}
    }
    return FIELD_PASTURE;
}

// The field of the period's country at a point. Laid out fields: districts of a few kilometres (cells about scattered
// points, `cells_at`), each its grid's heading (on the chart's axes in a share of them), a size of its own, its lines
// bent a little over hundreds of metres; in each, rows of fields of the region's size, each row shifted along, a field
// whole or split in two or three. The irregular ones are cells about scattered points, their edges wavering. Where the
// land is too steep to plough a share of the fields are the hills' rough grazing, or their woods.
fn country_field_worked(inputs: SurfaceInputs) -> CountryField {
    let region = country_region();
    let plan = country_layout(region);
    var field: CountryField;
    let districts = cells_rough(inputs.chart / 2600.0, 5003.0);
    let district = corner_offset(districts.square + 7001.0).x;
    if plan.irregular {
        // (A smooth noise: the hedges bend, they do not wiggle.)
        let q = inputs.chart / (plan.size.x * 2.7);
        let warp = (smooth_values(q, 11.0) - 0.5) * 0.5;
        let p = inputs.chart / plan.size.x + warp;
        let cells = cells_at(p, 9001.0);
        field.id = corner_offset(cells.square + 3001.0).y;
        let heading = field.id * 37.7;
        field.along = vec2<f32>(cos(heading), sin(heading));
        field.local = vec2<f32>(dot(inputs.chart, field.along),
            dot(inputs.chart, vec2<f32>(-field.along.y, field.along.x)));
        field.size = plan.size;
        field.edge = cells.edge * plan.size.x;
        field.home = distance(cells.point, p) * plan.size.x;
        field.middle = (cells.point - warp) * plan.size.x;
    } else {
        let d = corner_offset(districts.square + 4093.0);
        let heading = select(d.x * 3.14159265, (d.x - 0.5) * 0.05, d.y < plan.aligned);
        let axis = vec2<f32>(cos(heading), sin(heading));
        let side = vec2<f32>(-axis.y, axis.x);
        let bend = (smooth_values(inputs.chart / 620.0, 31.0) - 0.5)
            * (2.0 * plan.bend);
        let r = vec2<f32>(dot(inputs.chart, axis), dot(inputs.chart, side)) + bend;
        let cell = plan.size * (0.75 + 0.5 * fract(d.y * 7.3));
        let row = floor(r.y / cell.y);
        let shift = corner_offset(vec2<f32>(row, district * 977.0)).x * cell.x;
        let column = floor((r.x + shift) / cell.x);
        var lo = vec2<f32>(column * cell.x - shift, row * cell.y);
        var size = cell;
        let split = corner_offset(vec2<f32>(column, row) + district * 3001.0);
        var part = 0.0;
        if split.x < 0.3 {
            // Two side by side.
            let cut = (0.35 + 0.3 * split.y) * size.x;
            let second = r.x - lo.x >= cut;
            part = select(1.0, 2.0, second);
            lo.x += select(0.0, cut, second);
            size.x = select(cut, size.x - cut, second);
        } else if split.x < 0.5 {
            // Three strips.
            size.x /= 3.0;
            let k = clamp(floor((r.x - lo.x) / size.x), 0.0, 2.0);
            part = 3.0 + k;
            lo.x += k * size.x;
        } else if split.x < 0.72 {
            // Two one behind the other.
            let cut = (0.35 + 0.3 * split.y) * size.y;
            let second = r.y - lo.y >= cut;
            part = select(6.0, 7.0, second);
            lo.y += select(0.0, cut, second);
            size.y = select(cut, size.y - cut, second);
        }
        field.id = corner_offset(vec2<f32>(column * 8.0 + part, row) + district * 7919.0).y;
        let local = r - lo;
        field.edge = min(min(local.x, size.x - local.x), min(local.y, size.y - local.y));
        field.home = length(local);
        // (Its lines' bend here taken for the middle's.)
        let middle = lo + size * 0.5 - bend;
        field.middle = axis * middle.x + side * middle.y;
        // Rows along the field's length, or across it in a share of them.
        let across = fract(field.id * 5.3) < 0.3;
        let long_x = size.x >= size.y;
        let along_x = long_x != across;
        field.along = select(side, axis, along_x);
        field.local = select(local.yx, local, along_x);
        field.size = select(size.yx, size, along_x);
        // A district's grid ends at a road or a track as well.
        field.edge = min(field.edge, districts.edge * 2600.0);
    }
    field.kind = country_kind(region, field.id, district);
    let steep = ramp(0.1, 0.2, length(inputs.gradient));
    if field.kind != FIELD_WOOD && steep > 0.1 + 0.8 * fract(field.id * 9.13) {
        if region == COUNTRY_JAPAN || region == COUNTRY_TROPICS {
            field.kind = FIELD_WOOD;
        } else if region == COUNTRY_EAST || region == COUNTRY_BRITAIN || region == COUNTRY_EUROPE {
            field.kind = FIELD_PASTURE;
        } else {
            field.kind = FIELD_ROUGH;
        }
    }
    // Along the shore a yard rather than fields (the tropics' palms run down to the beach).
    let coast_byte = (inputs.coast - 0.5) * 2.0;
    let shore = 1.0 - ramp(90.0, 260.0, sign(coast_byte) * coast_byte * coast_byte * COAST_RANGE_M);
    if region != COUNTRY_TROPICS && shore > 0.1 + 0.8 * fract(field.id * 4.37) {
        field.kind = FIELD_YARD;
    }
    return field;
}

// The last field this invocation worked out, and where (`country_field`).
var<private> country_cached: CountryField;
var<private> country_cached_at: vec2<f32>;
var<private> country_cached_set: bool;

// The field of the period's country at a point (`country_field_worked`), worked out once however often a pixel asks
// (each cover lookup and the ground).
fn country_field(inputs: SurfaceInputs) -> CountryField {
    if !country_cached_set || any(inputs.chart != country_cached_at) {
        country_cached = country_field_worked(inputs);
        country_cached_at = inputs.chart;
        country_cached_set = true;
    }
    return country_cached;
}

// The window's rural colours for the country (`terrain.cover_means`, linear with the haze out): grass (else moss,
// shrubs, fields), fields, trees, bare ground and shrubs, each held to a plausible luminance, with a stand-in where the
// window has none of a class.
struct CountryMeans {
    grass: vec3<f32>,
    crop: vec3<f32>,
    tree: vec3<f32>,
    bare: vec3<f32>,
    shrub: vec3<f32>,
}

fn luminance_of(c: vec3<f32>) -> f32 {
    return max(dot(c, LUMA), 1e-5);
}

// `c` with its luminance held between `low` and `high`.
fn held(c: vec3<f32>, low: f32, high: f32) -> vec3<f32> {
    let l = luminance_of(c);
    return c * (clamp(l, low, high) / l);
}

// Luminance `l` in the hue of `hue`.
fn hued(l: f32, hue: vec3<f32>) -> vec3<f32> {
    return hue * (l / dot(hue, LUMA));
}

fn country_means() -> CountryMeans {
    let m = terrain.cover_means;
    var grass = vec3<f32>(0.045, 0.055, 0.028);
    if m[4].w > 0.5 {
        grass = m[4].rgb;
    }
    if m[2].w > 0.5 {
        grass = m[2].rgb;
    }
    if m[10].w > 0.5 {
        grass = m[10].rgb;
    }
    if m[3].w > 0.5 {
        grass = m[3].rgb;
    }
    var out: CountryMeans;
    out.grass = held(grass, 0.025, 0.11);
    out.crop = held(select(out.grass * vec3<f32>(0.95, 1.05, 0.9), m[4].rgb, m[4].w > 0.5), 0.03, 0.11);
    out.tree = held(select(vec3<f32>(0.018, 0.026, 0.012), m[1].rgb, m[1].w > 0.5), 0.01, 0.045);
    out.bare = held(select(vec3<f32>(0.13, 0.11, 0.085), m[6].rgb, m[6].w > 0.5), 0.06, 0.3);
    out.shrub = held(select(mix(out.grass, out.tree, 0.4), m[2].rgb, m[2].w > 0.5), 0.015, 0.09);
    return out;
}

// Ploughed earth: darker than the window's bare ground (a field's soil, not a dry wash's), in the region's hue.
fn country_soil(region: u32, means: CountryMeans) -> vec3<f32> {
    let crop = luminance_of(means.crop);
    let l = clamp(luminance_of(means.bare) * 0.55, crop * 0.9, crop * 1.9);
    switch region {
        case COUNTRY_WEST: {
            // The valleys' light adobe loam.
            return hued(l * 1.12, vec3<f32>(1.28, 1.0, 0.76));
        }
        case COUNTRY_JAPAN: {
            // Dark volcanic loam.
            return hued(l * 0.72, vec3<f32>(1.2, 1.0, 0.82));
        }
        case COUNTRY_TROPICS: {
            return hued(l * 0.85, vec3<f32>(1.5, 0.95, 0.66));
        }
        case COUNTRY_NORDIC: {
            // Peaty and dark.
            return hued(l * 0.62, vec3<f32>(1.2, 1.0, 0.86));
        }
        case COUNTRY_MEDITERRANEAN: {
            // Red earth.
            return hued(l * 1.05, vec3<f32>(1.45, 0.98, 0.64));
        }
        default: {}
    }
    return hued(l * 0.85, vec3<f32>(1.3, 1.0, 0.74));
}

// How dry the summer has left the grass and the stubble at `months` (0-1): the West's and the Mediterranean's from May
// to October, elsewhere the climate's own dry summer.
fn country_dry(region: u32, months: f32) -> f32 {
    let summer = cos((months - 6.5) * (TAU / 12.0));
    if region == COUNTRY_WEST || region == COUNTRY_MEDITERRANEAN {
        return max(ramp(-0.35, 0.45, summer), terrain.climate.w);
    }
    if region == COUNTRY_TROPICS {
        return 0.0;
    }
    return terrain.climate.w;
}

// The month (since mid-January) a region's grain is cut: the dry countries' winter wheat and barley in early June,
// Britain's in August.
fn country_harvest(region: u32) -> f32 {
    switch region {
        case COUNTRY_WEST, COUNTRY_MEDITERRANEAN, COUNTRY_JAPAN: {
            return 4.6;
        }
        case COUNTRY_EAST: {
            return 6.0;
        }
        case COUNTRY_EUROPE: {
            return 6.5;
        }
        case COUNTRY_BRITAIN: {
            return 6.8;
        }
        case COUNTRY_NORDIC: {
            return 7.4;
        }
        default: {}
    }
    return 6.0;
}

// The ground of the period's country at a point: colour (albedo, calibrated as the photograph's), roughness, grazing
// reflectance, relief (m), the share under trees' crowns, and the ground materials (grass, dry grass, ploughed soil,
// mud; forest floor, scrub).
struct CountryGround {
    color: vec3<f32>,
    roughness: f32,
    specular: f32,
    bump: f32,
    crowns: f32,
    mats: vec4<f32>,
    mats_more: vec2<f32>,
}

// A field's trees in rows (an orchard's, a grove's, a plantation's) on the grid the trees' placement stands its own on
// (`tree_place.wgsl`: the slots' spacing along the rows' heading, from the chart's origin), a crown every
// `spacing / k` (about `desired` m): the crown's share at the point, its dome (0-1), and the crowns' mean share.
struct Orchard {
    crown: f32,
    dome: f32,
    cover: f32,
}

fn country_orchard(inputs: SurfaceInputs, field: CountryField, desired: f32, radius: f32, star: f32) -> Orchard {
    let slot = max(terrain.field.w, 1.0);
    let spacing = slot / max(round(slot / desired), 1.0);
    var o: Orchard;
    o.cover = min(3.14159265 * radius * radius / (spacing * spacing), 0.9);
    o.crown = o.cover;
    o.dome = 0.0;
    let near = resolved(inputs.footprint, spacing);
    if near <= 0.0 {
        return o;
    }
    let across = vec2<f32>(-field.along.y, field.along.x);
    let q = vec2<f32>(dot(inputs.chart, field.along), dot(inputs.chart, across)) / spacing;
    let d = (q - round(q)) * spacing;
    let at = length(d);
    // A palm's fronds: the crown's edge in points.
    let reach = radius * (1.0 - star * (0.5 + 0.5 * cos(atan2(d.y, d.x) * 9.0 + field.id * 40.0)));
    let soft = max(inputs.footprint * 0.6, 0.25);
    let inside = ramp(reach + soft, reach - soft, at);
    o.crown = mix(o.cover, inside, near);
    o.dome = sqrt(max(1.0 - (at * at) / (radius * radius), 0.0)) * near;
    return o;
}

fn country_ground(inputs: SurfaceInputs, field: CountryField, grass_detail: f32, grass_fine: f32,
    mottle: f32) -> CountryGround {
    let region = country_region();
    let means = country_means();
    let footprint = inputs.footprint;
    let j1 = fract(field.id * 13.17) - 0.5;
    let j2 = fract(field.id * 29.71) - 0.5;
    // Each field a little ahead of the season or behind it.
    let months = terrain.country.y + j2 * 0.9;
    let dry = country_dry(region, months);
    // The northern countries' winter: grass duller, woods bare (the West, the Mediterranean and the tropics green or
    // dry instead).
    let seasonal = region == COUNTRY_EAST || region == COUNTRY_BRITAIN || region == COUNTRY_EUROPE
        || region == COUNTRY_NORDIC || region == COUNTRY_JAPAN;
    let winter = select(0.0, 1.0 - terrain.climate.z, seasonal);
    let soil = country_soil(region, means) * (1.0 + j1 * 0.25);
    let grass_l = luminance_of(means.grass);
    let crop_l = luminance_of(means.crop);
    let green = mix(means.grass, hued(grass_l * 1.05, vec3<f32>(0.8, 1.0, 0.5)), 0.5);
    let tawny = hued(grass_l * 1.25 + 0.006, vec3<f32>(1.32, 1.0, 0.58));
    let grass = wintered(mix(green, tawny, dry), winter * 0.5) * (1.0 + j1 * 0.2)
        * vec3<f32>(1.0 + j2 * 0.08, 1.0, 1.0 - j2 * 0.12);
    let crop_green = mix(means.crop, hued(crop_l * 1.05, vec3<f32>(0.78, 1.0, 0.46)), 0.45);
    let ripe = hued(crop_l * 1.6 + 0.01, vec3<f32>(1.3, 1.05, 0.52));
    let stubble = hued(crop_l * 1.75 + 0.012, vec3<f32>(1.22, 1.06, 0.78));
    var color = grass;
    var roughness = 0.95;
    var specular = 0.35;
    var bump = 0.0;
    var crowns = 0.0;
    // Grass, dry grass, ploughed soil, mud; forest floor, scrub.
    var mats = vec4<f32>(1.0 - dry, dry, 0.0, 0.0);
    var mats_more = vec2<f32>(0.0);
    // Furrows and rows across the field: their phase (m across).
    let across = field.local.y;
    switch field.kind {
        case FIELD_PASTURE: {
            color = grass * textured(grass_detail, footprint, 15.0, 0.8) * grass_fine;
        }
        case FIELD_HAY: {
            // Brighter and greener than the pasture (the dry countries' alfalfa watered, green all summer); mown in
            // swaths after midsummer.
            let watered = region == COUNTRY_WEST || region == COUNTRY_MEDITERRANEAN;
            var lush = select(green * 1.12, hued(grass_l * 1.2 + 0.004, vec3<f32>(0.7, 1.0, 0.4)), watered);
            if region == COUNTRY_NORDIC {
                lush = hued(max(grass_l * 1.3, 0.06), vec3<f32>(0.82, 1.0, 0.46));
            }
            let hay = wintered(mix(lush, tawny, dry * select(0.4, 0.08, watered)), winter * 0.45) * (1.0 + j1 * 0.12);
            let mown = select(ramp(5.2, 5.6, months) * (1.0 - ramp(7.5, 8.0, months)), 0.5, region == COUNTRY_WEST);
            let swaths = sin(across * (TAU / 4.5)) * resolved(footprint, 4.5) * mown;
            color = hay * (1.0 + swaths * 0.07) * textured(grass_detail, footprint, 15.0, 0.5) * grass_fine;
        }
        case FIELD_GRAIN: {
            // Sown, green, ripe, cut to stubble, ploughed, sown again (`country_harvest`).
            let d = fract((months - country_harvest(region)) / 12.0 + 0.5) * 12.0 - 6.0;
            let sown = mix(soil, crop_green, 0.3);
            var c = mix(sown, crop_green, ramp(-6.0, -3.0, d));
            c = mix(c, ripe, ramp(-1.3, -0.1, d));
            c = mix(c, stubble, ramp(0.3, 0.9, d));
            c = mix(c, soil, ramp(3.4, 4.4, d));
            c = mix(c, sown, ramp(5.0, 6.0, d));
            let bare = ramp(3.4, 4.4, d) * (1.0 - ramp(5.0, 6.0, d)) + (1.0 - ramp(-6.0, -3.0, d)) * 0.7;
            let golden = ramp(-1.3, -0.1, d) * (1.0 - ramp(3.4, 4.4, d));
            // Drill rows and the stubble's lines, a ripe field's sheen in the wind.
            let rows = sin(across * (TAU / 2.2)) * resolved(footprint, 2.2);
            var sheen = 1.0;
            if golden > 0.0 && resolved(footprint, 40.0) > 0.0 {
                sheen = 1.0 + (noise_r(inputs, 40.0) - 0.5) * 0.12 * golden * resolved(footprint, 40.0);
            }
            color = c * (1.0 + j1 * 0.12) * (1.0 + rows * (0.03 + 0.05 * bare)) * sheen * mix(1.0, grass_fine, 0.5);
            mats = vec4<f32>((1.0 - bare) * (1.0 - golden), golden * (1.0 - bare), bare, 0.0);
        }
        case FIELD_PLOUGHED: {
            var damp = 1.0;
            if resolved(footprint, 37.0) > 0.0 {
                damp = 1.0 + (noise_r(inputs, 37.0) - 0.5) * 0.3 * resolved(footprint, 37.0);
            }
            let furrows = sin(across * (TAU / 1.5)) * resolved(footprint, 1.5);
            color = soil * damp * (1.0 + furrows * 0.14) * mottle;
            bump = furrows * 0.08;
            mats = vec4<f32>(0.0, 0.0, 1.0, 0.0);
        }
        case FIELD_TRUCK: {
            // Strips of vegetables across the field, each its own crop: lettuce, cabbage, beans, a strip just
            // ploughed; the Mediterranean's vines in rows over pale earth.
            let width = select(select(8.0, 4.0, region == COUNTRY_JAPAN), 2.6, region == COUNTRY_MEDITERRANEAN);
            let strip = corner_offset(vec2<f32>(floor(across / width), field.id * 6151.0)).x;
            let leafy = hued(crop_l * 1.15, vec3<f32>(0.72, 1.0, 0.42));
            let dark = hued(crop_l * 0.8, vec3<f32>(0.7, 1.0, 0.72));
            var strip_color = select(select(select(leafy, dark, strip < 0.5), soil, strip < 0.25),
                mix(soil, crop_green, 0.55), strip < 0.12);
            var mean = mix(mix(leafy, dark, 0.4), soil, 0.35);
            if region == COUNTRY_MEDITERRANEAN {
                let vine = ramp(0.75, 0.55, abs(fract(across / width) - 0.5) * 2.0);
                let earth = mix(soil, tawny, 0.4);
                strip_color = mix(earth, dark, vine);
                mean = mix(earth, dark, 0.35);
            }
            let near = resolved(footprint, width * 1.5);
            let rows = sin(across * (TAU / 0.9)) * resolved(footprint, 0.9);
            color = mix(mean, strip_color, near) * (1.0 + rows * 0.06) * mottle;
            mats = vec4<f32>(0.5, 0.0, 0.5, 0.0);
        }
        case FIELD_ORCHARD, FIELD_OLIVES, FIELD_PALMS: {
            // Trees in rows over cultivated earth (citrus, walnuts) or grass (apples, palms), olives over pale earth.
            var desired = 8.0;
            var radius = 3.0;
            var star = 0.0;
            var height = 3.0;
            var crown = means.tree * 1.2;
            var ground = grass;
            if field.kind == FIELD_PALMS {
                desired = 9.0;
                radius = 3.8;
                star = 0.35;
                height = 2.0;
                crown = hued(luminance_of(means.tree) * 1.35, vec3<f32>(0.85, 1.0, 0.55));
                ground = mix(grass, green, 0.5);
            } else if field.kind == FIELD_OLIVES {
                desired = 9.0;
                radius = 3.0;
                crown = hued(luminance_of(means.tree) * 1.5 + 0.004, vec3<f32>(0.92, 1.0, 0.84));
                ground = mix(soil, tawny, 0.6);
            } else if region == COUNTRY_WEST || region == COUNTRY_MEDITERRANEAN {
                if fract(field.id * 7.7) < 0.7 {
                    // Oranges and lemons: dark, glossy, close.
                    desired = 7.0;
                    radius = 2.7;
                    height = 2.5;
                    crown = means.tree * vec3<f32>(0.85, 0.98, 0.8);
                    ground = soil * 1.08;
                } else {
                    // Walnuts: broad and lighter.
                    desired = 11.0;
                    radius = 4.6;
                    height = 4.0;
                    crown = means.tree * vec3<f32>(1.3, 1.35, 1.1);
                    ground = mix(soil, tawny, 0.5);
                }
            } else if region == COUNTRY_JAPAN {
                // Mulberry and tea: low bushes close in rows.
                desired = 3.0;
                radius = 1.2;
                height = 1.0;
                crown = hued(luminance_of(means.tree) * 1.4, vec3<f32>(0.75, 1.0, 0.6));
                ground = soil;
            }
            crown = wintered(crown, winter * select(0.6, 0.0, field.kind == FIELD_PALMS || region == COUNTRY_WEST
                || region == COUNTRY_MEDITERRANEAN));
            let o = country_orchard(inputs, field, desired, radius, star);
            let shaded = crown * (0.7 + 0.4 * o.dome);
            color = mix(ground * mottle, shaded, o.crown);
            bump = o.dome * height;
            crowns = o.crown;
            let earth = select(0.0, 1.0, field.kind == FIELD_OLIVES || ground.r > ground.g);
            mats = vec4<f32>((1.0 - earth) * (1.0 - o.crown), 0.0, earth * (1.0 - o.crown), 0.0);
            mats_more = vec2<f32>(o.crown, 0.0);
        }
        case FIELD_PADDY: {
            // Plots between bunds, each a little ahead or behind: in winter stubble or barley, flooded in May, the
            // young rice through the water in June, green all summer, gold in September, cut in October.
            let plot = vec2<f32>(32.0, 16.0);
            let q = field.local / plot;
            let cell = floor(q);
            let ph = corner_offset(cell + field.id * 2213.0);
            let m = months + (ph.x - 0.5) * 1.3;
            let water = hued(0.026, vec3<f32>(0.85, 1.0, 1.08));
            let rice = hued(crop_l * 1.3 + 0.008, vec3<f32>(0.74, 1.0, 0.36));
            let gold = hued(crop_l * 1.55 + 0.01, vec3<f32>(1.25, 1.06, 0.46));
            let cut = hued(luminance_of(soil) * 0.9, vec3<f32>(1.18, 1.0, 0.8));
            let barley = ph.y < 0.25;
            var winter_plot = mix(cut, soil, 0.35);
            if barley {
                winter_plot = mix(mix(soil, crop_green, 0.6), ripe, ramp(3.6, 4.3, m));
            }
            let flooded = ramp(3.6, 4.1, m) * (1.0 - ramp(8.4, 8.8, m));
            var c = mix(winter_plot, water, flooded);
            c = mix(c, mix(water, rice, 0.3), ramp(4.1, 4.6, m));
            c = mix(c, rice, ramp(4.6, 5.9, m));
            c = mix(c, gold, ramp(7.6, 8.3, m));
            c = mix(c, cut, ramp(8.6, 9.1, m));
            c = mix(c, winter_plot, ramp(9.5, 10.5, m));
            // The water shows where the rice has not closed over it.
            let wet = flooded * (1.0 - ramp(4.8, 6.2, m));
            let near = resolved(footprint, 16.0);
            let mean = mix(mix(cut, soil, 0.35), mix(water, rice, 0.5), flooded);
            let bund_near = resolved(footprint, 3.0);
            let e = min(min(fract(q.x), 1.0 - fract(q.x)) * plot.x, min(fract(q.y), 1.0 - fract(q.y)) * plot.y);
            let soft = max(footprint * 0.5, 0.15);
            let bund = ramp(0.45 + soft, 0.45 - soft, e) * bund_near;
            let bank = mix(grass, soil, 0.3) * 1.08;
            color = mix(mix(mean, c * (1.0 + (ph.y - 0.5) * 0.24), near), bank, bund);
            // Far off the bunds' share (a twelfth of the land) lightens the plots as their grass does.
            color = mix(color, bank, (1.0 - bund_near) * 0.08);
            roughness = mix(0.9, 0.1, wet * (1.0 - bund));
            specular = mix(0.35, 1.0, wet * (1.0 - bund));
            bump = bund * 0.25;
            mats = vec4<f32>((1.0 - flooded) * 0.3 + bund, 0.0, 0.0, (1.0 - bund) * (flooded + (1.0 - flooded) * 0.7));
        }
        case FIELD_GARDEN: {
            // Small plots of mixed crops, bare earth and fallow.
            let q = field.local / vec2<f32>(14.0, 10.0);
            let h = corner_offset(floor(q) + field.id * 3511.0);
            let leafy = hued(crop_l * 1.1, vec3<f32>(0.75, 1.0, 0.45));
            let plot = select(select(select(leafy, means.tree * 1.4, h.x < 0.55), soil, h.x < 0.3), grass, h.x < 0.12);
            let mean = mix(mix(leafy, soil, 0.4), means.tree * 1.4, 0.2);
            color = mix(mean, plot * (1.0 + (h.y - 0.5) * 0.15), resolved(footprint, 14.0)) * mottle;
            mats = vec4<f32>(0.5, 0.0, 0.5, 0.0);
        }
        case FIELD_WOOD: {
            // A woodlot, a village's grove: the canopy's crowns, bare in a northern winter (Japan's evergreen).
            let trees = canopy(inputs, 9.0, 6.0);
            let bare = winter * select(0.7, 0.0, region == COUNTRY_JAPAN);
            let wood = mix(means.tree * (1.0 + j1 * 0.2), hued(luminance_of(means.tree) * 1.3, vec3<f32>(1.15, 1.0, 0.85)),
                bare);
            color = wood * trees.shade * trees.tint;
            bump = trees.height;
            crowns = 1.0;
            mats = vec4<f32>(0.0);
            mats_more = vec2<f32>(1.0, 0.0);
        }
        case FIELD_ROUGH: {
            // Kunai, heath and moss, garrigue: blotched, shrubs darker among the grass.
            var rough = mix(means.shrub, grass, 0.5);
            if region == COUNTRY_TROPICS {
                rough = hued(grass_l * 1.1, vec3<f32>(0.88, 1.0, 0.52));
            } else if region == COUNTRY_NORDIC {
                rough = hued(max(luminance_of(mix(means.shrub, grass, 0.5)), 0.04), vec3<f32>(1.0, 1.0, 0.8));
            } else if region == COUNTRY_MEDITERRANEAN {
                rough = mix(means.shrub, tawny, 0.5);
            }
            var blotch = 1.0;
            if resolved(footprint, 23.0) > 0.0 {
                blotch = 1.0 + (noise_r(inputs, 23.0) - 0.5) * 0.36 * resolved(footprint, 23.0);
            }
            color = rough * blotch * textured(grass_detail, footprint, 15.0, 0.8) * grass_fine;
            let scrub = select(0.3, 0.6, region == COUNTRY_MEDITERRANEAN || region == COUNTRY_NORDIC);
            mats = vec4<f32>((1.0 - scrub) * (1.0 - dry), (1.0 - scrub) * dry, 0.0, 0.0);
            mats_more = vec2<f32>(0.0, scrub);
        }
        case FIELD_YARD: {
            // Packed earth, cinders and gravel, blotched by wear and puddles; the dry countries' sandier.
            let sandy = region == COUNTRY_WEST || region == COUNTRY_MEDITERRANEAN;
            let ground_l = luminance_of(means.bare) * select(0.62, 0.8, sandy);
            let yard = mix(hued(ground_l, select(vec3<f32>(1.0, 1.0, 0.96), vec3<f32>(1.12, 1.0, 0.84), sandy)), soil,
                0.25);
            var blotch = 1.0;
            if resolved(footprint, 23.0) > 0.0 {
                blotch = 1.0 + (noise_r(inputs, 23.0) - 0.5) * 0.34 * resolved(footprint, 23.0);
            }
            color = yard * blotch * mottle;
            roughness = 0.9;
            mats = vec4<f32>(0.15, 0.0, 0.85, 0.0);
        }
        default: {}
    }
    // Each open field a little lighter or darker than its neighbours, and blotched by damp and growth within.
    let open = field.kind == FIELD_PASTURE || field.kind == FIELD_HAY || field.kind == FIELD_GRAIN
        || field.kind == FIELD_PLOUGHED || field.kind == FIELD_TRUCK || field.kind == FIELD_ROUGH;
    if open {
        var tone = 1.0 + (fract(field.id * 41.3) - 0.5) * 0.28;
        let blotched = resolved(footprint, 60.0);
        if blotched > 0.0 {
            tone *= 1.0 + (noise_r(inputs, 60.0) - 0.5) * 0.16 * blotched;
        }
        color *= tone;
    }
    // The fields' edges: the West's dirt tracks, the East's fencerows, Britain's hedgerows, the open fields' balks,
    // Japan's bunds and lanes, the Nordic ditches, the Mediterranean's stone walls; the tropics' gardens unfenced.
    var edge_width = 0.0;
    var edge_color = grass;
    var edge_bump = 0.0;
    var edge_rough = 0.95;
    switch region {
        case COUNTRY_WEST: {
            edge_width = 4.0;
            edge_color = hued(luminance_of(means.bare) * 0.8 + 0.01, vec3<f32>(1.18, 1.0, 0.82));
        }
        case COUNTRY_EAST: {
            edge_width = 2.5;
            edge_color = mix(means.tree, grass, 0.4);
            edge_bump = 1.5;
        }
        case COUNTRY_BRITAIN: {
            edge_width = 4.5;
            edge_color = wintered(means.tree * 0.85, winter * 0.5);
            edge_bump = 2.5;
        }
        case COUNTRY_EUROPE: {
            edge_width = 1.5;
            edge_color = grass * 0.92;
        }
        case COUNTRY_JAPAN: {
            edge_width = 1.6;
            edge_color = mix(grass, soil, 0.35) * 1.08;
            edge_bump = 0.3;
        }
        case COUNTRY_NORDIC: {
            edge_width = 1.4;
            edge_color = hued(luminance_of(soil) * 0.5, vec3<f32>(0.92, 1.0, 1.05));
            edge_rough = 0.5;
        }
        case COUNTRY_MEDITERRANEAN: {
            edge_width = 1.2;
            edge_color = hued(luminance_of(means.bare) * 0.9 + 0.02, vec3<f32>(1.08, 1.02, 0.92));
            edge_bump = 0.8;
        }
        default: {}
    }
    if edge_width > 0.0 && field.kind != FIELD_WOOD && field.kind != FIELD_YARD {
        let soft = max(footprint * 0.5, 0.2);
        let edge_near = resolved(footprint, edge_width * 3.0);
        let on_edge = ramp(edge_width * 0.5 + soft, edge_width * 0.5 - soft, field.edge) * edge_near;
        // Far off the edges' share of the land still tints it.
        let share = (1.0 - edge_near) * min(edge_width * 2.0 / max(min(field.size.x, field.size.y), 1.0), 0.3);
        let on = max(on_edge, share);
        color = mix(color, edge_color, on);
        roughness = mix(roughness, edge_rough, on);
        specular = mix(specular, 0.35, on);
        bump = mix(bump, edge_bump, on_edge);
        let hedge = select(0.0, 1.0, region == COUNTRY_BRITAIN || region == COUNTRY_EAST);
        mats = mix(mats, vec4<f32>(1.0 - hedge, 0.0, 0.0, 0.0), on);
        mats_more = mix(mats_more, vec2<f32>(0.0, hedge), on);
        crowns = mix(crowns, hedge, on_edge);
    }
    var out: CountryGround;
    out.color = calibrated(max(color, vec3<f32>(0.0)));
    out.roughness = roughness;
    out.specular = specular;
    out.bump = bump;
    out.crowns = crowns;
    out.mats = mats;
    out.mats_more = mats_more;
    return out;
}

struct RealSurface {
    land: Layered,
    // The ground materials' weights (`materials.wgsl`): the cover's laid over the style's by the maps' weight; and how
    // much the photograph looks like a beach (bright and grey), which lets the shore's materials in.
    mats: Mats,
    beach: f32,
    occlusion: f32,
    // How much of the point the maps paint: the imagery's coverage there, 0 off their square.
    weight: f32,
    // Of `land`'s colour, the photograph's share: not the style's shore on the coast's water, nor snow (those take the
    // land's tone, `tone_down`).
    photo: f32,
    // The cover's built-up share, and the photograph's luminance over its broad sample's.
    built: f32,
    detail: f32,
    // The photograph's colour (calibrated, its detail kept), and how far it stands for the rock where the land is a
    // rock face (`rock_face`: neither snow nor ice nor water, nor as bright as them), 0-1 with the maps' weight.
    rock: vec3<f32>,
    rock_share: f32,
    // The season's snow over the point (`climate_snow`), 0-1.
    snow: f32,
}

fn add_layer(below: Layered, color: vec3<f32>, roughness: f32, specular: f32, bump: f32, weight: f32) -> Layered {
    var out: Layered;
    out.color = below.color + color * weight;
    out.roughness = below.roughness + roughness * weight;
    out.specular = below.specular + specular * weight;
    out.bump = below.bump + bump * weight;
    return out;
}

// What the surface maps hold at a point (`real_sample`): the photograph's colour and its broad colour (about six of the
// window maps' texels across, what its local contrast is measured against), both premultiplied by their coverage, the
// cover's class shares, and how much of the point the maps paint (the imagery's coverage, 0 off their square).
struct RealSample {
    sample: vec4<f32>,
    broad: vec4<f32>,
    s: CoverShares,
    weight: f32,
}

// A layer's colour read through `s` at `uv` with the derivatives `dx`, `dy` scaled by 2^`bias` (`textureSampleBias`'s
// level shift, as explicit derivatives).
fn surface_read(layer: i32, s: sampler, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>, bias: f32) -> vec4<f32> {
#ifdef LOD_ZERO
    return textureSampleLevel(surface_albedo, s, uv, layer, bias);
#else
    let k = exp2(bias);
    return textureSampleGrad(surface_albedo, s, uv, layer, dx * k, dy * k);
#endif
}

// The surface maps at a point (`terrain.surface` bound). The near tiles where they are drawn, by their coverage there,
// the window's maps elsewhere; where a near tile paints the point whole (its share and its imagery's coverage both 1)
// the window's maps beneath it are not read. Magnified (a texel over several pixels), either layer's colour is rebuilt
// from its level 0 (`magnified`), with its warp, only where it is magnified.
fn real_sample(inputs: SurfaceInputs) -> RealSample {
    let footprint = inputs.footprint;
    let far_texel = terrain.surface.z / max(terrain.surface.w, 1.0);
    var near = 0.0;
    var near_share_at = 0.0;
    var near_sample = vec4<f32>(0.0);
    var near_broad = vec4<f32>(0.0);
    if near_bound() {
        let nuv = near_uv(inputs.chart);
        let span = max(terrain.near.z * terrain.near.w, 1.0);
        // The broad sample spans the same ground on both layers: about six of the window maps' texels.
        let near_bias = select(4.0, 2.5 + log2(max(far_texel / max(near_texel_m(), 0.01), 1.0)), terrain.surface.w > 0.0);
        near_sample = surface_read(near_layer(), repeat_sampler, nuv, inputs.chart_dx / span, inputs.chart_dy / span, 0.0);
        near_broad = surface_read(near_layer(), repeat_sampler, nuv, inputs.chart_dx / span, inputs.chart_dy / span,
            near_bias);
        let near_texel = max(near_texel_m(), 0.01);
        let amount = magnified_share(footprint, near_texel);
        if amount > 0.0 {
            let warped = near_uv(magnified_warp(inputs, near_texel, amount));
            near_sample = mix(near_sample, magnified(1, repeat_sampler, warped, terrain.near.w * NEAR_TILE_TEXELS), amount);
        }
        // (Both premultiplied: where the near tiles hold no imagery the window's maps show through.)
        near_share_at = near_share(inputs.chart);
        near = near_share_at * clamp(near_sample.a, 0.0, 1.0);
    }
    var out: RealSample;
    out.sample = near_sample * near_share_at;
    out.broad = near_broad * near_share_at;
    if near < 1.0 {
        let uv = surface_uv(inputs.chart);
        let extent = max(terrain.surface.z, 1.0);
        var far_sample = surface_read(0, clamp_sampler, uv, inputs.chart_dx / extent, inputs.chart_dy / extent, 0.0);
        let far_broad = surface_read(0, clamp_sampler, uv, inputs.chart_dx / extent, inputs.chart_dy / extent, 2.5);
        if terrain.surface.w > 0.0 {
            let amount = magnified_share(footprint, far_texel);
            if amount > 0.0 {
                let warped = surface_uv(magnified_warp(inputs, far_texel, amount));
                far_sample = mix(far_sample, magnified(0, clamp_sampler, warped, terrain.surface.w), amount);
            }
        }
        let inside = surface_inside(uv);
        out.sample = far_sample * inside * (1.0 - near) + out.sample;
        out.broad = far_broad * inside * (1.0 - near) + out.broad;
        out.s = cover_shares(inputs);
        if near > 0.0 {
            out.s = mixed_shares(out.s, near_cover_shares(inputs), near);
        }
    } else {
        out.s = near_cover_shares(inputs);
    }
    out.weight = clamp(out.sample.a, 0.0, 1.0);
    return out;
}

// The land painted from the surface maps (`terrain.surface` bound, `real` their sample here): the photograph's colour,
// calibrated and with its own relief shading lessened where the land slopes (our light shades the relief), as the base
// of every class; the cover's classes choose the small-scale structure laid over it near the camera (canopy, furrows,
// roofs and streets where no real buildings stand, rock and sand, snow) and its roughness and relief, each worked out
// only where its class is present. Far off every pattern fades to its mean and the photograph alone remains.
// `procedural`, `procedural_mats`: the style's own cover beneath (shown on the coast's water where the photograph is no
// beach); `details`: the grass, sand and snow photographs' luminance (1 their mean).
fn real_surface(inputs: SurfaceInputs, g: Ground, real: RealSample, procedural: Layered, procedural_mats: Mats,
    details: vec3<f32>) -> RealSurface {
    let footprint = inputs.footprint;
    let sample = real.sample;
    let broad = real.broad;
    let s = real.s;
    var out: RealSurface;
    out.weight = real.weight;
    // The colour is premultiplied by its coverage; the haze the mosaic kept comes out (its blue cast, its veil).
    let haze = terrain.surface_haze.rgb;
    var raw = max(sample.rgb / max(sample.a, 1e-3) - haze, vec3<f32>(0.0));
    var raw_broad = max(broad.rgb / max(broad.a, 1e-3) - haze, vec3<f32>(0.0));
    // A cloud the mosaic kept, over green land: far brighter than the window's own colour for the class and grey. The
    // class's colour stands in (the patterns below give it structure). Towns, bare ground and snow may be bright.
    let typical = typical_color(s);
    let raw_luminance = max(dot(raw, LUMA), 1e-4);
    let spread = (max(max(raw.r, raw.g), raw.b) - min(min(raw.r, raw.g), raw.b)) / raw_luminance;
    let green = 1.0 - (s.b.y + s.b.z + s.b.w + s.c.z);
    let cloud = ramp(2.2, 3.5, raw_luminance / max(dot(typical.rgb, LUMA), 1e-3)) * ramp(0.5, 0.25, spread)
        * green * typical.w;
    raw = mix(raw, typical.rgb, cloud);
    raw_broad = mix(raw_broad, typical.rgb, cloud);
    // The sun of the photograph lit the slopes facing it and shaded the rest; ours does again. Where the land slopes,
    // most of the local contrast goes (on the flat it is fields, roads and hedges, and stays).
    let local = max(dot(raw_broad, LUMA), 1e-4) / max(dot(raw, LUMA), 1e-4);
    let relief = ramp(0.05, 0.35, g.slope) * 0.6;
    // The calibration compresses the luminance (`PHOTO_GAMMA`), and with it every detail's contrast against its
    // surroundings (a road, a roof, a field's edge against the broad sample): `PHOTO_DETAIL` of the exponent comes back
    // to the detail, the broad colour calibrated as before.
    let photo = calibrated(raw * pow(local, relief))
        * pow(clamp(1.0 / local, 0.25, 4.0), PHOTO_DETAIL * (1.0 - relief / 0.6));
    // Ground's own small structure where the pixel can hold it, under every open class: soil and growth mottled at a
    // few metres, finer than the photograph's 10 m (`MOTTLE_M`), and the grass photograph's grain a second time at a
    // quarter of its scale. Each read only while it shows at the pixel's footprint (as every pattern below).
    var mottle = 1.0;
    let mottle_near = resolved(footprint, MOTTLE_M);
    if mottle_near > 0.0 {
        mottle = mix(1.0, (noise_r(inputs, MOTTLE_M) - 0.5) * 0.3 + (noise_r(inputs, MOTTLE_M * 0.37) - 0.5) * 0.16
            + 1.0, mottle_near);
    }
    var grass_fine = mottle;
    if resolved(footprint, 3.7) > 0.0 {
        grass_fine = textured(detail(inputs, 3.7).g * 2.0, footprint, 3.7, 0.55) * mottle;
    }
    let grass_detail = details.x;
    let sand_detail = details.y;
    let snow_detail = details.z;
    let bare_season = leafless();

    // Each class's own pattern is worked out only where the class is present: elsewhere it weighs nothing.
    // Woods: the canopy's crowns over the photograph's colour (bare and grey-brown in a seasonal style's winter).
    var trees: Canopy;
    trees.shade = 1.0;
    trees.height = 0.0;
    trees.tint = 1.0;
    if s.a.x + s.a.y + s.a.z > 0.0 {
        trees = canopy(inputs, 11.0, 5.0);
    }
    let crowns = trees.shade * trees.tint;
    let forest = mix(photo, srgb(74.0, 67.0, 61.0) * 0.9, bare_season * 0.75) * crowns;
    let mangrove = photo * 0.9 * crowns;
    let shrub = wintered(photo, bare_season * 0.4) * mix(1.0, crowns, 0.55) * textured(grass_detail, footprint, 15.0, 0.6)
        * grass_fine;
    let open = wintered(photo, bare_season * 0.45) * textured(grass_detail, footprint, 15.0, 1.0)
        * ((g.speck - 0.5) * 0.12 + 1.0) * grass_fine;
    // Fields: furrows on a heading of each field's own in half of them (the rest in grass or stubble), the grass
    // photograph's grain at half strength.
    var field = 0.0;
    let furrows_near = resolved(footprint, 4.0);
    if s.b.x > 0.0 && furrows_near > 0.0 {
        field = noise_whole(inputs, 3600.0).a;
    }
    let heading = field * 37.7;
    let along = dot(inputs.chart, vec2<f32>(cos(heading), sin(heading))) / 4.0 * (2.0 * 3.141592653589793);
    let furrows = sin(along) * furrows_near * ramp(0.45, 0.55, fract(field * 7.0));
    let crop = wintered(photo, bare_season * 0.35) * (furrows * 0.07 + 1.0) * textured(grass_detail, footprint, 15.0, 0.5)
        * mix(1.0, grass_fine, 0.6);
    // The period's country (`country_shares`): of the cropland, the share that is its fields (`country_ground`), drawn
    // in place of the photograph (today's roofs, pools and streets beneath them). Nothing read where none is.
    var past = 0.0;
    var country: CountryGround;
    var crop_layer = crop;
    var crop_roughness = 0.95;
    var crop_specular = 0.35;
    var crop_bump = furrows * 0.15;
    if s.past > 0.0 {
        past = clamp(s.past / max(s.b.x, 1e-4), 0.0, 1.0);
        country = country_ground(inputs, country_field(inputs), grass_detail, grass_fine, mottle);
        crop_layer = mix(crop, country.color, past);
        crop_roughness = mix(0.95, country.roughness, past);
        crop_specular = mix(0.35, country.specular, past);
        crop_bump = mix(furrows * 0.15, country.bump, past);
    }
    // Towns. Where real buildings stand on the land (`buildings_stand`: a buildings layer drawn with it) the
    // photograph with the ground's own mottle, flat: the houses are the buildings, the streets the photograph's and the
    // map's roads. Elsewhere blocks between streets, each laid out in lots on a heading of its own, a roof on every lot
    // (its own tone, a few of tile) in a yard.
    var built = photo * mottle;
    var roofs_height = 0.0;
    if !buildings_stand() && s.b.y > 0.0 {
        // (Beyond the streets' and lots' reach, 13.5 m a pixel, the photograph alone: nothing read.)
        built = photo;
    }
    if !buildings_stand() && s.b.y > 0.0 && resolved(footprint, 27.0) > 0.0 {
        let blocks = parcels(inputs, 80.0, 9.0);
        let street = blocks.boundary * blocks.edge_near;
        let turn = blocks.id * 6.2831853;
        let along_block = vec2<f32>(cos(turn), sin(turn));
        let lot = vec2<f32>(dot(inputs.chart, along_block), dot(inputs.chart, vec2<f32>(-along_block.y, along_block.x)))
            / (vec2<f32>(15.0, 11.0) * (fract(blocks.id * 13.7) * 0.5 + 0.8));
        let lot_id = corner_offset(floor(lot) + floor(blocks.id * 977.0));
        let half_roof = (lot_id.x * 0.25 + 0.5) * 0.5;
        let from_middle = abs(fract(lot) - 0.5);
        // The roof's edge, softened to a pixel's width so it never aliases.
        let soft = max(inputs.footprint / 11.0, 0.02);
        let near_lots = resolved(footprint, 6.0);
        // One lot in six is a yard, a garden or a gap.
        let roofed = ramp(half_roof + soft, half_roof - soft, max(from_middle.x, from_middle.y)) * near_lots
            * ramp(0.15, 0.17, fract(lot_id.x * 5.3));
        let tiles = ramp(0.62, 0.66, lot_id.y);
        let roof = photo * (lot_id.y * 0.6 + 0.75) * mix(vec3<f32>(1.0), vec3<f32>(1.3, 0.9, 0.75), tiles);
        let lum = dot(photo, LUMA);
        built = mix(mix(photo, mix(photo * 0.85, roof, roofed), near_lots), vec3<f32>(lum * 0.62), street);
        roofs_height = (1.0 - street) * roofed * 1.5;
    }
    // Snow and ice: the bare ground's rock where a face is too steep to hold them (from about 40 degrees to 48, where
    // the window's 40 m cells draw a wall of 55-70; past 49 the rock faces cover every class), glacier ice elsewhere
    // (`glacier_ice`) for the season's snow (below) to lie on.
    let ice_rock = s.b.w * ramp(0.84, 1.1, g.slope + g.jag * 0.15);
    let ice = s.b.w - ice_rock;
    // Bare ground: sand and gravel on the flat, rock with its photograph's structure on the slopes.
    let steep = ramp(0.3, 0.6, g.slope + g.jag * 0.1);
    var stone: Rock;
    stone.color = photo;
    stone.height = 0.0;
    if s.b.z + ice_rock > 0.0 {
        stone = rock(inputs, photo, photo * 0.75);
    }
    let bare = mix(photo * textured(sand_detail, footprint, 30.0, 1.0) * mottle, stone.color, steep);
    let snow_color = srgb(232.0, 237.0, 241.0) * textured(snow_detail, footprint, 60.0, 0.45);
    var glacier = base_cover(snow_color, 0.75, 0.45, 0.0);
    if ice > 0.0 {
        glacier = glacier_ice(inputs, g, glacier);
    }
    // Inland water, dark and smooth. Water the cover puts on our land by the sea is the coast missing the photograph's
    // by a texel or two: the style's shore there.
    let inland = ramp(100.0, 300.0, g.coast);
    let water = photo * 0.55;
    // How much the photograph looks like a beach (bright and grey: sand, shingle, a dry riverbed). Water the cover puts
    // on our land by the sea is the photograph's beach where it shows one, and the style's shore where it does not.
    let photo_l = max(dot(photo, LUMA), 1e-4);
    let chroma = (max(max(photo.r, photo.g), photo.b) - min(min(photo.r, photo.g), photo.b)) / photo_l;
    // (The photograph's 0.062-0.129 of luminance and 0.33-0.67 of chroma, as calibrated.)
    let beachy = ramp(0.06, 0.107, photo_l) * ramp(0.6, 0.3, chroma);
    let coast_water = s.c.x * (1.0 - inland);
    let wetland = photo * 0.82 * textured(grass_detail, footprint, 15.0, 0.7) * grass_fine;

    var land = base_cover(vec3<f32>(0.0), 0.0, 0.0, 0.0);
    land = add_layer(land, forest, 0.95, 0.25, trees.height, s.a.x);
    land = add_layer(land, mangrove, 0.9, 0.35, trees.height * 0.7, s.a.y);
    land = add_layer(land, shrub, 0.95, 0.3, trees.height * 0.4, s.a.z);
    land = add_layer(land, open, 0.93, 0.35, 0.0, s.a.w + s.c.z);
    land = add_layer(land, crop_layer, crop_roughness, crop_specular, crop_bump, s.b.x);
    land = add_layer(land, built, 0.8, 0.4, roofs_height, s.b.y);
    land = add_layer(land, bare, mix(0.9, 0.85, steep), 0.45, stone.height * steep, s.b.z + ice_rock);
    land = add_layer(land, glacier.color, glacier.roughness, glacier.specular, glacier.bump, ice);
    land = add_layer(land, water, 0.12, 1.0, 0.0, s.c.x * inland);
    land = add_layer(land, photo * textured(sand_detail, footprint, 30.0, 1.0) * mottle, 0.9, 0.45, 0.0, coast_water * beachy);
    land = add_layer(land, procedural.color, procedural.roughness, procedural.specular, procedural.bump,
        coast_water * (1.0 - beachy));
    land = add_layer(land, wetland, 0.6, 0.6, 0.0, s.c.y);
    // The season's snow over it all, thinner over woods (the crowns stand dark through it).
    let woods = s.a.x + s.a.y;
    let snow = climate_snow(inputs, g) * (1.0 - woods * 0.5) * one(s.c.x * inland);
    land = cover(land, snow_color, 0.75, 0.45, land.bump * 0.3, snow);
    out.land = land;
    out.rock = photo;
    out.rock_share = clamp(1.0 - s.b.w - s.c.x, 0.0, 1.0) * ramp(0.32, 0.22, photo_l) * out.weight;
    out.snow = snow;
    out.photo = clamp(1.0 - coast_water * (1.0 - beachy) - ice, 0.0, 1.0) * (1.0 - snow);
    out.built = s.b.y;
    out.detail = clamp(1.0 / local, 0.25, 4.0);
    // The ground materials by class. Grass and scrub dry by the climate's summer or where the photograph is tawny
    // rather than green (summer-brown California, stubble); fields ploughed earth where it is brown; bare ground rock on
    // slopes, sand by the sea and stony dry ground inland.
    let tawny = ramp(-0.19, 0.048, (photo.r - photo.g) / photo_l);
    let dry = max(terrain.climate.w, tawny);
    let coastal = one(ramp(300.0, 1500.0, g.coast));
    // Laid over the style's own (`procedural_mats`) by the maps' weight as the colour is (`terrain_surface`), in place,
    // so the two sets are never held at once.
    let w = out.weight * photo_kept(g);
    var mats = mats_scaled(procedural_mats, 1.0 - w * (1.0 - coast_water * (1.0 - beachy)));
    mats = mats_add(mats, M_FOREST, (s.a.x + s.a.y * 0.4) * w);
    mats = mats_add(mats, M_MUD, (s.a.y * 0.6 + s.c.y) * w);
    mats = mats_add(mats, M_SCRUB, s.a.z * (1.0 - dry) * w);
    mats = mats_add(mats, M_DRY_SCRUB, (s.a.z * dry + s.b.z * (1.0 - steep) * (1.0 - coastal)) * w);
    // (The period's country takes its fields' own.)
    var fields = s.b.x;
    if s.past > 0.0 {
        fields = s.b.x * (1.0 - past);
        let cw = s.b.x * past * w;
        mats = mats_add(mats, M_GRASS, country.mats.x * cw);
        mats = mats_add(mats, M_DRY_GRASS, country.mats.y * cw);
        mats = mats_add(mats, M_SOIL, country.mats.z * cw);
        mats = mats_add(mats, M_MUD, country.mats.w * cw);
        mats = mats_add(mats, M_FOREST, country.mats_more.x * cw);
        mats = mats_add(mats, M_SCRUB, country.mats_more.y * cw);
    }
    mats = mats_add(mats, M_GRASS, ((s.a.w + s.c.z) * (1.0 - dry) + fields * (1.0 - tawny)) * w);
    mats = mats_add(mats, M_DRY_GRASS, (s.a.w + s.c.z) * dry * w);
    mats = mats_add(mats, M_SOIL, fields * tawny * w);
    mats = mats_add(mats, M_PAVING, s.b.y * w);
    mats = mats_add(mats, M_ROCK, (s.b.z * steep + ice_rock) * w);
    mats = mats_add(mats, M_SAND, (s.b.z * (1.0 - steep) * coastal + coast_water * beachy) * w);
    mats = mats_add(mats, M_SNOW, ice * w);
    mats = mats_add(mats, M_NONE, s.c.x * inland * w);
    out.mats = mats_cover(mats, M_SNOW, snow * w);
    out.beach = max(beachy, s.b.z + s.c.x);
    out.occlusion = mix(1.0, min(trees.shade, 1.0) * 0.3 + 0.7, woods + s.a.z * 0.5);
    if s.past > 0.0 {
        // Under an orchard's or a wood's crowns.
        out.occlusion *= mix(1.0, 0.75, country.crowns * s.b.x * past);
    }
    return out;
}

// Where the land is drawn more than this far below the sea (m), it is the seabed alone, seen through the water above it
// (`seabed_surface`); the relief the land's cover gives the sea floor above it fades out from `SEABED_RELIEF_M` down to
// it, so the two meet with no seam along the depth line.
const SEABED_PLAIN_M: f32 = -3.0;
const SEABED_RELIEF_M: f32 = -2.0;
// The seabed's roughness and grazing reflectance before the wet band's, as the styles' shore and the photographs' water
// classes give the sea floor (shingle and sand).
const SEABED_ROUGHNESS: f32 = 0.85;
const SEABED_SPECULAR: f32 = 0.5;

// The seabed deeper than `SEABED_PLAIN_M`, as `terrain_surface` paints it there: the style's sand silted darker with
// depth and its detail photograph's grain, rock where the floor is steep, the wet sand and rock materials close up,
// the wet band's darkening out to sea; without the land's cover, which the full surface works out beneath and then
// paints over with the seabed (its roughness and reflectance the shore's, its occlusion the open sky's alone, no
// relief).
fn seabed_surface(inputs: SurfaceInputs) -> Surface {
    let height = inputs.height;
    let footprint = inputs.footprint;
    let slope = length(inputs.gradient);
    let coast_byte = (inputs.coast - 0.5) * 2.0;
    let coast = sign(coast_byte) * (coast_byte * coast_byte) * COAST_RANGE_M;
    let shore = terrain.shore.rgb;
    let high = terrain.high.rgb;
    let sand_detail = detail_shown(inputs, 30.0, 30.0).b * 2.0;
    let steep = ramp(0.35, 0.7, slope);
    var strength = 0.0;
    if materials_bound() {
        strength = materials_strength(inputs.filtered);
    }
    let materials = ground_detail(mats_cover(mats_of(M_WET_SAND), M_ROCK, steep), inputs.chart, inputs.chart_dx,
        inputs.chart_dy, strength, flat_wall(), 0.0);
    let floor_color = mix(shore * 1.15, shore * 0.55, ramp(3.0, 25.0, -height)) * textured(sand_detail, footprint, 30.0, 1.0);
    let seabed = mix(floor_color, high * 0.5, steep) * materials.ratio;
    // The wet band's darkening, as it reaches the sea floor by the shore.
    let wet = 0.4 * one(ramp(25.0, 90.0, coast));
    var out: Surface;
    out.albedo = seabed * mix(1.0, 0.55, wet);
    out.roughness = mix(SEABED_ROUGHNESS, 0.3, wet * 0.85);
    out.specular = mix(SEABED_SPECULAR, 1.0, wet);
    let sky = clamp(inputs.sky, 0.0, 1.0);
    out.occlusion = sky * sqrt(sky);
    out.bump = 0.0;
    out.slope = materials.slope;
    out.material = materials.top;
    return out;
}

// `terrainSurface`: the land's cover for the map's style, then the seabed, the wet band and the foam line.
// `features_at`: the map features at the point (`feature_site`, taken where screen derivatives may be).
fn terrain_surface(inputs_in: SurfaceInputs, features_at: FeatureSite) -> Surface {
    var inputs = inputs_in;
    let height = inputs.height;
    let footprint = inputs.footprint;
    var g = ground_of(inputs);
    // The detail photographs' grain, each read only where its `textured` (grass at 15 m, sand at 30, snow at 60) shows.
    let grass_detail = detail_shown(inputs, 15.0, 15.0).g * 2.0;
    let sand_detail = detail_shown(inputs, 30.0, 30.0).b * 2.0;
    let snow_detail = detail_shown(inputs, 80.0, 60.0).a * 2.0;
    let shore = terrain.shore.rgb;
    let low = terrain.low.rgb;
    let high = terrain.high.rgb;
    var land: Layered;
    var occlusion = 1.0;
    // The ground materials' weights, laid as the land's cover is (`materials.wgsl`).
    var mats = mats_of(M_GRASS);
    // The real-world surface's sample first (a uniform branch: none on a battle map). Where it paints the point whole
    // (its coverage 1, a slope gentle enough for a photograph from above, no water class by the sea, where the style's
    // shore shows through), the style's own cover would be painted over entirely, and is not worked out.
    var real_at: RealSample;
    var procedural = true;
    if surface_bound() {
        real_at = real_sample(inputs);
        let gentle = photo_kept(g);
        let coast_water = real_at.s.c.x * one(ramp(100.0, 300.0, g.coast));
        procedural = real_at.weight * gentle < 1.0 || coast_water > 0.0;
    }
    // The place's rock (`cliff_rock`) and, where it is layered and stands bare and steep, its steps (`stepped`): from
    // here the land's slope is the stepped one (a cliff band steep, its bench gentle) for all the slope decides (the
    // cover, the snow, the rock faces), and the land's normal takes the steps (`out.slope`). Bare: the cover's bare
    // ground, and its grass, moss and lichen within 400-700 m of the snowline (the window's cover has the two in one
    // class), `exposed`; the steps also through shrubs and on steep grass, never through woods, fields, towns, snow and
    // ice or water, nor where a finer model's fine relief is drawn (`relief_real_share`: it has the real steps; a
    // synthetic or GLO-30's page has none, so the beds stand under it).
    var place_rock: CliffRock;
    let unstepped = inputs.gradient;
    var steps = flat_steps(height, g.slope);
    var exposed = 0.0;
    if rock_faces() && g.slope + g.jag * 0.25 > 0.2 {
        place_rock = cliff_rock(inputs.chart);
        var open = ramp(0.6, 0.9, g.slope);
        if surface_bound() {
            let s = real_at.s;
            let snowline = snowline_at(g);
            let alpine = ramp(snowline - 700.0, snowline - 400.0, height);
            exposed = (s.b.z + s.a.w * alpine * 0.7) * real_at.weight;
            let shown = exposed + (s.a.z * 0.8 + s.a.w * (1.0 - alpine * 0.7) * ramp(0.6, 0.9, g.slope) * 0.6
                + s.c.z) * real_at.weight;
            open = mix(open, shown, real_at.weight);
        }
        var weight = ramp(0.5, 0.75, place_rock.strata) * ramp(0.25, 0.45, g.slope) * clamp(open, 0.0, 1.0);
        if weight > 0.0 {
            weight *= 1.0 - relief_real_share(inputs.chart);
            let level = strata_level(height, g, place_rock.bed_m, strata_dip(inputs));
            let level_px = length(vec2<f32>(dot(unstepped, inputs.chart_dx), dot(unstepped, inputs.chart_dy)));
            steps = stepped(level, level_px, g.slope, g.grain, weight);
            inputs.gradient = unstepped * steps.k;
            g.slope *= steps.k;
        } else if place_rock.strata > 0.0 {
            steps.level = strata_level(height, g, place_rock.bed_m, strata_dip(inputs));
        }
    }
    if procedural {
#ifdef STYLE_SNOW
        // Vestfjord in April 1940: snow down to a ragged line, dark faces between white couloirs, heath and rock below
        // with snow lying in patches, birch scrub, dark skerries, and the tidal band.
        let m = snow_masks(inputs, g);
        let stone = rock(inputs, high, srgb(109.0, 112.0, 110.0));
        // Winter heath, brown and grey; thawed, grass and moss greening up the slopes toward the snow, browner near it.
        let thawed = terrain.climate.y * one(ramp(terrain.climate.x * 0.45, terrain.climate.x * 0.9, height));
        let grassland = mix(srgb(82.0, 88.0, 56.0), srgb(104.0, 104.0, 68.0), g.grain * 0.7 + g.knoll * 0.3) * mix(1.0, 0.85, g.hollow);
        let heath = mix(mix(srgb(56.0, 53.0, 47.0), srgb(68.0, 67.0, 63.0), g.grain), grassland, thawed)
            * textured(grass_detail, footprint, 15.0, 1.0);
        let ground = mix(heath, stone.color, min(ramp(0.25, 0.6, g.slope) * 0.7 + m.shore * 0.6, 1.0));
        let snow = srgb(232.0, 237.0, 241.0) * textured(snow_detail, footprint, 60.0, 0.45) * mix(0.95, 1.02, g.grain);
        // Birch: leafless, twigs and trunks over whatever lies beneath, snow or heath; in leaf, a low green canopy.
        let trees = canopy(inputs, 5.0, 4.0);
        let bare_wood = mix(mix(ground, snow, m.snow), srgb(59.0, 51.0, 46.0) * trees.tint, min(trees.shade * 0.35 + 0.3, 0.8));
        let wood = mix(bare_wood, srgb(62.0, 76.0, 44.0) * trees.shade * trees.tint, terrain.climate.z);
        land = base_cover(ground, 0.95, 0.3, stone.height * ramp(0.25, 0.6, g.slope));
        land = cover(land, snow, 0.75, 0.45, 0.0, m.snow);
        land = cover(land, wood, 0.95, 0.25, trees.height, m.woods);
        land = cover(land, stone.color, 0.85, 0.5, stone.height, m.bare);
        land = cover(land, shore * textured(detail_shown(inputs, 40.0, 40.0).r * 2.0, footprint, 40.0, 1.0), 0.5, 0.8, 0.0, m.tidal);
        occlusion = mix(1.0, min(trees.shade, 1.0), m.woods * 0.4);
        // Heath on scrub, grass where it has thawed, rock on the slopes and the shore, snow, birch's leaf litter in leaf
        // (bare, the floor is the snow or heath beneath), bare faces, the tidal band's stones.
        mats = mats_cover(mats_of(M_SCRUB), M_GRASS, thawed);
        mats = mats_cover(mats, M_ROCK, min(ramp(0.25, 0.6, g.slope) * 0.7 + m.shore * 0.6, 1.0));
        mats = mats_cover(mats, M_SNOW, m.snow);
        mats = mats_cover(mats, M_FOREST, m.woods * terrain.climate.z);
        mats = mats_cover(mats, M_CLIFF, m.bare);
        mats = mats_cover(mats, M_SHINGLE, m.tidal);
#endif
#ifdef STYLE_CHALK
        // Kent and the Pas-de-Calais in February: chalk cliffs, downland, winter farmland between hedgerows, bare woods,
        // shingle under the cliffs and sand across the flat bays.
        let m = chalk_masks(inputs, g);
#ifdef DETAILED
        let streaks = fall_streaks(inputs, 6.0, 60.0);
#else
        let streaks = 0.5;
#endif
        let chalk_color = high * mix(0.78, 1.05, streaks) * (textured(detail_shown(inputs, 50.0, 30.0).r, footprint, 30.0, 0.5) * 0.3 + 0.85);
        let chalk_height = streaks * 2.5 * resolved(footprint, 6.0);
        // The season and climate: February's as authored; the leaves out, the grass and crops grown green, the woods in leaf;
        // a dry summer, the downs straw and the fields stubble, ploughed red earth and olive groves, stone walls for
        // hedgerows, the woods dark olive maquis.
        let leaf = terrain.climate.z;
        let dry = terrain.climate.w;
        // A dry country's open ground: tawny grass and stubble, grey-green garrigue over the rougher, steeper ground.
        let garrigue = ramp(0.22, 0.48, g.patches * 0.6 + g.slope * 1.5 + g.jag * 0.5);
        let dry_ground = mix(srgb(92.0, 82.0, 58.0), srgb(62.0, 66.0, 48.0), garrigue);
        let downs = mix(mix(low, low * vec3<f32>(0.8, 0.94, 0.74), leaf), dry_ground, dry)
            * textured(grass_detail, footprint, 15.0, 1.0) * mix(0.88, 1.08, g.grain);
        let farm = crops(inputs, m.fields, array<vec3<f32>, 6>(
            // Ploughed and drilled: brown earth through the year, the dry country's red.
            mix(mix(srgb(102.0, 88.0, 74.0), srgb(89.0, 79.0, 62.0), leaf), srgb(99.0, 74.0, 55.0), dry),
            // Pasture and young wheat, deep green in May; vines and vegetable plots.
            mix(mix(srgb(86.0, 96.0, 63.0), srgb(60.0, 77.0, 40.0), leaf), srgb(74.0, 75.0, 52.0), dry),
            mix(mix(srgb(98.0, 107.0, 72.0), srgb(72.0, 88.0, 45.0), leaf), srgb(106.0, 94.0, 66.0), dry),
            // Rape in flower, one field in six; ripe barley.
            mix(mix(srgb(119.0, 112.0, 90.0), srgb(116.0, 112.0, 58.0), leaf), srgb(112.0, 100.0, 70.0), dry),
            mix(mix(srgb(90.0, 100.0, 70.0), srgb(67.0, 81.0, 43.0), leaf), srgb(92.0, 84.0, 61.0), dry),
            // Hay meadow, paler; olive groves over tilled earth.
            mix(mix(srgb(103.0, 93.0, 74.0), srgb(86.0, 93.0, 55.0), leaf), srgb(86.0, 81.0, 58.0), dry)), 6);
        let hedgerow = mix(mix(srgb(52.0, 54.0, 44.0), srgb(36.0, 46.0, 29.0), leaf), srgb(115.0, 108.0, 90.0), dry);
        let trees = canopy(inputs, 9.0, 6.0);
        let wood = mix(mix(srgb(74.0, 67.0, 61.0), srgb(41.0, 53.0, 31.0), leaf), srgb(56.0, 59.0, 41.0), dry) * trees.shade * trees.tint;
        let bay_flat = one(ramp(0.025, 0.07, g.slope));
        let sand = srgb(166.0, 151.0, 123.0) * textured(sand_detail, footprint, 30.0, 1.0);
        let shingle = shore * textured(detail_shown(inputs, 12.0, 12.0).r * 2.0, footprint, 12.0, 1.0);
        let hedges = m.fields.boundary * m.fields.edge_near;
        land = base_cover(downs, 0.92, 0.35, 0.0);
        land = cover(land, mix(farm.color, hedgerow, hedges), 0.95, 0.35, hedges * 3.0 + farm.furrows * 0.15, m.gentle);
        // Beyond the distance where hedgerows resolve, their share of the land still darkens it.
        land = cover(land, mix(hedgerow, wood, 0.6), 0.95, 0.25, 0.0, m.gentle * one(m.fields.edge_near) * 0.1);
        land = cover(land, wood, 0.95, 0.25, trees.height, m.woods);
        land = cover(land, chalk_color, 0.8, 0.5, chalk_height * 2.0, m.cliff);
        land = cover(land, mix(shingle, sand, bay_flat), 0.85, 0.5, 0.0, g.beach);
        // Grey shingle and fallen chalk at the foot of every cliff.
        land = cover(land, shingle * 1.1, 0.85, 0.5, 0.0, max(m.foot, m.cliff * one(ramp(4.0, 9.0, height + g.jag * 3.0))));
        occlusion = mix(1.0, min(trees.shade, 1.0), m.woods * 0.4);
        // Grass on the downs (dry in a dry summer, garrigue's stony ground), ploughed earth in the ploughed fields, scrub
        // along the hedgerows, leaf litter in the woods, chalk on the cliffs, shingle and the bays' sand.
        mats = mats_cover(mats_of(M_GRASS), M_DRY_GRASS, dry);
        mats = mats_cover(mats, M_DRY_SCRUB, dry * garrigue);
        let ploughed = one(ramp(1.0 / 6.0 - 0.005, 1.0 / 6.0 + 0.005, m.fields.id)) * m.fields.near;
        mats = mats_cover(mats, M_SOIL, m.gentle * ploughed);
        mats = mats_cover(mats, M_SCRUB, m.gentle * hedges);
        mats = mats_cover(mats, M_FOREST, m.woods);
        mats = mats_cover(mats, M_CLIFF, m.cliff);
        mats = mats_cover(mats, M_SHINGLE, g.beach);
        mats = mats_cover(mats, M_SAND, g.beach * bay_flat);
        mats = mats_cover(mats, M_SHINGLE, max(m.foot, m.cliff * one(ramp(4.0, 9.0, height + g.jag * 3.0))));
#endif
#ifdef STYLE_VOLCANIC
        // West Java in February: black volcanic sand, coconut palms along the shore, paddies across the plains with
        // village groves among them, forest and plantation over the hills, rock only on the steepest slopes.
        let m = volcanic_masks(inputs, g);
        let trees = canopy(inputs, 13.0, 4.0);
        let forest = low * trees.shade * trees.tint * mix(0.94, 1.08, g.knoll) * (g.jag * 0.12 + 1.0)
            * mix(1.06, 0.9, ramp(-8.0, 8.0, -g.fine));
        let paddy = crops(inputs, m.plots, array<vec3<f32>, 6>(srgb(90.0, 111.0, 69.0), srgb(85.0, 117.0, 54.0), srgb(98.0, 130.0, 58.0),
            srgb(77.0, 109.0, 45.0), srgb(111.0, 138.0, 68.0), vec3<f32>(0.0)), 5);
        // Palm crowns are smaller and paler than the forest's; the forest's crown pattern, finer, stands in for them.
        let palm_shade = trees.shade * 0.85 + 0.15;
        let palm_height = trees.height * 0.6;
        let palm_color = srgb(79.0, 102.0, 54.0) * palm_shade * trees.tint;
        let stone = rock(inputs, high, srgb(107.0, 99.0, 86.0));
        land = base_cover(forest, 0.95, 0.25, trees.height);
        land = cover(land, mix(paddy.color, srgb(72.0, 83.0, 58.0), m.plots.boundary * m.plots.edge_near * 0.8), 0.75, 0.55, 0.0, m.plain);
        land = cover(land, forest * 0.92, 0.95, 0.25, trees.height, m.plain * m.grove);
        land = cover(land, palm_color, 0.95, 0.25, palm_height, m.palms);
        land = cover(land, stone.color, 0.85, 0.5, stone.height, m.steep);
        // Black volcanic sand, glinting a little where the swash has left it wet.
        land = cover(land, shore * 0.8 * textured(sand_detail, footprint, 30.0, 1.0), 0.7, 0.6, 0.0, g.beach);
        occlusion = mix(min(trees.shade, 1.0) * 0.3 + 0.7, 1.0, max(m.plain * one(m.grove), g.beach));
        // Leaf litter where the forest's and the groves' trees stand over it (`volcanic_stand`, its edge as soft as
        // theirs), grass on the open ground about them and under the palms, the paddies' mud, rock on the steepest
        // slopes, the black sand.
        mats = mats_of(M_GRASS);
        mats = mats_cover(mats, M_MUD, m.plain);
        mats = mats_cover(mats, M_FOREST, under_broadleaves(volcanic_stand(inputs, g, m)));
        mats = mats_cover(mats, M_ROCK, m.steep);
        mats = mats_cover(mats, M_FINE_SAND, g.beach);
#endif
#ifdef STYLE_TROPICAL
        // Guadalcanal and the Florida Islands in 1942: rainforest, kunai on the plains and ridges, gallery forest down
        // every gully, coconut plantations in rows, dark grey sand, landslide scars on the steepest faces.
        let m = tropical_masks(inputs, g);
        let trees = canopy(inputs, 15.0, 5.0);
        let forest = mix(low, low * vec3<f32>(0.82, 0.92, 1.04), m.montane) * trees.shade * trees.tint
            * mix(0.94, 1.08, g.knoll) * mix(1.03, 0.86, m.gully) * (g.jag * 0.12 + 1.0);
        // Kunai in the wet tropics is green, a few shades lighter than the canopy, olive only where it dries on a knoll.
        let grass = mix(srgb(78.0, 100.0, 50.0), srgb(98.0, 112.0, 58.0), g.knoll * 0.6 + g.grain * 0.4)
            * textured(grass_detail, footprint, 15.0, 1.0) * (g.jag * 0.12 + 1.0);
        let row_phase = inputs.chart / 9.0 * (2.0 * 3.141592653589793);
        let rows = (sin(row_phase.x) * sin(row_phase.y) * 0.5 + 0.5) * resolved(footprint, 9.0);
        let palm_color = mix(srgb(86.0, 102.0, 58.0), srgb(106.0, 112.0, 66.0), g.grain) * mix(1.0, rows * 0.5 + 0.75, resolved(footprint, 9.0));
        let stone = rock(inputs, high, srgb(122.0, 94.0, 70.0));
        land = base_cover(forest, 0.95, 0.25, trees.height);
        land = cover(land, grass, 0.95, 0.35, 0.0, m.kunai);
        land = cover(land, palm_color, 0.95, 0.3, rows * 4.0, m.plantation);
        land = cover(land, stone.color, 0.9, 0.45, stone.height, m.scar);
        land = cover(land, shore * textured(sand_detail, footprint, 30.0, 1.0), 0.85, 0.5, 0.0, g.beach);
        occlusion = mix(min(trees.shade, 1.0) * 0.3 + 0.7, 1.0, max(m.kunai, g.beach));
        // Leaf litter where the rainforest's trees stand over it (`tropical_stand`, its edge as soft as theirs), grass on
        // the open ground about it, under the palms along the shore, on the kunai and between the plantations' rows; rock
        // on the scars, sand.
        mats = mats_of(M_GRASS);
        mats = mats_cover(mats, M_FOREST, under_broadleaves(tropical_stand(inputs, g, m)));
        mats = mats_cover(mats, M_ROCK, m.scar);
        mats = mats_cover(mats, M_FINE_SAND, g.beach);
#endif
    } else {
        // Never shown: the photograph paints the point whole.
        land = base_cover(low, 0.95, 0.3, 0.0);
    }

    // The real-world surface where a theatre's window has maps or near tiles.
    // Where the window paints the land, the shore's materials only where its photograph looks like a beach.
    var shore_gate = 1.0;
    var rock_photo = vec3<f32>(0.0);
    var rock_share = 0.0;
    var snow_share = 0.0;
    var real_weight = 0.0;
    var photo_weight = 0.0;
    var built = 0.0;
    var photo_detail = 1.0;
    if surface_bound() {
        let real = real_surface(inputs, g, real_at, land, mats, vec3<f32>(grass_detail, sand_detail, snow_detail));
        // A photograph from above sees little of a cliff or a steep face: the place's rock there (`rock_face`, below),
        // or with no place the style's rock and chalk.
        let weight = real.weight * photo_kept(g);
        land = cover(land, real.land.color, real.land.roughness, real.land.specular, real.land.bump, weight);
        occlusion = mix(occlusion, real.occlusion, weight);
        mats = real.mats;
        shore_gate = mix(1.0, real.beach, weight);
        real_weight = weight;
        photo_weight = weight * real.photo;
        built = real.weight * real.built;
        photo_detail = mix(1.0, real.detail, real.weight);
        rock_photo = real.rock;
        rock_share = real.rock_share * photo_kept(g);
        snow_share = real.snow * weight;
    }
    // A theatre's rock faces, over the photograph and the style: in the place's rock, whose colour is the photograph's
    // calibration's (not the styles' tone).
    var face = 0.0;
    if rock_faces() {
        var growth = vec2<f32>(0.0);
        if surface_bound() {
            let s = real_at.s;
            growth = vec2<f32>(s.a.x + s.a.y + s.a.z * 0.8, s.a.w + s.b.x + s.c.y) * real_at.weight;
        }
        let rock = rock_face(inputs, g, growth, place_rock, steps, exposed * (1.0 - snow_share));
        face = rock.weight;
        if face > 0.0 {
            let color = mix(rock.color, rock_photo * rock.tone, rock.photo * rock_share);
            land = cover(land, color, 0.85, 0.5, rock.bump, face);
            photo_weight = mix(photo_weight, 1.0, face);
            shore_gate = mix(shore_gate, 0.0, face);
        }
    }
    // Under standing trees, seen from beneath their crowns, the cover's leaf litter is the forest floor: litter's brown
    // in place of the canopy painted for a view from above (its crowns' shading, tint and relief, which from a man's
    // height read as hard-edged patches of light and dark ground), so rough and dull that the sky the crowns leave it
    // shows only faintly (`forest.wgsl`'s `forest_floor` takes most of it away). What the floor is (`floor_at`: leaves,
    // needles, dust) only where it shows: as the litter, or as the forest material's read while a pixel is smaller than
    // its leaves' 2 m tile.
    var strength = 0.0;
    if materials_bound() {
        strength = materials_strength(inputs.filtered);
    }
    let forest_share = mats_weight(mats, M_FOREST);
    var floor = Floor(0.0, terrain.climate.w, 0.0);
    if forest_share > 0.0 && (inputs.floor > 0.0 || (strength > 0.0 && inputs.footprint < 2.0)) {
        floor = floor_at(inputs, g);
    }
    var litter = 0.0;
    if inputs.floor > 0.0 {
        litter = forest_share * inputs.floor * one(face);
        land = cover(land, litter_color(g, floor), 0.95, 0.12, 0.0, litter);
        occlusion = mix(occlusion, 1.0, litter);
    }
    let underwater = one(ramp(-1.2, 0.2, height));
    // The wet band the waves wash: darker, glossy and a full mirror at grazing angles, wandering with the swash.
    // Only by the sea itself: a low plain a metre up a kilometre inland is not awash.
    let by_sea = one(ramp(25.0, 90.0, g.coast));
    let wet = one(ramp(0.25, 1.5, height + (g.grain - 0.5) * 0.7 + (g.speck - 0.5) * 0.3)) * one(underwater * 0.6) * by_sea;

    // The ground materials past the cover (`materials.wgsl`): faces steeper than about 40 degrees are cliff rock; the
    // shore band (the land within 30-80 m of the sea and a few metres above it) sand on the flat, shingle steeper and
    // rock steeper still, by the style (the chalk and snow coasts' shingle, the tropics' sand); the swash's sand wet;
    // the seabed sand, rock where it is steep. Structure only: the colours stay the cover's.
    mats = mats_cover(mats, M_CLIFF, select(ramp(0.75, 1.1, g.slope + g.jag * 0.15), face, rock_faces()));
    let shore_band = one(ramp(30.0, 80.0, g.coast + g.jag * 40.0)) * one(ramp(3.0, 6.0, height + g.jag * 2.0)) * shore_gate;
#ifdef STYLE_CHALK
    let sandy = one(ramp(0.03, 0.08, g.slope + g.jag * 0.05));
    let rocky = ramp(0.25, 0.45, g.slope + g.jag * 0.1);
#else
#ifdef STYLE_SNOW
    let sandy = one(ramp(0.03, 0.08, g.slope + g.jag * 0.05));
    let rocky = ramp(0.25, 0.45, g.slope + g.jag * 0.1);
#else
    let sandy = one(ramp(0.14, 0.26, g.slope + g.jag * 0.05));
    let rocky = ramp(0.32, 0.55, g.slope + g.jag * 0.1);
#endif
#endif
    // The tropical and volcanic coasts' beaches are fine sand to the swash, the others' the coarser sand of the aerial
    // photograph.
#ifdef STYLE_TROPICAL
    let shore_sand = M_FINE_SAND;
#else
#ifdef STYLE_VOLCANIC
    let shore_sand = M_FINE_SAND;
#else
    let shore_sand = M_SAND;
#endif
#endif
    // Where a photograph paints the land, its beach is sand on the gentle slopes whatever the style (it showed sand to
    // be let in): shingle on the steeper ones.
    let shore_sandy = mix(sandy, one(ramp(0.14, 0.26, g.slope + g.jag * 0.05)), real_weight);
    mats = mats_cover(mats, M_SHINGLE, shore_band);
    mats = mats_cover(mats, shore_sand, shore_band * shore_sandy);
    mats = mats_cover(mats, M_ROCK, shore_band * rocky);
    mats = mats_wet(mats, wet);
    mats = mats_cover(mats, M_WET_SAND, underwater * one(ramp(0.35, 0.7, g.slope)));
    mats = mats_cover(mats, M_ROCK, underwater * ramp(0.35, 0.7, g.slope));
    // The cliff's rock, read face on where the land stands steep (only where the cliff is chosen and the materials show).
    var wall = flat_wall();
    if mats.b.w > 0.0 && strength > 0.0 {
        let rock = cliff_rock(inputs.chart);
        wall.normal = normalize(vec3<f32>(-inputs.gradient.x, 1.0, -inputs.gradient.y));
        wall.height = height;
        wall.height_dx = dot(inputs.gradient, inputs.chart_dx);
        wall.height_dy = dot(inputs.gradient, inputs.chart_dy);
        wall.layer = rock.layer;
        wall.params = rock.params;
    }
    let materials = ground_detail(mats, inputs.chart, inputs.chart_dx, inputs.chart_dy, strength, wall, floor.needles);
    land.color *= materials.ratio;

    // Roads, railways, rivers and canals over it all (`features.wgsl`; none with no map features bound).
    let features = feature_cover(features_at, height, land.color, land.roughness, land.specular);
    land = Layered(features.color, features.roughness, features.specular, land.bump * (1.0 - features.weight));
    // The styles and the features take the land's tone; the photograph keeps its own colour (`calibrated`), and so does
    // the forest floor's litter (`litter_color`).
    land.color = mix(tone_down(land.color), land.color, max(photo_weight, litter) * (1.0 - features.weight));

    // Under water: the style's sand, silted darker with depth, rock where the floor is steep.
    let floor_color = mix(shore * 1.15, shore * 0.55, ramp(3.0, 25.0, -height)) * textured(sand_detail, footprint, 30.0, 1.0);
    let seabed = mix(floor_color, high * 0.5, ramp(0.35, 0.7, g.slope)) * materials.ratio;
    var color = mix(land.color, seabed, underwater);
    color *= mix(1.0, 0.55, wet);
    // The foam line: broken white water where the swell breaks on the shore, a few metres wide.
    let reach = 0.5 + one(ramp(0.05, 0.3, g.slope)) * 0.8;
    let band = ramp(-0.5, -0.1, height) * one(ramp(reach * 0.4, reach, height + g.jag * 0.4)) * by_sea;
    // Its broken water read only on the band, and where a pixel can hold it.
    let froth_near = resolved(footprint, 8.0);
    var froth = 0.45;
#ifdef DETAILED
    if band > 0.0 && froth_near > 0.0 {
        froth = ramp(0.4, 0.7, noise_r(inputs, 23.0) + (g.speck - 0.5) * 0.6);
    }
#endif
    let foam = band * mix(0.45, froth, froth_near) * 0.85;
    color = mix(color, vec3<f32>(0.72, 0.74, 0.74), foam);
    var out: Surface;
    out.albedo = color;
    out.roughness = mix(mix(land.roughness, 0.3, wet * 0.85), 0.7, foam);
    out.specular = mix(mix(land.specular, 1.0, wet), 0.4, foam);
    // Open sky: valleys and the feet of slopes receive less of the sky's light.
    let sky = clamp(inputs.sky, 0.0, 1.0);
    out.occlusion = occlusion * (sky * sqrt(sky));
    out.bump = land.bump;
    // The materials' tilt, the cliff's face-on relief turned onto the land's normal (`wall_slope`) where it has any.
    var slope = materials.slope;
    if any(materials.relief != vec3<f32>(0.0)) {
        slope = wall_slope(inputs.gradient, materials.slope, materials.relief);
    }
    out.slope = (slope + unstepped * (steps.k - 1.0)) * (1.0 - features.weight) * (1.0 - foam);
    out.material = materials.top;
    out.built = built;
    out.photo_detail = photo_detail;
    return out;
}

// ---- Trees (`treeCover`) ----

const TREE_BROADLEAF: f32 = 0.0;
const TREE_PALM: f32 = 1.0;
const TREE_CONIFER: f32 = 2.0;
const TREE_BARE: f32 = 3.0;

struct TreeCover {
    // Chance a tree stands on this spot, 0–1.
    density: f32,
    // Its kind (`TREE_*`).
    kind: f32,
    // Height in metres at the middle of its range.
    height: f32,
    // Albedo tint multiplying the impostor's colours.
    tint: vec3<f32>,
}

// Which tree stands at a point, from the same land-use masks the ground is painted with; `pick` is the tree's own
// random value. Sample with `LOD_ZERO` and a zero footprint.
fn tree_cover(inputs: SurfaceInputs, pick: f32) -> TreeCover {
    let height = inputs.height;
    let g = ground_of(inputs);
    // No tree on the beach, in the sea or on bare rock and cliffs.
    let land = ramp(2.5, 5.0, height) * one(g.beach) * one(ramp(0.45, 0.6, g.slope));
    var t: TreeCover;
#ifdef STYLE_SNOW
    let m = snow_masks(inputs, g);
    let conifer = pick < g.hollow * 0.3 + 0.15;
    // Birch in leaf with the season, its own share of them first.
    let in_leaf = fract(pick * 7.13) < terrain.climate.z;
    t.density = m.woods * one(m.bare) * land * 0.8;
    t.kind = select(select(TREE_BARE, TREE_BROADLEAF, in_leaf), TREE_CONIFER, conifer);
    t.height = select(11.0, 15.0, conifer);
    t.tint = select(select(vec3<f32>(1.15, 1.1, 1.05), vec3<f32>(1.0, 1.08, 0.9), in_leaf), vec3<f32>(0.95, 1.0, 1.0), conifer);
#endif
#ifdef STYLE_CHALK
    let m = chalk_masks(inputs, g);
    let conifer = pick < 0.12;
    let dry = terrain.climate.w;
    // Oak and ash in leaf with the season; in a dry country low olive and holm oak, stone walls in place of hedgerows.
    let in_leaf = fract(pick * 7.13) < max(terrain.climate.z, dry);
    // Olive groves: in a dry country, rows of low trees over one field in six.
    let orchard = dry * ramp(0.835, 0.845, m.fields.id) * m.fields.near * m.gentle;
    t.density = max(max(m.woods * mix(0.95, 0.7, dry), m.hedges * 0.35 * one(dry * 0.85)), orchard * 0.5) * one(m.cliff) * land;
    t.kind = select(select(TREE_BARE, TREE_BROADLEAF, in_leaf), TREE_CONIFER, conifer);
    t.height = select(mix(19.0, 8.0, dry), 18.0, conifer);
    t.tint = select(select(vec3<f32>(1.25, 1.2, 1.15), mix(vec3<f32>(0.92, 1.0, 0.9), vec3<f32>(0.95, 0.95, 0.8), dry), in_leaf),
        vec3<f32>(0.85, 0.9, 0.88), conifer);
#endif
#ifdef STYLE_VOLCANIC
    let m = volcanic_masks(inputs, g);
    let stand = volcanic_stand(inputs, g, m);
    let palm = pick < stand.y;
    t.density = stand.x;
    t.kind = select(TREE_BROADLEAF, TREE_PALM, palm);
    t.height = select(24.0, 20.0, palm);
    t.tint = select(vec3<f32>(0.95, 0.97, 0.95), vec3<f32>(0.92, 0.95, 0.9), palm);
#endif
#ifdef STYLE_TROPICAL
    let m = tropical_masks(inputs, g);
    let stand = tropical_stand(inputs, g, m);
    let palm = pick < stand.y;
    t.density = stand.x;
    t.kind = select(TREE_BROADLEAF, TREE_PALM, palm);
    t.height = select(mix(30.0, 20.0, m.montane), 22.0, palm);
    t.tint = select(vec3<f32>(0.95, 1.0, 0.98), vec3<f32>(0.95, 0.95, 0.88), palm);
#endif
    // Where surface maps cover the point, the land cover at the tree's foot says whether one stands there: woods and
    // mangroves dense, shrubland sparse, a few in fields, towns and wetland, none on grass, rock, snow or water; an
    // unknown class keeps the style's own masks. The kind and tint stay the style's (its palms, its season). The near
    // tiles' finer cover where they are drawn.
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
        var mapped = dot(s.a, vec4<f32>(0.92, 0.85, 0.22, 0.02)) + dot(s.b, vec4<f32>(0.012, 0.015, 0.0, 0.0))
            + s.c.y * 0.08 + s.c.z * t.density;
        if s.past > 0.0 {
            // The period's country (counted in the cropland's share): its woods and groves dense, its orchards' and
            // plantations' rows half closed, its fields' trees few (`tree_place.wgsl`'s `country_slot`).
            let kind = country_field(inputs).kind;
            var trees = 0.03;
            if kind == FIELD_WOOD {
                trees = 0.85;
            } else if kind == FIELD_ORCHARD || kind == FIELD_OLIVES || kind == FIELD_PALMS {
                trees = 0.45;
            }
            mapped += s.past * (trees - 0.012);
        }
        let standing = ramp(1.5, 3.0, height) * one(ramp(0.75, 1.0, g.slope));
        t.density = mix(t.density, mapped * standing, weight);
        // Shrubs, mangroves and the trees of fields and towns stand lower than a wood's.
        let scale = dot(s.a, vec4<f32>(1.0, 0.45, 0.45, 0.6)) + dot(s.b, vec4<f32>(0.7, 0.7, 0.6, 0.6))
            + dot(s.c, vec4<f32>(0.6, 0.6, 1.0, 0.0));
        t.height *= mix(1.0, scale, weight);
    }
    // No tree on a road, a railway or a river (`features.wgsl`).
    t.density *= 1.0 - feature_clearance(inputs.chart);
    return t;
}
