// Map features on the ground: roads, railways, rivers and canals from OpenStreetMap, drawn over the land's colour from
// distance fields on pages about the camera (`naval_terrain::features`, native/docs/terrain/surface.md, "Map features
// on the
// ground"). New to the native client (the browser game had no such thing). Kept apart from `surface.wgsl`, which calls
// `feature_cover` once for the land's colour (with the `feature_site` the land's fragment stage takes up front, where
// screen derivatives may be) and `feature_clearance` once for the trees.
//
// Two levels of pages (`terrain.features`: level 0's and level 1's texel, the slots a side, 1 while bound), each a pair
// of layers in `surface_features`: distances (m, encoded 0.5 + d / (2 R) over the level's range R) to paved roads'
// edges, unpaved roads' edges, railways' middle lines and water's edges, and the nearest feature's attributes (the
// paved road's half width and tone, the unpaved road's half width, the water's half width). Page (i, j) of a level lies
// in slot (i mod slots, j mod slots), read repeating; a slot draws only where its entry names the page at the point.
// One level is read a pixel, two texture reads, in uniform control flow (the level is chosen by `select`), so the
// distances' screen derivatives give each band its filter width across it.
#define_import_path naval_terrain::features

#ifdef FEATURES
#import naval_terrain::bindings::{terrain, repeat_sampler, surface_features}
#else
#import naval_terrain::bindings::terrain
#endif

const FEATURE_PAGE_TEXELS: f32 = 256.0;
const FEATURE_RANGE_TEXELS: f32 = 8.0;
const FEATURE_HALF_WIDTH_MAX_M: f32 = 16.0;
const FEATURE_WATER_HALF_WIDTH_MAX_M: f32 = 32.0;
// A railway's ballast, half its width, and its rails' half gauge (standard gauge, 1435 mm) and half head (m).
const BALLAST_HALF_M: f32 = 1.6;
const RAIL_GAUGE_HALF_M: f32 = 0.7175;
const RAIL_HEAD_HALF_M: f32 = 0.04;
// The muddy margin of a river or canal (m).
const BANK_M: f32 = 2.5;
// Level 0 is read where a pixel spans less than this many of its texels (level 1 is as good beyond).
const LEVEL_ZERO_PIXELS: f32 = 3.0;
// The feather toward a level-1 page not drawn, of a page's side.
const FEATURE_FEATHER: f32 = 0.15;

// The land at a point with the features laid over it: the colour, roughness and grazing reflectance, composited once by
// each feature's coverage, and how much of the point they cover in all (0 where none).
struct FeatureCover {
    color: vec3<f32>,
    roughness: f32,
    specular: f32,
    weight: f32,
}

// Whether pages are bound and shown (a uniform). Never on a device without the binding (`FEATURES` undefined).
fn features_bound() -> bool {
#ifdef FEATURES
    return terrain.features.w > 0.0;
#else
    return false;
#endif
}

fn feature_ramp(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn feature_texel(level: i32) -> f32 {
    return select(terrain.features.y, terrain.features.x, level == 0);
}

fn feature_origin(level: i32) -> vec2<f32> {
    return select(terrain.feature_origin.zw, terrain.feature_origin.xy, level == 0);
}

// Whether page `page` of `level` is drawn (its slot holds it).
fn feature_drawn(level: i32, page: vec2<i32>) -> bool {
    let slots = max(i32(terrain.features.z), 1);
    let slot = ((page % slots) + slots) % slots;
    let k = slot.y * slots + slot.x;
    let entry = terrain.feature_slots[level * 32 + k / 2];
    let held = select(entry.zw, entry.xy, k % 2 == 0);
    return all(held == page);
}

// Chart point `chart` in level `level`'s texels (texel (i, j) from (i, j) to (i + 1, j + 1) of the page grid).
fn feature_texels(level: i32, chart: vec2<f32>) -> vec2<f32> {
    return (chart - feature_origin(level)) / feature_texel(level);
}

// How much of a point level 1 paints: 1 inside a drawn page, fading toward an edge whose neighbour is not drawn.
fn feature_share_far(chart: vec2<f32>) -> f32 {
    let t = feature_texels(1, chart) / FEATURE_PAGE_TEXELS;
    let base = floor(t);
    let page = vec2<i32>(base);
    if !feature_drawn(1, page) {
        return 0.0;
    }
    let f = t - base;
    var share = 1.0;
    if !feature_drawn(1, page - vec2<i32>(1, 0)) {
        share = share * feature_ramp(0.0, FEATURE_FEATHER, f.x);
    }
    if !feature_drawn(1, page + vec2<i32>(1, 0)) {
        share = share * feature_ramp(0.0, FEATURE_FEATHER, 1.0 - f.x);
    }
    if !feature_drawn(1, page - vec2<i32>(0, 1)) {
        share = share * feature_ramp(0.0, FEATURE_FEATHER, f.y);
    }
    if !feature_drawn(1, page + vec2<i32>(0, 1)) {
        share = share * feature_ramp(0.0, FEATURE_FEATHER, 1.0 - f.y);
    }
    return share;
}

// The level a point reads: 0 where its page is drawn and a pixel spans no more than `LEVEL_ZERO_PIXELS` of its texels,
// else 1; and how much of the point it paints.
struct FeatureLevel {
    level: i32,
    share: f32,
}

fn feature_level(chart: vec2<f32>, footprint: f32) -> FeatureLevel {
    var out: FeatureLevel;
    let near_page = vec2<i32>(floor(feature_texels(0, chart) / FEATURE_PAGE_TEXELS));
    let near = feature_drawn(0, near_page) && footprint < terrain.features.x * LEVEL_ZERO_PIXELS;
    out.level = select(1, 0, near);
    out.share = select(feature_share_far(chart), 1.0, near);
    return out;
}

// Level `level`'s distances (m: paved edge, unpaved edge, railway middle, water edge) and attributes at `chart`, read
// at level 0 of the texture (explicit, so callable anywhere).
struct FeatureSample {
    distance: vec4<f32>,
    attrs: vec4<f32>,
}

fn feature_sample(level: i32, chart: vec2<f32>) -> FeatureSample {
    var out: FeatureSample;
#ifdef FEATURES
    let span = max(terrain.features.z, 1.0) * FEATURE_PAGE_TEXELS;
    let uv = feature_texels(level, chart) / span;
    let edges = textureSampleLevel(surface_features, repeat_sampler, uv, level * 2, 0.0);
    out.attrs = textureSampleLevel(surface_features, repeat_sampler, uv, level * 2 + 1, 0.0);
#else
    // Nothing within reach.
    let edges = vec4<f32>(1.0);
    out.attrs = vec4<f32>(0.0);
#endif
    out.distance = (edges - 0.5) * (2.0 * FEATURE_RANGE_TEXELS * feature_texel(level));
    return out;
}

// How much of a pixel `px` wide (across the band), its middle `centre` from a band's middle, a band of half width
// `half` covers: the box filter's overlap.
fn feature_band(centre: f32, half: f32, px: f32) -> f32 {
    let h = 0.5 * px;
    let c = abs(centre);
    return clamp((min(half, c + h) - max(-half, c - h)) / max(px, 1e-4), 0.0, 1.0);
}

// A distance's filter width across its feature: its screen derivatives' length (the field's gradient is 1 a metre).
fn feature_px(d: f32, footprint: f32) -> f32 {
    return clamp(length(vec2<f32>(dpdx(d), dpdy(d))), 0.01, max(footprint * 4.0, 0.02));
}

fn feature_srgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    return pow(vec3<f32>(r, g, b) / 255.0, vec3<f32>(2.2));
}

// The features at a point: its level's distances and attributes, each distance's filter width across its feature, and
// how much of the point the level paints.
struct FeatureSite {
    distance: vec4<f32>,
    attrs: vec4<f32>,
    px: vec4<f32>,
    share: f32,
    // The level read (`feature_level`).
    level: i32,
}

// The features at chart point `chart`, a pixel spanning `footprint` m. Call in uniform control flow (it takes screen
// derivatives), so `feature_cover` may run anywhere; nothing with no features bound.
fn feature_site(chart: vec2<f32>, footprint: f32) -> FeatureSite {
    var out: FeatureSite;
    out.distance = vec4<f32>(1.0);
    out.attrs = vec4<f32>(0.0);
    out.px = vec4<f32>(1.0);
    out.share = 0.0;
    out.level = 1;
    if !features_bound() {
        return out;
    }
    let lv = feature_level(chart, footprint);
    let s = feature_sample(lv.level, chart);
    let d = s.distance;
    out.distance = d;
    out.attrs = s.attrs;
    out.px = vec4<f32>(feature_px(d.x, footprint), feature_px(d.y, footprint), feature_px(d.z, footprint),
        feature_px(d.w, footprint));
    out.share = lv.share;
    out.level = lv.level;
    return out;
}

// The features at `site` (`feature_site`) over land coloured `below` (its roughness and grazing reflectance after it),
// `height` m up; the land as it was, weight 0, with no features bound.
fn feature_cover(site: FeatureSite, height: f32, below: vec3<f32>, below_roughness: f32, below_specular: f32)
    -> FeatureCover {
    var out: FeatureCover;
    out.color = below;
    out.roughness = below_roughness;
    out.specular = below_specular;
    out.weight = 0.0;
    if !features_bound() {
        return out;
    }
    let s = site;
    let d = s.distance;
    let px = s.px;
    // Only on land: nothing on the seabed or the shore's wet band (a bridge over the sea goes too).
    let land = feature_ramp(-0.2, 0.6, height) * s.share;
    let lum = dot(below, vec3<f32>(0.2126, 0.7152, 0.0722));
    let paved_half = s.attrs.x * FEATURE_HALF_WIDTH_MAX_M;
    let tone = s.attrs.y;
    let unpaved_half = s.attrs.z * FEATURE_HALF_WIDTH_MAX_M;
    let water_half = s.attrs.w * FEATURE_WATER_HALF_WIDTH_MAX_M;

    // Water first (a road crosses it on its bridge): a dark, smooth surface between muddy banks.
    let water_centre = max(d.w + water_half, 0.0);
    let water = feature_band(water_centre, water_half, px.w) * land;
    let bank = feature_band(water_centre, water_half + BANK_M, px.w) * land * (1.0 - water);
    var color = below;
    var roughness = below_roughness;
    var specular = below_specular;
    var weight = 0.0;
    let mud = below * vec3<f32>(0.72, 0.7, 0.66);
    color = mix(color, mud, bank * 0.6);
    weight = max(weight, bank * 0.6);
    let river = mix(feature_srgb(40.0, 52.0, 54.0), below * 0.5, 0.35);
    color = mix(color, river, water);
    roughness = mix(roughness, 0.12, water);
    specular = mix(specular, 1.0, water);
    weight = max(weight, water);

    // Unpaved: earth and gravel, half the ground's own colour.
    let unpaved = feature_band(max(d.y + unpaved_half, 0.0), unpaved_half, px.y) * land;
    let dirt = mix(feature_srgb(128.0, 113.0, 92.0), below * 1.15, 0.4);
    color = mix(color, dirt, unpaved * 0.85);
    roughness = mix(roughness, 0.95, unpaved);
    weight = max(weight, unpaved * 0.85);

    // Paved: asphalt, the major roads darker; white edge lines on the major roads where a pixel is finer than them.
    let paved_centre = max(d.x + paved_half, 0.0);
    let paved = feature_band(paved_centre, paved_half, px.x) * land;
    var asphalt = mix(feature_srgb(88.0, 87.0, 84.0), feature_srgb(50.0, 50.0, 52.0), tone);
    asphalt = mix(asphalt, vec3<f32>(lum), 0.1);
    let edge_line = feature_band(paved_centre - (paved_half - 0.45), 0.075, px.x) * feature_ramp(0.68, 0.72, tone)
        * feature_ramp(2.6, 3.0, paved_half);
    asphalt = mix(asphalt, feature_srgb(196.0, 196.0, 190.0), edge_line);
    color = mix(color, asphalt, paved);
    roughness = mix(roughness, 0.85, paved);
    specular = mix(specular, 0.35, paved);
    weight = max(weight, paved);

    // Railways: grey-brown ballast, and the rails a dark steel pair where a pixel can hold them.
    let ballast = feature_band(d.z, BALLAST_HALF_M, px.z) * land;
    let rails = feature_band(abs(d.z) - RAIL_GAUGE_HALF_M, RAIL_HEAD_HALF_M, px.z);
    let track = mix(feature_srgb(104.0, 97.0, 90.0), feature_srgb(64.0, 60.0, 57.0), rails * 0.9);
    color = mix(color, track, ballast);
    roughness = mix(roughness, mix(0.95, 0.45, rails), ballast);
    specular = mix(specular, mix(0.3, 0.7, rails), ballast);
    weight = max(weight, ballast);

    out.color = color;
    out.roughness = roughness;
    out.specular = specular;
    out.weight = clamp(weight, 0.0, 1.0);
    return out;
}

// How much a tree standing at `chart` would stand on a road, railway or river (with a couple of metres' margin): the
// trees keep off them. No derivatives: callable from the trees' placement.
fn feature_clearance(chart: vec2<f32>) -> f32 {
    return feature_clearance_by(chart, 2.0);
}

// As `feature_clearance`, with `margin` metres past the feature's edge.
fn feature_clearance_by(chart: vec2<f32>, margin: f32) -> f32 {
    if !features_bound() {
        return 0.0;
    }
    let lv = feature_level(chart, 0.0);
    let s = feature_sample(lv.level, chart);
    let paved = 1.0 - feature_ramp(0.0, margin, s.distance.x);
    let unpaved = 1.0 - feature_ramp(0.0, margin, s.distance.y);
    let rail = 1.0 - feature_ramp(BALLAST_HALF_M, BALLAST_HALF_M + margin, abs(s.distance.z));
    let water = 1.0 - feature_ramp(0.0, margin, s.distance.w);
    return max(max(paved, unpaved), max(rail, water)) * lv.share;
}
