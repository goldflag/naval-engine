// Oil on the water (native only; no TypeScript source): the slick a leaking hull trails, read from the oil atlas that
// src/oil.rs lays and shaders/stamps.wgsl paints, and what a film of fuel oil does to the sea's light. The atlas joins
// the surface's group 1 at binding 16 and its rows the `Wake` uniform (naval_ocean::wake). The surface reads it only in
// its OIL variant, which it draws on frames with oil on the water (`OceanSurface::oil`).

#define_import_path naval_ocean::oil

#import naval_ocean::wake::{wake, wake_sampler, within, inside_square, square_edge, sway}
#import naval_ocean::mx_noise::mx_noise_float

@group(1) @binding(16) var oil_atlas: texture_2d<f32>;

// The atlas's tiles a side and its slots (src/oil.rs `OIL_TILES`, `OIL_SLOTS`).
const OIL_TILES: f32 = 3.0;
const OIL_SLOTS: i32 = 9;
// A texel's film on a log scale (src/oil.rs `encode`): 0 is OIL_THINNEST (m), 1 is OIL_THINNEST × e^OIL_RANGE (1 mm).
const OIL_THINNEST: f32 = 1e-8;
const OIL_RANGE: f32 = 11.512925;
// The film is read through a warp of up to OIL_WARP metres at two scales, so its outline frays and lobes along the water
// instead of following its patches' ellipses.
const OIL_WARP: f32 = 12.0;
const OIL_WARP_SCALE: f32 = 0.03;
// The film gathers in windrows along the wind (Langmuir cells' convergences: bands about OIL_ROWS metres apart, drawn out
// OIL_ROWS_LONG times as long along it) and breaks into patches about OIL_PATCH metres across: thick oil in the streaks,
// a thin film in the gaps between them (up to OIL_GAPS of its log-thickness gone, about a third of the slick), where a
// dark slick shows its faint rainbow (the Bonn code's discontinuous true colour), and elsewhere its log-thickness moved
// by OIL_PATCHINESS of itself. A pixel wider than the rows or the patches takes their mean instead of shimmering.
const OIL_ROWS: f32 = 26.0;
const OIL_ROWS_LONG: f32 = 7.0;
const OIL_PATCH: f32 = 70.0;
const OIL_GAPS: f32 = 0.6;
const OIL_PATCHINESS: f32 = 0.1;
// How much of the short waves the film stills (`wave_surface`'s `calm`; the wake's slick stills 0.75): a sheen damps
// the ripples, as a radar sees a slick, a thick film the short waves under them too.
const OIL_CALM_SHEEN: f32 = 0.6;
const OIL_CALM_THICK: f32 = 0.85;
// The share of the breaking a film stilling the waves wholly damps: fewer whitecaps on a slick.
const OIL_WHITECAPS: f32 = 0.8;

struct Oil {
    // The film's thickness (m); 0 where there is none.
    thickness: f32,
    // How much of the short waves it stills, 0–1.
    calm: f32,
}

// The oil at world point `at` (the grid point) for a pixel `footprint` metres across.
fn oil_read(at: vec2<f32>, footprint: f32) -> Oil {
    let count = min(i32(wake.oil.x), OIL_SLOTS);
    if count <= 0 || !within(at, wake.oil_reach) {
        return Oil(0.0, 0.0);
    }
    let world = at + vec2<f32>(sway(at, OIL_WARP_SCALE, 0.4) + sway(at.yx, OIL_WARP_SCALE * 2.7, 2.2) * 0.4,
        sway(at.yx, OIL_WARP_SCALE * 1.3, 1.9) + sway(at, OIL_WARP_SCALE * 3.1, 5.3) * 0.4) * OIL_WARP;
    var film = 0.0;
    for (var i = 0; i < count; i++) {
        if !within(world, wake.oil_tiles[4 * OIL_SLOTS + i]) {
            continue;
        }
        let fine = wake.oil_tiles[i];
        // The wide square everywhere it holds the slick, the fine one where it does, faded into the wide at its edge.
        var value = 0.0;
        let wide_uv = (world - wake.oil_tiles[OIL_SLOTS + i].xy) / wake.oil.z + 0.5;
        if within(world, wake.oil_tiles[3 * OIL_SLOTS + i]) && inside_square(wide_uv) {
            value = textureSampleLevel(oil_atlas, wake_sampler, (wide_uv + fine.zw) / OIL_TILES, 0.0).g
                * square_edge(wide_uv);
        }
        let fine_uv = (world - fine.xy) / wake.oil.y + 0.5;
        if within(world, wake.oil_tiles[2 * OIL_SLOTS + i]) && inside_square(fine_uv) {
            let near = textureSampleLevel(oil_atlas, wake_sampler, (fine_uv + fine.zw) / OIL_TILES, 0.0).r;
            value = mix(value, near, square_edge(fine_uv));
        }
        film = max(film, value);
    }
    if film < 0.004 {
        return Oil(0.0, 0.0);
    }
    // Windrows and patches ride the surface's drift and lie along the wind it goes with; they change slowly.
    let flow = wake.oil_flow.xy;
    let drift = length(flow);
    let downwind = select(vec2<f32>(1.0, 0.0), flow / max(drift, 1e-6), drift > 1e-4);
    let drifted = world - wake.oil_flow.zw;
    let along = dot(drifted, downwind);
    let across = dot(drifted, vec2<f32>(-downwind.y, downwind.x));
    let clock = wake.trail.y * 0.002 + 0.37;
    let rows = mx_noise_float(vec3<f32>(across / OIL_ROWS, along / (OIL_ROWS * OIL_ROWS_LONG), clock))
        * (1.0 - smoothstep(OIL_ROWS * 0.2, OIL_ROWS * 0.6, footprint));
    let patches = mx_noise_float(vec3<f32>(drifted / OIL_PATCH, clock * 0.7 + 5.3))
        * (1.0 - smoothstep(OIL_PATCH * 0.2, OIL_PATCH * 0.6, footprint));
    let pattern = rows * 0.6 + patches * 0.4;
    let gaps = (1.0 - smoothstep(-0.25, -0.05, pattern)) * OIL_GAPS;
    let shaped = clamp(film * (1.0 - gaps) * (1.0 + pattern * OIL_PATCHINESS), 0.0, 1.0);
    let calm = smoothstep(0.0, 0.08, shaped) * mix(OIL_CALM_SHEEN, OIL_CALM_THICK, smoothstep(0.35, 0.75, shaped));
    return Oil(OIL_THINNEST * exp(shaped * OIL_RANGE), calm);
}

// Light and fuel oil: its refractive index and the water's under it, the wavelengths (m) the sea's red, green and blue
// stand for, and heavy fuel oil's absorption (1/m), strongest in the blue: a film a few tens of microns thick passes a
// brown light, one past a tenth of a millimetre none.
const OIL_IOR: f32 = 1.48;
const OIL_WATER_IOR: f32 = 1.333;
const OIL_WAVELENGTHS: vec3<f32> = vec3<f32>(6.5e-7, 5.5e-7, 4.5e-7);
const OIL_ABSORPTION: vec3<f32> = vec3<f32>(1.2e4, 2.4e4, 4.5e4);
// The interference's phase spread over a channel's band of wavelengths and the film's unevenness within a pixel, as a
// share of the phase: it washes the colours out past their second or third order (half a micron), as a camera sees an
// oil film.
const OIL_BAND: f32 = 0.09;
// How much of the film's colour the sea keeps (the physics gives the film 2 to 6 % of the light in its colours where
// clean water reflects 2 %; all of it drew a crisp, saturated rainbow along the slick's rim).
const OIL_COLOUR: f32 = 0.6;
// The oil's own diffuse albedo: fresh heavy fuel oil's dark, faintly warm brown-black, under the sea's own upwelling in
// every channel but the red (a percent of red drew the slick rust-red from the air).
const OIL_ALBEDO: vec3<f32> = vec3<f32>(0.005, 0.0034, 0.0022);
const OIL_PI: f32 = 3.141592653589793;

struct OilFilm {
    // The film's reflectance over the clean water's, per channel.
    reflect: vec3<f32>,
    // The share of the light from under it that crosses it, down and back up, per channel.
    through: vec3<f32>,
}

// Airy's reflectance of a thin film for one polarisation: `top` and `bottom` the amplitude coefficients at its two
// faces, `kept` what the absorption leaves of a pass through it and `wave` the interference's cosine as it survives.
fn oil_airy(top: f32, bottom: f32, kept: vec3<f32>, wave: vec3<f32>) -> vec3<f32> {
    let cross = 2.0 * top * bottom * kept * wave;
    let second = bottom * bottom * kept * kept;
    return (top * top + second + cross) / (1.0 + top * top * second + cross);
}

// A film `thickness` metres thick seen at `cos_view` from the surface's normal. A film of none is the clean sea.
fn oil_film(thickness: f32, cos_view: f32) -> OilFilm {
    let c1 = clamp(cos_view, 0.02, 1.0);
    let sine = 1.0 - c1 * c1;
    let n2 = OIL_IOR;
    let n3 = OIL_WATER_IOR;
    let c2 = sqrt(1.0 - sine / (n2 * n2));
    let c3 = sqrt(1.0 - sine / (n3 * n3));
    // Fresnel's amplitudes at its top (air to oil) and its bottom (oil to water), and the water's alone.
    let s_top = (c1 - n2 * c2) / (c1 + n2 * c2);
    let p_top = (n2 * c1 - c2) / (n2 * c1 + c2);
    let s_bottom = (n2 * c2 - n3 * c3) / (n2 * c2 + n3 * c3);
    let p_bottom = (n3 * c2 - n2 * c3) / (n3 * c2 + n2 * c3);
    let s_bare = (c1 - n3 * c3) / (c1 + n3 * c3);
    let p_bare = (n3 * c1 - c3) / (n3 * c1 + c3);
    let bare = 0.5 * (s_bare * s_bare + p_bare * p_bare);
    let kept = exp(-OIL_ABSORPTION * (thickness / c2));
    let phase = (4.0 * OIL_PI * n2 * c2 * thickness) / OIL_WAVELENGTHS;
    let spread = phase * OIL_BAND;
    let wave = cos(phase) * exp(-0.5 * spread * spread);
    let film = 0.5 * (oil_airy(s_top, s_bottom, kept, wave) + oil_airy(p_top, p_bottom, kept, wave));
    let grey = dot(film, vec3<f32>(1.0 / 3.0));
    return OilFilm(mix(vec3<f32>(grey), film, OIL_COLOUR) / max(bare, 1e-5), kept * kept);
}

// What the film itself sends up, its dark brown, under `irradiance` (the sky's and the sun's on the water): the share
// of the light the water's own no longer crosses.
fn oil_own(film: OilFilm, irradiance: vec3<f32>) -> vec3<f32> {
    return OIL_ALBEDO * (1.0 - film.through) * irradiance * (1.0 / OIL_PI);
}
