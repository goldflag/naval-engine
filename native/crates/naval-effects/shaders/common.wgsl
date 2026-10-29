// What every effect material shares: the scene's light, wind-free fog and flashes (the web game's `EffectLighting.ts`),
// the smoke occlusion map's `visibility` (the web game's `SmokeOcclusion.ts`), soft particle edges against the scene's
// depth (`EffectLighting.softEdge`) and the air between the eye and every effect (`naval::aerial`, as every material
// takes it).
//
// Groups: 0 the camera (the renderer's, with its sun shadow maps), 1 the lighting below, 2 the batch's own resources,
// 3 the scene reads (occlusion map, depth, the sky's environment the fog reads the sky behind from and the clouds the
// composite drew), which the occlusion pass itself never binds.
#define_import_path naval_effects::common

#import naval::camera::{camera, view_depth, ndc_from_pixel, view_ray}
#import naval::aerial::{aerial_apply, aerial_apply_additive}
#ifdef EFFECT_SKY
#import naval_sky::environment::{SkyEnvironment, env_radiance}
#endif
#ifdef EFFECT_SUN_SHADOW
#import naval::shadow::{sun_shadow, sun_shadow_maps, shadow_light_direction}
#endif
#ifdef EFFECT_CLOUD_SHADOW
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#endif

struct EffectLighting {
    // xyz toward the sun (or the brighter moon), w daylight 0–1.
    sun_direction: vec4<f32>,
    // rgb direct radiance scattered by smoke, w 1 while the occlusion map holds gas this frame.
    direct: vec4<f32>,
    // rgb sky fill reaching shaded folds where the sky's environment is not bound, w lightning's flash (a multiple of
    // the sky's light).
    ambient: vec4<f32>,
    // rgb fog tint, w power.
    fog_color: vec4<f32>,
    // start, end, sky blend distance, 1 with fog.
    fog: vec4<f32>,
    // rgb the sky behind a fragment when no environment is bound, w 1 to read the sky's environment instead.
    fog_sky: vec4<f32>,
    // Up to four transient point lights: xyz position, w power on smoke.
    flashes: array<vec4<f32>, 4>,
    // The clouds the composite drew before the effects (`cloud_visibility`): x the lowest layer's bottom, y the
    // highest layer's top (m above the sea), z the sea sphere's radius (m), w 1 while they are drawn.
    clouds: vec4<f32>,
    // The trial looks switched on (`Trial` in trial.rs), each 1 when on: x warp, y flipbook, z shadows, w volume.
    trial: vec4<f32>,
    // x the night's lift on real light (`EffectLighting::lift`: 1 by day, 16,700 after dark), which every glow given in
    // the sun's units takes (`emitted`): the hot fragments, the tracers; after dark the fires and the flashes take less
    // (`EffectLighting::set_lift`): y a fire's flames and embers, z its glow on its own smoke (`emitted_at`), w the
    // flashes' light on the smoke and its cap, and `flash_glow.x` the flashes' and blasts' own glow.
    emission: vec4<f32>,
    // rgb the colour of `flashes`' light (linear, weighed by their power): the flash's 2,500 K reddening to the
    // fireball's glow, as their light on the hulls and the sea; w 1 while the occlusion map holds the fires' smoke
    // columns (`columns_through`).
    flash_color: vec4<f32>,
    // x the lift of the flashes' and blasts' own glow: the flash sprites and sparks, the gas of a gun or a blast, a
    // detonation's fireball (`EffectLighting::flash_glow`); yzw unused.
    flash_glow: vec4<f32>,
}

// The brightest a glow is drawn (the scene's units): the scene's half floats hold 65,504, and a few glows may add over
// one another. After dark a flash's white heat lifted with the night stands some 16 stops over the moonlit sea (5 of
// the sun's units × 16,700); held here, it keeps its colour's balance (`emitted`).
const EMISSION_CEILING: f32 = 16384.0;

// What glows (radiance given in the sun's units) as the scene takes it this frame: lifted with the night as the moon
// and the lamps are, so a flash or a fire stands beside them at its real brightness, under `EMISSION_CEILING`.
fn emitted(radiance: vec3<f32>) -> vec3<f32> {
    return emitted_at(radiance, lighting.emission.x);
}

// What glows at a lift of its own: a fire's flames and embers (`lighting.emission.y`) and its glow on its smoke (`.z`).
fn emitted_at(radiance: vec3<f32>, lift: f32) -> vec3<f32> {
    let lifted = radiance * lift;
    let brightest = max(lifted.r, max(lifted.g, lifted.b));
    return lifted * min(1.0, EMISSION_CEILING / max(brightest, 1e-6));
}

// Candela a square metre in a unit of the scene's radiance by day: a unit of irradiance is 16,700 lux
// (`naval_render::lights::DAY_LUX`) and a white diffuser under it sends 1/π of it to the eye, as the hulls' shading does.
const DAY_LUX: f32 = 16700.0;

// A black body's light at `kelvin`, linear sRGB in the sun's units (radiance; `DAY_LUX` candela a square metre a unit by
// day): Planck's law integrated against the CIE 1931 observer, fitted within a few per cent over 1,200–1,800 K
// (`math.rs`' `Rgb::black_body` is its twin). Its luminance is 7,777 cd/m² at 1,500 K and goes as e^(−24,200 K / T)
// about it: 650 cd/m² at 1,300 K, 21,500 at 1,600 K, near 600,000 at platinum's freezing point (2,042 K, the old
// candle's 60 cd/cm²). Its green is 0.000242 × (T − 900 K) of its red, 0.145 at 1,500 K; its blue lies below the sRGB
// gamut, and is none.
fn black_body(kelvin: f32) -> vec3<f32> {
    let luminance = (7777.0 / DAY_LUX) * exp(24200.0 * (1.0 / 1500.0 - 1.0 / max(kelvin, 500.0)));
    let green = max(2.42e-4 * (kelvin - 900.0), 0.0);
    return vec3<f32>(1.0, green, 0.0) * (luminance / (0.2126 + 0.7152 * green));
}

// A fire's flame at `heat` on the gas sprite's ramp (about 1.4 through a full fire's thick core, 0.6 where its flame
// thins, nothing in its soot): luminous soot glowing as a black body (`black_body`) at the temperature a big fuel fire's
// flame burns at, 1,300–1,600 K, and as bright, in the sun's units. Its core at 1,590 K is twice as bright as grey paint
// in the noon sun (19,400 against 9,500 cd/m²) and deep orange, where it thins 1,385 K, a tenth of that and red-orange,
// its tips dull red, out by 800 K. Hotter than the core only a fire's rolling bodies of burning gas, to 1,610 K.
fn flame_light(heat: f32) -> vec3<f32> {
    return black_body(800.0 + 790.0 * pow(clamp(heat / 1.4, 0.0, 1.08), 0.355));
}

@group(1) @binding(0) var<uniform> lighting: EffectLighting;

@group(3) @binding(0) var occlusion_fronts: texture_2d<f32>;
@group(3) @binding(1) var occlusion_depths: texture_2d<f32>;
@group(3) @binding(2) var occlusion_sampler: sampler;
@group(3) @binding(3) var scene_depth: texture_depth_2d;
#ifdef EFFECT_SKY
// The sky's environment (naval-sky's atlas, sampler and uniform; a black placeholder until the host sets one), in the
// effects layer's scene reads (`EFFECT_SKY`); other users of this module (the shells) keep the flat stand-in.
@group(3) @binding(4) var sky_atlas: texture_2d<f32>;
@group(3) @binding(5) var sky_sampler: sampler;
@group(3) @binding(6) var<uniform> sky_env: SkyEnvironment;
// The clouds' reconstruction the composite drew this frame (naval-clouds' `resolve.wgsl`: premultiplied radiance,
// opacity in alpha, the drawing buffer's size; a clear 1 × 1 without clouds).
@group(3) @binding(7) var cloud_cover: texture_2d<f32>;
// The splash columns' composite drawn before the effects (naval-water-effects' `WaterEffects::cover`): premultiplied
// radiance and opacity at the march's resolution, and over the same texels the view depth (m) their water lies at times
// that opacity (a clear 1 × 1 without columns).
@group(3) @binding(8) var splash_cover: texture_2d<f32>;
@group(3) @binding(9) var splash_lies: texture_2d<f32>;
#endif
#ifdef EFFECT_CLOUD_SHADOW
// The clouds' shadow (naval-clouds' map, its sampler and uniforms; `naval_clouds::shadow`), read per vertex: the smoke
// takes the sun's light through the clouds as the hulls and the sea do (white, disabled, until the host sets it).
@group(3) @binding(44) var cloud_shadow_map: texture_2d<f32>;
@group(3) @binding(45) var cloud_shadow_sampler: sampler;
@group(3) @binding(46) var<uniform> cloud_shadow_params: CloudShadow;
#endif

// The Hermite step three's `smoothstep` compiles to, defined for any edge order.
fn sstep(low: f32, high: f32, x: f32) -> f32 {
    let t = clamp((x - low) / (high - low), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Corner `vertex` (0–5, two triangles) of a unit quad centred on the origin: −0.5 … 0.5 on each axis.
fn quad_corner(vertex: u32) -> vec2<f32> {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5),
        vec2<f32>(-0.5, -0.5), vec2<f32>(0.5, 0.5), vec2<f32>(-0.5, 0.5));
    return corners[vertex];
}

// Share of the sky's light that smoke scatters back: the albedo the direct light implies (`set_illumination`
// takes 1.25 of the sea's 5.8 at noon, a diffuser of albedo π × 1.25 / 5.8). naval-water-effects' spray.wgsl mirrors it.
const SKY_ALBEDO: f32 = 0.677;

// The sky's light on an effect from above (rgb, the radiance of a white diffuser facing up): the environment's, as the
// ships' is, so it dims with dusk, night and overcast and takes the sky's colour, lifted by lightning's flash. Without
// the environment (the shells, the occlusion pass), the authored stand-in.
fn sky_fill() -> vec3<f32> {
#ifdef EFFECT_SKY
    if lighting.fog_sky.w > 0.5 {
        let sky = env_radiance(sky_atlas, sky_sampler, sky_env, vec3<f32>(0.0, 1.0, 0.0), 1.0);
        return sky * SKY_ALBEDO * (1.0 + lighting.ambient.w);
    }
#endif
    return lighting.ambient.rgb;
}

// `sky_fill`'s light on a face of an effect looking along `direction` (unit, world): what that face sees of the sky
// and of the sea mirroring it, so a face looking at the horizon under a low sun takes its glow, and one looking down
// the sea's dark. Without the environment, `sky_fill`'s stand-in by the hemisphere's rule, the sea a tenth of it.
fn sky_toward(direction: vec3<f32>) -> vec3<f32> {
#ifdef EFFECT_SKY
    if lighting.fog_sky.w > 0.5 {
        let sky = env_radiance(sky_atlas, sky_sampler, sky_env, direction, 1.0);
        return sky * SKY_ALBEDO * (1.0 + lighting.ambient.w);
    }
#endif
    return lighting.ambient.rgb * mix(0.1, 1.0, 0.5 + 0.5 * direction.y);
}

// Luminance of the light on an effect under a clear noon sky (direct about 1.19, `sky_fill` about 0.3).
const CLEAR_NOON_LIGHT: f32 = 1.4;

// How bright the scene's light is, 0–1, for what is drawn in its daylight colour (the shells' vapour trails): the sky's
// and the sun's (or moon's) against a clear noon's, or the authored daylight without the sky's environment.
fn scene_light() -> f32 {
#ifdef EFFECT_SKY
    if lighting.fog_sky.w > 0.5 {
        let light = sky_fill() + lighting.direct.rgb;
        return min(dot(light, vec3<f32>(0.2126, 0.7152, 0.0722)) / CLEAR_NOON_LIGHT, 1.0);
    }
#endif
    return lighting.sun_direction.w;
}

// The air between the eye and an effect fragment at `world` (`naval::aerial`: the atmosphere and the weather's haze, as
// every material takes it), while the scene's fog is on. For straight alpha over what is behind.
fn effect_fog(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if lighting.fog.w <= 0.0 {
        return color;
    }
    return aerial_apply(color, world);
}

// The same for light added to what is behind (additive blending: flashes, glows): the air only dims it.
fn effect_fog_additive(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    if lighting.fog.w <= 0.0 {
        return color;
    }
    return aerial_apply_additive(color, world);
}

// Soft particles: 0 where the fragment meets the opaque surface behind it, 1 `meters` or more in front of it.
fn soft_edge(pixel: vec2<f32>, view_distance: f32, meters: f32) -> f32 {
    let depth = textureLoad(scene_depth, vec2<i32>(pixel), 0);
    return sstep(0.0, meters, view_depth(depth) - view_distance);
}

// 1 / (1 + metres): nearer is larger, and a cleared map (zero) holds no smoke.
fn nearness_of(meters: f32) -> f32 {
    return 1.0 / (max(meters, 0.01) + 1.0);
}

// Metres back from a nearness; far beyond any smoke where the map holds none.
fn distance_of(nearness: f32) -> f32 {
    return 1.0 / max(nearness, 1e-5) - 1.0;
}

// `SmokeOcclusion.visibility`: opacity multiplier for a smoke or fire fragment at framebuffer `pixel` whose near
// surface lies `front` metres away: what the gas in front lets through (`smoke_through`), the clouds and the splash
// columns in front.
fn smoke_visibility(pixel: vec2<f32>, front: f32, depth: f32, gas: bool) -> f32 {
    return smoke_through(pixel, front, depth, gas, !gas) * clouds_through(pixel, front) * splashes_through(pixel, front);
}

// What the occlusion map's gas lets through to a fragment at framebuffer `pixel` whose near surface lies `front` metres
// away: 1 in front of the gas, behind it what the gas lets through, reached `depth` metres behind each level's front. A
// `gas` batch reads only the densest level (see the web source). `spread`: and the gas's optical depth, spread about
// one middle for all of it.
fn smoke_through(pixel: vec2<f32>, front: f32, depth: f32, gas: bool, spread: bool) -> f32 {
    var visible = 1.0;
    if lighting.direct.w > 0.0 {
        let uv = pixel * camera.viewport.zw;
        let fronts = textureSampleLevel(occlusion_fronts, occlusion_sampler, uv, 0.0);
        let reach = max(depth, 1.0);
        let slack = reach * 0.1 + 0.5;
        let levels = vec4<f32>(0.25, 0.5, 0.8, 0.95);
        for (var i = 0; i < 4; i++) {
            if gas && i < 3 {
                continue;
            }
            let through = clamp((front - distance_of(fronts[i]) - slack) / reach, 0.0, 1.0);
            // (1 − level) ^ through.
            visible = min(visible, exp2(through * log2(1.0 - levels[i])));
        }
        if spread {
            // All the gas in front, spread evenly in depth from its nearest front to as far behind its middle.
            let depths = textureSampleLevel(occlusion_depths, occlusion_sampler, uv, 0.0);
            let optical = depths.r;
            let centre = depths.g / max(optical, 1e-4);
            let nearest = max(fronts.r, centre);
            let here = nearness_of(front);
            let share = clamp((nearest - here) / max((nearest - centre) * 2.0, 1e-7), 0.0, 1.0);
            visible = min(visible, exp(-optical * share));
        }
    }
    return visible;
}

// The patchiness of a fire's column (`columns_through`): the shape of its optical depth's spread about its mean.
const COLUMN_PATCHES: f32 = 1.0;

// What the fires' smoke columns let through to a fragment at framebuffer `pixel` whose near surface lies `front` metres
// away: their optical depth in the occlusion map (`depths.b`, which the column's links and billows add up above the
// flames they rose from: `column_occluder`), taken as lying at its fronts' mean distance (`depths.a` over it) and reached
// `depth` metres behind it. 1 while the map holds no column (`lighting.flash_color.w`). Only what draws after the columns
// reads it (a fire's flames and bodies of burning gas, its embers): the gas drawn before them they cover by drawing over
// it, and the columns' own links are not dimmed by each other twice.
//
// A column's smoke is patchy: its optical depth swings about the map's mean from eddy to eddy, and what a patchy screen
// lets through is the mean of e^(−τ), more than e^(−mean τ). Taken as gamma-distributed about its mean with shape
// `COLUMN_PATCHES`, that is (1 + τ / k)^(−k): a tenth through a mean depth of nine, where e^(−τ) let through a
// ten-thousandth, so from above a fire's flames glow dull orange through the column over them, brighter where it is thin
// (until 2026-10-25 they were black under it), and through a thin veil as before (half through a depth of one, against
// 0.37).
fn columns_through(pixel: vec2<f32>, front: f32, depth: f32) -> f32 {
    if lighting.flash_color.w <= 0.0 {
        return 1.0;
    }
    let depths = textureSampleLevel(occlusion_depths, occlusion_sampler, pixel * camera.viewport.zw, 0.0);
    let optical = depths.b;
    if optical < 1e-3 {
        return 1.0;
    }
    let reach = max(depth, 1.0);
    let through = clamp((front - distance_of(depths.a / optical) - (reach * 0.1 + 0.5)) / reach, 0.0, 1.0);
    return pow(1.0 + optical * through / COLUMN_PATCHES, -COLUMN_PATCHES);
}

// What the clouds the composite drew let through to a fragment `front` metres away (1 without `EFFECT_SKY`).
fn clouds_through(pixel: vec2<f32>, front: f32) -> f32 {
#ifdef EFFECT_SKY
    return cloud_visibility(pixel, front);
#else
    return 1.0;
#endif
}

// How many metres a fragment takes to pass from in front of a splash column's water to behind it, and the share of its
// distance added to them: the cover holds the depth the column's opacity lies at on the mean, in water tens of metres
// across, and at the march's resolution.
const SPLASH_SPAN: f32 = 12.0;
const SPLASH_SPAN_SHARE: f32 = 0.01;

// What the splash columns let through to a fragment at framebuffer `pixel` lying `front` metres along the view (1
// without `EFFECT_SKY`). Their composite laid them over the scene before the effects drew, and the scene's depth does
// not hold them, so without this a burning ship's smoke behind a column drew over it. In front of the depth the
// column's water lies at, all of the fragment; behind it, what the column leaves of the scene.
fn splashes_through(pixel: vec2<f32>, front: f32) -> f32 {
#ifdef EFFECT_SKY
    let size = textureDimensions(splash_cover);
    let texel = min(vec2<u32>(pixel * camera.viewport.zw * vec2<f32>(size)), size - 1u);
    let opacity = clamp(textureLoad(splash_cover, texel, 0).a, 0.0, 1.0);
    if opacity < 0.002 {
        return 1.0;
    }
    let lies = textureLoad(splash_lies, texel, 0).r / opacity;
    let span = SPLASH_SPAN + lies * SPLASH_SPAN_SHARE;
    return 1.0 - opacity * sstep(lies - span, lies + span, front);
#else
    return 1.0;
#endif
}

// Look `merge` (`Trial::merge`): the least weight a fragment of the merged smoke keeps, however much smoke lies in front
// of it, so a pixel whose every fragment lies behind the occlusion map's front still averages them.
const MERGE_LEAST: f32 = 1.0 / 256.0;

// Look `merge`: a fragment of smoke into the merged smoke's two targets (`EffectsLayer`'s merged smoke): its light
// times its opacity, and its opacity, both weighed by what the gas in front lets through (`smoke_through`), added up
// in `sum`; its opacity multiplied into what the scene still shows (`clear`). The composite lays `sum.rgb / sum.a`
// over the scene with an opacity of `1 − clear`: added, the fragments need no order, and the nearer smoke shows most.
// `colour` is the fragment as the layered draw blends it (its light and its opacity); `front` and `depth` as
// `smoke_through` takes them; `levels`: weighed by the map's levels only, not by its optical depth's spread
// (`smoke_through`'s `spread`).
struct MergedSmoke {
    @location(0) sum: vec4<f32>,
    @location(1) clear: vec4<f32>,
}

fn merged_smoke(colour: vec4<f32>, pixel: vec2<f32>, front: f32, depth: f32, levels: bool) -> MergedSmoke {
    let weight = colour.a * max(smoke_through(pixel, front, depth, false, !levels), MERGE_LEAST);
    return MergedSmoke(vec4<f32>(colour.rgb * weight, weight), vec4<f32>(colour.a));
}

#ifdef EFFECT_SKY
// How far the clouds' march takes a level ray through fog and rain (naval-clouds' `SURFACE_REACH`): what an effect over
// the sky stands in front of.
const CLOUD_SURFACE_REACH: f32 = 12000.0;

// Where a ray from the eye rising by `rise` (the sine of its elevation) is under `level` m above the sea: from x to y m
// along it, or x past y where it never is. Its height over the sea's sphere goes as eye + rise·s + s² / 2R, the clouds'
// march's sphere near the eye (naval-ships' `strip_under`).
fn cloud_under(level: f32, rise: f32) -> vec2<f32> {
    let curve = 0.5 / lighting.clouds.z;
    let lift = camera.position.y - level;
    let discriminant = rise * rise - 4.0 * curve * lift;
    if discriminant <= 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    // The roots without cancellation (`q` is never 0 here).
    let q = -0.5 * (rise + select(-1.0, 1.0, rise >= 0.0) * sqrt(discriminant));
    let a = q / curve;
    let b = lift / q;
    return vec2<f32>(min(a, b), max(a, b));
}

// Of the first `run` m of a ray from the eye rising by `rise`, the metres between the lowest layer's bottom and the
// highest's top.
fn cloud_run(run: f32, rise: f32) -> f32 {
    let top = cloud_under(lighting.clouds.y, rise);
    let bottom = cloud_under(lighting.clouds.x, rise);
    return max(min(run, top.y) - max(top.x, 0.0), 0.0) - max(min(run, bottom.y) - max(bottom.x, 0.0), 0.0);
}

// What clouds of opacity `opacity` over a pixel let through to a point `distance` m along its unit `ray`, the opaque
// scene `hit` m along it (0 for the sky). Spread evenly through the layers along the ray, they show (1 − opacity) ^ share
// through the share of the ray's metres in them that lies in front of the point. The march stops at the opaque scene;
// over the sky it runs on to the sea or its reach. Metres, not heights: a level ray from a bridge in a fog bank crosses
// the fog for kilometres and rises a few metres, and a funnel's smoke two miles off showed dark through the fog that
// hid her hull.
fn clouds_before(opacity: f32, ray: vec3<f32>, distance: f32, hit: f32) -> f32 {
    // From under the sea the composite draws no clouds.
    if opacity < 1e-3 || camera.position.y < 0.0 {
        return 1.0;
    }
    let before = cloud_run(distance, ray.y);
    if before <= 0.0 {
        return 1.0;
    }
    var reach = hit;
    if hit <= 0.0 {
        let sea = cloud_under(0.0, ray.y);
        reach = select(CLOUD_SURFACE_REACH, min(sea.x, CLOUD_SURFACE_REACH), sea.x > 0.0 && sea.x <= sea.y);
    }
    let share = clamp(before / max(cloud_run(max(reach, distance), ray.y), 1e-3), 0.0, 1.0);
    return exp2(share * log2(max(1.0 - opacity, 1e-4)));
}

// What the clouds let through to a fragment at framebuffer `pixel` lying `front` metres along the view. The composite
// laid the clouds over the scene before the effects drew, so without this a plume under a deck of cloud seen from above
// it (or a burning aircraft's trail above one seen from below) showed through the cloud, and a funnel's smoke through a
// fog bank (`clouds_before`). A fragment under the clouds seen from under them keeps all of itself.
fn cloud_visibility(pixel: vec2<f32>, front: f32) -> f32 {
    if lighting.clouds.w <= 0.0 {
        return 1.0;
    }
    let ray = view_ray(ndc_from_pixel(pixel));
    // Metres along the ray per metre along the view.
    let along = 1.0 / max(dot(ray, -camera.inverse_view[2].xyz), 1e-4);
    let size = textureDimensions(cloud_cover);
    let texel = min(vec2<u32>(pixel * camera.viewport.zw * vec2<f32>(size)), size - 1u);
    let opacity = clamp(textureLoad(cloud_cover, texel, 0).a, 0.0, 1.0);
    let depth = textureLoad(scene_depth, vec2<i32>(pixel), 0);
    let hit = select(0.0, view_depth(depth) * along, depth > 0.0);
    return clouds_before(opacity, ray, front * along, hit);
}
#endif

// Of the sun's (or moon's) direct light, what still reaches smoke standing in a shadow: the light the sunlit smoke
// about it scatters in, so a plume crossing a hull's shadow dims rather than going black.
const SHADOWED_SMOKE: f32 = 0.18;

// The share of the sun's (or moon's) direct light reaching a puff of smoke at `centre`, `radius` metres across its
// half, through the renderer's sun shadow maps (hulls, aircraft, land), read a radius toward the light so the puff's
// own surroundings below it do not count, and through the clouds (`EFFECT_CLOUD_SHADOW`: the clouds' shadow, which
// the hulls and the sea take; under a deck none of the beam reaches the smoke, and it takes the sky's light alone).
// The depth maps alone: the smoke's own shade is the smoke shadow map's (`naval_effects::shadow::puff_light`, which
// calls this per vertex). It scales only the direct light: the sky's light, flashes, firelight and the gas's own
// burning are not shadowed. 1 where the pipeline is built without `EFFECT_SUN_SHADOW` (the occlusion pass) or the
// maps are off.
fn puff_sunlight(centre: vec3<f32>, radius: f32) -> f32 {
#ifdef EFFECT_SUN_SHADOW
    let toward = shadow_light_direction();
    let sunlit = mix(SHADOWED_SMOKE, 1.0, sun_shadow_maps(centre + toward * radius, toward));
#ifdef EFFECT_CLOUD_SHADOW
    // The map holds the whole column's shade: smoke in the layers keeps the share of them above it (evenly, from their
    // bottom to their top: `lighting.clouds`), smoke over them (an aircraft's trail, flak over a deck) none.
    let shade = cloud_shadow(cloud_shadow_map, cloud_shadow_sampler, cloud_shadow_params, centre);
    let above = select(1.0, sstep(lighting.clouds.y, lighting.clouds.x, centre.y), lighting.clouds.w > 0.5);
    return sunlit * mix(1.0, shade, above);
#else
    return sunlit;
#endif
#else
    return 1.0;
#endif
}

// The share of the sun's light reaching a solid surface at `position` with unit normal `normal` (strike debris, shell
// bodies): the renderer's `sun_shadow` (its depth maps and the smoke over them), 1 without `EFFECT_SUN_SHADOW`.
fn surface_sunlight(position: vec3<f32>, normal: vec3<f32>) -> f32 {
#ifdef EFFECT_SUN_SHADOW
    return sun_shadow(position, normal);
#else
    return 1.0;
#endif
}

// `smokeRadiance` (the web game's `SmokeSpriteMaterial.ts`): radiance scattered by smoke with unit albedo for a world
// normal and view vector. `density` thins the forward-scattered rim; `thickness` darkens the sky fill in the core.
// `light` scales the direct light (x) and the sky's (y): `naval_effects::shadow::puff_light`.
fn smoke_radiance(n: vec3<f32>, to_camera: vec3<f32>, density: f32, thickness: f32, light: vec2<f32>) -> vec3<f32> {
    let sun = normalize(lighting.sun_direction.xyz);
    // Smoke scatters broadly: wrap the terminator well past 90°, and keep the shaded side lifted.
    let diffuse = clamp((dot(n, sun) + 0.6) / 1.6, 0.0, 1.0);
    let lit = diffuse * 0.8 + 0.2;
    // Looking toward the light: thin edges glow (silver lining), dense cores stay dark.
    let toward = clamp(dot(-to_camera, sun), 0.0, 1.0);
    let rim = pow(toward, 6.0) * (1.0 - density) * 1.4 + pow(toward, 2.0) * 0.18;
    let sky = sky_fill() * (0.72 + n.y * 0.28) * mix(1.0, 0.72, thickness);
    return sky * light.y + lighting.direct.rgb * ((lit + rim) * light.x);
}

struct SixWay {
    positive: vec3<f32>,
    negative: vec3<f32>,
}

// Squared components of a unit direction on its six axes: the six-way weights, summing to one.
fn six_way(direction: vec3<f32>) -> SixWay {
    let positive = max(direction, vec3<f32>(0.0));
    let negative = max(-direction, vec3<f32>(0.0));
    return SixWay(positive * positive, negative * negative);
}
