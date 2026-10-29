# Lighting

| Question | Page |
| --- | --- |
| How do hulls, aircraft and smoke take the sun and sky; why is a view into the sun or under a storm lit as it is; what does the camera's exposure do? | [Daylight and the camera](daylight.md) |
| How dark is the night, how is it graded for play, and how does the eye adapt to darkness and to flashes? | [The night](night.md) |
| How do flashes, fires, searchlights, star shells and lightning light hulls, land and the sea; what does scorch draw? | [Point lights and scorch](lights.md) |
| The web port's gap list, its file-by-file port, side-by-side measurements and the slice's wiring | [History](../../../docs/archive/native-lighting-history.md) |

How the native scene is lit beyond the sky and the sea's own models: the sun, moon and sky on lit meshes, the
camera's exposure, the night's grade and the eye, the point lights every lit layer and the sea read, and scorch on
hulls. Fog and haze are [haze.md](../haze.md)'s; the sky's and the sea's light are [sky.md](../sky.md)'s and
[ocean](../ocean/README.md)'s; the effects' own light is [combat effects](../combat-effects/README.md)'.

## Where the code is

Under `native/crates/`:

| Path | Owns |
| --- | --- |
| `naval-render/src/lights.rs`, `naval-render/shaders/naval/lights.wgsl` (`naval::lights`) | `PointLight`, `PointLights` (at most 64, the brightest at the camera), the clustered index (`LightClusters`), the light maps (`light_shadow_maps`), `candela_scale`, `PointLight::lightning`, `SceneFog` |
| `naval-render/shaders/display_pass.wgsl`, `naval-render/src/post/` | The display grade (the night's lift) and the eye's adaptation |
| `naval-conditions/src/lib.rs` | `mesh_light_shares`: the sun's, moon's and fill's shares on lit meshes |
| `naval-ships/shaders/ship.wgsl`, `naval-ships/src/renderer.rs` | Hulls: `ShipLighting` (daylight, fill, fog), the point lights of the pixel's cell, the brightest casters through their maps |
| `naval-aircraft/shaders/aircraft.wgsl`, `naval-aircraft/shaders/sprites.wgsl`, `naval-aircraft/src/layer.rs` | The same for airframes, canopies, tracers and silhouettes |
| `naval-terrain/shaders/lighting.wgsl` | Point lights on land and trees |
| `naval-effects/src/combat.rs`, `fires.rs`, `illumination.rs` | The effects' lights (`CombatEffects::point_lights`: flashes, then every fire in view), the star shells' and searchlights' candela |
| `naval-ocean/shaders/screen_reflection.wgsl`, `naval-ocean/shaders/surface.wgsl` | Screen-space reflections of hulls and land in the sea; point lights on the sea |
| `naval-ships/src/scorch.rs`, `naval-ships/shaders/ship.wgsl` (`scorch_damage`) | Scorch on hulls |
