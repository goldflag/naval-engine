# The cloud march

The port of takram's clouds as the game runs it: its files and departures, its use, its tiers, the light of its
clouds, its grain and near detail, its cost and how to run it. Weather and the cloud anchors are
[weather.md](weather.md); the index is [README.md](README.md).

## What is ported, and where

| Native | takram | Notes |
| --- | --- | --- |
| `src/layers.rs` | `CloudLayer.ts`, `CloudLayers.ts`, `DensityProfile.ts`, `updateCloudLayerUniforms` | takram's four default layers: 750–1400 m (red channel, shadows), 1000–2200 m (green, shadows), 7500–8000 m (blue, density 0.003, no detail), and an empty fourth; packed into vec4s with the gaps the march strides over. Each layer also has a coverage of its own, a weighted mix of the weather's channels, a shadow weight and a slab weight (takram: one coverage, one channel, a shadow flag), so two sets of layers blend ([`Layers::lerp`]). |
| `src/weathers.rs` | (none) | The layers each weather draws ([weather](weather.md#weather)). |
| `src/quality.rs` | `qualityPresets.ts` | Low, Medium, High (takram's defaults) and Ultra onto the tiers of the same names (table below). |
| `src/cascades.rs` | `CascadedShadowMaps.ts`, `splitFrustum.ts`, `FrustumCorners.ts` (after three-csm) | Practical splits (λ 0.6), light-space fitting, texel snapping, the fade margin. |
| `src/textures.rs`, `assets/`, `crates/naval-cloud-field` | `shape.bin`, `local_weather.png` | takram's shipped textures embedded as lossless PNGs: their generators hash value noise with `sin`, which differs between GPUs, so generating them would not give takram's texels (`examples/noise_compare.rs`). |
| `src/detail_noise.rs`, `shaders/detail_noise.wgsl` | (`shape_detail.bin`, replaced) | The detail noise, the crate's own: 128³, four channels, generated at startup ("Near detail"). |
| `shaders/noise.wgsl` | `turbulence.frag`, `perlin.glsl`, `tileableNoise.glsl` | The turbulence is generated at startup and matches takram's to the bit. |
| `shaders/common.wgsl`, `field.wgsl` | `parameters.glsl`, `clouds.glsl` | Uniforms, the weather (coverage, shape-altering function, coverage filter), the media (shape, turbulence, detail, density profiles). |
| `shaders/march.wgsl`, `march_pass.wgsl` | `clouds.frag` | The primary march (perspective steps, empty-space strides, sun march + BSM, multiple-scattering octaves over a dual-lobe phase, ground bounce, sky gradient, powder, Frostbite's energy-conserving integration), shadow length, haze, aerial perspective. Broken cloud's light is the game's since 2026-10-04 ("The light of broken cloud"); slabs and lightning keep takram's octaves and powder. |
| `shaders/shadow_pass.wgsl`, `shadow_resolve.wgsl` | `shadow.frag`, `shadowResolve.frag` | BSM per cascade with structured volume sampling (front depth, mean extinction, max optical depth, its tail), and their temporal pass. |
| `shaders/resolve.wgsl` | `cloudsResolve.frag` | Temporal upscaling: one pixel of each 4 × 4 block marched per update in Bayer order; every pixel keeps the running mean of its own samples (up to 16), its history reprojected with the clouds' drift and clipped to the variance of the march about it; a pixel with few of its own leans on the march about it (the prior). |
| `shaders/composite.wgsl` | `cloudsEffect.frag` | Premultiplied clouds over the scene; a pixel holding few samples drawn through a small spatial filter. |
| `shaders/ground.wgsl` | (none) | The ground shadow map the forward-lit ships and sea read (below). |
| `shaders/hooks.wgsl` | (none) | What the sky's dome and environment bake take: the shafts' length per pixel, a short march for the bake. |
| `src/weather.rs`, `lightning.rs`, `bolt.rs`, `shaders/rain.wgsl`, `splashes.wgsl`, `bolt.wgsl` | the web game's `src/game/sky/weather` (historical) | The lightning as ported, less what lit the old clouds. The rain and the splashes keep the web's layout (nested boxes of drops wrapping about the camera, splashes scattered afresh every life over a square of sea) and are drawn anew since 2026-10-20 ("Rain as a camera records it"); the web's veil (`post.rs`, `haze.wgsl`, `post.wgsl`, historical) is gone. The splashes stand on the drawn sea since 2026-10-03, as the web's do: `Weather::set_sea_heights` takes the ocean's heights about the camera (`naval_ocean::rain_heights`, wired by naval-client's `scene.rs`; `native/docs/ocean/foam.md`, "Weather at sea"), the flat mean sea without them. |

### Where it departs from takram, and why

- **The world is a flat sea under a curved sky.** The planet frame (takram's ECEF) has its centre straight under the
  camera (`naval-sky`'s convention), so heights and the layers' spheres curve away from the camera, while every
  texture is read in world-fixed planar coordinates (east, height, north) drifted by the wind: the weather tiles every
  100 km (takram's 100 repeats over a cube-sphere face), the shape every 3.3 km, the detail every 100 m (takram's 32³
  tiled every 167 m). Clouds stay
  where they are as the camera moves.
- **The blue noise is the crate's own** (`examples/blue_noise.rs`: Ulichney's void-and-cluster, 128², animated over
  frames by golden-ratio offsets). takram ships NVIDIA's STBN, licensed for non-commercial use only. takram reads its
  STBN at the march buffer's texel, which works because each of STBN's 64 frames is a fresh pattern; one pattern
  stepped by the golden ratio would give a block's 16 cells one value, offset only by the update each is marched in,
  and print the same 4-pixel pattern into every block, so the march reads it at the drawing buffer's pixel. It steps
  by the golden ratio once per march of that pixel (`march_jitter`: one update in 16), so the samples a pixel gathers
  spread evenly over the jitter, and so does the ray's place in the pixel (`march_subpixel`, from 2026-10-28: "Fine
  grain"). Until 2026-10-03 it stepped every update over a cycle of 64, which gave a pixel only
  four jitters, all within a third of [0, 1): its samples never averaged to the clouds, and their grain stood in every
  still (`stbn`, the shadow maps' jitter, still steps so).
- **No cloud gain** (from 2026-10-03; `CLOUD_GAIN` was 1.9 on the clouds' whole radiance): the clouds' sunlight takes
  the dome's grade gain instead (`direct_scale` × `sky_grade::GAIN`), as the sun on the sea and ships does, so clouds,
  sky and world share one scale (`native/docs/lighting/daylight.md`, "Realism").
- **Reconstruction** (rewritten 2026-10-03 to take the grain out): takram replaces a pixel with each new sample, and
  until 2026-10-03 the port blended it half into the pixel's history, about three samples' worth, so one sample's
  noise stood in every still as a sandy grain. Now every pixel of the drawing buffer keeps the running mean of its own
  samples and how many it holds (the shafts' history's green channel): up to 32 (16 updates each, about eight seconds;
  16 until 2026-10-28) where the view has not moved the pixel, fewer under a drift too slow to reproject (16 at the
  least), 8 where the view has moved it, whose history is resampled every update and gathered the
  blur of those resamplings and of the clouds' evolving shapes over the longer span (against the clouds marched at 32
  jitters the shorter span is as near or nearer in every view, and keeps edges a tenth sharper).
  - **Motion**: the history is reprojected by the motion of the march pixels about it (their mean weighted by a
    quadratic B-spline of 1.5 march texels; takram's nearest fragment steps from one 4 × 4 block to the next, which
    sheared a long history wherever the motion varies over the view), and the march's motion of a cloud includes the
    wind's drift since the previous update (`CloudUniforms::drift_step`): binoculars magnify a 12 m/s wind 20 km off to
    a fifth of a pixel an update, and without it a long history smeared along the wind (against the 32-jitter clouds:
    3.65 levels from them at 2 km among cumulus, 2.27 with it).
  - **History read**: a pixel the view did not move reads its own history texel; one moved under 0.3 px, bilinearly,
    blending to takram's Catmull–Rom read (`textureCatmullRom`, which takram leaves commented out) by 0.7 px. Under a
    slow drift a Catmull–Rom read kept the noise of samples carried in from other pixels and a bias of up to a level
    and a half (the nine-tap form also printed bright single pixels), where the bilinear read is the nearest to the
    32-jitter clouds in every view measured.
  - **Clipping**: to the march about the pixel, the box's moments weighted by the same spline (the nearest march
    texel's box stepped at the blocks' edges, which binoculars printed as soft 4 × 4 blocks under a pitching ship);
    variance gamma 8 for a pixel the view did not move, whose detail clipping to the quarter-resolution
    neighbourhood would drag back toward that picture, and takram's 2 from a pixel of motion up; on the radiance, the
    opacity following (a box over all four channels has no width inside a thick cloud, whose opacity is 1 throughout);
    at least a hundredth of the mean wide. A history the box finds more than one and a half half-widths out keeps
    proportionally fewer samples, none from two and a half, so a changed cloud takes the new samples at once.
  - **The prior** (from 2026-10-04, for a view that turns): a pixel with few samples of its own leans on the march
    about it, the spline-weighted mean the clipping box is built on. It joins the pixel's mean a quarter of a sample's
    weight an update until it holds two samples' worth (`PRIOR_WEIGHT`), so a new pixel's prior is the mean of eight
    updates' marches, each on another cell of the block; each sample of the pixel's own then lowers the weight the
    prior may hold (none from four, `PRIOR_SAMPLES`), so later samples weigh the more and the prior fades out of the
    mean. The state's alpha holds its weight. Before, a pixel without a sample followed the march buffer upsampled
    bilinearly, half a new estimate every update (a 4 px blotch that changed every frame), and then took its first
    sample whole. A still view is as it was (its pixels hold 8 to 32 samples and no prior); a view that turns measured
    as in the [history](../../../docs/archive/native-clouds-history.md#how-it-looks).
- **Near cloud a travelling camera passes** (from 2026-10-05): as the camera travels, a near cloud's near part
  passes its far part and its fringe passes the sky behind it, so no one motion carries a pixel's history; flying
  100 m/s past a cumulus 300 m off showed a snow of single samples and a torn fringe. Three changes, together:
  - the march walks a cloud within 800 m of the camera in steps of 20 m (takram's 50 m and more), growing back by
    1.5 km (`march.wgsl`, `NEAR_STEP`; slabs keep their optical-depth steps). 8 and 12.5 m measured the same;
  - a pixel whose march neighbourhood moves 0.25–1.5 px an update against the view's turn alone (the parallax a
    travelling camera gives near cloud; a turn, a ship's camera and the wind's drift stay under it) takes a quarter of
    the march about it each update through a Gaussian of 1.25 march texels, and its mean keeps two samples of its own
    (`resolve.wgsl`, the sliding view): the near cloud is drawn from the march as it stands, softer and steady;
  - the composite's filter for young pixels is 2.5 px wide at one sample (was 1 px), weighs each neighbour by its
    samples and takes the prior-only pixels too, within 5 × 5 (`composite.wgsl`).

  What a travelling view still shows: a soft, slightly dappled fringe where the converged clouds show an even haze.
- **The scene's silhouette** (from 2026-10-03): the march writes where each ray met the opaque scene (the view depth,
  0 for none) beside its motion, and the reconstruction reads the opaque depth too. Where a pixel's 3 × 3 march
  neighbourhood straddles the silhouette, only the march pixels that saw the pixel's surface give its sample, its
  motion and its clipping box. takram takes them all, so a ray stopped at a ship (no cloud, the ship's motion) left a
  pale cloud-free halo a few pixels wide round masts, directors and funnels against the clouds, widest while the view
  turned. The history also keeps the view depth (km) of the surface each pixel saw (the shafts' history's blue
  channel, 0 for sky or sea), and a reprojected history that saw another surface starts again from this update's
  estimate: a hull steaming across the clouds leaves no trail (before, the stale history behind a mast stood until
  its cell came round, a pale streak up to 16 updates long).
- **On a frame that leaves time standing** (a capture, a frame drawn again, a paused picture, the first frame), the
  first update after a cut marches every pixel (`march_full_main`) into the history the composite reads, so the picture
  is whole rather than the march buffer upsampled (the 4 × 4 mosaic a shot taken the frame its camera arrives showed).
  It gives every pixel one sample, which the composite's spatial filter (below) draws without its grain. It costs
  about ten quarter-resolution marches (see "GPU cost"); in play a cut fills in over the next 16 updates, as
  takram's does, rather than spend that long frame (every pixel the prior first, then its own samples). A sky put on screen in place of another in play
  (`GameSky::start_in_play`: the theatre's scene swaps, `native/docs/theatre.md`) takes its first frame as one of play,
  so the frame that swaps the scene in does not pay the full march.
- **The spatial filter** (from 2026-10-03, `composite.wgsl`): a pixel whose mean holds between one and seven samples
  (a capture's full march, the first seconds after a cut, sky a hull has just uncovered) is drawn through a Gaussian
  over its neighbours that saw the same surface, 1 px wide for one sample and narrower as the samples gather (1/√n),
  5 × 5 or 3 × 3. The march's jitter is blue noise, whose error a few pixels across cancels: on a capture taken the
  frame its camera arrives it brings the clouds from 1.8–2.9 levels off the 32-jitter clouds to 0.7–1.5. A pixel with
  no sample of its own (the prior alone, smooth already) or with eight or more is drawn as it is, and the history
  is never filtered.
- **Precision**: rays come from the lens and the view's rotation (as takram's vertex stage) and motion from the
  camera's world position, never from points near the camera or the planet frame's heights, whose f32 steps (2.4e-4 m
  at 2.6 km, half a metre at the planet's radius) moved a still view's history a third of a pixel an update.
- **Steep rays** take at least 16 samples through the shadow-casting layers' depth (takram's step grows with the
  distance to the layers, which left a steep ray from the chart four).
- **Scene shadows**: takram shades a scene through the BSM in its deferred aerial-perspective pass; the game's ships,
  islands and sea are lit forward and read the ground shadow map instead, through the interface they always had.
- **Light shafts** show in the sky's dome (the hooks), not over the sea or ships, whose haze is the game's aerial
  perspective rather than takram's aerial-perspective pass.
- **Haze** (2026-10-03, `haze.md`): takram's thin haze is off in every tier; the march takes the air in front of the
  clouds' front from the aerial-perspective volume (`naval_sky::aerial::aerial_along`, the same air every material
  takes) in place of `apply_aerial_perspective`, which the environment bake keeps. `GameSky` runs the volume's pass
  (`crate::aerial`) after the clouds' shadows.

## Using it

```rust
let mut sky = naval_clouds::GameSky::new(&device, &queue, SkyQuality::Medium, naval_clouds::Targets::default());
// or GameSky::new_in(ctx, quality, targets) to build on the renderer's registry (hot reload)
sky.apply(&queue, &scene);                       // naval_sky::SkyScene: sun, moon, air, cloud coverage, weather
renderer.render(input, &mut [&mut ships, &mut ocean, &mut sky], output)?;
let light = sky.light();                         // CelestialLight: the sun or the moon
let flash = sky.lightning();                     // LightningLight { flash, bolt }
for thunder in sky.thunder() { /* distance, delay, loudness for the audio */ }
```

- **Frame order** (`GameSky::prepare`): cut detection (250 m, 0.5 rad, a 20% zoom, a 2° sun move, a 0.05 coverage
  change) → weather (lightning, rain boxes, splashes) → the strike to the sky → the sky's tables → the clouds' CPU half
  (cascades, uniforms, the update's Bayer cell) → the BSM, their temporal pass and a slice of the ground shadow map →
  the environment bake. At render core II's opaque copy point (`after_copies(Opaque)`, `scene.opaque.depth`) the
  march and the reconstruction: rays stop at the opaque scene and the sea sphere. In `Phase::Sky` the dome (with the
  shafts) then the composite over everything drawn; splashes, the bolt and rain in `Phase::Transparent`.
  `GameSky::prepare` publishes what the composite draws (`CloudLayer::cover`, `naval_sky::CloudCover`) for the
  transparents drawn after it: the combat effects fade their smoke by the clouds in front (`native/docs/combat-effects/README.md`).
- **Under water** (`ray_starts_under_water`): a ray that starts under the waves meets the sea's surface before any
  cloud, so the march gives it no clouds, haze or shaft and the composite draws nothing there. Which rays do comes from
  the ocean's underwater probe (`GameSky::set_underwater_probe`, the client hands it over: each pixel's near-plane
  point's depth below the waves and the wake), so a camera in a trough keeps the sky above the wave and one under a
  crest loses it; without an ocean no ray starts under water. The composite is not depth-tested and knows only the
  opaque depth, taken before the sea is drawn, so until 2026-10-03 it painted the clouds over the sea's underside at
  any depth. The sky, clouds and all, shows under water only through Snell's window, which the sea's underside takes
  from the environment (`sea-sky.md`, "The underwater view").
- **Scene**: the clouds read the weather from `SkyScene` (`naval_clouds::weathers::CloudWeather`: the cover, the genus,
  the base and depth when chosen, the high sheet, fog banks, rain) and the wind's drift, and draw its layers ([weather](weather.md#weather)).
  `GameSky::apply` puts a scene on at once (a cover change over 0.05 cuts, as before); `GameSky::blend(from, to, t)`
  draws the weather `t` of the way between two scenes without a cut, for a weather change drifting in.
- **Held skies and paused pictures**: `hold(Some(t))` stands sky time and drift at `t`, restarts the sequence on each
  new view (a full march first) and stops marching after `HOLD_UPDATES` (496: every pixel marched 31 more times, its
  mean 32 samples; 240 before 2026-10-28, 48 before 2026-10-03); a paused frame converges the same way and then
  stands, so captures are deterministic.
- **Sky time at a rate** (from 2026-10-03): `GameSky::time_scale` (1 by default) steps the sky's time, the clouds'
  drift and their evolving shapes that many seconds a frame second; the rain and the lightning keep the frame's time.
  Since 2026-10-06 the theatre leaves it at 1× at every speed, so the camera riding an accelerated ship travels
  through the cloud field while the clouds keep their natural wind drift. Previously cruise scenes accelerated
  them with the ships up to 8×, making them appear to move alongside the camera. Pausing still holds their drift
  and shape evolution. A frame costs the same at any rate.
- **Diagnostics**: `force_strike` (the web's `forceStrike`), `GameSky::enabled` (profiling), `stills --hdr`.

### What other lanes read

- **Cloud shadows** (ships, islands, the sea): unchanged. `sky.clouds.shadow()` is a `CloudShadow { view, sampler,
  uniforms, bind_group_layout, bind_group }`; `naval_clouds::shadow::cloud_shadow(map, sampler, params, position)`
  (cubic B-spline, four reads) or `cloud_shadow_fast` (one read): 1 in full light, 0 under the thickest deck (0.15
  before 2026-10-03), faded in with its body's elevation (from 1° under the horizon to 2° up, as the sun's light at the
  sea fades; from 1° to 7° until 2026-10-20, when a storm's deck let the dusk sun onto the hulls: `native/docs/lighting/daylight.md`,
  "Storm light") and at the map's edge, clear at the port's berth
  (`CloudClearing::port`). The map (`shaders/ground.wgsl`) is the transmittance of the scene's light, the sun's or the
  moon's once it outshines the sun (`CloudUniforms::ground_light`, from 2026-10-02: before, nothing shaded moonlight),
  through takram's shadow-casting layers above each point of the sea, 40 km across in the clouds' drifting frame,
  marched with the clouds' own field, 16,384 texels an update (1024² on High and Ultra, a full pass about every 64
  frames; 512² on Low).
- **The environment** (`naval_sky::EnvironmentMap`) holds the clouds through `clouds_bake` (the tier's bake steps, the
  sky gradient, and the sun's light through six even steps up to the top of the shadow-casting layers, swept as
  before; a deck's steps lit at its base, below). The march on screen takes the bulk of that optical depth from the
  Beer shadow maps, which cover only the camera's frustum; with takram's 100 m march toward the sun alone the bake's
  clouds came out nearly unshaded. The sea reflects the bake and takes its water's light from it, so a cloudy sky now
  reads paler and greyer on the sea than the old clouds' did: under the partly cloudy chase view its mean is (53, 72,
  92) against (19, 50, 75) before (eye adaptation off, 2026-10-02). About half of that was the clouds' gain
  (`CLOUD_GAIN`, gone since 2026-10-03) and the rest is takram's brighter and fuller cover. The bake's alpha is the
  luminance the texel would have without clouds, prefiltered with the radiance: the eye adaptation meters against it
  (`native/docs/lighting/daylight.md`, "Realism").
  **A deck is lit at its base (2026-10-13).** The bake's steps are hundreds of metres long (the layers' whole depth in
  12 to 32 steps), more than a deck is deep, and each was lit at its middle: a deck seen from the sea took the light of
  a point most of the way up it, where little cloud is left overhead, so the sea, the hulls and the haze, which take
  the bake's light, were lit by a deck brighter than the one on screen, and brighter on a tier with fewer steps. A
  step of a slab (a deck, fog) is now lit where its light comes from (`light_depth`: the mean depth of what the step
  scatters back to its start, half of a thin step, one mean free path into an opaque one); broken cloud keeps its
  middle, so fair weather draws the same bytes. Found when Bill asked whether the night was too bright: an overcast
  night's sea was brighter than a clear night's. Measured in the client (Bismarck, the North Atlantic, a full moon 28°
  up at night, Medium clouds, 1728 × 1030, one binary with the bake swapped by hot reload), before → after, and in
  brackets the old bake marched 16 times as finely with its light in 48 steps:

  | Scene | Environment's sky mean | Sea beside the hull | Near sea, toward the moon or sun | Hull |
  | --- | --- | --- | --- | --- |
  | Overcast night | 0.0232 → 0.0138 (0.0142) | 9.7 → 5.8 (5.7) | 22.4 → 7.9 (8.8) | 14.2 → 8.4 (9.4) |
  | Overcast noon | 0.435 → 0.340 (0.340) | 103 → 100 (99) | 77 → 72 (72) | 73 → 73 (73) |
  | Storm, noon | 0.122 → 0.120 (0.117) | 74 → 74 (72) | 58 → 58 (58) | 67 → 64 (64) |
  | Fog, noon | 0.787 → 0.701 (0.694) | 136 → 135 (138) | 120 → 119 (120) | 125 → 125 (124) |

  Sea and hull are 8-bit medians. A clear night reads sea 21, near sea 12 (with the night sky's glow, 2026-10-12).
  The overcast noon's eye opens 0.4 stop where it had not, as the reference's does. `tests/gpu.rs`,
  `an_overcast_nights_ambient_is_the_same_on_every_tier`, holds Medium's and Ultra's deck ambient within 15% (0.0301
  against 0.0179 before, 0.0131 against 0.0139 after) and under a clear night's. Left as it was: the six light
  steps (doubling them from the sample left the deck brighter), and broken cloud's steps, which carry the same bias
  (a partly cloudy night's sky mean 0.0175 against 0.0149 by the fine march, 2026-10-12). Stills in
  `.build/native/night-look/`.
- **The cover** (`naval_sky::CloudCover`, `CloudLayer::cover`): what the composite draws each frame, for the layers
  drawn after it. The combat effects fade smoke and glows by the clouds in front of them (`cloud_visibility`), the
  aircraft's condensation likewise, and the ships' thin members drawn as strips take the share of the clouds in
  front of them over the scene behind (`wires.md`, "How it fits the frame"), for which the cover also publishes the
  reconstruction's state (since 2026-10-27).
- **Lightning on meshes, thunder, eye adaptation**: as before; lightning lights the clouds again since 2026-10-03, through
  the march ("Night: the moon and lightning").
- **The hooks** the sky composes (`naval_sky::CloudHooks`): `clouds_shadow_length(pixel)` and
  `clouds_bake(origin, direction)`.

### Post passes

None since 2026-10-20. The screen-space sun shafts went with the old clouds (takram's light shafts take their place
in the dome), and heavy rain's veil, a second fog over the finished scene, went when the rain took its share of the
haze ([rain](weather.md#rain-as-a-camera-records-it)).

## Quality tiers

takram's presets, onto the tiers of the same names (Graphics → Clouds), less three trims (below). The march renders a
quarter of the drawing buffer each axis and reconstructs the rest over 16 updates on every tier.

| Tier | Light shafts | Shape detail | Turbulence | Sun / sky light | Primary march | Shadow maps | Ground map |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Low | no | no (and no detail noise read) | no | interpolated | 200 steps of 100 m+, 100 km, ends at 10% transmittance, 1 sun step | 2 × 256², 25 steps | 512² |
| Medium | no | yes | no | interpolated | 500 steps of 50 m+, 200 km, 2 sun steps | 3 × 256², 50 steps | 1024² |
| High | yes, from 200 m steps | yes | yes | accurate | 500 steps of 50 m+, 200 km, 2 sun steps | 3 × 512², 50 steps | 1024² |
| Ultra | as High | yes | yes | accurate | as High, from 10 m steps | 3 × 768², 50 steps | 1024² |

**The trims** (2026-10-02), each held against takram's settings on six views at High and Ultra (the bridge at noon and
toward a dusk sun, 2.6 km at noon and at dusk, a dive, the chart), none of them visible side by side:

- **The light shafts' march starts from 200 m steps** (takram's 50 m). Its steps still grow 1% each to the same 200 km,
  so the longest ray takes about 240 steps instead of 370. Capping it at 128 steps instead stops it near 51 km and
  lightens a strip along the dusk horizon.
- **No step toward the ground** (takram marches three for the sea's light on the cloud bases; from 2026-10-02 High and
  Ultra drew no bounce, their bases a few levels darker, and Medium marched one step). Since 2026-10-04 every tier takes
  the bounce through the cloud its weather puts under a sample ("The light of broken cloud"), and three octaves of
  multiple scattering in broken cloud where takram's presets have eight.
- **Ultra's shadow maps at 768²** (takram's 1024²): a dusk sun's shafts over a low bank keep their number and length,
  slightly softer. At 512², High's size, most of them go.

## The light of broken cloud

From an aircraft at noon takram's cloud tops were evenly white and flat, and toward a low sun a cloud was one dull mass
without bright rims. The causes, from the research behind the plan (`.build/cloud-research`, not in the repository):
eight octaves of multiple scattering at a half each (the film paper's defaults: the first octave, the only one with the
true shadow and phase, is half the sum, and a quarter of it lies in octaves whose optical depth is cut 8 to 128 times),
a forward lobe tuned for midday, and the whole clear sky's light on every sample. Now, for broken cloud (cumulus, a
storm's towers, the high sheets), in `shaders/march.wgsl` (`ray_light`, `sun_scattering`, `sky_light`,
`sample_radiance`):

- **Three octaves** (every tier; shipped renderers use one to three; since 2026-10-25 the first octave and one diffuse
  term, "Bright tops" below): octave i carries 0.8^i of the light through
  0.7^i of the optical depth toward the sun, by a phase whose anisotropy is cut to 0.6^i (Dagor's form; its values are
  0.7, 0.3 and 0.6, "at noon 0.5 to 0.6 and 0.45"). The later octaves light a cloud's first few optical depths; from
  about ten its core takes only the sky's light. The contribution holds a sunlit top where takram's was (its 99th
  percentile from 2.6 km at noon is 208 of 255, as it was). The attenuation is steeper than Dagor's because the tops'
  relief grows with it at the same brightness: 9.7 levels at 0.45, 10.1 at 0.6, 10.3 at 0.75, against takram's 9.6
  (relief: the RMS of luma less its 12 px blur over the clouds). A ray takes the octaves' phases once (the angle to the
  sun is one along it), so a sample costs one exponential of a vector where takram's cost eight exponentials and
  sixteen powers.
- **A narrow forward lobe on the first octave** (Guerrilla 2017's form, max(lobe, 0.7 × HG(0.8))): a cloud's thin edges
  within about 28° of the sun, where the optical depth to the sun and to the eye are both small, take up to 3.3 times
  takram's forward lobe (2.3 times at 10°, 1.4 at 20°). The first octave's full extinction keeps it to the edges.
- **An edge term in place of powder** (since 2026-10-25 on the diffuse term, "Bright tops"): the later octaves are
  light scattered more than once, which gathers with the
  depth of cloud about a sample. They ramp from none at a view ray's entry into cloud to all from 0.8 of opacity gathered
  along the ray (Dagor keys its edge albedo the same way: a ramp on the local density, which takram's powder was, is one
  darkening everywhere, since the shell the eye sees is thin throughout), and not at all looking toward the sun (cos⁸),
  so the silver lining stays. The ramp is taken as its mean over each sample's stretch of the ray (`edge_ramp`): taken
  at the stretch's start it left the first sample of a dense cloud, most of what the eye sees of it, without its later
  octaves, by an amount that changed with the step (a tier's, the bake's). The sun's light only: darkening the sky's
  light lowers a cloud's brightness without adding contrast.
- **The sky's light less what the cloud about a sample hides** (`sky_occlusion`; Guerrilla 2022's pow(1 − profile,
  0.5), floored at 0.4): the profile is the sample's layer's weather before the shape and detail noise carve it
  (`WeatherSample.density`). Checked per layer: it is 0 at the envelope's edge, top and base and rises into the cell (to
  0.63 at takram's cover of 0.38, to 1 from a cover of about 0.6, where the floor holds); the shape noise puts the
  surface the eye sees where the profile equals what it carves, so hollows, carved deep into the envelope, take less of
  the sky than billows. It is a smooth field: it shades a cloud's hollows and core broadly and gives no billow-scale
  relief by itself (that is the sun's). Slabs do not take it (their light from above is `deck_skylight`).
- **Thin layers scatter once** (`layer_bodies`: a layer's optical depth at its thickest over 8; the high sheet 0.19, a
  rain shaft under 0.3, every cumulus and deck 1): in proportion to its thinness a layer has neither the back lobe (which
  stands for the light a body returns toward the sun), the narrow lobe nor the later octaves, and half its forward lobe
  (ice crystals and rain drops send half of what they scatter into a diffraction peak too narrow to tell from the sun).
  takram's powder had left the high sheet about a third of its light at every angle, and its density was chosen with
  that: without this the high sheet doubles in brightness and whites out the sky round a low sun.
- **Slabs keep takram's light** (decks, the storm's deck, fog: `Layer::slab`): its eight octaves and its powder. Under a
  higher sheet (an optical depth of a few, which stops the three octaves) takram's long tail is what reaches a deck's
  top, the deck's own light from above (`deck_skylight`, "less the octaves' share") was derived against it, and the
  creases on a deck's top turned out to be takram's powder on that light. With the new light alone a deck from above is
  a smooth sheet. The light volume (the plan's second stage) gives slabs their true optical depths; their light is its
  to rework. **Lightning** keeps takram's eight octaves too (`approximate_multiple_scattering`), and the moon lights the
  clouds by the same functions as the sun.
- **The sea's bounce on every tier, with no step marched** (`radiance_from_ground`; `quality.rs`): takram marches
  toward the ground for the cloud under a sample (three steps; one on Medium here, none on High and Ultra from
  2026-10-02, which drew no bounce). Without the sun's deep octaves the bounce is about half a cloud base's light, and
  the tiers without it drew bases darker than Medium's (11 levels at the 5th percentile from the sea at noon, 7 before).
  Now every tier takes it through the cloud its own layers' weather puts under the sample (a tenth of the weather at
  the sample over the depth down to the layers' bases: nothing at a base or a side, everything inside a cloud), by
  the frame's light at the camera. From the sea it is within a level of the marched step's in fair weather and under a
  deck, 1.3 under the storm.
- **The environment's bake** (`hooks.wgsl`) takes the same functions (`ray_light`, `sun_scattering`, `sky_light`), so
  the sea reflects the clouds the sky shows.

One lit sample's light is `sample_radiance` (position, weather, medium, the opacity the view ray has gathered before and
after the sample's stretch, the ray's `RayLight` and `FrameLight`): the far march calls it, and a near pass can.

### What it costs

`examples/timings.rs` (2400 × 1350, Apple M5 Pro, 2026-10-04), master's crate and this one built side by side and run
alternately, three rounds each, load average 5 to 8; the march's tenth percentile, the lowest round, milliseconds:

| Tier | Noon | Sunset | Overcast | 2.6 km | Storm |
| --- | --- | --- | --- | --- | --- |
| Low | 0.62 → 0.61 | 0.67 → 0.65 | 0.84 → 0.94 | 0.82 → 0.78 | 0.50 → 0.55 |
| Medium | 1.17 → 1.00 | 1.30 → 1.11 | 1.92 → 1.71 | 1.54 → 1.37 | 0.99 → 0.86 |
| High | 1.61 → 1.55 | 1.76 → 1.72 | 1.93 → 2.10 | 2.04 → 1.99 | 1.20 → 1.30 |
| Ultra | 2.03 → 2.00 | 2.24 → 2.22 | 2.12 → 2.29 | 3.21 → 3.13 | 2.66 → 2.93 |

Medium, the default, is 0.13 to 0.21 ms cheaper everywhere: the octaves (their phases once a ray), and the bounce
without its marched step. Fair weather is a little cheaper on every tier; under a deck Low, High and Ultra pay 0.05 to
0.27 ms for the bounce they did not draw. The other passes are untouched. (A marched step on every tier instead costs
High 0.1 to 0.5 ms and Ultra up to 0.8; the sea's own light under each sample, takram's accurate form, another 0.4 ms
under a deck.)

### Open

- **Slabs** (decks, the storm's deck, fog) keep takram's light: flat grey inside a deck, no long shadows across a deck
  at a low sun. The light volume is the place for it.
- **Self-shadow still reaches 150 m** (two marched samples, then the Beer shadow maps' one slab a texel): billows do not
  shade their neighbours yet. The relief gained here is the octaves' and the edge term's.
- **The sky's light has no direction**: a cloud's shadowed side takes the whole sky's irradiance (less the local
  occlusion), so at dusk the cores away from the sun stay about as bright as they were (the 5th percentile did not
  fall).
- **The sea's bounce is a guess** on every tier (takram's too): the clear sky's light at the camera on a sea of albedo
  0.3, the sun's by the uncovered share of the sky, which under a closed deck is far too much.
- **The high sheet near a low sun** is brighter than it was (the forward lobe a veil keeps is half of takram's, where
  powder had left a third of everything).
- **The interpolated tiers' light at dusk**: Low and Medium take the bounce's sun as the scalar irradiance at the camera,
  not on the sea's surface, as before.

## Bright tops

The clouds were too dark, most of all from above. A thick cloud's sunlit top returns most of the sun's light: a slab of
optical depth 50 scattering by Henyey–Greenstein 0.85 reflects 0.82–0.89 of it (Monte Carlo, `.build/cloud-tops/tops.py`,
not in the repository; measured cloud albedo is 0.7–0.9), and at midday a top's πL/E (1 is a white card facing the sun)
is 0.65–0.83. By the sun's term ours was 0.20–0.26, takram's 0.28–0.33, Wicked Engine's and Skybolt's about 0.3, HDRP's
0.23–0.29 (0.84–1.12 with the sun behind, its two lobes summed). Octaves whose optical depth to the sun is cut by a
fixed factor (Wrenninge's approximation) send too little back toward the sun. The eye then closed for what was left:
from aloft a sunlit field filled the view's brightest tenth as the aureole does into the sun, and closing for glare held
it under a noon's highlights.

Photos are the measure (Wikimedia, `.build/cloud-tops/ref/`: `tones.py` (historical) takes the cloud pixels, unsaturated and not
dark, and their 10th, 50th and 90th percentiles of luma). From the air the tops are bright and the cloud's inside is
grey: its median stands 21–50 levels of 255 under its 90th percentile (the gap), and a quarter to a half of it lies
within 20 levels of the tops. Master's gap was 9–17 with three fifths to four fifths near the tops. A first try lit
the inside with diffused light (1 / (1 + 0.35 τ), `EDGE_ALBEDO` 0.5, the eye's room 1.25 stops): the tops reached
225–234 but the gap fell to 5–14 and the darks rose, washed out. Now, for broken cloud (`shaders/march.wgsl`:
`ray_light`, `sun_scattering`; the bake's `hooks.wgsl` the same) and the eye:

- **One diffuse term in place of the later octaves, fading steeply**: `DIFFUSE_GAIN` 6 of the sun's light by a phase
  near isotropic (its anisotropy cut to 0.36), times exp(−1.5 τ) of the optical depth toward the sun
  (`DIFFUSE_FADE`). A march has no surface: a sample on a flank turned from the sun or in a hollow differs from one on a
  top only by how fast τ grows into it, so the fade has to be steep for the shading to show. In the slab model
  (`.build/cloud-tops/v2model.py`) a flank the sun meets at 25° takes 0.42 of a top's light (a Lambertian surface 0.47,
  the octaves 0.49, the first try 0.73) and a hollow three optical depths deep 0.03 (0.11, 0.44), while a top takes
  0.26–0.41 of a white card (0.20–0.26, 0.48–0.53). A top takes the sun by its area: `DIFFUSE_FLAT` 0.3 + 0.7 μ (μ the
  sine of the sun's height over the sample's up), so a low sun's flat tops stay near the slab's 0.2–0.3. It leaves out
  what the column above has closed (`ColumnAbove.closed`), the share `deck_skylight` lights. Slabs and lightning keep
  takram's octaves; a tier's `multi_scattering_octaves` over 1 turns the term on. Rates 1 and 1.5 were measured in the
  client; 0.25–0.35, the slab's diffusion, light the flanks nearly as the tops.
- **The edge term from none** (`EDGE_ALBEDO` 0, as with the octaves): a view ray's first sample in cloud takes none of
  the diffuse light, all of it from 0.8 of opacity gathered; half there lit the rims as the faces.
- **The eye leaves the exposure where it was** (`EyeAdaptationSettings::glare_away` 0.8, `post/mod.rs`; `glare_room` in
  `eye_adaptation.wgsl`): the brighter tops alone closed it 0.75 stop from the air at noon (the sky above the horizon
  23 levels darker, the sea and the shading with it). By day, looking away from the sun, the brightest tenth may stand
  0.8 × μ☉ stops more over `highlight` before the eye closes for glare (μ☉ the sine of the sun's elevation, since the
  tops' extra light goes as it; `sun_away`: 0 within 45° of the sun, whole from 90° off), so the exposure is master's
  and the tops alone are brighter. Into the sun the aureole and the glitter close it as before; after dark nothing
  changed.
- **A longer march toward the sun was tried and left out**: Medium's two steps (about 150 m, then the shadow maps) as
  three over 350 m changed the gap by at most a level and the relief by 1–4%, for 0.13–0.15 ms more of the march on Medium (7–9%, "What it costs"). The shadow maps already
  carry a billow's shadow onto its neighbours.

### What it costs

`examples/timings.rs` (2400 × 1350, Medium, Apple M5 Pro, 2026-10-25, load average 8 to 27 with other worktrees building),
master's cloud shaders, these, and these with the longer march toward the sun, in three release builds run alternately,
six rounds each (`.build/cloud-tops/ab/gpu3.sh`); the march's tenth percentile, the lowest round, ms:

| Scene | Master | This | With the longer march |
| --- | --- | --- | --- |
| Noon | 1.44 | 1.55 | 1.68 |
| Sunset | 1.62 | 1.76 | 1.89 |
| 2.6 km up, level | 2.14 | 2.15 | 2.29 |
| 20° down | 1.97 | 1.98 | 2.13 |

Up to 9% of the march, the per-sample arithmetic (the ray's light is taken once a ray; nothing more is marched), the
first try's cost; the environment pass is unchanged (0.03). The whole frame in the client (shot.ts) moves more with the
load than this and cannot show it.

### Open

- **The gap from the air is still under the photos'** looking away from the sun and down (13–16 against 21–50; across
  and toward it 24–33): the light is not what holds it. The field is soft and flat-topped, its surfaces fuzzy, where
  the photos' cumulus are tall and crisp, with flanks and hollows in view; that is the clouds' shapes (the weather's
  profiles, the shape noise and its erosion), or a surface normal from the density's gradient for the diffuse light
  (two or three more samples a step).
- **Tops are still under the real 0.65–0.83** of a white card by the sun's term (0.26–0.41); the eye's room and the
  display's shoulder carry the rest.
- **Not measured**: the overcast, the storm, fog, night and the tiers other than Medium.

## Fine grain

Bill asked for less grainy clouds: still, moving or after a switch of view, and flying near them, with High clouds
through a 38° lens at 70% of 3840 × 1600 under temporal AA (the clouds take none of its jitter). Held through that lens,
where every pixel holds its most samples, the clouds showed a fine sandy speckle the clouds converged did not. It was
two grains of about equal size:

- **The samples' noise**: a still pixel's mean held 16 samples, and kept 0.62 levels of 255 of fine-scale noise
  (against the same shaders converged at 64), falling as 1/√n.
- **Aliasing**: every ray passed through its pixel's centre, so a pixel's mean converged to the clouds at one point
  of it, and the detail and edges finer than a pixel stood as a fixed grain no number of samples takes out. The clouds
  converged that way differ by 0.69 levels of fine grain from the clouds converged over the whole pixel, and hold 7%
  more fine-scale energy.

Two changes:

- **The ray's place in the pixel steps** (`field.wgsl`, `march_subpixel`): each march of a pixel sends its ray
  through another point of it, the R2 sequence (Roberts 2018) shifted for each pixel by the blue noise read at two other
  places of the mask, stepping once per march of the pixel as the jitter along the ray does (the full march after a cut
  takes the step before the pixel's next). A pixel's mean converges to the clouds over the whole pixel.
- **A still pixel holds up to 32 samples** (`resolve.wgsl`, `MOST_STILL_SAMPLES`, about eight seconds at 60 frames a
  second; 16 before) where neither the view nor the clouds' drift moves it. A drift too slow to reproject (under a
  hundredth of a pixel an update: on those settings at 120 frames a second, a 7 m/s wind's clouds beyond about
  10 km) is read in place and smears the mean along it, so such a pixel holds only as many samples as span the drift
  16 did at that threshold (2.6 px), and 16 at the least. A moved pixel keeps 8. A paused picture marches 496 updates
  before it stands (`HOLD_UPDATES`, 31 Bayer cycles; 240 before), about four seconds more at 60 frames a second.

### What it costs

Two reads of the blue noise and an R2 step a marched ray, beside the hundreds of reads of its march, and a clamp in
the reconstruction: not measured. A paused picture marches 256 more updates before it stands, at play's cost.

### Open

- **Drifting and moving clouds are soft**: their pixels hold 8 samples resampled every update. The windy view keeps 95%
  of the converged clouds' fine detail and stands 3.3 levels off them, most of it that blur.
- **Flying inside cloud** was not measured: from within the low layer a second after a cut the clouds were visibly
  speckled. A pass outside a near cloud measures soft rather than grainy (above).
- **Faint horizontal striations** in the lower parts of the clouds in held and windy views, not explained.

## Near detail

Bill's brief (2026-10-04): "we have planes in the air, we basically want clouds as good as those in flight sims". The
problem this stage takes: a cloud within about a kilometre is a soft blob with no fine structure even when the picture
has settled, because the finest data was takram's 32³ detail volume (5.2 m texels, features of 20 to 40 m).

- **The detail noise** (`src/detail_noise.rs`, `shaders/detail_noise.wgsl`) is one 128³ four-channel volume with five
  mips, tiling every 100 m (0.78 m texels): low- and high-frequency wisps in r and g, low- and high-frequency billows in
  b and a. Every channel is layered "alligator" noise, written here from the kind's published description (a cellular
  noise: each lattice cell holds one bump of a random height at a random place, and the noise is the tallest bump at
  a point less the second tallest, so bumps stand rounded with creases between them; Guerrilla's Nubis³ names it for
  billows where inverted Worley noise reads as packed spheres). The wisps' lookups are displaced by curl noise first,
  which swirls the creases into filaments. Stored bumps bright: the field carves the bumps away for wisps (the web of
  creases stays) and the creases away for billows (the bumps stay). It is filled on the GPU at startup (49 ms on the
  M5 Pro, once; 9.6 MB of GPU memory), on a lattice hashed with integers, so every GPU places the same bumps: the
  texels differ between GPUs by float rounding alone (`reference_texel` is the same noise on the CPU; a test holds the
  fill to it, within a level of 255). No asset is shipped; takram's `shape_detail.png` is gone. Guerrilla's own noise
  pack carries no licence and was not used or read.
- **The up-rez** (`field.wgsl`, "The detail"; after Nubis³): the frequency by the depth into the cloud (the low
  frequency at the edge, more of the high toward the core: the direction the Nubis³ slides print; the other way round,
  which the speaker's photographs suggest, the edge is all fine grain here and no lobe shows), wisps at a layer's bottom
  and billows at its top (takram's rule), and the mip chosen from the march's step and the pixel (`detail_lod`: a texel
  no finer than an eighth of the step, which a pixel's 8 to 16 jittered samples resolve, nor than half a pixel), never
  finer than mip 3.
- **One field.** Every march reads the noise at mip 3 or coarser (6.25 m texels, about takram's), carved to takram's
  statistics (its wisps carved 0.17 of the edge on average and its billows 0.29, with its spread), so the clouds keep
  their size, their soft edge, their noise and their cost. A paused or held picture reads the same field as play and
  converges to the picture play drew the moment before (`gpu.rs`, `a_paused_picture_from_the_air_is_plays`).
- **The near field, retired 2026-10-08.** Until then a picture that stood (held, or paused) over a camera in the air
  read a second field within 600 m of it (fading out by 1,200 m), walked in steps of 0.35 of optical depth, 8 m at the
  least: the finer mips, the carving at 1.6 contrast (0.47 in the far field), a twice-folded copy of the fetch inside
  150 m, the edge sharpened by a power of 0.3 to 0.6, the inside uneven (the low-frequency wisps taking up to 0.6 of
  the density anywhere) and the density thinned around the camera. Play never read it: its 50 m steps cannot resolve
  it (short steps cost 1.6 ms more among cumulus on Medium), and the near pass meant to carry it into play (stage 4,
  draft #976) was shelved. Bill saw the result: pausing on an aircraft turned the cumulus beside it from play's soft
  cloud into a mottled web of dark creases ("the clouds start looking horrible"), and play brought the soft cloud
  back. A paused frame and the frame before it differed by 14 levels on average over a cumulus 350 m below the camera;
  they now differ by under 1 (the samples' own noise).

**The trap: the texture cache.** Reading mip 2 of the four-channel volume (32³ × 4 bytes, 131 KB) in every media
sample cost the march half as much again at sea level (1.16 → 1.77 to 2.1 ms at noon, Medium), where takram's 32³
single channel (32 KB) stays in cache; mip 3 (16 KB) costs 0.07 ms. Hence the 100 m tile (mip 3 holds 6.25 m texels,
about takram's) and the floor at mip 3 on every tier (without it Ultra's 10 m steps read mips 0 and 1 from
everywhere: +0.7 ms at noon from the sea, +2.0 to +3.5 ms in the air).

**Measured** (2026-10-04, Apple M5 Pro, Medium clouds, the High preset; before is origin/master d7f721e16, which read
takram's 32³ volume): the sea-level views are not noisier and cost 0.07 ms more of march (1.15 → 1.22 ms at noon,
1.28 → 1.35 at sunset, 1.91 → 2.04 under an overcast; 2400 × 1350); High +0.05 to +0.10, Ultra +0.0 to +0.2, Low
unchanged (it reads no detail). The air views cost the same 0.05 to 0.09 ms more. From 2,600 m the settled picture
loses 6% of its fine detail (mip 3's 6.25 m texels against takram's 5.2 m) and a tenth of its noise. The inside of a
deck is one flat grey: the light there (`deck_skylight`) is the same at every sample, so no density structure shows,
whatever it is; it needs light that varies inside the deck, not more density detail.

**Open.** Fine structure in near clouds, in play and paused alike: it needs short steps near the camera every frame
(the near pass, with a short history and a light volume), not a field only a standing picture reads. A step back to a
cloud's edge on the first sample of it. The run-time turbulence on High and Ultra was not retired: it displaces the
shape (30 to 300 m) at the cloud bases, a scale the baked curl in the wisps (5 to 40 m) does not reach. The 100 m tile
repeats along a long grazing view of a deck's top no more than takram's 167 m did; not seen in the views checked.

## Night: the moon and lightning

At night the clouds were dark shapes (takram's are lit by the sun alone) and lightning lit hulls, land and rain but
not the clouds it strikes from. Now:

- **The moon is the key light after dark** (`src/key_light.rs`). The body whose beam the march, the Beer shadow maps,
  the light shafts and the environment's bake follow (`CloudUniforms::sun`) is the sun by day and the moon once the
  moon's direct light at the top of the clouds (`heights.y`, where the sun lingers longest) outshines the sun's: the
  clouds' sunlight as the march takes it (the sun's luminance above the air, the twilight lift, the dome's gain)
  against the world's moonlight (`SkyUniforms::moon_irradiance`, the dome's gain), each through the air (`model::
  optical_depth` on the CPU). For a full moon opposite the sun and the clouds' 8 km top that is 2.5° after the sun
  sets at the sea, the two beams equal there (0.217 against 0.212: the test holds them within a tenth), so the direct
  light passes from one body to the other without a step; the sky's light on them is continuous through it. Before
  the switch the low clouds have already lost the sun's beam (it leaves them at about 1.4° below the horizon); after it
  the high ones keep the moon's.
- **The moon's light on them** (`shaders/march.wgsl`, `key_scalar_irradiance`, `key_irradiance`): its beam through the
  same tables (`get_sun_and_sky_scalar_irradiance` toward the moon) at the world's moonlight on the dome's scale
  (`key_light.xyz`: the dome's gain × the moon above the air over the tables' solar irradiance, per channel), no lift
  of its own; and the sky's light: the moonlit sky at the dome's grade (`sky.moon_scale`), the sun's twilight while it
  still lights the sky (`sun_active`), the night floor. By day `sun_sky_scalar_irradiance` as before, so day and dusk
  draw the same bytes. The haze, the ground bounce and the bake take the same light. The light shafts are off under
  the moon (`shafts.w`): the dome takes their length out of the sun's light.
- **Lightning lights the clouds** (`flash_radiance`). The flash the weather strikes (the sky's `lightning_position`, the
  channel's brightest point inside the cloud, and `lightning_intensity`, an irradiance of `intensity × (1 km / r)²`
  in the sea's units; set the same frame as the bolt) is a point light in the march: its light reaches each sample
  through the cloud between, the weather's extinction (the layers' coverage and profiles before shape and detail carve
  them, smooth at a stroke's reach) over three jittered samples, scattered by the sun's octaves and phase; deep in a
  deck, where the octaves' fall-off leaves nothing, by the light a non-absorbing cloud diffuses (the two-stream slab
  transmission 1 / (1 + ¾(1 − g)τ), g = 0.85 for cloud droplets; takram's dual lobe is fitted to the look, not the
  transmission). A ground stroke lights the deck's base about its channel and the cloud about its top from within; an
  intra-cloud flash glows through a deck's top and, faintly, its base. The march costs more only while a flash lights
  the clouds (a uniform branch).
- **The reconstruction follows the flash** (`resolve.wgsl`, `CloudUniforms::flash`): while a flash lights the clouds
  (this update's or the last's), every pixel clips its history to the march as a moving view's does (variance gamma 2),
  so the flash shows the frame its stroke does and goes with it (a still pixel's gamma 8 kept the history dark through
  the flash); held or paused clouds march again.
- **Cost** (Apple M5 Pro, 2026-10-03, High, 2400 × 1350, master against this alternating, 12 rounds a side, load
  7–20): under the moon the Beer shadow maps take 0.04–0.06 ms more (they march toward a moon above the clouds, where
  master marched toward a sun below the horizon) and the march 0.04–0.17 ms less (no shafts); with a flash lighting a
  partly cloudy sky the march stays within master's (0.98 against 1.02 ms median); by day nothing moves (the water pass
  4.217 against 4.221 ms, the march 1.060 against 1.048).
- **Strike placement**: the weather places a strike's channel from the scene's cloud base and depth
  (`StrikeClouds`), which the layers follow only where the scene chose them (`weathers.rs`; otherwise their heights are
  the weather's own); an intra-cloud flash placed above the layers lights only their top. The stills' intra-cloud flash
  is seed 15, 1.8 km up, inside the deck.
- **Star shells under a low base** (2026-10-20): a burning star shell under the clouds lights their base, drawn by
  naval-effects' light in the air (`air_light.rs`, `cloud_base`) over what the composite drew, not in the march: the base
  sends back 0.7 of the light from below where the clouds cover the pixel, taken through its lowest 150 m. It reads the
  lowest layer's bottom (`EffectLighting::clouds`) and the composite's cover; seen from above the deck it draws nothing
  (`native/docs/combat-effects/night-light.md`, "Light in the air at night"). A term in the march would light the clouds' own shapes from below
  and through a deck from above; it is the follow-up.

## GPU cost

`cargo run -p naval-clouds-sim --example timings --release`: five scenes (noon from the bridge; sunset toward a 6° sun;
overcast overhead at coverage 0.82; 2.6 km over the deck; a dive) per tier at 2400 × 1350 (1600 × 900 at the game's
1.5 pixel ratio), live clouds (not held). Compute passes by GPU timestamps through the renderer core's timer, medians
over 120 frames; the clouds' total is the five passes of a frame together; "after a cut" is the full march of the
first update after a cut on a frame that leaves time standing (a capture's); "sky, whole" is the whole sky's own work
(atmosphere, clouds, environment, dome, composite, weather) by wall clock, frames with it alternating with frames
without, serialised to GPU idle.

**One cascade an update.** The shadow maps march one cascade an update, round robin, and their temporal pass carries the
other cascades' history over unchanged. Each cascade is sampled with the matrix it was marched with
(`drawn_shadow_matrices`), so a cascade one or two updates old stays aligned with its contents. Every cascade is marched
after a cut or a fresh start (the round robin restarts with every new history, so held captures repeat), and so is any
cascade the view has left: `Cascade::escapes` takes the cascade's reach (its slice of the view widened by the fade
bands, the last one out to the marches' longest ray, since they sample it past the camera's far plane), its outline
along the light clipped to the freshly fitted square, and asks whether the square it was marched with holds all of it.
takram's temporal pass already blends a march in at 1%, so the maps look the same (the review captures match to 0.03 of
a level) for about a third of the march. In alternating runs of master and the round robin (2026-10-02, load average
19–28, the one-ship battle at 2400 × 1350) the clouds took 4.1–4.2 → 3.2–3.6 ms a frame at High and 8.0–8.3 → 4.4–4.5 ms
at Ultra.

The march is most of the cost; from High up the shadow maps come next and grow as the sun lowers (longer rays through
the layers), to about 45% of an Ultra sunset. The trims take the clouds from 3.3 to 2.6 ms a frame on average at High
and from 5.3 to 3.9 ms at Ultra (the sunset 7.8 → 5.3 ms), and the first full march after a cut by a quarter to nearly
half. A capture after a cut costs one full march once; a cut in play costs nothing extra.

The reconstruction costs about 0.8 ms on every tier (about 1.35 ms before the grain rework of 2026-10-03; the
measurements per tier and scene are in the [history](../../../docs/archive/native-clouds-history.md#gpu-cost-tables)): one 3 × 3 read of the march gives the sample, the motion and the
clipping box, where master read it again for the nearest fragment and sampled the march another eight times to clip.
In play the composite adds one read a pixel, and the scene pass holding it measures no dearer (its span takes in
waiting on the reconstruction). On a cut frame, where the filter runs 5 × 5 over every pixel, that pass costs about
1 ms more (a frame that also marches every pixel, 9–30 ms); in play after a cut the filter runs, 5 × 5 and then 3 × 3,
on the pixels still gathering their first eight samples, two seconds or so.

## Run it

From `native/` with `~/.cargo/bin` on `PATH`:

```sh
cargo run -p naval-clouds --example stills --release -- [--only bridge,air2600] [--tier ultra] \
    [--time morning|noon|evening|sunset|night] [--cover 0.38] [--frames 64] [--tag name] [--moving] [--bob] [--hdr]
cargo run -p naval-clouds-sim --example timings --release -- [--size 2400x1350] [--frames 120] [--tiers high,ultra]
cargo run -p naval-clouds --example blue_noise --release          # rewrites assets/blue_noise.png (same bytes)
cargo run -p naval-clouds --example noise_compare --release -- <three-geospatial>/packages/clouds/assets
cargo test -p naval-clouds --release
```

- `stills` draws the sky alone (no sea or ships: the clouds stop at the sea sphere) from fixed views (the bridge three
  ways and looking up, 900 m, 1.35 km, 2.6 km twice, 5 km, 9 km, a dive, the chart from 40 km) through the renderer
  core, held and converged, into `.build/native/clouds/takram/<tag>/<view>.png`; `--moving` lets time run, `--bob`
  also moves the camera as a ship's chase camera does (forward at 12 m/s, pitching ±1.5° over 8 s), and `--hdr` writes
  the scene's linear radiance before the display grade (`<view>.rgbf`, f32 RGB).
- The real client: `bun native/tools/shot.ts --battle "bismarck;;fletcher:static" --time noon
  --weather partly-cloudy --world-eye=0,2600,4000 --world-target=0,2300,-10000` (see `native/docs/harness.md`).
- `timings` is described under "GPU cost".

## Open problems and follow-ups

- **Cost**: Ultra spends 2.2–5.3 ms of GPU on the clouds at 2400 × 1350, High 1.6–3.5 ms, Medium 1.0–2.0 ms, Low
  0.8–1.3 ms (the 2026-10-02 tables, in the history), the worst at sunset. Every further lever measured shows (each against takram's settings,
  in interleaved runs): Ultra's maps at 512², High's size, lose most dusk shafts over a low bank (0.6–1.7 ms more at
  Ultra); turbulence off changes the clouds' shapes (about 1–1.5 ms); Ultra's march from High's 50 m steps gives hard
  edges from the air (0.8 ms); a 32-step shadow march changes the dusk light (0.5 ms at Ultra). Five multiple-scattering
  octaves and takram's interpolated sun and sky light save nothing measurable. The reconstruction is a fixed 0.8 ms a
  frame on every tier.
- **In play a cut fills in over 16 frames** (the march about each pixel at first, the prior): a camera switch shows a
  soft picture for a moment before it sharpens, and then reaches its full sample count over two to four seconds (the
  prior and the spatial filter hold the grain off meanwhile; a fifth of the prior is still in the mean at eight
  samples, two seconds in).
- **The march's own noise is what every view pays for** (measured 2026-10-04, Medium, 1600 × 900, the fine detail of
  the picture, levels of 255). One sample of a pixel is 2–2.5 levels from the converged clouds from the sea and about
  3.5 from 2.6 km looking down; the converged clouds (16 samples) keep 0.41 and 0.63 of it, beside 2.0 and 1.35 of
  fine detail. It is undersampling along the ray (steps of 50 m and more through a medium whose detail is 20–40 m
  across), not the jitter's use: a jitter of one step rather than two, the sun's short march jittered per sample
  rather than per ray, steps cut to a fixed optical depth in dense cloud and a ray stepping back to find the cloud's
  edge (three halvings) each changed the noise by under a tenth (the last makes edges crisper, the converged detail
  1.35 → 1.58 from above, for 0.6 ms of march). Steps half as long cut the converged noise by a third from above
  (0.63 → 0.43) and an eighth from the sea, for the march's cost again (1.2 → 2.0 ms at noon from the sea, 1.6 → 3.2 ms
  from the air, 2400 × 1350).
- **A camera held still in play is softer than the same view converged**, from above most: the wind's drift moves
  every pixel a few hundredths of a pixel an update, so its history is resampled every update and holds 8 samples,
  and the picture lags the clouds (from 2.6 km it keeps 0.65 of the converged clouds' fine detail, 0.90 with no
  wind; from the sea 0.97; under temporal AA 0.68 and 0.94). Tried and left: a history carried in whole pixels with each pixel's sub-pixel remainder
  kept (never resampled) keeps more detail (0.74) but shows the samples' own noise (1.07 → 1.88 levels), which the
  resampling had been averaging over neighbours, and a longer history to pay for it (16 or 32 samples) lags the
  clouds' drift and evolution (the error a few pixels across 1.6 → 2.5 → 3.7).
- **A flight loses the clouds' detail** (120 m/s at 1.2–3 km, the same measure, temporal AA, against the mean of
  four truths): the frames keep 0.42 of the converged clouds' fine detail looking down from 2.6 km at noon, 0.34 at
  dusk toward the sun, 0.39 over a storm's deck and 0.61 of the little a cumulus 300 m ahead has, though nearly every
  pixel holds its eight samples: near clouds move against far ones along a ray, the history is one picture, and it is
  resampled every update. The prior neither helps nor hurts there. More samples an update (a march of more than one pixel in 16 while the camera
  travels) or a march that needs fewer are what would. Since 2026-10-05 a near cloud the camera travels past is drawn
  from the march as it stands where its parallax is large ("Near cloud a travelling camera passes", under the departures from takram): steadier and
  softer, not sharper.
- **A long history follows the clouds' drift but not their evolving shapes** (takram shifts the shape texture
  vertically as the weather moves, 2.4 m/s in a 12 m/s wind) or the Beer shadow maps' own lag (one cascade an update,
  blended in at 1%): a view the camera held still for seconds is a little softer at the cloud edges than the clouds
  marched at that instant (measured at 0.8–0.9 of their edge contrast), where master's grain read sharper.
  The theatre's cruise scene at 8× (`GameSky::time_scale`) is softer still: after 6 s in play its edge contrast is 0.66
  of a converged frame of the same moment, against 0.82 at 1× (High, scattered cumulus at noon, 2026-10-03). A 2-sample
  history while the clouds drift (in place of `MOST_MOVED_SAMPLES` 8) only took it to 0.71, so it was left as it is.
- **Weather that changes the clouds while the history runs** (coverage or layers drifting over seconds rather than
  cut): a pixel's mean follows within its 8 or 16 samples, faster where the change clips it.
- **Rain shafts** read at a distance as dark grey curtains under the deck, which near the horizon look like low land.
  (2026-10-20: under the storm's deck at noon they read as the downpour's grey closing in, not as land; the dusk storm
  was not checked for it. A ship in a shaft also goes from sight sooner than the simulation's lookouts lose her: the
  lookouts read the weather's visibility, not the shafts.)
- **Theatre weather changes** are new scenes (the theatre's scene swaps, `GameSky::start_in_play`), not drifts: the new scene's clouds start
  at its own weather. Carrying the old scene's look into the new one's drift would smooth them.
- **One key light**: the march follows one body, so through the switch (2.5° after sunset) the low clouds are lit by
  neither beam (the sun's left them at about 1.4° down) and only by the twilight sky; a second body in the march would
  need a second set of shadow maps. The switch itself is continuous (the beams are equal there).
- **Lightning in the quarter-resolution march**: a flash's light on the clouds is the march buffer's, upsampled by the
  reconstruction's clipping box, so its edges are a quarter-resolution picture for the moment it lasts; at a still
  capture the full march shows it whole.
- **Light shafts over the sea and ships**: the shafts' length could darken the game's fog along the shadowed part of a
  ray, as takram's aerial perspective does.
- **The weather texture tiles** every 100 km: a cover pattern repeats at that period from the chart's height.
