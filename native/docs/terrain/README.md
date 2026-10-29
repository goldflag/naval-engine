# The land

The theatre's land, from the release's heights to the GPU. Each question has one page:

| Question | Page |
| --- | --- |
| How do the heights get from the sources to the screen? Which crate does what? Where is a coastline drawn? How far does a summit fall on the way? | [pipeline.md](pipeline.md) |
| Where are a scene's time of day, weather, sea, fog and land style resolved? | [conditions.md](conditions.md) |
| How does a host make and drive `TerrainLayer`? Its settings, field, level of detail, the land beyond the window, its GPU cost | [layer.md](layer.md) |
| Where do the satellite colour and land cover paint? Roads and rivers on the ground, snow, glacier ice and seasons, how dry the country is, tiling, the shore's wet band, the surface shader's cost | [surface.md](surface.md) |
| The ground close up: ground materials, beaches, cliffs' rock and stepped walls, the climate and rock maps, fine relief near the camera | [ground.md](ground.md) |
| Where are trees placed? Which species? Impostors, near trees, grown trees, ground cover, grass, the trees' shadows | [trees.md](trees.md) |
| Buildings: footprints, types, regions, roofs, parcels, facades, rooms, infill, the far level | [buildings.md](buildings.md) |
| The theatre as it stood in the scenario's year: built-up history, period styling, fewer buildings, the period's ground and roads | [period.md](period.md) |
| Ports, container stacks, harbour boats, bridges, airfields, power lines, generated wartime bases | [infra.md](infra.md) |
| The land's lights at night | [night.md](night.md) |
| The land in the sun's shadow maps, its own baked shadow, the open sky | [shadows.md](shadows.md) |

History (the web port, the battle maps' land, the lanes that built each part, superseded measurements, stills and
reviews) is in [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).

## Where the code is

| Path | Owns | Page |
| --- | --- | --- |
| `native/crates/naval-terrain/src/layer.rs`, `land.rs` | `TerrainLayer` (a `naval_render::SceneLayer`); `PreparedLand`, the CPU half a host builds off the render thread | [layer.md](layer.md) |
| `src/ground/` | `heightfield` (`Heightfield`), `release` (a land release's window cut without the simulation), `field` (gradients, coast distance, flow, sun visibility), `lod` (CDLOD), `surface_maps`, `surface_textures`, `textures`, `ground_materials`, `relief`, `features`, `country` | [layer.md](layer.md), [pipeline.md](pipeline.md), [surface.md](surface.md), [ground.md](ground.md), [period.md](period.md) |
| `src/trees/` | `species`, `impostors`, `tree_meshes`, `near_trees`, `tree_growth`, `canopy`, `crown`, `grass`, `tree_shadows`, `forest_shade` | [trees.md](trees.md) |
| `src/buildings/` | the buildings layer, meshes, facades, palettes, regions, parcels, infill, the far level, the period's thinning and history | [buildings.md](buildings.md), [period.md](period.md) |
| `src/infra/` | container stacks, marinas, bridges, airfields, power lines, generated bases (`bases/`) | [infra.md](infra.md) |
| `src/night.rs` | lamps and the night's rules | [night.md](night.md) |
| `src/land_shadows.rs`, `sunlight.rs`, `open_sky.rs` | the land in the sun's maps, its own shadow as the sun moves, the open sky | [shadows.md](shadows.md) |
| `src/horizon.rs` | the horizon ring | [layer.md](layer.md) |
| `src/earth/` | the Köppen-Geiger climate and GLiM rock maps, compiled in (`koppen`, `lithology`); baked by `scripts/theatre/bake-climate-and-rock.py` | [ground.md](ground.md) |
| `src/noise.rs`, `bc7.rs`, `dfg.rs`, `review.rs` | the 512² noise tile, BC7 on the CPU, the DFG table, the review cameras | |
| `shaders/` | flat: `terrain.wgsl`, `bindings.wgsl`, `lighting.wgsl`, `surface.wgsl`, `materials.wgsl`, the trees', buildings', infrastructure's and night's shaders | each page |
| `native/crates/naval-client/src/theatre/land/` | the host's half: the window, the release's squares, the streamed sources, the pages and meshes the layers draw | [pipeline.md](pipeline.md) |
| `native/crates/naval-conditions` | the scene's conditions, the land's style and climate | [conditions.md](conditions.md) |
| `naval-region-bake`, `naval-land`, `crates/naval-land-format`, `naval-osm`, `naval-overture` | the bake, the store, the formats, the OpenStreetMap and Overture readers | [pipeline.md](pipeline.md) |

naval-terrain is portable: `cargo check -p naval-terrain --target wasm32-unknown-unknown` passes; the stills example
and the GPU tests are native-only. Outputs go to `.build/native/terrain/`. Land draws in the game only in a theatre's
window; a staged battle is open sea. The crate's tests and stills draw a synthetic land (`tests/support.rs`: a 24 km
square at 40 m with cliffs, a ravine, a wooded scarp, downs, a hill and lowland) in each land style.

## Running it

```sh
cd native
cargo test -p naval-terrain                      # web parity (pins), CPU tests, GPU draws on the synthetic land, the shadow copy
cargo run -p naval-terrain --example stills --release [-- --styles snow --presets aerial --debug sky --quality medium]
cargo check -p naval-terrain --target wasm32-unknown-unknown
UPDATE_FIXTURES=1 cargo test -p naval-terrain --test all -- terrain_replay::   # re-record tests/fixtures/terrain.json after a deliberate change
```
