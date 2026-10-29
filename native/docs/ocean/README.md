# Ocean (`native/crates/naval-ocean`)

| Question | Page |
| --- | --- |
| How the waves are made: the spectrum, the swell the hulls ride, gusts, tiling | [waves.md](waves.md) |
| Whitecaps, foam, gale streaks, rain, spindrift and the spray haze | [foam.md](foam.md) |
| How the sea is lit: the sky, reflections, glitter, light through the waves, shadows, lamps at night | [light.md](light.md) |
| The water mesh: the clipmap, the view's standoff, magnified views | [mesh.md](mesh.md) |
| Coasts, surf, the sea under time compression, regional water colour | [coasts.md](coasts.md) |
| A leaking hull's oil slick, a wreck's oil, burning oil | [oil.md](oil.md) |
| The wake field, trails, torpedo tracks, ring waves from what strikes the sea | [wakes.md](wakes.md) |
| Bow waves, Kelvin arms, the stern's waves, hull-side wash | [bow-waves.md](bow-waves.md) |
| What the sea costs on the GPU | [cost.md](cost.md) |
| Splashes, spray and the other effects above the water | [water-effects.md](../water-effects.md) |
| The sea around hulls, the wet band, the underwater view | [sea-sky.md](../sea-sky.md) |

The game's sea in wgpu: the seeded spectrum, the GPU FFT cascades with whitecap foam, the clipmap surface and its
physical shading, the sea the hulls ride drawn everywhere, weather at sea, oil slicks and the wakes. It began as a port
of the browser game's ocean, `src/game/ocean` (historical). Portable crate (passes
`cargo check -p naval-ocean --target wasm32-unknown-unknown`); the stills example and GPU tests are native-only.
Outputs go to `.build/native/ocean/`. The sea around hulls and the underwater view are [sea-sky.md](../sea-sky.md).
History (the web comparison, the port's file map, its first costs): [native-ocean-history.md](../../../docs/archive/native-ocean-history.md).

The seabed the sea shows through its shallows is the land layer's, read from the opaque scene. Since 2026-10-05 the land
shades it, more than 3 m down, as the seabed alone (sand darkening with depth, rock on the steep, no relief:
native/docs/terrain/surface.md, "The land's shading cost"), not through the land's whole surface; nothing in the sea changed.

## Where the code is

Paths are in `native/crates/naval-ocean`.

| Path | What |
| --- | --- |
| `src/ocean.rs` | `Ocean { parameters, time, field, surface }`, a `SceneLayer` ("Renderer core" below) |
| `src/spectrum.rs`, `src/sea_state.rs`, `src/calibration.rs` | The seeded spectrum (f64, PCG: the same seed draws the same sea), the realistic sea state, the wind-sea table and a scene's parameters ([waves.md](waves.md)) |
| `src/field.rs`, `shaders/fft.wgsl`, `resolve.wgsl`, `mips.wgsl` | The cascades as compute: evolve, Stockham FFT (16·16 on 256², 8·8·8 on 512²), resolve, foam spread, box mips; fields in `Rgba16Float` 2D arrays |
| `src/whitecaps.rs`, `src/groups.rs`, `src/gusts.rs` | Breaking weights and whitecap area, breaking groups, gusts ([foam.md](foam.md), [waves.md](waves.md)) |
| `src/hull_sea.rs`, `shaders/waves.wgsl` | The sea the hulls ride and its open-sea spread; `displacement`, `height_at` ([waves.md](waves.md), [sea-sky.md](../sea-sky.md)) |
| `src/geometry.rs`, `src/surface.rs`, `shaders/surface.wgsl` | The clipmap and the fan ([mesh.md](mesh.md)); the horizon ring to 95 % of the far plane in circles 3.5 km apart (15 km apart from 60 to 480 km, where a theatre view draws the land beyond its window: `native/docs/terrain/layer.md`, "The horizon ring") and a skirt on to the sea's horizon (the Earth's curve applied per vertex); the surface's shading ([light.md](light.md)) |
| `shaders/reflection.wgsl`, `water_body.wgsl`, `screen_reflection.wgsl`, `point_lights.wgsl` | The reflection lobe and glitter, the water body (Lee et al.), screen-space reflections, the scene's lamps ([light.md](light.md)) |
| `src/reflection_tiles.rs`, `shaders/reflection_tiles.wgsl` | The tiles a reflected ray can meet ([cost.md](cost.md)) |
| `src/foam_texture.rs`, `shaders/foam.wgsl` | The 256² foam texture; whitecaps, windrows, bubbles, gale streaks ([foam.md](foam.md)) |
| `src/spindrift.rs`, `src/rain_heights.rs`, `shaders/spindrift.wgsl`, `spray_haze.wgsl`, `rain.wgsl`, `rain_heights.wgsl` | Weather at sea ([foam.md](foam.md)) |
| `src/shore.rs`, `src/optics.rs`, `shaders/water_properties.wgsl` | The coast's fetch and surf; geographic water ([coasts.md](coasts.md)) |
| `src/oil.rs`, `shaders/oil.wgsl` | Oil slicks ([oil.md](oil.md)) |
| `src/ship_wake.rs`, `src/wake/`, `src/trail.rs`, `src/stamps.rs`, `src/torpedo_tracks.rs`, `shaders/wake_field.wgsl`, `stamps.wgsl`, `wake.wgsl` | The wakes ([wakes.md](wakes.md)) |
| `src/bow_waves.rs`, `src/hull_foam.rs`, `src/hull_profile.rs`, `src/hull_slice.rs` | Bow and stern waves, hull contact foam and shelter, waterline profiles sliced from the models ([bow-waves.md](bow-waves.md)) |
| `src/hull_heights.rs`, `src/sea_read.rs`, `src/underwater.rs`, `src/caustics.rs` and their shaders | The sea read beside the surface (the hulls' wet band, the underwater probe), the underwater view, caustics ([sea-sky.md](../sea-sky.md)) |
| `src/environment.rs`, `shaders/environment_fill.wgsl`, `src/backdrop.rs`, `shaders/backdrop.wgsl` | The sky's environment as the sea binds it ([light.md](light.md)); `StandInSky` for the stills |
| `shaders/noise.wgsl`, `mx_noise.wgsl`, `pcg.wgsl` | Value noise, MaterialX Perlin noise, PCG hashes |

The spectrum is built on the CPU: the whitecap thresholds, gates, tail roughness and height bounds need its statistics
there. It runs once per sea change (a few milliseconds in release); evolution, the transform, foam and mips are GPU
compute every frame. `CASCADES` is a shader def, so changing the tier rebuilds the surface's pipeline.

## Renderer core

The surface draws in `Phase::Water`; with `Ocean::read_scene_copies` it asks for `OPAQUE_COLOR | OPAQUE_DEPTH` and
binds `scene.opaque.color`/`depth` when their version changes (straight-through transmission over submerged hulls and
shallows, shoreline foam where the water's column thins, the screen-space reflections). `lit` and the glitter's beam
take `water_sun_shadow(position, OceanSurface::water_shadows)` (every sun map, the view maps since 2026-10-05) times
the clouds' shadow (`Ocean::set_cloud_shadow`, group 1 bindings 11–13, a disabled placeholder until set);
`draw_motion` writes `motion_response(1.0)` with a second, position-only draw of the clipmap. Spindrift draws in
`Phase::Transparent`.

`Ocean` implements `SceneLayer` ([render core](../render-core/README.md)): `prepare` advances `ocean.time` by the frame's
`dt` (a paused frame, `dt` 0, steps nothing), encodes the wave field's compute pass on the frame's encoder, encodes
the wakes' GPU half when `Ocean::enable_wake` made them (`Ocean::prepare_wake`), and writes the surface's clipmap snap
from `ctx.camera.position`. The surface shader imports `naval::camera::camera` at group 0; its own groups are 1
(surface uniforms, foam texture, scene copies, and the wakes at bindings 5–10 and 16: [wakes.md](wakes.md), "Bind
groups"), 2 (wave field) and 3 (sky environment). `Ocean::new_in` and `enable_wake_in` register the field's,
surface's, wake field's and stamp painters' pipelines on the renderer's registry from the library's copies of the
ocean's files (`shaders::FILES`), so they compile on its build threads and hot-reload; `Ocean::new` builds its own for
standalone use. `Ocean::set_scene` hands it copies of its own (the stills), keeping the wakes bound.

## Run it

```sh
cd native
cargo test -p naval-ocean                      # spectrum parity, GPU FFT vs CPU DFT, every tier draws, wakes, tiling
cargo run -p naval-ocean --example stills --release [-- --only bridge9,glint9] [--frames 120]
```

The example renders fixed poses through `naval_render::Renderer` (the ocean and a stand-in sky as its layers, its
display pass for the game's grade and AgX) to `.build/native/ocean/native/<scene>.png`, under a gradient sky now that
the web driver that baked the sky and wrote the web's frames is gone; `--frames N` writes `timings.json`. Scenes: `bridge`,
`chase`, `overhead` at noon with the sun behind and `glint` at 16:30 facing it, each at 9 m/s (the port's sea), 15
and 25 (`bridge9` … `glint25`), the waves frozen at t = 60. In the game, stage a view with `bun native/tools/shot.ts`
(`--water`, [harness.md](../harness.md), "Staging a shot") and time shader variants with
`bun native/tools/shader-ab.ts` ([cost.md](cost.md)).
