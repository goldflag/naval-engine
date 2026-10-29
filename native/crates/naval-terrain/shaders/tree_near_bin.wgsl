// Bins the placed trees near the camera for their meshes (native/docs/terrain/trees.md, "Near trees"): one thread a
// placed
// tree (`tree_place.wgsl`'s instances, after the placement), each tree in view within its meshes' reach copied into
// its species' list for each level of detail whose share of its pixels is above 0 (`tree_wind::near_shares`), and
// counted into that list's indirect draw (`near_trees.wgsl`). A tree a list has no room for draws as its impostor
// whole: its width is negated in the placed trees (and in any list it was copied to), which the impostor and the
// meshes read.
struct NearView {
    // The view's frustum in chart metres (n · p + w ≥ 0 inside).
    planes: array<vec4<f32>, 6>,
    // The eye in chart metres; w: the species.
    eye: vec4<f32>,
    // The meshes' largest reach (m), their reach a metre of a tree's height, the full detail's share of the reach and
    // its crossfade band's share of that (`bindings.wgsl`'s `tree_near` and `tree_lod`).
    reach: vec4<f32>,
    // Trees a list, the trees' capacity.
    room: vec4<u32>,
}

// The most a tree's top leans over its height, a palm's, its crown's most stretch, and the form's bit that marks a
// palm (`impostor.wgsl`'s, which a test holds these to).
const LEAN_MOST: f32 = 0.16;
const PALM_LEAN_MOST: f32 = 0.64;
const STRETCH_MOST: f32 = 0.3;
const PALM_FORM: u32 = 16384u;

// A placed tree (`tree_place.wgsl`'s `Tree`).
struct Tree {
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
}

// `DrawIndexedIndirectArgs`: a species' mesh at a level of detail, the trees counted here.
struct NearArgs {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

@group(0) @binding(0) var<uniform> view: NearView;
// The placement's indirect draw (its instance count at 1).
@group(0) @binding(1) var<storage, read> placed: array<u32>;
@group(0) @binding(2) var<storage, read_write> trees: array<Tree>;
@group(0) @binding(3) var<storage, read_write> near: array<Tree>;
@group(0) @binding(4) var<storage, read_write> lists: array<NearArgs>;

// A slot in list `list`, or the room for none.
fn take(list: u32) -> u32 {
    let index = atomicAdd(&lists[list].instance_count, 1u);
    if index >= view.room.x {
        atomicSub(&lists[list].instance_count, 1u);
    }
    return index;
}

@compute @workgroup_size(64)
fn bin(@builtin(global_invocation_id) id: vec3<u32>) {
    let count = min(placed[1], view.room.y);
    let i = id.x;
    if i >= count {
        return;
    }
    let tree = trees[i];
    let height = tree.a.w;
    let species = u32(tree.b.y);
    let reach = min(view.reach.x, height * view.reach.y);
    if reach <= 0.0 || species >= u32(view.eye.w) {
        return;
    }
    let middle = tree.a.xyz + vec3<f32>(0.0, height * 0.5, 0.0);
    // Within a metre either side of where the draw's shares start (their arithmetic is the draw's, in world metres).
    let d = distance(view.eye.xyz, middle);
    if d >= reach + 1.0 {
        return;
    }
    // In view: the sphere about its middle that holds its crown (the model's radius across, stretched as far as a crown
    // may be, half the height up), its lean and the wind's bend. A tree culled here whose crown reached the view would
    // leave a hole: its impostor gives way all the same.
    let crown = tree.b.x * height * abs(tree.c.w) * (1.0 + STRETCH_MOST);
    let lean_most = select(LEAN_MOST, PALM_LEAN_MOST, (u32(tree.b.z) & PALM_FORM) != 0u);
    let bound = length(vec2<f32>(crown, height * 0.5)) * 1.05 + lean_most * height + 2.0;
    for (var k = 0u; k < 6u; k += 1u) {
        let plane = view.planes[k];
        if dot(plane.xyz, middle) + plane.w < -bound {
            return;
        }
    }
    let full_end = reach * view.reach.z;
    let full_start = full_end * (1.0 - view.reach.w);
    let full = d < full_end + 1.0;
    let reduced = d > full_start - 1.0;
    var slots = vec2<u32>(0u);
    var fits = true;
    if full {
        slots.x = take(species * 2u);
        fits = fits && slots.x < view.room.x;
    }
    if reduced {
        slots.y = take(species * 2u + 1u);
        fits = fits && slots.y < view.room.x;
    }
    var copy = tree;
    if !fits {
        copy.c.w = -abs(copy.c.w);
        trees[i].c.w = copy.c.w;
    }
    if full && slots.x < view.room.x {
        near[(species * 2u) * view.room.x + slots.x] = copy;
    }
    if reduced && slots.y < view.room.x {
        near[(species * 2u + 1u) * view.room.x + slots.y] = copy;
    }
}
