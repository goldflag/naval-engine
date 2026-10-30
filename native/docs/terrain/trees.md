# Trees

Trees stand to the end of level 1 of the land's patches, placed by a compute pass and thinned with distance, and are
drawn by one indirect draw; past them the canopy is painted into the ground. Each species is a procedural model, drawn
as an octahedral impostor, and as a mesh within about 250 m ("Near trees"). Within about 8 km of the camera a theatre's
trees stand on the crowns a canopy height map measures ("Trees where real trees stand").

**Placement** (`shaders/tree_place.wgsl`, a compute pass in the layer's `prepare`). The level-0 patches with land come
first in the patch buffer, as before (`TerrainLod::tree_patches` is lifted from the web's 256 to the buffer's capacity;
its default keeps the pins), and the layer moves the level-1 patches with land in behind them. One workgroup a patch,
a thread a slot (16 × 16): a level-0 slot is a 20 m cell, a level-1 slot covers 2 × 2 cells. The thread finds its cell,
the jittered foot and the ground there, decides from the level of detail and the view whether the slot keeps a tree at
all (a few hashes and the height: most threads stop here), and only then runs `tree_cover` at the foot, once, and
appends the standing tree (foot, height, width, kind, variant, open sky, tint: 48 bytes) to an instance buffer of 2^18
trees, counting it atomically into the instance count of the trees' `DrawIndexedIndirectArgs` (a full buffer takes the
count back). The draw is one card instanced by that count (`draw_indexed_indirect`): the CPU never learns how many
stood; `TerrainLayer::tree_draw_arguments` hands the buffer to a test or probe to copy out. The pass runs when the view,
the clearing or the surface maps change: every frame of a moving camera (and with temporal anti-aliasing's jitter every
frame). Medium has no trees: no pass, no buffers.

**Level of detail.** Every cell has a stratified rank: one cell of each 2 × 2 block (the block's hash picks which) ranks
below 1/4, one of those in each 4 × 4 block below 1/16, one in each 8 × 8 below 1/64, and within its tier a cell's rank
is its own hash. A share of the slots keeps its tree, the lowest ranks first, so a thinned forest keeps one tree a block
(neither a lattice nor random clumps), and a level-1 slot stands its tree in the cell of its block that ranks below a
quarter: the very tree a level-0 patch would stand there. The share, at the game's 52° lens:

- all of them within 4.5 km (`TREE_FULL_M`); beyond, (4.5 km / d)², which keeps their spacing on screen (a quarter at
  9 km);
- the distances are apparent: a narrower lens sees the trees further by the square root of its magnification (12×
  binoculars: 3.5 times; as far as it magnifies stood every tree of a whole hillside);
- past 2 km (apparent), a grazing view thins them with the sine of the view over the ground at the foot, to a third
  (0.3 and steeper keeps all): rows standing one behind another on screen make the same wall of crowns with fewer;
- down to a quarter by the end of level 0 (from 70% of its range), where the level-1 patches, which hold only the slots
  ranking below a quarter, take over: no tree is lost as a patch changes level;
- to none over the last 30% of their range: 12 km (`TREE_RANGE_M`, by the lens as above) or 92% of level 1's.

The trees still standing grow by d / 4.5 km, at most 1.35× (`TREE_GROWTH`), so a thinned forest keeps most of its
cover; the painted canopy carries the rest, as it did past the old ring. A tree leaving shrinks away over the last 30%
of its share (its rank against the share), so none pops, and none dithers. The first try thinned from 2.2 km and grew
the survivors 1.45×: Kawah Lagoon's island from 300 m up turned from rainforest into savanna (cards on a pale ground);
full density to 4.5 km keeps the closed canopy a forest is from the side.

**Look.**

- *Backlit crowns.* The card's normal faces the camera, so with the sun behind a tree the whole crown faced away from
  it and kept only the sky's light about a near-horizontal normal: black from a few hundred metres. A crown is lit as
  foliage now: the sun wraps round it past its terminator (`WRAP` 0.5: its leaves face every way); shines through its
  leaves toward an eye that looks toward the sun (the fourth power of that angle, more at the crown's thin edge than
  through its middle, yellower than the leaves seen lit); and the sky and the fill are taken about the normal bent up
  (0.45), as a crown's top sees the sky. Kawah Lagoon's island from 300 m up into the sun was near black and is dark
  green.
- *Each tree its own shade:* brightness 0.72-1.2 and a hue from a little yellower to a little bluer (red ±14%, blue
  ∓20%) by its cell's hash, over the style's tint; height 0.75-1.25 and width 0.85-1.15 as before.
- *Four variants of each kind* (the atlas 512 × 1024): rows 0 and 1 the web's; row 2 a broad, flat-topped crown (an
  emergent, an oak in the open) and row 3 a tall narrow one (an ash, a poplar). Replaced on 2026-10-05 by the species'
  impostors ("Impostors" below).
- The edge is the alpha test as before on the premultiplied atlas: no halo against a photograph's ground.

**Costs** (Windows, NVIDIA GeForce RTX 5070 Ti, release, 1600 × 900, 2026-10-04, other agents building). The stills
example's `--tree-cost` times frames of the land with its trees against the same land without (serialised, 24
alternating pairs, medians; about ±0.15 ms of noise): before, 0.06-0.9 ms (Iron Bottom Sound's binoculars the dearest at
0.91 ms); now 0.0-0.55 ms on every view of five maps but the binoculars, 0.9-0.96 ms there (fragment-bound: 56 000 cards
magnified 12×; 1.85 ms at 2560 × 1440). Every corner of every card evaluated the masks before; now a slot's are evaluated
once, and only where a tree may stand. In the theatre's cruise scene off Guadalcanal with the real world's surface (the
client's own GPU timestamps, one frame; the whole frame's busy time, then the scene's opaque pass, before → now): from a
ship 5 km off 1.56 → 1.59 ms (0.52 → 0.52), at 300 m 2.94 → 2.93 (2.15 → 2.09), at 1,500 m 3.58 → 3.36 (2.82 → 2.56),
500 m over the forest 3.28 → 3.24 (2.69 → 2.60), with 550-786 patches carrying trees where 214-256 did; the placement
pass itself 0.017-0.040 ms. Memory: the instance buffer 12.6 MB (2^18 trees), the atlas 2.8 MB with its mips (1.4 MB
before).

## Impostors

No 3D tree models exist, so each species is modelled procedurally and drawn as an **octahedral impostor** (Ryan Brucks'
octahedral impostors), chosen over crossed cards: a crossed card shows its cross and a flat disc from above, while an
impostor shows the tree as it looks from that direction, with a normal and a depth to light it by, from one card a tree.

- *The models* (`src/trees/species.rs`). Each species is built in metres as it stands: the trunk and limbs as tapering chains
  of capsules, the crown as lobes of leaf clumps (spheres), arranged per species (a broadleaf's lobes over an
  ellipsoid, a conifer's drooping whorls on a cone, a pine's flat pads on a bare bole, a palm's arching fronds, a
  mangrove's prop roots, a bare tree's recursive limbs and a haze of twigs), each clump's colour varied and its ambient
  occlusion found on the CPU (12 rays toward the sky against the other clumps). Normalised: the foot at 0, the top at
  1, its crown's reach from the axis (`Model::radius`) the card's width. 60 to 430 shapes a model, built once a
  process (`species::models`). Since 2026-10-26 each is grown into limbs and leafy sprays before it is drawn or baked
  ("Grown trees"). The coconut palm (2026-10-26; the tall type, measured: a trunk 15-25 m, 25-40 cm across over a foot
  swollen to about 60 cm, 25-35 living fronds 4-6 m long in a crown 8-10 m across) was 17 fronds of 4.6-5.6 m drooping
  from straight poles, and a beach's row read as a pale fence from the sea. Now, at 23 m: a trunk of 20 m, 30-36 cm
  across over a foot of 62 cm, its foot leaning 10 degrees along its own x and its top turned up (the placement lays
  the curve along its lean: "Each tree its own form"); thirty living fronds a golden angle apart, 3.9-5.2 m, narrow at
  the stalk and widest over the middle, their leaflets hanging from the rachis in a V turned down (`ribbon`; a date
  palm's rise), the youngest rising 60 degrees and arching over, the next level and arching down, the oldest from 40
  degrees below level hanging, and four dead ones brown beneath: a crown about 10 m across and 8.6 m deep. An atoll's
  groves and the Philippines' smallholdings widen it less (1.1 and 1.15, were 1.25 and 1.3). Its mesh is twice the old
  one's (34 ribbons); its costs held (`--tree-cost` on Kawah Lagoon's beach, air50 and grove against the build with the
  bark turned out, alternately: within 4%). Up close its fronds are still solid: the frond sprite's 46 leaflet pairs,
  stretched down a 5 m ribbon, overlap (a coconut frond has over a hundred a side), and the sprite atlas's twelve slots
  are full.
- *The bake* (`shaders/tree_bake.wgsl`, `shaders/tree_resolve.wgsl`, `src/trees/impostors.rs`), in the layer's first
  frame: each species' full mesh ("Grown trees") drawn orthographically from 8 × 8 directions over the upper
  hemisphere (hemi-octahedral, the corner frames on the horizon), one instance a frame, at twice the atlas's
  resolution, each pixel its spray or limb shaded as the near trees shade it (`shaders/tree_surface.wgsl`), then
  resolved two by two (the covered samples averaged) into frames of 96 × 96 texels (64 × 64 until 2026-10-08, "Near
  trees"): a 768 × 768 array texture, layer k species k's colour (sRGB-encoded, premultiplied by the coverage) and
  coverage, layer models + k its normal (model space, octahedral), its depth along the frame's direction and the sky
  its crown leaves the point with its material, premultiplied by the coverage as the colour is (`impostor::pack_material`:
  bark's sky over 0.45 → 0 the more open, leaves' over 0.55 → 1; each texel keeps the material more of its samples, or
  of its level's coverage, are), and the last three layers the near trees' leaf sprites. Until 2026-10-26 (the
  impostors' packing) the two halves met at 0.5 and a texel averaged both: a leaf under a sixth of the sky read as half
  bark under half the sky, a blend of a trunk and leaves as bark under the open sky, and an empty texel's open sky
  leaked into a crown's filtered edge, so the impostors drew brighter than their meshes. Until 2026-10-26 each model was
  ray-cast from its capsules and spheres instead, a leaf clump a sphere opened by a noise of its own size. Then the mip
  levels down to 3 texels a frame, 6 in all (`shaders/tree_mips.wgsl`; a level further would split a texel between two
  frames): the colour averaged premultiplied, the normal, depth and occlusion over the covered texels only. A frame's
  rectangle is what a cylinder of the model's radius from foot to top covers seen from there (the radius across,
  (cos e + 2 radius sin e) / 2 up at elevation e), so a tall narrow tree is not drawn into a square.
- *The draw* (`shaders/trees.wgsl`, `shaders/impostor.wgsl`). A tree is one card through its middle facing the eye,
  over the same rectangle for the eye's own direction (in the model's frame, after the tree's own yaw and width), so it
  is no larger than the old card from the side and the crown's disc from above. The three frames nearest the view are
  blended (the hemi-octahedral grid's triangle and its barycentric weights, per tree, so nothing pops between frames; a
  crown under about 30 pixels tall takes the frame that weighs most alone; its costs are in the archive);
  each frame's coordinates are the card point projected along that frame's direction, affine across the card, and held
  inside its frame. The coverage is the alpha test at 0.45 as before, sharpened by its screen derivative. The normal,
  depth and occlusion are blended where each frame covers; the depth gives the surface point each frame sees, so the
  shadow lookups, the fog and the crown's height are the leaf's, not the card's. Up close, where a frame's texel spans
  pixels, leaves finer than the texel are a noise fixed to the tree (0.5 m) over the colour.
- *Light*, as the cards had it: the sun wrapped round the leaves (`WRAP` 0.5; bark takes it as a surface does) and
  shining through them toward an eye that looks toward the sun at the crown's thin edge, the sky and the fill about the
  normal bent up, the other trees' shadows read in front of the tree's own caster card, the forest's shade beyond the
  maps; the impostor's normal now gives the crown its shape, its occlusion darkens the inner crown (the sky's light by
  0.12 + 0.88 occlusion², the sun's by 0.45-1).
- *Into the sun's maps* (`shaders/tree_caster.wgsl`): the same card facing the light through the tree's middle, over the
  rectangle the model covers from the sun, alpha-tested on the three frames nearest the light's direction (where a map
  texel spans two of a frame's texels or more, the wide map's trees, the frame that weighs most alone, as the impostor
  takes it from as far; read at its level directly, 2026-10-26: the wide map's trees cost 0.1-0.2 ms less); ground cover
  under 1.2 m casts nothing.
- `cargo run -p naval-terrain --example stills --release -- --styles tropical --presets air300 --atlas` writes the
  baked atlas, six species a row: `.build/native/terrain/tree-atlas.png` (colour over grey) and `tree-atlas-normals.png`.

**Memory.** The atlas 768 × 768 × 63 layers of RGBA8 with its 6 levels: 198 MB (`impostors::atlas_bytes`; at 64² frames
512 × 512 × 62 layers with 7 levels, 86 MB, and 64 MB with the first 23 species; the 2D atlas was 2.8 MB); the models'
shapes 0.3 MB; the instance buffer as before (12.6 MB). Medium draws no trees and binds a 1 × 1 placeholder.

**BC7 after the bake** (2026-10-09, `mem-gpu-assets`). On a device sampling BC (the client asks for it; every desktop
GPU wgpu targets has it) the bake goes to the queue on its own encoder, and a thread of its own (`tree impostors bc7`)
reads it back eight layers at a time (25 MB of staging) and encodes every level on the CPU (`src/bc7.rs`: intel_tex_2's
fastest BC7 with alpha, four threads), into a `Bc7RgbaUnorm` copy of the same layout: 50 MB in place of 198 MB, made
in 1.7 s with other agents building (M5 Pro). Every land sharing the atlas binds the copy in the `prepare` after it is
ready (`Impostors::refresh`), so the bake is drawn for the first second or two; the RGBA8 bake is freed once the last
land holding it has rebound. The draw reads the same unorm values, so no shader changed; the frame size stays 96²
(64² would have taken the atlas to 86 MB and lost the near trees' detail). A device without BC, a wasm32 build, and
`TreeAtlas::uncompressed` (tests and `stills --atlas`, which read the atlas back) keep the RGBA8 bake. GPU tests:
`impostor_sharing.rs` (both, on a BC device and on one without).

**One atlas for every land** (2026-10-05, `goldflag/land-hitches`). The atlas never changes, yet each land baked its
own: a theatre scene, with its land, comes every 16 km of passage, on a re-centre and when the surface maps arrive,
and each first frame spent 12-15 ms of GPU on the bake (M5 Pro) and held another 64 MB until the old scene went. A
`TreeAtlas` is the atlas on a device; the client's session makes one on its first theatre land with trees and hands
it to every land it builds (`TerrainLayer::for_scene_sharing`). The first land to draw bakes it in its first `prepare`;
a land made after that registers no bake pipelines at all (`bakes_tree_impostors`). A land given none makes and bakes
its own, as before (tests, the stills). GPU test: `impostor_sharing.rs`.

## Species

32 models. A land with no place (`Climate::AUTHORED`: the tests' and the stills') keeps its style's mix: each old kind is one species, a quarter
of the broadleaf the other species the old atlas's broad or tall variant stood for, so only the look changes. A theatre's
land is drawn where it stands: `Climate::place` (latitude and longitude, set by `naval-client`'s `theatre::land::climate`)
gives its biome (`species::Biome::of`: the Köppen-Geiger class's zone where no box of a region's flora takes the
place, below), and the placement (`tree_place.wgsl`, `choose`) picks each tree's species from the biome, the WorldCover class at its foot (where
the surface maps know it), its height, slope and distance from the coast, moist hollows and gullies (`ground_of`'s
`gully`, `hollow`, `wetness`), exposed headlands (within 120-500 m of the sea, on a knoll or a slope), and stands of one
species a few hundred metres across (a noise at 650 m, groves at 1.7 km); deciduous species turn bare by the climate's
leaves as the styles' do. Size (0.75-1.25 of the species' height, lower in fields, towns and on headlands), width,
yaw and colour vary per tree.

The right trees for the place (2026-10-08, lane D of the land-realism programme, `goldflag/land-trees`). A theatre's
`Climate::place` is its window centre's (`LandJob::climate_for`), and the placement's view carries it (`TreeView::place`),
so each tree finds its own latitude (`latitude_at`): no redwoods south of Big Sur, palms in southern California's towns.
`choose` gives each tree a `Choice`: its species, height and tint, and how it stands: its chance (the masks' density,
raised where a forest closes or a plantation is planted), its width over the species' (a closed forest's crowns a little
wider than a slot, so they meet), its variety (a plantation's trees of one height and shade) and a plantation's rows (a
grid turned by the estate's angle, a palm a slot where the grid falls in it). Three of the eight newest models are
stands rather than one tree, so a 20 m slot holds a plantation's or a thicket's closed cover: the sugi stand (19
trees on a 4.2 m grid), the bamboo grove (30 culms) and the chaparral thicket (56 shrubs over 16 m). A town's trees are
the gardens' and streets' by the chance its built share gives (15-60%), so a leafy suburb's tree cover grows them too
(California, Japan). For lane E's mesh trees the new models are the same `Primitive` data as the rest
(`species::models`).

Five more regions (2026-10-09, lane RT of the regional land work, `goldflag/region-trees`): Britain, the continent's
north, North Africa, Egypt, the United States' north-east and south-east and Hawaii got biomes of their own, and the
Mediterranean's and the tropics' were reworked. A land window spans 96 km and a strait or a frontier can run through it
(Tarifa and Tangier, Dover and Calais, Gaza and Sinai, the Chesapeake), so each tree reads its biome from a grid about
the eye (`species::region_grid`: 16 x 16 cells of 3 km on the chart's own lattice, `Biome::at` at each cell's middle,
worked out again when the eye crosses a cell; `TreeView::regions`), the lookup moved by up to 0.4 of a cell per tree so
an edge on land is no line of squares. `Biome::at` takes the cell's climate from the Köppen-Geiger map (ground.md,
"Climate and rock maps"; `species::cell_climate`: the commonest class of the land within 1.5 km, of the classes trees
grow in where there are any, so a fjord's valley under a plateau above the treeline is Dfc, not ET), and
`Biome::with_class` decides: the flora boxes first (California's but its deserts, BW, and its high country, D and E, and
the Great Basin's side of the Sierra Nevada's crest; Japan, Hawaii, the Northeast and Southeast, North Africa and Egypt,
the Mediterranean, Britain, northern Europe, Iceland and the Faroes), then the interior West, then the class's zone: A
tropical; BWh, BSh and the cold steppe south of 37° arid (the cold steppe north of it temperate: Anatolia, Patagonia);
Cs in the Old World Mediterranean (the Pacific north-west's temperate); Cfa and Cwa south of 35° subtropical; the
tropics' C and E (within 23.5°, or 30° on a tropical or volcanic style) tropical; Dfc, Dfd and the other cold-summer D
boreal (Labrador, Siberia from 52° N), C and the warm-summer D temperate south of 57°, boreal north of it; subarctic
past 70° and on the tundra (E alone) past 45°; with no land in the cell (an islet the map misses) the latitude's bands
(`by_latitude`: tropical within 23.5°, or 30° on a tropical or volcanic style, subtropical to 35°, temperate to 57°,
boreal to 70°, subarctic beyond). The region grid's 256 cells read some 25 map cells each (under 0.2 ms when the eye
crosses a cell, not measured).

The interior West (`Biome::InteriorWest`): North America from the Cascades' and British Columbia's Coast Mountains'
crests (their wet western slopes' C classes stay temperate) and the Sierra Nevada's (California's box) to 104° W, 28-52°
N, every class but the hot deserts and steppe (BWh, BSh: Phoenix, Las Vegas, Death Valley, the Mojave), which are arid.
Its trees stand in belts by height (`west_belt`: as at 36° N, 100 m lower a degree north, 150 m lower on a shaded face
than a sunny one, the edges wandering ±110 m on a 300 m noise), from existing models: below 1,450 m the desert's few
junipers; pinyon (the Monterey pine model at 0.28-0.36, 7-9 m, wide and grey-green) and Utah juniper (the olive at 0.7,
blue-grey) to 2,150 m; ponderosa (the Scots pine at 1.15-1.4, 23-28 m, its warm bark) with Gambel oak (the oak at 0.4)
to 2,700 m; Douglas fir and white fir (the spruce, dark green and grey-blue), aspen (the birch) in its groves, then
Engelmann spruce and subalpine fir (narrow) to the treeline, lower above 3,300 m. Cottonwoods (the ash, yellow-green) on
the rivers' flat, wet bottoms below 1,900 m and by the fields below 2,300 m (not by the gullies' moisture other biomes
read, which would line every dry wash on the Coconino Plateau); towns' cottonwoods, elms and ponderosa. WorldCover's
tree cover decides where they stand: the South Rim's own cell is Dsa, its 3 km BSk, its 40 km 65% arid, all one biome.
The ground cover: sagebrush (the garrigue, silver grey-green) over shrubland, grassland and stony ground to 2,700-3,100
m, blackbrush and cliffrose (the shrub, grey-olive), bunchgrass tufts.

`Biome::of` is still the window's, for the culling sphere. Three new rules read the place more closely: the photograph's own woods (`photo_brightness`: the mosaic about the tree, the haze and its slope's
light under the photograph's late-morning south-eastern sun taken out, against the window's mean tree-cover colour; a
wood under 0.6-0.8 of it is a conifer wood: Sitka and Norway spruce, pine), the longitude (`longitude_at`), and groves
in rows on the Mediterranean's and Africa's farmland (260 m parcels: olives, citrus, date palms). One new model, the
date palm (`OasisPalm`, one more pair of atlas layers).

Bark and the blue gum (2026-10-13). A tree's tint (the placement's, a grey casuarina's or a paler cypress's, and its
own hash) is its leaves' colour: the bark takes only its brightness (`foliage::tree_tint`), where it took the hue as
well and a blue-grey tint turned a pale trunk slate. The sunlit ground about a tree throws light back up onto its
trunk and its crown's underside (the land's albedo about 0.12, warm, past the land's own shadow at its foot, as the
buildings take it); without it a trunk's shaded side saw only the sky and read navy. The eucalyptus forks at 11 m (14
before) into four leaders leaning out, each with six hanging lobes from a third of the way up it to its top: a
ragged crown half the tree's height, the sky between its lobes, on a paler trunk (sRGB 170, 162, 144), where it was
three small tufts on a pole.

Open crowns, each tree its own, and the woods' edges (2026-10-14, branch `goldflag/town-trees-realism`).

- *Crowns.* The coast live oak was one dense, dark dome of fourteen lobes (a head of broccoli from any distance). It
  now divides low into five sinuous limbs, each carrying its own masses at its elbow and its end (the low ones'
  tips hanging past it), with a top mass over the fork: the sky and the dark limbs show between them, and its
  leaves are a little lighter. The oak's (and beech's) crown is fourteen lobes of 16 clumps over a wider ellipsoid
  (twelve of 24 before), the ash's thirteen of 16, both more open, so they read as masses rather than one lumpy
  dome. The olive's leaves were twice as bright as any broadleaf's (a town olive in summer sun read nearly white in
  Istanbul): now sRGB 74, 86, 64 with its silver undersides at 100, 108, 88; the blue gum's paler colour a little
  darker too.
- *Each tree its own form.* A placed tree's yaw and form share one float of its instance (`packed_form`,
  `impostor::form_of`): the yaw the fraction of a turn, the whole part a code of its top's lean (east and south, in
  fifteenths of `LEAN_MOST`, 0.16 of its height; a palm's in fifteenths of `PALM_LEAN_MOST`, 0.64, marked by the code's
  bit 14, `PALM_FORM`, so its turn keeps nine bits) and its crown's stretch along its own x (up to `STRETCH_MOST`, 0.3
  wider, half that narrower across). Most trees stand within a degree or so of upright, one in seven leans up to six
  degrees, a tree on a slope a little downhill; a plantation's trees stand straighter, and a stand of many trunks, a
  thicket or ground cover stands upright. A palm leans far (2026-10-26; measured, a coconut palm mostly 5-30 degrees,
  rarely upright): a coconut palm 3-25 degrees (half of them under 11), a date or oasis palm 1-16, a fan palm 1-9, a
  plantation's rows about two thirds of that, out over the open ground at a wild wood's edge (within 70 degrees of
  it); a coconut palm turns its model's own curve along its lean, one in four against it (an S). The lean is an offset
  of its own in all three of a tree's pictures, as the wind's bend is (`tree_wind::lean_by`: from the foot, bowing a
  little toward the top; a palm's straight from its foot, as its card's two rows of corners lean, so its mesh and card
  agree through the crossfade), and the impostor's lit point takes it, so its shadows and fog are the leaning tree's. The stretch scales the model's two level axes apart
  (`Placed::across`), so a crown is wider one way than the other. The species stays exact in its own float, as the
  tests and the binning read it. The culling sphere (`layer.rs`) and the near binning's bound grow by the lean and
  the stretch.
- *A wild wood's dead and young.* `Choice::wild` (a wood's share of the land cover, none in a town, a plantation or a
  field) stands 1.5% of a wood's trees dead (`SNAGS`): a deciduous tree as its winter self, an evergreen broadleaf as
  the bare oak, a conifer with its needles browned, all a little greyer; and one in seven of its trees half to three
  quarters of the canopy's height (`YOUNG`; a crown the canopy map measures stands as measured).
- *The woods' edges.* A wild wood's tree with open land within 24 m (`EDGE_M`; `woods_about` reads WorldCover's class
  at four points about the foot in the window's maps) leans out over the open ground, its crown wider that way, and
  stands up to 15% lower. Outside it the ground cover stands a mantle: shrubs over the open ground along the edge
  (`MANTLE`), three in ten of them the wood's saplings, 3-6 m (`sapling`: ash or birch in Britain and the temperate
  north, birch or spruce in the boreal north (Scots pine beyond the spruce's range), holm or live oak about the
  Mediterranean and California, laurel in
  Japan, the rainforest's young in the tropics).
- *Hedges and thickets.* Where the canopy map covers a ground-cover cell, the open land's shrubs stand where it
  measures growth 2 m or taller over a share of the texel with no crown of 3 m or taller in it or the texels about it:
  a hedge, a thicket, a field's scrubby corner, and none scattered over a mown field or a lawn (none in a town's
  built-up land, where a texel's bush could stand on a building). On the period's country Britain's hedgerows and the
  East's fencerows stand as bushes along the fields' edges the land paints: every ground-cover cell within 2.7 m of
  an edge moves its bush onto the edge's line (the edge's distance rises away from it either side), 2-3 m tall in
  Britain and lower in a fencerow, dark in leaf and brown-grey out of it, so the hedge closes near the camera. In
  every region's period country a field's middle stands none of the shrubs today's canopy measures there and no
  wood's mantle (a 1942 field ploughed or grazed to its edge, where scrub grows now).
- *Bark in its crown.* A trunk seen from inside or under its crown read near black: bark's sky now falls to a floor of
  0.25 of it (a leaf's 0.12) with its occlusion, the sun on its face is dimmed to 0.2 deep inside (0.45 before), and
  the light the leaves let through reaches it from every side, green, an eighth of the sun at most. Near bark is
  rounder at full detail (4 to 10 sides, 5 for a limb over 5 cm), where a thin limb seen from inside a crown was a
  square prism.
- *Low in a crown's shadow.* Another crown's shadow takes 0.7 of the sun from a crown (`CROWN_SHADOW`: its leaves let
  some through) but 0.95 from foliage within 1.5 m of the ground, rising to 0.7 by 5 m up (`light_foliage`): the
  floor about it takes the shadow whole, and a bush in a hedge under its trees read lit against the shaded grass.

Cost: the scene's opaque pass (GPU timestamps, the median of three rounds alternating with master, 1600×900 on High,
the development Mac) 2.71 → 2.58 ms over Palos Verdes' oaks, 1.73 → 1.37 ms in Stansted's wood (its quickest rounds
1.38 and 1.36), 2.24 → 1.88 ms over Leigh Park's hedge (fewer scattered shrubs) and 2.60 → 2.70 ms over
Waterlooville's fields: within the rounds' spread. No memory: the atlas keeps its size, an instance its 48 bytes (the
form shares the yaw's float), and the near meshes grow 0.1 MB.

| Biome (where) | Trees | Ground cover |
| --- | --- | --- |
| Authored (no place: the test land) | tropical and volcanic: rainforest, emergent (a quarter), coconut palm; snow: spruce, birch (bare out of leaf); chalk: oak, ash (a quarter), bare oak, Scots pine | shrubs under and about the woods; the tropics' grass tufts on open ground; the snow style's heath cushions |
| Tropical (A; the tropics' C and E, within 23.5°) | rainforest closed where the cover is mostly woods (its crowns deep, from a third of its height, and wider than a slot; patches of taller and lower canopy at 170 m; 0.7 of its height at the shore, full 3 km inland), emergents out of it; coconut estates on the coastal plains (within 1.5-4 km of the sea, below 20-40 m, flat; 700 m parcels, six in ten planted, in rows of one age), in the Philippines up the hills too (below 300-450 m); the strand (within 200-450 m of the sea and below 5-9 m: an atoll's islets, the flat behind a beach) coconut palms, their crowns nearly meeting, over breadfruit and pandanus, no rainforest; coconut palms behind beaches and about villages and grassland, mangroves on WorldCover mangroves or low wet shore, montane forest lower | shrubs, tufts on grassland (kunai) |
| Arid (BWh, BSh; the cold steppe and desert south of 37°: the Sahara to the Gulf, Iraq and Iran, the Sonoran and Mojave deserts) | acacia; date palms (the Sonoran desert's fan palms) within 2.5 km of the sea and in towns and oases | shrubs, garrigue, tufts |
| Mediterranean (Iberia but its Atlantic north, the Midi, Corsica and Sardinia, Liguria and Italy south of the Po's plain, Sicily, Malta and the Pelagie islands, the Adriatic's east shore, Albania and Greece, Turkey's Aegean, Marmara and southern coasts, Cyprus, the Levant's coast, Crimea's south shore; the Cape) | groves in rows on the farmland below 500-800 m and on gentle slopes (a 260 m parcel of one age; 15-55% of parcels olives, 18% citrus on the irrigated plains, below 120-220 m and flat); maquis and garrigue on shrubland; towns' stone pines, cypresses, planes, Canary and fan palms, ficus and orange trees, eucalyptus, olives; planes (west of 7° E alders and ashes, the ash model) and poplars along streams; stone pines by the sea; Aleppo pine (the Scots pine model, paler and lower) in stands east of 4.5° W (western Andalusia's, Portugal's and Tangier's sandstone hills keep the cork oak); holm and cork oak, wild olive, cypress; black pine and oak above 800 m, black pine and fir above 1,400 m; eucalyptus groves and field lines in Portugal, Sardinia, Sicily (not Malta) and the Levant | maquis shrubs and garrigue on shrubland, tufts |
| North Africa (the Maghreb and Libya to Cyrenaica, south of a line through the Strait of Gibraltar, between Sardinia and Tunisia, south of Sicily, Pantelleria, Lampedusa and Malta, and south of Crete) | the Mediterranean's, with more olive groves (45%), citrus on the coastal plains (the Mitidja's), and toward the Sahara (from 34.6° N to 32.2° N) date palm groves (up to 45% of parcels) in place of olives, date palms about the land; towns' date palms, Canary and fan palms, ficus, eucalyptus, stone pines, flame trees; the Atlas' cedars | garrigue, dry tufts |
| Egypt (the Nile, its delta, Sinai, the Red Sea's west shore; Libya's Marmarica from 23° E: Tobruk, Bardia) | date palms along the canals and about the villages, casuarina (the Scots pine model, grey) and eucalyptus windbreaks, sycamore figs; a few groves (8% citrus and mango, 10% date palms); towns' ficus, date palms, flame trees, casuarinas; the desert bare | garrigue, dry tufts |
| Britain (Great Britain, Ireland and their islands, north of a line up the Channel: Dover, Dungeness and the Lizard in, Calais, Boulogne and Cherbourg out) | hedgerow and field oaks and ashes; oak, ash and beech (the oak model, brighter and wider) woods, birch on the heaths; conifer woods where the photograph shows them dark: Sitka spruce compartments of one age (350 m), Scots pine (more in the Highlands); the uplands' birch and Scots pine; dune pines; towns' planes, limes and sycamores, birches, Leyland cypress, pines; Orkney, Shetland, Caithness, the Outer Hebrides, Tiree and Coll all but treeless (low, wind-cut farm trees) | gorse (dark green shrubs), heather (purple-brown cushions) on heaths and moors above 150-300 m |
| Northern Europe (Brittany to the Baltic below 58.5° N, the Alps and the Balkans' interior, Spain's Atlantic north and northern Portugal, Turkey's Black Sea coast) | Britain's lowland rules (its hills' oak and beech, not Britain's upland birch), with Norway spruce and Scots pine (more on the sandy plain from the Netherlands to the Vistula) where the photograph shows a wood dark, north of 44.5-46.5° N (south of it, below the mountains, a dark wood is oak, beech and hornbeam: Thrace's Istranca and Belgrad forests); spruce and fir above 800-1,100 m; the dunes' Corsican and Austrian pine (the black pine model); the Landes' maritime pine and Galicia's eucalyptus plantations in compartments of one age, a tenth clear-felled (bare, with bare rides between, beyond the canopy map's reach); poplar rows and pollard willows on the polders and the Po's plain | heath, gorse |
| Northeast (the United States from the Potomac, 38.6° N, to Maine, and the Maritimes) | oak, maple (the ash), beech and birch woods, closed; white pine (the Scots pine model, taller and bluer) and hemlock (spruce, in moist ground) where the photograph shows a wood dark; Maine's spruce and fir; pitch pine and scrub oak on the sandy coast; red cedars in the old fields; towns' maples, oaks, London planes, white pines and spruces | shrubs |
| Southeast (the coastal plain from Hampton Roads to Florida and round the Gulf to Texas) | loblolly and slash pine (the Scots pine model) plantations in 380 m compartments of one age (55%), some clear-felled (bare beyond the canopy map's reach); mixed pine and hardwood; bald cypress (the redwood model, half height; rust-brown in winter) and tupelo in the swamps and bottomlands; live oaks, cabbage palms (the fan palm, half height) and pines in the maritime forest and towns; South Florida's and the Keys' leafy towns (ficus, mahogany and gumbo-limbo, live oaks, royal poinciana, coconut and royal palms standing out), hammocks and slash pine; mangroves on WorldCover mangroves and the low wet shore | saw palmetto (yellow-green shrubs) under the pines |
| Hawaii | towns' monkeypods (the acacia, twice its height), coconut and royal palms, banyans, shower trees and mango; the shore's coconut palms and ironwoods; kiawe and haole koa on the dry leeward lowlands; the wet windward valleys' closed forest (laurel and rainforest), eucalyptus groves, Cook pines; ohia above 900 m | haole koa, tufts |
| California (32-42.5° N, 125-116.5° W; central Chile; south-western Australia; not their deserts, BW, nor their high country, D and E, nor the Great Basin's side of the Sierra Nevada) | coast live oak; redwoods in moist canyons of the coast ranges (30-700 m, 0.4-25 km inland) only north of 35.6-36° N (Big Sur); south of 34.6-35.4° N (Point Conception) sycamores and live oaks in the canyon bottoms and coastal sage scrub (the chaparral thicket lower, greyer and half as dense) on grassy hills; chaparral thickets on shrubland, closed (wider than a slot on the flat, narrower and taller on a slope); towns' Mexican fan and Canary Island date palms (half the town trees in the south, an eighth in the north), camphor and ficus (the laurel), blue gums, sycamores or Monterey pines, live oaks; Monterey cypress and pine on headlands, Monterey pine stands near the coast, eucalyptus groves below 350 m (more about farms and towns) | chaparral shrubs (85% of shrubland's cells, dark olive), dry tufts |
| Japan (Kyushu to Hokkaido by island boxes, Tsushima, the Ryukyus, Taiwan; not Korea, whose red pine and oak are the temperate biome's) | south of 26-27° N (Taiwan, the Ryukyus) evergreen to the treeline: the laurel forest to 2,400 m at 24° N (1,500 m plus 130 m a degree south of 31° N), its oaks evergreen (the live oak), the red cypress (the redwood at 0.6) among them from 1,800 m, above it hemlock and spruce (the spruce, 72%), cypress and rhododendron (the laurel), Taiwan's fir (narrow) over rhododendron higher, the towns' banyans and camphors; elsewhere the laurel forest (Castanopsis, Machilus, camphor: the laurel's deep lumpy dome) closed below its upper edge (1,500 m in the south, 700 m about Tokyo, the sea by 38.5° N); sugi plantations in 300 m compartments (four in ten) on slopes of 30-1,600 m between 30.6 and 41.8° N, every slot planted, one height a compartment, hinoki (lower and bluer) in a third; black pine on the shore (within 350 m of it, below 40 m) and the headlands, and on knolls in the laurel forest; bamboo groves by the villages (below 150-300 m); beech and oak (bare in winter), birch and fir above the laurel forest; Hokkaido's spruce and fir; towns' camphor, zelkova and cherry (the oak and ash), pines and bamboo | dwarf bamboo (shrubs) |
| Subtropical (Cfa, Cwa from the tropics to 35°) | evergreen oaks, coastal pines (the Monterey pine model) by the sea and in stands, deciduous oak | shrubs |
| Temperate (C and the warm-summer D to 57°; the cold steppe north of 37°; the Pacific north-west) | oak, ash, birch, Scots pine on coasts and in stands, spruce and pine above 800 m | shrubs |
| Boreal (the cold-summer D to 70°; C and D from 57°) | spruce (more in wet ground), Scots pine, birch; beyond the spruce's range (`spruce_native`: in Europe from 66.6° N on Norway's coast to 67.7° in the Torne valley and 69.2° in Inari, Pasvik and Kola, fading over half a degree) Scots pine in stands with birch; mountain birch in a belt some 250 m deep under the treeline, which falls with the latitude (`boreal_treeline`: 950 m at 63.5° N, 550 m at 68.5°, 430 m at 70°) | heath cushions |
| Interior West (North America east of the Cascades' and the Sierra Nevada's crests to 104° W, 28-52° N, but its hot deserts) | in belts by height (at 36° N, 100 m lower a degree north, 150 m lower on shaded faces): desert junipers below 1,450 m; pinyon (the Monterey pine at 0.3) and juniper (the olive, blue-grey) to 2,150 m; ponderosa (the Scots pine at 1.15-1.4) with Gambel oak to 2,700 m; Douglas and white fir (the spruce), aspen groves (the birch), Engelmann spruce and subalpine fir to the treeline; cottonwoods (the ash) on river bottoms below 1,900 m and by fields; towns' cottonwoods, elms, ponderosa | sagebrush (garrigue, silver), blackbrush and cliffrose (shrubs, grey-olive), bunchgrass tufts |
| Subarctic (Iceland and the Faroes, beyond 70°, the tundra alone, E, beyond 45°) | low birch (half height), a few planted spruce (none beyond the spruce's range: Finnmark, Kola) | heath cushions |

## Trees where real trees stand

Within about 8 km of the camera a theatre's trees follow Meta and the World Resources Institute's canopy height map
(below). WorldCover's 10 m classes and the masks' density place the trees beyond that, as before. The map is what
WorldCover is not. It has the street trees and garden trees of a suburb that WorldCover calls built-up, a canyon's oaks
among the chaparral, and the clearings, lawns and airfields where nothing stands. It adds those trees and opens those
clearings. It never thins a wood the land cover stands: there the land cover's trees stand as before, on the map's
crowns and raised where it measures them taller.

```rust
land.set_canopy_grid(&mut renderer.context(), CanopyGrid { origin, texel_m: 8.0, slots: 8 })?; // naval_terrain::canopy
land.queue_canopy_page(index, Arc::new(CanopyPage::from_heights(&heights, 4, 2)));          // uploaded a few a frame
land.show_canopy(false);                                                                    // the land cover's trees alone (comparisons)
land.canopy_page_counts();                                                                  // (read, waiting for their upload)
```

- **Pages** (`naval_terrain::canopy`). 256² texels of 8 m (2 km a page), 8² of them about the camera in one RGBA8
  texture (16 MB), page (i, j) in slot (i mod 8, j mod 8), as the map features' pages are. A texel holds:
  - R: the tallest crown in it (m).
  - G: where in the texel the crown stands, in sixteenths each way.
  - B: the share of its ground under canopy 2 m or taller.
  - A: 255 where the map covers it.

  A crown is a 2 m cell 3 m or taller that none of its eight neighbours tops, nor a crown as tall found before it in
  row order (`CanopyPage::from_heights`). The map's heights are whole metres, so a closed canopy has flat tops many
  cells wide; each keeps a crown every other cell. Ties going to the first cell alone left one crown for a whole top,
  and half a forest's 20 m squares without a tree. A crown topping the cells within 2 (10 m) still left one 20 m square
  of a closed forest in five without one, a taller tree just over its edge topping all of its cells. Within 1, one in
  twenty-five is left (Miura's hills: 79% and 96% of the squares with a crown). A slot takes only its tallest crown, so
  the extra crowns add no trees. The trees' placement reads the texture at its group 0's binding 4, with each slot's
  page in the view (`TreeView::canopy`, `canopy_slots`).
- **Placement** (`tree_place.wgsl`, `canopy_crown`). A slot whose cell a page covers reads the texels under it.
  - **A wood** (the land cover's chance of a tree 0.5 or more, `CANOPY_WOOD`: WorldCover's tree cover and mangroves, a
    biome's closed canopy or plantation) keeps its own trees: the land cover's density, species, height and width, one
    tree a slot, a plantation's rows. The slot's tree stands on its tallest crown, and as tall as the crown where that
    stands taller than the biome's tree (never lower: Meta's map reads a tall tropical canopy low, Guadalcanal's lowland
    forest 12-20 m where it stands 25-35 m).
  - **A bare patch** stands no tree, in a wood or out of one: a slot with no crown in it and under 10% of the square of
    32 m about it under canopy 2 m or taller, with no crown in that square either (`canopy_bare`, `CANOPY_BARE`,
    `CANOPY_BARE_M`; an air window's slot of 35 m or more is its own square). A clearing in a forest, a lawn, a field,
    an airfield's paving and grass (the trees WorldCover's grass and built-up land stood on LAX's runways and taxiways
    are gone). The square is read only for a slot whose tree would otherwise stand.
  - **Outside a wood** the map's crowns stand trees of their own (the rest of this list); a slot with no crown and not
    bare keeps the land cover's chance and tree.
  - **Parts.** A slot of up to 22 m (a sea window's 20 m on High) is one part. A wider one (an air window's slots are
    35-70 m) splits into 2 × 2 or 3 × 3 parts of about 20 m (`CANOPY_PART_M`, `CANOPY_PARTS`). One tree to a 70 m slot
    could not close a forest of 15 m trees without standing three times as wide as it is tall. The parts beside the
    slot's own tree thin with distance as the slots do, each by its own rank, so past the trees' full range the slots'
    own trees remain, grown as before.
  - **The tree.** Each part's tallest crown stands a tree at the crown's height (±5%). A part with no crown stands
    none. The species is the biome's (`choose`) at the slot's tallest crown, for
    all its parts. A crown 5 m or taller where the land cover grows scrub or grass takes the biome's woodland tree: a
    live oak in a chaparral canyon. Scrub (the chaparral, the shrubs, the garrigue) keeps the land cover's density,
    since the map's crowns start at 3 m. A plantation's rows give way to the measured crowns. The level of detail, the
    clearings and the shadows are unchanged.
  - **Roads, railways and rivers.** A measured tree keeps off them where its crown was measured, within half a metre
    of their edge (`feature_clearance_by`, `MEASURED_MARGIN`), and the land cover's trees by 2 m as before. Read at the
    slot's foot with the 2 m, a street's margins dropped every crown of a slot the street crossed (a quarter of a
    town's on a grid of streets), and at the crown the 2 m dropped the street trees themselves, whose trunks stand a
    metre or so from the kerb (2026-10-29: Bay Ridge, Costa Mesa).
  - **Street trees** (`street_trees`, `STREET_*`). Meta's map misses most of a North American town's street trees: it
    puts some 8% of Brooklyn's ground under canopy (about 20% is) and few of Costa Mesa's parkway trees, so Bay Ridge
    from 90 m was roofs and pavement. In built-up land (WorldCover's built share 0.5 or more) that the map covers,
    within 24-50° N, 125-66° W (`street_lined`: the United States and the Canadian cities on its border; Europe's,
    Asia's and the islands' towns are as they were), each slot's cell is cut into places about 10 m on a side
    (`STREET_TREE_M`; at most 4 a side, `STREET_PARTS`, so an air window's 35-70 m slots plant fewer). A place whose
    middle lies within half a place of the line 2.5 m past a lined road's drawn edge (`STREET_SETBACK_M`: the road is
    drawn at its class's width and the kerb stands a metre or two past it, so the trunk is in the pavement's tree pits
    or the parkway) moves across to that line down the paved distance's gradient, and up to a quarter of a place along
    it. A lined road is a minor road to a primary by the features' tone (`STREET_TONE` 0.42-0.78): no service road,
    driveway, paved track, trunk road or motorway. Eight places in ten stand a tree (`STREET_PLANTED`), the slot's
    town tree (`choose`) at 0.55-0.95 of its height and 0.7-0.9 of its width (`STREET_HEIGHT`, `STREET_WIDTH`; a
    palm as tall as it grows). None where the map measures a crown within a 12 m square about it or has a quarter of
    the square under canopy (`STREET_CLEAR_M`: the street's tree is the map's), on another feature (1 m), in a clearing
    or on the period's country. Each place thins with distance by its own rank, as a measured part does. The trees
    stand by the drawn roads and know nothing of the buildings: a trunk 2.5 m past the drawn edge clears Bay Ridge's
    and Costa Mesa's fronts, but a frontage built to a road drawn narrower than it is may take a crown over its eaves.
    Tried and dropped: the slot's tree's own height and width (14-17 m oaks and ashes), whose crowns met over Bay
    Ridge's streets as one wood. A place is read at its middle and jittered only along the line: jittered across it
    too, two places could both fall in the band and stand side by side.
  - **Its width.** One crown stands for the part's crowns, as wide as the part's canopy: its radius
    `part · √(cover / π) · 1.25 · (1 + 0.4 · cover)`. The 1.25 (`CANOPY_DRAWN`): a crown is drawn about 0.8 of its
    model's widest reach across, seen from above. The `0.4 · cover` (`CANOPY_OVERLAP`): a closed canopy's crowns
    overlap, as they stand where they were measured and not on a grid. The width runs from 0.6 of the species' to 1.8
    for a broad crown (`CANOPY_WIDEST`). A narrow crown (a spruce, a cypress, a palm) and the sugi's and bamboo's
    stands, which fill a slot as they are, go to 1.3. No crown reaches further than the tree is tall (`CANOPY_ASPECT`).
    Tried and dropped: at `· 1.1` a closed rainforest seen from 300 m stood as separate crowns over the dark ground;
    up to 2.2 wide, Miura's evergreen forest stood as blobs 30 m across, and a garden's 6 m hedge under full cover as
    a 30 m disc.
  - **Dropped: measured trees in the woods too** (#1208). Every slot under the map stood its tree at the measured
    height, as wide as its part's canopy. Guadalcanal's closed rainforest, which the audit called the one place that
    already looked real, became scattered lower crowns over dark ground (Mount Austen's regrowth reads 5-12 m with 30%
    bare ground). The lower canopy drew dearer: +4.3 ms in the scene's opaque pass at 120 m over it, +1.3 ms from 300 m,
    +0.9 ms over Palos Verdes and Miura.
- **The source** (`naval-landscape` `canopy`). Each zoom-9 web-mercator tile is one BigTIFF named by its quadkey:
  65,536² pixels of the zoom-17 pixel (1.19 m at the equator), a byte a pixel, a strip a row, deflated with the
  horizontal predictor. A tile with no land is missing (a 404, cached as absent). A tile's strip tables (1 MB) are read
  once and kept on disk. The disk cache's unit is a 2,048² pixel chunk, the tallest of each 2 × 2 pixels kept (1,024²
  cells: 2.4 m at the equator, 2 m at 35°). A strip is a whole row of the tile, so a chunk row is read whole: 2,048
  strips of 2-9 KB, in eight ranged requests at once. All 32 of its chunks go to the disk deflated, under the surface
  cache's `canopy-chm-v1`, so a place read once loads from the disk.
- **Pages from chunks.** The worker makes the nearest missing page and the others of its row with it, since they share
  chunk rows. Each 2 m cell takes the tallest of the chunk cells under four points in it. The theatre's frame maps to
  the mercator's bilinearly between a page's corners, as the frame turns against it away from its origin. The frame
  loop hands made pages to the layer (`theatre::canopy::Feed`). A page is re-placed from the frame its texels upload.
- **Switched** with the surface streaming, on High only (the trees' setting; Medium has no trees and is unchanged).
  `NAVAL_CANOPY=off` turns it off alone. `theatre_state.canopy` gives the pages read, waiting and pending, and the
  worker's state.
- **Cost** (M5 Pro, the land audit harness at 1920 × 1080 on Ultra, the canopy ignored and read alternately in one run,
  8 rounds of 10 frames, the camera nudged every frame so the trees are placed every frame as in flight; medians; other
  lanes' work shared the GPU):

  | View | Placement | Opaque pass | Frame p10 |
  | --- | --- | --- | --- |
  | Guadalcanal, 120 m over the forest | +0.13 ms | +0.22 ms | 10.00 → 10.26 ms |
  | Guadalcanal, 300 m straight down | +0.02 ms | +0.02 ms | 8.10 → 8.20 ms |
  | Palos Verdes, 60 m | +0.22 ms | +0.01 ms | 10.89 → 11.16 ms |
  | Miura, 80 m | +0.16 ms | -0.06 ms | 9.61 → 9.70 ms |

  - Memory: the pages' texture, 16 MB on High. The host keeps up to 96 made pages (24 MB) and 40 chunks (40 MB).
  - Network and disk: a place's 64 pages read 85-250 MB the first time (Kamakura 85 MB, Los Angeles' four views 160 MB,
    Hakone 190 MB, Guadalcanal 235 MB) and keep 40-110 MB of chunks on disk, which nothing prunes (as the surface
    cache). A chunk row reads in 1.5-1.8 s at about 6 MB/s and keeps in 0.3-0.5 s; from the disk a batch of pages
    takes 0.1-1 s.
- **Limits.**
  - A wood's trees are no lower than the biome's, so a wood the map measures low (Mount Austen's regrowth, Miura's
    12 m against the species' 17 m) stands as tall as before. Only its clearings and its taller crowns come from the
    map.
  - Outside a wood, low dense growth (3-8 m) stands as separate crowns: a tree no wider than it is tall can't close it.
  - The forest-shade bake (`tree_shade.wgsl`) still reads the land cover's density, so the floor of a clearing the map
    opens in a wood keeps the forest's sky occlusion (darker than open ground). On High and Ultra, where the map is
    read, the trees' own sun shadows replace the bake's within the land maps' 4 km. Making the bake read the map would
    take the pages and their slot table into its pass, and a re-bake as they stream.
  - Beyond the pages (8 km) and without them, trees stand on airfields' grass and paving where the land cover places
    them.

**Source.** Meta and World Resources Institute (WRI), 2024, *High Resolution Canopy Height Maps* (global, 1 m, from
Maxar imagery of 2009-2020, mostly 2018-2020; `s3://dataforgood-fb-data/forests/v1/alsgedi_global_v6_float/chm/`,
`theatre::canopy::ATTRIBUTION`). CC BY 4.0: free for a commercial game with the credit.

## Ground cover

On High, a second entry of the placement pass (`place_cover`) stands shrubs, garrigue cushions and tufts of grass on a
grid of 3.6 m cells within 360 m of the camera (no grid while the camera is higher than that above the land's top),
into the same instances and indirect draw: shrubland grows shrubs (chaparral, maquis) at 55% of the cells (California's
chaparral at 85%, a darker olive), garrigue on
dry stony ground in the Mediterranean and arid biomes, heath cushions in the north, a few shrubs under woods and in
grassland; tufts on grassland within 140 m, dense (45%) where the summer is dry or in the tropics, few in a wet
country's pasture; nothing on fields, towns, bare ground, snow, beaches, steep slopes, water, or the map features' roads,
railways and rivers (`feature_clearance`, as the trees keep off them). A land with no place takes them
from its style. Every cell keeps its plant within 126 m, (126 m / d)² of them beyond, each shrinking away toward the
reach as the trees do. They are the same impostors (shrub, garrigue, tuft models), drawn and lit as the trees.

## Near trees

Close to, an impostor is a flat picture: its texels show and it cannot move. Within about 250 m the trees are meshes
made from the same models, the impostors finer beyond them, and every tree sways in the wind. High and Ultra only (the
trees' setting); Medium draws no trees.

- *The meshes* (`src/trees/tree_meshes.rs`, once a process, `tree_meshes()`). Made generically from each species' grown
  model ("Grown trees"), so a new species needs nothing of its own: each limb a tube, each spray a card carrying its
  branchlet; a run of clumps strung along a line (a palm's frond, a blade of grass: `chains`) one ribbon down its
  length. Sprays buried in the crown (the model's ambient occlusion under 0.18) are left out, the most open first in
  the mesh and the bark last. The vertices stay in the model's space (foot 0, top 1), so a placed tree's height, width
  and yaw place them as they place its impostor. Two levels: the full one, and the reduced one of one in ten sprays at
  random (`REDUCED_SHARE`) in cards three sprays a side that carry their sprite three times over each way
  (`REDUCED_TILES`, so the leaves keep their size; a low shrub's cards grow less), without the finest limbs, the bark
  with fewer sides and no flare. A card is lit where it is drawn, a reduced card's corner standing out of its clump at
  the clump's surface (until 2026-10-26 a reduced card took the light of the spray it grew from: a buried spray's dark
  reached the crown's outside, and the reduced trees drew 0.86–0.93 of their impostors' brightness). 429,797 vertices and 762,354 indices for the 32 species' two levels, 16.8 MB (until
  2026-10-26 each capsule a
  cylinder and each leaf clump two crossed cards of one sprite a kind: 77,039 vertices, 141,342 indices, 3.0 MB).
- *Leaves.* Twelve kinds of branchlet painted on the CPU ("Grown trees"); a new conifer or palm reads better named
  under its kind (`foliage()`), the geometry needs nothing.
- *Which trees* (`shaders/tree_near_bin.wgsl`, `src/trees/near_trees.rs`). After the placement, one thread a placed tree: a
  tree in view within its reach is copied into its species' list for each level whose share of its pixels is above 0,
  and counted into that list's indirect draw (62 lists of 4,096 trees, `NEAR_ROOM`). A tree no list has room for draws
  whole as its impostor (its width negated). The reach is 250 m for the tallest tree and 12 m a metre of height for a
  smaller one (`NEAR_REACH_PER_M`, so a shrub gives way where its impostor is as fine on screen), up to 1.6 times
  further through a narrower lens (`near_reach`). The full detail holds the first 20% of the reach (40% until
  2026-10-26: a grown tree is 4,000 to 22,000 triangles at full detail, and a rainforest within 100 m stood hundreds).
- *The crossfade* (`shaders/tree_wind.wgsl`'s `near_shares` and `pixel_dither`). Over the last 20% of each reach a
  pixel's place in an interleaved gradient noise, moved each frame for temporal anti-aliasing, picks which picture draws
  it: mesh or impostor, full or reduced. Each draws its share with no seam: the light is shared (`shaders/foliage.wgsl`,
  moved there from `trees.wgsl`) and so is the wind.
- *The draw* (`shaders/near_trees.wgsl`). The leaf's alpha is tested at 0.5 (0.4 at the reduced level, whose sprays
  are larger; until 2026-10-26 0.36, and the most open clumps dropped up to a fifth of their leaves more). The full
  detail draws first, then the reduced, then the impostors, each twice ("The depth first" below), then the grass
  ("Grass").
- *The wind* (`tree_wind.wgsl`, `TerrainLayer::set_wind`: `SeaScene`'s true wind, the one the ensigns blow in; in
  a theatre its weather's `windMps` from `windFromDeg`). Every tree's top leans downwind by
  min(0.0004 s², 0.06) of its height (0.2 m for a 20 m tree in 5 m/s, 1.2 m in 12 m/s), the trunk bent by the square of
  the height. Gusts run downwind across the land at the wind's speed, and each tree sways at its own phase at
  0.32 √(20 / height) Hz. The impostor's card, its shadow caster and the mesh bend alike. A mesh's leaves also sway on
  their branches (min(0.006 s, 0.07) of the clump's distance from the axis) and flutter, each clump at its own phase.
  Below 0.4 m/s nothing moves.
- *Shadows.* The trees cast as before, from their impostors' caster cards (now bent by the wind). The meshes cast
  nothing of their own; the caster card's outline is the same tree.

**Drawn before the ground.** The cost of near trees on a tiled GPU is not their vertices (the vertex stage alone,
measured, 0.1-0.3 ms) but their discarded fragments: an alpha-tested draw stops Apple's hidden-surface removal from
skipping what it hides, so the ground drawn first under a canopy was shaded in full and then covered. The land layer
now draws its trees (meshes, then impostors) before its patches (`TerrainLayer::draw_trees`), and the ground under them
fails the depth test. With two cards a clump and the buried clumps left out, it took the dearest views (40-55 m
over a forest) from +2-2.8 ms to under 1 ms, the draw order the most of that. Tried and dropped: eight-sided cards
fitted to the sprite (more triangles, no gain), and the reduced level as the most open fifth of the clumps (conifers
went thin).

**The depth first** (2026-10-26). Every alpha-tested card that passed the depth test was
lit as it came, before the GPU knew what lay in front of it, and with the grown trees' light (the crown's light, the
soft shadow filter, the sky) that came to most of their cost: lit with a constant, the trees in Kawah Lagoon's views cost
3-3.75 ms less. Now the meshes (both levels) and the impostors draw their depth alone first, alpha-tested
(`fs_near_depth`, `fs_tree_depth`: the leaf sprite or the frames' coverage, the crossfade's dither, no colour written),
then their light where that depth stands (`fs_near`, `fs_tree`, depth test `Equal`, nothing written to the depth, no
pixel the prepass stood discarded), so each pixel of the trees is lit once (`near_trees::tree_pipelines`; the clip
position is `@invariant`, so both draws meet the same depth). The vertices are drawn twice, which made the same prepass
twice the cost in 2026-10 when the light was cheap and the cards few; now it halves the trees' cost near the ground. The
images are those of before but for the crossfade's dither at about one pixel in a thousand (its rounding in the shorter
shader; the dither moves every frame).

**The light's discard** (2026-10-27). Drawn under `Equal` by shaders that could not discard, the light took unsteady
derivatives on Apple's GPUs: in about one pixel of the trees' fifty they changed from frame to frame while the pixel's
own inputs stayed bit for bit the same, which pixels with the GPU's timing (in the first frames, and with other work on
the GPU; a 40 ms pause between frames hid it). The leaf sprite's level and the impostor frames' inset, blend and mip
level moved with them, by up to 15 levels, and the terrain tests' exact comparisons failed in about one full run in
four. The light writing the depth as well did not steady them; the depth test `GreaterEqual`, or any discard in the
shader, did. So `fs_near` and `fs_tree` keep a discard that no pixel the prepass stood takes (a tree its impostor draws
whole, a card its mesh does). A probe drawing one view ten frames running changed between frames in 8 runs of 10 before
and in none of 10 after. The crate's whole test run (`bun run rust:test -- --native -p naval-terrain`) failed 2 times in
12 before (`cover_patches_draw_as_the_classes_they_write`, 4 and 8 bytes by up to 11 levels), and none of the trees'
tests failed in 36 after. The tropical still at `at:900:1.7:60` (1920 × 1080, in the wood at 1.7 m) differs from before
in 2.0% of its pixels, 97.5% of those by one level and nearly all darker, at every brightness (the eye's exposure
following), the rest by up to 15; the build before differed from its own second run in 2,269 pixels, the fixed one in 4.
`grove`: 319 pixels, by up to 2 levels. The trees' cost (`--tree-cost`, 32 pairs, the two builds alternately four times
each under other agents' load of 7-9, medians): `at:900:1.7:60` -0.14 ms before and -0.09 ms after, `grove` -1.69 ms
both, inside the runs' spread of about ±0.5 ms.

**Finer impostors.** Beyond the meshes, frames of 96² rather than 64² put about a texel on a pixel at the meshes' reach
at 1080 lines. The atlas is 198 MB rather than 86 MB ("Impostors", Memory), and its bake casts 2.25 times the rays. 128²
would take four times the 64² memory.

**Costs** (Apple M5 Pro, 1920 × 1080, Ultra, the `test-fast` client in a temporary harness on the three user
theatres' land, 2026-10-08). The near trees on against off in the same scene (`naval_client::land::set_near_trees_shown`),
10 alternating batches of 20 frames, the GPU's own timestamps, each side's tenth percentile. The eye's height over the
ground is in the view's name.

| View | GPU, near trees on − off | Trees as meshes (full, reduced) | Mesh triangles |
| --- | --- | --- | --- |
| Palos Verdes, `canopy-palos-verdes` (60 m) | +0.15 ms | 2, 47 | 20k |
| Palos Verdes, `canopy-40` | +0.44 ms | 9, 57 | 31k |
| Miura, `canopy-miura` (80 m) | +0.38 ms | 0, 70 | 50k |
| Miura, `canopy-miura-40` | +0.95 ms | 18, 106 | 94k |
| Guadalcanal, `rainforest-120` | +0.35 ms | 0, 79 | 39k |
| Guadalcanal, `rainforest-55` | +0.67 ms | 24, 154 | 104k |

A ship's views, 250 m off a coast or more, draw no meshes; only the finer impostors change there. Memory: the meshes
3.0 MB, the lists 12.2 MB (31 species × 2 levels × 4,096 trees × 48 bytes), the atlas 112 MB more.

**Left.** The atlas uncompressed (BC7 would quarter it); the meshes cast no shadow of their own; at middle distance a
conifer's reduced mesh is a little more see-through than its impostor; up close, a limb's capsules show as segments; in
12 m/s the sway is lively.

## Grown trees

Each species' design (`src/trees/species.rs`, the capsules and clumps above) is grown into a tree before anything draws
it, and the impostors are baked from the same meshes, so near and far are one tree: limbs as tubes, each leaf clump a
few sprays.

- *Growth* (`src/trees/tree_growth.rs`, `grow`). The design's capsules become limbs (tubes), and each leaf clump a few
  sprays: a card carrying a branchlet with its leaves at their real size (a broadleaf's spray about 0.6 m long, a pine's
  tuft 0.45 m, a bare oak's twigs 1.5 m), its stem on the nearest limb, fanned out over the clump's surface and laid as
  the species carries them (`Habit`: how big a spray, how many a clump, how much they rise or hang, how thick its
  twigs). A limb that carries sprays branches toward them, each twig thicker by the clumps it serves. No spray reaches
  under the ground.
- *Leaves* (`tree_meshes::leaf_sprites`, `Foliage`): twelve kinds painted on the CPU, each a branchlet rather than a
  haze: broad, small, lobed (oak, sycamore), lanceolate (olive, eucalyptus, bamboo), tropical (rainforest, laurel),
  needles, pine tufts, scale leaves (the cypresses), a fan palm's fan, a frond, bare twigs (seven generations of forks)
  and grass. Three layers of 2 × 2 slots of 384² at the end of the atlas, their levels holding the coverage (the
  bare twigs' averaged plainly, so a far twig card carries the mass of its twigs: see the crown's light below). A
  sprite's red is a leaf's brightness, or under `TWIG_RED` (0.09) a twig, shaded as bark.
- *Bark* (`Bark`, `shaders/tree_surface.wgsl`), its colours measured (2026-10-26): most bark a linear albedo of
  0.06-0.18 (the oak's 0.07 was 0.04, the spruce's 0.06 was 0.04), pale by nature the birch 0.47, the blue gum 0.36, the
  rainforest's and the emergents' smooth grey 0.10 (was 0.06; the measured 0.18-0.3 stood pale in a wood's shade), the
  coconut palm's 0.17 (measured 0.18-0.25; 0.08 had only hidden the pale fence a beach's palms drew from the sea while
  every trunk took its light from inside, below); the
  sycamore's mottle 0.28 (was 0.52). A tree's tint takes bark by half its brightness. Six
  patterns, furrowed (oak, ash), smooth (the rainforest's, eucalyptus, sycamore, laurel, the shrubs), a birch's white
  with dark bands, a pine's plates, a redwood's or a cypress' fibres, a palm's rings, each with its relief in the
  normal. A limb's ridges go round it a whole number of times, so the pattern closes; a trunk flares into the ground,
  the rainforest's emergents and the mangroves into buttresses. A limb's tube is wound with its faces' fronts outward
  and its flare's normal leans up (2026-10-26): the near trees and the bake turn a back face's normal round, and every
  tube had been wound the other way, so every trunk and limb took its light from inside, lit on its side away from the
  sun. Seen against the sun a trunk's foot glowed with the low sun under its crown (`at:400:8:120`, in a white sunlit
  surface's light: 0.069, the sun 0.063 at a facing of 0.67; now 0.004), and a beach's palms seen from the sea stood a
  pale fence (a trunk 0.044, the sun 0.041 on normals turned 0.79 toward it, 27 % of the sand's; now 0.014: the sky
  0.010, the ground 0.003, the sun 0.003, 8 % of the sand's, where a shaded trunk of its albedo should draw 6-11 %).
- *One surface* (`tree_surface.wgsl`'s `spray_surface`, `bark_surface`): the near trees' fragments and the impostor bake
  shade a spray and a limb alike, so the crossfade shows no change of tree's form or colour. Its light agrees in front
  light (the impostors' packing, 2026-10-26; an impostor-only build against the normal one, tree pixels by band in Kawah
  Lagoon, before → after): `grove` 0.93 → 1.00, `at:400:8:120` 0.88 → 0.98, `at:900:1.7:60` 0.86 → 0.95 (1.01, 0.98 and
  0.96 with the bark's faces turned out). The step was the impostors' packing ("Impostors") and the reduced level's
  light ("Near trees"); the saturating sun of a blended sky was a small part (0.02). Looking into the sun from 30 m the
  meshes still draw 0.81 of the impostors (0.76 before), 0.70–0.76 in the nearest rows (0.59–0.65), open: with the sky
  held fixed the impostors' wrapped sun and sky still draw 5–20 % over the meshes' (their normals' codes read unfiltered
  took 0.01 of it), and up close a magnified impostor smooths away the dark gaps between sprays. A leaf's albedo is its
  card's colour × its brightness on the sprite (0.4–1.6) × `LEAF_LEVEL` 1.06. With it a wood seen from inside draws
  1.1-1.16 of the old trees' green (the mean light of its green pixels) and from afar 0.88-0.9. The impostors' levels
  lose none of it (finer levels, BC7 off, one frame a pixel each move it under 0.005): a far wood's brightest leaves
  equal the old trees' and its darkest quarter draws a fifth darker, in another crown's shadow with no floor under it
  (the old light had floors), and the green summed over the frame is 0.95-0.98 of theirs from afar, 1.25 inside. A
  leaf's normal is its clump's, its own tilt on the card a detail (0.35 up close, 0.15 in the bake, so a far crown keeps
  its clumps' light and shade); a card seen from behind mirrors only that tilt, never the clump's normal (until
  2026-10-26 a clump's far side was turned to face the eye, and far crowns went flat).
- *The light inside a crown* (`src/trees/crown.rs`, 2026-10-26; the owner: the far trees "look worse", the trunks "weirdly
  bright"). Each species' leaves become a density over a grid about its model (32 nodes on its longest side): each clump
  a soft ball of its sprays' leaf area (cards' area × 0.4 sprite cover × 0.5 facing: the light takes the leaves the eye
  sees through the crown's gaps; a buried clump half; a bare crown's twigs 0.03 cover, about a tenth of the light a
  metre, as a winter crown passes most of it), the ribbons' clumps by their openness. Each of 16 sky directions
  (cosine-weighted, 10° up to the zenith) is swept across the grid a slice at a time, and every vertex takes the share
  of the sky it sees through the crown (`color.a`) and that sky's slope (`sky_slope`, 4 bytes: the least-squares line
  through the 16 transmittances, so a trunk under a crown is open low and shut high, which a bent cone about one
  direction cannot hold). `light_foliage` takes the sky and the fill, the ground's bounce with them, times that sky with
  no floor (a leaf's to the power 0.84, for the light the leaves scatter on; a trunk's less what its wood's neighbours
  close, 0.7 of it at a full wood's foot, none at the canopy's top); a crown's mass (its leaves, a bare crown's twigs)
  the sun wrapped round its clump times `smoothstep(0, 0.45, sky)`; bark's sun from its slope (`bark_sees_sun`; an
  impostor's by its sky squared), another crown's shadow on a trunk or a limb whole but the 0.07 the leaves let through,
  on the meshes and the impostors alike (leaves and a bare crown's twigs keep a third of the sun under one; a trunk
  took that third at every height above 5 m and stood pale). The oak: its top 1.00, a metre in 0.63, three metres in
  0.08, its trunk at 1.5 m 0.49 (the sky under the skirt), a 12° sun on that trunk 1.00, a 60° one 0.01; the laurel's
  dome a metre in 0.48, the rainforest 0.73 (`crown::tests`). At twice the cards' share (until the second pass) three
  quarters of the rainforest's sprays saw under a tenth of the sky, and the inner leaves seen through the gaps drew
  black. The grids cost 208 ms for the 32 species one after another (dev-client, Apple M5 Pro), the meshes with them 37
  ms built on eight threads.
- *Light through and among the leaves* (2026-10-26, the second pass: the leaves had come out a quarter darker than
  the old trees', the inside of a canopy black). A leaf passes on about as much green as it sends back, less red, far
  less blue (`LEAF_TRANSMIT`, the albedo × 1.1, 1.25, 0.55). Leaves on the far side of their clump from the sun take
  what a leaf or two of it let through (0.22) and pass it on to an eye on that side: a crown glows yellow-green against
  the sun and from under it. Toward an eye looking toward the sun the forward part of that light lights the open rim
  (0.45 × the view's cosine to the fourth; was 0.55 and a 0.06 floor). The light the crown's leaves scatter fills it
  (`CROWN_SCATTER`): its sunlit outer leaves pass on inward about a tenth of the green on them, so a leaf inside takes
  0.7 × (1 − sky) × w / (1 − w) for a typical leaf's reflectance and transmittance w (0.11, 0.2, 0.05: every order of
  the light the leaves pass on, a quarter more green than the first alone) of half the sun and all the sky on the crown
  (the sky lights the whole crown, the sun one side), whatever its own brightness; the sky and the fill are taken about
  the normal bent up for leaves and twigs only (bark is a surface: bent, a trunk in the open took the sky of a face
  turned up to it, and a beach's palms stood pale from the sea); deep in the crown its limbs
  take it too (× 1 − smoothstep(0, 0.6, sky), so a trunk under its skirt keeps its shade). The heart is a dark green,
  not black, in the land's shadow too.
- *Bare crowns* (the second pass: Dover's bare and autumn woods had become dark grey skeletons from afar). A bare
  crown's twig cards are its mass (`Foliage::twig`; the bake packs them as leaves, and the impostors know the bare
  species by `SPECIES_BARE_OAK` and `SPECIES_BARE_BIRCH`): wrapped round the clump as leaves are, the sun reaching
  through the thin mass to its far side (0.3), and the twigs' lit sides seen less as the eye looks toward the sun (a
  round twig's phase, about ((1 + cos) / 2)^1.3, softened from a sphere's 1.6 for the lit land between the twigs); no
  light through them, none scattered, no neighbours' shade. Their colours are measured fine twigs' (a linear albedo of
  0.10-0.15, the oak's a grey brown, the birch's purpler). The twig sprite's levels are averaged plainly and the cards
  cut at 0.1 (`TWIG_CUT`, the leaves' 0.5): up close their twigs, from a few tens of metres the mass of them as a soft
  clump, so the bake and the reduced meshes draw a far bare crown as a soft dome with its trunk and main limbs under
  it, not as its limbs with sparse twigs.
- *The impostors* are drawn from the full meshes ("Impostors", the bake): 32 species, 64 frames each, at the layer's
  first frame; 211 MB baked, 53 MB in BC7, its copy made on its thread in 1.2-1.7 s (1.2-1.4 s before). Their levels
  keep each frame's coverage at the alpha test's edge (`tree_mips.wgsl`, Castaño's alpha mipmaps, 2026-10-26: one
  workgroup a frame bisects a scale on its coverage until its share of texels at 0.45 is the base level's, texels of
  one coverage spread ±2% so they cross the edge apart). Off the base level's share on average over every frame of
  every species (`the_impostor_levels_keep_their_coverage`), plain averages against kept: level 1 0.014 against
  0.0002, level 2 0.015 against 0.0005, level 3 0.025 against 0.0018, level 4 0.042 against 0.0074, level 5 (3 × 3
  texels a frame) 0.100 against 0.029; plain averages thinned every level (by 0.014 to 0.095), the kept ones by
  nothing.
- *Levels of detail* ("Near trees"): full detail within a fifth of a tree's reach, then one spray in ten in cards three
  sprays a side carrying their sprite three times over, so a dense wood's hundreds of reduced trees stay cheap and
  their leaves keep their size.

## Grass

Grass blades on the ground at close range, where under a low eye the land's grass would be its flat texture.

- *Placement* (`shaders/grass_place.wgsl`, `src/trees/grass.rs`; a compute pass each frame the view is selected): a grid fixed
  to the chart about the eye, 0.14 m cells. Within 8 m every cell stands a clump; beyond, fewer by (8 m / d)², each
  cell's own rank against its share, so a clump grows up out of the ground as the camera nears and none pops; none past
  55 m, the last third sinking into the ground. A clump stands where the land is grass: the window maps' grassland,
  a wetland's reeds, the ground between shrubs, a field's grass and a town's verges; the period's pastures, hay and
  rough grazing, half an orchard's or a grove's floor, none of a ploughed field; a style's own ground (0.45-0.85).
  Little under a wood's canopy (the open sky's green), none on snow, a beach, a road, railway or river, a building's
  clearing, or ground steeper than 0.45-0.75; and only on as much of the ground as the land's own two heaviest
  materials make grass (`grass_of`: grass and dry grass whole, mud 0.6 for a marsh's reeds, scrub half; leaf litter,
  soil, rock, sand and paving none). So the rainforest's floor carries none: the few tufts its canopy left took the
  litter's colour, nearly black, and stood black against a floor lit mostly by the sky it reflects at a low eye. It
  takes the ground's own colour there (`terrain_surface` at the footprint the land is drawn with) and stands as tall as
  that grass grows: 14-34 cm, a grazed pasture 0.55 of it, hay 1.3, reeds 1.7, taller on wet low ground and shorter on
  knolls. Written as instances and counted into an indirect draw, at most 65,536 clumps (2 MB).
- *Blades* (`shaders/grass.wgsl`): 16 a clump, each a triangle strip of four pairs and a tip (12 vertices, the strip
  running on to the next blade through triangles with no area): scattered over the clump's patch, tapering, curving
  over toward its own facing, bowed downwind in gusts and fluttering. A thinned clump far off stands for the clumps
  about it, so its blades widen with its patch (up to 8 times) and the field stays as thick to the eye.
- *Light* (`foliage.wgsl`'s `light_grass`): the ground's, its sun past the land's shadow and the maps' (read through
  the land maps' coarser filter, `coarse_sun_shadow_maps`, since 2026-10-26: the soft filter took 0.6 of the pasture's
  0.9 ms, and the blades break a shadow's edge up finer than its kernel; the crowns' leaves read them so too), the forest
  floor's shade and the land's shading, about a normal bent three quarters of the way to the land's, so a field seen
  from afar is its ground; darker down among the blades (the sun 0.35-1, the sky 0.4-1), a waxy sheen, and the sun
  through a blade (45% of what its other face takes, and more toward an eye that looks toward the sun). Without the
  light through it, the blades the eye saw turned from the sun stood black on the lit ground. Its colour is the
  ground's, 1.35 times as saturated (the land's colour carries the soil between the blades), varied a fifth blade to
  blade, a dry tip paler.
- High and Ultra, with the trees: drawn after the impostors and before the land's patches, opaque.

## Shadows

The trees cast into the sun's maps where the render core's maps reach, and a forest's shade stands in everywhere else.

**Into the sun's maps** (`shaders/tree_caster.wgsl`, `src/trees/tree_shadows.rs`). The placed trees draw into the near and
wide maps (native/docs/render-core/shadows.md, "Sun shadows") from the scene's own instance buffer and indirect arguments: no read-back,
one more indirect draw a map. Each tree is one card standing across the light at the tree's middle, as wide as the tree
and as long as the tree looks from the sun (its height foreshortened by the elevation, plus its crown's depth seen from
above), alpha-tested against the atlas at the cards' own threshold: a low sun throws the whole outline from the foot,
trunk and lobed crown (no gap under the trunk), a high one a round crown under the tree. A caster that discards needs a
fragment stage: `CasterPipeline::fragment_entry` (naval-render, depth only, no targets). Since 2026-10-05 the card is
the tree's impostor facing the light ("Impostors"): the outline the sun sees from its direction, the trunk and crown
from the side at a low sun, the crown's disc beneath it at a high one.

- *Which maps.* The near and the wide (and the aboard map, native/docs/render-core/shadows.md "Aboard", while the camera stands on a
  hull), and only while the map's target lies within the trees' full-density distance of
  the camera (4.5 km, apparent by the lens as the trees' own level of detail; `TREE_SHADOW_REACH_M`): past it the trees
  thin out and a 10 m crown's shadow is about 2 px at 1600 × 900. The view maps (squares about distant hulls the wide
  map misses) take none, for the same reason. The near map alone cost 0.12-0.19 ms against 0.17-0.35 ms with the wide
  too (low sun, six views), but the wide map is what holds a coast within 380 m of the ship. Since 2026-10-08 the
  render core's land maps (native/docs/render-core/shadows.md, "Land maps": three squares about the camera's view of the land out to 4 km,
  on the High and Ultra presets) take them too, from the same instances (no placement of their own: the trees already
  stand for the view out to 4.5 km), so from the air every tree in view casts (`TerrainLayer::trees_cast_land`; the
  terrain uniform's `shore.w` tells `forest.wgsl`). While they do, the placement takes the view's sides toward the sun
  moved out by a tall tree's shadow (`tree_shadow_margins`: from its middle to its top's shadow across each side, in
  25 m steps, at most 400 m), so a tree just beyond the frame stands, on the drawn patches, and throws its shadow in.
- *Out of view.* A tree behind the camera throws its shadow into the view (a 30 m tree's reaches 140 m at 12°). The
  placement stands every tree whose middle lies in a map the trees cast into, in view or not (`in_maps`), and the layer
  adds the level-0 patches with land inside the maps' footprint that no drawn patch covers (`shadow_patches`, at most
  256, after the drawn ones in the patch buffer: the CPU bounds each light box between the sea and the trees' tops). Their
  trees draw in the scene too, clipped. The render core now hands every layer's `prepare` the frame's fitted maps
  (`PrepareContext::shadow_maps`), and the trees are placed again when the maps move.
- *Receiving.* The land takes them through `sun_shadow` as it takes the ships'. A card takes the other trees' shadows
  read in front of its own caster card (a card leans toward the camera, and its top may pass behind its caster: read
  there, a crown shaded itself along a straight line), at 70% (`CROWN_SHADOW`: leaves let some light through, and the
  light inside a canopy scatters into a shadow), so crowns shade the lower parts of those behind them.

**The forest's shade** (`shaders/forest.wgsl`, `shaders/tree_shade.wgsl`). Beyond the maps the trees' shadow stands in
from a bake: the trees standing per tree cell and their mean height, from the masks the placement reads (`tree_cover`,
averaged over 2 × 2 points of each attribute texel), into the open sky texture's spare green and blue (written into a
storage texture kept for the layer and copied over it: the land's pipeline already binds the 16 sampled textures a
stage may have). Baked whole in the first frame, after the open sky, and when the surface maps, the features' grid or
the near tiles' grid change; after that only over the rectangles whose inputs changed (`forest_shade.rs`, since
2026-10-05): a map feature page or a near tile coming into its slot, or the one it put out, each grown by how far its
shading reaches (a level-1 page or a tile feathers into its neighbours: grown by a whole page or tile; a level-0 page by
two texels for the filter). Rectangles asked for within 8 frames of the last bake wait and go together, up to 8 a bake
(merged where they touch), one dispatch layer each; a whole bake waits for nothing. A theatre window's whole bake is 8-15 ms of GPU on an M5 Pro, and
it ran again whole for every page that arrived (12 frames running at 8×); two level-0 pages' reach is under 2% of it.
The GPU test `a_pages_forest_shade_is_baked_over_its_reach_as_a_whole_bake_would` (`background.rs`) holds the open sky
after level-0 pages and then a level-1 page arrive late equal, byte for byte, to a whole bake with them there from the
start. From it:

- the forest floor's sun: the share in the crowns' shadow at the sun's elevation (each crown shades a patch its width
  across and as long as its crown's shadow, the patches overlapping at random): half the floor under a closed canopy at
  noon, all of it at a low sun;
- the floor's sky, hidden by up to 35% under the crowns, and seen from beneath them (the eye under 1-2.5 times the
  trees' height over that ground, `beneath_crowns`) by up to 70% under a closed canopy, the closed canopy's share coming
  in with the square of the cover (a hedgerow's or a wood's edge, whose cells reach out over the field beside them, keep
  35%); what reaches a floor under a closed canopy is mostly passed and scattered by the leaves, so its light and its
  reflection are the leaves' (`under_leaves`): their strength kept, their colour the daylight on the canopy (the sun's
  and the sky's on the level) passed through leaves (0.9, 1, 0.7 of it), all of it beneath a closed canopy, the closed
  canopy's share of it beneath a thinner one, none from above, where the ground stands in for the crowns' sunlit tops
  (multiplying the sky's own colour by a leaf's left a floor's light a deep blue, its red half its green and its blue
  one and a half: ground.md, "The ground close up"), the grass lit as the ground under it is;
- far off, where the cards thin out and the ground paints the crowns left out, those crowns' tops a little self-shaded at
  a low sun (to 0.7 of the sun at the horizon under full cover), mixed by the share of the trees still standing;
- a card's lower crown in its neighbours' shadow: the crowns standing on the strip the ray to the sun sweeps under the
  canopy's top, their outlines 0.32 opaque (`CROWN_GAPS`).

Where a map carries the trees' shadows (`tree_map_share`: `sun_shadow`'s weights of the near, aboard and wide maps, and the
land maps' share beyond them, `shadow_land_share`, while the trees cast into those) the sun's stand-ins step aside, so no ground and no crown is darkened twice; the floor's sky occlusion stays. Matched against the
maps' shadows on the `canopy` view of Iron Bottom Sound (mean luminance of the lower middle, the maps' shadows / the
stand-in): 45.0 / 46.0 at a 12° sun, 86.2 / 84.3 at 35°, 95.0 / 95.2 at 60°; with the crowns unshaded, the floor alone
63.9 / 63.7; the `grove` view's lower half on two maps at the three suns within 5%. The seam where a map ends is faint:
the maps' shadows carry the contrast, the stand-in the mean.

**Settings.** Medium (no trees) compiles the land without any of it (`TREES` def) and draws as before. On High and Ultra
the trees cast while the renderer's maps are on (Graphics → Shadows low 1024, medium 2048 as the High preset sets,
high 4096); with shadows off nothing casts and the forest's shade stands in everywhere. Hosts do nothing: the battle and
theatre scenes already set the renderer's sun for the ships.

**Look.** A forest at a low sun has crown-shaped shadows across its floor and on the crowns behind, and far off reads as
a closed canopy in shade rather than the painted canopy's lit mean: Kawah Lagoon's land at a 60° sun is 19% darker from
6 km (`ship6`), 32% from 3 km (`coast3`), 29% from over it (`inland`).

## Open problems

- The trees' shadows reach only as far as the render core's maps (about 380 m around the ship, and the near map about
  what the camera looks at): a forested coast farther off has the stand-in's mean, not crown shadows; a wider map for
  land (or cascades fitted to the view) would carry them further. Shadows on the land at a low sun step in the near
  map's texels (0.6 m on the ground at 10°), as the ships' do. The stand-in is one calibration on rainforest; the chalk
  country's hedgerows and the snow style's birch were looked at only through the bake's numbers. The sea reads the
  same near and wide maps for the ships' shadows on it (Graphics → Water shadows), so trees at the shore shade the
  water by them too, but past the maps the stand-in is the land's alone.
- Straight down from an aircraft a tree is still a card tilted halfway toward the camera (the web's), not a crown seen
  from above. The far trees are single trees grown 1.35×, dots of 3-5 px at 10 km, not clumps on one card.
- Through 12× binoculars the cards cost 0.9 ms (1.85 ms at 1440p), the fragments of magnified crowns; nothing budgets
  the count but the 2^18-tree buffer.
- The scene pass has no MSAA or FXAA, so tree edges are an alpha test and their dissolve a dither.

History (measurements, stills and the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
