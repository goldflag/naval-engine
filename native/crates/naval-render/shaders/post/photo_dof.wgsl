// Photo mode's depth of field (native/docs/photo.md, `naval_render::photo`). No TypeScript source: the browser game had
// no photo mode. A thin lens's circle of confusion from the scene's depth, gathered at half resolution after Dennis
// Gustafsson's single-pass bokeh ("Bokeh depth of field in a single pass", 2018): a golden-angle spiral whose samples
// each count where their own circle covers the pixel, one behind the pixel limited to twice the pixel's own circle so a
// sharp edge never takes the blurred background behind it.
//
// Passes (each a full-screen triangle): `prefilter` (full to half: radiance, and the signed circle of the nearest of the
// four pixels, in half-resolution pixels), `tiles` (the largest circle in each 8 × 8 tile), `gather` (half: the spiral
// out to the largest circle among the tile and its neighbours; alpha the near blur that reached the pixel) and
// `composite` (full: the sharp pixel where its own circle and the near blur reaching it are under a pixel, else the
// gathered one).
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}

struct DofParams {
    // Metres to the plane in focus.
    focus: f32,
    // The circle of confusion's diameter, in full-resolution pixels, of a point at infinity: one `z` metres away has
    // `coc_scale * (1 - focus / z)`, negative nearer than the focus.
    coc_scale: f32,
    // The camera's planes, to read reversed depth as metres.
    near: f32,
    far: f32,
    // The largest circle's radius drawn, in half-resolution pixels.
    max_radius: f32,
    // The most samples one pixel's gather takes.
    max_samples: f32,
    full_size: vec2<f32>,
    half_size: vec2<f32>,
    padding: vec2<f32>,
    // The aperture's blades (0 round), their angle in the frame drawn (a vertex there) and the polygon's share of
    // the shape (`naval_render::photo_optics::aperture_shape`): the bokeh's shape.
    aperture: vec4<f32>,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: DofParams;
@group(0) @binding(3) var aux: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;

const GOLDEN_ANGLE: f32 = 2.39996323;
const TILE: i32 = 8;
const PI: f32 = 3.14159265;

// The aperture's edge at `angle` (radians, in the frame drawn) over its circumradius: 1 for a round one, else its
// blades' polygon blended from the circle by the iris's closing.
fn aperture_edge(angle: f32) -> f32 {
    let n = params.aperture.x;
    let polygon = params.aperture.z;
    if (n < 3.0 || polygon <= 0.0) {
        return 1.0;
    }
    let sector = 2.0 * PI / n;
    let a = angle - params.aperture.y;
    let b = a - sector * floor(a / sector) - 0.5 * sector;
    return mix(1.0, cos(PI / n) / cos(b), polygon);
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// Metres along the view axis from reversed depth (1 at the near plane, 0 at the far plane, the sky's clear).
fn view_depth(d: f32) -> f32 {
    return params.far * params.near / (d * (params.far - params.near) + params.near);
}

// The signed radius, in half-resolution pixels, of the circle a point `z` metres away blurs to.
fn half_radius(z: f32) -> f32 {
    let diameter = params.coc_scale * (1.0 - params.focus / max(z, 1e-3));
    return clamp(diameter * 0.25, -params.max_radius, params.max_radius);
}

@fragment
fn prefilter(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.position.xy) * 2;
    let last = vec2<i32>(params.full_size) - vec2<i32>(1);
    var color = vec3<f32>(0.0);
    var nearest = 1e9;
    for (var i = 0; i < 4; i++) {
        let p = min(base + vec2<i32>(i & 1, i >> 1u), last);
        color += textureLoad(source, p, 0).rgb;
        nearest = min(nearest, view_depth(textureLoad(depth, p, 0)));
    }
    return vec4<f32>(color * 0.25, half_radius(nearest));
}

@fragment
fn tiles(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let base = vec2<i32>(in.position.xy) * TILE;
    let last = vec2<i32>(params.half_size) - vec2<i32>(1);
    var largest = 0.0;
    for (var y = 0; y < TILE; y++) {
        for (var x = 0; x < TILE; x++) {
            largest = max(largest, abs(textureLoad(source, min(base + vec2<i32>(x, y), last), 0).a));
        }
    }
    return vec4<f32>(largest, 0.0, 0.0, 1.0);
}

@fragment
fn gather(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let p = in.position.xy;
    let last = vec2<i32>(params.half_size) - vec2<i32>(1);
    let center = textureLoad(source, vec2<i32>(p), 0);
    let center_size = abs(center.a);
    // The reach: the largest circle in this tile and its neighbours, any of whose blur may land here.
    let tile = vec2<i32>(p) / TILE;
    let tiles_last = vec2<i32>(textureDimensions(aux)) - vec2<i32>(1);
    var reach = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            reach = max(reach, textureLoad(aux, clamp(tile + vec2<i32>(x, y), vec2<i32>(0), tiles_last), 0).r);
        }
    }
    reach = min(reach, params.max_radius);
    if (reach < 0.5) {
        return vec4<f32>(center.rgb, 0.0);
    }
    // The spiral's spacing: each sample adds about `2 * step` to the radius squared, so it ends within the budget.
    let spacing = max(0.5, reach * reach / (2.0 * params.max_samples));
    var color = center.rgb;
    var total = 1.0;
    var spill = 0.0;
    var radius = spacing;
    var angle = 0.0;
    for (var i = 0; i < 1024; i++) {
        if (radius > reach + 0.5) {
            break;
        }
        let at = clamp(vec2<i32>(floor(p + vec2<f32>(cos(angle), sin(angle)) * radius)), vec2<i32>(0), last);
        let s = textureLoad(source, at, 0);
        var size = abs(s.a);
        // Behind the pixel: what is in front hides it, so it blurs no wider than the pixel's own circle allows.
        if (s.a > center.a) {
            size = min(size, center_size * 2.0);
        }
        // Its blur is the aperture's shape (turned over in front of the focus), reaching the pixel from it.
        let edge = size * aperture_edge(angle + select(PI, 0.0, s.a < 0.0));
        let m = smoothstep(radius - 0.5, radius + 0.5, edge);
        color += mix(color / total, s.rgb, m);
        total += 1.0;
        if (s.a < center.a) {
            spill = max(spill, size * m);
        }
        radius += spacing / radius;
        angle += GOLDEN_ANGLE;
    }
    return vec4<f32>(color / total, spill);
}

@fragment
fn composite(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.position.xy);
    let sharp = textureLoad(source, p, 0);
    let own = abs(half_radius(view_depth(textureLoad(depth, p, 0))));
    let blurred = textureSampleLevel(aux, linear_clamp, in.uv, 0.0);
    let t = smoothstep(0.25, 1.0, max(own, blurred.a));
    return vec4<f32>(mix(sharp.rgb, blurred.rgb, t), sharp.a);
}
