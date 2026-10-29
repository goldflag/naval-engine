# Wakes

The wakes are the ocean's: the surface shades them, so they live in `naval-ocean`. Splashes and spray above the water
are [water-effects.md](../water-effects.md); bow and stern waves are [bow-waves.md](bow-waves.md). Index:
[README.md](README.md).

## The wake field

`src/wake/` (`WakeField`; `kernel.rs`, `generators.rs`, `impulses.rs`; `shaders/wake_field.wgsl`) steps a dispersive
wave field about the focused hull: reduce, collapse, step and display as compute on storage textures (state
`Rgba32Float`, displays `Rgba16Float`, levels `R32Float`), at a fixed 30 Hz, at most four steps a frame, each step's
uniform in a dynamic-offset slot. `kernel.rs` fits √(−∇²) as a pyramid over the levels (it sums to zero and follows
|k| within 4 % from 6 to 12 × 2^levels cells); `generators.rs` sweeps each hull's bow and stern generators (a speed
gate, a 0.8 s depth response, teleports). `ShipWake` (`src/ship_wake.rs`) runs the generators of the nearest eight hulls
within 900 m, takes each event once by its sequence and packs the `Wake` uniform. The field covers 1,536 m at
256², 512² or 1,024² on Medium, High and Ultra; Low has none. `tests/wake.rs` replays the browser game's wake
diagnostic, `scripts/diagnostics/ocean-wake.html` (historical): a Bismarck's generators, 30 s at 15 m/s, a turn, a stop,
a teleport and the breaking run, landing within half a percent of its figures.

### The wake field across a change of view

The wake field (the hulls' Kelvin waves and the white water where they break, 1,536 m about the focused hull) used to
reset when the view moved to a hull beyond it: the field scrolled off everything it held and dropped the generators of
every hull more than 900 m from the new focus. Going from a friendly ship to an enemy and back, each found her waves
gone and growing again from a calm sea. In a theatre, a view more than 16 km from the scene's origin re-centres the
scene, and the new scene's field started calm as well.

`ShipWake` now keeps one more field. The field the view leaves for a hull it does not cover becomes the standby: it
steps on about the hull it followed, raising her waves and those of hulls near her. A view that comes back to her (or
to a hull whose waves the standby raises) swaps the two and finds her waves as they ran on. A view to a third distant
hull takes the standby's field for her place, calmed, so the last two hulls viewed keep their waves. The first time
the view reaches a distant hull, her waves still build from nothing. A view between hulls that are already in the
field's range (a formation's ships) moves the field as before. What strikes the sea is laid on both fields
(`ShipWake::strike`, `ShipWake::add_impulse`), including a strike in the frame the view reaches a hull, before her
field is made or calmed (`WakeField::restart_at` keeps what strikes it until `calm_textures`). The standby is let go
with its hull (no longer drawn), and with a reset (port).

A theatre re-centre carries both fields into the new scene's metres (`ShipWake::take_fields_from`,
`WakeField::take_from`): their textures are copied into the new scene's own fields, which keep their pipelines and
bindings, and their place, generators and strikes are moved by the shift. The field's grid is laid from a lattice that
moves with it, so a shift that is not whole cells moves nothing on the sea. A re-lay onto another chart
(`ShipWake::relay_fields_from`) carries each field by where it lays the field's centre, unturned, while the charts turn
its water by no more than half a degree (`RELAY_TURN`: under 3 m at 300 m from the hull); a field it would turn more,
or a field of another size (the graphics changed), starts calm. In Eastern Solomons, Saratoga and Aaron Ward are
226 km apart: a view from one to the other re-lays the scene, and the way back re-centres it.

The trails were carried across a re-centre before this, but each sample's trail point (where the churned band and
the slick are stamped) stayed where it was while the sample moved by the shift (`WakeFoam::translate`; a re-lay
already carried it). After a 30 km re-centre, a hull's band and slick were stamped 30 km off and vanished until she
laid them again; only the white water drawn from the samples themselves stayed. `trail::tests::a_re_centre_carries_the_band_with_the_trail`.

**Seen in the client** (dev-client, 1280 × 720, Eastern Solomons from a save at 08:30, the view on Saratoga for 30 s,
on another hull for 20 s, back on Saratoga for 2 s; `.build/native/wake-switch/`): Worden 4 km off (no re-centre),
Enterprise 30.5 km off (a re-centre each way) and Aaron Ward 301 km off (a re-lay, then a re-centre) all find her
wake, the band, slick and white water, as she left it. Before, the Enterprise and Aaron Ward views found it gone and
growing again.

**Cost.** None until the view first goes beyond the field. From then on, the standby's steps run beside the focus's
(both are timed as `ocean wake step`): about 0.3 ms a step on High's 512² and 1 ms on Ultra's 1,024², at 30 steps a
second (the timings above). It holds about 13 MB of textures on High and 51 MB on Ultra. Low has no field.

**Tests.** `tests/wake.rs`: `the_view_going_to_a_distant_hull_and_back_keeps_her_waves` (two hulls 6 km apart, the
view moved from one to the other and back, and then the scene re-centred 16.5 km over),
`a_field_left_with_a_hull_that_is_gone_serves_the_next_focus`, `a_strike_as_the_view_reaches_a_distant_hull_is_kept`
and `a_re_lay_carries_the_fields_it_barely_turns`. In the first, her wedge measured 0.0127 m mean
height before the view left and 0.0160 m when it came back, since she kept raising it (a calm field before the
change).

### Where the hulls' waves break

From above a fleet in action on Ultra's ocean, every heavy fall left a ring or blob of dense, pebbly white water 15–40 s
after the column, larger than the foam patch under it and lasting half a minute (the owner's screenshot, a theatre
battle at 1×). It was the wake field's breaking foam: the threshold tuned for the hulls' under-resolved waves (a slope
of 0.015 over 12 m) times the fastest nearby hull's speed share, so any hull under way set it for the whole field. A
strike's ring train held it off for 20 s ("The sea answers what strikes it", "Breaking", below), but the rings outlive it:
once forgotten, they broke into annuli wherever they were still steep. On Ultra's 1.5 m cells the rings are the
steepest, and every heavy fall broke; on High it was rarer.

A hull's own breaking lies along her sides. Two fields in lockstep, one breaking at 0.015 for its last step only, give
where it happens: a Bismarck at 15 m/s, straight or turning 0.03 rad/s, on High and Ultra, breaks 98.9–99.9 % within
30 m of the line between her bow and stern generators and all of it within 40 m (1.1 beams), while 114 m from either
generator alone (her middle); a Fletcher at 18.5 m/s breaks next to nothing in the field (under 2 % of her foam, only
while turning). So each generator now carries the stretch where its hull's own waves break, from it to her middle
(`GeneratorOptions::breaks_to`), and how far either side (`break_reach`, `ship_wake::BREAK_REACH` 1.25 beams, fading
out to 1.5 times that), and the step pass takes the breaking threshold only there (`wake_field.wgsl`, `breaks` in the
step uniform). The ring trains stay: they still hold the breaking off where fresh rings pass under a hull.

- A hull's breaking is unchanged: the same 30 s runs' total foam (a Bismarck straight 578.37 on High and 2,448.18 on
  Ultra, turning 532.83 and 2,222.24; a Fletcher 62.64 and 226.90) came out the same to the hundredth, and the web's
  breaking figure in `tests/wake.rs` reads 578.54 (web 578.26).
- `a_salvos_rings_never_break_beside_a_hull` (`tests/wake.rs`): on Ultra's grid a 38 cm round and a 70 kg burster
  ring the sea for a minute while a Bismarck steams by 400 m off. The foam about them only fades (0.19 at 5 s to
  0.0001 at 60 s); with the old field (the stretch's reach made unbounded) it rose from 0.05 at 15 s to 1.18 at 20 s
  and 1.67 at 25 s.
- Seen with `bun native/tools/effects-trial.ts --only splash-underway --stills --graphics '{"ocean":"ultra"}'` (added
  for this: a Bismarck at 15 m/s under another's fire, its first heavy fall after 75 s from 420 m up for 52 s), master's
  build (`--binary`) against this one (`.build/native/effects-trial/splash-rings-before-after.png`): master's
  dense white patches at 18–30 s, and a later salvo's at 44 s, are gone; the falls leave the trail's fading lace and
  pale water. On High master's build shows none in this scene. `parity-wake-astern`,
  `theatre-cruise`, `chase-one-ship` and `fleet-measure30` render within 0.03 levels of master's build (both miss their
  references by the same stale margins on this Windows machine).
- Cost: the step uniform grows from 1,616 to 1,872 bytes (slots of 2,048), and the step's generator loop takes one
  more capsule distance a generator, in a pass bound by its texture reads.

## The sea answers what strikes it

Splashes, torpedo hits and sinking hulls drew spray, columns and foam rings, but the wave surface under them did not
move. Now what strikes the sea disturbs the dispersive wake field itself, which turns each disturbance into ring waves
spreading at deep water's speeds, as the local interactive wave simulations of Sea of Thieves or Skull and Bones do.
Visual only: combat never reads these waves.

**What the field takes** (`naval-ocean` `src/wake/impulses.rs`, `shaders/wake_field.wgsl`): an impulse is a footprint
(a point, or a capsule along a keel) and what it does there over its duration: a height laid as a Mexican hat,
η₀ = −D (1 − s) e^−s at s = r²/R² (a crater for D > 0, a dome for D < 0; no net volume about a point, so it carries no
long-wave pedestal), raised smoothly (smoothstep) in the state's height and its previous step alike, so it has no
velocity of its own; a pressure head held under a Gaussian with a sin² envelope (negative lifts the sea, as a hull's
positive head presses it); and white water laid over e^−s (the field's foam, 1 dense). The step pass takes up to 16
a step (more wait a step, their course shifted whole, those under way first), at the field's own 30 Hz clock. Off the
field (1,536 m about the focused hull, and about the hull the view last left while a field is kept there: "The wake
field across a change of view" above) nothing is laid. `WakeField::add_impulse`, `WakeField::strike`.

**How big** (`strike_energy`, `crater_size`). The wave energy is 2 % of the energy released at or near the surface:
the linear crater of Kranzer and Keller, fitted to measured explosion wave trains, holds the waves' energy as its own
potential energy, about 2 % of a near-surface charge's (Le Méhauté and Wang, *Water Waves Generated by Underwater
Explosion*, 1996, from the US Navy's HYDRA analyses; the same linear theory came out about 40 % high against basin
tests). The same share is taken for impacts, which is less certain. The hat holds ½ρg∫η² dA = π ρ g D² R² / 8; it is
half as deep as it is wide, at most 4 m deep (wider beyond, keeping its energy: half the field's 8 m bound), and no
narrower than 1.5 of the field's cells (shallower then, keeping its energy, so the rings far off keep their height while
the grid carries their length). It opens over √(R/g), a quarter of the period of the waves it sheds.

| Strike | Energy source | Waves | Crater R × D (High) |
| --- | --- | --- | --- |
| 5-inch round entering at 400 m/s | ½ m v² × 0.44 (naval-sim keeps 0.75 of the speed at entry), m = 13,800 d³ kg (half the cube of the calibre in inches, in pounds) | 20 kJ | 4.5 × 0.5 m (floored) |
| 5-inch HE burster, 3.3 kg at the surface | kg TNT × 4.184 MJ | 0.28 MJ | 4.5 × 1.9 m |
| 16-inch AP round at 460 m/s | as the 5-inch | 0.86 MJ | 5.4 × 2.7 m |
| 16-inch AP burster, 18.5 kg 2.5 m down | × e^(−depth / bubble radius), the bubble 3.4 (W / (z + 10))^⅓ m (Cole) | 0.81 MJ | 5.4 × 2.7 m |
| 16-inch HE burster, 70 kg at the surface | | 5.8 MJ | 9.6 × 4 m |
| 533 mm torpedo, 350 kg 4 m down | warhead 350 (d / 533 mm)³ kg | 20 MJ | 17.6 × 4 m |
| Aircraft, 4 t at 60 m/s | ½ m v² | 0.14 MJ | 4.5 × 1.3 m |
| Depth charge, 130 kg 30 m (15 m) down | a dome where the charge is deeper than its bubble | 28 kJ (0.86 MJ) | dome 4.5 × 0.5 m (5.4 × 2.7) |
| Magazine beside the water, 2 t at 6 m | most of a magazine vents into the air: 2 t taken | 117 MJ | 43 × 4 m |

The events (`naval-client` `src/scene/sea_strikes.rs`, wired in `effects.rs` beside the wakes, each taken once by sequence):
`splash` (the round's calibre and its velocity at entry), `burst` with `water_burst_y` (a burster in the water; its
charge from the blast radius, 3 ∛kg m, naval-sim's `burst.rs`; its depth below the sea there), `torpedo-hit` (by
diameter, at the hit's depth), an aerial torpedo's `water_entry`, `depth-charge-blast` (130 kg), `aircraft-crash` over
water (its velocity, else 60 m/s), and a `module` detonation within 15 m of the surface (a magazine). The splash's
column rises at its event (`splash.rs` throws its column at once, at most 0.1 s late), so the crater opens under it.
naval-sim emits no mine hits; bombs are rounds and bursters as shells are.

**The rings are deep water's.** The field steps ∂²h/∂t² = −g √(−∇²) h, ω² = g|k|, with `kernel.rs`'s fitted √(−∇²),
within 4 % of |k| from 6 cells to 12 × 2^levels cells: on High (512 cells of 3 m, 4 levels) 18 to 576 m, on Medium (6 m
cells, 3 levels) 36 to 576 m, on Ultra (1.5 m, 5 levels) 9 to 576 m. The hat's spectrum peaks at k = 2/R, waves πR
long: 17 m for a 16-inch round, 30 m for an HE burster, 55 m for a torpedo (phase speeds √(gλ/2π) of 5.2, 6.8 and 9.3
m/s, group speeds half that). A crater's train is dispersive, longest first: at distance r after t seconds the local
wavenumber is k = g t² / 4r² (the Cauchy–Poisson problem's stationary phase). `tests/wake.rs`
(`a_crater_rings_the_sea_with_deep_water_dispersion`) lays a 6 × 2 m crater and reads the zero crossings 25 s later:
24, 36, 60 and 78 m waves at 81, 96, 120 and 154 m, where deep water has 26.7, 37.6, 58.8 and 97.5 m (the last at
the train's thin leading edge). Rings shorter than 6 cells are slowed and damped by the kernel's roll-off: on Medium
the shortest rings are about 36 m whatever struck. The field's friction (0.065 /s, the web's, which keeps the wakes
local) halves the rings' height in 21 s, far faster than water's viscosity would (days for these lengths).

**Breaking.** The field's breaking foam uses a threshold tuned for the hulls' under-resolved waves (a slope of 0.015
over 12 m) times the fastest nearby hull's speed share, so a strike's rings passing a fast hull would have turned
white for 200 m. Each impulse opens a ring train instead (up to 16, the newest kept): its footprint grown at √(gR/2),
the group speed of the waves at k = 1/(2R), for 20 s after it last acts, fading over the last 5 s; inside it the hulls'
breaking is not taken, and the strike's own white water (laid over the crater, decaying over the field's 9 s and
spreading 3 m²/s) is the rings' breaking. A hull's shoulder foam near a fresh splash fades for those seconds. Since
2026-10-10 the threshold is taken only along the hulls' own sides, so rings outliving their train never break either
("Where the hulls' waves break" above).

**Foundering hulls** (`Sinkings`). naval-sim takes a flooding hull down by her level long before it calls her lost:
the Bismarck of `bismarck;;iowa:hard,iowa:hard,iowa:hard` (10 km, seed 1941, her guns held) settles past 1.5 m at
about 200 s, is 5 m down at 240 s, capsizes and goes down by the head with her stern in the air (her origin 11 m down
at 330 s, 49 m at 410 s), and only at 441 s, 79 m down, does `sinking` turn true; her wreck stays drawn on the bottom.
So a hull founders when her level, averaged over 8 s from her waterline so the sea's heave (or a first sight in a
trough) does not count, is more than 1.5 m down, or once she is lost; her air pours out at full strength once she has
settled another 3 % of her length (deck awash). All she stirs lies over her keel's footprint on the sea: its length
times the cosine of her trim, at least her beam (a bow standing on end covers a beam's breadth, not her length). While
she settles the sea lifts over it (a held head of −0.3 m × √(her size against a 250 × 36 m hull), overlapping 2 s
pulses every second, whose sin² envelopes sum to a steady lift, with lace over it), and air boils up over it as a
Poisson process of 1.5 a second at full strength: each a dome as high as a bubble plume's fountain, 1.3 (g Q)^⅔ / g for
a flux Q log-uniform over 0.5–8 m³/s × her size (0.4 to 2 m: the plume's velocity at the surface, 1.6 (g Q)^⅓, squared
over 2g), 0.12–0.25 of her beam across, white water over it. She has gone under when her highest end and her upper
works are (her origin's depth less L (½ |sin trim| + 0.15 cos trim), 0.15 L the upper works' height), or when she is
no longer drawn: the sea heaps over where she went once, as her last air comes up in bursts: a cluster of domes, one
for each beam of her footprint and two more (at most eight), each half to four fifths of her beam across, lifting by
half to all of 1 m × √size, coming up within 2 s of one another and lasting 3.6 to 6 s, white water over each: an
irregular boil the size of what went down there (`Watched::bulge`; until 2026-10-21 one capsule along 0.8 of her length
whatever her trim, which drew a long, straight-edged box of white water where Hood's bow went under standing on end).
Boils keep coming at 3 a second, dying away over 35 s; after 150 s the sea is quiet, though her wreck may still be
drawn. A wreck first
seen under (a scene loaded over it) stirs nothing, and a hull that founders for 600 s without going under (on a
shallow bottom) has spent her air. A submarine's dive is not foundering: she boils only once she is lost.

**Shown where.** Everywhere the wake field runs: Medium, High and Ultra's ocean (none on Low, which has no field), in
battles, theatre battles at the helm and the cruise scene alike; the field follows the focused hull, so a strike more
than 768 m from her (an enemy's splashes in a long-range action) leaves no rings, unless it falls about the hull the
view last left, whose field is kept. The hulls' wet band, the underwater
probe and the rain's splashes read the field's height (`wake_height`), so a hull beside a strike or over a foundering
keel takes the water as drawn.

**Cost.** None while nothing strikes the sea: the step's two loops run zero times, the uniform grows from 576 to 1,616
bytes a step (slots of 1,792; 1,872 in slots of 2,048 since the hulls' breaking stretches), and no sampler or binding is added to the sea's pipeline (the surface reads the same
display textures). With sixteen impulses and sixteen ring trains in every step, 30-step batches timed to completion on
the GPU (Apple M5 Pro, Metal, load average 5–7, three runs) took 0.30 ms a step on High's 512² either way and 0.97–0.98
ms on Ultra's 1,024²: the loops are a few ALU operations a texel in a pass bound by its texture reads, below what the
timing resolves. In the client (dev-client, 1600 × 900, the salvo below from 520 m up, six alternating runs), the pass
`ocean wake step` took 0.25–0.42 ms on both builds, before and through the salvo. CPU: a few vector pushes per event and
per foundering hull a frame.

**Seen** (`.build/native/look-sea-ripples/`, `ripples.ts` scouts a battle, replays it to each moment and draws it from a
low and a high camera with this build and master's; `pair-*.png` are before | after | difference ×4): a 16-inch AP
salvo falling 140–700 m about a Bismarck (`bismarck;;iowa:normal`, 14 km, seed 1941, the salvo at tick 1960), at the
map's 9 m/s and at 3 m/s; a 533 mm torpedo hit on her bow (`bismarck;;fletcher:hard,fletcher:hard,fletcher:hard`,
6 km, tick 21543); her foundering under three Iowas (above). At 3 m/s the rings show in the sun's glitter from 520 m
up, concentric inside each splash's foam ring and 40–60 m across at 12 s; in the 9 m/s sea (2.4 m significant height,
an rms slope near 0.2) rings of 0.1–0.3 m are a faint change in the glitter and foam about each column, as they would
be at sea. The torpedo's 55 m rings move her wet band along her side and, 20 s on, the whole sea in the foreground
(13 % of the frame's pixels change by more than 8 levels, none of it a visible pattern). Foundering, white water boils
up along her as she goes down by the head. Stills with no event (`--camera broadside` and from 600 m) draw master's
bytes.

**Open.** The rings are real-sized, so most of them are hidden in a seaway. Rings stop at the field's edge (768 m about
the focused hull). A hull is no obstacle to them (they pass under her as the hulls' own waves do). Impact energy shares
are less certain than explosions'. No mine events exist in naval-sim.

## Trails

`src/trail.rs` (`WakeFoam`) samples each hull's course every 3 m of travel and paints her churned band, her slick and
the splashes' patches into an 8 × 8 tile atlas, each tile's painted boxes in the `Wake` uniform (`src/stamps.rs`,
`shaders/stamps.wgsl`: texel-aligned instanced quads, `max` blending). The surface reads it in `wake.wgsl`
(`wake_surface`: the normal, foam, bubbles and the slick, which stills each cascade's short waves through
`WaveUniforms::slick_shares`).

Since 2026-10-07 every hull keeps its sampled course even without one of the atlas's drawing slots. Changing the
focused ship assigns a slot to her existing trail instead of starting it from scratch. The theatre map continues
the CPU histories while hiding the scene; a rebuilt scene takes those histories in its new local coordinates.
Fast theatre frames allow the distance a ship can actually travel before rejecting a pose as a teleport.
The atlas remains the same size. `trail::tests::changing_atlas_slots_keeps_the_unwatched_ships_course_history`
and the client's `tests/effect_history.rs` cover the continuity.

The browser game's realistic wake drew one band of dense white water a beam wide, painted to its edge, that faded as a
whole. Real wakes (aerial and stern photographs) stay white only behind the stern; further astern the white water draws
out along the track into a braided rope of streaks down the middle with sparse lace on either side and a thin line
along each edge, over turquoise water lit by the bubble cloud, and a faint line of foam lasts as long as the slick.
Native-only changes:

- **Where a point lies across its band.** Each band stamp records its centre's offset from the band's centreline and its
  half-width there (`Stamp::across`; `NO_BAND` for splash rings and torpedo tracks), and paints how central each point
  is into the atlas's blue (churned water) or alpha (slick), now `Rgba8Unorm` (`stamps.wgsl`, `TRACK_RANGE`,
  `TRACK_ZERO`). Stamps write it past their extent (a quarter of it, and at least 4.5 m and a texel since 2026-10-05) so
  it holds to the band's edge. Every stamp of a band agrees on it at a point; where bands overlap (a ship crossing her
  own trail) max blending keeps the most central, and filtering toward an empty texel reads as the edge. The tile rows
  carry each hull's beam (`slick_center.z`).
- **Old water streaks along the track** (`wake.wgsl` `trail_read`, `wake_streaks`): streak noise fine across the band
  (16 per half-width) and drawn out along it (0.7 beams), braided by warps at two scales, read where the point lies,
  not through the warp that frays the outline. Its cover: 0.6 in a core out to 0.32 of the half-width, 0.1 across the
  rest, 0.55 in a line at 0.9; the band's edges wander ±25 %. "Young" water (turbulence 0.45–0.85, about the first
  150–400 m at full speed) keeps the eddies, a little streaked, thinning to half toward the sides. Old water is lace
  (0.7 of its cover at 0.75 opacity, `churned_water`'s `young`), young water the dense white water as before.
- **The bubble cloud** fills the band, densest in the middle and patchy like the foam, fading over its outer third; it
  is brighter and more turquoise (`BUBBLE_STRENGTH` 0.75, `BUBBLE_ALBEDO` 0.42, `AERATED_MATTE` 0.6).
- **The slick's middle** keeps a line of foam streaks (cover 0.16 within 0.22 of its half-width) and a residue of
  bubbles (0.2, was 0.08) for its four minutes; its edges wander and fade over the outer half instead of stopping.
- **Lifetimes**: churned water e-folds in 30 s (was 24) and lasts 90 s (was 55), fading from 60 s.
- **No foam at zero cover** (`torn_foam`): a threshold at 1 − cover with a fixed tear let the noise's peaks through
  where the cover was nothing, which drew white crescents beside the wake.

Cost: whole frames at 1600 × 900 over a Bismarck's wake, old and new shaders swapped by hot reload in alternating
rounds, medians −0.02 to +0.45 ms across three views (Apple M5 Pro, load average 35; within the run's noise).

Since (2026-10-04), against renders where the wake was a saturated teal band with a hard outer edge and painted streaks:

- **Lace, not a mask**: the churned water's mask tears over at least 0.3 of its pattern (`WAKE_TEAR`; the `tear` knob
  is 0.1), and churned water thins with its energy (`CHURN_THIN`: 0.4 opaque where it is nearly gone), so the band
  frays into the sea. Streaks are shorter and wider (9 per half-width, 0.45 beams long), the edge lines fainter (cover
  0.3) and the outline wanders ±40 % at two scales. Old churned water is sparser lace (0.62 cover, 0.55 opaque).
- **Pale turquoise close astern only**: the bubble cloud thins from 0.3 of the half-width to the edge and keeps 40 % of
  itself once the water is old; the cloud's colour is paler (`foam.wgsl` `BUBBLE_PATH` 2.6 m).
- **Splash patches** (no band) are dense white water only above 0.7 turbulence (`SPLASH_YOUNG`); elsewhere lace (since
  2026-10-08 a soft patch, not a ring: "A splash's patch" below).
- **The waterline**: the surface's shoreline foam, which drew a crisp white stripe along every hull, mostly gives way
  beside a hull to `hull_foam` ([foam.md](foam.md), "Foam shading").

Since (2026-10-05), against a wake that came out behind the stern in jerks and showed straight-edged patches:

- **The band keeps up with the stern.** Samples are laid every 3 m of travel and the trail painted at 20 Hz, so the
  band's end fell up to a sample and a repaint behind the stern and jumped after her: a destroyer at full speed left her
  transom bare and white again several times a second (the churned water at her stern swung from 0 to 0.95). Every rasterisation now paints a head,
  churn and slick, where the hull stands (`WakeFoam::head`), and the stern keeps over 0.75 of its coverage
  (`the_band_keeps_up_with_the_stern`).
- **No straight cuts.** The surface reads the band's coverage through a warp of up to about 3.75 m but where a point
  lies across the band where it is, and a band's stamps wrote that coordinate only a quarter of their extent past
  their coverage. Where the warp reached further (a destroyer's band is 1–2 texels of 6 m across), the coordinate was
  missing and the pixel was shaded as a splash: dense white water and a full bubble cloud, cut along the atlas's
  texels. A slick read where its coordinate was missing stilled the waves at full strength out to a 16 m texel's edge.
  Band stamps now write it at least 4.5 m and a texel past their coverage (`stamps.rs`, `TRACK_REACH`), coverage read
  where no band says where it lies counts as outside the band, and a slick there as past its edge.

**A splash's patch** (`trail.rs` impacts, `wake.wgsl`): a splash stamps one soft disc that spreads to about
33 m × scale in 6 s and keeps growing slowly, its strength falling from about 0.9 to 0.45 in the first seconds and out
between 6 s and 30 s (`IMPACT_LIFETIME`), and a brighter front ring for the first 10 s (the base surge). The shader
keeps a splash's energy smooth, mottled broadly over 2.8 eddy lengths (never below 0.3 of its cover), and lets the
surface's foam texture tear it into lace (`churned_water`), so the patch thins as it fades instead of being cut along
the eddies. Dense white water only under the column's first second or two (`SPLASH_YOUNG`). Seen low across the sea it
is the bright ellipse under a splash column.

## Torpedo tracks

`src/torpedo_tracks.rs` lays each running torpedo's track (its exhaust surfacing after a delay, drifting and
expiring) into a 1,536 m field at 1,024² (`R8Unorm`, binding 10), shaded as churned water and bubbles.

The torpedo track field follows the water under the view and a round near the sight line at every zoom. Previously
only a magnified lens followed a distant round, so zooming out could centre the 1,536 m field on the camera and drop
the visible wake. Moving the camera while paused repositions the field without ageing the water.

### An oxygen torpedo's faint track

An air torpedo's exhaust is mostly the nitrogen of its air, which does not dissolve: it rises as the bubble track. The
Type 93 burnt its fuel in pure oxygen, and its carbon dioxide dissolves and its steam condenses on the way up, so it
left almost no bubbles; the simulation's lookouts pick out its wake at 0.3 of an air torpedo's range
(`fleet_evasion::OXYGEN_WAKE`). The render packet carries `TorpedoRender::oxygen` (`Torpedo::oxygen`, every team's,
since the faint wake is what anyone sees of it). An oxygen torpedo's surface track is laid at `OXYGEN_TRACK` (0.2) of
an air torpedo's strength and thins over 5 s rather than 14 (`torpedo_tracks.rs`): the churned water's cover
(`churned_water`, a smoothstep from 0) leaves a few specks of the propellers' wash for some seconds, not a white line.
Under the sea its trail shows `OXYGEN_BUBBLES` (0.12) of the bubbles (`underwater_bubbles.wgsl`, the trail's
`seed.y`). The launch splash is unchanged.

## Bind groups

The surface already used the four bind groups WebGPU guarantees (0 camera, 1 surface, 2 wave field, 3 sky environment),
so the wakes take no group of their own. They join the surface's group 1 at bindings 5–10:

| Binding | What | Stages |
| --- | --- | --- |
| 5 | `Wake` uniform (39,664 bytes): the field's display origins and weights, trail and torpedo clocks and reach, shading knobs, bow wave slots and knobs, contact foam slots (8 × 263 vec4), 64 trail tiles × 5 rows | vertex, fragment |
| 6, 7 | the wake field's two display textures (`Rgba16Float`), blended by the step fraction without rebinding when they ping-pong | vertex, fragment |
| 8 | linear clamp sampler (wake field, trail atlas, torpedo field) | vertex, fragment |
| 9 | trail atlas (`Rgba8Unorm`, 2,048² on High: churned water in red, slick in green, where each lies across its band in blue and alpha) | fragment |
| 10 | torpedo field (`R8Unorm`, 1,024²) | fragment |
| 16 | oil atlas (`Rg8Unorm`, 1,536², since 2026-10-20: each leaking hull's oil film on a fine and a wide square; read by the surface's `OIL` variant alone, [oil.md](oil.md)) | fragment |

Since 2026-10-20 the `Wake` uniform also carries the oil slicks' rows after the contact foam's (40,560 bytes in all;
40,688 with the stern's waves, [bow-waves.md](bow-waves.md)).

One packed uniform, as the web packs its arrays with `PackedVec4Arrays`, keeps the surface at 5 uniform buffers, 11
sampled textures and 4 samplers per stage, well inside the defaults. `OceanSurface::set_wake` binds a `ShipWake` (or a
calm one: zeroed uniform, black 1 × 1 textures, which reproduces the open sea exactly); `Ocean::enable_wake` makes one for
the tier (field resolution 0/256/512/1,024 for Low/Medium/High/Ultra, as the web's `quality.ts` (historical)) and rebinds when the field is
reset. The render core II lane's note that group 0 will carry shadow maps changes nothing here: every pipeline takes
`ctx.camera_layout`.

## Cost

Apple M5 Pro, Metal, 2026-09-30, load average 85–100 all day (other agents' builds and browsers), High tier:

- **Wake field**: 0.44–0.59 ms per 512² step (reduce and collapse over four levels, the step, the display; batches of 30
  timed to completion, as the web's diagnostic times its steps). The web's own step on the same machine and load: 0.74 ms.
  Steps run at 30 Hz, so about 0.25 ms a 60 Hz frame.
- **Trail atlas repaint**: 0.14–0.27 ms for 500–600 stamps; it repaints at 20 Hz near (5 Hz beyond 2.5 km), under 0.1 ms
  a frame.
- **Surface shading of the wakes**: below the noise of serialised whole frames (2.9–3.0 ms at 1600 × 900 with and
  without the wakes bound, 150 interleaved frames each); the sampler exits early outside the trails' boxes and the bow
  wave and contact foam reaches. Measure again on a quiet machine before budgeting.
- **CPU wake update** (trails, torpedo tracks, bow waves, contact foam, packing): a 941-frame replay of 90 s takes about
  1 s including every frame's GPU submission.

## Tests

`naval-ocean`: `wake::kernel` (the pyramid sums to zero, follows |k| within 4 % from 6 to 12 × 2^levels cells, stays
positive, matches a CPU reduce/collapse), `wake::generators`, `trail`, `torpedo_tracks`, `bow_waves`, `hull_profile`,
every shader composes and validates, and `tests/wake.rs` (the GPU field against the web's diagnostic figures, the
strikes' rings, the field across a change of view). `naval-water-effects`' `tests/fixture_playback.rs` drives the
trails, tracks, bow waves and contact foam from 100 s of a recorded battle twice with identical results.

