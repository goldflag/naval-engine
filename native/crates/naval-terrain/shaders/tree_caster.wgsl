// The trees as sun shadow casters: the placed trees (`tree_place.wgsl`'s instances, the trees' own indirect draw) drawn
// into the sun's near and wide maps, each as an impostor card facing the light, alpha-tested against the three frames
// of its species nearest the light's direction (`naval_terrain::impostor`), so a crown casts the outline the sun sees.
//
// The card stands across the light through the tree's middle over the rectangle the model covers seen from the sun, so
// a low sun throws the whole tree's outline from its foot and a high one its crown beneath it. Group 0 is the map's
// shadow camera (`naval::camera::camera`), whose inverse view's third column points toward the light. Ground cover too
// small to matter in the maps casts nothing.
#import naval::camera::{camera, world_to_clip}
#import naval_terrain::bindings::{clamp_sampler, tree_atlas, chart_offset}
#import naval_terrain::tree_wind::{tree_sway, bend, lean_by}
#import naval_terrain::impostor::{IMPOSTOR_FRAMES, FRAME_TEXELS, placed_by, form_of, view_of, frame_direction, frame_uv, in_cell, frame_edge, pick_frames, to_model, to_world}

// Trees lower than this (m) cast nothing.
const LEAST_CASTER_M: f32 = 1.2;

struct VertexIn {
    // The card's corner: x across -0.5 to 0.5, y up 0 to 1.
    @location(0) card: vec2<f32>,
    // The placed tree (`trees.wgsl`'s `VertexIn`).
    @location(1) foot: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) tint: vec4<f32>,
}

struct CasterOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv01: vec4<f32>,
    @location(1) uv2_weights: vec4<f32>,
    @location(2) @interpolate(flat) species: u32,
    @location(3) @interpolate(flat) cells01: vec4<f32>,
    @location(4) @interpolate(flat) cell2: vec2<f32>,
}

@vertex
fn vs_tree_caster(in: VertexIn) -> CasterOut {
    let tree_height = in.foot.w;
    let radius = in.shape.x;
    let form = form_of(in.shape.z);
    let placed = placed_by(in.foot.xyz + vec3<f32>(0.0, tree_height * 0.5, 0.0), tree_height, abs(in.tint.w), form);
    let light = normalize(camera.inverse_view[2].xyz);
    let toward = normalize(to_model(placed, light) + vec3<f32>(0.0, 1e-6, 0.0));
    let card = view_of(toward, radius);
    let q = card.right * (in.card.x * 2.0 * card.half.x) + card.up * ((in.card.y - 0.5) * 2.0 * card.half.y);
    let offset = chart_offset();
    // Leaning and bent by the wind as the tree's card and mesh are (`naval_terrain::tree_wind`).
    let up = clamp(q.y + 0.5, 0.0, 1.0);
    let sway = bend(tree_sway(in.foot.xz, tree_height), up, tree_height) + lean_by(form.lean, form.bow, up, tree_height);
    let world = placed.middle + to_world(placed, q) + vec3<f32>(offset.x, 0.0, offset.y) + sway;
    let picked = pick_frames(toward);
    var out: CasterOut;
    out.clip = world_to_clip(world);
    if tree_height < LEAST_CASTER_M {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    out.uv01 = vec4<f32>(
        frame_uv(q, picked.cells[0], view_of(frame_direction(picked.cells[0]), radius)),
        frame_uv(q, picked.cells[1], view_of(frame_direction(picked.cells[1]), radius)),
    );
    out.uv2_weights = vec4<f32>(frame_uv(q, picked.cells[2], view_of(frame_direction(picked.cells[2]), radius)), picked.weights.xy);
    out.species = u32(in.shape.y);
    out.cells01 = vec4<f32>(picked.cells[0], picked.cells[1]);
    out.cell2 = picked.cells[2];
    return out;
}

@fragment
fn fs_tree_caster(in: CasterOut) {
    let w = vec3<f32>(in.uv2_weights.zw, 1.0 - in.uv2_weights.z - in.uv2_weights.w);
    let gx = dpdx(in.uv01.xy);
    let gy = dpdy(in.uv01.xy);
    let inset = frame_edge(gx, gy);
    let uv0 = in_cell(in.uv01.xy, in.cells01.xy, inset);
    let uv1 = in_cell(in.uv01.zw, in.cells01.zw, inset);
    let uv2 = in_cell(in.uv2_weights.xy, in.cell2, inset);
    // A frame's texels a map texel, as the impostors count a pixel's (`trees.wgsl`): where a map texel spans two or more
    // (a tree in the wide map), the frame that weighs most alone, its frames too alike at that size to show the change
    // (two samples a texel saved); else the three blended. The card faces the light, so its texels lie square in the map:
    // read at their level directly (an explicit level may be read in a branch, and costs no anisotropic taps).
    let texel_px = max(length(gx), length(gy)) * IMPOSTOR_FRAMES * FRAME_TEXELS;
    let level = log2(max(texel_px, 1.0));
    var coverage: f32;
    if texel_px < 2.0 {
        coverage = textureSampleLevel(tree_atlas, clamp_sampler, uv0, in.species, level).a * w.x
            + textureSampleLevel(tree_atlas, clamp_sampler, uv1, in.species, level).a * w.y
            + textureSampleLevel(tree_atlas, clamp_sampler, uv2, in.species, level).a * w.z;
    } else if w.x >= w.y && w.x >= w.z {
        coverage = textureSampleLevel(tree_atlas, clamp_sampler, uv0, in.species, level).a;
    } else if w.y >= w.z {
        coverage = textureSampleLevel(tree_atlas, clamp_sampler, uv1, in.species, level).a;
    } else {
        coverage = textureSampleLevel(tree_atlas, clamp_sampler, uv2, in.species, level).a;
    }
    // The edge where the cards' own alpha test puts it.
    if coverage < 0.45 {
        discard;
    }
}
