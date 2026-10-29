# Combat effects (`native/crates/naval-effects`)

| Question | Page |
| --- | --- |
| How are the effects drawn: passes, draw order, the effects kit both effect crates share, the flipbook look, merged smoke, sun shadows? | [The renderer](renderer.md) |
| Where does a ship burn, how do her flames light the scene, how do a fire's and a funnel's smoke climb and drift? | [Fire and smoke](fire-and-smoke.md) |
| What is a turret's blast stage by stage; what do blast waves and heavy AA bursts look like? | [Gun blasts, blast waves and flak](gun-blasts.md) |
| How are shells in flight drawn; what does a strike leave (debris, marks, holes, dents, wrecks)? | [Shells and strikes](shells-and-strikes.md) |
| What does a ship's loss leave: the magazine's pall, venting, flotsam, burning oil? | [A ship's loss](ship-loss.md) |
| How are smoke screens, searchlights, star shells, the light in the air at night and far flashes drawn? | [Night light, smoke screens and lights at range](night-light.md) |
| Which event draws what; where the code is; how to run and film it | This page |
| The web port, its parity measurements and the lane's early plans | [History](../../../docs/archive/native-combat-effects-history.md) |

The game's combat gas, fire and smoke on wgpu, spawned from simulation events: muzzle blasts, shell strikes, HE
bursts and magazine detonations (gas puffs relighting a GPU-baked atlas, or frames of baked fluid simulations: the
flipbook look), ship fires (flames, rolling flame bodies, embers and buoyant smoke columns), funnel smoke and the
quarter-resolution smoke occlusion map that hides smoke and fire behind denser gun smoke. Water effects (splashes,
spray, foam) are [water-effects.md](../water-effects.md)'s. Portable crate (`cargo check -p naval-effects --target wasm32-unknown-unknown`
passes); the examples are native.

## Where the code is

Under `native/crates/naval-effects/`:

| Path | Owns |
| --- | --- |
| `src/combat.rs` | `CombatEffects`: event dispatch by sequence, event-seeded randomness, the flash lights, the effects' point lights |
| `src/layer.rs` | `EffectsLayer`, the `SceneLayer`: draw order, passes, bindings ([the renderer](renderer.md)) |
| `src/particles.rs`, `src/fire_batch.rs` | The particle pools and the fire batch (flame, smoke and gas modes) |
| `src/blasts.rs`, `src/flipbook_blasts.rs` | Turret blasts, strikes, HE bursts, the magazine and smoulder recipes |
| `src/fires.rs`, `src/flipbook_fires.rs`, `src/cookoffs.rs` | A ship's fires, their columns, light and cook-offs ([fire and smoke](fire-and-smoke.md)) |
| `src/funnel.rs` | Funnel smoke |
| `src/flipbook.rs`, `shaders/flipbook.wgsl` | The flipbook look's runtime |
| `src/gas_atlas.rs`, `shaders/gas_bake.wgsl`, `shaders/gas_sprite.wgsl`, `shaders/common.wgsl` | The gas atlas, the gas material and what every effect material shares |
| `src/shock.rs`, `src/shock_layer.rs`, `src/flak.rs` | Blast waves and heavy AA bursts ([gun blasts](gun-blasts.md)) |
| `src/shells.rs`, `src/shell_layer.rs`, `src/debris.rs`, `src/damage.rs` | Shells in flight, strike debris, the strike half of impact marks ([shells and strikes](shells-and-strikes.md)) |
| `src/loss.rs`, `src/flotsam.rs`, `src/sea_fires.rs` | A ship's loss ([a ship's loss](ship-loss.md)) |
| `src/tactical.rs`, `src/illumination.rs`, `src/air_light.rs` | Smoke screens, searchlights, star shells and the light in the air ([night light](night-light.md)) |
| `src/shadow.rs`, `src/sun_casters.rs` | Sun shadows |
| `src/ships.rs`, `src/events.rs` | `ShipEffectsProfile`, what the effects know of a ship, and `CombatEvent`, an event as they read it; the game builds both from the simulation in `naval-effects-sim` ([below](#the-simulations-side)) |
| `src/rules.rs`, `src/sea.rs` | `EffectRules`, the simulation's numbers the effects draw by; `SeaWaves`, the combat sea the flotsam and the water effects ride |
| `src/trial.rs` | The look switches (`NAVAL_EFFECTS_TRIAL`) |
| `src/fixture.rs` | Recorded battles and their replay |

## Run it

```sh
cd native
cargo test -p naval-effects                     # unit tests, the GPU tests
cargo test -p naval-effects-sim                 # the inputs from the simulation, the recorded battle's determinism
cargo check -p naval-effects --target wasm32-unknown-unknown
cargo run -p naval-effects-sim --example record_fixture --release [-- --seed 6 --from 252 --seconds 30]
# from the repository root: moments of smoke, fire and explosions filmed in the native client
bun native/tools/effects-trial.ts --help
```

`effects-trial.ts` films the same battle, tick and camera every run, with the game's look or with looks switched on or
off (`--trial`, [the flipbook look](renderer.md#the-flipbook-look)). The `stills` example (`naval-effects-sim`'s) replays
scripts the removed web driver recorded; it and the web half are in the
[history](../../../docs/archive/native-combat-effects-history.md#run-it-the-web-half).

## Event to effect

Each event is read once, by sequence (`CombatEffects::update`), and seeded by `sequence × 7919 + shell id`, so a
stream replays identically. Ship state drives the fires and funnels.

| Simulation event | When | Effect |
| --- | --- | --- |
| `shot` | per barrel; barrels of one ship, tick and calibre within 3 + 18·calibre m, parallel to 0.995, are one turret | Turret blast (see [stage by stage](gun-blasts.md#stage-by-stage)): a white-hot core and a six-sprite flame tongue per bore, the secondary flash (2 fireball lobes per bore below 17 cm, 6 above, stopped by drag at staggered reach, the heads swelling by the cube root of the barrels), burning as propellant gas into charcoal smoke, a bore wisp, a flare, the gun-smoke bank (1 + n or 4 + 2n billows, 20–26 s above 17 cm), burning grains; a flash light of 2800·size²·√n, which above 17 cm keeps the fireball's glow (0.2 of its power, falling with a time constant of 0.45·√flash s, reddening from the flash's `#ffa64a` (2,500 K) to the fires' `#ff8a3c`, on the hulls, the sea and the smoke alike). The turret's blast is handed on for the sheet of spray it tears off the sea (`CombatEffects::turret_blasts`, `../water-effects.md`, "The blast on the sea under the guns") |
| `penetration` | with a normal, on her outside | The entry: a white flash, 34 spall sparks off the plate's face, a pale puff of burnt paint, 6 flakes; flash light 150·scale. Remembered for the round's burst; a penetration of a bulkhead or deck inside her draws nothing |
| `stopped` | no ship's: the round struck the land (`CombatEvent::struck_land`) | A burst in the earth (`BlastEffects::land_strike`), sized by calibre over 15 inches: a flash (light 1,300·k² for HE, 600·k² else), an HE round's orange glow and 12 burning fragments, a column of 11 brown billows of earth thrown up in a fan (the fastest straight up, the slower spread wider) leaning from the shot and falling back (a 15-inch round's tops out about 43 m up, a 6-inch's about 13 m), a skirt of 7 pale dust puffs rolling out over the ground, a pall of 3 puffs hanging 16–22 s, and 28·k clods of earth and stones with a few of the shell's splinters, trailing dust. Its blast wave takes half an HE round's filling, a quarter of another's |
| `stopped` | with a normal, on her outside | The round shattering on armour: a hard flash and glow, 54 sparks in a wide fan, 3 brownish puffs of scale and paint, 9 hot splinters; flash light 260·scale; no fire |
| `ricochet` | with a normal, on her outside | A flash, 26 sparks and 5 hot splinters along the reflection, 2 grey puffs; flash light 90·scale |
| `contact` | with a shell | Remembered only: an HE round's contact, whose `burst` follows on the plate |
| `burst` | detonation, not over water | Where it went off decides (see [shell strikes and debris](shells-and-strikes.md#shell-strikes-and-debris)): on the plating, the HE fireball (7 black lobes, 4 smoke puffs, 20 sparks) blown off the plate with torn plating; inside the hull, a jet of flame and smoke out of the round's hole and, close under the weather deck, an eruption through it; elsewhere, the fireball with wreckage all round. Sized by `burst_scale` (blast radius / 4.5 × 0.55); flash light 1500·s² |
| any other | detonation (a magazine's `module`) | Magazine ([a ship's loss](ship-loss.md), "The burst and its column"): 3 white flashes, a 16-lobe fireball (3 simulated blasts in the game's look), 18 jets of 7 billows flung out in a fan, a 16-billow column standing at its height 5 to 10 s on over a foot along half her length, a head of 7 billows spreading each its own way, 5 secondary explosions 0.35–3 s later, 48 sticks of burning cordite, 98 pieces of wreckage on smoking arcs (torn plates and girders among them), the pall (12·s billows standing for ten minutes); a 1.6 s flash light of 900000·s²; her oil burns when she goes |
| `aircraft-fire` | with an airburst and a target | At the fuze time ([Heavy AA bursts](gun-blasts.md#heavy-aa-bursts)): a white-yellow ball in an orange fringe, a 0.09 s flash light of 900·s², and one cloud of black soot 15·s m across (s the calibre over 5 inches, 0.5–1.5), burning orange inside for 0.2 s, carried by the wind at its height, nearly twice as wide half a minute on and gone 80–110 s on. Its pool holds 1,536 bursts. |
| `splash`, `burst` over water, `contact` with hull damage, `torpedo-*`, `depth-charge-*`, `aircraft-crash` | | Water: nothing here (the water lane) |
| `sunk`, `module` without detonation, others | | Nothing: state changes, not fireballs |

| Ship state (`ShipFrame`, from the packet) | Effect |
| --- | --- |
| `room_fires`, `mount_fires`, `zone_fires` and the rooms' `openings`, with the definition's vents, mount frames and upperworks zones | Localized fires ([Where a ship burns](fire-and-smoke.md#where-a-ship-burns)); the 32 most relevant to the camera emit, and every one in view lights the scene, a near one from up to four points among its flames (`EffectsFrame::lights`, [Flames and their light](fire-and-smoke.md#flames-and-their-light)) |
| pose, velocity, speed, engine power, sunk, with the definition's funnels | Funnel smoke: puffs along the plume (billows and ribbon with `-funnel-puffs`) |
| pose, `flooded`, with the definition's boiler rooms, funnel bases and waterplane | Her funnels venting, the moment she is gone and her flotsam (`loss.rs`, [a ship's loss](ship-loss.md)) |
| the host's burning oil (`CombatEffects::sea_fires`, from naval-ocean's slicks) | Fire on the water, as cells of a frame of its own (`sea_fires.rs`) |

Aircraft striking land use `aircraft-crash` with `detonation: true`, the terrain's surface position and
normal, and a 12 m blast radius. `CombatEffects` emits the HE fireball and smoke at that position. The client's
water-event adapter excludes these detonations; aircraft crashes at sea retain their splash. The simulation owns
terrain avoidance and swept impacts ([air operations](../../../docs/air-operations.md#terrain-clearance-and-crashes)).

## Scene rebuilds

Ship fire and funnel smoke retain their CPU history when a theatre scene is rebuilt, translated
from the old scene's local origin into the new one. The map hides their drawing while their histories continue
to advance with the ships. A departing hull stops only its own funnel emitter; it no longer clears every plume.
Teleport rejection allows the distance a ship can travel during an accelerated frame.
`tests/continuity.rs` covers preserved fire smoke and rebased funnel samples; the native client's
`tests/effect_history.rs` covers leaving the Ship view for the map and returning.

## Recorded battle

`naval-effects-sim`'s `tests/fixtures/battle.json` (98 KB) is 30 s of a `naval-sim-host` staged battle, Bismarck against King George V under
bots (seed 6, from 252 s): Bismarck's turret Anton burning throughout, 86 shots, 18 penetrations, 25 HE bursts, 11
stopped rounds, 6 ricochets, 19 contacts. `examples/record_fixture.rs` writes it (events through
`naval_effects_sim::combat_event`, compacted; ships every 12 ticks with fires and, once a second, every mount's attitude).
`fixture::Replayer` steps it into the effects at 60 Hz (poses interpolated, fires held, events at their ticks).
`tests/replay.rs` checks two replays spawn identical particles (digest every second), every event reaches the effects
exactly once, and the stream fills the gas, flash and soot pools, leaves smouldering holes, burns the turret with
flames, bodies and a column, and smokes both funnels. `stills` renders it as the `battle` scene with a camera off
Bismarck's starboard quarter.

## The simulation's side

`naval-effects` builds without `naval-sim` and `naval-protocol` (it reaches them only through `naval-clouds` until the
clouds have their own inputs), so another program can draw the effects from its own events. What it reads is its own
types, in the frames the simulation gives them: `CombatEvent` (positions in the scene's frame, a strike on a hull in her
frame or her gunhouse's, `SurfaceImpactInfo`), `ShipEffectsProfile` (ship-local boxes, vents and funnel mouths, and her
hull's inside as a `HullShape` the host answers), `EffectRules` (`CombatEffects::rules`), each star shell's candle
(`tactical::Flare::fresh`, `illumination::StarShellCandle`), the swell (`flotsam::Swell`, a `SeaWaves`) and a re-centred
scene's shift as an `[x, z]` offset (`CombatEffects::take_ship_history_from`).

The game fills them in `native/crates/naval-effects-sim`, the one crate here that reads the simulation for the effects:

| Function | From |
| --- | --- |
| `combat_event` | A render packet's `RenderEvent` (its `DamageEvent` and geometry) |
| `scar_strike` | A hull's `HullScar`: the strike its event recorded, for a hull marked again after a restore |
| `ship_profile`, `funnel_outlets`, `bunker_oil_m3`, `SimHull` | The `ShipDefinition`: vents lifted out of her structures, fire zones (`naval_sim::fire_zones`) and their seep places (`fires::seep_places`), mounts, funnels, bunkers, and `naval_sim::hull::hull_contains` |
| `rules` | `SMOKE_EXTINCTION_PER_M`, `FIRE_GROWTH_PER_SECOND`, `SUPPRESSION_PER_SECOND` (its test holds `EffectRules::default()` to them) |
| `star_shell_candle` | `star_shell_candela` and `star_shell_reach_m` by the gun's calibre |
| `sea_waves` | The `SeaState` (its test holds `SeaWaves::height` to `SeaState::height`, bit for bit) |

The client's effects glue (`naval-client`'s `scene/effects.rs`), its close-aboard sound (`scene/audio.rs`, the funnels'
rims) and the aircraft lane's examples call it. The recorded battle's replay (`tests/replay.rs`) and the tools that need
a battle (`examples/record_fixture.rs`, `examples/stills.rs`) are its too.

## Drawn elsewhere

- Torpedo and depth-charge bodies (`CombatEffects.updateTorpedoes`): they read the packet's `torpedoes` and
  `depth_charges` directly. Shell bodies, glows, tracer streaks and trails were ported as `ShellLayer`
  (`src/shells.rs`, `src/shell_layer.rs`; see [aa-tracers.md](../aa-tracers.md)) and since 2026-10-10 draw what was seen at
  sea instead ([shells in flight](shells-and-strikes.md#shells-in-flight)).
- Burning-aircraft smoke trails (`updateAircraftSmoke`): the trail logic is ported in naval-aircraft
  (`SmokeTrails`, see `native/docs/aircraft.md`), which emits into this crate's pools through
  `CombatEffects::aircraft_pools()`: `(aircraft_smoke, fire)`. `aircraft_smoke` is the web's `aircraftSmoke` pool
  (768, gas), advanced in `update` and drawn in the gas batch between the combat smoke and the flak (so it occludes,
  as the web's does); a host emits into it after `update` and before rendering, as the web emits after its advance.
  `EffectCounts::aircraft_smoke` counts it. With nothing emitted it changes no frame, digest or test.
- Water: splash columns, spray, mist, billows (the smoke's puffs, `../../../docs/archive/native-water-effects-history.md`, "Splash clouds and plumes"), foam patches and the magazine's surge. The web's blast rings under heavy guns are the water lane's blast sheets (`../water-effects.md`, "The blast on the sea under the guns"), fed from `CombatEffects::turret_blasts`.
