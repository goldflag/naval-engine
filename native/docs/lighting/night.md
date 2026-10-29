# Lighting: the night

## Night

### The moon and no fill after dark

After takram's sky and clouds came in (PR #829), ships glowed at night: a close Bismarck's side was 200–440 × the sea
beside it and as bright as at dusk, under a clear sky, broken cloud or a closed deck alike. Three things lit hulls (and
the land) that nothing else in the scene took, all tuned for the web's (and the old native sky's) brighter night, with
takram's night sky and sea about 4 × darker:

- **The moon at 1.5 ×** (`MESH_MOONLIGHT`): lit meshes now take the moon as it is, as the sea does
  (`ShipLighting::daylight`, `naval_conditions::mesh_light_shares`).
- **Cloud shadows stopped only the sun**: the clouds' ground shadow map was marched toward the sun and faded out as it
  set, so an overcast deck let the full moon onto every hull while the sea and sky stayed dark. It now marches toward
  the active body (`CloudUniforms::ground_light`: the moon once it outshines the sun), its strength from that body's
  elevation, so clouds shade moonlight on hulls, land and the sea's glitter.
- **A constant hemisphere fill after dark** (1.6 × the map's ambient, `MESH_NIGHT_FILL`), 5–25 × the night sky by
  itself: after dark there is none now (`ShipLighting::fill`, `mesh_light_shares`), a shaded side has only the
  environment's night sky, and a lightning flash still lifts the fill as the web's did (1.6 × the ambient × the flash).
  The web's fill ramp holds from an intensity of 1 up (dawn and dusk unchanged); below it the fill goes out with the
  sun's light, so a moonless night has none either and the switch to the moon does not jump. (Superseded on 2026-10-03:
  no fill by day either, see [realism](daylight.md#realism).)

Measured on a close Bismarck at the night preset (sun 28° down, full moon), 1600 × 900, the ship's linear luminance
against the sky beside it (`.build/native/night-ships/`, `shots.sh` and `sheet.py`; eye adaptation on and off read the
same, so it plays no part), before → the moon and its cloud shadow → no fill after dark: clear 0.076 → 0.056 → 0.025
(sky 0.006), broken cloud with the ship under a cloud 0.075 → 0.022 → 0.0015 (sky 0.001), overcast 0.070 → 0.018 →
0.0011 (sky 0.0012). Dusk and noon frames are identical before and after. Under a deck at night a hull is now a dark
shape against a dark sky, as at sea. Since 2026-10-06 the display grade below opens the world's shadows for gameplay.

### Night readability

Bill found the night world too dark to play while the sky could be darker. The display pass
(`naval-render/shaders/display_pass.wgsl`, `scene_radiance`) now lifts the world's shadows by at most 1.25 stops and
dims the sky's shadows by a quarter stop. Reversed scene depth distinguishes the sea, hulls, aircraft and land
(positive) from the sky and clouds (zero), so the same grade reaches every scene, including port and theatre. The
world takes the full lift within 1.5 km, fading smoothly to the sky's grade by 12 km: distant haze and the sea's horizon stay joined
to the sky, and the lift does not make distant shapes stand out through haze.

The grade fades in as the clear sky's mean luminance falls from −3 to −5.5 in log2, the same range in which the eye
stops opening. It uses the sky mean's clear-sky luminance in w, falling back to rgb when a host supplies no clear sky:
cloud cover and the camera's direction do not change the dusk fade. Since 2026-10-20 a host that gives the sun's
elevation (naval-client does) fades it in by the sun instead, from 10° down to −4° ([storm light](daylight.md#storm-light)):
a storm's weather scale had put its dusk sky under a clear night's. Without a sky mean it is disabled, even if a previous
scene left one in the buffer. The shadow adjustment fades out between scene luminances of 0.02 and 0.2, retaining
firelight, muzzle flashes and celestial highlights. It multiplies existing radiance: black stays black, and the
world retains its shading and haze. The physical light, cloud shadows, bloom and metering keep their existing values.

**Under water (2026-10-12).** A pixel seen through water takes the near world's grade whatever its depth. The open
deep has no depth, so it was graded as sky while the water above eye level, which ends on the sea's underside, was
graded as world: at night a camera under the sea showed a hard line at eye level, the deep about 1.5 stops darker
than the water over it. The display now reads the ocean's underwater probe (binding 7, `Renderer::set_underwater`,
which `naval-client` sets each frame while the ocean draws): where a pixel's ray starts under the waves the grade is
the near world's. It is per pixel, so with the waterline across the lens the sky above it keeps the sky's grade.
Above water nothing changes (pixel-identical). The sky through Snell's window is still graded as world, the sea's
surface being what the depth holds there.

**Under water at night (2026-10-20).** Bill found night under water "weirdly bright": a uniformly lifted teal. The grade
was not the cause. The water's own light under water (its pigment, which the underwater view, the motes and the sea's
underside take as emitted radiance; the water seen from above is the physical water body and never reads it) held a
quarter of the day's all night under any sky (`water_light_scale`, `0.24 + 0.76 × daylight`): a fill. It stood for the
light the water scatters back, so it now falls as that light does (`naval_conditions::water_light_scale`): by day
and through civil twilight as before (a quarter of the day's at its end, −6°), then with the twilight's real light
(`naval_render::lights::twilight_lux`) once that is under a full moon's (0.27 lux, about 8.5° down), which the game's
night lifts to the light it gives the end of civil twilight (`candela_scale`). At the night preset's −28° it is a
four-hundredth of the quarter. The night under water is black but for the moonlit sky through Snell's window (graded
as world still, as at dusk) and what lamps, star shells and fires light; dusk and noon are pixel-identical
(`.build/native/look-night-eye/under-*-view-{before,after}.png`, #1425's repro). Taking the sky's grade through water
as well was tried and dropped: at dusk, where the grade is half on, the deep went from (10, 26, 30) to (3, 11, 14).
The moon's own light on the water under water is not in it (the scale follows the sun's hour; the moon's phase and
height are the sky's, per frame): a follow-up.

**Smoke (2026-10-12).** Smoke, spray and fire write no depth, so the grade took them for whatever lay behind: a
burning ship's column was lifted where it crossed the sea or a hull and dimmed where it crossed the sky, a hard line
through it at the horizon and her upperworks a pale flat shape inside it. The scene now carries the transparents' cover: while
the grade is on, the renderer zeroes the scene's alpha as the `Transparent` phase starts (`transparent_cover.wgsl`, a
full-screen draw inside the scene pass; nothing drawn before reads that alpha), the transparents' own blending leaves
`a + A (1 − a)` there, and the display pass grades what they cover of a pixel as near world, whatever the depth behind
it. No copy and no extra pass. Checked on `effects-trial.ts --only underway-far --hours 21.5 --stills`.

**Under the sky (2026-10-25).** Bill found the night sea brighter than the sky. Ungraded it is darker, as it should
be (a clear moonless night through a 40° lens from 31 m: the sea at 1–2 km is half the sky's mean light, the sky by
the horizon about all of it); the lift took the sea to 1.4 times the sky as graded, and on screen the sea under the
horizon was (11, 25, 46) under a sky of (4, 11, 24). The lift now stops short of the sky that is there (`under_sky`:
the sky mean's luminance, clouds and all): whole 2.25 stops under it, none from the quarter stop under it the sky is
closed to, so nothing it lifts passes the graded sky. Spread over 2 stops and taken with the highlights' fade by the
lesser of the two, it never turns a brighter pixel darker than a dimmer one (checked numerically over every sky and
pixel light, and in `night_readability_lifts_the_world_short_of_the_skys_light`). A sea half the sky's light comes out
at two thirds of it and the darkest near sea is lifted as before; hulls, decks and land far under the sky keep the
whole lift. On screen the sea under the horizon is (1, 5, 17) and the moon's path on the water 23–39 where it was
58–80 (it is brighter than the sky's mean, so it takes no lift). It also ends the far funnel smoke's glow: what smoke
covers is graded as near world at any range, so a far ship's plume, its haze some 0.6 of the sky's light, took the whole
lift and came out a soft blue halo over her black hull, brighter than the sky (a Fletcher at 6 km through a 4° lens, no
moon); under the ceiling it takes little of it and reads faintly dark against the sky (`.build/night-smoke/v-rv-all.png`). (Grading the merged smoke's
cover by its distance was tried first and dropped: the scene's alpha is also the sun glare's cover, `sun_glare.wgsl`.) The night
tests' sky mean is now three stops over the world they draw (`NIGHT_SKY`). Stills:
`.build/night-smoke/v-look.png` (the old grade left).

**Overcast nights (2026-10-13).** An overcast night's sea toward the moon was 1.8 times a clear night's on screen
(22 against 12 in 8-bit), its environment 1.6 times what a converged bake gives. The cause was the clouds'
environment bake, which lit a deck from the middle of its long steps (`native/docs/clouds/march.md`, "The environment": a deck is lit
at its base). The grade here did not change.

`naval-render/tests/post.rs` checks the actual GPU display against paired sky/world surfaces: opposite adjustments
at night, a smooth dusk transition, an overcast day unchanged, the same grade under night clouds, preserved black
and highlights, no adjustment with a missing sky mean, and distant surfaces joining the sky's grade.

Reviewed against HEAD's packed reference in the native client at 1280 × 720: Bismarck, seed 1941, wind 8 m/s, one
second of battle, broadside and chase views, clear, partly cloudy and overcast nights, plus clear dusk and noon
controls. Captures and the comparison are in `.build/native/night-readability/`. The noon views differ by 0.06–0.10
of an 8-bit level on average between the packed release reference and the checkout's development client.

### Night vision

Bill found the nights too blue and gave a photograph of a moonlit sea to match: a charcoal sky, clouds lit grey and
white, a black sea with silver glitter and no saturated colour anywhere (its sky (46, 52, 60) in 8-bit, its near sea
(21, 24, 25)). The game's night was a saturated navy: from Takao's chase view at Iron Bottom Sound, 01:36, the sky
(0, 24, 59) and the sea (0, 4, 21). The blue is physical or long tuned (the tables scatter the moon's light as they do the
sun's, `sky_grade::SATURATION` over it; the moon's cool tint, `moon_tint`; the night floor's blue). What the eye does with
it was missing: by the moon and the stars the rods see, and they see no colour.

The display pass (`display_pass.wgsl`, `night_vision`) now shows what the night leaves dim as a faint cool grey
(`NIGHT_GREY`, (0.82, 1.03, 1.2): the photograph's sky and sea, its luminance one), keeping each pixel's luminance, while
light bright enough for the cones keeps its colour: the grey gives way from 2^−5.5 of the scene's luminance to all colour
at 2 (`CONES`), so fires, flashes, lamps and a star shell's light close by stay coloured. It comes in by the host's sun as
the twilight's light falls under a full moon's (`post::night_vision_weight`, `daylight::RODS`): none at 10 lux (the sun
some 5° down), 0.87 at a full moon's 0.27 lux (8.4° down), all of it from a tenth of a lux (9.4° down), so the blue hour
keeps its colour; a host that gives no sun gets none. It is applied after the night's lift and photo mode's lens and
diffusion, before the look's grade, so every view path and photo look takes it; the eye's metering and the bloom read
the scene ahead of the display and do not change. The same chase view now shows the sky (15, 21, 24) and the sea
(2, 3, 4); the moonlit clouds and the moon's path on the sea come out silver (`.build/night-vision/compare-*.jpg`, before,
after and the photograph). The photograph is brighter than the game's night (its sky some two and a half times on screen):
the night's levels, tuned for play, are unchanged.

`naval-render/tests/post.rs` checks it on the GPU: by moonlight a dim blue turns a faint cool grey while a fire's orange
keeps its colour to a level, and the blue hour keeps its blue; and the weight's fade against the real twilight.

## The eye at night

### The night eye

Bill: "the lights at night definitely should be brighter". Every light now burns at its real brightness under the same
lift as the rest of the scene, and the eye closes down when lights fill the view, as it does by day (lane night-eye;
pictures and commands in `.build/native/look-night-eye/`, `shots.sh`).

**One lift for every light.** After dark the game lifts the night: its full moon lit the sea at 0.52 of the scene's
units where the real one gives 0.27 lux (`candela_scale`, 1.93 units a lux; by day 1/16,700). That lift over the sun's
scale is the night's lift on real light (`naval_render::lights::night_lift`: 1 by day, 32,200, some 15 stops, after
dark, and between them through twilight as `candela_scale` goes). Since 2026-10-21 the full moon is the real 0.27 and
the lift 16,700, 14 stops ("The night's darkness"), and the fires and the flashes' light on the smoke take less of it
([fires and shots at night](lights.md#fires-and-shots-at-night)). Every light takes it whole:

- The star shells and searchlights, their real candela through `candela_scale` (`LAMP_NIGHT_SHARE`, 0.28, is gone).
- The flashes and fires, given in the sun's units, times the lift (`EffectLighting::lift`, which the client sets each
  frame): their light on the scene, the sea and the smoke (`CombatEffects::point_lights`, the gas's four flashes), and
  everything of them that glows, so a flame stands beside what it lights as it does by day: the gas's heat and the
  propellant's flame, the fires' flames and their light on their own smoke, the embers, sparks and flash sprites, the
  hot fragments, the tracers (`emitted` in `common.wgsl`; `sprite`, `gas_sprite`, `fire`, `debris`, `shells`,
  `volume`), and the aircraft's tracers and gun flashes (`naval-aircraft`'s `sprites.wgsl`,
  `AircraftLayer::night_lift`). The fires' own night light (`FIRE_NIGHT_LIGHT`) and their glow boosts after dark (the smoke's underglow
  ×4, the flipbook flame's light on its soot ×6, a longer cooling) are gone, and so are two sprites that stood in for
  light the renderer now gives: a soft halo about each fire's flames (the glare, which the bloom gives) and a pool of
  glow laid on the sea under it (its light on the water, which the sea's point lights give). Lifted with the night they
  drew a burning ship as a lit orange fog.

**One eye.** By day the eye meters a view against the clear sky's mean: it opens for a view darker than a camera's
middle grey, by at most 2 stops, and closes for glare, a view whose brightest tenth stands over `highlight` (1.5
stops over the clear sky's mean), by at most a stop. It did not meter at night: the opening and the glare both faded
out by the sun from 2° up to 4° down (`eye_daylight`), the closing against its own sky reached half a stop (a cell
counts at most `ceiling` over the sky), and the brightest tenth's histogram stopped at 8 stops over the clear sky. Now
the glare closes it at night as by day, with two things that follow from the night's lift (`eye_adaptation.wgsl`,
`post::reference_adapt_in`):

- **The ceiling is a clear noon's highlights on the screen.** By day `highlight` over the hour's clear sky; after dark
  over a clear noon's (`NOON_CLEAR_SKY`, 0.263), mixed by `eye_daylight` through twilight. The lift brought the moonlit
  scene to the screen, and only lights bring a tenth of a view to a noon's highlights: a quiet night's brightest tenth
  stands 6 stops under it (clear, full moon, the broadside: −1.5 over its clear sky, the ceiling +4.4), and the eye
  keeps the fixed exposure (the quiet views are pixel-identical before and after).
- **After dark the brightest fiftieth stands at most at the display's white** (`PEAK_HEADROOM`, 2 stops over the
  ceiling: AgX's top is 4 stops over middle grey, a noon's highlight 2 under it). A fire's flames are drawn some ten
  times brighter than the light their fire gives over their area (their emission was tuned against the display by
  day, `FIRE_LIGHT_PER_KW` is the physics': 60,000 cd for a 30 MW fire, whose flames of 30–40 m² would then shine at
  1,500–2,000 cd/m², where the game draws their cores near 50,000), so with the eye closed only until the fire-lit smoke
  (the brightest tenth) stood at the ceiling they stood 8–9 stops over it and the bloom laid an orange fog over the
  whole view. Kept within white, the burning ship is her flames white-hot, her smoke lit orange under them and her
  hull dark. By day it plays no part.
- **It may close by the lift on top of the day's stop** (`max_down + log2(lift) × (1 − eye_daylight)`, 16 stops at
  night): back to the day's sensitivity, never further. The histogram reaches 24 stops over the clear sky (34 since
  2026-10-21, the moonless sky being darker).
- **It still never opens at night** (`max_stops × eye_daylight`): true night stays as dark as its sky. Since
  2026-10-21 the dark-adapted eye opens for a night darker than a full moon's, by Weber's law with the rods' dark light
  ("The dark-adapted eye").
- **Its time.** Closing, the luminance it is adapted to moves toward the view's at the cones' 0.1 s; opening, its gain
  moves at 1.5 s (Durand and Dorsey 2000, "Interactive tone mapping": light adaptation 0.1 s for cones and 0.4 s for
  rods, after Adelson 1982 and Hayhoe, Benimoff and Hood 1987; the recovery from a fall in light that bleaches little
  "can take seconds", from one that bleaches much tens of minutes). A filter on the growing quantity takes a large
  step fast at first: from the fixed exposure to 12 stops closed, 9.3 in the first frame; back, 6.5 stops in a frame,
  under 2 left after half a second, a fifth of a stop after 3 s. It was the closing 0.4 s and the opening 1.5 s in
  stops; a day's change of a stop or two moves much as it did.

Through a 38 cm turret's salvo 80 m off at night (`eyetrace.ts`: the effects trial's `muzzle` scene, eye readings a
frame): the brightest tenth jumps from 0.5 under the clear sky to 15.75 over it in the flash's first frame, the eye
closes 8.6 stops that frame, 12.3 within a tenth of a second and 13.2 within a fifth, holds there while the fireball
glows, and has 6 left two-thirds of a second after the flash, when the glow is out. The frame shows the flash-lit
ship as a noon would, the fireball orange-white, the night black about it; with the eye off (Graphics → Eye
adaptation, or the effects trial's default) the first three-quarters of a second are white, as the lifted lights were
(`effects-trial.ts --eye` keeps it on).

**The half floats.** The scene's targets hold 65,504. A deck beside a heavy turret's flash takes more than that after
dark (89 million cd × 32,200 at 20 m), which a half float keeps as infinity, and a smoke drawn over it at full cover as
not a number. The transparents' cover pass, drawn at the start of the transparent phase each frame, now blends by
`min` and holds the opaque scene's light under 32,768 (`transparent_cover.wgsl`); a glow is drawn at most 16,384
(`EMISSION_CEILING`), keeping its colour's balance; the bloom and the display read the scene under 65,504. The same
triangle with the alpha left alone holds the scene before each copy other layers read (`DisplayPass::draw_hold`: the
opaque scene the sea looks through, the backdrop the blast waves bend).

What it does (`.build/native/look-night-eye/`, `<set>-<view>-{before,after}.png`, master a11ac14304's dev-client
against this branch's; the strips cut from the effects trial's clips with the eye on, as the game plays):

| View | Before → after |
| --- | --- |
| A quiet moonlit night (clear, the night action with every gun held: chase, broadside, deck) | Pixel-identical (4 pixels of 1.44 million differ by more than 1 % in the chase) |
| Clear noon (chase, broadside) | Pixel-identical |
| The night action under star shells, overcast and clear (chase, broadside at 1.5 km, deck, from 700 m) | The frames' means 63 → 96, 100 → 113, 39 → 74, 91 → 110 (overcast); the lamps' light 3.6 times what it was, their glow in the air the same; the eye closes 0.1–0.7 stop where a star shell's glow fills the view |
| A 38 cm salvo 80 m off (eight moments over 7.5 s) | The first frames show the flash-lit ship as a noon would and the fireball orange-white, the night black about them, where they were the dim ship by an unlit flash; the smoke lit by the next turret's flash comes out bright; the night returns in about a second |
| A battleship burning 130 m off (eight moments over 10 s) | Her flames white-hot, her smoke lit orange under them, her hull dark, a red glare about the fires; the sky and the sea black (the eye 7.6 stops closed) |
| Under water: night, dusk, noon (#1425's repro) | Night: black but for the moonlit window overhead (the deep (6, 20, 25) → (0, 0, 0)); dusk and noon pixel-identical |

**What it costs.** Nothing that shows: the eye's histogram is 160 bins where it was 96, the cover pass now blends by
`min`, and the lamps' brighter light lets one or two more lights take a shadow map in a busy night. Apple M5 Pro,
Metal, `dev-client`, High at 1920 × 1080, the staged night action (two star shells, two searchlights, every ship
firing) and the same battle at noon, 50 s in, 180 frames each from the chase camera and from 1.5 km abeam
(`.build/native/look-night-eye/cost.ts`), master's client then this branch's, load average 19–23, 2026-10-20 23:59:
GPU busy p50 at night 13.05 → 13.13 ms (chase) and 9.27 → 9.28 (abeam); at noon 10.37 → 10.26 and 7.37 → 7.48, within
the runs' spread. Of the passes at night the light maps 0.03 → 0.14 ms (chase) and 0.07 → 0.10 (abeam), the eye 0.03 →
0.04, the water and sky 4.84 → 4.95 and 4.19 → 4.23; by day every pass within 0.05 ms.

### The night's darkness

Bill: "can you also overall make the scene at night darker. i feel like its still too bright even when there is barely
any moon". Pictures and commands in `.build/native/look-night-fire/` (`shots.sh`, `nights.ts`, `twilight.ts`).

**Why a thin moon looked a full one.** Measured first on master (a Bismarck on the open North Atlantic at 23.5 h, the
moon held 35° up by the developer weather, or 30° under the horizon; the frame's mean of 255 from the chase camera):
a clear night's frame was 20.4 under a full moon, 19.9 half, 19.2 thin and 19.1 with none, and the clear sky's mean the
same at every phase (log2 −4.9 to −4.5: brighter with no moon). The eye did not open at night, and the display's night
grade is a fixed lift, so neither evened the phases out. The sky did: the dome's night floor (`sky_grade::FLOOR`, held
whole every night) and the night sky's glow (`NIGHT_GLOW`, "some ten times the real figure") gave a level surface about
0.17 of the scene's units, a third of the game's full moon and some seventy times a real moonless night's light, and
they lit the sky, the sea and the ships' shaded sides alike. The moon itself fell with its lit share only (a half moon
half a full one, where the real one gives a tenth).

**What changed.** Each in the scene's units, which after dark are now a lux. Dusk keeps what it was graded with: each
night's level comes in as the real twilight falls under a full moon's light (`naval_sky::model::night_share`, 0 to the
sun 8.4° under the horizon, then toward 1 as the twilight fades), so from the dusk preset's 3° to −8° every frame is
the one master drew (the table below).

- **The full moon at its real light**: 0.27 lux on the sea (Bond and Henderson, "The Conquest of Darkness", 1963;
  Krisciunas and Schaefer 1991), not the game's 0.52 (`naval_sky::state::MOONLIGHT` 0.8 → 0.4173 after dark,
  `model::moonlight_at`; dusk keeps 0.8, `MOONLIGHT_DUSK`, the moon the game lifts over the twilight's light). The
  night's lift on real light is now the day's lux a unit, 16,700 (14 stops; it was 32,200, 15): a lamp's real candela is
  its candela in the scene, and the lights keep their real brightness beside the moon.
- **A part moon by its phase law**: `state::moon_brightness`, Krisciunas and Schaefer's fit to the moon's magnitude by its
  phase angle α, 0.026 |α| + 4 × 10⁻⁹ α⁴ ("A model of the brightness of moonlight", PASP 103, 1991): a quarter moon
  gives 9 % of a full moon's light (0.025 lux; about 0.03 in the usual tables), a crescent two days old 0.2 %. The
  disc's own look keeps its shading by the sun's angle and the phase's darkening (`moon_lighting` takes a full moon's
  light), and the stars and Milky Way wash out by the moon's light, not its lit share.
- **The moonlit night sky is the tables' own**: after dark the dome scatters the whole of the moon's light
  (`sky_grade::MOON`, `model::moon_dome_share`), as it does the sun's; through dusk the quarter it was graded with.
- **The night floor falls after nautical twilight** (`model::night_floor_share`, `SkyUniforms::night_glow`, every place
  the floor stood: the dome, the clouds' march, the environment and the sea's light): all of it through dusk, then down
  with the real twilight's light to the night sky's own. `NIGHT_GLOW` is gone, and the Milky Way's brightest is four
  times the dark sky's zenith (0.04 → 0.006; Leinert et al., "The 1997 reference of diffuse night sky brightness", A&AS
  127, 1998).
- **The night sky's own light is 0.005 lux** (`model::NIGHT_SKY_LUX`), **five times the real one.** A clear moonless
  night gives 0.001–0.002 lux (Schlyter, "Radiometry and photometry in astronomy": starlight 0.001, an overcast night
  0.0001; 0.002 with airglow in the usual tables). At the real 0.001 the moonless view is black but for the stars (the
  chase frame's mean 0.2 of 255, the ship unseen); at 0.005 the ship shows as a dark shape against the horizon's glow and
  the sea is near black (2.1). The game needs that much to be played on a moonless night: a theatre's moon is its
  date's, and some battles (Savo) were fought with none.
- **The dark-adapted eye opened up to two stops beyond a full moon's** (replaced the same day by Weber's law with
  the rods' dark light, "The dark-adapted eye"; `post::NIGHT_OPEN`, `night_open`,
  `Renderer::set_night_open`): by the night's light at the sea (the sun's, the moon's beam and the sky's, `Sky::sea_level`,
  in real lux by the scene's scale) against a real full moon's, as the night comes in; 0 under a full moon and through
  dusk, two stops once the night's light is a quarter of a full moon's (a moon some four-fifths lit) and below, and never
  past the ceiling the lights keep to. A real eye gains far more over half an hour in the dark (Hecht, Haig and Chase,
  1937); kept to two, a moonless night stays about four stops under a full moon on the screen. It reads the clear air's
  light, as the display's night grade reads the clear sky: an overcast night is as much darker than a clear one as its
  clouds make it, and the moon's beam counts as on a surface facing it, so a low full moon does not open it.
- **An overcast moonless night was black** (the real 0.0001 lux), a ship showing there by lights alone, until the
  dark-adapted eye opened further ("The dark-adapted eye").
- **The lamps** keep their real candela: under a full moon a lamp-lit view is a stop darker on the screen with the
  moon (the night action under star shells: the chase frame's mean 80 → 46); on a darker night the eye opens and they
  stand out more.

What it does (`nights.ts`: the frame's mean of 255 and the eye's stops open; master 245a913267 against this branch;
`moons-<weather>-<view>-{now,after}.png`, the four moons in a quad):

| Moon (its light against a full moon's, at the sea) | Clear, chase | Clear, from 690 m | Overcast, chase | Overcast, from 690 m |
| --- | --- | --- | --- | --- |
| Full (1.0, 0.27 lux) | 20.4 → 9.4 | 1.8 → 0.5 | 13.0 → 10.1 | 1.4 → 1.0 |
| Quarter (0.091, 0.025 lux) | 19.9 → 6.1 (eye +2.0) | 1.5 → 0.2 (eye +2.0) | 4.7 → 2.1 (eye +2.0) | 0.6 → 0.2 (eye +2.0) |
| Thin crescent, two days (0.0022, 0.0006 lux) | 19.2 → 2.2 (eye +2.0) | 1.3 → 0.0 (eye +2.0) | 0.5 → 0.1 (eye +2.0) | 0.0 → 0.0 (eye +2.0) |
| None (under the horizon) | 19.1 → 2.1 (eye +2.0) | 1.3 → 0.0 (eye +2.0) | 1.2 → 0.4 (eye +2.0) | 0.3 → 0.1 (eye +2.0) |

Dusk into night, a full moon riding opposite the sun (`twilight.ts`, the chase camera, the clear weather;
`twilight-{now,after}.png`), the frame's mean of 255:

| Sun | 3° | 0° | −2° | −4° | −6° | −8° | −9° | −10° | −12° | −15° | −20° |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Master | 56.3 | 62.1 | 51.8 | 23.8 | 13.0 | 10.3 | 9.6 | 10.3 | 13.2 | 17.9 | 18.9 |
| This branch | 56.3 | 62.1 | 51.8 | 23.8 | 13.0 | 10.3 | 8.6 | 6.9 | 7.3 | 8.4 | 9.5 |

The night darkens from nautical twilight on and keeps darkening, where master's lightened again as the night's glow came
in; past −10° the frame lightens a little as the moon, opposite the sun, climbs. The dusk preset's views (chase and
broadside, the night action's ships with guns held) and noon's are pixel-identical to master's; under the sea at night
the view stays black (its mean 0.25 → 0.18 of 255, the moonlit window overhead a stop darker); the quiet full-moon
night's chase frame is 33 → 16 about the ships (`quiet-*-{now,after}.png`).

### The dark-adapted eye

Bill, having played the night's darkness: "ok can you make the night have a bit of brightness everywhere since right now
i basically cant see anything". Pictures and commands in `.build/native/look-night-floor/` (`shots.sh master|after`).

The two stops the night eye opened at most (`NIGHT_OPEN`, "The night's darkness") were ours, not the eye's: the rods
adapt over three to four log units, ten to thirteen stops (Hecht, Haig and Chase 1937). The night's light and the moon
stay as they were (`NIGHT_SKY_LUX` 0.005, the real full moon, the phase law); the eye now adapts further.

- **Weber's law with the rods' dark light** (`naval_render::post::night_open`, `eye_adaptation.wgsl`): after dark the
  eye's gain goes as 1 / (E + D), E the night's light on the sea (lux: the sun's, the moon's beam and the sky's,
  `Sky::sea_level` in real lux by the scene's scale, `Renderer::set_night_light`) and D the eye's own dark light, the
  noise floor under which more dark gains it nothing (Barlow, "Increment thresholds at low intensities considered as
  signal/noise discriminations", J. Physiol. 136, 1957: the threshold goes as the light plus a dark light). So it opens
  log2((F + D) / (E + D)) stops over the fixed exposure, F the light from which it starts: nothing at F or more, nearly
  a stop for each stop the light falls while it stands well over D, less and less toward D. F is 0.394 lux
  (`NIGHT_ADAPT_FROM`, a little over a full moon's light on the sea) and D 0.041 (`NIGHT_DARK_LIGHT`): **0.4 stop under
  a full moon, 2.6 under a quarter moon, 3.2 under a thin crescent or none, 3.4 at most in the dark.** The form is the
  physical one; F and D were fitted to the brightness wanted on the screen (the clear chase view's mean of 255 about 14,
  12, 10 and 10 for a full, quarter, thin and no moon), not measured. The most it opens is 8 stops (`NIGHT_OPEN`); the
  view floor below takes a dark view from 690 m to 7.
- **Under cloud it opens a third of a stop further for each stop the clouds take of the sky's light**
  (`NIGHT_CLOUDS`, the view's sky mean against the clear sky's, on the GPU), an empirical share: a deck that takes two
  stops opens it 0.7 stop more, the overcast moonless chase view 6 of 255 by that alone. Taking the clouds' dimming as a
  fall in the light instead, the dark light holds the opening to 3.4 stops and the overcast night stays near black.
- **A dark view opens at least to the night's view floor** (`NIGHT_VIEW`, `EyeAdaptationSettings::night_view`): **a
  floor**, not a law of the eye. The eye opens at least until the view's centre-weighted mean (the metered cells, log2
  of the scene's luminance) stands at −7.3, as an eye looking into a dark view adapts to it. The open-sea views above
  never need it under a clear sky; Cape St George's start (25 November 1943, 01:15, its moon down, a stratocumulus deck of
  95 % at 600 m with showers about) did: by the night's light and the deck alone its Helm view was 1.4 of 255, its sea
  and wake unseen; at the floor 8.4, the sea's swell, the wake and the ships' hulls reading as shapes and the sky just
  over the sea. Views from above the sea (690 m) took it too and came to about 14 at every moon, until 2026-10-25
  ("Looking down at night"): the floor now counts a view as no darker than 3 stops under its sky.
- **It opens past the fixed exposure at the rods' pace** (until 2026-10-25: "The night eye in a fight"; `NIGHT_BRIGHTEN`, 6 s, `eye_step_at`): dark adaptation
  takes minutes in life and some twenty seconds here; back to the fixed exposure after a glare it still opens at the
  cones' 1.5 s, to a tenth of a stop over the fixed exposure (`NIGHT_FAST_TO`, so its easing crosses it), and a
  capture's settling goes straight to the goal. Opened at once after a salvo's flash, the eye stood wide open when the
  next turret fired and its flash-lit smoke came back to the cream-white the night eye first drew; at this pace it is
  still closed from the first flash when the next comes (the trace through the salvo: −0.68 stops 2.5 s after it, where
  master's eye stood at −0.88).
- **A lit view is not opened further** (`EyeAdaptationSettings::night_lit`, `NIGHT_LIT`): the opening stops where the
  view's brightest tenth stands 3 stops under the ceiling the lights keep to. A view a star shell, a searchlight or a
  fire lights is seen by that light, not lifted for the dark about it, and the eye still closes on lights that fill the
  view (the glare rule, "The night eye"). Through dusk it does not open, as the night comes in (`night_share`).

What it does (`nights.ts`: the frame's mean of 255 and the eye's stops; master 38db7f5925 → this branch;
`moons-<weather>-<view>-{master,after}.png`, the four moons in a quad):

| Moon (its light on the sea) | Clear, chase | Clear, from 690 m | Overcast, chase | Overcast, from 690 m |
| --- | --- | --- | --- | --- |
| Full (0.27 lux) | 8.9 → 14.6 (eye +0.0 → +0.5) | 0.5 → 14.6 (eye +0.0 → +2.6) | 9.4 → 16.1 (eye +0.0 → +0.5) | 1.0 → 14.3 (eye +0.0 → +2.1) |
| Quarter (0.025 lux) | 5.8 → 12.5 (eye +2.0 → +2.6) | 0.2 → 14.3 (eye +2.0 → +5.0) | 2.0 → 6.9 (eye +2.0 → +2.9) | 0.2 → 14.4 (eye +2.0 → +5.1) |
| Two-day crescent (0.0006 lux) | 2.1 → 11.0 (eye +2.0 → +3.2) | 0.0 → 14.7 (eye +2.0 → +5.9) | 0.1 → 6.5 (eye +2.0 → +4.7) | 0.0 → 15.7 (eye +2.0 → +7.0) |
| None (0.005 lux, the night sky) | 2.0 → 10.9 (eye +2.0 → +3.2) | 0.0 → 14.7 (eye +2.0 → +5.9) | 0.4 → 10.3 (eye +2.0 → +4.4) | 0.1 → 13.9 (eye +2.0 → +5.4) |

The clear chase view keeps its order by the moon (14.6, 12.5, 11.0, 10.9). Under the overcast weather's deck a part
moon's night (6.9, 6.5) draws darker than a moonless one (10.3): with a moon behind the deck the horizon under the
cloud base goes dark, as it did on master (0.1 against 0.4). From 690 m every moon's view comes to the view floor,
about 14.

The rest (`tools/means.py`, `tools/frames.py`; master → this branch, the frame's mean of 255). The strips are recorded
without stills (`effects-trial.ts --clip-only`): a still settles the eye on its view, and settled at each moment the
night eye's slow opening never showed; both sides' strips are taken so.

| View | Master → this branch |
| --- | --- |
| Cape St George's start from the Helm view (25 November 1943, 01:15, its moon down, a 95 % deck at 600 m, showers) | 0.1 → 8.3: black → the sky over the sea, the swell, the wake and the ships' hulls as shapes (`scenario-cape-view-*.png`) |
| The quiet full-moon night (chase, broadside) | 9.4 → 15.4, 8.2 → 13.9 |
| The night action under star shells (chase, broadside, from 700 m) | 47.5, 105.9, 108.7: the same, the lamp-lit views not opened |
| A burning ship in a night fight (chase, from her beam at 600 m) | 14.8 → 23.0, 9.0 → 14.5: the dark about her lighter, her fires as they were |
| The burning ship 130 m off (the effects trial's strip, eight moments over 10 s) | The same at every moment (36.8 at 0.5 s, 38.1 at 9 s) |
| A 38 cm salvo 80 m off (the strip) | 0.7 s 78.4 → 81.6, 2.5 s 124.4 → 128.3, the rest the same |
| Under the sea at night | 0.3 → 0.3 |
| The dusk preset, noon | Pixel-identical |
| Dusk into night (`twilight.ts`) | 3° to −8° the same; −9° 8.2 → 10.3, −10° 6.6 → 10.3, −12° 7.0 → 12.4, −20° 9.0 → 14.5 |

### The night eye in a fight

Bill, in a moonless night fight: "if i dont have eye adaptation turned on its literally impossible to see anything",
and with it on: "when the ships shoot i cant see anything for a few sec after the flash which in know makes sense but
its not great for gameplay". Clips and commands in `.build/night-eye/` (`runs.sh`, `after2.sh`).

Both came from where the night's opening lived. The dark-adapted eye ("The dark-adapted eye") opens 3.2 stops over the
fixed exposure on a moonless night and up to 8 for a dark view, and that opening was the eye adaptation's alone.

- **The eye off kept the fixed exposure after dark**, which suits a full moon (its opening is 0.4 stop) and leaves a
  moonless night black. Now the eye off keeps the night's opening for the view, as the eye on settles at it
  (`EyeAdaptationSettings::dark_adapt`, which the client sets above water whatever the Graphics setting, and the harness
  always): the dark-adapted eye's Weber's law, the clouds' share and the view floor, held under the lights as the eye
  on is (`NIGHT_LIT`). It never closes under the fixed exposure for glare, so a flash dazzles white for its moment as it
  did, and it never meters the view by day: by day and under water it keeps the fixed exposure, as before.
- **After a glare at night the eye on came back only to the fixed exposure at the cones' pace**, then opened past it
  at the rods' 6 s (`NIGHT_BRIGHTEN`, now gone): on a moonless night 3.2 stops under where it stood, black for seconds
  after each salvo. Now after dark it opens at `NIGHT_RECOVER` (0.5 s, by its gain as by day) all the way back to the
  night's opening: 12 stops closed are within two thirds of a stop of it in half a second and a quarter of a stop in a
  second. In life a flash costs the rods minutes; this is for the game. The cost is the ripple the rods' pace was for:
  the next turret's flash finds the eye open, so the smoke it lights draws brighter than with the slow opening. Opening
  for a darker view (from a lit view to the dark sea, from the bridge to the air) is as quick.

What it does: a 38 cm salvo 80 m off at 1 a.m., no moon (`effects-trial.ts --only muzzle --hours 1 --moon none
--clip-only`, `--eye` for the eye on; the moon flag is new, as `shot.ts`'s), before (the 10-24 baseline client, the same
eye as master) → after. The frame's mean of 255 in the clip's video range, where 16 is black:

| | Before the shot | 2 s after | 5 s after | Frames black over the 8 s after the shot (of 480) |
| --- | --- | --- | --- | --- |
| Eye on | 21 → 24 | 16 → 21 | 16 → 22 | 352 → 58 |
| Eye off | 16 → 24 | 16 → 21 | 16 → 22 | 330 → 54 |

The frames still black are each later flash's next tenths of a second while the eye comes back, and the second the
smoke bank fills the view this near (every clip goes black then). By day nothing changes: the night's opening and
`NIGHT_RECOVER` come in with `Hour::night`.

**The white screen.** Then: "can you also make light at night a bit [less] bright so it doesnt instantly make the
screen all white". With the eye off a heavy salvo whited out the whole view for its first three or four tenths of a
second, from 80 m and from the chase view's 240 m astern alike; with the eye on its first frames were a grey-tan veil
over everything until the eye had closed. The bloom laid it: with the bloom off the same frame from astern is the ship
lit hard and the fireball white over a dark sea (mean 77 of 255 against 234). A flash's heart and what it lights stood
ten and more stops over the display's white, and the high pass took all of it into a blur that reaches across the view.
After dark, coming in as the night does:

- **A pixel brings the bloom at most 16** (`NIGHT_BLOOM_LIMIT`, the graded luminance; the threshold is 2;
  `post::bloom_limit`, `bloom.wgsl`'s high pass): what is brighter glows about itself, not over the view. By day no
  limit.
- **A flash's and a blast's own glow keep 64 times their day's** (`naval_effects::FLASH_GLOW_AT_NIGHT`, some eight
  stops under the lift: the flash sprites and sparks, the gas of a gun or a blast, a detonation's fireball; `flash_color.w`
  in the effects' lighting). The tracers and hot fragments keep the whole lift.
- **Their light takes a sixteenth of the lift on the scene and a thirty-second on the smoke** (`FLASHLIGHT_AT_NIGHT`,
  `FLASH_SMOKE_AT_NIGHT`, which was a half): four stops under where it was, the smoke a stop under the scene as before.

The salvo from astern (`effects-trial.ts --only muzzle-chase`, a scene the tool now names; master → each step → all
three; the frame's mean of 255 at the flash, 0.1 s and 0.3 s):

| | Master | Its light (a sixteenth) | And its glow (64×) | And the bloom's limit |
| --- | --- | --- | --- | --- |
| Eye off | 234, 235, 234 | 234, 235, 91 | 234, 230, 89 | 159, 149, 64 |
| Eye on | 179, 95, 33 | 187, 82, 21 | 159, 82, 28 | 98, 75, 25 |

Rebased over #1899 and #1900 (the fires' smoke in the wind, the cook-offs' pops) the last column reads 173, 160, 66 and
100, 75, 26. The bloom's limit is what lifts the veil; the glow and the light shorten it and keep the ship from blowing out with the
eye off. From 80 m, over the 8 s after the shot, frames white (230 of 235 or more) with the eye off 30 → 0, black with
it on 352 → 62. A far ship's flash still stands some eight stops over the sea about it.

### Looking down at night

Bill: "in night battles everything is much brighter when i look down at my ship from above". The night's view floor
(`NIGHT_VIEW`, "The dark-adapted eye") opened the eye until the view's mean stood at −7.3, and a view with no sky in it
(the sea from above, which reflects little of the sky) meters some three stops under one at the horizon: the eye opened
2.6 stops past the chase view's and drew the ship's sky-lit deck and the sea as at dusk.

- **The floor meters a view no darker than 3 stops under its sky** (`NIGHT_VIEW_UNDER`,
  `EyeAdaptationSettings::night_view_under`, `max(own, …)` in `eye_adaptation.wgsl` and its twin). Views at the
  horizon meter 1.6 (chase) to 1.8 (broadside) under a moonless sky and keep the floor as it was; Cape St George's Helm
  view meters about one under its deck and keeps its 5.3 stops. Two stops, a horizon view's, left the view from above
  near black (mean 1 of 255); three leaves it a stop over the chase view's eye.

`shot.ts`, Bismarck at night, the eye's stops / the frame's mean of 255, master → this change (commands and pictures in
`.build/night-floor/`):

| | Chase | Broadside | Above (55°, 260 m) | Top |
| --- | --- | --- | --- | --- |
| No moon | 3.74 / 6.3, the same | 3.96 / 6.6, the same | 6.35 / 15.3 → 5.18 / 4.0 | 6.59 / 13.0 → 5.14 / 1.9 |
| Full moon | 0.47 / 22.6, the same | 0.47 / 9.9, the same | 2.98 / 17.4 → 1.95 / 5.9 | 3.23 / 15.2 → 1.99 / 3.5 |

Views from far above the sea (690 m, "The dark-adapted eye") come down with them: the floor no longer brings them to
about 14 at every moon.
