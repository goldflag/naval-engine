# Terrain assets

The battle terrain (`native/crates/naval-terrain/`) draws its close-range detail from aerial photographs. Those below
are CC0, from [Poly Haven](https://polyhaven.com/license); the game serves its own local copies and makes
no third-party requests at runtime.

| Runtime asset | Source | Artist | Ground covered by one repeat |
| --- | --- | --- | --- |
| rock-color.jpg | [Aerial Rocks 02](https://polyhaven.com/a/aerial_rocks_02), 1K diffuse | Rob Tuytel | 12–190 m by surface |
| ground-color.jpg | [Grass Ground](https://polyhaven.com/a/grass_ground), 1K diffuse | Charlotte Baglioni | 15 m and 3.7 m |
| sand-color.jpg | [Aerial Beach 01](https://polyhaven.com/a/aerial_beach_01), 1K diffuse | Rob Tuytel | 30 m |
| snow-color.jpg | [Snow Field Aerial](https://polyhaven.com/a/snow_field_aerial), 1K colour | Rob Tuytel | 80 m |

The terrain uses only each photograph's luminance, relative to its mean, as structure under colours it
chooses itself (`textures.rs`). Everything else it draws (the noise tile, tree crowns, fields and
hedgerows) is generated procedurally at runtime.

## Ground materials

The land's tiling ground materials (`native/crates/naval-terrain/src/ground/ground_materials.rs`, native/docs/terrain/ground.md,
"Ground materials") are made from these Poly Haven textures, every one CC0 ([licence](https://polyhaven.com/license)),
by `native/crates/naval-terrain/tools/ground_materials.py`, which fetches each one's 1K colour, OpenGL normal and
displacement maps and writes `materials/<name>-albedo.jpg` (the colour as a ratio to its own local mean) and
`materials/<name>-surface.jpg` (normal x and y, height). The game uses only their structure and relief, under the
colours it chooses itself.

| Material | Source (Poly Haven, CC0) | Author | Ground the source covers | One repeat in the game | Written at |
| --- | --- | --- | --- | --- | --- |
| grass | [Grass Ground](https://polyhaven.com/a/grass_ground) | Charlotte Baglioni | 2.5 m | 6 m | 512² |
| dry-grass | [Withered Grass](https://polyhaven.com/a/withered_grass) | Charlotte Baglioni | 2 m | 6 m | 512² |
| soil | [Farm Soil](https://polyhaven.com/a/farm_soil) | Amal Kumar | 2 m | 6 m | 512² |
| scrub | [Sparse Grass](https://polyhaven.com/a/sparse_grass) | Amal Kumar | 2 m | 6 m | 512² |
| dry-scrub | [Aerial Rocks 01](https://polyhaven.com/a/aerial_rocks_01) | Rob Tuytel | 80 m | 30 m | 1024² |
| forest | [Leaves Forest Ground](https://polyhaven.com/a/leaves_forest_ground) | Dario Barresi, Dimitrios Savva | 1.26 m | 2 m | 1024² |
| rock | [Aerial Rocks 02](https://polyhaven.com/a/aerial_rocks_02) | Rob Tuytel | 50 m | 24 m | 1024² |
| cliff | [Marble Cliff 05](https://polyhaven.com/a/marble_cliff_05) | Amal Kumar | 20 m | 20 m | 1024² |
| sand | [Coast Sand 01](https://polyhaven.com/a/coast_sand_01) | Rob Tuytel | 15 m | 15 m | 1024² |
| wet-sand | [Coast Sand 02](https://polyhaven.com/a/coast_sand_02) | Rob Tuytel | 15 m | 15 m | 1024² |
| shingle | [Clean Pebbles](https://polyhaven.com/a/clean_pebbles) | Charlotte Baglioni, Dario Barresi | 2 m | 5 m | 512² |
| snow | [Snow 02](https://polyhaven.com/a/snow_02) | Rob Tuytel | 2 m | 10 m | 512² |
| mud | [Brown Mud 02](https://polyhaven.com/a/brown_mud_02) | Rob Tuytel | 1.3 m | 6 m | 512² |
| paving | [Aerial Asphalt 01](https://polyhaven.com/a/aerial_asphalt_01) | Rob Tuytel | 30 m | 30 m | 1024² |
| fine-sand (the tropical and volcanic beaches) | [Sand 03](https://polyhaven.com/a/sand_03) | Charlotte Baglioni | 2 m | 3 m | 1024² |
| limestone (the cliffs' chalk and limestone) | [Marble Cliff 04](https://polyhaven.com/a/marble_cliff_04) | Amal Kumar | 12.7 m | 16 m | 1024² |
| basalt (the cliffs' basalt) | [Dark Rock 02](https://polyhaven.com/a/dark_rock_02) | Amal Kumar | 2 m | 11 m | 1024² |
| granite (the cliffs' granite) | [Marble Rock 01](https://polyhaven.com/a/marble_rock_01) | Amal Kumar | 2 m | 13 m | 1024² |
| needles (a conifer wood's floor) | [Forest Ground 03](https://polyhaven.com/a/forrest_ground_03) | Rob Tuytel | 2 m | 2 m | 1024² |

The limestone, basalt and granite are photographed face on and read face on on the cliffs (`materials.wgsl`'s
`cliff_read`), each where its place's rock is that one (native/docs/terrain/ground.md, "Cliffs"); the needles are read through
the forest floor where its trees are conifers (`materials.wgsl`'s `floor_read`). The land's weights never choose them.

## Climate and rock

Two global maps are compiled into the game (`native/crates/naval-terrain/src/earth/`, each module's `ATTRIBUTION`;
native/docs/terrain/ground.md, "Climate and rock maps"), baked by `scripts/theatre/bake-climate-and-rock.py`. They
choose the land's climate and the rock of its cliffs.

| What | Source | Licence | Credit |
| --- | --- | --- | --- |
| Climate (`koppen.nkg`) | [Köppen-Geiger maps 1901-2099, version 3](https://doi.org/10.6084/m9.figshare.21789074), 1931-1960 at 30" | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) | Beck, H. E., T. R. McVicar, N. Vergopolan, A. Berg, N. J. Lutsko, A. Dufour, Z. Zeng, X. Jiang, A. I. J. M. van Dijk and D. G. Miralles (2023), High-resolution (1 km) Köppen-Geiger maps for 1901-2099 based on constrained CMIP6 projections, Scientific Data 10, 724, doi:10.1038/s41597-023-02549-6 |
| Rock (`glim.ngl`) | [GLiM, the Global Lithological Map](https://doi.org/10.1594/PANGAEA.788537), the 0.5° raster | [CC BY 3.0](https://creativecommons.org/licenses/by/3.0/) | Hartmann, J. and N. Moosdorf (2012), The new global lithological map database GLiM: A representation of rock properties at the Earth surface, Geochemistry, Geophysics, Geosystems 13, Q12004, doi:10.1029/2012GC004370 |

## Fine relief

Near the camera, a theatre's land takes heights finer than its window's 40 m grid where a free elevation model covers
it (`native/crates/naval-client/src/theatre/land/relief.rs`, `ATTRIBUTION`; native/docs/terrain/ground.md, "Fine relief near the
camera"). They are read at runtime near the camera and kept in the user's cache, not shipped with the game.

| Where | Source | Licence | Credit |
| --- | --- | --- | --- |
| United States (conterminous, Hawaii, Puerto Rico) | [AWS Terrain Tiles](https://registry.opendata.aws/terrain-tiles/) (Terrarium, zoom 15): USGS 3DEP | Public domain | 3DEP data courtesy of the U.S. Geological Survey |
| England | the same tiles: Environment Agency LIDAR | [Open Government Licence v3](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/) | © Environment Agency copyright and/or database right 2015. All rights reserved. |
| Norway | the same tiles: Kartverket DTM | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) | © Kartverket |
| New Zealand | the same tiles: LINZ 8 m DEM | [CC BY 3.0 NZ](https://creativecommons.org/licenses/by/3.0/nz/) | Copyright 2011 Crown copyright (c) Land Information New Zealand and the New Zealand Government. All rights reserved |
| (the tiles' coarser fill) | SRTM, GMTED2010 (USGS), ETOPO1 (NOAA) | Public domain | SRTM and GMTED2010 data courtesy of the U.S. Geological Survey; Global ETOPO1 terrain data U.S. National Oceanic and Atmospheric Administration |
| Japan | [GSI elevation tiles](https://maps.gsi.go.jp/development/ichiran.html) (`dem5a_png` 5 m, `dem_png` 10 m) | [GSI content terms](https://www.gsi.go.jp/kikakuchousei/kikakuchousei40182.html): the Public Data License 1.0 (compatible with CC BY 4.0, commercial use allowed), the source named and the processing stated | 出典：国土地理院 標高タイル（加工して作成） |

The Terrain Tiles' credits are tilezen/joerd's ([attribution](https://github.com/tilezen/joerd/blob/master/docs/attribution.md)).
Nothing finer than the window is used elsewhere: the Mediterranean (Malta, Gibraltar), Scotland and the Pacific islands
outside Hawaii keep the window's heights.

## The horizon ring

Beyond a theatre window the land is drawn as coarse silhouettes out to 450 km
(`native/crates/naval-client/src/theatre/land/horizon.rs`, `ATTRIBUTION`; native/docs/terrain/layer.md, "The horizon ring"). Its
tiles are read at runtime and kept in the user's cache under `surface-cache/horizon-v1/`, at most 256 MB (the least
recently read go first), not shipped with the game.

| What | Source | Licence | Credit |
| --- | --- | --- | --- |
| Heights (zooms 8 and 10) | [AWS Terrain Tiles](https://registry.opendata.aws/terrain-tiles/) (Terrarium): SRTM, GMTED2010 (USGS), ETOPO1 (NOAA), and above 60° ArcticDEM | Public domain (ArcticDEM unlicensed) | SRTM and GMTED2010 data courtesy of the U.S. Geological Survey; Global ETOPO1 terrain data U.S. National Oceanic and Atmospheric Administration; DEM(s) were created from DigitalGlobe, Inc., imagery and funded under National Science Foundation awards 1043681, 1559691, and 1542736 |
| Heights at zoom 10 (the near level) | the same tiles: 3DEP (United States), the Environment Agency (England), Kartverket (Norway), LINZ (New Zealand), CDEM (Canada), DGM (Austria), INEGI (Mexico), EU-DEM (Europe) | Public domain; Open Government Licence v3; CC BY 4.0; CC BY 3.0 NZ; Open Government Licence – Canada; CC BY 3.0 AT; INEGI's free-use licence; Copernicus terms | as "Fine relief" above, and: Contains information licensed under the Open Government Licence – Canada; © offene Daten Österreichs – Digitales Geländemodell (DGM) Österreich; Source: INEGI, Continental relief, 2016; Produced using Copernicus data and information funded by the European Union - EU-DEM layers |
| Colour (zooms 7 and 9) | [EOX Sentinel-2 cloudless 2016](https://s2maps.eu) | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) | Sentinel-2 cloudless - https://s2maps.eu by EOX IT Services GmbH (Contains modified Copernicus Sentinel data 2016) |
