// The scenario editor's globe (native/docs/ui-contract/maps.md, "The editor's globe"; `pass.rs`, `world.rs`): the
// Earth seen from straight above the view's centre, orthographic and north up. The world chart's meshes are unit
// vectors made once; each vertex is projected here, so a camera change tessellates nothing.
//
// - `fs_base`: the globe's disc in the sea's colour, space round it.
// - `vs_fill` / `fs_fill`: a layer's triangles (the depth bands, the land), cut where they pass behind the limb.
// - `vs_line` / `fs_line`: a layer's segments (the graticule, the coasts, the reefs), a quad each in device pixels.
//
// In the game the region's own ground is drawn over these within the reach (naval-client's `chart_ground.wgsl`);
// nothing veils the world beyond.
//
// Colours are linear and premultiplied; the output is encoded here where the frame does not encode itself.

struct Camera {
    // The view's east, north and up, unit vectors in the Earth's frame (xyz).
    east: vec4<f32>,
    north: vec4<f32>,
    up: vec4<f32>,
    // The disc's centre on the output (device pixels), the Earth's radius in device pixels, 1 to encode sRGB here.
    disc: vec4<f32>,
    // The output's width and height, device pixels.
    size: vec4<f32>,
    space: vec4<f32>,
    sea: vec4<f32>,
};

struct Layer {
    colour: vec4<f32>,
    // Half the line's width, device pixels (x), and what of the colour a line thinner than a pixel keeps (y).
    line: vec4<f32>,
};

@group(0) @binding(0) var<uniform> cam: Camera;
@group(0) @binding(1) var<uniform> layer: Layer;

fn encode_srgb(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(max(linear, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

// A premultiplied linear colour at `cover`, encoded where the output does not encode itself.
fn shade(c: vec4<f32>, cover: f32) -> vec4<f32> {
    var rgb = c.rgb;
    if cam.disc.w > 0.5 && c.a > 0.0 {
        rgb = encode_srgb(c.rgb / c.a) * c.a;
    }
    return vec4<f32>(rgb, c.a) * cover;
}

// A unit vector on the output, device pixels.
fn place(v: vec3<f32>) -> vec2<f32> {
    return cam.disc.xy + vec2<f32>(dot(v, cam.east.xyz), -dot(v, cam.north.xyz)) * cam.disc.z;
}

fn clip_of(p: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(p.x / cam.size.x * 2.0 - 1.0, 1.0 - p.y / cam.size.y * 2.0, 0.0, 1.0);
}

// How much of a pixel at height `d` toward the eye (v·u) is on the near side: a pixel's ramp across the limb.
fn near_side(d: f32) -> f32 {
    return clamp(d / max(fwidth(d), 1e-7) + 0.5, 0.0, 1.0);
}

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// How much of the pixel at `at` is on the Earth's disc.
fn on_disc(at: vec2<f32>) -> f32 {
    return clamp(cam.disc.z - length(at - cam.disc.xy) + 0.5, 0.0, 1.0);
}

@fragment
fn fs_base(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let cover = on_disc(at.xy);
    return shade(cam.space, 1.0 - cover) + shade(cam.sea, cover);
}

struct FillOut {
    @builtin(position) position: vec4<f32>,
    @location(0) toward: f32,
};

@vertex
fn vs_fill(@location(0) v: vec3<f32>) -> FillOut {
    var out: FillOut;
    out.position = clip_of(place(v));
    out.toward = dot(v, cam.up.xyz);
    return out;
}

@fragment
fn fs_fill(in: FillOut) -> @location(0) vec4<f32> {
    let cover = near_side(in.toward);
    if cover <= 0.0 {
        discard;
    }
    return shade(layer.colour, cover);
}

struct LineOut {
    @builtin(position) position: vec4<f32>,
    // Device pixels across the line from its middle.
    @location(0) across: f32,
    @location(1) toward: f32,
};

@vertex
fn vs_line(@builtin(vertex_index) index: u32, @location(0) a: vec3<f32>, @location(1) b: vec3<f32>) -> LineOut {
    // Two triangles: (0,0) (1,0) (0,1) and (0,1) (1,0) (1,1), along the segment and across it.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[index];
    let pa = place(a);
    let pb = place(b);
    let d = pb - pa;
    let length_px = length(d);
    var along = vec2<f32>(1.0, 0.0);
    if length_px > 1e-6 {
        along = d / length_px;
    }
    let normal = vec2<f32>(-along.y, along.x);
    // Half its width and a pixel more for the edge's ramp; the ends reach as far, so the joins close.
    let reach = max(layer.line.x, 0.5) + 1.0;
    let side = corner.y * 2.0 - 1.0;
    let end = corner.x * 2.0 - 1.0;
    let p = mix(pa, pb, corner.x) + along * end * reach + normal * side * reach;
    var out: LineOut;
    out.position = clip_of(p);
    out.across = side * reach;
    out.toward = mix(dot(a, cam.up.xyz), dot(b, cam.up.xyz), corner.x);
    return out;
}

@fragment
fn fs_line(in: LineOut) -> @location(0) vec4<f32> {
    let half = max(layer.line.x, 0.5);
    let cover = clamp(half + 0.5 - abs(in.across), 0.0, 1.0) * near_side(in.toward) * layer.line.y;
    if cover <= 0.0 {
        discard;
    }
    return shade(layer.colour, cover);
}
