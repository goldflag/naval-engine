# Foam and weather at sea

Whitecaps, the foam's shading, gale streaks, rain on the water, spindrift and the spray haze. The wakes' white water is
in [wakes.md](wakes.md) and [bow-waves.md](bow-waves.md). Index: [README.md](README.md).

## Whitecaps

The browser game's whitecaps drew round white discs 20–60 m across: the longest cascade, holding the peak, broke, its foam
spread 3 m²/s into a round patch, and a pixel too coarse for the lace drew the patch's mean, so the disc's outline was
the spread's. Native-only changes since, measured against the stills of `.build/ocean-review/` (no web reference
remains):

- **Breaking window** (`whitecaps.rs`): waves break from 1/32 to 0.3 of the sea's mean wavelength, fading out by 0.6
  (was 1 and 2). Breaking crest length peaks at a third to half of the peak's phase speed (Phillips' Λ(c), Kleiss &
  Melville 2010), so the dominant waves rarely break: on High at 9 m/s the 1,024 m cascade carries no breaking, and the
  breakers are 4–8 m waves (periods 2.2 and 1.5 s against 4.3 and 2.6). The thresholds still hold Monahan's coverage.
- **Spreading** (`field.rs`): 0.015 · λ · c, at most 1 m²/s, a quarter of it across the wind (was 0.05, 3, a half), so a
  thin crest stays thin.
- **Shading** (`foam.wgsl` `whitecap_foam`): a coarse pixel draws the same whitecap on the lace's coarse layout instead of
  its spread mean (no more `spread_whitecaps`, the surface's `foam_mean` read is gone), the core starts at 0.85–1.3 of
  fresh foam and covers 85 % of it, the old patch's lace covers 20–52 % at 10–50 % opacity and leans further into the
  wind's streaks. A breaking face stands up 0.4 m (`WHITECAP_FACE`), so a grazing pixel keeps its core as a short
  fleck. The bubble cloud under whitecaps is read over 4 × 1.5 m (was 8 × 2.5) from 0.12 to 0.7 of the bubbles, at 0.2 of
  its strength (was 0.45): it drew the pale pills that outlived each disc.

Share of sea pixels white (brighter than their surroundings by 35 levels), before → after, from 700 m up and from 60 m:
9 m/s 5.7 → 5.6 % and 0.55 → 0.42 %, 13 m/s 5.0 → 4.8 % and 1.4 → 1.1 %, 17 m/s 4.5 → 3.6 % (from 140 m 4.0 → 2.9 %).

## Foam shading

Renders of master showed whitecaps as hard-edged, uniformly white cut-outs at mid distance and in a storm, gale streaks
as smooth white ribbons and a crisp white stripe along every hull's waterline. Changes (`foam.wgsl`, the foam part of
`surface.wgsl`):

- **Whitecaps** (`whitecap_foam`): the core thickens from 0.45 to 1.4 of fresh foam (was 0.85–1.3), its outline torn by
  the churn and the lace together (`CORE_TORN`) and its outer flecks 0.4 opaque. Where the lace is filtered away (a
  grazing or distant pixel) the texture's sway goes, so a world-anchored value-noise tear (`foam_tear`, octaves of 9, 3.7
  and 1.5 m fading with the pixel) scales the amounts and the residue's cover, and the residue thins over the foam read
  across the bubble cloud's ellipse (`WaveSample::foam_soft`, the spread read the surface already took) instead of at
  the field's last texel. Near, where the lace resolves, its own threshold draws the edge as before.
- **Colour**: `FOAM_ALBEDO` 0.7 (dense sea foam reflects 0.55–0.75), deeper crevices (`CREASE_LIGHT` 0.62), and thin
  foam takes the water's colour (`thin_foam`: 1.4 m of the water's absorption, 75 %), so lace and windrows are bluish
  grey and only the dense core is near white.
- **Finer lace**: the lace's own finer copy (4.7× finer, turned 1.1 rad) moves every threshold ±0.15 where a pixel
  resolves it, so edges fray at centimetres.
- **Gale streaks** (`gale_streaks`): lace, not ribbons: cover 0.7 of the streaked lace torn by the round lace (45 %),
  0.3–0.85 opaque with how much old foam there is, breaking along each line (the tear laid six times longer along the
  wind, keeping 35 % where it breaks).
- **Shoreline foam**: torn along the line, thinning away from the contact (cover 0.4 at most) and tinted as thin foam;
  against a hull's side (where its shelter hides the sky) 85 % of it goes, the hull's contact foam (`wake.wgsl`
  `hull_foam`) drawing the white water there.

In a storm (18 m/s, a Bismarck at 7.7 m/s, from 140 m at 4°, paired with master's foam shaders in one client) the share
of sea pixels 35 levels brighter than their surroundings went 7.2 → 6.7 %; at 13 m/s close up, whitecaps keep a bright
core inside bluish, see-through lace. The bubble cloud under foam is paler (`BUBBLE_PATH` 2.6 m, was 4.5;
`BUBBLE_STRENGTH` 0.6). Before and after stills: `.build/native/foam/` (`final-before-*`, `final-after-*`).

## Weather at sea

The web's sea took no rain and threw no spray of its own. Native only since, all from the weather the client already
has (the sky scene's precipitation, the battle's wind): a dry sea under 13 m/s draws the same bytes as before.

- **Rain on the water** (`shaders/rain.wgsl`; `OceanSurface::rain`, 0 dry … 1 a downpour, which naval-client sets each
  frame from the sky scene's precipitation, the console's included): every drop sends a ring of capillary waves across
  the water. Where a pixel resolves them the rings go into the surface's normal (two sizes of drop on a jittered grid,
  each landing somewhere new every life, the front running out at 0.24 m/s); where it does not they are slope variance
  (0.02 in a downpour, 60 % of it drawn as rings where they resolve), so the mirrored sky blurs and the glitter spreads
  and dims.
- **Spindrift** (`src/spindrift.rs`, `shaders/spindrift.wgsl`): from 13 m/s (Beaufort 7; everywhere by 17.5) the ocean
  draws wisps of spray torn off its own breaking crests, where the whitecaps' fresh foam shows through the same breaking
  groups and crest gates the surface draws them with (`breaking_group` moved into `naval_ocean::waves`). No particle
  lives on the CPU: each wisp is a slot in four levels of 48² cells laid on the world about the camera (2.5 m at the
  finest, each level twice the last over its own band of distance, three wisps a cell), driven downwind, growing and
  thinning over its life. They draw in `Phase::Transparent`, lit as the splashes' white water (`naval-water-effects`'
  `spray_radiance`, whose constants `spindrift.wgsl` copies: that crate depends on this one), with lightning's flash the
  host sets in `ocean.spindrift.flash`, and fade into the sea and hulls behind them through the backdrop's depth
  (`SceneCopies::BACKDROP_DEPTH`, asked for only while the wind makes spindrift). Since 2026-10-04 a wisp's beam takes
  the ships' (and splash columns') shadows as the sea beneath it does, besides the clouds': `water_sun_shadow` at its
  centre in the vertex stage, at the surface's water-shadow quality (`SprayUniforms::spray.w`, which `Ocean::prepare`
  passes from `surface.water_shadows`) and faded under the moon as the surface fades it. Spray torn off a crest in a
  hull's shadow no longer glows white in it. One read per live wisp, in the vertex stage: no cost the noise could show.
- **Spray haze** (`spindrift::spray_haze`, `shaders/spray_haze.wgsl`): from 17 m/s a haze of spray lies over the sea,
  its extinction 1.8e-4 /m at 20 m/s doubling every 3 m/s (at most 0.01 /m) and falling off above the sea over a scale
  height of 3 m plus 0.4 of the significant wave height, integrated exactly along each view ray: visibility in it about
  14 km at 22 m/s and 3.4 km at 28 (Beaufort 9 to 12, "spray may affect visibility" to "very seriously"). The surface
  veils the water with it before its distance fog, and the wisps take it too (`Surface.weather` yzw).
- **Gale foam streaks** (`field.rs` `streak_params`, `shaders/resolve.wgsl`, `foam.wgsl` `gale_streaks`): from 13 m/s
  (full by 20) the second cascade's displacement `w` holds old foam (181 m on High, texels of 0.7 m): fed by its
  whitecaps, carried each update by the surface's drift (3.5 % of the wind: the wind-driven current and the waves'
  Stokes drift) and gathered by two meandering sets of Langmuir convergences 18 m and 29 m apart, with the continuity
  equation's compression (passive advection never gathers), decaying over 30 s. The surface draws it as the wind's
  streaked lace in long patches laid on the world. Top-down at 22 m/s the sea goes from 5.9 % to 10.5 % white, and its
  foam marks from 15 to 58 m long (aspect 1.9 to 6.8).
- **Rain's splashes on the drawn sea** (`src/rain_heights.rs`, `shaders/rain_heights.wgsl`): naval-clouds' rings and
  crowns stood on the flat mean sea, floating over troughs and hanging in front of a hull's boot-topping. While it rains
  the ocean evaluates its own surface (`height_at` and the wakes' `wake_height`, as the hulls' wet band does) on a 256 m
  square about the camera at a metre a texel, laid on the world so it repeats; `Ocean::rain_heights` hands it to the
  clouds' weather (`Weather::set_sea_heights`, which naval-client's `scene/mod.rs` wires; `native/docs/clouds/README.md`).

**Cost** (Apple M5 Pro, 2400 × 1350, a Fletcher in a 22 m/s storm and heavy rain, the camera 140 m off at 7°, load
average 7–15; six alternating rounds of 120 frames, the client's own shaders against a variant drawing none of the above
swapped in through hot reload; the fastest frames): the whole frame 9.61 ms against 9.58, the transparent pass 0.93
against 0.99, water and sky 4.53 against 4.54, all within the noise. The rain's heights take 0.068 ms of compute, only
while it rains; the streaks' update rides in the wave field's pass (0.36 ms either way).

**Left open: an opaque veil.** The spray haze veils only the water and the wisps; hulls, land and aircraft standing in
it are not veiled. A pass over the opaque scene (or the haze in the fog every material shares) was left out.
