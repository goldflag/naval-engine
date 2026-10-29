// The water of a shell's splash as a field of density in the splash's own frame (native, `src/columns.rs`; it replaced
// the web game's jets and billows on 2026-10-16). The frame's origin is the point on the sea the shell struck, +y up;
// lengths are the canonical splash's metres and times its seconds: a 38 cm shell's. Every other splash is the same
// splash scaled, its lengths by its size and its times by the root of it, so a small one rises and falls as a scale
// model would under the same gravity; and varied, at its own speed, girth and flare, lobes of its column thrown faster
// than the rest (`columns::Variety`).
//
// Four bodies of water, each where ballistics puts it:
// - The column: water thrown up out of the cavity at launch speeds up to `COLUMN_SPEED`, slowed by a drag in proportion
//   to its speed. Launched together from the cavity's rim, the water's place at time t is its launch velocity scaled by
//   the flight's travel T = (1 − e^(−kt)) / k and lowered by its fall (g / k)(t − T) (`Column::flight`). So each point
//   of the column is one launch velocity, found back from where it is (`thrown_at`), and the column collapses from the
//   sea up: the slow water at its foot falls back first, the fast water of its tips last, as the photographs show.
//   Since 2026-10-23 its water is a fan (`feather`): the sheet thrown up tears at once into feathers, each a bundle of
//   streaks, fanned off the vertical with sky between them, dense only at the foot and thinning toward the tips (the
//   photographs of falls of shot, a shaving brush rather than a pillar; a Houdini trial of the same,
//   `native/tools/effects-splash/`). Its streaks and beads are read in the water's launch directions and speeds, so
//   they rise, stretch and fall with it.
// - The veil: the mist the column sheds as it rises, the column's shape as it stood when it stopped rising, swelling,
//   settling and carried off by the wind.
// - The crown: the low skirt of water thrown out sideways at the strike, falling back within two seconds.
// - The surge: the ring of mist the falling column raises off the sea, rolling outward.
//
// The CPU works out each splash's flight once a frame (`columns.rs`, `ColumnGpu`, mirrored by `Column` here); this
// file holds the shapes. `column.wgsl` marches the field to draw it, `column_caster.wgsl` to cast its shadow.
#define_import_path naval_water_effects::column_field

// One splash (`columns::ColumnGpu`; keep the two in order).
struct Column {
    // xyz the point on the sea the shell struck (world m), w the splash's size (its lengths over the canonical splash's).
    origin: vec4<f32>,
    // The box about its water from the origin (world m, lowest corner), w its age in the canonical splash's seconds.
    low: vec4<f32>,
    // The box's highest corner, w the splash's own number (0–1).
    high: vec4<f32>,
    // xy the column's lean along the shell's way (horizontal metres per metre risen), zw the veil's drift on the wind (m).
    way: vec4<f32>,
    // The column's flight: its travel T and fall (m), how far its head has opened past its launch (a share of its
    // radius), and how far it has broken up into spray (0–1).
    flight: vec4<f32>,
    // The veil's: the column's travel and fall when it was shed, how far it has swollen (a share of its radius), and
    // how much of it there is (0–1).
    veil: vec4<f32>,
    // How far the veil has settled (m), the crown's travel and fall (m), and how much of the crown is left (0–1).
    crown: vec4<f32>,
    // The surge: the radius its rim has rolled out to, its rim's width and its height (m), and how much of it there is.
    surge: vec4<f32>,
    // The water's colour (linear, white unless dyed), and its opacity (0–1).
    tint: vec4<f32>,
    // How it differs from the canonical splash: x its launch speed, y its girth, z its head's flare (shares of the
    // canonical's), w its fastest water's speed, its lobes' (none stands higher).
    shape: vec4<f32>,
    // The box's frame on the sea, a turn about the vertical: its x and z axes (world xz). A splash's is the world's
    // own; a bow's burst's and sheet's lie along her side.
    frame: vec4<f32>,
    // x what the record holds: a splash (0), a bow's burst (1) or a bow's sheet (2), whose lanes `burst_of` and
    // `sheet_of` read; y for a burst or a sheet, which way its frame's z runs along her side (1 aft, −1 forward); z for
    // a splash, where round the vertical its ring of feathers starts (rad, `columns::ring_start`).
    kind: vec4<f32>,
    // A splash's feathers (`columns::feathers`): each one's direction (xyz, unit) and half-width (w, rad).
    feathers: array<vec4<f32>, 8>,
    // Their tips' launch speeds, four a lane (shares of `COLUMN_SPEED`).
    tips: array<vec4<f32>, 2>,
}

// A point or a way from the world (m from the box's origin) into the box's frame, and back.
fn to_frame(v: vec3<f32>, frame: vec4<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v.xz, frame.xy), v.y, dot(v.xz, frame.zw));
}

fn from_frame(v: vec3<f32>, frame: vec4<f32>) -> vec3<f32> {
    let xz = frame.xy * v.x + frame.zw * v.z;
    return vec3<f32>(xz.x, v.y, xz.y);
}

fn is_splash(column: Column) -> bool {
    return column.kind.x < 0.5;
}

fn is_burst(column: Column) -> bool {
    return column.kind.x > 0.5 && column.kind.x < 1.5;
}

fn is_sheet(column: Column) -> bool {
    return column.kind.x > 1.5;
}

@group(1) @binding(1) var column_noise: texture_3d<f32>;
@group(1) @binding(2) var column_sampler: sampler;
@group(1) @binding(3) var<storage, read> columns: array<Column>;

const GRAVITY: f32 = 9.81;
const PI: f32 = 3.14159265;
// The tallest water's launch speed (m/s): the column stands about 64 m at 2.6 s (`columns.rs` keeps the same).
const COLUMN_SPEED: f32 = 70.0;
// The column's fan (`columns.rs`: `FEATHERS`, `SPLASH_ROOT`, `SPLASH_REACH`). All its water (optical depth × m²,
// canonical): through its axis at 2 s, as the light sees it (`fan_spread`), it stops about 540 at the waterline, 100 at
// 20 m, 13 at 35 m and 1.6 at 50 m. The Houdini trial's 20 m³ stopped 100, 40, a few and under 1, and read grey-white
// where the photographs' columns stand white: more water than that. `BETWEEN` of it lies between the feathers, spread
// as the light sees the fan.
const FEATHERS: u32 = 8u;
const FAN_MASS: f32 = 77000.0;
const BETWEEN: f32 = 0.05;
// The fan as the light sees it: its water about the vertical (rad, × the root of the splash's flare), and its tips'
// speed (a share of the splash's). A tip leaning off the vertical is slower: as exp(−(lean / TIP_TILT)²).
const FAN_WIDTH: f32 = 0.26;
const TIP_MEAN: f32 = 0.86;
const TIP_TILT: f32 = 0.85;
// Along a feather: its slowest water (a share of its tip's speed), and the share past which its water thins as the
// sheet tears into spray (by the seventh power: about level below it, a fifth at 0.74, 3 % at the tip). The integral
// of `along_share` over the shares: a feather's water per unit of its speeds.
const SLOWEST: f32 = 0.08;
const THINS_AT: f32 = 0.6;
const ALONG_NORM: f32 = 0.503;
// The cavity's rim the water leaves from (m canonical, × girth), and the farthest the fan's water reaches from its axis
// (a share of the splash's launch speed times the travel: the box's, `columns.rs`).
const SPLASH_ROOT: f32 = 3.0;
const SPLASH_REACH: f32 = 0.55;
// How much of the way across the fan is open between its feathers and strands, where it has fanned out and torn.
const GAPS: f32 = 0.75;
// Toward its tip a feather's spray spreads wider (a share of its half-width).
const TIP_SPREAD: f32 = 0.8;
// Its spray reaches this much past its fastest water's height (`along_share`, under 2 % beyond).
const TIP_END: f32 = 1.15;
// The march's steps through the fan: each takes at most this much of the light, and is at most this many fine steps;
// between the feathers it steps to the nearest one's edge, `FEATHER_EDGE` of its half-widths from its middle (its
// water there under 1e-4 of its middle's).
const STRIDE_DEPTH: f32 = 0.4;
const STRIDE_FAN: f32 = 3.0;
const FEATHER_EDGE: f32 = 3.0;
// Within this lean of the vertical (rad) every feather of the ring is read: there their cones may all reach a point.
const WHOLE_RING: f32 = 0.25;
// A step stopping less of the light than this takes its strands as their mean: too faint to see them.
const SEEN_DEPTH: f32 = 0.004;
// Its streaks: the noise read across the water's directions (periods a radian) and along its speeds (periods a share
// of the splash's speed), and its beads along them.
const STREAKS: f32 = 3.0;
// The sheet's clumps (2026-10-24): the billows' noise read across the launch's lean (periods a unit) and along its speed
// (periods a share of the splash's), half of them read up to `RAGGED` faster, so they end up to a fifth short of the fan
// about them (never past it: the box holds the fan). Low in the column its water only thickens and thins between clumps (to `CLUMP_FOOT`
// of the mean); toward its tips the gaps between them empty, into spires.
const CLUMPS: f32 = 1.2;
const CLUMPS_ALONG: f32 = 0.5;
const RAGGED: f32 = 0.25;
const CLUMP_FOOT: f32 = 0.6;
// At its foot the cone between the feathers holds this share of its water (`BETWEEN` higher up): the sheet there has
// yet to part, so the fan shows no hollow.
const FOOT_BETWEEN: f32 = 0.45;
const STREAK_ALONG: f32 = 0.6;
const BEADS_ALONG: f32 = 2.0;
// The veil is shed from the column as it stood before the fan (2026-10-16): the cavity (m), the outward speed at the
// foot and how much faster the head opened (m/s, growing as the cube of the share of the head's speed).
const COLUMN_ROOT: f32 = 2.5;
const FOOT_SPREAD: f32 = 2.2;
const HEAD_SPREAD: f32 = 8.0;
// The veil: how much wider than the column it stands, its density, and how far below the column's foot and above
// its head it reaches (shares of the head's speed).
const VEIL_WIDTH: f32 = 1.55;
const VEIL_DENSITY: f32 = 0.22;
// The crown: launched this far off the vertical (radians), its spread, and its launch speeds (m/s).
const CROWN_TILT: f32 = 0.7;
const CROWN_TILT_WIDTH: f32 = 0.3;
const CROWN_SLOW: f32 = 5.0;
const CROWN_FAST: f32 = 30.0;
const CROWN_DENSITY: f32 = 0.18;
// The surge's density at the sea, and how much of it fills the ring inside its rim.
const SURGE_DENSITY: f32 = 0.035;
const SURGE_INSIDE: f32 = 0.4;

// One splash's state in the shapes' terms, unpacked once a fragment.
struct Splash {
    // Its record's place in `columns` (its feathers are read from there).
    index: u32,
    t: f32,
    seed: vec3<f32>,
    lean: vec2<f32>,
    drift: vec2<f32>,
    // Column: travel, fall, opening, breakup.
    flight: vec4<f32>,
    // Veil: travel, fall, swelling, amount.
    veil: vec4<f32>,
    // Veil settled, crown travel, crown fall, crown amount.
    crown: vec4<f32>,
    surge: vec4<f32>,
    // Speed, girth, flare, the fastest water's speed.
    shape: vec4<f32>,
}

fn splash_of(column: Column, index: u32) -> Splash {
    let seed = column.high.w;
    return Splash(index, column.low.w, vec3<f32>(seed * 17.3, seed * 7.9, seed * 29.1), column.way.xy, column.way.zw,
        column.flight, column.veil, column.crown, column.surge, column.shape);
}

fn square(x: f32) -> f32 {
    return x * x;
}

// Where the water at a point was thrown from the cavity's rim: its direction (unit, the splash's frame) and speed
// (canonical m/s); speed 0 where no water flies (below the fall of the water launched straight across).
struct Thrown {
    way: vec3<f32>,
    speed: f32,
}

fn thrown_at(p: vec3<f32>, s: Splash) -> Thrown {
    let travel = max(s.flight.x, 1e-3);
    let lifted = p.y + s.flight.y;
    if lifted <= 0.0 {
        return Thrown(vec3<f32>(0.0, 1.0, 0.0), 0.0);
    }
    let off = p.xz - s.lean * lifted;
    let r = length(off);
    let out = max(r - SPLASH_ROOT * s.shape.y, 0.0) / travel;
    let up = lifted / travel;
    let speed = length(vec2<f32>(out, up));
    let flat = off / max(r, 1e-4);
    return Thrown(vec3<f32>(flat.x * out, up, flat.y * out) / speed, speed);
}

// A feather's water at a share of its tip's speed (per unit share, before `ALONG_NORM`): from its slowest water, about
// level, then thinning fast past `THINS_AT` as its sheet tears into spray.
fn along_share(share: f32) -> f32 {
    let x = share / THINS_AT;
    let x3 = x * x * x;
    // Ending within the box (`TIP_END`): early on even the tail's 1 % stands thick (a velocity's volume is T³ its place's).
    return smoothstep(SLOWEST, SLOWEST * 1.9, share) * (1.0 - smoothstep(1.0, TIP_END, share)) / (1.0 + x3 * x3 * x);
}

// The fan's water per unit of launch speed and of solid angle, as the light sees it: its feathers spread evenly about
// the vertical, their tips' speed falling as they lean.
fn fan_spread(launch: Thrown, s: Splash) -> f32 {
    // The square of its lean off the vertical, as 2(1 − cos) (within a percent to half a radian).
    let lean2 = max(2.0 * (1.0 - launch.way.y), 0.0);
    let width2 = FAN_WIDTH * FAN_WIDTH * s.shape.z;
    let tip = COLUMN_SPEED * s.shape.x * TIP_MEAN * exp(-lean2 / (TIP_TILT * TIP_TILT));
    return exp(-lean2 / width2) / (PI * width2) * along_share(launch.speed / tip) / (ALONG_NORM * tip);
}

// One feather's water per unit of launch speed and of solid angle (`water`): about its direction (`axis.xyz`) within
// its half-width (`axis.w`), its spray spreading toward its tip and as the column opens, along its speeds to its tip's
// (`tip`, m/s); and how far round from the point its edge lies (`edge`, rad, at `FEATHER_EDGE` half-widths: the
// march steps over the sky between the feathers by it).
struct Plume {
    water: f32,
    edge: f32,
    width: f32,
}

fn feather(launch: Thrown, axis: vec4<f32>, tip: f32, opening: f32, ended: f32) -> Plume {
    let share = launch.speed / max(tip, 1e-3);
    let width = axis.w * (1.0 + TIP_SPREAD * smoothstep(0.5, 1.0, share)) * (1.0 + opening);
    let apart = max(2.0 * (1.0 - dot(launch.way, axis.xyz)), 0.0);
    let edge = sqrt(apart) - FEATHER_EDGE * width;
    // Where its water may lie the march steps finely, whatever its clump (`ended`: its share of the way along it, so a
    // clump ending short leaves no long step over a neighbour that does not).
    let whole = along_share(share);
    if whole <= 0.0 || edge > 0.0 {
        return Plume(0.0, select(1e3, edge, whole > 0.0), select(1e3, width, whole > 0.0));
    }
    let a = along_share(share * ended);
    return Plume(exp(-apart / (width * width)) / (PI * width * width) * a / (ALONG_NORM * tip), edge, width);
}

// The column's own box (canonical m from the strike): as high as its fastest water's spray stands, as wide as its
// leaning feathers reach, its foot and its head apart as it leans (`columns.rs` keeps the same).
struct FanBox {
    low: vec3<f32>,
    high: vec3<f32>,
}

fn fan_box(column: Column) -> FanBox {
    let travel = column.flight.x;
    let lifted = travel * COLUMN_SPEED * column.shape.w * TIP_END;
    let radius = SPLASH_ROOT * column.shape.y
        + travel * COLUMN_SPEED * column.shape.w * SPLASH_REACH * (1.0 + column.flight.z);
    let foot = column.way.xy * min(column.flight.y, lifted);
    let head = column.way.xy * lifted;
    return FanBox(vec3<f32>(min(foot.x, head.x) - radius, -3.0, min(foot.y, head.y) - radius),
        vec3<f32>(max(foot.x, head.x) + radius, lifted - column.flight.y, max(foot.y, head.y) + radius));
}

// The light's way through the column's box from `start` (canonical, on its face toward the light), in `CASTER_STEPS`
// through its envelope: where its water lies on average along it (canonical m from `start`), the depth its caster
// writes its shadow at (`column_caster.wgsl`), and how much it stops (optical depth, canonical).
const CASTER_STEPS: i32 = 8;
struct Cast {
    half: f32,
    total: f32,
}

fn cast_through(start: vec3<f32>, way: vec3<f32>, column: Column, s: Splash) -> Cast {
    let fan = fan_box(column);
    let safe = select(way, vec3<f32>(1e-6), abs(way) < vec3<f32>(1e-6));
    let exit = max((fan.low - start) / safe, (fan.high - start) / safe);
    let run = max(min(min(exit.x, exit.y), exit.z), 0.0);
    let step = run / f32(CASTER_STEPS);
    var total = 0.0;
    var middle = 0.0;
    var t = step * 0.5;
    for (var i = 0; i < CASTER_STEPS; i++) {
        let water = column_envelope(start + way * t, s) * step;
        total += water;
        middle += water * t;
        t += step;
    }
    return Cast(middle / max(total, 1e-6), total);
}

// From water per unit of launch velocity to water per cubic metre: the flight scales velocities by the travel T, so a
// launch velocity's volume (speed² × its speed and solid angle) is T³ that of the place it reaches.
fn fan_scale(launch: Thrown, s: Splash) -> f32 {
    let travel = max(s.flight.x, 1e-3);
    return FAN_MASS / (max(launch.speed * launch.speed, 1.0) * travel * travel * travel);
}

// A cloud's erosion (Schneider's remap): where `noise` (0–1) is low, `amount` of the shape is cut away, so the shape's
// thick middle stays whole and its thin rim keeps only the billows' hearts.
fn erode(shape: f32, noise: f32, amount: f32) -> f32 {
    let cut = (1.0 - noise) * amount;
    return clamp((shape - cut) / max(1.0 - cut, 1e-3), 0.0, 1.0);
}

// A body's shape (0–1) before its billows, and its water per metre where the shape is whole.
struct Shape {
    shape: f32,
    density: f32,
}

// The veil.
fn veil_body(p: vec3<f32>, s: Splash) -> Shape {
    var out = Shape(0.0, 0.0);
    if s.veil.w <= 0.0 {
        return out;
    }
    let travel = max(s.veil.x, 1e-3);
    let o = vec3<f32>(p.x - s.drift.x, p.y + s.crown.x, p.z - s.drift.y);
    let lifted = o.y + s.veil.y;
    let share = lifted / (travel * COLUMN_SPEED * s.shape.x);
    if share <= -0.1 || share >= 1.15 {
        return out;
    }
    let swell = 1.0 + s.veil.z;
    // The mist the column sheds, not the skirt of water at its foot: the column's width without the skirt.
    let held = clamp(share, 0.0, 1.0);
    let radius = s.shape.y * (COLUMN_ROOT + travel * (FOOT_SPREAD + HEAD_SPREAD * s.shape.z * held * held * held))
        * VEIL_WIDTH * swell;
    let q = length(o.xz - s.lean * lifted) / radius;
    // While the column rises its veil trails its head; once shed it reaches as high as the head stood. Its foot thins
    // toward the sea.
    let top = mix(0.85, 1.12, min(s.veil.w, 1.0));
    let vertical = smoothstep(0.0, 0.3, share) * (1.0 - smoothstep(top - 0.25, top, share));
    out.shape = vertical * (1.0 - smoothstep(0.0, 1.0, q));
    out.density = VEIL_DENSITY * s.veil.w / (swell * swell);
    return out;
}

// The crown.
fn crown_body(p: vec3<f32>, s: Splash) -> Shape {
    var out = Shape(0.0, 0.0);
    if s.crown.w <= 0.0 {
        return out;
    }
    let travel = max(s.crown.y, 1e-3);
    let r = length(p.xz);
    let away = (r - COLUMN_ROOT * 0.75) / travel;
    let up = (p.y + s.crown.z) / travel;
    if up <= 0.0 || away <= 0.0 {
        return out;
    }
    let speed = length(vec2<f32>(away, up));
    let tilt = atan2(away, up);
    out.shape = exp(-square((tilt - CROWN_TILT) / CROWN_TILT_WIDTH)) * smoothstep(CROWN_SLOW * 0.4, CROWN_SLOW, speed)
        * (1.0 - smoothstep(CROWN_FAST * 0.6, CROWN_FAST, speed));
    out.density = CROWN_DENSITY * s.crown.w / (max(r, 2.0) * travel * travel);
    return out;
}

// The surge.
fn surge_body(p: vec3<f32>, s: Splash) -> Shape {
    var out = Shape(0.0, 0.0);
    if s.surge.w <= 0.0 {
        return out;
    }
    let o = p.xz - s.drift * 0.5;
    let r = length(o);
    // Its rim rolls out in lobes, not a ring.
    let lobes = noise_at(vec3<f32>(o.x / max(r, 1e-3) * 0.35, 0.37, o.y / max(r, 1e-3) * 0.35) + s.seed).r;
    let rim = exp(-square((r - s.surge.x * (0.7 + 0.6 * lobes)) / s.surge.y));
    let inside = (1.0 - smoothstep(s.surge.x - s.surge.y, s.surge.x + s.surge.y * 0.5, r)) * SURGE_INSIDE;
    // A roll of mist: thickest a third of its height up, its top rounded off.
    let up = max(p.y, 0.0) / s.surge.z;
    let roll = smoothstep(-0.2, 0.35, up) * (1.0 - smoothstep(0.3, 1.0, up));
    out.shape = max(rim, inside) * roll;
    out.density = SURGE_DENSITY * s.surge.w;
    return out;
}

// What the billows leave of a body on average, for the light marched through the envelope.
const ERODED: f32 = 0.55;

// How much of the light crossing the fan at a point finds its way between its feathers and strands: none at its foot,
// where the sheet has yet to tear, up to `GAPS` where it has fanned out and torn (the light's march is through the fan
// without its feathers, `column_envelope`: as a solid it shaded every strand within it).
fn fan_open(p: vec3<f32>, s: Splash) -> f32 {
    let launch = thrown_at(p, s);
    let share = launch.speed / (COLUMN_SPEED * s.shape.x);
    return GAPS * smoothstep(0.1, 0.45, share) * mix(0.7, 1.0, s.flight.w);
}

// The optical depth the light meets crossing a medium `open` of whose way is clear between its clumps: what its water
// stops spread through the rest, so a long way through many clumps still goes dark (a single layer of clumps would let
// the open share through however deep the medium, and lit the core as brightly as the sunlit side).
fn through_gaps(depth: f32, open: f32) -> f32 {
    return depth * (1.0 - open);
}

// The column's water at a point as the light sees it (optical depth per metre, canonical): the fan without its
// feathers and streaks, what the light is marched through toward the sun and the sky from the column (its thin mist
// barely shades it).
fn column_envelope(p: vec3<f32>, s: Splash) -> f32 {
    let launch = thrown_at(p, s);
    if launch.speed <= 0.0 {
        return 0.0;
    }
    return fan_scale(launch, s) * fan_spread(launch, s);
}

// The water at a point without its lumps, tears and lobes (optical depth per metre, canonical): what the light is
// marched through toward the sun and the sky.
fn envelope(p: vec3<f32>, s: Splash) -> f32 {
    let veil = veil_body(p, s);
    let crown = crown_body(p, s);
    let surge = surge_body(p, s);
    return column_envelope(p, s)
        + (veil.density * veil.shape + crown.density * crown.shape + surge.density * surge.shape) * ERODED;
}

fn noise_at(coord: vec3<f32>) -> vec4<f32> {
    return textureSampleLevel(column_noise, column_sampler, coord, 0.0);
}

// The water at a point (optical depth per metre, canonical), its lumps and streaks in place; `detail` (0–1) fades
// the noise to its mean where a pixel spans more than it resolves.
struct Water {
    // The column's water, and the thin mist's about it (the veil, the crown, the surge).
    column: f32,
    mist: f32,
    // How far the march may step from here (m canonical): finely through the column's fan, where a feather may lie,
    // and `far` beyond.
    stride: f32,
    // How far the point lies in a fold between the column's strands (0) or on one (1): those up-sun shade the folds.
    fold: f32,
    // How much of its fan's water the point's clump holds (1 the mean): the light reaching it is as much the less.
    clump: f32,
}

fn water_at(p: vec3<f32>, s: Splash, detail: f32, fine: f32, far: f32) -> Water {
    var water = Water(0.0, 0.0, far, 1.0, 1.0);
    let thrown = thrown_at(p, s);
    if thrown.speed > 0.0 {
        let scale = fan_scale(thrown, s);
        // Its water gathered into clumps along and across its feathers (the sheet tears unevenly), each thrown a little
        // faster or slower than the fan about it: from afar jets and spires rather than an even brush, and no clean
        // sphere where a speed ends (its foot's hollow, its tips). Fixed to the launches, they fly and fall with the water.
        let share0 = thrown.speed / (COLUMN_SPEED * s.shape.x);
        let clumps = noise_at(vec3<f32>(thrown.way.x * CLUMPS, share0 * CLUMPS_ALONG, thrown.way.z * CLUMPS) + s.seed.zxy).r;
        let ended = 1.0 + 2.0 * RAGGED * max(clumps - 0.5, 0.0);
        let launch = Thrown(thrown.way, thrown.speed * ended);
        // The feathers, and the water between them: the tallest, and of the ring about it the two either side of the
        // point's bearing (the ring stands evenly round from `kind.z`, so the rest are too far round to reach it).
        let spacing = 2.0 * PI / f32(FEATHERS - 1u);
        let round = (atan2(launch.way.z, launch.way.x) - columns[s.index].kind.z) / spacing;
        let before = u32(round - f32(FEATHERS - 1u) * floor(round / f32(FEATHERS - 1u))) % (FEATHERS - 1u);
        let after = (before + 1u) % (FEATHERS - 1u);
        let tallest = feather(thrown, columns[s.index].feathers[0], columns[s.index].tips[0].x * COLUMN_SPEED,
            s.flight.z, ended);
        var feathers = tallest.water;
        var edge = tallest.edge;
        var narrow = tallest.width;
        // Near the vertical the whole ring's cones may reach the point.
        let whole = 2.0 * (1.0 - launch.way.y) < WHOLE_RING * WHOLE_RING;
        for (var k = 0u; k < select(2u, FEATHERS - 1u, whole); k++) {
            let i = 1u + select(select(before, after, k == 1u), k, whole);
            let ring = feather(thrown, columns[s.index].feathers[i],
                columns[s.index].tips[i / 4u][i % 4u] * COLUMN_SPEED, s.flight.z, ended);
            feathers += ring.water;
            edge = min(edge, ring.edge);
            narrow = min(narrow, ring.width);
        }
        let between = mix(FOOT_BETWEEN, BETWEEN, smoothstep(0.15, 0.7, launch.speed / (COLUMN_SPEED * s.shape.x)));
        let spread = scale * between * fan_spread(launch, s);
        let thinnest = mix(CLUMP_FOOT, 0.0, smoothstep(0.4, 0.9, share0));
        let gathered = mix(thinnest, 2.0 - thinnest, smoothstep(0.3, 0.7, clumps));
        let mass = (scale * (1.0 - between) / f32(FEATHERS) * feathers + spread) * gathered;
        water.clump = gathered;
        // Between the feathers, on to the nearest one's edge (its angle round times the point's distance out); within
        // one, steps that each take at most `STRIDE_DEPTH` of the light, so finely through its dense core, longer through
        // thin water, but never longer than the narrowest feather about it is half wide nor `STRIDE_FAN` fine steps;
        // `far` beyond the fan.
        let out = thrown.speed * s.flight.x;
        if edge > 0.0 {
            water.stride = clamp(edge * out, fine, far);
        } else {
            water.stride = clamp(STRIDE_DEPTH / max(mass, 1e-4), fine, max(fine, min(fine * STRIDE_FAN, narrow * out)));
        }
        // The sheet tears into strands, the more as the column breaks up and toward its tips: water, not cotton. Its
        // water gathers into the strands (as much water, in fewer of them), until it is raining out. A pixel far off
        // spans many strands, and a step stopping too little light to see takes them as their mean (`mean`).
        let share = launch.speed / (COLUMN_SPEED * s.shape.x);
        // Its lower body stays whole until it breaks up: torn from its foot up, the eye saw into a fan the light took as
        // solid, a dark V either side of a bright bell.
        let threshold = mix(0.1, 0.42, smoothstep(0.2, 0.7, share)) + 0.25 * s.flight.w
            + 0.12 * smoothstep(0.5, 1.0, share);
        let mean = 1.0 - smoothstep(0.25, 0.75, threshold);
        water.column = mass * mean / max(mean, 0.35);
        if mass * water.stride > SEEN_DEPTH {
            // Streaks fixed to the water's direction and beaded along its speed: they fly, stretch and fall with it.
            let across = launch.way.xz * STREAKS;
            // Lumps along them: the same read's billows.
            let streak = noise_at(vec3<f32>(across.x, share * STREAK_ALONG, across.y) + s.seed);
            var threads = mix(streak.b * 0.6 + streak.a * 0.4, streak.r, 0.3);
            // Seen close, finer strands within them and their beads (a read along its speeds, `BEADS_ALONG` the
            // streaks' periods).
            if detail > 0.6 {
                let fine = noise_at(vec3<f32>(across.x * 2.6, share * BEADS_ALONG, across.y * 2.6) + s.seed.yzx);
                threads = mix(threads, mix(fine.a, fine.r, 0.4), 0.2 * detail);
            }
            let torn = smoothstep(threshold - 0.04, threshold + 0.04, threads);
            water.column = mass * mix(mean, torn, detail) / max(mean, 0.35);
            // The folds between them shade softly, a little more toward the tips: in full they combed the column into hair.
            water.fold = mix(1.0, smoothstep(0.25, 0.7, threads), detail * (0.25 + 0.25 * smoothstep(0.4, 0.9, share)));
        }
    }
    let veil = veil_body(p, s);
    if veil.shape > 0.0 {
        let o = p - vec3<f32>(s.drift.x, -s.crown.x, s.drift.y);
        // Its billows, and their small ones (the read's second channel, twice as fine).
        let billows = noise_at(o / 34.0 + s.seed.yzx);
        let cloud = billows.r * 0.62 + billows.g * 0.38;
        water.mist += veil.density * erode(veil.shape, mix(0.5, cloud, detail), 0.85);
    }
    let crown = crown_body(p, s);
    if crown.shape > 0.0 {
        // Fingers of water thrown out round the strike: the noise read in the direction each was launched, so it
        // streaks along their flight; torn into froth as they spread.
        let r = max(length(p.xz), 1e-3);
        let launched = normalize(vec3<f32>(p.x / r * (r - COLUMN_ROOT * 0.75), p.y + s.crown.z, p.z / r * (r - COLUMN_ROOT * 0.75)));
        let fingers = noise_at(launched * 3.0 + s.seed.zyx).b;
        let froth = noise_at(p / 9.0 + s.seed).g;
        water.mist += crown.density * erode(crown.shape, mix(0.5, fingers * 0.7 + froth * 0.3, detail), 0.95);
    }
    let surge = surge_body(p, s);
    if surge.shape > 0.0 {
        let o = p - vec3<f32>(s.drift.x * 0.5, 0.0, s.drift.y * 0.5);
        let billows = noise_at(o / 30.0 + s.seed.xzy);
        let rolls = billows.r * 0.65 + billows.g * 0.35;
        water.mist += surge.density * erode(surge.shape, mix(0.5, rolls, detail), 0.92);
    }
    return water;
}

fn medium(p: vec3<f32>, s: Splash, detail: f32) -> f32 {
    let water = water_at(p, s, detail, 1.0, 1.0);
    return water.column + water.mist;
}

// ---- A bow's burst of spray (`src/bow_bursts.rs`) ----
// The spray a bow throws as she drives into a sea: water thrown off her flare as a fan of launch velocities in the
// plane across her side (outboard and up), or up over her stem head (across her deck and up), drawn out along the
// stretch of her side that met the sea, slowed by its drag and keeping some of her way, so that as she runs on it falls
// astern of her bow. Read in the burst's own coordinates: x across the fan (outboard), y up, z aft along her side, from
// where its water left at the sea's level (m). As a splash's column, a point is one launch, found back from where it
// is: its launch velocity in the fan's plane is ((x, y + fall) less the wind's drift) / T, and the place along her side
// it left from is z less the drift plus what it has run on of her way. So its strands, fixed to the launches, lie along
// the water's flight, and the fan thins as it spreads and tears as it ages.

// One burst (`bow_bursts::BowBurst::state`): a splash's record, its lanes read as these.
struct Burst {
    t: f32,
    seed: vec3<f32>,
    // How far the wind has carried its water and its mist (across the fan, aft; m).
    drift: vec2<f32>,
    mist_drift: vec2<f32>,
    // Its water's travel T and fall (m), how far it has run on with her way (m), and how far it has torn (0–1).
    travel: f32,
    fall: f32,
    carried: f32,
    breakup: f32,
    // Its mist's: the water's travel and fall when the mist left the sheets, how far it has swollen (a share of
    // itself) and how much is left (0–1).
    mist_travel: f32,
    mist_fall: f32,
    swelling: f32,
    mist: f32,
    // How far the mist has settled (m), the stretch of her side that threw it (ahead of the origin and aft of it, m), and
    // how far the mist ran on with her way.
    settled: f32,
    lead: f32,
    length: f32,
    mist_carried: f32,
    // The fan: its fastest launch speed (m/s), its middle (radians up from outboard), its half width (radians), and how
    // much water it throws (0–1).
    speed: f32,
    elevation: f32,
    width: f32,
    strength: f32,
    // How far its water and its mist have spread along her side past the stretch that threw it (m), its slowest water's
    // share of its fastest's, and how far below the origin its water falls before the sea takes it (m).
    spread: f32,
    mist_spread: f32,
    slowest: f32,
    below: f32,
    // Which way the box frame's z runs along her side.
    along: f32,
}

fn burst_of(column: Column) -> Burst {
    let seed = column.high.w;
    return Burst(column.low.w, vec3<f32>(seed * 17.3, seed * 7.9, seed * 29.1), column.way.xy, column.way.zw,
        column.flight.x, column.flight.y, column.flight.z, column.flight.w,
        column.veil.x, column.veil.y, column.veil.z, column.veil.w,
        column.crown.x, column.crown.y, column.crown.z, column.crown.w,
        column.surge.x, column.surge.y, column.surge.z, column.surge.w,
        column.shape.x, column.shape.y, column.shape.z, column.shape.w, column.kind.y);
}

// The water a burst throws at full strength: optical depth per metre of a unit of its fan (of launch speed by launch
// angle) spread over a unit of travel. A fan of 19 m/s, 0.3 rad either way, holds some 80 m²/s² of launches within a
// width of its middle, about 12 T metres thick across: some 4 optical depths across it at 0.3 s, white, and 1 by a
// second, torn and thinning. Its mist's per metre at its densest.
const BURST_DENSITY: f32 = 0.12;
const BURST_MIST: f32 = 0.05;
// How much faster than its fastest its lumps are thrown, how far round the fan's middle its water reaches (in its
// widths), and how much wider its mist stands (`bow_bursts.rs` keeps the same).
const BURST_LUMPS: f32 = 0.12;
const FAN_REACH: f32 = 2.2;
const MIST_WIDE: f32 = 1.6;
// The bursts' drag (1/s, `bow_bursts::BURST_DRAG`).
const BURST_DRAG: f32 = 0.6;
// How the stretch's ends soften (m past its water's spread fore and aft; `bow_bursts::END_SOFT`).
const END_SOFT: f32 = 1.0;
// Along the stretch that threw it, the plunge met her flare from forward aft: its water left the stretch's after end
// this much later (s) than its fore end. It was thrown hardest about the origin, falling off toward the stretch's
// ends, and unevenly along it: `ALONG_LUMPS` the least and most of the fan's speed the lumps leave
// (`bow_bursts::ALONG_LUMPS` keeps the most).
const BURST_DELAY: f32 = 0.3;
const ALONG_LUMPS: array<f32, 2> = array<f32, 2>(0.7, 1.15);

// The flight of the water a burst threw at a place along her side: its travel T and fall (m), how far it has run on
// with her way (m), and the share of the fan's speed it was thrown at there (0: none yet).
struct Throw {
    travel: f32,
    fall: f32,
    carried: f32,
    reach: f32,
}

// For a point `z` m aft of the origin (the burst's coordinates), the throw of the water that left there.
fn throw_at(z: f32, b: Burst) -> Throw {
    var out = Throw(b.travel, b.fall, b.carried, 0.0);
    let start = -b.lead;
    let end = b.length;
    // Where along the stretch it left, by the burst's own flight (its own is a little later and shorter).
    let left = z - b.drift.y + b.carried;
    let t = b.t - BURST_DELAY * saturate((left - start) / max(end - start, 1.0));
    if t <= 0.0 {
        return out;
    }
    let travel = (1.0 - exp(-BURST_DRAG * t)) / BURST_DRAG;
    out.travel = travel;
    out.fall = GRAVITY / BURST_DRAG * (t - travel);
    out.carried = b.carried / max(b.travel, 1e-3) * travel;
    let hump = exp(-square(left / (0.6 * max(b.lead, b.length) + 2.0)));
    let lumps = noise_at(vec3<f32>(left * 0.035 + b.seed.x, b.seed.y * 0.37, b.seed.z * 0.53)).r;
    out.reach = (0.55 + 0.45 * hump) * mix(ALONG_LUMPS[0], ALONG_LUMPS[1], lumps);
    return out;
}

// Where a point's water left and how: its launch speed over the fan's fastest, its launch angle off the fan's middle
// (in its widths), and where along her side it left (m aft of the origin).
struct Launch {
    share: f32,
    angle: f32,
    along: f32,
}

// `q` in the burst's coordinates (its z aft); `travel`, `fall` and `carried` the flight it has flown, `reach` the share
// of the fan's speed it was thrown at.
fn burst_launch(q: vec3<f32>, travel: f32, fall: f32, carried: f32, drift: vec2<f32>, reach: f32, b: Burst) -> Launch {
    let w = vec2<f32>(q.x - drift.x, q.y + fall) / max(travel, 1e-3);
    return Launch(length(w) / (b.speed * reach), (atan2(w.y, w.x) - b.elevation) / b.width, q.z - drift.y + carried);
}

// How much of the fan's water left with a launch (0–1): the most about its middle, more of its slower water than its
// faster and none past its fastest, along the stretch of her side that threw it, its ends softening as its water
// spreads fore and aft (`spread` m; `bow_bursts::END_SOFT`).
fn fan(l: Launch, b: Burst, spread: f32) -> f32 {
    let across = exp(-l.angle * l.angle);
    let speed = smoothstep(b.slowest * 0.4, b.slowest, l.share) * (1.0 - smoothstep(0.78, 1.0, l.share))
        * mix(1.3, 0.5, saturate(l.share));
    let soft = spread + END_SOFT;
    let along = smoothstep(-b.lead - 2.0 * soft, -b.lead + soft, l.along)
        * (1.0 - smoothstep(b.length - soft, b.length + 2.0 * soft, l.along));
    return across * speed * along;
}

// How far past the stretch's ends a fan's water reaches (m), its ends softened by `spread`.
fn fan_ends(spread: f32) -> f32 {
    return 2.0 * (spread + END_SOFT);
}

// The water per metre a unit of the fan holds now: its mass over the area its launches have spread to (the travel
// squared, across the fan) and the stretch of her side it has drawn out along.
fn fan_mass(b: Burst, travel: f32, spread: f32) -> f32 {
    let stretch = max(b.lead + b.length, 1.0);
    // Thrown out of one point, its first instant would be all its water in none of the sea: it leaves over a tenth
    // of a second or so, as wide as it has spread by then.
    return BURST_DENSITY * b.strength / max(travel * travel, 0.02) * stretch / (stretch + 1.5 * spread);
}

// The burst's water at a point of its box (frame m) without its strands, tears and lumps (optical depth per metre):
// what the light is marched through toward the sun and the sky (its thin mist barely shades it).
fn burst_envelope(p: vec3<f32>, b: Burst) -> f32 {
    let q = vec3<f32>(p.x, p.y, p.z * b.along);
    if b.travel <= 1e-3 || q.y < -b.below {
        return 0.0;
    }
    let flown = throw_at(q.z, b);
    if flown.reach <= 0.0 {
        return 0.0;
    }
    let l = burst_launch(q, flown.travel, flown.fall, flown.carried, b.drift, flown.reach, b);
    return fan_mass(b, flown.travel, b.spread) * fan(l, b, b.spread);
}

// The burst's water at a point of its box (frame m), its strands, tears and lumps in place: the sheets, and the mist
// they shed (`Water.mist`); `fine` and `far` the march's steps inside the fan and outside it.
fn burst_water(p: vec3<f32>, b: Burst, detail: f32, fine: f32, far: f32) -> Water {
    var water = Water(0.0, 0.0, far, 1.0, 1.0);
    let q = vec3<f32>(p.x, p.y, p.z * b.along);
    if q.y < -b.below {
        return water;
    }
    let flown = throw_at(q.z, b);
    if b.travel > 1e-3 && b.strength > 0.0 && flown.reach > 0.0 {
        let l = burst_launch(q, flown.travel, flown.fall, flown.carried, b.drift, flown.reach, b);
        let ends = fan_ends(b.spread) + 2.0;
        if abs(l.angle) < FAN_REACH && l.share < 1.0 + BURST_LUMPS && l.along > -b.lead - ends && l.along < b.length + ends {
            water.stride = fine;
            // Strands along the water's flight (a launch's noise holds the length of its flight: fine across the fan,
            // long along its speeds) and clumps of spray between them, fixed to the launches that threw them.
            let angle = l.angle * b.width;
            let strand = noise_at(vec3<f32>(angle * 2.4, l.share * 0.3, l.along * 0.08) + b.seed);
            let lump = noise_at(vec3<f32>(angle * 1.1, l.share * 1.4, l.along * 0.05) + b.seed.zxy);
            let fibres = mix(0.5, strand.b * 0.6 + strand.a * 0.4, detail);
            let clumps = mix(0.5, lump.r * 0.7 + lump.g * 0.3, detail);
            // Its outline ragged: some of its water thrown faster than the rest, in fingers along its flight (round
            // lumps there stood over her deck as puffs), some wider, the stretch's ends torn.
            let fingers = mix(fibres, clumps, 0.25);
            let edged = Launch(l.share - 2.0 * BURST_LUMPS * (fingers - 0.5), l.angle - 0.6 * (fibres - 0.5),
                l.along + 4.0 * (clumps - 0.5));
            // It tears into strands and drops as it flies, its rim and its fastest water first.
            let rim = smoothstep(0.55, 1.0, l.share) + smoothstep(0.8, 1.8, abs(l.angle));
            let threshold = 0.1 + 0.42 * b.breakup + 0.2 * rim;
            let held = smoothstep(threshold - 0.08, threshold + 0.08, mix(fibres, clumps, 0.35));
            water.column = fan_mass(b, flown.travel, b.spread) * fan(edged, b, b.spread) * held * mix(0.55, 1.45, clumps);
            water.fold = smoothstep(0.2, 0.75, clumps);
        }
    }
    if b.mist > 0.0 {
        // The mist: the fan as it stood when the mist left the sheets, wider, swollen, settling and carried off on the
        // wind; its outline pushed about by billows a few metres across, billowed and eroded as a cloud, its billows
        // drawn out along her side.
        let o = vec3<f32>(q.x - b.mist_drift.x, q.y + b.settled, q.z - b.mist_drift.y);
        let swell = 1.0 + b.swelling;
        let warp = noise_at(o / 23.0 + b.seed.zyx);
        let pushed = o + (warp.xyz - 0.5) * (2.5 + 2.0 * b.swelling);
        let l = burst_launch(pushed, b.mist_travel * swell, b.mist_fall, b.mist_carried, vec2<f32>(0.0), 1.0, b);
        let shape = fan(Launch(l.share / 1.1, l.angle / MIST_WIDE, l.along), b, b.mist_spread);
        if shape > 1e-3 {
            water.stride = min(water.stride, far);
            let billows = noise_at(vec3<f32>(o.x / 7.0, o.y / 7.0, o.z / 16.0) + b.seed.yzx);
            let cloud = billows.r * 0.6 + billows.g * 0.4;
            water.mist = BURST_MIST * b.mist / (swell * swell) * erode(min(shape * 1.6, 1.0), mix(0.5, cloud, detail), 0.92);
        }
    }
    return water;
}

// ---- A bow's sheet of spray (`src/bow_sheets.rs`) ----
// The water a bow throws off her crest as she runs: from the stem aft along the crest that climbs her side, it leaves a
// little over the crest, still climbing at the speed that carries it to the crest's peak and outboard a little faster
// than the crest line leaves her track, slowed by its drag; it passes her at her way and falls back onto the crest's
// outer face astern of where it left. It stands steady about her, so it is read in her own coordinates: x outboard
// square to her centre line, y up, z aft, from her stem at the sea's level (m). The water now `z` aft of the stem left
// a station s of the crest (z − s) / U seconds ago (from where the station's water leaves: her stem's rake and her trim
// move it fore and aft), U its speed aft past her: a point's cross-section is the water of every station from the stem
// to it. Its stations' water lies close together over the crest, so the cross-section is
// drawn as the line through its youngest, middle and oldest water, the water spread about it as far as its launches'
// spread has carried it. A point's water is known by the station and the moment it left, so its strands, fixed to
// them, lie along its flight and stream aft with it, and it tears as it ages.

// One sheet (`bow_sheets::BowSheet::state`): a splash's record, its lanes read as these.
struct Sheet {
    // Its clock (s, wrapped at `bow_sheets::CLOCK_WRAP`), and its number's seeds.
    clock: f32,
    seed: vec3<f32>,
    // At its four stations (the stem, a third, two thirds and the whole of the stretch that throws): how far out of her
    // centre line the crest stands (m), how high over the origin its water leaves (m), and how fast it climbs (m/s).
    lines: vec4<f32>,
    heights: vec4<f32>,
    climbs: vec4<f32>,
    // What the seas she met added to its climb (m/s), by the water's age: now, and `KICK_AGE`, twice and three times
    // that ago.
    kicks: vec4<f32>,
    // How far aft of its station (m; forward, less than nothing) each station's water leaves: her stem's rake at the
    // height it leaves, and her trim.
    shifts: vec4<f32>,
    // Its speed outboard at the stem and at the stretch's end (m/s), its speed aft past her (m/s), and the stretch that
    // throws it (m aft of the stem).
    out_stem: f32,
    out_end: f32,
    aft: f32,
    reach: f32,
    // How much water it throws (0–1), and how far below the origin its water falls before the sea takes it (m).
    strength: f32,
    below: f32,
    // Which way the box frame's z runs along her side.
    along: f32,
}

fn sheet_of(column: Column) -> Sheet {
    let seed = column.high.w;
    return Sheet(column.low.w, vec3<f32>(seed * 17.3, seed * 7.9, seed * 29.1), column.way, column.flight, column.veil,
        column.crown, column.shape, column.surge.x, column.surge.y, column.surge.z, column.surge.w, column.kind.z,
        column.kind.w, column.kind.y);
}

// The sheets' drag (1/s, `bow_spray`'s `SHEET_DRAG`), the oldest water drawn (s; it fades over the last of it), the
// water's spread about its line where it leaves (m), how far a metre of its flight spreads it every way (a share of its
// launch speed) and how far its climb spreads it up and down (a share of the climb: some of it is thrown higher, some
// lower), and its water per metre at full strength: optical depth per metre for a second's worth of the stretch's water
// over a square metre of the cross-section (`sheet_mass`). `bow_sheets.rs` keeps the same.
const SHEET_DRAG: f32 = 0.6;
const SHEET_AGE: f32 = 2.2;
const SHEET_FADE: f32 = 1.3;
const SHEET_ROOT: f32 = 0.08;
const SHEET_SPREAD: f32 = 0.06;
const SHEET_CLIMB_SPREAD: f32 = 0.12;
const SHEET_DENSITY: f32 = 3.0;
// Its water ends a spread from its line, its fingers throwing it up to `SHEET_FINGERS` spreads farther (or short); where
// it leaves the crest, a film, it holds this share of its water's spray.
const SHEET_FINGERS: f32 = 0.45;
const SHEET_FILM: f32 = 0.3;
// The march's finest step through it (m).
const SHEET_STEP: f32 = 0.08;
// The kicks' ages (s apart).
const KICK_AGE: f32 = 0.4;
// Its strands' noise: per second of the moment its water left (along the flight) and per metre of its station (across
// it).
const SHEET_FLOW: f32 = 1.4;
const SHEET_ACROSS: f32 = 0.3;

// A value at the four stations, at a share of the way from the first to the last, in line between them.
fn at_station(v: vec4<f32>, share: f32) -> f32 {
    let f = saturate(share) * 3.0;
    let a = mix(v.x, v.y, saturate(f));
    let b = mix(a, v.z, saturate(f - 1.0));
    return mix(b, v.w, saturate(f - 2.0));
}

// The water that left station `s` (m aft of the stem) and is now `z` m aft (z ≥ s): where it is across her side (x out,
// y up; m), how far it has spread about its line across her side and up and down (m), and its age (s).
struct Parcel {
    at: vec2<f32>,
    spread: vec2<f32>,
    age: f32,
}

fn sheet_parcel(z: f32, s: f32, sh: Sheet) -> Parcel {
    let share = s / max(sh.reach, 1e-3);
    let t = max(z - s - at_station(sh.shifts, share), 0.0) / max(sh.aft, 0.5);
    let travel = (1.0 - exp(-SHEET_DRAG * t)) / SHEET_DRAG;
    let fall = GRAVITY / SHEET_DRAG * (t - travel);
    let out = mix(sh.out_stem, sh.out_end, saturate(share));
    let up = at_station(sh.climbs, share) + at_station(sh.kicks, t / (3.0 * KICK_AGE));
    let at = vec2<f32>(at_station(sh.lines, share) + out * travel, at_station(sh.heights, share) + up * travel - fall);
    let spread = SHEET_ROOT + SHEET_SPREAD * length(vec2<f32>(out, up)) * travel;
    return Parcel(at, vec2<f32>(spread, spread + SHEET_CLIMB_SPREAD * max(up, 0.0) * travel), t);
}

// The cross-section `z` m aft of the stem: its youngest water (from the station abreast of it, or the stretch's end),
// its middle and its oldest (from the stem, or the oldest still drawn), and the stations they left (m aft of the stem).
struct Cross {
    young: Parcel,
    middle: Parcel,
    old: Parcel,
    near: f32,
    far: f32,
}

fn sheet_cross(z: f32, sh: Sheet) -> Cross {
    // The station whose water leaves abreast of `z`, and the one whose oldest water drawn is there (its shift read once
    // where it would be unshifted: the shifts change slowly along the stretch).
    let reach = max(sh.reach, 1e-3);
    let near = clamp(z - at_station(sh.shifts, z / reach), 0.0, sh.reach);
    let oldest = z - sh.aft * SHEET_AGE;
    let far = clamp(oldest - at_station(sh.shifts, oldest / reach), 0.0, near);
    return Cross(sheet_parcel(z, near, sh), sheet_parcel(z, 0.5 * (near + far), sh), sheet_parcel(z, far, sh), near,
        far);
}

// Where a point across her side (x out, y up; m) lies by a cross-section's line: how far along it (0 at its youngest
// water, 1 at its oldest), its offset from the line (m), and the line's way there (unit).
struct OnLine {
    along: f32,
    off: vec2<f32>,
    way: vec2<f32>,
}

fn on_line(p: vec2<f32>, c: Cross) -> OnLine {
    let first = c.middle.at - c.young.at;
    let second = c.old.at - c.middle.at;
    let u = saturate(dot(p - c.young.at, first) / max(dot(first, first), 1e-6));
    let v = saturate(dot(p - c.middle.at, second) / max(dot(second, second), 1e-6));
    let a = p - (c.young.at + first * u);
    let b = p - (c.middle.at + second * v);
    if dot(a, a) <= dot(b, b) {
        return OnLine(0.5 * u, a, normalize(first + vec2<f32>(1e-6, 0.0)));
    }
    return OnLine(0.5 + 0.5 * v, b, normalize(second + vec2<f32>(1e-6, 0.0)));
}

// A cross-section's value `along` its line, in line between its youngest, middle and oldest water's.
fn along_line(young: f32, middle: f32, old: f32, along: f32) -> f32 {
    return mix(mix(young, middle, saturate(along * 2.0)), old, saturate(along * 2.0 - 1.0));
}

// The water per metre on a cross-section's line at full (optical depth): the stretch's water come this far aft (its
// length over its speed aft past her: the seconds' worth of it) over the cross-section's area (along the line and its
// spread either side of it) (`bow_sheets::sheet_mass` keeps the same).
fn sheet_mass(c: Cross, spread: vec2<f32>, sh: Sheet) -> f32 {
    let line = length(c.middle.at - c.young.at) + length(c.old.at - c.middle.at);
    let wide = sqrt(spread.x * spread.y);
    return SHEET_DENSITY * sh.strength * (c.near - c.far) / max(sh.aft, 0.5) / ((line + 1.5 * wide) * 1.5 * wide);
}

// How much of its water lies `r` spreads from its line (its fingers in place): its body whole to half way, thinning to
// none at a spread.
fn sheet_profile(r: f32) -> f32 {
    return 1.0 - smoothstep(0.45, 1.0, r);
}

// What the oldest water keeps as it fades.
fn sheet_ageing(age: f32) -> f32 {
    return 1.0 - smoothstep(SHEET_FADE, SHEET_AGE, age);
}

// A point's place by a cross-section: on its line, and its spread and age there.
struct AtSheet {
    on: OnLine,
    spread: vec2<f32>,
    age: f32,
    // Its offset from the line in its spreads.
    r: f32,
}

fn at_sheet(p: vec2<f32>, c: Cross) -> AtSheet {
    let on = on_line(p, c);
    let spread = vec2<f32>(along_line(c.young.spread.x, c.middle.spread.x, c.old.spread.x, on.along),
        along_line(c.young.spread.y, c.middle.spread.y, c.old.spread.y, on.along));
    let age = along_line(c.young.age, c.middle.age, c.old.age, on.along);
    return AtSheet(on, spread, age, length(on.off / spread));
}

// The sheet's water at a point of its box (frame m) without its strands, tears and lumps (optical depth per metre):
// what the light is marched through toward the sun and the sky.
fn sheet_envelope(p: vec3<f32>, sh: Sheet) -> f32 {
    let q = vec3<f32>(p.x, p.y, p.z * sh.along);
    if q.y < -sh.below || q.z <= 0.0 || sh.strength <= 0.0 {
        return 0.0;
    }
    let c = sheet_cross(q.z, sh);
    if c.near <= c.far {
        return 0.0;
    }
    let at = at_sheet(q.xy, c);
    return sheet_mass(c, at.spread, sh) * sheet_profile(at.r) * sheet_ageing(at.age) * ERODED;
}

// The sheet's water at a point of its box (frame m), its strands, tears and lumps in place; `fine` and `far` the march's
// steps through it and outside it. It sheds no mist of its own: in calm water a bow's sheet throws little fine spray
// that hangs (a plunge's bursts shed theirs), and drawn as a halo about it, it stood as smoke along her side.
fn sheet_water(p: vec3<f32>, sh: Sheet, detail: f32, fine: f32, far: f32) -> Water {
    var water = Water(0.0, 0.0, far, 1.0, 1.0);
    let q = vec3<f32>(p.x, p.y, p.z * sh.along);
    if q.z <= 0.0 {
        // Ahead of her stem the sea is undisturbed: step on to it.
        water.stride = clamp(-q.z, fine, far);
        return water;
    }
    if q.y < -sh.below || sh.strength <= 0.0 {
        return water;
    }
    let c = sheet_cross(q.z, sh);
    if c.near <= c.far {
        return water;
    }
    let at = at_sheet(q.xy, c);
    let station = mix(c.near, c.far, at.on.along);
    let launched = sh.clock - at.age;
    // Across it: the offset either side of its line, in its spreads.
    let across = dot(at.on.off / at.spread, vec2<f32>(-at.on.way.y, at.on.way.x));
    // Its line bends gently aft: outside its water the march steps a little short of the way to it.
    let reach = (1.0 + SHEET_FINGERS) * max(at.spread.x, at.spread.y);
    let distance = length(at.on.off);
    water.stride = clamp(0.7 * (distance - reach), fine, far);
    if distance < reach {
        // Finer where it is thin, but never so fine that a ray along it spends its steps before it is through.
        water.stride = min(fine, max(0.4 * min(at.spread.x, at.spread.y), SHEET_STEP));
        // Strands along its flight (the station across them, the moment it left along them) and clumps of spray
        // between them, fixed to the water that threw them.
        let strand = noise_at(vec3<f32>(station * SHEET_ACROSS, launched * SHEET_FLOW, across * 2.0) + sh.seed);
        let lump = noise_at(vec3<f32>(station * SHEET_ACROSS * 1.5, launched * SHEET_FLOW * 2.5, across * 0.8)
            + sh.seed.zxy);
        let fibres = mix(0.5, strand.b * 0.6 + strand.a * 0.4, detail);
        let clumps = mix(0.5, lump.r * 0.7 + lump.g * 0.3, detail);
        // Its outline ragged: some of its water thrown farther, in fingers along its flight, some short.
        let fingers = mix(fibres, clumps, 0.3);
        let r = max(at.r - 2.0 * SHEET_FINGERS * (fingers - 0.5), 0.0);
        // It tears into strands and drops as it flies, its rim first; its rim keeps only the clumps' hearts.
        let breakup = smoothstep(0.1, 1.1, at.age);
        let rim = smoothstep(0.5, 1.0, r);
        let threshold = 0.36 + 0.3 * breakup + 0.2 * rim;
        let held = smoothstep(threshold - 0.07, threshold + 0.07, mix(fibres, clumps, 0.35));
        let body = erode(sheet_profile(r), clumps, 0.35 + 0.4 * breakup);
        // It swells and slackens with the short seas she meets, each swell running aft with the water.
        let pulse = noise_at(vec3<f32>(launched * 0.5, sh.seed.y, sh.seed.z)).b;
        // Where it leaves the crest it is a thin film; it whitens as it breaks up into spray.
        let film = mix(SHEET_FILM, 1.0, smoothstep(0.0, 0.35, at.age));
        water.column = sheet_mass(c, at.spread, sh) * body * held * mix(0.35, 1.65, fibres) * mix(0.6, 1.4, clumps)
            * mix(0.7, 1.3, pulse) * film * sheet_ageing(at.age);
        water.fold = smoothstep(0.2, 0.75, clumps);
    }
    return water;
}
