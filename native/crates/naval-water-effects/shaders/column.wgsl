// A shell's splash, drawn by marching the view ray through its water (native, `src/columns.rs`; the field is
// `column_field.wgsl`). One box a splash about its water, its front faces drawn (with the lens inside it, one triangle
// over the screen) into an adaptive-resolution target, which `column_composite.wgsl` lays over the scene;
// each texel follows the ray through its 2 × 2 block of pixels from the box's near side to its far side or to what the
// backdrop's depth puts behind it there (the sea, a hull: the nearest of the block's four depths, so a mast or a wire a
// pixel wide in front of a column is never painted over), whichever is nearer, so a hull standing in a column cuts it
// and the sea swallows its foot.
//
// The light is a cloud's (spray is a cloud of drops, scattering nearly all it stops): the sun's light through the
// water toward it, many times scattered (the octaves of Wrenninge's approximation, each further scattered light
// reaching deeper and spread wider), the sky's light from above, dimmed by the water over a point, and the sun's shadow
// maps for a hull or another column up-sun. Spray seen against the sun glows (forward scattering by drops); a dense
// column seen against it is grey in its middle with a bright rim, as backlit cumulus.
// Premultiplied, blended one · src + (1 − a) · dst, farthest column first; linear HDR radiance.

#import naval::camera::{camera, world_to_clip, view_ray, ndc_from_pixel, view_depth}
#import naval::shadow::{sun_shadow, shadow_light_direction}
#import naval_water_effects::spray::{EffectLighting, sky_fill, spray_fog}
#import naval_water_effects::column_field::{Column, Splash, Burst, Sheet, Water, columns, splash_of, burst_of, sheet_of,
    is_burst, is_sheet, column_envelope, water_at, burst_envelope, burst_water, sheet_envelope, sheet_water, to_frame,
    from_frame, fan_open, through_gaps, fan_box, cast_through}

@group(1) @binding(0) var<uniform> lighting: EffectLighting;
// The backdrop's reversed depth (opaque and water; a far-plane placeholder before the first copy).
@group(3) @binding(0) var scene_depth: texture_depth_2d;

// Steps along a view ray through the box (m canonical): about the column's outline no longer than `STEP_FINE`, so its
// dense face resolves within a fraction of a metre (longer steps left it grainy where temporal
// anti-aliasing keeps no history, over the sea), and through the thin mist about it `STEP_MIST`, its soft shapes need
// no more; at most `STEPS_MOST` a ray. Then the steps toward the sun from each (doubling from the first).
const STEP_FINE: f32 = 1.0;
const STEP_MIST: f32 = 5.0;
const STEPS_MOST: i32 = 128;
const LIGHT_STEPS: i32 = 3;
const LIGHT_FIRST: f32 = 2.0;
// A fold between the column's lumps keeps this share of the sun's light: the lumps up-sun of it stand in its way.
const FOLD_SUN: f32 = 0.55;
// The sky's light over a point is dimmed by the water above it, read this far up (m canonical).
const SKY_PROBE: f32 = 6.0;
// In the thin mist the light toward the sun is read once, this far up the beam (m canonical), over twice its length,
// through the column alone: the mist barely shades itself.
const MIST_REACH: f32 = 6.0;
const SKY_DIMMING: f32 = 0.6;
// Light scattered repeatedly between nearly lossless drops still reaches the shaded interior.
const SKY_FLOOR: f32 = 0.5;
const SUN_FILL: f32 = 0.18;
// A splash's column (2026-10-24): the sky is read up and also toward its middle, where its own water stands between a
// point on its shaded side and half the sky, down to `SPLASH_SKY_FLOOR` of it; and the sun's light diffused through
// it is what diffusion lets through a column of its depth (1 / (1 + 0.75 τ (1 − g)), some 6 % through a hundred), not
// `SUN_FILL`. Before, its shaded side stood nearly as white as its sunlit one: lit from the whole sky as a lone drop
// is, and by a fifth of the sun through it. A tenth (2026-10-24, the fill through some fifty): the water the eye sees
// between its feathers and strands is lit through the same gaps, which the light's smooth fan does not hold.
const SPLASH_SKY_FLOOR: f32 = 0.35;
const SPLASH_SUN_FILL: f32 = 0.1;
// And the sky's light on it as a column takes it, its own water hiding half the sky from a point of it, against the
// whole sky a lone drop sees (`SKY_GAIN`): the sky had lit a column about as much as the sun on its side, so that it
// read the same from every side.
const SPLASH_SKY_GAIN: f32 = 0.8;
// Multiple scattering (Wrenninge 2013): each octave keeps this share of the last's light, reaches through this
// share of its optical depth, and scatters this share as far forward.
const OCTAVES: i32 = 4;
const OCTAVE_GAIN: f32 = 0.5;
const OCTAVE_REACH: f32 = 0.25;
const OCTAVE_ROUND: f32 = 0.6;
// Drops a millimetre or so across scatter strongly forward (diffraction), with a weaker lobe back.
const FORWARD_G: f32 = 0.78;
const BACK_G: f32 = 0.2;
const FORWARD_SHARE: f32 = 0.7;
// The sky's light on white water against the sky's albedo-scaled fill (spray.wgsl's `SKY_GAIN`), and the sea's
// light from below (a share of the sky's).
const SKY_GAIN: f32 = 2.0;
const SEA_LIGHT: f32 = 0.12;
// Drops are near-perfect scatterers.
const ALBEDO: f32 = 0.985;
// Where the column meets the backdrop it fades over this depth (world m), so a hull or a wave cuts it softly.
const SOFT: f32 = 1.5;
// The noise's detail is drawn while a pixel spans less than this share of a lump (canonical m), faded beyond.
const DETAIL_FULL: f32 = 0.35;
const DETAIL_NONE: f32 = 2.5;
// A hull or another column up-sun shades the splash: its shadow read this far up the beam past the column's own
// caster (m, world), and at least `OWN_CASTER` up the beam before where that caster lies on the light's ray
// (`before_own_caster`): within its own shadow a grown column, wider than the lift, shaded itself a second time (its own
// water's shading is the march's), a dark V either side of a bright bell over its foot.
const SHADOW_LIFT: f32 = 18.0;
const OWN_CASTER: f32 = 2.0;
// The light marched toward the sun crosses the column's smooth fan; a point within a clump of its water lies that much
// deeper in it, one between clumps that much less (this share of the difference): the shading follows the clumps, not
// only the fan's smooth shells.
const CLUMP_SHADE: f32 = 0.7;
// A bow's burst is a few metres of spray beside her hull, marched finer (its steps through its fan and outside it, m)
// and shaded by her up-sun of it from just up the beam (it casts nothing of its own).
const BURST_FINE: f32 = 0.5;
const BURST_FAR: f32 = 1.5;
const BURST_SHADOW_LIFT: f32 = 1.0;
// A bow's sheet is thinner still: marched finer through its water (finer where it is thinnest: `sheet_water`) and in
// longer strides through the empty box about it, its light read once toward the sun and its sky left open over it (a
// sheet so thin and low barely shades itself), shaded where the ray first meets its water (it lies close along her
// side, which shades part of it), and cut softly by the crest it rises from and the hull beside it over a shorter depth
// (world m).
const SHEET_FINE: f32 = 0.3;
const SHEET_FAR: f32 = 3.0;
const SHEET_LIGHT_STEPS: i32 = 1;
const SHEET_SOFT: f32 = 0.4;
// What the record holds (`Column.kind`).
const SPLASH: u32 = 0u;
const BURST: u32 = 1u;
const SHEET: u32 = 2u;

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
    // How far the drawn world lies below the simulation's at the splash (the Earth's curvature; m).
    @location(1) @interpolate(flat) drop: f32,
}

// The box's 36 corners, wound counter-clockwise seen from outside (the pipeline culls the far faces).
const CORNERS = array<u32, 36>(
    0u, 4u, 6u, 0u, 6u, 2u, 1u, 3u, 7u, 1u, 7u, 5u, 0u, 1u, 5u, 0u, 5u, 4u,
    2u, 6u, 7u, 2u, 7u, 3u, 0u, 2u, 3u, 0u, 3u, 1u, 4u, 5u, 7u, 4u, 7u, 6u);

@vertex
fn vertex(@builtin(vertex_index) index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let column = columns[instance];
    let origin = column.origin.xyz;
    var out: Varyings;
    out.index = instance;
    let across = origin.xz - camera.position.xz;
    out.drop = camera.curvature * dot(across, across);
    // The lens inside the box (with room for the near plane): one triangle over the screen at the near plane.
    let eye = to_frame(camera.position + vec3<f32>(0.0, out.drop, 0.0) - origin, column.frame);
    let margin = vec3<f32>(camera.near * 4.0 + 0.5);
    if all(eye > column.low.xyz - margin) && all(eye < column.high.xyz + margin) {
        if index < 3u {
            out.clip = vec4<f32>(f32(i32(index & 1u) * 4 - 1), f32(i32(index >> 1u) * 4 - 1), 1.0, 1.0);
        } else {
            out.clip = vec4<f32>(-2.0, -2.0, 1.0, 1.0);
        }
        return out;
    }
    var corners = CORNERS;
    let corner = corners[index];
    let unit = vec3<f32>(f32(corner & 1u), f32((corner >> 1u) & 1u), f32((corner >> 2u) & 1u));
    out.clip = world_to_clip(origin + from_frame(mix(column.low.xyz, column.high.xyz, unit), column.frame));
    return out;
}

// The nearest of the backdrop's depths over a texel's 2 × 2 block of the frame's pixels (reversed: the largest), where
// its ray stops (`column_composite.wgsl` weighs the texels by the same).
fn block_depth(texel: vec2<i32>) -> f32 {
    let last = vec2<i32>(textureDimensions(scene_depth)) - 1;
    let at = texel * i32(lighting.column_sampling.x);
    let a = textureLoad(scene_depth, min(at, last), 0);
    if lighting.column_sampling.x < 1.5 { return a; }
    let b = textureLoad(scene_depth, min(at + vec2<i32>(1, 0), last), 0);
    let c = textureLoad(scene_depth, min(at + vec2<i32>(0, 1), last), 0);
    let d = textureLoad(scene_depth, min(at + vec2<i32>(1, 1), last), 0);
    return max(max(a, b), max(c, d));
}

// Henyey–Greenstein times 4π: 1 for an even scatterer.
fn henyey(cosine: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * g * cosine, 1e-4), 1.5);
}

fn phase(cosine: f32, round: f32) -> f32 {
    let value = mix(henyey(cosine, -BACK_G * round), henyey(cosine, FORWARD_G * round), FORWARD_SHARE);
    // The unresolved forward peak otherwise turns whole backlit heads into flat display white.
    return value / (1.0 + max(value - 1.0, 0.0) / 3.0);
}

// How far up the beam from `p` (world) a read must be to stand before the column's own caster on the light's ray
// through it, where its water lies on average along that ray (`cast_through`, as `column_caster.wgsl` writes it; m,
// world; negative where `p` already stands before it or the ray misses its box).
fn before_own_caster(p: vec3<f32>, toward: vec3<f32>, column: Column, s: Splash) -> f32 {
    let size = column.origin.w;
    let fan = fan_box(column);
    let way = -toward;
    let at = (p - column.origin.xyz) / size;
    let safe = select(way, vec3<f32>(1e-6), abs(way) < vec3<f32>(1e-6));
    let a = (fan.low - at) / safe;
    let b = (fan.high - at) / safe;
    let enter = max(max(min(a.x, b.x), min(a.y, b.y)), min(a.z, b.z));
    let leave = min(min(max(a.x, b.x), max(a.y, b.y)), max(a.z, b.z));
    if leave <= enter {
        return -1.0;
    }
    // Where the light's ray enters the box (the caster's face), and `p`'s way along it from there.
    let own = cast_through(at + way * enter, way, column, s);
    return (-enter - own.half) * size;
}

// The optical depth from a point in the column toward the sun, through the column's envelope (the folds between its
// lumps are shaded by `FOLD_SUN`, not marched).
fn toward_sun(p: vec3<f32>, sun: vec3<f32>, kind: u32, s: Splash, b: Burst, sh: Sheet) -> f32 {
    var depth = 0.0;
    var reach = 0.0;
    var stride = LIGHT_FIRST;
    let steps = select(LIGHT_STEPS, SHEET_LIGHT_STEPS, kind == SHEET);
    for (var i = 0; i < steps; i++) {
        depth += envelope_of(p + sun * (reach + stride * 0.5), kind, s, b, sh) * stride;
        reach += stride;
        stride *= 2.0;
    }
    return depth;
}

// A splash's column's envelope, a bow's burst's or her sheet's.
fn envelope_of(p: vec3<f32>, kind: u32, s: Splash, b: Burst, sh: Sheet) -> f32 {
    if kind == SHEET {
        return sheet_envelope(p, sh);
    }
    if kind == BURST {
        return burst_envelope(p, b);
    }
    return column_envelope(p, s);
}

fn water_of(p: vec3<f32>, kind: u32, s: Splash, b: Burst, sh: Sheet, detail: f32, fine: f32, far: f32) -> Water {
    if kind == SHEET {
        return sheet_water(p, sh, detail, fine, far);
    }
    if kind == BURST {
        return burst_water(p, b, detail, fine, far);
    }
    return water_at(p, s, detail, fine, far);
}

// The march's two targets: the columns' premultiplied light and opacity, and the view depth (m) their water lies at
// times that opacity, added up over the boxes as the light is, for the effects drawn after the composite
// (`WaterEffects::cover`).
struct Marched {
    @location(0) color: vec4<f32>,
    @location(1) lies: vec4<f32>,
}

@fragment
fn fragment(input: Varyings) -> Marched {
    let column = columns[input.index];
    let s = splash_of(column, input.index);
    let bow = burst_of(column);
    let sheet = sheet_of(column);
    let kind = select(select(SPLASH, BURST, is_burst(column)), SHEET, is_sheet(column));
    let size = column.origin.w;
    // One scene pixel for a distant column, or the middle of a 2 × 2 block for the half-resolution march,
    // stopped at the block's nearest backdrop depth (as the composite reads it).
    let pixel = input.clip.xy;
    let ray = view_ray(ndc_from_pixel(pixel * lighting.column_sampling.x));
    // The ray and the eye in the splash's frame (canonical m): the drawn world lies `drop` below the simulation's here.
    let dir = to_frame(ray, column.frame);
    let eye = to_frame((camera.position + vec3<f32>(0.0, input.drop, 0.0) - column.origin.xyz) / size, column.frame);
    let low = column.low.xyz / size;
    let high = column.high.xyz / size;
    let safe = select(dir, vec3<f32>(1e-6), abs(dir) < vec3<f32>(1e-6));
    let a = (low - eye) / safe;
    let b = (high - eye) / safe;
    let entry = min(a, b);
    let exit = max(a, b);
    let t0 = max(max(max(entry.x, entry.y), entry.z), 0.0);
    let forward = -camera.inverse_view[2].xyz;
    let along = max(dot(ray, forward), 1e-4);
    let scene = view_depth(block_depth(vec2<i32>(floor(pixel)))) / along / size;
    let t1 = min(min(min(exit.x, exit.y), exit.z), scene);
    if t1 <= t0 {
        discard;
    }

    let sun = to_frame(normalize(lighting.sun_direction.xyz), column.frame);
    let cosine = dot(dir, sun);
    var phases: array<f32, 4>;
    var round = 1.0;
    for (var n = 0; n < OCTAVES; n++) {
        phases[n] = phase(cosine, round);
        round *= OCTAVE_ROUND;
    }
    let sky = sky_fill(lighting) * select(SKY_GAIN, SPLASH_SKY_GAIN, kind == SPLASH);
    let direct = lighting.direct.xyz;
    // A hull or another column up-sun: read once, at the middle of the ray's run through the box (a sheet's, where the
    // ray first meets its water).
    let middle = column.origin.xyz + from_frame(eye + dir * (0.5 * (t0 + t1)), column.frame) * size;
    let toward = shadow_light_direction();
    var shade = 1.0;
    var unshaded = kind == SHEET;
    if !unshaded {
        var lift = select(SHADOW_LIFT, BURST_SHADOW_LIFT, kind == BURST);
        if kind == SPLASH {
            lift = max(lift, before_own_caster(middle, toward, column, s) + OWN_CASTER);
        }
        shade = sun_shadow(middle + toward * lift, toward);
    }
    let fine = select(select(STEP_FINE, BURST_FINE, kind == BURST), SHEET_FINE, kind == SHEET);
    let far = select(select(STEP_MIST, BURST_FAR, kind == BURST), SHEET_FAR, kind == SHEET);
    let soft_depth = select(SOFT, SHEET_SOFT, kind == SHEET);
    // Metres (canonical) a march texel spans per metre of distance.
    let footprint = 2.0 * lighting.column_sampling.x * camera.tan_half_fov_y * camera.viewport.w;

    // Deterministic quadrature: the sea keeps too little TAA history to average a randomly shifted march.
    // Keep the first sample within half a fine step of the box, including its narrow leading water.
    var t = t0 + min(fine * 0.5, (t1 - t0) * 0.5);
    var transmittance = 1.0;
    var light = vec3<f32>(0.0);
    var weight = 0.0;
    var mean = 0.0;
    for (var i = 0; i < STEPS_MOST; i++) {
        if t >= t1 || transmittance < 0.01 {
            break;
        }
        let p = eye + dir * t;
        let detail = 1.0 - smoothstep(DETAIL_FULL, DETAIL_NONE, t * footprint);
        let water = water_of(p, kind, s, bow, sheet, detail, fine, far);
        let density = water.column + water.mist;
        let stride = min(water.stride, t1 - t);
        // A step that stops less than this of the light is not worth lighting.
        if density * stride > 2e-3 {
            if unshaded {
                shade = sun_shadow(column.origin.xyz + from_frame(p, column.frame) * size + toward * BURST_SHADOW_LIFT,
                    toward);
                unshaded = false;
            }
            var depth: f32;
            // A step that stops little of the light through thin water takes the light as the mist does.
            if water.column * stride > 0.03 {
                depth = toward_sun(p, sun, kind, s, bow, sheet);
                if kind == SPLASH {
                    depth = through_gaps(depth, fan_open(p, s)) * mix(1.0, water.clump, CLUMP_SHADE);
                }
            } else {
                depth = envelope_of(p + sun * MIST_REACH, kind, s, bow, sheet) * MIST_REACH * 2.0;
            }
            var sunlit = 0.0;
            var gain = 1.0;
            var reach = 1.0;
            for (var n = 0; n < OCTAVES; n++) {
                sunlit += gain * exp(-depth * reach) * phases[n];
                gain *= OCTAVE_GAIN;
                reach *= OCTAVE_REACH;
            }
            // The octaves redistribute the incident light. Adding their unnormalised energies overlit the tops.
            sunlit = max(sunlit / 1.875, select(SUN_FILL, SPLASH_SUN_FILL, kind == SPLASH) * (1.0 - exp(-depth)));
            var above = 0.0;
            if kind != SHEET {
                above = envelope_of(p + vec3<f32>(0.0, SKY_PROBE, 0.0), kind, s, bow, sheet) * SKY_PROBE * 1.5;
            }
            if kind == SPLASH {
                // And toward the column's middle (its axis, leaning with it as its water rose: `thrown_at`).
                let lifted = p.y + s.flight.y;
                let inward = vec3<f32>(s.lean.x * lifted - p.x, 0.0, s.lean.y * lifted - p.z);
                let reach = length(inward);
                if reach > 1e-3 {
                    above += envelope_of(p + inward * (min(SKY_PROBE, reach) / reach), kind, s, bow, sheet)
                        * min(SKY_PROBE, reach) * 1.5;
                }
            }
            if kind == SPLASH {
                // The light that finds its way between the fan's feathers and strands.
                let gaps = fan_open(p, s);
                above = through_gaps(above, gaps);
            }
            let open = mix(select(SKY_FLOOR, SPLASH_SKY_FLOOR, kind == SPLASH), 1.0, exp(-above * SKY_DIMMING));
            let folded = mix(FOLD_SUN, 1.0, mix(1.0, water.fold, water.column / max(density, 1e-4)));
            let scattered = (direct * (sunlit * shade * folded) + sky * (open + SEA_LIGHT)) * ALBEDO;
            let soft = clamp((scene - t) * size / soft_depth, 0.0, 1.0);
            let through = exp(-density * max(stride, 0.0) * soft);
            let taken = transmittance * (1.0 - through);
            light += taken * scattered;
            weight += taken;
            mean += taken * t;
            transmittance *= through;
        }
        t += max(stride, 1e-3);
    }
    let alpha = (1.0 - transmittance) * column.tint.w;
    if alpha < 0.002 {
        discard;
    }
    let lies = mean / max(weight, 1e-6);
    let world = column.origin.xyz + from_frame(eye + dir * lies, column.frame) * size;
    let color = light / max(weight, 1e-6) * column.tint.rgb;
    return Marched(vec4<f32>(spray_fog(color, world) * alpha, alpha),
        vec4<f32>(lies * size * along * alpha, 0.0, 0.0, alpha));
}
