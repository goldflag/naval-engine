# Shadows

## Sun shadows

A port of the web's sun shadows: the fitting of `src/game/FocusShadowNode.ts`, the depth-only
caster passes of `src/game/ShadowCasterPass.ts` (both historical), and three r185's shadow sampling. Since 2026-10-03 the
fitting holds still under a moving hull and the filter is the client's own (see "Stability" below). Code:
`naval-render/src/shadows.rs` and `shaders/naval/shadow.wgsl`.

**Five maps about the ships and three about the camera over land**, fitted every frame from the camera and a `Sun`:

| Map | Covers | Size | Drawn | Receiver offsets |
| --- | --- | --- | --- | --- |
| near | the followed hull alone (`Sun::subject`: her sphere ÷ 0.9 + 2 m, so a 250 m battleship is 142 m) when she fills less than the frame and lies in it; else a square of half-width `reach` around where the camera looks at the subject's distance (`nearShadowFocus`: the visible frame there × 1.1, quarter-octave steps held against small changes, 12 m to the wide half-width). Its texel grid is laid from the subject's origin. It stands 800 m back along the light and reaches 1200 m past its square behind its centre, or farther both ways under a low sun (below) | setting, at most 2048 | every frame | normal 1.5 texels, depth 2 texels |
| aboard | with the camera's eye inside the subject's box (`Sun::hull`, `HullBox`: her fixed structure's bounds, at least her length and beam, from 2 m under her waterline to her top; aboard her on a walk or her bridge view; since 2026-10-13, "Aboard" below), what the view sees of her from where the near map begins to hand her over (`aboard_parts`), whole, an octave coarser than the near map; off otherwise | the near map's | while aboard | normal 1.5 texels, depth 2 texels |
| wide | the light's own square: `wide_half_width` (380 m at sea, `Game.fitSunShadow`) about `Sun::target`, snapped to a texel grid fixed in the world, the light `Sun::distance` (500 m) away, depth 1–1800 m, or farther both ways under a low sun (below) | setting (1024/2048/4096) | every frame | normal 0.75 × width / size, depth bias −0.0005 |
| view 1, view 2 | the ships in `Sun::casters` the wide map misses, within 1.5 km and 6 km of the camera (÷ `lens`), in quarter-octave squares of at least 60 m: centred on a band's ship when it holds one, else snapped to a world texel grid. The box runs from 100 m above the hulls to 1.2 km past them, or under a low sun to the sea under its square's far side (at most 6 km), which the sea reads it at (since 2026-10-05) | 1024 on Low (the near map's size: its array already holds their layers), 2048 from Medium up | every frame | normal 0.75 texels, depth 2 texels |
| land 1, land 2, land 3 | with `ShadowSettings::land_maps` on and land within 4 km × `lens` (`Sun::land`), the camera's view of the land in three depth slices out to that reach, wherever the camera is (since 2026-10-08; "Land maps" below) | the setting's, at most 2048 | every frame | normal 1 texel, depth 1.5 texels |

`lens` is the camera's magnification against the bare eye's 52° lens (`naval_client::scene::lens`: the binoculars' ×2 to
×32, the sea's 38° views' ×1.4, any narrow camera's): until 2026-10-03 the client left it at 1, so through binoculars every ship beyond 1.5 km and
6 km lost her view map and her own shadows (a Bismarck at 8 km through ×12 and ×24 looked flat).

A pixel samples the first map that covers it (near, aboard, wide, view 1, view 2: `SHADOW_ORDER`, each fading out over
90–98 % of its half-width; the web faded the near map over 80–95 % of its reach as a sphere, leaving its corners unused)
and what no map covers is lit, as `FocusShadowNode.setup` does. Beyond the near, aboard and wide maps it also takes the
land maps', the
darker of theirs and the view maps' (below). The clouds' transmittance is not in it; multiply it in (the sky lane's
clouds bring it).

```rust
renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap()); // off by default
let mut sun = Sun::new(toward_light, player_ship_position);  // distance 500, lens 1
sun.casters = ships_in_view.iter().map(|s| CasterSphere { center: s.center, radius: s.radius }).collect();
sun.subject = Some(CasterSphere { center: followed_hull_origin, radius: hull_radius }); // as drawn this frame
renderer.set_sun(Some(sun));                                   // every frame the sun or ships move
```

**Stability** (2026-10-03). The web's maps sat on texel grids fixed in the world (the wide map on none: it slid with
the player's ship), so a hull under way cast a different set of texels every frame and her shadow edges crawled; at
the chase camera's 345 m the near map spread 2048 texels over the whole 609 m frame (30 cm texels), and the view maps
took turns, so a distant hull's shadow lagged her by a frame every other frame. Now:

- the near map closes on the followed hull when she fills less than the frame (142 m for a Bismarck: 14 cm texels at
  the chase camera), and lays its texel grid from her origin, so she casts the same texels whatever she and the camera
  do but turn, pitch and roll (`SeaScene::sun` sets `Sun::subject` to the focused hull, or the one examined through
  binoculars);
- the near map's quarter-octave steps hold until the frame is 0.2 of a step past the midpoint (a camera resting at a
  boundary flipped its texel size each frame);
- the wide map snaps to a world grid (land shadows hold still); the view maps are both redrawn every
  frame, a band's lone ship centred in hers;
- the soft filter is a cubic B-spline over 6×6 texels from nine gathers (σ 0.87 texel), in place of three's PCFSoft
  (a 3×3 box of bilinear comparisons, four gathers), so an edge glides as a receiver moves rather than stepping.

Measured on the client (Bismarck at 7.5 m/s, clear sky at 09:00, 48 frames at 60 fps, the camera held on her, eye
adaptation off): the shadows' own frame-to-frame second difference with no anti-aliasing fell from 4.13 to 1.66 levels
at 120 m and 5.38 to 4.39 at 220 m; under temporal AA it was at TAA's own floor (1.6–1.9) before and after. The
probe was a scratch script over `set_camera`, `step` and `capture` against shadows-off renders of the same frames.
Under load from other sessions (load average 25–98, M5 Pro, 2026-10-03) the shadow passes took 0.12–0.19 ms and the
opaque pass 0.7–0.9 ms in quiet runs of either build: no cost the noise could show.

**Aboard** (2026-10-13). From a ship's bridge the near map's square at her origin's distance (the frame there, some
40 m across a battleship's bridge view) held turret 2 and left her forecastle, 60 to 110 m ahead, to the wide map's
37 cm texels (Medium): blotched, with streaks where a texel's step lies along the deck. Through the walk's lens it was
worse: past ×1.5 the maps followed the hull nearest the middle of the view whose origin lies ahead, which her own,
behind an eye on her bridge, never is, so they moved to any ship beyond her bow and left her to a view map (15 to 18 cm,
saw-toothed shadows of her barrels on her turret roofs). Now:

- the client keeps the maps on the hull the eye stands aboard (inside her box grown by `ABOARD_MARGIN`, 1 m) while the
  middle of the view comes down to her highest deck within her length and beam grown by her beam
  (`naval_client::scene::looks_down_on`): a look down at her forecastle keeps them, a look over her bow at a ship
  beyond examines that ship as before (`tests/binocular_shadows.rs`);
- with the eye inside the subject's box (`Sun::hull`) the near and aboard maps are two cascades over her: the near map
  holds what the view sees of her box nearer the eye, the aboard map the rest of her, each whole (the bounds of the
  view's slice clipped to her box: `view_of_hull`), in quarter-octave steps held as the view turns, snapped to her
  texel grid. The sea and her shadow on it alongside take them too;
- the near map is always an octave finer than the aboard map (`ABOARD_NEAR_FINER`): the cut along the view is found by
  halving where the near part needs half the aboard part's square (`aboard_parts`). The near map hands her over across
  a band along the view, its last 30 % (`ABOARD_BLEND`; the uniform's `seam`, which `shadow_cover` fades the near map
  by), where both maps hold her, so her shadows soften gradually with no line across her deck.

A cut fixed 40 m along the view (the first version, the same day) showed a line: through the walk's lens the near map
held only the narrow cone's first 40 m of her, several times finer than the aboard map past it, and the two met at the
near square's edge, a band 8 % of its half-width wide. Cutting where the two squares come out alike (one size) hid the
line but coarsened the near map to 4.4 cm, and the barrels' shadows on turret 2's roof grew saw-teeth again.

Iowa's navigation bridge, morning, Medium shadows (2048): the near map 32 m (3.1 cm a texel) and the aboard map 64 m
(6.2 cm) at the walk's 70° lens, 27 and 54 m (2.6 and 5.3 cm) at ×2.3, 23 to 27 and 45 to 54 m at ×4.6; before
2026-10-13, 38 m near (3.7 cm) then 380 m wide (37 cm), or through the lens the view map. The aboard map costs a caster pass of the hulls about her only while aboard, and a layer of the near
map's array always (16 MB at 2048, 4 MB on Low). The land, its buildings, clutter, infrastructure and trees cast into
it as into the near map (`naval_terrain::tree_shadows::TREE_CAST_MAPS`), so a tree beside a berth shades her deck
past the near map too.

**Under a low sun** (2026-10-05). A square map `half` across the light lies `half / tan(elevation)` along it either
side of its centre at sea level: the wide map's 380 m square 2.2 km at 10°, against its box's 500 m toward the light and
1.3 km past its target. A receiver past a box's near or far plane takes nothing from it (`shadow_cover` drops it, and
the view maps skip a hull inside the wide square), so under a low sun a hull 600 m sunward of the target stood in the
square unshaded, and the land's shadow on the sea (terrain/shadows.md, "The land's shadow") ended in a straight line across it.
`ShadowFit` now stands the near and wide maps' lights back, and reaches past their centres, by that footprint
(`depth_reach`: plus 100 m for masts and the shore, at most 6 km), keeping the web's boxes whenever they already hold it
(from about 37° up for the wide map); the sun's depth bias is rescaled to stay 0.9 m along the light. Casters nearer the
light than a box's near plane are still clipped, unless a layer clamps them as the land's do (`caster.wgsl`).

**Land maps** (land-realism lane C, 2026-10-08). The near and wide maps stand about the followed or examined hull
(`Scene::sun`) and the view maps hold hulls alone, so from a plane, the free camera or a watched aircraft nothing on the
land cast a shadow farther than 380 m from the ship: no building, no tree (the forest's shade stood in), no hill on a
town. Three more maps now follow the camera over land.

- *When.* `ShadowSettings::land_maps` is on (`Graphics::land_shadow_maps`: the High terrain, whose trees and buildings
  cast, with shadows from Medium up; the High and Ultra presets, so Medium and Low are as they were) and `Sun::land`
  holds land: `naval_client::scene::Scene::sun` asks the terrain for the land within the maps' reach of the eye
  (`TerrainLayer::land_about`: the bound pyramid at blocks some sixteen to the reach, the land more than 1.5 m above the
  sea, its highest point and how near it lies), never in the port. Open sea, land beyond the reach or a sun under about
  1° fit none, and the four other maps are then exactly as without them.
- *Fitting* (`ShadowFit::fit_land`). The camera's frustum from where land can first be seen (the nearest land, or the
  eye's height above the land's top, along the frustum's corner rays; at least 10 m) to the reach (`land_reach`: 4 km ×
  `lens`, at most 24 km), clipped to the slab from 10 m below the sea to 80 m above the land's top, cut in three by the
  practical split (0.75 geometric). Each slice's bounding square across the light, in quarter-octave steps held until a
  step is more than 1.2 too big, centred on a texel grid fixed in the world: flying on or turning a little, a map keeps
  its size and a point of the land keeps its place within a texel, so the shadows hold still without shimmer. Its box
  runs from 300 m sunward of the slab's highest corner to 300 m past its lowest, at most 30 km: under a sun 3° up or
  lower, where the square's footprint runs farther, it keeps its slice's own ground and gives up first the square's far
  corners, then its reach toward the sun (where the land's caster is clamped onto the near plane). Half-widths seen: 1, 2
  and 4 km from 150 m over Redondo Beach (texels of 1, 2 and 4 m at 2048); 431, 512 and 609 m straight down from 500 m;
  2, 2.9 and 3.4 km from 22 m with the coast 2 km off.
- *Casters.* The land (`land_shadows.rs`: level 0 in land 1 and 2, level 1 in land 3, pushed back 10 and 20 m along the
  light so it never shades itself, from up to 8 km beyond each square; terrain/shadows.md, "The land's shadow"), the placed
  trees (the instances placed for the view and the near and wide maps, the view's sunward sides moved out by a tall
  tree's shadow, at most 400 m, so a tree just beyond the frame still throws its shadow in: `tree_shadow_margins`) and
  the buildings (each chunk inside the map's sides and the view's sides moved out by a 150 m building's shadow, at most
  1.5 km). Hulls, aircraft and the combat and
  water effects cast into none of them: a hull shades herself through the near, wide and view maps' finer texels.
- *Receiving* (`shadow_land`). Beyond the near and wide maps, which already hold the land's casters, a receiver takes
  the darker of the view maps' shadow and the land maps' (never their product: both may hold the land). Among the land
  maps it takes the first that covers it, fading to the next over 90–98 % of the half-width. Their filter is four
  bilinear comparisons half a texel apart (`shadow_land_compare`, 3 texels across, its weights linear in the receiver's
  place, so an edge glides): with the soft filter's nine gathers the opaque pass cost 0.4–0.9 ms more over a view full
  of land, with these 0.1–0.3 ms. The forest's shade steps aside where they cover (`shadow_land_share`, as for the near
  and wide maps), and the sea reads them too (below).
- *Ship views.* Nothing the near and wide maps cover changes, and the land maps hold no hull: in the chase view off
  Palos Verdes not a pixel moved (the coast lay beyond the land maps' reach); from 22 m with the coast 2–4 km off, 0.01%
  of the pixels, all on the coast.
- *Memory.* The near map's array grows from three layers to six: 48 MB more at 2048.

What Bill sees: from a plane, the free camera or a watched aircraft, the houses, blocks and trees of a town cast
shadows on the streets and each other out to 4 km; a forest reads as crowns with shaded gaps (it was an even darker
mass); a hillside's trees throw their shadows down the slope. Before/after stills (land maps on, `-off`, and the
`shadow-cascades` tint, `-cascades`): `.build/land-lanes/stills/C/` in the main checkout.

**What the land maps cost** (M5 Pro, Metal, 1920 × 1080 at the Ultra preset, `test-fast`, 2026-10-08, load average
13–50 from other sessions: the same held frame, `layers {land_maps}` on and off interleaved, eight rounds of four
frames each, the frame's GPU total; the land passes' spans overlap other work on this tiled GPU):

| View | Land maps' half-widths | Their land patches | GPU total p10, off → on | p50 | The land passes' span (p50) |
| --- | --- | --- | --- | --- | --- |
| Redondo Beach, 150 m, toward the shore | 1024, 2048, 4096 m | 42, 169, 183 | 13.09 → 13.81 ms (+0.72) | +0.77 | 0.42 ms |
| Hermosa Beach, 500 m, straight down | 431, 512, 609 | 18, 21, 11 | 5.66 → 5.88 (+0.22) | +0.30 | 0.17 |
| The port, 400 m | 724, 1722, 4096 | 25, 110, 145 | 11.75 → 12.30 (+0.56) | +0.59 | 0.17 |
| Palos Verdes canopy, 60 m | 431, 1024, 4096 | 18, 64, 215 | 6.37 → 7.08 (+0.71) | +0.72 | 0.43 |
| Off Palos Verdes, 22 m (a ship's height) | 2048, 2896, 3444 | 54, 145, 72 | 13.97 → 14.69 (+0.72) | +0.67 | 0.13 |
| Kamakura, 150 m | 609, 1448, 4096 | 11, 76, 185 | 13.98 → 15.06 (+1.08) | +1.23 | 0.49 |
| Yokosuka, 500 m, over forest | 1218, 2436, 4096 | 85, 292, 175 | 9.64 → 10.65 (+1.01) | +0.81 | 0.57 |

So 0.2–1.1 ms, within the 1.5 ms budget. At 07:30, the sun low and the trees' margins in, the p10 deltas were Redondo
+0.76, the canopy +0.80, Hermosa +0.08, Kamakura +1.26 and Yokosuka +1.17 ms. Land 3 at level 0 took 600–780 patches (300–400 k triangles) in these views
and cost no more than the noise could show; level 1 is for slower GPUs. The land passes themselves span 0.1–0.6 ms;
the rest, not measured apart, is the receivers reading the maps. Cheaper, if needed: two land maps instead of three,
or the trees in land 1 and 2 only.

**Casting.** A layer draws its casters in `SceneLayer::draw_shadow(&self, map: &ShadowMap, pass,
ctx)`, once per map drawn this frame, with pipelines from `RenderContext::add_caster_pipeline`
(`CasterPipeline::new("vertex_entry")`, vertex buffers and groups 1+ like `ScenePipeline`): depth
only (`Depth32Float`, reversed, `GreaterEqual`, cleared to 0), both faces, no raster bias, as the
web's caster pass. Group 0 there is the map's **shadow camera** in the ordinary `naval::camera`
block (`camera.view_projection` is the light's orthographic matrix), so a layer's usual vertex code
places casters; `ctx.camera` is the same block and `map.view`/`map.projection` bound it for culling.
Caster pipelines must not import `naval::shadow` (their group 0 has no maps). A caster that alpha-tests
(`discard`) names a fragment stage, `CasterPipeline { fragment_entry: Some("fs_…"), .. }`, with no colour targets
(the land's trees, terrain/trees.md "Shadows"). The maps are fitted before the layers prepare, and
`PrepareContext::shadow_maps` hands each layer this frame's seven (`on`, `drawn`, their matrices), so a layer can place
casters for them in its own compute work (the trees out of view whose shadows fall into it). The land maps take the
land, its trees and buildings alone: a layer that draws hulls, aircraft or effects returns at once for a map whose
`kind.is_land()`.

**Receiving.** Group 0 of every scene pipeline now also holds the maps, so any shader can:

```wgsl
#import naval::shadow::{sun_shadow, water_sun_shadow}
let lit = sun_shadow(world_position, world_normal);          // 0..1, 1 when shadows are off
let sea = water_sun_shadow(world_position, 2u);               // the sea's filter: 0 off, 1 low, 2 medium, 3 high
```

| Group 0 binding | Holds |
| --- | --- |
| 0 | `Camera` (unchanged) |
| 1 | `Shadows` uniform (960 bytes): eight `ShadowMap { matrix (world → u, v down, reversed depth), params (size, 1/size, normal bias, depth bias), state (on, half width, half height), hulls (a view map's hulls along the light in its depth: first, last, has them) }` (near, wide, aboard, view 1, view 2, land 1–3), `focus` (xyz, reach), `settings` (intensity, soft, enabled, any land map on), `toward_light`, `seam` (aboard, the band along the view where the near map hands her to the aboard map) |
| 2 | `texture_depth_2d_array`: layer 0 near, 1 aboard, 2 view 1, 3 view 2, and with the land maps on 4–6 land 1–3 (map `i` ≥ 2 is layer `i` − 1) |
| 3 | `texture_depth_2d`: wide |
| 4 | `sampler_comparison` (`GreaterEqual`, linear: one comparison is a 2×2 bilinear one) |
| 5 | `CameraMotion` for temporal AA's motion pass (see "Temporal anti-aliasing") |
| 6 | `texture_3d<f32>`: the aerial-perspective volume (`naval_render::aerial`, `naval::aerial`; `haze.md`): 32 × 32 columns × 32 slices of in-scatter, then their opacity; zeros are clear air |
| 7 | its trilinear sampler |
| 8 | soft occluders (`SunOccluders`, since 2026-10-04): a storage buffer of `SUN_OCCLUDER_SIZE`² (256²) light-space texels in two planes of half floats: where the occluders along each texel begin and end, then their optical depth |
| 9 | `SunOccluderUniform`: the map's axes (world to u, v and depth along the light), the planes' depth origins, on |

**Soft occluders** (2026-10-04). Smoke stops part of the light, which a depth map cannot hold. A layer that has such
occluders draws them into a light-space map of its own in its `prepare`, copies it into
`PrepareContext::sun_occluders.texels` and writes the uniform (the combat effects' smoke: `native/docs/combat-effects/renderer.md`, "Sun
shadows"); the renderer zeroes the uniform every frame first, so with none drawn they let everything through.
`sun_shadow` and `water_sun_shadow` multiply in what they let through at the receiver's depth along the light
(`occluder_transmittance`: the optical depth spread evenly from the first occluder's front to the last's back, four
texels filtered bilinearly, at the shadows' intensity), so the ships, the sea, the aircraft and the land take the
smoke's shadow with no change of their own; `sun_shadow_maps` is the depth maps alone, for a receiver that shades
itself (smoke). A storage buffer, so no stage spends a sampled texture on it (the ships' fragment stage holds 15 of
16, the sea's 16).

Lower-level functions: `shadow_map_visibility(index, position, normal)` (0 near, 1 wide, 2 aboard, 3
and 4 view, 5–7 land; three's `mix(1, filter, intensity)`, lit outside the frustum), `shadow_cover(index,
position)`, `shadow_near_share(position)` (the near map's cover), `shadow_compare(index, coord, soft)`,
`shadow_land(position, normal)` (the land maps alone, lit where none covers), `shadow_land_visibility` and
`shadow_land_compare` (their filter), `shadow_land_share(position)` (their share of `sun_shadow`'s weights, for a
stand-in that steps aside where they cover), `water_land_shadow(position)` (the sea's reading of them).

**What the ocean needs** (`src/game/WaterShadows.ts` (historical), and the view maps since 2026-10-05):
`water_sun_shadow(position, quality)` reads the five maps by the same weights as `sun_shadow` (the near map, then the
aboard and wide maps, then the two view maps, each fading to the next over 90–98 % of its half-width; a pixel the near,
aboard or wide map holds whole looks no further, and a map that is off is passed over), offset straight up by each
map's normal bias. Low takes one bilinear comparison of each, Medium filters the near and aboard maps with the soft
filter, High the wide map too, as the web did; the view maps take their own 2×2 filter at every quality (below). The ocean then fades it
with the light: `mix(1, shadow, clamp(sun_intensity / 2, 0, 1))` (`OceanSurfaceMaterial`, faint under the moon), and
multiplies the clouds' shadow. It needs no new binding: the surface's group 0 is already the renderer's camera layout.

The web game left the view maps off the sea, "a sampler short of the limit" (`updateWaterShadows`: its ocean material
stood at WebGPU's 16 sampled textures a stage, and each map was a texture of its own), and ended the wide map at its
edge with a hard cut, so every hull beyond the wide map shaded herself but threw no shadow on the water. Here the view
maps are layers 1 and 2 of the array the near map is layer 0 of (binding 2), which the sea already binds: reading them
costs no binding. The sea casts into no map, so it cannot shade itself (no acne however the waves move it): the offsets
only keep a hull's waterline from shading the water on her sunlit side, and a view map's 0.75-texel offset straight
up (a lone battleship's square is 15 cm a texel at 2048, a band of several spread hulls 1–2 m at 1024) moves her
shadow's edge toward her by that over the sun's slope: centimetres on a lone hull, a few metres under a low sun on the
coarsest band. A band's lone ship has her map centred on her and a band of several is snapped to a world texel grid, so
the shadow on the sea moves with her, without swimming. Where a view map takes over from the wide one the two fade over
the wide map's last 8 %, the same band in which its casters' spheres pass to the view maps.

**Only the hulls' shadows from the view maps** (`water_view_shadow`). The land draws itself into the view maps from
the Medium setting up, clamped onto their near plane when it stands sunward (terrain/shadows.md, "The land's shadow"), so a
distant hull under a fjord wall is shaded. Read whole on the water, that shadow lay only in her view map's square, which
under a low sun runs a few hundred metres across and kilometres along the sun's bearing, and ended at its straight edges
across the sea beyond the wide map, which takes no land shadow: under Ravnsund's island at 17:27 the sun's glitter was
cut by a dark strip with a ruled edge. Each view map now carries its hulls' extent along the light
(`ShadowMap::hull_depths`, `ShadowMapUniform::hulls`: from 8 m before the first sphere's top to 8 m past the last one's
bottom, in its reversed depth), and the sea counts a texel's occluder only between them: three gathers (the receiver's
depth, the hulls' first and last) in a 2×2 bilinear filter, where the soft filter would take 27. A hull's own shadow
on the water is all inside that span; the land clamped sunward and the 100 m above her are not, nor what lies past
her. Land within her span along the light and inside her square (a quay alongside) still shows on the water; a band
of several hulls spans all of theirs. The near and wide maps' land shadows on the sea are unchanged.

**The land maps on the sea** (2026-10-08, `water_land_shadow`). Beyond the near and wide maps the sea takes the land
maps' shadows of the land, its trees and its buildings (a cliff's, a breakwater's, a waterfront block's on the water
off it), the darker of theirs and the view maps', one bilinear comparison each (texels of 1–4 m, about a pixel on the
water there), offset straight up by their normal bias. Under them the sea's land shadow no longer stops at the wide
map's square. They hold no hull, so a distant hull's shadow on the water is still her view map's alone.

Settings follow `GraphicsController.applyShadows`: `ShadowSettings::for_quality("off" | "low" |
"medium" | "high" | "ultra")` gives 0, 1024, 2048, 4096, 4096 texels, the soft filter on, view maps on,
380 m wide, intensity 1. A new map size remakes the textures (group 0 rebuilds itself). The view maps draw on every
setting with shadows since 2026-10-05: at 1024 on Low (until then none, so on the Medium preset, whose shadows are
Low, every hull beyond the wide map had no shadow at all), 2048 above. The land casts into them from Medium up only.
`ShadowSettings::land_maps` (off by default) adds the land maps at the setting's size, at most 2048, and three layers
to the near map's array; the client turns it on with the High terrain (trees and buildings) and shadows from Medium
up (`Graphics::land_shadow_maps`: the High and Ultra presets; the Medium preset is unchanged), and `layers
{land_maps: false}` turns it off for an A/B until the next graphics change.
`FrameReport::shadow_maps` lists the maps drawn; `Renderer::shadow_report()` and the harness
`telemetry` (`shadows`) list each map's size, on, drawn and half-width.

**What the view maps on the sea and on Low cost** (M5 Pro, 1920 × 1080, 2026-10-05, load average 2–16 with another
session's client drawing a window: the GPU was 47–73 % busy before ours, so take these as upper bounds):

| Sun shadows setting (`bench --scene measure30`: 30 ships, the chase camera; before → after) | Hull caster draws | Caster triangles | The sun maps' passes, serial ms a frame |
| --- | --- | --- | --- |
| Low (1024) | 18 → 42 | 92 k → 363 k | 0.01 → 0.14 (view maps 0.09) |
| Medium (2048) | 43, unchanged | 367 k | 0.38 → 0.51 (the same draws: noise) |
| High (4096) | 44, unchanged | 371 k | 0.28 → 0.43 (the same draws: noise) |

| The sea's water pass, the old `water_sun_shadow` → the new (one client, swapped by hot reload, medians of 4 interleaved rounds) | chase, the distant-shadows battle | toward the Fletcher | overhead a hull 2.9 km off | `measure30`'s chase |
| --- | --- | --- | --- | --- |
| Medium preset (shadows Low, water shadows Low) | +0.09 ms | +0.10 | +0.05 | +0.07 |
| High preset (shadows Medium, water shadows High) | +0.17 | +0.21 | +0.05 | +0.28 |

The battle is `iowa;fletcher:moving,cleveland:moving;bismarck:moving,yamato:moving` at 07:00 in clear weather, the enemy
3 km off. Low now draws the casters Medium always drew, at a quarter of the texels: about 0.1 ms of passes here.
Where a view map covers much of the water the hulls' 2×2 filter costs little (overhead a distant hull: +0.05 ms,
against +0.49 with the soft filter on the view maps, tried first); the +0.2–0.3 ms left on High in the chase views
stayed with every filter tried and with the loop unrolled by hand, so it comes with the sea shader's size rather than
with what the view maps cover. The Low preset draws no shadows, so the weakest machines pay nothing.

Not ported: cloud shadows (the clouds lane), the port's berth fitting (`wide_half_width` is the
caller's: hull length / 2 + 30, clamped 60–380), casters three draws itself (skinned, alpha-tested:
a layer's business), and variance shadow maps (the filter is a B-spline over comparisons).

Test: `cargo test -p naval-render --test all -- shadows::` renders a 10 m square caster 10 m over a ground
plane and checks the shadow lands where the light puts it, through both filters and at part
intensity, and a second square 900 m off, beyond the wide map, shading the ground and the sea (`water_sun_shadow`)
through view 1 at 1024, 2048 and 4096; the unit tests in `shadows.rs` check the fitting (orientation, projection,
quarter-octave steps and their hold, texel snapping, the near map closing on the followed hull and keeping her texel
phase as she moves, the wide map's world grid, both view bands every frame and their sizes, a frame fitted twice fitting
the same maps, the near and wide boxes holding their squares' footprint on the sea under a low sun). The land maps
(2026-10-08): the GPU test's far square with them on shades the ground and the sea through land 1 (and with them off
or no land, the report lists the near and wide maps alone and the far square is lit); the unit tests check that every
point of the ground and the land's top the camera sees out to the reach lies whole in a land map, that the maps are
off without land, without the setting or with the sun on the horizon (and the four others unchanged), that flying on
and turning a degree keeps each map's size and a point's place within its texel, that looking straight down from
500 m the first map's texels are under 0.6 m, and that under suns 1.5–5° up on five bearings the boxes hold the ground
the camera sees, at most 30 km deep, with the depth bias at 1.5 texels along the light.

The test scene's box casts on the grid and on itself: `cargo run -p naval-harness -- shot --camera
250,30,70 --out .build/native/render-core-2/shadows-close.png` (`--shadows off` to compare).

## Contact shadows

Under a clear sun the hulls read as grey clay: their small parts barely shade each other. The near map is 2048 texels
over the whole hull (15 cm on Iowa), its filter a B-spline over 6 × 6 texels, its receivers lifted 1.5 texels along
their normal and 2 along the light: a ledge's shadow starts 20 to 50 cm below it, and a rail's, a barrel's or a gun
mount's shadow blurs away, where in real sun a ledge a metre up casts an edge about a centimetre soft. A finer map
cannot reach it (High against 4096 maps and the High occlusion differed in under 0.5 % of the pixels of Iowa at 160 m).
Contact shadows take what the eye sees of the parts close above a surface instead (`naval-render/src/contact.rs`,
`shaders/contact.wgsl`):

1. **The contact depth.** While `ShadowSettings::contact` is on with the maps (every preset with shadows), the layers
   that take contact shadows draw their surfaces depth-only for the frame's own camera, jitter and all, into
   `contact.depth` (`CONTACT_DEPTH`, the scene's size), between the sun maps and the scene pass:
   `SceneLayer::draws_contact_depth` and `draw_contact_depth`, group 0 the frame's camera in the caster layout, so a
   layer's caster pipelines place its surfaces where the scene pass will. The hulls do (`ShipLayer`: the same ranges of
   the arena the scene pass draws, and the ensigns), those near enough for the walk to span a pixel and a half on them
   (`Fleet::contact`, `contact::reach_distance`: 370 m at the battle's lens on High's 1081 rows, further through
   binoculars). Their thin members stay out: the tubes are discarded under two pixels and the impostors are blended,
   so the pixels about them belong to the surface behind.
2. **The march** (a full-screen pass into `contact.mask`, `CONTACT_MASK`, `R8Unorm`): each pixel holding a surface
   walks toward the light through that depth, a pixel a step, over 24 of its own footprints, at most `REACH_M` (1 m,
   about what the near map's offsets and filter leave out), and is in shadow where a surface stands in front of the
   walk by more than a bias (its own surface's slope over three quarters of a pixel) and less than a thickness
   (30 cm, or four footprints), compared in the depth buffer's own units (reversed depth runs linear across the screen
   along the walk, so no step divides). A part met over the last 40 % of the walk fades out, so its end draws no line;
   a walk spanning under 5 pixels fades out, under 3 it is not taken (there the near map's texels are finer than the
   pixels and its offsets about a pixel). A pixel whose face (rebuilt from its neighbours on
   the same side of any edge) turns from the light is lit: its lighting's own cosine takes the sun there, and a curved
   part's facets would otherwise cut a hard edge across its terminator. 1 lit, 0 shadowed, the maps' intensity applied.
3. **The receivers** read the mask at their own pixel and take the darker of it and the maps' shadow, for the sun
   alone (`ship.wgsl`'s `lit`: both see the same parts, so never their product); never the sky's light, which the
   occlusion handles. The hulls' scene draws and their impact marks read it (`fs_main`, `fs_decal`); the thin members,
   the overlays (a sighting's silhouette, a ghost) and a second view do not.

Deterministic for a still camera: the walk takes a step a pixel while it is at most 24 pixels long (a longer one would
spread its 24 steps with a new start each frame, which a still capture, drawn again on the same frame, never averages:
2-pixel steps left saw-teeth along the edges they found). Temporal AA's jitter gives the edges their coverage. With no
layer drawing contact depth, or no light, the mask is cleared to lit (once, until the next march); with contact shadows
off neither texture exists. `layers {contact_shadows: false}` switches them off for an A/B until the next graphics
change, and `FrameReport::contact` says whether the march ran.

What it cannot do: a part the camera does not see (behind another, off the frame's edge) casts no contact shadow (the
maps' stands there); a thin part passed behind within the thickness shades as if it were that thick; beyond the
walk's metre the maps' soft shadow is all there is, so a deck fitting's shadow is crisp at its foot and soft at its far
end, and its whole shadow sharpens only in close-ups. Aircraft keep their own sun map (7 mm texels about the nearest
airframes, `aircraft.md`) and take none; the land takes none (its depth and its shader would need the same two steps).

**What Bill sees.** Close up, the deck fittings sit on the deck (a crisp dark line along a reel's, a box's and a Carley
float's foot), a ladder's rails throw thin shadows on the bulkhead behind them, a gun tub's rim shades its floor;
at 160 m a dark line under every gallery and platform edge where the near map's offsets let light in. Before and
after pairs (morning, clear, Iowa underway): `.build/native/look-render/` in the render lane's worktree, the befores
from master's cached client.

**Cost** (Apple M5 Pro, `dev-client`, the High preset at 3456 × 2234, so a 1672 × 1081 scene, 2026-10-20: one
client, `layers {contact_shadows}` on and off interleaved over six rounds of 20 frames, each round waiting up to a
minute for other clients and the GPU to go quiet; load average 15 to 38 from other sessions, and in the first two
views another client still busy on the GPU at times):

| View | Contact depth, p50 | March, p50 | Frame's GPU total, on − off (p10, p50) |
| --- | --- | --- | --- |
| The port's opening view, the Iowa task force | 0.14 ms | 0.09 ms | +0.41, +0.26 ms |
| 60 m off Iowa, 40° lens (she fills the frame) | 0.02 | 0.24 | +0.28, +0.21 |
| The chase camera, 345 m | 0.01 | 0.03 | +0.36, +0.78 |

The passes' own times are the cost: about 0.25 ms where hulls fill the frame, a few hundredths at the chase camera's
distance; the frame's totals swing by ±0.3 ms with the other sessions' load (the chase view's +0.78 is that). The
march is bound by its texture reads (dividing nothing per step saved nothing); a walk of 16 pixels would save a third of
it at the price of two thirds of the close-up's reach. With the walk's floor at 1.5 pixels (the hulls within 740 m in
the depth) the port's depth pass cost 0.24 ms, and with every drawn hull in it 0.21 ms under heavier load; a floor of
3 pixels (370 m) brought it to 0.14.
