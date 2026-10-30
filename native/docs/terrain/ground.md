# Ground materials, cliffs and fine relief

The ground within a kilometre or two of the camera: photographed materials tiled under the land's colour, rock on
cliff faces, and finer heights than the window's grid.
## Ground materials

From a ship or a low plane the satellite photograph (10-19 m a pixel outside the United States, NAIP's 2 m inside) or a
style's procedural paint is soft. From 30 m to 1,500 m the land tiles photographed ground materials under its own
colour: they add structure and relief finer than the photograph's pixel, and never their colour, so the photograph (or
the style) keeps the land's colours and their calibration, and nothing changes from about 1.5 km.

```rust
let materials = GroundMaterials::from_sources(&jpegs, 512)?;          // naval_terrain::ground_materials (RGBA8)
land.set_ground_materials(&mut ctx, &materials)?;                     // or PreparedLand::with_ground_materials(arc)
land.takes_ground_materials(); land.has_ground_materials();
land.show_ground_materials(false); land.set_ground_material_strength(1.3);
```

**The materials** (`src/ground/ground_materials.rs`, `GROUND_MATERIALS`): grass, dry grass, ploughed soil, scrub, dry stony scrub,
forest floor, rock, cliff rock, sand, wet sand, shingle, snow, mud, paving and fine sand, then the cliffs' limestone,
basalt and granite ("Cliffs") and the forest floor's needles (a conifer wood's: "The ground close up"), each a Poly Haven
texture under CC0
(sources, authors and links: `public/terrain/ASSETS.md`, "Ground materials"). Aerial photographs (12-30 m a repeat) for
the ground seen from 30 m to 1,500 m, where a 0.1-3 m feature is what a pixel can hold; close-up ones (soil, shingle,
snow, mud) at 5-10 m; and three seen from a man's height, the forest floor's leaf litter and needles (2 m a repeat)
and the fine sand of the tropical and volcanic beaches (3 m), a few millimetres a texel as the nearest ground's pixels
are.
`tools/ground_materials.py` fetches each one's 1K colour, OpenGL normal and displacement maps and writes two JPEGs a
material into `public/terrain/materials/` (14.9 MB in all, 512² for the close-up ones, 1024² for the aerial ones and the
three seen from a man's height): the colour as a ratio to its own local mean in linear light, halved (128 the mean;
everything wider than about a third of the tile taken out by a wrapping Gaussian, so no tile-sized blotch repeats; the
chroma at 40% of its variation; the contrast held between 0.10 and 0.32 in the logarithm; the forest floor's leaves 45%
and up to 0.75, its needles 35% and their own 0.42, squeezed toward the mean to fit the bytes: "The ground close up"), and the normal's x and y with the height
(high-passed alike, its 1st and 99th percentiles at 0.05 and 0.95).

**On the GPU** one array texture, two layers a material (layer 2 k the colour ratio and height, 2 k + 1 the normal),
bound at the terrain group's binding 15 for the land's fragment stage only (`bindings.wgsl`; the `MATERIALS` shader def).
The layer takes it on the High setting (`TerrainQuality::materials`) on a device allowing one more sampled texture a
stage than the land without it (17, or 18 with the map features; the client's devices ask for 24); Medium neither binds
nor compiles any of it. Each material's period, normal strength and contrast are a row of the uniforms
(`material_table`). Three ways to the array:

- *The pack* (`naval-assets`, `Kind::GroundMaterials`, format 6; native/docs/assets.md): 1024², every level BC7, about 48 MB on
  the GPU and 45 MB on disk with the cliffs' three rocks and the fine sand (37 and 34.3 MB without; "Cliffs"; the fine sand
  2.6 MB of each, the forest floor's sharper source nothing: every layer is 1024² in the pack), and the needles 2.7 and
  about 2.5 MB more. naval-client makes it when a land first takes it (`assets::GroundJob`, made with a theatre scene's
  land), decoded to RGBA8 on a device without BC, and keeps it only while a
  land holds it (`TerrainLayer::hold_ground_materials`): the port and a scene at sea do not keep it.
- *A checkout without a current pack*: the JPEGs made into RGBA8 at 512² (53 MB on the GPU with the needles, 39 MB
  without the cliffs' rocks, the fine sand and the needles; a quarter of the packed texels for the same memory), kept as
  the pack's are.
- *The stills and the tests*: `GroundMaterials::from_sources` at any side (the stills RGBA8 at 1024², 212.5 MB: an upper
  bound on the cost, `--material-side`).

**Choosing** (`surface.wgsl`, `materials.wgsl`'s `Mats`: a weight for each of 16 slots, laid as the land's cover is, so
each mask that paints a colour names the material under it):

- *The styles* (a theatre's land where its photograph does not reach, and the stills' test land): tropical leaf litter
  where the rainforest's trees stand (`tropical_stand`, the trees' own `tree_cover`, its edge as soft as theirs), grass
  on the open ground about it, under the palms along the shore, on the kunai and between the plantations' rows, scars on
  rock, beaches on fine sand; volcanic litter where the forest's and the groves' trees stand (`volcanic_stand`), grass
  on the open ground and under the palms, paddies on mud, steep rock, black fine sand; chalk downs on grass (dry grass
  and stony scrub in a dry summer), ploughed fields on soil, hedgerows on scrub, woods on leaf litter, cliffs on cliff
  rock, shingle and the bays' sand; snow heath on scrub (grass where it has thawed), rock on the slopes, snow, birch in
  leaf on leaf litter, bare faces on cliff rock, the tidal band on shingle.
- *WorldCover* (a theatre window): tree cover leaf litter, mangroves mud and leaf litter, shrubland scrub, grassland and
  moss grass, cropland grass or ploughed soil where the photograph is brown, built-up paving, bare ground rock on slopes,
  sand within about a kilometre of the sea and stony dry ground inland, snow, inland water none, wetland mud. Grass and
  scrub turn dry by the climate's dry summer or where the photograph is tawny rather than green (summer-brown California,
  stubble). Laid over the style's by the maps' weight, as the colour is.
- *Over it all*: faces steeper than about 40° (slope 0.75-1.1) cliff rock (on a theatre's land on High, its rock faces:
  "Cliffs"); the shore band (within 30-80 m of the sea and
  3-6 m of its level) sand on the flat, shingle steeper and rock steeper still, by the style (the chalk and snow coasts'
  shingle, the tropical and volcanic coasts' fine sand, sand elsewhere and wherever a photograph paints the beach); the
  swash band's sand (either) wet sand; the seabed wet sand,
  rock where it is steep; the season's snow.

**Blending.** The two heaviest are read, the third's weight taken from both (where the second and third trade places
neither shows) and a second under a tenth of the pair faded out (most land carries a little of another); they meet by
height (each weight raised by half its height, the lower fading over 0.25 below the higher), so sand fills between
pebbles and grass grows up through rock instead of fading into it. Each is hex-tiled (`hex.wgsl`, as the noise and the
detail photographs are, on a heading of its own; one round of the corner hash; at most two corners read, the lightest
corner's weight taken from all three so none switches with a seam), sampled at the level its pixel's footprint asks for
(the geometric mean of its axes; no anisotropic probes, a third of the cost on grazing views), its normal from the
corner that weighs most, faded flat where the two heaviest come level. The land's colour is multiplied by the blend's ratio before
the roads and the land's tone (`features.wgsl` stays on top; a road's own colour takes no grain), the seabed's likewise,
and the blend's normal is a slope added to the land's gradient (`terrain.wgsl`), flattened under roads and the foam. All
of it scales by a strength (1.3, `MATERIAL_STRENGTH`) and fades out as the pixel's filtered footprint (the longer axis
over the sampler's 8× anisotropy, or the shorter, whichever is more) grows from 0.5 to 1.6 m: gone by about 1.5 km at the
game's 52° lens on a 900-pixel view, sooner at a grazing angle, further through binoculars. A pixel past the fade reads
nothing and its colour is multiplied by exactly 1.

**Beaches.** Where WorldCover puts water on the land by the sea (the coastline a texel or two off the photograph's), the
land drew the style's shore: Santa Cruz's Main Beach in chalk-style cobbles. Now it is the photograph's beach where the
photograph shows one there (bright and grey, `beachy`), the style's shore only where it does not; and the shore band's
materials are let in only where the photograph looks like a beach (or where the style paints the land). So a sandy beach
reads as sand to its swash, wet sand there, meeting the existing wet band and foam line and the ocean's own shore foam
unchanged; a steep shore as rock.

**The ground close up** (2026-10-27, branch `goldflag/ground-textures`). Measured first with the albedo view (`--debug
albedo`, each still's ground read back against its strip of known albedos) on the Kawah Lagoon and Strait of Dover
battle maps' land, then fixed where each fault was made. Both maps went the same day: the views below are their record,
and Kawah's own palette and plantation crowns went with its map:

- *Kawah Lagoon's palm ground* was a chessboard of light and dark 4.5 m squares from 50 m and from a man's height: the
  tropical style painted its plantations' palms as the product of two sines on the chart's axes (a quarter of the
  colour either way and 4 m of relief), from before the trees stood. The fix (none painted with the trees drawn, only
  the ground between them; on Medium round crowns on a 9 m grid turned off the axes, `palm_crowns`) went with the map:
  the rows draw as before, in a theatre only off its photograph.
- *Its beach* was grey-brown dirt (0.081), and so was the shore band under the palms behind it: the map's palette held
  a dark volcanic sand (`#5a554c`), and close up the sand was an aerial photograph of a pebbly beach magnified
  (`coast_sand_01`, 15 m to its 1K texels: 1.5 cm a texel and its 1-2 m wet patches as blotches). It became coral
  sand, `#b8ab8e` (0.36 dry: coral sand measures 0.35-0.5, a tan quartz beach 0.25-0.35; the swash band 0.55 of it), a
  colour that went with the map. The fine sand under it stays (`sand_03`, 3 m a repeat, 3 mm a texel), to the swash,
  where it turns to wet sand: the tropical and volcanic styles' beaches take it under their own colours. The seabed
  takes the palette's colour as before (its shallows now over pale sand).
- *The forest floor* was near black (0.035-0.053, green) with a blue-grey sheen, blue-white sunflecks and hard-edged
  patches of light and dark ground: the land drew under the standing trees the canopy it paints for a view from above
  (its crowns' colour, shading, per-crown tint and 5 m of relief: the patches are the crowns' cells), lit by two-thirds
  of the sky and reflecting it. Now, seen from beneath the crowns (`floor_seen`: where trees stand, the eye below their
  tops, none from 2.5 times their height over that ground), the cover's ground is leaf litter, brown at 0.09-0.12
  (`litter_color`; litter measures 0.08-0.15), rough and dull (specular 0.12), on `leaves_forest_ground` (2 m a repeat,
  2 mm a texel), its sky hidden by up to 70% and its light the leaves' colour (the forest's shade, trees.md, "Shadows"). Seen
  from above the ground stands in for the crowns as before.

Albedo before → after (the median of each view's ground, luminance): Kawah's palm ground from 50 m 0.10-0.11 → 0.108
(the grass between the palms; nearer the sea the coral sand); its beach from 6 m (`at:-120:6:200`) 0.081 → 0.36; its
rainforest floor (`at:400:8:120`, `at:900:1.7:60`, `at:400:1.7:60`) 0.035-0.053 → 0.089-0.117; the Strait of Dover's
pasture (`at:400:1.7:60`) 0.099 → 0.099, its field (`at:900:1.7:60`) 0.109 → 0.115, its downs from 50 m 0.190 → 0.190.
Its pasture, field edges, beach and downs draw as before (under 1% of the pixels differ by more than 2%), but for a
wood's floor beside the eye at `at:900:1.7:60`, now litter (20% of the pixels). The land's cost (the stills' land
figure, 48 frame pairs a run against the branch's head built in the same worktree, either binary first, load average
8-11): +0.10 to +0.15 ms on Kawah's close views (its dearest, `at:900:1.7:60`, 2.94 → 3.09 ms, +5%) and +0.15 to
+0.3 ms on Dover's, whose figure is below zero; one run at load 11 read up to +0.57 ms on Kawah. Under the litter's
finer repeat the stills' RGBA8 reads more bytes than the game's BC7 does (a quarter of them).

*The litter from a man's height* (round 2, the same day) still read as dark grey gravel or asphalt: grey in shade, its
leaves pebble-sized blobs, its sunflecks cream. Measured on the albedo view, taken back through the display (the
strip's greys alone read colour paler through AgX's mixing of the channels), and on the lit stills, taken back to the
scene's colour and divided by the albedo for the light's: the litter was right in brightness (0.09-0.11) but greyed
(C\*ab 12, its red 1.38 and its blue 0.66 of its green, where the leaves' photograph is 24, 1.67 and 0.39), its leaves
flattened (its tenth and ninetieth percentiles 1:2, the photograph's 1:10), and the light on it in shade a deep blue
(red 0.45 and blue 1.5 of its green). Four causes, each fixed where it was made:

- the material held the photograph's contrast at 0.32 in the logarithm (its own is 0.86): the leaves and the dark gaps
  between them within 0.6-1.5 of the mean, so gravel. The forest floor now keeps up to 0.75 of it and 70% of its chroma
  (`tools/ground_materials.py`'s `LOOKS`), squeezed toward the mean by 2.5 to fit the bytes (ratios 0.1-2.7 kept;
  `GroundMaterial::squeeze`, which the uniforms multiply back into its contrast, so the shader reads it whole);
- that ratio, its chroma kept at 40%, lay on a brown of another balance than the photograph's, so its pale leaves came
  out grey and blue: the pebbles;
- the land's tone (`tone_down`) took 30% of the litter's chroma. `litter_color` is now the photograph's balance at
  0.095-0.125 and keeps its colour through the tone, as the photograph does;
- the light under the leaves was the sky's own, a deep blue under a leaf's tint (0.9, 1, 0.7). It is now the daylight on
  the canopy passed through leaves (`under_leaves`, the forest's shade: trees.md, "Shadows").

The tile was right (2 m for the photograph's 1.26 m: leaves of 6-12 cm), and so was the footprint fade (none within 10 m
of a man's eye, where a pixel's filtered footprint is about a centimetre and the fade starts at 0.5 m); the relief
(normal 0.8) did not make the pebbles, the flattened, greyed colour did. Before → after on Kawah's rainforest floor
(`at:400:1.7:60`, `at:900:1.7:60`, `at:400:8:120`, the ground nearest the eye): albedo 0.093-0.109 → 0.093-0.113 (the
mean; the median 0.090-0.110 → 0.082-0.110), C\*ab 12 → 22-23, red 1.38 → 1.67 and blue 0.66 → 0.41 of the green, its
tenth to ninetieth percentile 1:1.5-2.0 → 1:2.3-4.8 (the photograph at that albedo: C\*ab 24, 1.67 and 0.39, and 1:10 at
its full sharpness, finer than a pixel 4-15 m off holds). Lit, under the eye's exposure for the view (its sunflecks held
under the highlights' ceiling): the shade sRGB 11,23,25 (cyan, R > G > B in none of its pixels) → 21,15,3 (brown, in
85-97%); the sunflecks 172,155,136 (cream, C\*ab 13) → 180,152,121 (tan, C\*ab 21; red 1.7 and blue 0.4 of the green in
the scene's colour before the display). From 50 m nothing moves (Kawah 0.7% of the pixels by more than 5 levels, the
Strait of Dover 0.06%); Dover's wood floor beside the eye at `at:900:1.7:60` (10%) shows its leaves, its shade as blue
as before under its thin cover. The land's cost (the stills' land figure, 48 frame pairs a run, round 1's binary and
this one alternating, two batches of four and three runs each, load average 4-16) is within its noise, about ±0.3 ms
here: Kawah's views -0.46 to +0.04 ms (its dearest, `at:900:1.7:60`, 4.33-4.96 → 4.27-4.50 ms), the Strait of Dover's
-0.33 to +0.25 ms, the two batches disagreeing in sign; the leaves' light is a few more instructions and reads nothing,
the litter's file the same size. Round 1's two guesses at Dover's +0.15-0.3 ms, skipping the leaves' light away from
trees and reading the forest again after the surface rather than holding it across, drew the same pixels and read
0.1-0.4 ms dearer on Dover, not cheaper: neither is kept.

*Round 3* (the same day), from the world views and the theatre's places:

- *Litter where no tree stands*: the tropical and volcanic styles laid the forest material as their ground everywhere,
  so the shore strips behind Iron Bottom Sound's and the Sunda Strait's beaches and the tour's bare slopes read as
  litter from the ship and the beach. Their ground is grass again (master's colour), and the forest material follows
  the trees' own stand (`tropical_stand`, `volcanic_stand`: the density and the palms' share `tree_cover` places its
  trees by), litter where the broadleaves' stand passes 0.3-0.8 (`under_broadleaves`), soft as they thin; the palms
  stand on grass and sand. The floor also fades in over the bake's first 0.05 of cover (`floor_seen`).
- *Dover's hard diagonal edge* (`at:900:1.7:60`) was the canopy's relief, domes metres high, still drawn beneath the
  crowns where a wood's field's mask ramps, not its hedges (zeroing the hedges' bump left the band in the normal view;
  zeroing the canopy's took it away). The canopy's paint now fades out beneath the crowns (`canopy`, by one less
  `floor`). The floor in shade there reads darker for it (Dover's beside the eye L\* 7 → 4, 7 again with the relief
  back): the domes had lifted its light. Under Dover's thin cover that light was still the sky's blue (the trees' lane).
- *Orange*: the litter measured 0.079-0.100 at C\*ab 18-21, its red 1.6 and its blue 0.4-0.5 of its green, tan-orange
  in the sun, where floors measure: dry oak leaves 0.108 at C\*ab 15, damp broadleaf litter 0.079 at 10.5, a wet
  tropical floor 0.063 at 9.7, spruce needles 0.057 at 6.8, dusty ground 0.17 at 10. The leaves' photograph keeps 45%
  of its chroma (was 70%), and `litter_color` is dark brown by the floor's kind: broadleaf 0.093, wet 0.077, needles
  0.078, dry 0.19, before the photograph's grain and the hollows.
- *One litter under every forest*: the floor's kind comes from the biome grid the trees choose their species by
  (`biome_grid`, `biomes`: 16 × 16 cells of 3 km, the theatre's regions, jittered across the cells' edges); land
  without one (the stills' test land) takes its style's (snow: needles in the hollows; tropical and volcanic: wet).
  `floor_at` lays needles under the boreal biome's spruce and pine (fewer in the birch belt under its treeline,
  `birch_belt`, as the trees), under the
  temperate biomes' woods above 800 m, by the sea and in a share of the stands as the trees there, over a third to
  three-fifths of the floor in the Mediterranean, California, the subtropics and Japan; the wet tropics' and Hawaii's
  floors are darker, the arid and dry-summer woods' (the climate's dryness) pale with dust. The needles are Poly Haven's
  Forest Ground 03 (CC0, 2 m a repeat, 1024², 35% of its chroma, its contrast up to 0.6), read through the forest
  material (`floor_read`, as the cliffs' rock through `M_CLIFF`) in patches of the share (`floor_patches`, on 40 m
  noise).

Before → after on the albedo view (the floor nearest the eye at `at:900:1.7:60`, its median luminance and C\*ab; Kawah's
map under each place's biome and climate, where round 2 laid one floor at 0.080 and C\*ab 18): the tropics
(Guadalcanal) 0.054-0.061 at 9.6-12.0 (floors measure 0.063 at 9.7), temperate (Scapa) 0.065-0.074 at 10.7-10.8 (0.079
at 10.5), conifer (Narvik) 0.062-0.066 at 6.6-6.8 (0.057 at 6.8), the dry summer (Malta) 0.151-0.158 at 9.3-9.4 (0.17 at
10); Kawah's own floor (`at:400:1.7:60`, `at:900:1.7:60`, `at:400:8:120`) 0.079-0.100 at 18.5-21.1 → 0.053-0.070 at
10.7-13.9, R > G > B in 97-100% of its pixels. Lit: its sunflecks 171,143,122 (C\*ab 17) → 147,134,121 (9) beside the
eye and 206,182,152 (19) → 187,166,145 (14) from 8 m; its shade 20,14,3 → 11,9,3 and 39,29,15 → 27,22,14, brown in
78-100% of the pixels. The land's cost (the same figure, round 2's binary and this one alternating, three runs each,
load average 5-7) moves by -0.04 to +0.12 ms, within round 2's noise of ±0.3 ms: Kawah's dearest 2.99 → 2.99 ms, the
two views from 50 m +0.09 and +0.12 ms, their runs apart. Open: Britain's, Europe's and the Northeast's floors take the
temperate share of needles, not the photograph's dark woods their trees read; grass blades may now grow on the tropical
open ground. The terrain tests' one failure then, `fine_relief_reshapes_the_land_near_the_camera` (its re-draw with the
relief hidden 230-245 bytes off, by up to 10), was the trees' commits' (from the depth first, before the ground's): its
first picture was taken while the impostors' light still moved (its unsteady derivatives, since steadied: trees.md, "The light's
discard"), and the tests' runs now outlast the forest shade's wait (`tests/gpu.rs`, `render_at`).

Theatre (naval-client's own scenes on the pack's BC7 materials; the `theatre_ground` test below: the GPU's busy time with the
materials shown and hidden in the same scene, five alternating 30-frame batches a side, each side's tenth percentile,
three runs, medians), off Santa Cruz with NAIP and with Sentinel-2 alone (`NAVAL_NAIP=off`), frames of 7-9 ms:

| View | NAIP | Sentinel-2 alone |
| --- | --- | --- |
| a ship's bridge 400 m off the beach | +0.07 | +0.09 |
| a ship's bridge toward West Cliff | +0.08 | +0.06 |
| a plane at 50 m over the beach | +0.11 | +0.11 |
| a plane at 300 m | +0.04 | +0.06 |
| a plane at 1,500 m | +0.03 | +0.05 |

On the GPU the materials take 37 MB (the pack's BC7); on disk 34.3 MB in the pack and 9.6 MB of JPEGs in the
repository.

**Checked.** The GPU test `the_ground_materials_add_structure_close_up_and_nothing_far_off`: handed in, the materials
change a tenth of the pixels of a view 50 m over the shore and its mean brightness by under 2 levels; hidden, or 20 km
off, the land is the same picture as without; the Medium setting refuses them. `ground_materials`' unit tests: the
shader's ids are the table's, and the sources build a consistent array whose last level is each ratio's mean (128).
The theatre: `cargo test --release -p naval-client --test all -- theatre_ground:: --ignored --nocapture` (the California
scenario with Force A 400 m off Santa Cruz's Main Beach: a ship's bridge at the beach and at West Cliff, a plane at
50 m, 300 m and 1,500 m, each with the materials, without and again, the material debug view, and the GPU's busy
time); `naval_client::land::set_ground_materials_shown` and `set_terrain_debug` drive it. The stills example takes
`--materials off`, `--material-cost`, `--material-strength`, `--material-side` and `--debug material` (each pixel's two
materials by hue and weight).

**Open.** From a ship the beach is grazing and the materials mostly faded (a pixel there spans metres along the view):
what reads as a beach from the sea is still the colour's bands (dry sand, the wet band, the foam). Cliffs read their
rock face on since 2026-10-09 ("Cliffs"). Roughness is the cover's, not the material's. The aerial materials (grass,
sand, scrub) are 1-3 cm a texel, soft from an eye at a man's height (the grass's blades cover it on grass); the three
seen from there (leaves, needles, fine sand) are sharp to the nearest pixel, and the other close-up ones (soil,
shingle, mud, snow) could be written at 1024² from their 1K sources for no memory in the pack (it holds every layer at
1024²), 4 MB more in the repository. A theatre's photographed land was seen only in the tour's places (Kawah's map under
each one's biome; `theatre_ground` needs the imagery cache): its tree cover takes its biome's floor from beneath its
crowns (round 3). Under the canopy the trees' own low crowns, shrubs and trunks are still lit by the sky's blue (the
trees' lane): cyan-grey beside the brown floor since round 2.

## Cliffs

On High and Ultra a theatre's land keeps its photograph on every slope and draws its rock faces in the rock of the
place, read face on. Medium fades the photograph to the latitude's style past a slope of 0.55-0.95 and lays the style's
cliff material on the chart's plane.

```rust
CliffRock::at(lat, lon);                             // naval_terrain::ground_materials: the rock of a coast
ground_materials::cliff_grid(place, corner);         // the rocks about the eye, four bits a 4 km cell
land.show_rock_faces(false);                         // for comparisons; naval_client::land::set_rock_faces_shown
```

**Where the faces are** (`surface.wgsl`'s `rock_face`, on the `DETAILED` defs where the land has a place: `rock_faces`):
the sea's cliffs (within 120-450 m of the sea and 3-9 m above it) from a slope of 0.42 (23°) at the water's edge to 0.77
(38°) 400 m up, faces inland from 1.15 (49°); steeper where WorldCover puts growth there: by the sea 0.5 under woods and
scrub and 0.12 under grass and fields (grass tops most sea cliffs), inland 0.6 and 0.3. The edge is sharp (0.12 of
slope) and broken by the land's 210 m and 47 m noise and, where a pixel holds it, a 9 m one, so a face's top is ragged
rather than a contour. Over the face the rock covers the photograph and the style, its colour exempt from the styles'
tone as the photograph's is, and the shore's sand and shingle stay off it. Where a slope short of a face stays green, so
does the land: the photograph now holds there (the Koolau and Fløyen's forest, Diamond Head's dry grass) where the
style's rock and chalk used to. Bare rock is a face from a gentler slope: where WorldCover says bare or sparse ground
(60), or grass, moss and lichen (30 and 100, one class in the window's cover) within 400-700 m of the climate's
snowline, from 0.62 (32°), unless the season's snow lies there; so the Eiger's, the Oberland's and the canyon's rock
below 49° is rock (the place's material read face on, its beds and stains) rather than the photograph with the rock
material's grain. Its colour there is the photograph's ("How it looks"). The slope every face is judged by is the
stepped one ("Stepped walls"): a cliff band counts as steep, its bench as gentle.

**The rock** (`CliffRock::at`): GLiM's dominant rock of the place's half-degree cell (`earth::lithology`, see "Climate
and rock maps" below), or, where that cell is the sea's or a lake's (a coast's often is: half a degree is 55 km), the
nearest cell with rock within a degree (`rock_near`); far out on an ocean, past every cell, the latitude's (basalt in
the tropics, granite above 55°, sandstone between). A few coasts and mountains are named by hand first
(`CliffRock::named`), where the half-degree cell takes a cliff a few kilometres long for its neighbour's rock or an
island has no cell: the Channel's chalk, Rügen and Møn; Orkney's Old Red Sandstone; Malta; the Marianas; the Rock of
Gibraltar (its cell the sea's, the nearest Andalusia's mixed sediments); the Bernese Oberland's Helvetic limestone front
(46.53-46.72° N, 7.75-8.2° E: Lauterbrunnen's walls, the Schilthorn, the Eiger's and the Mönch's north faces, the
Wetterhorn; GLiM has the Eiger's cell carbonate but the Mönch's and Lauterbrunnen's, west of 8° E, the unconsolidated
sediments of Interlaken's lakes) and the Aar massif's gneiss south of it (to 46.3° N: the Jungfrau's and
Finsteraarhorn's crests, the Aletsch, whose cell GLiM gives the moraines); Taroko's gorge and the Qingshui cliffs'
marble; Brittany, Cornwall and Corsica's granite; and the volcanic coasts (Hawaii, Japan and the Ryukyus, the Aleutians,
Iceland and the Faroes, the Atlantic islands, Naples, Etna).

| GLiM class | Rock |
| --- | --- |
| sc carbonate sediments | Limestone |
| ss siliciclastic, sm mixed, su unconsolidated sediments, ev evaporites | Sandstone |
| mt metamorphic | Gneiss |
| pa acid and pi intermediate plutonic, ig ice and glaciers (the rock under it unknown) | Granite |
| vb basic, vi intermediate and va acid volcanic, py pyroclastic, pb basic plutonic | Basalt |
| wb water, nd no data, no cell | the nearest cell's within a degree, else the latitude's |

| Rock | Where | Material | Colour (linear) | Bedding, beds |
| --- | --- | --- | --- | --- |
| Chalk | the Channel's chalk from the Needles to Dover and Normandy to Cap Blanc-Nez; Rügen and Møn | limestone | 0.42, 0.42, 0.39 | 0.45, 2 m |
| Limestone | GLiM's carbonates (the Dinarides, Greece, the Levant, Normandy's Bessin, the Eiger's cell); the Marianas, Gibraltar, the Bernese Oberland | limestone | 0.28, 0.26, 0.21 | 0.7, 3 m |
| Golden limestone | Malta and Gozo | limestone | 0.29, 0.21, 0.115 | 0.8, 3.5 m |
| Sandstone | GLiM's other sediments (California's bluffs, the Baltic's glacial shores, the Grand Canyon, Essex); mid-latitudes past the cells | cliff | 0.15, 0.12, 0.085 | 0.8, 2.5 m |
| Old Red Sandstone | Orkney, Caithness, Shetland | cliff | 0.12, 0.065, 0.045 | 1, 1.5 m |
| Basalt | GLiM's volcanics (the Solomons, Lombok's Rinjani, Busan); Hawaii, Japan, the Aleutians, Iceland and the Faroes, the Atlantic islands, Naples, Etna; the tropics past the cells | basalt | 0.06, 0.055, 0.05 | 0.55, 6 m |
| Granite | GLiM's plutonic rocks (Big Sur, the Sierra, Sweden's and Finland's shield); Brittany, Cornwall, Corsica; above 55° past the cells | granite | 0.12, 0.115, 0.11 | none |
| Gneiss | GLiM's metamorphic rocks (Norway's fjords and Bergen, the Highlands, New England); the Aar massif | granite | 0.06, 0.058, 0.054 | none |
| Marble | Taroko and the Qingshui cliffs | limestone | 0.26, 0.26, 0.25 | 0.25, 6 m |

Gneiss is granite's photograph under half granite's colour: the fjords' gneiss stands dark, near basalt's grey, not
granite's pale grey. The colour reaches only the faces (`rock_face`'s weight: steeper than 49° inland, 23-38° by the
sea), over the photograph elsewhere: against granite's colour, Geiranger's Seven Sisters wall draws 15% darker (mean
luminance 118 to 101 over the face, 1920 × 1080, Ultra) and the wall west of the fjord 17% (131 to 109); the Eiger's
cone and the fjord's walls seen from the water, short of a face's slope in the 40 m window, keep the photograph's tone
and take only the material's structure. GLiM at the places the land lanes judge (`earth::lithology`'s
`report_the_places`): the Eiger sc (Limestone, and named), the Mönch and Lauterbrunnen su (named Limestone), the Grand
Canyon ss (Sandstone), Geiranger mt (Gneiss), Bergen the sea's, nearest mt (Gneiss), Taroko the sea's, nearest ss (named
Marble), Rinjani vi (Basalt), Gibraltar the sea's, nearest sm (named Limestone), Big Sur pi (Granite), Busan va
(Basalt), Palos Verdes su (Sandstone).

The colours are albedo on the calibrated photograph's scale (bare sand and chalk 0.19 of luminance there; chalk
brighter, as the chalk style keeps it). The layer keeps the rocks of 32 × 32 cells of 4 km about the eye
(`cliff_grid`, made again when the eye crosses a cell, as the trees' biomes are) and a table of the rocks
(`cliff_rocks`); a point takes the material of the cell it stands in and the colour and bedding blended over the four
about it, so a change of rock is kilometres wide.

**How it looks**: the colour blotched by the land's 210 m and 47 m noise and a 14 m one close up; bedded by height, the
height warped by the 930 m and 210 m noise so the beds dip and wander: packages six beds thick and the beds within
them, each a tone of its own from the noise tile's Worley cells along a line through it (beds of uneven thickness with
sharp edges; `beds`), each faded as a pixel spans it, standing a little proud of the face in the land's bump; stained
darker down the fall line (`fall_streaks`, 10 m across). Granite has no beds. The beds are read at the
stepped height ("Stepped walls", packed up a cliff band and spread across its bench) and also tilted over the country:
the height is warped by a 40 km noise (±800 m) and a 12 km one (±200 m), so the strata dip a degree or two on
average and up to about six, their direction wandering, and a peak's beds cross it rather than ring it (`strata_dip`).
The bedding shows in some stretches of a wall and not others (the 930 m noise takes it from full to a fifth), a hard
package's cliff is massive (its beds at 0.4), and each cliff band is a shade of its own (±14%). The face's colour is
the photograph's (calibrated, times the face's blotches, beds and stains) where the slope before the steps is under
1.0 (45°), the rock's own past 1.3 (52°): a photograph from above sees a 30-45° slope well, and the place's real colour
is a better witness there than the rock's one colour (the canyon's red beds, the Eiger's grey); it does not stand for
snow, ice or water, nor for anything as bright (calibrated luminance from 0.22 to 0.32).

**Stepped walls** (`surface.wgsl`'s `stepped`). Layered rock weathers into steps: a hard bed
stands as a cliff over the soft beds' slope beneath it, and its top is a bench where the soft rock above has worn back
(the Grand Canyon's Kaibab and Coconino cliffs over the Toroweap and Hermit slopes, the Supai's ledges, the Redwall over
the Muav, the Tapeats over the Tonto platform: formations 40-200 m thick). The 40 m window draws such a wall as a
smooth slope, so the steps are drawn into the land's normal alone, never its heights (the simulation, the RPC heights,
buildings, trees and the camera's clearance see the smooth land): the warped height (the beds' level above) is cut
into packages 120 m apart on average (each floor moved by up to a third of that, so 40-200 m thick), and across each
package the slope is a bench and then a cliff. Two packages in three are hard: a cliff of 2.4-3.4 (67-74°) over a
bench from a broad platform to a talus slope (0.1-0.6 of the wall's slope), never gentler than a ledge on a steep wall
(0.65, 33°, as the wall's slope nears 1.2); the third soft, its slope stepped by 1.3 and 0.7. The two shares are
chosen so the package keeps the wall's slope, its foot softened over a quarter of the cliff's width (talus) and its
rim sharp. The stepped slope is the slope from there on: the cover, the season's snow (which sheds from the cliffs and
lies on the benches), the faces and the ground materials (the cliff read face on up a cliff band) all take it, and the
normal takes it as a slope added to the land's gradient (`Surface::slope`). Where: rock with beds of 0.5 and more
(sandstone and red sandstone fully, limestone 0.9, basalt a tenth; never chalk or granite), slopes from 0.25 to 0.45 (14-24°) up,
on bare ground, shrubs (0.8), steep grass (0.6 from 0.6 to 0.9 of slope) and unknown cover, never woods, fields,
towns, snow and ice or water, nor where a finer model's fine relief is drawn (its pages hold the real steps within a few
km of the camera in the United States, England, Norway, New Zealand and Japan; a synthetic, GLO-30 or combined page holds
none, so the steps stay under it: `ReliefPage::real`, `bindings.wgsl`'s `relief_real_share`). Each package's cliff runs out along the wall
here and there (where the 210 m noise is under a cut of its own, 0.1-0.5, a quarter more on walls steeper than 0.8),
so no band rings a mountain. The steps fade into the slope as a pixel spans a cliff band (from two pixels to 0.6).
Cost: the cliff grid's four cells read on every slope over 0.2 (they were read only on a face), two noise reads for
the dip where the rock has beds, a hash and some arithmetic; not measured.

**The canyon's colour.** Under a hazy sky the Grand Canyon's walls read lilac-grey where the photograph is red-brown.
Measured in the client (the albedo debug view, one channel at a time against its strip; then the land lit with its
aerial perspective off), 6 June 08:00 at 60 km visibility: the land's albedo is the photograph's, unchanged by the
shading (the photograph alone and the whole surface agree within 0.01: a wall 2.6 km off at 0.246, 0.171, 0.123 linear);
lit without the air the wall keeps that hue (sRGB 122, 103, 89 for an albedo of 136, 115, 99); the air turns it to 126,
113, 111 at 2.6 km and the far walls, 5-20 km off, to 166, 149, 141 from 166, 144, 124 (the haze's blue in-scatter at 60
km visibility over a red rock: the lilac). The calibration takes some red too: Sentinel-2 over the canyon's red beds,
haze taken out, is 0.262, 0.145, 0.069 (red over green 1.81, blue 0.48), and `PHOTO_CHROMA`'s cap of 0.9 of luminance
makes that 1.60 and 0.61, about what the Supai's and Hermit's red rock reflects. So nothing in the land's shading is
wrong; the Southwest's air in the 1940s was clearer than 60 km (its visual range now averages some 200 km), and a
scenario there wants its own visibility.

**The material face on** (`materials.wgsl`'s `cliff_read`): where the cliff material is chosen (`M_CLIFF`: the face
here, the styles' cliff masks with no place), it reads the rock's material (`cliff_rock`: the cliff's own
`marble_cliff_05`, or limestone, basalt or granite, three Poly Haven photographs of rock faces,
`public/terrain/ASSETS.md`) biplanar (Quilez): from above where the land lies back, and on the two vertical planes the
face looks along most, up kept up, so its joints and beds stand up the face instead of streaking down it. Their weights
by the normal (`wall_weights`) read only the chart below 35°. The face-on reads' relief is a gradient in the world,
taken onto the land's normal (Mikkelsen's surface gradient, `wall_slope`) and handed to the fragment stage as one more
slope of the ground, so `terrain.wgsl` is unchanged. Up to three reads for one: only where the cliff is one of the two
materials read and the pixel is within the materials' 1.6 m.

**Memory.** The three rocks are six more layers of the materials' array: about 8 MB more on the GPU from the pack (BC7
1024², 37 to 45 MB), 8 MB more from the JPEGs (RGBA8 512², 39 to 47 MB); 2.2 MB more JPEGs in the repository.

**Cost** (the scene's opaque pass, the land with the ships, buildings and trees, on Ultra at 1920 × 1080 on an Apple M5
Pro: the rock faces shown and hidden in turn, 5 × 30 frames a side, medians): cheaper on every cliff view, the same
elsewhere. Hidden, the style is painted under every slope the photograph fades from (its masks, rock and canopy); shown,
the photograph holds there and the face's own work (a noise read for its edge, two for the beds, the streaks, the grid's
four cells) runs only where a face is, the face-on reads (up to three for one) within the materials' 1.6 m.

| View | Hidden → shown (ms) |
| --- | --- |
| Hoy, St John's Head through the binoculars | 4.85 → 4.32 |
| Dingli's cliffs through the binoculars | 4.66 → 4.15 |
| Jōgasaki through the binoculars | 5.34 → 5.06 |
| Palos Verdes' bluffs through the binoculars | 2.66 → 2.72 |
| Fløyen from 500 m, the Koolau from 500 m | 3.77 → 3.58, 3.71 → 3.57 |
| Big Sur from 500 m and 1,500 m | 1.93 → 1.75, 1.46 → 1.32 |
| Flat ground and far views (1,500 m over Malta, Hoy, the Downs) | within ±0.05 |

Medium compiles none of it (`DETAILED` and `MATERIALS` both off) and draws as before.

**Open** (stepped walls). The steps are a normal map: seen from the side a cliff band is as tall on screen as its share of
the smooth slope (0.1-0.5 of the package), not its share of the height, and the silhouette stays smooth. The packages
are random, not the real formations (their colours come from the photograph, so a band of the photograph's red need not
sit on a cliff). The dip is a noise: a plateau's beds may lean up to some six degrees where the real ones lie flat.

**Open.** The faces are where the heights put them: today's 25-40° slopes, so Palos Verdes' and Big Sur's bluffs read as
tan slopes with faint rock from the bridge, little changed. The fine relief ("Fine relief near the camera") stands them
up within about 3 km where a finer model covers them; beyond, and where none does, they stay. From the bridge at 4-10 km
the beds are faint in the haze; they read through the binoculars and below 1,500 m. Where the slope hovers about the
threshold the face breaks into patches of rock among the scrub (Palos Verdes, Izu). Close up through the binoculars the
beds' edges are thin dark lines, more drawn than weathered (Dingli). Trees still stand on a face below a slope of 1
where WorldCover says scrub (Izu). The vegetated pali take the photograph, whose Koolau tops are pale in the mosaic's
haze rather than the pali's deep green. One rock to a 4 km cell, from a half-degree map and a few named boxes: a rock
that changes within a cell (the Eiger's limestone over the Aar massif's gneiss) takes one of them. The Downs' views see
downland, not the Seven Sisters' faces, so chalk faces are unseen in the stills. The three rocks reach the GPU only once
the pack is built again (`bun run update`); a checkout without a current pack makes them from the JPEGs (a pack made
before them records fewer sources, and `Assets::ground_job` passes it by).

## Climate and rock maps

Two global maps are compiled into `naval-terrain` (`src/earth/`; no file is read at run time), baked from their
sources by `python scripts/theatre/bake-climate-and-rock.py` (numpy and Pillow; it fetches what is missing into
`.build/land-data/`, its docstring gives the sources and the layouts). The files are generated: change them only by
running it again. Credits: each module's `ATTRIBUTION` and public/terrain/ASSETS.md, "Climate and rock".

| Map | Source | Cells | File | Reader |
| --- | --- | --- | --- | --- |
| Climate | Köppen-Geiger 1931-1960, Beck et al. 2023 v3 (CC BY 4.0) | 30" (0.9 km), 30 classes and the sea | `koppen.nkg`, 6.4 MB: 1° tiles, 11,000 of one class throughout, 14,674 zlib streams | `earth::koppen` |
| Rock | GLiM, Hartmann and Moosdorf 2012, 0.5° raster (CC BY 3.0) | 0.5° (55 km), 16 classes, none at sea | `glim.ngl`, 26 KB | `earth::lithology` |

```rust
earth::koppen(lat, lon);                    // Option<Koppen>: the 30" cell's class, none at sea
earth::koppen_mode(lat, lon, radius_m);     // the commonest land class within radius_m north, south, east and west
earth::koppen_shares(lat, lon, radius_m);   // KoppenShares: cells per class; share_where(Koppen::is_arid), mode()
Koppen::Csa.group();                        // KoppenGroup::Temperate; is_arid(), is_desert(), is_steppe()
Koppen::Csa.dry_season();                   // Some(DrySeason::Summer) (s; w and m Winter); warmth(): Hot (h, a) .. Severe (d)
earth::lithology(lat, lon);                 // Option<Lithology>: GLiM's class of the 0.5° cell
earth::rock_near(lat, lon);                 // its class, or the nearest cell's with rock within a degree
```

The climate map decodes its index (65 KB) on the first lookup and a tile (14 KB) when a lookup first needs it, keeping
the last 48 (`CACHED_TILES`); a lookup is one cell, `koppen_shares` over 40 km reads about 9,000 cells over at most four
tiles. The rock map is decoded whole (259 KB) on its first lookup. Uses: the cliffs' rock ("Cliffs"); the climate map
gives the trees' biomes (`species::cell_climate`, the commonest class of each 3 km cell of the biomes' grid: trees.md,
"Species"), how dry the country looks and the land's style (surface.md, "Seasons, climate and tiling"). `--per-degree 60` bakes the climate at
1' (the commonest land class of each four cells), 2.8 MB, should the 30" file prove too large.
`KoppenShares::mode_where(|k| ..)` is the commonest of the classes that meet a test (the trees' classes, not E).

At the places the land lanes judge (`earth::koppen`'s test and `report_the_places`): the Grand Canyon's South Rim Dsa,
its inner gorge BWk; the Eiger ET, Interlaken Dfb; Taiwan's high range Cfb, Hualien Cfa; Rinjani's summit ET, Lombok's
coast Aw; Geiranger Dfc, Dalsnibba ET.

## Fine relief near the camera

A theatre window's heights are a 40 m grid (the region bake's GLO-30 and Terrain Tiles, 20 m vertices on High): cliffs,
crater rims and gullies narrower than that would be smooth slopes (Palos Verdes' bluffs, Dover's chalk). Where a finer
elevation model is free to use, the land near the camera takes it: its vertices stand on it and its fragments shade with
its slope, from a few kilometres in. Presentation only: the simulation, the coast and the chart read the window's
heights as before.

```rust
land.set_relief_grid(&mut ctx, ReliefGrid { origin, texel_m: 4.0, slots: 8 })?;   // High only; Medium refuses it
land.queue_relief_page([i, j], Arc::new(ReliefPage::from_residual(&residuals, 4.0)));  // no renderer context needed
land.holds_relief_page([i, j]); land.relief_counts();   // (drawn, waiting, patches drawn dense)
land.flush_relief_pages(&mut ctx);                       // a layer being built: upload what is queued now
land.show_relief(false);                                 // hidden, for comparisons and measurements
land.clear_relief(&mut ctx);
```

**The sources** (naval-landscape `relief`, `REGIONS` and `ATTRIBUTION`; their licences in
`public/terrain/ASSETS.md`, "Fine relief"), each only where it is finer than the window:

| Where | Source | Spacing |
| --- | --- | --- |
| The conterminous United States, Hawaii, Puerto Rico | AWS Terrain Tiles (Terrarium) at zoom 15: USGS 3DEP | 1-10 m (zoom 15 is 3.4-4.4 m a pixel) |
| England | the same tiles: the Environment Agency's LIDAR composite | 2 m |
| Norway | the same tiles: Kartverket's DTM | 10 m |
| New Zealand | the same tiles: LINZ's 8 m DEM | 8 m |
| Japan | the Geospatial Information Authority's elevation tiles: `dem5a_png` (5 m, airborne laser) at zoom 15, `dem_png` (10 m) at zoom 14 where the 5 m has no tile | 5-10 m |

Elsewhere no finer model: the Terrain Tiles there are SRTM's 30-90 m, no finer than the window (Malta, Gibraltar, the
rest of the Mediterranean, Scotland, the Alps, Taiwan, the Solomons, the Philippines). Spain's IGN MDT05 and Italy's
TINITALY are free (CC BY 4.0) but come as downloads, not tiles a client can read near the camera; Malta and Gibraltar
have no open national model. There the pages are **synthetic** (naval-landscape `relief_synth`), made from the window's own
heights:

- *The base*: Catmull-Rom between the window's samples rather than bilinear, so crests and valleys bend between them.
- *Gullies* down the fall lines, four octaves from 240 m to 30 m: phacelle stripes (four cells' waves across the slope,
  their phases blended and made unit, so stripes stay whole and fork where the cells disagree), each octave turned by
  the slope the coarser ones leave so the finer branch off them (after Rune Skovbo Johansen's erosion filter), cut as a
  V notch a quarter of the wavelength wide with the ground between a little rounded. Each octave 10 % of its
  wavelength deep, 0.42 of the last, times how steep the slope is (from 0.1 to full at 0.6), how much the land rises
  within 320 m (from 15 m to full at 200 m) and a patchiness (value noise three wavelengths across, from a fifth to
  all of it), so the gullies do not flute a whole slope evenly; none on flats, valley floors, lakes and below 4 m (full
  from 40 m).
- *Beds*: the smooth heights stepped along their contours into benches and risers (two sets, 90 and 156 m apart, their
  surfaces undulating, a riser three times the slope's mean), in patches a few hundred metres across, full where GLiM's
  rock is sedimentary and Köppen's climate dry (the canyons' cliff bands), a quarter of that in sedimentary rock under a
  wet climate (soil and forest: Taiwan's beds drawn full were white ladders of rock across its forests), a twentieth
  elsewhere; the rock and climate read at a page's corners so they blend between pages.
- *The same residual* as a finer model's: the synthetic heights less the lattice's bilinear surface through them, so a
  page is zero at the samples; the noise is a function of the theatre's metres, so neighbours agree in their gutters.
  A page is made only where a bounds block over it rises 20 m (`page_has_relief`), and dropped when it holds under 5 cm.

`NAVAL_RELIEF_SOURCE` picks the source: `auto` (a finer model where one covers, else the combined, below; the synthetic
alone offline), `real`, `synthetic`
(everywhere, over a finer model too, to compare with it), `glo30` (a test, below), `combined` (below) or `off`; `NAVAL_RELIEF_SYNTH`
overrides the generator's parameters (`amplitude=0.1,slope=0.15:0.7`, `relief_synth::Params::parse`). Against the real
where one covers (the ignored `relief_report` test: the same pages, plain, real and synthetic; its hillshades in
`.build/synth-relief/pages/`), a page's relief by band of wavelength, RMS (m) over the land:

| 4 km about | Source | Steeper than 30°, 45° | 40-160 m | 16-40 m | 4-16 m |
| --- | --- | --- | --- | --- | --- |
| Geiranger and Dalsnibba | the window | 36.7 %, 13.1 % | | | |
| | Kartverket's 10 m | 36.8 %, 14.1 % | 2.0 | 1.1 | 0.6 |
| | synthetic | 43.9 %, 17.5 % | 1.6 | 1.1 | 0.8 |
| The Inner Gorge and Zoroaster Temple | the window | 48.1 %, 16.2 % | | | |
| | 3DEP | 47.5 %, 18.9 % | 3.6 | 2.4 | 1.5 |
| | synthetic | 52.5 %, 21.9 % | 2.2 | 1.5 | 1.0 |
| The Eiger and Lauterbrunnen | the window | 46.3 %, 20.3 % | | | |
| | synthetic | 52.3 %, 24.3 % | 1.6 | 1.2 | 0.9 |
| Taroko and the Qingshui cliffs | the window | 72.3 %, 24.1 % | | | |
| | synthetic | 75.8 %, 31.7 % | 1.9 | 1.4 | 1.0 |

The real relief broadens the slopes (the canyon's benches and cliffs) and leaves their mean; the synthetic's gullies cut
across the slope and steepen it a little more, and its beds are fainter than the canyon's cliff bands (its 40-160 m band
three fifths of 3DEP's). Kartverket's model is smooth below 10 m, so Norway's finer bands are its own, not the
fjords'. A synthetic page takes 100-115 ms on the `dev-client` build (opt-level 1, other sessions building; four a
batch on four threads), a real one 15-18 ms from its tiles. The GPU's cost is the relief's own (the table below): the
frame 5.55 ms median at the Eiger's face without, 5.6 ms with (dev client, 1920 × 1080, the user's settings).

**GLO-30, a test** (`NAVAL_RELIEF_SOURCE=glo30`, naval-landscape `relief_glo30`; never chosen by `auto`): Copernicus's 30 m
surface model, its cells' blocks and filling masks read by range from the AWS copy (kept under the relief's cache),
through the same residual. A surface model measures the canopy and the roofs, so its detail is kept only on WorldCover's
open classes (grass, crops, bare, snow and ice, water, wetland, moss): none within 40 m of a tree, shrub, roof or
mangrove pixel (a lattice cell, so no sample the window's surface is taken through stands on a canopy), all of it 60 m
further in (a chamfer distance, a smoothstep). Heights the filling mask says came from another model (ASTER, SRTM and
the rest, under radar shadow and layover) are closed ground too; `NAVAL_RELIEF_GLO30=fills` keeps them.
`NAVAL_RELIEF_GLO30=band` lets GLO-30 replace the window's heights at wavelengths under about 330 m (GLO-30 less the
window, less that blurred by two passes of a 164 m box) instead of only adding what lies between the samples: the
samples then move, by metres to tens of metres on the crests, which the simulation does not see. Against the synthetic
on the same pages (the ignored `report_glo30_against_synthetic` test; its hillshades in `.build/glo30-relief/pages/`):

| 4 km about | Source | Steeper than 30°, 45° | 40-160 m | 16-40 m | 4-16 m | Kept, filled |
| --- | --- | --- | --- | --- | --- | --- |
| The Eiger and Lauterbrunnen | the window | 46.3 %, 20.3 % | | | | |
| | GLO-30, unmasked | 46.9 %, 20.5 % | 1.3 | 0.6 | 0.4 | |
| | GLO-30 | 46.5 %, 20.4 % | 0.3 | 0.15 | 0.1 | 18 %, 47 % |
| | GLO-30, band | 47.0 %, 20.5 % | 1.5 | 0.3 | 0.2 | |
| | GLO-30, band with the fills | 46.2 %, 20.8 % | 3.8 | 0.6 | 0.5 | 47 %, kept |
| | synthetic | 52.3 %, 24.3 % | 1.6 | 1.2 | 0.9 | |
| Rinjani's cone and north rim | the window | 47.3 %, 14.8 % | | | | |
| | GLO-30, unmasked | 50.2 %, 15.2 % | 1.1 | 0.55 | 0.35 | |
| | GLO-30 | 48.8 %, 14.8 % | 0.5 | 0.25 | 0.15 | 35 %, 31 % |
| | GLO-30, band | 52.4 %, 15.0 % | 2.4 | 0.4 | 0.3 | |
| | GLO-30, band with the fills | 53.3 %, 16.1 % | 3.9 | 0.6 | 0.5 | 64 %, kept |
| | synthetic | 57.9 %, 20.2 % | 1.7 | 1.3 | 1.0 | |

Through the residual GLO-30 adds next to nothing: its pixels are hardly finer than the lattice's 40 m, so what lies
between the samples is mostly their beat (a faint plaid in the hillshade), and the window's smooth crests stay at the
samples. The faces the look wants are where it is filled: 47 % of the Alps' land (the Eiger's faces among it), 31 % of
Rinjani's; with the fills and the canopy masked, a fifth of the Alps' land keeps any. GLO-30 and the window differ by
7-42 m RMS a page (10-33 m at the best shift: the window's smoothing more than an offset; Rinjani's best shift is 12-24
m south throughout), and GLO-30's highest point lies more than 20 m from the window's on half the crest crossings, yet
under 3 % of the crests are doubled (6 % with the fills; the synthetic's notches 40 %): the residual is too faint for a
second crest. A page takes 39 ms (the band 25 ms more; `dev-client`, other sessions building) and 60-400 ms to read its
blocks, masks and cover the first time; the frame at the Eiger's face is the synthetic's (5.65-5.8 ms median against
5.6-5.9 off).

In the stills (`.build/glo30-relief/`, the tour's views within the pages' reach: the Eiger's face and Lauterbrunnen,
Rinjani's cone from north and south and its caldera), GLO-30 through the residual is the window again, a little softer;
its band draws moraines, ledges and the cone's ribs where it is kept, but the Eiger's face is filled and stays the
window's dome. With the fills kept the band changes the mountain's shape (a notched summit, buttresses and couloirs on
the face), the couloirs smooth rounded troughs: the fills' own 30-90 m. No step shows at a forest's or a mask's edge.
GLO-30's 30 m cannot draw what the look misses below about 60 m (the ledges, flutes and gullies the synthetic draws);
the window's smoothness above it is what GLO-30 can fix, and only by moving the samples.

**Combined** (`NAVAL_RELIEF_SOURCE=combined`, naval-landscape `relief_combined`): GLO-30's band with its fills kept for the
shapes from about 60 to 330 m, the synthetic's detail under 60 m on top. A page reads GLO-30's cells, filling masks and
WorldCover as the test source does (`Inputs::load_with`), and lays a base over its texels and 16 more each way (every
sample its residual stands on): the mask's weight (open ground, GLO-30 held, no void), the band and its slopes. The
synthetic (`relief_synth::residuals_on`) then stands on it: where the weight is 1 its smooth part is the band rather than
Catmull-Rom's, its gullies run down the band's slopes and follow its heights, and its octaves longer than 60 m and its
beds fade out (`Params::under_m`, `under_kept`: GLO-30's own shape stands for them); where the weight is 0 (the canopy,
roofs, GLO-30's voids, a page GLO-30 cannot be read for offline) it is the synthetic whole, the two blended over the
mask's 60 m fade. Its gullies are a residual as before (zero at the samples), so only the band moves them. Two options
(`relief_combined::Options`, `NAVAL_RELIEF_COMBINED="cap=35,seam=60"`):

- *The samples held*: the band moves the window's samples by up to 106 m (Lauterbrunnen's walls, where the window and
  GLO-30 put the cliff's top in different places); what it would move a sample past a soft 35 m (`capped`, near
  unchanged under half of it) is taken off as the bilinear surface through the samples (`hold_samples`), so the band's
  shape between them stays. It costs a tenth of the 40-160 m band in the Alps (4.0 m to 3.6 m RMS), a twentieth
  elsewhere.
- *The fills' edges*: where GLO-30's heights meet its fills, the two models can step. Within 12 m of such an edge the
  band's 16-40 m and 4-16 m bands are 1.1-2 times as strong as 80 m from one (Norway's twice); within 60 m of one the
  band eases to a 60 m box mean, which takes Rinjani's edges to the far ground's and the Alps' and Norway's halfway.

**Held to the simulation and off the water** (naval-landscape `relief_limit`, every made page: synthetic, GLO-30 and its
band, combined). The simulation flies its aircraft over the release's heights, a bilinear surface on its 2.25″ lattice
(naval-sim `land::release`, `DataSquare::height` and `Block::bilinear_m`), keeping 60 m and 1.25 times their
pull-out's height above the highest of three lanes of their track read every 20 m (`aviation::aircraft_terrain`'s
`CLEARANCE_M`, `terrain::RAY_STEP_M`); the window is that surface resampled at 40 m. A made page may stand at most
4 m above the window at any texel (`RAISE_M`): a fifteenth of the clearance, under what a real page or the window's own
resampling already puts above the release (below). A page is made 18 texels past its own (`MARGIN`, the beds' rock
read at the nine pages' corners) so two pages agree where they meet, then held (`hold`):

- *A broad excess lowered whole*: the excess over 4 m where it covers most of the 52 m about a texel (a crest the band
  raises) is spread 24 m, blurred within that and taken off, so the crest keeps its shape a few metres lower and its
  flanks are carved a little more.
- *The rest eased*: what then stands above 2.5 m (a lip a Catmull-Rom curve throws over a cliff's top, a gully's
  rounding, a crest's very top) eases toward 4 m and never reaches it (`ease`, its slope continuous). Below the window
  a page carves as before.
- *Off the water*: no made detail on WorldCover's water nor on the waterways and water areas the features draw
  (rivers, canals, riverbanks, docks), nor on OpenStreetMap's lakes and ponds (`Water`, read with the page from the
  surface's and the map's caches), coming in over 50 m from them (15 m from a stream, drain or ditch); and none at
  the sea's level, all of it 50 m up (`SHORE_M`), so a fjord's foot keeps the window's slope.

Against the release itself (the report's `release` heights, the simulation's own reads at every texel), the window
already stands up to 9-71 m above it at a few texels (its summits raised to the lattice's, its 40 m chords across
cliffs) and a real page up to 108 m (a 3DEP or Kartverket cliff's top between two samples); the held pages add at most
the allowance to that. The drawn land less the window at the samples (all, crests, 0-50 m up: mean, RMS, 99th
percentile and largest of the size) and its highest over the window and over the release at any texel, with the share
of the land standing more than 4 m over the release (the ignored `report_combined` test, 2 × 2 pages about two places
a region):

| 4 km about | Source | All samples | Crests | 0-50 m up | Over the window, the release (m) | Over the release by 4 m |
| --- | --- | --- | --- | --- | --- | --- |
| The Eiger and Lauterbrunnen | the window | | | | 0, 18.7 | 0.27 % |
| | synthetic, unheld | 0.0, 0.5, 1.7, 6.2 | +0.2, 0.6, 1.6, 4.5 | | 19.0, 21.2 | 4.4 % |
| | synthetic | -0.1, 0.6, 2.5, 11 | 0.0, 0.9, 4.4, 11 | | 4.0, 16.3 | 1.3 % |
| | combined, unheld | 0.0, 6.5, 27, 39 | +1.9, 9.6, 31, 37 | | 72.3, 71.4 | 14.8 % |
| | combined | -2.2, 5.4, 23, 42 | -3.8, 7.2, 25, 36 | | 4.0, 16.3 | 0.76 % |
| Rinjani's cone and north rim | the window | | | | 0, 9.1 | 0.23 % |
| | combined, unheld | +0.1, 6.1, 21, 33 | +2.2, 7.8, 27, 33 | | 45.7, 47.2 | 19.3 % |
| | combined | -2.5, 5.1, 18, 33 | -3.0, 5.3, 17, 28 | | 4.0, 9.0 | 0.67 % |
| Taroko and the Qingshui cliffs | the window | | | | 0, 13.4 | 0.60 % |
| | combined, unheld | 0.0, 0.8, 2.2, 18 | +0.2, 0.6, 1.9, 9.7 | | 22.4, 24.5 | 6.9 % |
| | combined | -0.2, 0.9, 3.7, 18 | -0.1, 1.1, 4.4, 15 | | 4.0, 14.2 | 2.3 % |
| The Inner Gorge and Zoroaster | the window | | | | 0, 29.7 | 1.6 % |
| | 3DEP | 0.0, 1.0, 3.7, 7.7 | +0.4, 1.2, 4.4, 7.6 | | 102.9, 99.2 | 10.5 % |
| | combined, unheld | 0.0, 3.2, 15, 36 | +0.1, 3.4, 16, 31 | | 38.1, 46.2 | 12.0 % |
| | combined | -1.1, 3.0, 13, 31 | -1.5, 3.7, 15, 31 | | 4.0, 19.1 | 2.9 % |
| Geiranger and Dalsnibba | the window | | | | 0, 70.5 | 1.2 % |
| | Kartverket's 10 m | 0.0, 0.6, 2.6, 10 | +0.2, 0.5, 2.0, 5.6 | -0.5, 1.3, 5.0, 6.3 | 107.8, 107.7 | 3.5 % |
| | combined, unheld | 0.0, 2.1, 7.3, 34 | +0.7, 2.8, 8.6, 34 | -0.8, 3.9, 19, 34 | 41.3, 64.1 | 4.8 % |
| | combined | -0.4, 2.1, 8.0, 32 | -0.4, 2.8, 13, 32 | 0.0, 0.2, 0.8, 1.8 | 4.0, 59.1 | 1.2 % |

The held band lowers more samples than it raises (a mean 1-2.5 m down, 3.8 on the Alps' crests): what it would have
raised is gone, what it carves stays. GLO-30's band alone, held, is the same (its highest over the window 4.0 m
everywhere). What reads the window rather than the land drawn: the simulation (the aircraft over the land, the ships,
the guns' line of sight in the simulation), `theatre_landHeight`, the bases' runways and aprons (`Ground::height`). What
reads the land drawn: the camera's clearance (raised only: `relief_heights` never lowers it below the window's), the
sight's ray on the land, the trees, lamps, casters, buildings and infrastructure. So an aircraft the simulation flies
low over a crest has the 60 m it keeps less at most 4 m of made rock (a real page's cliff can still stand higher); the
camera never goes into the land.

The held pages' shape against the unheld (the same report; the RMS by band of wavelength, m, over the land; a page's
time on the `dev-client` build, other sessions running):

| 4 km about | Source | Steeper than 30°, 45° | 40-160 m | 16-40 m | 4-16 m | ms a page |
| --- | --- | --- | --- | --- | --- | --- |
| The Eiger and Lauterbrunnen | synthetic, unheld | 52.3 %, 24.3 % | 1.57 | 1.18 | 0.86 | 117 |
| | synthetic | 51.9 %, 23.9 % | 1.45 | 1.11 | 0.82 | 169 |
| | band with the fills, unheld | 46.2 %, 20.8 % | 3.84 | 0.60 | 0.52 | 61 |
| | band with the fills | 46.0 %, 20.3 % | 2.51 | 0.60 | 0.43 | |
| | combined, unheld | 51.2 %, 23.1 % | 3.56 | 1.04 | 0.82 | 179 |
| | combined | 50.6 %, 22.4 % | 2.42 | 1.00 | 0.76 | 247 |
| Rinjani's cone and north rim | synthetic, unheld | 57.9 %, 20.2 % | 1.66 | 1.26 | 0.95 | 118 |
| | synthetic | 57.2 %, 19.7 % | 1.56 | 1.20 | 0.91 | 167 |
| | combined, unheld | 59.7 %, 18.3 % | 3.83 | 1.02 | 0.84 | 177 |
| | combined | 56.9 %, 17.5 % | 2.57 | 0.98 | 0.78 | 239 |
| Taroko and the Qingshui cliffs | combined, unheld (forest: the synthetic) | 75.8 %, 31.7 % | 1.89 | 1.40 | 1.01 | 188 |
| | combined | 75.4 %, 30.8 % | 1.72 | 1.29 | 0.95 | 254 |
| The Inner Gorge and Zoroaster | 3DEP | 47.5 %, 18.9 % | 3.64 | 2.42 | 1.48 | |
| | combined, unheld | 52.3 %, 21.4 % | 2.75 | 1.38 | 0.97 | 191 |
| | combined | 51.7 %, 20.2 % | 2.10 | 1.23 | 0.88 | 255 |
| Geiranger and Dalsnibba | Kartverket's 10 m | 36.8 %, 14.1 % | 2.02 | 1.05 | 0.60 | |
| | combined, unheld | 41.4 %, 16.4 % | 1.99 | 0.95 | 0.71 | 161 |
| | combined | 41.2 %, 15.9 % | 1.50 | 0.88 | 0.67 | 225 |

Holding costs the band a third of its 40-160 m relief (the half that raised, less what the lowering keeps), the finer
bands 5-10 % (the rivers' and streams' fades, the eased tops) and the synthetic under a tenth. A held page takes 45-65
ms more: its wider grid 15 ms, the water's distances and the shore 19 ms, the hold 15 ms (`times_of_the_parts`); the
water's reads 1-9 ms. A combined page took 150-180 ms unheld on a quiet machine, the synthetic 100-115.

In the stills (`.build/combined-relief/`: a sheet a region, full-size rows in `rows/`), the combined keeps the band's
buttresses and couloirs on the Eiger and the radial ribs and notched top of Rinjani's cone, the synthetic's fine
gullies over them, and loses most of the synthetic's 240 m and 120 m combing on the faces GLO-30 holds. Taroko and the
Qingshui cliffs (forest) are the synthetic. No step shows at a fill's edge (each region's `seam` view, within a
kilometre of its pages' sharpest edge), with or without the easing. Against Kartverket's and 3DEP's pages at Geiranger
and the Inner Gorge it draws the same walls and benches more softly, and moves them a little where GLO-30 and the
window disagree. The frame at the Eiger's face is unchanged (median 5.60 ms off, 5.69 ms combined: three runs of 120
frames, `dev-client`, 1920 × 1080). So `auto` takes the combined where no finer model covers (the synthetic alone
offline). What it leaves: the broad cones above the band's 330 m stay the window's (the tops are notched, the domes'
mass stands); before the hold above, the synthetic's gullies crossed a river's bed as glossy ripples (the Colorado in
the Inner Gorge, the synthetic alone as well) and left a pale rocky apron along a fjord's foot (Geiranger; not the
real page's).

Held, in the stills (`.build/relief-safety/`: unheld | held, the combined forced, a sheet a region, full-size rows in
`rows/`): the Colorado in the Inner Gorge is one smooth surface, its ripples and the rock standing through them gone.
The Eiger keeps its buttresses and couloirs, shallower, the lower face's ribs softer (the band's lost third); Rinjani's
cone and rim and Geiranger's walls barely change. The pale apron along Geiranger's foot stays, its edge a little
smoother: the land drawn there is the window's (0-50 m up, 0.2 m RMS off it), so the apron is the window's gentle foot
under the ground's shading, not made rock, where a real page's cliff runs steeper into the water. The frame at the
Eiger's face is unchanged (median 5.86-5.91 ms unheld, 5.77-5.84 held: three runs of 120 frames each, `dev-client`,
1920 × 1080); the hold costs the worker, not the frame.

**A page** is 512² texels of 4 m (504² of its own, a gutter of 4 from each neighbour), 2,016 m a side, fixed in the
theatre's metres; the host holds 8 × 8 about the camera (about 8 km each way) and makes only those over the window's
land that a source covers. A texel holds the finer model's height less the window grid's own bilinear surface through
the finer model's heights at the grid's samples (the window's lattice, `Lattice`): what the finer model holds that the
grid cannot. Added to the window's heights it gives the finer model's shape wherever the two agree at the samples and
the window's heights wherever they do not, so a page never steps where it ends and never moves the land's broad shape
(a page is made for one lattice; a window cut on another makes it again). Heights below -5 m are taken as -5 (the
Terrain Tiles have spikes tens of metres deep off some coasts), and the GSI's no-data as the sea. `ReliefPage` keeps
three levels (each the mean of 2 × 2 of the last) and each level's slopes, central differences encoded `g / (1 + |g|)`
in a signed byte (`relief::encode_slope`: steps under 0.005 near level ground, cliffs to 50).

**On the GPU** two array textures of 64 layers, bound at the terrain group's bindings 16 and 17 (`RELIEF` in the land,
tree, lamp and caster shaders): the residual heights (`R16Float`, 44 MB with their levels) for every stage, their
slopes (`Rg8Snorm`, 44 MB) for the land's fragments. Page (i, j) lies in layer (j mod 8) · 8 + (i mod 8);
`terrain.relief` holds page (0, 0)'s chart corner, the texel and the slots, `terrain.relief_slots[32]` the page each
layer holds (two a vector), `terrain.relief_real` which layers hold a finer model's page (a bit each: the stepped beds
stand only under the others), and the land reads a layer only where its entry names the page at the point, fading out
over a page's outer 15 % toward a neighbour not held. The layer takes them on the High setting (`TerrainQuality::relief`) on
a device allowing two more sampled textures a stage than the land without them (18 to 20; the client's devices ask for
24); Medium neither binds nor compiles any of it. Pages upload in `prepare`, up to 2 MB a frame (1.4 MB a page), and a
page coming in or going marks the trees for placing again and its rectangle of the forest's shade for baking, with
its neighbours' feather about it (their share of the relief changes too).

**Drawing it** (`bindings.wgsl`'s `relief_height`, `relief_gradient`; `terrain.wgsl`'s `place_vertex`):

- *The waterline.* Land takes none of the relief at or below the sea, all of it from 4 m up (`RELIEF_SHORE_M`), and is
  never lowered below 0.15 of its window height (`RELIEF_FLOOR`): the bake's coast is where it was, whatever the finer
  model says of the beach.
- *The vertices* add the residual read at the level their spacing spans. Level-0 patches with land within a quarter of
  level 0's range (2 km at 1080 lines and a 55° lens, 3 km at 1620; at most 3 km, through binoculars too) are drawn
  dense: copied into an instance buffer of their own (`lod::DENSE_LAND`, `DENSE_COPY`, `DENSE_SHARE`, `DENSE_MAX_M`) and
  drawn with a 33² mesh (10 m on High) before the other patches, the plain draw leaving them out; their in-between
  vertices slide onto level 0's over the outer part of that reach, as a level's onto the next, so a dense patch meets a
  plain one with the same edge. The selection widens a patch's bounds by the highest residual held, so a lifted crest
  is not culled.
- *The fragments* add the page's slope, filtered to the pixel's footprint (one tap, levels of 4, 8 and 16 m), to the
  window's gradient: the land's normal and its materials' choice (rock on the steep) see the cliffs to 4 m.
- *What stands on the land* (trees, lamps, the land's shadow casters, the forest's shade) reads `height_at`, which adds
  the residual at level 1 (8 m, one bilinear tap). The casters' selection for the sun's maps widens the land's bounds
  by the highest residual held, as the view's does.
- *The camera* clears the land as drawn: the theatre hands the camera rig the window's heights raised by the relief
  (`TerrainLayer::relief_heights`, a snapshot of the pages drawn each frame, `ReliefHeights` reading level 0 on the
  CPU as `relief_height` does), never lowered below the window's.

**Streaming** (naval-landscape `relief`, native/docs/theatre.md's pattern for near tiles): a worker below the frame
loop makes the pages asked for, nearest first, four of a row together (they share their tiles), reading each source's
PNGs through the surface's fetchers. Tiles are kept on disk as fetched, a marker where the source has none, under the
surface cache's `relief-dem-v1` (`NAVAL_SURFACE_DIR` moves it), held under 512 MB (`CACHE_BYTES`: past it the least
recently read go, down to three quarters; checked when the worker starts and every 256 tiles fetched); 96 decoded tiles
and 72 made pages stay in memory. Nothing blocks a frame: a page not yet made is drawn without relief, and one comes in
faded over its edge toward a missing neighbour. A failed batch waits a minute (an offline error marks the surface
streaming offline, as its own fetches do). Switched with the surface streaming; `NAVAL_RELIEF=off` turns it off alone. `theatre_state.relief` gives the
pages drawn, uploading and still to make, the patches drawn dense, and the worker's state (pages made, the last batch's
time, tiles and bytes fetched, the cache's size).

**Costs** (Apple M5 Pro, 1920 × 1080, Ultra, the theatre's own scenes; the scratch harness's `NAVAL_AUDIT_TIME`: the
GPU's busy time with the relief shown and hidden in the same scene, five alternating 30-frame batches a side; load
average 30-140 from other sessions, so upper bounds):

| View | Patches dense | The frame, 10th percentile | The scene's opaque pass, median |
| --- | --- | --- | --- |
| a ship's bridge, Palos Verdes 2-3 km off | 0 | +0.25 | +0.06 |
| the same through binoculars | 1 | +0.16 | +0.12 |
| 40 m up, 1 km off Point Vicente's bluffs | 21 | +0.19 | +0.16 |
| a plane at 150 m off Palos Verdes | 10 | +0.19 | +0.13 |
| a plane at 500 m | 0 | +0.08 | +0.08 |
| a ship's bridge off Diamond Head | 22 | +0.24 | +0.17 |
| 120 m up, 1 km off Diamond Head | 34 | +0.24 | +0.26 |
| 700 m over Hakone's mountains | 1 | +0.21 | +0.16 |
| a plane at 1,500 m toward Hakone | 0 | +0.13 | +0.10 |
| a plane at 150 m off Dover | 19 | +0.11 | +0.15 |
| 30 m up, 1 km off Dover's cliffs (the dearest) | 42 | +0.30 | +0.38 |

The first cut cost up to 1 ms (binoculars, Hakone, Dover close): dense patches to half level 0's range drew 2.5-pixel
triangles at their edge, and through binoculars a quarter of the range is the whole block (171 patches). The quarter,
the 3 km cap and drawing the dense patches first brought it to the table's. The slopes' texture replaced four
trilinear taps of the heights a pixel (the opaque pass's share of the relief with no patches dense is about 0.1 ms).
Memory: 88 MB on the GPU (the dense mesh and instances 37 KB more), about 125 MB of pages and tiles in memory, and the
disk cache under 512 MB (166 MB after the seven places' stills below). Pages take 22 ms a batch from the disk cache,
0.3-1.1 s fetched.

**Checked.** `bun run rust:test -- --native -p naval-terrain --lib relief` (a page's levels, its slopes and their
encoding, its NaNs, the grid's layers and texels), `-p naval-terrain backends` (the shaders with `RELIEF` on every
backend), `-p naval-terrain gpu fine_relief` (ridges finer than the field's samples change the land close up, patches
are drawn dense, hidden the land is as before, and Medium refuses a grid), `--native -p naval-landscape --lib relief` (the GSI's encoding, the sources' boxes, the pages' tiling, a plane
has no relief and a wall between samples stands out, the sea and missing tiles are none, the lattice's key, pages only
over land, the cache's bound; the synthetic's: a page the same each time, neighbours equal in their gutters, nothing at
the samples, none on flats, the shore and the sea, its depth bounded, its parameters' spec). Stills: 32 views of seven places (Palos Verdes and Malibu; Hakone, Izu, Miura; Diamond
Head and the Pali; Bergen; Dover; Malta; Gibraltar) and three close views, from a ship's bridge, binoculars and planes
at 150, 500 and 1,500 m, before and after in `.build/land-lanes/stills/FR/` (the lead session's checkout). Point Vicente
and Dover's cliffs stand as faces with a sharp top edge and the gullies of the finer model; Diamond Head's crater rim
and its ribs are crisp; Izu's sea cliffs are rock; Malta and Gibraltar are unchanged (no source there).

**Open.** Buildings stand on the relief since 2026-10-14 (buildings.md, "Buildings", "Footing"); the window's heights still carry
them past its reach. On Dover's chalk the finer model's ledges take grass by their slope, as green bands across the face. Beyond 3 km
(binoculars) the relief is in the shading only. The cliff lines (OpenStreetMap's `natural=cliff`) are not used yet.
The synthetic alone cannot change the land at the window's samples, so under it a summit's silhouette and a face's
broad shape stay the window's (a dome stays a dome, fluted; the combined's band moves them below 330 m, by up to its
soft 35 m, which the simulation does not follow); its beds are fainter and more regular than real cliff bands, and its
gullies never meet in a drainage network (each octave runs down the slope it is given, nothing routes water). Its pages
cost six times a real page's CPU; a coarser grid for the 240 m and 120 m octaves would halve it.

History (measurements, stills, reviews and the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
