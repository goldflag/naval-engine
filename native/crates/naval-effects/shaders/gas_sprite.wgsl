// Gas puffs: camera-facing sprites that relight a baked puff of the gas atlas. A port of `gasSpriteMaterial`
// (the web game's `GasAtlas.ts`) and, in `occluder`, its smoke occlusion variant (`SmokeOcclusion.occluder`).
//
// Every puff draws its own tile (picked by its seed), turned by a seeded angle that drifts as it ages and
// squeezed across the projected axis of an elongated puff. The sun and flashes light it through the six baked
// directions; the sky lights it mostly from above. Hot gas is sooty and glows where the bake saw the hottest
// gas: as a fireball cools, the gas that still burns shrinks to its hottest cells. Thinning with dispersal
// lowers the puff's optical depth and its thinnest rim goes first. Close up, the tiling detail map erodes the
// thin rim into small billows. Per-puff values are worked out per vertex.

#import naval::camera::{camera, world_to_clip, curved, view_depth}
#import naval_effects::common::{sky_fill, sky_toward, lighting, emitted, emitted_at, flame_light, effect_fog, soft_edge, smoke_visibility, columns_through, clouds_through, splashes_through, nearness_of, quad_corner, six_way, sstep, scene_depth, MergedSmoke, merged_smoke}
#import naval::aerial::aerial_at
// Trial `flipbook`: a puff drawn from a simulation's frames (`puff.extra.z` names its clip).
#import naval_effects::flipbook::{FlipFrame, flipbook_puff, flipbook_quad, flipbook_varying, flipbook_frame, flipbook_body_at, flipbook_gas, flipbook_sides_at, flipbook_stand, flipbook_link, flipbook_loop, flipbook_flame, flipbook_views, flipbook_widen}
#import naval_effects::shadow::puff_light
#import naval_effects::warp::{roll_phase, roll_flow, roll_layers, roll_detail_phase}
#import naval_effects::smoke_screen::screen_coord

const GAS_GRID: f32 = 4.0;
const GAS_VARIANTS: f32 = 16.0;
const GAS_DEPTH: f32 = 12.0;
// A fire's flame (`flipbook_fires.rs`) is a card standing for a body of flame about as deep as it is wide (its tile's
// width, `2 × radius × aspect`): the card is drawn at the body's front, this many half-widths nearer the lens than the
// fire, and the flame fades out over this many half-widths where the deck or plating behind it lies within the body.
const FLAME_FRONT: f32 = 0.9;
const FLAME_DEPTH: f32 = 1.4;

// The sky's light on the gas (2026-10-23). Each of the six baked directions takes what a face looking that way sees:
// the sky above the horizon, the sea below it (`sky_share`, the hemisphere's rule), the sea at `SEA_SKY` of the sky.
// A direction's lobe of the six-way weights gathers 2π/3 of the radiance about it where the sun's beam gives its
// irradiance, and `sky_fill` is the irradiance from overhead (π of the sky's radiance), so a lobe takes 2/3 of
// `sky_fill`: `SKY_LOBE` × 1.7. Before, the sky came half from the tile's top and a tenth from its front whichever way
// the tile looked, so a bank's face toward the eye stood in the dark, from above its whole face did, and smoke under a
// cloud's shadow or an overcast was black.
const SEA_SKY: f32 = 0.12;
const SKY_LOBE: f32 = 0.39;
// Light scattered many times inside dense gas: shares of the sun's and the sky's reaching the puff (the six baked
// directions hold only the light scattered once, which leaves a dense bank's shaded side black), reached as the gas
// thickens over its optical depth (× `MULTIPLE_REACH`). They were set on gun smoke (2026-10-23). Each scattering gives
// back the gas's albedo of the light, so a gas darker than gun smoke gives back less of it again: the shares go as its
// reflectance over `MULTIPLE_ALBEDO`, the darkest gun smoke's (blasts.rs' `gunsmoke_dark` at 0.85 of its shade, tinted
// twice: 9 %), at most all of them, and a gun's own gas keeps them all (2026-10-24). A fuel-oil fire's soot (2 to 5 %)
// keeps a quarter to a half of them, a funnel's black smoke (2 %) a quarter; steam, a funnel's grey smoke and gun
// smoke all.
const MULTIPLE_SUN: f32 = 0.08;
const MULTIPLE_SKY: f32 = 0.6;
const MULTIPLE_REACH: f32 = 0.35;
const MULTIPLE_ALBEDO: f32 = 0.09;
// Smoke lies on the sea, never in it: seen along the sea, a puff thins away over its lowest share of its radius above
// the water, so a bank's foot is soft rather than cut straight along the waterline where the sea hides the rest of
// its card (the soft edge, which fades by the depth behind, does nothing at so flat an angle).
const SEA_FOOT: f32 = 0.25;

// What gas a puff is (`GasPuff.extra.w`; `GAS_SCREEN`, `GAS_PROPELLANT`, `GAS_COLUMN`, `GAS_FIRE`, `GAS_FIREBALL` and
// `GAS_FLAK` in src/instances.rs): smoke and an explosive's fireball (0), a smoke screen (1), a gun's propellant (2), a
// billow of a fire's column drawn on the links' lean path, which does not turn its tile (3), a fire's rolling flame body
// (4), a gun's fireball, its propellant's flash (5), a heavy AA round's burst of soot, marched through its sphere (6,
// `flak_soot`).
const GAS_SCREEN: f32 = 1.0;
const GAS_PROPELLANT: f32 = 2.0;
const GAS_COLUMN: f32 = 3.0;
const GAS_FIRE: f32 = 4.0;
const GAS_FIREBALL: f32 = 5.0;
const GAS_FLAK: f32 = 6.0;
// A gun's fireball burns white-hot, all but its thinnest gas, while the share of its burning still to come is over the
// second of these, going over to its flame's own colours by the first: the first quarter of a lobe's burning (a tenth
// of a second for a battleship's guns), as long as its flash's ball is white (blasts.rs' `FLASH_HOLD`). The films of
// battleships' broadsides overexpose it as a white ball through their first tenth of a second.
const WHITE_HEAT: vec2<f32> = vec2<f32>(0.95, 1.15);
// Its fresh soot reflects this share of its smoke's colour until it mixes with the air, paling to the smoke's own
// between these ages (s): black specks in the red fireball at a fifth of a second, a dark brown cloud at a third,
// brown-grey by 0.6 s.
const FRESH_SOOT: f32 = 0.5;
const SOOT_PALES: vec2<f32> = vec2<f32>(0.1, 0.6);

struct GasPuff {
    sphere: vec4<f32>,
    state: vec4<f32>,
    tint: vec4<f32>,
    evolution: vec4<f32>,
    quad: vec4<f32>,
    extra: vec4<f32>,
    // The warp trial: how far the puff has rolled, and the world direction it rolls about.
    warp: vec4<f32>,
}

struct GasBatch {
    // rgb colour of a fire's light on its gas; w 1 to read the flashes, which light it in their own colour
    // (`lighting.flash_color`, `flash_light_color`).
    flash_color: vec4<f32>,
    // x 1 for soft edges against the scene, y 1 for an occluding batch (reads only the densest level),
    // z 1 to apply the occlusion map at all, 2 to apply its fires' columns too (a batch drawn after them: `columns_through`),
    // w 1 to multiply the result by the puff's albedo once more: three multiplies
    // a material's colour by `instanceColor` when the mesh has one, and `EffectParticlePool` meshes do (the particle's
    // colour), so the web's pool gas is tinted twice, its fire included; `FireBatch` meshes (flame bodies) have none.
    flags: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> puffs: array<GasPuff>;
@group(2) @binding(1) var<uniform> batch: GasBatch;
@group(2) @binding(2) var atlas_sides: texture_2d<f32>;
@group(2) @binding(3) var atlas_body: texture_2d<f32>;
@group(2) @binding(4) var atlas_detail: texture_2d<f32>;
@group(2) @binding(5) var atlas_sampler: sampler;
@group(2) @binding(6) var detail_sampler: sampler;
@group(2) @binding(7) var screen_noise: texture_3d<f32>;
@group(2) @binding(8) var screen_sampler: sampler;

// The colour of the light `flash_lit` sums: the flashes' own where the batch reads them (their 2,500 K light, reddening
// to the fireball's glow, as on the hulls and the sea), the batch's fire light where it does not.
fn flash_light_color() -> vec3<f32> {
    return select(batch.flash_color.rgb, lighting.flash_color.rgb, batch.flash_color.w > 0.0);
}

// The sky's light on a face of the gas looking `up` of the way up (−1 straight down, 1 straight up), as a share of the
// light from overhead: the sky fills the half of its view above the horizon, the sea the rest (`SEA_SKY`).
fn sky_share(up: f32) -> f32 {
    return mix(SEA_SKY, 1.0, 0.5 + 0.5 * up);
}

// How far the tile's x, y and z axes look up in the world. Its z axis faces the lens (`light.z`); `right` is level, so
// its y axis is the quad's up turned by the tile's turn (`frame.xy`) and its x axis the rest of the way, mirrored with
// a mirrored flipbook puff's light (`flip.w` < 0).
fn axes_up(in: Varyings) -> vec3<f32> {
    let level = sqrt(max(1.0 - in.light.z * in.light.z, 0.0));
    return vec3<f32>(-in.frame.y * level * select(1.0, -1.0, in.flip.w < 0.0), in.frame.x * level, in.light.z);
}

// The share of a puff's coverage left at its foot on the sea (`SEA_FOOT`), by the height of the point on its own plane
// (`light.w`), the less the more steeply the eye looks down on it; a fire's flames and gas keep theirs.
fn sea_foot(in: Varyings) -> f32 {
    if (in.flip_loop.w > 1.5 && in.flip_loop.w < 2.5) || in.flip_loop.y > 0.5 {
        return 1.0;
    }
    return mix(sstep(-0.5, max(in.quad.w * SEA_FOOT, 2.0), in.light.w), 1.0, abs(in.light.z));
}

struct Varyings {
    @builtin(position) position: vec4<f32>,
    // Sun in the tile's frame (xy, scaled by its reach), detail strength, detail seed.
    @location(0) @interpolate(flat) detailed: vec4<f32>,
    // Cosine and sine of the tile's turn, the elongation axis on screen.
    @location(1) @interpolate(flat) frame: vec4<f32>,
    // Tile column and row, squeeze, soft-edge metres.
    @location(2) @interpolate(flat) place: vec4<f32>,
    // Albedo, opacity (cleared as the lens enters the puff).
    @location(3) @interpolate(flat) look: vec4<f32>,
    // Heat, extinction multiple as it thins, dispersal, age.
    @location(4) @interpolate(flat) gas_state: vec4<f32>,
    // Quad coordinate (−1 … 1), centre depth along the view (m), radius (m).
    @location(5) quad: vec4<f32>,
    // Six-way sun weights: positive xyz, negative x.
    @location(6) @interpolate(flat) sun_light: vec4<f32>,
    // Negative sun yz, positive flash xy.
    @location(7) @interpolate(flat) mixed: vec4<f32>,
    // Positive flash z, negative flash xyz.
    @location(8) @interpolate(flat) flash_light: vec4<f32>,
    // World position, distance along the view (m).
    @location(9) world: vec4<f32>,
    // The shares of the sun's and the sky's light reaching the puff here (`puff_light`; 1 in the occlusion pass), how
    // far the tile's z axis (toward the lens) looks up in the world (`axes_up`), and the height of this point on the
    // puff's own plane (`sea_foot`).
    @location(10) light: vec4<f32>,
    // Simulation smoke: xyz ellipsoid centre, w vertical radius (zero for ordinary gas); a burst's soot (`GAS_FLAK`):
    // its sphere's centre, w −1.
    @location(11) @interpolate(flat) tactical: vec4<f32>,
    // Trial `flipbook` (`flipbook_varying`); zero for a still puff.
    @location(13) @interpolate(flat) flip: vec4<f32>,
    // The warp trial: the way the puff rolls in the tile's frame (shorter the more end-on) and toward the eye, and
    // its place in its cycles (`roll_phase`).
    @location(12) @interpolate(flat) roll: vec4<f32>,
    // Trial `flipbook`, a fire's loop (`flipbook_loop`): its first layer (−1 otherwise), 1 for a fire's flame; w 1 for
    // a gun's propellant, which burns in its own colours (`propellant_flame`), 2 for a fire's gas, 3 for a gun's
    // fireball (its propellant white-hot at first, `WHITE_HEAT`).
    @location(14) @interpolate(flat) flip_loop: vec4<f32>,
    // Trial `flipbook`, a puff seen from its sides (`flipbook_views`); zero for one seen from one side.
    @location(15) @interpolate(flat) flip_views: vec4<f32>,
}

fn rotate_by(c: f32, s: f32, v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}

@vertex
fn vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> Varyings {
    return place(vertex_index, instance);
}

// A puff's corner and what its fragments read.
fn place(vertex_index: u32, instance: u32) -> Varyings {
    let puff = puffs[instance];
    var center = puff.sphere.xyz;
    var radius = max(puff.sphere.w, 0.01);
    let flip = flipbook_puff(puff.extra.z, puff.evolution.x, puff.state.y, lighting.trial.y);
    // The quad frame: turned toward the camera about the world's up, as Matrix4.lookAt builds it.
    var to_camera = normalize(camera.position - center + vec3<f32>(0.0, 0.0, 1e-5));
    let across = cross(vec3<f32>(0.0, 1.0, 0.0), to_camera);
    var right = select(normalize(across), vec3<f32>(1.0, 0.0, 0.0), length(across) < 1e-4);
    var up = cross(to_camera, right);
    // Trial `flipbook`, a fire: the sprite stands on its foot (`puff.sphere.xyz`), upright, leant by `puff.quad.xyz`.
    var rise = up;
    var widen = 1.0;
    var thinned = 1.0;
    if flip.kind > 1.5 {
        // A link of a smoke column: laid along the column (`puff.quad.xyz`) about its centre.
        let link = flipbook_link(puff.quad.xyz, flip.aspect, to_camera, right, up);
        right = link.right;
        up = link.up;
        rise = link.rise;
        // Its cross-section in a fire's pall (`fire_batch.rs`' `pall_spread`): an ellipse `evolution.z` times its
        // own across the wind (level, square to the link) and `evolution.w` times it up and down. The quad is as much
        // wider as that ellipse is seen across the screen and, seen end on (as tall as its cross-section), as much
        // taller as it is seen up it; its smoke is thinner by the area it gains (`gas_state.y`), so the soot across it
        // is the same. A funnel's link (0 there) and a column's near its fire are round, and drawn as before.
        let spread = max(puff.evolution.zw, vec2<f32>(1.0));
        let level = cross(puff.quad.xyz, vec3<f32>(0.0, 1.0, 0.0));
        let crosswind = select(normalize(level), right, length(level) < 1e-3);
        let deep = cross(crosswind, puff.quad.xyz);
        let sideways = vec2<f32>(dot(right, crosswind), dot(right, deep));
        widen = select(length(spread * sideways) / length(sideways), 1.0, length(sideways) < 1e-3);
        let upward = vec2<f32>(dot(up, crosswind), dot(up, deep));
        let stretch = select(length(spread * upward) / length(upward), 1.0, length(upward) < 1e-3);
        let seen = length(puff.quad.xyz - to_camera * dot(puff.quad.xyz, to_camera));
        let tall = max(seen, flip.aspect * stretch);
        rise = up * tall;
        thinned = widen * tall / max(seen, flip.aspect);
    } else if flip.first >= 0.0 {
        let stand = flipbook_stand(center, radius, flip.anchor, puff.quad.xyz, to_camera, right, up);
        right = stand.right;
        up = stand.up;
        rise = stand.rise;
        widen = stand.widen;
        to_camera = stand.facing;
        center = stand.center;
    }
    // A seeded turn that keeps rolling slowly with age, so neighbouring puffs never repeat.
    let seed = puff.state.y;
    var turn = seed * 2.39996 + puff.state.x * ((fract(seed * 0.7137) - 0.5) * 0.36);
    if flip.anchor > 0.0 {
        // A simulation with a ground stands upright on it, leaning a little by its seed.
        turn = (fract(seed * 0.7137) - 0.5) * 0.24;
    }
    // A fire's flame, a column's billow, and a puff seen from its sides stand upright as their simulations did: what
    // turns as the eye goes round a puff seen from its sides is the side it shows.
    if flip.first >= 0.0 || abs(puff.extra.w - GAS_COLUMN) < 0.5 || flip.views.w > 0.0 {
        turn = 0.0;
    }
    let cosine = cos(turn);
    let sine = sin(turn);
    // Seen from its sides: which it shows, and how much wider than its first side's tile its quad is drawn for them.
    let views = flipbook_views(flip, to_camera, seed);
    let shown_wide = flipbook_widen(views);
    if flip.on > 0.5 {
        // The gas's own size in this frame of its simulation, its foot kept where the full-grown puff's stands.
        center -= (right * sine + up * cosine) * (radius * flip.anchor * (1.0 - flip.scale));
        radius *= flip.scale;
    }
    // The elongation axis on screen; seen end-on the puff is round.
    let flat_ = sqrt(max(1.0 - puff.evolution.w * puff.evolution.w, 0.0));
    let axis = vec3<f32>(cos(puff.evolution.z) * flat_, puff.evolution.w, sin(puff.evolution.z) * flat_);
    let projected = vec2<f32>(dot(axis, right), dot(axis, up));
    let reach = length(projected);
    let along = select(projected / max(reach, 1e-3), vec2<f32>(1.0, 0.0), reach < 1e-3);
    let squeeze = mix(1.0, max(puff.tint.w, 1.0), clamp(reach, 0.0, 1.0));
    // Sun and flash light in the tile's frame: its x and y turn with the puff, z faces the viewer.
    let sun_dir = lighting.sun_direction.xyz;
    var sun_in_tile = vec3<f32>(rotate_by(cosine, sine, vec2<f32>(dot(sun_dir, right), dot(sun_dir, up))), dot(sun_dir, to_camera));
    if flip.mirror < 0.0 {
        // A mirrored flipbook puff takes the light from its left on its right.
        sun_in_tile.x = -sun_in_tile.x;
    }
    let sun = six_way(normalize(sun_in_tile));
    var flash_positive = vec3<f32>(0.0);
    var flash_negative = vec3<f32>(0.0);
    if batch.flash_color.w > 0.0 {
        for (var i = 0; i < 4; i++) {
            let flash = lighting.flashes[i];
            let offset = flash.xyz - center;
            let power = flash.w / (dot(offset, offset) + radius * radius + 16.0);
            let d = offset + vec3<f32>(0.0, 1e-3, 0.0);
            let weights = six_way(normalize(vec3<f32>(rotate_by(cosine, sine, vec2<f32>(dot(d, right), dot(d, up))), dot(d, to_camera))));
            flash_positive += weights.positive * power;
            flash_negative += weights.negative * power;
        }
    }
    // A fire's light on its own smoke from below, in the sun's units as its flames are: lifted with them after dark.
    let glow = puff.extra.y * lighting.emission.z;
    if glow > 0.0 {
        let d = vec3<f32>(0.0, -1.0, 0.0);
        let below = six_way(normalize(vec3<f32>(rotate_by(cosine, sine, vec2<f32>(dot(d, right), dot(d, up))), dot(d, to_camera))));
        flash_positive += below.positive * glow;
        flash_negative += below.negative * glow;
    }
    // At most what a flash gives the gas beside it by day, lifted with the night as the flashes are.
    let flash_most = 2.5 * lighting.emission.w;
    flash_positive = min(flash_positive, vec3<f32>(flash_most));
    flash_negative = min(flash_negative, vec3<f32>(flash_most));
    if flip.mirror < 0.0 {
        let swapped = flash_positive.x;
        flash_positive.x = flash_negative.x;
        flash_negative.x = swapped;
    }
    let variant = floor(fract(seed * 0.61803) * GAS_VARIANTS);
    // A puff around the camera would fill the view with one flat layer: clear it as the lens enters.
    let distance = length(camera.position - center);
    let near = sstep(radius * 0.8, radius * 1.9, distance);
    // Small billows only where the puff is large enough on screen to show them.
    let pixels = camera.viewport.y * 0.5 * camera.projection[1][1] * radius / max(distance, radius);

    let corner = quad_corner(vertex_index);
    var placed = puff.quad;
    if flip.on > 0.5 {
        placed = flipbook_quad(center, radius, to_camera, distance);
    }
    var world = placed.xyz + (right * (corner.x * shown_wide) + up * corner.y) * placed.w;
    if flip.first >= 0.0 {
        world = center + (right * (corner.x * flip.aspect * widen) + rise * corner.y) * (radius * 2.0);
        // Drawn nearer the lens, at the same place and size on screen (as a puff is, `flipbook_quad`): the flame has
        // depth, and a fire in a hatch or behind a coaming is not cut off along a line by the plating its flames stand
        // over. A fire's flame (since 2026-10-20) stands at the front of its body of flame, half its width nearer,
        // and fades into what lies within that body (`FLAME_DEPTH`): a flat card met the deck and the plating about
        // it in straight cuts. A column's link, a quarter of its width.
        let front = select(0.5, FLAME_FRONT, flip.kind > 0.5 && flip.kind < 1.5);
        let pull = min(radius * flip.aspect * front, max(distance - 1.0, 0.0));
        world = camera.position + (world - camera.position) * ((distance - pull) / max(distance, 1e-3));
    }
    let view = camera.view * vec4<f32>(curved(world), 1.0);

    var out: Varyings;
    out.position = world_to_clip(world);
    out.detailed = vec4<f32>(normalize(sun_in_tile.xy + vec2<f32>(0.0, 1e-4)) * min(length(sun_in_tile.xy), 1.0),
        sstep(28.0, 120.0, pixels), fract(seed * 0.3779));
    if flip.on > 0.5 {
        // The simulation carries its own small billows: the detail map erodes its rim far less.
        out.detailed.z *= 0.3;
    }
    if flip.kind > 0.5 && flip.kind < 1.5 {
        // And a flame's edge is the simulation's alone.
        out.detailed.z = 0.0;
    }
    if flip.kind > 1.5 {
        // A column's link is many times the simulated plume's size, and its edge is a soft one where the tile thins
        // to nothing: the detail map breaks it into billows as it does a still puff's.
        out.detailed.z = sstep(28.0, 120.0, pixels);
    }
    out.frame = vec4<f32>(cosine, sine, along);
    out.place = vec4<f32>(variant - GAS_GRID * floor(variant / GAS_GRID), floor(variant / GAS_GRID), squeeze, max(radius * 0.35, 0.6));
    if flip.kind > 0.5 && flip.kind < 1.5 {
        // A fire's flame fades into the deck and plating within its body (its soft edge, `GasBatch.flags.x`).
        out.place.w = max(radius * flip.aspect * FLAME_DEPTH, 0.6);
    }
    out.look = vec4<f32>(puff.tint.rgb, puff.extra.x * near);
    out.gas_state = vec4<f32>(puff.state.z, puff.state.w / 3.0 * (1.0 - puff.evolution.y * 0.8), puff.evolution.y, puff.state.x);
    if flip.kind > 1.5 {
        // A link spread across a pall (`thinned`) is that much thinner.
        out.gas_state.y /= thinned;
    }
    out.quad = vec4<f32>(corner * 2.0, -(camera.view * vec4<f32>(curved(center), 1.0)).z, radius);
    out.sun_light = vec4<f32>(sun.positive, sun.negative.x);
    out.mixed = vec4<f32>(sun.negative.yz, flash_positive.xy);
    out.flash_light = vec4<f32>(flash_positive.z, flash_negative);
    out.world = vec4<f32>(world, -view.z);
    // The renderer's sun shadows and the smoke's own, read per corner a little inside the puff; how far the tile's z
    // axis looks up; and the corner's height on the puff's own plane, through which the card drawn nearer the lens
    // (`flipbook_quad`) is seen from the eye.
    let drawn_at = max(dot(camera.position - world, to_camera), 1e-3);
    let own = camera.position + (world - camera.position) * (dot(camera.position - center, to_camera) / drawn_at);
    out.light = vec4<f32>(puff_light(mix(center, world, 0.6), center, radius), to_camera.y, own.y);
    out.flip = flipbook_varying(flip, pixels);
    let rolls = puff.warp.yzw;
    out.roll = vec4<f32>(rotate_by(cosine, sine, vec2<f32>(dot(rolls, right), dot(rolls, up))), dot(rolls, to_camera),
        roll_phase(puff.warp.x, seed));
    out.flip_loop = flipbook_loop(flip);
    // w 1 for a gun's propellant, 2 for a fire's gas (its rolling flame bodies, `GAS_FIRE`, and what carries its glow on
    // its smoke, `extra.y`: its plumes and smoke, drawn by the flipbook or not), which glows at the fires' flames' lift,
    // 3 for a gun's fireball.
    let fire = abs(puff.extra.w - GAS_FIRE) < 0.5 || puff.extra.y > 0.0;
    out.flip_loop.w = select(select(select(0.0, 2.0, fire), 1.0, abs(puff.extra.w - GAS_PROPELLANT) < 0.5), 3.0,
        abs(puff.extra.w - GAS_FIREBALL) < 0.5);
    out.flip_views = views;
    let screen = abs(puff.extra.w - GAS_SCREEN) < 0.5;
    // A burst's soot (−1) is marched through its sphere (`flak_soot`).
    let flak = abs(puff.extra.w - GAS_FLAK) < 0.5;
    out.tactical = vec4<f32>(center, select(select(0.0, -1.0, flak), puff.evolution.w, screen));
    if flak {
        // Its soot thins as it spreads (`EffectParticle::density_now`), not again as it disperses, which greys it.
        out.gas_state.y = puff.state.w / 3.0;
    }
    if screen {
        out.look.a = 1.0;
        out.gas_state.y = puff.state.w;
    }
    if screen || flak {
        // The fragment clips its integrated path against scene depth. Keep the bounding quad in front so a hull (or an
        // aircraft in a burst) halfway through the ellipsoid cannot reject the smoke between it and the eye.
        out.position.z = out.position.w * (1.0 - 1e-6);
    }
    return out;
}

// Trial `flipbook`: a link of a fire's smoke column (`fire_batch.rs`), on a lean path of its own. A link never burns
// (its simulation carries no flame and its heat is 0, so the flame ramp gives nothing), is never turned or squeezed in
// its tile, reads its two frames without their motion, is not an occluder and is drawn with soft edges, untinted.
// What is the same over the whole quad is worked out per vertex: the sky's light (one look-up of the sky's
// environment) and the air between the eye and each corner (two of the aerial volume), interpolated across it.
struct LinkVaryings {
    @builtin(position) position: vec4<f32>,
    // Quad coordinate (−1 … 1), centre depth along the view (m), radius (m).
    @location(0) quad: vec4<f32>,
    // World position, distance along the view (m).
    @location(1) world: vec4<f32>,
    // `flipbook_varying`.
    @location(2) @interpolate(flat) flip: vec4<f32>,
    // Albedo, opacity.
    @location(3) @interpolate(flat) look: vec4<f32>,
    // Extinction multiple as it thins, dispersal, detail strength, detail seed.
    @location(4) @interpolate(flat) gas: vec4<f32>,
    // Sun in the tile's frame (xy, scaled by its reach), age, the loop's first layer.
    @location(5) @interpolate(flat) detailed: vec4<f32>,
    @location(6) @interpolate(flat) sun_light: vec4<f32>,
    @location(7) @interpolate(flat) mixed: vec4<f32>,
    @location(8) @interpolate(flat) flash_light: vec4<f32>,
    // The sky's light on the gas (× 1.7, as the puff's fragment takes it).
    @location(9) @interpolate(flat) sky: vec4<f32>,
    // The air between the eye and this point: what it lets through, and what it adds (`aerial_apply`).
    @location(10) transmittance: vec4<f32>,
    @location(11) inscatter: vec4<f32>,
    @location(12) light: vec2<f32>,
}

@vertex
fn link_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> LinkVaryings {
    let v = place(vertex_index, instance);
    var out: LinkVaryings;
    out.position = v.position;
    out.quad = v.quad;
    out.world = v.world;
    out.flip = v.flip;
    out.look = v.look;
    out.gas = vec4<f32>(v.gas_state.y, v.gas_state.z, v.detailed.z, v.detailed.w);
    out.detailed = vec4<f32>(v.detailed.xy, v.gas_state.w, v.flip_loop.x);
    out.sun_light = v.sun_light;
    out.mixed = v.mixed;
    out.flash_light = v.flash_light;
    out.sky = vec4<f32>(sky_fill() * 1.7, 0.0);
    out.light = v.light.xy;
    out.transmittance = vec4<f32>(1.0);
    out.inscatter = vec4<f32>(0.0);
    if lighting.fog.w > 0.0 {
        let air = aerial_at(v.world.xyz);
        out.transmittance = vec4<f32>(air.transmittance, 1.0);
        out.inscatter = vec4<f32>(air.inscatter, 0.0);
    }
    return out;
}

// A link or billow of a fire's column at `s` in its tile's frame (the quad's, read right to left for a mirrored one):
// the frames it reads, the gas there, its small billows and how much of the light behind it it stops. `flip` is
// `flipbook_varying`'s and `first` its loop's first layer; `gas` its extinction multiple as it thins, dispersal, detail
// strength and detail seed; `age` its age (s).
struct ColumnGas {
    frame: FlipFrame,
    body: vec4<f32>,
    small: vec4<f32>,
    coverage: f32,
}

fn column_gas(flip: vec4<f32>, first: f32, s: vec2<f32>, gas: vec4<f32>, age: f32) -> ColumnGas {
    let frame = flipbook_frame(flip, vec4<f32>(first, 0.0, 1.0, 0.0), s, vec4<f32>(0.0));
    let body = flipbook_body_at(frame);
    let strength = gas.z;
    var small = vec4<f32>(0.5, 0.5, 0.5, 0.0);
    if strength > 0.0 {
        small = textureSample(atlas_detail, detail_sampler,
            s * 0.55 + vec2<f32>(gas.w, fract(gas.w * 7.3)) + vec2<f32>(age * 0.008, age * -0.014));
    }
    // As `puff_at`: thinner as it disperses, its thin rim broken into the small billows' crowns.
    let dispersal = gas.y;
    let depth = body.b * GAS_DEPTH;
    let torn = mix(1.0, sstep(dispersal * 1.6, dispersal * 1.6 + 0.9, depth), sstep(0.0, 0.3, dispersal));
    let solid = 1.0 - exp(-depth * gas.x);
    let crowns = sstep(0.5 - small.b * 0.45, 0.8 - small.b * 0.45, solid);
    return ColumnGas(frame, body, small, solid * mix(1.0, crowns, strength * 0.85) * torn);
}

@fragment
fn link_fragment(in: LinkVaryings) -> @location(0) vec4<f32> {
    // The tile's frame is the quad's, read right to left for a mirrored link.
    let s = vec2<f32>(select(in.quad.x, -in.quad.x, in.flip.w < 0.0), in.quad.y);
    let column = column_gas(in.flip, in.detailed.w, s, in.gas, in.detailed.z);
    let body = column.body;
    let sides = flipbook_sides_at(column.frame, vec3<f32>(1.0, 0.0, 0.0));
    let strength = in.gas.z;
    let small = column.small;
    var coverage = column.coverage;
    // As `fragment`, without the flame.
    let bump = small.rg * 2.0 - 1.0;
    let height = (small.b - 0.5) * strength;
    let sun_positive = in.sun_light.xyz;
    let sun_negative = vec3<f32>(in.sun_light.w, in.mixed.xy);
    let flash_positive = vec3<f32>(in.mixed.zw, in.flash_light.x);
    let flash_negative = in.flash_light.yzw;
    let relief = 1.0 + (dot(bump, in.detailed.xy) * 0.8 + height * 0.8) * strength;
    let direct = (sides.r * sun_positive.x + sides.g * sun_negative.x + sides.b * sun_positive.y + sides.a * sun_negative.y
        + body.r * sun_positive.z * 0.85 + body.g * sun_negative.z * (2.0 * 1.6)) * relief;
    let flash_lit = sides.r * flash_positive.x + sides.g * flash_negative.x + sides.b * flash_positive.y + sides.a * flash_negative.y
        + body.r * flash_positive.z + body.g * flash_negative.z * 2.0;
    let sky = (sides.b * 0.5 + (sides.r + sides.g + body.r + body.g * 2.0) * 0.11 + sides.a * 0.06) * (1.0 + height * 0.7);
    let radiance = in.look.rgb * (lighting.direct.rgb * (direct * in.light.x) + in.sky.rgb * (sky * in.light.y) + flash_light_color() * flash_lit);
    let bulge = sqrt(max(1.0 - dot(s, s), 0.0)) * in.quad.w;
    coverage *= smoke_visibility(in.position.xy, in.quad.z - bulge, in.quad.w * 0.5, false);
    let alpha = coverage * in.look.a * soft_edge(in.position.xy, in.world.w, max(in.quad.w * 0.35, 0.6));
    return vec4<f32>(radiance * in.transmittance.rgb + in.inscatter.rgb, alpha);
}

// This fragment in the tile's frame: squeezed across the elongation axis, then turned into the tile.
fn in_tile_frame(in: Varyings) -> vec2<f32> {
    // A quad drawn wider for the sides it shows spans that much more of the tile across.
    let q = vec2<f32>(in.quad.x * flipbook_widen(in.flip_views), in.quad.y);
    let axis = in.frame.zw;
    let squeezed = in.place.z;
    let on_axis = dot(q, axis);
    let off = (q.x * -axis.y + q.y * axis.x) * squeezed;
    let turned = rotate_by(in.frame.x, in.frame.y, vec2<f32>(axis.x * on_axis - axis.y * off, axis.y * on_axis + axis.x * off));
    // A mirrored flipbook puff reads its tile right to left.
    return vec2<f32>(select(turned.x, -turned.x, in.flip.w < 0.0), turned.y);
}

struct Puff {
    coord: vec2<f32>,
    body: vec4<f32>,
    small: vec4<f32>,
    strength: f32,
    solid: f32,
    coverage: f32,
    frame: FlipFrame,
    // The warp trial: the second layer's place in the atlas and its share (`coord` is the first's).
    other: vec2<f32>,
    blend: f32,
    // Trial `flipbook`: each side's share of the light (`FlipGas.light`).
    lights: vec3<f32>,
}

// The warp trial: where a layer reads the puff's tile for `s` in the tile's frame, held inside the tile so a pulled
// lookup never reads a neighbour's.
fn rolled_coord(in: Varyings, s: vec2<f32>) -> vec2<f32> {
    let held = clamp(s, vec2<f32>(-0.99), vec2<f32>(0.99));
    return (in.place.xy + (held * 0.5 + 0.5)) / GAS_GRID;
}

// How much of the light behind the puff it stops at `s` in the tile's frame, and the samples its light reads.
fn puff_at(in: Varyings, s: vec2<f32>) -> Puff {
    var inside = 1.0 - sstep(0.96, 1.0, length(s));
    var coord = (in.place.xy + (s * 0.5 + 0.5)) / GAS_GRID;
    var other = coord;
    var blend = 0.0;
    var body: vec4<f32>;
    var small: vec4<f32>;
    // Small billows in the tile's frame, a seeded patch of the tiling map that rolls slowly as the puff ages.
    let strength = in.detailed.z;
    let age = in.gas_state.w;
    if lighting.trial.x > 0.5 {
        // The warp trial: two layers of the puff's own tile riding its flow a half cycle apart (the puff keeps its
        // shape and its light; showing another tile each cycle was tried and read as one shape melting into another);
        // the small billows stream along the same flow in shorter cycles of their own, each on a fresh patch of their
        // tiling map.
        let rolling = roll_flow(s, in.roll.xy, in.roll.z, in.detailed.w);
        // A puff squeezed across its axis shows the tile's shear that much steeper: it is pulled that much less.
        let flow = rolling.flow / in.place.z;
        let roll = roll_layers(s, flow, in.roll.w + rolling.stagger.x);
        let fine = roll_layers(s, flow, roll_detail_phase(in.roll.w) + rolling.stagger.y);
        coord = rolled_coord(in, roll.first);
        other = rolled_coord(in, roll.second);
        let first = textureSample(atlas_body, atlas_sampler, coord);
        let second = textureSample(atlas_body, atlas_sampler, other);
        // The layers' optical depths add by their shares. What the gas sends the eye (its light, its heat) is baked
        // per unit of what it stops, so those mix by what each layer stops: a layer empty here does not dim the
        // other's gas, and one layer's thin cool rim does not cut a band of soot through the other's burning core.
        let stops = (vec2<f32>(1.0) - exp(vec2<f32>(first.b, second.b) * (-GAS_DEPTH * in.gas_state.y)))
            * vec2<f32>(1.0 - roll.blend, roll.blend) + vec2<f32>(1e-5, 0.0);
        blend = stops.y / (stops.x + stops.y);
        body = vec4<f32>(mix(first.rg, second.rg, blend), mix(first.b, second.b, roll.blend), mix(first.a, second.a, blend));
        // The small billows: two patches of their map mixed, and the mix lifted back to one patch's contrast.
        let drift = vec2<f32>(in.detailed.w, fract(in.detailed.w * 7.3)) + vec2<f32>(age * 0.008, age * -0.014);
        small = mix(textureSample(atlas_detail, detail_sampler, fine.first * 0.55 + drift + fine.cycles.x * vec2<f32>(0.37, 0.61)),
            textureSample(atlas_detail, detail_sampler, fine.second * 0.55 + drift + fine.cycles.y * vec2<f32>(0.37, 0.61) + 0.5), fine.blend);
        small = clamp((small - 0.5) * inverseSqrt(1.0 - 2.0 * fine.blend * (1.0 - fine.blend)) + 0.5, vec4<f32>(0.0), vec4<f32>(1.0));
    } else {
        body = textureSample(atlas_body, atlas_sampler, coord);
        small = textureSample(atlas_detail, detail_sampler,
            s * 0.55 + vec2<f32>(in.detailed.w, fract(in.detailed.w * 7.3)) + vec2<f32>(age * 0.008, age * -0.014));
    }
    // Trial `flipbook`: the simulation's frames for the still puff, over the whole square tile, from its sides.
    let frame = flipbook_frame(in.flip, in.flip_loop, s, in.flip_views);
    var lights = vec3<f32>(1.0, 0.0, 0.0);
    if frame.on {
        let gas = flipbook_gas(frame);
        body = gas.body;
        lights = gas.light;
        inside = 1.0;
        if !(strength > 0.0) {
            small = vec4<f32>(0.5, 0.5, 0.5, 0.0);
        }
    }
    // Thinner as it disperses: optical depth falls everywhere, and the thin rim tears away first.
    let thickness = in.gas_state.y;
    let dispersal = in.gas_state.z;
    let depth = body.b * GAS_DEPTH;
    let torn = mix(1.0, sstep(dispersal * 1.6, dispersal * 1.6 + 0.9, depth), sstep(0.0, 0.3, dispersal));
    let solid = 1.0 - exp(-depth * thickness);
    // The thin rim breaks into the small billows: where the gas is thin, only their crowns remain.
    let crowns = sstep(0.5 - small.b * 0.45, 0.8 - small.b * 0.45, solid);
    let coverage = solid * mix(1.0, crowns, strength * 0.85) * torn * inside;
    return Puff(coord, body, small, strength, solid, coverage, frame, other, blend, lights);
}

// How far the puff's near surface bulges toward the camera from its centre at `s`, as a ball's would.
fn bulge_at(in: Varyings, s: vec2<f32>) -> f32 {
    return sqrt(max(1.0 - dot(s, s), 0.0)) * in.quad.w;
}

struct TacticalSmoke {
    opacity: f32,
    front: f32,
    middle: f32,
    // Opacity-weighted sun and sky reaching the billows the eye sees.
    light: vec2<f32>,
}

// The simulation owns the screen's bounds, drift, age and bulk density. The renderer rolls billows within those
// bounds: a uniform ellipsoid alone looks like a solid, pale capsule, especially where several screens overlap.
// The existing fire-noise volume supplies three scales of billows, carried with this puff and slowly rising through
// it. Coordinates relative to the puff keep the pattern continuous across theatre recenters and camera changes.
fn screen_billow(coord: vec3<f32>) -> f32 {
    return dot(textureSampleLevel(screen_noise, screen_sampler, coord, 0.0).rgb, vec3<f32>(0.58, 0.28, 0.14));
}

fn screen_density(local: vec3<f32>, radii: vec3<f32>, age: f32, seed: f32) -> f32 {
    let noise = textureSampleLevel(screen_noise, screen_sampler, screen_coord(local, radii, age, seed), 0.0).rgb;
    let billow = dot(noise, vec3<f32>(0.58, 0.28, 0.14));
    let inside = 1.0 - length(local);
    // Feather the authoritative rim, carving folds into it while keeping the heart filled with smoke.
    let shape = inside - 0.1 + (billow - 0.35) * 0.4;
    return sstep(0.0, 0.12, shape) * sstep(0.0, 0.06, inside) * (0.8 + noise.g * 0.8);
}

// Ray distances are world metres (the normalized ray is divided by the ellipsoid radii); opaque scene depth clips
// the path so the screen also obscures a camera inside it without drawing smoke through a hull in front. Both the
// scene and its smoke-occlusion map integrate the same billows with the simulation's bulk extinction per metre.
fn tactical_smoke(in: Varyings) -> TacticalSmoke {
    var out: TacticalSmoke;
    out.opacity = 0.0;
    out.front = 0.0;
    out.middle = 0.0;
    out.light = vec2<f32>(0.0);
    let ray = normalize(curved(in.world.xyz) - camera.position);
    let radii = vec3<f32>(in.quad.w, in.tactical.w, in.quad.w);
    let origin = (camera.position - curved(in.tactical.xyz)) / radii;
    let direction = ray / radii;
    let a = dot(direction, direction);
    let b = dot(origin, direction);
    let c = dot(origin, origin) - 1.0;
    let discriminant = b * b - a * c;
    if discriminant <= 0.0 { return out; }
    let root = sqrt(discriminant);
    let start = max(0.0, (-b - root) / a);
    let view_scale = max(-(camera.view * vec4<f32>(ray, 0.0)).z, 1e-4);
    var end = (-b + root) / a;
#ifdef EFFECT_SKY
    // The occlusion preparation pass has no scene-depth binding; only the scene draw clips to opaque geometry.
    let pixel = vec2<i32>(clamp(in.position.xy / camera.viewport.xy * vec2<f32>(textureDimensions(scene_depth)),
        vec2<f32>(0.0), vec2<f32>(textureDimensions(scene_depth)) - 1.0));
    end = min(end, view_depth(textureLoad(scene_depth, pixel, 0)) / view_scale);
#endif
    if end <= start { return out; }
    let sigma = 0.025 * in.gas_state.y;
    let stride = (end - start) / 12.0;
    let sun = normalize(lighting.sun_direction.xyz / radii);
    let sun_scale = length(sun * radii);
    var through = 1.0;
    var middle = 0.0;
    var light = vec2<f32>(0.0);
    out.front = end * view_scale;
    for (var i = 0u; i < 12u; i++) {
        let distance = start + (f32(i) + 0.5) * stride;
        let local = origin + direction * distance;
        let density = screen_density(local, radii, in.gas_state.w, in.detailed.w);
        let weight = through * (1.0 - exp(-density * sigma * stride));
        if weight > 1e-4 {
            out.front = min(out.front, distance * view_scale);
        }
        middle += distance * weight;
#ifdef EFFECT_SKY
        // Approximate the light's remaining path through the ellipsoid. Crowns catch the sun; deeper folds and
        // lower billows receive less sun and sky. A diffuse floor keeps multiple scattering in dense white smoke.
        let along = dot(local, sun);
        let sun_path = (sqrt(max(along * along + 1.0 - dot(local, local), 0.0)) - along) * sun_scale;
        let sky_path = max(1.0 - local.y, 0.0) * radii.y;
        light += (vec2<f32>(0.25) + 0.75 * exp(-vec2<f32>(sun_path * 0.65, sky_path * 0.35) * sigma * density)) * weight;
#endif
        through -= weight;
    }
    out.opacity = 1.0 - through;
    out.middle = middle / max(out.opacity, 1e-4) * view_scale;
    out.light = light / max(out.opacity, 1e-4);
#ifdef EFFECT_SKY
    if out.opacity < 1e-5 { return out; }
    // The opacity-weighted visible billow's shape normal gives its crowns and folds local relief even once the
    // heart is opaque. The first occupied march step jumps between samples across the image; the weighted position
    // varies smoothly. Read its noise gradient once per fragment instead of tracing more light rays at every step.
    let local = origin + direction * (out.middle / view_scale);
    let coord = screen_coord(local, radii, in.gas_state.w, in.detailed.w);
    let step = 0.04;
    let base = screen_billow(coord);
    let gradient = vec3<f32>(
        screen_billow(coord + vec3<f32>(step, 0.0, 0.0)),
        screen_billow(coord + vec3<f32>(0.0, step, 0.0)),
        screen_billow(coord + vec3<f32>(0.0, 0.0, step))) - base;
    let period = clamp(radii.x * 1.5, 64.0, 240.0);
    let normal = normalize(local / radii - gradient * (0.4 / (step * period)));
    out.light *= vec2<f32>(0.35 + 0.9 * max(dot(normal, lighting.sun_direction.xyz), 0.0),
        0.65 + 0.65 * max(normal.y, 0.0));
#endif
    return out;
}

// A heavy AA round's burst (`GAS_FLAK`, `flak.rs`): its soot is one cloud marched through the puff's sphere, its shape
// drawn from its seed and its age, so no two bursts are alike from any side. As the war's photographs of barrages show
// it: black from the first frames, a body with lobes heaped on it and billowed over, crisp at its edge, some bursts
// with a streamer or two torn off and some tailing off one way, a comet or a "Y"; it greys and thins from its edge as
// the air mixes into it.
// The cloud's share of its sphere's radius (`flak.rs`' `SPHERE`): its lobes, streamers and billows reach into the rest.
const FLAK_CLOUD: f32 = 0.7;
// The optical depth through a fresh cloud's middle at `EffectParticle::density` 3. It falls as the square of the cloud's
// width (`EffectParticle::flak`), and as the puff fades: black through all but its rim, dark still half a minute on.
const FLAK_DEPTH: f32 = 24.0;
// Steps through the sphere, for a puff a few pixels across and for one a hundred and more (`Varyings.detailed.z`).
const FLAK_STEPS: vec2<f32> = vec2<f32>(8.0, 24.0);
// The share of the bursts that tail off one way (`flak_shape`), and of those the share forked.
const FLAK_TAILED: f32 = 0.3;
const FLAK_FORKED: f32 = 0.35;

// A burst's cloud in its own frame (world axes, the cloud's radius 1) at its age: its body and the lobes heaped on it
// (centre, radius); its streamers (way × length, radius at the root; 0, none); where its billows lie in the noise
// volume; how deep its soft rim is and how high its billows stand (cloud radii); and how far its streamers have thinned
// into the air (0–1).
struct FlakShape {
    body: vec4<f32>,
    lobe_a: vec4<f32>,
    lobe_b: vec4<f32>,
    lobe_c: vec4<f32>,
    lobe_d: vec4<f32>,
    streamer_a: vec4<f32>,
    streamer_b: vec4<f32>,
    streamer_c: vec4<f32>,
    billows: vec3<f32>,
    rim: f32,
    relief: f32,
    thinned: f32,
}

// An integer hash to 0–1 (PCG's output step): a burst's draws from its seed.
fn flak_hash(key: u32) -> f32 {
    var x = key * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x) * (1.0 / 4294967296.0);
}

// A way drawn evenly over the sphere.
fn flak_way(key: u32) -> vec3<f32> {
    let y = flak_hash(key) * 2.0 - 1.0;
    let yaw = flak_hash(key + 1u) * 6.2831853;
    let level = sqrt(max(1.0 - y * y, 0.0));
    return vec3<f32>(cos(yaw) * level, y, sin(yaw) * level);
}

// A lobe heaped on the body, the more of them the way the burst leans: centre and radius.
fn flak_lobe(key: u32, lean: vec3<f32>, spread: f32) -> vec4<f32> {
    let way = normalize(flak_way(key) + lean + vec3<f32>(0.0, 1e-4, 0.0));
    return vec4<f32>(way * ((0.38 + flak_hash(key + 2u) * 0.24) * spread), 0.3 + flak_hash(key + 3u) * 0.2);
}

// The `index`th streamer of `count`: its way × its length, shot out over the first tenth of a second, and its radius
// at its root; none past the count.
fn flak_streamer(key: u32, count: f32, index: f32, age: f32) -> vec4<f32> {
    if index >= count {
        return vec4<f32>(0.0);
    }
    let reach = (1.05 + flak_hash(key + 2u) * 0.3) * (1.0 - exp(-age / 0.04));
    return vec4<f32>(flak_way(key) * reach, 0.1 + flak_hash(key + 3u) * 0.07);
}

fn flak_shape(seed: f32, age: f32, dispersal: f32) -> FlakShape {
    let key = u32(seed * 16777216.0);
    var shape: FlakShape;
    // The lobes drift apart a little as the air mixes into the cloud.
    let spread = 1.0 + 0.15 * dispersal;
    let form = flak_hash(key + 8u);
    if form < FLAK_TAILED {
        // A tailed burst, its gas carried on one way: a head, and lobes in a tapering chain behind it, ending in a
        // thread; a forked one's chain parts in two arms, each with its thread.
        let axis = flak_way(key + 9u);
        let side = flak_way(key + 10u);
        let across = normalize(side - axis * dot(side, axis) + vec3<f32>(1e-3, 0.0, 0.0));
        let forked = form < FLAK_TAILED * FLAK_FORKED;
        let fork = select(0.0, 0.5, forked);
        let arm_a = normalize(axis + across * fork);
        let arm_b = normalize(axis - across * fork);
        let shot = 1.0 - exp(-age / 0.04);
        shape.body = vec4<f32>(axis * 0.35, 0.52 + flak_hash(key + 3u) * 0.08);
        shape.lobe_a = vec4<f32>((axis * 0.12 + across * 0.3) * spread, 0.38 + flak_hash(key + 11u) * 0.08);
        shape.lobe_b = vec4<f32>(arm_a * (-0.38 * spread), 0.34);
        shape.lobe_c = vec4<f32>(arm_b * (select(-0.78, -0.42, forked) * spread), select(0.26, 0.3, forked));
        shape.lobe_d = vec4<f32>(arm_a * (select(-1.08, -0.82, forked) * spread), select(0.18, 0.22, forked));
        shape.streamer_a = vec4<f32>(arm_a * (-1.36 * shot), 0.2);
        shape.streamer_b = select(vec4<f32>(0.0), vec4<f32>(arm_b * (-1.3 * shot), 0.18), forked);
        shape.streamer_c = vec4<f32>(0.0);
    } else {
        // Some heaped bursts are heavier on one side: their lobes lean that way, the body a little the other.
        let lean = flak_way(key) * (flak_hash(key + 2u) * 0.9);
        shape.body = vec4<f32>(lean * -0.1, 0.62 + flak_hash(key + 3u) * 0.1);
        shape.lobe_a = flak_lobe(key + 16u, lean, spread);
        shape.lobe_b = flak_lobe(key + 32u, lean, spread);
        shape.lobe_c = flak_lobe(key + 48u, lean, spread);
        shape.lobe_d = flak_lobe(key + 64u, lean * -0.5, spread);
        // No streamer for a third of them, one, two or for one in ten three for the rest.
        let draw = flak_hash(key + 4u);
        let count = f32(draw > 0.35) + f32(draw > 0.65) + f32(draw > 0.9);
        shape.streamer_a = flak_streamer(key + 80u, count, 0.0, age);
        shape.streamer_b = flak_streamer(key + 96u, count, 1.0, age);
        shape.streamer_c = flak_streamer(key + 112u, count, 2.0, age);
    }
    // The billows churn while the burst's gas slows, then barely move.
    let churn = vec3<f32>(0.11, -0.07, 0.13) * (1.0 - exp(-age / 2.5)) + vec3<f32>(0.0011, 0.0017, -0.0013) * age;
    shape.billows = vec3<f32>(flak_hash(key + 5u), flak_hash(key + 6u), flak_hash(key + 7u)) * 8.0 + churn;
    shape.rim = 0.06 + 0.4 * dispersal;
    shape.relief = 1.0 + 0.7 * dispersal;
    shape.thinned = sstep(2.0, 30.0, age);
    return shape;
}

// The union of two distances, rounded over `k`.
fn flak_union(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    return min(a, b) - h * h * k * 0.25;
}

// How far `q` lies outside the body and its lobes (cloud radii; negative inside), before the billows.
fn flak_body(q: vec3<f32>, shape: FlakShape) -> f32 {
    var d = length(q - shape.body.xyz) - shape.body.w;
    d = flak_union(d, length(q - shape.lobe_a.xyz) - shape.lobe_a.w, 0.3);
    d = flak_union(d, length(q - shape.lobe_b.xyz) - shape.lobe_b.w, 0.3);
    d = flak_union(d, length(q - shape.lobe_c.xyz) - shape.lobe_c.w, 0.3);
    return flak_union(d, length(q - shape.lobe_d.xyz) - shape.lobe_d.w, 0.3);
}

// How far `q` lies outside a streamer: a cone from the middle, tapering to a thread at its tip.
fn flak_thread(q: vec3<f32>, streamer: vec4<f32>) -> f32 {
    let reach = dot(streamer.xyz, streamer.xyz);
    if streamer.w <= 0.0 || reach < 1e-6 {
        return 10.0;
    }
    let along = clamp(dot(q, streamer.xyz) / reach, 0.0, 1.0);
    return length(q - streamer.xyz * along) - streamer.w * (1.0 - 0.8 * along);
}

// A billow of the noise volume's largest Worley cells, rounded: 1 at a cell's middle, folding in to its borders.
fn flak_billow(coord: vec3<f32>) -> f32 {
    let d = 1.0 - textureSampleLevel(screen_noise, screen_sampler, coord, 0.0).r;
    return 1.0 - d * d;
}

// How far the billows heave the surface out at `q` (cloud radii): three sizes of them, a quarter, a ninth and a
// twentieth of the cloud's width across.
fn flak_relief(q: vec3<f32>, shape: FlakShape) -> f32 {
    let o = shape.billows;
    return ((flak_billow(q * 0.5 + o) - 0.62) * 0.3 + (flak_billow(q.zxy * 1.1 + o.yzx) - 0.62) * 0.15
        + (flak_billow(q.yzx * 2.4 + o.zxy) - 0.62) * 0.07) * shape.relief;
}

// The billowed body's distance (cloud radii; negative inside): its surface.
fn flak_surface(q: vec3<f32>, shape: FlakShape) -> f32 {
    return flak_body(q, shape) - flak_relief(q, shape);
}

// The soot's density at `q` (0–1), and how deep inside the billowed body it lies (cloud radii, for its afterburning).
fn flak_density(q: vec3<f32>, shape: FlakShape) -> vec2<f32> {
    let body = flak_body(q, shape);
    let thread = min(min(flak_thread(q, shape.streamer_a), flak_thread(q, shape.streamer_b)), flak_thread(q, shape.streamer_c));
    // Beyond the reach of the highest billow: nothing, without reading them.
    if min(body, thread) > 0.4 * shape.relief + shape.rim {
        return vec2<f32>(0.0, -1.0);
    }
    let relief = flak_relief(q, shape);
    let inside = relief - body;
    var density = sstep(0.0, shape.rim, inside);
    // A streamer billows less than the body, and thins into the air within half a minute.
    let streamer = sstep(0.0, shape.rim * 0.7 + 0.04, relief * 0.5 - thread) * (1.0 - shape.thinned) * 0.85;
    density = max(density, streamer);
    return vec2<f32>(density, inside);
}

// The flame of the filling burning on inside the soot at `heat` (0–1): dull red, then orange where it is thick and hot,
// as an explosive's fireball burns (`gas_shade`).
fn flak_flame(heat: f32) -> vec3<f32> {
    let ember = mix(vec3<f32>(0.45, 0.03, 0.004), vec3<f32>(2.4, 0.45, 0.04), sstep(0.12, 0.55, heat));
    return mix(ember, vec3<f32>(4.2, 1.55, 0.3), sstep(0.55, 1.0, heat)) * (heat * heat);
}

// What a burst's soot shows at this fragment: its opacity, its light (straight, not premultiplied), the view depths (m)
// of its nearest gas and of its middle, weighed by what each step shows, and of its sphere's near side. `lit`: work its
// light out too (the occlusion map needs only how much it hides and where).
struct FlakSoot {
    opacity: f32,
    radiance: vec3<f32>,
    front: f32,
    middle: f32,
    entry: f32,
}

fn flak_soot(in: Varyings, lit: bool) -> FlakSoot {
    var out = FlakSoot(0.0, vec3<f32>(0.0), 0.0, 0.0, 0.0);
    // The ray through the puff's sphere, in its radii from its centre.
    let radius = in.quad.w;
    let ray = normalize(curved(in.world.xyz) - camera.position);
    let origin = (camera.position - curved(in.tactical.xyz)) / radius;
    let b = dot(origin, ray);
    let discriminant = b * b - dot(origin, origin) + 1.0;
    if discriminant <= 0.0 {
        return out;
    }
    let root = sqrt(discriminant);
    let start = max(0.0, -b - root);
    var end = -b + root;
    let view_scale = max(-(camera.view * vec4<f32>(ray, 0.0)).z, 1e-4);
#ifdef EFFECT_SKY
    // Cut off by what stands in front of its far side (the occlusion pass has no scene depth).
    let pixel = vec2<i32>(clamp(in.position.xy / camera.viewport.xy * vec2<f32>(textureDimensions(scene_depth)),
        vec2<f32>(0.0), vec2<f32>(textureDimensions(scene_depth)) - 1.0));
    end = min(end, view_depth(textureLoad(scene_depth, pixel, 0)) / (view_scale * radius));
#endif
    if end <= start {
        return out;
    }
    let age = in.gas_state.w;
    let dispersal = in.gas_state.z;
    let shape = flak_shape(in.detailed.w, age, dispersal);
    // It swells to its size within a few hundredths of a second, its soot the denser the smaller it is.
    let swell = 1.0 - 0.6 * exp(-age / 0.05);
    let to_cloud = 1.0 / (FLAK_CLOUD * swell);
    let heat = in.gas_state.x;
    // Optical depth per sphere radius through soot of density 1: the fresh cloud's `FLAK_DEPTH` across its middle (two
    // cloud radii), less as it spreads (`gas_state.y`) and as it fades (`look.a`). While the filling burns its soot is
    // still forming, and the eye sees deep into the fireball: a film's first frame of a burst is its flame.
    let extinction = FLAK_DEPTH * in.gas_state.y * in.look.a * to_cloud * 0.5 / (swell * swell) * (1.0 - 0.7 * heat * heat);
    let steps = mix(FLAK_STEPS.x, FLAK_STEPS.y, in.detailed.z);
    let stride = (end - start) / steps;
    // Each pixel starts its steps at its own place within the first, so their layers do not band.
    let jitter = fract(52.9829189 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));
    var through = 1.0;
    var front = end;
    var middle = 0.0;
    var place = vec3<f32>(0.0);
    var glow = vec3<f32>(0.0);
    for (var i = 0u; i < u32(steps); i++) {
        let t = start + (f32(i) + jitter) * stride;
        let u = origin + ray * t;
        let soot = flak_density(u * to_cloud, shape);
        if soot.x <= 0.0 {
            continue;
        }
        let weight = through * (1.0 - exp(-extinction * soot.x * stride));
        front = min(front, t);
        middle += t * weight;
        place += u * weight;
        if heat > 0.0 {
            // The filling burns on inside the soot for a fifth of a second, its flame withdrawing inward as it goes out.
            let withdrawn = 0.05 + 0.6 * (1.0 - heat);
            glow += flak_flame(heat * sstep(withdrawn, withdrawn + 0.25, soot.y)) * weight;
        }
        through -= weight;
        if through < 0.004 {
            break;
        }
    }
    out.opacity = 1.0 - through;
    if out.opacity < 1e-4 {
        return out;
    }
    out.front = front * radius * view_scale;
    out.entry = start * radius * view_scale;
    out.middle = middle / out.opacity * radius * view_scale;
    if !lit {
        return out;
    }
    // Its light where the eye sees into it (the steps' weighed place): the billowed surface's way there, held a little
    // toward the way out from its middle, lit by the sun (less where its lobes stand between) and by the sky above it.
    let point = place / out.opacity;
    let q = point * to_cloud;
    let k = vec2<f32>(1.0, -1.0);
    let e = 0.06;
    let gradient = k.xyy * flak_surface(q + k.xyy * e, shape) + k.yyx * flak_surface(q + k.yyx * e, shape)
        + k.yxy * flak_surface(q + k.yxy * e, shape) + k.xxx * flak_surface(q + k.xxx * e, shape);
    let normal = normalize(normalize(gradient + vec3<f32>(0.0, 1e-6, 0.0)) * 0.8 + normalize(q + vec3<f32>(0.0, 1e-6, 0.0)) * 0.4);
    let sun = lighting.sun_direction.xyz;
    let shaded = sstep(0.0, 0.35, -flak_body(q + sun * 0.5, shape));
    let sunlit = clamp(dot(normal, sun) * 0.85 + 0.15, 0.0, 1.0) * (1.0 - 0.7 * shaded);
    // The flashes nearby, its own among them: a burst at night lights its soot from within for a moment.
    var flashed = 0.0;
    if batch.flash_color.w > 0.0 {
        let world = in.tactical.xyz + point * radius;
        for (var f = 0; f < 4; f++) {
            let flash = lighting.flashes[f];
            let offset = flash.xyz - world;
            let way = offset * inverseSqrt(max(dot(offset, offset), 1e-6));
            flashed += flash.w / (dot(offset, offset) + radius * radius + 16.0) * (0.35 + 0.65 * max(dot(normal, way), 0.0));
        }
        flashed = min(flashed, 2.5 * lighting.emission.w);
    }
    // Soot reflects a few hundredths of the light; it greys as the air and its vapour mix into it.
    let albedo = in.look.rgb * (1.0 + 3.0 * dispersal);
    out.radiance = albedo * (lighting.direct.rgb * (sunlit * in.light.x)
        + sky_fill() * (sky_share(normal.y) * SKY_LOBE * 1.7 * in.light.y) + flash_light_color() * flashed)
        + emitted_at(glow / out.opacity, lighting.flash_glow.x);
    return out;
}

@fragment
fn fragment(in: Varyings) -> @location(0) vec4<f32> {
    return gas_shade(in, false).colour;
}

// Look `merge`: the puff into the merged smoke (`merged_smoke`).
@fragment
fn merged(in: Varyings) -> MergedSmoke {
    let shade = gas_shade(in, true);
    if shade.colour.a <= 0.0 {
        discard;
    }
    // A burst's soot is weighed by the map's levels only: the map's optical depth lies about one middle for all its gas,
    // and thin gas far nearer (a ship's funnel haze over the guns) draws it in front of a distant burst, which then lay
    // behind its own soot and took the haze's colour.
    return merged_smoke(shade.colour, in.position.xy, shade.front, in.quad.w * 0.5, in.tactical.w < 0.0);
}

// A puff's fragment as the layered draw blends it (its light and its opacity), and its near surface's distance along the
// view (m). `merged` (look `merge`): drawn into the merged smoke, which weighs it by the gas in front (`merged_smoke`)
// where the layered draw thins it by that gas.
struct GasShade {
    colour: vec4<f32>,
    front: f32,
}

fn gas_shade(in: Varyings, merged: bool) -> GasShade {
    if in.tactical.w > 0.0 {
        let smoke = tactical_smoke(in);
        let radiance = in.look.rgb * (lighting.direct.rgb * (0.2 * smoke.light.x) + sky_fill() * (0.8 * smoke.light.y));
        let opacity = smoke.opacity * splashes_through(in.position.xy, smoke.front);
        return GasShade(vec4<f32>(effect_fog(radiance, in.tactical.xyz), opacity), smoke.front);
    }
    if in.tactical.w < 0.0 {
        // A burst's soot: its fade is in its optical depth (`flak_soot`), not laid over its opacity again.
        let soot = flak_soot(in, true);
        // Seen from its sphere's near side, in front of its soot's place in the occlusion map wherever the map's quarter
        // resolution and its own steps put that: from its soot's own front, the rim of a burst hid behind its middle.
        let alpha = seen_through(in, soot.opacity, soot.entry, merged);
        return GasShade(vec4<f32>(effect_fog(soot.radiance, in.tactical.xyz), alpha), soot.entry);
    }
    let s = in_tile_frame(in);
    let puff = puff_at(in, s);
    // A still puff's light, read before any fragment leaves (the read takes its footprint from its neighbours').
    var sides = vec4<f32>(0.0);
    if !puff.frame.on {
        sides = textureSample(atlas_sides, atlas_sampler, puff.coord);
        if lighting.trial.x > 0.5 {
            sides = mix(sides, textureSample(atlas_sides, atlas_sampler, puff.other), puff.blend);
        }
    }
    // Where the puff covers nothing it adds nothing (its alpha is its coverage, blended over the scene): its light is not
    // worked out there, in the corners of its quad and the margin drawn for a side seen wider (`flipbook_widen`).
    if puff.coverage <= 0.0 {
        return GasShade(vec4<f32>(0.0), in.quad.z);
    }
    if puff.frame.on {
        sides = flipbook_sides_at(puff.frame, puff.lights);
    }
    let body = puff.body;
    let bump = puff.small.rg * 2.0 - 1.0;
    let height = (puff.small.b - 0.5) * puff.strength;
    let heat_scale = in.gas_state.x;
    let sun_positive = in.sun_light.xyz;
    let sun_negative = vec3<f32>(in.sun_light.w, in.mixed.xy);
    let flash_positive = vec3<f32>(in.mixed.zw, in.flash_light.x);
    let flash_negative = in.flash_light.yzw;
    // Faces toward the light catch more of it, and the folds between them hold shadow and less sky.
    let relief = 1.0 + (dot(bump, in.detailed.xy) * 0.8 + height * 0.8) * puff.strength;
    let direct = (sides.r * sun_positive.x + sides.g * sun_negative.x + sides.b * sun_positive.y + sides.a * sun_negative.y
        + body.r * sun_positive.z * 0.85 + body.g * sun_negative.z * (2.0 * 1.6)) * relief;
    let flash_lit = sides.r * flash_positive.x + sides.g * flash_negative.x + sides.b * flash_positive.y + sides.a * flash_negative.y
        + body.r * flash_positive.z + body.g * flash_negative.z * 2.0;
    // The sky's light from each baked direction as a face looking that way sees it: the side toward the eye takes the
    // sky behind the eye and the back the sky beyond the gas (`sky_toward`: under a low sun the horizon's glow), the
    // other four sides the sky overhead by the hemisphere's rule (`sky_share`). Light from behind counts twice, as the
    // sun's and the flashes' do.
    let looks_up = axes_up(in);
    let to_eye = camera.position - in.world.xyz;
    let level = normalize(to_eye.xz + vec2<f32>(1e-5, 0.0)) * sqrt(max(1.0 - looks_up.z * looks_up.z, 0.0));
    let facing = vec3<f32>(level.x, looks_up.z, level.y);
    let overhead = sky_fill();
    let sky = (overhead * (sides.r * sky_share(looks_up.x) + sides.g * sky_share(-looks_up.x) + sides.b * sky_share(looks_up.y)
        + sides.a * sky_share(-looks_up.y)) + sky_toward(facing) * body.r + sky_toward(-facing) * (body.g * 2.0))
        * (SKY_LOBE * 1.7 * (1.0 + height * 0.7));
    // Burning gas is dark soot lit by its own fire; as it cools only gas hotter than a rising threshold burns.
    let temperature = body.a + height * 0.5;
    let threshold = clamp(1.0 - heat_scale * 0.38, 0.1, 0.96);
    var burning = sstep(threshold - 0.05, threshold + 0.2, temperature);
    // White where the burning gas is thick and hot, yellow, then orange where it thins at the rim.
    var heat = burning * min(heat_scale, 1.7) * (puff.solid * 0.45 + temperature * 0.5 + 0.25);
    if in.flip_loop.y > 0.5 {
        // Trial `flipbook`, a fire's flame: its heat is the simulation's temperature all the way down.
        let flame_heat = flipbook_flame(body.a, heat_scale);
        burning = flame_heat.x;
        heat = flame_heat.y;
    }
    let fireball = in.flip_loop.w > 2.5;
    let propellant = (in.flip_loop.w > 0.5 && in.flip_loop.w < 1.5) || fireball;
    let fire = (in.flip_loop.w > 1.5 && in.flip_loop.w < 2.5) || in.flip_loop.y > 0.5;
    var emission: vec3<f32>;
    if propellant {
        let gun = propellant_flame(temperature, heat_scale, puff.solid, in.gas_state.w, puff.frame.on, fireball);
        burning = gun.x;
        emission = propellant_glow(gun.y, fireball);
    } else if fire {
        // A fire's flame and its rolling bodies of burning gas: soot glowing as a black body at a temperature from its
        // heat, as bright as one (`flame_light`). Until 2026-10-24 they took the explosive's ramp below, some four times
        // as bright and yellower, which the display paled to cream by day.
        emission = flame_light(heat);
    } else {
        // An explosive's fireball: dull red, orange, then white where the burning gas is thick and hot.
        let ember = mix(vec3<f32>(0.45, 0.03, 0.004), vec3<f32>(2.4, 0.45, 0.04), sstep(0.12, 0.55, heat));
        let flame = mix(ember, vec3<f32>(4.2, 1.55, 0.3), sstep(0.55, 1.0, heat));
        emission = flame * (heat * heat) + vec3<f32>(3.5, 2.8, 1.5) * sstep(1.25, 1.6, heat);
    }
    // A fire's gas glows at the fires' flames' lift (`EffectLighting::flames`), the gas of a gun or a blast at the
    // flashes' (`EffectLighting::flash_glow`).
    emission = emitted_at(emission, select(lighting.flash_glow.x, lighting.emission.y, fire && !propellant));
    var albedo = in.look.rgb * (1.0 - burning * min(heat_scale, 1.0) * 0.85);
    if fireball {
        albedo *= fresh_soot(in.gas_state.w);
    }
    // The sun's light through hulls' shadows and the smoke between the puff and the sun, the sky's a little less deep
    // in smoke (`puff_light`); flashes, firelight and the gas's own burning are not shadowed.
    // And the light scattered many times within the gas, the more the thicker it is.
    let deep = 1.0 - exp(-body.b * GAS_DEPTH * in.gas_state.y * MULTIPLE_REACH);
    // What of it the gas gives back: its reflectance (its albedo, twice for a batch tinting it again) over gun smoke's.
    let reflectance = select(albedo, albedo * in.look.rgb, batch.flags.w > 0.0);
    let returned = select(min(dot(reflectance, vec3<f32>(0.2126, 0.7152, 0.0722)) / MULTIPLE_ALBEDO, 1.0), 1.0, propellant);
    let multiple = (lighting.direct.rgb * (MULTIPLE_SUN * in.light.x) + overhead * (MULTIPLE_SKY * in.light.y)) * (deep * returned);
    let lit = albedo * (lighting.direct.rgb * (direct * in.light.x) + sky * in.light.y + multiple
        + flash_light_color() * flash_lit);
    let radiance = lit + emission;
    let front = in.quad.z - bulge_at(in, s);
    let alpha = seen_through(in, puff.coverage * sea_foot(in), front, merged) * in.look.a;
    var tinted = select(radiance, radiance * in.look.rgb, batch.flags.w > 0.0);
    if propellant {
        // A propellant's flame is its own colour (`propellant_glow`), not tinted by its smoke's as an explosive's is.
        tinted = select(lit, lit * in.look.rgb, batch.flags.w > 0.0) + emission;
    }
    return GasShade(vec4<f32>(effect_fog(tinted, in.world.xyz), alpha), front);
}

// How much of what a puff covers at this fragment (`coverage`, its near surface `front` m along the view) shows over the
// scene: behind denser smoke in front, from this batch or another, the puff sinks into it by its own size (merged, the
// smoke in front weighs it instead, and only the clouds and the splash columns in front thin it); soft where it meets
// what lies behind it.
fn seen_through(in: Varyings, coverage: f32, front: f32, merged: bool) -> f32 {
    var seen = coverage;
    if merged {
        seen *= clouds_through(in.position.xy, front) * splashes_through(in.position.xy, front);
    } else if batch.flags.z > 0.0 {
        seen *= smoke_visibility(in.position.xy, front, in.quad.w * 0.5, batch.flags.y > 0.0);
        if batch.flags.z > 1.5 {
            // Drawn after the fires' columns: what of them lies in front.
            seen *= columns_through(in.position.xy, front, in.quad.w * 0.5);
        }
    }
    // A burst's soot is cut off where the scene stands in its march (`flak_soot`), not at its quad.
    if batch.flags.x > 0.0 && in.tactical.w == 0.0 {
        seen *= soft_edge(in.position.xy, in.world.w, in.place.w);
    }
    return seen;
}

// A gun's propellant (`GAS_PROPELLANT`): the fuel-rich gas out of the bore burns as it mixes with the air ahead of the
// muzzle (the secondary flash), a fireball that burns through most of its gas for a few tenths of a second, then
// withdraws into its hottest pockets as it rolls up into smoke. `heat_scale` is the share of its burning still to
// come: about 1.3 out of the bore, nothing once its recipe's cooling is spent (`blasts.rs`). `temperature` is the
// `simulated` fireball's flame, about 0.15 to 0.45 through a young one, or a still puff's baked flame, which runs about
// twice as hot. How much of the gas burns (x) and how hot its flame is on `propellant_glow`'s ramp (y): in its first
// tenth of a second (`age`) the hottest cores burn white. A gun's fireball (`whole`, `GAS_FIREBALL`) burns white-hot
// through all but its thinnest gas while its burning still to come is over `WHITE_HEAT`, a white that overexposes as on
// film, hottest where it is thickest, and goes over to its flame's own colours below it.
fn propellant_flame(temperature: f32, heat_scale: f32, solid: f32, age: f32, simulated: bool, whole: bool) -> vec2<f32> {
    let hot = min(temperature * select(1.2, 2.4, simulated), 1.15);
    let front = 1.2 - heat_scale;
    // Once its burning is spent nothing burns, however hot the simulation's cell.
    var burning = sstep(front - 0.15, front + 0.2, hot) * sstep(0.0, 0.1, heat_scale);
    let fresh = 1.0 - sstep(0.03, 0.14, age);
    var heat = burning * (0.05 + heat_scale * (1.3 * hot - 0.25) + 0.1 * solid + 0.5 * fresh * sstep(0.75, 1.0, hot));
    if whole {
        let white = sstep(WHITE_HEAT.x, WHITE_HEAT.y, heat_scale) * sstep(0.02, 0.3, solid);
        burning = max(burning, white);
        heat = mix(heat, 1.95 + 0.45 * solid, white);
    }
    return vec2<f32>(burning, heat);
}

// A gun's fireball's soot: the share of its smoke's colour it reflects at `age` (s), dark while fresh and paling as it
// mixes with the air (`FRESH_SOOT`, `SOOT_PALES`).
fn fresh_soot(age: f32) -> f32 {
    return mix(FRESH_SOOT, 1.0, sstep(SOOT_PALES.x, SOOT_PALES.y, age));
}

// The propellant flame's light (linear, before the display transform) at `heat`: dull red where it is coolest, then
// the orange and yellow that cover most of a battleship's fireball in the photographs of her broadsides, and white only
// in the hottest cores. The display transform (AgX) pales any colour as bright as flame towards cream, the explosive's
// ramp's yellows included, so this one keeps little green and almost no blue below white heat. Past white heat a gun's
// fireball (`whole`) goes on to a white that overexposes, as the films of broadsides show its first tenth of a second.
fn propellant_glow(heat: f32, whole: bool) -> vec3<f32> {
    var light = vec3<f32>(0.06, 0.003, 0.0) * sstep(0.0, 0.15, heat);
    light += (vec3<f32>(0.42, 0.045, 0.0) - vec3<f32>(0.06, 0.003, 0.0)) * sstep(0.15, 0.4, heat);
    light += (vec3<f32>(1.0, 0.16, 0.003) - vec3<f32>(0.42, 0.045, 0.0)) * sstep(0.4, 0.7, heat);
    light += (vec3<f32>(1.55, 0.5, 0.012) - vec3<f32>(1.0, 0.16, 0.003)) * sstep(0.7, 1.0, heat);
    light += (vec3<f32>(2.3, 1.0, 0.05) - vec3<f32>(1.55, 0.5, 0.012)) * sstep(1.0, 1.3, heat);
    light += (vec3<f32>(3.6, 2.2, 0.32) - vec3<f32>(2.3, 1.0, 0.05)) * sstep(1.3, 1.6, heat);
    light += (vec3<f32>(5.2, 4.3, 1.15) - vec3<f32>(3.6, 2.2, 0.32)) * sstep(1.6, 1.9, heat);
    if whole {
        light += (vec3<f32>(9.0, 8.4, 5.6) - vec3<f32>(5.2, 4.3, 1.15)) * sstep(1.9, 2.4, heat);
    }
    return light;
}

struct Occlusion {
    // Nearness of the nearest front per coverage level (max blending).
    @location(0) fronts: vec4<f32>,
    // Optical depth and optical depth × nearness of its middle (added).
    @location(1) depths: vec4<f32>,
}

// The same puffs for the smoke occlusion map: at each coverage level the nearest front of gas at least that
// opaque, and the gas's optical depth.
@fragment
fn occluder(in: Varyings) -> Occlusion {
    if in.tactical.w > 0.0 {
        let smoke = tactical_smoke(in);
        let opaque = clamp(smoke.opacity, 0.0, 0.9999);
        if opaque < 0.01 { discard; }
        let near = nearness_of(smoke.front);
        let optical = -log(1.0 - opaque);
        var screen: Occlusion;
        screen.fronts = near * smoothstep(vec4<f32>(0.15, 0.4, 0.7, 0.85), vec4<f32>(0.27, 0.52, 0.82, 0.97), vec4<f32>(opaque));
        screen.depths = vec4<f32>(optical, optical * nearness_of(smoke.middle), 0.0, 0.0);
        return screen;
    }
    var opaque: f32;
    var front: f32;
    var centre: f32;
    if in.tactical.w < 0.0 {
        // A burst's soot, its middle where the steps that show it weigh it.
        let soot = flak_soot(in, false);
        opaque = clamp(soot.opacity, 0.0, 0.99);
        front = soot.front;
        centre = nearness_of(soot.middle);
    } else {
        let s = in_tile_frame(in);
        opaque = clamp(puff_at(in, s).coverage * in.look.a * sea_foot(in), 0.0, 0.99);
        front = in.quad.z - bulge_at(in, s);
        centre = nearness_of(in.quad.z);
    }
    if opaque < 0.01 {
        discard;
    }
    let near = nearness_of(front);
    let optical = -log(1.0 - opaque);
    // Each level fades in over the coverage just below it, so what lies behind the rim of a denser patch
    // disappears into it gradually rather than along its outline.
    let levels = vec4<f32>(0.25, 0.5, 0.8, 0.95);
    var out: Occlusion;
    out.fronts = vec4<f32>(
        near * sstep(levels.x - 0.1, levels.x + 0.02, opaque),
        near * sstep(levels.y - 0.1, levels.y + 0.02, opaque),
        near * sstep(levels.z - 0.1, levels.z + 0.02, opaque),
        near * sstep(levels.w - 0.1, levels.w + 0.02, opaque));
    out.depths = vec4<f32>(optical, optical * centre, 0.0, 0.0);
    return out;
}

// A link or billow of a fire's column as the occlusion map takes it (`column_occluder`).
struct ColumnVaryings {
    @builtin(position) position: vec4<f32>,
    // Quad coordinate (−1 … 1), centre depth along the view (m), radius (m).
    @location(0) quad: vec4<f32>,
    // World position, distance along the view (m).
    @location(1) world: vec4<f32>,
    // `flipbook_varying`.
    @location(2) @interpolate(flat) flip: vec4<f32>,
    // Extinction multiple as it thins, dispersal, detail strength, detail seed.
    @location(3) @interpolate(flat) gas: vec4<f32>,
    // Opacity, age, the loop's first layer, and the height (m) over which it stands clear of the flames it rose from
    // (`GasPuff::quad`'s w).
    @location(4) @interpolate(flat) look: vec4<f32>,
}

@vertex
fn column_occluder_vertex(@builtin(vertex_index) vertex_index: u32, @builtin(instance_index) instance: u32) -> ColumnVaryings {
    let v = place(vertex_index, instance);
    var out: ColumnVaryings;
    out.position = v.position;
    out.quad = v.quad;
    out.world = v.world;
    out.flip = v.flip;
    out.gas = vec4<f32>(v.gas_state.y, v.gas_state.z, v.detailed.z, v.detailed.w);
    out.look = vec4<f32>(v.look.a, v.gas_state.w, v.flip_loop.x, puffs[instance].quad.w);
    return out;
}

// A link or billow of a fire's column into the occlusion map's two channels of its own (`depths.ba`, which
// `columns_through` reads): its optical depth, and that times the nearness of its near surface. The column is drawn
// before a fire's flames, which would otherwise show through all of its smoke in front of them (from above, through the
// whole column). Only where its near surface stands above the flames it rose from: from the side the flames stand in
// the column's foot, among its glowing puffs, and those must not black them out; from above, the smoke over them dims
// them to a glow through it.
@fragment
fn column_occluder(in: ColumnVaryings) -> Occlusion {
    let s = vec2<f32>(select(in.quad.x, -in.quad.x, in.flip.w < 0.0), in.quad.y);
    let front = in.quad.z - sqrt(max(1.0 - dot(s, s), 0.0)) * in.quad.w;
    // How high its near surface stands where this pixel's ray meets it: the quad's point taken along its ray to the
    // surface's distance along the view.
    let height = camera.position.y + (in.world.y - camera.position.y) * (front / max(in.world.w, 1e-3));
    let clear = sstep(in.look.w, in.look.w + max(in.quad.w * 0.25, 2.0), height);
    let opaque = clamp(column_gas(in.flip, in.look.z, s, in.gas, in.look.y).coverage * in.look.x * clear, 0.0, 0.99);
    if opaque < 0.01 {
        discard;
    }
    let optical = -log(1.0 - opaque);
    var out: Occlusion;
    out.fronts = vec4<f32>(0.0);
    out.depths = vec4<f32>(0.0, 0.0, optical, optical * nearness_of(front));
    return out;
}
