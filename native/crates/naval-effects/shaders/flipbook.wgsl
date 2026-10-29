// Trial look `flipbook` (src/trial.rs, src/flipbook.rs): a gas puff drawn from the frames of a baked fluid simulation
// instead of a still puff of the atlas. Each frame holds the atlas's channels (the six-way light, optical depth and
// the flame the eye sees), so the sprite relights it as it does a still puff; two neighbouring frames are each read a
// share of the way along the motion between them and blended (Klemen Lozar, "Frame blending with motion vectors"),
// so the gas flows from one frame to the next instead of fading between them.
//
// A puff plays a clip (a stretch of a simulation's frames) over its life. Each frame was framed on its own gas, so a
// small early fireball fills its tile: the frame's share of the last frame's size comes with it, and the sprite is
// drawn that much smaller, standing on the simulation's ground where it has one.
//
// A fire's clip (src/flipbook_fires.rs) is a loop instead: the puff's life is its place in the loop, whose last frame
// flows into its first. Its sprite stands upright on its foot, turning about the vertical to face the lens, leans with
// the wind, and its flame's heat is the simulation's temperature all the way down the heat ramp.
//
// A sprite shows its simulation from one side whichever way it is seen, so a puff the eye circles turns with the eye.
// The fireball's later frames are baked from a second side, ninety degrees round to its right (`SIDE_BOOKS` in
// src/flipbook.rs). With the look `views` a puff playing them keeps a heading of its own, upright, and shows the two
// sides nearest the eye's bearing on it, blended (`flipbook_views`). Seen from behind, gas has the outline it has from
// in front, mirrored, so the far two sides are the near two read right to left. A side's tile is as tall as the first
// side's and as wide as its own gas reaches (a fireball blown along its way is long seen from the side); the side book
// keeps that width over the first side's in place of each frame's scale, and the sprite is drawn that much wider while
// it shows that side.
#define_import_path naval_effects::flipbook

struct Flipbooks {
    // Per clip: its first and last layer (last below first: no such simulation loaded), how fast it plays early in
    // the puff's life (0 evenly), and where the simulation's ground is under the tile's centre, in half tiles (0: the
    // tile is centred on the gas, and the puff may turn).
    clips: array<vec4<f32>, 8>,
    // Per clip: the share of the tile a motion channel's ±1 spans, texels across a tile, 1 for a fire's loop and 2 for
    // a smoke column's, the tile's width over its height where it stands in the world.
    motion: array<vec4<f32>, 8>,
    // Each layer's size as a share of its simulation's last frame's, four to a vector.
    scales: array<vec4<f32>, 64>,
    // Per clip baked from a second side as well: how many layers on the side's frame lies, the first and last of the
    // clip's layers the side's book holds, and the share of the tile its motion channels' ±1 span (0: one side only).
    // A side book's layers keep its tile's width over the first side's in `scales`.
    views: array<vec4<f32>, 8>,
}

@group(2) @binding(30) var flipbook_sides: texture_2d_array<f32>;
@group(2) @binding(31) var flipbook_body: texture_2d_array<f32>;
@group(2) @binding(32) var flipbook_motion: texture_2d_array<f32>;
@group(2) @binding(33) var flipbook_sampler: sampler;
@group(2) @binding(34) var<uniform> flipbooks: Flipbooks;

// A puff's place in its clip, worked out per vertex.
struct FlipPuff {
    // 1 when the puff is drawn from a simulation.
    on: f32,
    // The frame it shows and its share of the way to the next (layer + share), and the clip's last layer.
    position: f32,
    last: f32,
    // The gas's size in this frame as a share of the last frame's, and the ground under the tile's centre.
    scale: f32,
    anchor: f32,
    // −1 for a puff seen mirrored (every other one, by its seed).
    mirror: f32,
    // The share of the tile a motion channel's ±1 spans, and texels across a tile.
    range: f32,
    texels: f32,
    // A loop: its first layer (−1: not a loop) and the tile's width over its height.
    first: f32,
    aspect: f32,
    // 1 a fire's flame, standing on its foot; 2 a link of a smoke column, laid along it; 0 neither.
    kind: f32,
    // The clip's row of `Flipbooks.views` while the look `views` is on (zero otherwise), and its place in the table
    // counted from 1.
    views: vec4<f32>,
    row: f32,
}

fn flipbook_scale(layer: f32) -> f32 {
    let index = u32(layer);
    return flipbooks.scales[index / 4u][index % 4u];
}

// The clip `clip` (1 and up; 0 is a still puff) at `life` (0–1 of the puff's life); `trial` is `lighting.trial.y`.
fn flipbook_puff(clip: f32, life: f32, seed: f32, trial: f32) -> FlipPuff {
    var out = FlipPuff(0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 256.0, -1.0, 1.0, 0.0, vec4<f32>(0.0), 0.0);
    if trial < 0.5 || clip < 0.5 {
        return out;
    }
    let index = min(u32(clip + 0.5) - 1u, 7u);
    let layers = flipbooks.clips[index];
    if layers.y < layers.x {
        return out;
    }
    // Fast at first, as a blast is, and slower as its smoke hangs: 1 − e^(−k t), over the whole clip.
    let t = clamp(life, 0.0, 1.0);
    var played = t;
    if layers.z > 0.01 {
        played = (1.0 - exp(-layers.z * t)) / (1.0 - exp(-layers.z));
    }
    var position = layers.x + played * (layers.y - layers.x);
    var layer = min(floor(position), layers.y);
    var next = min(layer + 1.0, layers.y);
    if flipbooks.motion[index].z > 0.5 {
        // A loop: `life` is the place in it, and its last frame's neighbour is its first.
        position = layers.x + fract(life) * (layers.y - layers.x + 1.0);
        layer = min(floor(position), layers.y);
        next = select(layer + 1.0, layers.x, layer + 1.0 > layers.y);
        out.first = layers.x;
        out.aspect = flipbooks.motion[index].w;
        out.kind = flipbooks.motion[index].z;
    }
    out.on = 1.0;
    out.position = position;
    out.last = layers.y;
    out.scale = mix(flipbook_scale(layer), flipbook_scale(next), position - layer);
    out.anchor = layers.w;
    out.mirror = select(1.0, -1.0, fract(seed * 0.4142) > 0.5);
    out.range = flipbooks.motion[index].x;
    out.texels = flipbooks.motion[index].y;
    if trial > 1.5 {
        out.views = flipbooks.views[index];
        out.row = f32(index + 1u);
    }
    return out;
}

// Layers over which a puff turns from its first side to the eye's, from the first layer its side book holds (src/flipbook.rs'
// `VIEWS_RAMP`), and how far into a quarter turn from either end a puff shows that end's side alone: the two are blended
// only through the middle 0.4 of the quarter (blended all the way round, two outlines show through each other at every
// turn; and a blend reads twice).
const VIEWS_RAMP: f32 = 4.0;
const VIEWS_ALONE: f32 = 0.3;

// Whether `puff` is seen from its sides (it is playing frames its side book holds).
fn flipbook_turns(puff: FlipPuff) -> bool {
    return puff.views.w > 0.0 && puff.position > puff.views.y;
}

// The sides a puff shows, for `flipbook_frame` and `flipbook_widen`: x the first of the two sides nearest the eye's
// bearing, by quarter turns round from its first book's (0 that side, 1 the side book's, 2 and 3 those two read right
// to left), times 2, plus the second side's share (0: one side); y its clip's row of `Flipbooks.views` counted from 1 (0:
// one side, as the puff was before `views`); z the side book's tile's width over the first side's; w the first side's
// share while the puff turns from it to the eye's, over the side book's first few layers (`VIEWS_RAMP`): a cross-fade,
// since no turn of the bearing can take every bearing to the first side without a jump somewhere round the puff.
// `to_camera` points from the puff to the eye; the puff's heading is seeded by `seed`, and a mirrored puff is the
// simulation mirrored, its sides coming round the other way.
fn flipbook_views(puff: FlipPuff, to_camera: vec3<f32>, seed: f32) -> vec4<f32> {
    if !flipbook_turns(puff) {
        return vec4<f32>(0.0);
    }
    // Where the eye of its first side stands from it, and where this eye does: the angle from one to the other round
    // toward that side's right (the up axis crossed with it, as the billboard's right).
    let heading = fract(seed * 0.5698 + 0.31) * 6.2831853;
    let first = vec2<f32>(cos(heading), sin(heading));
    let level = to_camera.xz;
    var angle = 0.0;
    if dot(level, level) > 1e-8 {
        angle = atan2(level.x * first.y - level.y * first.x, dot(level, first));
    }
    let quarters = fract(angle * puff.mirror / 6.2831853 + 1.0) * 4.0;
    var side = floor(quarters);
    var second = smoothstep(VIEWS_ALONE, 1.0 - VIEWS_ALONE, quarters - side);
    if second >= 1.0 {
        side += 1.0;
        second = 0.0;
    }
    side -= 4.0 * floor(side / 4.0);
    // The side tile's width in this frame (`position` lies within the side book's layers: `flipbook_turns`).
    let layer = floor(puff.position);
    let next = min(layer + 1.0, puff.last);
    let wide = mix(flipbook_scale(layer + puff.views.x), flipbook_scale(next + puff.views.x), puff.position - layer);
    let turning = 1.0 - smoothstep(puff.views.y, puff.views.y + VIEWS_RAMP, puff.position);
    return vec4<f32>(side * 2.0 + second, puff.row, max(wide, 1.0), turning);
}

// How much wider than its first side's tile a puff's quad is drawn (`views` is `flipbook_views`'): as wide as the side
// book's tile while it shows that side at all.
fn flipbook_widen(views: vec4<f32>) -> f32 {
    if views.y < 0.5 {
        return 1.0;
    }
    let side = floor(views.x * 0.5);
    let shows = views.x - side * 2.0 > 0.0 || side == 1.0 || side == 3.0;
    return select(1.0, views.z, shows);
}

// The camera-facing quad of a puff at `center`: half its radius nearer the camera, scaled to keep its angular size
// (`place_facing` in src/particles.rs, which placed the puff at its full size).
fn flipbook_quad(center: vec3<f32>, radius: f32, to_camera: vec3<f32>, distance: f32) -> vec4<f32> {
    let pull = min(radius * 0.5, max(distance - 1.0, 0.0));
    return vec4<f32>(center + to_camera * pull, radius * 2.0 * (distance - pull) / max(distance, 1e-3));
}

// A fire's sprite, standing on its foot.
struct FlipStand {
    // The quad's axes: `right` and `up` are unit (the frame its light is worked out in, with `facing`); `rise` is its
    // upward edge per metre of the flame's length, leant with the wind; `widen` is how much wider the quad is drawn.
    right: vec3<f32>,
    up: vec3<f32>,
    facing: vec3<f32>,
    rise: vec3<f32>,
    center: vec3<f32>,
    widen: f32,
}

// The sprite of a fire's clip of half height `radius` on `foot`: upright, turned about the vertical to face the lens,
// easing to a full billboard seen from steeply above (so a fire under an aircraft does not collapse to a line), and
// leant by `lean` (metres downwind per metre up). A leant flame keeps its length and its tongues their thickness: the
// quad's upward edge is turned, not stretched, and the quad widened by as much as the slant would thin them (a plain
// shear draws a flame in a ship's wind as long thin streaks). `right` and `up` are the puff's billboard axes.
fn flipbook_stand(foot: vec3<f32>, radius: f32, anchor: f32, lean: vec3<f32>, to_camera: vec3<f32>, right: vec3<f32>, up: vec3<f32>) -> FlipStand {
    let steep = smoothstep(0.72, 0.96, abs(to_camera.y));
    let level = vec3<f32>(to_camera.x, 0.0, to_camera.z);
    let flat_facing = select(normalize(level), -normalize(vec3<f32>(up.x, 0.0, up.z) + vec3<f32>(1e-5, 0.0, 0.0)), dot(level, level) < 1e-6);
    var out: FlipStand;
    out.facing = normalize(mix(flat_facing, to_camera, steep));
    let stood = normalize(mix(vec3<f32>(0.0, 1.0, 0.0), up, steep));
    out.right = normalize(cross(stood, out.facing));
    out.up = cross(out.facing, out.right);
    out.rise = normalize(out.up + lean);
    let slant = dot(lean, out.right);
    out.widen = sqrt(1.0 + slant * slant);
    out.center = foot + out.rise * (radius * anchor);
    return out;
}

// What the vertex hands the fragment: the place in the layers, the clip's last layer, the mip level a puff of
// `pixels` radius on screen reads, and the motion's range, negative for a mirrored puff (0: a still puff).
fn flipbook_varying(puff: FlipPuff, pixels: f32) -> vec4<f32> {
    if puff.on < 0.5 {
        return vec4<f32>(0.0);
    }
    // A tile narrower than it is tall is read between the levels its width and its height ask for.
    let level = clamp(log2(puff.texels / max(pixels * 2.0 * sqrt(puff.aspect), 1.0)), 0.0, 8.0);
    return vec4<f32>(puff.position, puff.last, level, puff.range * puff.mirror);
}

// And for a loop: its first layer (−1: not a loop), 1 for a fire's flame, 1 for a link of a smoke column (whose two
// frames are cross-faded where they lie: its smoke moves a texel or two a frame, and the bake measured the blend along
// its motion no nearer the frames between than the plain fade, so the motion is not read).
fn flipbook_loop(puff: FlipPuff) -> vec4<f32> {
    return vec4<f32>(puff.first, select(0.0, 1.0, puff.kind > 0.5 && puff.kind < 1.5), select(0.0, 1.0, puff.kind > 1.5), 0.0);
}

// A link of a smoke column: the sprite faces the lens and lies along `axis` (the way the column runs past the puff, in
// the world), so the smoke rolls up the tile along the column and links chained along it overlap into one body.
// A column seen end on is as short as it is wide (`aspect`, the tile's width over its length), and turns toward the
// billboard's `up` rather than spin. `right` and `up` are the puff's billboard axes.
fn flipbook_link(axis: vec3<f32>, aspect: f32, to_camera: vec3<f32>, right: vec3<f32>, up: vec3<f32>) -> FlipStand {
    let across = axis - to_camera * dot(axis, to_camera);
    let seen = length(across);
    var out: FlipStand;
    out.facing = to_camera;
    out.up = normalize(across + up * (0.3 * (1.0 - smoothstep(0.0, 0.4, seen))));
    out.right = normalize(cross(out.up, to_camera));
    out.rise = out.up * max(seen, aspect);
    out.widen = 1.0;
    return out;
}

// The sides a fragment reads (one, or up to three seen from its sides: `flipbook_views`), each in its own fields (a, b,
// c; no arrays, which a shader compiler may keep in memory rather than registers when indexed in a loop): where it
// reads its book's two frames (xy this one, zw the next) and their layers, and per side (x a, y b, z c) its share of the
// blend (0: not read), 1 where its tile is read right to left (its light from ±X with it) and 1 where the fragment lies
// beyond its tile (no gas there).
struct FlipFrame {
    on: bool,
    read_a: vec4<f32>,
    read_b: vec4<f32>,
    read_c: vec4<f32>,
    layers_a: vec2<i32>,
    layers_b: vec2<i32>,
    layers_c: vec2<i32>,
    weights: vec3<f32>,
    mirrored: vec3<f32>,
    beyond: vec3<f32>,
    share: f32,
    level: f32,
}

// Where a fragment at `uv` reads `layer` and the layer after it, `share` of the way between them along the motion
// (its channels' ±1 spanning `range` of the tile), unless `still` (a column's link: both where they lie).
fn flipbook_reads(uv: vec2<f32>, layer: f32, share: f32, level: f32, range: f32, still: bool) -> vec4<f32> {
    if still {
        return vec4<f32>(uv, uv);
    }
    // rg: where this texel of the next frame was in this one; ba: where this texel of this frame is in the next.
    let motion = (textureSampleLevel(flipbook_motion, flipbook_sampler, uv, i32(layer), max(level - 1.0, 0.0)) * 2.0 - 1.0) * range;
    return vec4<f32>(uv + motion.rg * share, uv + motion.ba * (1.0 - share));
}

// One side as a fragment reads it: where, which layers, whether right to left and whether beyond its tile.
struct FlipSide {
    read: vec4<f32>,
    layers: vec2<i32>,
    mirrored: f32,
    beyond: f32,
}

// Side `side` (0–3: the first book's, the side book's, those two read right to left) for `s` in the first side's
// tile's frame, at `layer` and `next` of the first book, `share` of the way between: the side book's layers lie
// `row.x` on and its motion spans `row.w` (the first book's `range`); its tile is `wide` times as wide. `seen`: the
// puff is seen from its sides, so a quad drawn wider than this side's tile reads nothing beyond it (a puff seen from
// one side reads its tile's edge, as before `views`).
fn flipbook_side(s: vec2<f32>, side: u32, row: vec4<f32>, wide: f32, seen: bool, layer: f32, next: f32, share: f32,
    level: f32, range: f32, still: bool) -> FlipSide {
    let odd = (side & 1u) == 1u;
    let book = select(0.0, row.x, odd);
    let across = select(1.0, 1.0 / wide, odd) * select(1.0, -1.0, side >= 2u);
    let uv = vec2<f32>(s.x * across, s.y) * 0.5 + 0.5;
    var out: FlipSide;
    out.mirrored = select(0.0, 1.0, side >= 2u);
    out.beyond = select(0.0, 1.0, seen && abs(uv.x - 0.5) > 0.5);
    out.read = flipbook_reads(uv, layer + book, share, level, select(range, row.w, odd), still || out.beyond > 0.5);
    out.layers = vec2<i32>(i32(layer + book), i32(next + book));
    return out;
}

// `flip` is `flipbook_varying`'s, `wrap` `flipbook_loop`'s, `views` `flipbook_views`'; `s` the fragment in the tile's
// frame (−1 … 1 over the first side's tile, +Y up; wider than it across a quad drawn wider, `flipbook_widen`).
fn flipbook_frame(flip: vec4<f32>, wrap: vec4<f32>, s: vec2<f32>, views: vec4<f32>) -> FlipFrame {
    var out: FlipFrame;
    out.on = flip.w != 0.0;
    if !out.on {
        return out;
    }
    let layer = floor(flip.x);
    let share = flip.x - layer;
    var next = min(layer + 1.0, flip.y);
    if wrap.x >= 0.0 && layer + 1.0 > flip.y {
        next = wrap.x;
    }
    out.share = share;
    out.level = flip.z;
    let still = wrap.z > 0.5;
    let range = abs(flip.w);
    // One side, the puff's own (as before `views`); or the two nearest the eye's bearing and, while the puff turns to
    // them, its first side, which is one of the two when the eye is on its first side's half.
    var side = 0u;
    var row = vec4<f32>(0.0);
    var wide = 1.0;
    out.weights = vec3<f32>(1.0, 0.0, 0.0);
    let seen = views.y > 0.5;
    if seen {
        row = flipbooks.views[min(u32(views.y + 0.5) - 1u, 7u)];
        side = u32(floor(views.x * 0.5));
        let second = views.x - f32(side) * 2.0;
        out.weights = vec3<f32>(1.0 - second, second, 0.0) * (1.0 - views.w);
        if side == 0u {
            out.weights.x += views.w;
        } else if side == 3u {
            out.weights.y += views.w;
        } else {
            out.weights.z = views.w;
        }
        wide = views.z;
    }
    let a = flipbook_side(s, side, row, wide, seen, layer, next, share, flip.z, range, still);
    out.read_a = a.read;
    out.layers_a = a.layers;
    out.mirrored.x = a.mirrored;
    out.beyond.x = a.beyond;
    if out.weights.y > 0.0 {
        let b = flipbook_side(s, (side + 1u) & 3u, row, wide, seen, layer, next, share, flip.z, range, still);
        out.read_b = b.read;
        out.layers_b = b.layers;
        out.mirrored.y = b.mirrored;
        out.beyond.y = b.beyond;
    }
    if out.weights.z > 0.0 {
        let c = flipbook_side(s, 0u, row, wide, seen, layer, next, share, flip.z, range, still);
        out.read_c = c.read;
        out.layers_c = c.layers;
        out.mirrored.z = c.mirrored;
        out.beyond.z = c.beyond;
    }
    return out;
}

// Optical depth over the share of it the flipbook stores (pack.py's GAS_DEPTH, gas_sprite.wgsl's).
const FLIP_GAS_DEPTH: f32 = 12.0;

// The atlas's `body` from a side's two frames (`read`, `layers`): light from the front, from behind ÷ 2, optical depth ÷
// GAS_DEPTH (stored as its root, so thin smoke keeps fine steps), the flame the eye sees. Nothing `beyond` its tile.
fn flipbook_body_of(read: vec4<f32>, layers: vec2<i32>, share: f32, level: f32, beyond: f32) -> vec4<f32> {
    if beyond > 0.5 {
        return vec4<f32>(0.0);
    }
    let a = textureSampleLevel(flipbook_body, flipbook_sampler, read.xy, layers.x, level);
    let b = textureSampleLevel(flipbook_body, flipbook_sampler, read.zw, layers.y, level);
    var body = mix(a, b, share);
    body.b = mix(a.b * a.b, b.b * b.b, share);
    return body;
}

// The gas a fragment shows (the atlas's `body`) and each side's share of its light.
struct FlipGas {
    body: vec4<f32>,
    light: vec3<f32>,
}

// The sides' gas blended: their optical depths and flames by their shares, their light (baked per unit of what the gas
// stops) by what each stops, so a side with no gas here does not dim another's.
fn flipbook_gas(frame: FlipFrame) -> FlipGas {
    let a = flipbook_body_of(frame.read_a, frame.layers_a, frame.share, frame.level, frame.beyond.x);
    if frame.weights.y <= 0.0 && frame.weights.z <= 0.0 {
        return FlipGas(a, vec3<f32>(1.0, 0.0, 0.0));
    }
    var b = vec4<f32>(0.0);
    if frame.weights.y > 0.0 {
        b = flipbook_body_of(frame.read_b, frame.layers_b, frame.share, frame.level, frame.beyond.y);
    }
    var c = vec4<f32>(0.0);
    if frame.weights.z > 0.0 {
        c = flipbook_body_of(frame.read_c, frame.layers_c, frame.share, frame.level, frame.beyond.z);
    }
    let w = frame.weights;
    let depths = vec3<f32>(a.b, b.b, c.b);
    let stops = (vec3<f32>(1.0) - exp(depths * -FLIP_GAS_DEPTH)) * w + vec3<f32>(1e-5, 0.0, 0.0);
    let light = stops / (stops.x + stops.y + stops.z);
    let body = vec4<f32>(a.rg * light.x + b.rg * light.y + c.rg * light.z, dot(depths, w), dot(vec3<f32>(a.a, b.a, c.a), w));
    return FlipGas(body, light);
}

fn flipbook_body_at(frame: FlipFrame) -> vec4<f32> {
    return flipbook_gas(frame).body;
}

// The atlas's `sides` from a side's two frames: light from +X, −X, +Y, −Y of the tile as read (right to left: its light
// from +X is the book's from −X).
fn flipbook_sides_of(read: vec4<f32>, layers: vec2<i32>, share: f32, level: f32, mirrored: f32) -> vec4<f32> {
    let a = textureSampleLevel(flipbook_sides, flipbook_sampler, read.xy, layers.x, level);
    let b = textureSampleLevel(flipbook_sides, flipbook_sampler, read.zw, layers.y, level);
    let sides = mix(a, b, share);
    return select(sides, sides.grba, mirrored > 0.5);
}

// The light from the sides, `light` (`FlipGas.light`) of it from each; nothing read for a side with no share or
// beyond its tile.
fn flipbook_sides_at(frame: FlipFrame, light: vec3<f32>) -> vec4<f32> {
    var sides = vec4<f32>(0.0);
    if light.x > 0.0 && frame.beyond.x < 0.5 {
        sides = flipbook_sides_of(frame.read_a, frame.layers_a, frame.share, frame.level, frame.mirrored.x) * light.x;
    }
    if light.y > 0.0 && frame.beyond.y < 0.5 {
        sides += flipbook_sides_of(frame.read_b, frame.layers_b, frame.share, frame.level, frame.mirrored.y) * light.y;
    }
    if light.z > 0.0 && frame.beyond.z < 0.5 {
        sides += flipbook_sides_of(frame.read_c, frame.layers_c, frame.share, frame.level, frame.mirrored.z) * light.z;
    }
    return sides;
}

// A fire's flame: how much of the gas burns (x) and its heat on the sprite's ramp (y), from the simulation's
// `temperature` and the puff's `heat_scale`. Where a blast's fireball burns behind a threshold that rises as it cools,
// a flame's heat follows its temperature all the way down: white-yellow where the flame is thick and hot, orange, then
// dark red where it thins and cools at its edges and tips.
fn flipbook_flame(temperature: f32, heat_scale: f32) -> vec2<f32> {
    // The faint haze of flame the simulation carries far up into its soot is left dark: it would redden the smoke.
    return vec2<f32>(smoothstep(0.1, 0.45, temperature), heat_scale * temperature * smoothstep(0.1, 0.34, temperature));
}
