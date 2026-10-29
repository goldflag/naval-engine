# Lighting: daylight and the camera

## Realism

Bill asked for the most realistic light the scene can give. Five changes, each reviewed on the 20-view set
(`.build/native/takram/views.sh`: sea level at noon, morning, dusk and night in several weathers, aircraft at 0.9–9 km,
a dive, the chart), before and after:

- **One brightness scale.** The dome is graded `sky_grade::GAIN` (1.4) over the tables, and so was the clouds' skylight,
  but the sun's and moon's direct light was not: the sky stood 1.4 × over the sunlit world under it. The clouds made up
  for it with a gain of their own (`CLOUD_GAIN` 1.9, on their whole radiance, the bake's too, which washed the sea and
  lit ships' shaded sides with over-bright cloud). Now the sun and moon on the world (`SeaLevel`, so every lit mesh and
  the sea) and on the clouds (`direct_scale`) take the dome's gain, and the clouds take no gain of their own.
- **No hemisphere fill.** A mesh's ambient is the sky's environment (and the sea in it) alone, by day as by night: the
  web's fill (the map's ambient below an intensity of 4, which lifted dawn, dusk and overcast hulls) is gone. Only a
  lightning flash fills (`LIGHTNING_FILL` 1.6 × the ambient × the flash: `ShipLighting::fill`, `mesh_light_shares`).
  Ships, aircraft, land, and trees alike.
- **No cloud-shadow floor.** `CLOUD_SHADOW_FLOOR` is 0 (was 0.15): under a thick cloud the direct beam is gone, and the
  light there is the sky's, which the environment gives. Under overcast a ship is evenly lit, with no false
  directional shading from 15 % of the sun.
- **An eye that adapts both ways** (`eye_adaptation.wgsl`, `EyeAdaptationSettings`). The bake writes, in the atlas's
  alpha, the luminance each texel would have without clouds (prefiltered with the radiance), and the sky mean carries
  it in w. The eye opens against that clear sky: a view darker than a clear noon from the bridge (`reference` −0.37)
  opens by all it lacks (`strength` 1, at most 2 stops), so an overcast noon comes out near a clear one. Two limits keep
  it an eye's (2026-10-03, after a hull glowed over dark sea and ships glowed again at night): it never lifts the view's
  brightest tenth past `highlight` (1.5 stops over the clear sky's mean, a clear noon's highlights), so a sunlit hull
  filling a view of dark sea opens it 0.1–0.5 stop instead of a full stop or more; and it fades out as the clear sky
  darkens from day to night (`light_from` −3 to `dark_from` −5.5, log2 luminance), so dusk and night keep the fixed
  exposure (a dark night sea had opened it the full 3 stops). It closes against the view's own sky: only a view
  brighter than its sky (sunlit cloud tops from above) closes, by its excess (`close` 1, at most 1 stop), so looking at
  the sky keeps the fixed exposure. Since 2026-10-05 it also closes for glare by day (`glare` 1, see "Into the sun"),
  and since 2026-10-20 at night when lights fill the view ([the night eye](night.md#the-night-eye)). Since 2026-10-25 a view by day looking
  away from the sun lets its brightest tenth stand up to 0.8 × the sine of the sun's height stops more over `highlight`
  before closing for glare (`glare_away`, `sun_away`): the clouds' diffuse light made sunlit tops that much brighter,
  and the room keeps the exposure where it was, the sky above the horizon within 4 levels of 255 at noon and within 1
  at a 15° sun but from 2.6 km away from it (+5; `../clouds/march.md`, "Bright tops"); a second view's eye (the fall of shot
  window's) by its own camera. Hosts without the clear sky (the harness's sky mean, a stand-in) write w 0 and meter against the sky's own mean.
- **The sea mirrors every cloud.** With the clouds at their true brightness the reflection needs no correction (a
  bluer sea under cloud was tried and dropped: `.build/native/sea-wash/`).

## Into the sun

Bill found views into the sun washed out, with hulls and aircraft in the sun's path glowing as if emissive, and the
haze heavier than the visibility. Measured on a carrier group 420 m off, the sun 20–25° up, the North Atlantic's air at
46 km (Eastern Solomons' theatre visibility; `.build/native/sunglare/`):

- **The glow was the bloom.** The high pass takes a highlight's whole graded value and the five mips spread 0.9 of it
  up to a tenth of the view, so the glitter round a hull veiled her: bloom off, the same hull is crisp and detailed. The
  strength is 0.1 now (the web's 0.3, `native/docs/render-core/README.md`); fires and their light on the plating look the same at 0.3, 0.1
  and 0.05.
- **The wash-out was the fixed exposure.** Each metered cell counts at most 0.5 stop over the sky's mean, and the eye
  closed only for a view brighter than its own sky, so into the sun it closed 0.2 stop and the aureole, the horizon and
  the glitter sat in AgX's shoulder. The eye now closes for glare by day: the brightest tenth's excess over `highlight`
  (1.5 stops over the clear sky's mean), at most `max_down` (1). The brightest tenth reads about 1 at a clear noon, 0.75
  under overcast, 1.5 over sunlit clouds from 2.6 km, 1.75 with a sunlit flight deck in a view away from the sun (0.25
  stop), and 2.25–3.25 into the sun (0.75–1 stop). A sunset with the sun in view closes too (about a stop): the sea
  darkens and the ships go to silhouettes, as a camera's would.
- **The haze matches the visibility.** The extinction is `SIGHTING_DEPTH / V` or the map's own air where that is
  thicker (the North Atlantic's 0.056 per km is a 43 km visibility at A's calibration, so a theatre's 46 km draws as
  43). A broadside Bismarck keeps 85–90 % of her pixels over 5 % contrast out to 20 km into and away from the sun. Her
  mean contrast on screen into the sun was half of hers away from it (0.13 against 0.31 at 20 km, 0.25 against 1.2 at
  6 km): the sky behind her in the display's shoulder, not the air. Closing for glare raises her mean contrast into the
  sun by 14–19 % at every range (away from the sun nothing changes).

Since 2026-10-12 the sun itself glares (`native/docs/render-core/post.md`, "The sun's glare"): a veil about its disc by what the frame
shows of it, which the bloom adds. It is the sun's alone: the sea's glitter blooms as it did, at the strength of 0.1.

## Sun and sky at the sea

Carrier aircraft paint looked flat next to a photo of a Wildcat in flight. The sunlit paint stood about 1.5 × the sky
behind it, against 1.06 × in the photo, and the hinomaru went pink.

The readings below were taken at sea level in the North Atlantic's clear weather: turbidity 1.5, an aerosol optical
depth of 0.024 at 550 nm. Values are luminance in the sea's units with `sky_grade::GAIN`. They come from
`naval-sky/tests/light_balance.rs` (ignored; run with `--ignored --nocapture`; `TURBIDITY` and `MIE` override the air).

| Sun | Sun on a surface facing it: world before / now / tables | Sky on a level surface: tables / environment integrated / as meshes read it | Direct on a level surface ÷ sky: before / now | Physical: IES clear sky / Bird, AOD 0.1 |
| --- | --- | --- | --- | --- |
| 10° | 4.66 / 3.72 / 3.81 | 0.377 / 0.341 / 0.302 | 2.7 / 2.1 | 0.9 / 1.7 |
| 30° | 8.44 / 6.60 / 6.66 | 0.576 / 0.502 / 0.448 | 9.4 / 7.4 | 3.6 / 4.6 |
| 45° | 8.63 / 7.28 / 7.32 | 0.635 / 0.552 / 0.510 | 12.0 / 10.1 | 4.8 / 6.4 |
| 61° | 8.73 / 7.61 / 7.65 | 0.671 / 0.583 / 0.568 | 13.4 / 11.7 | 5.7 / 7.8 |
| 75° | 8.76 / 7.75 / 7.78 | 0.687 / 0.597 / 0.597 | 14.2 / 12.6 | 6.2 / 8.5 |

The physical column has two sources:

- **IES clear sky**, in illuminance (Rea 2000, the IESNA Lighting Handbook): the beam is 127.5 klx × exp(−0.21 / sin α)
  and the sky 0.8 + 15.5 √(sin α) klx.
- **Bird**, broadband (Bird and Hulstrom 1981). In illuminance it is about 0.8 × this, because the beam gives about
  100 lm/W and the sky about 125 (Perez 1990).

The open ocean's aerosol optical depth is about 0.11 (the Maritime Aerosol Network, Smirnov et al. 2009).

- **The world's sun skipped the air (fixed).** The dome, the clouds' sunlight and the environment take the tables' sun
  through the air. `model::sea_level_light`, the sun on every lit mesh and on the sea, did not:
  - From 18° up a readability ramp held it at the whole sun above the air. That is 12–27 % over the tables' sun
    between 75° and 30°.
  - It ignored the weather's aerosol: 8.75 against the tables' 7.10 at turbidity 3.

  Its luminance is now the tables' sun. The tint and desaturation are unchanged, and the readability floor holds only
  as it sets. Sunlit hulls, airframes and the glitter lose 0.17–0.33 stop of radiance by day (0.1–0.25 on screen) and
  about a quarter stop at dusk. Night is unchanged.
- **The environment keeps its energy.**
  - Level 0, integrated over the hemisphere, matches the dome.
  - The roughest level × π (`env_irradiance`) uses a Gaussian lobe, not a cosine one. It reads 3 % under the dome on a
    level surface, and 5–20 % over it on upright surfaces and the underside.
- **The dome's aureole is short.** In the clear weather `mie_gain` (`(mie × SUN_HAZE / 0.25)^0.7`) is 0.45. The tables
  take the aerosol's share out of the beam, but the dome puts back less than half of it round the sun. Its sky is
  therefore 13 % under the tables' own (`sun_sky_irradiance`); with a gain of 1 the two agree within 3 %.
- **The clear weather is clearer than the sea's air.** Even with the fix, the direct light stands about twice the
  physical ratio over the sky. At turbidity 3 (optical depth 0.09) with an aureole gain of 1, the ratio at 61° would be
  6.5 (sun 7.08, sky as meshes read it 0.95), within the physical range.
- **The exposure is about 0.8 stop brighter than a camera's.**
  - One unit is about 13.8 kcd/m², taking the tables' sun as 127.5 klx.
  - Middle grey sits at about 2.3 kcd/m², against sunny-16's 4.1 (EV 15, K 12.5).
  - A sun-facing grey at 61° reads 2.7 × its albedo (3.0 before); a camera gives 1.45.
  - The grade gain is consistent: one scale for the dome, the clouds and the sun. It accounts for half a stop of the
    difference.
- **AgX turns saturated reds pink in its shoulder, and desaturates them even at a camera's exposure.** For the
  hinomaru's albedo (190, 30, 40), sRGB out of the display:

  | Light on the paint | AgX | Punchy (AgX's power-1.35 look) | Khronos PBR Neutral |
  | --- | --- | --- | --- |
  | 1 × its albedo | (176, 58, 54) | | |
  | A camera's 1.45 × | (199, 80, 74) | | |
  | 2.7 × | (232, 120, 111) | (218, 87, 79) | (246, 60, 69) |

Proposals. The stills are in the lane's review set; the scripts are in `.build/light-balance/` (`retone.py` re-tones
a `capture` HDR offline). On 2026-10-20 the first was built (both fair weathers, at the ocean's measured aerosol) and
the second built as a trial behind a switch, with the paint at its reflectance: "Camera light" below.

1. **A hazier clear weather**: turbidity 3, with an authored mie of 0.556 for an aureole gain of 1.
   - Better: in the aircraft view the sky moves to the photo's (sRGB 109, 144, 180 against the photo's 113, 133, 168;
     today's is 89, 129, 168), and the paint stands closer to it.
   - Worse: the sea from a ship goes milky and flatter (+0.3 to +0.5 stop), and dusk loses its warm sky (−0.3 to
     −0.5 stop).
2. **Exposure down 0.8 stop** (sunny-16). This leaves the paint-to-sky ratio unchanged, and the sky goes darker and
   bluer than the photo's.
3. **Another look.**
   - Punchy: the hinomaru comes out (159, 69, 71) on screen against AgX's (171, 87, 88). It also deepens the
     shadows and darkens and saturates the sky.
   - PBR Neutral: (181, 52, 65), with a deep blue sky.
   - Either changes the look of every scene.

## The meshes' light follows the sky

Hulls and aircraft took the sun's direction, colour and strength once, when their scene was built
(`ShipLighting::daylight`), and only the lightning fill each frame after it. The sky, the sea, the land and the effects
read `GameSky::light` every frame, so a scene whose sun moved after it was built (a theatre's clock through an evening,
a battle's, the console's hour) kept its hulls under the build's sun: lit as at the hour the scene opened while the sky
and sea went dark, and shaded from the old bearing. In theatre it showed on a ship followed into dusk, and went away
when another ship was followed, since that builds a new scene. `SeaScene::update_effects` now hands both layers the
sky's light each frame (`ShipLighting::follow`).

Checked through the harness's `weather` RPC on a clear-sky Bismarck (`.build/native/relight/`): built at noon and taken
to 23:00, master's hull stays sunlit and mirrors in a black sea; with the fix the frame is the one a scene built at
23:00 draws, byte for byte. A scene whose sun stands still moves a little too (the noon frame by a mean of 0.11
levels): the build took the light before the scene's visibility was set (`GameSky::set_visibility`), so hulls had the
sun without the haze's share of the beam, which the sea had.

## Camera light

Bill asked for the light to land where a camera's photograph puts it, by physics rather than by offsets that cancel
(the photoreal batch's light-balance lane). Two changes by default, a storm's dusk mended, and a trial behind a switch.
Stills: `.build/native/look-light-balance/<set>-<view>-before.png` (master 8fbabd439's release client), `-after.png`
and `-trial.png` (this branch's development client), from `shots.sh` there; region measurements with `region.sh`, the
eye's readings with `eye.sh` (`eye-master.txt`, `eye-branch.txt`).

### Marine air

The fair weathers take the open ocean's aerosol, each with all of the aureole the tables' aerosol scatters (an aureole
gain of 1: authored mie 0.556 under the client's `SUN_HAZE` of 0.45), where the clear weather put back 0.45 of it and
partly cloudy 0.72 (`assets/maps/battle-conditions.v1.json`). The model's aerosol optical depth at 550 nm is
`(0.003996 + 0.027 e (1 + (e / 5)²)) / 0.9 × 1.2`, e the turbidity less 1.

| Weather | Turbidity, optical depth: before | After | Source |
| --- | --- | --- | --- |
| Clear (35 km) | 1.5, 0.024 | 2.2, 0.051 | The clean end of the open ocean's: 0.05–0.07 at 500 nm at remote islands and over the Southern Ocean (Smirnov et al. 2002, J. Atmos. Sci. 59, 501–523; Smirnov et al. 2009, the Maritime Aerosol Network, JGR 114, D06204) |
| Partly cloudy (25 km) | 2.2, 0.051 | 3.4, 0.112 | The open ocean's mean, about 0.11 at 500 nm (Smirnov et al. 2009; 2011, AMT 4, 583–597) |
| Overcast, fog, storm clouds, the maps' own | unchanged: 3.8 to 6, 0.14 to 0.37 | | |

0.024 is cleaner than any air at sea level: the sun stood about twice the physical ratio over the sky ("Sun and sky at
the sea" above). Clear took 0.05 rather than the 0.07 first tried: at 0.07 a clear dusk's warm band at the horizon went
grey ((109, 93, 85) to (87, 81, 85) on screen, `clear26/`), at 0.05 it keeps some of it ((95, 86, 86)).

A view still sees half of the tables' air on a fair day (`../haze.md`, "Clear air aloft"): `model::aerial` runs a fair day
to turbidity 3.4 (it was 2.5 to 4.5), so the overcast's 3.8 sees 0.87 of the air (0.86 before), the storm and the fog
all of it, and the port's own sky (3.2) half where it saw 0.64 (its straight-down view's sea 4 levels deeper). The haze
still closes each weather at its visibility: more of what the eye loses at the sea is the tables' aerosol, less the
250 m haze layer.

What it does, the mean sRGB of a region (L its luminance):

| View | Before | After |
| --- | --- | --- |
| Clear, from a ship (`sun-45_6_160`): sky, sea, hull | 160, 51, 115 | 160, 49, 113 |
| Clear, straight down from 700 m (`sun-0_88_700`): the sea | (35, 65, 93) L 61 | (40, 68, 94) L 64 |
| Clear, from 1,500 m (`sun-30_12_1500`): the sky above, the sea below | 157; (10, 31, 51) L 28 | 176; (21, 45, 67) L 41 |
| Partly cloudy, into the morning sun (`day-chase`): the horizon, the sea | 161, 50 | 167, 60 |
| Partly cloudy, from 1,500 m (`day-30_12_1500`): the sky, the sea | 154, 42 | 172, 51 |
| Clear dusk (`dusk-chase`): the band at the horizon | (109, 93, 85) | (95, 86, 86) |

From a ship it hardly shows; from the air the sky pales and the sea below takes a veil, as the sea does in photographs
from aircraft; toward the sun the sea is milkier; a clear dusk's band is less warm. The eye meters a view against the
clear sky's mean, which the aerosol brightens half a stop at noon (log2 −2.43 to −1.92), so a clear noon opens it 0.40
stop where it opened 0.10: of a clear noon's sea, 14 levels lighter, about half is the eye's.

### Storm light

The dusk storm frames (`storm-*`: storm clouds, the dusk preset's sun 3° up) were near black, the Kongō a flat brown,
and a sun showed through the deck. Three causes:

- **The eye stayed shut.** It opens for what a view lacks against the clear sky's mean, as far as 2 stops, and faded
  that out as the clear sky's luminance fell from log2 −3 to −5.5: a stand-in for the sun's setting that more than the
  sun moves. The weathers' `sunScale` (storm 0.3, fog 0.5, overcast 0.55) scales the sun above the air, the clear sky
  with it, and the sky's twilight lift brightens it about sunset. Under the storm at 3° the clear sky read −5.51, darker
  than a clear night's under the moon (−4.97): the chase view metered −2.18 (the reference is −0.37) and kept the fixed
  exposure, and the night grade came full on. Both now follow the sun the sky draws (`naval_render::post::daylight`;
  naval-client sets `Renderer::set_sun_elevation` each frame from the drawn sun): the eye may open fully from 2° up and
  not at all from −4° down; the night grade fades out from −4° to 10°. Hosts that give no sun keep the luminance gates.
- **The low sun lit the hull and the sea under the deck.** The clouds' ground shadow faded in with the sun from 1° to
  7° up (`naval_clouds::layer::SHADOW_SUN`), so at 3° a closed deck held back a quarter of the beam: the hull took the
  setting sun's light (the flat brown) and the sea its glitter. It now fades in as the sun's own light at the sea does,
  from 1° under the horizon to 2° up (`model::sun_grade::SETTING`): wherever the sun lights the sea, the clouds shade
  it.
- **The sun through the deck was the eye's glare.** With the bloom off it goes: no disc reaches the frame. The glare
  (`native/docs/render-core/post.md`, "The sun's glare") measures the disc's excess over the sky about it against the disc's own
  radiance, and lit its veil with the world's sun light. At 3° in the storm's air (optical depth 0.24) the disc keeps
  about e^−6 of the sun above the air, while the world's light is held at the readability floor, some forty times the
  sun's own through that air: rain streaks crossing the disc (bright, at clear depth) measured as a disc in full, and
  the veil drew a bright sun. Now the veil and photo mode's flare take the sun's own light (`model::sun_own_share`,
  `Sky::sun_own_share`, `photo::flare_sun`; a sun the floor does not hold glares as before), and a sample more than half
  covered by a transparent (a rain streak's core, thick smoke: the scene's alpha, which the night grade's cover pass
  keeps) shows no sky (`sun_glare.wgsl`).

The eye on the chase view (`eye.sh`: Kongō against Iowa, wind 9 m/s, one second), master against this branch:

| Time, weather | Clear sky, log2 | Metered | Stops open: master | This branch |
| --- | --- | --- | --- | --- |
| Noon, clear | −2.43 / −1.92 | −0.47 / −0.77 | +0.10 | +0.40 |
| Dusk, clear | −4.25 / −4.19 | −0.49 / −0.64 | +0.12 | +0.27 |
| Noon, overcast | −2.04 | −0.72 | +0.35 | +0.35 |
| Noon, storm | −2.37 | −2.27 | +1.90 | +1.90 |
| Dusk, storm | −5.51 | −2.18 / −2.31 | 0 (shut) | +1.94 |
| Dusk, overcast | −4.70 | −1.27 / −1.40 | +0.48 (held by the gate) | +1.03 |
| Morning, partly cloudy | −2.64 / −2.35 | +0.43 / +0.30 | −0.75 (glare) | −0.50 |
| Night (−28°, full moon), clear | −4.97 | −0.72 / −0.65 | +0.23 | 0 |

The night grade's weight and the eye's gate against the sun's elevation in the clear weather, by the clear sky's
luminance before (log2 measured at each, `curve-master-clear.txt`) and by the sun now:

| Sun | 10° | 5° | 3° | 1° | 0° | −2° | −4° | −6° | Night, moon up |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Clear sky, log2 | −3.36 | −3.87 | −4.25 | −4.03 | −4.01 | −4.36 | −5.15 | −5.81 | −4.97 |
| Night grade before / now | 0.06 / 0 | 0.28 / 0.29 | 0.50 / 0.50 | 0.37 / 0.71 | 0.36 / 0.80 | 0.57 / 0.94 | 0.95 / 1 | 1 / 1 | 0.88 / 1 |
| Eye's gate before / now | 0.94 / 1 | 0.72 / 1 | 0.50 / 1 | 0.63 / 0.93 | 0.64 / 0.74 | 0.43 / 0.26 | 0.05 / 0 | 0 / 0 | 0.12 / 0 |

The twilight lift brightened the clear sky about sunset (0° read brighter than 3°), so the old night grade dipped there;
by the sun it rises steadily. At the dusk preset, the deep twilight and the day the two agree. True night stays as dark
(`night-*`, clear, full moon): the frames move by a level (36 to 35 from the chase, the sky 38 to 35), the night grade
rising from 0.88 to 1 as the eye's opening falls from +0.23 to none.

On the storm's views (8-bit means): the chase's frame 6 to 24 (its sky 9 to 40, its sea 2 to 8); from the quarter,
6 to 23, the hull side from (27, 18, 11), brown, to (25, 20, 21); toward the sun (`storm-110_4_300`) the frame 25 to
35, and where the sun shone through the deck, (155, 126, 96), the cloud's own glow, (80, 67, 63), with no disc. A clear
dusk's chase frame is 64: the storm's dusk stays about 2.5 stops under it, the eye's 2-stop limit and the storm's
`sunScale` between them (its clear sky, which the eye meters against, reads 1.3 stops under the clear weather's at
dusk: the scale's 1.7 less what its hazier air adds).

Left as they are:

- **The weathers' `sunScale`** double counts the clouds: the deck already takes the light. It stays because the light
  under the decks is too bright without it. With `sunScale` 1 an overcast noon's frame would stand a stop over a clear
  noon's and a storm noon's level with it, and the eye, which opens against the clear sky of each weather's own air
  (turbidity 3.8 to 5, whose clear sky is 1.2 to 1.8 stops brighter than the clear weather's), would lift both further.
  Taking the scale out wants the eye's reference made the same air's for every weather, and its closing for views
  brighter than that reference, as a camera's: it moves every overcast, storm and fog scene.
- ~~The hull's brown cast and the funnel smoke's orange under a deck~~: mended 2026-10-20 ("Under a deck" below).

### Camera exposure

Bill took the trial (`NAVAL_CAMERA_EXPOSURE=on`, built with the light-balance lane) as the game's, with the eye's
reference moved back for the marine air. The switch is gone; nothing published changes (not `albedoScale`, not
`paints.json`, which is in every ship's finish fingerprint, not a model).

- **Paint at its reflectance.** A draw whose colour is a catalogue paint's (`paints.json`'s ids: not a ship's own
  paints, not a recipe's rope, bronze, glass or openings; an image of her own paint whose texels are the catalogue's
  swatches, `paint::is_catalogue_painting`, as Fletcher's, Fubuki's and Yukikaze's hulls are, since 2026-10-27) takes
  1 / `albedoScale` (× 1.85) as a fleet uploads her
  (`ShipPreset::paint_gains`, `preset::restore_reflectance` in `PresetGpu::new`): in its vertices' colour for painted
  and teak draws, in its material's for plain ones (gun covers' canvas, the fleet's material table), in the thin
  members' impostor strips. What a preset keeps and a pack bakes stay the paints as published, which her weathering
  and markings were laid out from; an `auto` marking judges the paint as published (`ship.wgsl` hands the
  markings the paint over its draw's gain, `PartMaterial::paint`), so Hood's 507B keeps white ink and a merchant's own
  grey its black. `albedoScale` folds into `paints.json`
  (the swatches at their reflectance, the scale 1, the gain gone) at the next whole-fleet rebuild
  (`docs/ship-appearance.md`, "Brightness").
- **The eye a camera's meter.** A camera exposes a view at sunny 16 times its meter's middle grey over the view's mean
  luminance; the eye exposes at the fixed exposure times 2^(reference − metered). They agree when the reference is
  log2(`CAMERA_EXPOSURE` × L_K / L_clear) (`naval_render::post::METER_REFERENCE`): a meter calibrated to ISO 2720's
  K = 12.5 reads EV 15 at ISO 100 from 4096 cd/m² (Frostbite's auto exposure takes the same calibration: Lagarde and de
  Rousiers 2014, "Moving Frostbite to physically based rendering"), 0.297 scene units; a clear noon's clear-sky mean is
  0.263 (marine air, the sun 70° up); `CAMERA_EXPOSURE` is 1.45 / 2.7 ("Sun and sky at the sea"). log2(0.537 × 0.297 /
  0.263) = −0.72, where the reference was −0.37, set by eye. That move takes out both the marine air's brighter clear
  noon (the eye opened 0.40 there, 0.10 before it) and the camera's −0.9 stop, each once: in every view the eye meters,
  the exposure is the camera meter's.
- **The fixed exposure stays the game's.** Lowering it to a camera's as the trial did (and the reference to +0.17
  with it, the highlight to 2.4 and the eye's reach to 2.9 stops, the same places on the display) gives the same
  exposure wherever the eye opens, and 0.9 stop less wherever it rests: at night (the eye does not adapt into the dark),
  with Graphics → Eye adaptation off, under water, and in views brighter than a meter's middle grey (the sky alone,
  into the sun, cloud tops from aloft). Tried on the lane's first build: the night's frame went from 36 to 18 (8-bit
  mean), a telephoto under the clouds' haze from 148 to 110. Those are tuned looks the trial never showed, and the night
  lanes tune their lights against the night's; Bill's to take, by `DisplaySettings` and those four constants.

The eye on the chase view (`.build/native/look-calls/eye.sh`: Kongō against Iowa, wind 9; master 7ca6daeba797's
release client against this branch):

| Time, weather, view | Metered: master / now | Stops open: master / now |
| --- | --- | --- |
| Noon, clear, chase | −0.77 / −0.75 | +0.40 / +0.03 |
| Noon, clear, 160 m | −1.12 / −0.98 | +0.75 / +0.26 |
| Morning, partly cloudy, chase into the sun | +0.30 / +0.21 | −0.50 / −0.50 (glare) |
| Noon, overcast | −0.74 / −0.84 | +0.37 / +0.12 |
| Noon, storm | −1.97 / −2.09 | +1.60 / +1.37 |
| Dusk, clear | −0.64 / −0.62 | +0.27 / 0 |
| Dusk, storm | −2.29 / −2.46 | +1.92 / +1.74 |
| Night, clear (full moon) | −0.65 / −0.65 | 0 / 0 |
| The port's opening view | −0.32 / −0.33 | 0 / 0 |

A clear noon now stands where it stood before the marine air (the light-balance lane's master, ca1bfbf03ca8): sky,
sea and frame (8-bit means) at the chase 163, 52, 116 then, 178, 65, 130 on master, 164, 53, 117 now; at 160 m 172,
67, 115, then 186, 81, 129, now 169, 64, 114; straight down from 700 m the frame 75, 98, 81. The hull side at 160 m
reads 56 then, 70 on master, 73 now: the paint at its reflectance, under the same sky and sea as before the marine air.
In the clear morning from 160 m (`sun-45_6_160`) the sky is the brightest tenth and holds the eye where it was: sky
and sea are unchanged (162, 47) and the hull goes from 123 to 140. The superstructure close up reads 6 to 9 levels
brighter. Overcast and storm views come out 0.2 to 0.4 stop darker, the eye opening less against a darker sky
("Under a deck"). Night and the port are unchanged. Stills: `.build/native/look-calls/<set>-<view>-before.png`
(master 7ca6daeba797's release client) and `-after.png` (this branch's development client), from `shots.sh` there;
`-prelb.png` from ca1bfbf03ca8.

The audit of what was set by eye under the game's exposure still holds, at the paint's gain against everything else
(the eye takes the overall exposure from the scene): ships stand 0.9 stop brighter against what is not set from the
catalogue.

| What | How its brightness was set | Against the paint now |
| --- | --- | --- |
| Sky, clouds, the sea seen from above | Physical: the tables, the march, the water's optics | Physical |
| Aircraft paint | Measured relics and charts, no `albedoScale` (`assets/aircraft/build.py`) | Physical |
| Satellite land, cliffs | Calibrated to the ground's reflectance (`../terrain/surface.md`) | Physical |
| Ships' weathering (rust, soot, salt, bare steel), markings' inks, scorch | Absolute albedos (`weathering.wgsl`, `markings.rs`), the levels read off War Thunder and World of Warships as ratios to the scaled paint | Darker against the paint: streaks read heavier, salt and white marks duller |
| Ships' own paints, recipe rope, bronze, glass, openings, canvas | Recipe colours, by eye | 0.9 stop darker against the paint |
| Land styles (a theatre window's land beyond its photographs), trees, buildings, harbour works | Web palettes toned by eye (`LAND_BRIGHTNESS` 0.86, buildings × 0.9) | 0.9 stop darker against the paint |
| Foam, whitecaps | `FOAM_ALBEDO` 0.75, the top of the measured 0.55–0.75, chosen so it read white beside the scaled white paint | Physical; may read grey beside restored white |
| Fire, flashes, explosions, night lamps | Emission in the sun's units, tuned against the display; since 2026-10-20 lifted with the night as the lamps' real candela are ([the night eye](night.md#the-night-eye)) | Unchanged by day |

### Under a deck

The hulls' brown cast under a storm's or an overcast's deck and the funnels' orange smoke were one fault: the sun lit
what the clouds shade.

- **The environment's air.** The clouds' environment bake (`naval-clouds`' `hooks.wgsl`, `clouds_bake`) put the
  atmosphere between the sea and the clouds' front over the clouds with takram's aerial perspective, sunlit the whole
  way. Under a deck the rays near the horizon run 15 to 50 km under it, and at a low sun that air glowed warm: the
  environment's horizon, which a hull's side takes half its light from, stood brighter and browner than the deck. The
  bake now takes the air under the clouds at the share of the scene's light they let through to the sea about the
  camera (the mean the bake already takes for the sea's glitter, `clouds_bake_light` at 64 points within 1.5 km): its
  aerial perspective sunlit and in shadow (takram's shadow length the whole way, as the dome takes a light shaft's),
  mixed by that share. Under broken cloud a part of the air is lit; under a deck almost none.
- **The environment's sea.** The bake's sea below the horizon mirrored the clear dome (`environment_bake.wgsl`'s
  `sea`): under a deck a clear dusk's warm horizon, at a Fresnel near one, in the light a hull's lower side and her
  overhangs take from below. It now mirrors the clouded sky, the same march along the mirror direction. The water
  body's light from the sky (1 to 3 % of it) still takes the clear dome's.
- **The smoke.** The effects' smoke took the sun's light through hulls and other smoke, not through the clouds:
  `native/docs/combat-effects/renderer.md`, "Clouds on smoke". It now takes the clouds' shadow as the hulls and the sea do.

At a storm's dusk (`storm-300_1_140`, 8-bit means) the hull side goes from (20, 13, 14) to (13, 11, 16), grey; the
plume from orange to dark grey, as soot under a deck. At an overcast's dusk the hull is grey, its plume dark. At a
storm's noon the plume, lit beige by a sun the deck had stopped, is black against the grey, as coal smoke under
overcast photographs. The environment's mean (the sky mean the eye meters against its own sky) under the overcast
noon goes from 0.341 to 0.232 and under the storm from 0.140 to 0.051: the deck's light, not the sun's in the haze
below it. The views under a deck come out 0.2 to 0.4 stop darker (overcast dusk chase 42 to 34, storm noon 112 to 99,
storm dusk chase 20 to 15): the eye opens as it did against the clear sky, and what it opens on is darker. The storm's
dusk stays darker than a clear dusk (Bill, 2026-10-20). Left: a yellow band on the sea along the hull at an overcast's
dusk, in master's stills too (the submerged hull, lit through the deck's gaps).

### What it costs

The light balance itself adds no pass and no texture: the eye's uniform holds one more float, the display's one, the
glare's measure reads the alpha it was reading the colour of, and the client sets the sun's elevation each frame. The
paint's gain is a pass over a preset's vertices as a fleet uploads her.

Under a deck (2026-10-20), on High at 2592 × 1545 on the M5 Pro, the chase view of a Kongō against Iowa duel, 180
frames each, the branch against its merge base's release client (ebb8b04ba7db) alternating, load average 6 to 14
(`.build/native/look-calls/cost.ts`): the environment bake's pass (`sky environment`, which sweeps its rows in bands
over several frames, so this is its mean a frame) 0.03 → 0.05–0.06 ms in a storm, 0.04 → 0.05 by day, 0.02 → 0.04 at
night: the sea's rows march the clouds along their mirror direction now. The smoke's clouds' shadow is four reads a
vertex; the merged smoke pass measured 0.33–0.38 ms on both when the GPU was quiet. GPU busy p50 12.2–12.5 ms on
both in the storm when quiet; the runs swung to 21–26 ms on either build when another session's client took the GPU.
