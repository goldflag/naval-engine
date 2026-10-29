// Trial (shadows): the casters of the smoke's opacity shadow map (`src/shadow.rs`; read back by `shadow.wgsl`).
//
// Every puff of smoke is a ball as the light sees it: a disc on the map, its optical depth greatest through the middle
// and spread evenly along the light through the ball's chord there. Two passes:
// - `front`: into the small front map (max blending), each ball's sun-side plane over a disc a texel wider than the
//   ball, so the map read with its linear filter is never behind any smoke: where the first smoke begins.
// - `slices`: into eight slices of the volume at a time (additive), how much of the ball's optical depth lies before
//   each slice's depth behind that front.
//
// Not a port: the browser game had no smoke shadows.

struct SmokeShadow {
    axis_u: vec4<f32>,
    axis_v: vec4<f32>,
    axis_d: vec4<f32>,
    // x the depth the front map counts back from, y and z the slices' spacing, w 1 while the map holds smoke.
    params: vec4<f32>,
    // x the map's width (m), y how far the front map's discs reach past a puff (m), z one over the slices' width in
    // texels.
    casting: vec4<f32>,
}

struct Blob {
    // Across and up the map (m from its corner), depth along the light (m from the map's depth origin), radius (m).
    place: vec4<f32>,
    // Optical depth through the middle.
    optical: vec4<f32>,
}

@group(0) @binding(0) var<uniform> smoke_shadow: SmokeShadow;
@group(0) @binding(1) var<storage, read> blobs: array<Blob>;
@group(1) @binding(0) var front_map: texture_2d<f32>;
@group(1) @binding(1) var front_sampler: sampler;
// x the first of this pass's eight slices.
@group(1) @binding(2) var<uniform> first_slice: vec4<f32>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    // Across the disc in radii of the ball.
    @location(0) disc: vec2<f32>,
    // Depth of the centre (m), radius (m), optical depth through the middle.
    @location(1) @interpolate(flat) ball: vec4<f32>,
}

fn place(vertex_index: u32, instance: u32, reach: f32) -> Varyings {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
    let blob = blobs[instance];
    let radius = max(blob.place.w, 0.01);
    let corner = corners[vertex_index] * (radius + reach);
    let uv = (blob.place.xy + corner) / smoke_shadow.casting.x;
    var out: Varyings;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.disc = corner / radius;
    out.ball = vec4<f32>(blob.place.z, radius, blob.optical.x, 0.0);
    return out;
}

@vertex
fn front_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    return place(vertex_index, instance, smoke_shadow.casting.y);
}

@vertex
fn slices_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    return place(vertex_index, instance, 0.0);
}

@fragment
fn front(in: Varyings) -> @location(0) vec4<f32> {
    return vec4<f32>(smoke_shadow.params.x - (in.ball.x - in.ball.y), 0.0, 0.0, 0.0);
}

struct Slices {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
    @location(3) d: vec4<f32>,
    @location(4) e: vec4<f32>,
    @location(5) f: vec4<f32>,
    @location(6) g: vec4<f32>,
    @location(7) h: vec4<f32>,
}

// Share of a chord beginning `start` metres behind the front and `run` metres long that lies before slice `index`.
fn before(index: f32, start: f32, run: f32) -> f32 {
    let i = first_slice.x + index;
    let depth = smoke_shadow.params.y * i + smoke_shadow.params.z * i * i;
    return clamp((depth - start) / run, 0.0, 1.0);
}

@fragment
fn slices(in: Varyings) -> Slices {
    let inside = 1.0 - dot(in.disc, in.disc);
    if inside <= 0.0 {
        discard;
    }
    // Thickest through the middle, thinning to nothing at the rim.
    let optical = in.ball.z * inside;
    let half = in.ball.y * sqrt(inside);
    let uv = in.position.xy * smoke_shadow.casting.z;
    let front = smoke_shadow.params.x - textureSampleLevel(front_map, front_sampler, uv, 0.0).r;
    let start = in.ball.x - half - front;
    let run = max(half * 2.0, 0.5);
    var out: Slices;
    out.a = vec4<f32>(optical * before(0.0, start, run), 0.0, 0.0, 0.0);
    out.b = vec4<f32>(optical * before(1.0, start, run), 0.0, 0.0, 0.0);
    out.c = vec4<f32>(optical * before(2.0, start, run), 0.0, 0.0, 0.0);
    out.d = vec4<f32>(optical * before(3.0, start, run), 0.0, 0.0, 0.0);
    out.e = vec4<f32>(optical * before(4.0, start, run), 0.0, 0.0, 0.0);
    out.f = vec4<f32>(optical * before(5.0, start, run), 0.0, 0.0, 0.0);
    out.g = vec4<f32>(optical * before(6.0, start, run), 0.0, 0.0, 0.0);
    out.h = vec4<f32>(optical * before(7.0, start, run), 0.0, 0.0, 0.0);
    return out;
}

struct Occluders {
    // How far before the front map's depth the ball begins, and how far past the depth origin it ends (max blending).
    @location(0) span: vec4<f32>,
    // Its optical depth along the light here (added).
    @location(1) optical: vec4<f32>,
}

// The same balls into the renderer's soft occluders' map (`naval_render::SunOccluders`), which the ships, the sea and
// every other receiver of the sun's shadows read: where the smoke along a texel begins and ends, and all it stops.
@fragment
fn occluders(in: Varyings) -> Occluders {
    let inside = 1.0 - dot(in.disc, in.disc);
    if inside <= 0.0 {
        discard;
    }
    let half = in.ball.y * sqrt(inside);
    var out: Occluders;
    out.span = vec4<f32>(smoke_shadow.params.x - (in.ball.x - half), in.ball.x + half, 0.0, 0.0);
    out.optical = vec4<f32>(in.ball.z * inside, 0.0, 0.0, 0.0);
    return out;
}
