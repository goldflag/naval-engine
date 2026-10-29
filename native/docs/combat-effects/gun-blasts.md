# Combat effects: gun blasts, blast waves and flak

## A turret's blast

### Stage by stage

The photographs of an Iowa's broadsides show a heavy turret's blast in stages, and the game's departed from them at
every stage after the first hundredth of a second: its fireball was a flat, pale peach blob (the explosive's flame
ramp through the display transform), turned to black soot within half a second, and left a ball of charcoal smoke that
was gone in ten seconds. Now (`blasts.rs` `turret`, `gas_sprite.wgsl` `propellant_flame` and `propellant_glow`):

| Stage | When | What is drawn |
| --- | --- | --- |
| Flash | 0–0.1 s | A white-hot core and a flame tongue per bore, the flare, and since 2026-10-24 the gas burning white-hot as one ball as tall as her superstructure, the flash's light held while it burns white ("The flash, a white ball" below) |
| Fireball | 0.03–0.25 s | The secondary flash's lobes burn as propellant: white-hot with the ball through the first quarter of their burning, then yellow and orange through most of the fireball, red where it thins and in pockets mottled with fresh soot by 0.2 s (since 2026-10-24; until then orange to half a second). A salvo's heads swell by the cube root of its barrels and are thrown aside of the line of fire, so the fireball is lobed. The air stops its lobes within about a tenth of a second (drag 10–12, growth easing at 8/s), so it stands at its full reach while it burns |
| Smoke | 0.2–0.6 s | The flame's last pockets go out by about a third of a second (a 38–40.6 cm turret's; sooner for lighter guns), leaving its smoke dark brown with fresh soot, paling to `gunsmoke_dark` by 0.6 s |
| Cloud | 0.5–18 s | The bank of brown-grey gunsmoke (`gunsmoke`, darker brown nearest the guns, lighter from a light gun), thrown out to about 60 m and spread along her side more than up or down, some 60–80 m long two seconds on, dense at first, thinning within three to five seconds and gone in 12–18 s (4–5.5 s for a light gun; until 2026-10-23 thrown to about 90 m, its billows growing to 45–100 m, dense for ten seconds and gone in 18–28 s: "The smoke's light and size" below). It is torn into two or three clumps (`Clump`), each thrown its own way and as far as 0.6–1.25 of the bank's reach, rolling up at up to 12 m/s and climbing and drifting (0.45–1.3 of the wind) at its own pace, of billows mostly middling and a few great ones, and a few small puffs are thrown out past it, so it stands in ragged masses and comes further apart as it drifts (2026-10-10; before, one rounded bank) |
| Sea | 0–3.4 s | The blast sheet (`../water-effects.md`, "The blast on the sea under the guns"; torn into strands of spray since 2026-10-24) |

**The propellant's flame.** A puff marked `EffectParticle::propellant` publishes `GAS_PROPELLANT` in `GasPuff::extra[3]`
(0 smoke and explosives, 1 a smoke screen; the fireball's lobes and its flash's ball, marked `EffectParticle::fireball`
too, `GAS_FIREBALL` since 2026-10-24: "The flash, a white ball" below), and its heat is the share of its burning still
to come: 1.25–1.35 out of the bore, falling to nothing over its cooling (0.12–0.36 s × √flash, the head longest, since
2026-10-24, 0.2–0.58 s before; 0.15–0.25 s × √size in the bank's hottest billows). As it falls, the burning front
rises through the simulation's flame channel (×2.4 for a flipbook frame, ×1.2 for the atlas's still puffs, whose baked
flame runs twice as hot) and the flame reddens. Its ramp keeps little green and almost no blue below white heat because
AgX pales any colour as bright as flame towards cream: the explosive's ramp at heat 1, (4.2, 1.55, 0.3), displays as
`#fbe6cc`, and the most saturated yellow the display transform can show is about `#e2b97a`, at (1.4, 0.52, 0). The
pool's second tint by the albedo (`GasBatch.flags.w`) applies to the propellant's scattered light, not to its flame.
Explosives, magazines and ship fires keep their ramp. A propellant puff casts its smoke shadow from a heat of 0.9 down,
fully below 0.3 (`shadow.rs`, `PROPELLANT_GLOW_HEAT`). The fireball's glow on the hull, the sea and its own smoke is
0.2 of the flash's power with a time constant of 0.14·√flash s, out in about 0.6 s.

**How long it burns (2026-10-10).** The owner found the fireball burning too long against real life, and it did: a
heavy turret's stood fully alight at 0.7 s and burned in pockets to about 1.2 s, its glow lit the sea for 1.8 s and the
bank's billows flickered with flame to about a second. Films of battleships' broadsides show the fireball whole for a
fifth of a second or so and gone into smoke by about half a second. The lobes' cooling went from 0.55–1.7 to
0.2–0.58 s × √flash, the bank's from 0.45–0.75 to 0.15–0.25 s × √size, the glow's time constant from 0.45 to 0.14 s ×
√flash and the burning grains' life from 0.5–1.3 to 0.35–0.85 s; the lobes stop and grow sooner (drag 7–8.5 → 10–12,
growth decay 5.5 → 8, their growth raised to keep their full size), so the shorter fireball is still as large.
`combat.rs`' tests hold the fireball below 0.75 of its heat at 0.3 s and out at 0.8 s.

**Colours.** Brown-grey since 2026-10-23 (`gunsmoke` `#beb9b2`, `gunsmoke_dark` `#a8a197`, some 24 % and 13 %
reflectance; "The smoke's light and size" below). Until then the grey the owner had kept (`#9c9fa5`, `#85888f`).
The photographs show it tan; a tan fitted to them was tried on 2026-10-09 and set aside: stored `#f0d6a0` (the pool
tints its gas twice, so a stored colour is about the square root of the reflectance), its lit side in the noon sun
measured `#90795c` to `#d2bb95` (5th to 90th percentile, `muzzle-side` at 4.5 s) against `#805020` to `#e8e1b7` in the
photograph of an Iowa's cloud lying beside her, with the fireball's soot dark brown (`#95897f`) paling into it over 4 s.
The flame was tuned against the display transform with the shader's maths and AgX ported to Python over the
flipbook's frames before any build.

**Seen with** `bun native/tools/effects-trial.ts --only muzzle,muzzle-side,muzzle-deck,muzzle-far,muzzle-above,muzzle-low,muzzle-light --stills`,
master's build with `--binary` against this one. Scenes added for this: `muzzle-deck` (from her own deck toward the
turret's nearer end, along her engaged side, as the photograph from an Iowa's forecastle), `muzzle-far` (from 1 km off
her quarter and 450 m up, for 16 s: the cloud on the sea) and `muzzle-light` (a Fletcher's 12.7 cm mount from 45 m).
`--hours 23` for the night, `--trial=-flipbook` for the still puffs.

**Cost.** GPU busy at 1280 × 720 (`--cost 20`, RTX 5070 Ti, Windows, other agents building), `muzzle-deck` (the
fireball filling much of the frame) and `muzzle-side`, eight moments each against master's build: the transparent pass
0.02 to 0.07 ms more over the deck view (0.07 → 0.09 ms at 0.08 s, 0.21 → 0.28 ms at 3 s), the side view within its
noise. Whole frames rose 0.03–0.21 ms on the deck view, most of it in passes the change does not touch.

**Open.** Close up (within about 100 m) the bank's billows are soft where the photographs' are crisp: the simulation's
256² frames are drawn 40–80 m across. The flame tongue's additive sprites draw before the gas, so the fireball hides
them; the pale jets out of each bore in the photograph from the forecastle are only the lobes' tongue at the muzzle.
The explosive's ramp still pales to cream through AgX in HE bursts, magazine detonations and ship fires.

### The flash, a white ball

The owner's North Sea film showed each turret's flash as an orange ball lasting about half a second. The films of the
Missouri's broadsides from a ship alongside (gathered 2026-10-22) show something else at every moment: at 0.03 and 0.1 s
an overexposed white ball, nearly round and as tall as her superstructure, her hull and the sea under it lit gold; at
0.2 s a red-orange fireball mottled with black soot; at a third of a second dark grey-brown smoke. The game's ball was a
third of that size at 0.03 s, its fireball uniform orange at 0.2 s and orange in pockets at 0.35 s (the films' 0.2 s).

- **The ball** (`blasts.rs` `turret`): one still puff a turret, `FLASH_BALL` 24 m × the flash's size × the cube root
  of the barrels across (an Iowa's triple 16-inch turret's 38 m, Hipper's twin 20.3 cm turret's 13 m, a destroyer's
  5-inch mount's 5 m), half its width ahead of the muzzles and a tenth of it up, born at 0.7 of its size and full within
  0.03 s. It burns white for `FLASH_HOLD` 0.1 s × √(the flash's size) and is gone soon after behind the fireball's flame.
- **White heat** (`gas_sprite.wgsl` `propellant_flame`): the ball and the fireball's lobes are `GAS_FIREBALL` (5,
  `EffectParticle::fireball`), propellant that burns white-hot through all but its thinnest gas while its burning
  still to come is over `WHITE_HEAT` (0.95–1.15): the first quarter of a lobe's burning. Its heat there is 1.95–2.4,
  hottest where the gas is thickest, and `propellant_glow`'s ramp goes on past white to (9, 8.4, 5.6), a white that
  overexposes, for this gas only.
- **The fireball** cools sooner: 0.12–0.36 s × √(the flash's size) (0.2–0.58 s before). It is yellow and orange to
  about 0.15 s, red in pockets by 0.2 s and out by a third of a second for a heavy turret, a tenth sooner for a cruiser's.
- **Fresh soot** (`fresh_soot`): a fireball's gas reflects half its smoke's colour while fresh, paling to the smoke's
  own between 0.1 and 0.6 s: black specks in the red at 0.2 s, a dark brown cloud at a third of a second. The bank's
  billows (`GAS_PROPELLANT`) keep their colours.
- **The light** stands at the ball's middle, at 3500 × size² × √barrels (2800 before), held at its full power while
  the ball burns white (`FlashRequest::hold`) and then spent over 0.22 s as before. The fireball's glow after it is
  unchanged.

`muzzle-low` (an Iowa's turret from 320 m off her engaged side, 12 m up, noon), the luma of a box in the fireball's
middle (of 255) at the films' moments, master's and now, against her hull beside it:

| Time | Fireball before | Fireball now | Hull before | Hull now |
| --- | --- | --- | --- | --- |
| 0.03 s | 165 | 221 | 88 | 117 |
| 0.1 s | 204 | 241 | 87 | 116 |
| 0.2 s | 190 | 128 | 80 | 81 |
| 0.33 s | 139 | 86 | 79 | 79 |
| 0.5 s | 102 | 99 | 78 | 78 |
| 1 s | 85 | 109 | 78 | 78 |

A second on, the cloud is 1.4 times as bright as her hull, as the Missouri's is. At night (`--hours 23`, fixed
exposure) the frames white out to 0.2 s where master's did to 0.33 s. The North Sea film's first broadside, low view and
Hipper's salvo, filmed again, show the white ball at each turret, its fireball orange by 0.1 s for her 20.3 cm guns and
smoke by 0.2 s. `combat.rs`' tests hold the ball at 32–42 m across with the lobes, the fireball under 0.9 of its heat
at 0.2 s and out at 0.4 s, and the light held while the ball burns.

**Cost.** GPU busy at 1280 × 720 (`--cost 20` on `muzzle-deck` and `muzzle-low`, eight moments each, two rounds
interleaved with master's build, M5 Pro under other agents' load; the dev-client build, whose GPU work is the
release's): the merged smoke pass, which draws the ball, within its noise (from her deck 0.40–1.18 ms against
0.48–1.83; from 320 m, in the quieter round, 0.25–0.91 against 0.24–0.76). The frames' totals swung by several
milliseconds with the load.

### The smoke's light and size

The owner's North Sea film (`north-sea-raid`, 16:36, partly cloudy) showed every turret's smoke as an ink-black ball
twice a ship's height: its cores measured 9–22 of 255 against a sky of 142–160, where films of an Iowa's broadside show
a warm brown-grey about half the sky's brightness and 1.3 times her shaded hull's, a cloud of 60–90 m two seconds on
whose dense part thins within two to four seconds. Measured with `effects-trial.ts --hours 16.6` against master's
build, the black came from the light, not the colour:

- **The cloud's shadow.** At 16:36 the scene's smoke stood in a cloud's shadow (`puff_sunlight`'s cloud term: without
  it alone, the `muzzle-low` core rose from 21 to 69). Under a shadow or an overcast only the sky lights smoke, and the
  sky came half from the tile's top and a tenth from its front, whichever way the tile looked: a bank's face toward the
  eye stood in the dark, from above its whole face did, and a dense bank's shaded side got nothing back, since the six
  baked directions hold only light scattered once.
- **Its colour.** The pool tints the gas twice (`GasBatch.flags.w`), so `#9c9fa5` reflected some 12 %.

Now (`gas_sprite.wgsl`, `common.wgsl` `sky_toward`; `blasts.rs`):

- **The sky each face sees.** A direction's lobe of the six-way weights gathers 2π/3 of the radiance about it where the
  sun's beam gives its irradiance, and `sky_fill` is the irradiance from overhead, so each lobe takes 2/3 of the sky's
  light that way (`SKY_LOBE` 0.39 × 1.7). The face toward the eye takes the environment toward the eye and the back the
  environment beyond the gas (`sky_toward`: the sky and the sea mirroring it, so under a low sun the horizon's glow);
  the four sides take the sky overhead by the hemisphere's rule (`sky_share`, the sea at `SEA_SKY` 0.12 of it). The
  vertex hands the fragment how far the tile's y and z axes look up (`Varyings.light.zw`, in the location that held
  only the two shares), and the x axis is the rest of the way, turned with the tile.
- **Light scattered within.** Dense gas keeps shares of the sun's and the sky's light reaching it (`MULTIPLE_SUN` 0.08,
  `MULTIPLE_SKY` 0.6), reached as its optical depth grows (`MULTIPLE_REACH` 0.35).
- **Its foot on the sea.** Seen along the sea a puff thins away over its lowest quarter radius above the water
  (`SEA_FOOT`, by its height on its own plane, undoing `flipbook_quad`'s pull toward the lens), less the more steeply
  the eye looks down on it; fires' flames and gas keep their foot. The soft edge fades by the depth behind a puff, which
  does nothing at so flat an angle: the sea hid the rest of each card along a straight line.
- **Brown-grey** (`gunsmoke` `#beb9b2`, `gunsmoke_dark` `#a8a197`, measured on 2026-10-22 against the films).
- **The films' size.** The bank's billows start at 6–17 m (were 10–28), grow by 12–26 m a second easing off (24–52) and
  spread 0.3–0.8 m a second (0.8–2), thrown out to about 60 m (90); its dense part thins over 3.5–5 s (8–10) and it is
  gone in 12–18 s (18–28); the strays likewise smaller, and the fireball's sooty smoke outlives its flame by a few
  seconds, not seven to twelve. The fireball keeps its size. `combat.rs`' test holds a 40.6 cm turret's billows under
  30 m across their middle and its bank under 140 m along the line of fire five seconds on, standing at eight and gone
  by twenty.

`muzzle-low`'s core one second on (`.build/smoke/tools/box.sh`, luma of 255; the hull is Iowa's dark side under her
superstructure):

| | 13:00 smoke / hull | 16:36 smoke / hull | `muzzle-side` 16:36 |
| --- | --- | --- | --- |
| Master | 66 / 72 | 13 / 43 | 8 |
| Now | 103 / 82 | 37 / 44 | 33 |

Funnel smoke and the magazine's column barely change: their soot is dark either way. Seen close from her deck the brown
cloud reads softer than the darker one did. Fire columns' links keep their own light (`link_fragment`).

**Cost** (`effects-trial.ts --profile release --cost 20`, 1280 × 720, Apple M5 Pro, load average about 30 from other
sessions, master's release baseline against this): the merged smoke pass takes up to 0.2 ms more in a close blast's
first second (`muzzle-deck`, the two looks at the environment a fragment) and 0.2–0.4 ms less from about four seconds
on, the cloud being smaller; whole frames move by −0.3 to +0.15 ms.

### The flash's colour on the smoke

The flashes light the gas they burn in (`EffectsFrame::flashes`, the four brightest, their power on smoke
`FLASH_ON_SMOKE` × `EffectLighting::flash_smoke`) in their own colour: the four's colours weighed by their power
(`EffectsFrame::flash_color`, the lighting uniform's `flash_color`), the flash's 2,500 K `#ffa64a` reddening to the
fireball's glow `#ff8a3c` as the flash fades (`FlashLight::color`), as their light on the hulls and the sea. The gas
batches the flashes light (`FLASH_LIT` in `layer.rs`: the combat gas, the soot, the funnels' smoke) take it in
`gas_sprite.wgsl` (`flash_light_color`); the volume trial's bodies and the strike debris too. Until 2026-10-21 they took
the flash's old `#ffd29a` (some 3,900 K), kept in the batches after the flash's light went to 2,500 K: at night the
salvo smoke lit by the next turret's flash came out cream beside an orange-lit deck (`native/docs/lighting/lights.md`, "Lamp glints and
salvo smoke"). The fires' batches keep their firelight colour (linear 1, 0.42, 0.12) on their own smoke.

## Blast waves

The owner asked for a shockwave from a gun firing and from a bomb bursting. Not a port: the web game drew none. A
turret's blast and every explosion in the air now send out a shock front (`src/shock.rs`: `ShockWaves`, the model and
its numbers; `src/shock_layer.rs`: `ShockLayer`; `shaders/shock.wgsl`).

**What sends one.** Each is so much TNT `W`, and everything about its front goes as `∛W` (its *scale*):

| What | Worth, kg of TNT | Notes |
| --- | --- | --- |
| A turret's salvo, calibre 10 cm and over (`CombatEffects::turret_blasts`) | 1,250 · calibre³ a barrel, the barrels together | Three tenths of the charge, left in the gas at the muzzle: 251 kg for an Iowa's turret, 5 kg for a five-inch twin. Its centre is 12 calibres ahead of the muzzles and it leans along the bore as the blast sheet does. The sheet's reach of 95 calibres is this charge's scaled distance 8.8 |
| A shell's or a bomb's burst on the plating, in the upperworks or in the open | Its filling (`explosiveKg`), else 1,000 · calibre³, else (blast radius / 3)³ | An aircraft striking land is the last: 64 kg |
| A burst inside the hull | 0.15 of it out of the round's hole, 0.35 up through the deck | Scaled by how near it burst, as the jet and the eruption are; nothing from deep inside |
| A burst at the sea's surface (the water lane's column) | Half of it | |
| A shell striking the land | Half an HE round's filling, a quarter of another's | The earth takes the rest |
| A magazine | 8,000 · (hull length / 240 m)³ | 880 kg for a destroyer's |
| Torpedoes and depth charges | None | They go off under water |

**The front** (`shock.rs`). Its overpressure at scaled distance Z = r / ∛W is Mills's fit to the free-air data (31 kPa
at Z = 5, 11 at 10, 5 at 20), its speed the sound's times √(1 + 6 Δp / 7 p₀): Mach 4 at Z = 1, 1.12 at Z = 5, 1.02 at
Z = 20. It is carried each frame at that speed, fades from Z = 10 and is dropped at Z = 28:

| Blast | Scale | At 0.1 s | Dropped at |
| --- | --- | --- | --- |
| Five-inch twin | 1.7 | 40 m | 48 m, 0.13 s |
| Eight-inch triple | 3.2 | 43 m | 88 m, 0.23 s |
| 38 cm shell's 55 kg | 3.8 | 44 m | 106 m, 0.28 s |
| 16-inch triple; a 1,000 lb bomb's 240 kg | 6.3 | 49 m | 177 m, 0.47 s |
| A battleship's magazine | 20 | 73 m | 560 m, 1.5 s |

**What is drawn of it**, three things:

- **The front in the air.** The air behind the front is denser (by 0.21 at Z = 5), and a ray that grazes the sphere
  crosses that step at a glancing angle and turns by √(2 δn) at each crossing: 0.011 rad at Z = 5, 0.007 at Z = 10. A
  ray through the middle does not turn. So the front bends the scene behind the sphere's outline and nothing
  inside it. In life that band is centimetres wide. The game draws the same turn, 1.5 times over (`SHOWN`), across
  the whole pressure pulse behind the front (0.6 to 1.9 scales long: 5 to 12 m for a heavy turret), as a swell that is
  nothing at the outline itself, and never moves the picture by more than 0.3 of the pulse's own width on the screen
  (more folds the band over and draws the sphere as a pane of glass). A scene standing just behind the crossing shows
  0.3 of the turn; the sky shows all of it. Against a plain sky there is nothing to bend and nothing shows, as in life.
- **Its foot on the sea.** From above, nothing grazes the sphere, and what is seen of a blast is its front running over
  the water. The air behind the front moves at 66 m/s at Z = 5, 26 at Z = 10 and 12 at Z = 20, for as long as the
  pulse lasts. The game draws the sea under the pulse paler (`RUFFLE`: up to 0.2 opaque, in streaks lying out from the
  blast, dying away over a pulse length), and reads the scene under the pulse 0.09 scales further out (`PUSH`), a
  ripple life does not show. The pale band comes in between scales 1 and 5: a five-inch gun's gust lasts thousandths
  of a second and raises none, a heavy turret's or a bomb's does. A hull's own plating takes a quarter of the ripple
  and no band.
- **The cloud behind it.** Behind the pulse the air is drawn below the surrounding pressure (by about 35 / Z kPa),
  cools, and where that takes it under its dew point mists for a moment. Air of relative humidity h needs
  1.23 · 20 (1 − h) kPa of it, so the cloud stands between Z = 3 and Z = 4 at 64 %, 6.5 at 78 %, 14 at 90 %, and never
  under 58 %. The humidity is the scene's surface air (`scene.air.column`, the column the aircraft's condensation
  reads): about 64 % under fair-weather cumulus, over 90 % under an overcast. The mist's extinction is 0.005 /m, so
  its opacity is its depth: a 15 cm shell's is nothing, a bomb's a faint ball for a tenth of a second, a magazine's a
  white dome over the ship. A gun's is 0.4 of an explosion's and shows only in damp air. The shell is drawn at least
  half its radius deep, clumped, and lit by the sun and the sky.

- **What it does to smoke.** The air behind the front carries the smoke it passes as far as the gust blows while
  the pulse lasts (`shove_m`: the gust's speed times the pulse's length over 340 m/s): a metre at Z = 5 from a heavy
  turret and four at Z = 2, three times that from a magazine, 0.3 m from a five-inch twin. `ShockWaves::shoves`
  gives each front's step and `ParticlePool::shove`, `FireBatch::shove` and `FunnelSmoke::shove` take it: gun
  smoke, strike smoke, flak, aircraft smoke and soot, the fires' smoke columns, the funnels' billows and trail. A
  puff is carried outward by the share of the pulse that passed over it in the step, so the whole passage adds up to
  the same at any frame rate, leaning along a gun's bore, never under the sea, and only if it was in the air before
  the blast (a blast's own puffs are thrown by its recipe). Flames, sparks, debris, spray and tracers are not moved.
  With `-shock` nothing is, and a recorded battle replays as before.

The pale band was first drawn up to 0.5 opaque, the ripple at 0.12 scales and the mist at 0.008 /m; the owner asked
for the front on the water and the cloud in damp air to be subtler the same day, and they are at 0.2, 0.09 and
0.005 /m.

**How it is drawn.** `ShockLayer` is a layer of its own, passed first of the layers that draw transparents (after the
ships): it moves the backdrop, so anything drawn before it would be drawn over. While a front is in the air it asks
for the backdrop's colour and depth (`scene.backdrop.color` had no reader before); with none it asks for nothing and
draws nothing. Each front is a quad about its sphere, or the whole view for a camera inside it. The fragment follows
its ray through the sphere against the backdrop's depth and writes the backdrop read at the moved place less the
backdrop where it is, so overlapping fronts add and the rain already drawn stays; a moved read that lands on
something nearer than the front is refused. In the motion pass it asks temporal AA for less history where the
picture moves. `CombatEffects` keeps at most 24 fronts (the one furthest through its run goes first). A camera under
the sea draws none.

**Seen with** `bun native/tools/effects-trial.ts --only shock,shock-side,shock-chase,shock-deck,shock-above,shock-light,shock-bomb,shock-damp,shock-magazine --stills`
(an Iowa's turret from another ship's bridge, from 80 m up, from astern, from her own deck and from overhead; a
destroyer's mount; a staged 1,000 lb bomb; the turret under an overcast; the staged magazine). `--slow 4` films four
frames a tick, a quarter of life's pace; `--trial=-shock` films the same moments without the fronts. `effects_probe`
lists the fronts in the air (`shocks`: centre, radius, scale, age). `shock.rs`'s tests hold the front's speed, reach
and fading, the cloud to damp air and large charges, the shove to the gust's reach at two frame rates, the arrival
time to the carried front and the list to its bound; `particles.rs` holds a pool's smoke to the shove; `tests/shock.rs` draws a bomb's front
over the renderer's test scene: no copy and no changed pixel without a front, no pixel moved outside its sphere, the
sea paler at its foot, and the same frame twice under temporal AA.

**Cost.** GPU busy at 1280 × 720 (`--cost 20`, Apple M5 Pro, Metal, 2026-10-12, load average 15 from other agents'
builds), against the same moments without the fronts (`--trial=-shock`), eight moments a scene: the transparent pass
0.06 to 0.14 ms more while a turret's front is in the air (`shock-chase`: 1.11–1.23 → 1.18–1.31 ms) and 0.01 to 0.12 ms
more under the magazine's, its cloud over much of the view (`shock-magazine`: 0.33–0.35 → 0.36–0.45 ms). Whole frames
(7.1–8.5 and 4.7–5.1 ms) differ by less than their noise. With no front in the air it costs nothing: no copy, no
draw. The backdrop's colour copy is made at the first front (16 MB at 1920 × 1080).

**Benchmark of what came with it** (the smoke's shove, the glare about flashes, the report's wait; 2026-10-12, Apple
M5 Pro, Metal, release builds, load average 3 to 16 from other agents). Each scene's eight moments at 1280 × 720,
`--cost 20`, against the commit before them (`bun native/tools/baseline.ts eace4907f`), which already drew the
fronts:

| Scene | Bloom pass, ms | Transparent pass, ms | Whole frame, ms |
| --- | --- | --- | --- |
| `shock-chase` | 0.11–0.12 → 0.11–0.12 | 1.13–2.05 → 1.14–2.06 | 7.28–8.22 → 7.13–8.33 |
| `shock-magazine` | 0.11–0.12 → 0.12 | 0.34–0.41 → 0.35–0.42 | 4.78–4.99 → 4.81–5.01 |
| `shock-deck` (the fireball over much of the view) | 0.12 → 0.12 | 0.37–0.59 → 0.37–0.59 | 5.72–5.82 → 5.76–5.96 |
| `shock-deck --hours 23` | 0.11–0.12 → 0.12 | 0.38–0.59 → 0.38–0.57 | 5.76–5.98 → 5.77–5.93 |

The glare is a loop of at most eight terms in the bloom's high pass and shows in none of them. On the CPU the combat
effects' update (`EffectsStats::combat_ms`, read on every stepped frame of the scene, two runs each) took a median
0.024 ms in `shock-chase` before, after and with `-shock`, and 0.041–0.050 → 0.043 ms in `shock-magazine`, 0.040–0.042
with `-shock`, its worst frame 0.10–0.12 ms in all three: the shove walks the smoke only while a front that moves it
5 cm or more is in the air. With no front in the air and no flash burning, none of the three costs anything. A
redrawn frame holds the fronts and carries nothing again.

**Open.**

- The front moves the backdrop only: smoke, spray, tracers and aircraft trails, drawn after it, are not bent, and the
  cloud lies under the fireball and smoke it should veil.
- A gun's front is a sphere whose strength leans along the bore; its speed does not.
- A theatre scene's re-cut drops the fronts in the air (they last half a second).
- There is no setting for it; `NAVAL_EFFECTS_TRIAL=-shock` switches it off.
- A front is heard and felt when it arrives (`../client.md`, "A report is felt and heard when it arrives"), on the
  wall clock: at a theatre speed above 1× the drawn front runs ahead of both.
- At night the mist and the pale band take the scene's own light where a flash lights it, and are otherwise as dark
  as the night's ambient: they are not lit by the flash's point light themselves.

## Heavy AA bursts

A heavy AA round's burst (`aircraft-fire` with an airburst and a target, `flak.rs`) was three charcoal puffs lasting
12–14 s, five brief gas fingers and 14 fragments: a grey blot with a few wisps, gone a quarter of a minute on. The
war's photographs of barrages (the 5-inch/38's over Yorktown at Midway and over the fleet off Okinawa, the 8.8 and
10.5 cm flak about the bombers over Germany) show something else: a cloud black from the first frames, a dozen metres
across for a 5-inch round, never a smooth ball but lumps heaped on one another, crisp at its edge, some with a tail
or streamers; it hangs where it burst and drifts with the wind, swelling and greying, so a barrage leaves the sky
dotted with the salvos before it and, low over the sea, a haze of them.

- **One cloud, marched.** A burst leaves one particle (`EffectParticle::flak`, `GAS_FLAK`), which the gas shader
  marches through its sphere (`gas_sprite.wgsl`'s `flak_soot`, 8 to 24 steps as it is a few pixels or a hundred and
  more across): a body with four lobes heaped on it, unions of spheres rounded together, billowed over by three
  sizes of the noise volume's Worley cells, and for two in three of these up to three streamers torn off; three in
  ten bursts are tailed instead (`FLAK_TAILED`), a head and a tapering chain of lobes ending in a thread, a third of
  them forked (a "Y"). All of it is drawn from the burst's seed, so no two are alike from any side. Fresh soot
  reflects some 5 % of the light, black faintly brown, lit by the sun on the billowed surface's way (less where its
  own lobes stand between) and by the sky above it. Its march stops at the scene's depth, and its quad is drawn at
  the near plane, as a tactical smoke screen's is: an aircraft inside a burst stays behind the soot in front of it.
- **The flash.** A 0.05 s white-yellow ball in a 0.12 s orange fringe (the fire pool's sprites), its light on the
  scene for 0.09 s (900·s²), and the filling burning orange inside the soot for 0.2 s, withdrawing inward as it goes
  out. While it burns the soot is still forming, its optical depth less by up to 0.7 (`1 − 0.7·heat²`), so a film's
  first frame of the burst is its fireball.
- **Size and life.** 15·s m across a third of a second on (`CLOUD`; s the calibre over 5 inches, 0.5–1.5), some eleven
  metres of black seen across; a third wider in its first five seconds as the burst's gas slows (`SWELL`), then 1.8 %
  of its first width a second as the air mixes into it (`SPREAD`): nearly twice as wide half a minute on, nearly three
  times a minute and a half on. Its optical depth falls as the square of its width, its soot spread through the
  volume, and not again as it disperses: that greys it, up to four times as pale, and softens its rim. It lasts
  80–110 s (`LINGER`), fading over the last half of its life only (`EffectParticle::fade`), and drifts with the wind
  at its height (`combat::wind_share`); a late frame draws it at its age.

**Merged, it hid behind itself.** In the merged smoke (look `merge`, the game's) a burst came out as a pale disc in a
dark rim from about half a second to seven seconds on, black again after. A merged fragment is weighed by what the
occlusion map says the gas in front of it lets through (`smoke_through`), and the map holds the burst's own soot:

1. Dense as it is, its soot's optical depth lay within a step of its front, and the map is drawn at a quarter of the
   resolution with its steps falling elsewhere: a fragment of its own a step behind that front lay behind all of it.
2. The map spreads all its gas's optical depth about one middle, a mean of nearness, and thin gas far nearer (her
   funnels' haze over the guns 60 m off) drew that middle in front of a burst 160 m or a kilometre off.

Either way its weight fell to the least (`MERGE_LEAST`) and the haze's colour, weighed by the haze's own opacity,
took its place under the burst's opacity. A burst's soot is now weighed from its sphere's near side, in front of its
place in the map, and by the map's levels only, not by its optical depth's spread (`merged_smoke`'s `levels`): smoke in
front of it still hides it, but the haze's share of one middle no longer does.

Seen in the game (Iowa quiet, seed 1941, the wind 9 m/s; before: master):

| Scene | Before | After |
| --- | --- | --- |
| `flak-near` (one 5-inch round 160 m off, 25° up) | a flash, then a grey puff with gas fingers greying by 5 s, gone by 15 s | a fireball, then a black lumpy cloud 11 m across, black still at 15 s, larger and greyer at 35 and 60 s |
| `flak` (two salvos 1.1 km off, 30° up) | small grey round puffs, gone by 15 s | black lumpy puffs, some tailed, hanging through 50 s |
| `flak-night` (the same under a full moon) | — | brief orange flashes, the soot dark against moonlit cloud |
| `flak-far` (64 rounds 3–5.5 km off over the horizon) | — | small dark irregular dots over the horizon, there still 75 s on |

**Seen with** `bun native/tools/effects-trial.ts --only flak-near,flak,flak-night,flak-far,flak-battle --stills`:
staged `aircraft-fire` events over a quiet battleship (`flakScene`), the camera 60 m off her beam keeping no station
on her as she steams on, square to the wind the probe gives (`effects_probe`'s `wind`) or (`flak-far`) looking out
from her beam, and moving with the wind at the bursts' height; `flak-battle`, a strike's real ones. **Checked by**
`tests/gpu.rs`' `an_aa_bursts_soot_seen_through_thin_smoke_stays_black_merged` (a burst 120 m off behind a thin pale
puff 40 m off and against a light one behind it: merged, its middle 166 levels against 193 beside it without the
weighing, 88 with it), `an_aa_bursts_soot_behind_a_dense_screen_is_mostly_hidden_merged` (the same behind a screen
that lets a fifth through: 152 against 188 beside it, where the screen alone shows 150; weighed by the densest gas
only, as a first fix had it, 129) and the unit tests in `flak.rs` and `combat.rs`.

**Open.** The tail is drawn at random, not along the round's way (the event carries no velocity). Any dense puff far
behind thin nearer gas can sink into the merged smoke's map as a burst did (a black shell burst behind the funnels'
haze); only the burst leaves the spread out. Behind smoke, the map's four levels weigh a burst by the deepest level
the smoke reaches, so a screen that lets 0.27 through passes it at 0.5 and it shows a little. Layered
(`--trial=-merge`), bursts draw after the other gas and over a screen in front of them. The light AA's (40 mm and
less) bursts are not these. GPU cost is measured on the development client only (`--cost`, 1280×720, noisy tiled
timestamps): the merged smoke pass took 0.5–1.5 ms with one burst near, 0.4–1.2 ms with two salvos 1.1 km off, and
up to 1.8 ms in `flak-battle` (149 bursts in 10 s). No release figure, no master baseline.
