# Waves and the swell

The wind sea the FFT cascades draw, the swell the hulls ride drawn over the whole sea, the gusts and the cascades'
tiling. Index: [README.md](README.md).

## Sea parameters

`calibration::scene_parameters(map, wind, port)` does what the web's `VisualEnvironment.applySea/applyFog` (historical)
did:

- **Waves**: `windSea(map, wind)`: significant height and peak wavelength interpolated from
  `seaCalibration.samples` (× the map's amplitude and wavelength scales), choppiness
  0.65 + 0.9·clamp((U − 3)/12); γ 2.6, directional sharpness 0.8, seed 1941 (the web's `Game.ts` (historical)); wind direction
  the map's (the port's 35°). With the realistic sea state (on) the field redraws it as a real wind sea:
  peak 70 m at 9 m/s, 146 m at 15, 298 m at 25, Donelan's γ, choppiness 1 under 3.4 m/s rising to 2.25 by 8 (since
  2026-10-21, below; was 2.25 at every wind).
- **Wind by weather preset** (`battle-conditions.v1.json`): map 9, clear 5, partly cloudy 9, overcast
  12, fog 5, storm clouds 16 m/s, × the map's wind scale; the port's standing wind is 9 m/s.
- **Foam**: crest opacity 1, wind stretch 0.5, coverage scale = map foam / 0.45 (1 on the North
  Atlantic), lifetime 1 period; windrows opacity 0.5 over `windrowCoverage(U, scale)`; shoreline
  `#edf9fd` at 0.8.
- **Water**: absorption the map's `absorptionColor` (sRGB hex → linear); the physical water body uses
  only it. Pigment and transmission colours pass through for the crest light and the underside.
- **Fog**: port `#819aa5`, 650–5,600 m, power 0.85, sky blend 2,600 m; battle the map's fog with
  power 1.4 (the ramp; real extinction is off in the game).
- **Time of day and cloud**: reach the sea only through the sky (light and environment).
- **Quality tiers**: `QualityTier` (Low 1 × 256², Medium 2 × 256², High 3 × 256², Ultra 3 × 512²;
  clipmap segments 16/32/64/128 on the web, 32/64/128/256 with finer levels since 2026-10-04, [mesh.md](mesh.md)), as `quality.ts` (historical).
  The stills use High.

## The swell: the sea the hulls ride, everywhere

The drawn sea was a pure wind sea whose longest, highest waves repeated on the first cascade's tile, and the long swell
combat poses hulls on (naval-sim's `SeaState`: two long-crested sines, 0.7 of the amplitude at four times the calibrated
peak wavelength along the wind, 0.3 at 0.57 of it turned 0.8 rad) showed only within a hull length or two of the 16
nearest hulls (`native/docs/sea-sky.md`, "The sea around hulls"). Now the realistic sea draws that sea everywhere
(`HullSeaUniforms::set_everywhere`, set by `WaveField::couple`; `waves.wgsl` takes the coupling's weight as 1 at every
point): the first cascade's long waves give way to the sines across the whole surface, the shorter waves, foam and
wakes riding on them as beside a hull.

- **Why the sim's sea and not a swell of the client's own**: hulls must sit in the water they ride, and their poses are
  the simulation's. A swell drawn only by the client would move the water under every hull without moving her; one added
  to `SeaState` would change heave, roll, flooding, shells' sea height and gunnery in every battle. The sim's first sine
  already is a swell: 128 m (9.1 s) at 9 m/s with 1.17 m crest to trough, 171 m at 12, 239 m (12.4 s, 3.6 m) at 18,
  where the wind sea's own peak is 70, 100 and 197 m.
- **What it changes**: every hull at any range (not only the nearest 16) sits in the water she rides; the open sea
  carries a long swell to the horizon (short-crested away from the hulls since 2026-10-05, below); the tile's long
  waves, its most visible repeat, are not drawn. The
  drawn significant height is the field's shorter waves with the sines on top, as beside a hull before: 2.4 m at 9 m/s
  (the field alone 1.8), 3.5 at 12, 6.7 at 18, 9.9 at 25 (a wind sea with a swell under it). The art-directed sea is
  drawn as given.
- **Cost** (RTX 5070 Ti, Vulkan, 2560 × 1440, High, `water-cost.ts`, three rounds, with the reflection change below, the
  GPU shared with other sessions): the water pass chase 0.678 → 0.695 ms, bridge 0.485 → 0.478, low 0.431 → 0.419, close
  0.634 → 0.672, overhead 0.689 → 0.675.

**Left as it is: the storm reads as chop.** At 18 m/s the calibration's height (5.5 m, plausible for a fetch-limited sea;
fully developed would be about 8 m) goes to the sines (3.6 m crest to trough over 239 m and 1.5 m over 136 m: steepness
0.015, a swell's) while the field's waves below the split (under about 120 m, near its 197 m peak) keep 3.8 m of height,
steep and sharp-crested: busy chop on a gentle swell. Two sines never make a wave higher than 0.93 Hs (2A) anywhere,
where a random sea's highest of a few hundred is about 1.8 Hs; the groups of big waves are what reads as heavy weather,
and hulls must ride them too, so they belong in naval-sim's sea. Its response there (`SeaState::response` as written,
before the hydrostatics' lag): Bismarck with the sea along her length heaves ±0.23 m and pitches ±0.5° (the 239 m sine is
her own length, so it cancels along her), across it ±1.9 m and rolls ±3.4°; a Fletcher along it pitches ±2.8°. Not
changed: gunnery, flooding and resistance are tuned on that sea.

## The open sea's swell is short-crested

With combat's sea drawn everywhere (`HullSeaUniforms::set_everywhere`), its two long sines crossed the whole sea as one
even corrugation, ruled straight to the horizon. The first fix (#1007, 2026-10-04) gave each sine groups (a value noise
on its height, 3 wavelengths along its run and 7 along its crests) and wandering crests (a noise on its phase, turning
them up to about 20°), but from a few hundred metres up the owner still read the open sea as geometric and tiled: the
crests still ran parallel for several wavelengths, and the noise's cells, laid along and across them, showed as blocks.

Now away from the hulls each sine is one of a spread of components, as a real sea is a spread of waves and not one
(`hull_sea::open_sea_swell`, which `shaders/waves.wgsl` reads as `HullSea.swell`). Since 2026-10-21 the spread is 24 and
8 on uneven turns (below, "The realistic sea from the air, in a light air, and its gusts"): the even turns described next wove a lattice seen from a
kilometre up. The first sine (0.84 of the energy)
has 12 more, the second 4. Their directions are spread evenly within ±0.8 rad (46°) of their sine's, their wavelengths
0.70–1.42 times its own on a golden-ratio sequence, their phases scattered; each runs at its own speed on the
simulation's clock and holds an equal share of its sine's energy. Their interference gives crests a wavelength or two
long and sets that build and fade, with nothing ruled or repeating. Near the hulls (`swell_freedom`, unchanged) the
spread hands its energy back: a sine's amplitude is √(1 − f²(1 − share)) and the spread's f, so the variance holds at
every distance and each hull rides combat's sea exactly (`swell_height` on the CPU, its tests in `hull_sea.rs`;
`tests/hull_heights.rs` on the GPU).

- **How it was found and judged**: stills from 150 to 1,200 m up at 9 m/s, clear, noon (`bun native/tools/shot.ts
  --battle "bismarck;;fletcher:static" --range 20000 --weather clear --wind 9 --time noon --views <views.json>`), each
  image row's mean removed and the sea's contrast stretched. With the open sea's swell set to zero the lines vanished,
  which pinned them on the swell. A spread of ±29° and 0.78–1.28 times was tried first: shorter crests, but still lines
  across the view from 900 m up; ±46° reads as a natural sea at every height.
- **Its highest waves**: eighteen components can stack into a set higher than two sines' 0.93 Hs, as a random sea's
  do; the hulls still ride the two sines.
- **Cost**: 16 more sines per vertex and per pixel of the water in place of the noise's four reads; not measured
  separately.

The hex tiling of the 181 m and 31 m cascades (PR #1004) was reverted on 2026-10-04: the lattice it removed was not
what the owner saw, and it cost 0.2–0.6 ms.

## The realistic sea from the air, in a light air, and its gusts

Bill found that the realistic sea state (`OceanParameters::sea_state`, on in the game) tiled the sea seen from high up
and made a light air's water look cheap, and asked how real the gusts were. Captures of one frozen moment with the
switch on, off, and on with the gusts off (North Atlantic, clear, noon; views 600 m and 1.5 km straight down, 1.2 and
3 km up looking out, 80 m and 25 m low; 3, 6, 9 and 15 m/s) pinned each on its cause:

- **The tile from the air was the swell.** With the realistic sea on, the sea the hulls ride is drawn everywhere ("The
  swell: the sea the hulls ride, everywhere" above), its two sines split into a spread away from the hulls ("The open sea's swell is short-crested"). That
  spread was 12 + 4 components at evenly spaced turns within ±46°, and evenly spaced crests interfere into a regular
  lattice: chevrons across the far field from a kilometre up, gone with the switch off (no swell drawn), there with the
  gusts off. `hull_sea::open_sea_swell` now spreads each sine into 24 and 8 (`SWELL_SPREAD`) on a raised cosine about
  its heading out to ±1.2 rad (`SWELL_TURN`; a deviation of 25°, as the even ±46° had), turns, lengths and phases each
  from its own golden-ratio sequence, so neither falls on a grid. They interfere into wave groups. Offline (the spread's slopes summed over 4 km), sixteen on
  uneven turns still drew long bands, 24 began to break up and 32 was clean. The hulls ride the two sines as before.
- **The rest of the tile was the gusts**: value noise puts the same oval on every lattice point, so from 3 km the glitter
  showed one stamp repeated on a grid (switch on or off).
- **A light air was crinkled.** `sea_state::drawn_sea` drew every sea at choppiness 2.25 (`BREAKING_CHOPPINESS`, a crest
  at Stokes' limit cusps), at every wind. At 3 m/s no crest is near breaking (Beaufort 2: "crests of glassy appearance"),
  and every wavelet came out pinched, an even crinkle at every distance; off drew 0.65, which is why off looked better
  there. Crests now start linear (1.0, `LINEAR_CHOPPINESS`) and sharpen to the limit as the sea starts to break, over
  Beaufort 3 to 5 (3.4 to 8 m/s, `sea_state_choppiness`): 1.75 at 6 m/s, unchanged from 8 m/s, so the default 9 m/s sea
  draws as before. Tried and left: holding the ripples to Cox and Munk's clean less slick slope at the wind (the ripple
  layer is twenty times the tail at 3 m/s, but shows only within metres of the eye: 0.03 % of a bridge view's pixels
  moved); scattering the finest cascade as the coarser two are (its 31 m repeat measured 0.036 of correlation straight
  down at 3 m/s and was hard to see).
- **The gusts.** Their size was right (patches 0.4 to 4 km, stretched downwind, as the wind's cells in radar images of
  the sea, about 0.3 to 4 km). They were 3 to 4 times too strong: at 9 m/s the roughest and smoothest twentieth of the
  sea differed as much as 4 and 17 m/s of wind would on Cox and Munk's law, where the open sea's wind varies some 10 to
  15 %. They drifted at 0.3 to 0.6 of the wind, where eddies ride it, and roughened every wave under 8 m, which takes
  some 20 minutes to answer a wind (a 1 m wave under a minute: Plant 1982), far longer than a gust takes to pass. Now:
  gradient noise (`noise::gradient_noise`, Perlin's), each layer's lattice turned off the wind (12°, −9°, 5°), drift
  1.15, 1 and 0.9 of the wind, strength 0.25 (0.4 in a light air, 0.1 in a gale; was 0.5, 0.65, 0.2), waves under 2 m
  (`GUST_WAVELENGTH`). At 9 m/s the patches now span 7 to 11 m/s worth of roughness. Gradient noise's rarer peaks (3.3
  deviations a layer, where value noise stopped at 2.3) made patches rough enough to catch the noon sun far off in a light
  air, a row of white flecks a few kilometres out; the sum is held softly within 2.5 deviations (`GUST_REACH`, a tanh),
  and the gain's mean correction takes the held sum's variance (`GUST_HOLD_LOSS`: v / (1 + 0.27 v), within 1 %), so its
  mean stays 1 (the Gaussian one left it 0.7 % low at 0.25 and 12 % low at 1; Codex review). The console's Water chips
  still set all four.

**Cost** (Apple M5 Pro, High, 2592 × 1545, `shader-ab.ts` with a hot-reload `dev-client`, 32 against 16 components
summed, two runs of three rounds, load 21–24): the water and sky pass +0.26 to 0.52 ms from 420 m above a ship, +0.06 to
0.15 in the chase view, +0.07 to 0.20 from the bridge, of 5.3 to 8.2 ms. The noise and the choppiness cost nothing
measurable. Taking the root and the power out of each component's surface (the compiler already had) saved nothing.

Captures: `.build/sea-state/cap/final/{before,after}-w{3,6,9,15}-*.png` (before from master `a179273572`'s client, after
from the branch, `--no-wildlife`); the rig is `shot.ts --water` (`native/docs/harness.md`, "Staging a shot"). Not done: a swell
in a light air (the simulation's sea is all wind sea, so a 3 m/s day has almost none, where the open ocean nearly always
has one: a simulation change, moving heave, roll and gunnery); cat's paws (in a light air real gusts are metres to tens
of metres and last seconds; ours are the kilometre layers); the finest cascade's repeat.

## FFT patch repetition

With gusts disabled, overhead views into the sun exposed repeated wave patches. Scattering only the middle
cascade's 181 m tile reduced one repeat but left visible repetition in the after picture. The initial normal-field
regression passed because it deliberately retained the longest cascade's periodic sampling; it did not establish
that the finished surface was free of repetition.

The follow-up isolated the remaining contribution in a frozen native scene (9 m/s, 16:30, seed 1941). Suppressing
the longest cascade's slopes removed its 1,024 m image correlation; suppressing fine waves, ripples, mesh displacement
or the first cascade's filtered variance did not. Scattering the longest cascade reduced that correlation too,
without removing its wave detail (`.build/native/ocean-tiling/detail-*.png`).

`waves.wgsl` scatters both coarser cascades: a rotated triangular grid chooses three independently shifted copies,
with cubed barycentric weights normalized so their squares sum to one. Displacement, slopes and strain keep their
variance; foam, bubbles and each copy's filtered slope variance mix by the squared weights, keeping their means.
The surface uses explicit texture gradients from the pixel's world footprint, so a triangle boundary's random UV
shift cannot select a coarse mip and draw a seam. Vertex displacement, inverted sea heights and spindrift use the
same copies. The first cascade's hull-sea lowpass uses those copies too: subtracting the original periodic field
would otherwise leave a mismatched wave beneath the hulls. The finest cascade and ripple reuse retain their original
sampling. Low scatters its one cascade; Medium, High and Ultra scatter their two coarser cascades. This adds reads to
the longest cascade; its cost has not been measured separately.

The GPU regression (`tests/tiling.rs`) now renders the complete sunlit surface, including its horizontal Jacobian
and filtered roughness, with gusts disabled. In a fixed overhead view, the previous fix's 1,024 m highlight correlation
was 0.208; the two-cascade sampler gives 0.008 and 0.013 along the two axes. The 181 m correlations are 0.015 on both
axes. The test requires absolute correlation below 0.15 and nonzero wave contrast. The field-level check uses jittered
samples over eight tiles, avoiding the old regular lattice's biased energy reference: slope-energy ratios are 0.978
and 1.003, within the required 15% of the original. Matching native captures at 70, 700 and 1,000 m use the frozen
reproduction, with gust strength zero (`.build/native/ocean-tiling/corrected-*.png`).
