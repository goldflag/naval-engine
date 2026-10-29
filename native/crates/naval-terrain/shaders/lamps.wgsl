// The lamps on the land at night (`naval_terrain::night`, native/docs/terrain/night.md): street lights,
// floods and their pools, obstruction lights, one instance each (`Lamp`), added over the scene in the transparent phase,
// tested against its depth. The vertex stage stands a lamp on the ground (a beacon at its own height), drops it where
// the scenario's rule puts it out (a leak kept now and then), a street light off built-up land and most floods inland,
// and draws a point at least `MIN_PX` across facing the camera (a pool flat on the ground) with its light spread over
// it; the fragment stage shapes it, the air dimming it on the way (`aerial_apply_additive`).
#import naval::camera::{camera, world_to_clip}
#import naval::aerial::aerial_apply_additive
#import naval_terrain::bindings::{terrain, attributes, surface_cover, clamp_sampler, height_at, coarse_uv, chart_offset, country_test_mark}
#import naval_terrain::night::{lamp_color, night_hash, night_ramp, LAMP_STREET, LAMP_FLOOD, LAMP_POOL, LAMP_BEACON, LAMP_RUNWAY, WINDOW}

// The least a lamp is drawn across (pixels; `night::MIN_PX`), and how its light falls off below it: 2 keeps its whole
// light (a far town's thousands of lamps sum to what they are), less keeps a far beacon a little brighter than its
// size, as the eye picks it out.
const MIN_PX: f32 = 1.25;
const SUB_PIXEL_POWER: f32 = 2.0;
const STREET_SUB_PIXEL_POWER: f32 = 1.7;
const BEACON_SUB_PIXEL_POWER: f32 = 1.5;
// The quad's half side over the lamp's core: the rest is its halo.
const QUAD: f32 = 2.5;
// The light's core: its radius (m) and its luminance by kind.
const STREET_M: f32 = 0.35;
const FLOOD_M: f32 = 0.9;
const BEACON_M: f32 = 0.3;
const RUNWAY_M: f32 = 0.25;
const STREET_LUM: f32 = 40.0;
const FLOOD_LUM: f32 = 60.0;
const BEACON_LUM: f32 = 30.0;
const RUNWAY_LUM: f32 = 30.0;
// A flood's pool: the spread of its light on the ground (m, a Gaussian's), its quad's half side in spreads, and its
// luminance beneath (the lit ground's): the masts' pools 110 m apart merge into a yard lit nearly evenly.
const POOL_M: f32 = 52.0;
const POOL_SPREADS: f32 = 1.8;
// Pools fade out between these distances (m): beyond, a sliver on screen, the flood's own light carries the yard.
const POOL_NEAR_M: f32 = 3500.0;
const POOL_FAR_M: f32 = 5000.0;
const POOL_LUM: f32 = 0.04;
// A flood's class on a pier (`lamps.rs`'s `ON_PIER`): kept over the water; a street light's on a jetty (`JETTY`):
// kept off built-up land.
const ON_PIER: u32 = 1u;
const JETTY: u32 = 8u;
// A leak's strength, of a lit lamp's.
const LEAK: f32 = 0.35;
// A flood farther than this from the sea (m) is inland industry: one in three of them, dimmer.
const INLAND_M: f32 = 450.0;
// The coast distances' reach (`field::COAST_RANGE_M`).
const COAST_RANGE_M: f32 = 8000.0;
// ESA WorldCover's built-up class.
const BUILT: u32 = 50u;

struct LampIn {
    @location(0) at: vec2<f32>,
    @location(1) height: f32,
    @location(2) info: u32,
}

struct LampOut {
    @builtin(position) clip: vec4<f32>,
    // The quad's corner, -1 to 1.
    @location(0) corner: vec2<f32>,
    @location(1) @interpolate(flat) radiance: vec3<f32>,
    @location(2) world: vec3<f32>,
    @location(3) @interpolate(flat) kind: u32,
}

// The signed distance to the coast at a chart point (m, positive on land).
fn coast_m(chart: vec2<f32>) -> f32 {
    let b = (textureSampleLevel(attributes, clamp_sampler, coarse_uv(chart), 0.0).b - 0.5) * 2.0;
    return sign(b) * b * b * COAST_RANGE_M;
}

// How much of the window's cover about a chart point is built up (four texels about it, 60 m apart); 1 with no maps.
fn built_about(chart: vec2<f32>) -> f32 {
    let size = i32(terrain.surface.w);
    if size <= 0 {
        return 1.0;
    }
    let texel = max(terrain.surface.z / terrain.surface.w, 1.0);
    var built = 0.0;
    for (var k = 0; k < 4; k++) {
        let offset = vec2<f32>(f32(k & 1) - 0.5, f32(k >> 1u) - 0.5) * 60.0;
        let at = clamp(vec2<i32>(floor((chart + offset - terrain.surface.xy) / texel)), vec2<i32>(0), vec2<i32>(size - 1));
        // Land a past period left as country (`cover::PAST_COUNTRY` over built-up, or a test's mark) is not built up:
        // no street light.
        let marked = country_test_mark(terrain.surface.xy + (vec2<f32>(at) + 0.5) * texel);
        built += select(0.0, 0.25, textureLoad(surface_cover, at, 0, 0).r == BUILT && !marked);
    }
    return built;
}

@vertex
fn vs_lamp(@builtin(vertex_index) vertex: u32, lamp: LampIn) -> LampOut {
    var out: LampOut;
    // Outside the view: nothing drawn.
    out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    out.corner = vec2<f32>(0.0);
    out.radiance = vec3<f32>(0.0);
    out.world = vec3<f32>(0.0);
    let kind = lamp.info & 15u;
    out.kind = kind;
    let road = (lamp.info >> 4u) & 15u;
    let bright = f32((lamp.info >> 8u) & 255u) / 100.0;
    let h = night_hash(lamp.info >> 16u);
    // The rule's strength for the kind, or a leak's.
    var scale = 0.0;
    if kind == LAMP_STREET || kind == LAMP_RUNWAY {
        scale = terrain.night.y;
    } else if kind == LAMP_FLOOD || kind == LAMP_POOL {
        scale = terrain.night.w;
    } else if kind == LAMP_BEACON {
        scale = terrain.night_rule.x;
    }
    var leaking = false;
    if scale < LEAK && kind != LAMP_BEACON && kind != LAMP_POOL && h < terrain.night_rule.y {
        scale = LEAK;
        leaking = true;
    }
    var power = bright * scale * terrain.night.x;
    if power <= 0.0 {
        return out;
    }
    let chart = lamp.at;
    let ground = max(height_at(chart), 0.0);
    if kind == LAMP_STREET && road != JETTY && built_about(chart) < 0.25 {
        return out;
    }
    if kind == LAMP_FLOOD || kind == LAMP_POOL {
        let coast = coast_m(chart);
        // A harbour's or a port's land drawn over its basins: no floods on the water but a pier's.
        if coast < -15.0 && road != ON_PIER {
            return out;
        }
        if coast > INLAND_M {
            if h > 0.35 {
                return out;
            }
            power *= 0.6;
        }
    }
    let offset = chart_offset();
    var y = ground + lamp.height;
    if kind == LAMP_BEACON || kind == LAMP_RUNWAY {
        y = lamp.height;
    }
    let world = vec3<f32>(chart.x + offset.x, y, chart.y + offset.y);
    let to_eye = camera.position - world;
    let d = max(length(to_eye), 1.0);
    let corner = vec2<f32>(f32(vertex & 1u), f32(vertex >> 1u)) * 2.0 - 1.0;
    out.corner = corner;
    var color = lamp_color(kind, road, h);
    if leaking {
        color = WINDOW;
    }
    if kind == LAMP_POOL {
        power *= night_ramp(POOL_FAR_M, POOL_NEAR_M, d);
        if power <= 0.0 {
            return out;
        }
        // Flat on the ground, raised a little (and toward the eye) so the land does not swallow it.
        let r = POOL_M * POOL_SPREADS * (0.85 + 0.3 * fract(h * 17.0));
        let p = vec3<f32>(world.x + corner.x * r, ground + 0.8, world.z + corner.y * r);
        out.world = p;
        out.clip = world_to_clip(p + normalize(camera.position - p) * min(0.5 + d * 0.002, 6.0));
        out.radiance = color * (POOL_LUM * power);
        return out;
    }
    // A shielded light (the dimout's hoods) shows only from above.
    if terrain.night_rule.z > 0.5 && !leaking && (kind == LAMP_STREET || kind == LAMP_FLOOD) {
        power *= night_ramp(0.2, 0.55, to_eye.y / d);
    }
    var radius = STREET_M;
    var lum = STREET_LUM;
    if kind == LAMP_FLOOD {
        radius = FLOOD_M;
        lum = FLOOD_LUM;
    } else if kind == LAMP_BEACON {
        radius = BEACON_M;
        lum = BEACON_LUM;
        // Half of them flash (about 37 a minute), the rest burn steady.
        if h < 0.5 {
            let f = fract(camera.time / 1.6 + h * 7.0);
            power *= night_ramp(0.0, 0.05, f) * night_ramp(0.5, 0.42, f);
        }
    } else if kind == LAMP_RUNWAY {
        radius = RUNWAY_M;
        lum = RUNWAY_LUM;
    }
    // Pixels across its core, and its light spread over `MIN_PX` when smaller.
    let pixel = 2.0 * camera.tan_half_fov_y * camera.viewport.w;
    let core_px = radius / (d * pixel);
    let size_px = max(core_px, MIN_PX);
    var fall = SUB_PIXEL_POWER;
    if kind == LAMP_BEACON || kind == LAMP_RUNWAY {
        fall = BEACON_SUB_PIXEL_POWER;
    } else if kind == LAMP_STREET {
        fall = STREET_SUB_PIXEL_POWER;
    }
    power *= pow(min(core_px / MIN_PX, 1.0), fall);
    if power < 1e-4 {
        return out;
    }
    // Toward the eye a little, clear of the road or roof it stands over.
    let pulled = world + to_eye / d * min(1.0 + d * 0.003, 40.0);
    var clip = world_to_clip(pulled);
    clip = vec4<f32>(clip.xy + corner * (size_px * QUAD) * 2.0 * camera.viewport.zw * clip.w, clip.zw);
    out.clip = clip;
    out.world = world;
    out.radiance = color * (lum * power);
    return out;
}

@fragment
fn fs_lamp(in: LampOut) -> @location(0) vec4<f32> {
    let r2 = dot(in.corner, in.corner);
    if r2 >= 1.0 {
        discard;
    }
    var shape: f32;
    if in.kind == LAMP_POOL {
        // A Gaussian, its tail eased to nothing at the quad's edge.
        shape = (exp(-r2 * (POOL_SPREADS * POOL_SPREADS)) - exp(-POOL_SPREADS * POOL_SPREADS)) * (1.0 - r2);
    } else {
        // The core (its radius a third of the quad's) and a faint halo out to the quad's edge.
        shape = exp(-r2 * (QUAD * QUAD)) + 0.04 * (1.0 - r2) * (1.0 - r2);
    }
    return vec4<f32>(aerial_apply_additive(in.radiance * shape, in.world), 1.0);
}
