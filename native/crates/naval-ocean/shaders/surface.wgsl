// The ocean surface: ports the web game's `ocean/surface/OceanGeometry.ts` (the clipmap's snapping and geomorph in the vertex
// stage), the `displacement` and `surface` reads of the web game's `ocean/waves/field.ts` with its ripples, `gustGain` of
// the web game's `ocean/waves/gusts.ts`, `breakingGroup` of `ocean/waves/groups.ts`, the physical shading graph of
// the web game's `ocean/surface/OceanSurfaceMaterial.ts` (`realism.reflections` and `realism.waterColor` on, as the game
// defaults) and the scene fog of the web game's `ocean/screen/fog.ts`.
//
// The wakes (the web's `WakeSampler` with the realistic trail: wake field, bow waves, trail foam, bubbles and slick,
// torpedo tracks, hull contact foam, and the shelter hull sides give the water beside them, surface/shelter.ts) come from
// naval_ocean::wake (shaders/wake.wgsl), whose bindings join group 1. Ship shadows on the sea are the renderer core's
// (naval::shadow's `water_sun_shadow`, the web game's `WaterShadows.ts`), the clouds' shadow naval_clouds::shadow's; the two
// multiply into the web's `shadow` node. Hulls and land the mirrored ray meets on screen replace the sky
// (naval_ocean::screen_reflection, the web game's `ocean/screen/reflections.ts`) on the tiers that trace them, its table of the
// depth copy's tiles at binding 15. The wave field's
// reads and the sea around hulls (the web game's `ocean/waves/hullSea.ts`) are naval_ocean::waves (group 2). A leaking
// hull's oil slick (native only) is naval_ocean::oil, its atlas at binding 16, read in the OIL variant alone.

// Bind groups follow the renderer core's scene pipelines (native/docs/render-core/README.md): 0 the camera
// (naval::camera) with the sun's shadow maps and the motion block, 1 the surface's own, 2 the wave field's, 3 the sky
// environment.

#import naval::camera::{camera, world_to_clip, curved}
#import naval::shadow::water_sun_shadow
#import naval::motion::motion_response
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#import naval_ocean::noise::{value_noise, gradient_noise}
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance}
#import naval_ocean::reflection::{TRACE_BELOW, footprint_variance, wind_variances, reflection_lobe, mean_fresnel, sky_lobe, sun_glitter, glitter_density, glitter_glints, lamp_glints, soft_knee, luminance_ceiling}
#import naval_ocean::screen_reflection::screen_space_reflection
#import naval_ocean::water_body::{BACKSCATTER, REFRACTION_SQUARED, subsurface_reflectance, upwelling, column_transmittance, water_body}
#import naval_ocean::water_properties::{WaterField, sample_water, water_beam, water_pigment}
#import naval_ocean::foam::{FOAM_GLOSS, STREAK_TEAR_STRETCH, bubble_walls, with_bubble_walls, foam_sparkle, foam_patterns, foam_footprint, foam_tear, torn_cover, foam_share, whitecap_foam, windrow_opacity, billows, thin_foam, foam_radiance, aeration_with_wake, aerated_reflectance, bubble_cloud, foam_over, churned_water, gale_streaks}
#import naval_ocean::wake::{wake_height, trail_relief, wake_surface, hull_shelter, shelter_light, shelter_reflect, wake_sampler}
#import naval_ocean::waves::{waves, field_displacement, field_derivatives, field_extras, field_sampler, field_uv, scatter, copies_grad, mix_zero, mix_mean, mix_variance, displacement, HullSurface, hull_surface, hull_sea_active, breaking_group, Shore, shore_at, shore_whitecaps, surf}
#import naval_ocean::underwater_ray::water_path_light
#import naval::aerial::aerial_apply
#import naval::lights::{LIGHT_WORDS, scene_lights, light_cell, light_word}
#import naval_ocean::point_lights::{LightShares, SeaLights, light_frame, light_dither, sea_point_light}
#import naval_ocean::rain::rain_surface
#import naval_ocean::spray_haze::{spray_depth, spray_light}
#import naval_ocean::oil::{Oil, OilFilm, OIL_WHITECAPS, oil_read, oil_film, oil_own}

struct Surface {
    // Per clipmap level: xy snapped centre, z cell (m), w 1 / cell (`geometry::MAX_CLIPMAP_LEVELS`).
    frames: array<vec4<f32>, 10>,
    // x: segments per level edge, y: levels, zw: the clipmap's centre (x, z): the eye, or what a magnified view is
    // aimed at (`geometry::focus`).
    clipmap: vec4<f32>,
    // x: the finest level drawn, y: how far it has morphed onto the next (`geometry::Clipmap::detail`); a magnified
    // view's fan (`geometry::Fan`): z its first row, counted from its knee, w its squeeze (what one over a row's range
    // falls by each row beyond the knee, 1 / m).
    detail: vec4<f32>,
    // The fan: x its first column's bearing (radians from +z toward +x), y its step (radians between columns, and the
    // log of a row's range over the one before within the knee), z its knee's range and w its last row's (m).
    fan: vec4<f32>,
    absorption: vec4<f32>,
    transmission: vec4<f32>,
    // rgb colour, a opacity.
    crest_foam: vec4<f32>,
    surface_foam: vec4<f32>,
    shoreline_foam: vec4<f32>,
    // x: crest foam's wind stretch, y: windrow coverage, z: the band mask's mean.
    foam: vec4<f32>,
    // rgb: fog colour, a: power.
    fog_color: vec4<f32>,
    // x: start, y: end, z: sky blend distance, w: 1 with fog.
    fog: vec4<f32>,
    // x: water shadows (the Graphics menu's 0 off, 1 low, 2 medium, 3 high), yzw unused.
    shadows: vec4<f32>,
    // Screen-space reflections: x 1 while traced (Graphics → Reflections `scene`), y the longest ray (m).
    reflections: vec4<f32>,
    // rgb the water's pigment (the underside's deep), w 1 while the underwater probe is this frame's.
    pigment: vec4<f32>,
    // x: rain (the weather's precipitation, 0 dry … 1 a downpour), yzw: the gale's spray haze (naval_ocean::spray_haze:
    // extinction at its base, scale height, base height).
    weather: vec4<f32>,
    water: WaterField,
}

@group(1) @binding(0) var<uniform> surface: Surface;
@group(1) @binding(1) var foam_map: texture_2d<f32>;
@group(1) @binding(2) var foam_sampler: sampler;
// The opaque scene behind the sea (colour and reversed depth), for straight-through transmission and shoreline foam.
@group(1) @binding(3) var scene_color: texture_2d<f32>;
@group(1) @binding(4) var scene_depth: texture_depth_2d;
// The clouds' shadow (naval_clouds::CloudShadow): the map, its linear repeat sampler and its uniforms (disabled: 1).
@group(1) @binding(11) var cloud_map: texture_2d<f32>;
@group(1) @binding(12) var cloud_sampler: sampler;
@group(1) @binding(13) var<uniform> cloud_params: CloudShadow;
// The underwater probe (naval_ocean::underwater): g at texel (0, 0) is the camera's depth below the waves.
@group(1) @binding(14) var underwater_probe: texture_2d<f32>;

@group(3) @binding(0) var sky_atlas: texture_2d<f32>;
@group(3) @binding(1) var sky_sampler: sampler;
@group(3) @binding(2) var<uniform> sky_env: SkyEnvironment;

const CASCADES: i32 = #CASCADES;
// Screen-space reflection steps along the clipped ray (the tier's `reflectionSteps`; 0 traces nothing).
const REFLECTION_STEPS: i32 = #REFLECTION_STEPS;
const ANISOTROPY: f32 = 16.0;
// A finer cascade's foam shows where the coarser cascades' compression, in its own deviations, passes these.
const GATE_NONE: f32 = -1.0;
const GATE_FULL: f32 = 0.75;
const FOAM_JACOBIAN_MIN: f32 = 0.35;
const FOAM_JACOBIAN_MAX: f32 = 2.5;
// The bubble cloud is read over an ellipse this many metres along the crests and across them.
const CREST_SPREAD: f32 = 4.0;
const CROSS_SPREAD: f32 = 1.5;
const MIN_JACOBIAN: f32 = 0.1;
// The ripples' slopes over the finest cascade's they are read from: the waves shorter than it draw that much more of the
// tail Cox and Munk measured (`waves.tail`), which then roughens the surface that much less.
const RIPPLE_GAIN: f32 = 1.7;
const SETTLE_START: f32 = 0.4;
const SETTLE_FULL: f32 = 0.1;
// Gusts: three octaves (cell along, across, weight, share of the wind's speed it drifts at) and the sum's deviation
// (gradient noise's 0.2156 times √GUST_TOTAL_WEIGHT).
const GUST_SUM_DEVIATION: f32 = 0.2779890;
const GUST_REACH: f32 = 2.5;
const GUST_HOLD_LOSS: f32 = 0.27;
const GUST_TOTAL_WEIGHT: f32 = 1.6625;
const GUST_FADE_START: f32 = 0.12;
const GUST_FADE_END: f32 = 0.35;
// Water shading.
const GLITTER_KNEE: f32 = 0.25;
// The brightest one glint's pixel may be, in the same units: eight times the knee the glitter's mean takes (was four
// until 2026-10-18, the Waterline look: a glint that stops near white reads as grey salt; sixteen bloomed each glint into
// a round halo).
const GLINT_PEAK: f32 = 2.0;
// Sparkle on sunlit white water (`foam_sparkle`): a lit cell's radiance over the sun's irradiance and the bubbles'
// Fresnel at half the angle between the sun and the eye.
const SPARKLE_GAIN: f32 = 12.0;
// A lamp is marched toward (`light_seen`) only where it adds more than the larger of these to a pixel (luminance): the
// least radiance that shows on the darkest sea (under 3 of 255 levels with the eye at its most open at night), and a
// share of the light the pixel has without the lamps, under which its shadow would not show.
const LAMP_SEEN_LEAST: f32 = 1e-4;
const LAMP_SEEN_SHARE: f32 = 0.03;
// The most lights a pixel marches toward (`light_seen`); the rest light it unhidden. Each march is 16 reads of the depth:
// a busy night has dozens of lights, of which the brightest few at the eye (first in the list) are the ones whose
// shadows show.
const LAMP_MARCHES: u32 = 4u;
// `CLOUD_SHADOW_FLOOR`: the beam a cloud's shadow keeps (none: the light under a cloud is the sky's).
const CLOUD_SHADOW_FLOOR: f32 = 0.0;
const SHORE_DEPTH: f32 = 0.8;
// The gale streaks show in patches this long along the wind and wide across it (m).
const STREAK_PATCH_ALONG: f32 = 170.0;
const STREAK_PATCH_ACROSS: f32 = 55.0;
const SHORE_EDGE: f32 = 0.3;
// The surf's lace edge, as the shoreline's.
const SURF_EDGE: f32 = 0.3;
// The share of the water the foam covers at the contact, and how much finer than the whitecaps' its tear is laid.
const SHORE_COVER: f32 = 0.4;
const SHORE_TEAR_SCALE: f32 = 4.0;
// How much of the shoreline foam a hull's side takes away (its contact foam is the hull's).
const SHORE_HULL: f32 = 0.85;

struct VertexIn {
    // Lattice position (x, z) in the level's frame, metres at unit cell.
    @location(0) position: vec2<f32>,
    // x: clipmap level (levels for the horizon ring), y: the horizon ring's vertex spacing (m). The fan's block
    // (`geometry::build_fan`) has level -1 and its (column, row) for position, a block of rows to an instance.
    @location(1) level: vec2<f32>,
    @builtin(instance_index) instance: u32,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // Undisplaced world xz: where the waves are read.
    @location(0) grid: vec2<f32>,
    // Displaced world position.
    @location(1) world: vec3<f32>,
}

// GLSL's mod, as three's TSL `mod` compiles: the result takes the divisor's sign.
fn glsl_mod(x: vec2<f32>, y: f32) -> vec2<f32> {
    return x - y * floor(x / y);
}

// The skirt's rows (`naval_ocean::geometry::SKIRT_ROWS`), and the share of the far plane water beyond it is drawn at.
const SKIRT_ROWS: f32 = 6.0;
const BEYOND_DEPTH: f32 = 0.995;
// Metres of water under the surface where nothing stands behind it.
const OPEN_DEEP: f32 = 1.0e4;
// A crest this high (m) past the sea's horizon still shows over it: the water is drawn that far beyond the horizon.
const CREST_BEYOND: f32 = 20.0;
// Rows of cells in the fan's block (`naval_ocean::geometry::FAN_BLOCK_ROWS`).
const FAN_BLOCK_ROWS: f32 = 32.0;

@vertex
fn vertex(input: VertexIn) -> VertexOut {
    let levels = i32(surface.clipmap.y);
    let segments = surface.clipmap.x;
    let fan = input.level.x < 0.0;
    let level = i32(clamp(input.level.x, 0.0, f32(levels - 1)));
    let horizon = input.level.x > f32(levels) - 0.5;
    let frame = surface.frames[level];
    let finer = surface.frames[max(level - 1, 0)];
    let cell = frame.z;
    // Multiplying by the exact reciprocal keeps the lattice arithmetic exact.
    let lattice = input.position * frame.w;
    // A ring's inner edge sits on the finer level's outer edge, wherever that level snapped.
    let inner = max(abs(lattice.x), abs(lattice.y)) <= segments / 4.0 && level > 0 && !horizon;
    let index = lattice + select(vec2<f32>(0.0), (finer.xy - frame.xy) * frame.w, inner);
    let snapped = frame.xy + index * cell;
    // Geomorph by true distance from the centre; the outer row is always fully morphed. The finest level drawn also
    // morphs as a whole as the view draws back, until it is the middle of the next and that takes over.
    let reach = max(abs(snapped.x - surface.clipmap.z), abs(snapped.y - surface.clipmap.w)) / (cell * (segments / 2.0));
    let receding = select(0.0, surface.detail.y, level == i32(surface.detail.x));
    let morph = select(max(smoothstep(0.6, 0.85, reach), receding), 0.0, horizon);
    var grid = snapped - glsl_mod(index, 2.0) * morph * cell;
    // The skirt beyond the ring (`geometry.rs`): from an eye high enough to see past the ring, its rows run on from the
    // ring to the sea's horizon, where the Earth's drop equals the eye's height (`camera::curved`), each a share of
    // the way in equal ratios. Lower, the horizon lies within the ring and the skirt stays on the ring's circle.
    let skirt = input.level.x - f32(levels);
    let across = grid - camera.position.xz;
    let range = length(across);
    if skirt > 0.5 && camera.curvature > 0.0 {
        let sea_horizon = sqrt(max(camera.position.y, 0.0) / camera.curvature);
        if sea_horizon > range && range > 1.0 {
            // The first row stands where the water reaches the far plane, so the water within it keeps its own depth
            // (a hull out there stands in it as she does nearer); the rest run on to the horizon in equal ratios.
            let edge = BEYOND_DEPTH * camera.far;
            let first = clamp(sqrt(max(edge * edge - camera.position.y * camera.position.y, 0.0)), range, sea_horizon);
            let reach = first * pow(sea_horizon / first, (skirt - 1.0) / (SKIRT_ROWS - 1.0));
            grid = camera.position.xz + across * (reach / range);
        }
    }
    // Water past the sea's horizon is hidden by the water before it: the ring's rows beyond (and the skirt's left on the
    // ring's circle) close up where a crest past the horizon would still show, so a view whose far plane lies hundreds of
    // kilometres off (the land beyond a theatre window) shades no water only to lose it to the depth test. A closed-up
    // vertex takes the spacing the ring has there (`geometry.rs`: 4 r / segments), the same for every row closed onto
    // it, so their waves are read alike and the triangles between have no area.
    var spacing = select(cell * (morph + 1.0), input.level.y, horizon);
    if horizon && camera.curvature > 0.0 {
        let seen = sqrt(max(camera.position.y, 0.0) / camera.curvature) + sqrt(CREST_BEYOND / camera.curvature);
        let out = grid - camera.position.xz;
        let reach = length(out);
        if reach > seen {
            grid = camera.position.xz + out * (seen / reach);
            spacing = 4.0 * seen / segments;
        }
    }
    if fan {
        // A magnified view's mesh (`geometry::fan`): a polar lattice about the eye, columns a step of bearing apart.
        // Within its knee each row is e^step times the range of the one before, so a cell is square and the step
        // times its range across; beyond, one over a row's range falls by the squeeze each row, so the rows stand
        // evenly down the screen. The waves are read at the longer of a cell's sides, as a level's are at its cell.
        // The last row stands where the fan ends, as do the rows a draw's last block holds beyond it.
        let row = surface.detail.z + f32(input.instance) * FAN_BLOCK_ROWS + input.position.y;
        let bearing = surface.fan.x + input.position.x * surface.fan.y;
        var reach = surface.fan.z * exp(min(row, 0.0) * surface.fan.y);
        if row > 0.0 {
            reach = 1.0 / max(1.0 / surface.fan.z - surface.detail.w * row, 1e-9);
        }
        reach = min(reach, surface.fan.w);
        grid = camera.position.xz + vec2<f32>(sin(bearing), cos(bearing)) * reach;
        spacing = reach * max(surface.fan.y, surface.detail.w * reach);
    }
    let offset = displacement(grid, spacing);
    let world = vec3<f32>(grid.x + offset.x, offset.y + wake_height(grid) + trail_relief(grid, spacing), grid.y + offset.z);
    var output: VertexOut;
    output.clip = world_to_clip(world);
    if (horizon || fan) && camera.curvature > 0.0 {
        // Water beyond the far plane (the skirt's, and the ring's own far side when the clipmap is centred ahead of
        // the eye) is drawn along its own line of sight just inside it, each skirt row behind the one before: it
        // covers the pixels it would, behind what the far plane holds but for the last ring step before it.
        let line = curved(world) - camera.position;
        let limit = camera.far * (BEYOND_DEPTH + (0.9995 - BEYOND_DEPTH) * max(skirt, 0.0) / SKIRT_ROWS);
        if length(line) > limit {
            output.clip = camera.view_projection * vec4<f32>(camera.position + line * (limit / length(line)), 1.0);
        }
    }
    output.grid = grid;
    output.world = world;
    return output;
}

// The gain on the slope variance of waves shorter than the gust wavelength at grid point `xz`, for a pixel whose
// footprint reaches `length` metres along the view: lognormal, mean 1. The patches ride the wind (Taylor's frozen
// turbulence: the larger eddies reach higher, faster air), each octave's lattice turned off the wind by its own angle
// so no two share a grid.
fn gust_gain(xz: vec2<f32>, length: f32) -> f32 {
    var octaves = array<vec4<f32>, 3>(
        vec4<f32>(4200.0, 2600.0, 0.6, 1.15),
        vec4<f32>(1200.0, 750.0, 1.0, 1.0),
        vec4<f32>(400.0, 250.0, 0.55, 0.9),
    );
    var turns = array<vec2<f32>, 3>(
        vec2<f32>(0.978031, 0.208460),
        vec2<f32>(0.987227, -0.159318),
        vec2<f32>(0.995953, 0.089879),
    );
    let wind = waves.wind.xy;
    let size = waves.gusts.z;
    let travel = waves.wind.z * waves.gusts.y * waves.gusts.w;
    var gust = 0.0;
    var variance = 0.0;
    for (var i = 0; i < 3; i++) {
        let octave = octaves[i];
        let turn = turns[i];
        let along = vec2<f32>(wind.x * turn.x - wind.y * turn.y, wind.x * turn.y + wind.y * turn.x);
        let carried = xz - wind * (travel * octave.w);
        let width = size * octave.y;
        let shown = (1.0 - smoothstep(width * GUST_FADE_START, width * GUST_FADE_END, length)) * octave.z;
        let cell = vec2<f32>(dot(carried, along) / (size * octave.x), dot(carried, vec2<f32>(-along.y, along.x)) / width);
        gust += gradient_noise(cell, 301u + u32(i)) * shown;
        variance += shown * shown;
    }
    // Held softly within GUST_REACH deviations: gradient noise's rare peaks (3.3 deviations an octave, where value noise
    // stopped at 2.3) made patches rough enough to catch the sun far off in a light air.
    gust = GUST_REACH * tanh(gust / (GUST_SUM_DEVIATION * GUST_REACH));
    variance /= GUST_TOTAL_WEIGHT;
    // The held sum's variance (v / (1 + GUST_HOLD_LOSS·v) of the unheld v, within 1 % to v = 1), so the gain's mean stays
    // 1: within 0.1 % at the wind's own strengths, 2 % at a strength of 1 (Codex review).
    let held = variance / (1.0 + GUST_HOLD_LOSS * variance);
    let strength = waves.gusts.x;
    return exp(gust * strength - held * strength * strength * 0.5);
}

struct WaveSample {
    // World-space slope of the displaced surface (∂y/∂x, ∂y/∂z), mip-filtered.
    slope: vec2<f32>,
    jacobian: f32,
    foam: f32,
    fresh: f32,
    // The foam read over the bubble cloud's ellipse: the whitecaps' outline, softened.
    foam_soft: f32,
    whitecap_share: f32,
    bubbles: f32,
    slope_variance: f32,
    unresolved_variance: f32,
    // The surf's cover and its fresh share (`waves::surf`).
    breakers: vec2<f32>,
    // The world slope and the Jacobian of the waves longer than the finest cascade's alone: their faces and crests, smooth
    // under the chop (the light through the waves, `wave_light`).
    swell_slope: vec2<f32>,
    swell_jacobian: f32,
}

// The waves at grid point `xz` (`surface()` in field.ts), a wake's slick stilling `calm` of the short waves; near hulls the
// first cascade's long waves give way to the sea they ride, evaluated where the point is drawn (`at`); near a coast what
// its fetch and depth leave of each cascade (`waves::shore_at`), and its surf. Takes screen-space derivatives.
fn wave_surface(xz: vec2<f32>, calm: f32, at: vec2<f32>) -> WaveSample {
    // The pixel's footprint on the grid (m), as the anisotropic filter resolves it.
    let ddx = dpdx(xz);
    let ddy = dpdy(xz);
    let across = length(ddx);
    let down = length(ddy);
    let footprint = max(min(across, down), max(across, down) / ANISOTROPY);
    let wide = max(across, down);
    let wind = waves.wind.xy;
    let crestward = vec2<f32>(-wind.y, wind.x);
    let grazing = smoothstep(ANISOTROPY / 2.0, ANISOTROPY, wide / max(min(across, down), 1e-6));
    let gain = gust_gain(xz, wide);
    let top = waves.info.x;
    let shore = shore_at(xz);
    // The hull sea's share (`hullSurface`), read only while a hull is coupled (a uniform branch: explicit levels).
    var hulls = HullSurface(vec2<f32>(0.0), vec3<f32>(0.0), vec2<f32>(0.0), 0.0);
    if hull_sea_active() {
        hulls = hull_surface(xz, at, footprint, shore);
    }
    var slope = vec2<f32>(0.0);
    var strain = vec3<f32>(0.0);
    var swell_slope = vec2<f32>(0.0);
    var swell_strain = vec3<f32>(0.0);
    var foam = 0.0;
    var fresh = 0.0;
    var foam_soft = 0.0;
    var bubbles = 0.0;
    let still = 1.0 - calm;
    var variance = waves.tail.x * still * gain;
    var unresolved = waves.tail.y * still * gain;
    for (var i = 0; i < CASCADES; i++) {
        let tile = waves.tile[i];
        let copies = scatter(xz, i);
        // A cascade finer than its coarsest texel under this pixel fades out over the last level.
        let level = log2(footprint * waves.texels[i]);
        let detail = 1.0 - smoothstep(top - 1.0, top, level);
        let read_derivatives = copies_grad(field_derivatives, copies, i, ddx / tile, ddy / tile);
        let read_extras = copies_grad(field_extras, copies, i, ddx / tile, ddy / tile);
        // A coast's share of the cascade's amplitude: its slopes and strain scale with it, its variance and the foam its
        // breaking leaves with its square.
        let kept = shore.scales[i];
        let derivatives = mix_zero(read_derivatives, copies) * kept;
        let means = mix_mean(read_extras, copies) * vec4<f32>(1.0, kept * kept, 1.0, kept * kept);
        let extras = vec4<f32>(mix_zero(read_extras, copies).x * kept, means.y, means.z, means.w);
        // A finer cascade's foam shows on the crests of the coarser ones summed so far.
        let crests = select(1.0, smoothstep(GATE_NONE, GATE_FULL, -(strain.x + strain.y) * waves.gates[i]), i > 0);
        let shown = crests * breaking_group(xz, extras.y, extras.w, i) * detail;
        foam = max(foam, extras.y * shown);
        fresh = max(fresh, extras.w * shown);
        // The bubble cloud under the crests, read drawn out along them.
        let spread = mix_mean(copies_grad(field_extras, copies, i, crestward * max(wide, CREST_SPREAD) / tile,
            wind * max(wide, CROSS_SPREAD) / tile), copies) * (kept * kept);
        let spread_shown = crests * breaking_group(xz, spread.y, spread.w, i) * detail;
        bubbles = max(bubbles, spread.w * spread_shown);
        foam_soft = max(foam_soft, spread.y * spread_shown);
        let roughened = 1.0 + (gain - 1.0) * waves.gust_shares[i];
        let stilled = 1.0 - calm * waves.slick_shares[i];
        slope += derivatives.xy * detail * stilled * sqrt(roughened);
        strain += vec3<f32>(derivatives.zw, extras.x) * detail;
        let filtered = mix_variance(read_extras, read_derivatives, copies);
        // Once the pixel's length has averaged away most of the cascade, its variance is the expectation.
        let share = waves.slope_lengths[i] / max(wide, 1e-3);
        let settled = mix(filtered, waves.slopes[i] * max(1.0 - share, 0.0), (1.0 - smoothstep(SETTLE_FULL, SETTLE_START, share)) * grazing);
        let cascade_variance = mix(waves.slopes[i], settled, detail) * stilled * roughened * kept * kept;
        variance += cascade_variance;
        unresolved += cascade_variance;
        // Near hulls the first cascade's long waves give way to the sea they ride, before finer foam follows its crests.
        if i == 0 {
            slope += hulls.slope * detail;
            strain += hulls.strain * detail;
        }
        if i == 0 || i < CASCADES - 1 {
            swell_slope = slope;
            swell_strain = strain;
        }
    }
    // World slope of the displaced surface through the inverse transpose of the horizontal map's Jacobian.
    let xx = strain.x + 1.0;
    let zz = strain.y + 1.0;
    let cross_strain = strain.z;
    let jacobian = xx * zz - cross_strain * cross_strain;
    let world = vec2<f32>(zz * slope.x - cross_strain * slope.y, xx * slope.y - cross_strain * slope.x) / max(jacobian, MIN_JACOBIAN);
    let swell_xx = swell_strain.x + 1.0;
    let swell_zz = swell_strain.y + 1.0;
    let swell_jacobian = swell_xx * swell_zz - swell_strain.z * swell_strain.z;
    let swell_world = vec2<f32>(swell_zz * swell_slope.x - swell_strain.z * swell_slope.y,
        swell_xx * swell_slope.y - swell_strain.z * swell_slope.x) / max(swell_jacobian, MIN_JACOBIAN);
    // Waves shorter than the finest cascade: its slopes read as many times finer as its band spans.
    var ripple_slope = vec2<f32>(0.0);
    var ripple_variance = 0.0;
    var ripple_unresolved = 0.0;
    if CASCADES > 1 {
        let finest = CASCADES - 1;
        let ripple_level = max(log2(footprint * waves.tail.w), 0.0);
        let ripple_shown = 1.0 - smoothstep(top - 1.0, top, ripple_level);
        let read = textureSample(field_derivatives, field_sampler, xz / waves.tail.z + 0.5 / waves.info.z, finest);
        let drawn = -max(1.0 - ripple_level / top, 0.0) * ripple_shown;
        let kept = shore.scales[finest];
        ripple_slope = read.xy * (ripple_shown * still * sqrt(gain) * kept * RIPPLE_GAIN);
        let ripples = waves.slopes[finest] * RIPPLE_GAIN * RIPPLE_GAIN;
        ripple_variance = min(ripples, waves.tail.x) * drawn * still * gain * kept * kept;
        ripple_unresolved = min(ripples, waves.tail.y) * drawn * still * gain * kept * kept;
    }
    // The surf: the breakers' white water where the waves break on a shoaling shore, and the bubbles they drive under.
    let surf_cover = surf(xz, shore, wide, footprint);
    bubbles = max(bubbles, surf_cover.x);
    // Foam rides the water: per area of sea it is as dense as the surface is compressed.
    let gathered = 1.0 / clamp(jacobian, FOAM_JACOBIAN_MIN, FOAM_JACOBIAN_MAX);
    return WaveSample(
        world + ripple_slope + hulls.world,
        jacobian,
        foam * gathered,
        fresh * gathered,
        foam_soft * gathered,
        waves.info.w * shore_whitecaps(shore),
        bubbles,
        variance + ripple_variance + hulls.variance,
        unresolved + ripple_unresolved + hulls.variance,
        surf_cover,
        swell_world + hulls.world,
        swell_jacobian,
    );
}

// Light through the waves (native; was the web's `crest_transmission`, 0.04 of the beam tinted by the map's dark
// transmission colour and only looking into the sun, which drew nothing anyone could see). Light enters a wave, scatters
// in its water and leaves through the thin water of its upper face, coloured by the path it crossed (`WAVE_LIGHT_PATH`
// of the water's own absorption). The sky's light crosses a wave from every side; the sun's is strongest where its light
// and the eye's line meet at a small angle (a forward Henyey–Greenstein lobe), so a low sun ahead lights the faces through
// and a high one barely does, and only the sun's is stopped by a shadow (`beam`). Only a line of sight low over the water
// crosses a crest's thin water: looking down it goes into the deep, so the light fades out between `WAVE_LIGHT_LOW` and
// `WAVE_LIGHT_STEEP` of the eye's line (the sine of its angle below the horizon; from straight above, over a ship on the
// start screen, it drew the crests as blotches). How thin the water is comes from the waves longer than the finest
// cascade's (`WaveSample::swell_slope`, `swell_jacobian`): their crests (the Jacobian under 1) and the faces they turn
// toward the eye, a little more high on the swell (`height`, over the significant height). Bubbles (a bow wave, a wake,
// under a whitecap) scatter more of it. The coefficients are radiance over irradiance (1/sr), set by eye after the
// Waterline PRO ocean the look began from (2026-10-18, `native/docs/ocean/light.md`) and then held to a hint of teal on the near
// faces: Bill found the first, emerald version far too green.
const WAVE_LIGHT_PATH: f32 = 10.0;
const WAVE_LIGHT_SKY: f32 = 0.09;
const WAVE_LIGHT_SUN: f32 = 0.3;
const WAVE_LIGHT_FORWARD: f32 = 0.55;
const WAVE_LIGHT_SIDE: f32 = 0.15;
const WAVE_LIGHT_AERATED: f32 = 0.5;
const WAVE_LIGHT_LOW: f32 = 0.25;
const WAVE_LIGHT_STEEP: f32 = 0.7;

fn henyey_greenstein(g: f32, cosine: f32) -> f32 {
    return (1.0 - g * g) / pow(1.0 + g * g - 2.0 * g * cosine, 1.5);
}

// `sun_irradiance` is the sun's on a surface facing it, `sky_irradiance` the sky's on a level one.
fn wave_light(view: vec3<f32>, normal: vec3<f32>, sun: vec3<f32>, sun_irradiance: vec3<f32>, sky_irradiance: vec3<f32>,
    beam: f32, absorption: vec3<f32>, height: f32, jacobian: f32, aerated: f32, footprint: f32) -> vec3<f32> {
    let flat_view = normalize(vec2<f32>(view.x, view.z) + vec2<f32>(1e-6, 0.0));
    // The sun's light turned toward the eye: the angle between its way through the wave and the eye's line, so a low sun
    // ahead lights the waves through and a high one, whatever its bearing, little.
    let phase = henyey_greenstein(WAVE_LIGHT_FORWARD, dot(-sun, view)) / henyey_greenstein(WAVE_LIGHT_FORWARD, 1.0);
    let lobe = mix(WAVE_LIGHT_SIDE, 1.0, phase) * smoothstep(0.0, 0.08, sun.y);
    let crest = smoothstep(1.05, 0.7, jacobian);
    let long_crest = smoothstep(-0.2, 0.5, height);
    // The face a wave turns toward the eye (its slope toward the eye) is the one the light leaves through.
    let facing = smoothstep(-0.05, 0.3, dot(normal.xz, flat_view) / max(normal.y, 0.1));
    let low = 1.0 - smoothstep(WAVE_LIGHT_LOW, WAVE_LIGHT_STEEP, abs(view.y));
    // A pixel wider than the waves it would light draws none of it rather than their mean, which reads as a tint.
    let resolved = 1.0 - smoothstep(0.25, 1.5, footprint);
    let thin = mix(0.25, 1.0, crest) * facing * mix(0.6, 1.0, long_crest) * low * resolved;
    let light = sky_irradiance * WAVE_LIGHT_SKY + sun_irradiance * (WAVE_LIGHT_SUN * lobe * beam);
    return light * exp(absorption * -WAVE_LIGHT_PATH) * (thin * (1.0 + WAVE_LIGHT_AERATED * aerated));
}

// The air between the eye and the sea at `world` (`naval::aerial`: the atmosphere and the weather's haze, as every
// material takes it, so the far sea meets the dome's hazed horizon), while the scene's fog is on.
fn apply_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if surface.fog.w < 0.5 {
        return color;
    }
    return aerial_apply(color, world);
}

// Seen from below (`underside`): Snell's window shows the sky refracted through the surface, its radiance raised by
// water's n² on the way in; outside it, and in the window's own reflection, the surface mirrors the water below it by
// total internal reflection: the deep's light along the mirrored ray, as the underwater view lights the water about the
// camera (`water_path_light`), so beyond the window the underside is the deep seen in a mirror. The underwater view then
// takes the water between the surface and the camera.
const WATER_F0: f32 = 0.02;
const WATER_IOR: f32 = 1.333;
// Radiance crossing into water gains its refractive index squared (the basic radiance theorem).
const WINDOW_GAIN: f32 = WATER_IOR * WATER_IOR;
// Metres of water the mirrored ray crosses: the open deep.
const MIRRORED_DEEP: f32 = 1.0e4;
// Foam seen from below: the share of the light it shows from above that it passes down, how far that light takes the
// water's transmission tint, and how much of the underside dense foam (whitecaps, churned water), thin foam (windrows,
// a shore's) and the bubbles under them cover. The bubbles are the soft cloud a wake reads by from below; the foam's
// own lace is drawn through them at part strength, so the surface's hard edges do not print on the underside.
const FOAM_BELOW_LIGHT: f32 = 0.4;
const FOAM_BELOW_TINT: f32 = 0.4;
const FOAM_BELOW_DENSE: f32 = 0.5;
const FOAM_BELOW_THIN: f32 = 0.25;
const FOAM_BELOW_BUBBLES: f32 = 0.75;

fn water_fresnel(cosine: f32) -> f32 {
    return WATER_F0 + (1.0 - WATER_F0) * pow(1.0 - max(cosine, 1e-4), 5.0);
}

// `skyReflection`: the environment along a direction, the horizon's for anything below it.
fn sky_reflection(direction: vec3<f32>, roughness: f32) -> vec3<f32> {
    return env_radiance(sky_atlas, sky_sampler, sky_env, normalize(vec3<f32>(direction.x, max(direction.y, 0.0), direction.z)), roughness);
}

fn underside(
    view: vec3<f32>,
    normal: vec3<f32>,
    pigment: vec3<f32>,
    absorption: vec3<f32>,
    sun_direction: vec3<f32>,
    sun_color: vec3<f32>,
) -> vec3<f32> {
    let incident = -view;
    let down = -normal;
    let refracted = refract(incident, down, WATER_IOR);
    let mirrored = water_path_light(reflect(incident, down), 0.0, MIRRORED_DEEP, pigment, absorption, sun_direction, sun_color);
    let window = mix(sky_reflection(refracted, 0.0) * WINDOW_GAIN, mirrored, water_fresnel(max(dot(refracted, normal), 0.0)));
    return select(mirrored, window, dot(refracted, refracted) > 1e-6);
}

@fragment
fn fragment(input: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let xz = input.grid;
    // Metres per pixel, taken in uniform control flow: the trail's eddies give way to their mean below it.
    let footprint = max(fwidth(xz.x), fwidth(xz.y));
    // The pixel's area on the water (m²), for the sun's glints (`glitter_glints`).
    let grid_dx = dpdx(xz);
    let grid_dy = dpdy(xz);
    let pixel_area = abs(grid_dx.x * grid_dy.y - grid_dx.y * grid_dy.x);
    let wake = wake_surface(xz, input.world, footprint);
#ifdef OIL
    // A leaking hull's slick (naval_ocean::oil) stills the short waves as the wake's slick does, more where it is
    // thick, and damps their breaking: fewer whitecaps on it.
    let oil = oil_read(xz, footprint);
    var sample = wave_surface(xz, max(wake.slick, oil.calm), input.world.xz);
    let unbroken = 1.0 - oil.calm * OIL_WHITECAPS;
    sample.foam *= unbroken;
    sample.fresh *= unbroken;
    sample.foam_soft *= unbroken;
    sample.bubbles *= unbroken;
#else
    let sample = wave_surface(xz, wake.slick, input.world.xz);
#endif
    let to_camera = camera.position.xyz - input.world;
    let distance = length(to_camera);
    let view = to_camera / distance;
    // Rain rings the water: drawn where the pixel resolves them, unresolved roughness beyond (naval_ocean::rain). The
    // pixel's size on the water from the view (the mean of its extents across and along), smooth across triangles.
    let pixel_size = distance * 2.0 * camera.tan_half_fov_y * camera.viewport.w / sqrt(max(abs(view.y), 0.05));
    let rain = rain_surface(xz, waves.wind.z, surface.weather.x, pixel_size);
    // The wave slope plus the wake's own slope (`surfaceNormal`).
    let total = sample.slope - wake.normal.xz / max(wake.normal.y, 0.05);
    // The rain's rings join after the pixel's own spread of normals is taken (`footprint_variance`): their fade keeps
    // them to pixels that resolve them.
    let dry = normalize(vec3<f32>(-total.x, 1.0, -total.y));
    let up = normalize(vec3<f32>(-total.x - rain.slope.x, 1.0, -total.y - rain.slope.y));
    // The sea shades with the raw celestial light: the sun by day, the moon once it outshines it.
    let sun_direction = sky_env.light_direction;
    let sun_intensity = sky_env.light_intensity;
    let sun_radiance = sky_env.light_color * sun_intensity;
    // Ship shadows times the clouds' (WaterShadows.ts); faint under the moon, as the light that casts them.
    let shadow = water_sun_shadow(input.world, u32(surface.shadows.x)) * cloud_shadow(cloud_map, cloud_sampler, cloud_params, input.world);
    let lit = mix(1.0, shadow, clamp(sun_intensity / 2.0, 0.0, 1.0));
    // The glint and the light through crests need the beam itself: a cloud's floor of scattered light is not the
    // sun's disc, and a ship's shadow stops the beam outright.
    let beam = clamp((shadow - CLOUD_SHADOW_FLOOR) / (1.0 - CLOUD_SHADOW_FLOOR), 0.0, 1.0);
    let heading = waves.sea.z;
    let downwind = vec2<f32>(cos(heading), sin(heading));
    let wind_speed = waves.sea.y;
    // Physical reflections mirror the facets the viewer sees through their spread.
    let variances = wind_variances(sample.unresolved_variance + footprint_variance(dry), wind_speed)
        + vec2<f32>(rain.variance + wake.variance) * 0.5;
    let lobe = reflection_lobe(view, up, variances, downwind);
    let daylight = env_irradiance(sky_atlas, sky_sampler, sky_env, vec3<f32>(0.0, 1.0, 0.0));
    // Hull sides beside the water hide part of the sky it mirrors and is lit by (surface/shelter.ts).
    let contact = shelter_light(hull_shelter(input.world, lobe.direction, lobe.in_plane), daylight, sun_radiance, sun_direction);

    // The opaque scene behind this pixel, and how much water lies between it and the surface.
    let pixel = vec2<i32>(input.clip.xy);
    let depth = textureLoad(scene_depth, pixel, 0);
    let near = camera.near;
    let far = camera.far;
    let scene_view_z = -near * far / (depth * (far - near) + near);
    let ray_view_z = (camera.view * vec4<f32>(-view, 0.0)).z;
    // Nothing behind (the clear depth) is the open deep, however far the water is drawn: the skirt's lies beyond the
    // far plane, where a column measured to the far plane would read as a shore.
    let column = select(max(scene_view_z / min(ray_view_z, -1e-4) - distance, 0.0), OPEN_DEEP, depth <= 0.0);
    let optics = sample_water(surface.water, xz, surface.absorption.rgb);
    let absorption = optics.absorption;
    let backscatter = optics.backscatter;
    let attenuation = select(absorption, water_beam(optics) - backscatter, surface.water.absorption.w > 0.5);
    let pigment = select(surface.pigment.rgb, water_pigment(optics, surface.absorption.rgb, surface.pigment.rgb), surface.water.absorption.w > 0.5);
    // Light scattered back out of the water, lit by the sun and sky that cross the surface.
    let deep = upwelling(subsurface_reflectance(absorption, backscatter), sun_direction, sun_radiance, contact.irradiance, lit);
    let body = water_body(textureLoad(scene_color, pixel, 0).rgb, deep, column_transmittance(attenuation, backscatter, column, view));

    // Fresnel between the reflected sky (or ships and land, where the screen trace finds them) and the water body.
    let reflectance = mean_fresnel(dot(up, view), lobe.sigma_view);
    let sky = shelter_reflect(contact, sky_lobe(sky_atlas, sky_sampler, sky_env, lobe));
    // Hulls and islands the mirrored ray meets on screen replace the sky: the lobe's traced ray, its image smeared
    // over the lobe's in-plane spread about its centre.
    let to_view = camera.view;
    let traced = screen_space_reflection(
        scene_color,
        wake_sampler,
        scene_depth,
        (to_view * vec4<f32>(curved(input.world), 1.0)).xyz,
        (to_view * vec4<f32>(lobe.traced, 0.0)).xyz,
        // A constant false where the surface is built without them (`OceanSurface::traces`): the trace compiles out.
        REFLECTION_STEPS > 0 && surface.reflections.x > 0.5,
        surface.reflections.y,
        REFLECTION_STEPS,
        (to_view * vec4<f32>(lobe.up, 0.0)).xyz,
        lobe.in_plane,
        TRACE_BELOW,
    );
    let reflected = mix(sky, traced.color, traced.confidence);
    // How far up a wave this point sits: crests reach about Hs / 2.
    let height = input.world.y / max(waves.sea.x, 0.1);
    // The glitter's mean over the pixel, drawn as the few glints that make it (`glitter_glints`) on the waves' clock.
    let glint = luminance_ceiling(
        soft_knee(sun_glitter(up, view, sun_direction, sun_radiance, variances, downwind), sun_intensity * GLITTER_KNEE)
            * glitter_glints(xz, pixel_area, glitter_density(up, view, sun_direction, variances, downwind), waves.wind.z),
        sun_intensity * GLINT_PEAK,
    );

    // Foam: the texture laid in the wind's frame and drawn out along it by the crest foam's wind stretch.
    let stretch = surface.foam.x * 2.0 + 1.0;
    var patterns = foam_patterns(foam_map, foam_sampler, surface.foam.z, xz, heading, stretch, sun_direction);
    let foam_size = foam_footprint(xz);
    // Near the eye, every foam's lace takes the bubbles' walls (`bubble_walls`), only where there is foam to draw them:
    // whitecaps, a wake's white water, the surf, a shore's (no derivatives taken).
    if max(max(sample.foam, sample.fresh), max(wake.foam, sample.breakers.x)) > 1e-3 || column < SHORE_DEPTH {
        let walls = bubble_walls(xz, foam_size.size);
        patterns.pattern.r = with_bubble_walls(patterns.pattern.r, walls);
        patterns.streaks = with_bubble_walls(patterns.streaks, walls);
    }
    let lace = patterns.pattern.r;
    let whitecaps = whitecap_foam(sample.foam, sample.fresh, sample.foam_soft, sample.whitecap_share, patterns, foam_size, xz);
    let sky_up = env_radiance(sky_atlas, sky_sampler, sky_env, normalize(vec3<f32>(up.x, max(up.y, 0.0), up.z)), 1.0);
    let foam_light = foam_radiance(sky_up * contact.sky_share, sun_radiance, up, sun_direction, lit);
    // The realistic wake's churned water is the whitecaps' dense white water.
    let churned = churned_water(wake.foam, wake.young, patterns.pattern, patterns.blur);
    let aerated = aeration_with_wake(sample.bubbles * (1.0 - whitecaps.averaged), wake.bubbles);
#ifdef OIL
    // Under a film of oil the water's light crosses it down and back up, the oil's own dark brown takes what it stops,
    // and the film reflects as its thickness has it: from the clean sea's 2 % to 6 % in its colours, the oil's 4 % once
    // it is thick (naval_ocean::oil).
    var film = OilFilm(vec3<f32>(1.0), vec3<f32>(1.0));
    if oil.thickness > 0.0 {
        film = oil_film(oil.thickness, dot(up, view));
    }
    let film_light = oil_own(film, contact.irradiance + sun_radiance * (max(sun_direction.y, 0.0) * lit));
    let water = mix(bubble_cloud(body, aerated, foam_light, absorption) * film.through + film_light, reflected,
        min(aerated_reflectance(reflectance, aerated) * film.reflect, vec3<f32>(1.0)));
#else
    let water = mix(bubble_cloud(body, aerated, foam_light, absorption), reflected, aerated_reflectance(reflectance, aerated));
#endif
    // The light through the waves leaves through the surface (what it does not reflect) and across into air; the
    // churned water heaped against a hull under way gives little of it back (`froth`).
    let swell = sample.swell_slope - wake.normal.xz / max(wake.normal.y, 0.05);
    let swell_up = normalize(vec3<f32>(-swell.x, 1.0, -swell.y));
    let through = wave_light(view, swell_up, sun_direction, sun_radiance, daylight * contact.sky_share, beam, absorption,
        height, sample.swell_jacobian, aerated, footprint) * ((1.0 - reflectance) / REFRACTION_SQUARED)
        * (1.0 - wake.froth);
#ifdef OIL
    var above = water + through * film.through + glint * beam * film.reflect;
#else
    var above = water + through + glint * beam;
#endif

    // From the widest and faintest to the brightest: windrows, whitecaps, shorelines.
    var windrows = windrow_opacity(surface.foam.y, patterns.lines, surface.foam.z, patterns.gather, lace, patterns.streaks,
        max(patterns.blur, patterns.streaks_blur), sample.jacobian) * surface.surface_foam.a;
    // In a gale the whitecaps' old foam drifts downwind in long streaks (the second cascade's streak channel, written
    // by resolve.wgsl), shown in long patches laid on the world so the tile's repeat does not.
    if CASCADES > 1 {
        let old = textureSample(field_displacement, field_sampler, field_uv(xz, 1), 1).w;
        let wind = waves.wind.xy;
        let patches = value_noise(vec2<f32>(dot(xz, wind) / STREAK_PATCH_ALONG, dot(xz, vec2<f32>(-wind.y, wind.x)) / STREAK_PATCH_ACROSS), 907u);
        let gathered = old * smoothstep(0.25, 0.6, patches);
        // The streaks break along their length: the foam's tear laid long along the wind (no derivatives taken).
        var tear = 0.5;
        if gathered > 1e-3 {
            let wind_frame = vec2<f32>(dot(xz, wind) / STREAK_TEAR_STRETCH, dot(xz, vec2<f32>(-wind.y, wind.x)));
            tear = foam_tear(wind_frame, foam_size.size);
        }
        let streaks = gale_streaks(gathered, patterns.streaks, lace, max(patterns.streaks_blur, patterns.blur), tear);
        windrows = max(windrows, streaks);
    }
#ifdef OIL
    // The breaking that leaves the windrows' foam is damped on a slick too.
    windrows *= unbroken;
#endif
    let white = billows(patterns.pattern.a, patterns.churn_sunward, lace, max(whitecaps.dense, churned.dense), patterns.blur,
        absorption);
    // Where the water thins against a hull or a shore, broken foam: torn along the line at metres (the foam's tear, laid
    // finer), thinning away from the contact, lace rather than a painted stripe.
    var shore_tear = 0.5;
    if column < SHORE_DEPTH {
        shore_tear = foam_tear(xz * SHORE_TEAR_SCALE, foam_size.size * SHORE_TEAR_SCALE);
    }
    let shore_thin = 1.0 - smoothstep(0.0, SHORE_DEPTH, column);
    // Against a hull (whose sides hide the sky here) the hull's own contact foam draws the white water; the thin
    // column there is only where the water meets her plating, a line in any view.
    let against_hull = smoothstep(0.03, 0.2, 1.0 - contact.sky_share) * SHORE_HULL;
    let shore_cover = shore_thin * shore_thin * SHORE_COVER * torn_cover(shore_tear) * (1.0 - against_hull);
    let shore = foam_share(shore_cover, lace, patterns.blur, SHORE_EDGE) * surface.shoreline_foam.a;
    above = foam_over(above, water, foam_light * surface.surface_foam.rgb * thin_foam(absorption), reflected, reflectance, windrows);
    above = foam_over(above, water, foam_light * surface.crest_foam.rgb * white, reflected, reflectance,
        max(whitecaps.opacity * surface.crest_foam.a, churned.opacity));
    // The surf's white water: its bores dense, the broken water left behind them thinner.
    let breakers = foam_share(sample.breakers.x, lace, patterns.blur, SURF_EDGE) * surface.crest_foam.a;
    let breakers_white = billows(patterns.pattern.a, patterns.churn_sunward, lace, sample.breakers.y, patterns.blur,
        absorption);
    above = foam_over(above, water, foam_light * surface.crest_foam.rgb * breakers_white, reflected, reflectance,
        breakers);
    above = foam_over(above, water, foam_light * surface.shoreline_foam.rgb * thin_foam(absorption), reflected, reflectance, shore);
    // The white water's bubbles glint in the sun (`foam_sparkle`), brightest looking toward it.
    let white_cover = clamp(max(max(whitecaps.opacity * surface.crest_foam.a, churned.opacity), breakers), 0.0, 1.0);
    if white_cover > 0.02 && beam > 0.0 {
        let bubble_fresnel = water_fresnel(sqrt(clamp((1.0 + dot(view, sun_direction)) * 0.5, 0.0, 1.0)));
        let sparkle = foam_sparkle(xz, foam_size.size, waves.wind.z) * white_cover * beam
            * smoothstep(0.0, 0.05, sun_direction.y);
        above += luminance_ceiling(sun_radiance * (SPARKLE_GAIN * bubble_fresnel * sparkle), sun_intensity * GLINT_PEAK);
    }
#ifdef POINT_LIGHTS
    // The scene's point lights (flashes, fires, lightning's stroke), in the variant drawn on frames that have any
    // (`OceanSurface::point_lights`), and taken last, where few of the shader's values are live: the block costs it
    // registers. What the opaque scene hides from a light (a hull, her upperworks, another ship, the land) it does not
    // light: each is marched toward over the scene's depth (`light_seen`). Each term reaches the surface by the paths
    // the sun's does: the glints where no foam covers the water; the light in the water through the body's share of the
    // surface (`water_body`, the reflectance); the foam's light through each layer's opacity (`foam_over`). The glints
    // take the knee the sun's or moon's take.
    if scene_lights.count.x > 0u {
        let windrow_cover = clamp(windrows, 0.0, 1.0);
        let crest_cover = clamp(max(max(whitecaps.opacity * surface.crest_foam.a, churned.opacity), breakers), 0.0, 1.0);
        let shore_cover = clamp(shore, 0.0, 1.0);
        let uncovered = (1.0 - windrow_cover) * (1.0 - crest_cover) * (1.0 - shore_cover);
        let water_share = 1.0 - aerated_reflectance(reflectance, aerated);
        let in_water = subsurface_reflectance(absorption, backscatter)
            * (1.0 - column_transmittance(attenuation, backscatter, column, view)) * (water_share * uncovered / REFRACTION_SQUARED);
        let foam_cover = (surface.surface_foam.rgb * (windrow_cover * (1.0 - crest_cover) * (1.0 - shore_cover))
            + surface.crest_foam.rgb * (white * crest_cover * (1.0 - shore_cover))
            + surface.shoreline_foam.rgb * shore_cover) * (1.0 - reflectance * FOAM_GLOSS);
        // A light adding less than this to the pixel is not marched toward: its shadow could not show. With the water's
        // shadows off (the Graphics menu; the Low preset) none is.
        let least = select(3.4e38, max(LAMP_SEEN_LEAST, LAMP_SEEN_SHARE * dot(above, vec3<f32>(0.2126, 0.7152, 0.0722))),
            surface.shadows.x > 0.5);
        let shares = LightShares(vec3<f32>(uncovered), in_water, foam_cover, least, surface.shadows.x > 0.5);
        var lamps = SeaLights(vec3<f32>(0.0), vec3<f32>(0.0), vec3<f32>(0.0), false, 0.0, 0.0);
        var brightest = 0.0;
        // Each light's irradiance weighed by its glitter here (luminance): what the glints are held at.
        var held = 0.0;
        let frame = light_frame(up, variances, downwind, view);
        let dither = light_dither(input.clip.xy);
        // The lights that reach this pixel's cell, brightest at the eye first; the first few that show are marched
        // toward (`LAMP_MARCHES`).
        var marched = 0u;
        let cell = light_cell(input.world);
        for (var word = 0u; word < LIGHT_WORDS; word += 1u) {
            var bits = light_word(cell, word);
            while bits != 0u {
                let index = word * 32u + firstTrailingBit(bits);
                bits &= bits - 1u;
                let lamp = sea_point_light(scene_lights.lights[index], frame, input.world, up, view, variances,
                    downwind, scene_depth, dither, shares, marched < LAMP_MARCHES);
                marched += u32(lamp.marched);
                // The glints are counted by the facets of the light whose glitter is brightest here.
                let shine = dot(lamp.glint, vec3<f32>(0.2126, 0.7152, 0.0722));
                if shine > brightest {
                    brightest = shine;
                    lamps.density = lamp.density;
                }
                // Each light's glitter under the knee of its own light, as the sun's is under the sun's: a star shell
                // giving the sea twenty full moons' light, kneed at the moon's, was held flat at a fifth of a moon
                // over the water near it, a matte tan sheet.
                let kneed = soft_knee(lamp.glint, lamp.peak * GLITTER_KNEE);
                lamps.glint += kneed;
                held += lamp.peak * dot(kneed, vec3<f32>(0.2126, 0.7152, 0.0722));
                lamps.into_water += lamp.into_water;
                lamps.foam += lamp.foam;
            }
        }
        // The lamps' glitter as glints, as the sun's is: the mean drawn as the few wavelets that turn them to the eye,
        // at every pixel size, each glint's light held to one pixel at the ceiling their own light sets (`lamp_glints`):
        // each light's irradiance weighed by its share of the glitter here. Until 2026-10-21 the ceiling was the sum of
        // every light in the pixel's cell, so a searchlight lighting the water from 7 km, or a star shell whose glitter
        // lay elsewhere, held another star shell's glints three to ten times too high, and the bloom spread each into a
        // round soft dot.
        let mean = max(dot(lamps.glint, vec3<f32>(0.2126, 0.7152, 0.0722)), 1e-12);
        let most = held / mean * GLINT_PEAK / mean;
        let glinting = lamps.glint * lamp_glints(xz, pixel_area, lamps.density, waves.wind.z, most);
        above += glinting * uncovered + lamps.into_water * in_water + lamps.foam * foam_cover;
    }
#endif
    // Everything above was evaluated in uniform control flow (it takes screen-space derivatives). Only back faces seen
    // from under water pay for the underside; a camera above water sees back faces only where a storm crest folds over,
    // and those keep the crest's shading; a back face above the camera still counts, for a near plane that dips under.
    let submerged = surface.pigment.w > 0.5 && textureLoad(underwater_probe, vec2<i32>(0, 0), 0).g > 0.0;
    // A gale's spray hangs low between the eye and the water (naval_ocean::spray_haze), then the distance fog. The
    // underside is seen through water, which the underwater view takes: the air's spray and fog are for the sea seen
    // from above.
    let veil = exp(-spray_depth(camera.position.xyz, input.world, vec4<f32>(surface.weather.yzw, 0.0)));
    if veil < 1.0 {
        above = mix(spray_light(daylight * (1.0 / 3.141592653589793), sun_radiance * lit, view, sun_direction), above, veil);
    }
    var color = apply_fog(above, input.world);
    if !front && (submerged || input.world.y > camera.position.y) {
        color = underside(view, up, pigment, select(absorption, water_beam(optics), surface.water.absorption.w > 0.5), sun_direction, sky_env.light_color);
        // What floats on the sea shows from below (native only): whitecaps, a wake's churned water and a torpedo's
        // track let no image through and mirror nothing; they pass the daylight they gather down as a milky glow,
        // darker than the sky in Snell's window and brighter than the deep mirrored outside it. The bubbles under
        // them glow more softly. A ship is found from below by her wake.
        let crest_cover = clamp(max(max(whitecaps.opacity * surface.crest_foam.a, churned.opacity), breakers), 0.0, 1.0);
        let afloat = 1.0 - (1.0 - crest_cover * FOAM_BELOW_DENSE) * (1.0 - clamp(max(windrows, shore), 0.0, 1.0) * FOAM_BELOW_THIN)
            * (1.0 - clamp(aerated, 0.0, 1.0) * FOAM_BELOW_BUBBLES);
        let glow = foam_light * mix(vec3<f32>(1.0), surface.transmission.rgb, FOAM_BELOW_TINT) * FOAM_BELOW_LIGHT;
        color = mix(color, glow, afloat);
    }
    return vec4<f32>(color, 1.0);
}

// The motion pass for temporal anti-aliasing: the sea takes 70 % history (without it its glints dance), and keeps the
// motion below it. The rain's rings land somewhere new every frame, as its streaks do, so as the rain thickens the sea
// takes less, none from a third of a downpour (under history the rings faded).
@fragment
fn motion(input: VertexOut) -> @location(0) vec4<f32> {
    return motion_response(mix(0.3, 1.0, smoothstep(0.0, 0.3, surface.weather.x)));
}
