# The land layer

`naval_terrain::TerrainLayer` is a `naval_render::SceneLayer` that draws a theatre window's land: the heights a host
hands it as a `Heightfield`, the surface over them, the trees and the land's shadows. Buildings and infrastructure are
their own layers (`BuildingsLayer`, `InfraLayer`). The land draws in `Phase::Opaque`.

## Using the layer

`scene` is the scene's `MapScene` ([conditions.md](conditions.md)); `game_sky` the sky and clouds.

```rust
use naval_terrain::{Heightfield, TerrainLayer, TerrainSettings, TerrainLight};

// The land: a `Heightfield` the host builds (a theatre window resampled from the land release; a test's synthetic land).
{
    let field: Heightfield = window_heights();
    let mut land = TerrainLayer::new(&mut renderer.context(), &field, &scene, TerrainSettings::default(),
        &game_sky.sky.environment().bind_group)?;
    land.set_detail_photos(&queue, photos)?;    // the four JPEGs in naval_terrain::textures::DETAIL_PHOTOS (neutral until then)
    land.set_cloud_shadow(Some(&game_sky.clouds.shadow().bind_group));
    // Each frame, before render:
    land.set_light(&TerrainLight { light: game_sky.light(), ambient: scene.ambient as f32, flash: game_sky.lightning().flash });
    // Layers: [sky, land, ships…, ocean]; the land draws in Phase::Opaque.
}

// Off the render thread (what a theatre scene's land does): the CPU half on a worker, then only the uploads.
let prepared = std::thread::spawn(move || -> Result<PreparedLand, String> {
    Ok(PreparedLand::new(&field).with_detail_photos(photos)?.with_sun(light_direction))   // `naval_terrain::PreparedLand`
});
let land = TerrainLayer::for_scene(&mut renderer.context(), prepared.join()??, &scene, settings, &environment)?;
```

- `TerrainSettings { quality: TerrainQuality::HIGH | MEDIUM, cast_shadows, cast_view_maps, debug }`: the web's `high`
  and `medium` terrain settings (20 m / 40 m vertices, trees on High only, the finest surface detail on High only),
  fixed when the layer is made (another setting is another layer, as the web rebuilds its land); `set_debug` takes the
  web's `TERRAIN_DEBUG` views (`TerrainDebug::by_name("sun")`; `albedo` is measurable: the albedo linear with no
  specular light, beside a strip of known albedos along the view's bottom edge, 0 to 0.6 in sixteen steps, drawn through
  the same display, so a still's pixels read back as albedo against the strip). `cast_shadows` (off by default; the client turns it on
  with the sun's maps) draws the land into the near and wide maps, and `cast_view_maps` into the view maps too;
  `set_cast_shadows(cast, view_maps)` changes them live, and `land_shadow_patches()` says how many patches each map
  took in the last frame (shadows.md, "The land's shadow"). The layer chooses each map's casters from the maps the renderer fitted
  (`PrepareContext::shadow_maps`): a host sets no focus.
- `set_environment` when the sky replaces its atlas (a quality change); `set_fog` when the fog changes live;
  `set_offset(offset)` moves the land (world = chart + offset) from the next frame, the patches chosen again (a theatre
  cruise scene's land follows where the scene draws the theatre); `set_clearing(Some((centre, radius, edge)))` clears a
  round of chart (no tree card stands there, thinning back to the cover over `edge`; the land mown grass with no
  canopy's shade or relief, the shore and sea as they are: a theatre airfield's apron, `terrain.clearing` in the
  shaders).
- The trees cast into the renderer's sun maps by themselves whenever those are on (Graphics → Shadows); a host does
  nothing for it. `set_tree_shadows(false)` leaves the forest's shade to stand in everywhere (A/B and timing);
  `tree_shadow_maps()` says which maps the trees cast into in the last frame (trees.md, "Shadows").
- `TerrainLayer::field()` exposes the prepared field: once the layer is built it keeps only the samples as stored
  (i16 quanta and their step, 11.5 MB on a 2401² map), the grids and the bounds; the heights in metres, the gradients
  and the attribute grid it uploaded (about 55 MB more) are let go (`PreparedField::release_uploaded`). Its
  `height(x − offset…)` is `Heightfield::height` on the same samples, the surface the simulation grounds ships on; a
  battle's camera rig reads its clearance from it (`TerrainLayer::shared_field`) rather than from a copy of the map.
  The GPU holds the heights as stored too (`R16Sint`, one texel a sample; the shaders' `sample_height` multiplies by
  `terrain.params.z`, the step), half R32Float's memory for the same heights.
- **Shoreline interaction needs the sea in `Phase::Water`.** The land writes depth and colour in `Phase::Opaque`; the
  ocean's surface already reads copies of the opaque scene for shoreline foam and the shallows' straight-through
  transmission (`Ocean::set_scene`). A host must draw the ocean in `Phase::Water` and ask for
  `SceneCopies::OPAQUE_COLOR | OPAQUE_DEPTH` (render core II). The stills do this with a small wrapper,
  `examples/common/scene.rs` (`SeaInWater`); copy it, or move `Ocean::draw` to `Phase::Water` in naval-ocean.
  `SkyWithPost` draws the sky as `GameSky` does (it ran the post passes, sun shafts and the rain's veil, at the
  backdrop copy point until both went), and `SkyMean` feeds the eye adaptation from the sky's environment.

## The field and its level of detail

| File | What |
| --- | --- |
| `src/ground/heightfield.rs` | `Heightfield`: samples in quanta of a step, as a theatre window or a test builds them; bilinear `height`, `land_within`, `PlacedTerrain` |
| `src/ground/field.rs` | Heights, half-float gradients, box means, the coast distance (chamfer), D8 flow accumulation, the surface attributes, the bound pyramid, the sun's visibility (a line sweep with penumbra). f64 arithmetic stored to f32 as the web game did, pinned to its arrays |
| `src/noise.rs` | The 512² value and Worley tile |
| `src/ground/lod.rs` | CDLOD selection, the tree patches nearest first |
| `src/land.rs` | `PreparedLand`: everything the land draws from that the CPU computes, with no device (`Send`), so a host prepares it on a worker; `TerrainLayer::for_scene` and `from_land` upload it |
| `src/layer.rs`, `shaders/terrain.wgsl`, `bindings.wgsl`, `lighting.wgsl` | The layer: textures with CPU mips, the vertex stage (morph, `Heightfield.height` in WGSL), the fragment's inputs, `bumped`, the terrain lighting model with three.js r185's indirect light (DFG table, multiple scattering, ambient occlusion), the debug views, the ocean's scene fog |
| `shaders/surface.wgsl` | `terrain_surface` and `tree_cover`: the styles, the seabed, the wet band and the foam line, the real-world surface ([surface.md](surface.md)) |
| `src/review.rs` | The review cameras: `main_coast`, `steepest_coast` and the presets `coast3`, `coast8`, `coast20`, `binoculars`, `aerial`, `cliffs`, `cliffsNear`, `overview` |

## The horizon ring

A theatre window's land stops at its edge (about 60 km about a ship, 120 km about a flight) and sinks into a 6 km band
of sea floor there. Beyond it the land is drawn as silhouettes out to the far land a view sees (Fuji, 3,776 m and 120 km
from Sagami Bay; the Moroccan interior, the Mojave or Hardangervidda from a plane at 3,000 m): coarse real heights and
the satellite colour, lit by the sun, under the same air as everything. Presentation only: the simulation, its sighting
and the chart read the window's heights as before.

```rust
let mut ring = HorizonLayer::new(ctx, &environment_bind_group)?;  // naval_terrain::horizon
ring.set_cloud_shadow(Some(&cloud_shadow_bind_group));
ring.set_maps(&maps);              // Arc<HorizonMaps>, made off the frame loop's thread; uploaded 4 MB a frame
ring.place(offset, Some((inner_centre, inner_half)));  // where the theatre is drawn; the window's square
ring.set_shown(shown);             // the chart and the table hide it
ring.set_climate(Some((snowline_m, thaw, leaf)), equator);   // the window's snow rule, or None
ring.draws_land();                 // something on the GPU and shown: the scene raises its far plane
ring.counts();                     // drawn, uploading, patches, GPU bytes
```

**The maps** (`HorizonMaps`). Two nested squares about one centre: a near level of 2048² heights at 120 m (246 km a
side) and a far one four times coarser (480 m, 983 km a side), each with 1024² of colour; `R16Float` heights and
`Rgba8UnormSrgb` colour with their mips, 33.5 MB on the GPU. The far level takes over over the near level's last 32
texels. The host (`naval-client` `theatre::horizon`) makes them on one utility-priority thread from the AWS Terrain
Tiles (Terrarium; zoom 10 for the near level at 35°, 8 for the far: SRTM, GMTED2010, ETOPO1 and ArcticDEM, and at zoom
10 the national models the tiles hold, credited in `ATTRIBUTION` and public/terrain/ASSETS.md) and EOX's Sentinel-2
cloudless 2016 (zooms 9 and 7), through the theatre's projection about the scenario's origin (every 16th texel exactly,
those between interpolated; longitude unwrapped about the centre, so a ring over the antimeridian is continuous). The
seabed is clamped to the open sea's floor (`SEA_FLOOR_M`, -60 m): under the sea the ring shows nothing (its patches are
left out) and the ocean draws the water. A centre stands on a 3.84 km grid; the maps are made again once the anchor (the
followed ship or flight, else the scene's origin) is 40 km from it, the latest asked for made next, the last two made
kept, a failed one asked again after a minute (the offline backoff is the surface streaming's). A first ring fetches
about 210 tiles (18 MB) and takes up to about 9 s on a home connection; from the cache a make takes 0.2-0.7 s.

**Disk cache.** The tiles as fetched, and a marker where a source has none, under the surface cache's `horizon-v1/`
(`~/Library/Application Support/gg.tomato.naval/surface-cache/horizon-v1/` on a Mac), **bounded at 256 MB**
(`CACHE_BYTES`): past it the least recently read tiles go, down to three quarters, checked every 128 tiles fetched.

**Drawn.** Each frame a quadtree of 16 × 16-quad patches about the maps' centre is chosen for the camera (CDLOD, as the
land's: 8 px triangle edges, the levels morphing into each other), leaving out patches wholly inside the window's inner
square, wholly under the sea, wholly below the sea's horizon from the eye or beyond the far plane: 340-900 patches a
view, 0.015-0.03 ms of CPU. The shader draws the photograph with `surface.wgsl`'s calibration, its baked relief shading
lessened on slopes, the sun and the sky's environment on the heights' normals, the clouds' shadow, and no shadow of its
own; no trees, buildings or features. Where the region's style takes the season's snow (`snow`, `chalk`, `rock`), the
window's climate snowline lays it: higher where sunny, lower in hollows, shed by steep faces as the window's are
(from a gentler slope in the melt, `snow_shed`). The curvature is `world_to_clip`'s: Fuji stands 2.98° over the
sea's horizon from a bridge in Sagami Bay, as 3,700 m at 120 km should. A fragment the air lets less than a 2048th of
through (fog, thick haze) is drawn as the air alone, which is what its light under the air comes to.

**The seam.** The window's land sinks over its outer 6 km (`WINDOW_FALLOFF_M`, `AIR_FALLOFF_M`) toward the sea floor.
The ring is drawn over that band and outside, and sinks 1 m a metre inward from the inner square's edge (`SINK`): the
surface seen is the higher of the two, continuous across the edge, and the window's land is the only one inside. The
camera clears both (`place_land`). What changes at the seam is the colour: the window's 23 m photograph gives way to
the ring's 240 m.

**The far plane.** Only the drawn frame's camera reaches past 60 km: `SeaScene::drawn_far` raises it to `HORIZON_FAR`
(450 km) while the ring has land on the GPU and the camera is curved, and the session renders with that camera; the
session's own camera keeps 60 km for the page's anchors, the tags, the sounds and the sun's shadow fit (which reads
`LAND_REACH`, not the far plane). Depth is reversed in 32-bit floats, so nothing near loses precision. What reads the
frame's far plane and follows it: the sea's ring (circles 15 km apart past 60 km, to 480 km; a chord sags 4.4 m:
native/docs/ocean/README.md) and the fleet's cull (ships beyond 60 km may draw); the aerial volume reaches 450 km in every view
(native/docs/haze.md). The clouds' shadow cascades keep a battle view's span (`max_far`), and the clouds' march stops at
the land's depth, so a cloud in front of a peak hides it and a cloud behind it does not draw over it. Two things kept
the sea's cost: the screen-space reflections mark only surfaces a ray from the drawn sea can reach, at most 60 km off
(the land's band along the horizon put every ray from the bridge into a full march for nothing: native/docs/ocean/cost.md),
and the sea's rows past its horizon close up a 20 m crest's distance beyond it (native/docs/render-core/frame.md, "The sea to its
horizon").

**Where.** Ship view, bridge, binoculars, cruise, the air views and the free camera, on the High terrain setting (High
and Ultra graphics). Not on the chart or the table. `NAVAL_HORIZON=off` turns it off alone; it is off with the surface
streaming. The theatre RPC's state carries `horizon` (made, last make's milliseconds, tiles and bytes fetched, the
cache's size and bound, and what is drawn).

**Cost** (Apple M5 Pro, Metal, Ultra, 1920 × 1080, `test-fast` client, 2026-10-09; the ring shown and hidden
alternately in the same frame state, four pairs of 250 frames a view over the 24 views under "Checked"; GPU timestamps per pass,
the median of the pairs' differences; other sessions' builds and Blender held the load average at 50-70, so a single
pair's frame moves ±0.3-0.5 ms):

| What | Cost |
| --- | --- |
| The scene's opaque pass (the ring's patches) | +0.0-0.26 ms in clear air; +0.57 through binoculars in fog or overcast |
| The sea | -0.95 to +0.42 ms: less from the air, where the land covers it; +0.42 off Bergen, where rays from water near the window's edge meet the ring's land within reach and are traced (its reflections); +0.16-0.26 over Tarawa's open sea |
| The clouds' march (stops at the ring's land) | -0.0-0.4 ms |
| The frame's whole GPU time | -0.93 to +0.71 ms, median -0.1 |
| Selection on the CPU | 0.015-0.03 ms |
| GPU memory | 33.5 MB (two levels of heights and colour with mips) |
| Disk | at most 256 MB of tiles; a first ring about 18 MB |

**Checked.** `bun run rust:test -- --native -p naval-terrain --lib horizon` (the levels against the quadtree and the far
plane, heights read as the GPU samples them and blended between the levels, the open sea and the window's square drawing
nothing, levels coarsening with distance), `-p naval-terrain backends` (`horizon.wgsl` on every backend), `--native -p
naval-client --lib horizon` (the tiles about the antimeridian, a mosaic's edges, the centres' grid), `naval-ocean`'s
ring spacing to 480 km, `naval-render`'s aerial slices. Stills: Fuji from Sagami Bay (bridge and binoculars, clear, the
scenario's weather and winter), the Rif behind Tangier, the Sierra Nevada, Bergen, the San Gabriels from a ship off San
Pedro and from planes at 1,500 m and 3,000 m, Tarawa (no far land: unchanged), and the seam from 3,000 m, before and
after in `.build/land-lanes/stills/HR/` (the lead session's checkout).

**Open.** Fuji's snow is a smooth cap with a crisp lower edge: the 120 m heights have no gullies for it to run down.
Far snowfields (Hardangervidda from the air, Folgefonna through binoculars off Bergen) read as a flat, bright white
band. The ring's photograph is paler and softer than
the window's at the seam. Distant cumulus over the ring's land reads as flat sheets from 3 km up (the march now stops at
the land where before it ran on over the sea). The south's seasons are the window's (northern) rule. Through binoculars
in fog or overcast the opaque pass costs +0.5-0.6 ms against +0.1-0.25 in clear air, with or without the far plane, and
with the fragment stage stubbed out: not pinned down.

## GPU cost

Apple M5 Pro, Metal, 1600 × 900, 2026-09-30, load average 23–35 (other agents building and running browsers):
wall-clock frames with the layer and without it, serialised (submit and wait), 24 alternating pairs, medians
(`timings.json`). Render-pass timestamps are unusable on this GPU for this (see native/docs/render-core/README.md),
so these are upper bounds under load.

| What | Cost |
| --- | --- |
| The land with its trees, per view | 0.12–1.3 ms (median 0.6); binoculars and the cliff views with 256 tree patches are the dearest (0.9–1.3 ms), a coast at 20 km 0.1–0.4 ms (the trees as the web placed them; their level of detail since: trees.md, "Costs") |
| The open-sky bake (once per battle) | below the noise of a frame (1201² × 16 directions × 20 steps) |
| The land as a shadow caster (on in the game since 2026-10-05, shadows.md, "The land's shadow") | each map's own patches: 9-36k triangles under a 10° sun, the shadow passes +0.00-0.03 ms on Low, +0.02-0.06 ms on Medium, +0.07-0.09 ms on High (none measurable among Kawah Lagoon's trees); it was about 2 ms when up to 1024 patches about a focus went into every map |
| The trees in the sun's maps, and the forest's shade (High) | Windows, RTX 5070 Ti: the shadows 0.1-0.35 ms; with the shade the trees 0.11-0.66 ms where they were 0.06-0.49 ms (trees.md, "Shadows") |
| The forest's shade bake (whole in the first frame and when the surface maps change; then a feature page's or near tile's rectangles, at most once every 8 frames) | a theatre window's whole bake 8-15 ms (M5 Pro, 1200² × 4 tree covers; a battle map's 601² a quarter of it); a page's or tile's rectangles 0.4-1.4 ms in a theatre flight (docs/archive/native-theatre-history.md, "The land's frame budget") |
| The trees' impostor atlas bake (once per device with a shared `TreeAtlas`, else once per land) | 12-15 ms (M5 Pro) |
| The fine relief near the camera (High, a theatre with a finer model than its window's; ground.md, "Fine relief near the camera") | +0.08-0.25 ms a frame at 1080 lines (+0.30 30 m up 1 km off Dover's cliffs); 88 MB |
| Building a map's land on the CPU (release) | 200–700 ms: field preparation, the noise tile, CPU mips, the tree atlas (the web: 150–230 ms for its part) |
| The sun visibility after the sun moves 0.3° | 25-28 ms of CPU for a theatre window (2401²): on the host's worker where it hands one (the upload, about a millisecond, on the render thread), else the render thread (shadows.md, "The land's own shadow") |

Draws: the land is one instanced draw (plus the trees' placement pass and their one indirect draw, and one more indirect
draw into each of the near and wide sun maps; with the land casting, one more instanced draw of its patches into each
map drawn). Memory for a map: heights 23 MB, gradients 31 MB with mips, attributes
7.7 MB, sunlight 5.8 MB, open sky 5.8 MB, detail 5.6 MB, noise 1.4 MB.

History (the web port, the battle maps' land, the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
