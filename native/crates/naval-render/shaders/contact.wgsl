// Contact shadows (native only, 2026-10-20; `naval_render::contact`, native/docs/render-core/frame.md "Contact
// shadows").
//
// The sun's maps leave out what stands close above a surface: their receiver offsets lift it 20 to 50 cm off itself on a
// battleship's near map (a texel and a half along the normal, two along the light, at 15 cm texels), and their filter
// spreads an edge over six texels, so a ledge's shadow begins a hand's breadth below it and a rail's, a barrel's or a
// mount's thin shadow blurs away. In sunlight that shadow is crisp: a ledge a metre up casts an edge about a centimetre
// soft. Here each pixel of the surfaces that take contact shadows (the hulls, drawn depth-only for this frame's camera
// before the scene: `SceneLayer::draw_contact_depth`) walks toward the light through that depth, a pixel a step, and
// is in shadow where a surface stands in front of the walk by less than a thickness: a part close above it, as the eye
// sees it. The walk spans `REACH_PIXELS` of the receiver's own footprint, at most `REACH_M`, so it covers the same few
// pixels near or far and stays inside what the maps leave out; it fades out where it would span under `MIN_PIXELS`.
// A still camera takes the same steps each frame (no noise while the walk is at most `MAX_STEPS` pixels long), and
// temporal AA's jitter gives the shadow's edges their coverage.
//
// The mask is one channel, 1 lit and 0 shadowed, the shadows' intensity applied. A receiver reads it at its own pixel
// and takes the darker of it and the maps' for the sun alone (`ship.wgsl`'s `lit`), never for the sky's light.

#import naval::camera::camera
#import naval::shadow::shadows
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}

// The surfaces that take contact shadows, as the frame's camera sees them (reversed depth, 0 where none).
@group(1) @binding(0) var contact_depth: texture_depth_2d;

// The walk's length in the receiver's footprints (a pixel's width there), and the most it reaches (m).
const REACH_PIXELS: f32 = 24.0;
const REACH_M: f32 = 1.0;
// The most steps a walk takes: one a pixel up to this length, then spread evenly with a new start each frame.
const MAX_STEPS: u32 = 24u;
// A walk spanning fewer pixels than this finds little the maps miss (their texels are finer than its pixels there, and
// their offsets a pixel or so); it fades in up to `FULL_PIXELS`.
const MIN_PIXELS: f32 = 3.0;
const FULL_PIXELS: f32 = 5.0;
// How thick a surface in the depth is taken to be (m, and the receiver's footprints, whichever is more): a walk passing
// further behind it than that passes behind it, not through it.
const THICKNESS_M: f32 = 0.3;
const THICKNESS_PIXELS: f32 = 4.0;
// The receiver's own surface, seen at a neighbour's texel centre rather than where the walk crosses that texel: its
// depth there may differ by its slope over most of a pixel. And a floor, in footprints.
const SLOPE_BIAS: f32 = 0.75;
const BIAS_PIXELS: f32 = 0.1;
// Where along the walk (its share) a surface's shadow starts to fade, so the walk's end draws no line.
const FADE_FROM: f32 = 0.6;

@vertex
fn vs_contact(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// The view-space point drawn at the pixel `offset` from `texel`.
fn neighbour(texel: vec2<i32>, offset: vec2<i32>) -> vec3<f32> {
    let at = texel + offset;
    return view_point(vec2<f32>(at) + 0.5, textureLoad(contact_depth, at, 0));
}

// Of the steps to a pixel's two neighbours along one axis, the one on the same surface: the shorter along the view.
fn surface_step(p: vec3<f32>, before: vec3<f32>, after: vec3<f32>) -> vec3<f32> {
    return select(after - p, p - before, abs(p.z - before.z) < abs(after.z - p.z));
}

// The view-space point (the camera looking down −Z) at framebuffer position `pixel` (y down) and reversed `depth`.
fn view_point(pixel: vec2<f32>, depth: f32) -> vec3<f32> {
    let ndc = vec2<f32>(pixel.x * camera.viewport.z * 2.0 - 1.0, 1.0 - pixel.y * camera.viewport.w * 2.0);
    let p = camera.inverse_projection * vec4<f32>(ndc, depth, 1.0);
    return p.xyz / p.w;
}

// Where view-space point `p` lands on the framebuffer (pixels, y down).
fn pixel_of(p: vec3<f32>) -> vec2<f32> {
    let clip = camera.projection * vec4<f32>(p, 1.0);
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5) * camera.viewport.xy;
}

// Interleaved gradient noise (Jimenez 2014) in [0, 1), moved on each frame.
fn frame_noise(texel: vec2<i32>) -> f32 {
    let p = vec2<f32>(texel) + 5.588238 * f32(camera.frame % 64u);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn fs_contact(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(in.position.xy);
    let depth = textureLoad(contact_depth, texel, 0);
    let intensity = shadows.settings.x;
    if depth <= 0.0 || intensity <= 0.0 {
        return vec4<f32>(1.0);
    }
    let p = view_point(in.position.xy, depth);
    let distance = -p.z;
    let footprint = 2.0 * distance * camera.tan_half_fov_y * camera.viewport.w;
    var reach = min(REACH_PIXELS * footprint, REACH_M);
    let span = reach / footprint;
    if span < MIN_PIXELS {
        return vec4<f32>(1.0);
    }
    let toward = normalize((camera.view * vec4<f32>(shadows.toward_light.xyz, 0.0)).xyz);
    // The receiver's surface, from its neighbours on the same side of any edge: a face turned from the light takes no
    // sun to shade (its lighting's own cosine), and its distance's slope per pixel sizes the bias.
    let across = surface_step(p, neighbour(texel, vec2<i32>(-1, 0)), neighbour(texel, vec2<i32>(1, 0)));
    let down = surface_step(p, neighbour(texel, vec2<i32>(0, -1)), neighbour(texel, vec2<i32>(0, 1)));
    let face = cross(across, down);
    let facing = select(face, -face, dot(face, p) > 0.0);
    if dot(facing, toward) <= 0.0 {
        return vec4<f32>(1.0);
    }
    let slope = length(vec2<f32>(across.z, down.z));
    // A walk toward the camera stops short of its near plane.
    if toward.z > 0.0 {
        reach = min(reach, max(distance - 2.0 * camera.near, 0.0) / toward.z);
    }
    let end = p + toward * reach;
    let from_pixel = in.position.xy;
    let to_pixel = pixel_of(end);
    // Reversed depth is `k (1 / distance − 1 / far)`, linear across the screen along the walk as 1 / distance is, so the
    // walk's own depth interpolates and a surface's lead on it compares in depth: a lead of `m` metres at the receiver's
    // distance is `m k / distance²` there (the walk spans a few hundredths of the distance, so as near as matters).
    let k = camera.far * camera.near / (camera.far - camera.near);
    let depth_to = k * (1.0 / max(-end.z, camera.near) - 1.0 / camera.far);
    let per_metre = k / (distance * distance);
    let pixels = length(to_pixel - from_pixel);
    let bias = (SLOPE_BIAS * slope + BIAS_PIXELS * footprint) * per_metre;
    let thickness = max(THICKNESS_M, THICKNESS_PIXELS * footprint) * per_metre + bias;
    let steps = u32(clamp(ceil(pixels), 1.0, f32(MAX_STEPS)));
    let start = select(0.0, frame_noise(texel), pixels > f32(MAX_STEPS));
    var shadow = 0.0;
    for (var i = 1u; i <= steps; i++) {
        let along = (f32(i) - start) / f32(steps);
        let at_pixel = mix(from_pixel, to_pixel, along);
        if any(at_pixel < vec2<f32>(0.0)) || any(at_pixel >= camera.viewport.xy) {
            break;
        }
        let at = vec2<i32>(at_pixel);
        if all(at == texel) {
            continue;
        }
        // How far the surface seen there stands in front of the walk (in depth).
        let before = textureLoad(contact_depth, at, 0) - mix(depth, depth_to, along);
        if before > bias && before < thickness {
            shadow = max(shadow, 1.0 - smoothstep(FADE_FROM, 1.0, along));
            if shadow >= 1.0 {
                break;
            }
        }
    }
    let fade = smoothstep(MIN_PIXELS, FULL_PIXELS, span);
    return vec4<f32>(1.0 - shadow * fade * intensity);
}
