# engine-example

The game's engine crates on their own, with no simulation: a window with a fly camera over the sky (its atmosphere,
clouds and weather), the sea, a coast cut from a land release, a few combat effects and, with `--ship`, ships from any
glTF file afloat with their wakes and smoke, each a `naval_render::SceneLayer` on one renderer; and the round Earth
from space, the theatre chart's globe. It is the template another project copies: `src/world.rs` builds and feeds
every layer, `src/coast.rs` cuts the coast, `src/ship.rs` loads and draws the ships' models, `src/join.rs` turns a
model's numbers into a hull afloat and her pose afloat back into the model's, `src/afloat.rs` sails the ships (and a
stand-in, `--afloat`), `src/globe.rs` draws the globe, `src/main.rs` is the window, the still and the frame loop.

```sh
# From the repository root (the native workspace's Cargo with packaging's environment).
bun run cargo -- run -p engine-example --profile dev-client
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/still.png"
# The globe: the window starting on it, or its still (the whole Earth over the North Atlantic).
bun run cargo -- run -p engine-example --profile dev-client -- --globe
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/globe.png" --globe
# A ship from a glTF file and its setup (naval-vessel) afloat at 15 knots, in the window; two abreast at 28 knots; a
# still of her at 5 knots in a rough sea; one at night.
bun run cargo -- run -p engine-example --profile dev-client -- --ship "$PWD/ships/my-ship.vessel.json"
bun run cargo -- run -p engine-example --profile dev-client -- --ship "$PWD/ships/my-ship.vessel.json" --ship "$PWD/ships/other.vessel.json" --knots 28
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/ship.png" --ship "$PWD/ships/my-ship.vessel.json" --knots 5 --wind 15
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/night.png" --ship "$PWD/ships/my-ship.vessel.json" --hour 22
# A stand-in ship (a destroyer's numbers, no model) sailing a circle at 15 knots, in the window or a still of her in a
# 15 m/s wind.
bun run cargo -- run -p engine-example --profile dev-client -- --afloat 15
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/afloat.png" --afloat 15 --wind 15
```

(`bun run cargo` runs Cargo in `native/`, so a relative path lands under it.)

In the window: WASD to fly, Q and E down and up, a mouse button held to look, Shift eight times as fast, the wheel for
the speed. T and G move the clock an hour on and back, B steps the weather (clear, partly cloudy, overcast, fog, storm),
N and M make the sea calmer and rougher (the wind 3 m/s down and up), and F spawns a gun's blast, a fall of splashes, an
HE burst, a shell on the coast and a smoke screen ahead. O shows the round Earth from space and goes back (a drag turns
the globe under the pointer, the wheel zooms about it; `--globe` starts there). Escape quits; `--exit-after <seconds>`
closes it by itself. `--still <out.png>` renders one frame offscreen from the window's first view (700 m up, 2.5 km off
the coast) in fixed conditions (15:00, partly cloudy, the effects 1.2 s old), writes it and exits; `--size <w>x<h>` sets
either's size, `--hour <h>` the hour both start at. The coast is the theatre's 96 km window of the synthetic release on
its peninsula's south shore.

`--ship <setup.json>` reads a ship's setup file and the glTF or GLB it names (beside it) and puts her to sea 2.3 km off
the coast, heading 250° along it at `--knots` (15 without it), sailing straight on. Her hull is her model's numbers
(`src/join.rs`): she rides the sea as drawn, trails her wake and foam, pushes her bow waves, throws her bow's spray and
smokes from her funnels, each scaled with her speed and size ([naval-afloat](../../docs/afloat.md)). Each further
`--ship` sails abreast of the first to starboard, a length and a half of the longest apart. `naval-vessel inspect
<model.glb> --write` writes a starter setup beside a model ([the vessel page](../../docs/vessel.md): what it draws, what
it refuses, the setup's fields).

`--afloat <knots>` puts a stand-in ship to sea, a model's numbers without the model: a fleet destroyer's (114 m by 12,
4 m draft, 2,500 t, two funnels) sailing a 600 m circle to starboard before the first view, her proxy hull drawn grey.

With ships afloat the window starts on the chase camera, off the first's port bow looking aft along her side at
distances of her length; a fly key or a look leaves it where it is, C takes it back, Tab follows the next ship. `[`
and `]` take the ship followed a knot slower and faster, `,` and `.` five (0–36, in the title), and R puts her about:
half a turn to starboard on a circle of 2.5 of her lengths (with no way on she does not turn). With `--still`, the frame
is the chase camera's after 30 s under way at 30 frames a second (about 30 s to render; her wake then streams 230 m
astern at 15 knots), the combat effects not spawned; `--wind <m/s>` sets the still's wind (9 m/s a moderate sea, 15 a
rough one). Each ship put to sea prints her hull's numbers (given and guessed) and natural periods, and a still prints
each ship's heave, pitch and roll every 5 s with their root mean square over those 5 s.

## The crates

| Crate | What it draws or gives | Path in a checkout |
| --- | --- | --- |
| `naval-render` | The renderer: passes, HDR and reversed-depth targets, cameras, shadows, bloom, the eye, the display transform; the `SceneLayer` trait | `native/crates/naval-render` |
| `naval-sky` | The atmosphere, the sun, moon and stars, and the environment lighting every other layer reads | `native/crates/naval-sky` |
| `naval-clouds` | The clouds, rain, lightning and the clouds' shadows: `GameSky`, the sky with its weather | `native/crates/naval-clouds` |
| `naval-conditions` | The conditions: a map, an hour, a weather and a wind resolved into the sky's scene, the sea's parameters, the fog and the land's style (`MapSelection` → `MapScene`) | `native/crates/naval-conditions` |
| `naval-ocean` | The sea: spectrum, FFT, foam, the clipmap, wakes, the coast's shelter | `native/crates/naval-ocean` |
| `naval-terrain` | The land and its trees from a heightfield; `naval_terrain::release` cuts one from a land release | `native/crates/naval-terrain` |
| `naval-effects` | Gas, fire and smoke (blasts, bursts, strikes, smoke screens), shells, blast waves | `native/crates/naval-effects` |
| `naval-water-effects` | Splashes, the spray of gun blasts and bows | `native/crates/naval-water-effects` |
| `naval-vessel` | A ship from any glTF 2.0 file, lit by the sky (sun, environment, shadows, haze, night, lamps), placed by a setup file; `naval-vessel inspect` | `native/crates/naval-vessel` |
| `naval-afloat` | A hull on the drawn sea: heave, pitch and roll from the sea under her, and the inputs of her wake, bow waves, bow spray and funnel smoke | `native/crates/naval-afloat` |
| `naval-globe` | The round Earth seen from space: an orthographic, north-up globe, its picking and its grab, and the world chart on it (Natural Earth's land, coasts, reefs and depth bands) | `native/crates/naval-globe` |
| `naval-land-format` | Land releases' formats, and the `synthetic` release (a coastal world made in code) | `crates/naval-land-format` |
| `native-offscreen` | A headless device and PNG read-back, for stills (optional) | `native/crates/native-offscreen` |

They pull in `naval-sky-types`, `naval-cloud-field` and `naval-earth` (root `crates/`) and `native-unify` by themselves,
and none of them reaches `naval-sim`, `naval-protocol` or the game's adapters (`naval-clouds-sim`, `naval-effects-sim`,
which turn the simulation's packets into these crates' inputs). Check with
`bun run cargo -- tree -e normal -p engine-example | grep -E "naval-(sim|protocol|clouds-sim|effects-sim) "` (none).

### In another project

The checkout is the dependency: path dependencies on a local clone (the repository's pack is about 40 GiB, too large
for a git dependency). In the other project's `Cargo.toml`, with `<checkout>` the clone's root:

```toml
[dependencies]
naval-render = { path = "<checkout>/native/crates/naval-render" }
naval-sky = { path = "<checkout>/native/crates/naval-sky" }
naval-clouds = { path = "<checkout>/native/crates/naval-clouds" }
naval-conditions = { path = "<checkout>/native/crates/naval-conditions" }
naval-ocean = { path = "<checkout>/native/crates/naval-ocean" }
naval-terrain = { path = "<checkout>/native/crates/naval-terrain" }
naval-effects = { path = "<checkout>/native/crates/naval-effects" }
naval-water-effects = { path = "<checkout>/native/crates/naval-water-effects" }
naval-globe = { path = "<checkout>/native/crates/naval-globe" }
naval-vessel = { path = "<checkout>/native/crates/naval-vessel" }
naval-afloat = { path = "<checkout>/native/crates/naval-afloat" }
naval-land-format = { path = "<checkout>/crates/naval-land-format", features = ["synthetic"] }
native-offscreen = { path = "<checkout>/native/crates/native-offscreen" }
# The same majors as the crates' (`<checkout>/native/Cargo.toml`, [workspace.dependencies]): their types cross.
wgpu = "30"
winit = "0.30.13"
glam = "0.33"
pollster = "1"
anyhow = "1"

# Windows only: the crates are built and checked against a patched wgpu-hal (a larger DX12 compiled-shader cache,
# `<checkout>/native/vendor/README.md`). A [patch] applies only from the root of the workspace being built, so a
# Windows project copies it into its own root manifest; elsewhere the patch changes nothing and may be left out.
[patch.crates-io]
wgpu-hal = { path = "<checkout>/native/vendor/wgpu-hal" }
```

- **Workspace inheritance resolves in the checkout.** The crates' manifests say `version.workspace = true`,
  `wgpu.workspace = true` and the like; Cargo reads those from the workspace each crate belongs to, the checkout's
  `native/Cargo.toml` (and the root `Cargo.toml` for `crates/naval-land-format`), not from the other project. Keep
  the checkout whole; copy nothing from it. Its `[profile]` and `[patch]` sections do not carry over: only the other
  project's own apply (hence the patch above, and an `opt-level` of at least 1 for the dev profile, or the sea's and
  the clouds' CPU work is slow).
- **`native-unify`** comes along with every crate: no code, only the union of the features the game's builds ask of
  their shared dependencies (generated by `scripts/build/native-features.ts`), so those dependencies build with a few
  more features than a project would ask for on its own.
- The toolchain is the checkout's (`rust-toolchain.toml`, Rust 1.98.1, edition 2024).
- Every crate is portable (no file reads); the host hands it bytes. Their shaders are compiled in and need no files at
  run time. `naval-globe` reads no file either, but is native only: its `WorldStore` makes the world chart's meshes on
  a thread of its own (a wasm32 host would call `world::meshes` itself).

## What each layer needs from its host

`src/world.rs` does all of this, in this order. The world is Y up, right-handed, metres, x east and z south (north is
−Z); the sea's mean surface is y = 0; depth is reversed. The renderer draws the world curving over the Earth
(`Camera::curvature`).

- **The device** (`naval-render`): a wgpu device with `naval_effects::trial::limits(&adapter)` (the effects bind more
  sampled textures than wgpu's default) and `TEXTURE_COMPRESSION_BC` where the adapter has it (the trees' impostors
  then draw from BC7). `Renderer::new(device, queue, width, height, format)` with the output's format (sRGB: the
  display transform writes display-referred colour). Bloom, the eye's adaptation, its sky mean and the shadows' quality
  are the renderer's settings (`set_bloom`, `set_eye_adaptation`, `set_sky_mean`, `set_shadow_settings`). Each layer is
  built on `renderer.context()`. Each frame the eye follows the sky as the game's client has it (`World::set_eye`):
  `set_sun_elevation`, `set_night_lift`, `set_night_light` and `set_night_share`; without the last the bloom takes a
  lamp's whole light after dark and a ship's masthead light veils her.
- **The camera and the frame**: each frame a `naval_render::Camera` (eye, target, vertical field of view, near, far,
  `camera::EARTH_CURVATURE`), a `FrameTime` (seconds since the start, the frame's dt, its index) and the output's view;
  `renderer.set_sun(Sun::new(light.direction, centre))` with the sky's light (`GameSky::light`) and where the sun's
  shadow maps centre. Then `renderer.render(input, &mut layers, output)` with the layers in draw order: sky, sky mean,
  land, sea, water effects, combat effects.
- **The conditions** (`naval-conditions`): a `MapSelection` (map `north-atlantic`, a weather preset, and
  `BattleConditions`: `time_hours`, `wind_speed`, cloud cover and the rest) resolves to a `MapScene`; its `sky` goes to
  `GameSky::apply`, its `ocean` to `Ocean::parameters`, its `fog` to the land, `SceneFog::from(scene.ocean.fog)` to the
  effects, `ocean.fog.end` to `GameSky::set_visibility`, `ambient` to the land's and the effects' light, `wind`
  (m/s, degrees from +X toward +Z) to the effects, `style` to the land. Change them at any time and hand them on
  again (`World::set_conditions`). The maps and presets are the game's JSON, compiled in.
- **The sky and clouds** (`naval-sky`, `naval-clouds`): `GameSky::new_in` and a `SkyMeanLayer` after it (the eye's
  adaptation meters the sky). The sky runs its own clock from `FrameTime::seconds`. **Placement**: with
  `GameSky::placement` left `None` the clouds lie flat about the camera from the weather's layers; a host with a weather
  field of its own lays them by implementing `naval_clouds::laying::Placement` (the game's is `naval-clouds-sim`'s
  `SimClouds`, from the simulation's cloud anchors). Its environment map (`sky.sky.environment()`), clouds' shadow
  (`sky.clouds.shadow()`) and cover (`sky.clouds.cover()`) are what the other layers bind.
- **The sea** (`naval-ocean`): `Ocean::new_in` with the sky's environment, `read_scene_copies = true` (it reads the
  opaque scene: the land meets it in shallows and foam), the clouds' shadow, and optionally the coast it shelters
  behind (`Ocean::set_shore` with a `shore::ShoreMap::build` of the land's heights, `set_shore_offset`). Hand its
  underwater probe to the sky (`GameSky::set_underwater_probe`) so the clouds stop at the surface from below.
- **The land** (`naval-terrain`): a `Heightfield` (the host's grid of quanta) and the `MapScene` it is drawn in (world =
  chart + `scene.offset`), the sky's environment, the four detail photographs' bytes (`textures::DETAIL_PHOTOS`, in
  the checkout's `public/terrain/`; the crate reads no file), the clouds' shadow, and each frame a `TerrainLight` from
  the sky (its light, the scene's ambient, lightning's flash). Ground materials (`GroundMaterials`) are optional.
- **A coast from a land release** (`naval_terrain::release`): a `ReleaseWindow` (a place, samples, spacing, the band
  that settles under the sea; `ReleaseWindow::sea` is the theatre's 96 km window) names the squares it meets
  (`squares()`); the host reads those squares' final files (from a store such as `naval-land`'s, a download, or
  `naval_land_format::synthetic::square`, which makes the synthetic world's in code) into a `ReleaseLand`, and
  `heightfield(&land)` cuts the window as the theatre does, on the theatre's chart about the place. A real place's
  squares are baked into a store by `naval-land-bake`, without the game ([its README](../naval-land-bake/README.md):
  the command, and reading the store's squares into a `ReleaseLand`).
- **The combat effects** (`naval-effects`): `EffectsLayer::new`, its depth from the backdrop
  (`DepthSource::Backdrop`), the sky's environment, the clouds' cover and shadow, the water effects' cover. Each frame
  its light from the sky (`effects.lighting`: sun, illumination, daylight, flash, wind) and
  `effects.update(dt, now, &CameraView::from_render(..), &events, &ships)`. **Events** are `CombatEvent`s with strictly
  rising `sequence`s; the kinds the example sends: `shot` (a muzzle's blast and gun smoke: `position`, `shell` with
  its velocity and calibre), `burst` with `detonation: Some(true)` (an HE burst), `stopped` with no `ship_id` (a shell
  on the coast). A detonation of another kind is a magazine's; `ShipFrame`s place fires and funnel smoke on hulls
  (none here). Smoke screens are the host's to keep: `effects.tactical.smoke`, a `ScreenSmoke` each, its age and
  spread the host's. The rules default to the game's numbers (`EffectRules::default`).
- **The water effects** (`naval-water-effects`): `WaterEffects::new` with the sky's environment; each frame its light,
  the turrets that fired (`gun_blasts`, from `CombatEffects::turret_blasts`) and `update(&EffectsFrame { camera,
  events: &[WaterEvent], ships, sea, waves, crest, lighting })`; a `WaterEvent` of kind `Splash` throws a shell's
  column (`position` on the sea, the shell's `velocity` and `caliber_m`).
- **A ship** (`naval-vessel`, `src/ship.rs`): a `VesselModel` from the GLB's bytes and its `Setup` (the crate's
  `files::load` reads both from disk; on wasm32 the host hands `VesselModel::load` the bytes and a resolver for any
  external buffers and images), a `VesselLayer::new` with the sky's environment and a `VesselLighting`, each model
  through `add_model`, and a `VesselInstance` per ship (`VesselPose::transform()`: her place, heading, pitch, roll and
  heave). Each frame `lighting.follow(&sky.sky, ambient, flash)` and the scene's fog. It draws after the land and
  before the sea, into the sun's shadow maps too.
- **A ship afloat** (`naval-afloat`, `src/afloat.rs`): a `Hull` (waterline length, beam, draft; optionally
  displacement, freeboard, full speed, metacentric height, funnels' rims in ship space) in an `Afloat`, fed to the sea
  and the effects each frame; [any glTF ship afloat](#any-gltf-ship-afloat) below, step by step. `Afloat::new` refuses
  a hull with a non-positive or non-finite dimension (`Hull::check`).
- **The globe** (`naval-globe`, `src/globe.rs`; drawn instead of the scene, not as a layer): the world chart's bytes
  (`public/theatre/world/world-v1.bin` in the checkout, about 3.7 MB; or a host's own, `world::encode`) to
  `WorldStore::from_bytes`, which makes its meshes on a thread (under two seconds; the globe is a disc of sea until they
  are in); `pass::layout` and `pass::register_pipelines` on `renderer.context()` for the output's format; a `GlobePass`;
  a `Globe` (its centre `[lat, lon]`, pixels per metre, the output's size) and a `GlobeStyle` (`GlobeStyle::chart()`,
  the chart's own colours from the palette, or `GlobeStyle::parse` of a page's CSS); each frame `pass.draw_under(device,
  queue, &pipes, output, &GlobeDraw { globe, rect, clips, style }, meshes, device_scale)`. Its space is opaque, so it
  covers the output. `Globe::inverse` picks the point under a pixel and `Globe::grab` turns the globe so a point stays
  under the pointer. It draws no sun: no terminator, the day side and the night side alike.

Left out here, though the crates have them: the game's own ships (`naval-ships`, built on the simulation's
definitions; `--afloat` sails a stand-in's proxy), aircraft and wildlife (other crates), the land beyond the window
(`HorizonLayer`), buildings, roads and night lights, ground materials, shells in flight (`ShellLayer`), blast waves
(`ShockLayer`) and the underwater view (`naval_ocean::underwater`).

## Any glTF ship afloat

How `--ship` floats a model downloaded from anywhere (Sketchfab's GLBs included), in the order `src/world.rs` and
`src/afloat.rs` do it. Ship space, for the setup file and both crates: metres, x to starboard, y up from her waterline,
z aft (her bow toward −z), the origin on her waterline amidships (the middle of her bounds' length and breadth). Her
pose, for both: heading in radians clockwise from north (−z) seen from above, pitch bow up, roll starboard side up,
heave up, turned as three's Euler YXZ of −heading, pitch and roll.

1. **The setup file.** `naval-vessel inspect <model.glb> --write` writes `<model>.vessel.json` beside it from a guess at
   her axes. Set `length` to her real length overall (the model is scaled to it), check `up` and `forward`, and give
   her `waterline` (the model's own coordinate there) or `draft` (metres from it down to her lowest point): the
   waterline is where the sea meets her, the one number the floating needs most. Add her `funnels` (`at` each mouth's
   middle in ship space, `smoke` 0 for a dummy to 1, `radius` of its mouth) and her navigation `lights`
   ([the vessel page](../../docs/vessel.md#the-setup-file)). For example, a destroyer's:

   ```json
   {
     "version": 1,
     "model": "jervis.glb",
     "length": 108.66,
     "waterline": 0,
     "funnels": [{ "name": "funnel", "at": [0, 12.0, -2.8], "smoke": 1, "radius": 1.4 }],
     "lights": [{ "name": "steaming", "at": [0, 15.85, -8.8], "kind": "masthead" }]
   }
   ```

2. **Load and draw her** (`src/ship.rs`): `naval_vessel::files::load(setup)` (or `VesselModel::load` with the bytes),
   `VesselLayer::add_model`; the `Vessel` it resolves to holds her numbers.
3. **Her hull from her model** (`src/join.rs`, `join::hull(&vessel)`, the glue a host copies): the waterline's length,
   breadth and middle where her surfaces cross y = 0 (her bounds' where none does), her draft to her lowest point, and
   her funnels' mouths (round, of the setup's radius or 0.15 of her beam across) with their smoke. naval-afloat
   guesses the rest (displacement as a block of 0.55, freeboard, full speed from her length up to 31 knots, metacentric
   height 0.07 of her beam); set any of them on the `Hull` before `Afloat::new(id, hull)` if known.
4. **Before the first frame**: `Ocean::enable_wake_in` and `enable_hull_heights_in` (the wakes, and the sea's heights
   about hulls for the floating to read back), and a `naval_afloat::DrawnSea`.
5. **Each frame, before the renderer's** (`Fleet::advance`): `sea = naval_afloat::swell(&ocean.parameters.waves)`;
   `drawn.read(device, queue, heights)` once (last frame's heights about each hull, copied back without waiting, less
   the sines they were drawn on); for each ship `afloat.update(course, &drawn.surface(id, &sea), now)` with her x, z,
   heading and speed (the host steers her: `src/afloat.rs`'s `Helm` sails straight and puts about, or round a circle);
   then for all of them at once `Ocean::set_hull_sea(ocean_waves(&sea), now, &hull_footprints(&sea_hulls, eye, 16),
   eye)` with each `stir.sea_hull()`, `drawn.couple` with each ship's id and her footprint among those, and
   `ShipWake::update` with each `stir.wake_ship()`, the one the camera follows first.
6. **The effects**: each `stir.ship_frame()` in the combat effects' `ships` (her funnels' smoke) and each
   `stir.effect_ship()` in the water effects' (her bow's spray, with `sea: Some((sea, now))` and `crest` from
   `ShipWake::bow_waves`).
7. **Her model where she floats**: `join::pose(&afloat.pose())` is her `VesselPose` (the same place, heading, pitch and
   roll; her origin's height her heave), its `transform()` her `VesselInstance`'s.

Not there: no hull mask, so the sea is not cut out of her hull: a wave washes over a low deck and the sea shows inside
an open or low hull (a well deck, an open boat, a hull modelled without plating below the waterline). No heel in a
turn: she stays upright through it (the game heels its ships from their turn). Her hull's numbers are guessed from her
model, not her lines: a sonar dome or screws below her keel deepen her draft, and her proxy's warship lines shape her
bow waves and spray, not her own. Morph targets, skins and animations are not read (rest pose).
