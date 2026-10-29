// The land's tiling ground materials (`naval_terrain::ground_materials`; native/docs/terrain/ground.md, "Ground
// materials"):
// grass, dry grass, soil, scrub, dry scrub, forest floor, rock, cliff, sand, wet sand, shingle, snow, mud and paving,
// photographs tiled under the land's own colour, and the cliffs' rock by region (limestone, basalt, granite). New to the
// native client.
//
// Each material is two layers of `ground_materials`: its colour as a ratio to its own local mean (halved, linear, so
// 0.5 is the mean) with its height, and its normal (OpenGL's: green toward the image's top). The land gives every
// material a weight where it chooses its colour (`surface.wgsl`: the style's masks, the real-world cover, slope, the
// shore, snow); the two heaviest are read, hex-tiled as the detail photographs are, blended by their heights (the higher
// texel shows through, so sand fills between pebbles rather than fading into them), and the land's colour is multiplied
// by the blend's ratio, its normal tilted by the blend's: structure and relief, never colour, so the satellite
// photograph and the styles keep the land's colours and their calibration. Both fade out as a pixel grows past
// `terrain.materials.z` to `w` metres, so nothing changes from about 1.5 km at the game's lens.
//
// Every material is read on the chart, as from above, but the cliff's: its weight reads the rock of the land's place
// (`cliff_rock`: limestone, basalt, granite or the cliff's own), and on a steep face it is read face on (`cliff_read`:
// biplanar), so a wall's joints and beds stand up it instead of streaking down it.
#define_import_path naval_terrain::materials

#ifdef MATERIALS
#import naval_terrain::bindings::{terrain, repeat_sampler, ground_materials}
#import naval_terrain::hex::hex_taps_fast
#else
#import naval_terrain::bindings::terrain
#endif

// Material ids: the order of `ground_materials::GROUND_MATERIALS` (a test holds them to it). `M_NONE` reads nothing
// (inland water, for one). The cliffs' regional rocks follow in the array, read only through `M_CLIFF`.
const M_GRASS: u32 = 0u;
const M_DRY_GRASS: u32 = 1u;
const M_SOIL: u32 = 2u;
const M_SCRUB: u32 = 3u;
const M_DRY_SCRUB: u32 = 4u;
const M_FOREST: u32 = 5u;
const M_ROCK: u32 = 6u;
const M_CLIFF: u32 = 7u;
const M_SAND: u32 = 8u;
const M_WET_SAND: u32 = 9u;
const M_SHINGLE: u32 = 10u;
const M_SNOW: u32 = 11u;
const M_MUD: u32 = 12u;
const M_PAVING: u32 = 13u;
// Fine beach sand photographed close up: the tropical and volcanic coasts' beaches, fine-grained to the swash.
const M_FINE_SAND: u32 = 14u;
const M_NONE: u32 = 15u;

// The second material's least share of the pair to be read (`ground_detail`).
const MATERIAL_SECOND_MIN: f32 = 0.1;
// The height-based blend: the heights' share against the weights', and the depth over which the lower one fades.
const MATERIAL_HEIGHT: f32 = 0.5;
const MATERIAL_DEPTH: f32 = 0.25;

// A weight for each material, 16 slots (`M_*`).
struct Mats {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    d: vec4<f32>,
}

fn mats_of(id: u32) -> Mats {
    let k = vec4<u32>(0u, 1u, 2u, 3u);
    let i = vec4<u32>(id);
    let one = vec4<f32>(1.0);
    let zero = vec4<f32>(0.0);
    return Mats(select(zero, one, k == i), select(zero, one, k + 4u == i), select(zero, one, k + 8u == i),
        select(zero, one, k + 12u == i));
}

fn mats_mix(a: Mats, b: Mats, t: f32) -> Mats {
    return Mats(mix(a.a, b.a, t), mix(a.b, b.b, t), mix(a.c, b.c, t), mix(a.d, b.d, t));
}

// `m` covered by material `id` by `weight`, as `surface.wgsl`'s `cover` lays a layer over the land.
fn mats_cover(m: Mats, id: u32, weight: f32) -> Mats {
    return mats_mix(m, mats_of(id), clamp(weight, 0.0, 1.0));
}

fn mats_add(m: Mats, id: u32, weight: f32) -> Mats {
    let h = mats_of(id);
    return Mats(m.a + h.a * weight, m.b + h.b * weight, m.c + h.c * weight, m.d + h.d * weight);
}

// Every weight of `m` times `s`.
fn mats_scaled(m: Mats, s: f32) -> Mats {
    return Mats(m.a * s, m.b * s, m.c * s, m.d * s);
}

// `m` with every weight of `p` added by `weight`.
fn mats_add_all(m: Mats, p: Mats, weight: f32) -> Mats {
    return Mats(m.a + p.a * weight, m.b + p.b * weight, m.c + p.c * weight, m.d + p.d * weight);
}

fn mats_none() -> Mats {
    return Mats(vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0), vec4<f32>(0.0));
}

// `m` with `wet` of its sand (either) turned to wet sand (the swash band).
fn mats_wet(m: Mats, wet: f32) -> Mats {
    var out = m;
    // M_SAND is c.x, M_WET_SAND c.y, M_FINE_SAND d.z.
    let w = clamp(wet, 0.0, 1.0);
    out.c.x = m.c.x * (1.0 - w);
    out.d.z = m.d.z * (1.0 - w);
    out.c.y = m.c.y + (m.c.x + m.d.z) * w;
    return out;
}

// Material `id`'s weight in `m`.
fn mats_weight(m: Mats, id: u32) -> f32 {
    let h = mats_of(id);
    return dot(m.a, h.a) + dot(m.b, h.b) + dot(m.c, h.c) + dot(m.d, h.d);
}

// The heaviest weight in `m`.
fn mats_max(m: Mats) -> f32 {
    let v = max(max(m.a, m.b), max(m.c, m.d));
    return max(max(v.x, v.y), max(v.z, v.w));
}

// The first slot of `m` holding `w`.
fn mats_slot(m: Mats, w: f32) -> f32 {
    let k = vec4<f32>(0.0, 1.0, 2.0, 3.0);
    let none = vec4<f32>(99.0);
    let x = vec4<f32>(w);
    let v = min(min(select(none, k, m.a == x), select(none, k + 4.0, m.b == x)),
        min(select(none, k + 8.0, m.c == x), select(none, k + 12.0, m.d == x)));
    return min(min(v.x, v.y), min(v.z, v.w));
}

// `m` with slot `slot` emptied (below any weight).
fn mats_drop(m: Mats, slot: f32) -> Mats {
    let k = vec4<f32>(0.0, 1.0, 2.0, 3.0);
    let s = vec4<f32>(slot);
    let gone = vec4<f32>(-1.0);
    return Mats(select(m.a, gone, k == s), select(m.b, gone, k + 4.0 == s), select(m.c, gone, k + 8.0 == s),
        select(m.d, gone, k + 12.0 == s));
}

// The two heaviest materials and their weights (x, y the first's id and weight, z, w the second's), the third's weight
// taken from both and the pair renormalised: where the second and third trade places neither shows, so no seam.
// Branchless: three rounds of a maximum over the slots.
fn mats_top(m: Mats) -> vec4<f32> {
    let w1 = mats_max(m);
    let s1 = mats_slot(m, w1);
    let rest = mats_drop(m, s1);
    let w2 = mats_max(rest);
    let s2 = mats_slot(rest, w2);
    let third = max(mats_max(mats_drop(rest, s2)), 0.0);
    let first = max(w1 - third, 0.0);
    let second = max(w2 - third, 0.0);
    let total = max(first + second, 1e-5);
    return vec4<f32>(s1, first / total, s2, second / total);
}

// What the materials add to a point: its colour's ratio (1 where none), the tilt of its normal as a slope of the
// ground (dh/dx, dh/dz in chart metres, added to the land's own gradient) and, from the cliff read face on, as a
// gradient in the world (x, height, z; `wall_slope` turns it onto the land's normal), and which two materials (for the
// debug view).
struct GroundDetail {
    ratio: vec3<f32>,
    slope: vec2<f32>,
    relief: vec3<f32>,
    top: vec4<f32>,
}

fn no_detail() -> GroundDetail {
    var d: GroundDetail;
    d.ratio = vec3<f32>(1.0);
    d.slope = vec2<f32>(0.0);
    d.relief = vec3<f32>(0.0);
    d.top = vec4<f32>(f32(M_NONE), 0.0, f32(M_NONE), 0.0);
    return d;
}

// Where the land faces (its macro normal, unit) and how high it stands, with the height's screen derivatives, and the
// rock its cliff is (`cliff_rock`'s material: its layer, and its period, normal strength and contrast): what the cliff's
// read takes (`ground_detail`). `flat_wall` for none (the seabed: the cliff's own rock, read from above).
struct Wall {
    normal: vec3<f32>,
    height: f32,
    height_dx: f32,
    height_dy: f32,
    layer: i32,
    params: vec4<f32>,
}

fn flat_wall() -> Wall {
    var w: Wall;
    w.normal = vec3<f32>(0.0, 1.0, 0.0);
    w.height = 0.0;
    w.height_dx = 0.0;
    w.height_dy = 0.0;
    w.layer = i32(M_CLIFF);
    w.params = terrain.material_table[M_CLIFF];
    return w;
}

// ---- The cliffs' rock (`ground_materials::CliffRock`) ----

const CLIFF_CELLS: i32 = 32;

// A rock: its material (its layer in the array; period, normal strength, contrast), its colour (linear; w 1 where it
// stands in for the photograph's and the style's, 0 the style's own), its bedding (0-1) and its beds' thickness (m).
struct CliffRock {
    layer: i32,
    params: vec4<f32>,
    colour: vec4<f32>,
    strata: f32,
    bed_m: f32,
}

fn cliff_id(cell: vec2<i32>) -> u32 {
    let c = clamp(cell, vec2<i32>(0), vec2<i32>(CLIFF_CELLS - 1));
    let k = u32(c.y * CLIFF_CELLS + c.x);
    return (terrain.cliff_grid[k / 32u][(k / 8u) % 4u] >> (4u * (k % 8u))) & 15u;
}

// The cliffs' rock at a chart point (`terrain.cliff_grid`; the authored rock with no place, a battle map's): the
// material of the cell it stands in, its colour and bedding blended between the four cells about it, so a coast's rock
// changes over a few kilometres rather than along a cell's edge.
fn cliff_rock(chart: vec2<f32>) -> CliffRock {
    var r: CliffRock;
    if terrain.cliff_corner.w < 0.5 {
        r.layer = i32(terrain.cliff_rocks[0].x + 0.5);
        r.params = vec4<f32>(terrain.cliff_rocks[0].yzw, 0.0);
        r.colour = terrain.cliff_rocks[1];
        r.strata = terrain.cliff_rocks[2].x;
        r.bed_m = terrain.cliff_rocks[2].y;
        return r;
    }
    let at = (chart - terrain.cliff_corner.xy) / terrain.cliff_corner.z - 0.5;
    let base = vec2<i32>(floor(at));
    let f = at - floor(at);
    let ids = vec4<u32>(cliff_id(base), cliff_id(base + vec2<i32>(1, 0)), cliff_id(base + vec2<i32>(0, 1)),
        cliff_id(base + vec2<i32>(1, 1)));
    let w = vec4<f32>((1.0 - f.x) * (1.0 - f.y), f.x * (1.0 - f.y), (1.0 - f.x) * f.y, f.x * f.y);
    // The cell the point stands in: its material (a pair of materials is never read across the blend).
    let east = f.x >= 0.5;
    let south = f.y >= 0.5;
    let nearest = select(select(ids.x, ids.y, east), select(ids.z, ids.w, east), south);
    r.layer = i32(terrain.cliff_rocks[nearest * 3u].x + 0.5);
    r.params = vec4<f32>(terrain.cliff_rocks[nearest * 3u].yzw, 0.0);
    r.colour = terrain.cliff_rocks[ids.x * 3u + 1u] * w.x + terrain.cliff_rocks[ids.y * 3u + 1u] * w.y
        + terrain.cliff_rocks[ids.z * 3u + 1u] * w.z + terrain.cliff_rocks[ids.w * 3u + 1u] * w.w;
    let bedding = terrain.cliff_rocks[ids.x * 3u + 2u] * w.x + terrain.cliff_rocks[ids.y * 3u + 2u] * w.y
        + terrain.cliff_rocks[ids.z * 3u + 2u] * w.z + terrain.cliff_rocks[ids.w * 3u + 2u] * w.w;
    r.strata = bedding.x;
    r.bed_m = bedding.y;
    return r;
}

// Whether materials are bound and shown (a uniform). Never without the binding (`MATERIALS` undefined).
fn materials_bound() -> bool {
#ifdef MATERIALS
    return terrain.materials.x > 0.0;
#else
    return false;
#endif
}

// How strongly the materials show where a pixel spans `footprint` metres (its filtered footprint, `SurfaceInputs`).
fn materials_strength(footprint: f32) -> f32 {
    let t = clamp((footprint - terrain.materials.z) / max(terrain.materials.w - terrain.materials.z, 1e-3), 0.0, 1.0);
    return terrain.materials.y * (1.0 - t * t * (3.0 - 2.0 * t));
}

// A material's period (m), normal strength and contrast.
fn material_params(id: u32) -> vec4<f32> {
    return terrain.material_table[id];
}

#ifdef MATERIALS
// One material read: its ratio and height about their means (0.5), and its normal's x and y (0.5 flat) in the frame of
// the read (u to the image's right, v down it); `turn` turns that frame back into the chart's.
struct MaterialRead {
    colour: vec4<f32>,
    normal: vec2<f32>,
    turn: vec2<f32>,
}

// Material `id`'s texels at `uv` (tiles), hex-tiled, at the level its derivatives ask for (explicit levels, so this may
// run in non-uniform control flow; their geometric mean, and no anisotropic probes, which cost a third of the reads'
// time on grazing views, where the materials fade anyway).
fn material_texels(id: i32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> MaterialRead {
    let t = hex_taps_fast(uv);
    // The lattice from the whole coordinate (its corners' identities), the reads from its fraction: a tile's texels are
    // the same in every repeat, and a coordinate in the tens of thousands addresses no texel finely.
    let f = uv - floor(uv);
    let layer = id * 2;
    let texels = f32(textureDimensions(ground_materials).x);
    let lod = log2(max(sqrt(length(dx) * length(dy)) * texels, 1e-4));
    // Two corners at most: the lightest's weight taken from all three, so it is never read and where the two lightest
    // trade places neither shows (no seam: every weight is continuous across the lattice).
    let least = min(t.weights.x, min(t.weights.y, t.weights.z));
    var w = max(t.weights - least, vec3<f32>(0.0));
    w = w / max(w.x + w.y + w.z, 1e-5);
    var colour = vec4<f32>(0.0);
    if w.x > 0.0 {
        colour += (textureSampleLevel(ground_materials, repeat_sampler, f + t.a, layer, lod) - 0.5) * w.x;
    }
    if w.y > 0.0 {
        colour += (textureSampleLevel(ground_materials, repeat_sampler, f + t.b, layer, lod) - 0.5) * w.y;
    }
    if w.z > 0.0 {
        colour += (textureSampleLevel(ground_materials, repeat_sampler, f + t.c, layer, lod) - 0.5) * w.z;
    }
    // The contrast kept across the lattice's blend (about the means).
    let spread = 1.0 / max(sqrt(dot(w, w)), 1e-3);
    // The normal from the corner that weighs most alone (one read), faded to flat where the two heaviest come level, so
    // it is continuous where they trade places.
    let dominant = select(select(t.c, t.b, w.y >= w.z), t.a, w.x >= w.y && w.x >= w.z);
    let most = max(w.x, max(w.y, w.z));
    let next = w.x + w.y + w.z - most - min(w.x, min(w.y, w.z));
    let normal = (textureSampleLevel(ground_materials, repeat_sampler, f + dominant, layer + 1, lod).xy - 0.5)
        * clamp((most - next) * 3.0, 0.0, 1.0);
    var out: MaterialRead;
    out.colour = colour * spread + 0.5;
    out.normal = normal + 0.5;
    out.turn = vec2<f32>(1.0, 0.0);
    return out;
}

// One material read on the chart (as from above) at a chart point: layer `layer` of the array, `period` metres a repeat,
// on the heading of material `id`'s own, so no two share their lattice or their grain's direction. `chart_dx`,
// `chart_dy`: the point's screen derivatives, which set the level read.
fn chart_read(id: u32, layer: i32, period: f32, chart: vec2<f32>, chart_dx: vec2<f32>, chart_dy: vec2<f32>) -> MaterialRead {
    let angle = f32(id) * 2.399963 + 0.7;
    let turn = vec2<f32>(cos(angle), sin(angle));
    let m = mat2x2<f32>(turn.x, turn.y, -turn.y, turn.x);
    var out = material_texels(layer, (m * chart) / period, (m * chart_dx) / period, (m * chart_dy) / period);
    out.turn = turn;
    return out;
}

fn material_read(id: u32, chart: vec2<f32>, chart_dx: vec2<f32>, chart_dy: vec2<f32>) -> MaterialRead {
    return chart_read(id, i32(id), material_params(id).x, chart, chart_dx, chart_dy);
}

// A read's colour ratio by its contrast (`p.z` of its period, normal strength and contrast `p`).
fn read_ratio(p: vec4<f32>, colour: vec4<f32>) -> vec3<f32> {
    return max(vec3<f32>(1.0) + (colour.rgb * 2.0 - 1.0) * p.z, vec3<f32>(0.05));
}

// The read's height's slope in its own frame (dh/du, dh/dv: u to the image's right, v down it; OpenGL's green is up
// the image), by its normal's strength (`p.y`).
fn read_tilt(p: vec4<f32>, read: MaterialRead) -> vec2<f32> {
    let xy = (read.normal - 0.5) * 2.0;
    let z = sqrt(max(1.0 - dot(xy, xy), 0.05));
    return vec2<f32>(-xy.x, xy.y) / z * p.y;
}

// Its slope in chart metres: turned back into the chart's frame.
fn read_slope(p: vec4<f32>, read: MaterialRead) -> vec2<f32> {
    let s = read_tilt(p, read);
    let c = read.turn;
    return vec2<f32>(c.x * s.x + c.y * s.y, -c.y * s.x + c.x * s.y);
}

// What a cliff read gives the land: its ratio and height (as `MaterialRead`'s colour), its slope on the chart (the read
// from above) and its gradient in the world (the reads face on).
struct CliffRead {
    colour: vec4<f32>,
    slope: vec2<f32>,
    relief: vec3<f32>,
}

// The cliff's rock read on a wall: face on along the horizontal axes the wall faces (along x the image across z, along
// z across x; up kept up, so the rock's joints and beds stand as they do), from above where it lies back
// (`material_read`), the projections weighed by `wall_weights`. Each face-on read is hex-tiled on the wall's plane, at
// the level its derivatives ask.
fn cliff_read(chart: vec2<f32>, chart_dx: vec2<f32>, chart_dy: vec2<f32>, wall: Wall) -> CliffRead {
    let w = wall_weights(wall.normal);
    let p = wall.params;
    let layer = wall.layer;
    var out: CliffRead;
    out.colour = vec4<f32>(0.0);
    out.slope = vec2<f32>(0.0);
    out.relief = vec3<f32>(0.0);
    if w.y > 0.0 {
        let read = chart_read(M_CLIFF, layer, p.x, chart, chart_dx, chart_dy);
        out.colour += (read.colour - 0.5) * w.y;
        out.slope = read_slope(p, read) * w.y;
    }
    // Up the image is up the wall: v runs down it.
    let v = -wall.height / p.x;
    let v_dx = -wall.height_dx / p.x;
    let v_dy = -wall.height_dy / p.x;
    if w.x > 0.0 {
        // Facing +x the image's right runs toward -z (toward +z facing -x): it reads unmirrored from either side.
        let side = select(-1.0, 1.0, wall.normal.x < 0.0);
        let read = material_texels(layer, vec2<f32>(side * chart.y / p.x, v),
            vec2<f32>(side * chart_dx.y / p.x, v_dx), vec2<f32>(side * chart_dy.y / p.x, v_dy));
        out.colour += (read.colour - 0.5) * w.x;
        let t = read_tilt(p, read);
        out.relief += vec3<f32>(0.0, -t.y, side * t.x) * w.x;
    }
    if w.z > 0.0 {
        let side = select(-1.0, 1.0, wall.normal.z > 0.0);
        let read = material_texels(layer, vec2<f32>(side * chart.x / p.x + 0.37, v),
            vec2<f32>(side * chart_dx.x / p.x, v_dx), vec2<f32>(side * chart_dy.x / p.x, v_dy));
        out.colour += (read.colour - 0.5) * w.z;
        let t = read_tilt(p, read);
        out.relief += vec3<f32>(side * t.x, -t.y, 0.0) * w.z;
    }
    // The blend's contrast kept, as the hex lattice keeps its own.
    out.colour = out.colour / max(sqrt(dot(w, w)), 1e-3) + 0.5;
    return out;
}

// What the forest floor's read gives the land: its colour ratio, its slope on the chart and its height.
struct FloorRead {
    ratio: vec3<f32>,
    slope: vec2<f32>,
    height: f32,
}

// The forest floor (`M_FOREST`) read where `needles` of it is a conifer wood's (`surface.wgsl`'s `floor_at`): its own
// leaf litter, the needles (`terrain.needles`: their layer, period, normal strength and contrast) for that share, both
// read only between (the needles' patches' edges), on their own heading.
fn floor_read(chart: vec2<f32>, chart_dx: vec2<f32>, chart_dy: vec2<f32>, needles: f32) -> FloorRead {
    var out: FloorRead;
    out.ratio = vec3<f32>(0.0);
    out.slope = vec2<f32>(0.0);
    out.height = 0.0;
    if needles < 1.0 {
        let p = material_params(M_FOREST);
        let read = material_read(M_FOREST, chart, chart_dx, chart_dy);
        out.ratio = read_ratio(p, read.colour) * (1.0 - needles);
        out.slope = read_slope(p, read) * (1.0 - needles);
        out.height = read.colour.a * (1.0 - needles);
    }
    if needles > 0.0 {
        let p = vec4<f32>(terrain.needles.yzw, 0.0);
        let read = chart_read(NEEDLES_HEADING, i32(terrain.needles.x + 0.5), p.x, chart, chart_dx, chart_dy);
        out.ratio += read_ratio(p, read.colour) * needles;
        out.slope += read_slope(p, read) * needles;
        out.height += read.colour.a * needles;
    }
    return out;
}
#endif

// The needles' heading's id (`chart_read`): past every chosen material's, so their lattice and grain are their own.
const NEEDLES_HEADING: u32 = 16u;

// The weights of a wall's three projections (along x, the chart's y and z) by its normal: Quilez's biplanar mapping,
// the two the normal leans toward most, the third's weight nothing. None face on where the land is gentler than about
// 35 degrees (no horizontal axis past 0.577 of the normal): the land seen from above reads the chart alone.
fn wall_weights(n: vec3<f32>) -> vec3<f32> {
    let a = abs(n);
    var w = clamp((a - 0.5773) / (1.0 - 0.5773), vec3<f32>(0.0), vec3<f32>(1.0));
    w = w * w;
    let total = w.x + w.y + w.z;
    if total < 1e-5 {
        // The three level (a face at 55 degrees to each axis): the chart's.
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return w / total;
}

// The materials' detail at a point from their weights `m`, `strength` already faded by the pixel's footprint
// (`materials_strength`): the two heaviest read and blended by their heights; the cliff read face on where `wall`
// stands steep (`cliff_read`); the forest floor needles for the share `needles` of it (`floor_read`).
fn ground_detail(m: Mats, chart: vec2<f32>, chart_dx: vec2<f32>, chart_dy: vec2<f32>, strength: f32, wall: Wall, needles: f32) -> GroundDetail {
    var d = no_detail();
#ifdef MATERIALS
    if strength <= 0.0 {
        return d;
    }
    var top = mats_top(m);
    // A second material under a tenth of the pair is not read (most of the land carries a little of one), and fades in
    // between a tenth and a fifth, so none pops.
    let second_share = clamp((top.w - MATERIAL_SECOND_MIN) / MATERIAL_SECOND_MIN, 0.0, 1.0);
    top = vec4<f32>(top.x, top.y + top.w * (1.0 - second_share), top.z, top.w * second_share);
    d.top = top;
    let first = u32(top.x + 0.5);
    let second = u32(top.z + 0.5);
    var ratio_a = vec3<f32>(1.0);
    var slope_a = vec2<f32>(0.0);
    var relief_a = vec3<f32>(0.0);
    var height_a = 0.5;
    if first == M_CLIFF && top.y > 0.0 {
        let read = cliff_read(chart, chart_dx, chart_dy, wall);
        ratio_a = read_ratio(wall.params, read.colour);
        slope_a = read.slope;
        relief_a = read.relief;
        height_a = read.colour.a;
    } else if first == M_FOREST && needles > 0.0 && top.y > 0.0 {
        let read = floor_read(chart, chart_dx, chart_dy, needles);
        ratio_a = read.ratio;
        slope_a = read.slope;
        height_a = read.height;
    } else if first != M_NONE && top.y > 0.0 {
        let read = material_read(first, chart, chart_dx, chart_dy);
        ratio_a = read_ratio(material_params(first), read.colour);
        slope_a = read_slope(material_params(first), read);
        height_a = read.colour.a;
    }
    var ratio_b = vec3<f32>(1.0);
    var slope_b = vec2<f32>(0.0);
    var relief_b = vec3<f32>(0.0);
    var height_b = 0.5;
    if second == M_CLIFF && top.w > 0.0 {
        let read = cliff_read(chart, chart_dx, chart_dy, wall);
        ratio_b = read_ratio(wall.params, read.colour);
        slope_b = read.slope;
        relief_b = read.relief;
        height_b = read.colour.a;
    } else if second == M_FOREST && needles > 0.0 && top.w > 0.0 {
        let read = floor_read(chart, chart_dx, chart_dy, needles);
        ratio_b = read.ratio;
        slope_b = read.slope;
        height_b = read.height;
    } else if second != M_NONE && top.w > 0.0 {
        let read = material_read(second, chart, chart_dx, chart_dy);
        ratio_b = read_ratio(material_params(second), read.colour);
        slope_b = read_slope(material_params(second), read);
        height_b = read.colour.a;
    }
    // Height-based: each side's weight raised by its height; the lower fades out over `MATERIAL_DEPTH` below the higher.
    let a = top.y + height_a * MATERIAL_HEIGHT;
    let b = top.w + height_b * MATERIAL_HEIGHT;
    let level = max(a, b) - MATERIAL_DEPTH;
    let wa = max(a - level, 0.0);
    let wb = max(b - level, 0.0);
    let total = max(wa + wb, 1e-5);
    let ratio = (ratio_a * wa + ratio_b * wb) / total;
    let slope = (slope_a * wa + slope_b * wb) / total;
    let relief = (relief_a * wa + relief_b * wb) / total;
    d.ratio = mix(vec3<f32>(1.0), ratio, strength);
    d.slope = slope * strength;
    d.relief = relief * strength;
#endif
    return d;
}

// The land's macro normal (from `gradient`) tilted by the chart's `slope`, as the land's fragment stage tilts it, then
// by a gradient `relief` in the world (Mikkelsen's surface gradient: its part along the land taken off the normal),
// given back as one slope of the ground (dh/dx, dh/dz) the fragment stage adds to `gradient`: a face tilted past the
// vertical is held at a steep lean (the land draws no overhangs).
fn wall_slope(gradient: vec2<f32>, slope: vec2<f32>, relief: vec3<f32>) -> vec2<f32> {
    let n = normalize(vec3<f32>(-gradient.x - slope.x, 1.0, -gradient.y - slope.y));
    let tilted = normalize(n - (relief - dot(relief, n) * n));
    let up = max(tilted.y, 0.05);
    return vec2<f32>(-tilted.x / up - gradient.x, -tilted.z / up - gradient.y);
}
