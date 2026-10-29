// SMAA 1x Medium with colour edge detection, as the game runs it on Ultra (`smaa(frame)` in
// the web game's `controllers/GraphicsController.ts`, on the display-space frame). Ports three.js r185
// examples/jsm/tsl/display/SMAANode.js line for line: its three passes (edges, blending weights,
// neighbourhood blending), threshold 0.1, eight search steps, its area texture (160 x 560, RG) and
// search texture (66 x 33, R, sampled nearest and scaled by 255) and its texture coordinates (y
// down, as wgpu's). Every sample is at level 0; the edges, weights and frame are read bilinearly,
// as three's render targets are. An HDR frame's edges are found in the perceptual domain
// (`naval::output::perceptual`): exactly three's below SDR white.
#import naval::fullscreen::fullscreen_vertex
#import naval::color::srgb_eotf
#import naval::output::{dither_encoded, perceptual}

const SMAA_THRESHOLD: f32 = 0.1;
const SMAA_MAX_SEARCH_STEPS: i32 = 8;
const SMAA_AREATEX_MAX_DISTANCE: f32 = 16.0;
const SMAA_AREATEX_PIXEL_SIZE: vec2<f32> = vec2<f32>(1.0 / 160.0, 1.0 / 560.0);
const SMAA_AREATEX_SUBTEX_SIZE: f32 = 1.0 / 7.0;

// Edges and blend: the frame being smoothed. Weights: the edges.
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var bilinear: sampler;
// Weights: the area and search lookups. Blend: the weights (binding 2).
@group(0) @binding(2) var lookup: texture_2d<f32>;
@group(0) @binding(3) var search_texture: texture_2d<f32>;
@group(0) @binding(4) var nearest: sampler;

fn texel() -> vec2<f32> {
    return 1.0 / vec2<f32>(textureDimensions(source));
}

fn fetch(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(source, bilinear, uv, 0.0);
}

// The frame's colour at `uv` as edge detection weighs it.
fn fetch_perceptual(uv: vec2<f32>) -> vec3<f32> {
    return perceptual(fetch(uv).rgb);
}

fn max3(v: vec3<f32>) -> f32 {
    return max(max(v.r, v.g), v.b);
}

// ---- Edges ----

struct EdgesVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset0: vec4<f32>,
    @location(2) offset1: vec4<f32>,
    @location(3) offset2: vec4<f32>,
}

@vertex
fn edges_vertex(@builtin(vertex_index) index: u32) -> EdgesVertex {
    let full = fullscreen_vertex(index, 0.0);
    let size = texel();
    var out: EdgesVertex;
    out.position = full.position;
    out.uv = full.uv;
    out.offset0 = full.uv.xyxy + size.xyxy * vec4<f32>(-1.0, 0.0, 0.0, -1.0);
    out.offset1 = full.uv.xyxy + size.xyxy * vec4<f32>(1.0, 0.0, 0.0, 1.0);
    out.offset2 = full.uv.xyxy + size.xyxy * vec4<f32>(-2.0, 0.0, 0.0, -2.0);
    return out;
}

@fragment
fn edges(in: EdgesVertex) -> @location(0) vec4<f32> {
    let threshold = vec2<f32>(SMAA_THRESHOLD);
    var delta = vec4<f32>(0.0);
    let c = fetch_perceptual(in.uv);
    // Left and top deltas.
    delta.x = max3(abs(c - fetch_perceptual(in.offset0.xy)));
    delta.y = max3(abs(c - fetch_perceptual(in.offset0.zw)));
    var found = step(threshold, delta.xy);
    // No edge: the target keeps its clear (0).
    if dot(found, vec2<f32>(1.0)) == 0.0 {
        discard;
    }
    // Right and bottom deltas, the largest in the direct neighbourhood.
    delta.z = max3(abs(c - fetch_perceptual(in.offset1.xy)));
    delta.w = max3(abs(c - fetch_perceptual(in.offset1.zw)));
    var max_delta = max(max(delta.x, delta.y), max(delta.z, delta.w));
    // Left-left and top-top deltas.
    delta.z = max3(abs(c - fetch_perceptual(in.offset2.xy)));
    delta.w = max3(abs(c - fetch_perceptual(in.offset2.zw)));
    max_delta = max(max_delta, max(delta.z, delta.w));
    // Local contrast adaptation.
    found *= step(vec2<f32>(0.5 * max_delta), delta.xy);
    return vec4<f32>(found, 0.0, 0.0);
}

// ---- Blending weights ----

struct WeightsVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) pixcoord: vec2<f32>,
    @location(2) offset0: vec4<f32>,
    @location(3) offset1: vec4<f32>,
    @location(4) offset2: vec4<f32>,
}

@vertex
fn weights_vertex(@builtin(vertex_index) index: u32) -> WeightsVertex {
    let full = fullscreen_vertex(index, 0.0);
    let size = texel();
    var out: WeightsVertex;
    out.position = full.position;
    out.uv = full.uv;
    out.pixcoord = full.uv / size;
    // Offsets for the searches (@PSEUDO_GATHER4), and the ends of the loops.
    out.offset0 = full.uv.xyxy + size.xyxy * vec4<f32>(-0.25, -0.125, 1.25, -0.125);
    out.offset1 = full.uv.xyxy + size.xyxy * vec4<f32>(-0.125, -0.25, -0.125, 1.25);
    out.offset2 = vec4<f32>(out.offset0.xz, out.offset1.yw)
        + vec4<f32>(-2.0, 2.0, -2.0, 2.0) * vec4<f32>(size.xx, size.yy) * f32(SMAA_MAX_SEARCH_STEPS);
    return out;
}

fn search_length(e: vec2<f32>, bias: f32, scale: f32) -> f32 {
    var coord = e;
    coord.x = bias + coord.x * scale;
    return 255.0 * textureSampleLevel(search_texture, nearest, coord, 0.0).r;
}

fn area(dist: vec2<f32>, e1: f32, e2: f32, offset: f32) -> vec2<f32> {
    // Rounding prevents precision errors of bilinear filtering.
    var coord = SMAA_AREATEX_MAX_DISTANCE * round(4.0 * vec2<f32>(e1, e2)) + dist;
    // Scale and bias to texel space, then to the subpixel offset's subtexture.
    coord = SMAA_AREATEX_PIXEL_SIZE * coord + 0.5 * SMAA_AREATEX_PIXEL_SIZE;
    coord.y += SMAA_AREATEX_SUBTEX_SIZE * offset;
    return textureSampleLevel(lookup, bilinear, coord, 0.0).rg;
}

fn search_x_left(start: vec2<f32>, end: f32) -> f32 {
    let size = texel();
    var e = vec2<f32>(0.0, 1.0);
    var coord = start;
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        e = fetch(coord).rg;
        coord -= vec2<f32>(2.0, 0.0) * size;
        if coord.x <= end || e.g <= 0.8281 || e.r != 0.0 {
            break;
        }
    }
    // Undo the (-0.25, -0.125) offset, the search's bias of 1, and the last step's length.
    coord.x += 0.25 * size.x;
    coord.x += size.x;
    coord.x += 2.0 * size.x;
    coord.x -= size.x * search_length(e, 0.0, 0.5);
    return coord.x;
}

fn search_x_right(start: vec2<f32>, end: f32) -> f32 {
    let size = texel();
    var e = vec2<f32>(0.0, 1.0);
    var coord = start;
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        e = fetch(coord).rg;
        coord += vec2<f32>(2.0, 0.0) * size;
        if coord.x >= end || e.g <= 0.8281 || e.r != 0.0 {
            break;
        }
    }
    coord.x -= 0.25 * size.x;
    coord.x -= size.x;
    coord.x -= 2.0 * size.x;
    coord.x += size.x * search_length(e, 0.5, 0.5);
    return coord.x;
}

fn search_y_up(start: vec2<f32>, end: f32) -> f32 {
    let size = texel();
    var e = vec2<f32>(1.0, 0.0);
    var coord = start;
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        e = fetch(coord).rg;
        coord += vec2<f32>(0.0, -2.0) * size;
        if coord.y <= end || e.r <= 0.8281 || e.g != 0.0 {
            break;
        }
    }
    coord.y += 0.25 * size.y;
    coord.y += size.y;
    coord.y += 2.0 * size.y;
    coord.y -= size.y * search_length(e.gr, 0.0, 0.5);
    return coord.y;
}

fn search_y_down(start: vec2<f32>, end: f32) -> f32 {
    let size = texel();
    var e = vec2<f32>(1.0, 0.0);
    var coord = start;
    for (var i = 0; i < SMAA_MAX_SEARCH_STEPS; i++) {
        e = fetch(coord).rg;
        coord -= vec2<f32>(0.0, -2.0) * size;
        if coord.y >= end || e.r <= 0.8281 || e.g != 0.0 {
            break;
        }
    }
    coord.y -= 0.25 * size.y;
    coord.y -= size.y;
    coord.y -= 2.0 * size.y;
    coord.y += size.y * search_length(e.gr, 0.5, 0.5);
    return coord.y;
}

// `source` holds the edges here.
@fragment
fn weights(in: WeightsVertex) -> @location(0) vec4<f32> {
    let size = texel();
    var result = vec4<f32>(0.0);
    let subsample_indices = vec4<f32>(0.0);
    let e = fetch(in.uv).rg;
    // Edge at north.
    if e.g > 0.0 {
        var d: vec2<f32>;
        // The distance to the left, then the left crossing edges, two at a time (@CROSSING_OFFSET).
        let coords_left = vec2<f32>(search_x_left(in.offset0.xy, in.offset2.x), in.offset1.y);
        d.x = coords_left.x;
        let e1 = fetch(coords_left).r;
        // The distance to the right.
        let coords_right = vec2<f32>(search_x_right(in.offset0.zw, in.offset2.y), in.offset1.y);
        d.y = coords_right.x;
        // In pixels; the area texture is compressed quadratically.
        d = d / size.x - in.pixcoord.x;
        let sqrt_d = sqrt(abs(d));
        let e2 = fetch(coords_right + vec2<f32>(1.0, 0.0) * size).r;
        result = vec4<f32>(area(sqrt_d, e1, e2, subsample_indices.y), result.zw);
    }
    // Edge at west.
    if e.r > 0.0 {
        var d: vec2<f32>;
        let coords_up = vec2<f32>(in.offset0.x, search_y_up(in.offset1.xy, in.offset2.z));
        d.x = coords_up.y;
        let e1 = fetch(coords_up).g;
        let coords_down = vec2<f32>(in.offset0.x, search_y_down(in.offset1.zw, in.offset2.w));
        d.y = coords_down.y;
        d = d / size.y - in.pixcoord.y;
        let sqrt_d = sqrt(abs(d));
        let e2 = fetch(coords_down + vec2<f32>(0.0, 1.0) * size).g;
        result = vec4<f32>(result.xy, area(sqrt_d, e1, e2, subsample_indices.x));
    }
    return result;
}

// ---- Neighbourhood blending ----

struct BlendVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset1: vec4<f32>,
}

@vertex
fn blend_vertex(@builtin(vertex_index) index: u32) -> BlendVertex {
    let full = fullscreen_vertex(index, 0.0);
    var out: BlendVertex;
    out.position = full.position;
    out.uv = full.uv;
    out.offset1 = full.uv.xyxy + texel().xyxy * vec4<f32>(1.0, 0.0, 0.0, 1.0);
    return out;
}

// The frame in `source`, the weights in `lookup`. Written to the output, or to the frame the
// upscale reads; onto 8 or 10 bits (DITHER_LEVELS) dithered, and an sRGB target (DECODE_SRGB)
// gets the display values decoded after.

fn weight_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(lookup, bilinear, uv, 0.0);
}

@fragment
fn blend(in: BlendVertex) -> @location(0) vec4<f32> {
    var a: vec4<f32>;
    let here = weight_at(in.uv);
    a.x = here.x;
    a.z = here.z;
    a.y = weight_at(in.offset1.zw).g;
    a.w = weight_at(in.offset1.xy).a;
    var color: vec4<f32>;
    if dot(a, vec4<f32>(1.0)) < 1e-5 {
        color = fetch(in.uv);
    } else {
        // Up to four lines can cross a pixel; take the heaviest in each direction, then the
        // heavier direction.
        var offset: vec2<f32>;
        offset.x = select(-a.b, a.a, a.a > a.b);
        offset.y = select(-a.r, a.g, a.g > a.r);
        if abs(offset.x) > abs(offset.y) {
            offset.y = 0.0;
        } else {
            offset.x = 0.0;
        }
        let c = fetch(in.uv);
        let opposite = fetch(in.uv + sign(offset) * texel());
        let s = select(abs(offset.y), abs(offset.x), abs(offset.x) > abs(offset.y));
        color = mix(c, opposite, s);
    }
    var out = color.rgb;
#ifdef DITHER_LEVELS
    out = dither_encoded(out, in.position.xy, f32(#{DITHER_LEVELS}));
#endif
#ifdef DECODE_SRGB
    out = srgb_eotf(out);
#endif
    return vec4<f32>(out, 1.0);
}
