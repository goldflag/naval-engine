// Grass about the camera (native/docs/terrain/trees.md, "Grass"): each clump the placement stood (`grass_place.wgsl`)
// drawn
// as `BLADES` blades made from the vertex's index, each a strip of four pairs and a tip: its root scattered over the
// clump's patch, its height, lean and facing its own, tapering to a point and curving over as it rises, bowed by the
// wind (`terrain.wind`) in gusts running downwind and fluttering at its own phase. A thinned clump far off stands for
// the clumps around it, so its blades widen with its patch and the field stays as thick to the eye. Its colour is the
// ground's under it (the placement read it), a little greener, varied blade to blade, a dry tip paler; it is lit as the
// ground is (`naval_terrain::foliage::light_grass`). Opaque, so a GPU that sorts a tile's pixels before shading them
// shades only the blades in front.
#import naval::camera::{camera, world_to_clip}
#import naval_terrain::bindings::{terrain, chart_offset}
#import naval_terrain::foliage::{Blade, light_grass}

const TAU: f32 = 6.283185307179586;
// Blades a clump, and the vertices each takes (`grass::BLADES`, `grass::BLADE_VERTICES`): the strip's first pair's left
// twice, four pairs up the blade, its tip three times, so the strip runs on to the next blade through triangles with
// no area, each blade's first triangle at the same parity.
const BLADES: u32 = 16u;
const BLADE_VERTICES: u32 = 12u;
// A blade's width at its root (m).
const BLADE_WIDTH_M: f32 = 0.018;
// A clump's patch at full density (m; `grass::GRASS_CELL_M` × 0.75): a thinned clump's wider patch widens its blades.
const NEAR_PATCH_M: f32 = 0.105;
// The widest a thinned clump's blades grow, against their own width.
const WIDEST: f32 = 8.0;

struct VertexIn {
    @builtin(vertex_index) vertex: u32,
    // The clump (`grass_place.wgsl`'s `Clump`).
    @location(0) foot: vec4<f32>,
    @location(1) look: vec4<u32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // Up the blade (0 at the root, 1 at the tip).
    @location(3) up: f32,
    // The land's normal at the clump's foot.
    @location(4) @interpolate(flat) ground: vec3<f32>,
    // The sun past the land at the foot, and its open sky.
    @location(5) @interpolate(flat) sun_open: vec2<f32>,
}

fn blade_hash(seed: u32, blade: u32, salt: u32) -> f32 {
    var v = vec2<u32>(seed ^ (blade * 747796405u), salt * 2891336453u + 1013904223u);
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32(v.x >> 8u) / 16777216.0;
}

@vertex
fn vs_grass(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let blade = in.vertex / BLADE_VERTICES;
    let local = in.vertex % BLADE_VERTICES;
    // The corner: up the blade in left-right pairs, then its tip.
    var level = 4.0;
    var side = 0.0;
    if local <= 1u {
        level = 0.0;
        side = -1.0;
    } else if local <= 8u {
        level = f32((local - 1u) / 2u);
        side = select(-1.0, 1.0, local % 2u == 0u);
    }
    let t = level / 4.0;
    let packed = unpack4x8unorm(in.look.x);
    let state = unpack4x8unorm(in.look.y);
    let tall = bitcast<f32>(in.look.z);
    let seed = in.look.w;
    let growth = state.x;
    // Its root on the clump's patch, its facing and its build.
    let around = blade_hash(seed, blade, 1u) * TAU;
    let out_m = in.foot.w * sqrt(blade_hash(seed, blade, 2u));
    let facing_angle = blade_hash(seed, blade, 3u) * TAU;
    let facing = vec3<f32>(cos(facing_angle), 0.0, sin(facing_angle));
    let height = tall * mix(0.55, 1.2, blade_hash(seed, blade, 4u)) * growth;
    let widen = clamp(in.foot.w / NEAR_PATCH_M, 1.0, WIDEST);
    let width = BLADE_WIDTH_M * mix(0.7, 1.35, blade_hash(seed, blade, 5u)) * widen * mix(0.6, 1.0, growth);
    let lean = mix(0.08, 0.55, blade_hash(seed, blade, 6u));
    let offset = vec3<f32>(chart_offset().x, 0.0, chart_offset().y);
    let root = vec3<f32>(in.foot.x, in.foot.y - 0.02, in.foot.z) + offset + vec3<f32>(cos(around), 0.0, sin(around)) * out_m;
    // The wind: a bow downwind growing with the square of its speed, in gusts running downwind across the land, and
    // each blade's flutter.
    let speed = terrain.wind.z;
    let dir = vec3<f32>(terrain.wind.x, 0.0, terrain.wind.y);
    let clock = terrain.wind.w;
    let along_wind = dot(root.xz, terrain.wind.xy) - speed * clock * 1.2;
    let gust = 0.55 + 0.3 * sin(along_wind * 0.31) + 0.15 * sin(along_wind * 0.83 + 1.3);
    let flutter = sin(clock * (2.1 + blade_hash(seed, blade, 7u) * 1.4) * TAU + around * 3.0);
    let bow = min(0.012 * speed * speed, 0.7) * gust * smoothstep(0.3, 2.0, speed) + 0.06 * flutter * min(speed * 0.25, 1.0);
    // The blade's line: a curve from its root up and over toward its facing, its tip pushed downwind.
    let p1 = root + vec3<f32>(0.0, height * 0.55, 0.0);
    let push = dir * bow * height;
    let tip = root + vec3<f32>(0.0, height * (1.0 - 0.4 * lean - 0.3 * bow), 0.0) + facing * (height * lean * 0.75) + push;
    let a = mix(root, p1 + push * 0.4, t);
    let b = mix(p1 + push * 0.4, tip, t);
    let centre = mix(a, b, t);
    let tangent = normalize(b - a + vec3<f32>(0.0, 1e-4, 0.0));
    let across = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), facing));
    let half = 0.5 * width * pow(1.0 - t, 0.75);
    let world = centre + across * (side * half);
    // Rounded across the blade, its face turned toward the eye (the whole blade's way, so no triangle's normals part).
    let toward = select(-1.0, 1.0, dot(cross(across, tip - root), camera.position - (root + tip) * 0.5) >= 0.0);
    let face = normalize(cross(across, tangent)) * toward;
    out.normal = normalize(face + across * side * 0.4);
    out.world = world;
    out.clip = world_to_clip(world);
    if growth <= 0.0 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    // The ground's colour a little greener (the land's carries the soil between the blades too), varied blade to blade,
    // a dry tip paler and yellower now and then.
    let ground = packed.rgb * packed.rgb;
    let grey = dot(ground, vec3<f32>(0.2126, 0.7152, 0.0722));
    let dry = blade_hash(seed, blade, 8u);
    var color = max(mix(vec3<f32>(grey), ground, 1.35), vec3<f32>(0.0)) * mix(0.8, 1.2, blade_hash(seed, blade, 9u));
    color = mix(color, color * vec3<f32>(1.25, 1.12, 0.75), t * t * dry * 0.6);
    out.color = color;
    out.up = t;
    let slope = (state.zw - 0.5) * 4.0;
    out.ground = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));
    out.sun_open = vec2<f32>(state.y, packed.a);
    return out;
}

@fragment
fn fs_grass(in: VertexOut) -> @location(0) vec4<f32> {
    var b: Blade;
    b.albedo = in.color;
    b.world = in.world;
    b.normal = normalize(in.normal);
    b.ground = in.ground;
    // Darker down among the blades.
    b.occluded = mix(0.4, 1.0, pow(in.up, 0.7));
    b.open = in.sun_open.y;
    b.sun_at_foot = in.sun_open.x;
    return vec4<f32>(light_grass(b), 1.0);
}
