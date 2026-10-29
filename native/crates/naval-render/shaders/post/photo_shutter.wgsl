// Photo mode's shutter (native/docs/photo.md, `naval_render::shutter`). No TypeScript source: the browser game had no
// photo mode. Time stands still in photo mode, so nothing moves between frames; the shutter blurs the scene by what
// would move while it is open. Each pixel's world point takes the velocity of the body holding it (a hull with her
// turn, an airframe, a round and its tracer, a splash's rising water; the rest of the world stands still), relative to
// the camera (held still, or riding with the subject when panning), and its radiance is spread along the line that
// point would draw on the sensor.
//
// Passes (each a full-screen triangle):
//   `velocity` (full: half the pixel's travel while the shutter is open, in pixels, at most `max_radius`; the depth of
//     what is there, the nearer of the scene's and a splash's), or in play `motion_velocity` (the film camera,
//     `naval_render::play_camera`: the pixel's travel since the last frame by temporal AA's motion, scaled to the
//     exposure),
//   `reduce` twice (1/4, then 1/16: the longest travel in each 16 × 16 tile),
//   `neighbours` (1/16: the longest travel among the tiles near enough to reach this one),
//   `gather` (full: the radiance gathered along the neighbourhood's longest travel by the reconstruction filter of
//     McGuire, Hennessy, Bukowski and Osman, "A Reconstruction Filter for Plausible Motion Blur" (2012), sampled in
//     mirrored pairs as Jimenez's "Next Generation Post Processing in Call of Duty: Advanced Warfare" (2014): a sample
//     in front of the pixel counts where its own travel reaches the pixel, one behind it where the pixel's travel
//     reaches the sample, so a sharp hull never takes the streaking sea behind her while a moving one smears over a
//     still sea).
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}

struct ShutterParams {
    // The scene's projection (with temporal AA's jitter) inverted: a pixel's depth to view space.
    inverse_projection: mat4x4<f32>,
    // The projection without the jitter: view space to clip.
    projection: mat4x4<f32>,
    // The velocity of a still point of the world relative to the camera, view space, m/s (zero for a still camera).
    world_velocity: vec4<f32>,
    // The viewport (pixels) and its tile grid.
    size: vec2<f32>,
    tiles: vec2<f32>,
    // How long the shutter is open, seconds.
    exposure: f32,
    // The longest half-travel drawn, pixels.
    max_radius: f32,
    // Bodies in `bodies`.
    body_count: u32,
    // Tiles each way the neighbourhood reaches.
    reach: i32,
    // The most sample pairs one pixel's gather takes, and the pixels between pairs it aims for.
    max_pairs: f32,
    pair_spacing: f32,
    // The camera's near plane, metres.
    near: f32,
    // The fewest sample pairs a moving pixel's gather takes.
    min_pairs: f32,
    // In play (`motion_velocity`): view space to the last frame's clip space (its unjittered view-projection after this
    // frame's inverse view), and in x the share of a frame's travel the shutter draws (its exposure over the frames'
    // interval).
    reproject: mat4x4<f32>,
    motion: vec4<f32>,
}

// A moving body in view space (`naval_render::shutter::MovingBody`).
struct Body {
    // Its centre; w: the radius of a sphere holding all of it.
    center: vec4<f32>,
    // Hull: her bow (level), w half her length. Capsule: from the head back along its trail, w the trail's length.
    // Column: up, w its height.
    axis: vec4<f32>,
    // Hull: up, w half her beam. Sphere, capsule and column: w the radius.
    up: vec4<f32>,
    // Hull: her bottom, the top of the waterline band and her top (metres along up from the centre). Column: the
    // seconds since the water was thrown (x) and the speed of the water at its foot (y, m/s). w: the kind.
    extent: vec4<f32>,
    // Its velocity relative to the camera (m/s), and its angular velocity (rad/s).
    velocity: vec4<f32>,
    spin: vec4<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: ShutterParams;
@group(0) @binding(3) var aux: texture_2d<f32>;
@group(0) @binding(4) var aux2: texture_2d<f32>;
@group(0) @binding(5) var<storage, read> bodies: array<Body>;
@group(0) @binding(6) var linear_clamp: sampler;

const TILE: i32 = 16;
// The depth written for the sky (metres; within half floats).
const SKY_DEPTH: f32 = 65000.0;
// Two samples within this share of the nearer one's depth are at the same depth, softly.
const SOFT_DEPTH: f32 = 0.02;
// A hull's box above her waterline band reaches this far past her length and beam (flare, boats, yards).
const HULL_MARGIN: f32 = 1.5;
const KIND_HULL: f32 = 0.0;
const KIND_SPHERE: f32 = 1.0;
const KIND_CAPSULE: f32 = 2.0;
const KIND_COLUMN: f32 = 3.0;
const GRAVITY: f32 = 9.81;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// The view-space point drawn at `pixel` (pixel centres) with reversed depth `d`.
fn view_position(pixel: vec2<f32>, d: f32) -> vec3<f32> {
    let ndc = vec2<f32>(pixel.x / params.size.x * 2.0 - 1.0, 1.0 - pixel.y / params.size.y * 2.0);
    let p = params.inverse_projection * vec4<f32>(ndc, d, 1.0);
    return p.xyz / p.w;
}

// Where a view-space point lands on the viewport, pixels; a point behind the camera is held at its near plane.
fn screen(p: vec3<f32>) -> vec2<f32> {
    let clip = params.projection * vec4<f32>(p.xy, min(p.z, -params.near), 1.0);
    let ndc = clip.xy / clip.w;
    return vec2<f32>((ndc.x * 0.5 + 0.5) * params.size.x, (0.5 - ndc.y * 0.5) * params.size.y);
}

// Whether hull `body` holds `r` (from her centre): her box from bottom to top, but at her waterline only her
// waterplane, a superellipse (finer forward than aft), so the sea round her bow and quarters stays the sea's.
fn in_hull(body: Body, r: vec3<f32>) -> bool {
    let along = dot(r, body.axis.xyz);
    let up = dot(r, body.up.xyz);
    if (up < body.extent.x || up > body.extent.z) {
        return false;
    }
    let across = length(r - body.axis.xyz * along - body.up.xyz * up);
    if (up > body.extent.y) {
        return abs(along) <= body.axis.w + HULL_MARGIN && across <= body.up.w + HULL_MARGIN;
    }
    let n = select(3.0, 2.0, along > 0.0);
    return pow(abs(along) / body.axis.w, n) + pow(across / body.up.w, n) <= 1.0;
}

// The velocity of a view-space point relative to the camera: the body's holding it, else the still world's.
fn point_velocity(p: vec3<f32>) -> vec3<f32> {
    for (var i = 0u; i < params.body_count; i++) {
        let body = bodies[i];
        let r = p - body.center.xyz;
        if (dot(r, r) > body.center.w * body.center.w) {
            continue;
        }
        let kind = body.extent.w;
        var inside = false;
        if (kind == KIND_HULL) {
            inside = in_hull(body, r);
        } else if (kind == KIND_SPHERE) {
            inside = dot(r, r) <= body.up.w * body.up.w;
        } else if (kind == KIND_CAPSULE) {
            let off = r - body.axis.xyz * clamp(dot(r, body.axis.xyz), 0.0, body.axis.w);
            inside = dot(off, off) <= body.up.w * body.up.w;
        }
        if (inside) {
            return body.velocity.xyz + cross(body.spin.xyz, r);
        }
    }
    return params.world_velocity.xyz;
}

// The water of a splash column at `r` from its foot: thrown from the foot `t` seconds ago, it rises (and falls) as a
// ballistic jet, so the water at height h moves up at h / t − g t / 2 and spreads at the foot's speed out from the axis.
fn column_velocity(body: Body, r: vec3<f32>) -> vec3<f32> {
    let t = max(body.extent.x, 0.05);
    let h = max(dot(r, body.axis.xyz), 0.0);
    let out = r - body.axis.xyz * h;
    let spread = select(vec3<f32>(0.0), normalize(out) * body.extent.y, dot(out, out) > 1e-6);
    return body.velocity.xyz + body.axis.xyz * (h / t - 0.5 * GRAVITY * t) + spread;
}

// The nearest splash column the view ray towards `ray` (unit, view space) enters before `limit` metres: the distance
// along the ray, and the column's index (-1 for none).
fn column_hit(ray: vec3<f32>, limit: f32) -> vec2<f32> {
    var nearest = limit;
    var hit = -1.0;
    for (var i = 0u; i < params.body_count; i++) {
        let body = bodies[i];
        if (body.extent.w != KIND_COLUMN) {
            continue;
        }
        // The ray from the camera against the cylinder up `axis` from its foot `center`, `up.w` round and `axis.w` tall.
        let a = body.axis.xyz;
        let o = -body.center.xyz;
        let ray_across = ray - a * dot(ray, a);
        let o_across = o - a * dot(o, a);
        let qa = dot(ray_across, ray_across);
        let qb = dot(ray_across, o_across);
        let qc = dot(o_across, o_across) - body.up.w * body.up.w;
        let disc = qb * qb - qa * qc;
        if (qa < 1e-8 || disc < 0.0) {
            continue;
        }
        let root = sqrt(disc);
        var enter = (-qb - root) / qa;
        var leave = (-qb + root) / qa;
        // Within its height: the slab from its foot to its top along the ray.
        let rise = dot(ray, a);
        let foot = dot(o, a);
        if (abs(rise) > 1e-6) {
            let t0 = -foot / rise;
            let t1 = (body.axis.w - foot) / rise;
            enter = max(enter, min(t0, t1));
            leave = min(leave, max(t0, t1));
        } else if (foot < 0.0 || foot > body.axis.w) {
            continue;
        }
        enter = max(enter, params.near);
        if (enter <= leave && enter < nearest) {
            nearest = enter;
            hit = f32(i);
        }
    }
    return vec2<f32>(nearest, hit);
}

@fragment
fn velocity(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    let d = textureLoad(depth, vec2<i32>(pixel), 0);
    let sky = d <= 0.0;
    // The sky at infinity: a camera that only moves leaves it where it is.
    var p = view_position(pixel, select(d, 1e-6, sky));
    var v = vec3<f32>(0.0);
    var z = SKY_DEPTH;
    if (!sky) {
        v = point_velocity(p);
        z = -p.z;
    }
    // A splash's water in front of what is drawn there takes the water's velocity and depth.
    let ray = normalize(p);
    let column = column_hit(ray, select(length(p), 1e9, sky));
    if (column.y >= 0.0) {
        let body = bodies[u32(column.y)];
        p = ray * column.x;
        v = column_velocity(body, p - body.center.xyz);
        z = -p.z;
    }
    let half = v * (params.exposure * 0.5);
    var travel = (screen(p + half) - screen(p - half)) * 0.5;
    let reach = length(travel);
    if (reach > params.max_radius) {
        travel *= params.max_radius / reach;
    }
    return vec4<f32>(travel, z, 0.0);
}

// The film camera in play: the pixel's travel since the last frame, temporal AA's way (the camera's from its depth and
// the last frame's view-projection, the sky's from its direction alone; a moving surface's own from the motion pass,
// `aux`, in the last frame's NDC), times the share of it the shutter draws; half of that, as `velocity` writes.
@fragment
fn motion_velocity(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let pixel = in.position.xy;
    let d = textureLoad(depth, vec2<i32>(pixel), 0);
    let sky = d <= 0.0;
    let p = view_position(pixel, select(d, 1.0, sky));
    // The sky at infinity: a direction, so only the camera's turn moves it.
    let point = vec4<f32>(p, select(1.0, 0.0, sky));
    let now = params.projection * point;
    let before = params.reproject * point;
    if (now.w <= 1e-6 || before.w <= 1e-6) {
        return vec4<f32>(0.0, 0.0, select(-p.z, SKY_DEPTH, sky), 0.0);
    }
    let own = textureLoad(aux, vec2<i32>(pixel), 0).xy;
    let ndc = now.xy / now.w - (before.xy / before.w - own);
    var travel = ndc * vec2<f32>(0.5, -0.5) * params.size * (params.motion.x * 0.5);
    let reach = length(travel);
    if (reach > params.max_radius) {
        travel *= params.max_radius / reach;
    }
    return vec4<f32>(travel, select(-p.z, SKY_DEPTH, sky), 0.0);
}

// The longest travel among 4 × 4 texels of `aux`: the velocities to a quarter of their resolution, then the quarter to
// the 16 × 16 tiles (two passes, so each fragment reads 16 texels, not 256).
@fragment
fn reduce(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.position.xy) * 4;
    let last = vec2<i32>(textureDimensions(aux)) - vec2<i32>(1);
    var longest = vec2<f32>(0.0);
    var longest_sq = 0.0;
    for (var y = 0; y < 4; y++) {
        for (var x = 0; x < 4; x++) {
            let v = textureLoad(aux, min(base + vec2<i32>(x, y), last), 0).xy;
            let sq = dot(v, v);
            if (sq > longest_sq) {
                longest_sq = sq;
                longest = v;
            }
        }
    }
    return vec4<f32>(longest, 0.0, 1.0);
}

@fragment
fn neighbours(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let tile = vec2<i32>(in.position.xy);
    let last = vec2<i32>(params.tiles) - vec2<i32>(1);
    var longest = vec2<f32>(0.0);
    var longest_sq = 0.0;
    for (var y = -params.reach; y <= params.reach; y++) {
        for (var x = -params.reach; x <= params.reach; x++) {
            let v = textureLoad(aux2, clamp(tile + vec2<i32>(x, y), vec2<i32>(0), last), 0).xy;
            let sq = dot(v, v);
            // The nearest pixels of that tile and this one are this far apart: a shorter travel cannot reach here.
            let gap = max(abs(vec2<f32>(f32(x), f32(y))) - 1.0, vec2<f32>(0.0)) * f32(TILE);
            if (sq > longest_sq && dot(gap, gap) <= sq) {
                longest_sq = sq;
                longest = v;
            }
        }
    }
    return vec4<f32>(longest, 0.0, 1.0);
}

// x: the sample lies behind the pixel; y: in front of it. Soft across a band `SOFT_DEPTH` of the nearer depth.
fn depth_compare(center: f32, sample: f32) -> vec2<f32> {
    let s = (sample - center) / (SOFT_DEPTH * max(min(center, sample), 0.1));
    return clamp(vec2<f32>(0.5 + s, 0.5 - s), vec2<f32>(0.0), vec2<f32>(1.0));
}

// How fully a travel of `spread` pixels covers a sample `offset` pixels away, pairs `spacing` pixels apart (none for a
// pixel that does not move).
fn covers(spread: f32, offset: f32, spacing: f32) -> f32 {
    return clamp(spread / spacing - max(offset / spacing - 1.0, 0.0), 0.0, 1.0);
}

// Jimenez's interleaved gradient noise: a different offset for the pairs of neighbouring pixels.
fn noise(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment
fn gather(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let p = in.position.xy;
    let center = textureLoad(source, vec2<i32>(p), 0);
    let dominant = textureLoad(aux2, vec2<i32>(p) / TILE, 0).xy;
    let length_n = length(dominant);
    if (length_n < 0.5) {
        return center;
    }
    let own = textureLoad(aux, vec2<i32>(p), 0);
    let center_spread = length(own.xy);
    let center_depth = own.z;
    let pairs = clamp(ceil(length_n / params.pair_spacing), params.min_pairs, params.max_pairs);
    let spacing = length_n / pairs;
    // Every other pair along the pixel's own travel where it has one (Guertin, McGuire and Nowrouzezahrai 2014), so
    // an edge moving across the neighbourhood's longest travel keeps its own line.
    let direction_n = dominant / length_n;
    let direction_c = select(direction_n, own.xy / max(center_spread, 1e-6), center_spread >= 0.5);
    let jitter = noise(p) - 0.5;
    let last = vec2<i32>(params.size) - vec2<i32>(1);
    var color = vec3<f32>(0.0);
    var weight = 0.0;
    for (var i = 0.0; i < pairs; i += 1.0) {
        let offset = (i + 0.5 + jitter * 0.95) * spacing;
        let step = select(direction_n, direction_c, (u32(i) & 1u) == 1u) * offset;
        let at0 = p + step;
        let at1 = p - step;
        let s0 = textureLoad(aux, clamp(vec2<i32>(floor(at0)), vec2<i32>(0), last), 0);
        let s1 = textureLoad(aux, clamp(vec2<i32>(floor(at1)), vec2<i32>(0), last), 0);
        let c0 = textureSampleLevel(source, linear_clamp, at0 / params.size, 0.0).rgb;
        let c1 = textureSampleLevel(source, linear_clamp, at1 / params.size, 0.0).rgb;
        let spread0 = length(s0.xy);
        let spread1 = length(s1.xy);
        let near0 = depth_compare(center_depth, s0.z);
        let near1 = depth_compare(center_depth, s1.z);
        let pixel_covers = covers(center_spread, offset, spacing);
        var w0 = near0.x * pixel_covers + near0.y * covers(spread0, offset, spacing);
        var w1 = near1.x * pixel_covers + near1.y * covers(spread1, offset, spacing);
        // The mirror: a background sample on one side stands in for the background the foreground hides on the other.
        let mirror = vec2<bool>(s0.z > s1.z, spread1 > spread0);
        w0 = select(w0, w1, all(mirror));
        w1 = select(w0, w1, any(mirror));
        color += w0 * c0 + w1 * c1;
        weight += w0 + w1;
    }
    let samples = 2.0 * pairs;
    return vec4<f32>(color / samples + (1.0 - weight / samples) * center.rgb, center.a);
}
