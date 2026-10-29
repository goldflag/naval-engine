// Trees standing on the land, drawn as octahedral impostors (`naval_terrain::impostor`): one card instanced for every
// tree the placement pass (`tree_place.wgsl`) left standing this frame, through an indirect draw, facing the eye and
// blending the three frames of the tree's species nearest its view, each with its colour, coverage, normal, depth and
// ambient occlusion (`tree_bake.wgsl`). The tree is lit per pixel from the normal at the depth the frames give.
//
// Ports the web game's `terrain/TerrainTrees.ts` (`TreeMaterial`): the tree stands on the exact ground height (sunk
// on slopes), lit as a crown by the sun (shaded by the land's own shadow at its foot) and the sky (shaded by the open
// sky there). Native since: the placement and its level of detail (a pass of their own), the crown's foliage light
// (the sun wrapped round the crown and shining through its leaves, the sky taken from above the crown: a crown with
// the sun behind it read black from a few hundred metres), each tree's own shade, the other trees' shadows (cast into
// the sun's maps by `tree_caster.wgsl`, and beyond them `naval_terrain::forest`'s stand-in), and since 2026-10-04 the
// impostors in place of flat cards. The edge is an alpha test at the frames' half coverage (no MSAA in the scene
// pass; temporal anti-aliasing smooths it). The light is `naval_terrain::foliage`'s, which the near trees' meshes
// (`near_trees.wgsl`) share; within their reach a tree's pixels go to its mesh by a dither (`naval_terrain::tree_wind`),
// and a tree wholly its mesh's draws no card. The wind bends the card as it bends the mesh.
#import naval::camera::{camera, world_to_clip}
#import naval_terrain::bindings::{sunlight, clamp_sampler, tree_atlas, field_uv, chart_offset}
#import naval_terrain::surface::{ramp, tone_down}
#import naval_terrain::foliage::{Foliage, light_foliage, tree_tint}
#import naval_terrain::tree_wind::{tree_sway, bend, lean_by, near_shares, pixel_dither}
#import naval_terrain::tree_surface::srgb_decode
#import naval_terrain::impostor::{IMPOSTOR_FRAMES, FRAME_TEXELS, LEAF_LAYERS, Placed, placed_by, form_of, view_of, frame_direction, frame_uv, in_cell, frame_edge, pick_frames, to_model, to_world, normal_to_world, decode_normal, material_of}

// The frames' coverage at the crown's edge.
const EDGE: f32 = 0.45;
// The bare species (`naval_terrain::species::Species::constant`): their crowns' texels are twigs, not leaves.
const SPECIES_BARE_OAK: u32 = 7u;
const SPECIES_BARE_BIRCH: u32 = 8u;

struct VertexIn {
    // The card's corner: x across -0.5 to 0.5, y up 0 to 1.
    @location(0) card: vec2<f32>,
    // The placed tree (`tree_place.wgsl`'s `Tree`): foot (chart x, the model's base height, chart z) and height (m);
    // the model's radius, species, its turn and form (`impostor::form_of`) and open sky; tint and width (m a model unit
    // level, over the height).
    @location(1) foot: vec4<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) tint: vec4<f32>,
}

struct VertexOut {
    // The same in the depth prepass and the light (`fs_tree_depth`, `fs_tree`), so the light's `Equal` test meets the
    // depth the prepass wrote.
    @builtin(position) @invariant clip: vec4<f32>,
    // The three frames' texture coordinates on the species' square; the first two weights.
    @location(0) uv01: vec4<f32>,
    @location(1) uv2_weights: vec4<f32>,
    // The card point in model space, about the model's middle.
    @location(2) q: vec3<f32>,
    // The tree's middle (world) and height.
    @location(3) middle: vec4<f32>,
    // cos and sin of the yaw, the widths (m a model unit along its own x and z).
    @location(4) turn: vec4<f32>,
    @location(5) tint: vec3<f32>,
    // The open sky at the foot, the model's bounding radius about its middle, the sun past the land at the foot.
    @location(6) open_bound: vec3<f32>,
    @location(7) @interpolate(flat) species: u32,
    // The three frames' cells on the species' square (their directions are found again where needed: fewer
    // varyings for a tiled GPU to store).
    @location(8) @interpolate(flat) cells01: vec4<f32>,
    @location(9) @interpolate(flat) cell2: vec2<f32>,
    // The share of the tree's pixels its mesh draws (`tree_wind::near_shares`).
    @location(10) @interpolate(flat) near: f32,
    // Its lean and how far it bows (`impostor::Form`'s `lean`, `bow`).
    @location(11) @interpolate(flat) lean: vec3<f32>,
}

@vertex
fn vs_tree(in: VertexIn) -> VertexOut {
    let tree_height = in.foot.w;
    let radius = in.shape.x;
    let offset = chart_offset();
    let middle = in.foot.xyz + vec3<f32>(0.0, tree_height * 0.5, 0.0);
    // (A tree the near meshes had no room for carries its width negated: `tree_near_bin.wgsl`.)
    let form = form_of(in.shape.z);
    let placed = placed_by(middle, tree_height, abs(in.tint.w), form);
    // The eye's direction in model space, and the card facing it over the rectangle the model covers from there.
    let eye = camera.position - vec3<f32>(offset.x, 0.0, offset.y);
    let toward = normalize(to_model(placed, eye - middle) + vec3<f32>(0.0, 1e-6, 0.0));
    let card = view_of(toward, radius);
    let q = card.right * (in.card.x * 2.0 * card.half.x) + card.up * ((in.card.y - 0.5) * 2.0 * card.half.y);
    let up = clamp(q.y + 0.5, 0.0, 1.0);
    let sway = bend(tree_sway(in.foot.xz, tree_height), up, tree_height) + lean_by(form.lean, form.bow, up, tree_height);
    let world = middle + to_world(placed, q) + vec3<f32>(offset.x, 0.0, offset.y) + sway;
    let middle_world = middle + vec3<f32>(offset.x, 0.0, offset.y);
    let near = select(near_shares(middle_world, tree_height).x, 0.0, in.tint.w < 0.0);

    let picked = pick_frames(toward);
    let d0 = frame_direction(picked.cells[0]);
    let d1 = frame_direction(picked.cells[1]);
    let d2 = frame_direction(picked.cells[2]);
    var out: VertexOut;
    out.clip = world_to_clip(world);
    // A tree its mesh draws whole draws no card.
    if near > 0.999 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
    }
    out.near = near;
    out.uv01 = vec4<f32>(frame_uv(q, picked.cells[0], view_of(d0, radius)), frame_uv(q, picked.cells[1], view_of(d1, radius)));
    out.uv2_weights = vec4<f32>(frame_uv(q, picked.cells[2], view_of(d2, radius)), picked.weights.xy);
    out.q = q;
    out.middle = vec4<f32>(middle_world, tree_height);
    out.turn = vec4<f32>(placed.turn, placed.across);
    out.tint = in.tint.rgb;
    out.open_bound = vec3<f32>(in.shape.w, length(vec2<f32>(radius, 0.5)),
        textureSampleLevel(sunlight, clamp_sampler, field_uv(in.foot.xz), 0.0).r);
    out.lean = vec3<f32>(form.lean, form.bow);
    out.species = u32(in.shape.y);
    out.cells01 = vec4<f32>(picked.cells[0], picked.cells[1]);
    out.cell2 = picked.cells[2];
    return out;
}

fn leaf_hash(p: vec3<f32>) -> f32 {
    var v = bitcast<vec3<u32>>(vec3<i32>(floor(p))) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    return f32((v.x + v.y * v.z) >> 8u) / 16777216.0;
}

// Value noise, 0-1.
fn leaf_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let x00 = mix(leaf_hash(i), leaf_hash(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let x10 = mix(leaf_hash(i + vec3<f32>(0.0, 1.0, 0.0)), leaf_hash(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let x01 = mix(leaf_hash(i + vec3<f32>(0.0, 0.0, 1.0)), leaf_hash(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let x11 = mix(leaf_hash(i + vec3<f32>(0.0, 1.0, 1.0)), leaf_hash(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

// The surface point frame `d` sees along its own direction through card point `q`, from its depth there.
fn frame_point(q: vec3<f32>, d: vec3<f32>, depth: f32, bound: f32) -> vec3<f32> {
    return q - d * dot(q, d) + d * ((depth * 2.0 - 1.0) * bound);
}

// The frames under a pixel: their places on the atlas, their weights, and their colours and coverage (one frame alone,
// the one that weighs most, where a frame's texel spans under half a pixel).
struct Frames {
    gx: vec2<f32>,
    gy: vec2<f32>,
    uv0: vec2<f32>,
    uv1: vec2<f32>,
    uv2: vec2<f32>,
    w: vec3<f32>,
    texel_px: f32,
    c0: vec4<f32>,
    c1: vec4<f32>,
    c2: vec4<f32>,
    k: vec3<f32>,
    coverage: f32,
}

fn frames_of(in: VertexOut) -> Frames {
    var f: Frames;
    let colour_layer = i32(in.species);
    let gx = dpdx(in.uv01.xy);
    let gy = dpdy(in.uv01.xy);
    f.gx = gx;
    f.gy = gy;
    let inset = frame_edge(gx, gy);
    f.uv0 = in_cell(in.uv01.xy, in.cells01.xy, inset);
    f.uv1 = in_cell(in.uv01.zw, in.cells01.zw, inset);
    f.uv2 = in_cell(in.uv2_weights.xy, in.cell2, inset);
    let w = vec3<f32>(in.uv2_weights.zw, 1.0 - in.uv2_weights.z - in.uv2_weights.w);
    f.w = w;
    // A frame's texels a pixel: under 2, the three frames blended; a crown so small that a frame's texel spans less than
    // half a pixel (under about 30 pixels tall) takes the frame that weighs most alone, two samples a pixel saved, its
    // frames too alike at that size to show the change.
    f.texel_px = max(length(gx), length(gy)) * IMPOSTOR_FRAMES * FRAME_TEXELS;
    f.c0 = vec4<f32>(0.0);
    f.c1 = vec4<f32>(0.0);
    f.c2 = vec4<f32>(0.0);
    f.k = w;
    if f.texel_px < 2.0 {
        f.c0 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv0, colour_layer, gx, gy);
        f.c1 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv1, colour_layer, gx, gy);
        f.c2 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv2, colour_layer, gx, gy);
    } else if w.x >= w.y && w.x >= w.z {
        f.c0 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv0, colour_layer, gx, gy);
        f.k = vec3<f32>(1.0, 0.0, 0.0);
    } else if w.y >= w.z {
        f.c1 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv1, colour_layer, gx, gy);
        f.k = vec3<f32>(0.0, 1.0, 0.0);
    } else {
        f.c2 = textureSampleGrad(tree_atlas, clamp_sampler, f.uv2, colour_layer, gx, gy);
        f.k = vec3<f32>(0.0, 0.0, 1.0);
    }
    f.coverage = f.c0.a * f.k.x + f.c1.a * f.k.y + f.c2.a * f.k.z;
    return f;
}

// The depth prepass (`near_trees::tree_pipelines`): the card's pixels inside the crown's sharpened edge (it keeps its
// coverage in every mip level) and not its mesh's write their depth; the light (`fs_tree`) lights only those still in
// front once every tree's depth is in.
@fragment
fn fs_tree_depth(in: VertexOut) -> @location(0) vec4<f32> {
    let coverage = frames_of(in).coverage;
    let edge = clamp((coverage - EDGE) / max(fwidth(coverage), 1e-4) + 0.5, 0.0, 1.0);
    if edge < 0.5 || pixel_dither(in.clip.xy) < in.near {
        discard;
    }
    return vec4<f32>(0.0);
}

// A pixel the prepass found standing in front, lit (drawn with the depth test `Equal`). It keeps a discard that no
// pixel takes: on Apple's GPUs a shader that cannot discard, drawn under `Equal`, read unsteady neighbours across its
// 2×2 quads, so the frames' derivatives (`frames_of`: their inset, blend and mip level) changed from frame to frame in
// about one pixel of the trees' fifty, more under load (the terrain tests' flakes, 2026-10-27).
@fragment
fn fs_tree(in: VertexOut) -> @location(0) vec4<f32> {
    let colour_layer = i32(in.species);
    let frames = frames_of(in);
    // A card its mesh draws whole (the vertex stage drops it): the discard the quads need (above).
    if in.near >= 1.0 {
        discard;
    }
    let gx = frames.gx;
    let gy = frames.gy;
    let uv0 = frames.uv0;
    let uv1 = frames.uv1;
    let uv2 = frames.uv2;
    let w = frames.w;
    let texel_px = frames.texel_px;
    let c0 = frames.c0;
    let c1 = frames.c1;
    let c2 = frames.c2;
    var k = frames.k;
    let coverage = frames.coverage;
    // The colour layers are premultiplied (a distant crown's mip averages no empty texel's black into it) and sRGB;
    // the colour takes the land's tone (`tone_down`), as the ground under the trees does.
    let encoded = (c0.rgb * k.x + c1.rgb * k.y + c2.rgb * k.z) / max(coverage, 1e-3);
    var albedo = srgb_decode(encoded);
    // The normal, depth and occlusion: up close blended over the three frames where each covers; where a frame's texel
    // spans less than a pixel, the frame that weighs most alone (a crown a few pixels across shows no seam between
    // frames, and it saves two samples a pixel).
    let normal_layer = i32((textureNumLayers(tree_atlas) - LEAF_LAYERS) / 2u) + colour_layer;
    k = k * vec3<f32>(c0.a, c1.a, c2.a);
    let bound = in.open_bound.y;
    var model_normal: vec3<f32>;
    var p: vec3<f32>;
    var packed: f32;
    if texel_px < 0.75 {
        let n0 = textureSampleGrad(tree_atlas, clamp_sampler, uv0, normal_layer, gx, gy);
        let n1 = textureSampleGrad(tree_atlas, clamp_sampler, uv1, normal_layer, gx, gy);
        let n2 = textureSampleGrad(tree_atlas, clamp_sampler, uv2, normal_layer, gx, gy);
        let total = max(k.x + k.y + k.z, 1e-4);
        model_normal = decode_normal(n0.xy) * k.x + decode_normal(n1.xy) * k.y + decode_normal(n2.xy) * k.z;
        let d0 = frame_direction(in.cells01.xy);
        let d1 = frame_direction(in.cells01.zw);
        let d2 = frame_direction(in.cell2);
        p = (frame_point(in.q, d0, n0.z, bound) * k.x + frame_point(in.q, d1, n1.z, bound) * k.y
            + frame_point(in.q, d2, n2.z, bound) * k.z) / total;
        packed = (n0.w * w.x + n1.w * w.y + n2.w * w.z) / max(c0.a * w.x + c1.a * w.y + c2.a * w.z, 1e-4);
    } else {
        var uv = uv0;
        var cell = in.cells01.xy;
        var cover = c0.a;
        if k.y > k.x && k.y >= k.z {
            uv = uv1;
            cell = in.cells01.zw;
            cover = c1.a;
        } else if k.z > k.x && k.z > k.y {
            uv = uv2;
            cell = in.cell2;
            cover = c2.a;
        }
        let n = textureSampleGrad(tree_atlas, clamp_sampler, uv, normal_layer, gx, gy);
        model_normal = decode_normal(n.xy);
        p = frame_point(in.q, frame_direction(cell), n.z, bound);
        packed = n.w / max(cover, 1e-4);
    }
    model_normal = normalize(model_normal + vec3<f32>(0.0, 1e-4, 0.0));
    let material = material_of(packed);
    // A bare crown's mass is its twigs, baked as its leaves would be (`tree_bake.wgsl`).
    let bare = select(0.0, material.x, in.species == SPECIES_BARE_OAK || in.species == SPECIES_BARE_BIRCH);
    let leafy = material.x - bare;
    let sky = material.y;
    albedo = tone_down(albedo * tree_tint(in.tint, leafy));
    var placed: Placed;
    placed.middle = in.middle.xyz;
    placed.height = in.middle.w;
    placed.across = in.turn.zw;
    placed.turn = in.turn.xy;
    // Height up the tree, 0 at its foot.
    let up_tree = clamp(p.y + 0.5, 0.0, 1.0);
    let world = in.middle.xyz + to_world(placed, p) + lean_by(in.lean.xy, in.lean.z, up_tree, placed.height);
    let normal = normal_to_world(placed, model_normal);
    // Up close a frame's texel spans pixels: leaves finer than it, from a noise fixed to the tree (about 0.5 m), faint
    // (the bake's own leaves and their depth in the crown vary it enough).
    let fine = (1.0 - ramp(0.3, 0.75, texel_px)) * leafy;
    if fine > 0.01 {
        albedo *= mix(1.0, 0.55 + 0.9 * leaf_noise(p * vec3<f32>(in.turn.z, in.middle.w, in.turn.w) * 2.0), fine * 0.3);
    }

    var f: Foliage;
    f.albedo = albedo;
    f.world = world;
    f.normal = normal;
    f.middle = in.middle.xyz;
    f.leafy = leafy;
    f.twig = bare;
    f.sky = sky;
    f.slope = vec4<f32>(0.0);
    // Its bark takes another crown's shadow as the mesh's does, so the crossfade keeps the bark's light.
    f.tube = 1.0 - material.x;
    f.up_tree = up_tree;
    f.height = placed.height;
    f.open = in.open_bound.x;
    f.sun_at_foot = in.open_bound.z;
    return vec4<f32>(light_foliage(f), 1.0);
}
