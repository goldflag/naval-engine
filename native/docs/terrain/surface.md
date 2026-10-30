# Real-world surface

A theatre window is painted from real-world data, satellite colour and land cover, over the styles' procedural masks, so
the land reads as the place it is: its fields, towns, roads, canals and woods where they are, modern ones included. The
host (`naval-client` `theatre::surface`; docs/archive/native-theatre-history.md, "Real-world surface (streamed)")
fetches the maps for its window and hands them in; a land with none (the tests') draws its style's procedural cover.

```rust
let maps = SurfaceMaps { origin, extent_m, size, albedo, cover };   // naval_terrain::surface_maps
land.set_surface_maps(&mut renderer.context(), &maps)?;              // uploaded now, drawn from the next frame
land.clear_surface_maps(&mut renderer.context());                    // the procedural cover alone again
land.has_surface_maps();
land.surface_photo();   // the colour array's view, the maps' frame and haze, a version: what is drawn over the land
```

`SurfaceMaps` is a square of `size`² texels over `extent_m` metres of chart whose corner is `origin`; texel (i, j)
covers chart x from `origin[0] + i · extent_m / size`, z likewise, row j = 0 at the smallest z (checked with a synthetic
quadrant map: the split falls on the chart's own axes, its empty band where its alpha is 0). `albedo` is sRGB RGBA8,
alpha 255 where imagery covers the texel and 0 where it does not; `cover` an ESA WorldCover class byte (0 unknown).

**Textures and bindings.** Two more bindings on the terrain's group 1, which every land pipeline (land, trees, shadow
casters) shares: 14 in all (one uniform, eleven sampled textures, two samplers). With the camera group's two shadow
maps, the sky's two and the cloud shadow's one, a land pipeline samples 16 textures a stage, the default limit: nothing
more can be bound without asking the device for more (so the near tiles below share these two). The maps could not take
a group of their own: a pipeline's four groups are camera, terrain, sky and cloud shadow.

| Binding | What | Read |
| --- | --- | --- |
| 12 `surface_albedo` | `Rgba8UnormSrgb` array: layer 0 the window's maps, layer 1 (with a near grid only) the near tiles; mips to 10 levels (512² to 1² of a near tile), built on the CPU with the colour **premultiplied by its alpha in linear light** (`textures::mips_srgb8_premultiplied`), so filtering never bleeds an empty texel into the imagery's edge (the shader divides by the filtered alpha) | layer 0 through the clamped sampler, layer 1 the repeating one: filtered, mip-mapped, anisotropic |
| 13 `surface_cover` | `R8Uint` array, the same layers, one level | `textureLoad`, texel by texel: class ids are never filtered |
| `terrain.surface` | the maps' chart origin (x, z), side (m) and texels a side; all 0 with none bound | `surface_bound()` (maps or near tiles) is a uniform branch, so a land with none runs none of the maps' code |
| `terrain.surface_haze`, `terrain.cover_means[12]` | `SurfaceMaps::statistics`, computed in `set_surface_maps` from every other texel of every other row: the haze to take out, each class's mean colour in the window | below |
| `terrain.near`, `terrain.near_slots[32]` | the near grid (tile (0, 0)'s chart corner, a tile's side, tiles a side; 0 with none) and which tile each slot holds | "Near tiles" below |

With no maps the layer binds 1 × 1 placeholders (an empty colour, an unknown cover), so every pipeline keeps one
layout; setting or clearing maps rebuilds the terrain bind group only (and places the trees again). The trees' placement
pass reads both maps (and the near tiles' cover where they are drawn).

**Near tiles** (2026-10-04). A window's maps are 23 m a texel (a sea window's) or 117 m (an air window's); near the
camera the host adds finer squares of the same sources (docs/archive/native-theatre-history.md, "Near tiles"; in the
conterminous United States a sea window's are 2 m, NAIP's aerial imagery matched to Sentinel-2's brightness, keeping
most of its own hue: "NAIP over the United States" and "Yards and standing trees" below), drawn in their place:

```rust
land.set_near_grid(&mut renderer.context(), NearGrid { origin, tile_m, slots })?;  // before set_surface_maps
land.queue_near_tile([i, j], Arc::new(NearTile::from_maps(&maps_512)?));            // no renderer context needed
land.holds_near_tile([i, j]); land.near_tile_counts(); land.camera_chart();
land.flush_near_tiles(&mut renderer.context());   // a layer being built: upload what is queued now
land.clear_near_tiles(&mut renderer.context());
```

Tile (i, j) covers chart x from `origin[0] + i · tile_m`, z likewise, 512² texels (`NEAR_TILE_TEXELS`), and lies in
slot (i mod `slots`, j mod `slots`) of layer 1, so the layer's texture coordinates are the chart's over the slots' span,
read repeating: continuous from slot to slot, with no seam in the filtering's derivatives, and a tile's own mips are
exactly the layer's there (512 is 2⁹). A `NearTile` comes with its colour's ten levels built (off the frame loop's
thread); `queue_near_tile` only queues it, and the layer's `prepare` uploads queued tiles, up to 4 MB a frame (two
tiles, 0.17 ms each on the render thread), changing the slot's entry in the uniforms in the same frame, so a slot never
draws half a tile, and a slot keeps drawing its old tile until replaced. The shader draws a slot only where its entry
names the tile at the point (`near_drawn`), fading a tile out over its outer fifth toward a neighbour not drawn
(`near_share`); the near colour and cover take the window maps' place by the tile's coverage there (both
premultiplied: where a near tile holds no imagery the window's maps show through), and the trees' placement takes the
near cover likewise. The grid's texels (`slots · 512`) must match the maps' side, and the grid is set before the maps:
a layer given maps alone makes their textures with one layer (no memory spent on tiles a window does not take).

**Magnified** (2026-10-04). Within a few kilometres of the camera a texel of either layer spans tens of pixels, and
the bilinear filter drew the photograph as soft blobs and diamonds. Where a layer's texel spans more than a pixel
(`magnified_share`: fully from 2.5 pixels a texel), its colour is rebuilt from level 0 (`magnified`): a Catmull-Rom
spline through five bilinear taps (Jimenez's approximation), the photograph's finest detail against its level 1 added
back by 0.35, and half of it the bilinear filter with its weights steepened about each texel's middle, so texel meets
texel at an edge 0.36 of a texel wide (a roof, a road, a field's edge stay edges); the lookup warped by noise a few
texels across by up to 0.8 of a texel, so the edges are irregular, the land's, not the texel grid's. Explicit levels
only, so it runs per pixel only where it shows. The source's own pixels are 8 to 19 m (the 2016 layer stops at zoom
13): nothing here adds what it does not hold, but it is drawn as patches with edges rather than a blur.

**Where the maps paint** (`real_surface` in `surface.wgsl`, mixed over the style's cover in `terrain_surface`): by the
imagery's coverage (its filtered alpha), feathered over two texels at the square's edge, faded back to the style on
steep faces (slope 0.55-0.95), which a photograph from above barely sees (the chalk cliffs, the snow style's bare faces
and the volcanic rock stay the style's). On High and Ultra since 2026-10-09 the photograph holds on every slope and the
place's rock covers the faces instead (ground.md, "Cliffs"). The seabed, wet band and foam line are the style's as before.

**The photograph as albedo.** Measured on EOX s2cloudless (linear luminance by WorldCover class), the 2020 edition over
Kent and the Pas-de-Calais: woods 0.029, grass 0.052, fields 0.069, towns 0.095, bare chalk and sand 0.27, the land
0.060, chroma 0.75-0.9 of the luminance; over Guadalcanal the rainforest 0.025 at chroma 1.1. The 2016 edition the
client streams (the one under CC BY) has the same luminance with a hazy blue-green veil: its darkest land holds
0.011-0.020 of blue where the 2020 edition's holds 0.002, and it keeps a few clouds over the Solomons' ridges. The styles
were authored brighter and calmer: the chalk style's downs 0.16 and fields 0.10, the tropical canopy 0.054, the snow
style's heath 0.04, chroma 0.6-0.7. So, in order:

1. *Haze.* The window's darkest land (first percentile per channel, water, snow and unknown cover left out) less a
   clear mosaic's (`DARK_LAND`, 0.002, 0.010, 0.002) is taken out of every texel, at most 0.02 a channel
   (dark-object subtraction): the 2016 edition's veil goes and the 2020 edition is left as it is.
2. *Clouds the mosaic kept.* Over green cover, a texel 2.2-3.5 times brighter than the window's mean for its class
   (clouds left out of that mean) and grey is replaced by that mean.
3. *Calibration* (since 2026-10-08, branch `goldflag/land-light`). The imagery is close to true reflectance, so it is
   compressed only a little (`PHOTO_GAIN · l^PHOTO_GAMMA`, 0.552 and 0.8: woods 0.033, grass 0.052, fields 0.065, towns
   0.084, bare ground 0.19, the land 0.058), its chroma kept at `PHOTO_SATURATION` 0.9 and capped at `PHOTO_CHROMA` 0.9
   of the luminance (the rainforest's 1.1 becomes 0.9), and it does not take the land's tone below: the styles do, the
   photograph keeps its colour (a road drawn over it takes the tone as before). Until then it was lifted and compressed
   toward the styles (0.447 and 0.643, chroma 0.75 capped at 0.7) and then toned (30% less saturated, 14% darker): it
   kept about half its chroma, and from 1,500-3,000 m the Los Angeles basin read frosted and the Kanto plain grey felt.
   Measured over the cached mosaic (the shader's arithmetic in numpy over its zoom-13 tiles; linear luminance, chroma as
   (max − min) / luminance, and the standard deviation of the log luminance as contrast): the basin 0.152, 0.31 and 0.47
   before, 0.178, 0.54 and 0.58 now (the mosaic itself 0.25, 0.60, 0.73); Yokohama's plain 0.084, 0.19, 0.51 before,
   0.086, 0.32, 0.64 now; Guadalcanal 0.041, 0.49, 0.43 before, 0.036, 0.87, 0.54 now. The pin is the towns' luminance
   (0.095 in the mosaic, 0.084 either way): woods and the rainforest are a fifth darker than before, bright ground a
   sixth brighter. Judged against USGS NAIP (public domain, `.build/land-light/naip/`, the views' own ground from
   `exportImage`): over Hermosa Beach from 500 m the ballfield is NAIP's green with its tan infield, not grey; NAIP
   itself is greyer than the mosaic over the basin (chroma 0.17), so the mosaic's warmth is kept short of its full 0.6.
   The gamma stays below 1: the mosaic's range (the basin's 0.25 against the rainforest's 0.035, seven to one) is wider
   than the ground's. From altitude most of the basin's pallor is the air's (the land drawn without its aerial
   perspective is tan), which this leaves as it is.
4. *Its own relief shading.* The photograph's sun lit the slopes facing it, and ours shades them again: where the land
   slopes (0.05-0.35) up to 60% of its local contrast against a mip 2.5 levels coarser is divided out. On the flat that
   contrast is fields, hedges and roads, and stays.
5. *Its detail's contrast* (2026-10-04). The calibration's exponent compressed every detail's contrast against its
   surroundings too, a road or a roof against the broad sample (about six texels of the window's maps; the near
   tiles' broad sample spans the same ground), and the land read flat and soft: the detail gets `PHOTO_DETAIL` of the
   exponent back (the colour times (texel / broad)^`PHOTO_DETAIL`, clamped to a ratio of 4 either way, less on slopes
   as the relief above), the broad colour calibrated as before, so the class means stand. 0.25 over the old 0.643, 0.12
   over 0.8 since 2026-10-08: a detail's exponent about 0.9 either way.

**Near the camera** the photograph (tens of metres a texel) is a blur, so each class lays the small structure of its
kind over it; every pattern fades to its mean as the pixel outgrows it, so far off the photograph alone remains. The
class masks are the four cover texels about the point blended bilinearly, sharpened toward the nearest class (cubed and
normalised: a wood ends where it ends) and looked up through a noise warp about a texel across so the texel grid never
shows as stairs. Under the open classes (grass, shrubs, fields at 60%, wetland; bare ground's mottling alone) the
ground also takes structure finer than the photograph's pixel: soil and growth mottled at 5.3 and 2 m (30% and 16%),
and the grass photograph's grain a second time at 3.7 m (55%), each fading out as the pixel outgrows it.

| WorldCover class | Drawn as | Trees (density at the foot, height × the style's) |
| --- | --- | --- |
| 10 tree cover | the canopy's crowns (11 m) over the photograph, their relief and shade, where no trees stand drawn (the cards' crowns are theirs: "Yards and standing trees" below); in a seasonal style's winter (chalk, snow: `1 - leaf`) grey-brown, as the cards turn bare | 0.92, × 1 |
| 95 mangroves | the canopy, darker and a little glossier | 0.85, × 0.45 |
| 20 shrubland | half the canopy's pattern, the grass photograph's grain | 0.22, × 0.45 |
| 30 grassland, 100 moss | the grass photograph's grain and speck (browner in a seasonal winter) | 0.02, × 0.6 |
| 40 cropland | furrows 4 m apart on each field's own heading in half the fields, the grass grain at half strength | 0.012, × 0.7 |
| 50 built-up | under real buildings (a theatre's buildings layer, "Under real buildings" below): the photograph with the ground's mottle, flat, its yards gardens (grass) where it has colour and paving where it is grey. Elsewhere (the buildings off): blocks between asphalt streets (80 m parcels), each laid out in lots of about 15 × 11 m on its own heading, a roof on five lots in six (its own tone, one in three of tile) with 1.5 m of relief; no geometry | 0.015, × 0.7 |
| 60 bare | sand on the flat, rock with the photograph's structure on slopes (the style's `rock`) | none |
| 70 snow and ice | glacier ice where the season's snow has gone from it (`glacier_ice`, below), the season's snow and firn over it above the snowline; the bare ground's rock where a face is too steep to hold either (40-48°, the rock faces past 49°). The tropical and volcanic styles, which lay no season's snow, keep the snow style's snow | none |
| 80 water | inland (over 100-300 m from the sea): dark, smooth and glossy. Water the cover puts on our land by the sea is the coast missing the photograph's by a texel or two: the style's shore there | none |
| 90 wetland | darker and damp (roughness 0.6) | 0.08, × 0.6 |
| 0 unknown | the photograph with the grass grain | the style's own masks |

In the seasonal styles the climate's snow lies over it all (the snow style's snowline, ragged with the aspect and the
relief, lying in patches near the line until the thaw, thinner over woods), so a winter scenario gets snow on summer
photographs; the tropical and volcanic styles take none (a high peak's snow is in its photograph). On WorldCover's snow
and ice that snow is the firn above the snowline; below it the ice shows (`glacier_ice`): blue-grey (sRGB 150, 168,
180), greyer with the dirt the melt leaves on it, to 0.65 of that 900 m below the snowline, streaked along the flow by
bands (12 m by 600 m, ±17%); medial moraines along it (a noise in cells 150 m across and 12 km long over 0.68-0.73,
about one part in twelve of the ice: debris at sRGB 98, 93, 88, raised 2 m), drawn while a pixel is under some 30 m and
their share from 120 m; crevasses across it (cells 10 m by 200 m along the contour, dark blue, sunk 1.5 m), a few on the
flat and about one part in nine past 17° of the ice's own slope, where it steepens and stretches. Each fades to its
share (a logistic over the noise tile's values, `share_over`) as a pixel outgrows it, so a far icefall reads greyer, not
plain. The flow is the gradients' third mip (320 m): the window's own 40 m gradient turns with every hummock on ice so
nearly flat (the moraines lay as blotches at every heading), the fourth mip (640 m) took in the valley's walls and
turned the bands across the Aletsch's trunk, the third keeps them along it over its 1-1.5 km width. The ice is worked
out only where the class is present: a gradients read and six of the noise tile. Seen from 3.7 km over the tongue
(`swiss-alps-aletsch-tongue` in the land tour), the moraines run with the trunk in bands a few hundred metres to a
kilometre or two long, broken where the flow turns past one of the streaks' eight headings, and the steeper side
glaciers are hatched with crevasses. With `Climate::AUTHORED` (230 m) a chalk window would snow above 230 m: the theatre
passes its own climate. The trees (`tree_cover`, in their placement pass) take their density from the cover at the foot,
by the same coverage weight; since 2026-10-05 their species is the theatre's biome's for that cover (trees.md,
"Species"), and shrubland and grassland grow ground cover near the camera.

**Under real buildings** (2026-10-05, branch `goldflag/builtup-pattern-off`). The built-up lots were made before the
theatre had buildings or the map's roads, and drew on beside them: over Grover Beach from a plane at 280 m (the player's
California scenario) house-sized pink and tan rectangles with dark L-shaped edges (the roofs' 1.5 m of relief in the
sun) lay on the ground between the real houses, and a grid of grey streets crossed the real ones. The land is now told
when a buildings layer stands on it (`TerrainLayer::set_buildings_stand`, from `naval-client`'s `build_scene` whenever
it makes one: High and Medium with the buildings on; never with `NAVAL_BUILDINGS=off`), and the
class then draws the photograph with the ground's mottle, flat, the paving material under it on High: the houses are the
buildings, the streets the photograph's and the map's roads. A uniform switches it (`surface_haze.w`, `buildings_stand`
in `surface.wgsl`), so the lots' lookups are skipped there, not hidden. Without the buildings the photograph shows what
remains: NAIP's houses as soft roof-coloured patches with no edge, standing under the buildings raised on their
footprints, so they show only where a footprint is missing; the outlines were the lots'. The lots changed 9% of the
pixels at 280 m over the town and 5-8% at 120 m (High, Medium), none from a ship's bridge 2 km off Pismo Beach (they
have faded to their mean at that range) and none over the hills above Shell Beach. Frame times with the lots and without
(Apple M5 Pro, release, 1600 × 900, `measure_frames` medians of 40 over eleven turns, the same frames, other agents
building at a load average of about 20): 0.5-1.3 ms less without on High, -1.5 to +0.1 ms on Medium; no rise, a saving
within the load's noise. Checked: `--test all -- gpu::`'s `standing_buildings_take_the_built_up_lots_off` (a built-up map from
70 m: 5.2% of the pixels stand out from the ground about them with the lots, 0.2% without; the setting kept across new
maps; grassland the same either way); the stills,
`cargo test --release -p naval-client --test all -- theatre_built_up:: --ignored --nocapture` into `.build/native/built-up/`
(`high/` and `medium/`, each view with the lots, without them and without the buildings; `pairs/` the three stacked).

**Yards and standing trees** (2026-10-29, branch `goldflag/suburb-ground`). Seen from a plane at 90-300 m over Tampa,
Bay Ridge and Costa Mesa (the player's: "too arid", "no grass on the ground", "the ground texture is rocky"), with
today's buildings:

- *The yards.* Under real buildings every built-up texel took the paving material, an aerial photograph of asphalt
  whose stains (its ratio 0.56-1.62 at the 5th and 95th percentiles) speckled every lawn, and the photograph's hue: NAIP
  matched to Sentinel-2 over 64 m squares, whose mean in a town is its roofs' and concrete's, so a Costa Mesa lawn was
  brown-grey (albedo sRGB 64, 60, 57) and under the noon sun near white. Now a yard's texel with any colour (its chroma
  0.1-0.25 of its luminance, `gardens`) is a garden: the grass material (dry grass where the photograph is tawny, as
  the open land's; not by the climate's summer, the gardens being watered) and the grass photograph's grain. Grey
  texels (a car park, a yard of concrete) keep the paving. And NAIP now takes only Sentinel-2's brightness over each
  square and `naip::MATCH_HUE` (0.25) of its hue, in the logarithm (`match_ratios`), keeping the rest of its own: its
  0.6 m sees a lawn's green where Sentinel-2's 10-19 m sees roofs. Matching every channel over 256 m or 1 km squares
  instead changed the hue hardly at all (an offline match of the cached tiles): the warmth is the 2016 mosaic's, over
  the whole of a town. Pushing a garden's hue toward the window's leaves' and grass's class means did nothing in a dry
  window, whose means are brown too.
- *Under standing trees.* The canopy's painted crowns (`canopy`) stood under the trees' cards from above as well as
  beneath them, so a wood seen from a plane took them twice: lit lumps 5 m high between the cards, over NAIP's own 2 m
  crowns and their shadows, and a leafy suburb's woods (WorldCover's tree cover over Tampa's yards) read as a field of
  rocks. They now fade out by the trees standing drawn (`SurfaceInputs::standing`, `forest.wgsl`'s `cards_standing`:
  all of them within 4.5 km, thinning beyond), from above or beneath, and paint only the trees left out, far off and
  with no trees drawn (Medium).
- *Watered lawns.* NAIP over southern California was flown in the 2022 drought: Costa Mesa's gardens were olive-brown,
  near white under the noon sun. Where NAIP is the photograph (`RealSample::naip`: the share a near tile at its 2 m
  texel draws, `NAIP_TEXEL_MAX_M` 3, Sentinel-2's near tiles being 8 m or more, so only in the United States and
  within its few kilometres of the camera), a garden showing growth goes toward a watered lawn: its green over red
  ramped from 0.82 to 0.95 (a bare yard's red-brown below) and over blue from 1.0 to 1.25 (not a pool's or a shadow's
  blue; `LAWN_GROWTH`), 0.75 of the way (`LAWN_TINT`) to the hue of sRGB 85, 110, 55 (`LAWN_GREEN`) at 0.75 of the
  photograph's brightness (`LAWN_DARKER`), and not in a leafless season. Paving and bare dirt stay; such a lawn takes
  the grass material, not the dry grass. The gate is NAIP and not the climate's dry summer (`terrain.climate.w`),
  which Orange County shares with Taranto and Valletta, whose dry gardens stay as their photographs show them. Bay
  Ridge's pale yards take some green too (its photograph shows growth in them); Tampa's, green already, change little.
  Past the near tiles (and over an air window, whose near
  tiles are Sentinel-2's) a town's colour is its 10-19 m mean and no lawn is drawn.

`theatre_town_look`'s `tampa`, `brooklyn` and `orange-county` (`NAVAL_LOOK_BUILDINGS=today`, `NAVAL_LOOK_DEBUG=1` for
each view without the ground materials and as the albedo alone) into `.build/native/towns/<tag>/`.

**Costs** (Windows, NVIDIA GeForce RTX 5070 Ti, release, other agents building): `set_surface_maps` on a 2048² map, its
CPU mip chain, statistics and upload, 40-65 ms on the render thread (a 4096² one about four times that); memory 22 MB
of colour and 4 MB of cover at 2048², 89 MB and 16 MB at 4096². The shading adds two filtered colour samples, four
cover loads, a few noise lookups, the canopy and the lot pattern a pixel, and the style's own cover still ran beneath
(it shows where the imagery ends): whole frames (sky, sea, land with 256 tree patches, 1280 × 720, median of 24
serialised) took 2.0-2.5 ms with maps against 1.4-1.8 ms without over the same four views near Calais, about 0.5-0.7 ms
for the maps. Since 2026-10-05 each class's pattern is worked out only where its class is present, and the style's
cover only where the photograph does not paint the point whole ("The land's shading cost" below).
Near tiles and the magnified rebuild (2026-10-04, the theatre's cruise scene off Reykjavík, 1280 × 720, median of 40
`measure_frames`, other agents building): near tiles on against `NAVAL_SURFACE_NEAR=off`, +0.15-0.35 ms a frame
(3.42-3.70 against 3.10-3.18 ms at 150 and 300 m, 2.86-2.88 against 2.66-2.72 at 1,000 m, 2.45-2.48 against
2.27-2.34 from a ship); against the commit before them (which also lacks the trees' shadows of the same day)
+0.3-0.6 ms. Memory: the second layer doubles the textures when a window takes near tiles (178 MB of colour and 32 MB of
cover for a sea window's 4096², 42 and 8 MB for an air window's 2048²); a window taking none keeps one layer.

**BC7 colour** (2026-10-09, `mem-gpu-assets`). On a device sampling BC the colour array is `Bc7RgbaUnormSrgb` (a
quarter of the bytes: 45 MB of colour for a sea window's two 4096² layers in place of 178 MB, 11 MB for an air window's)
with 8 levels rather than 10, the smallest a slot's 4 × 4 texels (one block; the sea window's array's last level is
32²), so every slot's every level is whole blocks. The window's maps are encoded as they are handed in
(`set_surface_maps`, on the scene's build thread: 0.2 s for a 2048² map and its levels on four threads, 0.75 s for a
4096² one, M5 Pro with other agents building), the near tiles on the near worker as they are made
(`NearTile::from_maps_bc7`, about 15 ms a tile; a tile kept in memory is 0.6 MB rather than 1.66), and a tile made in
RGBA8 is encoded as it is uploaded. The shader reads the same sRGB values (the
coverage in alpha, the premultiplied colour), so it is unchanged. A device without BC, a wasm32 build and maps whose
side is not a multiple of 512 texels keep RGBA8 with 10 levels. GPU test: `near.rs` (on a BC device and on one
without).

**Checked.** The GPU test `surface_maps_paint_the_land_where_their_imagery_lies` holds that maps with no imagery draw
the procedural land bit for bit, that imagery shows, and that clearing restores it. `tests/near.rs`'s
`near_tiles_paint_over_the_window_maps_where_they_are_drawn`: a grid alone draws the procedural land and a grid with no
tiles the window's maps, bit for bit; a tile with no imagery leaves the window's maps; a tile drawn shows its colour
over them; another tile in its slot gives the point back to the window's maps; a grid after the maps, or of another
size, is refused.

**Open.** The photograph is a blur within a few hundred metres at the client's 23 m a texel, where no near tile
paints. `set_surface_maps` builds its mip chain on the render thread (a worker could prepare the levels, as
`PreparedLand` does). The window's edge, where the maps end, is a hard change to the style's colours.
## The land's tone

The land read too saturated and too bright against the sea and sky. Every land colour, the styles, the trees' cards and
an airfield's clearing go through `tone_down` (`surface.wgsl`) before it is lit (the real-world surface did until
2026-10-08, when its calibration took over): 30% less saturation (`LAND_SATURATION` 0.7, toward its luminance in
linear light) and 14% darker (`LAND_BRIGHTNESS` 0.86), snow and chalk keeping their brightness (the darkening fades out
between luminance 0.35 and 0.6). The seabed, the wet band and the foam line are not toned.

The tree atlas's mip chain is premultiplied now too (`trees.wgsl` divides by the alpha): the straight-alpha chain
averaged the empty texels' black into the crowns' edges at a distance. (Since 2026-10-05 the impostor atlas's colour
layers, mipped on the GPU, are premultiplied the same way.)

## Seasons, climate and tiling

Each style was authored for one place and season: snow as Vestfjord in April 1940 (snow to a 230 m line and lying in
patches below it, birch bare), chalk as Kent in February (winter farmland, bare woods). A theatre's coast can stand
anywhere at any date (Iceland and Lofoten in late May, Malta in summer), so the land takes a climate
(`naval_conditions::Climate`, the `climate` uniform): the snowline (m), the thaw below it, the leaves out, a dry summer (0-1).
`Climate::AUTHORED` (230 m, 0, 0, 0) draws every style exactly as before, and a land with no place (the tests') keeps it. The theatre
computes one from where and when its scene stands (`naval-client` `theatre::land::climate`, native/docs/theatre.md):

- **Snow.** Snow lies above the climate's snowline; the thaw clears the patches below it and greens the heath (grass
  and moss, browner toward the snow); the birch comes into leaf with the season (a canopy on the ground, broadleaf
  cards for a share of the trees as `leaf` grows). The patches thin out over a broad band of their measure
  (`lying_snow`): the drifts move it (a 420 m noise, some 0.05 either way), the share of the ground the patches cover
  rises from none 0.2 under its middle to all of it 0.2 over, and the patches stand where a fine noise (40 m, its cells
  some 5 m across) is over its quantile for that share; as a pixel grows past them the share itself shows, a thinning
  cover. So from 1-3 km the thaw is a soft edge of finely broken patches, along no contour or line; the measure is
  lowered 0.05 so that as much snow lies as before (in six views of Kirkenes, Narvik and Tromsø on 23 May, 0.604 of
  the ground it could cover against 0.599 with the hard cut, its edges about twice as wide). Until 2026-10-27 the patches ended within 0.06 of their measure:
  at Narvik in late May a hard edge ran some 2 km straight across a wooded slope. That morning's first fix added the
  420 m noise and an 85 m one to the measure and kept the hard cut, and Kirkenes' snow broke into hard-edged black
  blots 100-200 m across (with the snow off the ground under it is evenly dark at a 2.7° sun; with the noises off they
  fell back to the earlier, smaller patches). The horizon's far land keeps the plain rule.
- **The summer's snowline and steep faces.** The snowline rises from `SNOWLINE_66_M` at 66° N by 80 m a degree toward
  the equator in midwinter, as authored, and 100 m in midsummer, between them with the season. The summer's line is the
  glaciers' equilibrium line, the snow's highest by late summer: about 1,200-1,600 m in western Norway at 61-62° N
  (Ålfotbreen about 1,200 m, Nigardsbreen 1,550 m) and 2,900-3,100 m in the Alps at 46.5° N (the Great Aletsch's
  2,900-2,950 m), about 100 m a degree between them. The Bernese Oberland snows above 2,956 m on 20 July 1943; western
  Norway's mid-August line is 1,524 m, Geiranger's on 8 June 1,063 m; Fuji (35.4° N) is bare in August (4,119 m). Steep
  faces shed the snow (`snow_shed`): in winter from 35° to 45°, in the melt from 30° to 40°, the wet snow sliding off
  and the dark rock warming through it. The melt is the season's (`snow_melt`: the climate's leaves, which come out as
  it starts and fall as it ends; none for `Climate::AUTHORED`), up to 300 m over the snowline and gone 900 m over it,
  where the cold keeps the north faces' ice as in winter. The snow style's bare faces between the couloirs start 0.1
  lower in the melt, and the horizon ring sheds its snow by the same rule. The window's 40 m cells draw a 50-60° wall at
  some 40-45°, so these slopes are the drawn land's, not the rock's. - **Chalk** (and `rock`). The leaves out: the downs
  and fields grown green (pasture, young wheat, rape in flower, hay), hedgerows and woods in leaf, broadleaf cards. A
  dry summer: tawny grass with grey-green garrigue over the rough ground, smaller walled fields (130 m, not 260 m) of
  stubble, red earth and olive groves, pale stone walls for hedgerows (and fewer trees on them), low dark olive and holm
  oak, olive groves in rows over one field in six.

**How dry, from the climate map.** `dry` is the Köppen-Geiger classes of the land within 40 km of the scene (ground.md,
"Climate and rock maps"), each class by its share of the land's cells (`theatre::land::class_dry`): BW 1.0 all year, BSh
0.8, BSk 0.65; Csa 1.0 in summer (Malta, Crete, the Levant), Csb and Dsa 0.7, Csc and the other Ds 0.5; Cw and Dw (not
the tropics' Aw and Am) 0.35 in winter; the rest none. Summer and winter are a ramp (full from early May to
mid-September, none mid-November to mid-March; half a year on south of the equator). Where no land lies within 40 km, a
Mediterranean box (30-44° N, 10° W-37° E) in summer and a desert box (14-31° N, the Sahara to the Gulf) decide (the open
Ionian Sea in July is dry). Measured with the map: Malta in May 1.0, in January 0; Tobruk in January and Alexandria in
July 1.0; the Grand Canyon on 6 June 0.35-0.75 (test; the land within 40 km is 58% BSk, the forested rims Dfa, Dfb and
Dsa); Taiwan, Kent, Tokyo, Iceland 0.

**The style from the climate map.** A theatre's style (its palette, and the `STYLE_*` shader defs: one for the whole
scene, compile time) is the latitude's band (`LandStyle::for_latitude`: tropical below 24°, chalk to 45°, rock to 58°,
snow beyond), except where (`LandStyle::for_place`, `naval-client` `theatre::land`) the Köppen-Geiger class of the land
nearest the scenario's origin (the commonest of the trees' classes within 1.5, 5, 15, 40 or 100 km, the first that holds
land) says it is wrong: A (the wet tropics) or Cfa and Cwa south of 27° (Taiwan, the Ryukyus, Fujian: evergreen, so no
winter brown) are tropical; E alone (Greenland's coast, Kerguelen, South Georgia) is snow. The six stock scenarios keep
their bands' styles (Cape St George, the Eastern Solomons and the light carrier duel tropical, the Denmark Strait and
Glowworm snow, Ile de Batz rock); of the land tour's places only Taiwan's differs from its band's (tropical, not chalk).

**Tiling.** The surface read one 512² noise tile and the detail photographs at fixed periods (the speck's 47 m, the
grain's 210 m, the photographs' 12-190 m), and from the air each showed its period as a lattice over kilometres (the
"dashes" over the Solomons, a diamond pattern in Iceland's snow). Every value lookup is hex-tiled now (Mikkelsen,
"Practical Real-Time Hex-Tiling", JCGT 2022: `hex_taps`, `noise_r`, `detail`): the plane cut into triangles about 0.6
of a tile across, each corner reading the tile at an offset hashed from the corner (pcg2d), the three readings blended
by the point's place in its triangle with their contrast kept (about the tile's mean, divided by the weights' root sum
of squares), with the tile's own gradients so no seam shows. The patterns whose cells must stay whole (a parcel's id
and boundary, a crown's dome) read the tile whole at the nearest corner's offset (`noise_whole`): the lattice's edges
cut them as a field's edge or a gap in the canopy would. Three taps for one on about a dozen lookups a pixel; not
measured.

The tree cards stood in diagonal rows from the air: each slot's jitter came from the web's
`fract(sin(dot(cell, k)) * 43758.5)`, which runs out of f32 precision at cells some thousands from the chart's corner
(a theatre's 240 km window has them by the million) and correlates along lines. `cell_hash` is an integer hash now
(pcg2d), and the jitter spans the whole slot (it kept a tenth of the slot clear on each side, a grid in all but name
under a dense canopy).

**The shore.** The wet band and the foam line drew wherever the land lay under 1.5 m on a gentle slope; on a low coastal
plain (Reykjavík's, 1-5 m) that was a field of surf hundreds of metres wide. Both now stand only within 25-90 m of the
coast (the attribute grid's signed coast distance).

## Map features on the ground

Roads, railways, rivers and canals from
OpenStreetMap drawn crisp on a theatre window's land, from a plane and from a ship inshore: the host
(`naval-client` `theatre::features`, docs/archive/native-theatre-history.md, "Map features on the ground") rasterises
the ways about the camera into distance-field pages and hands them in; the layer draws them in `shaders/features.wgsl`,
which `surface.wgsl` calls once for the land's colour (`feature_cover`, after the real-world surface and before the
land's tone) and once for the trees (`feature_clearance`: no tree on a road, a railway or a river).

```rust
land.set_feature_grid(&mut ctx, FeatureGrid { origin: [o0, o1], texel_m: [2.0, 8.0], slots: 8 })?;
land.queue_feature_page(level, [i, j], Arc::new(FeaturePage::rasterize(corner, texel_m, &strokes, &areas)));
land.holds_feature_page(level, [i, j]); land.feature_page_counts(); land.show_features(false);
land.flush_feature_pages(&mut ctx);  land.clear_features(&mut ctx);
```

**Why distance fields.** A road is 3 to 13 m wide; the window's maps are 23 m a texel and the near tiles 8 to 19 m, so a
road rasterised into colour there is a smear, and one finer colour layer crisp enough for a ship's eye (a metre a texel)
over the camera's surroundings would be gigabytes. Analytic segments in a GPU buffer are exact at any distance but cost
a loop over a cell's segments a fragment (a town's 64 m cell holds 10 to 40), against the budget of one or two texture
reads. A signed distance field keeps the edge at a fraction of a texel however magnified (bilinear filtering of a
distance is linear across a straight edge), so 2 m texels draw a 7 m road with a sharp edge a metre away, and the
distance's own screen derivatives give each band the filter width across it at range: two filtered reads a land pixel.
Measured: eight pages rasterised in about 3 ms on the worker; the frame cost below.

- **Pages.** `features::PAGE_TEXELS`² (256²) texels, two levels: 2 m (512 m a page) and 8 m (2 km), each `slots`² pages
  about the camera (8 × 8: 4 km and 16 km) in one `Rgba8Unorm` array texture at the terrain group's binding 14 (four
  layers of 2048²: level l's distances in layer 2 l, its attributes in 2 l + 1; 64 MB). The binding is in the layout
  only on a device allowing 17 sampled textures a stage (the land's fragment stage samples 16 without it, WebGPU's
  default; the client's devices ask for 24): elsewhere the land is built without it (the `FEATURES` shader def unset)
  and a grid is refused. Page (i, j) lies in slot
  (i mod slots, j mod slots), read repeating, as the near tiles are; a slot draws only where its entry in
  `terrain.feature_slots` names the page at the point, and level 1 feathers over its outer 15% toward a page not drawn.
  The layer uploads queued pages from its `prepare`, 4 MB a frame (eight pages), changing the slot's entry in the same
  frame.
- **What a texel holds.** Distances in metres to the nearest paved road's edge, unpaved road's edge, railway's middle
  line and water's edge (negative on the feature; rivers and canals as lines with a half width, riverbanks and docks as
  areas, filled by scanline), each `0.5 + d / (2 R)` over `R` = 8 texels (16 m at level 0, 64 m at level 1; 6 and 25 cm
  a step). The union of overlapping features is the minimum distance, so junctions of roads of different widths join
  cleanly. The attributes: the nearest paved road's half width and tone (its class), the unpaved road's half width,
  the water's half width (an area's the most, 32 m). `FeaturePage::rasterize` walks each segment's capsule row by row
  (each row's span solved exactly), so a page costs its ways' length, not its area.
- **Drawn** (`feature_cover`): one level a pixel, chosen by `select` (level 0 where its page is drawn and a pixel spans
  under three of its texels), its two reads and each distance's `dpdx`/`dpdy` taken by `feature_site` at the top of the
  land's fragment stage, in uniform control flow (the surface after it runs in branches); the derivatives give the
  box filter's width across its band (`feature_band`: the overlap of the pixel and the band); a road far narrower than a
  pixel fades to its share of it instead of a bold line. Water: dark and glossy (roughness 0.12, full grazing
  reflection) between muddy 2.5 m banks; unpaved: earth half the ground's own colour; paved: asphalt from a light worn
  grey (minor roads) to dark (motorways), with white edge lines on the major roads where a pixel can hold them;
  railways: a 3.2 m grey-brown ballast with a dark steel rail pair at standard gauge where a pixel can hold it. Only on
  land (faded out between 0.6 and -0.2 m: a pier or bridge over the sea is not drawn on the seabed). The bump under a
  road flattens with its cover. Everything then takes the land's tone, as every land colour does.
- **Against the photograph.** Where Sentinel-2 shows a road it is a blur 10 to 20 m wide; the drawn road sits inside it
  and the blur reads as the verge. OSM and the 2016 mosaic agree within a few metres over Santa Cruz (no doubling seen in
  the captures).
- **Trees** (`feature_clearance`, in the placement pass): a tree is not placed within 2 m of a road's or a river's edge or
  of a railway's ballast. The pages arriving re-place the trees, as the near tiles do.
- **Hidden** (`show_features(false)`) the uniform's flag is 0 and the shader skips every read, as with no grid: the
  before-and-after captures and the frame measurement use it.

**Costs** (Apple M5 Pro, Metal, release, 1280 × 720, 2026-10-05, the machine loaded by other agents' builds, load
average 25-35; the theatre's cruise scene off Santa Cruz in the California user scenario, every stream on;
`theatre_features`' ignored test: the median of five alternating 30-frame `measure_frames` medians with the features
shown and hidden): from a ship's bridge 2.5 km off +0.17 and -0.01 ms (two runs) on 9-12 ms frames, the same through a
12° lens -0.00 and +1.36 (an outlier of the load), from a plane at 150 m over the town +0.10 and +0.29, at 300 m +0.04
and +0.25, at 1,500 m off the coast +0.13 and -0.60; a quieter run (2026-10-05, the NAIP branch with NAIP
off) +0.17, +0.10, +0.17, +0.04 and -0.02 in the same order; a run under heavier load scattered by several
milliseconds either way and is left out. So about 0.0-0.3 ms, at the 0.2 ms line: the host draws them on
the High terrain setting only. GPU memory 64 MB (the texture), CPU up to 96 MB of made pages (192 kept).

**Checked.** `--test all -- features::`' `the_land_draws_on_a_device_with_no_room_for_the_features`: on a device with WebGPU's
default limits the land builds and draws without the binding, and a grid is refused. `cargo test -p naval-terrain --lib features`: a capsule's row spans exactly the texels within its reach
(diagonal, horizontal, vertical, a point); a 7 m road drawn as its band, its half width and tone in the attributes, a
railway's middle line, a riverbank square with an island (inside negative, the island and outside positive), an empty
page where nothing reaches. `--test all -- features::` (GPU): a grid with no pages and a page with nothing on it draw the land
as before (the Medium setting, whose land has no trees: two placements of the trees differ by a few overlapping cards),
but for at most 0.2% of the pixels by up to 2 levels (on Apple's GPUs 2 runs of the crate's tests in 46 moved 51 pixels
a level, nearly all in a 2 × 2 quad's right column, as in trees.md, "The light's discard"), a page of wide roads paints grey
bands, clearing restores the land (`.build/native/terrain/features-gpu-test.png`). The look in the theatre: docs/archive/native-theatre-history.md, "Map features on the
ground".

**Open.** Lakes and ponds are not drawn as water: a land release carries no lake levels (`docs/archive/native-ocean-history.md`,
"Lakes and ponds (removed 2026-10-17)"). The infill's generic houses
(buildings.md) stand where they will, roads or not (the roads paint over the built-up lots where
no buildings stand); the infill could take its streets from these ways. No lane markings but the major
roads' edge lines (a centre line wants the country's colour and the way's direction). A bridge over the sea is not
drawn; nor are aerodromes' runways (`aeroway`) yet.

## The land's shading cost

The owner, of the theatre's land: "is nuking my fps". At the owner's settings (High, 3456 × 2160 drawn at 75 %) the
land doubled the frame, and nearly all of it was the land's fragment stage (`fs_land`, in the scene's opaque pass): the
photograph's surface (`real_surface`) worked out every cover class's pattern at every pixel; the style's own cover ran
beneath it to be painted over; the window's maps were read, warped and rebuilt under near tiles that painted the point
whole; and the seabed under the sea (53 % of the frame from a ship off Grover Beach) took the land's whole surface, only
for the seabed's colour to be painted over it. The shadow casters, trees, ground cover, buildings, ground materials,
roads and lakes each cost under 1 ms. Now:

- **The seabed** more than 3 m down (`SEABED_PLAIN_M`) takes `seabed_surface` alone: the style's sand darkened with depth
  and its detail photograph's grain, rock where the floor is steep, the wet sand and rock materials close up, the wet
  band's darkening by the shore, the shore's roughness (0.85) and grazing reflectance (0.5), the open sky's occlusion,
  no relief. What the full surface added there was the land cover's relief under the water (a tropical canopy's crowns,
  a cover class's rock), and its roughness. That relief fades out between 2 and 3 m down (`SEABED_RELIEF_M`), so no
  seam shows along the depth line (at 2 m with no fade, a reef's lumpy patches stopped at a hard edge); shallower than
  2 m nothing changed.
- **Each class's pattern only where its class is present** (`real_surface`): the canopy (woods, mangroves, shrubland),
  the fields' furrows, the lots (built-up land with no buildings standing) and the rock (bare ground). A class absent at
  a point weighs nothing there.
- **The near tiles first** (`real_sample`): where a near tile paints the point whole (its share and its imagery's
  coverage both 1) the window's maps beneath are not read (their samples, their magnified rebuild, their cover), and each
  layer's magnified warp is worked out only where the layer is magnified.
- **The style under the photograph only where it shows** (`terrain_surface`): where the photograph paints the point
  whole (coverage 1, a slope gentle enough for a photograph from above, no water class by the sea, where the style's
  shore shows through) the style's own cover is not worked out.
- **What lets them branch.** No read in `surface.wgsl` takes screen derivatives of its own: each filtered read takes its
  level from the chart point's derivatives (`SurfaceInputs::chart_dx`, `chart_dy`, taken once in the fragment stage)
  carried through its pattern's turn and scale (`tile_uv`; `surface_read` turns `textureSampleBias`'s level shift into
  scaled derivatives), and the map features take their distances' derivatives in `feature_site`, which the fragment
  stage calls before any branch. naga lifts the derivative-uniformity rule for fragment shaders, so no pipeline build
  checks it: `tests/backends.rs` holds `surface.wgsl`, `materials.wgsl` and `hex.wgsl` to it, and writes the land's
  shaders out as Metal (MSL), DirectX 12 (HLSL, shader model 6.0) and Vulkan (SPIR-V) code for each style and setting.

**Cost** (Apple M5 Pro, Metal, release, 2026-10-05, load average 1-6 with no other GPU client busy; the owner's
graphics at 3456 × 2160, render scale 75 %; the player's California scenario, Force A 6 km off Grover Beach; the GPU's
busy time, each pass's serial cost summed, median of 120 frames in four batches, the frames waited for in threes; a
local harness on the branch `goldflag/land-perf-probe`, not in the tree). Before is origin/master's land in four runs,
after in two; another lane's harness on the machine slowed a run by up to half when it woke, and those runs are left
out.

| View | Before | After | Frames a second |
| --- | --- | --- | --- |
| a) a ship's chase 6 km off Grover Beach, looking at the town | 33.2-34.4 ms | 19.5-19.6 ms | 29-30 → 51 |
| b) 280 m over Grover Beach, looking down at the town | 25.2-26.5 ms | 14.4 ms | 38-40 → 69 |
| b2) 280 m over the shore, along the coast toward Pismo | 35.5-36.4 ms | 18.5 ms | 27-28 → 54 |
| c) a patrol's scene (the air window), 1,500 m over the bay, toward the town | 27.8-28.4 ms | 16.5 ms | 35-36 → 61 |
| c2) the same, along the coast (sea and land each half the frame) | 29.5-30.1 ms | 15.7 ms | 33-34 → 64 |
| d) open sea, no land | 14.4-15.2 ms | 15.0-15.1 ms | 66-69 → 66-67 |

The land costs at most 4.5 ms of a frame where it cost 11-21 ms (from 280 m over the town, none: it hides as much sea
as it costs). What is left of it in view a: the scene's opaque pass 4.2 ms against 1.3 ms with no land and 1.9 ms with
its fragment stage flat. The render thread's CPU is unchanged (1.4-2.1 ms a frame).

**Patterns faded out are not read** (2026-10-05, `goldflag/land-shader-fades`). Every pattern of the surface fades to its
mean as a pixel grows past it (`resolved`, `textured`), and each was still read and multiplied by nothing. Now its reads
are skipped where its fade is exactly 0, so the picture is the same to the bit: the detail photographs' grain (grass to
7.5 m a pixel, sand to 15 m, snow to 30 m; `detail_shown`), the mottle and the fine grass grain (2.65 m and 1.85 m), the
canopy and its leaf clumps (half a crown, a sixth of one), the fields' furrows (2 m), the lots (13.5 m), the rock's
structure, relief and streaks (15, 4 and 6 m), the crops' tint, the fall-line streaks, and the foam's froth (off its
band, or past 4 m). The ground materials already stopped reading past their fade. The land's shadow caster draws depth
alone (no fragment stage), and the trees' caster blends three impostor frames whose weights are seldom 0: neither had
anything to skip. From a plane most of the land is seen at 2-20 m a pixel, where most of these patterns have faded.

| View (the same scenario and settings as above) | Before | After | Frames a second |
| --- | --- | --- | --- |
| a) a ship's chase 6 km off Grover Beach | 19.5-19.6 ms | 19.2-20.6 ms | 51 → 49-52 |
| b) 280 m over Grover Beach | 14.4-14.8 ms | 13.4-14.8 ms | 68-69 → 68-75 |
| b2) 280 m over the shore | 18.5-19.0 ms | 17.1-17.9 ms | 53-54 → 56-58 |
| c) a patrol at 1,500 m over the bay | 16.5-19.8 ms | 14.0-14.8 ms | 51-61 → 68-71 |
| c2) the same, along the coast | 15.6-17.2 ms | 14.0-14.4 ms | 58-64 → 69-71 |
| c3) a plane at 500 m, 2 km short of the town, looking down at it | 14.9-16.3 ms | 13.2-13.7 ms | 61-67 → 73-76 |
| c4) a low pass at 150 m over the town, looking 1.2 km ahead | 14.9-15.7 ms | 13.5-14.2 ms | 64-67 → 70-74 |
| d) open sea | 15.0-15.5 ms | 15.0 ms | 65-67 |

(Apple M5 Pro, release, 2026-10-05, runs with no other GPU client busy, load average 2-4; each a median of 120 frames.
The scene's opaque pass, where the land shades, fell 10.6 → 8.3 ms at 1,500 m, 9.4 → 7.7 ms at 500 m, 9.4 → 8.1 ms
at 150 m.) The stills (Grover Beach from a ship, from 280 m, along its shore, its shallows from 600 m and 120 m, a plane
over the coast at 1,500, 500 and 150 m, Big Sur's cliffs, Leyte) before and after are identical to the bit.

History (measurements, stills and the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
