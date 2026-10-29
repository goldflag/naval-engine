// The trees near the camera as meshes (native/docs/terrain/trees.md, "Near trees"): every species' grown tree as tubes
// of bark and cards of leafy sprays (`naval_terrain::tree_meshes`, built once a process), instanced for the trees the
// binning (`tree_near_bin.wgsl`) put in each species' and level's list this frame, through an indirect draw a list.
//
// A tree is placed as its impostor is (`trees.wgsl`: its foot, height, width and turn), lit as it is
// (`naval_terrain::foliage`) and bent by the wind as it is (`naval_terrain::tree_wind`), so the crossfade between the
// two shows no seam: a pixel's place in the crossfade (`pixel_dither`) gives it to the full mesh, the reduced one or
// the impostor by the tree's distance. The sprays are alpha-tested against the leaf sprites (the impostor atlas's last
// layers), each leaf brightened and its clump's normal tilted a little as the sprite says, its twigs in the bark's
// colour (`naval_terrain::tree_surface`, which the impostors' bake shares), and flutter in the wind on their own. Each
// vertex carries the sky its crown leaves it and that sky's slope (`naval_terrain::crown`).
#import naval::camera::{world_to_clip}
#import naval_terrain::bindings::{terrain, sunlight, clamp_sampler, tree_atlas, field_uv, chart_offset}
#import naval_terrain::surface::{tone_down}
#import naval_terrain::foliage::{Foliage, light_foliage, tree_tint}
#import naval_terrain::tree_wind::{tree_sway, bend, lean_by, near_shares, pixel_dither}
#import naval_terrain::impostor::{LEAF_LAYERS, placed_by, form_of, to_world, normal_to_world, turn_by}
#import naval_terrain::tree_surface::{TreeSurface, spray_surface, bark_surface}

const TAU: f32 = 6.283185307179586;
// The leaf sprites' slots a side on a layer, and the sprays a side a reduced card stands for, its sprite repeated as
// many times each way (`tree_meshes::SPRITE_GRID`, `REDUCED_TILES`).
const SPRITE_GRID: f32 = 2.0;
const REDUCED_TILES: f32 = 3.0;
// How far a leaf's own tilt on its card turns its clump's normal up close (`tree_bake.wgsl` takes less).
const NEAR_TILT: f32 = 0.35;
// A vertex's slope of the sky through its crown, over this (`crown::SLOPE_RANGE`).
const SLOPE_RANGE: f32 = 4.0;

struct VertexIn {
    // The mesh (`tree_meshes::MeshVertex`): model space (the foot at 0, the top at y = 1); the shading normal (w: the
    // level of detail); a card's direction across its sprite (w: its sprite's layer) or a limb's along it; the
    // sprite's or the bark's coordinates; the albedo (sRGB) and the sky through its crown; what it is (0 bark, 0.5
    // leaves, 1 bare twigs), its alpha cut (bark: its pattern), its phase and flutter; that sky's slope.
    @location(0) position: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    @location(5) wind: vec4<f32>,
    @location(9) sky_slope: vec4<f32>,
    // The placed tree (`trees.wgsl`'s `VertexIn`).
    @location(6) foot: vec4<f32>,
    @location(7) shape: vec4<f32>,
    @location(8) tint: vec4<f32>,
}

struct VertexOut {
    // The same in the depth prepass and the light (`fs_near_depth`, `fs_near`), so the light's `Equal` test meets the
    // depth the prepass wrote.
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) color: vec4<f32>,
    // The height up the tree (0-1).
    @location(5) up_tree: f32,
    @location(6) @interpolate(flat) middle: vec4<f32>,
    @location(7) @interpolate(flat) tint_open: vec4<f32>,
    // The sun past the land at the foot; what it is; its alpha cut; the level of detail (0 or 1).
    @location(8) @interpolate(flat) sun_kind: vec4<f32>,
    // The tree's shares: of its pixels its mesh's, of those its full detail's.
    @location(9) @interpolate(flat) shares: vec2<f32>,
    // A card's sprite's layer among the leaf sprites', and how many times over each way the card carries it.
    @location(10) @interpolate(flat) sprite: vec2<f32>,
    // The slope of the sky through the crown (world).
    @location(11) slope: vec3<f32>,
}

@vertex
fn vs_near(in: VertexIn) -> VertexOut {
    let height = in.foot.w;
    let offset = vec3<f32>(chart_offset().x, 0.0, chart_offset().y);
    let form = form_of(in.shape.z);
    let placed = placed_by(in.foot.xyz + vec3<f32>(0.0, height * 0.5, 0.0) + offset, height, abs(in.tint.w), form);
    let q = in.position - vec3<f32>(0.0, 0.5, 0.0);
    var world = placed.middle + to_world(placed, q);
    let normal = normal_to_world(placed, in.normal.xyz);
    let up = clamp(in.position.y, 0.0, 1.0);
    // The tree bent by the wind as its card is; its leaves and twigs swaying on their branches, more the farther out,
    // and fluttering, each clump at its own phase.
    let top = tree_sway(in.foot.xz, height);
    world += bend(top, up, height) + lean_by(form.lean, form.bow, up, height);
    let speed = terrain.wind.z;
    let kind = in.wind.x;
    if kind > 0.25 && speed >= 0.4 {
        let t = terrain.wind.w;
        let phase = in.wind.z * TAU;
        let calm = smoothstep(0.4, 3.0, speed);
        let out_m = length(q.xz * placed.across);
        let branch = min(0.006 * speed, 0.07) * out_m * calm;
        let dir = vec3<f32>(terrain.wind.x, 0.0, terrain.wind.y);
        world += (dir * (0.5 + 0.5 * sin(t * 1.3 * TAU + phase)) + vec3<f32>(0.0, 0.6, 0.0) * sin(t * 1.7 * TAU + phase * 1.3))
            * branch;
        let flutter = in.wind.w * min(0.004 * speed, 0.05) * calm;
        let shake = sin(t * 4.1 * TAU + phase * 3.0 + dot(in.position, vec3<f32>(37.0, 23.0, 41.0)));
        world += normal * (flutter * shake * min(height, 20.0) * 0.25);
    }
    // The tree's shares, for a tree the meshes have room for (a tree the binning had none for carries its width
    // negated, and its impostor draws it whole).
    var shares = near_shares(placed.middle, height);
    var out: VertexOut;
    out.clip = world_to_clip(world);
    if in.tint.w < 0.0 || height <= 0.0 {
        out.clip = vec4<f32>(0.0, 0.0, -1.0, 1.0);
        shares = vec2<f32>(0.0);
    }
    out.world = world;
    out.normal = normal;
    out.tangent = normalize(to_world(placed, in.tangent.xyz) + vec3<f32>(1e-6, 0.0, 0.0));
    out.uv = in.uv;
    out.color = in.color;
    out.slope = turn_by(in.sky_slope.xyz * SLOPE_RANGE, placed.turn);
    out.up_tree = up;
    // The layer in 32s, a reduced card marked over it (`tree_meshes::TILED_MARK`).
    let code = in.tangent.w * 127.0 / 32.0;
    out.sprite = vec2<f32>(round(code), select(1.0, REDUCED_TILES, code - round(code) > 0.125));
    out.middle = vec4<f32>(placed.middle, height);
    out.tint_open = vec4<f32>(in.tint.rgb, in.shape.w);
    out.sun_kind = vec4<f32>(
        textureSampleLevel(sunlight, clamp_sampler, field_uv(in.foot.xz), 0.0).r,
        kind,
        in.wind.y,
        select(0.0, 1.0, in.normal.w > 0.5),
    );
    out.shares = shares;
    return out;
}

// The leaf sprite under a pixel (its level from the coordinates' own derivatives); bark's is plain. A reduced card
// carries its slot's sprite `tiles` times over each way, so its leaves keep their size.
fn leaf_of(in: VertexOut) -> vec4<f32> {
    let tiles = in.sprite.y;
    let gx = dpdx(in.uv) * tiles;
    let gy = dpdy(in.uv) * tiles;
    if in.sun_kind.y <= 0.25 {
        return vec4<f32>(0.5, 0.5, 0.5, 1.0);
    }
    var uv = in.uv;
    if tiles > 1.0 {
        // The slot's corner (the card's coordinates lie inside it), then the place in it repeated.
        let slot = floor(uv * SPRITE_GRID) / SPRITE_GRID;
        uv = slot + fract((uv - slot) * SPRITE_GRID * tiles) / SPRITE_GRID;
    }
    let layer = i32(textureNumLayers(tree_atlas) - LEAF_LAYERS) + i32(in.sprite.x);
    return textureSampleGrad(tree_atlas, clamp_sampler, uv, layer, gx, gy);
}

// Whether a pixel of a tree's mesh stands: its part of the crossfade (the full mesh takes [0, m f), the reduced one
// [m f, m), the impostor the rest), and for a spray, what its sprite covers.
fn stands(in: VertexOut, leaf: vec4<f32>) -> bool {
    let u = pixel_dither(in.clip.xy);
    let full = in.shares.x * in.shares.y;
    let lod1 = in.sun_kind.w > 0.5;
    let mine = select((u < full), (full <= u) && (u < in.shares.x), lod1);
    // The reduced level's fewer, larger cards hold their leaves a little fuller, as its crowns are as solid as the
    // full level's from where it is seen.
    let cut = in.sun_kind.z * select(1.0, 0.8, lod1);
    return mine && !(in.sun_kind.y > 0.25 && leaf.a < cut);
}

// The depth prepass (`near_trees::tree_pipelines`): the pixels of the mesh that stand write their depth, and the light
// (`fs_near`) lights only those that are still in front once every tree's depth is in.
@fragment
fn fs_near_depth(in: VertexOut) -> @location(0) vec4<f32> {
    // The sprite read before any pixel leaves (its derivatives need the whole quad).
    if !stands(in, leaf_of(in)) {
        discard;
    }
    return vec4<f32>(0.0);
}

// A pixel the prepass found standing in front, lit (drawn with the depth test `Equal`). It keeps a discard that no
// pixel the prepass stood takes, for its quads' derivatives on Apple's GPUs (`trees.wgsl`, `fs_tree`): without it the
// leaf sprite's level changed from frame to frame in a few of the trees' pixels.
@fragment
fn fs_near(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let leaf = leaf_of(in);
    // A tree its impostor draws whole, its mesh's pixels none of the prepass's.
    if in.shares.x <= 0.0 {
        discard;
    }
    var normal = normalize(in.normal);
    var s: TreeSurface;
    if in.sun_kind.y > 0.25 {
        // The clump's normal from either side (its far side faces away from the eye and shades so: the crown's depth
        // is its sky's), each leaf's tilt on it a detail.
        s = spray_surface(leaf, in.color.rgb, normal, in.tangent, in.sun_kind.y > 0.75, NEAR_TILT, !front);
    } else {
        if !front {
            normal = -normal;
        }
        s = bark_surface(u32(round(in.sun_kind.z * 255.0 / 32.0)), in.uv, in.color.rgb, normal, in.tangent);
    }
    let leafy = s.leafy > 0.5;
    var f: Foliage;
    f.albedo = tone_down(s.albedo * tree_tint(in.tint_open.rgb, s.leafy));
    f.world = in.world;
    f.normal = s.normal;
    f.middle = in.middle.xyz;
    f.leafy = s.leafy;
    f.twig = select(0.0, 1.0, in.sun_kind.y > 0.75);
    f.sky = in.color.a;
    f.slope = vec4<f32>(in.slope, 1.0);
    f.tube = select(0.0, 1.0, in.sun_kind.y <= 0.25);
    f.up_tree = in.up_tree;
    f.height = in.middle.w;
    f.open = in.tint_open.w;
    f.sun_at_foot = in.sun_kind.x;
    return vec4<f32>(light_foliage(f), 1.0);
}
