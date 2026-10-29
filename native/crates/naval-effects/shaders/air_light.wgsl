// The light in the air at night (naval-effects' air_light.rs, which says what is drawn and why): what the haze, the
// rain and the smoke screens scatter toward the eye from the searchlights' beams and the burning star shells, and what
// those lights show of themselves (the lamp seen down its beam, the flare's flame, the smoke trailing above it, the
// cloud base it lights). One triangle over the screen, added to the scene after every other effect. No TypeScript
// source: the web game drew no light in the air.
//
// Single scattering along each view ray up to what it hits: σ_s(x) p(θ) I / r² × what the air (and a beam's smoke
// screens) let through from the light to x × what the air, the clouds and the gas let through from x to the eye. The
// integral is taken over the scattering angle θ about the light (θ = π/2 + atan((t − t_c) / D), D the light's distance
// from the ray, t_c where the ray passes it): dt = r² / D dθ takes the inverse square out, and samples spaced evenly in
// P(θ) = ∫ p dθ (the tables) take the phase out, so a few samples carry what is left, which varies slowly.
#import naval::camera::{camera, curved, view_depth, ndc_from_pixel, view_ray}
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::aerial_volume::aerial_sample
#import naval::aerial::{aerial_volume, aerial_sampler}
#import naval_effects::common::{lighting, scene_depth, occlusion_fronts, occlusion_depths, occlusion_sampler, cloud_cover, clouds_before, distance_of, nearness_of, sstep}
#import naval_effects::smoke_screen::screen_coord

struct Beam {
    // xyz the lamp (world m), w the light's reach (m; 0 none).
    origin: vec4<f32>,
    // xyz the unit axis, w tan² of the outer half angle.
    axis: vec4<f32>,
    // rgb colour × candela, w the near field's radius (m).
    color: vec4<f32>,
    // x cos of the outer half angle, y of the inner, z the first of its screens, w their count.
    cone: vec4<f32>,
}

struct Flare {
    // xyz the burning candle (world m), w the light's reach (m; 0 none).
    position: vec4<f32>,
    // rgb colour × candela, w the flame's radius (m).
    color: vec4<f32>,
    // x the trail's length (m), y σ ρ² of its smoke (m), z its points, w 0.
    trail: vec4<f32>,
    // xyz the candle in the air (m from where it lit, the wind's drift taken out), w the billows' seed.
    air: vec4<f32>,
    // xyz the centre of a sphere about the flame and its smoke (m from the candle), w its radius (m).
    bound: vec4<f32>,
    // The trail's points from the candle up (`Flare::trail`): xyz from the candle (m), w the smoke's spread (ρ², m²).
    points: array<vec4<f32>, 28>,
}

struct Screen {
    // xyz the centre (world m), w the horizontal radius (m).
    centre: vec4<f32>,
    // x the vertical half height (m), y the bulk extinction (per m), z the age (s), w the billows' seed.
    shape: vec4<f32>,
}

struct AirLight {
    // Beams, flares, screens, and 1 when the halo was drawn this frame.
    counts: vec4<u32>,
    // x the haze's extinction at the sea (per m), y the layer's top, z its scale height, w the taper's (m).
    haze: vec4<f32>,
    // x the haze's albedo, y the rain's extinction at the sea (per m), z the rain's top (m), w the atmosphere's
    // extinction at the sea (per m).
    medium: vec4<f32>,
    // x the haze's asymmetry, y P(π), z the cloud base's albedo, w the screens' albedo.
    phase: vec4<f32>,
    // x the trail's albedo, y its asymmetry, z a pixel's angle (rad).
    trail: vec4<f32>,
    // P(θ) at 256 angles from 0 to π, at π (k / 255)² (`table_angle`).
    cdf: array<vec4<f32>, 64>,
    beams: array<Beam, 8>,
    flares: array<Flare, 8>,
    screens: array<Screen, 32>,
}

@group(2) @binding(0) var<uniform> air: AirLight;
// The fire's Worley volume, which the smoke screens' billows are rolled from (`gas_sprite.wgsl`).
@group(2) @binding(1) var screen_noise: texture_3d<f32>;
@group(2) @binding(2) var screen_sampler: sampler;
// The star shells' halo at half the resolution (`halo_fragment`): the composite's only (the halo pass draws into it).
@group(2) @binding(3) var halo: texture_2d<f32>;

const PI: f32 = 3.141592653589793;
const HALF_PI: f32 = 1.5707963267948966;
// Samples along a ray in the air about a beam's lamp (its edge and screens vary along it) and a star shell (its air
// varies slowly), and through a smoke screen in a beam.
const BEAM_SAMPLES: u32 = 4u;
const FLARE_SAMPLES: u32 = 3u;
const SCREEN_SAMPLES: u32 = 4u;
// The share of a screen's radii its billows fill (`screen_density` feathers the outer tenth or so to nothing).
const SCREEN_FILL: f32 = 0.85;
// The scale height (m) over which rain thins above the clouds' base (naval-clouds' aerial.wgsl).
const RAIN_TAPER: f32 = 100.0;
// The brightest a pixel is given (the scene's half floats hold 65504).
const CEILING: f32 = 30000.0;
// The farthest distance the halo's alpha holds (m, under a half float's 65504): the sky, and all beyond it, alike.
const HALO_FAR: f32 = 60000.0;

// What a pixel reads once: where it looks, how far it sees, the gas the occlusion map holds over it.
struct View {
    pixel: vec2<f32>,
    uv: vec2<f32>,
    ray: vec3<f32>,
    // Metres along the ray to what it hits (far beyond everything for the sky).
    hit: f32,
    fronts: vec4<f32>,
    depths: vec4<f32>,
    // The rain's share of the haze.
    rain_share: f32,
    // 0–1, moved each frame for temporal anti-aliasing.
    jitter: f32,
    // The ray's cosine to the view's axis: depth along the axis per metre along the ray.
    axial: f32,
    // The opacity of the clouds the composite drew over the pixel (`cloud_cover`), read once.
    cloud: f32,
}

// ---- The air ----

fn aerosol_density(height: f32) -> f32 {
    let h = max(height, 0.0);
    let top = air.haze.y;
    if h <= top {
        return exp(-h / air.haze.z);
    }
    return exp(-top / air.haze.z) * exp(-(h - top) / air.haze.w);
}

fn rain_density(height: f32) -> f32 {
    let h = max(height, 0.0);
    if h <= air.medium.z {
        return 1.0;
    }
    return exp(-(h - air.medium.z) / RAIN_TAPER);
}

// The haze's density at `height` over its density at the sea: the aerosol's and the rain's in their shares.
fn haze_density(height: f32, rain_share: f32) -> f32 {
    return mix(aerosol_density(height), rain_density(height), rain_share);
}

// What the air scatters (per m) at `height`.
fn scattering(height: f32, rain_share: f32) -> f32 {
    return air.medium.x * air.haze.x * haze_density(height, rain_share) + air.medium.w;
}

// The air's optical depth along a straight path of `length` m between two heights, at the density half way up: exact
// in rain (even up to the clouds' base), and within a fifth of the haze's over the 600 m a star shell hangs, where the
// whole path's depth is a few hundredths.
fn air_depth(from_height: f32, to_height: f32, length: f32, rain_share: f32) -> f32 {
    return (air.haze.x * haze_density(0.5 * (from_height + to_height), rain_share) + air.medium.w) * length;
}

// The height above the sea of a point the camera draws `offset` from itself (undoing the drawn curvature).
fn height_of(offset: vec3<f32>) -> f32 {
    return camera.position.y + offset.y + camera.curvature * dot(offset.xz, offset.xz);
}

// Cornette–Shanks.
fn phase_of(g: f32, mu: f32) -> f32 {
    let g2 = g * g;
    return 3.0 / (8.0 * PI) * (1.0 - g2) * (1.0 + mu * mu) / ((2.0 + g2) * pow(max(1.0 + g2 - 2.0 * g * mu, 1e-6), 1.5));
}

fn cdf_entry(i: u32) -> f32 {
    return air.cdf[i >> 2u][i & 3u];
}

// The table's last index: its entries stand at the angles π (k / LAST)² (`naval_effects::air_light::table_angle`).
const LAST: u32 = 255u;

// The table's index (fractional) at the angle `theta`.
fn table_index(theta: f32) -> f32 {
    return sqrt(clamp(theta / PI, 0.0, 1.0)) * f32(LAST);
}

// The angle at the table's (fractional) index `x`.
fn table_angle(x: f32) -> f32 {
    let s = x / f32(LAST);
    return PI * s * s;
}

// P(θ), the phase's integral over the angle from 0: linear between the table's entries in their index.
fn cdf_at(theta: f32) -> f32 {
    let x = table_index(theta);
    let i = min(u32(x), LAST - 1u);
    return mix(cdf_entry(i), cdf_entry(i + 1u), x - f32(i));
}

// The phase the table holds at `theta`: its step's rise over its span of angle, what `angle_at`'s samples follow.
fn held_phase(theta: f32) -> f32 {
    let i = min(u32(table_index(theta)), LAST - 1u);
    let span = table_angle(f32(i + 1u)) - table_angle(f32(i));
    return (cdf_entry(i + 1u) - cdf_entry(i)) / max(span, 1e-12);
}

// The angle at which P reaches `p`: `cdf_at` inverted exactly (a search of the table, then its line), so the samples
// follow the same piecewise phase the span was measured with, however wide or narrow the span.
fn angle_at(p: f32) -> f32 {
    var low = 0u;
    var high = LAST;
    for (var step = 0u; step < 8u; step++) {
        let middle = (low + high) / 2u;
        if cdf_entry(middle) <= p {
            low = middle;
        } else {
            high = middle;
        }
    }
    let a = cdf_entry(low);
    let b = cdf_entry(low + 1u);
    return table_angle(f32(low) + clamp((p - a) / max(b - a, 1e-12), 0.0, 1.0));
}

// three's window on a light's reach (`naval::lights::distance_attenuation` without the falloff).
fn reach_window(distance: f32, reach: f32) -> f32 {
    if reach <= 0.0 {
        return 1.0;
    }
    let ratio = distance / reach;
    let window = clamp(1.0 - ratio * ratio * ratio * ratio, 0.0, 1.0);
    return window * window;
}

// ---- In front of the eye ----

// What the occlusion map's gas lets through to `distance` m along the ray (`smoke_through` with its reads made once; the
// map holds depths along the view's axis, as `smoke_through` takes them).
fn gas_through(view: View, distance: f32) -> f32 {
    // No gas this frame, or none over this pixel.
    if lighting.direct.w <= 0.0 || (view.fronts.r <= 0.0 && view.depths.r <= 0.0) {
        return 1.0;
    }
    let depth = distance * view.axial;
    var visible = 1.0;
    // How deep into a level's gas its light is seen from: about the screens' 1 / (0.025 per metre), less their gaps.
    let reach = 30.0;
    let slack = reach * 0.1 + 0.5;
    let levels = vec4<f32>(0.25, 0.5, 0.8, 0.95);
    for (var i = 0; i < 4; i++) {
        let through = clamp((depth - distance_of(view.fronts[i]) - slack) / reach, 0.0, 1.0);
        visible = min(visible, exp2(through * log2(1.0 - levels[i])));
    }
    let optical = view.depths.r;
    let centre = view.depths.g / max(optical, 1e-4);
    let nearest = max(view.fronts.r, centre);
    let here = nearness_of(depth);
    let share = clamp((nearest - here) / max((nearest - centre) * 2.0, 1e-7), 0.0, 1.0);
    return min(visible, exp(-optical * share));
}

// What the clouds the composite drew let through to `distance` m along the ray (`cloud_visibility` with the pixel's
// cover read once): the share of the ray's metres in the clouds that lies in front takes that share of their opacity;
// a point under the clouds seen from under them keeps all of itself.
fn clouds_through(view: View, distance: f32) -> f32 {
    if lighting.clouds.w <= 0.0 {
        return 1.0;
    }
    return clouds_before(view.cloud, view.ray, distance, select(0.0, view.hit, view.hit < 1e7));
}

// What reaches the eye of light scattered `distance` m along the ray: the air (the aerial volume), the clouds and the
// gas in front.
fn to_eye(view: View, distance: f32) -> vec3<f32> {
    let aerial = aerial_sample(aerial_volume, aerial_sampler, view.uv, distance).transmittance;
    return aerial * (clouds_through(view, distance) * gas_through(view, distance));
}

// ---- The smoke screens in a beam ----

// The billows `gas_sprite.wgsl` draws the screen with (`screen_density`), so the beam lights the smoke one sees.
fn screen_density(local: vec3<f32>, radii: vec3<f32>, age: f32, seed: f32) -> f32 {
    let noise = textureSampleLevel(screen_noise, screen_sampler, screen_coord(local, radii, age, seed), 0.0).rgb;
    let billow = dot(noise, vec3<f32>(0.58, 0.28, 0.14));
    let inside = 1.0 - length(local);
    let shape = inside - 0.1 + (billow - 0.35) * 0.4;
    return sstep(0.0, 0.12, shape) * sstep(0.0, 0.06, inside) * (0.8 + noise.g * 0.8);
}

fn screen_radii(screen: Screen) -> vec3<f32> {
    return vec3<f32>(screen.centre.w, max(screen.shape.x, 0.1), screen.centre.w);
}

// The span (metres from `start` along the unit `direction`) inside the screen's ellipsoid: x enter, y leave.
fn screen_span(screen: Screen, start: vec3<f32>, direction: vec3<f32>) -> vec2<f32> {
    return screen_span_within(screen, start, direction, 1.0);
}

// As `screen_span`, for the ellipsoid shrunk to `fill` of its radii.
fn screen_span_within(screen: Screen, start: vec3<f32>, direction: vec3<f32>, fill: f32) -> vec2<f32> {
    let radii = screen_radii(screen) * fill;
    let o = (start - (curved(screen.centre.xyz) - camera.position)) / radii;
    let d = direction / radii;
    let a = dot(d, d);
    let b = dot(o, d);
    let c = dot(o, o) - 1.0;
    let disc = b * b - a * c;
    if disc <= 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    let root = sqrt(disc);
    return vec2<f32>((-b - root) / a, (-b + root) / a);
}

// The screens' bulk optical depth along the path from the lamp (`lamp`, eye-relative) to `point`. The billows feather
// the authoritative rim away (`screen_density`), so the light is taken through the filled heart, `SCREEN_FILL` of the
// radii: a beam grazing a screen's thin edge is not cut by smoke that is not drawn.
fn screens_depth(beam: Beam, lamp: vec3<f32>, point: vec3<f32>) -> f32 {
    let first = u32(beam.cone.z);
    let count = u32(beam.cone.w);
    let path = point - lamp;
    let length_total = length(path);
    if count == 0u || length_total < 1e-3 {
        return 0.0;
    }
    let direction = path / length_total;
    var depth = 0.0;
    for (var k = 0u; k < count; k++) {
        let screen = air.screens[first + k];
        let span = screen_span_within(screen, lamp, direction, SCREEN_FILL);
        let inside = min(span.y, length_total) - max(span.x, 0.0);
        depth += max(inside, 0.0) * screen.shape.y;
    }
    return depth;
}

// ---- A light's air ----

// Where the ray passes the light `light` (eye-relative): along it, and how far off.
struct Passing {
    along: f32,
    off: f32,
}

fn passing(view: View, light: vec3<f32>) -> Passing {
    let along = dot(light, view.ray);
    return Passing(along, length(light - view.ray * along));
}

// A beam's share of its light toward `point` (eye-relative), from its lamp at `lamp`: the cone's feathered edge.
fn beam_profile(beam: Beam, lamp: vec3<f32>, point: vec3<f32>) -> f32 {
    let toward = point - lamp;
    let cosine = dot(toward, beam.axis.xyz) / max(length(toward), 1e-4);
    return smoothstep(beam.cone.x, beam.cone.y, cosine);
}

// The air's in-scatter from a light of `color` (rgb × candela) at `light` (eye-relative, world `light_height`) over
// [t0, t1] along the view ray. `near` is its near field's radius (m), `reach` its window; a beam (`beam_index` ≥ 0)
// also takes its cone's edge and its screens.
fn air_light(view: View, samples: u32, light: vec3<f32>, light_height: f32, color: vec3<f32>, near: f32, reach: f32, t0: f32,
    t1: f32, beam_index: i32) -> vec3<f32> {
    let pass_by = passing(view, light);
    let g = air.phase.x;
    var sum = vec3<f32>(0.0);
    // Far enough off the ray that the light falls as the inverse square all along it: samples even in P(θ).
    let in_angle = pass_by.off > near;
    var span = 0.0;
    var a = 0.0;
    var scale = 0.0;
    if in_angle {
        a = cdf_at(HALF_PI + atan((t0 - pass_by.along) / pass_by.off));
        let b = cdf_at(HALF_PI + atan((t1 - pass_by.along) / pass_by.off));
        span = b - a;
        scale = span / pass_by.off;
    } else {
        // Through the near field (the ray passes the lamp's mirror or the flame): even in angle about it with the
        // near field's radius taken in, the phase taken at each sample.
        let off = sqrt(pass_by.off * pass_by.off + near * near);
        a = atan((t0 - pass_by.along) / off);
        span = atan((t1 - pass_by.along) / off) - a;
        scale = span / off;
    }
    if span <= 0.0 {
        return vec3<f32>(0.0);
    }
    for (var k = 0u; k < samples; k++) {
        let u = (f32(k) + view.jitter) / f32(samples);
        var t: f32;
        var weight: f32;
        let off2 = pass_by.off * pass_by.off;
        if in_angle {
            let theta = angle_at(a + u * span);
            t = pass_by.along + pass_by.off * tan(theta - HALF_PI);
            let r2 = off2 + (t - pass_by.along) * (t - pass_by.along);
            weight = r2 / (r2 + near * near);
            if beam_index >= 0 {
                // The samples follow the table's phase, constant over each of its steps; weighed by the true phase over
                // it, a beam's glow follows the true one, and the steps it changes at leave no rings about the lamp seen
                // from inside the beam. The beam's samples move each frame, so the temporal filter resolves what the
                // weights scatter; the star shells' halo, held still, keeps the table's.
                weight *= phase_of(g, cos(theta)) / max(held_phase(theta), 1e-12);
            }
        } else {
            let off = sqrt(off2 + near * near);
            t = pass_by.along + off * tan(a + u * span);
            let r = sqrt(off2 + (t - pass_by.along) * (t - pass_by.along));
            weight = phase_of(g, -(t - pass_by.along) / max(r, 1e-4));
        }
        t = clamp(t, t0, t1);
        let point = view.ray * t;
        let r = distance(point, light);
        let height = height_of(point);
        var depth = air_depth(light_height, height, r, view.rain_share);
        var share = reach_window(r, reach) * weight;
        if beam_index >= 0 {
            let beam = air.beams[beam_index];
            share *= beam_profile(beam, light, point);
            depth += screens_depth(beam, light, point);
        }
        sum += to_eye(view, t) * (scattering(height, view.rain_share) * share * exp(-depth));
    }
    return color * sum * (scale / f32(samples));
}

// A beam's light scattered by the smoke screens it passes through, inside [t0, t1] of the view ray.
fn beam_screens(view: View, beam: Beam, lamp: vec3<f32>, t0: f32, t1: f32) -> vec3<f32> {
    let first = u32(beam.cone.z);
    let count = u32(beam.cone.w);
    var sum = vec3<f32>(0.0);
    let g = air.phase.x;
    for (var k = 0u; k < count; k++) {
        let screen = air.screens[first + k];
        let span = screen_span(screen, vec3<f32>(0.0), view.ray);
        let s0 = max(span.x, t0);
        let s1 = min(span.y, t1);
        if s1 <= s0 {
            continue;
        }
        let radii = screen_radii(screen);
        let centre = curved(screen.centre.xyz) - camera.position;
        let stride = (s1 - s0) / f32(SCREEN_SAMPLES);
        // What the air and the screens take from the lamp's light on its way in, at the chord's two ends and between
        // them in proportion: twice a chord rather than at every sample.
        let near_end = view.ray * s0;
        let far_end = view.ray * s1;
        let depths = vec2<f32>(
            air_depth(height_of(lamp), height_of(near_end), distance(near_end, lamp), view.rain_share)
                + screens_depth(beam, lamp, near_end),
            air_depth(height_of(lamp), height_of(far_end), distance(far_end, lamp), view.rain_share)
                + screens_depth(beam, lamp, far_end));
        for (var i = 0u; i < SCREEN_SAMPLES; i++) {
            let t = s0 + (f32(i) + view.jitter) * stride;
            let point = view.ray * t;
            let local = (point - centre) / radii;
            let density = screen_density(local, radii, screen.shape.z, screen.shape.w);
            if density <= 0.0 {
                continue;
            }
            let toward = point - lamp;
            let r2 = dot(toward, toward);
            let mu = -dot(toward, view.ray) * inverseSqrt(max(r2, 1e-8));
            let share = beam_profile(beam, lamp, point) * reach_window(sqrt(r2), beam.origin.w) * phase_of(g, mu)
                / (r2 + beam.color.w * beam.color.w);
            let through = exp(-mix(depths.x, depths.y, (f32(i) + view.jitter) / f32(SCREEN_SAMPLES)));
            sum += to_eye(view, t) * (air.phase.w * screen.shape.y * density * share * through * stride);
        }
    }
    return beam.color.rgb * sum;
}

// The ray's span inside a beam's cone (from its lamp out to its reach) and in front of what it hits: x enter, y leave.
fn beam_span(view: View, beam: Beam, lamp: vec3<f32>) -> vec2<f32> {
    let axis = beam.axis.xyz;
    let tan2 = beam.axis.w;
    let w = -lamp;
    let va = dot(view.ray, axis);
    let wa = dot(w, axis);
    let vp = view.ray - axis * va;
    let wp = w - axis * wa;
    // Inside where tan²α (along the axis)² ≥ (off it)², along ≥ 0.
    let qa = tan2 * va * va - dot(vp, vp);
    let qb = 2.0 * (tan2 * wa * va - dot(wp, vp));
    let qc = tan2 * wa * wa - dot(wp, wp);
    var t0 = 0.0;
    var t1 = view.hit;
    // The forward nappe.
    if abs(va) > 1e-7 {
        let apex = -wa / va;
        if va > 0.0 { t0 = max(t0, apex); } else { t1 = min(t1, apex); }
    } else if wa < 0.0 {
        return vec2<f32>(1.0, 0.0);
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if qa > 0.0 {
        // Steeper than the cone's side: inside beyond the roots, one nappe each way.
        if disc > 0.0 {
            let q = -0.5 * (qb + select(-1.0, 1.0, qb >= 0.0) * sqrt(disc));
            let r0 = min(q / qa, qc / q);
            let r1 = max(q / qa, qc / q);
            if va > 0.0 { t0 = max(t0, r1); } else { t1 = min(t1, r0); }
        }
    } else {
        if disc <= 0.0 {
            return vec2<f32>(1.0, 0.0);
        }
        let q = -0.5 * (qb + select(-1.0, 1.0, qb >= 0.0) * sqrt(disc));
        var r0 = q / min(qa, -1e-12);
        var r1 = qc / select(q, 1e-12, abs(q) < 1e-12);
        let low = min(r0, r1);
        let high = max(r0, r1);
        t0 = max(t0, low);
        t1 = min(t1, high);
    }
    // Out to its reach.
    let reach = beam.origin.w;
    if reach > 0.0 {
        let vw = dot(view.ray, w);
        let s = vw * vw - (dot(w, w) - reach * reach);
        if s <= 0.0 {
            return vec2<f32>(1.0, 0.0);
        }
        let root = sqrt(s);
        t0 = max(t0, -vw - root);
        t1 = min(t1, -vw + root);
    }
    return vec2<f32>(t0, t1);
}

// A small bright source of `radius` m and luminous `intensity` (rgb × candela) at `at` (eye-relative), drawn with its
// light kept however small it stands on the screen: spread over at least a pixel.
fn source(view: View, at: vec3<f32>, radius: f32, intensity: vec3<f32>) -> vec3<f32> {
    let distance_to = length(at);
    if distance_to > view.hit || distance_to < 1e-3 {
        return vec3<f32>(0.0);
    }
    // The chord between the two directions: the angle, without `1 − cos`'s loss of precision at a lens's small ones.
    let angle = length(at / distance_to - view.ray);
    let size = radius / distance_to;
    let foot = max(size, air.trail.z * 0.75);
    let shape = 1.0 - smoothstep(0.5 * foot, 1.5 * foot, angle);
    if shape <= 0.0 {
        return vec3<f32>(0.0);
    }
    // Its luminance I / (π r²) over its own disc, spread over the footprint (the shape's solid angle, π foot² × 1.05).
    let luminance = intensity / (PI * radius * radius) * (size * size) / (foot * foot * 1.05);
    return luminance * shape * to_eye(view, distance_to);
}

// The trail's points at most (air_light.rs's `TRAIL_POINTS`).
const TRAIL_POINTS: u32 = 28u;
// The billows' tile in the air (m): the Worley volume's 4 and 8 cells across it make lumps of 12 and 6 m (its finest,
// 4 m, against a column a few metres across, serrates the edge like a fern).
const TRAIL_BILLOW_M: f32 = 48.0;
// How far the billows swell the column and draw it in (a share of its width; air_light.rs's bound allows for half),
// and the height (m) over which they grow in: the smoke leaves the candle smooth and roils as it ages.
const TRAIL_SWELL: f32 = 0.4;
const TRAIL_ROIL_M: f32 = 40.0;
// The light the burning flares put on the smoke at `at` (eye-relative), scattered toward the eye: each flame's I / r²
// by the smoke's phase.
fn flares_light(view: View, at: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    for (var j = 0u; j < min(air.counts.y, 8u); j++) {
        let flare = air.flares[j];
        let toward = at - (curved(flare.position.xyz) - camera.position);
        let r2 = dot(toward, toward) + flare.color.w * flare.color.w;
        let mu = -dot(toward, view.ray) * inverseSqrt(r2);
        light += flare.color.rgb / r2 * (reach_window(sqrt(r2), flare.position.w) * phase_of(air.trail.y, mu));
    }
    return light;
}

// One straight piece of a flare's trail, from the point `a` to `b` (xyz from the candle at `candle`, eye-relative; w
// the smoke's spread, ρ²), taken where the ray passes nearest it, its ends rounded: the light it scatters toward the
// eye.
fn trail_piece(view: View, flare: Flare, candle: vec3<f32>, a: vec4<f32>, b: vec4<f32>, shift: vec3<f32>) -> vec3<f32> {
    let piece = b.xyz - a.xyz;
    let span = length(piece);
    if span < 1e-3 {
        return vec3<f32>(0.0);
    }
    let direction = piece / span;
    let start = candle + a.xyz;
    // Where the ray passes nearest the piece: `f` of the way along it, held to its ends.
    let cosine = dot(view.ray, direction);
    let denom = max(1.0 - cosine * cosine, 1e-6);
    let f = clamp((cosine * dot(view.ray, start) - dot(direction, start)) / (denom * span), 0.0, 1.0);
    let on_axis = start + piece * f;
    let t = max(dot(on_axis, view.ray), 0.0);
    if t > view.hit {
        return vec3<f32>(0.0);
    }
    let at = view.ray * t;
    let off = at - on_axis;
    let spread = mix(a.w, b.w, f);
    // A pixel's footprint widens a column narrower than it (its smoke spread over it, σ ρ² kept).
    let footprint = 0.75 * air.trail.z * t;
    if dot(off, off) > 9.0 * ((1.0 + TRAIL_SWELL) * (1.0 + TRAIL_SWELL) * spread + footprint * footprint) {
        return vec3<f32>(0.0);
    }
    // The billows, rolled in the air where the smoke holds still: hardly any at the candle, growing as it rises.
    let height = mix(a.y, b.y, f);
    let roil = clamp(height / TRAIL_ROIL_M, 0.1, 1.0);
    let noise = textureSampleLevel(screen_noise, screen_sampler, (at - candle + flare.air.xyz) / TRAIL_BILLOW_M + shift,
        0.0).rg;
    let billow = (smoothstep(0.15, 0.75, dot(noise, vec2<f32>(0.65, 0.35))) * 2.0 - 1.0) * roil;
    let swell = 1.0 + TRAIL_SWELL * billow;
    let width2 = spread * swell * swell + footprint * footprint;
    let width = sqrt(width2);
    // The chord's optical depth through the column, slanted by the ray's angle to it (no more than the piece's length
    // along it), thicker in a billow, out of the column's own top fading with its last tenth.
    let sigma = flare.trail.y / width2 * max(1.0 + 1.5 * billow, 0.2);
    let fade = 1.0 - sstep(0.9, 1.0, height / flare.trail.x);
    let chord = sigma * exp(-dot(off, off) / width2) * min(width * 1.7724539 / sqrt(denom), span) * fade;
    return flares_light(view, at) * (air.trail.x * (1.0 - exp(-chord))) * to_eye(view, t);
}

// The smoke trailing above a flare, lit by the flames: a Gaussian column through the trail's points (straight between
// each two, the candle at `candle`, eye-relative, the first), its smoke's extinction on the axis σ = `trail.y` / ρ², ρ²
// each point's spread, swollen, drawn in and thickened by billows rolled in the air. Pieces far apart along the trail
// add; two that meet at a point overlap about it, where the brighter is taken (their union: neither a gap on the
// outside of a bend nor twice the smoke at the joint, and never less than nothing along a chain of them).
fn flare_trail(view: View, flare: Flare, candle: vec3<f32>) -> vec3<f32> {
    let count = min(u32(flare.trail.z), TRAIL_POINTS);
    if count < 2u || flare.trail.x < 1.0 {
        return vec3<f32>(0.0);
    }
    let seed = flare.air.w;
    let shift = vec3<f32>(fract(seed * 0.61803), fract(seed * 0.3779), fract(seed * 0.2718));
    var light = vec3<f32>(0.0);
    var previous = vec3<f32>(0.0);
    for (var k = 0u; k + 1u < count; k++) {
        let piece = trail_piece(view, flare, candle, flare.points[k], flare.points[k + 1u], shift);
        light += piece - min(piece, previous);
        previous = piece;
    }
    return light;
}

// The light a flare under the clouds puts on their base (camera and flare under the lowest layer's bottom): a thick
// cloud sends back its albedo of the light from below, `cos / r²` of the flare's, where the clouds cover the pixel. A
// cloud's base is no plane: its water thickens over the lowest `BASE_RAMP` metres, ragged, and the light goes some way
// in, so the lit cloud is taken at two heights through that ramp, which keeps a far patch seen nearly edge-on from
// drawing as one thin line.
const BASE_RAMP: f32 = 150.0;
fn cloud_base(view: View, flare: Flare, candle: vec3<f32>) -> vec3<f32> {
    let base = lighting.clouds.x;
    let flare_height = flare.position.y;
    if lighting.clouds.w <= 0.0 || camera.position.y >= base || flare_height >= base || view.ray.y <= 0.0 {
        return vec3<f32>(0.0);
    }
    let cover = view.cloud;
    if cover <= 0.0 {
        return vec3<f32>(0.0);
    }
    let world_candle = candle + vec3<f32>(0.0, camera.curvature * dot(candle.xz, candle.xz), 0.0);
    let across = max(1.0 - view.ray.y * view.ray.y, 0.0);
    var sum = vec3<f32>(0.0);
    for (var k = 0; k < 2; k++) {
        let height = base + BASE_RAMP * (f32(k) + 0.5) / 2.0;
        // Where the ray reaches that height, the drawn curvature taken in.
        let rise = height - camera.position.y;
        let t = 2.0 * rise / (view.ray.y + sqrt(view.ray.y * view.ray.y + 4.0 * camera.curvature * across * rise));
        if t > view.hit {
            continue;
        }
        let point = view.ray * t;
        let toward = point + vec3<f32>(0.0, camera.curvature * dot(point.xz, point.xz), 0.0) - world_candle;
        let r2 = max(dot(toward, toward), 1.0);
        let r = sqrt(r2);
        let incidence = max(toward.y, 0.0) / r;
        let lit = incidence / r2 * reach_window(r, flare.position.w)
            * exp(-air_depth(flare_height, height, r, view.rain_share));
        sum += aerial_sample(aerial_volume, aerial_sampler, view.uv, t).transmittance * (lit * gas_through(view, t));
    }
    return flare.color.rgb * sum * (air.phase.z / PI * cover / 2.0);
}

// Interleaved gradient noise (Jimenez 2014), moved each frame.
fn jitter_of(pixel: vec2<f32>) -> f32 {
    let f = f32(camera.frame % 64u) * 5.588238;
    let p = floor(pixel) + vec2<f32>(f, f);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// What a full-resolution pixel's ray at `pixel` reads once, its scene `depth` given.
fn view_at(pixel: vec2<f32>, depth: f32) -> View {
    var view: View;
    view.pixel = pixel;
    view.uv = pixel * camera.viewport.zw;
    view.ray = view_ray(ndc_from_pixel(pixel));
    let forward = -camera.inverse_view[2].xyz;
    view.hit = select(1e7, view_depth(depth) / max(dot(view.ray, forward), 1e-4), depth > 0.0);
    view.fronts = vec4<f32>(0.0);
    view.depths = vec4<f32>(0.0);
    if lighting.direct.w > 0.0 {
        view.fronts = textureSampleLevel(occlusion_fronts, occlusion_sampler, view.uv, 0.0);
        view.depths = textureSampleLevel(occlusion_depths, occlusion_sampler, view.uv, 0.0);
    }
    view.rain_share = select(0.0, min(air.medium.y / max(air.haze.x, 1e-9), 1.0), air.haze.x > 0.0 && air.medium.y > 0.0);
    view.jitter = jitter_of(pixel);
    view.axial = max(dot(view.ray, forward), 1e-4);
    view.cloud = 0.0;
    if lighting.clouds.w > 0.0 {
        let size = textureDimensions(cloud_cover);
        let texel = min(vec2<u32>(view.uv * vec2<f32>(size)), size - 1u);
        view.cloud = clamp(textureLoad(cloud_cover, texel, 0).a, 0.0, 1.0);
    }
    return view;
}

// Neither a NaN nor more than the scene's target holds.
fn held(light: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0), min(light, vec3<f32>(CEILING)), light == light);
}

@vertex
fn air_vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    // At the near plane (reversed depth): passes the depth test over everything; the fragment ends each ray itself.
    return fullscreen_vertex(index, 1.0);
}

// The star shells' light in the air and on the cloud base, at half the resolution (it varies slowly across the screen,
// and covers all of it): each texel the ray through the middle of its four pixels, out to the nearest of what they hit,
// and that distance in alpha for the composite's upsampling (`halo_at`). Drawn at the backdrop's copy point.
@fragment
fn halo_fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let pixel = in.position.xy * 2.0;
    let size = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let corner = vec2<i32>(in.position.xy) * 2;
    var depth = 0.0;
    for (var k = 0; k < 4; k++) {
        depth = max(depth, textureLoad(scene_depth, min(corner + vec2<i32>(k & 1, k >> 1), size), 0));
    }
    var view = view_at(pixel, depth);
    // The samples at the middles of their shares, the same every frame: the glow has no edges for temporal
    // anti-aliasing to resolve a jitter over, and held still it neither crawls nor shows the noise's pattern.
    view.jitter = 0.5;
    var light = vec3<f32>(0.0);
    for (var i = 0u; i < min(air.counts.y, 8u); i++) {
        let flare = air.flares[i];
        let candle = curved(flare.position.xyz) - camera.position;
        light += cloud_base(view, flare, candle);
        // The air about it out to its reach.
        let pass_by = passing(view, candle);
        let reach = select(1e7, flare.position.w, flare.position.w > 0.0);
        let s = reach * reach - pass_by.off * pass_by.off;
        if s <= 0.0 {
            continue;
        }
        let root = sqrt(s);
        let t0 = max(pass_by.along - root, 0.0);
        let t1 = min(pass_by.along + root, view.hit);
        if t1 <= t0 {
            continue;
        }
        light += air_light(view, FLARE_SAMPLES, candle, flare.position.y, flare.color.rgb, flare.color.w, flare.position.w,
            t0, t1, -1);
    }
    return vec4<f32>(held(light), min(view.hit, HALO_FAR));
}

// The half-resolution halo at a full-resolution pixel whose ray reaches `hit` m: its four nearest texels, each weighed
// by how near its own distance is to this pixel's, so the glow behind a hull does not bleed over her edge.
fn halo_at(pixel: vec2<f32>, hit: f32) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(halo));
    let here = min(hit, HALO_FAR);
    let place = pixel * 0.5 - 0.5;
    let base = vec2<i32>(floor(place));
    let f = place - floor(place);
    var sum = vec3<f32>(0.0);
    var weight = 0.0;
    var nearest = vec3<f32>(0.0);
    var closest = 1e30;
    for (var k = 0; k < 4; k++) {
        let offset = vec2<i32>(k & 1, k >> 1);
        let texel = textureLoad(halo, clamp(base + offset, vec2<i32>(0), size - 1), 0);
        let bilinear = select(1.0 - f.x, f.x, offset.x == 1) * select(1.0 - f.y, f.y, offset.y == 1);
        let apart = abs(texel.a - here) / max(min(texel.a, here), 1.0);
        let w = bilinear * exp(-apart * 20.0) + 1e-6;
        sum += texel.rgb * w;
        weight += w;
        if apart < closest {
            closest = apart;
            nearest = texel.rgb;
        }
    }
    // None of them at this pixel's distance (a thin mast against the glow): the nearest in distance.
    return select(sum / weight, nearest, weight < 0.01);
}

// At full resolution, added over everything the air holds: the beams (thin, with the smoke screens they light), the
// lamps and flames themselves, the smoke over each flare, and the halo upsampled.
@fragment
fn air_fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let view = view_at(in.position.xy, textureLoad(scene_depth, vec2<i32>(in.position.xy), 0));
    var light = vec3<f32>(0.0);
    for (var i = 0u; i < min(air.counts.x, 8u); i++) {
        let beam = air.beams[i];
        let lamp = curved(beam.origin.xyz) - camera.position;
        // The lamp itself, from inside its beam: its mirror at the beam's luminance toward the eye.
        let facing = beam_profile(beam, lamp, vec3<f32>(0.0));
        if facing > 0.0 {
            let mirror = beam.color.w * sqrt(beam.axis.w);
            light += source(view, lamp, mirror, beam.color.rgb * facing);
        }
        let span = beam_span(view, beam, lamp);
        if span.y <= span.x {
            continue;
        }
        light += air_light(view, BEAM_SAMPLES, lamp, height_of(lamp), beam.color.rgb, beam.color.w, beam.origin.w, span.x,
            span.y, i32(i));
        light += beam_screens(view, beam, lamp, span.x, span.y);
    }
    for (var i = 0u; i < min(air.counts.y, 8u); i++) {
        let flare = air.flares[i];
        let candle = curved(flare.position.xyz) - camera.position;
        // Rays passing well clear of the flame and its smoke (a sphere about them, a pixel's footprint included)
        // skip both.
        let middle = candle + flare.bound.xyz;
        let along = max(dot(middle, view.ray), 0.0);
        let bound = flare.bound.w + 3.0 * air.trail.z * length(middle) + 1.0;
        if dot(middle - view.ray * along, middle - view.ray * along) > bound * bound {
            continue;
        }
        light += source(view, candle, flare.color.w, flare.color.rgb);
        light += flare_trail(view, flare, candle);
    }
    if air.counts.w > 0u {
        light += halo_at(view.pixel, view.hit);
    }
    return vec4<f32>(held(light), 0.0);
}
