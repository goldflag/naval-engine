# Combat effects: fire and smoke

## Where a ship burns

The simulation's fires burn where real ships' did ([docs/damage-model.md](../../../docs/damage-model.md#fires)): in her
upperworks, an eighth of her length a zone (`naval_sim::fire_zones`), in her rooms, and in her main turrets'
gunhouses; her secondary and anti-aircraft guns no longer burn one by one. The fires draw them so:

| Fire | Drawn | Flames | Light |
| --- | --- | --- | --- |
| An upperworks zone (`ShipFrame::zone_fires`) | Over the roof of its broadest structure (`ShipEffectsProfile::zones`, `ZoneVent`), its flames spread over 0.84 of that roof's width and length (at most 24 by 28 m), drawn at up to twice a room's size | At any intensity, as a gunhouse's | Fire light |
| A main turret (`mount_fires`) | On its gunhouse roof, as before | At any intensity | As before |
| A room with a shell hole above the water (`ShipFrame::openings`: the render packet's `FloodRender::opening`, the highest such hole) | At the hole, narrowed to it | Once it burns hard (0.3 to 0.65), as a hatch's did | Once it shows flames |
| A room without one | At its vent, lifted out of any structure over it to the roof it comes out on (`out_of_structures`) | None: its smoke column only, no embers | None |

So a burning ship shows one broad fire in each burning eighth of her superstructure, its smoke merging with her rooms'
columns into one pall, flames from her side only where a shell opened a burning room, and no flames on her guns but her
main turrets'. The scorch blackens each zone's roof as it burns, keyed after her rooms. A theatre stand-in seen burning
shows its flames in her largest zone (`theatre::damage::burning_zone`). A place gone under the sea burns no more: its
fire dies in steam on the water and its column is let go (since 2026-10-21, [a ship's loss](ship-loss.md)).

**Seen with** `bun native/tools/effects-trial.ts --only fire,flames,column,far,above,fire-chase,fire-lens,fire-deck
--stills`. Since 2026-10-24 these film the `he-fire` battle (`HE_FIRE`: a light cruiser's six-inch fire on a battleship
lying still, filmed at 270 s), since the Bismarck duel they filmed before no longer sets her alight: fires catch by
chance. `fire-chase` is the view that follows a ship, `fire-lens` binoculars at 3 km, and `fire-deck` her own deck
35 m from the fire.

## Fire inside her, and cook-offs

The fire survey of 2026-10-24 found a burning ship dark wherever her fire was not on a roof. Through the scuttles of the
deckhouse under `fire-glass`'s flames her rooms showed their daylight interiors (4–35 of 255 in red by day, 0–4 at night
against flames of 193 and 178); a shell hole into the burning deckhouse showed only its sooted pocket (13 by day, 1 at
night); nothing seeped from her doors and scuttles; and the guns standing in the fire did nothing. In the carriers burnt
in 1944–45 (Princeton, 24 October 1944; Franklin, 19 March 1945; Bunker Hill, 11 May 1945) the fire showed at the
openings of the burning decks, dark smoke poured from doors, scuttles and holes along the island and the gallery deck,
and the ready-use 20 mm and 40 mm rounds in the gun tubs and clipping rooms went off for an hour and more in irregular
bursts, tracers arcing out of the smoke. A room fire burns under its own smoke: the hot layer fills the deckhead and
comes down toward half the room's height as the fire grows, the flames low under it.

- **Lit glass** (`ship.wgsl` `glass_fire_seen`, beside the glass's room, `glass_room_seen`). The client tells her shader
  where she burns (`naval_ships::FireInside`, `inside.rs`): each burning upperworks zone's structures
  (`ShipEffectsProfile::zone_boxes`, from `naval_sim::fire_zones`) at the zone's intensity, seated on the roof its
  flames stand on, and each burning compartment, seated on its deck; up to 32 boxes, the hottest kept, laid each frame
  after the holes in the hole table (her ship record's `holes.w`). A window whose room lies in one is alight: its room a
  cell 4 m along the wall and a deck high, a share of them alight from 10% at the least heat to 45% at full, fewer above
  the roof the flames stand on (the heat falls to a quarter between 1 and 9 m above it). Through the glass the room's
  deck and walls are deep orange in the fire's light (linear 1 : 0.24 : 0.04, about 1,300 K), flames lick along its
  deck, and its smoke is banked dark under the deckhead down to 0.65 of its height at the least heat and 0.4 at full; a
  scuttle too small on screen for that shows their mean, 0.15 of the sun's units at full heat. Each room flickers on its
  own (a slow swell and a quicker flutter on the fires' clock). After dark it takes the flames' lift
  (`EffectLighting::flames`, `FLAMES_AT_NIGHT`), so it stays under the flames over it.
- **Glowing holes** (`fs_hole`). A hole torn into a burning place glows from within: its pocket the brighter the deeper
  in (0.45 of the sun's units at its back), its tongues' inner faces and torn edges in its light (0.15, 0.06), all of it
  in patches rising and flickering with the flames behind. A burning room's hole already carries its flames and column
  ("Where a ship burns").
- **Smoke from her openings** (`fires.rs` `seep`, `seep_places`). Each burning zone's structures have places where a
  door, a scuttle or a window would let smoke out, found once from its boxes: every 5 m along each wall, 1.4 m over each
  deck, up to 3 m above the roof its flames stand on, not where another structure stands against the wall. From 12% of
  them at the least heat to 45% at full, at most ten a ship and within 1.5 km, puff a thin dark wisp (albedo 0.10–0.18,
  0.4–0.65 m across, a metre more as it drifts out and up, gone in 4–6 s) every one to three seconds at full heat. They
  give way to the columns: none while the smoke's batch is over 95% full (the columns keep to 92% of it).
- **Cook-offs** (`naval_effects::cookoffs`). Each secondary or anti-aircraft mount standing in a burning zone
  (`FireZone::mounts`, `ShipEffectsProfile::zone_mounts`) has its ready-use rounds. While the zone burns hard (intensity
  0.45, full from 0.85) each 4 s a gun may start a burst, in 14% of them at the full rate: about two bursts a minute a
  gun, of one to four rounds over 1.8 s, thinning as six minutes' rounds at the full rate are spent. A round goes off on
  the deck round its tub or gunhouse (up to 1.2 m out, 0.8 m up: one inside a turret would not be seen): an orange-white
  spark of a flash, 0.25–0.45 m for 0.035 s, lighting only the gun's side and the deck beside it (`FlashRequest`, 1–3
  within 4–6 m for 0.03 s: under the bloom's glare), 8–17 sparks within 2.5 km, a small grey-white puff (0.35–0.6 m, 0.8
  m more as it rises and thins over 3–4.5 s), and one to three rounds flung out at 18–70 m/s every way but down, slowing
  as they tumble and falling, their tracers lit 1.5–4 s (drawn by the shells' tracers, `shells.rs`, as 30 mm rounds with
  ids below zero). Its crack is the flak burst's clip at 0.22–0.4 of its level and 1.45–1.9 times its rate, the smaller
  round the higher (`naval-client`'s `audio.rs`, `cook_off_cue`, by `Director::cue`). All of it follows from the ship,
  the gun and the effects' clock (hashed), the same at any frame rate; the simulation knows nothing of it. After dark
  its flash and sparks take the fires' flames' share of the night's lift and its light the fires' light's
  (`EffectLighting::flames`, `firelight`), not a gun flash's: until 2026-10-25 a pop at night was a white star with the
  bloom's spikes, 0.7–1.4 m and lit 40–100 at the full lift, that lit her upperworks white and closed the eye (the
  flames beside it 242 to 129 of 255 in `cookoff` at 2.35 s, --eye); now the flames hold at 242 and the pop is an orange
  spark (205/140/106 where it was white).

`fire-glass` (the burning deckhouse from 25 m off her beam) and `cookoff` (the same fire from 45 m off her beam and 9 m
up), regions' means (of 255, red; the pop's and the tracer's peak), master's build and now:

| | Day before | Day now | Night before | Night now |
| --- | --- | --- | --- | --- |
| A scuttle alight | 4–35 | 45–88 | 0–4 | 56–69 |
| The hole into the deckhouse | 13 | 66 | 1 | 66 |
| The flames over it (unchanged) | 193 | 193 | 178 | 176 |
| A pop's ball (`cookoff` at 2.35 s) | 51 | 180 | 122 | 255 (205, orange, since 2026-10-25) |
| Its tracer at 2.45 s (peak) | 63 | 242 | | |

By day the lit scuttles are orange dots about as bright as her sunlit deck (55), three to five times the shaded wall
about them (8–18); at night they and the hole glow at about a third of the flames over them, four times the wall (15).
At night a pop lit the upperworks round it as a gun's flash does until 2026-10-25 (above). The wide `fire` view, 130 m
off, is unchanged: its frames' mean within 0.4 of 255, the pixels differing by more than 12% under 2.5% (smoke drawn
differently, and the battle ensigns merged since master's build).

Beside the black-body flames merged the same day ("Flames and their light": dimmer by day, `FLAMES_AT_NIGHT` 4 to 16,
which this glow takes too), the same still: by day the flames 140–156 and the lit scuttles 40–77; at night (`--eye`) the
flames 201–213, the scuttles 89–139 and the hole 121, still under them.

**Cost.** GPU busy at 1280 × 720 (`--cost 20` on `fire-glass` and `fire`, four and eight moments, two rounds alternating
with master's build; the dev-client build, whose GPU work is the release's; M5 Pro under the other fire lanes' load):
within the machine's noise. The opaque pass, which draws the glass and the holes, took 5.0–8.2 ms before and 5.6–7.9 ms
now at `fire-glass`, 5.7–8.3 and 6.8–10.0 ms at `fire` (its second round under heavier load, the frames' totals 17–26 ms
against 10–17). Only the glass and hole pixels of a ship something burns in pay: `fire_inside` returns at once for a
ship nothing burns in, and tests at most 32 boxes otherwise. On the CPU a preset's seep places are found once, a frame
takes at most ten a ship, and a burning gun draws one hash every 4 s.

**Seen with** `bun native/tools/effects-trial.ts --only fire-glass,cookoff,fire --stills` (by day; `--hours 22 --eye` at
night). `cookoff` takes a still every half second for 12 s and four about the first pop in sight (an anti-aircraft
gun's, above the fire's right, at 2.3 s); the probe's `cookoffs` gives the rounds popped, in flight and the clock they
go off by.

## Flames and their light

**What the survey saw.** By day, 40 m from a burning turret (`flames`), the flame's core was 196/150/110 on the screen
(red over blue 1.8), cream beside the sunlit turret at 164/168/172: the fires' flames took the explosive's ramp (an
ember's red to a flame of (4.2, 1.55, 0.3), and white above a heat of 1.25), about four times as bright as a real fire's
flame and yellower, and AgX pales so bright an orange to cream. From above (`above`) and through the smoke in `fire` and
`column` the flames shone over their own column, as they drew after it and it was not in the occlusion map.

**What real fires do.** A big fuel fire's luminous flame is soot glowing at about 1,300–1,600 K at any instant (lower
averaged over time; its light follows the hot moments). Planck's law integrated against the CIE 1931 observer gives
650 cd/m² at 1,300 K, 2,450 at 1,400 K, 7,800 at 1,500 K, 21,500 at 1,600 K and 118,000 at 1,800 K (600,000 at
platinum's freezing point, 2,042 K, the old candle's 60 cd/cm²: the check), green a tenth to a sixth of red, blue next to
none. Grey paint in the noon sun (100 klx, albedo 0.3) is about 9,500 cd/m². So by day a fire's core is as bright as
sunlit paint to twice it and deep orange, its cooler edges a tenth of that and red-orange.

**What is drawn now.** A fire's flames glow as a black body (`common.wgsl`'s `black_body`, a fit of the table within a
few per cent, and `flame_light`) at physical brightness in the sun's units (a unit of the scene's radiance is
16,700 cd/m² by day, `naval_render::lights::DAY_LUX`: sunlit grey paint is about 0.57). The heat the flipbook's flame
gives (`flipbook_flame`: about 1.4 through a full fire's thick core) maps to 800 + 790 (heat / 1.4)^0.355 K: 1,590 K at
1.4 (19,400 cd/m², twice the paint), 1,500 K at 1.0, 1,385 K at 0.6 (a tenth of the core), dull red below, nothing in
the soot; a fire's rolling bodies of burning gas (`GAS_FIRE`) to 1,610 K. That covers every fire's flame: the
flipbook's flame of a room, mount or upperworks zone fire (`gas_sprite.wgsl`, `fire`), the bodies, the tongues of the
look before the flipbook (`fire.wgsl`), and the embers, brands at 1,380–1,520 K (`fires.rs`, `Rgb::black_body` in
`math.rs`, the shader's twin). A gun's propellant and fireball, the flash and a shell's burst keep their white heat.
After dark the flames' lift was sixteen times their day's glow (`FLAMES_AT_NIGHT`, four before): their day's glow fell to
about a quarter, so at night they shone as Bill's levels had them ([lighting](../lighting/lights.md), "Fires and shots at night").
It is 32 since 2026-10-25 ("At night", below).

The fires' columns go into the smoke-occlusion map (look `column-cover`, on; `-column-cover` for the look before): their
links and billows add their optical depth and its nearness into the map's two free channels (`depths.ba`,
`gas_sprite.wgsl`'s `column_occluder`), and what draws after the columns reads them (`columns_through`): the flipbook's
flames and the bodies (their batch's flags' z 2), the tongues and the embers, which fade behind the column's smoke in
front of them. A column's smoke is patchy, its depth swinging about the map's mean from eddy to eddy, and a patchy
screen lets through the mean of e^(−τ), more than e^(−mean τ): taken as spread exponentially about its mean
(`COLUMN_PATCHES` 1), (1 + τ)^(−1), a tenth through a mean depth of nine (2026-10-25; e^(−τ) let through a
ten-thousandth, and from above the flames under the column were black). Nothing else reads those channels, so the gas, the merged smoke and the columns' own links draw as
before. A column's foot, the glowing puffs the flames stand in, must not black them out from the side: a puff counts
only where its near surface stands above the flames it rose from (`fire_batch.rs`' `clear_of_flames`: where it left
them, raised by the radius it left with, carried in its quad's w), fading in over a quarter of its radius or 2 m.

| Scene, by day | Before | After |
| --- | --- | --- |
| `flames`, the flame's core (mean) | 193/146/108 | 140/74/50 |
| `flames`, its brightest twentieth | 252/233/210 | 236/164/119 |
| `flames`, the sunlit turret top beside it | 164/168/172 | 162/167/171 |
| `fire-deck`, the flame's brightest twentieth | 250/228/201 | 215/158/124 |
| `fire` (130 m), the flame's brightest twentieth | 246/221/191 | 219/144/105 |
| `above`, where the flame stood over the column | 232/199/166 | 91/91/93 (the column's smoke) |
| `above`, the flame's region, its brightest twentieth (two moments) | 232/199/166 | 85/84/86 and 58/58/60, the smoke, until 2026-10-25; now 113/89/84 and 109/66/54 |

At night (22:00) the flames keep their brightness and turn deeper orange: `flames`' core 248/222/193 → 254/220/189 at a
fixed exposure, and 204/156/113 → 224/150/106 with the eye on (the same luminance, 163 of 255). From above the flames
under the column glow dull red-orange through it, patchy, brighter where it is thin (until 2026-10-25 it hid them
whole); from the side and on her deck (`flames`, `fire-deck`) they stand in its foot as bright as before the cover (the
patchy screen changed `column` and `fire` by 0.02–0.1 % RMSE, `above` by 0.7 %).

**Cost** (`--cost 20 --profile release`, 1280×720, Apple M5 Pro, 2026-10-25, the other fire lanes' clients and builds
sharing the GPU; `above`, `fire`, `column`, two runs with the cover beside one with `-column-cover`): the occlusion pass,
now timed (`smoke occlusion`), took a median 0.04, 0.02 and 0.00 ms of the three scenes' moments with the columns in it
against 0.06, 0.03 and 0.02 ms without; the transparent pass 1.0–1.6, 0.9–1.2 and 1.6–2.3 ms against 1.6, 1.0 and
1.9 ms. Nothing measurable either way: the frame's own swings (17 to 30 ms GPU busy from run to run) are the other
sessions'. The column's puffs draw into the quarter-resolution map once more, and the flames, bodies and embers read
one texel each.

Seen with `bun native/tools/effects-trial.ts --only flames,fire,column,above,far,fire-deck --stills` (`--hours 22` for
the night, `--trial=-column-cover` for the flames through their smoke).

### At night

**What the survey saw.** At 22:00 with the eye on, 40 m from the burning turret (`flames`), the flame's core was
212/167/124 and the deck 20 m off 206/133/89: lit as bright as the flame lighting it. With a fixed exposure, 130 m off
(`fire`), her deck 80 m forward of the fire, behind her turrets from it, was 69/39/27, lit through them. Three causes:
the fires' light on the scene took 125 times the flames' share of the night's lift (`FIRELIGHT_AT_NIGHT` 2,000 against
`FLAMES_AT_NIGHT` 16); a fire's light cast no shadow (only the searchlights and star shells have maps); and it shone
from one point over the middle of its place, 2.5–5 m up, while its flames stand spread over up to 24 × 28 m.

**What real fires do.** A matt surface lit by a flame of luminance L filling a projected solid angle Ω of its sky is
ρ L Ω / π bright: at most ρ L right inside it, 0.1–0.3 L beside a big flame, about L / 40 at 20 m from a 10 m flame.
By day the fire's light (`FIRE_LIGHT_PER_KW`: a quarter of its heat radiated at 0.1 lm/W, 60,000 cd for 30 MW) is
a sixth to an eighth of what the flames the flipbook draws would give (some 50 m² of flame at a mean 6,000–7,000 cd/m²
for a scale-1 fire fully alight, 300,000–400,000 cd): the flames and their light stand in proportion where the light
takes six to eight times the flames' share of a lift.

**What is drawn now.**

- **The levels** (`combat.rs`). After dark the flames glow at 32 times their day's glow (`FLAMES_AT_NIGHT`, sixteen
  before) and their light on the scene is 500 times its day's (`FIRELIGHT_AT_NIGHT`, 2,000 before): sixteen to one,
  where it was 125. The flames are drawn a stop brighter and light the scene two stops less; where they fill enough of
  the view to close the eye (`flames`, 40 m off) it closes further. The fire inside her and a cook-off's flash and
  sparks (below) take the flames' level, so they double with them, and a pop's light falls with the fires' light. The
  look `bright-flames` (`BRIGHT_FLAMES_AT_NIGHT`) takes the flames a stop further (64) with the same light: eight to
  one, as the drawn flames would light the scene; where they close the eye, what they light shows a stop darker again.
  **Bill to pick.**
- **Spread among the flames** (`fires.rs`, `illuminate` and `flame_lights`). A fire within 500 m of the lens
  (`FIRE_LIGHT_SPREAD`) shines from up to four points (`FIRE_LIGHT_POINTS`) where the flipbook draws its flames: each
  sprite's light at 0.35 of its flame's height above its foot, leaning as it leans (`FLAME_LIGHT_HEIGHT`), with a share
  of the fire's light as its flame's area; the two nearest merged while there are more than the room or two stand within
  4 m (`FIRE_LIGHT_APART`). Each flickers on its own, so the light moves among the flames. The fires' 32 lights
  (`FIRE_LIGHTS`) go to the most relevant first, leaving each fire after one; the renderer keeps the 64 brightest at the
  eye of every light. A hatch's fire too low to show flames, and the look before the flipbook, keep the one point.
- **As big as its flames** (`naval_render::PointLight::radius`, `lights.wgsl`'s `point_light_at`, the uniform's
  `shadow.w`): a surface nearer a light than its size (0.3 of its flame's height) takes the light it would at that
  distance, as one beside a body of flame of luminance L takes no more than ρ L. Without it a point a metre off a
  bulkhead lit it hundreds of times brighter than the flame.
- **Shadowed on the hulls** (`ship.wgsl`'s `fire_seen`, `naval_render::lights::LightCaster::Screen`, the uniform's
  `shadow.z`). After sunset (once the fires' light is lifted more than four times its day's, `FIRELIGHT_MARCHED`) a
  hull's pixel marches toward each fire light on screen through the hulls' depth for the frame's camera (the contact
  shadows' `CONTACT_DEPTH`, bound in the place of this crate's own sun maps, group 1 binding 8, which keeps the core's
  pipeline within sixteen sampled textures a stage; cleared to the far plane when the contact shadows stop marching),
  as the sea marches toward its lamps (`light_seen`): 16 steps in batches of four, the first shifted by interleaved
  gradient noise moved each frame; a step is hidden where a surface stands in front of it by more than 0.3 m and 0.4 %
  of its distance and less than 30 m or 5 % of it; the march stops the light's size (at least 2 m) short of it. Two
  marches a pixel at most (the brightest lights at the eye first), each toward a light that adds at least 2 % of the
  sun's and sky's light there (`FIRE_SEEN_SHARE`); a fire's other lights within 8 m of one marched toward
  (`FIRE_SHARED`, its flames' other points) take that march's answer, so a pixel marches about once a fire. The
  rounded edges (`edge_lit`) take what the face found. A hole's own surface, the wires' impostors (a thin member's
  share of a pixel) and a sighting's silhouette are not marched. What the screen does not show hides nothing: off its
  edges, and hulls past the contact shadows' reach (some 450 m at the game's lens) or without contact shadows (the Low
  preset, a second view). By day nothing is marched.

| At 22:00 (median, of 255) | Before | After | `bright-flames` |
| --- | --- | --- | --- |
| `flames` (40 m), eye on: the flame | 230/153/107 | 250/183/141 | 250/184/142 |
| the wall beside it | 133/76/41 | 129/64/25 | 119/50/13 |
| her deck 20 m off | 234/173/131 | 108/47/16 | 82/25/1 |
| the next turret | 201/143/99 | 84/34/5 | 54/14/0 |
| `fire` (130 m), fixed exposure: the flame | 252/200/162 | 255/225/197 | 255/241/224 |
| her deck at the fire | 211/149/108 | 181/114/74 | 205/137/93 |
| her deck 80 m forward, behind her turrets | 111/60/40 | 27/20/24 | 57/20/16 |
| `fire`, eye on: her deck at the fire | 213/152/111 | 200/131/90 | 222/153/109 |

The flame is the brightest thing in every view, a third to half a stop brighter than before, and what stands right
beside it in its light stays about as bright (the wall at 40 m, a third of a stop darker; her deck at the fire at 130 m,
under three quarters of a stop, and a third with the eye on). What stands off it, or behind her structure from it,
darkens: with the eye on, the deck 20 m off by some three stops, where it had shone brighter than the flame (a matt deck
20 m from a 10 m flame takes about a fortieth of its luminance, five stops under it; on screen it now stands three and
a half under, the flame's top pressed by the tone curve, and the gunhouse's foot shades the deck beside it), the next
turret by three and a half, and the deck 80 m forward behind her turrets by two and a half to three. Where the flames
fill enough of the view to close the eye (`flames`), brighter flames darken all they light: under `bright-flames` the
deck 20 m off is a further stop down. Before is master at b97b6a62de, with the fire inside her (below) already merged.

**Cost** (`--cost 20 --profile release --hours 22`, 1280×720, Apple M5 Pro, 2026-10-25, two runs each alternated with
master's baseline, the machine otherwise quiet): the scene's opaque and transparent passes +0.59 ms in `flames` (40 m,
the ship filling the view), +0.65 in `fire` (130 m) and +0.54 in `fire-deck`, the frame's GPU time +0.60 to +0.75; of
it the spread lights and the levels take +0.12 to +0.14 (measured without the march) and the march some 0.45 to 0.5.
By day, when nothing is marched, the march's code in the ship shader costs some 0.17 ms. Three traps on the way: the
hidden lights' mask as a private array indexed by the lights' word went to the stack and cost the ship pass 5 to 7 ms a
frame even by day (a vector now); four marches a pixel, the wires' impostors marching too, cost 1.0 to 1.3 ms; and
eight steps, 0.1 ms cheaper, left a coarse grain along the shadows' edges.

Seen with `bun native/tools/effects-trial.ts --only flames,fire,column,far,fire-deck,fire-chase,fire-lens --stills
--hours 22` (`--eye` for the eye, `--trial bright-flames` for the brighter flames; `--binary "$(bun
native/tools/baseline.ts b97b6a62de)"` for before).

### A fire close up

Close over a burning ship's upperworks at noon (`shot.ts`' fight, `--camera effect:fire`) three things were wrong:

- **Flames ended in straight cuts where they met the deck and plating.** A flame is a flat card standing on its roof;
  where the card passed into the deck or a coaming the depth test cut it along a line. The fire bodies' batch is now
  soft against the scene, and a fire's flame card stands at the front of the body of flame it stands for, its half-width
  nearer the lens than the fire (`FLAME_FRONT`, was a quarter of its width), fading out over 1.4 half-widths where the
  deck or plating behind it lies within that body (`FLAME_DEPTH`). Plating in front of the whole body still hides it,
  as it would a fire.
- **Thin dark wedges at the edges of the smoke near the top of the frame.** Not the fire's: the damaged Iowa's black
  funnel smoke, its ribbon twisting end on; the dark holes through her grey column were the same smoke drawn after the
  column. The funnels' puffs (above) have neither.
- **The superstructure under the dense column read pale pink-white.** Traced: (1) the fires' point lights were the noon
  sun's own strength at 10 m by day (340 cd a scale in the scene's units) and orange, lighting the shaded upperworks
  pink. A fire's light by day is now what its flames give off (`FIRE_LIGHT_PER_KW`: a quarter of its heat radiated, a
  tenth of a lumen a watt at 1,300–1,500 K, 3.6 for 30 MW, half a percent of the sun at 10 m); after dark it kept the
  night scenes' light (`FIRE_NIGHT_LIGHT`, 1,700 a scale) until 2026-10-20, when the night's lift took its place
  (`native/docs/lighting/night.md`, "The night eye"); since 2026-10-21 some 2,000 times its day's light after dark, an eighth of the lift
  (`native/docs/lighting/lights.md`, "Fires and shots at night"), and 500 times since 2026-10-25 ("At night: flames and their light in
  proportion", below). (2) The smoke
  shaded the ships' sun but not their sky: the ships' smoke term (`naval-ships`' `ship.wgsl`, `smoke_sky`) now takes
  the sky's light through the smoke over her as the smoke's own puffs do, down to `SKY_UNDER_SMOKE` (0.45) under the
  deepest smoke with a high sun, from the soft occluders' transmittance. (3) Not this lane's: at a fixed exposure the
  upperworks under the column are well shaded, but the eye adaptation opens two stops on a frame of dark smoke
  (`naval-render`'s `EyeAdaptationSettings`), which lifts the shaded ship toward pale.

**Stills**: `fight-effect-fire`, `fight-effect-funnel`, `fight-broadside` pairs beside the funnel's.

## A fire's smoke

### How fast a fire's smoke climbs

The owner watched a carrier burn at 14 kn and found the smoke over her deck rising too fast. It was. Every column's
puffs left their fire at 15 m/s (`rise` 5 + 7i, `lift` 2.2 + i over 8 s), whatever the fire, and each plume link rolls
its smoke up itself at a further 0.35 of that. Smoke tracked in the frames climbed at 10–15 m/s just over the fire,
1.5 to 3 times as fast as a real fire's plume. Now (`fires.rs`: `column_climb`, `column_heat`, `plan_columns`):

- **The plume.** Heskestad's plume rises on its axis at 1.03·Qc^(1/3)·z^(−1/3) m/s (Qc the convected heat, 0.7 of
  the heat in kW, z metres above the fire), and in the flame no faster than McCaffrey's 1.93·Q^(1/5). The smoke one
  sees is the plume's mean, half the axis speed. Fitted to the puffs' `rise·e^(−t/τ) + lift` over a minute, the climb
  is the same at any heat but for its scale: τ = 8.3 s, rise 0.30·Q^(1/4), lift 0.206·Q^(1/4).
  - A 30 MW fire's smoke leaves at 6.7 m/s and is 50 m up after 10 s and 200 m after a minute. Before, any full fire's
    left at 15.2 m/s and was 100 m up after 10 s.
  - The column's growth and diffusion go as Q^(1/4) too, so its width at a height is what it was (about a seventh of
    the height, plus its puffs' size at the fire).
  - Its puffs live 36 + 20i (+ up to 8) s, which takes a full column about as high as before (200–250 m).
- **The heat.** A place of scale 1 burning in the open at full intensity gives off 30 MW (a gunhouse or a 10 m
  square of upperworks fully alight, 0.25–0.5 MW/m²). It goes as the scale squared and as 0.2 + 0.8 of the
  intensity, and a room's fire gives half. A battleship's broadest superstructure zone (scale 2) is 120 MW, and each
  of the carrier's hangar fires in the `carrier` scene gives off 10–20 MW.
- **Her fires merge.** Columns side by side climb apart off their fires and merge as they widen. A column's rise comes
  from its own fire's heat and its lift from the heat of the columns it has merged with by then: the sum of her
  burning fires' heat, each weighted by e^(−d/40 m).
- **Under way and in wind.** Air crossing a plume bends it over, and then it climbs only as 1.6·F^(1/3)·x^(2/3)/U
  (Briggs; F = g·Qc/(cp·T·ρ), U the air's speed past her). Over the first minute that keeps 0.95 of the rise and
  0.56 of the lift times F^(1/12)·U^(−1/3), each at most all of it. U is the effects' wind less her way, the air her
  smoke is drawn drifting in (the weather's whole wind at the fire's height since 2026-10-24, "A fire's smoke in the
  wind" below; 0.35 of it before, a choice kept from the web game). A 30 MW
  fire at 29 kn is 32 m up after 10 s against 50 m in still air, and a burning ship's column now trails low aft of
  her rather than standing up out of frame.
- **Her flames lean into the column's foot.** The air past them (0.6 of the wind less 0.55 of her way) leans them by
  its speed over the column's starting climb (`flipbook_fires.rs`, `Fire::climb`). Before, it was over fixed speeds
  of 11–15 m/s. The flames lean further under way and in wind, and a following wind takes back her way's lean as it
  does the smoke's. Since 2026-10-25 they lean as a pool fire does, by the air past them over the fire's own
  characteristic velocity, not the column's climb ("A fire's flames and pall in the wind" below). Once a fire is out, its heat follows its dying flames down to nothing
  (`heat_share`).

The `effects_probe` RPC gives each column's heat, rise and lift (`columns`); `effects-trial.ts` writes the filmed
fire's beside each clip (`<scene>-camera.json`) and logs it. Unit tests hold the climb to the integrated plume law
within an eighth over a minute, to Briggs within a fifth under way, and the column's links to touching at rest and
under way (`fires.rs`, `a_column_climbs_as_a_fires_plume_does`, `a_chained_column_stays_whole_at_rest_and_under_way`).

**Measured in frames.** The scenes were filmed with master's fire code (before) and this one (after), with the
same ticks and cameras (`bun native/tools/effects-trial.ts --only column,fire,underway-column,carrier`). Corners in the
moving dark smoke within 15–25 m of the filmed fire were tracked for 0.25 s with Lucas–Kanade. Their heights were read
on the vertical plane through the fire. The table gives the median climb in m/s.

The law column is half Heskestad's axis speed at that height, from the fire's own heat to its merged heat. Under way
it is Briggs's bent-over climb, with U the air past her (22 m/s for the battleship and 13 m/s for the carrier, from
the probe's lift). Each scene's heats are in its `<scene>-camera.json`.

| Scene (own heat, merged heat) | Above the fire | Before | After | Law |
| --- | --- | --- | --- | --- |
| `fire`: Bismarck lying still, 130 m off (120 MW, 227 MW) | 15–30 m | 14.6 | 9.6 | 8.0–9.9 |
| | 50–80 m | 10.7 | 7.8 | 5.6–6.9 |
| `column`: the same, 450 m off, 30 s | 25–50 m | 13.4 | 6.4 | 6.7–8.3 |
| | 50–80 m | 7.6 | 5.2 | 5.6–6.9 |
| | 80–120 m | 7.9 | 5.6 | 4.9–6.0 |
| | 120–170 m | 7.8 | 6.5 | 4.3–5.3 |
| `underway-column`: Bismarck at 29 kn, 450 m off (120 MW, 294 MW) | 15–30 m | 7.1 (15.3 at 30–50 m) | 4.7 | 2.9–4.6 |
| `carrier`: Shōkaku at 14 kn, 18 fires, 300 m off (16 MW, 360 MW) | 20–40 m | 10.0 | 4.7 | 1.2–5.7 |
| | 40–80 m | 12.4 | 4.0 | 0.8–4.0 |

The carrier's pall now leaves her at about 19° above the horizontal: it climbs 4–5 m/s and drifts aft at 13.5 m/s.

The look is kept: the flipbook flames, the plume-link column, its near-black fuel-oil smoke and its height at rest.
Under way the column now bends over aft into a low pall, as burning ships' smoke did in the photographs.

**The flames were right and are unchanged.** In the fire book the flame puffs about three times a loop: 0.70 Hz at the
bake's pace (`FIRE_LOOP_SECONDS`, 4.27 s). It is drawn at √(1.5/scale) of that pace, 0.86 Hz for a fire of scale 1.
The book's flame stands on a base 0.475 of its height wide, and pool fires puff at 1.5/√D Hz (Cetegen and Ahmed). At
scale 1 the drawn flame's base D is 4.1 m at full intensity, puffing 0.74 Hz, so the drawing is 1.16 times as quick.
At intensity 0.3, D is 2.1 m and 1.03 Hz, and the drawing is 0.83 times as quick. The pace's root of the size is the
Froude scaling.

**The effects' time** is the frame's dt times the theatre's running speed (`app/session/frame.rs`, `TheatreDriver::effects_rate`:
1 at 1×), and each pool and batch advances once a frame. Nothing else speeds it up. The funnels' smoke leaves the
rim under forced draught at its own speed, has its own model and is unchanged.

**Open.**

- The merging weight (e^(−d/40 m)) is a heuristic for plumes merging as they widen.
- Under way, a line of fires climbs 30–40 % below what Briggs gives for their summed heat, because each column's rise
  is its own fire's.
- A plume link's texture rolls its smoke up at 0.35 of its puff's climb (`fire_batch.rs`), so what the eye follows
  climbs 1.35 times as fast as the puffs. That keeps it within the law near the fire but above it at the column's top
  (the `column` scene's 120–170 m).
- The effects' wind was 0.35 of the weather's (`EffectLighting::set_wind`), so in a strong wind at rest the column
  stood up more than it should. Since 2026-10-24 it is the whole wind ("A fire's smoke in the wind" below).

### A column of billows, not a tube

The owner found a burning ship's smoke too orderly, like a funnel's: each column was a chain of plume links of one
width, laid end to end on one line, so it rose as an even tube. A burning ship's smoke in the photographs (and in
Sea Power) is a heap of billows large and small that bulge out of the column, thick and thin by turns, its line
swinging as it climbs. Now (`fires.rs`, `puffing`, `wander`; `fire_batch.rs`, `FireParticle::lean`):

- **It puffs.** A fire puffs its smoke out in swells of 3 to 9 s (`puffing`, a sum of three sines from the fire's
  phase), and its puffs are drawn up to 0.4 larger in a swell and smaller between them, their opacity 0.15 lower
  between. Each puff's own size spreads 0.7–1.3 (was 0.8–1.2).
- **It wanders.** The eddies over the fire swing the column's foot off its line (`wander`, turning over 7 to 22 s): a
  puff leaves with a sideways drift of up to 0.3 of its climb, so the column snakes by several metres either way.
- **Its links lean.** Each link lies off the column's line at its own slant, up to 0.4 m across per metre along
  (`lean`), so they stand out of its sides rather than stacking into a tube.
- **It heaps up in billows.** 5–55 % of its puffs (more in a swell) are drawn as round heads among the links: the
  fireball book's smoke (`clip::SMOKE`, as a gun's bank), 1.4 puff radii across (`BILLOW_REACH`), thrown off the line by
  up to a quarter of the climb. They are drawn on the links' lean path, sorted with them; `GAS_COLUMN` in
  `GasPuff::extra[3]` keeps such a puff upright in its tile (that path does not turn it), and it never burns. Links alone
  lie end to end into an even tube with soft edges; the billows give the column the crisp rounded heads of a burning
  ship's smoke.

The climb is unchanged (the speeds above), and the chain stays whole: `a_chained_column_stays_whole_at_rest_and_under_way`
finds no step wider than 0.7 of the reach of the puffs either side (0.44–0.47 before).

### A fought fire's smoke

A fire a damage-control party is on (`FireState::suppressed`) used to draw its whole column one even grey, so beside
an unfought fire's black column a ship showed two smokes of two colours. The two colours are right: hoses turn the
smoke pale with steam, and burning ships' photographs (Bunker Hill and Franklin in 1945) show black and white columns
side by side. The flat grey was not right. A fought fire's smoke is mottled: steam where the water reaches it, the oil
fire's black where it breaks through, changing as the party gains or loses ground. Now (`fires.rs`, `hoses`, `EffectRules::hoses_gaining`):

- **How far the hoses have it** (`hold`, −1 to 1): 0.6 of how fast its heat falls, 1 at the fall a party at its full
  rate makes against a fire's full growth (`SUPPRESSION_PER_SECOND − FIRE_GROWTH_PER_SECOND`), plus a swing of ±0.5
  over 13 to 27 s as the hoses reach it and lose it.
- **Links** stay grey with steam, a little paler as the hoses gain (0.28–0.82 of the way from soot to steam, by
  `hold` and the link's seed). A few (18 % − 10 % of `hold`) are its soot breaking through, at 0.08–0.16 of the way:
  darker streaks up the column. Links are still not mixed black and white one by one, since that would show as
  bands up the column.
- **Billows** carry the contrast: each is a burst of white steam (thinner, opacity 0.6) or a head of black oil smoke,
  white for 55 % + 25 % of `hold` of them. The puffs path (with the flipbook look off) takes the same share as steam.

`a_fought_fires_column_is_mottled_and_paler_as_the_hoses_gain_on_it` holds a fought column's links grey with dark
streaks, its billows black and pale, and more of them pale when its heat is falling.

On 2026-10-17 the steam was darkened (`STEAM_SHADE`, 0.78 to 0.45: the shade the links and billows go toward, and
the puffs path's steam), since a fought and an unfought fire still read as two smokes. The fought column is now a
grey oil smoke a shade or two above the unfought one's black, its steam billows a light grey rather than white, and
its links keep half of an unfought column's firelight from below after dark (a third before).

### A fire's smoke in the wind

The fire survey of 2026-10-24 filmed the `he-fire` battleship (184 MW of fire under her column, a 9 m/s wind) and
found her smoke wrong in four ways:

- **It stood up in a wind that whitecapped the sea.** The effects drifted everything in 0.35 of the weather's wind
  (`EffectLighting::set_wind`). That came from the web game, where the wind was a parameter of the waves' shader (it
  first drifted funnel smoke on 2026-09-06, at 0.28 and 0.11 of it, then "35 % of the ocean wind" when combat smoke
  was made to follow the map's); nothing physical set it. Her column met 3.2 m/s of air and climbed off the fire at
  8.9 m/s (rise 5.58, lift 3.27).
- **It ended in a rounded cap some 250 m up.** Its puffs lived 36 to 64 s and climbed for all of it, so the column
  stopped where its oldest puff died. A real fire's smoke climbs until its buoyancy is spent, levels off and drifts
  downwind for kilometres as a pall.
- **Its soot read grey.** At 130 m (`fire`) the column's heart was 60–70 (sRGB, a blue-grey, 66/67/72) and its
  sunlit billows 96–111 (109/111/118) against a sky of 143/165/191; the paler smoke at its foot 90–120. Fuel-oil smoke
  in sun is brown-black, browner where the sun catches it (Bunker Hill on 11 May 1945, Princeton on 24 October 1944,
  the colour film of Pearl Harbor).
- **The plating under its flames did not glow** until it had charred, some two minutes in.

What real fires do, and what is drawn now:

- **The wind.** Smoke drifts with the air at its height. The effects take the weather's wind ten metres up, whole
  (`EffectLighting::wind`), and each puff the wind at its own height: the sea's power law in near-neutral air,
  u(z) = u₁₀·(z/10)^0.11 (Hsu, Meindl and Gilhousen, 1994), from 2 m up, to the clouds' drift, 4/3 of it, which it
  reaches some 140 m up (`combat::wind_share`; particles, the funnels' smoke, flak, the fires' columns). Gun smoke and
  funnel smoke take it too: nothing held the 0.35. What floats keeps its share of the 10 m wind (`flotsam.rs`'
  `DRIFT_SHARE`, 3.5 %), as the ocean's oil does, so a burning patch on the sea now drifts with its oil (it drifted at
  a third of the oil's pace). The bent-over climb (`column_climb`) takes the wind at the fire: her column now leaves at
  7.2 m/s (rise 4.93, lift 2.30) and leans downwind as burning ships' columns do. The flames leant with 0.6 of the
  10 m wind over the column's climb, as before: 0.75 (37°) in 9 m/s at rest, where a pool fire of her size would lean
  some 60° (the AGA correlation). They lean by that correlation since 2026-10-25 (below).
- **Where it levels off.** A plume rises until its buoyancy is spent: Briggs's final rise of a bent-over plume in
  neutral air, 1.6·F^(1/3)·(3.5x*)^(2/3)/U (x* = 34·F^(2/5) m, 14·F^(5/8) under 55 m⁴/s³), at most the top of the
  sea's boundary layer, whose inversion stands 500 to 1,500 m up (`INVERSION`, 1,000 m) (`fires.rs`' `column_top`).
  Her 184 MW at rest in 9 m/s levels off 606 m up, three kilometres downwind; under way at 29 kn into that wind,
  some 220 m; a 30 MW fire in it 200 m; a 5 MW one 70 m. Each puff eases into it (`FireParticle::climb_at`:
  top·tanh(climb ÷ top), its own climb near the fire).
- **Its pall.** A chained column's links now live up to seven minutes (`PALL_SECONDS`, 420 s) and the chain keeps
  fewer of them the wider and slower they get (`link_life`, ranks to 10): a handful of links hundreds of metres
  across, minutes old, lie along the top and drift off downwind at 12 m/s, thinning as they spread to a fifth of the
  column's optical depth (`column_look`) and fading over the last third of their lives, four kilometres downwind. A 24 MW
  fire keeps some 80 links alive after six minutes (36 of them its pall) where it kept about 20; the batch's share for
  it (`demand`, about 160) holds. The magazine's pall drifts at the wind at its height too.
- **Its colour.** The column's light is nine tenths the sun's on its sunlit side (measured 2026-10-24 in the `fire`
  view, the sun's and the sky's terms drawn in turns of stripes: the sky gave a tenth), so its soot's cast shows
  there: the column keeps the whole brown cast of its soot (it kept a third), which is browner (1.18 : 1 : 0.74, was
  1.1 : 1 : 0.86). The sky and the air between take it back toward grey at a distance.
- **Light scattered many times** within gas (`gas_sprite.wgsl`'s `MULTIPLE_SUN`, `MULTIPLE_SKY`, "The smoke's light
  and size") was set on gun smoke and applied whatever the gas's albedo. Each scattering gives back the gas's albedo
  of the light, so it now goes as the gas's reflectance over the darkest gun smoke's (`MULTIPLE_ALBEDO`, 9 %), at most
  all of it, and a gun's own gas keeps it all: gun smoke is unchanged to the pixel. A fire's flame sprites' soot (5 %)
  keeps three fifths, a strike's soot two fifths, a funnel's black smoke (2 %) a quarter; steam and a funnel's grey
  smoke keep all of it. The column's links have no such term (their sky is a tenth of their light).
- **A burning bed.** What burns on the plating under a fire's flames (paint, deck coverings, fittings, oil run onto it)
  glows from the first seconds, before it chars. Each fire's scorch spot carries its bed, the footprint of its flames
  and where they stand (`ScorchSpot::bed`, `bed_at`; the fourth and fifth texels a spot in the scorch row): an upperworks
  zone's whole roof (`RoomFire::flames`), a gunhouse's roof, 0.4 of its longer side either way, a compartment's hole
  above the water, 1.2 to 1.6 m (`RoomFire::hole`), and none for a compartment showing only smoke, as `LocalizedFires`
  stands their flames. `scorch_damage` lights it by the spot's heat (`HEAT_RISE` 2 s, `HEAT_FALL` 30 s) over the oval
  its flames stand in (0.8 of the footprint): patches a hand to a metre or two across among dark ones, each flaring and
  falling back one to three times a second, more of them and more orange the hotter the fire, on what the flames stand
  on and a third as much on the foot of a wall standing in it. Its radiance goes from a dull red to 1.2,
  0.17, 0 at full heat (`SCORCH_BED`, `scorch_glow`: a black body at 1,480 K, 6,200 cd/m², a third of the flames' core
  at 1,590 K, `common.wgsl`'s `flame_light`); after dark it keeps its day's glow where the flames take sixteen times
  theirs (`FLAMES_AT_NIGHT`). Charred paint holds no embers since 2026-10-25 ([no embers](../lighting/lights.md#scorch)).

Measured in the same frames (before: master; after: this change):

| Scene | Before | After |
| --- | --- | --- |
| `fire` (130 m), the column's heart | 60–70, blue-grey (66/67/72) | 42–53, brown-black (46/42/39) |
| `fire`, its sunlit billows | 96–111 (109/111/118) | 88–93 (88/83/79) |
| `column` (450 m) | upright to 250 m, rounded cap | leaning downwind out of frame, 70–75 (74/71/71) |
| `fire-pall` (6 km off, square to the wind) | a short column standing up | climbing at 20–40° over the first kilometre, levelling off 400–600 m up and drifting nearly four kilometres downwind |

**Cost** (`--cost 20`, GPU busy, median of the stills; before: master's 384ec27 build, after: this one, both with other
sessions' clients on the GPU, ±0.5 ms): `fire` 10.6 → 11.0 ms, `column` 7.0 → 7.5 ms, `fire-pall` 5.1 → 4.9 ms. The
transparent pass, where the column and its pall are drawn, takes 0.08 ms more in `fire` and `column` (0.63 → 0.71,
1.21 → 1.29 ms); the rest of the difference is the opaque pass, as much with the burning bed switched off in this build
(its minimum 6.24 ms against 6.21 with it): master's own changes to the ship shader since that build (dents, the
waterline), not this one's.

**Seen with** `bun native/tools/effects-trial.ts --only fire,flames,column,fire-chase,fire-lens,fire-pall --stills`
(and `--hours 22 --eye --only flames` after dark). `fire-pall` films her from 6 km off, square to the wind and abreast
of a point 2.2 km downwind, 150 to 270 s after the moment (`pall`). Unit tests: `smoke_drifts_with_the_weathers_wind_at_its_height`
(`combat.rs`), `a_column_levels_off_where_its_buoyancy_spends_itself`, `a_columns_smoke_spreads_downwind_as_a_pall_for_minutes`
(`fires.rs`), `rows_carry_the_hull_and_its_spots` (`naval-ships`' `scorch.rs`).

**Open** (both taken up on 2026-10-25, below). The pall widened only as its links do (`COLUMN_DIFFUSION` with the
fire's heat): no wider across the wind than up and down, where a real pall spreads several times as wide as it is
deep. The flames' lean stopped at `MAX_LEAN` (0.9) in a fresh wind.

### A fire's flames and pall in the wind

- **The flames lean as a pool fire does.** A pool fire's flame tilts from the vertical by the air past it over the
  fire's own characteristic velocity, u* = u ÷ (g·m″·D/ρₐ)^⅓: upright for u* ≤ 1, past it cos θ = u*^(−½) (the
  American Gas Association's correlation, Thomas's; the SFPE handbook's). Fuel oil burns at some 0.05 kg/m²s (m″) in
  air of 1.2 kg/m³; D is the fire's width, 2·√(half x·half z) of its roof or hatch, and u the wind at half the flame's
  height less her way (`flipbook_fires.rs`' `lean_in`). The leant flame no longer goes by the column's climb (it
  leant with 0.6 of the 10 m wind and 0.55 of her way over that climb, at most 0.9: `Fire::climb` and `MAX_LEAN` are
  gone). A burning roof 15 m across stands upright in 1.8 m/s and leans 39° in 3 m/s; a hatch 3 m across, upright in
  1 m/s, 43° in 2 m/s. The `he-fire` battleship's upperworks fire (19 m across, 10 m/s at half its flame) would lean
  63°. **It stops at 50°** (`MAX_TILT`): the flame is a card sheared along its lean (`flipbook_stand`), and filmed at
  40 m (`flames`) at 50, 55 and 63° its tongues hold at 50, begin to streak along the deck at 55 and are a flat
  smeared sheet at 63. A big fire reaches the cap in 5 m/s of air, a hatch's in 2.6, so in a fresh wind nearly every
  fire leans 50° (37° before). From her own deck (`fire-deck`) the flames stand lower and lie aft along the roof.
- **The pall spreads across the wind.** Downwind of a source a neutral plume spreads across the wind as
  σy ≈ 0.08·x/√(1 + 0.0001·x) and up and down as σz ≈ 0.06·x/√(1 + 0.0015·x), x metres downwind (Pasquill–Gifford
  class D, Briggs's open-country fit; `fire_batch.rs`' `dispersal`): 270 and 91 m at 4 km. A link's smoke reaches
  about its radius either side of its line, some two σ, so its own σ is half its radius; where σy or σz at its
  distance from its fire is the wider, the link takes it (`pall_spread`): its cross-section an ellipse that many
  times its own across the wind and up and down, round where it stands up the column. The distance is from where its
  fire is now, had the ship held her way (the link's `wake`), so a ship under way trails a spreading pall in calm air
  too. The sprite shader draws the link as much wider as that ellipse is seen across the screen and, seen end on, as
  much taller as it is seen up it (`gas_sprite.wgsl`'s `place`, the link's `evolution.zw`), and its extinction
  thinner by the area it gains, so the soot across it is the same; the smoke's shadow takes a ball as much wider and
  thinner (`shadow.rs`' `fire_blobs`). The column's own diffusion already grew about as σz: a 184 MW fire's link,
  4 km off, is some 230 m in radius (115 m a σ), so its pall keeps its depth and widens across the wind, about 1.8
  times at 1 km, 2.2 at 2 km and 2.3 at 4 km, some 1.1 km across there to its two σ. A 24 MW fire's thinner pall
  widens about 3.9 times at 4 km and deepens 1.3 times. Seen from the side across the wind it is unchanged; from
  above or below it is that much wider and fainter. Near the fire nothing changes: σ is under the column's own growth
  for the first few hundred metres.

Measured in the same frames (before: master; after: this change; the `he-fire` battleship in 9 m/s at rest):

| Scene | Before | After |
| --- | --- | --- |
| `flames` (40 m) | leaning 37°, tongues up | leaning 50° downwind, tongues kept; the uncapped 63° a flat smear |
| `fire-deck` | flames standing over the roof | lower, lying aft over it |
| `column` (450 m), `fire-pall` (6 km, square to the wind) | the column leaning off, its pall 4 km long | the same (`fire-pall` to the pixel): seen across the wind the pall keeps its depth |
| `fire-pall-above` (5 km up, over the cumulus) | a narrow brown streak between the clouds | a wider, fainter band |
| `fire-pall-under` (under it, 3.5 km downwind) | a narrow cone of smoke overhead | a wider, thinner fan spreading overhead |

**Seen with** `bun native/tools/effects-trial.ts --only flames,fire-deck,column,fire-pall,fire-pall-above,fire-pall-under --stills`.
`fire-pall-above` films the pall from 5 km up and 2 km off its line, over the cumulus (750–1,400 m), and
`fire-pall-under` from a ship 3.5 km downwind under it, looking back up, both 150 to 270 s after the moment
(`pallScene`). The probe gives an upperworks fire's half extents (`half`), and the trial logs its width. Unit tests:
`a_wide_fire_lies_over_in_a_fresh_wind_and_a_small_one_stands_in_light_air` (`flipbook_fires.rs`),
`a_pall_spreads_across_the_wind_as_a_neutral_plume_does` (`fire_batch.rs`).

**Open.** The flame card is sheared, not bent: a flame that lay over further would need a book of flames simulated
in a crosswind. Over the deck the flames (50°) now lie over further than the foot of the column above them (its
smoke leaves at 7.2 m/s in 3 m/s of drift there, some 25°): real flames hand over to a plume bent as far.

## Funnel smoke

Seen from above (the start screen's view straight down, a camera 650–900 m up) a funnel's plume was a smooth, opaque
beige band with hard edges: the ribbon is a strip turned to face the lens, so from above it is flat, one opacity
across, and its opacity fell only as its width's 0.35th power. Seen end on it twisted into thin dark wedges (a burning
Iowa's black funnel smoke showed them in the fire close-up, and, drawn after the fires' columns whatever lay in front,
dark holes through her grey column). From the side it read well.

Now (`funnel.rs`, `draw_puffs`; look `funnel-puffs`, on; `-funnel-puffs` draws the billows and the ribbon) the plume is
puffs of the gun smoke's simulated smoke (`clip::SMOKE`, seen from its sides) laid along the same released samples and
the rim, the trail's bend, length and colour unchanged:

- **Placed as the links are.** Each stretch between two samples is cut into eight places carried by the air with them;
  a place is drawn only while the plume needs it (places whose number ends in more zero bits stand in for their
  neighbours as it widens), so the puffs stand 0.55–1.1 of their radius apart all the way along, each fading in or out
  as it is needed. A puff's radius is 0.45 of the plume's width, 0.8–1.2 of that by its place, and it stands off the
  spine by up to 0.22 of the width across and 0.12 up: the rim is ragged, a few puffs' thin edges. By the rim a puff
  stands out of the funnel, its gas reaching down to the mouth.
- **It widens.** `puff_spread`: the hot exhaust takes in air fastest in its first seconds (as the billows grew), then
  keeps widening downwind: for an 8 m mouth some 45 m across ten seconds out, 70 m half a minute out.
- **It thins.** `puff_optical`: the optical depth through the plume's middle is what it stopped at the rim
  (`opacity0`) times the mouth's width over the plume's: its soot spreads over a section that widens, as a real plume's
  does. Each puff stops a share of that over its disc (`PUFF_OVERLAP`), with the smoke clip's own thinning made up
  (`smoke_depth`, measured over the clip's frames of both books). The plume's last stretch clears over its life as the
  ribbon's did.
- **Merged with the gas.** The puffs are a batch of their own ("funnel smoke": soft against the scene, in the
  occlusion map, untinted), added up with the gun smoke and the soot in the merged pass and laid over the scene before
  the fires' columns, so a burning ship's column hides her funnel smoke behind it and is hidden by funnel smoke in front
  of it. The merged targets are now made in any scene with funnel smoke in view (port included): 10 bytes a pixel.
- **Its shade.** The smoke's shadow balls (`shadow_blobs`) take the puffs' width and optical depth, so the plume shades
  the ships and the sea as dark as it is thick. A cruising plume is translucent and its shade on open water is faint:
  from straight down at 09:00 the sea under it darkens by a few levels at the fixed exposure, as faint as a hull's own
  shadow there; a sooty or burning ship's smoke shades it plainly.

Checked by `funnel.rs`' `a_plume_of_puffs_runs_from_the_rim_and_widens_and_thins_downwind` (puffs in place of billows,
ribbon and links; one at the rim; no gap along the trail; wider and thinner downwind, at rest and under way).

**Stills** (the before the cached master client, the after this branch's dev-client, same commands):
`.build/native/look-effects/` in the effects worktree, `<name>-before.png` and `<name>-after.png`.

**Cost**: `effects-trial.ts --only funnel,funnel-far,fleet --cost 30 --size 1728x1030`, dev-client, the High preset, Apple M5
Pro, load average 22–25 (2026-10-20), the puffs against `--trial=-funnel-puffs`, medians over each scene's moments of the
effects' passes (the transparent pass, the merged smoke, the smoke's shadow maps): a battleship's plume close abeam
0.29 → 0.50 ms (whole frame 7.95 → 8.29), from 600 m off 0.71 → 0.86 ms (6.66 → 6.93), twelve ships' plumes from
1.8 km 0.91 → 0.94 ms. A few tenths of a millisecond more where a plume fills much of the view; the billows' overlap
(twenty layers near the rim) is gone, the gas shader's reads are dearer.

**Open.** From steeply above a puff still shows a side of the simulation (no top view is baked). The shade on open
water from above is faint for a translucent plume; whether the sea's sunlit share is right is the ocean's question.

**Its CPU (2026-10-26).** Publishing a fleet's puffs walked every funnel's trail every frame, eight places a link, and
was a tenth of the main thread in Eastern Solomons. Three cuts that draw the same puffs to the bit (a hash over 6297
puffs in 228 publishes, before and after): a plume wholly outside one of the view's planes by the most any of its puffs
could reach is skipped before its samples are copied (`plume_unseen`, held to the puffs' own test by
`a_plume_ruled_out_of_view_draws_no_puff_there`); a link whose most-ranked place is two ranks short draws nothing and
is skipped before its frame and its samples' optical depth (a logarithm each) are worked out; and the drawing order
sorts one number a puff (`order_key`). Eastern Solomons on Saratoga, three rounds of 40 s: the effects' prepare 1.12 →
0.75 ms, 119.5 → 123.5 fps. Left: `FunnelSmoke::update` (the trail's exact drag, an exponential a sample) and
`shadow_blobs` (four transcendental functions a kept sample; the shadow map's fit reads all of them, so they cannot be
culled to the view), each a few per cent of the main thread.
