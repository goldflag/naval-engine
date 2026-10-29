// The blast wave of a gun or an explosion in the air: the shock front, seen by the bend it puts in the light crossing
// it, and the cloud that condenses behind it in damp air. Not a port: the web game had none. `naval_effects::shock`
// has the physics and the numbers; native/docs/combat-effects/gun-blasts.md, "Blast waves".
//
// Each front is a sphere about its blast, drawn first of the transparents over a quad about it (the whole view from
// inside it). A fragment follows its view ray through the sphere. Where the ray grazes the front (its nearest point to
// the centre within the pressure pulse of the front's radius) it turns toward the centre, by the grazing ray's turn
// shaped across the pulse, once for each crossing of the front that lies before the scene behind (the backdrop's
// depth), and less the nearer that scene stands behind the crossing (a background just behind the front hardly
// moves; the sky moves by the whole turn). The backdrop is read along the turned ray and written as its difference
// from the backdrop where the fragment is, so fronts that overlap add and what is already drawn there (the rain's
// veil) stays. Where the scene behind the fragment (the sea, a hull) lies in the pulse itself, the front's foot, it
// is read a little further out from the blast, and the sea there is laid over pale: its ruffled water. The cloud is
// a shell of mist behind the pulse: the fragment takes the length of its ray inside the shell and before the scene,
// and lays the mist over by that depth. `shock_motion` asks temporal AA for less history where any of them moves the
// picture.
#import naval::camera::{camera, curved, view_depth, ndc_from_pixel, view_ray}
#import naval::motion::motion_response

struct Shock {
    // xyz the centre (scene m), w the front's radius (m).
    centre: vec4<f32>,
    // x the pulse's length (m), y the turn at one crossing (rad), z the lean along the bore (0 a burst, 1 a gun),
    // w 1 where the camera is given the whole view.
    band: vec4<f32>,
    // xyz the line of fire, w the seed.
    bore: vec4<f32>,
    // The cloud: x its inner and y its outer radius (m), z its extinction (1/m), w the blast's scale (m).
    veil: vec4<f32>,
    // The pulse on what lies under it: x how far further out the scene there is read (m), y the ruffled sea's opacity.
    surface: vec4<f32>,
}

struct Shocks {
    // x the radians a pixel spans, y the pixels of shift that take a whole frame's history from temporal AA.
    params: vec4<f32>,
    // The mist's radiance in the scene's light.
    radiance: vec4<f32>,
    items: array<Shock, 24>,
}

@group(1) @binding(0) var<uniform> shocks: Shocks;
@group(1) @binding(1) var backdrop_depth: texture_depth_2d;
@group(1) @binding(2) var backdrop_color: texture_2d<f32>;
@group(1) @binding(3) var linear_clamp: sampler;

// The turn across the pulse, in pulse lengths in from the front: a swell that is nothing at the front itself (a step
// there would draw the sphere's outline as a pane of glass), greatest SWELL in from it and gone within the pulse; the
// ripple on what lies under the pulse likewise, greatest RIPPLE in.
const SWELL: f32 = 0.3;
const RIPPLE: f32 = 0.45;
// A gun's blast off its line of fire: its overpressure as (1/2 + 1/2 cos)^DIRECTIVITY, never below FLOOR
// (naval-water-effects' blast sheet); the turn goes as its square root.
const DIRECTIVITY: f32 = 1.3;
const FLOOR: f32 = 0.18;
// The share of the turn a background standing right behind the crossing still shows.
const NEAR_SHARE: f32 = 0.3;
// A turn below this (rad) reads nothing.
const MIN_TURN: f32 = 2e-5;
// The most the turn moves the picture at the front, as a share of the pulse's own width there: more would fold the
// band over itself and draw the sphere's outline as a pane of glass.
const MAX_FOLD: f32 = 0.3;
// The ruffled sea comes up over RUFFLE_RISE pulse lengths behind the front's foot and dies away over RUFFLE_TRAIL;
// the pulse is followed PULSE_CUT lengths in.
const RUFFLE_RISE: f32 = 0.3;
const RUFFLE_TRAIL: f32 = 0.9;
const PULSE_CUT: f32 = 8.0;
// Samples through each crossing of the cloud's shell.
const VEIL_STEPS: i32 = 4;
// The sea is what lies under this height (m); a hull's own plating takes this share of the ripple.
const SEA_TOP: f32 = 3.0;
const HULL_SHARE: f32 = 0.25;

struct ShockOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) index: u32,
}

@vertex
fn shock_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> ShockOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex];
    let shock = shocks.items[instance];
    var out: ShockOut;
    out.index = instance;
    if shock.band.w > 0.5 {
        out.position = vec4<f32>(corner, 1.0, 1.0);
        return out;
    }
    // A square about the cone of rays that touch the sphere, on the plane through its centre.
    let centre = curved(shock.centre.xyz);
    let to = centre - camera.position;
    let range = length(to);
    let radius = shock.centre.w;
    let forward = to / range;
    let reach = radius * range / sqrt(max(range * range - radius * radius, 1e-3)) * 1.03;
    var across = cross(forward, vec3<f32>(0.0, 1.0, 0.0));
    if dot(across, across) < 1e-6 {
        across = vec3<f32>(1.0, 0.0, 0.0);
    }
    across = normalize(across);
    let up = cross(across, forward);
    let world = centre + (across * corner.x + up * corner.y) * reach;
    let clip = camera.view_projection * vec4<f32>(world, 1.0);
    // At the near plane (reversed depth 1): the fragment tests the scene's depth itself.
    out.position = vec4<f32>(clip.xy, clip.w, clip.w);
    return out;
}

fn hash(p: vec3<f32>) -> f32 {
    let q = fract(p * 0.3183099 + vec3<f32>(0.71, 0.113, 0.419)) * 17.0;
    return fract(q.x * q.y * q.z * (q.x + q.y + q.z));
}

fn value_noise(p: vec3<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let low_near = mix(hash(cell), hash(cell + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let high_near = mix(hash(cell + vec3<f32>(0.0, 1.0, 0.0)), hash(cell + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let low_far = mix(hash(cell + vec3<f32>(0.0, 0.0, 1.0)), hash(cell + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let high_far = mix(hash(cell + vec3<f32>(0.0, 1.0, 1.0)), hash(cell + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(low_near, high_near, u.y), mix(low_far, high_far, u.y), u.z);
}

// A gun's blast toward `outward` (unit, from the blast's centre), against its strength along the bore.
fn lean(shock: Shock, outward: vec3<f32>) -> f32 {
    let along = max(0.5 + 0.5 * dot(outward, shock.bore.xyz), FLOOR);
    return mix(1.0, pow(along, DIRECTIVITY), shock.band.z);
}

// A swell `within` its width of its start: nothing there and level (no edge to see), 1 at its crest (`within` = 1),
// dying away after.
fn swell(within: f32) -> f32 {
    return within * within * exp(2.0 - 2.0 * within);
}

// The mist along a stretch of the ray (from `start` to `stop`, m) through the cloud's shell: the shell is thickest in
// its middle and thins to nothing at both faces, uneven in clumps that ride out with it, and leans as the gun's blast
// does.
fn veil_depth(shock: Shock, ray: vec3<f32>, to: vec3<f32>, start: f32, stop: f32) -> f32 {
    if stop <= start {
        return 0.0;
    }
    let pace = (stop - start) / f32(VEIL_STEPS);
    var sum = 0.0;
    for (var i = 0; i < VEIL_STEPS; i++) {
        let spoke = ray * (start + (f32(i) + 0.5) * pace) - to;
        let range = length(spoke);
        let across = (range - shock.veil.x) / max(shock.veil.y - shock.veil.x, 1e-3);
        if across > 0.0 && across < 1.0 {
            let grain = spoke / shock.veil.w * 0.55 + vec3<f32>(shock.bore.w);
            let clumps = 0.15 + 1.7 * value_noise(grain) * value_noise(grain * 2.3 + vec3<f32>(7.3)) * 2.0;
            sum += 6.0 * across * (1.0 - across) * clumps * lean(shock, spoke / range);
        }
    }
    return shock.veil.z * sum * pace;
}

// How much of a turn at `crossing` metres along the ray shows on a scene `behind` metres along it.
fn beyond(crossing: f32, behind: f32) -> f32 {
    return mix(NEAR_SHARE, 1.0, saturate(1.0 - crossing / behind));
}

struct Trace {
    ray: vec3<f32>,
    // The turn of the view ray (rad), the unit vector it turns along (toward the blast's centre) and the metres along
    // the ray to where it enters the front (0 from inside it): what stands nearer is not the front's to move.
    turn: f32,
    toward: vec3<f32>,
    crossing: f32,
    // The cloud's optical depth along the ray.
    depth: f32,
    // The pulse on the scene behind: metres along the ray to it, how far further out from the blast it is read (m,
    // along `outward`) and the ruffled sea's opacity there.
    behind: f32,
    push: f32,
    outward: vec3<f32>,
    ruffle: f32,
}

fn trace_shock(index: u32, pixel: vec2<f32>) -> Trace {
    var out: Trace;
    let shock = shocks.items[index];
    let ray = view_ray(ndc_from_pixel(pixel));
    out.ray = ray;
    let to = curved(shock.centre.xyz) - camera.position;
    let along = dot(to, ray);
    // From the ray's nearest point to the centre.
    let off = to - ray * along;
    let radius = shock.centre.w;
    let miss_squared = dot(off, off);
    if miss_squared >= radius * radius {
        return out;
    }
    let half = sqrt(radius * radius - miss_squared);
    let near = along - half;
    let far = along + half;
    if far <= 0.0 {
        return out;
    }
    let miss = sqrt(miss_squared);
    let toward = off / max(miss, 1e-4);
    // The scene behind, metres along this ray (the far plane where nothing wrote depth).
    let forward = -camera.inverse_view[2].xyz;
    let behind = view_depth(textureLoad(backdrop_depth, vec2<i32>(pixel), 0)) / max(dot(ray, forward), 1e-4);
    out.behind = behind;
    // Nothing stands before the front for a camera inside it.
    out.crossing = max(near, 0.0);

    // The front's foot: where what stands behind this pixel lies in the pulse.
    if behind < camera.far * 0.9 {
        let spoke = ray * behind - to;
        let range = length(spoke);
        let within = (radius - range) / shock.band.x;
        if within > 0.0 && within < PULSE_CUT {
            let outward = spoke / max(range, 1e-4);
            let leaning = sqrt(lean(shock, outward));
            // The point's height over the sea's level, the Earth's drop given back.
            let level = ray.xz * behind;
            let height = camera.position.y + ray.y * behind + camera.curvature * dot(level, level);
            let sea = 1.0 - smoothstep(0.6 * SEA_TOP, SEA_TOP, height);
            out.push = shock.surface.x * swell(within / RIPPLE) * leaning * mix(HULL_SHARE, 1.0, sea);
            out.outward = outward;
            // Torn off in streaks that lie along the gust, out from the blast, thick and thin by turns.
            let around = normalize(spoke.xz + vec2<f32>(1e-4, 0.0));
            let streaks = value_noise(vec3<f32>(around * 11.0, range * 0.07 + shock.bore.w));
            let tufts = value_noise(vec3<f32>(spoke.xz * 0.45, shock.bore.w + 3.7));
            let torn = 0.25 + 1.9 * streaks * streaks + 0.5 * tufts;
            out.ruffle = min(shock.surface.y * smoothstep(0.0, RUFFLE_RISE, within) * exp(-within / RUFFLE_TRAIL)
                * leaning * sea * torn, 0.5);
        }
    }

    if shock.band.y > 0.0 {
        let pulse_m = shock.band.x;
        let pulse = swell((radius - miss) / pulse_m / SWELL);
        // A pulse finer than two pixels is seen as its mean.
        let resolved = saturate(pulse_m / (2.0 * max(along, 1.0) * shocks.params.x));
        let turn = min(shock.band.y, MAX_FOLD * pulse_m / max(along, 1.0)) * pulse * sqrt(lean(shock, -toward)) * resolved;
        var total = 0.0;
        if near > 0.0 && behind > near {
            total += turn * beyond(near, behind);
        }
        if behind > far {
            total += turn * beyond(far, behind);
        }
        out.turn = total;
        out.toward = toward;
    }

    let outer = shock.veil.y;
    if shock.veil.z > 0.0 && miss < outer {
        // The ray's two stretches inside the shell and before the scene, one each side of its nearest point to the
        // centre (or of the hollow).
        let reach = sqrt(outer * outer - miss_squared);
        let inner = shock.veil.x;
        let hollow = sqrt(max(inner * inner - miss_squared, 0.0));
        out.depth = veil_depth(shock, ray, to, max(along - reach, 0.0), min(along - hollow, behind))
            + veil_depth(shock, ray, to, max(along + hollow, 0.0), min(along + reach, behind));
    }
    return out;
}

// Where on the screen (UV) a clip position lands.
fn screen(clip: vec4<f32>) -> vec2<f32> {
    return clip.xy / clip.w * vec2<f32>(0.5, -0.5) + 0.5;
}

// How far (UV) the backdrop is read from `uv`, the fragment's own place: along the turned ray, and at the scene's
// point pushed out from the blast.
fn shift_of(trace: Trace, uv: vec2<f32>) -> vec2<f32> {
    var shift = vec2<f32>(0.0);
    if trace.turn > MIN_TURN {
        let bent = normalize(trace.ray + trace.toward * trace.turn);
        shift += screen(camera.view_projection * vec4<f32>(camera.position + bent * 1000.0, 1.0)) - uv;
    }
    if trace.push > 0.0 {
        let point = camera.position + trace.ray * trace.behind + trace.outward * trace.push;
        shift += screen(camera.view_projection * vec4<f32>(point, 1.0)) - uv;
    }
    return shift;
}

@fragment
fn shock_fragment(in: ShockOut) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    let trace = trace_shock(in.index, pixel);
    let mist = 1.0 - exp(-trace.depth) * (1.0 - trace.ruffle);
    let size = vec2<f32>(textureDimensions(backdrop_color));
    let shift = shift_of(trace, pixel / size);
    var moved = vec3<f32>(0.0);
    if dot(shift * size, shift * size) > 1e-3 {
        let uv = clamp(pixel / size + shift, vec2<f32>(0.5) / size, vec2<f32>(1.0) - vec2<f32>(0.5) / size);
        // What is drawn there must lie behind the front too, or the front would bend what stands before it.
        let forward = -camera.inverse_view[2].xyz;
        let there = view_depth(textureLoad(backdrop_depth, vec2<i32>(uv * size), 0))
            / max(dot(view_ray(vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0)), forward), 1e-4);
        if there > trace.crossing {
            moved = textureSampleLevel(backdrop_color, linear_clamp, uv, 0.0).rgb
                - textureLoad(backdrop_color, vec2<i32>(pixel), 0).rgb;
        }
    } else if mist < 0.002 {
        discard;
    }
    // Over what is there: the scene moved by the turn and the ripple, under the mist and the ruffled sea. The mist is
    // never darker than what it lies over: at night a flash lights the sea under it, and it with the sea.
    let here = textureLoad(backdrop_color, vec2<i32>(pixel), 0).rgb;
    let pale = max(shocks.radiance.rgb, here * 1.1);
    return vec4<f32>(moved * (1.0 - mist) + pale * mist, mist);
}

@fragment
fn shock_motion(in: ShockOut) -> @location(0) vec4<f32> {
    let trace = trace_shock(in.index, in.position.xy);
    let size = camera.viewport.xy;
    let shift = length(shift_of(trace, in.position.xy / size) * size);
    let response = max(saturate(shift / shocks.params.y), 1.0 - exp(-trace.depth) * (1.0 - trace.ruffle));
    if response < 0.02 {
        discard;
    }
    return motion_response(response);
}
