# Foam and weather at sea

Whitecaps, the foam's shading, the foam ship wakes and shell splashes leave (the foam field), gale streaks, rain on the
water, spindrift and the spray haze. The wakes' trails and waves are in [wakes.md](wakes.md) and
[bow-waves.md](bow-waves.md). Index: [README.md](README.md).

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

## The foam wakes and splashes leave

Foam is a tracer the water stirs, not a pattern painted over the sea: a stern's churned water and a splash's falling
column lay it, the water carries and spreads it, and it dies thin first and thick last. The foam field
(`src/foam_sim.rs`, `shaders/foam_sim.wgsl`; after Crest 4's foam simulation and the Atlas talk at GDC 2019, written from
their descriptions) simulates it about the focus hull; the surface reads it there (`wake.wgsl` `sim_read`) and draws
the same foam itself beyond the field's window and on Low (`trail_read`, `splash_read`).

**The field.** Each cell holds thickness (about 1 is dense white water), turbulence (the churn that stirs it), residue
(the scum and bubbles foam leaves as it dies) and validity, in `Rgba16Float`: two states as one array texture (the
surface's group 1, binding 18) and a scratch layer. Each step stores its cells rounded up or down at random by up to
half an fp16 unit (`dithered`): a 30 Hz step's decay of thick foam is about one unit, which rounding to the nearest
would take or keep whole every step. It is addressed toroidally: the field's cell k lies in texel k mod
the cells along an edge, so a window that moves clears only the cells it takes in, with no copy.

| Tier | Cells | Cell | Window | Memory |
| --- | --- | --- | --- | --- |
| Ultra, High | 1,024² | 0.75 m | 768 m | 24 MB |
| Medium | 512² | 1.5 m | 768 m | 6 MB |
| Low | none: all the foam drawn | | | |

Medium keeps the window and halves the cells, as the wake field does there (`FoamGrid::for_tier`; `Ocean::enable_wake`
sizes it, and a tier taken by the next scene makes it afresh).

**The window** lies about the focus hull, astern of her middle by 0.22 of its edge at her full speed, so more of her
wake lies in it. Where the view looks at water further than 0.6 of the half-edge from there (230 m on High: a camera
held over a fall of shot), it goes about that water instead (`place_window`).

**A step** (30 Hz, at most three a frame): `advect_pass` carries the field back along the flow (semi-Lagrangian);
`settle_pass` corrects that (MacCormack, clamped to the four cells it was read between, Selle et al. 2008), spreads it
(0.01 m²/s, plus 0.6 times the turbulence), decays it and lays the sources. The flow is the surface's drift (3.5 % of
the wind); the wind's Langmuir cells (lines about 17 m apart that bend and lie unevenly, meeting at up to 0.04 m/s in
stretches about 45 m long with slack water between; the one part of the flow that gathers foam, into windrows, and only
thin foam: past a thickness of 0.2 to 0.6 the water that meets sinks with it, so a windrow is a broken line of old lace,
never a white bar); each landing splash's base surge, which spreads its deposit as a whole; and eddies 6 and 18 m
across, the curl of a stream function times the turbulence (2 m/s at full churn), which stir the churned water's edges
into streaks without gathering or thinning it.

**Time scales** (`schedule`). A frame longer than three steps (a time scale above about 3× at 30 fps) runs three longer
steps that share all of its time; none is dropped. Their flow runs at most 0.2 s, so a fresh splash's surge carries its
water at most a quarter of its radius in one. Past that (above 18× at 30 fps, 36× at 60) each step stands for more game
seconds than its flow runs: the foam decays and the splashes land over all of them, so the wakes keep their length to
the theatre's 256×, while the drift and the eddies run slower than the game. A stern counts as put there only when she
moved 100 m further than her speed carries her since the last frame that stepped (at 256× and 30 fps a hull at 30 kn
runs 131 m a frame). Longer steps keep a little more foam than game speed's: 30 s after one splash the field holds 1.15
times game speed's foam at 4×, 1.52 at 15×, 1.28 at 60× and 1.08 at 120×, and 0.93 to 1.16 of its residue (dropping the
time held 1.7, 3.7, 4.6 and 4.7 times, and from 0.87 of the residue down to 0.06). The cells are rounded at random, so
this is the longer steps' own error, each taking the foam's life and the splash's landing once for all of its time
(before the splashes laid less water and the windrows only thin foam it held 1.10, 1.33, 1.09 and 0.95, and rounding the
cells to the nearest rather than at random 1.13, 1.42, 1.17 and 0.98).

**Decay.** Thin foam (thickness 0.06) lasts 3.5 s and thick (0.9) 34 s, sooner or later in patches 8 m across.
Turbulence lasts 5 s. A third of the foam that dies becomes residue, which lasts 75 s. Each decays exponentially over the
game seconds its step stands for.

**Sources.**

- *Sterns* (8, those in the window): each lays her churned water along the run her stern made since the last step, a
  Gaussian 0.59 of her beam across swept along the run (`swept`, two erf terms), so her runs join into one band at any
  time scale and lay 2.2 times her churn (0–1 with her speed) down its middle once she has passed, on lumps the size of
  her boils. Her speed is taken as far as her wake reaches the surface (`surfaced`, as her trail and waves take it: all
  of it at the surface, none from 3 m down), so a submarine running submerged lays none (`stern_source`). A stern that
  was put there (see *Time scales*) lays nothing between.
- *Splashes* (32, the youngest; more falling at once leave the oldest out, whose foam laid so far stays): each lays its
  deposit's water (`src/splash_foam.rs`) as the column falls back, from 0.8 s, most of it from 2.5 to 6.5 s and the last
  by 10 s, in lumps, its outline frayed, over the game seconds each step stands for. A charge going off in the sea lays
  one too (below). A strike lays no white water of its own on the wake field.

**Validity.** A cell the window takes in (it moves on or jumps, the field is new at a battle's start or a tier change,
the wakes are reset, or a re-lay turns the charts too far to carry it) starts from what the surface draws there
(`seeded`): on a trail, the field's own foam of the trail's age (the cover the surface draws, in the lumps a stern
lays, thinned where the field's foam dies sooner the more the older the trail, its age read off its churn), turbulence
so it is as young and a little residue where its churned water lay (the slick beyond it the trail draws itself); and
the splashes' deposits, old ones too. Its validity starts at 0 and rises to 1 over 1.2 to 6.6 s, sooner in some
patches 24 m across than in others, and the surface fades from its own foam to the field's by it, so no clear shows,
the field takes over a patch at a time and an old wake carries on into the field.

**Re-centres.** A theatre re-centre moves the field's frame with the scene (`FoamSim::translate`, `take_from`, from
`ShipWake::take_fields_from`): no cell is cleared and the foam stays on its water, whatever the shift. A re-lay
(`relay_fields_from`) carries it by the shift at its centre where the charts turn its water by less than the wake
field's limit, and otherwise lets the new scene's start afresh.

**Drawing** (`sim_read`). Thickness sets the white water's cover (0.03–2.0). Thin foam is clumps of white water (the
drawn foam's clump field, carried by the drift), each torn over 0.2 of that field at its edge, never a grey film; from
1.0 to 2.4 it closes into one mass, as dense as a fresh wake's. Turbulence says how young it is (0.12–0.55), and so does
thickness (0.8–1.8), so thick foam stays dense white water however old; the bubbles under it come from both, and the
residue is a pale, stilled scar. The field's weight fades over 0.84–0.97 of the window's half-edge, times its validity,
and where a pixel spans 1.5 to 3 of its cells (it has no mips) to the drawn foam, which takes their mean there.

**The drawn foam** (beyond the window and on Low): a trail's band is white in patches along it and in masses on its
boils, its old foam in clumps, its eddies' and streaks' detail carried on the trail's water by two-phase flow maps
(Vlachos 2010) rather than scrolled through time; under a wake's foam the foam texture is hex-tiled (Mikkelsen 2022),
so its 40 m tile never repeats. A splash's deposit (64 in the `Wake` uniform) is lobed from its hash,
pushed out by the base surge and skewed downwind; it whitens as the column's water falls back (from 1.3 s, densest to
9 s, gone by 15 s), then breaks into clumps and streaks and leaves a faint scar of pale, stilled water to two minutes,
carried by the drift. With more deposits on the water than the uniform holds, each fresh splash has the least seen of
the others give up its slot (`SplashDeposits::give_up_slot`): how white it draws at its age times the share of the view
its radius takes from the camera, one that has landed before one still landing. It draws into itself over 0.9 s, and
the fresh one, which draws nothing for its first second, takes the slot after. Only a burst of more than 64 within a
second drops a fresh one at once.

**A charge's white water** (`SplashDeposits::lay_charge`, from `ShipWake::strike`): a torpedo hit, a depth charge, a
burster going off in the water or a magazine going up beside it lays a deposit as a splash does, on every tier, its
deposit starting as wide as the charge's gas bubble grows (`charge_scale`: Cole's 3.4 (W / (z + 10))^⅓ m over the
deposit's 7 m): a 533 mm torpedo's warhead 4 m down 1.4 times a 38 cm round's, a 610 mm one's 1.6, a depth charge at 10
to 15 m 0.9 to 0.85 (its dome's wide white circle), a magazine 2.4. A shell's burster goes off in its own splash: one
within 30 m of a deposit under 0.5 s old (before any of it is laid or drawn) widens that deposit if it is the wider
(a bomb's near miss) rather than laying a second, and a shell's burster is narrower than its splash (a 12.7 cm HE
shell's 0.28 to 0.49, a 46 cm one's 1.10 to 1.13), so shelling lays no more deposits than it did.

**Cost.** Two compute dispatches over the field a step (1,024² on High, 512² on Medium), at most six a frame, timed as
`ocean foam step` ([cost.md](cost.md)). A cell taken in reads the trails' tiles and the splashes once. Within the window
a pixel reads four texels and the clump field instead of the drawn foam.

**Knobs.** The constants at the top of `foam_sim.wgsl` (sources, life, flow, `VALID_*`, `SEED_*`) and the `SIM_*` constants in
`wake.wgsl` (drawing); both reload while the client runs. `NAVAL_FOAM_SIM_DEBUG=1` prints the window and its sources.
Tests: `foam_sim::tests` (tier sizes, the window's place, a re-centre's carry, the swept runs, the splash slots, the
schedule under time scales against the old one), `splash_foam::tests` (a slot given up over a second, the least seen
first), every shader composes and validates; on the GPU, `tests/foam_sim.rs` (the foam stays on its water within half a
cell across a re-centre and a re-lay by no whole number of cells; 30 s after a splash at 4× to 120× the field holds
0.8–1.6 times game speed's foam).

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
