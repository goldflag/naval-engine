# A hull afloat (`native/crates/naval-afloat`)

A ship on the drawn sea for the engine, visual only: no weapons, no insides, nothing of the game's simulation. A host
describes her by the numbers, steers her, and gets back her pose on the sea (heave, pitch and roll) and the inputs of
everything a moving hull stirs: the wake and its foam, the bow waves, the bow spray and her funnels' smoke. Built for
the public engine's ships, whose models are any GLB (`naval-vessel` loads and draws them); started 2026-10-29 (branch
`goldflag/engine-afloat`).

## Where it lives

| Path | Holds |
| --- | --- |
| `src/hull.rs` | `Hull`: waterline length, beam, draft, and what is guessed without being given (displacement, freeboard, full speed, metacentric height); her natural periods; her proxy (`SimpleHull`) and its waterline profile |
| `src/sea.rs` | `SeaSurface` (what a hull reads of the sea); the combat sea's sines from the drawn sea's parameters (`swell`) and as the ocean draws them (`ocean_waves`) |
| `src/drawn.rs` | `DrawnSea`: the ocean's heights about each hull read back from the GPU, without waiting, and what they hold beyond the sines |
| `src/motion.rs` | `Floating`: the waterplane's points and the plane fitted to the sea there, the three oscillators, the fixed-step clock |
| `src/stir.rs` | `Stir`: her `WakeShip`, `SeaHull`, `EffectShip` and `ShipFrame`, refreshed with her pose |
| `src/lib.rs` | `Afloat`, the two behind one call a frame |
| `native/crates/engine-example/src/afloat.rs` | The ships afloat, fed to every layer: the stand-in (`--afloat <knots>`, a destroyer's numbers sailing a circle) and the ships from glTF files (`--ship`), each steered straight on or put about |
| `native/crates/engine-example/src/join.rs` | A `naval_vessel::Vessel` (a glTF ship's setup resolved) to a `Hull`, and her pose afloat to the model's `VesselPose` |

Its dependencies are the effect crates whose inputs it fills (`naval-effects`, `naval-ocean`, `naval-water-effects`)
and wgpu for the read-back; no simulation (`bun run cargo -- tree -e normal -p naval-afloat`). Portable.

## The hull

Ship space is the game's model frame: metres, x to starboard, y up from the design waterline, z aft (the bow toward
−z). Her pose's origin is her design waterline amidships (her waterplane's centre less `Hull::centre`). Everything
she stirs is in ship space from that origin's pose, her centre's offset in it: her proxy's waterline profile is moved
by `Hull::centre` as a model's slice is in the model's frame, so her bow waves and bow spray rise at the stem the wake's
generators and the sea about her put there; a host draws her proxy (`Stir::shape`, about her waterplane's centre) at
her pose moved by `Hull::centre` too.

`Afloat::new` (and `Floating::new`, `Stir::new`) refuses a hull `Hull::check` does not pass (`BadHull`): her length,
beam, draft and each number given must be positive and finite, her centre and funnels' rims finite, each funnel's
share of her smoke within 0–1. A zero length's pitch period is infinite, and the ocean's wake divides by it.

| Field | Without one |
| --- | --- |
| `length`, `beam`, `draft` | (required) |
| `displacement` (t) | block coefficient 0.55 of length × beam × draft in sea water; a given one is read as a block within 0.35–0.85 |
| `freeboard` (m, amidships) | 1.5 + 0.03 L, a warship's (`SimpleHull::warship`) |
| `full_speed` (m/s) | Froude number 0.5 on her length, at most 16 m/s (31 knots) |
| `metacentric_height` (m) | 0.07 of her beam, the game's catalog's (a Fletcher's 0.85 m, Bismarck's 2.52 m) |
| `centre` (ship x, z) | 0: the model's origin amidships |
| `funnels` | none: each a rim's middle in ship space, its width and length, its share of her smoke (`smoke`, 0 a dummy to 1; `Funnel::round` 1) |

Her natural periods (`Hull::periods`): heave from her waterplane's stiffness against her mass and as much water again
(waterplane coefficient (1 + 2 Cb) / 3); pitch from her longitudinal metacentric radius (3 Cw² / ((1 + Cw)(1 + 2 Cw))
of the rectangle's moment) against a gyradius of a quarter of her length; roll as the simulation's, √(g GM) over 0.4 of
her beam. A 110 m destroyer heaves in 4.8 s, pitches in 5.8 and rolls in 10; a 250 m battleship in 8, 9.6 and 18.

## Floating

Each of heave, pitch and roll is a damped oscillator driven toward the sea's level under her: the plane fitted by
least squares to the sea's heights at points over her waterplane, three across (on her centre line and at 0.7 of the
half-breadth either side) at each of 5–24 stations (three to the sea's shortest wave), weighted by her proxy's breadth
there. The fit averages a short sea away under a long hull. Damping acts on her motion relative to that level (strip
theory's relative motion), so a long swell carries her without lag: heave and pitch at 0.3 of critical, roll as the
simulation's `roll_decay` (0.08 at rest, 0.25 more per unit of Froude number, a quadratic term of 0.35 per rad/s).

The host gives her course each frame (`Course`: x, z, heading clockwise from −z, speed along her bow) and the clock the
sea is drawn at. She steps on her own clock in fixed 1/120 s steps, the course drawn straight between frames and on at
its speed past the last; the pose is the motion between the two steps about the frame's time. Her x, z and heading are
the host's exactly; nothing accumulates. A first frame, a clock that goes back or jumps more than 2 s settles her: her
motion of the last two of her longest periods along her course run back, so a hull that appears underway is already
working. A level that jumps faster than 4 m/s or 0.4 rad/s (a new wind) is a sea changed: her damping does not chase it.

## The sea she rides

The wave field (FFT cascades) is GPU-only. What the game's hulls ride is the combat sea's two sines, and the ocean
draws them about them: handed the sines and the hulls' footprints (`Ocean::set_hull_sea`, `hull_sea::hull_footprints`),
the surface replaces the field's long waves with the sines (everywhere on the realistic sea state, exactly so about each
hull) and keeps its shorter waves. `swell` makes the sines from the ocean's own `parameters.waves` as the simulation
makes them from the same wind (Hm0 / (4√0.29), four times the peak wavelength, the wind's direction); `ocean_waves` is
them as the ocean draws them, term for term `SeaWaves::height`. Without the coupling the drawn sea is the field alone,
and nothing on the CPU knows it.

Measured (`tests/drawn_sea.rs`, the High tier at 9 and 15 m/s, against the ocean's heights read back at the same
points): coupled, the drawn sea under a hull is the sines plus 0.25–0.49 m RMS of shorter waves at points; over her
waterplane those largely cancel, the fitted plane within 0.08–0.26 m of the sines' in heave and 0.4–2.8° in roll (a
battleship the least, a 30 m launch the most). The field alone is two to five times further off. Ridden, those short
waves move a battleship 0.1–0.3 m and a fifth of a degree, a destroyer 0.1–0.3 m and up to 1.2° of roll, a launch up to
0.4 m, 2.3° of pitch and 4° of roll, more than the sines alone do to her.

So she rides both (`DrawnSea`): the ocean already evaluates the drawn height on a patch about each coupled hull once a
frame for the ships' wet band (`hull_heights`, on with `Ocean::enable_hull_heights_in`: the surface's own `height_at`,
choppy displacement inverted, and the wakes'). `DrawnSea::read`, once a frame before the ocean's next prepare, copies
each followed hull's layer (found by its centre: the layers are the patches in view, packed) to a buffer and maps it
without waiting; `take` keeps what arrived, less the sines it was drawn on; `surface(id, &sea)` is the sines now plus
that residual, held for 0.5 s. A read arrives a frame or two late. Until one does, or for a hull with no patch
(out of view, beyond the ocean's 16 slots), she rides the sines alone. The read-back returns the drawn heights at her
waterplane's points within a millimetre.

## What she stirs

The inputs the game's client builds for its ships (`naval-client`'s `scene/effects.rs`), from her pose, speed and size
in place of a preset; each crate scales its own effect by them as it does for the game's ships:

- **Wake, foam, bow waves, stern wash** (`Stir::wake_ship` → `ShipWake::update`, the focus hull first): her pose,
  speed, length, beam, draft, full speeds and her proxy's waterline profile. The ocean scales the wake field's bow and
  stern generators, the trail's foam and slick, the analytic bow waves and the contact foam by her speed against her
  full speed and her size. The host enables the wakes (`Ocean::enable_wake_in`).
- **The sea about her** (`Stir::sea_hull` → `hull_footprints` → `Ocean::set_hull_sea`), with `ocean_waves(&sea)` at the
  clock she rides it on.
- **Bow spray** (`Stir::effect_ship` → `WaterEffects::update`'s `ships`, with `sea: Some((sea, time))` and the bow waves'
  `CrestShape`): the sheets off her bow's crests and what a plunge throws, from her profile and pose on the sea.
- **Funnel smoke** (`Stir::ship_frame` → `CombatEffects::update`'s `ships`): a `ShipEffectsProfile` with her funnels'
  rims and their shares of her smoke (`FunnelOutlet::smoke`: a funnel's billows as many and its smoke as thick as the
  root of its share), full speeds, draft, depth and waterplane; her pose, world velocity and speed each frame; full
  power. The smoke's density and colour follow her speed against full speed and how hard she works up.

## Using it

Per frame, before the renderer's frame: `drawn.read(device, queue, heights)`; `afloat.update(course,
&drawn.surface(id, &sea), time)`; couple the sea about her and tell `drawn.couple(&[(id, footprint)], &sea, time)` what
the ocean was handed (the next read is taken less those sines, so a change of wind between a frame and its read does not
leave two long seas' difference in it); `ShipWake::update` with her
`wake_ship`; the combat and water effects with her `ship_frame` and `effect_ship`; draw her model at `afloat.pose()`
(`naval_effects::ShipPose`, three's Euler YXZ with yaw −heading). Several hulls share one `DrawnSea`, one
`set_hull_sea` (all their footprints) and one `ShipWake::update` (the focus hull first); each couples by her own
footprint among those handed over (`hull_footprints` ranks them by distance from the camera). `engine-example/src/
afloat.rs` does all of it.

A glTF ship's model (`naval-vessel`) shares this ship space and pose: `engine-example/src/join.rs`'s `hull` makes her
`Hull` from her `Vessel` (her waterline's length, breadth and middle cut from her surfaces, her draft, her funnels'
mouths and smoke) and `pose` turns `afloat.pose()` into her `VesselPose` with nothing turned or negated; its tests hold
her drawn bow up where the sea is higher at it, her starboard side up where it lifts her, and her middle with the
crest (the engine-example's [README](../crates/engine-example/README.md#any-gltf-ship-afloat), step by step).

## Checks

- `bun run rust:test -- --native -p naval-afloat`: the motion (at rest at her draft in a calm, heave following a long
  swell, a long hull pitching less in short waves, settling once the sea goes at her roll period and decrement, the
  same motion at 20, 30, 60, 144 and uneven frame rates over five minutes within 1e-4 m and 1e-5 rad, a hull appearing
  underway already working) and the drawn sea (GPU: prints its measurements with `-- --nocapture`).
- `bun run cargo -- run -p engine-example --profile dev-client -- --still <out.png> --afloat <knots> [--wind <m/s>]`:
  the stand-in from off her port bow after 30 s (the window: `--afloat <knots>`, `,` and `.` for 5 knots less or more);
  `--ship <setup.json> --knots <knots>` for a glTF ship (`[` and `]` a knot, R puts her about).
- `bun run rust:test -- --native -p engine-example`: the join's conventions (a glTF ship's hull from her numbers, her
  model placed where her pose afloat places her, her bow, side and middle rising with the sea) and putting about.

## Open problems

- No heel in a turn: the host steers her heading and she stays upright through it (the simulation heels its ships
  from their lateral acceleration and turn lever).
- Her own wake is in the read-back: she rides her bow wave's crest and the trough aft of it, a little sinkage and trim
  at speed that no hydrodynamics set.
- The residual is held frozen between reads (a frame or two); the field's short waves move on in that time.
- One read-back copy per hull per frame (64 KB); a host following many hulls might read fewer, or reduce the patch on
  the GPU.
