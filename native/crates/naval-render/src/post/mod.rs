//! Passes between the scene and the display: eye adaptation (the web game's `EyeAdaptation.ts`) and
//! bloom (three's `BloomNode` with `BLOOM` from the web game's `DisplayTransform.ts`). Both read
//! `scene.hdr` after the transparent phase; the display pass then multiplies radiance by the
//! eye's exposure and adds the bloom before AgX, as `DisplayTransform.build` does.

pub mod antialiasing;
pub mod display;
pub mod dither;
pub mod present;
pub mod shimmer;
pub mod sun_glare;
pub mod taa;

use bytemuck::{Pod, Zeroable};

use crate::SCENE_HDR;
use crate::pipelines::{Pipeline, PipelineId, PipelineRecipe, PipelineRegistry};
use crate::resources::Resources;
use crate::shaders::{ShaderError, ShaderFile, ShaderLibrary};
use crate::timing::GpuTimer;
use crate::upload::Uploads;

pub const BLOOM_SHADER: ShaderFile = crate::shader_file!("shaders/post/bloom.wgsl");
pub const EYE_ADAPTATION_SHADER: ShaderFile =
    crate::shader_file!("shaders/post/eye_adaptation.wgsl");

/// The bloom's targets and passes' format. `Rg11b10Ufloat` (half the bytes) was tried on 2026-10-09: the chain's
/// twelve passes each round its 5- and 6-bit mantissas down, and a constant image's bloom came out 4 % short in red and
/// green and 12 % in blue, a warmer glow.
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Blur mips (`BloomNode._nMips`) and their kernel radii (`kernelSizeArray`).
const BLOOM_MIPS: usize = 5;
const KERNEL_RADII: [f32; BLOOM_MIPS] = [6.0, 10.0, 14.0, 18.0, 22.0];
/// The blur passes' labels, horizontal and vertical per mip (named once, not per frame).
const BLUR_LABELS: [[&str; 2]; BLOOM_MIPS] = [
    ["bloom blur h0", "bloom blur v0"],
    ["bloom blur h1", "bloom blur v1"],
    ["bloom blur h2", "bloom blur v2"],
    ["bloom blur h3", "bloom blur v3"],
    ["bloom blur h4", "bloom blur v4"],
];
/// `bloomFactors`.
const BLOOM_FACTORS: [f32; BLOOM_MIPS] = [1.0, 0.8, 0.6, 0.4, 0.2];

/// `BLOOM` in the web game's `DisplayTransform.ts`: glow on HDR highlights (sun glints, muzzle
/// flashes, fires, tracers) above a graded luminance of 2, over a knee of 1. Its strength is a third
/// of the web's 0.3 since 2026-10-05: the high pass takes a highlight's whole value, and the five
/// mips spread 0.9 of it as far as a tenth of the view, so into the sun the sea's glitter laid a
/// milky veil over every hull and aircraft standing in it and whitened the sky about the sun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BloomSettings {
    pub strength: f32,
    pub radius: f32,
    pub threshold: f32,
    /// three's `smoothWidth`: the width of the threshold's ramp.
    pub knee: f32,
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            strength: 0.1,
            radius: 0.5,
            threshold: 2.0,
            knee: 1.0,
        }
    }
}

/// Flashes the bloom takes at once ([`BloomGlare`]).
pub const BLOOM_GLARES: usize = 8;

/// After dark, the most a pixel brings to the bloom (the graded scene's luminance, `bloom.wgsl`'s high pass; the
/// threshold is 2), coming in as the night does (`Renderer::set_night_share`); by day none. A flash's heart, its fireball
/// and what it lights stand ten and more stops over the display's white at night, more with the eye off, and their bloom
/// laid a white veil over the whole view for its first tenths of a second (Bill, 2026-10-25: "make light at night a bit
/// [less] bright so it doesnt instantly make the screen all white"); held here they glow about themselves.
pub const NIGHT_BLOOM_LIMIT: f32 = 16.0;

/// The most a pixel brings to the bloom as far as the night has come (`night`, [`Hour::night`]): none (0) by day, from a
/// hundred times [`NIGHT_BLOOM_LIMIT`] as the twilight fades down to it.
pub fn bloom_limit(night: f32) -> f32 {
    if night > 0.0 {
        NIGHT_BLOOM_LIMIT * 100f32.powf(1.0 - night.min(1.0))
    } else {
        0.0
    }
}

/// A flash on the screen (a muzzle flash, a burst, a magazine and the fireball each leaves), for the bloom's high
/// pass: about it the bloom is up to five times as strong and flame-coloured light blooms from three tenths of the
/// threshold (`bloom.wgsl`, `GLARE_GAIN` and `GLARE_THRESHOLD`), so flashes and their fireballs glare while the
/// rest of the view, the sea's glitter into the sun with it, blooms as it did. Set each frame
/// (`Renderer::set_bloom_glares`); with none the bloom is the plain one, to the bit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BloomGlare {
    /// Its place in the view: x right and y down, 0 to 1.
    pub uv: [f32; 2],
    /// Its reach as a share of the view's height.
    pub radius: f32,
    /// How hot it is, 0 to 1.
    pub heat: f32,
}

/// The eye's reference ([`EyeAdaptationSettings::reference`]): a camera's meter's. Over a view it meters, a camera
/// exposes at sunny 16 times a meter's middle grey over the view's mean luminance; the eye exposes at the game's fixed
/// exposure times 2^(reference − metered), its view's mean over the clear sky's. The two agree when the reference is
/// log2(`crate::CAMERA_EXPOSURE` × L_K / L_clear): a meter calibrated to ISO 2720's K = 12.5 reads EV 15 at ISO 100
/// from a mean of 2^15 × 12.5 / 100 = 4096 cd/m² (the calibration Frostbite's auto exposure takes over a view's average
/// luminance: Lagarde and de Rousiers 2014, "Moving Frostbite to physically based rendering"; the eye's average is a
/// log-average), 0.297 in scene units of about 13.8 kcd/m² (`native/docs/lighting/daylight.md`, "Sun and sky at the sea"); a
/// clear noon's clear-sky mean is 0.263 (marine air, the sun 70° up); a camera's exposure is 1.45 / 2.7 of the game's.
/// log2(0.537 × 0.297 / 0.263) = −0.72. A clear noon from a ship (metered −0.75 with the paints at their reflectance)
/// then opens 0.03 over the fixed exposure, where the eye opened 0.10 before the marine air (2026-10-20) and 0.40 after
/// it at the reference of −0.37, which suited the game's exposure to a clear noon from the bridge by eye.
pub const METER_REFERENCE: f32 = -0.72;

/// A clear noon's clear-sky mean (the scene's luminance; marine air, the sun 70° up), against which [`METER_REFERENCE`]
/// was set. After dark the eye's ceiling on the brightest tenth of a view is [`EyeAdaptationSettings::night_highlight`]
/// over it: the night's lift ([`crate::lights::night_lift`]) brings the moonlit scene to the screen, and the eye closes
/// down only for a view that lights bring brighter on the screen than a stop under a clear noon's highlights
/// (`eye_adaptation.wgsl`).
pub const NOON_CLEAR_SKY: f32 = 0.2634;

/// The dark-adapted eye (`native/docs/lighting/night.md`, "The dark-adapted eye"): after dark its gain goes as Weber's law
/// with the rods' own dark light, 1 / (light + dark), so it opens by log2((from + dark) / (light + dark)) stops over the
/// fixed exposure ([`night_open`]): nothing while the night's light on the sea is [`NIGHT_ADAPT_FROM`] lux or more, as
/// much again for each stop it falls while it stands well over [`NIGHT_DARK_LIGHT`], and less and less as it falls
/// toward it, to log2((from + dark) / dark) in the dark. Under cloud it opens further by [`NIGHT_CLOUDS`] of each stop
/// the clouds take of the sky's light. At most [`NIGHT_OPEN`] stops.
pub const NIGHT_ADAPT_FROM: f32 = 0.394;
pub const NIGHT_DARK_LIGHT: f32 = 0.041;
pub const NIGHT_CLOUDS: f32 = 0.33;
pub const NIGHT_OPEN: f32 = 8.0;
/// The least a dark view is shown at after dark (log2 of the scene's luminance, `EyeAdaptationSettings::night_view`): the
/// eye opens at least until the view's centre-weighted mean stands there, as an eye looking into a dark view adapts to
/// it. A floor, set so a moonless night under a heavy deck and rain (Cape St George) shows its sea, wakes and hulls.
pub const NIGHT_VIEW: f32 = -7.3;
/// The darkest a view counts against its sky for the night's view floor (stops, `EyeAdaptationSettings::night_view_under`):
/// a stop and more under a view at the horizon (the chase view's 1.6, a broadside's 1.8 under a moonless sky). A view of
/// the sea from above, with no sky in it, opened the eye some 2.6 stops past the chase view's and drew the ship looked down
/// on as at dusk (Bill, 2026-10-25); the night's light is the same whichever way the eye looks. Two stops, a horizon view's,
/// left that view near black; at three it stands a stop over the chase view's.
pub const NIGHT_VIEW_UNDER: f32 = -3.0;
/// The time constant (s) at which the eye opens after dark (`EyeAdaptationSettings::night_recover`), its gain as by day:
/// after a salvo's glare, back to the night's opening, 12 stops closed coming to within two thirds of a stop of a
/// moonless night's opening in half a second. In life the rods take minutes to come back from a flash; that left a night
/// fight unseen for seconds after each salvo, at Bill's word (2026-10-25: "not great for gameplay"). It was the cones'
/// `brighten` back to the fixed exposure, then the rods' 6 s past it (`NIGHT_BRIGHTEN`, 2026-10-21 to 25).
pub const NIGHT_RECOVER: f32 = 0.5;
/// The night's opening stops this many stops under the ceiling the lights keep to (`EyeAdaptationSettings::night_lit`):
/// a view a lamp or a fire lights is seen by that light, not opened further for the dark about it.
pub const NIGHT_LIT: f32 = 3.0;

/// Stops the dark-adapted eye opens by for the night's light at the sea (`lux`: the sun's, the moon's beam and the sky's
/// light, in real lux), by Weber's law with the dark light (`from`, `dark`, at most `cap`).
pub fn night_open_by(lux: f32, from: f32, dark: f32, cap: f32) -> f32 {
    ((from + dark) / (lux.max(0.0) + dark).max(1e-9))
        .log2()
        .clamp(0.0, cap)
}

/// [`night_open_by`] at the game's dark adaptation ([`NIGHT_ADAPT_FROM`], [`NIGHT_DARK_LIGHT`], [`NIGHT_OPEN`]).
pub fn night_open(lux: f32) -> f32 {
    night_open_by(lux, NIGHT_ADAPT_FROM, NIGHT_DARK_LIGHT, NIGHT_OPEN)
}

/// The eye adaptation's constants: a camera's auto exposure, metering the view against the mean the clear sky would
/// have at this hour (`eye_adaptation.wgsl`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeAdaptationSettings {
    /// Adapt (the Graphics setting, and the camera above water). Off, the eye eases back to the
    /// fixed exposure.
    pub enabled: bool,
    /// Frame brightness (log2 of its centre-weighted mean over the clear sky's mean radiance) the fixed exposure
    /// suits: a camera's meter's middle grey at sunny 16, against a clear noon's sky ([`METER_REFERENCE`]).
    pub reference: f32,
    /// Share of a darker view's shortfall from `reference` the eye makes up by opening.
    pub strength: f32,
    /// Share of a view's brightness over its own sky, beyond `close_above` stops, the eye closes down for (sunlit cloud
    /// tops from above); a view of the sky itself, or into a sunset, keeps the fixed exposure.
    pub close: f32,
    pub close_above: f32,
    /// The ceiling (stops over the clear sky's mean) the eye never lifts the view's brightest tenth past: a sunlit hull
    /// filling half a view of dark sea, or an overcast sky, keeps it from opening into a glare.
    pub highlight: f32,
    /// After dark the ceiling's stops over a clear noon's clear-sky mean ([`NOON_CLEAR_SKY`]): a stop under a noon's
    /// highlights, coming in as the night does ([`Hour::night`]; through dusk `highlight`). The lights that bring a view there (a salvo's
    /// flash and the smoke it lights, a star shell's glow filling the view) show a stop darker than a noon's highlights,
    /// at Bill's word (2026-10-21: "make shots at night a bit less bright"); it was `highlight` (2026-10-20 to 21).
    pub night_highlight: f32,
    /// The dark-adapted eye's light from which it opens and the rods' dark light (lux), the stops it opens further for
    /// each the clouds take of the sky's light, the most it opens ([`NIGHT_ADAPT_FROM`], [`NIGHT_DARK_LIGHT`],
    /// [`NIGHT_CLOUDS`], [`NIGHT_OPEN`]), and how far under the ceiling a view's brightest tenth stops its opening
    /// ([`NIGHT_LIT`]).
    pub night_from: f32,
    pub night_dark: f32,
    pub night_clouds: f32,
    pub night_cap: f32,
    pub night_lit: f32,
    /// The least a dark view is shown at after dark ([`NIGHT_VIEW`]), and the darkest a view counts against its sky for
    /// it ([`NIGHT_VIEW_UNDER`]).
    pub night_view: f32,
    pub night_view_under: f32,
    /// After dark, the time constant (s) at which the eye opens ([`NIGHT_RECOVER`]).
    pub night_recover: f32,
    /// With `enabled` off, the eye keeps the night's opening for the view, never metering it by day or closing under the
    /// fixed exposure for glare: the Graphics setting off above water. Off too, the fixed exposure.
    pub dark_adapt: bool,
    /// The clear sky's log2 mean luminance from which the eye may open, and at which it may open fully: it does not
    /// adapt into the dark, so dusk and night stay as dark as the scene.
    pub dark_from: f32,
    pub light_from: f32,
    /// The most it opens up, in stops.
    pub max_stops: f32,
    /// The most it closes down by day, in stops. After dark it may close by the night's lift on top
    /// ([`crate::lights::night_lift`], 15 stops: back to the day's sensitivity), as far as the eye is dark-adapted (the
    /// share it may not open by, [`eye_daylight`]).
    pub max_down: f32,
    /// Share of the brightest tenth's excess over `highlight` the eye closes down for: glare. Into the sun the aureole
    /// and the sea's glitter fill a tenth of the view 2–3.5 stops over the clear sky's mean, where a clear noon's
    /// highlights stand at about 1; the eye stops down for them as a pupil and a camera do, so the bright sky keeps its
    /// colour and what stands against it its contrast, rather than both going white in the display's shoulder at the
    /// fixed exposure. After dark the same rule closes it for lights that fill the view (a salvo, a burning ship, the
    /// air about a star shell), against a noon's highlights ([`NOON_CLEAR_SKY`]).
    pub glare: f32,
    /// Stops the brightest tenth may stand over `highlight` by day before the eye closes for glare, in a view looking away
    /// from a sun overhead ([`sun_away`]: less as the sun is lower): sunlit cloud tops from aloft. The clouds' diffuse
    /// light makes a midday top about 0.75 stop brighter than the octaves before it did (`native/docs/clouds/march.md`, "Bright
    /// tops"), and closing for all of it darkened the whole view from the air by as much (the sky above the horizon 23
    /// levels of 255 darker); this room keeps the exposure where it was (the sky within 4 levels at noon) and lets the
    /// tops alone be brighter. It goes as the sun's height because the tops' extra light does. Toward the sun the
    /// aureole and the glitter close the eye as before; after dark lights do.
    pub glare_away: f32,
    /// A metered cell counts at most this many stops above the sky's mean radiance, and at
    /// least `floor`.
    pub ceiling: f32,
    pub floor: f32,
    /// Time constants (s): opening up into a darker view, closing down into a brighter one. Closing, the luminance the
    /// eye is adapted to rises toward the view's at the cones' 0.1 s (Durand and Dorsey 2000, "Interactive tone
    /// mapping", after Adelson 1982 and Hayhoe, Benimoff and Hood 1987), so a salvo's flash dazzles for a frame or two
    /// rather than a third of a second; opening, its gain (the exposure) rises toward the view's over seconds, the
    /// recovery from a fall in light that bleaches little (Durand and Dorsey: seconds, against the tens of minutes of
    /// dark adaptation after a bleaching light). A filter on the quantity that grows takes a large step fast at first:
    /// 12 stops closed after a salvo are 6.5 a frame later, under 2 after half a second and a fifth of a stop after 3 s.
    pub brighten: f32,
    pub darken: f32,
}

impl Default for EyeAdaptationSettings {
    /// Switched off: a renderer adapts only when asked.
    fn default() -> Self {
        Self {
            enabled: false,
            reference: METER_REFERENCE,
            strength: 1.0,
            close: 1.0,
            close_above: 0.0,
            highlight: 1.5,
            night_highlight: 0.5,
            night_from: NIGHT_ADAPT_FROM,
            night_dark: NIGHT_DARK_LIGHT,
            night_clouds: NIGHT_CLOUDS,
            night_cap: NIGHT_OPEN,
            night_lit: NIGHT_LIT,
            night_view: NIGHT_VIEW,
            night_view_under: NIGHT_VIEW_UNDER,
            night_recover: NIGHT_RECOVER,
            dark_adapt: false,
            dark_from: -5.5,
            light_from: -3.0,
            max_stops: 2.0,
            max_down: 1.0,
            glare: 1.0,
            glare_away: 0.8,
            ceiling: 0.5,
            floor: -8.0,
            brighten: 1.5,
            darken: 0.1,
        }
    }
}

/// How the eye's opening and the night grade follow the sun, where the host gives its elevation
/// ([`crate::Renderer::set_sun_elevation`]). Without it both follow the clear sky's mean luminance
/// ([`EyeAdaptationSettings::dark_from`] to `light_from`, and the display's night grade over the same range), which more
/// than the sun moves: a weather's light scale and the sky's twilight lift put a storm's dusk sky under a clear night's
/// (log2 −5.5 against −5.0), so the eye stayed shut under it and the night grade came full on (2026-10-20,
/// `native/docs/lighting/daylight.md`, "Storm light").
pub mod daylight {
    /// The sun's elevations (degrees) over which the eye comes to open as far as it may: not at all from 4° under the
    /// horizon down, fully from 2° up. By day and at dusk it opens for what the clouds take, as a camera does; deep
    /// twilight and night keep the fixed exposure. The clear weather's sky, which gated it by its luminance before,
    /// let it open a twentieth at −4° and none from −6°.
    pub const EYE: [f32; 2] = [-4.0, 2.0];
    /// The sun's elevations (degrees) over which the night grade fades out: all of it from −4° down, none from 10° up.
    /// Fitted to the clear weather's sky, which faded it by its luminance: 0.06 at 10°, 0.28 at 5°, half at the dusk
    /// preset's 3° and 0.95 at −4° then (0.29, 0.5 and 1 now); its dip to 0.36 about sunset, where the sky's twilight
    /// lift brightens it, is gone (0.8 at 0° now).
    pub const NIGHT: [f32; 2] = [-4.0, 10.0];
    /// The real light on a level surface (log10 lux, [`crate::lights::twilight_lux`]) over which the dark-adapted eye's
    /// grey comes in ([`super::night_vision_weight`]): none while the twilight gives 10 lux (the sun some 5° under the
    /// horizon, past civil twilight's end at 3.4 lux), all of it from a tenth of a lux (9.4° under; a full moon gives
    /// 0.27). The blue hour keeps its colour; by moonlight the cones see next to none.
    pub const RODS: [f32; 2] = [1.0, -1.0];
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far a view looking along `forward` looks from the sun at `sun` (unit vectors, Y up), by the sun's height: 0
/// within 45° of it, from 90° off the sine of its elevation, smoothly between; 0 with the sun down. Into the sun the
/// aureole and the sea's glitter fill the view's brightest tenth; away from it that tenth is lit surfaces, sunlit cloud
/// tops from aloft above all, whose diffuse light goes as the sun's height ([`EyeAdaptationSettings::glare_away`]).
pub fn sun_away(forward: glam::Vec3, sun: glam::Vec3) -> f32 {
    (1.0 - smoothstep(0.0, std::f32::consts::FRAC_1_SQRT_2, forward.dot(sun))) * sun.y.max(0.0)
}

/// The share of its opening the eye may make with the sun `elevation` degrees up ([`daylight::EYE`]).
pub fn eye_daylight(elevation: f32) -> f32 {
    smoothstep(daylight::EYE[0], daylight::EYE[1], elevation)
}

/// The night grade's weight with the sun `elevation` degrees up ([`daylight::NIGHT`]).
pub fn night_weight(elevation: f32) -> f32 {
    1.0 - smoothstep(daylight::NIGHT[0], daylight::NIGHT[1], elevation)
}

/// The night grade's weight by the host's sun, where it gave one (the display's `night`).
pub(crate) fn night_of(sun_elevation: Option<f32>) -> Option<f32> {
    sun_elevation.map(night_weight)
}

/// How much of the dark-adapted eye's grey the display takes with the sun `elevation` degrees up ([`daylight::RODS`]):
/// the rods, which see no colour, take over from the cones as the twilight's light falls under a full moon's
/// (`display_pass.wgsl`, `night_vision`).
pub fn night_vision_weight(elevation: f32) -> f32 {
    let lux = crate::lights::twilight_lux(elevation).max(1e-9).log10();
    1.0 - smoothstep(daylight::RODS[1], daylight::RODS[0], lux)
}

/// [`night_vision_weight`] by the host's sun; none without one (the display's `night_vision`).
pub(crate) fn night_vision_of(sun_elevation: Option<f32>) -> f32 {
    sun_elevation.map_or(0.0, night_vision_weight)
}

/// Who sets the exposure the display applies over the fixed one (`exp2(stops)`, the eye's state): the eye, or the
/// developer panel (`Renderer::set_exposure_control`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum ExposureControl {
    /// The eye adaptation as the settings say (off: the fixed exposure).
    #[default]
    Auto,
    /// The eye's stops held where they are: neither metering nor easing moves them.
    Hold,
    /// The stops set (exposure compensation over the fixed exposure, `exp2(stops)`).
    Fixed(f32),
}

/// Where the eye adaptation's sky mean radiance comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SkyMean {
    /// None yet: the eye holds where it is (the web returns early without an environment).
    None,
    /// A linear RGB value the host knows (a stand-in sky, a test).
    Rgb([f32; 3]),
    /// A layer writes it on the GPU each frame, in `prepare`, into the buffer registered as
    /// [`SKY_MEAN_BUFFER`] (a `vec4<f32>` storage buffer: rgb, then anything).
    Gpu,
}

/// Registry name of the sky mean radiance buffer (`vec4<f32>`, `STORAGE | COPY_DST | COPY_SRC`).| COPY_DST`).
pub const SKY_MEAN_BUFFER: &str = "exposure.sky_mean";
/// Registry name of the eye's state buffer (`array<f32, 4>`: stops open, metered, target,
/// unused), `STORAGE | COPY_SRC`. The display's exposure is `exp2(state[0])` × the scene's.
pub const EYE_STATE_BUFFER: &str = "exposure.eye_state";

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EyeUniforms {
    reference: f32,
    strength: f32,
    max_stops: f32,
    ceiling: f32,
    floor: f32,
    brighten: f32,
    darken: f32,
    dt: f32,
    allow: f32,
    snapped: f32,
    max_down: f32,
    close: f32,
    close_above: f32,
    highlight: f32,
    dark_from: f32,
    light_from: f32,
    /// [`ExposureControl`]: 1 holds the stops, 1 in `fixed` sets them to `fixed_stops`.
    hold: f32,
    fixed: f32,
    fixed_stops: f32,
    glare: f32,
    /// [`eye_daylight`] of the host's sun, or −1 to follow the clear sky's luminance.
    daylight: f32,
    /// log2 of the night's lift ([`EyeAdaptation::night_lift`]), and of [`NOON_CLEAR_SKY`]; the night's light at the sea
    /// (lux, [`EyeAdaptation::night_light`]).
    night_reach: f32,
    noon_clear: f32,
    night_light: f32,
    /// [`EyeAdaptationSettings::night_highlight`] and how far the night has come ([`EyeAdaptation::night_share`]), and the
    /// dark adaptation's constants (`EyeAdaptationSettings::night_*`); padded to 128 bytes.
    night_highlight: f32,
    night: f32,
    night_cap: f32,
    night_lit: f32,
    night_from: f32,
    night_dark: f32,
    night_clouds: f32,
    night_view: f32,
    /// [`EyeAdaptationSettings::dark_adapt`] (1 or 0) and `night_recover`; padded to 144 bytes.
    dark_adapt: f32,
    night_recover: f32,
    /// [`EyeAdaptationSettings::glare_away`] by how far this frame's view looks from the sun ([`sun_away`]), and
    /// [`EyeAdaptationSettings::night_view_under`].
    glare_room: f32,
    night_view_under: f32,
}

/// The eye's state read back to the CPU on request ([`EyeAdaptation::request_reading`]): one small copy in flight at
/// a time, mapped without waiting, its values kept once they arrive.
#[derive(Default)]
struct EyeReading {
    wanted: bool,
    in_flight: Option<(wgpu::Buffer, std::sync::Arc<std::sync::Mutex<Option<bool>>>)>,
    /// Copied this frame, to be mapped once it is submitted.
    copied: bool,
    latest: Option<[f32; 4]>,
    /// Readings arrived so far: a caller that needs one taken after some change waits for this to pass a mark.
    serial: u64,
}

/// The eye adaptation: two compute passes after the scene, a state buffer the display reads.
pub(crate) struct EyeAdaptation {
    pub settings: EyeAdaptationSettings,
    pub sky_mean: SkyMean,
    /// The sun's elevation (degrees) the host gives, which the eye's opening and the night grade follow ([`daylight`]).
    pub sun_elevation: Option<f32>,
    /// How far this frame's view looks from the sun, by its height ([`sun_away`]: 0 toward it, with it down or with no
    /// sun given).
    pub sun_away: f32,
    /// The night's lift on real light the host gives ([`crate::lights::night_lift`], 1 by day): how far past
    /// `max_down` the dark-adapted eye may close.
    pub night_lift: f32,
    /// The night's light at the sea (lux) the dark-adapted eye opens for ([`night_open`]).
    pub night_light: f32,
    /// How far the night has come (0 by day and through dusk, toward 1 as the twilight fades), by which the ceiling goes
    /// to its night's ([`EyeAdaptationSettings::night_highlight`]).
    pub night_share: f32,
    meter: PipelineId,
    adapt: PipelineId,
    layout: wgpu::BindGroupLayout,
    cells: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    group: Option<(u64, wgpu::BindGroup)>,
    /// Settle at once on the next update.
    snap: bool,
    /// The last frame index adapted, so rendering one frame again does not advance the eye.
    last_frame: Option<u64>,
    pub control: ExposureControl,
    reading: EyeReading,
}

impl EyeAdaptation {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
        resources: &mut Resources,
    ) -> Result<Self, ShaderError> {
        library.add(EYE_ADAPTATION_SHADER)?;
        let compute = wgpu::ShaderStages::COMPUTE;
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: compute,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("eye adaptation"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: compute,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(1, false),
                storage(2, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: compute,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(4, true),
            ],
        });
        let meter = pipelines.add(
            device,
            library,
            "eye adaptation meter",
            EYE_ADAPTATION_SHADER.path,
            Vec::new(),
            compute_recipe(vec![layout.clone()], "meter"),
        )?;
        let adapt = pipelines.add(
            device,
            library,
            "eye adaptation adapt",
            EYE_ADAPTATION_SHADER.path,
            Vec::new(),
            compute_recipe(vec![layout.clone()], "adapt"),
        )?;
        let cells = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eye adaptation cells"),
            size: 256 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eye adaptation"),
            size: std::mem::size_of::<EyeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        resources.create_buffer(
            device,
            EYE_STATE_BUFFER,
            16,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            crate::Lifetime::Persistent,
        );
        resources.create_buffer(
            device,
            SKY_MEAN_BUFFER,
            16,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            crate::Lifetime::Persistent,
        );
        Ok(Self {
            settings: EyeAdaptationSettings::default(),
            sky_mean: SkyMean::None,
            sun_elevation: None,
            sun_away: 0.0,
            night_lift: 1.0,
            night_light: 0.0,
            night_share: 0.0,
            meter,
            adapt,
            layout,
            cells,
            uniforms,
            group: None,
            snap: false,
            last_frame: None,
            control: ExposureControl::Auto,
            reading: EyeReading::default(),
        })
    }

    /// Asks for the eye's state to be read back after the next frame (see [`EyeAdaptation::reading`]).
    pub(crate) fn request_reading(&mut self) {
        self.reading.wanted = true;
    }

    /// The eye's state as last read back (stops, metered, target, brightest tenth), if any arrived.
    pub(crate) fn reading(&self) -> Option<[f32; 4]> {
        self.reading.latest
    }

    /// The number of readings arrived so far, and whether one is on its way: the first reading that can see a change
    /// made now is the one after any in flight.
    pub(crate) fn reading_serial(&self) -> (u64, bool) {
        (self.reading.serial, self.reading.in_flight.is_some())
    }

    /// Takes in a read-back that has arrived, without waiting.
    pub(crate) fn poll_reading(&mut self, device: &wgpu::Device) {
        let Some((buffer, mapped)) = &self.reading.in_flight else {
            return;
        };
        let state = *mapped.lock().expect("not poisoned");
        let state = match state {
            None => {
                let _ = device.poll(wgpu::PollType::Poll);
                *mapped.lock().expect("not poisoned")
            }
            done => done,
        };
        match state {
            Some(true) => {
                if let Ok(bytes) = buffer.slice(..).get_mapped_range() {
                    let mut values = [0f32; 4];
                    for (value, chunk) in values.iter_mut().zip(bytes.as_chunks::<4>().0) {
                        *value = f32::from_le_bytes(*chunk);
                    }
                    self.reading.latest = Some(values);
                    self.reading.serial += 1;
                }
                buffer.unmap();
                self.reading.in_flight = None;
            }
            Some(false) => self.reading.in_flight = None,
            None => {}
        }
    }

    /// Copies the state for a wanted reading into this frame's encoder, after the eye's passes.
    pub(crate) fn copy_reading(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        resources: &Resources,
    ) {
        if !self.reading.wanted || self.reading.in_flight.is_some() {
            return;
        }
        let Some(state) = resources.buffer(EYE_STATE_BUFFER) else {
            return;
        };
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("eye reading"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&state.buffer, 0, &buffer, 0, 16);
        self.reading.wanted = false;
        self.reading.copied = true;
        self.reading.in_flight = Some((buffer, Default::default()));
    }

    /// After the frame's submission: starts mapping a copy taken in it.
    pub(crate) fn map_reading(&mut self) {
        if !std::mem::take(&mut self.reading.copied) {
            return;
        }
        if let Some((buffer, mapped)) = &self.reading.in_flight {
            let mapped = mapped.clone();
            buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    *mapped.lock().expect("not poisoned") = Some(result.is_ok());
                });
        }
    }

    pub(crate) fn snap(&mut self) {
        self.snap = true;
    }

    /// Resets the eye to the fixed exposure.
    pub(crate) fn reset(&mut self, queue: &wgpu::Queue, resources: &Resources) {
        if let Some(state) = resources.buffer(EYE_STATE_BUFFER) {
            queue.write_buffer(&state.buffer, 0, bytemuck::bytes_of(&[0f32; 4]));
        }
        self.last_frame = None;
    }

    /// Before the frame's passes: the sky mean from the CPU, when the host gave one.
    pub(crate) fn write_sky_mean(&self, uploads: &mut Uploads, resources: &Resources) {
        if let SkyMean::Rgb(rgb) = self.sky_mean
            && let Some(buffer) = resources.buffer(SKY_MEAN_BUFFER)
        {
            uploads.write_buffer(
                &buffer.buffer,
                0,
                bytemuck::bytes_of(&[rgb[0], rgb[1], rgb[2], 0.0]),
            );
        }
    }

    /// Meters `scene.hdr` and steps the eye by `dt` (0 when `frame` was adapted already).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        frame: u64,
        dt: f32,
    ) {
        if self.sky_mean == SkyMean::None {
            if let Some(state) = resources.buffer(EYE_STATE_BUFFER) {
                self.write_fixed(uploads, &state.buffer);
            }
            return;
        }
        let state = resources.buffer(EYE_STATE_BUFFER).expect("registered");
        let dt = if self.last_frame == Some(frame) {
            0.0
        } else {
            dt
        };
        self.last_frame = Some(frame);
        let uniforms = self.uniforms(dt, self.snap, self.sun_away);
        self.snap = false;
        uploads.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let hdr = resources.texture(SCENE_HDR).expect("registered");
        if self
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != hdr.version)
        {
            let group = self.bind_group(device, resources, &state.buffer, &self.uniforms);
            self.group = Some((hdr.version, group));
        }
        let (_, group) = self.group.as_ref().expect("just made");
        self.dispatch(
            encoder,
            pipelines,
            group,
            timer.compute_pass("eye adaptation"),
        );
    }

    /// A second view's eye ([`ViewEye`]): meters the view's `scene.hdr` (the registry holds the view's textures while it
    /// draws) into the view's own state, stepped by the scene's time since the view's last draw, `sun_away` its own
    /// camera's ([`sun_away`]). Its first draw, or one after a pause in its draws, settles at once.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_view(
        &self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        view: &mut ViewEye,
        seconds: f64,
        sun_away: f32,
    ) {
        if self.sky_mean == SkyMean::None {
            self.write_fixed(uploads, &view.state);
            return;
        }
        let since = view.stepped.map(|stepped| seconds - stepped);
        view.stepped = Some(seconds);
        let snap = since.is_none_or(|since| !(0.0..=VIEW_EYE_GAP_S).contains(&since));
        let uniforms = self.uniforms(since.unwrap_or(0.0).max(0.0) as f32, snap, sun_away);
        uploads.write_buffer(&view.uniforms, 0, bytemuck::bytes_of(&uniforms));
        let hdr = resources.texture(SCENE_HDR).expect("registered");
        if view
            .group
            .as_ref()
            .is_none_or(|(version, _)| *version != hdr.version)
        {
            let group = self.bind_group(device, resources, &view.state, &view.uniforms);
            view.group = Some((hdr.version, group));
        }
        let (_, group) = view.group.as_ref().expect("just made");
        self.dispatch(encoder, pipelines, group, None);
    }

    /// No metering, so the eye holds; a set exposure is written as it is.
    fn write_fixed(&self, uploads: &mut Uploads, state: &wgpu::Buffer) {
        if let ExposureControl::Fixed(stops) = self.control {
            uploads.write_buffer(state, 0, bytemuck::bytes_of(&stops));
        }
    }

    fn uniforms(&self, dt: f32, snap: bool, sun_away: f32) -> EyeUniforms {
        let s = self.settings;
        EyeUniforms {
            reference: s.reference,
            strength: s.strength,
            max_stops: s.max_stops,
            ceiling: s.ceiling,
            floor: s.floor,
            brighten: s.brighten,
            darken: s.darken,
            dt: dt.max(0.0),
            allow: if s.enabled { 1.0 } else { 0.0 },
            snapped: if snap { 1.0 } else { 0.0 },
            max_down: s.max_down,
            close: s.close,
            close_above: s.close_above,
            highlight: s.highlight,
            dark_from: s.dark_from,
            light_from: s.light_from,
            hold: f32::from(u8::from(self.control == ExposureControl::Hold)),
            fixed: f32::from(u8::from(matches!(self.control, ExposureControl::Fixed(_)))),
            fixed_stops: match self.control {
                ExposureControl::Fixed(stops) => stops,
                _ => 0.0,
            },
            glare: s.glare,
            daylight: self.sun_elevation.map_or(-1.0, eye_daylight),
            night_reach: self.night_lift.max(1.0).log2(),
            noon_clear: NOON_CLEAR_SKY.log2(),
            night_light: self.night_light.max(0.0),
            night_highlight: s.night_highlight,
            night: self.night_share.clamp(0.0, 1.0),
            night_cap: s.night_cap,
            night_lit: s.night_lit,
            night_from: s.night_from,
            night_dark: s.night_dark,
            night_clouds: s.night_clouds,
            night_view: s.night_view,
            dark_adapt: if s.dark_adapt { 1.0 } else { 0.0 },
            night_recover: s.night_recover,
            glare_room: s.glare_away * sun_away,
            night_view_under: s.night_view_under,
        }
    }

    fn bind_group(
        &self,
        device: &wgpu::Device,
        resources: &Resources,
        state: &wgpu::Buffer,
        uniforms: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let hdr = resources.texture(SCENE_HDR).expect("registered");
        let sky = resources.buffer(SKY_MEAN_BUFFER).expect("registered");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("eye adaptation"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hdr.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.cells.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: state.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: sky.buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn dispatch(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        group: &wgpu::BindGroup,
        timestamp_writes: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("eye adaptation"),
            timestamp_writes,
        });
        pass.set_bind_group(0, group, &[]);
        pass.set_pipeline(pipelines.compute(self.meter));
        pass.dispatch_workgroups(4, 1, 1);
        pass.set_pipeline(pipelines.compute(self.adapt));
        pass.dispatch_workgroups(1, 1, 1);
    }
}

/// A pause in a second view's draws longer than this (seconds of the scene's time) settles its eye at once when it draws
/// again, as its first draw does: what it saw before the pause is no guide.
const VIEW_EYE_GAP_S: f64 = 1.0;

/// A second view's own eye ([`crate::frame::SecondView`]): its state, which its display pass reads in place of the main
/// view's, so its exposure follows what its own camera sees. The settings, the sky's mean and the night's light are the
/// main eye's ([`EyeAdaptation::encode_view`]).
pub(crate) struct ViewEye {
    state: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    group: Option<(u64, wgpu::BindGroup)>,
    /// The scene's time it was last stepped at.
    stepped: Option<f64>,
}

impl ViewEye {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        Self {
            state: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("second view eye state"),
                size: 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("second view eye"),
                size: std::mem::size_of::<EyeUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            group: None,
            stepped: None,
        }
    }

    /// The state buffer its display pass reads (`exp2(state[0])` × the scene's exposure).
    pub(crate) fn state(&self) -> &wgpu::Buffer {
        &self.state
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BloomUniforms {
    direction: [f32; 2],
    inv_size: [f32; 2],
    kernel_radius: f32,
    threshold: f32,
    knee: f32,
    strength: f32,
    factors: [f32; 4],
    factor_last: f32,
    exposure: f32,
    /// The view's width over its height, and how many of `glares` are set.
    glare_view: [f32; 2],
    /// Each flash: its UV, its reach and its heat.
    glares: [[f32; 4]; BLOOM_GLARES],
    /// The sun's glare in the eye ([`crate::sun_glare::SunTerms`]): its direction in the camera's frame and its drawn
    /// disc's angular radius; its veil's light and 1 with a sun; the tangent of half the view across and up.
    sun: [f32; 4],
    sun_light: [f32; 4],
    sun_view: [f32; 4],
    /// x the most a pixel brings to the bloom ([`NIGHT_BLOOM_LIMIT`] after dark), 0 for no limit.
    limit: [f32; 4],
}

/// A texture of the bloom's chain and its view.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl Target {
    fn new(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BLOOM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            size,
        }
    }
}

struct BloomTargets {
    viewport: (u32, u32),
    bright: Target,
    horizontal: Vec<Target>,
    vertical: Vec<Target>,
    /// Per pass: the uniforms and bind group (high pass, 10 blurs, composite).
    groups: Vec<wgpu::BindGroup>,
    buffers: Vec<wgpu::Buffer>,
    /// `scene.hdr`'s version the high pass was bound to.
    hdr_version: u64,
}

/// The bloom chain. Its result is the first horizontal target (half resolution), which the
/// display samples.
pub(crate) struct Bloom {
    pub settings: BloomSettings,
    /// The flashes on the screen this frame (`Renderer::set_bloom_glares`).
    pub glares: Vec<BloomGlare>,
    high_pass: PipelineId,
    blur: PipelineId,
    composite: PipelineId,
    layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    targets: Option<BloomTargets>,
    /// Bumped when the targets are remade, for the display's bind group.
    pub version: u64,
}

impl Bloom {
    pub(crate) fn new(
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        pipelines: &mut PipelineRegistry,
    ) -> Result<Self, ShaderError> {
        library.add(BLOOM_SHADER)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let texture = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: fragment,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let common = [
            texture(0),
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: fragment,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: fragment,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &common,
        });
        let mut composite_entries = common.to_vec();
        composite_entries.extend((4..9).map(texture));
        // What is seen of the sun's disc (`crate::sun_glare`).
        composite_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 9,
            ..common[3]
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom composite"),
            entries: &composite_entries,
        });
        let mut add = |label: &str, entry: &'static str, layout: &wgpu::BindGroupLayout| {
            pipelines.add(
                device,
                library,
                label,
                BLOOM_SHADER.path,
                Vec::new(),
                fullscreen_recipe(vec![layout.clone()], "vertex", entry, BLOOM_FORMAT, None),
            )
        };
        let high_pass = add("bloom high pass", "high_pass", &layout)?;
        let blur = add("bloom blur", "blur", &layout)?;
        let composite = add("bloom composite", "composite", &composite_layout)?;
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self {
            settings: BloomSettings::default(),
            glares: Vec::new(),
            high_pass,
            blur,
            composite,
            layout,
            composite_layout,
            sampler,
            targets: None,
            version: 1,
        })
    }

    /// The linear clamp sampler the display samples the bloom with.
    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    /// Drops the chain's targets while the bloom is off (about 45 MB at 3840 × 1600 in `Rgba16Float`); the next
    /// frame with bloom makes them again.
    pub(crate) fn release(&mut self) {
        if self.targets.take().is_some() {
            self.version += 1;
        }
    }

    /// The composited bloom, once a frame has drawn it.
    pub(crate) fn output(&self) -> Option<&wgpu::TextureView> {
        self.targets
            .as_ref()
            .map(|targets| &targets.horizontal[0].view)
    }

    /// Remakes the chain for a new viewport: half its size, then halving (`BloomNode.setSize`).
    fn ensure(&mut self, device: &wgpu::Device, resources: &Resources, viewport: (u32, u32)) {
        let hdr = resources.texture(SCENE_HDR).expect("registered");
        let eye = resources
            .buffer(EYE_STATE_BUFFER)
            .expect("registered by the eye adaptation");
        let sun = resources
            .buffer(crate::sun_glare::SUN_GLARE_BUFFER)
            .expect("registered by the sun's glare");
        if let Some(targets) = &self.targets
            && targets.viewport == viewport
            && targets.hdr_version == hdr.version
        {
            return;
        }
        let mut size = (
            (viewport.0 as f32 * 0.5) as u32,
            (viewport.1 as f32 * 0.5) as u32,
        );
        let bright = Target::new(device, "bloom bright", size);
        let mut horizontal = Vec::with_capacity(BLOOM_MIPS);
        let mut vertical = Vec::with_capacity(BLOOM_MIPS);
        for level in 0..BLOOM_MIPS {
            horizontal.push(Target::new(device, &format!("bloom h{level}"), size));
            vertical.push(Target::new(device, &format!("bloom v{level}"), size));
            size = (size.0 / 2, size.1 / 2);
        }
        let mut buffers = Vec::new();
        let mut groups = Vec::new();
        let mut group = |layout: &wgpu::BindGroupLayout,
                         source: &wgpu::TextureView,
                         extra: &[&wgpu::TextureView],
                         seen: Option<&wgpu::Buffer>| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("bloom pass"),
                size: std::mem::size_of::<BloomUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: eye.buffer.as_entire_binding(),
                },
            ];
            for (index, view) in extra.iter().enumerate() {
                entries.push(wgpu::BindGroupEntry {
                    binding: 4 + index as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                });
            }
            if let Some(seen) = seen {
                entries.push(wgpu::BindGroupEntry {
                    binding: 9,
                    resource: seen.as_entire_binding(),
                });
            }
            groups.push(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom pass"),
                layout,
                entries: &entries,
            }));
            buffers.push(buffer);
        };
        group(&self.layout, &hdr.view, &[], None);
        for level in 0..BLOOM_MIPS {
            let input = if level == 0 {
                &bright.view
            } else {
                &vertical[level - 1].view
            };
            group(&self.layout, input, &[], None);
            group(&self.layout, &horizontal[level].view, &[], None);
        }
        let levels: Vec<&wgpu::TextureView> = vertical.iter().map(|target| &target.view).collect();
        group(
            &self.composite_layout,
            &bright.view,
            &levels,
            Some(&sun.buffer),
        );
        self.targets = Some(BloomTargets {
            viewport,
            bright,
            horizontal,
            vertical,
            groups,
            buffers,
            hdr_version: hdr.version,
        });
        self.version += 1;
    }

    /// The high pass, ten blurs and the composite, from `scene.hdr` graded with `exposure`
    /// (the scene's; the eye's is read on the GPU), and the sun's glare in the eye over it (`sun`). `night` (0 by day,
    /// toward 1 as the twilight fades) brings in [`NIGHT_BLOOM_LIMIT`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        encoder: &mut wgpu::CommandEncoder,
        pipelines: &PipelineRegistry,
        resources: &Resources,
        timer: &GpuTimer,
        exposure: f32,
        sun: crate::sun_glare::SunTerms,
        night: f32,
    ) {
        let viewport = resources.viewport();
        self.ensure(device, resources, viewport);
        let settings = self.settings;
        let targets = self.targets.as_ref().expect("just made");
        let lerp = |factor: f32| factor + (1.2 - factor - factor) * settings.radius;
        let base = BloomUniforms {
            direction: [0.0, 0.0],
            inv_size: [0.0, 0.0],
            kernel_radius: 0.0,
            threshold: settings.threshold,
            knee: settings.knee,
            strength: settings.strength,
            factors: [
                lerp(BLOOM_FACTORS[0]),
                lerp(BLOOM_FACTORS[1]),
                lerp(BLOOM_FACTORS[2]),
                lerp(BLOOM_FACTORS[3]),
            ],
            factor_last: lerp(BLOOM_FACTORS[4]),
            exposure,
            glare_view: [
                viewport.0.max(1) as f32 / viewport.1.max(1) as f32,
                self.glares.len().min(BLOOM_GLARES) as f32,
            ],
            glares: std::array::from_fn(|i| {
                self.glares
                    .get(i)
                    .map_or([0.0; 4], |g| [g.uv[0], g.uv[1], g.radius, g.heat])
            }),
            sun: sun.sun,
            sun_light: sun.light,
            sun_view: sun.view,
            limit: [bloom_limit(night), 0.0, 0.0, 0.0],
        };
        // Pass order: high pass, (horizontal, vertical) per mip, composite.
        let mut passes: Vec<(PipelineId, &Target, BloomUniforms, &str)> =
            Vec::with_capacity(2 + 2 * BLOOM_MIPS);
        passes.push((self.high_pass, &targets.bright, base, "bloom high pass"));
        for (level, (&kernel_radius, (horizontal, vertical))) in KERNEL_RADII
            .iter()
            .zip(targets.horizontal.iter().zip(&targets.vertical))
            .enumerate()
        {
            for (direction, target, label) in [
                ([1.0, 0.0], horizontal, BLUR_LABELS[level][0]),
                ([0.0, 1.0], vertical, BLUR_LABELS[level][1]),
            ] {
                let size = target.size;
                passes.push((
                    self.blur,
                    target,
                    BloomUniforms {
                        direction,
                        inv_size: [1.0 / size.0.max(1) as f32, 1.0 / size.1.max(1) as f32],
                        kernel_radius,
                        ..base
                    },
                    label,
                ));
            }
        }
        passes.push((
            self.composite,
            &targets.horizontal[0],
            base,
            "bloom composite",
        ));
        // One timestamp pair for the whole chain: twelve small passes would crowd the frame's.
        let last = passes.len() - 1;
        let chain = timer.render_pass("bloom");
        let (chain_set, chain_begin, chain_end) = match &chain {
            Some(writes) => (
                Some(writes.query_set),
                writes.beginning_of_pass_write_index,
                writes.end_of_pass_write_index,
            ),
            None => (None, None, None),
        };
        for (index, (pipeline, target, uniforms, label)) in passes.into_iter().enumerate() {
            uploads.write_buffer(&targets.buffers[index], 0, bytemuck::bytes_of(&uniforms));
            let timestamp_writes = chain_set.map(|query_set| wgpu::RenderPassTimestampWrites {
                query_set,
                beginning_of_pass_write_index: if index == 0 { chain_begin } else { None },
                end_of_pass_write_index: if index == last { chain_end } else { None },
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: timestamp_writes.filter(|writes| {
                    writes.beginning_of_pass_write_index.is_some()
                        || writes.end_of_pass_write_index.is_some()
                }),
                ..Default::default()
            });
            pass.set_pipeline(pipelines.render(pipeline));
            pass.set_bind_group(0, &targets.groups[index], &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The composite's texture, for tests and captures.
    pub(crate) fn output_texture(&self) -> Option<&wgpu::Texture> {
        self.targets
            .as_ref()
            .map(|targets| &targets.horizontal[0].texture)
    }
}

/// A full-screen pass: `vertex_entry` draws one triangle (`naval::fullscreen`), no depth, one
/// color target of `format`. Groups from 0.
pub fn fullscreen_recipe(
    bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    vertex_entry: &'static str,
    fragment_entry: &'static str,
    format: wgpu::TextureFormat,
    blend: Option<wgpu::BlendState>,
) -> PipelineRecipe {
    Box::new(move |build| {
        let groups: Vec<Option<&wgpu::BindGroupLayout>> =
            bind_group_layouts.iter().map(Some).collect();
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(build.label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            });
        Pipeline::Render(
            build
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: build.module,
                        entry_point: Some(vertex_entry),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: build.module,
                        entry_point: Some(fragment_entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: build.cache,
                }),
        )
    })
}

/// A compute pipeline at `entry` with groups from 0.
pub fn compute_recipe(
    bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    entry: &'static str,
) -> PipelineRecipe {
    Box::new(move |build| {
        let groups: Vec<Option<&wgpu::BindGroupLayout>> =
            bind_group_layouts.iter().map(Some).collect();
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(build.label),
                bind_group_layouts: &groups,
                immediate_size: 0,
            });
        Pipeline::Compute(
            build
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&layout),
                    module: build.module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: build.cache,
                }),
        )
    })
}

/// A CPU twin of the eye adaptation's `adapt` step over already metered cells, for tests: the
/// new stops open from `stops`. `clear_luminance` is the clear sky's mean (0 when unknown: the sky's own). The eye
/// follows the clear sky's luminance from day to night, as without a sun from the host ([`reference_adapt_in`]).
#[allow(clippy::too_many_arguments)]
pub fn reference_adapt(
    settings: &EyeAdaptationSettings,
    cells: &[f32; 256],
    sky_luminance: f32,
    clear_luminance: f32,
    stops: f32,
    dt: f32,
    snapped: bool,
) -> (f32, f32, f32) {
    reference_adapt_in(
        settings,
        cells,
        sky_luminance,
        clear_luminance,
        Hour::default(),
        stops,
        dt,
        snapped,
    )
}

/// What the host tells the eye of the hour ([`reference_adapt_in`]): the sun's elevation (degrees;
/// [`crate::Renderer::set_sun_elevation`]), which the eye's day and night then follow ([`daylight`]), the night's
/// lift on real light ([`crate::lights::night_lift`]; [`crate::Renderer::set_night_lift`]), and the stops the
/// night's light at the sea in lux, which the dark-adapted eye opens for ([`night_open`];
/// [`crate::Renderer::set_night_light`]), and how far the night has come (0 by day and through dusk, toward 1 as the
/// twilight fades; [`crate::Renderer::set_night_share`]), and how far the view looks from the sun ([`sun_away`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hour {
    pub elevation: Option<f32>,
    pub lift: f32,
    pub light: f32,
    pub night: f32,
    pub sun_away: f32,
}

impl Default for Hour {
    fn default() -> Self {
        Self {
            elevation: None,
            lift: 1.0,
            light: 0.0,
            night: 0.0,
            sun_away: 0.0,
        }
    }
}

/// The brightest tenth's histogram: quarter-stop bins from 16 stops under the clear sky's mean to 34 over it (a fire's
/// flames beside the eye on a moonless night stand some 25 over), as `eye_adaptation.wgsl` has them.
const BINS: usize = 200;
const BIN_STEPS: f32 = 4.0;
const BIN_FLOOR: f32 = -16.0;
/// After dark the brightest fiftieth of a view stands at most this many stops over the ceiling the brightest tenth keeps
/// to (`EyeAdaptationSettings::highlight` over a noon's clear sky): the display's white, AgX's top 4 stops over middle
/// grey, about 2 over a noon's highlights. A fire's flames (drawn some ten times brighter than the light their fire
/// gives over their area: `native/docs/lighting/night.md`, "The night eye") stand 8–9 stops over the smoke they light; kept
/// to the tenth alone, the eye left them to bloom a lit fog over the whole view.
pub const PEAK_HEADROOM: f32 = 2.0;

/// One step of the eye from `stops` toward `goal` over `dt` (s): closing, the luminance it is adapted to (2^−stops)
/// moves toward the goal's at `darken`; opening, its gain (2^stops) at `brighten`
/// ([`EyeAdaptationSettings::darken`]). `snapped` settles at once.
pub fn eye_step(
    settings: &EyeAdaptationSettings,
    stops: f32,
    goal: f32,
    dt: f32,
    snapped: bool,
) -> f32 {
    eye_step_at(settings, stops, goal, dt, snapped, 0.0)
}

/// [`eye_step`] as far as the night has come (`night`, [`Hour::night`]): after dark it opens at
/// [`EyeAdaptationSettings::night_recover`].
pub fn eye_step_at(
    settings: &EyeAdaptationSettings,
    stops: f32,
    goal: f32,
    dt: f32,
    snapped: bool,
    night: f32,
) -> f32 {
    let opening =
        settings.brighten + (settings.night_recover - settings.brighten) * night.clamp(0.0, 1.0);
    let (tau, sign) = if goal < stops {
        (settings.darken, -1.0)
    } else {
        (opening, 1.0)
    };
    let rate = if snapped {
        1.0
    } else {
        1.0 - (-dt / tau).exp()
    };
    let from = (sign * stops).exp2();
    let to = (sign * goal).exp2();
    sign * (from + (to - from) * rate).log2()
}

/// [`reference_adapt`] at the host's [`Hour`].
#[allow(clippy::too_many_arguments)]
pub fn reference_adapt_in(
    settings: &EyeAdaptationSettings,
    cells: &[f32; 256],
    sky_luminance: f32,
    clear_luminance: f32,
    hour: Hour,
    stops: f32,
    dt: f32,
    snapped: bool,
) -> (f32, f32, f32) {
    let sky = sky_luminance.max(1e-8).log2();
    let clear = if clear_luminance > 0.0 {
        clear_luminance.max(1e-8).log2()
    } else {
        sky
    };
    let (mut sum, mut weights) = (0.0f32, 0.0f32);
    for (i, cell) in cells.iter().enumerate() {
        let offset = [
            ((i % 16) as f32 + 0.5) / 16.0 - 0.5,
            ((i / 16) as f32 + 0.5) / 16.0 - 0.5,
        ];
        let weight = (-8.0 * (offset[0] * offset[0] + offset[1] * offset[1])).exp() + 0.15;
        sum += (cell - sky).clamp(settings.floor, settings.ceiling) * weight;
        weights += weight;
    }
    let own = sum / weights;
    let metered = own + sky - clear;
    let allow = if settings.enabled { 1.0 } else { 0.0 };
    let opening = settings.reference - metered;
    let closing = own - settings.close_above;
    // The brightest tenth and fiftieth as the shader finds them: the lower edges of the quarter-stop bins (`BINS` against
    // the clear sky) holding the 26th and the 5th brightest cells.
    let mut counts = [0u32; BINS];
    for cell in cells {
        let bin = ((cell - clear - BIN_FLOOR) * BIN_STEPS).floor();
        counts[bin.clamp(0.0, (BINS - 1) as f32) as usize] += 1;
    }
    let mut above = 0;
    let mut tenth = 0;
    let mut fiftieth = 0;
    for bin in (0..BINS).rev() {
        above += counts[bin];
        if above >= 5 && fiftieth == 0 {
            fiftieth = bin;
        }
        if above >= 26 {
            tenth = bin;
            break;
        }
    }
    let bright = tenth as f32 / BIN_STEPS + BIN_FLOOR;
    let peak = fiftieth as f32 / BIN_STEPS + BIN_FLOOR;
    let daylight = hour.elevation.map_or_else(
        || smoothstep(settings.dark_from, settings.light_from, clear),
        eye_daylight,
    );
    // The ceiling on the brightest tenth: `highlight` over this hour's clear sky by day, over a clear noon's after dark
    // (the night's lift has brought the moonlit scene to the screen; only lights bring a view up to a noon's highlights).
    let ceiling = settings.highlight
        + (NOON_CLEAR_SKY.log2() - clear) * (1.0 - daylight)
        + (settings.night_highlight - settings.highlight) * hour.night.clamp(0.0, 1.0);
    let glare = (bright - ceiling - settings.glare_away * hour.sun_away * daylight)
        .max((peak - ceiling - PEAK_HEADROOM) * (1.0 - daylight))
        .max(0.0)
        * settings.glare;
    let aim = if opening > 0.0 {
        (opening * settings.strength)
            .min(settings.max_stops * daylight)
            .min((ceiling - bright).max(0.0))
    } else if closing > 0.0 {
        -closing * settings.close
    } else {
        0.0
    };
    // After dark the eye is dark-adapted to the night's light (`hour.light`) and a share of what the clouds take of the
    // sky, as far as the brightest tenth stays `night_lit` under the ceiling and the fiftieth within its headroom. The
    // view floor meters a view no darker than a stop past one at the horizon (`night_view_under`).
    let clouded = (clear - sky).max(0.0) * settings.night_clouds;
    let open = (night_open_by(
        hour.light,
        settings.night_from,
        settings.night_dark,
        settings.night_cap,
    ) + clouded)
        .max(settings.night_view - (own.max(settings.night_view_under) + sky))
        .clamp(0.0, settings.night_cap);
    let night = open * hour.night.clamp(0.0, 1.0) * (1.0 - daylight);
    let lit = (ceiling - settings.night_lit - bright).min(ceiling + PEAK_HEADROOM - peak);
    let aim = aim + (lit - aim).max(0.0).min(night);
    let aim = if glare > 0.0 { aim.min(-glare) } else { aim };
    // The eye off keeps to the night's opening alone, as far as the view's lights let it: never under the fixed exposure.
    if !settings.enabled && settings.dark_adapt {
        let rest = lit.max(0.0).min(night);
        return (
            eye_step_at(settings, stops, rest, dt, snapped, hour.night),
            metered,
            rest,
        );
    }
    // The dark-adapted eye may close by the night's lift on top: back to the day's sensitivity.
    let reach = settings.max_down + hour.lift.max(1.0).log2() * (1.0 - daylight);
    let goal = aim.clamp(-reach, settings.max_stops.max(night)) * allow;
    (
        eye_step_at(settings, stops, goal, dt, snapped, hour.night),
        metered,
        goal,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The eye's reference is a camera meter's: its middle grey at sunny 16 (4096 cd/m², 0.297 scene units) at a
    /// camera's exposure against the game's, over a clear noon's clear sky (0.263).
    #[test]
    fn the_room_for_sunlit_tops_is_away_from_a_high_sun() {
        let sun = glam::Vec3::new(0.0, 65f32.to_radians().sin(), 65f32.to_radians().cos());
        // Toward the sun none; looking level away from it the sine of its height; a low sun gives less.
        assert_eq!(sun_away(sun, sun), 0.0);
        assert!(
            (sun_away(glam::Vec3::new(0.0, 0.0, -1.0), sun) - 65f32.to_radians().sin()).abs()
                < 1e-6
        );
        let low = glam::Vec3::new(0.0, 15f32.to_radians().sin(), 15f32.to_radians().cos());
        assert!(sun_away(glam::Vec3::new(1.0, 0.0, 0.0), low) < 0.3);
        assert_eq!(
            sun_away(glam::Vec3::new(0.0, 0.0, -1.0), -glam::Vec3::Y),
            0.0
        );
    }

    #[test]
    fn the_reference_is_a_camera_meters() {
        let middle_grey = 2f32.powi(15) * 12.5 / 100.0 / 13_800.0;
        let derived = (crate::CAMERA_EXPOSURE * middle_grey / 0.2634).log2();
        assert!((derived - METER_REFERENCE).abs() < 0.01, "{derived}");
        assert_eq!(EyeAdaptationSettings::default().reference, METER_REFERENCE);
    }

    #[test]
    fn the_eye_opens_against_the_clear_sky_and_closes_against_its_own() {
        let settings = EyeAdaptationSettings {
            enabled: true,
            ..Default::default()
        };
        let reference = settings.reference;
        // A view metered at the reference keeps the fixed exposure.
        let (stops, metered, goal) =
            reference_adapt(&settings, &[reference; 256], 1.0, 0.0, 0.0, 1.0, false);
        assert!((metered - reference).abs() < 1e-5);
        assert!(goal.abs() < 1e-6 && stops.abs() < 1e-6);
        // Every cell eight stops under the sky: metered −8 (the floor), the goal held at the most it opens.
        let (stops, metered, goal) =
            reference_adapt(&settings, &[-20.0; 256], 1.0, 0.0, 0.0, 0.0, true);
        assert!((metered + 8.0).abs() < 1e-5);
        assert_eq!(goal, settings.max_stops);
        assert_eq!(stops, settings.max_stops, "a snap settles at once");
        // A view as bright as its own sky (the sky itself) keeps the fixed exposure; one brighter than its sky (sunlit
        // cloud tops from above) closes by its excess, up to the ceiling a cell counts.
        let (_, _, goal) = reference_adapt(&settings, &[0.0; 256], 1.0, 0.0, 0.0, 0.0, true);
        assert_eq!(goal, 0.0);
        let (_, _, goal) = reference_adapt(&settings, &[2.0; 256], 1.0, 0.0, 0.0, 0.0, true);
        assert!((goal + settings.ceiling * settings.close).abs() < 1e-5 && goal < 0.0);
        // It opens against the clear sky: the same view meters a stop brighter against a clear sky half as bright as
        // a bright overcast deck, so the eye opens a stop × strength less for it.
        let cells = [-2.0; 256];
        let (_, own, own_goal) = reference_adapt(&settings, &cells, 1.0, 0.0, 0.0, 0.0, true);
        let (_, clear, goal) = reference_adapt(&settings, &cells, 1.0, 0.5, 0.0, 0.0, true);
        assert!((clear - own - 1.0).abs() < 1e-5);
        assert!(
            (own_goal - goal - settings.strength).abs() < 1e-5,
            "{own_goal} {goal}"
        );
        // Opening up, the gain moves toward the goal's with a 1.5 s time constant; closing, the luminance the eye is
        // adapted to with 0.1 s.
        let (stops, ..) = reference_adapt(&settings, &[-20.0; 256], 1.0, 0.0, 0.0, 1.5, false);
        let gain = 1.0 + (settings.max_stops.exp2() - 1.0) * (1.0 - (-1.0f32).exp());
        assert!((stops - gain.log2()).abs() < 1e-5, "{stops}");
        let target = reference_adapt(&settings, &[2.0; 256], 1.0, 0.0, 2.0, 0.0, true).2;
        let (stops, ..) = reference_adapt(&settings, &[2.0; 256], 1.0, 0.0, 2.0, 0.1, false);
        let adapted = 0.25 + ((-target).exp2() - 0.25) * (1.0 - (-1.0f32).exp());
        assert!((stops + adapted.log2()).abs() < 1e-5, "{stops}");
        // It never lifts the brightest tenth past `highlight`: a sunlit hull over dark sea holds it back (to the
        // quarter stop the histogram reads it to).
        let mut hull = [-3.0; 256];
        for cell in hull.iter_mut().take(40) {
            *cell = settings.highlight - 0.25;
        }
        let (_, _, goal) = reference_adapt(&settings, &hull, 1.0, 0.0, 0.0, 0.0, true);
        assert!((goal - 0.25).abs() < 1e-5, "{goal}");
        // Glare: into the sun a tenth of the view (the aureole, the glitter) stands 3.25 stops over the clear sky over
        // a sea darker than the reference; the eye stops down for the brightest tenth's excess over `highlight`, at
        // most `max_down`, where it would otherwise have opened. A tenth just over the ceiling closes it by as much.
        let mut sun = [-1.5; 256];
        for cell in sun.iter_mut().take(40) {
            *cell = 3.25;
        }
        let (_, _, goal) = reference_adapt(&settings, &sun, 1.0, 0.0, 0.0, 0.0, true);
        assert_eq!(goal, -settings.max_down);
        let mut glint = [-1.5; 256];
        for cell in glint.iter_mut().take(40) {
            *cell = settings.highlight + 0.5;
        }
        let (_, _, goal) = reference_adapt(&settings, &glint, 1.0, 0.0, 0.0, 0.0, true);
        assert!((goal + 0.5 * settings.glare).abs() < 1e-5, "{goal}");
        let no_glare = EyeAdaptationSettings {
            glare: 0.0,
            ..settings
        };
        let (_, _, goal) = reference_adapt(&no_glare, &sun, 1.0, 0.0, 0.0, 0.0, true);
        assert_eq!(
            goal, 0.0,
            "without glare the bright tenth only holds the opening back"
        );
        // It does not adapt into the dark: under a night's clear sky a dark view keeps the fixed exposure.
        let night = 2f32.powf(settings.dark_from - 0.5);
        let (_, _, goal) = reference_adapt(&settings, &[-20.0; 256], night, night, 0.0, 0.0, true);
        assert_eq!(goal, 0.0);
        // Off, the eye eases back to the fixed exposure.
        let off = EyeAdaptationSettings::default();
        let (stops, _, goal) = reference_adapt(&off, &[-20.0; 256], 1.0, 0.0, 2.0, 10.0, false);
        assert_eq!(goal, 0.0);
        assert!(stops < 1e-6);
    }

    /// With the host's sun the eye's day is the sun's, not the clear sky's luminance: a storm's dusk, whose clear sky
    /// reads as dark as a night's, opens as far as it may, and a night under a bright clear-sky mean does not open.
    #[test]
    fn with_the_hosts_sun_the_eye_opens_by_day_and_dusk_and_never_at_night() {
        let settings = EyeAdaptationSettings {
            enabled: true,
            ..Default::default()
        };
        let dark = [-20.0; 256];
        let sun = |elevation: f32| Hour {
            elevation: Some(elevation),
            lift: 1.0,
            light: 0.0,
            night: 0.0,
            sun_away: 0.0,
        };
        let storm_dusk = 2f32.powf(settings.dark_from - 0.1);
        let (_, _, by_sky) = reference_adapt_in(
            &settings,
            &dark,
            storm_dusk,
            storm_dusk,
            Hour::default(),
            0.0,
            0.0,
            true,
        );
        assert_eq!(by_sky, 0.0, "the clear sky's luminance shut it");
        let (_, _, by_sun) = reference_adapt_in(
            &settings,
            &dark,
            storm_dusk,
            storm_dusk,
            sun(3.0),
            0.0,
            0.0,
            true,
        );
        assert_eq!(by_sun, settings.max_stops);
        // Through the sunset it opens less and less; deep twilight and night not at all, whatever the sky's mean.
        let twilight = reference_adapt_in(&settings, &dark, 1.0, 1.0, sun(-1.0), 0.0, 0.0, true).2;
        assert!(
            twilight > 0.0 && twilight < settings.max_stops,
            "{twilight}"
        );
        for night in [-4.0, -6.0, -28.0] {
            let goal = reference_adapt_in(&settings, &dark, 1.0, 1.0, sun(night), 0.0, 0.0, true).2;
            assert_eq!(goal, 0.0, "{night}");
        }
        // The highlights still hold it back by day.
        let mut hull = [-3.0; 256];
        for cell in hull.iter_mut().take(40) {
            *cell = settings.highlight - 0.25;
        }
        let goal = reference_adapt_in(&settings, &hull, 1.0, 0.0, sun(30.0), 0.0, 0.0, true).2;
        assert!((goal - 0.25).abs() < 1e-5, "{goal}");
        // The night grade: all of it from deep twilight down, half at the dusk preset's 3°, none by day.
        assert_eq!(night_weight(-28.0), 1.0);
        assert_eq!(night_weight(daylight::NIGHT[0]), 1.0);
        assert!((night_weight(3.0) - 0.5).abs() < 1e-6);
        assert_eq!(night_weight(daylight::NIGHT[1]), 0.0);
        assert_eq!(night_weight(70.0), 0.0);
    }

    /// The night eye: after dark the eye that never opens closes down, as by day, when lights bring a tenth of the view
    /// over a clear noon's highlights on the screen, as far as back to the day's sensitivity; a moonlit view keeps the
    /// fixed exposure. It closes for a flash at once and opens again over seconds.
    #[test]
    fn at_night_the_eye_closes_for_lights_that_fill_the_view() {
        let settings = EyeAdaptationSettings {
            enabled: true,
            ..Default::default()
        };
        // A night as light as the eye's adaptation starts from: it keeps the fixed exposure.
        let night = Hour {
            elevation: Some(-28.0),
            lift: 16_700.0,
            light: NIGHT_ADAPT_FROM,
            night: 1.0,
            sun_away: 0.0,
        };
        // A clear night's sky (log2 −5), its moonlit views two stops under it (over the night's view floor).
        let clear = 2f32.powf(-5.0);
        let noon = NOON_CLEAR_SKY.log2() + settings.night_highlight;
        let mut moonlit = [-7.0; 256];
        for cell in moonlit.iter_mut().take(40) {
            // The moon's glitter path, a tenth of the view, two stops under a noon's highlights on the screen.
            *cell = noon - 2.0;
        }
        let quiet = reference_adapt_in(&settings, &moonlit, clear, clear, night, 0.0, 0.0, true);
        assert_eq!(quiet.2, 0.0, "a moonlit night keeps the fixed exposure");
        // A moonless night opens the dark-adapted eye by Weber's law with the dark light: a dark view all of it; one whose
        // brightest tenth (the moon's glitter, a lamp's light) stands near the ceiling only as far as that tenth stays
        // `night_lit` under it; a lamp-lit tenth over that not at all.
        let dark = Hour {
            light: 0.005,
            ..night
        };
        let full = night_open(0.005);
        assert!((3.0..3.5).contains(&full), "{full}");
        let unlit = [-9.0; 256];
        let opened = reference_adapt_in(&settings, &unlit, clear, clear, dark, 0.0, 0.0, true).2;
        assert!((opened - full).abs() < 1e-5, "{opened}");
        let glitter = reference_adapt_in(&settings, &moonlit, clear, clear, dark, 0.0, 0.0, true).2;
        assert!(
            (glitter - (2.0 - settings.night_lit).max(0.0)).abs() < 0.26,
            "{glitter}"
        );
        let mut lit = moonlit;
        for cell in lit.iter_mut().take(40) {
            *cell = noon - 0.5;
        }
        let lamp = reference_adapt_in(&settings, &lit, clear, clear, dark, 0.0, 0.0, true).2;
        assert_eq!(lamp, 0.0);
        // Under cloud it opens further by its share of what the clouds take of the sky: two stops' dimming.
        let clouded =
            reference_adapt_in(&settings, &unlit, clear / 4.0, clear, dark, 0.0, 0.0, true).2;
        assert!(
            (clouded - (full + 2.0 * NIGHT_CLOUDS)).abs() < 1e-4,
            "{clouded}"
        );
        // A night darker still (a heavy deck taking two stops of a sky at log2 −9) opens at least until a view at the
        // horizon, `NIGHT_VIEW_UNDER` under its sky, stands at the night's view floor.
        let (decked, deck_clear) = (2f32.powf(-11.0), 2f32.powf(-9.0));
        let horizon = [-11.0 + NIGHT_VIEW_UNDER; 256];
        let floored = reference_adapt_in(
            &settings, &horizon, decked, deck_clear, dark, 0.0, 0.0, true,
        )
        .2;
        assert!(
            (floored - (NIGHT_VIEW - NIGHT_VIEW_UNDER + 11.0)).abs() < 1e-4,
            "{floored}"
        );
        assert!(floored > full + 2.0 * NIGHT_CLOUDS, "{floored}");
        // Looking down on the sea from above, no sky in view, opens it no further: the night's light is the same.
        let below = [-11.0 + NIGHT_VIEW_UNDER - 3.0; 256];
        let looked_down =
            reference_adapt_in(&settings, &below, decked, deck_clear, dark, 0.0, 0.0, true).2;
        assert!((looked_down - floored).abs() < 1e-5, "{looked_down}");
        // A salvo beside the eye lights a third of the view 12 stops over a noon's highlights: it closes by as much.
        let mut salvo = moonlit;
        for cell in salvo.iter_mut().take(90) {
            *cell = noon + 12.0;
        }
        let closed = reference_adapt_in(&settings, &salvo, clear, clear, night, 0.0, 0.0, true);
        assert!((closed.2 + 12.0).abs() < 0.26, "{closed:?}");
        // Never past the day's sensitivity: the lift's 15 stops and the day's stop of glare.
        let mut sun_bright = moonlit;
        for cell in sun_bright.iter_mut().take(90) {
            *cell = noon + 20.0;
        }
        let deepest =
            reference_adapt_in(&settings, &sun_bright, clear, clear, night, 0.0, 0.0, true).2;
        assert!(
            (deepest + settings.max_down + night.lift.log2()).abs() < 1e-4,
            "{deepest}"
        );
        // A burning ship: a twenty-fifth of the view (her flames) 12 stops over a noon's highlights, the rest under
        // them. The brightest tenth keeps the eye open; after dark the brightest fiftieth closes it until the flames
        // stand at the display's white, `PEAK_HEADROOM` over the ceiling. By day it plays no part.
        let mut burning = moonlit;
        for cell in burning.iter_mut().take(10) {
            *cell = noon + 12.0;
        }
        let flames = reference_adapt_in(&settings, &burning, clear, clear, night, 0.0, 0.0, true).2;
        assert!((flames + 12.0 - PEAK_HEADROOM).abs() < 0.26, "{flames}");
        let noon_sky = NOON_CLEAR_SKY;
        let mut day_flames = [noon_sky.log2() - 1.0; 256];
        for cell in day_flames.iter_mut().take(10) {
            *cell = noon + 3.0;
        }
        let by_day_hour = Hour {
            elevation: Some(60.0),
            lift: 1.0,
            light: 0.0,
            night: 0.0,
            sun_away: 0.0,
        };
        let by_day = reference_adapt_in(
            &settings,
            &day_flames,
            noon_sky,
            noon_sky,
            by_day_hour,
            0.0,
            0.0,
            true,
        )
        .2;
        assert!(by_day >= 0.0, "{by_day}");
        // By day the glare still closes it by at most `max_down`, whatever the lift says.
        let by_day = Hour {
            elevation: Some(60.0),
            lift: 1.0,
            light: 0.0,
            night: 0.0,
            sun_away: 0.0,
        };
        let day = reference_adapt_in(&settings, &sun_bright, 0.26, 0.26, by_day, 0.0, 0.0, true).2;
        assert_eq!(day, -settings.max_down);
        // And the night's opening plays no part by day.
        let noon_view = [noon_sky.log2() - 1.0; 256];
        let quiet_day =
            reference_adapt_in(&settings, &noon_view, 0.26, 0.26, by_day, 0.0, 0.0, true).2;
        let quiet_day_shut = reference_adapt_in(
            &settings,
            &noon_view,
            0.26,
            0.26,
            Hour {
                light: 100_000.0,
                ..by_day
            },
            0.0,
            0.0,
            true,
        )
        .2;
        assert_eq!(quiet_day, quiet_day_shut);
        // Weber's law with the dark light: shut through dusk, about half a stop under a full moon (0.27 lux, the sky's 0.005
        // and the moonlit sky's), 2.6 under a quarter moon, 3.2 with none, 3.4 in the dark.
        assert_eq!(night_open(50.0), 0.0);
        assert_eq!(night_open(NIGHT_ADAPT_FROM), 0.0);
        assert!(
            (night_open(0.28) - 0.44).abs() < 0.02,
            "{}",
            night_open(0.28)
        );
        assert!(
            (night_open(0.0296) - 2.63).abs() < 0.02,
            "{}",
            night_open(0.0296)
        );
        assert!(
            (night_open(0.005) - 3.24).abs() < 0.02,
            "{}",
            night_open(0.005)
        );
        assert!((night_open(0.0) - 3.41).abs() < 0.02, "{}", night_open(0.0));
        // At 60 frames a second the first frame of the flash closes all but three stops of the twelve; after it, the
        // eye takes half of them back in a frame and all but two thirds of a stop in a second and a half.
        let frame = 1.0 / 60.0;
        // After a salvo's glare at night it comes back to the night's opening, not only to the fixed exposure: 12 stops
        // closed are within two thirds of a stop of a moonless night's in half a second, a quarter in a second; a snap goes
        // straight to the goal.
        let mut night_stops = -12.0;
        for _ in 0..30 {
            night_stops = eye_step_at(&settings, night_stops, full, frame, false, 1.0);
        }
        assert!((full - 0.7..full).contains(&night_stops), "{night_stops}");
        for _ in 0..30 {
            night_stops = eye_step_at(&settings, night_stops, full, frame, false, 1.0);
        }
        assert!((full - 0.25..full).contains(&night_stops), "{night_stops}");
        assert_eq!(eye_step_at(&settings, -5.0, 3.0, frame, true, 1.0), 3.0);
        // The eye off (the Graphics setting, above water) keeps the night's opening as the eye has it settled. A salvo
        // closes it to the fixed exposure and no further (the flash dazzles), and it is back in a second.
        let off = EyeAdaptationSettings {
            enabled: false,
            dark_adapt: true,
            ..settings
        };
        let steady = reference_adapt_in(&off, &unlit, clear, clear, dark, 0.0, 0.0, true);
        assert!((steady.0 - full).abs() < 1e-5, "{steady:?}");
        let mut off_stops = steady.0;
        for _ in 0..20 {
            let step =
                reference_adapt_in(&off, &salvo, clear, clear, dark, off_stops, frame, false);
            assert_eq!(step.2, 0.0, "{step:?}");
            off_stops = step.0;
        }
        assert!((0.0..0.2).contains(&off_stops), "{off_stops}");
        for _ in 0..60 {
            off_stops =
                reference_adapt_in(&off, &unlit, clear, clear, dark, off_stops, frame, false).0;
        }
        assert!((full - 0.25..full).contains(&off_stops), "{off_stops}");
        // Under water (and with `dark_adapt` off) it eases back to the fixed exposure; by day it keeps it.
        let under = EyeAdaptationSettings {
            dark_adapt: false,
            ..off
        };
        assert_eq!(
            reference_adapt_in(&under, &unlit, clear, clear, dark, 0.0, 0.0, true).2,
            0.0
        );
        let day_off = reference_adapt_in(&off, &noon_view, 0.26, 0.26, by_day, 0.0, 0.0, true);
        assert_eq!(day_off.2, 0.0, "{day_off:?}");
        let first = eye_step(&settings, 0.0, -12.0, frame, false);
        assert!((-10.0..-8.0).contains(&first), "{first}");
        let after = eye_step(&settings, -12.0, 0.0, frame, false);
        assert!((-7.0..-6.0).contains(&after), "{after}");
        let mut stops = -12.0;
        for _ in 0..90 {
            stops = eye_step(&settings, stops, 0.0, frame, false);
        }
        assert!((-0.8..-0.5).contains(&stops), "{stops}");
    }

    #[test]
    fn after_dark_a_pixel_brings_the_bloom_no_more_than_the_limit() {
        assert_eq!(bloom_limit(0.0), 0.0);
        assert_eq!(bloom_limit(1.0), NIGHT_BLOOM_LIMIT);
        let dusk = bloom_limit(0.5);
        assert!(
            dusk > NIGHT_BLOOM_LIMIT && dusk < 100.0 * NIGHT_BLOOM_LIMIT,
            "{dusk}"
        );
    }

    #[test]
    fn the_games_bloom_weights_every_mip_alike() {
        let settings = BloomSettings::default();
        for factor in BLOOM_FACTORS {
            let lerped = factor + (1.2 - factor - factor) * settings.radius;
            assert!(
                (lerped - 0.6).abs() < 1e-6,
                "radius 0.5 folds each factor to 0.6"
            );
        }
    }
}
