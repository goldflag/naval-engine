# The land pipeline, bake to drawing

The land's heights come from one place, a **land release**: immutable 1° squares on one global lattice, baked from
pinned sources. The simulation and the native scene read the same squares. Everything else the land draws
(photographs, land cover, roads, trees, buildings, lights) the client fetches about the camera from open sources and
hands to the GPU layers. None of that reaches the simulation.

| Stage | Code | What it does | More |
| --- | --- | --- | --- |
| Sources | `naval-region-bake/src/release/` (`snapshot`, `pieces`, `sources`, `gebco`, `surveyed`) | Pins a release's inputs: OpenStreetMap's water polygons (the coast), Terrarium or Copernicus GLO-30 heights, GEBCO 2026 for the sea floor from `land-v2` on | [land.md](../../../docs/theatre/land.md), "Land releases" |
| Bake | `native/crates/naval-region-bake` (`square`, `glo30`, `terrarium`, `despike`, `finish/`) | Pure: no network, files or threads. Bakes one final square from its sources; `finish` reconciles the coast with the heights on the lattice, so a point's value depends on its place alone | `docs/earth.md` |
| Bake's host | `naval-land-bake` (`release`, `dem`, `hosted`, `cli`); the client's `theatre::land::region_bake` runs it on the user data directory | The network, the disk and the threads around the bake, with nothing of the client or the simulation; `naval-land-bake --data <folder>` and `naval-client land-release` make a release and bake missing squares ([README](../../crates/naval-land-bake/README.md)) | [land.md](../../../docs/theatre/land.md), "Where the sources are hosted" |
| Format | `crates/naval-land-format` | The square key (`N36W123`), the lattice (1/1,600°), NSQ1 blocks of 0.25°, the release file, the square table, the manifest, the synthetic release | [crates/README.md](../../../crates/README.md) |
| Store | `native/crates/naval-land` | Squares on disk by content, pins and leases, the size bound, export and import; `naval-land synthetic <data>/land` installs the synthetic release | |
| Simulation | `crates/naval-sim/src/land/` (`ReleaseLand`, `Land`) | Grounding, collisions, sight lines and route planning, by place | [land.md](../../../docs/theatre/land.md), "Reading a release" |
| The client's release | `theatre::land::land_release` | Admits squares through the store (`release_land`); cuts a window from them (`release_window`) | |
| Without the simulation | `naval_terrain::release` (`src/ground/release.rs`) | The same cut for a host with no simulation: a `ReleaseWindow` about a place names its squares, a `ReleaseLand` holds their files, `heightfield` cuts the window on the theatre's chart (no summits raised); `engine-example` draws the synthetic release through it | [engine-example](../../crates/engine-example/README.md) |
| The window | `theatre::land` (`mod.rs`) | A ship's view takes a 96 km window at 40 m, an aircraft's 240 km; the window is in the scene chart's metres, recut as the view moves | [layer.md](layer.md) |
| The window's data | `theatre::land`'s other modules (table below) | Fetch, cache and resample the open sources into maps and pages for the layers, off the frame loop's thread | each page |
| Drawing | `native/crates/naval-terrain`: `TerrainLayer`, `BuildingsLayer`, `InfraLayer` | Draw the land, its trees, buildings, infrastructure and lights; `theatre::land_layer` (`theatre/mod.rs`) builds the land layer for a scene | [layer.md](layer.md) |

## The land's modules

Every land module is presentation only. The land's look, its ground and what stands on it, is
`native/crates/naval-landscape`, which needs nothing of the client or the simulation: its host names the folder its
caches live under and the threads its workers run on (`naval_landscape::host`; the client's
`theatre::land::install_host` gives the user data folder and its utility class, `qos`), and hands it a window's heights
in its own type (`near::WindowHeights`: the client's `land::NearHeights` over the simulation's field). The client
re-exports its modules at their old paths (`theatre::surface`, `theatre::buildings`, and the rest) and keeps the window,
the release, the shore and the game's glue in `native/crates/naval-client/src/theatre/land/`: `structures.rs` makes the
buildings' and infrastructure's layers with a cruise scene (`buildings::SceneLook`: its sky, clouds' shadow and fog)
and feeds them each frame, and hands the bases' parked aircraft (`bases::ParkedPlane`) to the scene's.

| Module | Crate | Source | Hands the layer | Page |
| --- | --- | --- | --- | --- |
| `surface`, `near`, `naip` | naval-landscape | EOX Sentinel-2 cloudless 2016, ESA WorldCover 2021; USDA NAIP in the United States | `SurfaceMaps`, near tiles | [surface.md](surface.md) |
| `osm`, `features`, `features_period` | naval-landscape | OpenStreetMap vector tiles from OpenFreeMap, read by `naval-osm` | road, rail and water distance-field pages | [surface.md](surface.md), "Map features on the ground" |
| `relief` | naval-landscape | AWS Terrain Tiles, GSI elevation tiles (Japan) | relief pages | [ground.md](ground.md), "Fine relief near the camera" |
| `horizon` | naval-landscape | AWS Terrain Tiles, EOX imagery | `HorizonMaps` | [layer.md](layer.md), "The horizon ring" |
| `canopy` | naval-landscape | Meta and WRI's 1 m canopy height map | canopy pages | [trees.md](trees.md), "Trees where real trees stand" |
| `period_land` | naval-landscape | HYDE 3.3 (`naval_terrain::buildings::history`) | the period's country marks | [period.md](period.md), "The period's ground" |
| `lamps` | naval-landscape; the scenario's rule and the scene on screen the client's | the features' pages | lamps | [night.md](night.md) |
| `water` | naval-landscape | an authored atlas of coastal waters | the sea's colour | |
| `buildings`, `buildings_split`, `buildings_period` | naval-landscape; the cruise scene's layer and feed the client's (`land::structures`) | Microsoft Global ML Building Footprints, OpenStreetMap's tags | building chunks | [buildings.md](buildings.md), [period.md](period.md) |
| `infra`, `bases` | naval-landscape; the scene's layer, feed and parked aircraft the client's (`land::structures`) | OpenStreetMap; `assets/bases/bases.v1.json` | infrastructure chunks, bases, their clearings and parked aircraft | [infra.md](infra.md) |
| `power` | naval-landscape | Overture Maps, read by `naval-overture` | power lines and masts | [infra.md](infra.md), "Power lines" |
| `shore` | naval-client | the window's heights and the wind | the shore map for the waves (`naval_ocean::shore`) | [ocean/coasts.md](../ocean/coasts.md), "Coasts" |

## The imports

- **OpenStreetMap** (`native/crates/naval-osm`). Portable: the Mapbox Vector Tile decoder (`mvt`), the OpenMapTiles
  layers the game reads, typed (`omt`: roads and railways, waterways, water areas, land use and cover, buildings,
  points of interest, aerodromes), and the web-mercator tile math (`tiles`). naval-landscape's `osm` fetches
  the tiles from OpenFreeMap, caches them and projects them into a scenario's frame for the features, the buildings'
  tags, the bridges, the airfields and the lamps.
- **Overture Maps** (`native/crates/naval-overture`). Its base theme's `infrastructure` type, GeoParquet on the public
  bucket, read over a box without downloading the files: overhead lines, power towers and poles, communication masts
  (`PowerCell`). naval-landscape's `power` makes the requests and keeps the cells under `overture-cache`.
- **The rest** are read by the land's modules themselves (table above). Their credits are each module's
  `ATTRIBUTION`.

## Where a coastline is drawn

1. **Baked.** `naval-region-bake/src/finish/`: `coverage` takes the sea from OpenStreetMap's water polygons, `coast`
   slides the heights' coastal profile onto the coastline and rounds it, `keep` decides which squares hold land.
2. **Read by the simulation** from the same squares (`naval_sim::land`): grounding and the coast's collisions.
3. **Cut into the window** by the client (`land_release::release_window`), or by a host without the simulation
   through `naval_terrain::release`. The window's land sinks over its outer 6 km toward the sea floor.
4. **Drawn** by the land layer. `ground/field.rs` keeps each sample's signed distance to the coast; `surface.wgsl` draws
   the seabed, the wet band and the foam line within 25-90 m of it ([surface.md](surface.md), "Seasons, climate and
   tiling"), and the beaches' sand ([ground.md](ground.md), "Ground materials").
5. **In the waves.** `theatre::land::shore` builds the shore map that `naval_ocean::shore` reads for fetch, swell and
   depth ([ocean/coasts.md](../ocean/coasts.md), "Coasts").
6. **Beyond the window** the horizon ring draws the coast's silhouette from coarser heights ([layer.md](layer.md),
   "The horizon ring").
7. **On the chart** the client draws the sea and land itself ([theatre.md](../theatre.md), "The chart's ground").

## Summit heights

How far a summit falls between the source and the screen, measured at thirteen summits: the highest value within
900 m of each, the window's over 16 placings of its grid
(`theatre::land::land_release::summit_tests::report_summits_through_the_window`, an ignored test reading the squares
in `NAVAL_LAND_DIR`'s store; Terrarium's and GLO-30's own pixels read from their tiles). "Window's cut" is the window
before `lattice_summits` raises its samples (below).

| Summit | Real | Terrarium z11 (z13) | Release lattice | Window's cut | Window | Tour's probe |
| --- | --- | --- | --- | --- | --- | --- |
| Jungfrau | 4,158 | 4,037 (4,045) | 4,027.2 | 4,024.5 | 4,027.2 | 4,020.9 |
| Finsteraarhorn | 4,274 | 4,116 (4,133) | 4,095.5 | 4,093.5 | 4,095.5 | 4,090.9 |
| Eiger | 3,967 | 3,918 (3,916) | 3,904.5 | 3,901.4 | 3,904.5 | 3,894.1 |
| Mönch | 4,107 | 4,061 (4,062) | 4,053.0 | 4,048.7 | 4,053.0 | 4,043.1 |
| Schilthorn | 2,970 | 2,937 (2,938) | 2,928.0 | 2,924.7 | 2,928.0 | 2,921.2 |
| Nanhu Dashan | 3,742 | 3,720 (3,732) | 3,712.0 | 3,710.8 | 3,712.0 | 3,709.6 |
| Qilai North Peak | 3,607 | 3,573 (3,588) | 3,567.7 | 3,566.7 | 3,567.8 | 3,565.3 |
| Rinjani | 3,726 | 3,671 (3,684) | 3,667.0 | 3,663.6 | 3,667.0 | 3,655.2 |
| Dalsnibba (GLO-30) | 1,476 | 1,462 (1,474) | 1,462.7 | 1,461.7 | 1,462.8 | 1,460.7 |
| Zoroaster Temple | 2,188 | 2,145 (2,178) | 2,113.2 | 2,104.8 | 2,113.2 | 2,099.0 |
| Isis Temple | 2,190 | 2,102 (2,136) | 2,071.2 | 2,066.8 | 2,071.2 | 2,054.2 |

Mather and Bright Angel Points are on rims rising away from them, no summits: the window's cut reads them within a
metre. Qilai North, Zoroaster and Isis stand where Terrarium's zoom-13 tiles put their tops (Isis Temple's 2,190 m is
likely high, as zoom 13 tops it at 2,136). What each step loses:

- **The source**: Terrarium at zoom 11, the bake's below 60° N, stands 22-158 m (0.6-3.7 %) under the real summits,
  and zoom 13 gains at most 33 m: the elevation models under the tiles hold the summits low. Copernicus GLO-30 holds
  Eiger, Mönch, Schilthorn and Nanhu Dashan within 12 m of the real, Rinjani 21 m low, but Jungfrau 66 m and
  Finsteraarhorn 188 m low. Most of the shortfall is here.
- **The bake** (`naval-region-bake` `square::bake_heights`: each 70 m sample the mean of 2 × 2 bilinear reads ±17 m
  about it, two passes of the 630 m despike, the columns resampled linearly onto 1/1,600°): 4-21 m on the alpine and
  volcanic summits, 31-32 m on the canyon's temples, whose tops are narrower than the supersample's reach.
- **The window's cut**: 40 m samples read the lattice between its points, so a summit held by one point kept only part
  of its height over its neighbours: 1-8 m. `land_release::lattice_summits` finds every lattice point above
  its eight neighbours and 10 m (on a few threads, each tile of 64² points read once, a tile whose summary cells hold no
  land passed over), and `land::settled_window` raises the sample nearest each to it, half a cell from it at most. A
  sea window raises 4,397 samples about the Bernese Oberland (2.0 m on average, 15.5 m at most), 9,856 about the Grand
  Canyon (1.4 m, 52 m) and 8,288 about Geiranger (2.4 m, 43 m); an air window's 140 m samples 51,773 about the
  Oberland (3.4 m, 101 m). Finding the summits takes 0.01-0.05 s for a sea window and 0.19 s for an air window (each
  tile's points in one run, `ReleaseLand::lattice_heights`), against the cut's own 0.6-1.1 s and 0.5 s (i7-12700K,
  Windows, the least of three with other worktrees building). Presentation only: the simulation reads the
  lattice, and everything that reads the window (the GPU, `theatre_landHeight`, buildings' footing, the shore, the
  camera's clearance) reads the raised samples alike. Where fine relief draws, its pages add the finer model less its
  own reading at the window's samples, so the drawn top is the finer model's plus the window's shortfall from it at the
  nearest sample: a raised sample shortens that, or stands above the finer model by at most its raise.
- **The mesh**: on High, level 0's 20 m and level 1's 40 m vertices stand on the window's samples to about 9 and 18 km
  (1080 lines, a 50° lens); beyond, level 2's 80 m vertices take one sample in four, and a far summit is drawn low by up
  to its raise again.
- **The tour's probe** (`theatre_landHeight` on a 75 m grid of bilinear reads) reads 1-13 m under the window's highest
  sample; its "drawn" heights are the probe's, not the window's (the tour also climbs to the sample).
- **The horizon ring** (beyond the window: 120 m texels, one bilinear read of Terrarium's zoom 10 at each centre)
  reads 2-16 m under zoom 10's highest pixel there, which keeping the footprint's maximum would recover; from the
  ring's 42 km out that is at most half a pixel at 1080 lines and 50° (a pixel at 1,600 lines and 38°), so the ring is
  unchanged. Zoom 10 itself is 0-34 m under zoom 11 at the summits above, 83 m at Zoroaster.

**A future bake** (a new release version; the simulation's heights change with it) would need, by what it gains: a
source that holds the summits Terrarium's models hold low (no single one does: GLO-30 is better at four of the five Alps
summits and far worse at Finsteraarhorn, so a choice per square or a national model such as swisstopo's swissALTI3D);
zoom 12 where Terrarium holds a finer model (3DEP in the United States: +29-40 m at the temples); a supersample that
keeps a summit (the 2 × 2 reads' greatest where the sample stands above its neighbours, as the window now does); and the
despike's threshold checked against needles and stacks, which it may erase outright (not measured here).
