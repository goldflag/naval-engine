# The water mesh

The clipmap the surface is drawn on, how its finest level follows the view's standoff, and the fan a magnified view
draws instead. Index: [README.md](README.md).

## The clipmap and the view's standoff

The web's clipmap, ported as it was, had six levels from a 256 m square whatever the view: 4 m cells on High (16, 8 and
2 m on the other tiers), each level's cells 2/64 to 4/64 of their distance from the centre, 40 to 70 pixels across at
1080p. Beside a hull from a low camera the water was 4 m facets, and every wave shorter than about 16 m was in the
normals alone. Now (`geometry.rs` `Clipmap`, `QualityTier::clipmap`):

| Tier | Cells a level edge | Levels | Finest cell | Was |
| --- | --- | --- | --- | --- |
| Low | 32 | 6 | 8 m | 16 m |
| Medium | 64 | 7 | 2 m | 8 m |
| High | 128 | 9 | 0.25 m | 4 m |
| Ultra | 256 | 9 | 0.125 m | 2 m |

- **Every tier's lattice reaches 4,096 m** from its centre (`CLIPMAP_REACH`, as before), where the horizon ring takes
  over; a level is half the one around it, so more levels are finer levels in the middle. A level's cells double the
  cells a level edge holds, so a cell covers half the angle it did at every distance (20 to 37 pixels at 1080p on
  High).
- **The finest level drawn is the first as wide as the view's standoff** (`Clipmap::detail`, `geometry::standoff`): the
  eye's distance from the water at the clipmap's centre, less the significant wave height, over the lens's
  magnification. A level narrower than that holds cells smaller than the levels beyond it show on screen, so a
  periscope or a lens at the waterline draws all nine levels (25 cm cells within 16 m), a bridge 25 m up and the chase
  camera start at the level 32 m wide (50 cm; the chase camera's own hull, 150 to 250 m off, rides 2 to 4 m cells
  where she rode 4 to 8), and a view from 1,000 m up starts at the level 1,024 m wide (16 m).
  The mesh holds every level's middle and its ring apart (`ClipmapMesh::fills`, `rings`): a frame draws the first
  level's middle, then the rings from it out and the horizon, two draws. Under water the same holds by depth.
- **The hand-over never shows**: over the second half of the way to the next level the finest level drawn morphs as a
  whole onto the next one's lattice (`Surface.detail`, `receding` in `surface.wgsl`'s vertex stage, the geomorph the
  levels' outer edges already use), so by the standoff that drops it, it is the middle of the level that takes over,
  vertex for vertex.
- **Binoculars** bring far water as near as its distance over the magnification: 8× on a hull 2 km off is a standoff of
  250 m, the level 256 m wide, 4 m cells about her on High where she had 4 m before on the coarser lattice (8 m a
  level out), 2 m on Ultra.
- **The bow waves' vertex crest** flattens by the mesh as drawn: `Wake.bow.w` is the cell of the finest level drawn
  (doubling as it morphs, so it does not step when a level is dropped), set each frame with the mesh's centre, and
  `Wake.field.w` how fast the cells grow with distance from the centre (two over the cells a level edge holds); the
  hulls' wet band, the underwater probe and the rain's splashes read the same crest (`wake_height`).

Triangles on High: about 230,000 from a periscope, 180,000 from the chase camera, 107,000 from 420 m up, against
39,680 always. `shaders/surface.wgsl` takes the frames of up to ten levels (`MAX_CLIPMAP_LEVELS`).

**Cost** (Apple M5 Pro, Metal, 2560 × 1440, High at its 70 % scale, `water-cost.ts` back to back on master's shaders
and these in one worktree, 120 frames a view, load average 9 to 19 with other sessions' builds and clients on the GPU):
the water and sky pass

| View | Before | After |
| --- | --- | --- |
| chase | 2.97 ms | 3.32 |
| bridge | 2.33 | 2.55 |
| low | 2.08 | 2.40 |
| close | 2.86 | 3.14 |
| overhead | 3.29 | 3.44 |

The same mesh with every level collapsed onto the next (its vertices, half its triangles on screen; the morph forced
to 1 through hot reload) was 0.1 to 0.2 ms cheaper in the bridge, low and overhead views and the same in the chase
view (two rounds each at load 40 to 50): about half the added cost is the vertices and half the smaller triangles'
shading. Ultra's 256 cells a level were measured only at load 25 to 50, where runs differed by a factor of two; the
low views came out 0.5 to 1 ms dearer of 5 to 9. Measure it on a quiet machine before changing its count.

**Left open.** The rings are drawn whole: most of their vertices are behind the eye or beside the view. Drawing each
ring in blocks culled against the view would take back the vertices' share (0.1 to 0.2 ms on High, more on Ultra). The
far levels' cells are slivers a few pixels tall at grazing views; collapsing levels beyond the range where a wave's
height is a pixel would take back some of the shading's.

Stills, before and after, plain and with the mesh's cells drawn over the sea: `.build/native/water/`
(`stills.sh <label>`; `wire.ts on|off` patches the cells' edges into `surface.wgsl` through hot reload, coloured by
level, and takes them off again).

## The clipmap follows a magnified view

The clipmap's cells grow with distance from its centre, which matches a pixel's footprint only at the normal 52° lens.
Through binoculars a hull 2 km off fills as many pixels as one 250 m off at 8×, but she rode 16–32 m cells (of the
mesh as it was then: "The clipmap and the view's standoff" above), and the bow
waves' vertex crest (`wake.wgsl`, `finest`) flattened to suit them: a 2 m crest drew about 0.4 m, and the bow spray,
which rises from the full crest, stood over flat water. `geometry::focus` now moves the centre from the eye (up to
1.25×) to what the view is aimed at (from 2×): the coupled hull nearest the middle of the view (`set_hull_sea`'s
footprints, within 1.2 half-heights of it), or else where the view ray meets the sea, at most 12 km off. The surface's
geomorph and the bow waves' vertex filter measure from that centre (`Surface.clipmap.zw`, `Wake.torpedo.zw`), so the
hull heights and the underwater probe read the sea as drawn. At the normal lens the frame is byte-identical. Since
2026-10-12 the hull heights also read the sea at the mesh's spacing where the hull stands (`wake.wgsl`'s
`mesh_spacing`, `waves.wgsl`'s `drawn_height_at`): a hull past the focus's 12 km, or off the lattice on the horizon
ring, stands in water drawn flat, and with the wave field's full heights in her patch the plating between the two was
lit as under water, black strips along the waterline of a ship 9 nm off through binoculars. The water
between the eye and the focus, at the bottom of a narrow view, takes coarser cells than before; its normals still
carry the short waves. A hull change under the sight moves the centre at once (no easing). Since 2026-10-12 a magnified view from 10 m or
more above the surface draws the fan above instead, and this centring is left to the views that keep the clipmap. An eye within 3–10 m of the highest
the surface reaches (a periscope, a low free camera) keeps the centre, so the water beside it is the sea the underwater
probe reads.

## A magnified view draws a fan

Moving the clipmap's centre to what a lens is aimed at (the next section) makes the water fine about one point. A lens
high over the sea shows a strip of it kilometres long: at 8× from a foretop 35 m up, from 600 m off to the horizon.
All of that strip but the patch at the aimed hull lay on cells hundreds of metres across, the horizon ring's among
them, where every wave is faded out of the mesh and only shaded: through binoculars the sea was a flat sheet with a
detailed patch at the ship in the sight, and a hull past the focus's 12 km had not even that.

A view magnified 1.25× or more, from an eye 10 m or more above the highest the surface reaches, now draws the sea with
a second mesh in the clipmap's place (`geometry::fan`, `Fan`): a polar lattice about the eye over the wedge of sea
between the frame's edges, from where its steepest line of sight meets the water to where its shallowest does, or
to the sea's horizon and a crest beyond.

- **Cells are the pixels across that the normal lens's are.** Columns stand a step of bearing apart: 1.25 of the least
  a clipmap cell is of its range (two over the cells a level edge holds), over the magnification. On High at 8× that
  is 1/410 of the range: 1.3 m where the frame's lower edge meets the sea, 24 m at 10 km, where the clipmap about the
  eye has 312 m. A coarser tier's cells are as much larger as its lattice's.
- **Rows are square to the knee and a set height on screen beyond it.** Within the knee each row is e^step times the
  range of the one before, so a cell is square. The sea at a grazing angle is foreshortened (a square cell 10 km off at
  8× is a twelfth of a pixel tall), and triangles thinner than a pixel cost their pixels several times over: each
  shades the whole of every two-by-two block it touches. Past the range where a square cell would stand under a pixel
  and a half tall (on High; three pixels on Medium: `FAN_ROW_PIXELS`), rows stand that far apart down the screen
  instead (one over a row's range falls by `Fan::squeeze` a row). From 35 m the knee lies about 500 m off at any lens.
- **The waves are read at the longer of a cell's sides** (`Fan::spacing`), as a level's are at its cell, so the rows
  beyond the knee carry the waves no shorter than they are deep: at 8× full relief over the lower three fifths of the
  sea in the frame, the long swell alone toward the horizon. `wake.wgsl`'s `mesh_spacing` and the bow waves' vertex
  crest read the same spacing (`wake.shading[3].w` set, `wake.field.w` the step, `wake.bow.w` the squeeze), so the
  hull heights' patches hold the sea as drawn under the fan as under the clipmap.
- **Turning or tilting the view moves no vertex.** Columns stand on whole steps of bearing (a whole number of them
  to the turn, so there is no seam where the heading wraps) and rows on whole rows from the knee, so a turn or a tilt
  adds and drops cells at the frame's edges. The lens and the eye's height reach the
  lattice a notch at a time (an eighth of a doubling), so neither's drift moves it. The lattice does travel with the
  eye: a ship under way slides it through the waves, which the waves' own filter (nothing shorter than two cells) keeps
  from showing; a frozen sea seen from an eye moved 1.25 m changes a quarter more under the fan than under the old flat
  mesh, most of either change the parallax of the shaded waves.
- **One block, instanced.** `geometry::build_fan` holds 512 columns of 32 rows of cells, a vertex's position its
  (column, row) and its level −1; `surface.wgsl`'s vertex stage places it from `Surface.fan` and `Surface.detail.zw`.
  The indices run column by column, so a draw takes the columns the view needs and stacks an instance per 32 rows,
  nearest first. The last row stands where the fan ends (`Fan::far`), whatever its place in the lattice: a coarse
  tier's rows are deeper than the sea's horizon dips, and the lattice alone would stop short of it. Water beyond the
  far plane is drawn along its line of sight just inside it, as the horizon ring's is.
- **What keeps the clipmap**: the normal lens (its frames are unchanged), an eye within 10 m of the surface (a
  periscope, a low free camera: the water at the lens is the sea the underwater probe reads), a view whose forward
  lies less than half along the sea (past 60° down) or whose lowest line of sight looks more than 70° down (the water
  lies all round the point under the eye), and one wider than a half turn. Those still centre the clipmap as the next
  section has it.

**Cost.** Apple M5 Pro, 2026-10-12, High, 1728×1030, `dev-client`, the frame's median over 90 redraws of a frozen
moment (`shot.ts --settle 90`), fan and clipmap alternated in one process, three pairs each: at 8× 8.9 ms against
7.3 (+1.5 ms), at 2× 7.9 against 6.6 (+1.3), at 24× 9.6 against 8.4 (+1.2). The same eye at the normal lens: 8.5 to
9.0 ms. A magnified frame had been cheaper than a normal one, its mesh two coarse levels and the ring; it now costs
what the normal lens's does. With square cells to the horizon the fan added 3.6 ms at 8× (158,000 cells against
40,000); a fan drawn flat cost as much as one with its waves, so the cost is the triangles' (their vertices, and the
pixels thin ones shade twice), not the relief's. Rows 4 pixels tall add 0.7 ms and leave relief in the lower two
fifths of the sea.

Tests: `geometry.rs` (the fan covers every line of sight in the frame, turned, tilted and rolled; its lattice holds
under a turn, a tilt, a lens a hair stronger and an eye a metre higher; rows no thinner than the floor; the block
winds as the clipmap does) and `tests/surface.rs` (`a_magnified_view_s_fan_covers_the_sea_in_the_frame`: on High and
Low, water in every pixel from the horizon's row down at 2×, 8× and 24×, none in a view of the sky).

## The sea views' own lens keeps the clipmap

Bill saw the sea's look jump as he tilted the chase camera up and down: fine chop for a few tenths of a second, then
the smoother sea again, and back. Since the sea views took a 38° lens (2026-10-26, `naval_presentation::view::NORMAL_FOV`,
the Graphics menu's Field of view) every view of the sea was 1.4 times the bare eye's 52° to the ocean, past the fan's
1.25×: the chase view drew the binoculars' fan ("A magnified view draws a fan" above) from every eye 10 m or more above
the crests, the clipmap below. The fan's rows are notched by the eye's height (an eighth of a doubling a notch, so the
lattice holds still while a scope turns or tilts); an orbiting camera rises and falls as it tilts, and each notch it
crossed re-laid the mesh and the waves it carries. Now `Ocean::view_fov_y` holds the views' own lens (naval-client's
`apply_graphics` sets it from the Field of view; the bare eye's by default), and only optics narrower than the
narrower of it and the bare eye's by the fan's 1.25× draw the fan or leave the eye. Binoculars at 2× through the 38°
lens still take it (1.4×); at the 30° lens a 2× glass is 1.1× and keeps the clipmap. Measured: the fan's choice over
eyes 6–80 m and views level to 15° down, High, a 3 m crest bound: at 38° the fan from 13 m up, its rows re-laid at
23 heights; at 52° the clipmap throughout. Test: `tests/surface.rs`, `the_sea_views_own_lens_keeps_the_clipmap`.
