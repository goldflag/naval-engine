// Ports and infrastructure over a theatre's land (native/docs/terrain/infra.md): container stacks,
// moored boats and bridges in chunks, each chunk's vertices chunk-local and placed by its instance's translation; each
// face in its colour and material (`naval_terrain::infra::mesh::Material`): painted steel and fibreglass, concrete, the
// stacks (each box's colour hashed from its row, slot and tier), a road deck's asphalt and lines, glass, a hull's gloss,
// structural steel, a runway's, taxiway's and apron's marks (`vs_infra`, `fs_infra`, opaque); and lines
// (`vs_infra_line`, `fs_infra_line`, blended over the finished scene): power lines' wires and lattice towers' members
// afar, opened across the view to at least about a pixel and as opaque as the share of that pixel they cover (a wire a
// little more), lit at their vertices; in temporal anti-aliasing's motion pass (`vs_infra_line_motion`), the history
// they ask to be left out of while the camera moves. Lit and fogged as the buildings are (the sun through the shadow
// maps and the clouds, the hemisphere fill, the sky's environment).
#import naval::camera::{camera, curved, world_to_clip}
#import naval::motion::{camera_motion, motion_response}
#import naval::aerial::aerial_apply
#import naval::shadow::{sun_shadow}
#import naval_clouds::shadow::{CloudShadow, cloud_shadow}
#import naval_terrain::structures::{srgb_to_linear, hash12, hash13, band, tone, brdf_ggx}
#import naval_sky::environment::{SkyEnvironment, env_radiance, env_irradiance, env_reflection_direction}

struct Infra {
    // Toward the sun (or moon); w: night, 0-1.
    sun_direction: vec4<f32>,
    // The directional light on meshes; w: the environment's intensity.
    sun_radiance: vec4<f32>,
    // The hemisphere fill's sky and ground; fill_sky.w: 1 while the clouds' shadow is bound.
    fill_sky: vec4<f32>,
    fill_ground: vec4<f32>,
    // Scene fog: colour and power; start, end, sky blend (m), 1 while on.
    fog_color: vec4<f32>,
    fog: vec4<f32>,
    // x: the near range (m), y: the range (m), z: the share of the cabins dark at night beyond the usual (a scenario's
    // dimout or blackout, `InfraLayer::set_windows`), w: unused.
    params: vec4<f32>,
}

@group(1) @binding(0) var<uniform> infra: Infra;

@group(2) @binding(0) var sky_atlas: texture_2d<f32>;
@group(2) @binding(1) var sky_sampler: sampler;
@group(2) @binding(2) var<uniform> sky_env: SkyEnvironment;
@group(2) @binding(3) var sky_fog: texture_2d<f32>;

@group(3) @binding(0) var cloud_map: texture_2d<f32>;
@group(3) @binding(1) var cloud_sampler: sampler;
@group(3) @binding(2) var<uniform> cloud_params: CloudShadow;

const PI: f32 = 3.141592653589793;
const RECIPROCAL_PI: f32 = 0.3183098861837907;

// `naval_terrain::infra::mesh::Material`.
const MAT_PAINT: u32 = 0u;
const MAT_CONCRETE: u32 = 1u;
const MAT_CONTAINER: u32 = 2u;
const MAT_STACKS: u32 = 3u;
const MAT_DECK: u32 = 4u;
const MAT_GLASS: u32 = 5u;
const MAT_HULL: u32 = 6u;
const MAT_STEEL: u32 = 7u;
// `MAT_RUNWAY` to `MAT_APRON` lie on the ground: pulled toward the eye.
const MAT_RUNWAY: u32 = 8u;
const MAT_TAXIWAY: u32 = 9u;
const MAT_APRON: u32 = 10u;
const MAT_WIRE: u32 = 11u;
// `MAT_WIRE` and `MAT_LATTICE` are lines: opened across the view, drawn by `vs_infra_line` and `fs_infra_line`.
const MAT_LATTICE: u32 = 12u;
// A generated base's (`naval_terrain::infra::bases`): coral and Marston mat lie on the ground (pulled toward the eye, as
// `MAT_RUNWAY` to `MAT_APRON`); earth berms and sandbags, corrugated iron, tent canvas, weathered planks and fuel drums
// stand.
const MAT_CORAL: u32 = 13u;
const MAT_MARSTON: u32 = 14u;
const MAT_EARTH: u32 = 15u;
const MAT_IRON: u32 = 16u;
const MAT_CANVAS: u32 = 17u;
const MAT_WOOD: u32 = 18u;
const MAT_DRUMS: u32 = 19u;
// A line is drawn at least this many pixels wide.
const WIRE_MIN_PX: f32 = 1.0;

// `naval_terrain::infra::containers`: a slot's pitch along a block, a box's length and a tier's height (m).
const SLOT_ALONG_M: f32 = 12.75;
const BOX_LONG_M: f32 = 12.19;
const TIER_M: f32 = 2.6;

// The world's container colours (sRGB bytes) as a yard shows them from above, the commoner twice or more: the lessors'
// rust-reds, browns, greys, whites and beige, the lines' blues, a few greens, oranges, reds and magentas.
const BOXES = array<vec3<f32>, 32>(
    vec3<f32>(132.0, 60.0, 46.0),   // rust red
    vec3<f32>(132.0, 60.0, 46.0),
    vec3<f32>(112.0, 62.0, 48.0),   // brown
    vec3<f32>(112.0, 62.0, 48.0),
    vec3<f32>(146.0, 74.0, 54.0),   // faded red
    vec3<f32>(98.0, 54.0, 42.0),    // dark brown
    vec3<f32>(86.0, 146.0, 180.0),  // light blue
    vec3<f32>(86.0, 146.0, 180.0),
    vec3<f32>(76.0, 128.0, 162.0),  // faded light blue
    vec3<f32>(38.0, 62.0, 104.0),   // dark blue
    vec3<f32>(38.0, 62.0, 104.0),
    vec3<f32>(46.0, 76.0, 120.0),   // blue
    vec3<f32>(46.0, 76.0, 120.0),
    vec3<f32>(198.0, 198.0, 192.0), // white
    vec3<f32>(198.0, 198.0, 192.0),
    vec3<f32>(170.0, 172.0, 170.0), // light grey
    vec3<f32>(170.0, 172.0, 170.0),
    vec3<f32>(136.0, 140.0, 142.0), // grey
    vec3<f32>(188.0, 174.0, 140.0), // beige
    vec3<f32>(188.0, 174.0, 140.0),
    vec3<f32>(170.0, 160.0, 130.0), // cream
    vec3<f32>(44.0, 96.0, 70.0),    // green
    vec3<f32>(70.0, 108.0, 80.0),   // grey-green
    vec3<f32>(190.0, 102.0, 46.0),  // orange
    vec3<f32>(190.0, 102.0, 46.0),
    vec3<f32>(152.0, 42.0, 38.0),   // red
    vec3<f32>(152.0, 42.0, 38.0),
    vec3<f32>(150.0, 44.0, 88.0),   // magenta
    vec3<f32>(96.0, 110.0, 124.0),  // grey-blue
    vec3<f32>(96.0, 110.0, 124.0),
    vec3<f32>(54.0, 108.0, 114.0),  // teal
    vec3<f32>(184.0, 156.0, 64.0),  // yellow
);

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) colour: vec4<u32>,
    // The chunk's translation into the world (x, y, z).
    @location(4) chunk: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) colour: vec4<u32>,
    @location(4) @interpolate(flat) param: f32,
}

// A line's vertex (`vs_infra_line`): its colour, lit and seen through the air, and how opaque it is.
struct LineOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) colour: vec4<f32>,
}

@vertex
fn vs_infra(in: VertexIn) -> VertexOut {
    return place(in);
}

// A vertex in the world and on the screen, a line's opened across the view.
fn place(in: VertexIn) -> VertexOut {
    var world = in.position + in.chunk.xyz;
    var out: VertexOut;
    out.normal = in.normal.xyz;
    // snorm8's w: the material's parameter, 1/127 each.
    out.param = round(in.normal.w * 127.0);
    if in.colour.w == MAT_WIRE || in.colour.w == MAT_LATTICE {
        // A line (`MeshBuilder::wire`, `member`): its normal its direction, `uv.y` its side, the parameter its radius
        // (cm). Opened across the view to its width or about a pixel, whichever is more, and in the parameter the share
        // of the drawn width it fills. Its normal a cylinder's across it as far as it is drawn at its width; opened,
        // the middle's (toward the eye) at both sides, so a line a pixel wide is one colour across and does not
        // flicker light and dark as it steps from one row of pixels to the next.
        let dir = normalize(in.normal.xyz);
        let to_eye = camera.position - world;
        let distance = max(length(to_eye), 1e-3);
        let across = cross(dir, to_eye);
        let side = across / max(length(across), 1e-4);
        let facing = normalize(to_eye - dir * dot(to_eye, dir) + vec3<f32>(0.0, 1e-4, 0.0));
        let pixel = 2.0 * distance * camera.tan_half_fov_y * camera.viewport.w;
        let radius = out.param * 0.01;
        let half = max(radius, 0.5 * WIRE_MIN_PX * pixel);
        world += side * (half * in.uv.y);
        out.param = radius / half;
        out.normal = facing * 0.6 + side * (0.8 * in.uv.y * out.param);
    }
    var clip = world_to_clip(world);
    if in.colour.w >= MAT_RUNWAY && in.colour.w <= MAT_APRON {
        // Reversed depth: a little nearer, by a share of the distance (the land's mesh under it is coarser far off).
        clip.z = clip.z * 1.004;
    } else if in.colour.w == MAT_CORAL || in.colour.w == MAT_MARSTON {
        // A base's ground, where aircraft, tents and trucks stand on it: nearer by the same share, but by no more than
        // a metre within 2 km (pulled 4 m at a kilometre it would hide a parked plane), the share again past that.
        let d = max(clip.w, 1e-3);
        let pull = min(0.004 * d, 1.0 + max(d - 2000.0, 0.0) * 0.004);
        clip.z = clip.z * d / max(d - pull, 1e-3);
    }
    out.clip = clip;
    out.world = world;
    out.uv = in.uv;
    out.colour = in.colour;
    return out;
}

struct Surface {
    albedo: vec3<f32>,
    roughness: f32,
    // The reflectance at normal incidence over 0.04.
    specular: f32,
    emissive: vec3<f32>,
}

// A box's colour: the palette's entry by a hash of the row's seed, the slot, the tier and the half (a 20 ft box's).
fn box_colour(seed: vec3<f32>, slot: f32, tier: f32, half: f32) -> vec3<f32> {
    let h = hash13(vec3<f32>(seed.x * 3.1 + slot * 1.7 + half * 0.37, seed.y * 2.3 + tier * 5.3, seed.z * 1.9 + 11.0));
    let i = min(u32(h * 32.0), 31u);
    // Each box a little faded or fresh.
    let fade = 0.86 + 0.18 * hash13(vec3<f32>(slot + 3.0, tier + seed.x, half + seed.z));
    return srgb_to_linear(BOXES[i]) * fade;
}

// The stacks: a slot's column (`param` its bay, bit 6 a pair of 20 ft boxes; `uv` metres along the slot and up from the
// lowest box's foot) or a row's run of slots afar (`param` −1; `uv` metres along the block and up), each box its colour,
// the corrugation and the seams between tiers and slots resolved where a pixel is small enough.
fn stacks_surface(in: VertexOut, n: vec3<f32>, wuv: vec2<f32>) -> Surface {
    var s: Surface;
    let seed = vec3<f32>(in.colour.xyz);
    var slot: f32;
    var along: f32;
    var half = 0.0;
    if in.param < 0.0 {
        slot = floor(in.uv.x / SLOT_ALONG_M);
        along = in.uv.x - slot * SLOT_ALONG_M;
    } else {
        let p = u32(in.param);
        slot = f32(p & 63u);
        along = in.uv.x;
        if (p & 64u) != 0u && along > BOX_LONG_M * 0.5 {
            half = 1.0;
        }
    }
    let tier = max(floor(in.uv.y / TIER_M), 0.0);
    var colour = box_colour(seed, slot, tier, half);
    let w_along = wuv.x;
    let w_up = wuv.y;
    let resolved = 1.0 - smoothstep(0.15, 0.5, max(w_along, w_up));
    // The seams: under each box, and the gap between slots in a row afar.
    let seam = band(fract(in.uv.y / TIER_M), 0.0, 0.035, w_up / TIER_M);
    var gap = 0.0;
    if in.param < 0.0 {
        gap = band(along, BOX_LONG_M, SLOT_ALONG_M, w_along);
    }
    let side = abs(n.y) < 0.5;
    if side {
        // Corrugation along the side, 0.3 m apart; the doors' end plain.
        let ribs = 0.5 + 0.5 * cos(along / 0.3 * 2.0 * PI);
        colour *= mix(1.0, 0.86 + 0.14 * ribs, resolved);
        colour *= 1.0 - 0.55 * seam * resolved;
        colour = mix(colour, vec3<f32>(0.02), gap * 0.85);
    } else {
        // A roof's dents and dust.
        colour *= 0.92;
    }
    s.albedo = colour;
    s.roughness = 0.62;
    s.specular = 0.7;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// A road deck: asphalt (`uv` metres along and across from the middle) with its edge lines, the lanes' dashes 3.6 m
// apart and, on a wide deck, a median barrier, a narrow one a double yellow line; `param` the deck's half width (m).
fn deck_surface(in: VertexOut, wuv: vec2<f32>) -> Surface {
    var s: Surface;
    let half = max(in.param, 1.0);
    let x = in.uv.y;
    let ax = abs(x);
    let w = wuv.y;
    let wa = wuv.x;
    let resolved = 1.0 - smoothstep(0.08, 0.3, w);
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    // Tyre-worn bands, a little darker.
    let lanes = max(floor((half - 1.0) / 3.6), 1.0);
    let lane_pos = fract(ax / 3.6);
    colour *= 1.0 - 0.08 * band(lane_pos, 0.2, 0.35, w / 3.6) - 0.08 * band(lane_pos, 0.65, 0.8, w / 3.6);
    let white = vec3<f32>(0.62, 0.62, 0.6);
    let yellow = vec3<f32>(0.6, 0.45, 0.08);
    var line = band(ax, half - 0.75, half - 0.6, w);
    let k = round(ax / 3.6);
    let dash = band(fract(in.uv.x / 12.0), 0.0, 0.25, wa / 12.0);
    if k >= 1.0 && k < lanes && ax < half - 1.5 {
        line += band(ax, k * 3.6 - 0.07, k * 3.6 + 0.07, w) * dash;
    }
    colour = mix(colour, white, clamp(line, 0.0, 1.0) * resolved);
    if half >= 9.0 {
        // A median barrier's top, pale concrete.
        colour = mix(colour, vec3<f32>(0.42, 0.41, 0.38), band(ax, 0.0, 0.35, w));
    } else if half >= 4.0 {
        let double = band(ax, 0.08, 0.2, w);
        colour = mix(colour, yellow, double * resolved);
    }
    s.albedo = colour;
    s.roughness = 0.9;
    s.specular = 0.5;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// A runway's, taxiway's and apron's marks (`naval_terrain::infra::airfield`): white paint, taxiway yellow.
const PAINT_WHITE: vec3<f32> = vec3<f32>(0.72, 0.72, 0.7);
const PAINT_YELLOW: vec3<f32> = vec3<f32>(0.66, 0.42, 0.03);
// 3 by 5 glyphs, rows top to bottom, 3 bits each (the left the high bit): 0 to 9, then L, C and R.
const GLYPHS = array<u32, 13>(
    0x7b6fu, 0x2c97u, 0x73e7u, 0x73cfu, 0x5bc9u, 0x79cfu, 0x79efu, 0x7249u, 0x7befu, 0x7bcfu,
    0x4927u, 0x7927u, 0x6badu,
);
// The touchdown zone's bar groups: metres from the threshold and bars a side.
const TOUCHDOWN = array<vec2<f32>, 5>(
    vec2<f32>(152.4, 3.0), vec2<f32>(457.2, 2.0), vec2<f32>(609.6, 2.0), vec2<f32>(762.0, 1.0), vec2<f32>(914.4, 1.0),
);

fn glyph_bit(glyph: u32, col: i32, row: i32) -> f32 {
    if col < 0 || col > 2 || row < 0 || row > 4 {
        return 0.0;
    }
    let bit = (4u - u32(row)) * 3u + (2u - u32(col));
    return f32((GLYPHS[glyph] >> bit) & 1u);
}

// Glyph `glyph`'s coverage at (`gx`, `gy`) in its cells (x from its left, y down from its top), its edges filtered over
// `w` (the pixel's footprint in cells).
fn glyph(glyph: u32, gx: f32, gy: f32, w: vec2<f32>) -> f32 {
    let u = vec2<f32>(gx, gy) - 0.5;
    let c = floor(u);
    let t = clamp((u - c - 0.5) / max(w, vec2<f32>(1e-3)) + 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
    let ci = vec2<i32>(c);
    let a = mix(glyph_bit(glyph, ci.x, ci.y), glyph_bit(glyph, ci.x + 1, ci.y), t.x);
    let b = mix(glyph_bit(glyph, ci.x, ci.y + 1), glyph_bit(glyph, ci.x + 1, ci.y + 1), t.x);
    return mix(a, b, t.y);
}

// Stripes `duty` of each period (`t` in periods, `w` the footprint in periods), their mean where they no longer resolve.
fn stripes(t: f32, duty: f32, w: f32) -> f32 {
    return mix(band(fract(t), 0.0, duty, w), duty, smoothstep(0.3, 0.6, w));
}

// A runway's half from its end (`uv` metres along from the threshold and across, the landing pilot's right positive;
// `param` the half width; the colour's bytes its designator's number and letter, and flags: 1 the number's leading
// zero kept, 2 the touchdown zone marked, 4 plain, a wartime runway's): asphalt worn dark where wheels touch down, the
// side stripes, the threshold's bars, the designator, the aiming point and the touchdown zone's bars, the dashed centre
// line. The FAA's sizes.
fn runway_surface(in: VertexOut, wuv: vec2<f32>, footprint: f32) -> Surface {
    var s: Surface;
    let half = max(in.param, 8.0);
    let along = in.uv.x;
    let a = in.uv.y;
    let x = abs(a);
    let ws = max(wuv.x, 1e-3);
    let wa = max(wuv.y, 1e-3);
    let number = in.colour.x;
    let letter = in.colour.y;
    let flags = in.colour.z;
    var colour = srgb_to_linear(vec3<f32>(96.0, 96.0, 93.0));
    // Patches of resurfacing, a few metres across.
    let shown = 1.0 - smoothstep(2.0, 6.0, footprint);
    colour *= mix(1.0, 0.92 + 0.14 * hash12(floor(in.world.xz / 6.0)), shown);
    // Rubber where the wheels touch down, streaked along.
    let streak = 0.7 + 0.3 * hash12(vec2<f32>(floor(a / 0.8), floor(along / 30.0)));
    let rubber = smoothstep(150.0, 330.0, along) * (1.0 - smoothstep(700.0, 1100.0, along))
        * (1.0 - smoothstep(6.0, 11.0, x));
    colour *= 1.0 - 0.55 * rubber * mix(1.0, streak, shown);
    var mark = band(x, half - 1.5, half - 0.6, wa);
    // The threshold's bars, 1.75 m wide and as far apart, from 1.75 m off the centre line: 6 a side on a 45 m runway.
    let keys = clamp(floor((half - 1.0) / 3.5), 2.0, 8.0);
    let k = (x - 1.75) / 3.5;
    if k >= 0.0 && k < keys {
        mark += band(along, 6.0, 51.7, ws) * stripes(k, 0.5, wa / 3.5);
    }
    // The designator: the letter nearer the threshold, then the number, 18.3 m tall and 6.1 m wide a figure.
    let height = 18.3;
    let width = 6.1;
    if along > 60.0 && along < 110.0 {
        let has_letter = letter > 0u;
        let number_from = select(64.0, 88.4, has_letter);
        let g = vec2<f32>(wa / width * 3.0, ws / height * 5.0);
        if has_letter {
            let gy = (64.0 + height - along) / height * 5.0;
            mark += glyph(9u + letter, (a + width * 0.5) / width * 3.0, gy, g);
        }
        let gy = (number_from + height - along) / height * 5.0;
        let tens = number / 10u;
        let ones = number % 10u;
        if tens == 0u && (flags & 1u) == 0u {
            mark += glyph(ones, (a + width * 0.5) / width * 3.0, gy, g);
        } else {
            mark += glyph(tens, (a + 2.3 + width) / width * 3.0, gy, g);
            mark += glyph(ones, (a - 2.3) / width * 3.0, gy, g);
        }
    }
    if (flags & 2u) != 0u {
        // The aiming point, 1,020 ft on; the touchdown zone's bars in threes, twos and ones each 500 ft.
        mark += band(along, 311.0, 356.7, ws) * band(x, 11.0, 20.1, wa);
        let bars = (x - 11.0) / 3.3;
        for (var i = 0u; i < 5u; i++) {
            let group = TOUCHDOWN[i];
            if bars >= 0.0 && bars < group.y {
                mark += band(along, group.x, group.x + 22.9, ws) * stripes(bars, 0.55, wa / 3.3);
            }
        }
    }
    // The centre line: 120 ft dashes 80 ft apart past the designator.
    if along > 125.0 {
        mark += band(a, -0.45, 0.45, wa) * stripes((along - 125.0) / 61.0, 0.6, ws / 61.0);
    }
    // A wartime runway (flag 4): plain pavement, no paint.
    if (flags & 4u) != 0u {
        mark = 0.0;
    }
    s.albedo = mix(colour, PAINT_WHITE, clamp(mark, 0.0, 1.0));
    s.roughness = 0.85;
    s.specular = 0.5;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// A taxiway (`uv` metres along, wrapping, and across; `param` its half width): asphalt with its yellow centre line and
// edge lines.
fn taxiway_surface(in: VertexOut, wuv: vec2<f32>, footprint: f32) -> Surface {
    var s: Surface;
    let half = max(in.param, 4.0);
    let x = abs(in.uv.y);
    let wa = max(wuv.y, 1e-3);
    var colour = srgb_to_linear(vec3<f32>(104.0, 103.0, 99.0));
    let shown = 1.0 - smoothstep(2.0, 6.0, footprint);
    colour *= mix(1.0, 0.93 + 0.12 * hash12(floor(in.world.xz / 5.0)), shown);
    var mark = band(in.uv.y, -0.2, 0.2, wa);
    mark += band(x, half - 1.1, half - 0.9, wa) + band(x, half - 0.75, half - 0.55, wa);
    s.albedo = mix(colour, PAINT_YELLOW, clamp(mark, 0.0, 1.0));
    s.roughness = 0.85;
    s.specular = 0.5;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// An apron: concrete slabs 7.5 m square, each its shade, the joints between them and a stain here and there.
fn apron_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    let p = in.world.xz / 7.5;
    let shown = 1.0 - smoothstep(1.0, 4.0, footprint);
    colour *= mix(1.0, 0.9 + 0.14 * hash12(floor(p)), shown);
    let w = footprint / 7.5;
    let joint = max(band(fract(p.x), 0.0, 0.015, w), band(fract(p.y), 0.0, 0.015, w));
    colour *= 1.0 - 0.35 * joint * (1.0 - smoothstep(0.3, 0.8, footprint));
    let stain = select(1.0, 0.78, hash12(floor(in.world.xz / 4.0) + 17.0) < 0.05);
    colour *= mix(1.0, stain, shown);
    s.albedo = colour;
    s.roughness = 0.9;
    s.specular = 0.5;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Value noise, smooth between the lattice's points (0-1).
fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash12(i);
    let b = hash12(i + vec2<f32>(1.0, 0.0));
    let c = hash12(i + vec2<f32>(0.0, 1.0));
    let d = hash12(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// The ground beside a base's coral (sRGB): trampled earth, the coral's edge frays into it.
const BASE_EARTH: vec3<f32> = vec3<f32>(122.0, 104.0, 80.0);
// A runway's overrun past each end (m): `infra::bases::strip`'s.
const RUNWAY_OVERRUN_M: f32 = 45.0;

// Crushed coral (`MAT_CORAL`: `uv` metres along a strip and across it, or for a pad (0, how far in from its rim);
// `param` the paved half width; the colour's bytes its look (0 a runway, 1 a taxiway, 2 a hardstand or lot, 3 a road,
// 4 tyre tracks, 5 trampled dust, 6 a camp's street, 7 a hand-built runway, 8 packed earth, 9 tarmac, 10 concrete
// slabs, 11 a hedgerow painted on grass, 12 grass mown for a landing run, 13 a tarmac pad, 14 a concrete pad), its whole
// half width or a pad's reach, and a runway's length in 16 m (else a seed)).
// A runway near white, patched where it was dumped, rolled and mended, worn darker down its middle and where wheels
// touch down, its shoulders rougher and yellower; the taxiways, hardstands and roads dirtier, tan-grey, earth tracked
// over them, a road's two ruts; tyre tracks and dust earth with coral ground into it; a Japanese field's runway greyer
// and patchier, its craters filled, its taxiways and roads packed earth. Every edge frays into the earth beside it,
// broken unevenly; a road's, a track's and the like's breaks off into the land itself (what lies among the trees).
fn coral_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    let look = in.colour.x;
    let outer = f32(in.colour.y);
    let third = f32(in.colour.z);
    let paved = max(in.param, 1.0);
    let p = in.world.xz;
    let shown = 1.0 - smoothstep(0.6, 2.5, footprint);
    let grain = vnoise(p / 0.45) * 0.6 + vnoise(p / 0.17 + 5.3) * 0.4 * (1.0 - smoothstep(0.05, 0.3, footprint));
    let earth = srgb_to_linear(BASE_EARTH) * (0.84 + 0.3 * vnoise(p / 1.9)) * mix(1.0, 0.9 + 0.2 * grain, shown);
    let patches = vnoise(p / 23.0 + third) * 0.6 + vnoise(p / 7.0 + 3.7) * 0.4;
    let worn = srgb_to_linear(vec3<f32>(146.0, 138.0, 124.0));
    let pad = look == 2u || look == 5u || look == 13u || look == 14u;
    // A temperate base's looks fray into grass, not earth.
    let temperate = look >= 9u;
    let grass = srgb_to_linear(vec3<f32>(80.0, 100.0, 50.0)) * (0.85 + 0.3 * vnoise(p / 2.6));
    let beside = select(earth, grass, temperate);
    var rag = 1.0;
    // Across: from a strip's middle, or in from a pad's rim.
    let x = select(abs(in.uv.y), outer - in.uv.y, pad);
    let edge_in = select(outer - abs(in.uv.y), in.uv.y, pad);
    var colour: vec3<f32>;
    var fray_m = 5.0;
    if look == 0u {
        colour = srgb_to_linear(vec3<f32>(192.0, 186.0, 168.0)) * (0.78 + 0.3 * patches);
        colour *= mix(1.0, 0.9 + 0.2 * grain, shown);
        let along = in.uv.x;
        let from_end = min(along, third * 16.0 - along) - RUNWAY_OVERRUN_M;
        let streak = 0.6 + 0.4 * vnoise(vec2<f32>(in.uv.y / 1.3, along / 40.0));
        // Worn down the middle; darker still where the wheels touch down, 100 to 550 m in from either end.
        let middle = 1.0 - smoothstep(4.0, 12.0, x);
        let touchdown = smoothstep(60.0, 140.0, from_end) * (1.0 - smoothstep(420.0, 600.0, from_end));
        colour = mix(colour, worn, (0.42 + 0.25 * touchdown) * middle * streak);
        colour = mix(colour, worn * 0.66, 0.45 * touchdown * (1.0 - smoothstep(6.0, 16.0, x)) * streak * streak);
        // Mended: rectangles of fresh coral, or darker fill, here and there on the paving.
        let cell = floor(vec2<f32>(along / 26.0, in.uv.y / 9.0 + 0.5));
        let f = fract(vec2<f32>(along / 26.0, in.uv.y / 9.0 + 0.5));
        let mend = hash12(cell + third * 0.37 + 41.0);
        let w = vec2<f32>(max(footprint, 1e-3) / 26.0, max(footprint, 1e-3) / 9.0);
        let inside = band(f.x, 0.12, 0.8, w.x) * band(f.y, 0.18, 0.78, w.y) * (1.0 - smoothstep(paved - 1.0, paved, x));
        colour *= mix(1.0, select(0.8, 1.1, hash12(cell + 7.0) > 0.5), inside * step(mend, 0.22));
        // The shoulders: rougher, yellower, patchier.
        let shoulder = smoothstep(paved - 0.5, paved + 0.5, x);
        let rough = srgb_to_linear(vec3<f32>(176.0, 164.0, 136.0)) * (0.82 + 0.28 * vnoise(p / 3.0));
        colour = mix(colour, rough, shoulder * 0.8);
        fray_m = 3.5;
    } else if look == 4u {
        // Tyre tracks: earth with coral ground in, two darker ruts.
        colour = mix(earth, srgb_to_linear(vec3<f32>(150.0, 140.0, 120.0)), 0.25 * patches);
        let rut = band(abs(x - 0.9), 0.0, 0.32, max(footprint, 0.05));
        colour = mix(colour, earth * 0.7, 0.55 * rut);
        fray_m = 1.4;
    } else if look == 6u {
        // A camp's street worn to earth, grass still at its edges.
        colour = mix(earth, srgb_to_linear(vec3<f32>(150.0, 140.0, 118.0)), 0.2 * patches);
        colour *= 0.9 + 0.15 * vnoise(p / 3.0);
        fray_m = 0.6 * outer;
    } else if look == 7u {
        // A hand-built runway: coral and gravel rolled over packed earth, greyer and patchier, the earth through it,
        // worn down the middle; craters filled with darker earth or fresh coral; its shoulders earthier.
        colour = srgb_to_linear(vec3<f32>(166.0, 160.0, 142.0)) * (0.7 + 0.38 * patches);
        colour *= mix(1.0, 0.86 + 0.28 * grain, shown);
        // Rolled in strips along it, a shade apart; the earth through it in patches.
        colour *= 0.9 + 0.2 * vnoise(vec2<f32>(in.uv.y / 3.5, in.uv.x / 70.0) + third);
        let through = 0.4 * smoothstep(0.4, 0.8, vnoise(p / 13.0 + third)) + 0.2 * smoothstep(0.55, 0.9, vnoise(p / 3.1 + 2.0));
        colour = mix(colour, earth, through);
        let middle = 1.0 - smoothstep(5.0, 14.0, x);
        colour = mix(colour, worn * 0.85, 0.3 * middle * (0.6 + 0.4 * vnoise(vec2<f32>(in.uv.y / 1.5, in.uv.x / 35.0))));
        // Craters filled in, here and there: ragged patches of darker earth or fresher coral.
        let cell = floor(p / 41.0);
        let pick = hash12(cell + 13.0);
        let crater = (cell + 0.2 + 0.6 * vec2<f32>(hash12(cell + 3.1), hash12(cell + 7.7))) * 41.0;
        let r = 2.5 + 3.5 * hash12(cell + 5.3);
        let ragged = distance(p, crater) + (vnoise(p / 1.6) - 0.5) * 2.4 + (vnoise(p / 0.5) - 0.5) * 0.8;
        let filled = (1.0 - smoothstep(r - 1.5, r + 1.0, ragged)) * step(pick, 0.5) * (1.0 - smoothstep(paved - 2.0, paved, x));
        colour = mix(colour, select(mix(colour, earth, 0.6) * 0.85, colour * 1.1, pick > 0.3), 0.6 * filled);
        let shoulder = smoothstep(paved - 1.5, paved + 1.0, x);
        colour = mix(colour, mix(earth, colour, 0.45), shoulder * 0.8);
        fray_m = 9.0;
    } else if look == 8u {
        // Packed earth and gravel: coral grit in the earth, the wheels' paths worn darker.
        colour = mix(earth, srgb_to_linear(vec3<f32>(150.0, 142.0, 124.0)), 0.18 + 0.22 * patches);
        colour *= 0.9 + 0.15 * vnoise(p / 2.6);
        let w = max(footprint, 0.05);
        let paths = band(abs(x - 1.6), 0.0, 0.6, w) + 0.6 * band(abs(x - 0.45 * outer), 0.0, 0.8, w);
        let broken_paths = 0.35 + 0.65 * smoothstep(0.3, 0.7, vnoise(vec2<f32>(in.uv.x / 6.0, x / 2.0) + third));
        colour = mix(colour, earth * 0.75, 0.35 * clamp(paths, 0.0, 1.0) * broken_paths);
        colour *= 0.88 + 0.22 * vnoise(p / 7.0 + 1.3);
        fray_m = max(0.45 * outer, 1.0);
    } else if look == 9u || look == 13u {
        // Tarmac over concrete: dark, its aggregate grey through it where wheels run, patched darker with fresh tar,
        // a shade apart where it was laid at other times; its edge crisp.
        colour = srgb_to_linear(vec3<f32>(74.0, 74.0, 72.0)) * (0.82 + 0.3 * patches);
        colour *= mix(1.0, 0.86 + 0.28 * grain, shown);
        colour *= 0.92 + 0.14 * hash12(floor(vec2<f32>(in.uv.x / 30.0, in.uv.y / 15.0 + 0.5)) + third);
        let run = select(1.0 - smoothstep(3.0, 9.0, x), smoothstep(0.3, 0.7, vnoise(p / 6.0)), pad);
        colour = mix(colour, srgb_to_linear(vec3<f32>(126.0, 124.0, 118.0)), 0.35 * run * (0.5 + 0.5 * vnoise(vec2<f32>(in.uv.y / 1.2, in.uv.x / 25.0))));
        // Mended with fresh tar here and there: patches of their own sizes, not in rows.
        let cell = floor(p / 19.0);
        let f = fract(p / 19.0);
        let w = max(footprint, 1e-3) / 19.0;
        let h0 = hash12(cell + third + 3.0);
        let h1 = hash12(cell + 5.1);
        let h2 = hash12(cell + 8.3);
        let h3 = hash12(cell + 2.7);
        let tarred = band(f.x, 0.1 + 0.4 * h1, 0.3 + 0.6 * h2, w) * band(f.y, 0.1 + 0.4 * h3, 0.35 + 0.55 * h1, w)
            * step(h0, 0.07);
        colour = mix(colour, srgb_to_linear(vec3<f32>(46.0, 46.0, 46.0)), 0.6 * tarred);
        fray_m = 0.8;
        rag = 0.15;
    } else if look == 10u || look == 14u {
        // Concrete in slabs 5 m square, each a shade off, its joints dark (resolved close up), tyre-marked down the
        // middle, mended here and there.
        colour = srgb_to_linear(vec3<f32>(136.0, 133.0, 125.0)) * (0.8 + 0.26 * patches);
        colour *= mix(1.0, 0.88 + 0.24 * grain, shown);
        let slab = floor(select(vec2<f32>(in.uv.x / 5.0, in.uv.y / 5.0), p / 5.0, pad));
        let sf = fract(select(vec2<f32>(in.uv.x / 5.0, in.uv.y / 5.0), p / 5.0, pad));
        colour *= 0.9 + 0.16 * hash12(slab + third);
        let jw = max(footprint, 1e-3) / 5.0;
        let joint = max(band(sf.x, 0.0, 0.012, jw), band(sf.y, 0.0, 0.012, jw)) * (1.0 - smoothstep(0.08, 0.3, footprint));
        colour = mix(colour, colour * 0.45, joint);
        let marks = select(1.0 - smoothstep(2.0, 8.0, x), 0.0, pad) * smoothstep(0.45, 0.85, vnoise(vec2<f32>(in.uv.y / 0.9, in.uv.x / 40.0)));
        colour = mix(colour, srgb_to_linear(vec3<f32>(70.0, 70.0, 68.0)), 0.45 * marks);
        colour = mix(colour, colour * 0.75, step(hash12(slab * 1.7 + 9.0), 0.05));
        fray_m = 0.6;
        rag = 0.12;
    } else if look == 11u {
        // A hedgerow painted across the grass: dark green, mottled, broken here and there.
        colour = srgb_to_linear(vec3<f32>(48.0, 60.0, 34.0)) * (0.75 + 0.5 * vnoise(p / 1.4));
        if vnoise(vec2<f32>(in.uv.x / 7.0, 0.5) + third) < 0.18 {
            discard;
        }
        fray_m = 0.8 * outer;
        rag = 0.3;
    } else if look == 12u {
        // Grass mown for a landing run: paler, in broad stripes across it.
        let stripe = step(0.5, fract(in.uv.y / 9.0 + 0.5));
        colour = srgb_to_linear(vec3<f32>(80.0, 96.0, 50.0)) * (0.84 + 0.04 * stripe + 0.16 * patches);
        fray_m = 0.6 * outer;
    } else if look == 5u {
        // Trampled dust: earth and coral dust, a little oil.
        colour = mix(earth, srgb_to_linear(vec3<f32>(160.0, 150.0, 128.0)), 0.25 + 0.35 * patches);
        colour *= 1.0 - 0.25 * smoothstep(0.7, 0.9, vnoise(p / 4.0 + third));
        fray_m = max(0.45 * outer, 4.0);
    } else {
        // Taxiways, hardstands and roads: tan-grey coral, dirtier, earth tracked over it.
        colour = srgb_to_linear(vec3<f32>(176.0, 168.0, 148.0)) * (0.78 + 0.3 * patches);
        colour *= mix(1.0, 0.9 + 0.2 * grain, shown);
        let dirt = select(select(0.28, 0.32, look == 2u), 0.45, look == 3u) + 0.2 * vnoise(p / 9.0);
        colour = mix(colour, earth, dirt);
        if look == 1u {
            colour = mix(colour, worn * 0.85, 0.3 * (1.0 - smoothstep(1.0, 4.0, x)));
        } else if look == 3u {
            let rut = band(abs(x - 1.05), 0.0, 0.35, max(footprint, 0.05));
            colour = mix(colour, worn * 0.75, 0.45 * rut);
        } else {
            // A hardstand's oil and scuffs about its middle.
            let stain = smoothstep(0.5, 0.78, vnoise(p / 3.5 + third));
            colour = mix(colour, worn * 0.62, 0.5 * stain * smoothstep(0.35, 0.75, edge_in / max(outer, 1.0)));
        }
        fray_m = select(5.0, 3.0, look == 3u);
    }
    // The edge frays into the earth beside it, broken unevenly (a hand-built runway's the more). A road's, a track's,
    // a street's and dust's edge breaks off into the land itself: the ground about it is the land's, among the trees.
    if outer > 0.0 {
        let broken = (vnoise(p / 2.2) - 0.5) * 3.0 + (vnoise(p / 0.6) - 0.5) * 1.0 + (vnoise(p / 9.0) - 0.5) * 3.0;
        let into = smoothstep(0.0, fray_m, edge_in + broken * select(rag, 2.4, look == 7u));
        if look >= 3u && into < 0.3 {
            discard;
        }
        colour = mix(beside, colour, into);
    }
    s.albedo = colour;
    s.roughness = 0.95;
    s.specular = 0.4;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Marston mat (`MAT_MARSTON`: `uv` metres along and across): planks 15 in wide across the strip, 10 ft long, staggered
// by half a plank each row, three rows of holes 1.5 in across in each, two ribs pressed between; steel, rusted here
// and there, the coral showing through the holes. The holes resolve only within some metres, the seams within some
// tens; past that the mat's mean.
const PLANK_M: f32 = 0.380952;
const PLANK_LONG_M: f32 = 3.048;
const HOLE_PITCH_M: f32 = 0.1051;
const HOLE_R_M: f32 = 0.019;
fn marston_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    let p = in.world.xz;
    let along = in.uv.x / PLANK_M;
    let row = floor(along);
    let fa = along - row;
    let stagger = select(0.0, 0.5, fract(row * 0.5) > 0.25);
    let fc = fract(in.uv.y / PLANK_LONG_M + stagger);
    var steel = srgb_to_linear(vec3<f32>(80.0, 82.0, 78.0));
    let rust = srgb_to_linear(vec3<f32>(104.0, 86.0, 66.0));
    let rusted = smoothstep(0.45, 0.85, vnoise(p / 2.5) * 0.6 + vnoise(p / 0.9) * 0.4) * f32(in.colour.x) / 255.0 * 1.4;
    steel = mix(steel, rust, clamp(rusted, 0.0, 1.0));
    // Mats laid at other times a shade apart (some tens of planks each), each plank a shade off, coral dust blown over
    // it, the wheels' path brighter (worn to bare steel) down the middle.
    steel *= 0.84 + 0.26 * hash12(floor(vec2<f32>(in.uv.x / 9.14, in.uv.y / 6.1 + stagger * 0.5)) + 2.0);
    steel *= 0.9 + 0.16 * hash12(vec2<f32>(row, floor(in.uv.y / PLANK_LONG_M + stagger)) + 5.0);
    steel = mix(steel, srgb_to_linear(vec3<f32>(170.0, 160.0, 138.0)) * 0.6, 0.3 * smoothstep(0.55, 0.9, vnoise(p / 14.0 + 4.0)));
    steel *= 1.0 + 0.18 * (1.0 - smoothstep(3.0, 10.0, abs(in.uv.y)));
    // Darkened where the wheels touch down (rubber and oil), 80 to 500 m in from either end; mats laid new here and
    // there, not yet rusted.
    let from_end = min(in.uv.x, f32(in.colour.z) * 16.0 - in.uv.x);
    let touchdown = smoothstep(40.0, 120.0, from_end) * (1.0 - smoothstep(380.0, 550.0, from_end));
    steel *= 1.0 - 0.3 * touchdown * (1.0 - smoothstep(5.0, 14.0, abs(in.uv.y)))
        * (0.6 + 0.4 * vnoise(vec2<f32>(in.uv.y / 1.1, in.uv.x / 30.0)));
    let fresh = step(hash12(floor(vec2<f32>(in.uv.x / 9.14, in.uv.y / 6.1 + stagger * 0.5)) + 19.0), 0.1);
    steel = mix(steel, srgb_to_linear(vec3<f32>(92.0, 96.0, 96.0)), 0.7 * fresh);
    let coral = srgb_to_linear(vec3<f32>(176.0, 166.0, 144.0)) * 0.55;
    // The holes: three rows along each plank, the middle row offset half a pitch.
    let w = max(footprint, 1e-4);
    var hole = 0.0;
    for (var k = 0; k < 3; k++) {
        let at = 0.25 + 0.25 * f32(k);
        let off = select(0.0, 0.5, k == 1);
        let dx = (fract(in.uv.y / HOLE_PITCH_M + off) - 0.5) * HOLE_PITCH_M;
        let dy = (fa - at) * PLANK_M;
        hole = max(hole, clamp((HOLE_R_M - length(vec2<f32>(dx, dy))) / w + 0.5, 0.0, 1.0));
    }
    let resolved = 1.0 - smoothstep(0.012, 0.035, footprint);
    let share = 3.0 * PI * HOLE_R_M * HOLE_R_M / (HOLE_PITCH_M * PLANK_M);
    hole = mix(share, hole, resolved);
    // The ribs between the rows catch the light; the seams between planks are dark.
    let rib = band(fa, 0.36, 0.40, w / PLANK_M) + band(fa, 0.60, 0.64, w / PLANK_M);
    let seam = max(band(fa, 0.0, 0.035, w / PLANK_M), band(fc, 0.0, 0.008, w / PLANK_LONG_M));
    let seams = 1.0 - smoothstep(0.05, 0.2, footprint);
    var colour = steel * (1.0 + 0.25 * rib * seams);
    colour = mix(colour, steel * 0.35, seam * seams * 0.8);
    colour = mix(colour, mix(coral, steel * 0.5, 1.0 - resolved), hole);
    s.albedo = colour;
    s.roughness = mix(0.55, 0.75, rusted);
    s.specular = 1.0;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Bulldozed earth (`MAT_EARTH`), its tops grown over; with `param` 1, sandbags in courses (`uv` metres along and up).
fn earth_surface(in: VertexOut, n: vec3<f32>, footprint: f32) -> Surface {
    var s: Surface;
    let p = in.world.xz;
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    let shown = 1.0 - smoothstep(0.3, 1.5, footprint);
    if in.param > 0.5 {
        // Bags 0.55 m long in courses of 0.16 m, each course half a bag along.
        let course = floor(in.uv.y / 0.16);
        let cy = fract(in.uv.y / 0.16);
        let bx = fract(in.uv.x / 0.55 + course * 0.5);
        let ty = (2.0 * cy - 1.0) * (2.0 * cy - 1.0);
        let tx = (2.0 * bx - 1.0) * (2.0 * bx - 1.0);
        let pillow = (1.0 - ty * ty) * (1.0 - tx * tx * tx);
        let bag = hash12(vec2<f32>(floor(in.uv.x / 0.55 + course * 0.5), course));
        colour *= mix(1.0, (0.62 + 0.42 * pillow) * (0.88 + 0.2 * bag), 1.0 - smoothstep(0.02, 0.08, footprint));
        colour *= 0.9 + 0.15 * vnoise(p / 0.9);
        s.albedo = colour;
        s.roughness = 0.95;
        s.specular = 0.35;
        s.emissive = vec3<f32>(0.0);
        return s;
    }
    colour *= 0.82 + 0.3 * (vnoise(p / 2.5) * 0.6 + vnoise(p / 0.7 + 9.1) * 0.4 * shown);
    // Its foot is the ground it was pushed up from: trampled earth, the bank's own colour above.
    let foot = 1.0 - smoothstep(0.1, 0.9, in.uv.y);
    colour = mix(colour, srgb_to_linear(BASE_EARTH) * (0.85 + 0.25 * vnoise(p / 1.7)), foot * 0.7);
    // Grass and weeds on its top and upper flanks, thinning down its sides.
    let grown = smoothstep(0.55, 0.95, n.y) * smoothstep(0.3, 0.6, vnoise(p / 3.0 + 2.0));
    let grass = srgb_to_linear(vec3<f32>(92.0, 104.0, 58.0)) * (0.85 + 0.3 * vnoise(p / 0.8));
    colour = mix(colour, grass, grown * 0.7);
    // Rain has cut runnels down its flanks.
    let runnel = smoothstep(0.65, 0.9, vnoise(vec2<f32>(in.uv.x / 0.6, in.uv.y / 3.0))) * (1.0 - smoothstep(0.5, 0.9, n.y));
    colour *= 1.0 - 0.18 * runnel * shown;
    s.albedo = colour;
    s.roughness = 0.97;
    s.specular = 0.3;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Corrugated iron (`MAT_IRON`: `uv.x` metres across the corrugation): its ribs 76 mm apart where a pixel resolves them,
// streaks of rust down galvanised sheet.
fn iron_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    let ribs = 0.5 + 0.5 * cos(in.uv.x / 0.0762 * 2.0 * PI);
    let resolved = 1.0 - smoothstep(0.02, 0.07, footprint);
    colour *= mix(1.0, 0.84 + 0.2 * ribs, resolved);
    // Sheets a shade apart (0.8 m wide), rust run down from the laps.
    let sheet = floor(in.uv.x / 0.8);
    colour *= 0.92 + 0.12 * hash12(vec2<f32>(sheet, floor(in.uv.y / 2.4)));
    let grey = abs(colour.r - colour.g) < 0.02;
    let streak = smoothstep(0.62, 0.9, vnoise(vec2<f32>(in.uv.x / 0.35, in.uv.y / 3.5)));
    colour = mix(colour, srgb_to_linear(vec3<f32>(120.0, 86.0, 62.0)), streak * select(0.25, 0.5, grey));
    s.albedo = colour;
    s.roughness = select(0.7, 0.45, grey);
    s.specular = select(0.7, 1.0, grey);
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Tent canvas (`MAT_CANVAS`): mottled, faded where the sun has it (the roof), its panels' seams up close.
fn canvas_surface(in: VertexOut, n: vec3<f32>, footprint: f32) -> Surface {
    var s: Surface;
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    let p = in.world.xz + in.world.y;
    colour *= 0.88 + 0.18 * vnoise(p / 1.6);
    colour *= mix(1.0, 1.12, smoothstep(0.2, 0.7, n.y));
    let seam = band(fract(in.uv.x / 1.2), 0.0, 0.02, max(footprint, 1e-3) / 1.2);
    colour *= 1.0 - 0.25 * seam * (1.0 - smoothstep(0.03, 0.1, footprint));
    s.albedo = colour;
    s.roughness = 0.95;
    s.specular = 0.35;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// Weathered planks (`MAT_WOOD`: `uv` metres along and up): boards 20 cm high each a shade off, the gaps between dark,
// the grain along them.
fn wood_surface(in: VertexOut, footprint: f32) -> Surface {
    var s: Surface;
    var colour = srgb_to_linear(vec3<f32>(in.colour.xyz));
    let board = floor(in.uv.y / 0.2);
    let resolved = 1.0 - smoothstep(0.03, 0.12, footprint);
    colour *= mix(1.0, 0.86 + 0.24 * hash12(vec2<f32>(board, floor(in.uv.x / 3.6) + 11.0)), resolved);
    let gap = band(fract(in.uv.y / 0.2), 0.0, 0.06, max(footprint, 1e-3) / 0.2);
    colour *= 1.0 - 0.5 * gap * resolved;
    colour *= 0.94 + 0.1 * vnoise(vec2<f32>(in.uv.x / 0.9, in.uv.y / 0.05)) * resolved;
    s.albedo = colour;
    s.roughness = 0.9;
    s.specular = 0.4;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// The fuel drums' colours (sRGB): olive drab commonest, then grey, dark blue-grey, red (aviation gasoline's band), rust
// and bare steel.
const DRUM_COLOURS = array<vec3<f32>, 8>(
    vec3<f32>(78.0, 80.0, 56.0),
    vec3<f32>(78.0, 80.0, 56.0),
    vec3<f32>(84.0, 84.0, 60.0),
    vec3<f32>(110.0, 112.0, 108.0),
    vec3<f32>(58.0, 66.0, 78.0),
    vec3<f32>(122.0, 46.0, 38.0),
    vec3<f32>(108.0, 72.0, 50.0),
    vec3<f32>(146.0, 146.0, 140.0),
);
const DRUM_PITCH: f32 = 0.62;
const DRUM_TIER: f32 = 0.88;

// A block of standing drums (`MAT_DRUMS`: `uv` metres along and across its top, or along and up a side; the colour's
// first byte its seed): their heads, round, with dark gaps between; their sides as columns, rounded by the light, two
// rolling hoops each; each drum's colour by its place.
fn drums_surface(in: VertexOut, n: vec3<f32>, footprint: f32) -> Surface {
    var s: Surface;
    let seed = f32(in.colour.x);
    let resolved = 1.0 - smoothstep(0.08, 0.3, footprint);
    var colour: vec3<f32>;
    if n.y > 0.7 {
        let cell = floor(in.uv / DRUM_PITCH);
        let f = (fract(in.uv / DRUM_PITCH) - 0.5) * DRUM_PITCH;
        let r = length(f);
        let pick = min(u32(hash12(cell + seed * 1.37) * 8.0), 7u);
        colour = srgb_to_linear(DRUM_COLOURS[pick]);
        let rim = band(r, 0.25, 0.29, footprint);
        colour *= 1.0 + 0.25 * rim * resolved;
        let gap = 1.0 - clamp((0.29 - r) / max(footprint, 1e-3) + 0.5, 0.0, 1.0);
        colour = mix(colour, vec3<f32>(0.02), mix(0.18, gap, resolved));
    } else {
        let column = floor(in.uv.x / DRUM_PITCH);
        let fx = fract(in.uv.x / DRUM_PITCH);
        let tier = floor(max(in.uv.y - 0.05, 0.0) / DRUM_TIER);
        let pick = min(u32(hash12(vec2<f32>(column, tier) + seed * 1.37 + 0.5) * 8.0), 7u);
        colour = srgb_to_linear(DRUM_COLOURS[pick]);
        let round = sqrt(max(1.0 - (2.0 * fx - 1.0) * (2.0 * fx - 1.0), 0.0));
        colour *= mix(0.78, 0.45 + 0.6 * round, resolved);
        let fy = fract(max(in.uv.y - 0.05, 0.0) / DRUM_TIER);
        let hoops = band(fy, 0.32, 0.36, footprint / DRUM_TIER) + band(fy, 0.64, 0.68, footprint / DRUM_TIER);
        colour *= 1.0 - 0.3 * hoops * resolved;
    }
    s.albedo = colour;
    s.roughness = 0.6;
    s.specular = 0.8;
    s.emissive = vec3<f32>(0.0);
    return s;
}

// A face's colour, lit and seen through the air.
fn shade(in: VertexOut) -> vec3<f32> {
    let n = normalize(in.normal);
    let material = in.colour.w;
    // Every derivative before the branches.
    let dx = dpdx(in.world);
    let dy = dpdy(in.world);
    let footprint = max(max(length(dx), length(dy)), 0.01);
    let wuv = fwidth(in.uv);
    var s: Surface;
    s.albedo = srgb_to_linear(vec3<f32>(in.colour.xyz));
    s.roughness = 0.55;
    s.specular = 0.8;
    s.emissive = vec3<f32>(0.0);
    if material == MAT_STACKS || material == MAT_CONTAINER {
        s = stacks_surface(in, n, wuv);
    } else if material == MAT_DECK {
        s = deck_surface(in, wuv);
    } else if material == MAT_RUNWAY {
        s = runway_surface(in, wuv, footprint);
    } else if material == MAT_TAXIWAY {
        s = taxiway_surface(in, wuv, footprint);
    } else if material == MAT_APRON {
        s = apron_surface(in, footprint);
    } else if material == MAT_CORAL {
        s = coral_surface(in, footprint);
    } else if material == MAT_MARSTON {
        s = marston_surface(in, footprint);
    } else if material == MAT_EARTH {
        s = earth_surface(in, n, footprint);
    } else if material == MAT_IRON {
        s = iron_surface(in, footprint);
    } else if material == MAT_CANVAS {
        s = canvas_surface(in, n, footprint);
    } else if material == MAT_WOOD {
        s = wood_surface(in, footprint);
    } else if material == MAT_DRUMS {
        s = drums_surface(in, n, footprint);
    } else if material == MAT_CONCRETE {
        // Weathered: mottled a metre or two across, darker low down.
        let cell = floor(in.world.xz / 2.0) + floor(in.world.y / 2.0);
        let mottle = hash12(cell);
        let shown = 1.0 - smoothstep(1.0, 3.0, footprint);
        s.albedo *= mix(1.0, 0.9 + 0.16 * mottle, shown);
        s.roughness = 0.92;
        s.specular = 0.5;
    } else if material == MAT_GLASS {
        s.albedo = vec3<f32>(0.02, 0.025, 0.03);
        s.roughness = 0.07;
        s.specular = 1.2;
        // A cabin lit now and then at night (fewer under a dimout or a blackout).
        let lit = select(0.0, 1.0, hash12(floor(in.world.xz / 4.0)) < 0.18 * (1.0 - infra.params.z));
        s.emissive = vec3<f32>(1.0, 0.78, 0.5) * 0.25 * lit * infra.sun_direction.w;
    } else if material == MAT_HULL {
        s.roughness = 0.28;
        s.specular = 1.0;
    } else if material == MAT_STEEL {
        s.roughness = 0.5;
        s.specular = 0.9;
    }
    return lit(in.world, n, s);
}

// Surface `s` at `world`, facing unit `n`, lit (the sun through the shadow maps and the clouds' shadow, the hemisphere
// fill, the sky's environment) and seen through the air. No derivatives: lines are lit at their vertices.
fn lit(world: vec3<f32>, n: vec3<f32>, s: Surface) -> vec3<f32> {
    let albedo = tone(s.albedo);
    let view = normalize(camera.position - world);
    let light_dir = infra.sun_direction.xyz;
    var sun = infra.sun_radiance.rgb * sun_shadow(world, n);
    if infra.fill_sky.w > 0.5 {
        sun *= cloud_shadow(cloud_map, cloud_sampler, cloud_params, world);
    }
    let dot_nl = clamp(dot(n, light_dir), 0.0, 1.0);
    let dot_nv = clamp(dot(n, view), 1e-4, 1.0);
    let roughness = clamp(s.roughness, 0.06, 1.0);
    let f0 = vec3<f32>(0.04 * s.specular);
    let f90 = clamp(s.specular, 0.0, 1.0);
    let intensity = infra.sun_radiance.w;
    let fill = mix(infra.fill_ground.rgb, infra.fill_sky.rgb, n.y * 0.5 + 0.5);
    let bounce = infra.sun_radiance.rgb * max(light_dir.y, 0.0) * vec3<f32>(0.13, 0.12, 0.1) * (0.5 - 0.5 * n.y);
    let irradiance = env_irradiance(sky_atlas, sky_sampler, sky_env, n) * intensity + bounce;
    let reflected = env_radiance(sky_atlas, sky_sampler, sky_env, env_reflection_direction(n, view, roughness), roughness) * intensity;
    let fresnel = f0 + (vec3<f32>(f90) - f0) * pow(1.0 - dot_nv, 5.0) * (1.0 - roughness * 0.7);
    let direct = dot_nl * sun * (albedo * RECIPROCAL_PI + brdf_ggx(light_dir, n, view, f0, f90, roughness));
    let indirect = albedo * (1.0 - fresnel) * RECIPROCAL_PI * (irradiance + fill) + reflected * fresnel;
    var color = direct + indirect + s.emissive;
    if infra.fog.w > 0.5 {
        color = aerial_apply(color, world);
    }
    return color;
}

@fragment
fn fs_infra(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(shade(in), 1.0);
}

// A line (`MAT_WIRE`, `MAT_LATTICE`), drawn wider than it is (`place`), lit at its vertices: a pixel or two wide, it
// shows no more, and its fragments (several over each other where a tower afar or a corridor's spans stand) cost a
// blend each. Over what stands behind it, as opaque as the share of its pixel it covers. A lattice's member afar no
// more, so a tower thins to nothing with distance as its steel does. A wire more (the share's root from a quarter of
// the pixel up, as the sky's fade had it: a span across the sky stays just seen), below that the share again (2.8
// times), so a corridor's many spans afar never add up to a band.
@vertex
fn vs_infra_line(in: VertexIn) -> LineOut {
    let v = place(in);
    var s: Surface;
    s.albedo = srgb_to_linear(vec3<f32>(in.colour.xyz));
    s.emissive = vec3<f32>(0.0);
    if in.colour.w == MAT_WIRE {
        // Weathered aluminium strands.
        s.roughness = 0.45;
        s.specular = 1.0;
    } else {
        // Galvanised angles, a line standing for a lattice whose inner members and far faces are in the near ones'
        // shade: darker than a beam of it close by.
        s.albedo *= 0.6;
        s.roughness = 0.5;
        s.specular = 0.9;
    }
    let covered = clamp(v.param, 0.0, 1.0);
    let wire = min(min(sqrt(covered) * 1.4, covered * 2.8), 1.0);
    let alpha = select(covered, wire, in.colour.w == MAT_WIRE);
    var out: LineOut;
    out.clip = v.clip;
    out.colour = vec4<f32>(lit(v.world, normalize(v.normal), s), alpha);
    return out;
}

@fragment
fn fs_infra_line(in: LineOut) -> @location(0) vec4<f32> {
    return in.colour;
}

// A line in the motion pass (`InfraLayer::draw_motion`). It writes no depth, so the resolve follows its pixels by
// what lies behind it, the sky's or the ground's: as the eye travels a line seen to move against that (a wire before
// the sky, by the eye's step over its distance) would trail across its history. By as much as it moves against the
// sky (its point at infinity, the farthest behind it can be), from a quarter of a pixel to a pixel, it takes no
// history; held still or turned, the eye sees no line move against its background, and the line keeps its history.
// A line afar barely moves (at 1 km, a third of a pixel a frame with the eye crossing at 30 m/s) and keeps most of it.
struct LineMotionOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) response: f32,
}

@vertex
fn vs_infra_line_motion(in: VertexIn) -> LineMotionOut {
    let v = place(in);
    let here = curved(v.world);
    let line = camera_motion.previous_view_projection * vec4<f32>(here, 1.0);
    let sky = camera_motion.previous_view_projection * vec4<f32>(here - camera.position, 0.0);
    let apart = (line.xy / line.w - sky.xy / sky.w) * 0.5 * camera.viewport.xy;
    var out: LineMotionOut;
    out.clip = v.clip;
    out.response = smoothstep(0.25, 1.0, length(apart));
    return out;
}

@fragment
fn fs_infra_line_motion(in: LineMotionOut) -> @location(0) vec4<f32> {
    return motion_response(in.response);
}
