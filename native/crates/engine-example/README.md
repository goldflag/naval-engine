# engine-example

The game's engine crates on their own, with no simulation: a window with a fly camera over the sky (its atmosphere,
clouds and weather), the sea, a coast cut from a land release, and a few combat effects, each a
`naval_render::SceneLayer` on one renderer; and the round Earth from space, the theatre chart's globe. It is the
template another project copies: `src/world.rs` builds and feeds every layer, `src/coast.rs` cuts the coast,
`src/globe.rs` draws the globe, `src/main.rs` is the window, the still and the frame loop.

```sh
# From the repository root (the native workspace's Cargo with packaging's environment).
bun run cargo -- run -p engine-example --profile dev-client
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/still.png"
# The globe: the window starting on it, or its still (the whole Earth over the North Atlantic).
bun run cargo -- run -p engine-example --profile dev-client -- --globe
bun run cargo -- run -p engine-example --profile dev-client -- --still "$PWD/.build/engine-example/globe.png" --globe
```

(`bun run cargo` runs Cargo in `native/`, so a relative path lands under it.)

In the window: WASD to fly, Q and E down and up, a mouse button held to look, Shift eight times as fast, the wheel for
the speed. T and G move the clock an hour on and back, B steps the weather (clear, partly cloudy, overcast, fog, storm),
N and M make the sea calmer and rougher (the wind 3 m/s down and up), and F spawns a gun's blast, a fall of splashes, an
HE burst, a shell on the coast and a smoke screen ahead. O shows the round Earth from space and goes back (a drag turns
the globe under the pointer, the wheel zooms about it; `--globe` starts there). Escape quits; `--exit-after <seconds>`
closes it by itself. `--still <out.png>` renders one frame offscreen from the window's first view (700 m up, 2.5 km off
the coast) in fixed conditions (15:00, partly cloudy, the effects 1.2 s old), writes it and exits; `--size <w>x<h>` sets
either's size. The coast is the theatre's 96 km window of the synthetic release on its peninsula's south shore.

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
  built on `renderer.context()`.
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
- **The globe** (`naval-globe`, `src/globe.rs`; drawn instead of the scene, not as a layer): the world chart's bytes
  (`public/theatre/world/world-v1.bin` in the checkout, about 3.7 MB; or a host's own, `world::encode`) to
  `WorldStore::from_bytes`, which makes its meshes on a thread (under two seconds; the globe is a disc of sea until they
  are in); `pass::layout` and `pass::register_pipelines` on `renderer.context()` for the output's format; a `GlobePass`;
  a `Globe` (its centre `[lat, lon]`, pixels per metre, the output's size) and a `GlobeStyle` (`GlobeStyle::chart()`,
  the chart's own colours from the palette, or `GlobeStyle::parse` of a page's CSS); each frame `pass.draw_under(device,
  queue, &pipes, output, &GlobeDraw { globe, rect, clips, style }, meshes, device_scale)`. Its space is opaque, so it
  covers the output. `Globe::inverse` picks the point under a pixel and `Globe::grab` turns the globe so a point stays
  under the pointer. It draws no sun: no terminator, the day side and the night side alike.

Left out here, though the crates have them: ships, aircraft and wildlife (other crates), the land beyond the window
(`HorizonLayer`), buildings, roads and night lights, ground materials, shells in flight (`ShellLayer`), blast waves
(`ShockLayer`) and the underwater view (`naval_ocean::underwater`).
