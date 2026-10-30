// What the ocean surface reads of the wakes (the web's `WakeSampler` as `ShipWake.ts` binds it with the
// realistic trail): the dispersive wake field's height, slope and breaking foam (the web game's `ocean/wake/WakeField.ts`,
// `createSampler`), the analytic bow waves (the web game's `BowWaves.ts`, `evaluate`), the fleet's trail foam, bubbles and slick
// (the web game's `FleetWakeFoam.ts`, `evaluate`), the torpedo tracks (`TorpedoTrackFoam.ts`, `sample`) and the hull
// contact foam (the web game's `HullContactFoam.ts`, `evaluate`).
//
// Its bindings join the surface's own group 1 (bindings 5–10, and 17 for the contact foam's slap): the surface already
// uses the four bind groups WebGPU guarantees (camera, surface, waves, sky). One uniform buffer packs every system's
// arrays (the removed web game's `PackedVec4Arrays` layout, which kept them under its per-stage uniform buffer limit).

#define_import_path naval_ocean::wake

#import naval::camera::camera
#import naval_ocean::mx_noise::mx_noise_float
#import naval_ocean::noise::{noise_hash, noise_seed, gradient_noise}

const TILES: f32 = 8.0;
const TILE_COUNT: i32 = 64;
const WAKE_EXTENT: f32 = 1536.0;
const SLICK_EXTENT: f32 = 4096.0;
const EDGE_FADE: f32 = 0.03;
const BOW_SLOTS: i32 = 8;
const TAU: f32 = 6.283185307179586;
// Hull profiles (hull_profile.rs) and the contact foam's per-slot layout (hull_foam.rs).
const PROFILE_STATIONS: i32 = 32;
const PROFILE_LEVELS: i32 = 7;
const PROFILE_LEVEL_STEP: f32 = 2.0;
const PROFILE_LOWEST: f32 = -6.0;
const HULL_SLAP: i32 = 7;
const HULL_PROFILE: i32 = 8;
const HULL_STRIDE: i32 = 264;

struct Wake {
    // The wake field's display textures: origin of display 0 (xy) and 1 (zw); weights, world size; w: how fast the
    // water mesh's cells grow with distance from its centre (m per m).
    field_origins: vec4<f32>,
    field: vec4<f32>,
    // x: trail tiles in use, y: the trail's clock, z: the torpedo field's clock, w: 1 when torpedo foam is painted.
    trail: vec4<f32>,
    // Every trail tile's painted extent together (minX, minZ, maxX, maxZ).
    trail_reach: vec4<f32>,
    // xy: the torpedo field's centre, zw: the water mesh's centre (the eye, or what a magnified view is aimed at).
    torpedo: vec4<f32>,
    // The trail's shading knobs (trail.rs `WakeShading`); [3].w: 1 while the water mesh is a magnified view's fan.
    shading: array<vec4<f32>, 4>,
    // x: bow wave slots in use, y: their clock, z: radians per pixel, w: the cell of the water mesh's finest level
    // drawn; under a magnified view's fan, how fast its far rows' cells grow with the square of their range (1 / m).
    bow: vec4<f32>,
    bow_knobs: array<vec4<f32>, 4>,
    // x: contact foam slots in use, y: its clock, z: radians per pixel, w: the bow's extra foam length share.
    hull: vec4<f32>,
    // Trail tiles, section-major: centre and atlas tile, slick centre, churned-water box, slick box, both.
    tiles: array<vec4<f32>, 320>,
    // Bow wave slots, section-major: pose, wave, hull, bend, entrance, course, stern.
    bow_slots: array<vec4<f32>, 56>,
    hull_data: array<vec4<f32>, 2112>,
    // The oil slicks (naval_ocean::oil, src/oil.rs): x slots in use, y and z the edges (m) of each slick's fine and
    // wide squares.
    oil: vec4<f32>,
    // Every oil slot's painted extent together (minX, minZ, maxX, maxZ).
    oil_reach: vec4<f32>,
    // xy: the surface's drift (m/s) that carries the oil, zw: how far it has gone (m, wrapped).
    oil_flow: vec4<f32>,
    // Oil slots, section-major: the fine square's centre and the atlas tile, the wide square's centre, what each
    // painted, both.
    oil_tiles: array<vec4<f32>, 45>,
    // The splashes' deposits (naval_ocean::splash_foam, src/splash_foam.rs), youngest first, two rows each: where the
    // deposit lies now (xy), its age (s) and its round's scale; its seed (0–1), its skew downwind (m) and how far from
    // xy it reaches.
    splashes: array<vec4<f32>, 128>,
    // x the deposits in use; y the foam field's newest state (its layer), z 1 once there is a field
    // (naval_ocean::foam_sim, src/foam_sim.rs).
    foam_state: vec4<f32>,
    // The foam field's window, last: its centre cell's corner (world m, xy) and that cell's texel (zw); a cell's edge
    // (m) and the cells along an edge.
    sim: vec4<f32>,
    sim_grid: vec4<f32>,
}

@group(1) @binding(5) var<uniform> wake: Wake;

// Half the edge of the water mesh's outermost level, where its horizon ring takes over (`geometry::CLIPMAP_REACH`).
const MESH_REACH: f32 = 4096.0;

// The water mesh's vertex spacing at a world position (m), as `surface.wgsl`'s vertex stage has it: the cell of the
// level that holds the point (each level's half edge half the next's, its cell `wake.field.w` of it), morphed onto the
// next level's toward its edge; never finer than the finest level drawn; on the horizon ring beyond the lattice,
// `geometry.rs`'s 4 r / segments. A magnified view's fan (`geometry::Fan::spacing`, `wake.shading[3].w` set) has
// cells `wake.field.w` of their range from its centre, the eye, or `wake.bow.w` of its square where that is more. No
// TypeScript source.
fn mesh_spacing(xz: vec2<f32>) -> f32 {
    let off = xz - wake.torpedo.zw;
    if wake.shading[3].w > 0.5 {
        let range = length(off);
        return range * max(wake.field.w, wake.bow.w * range);
    }
    let reach = max(abs(off.x), abs(off.y));
    if reach >= MESH_REACH {
        return max(wake.bow.w, 2.0 * wake.field.w * length(off));
    }
    let half = MESH_REACH * exp2(-floor(log2(MESH_REACH / max(reach, 1e-3))));
    return max(wake.bow.w, wake.field.w * half * (1.0 + smoothstep(0.6, 0.85, reach / half)));
}
@group(1) @binding(6) var wake_display_a: texture_2d<f32>;
@group(1) @binding(7) var wake_display_b: texture_2d<f32>;
@group(1) @binding(8) var wake_sampler: sampler;
@group(1) @binding(9) var trail_atlas: texture_2d<f32>;
@group(1) @binding(10) var torpedo_field: texture_2d<f32>;
// The water's memory against the remembered hulls' sides (`naval_ocean::hull_sides`, the heights' pass's own): per
// slot, side (port, starboard) and station from her stem aft, (now, climb, slap, rise). A contact slot's slap row names
// her slot in it; read by the fragment stage alone (`side_slap`).
@group(1) @binding(17) var<storage, read> hull_side_state: array<vec4<f32>>;
// The foam field's two states (src/foam_sim.rs, shaders/foam_sim.wgsl): (thickness, turbulence, residue, validity).
@group(1) @binding(18) var foam_sim: texture_2d_array<f32>;

// ---- The wake field -------------------------------------------------------------------------------------------------

fn edge_fade(uv: vec2<f32>) -> f32 {
    return smoothstep(0.0, EDGE_FADE, uv.x) * smoothstep(0.0, EDGE_FADE, 1.0 - uv.x) * smoothstep(0.0, EDGE_FADE, uv.y)
        * smoothstep(0.0, EDGE_FADE, 1.0 - uv.y);
}

// Both displays at a world point, blended by the step fraction and the gain; calm outside the field.
// (h, ∂h/∂x, ∂h/∂z, foam).
fn wake_field(xz: vec2<f32>) -> vec4<f32> {
    let size = max(wake.field.z, 1.0);
    let uv_a = (xz - wake.field_origins.xy) / size + 0.5;
    let uv_b = (xz - wake.field_origins.zw) / size + 0.5;
    let a = textureSampleLevel(wake_display_a, wake_sampler, uv_a, 0.0) * edge_fade(uv_a);
    let b = textureSampleLevel(wake_display_b, wake_sampler, uv_b, 0.0) * edge_fade(uv_b);
    return a * wake.field.x + b * wake.field.y;
}

// ---- Bow waves ------------------------------------------------------------------------------------------------------

struct BowWaveSample {
    height: f32,
    slope: vec2<f32>,
    foam: f32,
    bubbles: f32,
    // How young the white water is: 1 where the crest breaks, less where it only left foam behind.
    young: f32,
    // The slope variance of the V's crests too short for the pixel to draw: from aloft and at range an arm still reads
    // as a line of rougher water after its crests have gone below the pixel.
    variance: f32,
}

// The entrance the crest follows: this share of the hull's length aft of the stem (`bow_waves.rs`, `ENTRANCE`).
const BOW_ENTRANCE: f32 = 0.3;
// The crest's climb starts this share of its peak's distance ahead of the stem (`CLIMB_LEAD`); the water standing
// against the bow's sides (`HEAP_REACH`, `HEAP_WIDTH`, `HEAP_SPREAD`).
const CLIMB_LEAD: f32 = 0.15;
const HEAP_REACH: f32 = 0.07;
const HEAP_WIDTH: f32 = 0.2;
const HEAP_SPREAD: f32 = 0.12;
// The crest's inboard side is this many times as broad as its outer face: a breaking front outboard, the water it has
// passed falling away gently toward the hull.
const CREST_INNER: f32 = 2.2;
// The trough behind the crest: this share of its height, centred this many of the outer face's widths inboard of the
// crest line (`bow_waves.rs` takes what it leaves of the crest on its own line).
const CREST_TROUGH: f32 = 0.25;
const CREST_TROUGH_INBOARD: f32 = 3.2;
// What the crest's breaker leaves on the water it has passed lasts this share of the hull's length downstream, and the
// white water the surface draws from it is this young (lace, not a breaker's dense white).
const BOW_RESIDUE_LENGTH: f32 = 0.16;
const BOW_RESIDUE: f32 = 0.75;
const BOW_RESIDUE_YOUNG: f32 = 0.7;
// How hard the streaks tear the white water: a breaker's dense core least, what it left behind most.
const BOW_TEAR: f32 = 0.45;
const BOW_LACE_TEAR: f32 = 1.0;
// The Kelvin V's cusp line, where its two wave systems meet at 1/√8 (19.47°) off the track. How far inside it a point
// lies is the Airy function's argument, CUSP_ZETA (k₀·run)^(2/3) (1/√8 − t), t the point's offset over its run (the
// systems' phase difference, linearised about the cusp): outside the line no wave stands and the sea settles within a
// few metres at any range; inside it the first fringe, the line itself, keeps its full slope (the Airy function peaks
// at 1.02 and first falls to nothing at 2.34) and the fringes within it `kelvin_interior` of theirs. From aloft the V
// reads as two thin lines of short crests in echelon, a band some 20 m wide 300 m astern of a hull at 15 m/s and 40 m
// wide 2 km astern, not a fan of ripples (`bow_waves.rs`, `kelvin_interior`).
const CUSP_T: f32 = 0.35355339;
const CUSP_ZETA: f32 = 1.9645;
const CUSP_LINE: f32 = 1.0;
const CUSP_LINE_WIDTH: f32 = 1.2;
// The line's slopes a little steeper than the V's elsewhere: its crests are where the two systems pile up.
const CUSP_LINE_GAIN: f32 = 1.15;
// The crests' slope variance the pixel cannot draw, as a share of what it would shade, and how patchy it is along the
// arm (the sea's own waves break the line up): a far arm reads as a line of rougher water, not a stroke of paint.
const KELVIN_ROUGHNESS: f32 = 0.12;
// Past a few hull lengths astern only the cusp lines are drawn: the water between them, quiet there and low, fades out
// over these Airy arguments inside the lines, from two to four hull lengths astern. A long V then costs two narrow bands
// of sea, not the whole fan.
const CUSP_INTERIOR_FROM: f32 = 6.0;
const CUSP_INTERIOR_TO: f32 = 9.0;
// The V's crests are drawn down to this share of the five pixels the crest needs (`finest`): they stand still in a view
// that follows the hull, and a far arm keeps its dashes.
const KELVIN_FINEST: f32 = 0.6;
const KELVIN_ROUGHNESS_PATCH: f32 = 180.0;

// The waterline's half-breadth `along` metres astern of the stem and how fast it grows (m per m): through the hull's own
// breadths an eighth, a quarter, a half and the whole of the `entrance` aft of it (`bow_waves.rs`, `entrance`), then
// out to her `full` half-breadth. No TypeScript source: `BowWaves.ts` assumed a parabola.
fn entrance_breadth(along: f32, entrance: f32, nodes: vec4<f32>, full: f32) -> vec2<f32> {
    let u = along / entrance;
    if u < 0.125 {
        return vec2<f32>(nodes.x * u * 8.0, nodes.x * 8.0 / entrance);
    }
    if u < 0.25 {
        return vec2<f32>(mix(nodes.x, nodes.y, (u - 0.125) * 8.0), (nodes.y - nodes.x) * 8.0 / entrance);
    }
    if u < 0.5 {
        return vec2<f32>(mix(nodes.y, nodes.z, (u - 0.25) * 4.0), (nodes.z - nodes.y) * 4.0 / entrance);
    }
    if u < 1.0 {
        return vec2<f32>(mix(nodes.z, nodes.w, (u - 0.5) * 2.0), (nodes.w - nodes.z) * 2.0 / entrance);
    }
    // Aft of the entrance she fills out to her full breadth by 0.6 of her length.
    let t = clamp(u - 1.0, 0.0, 1.0);
    let more = max(full, nodes.w) - nodes.w;
    return vec2<f32>(nodes.w + more * t * t * (3.0 - 2.0 * t), more * 6.0 * t * (1.0 - t) / entrance);
}

// World-anchored streaks: white water flows aft past the hull, instead of riding with it as a painted band.
fn bow_foam_noise(xz: vec2<f32>, heading: vec2<f32>, finest: f32) -> f32 {
    let downstream = xz.x * heading.x - xz.y * heading.y;
    let abeam = xz.x * heading.y + xz.y * heading.x;
    let time = wake.bow.y;
    let strands = mx_noise_float(vec3<f32>(downstream * 0.02, abeam * 0.3, time * 0.25))
        * (1.0 - smoothstep(4.0, 10.0, finest));
    let lace = mx_noise_float(vec3<f32>(downstream * 0.06 + 7.0, abeam * 0.55, time * 0.4))
        * (1.0 - smoothstep(2.0, 5.5, finest));
    let fleck = mx_noise_float(vec3<f32>(downstream * 0.35 + 3.0, abeam * 0.9, time * 0.9))
        * (1.0 - smoothstep(1.2, 3.5, finest));
    return (strands * 0.5 + lace * 0.4 + fleck * 0.2) * wake.bow_knobs[3].z;
}

// The stern's waves (`bow_waves.rs`, `stern_crest`; 2026-10-23), as shares of the stern's crest. Her quarters sit in a
// trough, STERN_TROUGH deep about STERN_TROUGH_AT of her length forward of her stern and STERN_TROUGH_LENGTH of it long,
// fading out from her side over 0.3 of her beam and 2 m. Behind her stern the water falls into a hollow, STERN_HOLLOW
// deep and a little wider than her transom, and closes on her centreline in the hump `peak` metres astern, steep in
// front and long behind (STERN_HUMP_FRONT and STERN_HUMP_BACK of `peak`). Her quarter waves leave her side
// QUARTER_FROM of her length forward of her stern and run out at QUARTER_ANGLE (a tangent: 23°), QUARTER_HEIGHT high,
// with a steep outer face and a gentle inner one, gone a little over half her length astern.
const STERN_TROUGH: f32 = 0.35;
const STERN_TROUGH_AT: f32 = -0.06;
const STERN_TROUGH_LENGTH: f32 = 0.1;
const STERN_HOLLOW: f32 = 0.55;
const STERN_HUMP_FRONT: f32 = 0.42;
const STERN_HUMP_BACK: f32 = 1.1;
const QUARTER_FROM: f32 = -0.04;
const QUARTER_ANGLE: f32 = 0.42;
const QUARTER_HEIGHT: f32 = 0.6;
// Where the course row's quarter half-breadth was read, as a share of her length forward of her stern (`STERN_QUARTER`).
const STERN_QUARTER: f32 = 0.12;
// What a quarter wave's breaker leaves on the water inboard of it, as a share of its white ribbon, and how young that is
// (lace, not a breaker's dense white).
const STERN_RESIDUE: f32 = 0.7;
const STERN_RESIDUE_YOUNG: f32 = 0.75;

struct SternSample {
    // The sea's height, a share of the stern's crest.
    height: f32,
    // The hump's own share of it; the quarter waves' white ribbon along their outer faces and what their breakers left
    // inboard of them (0–1).
    hump: f32,
    ribbon: f32,
    residue: f32,
}

// The stern's waves `s` metres astern of her stern (negative forward of it) and `ay` abeam of her centreline. `transom`
// and `quarter` are her half-breadths just forward of her stern and STERN_QUARTER of her length forward of it; `blur`
// the narrowest feature the stage holds, which draws anything narrower broader and lower. No TypeScript source.
fn stern_shape(s: f32, ay: f32, peak: f32, transom: f32, quarter: f32, length: f32, beam: f32, blur: f32) -> SternSample {
    // Her side over her run, from her quarter's breadth in to her transom's.
    let quarter_at = STERN_QUARTER * length;
    let side = mix(quarter, transom, clamp((s + quarter_at) / quarter_at, 0.0, 1.0));
    let trough_along = (s - STERN_TROUGH_AT * length) / (STERN_TROUGH_LENGTH * length);
    let trough_out = max(ay - side, 0.0) / (beam * 0.3 + 2.0);
    let trough = -STERN_TROUGH * exp(-(trough_along * trough_along + trough_out * trough_out));
    let hollow_width = transom + 1.0 + max(s, 0.0) * 0.15;
    let hollow_drawn = max(hollow_width, blur);
    let hollow_along = (s - 0.3 * peak) / (0.38 * peak);
    let hollow_across = ay / hollow_drawn;
    let hollow = -STERN_HOLLOW * exp(-(hollow_along * hollow_along + hollow_across * hollow_across))
        * (hollow_width / hollow_drawn);
    let hump_along = (s - peak) / (peak * select(STERN_HUMP_FRONT, STERN_HUMP_BACK, s > peak));
    let hump_width = transom * 0.9 + peak * 0.2 + 1.0;
    let hump_drawn = max(hump_width, blur);
    let hump_across = ay / hump_drawn;
    let hump = exp(-(hump_along * hump_along + hump_across * hump_across)) * (hump_width / hump_drawn);
    var quarter_wave = 0.0;
    var ribbon = 0.0;
    var residue = 0.0;
    let run = s - QUARTER_FROM * length;
    if run > 0.0 {
        let leaves = mix(quarter, transom, clamp((QUARTER_FROM * length + quarter_at) / quarter_at, 0.0, 1.0));
        let offset = ay - (leaves + run * QUARTER_ANGLE);
        let face = beam * 0.04 + 0.8 + run * 0.04;
        let drawn = max(face, blur);
        let across = offset / select(drawn * 2.2, drawn, offset > 0.0);
        let rise = smoothstep(0.0, 0.6 * peak, run);
        let fall = exp(-max(run - 0.6 * peak, 0.0) / (0.5 * peak + 0.15 * length));
        quarter_wave = QUARTER_HEIGHT * rise * fall * exp(-(across * across)) * (face / drawn);
        // Where each quarter wave breaks: dense white along its outer face from her quarter aft, and inboard of it what
        // the breaker left, thinner the farther inboard (the longer ago the crest passed).
        let breaks = rise * exp(-run / (1.1 * peak + 0.2 * length));
        let band = (offset - drawn * 0.3) / (drawn * 0.9 + 1.5);
        ribbon = breaks * exp(-(band * band) * 1.6);
        residue = breaks * STERN_RESIDUE * exp(-max(-offset, 0.0) / (drawn * 2.0 + 2.0))
            * (1.0 - smoothstep(0.0, drawn, offset));
    }
    return SternSample(trough + hollow + hump + quarter_wave, hump, ribbon, residue);
}

// Every hull in reach. The vertex stage takes only the height; the fragment stage the slope, breaking white water
// and the aerated water underneath it. `BowWaves.evaluate`.
fn bow_waves(x: f32, z: f32, vertex_stage: bool) -> BowWaveSample {
    var height = 0.0;
    var slope_x = 0.0;
    var slope_z = 0.0;
    var foam = 0.0;
    var bubbles = 0.0;
    var young = 1.0;
    var variance = 0.0;
    let crest_angle = wake.bow_knobs[0].x;
    let crest_width = wake.bow_knobs[0].y;
    let hump_share = wake.bow_knobs[0].z;
    let transverse = wake.bow_knobs[0].w;
    let divergent_gain = wake.bow_knobs[1].x;
    let kelvin_interior = wake.bow_knobs[1].y;
    let foam_gain = wake.bow_knobs[1].z;
    let foam_length = wake.bow_knobs[1].w;
    let foam_spread = wake.bow_knobs[2].x;
    let kelvin_foam = wake.bow_knobs[2].w;
    let kelvin_steep = wake.bow_knobs[3].x;
    let kelvin_foam_length = wake.bow_knobs[3].y;
    let vertex_feature = wake.bow_knobs[3].w;
    let eye = camera.position.xyz;
    let range = max(length(vec3<f32>(eye.x - x, eye.y, eye.z - z)), 1.0);
    // The mesh coarsens with distance from its own centre, which leaves the eye in a magnified view.
    let mesh = wake.torpedo.zw;
    let mesh_range = max(length(vec3<f32>(mesh.x - x, eye.y, mesh.y - z)), 1.0);
    // The shortest feature each stage can hold without shimmering: a few vertices of the water mesh, or five pixels
    // stretched by the grazing view.
    // A magnified view's fan (`mesh_spacing`): its cells grow with their range along the sea.
    let fan_range = length(mesh - vec2<f32>(x, z));
    let cell = select(wake.bow.w + mesh_range * wake.field.w,
        fan_range * max(wake.field.w, wake.bow.w * fan_range), wake.shading[3].w > 0.5);
    let finest = select(range * wake.bow.z / max(abs(eye.y) / range, 0.04) * 5.0, cell * vertex_feature, vertex_stage);
    let count = i32(wake.bow.x);
    for (var i = 0; i < count; i++) {
        let pose = wake.bow_slots[i];
        let wave = wake.bow_slots[BOW_SLOTS + i];
        let hull = wake.bow_slots[2 * BOW_SLOTS + i];
        let bend = wake.bow_slots[3 * BOW_SLOTS + i].x;
        // The V's slopes are shaded at its own amplitude (`kelvin_shading`), not the drawn height's; its white water
        // breaks from the drawn wave's steepness.
        let shading_share = wake.bow_slots[3 * BOW_SLOTS + i].y / max(wave.y, 1e-6);
        let dx = x - pose.x;
        let dz = z - pose.y;
        let hull_length = hull.x;
        let beam = hull.y;
        // How far astern the V is drawn: as far as the arc it is laid on still follows her track (`bow_waves.rs`).
        let reach = max(wake.bow_slots[3 * BOW_SLOTS + i].w, hull_length);
        // Nothing of hers reaches past the V's far corners.
        let span = reach * 1.42 + beam * 1.5;
        if dx * dx + dz * dz > span * span {
            continue;
        }
        // Ship frame: `a` metres astern of the stem, `y` to starboard.
        let a = dz * pose.w - dx * pose.z;
        let y = dx * pose.w + dz * pose.z;
        let ay = abs(y);
        let side = sign(y);
        var slope_a = 0.0;
        var slope_y = 0.0;
        var white = 0.0;
        var aerated = 0.0;
        var fresh = 1.0;
        let along = max(a, 0.0);
        // How far aft of the stem the crest stands highest against the side.
        let peak_at = max(wake.bow_slots[3 * BOW_SLOTS + i].z, 0.5);
        // The crest, the water against the bow's sides and the hull-side wash live close to the hull; the V covers far
        // more water. Nothing is raised or broken ahead of the stem: the sea meets a fine bow undisturbed.
        if a > -CLIMB_LEAD * peak_at && a < hull_length * 0.8 && ay < beam * 0.9 + along * 0.3 {
            let crest_height = wave.z;
            let crest_length = wave.w;
            // The hull's own waterline along her entrance.
            let breadth = entrance_breadth(along, hull_length * BOW_ENTRANCE, wake.bow_slots[4 * BOW_SLOTS + i], beam * 0.5);
            let half_beam = breadth.x;
            let half_beam_rate = breadth.y;
            // The crest climbs the side from the stem, stands highest `peak_at` aft of it, then leaves the side at the
            // crest angle and decays: a steep outer face where it breaks, a gentle fall toward the hull behind it and
            // a trough inboard of that.
            let offset = ay - (half_beam + along * crest_angle);
            let width = beam * crest_width + 0.8 + along * 0.05;
            let blurred = max(width, finest * select(0.2, 0.4, vertex_stage));
            let rise = clamp((a + CLIMB_LEAD * peak_at) / ((1.0 + CLIMB_LEAD) * peak_at), 0.0, 1.0);
            let climb = rise * rise * (3.0 - 2.0 * rise);
            let climb_rate = 6.0 * rise * (1.0 - rise) / ((1.0 + CLIMB_LEAD) * peak_at);
            let fall = exp(-max(along - peak_at, 0.0) / crest_length);
            let fall_rate = -fall * step(peak_at, along) / crest_length;
            let scale = crest_height * (width / blurred);
            let crest = scale * climb * fall;
            let face = select(blurred * CREST_INNER, blurred, offset > 0.0);
            let across = offset / face;
            let inboard = (offset + blurred * CREST_TROUGH_INBOARD) / (blurred * CREST_INNER);
            let peak = exp(-(across * across));
            let trough = exp(-(inboard * inboard)) * CREST_TROUGH;
            let profile = peak - trough;
            // The water stands higher against the bow's sides as a whole, not only along the crest line.
            let outboard = max(ay - half_beam, 0.0);
            let heap_reach = hull_length * HEAP_REACH;
            let heap_width = beam * HEAP_WIDTH + 1.0 + along * HEAP_SPREAD;
            let heap_along = max(along - 2.0 * peak_at, 0.0) / heap_reach;
            let heap_across = outboard / heap_width;
            let heap = crest_height * hump_share * exp(-(heap_along * heap_along + heap_across * heap_across));
            if vertex_stage {
                height += crest * profile + heap * climb;
            } else {
                let rate = across * -2.0 / face * peak + inboard * 2.0 / (blurred * CREST_INNER) * trough;
                let heap_rate = heap * -2.0 * heap_across / heap_width * step(half_beam, ay);
                slope_a += scale * (climb_rate * fall + climb * fall_rate) * profile
                    + crest * rate * -(half_beam_rate + crest_angle)
                    + heap * climb_rate - heap * climb * 2.0 * heap_along / heap_reach - heap_rate * climb * half_beam_rate;
                slope_y += (crest * rate + heap_rate * climb) * side;
                // A fine bow's lower crest can still break: use its steepness against the breadth of the mound it
                // stood on before, with a height gate that keeps slow hulls quiet.
                let breaking = smoothstep(0.03, 0.18, crest_height / (beam * 0.12 + 1.5))
                    * smoothstep(0.12, 0.6, crest_height);
                // The sheet the bow throws up overturns and plunges along the crest's outer face: a narrow white
                // ribbon from just aft of the stem, a little wider and thinner as the water passes aft, gone by half
                // her length. Nothing breaks ahead of the stem.
                let run = breaking * smoothstep(0.0, peak_at * 0.6, a)
                    * exp(-max(along - peak_at, 0.0) / (crest_length * foam_length))
                    * (1.0 - smoothstep(hull_length * 0.25, hull_length * 0.5, along));
                let narrow = max(beam * 0.03 + 1.0, finest * 0.2);
                let spread = narrow + along * foam_spread;
                let band = (offset - blurred * 0.35) / spread;
                let ribbon = run * sqrt(narrow / spread) * exp(-(band * band) * 1.6);
                // What the breaker leaves lies on the water the crest has passed, between its line and the hull:
                // thinner the longer ago the crest crossed it, which is the farther inboard it lies.
                let passed = max(-offset, 0.0) / max(half_beam_rate + crest_angle, 0.05);
                let residue = run * BOW_RESIDUE * exp(-passed / (hull_length * BOW_RESIDUE_LENGTH))
                    * (1.0 - smoothstep(0.0, blurred * 0.6, offset));
                // The sheet's root: white water climbing the side from the stem to the crest's peak.
                let root = breaking * smoothstep(0.0, 1.0, a) * exp(-along / (peak_at * 1.5)) * exp(-(outboard * outboard) / 2.5);
                let breaker = max(ribbon, root);
                white = max(breaker, residue);
                // Dense white where the crest breaks; what it leaves, and the wash along the side, are lace.
                fresh = mix(BOW_RESIDUE_YOUNG, 1.0, clamp(breaker / max(white, 1e-3), 0.0, 1.0));
                // The bubble cloud lies under the white water, no wider (a wider cloud paled the sea a beam out from
                // the bow); faint, so it reads as churned water.
                let fringe = 1.0 - smoothstep((beam * 0.9 + along * 0.3) * 0.7, beam * 0.9 + along * 0.3, ay);
                aerated = white * 0.22 * fringe;
                white *= fringe;
            }
        }
        let course = wake.bow_slots[5 * BOW_SLOTS + i];
        // The stern's waves: her quarters' trough, the hollow behind her stern, the hump where it closes and her quarter
        // waves (`stern_shape`), the hump and the quarter waves' outer faces breaking white at speed.
        let stern = wake.bow_slots[6 * BOW_SLOTS + i];
        let stern_s = a - stern.x;
        let stern_peak = max(stern.z, 1.0);
        if stern.y > 0.01 && stern_s > -0.25 * hull_length && stern_s < stern_peak * 4.0 + hull_length * 0.6
            && ay < beam + max(stern_s, 0.0) * 0.6 + 6.0 {
            let blur = finest * select(0.2, 0.4, vertex_stage);
            let shape = stern_shape(stern_s, ay, stern_peak, stern.w, course.w, hull_length, beam, blur);
            if vertex_stage {
                height += stern.y * shape.height;
            } else {
                let e = clamp(finest * 0.1, 0.3, 4.0);
                let ahead = stern_shape(stern_s - e, ay, stern_peak, stern.w, course.w, hull_length, beam, blur).height;
                let behind = stern_shape(stern_s + e, ay, stern_peak, stern.w, course.w, hull_length, beam, blur).height;
                let outer = stern_shape(stern_s, ay + e, stern_peak, stern.w, course.w, hull_length, beam, blur).height;
                let inner = stern_shape(stern_s, abs(ay - e), stern_peak, stern.w, course.w, hull_length, beam, blur).height;
                slope_a += stern.y * (behind - ahead) / (2.0 * e);
                slope_y += stern.y * (outer - inner) / (2.0 * e) * side;
                let breaker = max(smoothstep(0.35, 0.85, shape.hump), shape.ribbon) * course.z;
                let left = shape.residue * course.z;
                let stern_white = max(breaker, left);
                if stern_white > white {
                    white = stern_white;
                    fresh = mix(STERN_RESIDUE_YOUNG, 1.0, clamp(breaker / max(stern_white, 1e-3), 0.0, 1.0));
                }
                aerated = max(aerated, stern_white * 0.22);
            }
        }
        // Kelvin V behind the stem, inside the 19.47° cusp, laid along the arc the stem has run from her course over the
        // ground (`v_a` astern, `v_y` to starboard of it): `run` metres back along the track and `off` metres to
        // starboard of it (v_a and v_y when straight).
        let v_a = dz * course.y - dx * course.x;
        let v_y = dx * course.y + dz * course.x;
        let cross_term = 1.0 - bend * v_y;
        let off = (v_y * 2.0 - bend * (v_y * v_y + v_a * v_a))
            / (1.0 + sqrt(bend * bend * v_a * v_a + cross_term * cross_term));
        let run = select(v_a, atan2(abs(bend) * v_a, cross_term) / max(abs(bend), 1e-6), abs(bend) > 1e-6);
        let aoff = abs(off);
        let off_side = sign(off);
        let k_zero = wave.x;
        let t_off = max(aoff / max(run, 0.5), 1e-3);
        var zeta = -4.0;
        var interior = 0.0;
        // Within the V's own bounds along its arc (a curved arm leaves any box drawn about her heading).
        if run > 1.0 && run < reach && aoff < run * 0.42 + beam * 1.5 {
            zeta = CUSP_ZETA * pow(k_zero * run, 2.0 / 3.0) * (CUSP_T - t_off);
            interior = 1.0 - smoothstep(CUSP_INTERIOR_FROM, CUSP_INTERIOR_TO, zeta)
                * smoothstep(hull_length * 2.0, hull_length * 4.0, run);
        }
        if zeta > -3.0 && interior > 0.0 {
            var along_slope = 0.0;
            var off_slope = 0.0;
            // Outside the cusp line the systems meet in one crest that dies away as the Airy function does,
            // Ai(x)/Ai(0) ≈ exp(-(2/3)x^(3/2) - x/4); inside it the line keeps its slope and the fringes within it less.
            let beyond = max(-zeta, 0.0);
            let settle = exp(-beyond * (sqrt(beyond) * 0.6667 + 0.25));
            let inner = max(zeta - CUSP_LINE, 0.0) / CUSP_LINE_WIDTH;
            let line = mix(kelvin_interior, CUSP_LINE_GAIN, exp(-(inner * inner)));
            let t = min(t_off, CUSP_T);
            let t_sq = t * t;
            let r = max(max(sqrt(run * run + off * off), beam), TAU * 0.3 / k_zero);
            let root = sqrt(max(1.0 - t_sq * 8.0, 0.0));
            let cusp = pow(k_zero * r, -1.0 / 3.0) * 1.5;
            // The far end fades over half the reach, or three hull lengths of a long one.
            let fade = min(reach * 0.5, hull_length * 3.0);
            let envelope = smoothstep(0.0, hull_length * 0.2, run) * (1.0 - smoothstep(reach - fade, reach, run))
                * settle * interior * wave.y;
            let secant = sqrt(t_sq + 1.0);
            var unresolved = 0.0;
            for (var system = 0; system < 2; system++) {
                let divergent = system == 1;
                let T = select(t * 2.0 / (root + 1.0), (root + 1.0) / (t * 4.0), divergent);
                let shift = select(0.7853981633974483, -0.7853981633974483, divergent);
                let gain = select(transverse, divergent_gain, divergent);
                let big_t_sq = T * T;
                let s_sq = big_t_sq + 1.0;
                let S = sqrt(s_sq);
                let k = k_zero * s_sq;
                let phase = k_zero * S * (run - aoff * T) + shift;
                let cos_ta = max((1.0 - t * T) / (S * secant), 0.05);
                // |∂²φ/∂θ²| vanishes on the cusp; the Airy limit keeps the amplitude finite there.
                let phase_bend = max(abs(1.0 - big_t_sq * 2.0), cusp);
                let shortest = select(finest * KELVIN_FINEST, finest, vertex_stage);
                let resolved = smoothstep(shortest * 0.5, shortest, TAU / k);
                let wave_amp = envelope * gain * exp(-k * hull.z) * sqrt(TAU / (k_zero * r * s_sq * cos_ta * phase_bend));
                let amp = wave_amp * resolved;
                // The interior's attenuation already shapes the shaded slopes. Without it in the height too,
                // apparently quiet water beside a fast hull held metre-deep transverse troughs. Keep the cusp's
                // existing height: its extra shading gain is a contrast choice, not extra displacement.
                let drawn_amp = amp * min(line, 1.0);
                if vertex_stage {
                    height += drawn_amp * cos(phase);
                    continue;
                }
                // What the pixel cannot draw of the crests' slope (k times their amplitude) roughens it instead.
                let unresolved_slope = wave_amp * shading_share * line * k;
                unresolved += 0.5 * unresolved_slope * unresolved_slope * (1.0 - resolved);
                let swing = sin(phase) * amp * shading_share * line * k_zero * S;
                along_slope -= swing;
                off_slope += swing * T * off_side;
                if divergent {
                    // Divergent crests steepen and break where they meet the transverse waves at the cusp, if the
                    // drawn wave is steep enough (ak), not the steeper one the slopes are shaded at: a fast destroyer's
                    // first crests break, a battleship's barely, and the arms astern carry none.
                    let crest_foam = smoothstep(0.55, 0.98, cos(phase)) * smoothstep(0.06, 0.11, t_sq)
                        * smoothstep(kelvin_steep, kelvin_steep * 3.3, drawn_amp * k)
                        * exp(-run / (hull_length * kelvin_foam_length)) * kelvin_foam;
                    // A crest breaking is fresh white water, whatever the bow left under it.
                    if crest_foam > white {
                        white = crest_foam;
                        fresh = 1.0;
                    }
                }
            }
            if unresolved > 0.0 {
                variance += unresolved * KELVIN_ROUGHNESS * clamp(0.9 + mx_noise_float(vec3<f32>(run / KELVIN_ROUGHNESS_PATCH,
                    off_side * 5.3 + f32(i) * 11.7, wake.bow.y * 0.02)) * 1.2, 0.15, 1.6);
            }
            if !vertex_stage {
                // The arc's frame is turned by her change of course since the stem passed this point, then laid back
                // onto world x/z from her course.
                let turn = bend * run;
                let turn_cos = cos(turn);
                let turn_sin = sin(turn);
                let v_slope_a = along_slope * turn_cos - off_slope * turn_sin;
                let v_slope_y = along_slope * turn_sin + off_slope * turn_cos;
                slope_x += v_slope_y * course.y - v_slope_a * course.x;
                slope_z += v_slope_a * course.y + v_slope_y * course.x;
            }
        }
        if !vertex_stage {
            // Rotate the ship-frame slope back to world x/z.
            slope_x += slope_y * pose.w - slope_a * pose.z;
            slope_z += slope_a * pose.w + slope_y * pose.z;
            if max(white, aerated) > 0.02 {
                let noise = bow_foam_noise(vec2<f32>(x, z), pose.zw, finest);
                // A noise-dithered threshold: dense at the core, torn at the edges, strands where it thins.
                let tear = mix(BOW_LACE_TEAR, BOW_TEAR, smoothstep(BOW_RESIDUE_YOUNG, 1.0, fresh));
                let coverage = smoothstep(0.18, 0.58, white + noise * tear);
                let covered = coverage * min(white, 0.85) * foam_gain;
                if covered > foam {
                    foam = covered;
                    young = fresh;
                }
                bubbles = max(bubbles, aerated * clamp(0.8 + noise * 0.4, 0.45, 1.0));
            }
        }
    }
    return BowWaveSample(height, vec2<f32>(slope_x, slope_z), min(foam, 1.0), bubbles, young, variance);
}

// ---- The fleet's trails ----------------------------------------------------------------------------------------------

fn inside_square(uv: vec2<f32>) -> bool {
    return uv.x > 0.025 && uv.x < 0.975 && uv.y > 0.025 && uv.y < 0.975;
}

fn within(p: vec2<f32>, b: vec4<f32>) -> bool {
    return p.x > b.x && p.y > b.y && p.x < b.z && p.y < b.w;
}

fn square_edge(uv: vec2<f32>) -> f32 {
    return smoothstep(0.025, 0.05, uv.x) * (1.0 - smoothstep(0.95, 0.975, uv.x)) * smoothstep(0.025, 0.05, uv.y)
        * (1.0 - smoothstep(0.95, 0.975, uv.y));
}

fn sway(p: vec2<f32>, f: f32, phase: f32) -> f32 {
    return sin(p.x * f + sin(p.y * (f * 0.61) + phase) * 1.7);
}

// Where a texel lies across its band, in the band's half-widths from its centreline, from the atlas's blue or alpha
// (shaders/stamps.wgsl's encoding of how central it is); -1 where no band's stamp reached it.
const TRACK_RANGE: f32 = 1.3;
const TRACK_ZERO: f32 = 0.1;
fn track_coordinate(encoded: f32) -> f32 {
    return select(-1.0, (1.0 - (encoded - TRACK_ZERO) / (1.0 - TRACK_ZERO)) * TRACK_RANGE, encoded > TRACK_ZERO * 0.5);
}

// The churned water's foam, as it ages: dense white water whipped into eddies behind the stern, drawn out along the track
// into streaks and braids that thin as the wake spreads, densest down the middle and along both edges.
const YOUNG_START: f32 = 0.45;
const YOUNG_FULL: f32 = 0.85;
// How much the young water's eddies are drawn out along the track.
const YOUNG_STREAKS: f32 = 0.35;
// Streaks across a band's half-width, and their length along it (in beams).
const STREAKS_ACROSS: f32 = 9.0;
const STREAK_LENGTH: f32 = 0.45;
// Old foam's cover: a braided rope of streaks down the middle of the band (out to CORE_WIDTH of its half-width), sparse
// lace across the rest, and a line along each edge, where the wake's spreading front gathers it.
const CORE_COVER: f32 = 0.6;
const CORE_WIDTH: f32 = 0.32;
const LACE_COVER: f32 = 0.1;
const EDGE_AT: f32 = 0.9;
const EDGE_WIDTH: f32 = 0.09;
const EDGE_COVER: f32 = 0.3;
// The young water's white thins toward the band's sides, where turquoise water shows through its lace.
const YOUNG_SIDES: f32 = 0.5;
// The faint line of foam and bubbles the slick keeps down its middle long after the churned water is gone.
const RESIDUAL_COVER: f32 = 0.16;
// How far the band's edges wander along it (a share of its half-width).
const EDGE_WANDER: f32 = 0.4;
const RESIDUAL_WIDTH: f32 = 0.22;
// The churned water's edge is torn over at least this much of its pattern: lace that thins into the sea around it, not
// a mask cut along one level of the eddies.
const WAKE_TEAR: f32 = 0.3;
// The bubble cloud thins from the band's middle (this share of its half-width) to its edge, and keeps this share of
// itself once the water is old: the pale turquoise stays close astern.
const BUBBLES_INSIDE: f32 = 0.3;
const OLD_BUBBLES: f32 = 0.4;
// Streaks along a band at `band` half-widths from its centreline: fine across it, drawn out along it (world-anchored, in
// beams), braided by warps that wander at two scales, evolving at `time`. 0–1, about uniform.
fn wake_streaks(band: f32, world: vec2<f32>, beam: f32, time: f32) -> f32 {
    let along = world / (beam * STREAK_LENGTH);
    let braid = mx_noise_float(vec3<f32>(along * 0.31 + 11.0, time * 0.02)) * 1.4
        + mx_noise_float(vec3<f32>(along * 1.3 + 29.0, time * 0.04)) * 0.5;
    let across = band * STREAKS_ACROSS + braid;
    let coarse = mx_noise_float(vec3<f32>(across, along.x, along.y + time * 0.015));
    let fine = mx_noise_float(vec3<f32>(across * 2.3 + 17.0, along.x * 2.6 + 5.0, along.y * 2.6 + time * 0.03));
    let flecks = mx_noise_float(vec3<f32>(across * 4.1 + 41.0, along * 6.0 + time * 0.05));
    return clamp((coarse * 0.55 + fine * 0.35 + flecks * 0.25) * 1.35 + 0.5, 0.0, 1.0);
}

// Foam covering `cover` of a 0–1 `pattern`, its edge torn over `tear`, or the cover itself where the pattern is finer than
// the pixel (`resolved` 0); none at all where there is no cover, however high the pattern peaks.
fn torn_foam(cover: f32, pattern: f32, tear: f32, resolved: f32) -> f32 {
    let share = clamp(cover, 0.0, 1.0);
    let threshold = 1.0 - share;
    let width = max(min(tear, min(share, threshold)), 1e-4);
    return mix(smoothstep(0.0, 1.0, share), smoothstep(threshold - width, threshold + width, pattern), resolved)
        * smoothstep(0.0, 1e-3, share);
}

// The churned water's relief (2026-10-23): behind a stern the young churned water heaves in boils, lumps RELIEF_LUMP of
// her beam across (and finer ones RELIEF_FINE of that) as tall as RELIEF of her beam, within RELIEF_LEAST and
// RELIEF_MOST, turning over at RELIEF_RATE. They flatten as the water ages through RELIEF_YOUNG_START–RELIEF_YOUNG_FULL
// of its turbulence, so old water lies flat under its slick. The vertex stage raises the lumps the water mesh holds
// (`trail_relief`); the fragment stage shades them all (`trail_read`) and roughens the boiling water by
// RELIEF_ROUGHNESS of slope variance, where the slick laid with it no longer stills it. Bands only: a splash's patch
// keeps its own look.
const RELIEF: f32 = 0.035;
const RELIEF_LEAST: f32 = 0.2;
const RELIEF_MOST: f32 = 0.8;
const RELIEF_LUMP: f32 = 0.33;
const RELIEF_FINE: f32 = 0.4;
const RELIEF_RATE: f32 = 0.35;
const RELIEF_YOUNG_START: f32 = 0.55;
const RELIEF_YOUNG_FULL: f32 = 0.9;
const RELIEF_ROUGHNESS: f32 = 0.015;

// The boils' shape at `at`, about -1 to 1: lumps `lump` metres across, with `fine` of the finer ones.
fn boil(at: vec2<f32>, lump: f32, fine: f32) -> f32 {
    let time = wake.trail.y * RELIEF_RATE;
    let coarse = mx_noise_float(vec3<f32>(at / lump, time));
    let finer = mx_noise_float(vec3<f32>(at / (lump * RELIEF_FINE) + 31.0, time * 1.7));
    return coarse * 1.6 + finer * 0.6 * fine;
}

// The boils' height (m) and how much of it the water is boiling, for water whose band is `turbulence` churned behind a
// hull `beam` wide.
fn boil_height(turbulence: f32, beam: f32) -> vec2<f32> {
    let boiling = smoothstep(RELIEF_YOUNG_START, RELIEF_YOUNG_FULL, turbulence);
    return vec2<f32>(clamp(beam * RELIEF, RELIEF_LEAST, RELIEF_MOST) * boiling, boiling);
}

// The strongest churned water at grid point `at`, read without the outline's warp: its turbulence and her beam; none
// where that water is a splash's patch (no band).
fn trail_churn(at: vec2<f32>) -> vec2<f32> {
    var strongest = vec2<f32>(0.0, 10.0);
    if wake.trail.x < 0.5 || !within(at, wake.trail_reach) {
        return strongest;
    }
    var banded = false;
    let count = i32(wake.trail.x);
    for (var i = 0; i < count; i++) {
        if !within(at, wake.tiles[2 * TILE_COUNT + i]) {
            continue;
        }
        let bounds = wake.tiles[i];
        let uv = (at - bounds.xy) / WAKE_EXTENT + 0.5;
        if !inside_square(uv) {
            continue;
        }
        let tile = textureSampleLevel(trail_atlas, wake_sampler, (uv + bounds.zw) / TILES, 0.0);
        let value = tile.r * square_edge(uv);
        if value > strongest.x {
            strongest = vec2<f32>(value, max(wake.tiles[TILE_COUNT + i].z, 1.0));
            banded = track_coordinate(tile.b) >= 0.0;
        }
    }
    return select(vec2<f32>(0.0, 10.0), strongest, banded);
}

// The boils' height at grid point `at` (the surface's vertex stage), where the water mesh's cells are `spacing` metres:
// only the lumps a few of its cells across.
fn trail_relief(at: vec2<f32>, spacing: f32) -> f32 {
    let churn = trail_churn(at);
    let height = boil_height(churn.x, churn.y);
    if height.y <= 0.0 {
        return 0.0;
    }
    let lump = churn.y * RELIEF_LUMP;
    let held = 1.0 - smoothstep(0.25, 0.5, spacing / lump);
    let fine = 1.0 - smoothstep(0.25, 0.5, spacing / (lump * RELIEF_FINE));
    return height.x * held * boil(at, lump, fine);
}

struct TrailSample {
    // Foam cover 0–1, the bubble cloud's density, how much of the short waves the slick stills.
    foam: f32,
    bubbles: f32,
    slick: f32,
    // How young the churned water is: 1 its dense, billowing white water, 0 the lace of old streaks.
    young: f32,
    // The boils' slope (x, z) and how much the water is boiling (`trail_relief`).
    boil: vec3<f32>,
}

// The realistic wake at a world point. `footprint` is the pixel's metres, max(fwidth(x), fwidth(z)), taken by the caller
// in uniform control flow. `FleetWakeFoam.evaluate`, with the churned water streaked along the track.
fn trail_read(at: vec2<f32>, footprint: f32) -> TrailSample {
    // No trails (a calm wake binds zeroed uniforms, whose knobs would put equal edges in the smoothsteps below).
    if wake.trail.x < 0.5 {
        return TrailSample(0.0, 0.0, 0.0, 0.0, vec3<f32>(0.0));
    }
    let foam_start = wake.shading[0].x;
    let foam_full = wake.shading[0].y;
    let foam_cover = wake.shading[0].z;
    let tear = wake.shading[0].w;
    let bubble_start = wake.shading[1].x;
    let bubble_full = wake.shading[1].y;
    let bubble_share = wake.shading[1].z;
    let bubble_tear = wake.shading[1].w;
    let lobes = wake.shading[2].x;
    let k = wake.shading[2].y;
    let f = wake.shading[2].z;
    let fine = wake.shading[2].w;
    let slick_full = wake.shading[3].x;
    let calm_share = wake.shading[3].y;
    let residue = wake.shading[3].z;
    var turbulence = 0.0;
    var slick = 0.0;
    // Where the point lies across the strongest band here and across the strongest slick, and that hull's beam.
    var band = -1.0;
    var slick_band = -1.0;
    var beam = 10.0;
    var slick_beam = 10.0;
    var foam = 0.0;
    var bubbles = 0.0;
    var young = 0.0;
    var boils = vec3<f32>(0.0);
    var world = at;
    var strongest = -1;
    // How far inside the slick's wandering edge the point lies (1 well inside it).
    var slick_edge = 1.0;
    // Most of the sea lies outside every trail: one test against their union spares it the tiles.
    if within(at, wake.trail_reach) {
        // The trail is read through a gentle warp, so its outline frays at a few metres.
        world = at + vec2<f32>(sway(at, k, 1.3) + sway(at.yx, k * 1.83, 4.1) * 0.5,
            sway(at.yx, k * 1.19, 2.9) + sway(at, k * 2.07, 0.7) * 0.5) * lobes;
        let count = i32(wake.trail.x);
        for (var i = 0; i < count; i++) {
            if !within(world, wake.tiles[4 * TILE_COUNT + i]) {
                continue;
            }
            let bounds = wake.tiles[i];
            let slick_row = wake.tiles[TILE_COUNT + i];
            let slick_uv = (world - slick_row.xy) / SLICK_EXTENT + 0.5;
            if within(world, wake.tiles[3 * TILE_COUNT + i]) && inside_square(slick_uv) {
                let wide = textureSampleLevel(trail_atlas, wake_sampler, (slick_uv + bounds.zw) / TILES, 0.0);
                let value = wide.g * square_edge(slick_uv);
                if value > slick {
                    slick = value;
                    slick_band = track_coordinate(wide.a);
                    slick_beam = max(slick_row.z, 1.0);
                }
            }
            let uv = (world - bounds.xy) / WAKE_EXTENT + 0.5;
            if within(world, wake.tiles[2 * TILE_COUNT + i]) && inside_square(uv) {
                let tile = textureSampleLevel(trail_atlas, wake_sampler, (uv + bounds.zw) / TILES, 0.0);
                let value = tile.r * square_edge(uv);
                if value > turbulence {
                    turbulence = value;
                    strongest = i;
                    beam = max(slick_row.z, 1.0);
                }
            }
        }
        // The streaks follow the track itself, not the warp that frays the outline: where the point lies across the
        // strongest band is read where it is.
        if strongest >= 0 {
            let bounds = wake.tiles[strongest];
            let uv = (at - bounds.xy) / WAKE_EXTENT + 0.5;
            band = track_coordinate(textureSampleLevel(trail_atlas, wake_sampler, (uv + bounds.zw) / TILES, 0.0).b);
            if band < 0.0 {
                // Coverage the warp brought from a band, read past where the band says where it lies: outside it.
                band = TRACK_RANGE;
            }
        }
        let time = wake.trail.y;
        // Where the eddies and streaks fall below a pixel, it takes their mean cover instead of shimmering.
        let resolved = 1.0 - smoothstep(0.15, 0.6, footprint * f);
        if turbulence > 0.01 {
            // World-anchored eddies from ten metres down to a metre and a half: the young water's churn, carried on the
            // trail's water (flow maps), not scrolled through time.
            let flow = flow_phases(world, trail_velocity(at, strongest, turbulence, band, beam), time);
            let eddies = blend0(trail_eddies(flow.from_a, f, fine), trail_eddies(flow.from_b, f, fine), 1.0 - flow.weight);
            young = smoothstep(YOUNG_START, YOUNG_FULL, turbulence);
            // The band's edges wander a little along it, as a spreading wake's do.
            let wander = 1.0 + (mx_noise_float(vec3<f32>(world / (beam * 2.2), time * 0.01)) * 0.7
                + mx_noise_float(vec3<f32>(world / (beam * 0.6) + 7.0, time * 0.02)) * 0.3) * EDGE_WANDER;
            let across = band * wander;
            // Away from the stern the foam lies in streaks along the track, down the middle and along the edges.
            let streaks = blend01(wake_streaks(band, flow.from_a, beam, 0.0), wake_streaks(band, flow.from_b, beam, 0.0),
                1.0 - flow.weight);
            // Its white in patches along it and in masses on the boils, its old foam gathered in clumps.
            let clumps = blend01(clump_field(flow.from_a), clump_field(flow.from_b), 1.0 - flow.weight);
            let banded = banded_foam(world, at, across, turbulence, young, beam, streaks,
                clamp(eddies * 1.3 + 0.5, 0.0, 1.0), mix(0.5, clumps, clumps_resolved(footprint)),
                vec3<f32>(foam_start, foam_full, foam_cover));
            let cover = banded.x;
            let pattern = banded.y;
            // The bubble cloud fills the band, densest down the middle, patchy and streaked like the foam over it,
            // fading out across the outer part of the band.
            let patches = smoothstep(0.2, 0.8, streaks * 0.5 + (eddies * 0.5 + 0.5) * 0.5);
            let aerated = (1.0 - smoothstep(BUBBLES_INSIDE, 1.1, across)) * mix(1.0, mix(0.35, 1.0, patches), 1.0 - young)
                * mix(OLD_BUBBLES, 1.0, young);
            // The young water's boils (`trail_relief`), read where the point lies as the vertex stage raises them,
            // down to lumps a few pixels across.
            let boiling = boil_height(turbulence, beam);
            if boiling.y > 0.0 {
                let lump = beam * RELIEF_LUMP;
                let shown = 1.0 - smoothstep(0.15, 0.35, footprint / lump);
                let finer = 1.0 - smoothstep(0.15, 0.35, footprint / (lump * RELIEF_FINE));
                let e = lump * 0.12;
                let here = boil(at, lump, finer);
                let rise = vec2<f32>(boil(at + vec2<f32>(e, 0.0), lump, finer), boil(at + vec2<f32>(0.0, e), lump, finer))
                    - here;
                boils = vec3<f32>(rise * (boiling.x * shown / e), boiling.y);
            }
            foam = torn_foam(cover, pattern, max(tear, WAKE_TEAR), resolved);
            bubbles = smoothstep(bubble_start, bubble_full, turbulence + eddies * bubble_tear) * (pattern * 0.4 + 0.6)
                * bubble_share * aerated;
        }
        // A slick read where its band was never written lies past its edge (stilled water all the way out to a texel
        // boundary drew straight-edged calm patches).
        if slick > 0.01 && slick_band < 0.0 {
            slick_edge = 0.0;
        }
        if slick > 0.01 && slick_band >= 0.0 {
            // The slick's edges wander and fade over its outer part instead of stopping at a line.
            let wander = 1.0 + mx_noise_float(vec3<f32>(world / (slick_beam * 3.1) + 23.0, time * 0.01)) * EDGE_WANDER;
            let across = slick_band * wander;
            slick_edge = 1.0 - smoothstep(0.45, 1.0, across);
            // The old wake's middle: a few thin streaks of foam over a faint bubble cloud, as long as the slick lasts.
            let strength = smoothstep(0.0, 0.6, slick);
            let middle = 1.0 - smoothstep(RESIDUAL_WIDTH * 0.4, RESIDUAL_WIDTH, across);
            // Its streaks ride the drift, gathered in clumps.
            let drift = flow_phases(world, wake.oil_flow.xy, time);
            let streaks = blend01(wake_streaks(slick_band * 0.5, drift.from_a, slick_beam, 0.0),
                wake_streaks(slick_band * 0.5, drift.from_b, slick_beam, 0.0), 1.0 - drift.weight);
            let residual_pattern = blend01(streaks, blend01(clump_field(drift.from_a), clump_field(drift.from_b),
                1.0 - drift.weight), TRAIL_CLUMPING);
            let residual = torn_foam(strength * middle * RESIDUAL_COVER, residual_pattern, tear, resolved);
            foam = max(foam, residual * (1.0 - young));
            let patches = smoothstep(0.25, 0.75, streaks);
            bubbles = max(bubbles, strength * (1.0 - smoothstep(0.2, 0.75, across)) * mix(0.5, 1.0, patches) * residue);
        }
    }
    let calm = smoothstep(0.0, max(slick_full, 1e-6), slick) * slick_edge;
    return TrailSample(foam, bubbles, calm * calm_share, young, boils);
}

// ---- Torpedo tracks ------------------------------------------------------------------------------------------------

fn torpedo_foam(at: vec2<f32>) -> f32 {
    if wake.trail.w < 0.5 {
        return 0.0;
    }
    let uv = (at - wake.torpedo.xy) / WAKE_EXTENT + 0.5;
    var energy = 0.0;
    if uv.x > 0.0 && uv.x < 1.0 && uv.y > 0.0 && uv.y < 1.0 {
        let coverage = textureSampleLevel(torpedo_field, wake_sampler, uv, 0.0).r;
        if coverage > 0.01 {
            // Metre-scale clumps: a track is a string of bubble patches, not a painted line.
            let time = wake.trail.z;
            let clumps = mx_noise_float(vec3<f32>(at * 0.32, time * 0.12));
            let patches = mx_noise_float(vec3<f32>(at * 0.09 + 31.0, time * 0.05));
            let edge = smoothstep(0.0, 0.03, uv.x) * (1.0 - smoothstep(0.97, 1.0, uv.x)) * smoothstep(0.0, 0.03, uv.y)
                * (1.0 - smoothstep(0.97, 1.0, uv.y));
            energy = coverage * clamp(clumps * 0.55 + patches * 0.5 + 0.74, 0.3, 1.0) * edge;
        }
    }
    return energy;
}

// ---- Hull contact foam ---------------------------------------------------------------------------------------------

// The contact line and the side wash start at the plating: full against it and falling away outboard over these many
// of their widths, so each reaches half strength about as far out as when it peaked a third (the line) and a half (the
// wash) of a width from the side and left darker water between.
const LINE_SPREAD: f32 = 1.3;
const WASH_SPREAD: f32 = 1.6;
// The slap: white water where the sides' memory says the water has just rushed up a hull's side (its slap, m/s), none
// below SLAP_START and full from SLAP_FULL. The sea at rest in a 12 m/s wind rushes up a battleship's side at about
// 0.3 m/s on the mean, 0.45–0.6 at the tenth highest stations and 0.9–1.2 at the hardest; a destroyer at 16 m/s, whose
// own heave and pitch keep the water working up her sides, 0.9 on the mean, 1.1–1.8 at the tenth highest and 2–3.5 at
// the hardest, so a hull under way whitens from SLAP_UNDERWAY m/s higher at her full speed. It lies against her
// plating, SLAP_REACH metres out at the start and the second at full, torn by the contact line's eddies (SLAP_TEAR)
// into patches a metre or two across that thin as it fades (at most SLAP_COVER). The side the wind blows onto takes
// all of it, her lee side SLAP_LEE; the water it whitens holds air (SLAP_BUBBLES). It keeps to her plating: a station
// with no crossing (beyond her stem and stern) reads a quarter of her half beam inside her centreline, near enough for
// the slap's reach to cross it, so the slap is gone once the side's breadth falls SLAP_PLATING of her half beam inside
// her centreline.
const SLAP_START: f32 = 0.2;
const SLAP_FULL: f32 = 0.8;
const SLAP_UNDERWAY: f32 = 0.7;
const SLAP_REACH: vec2<f32> = vec2<f32>(0.6, 2.0);
const SLAP_TEAR: f32 = 1.6;
const SLAP_COVER: f32 = 0.6;
const SLAP_LEE: f32 = 0.5;
const SLAP_BUBBLES: f32 = 0.12;
const SLAP_PLATING: f32 = 0.05;
// The water climbing a hull under way is churned against her plating and full of air, not glass: out to FROTH_REACH
// of the wash's width from her side, from her stem to her waist, it holds up to FROTH_BUBBLES of bubbles in streaks
// (the wash's lace) and takes back FROTH_DIM of the light through the waves (the surface's `wave_light`).
const FROTH_REACH: f32 = 1.0;
const FROTH_BUBBLES: f32 = 0.2;
const FROTH_DIM: f32 = 0.7;
// Stations along each remembered side (`naval_ocean::hull_sides::SIDE_STATIONS`).
const MEMORY_STATIONS: u32 = 128u;

struct HullFoamSample {
    contact: f32,
    wash: f32,
    bubbles: f32,
    // How much of the light through the waves the churned water against a hull under way takes back (0 none, 1 all).
    froth: f32,
}

// How hard the water has lately rushed up a remembered hull's side (m/s, the sides' memory's slap): `memory` her slot
// there, `starboard` the side, `along` the share of her waterline's length aft of her stem; between the two nearest
// stations.
fn side_slap(memory: u32, starboard: bool, along: f32) -> f32 {
    let s = clamp(along * f32(MEMORY_STATIONS) - 0.5, 0.0, f32(MEMORY_STATIONS - 1u));
    let k = u32(s);
    let row = (memory * 2u + select(0u, 1u, starboard)) * MEMORY_STATIONS;
    let next = min(k + 1u, MEMORY_STATIONS - 1u);
    return mix(hull_side_state[row + k].z, hull_side_state[row + next].z, s - f32(k));
}

// Foam coverage on the water where hulls meet it. `x`, `z` anchor the noise (the grid point); the hull test reads the
// fragment's drawn position `world`. `HullContactFoam.evaluate`, with the slap and the froth of a hull under way.
fn hull_foam(x: f32, z: f32, world: vec3<f32>) -> HullFoamSample {
    var foam = 0.0;
    var wash_foam = 0.0;
    var bubbles = 0.0;
    var froth = 0.0;
    let point = vec4<f32>(world, 1.0);
    let eye = camera.position.xyz;
    let range = max(length(vec3<f32>(eye.x - x, eye.y, eye.z - z)), 1.0);
    // Three pixels stretched by the grazing view: a line narrower than that fades instead of shimmering.
    let finest = range * wake.hull.z / max(abs(eye.y) / range, 0.04) * 3.0;
    let count = i32(wake.hull.x);
    for (var i = 0; i < count; i++) {
        let base = i * HULL_STRIDE;
        let bound = wake.hull_data[base];
        let dx = world.x - bound.x;
        let dz = world.z - bound.y;
        if dx * dx + dz * dz >= bound.z {
            continue;
        }
        let shape = wake.hull_data[base + 4];
        let drive = wake.hull_data[base + 5];
        let lx = dot(wake.hull_data[base + 1], point);
        let lz = dot(wake.hull_data[base + 3], point);
        let width = drive.z;
        let wash_width = max(wake.bow_knobs[2].z, 0.01);
        let reach_width = max(width, wash_width);
        let hull_length = drive.w;
        let half = shape.z;
        if !(abs(lx) < half + reach_width * 3.0 && lz > shape.w - reach_width * 3.0 - shape.y
            && lz < shape.w + hull_length + reach_width * 3.0 + shape.y) {
            continue;
        }
        // Bilinear in station and level: the hull's breadth where the water actually stands against it.
        let ly = dot(wake.hull_data[base + 2], point);
        let level = clamp((ly - PROFILE_LOWEST) / PROFILE_LEVEL_STEP, 0.0, f32(PROFILE_LEVELS - 1));
        let lower = min(floor(level), f32(PROFILE_LEVELS - 2));
        let rise = level - lower;
        let station = clamp((lz - shape.x) / shape.y, 0.0, f32(PROFILE_STATIONS - 1));
        let first = min(floor(station), f32(PROFILE_STATIONS - 2));
        let along = station - first;
        let cell = base + HULL_PROFILE + i32(lower) * PROFILE_STATIONS + i32(first);
        let below = wake.hull_data[cell];
        let above = wake.hull_data[cell + PROFILE_STATIONS];
        let breadth = mix(mix(below.xy, below.zw, along), mix(above.xy, above.zw, along), rise);
        // Metres outboard of the hull's side on this side of the centreline.
        let edge = select(-lx - breadth.y, lx - breadth.x, lx >= 0.0);
        let starboard = lx >= 0.0;
        // Every band is full against the plating and falls away outboard; inside the side, where the profile stands a
        // little proud of the drawn hull, it fades over 0.6 m.
        let inside = smoothstep(-0.6, 0.0, edge);
        let out = max(edge, 0.0);
        let blurred = max(width, finest);
        let across = out / (blurred * LINE_SPREAD);
        let line = exp(-(across * across)) * inside * (width / blurred);
        let from_bow = max((lz - shape.w) / hull_length, 0.0);
        let strength = drive.x + drive.y * (1.0 - smoothstep(0.0, wake.hull.w, from_bow));
        let white = line * strength;
        let underway = wake.hull_data[base + 6].z;
        // The slap: the water has just rushed up her side here (the sides' memory, at her station on this side), and
        // left white water against her plating, wider the harder it came, more on the side the wind blows onto.
        let sides = wake.hull_data[base + HULL_SLAP];
        var slap_white = 0.0;
        if sides.x > 0.5 {
            let slap = side_slap(u32(sides.x + 0.5) - 1u, starboard, (lz - shape.w) / hull_length);
            let start = SLAP_START + SLAP_UNDERWAY * underway;
            let share = smoothstep(start, start + SLAP_FULL - SLAP_START, slap);
            let windward = mix(SLAP_LEE, 1.0, smoothstep(-0.5, 0.5, select(-sides.y, sides.y, starboard)));
            let plating = smoothstep(-SLAP_PLATING * half, 0.0, select(breadth.y, breadth.x, starboard));
            let reach = mix(SLAP_REACH.x, SLAP_REACH.y, share);
            let slap_blurred = max(reach, finest);
            let slap_across = out / slap_blurred;
            slap_white = share * windward * plating * exp(-(slap_across * slap_across)) * inside
                * (reach / slap_blurred);
            bubbles = max(bubbles, slap_white * SLAP_BUBBLES);
        }
        if max(white, slap_white) > 0.01 {
            // World-anchored eddies: the water streams aft past a moving hull and churns slowly at rest.
            let time = wake.hull.y;
            let strands = mx_noise_float(vec3<f32>(x * 0.21, z * 0.21, time * 0.35));
            let flecks = mx_noise_float(vec3<f32>(x * 0.63 + 5.0, z * 0.63, time * 0.8));
            // Distant water keeps the mean coverage instead of sub-pixel noise.
            let detail = 1.0 - smoothstep(1.5, 6.0, finest);
            let noise = (strands * 0.6 + flecks * 0.4) * detail;
            foam = max(foam, smoothstep(0.1, 0.7, white + noise * 0.6) * min(white, 0.55));
            // The slap's white water tears into patches as it fades, never a painted stripe.
            foam = max(foam, smoothstep(0.2, 0.6, slap_white + noise * SLAP_TEAR) * min(slap_white, SLAP_COVER));
        }
        // The narrow side wash uses the same actual breadth and full pose as the contact line. The bow wave's
        // yaw-only entrance approximation detached this band from tapered or heeled sides, and its clamped outboard
        // distance painted wash inside the hull too. Keep the original strength, width, downstream fade and lace.
        if wake.bow.x > 0.0 && underway > 0.001 {
            let a = lz - shape.w;
            let wash_blurred = max(wash_width, finest);
            let wash_edge = out / (wash_blurred * WASH_SPREAD);
            let waist = 1.0 - smoothstep(hull_length * 0.45, hull_length * 0.75, a);
            let wash = exp(-(wash_edge * wash_edge)) * inside
                * smoothstep(hull_length * 0.02, hull_length * 0.1, a) * waist
                * underway * wake.bow_knobs[2].y * (wash_width / wash_blurred);
            // The water her way heaps against her side, from the stem to her waist, is churned and full of air.
            let near_reach = wash_width * FROTH_REACH;
            let near = exp(-(out * out) / (near_reach * near_reach)) * inside
                * smoothstep(0.0, hull_length * 0.04, a) * waist * underway;
            froth = max(froth, near * FROTH_DIM);
            if max(wash, near) > 0.02 {
                let axis = wake.hull_data[base + 3];
                let heading = normalize(vec2<f32>(-axis.x, axis.z));
                let noise = bow_foam_noise(vec2<f32>(x, z), heading, finest);
                let covered = smoothstep(0.18, 0.58, wash + noise * BOW_LACE_TEAR)
                    * min(wash, 0.85) * wake.bow_knobs[1].z;
                wash_foam = max(wash_foam, covered);
                bubbles = max(bubbles, wash * 0.22 * clamp(0.8 + noise * 0.4, 0.45, 1.0));
                bubbles = max(bubbles, near * FROTH_BUBBLES * clamp(0.5 + noise * 0.8, 0.15, 1.0));
            }
        }
    }
    return HullFoamSample(foam, wash_foam, bubbles, froth);
}

// ---- The sea beside hulls (shelter) ---------------------------------------------------------------------------------

const HULL_SIDES: i32 = HULL_PROFILE + PROFILE_LEVELS * PROFILE_STATIONS;
const PI: f32 = 3.141592653589793;
// Distances from a hull's side, in heights of the side, over which its shelter fades out.
const SHELTER_FADE: f32 = 3.0;
const SHELTER_REACH: f32 = 5.0;
// Softest edge (radians) of the side's top in a mirrored lobe.
const MIRROR_EDGE: f32 = 0.02;

// Height of the side's top at `station` (clamped to the hull's ends) on the starboard or port side, in hull coordinates.
fn side_top(base: i32, station: f32, starboard: bool) -> f32 {
    let first = min(floor(station), f32(PROFILE_STATIONS - 2));
    let along = station - first;
    let cell = wake.hull_data[base + HULL_SIDES + i32(first)];
    let height = mix(cell.xy, cell.zw, along);
    return select(height.y, height.x, starboard);
}

// Height of the side standing at `station` above the water at hull coordinates `local`, on the starboard or port side,
// `breadth` out from the centre line, in the world (`up`, the world's up in hull coordinates): a heeled hull's high side
// does not stand taller the farther out the water lies.
fn side_height(base: i32, station: f32, starboard: bool, local: vec3<f32>, breadth: f32, up: vec3<f32>) -> f32 {
    let top = vec3<f32>(select(-breadth, breadth, starboard), side_top(base, station, starboard), local.z);
    return max(dot(up, top - local), 0.0);
}

// A wall's view factor term for one end at `x` (the end's distance along the side over the distance out), `k` =
// 1/√(1 + Y²).
fn wall_end(x: f32, k: f32) -> f32 {
    return atan(x) - atan(x * k) * k;
}

// How the hulls near the displaced point `world` stand over the water there (`HullContactFoam.evaluateShelter`): the
// share of the sky's light their sides hide, the share of the mirrored lobe along `direction` (spread `spread` radians)
// that meets a side instead of the sky, and that side's horizontal outward normal (zw).
fn hull_shelter(world: vec3<f32>, direction: vec3<f32>, spread: f32) -> vec4<f32> {
    var open = 1.0;
    var mirror = 0.0;
    var outward = vec2<f32>(0.0);
    var dominant = 0.0;
    let point = vec4<f32>(world, 1.0);
    let ray = vec4<f32>(direction, 0.0);
    let edge_softness = max(spread, MIRROR_EDGE);
    let count = i32(wake.hull.x);
    for (var i = 0; i < count; i++) {
        let base = i * HULL_STRIDE;
        let bound = wake.hull_data[base];
        let dx = world.x - bound.x;
        let dz = world.z - bound.y;
        if dx * dx + dz * dz >= bound.w {
            continue;
        }
        let across = wake.hull_data[base + 1];
        let row_y = wake.hull_data[base + 2];
        let row_z = wake.hull_data[base + 3];
        let shape = wake.hull_data[base + 4];
        let drive = wake.hull_data[base + 5];
        let shelter = wake.hull_data[base + 6];
        let lx = dot(across, point);
        let ly = dot(row_y, point);
        let lz = dot(row_z, point);
        let bow = shape.w;
        let stern = shape.w + drive.w;
        let starboard = lx >= 0.0;
        let strength = shelter.x;
        let reach = shelter.y;
        // Water farther out than the tallest side's reach takes no shelter from this hull.
        if !(abs(lx) < shape.z + reach && lz > bow - reach && lz < stern + reach) {
            continue;
        }
        let first_station = (bow - shape.x) / shape.y;
        let last_station = (stern - shape.x) / shape.y;
        // Breadth at the water's level, as the foam reads it, at the nearest station of the hull.
        let station = clamp((lz - shape.x) / shape.y, first_station, last_station);
        let first = min(floor(station), f32(PROFILE_STATIONS - 2));
        let along = station - first;
        let level = clamp((ly - PROFILE_LOWEST) / PROFILE_LEVEL_STEP, 0.0, f32(PROFILE_LEVELS - 1));
        let lower = min(floor(level), f32(PROFILE_LEVELS - 2));
        let rise = level - lower;
        let cell = base + HULL_PROFILE + i32(lower) * PROFILE_STATIONS + i32(first);
        let below = wake.hull_data[cell];
        let above = wake.hull_data[cell + PROFILE_STATIONS];
        let breadth = mix(mix(below.xy, below.zw, along), mix(above.xy, above.zw, along), rise);
        let side = select(breadth.y, breadth.x, starboard);
        let out_dist = max(abs(lx) - side, 0.05);
        let up = vec3<f32>(across.y, row_y.y, row_z.y);
        let wall = side_height(base, station, starboard, vec3<f32>(lx, ly, lz), side, up);
        let fade = (1.0 - smoothstep(wall * SHELTER_FADE, wall * SHELTER_REACH + 1e-3, out_dist)) * strength;
        // The view factor of the wall, both ends.
        let ratio = wall / out_dist;
        let k = 1.0 / sqrt(ratio * ratio + 1.0);
        let hidden = clamp((wall_end((lz - bow) / out_dist, k) + wall_end((stern - lz) / out_dist, k)) / (2.0 * PI), 0.0, 0.5);
        let hides = hidden * fade;
        open *= 1.0 - hides;
        // The mirrored ray: where it reaches the wall's plane, and whether it passes below the wall's top there.
        let local = vec3<f32>(dot(across, ray), dot(row_y, ray), dot(row_z, ray));
        let toward = select(local.x, -local.x, starboard);
        let flat_length = max(length(local.xz), 1e-4);
        let travel = min(out_dist / max(toward, 1e-3), 4000.0);
        let hit = lz + local.z * travel;
        let hit_station = clamp((hit - shape.x) / shape.y, first_station, last_station);
        // In the hull's frame, as the ray is: the side's plane and its top there are hers.
        let top = atan2(max(side_top(base, hit_station, starboard) - ly, 0.0), travel * flat_length) - atan2(max(local.y, 0.0), flat_length);
        let ends = smoothstep(bow - shape.y, bow + shape.y, hit) * (1.0 - smoothstep(stern - shape.y, stern + shape.y, hit));
        let met = smoothstep(-edge_softness, edge_softness, top) * smoothstep(0.0, 0.05, toward) * ends * fade;
        mirror = max(mirror, met);
        // The side that shelters this water most names the light the water sees on it.
        if max(met, hides) > dominant {
            dominant = max(met, hides);
            outward = normalize(across.xz * select(-1.0, 1.0, starboard) + vec2<f32>(1e-6, 0.0));
        }
    }
    return vec4<f32>(1.0 - open, mirror, outward);
}

// The water's light beside hulls (`surface/shelter.ts`): the side is lit by half the sky and by the sun where it faces
// it, and grey paint returns a share of that.
const HULL_ALBEDO: f32 = 0.3;
const SIDE_SKY: f32 = 0.5;
const SIDE_SUN: f32 = 1.0;

struct HullShelter {
    // The sky's irradiance on the water with what the sides hide replaced by the light they scatter.
    irradiance: vec3<f32>,
    // The share of the sky's light that still reaches the water.
    sky_share: f32,
    // The sides' exitance divided by π (their radiance), and the share of the mirrored lobe they fill.
    side_radiance: vec3<f32>,
    mirrored: f32,
}

// `shelter` from [`hull_shelter`]; `irradiance` the sky's on the horizontal; `sun_irradiance` the sun's on a surface
// facing it, from `sun`.
fn shelter_light(shelter: vec4<f32>, irradiance: vec3<f32>, sun_irradiance: vec3<f32>, sun: vec3<f32>) -> HullShelter {
    let hidden = shelter.x;
    let outward = vec3<f32>(shelter.z, 0.0, shelter.w);
    let sunlit = sun_irradiance * (max(dot(outward, sun), 0.0) * smoothstep(-0.1, 0.05, sun.y) * SIDE_SUN);
    let side = (irradiance * SIDE_SKY + sunlit) * HULL_ALBEDO;
    return HullShelter(mix(irradiance, side, hidden), 1.0 - hidden * (1.0 - HULL_ALBEDO * SIDE_SKY), side / PI, shelter.y);
}

// `sky`, mirrored by the water, where the mirrored lobe meets a side: never brighter than the sky it hides.
fn shelter_reflect(contact: HullShelter, sky: vec3<f32>) -> vec3<f32> {
    return mix(sky, min(sky, contact.side_radiance), contact.mirrored);
}

// ---- Together (`ShipWake`'s sampler) ------------------------------------------------------------------------------

// Height the wakes add to the sea at a grid point (vertex stage).
fn wake_height(xz: vec2<f32>) -> f32 {
    return wake_field(xz).x + bow_waves(xz.x, xz.y, true).height;
}

struct WakeSurface {
    // The wake's upward normal (the field's slope with the bow waves' added).
    normal: vec3<f32>,
    // The greatest of every foam: field breaking, trail, torpedo, bow waves, contact.
    foam: f32,
    bubbles: f32,
    // How much of the short waves the slick stills.
    slick: f32,
    // How young that foam is: 1 dense white water, 0 the lace of an old wake's streaks.
    young: f32,
    // The slope variance of the bow waves' crests too short to draw (`BowWaveSample::variance`).
    variance: f32,
    // How much of the light through the waves the churned water against a hull under way takes back (`hull_foam`).
    froth: f32,
    // How much of the foam's cover is drawn as its mean rather than the foam texture's lace (`soft` 1 its mean alone): a
    // splash's deposit and the field's foam read as soft blotches, not cut-out flakes of lace.
    soft: f32,
    // How much of that foam is one dense mass, the lace's holes closed (the foam field's thick foam, `SIM_SOLID`): its
    // flakes show only where it thins.
    mass: f32,
}

// Everything the fragment stage reads at grid point `xz` (the displaced point `world`); `footprint` from uniform flow.
fn wake_surface(xz: vec2<f32>, world: vec3<f32>, footprint: f32) -> WakeSurface {
    let field = wake_field(xz);
    var trail = trail_read(xz, footprint);
    // The foam field's foam takes the trails' and the splashes' place within its window; the surface draws them itself
    // beyond it, over its edge and where the field's cells have yet to hold what lies there (`sim_sample`). A splash's
    // deposit and the field's foam are soft.
    let sim = sim_sample(xz, footprint);
    var soft = 0.0;
    var mass = 0.0;
    if sim.weight < 1.0 {
        let splash = splash_read(xz, footprint);
        soft = select(0.0, mix(SPLASH_SOFT_OLD, SPLASH_SOFT, splash.young), splash.foam > trail.foam);
        trail = with_splashes(trail, splash);
    }
    if sim.weight > 0.0 {
        let drawn = sim_read(xz, footprint, sim.field);
        trail.foam = mix(trail.foam, drawn.foam, sim.weight);
        trail.young = mix(trail.young, drawn.young, sim.weight);
        // The bubble cloud under a trail is its own (the field's foam lies on top of it).
        trail.bubbles = max(trail.bubbles, drawn.bubbles * sim.weight);
        trail.slick = max(trail.slick, drawn.slick * sim.weight);
        soft = mix(soft, mix(SIM_SOFT.y, SIM_SOFT.x, drawn.young), sim.weight);
        mass = smoothstep(SIM_SOLID.x, SIM_SOLID.y, sim.field.x) * sim.weight;
    }
    // The young churned water's boils tilt it with the field's waves.
    let base = normalize(vec3<f32>(-field.y - trail.boil.x, 1.0, -field.z - trail.boil.y));
    let bow = bow_waves(xz.x, xz.y, false);
    let normal = normalize(vec3<f32>(base.x - bow.slope.x * base.y, base.y, base.z - bow.slope.y * base.y));
    let hull = hull_foam(xz.x, xz.y, world);
    let fresh = max(max(max(field.w, 0.0), torpedo_foam(xz)), hull.contact);
    let bow_foam = max(bow.foam, hull.wash);
    let bow_young = select(bow.young, BOW_RESIDUE_YOUNG, hull.wash > bow.foam);
    let other = max(fresh, bow_foam);
    // The trail's own foam keeps its age, and so does what a bow's breaker left behind it; every other foam here is
    // fresh white water.
    let young = select(select(1.0, bow_young, bow_foam > fresh), trail.young, trail.foam > other);
    // Boiling water is not stilled by the slick laid with it, and its chop roughens it.
    return WakeSurface(normal, max(other, trail.foam), max(max(trail.bubbles, bow.bubbles), hull.bubbles),
        trail.slick * (1.0 - trail.boil.z), young, bow.variance + trail.boil.z * RELIEF_ROUGHNESS, hull.froth, soft,
        select(0.0, mass, trail.foam > other));
}

// ---- The foam's look ------------------------------------------------------------------------------------------------
// A trail's and a splash's foam as the surface draws it (clumps, flow maps, a band's patches and masses, a splash's
// deposit), and the foam field's (src/foam_sim.rs), which takes their place within its window.

// Two patterns, each about uniform 0–1 (or noise of mean 0, `blend0`), mixed by `t` with the spread of either kept.
fn blend01(a: f32, b: f32, t: f32) -> f32 {
    return clamp(blend0(a - 0.5, b - 0.5, t) + 0.5, 0.0, 1.0);
}

fn blend0(a: f32, b: f32, t: f32) -> f32 {
    return (a * (1.0 - t) + b * t) / sqrt((1.0 - t) * (1.0 - t) + t * t);
}

// Thin foam in clumps: a field over the water, about uniform 0–1, high in clumps about CLUMP_SIZE metres
// across, which its coarse octave swells and shrinks from about 2 m to 10 m, with holes between. `clumped` keeps about
// the share of the water where it is highest, thickening into each clump over CLUMP_SOFT of the field (soft, torn
// edges, never cut-out flakes); where the pixel is wider than the clumps it takes that share, their mean.
const CLUMP_SIZE: f32 = 4.0;
const CLUMP_SOFT: f32 = 0.5;

fn clump_field(p: vec2<f32>) -> f32 {
    let coarse = gradient_noise(p / (CLUMP_SIZE * 2.7), 811u);
    let body = gradient_noise(p / CLUMP_SIZE + 19.3, 812u);
    let fine = gradient_noise(p / (CLUMP_SIZE * 0.4) + 47.1, 813u);
    return clamp((coarse * 0.9 + body + fine * 0.3) * 0.87 + 0.5, 0.0, 1.0);
}

fn clumps_resolved(footprint: f32) -> f32 {
    return 1.0 - smoothstep(0.3, 0.9, footprint / CLUMP_SIZE);
}

fn clumped(cover: f32, field: f32, resolved: f32) -> f32 {
    let share = clamp(cover, 0.0, 1.0);
    let threshold = 1.0 - share;
    return mix(share, smoothstep(threshold - CLUMP_SOFT * 0.3, threshold + CLUMP_SOFT, field), resolved)
        * smoothstep(0.0, 0.01, share);
}

// Two-phase flow maps (Valve's water, Vlachos 2010): the detail at a point is read where its water lay a moment
// ago, `velocity` times the time since its phase began, in two phases half a FLOW_PERIOD apart, each faded out as it
// starts over. A slow noise sets each place's phase, so the restarts never pulse together; the second phase reads its
// detail FLOW_SHIFT off the first's, so the two never show the same foam.
const FLOW_PERIOD: f32 = 6.0;
const FLOW_SHIFT: vec2<f32> = vec2<f32>(37.1, -21.7);

struct FlowPhases {
    from_a: vec2<f32>,
    from_b: vec2<f32>,
    // The first phase's share.
    weight: f32,
}

fn flow_phases(at: vec2<f32>, velocity: vec2<f32>, time: f32) -> FlowPhases {
    let phase = fract(time / FLOW_PERIOD + gradient_noise(at / 45.0, 831u) * 2.0);
    let second = fract(phase + 0.5);
    return FlowPhases(at - velocity * (phase * FLOW_PERIOD), at - velocity * (second * FLOW_PERIOD) + FLOW_SHIFT,
        1.0 - abs(2.0 * phase - 1.0));
}

// A trail's water as the flow maps carry it (m/s): out from the band's middle as fast as it is churned (FLOW_SPREAD), a
// following current toward the hull close astern (FLOW_FOLLOW), slow eddies TRAIL_EDDY of her beam across
// (FLOW_EDDY, curl noise), and the surface's drift. Which way the band's middle and its younger water lie is read off the
// atlas about `at` in the strongest tile.
const FLOW_SPREAD: f32 = 0.7;
const FLOW_FOLLOW: f32 = 1.4;
const FLOW_EDDY: f32 = 0.35;
const TRAIL_EDDY: f32 = 0.4;

fn trail_velocity(at: vec2<f32>, tile: i32, turbulence: f32, band: f32, beam: f32) -> vec2<f32> {
    let bounds = wake.tiles[tile];
    let step = 1.0 / (f32(textureDimensions(trail_atlas).x) / TILES);
    let uv = (at - bounds.xy) / WAKE_EXTENT + 0.5;
    let here = textureSampleLevel(trail_atlas, wake_sampler, (uv + bounds.zw) / TILES, 0.0);
    let east = textureSampleLevel(trail_atlas, wake_sampler, (uv + vec2<f32>(step, 0.0) + bounds.zw) / TILES, 0.0);
    let south = textureSampleLevel(trail_atlas, wake_sampler, (uv + vec2<f32>(0.0, step) + bounds.zw) / TILES, 0.0);
    // The band's encoding rises toward its middle, its churn toward the hull.
    let middle = vec2<f32>(east.b - here.b, south.b - here.b);
    let younger = vec2<f32>(east.r - here.r, south.r - here.r);
    let outward = select(vec2<f32>(0.0), -normalize(middle), dot(middle, middle) > 1e-10);
    var ahead = vec2<f32>(-outward.y, outward.x);
    ahead *= select(-1.0, 1.0, dot(ahead, younger) >= 0.0);
    if dot(outward, outward) < 0.5 {
        ahead = select(vec2<f32>(0.0), normalize(younger), dot(younger, younger) > 1e-10);
    }
    let p = at / (beam * TRAIL_EDDY);
    let psi = gradient_noise(p, 841u);
    let curl = vec2<f32>(gradient_noise(p + vec2<f32>(0.0, 0.1), 841u) - psi, psi - gradient_noise(p + vec2<f32>(0.1, 0.0), 841u)) * 10.0;
    return outward * (FLOW_SPREAD * turbulence * clamp(band, 0.0, 1.3)) + ahead * (FLOW_FOLLOW * turbulence * turbulence)
        + curl * (FLOW_EDDY * (0.3 + turbulence)) + wake.oil_flow.xy;
}

// The young water's eddies (`trail_read`'s) at `p`, still: the flow carries them.
fn trail_eddies(p: vec2<f32>, f: f32, fine: f32) -> f32 {
    let coarse = 1.0 - fine;
    return mx_noise_float(vec3<f32>(p * f, 0.0)) * (coarse * 0.56)
        + mx_noise_float(vec3<f32>(p * (f * 2.4) + 17.0, 0.0)) * (coarse * 0.44)
        + mx_noise_float(vec3<f32>(p * (f * 6.0) + 41.0, 0.0)) * fine;
}

// A band's foam. Along it the white comes in patches every TRAIL_PATCH beams or so, the band wider and
// narrower with them (TRAIL_PATCH_WIDTH of it), thinning to TRAIL_PATCH_LEAST of its cover between them; close astern
// it lies in masses on the boils (`boil`, RELIEF_LUMP of her beam) over TRAIL_MASS_COVER of the game's cover, darker
// water between; older, its braided rope and lace gather in clumps (TRAIL_CLUMPING). `knobs`: the trail's foam start,
// full and cover. Returns the cover and the pattern it covers (`torn_foam`).
const TRAIL_PATCH: f32 = 1.8;
const TRAIL_PATCH_LEAST: f32 = 0.62;
const TRAIL_PATCH_WIDTH: f32 = 0.2;
const TRAIL_MASS_COVER: f32 = 0.92;
const TRAIL_MASSES: f32 = 0.7;
const TRAIL_CLUMPING: f32 = 0.55;

fn banded_foam(world: vec2<f32>, at: vec2<f32>, across: f32, turbulence: f32, young: f32, beam: f32, streaks: f32,
    eddy_pattern: f32, clumps: f32, knobs: vec3<f32>) -> vec2<f32> {
    let along = gradient_noise(world / (beam * TRAIL_PATCH) + 71.3, 851u)
        + gradient_noise(world / (beam * TRAIL_PATCH * 0.45) + 13.9, 852u) * 0.45;
    let patched = smoothstep(-0.18, 0.16, along);
    let width = across * mix(1.0 + TRAIL_PATCH_WIDTH, 1.0 - TRAIL_PATCH_WIDTH, patched);
    let middle = 1.0 - smoothstep(CORE_WIDTH * 0.35, CORE_WIDTH, width);
    let inside = 1.0 - smoothstep(0.75, 1.0, width);
    let edge = exp(-pow((width - EDGE_AT) / EDGE_WIDTH, 2.0));
    let old_cover = smoothstep(0.0, knobs.y, turbulence) * max(max(middle * CORE_COVER, inside * LACE_COVER), edge * EDGE_COVER);
    let young_cover = smoothstep(knobs.x, knobs.y, turbulence) * knobs.z * TRAIL_MASS_COVER
        * (1.0 - smoothstep(0.35, 0.9, width) * YOUNG_SIDES) * (1.0 - smoothstep(0.8, 1.1, width));
    let cover = mix(old_cover, max(young_cover, edge * EDGE_COVER), young) * mix(TRAIL_PATCH_LEAST, 1.0, patched);
    // The boils' lumps, as the relief raises them: white on their tops.
    let masses = clamp(boil(at, beam * RELIEF_LUMP, 1.0) * 0.4 + 0.5, 0.0, 1.0);
    let young_pattern = blend01(blend01(eddy_pattern, streaks, YOUNG_STREAKS), masses, TRAIL_MASSES);
    let old_pattern = blend01(streaks, clumps, TRAIL_CLUMPING);
    return vec2<f32>(cover, mix(old_pattern, young_pattern, young));
}

// A splash's own deposit (src/splash_foam.rs `SplashDeposits`, `wake.splashes`). Its radius at `age` (s) for a round
// of `scale`, as `splash_foam::splash_radius` has it: the base surge pushes it out, then it creeps as it spreads.
const SPLASH_RADIUS_START: f32 = 7.0;
const SPLASH_RADIUS_SURGE: f32 = 25.0;
const SPLASH_SURGE_SECONDS: f32 = 2.8;
const SPLASH_RADIUS_CREEP: f32 = 0.22;
// Lobes about its body, and how far its outline frays (a share of its radius).
const SPLASH_LOBES: i32 = 5;
const SPLASH_FRAY: f32 = 0.3;
// The cover of its white water (as the column's water falls back), and of the clumps and streaks it breaks into.
const SPLASH_WHITE_COVER: f32 = 0.92;
const SPLASH_BROKEN_COVER: f32 = 0.4;
// Bubbles under its young white water (the pale turquoise, gone by 30 s), the faint pale water of its scar, and how far
// the scar stills the short waves (a trail's slick stills 0.75 of them).
const SPLASH_TINT: f32 = 0.4;
// How much larger the white water's clumps (and holes) are than those it breaks into, and how far the finer clumps tear
// its edge and hole it.
const SPLASH_MASS: f32 = 2.0;
const SPLASH_TORN: f32 = 0.4;
// The share of its foam's cover drawn as a soft mean (`WakeSurface::soft`): of its white water, and of what that breaks
// into.
const SPLASH_SOFT: f32 = 0.62;
const SPLASH_SOFT_OLD: f32 = 0.45;
const SPLASH_PALE: f32 = 0.06;
const SPLASH_STILL: f32 = 0.35;

fn splash_radius(age: f32, scale: f32) -> f32 {
    return scale * (SPLASH_RADIUS_START + SPLASH_RADIUS_SURGE * (1.0 - exp(-age / SPLASH_SURGE_SECONDS))
        + SPLASH_RADIUS_CREEP * age);
}

// How much of a splash's deposit lies at `q` (m from where it fell, carried by the drift), 0–1: a body and
// SPLASH_LOBES lobes about it, each its own size, weight and place from the splash's `seed`, pushed out with `radius`;
// the column's water blown `skew` downwind as it `fell` (0–1), the downwind lobes the larger.
fn splash_deposit(q: vec2<f32>, radius: f32, seed: u32, skew: vec2<f32>, fell: f32) -> f32 {
    let downwind = select(vec2<f32>(0.0), normalize(skew), dot(skew, skew) > 1e-6);
    let turn = noise_hash(0, 0, seed) * TAU;
    var sum = 0.0;
    for (var k = 0; k <= SPLASH_LOBES; k++) {
        let a = noise_hash(k, 1, seed);
        let b = noise_hash(k, 2, seed);
        let c = noise_hash(k, 3, seed);
        let angle = turn + f32(k) * (TAU / f32(SPLASH_LOBES)) + (a - 0.5) * 1.1;
        let toward = vec2<f32>(cos(angle), sin(angle));
        let lee = max(dot(toward, downwind), 0.0);
        let out = select(mix(0.3, 0.62, b), 0.12 * b, k == 0);
        let size = select(mix(0.32, 0.55, c), 0.62, k == 0) * (1.0 + 0.3 * lee);
        let centre = toward * (radius * out) + skew * (fell * mix(0.6, 1.0, lee));
        let off = (q - centre) / (radius * size);
        sum += mix(0.55, 1.0, a) * exp(-dot(off, off));
    }
    return 1.0 - exp(-1.7 * sum);
}

// A splash's deposit at `q` from where it fell, `age` seconds after, its outline frayed (SPLASH_FRAY of its radius):
// what the surface draws and the foam field's cells taken in start from.
fn splash_laid(q: vec2<f32>, age: f32, scale: f32, seed: u32, skew: vec2<f32>) -> f32 {
    let radius = splash_radius(age, scale);
    let at = q / (radius * 0.35);
    let fray = vec2<f32>(gradient_noise(at, 821u), gradient_noise(at + 31.7, 822u));
    return splash_deposit(q + fray * (radius * SPLASH_FRAY), radius, seed, skew, smoothstep(1.0, 6.0, age));
}

// A splash's white water at `age`: it grows as the column's water falls back (1.3–4.5 s), densest until 9 s and gone
// by 15 s.
fn splash_white(age: f32) -> f32 {
    return smoothstep(1.3, 4.5, age) * (1.0 - smoothstep(9.0, 15.0, age));
}

// The cover of a splash's foam where `deposit` of it lies at `age`: its white water, then what that breaks into
// (thinning from 12 s, gone by two minutes).
fn splash_cover(deposit: f32, age: f32) -> f32 {
    let broken = smoothstep(7.0, 13.0, age) * exp(-max(age - 12.0, 0.0) / 22.0) * (1.0 - smoothstep(85.0, 118.0, age));
    return max(min(deposit * 1.5, 1.0) * splash_white(age) * SPLASH_WHITE_COVER, deposit * broken * SPLASH_BROKEN_COVER);
}

// How far a splash's scar stills the short waves where `deposit` of it lies at `age`.
fn splash_still(deposit: f32, age: f32) -> f32 {
    return deposit * smoothstep(6.0, 20.0, age) * (1.0 - smoothstep(70.0, 118.0, age)) * SPLASH_STILL;
}

// One splash's foam at `q` from its deposit's centre, `age` seconds after it fell, its clumps laid on the deposit (they
// drift with it) and drawn out along the `wind` into streaks as they age, their detail carried out by the surge.
fn splash_foam(q: vec2<f32>, age: f32, scale: f32, seed_value: f32, skew: vec2<f32>, wind: vec2<f32>, resolved: f32)
    -> TrailSample {
    var out = TrailSample(0.0, 0.0, 0.0, 0.0, vec3<f32>(0.0));
    let seed = noise_seed(u32(seed_value * 16777215.0));
    let deposit = splash_laid(q, age, scale, seed, skew);
    if deposit < 0.002 {
        return out;
    }
    let radius = splash_radius(age, scale);
    let white = splash_white(age);
    // The white water is one lumpy mass with a few holes, its clumps SPLASH_MASS times the size of what it breaks into.
    let cover = splash_cover(deposit, age);
    let local = vec2<f32>(dot(q, wind), dot(q, vec2<f32>(-wind.y, wind.x)));
    let streaking = vec2<f32>(1.0 / (1.0 + 2.0 * smoothstep(10.0, 45.0, age)), 1.0) / mix(1.0, SPLASH_MASS, white);
    let shift = vec2<f32>(noise_hash(7, 7, seed), noise_hash(8, 8, seed)) * 500.0;
    // Its detail carried out by the surge, which spreads the deposit as a whole (the water at q lay at q R(then) /
    // R(now)), and with it on the drift.
    let phase = fract(age / FLOW_PERIOD + noise_hash(9, 9, seed));
    let second = fract(phase + 0.5);
    let from_a = local * max(splash_radius(max(age - phase * FLOW_PERIOD, 0.0), scale) / radius, 0.55);
    let from_b = local * max(splash_radius(max(age - second * FLOW_PERIOD, 0.0), scale) / radius, 0.55);
    var field = blend01(clump_field(from_a * streaking + shift), clump_field(from_b * streaking + shift + FLOW_SHIFT),
        abs(2.0 * phase - 1.0));
    // The white water's mass torn at its edge and holed by the finer clumps it will break into.
    field = blend01(field, clump_field(local * (streaking * SPLASH_MASS) + shift + 211.0), SPLASH_TORN * white);
    out.foam = clumped(cover, field, resolved) * mix(1.0, 0.4, smoothstep(12.0, 50.0, age));
    out.young = smoothstep(0.1, 0.7, white);
    let patchy = smoothstep(0.3, 0.7, clump_field(local / 3.0 + shift + 101.0));
    let tint = smoothstep(1.0, 3.5, age) * (1.0 - smoothstep(12.0, 30.0, age)) * mix(0.35, 1.0, white) * patchy * SPLASH_TINT;
    let pale = smoothstep(10.0, 25.0, age) * (1.0 - smoothstep(60.0, 115.0, age)) * SPLASH_PALE;
    out.bubbles = deposit * max(tint, pale);
    out.slick = splash_still(deposit, age);
    return out;
}

// Every splash's foam at `at`: the densest, with its age; the most bubbles and stilling.
fn splash_read(at: vec2<f32>, footprint: f32) -> TrailSample {
    var out = TrailSample(0.0, 0.0, 0.0, 0.0, vec3<f32>(0.0));
    let count = i32(wake.foam_state.x);
    let drift = wake.oil_flow.xy;
    let wind = select(vec2<f32>(1.0, 0.0), normalize(drift), dot(drift, drift) > 1e-10);
    let resolved = clumps_resolved(footprint);
    for (var i = 0; i < count; i++) {
        let row = wake.splashes[2 * i];
        let more = wake.splashes[2 * i + 1];
        let q = at - row.xy;
        if dot(q, q) > more.w * more.w {
            continue;
        }
        let splash = splash_foam(q, row.z, row.w, more.x, more.yz, wind, resolved);
        if splash.foam > out.foam {
            out.foam = splash.foam;
            out.young = splash.young;
        }
        out.bubbles = max(out.bubbles, splash.bubbles);
        out.slick = max(out.slick, splash.slick);
    }
    return out;
}

// A trail's sample with the splashes' over it.
fn with_splashes(trail: TrailSample, splash: TrailSample) -> TrailSample {
    var out = trail;
    if splash.foam > trail.foam {
        out.foam = splash.foam;
        out.young = splash.young;
    }
    out.bubbles = max(trail.bubbles, splash.bubbles);
    out.slick = max(trail.slick, splash.slick);
    return out;
}

// The foam field (src/foam_sim.rs, shaders/foam_sim.wgsl) at `at`: its (thickness, turbulence, residue), bilinear from
// the newest state, and how far it holds there: its cells' validity (0 as the window takes a cell in, 1 a few seconds
// later), fading to 0 from SIM_EDGE to SIM_GONE of the window's half-edge (and 0 with no field), and from SIM_ALIAS.x to
// SIM_ALIAS.y cells a pixel (`footprint`, m): the field has no mips, so where a pixel spans a few of its cells they would
// shimmer, and the drawn foam, which takes its mean there, draws it instead.
const SIM_EDGE: f32 = 0.84;
const SIM_GONE: f32 = 0.97;
const SIM_ALIAS: vec2<f32> = vec2<f32>(1.5, 3.0);

struct SimSample {
    field: vec3<f32>,
    weight: f32,
}

fn sim_texel(cell: vec2<i32>, size: i32, layer: i32) -> vec4<f32> {
    return textureLoad(foam_sim, ((cell % size) + size) % size, layer, 0);
}

fn sim_sample(at: vec2<f32>, footprint: f32) -> SimSample {
    var out = SimSample(vec3<f32>(0.0), 0.0);
    if wake.foam_state.z < 0.5 {
        return out;
    }
    let n = wake.sim_grid.y;
    // Cells from the centre cell's corner, then texel coordinates (texel centres at whole numbers).
    let cells = (at - wake.sim.xy) / wake.sim_grid.x;
    out.weight = (1.0 - smoothstep(SIM_EDGE, SIM_GONE, max(abs(cells.x), abs(cells.y)) / (n * 0.5)))
        * (1.0 - smoothstep(SIM_ALIAS.x, SIM_ALIAS.y, footprint / wake.sim_grid.x));
    if out.weight <= 0.0 {
        return out;
    }
    let s = cells + wake.sim.zw - 0.5;
    let base = floor(s);
    let f = s - base;
    let cell = vec2<i32>(base);
    let size = i32(n);
    let layer = i32(wake.foam_state.y);
    let a = sim_texel(cell, size, layer);
    let b = sim_texel(cell + vec2<i32>(1, 0), size, layer);
    let c = sim_texel(cell + vec2<i32>(0, 1), size, layer);
    let d = sim_texel(cell + vec2<i32>(1, 1), size, layer);
    let field = mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
    out.field = field.rgb;
    out.weight *= field.a;
    return out;
}

// The field's foam as the surface draws it. Its thickness sets the white water's cover (SIM_THIN to SIM_FULL) over the
// drawn foam's clumps, carried by the drift: thin foam is a few clumps (`sim_clumps`), so the foam's edges break up
// where its thickness falls off, and dense where it is thick. Its turbulence says how young it is (SIM_YOUNG); the
// bubbles under it come from both, and the residue left as it dies is a pale, stilled scar.
const SIM_THIN: f32 = 0.03;
const SIM_FULL: f32 = 2.0;
const SIM_YOUNG: vec2<f32> = vec2<f32>(0.12, 0.55);
const SIM_BUBBLES: f32 = 0.45;
const SIM_SCAR: f32 = 0.16;
const SIM_STILL: f32 = 0.4;
// Thick foam is one mass (its texture's billows), thin foam soft clumps: the thicknesses between.
const SIM_MASS: vec2<f32> = vec2<f32>(0.9, 2.4);
// Thick foam is dense white water however long ago its water was churned: it reads young until it thins through these
// thicknesses, then turns to the lace of old foam.
const SIM_DENSE: vec2<f32> = vec2<f32>(0.8, 1.8);
// And through these its lace's holes close (`WakeSurface::mass`): flakes only where it thins.
const SIM_SOLID: vec2<f32> = vec2<f32>(1.0, 2.4);
// The share of its cover drawn as a soft mean (`WakeSurface::soft`): young foam's, old foam's.
const SIM_SOFT: vec2<f32> = vec2<f32>(0.35, 0.15);
// Thin foam lies in clumps of white water, `share` of the water where the clumps' field is highest, each torn over at
// most SIM_TORN of that field at its edge (the clumps' own torn edges, never a faint film), or that share as their mean
// where a pixel is wider than the clumps (`resolved` 0).
const SIM_TORN: f32 = 0.2;

fn sim_clumps(share: f32, field: f32, resolved: f32) -> f32 {
    let s = clamp(share, 0.0, 1.0);
    let threshold = 1.0 - s;
    let width = max(min(SIM_TORN, min(s, threshold)), 1e-4);
    return mix(s, smoothstep(threshold - width, threshold + width, field), resolved) * smoothstep(0.0, 0.01, s);
}

fn sim_read(at: vec2<f32>, footprint: f32, field: vec3<f32>) -> TrailSample {
    var out = TrailSample(0.0, 0.0, 0.0, 0.0, vec3<f32>(0.0));
    let thickness = field.x;
    let turbulence = field.y;
    let residue = field.z;
    if thickness + residue < 1e-3 {
        return out;
    }
    let cover = smoothstep(SIM_THIN, SIM_FULL, thickness);
    let drift = flow_phases(at, wake.oil_flow.xy, wake.trail.y);
    let detail = blend01(clump_field(drift.from_a), clump_field(drift.from_b), 1.0 - drift.weight);
    out.foam = mix(sim_clumps(cover, detail, clumps_resolved(footprint)), cover, smoothstep(SIM_MASS.x, SIM_MASS.y, thickness));
    out.young = max(smoothstep(SIM_YOUNG.x, SIM_YOUNG.y, turbulence), smoothstep(SIM_DENSE.x, SIM_DENSE.y, thickness));
    let patchy = smoothstep(0.25, 0.75, detail);
    out.bubbles = min(smoothstep(0.08, 0.8, thickness) * mix(0.25, 1.0, turbulence) * mix(0.6, 1.0, patchy) * SIM_BUBBLES
        + smoothstep(0.05, 0.5, residue) * patchy * SIM_SCAR, 1.0);
    out.slick = smoothstep(0.05, 0.6, residue) * SIM_STILL;
    return out;
}
