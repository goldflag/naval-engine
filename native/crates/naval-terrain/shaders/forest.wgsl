// The forest's shade: what the trees take from the sun and the sky beneath and among them, for the land and the trees.
//
// Native (the web's trees cast nothing). Near the sun's focus the trees cast into the near and wide shadow maps, and on
// High and Ultra into the land maps over the camera's view of the land (`tree_caster.wgsl`), and the land and the cards
// receive those shadows through `sun_shadow`. Everywhere else, and for
// the sky's light everywhere, these stand in from the forest's shade on the attribute grid (`tree_shade.wgsl`: how many
// trees stand per cell and how tall): the share of the forest floor in the crowns' shadow at the sun's elevation, the
// floor's sky hidden by the crowns, and the lower crowns in their neighbours' shadow. Where the maps carry the trees'
// shadows (`tree_map_share`), the sun's stand-ins step aside, so no ground is darkened twice.
#define_import_path naval_terrain::forest

#import naval::camera::{camera}
#import naval::shadow::{shadows, shadows_enabled, shadow_near_share, shadow_cover, shadow_land_share}
#import naval_terrain::bindings::{terrain, open_sky, clamp_sampler, coarse_uv, cleared}

// The forest shade's height scale: its blue channel × this is the trees' mean height (m).
const FOREST_HEIGHT_M: f32 = 50.0;
// The trees' full-density distance and the game's lens (`TREE_FULL_M`, `TREE_LENS_TAN`).
const TREE_FULL_M: f32 = 4500.0;
const TREE_LENS_TAN: f32 = 0.4877326;
// A crown's width over the tree's height, and the share of the tree its crown fills (the trunk below it).
const CROWN_WIDTH: f32 = 0.7;
const CROWN_DEPTH: f32 = 0.6;
// The share of a crown's outline that shades another crown's leaves, matched to the cards' own shadows in the maps
// (the ray toward the sun from a lit leaf passes the neighbours' outlines through their gaps and over their edges).
const CROWN_GAPS: f32 = 0.32;

struct Forest {
    // The trees standing per tree cell (0-1).
    cover: f32,
    // Their mean height (m).
    height: f32,
}

// The forest at chart point `chart` (none in a clearing).
fn forest_at(chart: vec2<f32>) -> Forest {
    // Baked into the open sky's green and blue (`tree_shade.wgsl`).
    let shade = textureSampleLevel(open_sky, clamp_sampler, coarse_uv(chart), 0.0);
    var f: Forest;
    f.cover = shade.g * (1.0 - cleared(chart));
    f.height = shade.b * FOREST_HEIGHT_M;
    return f;
}

// How much of the sun's shadow at `world` comes from maps the trees cast into this frame (`sun_shadow`'s weights of
// the near, aboard and wide maps, and of the land maps beyond them): there the trees' own shadows fall.
fn tree_map_share(world: vec3<f32>) -> f32 {
    if !shadows_enabled() {
        return 0.0;
    }
    let near = shadow_near_share(world);
    let aboard = (1.0 - near) * shadow_cover(2u, world) * shadows.maps[2].state.x;
    let wide = (1.0 - near - aboard) * shadow_cover(1u, world) * shadows.maps[1].state.x;
    return near * terrain.fill_sky.w + aboard * terrain.low.w + wide * terrain.fill_ground.w
        + shadow_land_share(world) * terrain.shore.w;
}

// The cotangent of the sun's elevation, from its direction's height (kept finite at the horizon).
fn sun_cotangent(sun_y: f32) -> f32 {
    let s = clamp(sun_y, 0.04, 1.0);
    return sqrt(1.0 - s * s) / s;
}

// The share of the forest floor in the crowns' shadow: each crown shades a patch its width across and as long as the
// crown throws its shadow at the sun's elevation, one tree per cell of the level-0 spacing where they stand, the
// patches falling over each other at random (Beer's law). Half the floor under a closed canopy at noon; all of it at
// a low sun.
fn floor_shadow(f: Forest, sun_y: f32) -> f32 {
    let spacing = terrain.field.w;
    let width = f.height * CROWN_WIDTH;
    let area = width * (width + f.height * CROWN_DEPTH * sun_cotangent(sun_y));
    return 1.0 - exp(-f.cover * area / (spacing * spacing));
}

// The share of the sun the neighbouring crowns toward it take from a crown at `height` (0 at its foot, 1 its top): the
// ray toward the sun runs under the canopy's top for (1 - height) of the tree's height over the elevation's tangent,
// crossing as many crowns as stand on the strip it sweeps.
fn crown_shade(f: Forest, height: f32, sun_y: f32) -> f32 {
    let spacing = terrain.field.w;
    let run = (1.0 - height) * f.height * sun_cotangent(sun_y);
    return 1.0 - exp(-CROWN_GAPS * f.cover * run * f.height * CROWN_WIDTH / (spacing * spacing));
}

// The share of the trees still standing at `reach` metres from the camera: all of them near, thinning as (full / d)²
// past the full-density distance (apparent, by the lens), none past the trees' range (`tree_place.wgsl`'s level of
// detail, as a mean over the ranks).
fn trees_standing(reach: f32) -> f32 {
    let zoom = sqrt(camera.tan_half_fov_y / TREE_LENS_TAN);
    let apparent = reach * zoom;
    let range = max(terrain.offset.z, 1.0);
    let ending = clamp((reach - range * 0.7) / (range * 0.3), 0.0, 1.0);
    return min(pow(TREE_FULL_M / max(apparent, 1.0), 2.0), 1.0) * (1.0 - ending);
}

// How far the eye sees the ground at `world` from beneath the crowns of the trees standing there (1) rather than from
// above them (0), by its height over that ground against theirs: beneath, the ground between the cards is their floor;
// from above, it stands in for the crowns the cards leave out between them (the canopy's pattern).
fn beneath_crowns(f: Forest, world: vec3<f32>) -> f32 {
    let top = max(f.height, 6.0);
    return 1.0 - smoothstep(top, top * 2.5, camera.position.y - world.y);
}

// How much of the ground at `world`, under the forest `f` (`forest_at`), the eye sees as the forest floor
// (`SurfaceInputs::floor`): where the trees stand (all of them near, thinning far off), from beneath their crowns, fading
// in over the first 0.05 of their cover (so it has no hard edge where a wood's cells end, the cover's bilinear ramp
// there crossing zero). None where no tree stands (most of a country's land, which so skips it).
fn floor_seen(f: Forest, world: vec3<f32>) -> f32 {
    return cards_standing(f, world) * beneath_crowns(f, world);
}

// How many of the trees of the forest `f` at `world` stand drawn (`SurfaceInputs::standing`): all of them near,
// thinning far off, faded in over the first 0.05 of their cover as `floor_seen`. Where they stand the ground paints no
// crowns of its own (`surface.wgsl`'s `canopy`), from above or beneath: the crowns are theirs.
fn cards_standing(f: Forest, world: vec3<f32>) -> f32 {
    if f.cover <= 0.0 {
        return 0.0;
    }
    return trees_standing(distance(camera.position, world)) * smoothstep(0.0, 0.05, f.cover);
}

// The ground's sun and sky under the forest `f` at `world` (`forest_at`), as factors on the sun reaching it and on its
// ambient occlusion, and how much of its light the leaves give it (`under_leaves`). Near, the ground between the cards
// is the forest floor: the crowns over it hide its sky and, past the trees' own shadows in the maps, its sun. Far,
// where the cards thin out, the ground paints the crowns of the trees left out (the canopy's pattern): their tops, a
// little self-shaded at a low sun. Seen from beneath the crowns the floor keeps a third of the sky under a closed
// canopy (its gaps, 2-10% of the sky on a rainforest's floor and 5-20% in a broadleaf wood in leaf, and the light the
// leaves pass and scatter down), and that light is the leaves' (`under_leaves`): so it shows the sky's blue, and
// reflects its sheen, only faintly. Seen from above it keeps two-thirds, standing in for crowns, lit by the sky; and so
// does the ground under a thin cover (a hedgerow, a wood's edge, whose cells' cover reaches out over the field beside
// it), the closed canopy's share coming in with the square of the cover.
fn forest_floor(f: Forest, world: vec3<f32>) -> vec3<f32> {
    if f.cover <= 0.0 {
        return vec3<f32>(1.0, 1.0, 0.0);
    }
    let sun_y = terrain.sun_direction.y;
    let standing = trees_standing(distance(camera.position, world));
    let floor_lit = mix(1.0 - floor_shadow(f, sun_y), 1.0, tree_map_share(world));
    let canopy_lit = 1.0 - 0.35 * f.cover * (1.0 - clamp(sun_y, 0.0, 1.0));
    let sun = mix(canopy_lit, floor_lit, standing);
    let closed = beneath_crowns(f, world) * f.cover * f.cover;
    let hidden = mix(0.35, 0.7, closed);
    let sky = 1.0 - hidden * f.cover * standing;
    return vec3<f32>(sun, sky, closed * standing);
}

const LIGHT_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// The colour of the light leaves pass and scatter down, of unit luminance: they take more of the blue and the red than
// of the green (0.9, 1, 0.7 of the daylight on them; the floor's light under a closed canopy, Endler's "forest
// shade", is greener still, its gaps' sky bluer).
const LEAF_LIGHT: vec3<f32> = vec3<f32>(0.9404, 1.0448, 0.7314);

// The light `light` (a share of the sky's, its reflection or the fill) as it reaches the ground under the leaves, by
// how much of it the leaves give (`leaves`, `forest_floor`'s third factor): its strength kept, its colour the leaves'
// light from the `daylight` on the canopy (the sun's and the sky's on the level, so warm at a low sun). Under a closed
// canopy nearly all of what reaches the floor has passed through or off leaves, and what it reflects is leaves, not the
// sky: kept the sky's own colour (multiplied by the leaves' only), the floor's light was a deep blue (red half its
// green, blue one and a half), so the litter read as grey or green gravel. None from above (the ground stands in for
// the crowns' tops, lit by the sky) or under a thin cover; all beneath a closed canopy.
fn under_leaves(light: vec3<f32>, daylight: vec3<f32>, leaves: f32) -> vec3<f32> {
    let passed = daylight / max(dot(daylight, LIGHT_LUMA), 1e-6) * LEAF_LIGHT;
    return mix(light, passed * dot(light, LIGHT_LUMA), leaves);
}
