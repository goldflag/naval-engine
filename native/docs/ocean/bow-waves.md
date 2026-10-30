# Bow waves and the stern's waves

The analytic waves a hull raises about her (`src/bow_waves.rs`, `shaders/wake.wgsl`'s `bow_waves` and `stern_shape`)
and the white water on them; the wake field's own Kelvin waves are [wakes.md](wakes.md). Index: [README.md](README.md).

## The stem crest

### The crest's measured height

The owner noticed how little water a bow raised at low speed. The stem crest was `crest` (0.3) × U²/2g × the beam
over 32 m (within 0.45 and 1.1), capped at 4 m: pure U², set to look right at full speed. Measured bow waves grow more
slowly than that. Noblesse et al. 2008 (the paper `crest_peak` already follows) fit towed wedge bows with
gZ_b/U² ≈ 2.2 tan α ÷ (1 + F cos α), α the waterline's half angle of entrance and F = U/√(g·draft). So the old curve
stood about half the measured height at cruising speeds, and a third of it on a destroyer, whose narrow beam it
scaled down to the floor.

`stem_crest` now takes her draft and her entrance (`entrance_slope`: the design waterline's half-breadth 0.05 L aft of
the stem over that distance, from the hull's own slice, within 5° and 25°; 11° without a slice) and draws the crest
line at `crest` (0.8) of the measured height, so the water against the side (× 1.27, "The crest along the hull" below) stands at it. The beam no
longer enters. The bow spray reads the same slope (`BowShape::slope`), so its sheets rise from the same crest. The
Kelvin V is still 0.9 × the crest and the white water still gates on the crest's height and steepness, so both follow.

The roster's entrances (tan α) run from 0.14 (Atlanta) through 0.17 (Iowa), 0.21 (Fletcher) and 0.25 (Yamato) to
0.30 (Omaha), with Fuso's model an outlier at 0.40. The crest line before → after, in metres:

| Hull | 10 kn | Full speed |
| --- | --- | --- |
| Fletcher (36.5 kn) | 0.18 → 0.54 | 2.43 → 3.27 |
| Atlanta (32.5 kn) | 0.21 → 0.39 | 2.19 → 2.19 |
| Iowa (33 kn) | 0.42 → 0.53 | 4.00 → 3.23 |
| Yamato (27 kn) | 0.45 → 0.81 | 3.24 → 3.76 |
| Bismarck (30 kn) | 0.45 → 0.56 | 4.00 → 2.98 |
| Enterprise (32.5 kn) | 0.36 → 0.47 | 3.75 → 2.71 |
| Warspite (24 kn) | 0.43 → 0.89 | 2.48 → 3.52 |
| Fuso (24.7 kn) | 0.41 → 1.28 | 2.52 → 4.00 (the cap) |
| Flower corvette (16 kn) | 0.18 → 0.61 | 0.47 → 1.23 |

Fine, fast hulls come down a little at full speed and fuller, older or slower ones go up; every hull is higher at
cruising speeds. At 5 kn nothing breaks (the white water's height gate); by 10 kn a destroyer's crest starts a thin
ribbon. The over-the-bow calibration ([water-effects.md](../water-effects.md), "Spray over the bow and water aboard") has not been run again on the new heights.

### The crest along the hull

The owner's reference (a Nimitz-class carrier at speed from her port bow): white water starts at the stem, a thin
bright ribbon hugs the side and peels away, and the sea ahead of the bow is undisturbed. The client drew three things
that photograph does not show:

- **White water ahead of the bow.** The crest's ramp began 0.04 of the hull's length ahead of the stem and the heap
  about it was a Gaussian centred 0.02 L aft, 0.07 L long and half a beam wide: a mound that stood 15 to 20 m ahead of a
  battleship's stem and as far abeam, with the stem's white water (a Gaussian about the stem, fore and aft alike) on
  top of it. The stem itself was placed 0.49 L ahead of the origin, the end of her length: 0.4 m ahead of a Fletcher's
  stem at the waterline, 2.7 to 3.9 m ahead of Enterprise's, Yamato's, Iowa's and Bismarck's, under their bows' rake.
- **A fountain.** The sheets left that mound, already at its full height at the stem, upward at 0.45 of the hull's speed
  and outboard at 0.55: 2 m over the crest and 8 m out, over a destroyer's hull number.
- **A mound, not a wave.** The crest was a Gaussian ridge 0.12 beam + 1.5 m in half width (6.2 m for Yamato) on a
  parabolic entrance twice as full near the stem as a battleship's.

What draws it now (`bow_waves.rs`, `shaders/wake.wgsl` `bow_waves`, `bow_spray.rs`):

- **Nothing ahead of the stem.** The crest climbs the side from the stem (0.15 of the peak's distance ahead of it, so
  the surface meets the stem without a step) and stands highest `crest_peak` aft of it: 1.1 U²/g ÷ (1 + U/√(g·draft))
  ([Noblesse et al. 2008](https://doi.org/10.1017/S0022112008000220), "Simple analytical relations for ship bow
  waves"), 7 to 9 m for a battleship at 25 kn and a destroyer at 30. The heap (`hump`, 0.7 → 0.3 of the crest) is the
  water standing against the bow's sides: 0.2 beam + 1 m out from the side, not half a beam about the centreline. The
  white water starts at the stem.
- **The hull's own waterline.** The stem is the foremost point of the sliced waterline (`bow_waves::waterline_stem`),
  and the crest follows the hull's half-breadths an eighth, a quarter, a half and the whole of 0.3 L aft of it
  (`bow_waves::entrance`, a fifth vec4 a slot: the `Wake` uniform is 39,664 bytes). Yamato's are 2.6, 4.7, 9.3 and
  15.8 m against the parabola's 4.6, 8.5, 14.6 and 19.5; Iowa's 1.9, 3.2, 6.5 and 13.1 against 3.9, 7.2, 12.4 and 16.5.
- **A crest with a face.** Half width 0.05 beam + 0.8 m (2.7 m for Yamato, 1.4 m for a Fletcher), its inboard side 2.2
  times as broad as its outer face, a trough inboard of that. The vertex stage draws it no narrower than 1.6 cells of
  the water mesh.
- **White water with an age.** A ribbon along the crest's outer face where the sheet plunges (`foam_length` 2.5 crest
  decay lengths, gone by 0.5 L), the sheet's root against the side from the stem to the peak, and what the breaker left
  on the water the crest has passed, which fades over 0.16 L downstream: the farther inboard of the crest line, the
  longer ago. That residue and the wash along the side are drawn as part lace (`BowWaveSample::young` 0.7, read by
  `churned_water`) and torn into streaks along the flow; the noise's clumps are gone.
- **Sheets that climb only as far as the crest.** A strand leaves the crest line 1.1 times as high as the crest there,
  climbing at the speed that carries it to the crest's peak (`Bow::rise_at`: √(2g × what is left to climb), nothing
  from the peak aft) and outboard at 0.45 of the hull's speed, a little faster than the crest line leaves her track.
  A sea met adds up to 0.2 of her speed upward and a plunge 0.55, so a bow driving into a sea still throws it up.
  Tufts are 0.6 the size.

**A slicer bug found on the way.** `hull_slice.rs` took a triangle for a level only when it crossed it. A hull modelled
with an edge loop on the design waterline has none: Yamato's waterline slice held 7 of 30 stations, so she had no
contact foam along most of her side and her shelter read nothing there. Each level is now sliced 1 mm over its height
(`LEVEL_NUDGE`; `an_edge_loop_on_the_waterline_still_slices`).

**What a bow ships.** The readback of the drawn sea about a stem (`wet_ships.rs`) now reads 0.08 L aft of it (0.04
before), so the box holds the crest's peak. The crest with the water against the side is 1.27 times the stem crest
where it was 1.65 at the stem itself: a Fletcher's at 18.5 m/s stands 3.0 m, not 3.9 m. The over-the-bow calibration ([water-effects.md](../water-effects.md))
was measured on the taller one and has not been run again.

Seen in the native client against master's build, in the default seaway at 7 m/s of wind: Yamato at 12.7 m/s, a
Fletcher at 15.2 and Enterprise at 14.8, each from high on the port bow, the whole ship from the bow quarter, overhead,
low on the bow, abeam of the bow, ahead and the stern quarter. Not measured: the frame's cost (the fragment stage
trades one Gaussian for a four-branch breadth lookup).

### The bow's white water

The stem's white water used a 0.4–1.2 m crest-height gate and a high noise threshold. A Fletcher at 8 m/s raised a
0.44 m crest but drew almost no breaking foam. The bow's white water now follows the crest's steepness, with a separate
height gate keeping slow hulls quiet. The visual aim is the spreading, broken white water visible in the Navy's
[McCampbell underway photograph](https://www.dvidshub.net/image/6143119/uss-mccampbell-aerial-shots).

- The foam spreads farther outboard and lasts longer along the bow (`BowWaveTuning`: foam 1.15, length 2.8, spread
  0.12, side wash 0.7). It fades before the analytic bow region's bounds, keeping the edge soft. (Pulled back in
  to the crest on 2026-10-08: "White water on the V and at the bow" below.)
- The world-anchored strands retain more of the thinning foam instead of discarding it at the noise threshold.
- `BowWaveSample` carries a faint bubble cloud beneath and just beyond the foam, combined with the trail's bubbles
  in `wake_surface`; the cloud has the same moving breakup as the white water. (Since 2026-10-08 it lies only under
  the white water.)

The crest's height, the Kelvin V's height and the spray's launch height keep the proportions of "The V's height" below. (The crest's place
and shape, its white water and the sheets changed on 2026-10-12: "The crest along the hull" above.)

Verified in the native client from the bow quarter and a low camera: Fletcher at 2.6, 9.6 and 17.0 m/s and Bismarck
at 13.5 m/s in a 5 m/s wind, plus Fletcher at 11.5 m/s in a 12 m/s wind under overcast skies. The 76 ocean unit tests
(including shader composition and bow-height proportions) and `parity-wake-astern` pass. Captures: `.build/native/bow-wave/`.

## The Kelvin V

### The V's height

The Kelvin V's amplitude (`BowSlot` wave.y) was `kelvin` (1.6) × U²/2g × √(B/L): speed alone, nothing of the hull's
size. A Fletcher at 36 kn (Froude 0.55) drew a V of ±10–12 m about a 3.8 m draft and 6 m of freeboard: the divergent
waves heaped the sea over her forecastle 0.13 L aft of the stem (+8 m on her side) and the transverse trough bared her
bottom at 0.68 L (−5 m). It is now `kelvin` (0.9) × the stem crest (`stem_crest`, capped at 4 m; its height is the
measured bow wave's since 2026-10-15, "The crest's measured height" above), so every hull keeps the crest's proportion. Measured with a CPU mirror of the vertex stage (the hull side
0.3 m out, then the sea within 3 L astern and 1 L abeam):

| Hull at full speed | Hull side, highest / lowest (m) | V about her (m) | Draft (m) |
| --- | --- | --- | --- |
| Fletcher, 18.5 m/s | +8.0 / −5.2 → +3.7 (the stem) / −1.3 | +11.9 −9.1 → +3.3 −2.1 | 3.8 |
| Gleaves, 19.3 m/s | +8.8 / −5.7 → +4.0 / −1.4 | +13.2 −10.0 → +3.7 −2.4 | 3.9 |
| Atlanta, 16.7 m/s | +4.5 / −4.3 → +3.4 / −1.3 | +5.0 −6.1 → +1.6 −1.5 | 6.3 |
| Bismarck, 15.4 m/s | +6.2 / −3.8 → +6.2 / −1.9 | +3.4 −4.0 → +1.7 −1.8 | 9.3 |

The stem crest itself is unchanged (a Fletcher's 3.7 m with the heap is about what a fine bow raises at 36 kn). The
surface still shades the V, its slopes and the white water on its crests, at the old amplitude (`kelvin_shading`, 1.6 ×
U²/2g × √(B/L), carried in the slot's fourth vec4): long and low at its true height, the V did not read from astern at
all (`parity-wake-astern`, a Fletcher at 20 kn, lost it entirely; its reference was blessed again for the lower
transverse trough). Only the drawn height, which the hull meets, keeps the crest's proportion. The bow spray's surges
stop throwing sheets over a destroyer's bridge, since the sea no longer rose 8 m beside her stem.

### The crest train's spacing

From above, the V showed a few isolated whitecaps with large gaps between them. The visual reference for this pass is
the user's aerial container-ship photograph: a feathered train of broken crests along both arms, extending well astern.
`BowWaveTuning::kelvin_wavelength` now gives the V its own spacing (0.3 of the speed-derived wavelength), while
`speed_scale` still controls the stem crest's decay and the bow spray. This is an artistic calibration: the physical
deep-water wavelength varies with speed squared ([Rabaud and Moisy](https://arxiv.org/abs/1304.2653)), and the photo's
ship speed is unknown.

The V reaches 4.5 hull lengths from the stem instead of 2.6, with the foam's decay length increased from 2 to 3.5 hull
lengths (1 since 2026-10-08, when the crests stopped breaking along the arms: the next section). The analytic source depth is 0.12 beam instead of 0.25 so it does not suppress the shorter crests. Height,
slopes and crest foam all read the same wavelength; the existing pixel/mesh filtering, curved track, cusp angle and
soft outer fade still apply. The stem's height, decay and spray launch shape keep their previous tuning.

Verified against an isolated `2f834d25dc17` baseline in overhead and astern views: Fletcher at 9.9 and Bismarck at
13.8 m/s, plus Fletcher at 2.7 and 18.1 m/s and in a 12 m/s wind at 11.5 m/s. The 83 ocean tests and Clippy pass.
`parity-wake-astern` retains its wake-visibility assertion; its image reference was refreshed for the closer,
longer crest train. Captures and logs: `.build/native/kelvin-crests/`.

### White water on the V and at the bow

The owner's references (a carrier group from altitude, an Iowa and a Midway from low over the water, a battleship, an
oiler and a carrier from directly above) show the Kelvin arms as thin dark ripple lines with no foam on them, and at
the bow a narrow white strip hugging the hull. The client broke every divergent crest near the cusp in white for three
hull lengths astern, and paled the sea far outboard of the bow.

- **The arms.** The crests' white water came only from `crest_foam` in `wake.wgsl` (the dispersive wake field adds none
  there: switching the analytic terms off left the arms clear). Its steepness gate read the slope the surface is
  shaded at (`kelvin_shading`, 4.2 times the drawn height for a Fletcher, 2 for an Iowa), so it was always open. It
  now reads the drawn wave's steepness (ak), opening at `kelvin_steep` 0.06 and full at 3.3 times that, fades over 1
  hull length instead of 3.5, and is fainter (`kelvin_foam` 0.7). Only a fast hull's first crests break.
- **The bow.** The paler area was the bubble cloud: 0.3 of a band 1.5 times as wide as the white water's. It now
  lies only under the white water, at 0.22 of it. The crest's white water stays near
  the crest (`foam_length` 1.3 of the crest's decay lengths instead of 2.8, `foam_spread` 0.05 m per m instead of 0.12,
  a band 1.1 instead of 1.3 times the crest's width) and is gone by 0.4 of her length instead of 0.8.
- **Kept:** the wash along the hull's sides (`wash` 0.7, `wash_width` 3.5 m, out to 0.75 L), which the owner likes,
  and the stem's own white water.

Measured with a CPU mirror of the fragment stage before its noise (white water over 0.2, the paler sea where the
bubbles pass 0.1; scratch, not in the tree):

| Hull | White water (m²) | Paler sea (m²) and its reach out from the centreline | Crest foam on the V |
| --- | --- | --- | --- |
| Fletcher, 17.0 m/s | 1,319 → 852 | 1,960 → 902, 22 → 13 m | 843 m² to 2.7 L → 7 m² within 0.6 L |
| Fletcher, 9.6 m/s | 372 → 151 | 1,187 → 354, 17 → 10 m | 791 m² → none |
| Iowa, 14.6 m/s | 3,000 → 2,024 | 5,891 → 2,301, 43 → 24 m | 4,414 m² to 3.1 L → 14 m² within 0.2 L |
| Bismarck, 14.0 m/s | 2,888 → 1,965 | 5,559 → 2,183, 43 → 24 m | 3,716 m² → none |

From astern a fast destroyer's V still shows bright bands and grainy grey strips on its crests: the slopes shaded at
`kelvin_shading` and the sun's glitter on their steep faces, not foam (a render with every wake's foam and bubbles off
shows them unchanged). `kelvin_shading` is the knob if the arms should read fainter.

Seen in the native client against master's build: an Iowa at 14.6 m/s and a Fletcher at 17.0 and 9.6 m/s, from
380 m up ahead, overhead the bow, low on the bow quarter, close on the bow and from astern, in a 5 m/s wind; and the
port's Bismarck. The 107 ocean tests pass; `parity-wake-astern` was blessed again (its old reference was already 4.8
levels off on master; this change moves it 0.9, the white dashes on the arms). `chase-one-ship`, `fleet-measure30`,
`port-bismarck` and `theatre-cruise` fail on master's build by the same margins (stale references; `port-bismarck`
moves 1.1 levels nearer its reference here). Captures: `.build/native/wake-tune/`.

### Kelvin arms as thin lines

The owner's reference, a carrier group from some 500 m up, shows each hull's V as two thin dark lines of short crests
in echelon, a few white flecks near the bow, quiet water between them, running many hull lengths astern until they
cross the next ship's. The client drew a fan of evenly spaced ripples filling the V for a hull length or two and
nothing past 4.5 lengths (a Fletcher's arms ended 510 m astern), and the far end of the V swung with every yaw.

- **The cusp line.** `bow_waves` in `wake.wgsl` takes how far inside the 19.47° line a point lies as the Airy
  function's argument, ζ = 1.96 (k₀·run)^(2/3) (1/√8 − t): the two systems' phase difference, linearised about the cusp
  (`CUSP_ZETA`). Outside the line the crest dies away as Ai does (exp(−⅔ x^(3/2) − x/4)), within metres at any range,
  where a fixed angular fade (t² from 0.095 to 0.125) used to stand: 46 m wide 1 km astern. Inside it the first fringe
  keeps 1.15 of the V's shaded slope (`CUSP_LINE_GAIN`) and the fringes within it 0.15
  (`BowWaveTuning::kelvin_interior`, in the knob the shader no longer reads the reach from). The drawn height, which
  the hull meets, keeps the interior. The band is some 20 m wide 300 m astern of a hull at 15 m/s and 40 m wide 2 km
  astern.
- **Shorter crests.** `kelvin_wavelength` 0.3 → 0.2 of the speed-derived wavelength: denser dashes along each arm.
- **As far as her track follows the arc.** The V reaches 12 hull lengths (`reach`, was 4.5) where her track allows.
  `BowWaves` keeps each hull's track a point every 20 m and draws the V no farther than two thirds of the way back to
  the first kept point that strays from the arc it is laid on by more than 4 % of the run and half her beam (the cusp
  `run` astern was raised 1.5 run back along her track: its waves leave 35.26° off her heading at their group
  velocity). In a turn the arms shrink, eased over a second, and grow again along the new track instead of swinging
  a kilometre with the helm. A track not yet kept (a new hull, a jump of 100 m) counts as following it.
- **The arc from the track, not the heading.** The V's curvature was her heading's change over distance, smoothed over
  1.2 s; a destroyer's yaw in the default seaway made it wander by ±0.0006 per metre, which moved a 4.5-length arm's end
  by tens of metres a second. It is now the circle through where she is and her kept track 0.4 and 0.8 of her length
  back (`fitted_bend`), smoothed the same, and the V leaves the stem along her course over the ground (`course`, a sixth
  vec4 a slot: the `Wake` uniform is 39,792 bytes), not her heading, which yaws and drifts in a turn: 5° between them
  put the end of a 1.4 km arm 120 m off her track. The bow's crest keeps her heading. The V's bounds are tested along
  its own arc, so the inner arm of a hull in a long turn is not cut off by a box drawn about her heading.
- **Far arms.** The V's crests are drawn down to three pixels (`KELVIN_FINEST`; five, `finest`, before): they stand
  still in a view that follows the hull. The slope variance of crests too short to draw roughens the sea instead
  (`BowWaveSample::variance`, `KELVIN_ROUGHNESS` 0.12, patchy along the arm over 180 m; `surface.wgsl` adds it to the
  reflection's spread as it does the rain's): a far arm reads as a line of rougher water, darker from aloft at noon.
- **Cost.** Past two to four hull lengths astern only the two bands are evaluated (ζ under 6 to 9,
  `CUSP_INTERIOR_FROM`, `CUSP_INTERIOR_TO`), so a long V costs two strips of sea, not the whole fan. Measured at
  1920 × 1080 from 420 m up ahead of the carrier group and 90 m up on her quarter (GPU busy, medians of three
  alternating rounds, one binary per variant, Apple M5 Pro shared with other sessions): no bow waves 7.79 / 7.57 ms,
  the V cut at 4.5 lengths 8.14 / 7.91, at 12 lengths 8.15 / 7.91.

Seen in the native client against master's build, in the default seaway at 5 m/s of wind, noon: Enterprise with a
Fletcher, an Atlanta and a Gleaves in company at 15 m/s from 420 m up ahead (the reference's view), from 90 m up on the
quarter and from the chase camera; a Fletcher alone at 15 m/s from 420 m up on the bow quarter, 60 m up astern and the
chase camera; and a Fletcher 25 s into a hard turn. `parity-wake-astern` was blessed again: it was already 7.0 levels
off its reference on master and this change moves it to 7.4 (the rings across her track gone, the arms with flecks).
Captures: `.build/native/kelvin-photo/`.

The same day the arms were made much fainter at Bill's request: `kelvin_shading` 1.6 → 0.7, the V's shaded slopes 0.44
of what they were and its unresolved roughness, which goes as their square, a fifth. From aloft each ship keeps two
faint thin lines; from 90 m up on the quarter the carrier's arm is barely there; close astern of a fast destroyer the
ribs are softer but still read. The drawn height, which the hull meets and whose steepness breaks the first crests, is
unchanged.

## Hull-side wash and fast hulls

The side wash used the bow wave's approximate design-waterline entrance and yaw, while the contact foam used the
drawn hull's profile, heave, pitch and roll. The two bands could separate, leave gaps or put white water inside the
side. `wake.wgsl` now evaluates wash with the contact profile at the displaced water point, clips it at the side and
keeps its original 0.7 strength, 3.5 m width, downstream fade, streaks and age. A GPU pose regression puts master's
wash peak 0.56 m away from the tilted hull's band; the corrected peak follows the side within 0.12 m on a 5 cm grid.
Underway hulls take the contact slots before stopped hulls, with camera distance deciding within each group, so nearer
stopped hulls cannot displace an underway hull's side wash.

Both bands start at the plating (2026-10-29). The contact line peaked a third of its width outboard of the side
(`blurred * 0.3`) and the wash half of its (`* 0.5`), which left darker water against the plating, most of all from
above, where her shadow falls on it. Each is now full against the side and falls away outboard, the line over 1.3 of
its width and the wash over 1.6 of its, so each still reaches half strength about where it did; inside the side
(where the profile stands proud of the drawn hull) both fade over 0.6 m as before. `side_wash_follows_the_drawn_hull`
finds the wash's peak on the side within 0.12 m through heave, pitch and roll.

The water a fast hull heaps against her side read as glass: clean water under the wash, lit through by `wave_light`.
From her stem to her waist (the wash's own downstream fade), out to the wash's width from her side (3.5 m, falling as
a Gaussian), the water of a hull under way holds bubbles in the wash's streaks (up to 0.2, `FROTH_BUBBLES`) and gives
back 0.7 of the light through the waves (`HullFoamSample.froth` into `WakeSurface.froth`, which the surface multiplies
into `through`). From above the band against her side reads as aerated water with the wash's lace reaching her
plating; in low views at noon her bow wave's turquoise dome changes little, since most of that colour is the hull seen
through the raised water (the see-through, another lane's). The white water a slap leaves against her side, at rest
or under way, comes from the sides' memory ([sea-sky.md](../sea-sky.md), "The slap's white water").

The analytic Kelvin interior attenuated its shaded slopes but displaced the full wave. Height now takes that same
attenuation, retaining the cusp's existing height. Around length Froude 0.45 (eased over 0.43–0.46), `stem_crest`
bounds the crest line to half the draft and the Kelvin source to 0.15 of the draft; its shaded amplitude takes the
same reduction. The slower hulls keep their amplitudes, including shallow cruisers. These are visual bounds on the
wedge relation and analytic source; resistance, ship motion and the pressure field are unchanged. Ocean and bow spray
use the preset hull length for the Froude bound; the shorter sliced profile places the spray. The wet band reads the
drawn wake heights.

GPU readback, master `a0ab9c96d` → fix, using the roster models' profiles and full handling speeds, 20 s of steady
motion, High's 512² pressure field over 1,536 m, and a resolved 0.1 m water mesh. Heights are metres above a flat base
sea, without wind waves. Exterior extrema sample up to two beams to starboard, outside the side within one hull
length of the stem;
the amidships hollow samples 0.25–0.75 lengths aft, 0.5–3.5 m outside the side. The stem column is the crest line
with its surrounding heap, before overlap from the Kelvin waves.

| Hull, speed | Stem crest | Exterior max / min | Amidships hollow | Pressure minimum |
| --- | --- | --- | --- | --- |
| Fletcher, 35 kn | 3.83 → 2.34 | +4.70 / −1.89 → +2.51 / −0.33 | −0.68 → −0.05 | −0.063, unchanged |
| Yukikaze, 35 kn | 3.91 → 2.31 | +4.79 / −1.77 → +2.48 / −0.27 | −0.71 → −0.04 | −0.064, unchanged |
| Bismarck, 30 kn | 3.75, unchanged | +3.72 / −0.47, unchanged | −0.27 → −0.13 | −0.195, unchanged |
| Cleveland, 32.5 kn | 4.70, unchanged | +4.89 / −1.49, unchanged | −0.54 → −0.35 | −0.096, unchanged |

The battleship and cruiser share the interior-height fault, but their stem crests, outer Kelvin extrema and shaded
amplitudes are unchanged. Photographic shape references: the U.S. Navy's
[Fletcher off New York, 1942](https://commons.wikimedia.org/wiki/File:USS_Fletcher_(DD-445)_off_New_York,_1942.jpg) and
[Nicholas on trials, 1942](https://commons.wikimedia.org/wiki/File:USS_Nicholas_(DD-449)_underway_during_trials_on_28_May_1942_(19-N-36881).jpg).
Their narrow side wash and localized bow crest guide the look; they do not establish measured metre-scale heights.
Paired wind-4 captures and consecutive RPC frames, with eye adaptation off, are under
`.build/native/wake-review/{before,after}/` in the `fix-wake` worktree; numerical readbacks are beside them.

## The stern's waves and the boil

A Fletcher at 31 kn and a Bismarck at 28 kn (noon, a 6 m/s wind) showed two things real wakes do not. The churned water
was foam laid flat on the sea: `wake_height` took the wake field and the bow waves only, so from the chase camera or low
on her quarter the wake was a white ribbon painted on the water. And nothing marked the stern: no hollow behind it, no
hump where the water closes again, no quarter waves. The wake field's stern generator presses the sea 0.18 m for every
hull, in cells of 3 m on High, too little and too coarse to draw them. The references: USS Walke's stern wash standing
over her fantail at speed (NH 99830) and Shimakaze on her trials (NH 73053).

- **The stern's waves** (`bow_waves.rs`: `stern_crest`, `stern_peak`, `stern_breaking`, `stern`; `shaders/wake.wgsl`:
  `stern_shape`). Each bow slot has a seventh row: how far aft of the stem her stern stands, the stern's crest, how far
  astern of her stern it stands highest and her half-breadth just forward of her stern; the course row's last two
  floats carry how much it breaks and her half-breadth at her quarter (0.12 of her length forward of her stern). The
  breadths and the stern are her own slice's, a `SimpleHull`'s without one. The crest is `stern` (0.13) × U²/2g, grown
  in over length Froude 0.12–0.45 and never over her stem crest or half her draft: a Fletcher at 31 kn raises 1.7 m,
  Bismarck at 28 kn 0.8 m, Iowa at 32 kn 1.2 m, a destroyer at 10 kn nothing. As shares of it: her quarters sit in a
  trough (0.35 deep, about 0.06 of her length forward of her stern), the water behind her stern falls into a hollow
  (0.55, a little wider than her transom), the hump stands on her centreline 0.4 of her beam and 0.04 of the speed's
  wavelength astern (11 m for the Fletcher, 20 m for Bismarck), and her quarter waves leave her quarters at 23°, 0.6
  high with a steep outer face, gone a little over half her length astern. The vertex stage raises them, so the hull's
  wet band and the sea read follow; the fragment stage shades them by central differences.
- **Their white water.** The hump's top and each quarter wave's outer face break white, and what the breakers left lies
  inboard of them as lace, as much as `stern_breaking` allows: none below length Froude 0.22, all of it at 0.44, none on
  a crest under 0.3 m. A Fletcher at 31 kn breaks fully, Bismarck at 28 kn about a quarter: from astern the destroyer
  throws two white quarter waves in a wide V, the battleship's long, low hump barely breaks.
- **The boil** (`trail_relief`, and `trail_read` for the fragment stage). Young churned water heaves in lumps a third
  of her beam across and 0.035 of her beam tall (within 0.2 and 0.8 m), flattening as its turbulence falls from 0.9 to
  0.55: the first few hundred metres behind her at full speed. The vertex stage raises the lumps the water mesh holds;
  the fragment stage shades them all and roughens the boiling water (0.015 of slope variance), and the slick laid with
  it stills the waves only as the boiling stops. Bands only: a splash's patch keeps its own look. The trail atlas is
  now visible to the surface's vertex stage; the other passes that read `wake_height` (the hulls' heights, the sea
  read, rain, reflections, the underwater probe) do not bind it, so the boil is the drawn surface's alone.

The `Wake` uniform is 40,688 bytes (128 more).
