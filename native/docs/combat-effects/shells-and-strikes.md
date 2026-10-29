# Combat effects: shells and strikes

## Shells in flight

The rounds in flight (`shells.rs`, `shell_layer.rs`, `shaders/shells.wgsl`) kept the web game's look until now: every
round above the water an incandescent orange body with a gold nose, an amber halo, a white-gold streak 19 m long and
a pale trail of its last 1.25 s (a kilometre behind a heavy round), the halo and streak sized to the screen (0.3 and
0.45 % of the view's height, up to 16 and 18 m), so a round 15 km off was drawn as large on screen as one at 2 km.
That was not what was seen at sea:

- **Nothing glows from heat.** At 800 m/s the air at a shell's nose reaches under 400 °C (stagnation at Mach 2.4),
  short of the 525 °C at which steel first shows red, and it warms a skin for seconds, not the round.
- **Heavy rounds carried no tracer.** A tracer would have had to burn through a 20–40 s flight to mark the fall; navies
  dyed their splashes instead (about a pound and a half of dye in an Iowa's AP round) and spotters watched the
  splashes, not the shells. Lighter guns' rounds did carry them for night fighting: the Royal Navy's HE "night
  tracer", the "red pencil-lines of the shells arching through the sky" a correspondent saw over Guadalcanal.
- **A tracer is a point of light.** Its burning composition (strontium red) is a few centimetres across. It reads as a
  line only in a long exposure or the eye's persistence, it dims with the square of its distance, and its only trail
  is a thin smoke.

Now:

| | Rounds of 20 cm and over (heavy cruisers', battleships') | Rounds under 20 cm (`TRACER_CALIBER`: destroyers', light cruisers', secondaries') |
| --- | --- | --- |
| Body | The lathe, cold steel lit by the scene: a dark speck, seen only close (the follow camera) | The same, its tracer burning in the cup in its base (`#ff5a24` × 30) |
| Spark | None | A camera-facing glow at the base, three calibres across, `#ff4a1c` × 510 at its centre falling off as exp(−12 r²) (computed in the shader: at that intensity a texture's last 8-bit step was still bright, a hard rim seen close), drawn at least `MIN_SPARK_PX` (3) wide with its light spread over that: it fills its pixels out to about 3 km and dims with the square of its distance beyond |
| Streak | None | The spark's light across one 60 Hz frame (`STREAK_EXPOSURE`: 13 m at 800 m/s, was 19), 0.6 calibres wide, `#ff6a30` × 20; inside 65–150 m it shrinks, as before |
| Smoke | None | 0.6 s of it (was 1.25 s of vapour), a calibre wide at the round and spreading 10 calibres a second, opacity 0.35 thinning as it spreads, in the scene's light (replaced by the trail, 2026-10-17) |
| Marker (an aid to play) | On the heavy rounds of the ship the local seat commands (`ShellState::helm`, the packet's shooter against its helm): a dot at the round, `max(2 px, 0.18 % of the view height)` across, `#ff8c1a` × 0.5 painted over the scene (dark enough to keep its colour through the display transform, so it shows against white cloud as on the sea), 30 % of that at night, giving way to the round once it is 6–12 px long on screen | None: the tracer marks it |

Every size is the world's, but the marker's, which is the one departure from what was seen: without it a player saw
nothing of a heavy salvo between the flash and the splashes, where a fire-control team at sea had its own watch on the
shot. Below `MIN_LINE_PX` a streak or smoke is drawn 2 px wide with its opacity scaled by the width it gained, as
before; the screen-sized floors are gone. The smoke no longer writes depth or takes an alpha test (the spark and streak
still do): kilometres off it is a few percent opaque, its depth cut the clouds and the haze behind it into grey lines,
and the test dropped the half-covered pixel rows of a faint line. The split is by calibre alone: a team view hides a
round's type (AP or HE). The shutter's moving bodies follow (`naval-client` `shutter.rs`): each airborne round's body,
and a tracer's spark and streak. `ShellState::luminous` became `airborne` (above the water) and `tracer()`;
`effects_probe` reports both. `tests/shells.rs` checks a tracer salvo's red sparks and smoke, a heavy salvo with nothing
that glows, a tracer's burning base, a heavy round's cold steel, a sub-pixel smoke line without gaps and the player's
heavy salvo's markers.

**Trails back (2026-10-17).** The owner found that a salvo could no longer be seen in flight: a round and its tracer
are a few centimetres of light or steel, and a screen has far fewer pixels than the eye, so between the flash and the
splashes a player saw nothing, whatever was true at sea. Every round above the water, of every calibre, leaves a short
trail of the web game's pale vapour (`#e2ddd0` in the scene's light, so dimmer at dusk and night): its last 0.6 s
(`LIFE`, about 500 m behind a heavy round; the web's was 1.25 s), sampled every 0.1 s and at a ricochet's corner,
opacity 0.4 · √(calibre / 0.38) at the round (`TRAIL_OPACITY`; the web's 0.48) fading over its life, a Gaussian
across. Its width is the screen's, not the world's: 0.32 % of the view's height (`TRAIL_VIEW`, 3.5 px at 1080p, the
web's) · √(calibre / 0.38), and never narrower than the round. The web capped it at 10 m wide, which thinned a trail
past 3 km to a faint hair; the cap is gone, so a battleship's salvo 20 km off draws as wide a line as one at 2 km,
faded only by the air's haze. The owner looked at stills of the web's 1.25 s and of a wider, denser 1.25 s line
(0.45 %, 0.65) and chose a shorter, more subtle one. It writes no depth, as the smoke did not. The rest stands: no glow from heat, tracers only under 20 cm
(now with the trail in place of their smoke), the helm's markers at the head of their heavy rounds. `tests/shells.rs`
checks a heavy salvo's trails and that a trail far off keeps its pixels.

## Shell strikes and debris

Not a port: the web's strikes threw additive sparks and the same sooty fireball jets for every kind of hit, drew an
armour-piercing round's delayed burst as an open-air HE fireball wherever it was (inside the hull, where it poked out
through the deck and plating), and lit the hull white for a frame. Reviewed against a close Bismarck duel in the real
client (below); the photographs and gun-camera films of WWII hits are the reference, not the web.

**What a round did.** `BlastEffects::impact` remembers every strike of a round on a hull (`Hole`: shell, ship, the
point and the plate's normal toward the shot in her frame, whether it is her outside, whether it holed; kept 3 s) and
draws only those on her outside: `HullVolume::is_exterior` steps 0.7 m out along the normal and asks the simulation's
own `naval_sim::hull::hull_contains` through the host's `HullShape` (`naval_effects_sim::SimHull`) (the side, the weather deck and anything above it are outside; a transverse
bulkhead or an armoured deck inside has hull on both sides). `BlastEffects::burst` then places the detonation:

- **On the plating** (an outside strike of this round within 1.6 + s m): the HE fireball at that strike, blown off
  along its normal, with torn plating thrown in a cone about it. HE rounds on contact and armed AP rounds stopped by the
  plate land here (in the scouted duel every 10.5 and 15 cm HE burst lies 0.0 m from its contact).
- **Inside the hull** (`HullVolume::contains`): out through the nearest hole the round made in her outside, within
  26·√s m, as a jet of six hot puffs, five puffs of smoke pouring out after, 22 splinters and 8–24 pieces, a 650·size²
  flash light at the hole and a smoulder of 14–30 s; and, when the deck above is within 4.5·√s m
  (`HullVolume::deck_above`), up through the deck: a fireball of six lobes, a smoke column, 24 sparks, 10–36 pieces
  with planking among them, 1000·size² light. Deep inside and far from both, nothing shows (her fires and flooding are
  the simulation's).
- **Elsewhere** (in the superstructure, all light plating, or in the open): the fireball with wreckage thrown all
  round.

**Size.** Bursts are drawn at 0.55 of the web's scale (`burst_scale`): the web's 15 cm burst stood 15–18 m high; a
photograph of one shows a fireball of a few metres and a cloud of eight or ten. Flash lights are a third to a half of
the web's, which washed a hull white for 20 m around a 15 cm hit.

**Debris** (`src/debris.rs`, `shaders/debris.wgsl`): up to 640 pieces in flight, each a small irregular shard (a box
whose corners the seed pulls in and out; since 2026-10-21 a magazine's larger pieces torn plates and girders and deck
timber planks, [a ship's loss](ship-loss.md)) tumbling about a fixed axis, flown with linear drag (more for small pieces)
and gravity integrated exactly per step, faceted and lit by the effects' sun (through the sun shadow maps, which the
shards also cast into), sky fill and flashes with a glint off bare steel. A hot piece glows yellow-white to dull red as it cools and licks a short flame into the fire pool; the
larger pieces of a burst trail thin grey smoke into the soot pool (`DebrisBurst::trail`: none for an entry, a stop or
a glance; 0.5–0.75 for plating torn by a burst; 1.5 for a magazine's burning wreckage); a piece reaching the sea throws
a puff of spray and is gone. A full set gives up the piece furthest through its flight. `EffectCounts::debris` counts
them, `DrawStats::debris` the drawn.

**Impact marks** (naval-ships `marks.rs`, `impact_atlas`; since 2026-10-10 the strikes of bombs and torpedoes too): a
procedural 1024 × 1024 atlas of 256² tiles. A strike names one of six kinds (`ImpactStyle::tile`):

- an AP hole about its calibre with a jagged lip bent in, patchy bare steel and blistered paint, and a soot stain
  climbing above (4.5 calibres wide, 1.6 tall over wide), its hole cut through where her plating can be ("Holes"
  below); a dish of scoured steel where armour stopped it, the plate's hard face flaked off round it (4); a gouge
  along a glance, scraped bright and its torn steel shoved up along it (9 × 3); both pressed into her plate where it
  can be ("Dents" below);
- an HE burst, sized from its **charge**, not its calibre (`ShellEffect.explosiveKg` on the event; 1000·calibre³ kg
  when an event has none): 3.2·∛charge m across holed, 3.0 not, as a blast's reach scales. The tear is a sixth of it,
  the simulation's own tear for the charge (`tear_m2`, 0.56·∛charge across). A five-inch shell's mark is 4 m across,
  a 15 cm shell's 4.9 m, a 38 cm HE shell's 12.8 m, a 500 lb bomb's 120 kg 15.8 m (before, a bomb's was sized as a
  35 cm shell's, 4.2 m). Holed, the tear is cut through (see "Holes" below) and the decal paints only a dark core in
  it; not holed, a crater scoured in armour and pressed into her plate ("Dents" below), its paint blown off to bare
  steel and red-lead primer round it, a black stain thrown out in rays, blistered paint, the smoke's stain above, and
  fragments chipping the paint (bare steel and primer). Round a hole (since 2026-10-20, Bill: "remove the red stuff around the hole", then "make the surroundings
  more realistic too. right now they look cartoony"): soot in a few ragged, wandering lobes (domain-warped noise,
  `blast_soot`), heaviest at the wound and reaching further up than down, climbing the plating above in wisps; the
  paint under it heat-browned and blistered pale in places (`heat_browned`); near the tear scoured off in flakes to
  bare steel and mill scale, blackened the nearer the wound (`scoured`); no red lead. Splinters (`splinters`) flew in
  three sprays, most near the hole: 70 ragged holes punched through, most of them small, and 220 strikes that only
  chipped the paint bright or dented it dark. Before, an even black disc thrown out in rays and 460 round pits with
  lit lips spread evenly round it read as a cartoon;
- a torpedo's burst against her side (the `torpedo-hit` event's hull-frame `surfaceImpact`, two metres or more under
  water): 2.2·∛warhead m across, the warhead 300 kg at 533 mm growing with the cube of the diameter (14.7 m for a
  533 mm torpedo). A hole about two fifths of it across, wider than tall, cut through with its plating blown in in
  great tongues ("Holes" below), its plating round it lit as dished in between the frames (the dish, below); its paint
  scoured in flakes to bare steel, and fuel oil and the warhead's residue thrown up the side in a ragged, patchy fan.
  Until 2026-10-20 the tile painted the frames' dishing as even stripes and the oil as drips in regular lanes, which
  read as a curtain. It does not glow (the sea quenched it) and leaves no scorch spot. Its tile is a 512² block.

On a deck or a roof (a face within 20° of level) an AP hole and both bursts take their own radial kinds instead
(`marks::deck_kind`): nothing climbs a deck, so the soot falls round the wound. A crater's black is thrown out in long
rays and its fragments chip the deck. A torn deck's soot lies in lobes swept further one way than the other and
faintly streaked out from the hole, its splinters' marks drawn out a little along their flight (no starburst since
2026-10-20), and its tongues are bent down into the hole. A mark on a side or a
superstructure stands upright, its stain up the plating, tilted a little by the shell's id; on a deck it turns freely.
Every other pair of ids mirrors its mark. A mark's box is deep enough for the plating's curve under it (a tenth of its
size, 0.8–2 m), and every part of the struck body inside it takes its share, each in its own frame: a hull is drawn in
parts, and a torpedo's mark two metres down crosses the boot-topping onto her side (before 2026-10-10 a mark covered
only the part it landed on, so a torpedo's lay wholly under the sea). A fresh wound's torn steel glows and cools (`MARK_COOLING_S` 1.4 s to 1/e) on the fleet's scorch clock
(`Fleet::scorch_advance`); a host that never runs it keeps its marks cold. The glow reads each pixel's place from the
wound from the vertices (`MarkVertex`'s last pair, `wear.zw` in `fs_decal`). `cargo run -p naval-ships --example
impact_atlas` writes the atlas over grey paint to `.build/native/marks/atlas.png`.

**Holes** (naval-ships `holes.rs`, since 2026-10-20). Close up, a large wound's decal read as paint: its torn tongues
and dark opening lay flat on the plate, lit as the plate was and still however the camera moved. A wound that opens her
plating now cuts it: a burst that holed a plate (`BURST_TORN`, `DECK_BURST_TORN`), an armour-piercing round's hole
(`AP_HOLE`, `DECK_AP_HOLE`) and a torpedo's (`TORPEDO`), on her hull, her upperworks or a gunhouse. A strike on
anything else a joint moves (barrels, a director, an aerial) keeps its decal alone.

- **Which way is out** (`marks::project`): the struck face's own normals as drawn, else the round's path (it came from
  outside), and only failing both the strike's normal. The strike's normal is the simulation's plate's as its corners
  were listed (`protection::segment_plate`), into her where they run the other way; until 2026-10-26 it decided, and
  a hole on such a plate had its pocket and tongues built out of her side, black cones standing off her plating.
- **The outline**: 48 radii about the wound in the plate's plane, jagged, wider where a crack runs out between two
  tongues, and never inside the dark core the decal still paints (`HOLE_CORE`, 0.8 of the tile's tear; the opening
  kinds' tiles no longer paint tongues; an AP hole's outline is its tile's painted hole, lip and all left round it).
  Each radius is laid on her surface by rays square to the plate, stepping out
  from the wound, and stops where the first face a ray meets is not her plate (a deck edge, a corner, a fitting
  standing off it) (`holes::lay_outline`, which dents share). An outline cut short round more than half the wound cuts
  nothing: the decal alone. Between its samples the outline runs straight (`outline_radius`, `hole_radius`; before
  2026-10-24 it ran straight in angle), so a mesh whose edge runs through the samples (a dent's) meets the cut exactly.
- **The cut** (`fs_holed`, `hole_scan`): a ship with holes draws her hulls without every fragment inside an outline,
  within its slab of the plate's plane (her surface's own curve there and a margin) on a face standing square enough
  to it (`HOLE_FACING` 0.35: a wall rising from a holed deck stays); her decals are cut the same way. The rest of the
  fleet draws with `fs_main` and pays nothing. Her shadow is not cut, so the sun never lights her inside through it.
  A gunhouse's hole is cut in its frame and turns with it: the parts its outline is laid on and its pocket kept short
  of are those of the gunhouse alone (hung from its mount's yaw node), and its record is laid afresh each frame from
  where the gunhouse stands.
- **The tongues**: 4 to 9 arcs of the outline (a torpedo's fewer and greater), some torn in two, one in five a stub,
  now and then one gone. Each is bent in at its root (11° to 37°), curls toward its tip (to as much as 95°) and folds at
  a crease part way along, crumpled, its sides torn ragged and its tip uneven; about one in eight is bent out instead.
  An AP hole's are petals: 6 to 9, short (0.3 to 0.6 of its radius), every one bent in, the cracks between them short.
  They are flat-shaded, so each facet catches the light as crumpled steel does. Their outer faces are soot with her
  grey paint blistered through in patches, their inner faces soot, their torn edges dull bare steel that glows while
  fresh as a mark's does (`fs_hole`). No red lead or rust shows round a hole, in its decal or on its tongues.
- **The dish** (`hole_scan`, since 2026-10-20): her plating round the outline is lit as dished in toward it, sunk
  `HOLE_DISH_DEPTH` (0.16) of the hole's radius at its edge, in five lobes, out to `HOLE_DISH_SPAN` (1.2) radii beyond
  it; and near it her plating between her frames (`HOLE_FRAME_SPACING` 1.2 m apart along her) sunk by 5 % of the
  hole's radius (at most 12 cm), fading out over two radii: a torpedo's hole shows her frames standing, a shell's
  hardly. Only the normal tilts (her shape stays), in the hull's and her marks' fragments alike, so it reads as bent
  steel from any side and under any light where painted shading would not.
- **The pocket**: the plate's torn edge as thick as the plate (2.5 to 4 cm), then a sooted pocket about the hole's
  radius deep, kept short of whatever stands behind it (a deckhouse's far wall). Her hull is an empty shell: without it
  the hole would show her far side's plating from within.
- **Light**: behind the plate, the sun reaches a tongue or the pocket only through the opening (`fs_hole` finds where
  the sunlight crossed the plate and reads her shadow there), and the sky the less the deeper in. The tongues and the
  pocket are drawn into the sun's maps and the contact depth where they are cut, so a tongue bent out of her shades her
  plate, and one across the opening the sunlight in.
- A ship keeps 48 holes and dents (`MAX_HOLES`, 32 before 2026-10-24, when dents came to share the table); a new hole
  past them takes the slot of her oldest dent, else of the oldest hole no larger than itself (a torpedo's is not given
  up for a shell's), a new dent only an older dent's, and one with none to take keeps its decal alone, as does a wound
  whose hole gave its slot up. A hole goes with its mark and with `Fleet::clear_marks`. The table of every ship's holes
  (binding 27, `HOLE_WORDS` vec4s a hole: its centre and how far from it it does anything, so a fragment far from it
  reads one word of it, its normal and reach, its across axis and heat, its shape, its slab and her surface's offset
  at the wound, its radii, and her surface's offset at each) is written when one is torn or given back and while one
  glows; each ship's hole geometry, a few thousand vertices a hole, is packed afresh in her own buffers when hers change
  (her holes' first, her dents' after) and drawn under her mark record.
- **Cost**: a hole is cut on the CPU when its mark is laid: its outline's rays meet only her triangles binned in a grid
  about the wound. 0.5 to 0.8 ms a shell's hole and 1.3 to 2.2 ms a torpedo's in a `dev-client` build (M5 Pro, other
  sessions building); before the grid a deck's large triangles took one 180 ms. A ship's holes are cut only while one
  of them stands at least 1.5 px across on screen (`HOLE_MIN_PX`): farther off, her marks keep their dark cores and her
  hulls draw with `fs_main`. The GPU cost of `fs_holed` (the ship shader with a discard and a loop over her holes) is
  not measured yet.
- **Restored**: the simulation keeps where each hull was struck (`Vessel::scars`, the latest 96: the strike's place,
  normal and way in her or her gunhouse's frame, and the round), saved with the battle and never in a frame. The packet
  carries them (`ShipVisual::scars`, shared while unchanged; a team view's under the ids its events give the rounds, so
  a scar's mark is its strike's), and the effects lane marks a hull drawn for the first time
  from hers (`naval_effects_sim::scar_strike`, `Fleet::restore_impact`), cold, a few a frame within 4 ms (`RESTORE_MS`), paused
  or not, with her shell strikes' scorch: a loaded save, a replay's checkpoint and a theatre's late arrival show the
  marks and holes she bore. Her new strikes wait behind them (her oldest marks give way first), and a strike whose
  event comes with her (a restored battle's event log replays its latest) is left to the live path, mark and scorch,
  so neither is laid twice. Before, every mark and hole vanished on a load.
- **Review**: `strikes.ts` draws a hole's moment and close-up as it does a mark's; a hole is easiest judged from a few
  sides with the effects off (the `layers` RPC), since its tongues move against the opening as the eye moves.

**Dents** (naval-ships `dents.rs`, since 2026-10-24, Bill: "recently we made shell penetrations 3D but can you make non
penetrations 3D too?"). Where armour held, the decal alone read as paint: a stopped round's dish, a ricochet's gouge and
a burst's crater were drawn flat on the plate, lit from one side whatever the sun (the tiles painted their shading in).
Such a mark now presses her plate in: the same cut as a hole's, over its outline, with the dent's own mesh in its place.

- **What is pressed**: a stopped round's dish (`DISH`) in a side's face-hardened armour, a bowl about a calibre across
  and a tenth to a quarter of a calibre and more deep, its plate's hard face flaked off round it in a ragged ring a
  fifth of its depth down (out to `marks::DISH_FLAKED` dishes); the same in a deck's homogeneous plate, its lip pushed
  up instead. A ricochet's gouge (`GOUGE`): a furrow along the round's path, ploughed in from nothing, flat-floored and
  steep-walled, deepest past its middle and shallower again as the round rose out, its walls torn in and out along it,
  the steel shoved up along both its sides in ragged ridges and heaped in lumps where it left; a twelfth to a seventh of
  its calibre deep, its tile's 9 × 3 calibres long. A burst's crater (`BURST_CRATER`, `DECK_BURST_CRATER`): a shallow
  scoop (about a hundredth of its mark's half-width, 2 cm for a 15 cm shell) with a pit as deep again where it sat and a
  low rim, its floor pitted; on a teak deck (`HoleSite::teak`) its planks blown out to the steel deck under them
  (`TEAK_PLANKS_M`, 7.5 cm), each plank broken off on its own about the crater's edge, so the hole in the planking runs
  along the seams and steps out and in plank by plank, the steel under it dished as a crater.
- **Where its tile paints it**: the tiles of the marks armour held draw what is pressed (`marks::dish_radius`,
  `flaked_radius`, `crater_radius`, `gouge_parts`, with the tile's own noise, `tile_seed`), and since 2026-10-24 paint
  no light of their own; each is opaque a little past its dent's outline, so a dent, which wears its tile opaque, never
  shows paint the tile leaves clear. The atlas's colours are carried twelve texels out into its clear texels (`bleed`),
  so filtered and mipped a mark's edge fades to its own colour, not to black.
- **The mesh** (`press`): 48 spokes, one through each of the outline's samples, and ten rings and more where its
  profile turns; each point on her surface (its offset off the plate's plane a quadric from the wound's to the outline's
  sample, so it follows a barbette's curve and her side's flare), pressed by its profile, with its tile's uv, and a
  skirt out past the outline under her plate. 4,000 to 5,000 vertices a dent, about a hole's.
- **Light** (`fs_dent`, `dent_light`, `dent_sun`): it is shaded as its decal is (`shade`, its mark's tile), its bare
  steel brighter, smoother and half metallic (`dent_finish`, from its vertex colour), a teak deck's plank ends raw and
  charred timber, and finely relieved (`dent_relief`: a gouge scored along its length, a dish's and a crater's floor
  pitted). Below her plate the sun reaches it as it reaches a hole's pocket, her shadow read where the light crosses
  her plate, but its own walls shade it too: `ship.wgsl` marches the same profile toward the sun over her surface's
  offsets (the record carries them), twelve steps bunched toward the point, and the sky reaches it the less the deeper
  in (`hole_cavity`). The profile the march reads is the mesh's without its noise (`dent_round`, `dent_gouge` mirror
  `dents::round_height` and `marks::gouge_parts`).
- **Later marks** lie on it: a strike's decal covers the dents under its box as well as her plate (`marks::project`,
  `Mark::on_dents`), its pieces on a dent drawn with the dent's light and cut by no dent (`fs_decal` reads them by
  `ON_DENT` added to the kind), so a later burst's soot darkens a dent as it does the plate round it; a mark older than
  the dent stays cut there, scoured off by the strike that made it. While her dents are not drawn her plate's pieces
  stand for them. A dent pressed over a smaller one replaces it; one within a larger keeps its decal alone, which lies
  on the larger's plate.
- **Cut and kept** as holes are: in the hole table (`Hole::shape` says a dent: its stretch, depth, scale and shape), its
  outline laid on her surface by `lay_outline` and held only if 85 % of its samples reach nine tenths of their way (a
  dent hanging over a plate's edge keeps its decal alone), no deeper than 0.8 of the room behind her plate, from rounds
  of 75 mm and more (`MIN_CALIBRE_M`: a light gun's would be too small to show its shape and would take larger dents'
  slots), a gouge's
  outline taken with its path shrunk four times so its samples ring it evenly. Her plating is cut for her dents only
  while one stands 4 px across (`DENT_MIN_PX`). A dent goes with its mark, gives up its slot to a new hole first, and
  is restored from her scars as a hole is.
- **Cost**: 0.65 to 0.7 ms a dent laid with its mark (1 ms a teak deck's), `dev-client`, M5 Pro, other sessions
  building. A hole's record grew to 29 vec4s, but a fragment far from a hole or dent now reads one of them (its bound)
  where it read two.
- **Review**: `tests/dents.rs` lays each kind on Bismarck's belt and teak deck under a low sun and writes
  `.build/native/fleet/test-dent-*.png`; in a battle, `strikes.ts --only` finds the rounds and close-ups are best from a
  few sides with the effects off (the `layers` RPC), the camera on the shooter's side of the plate.

**Wrecks** (naval-ships `wreck.rs`, since 2026-10-20, Bill: "yea lets start that now", after wrecked parts were
offered as the next step from the holes). A destroyed mount (`MountPose::lost`, its hit points spent: no repair restores
it; `status` says `Disabled` for a jam or an empty magazine too) and a destroyed module (`ShipVisual::lost_modules`,
indices in her definition's `modules`, shared while unchanged) are drawn wrecked, each a turn of its own joint laid in
the pose after the joints (`ShipPose::wrecks`). Every wreck is drawn from its gear's id alone, so a ship looks the same
after a load:

- **A turret or an enclosed mount**: its train held where the simulation left it; each gun where it fell, on its
  depression stop (or past it, off its trunnions), part way up, or cocked 12° to 32° high, never all alike; a
  secondary's gunhouse jammed 1° to 2.5° askew on its roller path.
- **A light gun** (an open mount of 40 mm or less): knocked over 20° to 42° with its barrels hanging, or blown overboard
  (two in five), its pedestal left. A heavier open mount (a 10.5 cm twin, a 5-inch) is knocked 4° to 11° askew.
- **A director**: knocked askew and turned off its line, or blown off (one in four). **A radar aerial**: hanging off its
  mast, or shot away (nearly half). A fitting longer than 6 m is never carried off, and none tips further than drops
  its end 1.2 m (`MAX_DROP_M`), so a long rangefinder leans less than a small aerial.
- **A torpedo mount**: jammed 2° to 6° askew.
- **A roof blown open** (since 2026-10-20, the third pass): a destroyed gunhouse (not an open mount) that a shell went
  into (one of her scars on that mount with its plate penetrated) had its roof blown open by the burst and the cordite
  it lit, as Lion's Q turret's was at Jutland (`Fleet::blow_roof`, from `naval-client`'s `RoofWatch`). The roof takes
  a torn hole from within: a mark 2.2 gunhouse-widths across laid on its roof (a deck's torn burst, so its hole is
  about half the roof and its soot runs down the gunhouse's walls), its tongues all thrown out and up and curling back
  (`HoleSite::outward`), the gunhouse within a sooted pocket. Laid hot when the turret was seen standing the frame
  before, cold when she is first drawn already wrecked (a load); once per turret. A turret lost without a shell inside
  it (flooded, burned out, its crew lost) keeps its roof and only its guns' wreck.

A director's or an aerial's assembly is found for its module when the preset is built (`wreck::find_fittings`): by id
(the module's, less an `equipment-` or `aerial-` prefix, or its radar's rig joint), else as the model's assembly whose
box overlaps the module's by at least a quarter, no larger than three times it any way (8 m at least; an id drawn as
several roots goes by its box). Directors are
found first; an aerial whose assembly is a director's or holds one (Fletcher's turns on her Mk 37) falls with that
director and is no wreck of its own. 349 of the fleet's 430 are found; the rest are drawn inside larger assemblies and
stay as they stand. They are found when the preset is dressed and ride the pack (`BakedPreset::fittings`): the packed
model carries no geometry to find them by. Each found one is a moving
joint, so it is dressed apart from her hull (its own parts; the ship is still one draw). A hole through a director's
house is cut in its frame and laid each frame as it stands; a wreck that carries its gear off draws it to nothing, and
its holes cut nothing. The packed format went to 16 with the definition's modules (`Module`) and a weapon's mounting and
bore. The harness's `battle_state` lists each ship's `lost_mounts` and `lost_modules`.

**Wrecked by bursts** (since 2026-10-20, the second pass). Her boats, rafts, searchlights, cranes, catapults and masts
have no state in the simulation, so the scene wrecks them from the strikes she bore (`Vessel::scars`, which a save
keeps: `ShipJoints::blast_wrecks_into`, recomputed per hull when her scars change, `naval-client`'s `BlastWrecks`).
A strike counts as a burst of its charge where it struck (a high-explosive shell's or a bomb's; 1000·calibre³ kg where
the event gave none), or as a round through whatever it went through (an armour-piercing shell's); a torpedo's (under
water) and a gunhouse's (in its own frame) count for nothing here.

- **Light gear**, smashed by one burst within its reach per cube root of the charge: a raft 1.4 m (gone), a boat 1.2 m
  (gone, or knocked 12° to 28° off its davits), a searchlight 1.0 m (gone, or knocked 15° to 35° askew). A 15 cm
  shell's 3.6 kg smashes a boat within 1.8 m, a 38 cm shell's 55 kg within 4.6 m, a 500 lb bomb's within 5.9 m.
- **Heavier gear**, wrecked once the bursts reaching it add up (each ∛charge / 1.5 at the gear, to nothing at
  1.6 m per cube root; a round through it 1): a crane 2 (knocked 8° to 16° askew), a catapult 2 (3° to 7°), a mast 2.5
  (a short one shot away or leaning 10° to 22°, a tall one leaning 4° to 9°).

Each is found by its assembly's name (a boat, whaleboat, cutter, pinnace, launch, gig; a raft or float; a searchlight;
a crane or derrick; a catapult; a mast), never the platform, davits, crutches, casing or deckhouse that carries one,
the innermost where one assembly of a kind holds another, no larger than its kind may be, drawn as one root (Hood's
`foremast` is two: her spotting top's glass beside its walls), holding no gun mount and no ensign (Nagato's mainmast
carries her gaff, and the flag is hoisted where her rig says), and nothing else standing mostly inside its box that
would not go with it (a mast's platforms: most masts stay). Gear once wrecked stays so for as long as the client draws
her, though her scars keep only her latest 96 strikes; a hull restored after more bears only their wrecks. Funnels
are left as they stand: several models build a funnel's cap and the platforms on it apart, and a lean would leave them
in the air; their holes and soot show their damage. Nothing yet flies off when gear is wrecked live: the burst that
wrecked it throws its own debris.

**Review**: `bun native/tools/strikes.ts` (`--help`). A torpedo's close-up looks square at the plate it tore (the
event's hull-frame normal turned by her heading). It scouts a staged battle (default `bismarck;;bismarck:static` at
4 km, seed 1941, 300 s) with `battle_advance` for every strike and burst on a hull, torpedo hits among them (`--list`
prints each round's story; a torpedo's id is its own negated, less one), then for each chosen round (`--only <ids>`, or
one of each kind) reloads, skips to 1.5 s before it,
`clock_sync`s, and `frame_advance`s and `frame_draw`s a frame at a time from the shooter's side, 34·√(calibre / 0.38) m
off the strike (fov 40), capturing at −0.05, 0.017, 0.05, 0.1, 0.2, 0.35, 0.6, 1, 2, 4, 8 and 15 s, and a close-up of the
mark after, into `.build/native/strikes/<label>/`. Battles replay exactly, so `--binary <another naval-client>` draws the
same rounds for a before and after. In that duel the target took 231 rounds in 300 s: 139 10.5 cm and 55 15 cm HE
contacts that burst on the plate, 26 38 cm AP penetrations (most bursting inside, 0–24 m from their entry), 4 stopped,
1 ricochet. A magazine never went up in three scouted battles; the magazine's wreckage was reviewed by staging one in
`stills`' recorded battle (an event added to the fixture by hand, not committed).

## Scorch and impact marks

The ship meshes draw lasting damage (`naval-ships`: `marks.rs`, `scorch.rs`, `holes.rs`, `dents.rs`; scorch in
[lighting](../lighting/lights.md#scorch)). What `src/damage.rs` provides is the effect side, derived from the same
events:

- `HullStrike::from_event(&CombatEvent)`: every strike with a `surfaceImpact` and a shell, or a `torpedo-hit` with a
  `surfaceImpact` and its torpedo, in the struck body's frame (the hull, or a mount's gunhouse via `mount_id`), with its
  outcome, filling (AP, HE or a torpedo's warhead), calibre, id, the decal turn the web uses and `impact_style` (the
  mark's kind and width/height, an HE burst's from its charge; no longer the web's `impactStyle`, see "Shell strikes
  and debris"). Fires (place and intensity) are
  in `ShipFrame`; the smoulder a strike leaves is in `BlastEffects`.
