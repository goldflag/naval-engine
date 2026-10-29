# Lighting: point lights and scorch

## Lights at night

Night fights (Savo, the Guadalcanal campaign) were the weakest look left: eight point lights for the whole scene, in
which the gun flashes, the fires, the star shells and the searchlights competed (the four nearest tactical lights first,
so a cruiser's four lamps trained elsewhere took every place and the star shells over the ship in view lit nothing on
the water), no light cast a shadow, and the star shells' and searchlights' light came from their gameplay radius
(`range² × 0.5`, `range² × 1.5`). Three changes (lane night-lights; pictures and commands in
`.build/native/look-night-lights/`, `shots.sh`):

**One list for every surface, culled for each view.** The host gathers every light the frame has (the effects' flashes
and fires, the star shells and searchlights, lightning's stroke: `naval-client` `scene/mod.rs`) and hands them to the
renderer (`Renderer::set_point_lights`), which keeps the 64 brightest at the camera (`PointLights::brightest`, their
power over the squared distance: a light past the budget drops by its light at the camera, not by order) in one block at
group 0, binding 10 (`naval::lights::scene_lights`). Every lit layer reads it: the hulls, aircraft, wildlife, the land,
the sea and the blast sheets' spray; their own uniforms no longer carry lights. A clustered index of the view culls it
(binding 11, `LightClusters`, built on the CPU each frame for the frame's camera, the second view's for its own): 16 × 9
tiles by 16 slices of depth (the first to 4 m, then geometric to 40 km), a bit for each light in each cell its reach
touches, the reach where its light falls under `LIGHT_CUT` (5 × 10⁻⁵ of the scene's irradiance, a ten-thousandth of the
game's full moon). A surface loops over its cell's bits (`light_cell`, `light_word`). By day a flash reaches a few
kilometres and a fire a couple of hundred metres; after dark the lamps reach the whole view, and the index culls what is
behind the eye or off the view. The effects keep 32 flashes burning at once (a ring, the newest replacing the oldest;
the gas still takes the four brightest) and offer every fire in view (until now two to the hulls within 1.5 km and three
to the sea). No flash or fire has a cut-off any more (three's window, 126 m for a 16-inch flash, was a culling
convenience); a star shell's and a searchlight's light end at the simulation's reach (its light radius, the lamp's
range), on the hulls, the land and the sea alike, as the index culls them. A crate's own group 0 (the ships' stills and tests) binds `StandaloneLights`.

**Shadows for the brightest few.** Searchlights and star shells cast through maps of their own (`light_shadow_maps`):
a searchlight's along its beam (a perspective map as wide as its cone and a tenth, from 3 m ahead of the lamp to 12 km),
a star shell's looking straight down under it (±60°: a footprint 1.7 times its height each side, its texels coarsest at
the edge, where the shadow fades out). They are layers of the sun's near-map array, at its size (1,024 texels on Low,
2,048 from Medium up), made when the first light needs one and kept until the settings change, so a night's star shells
come and go without remaking it. The lights that take them are those that give what the camera sees the most light (at
the eye or at its target, through a searchlight's cone: a lamp trained elsewhere takes none), and only above
`SHADOW_LEAST` (a hundredth of the game's full moon), so by day none is drawn. How many is the Shadows row's: Off none,
Low 1, Medium 4 (the High preset's), High 6, Ultra 8. The hulls cast into them (`SceneLayer::draw_light_shadow`; the
land, aircraft and effects do not), and the hulls, the land and the sea receive them (`light_shadow`: four bilinear
comparisons, the receiver a texel and a half off its surface). On the sea a mapped light no longer needs its march over
the screen's depth (`light_seen`), and a pixel marches toward at most four of the others (the brightest at the camera
first), where it marched toward every one of eight. The gun flashes cast none: an omni light needs a cube of six maps,
redrawn for a light that burns a fifth of a second, and the brightest flash is beside the camera's own ship; skipped.
After sunset the fires' light is shadowed on the hulls by a march on screen instead (`LightCaster::Screen`,
`ship.wgsl`'s `fire_seen`, through the contact shadows' hull depth; since 2026-10-25: [Flames and their
light](../combat-effects/fire-and-smoke.md#at-night)).

The beam pass (lane night-air) can show a ship cutting a searchlight's beam from these maps: the lamp's map is
`scene_lights.shadows[m − 1]` (world to the map's u, v and reversed depth) at layer `scene_lights.count.z + m − 1` of
`naval::shadow::shadow_cascades`, where `m` is the lamp's `shadow.x`; marching the beam's ray through it and dropping the
in-scattering where the map is nearer the lamp than the march gives the dark shaft behind a hull. A follow-up.

**Real values.** The star shells and searchlights are real candela (`naval-effects` `illumination.rs`, a star shell's
candle and reach the simulation's own, `naval_sim::battle_effects::star_shell_candela` and `star_shell_reach_m`, by
which its lookouts see too ([battle tools](../../../docs/battle-tools.md#illumination-shells)), which
`tactical.rs`'s `Searchlight::light` and `Flare::light` take: the one light each that lights the scene and the air
about it), scaled into the scene by the night (`naval_render::lights::candela_scale`): by day the sun's scale
(`DAY_LUX`, 16,700 lux a unit), after dark the lift the game gives the moon (its full moon at the sea is 0.27 of the
scene's luminance, `full_moon_scene`, as the real one gives 0.27 lux: a unit a lux; 0.52, 1.93 units a lux, until
2026-10-21, [the night's darkness](night.md#the-nights-darkness)), and through twilight the
game's moonlight over the real twilight's light (Bond and Henderson 1963; the US Naval Observatory: 3.4 lux at the end
of civil twilight, 0.008 at nautical), never under the day's. Since 2026-10-20 every light takes the whole of it
([the night eye](night.md#the-night-eye)): a 5-inch star shell burns at its real 825,000 cd in the scene after dark, a 36-inch lamp at
340 million (1.6 and 660 million at the lift of 2026-10-20); until then the lamps took 0.28 of it (`LAMP_NIGHT_SHARE`), because at the whole their light in the air
whitened the sky about every star shell at the fixed night exposure.

| Light | Value | Source | Colour |
| --- | --- | --- | --- |
| Star shell | 6,500 cd a millimetre of bore: 660,000 cd for a 4-inch, 825,000 for a 5-inch, 990,000 for a 6-inch (the gun's calibre now rides the packet: `IlluminationFlare::caliber_m`, `FlareRender::caliber_m`), faded with the simulation's reach in its last 12 s; the reach is a 5-inch's 3 km as the square root of the candle | US Army illuminating rounds: 81 mm M853A1 about 525,000 cd for 50–60 s, 155 mm M485A2 1,000,000 cd for 120 s (FAS, GlobalSecurity); the 5"/38's star shell burned about 50 s (NavWeaps); a US Navy parachute star shell of 1922, 800,000 candlepower | 4,000 K (`#ffd4a5`): the sodium nitrate and magnesium candle's sodium lines over its white continuum (Conkling and Mocella, "Chemistry of Pyrotechnics") |
| Searchlight | 340 million cd on the axis at full power, times the lamp's power | A 36-inch (91 cm) high-intensity arc, by its mirror's area between the German 60 cm lamp's 135 million cd and the 150 cm's 990 million (375–440 million a square metre; Wikipedia, "German searchlights of World War II") | 5,500 K (`#ffeee1`), the high-intensity carbon arc |
| Gun flash | 2,800 × size² × √barrels in the sun's units, 89 million cd for a 38 cm turret's three barrels, fading over 0.12–0.22 s; after dark times the night's lift (`night_lift`), as the flash's own gas (since 2026-10-25 64 times its day's glow and a sixteenth of the lift on the scene: [the night eye in a fight](night.md#the-night-eye-in-a-fight)) | No measured candela for a naval gun's flash; a fireball of burning propellant gas at 1,800–2,000 K (Klingenberg and Heimerl, "Gun Muzzle Blast and Flash") is 120,000–470,000 cd/m² as a black body (Planck), 30–100 m² for a 6-inch and 700–1,200 m² for a 16-inch salvo: 4–560 million cd. A 152 mm howitzer's flashes last 50–100 ms (Applied Spectroscopy, 2012) | 2,500 K (`#ffa64a`; was `#ffd29a`, some 3,900 K), the fireball's glow after it 2,000 K (`#ff8a3c`) |
| Fire | 60,000 cd for a 30 MW fire (`FIRE_LIGHT_PER_KW`) in the sun's units, after dark times the night's lift, as its flames (its own night light, `FIRE_NIGHT_LIGHT`, a seventieth of that, is gone) | A quarter of its heat radiated at 0.1 lm/W (the top of what 1,300–1,500 K flame gives: Planck has 0.013–0.085 lm/W) | 2,000 K (`#ff8a3c`) |

**The flashes and fires were not lifted until 2026-10-20.** Their real light lifted with the night as the moon is
whited the frame out at the fixed night exposure: from 80 m off a 38 cm turret the first third of a second after a
salvo was white from edge to edge, and a battleship burning on 227 MW from 130 m was lit white-orange over her whole
length, her flames lost in it (`.build/native/look-night-lights/effects/night-lifted/`), so they kept the sun's units
and the fires a night light of their own, until the eye adapted at night. It does now ([the night eye](night.md#the-night-eye)).

**The sea's lamp glints as glints.** Under the stronger, warmer star shells the lamps' glitter (summed and held under
the knee the moon's takes) drew a flat tan sheet over the sea toward them. It is drawn as the sun's is
(`glitter_glints`: the mean as the few wavelets that turn the light to the eye, counted by the brightest lamp's facet
density, under the ceiling on one glint's pixel): a star shell lays a path of sparkles like the moon's. Since
2026-10-20 each lamp's glitter is kneed and held at its own light (its irradiance on a surface facing it,
`SeaLights::peak`), as the sun's is at the sun's, where all of them took the moon's (or a full moon's, `LAMP_KNEE_FLOOR`,
with the moon down); and it is drawn as glints at every pixel size (`lamp_glints`). Within a few hundred metres of the
camera, where a pixel spans few facets, the mean stood, as the sun's does: under a star shell overhead it lay over the
water about the eye as a matte sheet the flare's colour, its slopes toward and away from the flare brighter and darker
like sand (night-lights' overcast chase and deck views, bottom right). A flame a few arcminutes across turns few of the
facets a pixel spans to the eye however near: the near water breaks into glints as the far does, and the sheet's hard
edge, where the glints took over from the mean, is gone. Since 2026-10-21 their ceiling is each light's irradiance
weighed by its share of the glitter (it was every light's in the pixel's cell, summed), and a lamp's glint carries one
pixel's worth of light whichever cell size draws it ("Lamp glints and salvo smoke" below).

Before and after (left and right), the release client of master after the photoreal batch against this branch, a night
action at 7 km (New Orleans, Pensacola and Fletcher against Takao, Myoko, Fubuki and Yukikaze; Fubuki's and Yukikaze's
star shells ahead of the American cruisers, Takao's searchlight on New Orleans and Pensacola's on Takao, 50 s in),
overcast and under a clear full moon, 1600 × 900, the High preset (`.build/native/look-night-lights/pairs/`):

| View | What changed |
| --- | --- |
| Chase (the player's) | The star shells' glitter is on the water (before, Pensacola's four lamps held the tactical places and the shells lit nothing on the sea); the teal wash the old star shells (`range² × 0.5`, three times the real candle) laid over the whole sea is gone; New Orleans is lit white by Takao's searchlight 7 km off (it lit her at a fifteenth of that) |
| Broadside at 1.5 km | Two star shells' glitter paths of sparkles; the ships in the beams and under the shells stand out |
| Deck from 100 m | Her upperworks' shadows across the deck from the star shell to starboard; the light warm-white |
| From 700 m up | The pool under each star shell, her shadow on the water beside her |

### What it costs

Apple M5 Pro, Metal, `dev-client` (the GPU's work is the release build's), the High preset at 1920 × 1080, the
staged action above at night and the same battle at noon without the star shells and searchlights, the chase camera
and the broadside at 1.5 km, the renderer's pass timestamps over 60 frames, two rounds interleaved
(`.build/night-cost.ts`; `NAVAL_POINT_LIGHTS=off` and `=shadowless` set beside the game's: no point lights at all, no
light maps). At load average 17–30 (several agents building and capturing; the frame totals swung by 3–6 ms between rounds, so
these are the passes' medians, which held better), 2026-10-20 19:13–19:25, branch merged with master (night-air's
air light pass included in both "after" columns):

| View | Pass | Before (master's release) | After | After, no point lights | After, no light maps |
| --- | --- | --- | --- | --- | --- |
| Night, chase | Water and sky | 6.37 | 5.54 | 4.22 | 6.14 |
| Night, chase | Opaque | 2.44 | 2.45 | 2.18 | 2.38 |
| Night, chase | Light maps (3) | – | 0.13 | – | – |
| Night, broadside | Water and sky | 5.11 | 5.19 | 3.75 | 5.88 |
| Night, broadside | Opaque | 1.00 | 1.10 | 0.91 | 1.01 |
| Night, broadside | Light maps (3) | – | 0.12 | – | – |
| Noon, chase | Light maps | – | none drawn | – | – |

The light maps cost 0.12–0.13 ms for three (a few hulls each), and save the sea 0.6–0.7 ms of marching toward the
lights they map (the "no light maps" column): net, they save time. Every light on the sea at once (7–15 of them here,
with the star shells now on it) costs 1.3–1.4 ms over none, about what master's eight cost (its water pass at night is
no cheaper, though its star shells lit nothing on the water). By day no map is drawn and the lights are the few
flashes and fires near the camera. Earlier rounds at load 6–10 (18:10–18:20, before the merge): the frame's GPU time
p10 over master's +0.7 ms from the chase and +2.0 ms on the broadside at 1.5 km, where the sea fills the view with two
star shells' glints.

### Fires and shots at night

Bill: "can you make shots at night a bit less bright and make fires significantly less bright but still a bit brighter
than before". "Before" is master before the night eye (a11ac14304): the strips in `.build/native/look-night-fire/`
(`muzzle-strip-{pre,now,after}.png`, `fire-strip-{pre,now,after}.png`; the effects trial's clips with the eye on).

**What the physical levers do.** With every light at the night's whole lift, a burning ship 130 m off closed the eye
8.3 stops (her flames, the brightest fiftieth, held at the display's white) and showed as a red glare on a black sky.
Her flames drawn at their own light's luminance (a tenth of what the game draws) still closed it 6.3 stops; a lower
ceiling closes it further; letting the flames stand past white blooms them over the view. The eye answers any light
that fills a view as it should, so a fire as bright as its real light stays a glare. At Bill's word the fires and the
flashes' light on the smoke take less of the night's lift (`naval_effects::EffectLighting::set_lift`; by day and through
dusk they take all of it as before, and the night's levels come in as the night does):

- **A fire's flames and embers glow at four times their day's glow** (sixteen since 2026-10-24, when their day's glow
  fell to about a quarter, and 32 since 2026-10-25, when its light on the scene fell from 2,000 to 500 times its day's to
  stand nearer proportion to them, spread among its flames and shadowed by her structure: [Flames and their
  light](../combat-effects/fire-and-smoke.md#at-night);
  `FLAMES_AT_NIGHT`; `fire.wgsl`'s tongues, the flipbook's flame, the embers' batch, a fire's gas wherever the flipbook
  does not draw it: its rolling flame bodies, `GAS_FIRE`, and its plumes and smoke, which carry its glow; `emission.y`),
  **its glow on its own smoke at twenty times** (`FIRE_GLOW_AT_NIGHT`, `emission.z`) and **its light on the scene at
  500 times** (`FIRELIGHT_AT_NIGHT`; some 2,000, an eighth of the lift, until 2026-10-25). Before the night eye they
  kept their day's glow, four to six times it on the smoke and some 470 times their day's light (`FIRE_NIGHT_LIGHT`).
  The eye on the burning ship stays open (0 against −8.3): her hull lit orange by her fires, the column dark over its
  lit foot, the clouds behind it.
- **The flashes' light on the smoke takes half the lift** (`FLASH_SMOKE_AT_NIGHT`, `emission.w`, `FLASH_ON_SMOKE`'s
  four flashes on the gas and its cap), a stop under their light on the scene (a thirty-second since 2026-10-25: "The
  night eye in a fight").
- **After dark the eye's ceiling stands a stop under a noon's highlights** (`EyeAdaptationSettings::night_highlight`,
  0.5 where `highlight` is 1.5, coming in as the night does: `Renderer::set_night_share`). What a light brings to the
  ceiling shows a stop darker: the salvo's flash-lit smoke, cream-white 0.7 s and 2.5 s after the flash, is tan; a star
  shell's glow filling a view a stop darker than the night eye left it.

The strips' frames, their means of 255 (`tools/frames.py`), before the night eye → with it → now:

| Moment | Salvo 0.7 s | Salvo 1.2 s | Salvo 2.5 s | Burning ship, 0.5 s | 3 s | 9 s |
| --- | --- | --- | --- | --- | --- | --- |
| Mean | 13.7 → 219.8 → 184.2 | 10.0 → 94.4 → 45.1 | 7.2 → 204.6 → 151.3 | 32.8 → 45.8 → 43.7 | 33.0 → 51.6 → 50.3 | 34.2 → 51.6 → 48.5 |

The burning ship's frame keeps about its mean, now a lit ship and a dark column against the clouds rather than an
orange veil on black. In a night fight (`burning-{chase,beam}-{now,after}.png`: two Bismarcks at 4 km, the far one
burning four minutes in) the chase view's mean is 90 → 60, the view no longer filled by the glare. The day's fire is the
same before and after (`fire-strip-day-{now,after}.png`).

### Lamp glints and salvo smoke

Three leftovers of a night fight (lane night-glints; pictures and commands in `.build/native/look-night-glints/`,
`shots.sh before|after`, `tools/`).

**The fairy lights.** Under a near star shell the lamps' glints near the eye bloomed into large soft round dots (the
night action's chase view, lower right; the overcast deck from 100 m), laying a brown veil over the sea between them.
With the bloom off they were fine points: the dots were the bloom of glints far brighter than their ceiling meant
(`GLINT_PEAK`, a glint's pixel at most twice its light's irradiance, as the sun's). Two causes:

- **The ceiling was every light's.** Each light's glitter is kneed at its own irradiance, but the ceiling on the glints
  was the sum of every light in the pixel's cell: Takao's searchlight lighting the water about New Orleans from 6.5 km
  (8 lux at grazing incidence, with no glint there), the other star shell, a flash. A star shell's glints were held
  three to ten times too high. The ceiling is now each light's irradiance weighed by its share of the glitter at the pixel
  (`surface.wgsl`, `held`): a light alone is held at its own, as before.
- **A wide cell's glint carried four glints' light.** The glints are drawn on cells about a pixel across, the two
  power-of-two sizes about it blended (`reflection::glints_at`); the coarser cell is wider than the pixel and lights every
  pixel it covers, up to four, each held to the ceiling. A lamp's image in a wavelet is a point (a flame or a mirror a few
  arcminutes across mirrored in a wavelet of centimetres is a few millimetres wide), so all of a glint's light falls in
  one pixel: `lamp_glints` holds a glint to one pixel's worth of light, a coarse cell's pixels each to their share. The
  sun's disc is 1.4° across here and its image in a facet near the eye fills pixels: its ceiling stays per pixel, and
  its glitter is unchanged.

The weighed ceiling took most of the dots and their veil, the glint held to one pixel most of the rest, about the
nearest star shell's own glints: the sea near the eye under a star shell is fine sparkles on dark water, a few of the
brightest keeping a faint glow. The glitter path toward a star shell keeps its
sparkles; the moon's and the sun's glitter are not the lamps' and do not change (noon is pixel-identical).

**The cream salvo smoke.** After a 38 cm salvo at night the smoke lit by the next turret's flash came out cream
(#1841's strip, 0.7 s and 2.5 s). With the bloom off the 2.5 s frame stayed cream: the smoke's own light. The gas, the
volume trial's bodies and the strike debris took the flash's old colour, `#ffd29a` (some 3,900 K, linear 1, 0.62, 0.3),
after the flash's light on the hulls and the sea went to 2,500 K (`#ffa64a`, linear 1, 0.38, 0.07) on 2026-10-20.
They now take the four flashes' colour weighed by their power (`EffectsFrame::flash_color`, the lighting's `flash_color`),
the flash's 2,500 K reddening to the fireball's glow as on the hulls: the smoke beside a turret firing is lit orange as
its deck is (frame's luma 128 → 112 of 255, saturation of its mean colour 0.29 → 0.55). The flashes' share of the night's
lift on the smoke (`FLASH_SMOKE_AT_NIGHT`) is Bill's and unchanged. The cream lasts the flash's first two or three
frames: the eye closes on it at the cones' 0.1 s while the flash's light falls with a time constant of 0.04 s.

At 0.7 s the next turret's flash comes while the eye is still some 6 stops closed from the first: the smoke itself is
dark brown (the frame's luma 45 of 255 with the bloom off, 82 with it), and what lights the frame is the bloom's glare
about the flash (`bloom.wgsl`, `GLARE_GAIN` 4: about a flash the bloom is five times as strong and flame-coloured
light blooms from three tenths of its threshold), from the flash's core seen through the smoke (82 → 76 here, the
smoke's own orange the only change). Left as it is: it is the flashes' look, and by the CIE's disability glare (the
sun's veil, `sun_glare.rs`) a flash's veil 5° off would be some twenty-five times fainter than the glare draws it. For
Bill.

**The star shell's halo.** Its "blocky band" did not show in the star shell views (`air-*`, the night action's,
overcast and clear, and from Takao 6 km off): the halo's radial profiles are smooth and the same in every direction
but the height's, the bloom's separable kernels draw no square about it, and the simulation's reach (the halo and
the cloud base it lights end at 3 km from the flare) leaves no edge in these views. A halo without the reach, its
span on a smooth (Hermite) phase integral, 24 samples instead of 3, and the air's own optical depth to the eye in
place of the aerial volume each changed the views by at most 5 of 255 and were dropped. The faint rings a strong
high-pass shows about the flare are also in the sky with the effects off.

| View | Before → after (frame's luma of 255) |
| --- | --- |
| Night action under star shells, clear: chase, 1.5 km abeam, deck from 100 m, from 900 m | 47.5 → 44.5, 105.9 → 105.9, 46.6 → 38.7, 108.7 → 108.7; the chase view's lower right 30.3 → 14.6 (the dots and their veil) |
| The same, overcast | 74.3 → 69.7, 105.3 → 105.2, 48.2 → 37.2, 93.2 → 93.2; the deck view's lower right 33.3 → 9.9 |
| A star shell overhead, a beam abeam, down a beam (overcast) | Pixel-identical; 0.05 % and 0.002 % of the pixels (glints under the beams) |
| A 38 cm salvo 80 m off at night (eight moments over 7.5 s, as played) | 0.7 s 81.6 → 75.5, 2.5 s 128.3 → 111.8, the rest within 0.1 |
| Noon (chase, broadside); the same salvo by day | Pixel-identical; within 0.1 |

**What it costs.** A few operations a lit sea pixel (the weighed ceiling, the glint's bound on two cells it already
drew) and a choice of colour in the gas: nothing that shows. Apple M5 Pro, Metal, `dev-client`, High at 1920 × 1080,
the staged night action (two star shells, two searchlights, every ship firing), overcast, 50 s in, 180 frames from the
chase camera and from 1.5 km abeam (`.build/native/look-night-glints/cost.ts`), master's client and this branch's
alternated, load average 65–110 (other lanes building), 2026-10-21 14:50–15:05: GPU busy p50 from the chase 13.34 and 13.31 ms
→ 14.32 and 13.21, abeam 9.49 and 9.52 → 9.64 and 9.62; the water and sky pass 5.23 and 5.13 → 5.40 and 5.06 (chase),
4.35 and 4.39 → 4.45 and 4.46 (abeam). By day the sea draws the lamps' variant only while a flash burns.

## Point lights and the sea

The sea takes the scene's point lights (`native/docs/ocean/light.md`, "The scene's point lights on the sea"): every
flash, fire and lightning's stroke lights it through its own glitter, water-body and foam models, inverse square with no
cut-off, the glints under the knee the sun's or moon's take. A salvo lays a glitter path to the eye like the moon's, a
burning ship an orange one across the swell, a stroke a white one. Since 2026-10-20 the sea reads the same list as the
hulls ("Lights at night"); a flash or a fire reaches as far as its light does, a star shell or a searchlight to the
simulation's reach.

## Scorch

`Fleet` holds a `ShipScorch` (`fleet.scorch`). Each frame the host hands it the strikes the events record
(`Fleet::scorch_strike(ship, &ScorchStrike)`: hull-frame point, HE or AP, outcome, the impact mark's width; an HE
burst's scorch is half its mark across, as the mark is sized from its charge since 2026-10-10; a torpedo leaves none) and every
burning or scorched hull's fires (`Fleet::scorch_update(dt, drift, &[(ShipId, ScorchHull)])`: per compartment its
vent, size and intensity; per mount its frame at its train, gunhouse and intensity; the hull's world velocity).
`Fleet::prepare` publishes the nearest 16 scorched visible hulls into a 66 × 16 `Rgba32Float` texture (binding 20 of the
ship frame group) with the web's row layout but its hull texel (the spot count and the fires' clock), and writes each
ship's row into its `FleetShip` record (the former padding), so a fragment reads only its own hull's row instead of
searching the rows by position as the web does. The damage applies to paint and teak
under the weather, as `ShipMaterialPalette.weather` layers it; impact marks and plain glTF materials take none.

Differences from the web, all small: the bounds are the model's exact vertices at the current pose (the web unions each
mesh's bounding box, slightly larger); raycasts walk the posed triangles with each part's sphere as the only cull (the
web uses its `SurfaceChunks`; since the frame-hygiene lane the searches skip runs of triangles by their bounds, with the
same results); a hull is measured once, on its first strike or fire (about 10 ms for a large preset then, 0.2–0.5 ms
now: `../frame-hygiene.md`).

**No burn-out (2026-10-25).** The web's scorch burned a lost ship out in 12 s: her superstructure charred and
rust-browned in patches all over, every spot half as big again, fully charred, and glowing red for minutes (fully for a
magazine or the structural fallback, 0.35 plus her fire for flooding or capsize). It read as red and orange marks all
over a ship the moment she blew up. Paint takes hours of fire to burn off a whole superstructure (Arizona burned two days
in harbour), and a ship lost to a magazine or flooding is gone in seconds to minutes (Hood in three), so a lost ship now
keeps what her fires and strikes left, and the loss's cause no longer reaches the scorch. Where a magazine breaks her,
her sections' torn edges draw as sooted steel (`HullSection`).

**No embers (2026-10-25).** The web's scorch lit embers in every charred seat: a shell strike's at half heat, cooling to
a third in 18 s, and a fire's at its heat, falling over 30 s once it was out, in blobs about half a metre across
breathing slowly over the whole seat. After dark they read as lava speckled over the struck upperworks. Painted steel
plate does not hold a glow once the flames over it are gone, so a strike's scorch carries no heat and charred paint
glows nowhere; only the bed under a fire's flames glows (`scorch_damage`, [a burning bed](../combat-effects/fire-and-smoke.md#a-fires-smoke-in-the-wind)).

**Open.** A broken hull's second section is scorched in her first's frame: the twin reads her first section's row
(`Fleet::set_twin`), whose world-to-hull matrix is the first section's, so the soot and char on the second section
slide off their places as the two part. `scorch_damage` could take the fragment's own hull-frame point
(`in.hull_position`) instead, and the row drop its sphere and matrix.
