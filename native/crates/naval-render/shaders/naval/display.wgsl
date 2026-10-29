// The display transform's color math, for the display pass and any crate that
// previews display-referred color.
// Ports three.js r185 `agxToneMapping` (node_modules/three/src/nodes/display/
// ToneMappingFunctions.js) line for line, and `displayGrade` with `DISPLAY_LOOK`
// from the web game's `DisplayTransform.ts` (exposure 0.9, contrast 1.25; its saturation 1.25 is 1 since 2026-10-04: the
// extra saturation turned the sea a postcard navy, and contrast alone reads as a photograph).
// `agx_hdr_tone_mapping` is native only: AgX for a display with headroom above SDR
// white (see there). The CPU twin that tests it is `naval_render::display::reference`.
#define_import_path naval::display

#import naval::color::luminance

// DISPLAY_LOOK in the web game's `DisplayTransform.ts`, saturation 1 since 2026-10-04.
const LOOK_EXPOSURE: f32 = 0.9;
const LOOK_CONTRAST: f32 = 1.25;
const LOOK_SATURATION: f32 = 1.0;
const MIDDLE_GRAY: f32 = 0.18;

// `displayGrade(radiance, exposure)`: exposure, then saturation and a contrast
// about middle gray that scales luminance only, ahead of AgX.
fn display_grade(radiance: vec3<f32>, exposure: f32) -> vec3<f32> {
    let exposed = radiance * (exposure * LOOK_EXPOSURE);
    let lum = max(luminance(exposed), 1e-5);
    return max(mix(vec3<f32>(lum), exposed, LOOK_SATURATION), vec3<f32>(0.0))
        * pow(lum / MIDDLE_GRAY, LOOK_CONTRAST - 1.0);
}

// How much of the night's readability grade a surface `distance` metres along the view takes (`display_pass.wgsl`,
// `scene_radiance`): all of the near world's within 1.5 km, smoothly back to the sky's by 12 km, so the distant haze
// and sea stay joined to the sky.
fn night_nearness(distance: f32) -> f32 {
    return 1.0 - smoothstep(1500.0, 12000.0, distance);
}

// three's matrices are built from columns, as WGSL's constructor is.
const LINEAR_REC2020_TO_LINEAR_SRGB: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.6605, -0.1246, -0.0182),
    vec3<f32>(-0.5876, 1.1329, -0.1006),
    vec3<f32>(-0.0728, -0.0083, 1.1187),
);
const LINEAR_SRGB_TO_LINEAR_REC2020: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.6274, 0.0691, 0.0164),
    vec3<f32>(0.3293, 0.9195, 0.0880),
    vec3<f32>(0.0433, 0.0113, 0.8956),
);
const AGX_INSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.856627153315983, 0.137318972929847, 0.11189821299995),
    vec3<f32>(0.0951212405381588, 0.761241990602591, 0.0767994186031903),
    vec3<f32>(0.0482516061458583, 0.101439036467562, 0.811302368396859),
);
const AGX_OUTSET: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.1271005818144368, -0.1413297634984383, -0.14132976349843826),
    vec3<f32>(-0.11060664309660323, 1.157823702216272, -0.11060664309660294),
    vec3<f32>(-0.016493938717834573, -0.016493938717834257, 1.2519364065950405),
);
const AGX_MIN_EV: f32 = -12.47393;
const AGX_MAX_EV: f32 = 4.026069;

// three's `agxDefaultContrastApprox`, with its grouping.
fn agx_default_contrast_approx(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * (x4 * x2) - 40.14 * (x4 * x)
        + (31.96 * x4 - 6.868 * (x2 * x) + (0.4298 * x2 + (0.1191 * x - 0.00232)));
}

// three's `agxToneMapping` up to its sigmoid, open-ended: the colour in AgX's inset
// Rec. 2020, log-encoded over AgX's range of exposures (0..1 within it, above 1 past
// its top).
fn agx_log_open(color: vec3<f32>, exposure: f32) -> vec3<f32> {
    var tone = color * exposure;
    tone = LINEAR_SRGB_TO_LINEAR_REC2020 * tone;
    tone = AGX_INSET * tone;
    tone = max(tone, vec3<f32>(1e-10));
    tone = log2(tone);
    return (tone - AGX_MIN_EV) / (AGX_MAX_EV - AGX_MIN_EV);
}

// three's `agxToneMapping` up to its sigmoid: the log encoding clamped to 0..1.
fn agx_log(color: vec3<f32>, exposure: f32) -> vec3<f32> {
    return clamp(agx_log_open(color, exposure), vec3<f32>(0.0), vec3<f32>(1.0));
}

// three's `agxToneMapping` after its sigmoid: back out of the inset, through AgX's
// 2.2 power, to display-linear sRGB (not yet clamped).
fn agx_display(sigmoid: vec3<f32>) -> vec3<f32> {
    var tone = AGX_OUTSET * sigmoid;
    tone = pow(max(vec3<f32>(0.0), tone), vec3<f32>(2.2));
    return LINEAR_REC2020_TO_LINEAR_SRGB * tone;
}

// three's `agxToneMapping(color, exposure)`: linear sRGB in, display-linear
// sRGB in 0..1 out, ready for the sRGB transfer.
fn agx_tone_mapping(color: vec3<f32>, exposure: f32) -> vec3<f32> {
    let sigmoid = agx_default_contrast_approx(agx_log(color, exposure));
    return clamp(agx_display(sigmoid), vec3<f32>(0.0), vec3<f32>(1.0));
}

// The sigmoid at the top of AgX's range (`agx_default_contrast_approx(1)`): SDR's
// brightest neutral, 0.99858^2.2 = 0.9969 of white.
const AGX_TOP: f32 = 0.99858;
// Where the HDR transform starts to leave AgX: a graded value of 1 (log2 1 = 0 in
// AgX's encoding), about 2.5 stops above middle grey and 0.59 of SDR white on screen.
// Below it the picture is AgX's whatever the headroom: sea, sky, hulls in the sun.
const HDR_KNEE: f32 = 0.7560;
// How sharply the lift leaves AgX at the knee: the lift is spread over sqrt(lift / this) of
// AgX's encoding, so it starts with the same curvature whatever the headroom (a display with
// more headroom carries the same rise further, never dimming a highlight). 20 spreads a
// headroom of 4 over 3.5 graded stops, the scene's own contrast on average (0.8 display stops
// a graded stop; the look grades 1.25 a scene stop).
const HDR_CURVATURE: f32 = 20.0;

// AgX for a display with `headroom` (its brightest over SDR white, at least 1):
// display-linear sRGB in 0..headroom out, SDR white at 1. AgX's own curve up to
// `HDR_KNEE`; above it each channel's sigmoid is lifted, smoothly from nothing at the knee
// (no kink: the lift starts flat) to `AGX_TOP · (headroom^(1/2.2) − 1)`, where AgX's top
// reaches the headroom as SDR's reaches white. The lift is spread over the graded stops above
// the knee as `HDR_CURVATURE` says: 2.3 at a headroom of 2, 3.5 (to a graded 11) at 4, 5.9
// (a graded 60, past AgX's own top, which is why the encoding is open-ended here) at 16. So
// what SDR packs into its last 0.8 stop (sun glitter, the sun's disc, fires, muzzle flashes,
// lightning) spreads over the headroom. At a headroom of 1 the lift is zero: exactly AgX.
fn agx_hdr_tone_mapping(color: vec3<f32>, exposure: f32, headroom: f32) -> vec3<f32> {
    let range = max(headroom, 1.0);
    let encoded = agx_log_open(color, exposure);
    let lift = AGX_TOP * (pow(range, 1.0 / 2.2) - 1.0);
    let span = max(sqrt(lift / HDR_CURVATURE), 1e-4);
    let sigmoid = agx_default_contrast_approx(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0)))
        + lift * smoothstep(vec3<f32>(HDR_KNEE), vec3<f32>(HDR_KNEE + span), encoded);
    return clamp(agx_display(sigmoid), vec3<f32>(0.0), vec3<f32>(range));
}
