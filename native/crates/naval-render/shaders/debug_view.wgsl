// The developer panel's full-screen debug views (`naval_render::debug_view`), drawn onto the output after the post
// chain and before the layers' overlays. No TypeScript source: the browser game had no such views. Each reads the
// scene's reversed depth (at the scene's size, which the render scale may make smaller than the output) and the
// camera block it was drawn with (temporal AA's jitter included, so positions rebuilt from depth land where they were
// drawn):
//
// - `fs_normals`: each surface's face normal, rebuilt from the depth of its neighbours (the one on each side nearer in
//   depth, so silhouettes do not bend), as colour: x red, y green, z blue, `n * 0.5 + 0.5` encoded. The sky is black.
// - `fs_depth`: distance from the camera on a log scale, white at the near plane to black at 100 km, with a line at
//   every power of ten metres (10 m, 100 m, 1 km, 10 km).
// - `fs_cascades`: the sun's shadow map each surface samples (`naval::shadow::sun_shadow`'s weights), multiplied over
//   the lit frame: near red, wide green, view 1 blue, view 2 yellow; beyond the near and wide maps the land maps in
//   their place where they cover (land 1 magenta, land 2 cyan, land 3 orange); untinted where no map reaches or
//   shadows are off.
#import naval::camera::{camera, view_depth}
#import naval::color::{srgb_eotf, srgb_oetf}
#import naval::shadow::{shadows, shadows_enabled, shadow_cover, shadow_land_on, SHADOW_LAND_FIRST, SHADOW_MAPS, SHADOW_ORDER, SHADOW_HELD}

@group(1) @binding(0) var scene_depth: texture_depth_2d;

struct DebugVertex {
    @builtin(position) position: vec4<f32>,
    // 0,0 at the top left, 1,1 at the bottom right.
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_debug(@builtin(vertex_index) index: u32) -> DebugVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: DebugVertex;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// A display-space colour (sRGB-encoded, as a picture shows it) for the target: linear for one that encodes in
// hardware, as it is for one that does not.
fn shown(encoded: vec3<f32>, alpha: f32) -> vec4<f32> {
#ifdef ENCODE_SRGB
    return vec4<f32>(encoded, alpha);
#else
    return vec4<f32>(srgb_eotf(encoded), alpha);
#endif
}

fn depth_size() -> vec2<i32> {
    return vec2<i32>(textureDimensions(scene_depth));
}

fn depth_at(pixel: vec2<i32>) -> f32 {
    return textureLoad(scene_depth, clamp(pixel, vec2<i32>(0), depth_size() - 1), 0);
}

// The scene pixel under an output uv.
fn scene_pixel(uv: vec2<f32>) -> vec2<i32> {
    return clamp(vec2<i32>(uv * vec2<f32>(depth_size())), vec2<i32>(0), depth_size() - 1);
}

// The world position drawn at scene pixel `pixel` with reversed depth `depth`.
fn world_at(pixel: vec2<i32>, depth: f32) -> vec3<f32> {
    let uv = (vec2<f32>(pixel) + 0.5) / vec2<f32>(depth_size());
    let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
    let world = camera.inverse_view_projection * vec4<f32>(ndc, depth, 1.0);
    return world.xyz / world.w;
}

@fragment
fn fs_normals(in: DebugVertex) -> @location(0) vec4<f32> {
    let pixel = scene_pixel(in.uv);
    let depth = depth_at(pixel);
    if depth <= 0.0 {
        return shown(vec3<f32>(0.0), 1.0);
    }
    let here = world_at(pixel, depth);
    let distance = view_depth(depth);
    // Each axis takes the neighbour nearer in depth: across a silhouette the far one belongs to another surface.
    let left = depth_at(pixel - vec2<i32>(1, 0));
    let right = depth_at(pixel + vec2<i32>(1, 0));
    let up = depth_at(pixel - vec2<i32>(0, 1));
    let down = depth_at(pixel + vec2<i32>(0, 1));
    var across: vec3<f32>;
    if abs(view_depth(left) - distance) < abs(view_depth(right) - distance) {
        across = here - world_at(pixel - vec2<i32>(1, 0), left);
    } else {
        across = world_at(pixel + vec2<i32>(1, 0), right) - here;
    }
    var along: vec3<f32>;
    if abs(view_depth(up) - distance) < abs(view_depth(down) - distance) {
        along = here - world_at(pixel - vec2<i32>(0, 1), up);
    } else {
        along = world_at(pixel + vec2<i32>(0, 1), down) - here;
    }
    var normal = normalize(cross(along, across));
    if dot(normal, camera.position - here) < 0.0 {
        normal = -normal;
    }
    return shown(normal * 0.5 + 0.5, 1.0);
}

@fragment
fn fs_depth(in: DebugVertex) -> @location(0) vec4<f32> {
    let depth = depth_at(scene_pixel(in.uv));
    if depth <= 0.0 {
        return shown(vec3<f32>(0.0), 1.0);
    }
    let metres = max(view_depth(depth), camera.near);
    // 1 at the near plane, 0 at 100 km.
    let decades = log2(metres / camera.near) / log2(10.0);
    let span = log2(1e5 / camera.near) / log2(10.0);
    let grey = clamp(1.0 - decades / span, 0.0, 1.0);
    // A line at each power of ten metres, a pixel or so wide.
    let tens = log2(metres) / log2(10.0);
    let width = max(fwidth(tens), 1e-4) * 1.5;
    let near_line = abs(tens - round(tens));
    let line = (1.0 - smoothstep(0.0, width, near_line)) * select(0.0, 1.0, round(tens) >= 1.0);
    let colour = mix(vec3<f32>(grey), vec3<f32>(1.0, 0.55, 0.1), line * 0.8);
    return shown(colour, 1.0);
}

// Near, wide, aboard, view 1, view 2, land 1, land 2, land 3 (map index order).
const CASCADE_TINTS = array<vec3<f32>, 8>(
    vec3<f32>(1.0, 0.35, 0.35),
    vec3<f32>(0.35, 1.0, 0.35),
    vec3<f32>(0.75, 0.45, 1.0),
    vec3<f32>(0.4, 0.55, 1.0),
    vec3<f32>(1.0, 0.95, 0.3),
    vec3<f32>(1.0, 0.4, 1.0),
    vec3<f32>(0.35, 1.0, 1.0),
    vec3<f32>(1.0, 0.65, 0.25),
);

// Multiplied over the frame (the blend is `src * dst`): white leaves it as it is.
@fragment
fn fs_cascades(in: DebugVertex) -> @location(0) vec4<f32> {
    let pixel = scene_pixel(in.uv);
    let depth = depth_at(pixel);
    if depth <= 0.0 || !shadows_enabled() {
        return vec4<f32>(1.0);
    }
    let position = world_at(pixel, depth);
    // `sun_shadow`'s weights: each square map in `SHADOW_ORDER` taking by its cover what those before it leave.
    var rest = 1.0;
    var tint = vec3<f32>(0.0);
    var beyond = 1.0;
    for (var k = 0u; k < 5u; k++) {
        let i = SHADOW_ORDER[k];
        let share = shadow_cover(i, position) * shadows.maps[i].state.x;
        tint += CASCADE_TINTS[i] * rest * share;
        rest *= 1.0 - share;
        if k + 1u == SHADOW_HELD {
            beyond = rest;
        }
    }
    tint += vec3<f32>(rest);
    // Beyond the near, aboard and wide maps, the land maps' tints in place of the view maps' where they cover.
    if shadow_land_on() && beyond > 0.0 {
        var land = vec3<f32>(0.0);
        var left = 1.0;
        for (var i = SHADOW_LAND_FIRST; i < SHADOW_MAPS; i++) {
            if shadows.maps[i].state.x > 0.5 {
                let share = shadow_cover(i, position);
                land += CASCADE_TINTS[i] * left * share;
                left *= 1.0 - share;
            }
        }
        let covered = 1.0 - left;
        if covered > 0.0 {
            tint = mix(tint, land / covered, beyond * covered);
        }
    }
    // The tint is linear: a multiply in either encoding darkens alike enough for a debug view.
#ifdef ENCODE_SRGB
    return vec4<f32>(srgb_oetf(tint), 1.0);
#else
    return vec4<f32>(tint, 1.0);
#endif
}
