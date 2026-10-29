// How much of the sun's disc the frame shows, for its glare in the eye (`naval_render::sun_glare`; the glare itself is
// the bloom's composite's, `sun_glare` in bloom.wgsl). The disc the sky draws is measured where it falls in the frame,
// as photo mode's lens measures it (`sun` in photo_optics.wgsl): its radiance over the sky about it against the radiance
// it has unobstructed, so a cloud, smoke, a hull or the horizon in front dims the glare by what it hides. Where the
// scene's alpha holds what the transparents cover (`terms.w`: the night grade's cover pass ran), a sample more than
// half covered (thick smoke, a rain streak's core) shows no sky: what it shows is the transparent's own light, which a
// low sun's faint disc behind a deck would otherwise take for itself (2026-10-20). Native only.
#import naval::color::luminance

struct SunParams {
    // The sun's direction in the camera's frame (looking down -Z), and its drawn disc's angular radius (radians).
    sun: vec4<f32>,
    // The tangent of half the view across (x) and up (y), a pixel's angle at the frame's centre, 1 with a sun.
    view: vec4<f32>,
    // The drawn disc's unobstructed luminance, the share of the way the state moves to this frame's measure, 1 for a
    // capture, which settles at once and takes nothing from the frames before it, and 1 where the scene's alpha holds
    // the transparents' cover.
    terms: vec4<f32>,
}

// The cover over which a sample shows the transparent in front rather than the sky.
const COVERED: f32 = 0.5;

// Whether the scene at `at` shows the sky: nothing opaque there (reversed depth 0), nor a transparent covering it.
fn shows_sky(at: vec2<i32>, depth_at: vec2<i32>) -> bool {
    let covered = params.terms.w > 0.5 && textureLoad(scene, at, 0).a > COVERED;
    return textureLoad(depth, depth_at, 0) <= 0.0 && !covered;
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: SunParams;
// The share of the disc seen (eased), this frame's measure, and its samples inside the frame.
@group(0) @binding(3) var<storage, read_write> state: array<f32, 4>;

const TAU: f32 = 6.28318531;
const GOLDEN_ANGLE: f32 = 2.39996323;

var<workgroup> parts: array<vec4<f32>, 64>;
var<workgroup> counts: array<vec2<f32>, 64>;

// Where the ray `offset` radians from the sun's centre, `around` about it, falls in the frame: xy in pixels, z 1 when
// it is inside the frame. The samples are laid out in angle and projected one by one, since a disc away from the view's
// axis is drawn stretched (twice as long toward the frame's corner 45 degrees off it).
fn sample_at(offset: f32, around: f32, size: vec2<f32>) -> vec3<f32> {
    let s = params.sun.xyz;
    let helper = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(s.y) > 0.9);
    let u = normalize(cross(helper, s));
    let v = cross(s, u);
    let ray = s + (u * cos(around) + v * sin(around)) * tan(offset);
    if ray.z > -1e-4 {
        return vec3<f32>(0.0);
    }
    let plane = ray.xy / (-ray.z * params.view.xy);
    let p = vec2<f32>(0.5 + 0.5 * plane.x, 0.5 - 0.5 * plane.y) * size;
    return vec3<f32>(p, select(0.0, 1.0, all(p >= vec2<f32>(0.0)) && all(p < size)));
}

@compute @workgroup_size(64)
fn measure(@builtin(local_invocation_index) local: u32) {
    let has = params.view.w > 0.5;
    let size = vec2<f32>(textureDimensions(scene));
    // The depth the scene was drawn with may be of another size (a render scale under 1).
    let depth_scale = vec2<f32>(textureDimensions(depth)) / size;
    let radius = params.sun.w;
    // x the disc's luminance where it is sky, z and w the sky's about it and its samples.
    var part = vec4<f32>(0.0);
    // A disc sample inside the frame, and whether it shows the sky (not a hull or the sea in front, however bright).
    var count = vec2<f32>(0.0);
    if has {
        if local < 48u {
            // Across the drawn disc, inside its soft rim: a golden-angle spiral.
            let i = f32(local) + 0.5;
            let a = i * GOLDEN_ANGLE;
            let found = sample_at(radius * 0.85 * sqrt(i / 48.0), a, size);
            let p = found.xy;
            if found.z > 0.5 {
                let at = vec2<i32>(p);
                let sky = shows_sky(at, vec2<i32>(p * depth_scale));
                count = vec2<f32>(1.0, select(0.0, 1.0, sky));
                if sky {
                    part.x = luminance(textureLoad(scene, at, 0).rgb);
                }
            }
        } else {
            // The sky about it: a ring outside the rim (and its soft edge, two pixels), where it is sky.
            let a = (f32(local - 48u) + 0.5) / 16.0 * TAU;
            let found = sample_at(radius * 1.6 + 2.0 * params.view.z, a, size);
            let p = found.xy;
            if found.z > 0.5 {
                let at = vec2<i32>(p);
                if shows_sky(at, vec2<i32>(p * depth_scale)) {
                    part = vec4<f32>(0.0, 0.0, luminance(textureLoad(scene, at, 0).rgb), 1.0);
                }
            }
        }
    }
    parts[local] = part;
    counts[local] = count;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if local < stride {
            parts[local] += parts[local + stride];
            counts[local] += counts[local + stride];
        }
        workgroupBarrier();
    }
    if local == 0u {
        let total = parts[0];
        let samples = counts[0];
        var seen = state[0];
        var measured = 0.0;
        if !has {
            // No sun (set, or the moon's night): no glare.
            seen = 0.0;
        } else if samples.x > 0.0 {
            // The samples a hull or the sea hides give none.
            let sky = select(0.0, total.z / max(total.w, 1.0), total.w > 0.0);
            let excess = max(total.x - samples.y * sky, 0.0) / samples.x;
            measured = clamp(excess / max(params.terms.x, 1e-6), 0.0, 1.0);
            seen += (measured - seen) * params.terms.y;
        } else if params.terms.z > 0.5 {
            // A capture must not depend on the frames before it: a sun it cannot measure gives none.
            seen = 0.0;
        }
        // Out of the frame its disc cannot be measured: the glare keeps what was last seen of it while it fades.
        state[0] = seen;
        state[1] = measured;
        state[2] = samples.x;
        state[3] = 0.0;
    }
}
