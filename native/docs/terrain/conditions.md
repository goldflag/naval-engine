# Scene conditions

`native/crates/naval-conditions` resolves what a scene is drawn in: the map (the North Atlantic's open sea, the one
map left), the time of day, the weather, the sea and the fog. It turns them into the sky's scene, the sea's
parameters, the fog, the meshes' ambient and the land's style. It reads the game's own `assets/maps/environments.v1.json`
and `battle-conditions.v1.json`, embedded with `include_str!`, so it reads no file at run time and follows every edit
to them. Portable (checked for `wasm32-unknown-unknown`). It was `naval_terrain::catalog` until 2026-10-28.

- `src/lib.rs`: the maps, the time and weather presets, `BattleConditions`, `battle_environment`, `resolve_clouds`,
  `battle_precipitation`, the sun and moon, and `MapSelection` → `MapScene`.
- `src/style.rs`: the land's look a scene carries (`MapScene::style`): `LandStyle`, `Climate`, `SurfaceStyle`.
- `naval-ocean/src/calibration.rs`: `MAP_WATERS` (the North Atlantic's) and `map_water(id)`, tested against the JSON.

Who resolves a scene: the port (`naval-client` `port::build_scene`, `MapSelection::port()`), a task force's sky
(`port::task_force`), a theatre scene (`theatre/mod.rs`) and the developer harness (`debug::rpc_harness`).

## The map selection API

```rust
use naval_conditions::{BattleConditions, MapSelection};

// A scene: map, time of day, weather, optional conditions, the chart's placement.
let mut selection = MapSelection::battle("north-atlantic");     // or MapSelection::port()
selection.time_of_day = "dusk".into();                            // map | dawn | morning | noon | dusk | night
selection.weather = "overcast".into();                            // map | clear | partly-cloudy | overcast | fog | storm-clouds
selection.conditions = BattleConditions { time_hours: Some(7.5), cloud_cover: Some(55.0), ..Default::default() };
selection.spawn_distance = Some(8000.0);                          // None: the map's own
let scene = selection.resolve().expect("known map and presets");  // MapScene

game_sky.apply(&queue, &scene.sky);            // naval_sky::SkyScene (clouds, weather, sun turned by the chart's bearing)
ocean.parameters = scene.ocean;                // naval_ocean::OceanParameters (waves, colours, foam, the scene fog)
// scene.fog: the fog every material takes; scene.ambient: the hemisphere fill before the meshes' share;
// scene.wind: (m/s, chart degrees); scene.offset: world = chart + offset.
```

The port: `MapSelection::port().resolve()` gives the port's sky (`DEFAULT_SKY_SCENE`), its standing 9 m/s sea on
the North Atlantic's water, a fair day's haze (22 km visibility since 2026-10-03; it closed at 5.6 km before:
`naval_ocean::calibration::PORT_FOG`) and ambient 1.75. The port is open sea; its harbour backdrop was removed on 2026-10-04.

## The land's style and climate

`LandStyle` is what covers the ground where no photograph paints it: tropical, snow, volcanic, chalk or rock
(`terrain.wgsl`'s `STYLE_*`). `SurfaceStyle` is its palette in linear light and the chart's direction toward the
equator. `Climate` says where and when the land is drawn: the snowline, the thaw below it, the leaves out, a dry summer,
and the place, from which each tree's species is chosen. `Climate::AUTHORED` draws each style as it was authored for
the removed land maps. A theatre computes its climate from where and when its scene stands (`naval-client`
`theatre::land::climate`). How the surface draws by it: [surface.md](surface.md), "Seasons, climate and tiling"; the
trees: [trees.md](trees.md), "Species".
