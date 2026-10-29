# The land's shadows

How the land shades and is shaded by the sun: it casts into the renderer's sun maps, darkens itself with its own baked
sun visibility, and takes its sky light from the open sky. The trees' shadows are in [trees.md](trees.md), "Shadows".
## The land's shadow

With the sun's maps on, the land casts into them, so cliffs, fjord walls and hills shade the ships and the sea under
them, not only themselves through their baked sun visibility.

**What casts, and into what.** With the sun's maps on, the land draws itself into them (`land_shadows.rs`,
`caster.wgsl`). The client's gating (`naval_client::scene::land_casts`, live from Graphics → Shadows): off with shadows
off; the near and wide maps on Low; every map from Medium up, the view maps too (squares about hulls kilometres off, so
a distant ship in a fjord's shadow is shaded). The battle's land and a theatre scene's (cruise, flight and field
windows, a world's) are the same layer and settings. Each map takes its own patches, chosen every frame from the maps
the renderer fitted (`PrepareContext::shadow_maps`, as the trees do; the old caster drew up to 1024 patches about a
host's focus into every map): the level-0 patches whose bounds (the bound pyramid's base blocks, land above the sea
only) meet the map's box: inside its square across the light, short of its far plane, and anywhere toward the sun. A
coarse pass over the pyramid's level 3 (2.56 km blocks) rejects most of a low sun's long footprint first; a map's
selection takes at most 0.021 ms on the CPU (a 24 km rolling field, every sun the unit test tries), at most 1024 patches
a map. What the camera sees plays no part, so a cliff behind the camera or kilometres off shades a ship it cannot see.
The render core's land maps (2026-10-08, native/docs/render-core/shadows.md "Land maps": the camera's view of the land out to 4 km on the
High and Ultra presets) take the land the same way, pushed back further (`base_push`: 10 m, since their texels reach
4 m), the farthest at level 1 (`caster_level`: 40 m triangles on its 8 km square, a quarter of level 0's 600-780 patches
from the air, pushed back 20 m past where they bridge a gully above the drawn land). Through them a hill shades the
town and the trees behind it, and the sea under a cliff, wherever the camera is. The selection's limit is 8 km beyond
a map's square (its half-diagonal from its target, so a land map through a lens, 11.6 km across at ×6, takes land all
across it), and a map cut short at 1024 patches keeps those nearest its target.

**Long shadows from a low sun.** A 300 m cliff under a 10° sun throws its shadow 1.7 km. Three things carry it:

- *Clamped, not clipped, at the near plane.* `caster.wgsl` clamps a vertex nearer the light than the map's near plane
  onto it (`clip.z = min(clip.z, clip.w)`: depth clamping, without `unclipped_depth`), so a mountain far sunward, past
  every box's near plane, still writes its depth and shades what lies behind it. The selection has no near limit to
  match (`LAND_THROW_MAX_M`: 8 km beyond the map's square, its half-diagonal from its target, along either axis bounds
  it).
- *Boxes as deep as their squares' footprint.* The near and wide maps' boxes reached a fixed 500-800 m toward the light
  and 1.3 km past their target, but under a low sun a square `half` across lies `half / tan(elevation)` along the light
  either side (the wide map's 380 m square: 2.2 km at 10°). A receiver past a box's near or far plane took nothing from
  it: the land's shadow on the sea ended in a straight line across the glitter there, and (on master already) a hull
  600 m sunward of the target under a 10° sun stood in the wide square unshaded, skipped by the view maps as inside it.
  The render core now stands both boxes back and reaches past their target by that footprint (plus 100 m, at most 6 km;
  native/docs/render-core/shadows.md, "Sun shadows"), the sun's depth bias kept at 0.9 m along the light. A high sun keeps the old boxes.
- *The view maps* (a hull's box, from 100 m above her highest point to 1.2 km below) take the land the same way: what
  stands sunward of the box is clamped onto its near plane.

**No double darkening.** The land darkens itself with its baked visibility (`sunlight`, the field's 40 m line sweep
with a penumbra). Read through the maps too, its slopes in its own shadow would go doubly dark, and the two would not
line up (texels of 14 cm to 1.5 m against the field's 40 m, different offsets and filters). Two things keep them apart:

- *The land never shades itself through the maps.* `caster.wgsl` pushes each vertex back along the light by its map's
  base (2 m in the near and wide maps, 6 m in the view maps, whose land kilometres off may be drawn at a coarser level
  than the caster's level 0) plus how far its slope runs along the light across the receivers' filter: 3.5 texels times
  the cotangent of its angle to the light, at most 40 m (the texel from the shadow camera's projection). A lit slope then
  lies in front of its own caster under every texel of the soft filter, and only the first metres of a shadow along the
  light from its caster are lost (the low shore's own on the sea beside it). The caster and the drawn land are the same
  20 m (40 m) triangles near the camera, so nothing else separates them.
- *The darker of the two, not both.* `terrain.wgsl` takes `min(baked, sun_shadow_maps)` and then the smoke's
  transmittance (`occluder_transmittance`, as `sun_shadow` would). Where a map holds the land's shadow the two agree and
  nothing changes; a ship's, tree's or building's shadow on lit land comes through the maps as before; beyond the maps
  (which fade to lit at their edges) the baked shadow stands alone, so there is no seam where they end: the land outside
  them is the same with casting or without, and inside them it changes only where a map's shadow is darker than the
  baked one (the maps' sharper edge inside the baked penumbra). The trees keep their foot's baked visibility times the
  maps' (binary shadows agree either way); the buildings, which never took the baked term, now take the land's shadow
  inside the maps.

**Checked.** GPU test `the_land_casts_into_the_sun_maps_and_does_not_shade_itself_twice` (Kawah Lagoon at a 10° morning
sun, the render core's test sea as a receiver): the land casts into the near and wide maps; the sea 1.2 km west of the
900 m cone is 57% as bright with it casting; the cone's lit east face and the slopes it shades, inside the maps, are the
same with casting and without (no pixel more than 8 levels apart, mean under a level; it wrote no difference at all).
Unit tests (`land_shadows`): a 300 m wall 1.5 km sunward of a 10° sun casts into the near and wide maps and one 3 km
down-sun into neither; the chart offset moves the patches; the coarse pass keeps every patch a test of each patch alone
finds.

**Cost.** Apple M5 Pro, Metal, 1728 × 1030 at the High preset's 70% render scale, release, 2026-10-05, load average
5-16 with other sessions' GPU use sampled at 0-67%. The same frozen frame drawn 240 times with the land in the maps and
out (`layers {land_shadows}`, interleaved over four rounds), the renderer's pass timestamps, medians; the shadow
passes' spans (their serial costs undercount: on this tiled GPU they overlap other work):

| Scene | Land patches (near, wide) | Caster triangles | Shadow passes, Low | Medium | High |
| --- | --- | --- | --- | --- | --- |
| Ravnsund, under the island at a 10° sun, 4 views | 17-33, 38 | 28-36k | +0.00-0.03 ms | +0.02-0.06 ms | +0.07-0.09 ms |
| Kawah Lagoon shore, forest, 10° sun, 3 views | 3-25, 15 | 9-20k | within ±0.03 ms | -0.2 to +0.01 ms | -0.05 to +0.07 ms |

The frame's GPU busy time moved within its noise in every view (±0.1 ms; one Kawah view at High read 4 ms apart under
another session's load). Among Kawah Lagoon's trees the land costs nothing measurable: it fills the maps' depth before
the trees' alpha-tested cards, whose hidden fragments it rejects (before master's tree rework of the same day, measured
the same way, the passes were 0.03-0.08 ms cheaper with it). The gate-B bench's
`vestfjord` (noon, mid-fjord, the land far from the maps; 1920 × 1080, 20 s, load 2-3): GPU p50 6.13 ms on master and
6.16 ms after, the shadow passes 0.019 and 0.012 ms a frame live. The view maps took no land in these scenes (no hull
there near land); a hull in one takes patches only from its small square's prism.

**Open.**

- *The sea beyond the maps.* The land's shadow on the sea reaches only as far as the maps: the wide square's sides
  (380 m either side of the followed hull, across the light) cut a broad cliff shadow on the sea, faded over 90-98% of
  the half-width; along the light the square's whole footprint is covered now. It shows mainly in the sun's glitter,
  looking toward the sun. A stand-in beyond the maps would be the land's visibility at sea level (the baked sweep
  holds the seabed's, not the surface's), read by the sea (`water_sun_shadow`, whose fragment stage has no sampled
  texture to spare: a storage buffer, as the smoke's occluders are) or a coarse land-only map in light space.
- ~~*The view maps and the sea.*~~ The sea reads the view maps since 2026-10-05, and takes from them only the shadows
  of the hulls each was fitted to (native/docs/render-core/shadows.md, "Only the hulls' shadows from the view maps"), so the land drawn into
  them shades a distant hull without lying on the water in her square alone. Land within her own span along the light
  and inside her square (a quay alongside) would still show there.
- The trees' and buildings' casters are still clipped at a map's near plane (the deeper boxes hold most of them).
- Not checked in a theatre scene with land in the maps in daylight (the test's cruise scene had none near); its path is
  the battle's layer and settings.

## The land's own shadow

The land darkens itself by its baked sun visibility (`PreparedField::sunlight`, `field.rs`: a line sweep from the sun's
side over every sample, with the web's 1.2° penumbra), recomputed once the light has moved past 0.29° (`SUN_TOLERANCE`,
`sunlight.rs`). `PreparedLand::with_sun` computes it with the rest of a land on the worker that prepares it. A theatre window is 2401² samples: the sweep is 25-28 ms of CPU and the texture 5.8 MB, and the
theatre's sun steps a quarter degree at a time (docs/archive/native-theatre-history.md, "The land's frame budget"), so
the render thread spent that every 2 minutes at 1×, every 15 s at 8× and every 2 s at 64×, and in the first frame of
every scene, which built its land without a shadow.

Since 2026-10-05 (`goldflag/land-hitches`):

- **On the host's worker.** A layer handed a `LandWorker` (`set_worker`: a closure that runs a job on another thread)
  computes the sweep there, the old shadow drawn meanwhile, and the first `prepare` after it is done uploads it whole
  (about a millisecond). The upload stays on the render thread on purpose: wgpu holds its queue's pending-writes lock
  through a texture write's copy, and with the worker uploading, a scene's first frame (whose layers write a great deal)
  waited 18-20 ms behind a worker at the utility class holding it. One job at a time; a light that moved on meanwhile is
  taken once it lands. `set_background_work(false)` puts it back in the frame (a theatre's manual clock outside the Map
  view, whose scripted frames must repeat; a job still out then is let go and the frame computes the light itself), and
  so does a land's first shadow when it was made without one, and a job the worker dropped unrun. `sun_pending` and `sun_light` say where it stands. The client's worker is a thread a job at the
  utility class (`naval_client::land::sun_worker`), handed to every theatre land.
- **With the scene's build.** A theatre land is prepared with `PreparedLand::with_sun` for the scene's light
  (`theatre::land_layer`), on the scene's build thread in a live theatre, so the first frame computes nothing. A sun that
  stepped during the build is taken on the worker.
- On wasm32 there are no threads: the render thread computes it, as before.

The GPU test `the_land_shadow_moves_on_the_worker_and_lands_as_the_render_threads` (`background.rs`) holds the first
shadow in the frame, the moved one on the worker with the old one drawn until it lands, the landed texture equal to the
sweep byte for byte, and the render thread taking it again with background work off.

## The open sky

The land's sky light is the open sky, baked once in compute into an `Rgba8Unorm` storage texture
(`shaders/sky_bake.wgsl`, `src/open_sky.rs`): red the open sky; with trees its green and blue carry the forest's shade
([trees.md](trees.md), "Shadows"). The bake walks the spiral the web game's bake actually ran, not the 16 straight
directions `TerrainBake.ts` reads as (historical): its two nested TSL loops both named their counter `i`, so step `s`
walked direction `s` (s × 22.5° + 0.2 rad) at 40 × 1.25^s m, one spiral of 20 samples whose steepest rise over the
tangent plane sets one horizon, repeated by 16 outer passes. The spiral finds a higher horizon than one straight walk,
so valleys, gullies and the feet of cliffs are darker than the as-written average. The game shipped that look, so the
native bake keeps it. `open_sky::open_sky` is the same on the CPU, pinned to the web's read-back in a test, and a GPU
test holds the bake to it; `open_sky_as_written` keeps the walk as written. The bake costs below a frame's noise
(1201² × 16 directions × 20 steps).

History (the web comparison of the open sky, the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
