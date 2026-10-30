# Combat effects: the renderer

How `EffectsLayer` draws: its passes and draw order, the flipbook look and its switches, merged smoke, and the effects'
part in the sun's shadows. The gas atlas is deterministic: naval-assets bakes it at pack time and naval-client uploads
it (`EffectsLayer::with_atlas`, `GasAtlas::from_baked`), baking only when the pack has none (`native/docs/assets.md`,
"Gas atlas"). The web port's parity measurements and its first cost table are in the
[history](../../../docs/archive/native-combat-effects-history.md).

## Renderer core

```rust
let mut effects = EffectsLayer::new(&mut renderer.context())?;     // bakes the atlas (submitted, not waited on)
effects.fog = EffectFog { /* the ocean's fog */ .. };
effects.effects.lighting.set_illumination(color, intensity, ambient);
// each frame, before rendering:
effects.effects.update(dt, now, &CameraView::from_uniforms(&camera_uniforms), &events, &ships);
renderer.render(input, &mut [&mut ocean, &mut ships, &mut sky, &mut effects], output)?;
```

- The sky's light on smoke, gas, fires, debris and shell bodies is the sky's environment (`sky_fill` in `common.wgsl`:
  its radiance from above × 0.677, the albedo the direct term implies), as the ships take it, so effects dim with dusk,
  night and overcast; `lighting.ambient` is only the stand-in where the environment is not bound, and `lighting.flash`
  lifts the sky's light with lightning. A tracer's smoke, drawn in its daylight colour, takes `scene_light` (the
  sky's and the sun's or moon's light against a clear noon's, at most 1); its spark, streak and burning base are their
  own light. `ShellLayer::follow` takes the environment from the effects layer and binds it as group 3. The client sets the light, its direction and the daylight
  every frame (`set_flash`), so a live sun (the theatre's clock) moves the effects' light too.
- `prepare` publishes the simulation for the frame's own camera (cull, sort, pack), uploads instances to storage
  buffers (one quad of six vertices per instance), draws the occlusion map (quarter resolution, two `Rgba16Float`
  targets, max and additive blending) from the occluding gas as the web does before its scene pass, and refreshes the
  soft-particle depth.
- `draw` draws the strike debris in `Phase::Opaque` (solid shards writing depth: the sea hides what has sunk, and the
  smoke drawn after sorts against them), and the rest in `Phase::Transparent`, in the web's order (three sorts these meshes by render order, then creation):
  combat fire, combat gas, fire-smoke columns, soot, funnel ribbon, funnel billows, flame bodies, flames, embers. Groups: 0 the camera (with the sun's shadow maps), 1 the lighting, 2 the batch, 3 the scene
  reads (occlusion map, depth). `draw_shadow` draws the debris and the dense smoke into each sun map ("Sun shadows").
- **Behind the clouds** (2026-10-03): the clouds' composite draws in `Phase::Sky`, before the transparents, so smoke
  drawn after it showed through a cloud standing in front of it (a funnel's ribbon over a cumulus deck seen from
  above). The client hands the layer the clouds' published cover (`set_cloud_cover`, `naval_sky::CloudCover`: the
  reconstruction's two histories, which one the composite drew this frame, the lowest layer's bottom and the highest's
  top). `smoke_visibility` (every smoke, fire and glow material) multiplies in `cloud_visibility`: the share of the
  ray's run through the cloud heights that lies in front of the fragment takes that share of the pixel's cloud opacity.
  Smoke under the clouds seen from under them is unchanged. The cover binds at scene reads binding 7; the lighting's
  `clouds` holds the heights.
- **Behind the splash columns** (2026-10-25): the water effects' column composite draws before this layer, and the
  scene's depth does not hold the columns, so a burning ship's smoke behind a column fell before her drew over it. The
  client hands the layer the columns' published cover (`set_splash_cover`, `WaterEffects::cover`, a
  `naval_render::Cover`: their opacity and the depth it lies at, `../water-effects.md`, "The cover for the smoke"), read at
  the backdrop's copy point after the march and bound at scene reads bindings 8 and 9. `splashes_through` (in
  `smoke_visibility`, the merged gas and the flashes' and flames' sprites) leaves a fragment behind that depth what the
  column leaves of the scene, passing over 12 m and a hundredth of the distance. Look `splash-cover` (on;
  `-splash-cover` for the look before). GPU test `smoke_behind_a_splash_column_is_hidden_and_smoke_before_it_is_not`;
  harness scene `splash-smoke`.
- **Soft-particle depth sits behind a seam** (`DepthSource`). Until render core II is on master the layer copies
  `scene.depth` in `prepare`, which then holds the previous frame: right for a still camera (the stills render each
  shot twice), one frame late for a moving one. **Switching to render core II's scene copies**: return
  `SceneCopies::BACKDROP_DEPTH` from `scene_copies()`, bind `scene.backdrop.depth` (`texture_depth_2d`, the same binding
  type) in the scene-reads group and rebuild it when its `version` changes, and delete the copy pass and `OwnCopy`. The
  occlusion map stays in `prepare` (the web draws it without depth). The client reads `Backdrop` (`src/scene/effects.rs`); the
  stills and trials still use `OwnCopy`. Since 2026-10-09 the layer makes its own full-size `Depth32Float` copy only
  for `OwnCopy` (25 MB at 3840 × 1600), and binds the far plane until the backdrop copy is bound.
- Scene pipelines are built through `RenderContext::add_scene_pipeline`, so group 0 comes from `ctx.camera_layout`
  and render core II's shadow bindings will not disturb them. The occlusion pass runs outside the scene pass and binds a
  camera uniform of its own at group 0. Every pipeline goes through the registry, so the effect shaders hot-reload.

## The effects kit

The two effect crates were ported in parallel from the same web kit, and each kept a copy of it. Since 2026-10-28
`naval-water-effects` depends on this crate and draws from one kit here:

- `particles.rs`: the one `ParticlePool` (`EffectParticle`, claim, emit, thinning, the analytic step, culls, sort). The
  water's drops and billows are pools of kind `PoolKind::Water { fine, offscreen }`, published by
  `publish_water_into` as `instances::WaterSprite`, the quads `naval-water-effects`' `sprite.wgsl` draws. Where the two
  copies differed, the kind keeps each crate's behaviour: a water particle takes the wind as given (not
  `combat::wind_share` at its height) and rolls no warp, and is culled by its own radius and a pixel (`fine`, drops) or
  its bounding sphere (`offscreen`, billows); its fade, quad and seed are the water copy's, term for term.
- `camera.rs`: one `CameraView`, two constructors. The combat effects read the renderer's uniform
  (`from_uniforms`); the water effects place theirs in `f64` from the renderer's camera (`from_render`, `look_at`),
  as their copy did, so neither crate's culls move. `rotation`, `sees_sphere` and `pixel_angle` came with the water's.
- `textures.rs`: `SpriteMap::Droplet` and `SpriteMap::Smoke` (the spray's maps, light in RGB) beside the additive
  ones, and the one `smoke_billow_texture`; the merged maps are the water copy's bytes, every texel of every mip.
- `random.rs`: `Lcg` is the water's stream too (`Lcg::for_event`, the web's `randomFor(sequence × 7919 + shell)`).

Not merged: the two fixtures record different things (here ship states and combat events at simulation ticks; the
water's hull sizes, poses, torpedoes, water events and the sea at 10 Hz) and share no code, so one format is a design
of its own, left for a later slice. The water's `splash::hex` keeps its own `srgb_to_linear` constants (more digits
than `math.rs`'s), so the splashes' colour does not move by a rounding.

Before and after the merge, the client drew a salvo's splashes and columns, a burning ship, a destroyer's wake, bow
spray and funnel smoke, and shell and torpedo strikes close up with the same pixels
(`.build/structure-w3/effects/`; the PR has the numbers).

## The flipbook look

On by default since 2026-10-04 (the owner's pick from four trial looks, `bun native/tools/effects-trial.ts`). Gas
puffs play frames of real fluid simulations, JangaFX's free EmberGen VDB sets (CC0), marched through the gas atlas's
own six-way lighting and blended between frames along their motion vectors: muzzle blasts, HE bursts, shell strikes and
the magazine detonation (`flipbook_blasts.rs`), a burning ship's flames (`flipbook_fires.rs`) and her smoke columns, a
chain of plume links (`fire_batch.rs`, its own lean `link_pipeline`). Runtime in `src/flipbook.rs` and
`shaders/flipbook.wgsl`, with hooks in `gas_sprite.wgsl`, `particles.rs` and the gas material's bindings 30–34
(`layer.rs`). With the flipbook and tactical billows the gas material's fragment stage binds 15 sampled textures, within WebGPU's 16; all
four looks together bind 15 there (the smoke shadows' two are read in the vertex stage since 2026-10-04); the client
still asks for up to 24 (`trial::limits`).

**Switches** (`naval_effects::Trial`, the client's `NAVAL_EFFECTS_TRIAL`, a comma list changing the game's look; an
unknown name prints one warning and keeps the game's):

| Name | Effect |
| --- | --- |
| `-flipbook` | the effects from before the flipbook look (the atlas's still puffs, the flame tongues) |
| `-views` | the flipbook's smoke from one side only, each puff a card facing the lens that turns with it (the look before 2026-10-13, "Seen from its sides" below) |
| `-merge` | the smoke's puffs layered one over another, back to front by their middles (the look before 2026-10-14, "Merged, not layered" below) |
| `-shadows` | no smoke shadows: neither the smoke's own shade from the smoke between it and the sun nor its shadow on the ships and the sea (on in the game since 2026-10-04, see "Sun shadows") |
| `-shock` | no blast waves (on in the game since 2026-10-12, see [blast waves](gun-blasts.md#blast-waves)) |
| `-funnel-puffs` | the funnels' billows and ribbon in place of the flipbook's puffs (the look before 2026-10-20, [funnel smoke](fire-and-smoke.md#funnel-smoke)) |
| `-column-cover` | a fire's flames, bodies and embers shine through all of its column's smoke in front of them (the look before 2026-10-24, [flames and their light](fire-and-smoke.md#flames-and-their-light)) |
| `-splash-cover` | the smoke, fire and glows draw over the splash columns in front of them (the look before 2026-10-25, "Behind the splash columns" above) |
| `bright-flames` | after dark a fire's flames a stop brighter, their light on the scene the same (2026-10-25, for Bill to pick: [at night](fire-and-smoke.md#at-night)) |
| `funnel-plume` | funnel exhaust as one chain of plume links from the rim (`funnel.rs`, `draw_links`) in place of the puffs (it wins over them); not chosen, off |
| `warp`, `volume` | the other trial looks (puffs rolled along a flow field, ray-marched volumes for the largest bodies); not chosen, off |

A leading `-` switches any look off; the last word on a look wins. `bun native/tools/effects-trial.ts --trial=-flipbook`
(or any list) films the same moments with the list, so a look can be compared against the game's.

**The books** are five (`blast`, `fireball`, `fireball-side`, `fire`, `plume`; 249 frames of 256² light and 128² motion,
147 MB raw) in `assets/effects/flipbook/`, stored in **Git LFS**. Their frames are the layers of one array texture,
which may hold 256 (`MAX_LAYERS`). The pack carries them with their light BC7 (`../assets.md`,
"Flipbooks": 170 MB of texture memory becomes 57 MB). The client takes them from `NAVAL_EFFECTS_FLIPBOOK_DIR` when set,
else the pack's, else the checkout's (`Assets::flipbooks`); with none, or a Git LFS pointer in place of a book, it warns
once (`[effects] flipbook: …`) and draws as with `-flipbook`. A host outside the game reads the five itself
(`flipbook::GAME_BOOKS`, each `FlipbookData::from_bytes`) and hands them to `EffectsLayer::set_game_look`, which refuses
them with a book missing and otherwise sets `Trial::GAME`: a layer is made with every look off (`Trial::default`), the
effects from before the flipbook look, funnels' billows and ribbon included (engine-example's `game_books`). They are read and uploaded once a process, by the first
scene's effects, and every later scene's (each port, battle and theatre scene builds its own renderer) shares them, as
it shares the pack's gas atlas (`effects.rs`' `FLIPBOOKS` and `GAS_ATLAS`, `naval_render::DeviceCache`; 2026-10-09):
a theatre holding two scenes while it swaps them holds one copy, and a scene after the first spends nothing on them.
Loading them (the `flipbooks` start-up phase) took 307–394 ms from the pack (one thread a book) against
0.9–2.1 s from the raw files and 1.3–1.9 s in the trials build, `shot --frames 1`, three runs each, alternating (Apple
M5 Pro, 2026-10-04, load average 33 from other agents' builds). The pack's stills match the raw files' within a mean
0.04–0.09 levels (fire, flames, muzzle, magazine; at most 0.08 % of pixels off by more than 8).

**Rebaking**: `python3 native/tools/effects-flipbook/bake.py [--only blast] [--force]` downloads the VDB sets (about 7.5
GB, into `.build/effects-trial/vdb/`), marches them in Blender (`march.py`), packs them (`pack.py`, numpy and OpenCV in a
venv it makes) and writes `assets/effects/flipbook/<name>.flipbook`. Commit those (LFS) and rebuild the pack; a book's
tile, frame count and clips are read by `flipbook.rs` (`CLIPS`, `PLUME_ASPECT`, `PLUME_CLIMB`, `FIRE_LOOP_SECONDS`).
`fireball-side` is framed by the fireball's own march: bake them together (`--only fireball,fireball-side`). Since
2026-10 MediaFire answers the archive's page with the archive itself; `fetch` takes either. A march of the same
frames is deterministic: on 2026-10-13 a fresh march of the Mid-Air Explosion packed to a `fireball.flipbook` byte for
byte the one committed.

### Seen from its sides

A puff is a card facing the lens, and a card shows one side of its simulation however it is seen: circled, a puff of
smoke kept its outline and turned with the eye, and a gun's smoke bank read as a stack of flipbook cards. The
fireball's smoke is now baked from a second side too. `fireball-side` is the Mid-Air Explosion marched from +X
(`march.py --view 1`, ninety degrees round to the right of the fireball's eye), holding the fireball book's last 33
frames (the books fill 249 of the 256 layers). The smoke clip plays only frames it holds, and a battleship's fireball lobes reach
them about 0.9 s after the shot. It is framed as the fireball's own frames are (`pack.py --framed-like`): the same height and the same
texels a voxel up the tile, centred across on its own gas. That simulation is a fireball blown along its way, round
seen along it and up to twice as wide from the side, so the side's tile is as wide as its gas reaches, 1 to 1.5 times
the fireball's (`WIDEN_MAX`; what lies further is 1.45 % of its gas at most, the thin ends of its streaks), and the
book stores that width in place of each frame's scale.

With the look on (`lighting.trial.y` 2, `Trial::views`; `-views` is the card), a puff playing those frames
(`flipbook::SIDE_BOOKS`, the clip table's `views` row) stands upright with a seeded heading, and the shader
(`flipbook_views` in `flipbook.wgsl`) finds the eye's bearing on it round from that heading:

- **Four sides from two books.** Seen from behind, gas has the outline it has from in front, mirrored, so a quarter
  turn round shows the side book, a half turn the fireball read right to left, three quarters the side book right to
  left. Each read right to left swaps its light from ±X. A puff mirrored by its seed is the simulation mirrored: its
  sides come round the other way.
- **Two sides blended through the middle of each quarter.** Each side shows alone over 0.6 of a quarter turn about its
  own bearing (`VIEWS_ALONE` 0.3), and the two nearest are blended over the 0.4 between: their optical depths and flames by their
  shares, their light (baked per unit of what the gas stops) by what each stops, so a side with no gas at a texel does
  not dim the other's. Each side reads its frames along their motion: a side read without it changed its outline as it
  took over from the other (found in review). Blended all the way round, two outlines showed through each other at every
  bearing.
- **The quad drawn wider for a side.** While a puff shows the side book at all, its quad is drawn as wide as the side's
  tile (`flipbook_widen`); the fireball's reads stay where they were and read nothing beyond its tile.
- **A cross-fade in, not a jump.** The fireball clip reaches the side book's frames while its flame is going out: over
  the first four of them (`VIEWS_RAMP`, about 0.35 s for a battleship's lobes) the fireball's own side fades into the
  sides nearest the eye's bearing, a third side read while it fades when it is not one of those two. It is a fade
  because no turn of the bearing can take every bearing to the fireball's own side without a jump somewhere round the
  puff: easing the bearing toward it flipped the smoke between two sides seen from behind (found in review).

Puffs with a side book are upright: the seeded turn and its slow roll as they age (which keep cards from repeating) are
gone for them, since a turn of the card would have to be a lean of the gas, foreshortened from the side and mirrored
from behind. A fire's column billows (the smoke clip on the links' path) were upright already.

**Cost** (`effects-trial.ts --only muzzle,muzzle-far --cost 60`, 1280×720, dev-client, Apple M5 Pro, 2026-10-13,
three alternating pairs, the least of each moment's three, load average 22–48 from other sessions): from 80 m, with a
38 cm turret's smoke filling the frame from 1.2 s to 7.5 s, the transparent pass takes 0.3–0.45 ms more than with
`-views` on the same build (2.6–2.8 ms against 2.1–2.5); from 1 km nothing measurable. Where a puff covers nothing its
fragment now returns before its light is worked out (`fragment`, exact under the gas's alpha blend: the still puff's
implicit-derivative read stays above the return), and that pays for the look: the pass measured 2.2–2.9 ms at those
moments on master's shaders. What the look costs is mostly the quad's wider margin (drawn no wider it cost about 0.1 ms)
and the second side's reads through the blend. The sides are fields of `FlipFrame`, not arrays: written as arrays
indexed in a loop the look cost 1.3–1.8 ms more, the shader compiler keeping them out of registers.

**Checked by** `tests/gpu.rs`, `a_smoke_puff_turns_its_sides_as_the_eye_goes_round_it`: one puff of smoke, still, seen
level at each eighth of a turn round it. With the look, a quarter turn round its outline overlaps the first by under
0.8 and half a turn round the first mirrored by over 0.95; as a card, a quarter turn round overlaps itself by over 0.95.
`NAVAL_EFFECTS_TEST_PNG=<dir>` writes the frames. In the game: `bun native/tools/effects-trial.ts --only muzzle-orbit`
holds the battle 5 s after a 38 cm turret fires and takes the camera once round its smoke at 190 m in 12 s, each frame
held (`frame_draw`'s `held`: the clock stands) so the gas stands still (`--trial=-views` for the card).

**Open**: from steeply above a puff still shows a side (no top view is baked; from overhead its sides come round as fast
as the eye's bearing on it does); the blast (a shell's burst on plating, the magazine) and the fires' flames and columns
are one side each still (a blast stands on its foot, so a side book of it would need its own framing on the ground).

### Merged, not layered

Layered, each puff is blended over the scene in turn, back to front by its middle's distance (`particles.rs`'
`visible`). Where two big puffs overlap, the nearer one's whole outline lies over the other, and when the eye or the
smoke moves enough to change which middle is nearer, the overlap turns from one puff to the other in one frame: a
puff deep in a gun's bank, dark in the bank's own shadow, jumped out over the lit ones with a hard outline. Held 5 s
after a 38 cm turret fires and circled (`effects-trial.ts --only muzzle-orbit`), the bank did it every few frames.

Merged (`Trial::merge`; `-merge` layers them), the gas batch (guns' and shells' smoke, flak, the aircraft's trails, smoke
screens) and the soot are added up in a pass of their own at the backdrop's copy point (`EffectsLayer::after_copies`; this frame's opaque and sea depth taken,
tested against the scene's depth as the layered draws are) into two targets the size of the scene: the sum of each
fragment's light times its opacity and of its opacity, each weighed by what the gas in front of it lets through, and
the product of what each leaves of the scene (`merged_smoke` in `common.wgsl`; Rgba16Float and R16Float; an AA burst's
soot is weighed by the map's levels only, not by its optical depth's spread, since 2026-10-26:
[Heavy AA bursts](gun-blasts.md#heavy-aa-bursts)). Where the
gas batch draws, `merge.wgsl` lays their weighed mean light over the scene with the opacity they have together, as one
puff. Sums need no order: two puffs trading places change the picture as their weights change, gradually. The weight is
the occlusion map's estimate of the gas in front (`smoke_through`, what a layered link or a fire reads: its four
levels' fronts and its optical depth spread from the nearest front to behind its middle), so the nearer smoke shows
most, never less than 1/256 of a fragment's opacity (`MERGE_LEAST`). A merged fragment is not thinned by the gas in
front as a layered one is (the weight does that), only by the clouds and the splash columns in front (`clouds_through`,
`splashes_through`).

It merges only where the backdrop's depth is read (`DepthSource::Backdrop`, the client's); elsewhere (the crate's own
tests unless they ask for it) the smoke is layered. The funnels' puffs (since 2026-10-20, a batch of their own, "funnel
smoke") merge with the gas and the soot. The fires' columns and the funnels' links, the fire bodies and
flames, the fire smoke's quads, the funnels' billows and ribbon (`-funnel-puffs`) stay layered, and draw over the merged smoke as they
drew over the gas. The columns were merged in the first trial, but they are not in the occlusion map, so nothing told
their links which lay in front: merged, every link counted alike, and from 450 m off a burning ship's beam (`column`)
her black column and the white ones beside it averaged into one hazier grey where layered the black stays crisp.
Merging them needs their own fronts and optical depth (since 2026-10-24 the occlusion map's two free `depths` channels
hold the columns' optical depth and its nearness, which the flames drawn after them read: [flames and their light](fire-and-smoke.md#flames-and-their-light)). The targets are made on the
first frame with smoke to merge and kept: 10 bytes a pixel (9 MB at 1280×720, 83 MB at 3840×2160).

**Cost** (`effects-trial.ts --only muzzle,muzzle-far --cost 60`, 1280×720, dev-client, Apple M5 Pro, 2026-10-14,
three alternating pairs, the least of each moment's three, other sessions building): with a 38 cm turret's smoke filling
the frame from 80 m (1.2 s to 7.5 s after the shot), the merged pass and the transparent pass together took 2.05–2.79 ms
against the layered transparent pass's 2.11–2.93 (the merged pass 1.73–2.50 of it, the composite and the rest 0.24–0.32);
from 1 km nothing measurable either way. The whole frame was 7.8–8.4 ms merged and 7.9–8.8 layered at those moments.

**Checked by** `tests/gpu.rs`, `merged_smoke_does_not_jump_when_two_puffs_swap_places`: a grey puff and a light one 8 m apart,
overlapping on the screen, seen from 80 m with the camera 0.25 m to one side of the line where they lie equally far and
then 0.25 m to the other: layered the step changed the smoke's pixels by a mean 42 levels in 255, merged by 0.23. In
the game, `effects-trial.ts --only muzzle-orbit` (`--trial=-merge` for the layered): circled, the layered bank's worst
one-frame jumps changed about 20,000 pixels, the merged bank's about 4,700 (the ship and the sea's glints passing).

## Sun shadows

Since 2026-10-04. Until then a funnel plume or a fire's smoke crossing a hull's shadow stayed sunlit, smoke shaded no
other smoke (outside the `shadows` trial), and no smoke, debris or shell shaded anything. Code: `src/shadow.rs` and
`shaders/shadow.wgsl`, `shaders/shadow_cast.wgsl` (the smoke's opacity map), `src/sun_casters.rs` (debris casters),
`puff_sunlight` and `surface_sunlight` in `shaders/common.wgsl`; the renderer's side is `native/docs/render-core/shadows.md`, "Sun shadows"
(its soft occluders).

**The smoke's opacity map** (the `shadows` look, a trial until now; on in the game whenever the sun's shadow maps are,
that is at every Graphics shadow setting but Off; `NAVAL_EFFECTS_TRIAL=-shadows` switches it off). The deep opacity
map of `shadow.rs` (a front map and slices of optical depth behind it, along the light, over a square fitted to the
smoke in view) is drawn every frame in `prepare`. The slices are a 2D texture array with permanent attachment views;
the sampling shader interpolates between adjacent layers as the former 3D volume's filter did. This avoids wgpu
30's DX12 leak of temporary 3D-slice render-target descriptors ([Windows](../windows.md#dx12-out-of-memory-after-staying-in-port)).
The map supplies two kinds of shading:

- **The smoke shades itself.** Every smoke material reads it **per vertex** (`puff_light` in `shaders/shadow.wgsl`, from
  gas puffs and the fire columns' links in `gas_sprite.wgsl`'s `place`, fire smoke in `fire.wgsl`, funnel billows and
  the ribbon in `smoke.wgsl`), at each corner pulled 0.4 of the way in toward the puff's middle, on the plane touching
  the puff's sun side. The shares of the sun's and the sky's light interpolate over the quad. A column's far side from
  the sun and its underside darken; a puff alone is unchanged. The trial read it per fragment.
- **Everything else takes its shadow.** The same balls are drawn into the renderer's soft occluders' map (a 256²
  `Rg16Float` target with max blending for where the smoke along each texel begins and ends, an `R16Float` one adding
  its optical depth; copied into `PrepareContext::sun_occluders`). `naval::shadow::sun_shadow` and `water_sun_shadow`
  multiply in what it lets through at the receiver's depth along the light, so the ships, the sea, the aircraft and the
  land darken under the smoke with no change to their materials and no sampled texture added (a storage buffer at
  group 0: the ships' fragment stage keeps 15 of 16, the sea's 16).
- **The ships' sky under the smoke** (2026-10-20). The ships take the same map for their sky's light too
  (`naval-ships`' `ship.wgsl`, `smoke_sky`, folded into the sky's occlusion): with a high sun, depth along it is depth
  under the sky, and under the deepest smoke a surface keeps 0.45 of the sky (see [a fire close up](fire-and-smoke.md#a-fire-close-up)).

**The balls are the gas as drawn** (2026-10-08). A ball was the puff's full-grown size at its full opacity, while the
flipbook look draws a clip's frame at the share of the clip's last frame its gas fills (about half in a young blast),
and a burning fireball counted as smoke; so every heavy turret laid a dark disc on the sea around its blast, wider than
the smoke and from the instant it fired. With the flipbook look on, a puff playing a clip now casts its frame's gas
(`flipbook::ClipSizes::frame`, the table `flipbook_puff` plays: the frame's size and its foot kept where the grown puff's
stands), 0.8 of that frame's half-width (`FLIPBOOK_GAS`: each bake frames its gas with a margin and the gas is ragged
at its rim). A puff's flame casts less as its heat climbs from 0.9 to 1.9 (`GLOW_HEAT`) and nothing above, where the
fireball burns through (its own light outweighs the sun it stops), its shadow coming as it cools to smoke. Heat a burst
holds past its life (`flipbook_blasts::held_heat`) is left to the simulation and casts as smoke.

**Hulls on smoke.** `puff_sunlight` (in `puff_light`) reads the renderer's depth maps alone (`sun_shadow_maps`, so the
smoke's own shadow is not counted twice) a radius toward the light from each vertex's point, down to `SHADOWED_SMOKE`
(0.18 of the direct light: what the sunlit smoke about it scatters in). Only the direct light is scaled; the sky's light
by the smoke map's sky share; flashes, firelight and the gas's own burning are never shadowed, and flames (their own
light) ignore it. The pipelines are built with `EFFECT_SUN_SHADOW` (`scene_defs()` in `layer.rs`, and `ShellLayer`'s);
the occlusion pass, whose groups hold neither map, without it.

**Clouds on smoke** (2026-10-20). The smoke's direct light passes the clouds' shadow too, as the hulls' and the sea's
does: `puff_sunlight` multiplies in naval-clouds' `cloud_shadow` at the puff (`EFFECT_CLOUD_SHADOW`, the layer's own
pipelines: `layer_defs()`; the map, its sampler and uniforms at bindings 44 to 46 of the scene reads, read per vertex,
`EffectsLayer::set_cloud_shadow`, which the client hands the sky's clouds' shadow as it hands the ships theirs). Under a
deck none of the beam gets through, and the smoke takes the sky's light alone. The map holds the whole column's shade,
so smoke within the layers takes the share of them above it (evenly from their bottom to their top, the lighting's
`clouds`) and smoke over them, an aircraft's trail or flak above a deck, none: at a storm's dusk the funnels' plumes
had stood orange against the grey deck (the low sun's light, which the hulls under them did not get), and at a storm's
noon a pale beige. The shells' bodies and the debris keep the sun unclouded (their own sunlight is a few pixels).

**Debris and shells.** Strike debris and shell bodies take `surface_sunlight` (the renderer's `sun_shadow`: depth maps
and smoke) per fragment on their sun-lit diffuse and glint. Debris casts into the depth maps (`EffectsLayer::draw_shadow`,
`debris.wgsl`'s `caster_vertex`): every piece in flight, in view or not (`Debris::publish_casters_into`), since a piece
off the screen can shade what is on it. Shell bodies do not cast: a 38 cm round is a few texels of the near map for a frame.

**Choosing the opacity map over depth casters.** The first cut drew the dense smoke into the renderer's depth maps as a
screen door (texels kept by a per-texel threshold against the ball's opacity), which needs no receiver change. On a
level deck under a low sun its texels showed as streaks; a threshold stepped each frame for temporal anti-aliasing to
average broke the client's determinism check (`determinism-duel`); and the smoke shading itself through it double
counted with the trial's map. The opacity map gives a soft, even shadow at the map's texel (0.5–8 m) and the self-shade
from one set of passes.

**What it looks like** (a burning Iowa: `bismarck;;iowa:static`, 5 km, bearing 90, the player's helm released, 300 s,
19 fires, clear, 10:30; `bun native/tools/shot.ts --battle "bismarck;;iowa:static" --range 5000 --bearing 90 --seconds
300 --orders '{"command":{"type":"release-helm"},"direct":false}' --weather clear --hours 10.5 --ship enemy-1 --camera
effect:fire --camera 240,30,420 --camera top`, before with the master baseline): the columns' lower and far sides
darken; the decks, gunhouses and superstructure under the smoke take a soft shadow; the columns throw a broad soft
shadow on the sea beside her. Images in `.build/fx-shadows/iowa/` (`pair-before-after2-*.png`).

**Cost** (Apple M5 Pro, 2560 × 1440, the High preset, load average 42–59 from other agents' builds, 2026-10-05; the
burning Iowa above frozen, three views: close on a fire, the whole ship from 400 m, the firing Bismarck; 57 balls in the
map at 0.5 m texels; medians of the pass timestamps over three interleaved rounds of 40 frames, the shaders' parts
switched off through hot reload in one client, then master's baseline at the same moment):

| | Close on the fire | The whole ship | The Bismarck |
| --- | --- | --- | --- |
| The map's passes (front, slices, soft occluders), summed spans | 0.57 ms | 0.19 ms | 0.14 ms |
| The same at the trial's 512² × 16 slices | 1.06 ms | 0.50 ms | 0.27 ms |
| Receiving the soft occluders (opaque, sea and transparent passes) | +0.17 ms | +0.17 ms | within noise |
| The smoke's self-shade per vertex | within noise | within noise | within noise |
| GPU busy, this branch / master | 9.66 / 9.43 ms | 9.51 / 10.14 ms | 8.56 / 8.47 ms |

The busy times move by ±0.3 ms between rounds under this load, so the whole is "a few tenths of a millisecond":
cheap enough to leave on at every shadow setting. The trial's own figures (+0.9 ms on a burning ship, up to +3 ms
with gun smoke filling the frame, at 2560 × 1440) were mostly its per-fragment reads, which are now per vertex, and its
fill at 512² × 16 slices. `effects_probe` reports `smoke_shadow` (balls and texel).

**Open.** Smoke outside the fitted square (far off, small on screen beside nearer smoke) neither shades smoke nor
casts. The soft occluders' depth profile is one run per texel (the first smoke's front to the last's back), so a
receiver between two banks along the light takes a share of both. A smoke-filled near view at the gun-smoke scale of
the `o-broadside` stills was not measured separately.
