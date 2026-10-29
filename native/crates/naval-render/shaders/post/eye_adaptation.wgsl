// The eye's adaptation to what fills the view, both ways: the frame is metered on a 16 x 16 grid (4 x 4 reads a cell,
// log2 of each cell's mean luminance), each cell held within a few stops of the sky's mean radiance (`env_radiance(up,
// 1)` from naval_sky), centre-weighted. Against the mean the clear sky would have at this hour (the sky mean's w; the
// sky's own mean when the host has none) the eye opens up for what a view darker than the reference lacks
// (`strength`), so an overcast noon comes out near a clear one while dusk and night stay as dark as their sky; against
// its own sky it closes down for what a view brighter than that sky has over it (`close`, from `close_above`), as sunlit
// cloud tops from above, while a view of the sky itself keeps the fixed exposure. Two limits keep the opening an eye's:
// it never lifts the view's brightest tenth (against the clear sky) past `highlight`, so a sunlit hull filling half a
// view of dark sea, or an overcast sky, does not blow out; and it fades out from day to night: through civil twilight
// by the host's sun (`daylight`, `naval_render::post::eye_daylight`), so it opens for a storm at dusk as a camera
// does and true night keeps the fixed exposure, or without a sun as the clear sky darkens from `light_from` to
// `dark_from` (log2 luminance). The same ceiling closes it for glare (`glare`): a view whose brightest tenth stands over
// it (into the sun, the aureole and the sea's glitter; after dark a salvo, a burning ship, the air about a star shell)
// stops down for the excess, so the bright sky keeps its colour and a light does not white the frame out. After dark the
// ceiling is a clear noon's highlights (`noon_clear`): the night's lift brought the moonlit scene to the screen, and the
// dark-adapted eye may close as far as back to the day's sensitivity (`night_reach`, the lift in stops). Closing, the
// luminance it is adapted to moves toward the goal's (`darken`); opening, its gain (`brighten`, after dark
// `night_recover`). With the eye off (`allow` 0) the night's opening stays (`dark_adapt`): it neither meters the view by
// day nor closes for glare. The display multiplies radiance by exp2(state[0]). Its Rust twin is
// `naval_render::post::EyeUniforms` (and `reference_adapt` on the CPU).
#import naval::color::luminance

struct EyeParams {
    reference: f32,
    strength: f32,
    max_stops: f32,
    ceiling: f32,
    floor_stops: f32,
    brighten: f32,
    darken: f32,
    dt: f32,
    // 1 to adapt; 0 eases back to the fixed exposure (the setting off, or under water).
    allow: f32,
    // 1 settles at once on the target: a capture must not depend on the frames before it.
    snapped: f32,
    // The most it closes down, in stops, the share of a brighter view's excess it closes for, and the brightness over
    // its own sky (stops) from which it closes.
    max_down: f32,
    close: f32,
    close_above: f32,
    // The brightest tenth's ceiling (stops over the clear sky's mean), and the clear sky's log2 luminance from which
    // the eye may open (`dark_from`) and at which it may open fully (`light_from`). Scalars, not a vec3 (which would
    // align to 16 bytes).
    highlight: f32,
    dark_from: f32,
    light_from: f32,
    // The developer panel's exposure (`ExposureControl`): `hold` 1 keeps the stops where they are, `fixed` 1 sets them
    // to `fixed_stops`. Metering goes on either way, so the readings stay live.
    hold: f32,
    fixed: f32,
    fixed_stops: f32,
    // Share of the brightest tenth's excess over its ceiling it closes down for (glare: into the sun; lights at night).
    glare: f32,
    // The share of its opening the eye may make by the host's sun (`eye_daylight`), or −1 to follow the clear sky's
    // luminance (`dark_from` to `light_from`).
    daylight: f32,
    // log2 of the night's lift on real light (`naval_render::lights::night_lift`; 0 by day), the stops past `max_down`
    // the dark-adapted eye may close; and log2 of a clear noon's clear-sky mean (`naval_render::post::NOON_CLEAR_SKY`).
    // The stops the dark-adapted eye opens by after dark for a night lit less than a full moon
    // (`naval_render::post::night_open`, at most `NIGHT_OPEN`; 0 by day).
    night_reach: f32,
    noon_clear: f32,
    // The night's light at the sea (lux) the dark-adapted eye opens for.
    night_light: f32,
    // After dark the ceiling's stops over a clear noon's clear-sky mean (`night_highlight`), coming in as the night does
    // (`night`: 0 by day and through dusk, toward 1 as the twilight fades); padded to 112 bytes.
    night_highlight: f32,
    night: f32,
    // The dark adaptation (`naval_render::post::night_open`): the most it opens, how far under the ceiling a view's
    // brightest tenth stops it, the light from which it opens and the rods' dark light (lux), the stops it opens further
    // for each the clouds take of the sky's light. Padded to 128 bytes.
    night_cap: f32,
    night_lit: f32,
    night_from: f32,
    night_dark: f32,
    night_clouds: f32,
    // The least a dark view is shown at after dark (log2 luminance): the eye opens at least until the view's
    // centre-weighted mean stands there.
    night_view: f32,
    // 1 when the eye off keeps the night's opening, and the time constant (s) at which the eye opens after dark (after a
    // glare, back to the night's opening); padded to 144 bytes.
    dark_adapt: f32,
    night_recover: f32,
    // The stops the brightest tenth may stand over the ceiling by day before glare closes the eye, by how far the view
    // looks from the sun and the sun's height (`EyeAdaptationSettings::glare_away`: sunlit cloud tops from aloft); the
    // darkest a view counts against its sky for the night's view floor, a stop past one at the horizon
    // (`night_view_under`).
    glare_room: f32,
    night_view_under: f32,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> cells: array<f32, 256>;
// Stops open, the metered brightness, the target, the brightest tenth (stops over the clear sky).
@group(0) @binding(2) var<storage, read_write> state: array<f32, 4>;
@group(0) @binding(3) var<uniform> params: EyeParams;
// The sky's mean radiance (rgb), linear, and the clear sky's mean luminance (w; 0 when unknown).
@group(0) @binding(4) var<storage, read> sky_mean: vec4<f32>;

const GRID: u32 = 16u;
const READS: u32 = 4u;
// The brightest tenth's histogram: quarter-stop bins from 16 stops under the clear sky's mean to 34 over it (a fire's
// flames beside the eye on a moonless night stand some 25 over: `BINS` in post.rs).
const BINS: u32 = 200u;
const BIN_STEPS: f32 = 4.0;
const BIN_FLOOR: f32 = -16.0;
// After dark the brightest fiftieth of a view (a burning ship's flames, a flash's fireball) stands at most this many stops
// over the ceiling the brightest tenth keeps to: the display's white, AgX's top 4 stops over middle grey, about 2 over a
// noon's highlights (`PEAK_HEADROOM` in post.rs).
const PEAK_HEADROOM: f32 = 2.0;

fn bin_of(stops: f32) -> u32 {
    return u32(clamp(floor((stops - BIN_FLOOR) * BIN_STEPS), 0.0, f32(BINS) - 1.0));
}

@compute @workgroup_size(64)
fn meter(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if index >= GRID * GRID {
        return;
    }
    let size = vec2<f32>(textureDimensions(scene));
    let cell = vec2<f32>(f32(index % GRID), f32(index / GRID));
    var sum = 0.0;
    for (var y = 0u; y < READS; y++) {
        for (var x = 0u; x < READS; x++) {
            let read = vec2<f32>((f32(x) + 0.5) / f32(READS), (f32(y) + 0.5) / f32(READS));
            let at = (cell + read) / f32(GRID) * size;
            sum += luminance(textureLoad(scene, vec2<i32>(min(at, size - 1.0)), 0).rgb);
        }
    }
    cells[index] = log2(max(sum / f32(READS * READS), 1e-8));
}

@compute @workgroup_size(1)
fn adapt() {
    let sky = log2(max(luminance(sky_mean.rgb), 1e-8));
    let clear = select(sky, log2(max(sky_mean.w, 1e-8)), sky_mean.w > 0.0);
    var sum = 0.0;
    var weights = 0.0;
    for (var i = 0u; i < GRID * GRID; i++) {
        // Centre-weighted: the middle of the view counts about seven times its corners.
        let offset = (vec2<f32>(f32(i % GRID), f32(i / GRID)) + 0.5) / f32(GRID) - 0.5;
        let weight = exp(dot(offset, offset) * -8.0) + 0.15;
        sum += clamp(cells[i] - sky, params.floor_stops, params.ceiling) * weight;
        weights += weight;
    }
    // The view against its own sky, and against the clear sky.
    let own = sum / weights;
    let metered = own + sky - clear;
    let opening = params.reference - metered;
    let closing = own - params.close_above;
    // The brightest tenth and fiftieth: the quarter-stop bins (−16…+24 stops against the clear sky) holding the 26th
    // and the 5th brightest cells, their lower edges (a histogram: one pass over the cells and one down the bins).
    var counts: array<u32, BINS>;
    for (var i = 0u; i < GRID * GRID; i++) {
        counts[bin_of(cells[i] - clear)] += 1u;
    }
    var above = 0u;
    var tenth = 0u;
    var fiftieth = 0u;
    for (var b = i32(BINS) - 1; b >= 0; b--) {
        above += counts[b];
        if above >= 5u && fiftieth == 0u {
            fiftieth = u32(b);
        }
        if above >= 26u {
            tenth = u32(b);
            break;
        }
    }
    let bright = f32(tenth) / BIN_STEPS + BIN_FLOOR;
    let peak = f32(fiftieth) / BIN_STEPS + BIN_FLOOR;
    let daylight = select(smoothstep(params.dark_from, params.light_from, clear), params.daylight,
        params.daylight >= 0.0);
    // The ceiling on the brightest tenth: `highlight` over this hour's clear sky by day, over a clear noon's after dark.
    let ceiling = params.highlight + (params.noon_clear - clear) * (1.0 - daylight)
        + (params.night_highlight - params.highlight) * clamp(params.night, 0.0, 1.0);
    var aim = 0.0;
    if opening > 0.0 {
        let headroom = max(ceiling - bright, 0.0);
        aim = min(min(opening * params.strength, params.max_stops * daylight), headroom);
    } else if closing > 0.0 {
        aim = -closing * params.close;
    }
    // After dark the eye is dark-adapted to the night's light (`night_light`) by Weber's law with the rods' dark light,
    // and to a share of what the clouds take of the sky, as far as the brightest tenth stays `night_lit` under the
    // ceiling and the fiftieth within its headroom. The view floor meters a view no darker than a stop past one at the
    // horizon, so looking down on the sea, no sky in view, opens it little further.
    let weber = max(log2((params.night_from + params.night_dark) / max(params.night_light + params.night_dark, 1e-9)), 0.0);
    let open = clamp(max(weber + max(clear - sky, 0.0) * params.night_clouds,
        params.night_view - (max(own, params.night_view_under) + sky)), 0.0, params.night_cap);
    let night = open * clamp(params.night, 0.0, 1.0) * (1.0 - daylight);
    let lit = min(ceiling - params.night_lit - bright, ceiling + PEAK_HEADROOM - peak);
    aim += min(max(lit - aim, 0.0), night);
    // The eye off keeps to the night's opening alone, as far as the view's lights let it: never under the fixed exposure.
    let rest = min(max(lit, 0.0), night);
    // Glare (into the sun, the aureole and the sea's glitter; lights filling the view at night): it stops down until the
    // brightest tenth is back at the ceiling, whatever else the view holds, as a pupil and a camera do.
    // After dark the brightest fiftieth too, kept within the display's white: a fire's flames, art-bright beside the
    // light they give, stand 8–9 stops over the smoke they light, and bloomed a lit fog over the whole view.
    let glare = max(max(bright - ceiling - params.glare_room * daylight, (peak - ceiling - PEAK_HEADROOM) * (1.0 - daylight)),
        0.0) * params.glare;
    if glare > 0.0 {
        aim = min(aim, -glare);
    }
    let reach = params.max_down + params.night_reach * (1.0 - daylight);
    let goal = select(clamp(aim, -reach, max(params.max_stops, night)) * params.allow, rest,
        params.allow < 0.5 && params.dark_adapt > 0.5);
    let stops = state[0];
    // Closing, the luminance the eye is adapted to (2^−stops) moves toward the goal's; opening, its gain (2^stops), after
    // dark at `night_recover`, so a flash's glare passes in a moment.
    let closes = goal < stops;
    let opening_tau = mix(params.brighten, params.night_recover, clamp(params.night, 0.0, 1.0));
    let keep = select(exp(-params.dt / opening_tau), exp(-params.dt / params.darken), closes);
    let rate = max(1.0 - keep, params.snapped);
    let way = select(1.0, -1.0, closes);
    let now = exp2(way * stops);
    var next = way * log2(now + (exp2(way * goal) - now) * rate);
    if params.hold > 0.5 {
        next = stops;
    }
    if params.fixed > 0.5 {
        next = params.fixed_stops;
    }
    state[0] = next;
    state[1] = metered;
    state[3] = bright;
    state[2] = goal;
}
