// Photo mode's optics (native/docs/photo.md, "The lens"; `naval_render::photo_optics`). No TypeScript source: the
// browser game had no photo mode. The flare a camera's lens draws, in the scene's radiance before the tone map:
//
// - `sources` (compute, a workgroup per 64-pixel tile): the frame's radiance above twice its white (the eye's exposure
//   and photo mode's EV), each tile's flux (radiance times pixels), the area it covers and its centroid. The sky's
//   drawn sun is left out: the sun is a source of its own.
// - `sun` (compute, one workgroup): the sun's flux, its irradiance over a pixel's solid angle (from the host), times
//   what of its drawn disc the frame shows over the sky about it, against the disc's unobstructed radiance (a hull or
//   a cloud in front dims it); a sun outside the frame is taken as clear.
// - `glare_first`, `glare_down`, `glare_up` (the uncoated lens): the frame at a quarter of its size, halved seven
//   times, then summed back up, each level weighted 2^-0.5 against the next finer (an r^-2.5 kernel).
// - `composite` (full resolution): the frame less the share the lens scatters, the glare it scatters back and the
//   sun's own glare (the same kernel as Gaussians an octave apart).
// - `sprite_vertex`, `sprite_fragment` (instanced, a source each, added onto the composite): the ghosts (images of
//   the aperture on the line through the centre, the flux times the two surfaces' reflectances over their area) and
//   the starburst's spikes (along each blade edge's normal, falling as the square of the distance).
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::color::luminance

struct OpticsParams {
    // Width, height, 1 / width, 1 / height of the frame drawn.
    frame: vec4<f32>,
    // Tiles across and down, a tile's pixels, sources (tiles + the sun's, first).
    tiles: vec4<f32>,
    // The display's exposure with photo mode's EV (the eye's stops multiply in), the sources' threshold (graded), the
    // sprites' cutoff (graded), the frame drawn over the photo's.
    exposure: vec4<f32>,
    // Blades (0 round), the blades' angle in the frame drawn (a vertex there, radians), the polygon's share of the
    // aperture's shape, the f-number.
    lens: vec4<f32>,
    // The photo's height in drawn pixels, the ghosts' size factor (4 / N), the share of the frame's light the lens
    // scatters (η, 0 for a coated lens), 0.
    lens2: vec4<f32>,
    // The glare's weights: the frame's halo over its levels' weights' sum and its plateau, as shares of its η; the
    // sun's halo η over the weights' sum and its plateau η over the image circle's area.
    veil: vec4<f32>,
    // The spikes' flux per pixel of length at a pixel from a source of unit flux, per channel; the spike count.
    spike: vec4<f32>,
    // The sun's image (drawn pixels), 1 in front of the camera, the lens's acceptance of it.
    sun: vec4<f32>,
    // The sun's flux per unit of what reaches the lens; 1 with a sun.
    sun_flux: vec4<f32>,
    // The sun's own radius (pixels), its drawn disc's (pixels), the drawn disc's unobstructed radiance (luminance),
    // what reaches the lens of a sun out of the frame.
    sun_disc: vec4<f32>,
    // The glare pyramid's base size, its levels, 0.
    glare: vec4<f32>,
    // The camera's near and far planes, the point lights given.
    planes: vec4<f32>,
    // The depth of field: its circle of confusion's diameter at infinity (pixels), the focus (m), 1 when on.
    focus: vec4<f32>,
    // Per point light: its image (drawn pixels), its distance along the view axis (m), its radius (pixels); the same
    // of the top of the column above it its flames may rise to; its flux before what of it the frame hides.
    lights: array<vec4<f32>, 24>,
    // Per ghost: its centre over the source's offset from the centre, its radius in frame heights at f/4.
    ghosts: array<vec4<f32>, 8>,
    // Per ghost: its share of the source's flux per channel.
    tints: array<vec4<f32>, 8>,
}

// A source: flux (RGB) and the area it covers (pixels); its centroid (drawn pixels), its radius and its distance along
// the view axis (m; the sky's far off).
struct Source {
    flux: vec4<f32>,
    at: vec4<f32>,
}

@group(0) @binding(0) var scene: texture_2d<f32>;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var<uniform> params: OpticsParams;
@group(0) @binding(3) var aux: texture_2d<f32>;
@group(0) @binding(4) var linear_clamp: sampler;
@group(0) @binding(5) var<storage, read_write> sources_out: array<Source>;
@group(0) @binding(6) var<storage, read> eye: array<f32, 4>;
@group(0) @binding(7) var<storage, read> sources_in: array<Source>;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.28318531;
const GOLDEN_ANGLE: f32 = 2.39996323;
const SQRT_TAU: f32 = 2.50662827;
// `GLARE_FALLOFF` and `GLARE_SIGMA0` in photo_optics.rs.
const GLARE_FALLOFF: f32 = 0.70710678;
const GLARE_SIGMA0: f32 = 4.8;
// `COMPOSITE_CEILING` and `SPRITE_CEILING`: the half-float targets stay finite where the flare piles up.
const COMPOSITE_CEILING: f32 = 16384.0;
const SPRITE_CEILING: f32 = 1024.0;
// `MAX_LIGHTS`: sources 1 to 8 are the point lights, the tiles follow.
const LIGHTS: u32 = 8u;
// A source in the sky, as far as the starburst's focus goes.
const FAR: f32 = 1e9;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

// ---- Sources ----

var<workgroup> partial: array<array<f32, 8>, 64>;

@compute @workgroup_size(8, 8)
fn sources(
    @builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) local_id: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
) {
    let size = vec2<i32>(params.frame.xy);
    let tile = i32(params.tiles.z);
    let step = tile / 8;
    let base = vec2<i32>(group.xy) * tile + vec2<i32>(local_id.xy) * step;
    let exposure = params.exposure.x * exp2(eye[0]);
    let threshold = params.exposure.y;
    // The sky's drawn sun (and its rim) is the sun's source, not the frame's.
    let sun_in = params.sun.z > 0.5;
    let exclude = params.sun_disc.y * 1.25 + 2.0;
    var flux = vec3<f32>(0.0);
    var area = 0.0;
    var moment = vec2<f32>(0.0);
    var weight = 0.0;
    for (var y = 0; y < step; y++) {
        for (var x = 0; x < step; x++) {
            let p = base + vec2<i32>(x, y);
            if (p.x >= size.x || p.y >= size.y) {
                continue;
            }
            let centre = vec2<f32>(p) + 0.5;
            if (sun_in && distance(centre, params.sun.xy) < exclude) {
                continue;
            }
            let radiance = textureLoad(scene, p, 0).rgb;
            let l = luminance(radiance);
            let w = smoothstep(threshold, 2.0 * threshold, l * exposure);
            if (w <= 0.0) {
                continue;
            }
            flux += radiance * w;
            area += w;
            moment += centre * (l * w);
            weight += l * w;
        }
    }
    partial[local] = array<f32, 8>(flux.r, flux.g, flux.b, area, moment.x, moment.y, weight, 0.0);
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (local < stride) {
            for (var k = 0; k < 7; k++) {
                partial[local][k] += partial[local + stride][k];
            }
        }
        workgroupBarrier();
    }
    if (local == 0u) {
        let total = partial[0];
        let index = 1u + LIGHTS + group.y * u32(params.tiles.x) + group.x;
        var centroid = vec2<f32>(0.0);
        var far = FAR;
        if (total[6] > 0.0) {
            centroid = vec2<f32>(total[4], total[5]) / total[6];
            let d = textureLoad(depth, clamp(vec2<i32>(centroid), vec2<i32>(0), size - vec2<i32>(1)), 0);
            far = select(view_depth(d), FAR, d <= 0.0);
        }
        sources_out[index] = Source(
            vec4<f32>(total[0], total[1], total[2], total[3]),
            vec4<f32>(centroid, sqrt(total[3] / PI), far),
        );
    }
}

var<workgroup> sun_parts: array<vec4<f32>, 64>;
var<workgroup> sun_seen: array<vec2<f32>, 64>;

@compute @workgroup_size(64)
fn sun(@builtin(local_invocation_index) local: u32) {
    let has = params.sun.z > 0.5 && params.sun_flux.w > 0.5;
    let size = params.frame.xy;
    let centre = params.sun.xy;
    let drawn = params.sun_disc.y;
    var part = vec4<f32>(0.0);
    // A disc sample in the frame, and whether it shows the sky (not a hull or land in front, however bright).
    var disc = vec2<f32>(0.0);
    if (has) {
        if (local < 48u) {
            // Across the drawn disc, inside its soft rim: a golden-angle spiral.
            let i = f32(local) + 0.5;
            let a = i * GOLDEN_ANGLE;
            let p = centre + vec2<f32>(cos(a), sin(a)) * drawn * 0.85 * sqrt(i / 48.0);
            if (all(p >= vec2<f32>(0.0)) && all(p < size)) {
                let at = vec2<i32>(p);
                let sky = textureLoad(depth, at, 0) <= 0.0;
                disc = vec2<f32>(1.0, select(0.0, 1.0, sky));
                if (sky) {
                    part = vec4<f32>(luminance(textureLoad(scene, at, 0).rgb), 0.0, 0.0, 0.0);
                }
            }
        } else {
            // The sky about it: a ring outside the rim, where it is sky.
            let a = (f32(local - 48u) + 0.5) / 16.0 * TAU;
            let p = centre + vec2<f32>(cos(a), sin(a)) * (drawn * 1.6 + 2.0);
            if (all(p >= vec2<f32>(0.0)) && all(p < size)) {
                let at = vec2<i32>(p);
                if (textureLoad(depth, at, 0) <= 0.0) {
                    part = vec4<f32>(0.0, 0.0, luminance(textureLoad(scene, at, 0).rgb), 1.0);
                }
            }
        }
    }
    sun_parts[local] = part;
    sun_seen[local] = disc;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride = stride >> 1u) {
        if (local < stride) {
            sun_parts[local] += sun_parts[local + stride];
            sun_seen[local] += sun_seen[local + stride];
        }
        workgroupBarrier();
    }
    if (local == 0u) {
        var flux = vec3<f32>(0.0);
        if (has) {
            let total = sun_parts[0];
            let samples = sun_seen[0];
            // What of it reaches the lens: measured where its disc is in the frame, the samples a hull or land hides
            // giving none.
            var seen = params.sun_disc.w;
            if (samples.x > 0.0) {
                let sky = select(0.0, total.z / max(total.w, 1.0), total.w > 0.0);
                let excess = max(total.x - samples.y * sky, 0.0) / samples.x;
                seen = clamp(excess / max(params.sun_disc.z, 1e-6), 0.0, 1.0);
            }
            flux = params.sun_flux.rgb * (seen * params.sun.w);
        }
        let r = params.sun_disc.x;
        sources_out[0] = Source(vec4<f32>(flux, PI * r * r), vec4<f32>(centre, r, FAR));
    }
}

// Metres along the view axis from reversed depth (1 at the near plane, 0 at the far plane, the sky's clear).
fn view_depth(d: f32) -> f32 {
    let near = params.planes.x;
    let far = params.planes.y;
    return far * near / (d * (far - near) + near);
}

var<workgroup> light_seen: array<vec4<f32>, 64>;

// The point lights: the lens takes each one's light where the frame shows its image, the flame or flash it stands for,
// at least as bright as the frame's white within a window about it (from a sixth under the light to the top of the
// column its flames may rise to: a fire's light stands a few metres over the burning compartment, inside the hull from
// most sides). None shows (hidden behind a hull, or out of sight): none of its light; else all of it, set at the
// image's centre by its light, as wide as the image.
@compute @workgroup_size(64)
fn lights(@builtin(local_invocation_index) local: u32) {
    let light = local / 8u;
    let lane = local % 8u;
    let base = params.lights[3u * light];
    let top = params.lights[3u * light + 1u];
    let flux = params.lights[3u * light + 2u].rgb;
    let exposure = params.exposure.x * exp2(eye[0]);
    var found = vec4<f32>(0.0);
    var outside = 0.0;
    if (f32(light) < params.planes.z && max(flux.r, max(flux.g, flux.b)) > 0.0) {
        let low = mix(base.xy, top.xy, -1.0 / 6.0);
        let centre = 0.5 * (low + top.xy);
        let half = min(max(0.5 * length(top.xy - low) + 2.0 * base.w, 4.0), 16.0);
        let first = vec2<i32>(floor(centre - vec2<f32>(half)));
        let side = i32(ceil(2.0 * half));
        let size = vec2<i32>(params.frame.xy);
        if (any(centre < vec2<f32>(0.0)) || any(centre >= params.frame.xy)) {
            outside = 1.0;
        } else {
            // Eight lanes share the window's rows.
            for (var y = i32(lane); y < side; y += 8) {
                for (var x = 0; x < side; x++) {
                    let p = first + vec2<i32>(x, y);
                    if (any(p < vec2<i32>(0)) || any(p >= size)) {
                        continue;
                    }
                    let l = luminance(textureLoad(scene, p, 0).rgb) * exposure;
                    let bright = smoothstep(0.5, 1.0, l);
                    if (bright > 0.0) {
                        found += vec4<f32>((vec2<f32>(p) + 0.5) * (bright * l), bright * l, bright);
                    }
                }
            }
        }
    }
    light_seen[local] = found;
    workgroupBarrier();
    if (lane == 0u) {
        var total = vec4<f32>(0.0);
        for (var k = 0u; k < 8u; k++) {
            total += light_seen[local + k];
        }
        var at = base.xy;
        var radius = base.w;
        var shown = outside;
        if (total.w > 0.0) {
            at = total.xy / max(total.z, 1e-6);
            radius = clamp(sqrt(total.w / PI), base.w, 8.0);
            shown = 1.0 - exp(-total.w / max(PI * base.w * base.w, 1.0));
        }
        sources_out[1u + light] = Source(vec4<f32>(flux * shown, PI * radius * radius), vec4<f32>(at, radius, base.z));
    }
}

// ---- Veiling glare ----

// A quarter of the frame: the average of its 4 × 4 pixels, four bilinear taps at its quarters' centres.
@fragment
fn glare_first(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let inv = 1.0 / vec2<f32>(textureDimensions(scene));
    let centre = in.position.xy * 4.0;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let offset = vec2<f32>(f32(i & 1) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0);
        sum += textureSampleLevel(scene, linear_clamp, (centre + offset) * inv, 0.0).rgb;
    }
    return vec4<f32>(sum * 0.25, 1.0);
}

// Half of the level above: four bilinear taps over its 4 × 4 texels.
@fragment
fn glare_down(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let inv = 1.0 / vec2<f32>(textureDimensions(aux));
    let centre = in.position.xy * 2.0;
    var sum = vec3<f32>(0.0);
    for (var i = 0; i < 4; i++) {
        let offset = vec2<f32>(f32(i & 1) * 2.0 - 1.0, f32(i >> 1u) * 2.0 - 1.0);
        sum += textureSampleLevel(aux, linear_clamp, (centre + offset) * inv, 0.0).rgb;
    }
    return vec4<f32>(sum * 0.25, 1.0);
}

// A level's weight in the glare: its share of the halo, and the coarsest's the plateau besides.
fn glare_weight(level: f32) -> f32 {
    let levels = params.glare.z;
    return params.veil.x * pow(GLARE_FALLOFF, level) + select(0.0, params.veil.y, level >= levels - 1.0);
}

// A level weighted, with the coarser sum brought up through a tent.
@fragment
fn glare_up(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let own = textureLoad(scene, vec2<i32>(in.position.xy), 0).rgb;
    let level = round(log2(params.glare.x / f32(textureDimensions(scene).x)));
    let levels = params.glare.z;
    // The coarsest level comes in unweighted: weight it here.
    let coarser_weight = select(1.0, glare_weight(levels - 1.0), level >= levels - 2.0);
    let inv = 1.0 / vec2<f32>(textureDimensions(aux));
    var up = vec3<f32>(0.0);
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let w = f32((2 - abs(x)) * (2 - abs(y))) / 16.0;
            up += textureSampleLevel(aux, linear_clamp, in.uv + vec2<f32>(f32(x), f32(y)) * inv, 0.0).rgb * w;
        }
    }
    return vec4<f32>(own * glare_weight(level) + up * coarser_weight, 1.0);
}

// The glare's halo at `r2` pixels squared from a unit flux (per pixel), before the weights' sum: the pyramid's levels
// as Gaussians an octave apart.
fn glare_kernel(r2: f32) -> f32 {
    var k = 0.0;
    var weight = 1.0;
    var sigma = GLARE_SIGMA0;
    let levels = i32(params.glare.z);
    for (var i = 0; i < levels; i++) {
        let s2 = sigma * sigma;
        k += weight * exp(-0.5 * r2 / s2) / (TAU * s2);
        weight *= GLARE_FALLOFF;
        sigma *= 2.0;
    }
    return k;
}

@fragment
fn composite(in: FullscreenVertex) -> @location(0) vec4<f32> {
    let base = textureLoad(scene, vec2<i32>(in.position.xy), 0);
    var color = base.rgb;
    let eta = params.lens2.z;
    if (eta > 0.0) {
        let glare = textureSampleLevel(aux, linear_clamp, in.uv, 0.0).rgb;
        color = color * (1.0 - eta) + glare * eta;
    }
    // The sun's: its halo about it and its plateau over the image circle.
    let sun = sources_in[0];
    if (max(sun.flux.r, max(sun.flux.g, sun.flux.b)) > 0.0) {
        let d = in.position.xy - sun.at.xy;
        color += sun.flux.rgb * (params.veil.z * glare_kernel(dot(d, d)) + params.veil.w);
    }
    return vec4<f32>(min(color, vec3<f32>(COMPOSITE_CEILING)), base.a);
}

// ---- Ghosts and spikes ----

struct SpriteOut {
    @builtin(position) position: vec4<f32>,
    // A ghost: the offset from its centre over its radius; a spike: pixels along it and across it.
    @location(0) local: vec2<f32>,
    // A ghost: its irradiance at the centre; a spike: its flux per pixel of length at a pixel.
    @location(1) @interpolate(flat) color: vec3<f32>,
    // 0 a ghost, 1 a spike; a spike's spread across it (sigma) and where it starts (pixels).
    @location(2) @interpolate(flat) shape: vec4<f32>,
}

const CORNERS = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, 1.0),
);

fn nothing() -> SpriteOut {
    var out: SpriteOut;
    out.position = vec4<f32>(-2.0, -2.0, 0.0, 1.0);
    out.local = vec2<f32>(0.0);
    out.color = vec3<f32>(0.0);
    out.shape = vec4<f32>(0.0);
    return out;
}

// The aperture's edge at `angle` from its first vertex (a unit circumradius): the polygon's, blended toward the
// circle by the iris's openness.
fn aperture_edge(angle: f32) -> f32 {
    let n = params.lens.x;
    let polygon = params.lens.z;
    if (n < 3.0 || polygon <= 0.0) {
        return 1.0;
    }
    let sector = TAU / n;
    let a = angle - sector * floor(angle / sector) - 0.5 * sector;
    return mix(1.0, cos(PI / n) / cos(a), polygon);
}

// The aperture's area at a unit circumradius.
fn aperture_area() -> f32 {
    let n = params.lens.x;
    if (n < 3.0) {
        return PI;
    }
    return mix(PI, 0.5 * n * sin(TAU / n), params.lens.z);
}

@vertex
fn sprite_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> SpriteOut {
    let source = sources_in[instance];
    let flux = source.flux.rgb;
    if (luminance(flux) <= 0.0) {
        return nothing();
    }
    let exposure = params.exposure.x * exp2(eye[0]);
    let cutoff = params.exposure.z;
    let sprite = vertex / 6u;
    let corner = CORNERS[vertex % 6u];
    let middle = params.frame.xy * 0.5;
    let at = source.at.xy;
    var out: SpriteOut;
    var position: vec2<f32>;
    if (sprite < 8u) {
        let ghost = params.ghosts[sprite];
        // An image of the aperture, grown by the source's own image along the line.
        let radius = ghost.y * params.lens2.y * params.lens2.x + abs(ghost.x) * source.at.z;
        let peak = flux * params.tints[sprite].rgb / (aperture_area() * radius * radius);
        if (luminance(peak) * exposure < cutoff || radius < 0.5) {
            return nothing();
        }
        let centre = middle + (at - middle) * ghost.x;
        position = centre + corner * radius * 1.15;
        out.local = corner * 1.15;
        out.color = peak;
        out.shape = vec4<f32>(0.0);
    } else {
        let spike = sprite - 8u;
        let blades = u32(params.lens.x);
        if (blades < 3u || spike >= u32(params.spike.w)) {
            return nothing();
        }
        // Along an edge's normal (an edge between vertices k and k + 1 faces π (2k + 1) / n past the first); an odd
        // count's spikes go both ways along each.
        let k = spike % blades;
        let back = select(0.0, PI, spike >= blades);
        let angle = params.lens.y + PI * (2.0 * f32(k) + 1.0) / f32(blades) + back;
        // Diffraction draws its spikes in the focal plane: a source out of focus spreads its light over its circle of
        // confusion instead (the bokeh), and its spikes fade as that grows past a pixel or two.
        var focused = 1.0;
        if (params.focus.z > 0.5) {
            let coc = abs(params.focus.x * (1.0 - params.focus.y / max(source.at.w, 1e-3)));
            focused = 1.0 / (1.0 + 0.25 * coc * coc);
        }
        let coefficient = params.spike.rgb * flux * focused;
        let sigma = max(source.at.z * 0.5, 0.8);
        let start = max(source.at.z, 1.5);
        // As far as its brightest line (the coefficient over r² across a Gaussian) stays over the cutoff.
        let reach = sqrt(luminance(coefficient) * exposure / (SQRT_TAU * sigma * cutoff));
        let longest = length(params.frame.xy) * 1.5;
        if (reach <= start) {
            return nothing();
        }
        let length_px = min(reach, longest);
        let dir = vec2<f32>(cos(angle), sin(angle));
        let side = vec2<f32>(-dir.y, dir.x);
        let along = mix(start * 0.5, length_px, corner.x * 0.5 + 0.5);
        let across = corner.y * (4.0 * sigma + 1.0);
        position = at + dir * along + side * across;
        out.local = vec2<f32>(along, across);
        out.color = coefficient;
        out.shape = vec4<f32>(1.0, sigma, start, 0.0);
    }
    out.position = vec4<f32>(position.x * params.frame.z * 2.0 - 1.0, 1.0 - position.y * params.frame.w * 2.0, 0.0, 1.0);
    return out;
}

@fragment
fn sprite_fragment(in: SpriteOut) -> @location(0) vec4<f32> {
    if (in.shape.x < 0.5) {
        // A ghost: the aperture's shape, its edge soft, a little brighter toward its rim; the glass's dispersion
        // makes the red image a little larger than the blue, fringing its edge.
        let angle = atan2(in.local.y, in.local.x) - params.lens.y;
        let q = length(in.local) / aperture_edge(angle);
        let qc = q * vec3<f32>(0.97, 1.0, 1.03);
        let body = vec3<f32>(1.0) - smoothstep(vec3<f32>(0.86), vec3<f32>(1.06), qc);
        let rim = (vec3<f32>(1.0) + 0.3 * smoothstep(vec3<f32>(0.5), vec3<f32>(1.0), qc)) / 1.12;
        return vec4<f32>(min(in.color * body * rim, vec3<f32>(SPRITE_CEILING)), 0.0);
    }
    // A spike: the flux per length over r², spread across it.
    let r = max(in.local.x, 1e-3);
    let sigma = in.shape.y;
    let across = exp(-0.5 * in.local.y * in.local.y / (sigma * sigma)) / (SQRT_TAU * sigma);
    let ramp = smoothstep(in.shape.z * 0.5, in.shape.z, r);
    return vec4<f32>(min(in.color * (across * ramp / (r * r)), vec3<f32>(SPRITE_CEILING)), 0.0);
}
