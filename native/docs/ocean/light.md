# Light on the sea

The sky's environment the surface reads, the screen-space reflections, the sun's glitter, light through the waves,
ships' shadows and the scene's lamps on the water. Index: [README.md](README.md).

## The sky's environment

The surface binds the sky's environment (`native/docs/sky.md`, "Environment lighting (the interface ships and the ocean read)") at group 3: binding 0 the `Rgba16Float` atlas in three's
CubeUV layout, 1 its sampler, 2 the 80-byte `SkyEnvironment` uniform. The shaders call
`naval_sky::environment::{env_radiance, env_irradiance}` exactly as the web calls `pmremTexture`:
the reflection lobe converts its angular spread to a PMREM roughness with the web's measured table
(`pmrem_roughness`), the water body and foam read `env_irradiance(up)` and `env_radiance(up, 1)`. The
sun (or moon) the sea shades with is the uniform's `light_direction`, `light_color × light_intensity`:
the web's `ocean.sun` is `sky.light`.

The shaders import naval-sky's own `environment.wgsl` (`naval_sky::shaders::ENVIRONMENT_FILE`, which
`shaders::register` and `shaders::library` add unless another layer has). The crate still mirrors its Rust types and
bind group layout (`src/environment.rs`) and keeps `StandInEnvironment`, which fills an atlas of that layout from an
equirectangular map for tools and tests; the game hands the ocean the sky's bind group (`Ocean::set_environment`).
The sea's fog is the aerial perspective (`apply_fog` reads `naval::aerial`, [haze.md](../haze.md)).

## Screen-space reflections

### Hull bottoms and decks

Tan, rust and brown speckle hugged every hull (a brown halo from the bow quarter, rust grain beside the waterline from
low). It went with screen-space reflections off (`--graphics '{"reflections":"sky"}'`), and debug colours over the trace
showed two sources, both reads of the opaque copy that the mirrored ray cannot see (`shaders/screen_reflection.wgsl`):

- **The hull below the waterline**: a ray from a trough struck the copy's hull bottom (red anti-fouling) under the sea
  drawn at the hull. A read now counts only above the sea where the ray struck (`displacement` at a 0.5 m spacing plus
  `wake_height` there, faded in from 0.05 m below to 0.35 m above).
- **The deck, through the smear**: rough water smears the image over the lobe's spread, and the smear ran along the
  screen, where above a hull side's image lies her deck seen from above. The smear now turns the ray in its vertical
  plane, meeting the struck wall (taken upright) higher or lower, and a read counts only where the copy's surface is
  where that ray meets the wall (within 0.5 m, a tenth of the rise and 0.5 % of the depth). From high views a ship's
  long reflection streak, which was mostly her deck, is shorter; from low ones her side still streaks.

**Not changed: the open sea's warm specks** (a duel's shell camera, no ship near) are the sun's glitter
(`reflection::sun_glitter`, confirmed by colouring it) on near wave faces turned toward the half vector. A pixel draws
the glitter's mean, which on centimetre pixels is a dim, sun-coloured patch, where real glitter there is a few pinpoints
far brighter than white. Widening the facets' spread (a capillary floor on the unresolved variance, a larger footprint
share) spreads the same energy into a tan sheen; dropping the mean where a pixel holds too few glinting facets darkens
the near glitter path into the sun. Sparse, world-anchored sparkles there are the fix, left out for their temporal
aliasing. Since 2026-10-08 pixels wider than 0.1 m draw them ("Sun glitter as glints" below); these centimetre pixels
still draw the mean.

### Compiled out when off

The surface builds two more pipelines on a tier that traces the screen-space reflections, the surface and its lit
twin with `REFLECTION_STEPS` 0 (`OceanSurface::sky_pipelines`, `SurfacePipelines::sky`), and draws them whenever the
reflections are off (`OceanSurface::traces`: Graphics → Water reflections `sky`, which every preset below Extra high
sets since 2026-10-18, [graphics-presets.md](../graphics-presets.md)). The shader takes the trace's `enabled` as
`REFLECTION_STEPS > 0 && surface.reflections.x > 0.5`, a constant false in those builds, so the compiler drops the
trace. The reflection tiles (`crate::reflection_tiles`) are marked only for a frame that traces.

Why: compiled in, the trace costs the whole shader registers where it runs nothing. On the port's opening view with the
main menu's task force at 3456 × 2234, High at full density (2419 × 1564), another client on the GPU, the water and sky
pass was 17.3 ms with the reflections, 10.6 with them off at run time and 8.0 compiled out
(`bun native/tools/shader-ab.ts <variants> --port iowa --size 3456x2234` with `NAVAL_CLIENT_BINARY` a hot-reload
`dev-client`). At High's capped size
(1672 × 1081) the pass is 5.3–5.7 ms. Halving the steps, the refinements or the searches saved 1–3 ms each at full
density and only about 1 at the capped size; the waterline's height at each hit (`displacement` and `wake_height`
evaluated again) was 1.2 ms of it.

**The surface's stages on that view now** (High, 1672 × 1081, each stage's end returned and the rest compiled out, ms
of the pass, fastest tenth): a flat colour 0.26 (raster, vertices and the sky dome), the wake +0.65, the waves +0.70,
shadows, the cloud shadow, rain, the lobe and the hull shelter +0.65, the water body +0.69, the sky's lobe +0.44, the
glints +0.17, then the foam and everything after +2.1. That last step is an occupancy cliff, not one stage's work: the
bubble walls alone take it from 3.8 to 5.4 ms when cut there, yet the whole shader without them is only 0.2 ms cheaper,
and branch-free or single-hash versions of their cells saved nothing.

## Sun glitter as glints

From an aircraft (600–2,500 m, into the sun) the glitter path was an even grey sheen, like brushed metal: each pixel
drew `reflection::sun_glitter`, the Cox–Munk mean over its facets, where real glitter from the air is a sparse field of
sun images, none to a few glinting wavelets a pixel, each far brighter than white. The surface now draws that mean as
glints (`reflection::glitter_glints`, called in `surface.wgsl`):

- **The count**: a pixel expects `glitter_density` (the slope density at the half vector, the mean's own Gaussian) ×
  `GLINT_DISC_SLOPES` (the slope area that mirrors part of the 1.4° sun disc, 1.17e-4) × `GLINT_FACETS` (400 a square
  metre) × its area on the water (from the grid's screen derivatives) glinting facets. It draws a Poisson count about
  that (inverse CDF up to ten, Gaussian above) and scales the mean by the count over the expected number, so the mean
  over many pixels is kept in linear light.
- **Where**: counts are drawn on cells laid on the world about a pixel across, the two power-of-two sizes about the
  pixel blended (one rounded size drew horizontal seams where it changed). Below 0.1 m pixels the mean stands, the
  waves the pixel resolves drawing the glints; the glints take over by 0.4 m.
- **When**: each cell draws again four times a second (`GLINT_TWINKLE`) on its own phase, held for 60 % of the slot
  and faded into the next, on the waves' clock.
- **How bright**: the mean keeps its knee (`GLITTER_KNEE`, 0.25 × the sun's intensity) and one glint's pixel is held
  to `GLINT_PEAK` (1.0 × the sun's intensity, `reflection::luminance_ceiling`). Without the cap a lone glint 100–400 m
  from a bridge came out a hundred times the knee, and the bloom spread it into a round, soft halo.

North Atlantic, 9 m/s, 15:30, looking into the sun (luma 1st/10th/50th/90th/99.9th percentiles of a patch in the near
glitter path, 1600 × 1200): from 1,200 m 89/120/152/168/178 before, 35/74/138/174/202 after; from 600 m
104/134/157/170/179 before, 28/67/128/176/210 after. From 2,500 m, where a pixel holds hundreds of glinting facets, the
sheen barely changes. From the bridge (25 m) and the deck (8 m) the far glitter path sparkles and the nearest waves
draw the mean. The old note under "Hull bottoms and decks" above (sparse sparkles left out for their temporal
aliasing) is what this does for the far path; centimetre pixels still draw the mean.

Not checked: the twinkle in motion under the temporal filter (the sea keeps 70 % history), the GPU cost against the old sheen (a
300 m air view draws in 9.9–10.1 ms a frame on the dev client), and other facet densities or a looser knee (the
brightest glints stop near 220 of 255).

## Light through the waves (the Waterline look)

Bill asked for the sea of Waterline PRO 6 (an Unreal ocean on Fab, listing 0c1fc983-db84-4df3-b623-03db76d552c6),
starting from the shelved lane `goldflag/sea-foam-crest-glints` (historical; 14 October, never merged: it "looked worse than
master"). Its gallery's near water is navy troughs under emerald wave faces, crisp ripple highlights and many white
glints; ours was an even navy with soft highlights. Shader-only (`surface.wgsl`, `foam.wgsl`):

- **Light through the waves** (`wave_light`). The shelved lane lit it with the sun alone, on crest-height and steepness
  thresholds, three times stronger in bubbly water: toward a low sun it drew olive blotches and in a wake teal patches.
  Now the sky lights it from every side (`WAVE_LIGHT_SKY` 0.09 / sr of its level irradiance) and the sun through a
  forward Henyey–Greenstein lobe (g 0.55) on the 3D angle between its light and the eye's line (`WAVE_LIGHT_SUN` 0.3 at
  the peak, 0.15 of it off the lobe), so a low sun ahead lights the faces through and a high one, whatever its bearing,
  barely does. Where: the crests (Jacobian under 1) and the faces turned toward the eye of the waves longer than the
  finest cascade's (`WaveSample::swell_slope`, `swell_jacobian`, summed in the same loop), a little more high on the
  swell; a pixel over 0.25–1.5 m of water draws none (from the air its mean read as a teal haze), and only a line of
  sight low over the water sees it, fading out as the sine of its angle below the horizon goes from 0.25 to 0.7
  (`WAVE_LIGHT_LOW`, `WAVE_LIGHT_STEEP`): looking down, the eye's line goes into the deep, not through a crest. Colour:
  the place's own absorption over 10 m, teal in oceanic water. Bubbles add half again (was three times).
- **Ripples** 1.7 times their slope (`RIPPLE_GAIN`): the ripple layer takes that much more of Cox and Munk's tail as
  drawn facets and the roughness that much less, so near water has more crisp highlights.
- **Glints**: `GLINT_PEAK` 2 (was 1; the shelved lane's 4 bloomed each glint into a round halo once its bloom stars went).
- **Kept from the shelved lane**: bubble walls in the lace near the eye, the sparkle on sunlit white water, foam albedo
  0.75 and `FOAM_SUN` 0.7, thin foam tinted 0.55. **Dropped**: the four-point stars in the bloom (`bloom.wgsl`), and its
  1.6 times wider hull foam (master's hull-side wash of 17 October replaced that code).

**Toned down the same day** (Bill, of the start screen's view straight down on a battleship: "the green is way way way too
visible. at most do something like iteration 8"). The first version merged (#1729) drew the light at 0.11 and 0.4,
greener (a chlorophyll absorption, `WAVE_LIGHT_PIGMENT` (0, 0.005, 0.035) / m, on its path), over a wider mask and at
every angle, so from above the crests' network covered the sea in teal blotches. Now: the iteration-8 strengths and
mask, no pigment, and the low-sight fade. Near water at noon with the sun behind the eye (sRGB median, rows 400–720 of
1280 × 720 from 14 m): master 10, 42, 64; #1729 22, 61, 78; iteration 8 16, 53, 78; now 13, 48, 72. Straight down over a
ship the light is gone (the brightest tenth of the frame from teal 109, 159, 156 to the sky's 143, 142, 141).

Cost of the scene water pass (#1729's version; the toning adds one `smoothstep`), master's shaders swapped in by hot
reload in one client (`shader-ab.ts`, 2592 × 1545, High, other sessions on the GPU, about ±0.2 ms): chase 9.50 → 9.91 ms,
bridge 8.96 → 9.60, overhead 11.36 → 11.59. Captures and the rig (`cap.sh`, `iter.sh` with a hot-reload `dev-client`
built with default features; `tone.sh` for the views above the water) are in `.build/native/waterline/`. Not done: a
vertex-stage height of the shorter waves, which would mark their crests better than the Jacobian.

## Ships' shadows on the sea

The sea took the ships' shadows from the sun's near and wide maps only (the web game's `updateWaterShadows`, which left
the view maps off for want of a sampler), so a hull beyond the wide map (380 m across the sun's bearing from the ship
the maps follow) shaded her own decks from a view map but threw no shadow on the water, and on the Medium preset, whose
sun shadows are Low, she had none at all (no view maps under 2048 texels). Now `naval::shadow::water_sun_shadow` reads
the view maps too, by the weights the ships' paint uses, and Low draws its view maps at 1024; the surface and the spray
(`spindrift.wgsl`) take it unchanged. The sea needs no binding for them (they are layers of the near map's array) and
casts into no map, so it cannot shade itself however the waves move it. From a view map it takes only the shadows of
the hulls the map was fitted to, not the land's that is drawn into it too (which would lie on the water in her square
alone, ending in straight edges across the glitter). Why each choice, the filters per water-shadow quality and the
offsets: `native/docs/render-core/shadows.md`, "Sun shadows", "What the ocean needs" and "Only the hulls' shadows from the view maps".
The water pass's cost (M5 Pro, 1920 × 1080, a loaded machine): about +0.05–0.10 ms on the Medium preset and +0.05–0.28 ms on High (`native/docs/render-core/shadows.md`, "What the view maps on the
sea and on Low cost").

The Low preset's water shadows are off, so on Low the sea shows no ship's shadow at all, near or far: a setting
choice, left as it is.

## The scene's point lights on the sea

The web's sea took none of the scene's point lights (its material replaced three's lighting): at night a salvo, a
burning ship or a lightning stroke left the water black but for what the screen-space reflections caught of a lit hull.
The sea now takes them (`shaders/point_lights.wgsl`, the loop in `surface.wgsl` beside the sun's terms), each as the sun
or moon lights it:

- **Glints**: the sun's glitter BRDF (`reflection::sun_glitter`: Cox–Munk's slope distribution of the pixel's
  unresolved facets about its resolved normal, Smith masking, Fresnel), with what every light shares taken once a
  pixel (`light_frame`: the facets' frame, their spread, the view's Λ) and a light skipped where the half vector needs
  facets more than √40 deviations steep. Each light's glints are kneed at its own irradiance on a surface facing it
  (`SeaLights::peak × GLITTER_KNEE`), as the sun's are at the sun's, and summed under the ceiling their lights set
  (since 2026-10-20, `native/docs/lighting/lights.md`, "The sea's lamp glints as glints"): since 2026-10-21 each light's irradiance
  weighed by its share of the glitter at the pixel, and a lamp's glint, a point, held to one pixel's worth of light
  however many pixels its cell covers (`reflection::lamp_glints`; `native/docs/lighting/lights.md`, "Lamp glints and salvo smoke").
  Until 2026-10-20 they summed under the knee the
  sun's or moon's take, never lower than a full moon's (`LAMP_KNEE_FLOOR`), because kneed at its own irradiance a
  fire's glitter path 100 m off came out 30–40 × dimmer than the same BRDF gives (the forward glitter at grazing views
  being where its BRDF is largest): a fire's night light was a seventieth of its real light then. Lifted with the night
  (`native/docs/lighting/night.md`, "The night eye") a fire's own irradiance is 15 times the moon's at 100 m, and a star shell's the moon's
  8–35 times: kneed at the moon's, their glitter was held to a small part of what the BRDF gives.
- **Into the water**: the irradiance crossing the mean surface (Fresnel's transmitted share, the cosine of incidence),
  of which the water scatters `subsurface_reflectance` back up, as the sunlit term of `upwelling`.
- **Foam**: the foam's wrapped diffuse light (`foam_radiance`'s share and albedo), whitecaps and wakes about a fire or
  a flash.

A light is a point of three's intensity (its colour × candela in the sea's units), inverse square from 1 m out, with no
cut-off: the reach three's lights carry (126 m for a 16-inch muzzle flash, 70–130 m for a fire) is a culling
convenience, and the sea mirrors a light from wherever it stands. Below `FAINT` (1e-3 of irradiance) a light's diffuse
terms are skipped (a thousandth of the moonlit sea). Since 2026-10-20 the sea reads the renderer's list at group 0,
as every lit layer does (`native/docs/lighting/lights.md`, "Lights at night"): the 64 brightest at the camera, the flashes, every fire in
view, the star shells, the searchlights (a pixel outside a beam's cone skips it) and the stroke, a pixel taking those
of its cell of the clustered index; `OceanSurface::point_lights` only says the frame has any (the variant that takes
them). The lamps' glints are drawn as glints, as the sun's are (`glitter_glints`, counted by the facets of the lamp
whose glitter is brightest at the pixel), where their summed mean drew a flat sheet toward a star shell; since
2026-10-20 at every pixel size (`lamp_glints`): the mean stood within a few hundred metres of the eye, a matte sheet
under a star shell overhead.

**What hides a light (2026-10-10).** The lights were unshadowed: a fire on a ship's upperworks laid its glitter path and
glow on the water right up to her side, though her side, her turrets and her upperworks stood between, and a fire on her
far side mirrored on the near water where it could not be seen. Each light is now marched toward over the opaque scene's
depth behind the sea (`light_seen`, the binding the water's column already reads), toward a point `SEEN_RISE` 5 m over
the light: a fire's light stands about half way up its flames, and water that sees their tops mirrors them (marched to
the light itself, flames plainly in view lost their reflections where a deckhouse hid the light's point). The march
starts at the light's end, where the burning ship's own structure stands: sixteen steps placed as the square of their
count, read in batches of four, shifted by a pixel's interleaved gradient noise that moves each frame for the sea's
temporal history to blend (a redrawn frame keeps its frame index, so stills show the noise where a fire is partly hidden),
and stopping 4 m short. A step hides the light where the screen shows a surface in front of it by 0.5 m to the larger of
30 m and a twentieth of its distance (a hull's beam at hand, a hull seen end-on far off); farther behind than that it
passed behind something nearer the eye, which hides nothing. Where the water lies between the eye and the burning ship,
which is where her glitter path lies, the eye's line to any point of the water's path to the light runs above that path,
so a point the screen does not show is inside what hides it. Off the screen's edges and behind the eye nothing hides a
light. A light is marched only where its glints, its light in the water and its light on foam, each weighed by its share
of the pixel (`LightShares`), add more than the larger of 1e-4 (under 3 of 255 levels with the eye at its most open at
night) and 3 % of the pixel's light without the lamps: marched wherever it lit the water, the march cost three times as
much, most of it on water a light lights too faintly for its shadow to show. With the water's shadows off (Graphics; the Low preset) no light is
marched. The fire's glow sprite on the water beneath it (`fires.rs`, `pools`) is not shadowed. Cost (GPU busy, 1728 × 1030, High, the burning Bismarck 120–1200 m off at night,
three fire lights, other lanes building): +0.24–0.45 ms over the unshadowed lights; twelve steps saved 0.03–0.08 ms and
changed up to 20,000 pixels by more than 8 levels.

**Cost.** The lights' block costs the sea's shader registers even where it runs nothing (0.1 ms of the water pass at
2400 × 1350), so the surface is built twice (`POINT_LIGHTS`) and a frame without lights draws the variant without
them: day, and night without combat, draw master's shader. In the lit variant the block runs after the foam, where few
values are live (mid-shader, three fires cost 0.30–0.35 ms; there 0.17–0.22), each term reaching the surface by the
paths the sun's takes (the glints where no foam covers the water, the light in the water through the body's share, the
foam's through each layer's opacity), and a light's glint terms are taken only where it can glint. Apple M5 Pro,
2026-10-03, High, the burning Cleveland 700 m off at night (`.build/native/look-night-light/ab_fold.sh`, 16 alternating
rounds a side, load 6–16): the water pass 4.41 ms without the lights, 4.62 ms with three fires (min 4.38 → 4.57).
