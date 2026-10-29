# Combat effects: a ship's loss

## A ship's loss

The look survey of 2026-10-21 found a loss drawn as a fireball, a hull going down like a lift and an empty sea: a
magazine's smoke gone by 90 s, a capsizing hull's twelve fire columns cut off at once and left hanging, no venting, no
water off the hull, no oil and no debris where she went, and her wreckage cubes trailing dotted smoke. What real losses
showed, and what is drawn now (`loss.rs`, `flotsam.rs`, `sea_fires.rs`; the oil is the ocean's, `../ocean/oil.md`, "Her oil
after she goes, and burning"; the water off her hull the water lane's, `../water-effects.md`, "Water off a lost hull"):

- **The burst and its column** (`blasts.rs`, `MAGAZINE_*`, 2026-10-24). The North Sea film showed Belfast's magazine
  as a game's: a round orange fireball under a neat mushroom (a cap of nine even billows thrown up at 125 to 140 m/s
  in a ring, on a stem of twelve, within a second), sparkling with orange dots: 40 grains a metre wide burning for 1 to
  3 s and the flames of half her wreckage, each drawn with at least 1.5 pixels' light however far off
  (`sprite.wgsl`, `LEAST_PIXELS`). Real ones: Barham's film (British Pathé, 25 November 1941) shows within a quarter
  second a fan of grey rays on every side, smoke and wreckage flung out, a lumpy dark mass among them, then a broad
  cloud over her that a column climbs out of, "wreathing and eddying high into the sky"; Shaw's photograph at Pearl
  Harbor a white-hot burst with white streamers; Lexington's, Princeton's and Mount Hood's lumpy columns with ragged
  heads, Mount Hood's with long thin arcs of debris. Now: 18 jets, each 7 billows thrown together along one way
  (20° to 77° up), the nearer slower, so a ray lengthens to 55 to 140 m · s as it flies and tapers to its tip,
  grey between the soot and the gun smoke, standing out of the fireball for its first second or two and rolling into
  the cloud (the elongation 2.6 relaxes as every puff's does). The column's 16 billows, of every size from 14 to 34 m
  · s, climb out of a foot over half her length at 12 to 72 m/s · s, slowed by the air (0.3 a second) to their
  heights within five to ten seconds, some 210 to 270 m · s at the top, leaning together 3° to 13° one way; the lower
  burn for their first seconds; a fifth are pale with steam; grey-black to brown-black otherwise. Its head is 7 billows
  thrown up half a second to two seconds after with the column's highest, spreading 10 to 65 m · s each its own way and
  height. The flash's three sprites are white and gone within a third of a second (they were orange and grew to
  100 m for two thirds of one). The cordite is 48 sticks 0.2 to 0.45 m across, burning yellow-white for 0.4 to 1.1 s:
  streaks close to, too faint to see miles off. Three in ten of the large pieces and a quarter of the small burn in
  flight (were half), and her wreckage trails paler smoke (`DebrisBurst::trail_colour`). Seen with `effects-trial.ts
  --only magazine,magazine-far` (`magazine-far`: a cruiser's, through a long lens from 4.5 km, as the film saw
  Belfast's). Left as they were: the white dome of the blast's cloud (`shock.rs`) and the flash's glare (`bloom.wgsl`).
- **The pall** (`blasts.rs`, `PALL_*`). Hood's column stood "600 to 700 feet" (Phillips, Norfolk) and her pall "at least
  100 feet high" (Mockridge) was still standing as Prince of Wales passed it (Slade; the boards' evidence, ADM 116/4351);
  Queen Mary's cloud rose "to a height of approximately 800 feet" (Proceedings, April 1925); Barham's "great cloud of
  yellowish black smoke … went wreathing and eddying high into the sky", and when it cleared she was gone (Cunningham,
  *A Sailor's Odyssey*, 1951); Mount Hood's smoke spread some 500 yards about her (DANFS). A magazine now lays 12 billows
  (for a 240 m hull, as the cube root of her length goes) through its column, 40 m to 220 m up, 2 to 8 s after the
  blast, fading in within the column: 50 to 80 m across, each widening 1.2 m a second by the air's turbulence (some
  300 m across five minutes on), carried by the wind at its height (0.9 to 1 of it), standing for ten minutes and
  thinning over the second five. Brown-black, cordite's and fuel oil's ("yellowish black", "a mass of brown smoke").
  In the game's look they are billows of the smoke clip, playing it over their ten minutes.
- **A fire going under** (`fires.rs`, `remember`, `FireBatch::release`). A burning place whose point goes under the sea
  (she settles, rolls over or plunges) burns no more, whatever the simulation still reads of it (a keel-up hull keeps
  her fires' state). Under is under the swell she rides there (`CombatEffects::swell`) by half a metre, and out again
  only half a metre clear (`DROWN_MARGIN`), so a hole by her water line dipping into a wave as she rolls does not put
  its fire out and light it again with every roll; burning oil on the water never goes under (`on_the_water`). The
  fire dies in a burst of steam on the water over it (twice a flooded fire's 2.2 s), and its column,
  fed no more, is let go. Each of its puffs lives on as long again as it has been climbing and a quarter more (at least
  4 s), never longer than it would have, so the youngest smoke over the fire clears first and the oldest, aloft, last,
  while all of it drifts on with the wind: the column thins from its foot. Rolled over in 12 s, Bismarck's fires go out
  one after another as each place goes under, each in its own steam.
- **Her funnels venting** (`loss.rs`). No warship's account was found; the merchant ships' are the reference: as
  Titanic's boiler rooms flooded her safety valves lifted and roared, and at the break "smoke and sparks rushed from
  the funnels" in "a big black cloud" (Bride, Dahl; US Inquiry); Lusitania's sea reaching a funnel blew a passenger back
  out of it, black with soot (Larson, *Dead Wake*, 2015). A boiler room flooding (a quarter full) vents steam from the
  funnel over it (the nearest along her) for 10 s, white with a little soot. The sea at a funnel's base while its mouth
  is clear drives her air up it, black: a quarter of the air her settling hull drives out (her waterplane's share over
  the funnel's area, times the rate she settles there; an estimate), some 10 m/s from a battleship settling 0.5 m/s, at
  most 40. When its mouth goes under, its last air bursts white on the water. Billows of the smoke clip in the game's
  look. Only a funnel standing within 60° of upright vents a jet or a burst: one lying along the sea as she rolls over
  vents nothing (her air comes up through the water, the sea's boils). The vented gas is a stream (since 2026-10-21,
  the second pass): puffs under half the mouth's width as they leave it, eight a second and more, widening some two and
  a half widths in five seconds as the jet takes in air, as dense at the mouth as a working funnel's smoke (steam
  stopping some two fifths of the light through its middle, soot half) and thinning as they widen
  (`EffectParticle::thins`: the optical depth through a puff falls as its width grows, as a funnel's plume does,
  `funnel.rs`' `puff_optical`). The first pass drew them as wide as the mouth at a density of 2.6 that never fell as
  they grew 2.4 widths, slowed to a stop within a few metres: from 2 km, solid dark balls standing on the sea while she
  rolled over and her funnels lay along it.
- **The moment she is gone** (`loss::all_under`): a lost hull (`ShipFrame::lost`: sinking or out of the fight for
  good; a submarine's dive is no loss, and only a lost hull blows her air out of her funnels) with every corner of a
  box about her, her ends to her deck and her middle half to her highest structure, under the sea. The host reads it
  (`Losses::went_under`, `since_gone`) for her oil and its fire.
- **Flotsam** (`flotsam.rs`). Hood's site five minutes on: "one large red raft and a considerable amount of wreckage
  amidst a huge patch of oil" (Sunderland Z/201, AIR 15/415); two hours on, "wreckage of all descriptions … hammocks,
  broken rafts" (Electra's Jack Taylor); her survivors on "small grey 3 ft square rafts" (her Denton rafts, over twenty
  of them). Barham's: "a bubbling, oily-looking patch … dotted with wreckage". Where she went under, over her length and
  beam, a ship of length L leaves about L/22 Carley floats (Nos. 17 to 20: 5 × 10 ft to 9 × 14 ft, a 16 to 20 in tube;
  Australian War Memorial technical paper, 1993), L/12 cork rafts (3 ft square), L/100 boats (at least one: a 27 ft
  whaler to a 32 ft cutter, swamped), L/1.6 planks (4 to 6 in by 3 in, 3 to 20 ft) and L/7 baulks and spars: Hood's
  12, 22, 3, 164 and 37, a Fubuki's under half of that (the counts scale Hood's; estimates). They come up within the
  first minute (floats first, timber as it works free), thrown off her by the air boiling up after her (to 2 m/s, dying
  over 30 s), and drift with the surface (the oil's 3.5 % of the wind) and a leeway of their own (a float or raft 1 %
  more, a boat 2 %: the search-and-rescue manuals' 3 to 5 % for a raft; an estimate). Each rides the combat sea's long
  swell at its own freeboard, tilted to its slope, turning slowly, for half an hour (the oil's patches' life), its last
  minute going under. Drawn as the debris is (lit, faceted, in the sun's maps), in shapes of their own: a Carley float a
  ring of four tubes, a raft a slab, a boat an open hull fine at her stem and narrowed to her keel, a plank a long box
  with splintered ends. Planks a blast threw that come down on the water float there too (`Debris::take_landed`).
- **Torn plates and girders** (`debris.rs`, `DebrisBurst::plating`, `shaders/debris.wgsl`). Queen Mary's "turret roofs
  … blown 100 feet into the air"; Indefatigable's picket boat "blown intact to a height of approximately 200 feet"
  (Proceedings, April 1925); Mount Hood's largest pieces "no bigger than 16 by 10 feet" (DANFS). A magazine's larger
  pieces are now 85 % torn plating, of which a third girders: a plate folded at a crease (two boxes hinged 20° to 70°,
  its outer edges torn), an angle section kinked part way (four boxes), sizes up to 5 by 4 m for a battleship's. Each
  piece is built of up to four boxes (`PIECE_VERTICES`, 144 a piece; a shape of fewer leaves the rest with no area). A
  magazine's burning wreckage trails a plume of a few metres (0.3 + 1.2 × its size, at most 2.5 m), its puffs laid at
  half their width and drawn out along its flight (up to 1.8 times as long as wide), an unbroken streak, paler than a
  strike's (`#9a948b`, since 2026-10-24; a strike's wisps stay `#707275`). Deck timber flies as planks. The soot pool holds 4,096 puffs (was 2,048) for the trails.
- **Burning oil** (`sea_fires.rs`). The exception, as in the war (since 2026-10-21, the second pass): it burns only
  where a ship still burning above the water goes down in a calm sea (waves under a metre), her own fire lighting the
  fresh, thick oil she brings up (`loss::FIRE_MEMORY_SECONDS`; when and why, with the record: `../ocean/oil.md`, "Her oil after
  she goes, and burning"). A magazine alone lights nothing, and in the staged battles' 9 m/s sea nothing burns. Where it burns, the
  host hands the effects each burning slick's patches, and each burning slick is drawn as a frame of its own whose
  upperworks zones are cells of the sea, 16 m squares about where it was lit (out to 192 m), each lit where a burning
  patch's core covers its middle and its oil lies thick: the slick's oil lies in pools some 25 to 55 m across with open
  water between, two thirds to three quarters of it open (`pool`: one pool or none in each 56 m square, its middle in
  the square's inner half, so they mostly lie apart; each slick's laid out its own way, from its id; the bulk of a
  slick's oil in a small share of its area, ITOPF TIP 1), twice the slick's film in a pool's middle thinning to its
  edge, and a cell burns as hard as its own film (full from 5 mm, a third at the residue). So it burns in separate pools,
  their columns rising apart and merging aloft and downwind as `fires.rs`' columns do (Fuso "surrounded by multiple
  burning oil pools", Tully and Lundgren, 2024; Liscome Bay's "burning patches", War Damage Report 45). The first pass
  lit every cell a core covered: a solid field whose near edge read as a row of flames under one wall of smoke, lit by
  any magazine. A cell's middle stands anywhere in the inner two thirds of its square and its size is two thirds to four
  thirds of one. The grid drifts with the
  surface as the oil does (3.5 % of the wind), so a cell keeps to the oil it lies on, and each burning cell rides the
  swell under it (`Source::lift`); burning oil never goes under. The fires' own presentation
  draws them: flames as a zone's (1.5 + 3.6 × intensity, times the cell's scale: some 7 m, Heskestad's height for a
  16 m pool of heavy oil at half a megawatt a square metre), the black column (heavy fuel oil's soot yield of 8 to 15 %:
  Fingas 2007; Evans et al., J. Res. NIST 106, 2001), their light at night as every fire's. A cell's heat is a zone's,
  63 MW at full intensity (a quarter of a megawatt a square metre, within the measured range). At Hood's site there
  was only "a flicker of flame and smoke on the water" (Nixon, Daily Express, 31 May 1941); Arizona's burning oil, in a
  calm harbour fed by her wreck, was "a real menace" for a day (Tennessee's report, 11 December 1941).

**Torpedo chevrons** (`naval-client`'s `scene/mod.rs`): the heading chevrons at running torpedoes are the helm's marks, as
the gun-aim circles are, and a camera a script placed (an RPC orbit or fixed view, every capture without `--hud`) draws
none; the torpedo camera still does.

**Tests**: `blasts.rs`' `a_magazines_pall_stands_for_minutes_and_drifts_downwind` (the column gone, the pall standing
at eight minutes and gone by twelve, carried at the wind's pace, plates and girders among the wreckage);
`fires.rs`' `a_fires_column_is_let_go_when_its_place_goes_under` (steam off the water, the column's foot clearing,
nothing left at 90 s, no light); `loss.rs`' `a_flooded_boiler_room_vents_steam_from_the_funnel_over_it` and
`a_sinking_hull_blows_her_air_out_of_her_funnels_then_goes_and_leaves_her_flotsam` (some 13 m/s for the ten seconds
the sea stands between base and mouth, gone with her upperworks); `flotsam.rs`'
`a_lost_ship_leaves_her_floats_rafts_boats_and_timber_where_she_went` and
`flotsam_drifts_downwind_with_the_surface_and_rides_the_swell`; `sea_fires.rs`'
`a_burning_slick_lights_the_cells_its_patches_cover` and `the_oil_lies_in_pools_apart`; `oil.rs`'
`only_a_calm_sea_holds_a_burn`; `loss.rs`' `venting_thins_as_it_widens_and_a_funnel_lying_down_vents_no_jet`.

**Cost** (Apple M5 Pro, `dev-client`, 1600 × 900, High, load average 30 to 45 from other sessions, 2026-10-21;
`.build/native/look-loss-sea/cost.ts`: KGV seed 2's friendly-2 90 s after her magazine went, the effects layer on and
off at a frozen moment, three rounds of 60 waited frames, medians): with her pall, 18 cells of her oil burning and 230
pieces of flotsam in view, the effects layer costs 5.5 ms from the chase camera (3.9 on master, where nothing of her is
left) and 4.6 ms from 1.8 km at 13° (2.2): 1.6 and 2.4 ms more, nearly all the burning oil's black column filling the
view (fill, not count). The whole frame 17.7 → 20.3 ms and 13.9 → 16.8 ms; with the effects off 13.8 → 14.8 and
11.7 → 12.2 ms (her oil on the sea, and master's newer commits in the branch's client). Only while a loss's fire and
smoke are in view: a few minutes. The CPU side (her venting, flotsam, the cells' frames) was not measured apart.

The second pass (the same moment and cameras, `.build/native/look-loss-sea-fixes/cost.ts`, load average 18 to 37,
2026-10-21; first pass → second): in the battle's own 9 m/s sea her oil no longer burns, and the effects layer costs
3.2 ms from the chase camera (5.0) and 1.5 ms from 1.8 km (3.3), the whole frame 14.6 ms (17.3) and 11.4 ms (13.7). In
a calm (the console's wind at 3 m/s from 0.1 s after the magazine), where it burns, 6 cells in two or three pools
(20 in one field): 3.7 ms (4.7) and 2.1 ms (2.8).

**Probe**: `effects_probe`'s `loss`: funnels venting, each gone hull's seconds since and flotsam, the flotsam on the
water, the cells burning, the drops lost hulls shed.

**Stills**: `.build/native/look-loss-sea/` in the sea lane's worktree (before: this master's dev-client; after: the
branch's; `shots.sh before|after`, `sheets.sh`, `pairs/`).

**Open.** The flotsam rides the combat sea's long swell, not the shorter waves drawn on it, as the gulls do
(`../wildlife.md`, "Open"): in a seaway a plank floats a hand's breadth over a trough or under a crest; sampling the drawn
surface in a compute pass (`naval_ocean`'s `SeaRead`) would fix both. No people in the water (not decided). The
breaking lane's hull sections are not followed yet: venting and the moment she is gone read her whole hull's pose.
