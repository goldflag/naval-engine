// The warp trial (`Trial::warp`, `lighting.trial.x`): still puffs made to roll in the sprite shaders. Not a port.
//
// A puff is a still picture: a tile of the gas atlas, the funnel's cauliflower, a ball of noise. Here its lookups are
// pulled along a flow fixed in the puff: a rolling thermal about the direction the puff was thrown or rises in (up the
// middle, out over the head and back down the rim, the way a fireball turns itself inside out), and eddies from a
// small tiling map of divergence-free flow (`warp::flow_texture`). A flow stretches a picture without bound, so two
// layers ride it a half cycle apart and cross-fade: each fades in as its cycle starts, is alone when it is undisturbed
// and fades out as its stretch grows. What tiles (the small billows, the fire columns' noise) moves to a fresh patch
// while it is out, so it streams on for good; a puff's own shape stays its own, pulled no further than half a cycle
// either way. The parts of a puff run staggered through their cycles, so it never fades all at once. How far the roll
// has turned is the puff's own (`warp::roll`: from its swelling and its age), never the clock's: a held frame holds
// still, a replay rolls the same.
#define_import_path naval_effects::warp

// The eddies: flow x and y in R and G (0.5 still), two unrelated smooth fields in B and A, repeating.
@group(1) @binding(20) var warp_flow: texture_2d<f32>;
@group(1) @binding(21) var warp_sampler: sampler;

// Puff radii of roll a layer rides before it starts again: it is never pulled further than half of this (times the
// flow, at most about 1) from its own picture. Shorter and nothing travels far enough to be seen to move; longer and
// the flow's shear shows in the picture.
const ROLL_CYCLE: f32 = 0.5;
// The thermal's speed up the middle (down the rim it is twice that) and the eddies' fastest, as multiples of the roll.
const ROLL_THERMAL: f32 = 0.5;
const ROLL_EDDIES: f32 = 0.5;
// The eddy map's repeats across a puff's radius: about three eddies across a puff.
const ROLL_EDDY_SCALE: f32 = 0.35;
// How far apart in their cycles the parts of one puff run, end to end.
const ROLL_STAGGER: f32 = 0.6;
// The small billows stream over the body: they ride the same flow through this many cycles for each of the body's.
const ROLL_DETAIL: f32 = 2.5;

// A puff's place in its cycles from how far it has rolled (`warp::roll`) and its seed, which starts every puff of a
// burst at a place of its own so they never fade together.
fn roll_phase(rolled: f32, seed: f32) -> f32 {
    return rolled / ROLL_CYCLE + fract(seed * 0.7548);
}

// A sprite's roll and its seed (0 … 1) from the one number its pool sends (`warp::sprite`): a sprite has no seed of its
// own, so it comes in whole sixteens above the roll.
fn roll_sprite(packed: f32) -> vec2<f32> {
    let sixteens = floor(packed / 16.0);
    return vec2<f32>(packed - sixteens * 16.0, (sixteens + 0.5) / 256.0);
}

// The small billows' place in their own, shorter cycles for a puff at `phase`.
fn roll_detail_phase(phase: f32) -> f32 {
    return phase * ROLL_DETAIL + 0.31;
}

struct RollFlow {
    // The flow, where 1 carries a picture one radius per radius of roll.
    flow: vec2<f32>,
    // How far ahead in their cycles the body and the small billows run here.
    stagger: vec2<f32>,
}

// The flow at `s` in the puff's unit disc. `axis` is the way the puff travels as seen in the disc's frame (its length
// how side-on that is seen, 1 from abeam), `toward` its share toward the eye, `seed` the puff's own (0 … 1).
fn roll_flow(s: vec2<f32>, axis: vec2<f32>, toward: f32, seed: f32) -> RollFlow {
    let abeam = length(axis);
    let ahead = axis / max(abeam, 1e-3);
    let across = vec2<f32>(ahead.y, -ahead.x);
    let a = dot(s, ahead);
    let c = dot(s, across);
    // From abeam, a vortex pair filling the disc: ahead up the middle, apart over the head, back down the rim and in
    // again behind. It neither gathers nor spreads the picture and runs along the rim, never through it.
    let thermal = ahead * (1.0 - 3.0 * c * c - a * a) + across * (2.0 * a * c);
    // End-on, the face of the same ring: out from the middle of a puff coming at the eye, in toward it going away.
    let r2 = dot(s, s);
    let inside = max(1.0 - r2, 0.0);
    let face = s * (inside * toward * 2.0);
    // Eddies, each puff reading a patch of its own, stilled toward the rim.
    let eddy_at = s * ROLL_EDDY_SCALE + vec2<f32>(seed * 3.1, fract(seed * 5.7) * 2.3);
    let map = textureSampleLevel(warp_flow, warp_sampler, eddy_at, 0.0);
    let eddies = (map.rg * 2.0 - 1.0) * inside;
    return RollFlow((thermal * abeam + face) * ROLL_THERMAL + eddies * ROLL_EDDIES, (map.ba - 0.5) * ROLL_STAGGER);
}

struct Roll {
    // Where each layer looks its picture up, in the puff's unit disc.
    first: vec2<f32>,
    second: vec2<f32>,
    // The cycle each layer is in: a whole number, for the caller to choose each cycle's picture by.
    cycles: vec2<f32>,
    // The second layer's share of the two.
    blend: f32,
}

// The two layers for the fragment at `s`, `phase` through its cycles, where the flow is `flow`.
fn roll_layers(s: vec2<f32>, flow: vec2<f32>, phase: f32) -> Roll {
    let at = vec2<f32>(phase, phase + 0.5);
    let cycles = floor(at);
    let within = at - cycles;
    // A picture now at `s` came from back along the flow; each layer is undisturbed at the middle of its cycle.
    let pulled = (within - 0.5) * ROLL_CYCLE;
    var roll: Roll;
    roll.first = s - flow * pulled.x;
    roll.second = s - flow * pulled.y;
    roll.cycles = cycles;
    roll.blend = 1.0 - abs(2.0 * within.y - 1.0);
    return roll;
}
