// The trees' impostor atlas, baked once when the land is made: every species' grown tree as the near trees draw it at
// full detail (`naval_terrain::tree_meshes`), drawn orthographically from each of its `IMPOSTOR_FRAMES`² frames'
// directions (`naval_terrain::impostor`), one frame an instance, into a scratch target twice the atlas's side, each
// frame in its cell as the atlas lays them out. `tree_resolve.wgsl` then brings the scratch down into the atlas. One
// target takes the colour (linear albedo, before the tree's tint and the land's tone); the other the normal (model
// space, octahedral), the depth along the frame's direction about the model's middle (over its bounding radius) and the
// sky its crown leaves the point with the material (`naval_terrain::impostor::pack_material`: leaves' over the upper
// part of the channel, bark's and twigs' over the lower), so the draw lights them apart.
//
// A spray's leaves and a limb's bark take the near trees' surface (`naval_terrain::tree_surface`), so the impostor a
// tree crossfades into shades as its mesh did: a leaf's normal its clump's, its own tilt on it less than up close (a
// texel holds many leaves, and a far crown keeps its clumps' light and shade), and the occlusion the sky the crown
// leaves the point (`naval_terrain::crown`); the impostor takes its bark's sun from that sky alone.
#import naval_terrain::impostor::{IMPOSTOR_FRAMES, LEAF_LAYERS, view_of, frame_direction, encode_normal, pack_material}
#import naval_terrain::tree_surface::{TreeSurface, spray_surface, bark_surface}

// How far a leaf's own tilt on its card turns its clump's normal in the bake (`near_trees.wgsl` takes more).
const BAKE_TILT: f32 = 0.15;

struct BakeModel {
    // The species' index and its model's radius.
    index: u32,
    radius: f32,
    pad: vec2<u32>,
}

@group(0) @binding(0) var<uniform> model: BakeModel;
// The whole atlas: its last layers are the leaf sprites.
@group(0) @binding(1) var sprites: texture_2d_array<f32>;
@group(0) @binding(2) var sprite_sampler: sampler;

struct VertexIn {
    // `tree_meshes::MeshVertex`, as `near_trees.wgsl` reads it.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) wind: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    // The point across its frame (-1 to 1 each way) and its depth toward the eye (over the bound).
    @location(0) local: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    // What it is, its alpha cut (bark: its pattern), its sprite's layer.
    @location(5) @interpolate(flat) kind: vec3<f32>,
    // The frame's direction toward its eye.
    @location(6) @interpolate(flat) toward: vec3<f32>,
}

@vertex
fn vs_bake(in: VertexIn, @builtin(instance_index) frame: u32) -> VertexOut {
    let frames = u32(IMPOSTOR_FRAMES);
    let cell = vec2<f32>(f32(frame % frames), f32(frame / frames));
    let view = view_of(frame_direction(cell), model.radius);
    let q = in.position - vec3<f32>(0.0, 0.5, 0.0);
    let s = dot(q, view.right) / view.half.x;
    let t = dot(q, view.up) / view.half.y;
    let bound = length(vec2<f32>(model.radius, 0.5));
    let depth = clamp(dot(q, view.toward) / bound, -1.0, 1.0);
    // Its cell on the target (v down), the frame laid in it as `impostor::frame_uv` reads it back.
    let x = (cell.x + s * 0.5 + 0.5) / IMPOSTOR_FRAMES;
    let y = (cell.y + 0.5 - t * 0.5) / IMPOSTOR_FRAMES;
    var out: VertexOut;
    // Nearer the eye deeper: the target's depth keeps the greatest.
    out.clip = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.5 + 0.49 * depth, 1.0);
    out.local = vec3<f32>(s, t, depth);
    out.normal = in.normal.xyz;
    out.tangent = in.tangent.xyz;
    out.uv = in.uv;
    out.color = in.color;
    out.kind = vec3<f32>(in.wind.x, in.wind.y, round(in.tangent.w * 127.0 / 32.0));
    out.toward = view.toward;
    return out;
}

struct BakeOut {
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
}

@fragment
fn fs_bake(in: VertexOut, @builtin(front_facing) front: bool) -> BakeOut {
    // The sprite read before any pixel leaves (its derivatives need the whole quad).
    let gx = dpdx(in.uv);
    let gy = dpdy(in.uv);
    let layer = i32(textureNumLayers(sprites) - LEAF_LAYERS) + i32(in.kind.z);
    let leaf = textureSampleGrad(sprites, sprite_sampler, in.uv, layer, gx, gy);
    let card = in.kind.x > 0.25;
    // A shape reaching past its frame's rectangle is cut at it (the frames' margin keeps the model inside).
    if any(abs(in.local.xy) > vec2<f32>(1.0)) || (card && leaf.a < in.kind.y) {
        discard;
    }
    var normal = normalize(in.normal);
    var s: TreeSurface;
    if card {
        s = spray_surface(leaf, in.color.rgb, normal, in.tangent, in.kind.x > 0.75, BAKE_TILT, !front);
    } else {
        if !front {
            normal = -normal;
        }
        s = bark_surface(u32(round(in.kind.y * 255.0 / 32.0)), in.uv, in.color.rgb, normal, in.tangent);
    }
    let sky = in.color.a;
    // A bare crown's twigs are its mass, as a leafy crown's leaves are: packed as leaves (the draw knows the bare
    // species and lights them as twigs).
    let mass = s.leafy > 0.5 || (card && in.kind.x > 0.75);
    var out: BakeOut;
    out.color = vec4<f32>(s.albedo, 1.0);
    out.normal = vec4<f32>(
        encode_normal(s.normal),
        in.local.z * 0.5 + 0.5,
        pack_material(sky, mass),
    );
    return out;
}
