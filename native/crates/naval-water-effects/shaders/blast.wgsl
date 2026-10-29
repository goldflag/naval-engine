// The sheet of spray a heavy turret's blast tears off the sea under its guns (`src/blast_sheet.rs`): native, the web
// game drew none. One horizontal quad a sheet, lying at the top of its mist; each fragment is the column of mist between
// the quad and what is behind it on the backdrop's depth (the sea, or a hull), followed down the view ray, its density
// the blast's at the column's foot. Lit as spray (`spray_radiance`, the sun's shadow maps at the foot) and by the scene's
// point lights as the sea takes them: the flash and the fireball's glow over it light it orange.
// Straight alpha, blended src·a + dst·(1 − a); linear HDR radiance.

#import naval::camera::{camera, world_to_clip, curved}
#import naval::lights::{LIGHT_WORDS, scene_lights, light_cell, light_word, point_light_at}
#import naval::shadow::{sun_shadow, shadow_light_direction}
#import naval_water_effects::spray::{EffectLighting, spray_radiance, spray_fog}
#import naval_water_effects::value_noise::value_noise

@group(1) @binding(0) var<uniform> lighting: EffectLighting;
// The backdrop's reversed depth (opaque and water; a far-plane placeholder before the first copy).
@group(3) @binding(0) var scene_depth: texture_depth_2d;

// The blast's reach (`blast_sheet.rs`: FALLOFF, DIRECTIVITY, FLOOR, COAST, COAST_SECONDS, LIFE).
const FALLOFF: f32 = 1.6;
const DIRECTIVITY: f32 = 1.3;
const FLOOR: f32 = 0.18;
const COAST: f32 = 0.22;
const COAST_SECONDS: f32 = 0.45;
const LIFE: f32 = 3.4;
// The shock that tears the spray sweeps out at about the speed of sound (m/s).
const SHOCK_SPEED: f32 = 340.0;
// The least slope of a view ray followed down a column (`blast_sheet.rs` RAY_SLOPE): a grazing ray's column is no
// longer than this allows.
const RAY_SLOPE: f32 = 0.12;
// The mist's density falls with height over the sea as exp(−h / H): the sheet behind the rim lies low, the rim's
// piled spray stands taller (m).
const FILL_HEIGHT: f32 = 0.8;
const RIM_HEIGHT: f32 = 1.6;
// The mist's density at the sea (optical depth per metre): the sheet behind the rim is a veil the sea shows through
// from above, the rim a bank of white.
const FILL_DENSITY: f32 = 0.5;
const RIM_DENSITY: f32 = 2.6;
// How fast each clears (s): the sheet behind the rim first, the rim's piled spray slower.
const FILL_SECONDS: f32 = 0.5;
const RIM_SECONDS: f32 = 1.0;
// The rim's width as a share of the blast's reach, and how it widens as it coasts and spreads (per second).
const RIM_WIDTH: f32 = 0.05;
const RIM_SPREAD: f32 = 0.06;
// Outside the rim the spray stops this much sharper than it thins inward, save in its strands, which stream past it.
const RIM_OUTER: f32 = 0.3;
// The spray is torn into strands along the blast: noise read across it in cells of an angle and along it in cells of
// a length (cells a radian round, metres out), in three octaves each this much finer across and shorter along, and
// cut into strands with thin mist between them where the sum crosses `STRANDS`. They are beaded into clusters
// of drops (three octaves from cells a metre, each this much finer), which also fray the strands' edges, and the rim
// is heaped in tufts (cells a metre). Its edge bulges in broad lobes and smaller bights round the muzzle (cells a
// radian), wanders with the tufts and frays with the strands' broadest octave, each by this share of the reach either
// way. Until 2026-10-24 one octave of streaks (14 cells a radian, 22 m) and no drops: seen close and low the sheet was
// a smooth white disc banded by its rim.
const STREAK_CELLS: f32 = 18.0;
const STREAK_LENGTH: f32 = 14.0;
const STREAK_OCTAVE: vec2<f32> = vec2<f32>(2.9, 0.42);
const STRANDS: vec2<f32> = vec2<f32>(0.3, 0.8);
const STRAND_GRAIN: f32 = 0.35;
const GRAIN_CELLS: f32 = 1.1;
const GRAIN_OCTAVE: f32 = 2.6;
const CLUMP_CELLS: f32 = 0.2;
const LOBE_CELLS: f32 = 2.2;
const BIGHT_CELLS: f32 = 7.0;
const EDGE_LOBES: f32 = 0.11;
const EDGE_BIGHTS: f32 = 0.04;
const EDGE_WANDER: f32 = 0.03;
const EDGE_FRAY: f32 = 0.03;
// A thick bank of spray shades its own underside from the sun (its vertical optical depth × this), and its drops'
// clusters catch the light unevenly (their noise, this much either way of its mean, held at the full light).
const SELF_SHADE: f32 = 0.6;
const GRAIN_LIGHT: f32 = 0.25;
// Spray takes a point light as it takes the sun: `direct` is the sun's light × 1.25 / 5.8 (`EffectLighting::
// set_illumination`).
const POINT_SCALE: f32 = 0.2155;
// Metres a pixel covers where the tears and clumps are lost and the sheet is drawn as their average.
const DETAIL_PIXEL: vec2<f32> = vec2<f32>(0.6, 4.0);

struct Instance {
    // xyz the quad's centre (world m), w the sheet's age (s).
    @location(0) center: vec4<f32>,
    // xyz the quad's full edge along the line of fire, w the blast's reach straight ahead (m).
    @location(1) axis_u: vec4<f32>,
    // xyz the quad's full edge across it, w the sheet's own number (0–1).
    @location(2) axis_v: vec4<f32>,
    // xyz the muzzles' centre (world m), w how dense the sheet is (0–1).
    @location(3) muzzle: vec4<f32>,
    // xyz the line of fire (unit), w the water level (world y).
    @location(4) bore: vec4<f32>,
}

struct Varyings {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    // The fragment's depth in view space (m, negative ahead).
    @location(1) view_z: f32,
    @location(2) @interpolate(flat) center: vec4<f32>,
    @location(3) @interpolate(flat) axis_u: vec4<f32>,
    @location(4) @interpolate(flat) axis_v: vec4<f32>,
    @location(5) @interpolate(flat) muzzle: vec4<f32>,
    @location(6) @interpolate(flat) bore: vec4<f32>,
}

// Noise in cells of `cells` a radian round the muzzle and `along` out from it, read on the angle round it, which jumps
// from π to −π behind the guns: there the readings either side of the jump are blended, so no seam shows.
fn round_noise(angle: f32, cells: f32, along: f32, offset: vec2<f32>) -> f32 {
    let here = value_noise(vec2<f32>(angle * cells, along) + offset);
    let across_jump = value_noise(vec2<f32>((angle - sign(angle) * 6.2831853) * cells, along) + offset);
    return mix(here, across_jump, smoothstep(2.6, 3.1415927, abs(angle)) * 0.5);
}

// The optical depth of a column of mist from `low` to `high` metres over the sea, at unit density, falling as
// exp(−h / height).
fn column(low: f32, high: f32, height: f32) -> f32 {
    return height * max(exp(-low / height) - exp(-high / height), 0.0);
}

// Four vertices a sheet, as a triangle strip over its quad.
@vertex
fn vertex(@builtin(vertex_index) index: u32, instance: Instance) -> Varyings {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u)) - 0.5;
    let world = instance.center.xyz + instance.axis_u.xyz * corner.x + instance.axis_v.xyz * corner.y;
    var out: Varyings;
    out.clip = world_to_clip(world);
    out.world = world;
    out.view_z = (camera.view * vec4<f32>(curved(world), 1.0)).z;
    out.center = instance.center;
    out.axis_u = instance.axis_u;
    out.axis_v = instance.axis_v;
    out.muzzle = instance.muzzle;
    out.bore = instance.bore;
    return out;
}

@fragment
fn fragment(input: Varyings) -> @location(0) vec4<f32> {
    // Metres a pixel covers, where derivatives are defined.
    let pixel = max(fwidth(input.world.x), fwidth(input.world.z));
    let age = input.center.w;
    let ahead = input.axis_u.w;
    let seed = input.axis_v.w;
    let muzzle = input.muzzle.xyz;
    let strength = input.muzzle.w;
    let bore = input.bore.xyz;
    let level = input.bore.w;
    let top = input.center.y - level;

    // The column under this fragment, from the quad down the view ray to the backdrop: how far below the quad the
    // backdrop lies, from the two depths along the ray (the drawn points', so the Earth's curvature cancels).
    let depth = textureLoad(scene_depth, vec2<i32>(input.clip.xy), 0);
    let behind = -camera.near * camera.far / (depth * (camera.far - camera.near) + camera.near);
    let along = clamp(behind / min(input.view_z, -1e-3), 1.0, 1e4);
    let drop = min((camera.position.y - curved(input.world).y) * (along - 1.0), top + 20.0);
    let eye_height = camera.position.y - level;
    // From above, the column runs from the quad down to the backdrop; from inside the mist, from the eye up to the quad.
    var low = top - max(drop, 0.0);
    if eye_height < top {
        low = eye_height;
    }
    low = max(low, 0.0);
    let ray = normalize(input.world - camera.position);
    let slope = max(abs(ray.y), RAY_SLOPE);
    // The column's foot: where the ray is low in the mist, a height of the sheet's own mist over its bottom (down the
    // ray from the quad, or back toward an eye inside the mist).
    let foot_height = min(low + FILL_HEIGHT, top);
    let run = (top - foot_height) / slope;
    let foot_xz = input.world.xz + ray.xz * select(run, -run, ray.y > 0.0);
    let foot = vec3<f32>(foot_xz.x, level + foot_height, foot_xz.y);

    // How far into the blast's reach the sea at the foot lies (below 1 inside it).
    let sea = vec3<f32>(foot.x, level, foot.z) - muzzle;
    let distance = max(length(sea), 1e-3);
    let lean = max(0.5 + 0.5 * dot(sea / distance, bore), FLOOR);
    let reach = ahead * pow(lean, DIRECTIVITY / FALLOFF);
    let q = distance / reach;

    // Polar about the muzzle's foot, for the streaks and clumps (relative to the muzzle: small numbers for the hash).
    let forward = normalize(input.axis_u.xz);
    let across = normalize(input.axis_v.xz);
    let offset = foot.xz - muzzle.xz;
    let angle = atan2(dot(offset, across), dot(offset, forward));
    let radius = length(offset);
    let coasted = ahead * COAST * (1.0 - exp(-age / COAST_SECONDS));
    let jitter = vec2<f32>(seed * 61.0, seed * 23.0);
    let detail = 1.0 - smoothstep(DETAIL_PIXEL.x, DETAIL_PIXEL.y, pixel);
    // The strands, each octave drawn only where its cells are a few pixels across (near the muzzle a radian's cells
    // close up), its mean in their place.
    let outward = radius - coasted;
    var streaks = 0.0;
    var broad = 0.5;
    var round_cells = STREAK_CELLS;
    var long_cells = STREAK_LENGTH;
    for (var octave = 0; octave < 3; octave += 1) {
        let n = round_noise(angle, round_cells, outward / long_cells, jitter + f32(octave) * 13.7);
        let resolved = mix(0.5, n, smoothstep(pixel, pixel * 3.0, min(radius / round_cells, long_cells)));
        if octave == 0 {
            broad = resolved;
        }
        streaks += resolved * select(0.5, select(0.2, 0.3, octave == 1), octave > 0);
        round_cells *= STREAK_OCTAVE.x;
        long_cells *= STREAK_OCTAVE.y;
    }
    // The drops' clusters, three octaves, each drawn only where its cells are a few pixels across.
    var grain = 0.0;
    var grain_cells = GRAIN_CELLS;
    for (var octave = 0; octave < 3; octave += 1) {
        let n = value_noise(offset * grain_cells + jitter + f32(octave) * 7.3);
        grain += mix(0.5, n, smoothstep(pixel, pixel * 3.0, 1.0 / grain_cells))
            * select(0.5, select(0.2, 0.3, octave == 1), octave > 0);
        grain_cells *= GRAIN_OCTAVE;
    }
    grain = mix(0.5, grain, detail);
    // Strands with ragged edges where the drops' clusters bead them.
    let strands = mix(0.5, smoothstep(STRANDS.x, STRANDS.y, streaks + STRAND_GRAIN * (grain - 0.5)), detail);
    let clump = mix(0.5, value_noise(offset * CLUMP_CELLS + jitter) * 0.6
        + value_noise(offset * CLUMP_CELLS * 2.7 + jitter + 17.0) * 0.4, detail);

    // Lobes and bights round the muzzle, the noise read on a circle so it closes on itself behind the guns.
    let round = vec2<f32>(cos(angle), sin(angle));
    let lobe = value_noise(round * LOBE_CELLS + jitter) - 0.5;
    let bight = value_noise(round * BIGHT_CELLS + jitter + 41.0) - 0.5;

    // The sheet's edge: the shock's front while it sweeps out, then the spray's as it coasts past the reach.
    let front = SHOCK_SPEED * age / reach;
    let edge = min(front, 1.0 + COAST * (1.0 - exp(-age / COAST_SECONDS)))
        * (1.0 + 2.0 * (EDGE_LOBES * lobe + EDGE_BIGHTS * bight + EDGE_WANDER * (clump - 0.5)
        + EDGE_FRAY * (broad - 0.5)));
    let inside = edge - q;
    let width = RIM_WIDTH + RIM_SPREAD * age;
    let from_rim = inside / select(width, width * RIM_OUTER * (0.7 + 0.6 * strands), inside < 0.0);
    let rim_shape = exp(-from_rim * from_rim);
    let fill_shape = smoothstep(-0.01, 0.08, inside) * (0.55 + 0.45 * clamp(q / max(edge, 1e-3), 0.0, 1.0));
    // Torn away as it ages, the gaps between the strands first; gone by the end of its life.
    let life = age / LIFE;
    let held = smoothstep(life - 0.25, life + 0.1, clump * 0.4 + strands * 0.6) * (1.0 - smoothstep(0.75, 1.0, life));
    // Strands of spray beaded with drops over a thin mist the sea shows through, not a veil.
    let beaded = 0.45 + 1.1 * grain;
    let fill = FILL_DENSITY * exp(-age / FILL_SECONDS) * fill_shape * (0.3 + 1.1 * strands * strands) * beaded;
    // The rim heaped in tufts, densest where a lobe carried it out, and streaked as the sheet is.
    let tufts = 0.2 + 0.8 * smoothstep(0.25, 0.75, clump);
    let rim = RIM_DENSITY * exp(-age / RIM_SECONDS) * rim_shape * tufts * clamp(1.0 + 2.0 * lobe, 0.4, 1.6)
        * (0.3 + 0.95 * strands) * beaded;
    let upright = (fill * column(low, top, FILL_HEIGHT) + rim * column(low, top, RIM_HEIGHT)) * strength * held;
    let depth_of = upright / slope;
    let alpha = 1.0 - exp(-depth_of);
    if alpha < 0.002 {
        discard;
    }

    // A flat bank of drops: lit from above as spray, its thick parts' undersides in their own shade, its clusters of
    // drops catching the light unevenly, and shaded where a hull or smoke stands up-sun of its foot.
    let up = vec3<f32>(0.0, 1.0, 0.0);
    let to_camera = -ray;
    let sun = shadow_light_direction();
    let shadow = sun_shadow(foot + sun * 0.5, up);
    let sunlit = mix(1.0, exp(-upright * SELF_SHADE), 0.5) * (1.0 - GRAIN_LIGHT + 2.0 * GRAIN_LIGHT * grain);
    var color = spray_radiance(lighting, up, to_camera, min(sunlit, 1.0), shadow, 1.0, exp(-depth_of), 0.0);
    // The flashes (the fireball's glow among them), fires and searchlights over it: the renderer's lights that reach
    // its foot's cell (`naval::lights`).
    let cell = light_cell(foot);
    for (var word = 0u; word < LIGHT_WORDS; word += 1u) {
        var bits = light_word(cell, word);
        while bits != 0u {
            let index = word * 32u + firstTrailingBit(bits);
            bits &= bits - 1u;
            let light = point_light_at(scene_lights.lights[index], foot);
            color += light.radiance * POINT_SCALE * (0.4 + 0.6 * max(light.direction.y, 0.0));
        }
    }
    return vec4<f32>(spray_fog(color, input.world), alpha);
}
