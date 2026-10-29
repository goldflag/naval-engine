// The land's own light at night (`naval_terrain::night`, native/docs/terrain/night.md): the paved roads under
// their street lights where the map's pages reach, and beyond them the built-up land's glow, points of light where a
// pixel is finer than the blocks they stand in and their mean beyond. The lamps themselves are `lamps.wgsl`'s; the
// colours of both are here. Nothing by day (`terrain.night.x` 0) or where the scenario's rule puts the lights out.
#define_import_path naval_terrain::night

#import naval_terrain::bindings::terrain
#import naval_terrain::features::{FeatureSite, feature_band, feature_sample, feature_texel, features_bound, FEATURE_HALF_WIDTH_MAX_M}

// What a lamp is (`night::LampKind`).
const LAMP_STREET: u32 = 0u;
const LAMP_FLOOD: u32 = 1u;
const LAMP_POOL: u32 = 2u;
const LAMP_BEACON: u32 = 3u;
const LAMP_RUNWAY: u32 = 4u;

// The lights' colours (linear, unit red, the green unit green): high-pressure sodium; a warm white (3000 K) and a
// cooler one (4000 K, LED); mercury and fluorescent's blue-green white (Japan's streets); metal halide over a yard;
// aviation red; a runway's edge white and threshold green; a window's.
const SODIUM: vec3<f32> = vec3<f32>(1.0, 0.42, 0.06);
const WARM: vec3<f32> = vec3<f32>(1.0, 0.55, 0.22);
const WHITE: vec3<f32> = vec3<f32>(1.0, 0.70, 0.42);
const MERCURY: vec3<f32> = vec3<f32>(0.72, 0.92, 1.0);
const HALIDE: vec3<f32> = vec3<f32>(1.0, 0.82, 0.62);
const AVIATION_RED: vec3<f32> = vec3<f32>(1.0, 0.03, 0.01);
const RUNWAY_WHITE: vec3<f32> = vec3<f32>(1.0, 0.78, 0.5);
const RUNWAY_GREEN: vec3<f32> = vec3<f32>(0.1, 1.0, 0.35);
const WINDOW: vec3<f32> = vec3<f32>(1.0, 0.62, 0.3);

// The light a street light lays on the ground under it (the lit ground's radiance over its albedo, at the foot of a
// secondary road's lamp), and the built-up land's glow beyond the pages (its mean luminance where the land is wholly
// built up), the blocks its points stand in (m), and how small a point is drawn (m).
const STREET_E: f32 = 2.2;
const GLOW_LUM: f32 = 0.04;
const NIGHT_CELL_M: f32 = 46.0;
const NIGHT_POINT_M: f32 = 2.5;
// How much the glow follows the photograph's bright streets and yards (its local contrast, `Surface::photo_detail`).
const GLOW_DETAIL: f32 = 3.0;

fn night_hash(n: u32) -> f32 {
    var x = n * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x) * (1.0 / 4294967295.0);
}

fn night_cell_hash(cell: vec2<f32>, salt: u32) -> f32 {
    let i = vec2<i32>(cell);
    return night_hash(bitcast<u32>(i.x) * 73856093u ^ bitcast<u32>(i.y) * 19349663u ^ salt);
}

fn night_ramp(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn white_palette() -> bool {
    return terrain.night_rule.w > 0.5;
}

// A lamp's colour by its kind, its road's class (0-3 the major roads) and its own hash `h`.
fn lamp_color(kind: u32, road: u32, h: f32) -> vec3<f32> {
    let white = white_palette();
    if kind == LAMP_BEACON {
        return AVIATION_RED;
    }
    if kind == LAMP_RUNWAY {
        // An edge light white, a threshold's green.
        return select(RUNWAY_WHITE, RUNWAY_GREEN, road == 1u);
    }
    if kind == LAMP_FLOOD || kind == LAMP_POOL {
        return select(select(SODIUM, HALIDE, h < 0.35), select(HALIDE, SODIUM, h < 0.2), white);
    }
    let major = road <= 3u;
    if white {
        return select(select(WHITE, WARM, h < 0.6), select(MERCURY, WHITE, h < 0.5), major);
    }
    let minor = select(select(SODIUM, WARM, h < 0.55), WHITE, h < 0.15);
    return select(minor, select(SODIUM, WHITE, h < 0.1), major);
}

// A lit road's colour by its tone (the class's shade: the major roads toward 1).
fn road_light_color(tone: f32) -> vec3<f32> {
    let minor = 1.0 - night_ramp(0.55, 0.7, tone);
    return select(mix(SODIUM, WARM, minor * 0.45), mix(mix(MERCURY, WHITE, 0.5), WARM, minor * 0.6), white_palette());
}

fn glow_color() -> vec3<f32> {
    return select(mix(SODIUM, WARM, 0.4), mix(WARM, MERCURY, 0.3), white_palette());
}

// The far glow's points: in each `NIGHT_CELL_M` block one light at a place of its own (three in four lit), its light
// spread over a few metres or the pixel, the mean (1) where a pixel spans half a block; and districts brighter and darker
// over a few hundred metres.
fn night_points(chart: vec2<f32>, footprint: f32) -> f32 {
    let q = chart / 640.0;
    let base = floor(q);
    let f = q - base;
    let s = f * f * (3.0 - 2.0 * f);
    let district = mix(mix(night_cell_hash(base, 7u), night_cell_hash(base + vec2<f32>(1.0, 0.0), 7u), s.x),
        mix(night_cell_hash(base + vec2<f32>(0.0, 1.0), 7u), night_cell_hash(base + vec2<f32>(1.0, 1.0), 7u), s.x), s.y);
    let variation = 0.45 + 1.1 * district;
    let shown = night_ramp(NIGHT_CELL_M * 0.5, NIGHT_CELL_M * 0.15, footprint);
    if shown <= 0.0 {
        return variation;
    }
    let p = chart / NIGHT_CELL_M;
    let cell = floor(p);
    let at = vec2<f32>(night_cell_hash(cell, 1u), night_cell_hash(cell, 2u)) * 0.7 + 0.15;
    let d = length(p - cell - at) * NIGHT_CELL_M;
    let r = max(NIGHT_POINT_M, footprint * 0.7);
    let lit = select(0.0, 1.0 / 0.75, night_cell_hash(cell, 3u) < 0.75);
    let point = exp(-(d * d) / (r * r)) * (NIGHT_CELL_M * NIGHT_CELL_M) / (3.14159265 * r * r) * lit;
    return mix(1.0, point, shown) * variation;
}

// A paved road's street lights by its tone and half width (m), as `night::street_light`: x the spacing (m, 0 where
// unlit), y the height (m), z the brightness. The lamps stand where the road crosses the lattice's lines (x or z a
// multiple of the spacing in the theatre's metres, `terrain.night_grid` the chart's origin there), so each pool falls
// under its lamp.
fn street_light(tone: f32, half: f32) -> vec3<f32> {
    if tone >= 0.95 {
        return vec3<f32>(55.0, 12.0, 1.4);
    }
    if tone >= 0.85 {
        return vec3<f32>(45.0, 11.0, 1.3);
    }
    if tone >= 0.70 {
        return vec3<f32>(40.0, 10.0, 1.2);
    }
    if tone >= 0.60 {
        return vec3<f32>(42.0, 9.0, 1.0);
    }
    if tone >= 0.47 {
        return vec3<f32>(45.0, 8.0, 0.9);
    }
    if tone >= 0.42 {
        return vec3<f32>(50.0, 7.0, 0.7);
    }
    if tone >= 0.30 && tone < 0.38 && half >= 2.2 {
        return vec3<f32>(60.0, 6.0, 0.5);
    }
    return vec3<f32>(0.0);
}

// A lamp `h` m up lighting the ground `r` (squared, m²) from its foot: an even lamp's cosine-cubed fall-off, 1 beneath.
fn street_pool(r2: f32, h: f32) -> f32 {
    let q = 1.0 + r2 / (h * h);
    return 1.0 / (q * sqrt(q));
}

// The street lights' light on the ground at chart point `chart` by the paved road nearest (`site`), of the light at a
// lamp's foot: the pools of the two lamps either side along the road where a pixel (`footprint` m) is finer than a
// fraction of their spacing, else their mean along it; across, their fall-off, or where a pixel is wider than it a band
// holding as much light (the features' filter). The road's direction is the distance field's gradient (two reads more,
// only near a lit road).
fn street_light_at(site: FeatureSite, chart: vec2<f32>, footprint: f32) -> f32 {
    let half = site.attrs.x * FEATURE_HALF_WIDTH_MAX_M;
    let lamp = street_light(site.attrs.y, half);
    let s = lamp.x;
    let h = lamp.y;
    if s <= 0.0 {
        return 0.0;
    }
    // From the road's middle line (m).
    let c = max(site.distance.x + half, 0.0);
    if c > h * 5.0 + site.px.x {
        return 0.0;
    }
    let across_fine = 1.0 / (1.0 + c * c / (h * h));
    let across = mix(across_fine, feature_band(c, 1.5708 * h, site.px.x), night_ramp(0.5 * h, 2.0 * h, site.px.x));
    var light = across * (2.0 * h / s);
    let pattern = night_ramp(s * 0.3, s * 0.12, footprint);
    if pattern > 0.0 {
        let e = feature_texel(site.level);
        let gx = feature_sample(site.level, chart + vec2<f32>(e, 0.0)).distance.x - site.distance.x;
        let gz = feature_sample(site.level, chart + vec2<f32>(0.0, e)).distance.x - site.distance.x;
        // The road runs along x where the distance changes less that way.
        let along_x = abs(gx) <= abs(gz);
        let direction = normalize(vec2<f32>(-gz, gx) + vec2<f32>(1e-6, 0.0));
        let theatre = chart + terrain.night_grid.xy;
        let u = select(theatre.y, theatre.x, along_x);
        let cosine = max(select(abs(direction.y), abs(direction.x), along_x), 0.7071);
        let k = floor(u / s);
        let a1 = (u - k * s) / cosine;
        let a2 = (s - (u - k * s)) / cosine;
        let pools = street_pool(a1 * a1 + c * c, h) + street_pool(a2 * a2 + c * c, h);
        light = mix(light, pools, pattern);
    }
    return light * lamp.z;
}

// The land's light at night at chart point `chart`, `height` m up, its albedo `albedo`: the ground under the street
// lights of the paved roads at `site` (`feature_site`) where the land is built up (`built`, the window's cover: 0-1), and
// the built-up land's glow where the pages do not reach, brighter where the photograph is (`detail`, its local
// contrast); `footprint` the metres a pixel spans. Linear radiance, added to the lit land before the air.
fn night_land(site: FeatureSite, built: f32, detail: f32, albedo: vec3<f32>, chart: vec2<f32>, height: f32,
    footprint: f32) -> vec3<f32> {
    let night = terrain.night.x;
    if night <= 0.0 || built <= 0.0 {
        return vec3<f32>(0.0);
    }
    let land = night_ramp(-0.2, 0.6, height);
    var light = vec3<f32>(0.0);
    let town = night_ramp(0.1, 0.45, built);
    if features_bound() && terrain.night.y > 0.0 && site.share > 0.0 {
        let street = street_light_at(site, chart, footprint);
        if street > 0.0 {
            light += road_light_color(site.attrs.y) * albedo
                * (STREET_E * street * site.share * land * town * terrain.night.y);
        }
    }
    let far = built * (1.0 - site.share) * land;
    if far > 0.0 && terrain.night.z > 0.0 {
        let streets = clamp(1.0 + GLOW_DETAIL * (detail - 1.0), 0.15, 3.0);
        light += glow_color() * (GLOW_LUM * far * streets * terrain.night.z) * night_points(chart, footprint);
    }
    return light * night;
}
