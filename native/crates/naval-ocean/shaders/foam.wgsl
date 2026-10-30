// Ports the web game's `ocean/surface/foam.ts`: foam shading for the ocean surface. Whitecaps through their life (a small,
// dense, billowing core on the breaking crest and a larger patch_share of lacy, see-through foam streaked along the wind
// behind it, Monahan & Mac Niocaill 1986), the bubble cloud under them, wind-drawn windrows, and the light every kind
// of foam is lit by. The wave field decides where crests break and how long ago; this decides what they look like.
// Every function reading the foam texture takes screen-space derivatives: call them in uniform control flow.
#define_import_path naval_ocean::foam

#import naval_ocean::noise::{value_noise, noise_hash, noise_seed}

const PI: f32 = 3.141592653589793;
const FOAM_TEXELS: f32 = 256.0;
const FOAM_ANISOTROPY: f32 = 8.0;
// Metres across the wind per tile of the foam texture; along the wind it stretches with `windStretch`.
const FOAM_TILE: f32 = 40.0;
const STREAK_SCALE: f32 = 0.5;
const STREAK_STRETCH: f32 = 2.5;
// The windrows' band mask and patches, read at these multiples of the lace's scale (an irrational ratio apart).
const BAND_NEAR: f32 = 4.0;
const BAND_FAR: f32 = 6.3922453019264;
const BAND_STRETCH: f32 = 2.0;
const BAND_WARP_CELL: f32 = 600.0;
const BAND_WARP_ALONG: f32 = 100.0;
const BAND_WARP_ACROSS: f32 = 80.0;
const WINDROW_GATHER: f32 = 0.35;
const WINDROW_PEAK: f32 = 0.4;
const WINDROW_STREAKING: f32 = 0.3;
const WINDROW_EDGE: f32 = 0.35;
const RESIDUE_EDGE: f32 = 0.35;
// The finer lace: its layout's scale against the lace's, its turn (radians) and how far it moves a threshold.
const DETAIL_SCALE: f32 = 4.7;
const DETAIL_TURN: f32 = 1.1;
const DETAIL_GAIN: f32 = 0.3;
const FOAM_BLUR_START: f32 = 4.0;
const FOAM_BLUR_END: f32 = 24.0;
// Dense sea foam reflects about 0.55–0.75 of the light (Whitlock et al. 1982, Frouin et al. 1996): its bubbles shade
// each other, so even the freshest white water is no white paper. The freshest, densest white water stands at the top of
// that range (0.7 drew sunlit white water as grey beside a hull's white paint).
const FOAM_ALBEDO: f32 = 0.75;
const FOAM_WRAP: f32 = 0.5;
const FOAM_SUN: f32 = 0.7;
const FOAM_GLOSS: f32 = 0.5;
// A breaking crest's white water: it thickens from CORE_START of fresh foam to CORE_FULL over a wide ramp, so it
// dissolves into flecks toward its edge instead of being cut along one level of the field.
const CORE_START: f32 = 0.45;
const CORE_FULL: f32 = 1.4;
const CORE_COVER: f32 = 0.8;
const CORE_EDGE: f32 = 0.2;
// Its outline is torn by the churn's billows as much as by the lace: two scales of noise, not one contour.
const CORE_TORN: f32 = 0.45;
// The opacity of its outermost flecks: spatter and bubbles over the water, not paint.
const CORE_THIN: f32 = 0.4;
const PATCH_GONE: f32 = 0.01;
const PATCH_SOFT: f32 = 0.5;
const PATCH_FULL: f32 = 0.35;
const RESIDUE_SPARSE: f32 = 0.22;
const RESIDUE_COVER: f32 = 0.62;
const RESIDUE_FAINT: f32 = 0.15;
const RESIDUE_DENSE: f32 = 0.6;
// Thin foam is a film of bubbles over the water: some of its light comes up through the water under it, so where it
// thins it takes the water's colour (metres of the water's absorption that light crosses), and a dense crest's own
// crevices darken it (the share of its brightness the billows' shading spans).
const THIN_FOAM_PATH: f32 = 1.4;
const THIN_FOAM_TINT: f32 = 0.55;
const RESIDUE_STREAKING: f32 = 0.55;
const RESIDUE_DIM: f32 = 0.7;
const DENSE_OPACITY: f32 = 0.97;
const THIN_OPACITY: f32 = 0.08;
const THIN_LOW: f32 = 0.16;
const THIN_HIGH: f32 = 0.3;
const CREASE_LIGHT: f32 = 0.62;
const CREASE_LOW: f32 = 0.3;
const CREASE_HIGH: f32 = 0.8;
const EMBOSS_GAIN: f32 = 1.5;
const EMBOSS_METRES: f32 = 0.3;
const FLECK_START: f32 = 1.5;
const FLECK_END: f32 = 4.0;
const FAR_START: f32 = 8.0;
const FAR_END: f32 = 20.0;
const WHITECAP_DEPTH: f32 = 3.0;
// Height of a breaking crest's white face, standing up out of the water: what a grazing pixel sees of an active whitecap.
const WHITECAP_FACE: f32 = 0.4;
const WHITECAP_LENGTH: f32 = 8.0;
const LINE_START: f32 = 20.0;
const LINE_END: f32 = 60.0;
const AREA_OPACITY: f32 = 0.26;
const GATHERED_LEAST: f32 = 0.3;
const GATHERED_MOST: f32 = 2.2;
const GATHERED_MEAN: f32 = 1.12;
const BUBBLE_START: f32 = 0.12;
const BUBBLE_FULL: f32 = 0.7;
const WHITECAP_BUBBLES: f32 = 0.2;
const BUBBLE_STRENGTH: f32 = 0.6;
const BUBBLE_ALBEDO: f32 = 0.42;
const BUBBLE_PATH: f32 = 2.6;
const AERATED_MATTE: f32 = 0.6;
const WAKE_BUBBLES: f32 = 0.6;
// The realistic wake's churned water: foam energy over which it goes from clear to its densest cover, and its edge.
const CHURN_START: f32 = 0.0;
const CHURN_FULL: f32 = 1.0;
const CHURN_EDGE: f32 = 0.3;
// The share of the churned water's own cover its dense white water fills: the rest shows the bubble-lit water between.
const CHURN_DENSE_COVER: f32 = 0.78;
// An old wake's streaks: the share of them the lace covers, and its opacity.
const CHURN_LACE_COVER: f32 = 0.62;
const CHURN_LACE_OPACITY: f32 = 0.55;
// The opacity churned water keeps where its energy has all but gone.
const CHURN_THIN: f32 = 0.4;
// Means of dense water's opacity and brightness over the equalised churn channel.
const DENSE_MEAN: f32 = 0.08 + (0.97 - 0.08) * (1.0 - 0.3 + (0.3 - 0.16) / 2.0);
const BILLOW_MEAN: f32 = CREASE_LIGHT + (1.0 - CREASE_LIGHT) * (1.0 - CREASE_HIGH + (CREASE_HIGH - CREASE_LOW) / 2.0);

// The foam texture's coordinates at grid point `xz`: laid in the wind's frame, `stretch` times longer along it.
fn foam_layout(xz: vec2<f32>, wind: f32, stretch: f32, tile: f32) -> vec2<f32> {
    let along = vec2<f32>(cos(wind), sin(wind));
    return vec2<f32>(dot(xz, along) / stretch, dot(xz, vec2<f32>(-along.y, along.x))) / tile;
}

// A step of `metres` toward the sun in the foam texture's coordinates.
fn sunward(sun: vec3<f32>, wind: f32, stretch: f32, metres: f32, tile: f32) -> vec2<f32> {
    let along = vec2<f32>(cos(wind), sin(wind));
    let offset = sun.xz * metres;
    return vec2<f32>(dot(offset, along) / stretch, dot(offset, vec2<f32>(-along.y, along.x))) / tile;
}

// Share of the foam texture's contrast that filtering has averaged away at `uv`.
fn foam_blur(uv: vec2<f32>) -> f32 {
    let across = length(dpdx(uv)) * FOAM_TEXELS;
    let down = length(dpdy(uv)) * FOAM_TEXELS;
    let minor = min(across, down);
    let major = max(across, down);
    let resolved = max(minor, major / FOAM_ANISOTROPY);
    let averaged = sqrt(max(minor, FOAM_BLUR_START) * major);
    return smoothstep(FOAM_BLUR_START, FOAM_BLUR_END, max(resolved, averaged));
}

struct FoamFootprint {
    // The pixel's extent across the view or a quarter of its length along it, whichever is larger (m).
    size: f32,
    along: f32,
    across: f32,
}

fn foam_footprint(xz: vec2<f32>) -> FoamFootprint {
    let across = length(dpdx(xz));
    let down = length(dpdy(xz));
    return FoamFootprint(max(min(across, down), max(across, down) / 4.0), max(across, down), min(across, down));
}

struct FoamPatterns {
    pattern: vec4<f32>,
    blur: f32,
    streaks: f32,
    streaks_blur: f32,
    lines: f32,
    gather: f32,
    coarse: vec4<f32>,
    coarse_blur: f32,
    churn_sunward: f32,
}

// The foam texture read at grid point `xz`: the lace layout, the old foam's streaked lace, the windrows' band mask and
// gathering patches, and the churn channel a step toward the sun, with each layout's blur.
fn foam_patterns(map: texture_2d<f32>, map_sampler: sampler, streak_mean: f32, xz: vec2<f32>, wind: f32, stretch: f32,
    sun: vec3<f32>) -> FoamPatterns {
    let uv = foam_layout(xz, wind, stretch, FOAM_TILE);
    let streak_uv = foam_layout(xz, wind, stretch * STREAK_STRETCH, FOAM_TILE * STREAK_SCALE);
    let along = vec2<f32>(cos(wind), sin(wind));
    let cell = xz / BAND_WARP_CELL;
    let bands_at = xz + along * ((value_noise(cell, 1u) - 0.5) * (2.0 * BAND_WARP_ALONG))
        + vec2<f32>(-along.y, along.x) * ((value_noise(cell, 2u) - 0.5) * (2.0 * BAND_WARP_ACROSS));
    let near_uv = foam_layout(bands_at, wind, stretch * BAND_STRETCH, FOAM_TILE * BAND_NEAR);
    let far_uv = foam_layout(bands_at, wind, stretch * BAND_STRETCH, FOAM_TILE * BAND_FAR);
    let near_bands = textureSample(map, map_sampler, near_uv);
    let far_bands = textureSample(map, map_sampler, far_uv);
    let coarse_blur = foam_blur(near_uv);
    let churn_uv = uv + sunward(sun, wind, stretch, EMBOSS_METRES, FOAM_TILE);
    // The lace's own finer lace, laid turned against it: where a pixel resolves it, it frays every threshold's edge at
    // centimetres (bubbles and filaments), which the texture alone draws only down to its texels.
    let detail_uv = foam_layout(xz, wind + DETAIL_TURN, stretch, FOAM_TILE / DETAIL_SCALE);
    let fray = (textureSample(map, map_sampler, detail_uv).r - 0.5) * DETAIL_GAIN * (1.0 - foam_blur(detail_uv));
    var lace = textureSample(map, map_sampler, uv);
    lace.r = clamp(lace.r + fray, 0.0, 1.0);
    return FoamPatterns(
        lace,
        foam_blur(uv),
        clamp(textureSample(map, map_sampler, streak_uv).r + fray, 0.0, 1.0),
        foam_blur(streak_uv),
        mix(near_bands.b, streak_mean, coarse_blur),
        mix((near_bands.g + far_bands.g) * 0.5, 0.5, coarse_blur),
        near_bands,
        coarse_blur,
        textureSample(map, map_sampler, churn_uv).a,
    );
}

// Opacity of foam covering a share `coverage` of the sea, thresholding the equalised `pattern`.
fn foam_opacity(coverage: f32, pattern: f32, blur: f32, edge: f32) -> f32 {
    let share = clamp(coverage, 0.0, 1.0);
    let threshold = mix(1.0 + edge, -edge, share);
    return mix(smoothstep(threshold - edge, threshold + edge, pattern), share, blur);
}

// Opacity of foam covering exactly a share `coverage` of the sea.
fn foam_share(coverage: f32, pattern: f32, blur: f32, edge: f32) -> f32 {
    let share = clamp(coverage, 0.0, 1.0);
    let threshold = 1.0 - share;
    let width = max(min(edge, min(share, threshold)), 1e-4);
    return mix(smoothstep(threshold - width, threshold + width, pattern), share, blur) * smoothstep(0.0, 1e-3, share);
}

fn swayed(amount: f32, pattern: vec4<f32>, blur: f32) -> f32 {
    let sway = mix(0.6, 1.4, pattern.g) * mix(0.55, 1.45, pattern.a);
    return amount * mix(sway, 1.0, blur);
}

// World-anchored fractal noise that tears foam's outline where the foam texture's own sway is filtered away (a grazing
// or distant pixel): value noise in octaves of TEAR_CELLS metres, turned against each other, each fading to its mean
// as the pixel grows to its size. 0–1, mean ½.
const TEAR_CELLS: vec3<f32> = vec3<f32>(9.0, 3.7, 1.5);
const TEAR_WEIGHTS: vec3<f32> = vec3<f32>(0.5, 0.32, 0.18);
// How strongly the tear scales the foam's amount: from nothing at its lowest to twice at its highest.
const TEAR_GAIN: f32 = 2.6;

fn foam_tear(xz: vec2<f32>, size: f32) -> f32 {
    let turned = vec2<f32>(xz.x * 0.8 - xz.y * 0.6, xz.x * 0.6 + xz.y * 0.8);
    let shown = vec3<f32>(1.0) - smoothstep(TEAR_CELLS * 0.25, TEAR_CELLS * 0.6, vec3<f32>(size));
    let octaves = vec3<f32>(
        value_noise(xz / TEAR_CELLS.x, 611u),
        value_noise(turned / TEAR_CELLS.y + 17.0, 612u),
        value_noise(xz.yx / TEAR_CELLS.z + 41.0, 613u),
    );
    return dot(mix(vec3<f32>(0.5), octaves, shown), TEAR_WEIGHTS) / dot(TEAR_WEIGHTS, vec3<f32>(1.0));
}

// The factor the tear scales an amount of foam by: none where it is lowest, about twice where highest.
fn torn_amount(tear: f32) -> f32 {
    return max(1.0 + (tear - 0.5) * TEAR_GAIN, 0.0);
}

// The factor the tear scales a cover of foam by: holes where it is low, half again where it is high.
fn torn_cover(tear: f32) -> f32 {
    return smoothstep(0.25, 0.6, tear) * 1.4;
}

fn whitecap_patch(amount: f32) -> f32 {
    return smoothstep(PATCH_GONE, PATCH_FULL, amount);
}

fn dense_opacity(churn: f32, blur: f32) -> f32 {
    return mix(mix(THIN_OPACITY, DENSE_OPACITY, smoothstep(THIN_LOW, THIN_HIGH, churn)), DENSE_MEAN, blur);
}

struct Whitecap {
    opacity: f32,
    // How much of it is dense core (for the billows).
    dense: f32,
    // How far the pixel has gone to a mean.
    averaged: f32,
}

// A whitecap from the wave field's foam at this pixel: near, a dense core on the breaking crest and sparse lace streaked
// along the wind behind it; coarser, the same drawn on the lace's coarse layout, never spread wider than the foam; far
// off, the wind's mean.
fn whitecap_foam(amount: f32, fresh: f32, soft: f32, share: f32, patterns: FoamPatterns, footprint: FoamFootprint, xz: vec2<f32>) -> Whitecap {
    // The share of a long pixel a whitecap lying on the water fills: its amounts are read undiluted by the rest and its
    // opacity scaled back. A breaking crest stands up, so a grazing pixel sees its face over a larger share.
    let covered = min(1.0, WHITECAP_DEPTH / max(footprint.along, 1e-3));
    let standing = max(covered, min(1.0, WHITECAP_FACE / max(footprint.across, 1e-3)));
    // Where the pixel averages the lace away, the whitecap keeps its own outline and takes the coarse layout's marbling
    // inside it.
    let to_mean = smoothstep(FLECK_START, FLECK_END, footprint.size);
    let to_coarse = patterns.blur;
    // There, too, the lace left behind thins out over the softened field's falloff (`soft`, the foam read over a few
    // metres) rather than at the field's last texel; where the lace resolves, its own threshold draws the edge.
    let in_patch = mix(amount, soft, PATCH_SOFT * to_coarse) / covered;
    let in_core = fresh / covered;
    let pattern = mix(patterns.pattern, patterns.coarse, to_coarse);
    let blur = mix(patterns.blur, patterns.coarse_blur, to_coarse);
    // The world's tear only where there is foam to tear (it takes no derivatives).
    var tear = 0.5;
    if max(in_patch, in_core) > 1e-4 {
        tear = mix(0.5, foam_tear(xz, footprint.size), to_coarse);
    }
    let core = smoothstep(CORE_START, CORE_FULL, swayed(in_core, pattern, blur) * torn_amount(tear));
    let patch_share = whitecap_patch(swayed(in_patch, pattern, blur) * torn_amount(tear));
    let torn = mix(pattern.r, pattern.a, CORE_TORN);
    let core_opacity = foam_share(core * CORE_COVER, torn, blur, CORE_EDGE)
        * mix(CORE_THIN, dense_opacity(pattern.a, blur), core * core);
    // The residue's lace leans toward the wind's streaks as the patch thins.
    let lace = mix(pattern.r, patterns.streaks, (1.0 - patch_share) * RESIDUE_STREAKING * (1.0 - to_coarse));
    let residue = foam_share(mix(RESIDUE_SPARSE, RESIDUE_COVER, patch_share) * sqrt(patch_share) * torn_cover(tear), lace,
        max(blur, mix(patterns.streaks_blur, blur, to_coarse)), RESIDUE_EDGE) * mix(RESIDUE_FAINT, RESIDUE_DENSE, patch_share);
    let drawn = max(core_opacity * standing, residue * covered);
    // Thinner than a pixel row yet many pixels long, a whitecap lying flat on the water can only draw a line.
    let rows = WHITECAP_DEPTH / max(footprint.along, 1e-4);
    let line_length = WHITECAP_LENGTH / max(footprint.across, 1e-4);
    let line = smoothstep(LINE_START, LINE_END, line_length) * (1.0 - smoothstep(0.5, 1.5, rows));
    let to_far = max(smoothstep(FAR_START, FAR_END, footprint.size), line);
    let far = share * AREA_OPACITY;
    return Whitecap(mix(drawn, far, to_far), core * (1.0 - to_mean), to_mean);
}

// Windrows: old foam the wind has gathered into lines along itself, covering `coverage` of the sea.
fn windrow_opacity(coverage: f32, lines: f32, lines_mean: f32, gather: f32, lace: f32, streaks: f32, blur: f32, jacobian: f32) -> f32 {
    let gathered = coverage * ((gather - 0.5) * (2.0 * WINDROW_GATHER) + 1.0) * lines / lines_mean;
    let density = clamp(1.0 / max(jacobian, 0.05), GATHERED_LEAST, GATHERED_MOST) / GATHERED_MEAN;
    return foam_share(min(gathered, WINDROW_PEAK), mix(lace, streaks, WINDROW_STREAKING), blur, WINDROW_EDGE) * density;
}

// Gale streaks (native only): the old foam the field drifts downwind and gathers into lines (resolve.wgsl), `old` of
// it at this pixel, drawn as the wind's streaked lace (`streaks`, its blur) over the share of the water it covers.
// Each streak is lace, not a ribbon: its cover thins and breaks along it (`tear`, laid long along the wind), its lace is
// the wind's streaks torn by the round lace, and it is thickest down its middle.
const STREAK_FROM: f32 = 0.08;
const STREAK_FULL: f32 = 0.6;
const STREAK_COVER: f32 = 0.7;
const STREAK_OPACITY: f32 = 0.85;
const STREAK_THIN: f32 = 0.3;
const STREAK_EDGE: f32 = 0.25;
const STREAK_LACE: f32 = 0.45;
// The share of its cover a streak keeps where it breaks.
const STREAK_BROKEN: f32 = 0.35;
// The tear's cells are this many times longer along the wind than across it.
const STREAK_TEAR_STRETCH: f32 = 6.0;

fn gale_streaks(old: f32, streaks: f32, lace: f32, blur: f32, tear: f32) -> f32 {
    let thick = smoothstep(STREAK_FROM, STREAK_FULL, old);
    let cover = min(thick * STREAK_COVER * mix(STREAK_BROKEN, 1.3, smoothstep(0.3, 0.7, tear)), 1.0);
    return foam_share(cover, mix(streaks, lace, STREAK_LACE), blur, STREAK_EDGE) * mix(STREAK_THIN, STREAK_OPACITY, thick);
}

// The colour of thin foam relative to dense: a film of bubbles lit partly through the water under it.
fn thin_foam(absorption: vec3<f32>) -> vec3<f32> {
    return mix(vec3<f32>(1.0), exp(absorption * -THIN_FOAM_PATH), THIN_FOAM_TINT);
}

// Colour of foam relative to its light: dense white water billows, its crevices shaded by its own bubbles; thinner foam
// dims where its lace thins and takes the water's colour (`absorption`).
fn billows(churn: f32, churn_sunward: f32, lace: f32, dense: f32, blur: f32, absorption: vec3<f32>) -> vec3<f32> {
    let shade = mix(CREASE_LIGHT, 1.0, smoothstep(CREASE_LOW, CREASE_HIGH, churn)) / BILLOW_MEAN;
    let billowed = max(shade + (churn - churn_sunward) * EMBOSS_GAIN, 0.0);
    let film = mix(RESIDUE_DIM, 1.0, lace) / ((1.0 + RESIDUE_DIM) / 2.0);
    let brightness = mix(1.0, mix(film, billowed, dense), 1.0 - blur);
    return mix(thin_foam(absorption), vec3<f32>(1.0), clamp(dense, 0.0, 1.0)) * brightness;
}

// Radiance of a diffuse white scatterer facing `normal`, lit by the sky about its normal and the sun, wrapped.
fn foam_radiance(sky: vec3<f32>, sun_radiance: vec3<f32>, normal: vec3<f32>, sun: vec3<f32>, lit: f32) -> vec3<f32> {
    let facing = max((dot(normal, sun) + FOAM_WRAP) / (1.0 + FOAM_WRAP), 0.0) * smoothstep(-0.1, 0.05, sun.y);
    return (sky + sun_radiance * facing * lit * (FOAM_SUN / PI)) * FOAM_ALBEDO;
}

// How aerated the water is from the whitecaps' spread bubbles.
fn aeration(bubbles: f32) -> f32 {
    return smoothstep(BUBBLE_START, BUBBLE_FULL, bubbles) * WHITECAP_BUBBLES;
}

// How aerated the water is, 0–1: the whitecaps' spread `bubbles`, or the wake's `wake_bubbles` where there is a wake.
fn aeration_with_wake(bubbles: f32, wake_bubbles: f32) -> f32 {
    return max(aeration(bubbles), clamp(wake_bubbles / WAKE_BUBBLES, 0.0, 1.0));
}

struct Churned {
    opacity: f32,
    dense: f32,
}

// The realistic wake's churned water from its sampler's foam `energy` and how `young` it is: behind the stern the same
// dense, billowing white water as a whitecap's core, over the cover the sampler's eddies and ramp give it
// (`churnedWater`); older, the streaks it leaves are lace, thinner and see-through.
fn churned_water(energy: f32, young: f32, mass: f32, pattern: vec4<f32>, blur: f32) -> Churned {
    let cover = smoothstep(CHURN_START, CHURN_FULL, energy);
    // Where the churned water thins toward its edge it is thinner foam too, not the same white cut shorter.
    let body = mix(CHURN_THIN, 1.0, smoothstep(0.2, 0.85, energy));
    // Its dense white water covers CHURN_DENSE_COVER of the lace; where it is one mass (`mass`), all of it, at full
    // opacity.
    let dense = foam_share(cover * mix(CHURN_DENSE_COVER, 1.0, mass), pattern.r, blur, CHURN_EDGE)
        * mix(dense_opacity(pattern.a, blur), DENSE_OPACITY, mass) * body;
    let lace = foam_share(cover * CHURN_LACE_COVER, pattern.r, blur, CHURN_EDGE) * CHURN_LACE_OPACITY * body;
    return Churned(mix(lace, dense, young), cover * young);
}

fn aerated_reflectance(reflectance: f32, aerated: f32) -> f32 {
    return reflectance * (1.0 - aerated * AERATED_MATTE);
}

// The water body brightened by the bubble clouds in it: pale turquoise.
fn bubble_cloud(body: vec3<f32>, aerated: f32, light: vec3<f32>, absorption: vec3<f32>) -> vec3<f32> {
    let cloud = light * exp(absorption * -BUBBLE_PATH) * (BUBBLE_ALBEDO / FOAM_ALBEDO);
    return mix(body, max(body, cloud), aerated * BUBBLE_STRENGTH);
}

// Bubble walls (native, 2026-10-14). The foam texture's texels are 16 cm on the water (its 40 m tile over 256 texels),
// its finer copy's 3 cm, and both are the same lace scaled: near the eye white water read as soft grey blots. Close up,
// foam is a network of bubble walls, bright filaments about holes from a few centimetres across to tens. Two octaves of
// cellular noise laid on the water (a point's distance to the edge between its two nearest jittered feature points,
// F2 − F1, in cells) draw it where a pixel resolves their cells: 1 on a wall, falling to 0 `BUBBLE_WALL` of a cell from
// it, mean ½ (standard deviation 0.37), and ½ where the pixel is more than about half a cell. Added to the lace before
// any threshold reads it (`BUBBLE_GAIN`), it moves every foam's edge onto the walls. The lace's mean is kept, but the
// thresholds after it are not linear: a soft, thin patch of foam opens holes and loses a little of its cover.
const BUBBLE_CELLS: vec2<f32> = vec2<f32>(1.0, 0.25);
const BUBBLE_WEIGHTS: vec2<f32> = vec2<f32>(0.65, 0.35);
const BUBBLE_WALL: f32 = 0.5;
const BUBBLE_GAIN: f32 = 0.8;

fn cell_walls(p: vec2<f32>, seed: u32, blur: f32) -> f32 {
    let mixed = noise_seed(seed);
    let cell = floor(p);
    let f = p - cell;
    var first = 9.0;
    var second = 9.0;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let x = i32(cell.x) + dx;
            let y = i32(cell.y) + dy;
            let point = vec2<f32>(f32(dx) + noise_hash(x, y, mixed), f32(dy) + noise_hash(y, x, mixed ^ 0x5bd1e995u));
            let d = distance(point, f);
            if d < first {
                second = first;
                first = d;
            } else if d < second {
                second = d;
            }
        }
    }
    // A wall narrower than the pixel is drawn as its mean over the pixel, not a shimmering line.
    let wall = BUBBLE_WALL + blur;
    return (1.0 - smoothstep(0.0, wall, second - first)) * (BUBBLE_WALL / wall) + 0.5 * (1.0 - BUBBLE_WALL / wall);
}

// The bubble walls at grid point `xz` for a pixel `size` metres across: 0–1, mean ½ (no derivatives taken).
fn bubble_walls(xz: vec2<f32>, size: f32) -> f32 {
    let cells = vec2<f32>(size) / BUBBLE_CELLS;
    let shown = vec2<f32>(1.0) - smoothstep(vec2<f32>(0.25), vec2<f32>(0.6), cells);
    var walls = vec2<f32>(0.5);
    if shown.x > 0.0 {
        walls.x = cell_walls(xz / BUBBLE_CELLS.x, 721u, cells.x);
    }
    if shown.y > 0.0 {
        let turned = vec2<f32>(xz.x * 0.8 - xz.y * 0.6, xz.x * 0.6 + xz.y * 0.8);
        walls.y = cell_walls(turned / BUBBLE_CELLS.y + 13.0, 722u, cells.y);
    }
    return dot(mix(vec2<f32>(0.5), walls, shown), BUBBLE_WEIGHTS);
}

// The lace `lace` (0–1) with the bubble walls' share of it.
fn with_bubble_walls(lace: f32, walls: f32) -> f32 {
    return clamp(lace + (walls - 0.5) * BUBBLE_GAIN, 0.0, 1.0);
}

// Sparkle on foam (native, 2026-10-14). Every bubble of fresh foam is a tiny curved mirror, and some turn the sun to the
// eye from any direction: a field of sun points over sunlit white water, brightest looking toward the sun, where the
// light meets the bubbles' faces at a grazing angle (Fresnel at half the angle between the sun and the eye). Drawn on
// cells about a pixel across laid on the water (the two power-of-two sizes about it, blended), each lit for a twinkle at
// `SPARKLE_CHANCE`, on the waves' clock. Returns how lit the cell at `xz` is, 0–1.
const SPARKLE_CHANCE: f32 = 0.02;
const SPARKLE_TWINKLE: f32 = 5.0;
const SPARKLE_SHIFT: f32 = 0.37;

fn sparkle_cell(xz: vec2<f32>, size: f32, time: f32) -> f32 {
    let cell = floor(xz / size);
    let x = i32(cell.x);
    let y = i32(cell.y);
    let clock = time * SPARKLE_TWINKLE + noise_hash(x, y, noise_seed(731u));
    let slot = floor(clock);
    let lit = step(noise_hash(x, y, noise_seed(732u + u32(i32(slot) & 0xffff) * 977u)), SPARKLE_CHANCE);
    return lit * sin(3.141592653589793 * (clock - slot));
}

fn foam_sparkle(xz: vec2<f32>, size: f32, time: f32) -> f32 {
    let level = log2(max(size, 1e-3));
    let lower = floor(level);
    let cell = exp2(lower);
    // Each size's cells keep their place (shifted by the same share of their own size) as the pixel grows past it.
    let finer = sparkle_cell(xz + SPARKLE_SHIFT * cell, cell, time);
    let coarser = sparkle_cell(xz + SPARKLE_SHIFT * 2.0 * cell, cell * 2.0, time);
    return mix(finer, coarser, level - lower);
}

// `sea` with foam of radiance `foam` over `opacity` of it: its wet top keeps half the water's reflection, and it is
// never darker than the water it covers without glints.
fn foam_over(sea: vec3<f32>, water: vec3<f32>, foam: vec3<f32>, mirrored: vec3<f32>, reflectance: f32, opacity: f32) -> vec3<f32> {
    return mix(sea, max(mix(foam, mirrored, reflectance * FOAM_GLOSS), water), clamp(opacity, 0.0, 1.0));
}

// ---- The foam texture hex-tiled ------------------------------------------------------------------------------------
// A wake's foam reads the foam texture through three copies of it laid on a grid of triangles HEX_CELL
// tiles across, each copy shifted at random, blended toward the copy nearest its own corner and toward the one with
// the most lace (HEX_SHARPNESS, HEX_LACE): its 40 m tile never shows as a repeat. Hex-tiling after Mikkelsen 2022
// ("Practical Real-Time Hex-Tiling", JCGT 11(2)), written here from its description.
const HEX_CELL: f32 = 0.45;
const HEX_SHARPNESS: f32 = 7.0;
const HEX_LACE: f32 = 0.6;

// The lace's layout at grid point `xz` (`foam_patterns`' own), in tiles.
fn lace_layout(xz: vec2<f32>, wind: f32, stretch: f32) -> vec2<f32> {
    return foam_layout(xz, wind, stretch, FOAM_TILE);
}

fn hex_shift(corner: vec2<i32>) -> vec2<f32> {
    let mixed = noise_seed(907u);
    return vec2<f32>(noise_hash(corner.x, corner.y, mixed), noise_hash(corner.y, corner.x, mixed ^ 0x68e31da4u));
}

// The foam texture at `uv` (tiles) hex-tiled; `ddx`, `ddy` its derivatives, taken by the caller in uniform control flow.
fn hex_foam(map: texture_2d<f32>, map_sampler: sampler, uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>) -> vec4<f32> {
    // The plane skewed onto the lattice of equilateral triangles, and the triangle's corners and barycentric weights.
    let p = uv / HEX_CELL;
    let skewed = vec2<f32>(p.x - p.y * 0.57735027, p.y * 1.15470054);
    let base = vec2<i32>(floor(skewed));
    let f = fract(skewed);
    let upper = f.x + f.y >= 1.0;
    let first = base + select(vec2<i32>(0), vec2<i32>(1), upper);
    let second = base + select(vec2<i32>(0, 1), vec2<i32>(1, 0), upper);
    let third = base + select(vec2<i32>(1, 0), vec2<i32>(0, 1), upper);
    let weights = select(vec3<f32>(1.0 - f.x - f.y, f.y, f.x), vec3<f32>(f.x + f.y - 1.0, 1.0 - f.y, 1.0 - f.x), upper);
    let a = textureSampleGrad(map, map_sampler, uv + hex_shift(first), ddx, ddy);
    let b = textureSampleGrad(map, map_sampler, uv + hex_shift(second), ddx, ddy);
    let c = textureSampleGrad(map, map_sampler, uv + hex_shift(third), ddx, ddy);
    var w = pow(max(weights, vec3<f32>(0.0)), vec3<f32>(HEX_SHARPNESS)) * mix(vec3<f32>(1.0), vec3<f32>(a.r, b.r, c.r) + 0.1, HEX_LACE);
    w /= max(w.x + w.y + w.z, 1e-6);
    return a * w.x + b * w.y + c * w.z;
}
