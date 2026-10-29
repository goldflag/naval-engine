# Post: exposure, bloom, anti-aliasing and output

## Bloom and eye adaptation

After the scene passes the renderer meters the frame and blurs its highlights, then the display
pass applies both, as `DisplayTransform.build` does: `agx(displayGrade(radiance, exposure) + bloom)`
with `exposure = DisplaySettings::exposure × exp2(eye stops)`. Code: `naval-render/src/post/mod.rs`,
`shaders/post/bloom.wgsl`, `shaders/post/eye_adaptation.wgsl`, `shaders/display_pass.wgsl`. Both
are **off by default**, so other lanes' stills do not change; the harness turns them on (the web's
High preset).

**Bloom** is three r185's `BloomNode` with the game's `BLOOM` (`BloomSettings::default()`: strength
0.1, radius 0.5, threshold 2, knee 1; the web's strength was 0.3, see below): the luminosity high pass on the scene graded with the look
(bilinear, half resolution), five mips of separable Gaussian blur (kernel radii 6, 10, 14, 18, 22,
σ = radius / 3, three's unnormalised coefficients, each mip at half the last), and the composite
with `lerpBloomFactor` (radius 0.5 folds every factor to 0.6) × strength into the half-resolution
target the display samples. The strength fell from the web's 0.3 to 0.1 on 2026-10-05: the high pass
takes a highlight's whole value and the mips spread 0.9 of it as far as a tenth of the view, so into
the sun the sea's glitter laid a milky veil over every hull and aircraft standing in it and whitened
the sky about the sun; fires and flashes, far brighter than their surroundings, keep a glow at a third
of it. `Renderer::set_bloom(Some(BloomSettings::default()))`; the composite is
`Renderer::bloom_texture()`. The GPU test renders a uniform radiance and matches the bloom to what
three's chain gives a constant image (within 1 %), and the displayed pixel to AgX of graded +
bloom within one level.

**Glare about flashes (2026-10-12).** The owner asked for stronger bloom on flashes, without bringing back the veil
that cut the strength to 0.1. `Renderer::set_bloom_glares(&[BloomGlare])` names up to eight flashes on the screen
(their place, their reach as a share of the view's height, their heat from 0 to 1); the high pass makes the bloom
1 + 4·heat times as strong about each (a Gaussian of its reach) and lets flame-coloured light there (far redder than
blue: (r − b) / r from 0.45 to 0.75) bloom from 0.3 of the threshold, so a fireball at a luminance of 1 glares where
it gave nothing before. With none set the high pass is the plain one, to the bit, and the sea's glitter away from a
flash blooms as it did. The client sets them each frame from the effects' flash lights
(`BattleEffects::bloom_glares`): a muzzle flash, a strike, a burst, a magazine and the fireball's glow each leaves,
heat √(candela / 1500) up to 1 (a heavy turret's flash is at full heat for its fifth of a second, its fireball's
glow at 0.85 and falling; a strike's spark 0.25), reach half the light's own between 8 and 150 m. It is not a
setting: it follows the bloom's.

**The sun's glare (2026-10-12).** The owner found the sun "a yellow circle in the sky" one can look straight at, and
asked for a bloom about it that blinds a little, as the real one does. The disc the sky draws is dimmed for the display
(1.35 times the irradiance, where the sun's own radiance is some 14,500 times it) and grades under or just over the
bloom's threshold, so the bloom gave it nothing. `naval-render/src/post/sun_glare.rs` adds what the eye does: the light its
own cornea, lens and fundus scatter, a veil about the sun by the CIE's general disability-glare function for a young
eye (Vos 1999), `E (10 / θ³ + 5 / θ²)` at θ degrees from a source of illuminance `E`, here the sun's own irradiance at
the sea (`FlareSun::irradiance`, so a low sun's glare is weaker and redder; since 2026-10-20 without the readability
floor that holds the world's light up as the sun sets, which in thick air at 3° is forty times the sun's own:
`native/docs/lighting/daylight.md`, "Storm light"). In full that is 2.5
times the sun's irradiance 2° off and 0.06 of it at 10°, more than a display should show; the game shows half of it
(`SHARE` 0.5): the disc and about twice its radius go white, a glow falls away beyond, and a hull or a mast standing
next to the sun is veiled. It is held flat across the drawn disc.

- **What is seen of the sun.** A compute pass ahead of the bloom (`shaders/post/sun_glare.wgsl`, `measure`: 48 reads
  across the drawn disc, 16 on a ring about it, as photo mode's lens measures its sun) sets the disc's radiance over the
  sky about it against the radiance it has unobstructed (`Sky::drawn_sun`): a cloud, smoke, a hull or the horizon in
  front dims the glare by what it hides, and under an overcast there is none. Since 2026-10-20 a read more than half
  covered by a transparent (a rain streak's core, thick smoke: the scene's alpha, which the night grade's cover pass
  keeps) shows no sky: rain crossing a low sun hidden behind a deck read as the disc seen in full. The samples are laid out in angle about
  the sun and projected one by one: away from the view's axis the disc is drawn stretched, and a ring set in pixels
  would fall inside it. Eased over 0.12 s (`EASE_S`), so rigging crossing the disc does not flicker; with the clock
  stopped (a pause, photo mode) it settles at once, so it follows a camera that still moves. The state is a storage
  buffer (`exposure.sun_glare`: seen, this frame's measure,
  its samples in the frame).
- **Where it is added.** The bloom's composite (`sun_glare` in `shaders/post/bloom.wgsl`, per pixel from its ray's angle
  to the sun), in the bloom's graded units: the veil times the scene's exposure and the eye's, without the look's
  contrast. So it follows the bloom's setting (none with bloom off: the Low preset), dims as the eye closes for glare,
  and takes a look's halation as the bloom does. It is not a setting of its own.
- **Out of the frame** the disc cannot be measured: the glare keeps what was last seen of it and fades over 0.3 of the
  view's height past the edge (`MARGIN`). A capture (`snap_eye_adaptation`) takes none from a sun it cannot measure,
  so a still does not depend on the frames before it.
- **Angles, not pixels.** Through a narrow lens (the scope, binoculars) the same veil covers more of the view: a
  target a few degrees from the sun is as hard to see zoomed as not.
- **Photo mode** keeps it (it is the frame's own glow, as the bloom is), and the lens adds its flare and its own
  scatter over it (`photo.md`).
- The client sets the sun each frame (`Renderer::set_sun_glare`, the same `FlareSun` the photo lens takes): none by
  night (the moon draws no glare) or once the sun has set.

Seen 2026-10-12 with `shot.ts --battle "enterprise-cv6;;bismarck:static" --weather clear --hours 7.2|6.3|9` from a low
camera facing the sun, against master's client (`.build/native/sunglare/`): the disc white with a glow at 5°, 22° and
50° of elevation; nothing behind a cumulus deck; part of it with the disc partly behind the carrier's funnel; and
through a 100° lens with the sun 45° off the axis. The
image checks' renders differ from their references by the same amounts with and without it. Not measured: its cost (one compute dispatch of
64 invocations and a few operations per pixel of the half-resolution composite).

**Eye adaptation** began as `src/game/EyeAdaptation.ts` (historical) and adapts both ways since 2026-10-03
(`native/docs/lighting/daylight.md`, "Realism"): a compute pass meters `scene.hdr` on a 16 × 16 grid (4 × 4 reads a
cell, log2 of each cell's mean luminance), a second one weighs the cells toward the centre against
the sky's mean radiance (clamped to −8…+0.5 stops). Against the clear sky's mean (the sky mean's w)
it opens by all a view below −0.37 lacks, at most 2 stops, never lifting the view's brightest tenth
past 1.5 stops over the clear sky and fading out from day to night (by the sun's elevation where the host gives it,
`Renderer::set_sun_elevation`: fully from 2° up, not at all from −4° down; else by the clear sky's luminance,
`native/docs/lighting/daylight.md`, "Storm light"); against its own sky it closes by
what a view has over it, and by day for glare, the brightest tenth's excess over the same 1.5 stops
(into the sun: the aureole and the sea's glitter), at most 1 stop; easing with 1.5 s (opening) and 0.4 s (closing) time constants;
off, it eases back to the fixed exposure. The state is a storage buffer
(`exposure.eye_state`: stops, metered, target) the display and the bloom's high pass read, so
nothing returns to the CPU. `Renderer::set_eye_adaptation(EyeAdaptationSettings { enabled: true,
..Default::default() })`.

**Night readability** (2026-10-06) is a separate display grade, independent of the eye setting: with a night sky it
opens nearby world shadows by at most 1.25 stops and closes sky shadows by a quarter stop, fading through dusk and
with distance and leaving highlights alone. The display binds scene depth (binding 5, zero for sky) and the sky mean
(binding 6); its bind-group cache includes the depth's version for resize. Pixels seen through water (the host's
underwater probe, binding 7, `Renderer::set_underwater`) take the near world's grade whatever their depth, and so does
what smoke, spray and fire cover of a pixel (the scene's alpha, zeroed ahead of the `Transparent` phase). See
[night readability](../lighting/night.md#night-readability) for the thresholds and GPU checks.

Differences from the web, on purpose:

- **Same-frame metering.** The web meters after the frame and the display reads the result the
  next frame; here the meter runs before the display, so a frame shows its own exposure. A capture
  after a snap is right in one frame instead of two.
- **Frame-indexed steps.** The eye steps by `FrameTime::dt` only when the frame index changes, so
  rendering one frame again (a capture, `measure_frames`) never moves it: frames depend only on
  the clock.
- **Snap for captures.** `Renderer::snap_eye_adaptation()` settles the next frame at once (the
  web's `snap()`). The harness snaps on `load_scene`, on `set_camera` (a cut) and before every
  capture unless `capture {"snap": false}`; the viewer's orbit is continuous and adapts.

**The sky mean seam.** The eye meters against the sky's mean radiance, which the web reads from its
environment's roughest level straight up (`pmremTexture(env, up, 1)`), natively
`naval_sky::environment::env_radiance(up, 1.0)`. naval-render does not depend on naval-sky; the
host chooses the source with `Renderer::set_sky_mean`:

- `SkyMean::Rgb([r, g, b])`: a value the host knows (the test scene's CPU integral of its own sky,
  `TestScene::sky_mean()`).
- `SkyMean::Gpu`: a layer writes the buffer registered as `exposure.sky_mean` (`SKY_MEAN_BUFFER`,
  `vec4<f32>` storage) in its `prepare`. The harness's `sky` scene does it
  (`naval-harness/src/sky_scene.rs`, `SkyMeanLayer`): it registers naval-sky's
  `environment.wgsl` in the renderer's library under the path naval-sky's composer uses
  (`naval-sky/shaders/environment.wgsl`), builds a one-invocation compute pipeline
  (`shaders/sky_mean.wgsl`) with `compute_recipe` over the sky's environment bind group layout
  and a storage binding, and dispatches it after the sky layer's prepare. **The vertical slice
  copies this layer (or moves it into naval-sky).**
- `SkyMean::None` (the default): the eye holds, as the web does without an environment.

Measured in the harness `sky` scene (M5 Pro): sky mean (0.077, 0.140, 0.297) from the environment
at sun 20°; the default battle view meters −1.19 and keeps the fixed exposure, looking straight down
from 300 m meters −1.86 and opens 0.25 stops. The test scene's CPU sky mean is (0.094, 0.152,
0.312); with no clear-sky mean (w 0) the two-way eye opens against that, so the test scene's default view opens a
little (its reference image was re-blessed for it on 2026-10-03). `telemetry` reports it all under `exposure`.

**Pipeline helpers for other crates** (the sky lane asked for a hook to make adopting the registry
easy): `naval_render::fullscreen_recipe(layouts, vertex_entry, fragment_entry, format, blend)` and
`compute_recipe(layouts, entry)` build `PipelineRecipe`s for `RenderContext::add_pipeline`, with
groups from 0 as the caller lists them. With those, the sky's LUT, environment and dome passes move
to the registry by registering its WGSL (`ctx.library.add_source(path, text)` with the paths its
composer already uses, or `shader_file!` for hot reload from disk) and building each pipeline as
`ctx.add_pipeline(label, path, defs, compute_recipe(vec![layout], "entry"))`; they then hot-reload.
Bloom and the eye adaptation are built exactly this way.

## Debug views, exposure control and the eye's reading

For the developer panel's Render menu (`native/docs/ui-contract/settings.md`, "The Render menu"):

- **`Renderer::set_debug_view(DebugView)`** (`src/debug_view.rs`): `Off`, `Wireframe`, `Normals`, `Depth`,
  `ShadowCascades`; `Renderer::debug_views()` lists those the device offers. The three full-screen views are overlay
  pipelines (`shaders/debug_view.wgsl`, made when first chosen) drawn in a `debug view` pass after the post chain onto
  the output, before the layers' overlays, from `scene.depth` (group 1) and the camera group (the camera block with
  TAA's jitter, and the shadow uniforms). The cascades' tint multiplies the frame (`src × dst`); beyond the near and
  wide maps the land maps' tints (magenta, cyan, orange for land 1–3) replace the view maps'.
- **Wireframe twins** (`src/pipelines.rs`): a pipeline whose recipe takes `PipelineBuild::polygon_mode` into its
  primitive state may allow a line twin (`PipelineRegistry::allow_wireframe`, `RenderContext::allow_wireframe`).
  `add_scene_pipeline` allows it by itself for opaque triangles (depth-writing triangle lists and strips); the ships'
  main pass allows it in naval-ships. With the view on, `set_wireframe` starts every allowed twin on the build threads
  and `PipelineRegistry::render` answers with a twin once it is built, never waiting for one; a hot reload rebuilds a
  rebuilt pipeline's twin. A twin's module has every `vec4<f32>` fragment entry rewritten to return `WIRE_RADIANCE`
  (`flat_fragments`, on the composed naga module): shading from screen-space derivatives is NaN along a line. The rule for a new layer: a custom recipe for opaque scene geometry should take
  `build.polygon_mode` and call `allow_wireframe`. Needs `Features::POLYGON_MODE_LINE`.
- **`Renderer::set_exposure_control(ExposureControl)`** (`src/post/mod.rs`): `Auto` (the eye as its settings say),
  `Hold` (the stops stay where they are) or `Fixed(stops)`. The eye's uniforms grew `hold`, `fixed` and `fixed_stops`
  (80 bytes); metering goes on in every mode. Without a sky mean (the eye does not run) a fixed value is written to the
  state buffer directly.
- **`Renderer::request_eye_reading()` / `eye_reading()`**: the eye's state (stops, metered, target, brightest tenth)
  copied after the next frame into a 16-byte buffer and mapped without waiting, one in flight at a time; the value
  arrives a few frames later. `Session::eye_state` still reads it at once (and waits) for the harness.

Tested in `tests/debug_view.rs` (each full-screen view changes the frame and `Off` restores it bit for bit; the
wireframe's twin draws the test box as lines; a fixed +1.5 stops brightens the frame and reads back as 1.5, and `Hold`
keeps it).

## Temporal anti-aliasing

A port of `src/game/TemporalAntialiasing.ts` (historical; `naval-render/src/post/taa.rs`, `shaders/post/taa.wgsl`,
`shaders/naval/motion.wgsl`), off in the renderer by default and the client's default on Medium, High and Ultra since
2026-10-03: `Renderer::set_taa(Some(TaaSettings::default()))` (minimum weight 0.06, full replacement at 96 px/frame,
depth tolerance 5 %). Since then it also holds a paused picture, drops its history on cuts, draws pixels without
history back from the jitter, filters its current sample and sharpens lightly (`aa-tracers.md`, "Temporal AA as the
default", which supersedes the details below where they differ). Below full render scale it upscales: the resolve and
its history are at the output's size, rebuilt from the smaller jittered scene, which every client preset but Ultra
draws (`aa-tracers.md`, "Temporal upscaling").

With it on, a frame goes: the camera jittered by the web's 16 Halton (2, 3) offsets
(`Camera::uniforms_jittered`, three's `setViewOffset`) → shadows, prepare, scene → **motion pass**
→ eye adaptation and bloom → display into `display.frame` (graded, tone mapped and sRGB-encoded,
since the web resolves its display frame) → **resolve** into `taa.resolved` → **present** onto
the output (decoding sRGB first for an sRGB target). The resolve is the web's line for line: camera
motion reconstructed from the nearest depth of each 3 × 3 neighbourhood plus the surface's own
motion, a five-tap Catmull-Rom history, variance clipping (γ 0.75–1.25 by speed), luminance
weighting, and no history where the pixel is off screen, disoccluded (history alpha keeps view
depth; off by more than 5 %) or asks for none. Output alpha is view depth, negative where the pixel
took no history, which the output pass smooths with FXAA as the web does (FXAA and SMAA are ported by the aa-tracers
lane: see `native/docs/aa-tracers.md`).

**Marking pixels out of history, and moving surfaces.** The web writes a second scene target
(`motion`) from every material. Here the scene pass keeps its one color target, so no existing
pipeline changes: instead a **motion pass** after the scene draws into `scene.motion`
(`Rgba16Float`, cleared to 0) with `scene.depth` read-only, and a layer draws there in
`SceneLayer::draw_motion(&self, pass, ctx)` with pipelines from
`RenderContext::add_motion_pipeline(label, shader, MotionPipeline::new("vertex", "fragment"))`
(depth `GreaterEqual` with a small bias toward the camera, so a surface's motion lands exactly
where it is the nearest thing drawn; the web's blend, One / OneMinusSrcAlpha):

```wgsl
#import naval::motion::{surface_motion, motion_response}
// A moving hull: where this point is now and was a frame ago (world), response 0. Alpha 1 replaces.
return surface_motion(in.world, in.previous_world, 0.0);
// The sea, rain, tracers: take no history. Alpha 0 adds the response and keeps what is below.
return motion_response(1.0);
```

A layer drawn early whose transparent surfaces lie in front of later layers' (the sky's rain in front of a hull)
adds its response in `SceneLayer::draw_motion_over`, which runs after every layer's `draw_motion`.

Static geometry draws nothing in the motion pass. What motion vectors need sits beside the camera
in group 0 at binding 5: `CameraMotion { unjittered_view_projection, previous_view_projection (the
previous frame's, unjittered; this frame's without TAA), jitter (pixels) }` (`naval::motion`,
Rust `naval_render::CameraMotion`, 144 bytes). It is kept out of `naval::camera::Camera`, whose
448-byte layout other crates fill by hand (naval-ships' standalone camera). **The ocean should add a
`draw_motion` writing `motion_response(1.0)` over its surface** (the web's
`userData.temporalResponse = 1`); ships add `surface_motion` with their previous pose; the test
scene's turning box does (`box.wgsl`, `motion_vertex`). Costs the moving geometry a second,
position-only draw instead of an extra target on every pipeline.

**Determinism.** The jitter comes from the frame index and the history steps only when the frame
index changes, so rendering a frame again (a capture) resolves against the same history: shots
are identical run to run, and a capture equals the stepped frame. `reset_temporal()` drops the
history (a scene change); resizing restarts it.

Test: `cargo test -p naval-render --test all -- taa::` (a slanted edge takes sub-pixel levels one frame
lacks while flat regions keep theirs exactly; the motion pass writes a mover's motion only where it
is nearest and water's response; water takes no history; a repeated frame is identical);
`naval-harness` `tests/features.rs` checks the same in shots.

**Measured** (M5 Pro, 1920 × 1080, test scene, release, 120 frames, load average about 82, so
treat as upper bounds): shadows near 0.03 + wide 0.02, scene 0.21, eye adaptation 0.03, bloom
0.25, display 0.05, motion 0.05, TAA resolve 0.47, present 0.02 ms serial; frame total 1.2 ms
without TAA and 1.5 ms with it.

## HDR output and the output dither

Branch `goldflag/hdr-output`. Code: `naval-render` `src/post/dither.rs`, `src/post/display.rs`, `src/post/present.rs`,
`shaders/naval/output.wgsl` (`naval::output`), `shaders/naval/display.wgsl` (`agx_hdr_tone_mapping`), the post
shaders; `naval-client` `src/app/hdr.rs`, `view.rs`, `session/`; Graphics → HDR on the page
(`src/game/settings/graphicsSettings.ts`). Stills and measurements: `.build/native/look-display/` (`shots.sh`, `notes.md`).

**The output dither.** Whichever pass writes an 8- or 10-bit output (the display pass writing it directly, edge
smoothing's last stage, the upscale, a look's film pass; `naval_render::dither_levels` by format) adds up to one step of triangular blue
noise to the display-encoded value before it is stored (`naval::output::dither_encoded`): a 64 × 64 void-and-cluster
mask (`examples/blue_noise.rs`, `assets/blue_noise_64x64_r8.bin`, its 256 levels 16 texels each) remapped through the
triangular distribution's inverse (Gjøl, "Banding in Games"), offset by the golden ratio every sixtieth of a second of
the presentation clock: the same presentation time gets the same noise, so captures stay deterministic and a paused
battle's picture stands to the byte (`tests/pause_freeze.rs`). Black and white stay as they are (the noise fades out
within half a step of either). An sRGB target gets the dithered value decoded again for its hardware encoding. Float
outputs (the post chain's frames, an HDR surface, the HDR capture) are not dithered. The sky dome's own dither (the
web's ±half a level of interleaved gradient noise on its radiance, a visible diagonal lattice at ×8 contrast) is gone.

**HDR output.** `Renderer::set_headroom(h)`: a float output (`naval_render::extended_range`: `Rgba16Float`,
`Rgba32Float`) is drawn for a display that can show `h` × SDR white; 8- and 10-bit outputs always for SDR. The display
pass then takes `agx_hdr_tone_mapping`: AgX exactly up to a graded 1 (the knee: 2.5 stops over middle grey, 0.59 of
white on screen; sea, sky, sunlit hulls), and above it each channel's sigmoid lifted smoothly (no kink: the lift starts
flat) by up to `AGX_TOP · (h^(1/2.2) − 1)`, so AgX's top reaches `h` as SDR's reaches 1. The lift is spread over
`sqrt(lift / 20)` of AgX's log encoding: 2.3 graded stops at a headroom of 2, 3.5 (to a graded 11) at 4, 5.9 (a graded
60, past AgX's own top, so the encoding is open-ended there) at 16, which keeps the highlights at about the scene's own
contrast (0.8 display stops a graded stop on average; the look grades 1.25 a scene stop) and starts every headroom's
curve the same way, so more headroom never dims a highlight (tested). At a headroom of 1 the lift is zero: bit for bit
AgX. Eye adaptation and bloom are untouched (scene-referred, before it). The frame keeps the post chain's encoding, the
sRGB curve carried on above white (1.83 at a headroom of 4), so everything composited after it (the aim marks'
`ENCODE_SRGB`, the labels' `encode`, the page's gamma-space blend, the theatre window's ground) works on an HDR frame
as on an 8-bit `Unorm` one and stays at SDR white. Edge smoothing decides in a perceptual domain above white
(`naval::output::perceptual`, `2 − 1/x` over 1, the identity below): FXAA's luminance, SMAA's edges, temporal AA's
neighbourhood clip and blend weights.

The window (`naval_client::hdr`, `view.rs`): with Graphics → HDR `auto` (the default, in every preset) and a display
whose potential headroom is 1.05 or more, the surface is `Rgba16Float` in **extended sRGB** where the surface offers it
(Metal: `kCGColorSpaceExtendedSRGB`, the frame as it is), else in **linear extended sRGB** (scRGB: DX12 and Vulkan on
Windows in HDR mode), where the frame and the page are drawn into an encoded texture and `naval_render::LinearPresent`
decodes it onto the surface with SDR white at the display's SDR white level (scRGB's 1 is 80 nits; one full-screen
half-float pass). Anything else keeps the 8-bit sRGB surface: an SDR screen, Windows in SDR mode, a surface without a
float format, `off`. macOS grants EDR headroom only to a window that asks for it, so the potential headroom decides the
surface and the current one (`maximumExtendedDynamicRangeColorComponentValue`) what the frames are drawn for; on
Windows both are the display's peak over its SDR white level. The window asks the display twice a second and eases the
headroom over a quarter second, so brightness and the window's screen move it, and a move to an SDR screen (or the
setting) reconfigures the surface; scenes prepare both formats' pipelines, so a switch compiles nothing. The theatre's
live window is drawn for SDR. Telemetry's `display` block: the setting, encoding, format, colour space, what the
display reported and the headroom drawn.

**Seeing it without an HDR screen.** `capture {path, hdr}` (`shot.ts --hdr 4`) draws the frame an HDR window with that
headroom would present, onto a float target, and writes it as an OpenEXR of half floats (linear light, Rec. 709
primaries, SDR white at 1; `naval_client::hdr::write_exr`), whatever Graphics → HDR says. `measure_frames {hdr}` times
frames onto that target. `.build/native/look-display/falsecolor.py` maps an EXR's stops above white.

**Measured** (M5 Pro, release, 1600 × 900 stills, FXAA). SDR against master: within the dither everywhere (mean 0.23
to 0.43 levels, at most 2; the skies at most 11 on a few star and sea-glint pixels where the dome's dither went), the
mean unmoved (±0.005 levels). HDR at a headroom of 4 against the SDR frame below the knee: mean 0.3 to 0.5 levels, the
dither. Above white at a headroom of 4: noon glitter 3.8 % of the frame (1.8 % over twice white, peak 3.93), the night
salvo's flashes 0.14 % (peak 3.87), the burning Cleveland's fire 0.014 % (peak 3.98), the dusk sun's disc 1.25 × white
(its radiance is the web's stylised irradiance × 1.35, about a graded 5 at dusk: brightening it is the sky's), the
night lightning bolt 0.05 % (peak 3.84). GPU at 1920 × 1080 (`measure30`, master's shaders and the branch's alternated in
one binary, two rounds each, load average 11 to 19), per pass: the dither adds 0.03 to 0.05 ms to whichever pass writes
the 8-bit output (the display pass alone 0.061 → 0.105 ms, FXAA 0.169 → 0.226, temporal AA's present 0.183 → 0.236); the
perceptual domain about 0.02 (temporal AA's resolve 0.467 → 0.485); the HDR transform 0.01 (the display pass onto a
float target 0.068 → 0.079). The frame's GPU time (7.1 to 8.0 ms by edge smoothing) is the same within ±0.05 ms with
HDR on or off, and against master. `LinearPresent` (Windows) is one half-float full-screen pass, as the upscale's.

Not done: wide gamut (the output stays in sRGB primaries, negatives clamped, as SDR); PQ/HLG surfaces; photo mode's
look, grain and vignette work on SDR's range (under one, an HDR frame's highlights stop at white; the screenshot key
keeps an HDR frame up to SDR white too); Windows has not been run (no Windows machine here: the scRGB path,
`LinearPresent` and the DXGI headroom are tested offscreen and by unit tests only).

## Photo mode's post

`Renderer::set_photo(Option<PhotoPost>)` (`naval_render::photo`, `native/docs/photo.md`): a depth of field of four
passes after the eye and the bloom (half resolution, `photo dof` in the timings), whose result the display pass reads in
place of `scene.hdr`, and a finish in the display pass (exposure compensation, which the bloom takes too, a look and
the camera's roll as a turn of the frame). `None` costs nothing; `Renderer::probe_depth` reads one texel of the last
frame's depth back for photo mode's focus.

**Looks and the film pass (2026-10-09).** `Renderer::set_look(PhotoLook)` grades play too (the Settings menu's Look tab;
photo mode's look replaces it while photo mode is up). The display pass grades the look after AgX and adds its
halation (the bloom's blur, tinted red) and diffusion (a 12-sample blur of the scene, which the display pass therefore
binds as a filterable texture). The rest of the film (grain, vignette, gate weave, flicker, dust and scratches, the
autochrome mosaic) is `naval_render::film`, the chain's last pass: after edge smoothing and the upscale, at the
output's size, so temporal AA's history never holds moving grain. When a look has film, whichever pass would write the
output writes `film.frame` (`FRAME_FORMAT`, the output's size) and the film pass writes the output, dithered; Natural
with no photo grain or vignette routes the chain as before, to the byte. In play the film runs at 24 frames a second of
the frame's clock. `photo.md`, "Looks", has each look and effect. `Renderer::look_previews(looks, width)` draws the
Settings menu's Look tab from the frame last drawn (its HDR scene, bloom and eye): per look the display pass in preview
mode (the whole frame at a tile's size, `finish[3]` the frame pixels per tile pixel, four radiance samples a pixel)
into a `FRAME_FORMAT` tile, then the film pass into an `Rgba8Unorm` tile copied into one read-back buffer the client maps
later (`naval_render::look_preview`; `photo.md`, "The Look tab").

## GPU timings

Every pass the renderer records (each shadow map, each scene pass, the post passes and the
display pass) takes pass-level timestamp writes when the device has `Features::TIMESTAMP_QUERY`.
`Renderer::gpu_timer()` holds the results (`latest()`, `history()` of 240 frames); the harness asks
the adapter for the feature, reports each pass's median serial cost and span from `measure_frames`
(`gpu`), and the latest frame in `telemetry` (`gpu`). A layer times its own passes with the timer
in its `PrepareContext`:

```rust
let mut pass = ctx.encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
    label: Some("ocean field"),
    timestamp_writes: ctx.timer.compute_pass("ocean field"),   // None when timing is off
});
```

**What works on this machine** (Apple M5 Pro, Metal, wgpu 30.0.1, macOS, 2026-09-30,
`cargo run -p naval-harness --release --example timestamp_probe`, load average 90–100):

| Path | Result |
| --- | --- |
| `TIMESTAMP_QUERY` (pass-level writes) | advertised; works for render passes (color or depth only) and compute passes |
| `TIMESTAMP_QUERY_INSIDE_ENCODERS` (`encoder.write_timestamp`) | not advertised |
| `TIMESTAMP_QUERY_INSIDE_PASSES` (`pass.write_timestamp`) | not advertised |
| Timestamp period | 1 ns (`queue.get_timestamp_period()`); a 2048² draw measured this way agrees with wall time |
| Resolving in the **same** submission as the passes | unreliable: the last pass's end reads back **zero**, or a **stale value from an earlier frame** (an end before its begin), and depth-only passes always do. This is the "zero inside draw passes" the ocean and sky lanes saw |
| Resolving in a **later** submission, after the frame finished | every sample valid, depth-only passes included (24 passes in one encoder, all good) |

The cause is that Metal does not hazard-track the counter sample buffer, so the resolve blit can
run before the end-of-fragment sample is written. The timer therefore keeps a ring of four query
sets: a frame's set waits until `queue.on_submitted_work_done` says the frame finished, is resolved
in the next frame's encoder, and is mapped once that submission finishes. Results arrive two or
three frames late and nothing ever blocks; on native the timer polls the device without waiting
(`PollType::Poll`), on the web the callbacks run on their own. `Renderer::drain_gpu_timings()`
moves the ring along without a frame, for a harness that has just waited for the GPU.

**Spans overlap on a tile-based GPU.** A render pass's begin timestamp is the start of its vertex
stage, which may run ahead of the previous pass's fragment work (the probe's 24th pass "took" 150 ms
because its vertex stage began with the first). Each pass therefore also gets a **serial cost**:
end − max(begin, the latest end of the passes before it). Serial costs sum to the frame's busy time
(`total`: latest end − earliest begin) and are the numbers to budget with; the span is kept for
diagnosis. Work outside passes (the scene copies, buffer and texture copies) has no timestamps of
its own: it shows in the serial cost of the pass after it. Independent passes can overlap (a
compute pass beside a render pass), and then a serial cost undercounts the overlapped one.

**Measurements** (M5 Pro, 1920 × 1080, test scene, release build, `measure_frames` of 120 frames,
load average about 100 from other agents' builds):

| Pass | Serial cost, median |
| --- | --- |
| shadow: near (2048) | 0.08 ms |
| shadow: wide (2048) | 0.06 ms |
| scene, one pass | 0.64 ms |
| display | 0.17 ms |
| frame total | 0.95 ms |
| scene with all four copies (three passes) | 0.62 + 0.24 + 0.15 = 1.02 ms: the copies and the two splits cost about 0.38 ms |

`set_graphics {"scene_copies": "all"}` (a harness debugging field) forces copies to measure them;
`Renderer::set_extra_scene_copies` is the same for any host.
