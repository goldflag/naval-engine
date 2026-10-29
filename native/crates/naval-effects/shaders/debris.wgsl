// Strike debris (src/debris.rs) and the flotsam a lost ship leaves (src/flotsam.rs): each piece built of up to four
// boxes in a shape of its own (`extents.w`, `flotsam::shape`): a strike's irregular shard (a box whose eight corners are
// pulled in and out by its seed), a torn plate folded at a crease, a girder's angle section kinked part way, a plank,
// a Carley float's ring of tube, a cork raft, a boat's open hull; turned by its tumble or its ride on the swell. A shape
// of fewer boxes leaves the rest with no area. Drawn in the opaque phase (writing depth, so the sea hides what has sunk
// and the smoke after sorts against it). Faceted: the normal is the face's own, from the screen derivatives. Lit by the
// effects' sun (through the renderer's sun shadow maps) and sky fill, a glint off bare steel or wet timber, the blast
// flashes, and its own glow while it is hot. `caster_vertex` draws the same pieces into the sun's shadow maps. Not a
// port: the browser game's strikes threw sparks only.

#import naval::camera::{camera, world_to_clip}
#import naval_effects::common::{sky_fill, effect_fog, lighting, emitted, surface_sunlight}
#import naval::math::rotate

struct DebrisPiece {
    // Centre (m) and seed.
    position: vec4<f32>,
    // Quaternion.
    rotation: vec4<f32>,
    // Half extents (m) and the shape (`flotsam::shape`).
    extents: vec4<f32>,
    // Albedo and glow (0 cold … 1 yellow-hot).
    color: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> pieces: array<DebrisPiece>;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    // Its shape (`flotsam::shape`).
    @location(2) @interpolate(flat) shape: u32,
}

// The vertices a piece is drawn with: four boxes of twelve triangles (`src/layer.rs` draws this many).
const PIECE_VERTICES: u32 = 144u;

const SHARD: u32 = 0u;
const PLATE: u32 = 1u;
const GIRDER: u32 = 2u;
const PLANK: u32 = 3u;
const CARLEY: u32 = 4u;
const RAFT: u32 = 5u;
const BOAT: u32 = 6u;

fn hash(n: f32) -> f32 {
    return fract(sin(n * 12.9898 + 4.1414) * 43758.5453);
}

// The box's six faces as corner indices (bit 0 x, bit 1 y, bit 2 z), two triangles each, wound outward.
const FACES = array<u32, 36>(
    0u, 2u, 3u, 0u, 3u, 1u, // −z
    4u, 5u, 7u, 4u, 7u, 6u, // +z
    0u, 4u, 6u, 0u, 6u, 2u, // −x
    1u, 3u, 7u, 1u, 7u, 5u, // +x
    0u, 1u, 5u, 0u, 5u, 4u, // −y
    2u, 6u, 7u, 2u, 7u, 3u  // +y
);

fn turn_y(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * v.x + s * v.z, v.y, -s * v.x + c * v.z);
}

fn turn_z(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * v.x - s * v.y, s * v.x + c * v.y, v.z);
}

// Between `low` and `high` by a corner's sign along that axis.
fn span(low: f32, high: f32, sign: f32) -> f32 {
    return mix(low, high, sign * 0.5 + 0.5);
}

// Corner `corner` (bit 0 x, bit 1 y, bit 2 z) of box `part` of a piece of `shape`, half extents `e` and seed `seed`,
// in the piece's frame (x its length, y up, z across); the origin for a box the shape has not (no area).
fn shape_corner(shape: u32, part: u32, corner: u32, e: vec3<f32>, seed: f32) -> vec3<f32> {
    let signs = vec3<f32>(f32(corner & 1u), f32((corner >> 1u) & 1u), f32((corner >> 2u) & 1u)) * 2.0 - 1.0;
    let jitter = seed * 97.0 + f32(corner) * 7.31 + f32(part) * 3.71;
    switch shape {
        case 1u: {
            // Torn plating folded at a crease across it: two boxes hinged there, its outer edges torn uneven.
            if part > 1u {
                return vec3<f32>(0.0);
            }
            let crease = e.x * (0.6 * hash(seed * 13.1) - 0.3);
            let fold = (0.35 + 0.9 * hash(seed * 5.3)) * select(-1.0, 1.0, hash(seed * 2.9) > 0.5);
            let low = select(-e.x, crease, part == 1u);
            let high = select(crease, e.x, part == 1u);
            let outer = (part == 0u && signs.x < 0.0) || (part == 1u && signs.x > 0.0);
            let torn = select(1.0, 0.7 + 0.6 * hash(jitter), outer);
            var p = vec3<f32>(crease + (span(low, high, signs.x) - crease) * torn, signs.y * e.y,
                signs.z * e.z * select(1.0, 0.6 + 0.5 * hash(jitter + 1.7), outer));
            if part == 1u {
                p = turn_z(p - vec3<f32>(crease, 0.0, 0.0), fold) + vec3<f32>(crease, 0.0, 0.0);
            }
            return p;
        }
        case 2u: {
            // An angle section, flange (in x z) and web (in x y) meeting along its length, `e.y` the legs and `e.z`
            // the steel's thickness: kinked at `kink` (the far half bent about the web), its ends torn.
            let kink = e.x * (0.8 * hash(seed * 11.3) - 0.4);
            let bend = (hash(seed * 4.7) - 0.5) * 1.4;
            let near = part < 2u;
            let low = select(kink, -e.x, near);
            let high = select(e.x, kink, near);
            let end = (near && signs.x < 0.0) || (!near && signs.x > 0.0);
            let x = kink + (span(low, high, signs.x) - kink) * select(1.0, 0.85 + 0.3 * hash(jitter), end);
            var p: vec3<f32>;
            if part % 2u == 0u {
                p = vec3<f32>(x, signs.y * e.z, span(0.0, 2.0 * e.y, signs.z));
            } else {
                p = vec3<f32>(x, span(0.0, 2.0 * e.y, signs.y), signs.z * e.z);
            }
            p -= vec3<f32>(0.0, 0.5 * e.y, 0.5 * e.y);
            if !near {
                p = turn_y(p - vec3<f32>(kink, 0.0, 0.0), bend) + vec3<f32>(kink, 0.0, 0.0);
            }
            return p;
        }
        case 3u, 5u: {
            if part > 0u {
                return vec3<f32>(0.0);
            }
            // A plank's ends splintered; a raft's cork square cut true.
            let ragged = select(1.0, 0.9 + 0.2 * hash(jitter), shape == PLANK);
            return vec3<f32>(signs.x * e.x * ragged, signs.y * e.y, signs.z * e.z);
        }
        case 4u: {
            // A ring of its tube (radius `e.y`), `e.x` by `e.z` over all: two long sides and two ends.
            let r = e.y;
            if part < 2u {
                let side = select(-1.0, 1.0, part == 1u);
                return vec3<f32>(signs.x * (e.x - 0.4 * r), signs.y * r, side * (e.z - r) + signs.z * r);
            }
            let end = select(-1.0, 1.0, part == 3u);
            return vec3<f32>(end * (e.x - r) + signs.x * r, signs.y * r, signs.z * max(e.z - 2.0 * r, 0.0));
        }
        case 6u: {
            if part > 0u {
                return vec3<f32>(0.0);
            }
            // An open boat's hull: fine at her stem, her sheer rising to it, narrowed to her keel; a full stern.
            var p = signs * e;
            if signs.x > 0.0 {
                p.z *= 0.12;
                p.y += 0.25 * e.y * (signs.y * 0.5 + 0.5);
            } else {
                p.z *= 0.8;
            }
            if signs.y < 0.0 {
                p.z *= 0.45;
            }
            return p;
        }
        default: {
            // A strike's shard: each corner pulled in or out by the seed, a torn, uneven chunk rather than a brick.
            if part > 0u {
                return vec3<f32>(0.0);
            }
            let pull = vec3<f32>(hash(jitter), hash(jitter + 1.7), hash(jitter + 3.1)) * 0.9 + 0.35;
            return signs * e * pull;
        }
    }
}

// Corner `vertex_index` of piece `instance` in the world.
fn piece_corner(vertex_index: u32, instance: u32) -> vec3<f32> {
    let piece = pieces[instance];
    let corner = FACES[vertex_index % 36u];
    let part = vertex_index / 36u;
    let shape = u32(piece.extents.w + 0.5);
    let local = shape_corner(shape, part, corner, piece.extents.xyz, piece.position.w);
    return piece.position.xyz + rotate(piece.rotation, local);
}

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    let piece = pieces[instance];
    let world = piece_corner(vertex_index, instance);
    var out: Varyings;
    out.position = world_to_clip(world);
    out.world = world;
    out.color = piece.color;
    out.shape = u32(piece.extents.w + 0.5);
    return out;
}

// The shards into the sun's shadow maps (`EffectsLayer::draw_shadow`): group 0 is the map's shadow camera.
@vertex
fn caster_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> @builtin(position) vec4<f32> {
    return world_to_clip(piece_corner(vertex_index, instance));
}

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    var normal = normalize(cross(dpdx(in.world), dpdy(in.world)));
    let to_camera = normalize(camera.position - in.world);
    if dot(normal, to_camera) < 0.0 {
        normal = -normal;
    }
    let sun = normalize(lighting.sun_direction.xyz);
    let albedo = in.color.rgb;
    // The sun through the renderer's shadow maps: a shard in a hull's shadow (or its smoke's) takes only the sky.
    let sunlit = surface_sunlight(in.world, normal);
    let diffuse = max(dot(normal, sun), 0.0) * 1.3 * sunlit;
    let sky = sky_fill() * (0.65 + 0.35 * normal.y);
    // A glint off torn steel as it tumbles through the sun's reflection; off what floats, the water on it (wet timber,
    // canvas and paint: a broader, weaker sheen).
    let half_vector = normalize(sun + to_camera);
    let afloat = in.shape >= PLANK;
    let glint = pow(max(dot(normal, half_vector), 0.0), select(40.0, 16.0, afloat)) * select(0.6, 0.25, afloat)
        * (1.0 - in.color.a) * sunlit;
    var color = albedo * (sky + lighting.direct.rgb * diffuse) + lighting.direct.rgb * glint;
    // The blast flashes close by.
    for (var i = 0u; i < 4u; i++) {
        let flash = lighting.flashes[i];
        let offset = flash.xyz - in.world;
        let reach = flash.w / (dot(offset, offset) + 4.0);
        color += albedo * lighting.flash_color.rgb * reach * (0.5 + 0.5 * max(dot(normal, normalize(offset)), 0.0)) * 40.0;
    }
    // Its own glow: yellow-white when fresh, dull red as it cools.
    let glow = in.color.a;
    let hot = mix(vec3<f32>(1.6, 0.22, 0.03), vec3<f32>(6.0, 3.0, 1.0), smoothstep(0.35, 1.0, glow));
    color += emitted(hot * glow * glow);
    return vec4<f32>(effect_fog(color, in.world), 1.0);
}
