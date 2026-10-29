// The cloud field: weather, shape and density at a point, a port of `clouds.glsl` of @takram/three-clouds. Where
// takram looks its textures up from ECEF positions and a cube-sphere UV of the globe, the game's are world-fixed and
// planar in the clouds' drifting frame (`noise_position`): the weather tiles every `1 / repeats.x` metres (100 km, the
// scale of takram's 100 repeats per cube face), the shape and detail volumes every `1 / repeats.y` and `1 / repeats.z`
// metres as takram's do, and the whole field rides the scene's wind.
//
// MIT License, Copyright (c) 2024 Shota Matsuda (https://github.com/takram-design-engineering/three-geospatial). The
// full notice is in `naval-clouds/THIRD_PARTY.md`.
#define_import_path naval_clouds::field

#import naval_clouds::common::{clouds, local_weather, anchor_weights, composite_near, composite_far, weather_tables, shape_volume, detail_volume, turbulence_map, blue_noise, repeat_sampler, clamp_sampler, remap_clamped, remap_clamped4, saturate4}

const GOLDEN_RATIO: f32 = 0.6180339887498949;

// takram's `getSTBN`: a blue-noise value in [0, 1) for a pixel (the pass's own pixel grid) on this update. The
// clouds' 128² mask, offset each frame by the golden ratio over a cycle of 64 frames, stands in for NVIDIA's
// spatiotemporal blue noise.
fn stbn(pixel: vec2<u32>) -> f32 {
    let level = textureLoad(blue_noise, vec2<i32>(pixel % vec2<u32>(128u)), 0).r;
    let frame = f32(u32(clouds.frame.x) % 64u);
    return fract((level * 255.0 + 0.5) / 256.0 + frame * GOLDEN_RATIO);
}

// The march's jitter for a pixel of the drawing buffer on its `sample`-th march (the update counter over 16: each pixel
// is marched once in 16 updates): the blue noise at the pixel, stepped by the golden ratio once per march of that
// pixel, so the samples a pixel's mean gathers spread evenly over [0, 1). `stbn` steps every update over a cycle of
// 64, which gave a pixel, marched every 16th update, only four jitters, all within a third of [0, 1): its mean never
// reached the clouds, and the grain stood in every still.
fn march_jitter(pixel: vec2<u32>, sample: u32) -> f32 {
    let level = textureLoad(blue_noise, vec2<i32>(pixel % vec2<u32>(128u)), 0).r;
    return fract((level * 255.0 + 0.5) / 256.0 + f32(sample % 256u) * GOLDEN_RATIO);
}

// The steps of the plastic number's R2 sequence (Roberts 2018), whose first points fill a square evenly.
const R2: vec2<f32> = vec2<f32>(0.7548776662466927, 0.5698402909980532);

// Where the ray of a pixel's `sample`-th march crosses the pixel, from its centre (−0.5 to 0.5 each way): the R2
// sequence, shifted for each pixel by the blue noise at two other places of the mask, so the samples a pixel's mean
// gathers cover the pixel evenly and neighbouring pixels' are out of step. Until 2026-10-28 every ray passed through
// the pixel's centre, so a mean converged to the clouds at one point of each pixel, and the detail and edges finer than
// a pixel stood as a fixed grain no number of samples took out: on a held view through a 38° lens, the clouds
// converged at the centres (64 samples) and over the pixels differ by 0.69 levels of 255 of fine grain, the latter
// holding 6% less fine-scale energy.
fn march_subpixel(pixel: vec2<u32>, sample: u32) -> vec2<f32> {
    let shift = vec2<f32>(
        textureLoad(blue_noise, vec2<i32>((pixel + vec2<u32>(61u, 17u)) % vec2<u32>(128u)), 0).r,
        textureLoad(blue_noise, vec2<i32>((pixel + vec2<u32>(29u, 83u)) % vec2<u32>(128u)), 0).r,
    );
    return fract(shift + f32(sample % 256u) * R2) - 0.5;
}

// The shapes' frame at a planet-frame position's offset from the camera `d` (m, the scene's axes): the noise anchor's
// chart through its second-order map (`naval_clouds::laying`), so the shapes stand still on the Earth across a re-lay
// of the scene.
fn noise_xz(d: vec2<f32>) -> vec2<f32> {
    let v = clouds.noise_origin;
    let j = clouds.noise_jacobian;
    let h = clouds.noise_hessian;
    let east = v.x + j.x * d.x + j.y * d.y + 0.5 * (h.x * d.x * d.x + 2.0 * h.y * d.x * d.y + h.z * d.y * d.y);
    let south = v.y + j.z * d.x + j.w * d.y + 0.5 * (h.w * d.x * d.x + 2.0 * v.z * d.x * d.y + v.w * d.y * d.y);
    return vec2<f32>(east, south);
}

// A planet-frame position (m) in the clouds' drifting frame the shape, detail and turbulence are looked up in: (east,
// height above the sea sphere, south) on the noise anchor's chart.
fn noise_position(position: vec3<f32>) -> vec3<f32> {
    let n = noise_xz(position.xz);
    return vec3<f32>(n.x, length(position) - clouds.sun.w, n.y);
}

// Where the weather is read for a planet-frame position: its offset from the camera in the scene's metres, which
// `weather_texel` lays on the cloud anchors. (The planet's centre stands under the camera, on the scene's axes.)
fn weather_uv(position: vec3<f32>) -> vec2<f32> {
    return position.xz;
}

// Anchor `i`'s chart at the offset `d` from the camera: its second-order map (`naval_sim::clouds::laying::AnchorMap`).
fn anchor_point(i: u32, d: vec2<f32>) -> vec2<f32> {
    let v = clouds.anchor_values[i];
    let j = clouds.anchor_jacobians[i];
    let h = clouds.anchor_hessians[i];
    let east = v.x + j.x * d.x + j.y * d.y + 0.5 * (h.x * d.x * d.x + 2.0 * h.y * d.x * d.y + h.z * d.y * d.y);
    let south = v.y + j.z * d.x + j.w * d.y + 0.5 * (h.w * d.x * d.x + 2.0 * v.z * d.x * d.y + v.w * d.y * d.y);
    return vec2<f32>(east, south);
}

// The weather texel at the offset `d` from the camera (`weather_uv`), read at the tile's level `mip_level`: one anchor
// reads its tile; more read the composite the weather pass laid this frame (`naval_clouds::laying`, one fetch), and
// past it, or at a coarser level than it holds, each anchor's tile (`weather_texel_direct`).
fn weather_texel(d: vec2<f32>, mip_level: f32) -> vec4<f32> {
    if u32(clouds.anchors.x) <= 1u {
        return anchor_texel(0u, d, mip_level);
    }
    // A bilinear read at level `m` takes texels up to 2^m / 2 of the finest from the point: four of them stay inside.
    let near = composite_uv(d, COMPOSITE_NEAR_SIZE * COMPOSITE_NEAR_TEXEL, true, 4.0 / COMPOSITE_NEAR_SIZE);
    if mip_level <= COMPOSITE_TOP && near.z > 0.5 {
        return textureSampleLevel(composite_near, clamp_sampler, near.xy, mip_level);
    }
    let far = composite_uv(d, COMPOSITE_FAR_SIZE * 2.0 * COMPOSITE_NEAR_TEXEL, false, 4.0 / COMPOSITE_FAR_SIZE);
    let far_level = mip_level - 1.0;
    if far_level >= 0.0 && far_level <= COMPOSITE_TOP && far.z > 0.5 {
        return textureSampleLevel(composite_far, clamp_sampler, far.xy, far_level);
    }
    return weather_texel_direct(d, mip_level);
}

// Where `d` lies on a composite `extent` metres across (the near one, else the far), and z 1 where a read there with
// `margin` (a share of the composite) about it falls wholly within it and on rows one laying holds: the rows the
// latest laying has reached by its corners (`clouds.composite`), the rest by the laying before's (`composite_last`).
// Across the boundary between them, or near an edge, z is 0 and the weather is read anchor by anchor.
fn composite_uv(d: vec2<f32>, extent: f32, near: bool, margin: f32) -> vec3<f32> {
    let boundary = clouds.composite_slices.z;
    let latest = (d - select(clouds.composite.zw, clouds.composite.xy, near)) / extent;
    if all(latest > vec2<f32>(margin)) && latest.x < 1.0 - margin && latest.y < min(boundary, 1.0) - margin {
        return vec3<f32>(latest, 1.0);
    }
    let last = (d - select(clouds.composite_last.zw, clouds.composite_last.xy, near)) / extent;
    let inside = all(last < vec2<f32>(1.0 - margin)) && last.x > margin && last.y > max(boundary, 0.0) + margin;
    return vec3<f32>(last, select(0.0, 1.0, inside));
}

// The composites' sizes (texels), the near one's texel (m: the tile's) and the highest level each holds
// (`naval_clouds::laying`).
const COMPOSITE_NEAR_SIZE: f32 = 2048.0;
const COMPOSITE_FAR_SIZE: f32 = 1536.0;
const COMPOSITE_NEAR_TEXEL: f32 = 195.3125;
const COMPOSITE_TOP: f32 = 2.0;

// The weather texel at `d` read anchor by anchor: each cloud anchor's texture where `d` lies on its chart, blended by
// the weights as the simulation blends them (`naval_sim::clouds::blend`): one anchor alone is itself; more go through
// the tables' Gaussian values, summed keeping their mean and variance and brought back, with the tables' round trip
// taken off in the weights' proportion. What the weather pass lays in the composite.
fn weather_texel_direct(d: vec2<f32>, mip_level: f32) -> vec4<f32> {
    let count = u32(clouds.anchors.x);
    if count <= 1u {
        return anchor_texel(0u, d, mip_level);
    }
    let w = textureSampleLevel(anchor_weights, clamp_sampler, (d + clouds.anchors.zw) * clouds.anchors.y, 0.0);
    let mean = weather_tables.mean;
    var sum = vec4<f32>(0.0);
    var round_trip = vec4<f32>(0.0);
    var total = 0.0;
    var squares = 0.0;
    var held = 0u;
    var alone = vec4<f32>(0.0);
    for (var i = 0u; i < min(count, 4u); i++) {
        if w[i] > 0.0 {
            let v = anchor_texel(i, d, mip_level);
            let g = gauss_of(v);
            sum += w[i] * (g - mean);
            round_trip += w[i] * (v - value_of(g));
            total += w[i];
            squares += w[i] * w[i];
            held += 1u;
            alone = v;
        }
    }
    if held <= 1u {
        return alone;
    }
    return saturate4(value_of(mean + sum / sqrt(squares)) + round_trip / total);
}

// A texel's Gaussian values (the blend's forward table, linear between levels), a channel at a time.
fn gauss_of(v: vec4<f32>) -> vec4<f32> {
    let x = clamp(v, vec4<f32>(0.0), vec4<f32>(1.0)) * 255.0;
    let k = min(vec4<u32>(x), vec4<u32>(254u));
    let f = x - vec4<f32>(k);
    let t = &weather_tables.forward;
    return vec4<f32>(
        mix((*t)[k.x].x, (*t)[k.x + 1u].x, f.x),
        mix((*t)[k.y].y, (*t)[k.y + 1u].y, f.y),
        mix((*t)[k.z].z, (*t)[k.z + 1u].z, f.z),
        mix((*t)[k.w].w, (*t)[k.w + 1u].w, f.w),
    );
}

// Gaussian values' texel values (the blend's inverse table, linear between entries), a channel at a time.
fn value_of(g: vec4<f32>) -> vec4<f32> {
    let y = clamp((g + 5.0) / 10.0, vec4<f32>(0.0), vec4<f32>(1.0)) * 1023.0;
    let j = min(vec4<u32>(y), vec4<u32>(1022u));
    let f = y - vec4<f32>(j);
    let t = &weather_tables.inverse;
    return vec4<f32>(
        mix((*t)[j.x].x, (*t)[j.x + 1u].x, f.x),
        mix((*t)[j.y].y, (*t)[j.y + 1u].y, f.y),
        mix((*t)[j.z].z, (*t)[j.z + 1u].z, f.z),
        mix((*t)[j.w].w, (*t)[j.w + 1u].w, f.w),
    );
}

fn anchor_texel(i: u32, d: vec2<f32>, mip_level: f32) -> vec4<f32> {
    return textureSampleLevel(local_weather, repeat_sampler, anchor_point(i, d) * clouds.repeats.x, mip_level);
}

// True between layers, where the march strides over empty air.
fn inside_layer_intervals(height: f32) -> bool {
    let gt = vec3<f32>(height) > clouds.min_interval_heights.xyz;
    let lt = vec3<f32>(height) < clouds.max_interval_heights.xyz;
    return any(gt & lt);
}

struct WeatherSample {
    // Each layer's normalised height.
    height_fraction: vec4<f32>,
    density: vec4<f32>,
}

// A semicircle that rounds the clouds toward their tops.
fn shape_altering(height_fraction: vec4<f32>, bias: vec4<f32>) -> vec4<f32> {
    let biased = pow(height_fraction, bias);
    let x = clamp(biased * 2.0 - 1.0, vec4<f32>(-1.0), vec4<f32>(1.0));
    return 1.0 - x * x;
}

// The weather of every layer at a point; `shadow` keeps only the layers that cast shadows.
fn sample_weather(uv: vec2<f32>, height: f32, mip_level: f32, shadow: bool) -> WeatherSample {
    var weather: WeatherSample;
    weather.height_fraction = remap_clamped4(vec4<f32>(height), clouds.min_layer_heights, clouds.max_layer_heights);
    let texel = weather_texel(uv, mip_level);
    let local = pow(texel * clouds.channels, clouds.weather_exponents);
    let height_scale = shape_altering(weather.height_fraction, clouds.shape_altering_biases);
    // Modulation that lets the coverage control the weather (after Skybolt's `Clouds.h`), each layer by its own.
    let factor = 1.0 - clouds.layer_coverages * height_scale;
    weather.density = remap_clamped4(
        mix(local, vec4<f32>(1.0), clouds.coverage_filter_widths),
        factor,
        factor + clouds.coverage_filter_widths,
    );
    // The mask after the coverage (takram masks the weather before it, which a high coverage mixes back toward one, so
    // a layer that casts no shadow still shaded under it).
    if shadow {
        weather.density *= clouds.shadow_layer_mask;
    }
    return weather;
}

fn layer_density(height_fraction: vec4<f32>) -> vec4<f32> {
    return clouds.density_exp_terms * exp(clouds.density_exponents * height_fraction)
        + clouds.density_linear_terms * height_fraction
        + clouds.density_constant_terms;
}

struct MediaSample {
    density: f32,
    // Each layer's share of the density.
    weight: vec4<f32>,
    scattering: f32,
    extinction: f32,
}

// ---- The detail ---------------------------------------------------------------------------------------------------
//
// The clouds' structure from a few tens of metres down, carved out of their edges with the detail noise
// (`naval_clouds::detail_noise`: one 128³ volume tiling every `1 / repeats.z` metres, 100 m; r and g low- and
// high-frequency wisps, b and a low- and high-frequency billows, each stored bumps bright and creases dark). After the
// up-rez of Guerrilla's Nubis³ (Schneider, SIGGRAPH 2023): the frequency by the depth into the cloud, wisps or billows
// by the height in the layer. Every march reads it at mip 3 or coarser (texels of 6 m, about takram's 5 m; 16 KB, which
// stays in the texture cache as takram's 32³ did, where mip 2's 131 KB cost the march half as much again), carved to
// takram's statistics, so the clouds keep their size, their soft edge and their noise. A paused or held picture reads
// the same field as play: until 2026-10-08 it read a sharper near field close to a camera in the air (the carving at
// full contrast, the inside uneven, the edge sharpened, the finer mips), which drew a near cumulus as a mottled web of
// dark creases the moment the game paused.

// Texels per edge of the detail noise.
const DETAIL_SIZE: f32 = 128.0;
// The detail's mip never holds texels finer than a DETAIL_STEP_SHARE of the march's step (detail a march steps over is
// noise; the 8 to 16 jittered samples a pixel gathers resolve what lies within a step that far) nor than
// DETAIL_PIXEL_SHARE of a pixel of the drawing buffer at the sample.
const DETAIL_STEP_SHARE: f32 = 0.125;
const DETAIL_PIXEL_SHARE: f32 = 0.5;
// The finest mip any march reads: 16³ texels of 6 m (a march at Ultra's 10 m steps reading mips 0 and 1 paid their cache
// misses, 0.7 to 3.5 ms, on every sample).
const DETAIL_COARSE_LOD: f32 = 3.0;
// What the detail carves: the mean (takram's wisps carved 0.17 of the edge on average, its billows 0.29: the clouds
// keep their size and their depth) and the contrast about it (the spread takram's had). The last is the bumps' own mean.
const WISP_MEAN: f32 = 0.17;
const BILLOW_MEAN: f32 = 0.29;
const CARVE_CONTRAST: f32 = 0.47;
const BUMP_MEAN: f32 = 0.35;

// The noise's mip for a sample `distance` metres from the camera on a march stepping `step` metres: by the step and the
// pixel, never finer than DETAIL_COARSE_LOD.
fn detail_lod(step: f32, distance: f32) -> f32 {
    let texel = 1.0 / (clouds.repeats.z * DETAIL_SIZE);
    let pixel = distance * 2.0 / (clouds.forward.w * clouds.screen.y);
    let lod = log2(max(max(step * DETAIL_STEP_SHARE, pixel * DETAIL_PIXEL_SHARE) / texel, 1.0));
    return max(lod, DETAIL_COARSE_LOD);
}

// What the detail carves of each layer from the wisps' and the billows' bumps there (0 a crease to 1 a bump's top).
fn detail_erosion(wisp_bumps: vec4<f32>, billow_bumps: vec4<f32>, billowy: vec4<f32>) -> vec4<f32> {
    let wisps = WISP_MEAN + (wisp_bumps - BUMP_MEAN) * CARVE_CONTRAST;
    let billows = BILLOW_MEAN + (BUMP_MEAN - billow_bumps) * CARVE_CONTRAST;
    return saturate4(mix(wisps, billows, billowy));
}

// Each layer's density (0 at the edge to 1 in the core, the weather carved by the shape) with the detail carved out of
// it, at `at` (the clouds' drifting frame, m), the noise read at mip `lod` (`detail_lod`).
fn carve_detail(density: vec4<f32>, height_fraction: vec4<f32>, at: vec3<f32>, lod: f32) -> vec4<f32> {
    let noise = textureSampleLevel(detail_volume, repeat_sampler, at * clouds.repeats.z, lod);
    let depth = saturate4(density);
    // Wisps at the bottom of a layer, billows at its top (takram's "fluffy at the top and whippy at the bottom").
    let billowy = remap_clamped4(height_fraction, vec4<f32>(0.2), vec4<f32>(0.4));
    // Wisps: the bumps' tops carved away, the web of creases between them the filaments that stay. Billows: the creases
    // carved away, the bumps the lobes that stay. Each the low frequency at the edge, with more of the high frequency
    // toward the core (the direction Nubis³ prints; the other way round the edge is all fine grain and no lobe shows).
    let erosion = detail_erosion(
        mix(vec4<f32>(noise.r), vec4<f32>(noise.g), depth * 0.5),
        mix(vec4<f32>(noise.b), vec4<f32>(noise.a), depth * 0.6),
        billowy,
    ) * clouds.shape_detail_amounts;
    return remap_clamped4(density * 2.0, erosion * 0.5, vec4<f32>(1.0));
}

// The participating medium at a point (`sampleMedia`), its detail read at mip `detail_lod`.
fn sample_media_at(weather: WeatherSample, position: vec3<f32>, uv: vec2<f32>, mip_level: f32, jitter: f32, detail_lod: f32) -> MediaSample {
    var density = weather.density;
    let at = noise_position(position);
    // The shape evolves as the weather moves: takram shifts it along the surface normal by the weather's offset.
    let evolution = vec3<f32>(0.0, -clouds.drift.w, 0.0);

    var turbulence = vec3<f32>(0.0);
    if clouds.switches.y > 0.5 {
        let turbulence_uv = noise_xz(uv) * clouds.repeats.x * clouds.repeats.w;
        turbulence = clouds.medium.w
            * (textureSampleLevel(turbulence_map, repeat_sampler, turbulence_uv, 0.0).rgb * 2.0 - 1.0)
            * dot(density, remap_clamped4(weather.height_fraction, vec4<f32>(0.3), vec4<f32>(0.0)));
    }

    let shape_position = (at + evolution + turbulence) * clouds.repeats.y;
    let shape = textureSampleLevel(shape_volume, repeat_sampler, shape_position, 0.0).r;
    density = remap_clamped4(density, vec4<f32>(1.0 - shape) * clouds.shape_amounts, vec4<f32>(1.0));

    if clouds.switches.x > 0.5 && mip_level * 0.5 + (jitter - 0.5) * 0.5 < 0.5 {
        density = carve_detail(density, weather.height_fraction, at + turbulence, detail_lod);
    }

    // The density profiles.
    density = saturate4(density * clouds.density_scales * layer_density(weather.height_fraction));

    var media: MediaSample;
    let sum = density.x + density.y + density.z + density.w;
    media.density = sum;
    media.weight = density / max(sum, 1e-12);
    media.scattering = sum * clouds.medium.x;
    media.extinction = sum * clouds.medium.y + media.scattering;
    return media;
}

// The same for a march toward the light: the detail at its coarsest mip, so the light stands still as the camera moves.
fn sample_media(weather: WeatherSample, position: vec3<f32>, uv: vec2<f32>, mip_level: f32, jitter: f32) -> MediaSample {
    return sample_media_at(weather, position, uv, mip_level, jitter, DETAIL_COARSE_LOD);
}

// The share of the coverage-filtered weather's density the shape and detail noise leave on average, for estimates that
// read the weather alone (`column_above`).
const SHAPE_FILL: f32 = 0.5;

// The clouds straight above a point: the optical depth of every layer over it, from the point (or the layer's bottom)
// to the layer's top, its weather read at the point's column at a quarter and three quarters of that span; and how much
// of a closed slab lies over it, 0 none to 1 a deck or fog (the largest slab weight of the layers above,
// `naval_clouds::layers::Layer::slab`). For the light that diffuses down through a deck
// (`naval_clouds::march::deck_skylight`).
struct ColumnAbove {
    optical_depth: f32,
    closed: f32,
}

// Whether any layer is a slab: fair weather (takram's layers) skips `column_above`.
fn any_deck() -> bool {
    return any(clouds.layer_slabs > vec4<f32>(0.0));
}

fn column_above(uv: vec2<f32>, height: f32, mip_level: f32) -> ColumnAbove {
    let texel = weather_texel(uv, mip_level);
    let local = pow(texel * clouds.channels, clouds.weather_exponents);
    let filtered = mix(local, vec4<f32>(1.0), clouds.coverage_filter_widths);
    let bottom = max(vec4<f32>(height), clouds.min_layer_heights);
    let top = clouds.max_layer_heights;
    let extent = max(top - bottom, vec4<f32>(0.0));
    var density = vec4<f32>(0.0);
    for (var i = 0; i < 2; i++) {
        let at = bottom + extent * (0.25 + 0.5 * f32(i));
        let fraction = remap_clamped4(at, clouds.min_layer_heights, clouds.max_layer_heights);
        let factor = 1.0 - clouds.layer_coverages * shape_altering(fraction, clouds.shape_altering_biases);
        let weather = remap_clamped4(filtered, factor, factor + clouds.coverage_filter_widths);
        density += 0.5 * saturate4(weather * clouds.density_scales * layer_density(fraction));
    }
    let extinction = density * SHAPE_FILL * (clouds.medium.x + clouds.medium.y);
    let closure = select(vec4<f32>(0.0), clouds.layer_slabs, extent > vec4<f32>(0.0));
    return ColumnAbove(dot(extinction, extent), max(max(closure.x, closure.y), max(closure.z, closure.w)));
}

// The weather texture's mip for a ray's start (takram's `getMipLevel` of the globe UV's screen derivatives, taken here
// from the footprint of one pixel there): `distance` (m) along a ray at `slant` (|cos| between the ray and the vertical)
// from a camera whose pixels each span `pixel_angle` radians.
fn weather_mip_level(distance: f32, slant: f32, pixel_angle: f32) -> f32 {
    let footprint = distance * pixel_angle / max(slant, 0.05);
    // Weather tiles per pixel of the march buffer (a quarter of the drawing buffer's), in units of the buffer.
    let delta = footprint * clouds.repeats.x * clouds.screen.y * 4.0;
    let delta_max_sqr = delta * delta * 0.1;
    return max(0.0, 0.5 * log2(max(1.0, delta_max_sqr)));
}
