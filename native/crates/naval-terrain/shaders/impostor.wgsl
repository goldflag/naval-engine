// Octahedral impostors of the trees' models (Ryan Brucks' octahedral impostors, hemi-octahedral): every species'
// model (`naval_terrain::species`) ray-cast once from `IMPOSTOR_FRAMES`² directions over the upper hemisphere into a
// frame each of the atlas (`tree_bake.wgsl`), its colour and coverage in one layer and its normal, depth and ambient
// occlusion in another; a tree drawn as one card facing the eye blends the three frames nearest its view.
//
// Model space: the model's foot at the origin, its top at y = 1, its crown within `radius` of the y axis; positions are
// taken about its middle (0, 0.5, 0). A frame looks at the model along its direction (toward the eye) orthographically,
// over the rectangle a cylinder of that radius from 0 to 1 covers seen from there: `radius` across, and
// (cos e + 2 radius sin e) / 2 up at elevation e. A card is the same rectangle for the eye's own direction, so it covers
// the tree from any view with no more than the crown's room, and each frame's texture coordinates on it are the card
// point projected along that frame's direction: affine across the card, so interpolated exactly.
#define_import_path naval_terrain::impostor

// Frames a side of each species' square of frames (hemi-octahedral: the corner frames on the horizon); the frames'
// texels a side. The Rust side's `IMPOSTOR_FRAMES` and `FRAME_TEXELS` match.
const IMPOSTOR_FRAMES: f32 = 8.0;
const FRAME_TEXELS: f32 = 96.0;
// Layers after the species' colour and normal layers: the near trees' leaf sprites (`naval_terrain::tree_meshes`).
const LEAF_LAYERS: u32 = 3u;
// The frame's rectangle grown, so the model never touches a frame's border texels (bilinear filtering would take the
// next frame's).
const FRAME_MARGIN: f32 = 1.035;

// A direction over the upper hemisphere (y up) to 0-1 on the square of frames.
fn hemi_octahedral(d: vec3<f32>) -> vec2<f32> {
    let s = abs(d.x) + abs(d.y) + abs(d.z);
    let p = vec2<f32>(d.x, d.z) / max(s, 1e-6);
    return vec2<f32>(p.x + p.y, p.x - p.y) * 0.5 + 0.5;
}

// 0-1 on the square of frames back to the direction.
fn from_hemi_octahedral(uv: vec2<f32>) -> vec3<f32> {
    let o = uv * 2.0 - 1.0;
    let p = vec2<f32>(o.x + o.y, o.x - o.y) * 0.5;
    let y = max(1.0 - abs(p.x) - abs(p.y), 0.0);
    return normalize(vec3<f32>(p.x, y, p.y));
}

// The direction of frame `cell` (0 to IMPOSTOR_FRAMES - 1 a side).
fn frame_direction(cell: vec2<f32>) -> vec3<f32> {
    return from_hemi_octahedral(cell / (IMPOSTOR_FRAMES - 1.0));
}

// A view's frame of reference in model space: across (level), up the picture, toward the eye, and the half-width and
// half-height of the rectangle the model's cylinder covers seen from there.
struct View {
    right: vec3<f32>,
    up: vec3<f32>,
    toward: vec3<f32>,
    half: vec2<f32>,
}

fn view_of(toward: vec3<f32>, radius: f32) -> View {
    var v: View;
    v.toward = toward;
    let across = cross(vec3<f32>(0.0, 1.0, 0.0), toward);
    let level = length(across);
    v.right = select(vec3<f32>(1.0, 0.0, 0.0), across / max(level, 1e-6), level > 1e-4);
    v.up = cross(toward, v.right);
    let sine = clamp(abs(toward.y), 0.0, 1.0);
    let cosine = sqrt(max(1.0 - sine * sine, 0.0));
    v.half = vec2<f32>(radius, 0.5 * cosine + radius * sine) * FRAME_MARGIN;
    return v;
}

// Where model point `q` (about the middle) falls in frame `cell` of the species' square, 0-1 over the whole square
// (v down the texture, as the bake writes it).
fn frame_uv(q: vec3<f32>, cell: vec2<f32>, frame: View) -> vec2<f32> {
    let s = dot(q, frame.right) / frame.half.x;
    let t = dot(q, frame.up) / frame.half.y;
    let local = vec2<f32>(s * 0.5 + 0.5, 0.5 - t * 0.5);
    return (cell + local) / IMPOSTOR_FRAMES;
}

// Texture coordinates `uv` on the species' square held inside frame `cell` (its border texels are empty): a frame's
// projection of a card point can fall past its own frame, into the next one's tree. `edge` is half a texel of the mip
// level sampled, in a frame's width (`frame_edge`), so the filter never reaches into the next frame.
fn in_cell(uv: vec2<f32>, cell: vec2<f32>, edge: f32) -> vec2<f32> {
    let local = clamp(uv * IMPOSTOR_FRAMES - cell, vec2<f32>(edge), vec2<f32>(1.0 - edge));
    return (cell + local) / IMPOSTOR_FRAMES;
}

// Half a texel of the mip level a sample with these screen derivatives (of coordinates on the species' square) reads,
// in a frame's width, up to a base texel: the model keeps `FRAME_MARGIN`'s 1.1 base texels off a frame's border, so a
// clamp that far never smears its edge outward. Coarser levels (a crown a few pixels across) may take a little of the
// next frame, the same tree seen 26 degrees round.
fn frame_edge(dx: vec2<f32>, dy: vec2<f32>) -> f32 {
    let texels = max(length(dx), length(dy)) * IMPOSTOR_FRAMES * FRAME_TEXELS;
    let level = exp2(ceil(log2(max(texels, 1.0))));
    return min(0.5 * level, 1.0) / FRAME_TEXELS;
}

// The three frames nearest a view (model-space direction toward the eye, at or above the horizon) and their weights.
struct Picked {
    cells: array<vec2<f32>, 3>,
    weights: vec3<f32>,
}

fn pick_frames(toward: vec3<f32>) -> Picked {
    let up = normalize(vec3<f32>(toward.x, max(toward.y, 0.0), toward.z) + vec3<f32>(0.0, 1e-5, 0.0));
    let g = hemi_octahedral(up) * (IMPOSTOR_FRAMES - 1.0);
    let cell = clamp(floor(g), vec2<f32>(0.0), vec2<f32>(IMPOSTOR_FRAMES - 2.0));
    let f = clamp(g - cell, vec2<f32>(0.0), vec2<f32>(1.0));
    var p: Picked;
    if f.x + f.y <= 1.0 {
        p.cells = array<vec2<f32>, 3>(cell, cell + vec2<f32>(1.0, 0.0), cell + vec2<f32>(0.0, 1.0));
        p.weights = vec3<f32>(1.0 - f.x - f.y, f.x, f.y);
    } else {
        p.cells = array<vec2<f32>, 3>(cell + vec2<f32>(1.0, 1.0), cell + vec2<f32>(1.0, 0.0), cell + vec2<f32>(0.0, 1.0));
        p.weights = vec3<f32>(f.x + f.y - 1.0, 1.0 - f.y, 1.0 - f.x);
    }
    return p;
}

// A unit normal to 0-1 (octahedral) and back.
fn encode_normal(n: vec3<f32>) -> vec2<f32> {
    let p = n.xz / (abs(n.x) + abs(n.y) + abs(n.z));
    var o = p;
    if n.y < 0.0 {
        o = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    }
    return o * 0.5 + 0.5;
}

fn decode_normal(e: vec2<f32>) -> vec3<f32> {
    let o = e * 2.0 - 1.0;
    var n = vec3<f32>(o.x, 1.0 - abs(o.x) - abs(o.y), o.y);
    let t = max(-n.y, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.z += select(t, -t, n.z >= 0.0);
    return normalize(n);
}

// A placed tree's turn and form, packed in one float of its instance (`tree_place.wgsl`'s `append`): the yaw the
// fraction of a turn, the whole part a code of its form: how far its top leans east and south (5 bits each, 15
// upright), how much wider its crown is along its own x than along its z (4 bits) and whether it is a palm's (bit 14:
// its lean in steps of `PALM_LEAN_MOST`, straight from its foot).
struct Form {
    // The yaw (radians).
    yaw: f32,
    // Its top's offset over its height (chart x, z).
    lean: vec2<f32>,
    // How far its lean bows toward its top (`tree_wind::lean_by`): 1 a tree's, 0 a palm's, which leans straight from
    // its foot as its card does.
    bow: f32,
    // Its crown's width along its own x and z, over its width.
    widths: vec2<f32>,
}

// The most a tree's top leans, over its height (about 9 degrees), a palm's (about 33), and its crown's most stretch
// along its x; the code's bit that marks a palm.
const LEAN_MOST: f32 = 0.16;
const PALM_LEAN_MOST: f32 = 0.64;
const STRETCH_MOST: f32 = 0.3;
const PALM_FORM: u32 = 16384u;

fn form_of(packed: f32) -> Form {
    var f: Form;
    f.yaw = fract(packed) * 6.283185307179586;
    let code = u32(packed);
    let palm = (code & PALM_FORM) != 0u;
    let most = select(LEAN_MOST, PALM_LEAN_MOST, palm);
    f.lean = (vec2<f32>(f32(code & 31u), f32((code >> 5u) & 31u)) - 15.0) / 15.0 * most;
    f.bow = select(1.0, 0.0, palm);
    let stretch = f32((code >> 10u) & 15u) / 15.0 * STRETCH_MOST;
    f.widths = vec2<f32>(1.0 + stretch, 1.0 - 0.5 * stretch);
    return f;
}

// A placed tree's model-to-world map: the model scaled by the tree's height (up) and `across` (m per model unit
// level, along its own x and z: a crown a little wider one way), turned by its yaw about its axis, its middle at
// `middle`. (Its lean is an offset of its own, `tree_wind::lean_by`, as the wind's bend is.)
struct Placed {
    middle: vec3<f32>,
    height: f32,
    across: vec2<f32>,
    // cos and sin of the yaw.
    turn: vec2<f32>,
}

// The tree at `middle`, `height` tall, `width` (m a model unit level, over the height) wide, of form `form`.
fn placed_by(middle: vec3<f32>, height: f32, width: f32, form: Form) -> Placed {
    var p: Placed;
    p.middle = middle;
    p.height = height;
    p.across = height * width * form.widths;
    p.turn = vec2<f32>(cos(form.yaw), sin(form.yaw));
    return p;
}

fn turn_by(v: vec3<f32>, turn: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(v.x * turn.x + v.z * turn.y, v.y, -v.x * turn.y + v.z * turn.x);
}

// A world offset (from the middle) into model space.
fn to_model(t: Placed, v: vec3<f32>) -> vec3<f32> {
    let u = turn_by(v, vec2<f32>(t.turn.x, -t.turn.y));
    return vec3<f32>(u.x / t.across.x, u.y / t.height, u.z / t.across.y);
}

// A model offset (from the middle) into the world.
fn to_world(t: Placed, q: vec3<f32>) -> vec3<f32> {
    return turn_by(vec3<f32>(q.x * t.across.x, q.y * t.height, q.z * t.across.y), t.turn);
}

// A model normal into the world.
fn normal_to_world(t: Placed, n: vec3<f32>) -> vec3<f32> {
    return normalize(turn_by(vec3<f32>(n.x / t.across.x, n.y / t.height, n.z / t.across.y), t.turn));
}

// The material and the sky its crown leaves the point (0 none, 1 open: `naval_terrain::crown`), packed in the normal
// layers' alpha: bark's and twigs' sky over [BARK_TOP, 0] (the more open, the lower), leaves' over [LEAF_FOOT, 1], a
// gap between, so a texel of one never reads as the other (when the two met at a half, leaves under a sixth of the sky
// read as half bark under half the sky, and a crown's darkest leaves drew lit), and the two meet at their darkest, so
// a texel that blends them (a trunk's edge against the leaves, frames blended up close) reads dark, not as bark under
// the open sky.
const BARK_TOP: f32 = 0.45;
const LEAF_FOOT: f32 = 0.55;

fn pack_material(sky: f32, leaves: bool) -> f32 {
    return select(BARK_TOP * (1.0 - sky), LEAF_FOOT + (1.0 - LEAF_FOOT) * sky, leaves);
}

// The packed material read back: x how much is leaves (0 bark), y the sky.
fn material_of(packed: f32) -> vec2<f32> {
    let leaves = clamp((packed - BARK_TOP) / (LEAF_FOOT - BARK_TOP), 0.0, 1.0);
    let sky = mix(1.0 - packed / BARK_TOP, (packed - LEAF_FOOT) / (1.0 - LEAF_FOOT), leaves);
    return vec2<f32>(leaves, clamp(sky, 0.0, 1.0));
}
