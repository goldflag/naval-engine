# Ports and infrastructure

A port's container stacks, a marina's boats, bridges over water, airfields and power lines. `naval_terrain::infra` draws
them as the buildings are drawn: portable meshes per chunk of 1 km (the buildings' `ChunkKey`, `Lod::Full` within the
near range, `Lod::Box` beyond) and a layer of its own (`InfraLayer`, `shaders/infra.wgsl`, `infra_caster.wgsl`). The
host's side is `naval-client`'s `theatre::infra` (its own worker, `theatre-infra`, over the buildings' `Context`). The
data is OpenStreetMap's, from the tiles the land already reads (`theatre::osm`, zoom 14; OpenMapTiles carries no
`man_made`, `power` or crane tags). Presentation only: the simulation never knows of any of it.

```rust
let mesh = infra::build_chunk(&infra::ChunkInput { key, tags: &map_tags, ground: &|p| h(p), marina: &marina,
    water: &|p| is_water(p), bridges: &lines, bridge_outlines: &outlines, under: &|p| under(p), airfield: &field });
let mut layer = InfraLayer::new(&mut ctx, &environment.bind_group, InfraSettings::for_quality(&quality))?;
layer.queue_chunk(key, Arc::new(mesh)); layer.set_offset(offset); layer.set_light(&light); layer.status();
let runways: Vec<RunwaySpan> = layer.runways(); // each held runway once: thresholds (theatre m), half width, designators
```

- **Container stacks** (`infra::containers`). A yard is industrial land of a working port (the map's ferry terminal or
  harbour point about the chunk), dry, 55 m or more back from the water the map draws (the quay's apron and its
  gantries), within 600 m of water deeper than 8 m by the window's heights (a container berth's: a marina's or fishing
  harbour's is shallower), amid industrial land or water (13 of 16 points 150 and 300 m about it: not a town's
  industrial block) and open (OSM's outlines cover less than 18% within 120 m: not a warehouse district) (`YardTest`).
  Its blocks lie on a grid
  along the zone's quay (its longest edge with water beyond, else its smallest rectangle's long side), anchored in the
  theatre's metres so blocks run on across chunks: 10 to 18 slots of 40 ft (12.19 m, a quarter of them two 20 ft
  boxes) along, 5 to 7 rows across, 9 m lanes beside and 22 m across their ends, kept 4 m off the roads and railways,
  3 m off OSM's outlines and 2 m off the piers (each box's corners and middle tested). A block is filled to its own
  level, two to five high (one in twelve empty), each bay across its rows mostly at that level (a quarter a tier lower,
  one in ten two), now and then a slot a tier lower (9%) or empty (3%), so the blocks read as ordered rows. Within
  the close range each slot is one column, its sides drawn only above its neighbours; past it each row's run is one box.
  The shader colours each box from a hash of the row's seed, the slot and the tier over 32 muted entries (the
  lessors' rust-reds, browns, greys, whites and beige commonest, the lines' blues, a few greens, oranges, reds), with the
  corrugation and the seams where a pixel resolves them. The buildings' infill keeps off a yard
  (`theatre::infra::clear_yards`, called from the buildings' `chunk_mesh`).
- **Boats** (`infra::marina`): [Harbour boats](#harbour-boats), below.
- **Bridges** (`infra::bridges`). Each `brunnel=bridge` road or railway, its tiles' pieces joined across the 2.5 km
  about the chunk and run from its lesser end, so every chunk it crosses draws it alike, each drawing the segments
  whose middles it holds. The main span is its longest run over water; the clearance over its middle grows with it
  (over the sea 6 m plus a ninth of the span, to 62 m: Vincent Thomas's 457 m span gives 56 m; over a river or lake
  2.5 m plus 3%, to 12 m), reached from the road at each end at 5.5% at most (8% inland). The deck (1.3 to 3.4 m deep
  by the span) is as wide as the bridge's outline (class `bridge`) where the line lies in one (a dual carriageway's
  two ways on one deck draw once), else the road and a metre; parapets, asphalt with its edge lines, lane dashes and a
  median or a double yellow line. Piers of concrete every 32 to 90 m where the deck stands clear of the ground, two
  columns and a cap when tall, none in the channel (the middle 60% of a sea span, up to 200 m). A sea span of 300 m and
  more standing 35 m up is a suspension bridge: two steel towers a tenth of the span over the deck, the main cables'
  parabola to 3 m over the deck's middle and down the side spans, hangers every 15 m (near only).
- **Airfields** (`infra::airfield`), from OpenMapTiles' `aeroway` layer, laid on the ground as decals (`Group::Decal`,
  materials `Runway`, `Taxiway` and `Apron`: pulled toward the eye in the vertex shader, casting nothing, drawn at
  every level). A runway's pieces are joined (the same `ref` only) across the 4.5 km about the chunk, so each chunk
  knows both ends; each half is drawn from its end with the FAA's marks, in the shader: the side stripes, the
  threshold's bars (6 a side on a 45 m runway, 4 on 30 m, 3 on 23 m), the designator (OSM's `ref`, `07L/25R`, matched
  to the ends by heading, else each end's heading rounded; a 3 by 5 figure 18.3 m tall, the letter nearer the
  threshold; the leading zero kept outside North America), on runways of 1,800 m and more the aiming point and the
  touchdown zone's bars, the dashed centre line, and rubber where the wheels touch down. OpenMapTiles has no runway
  width: a mapped runway area's gives it, else the length does (45 m from 1,800 m, 30 m from 1,000 m, else 23 m).
  Taxiways are 23 m wide strips (15 m at a field without a long runway) with the yellow centre line and edge lines;
  aprons and paved areas are concrete slabs with their joints. Aprons lie 10 cm over the ground, taxiways 16 to 21 cm
  (by their `ref`, so crossing taxiways never fight), runways 26 cm. The buildings' infill keeps off an aerodrome's
  outline, its paved areas and 60 m about a runway's or taxiway's line (`theatre::infra::clear_airfields`, called from
  the buildings' `chunk_mesh`). `InfraLayer::runways` hands each held runway once (its thresholds in theatre metres
  with the paint's height, its half width and designators) for the lights (night.md, "Night lights", Runway lights).
- **Power lines** (`infra::power`), from Overture Maps' base theme, type `infrastructure` (OpenStreetMap's power
  mapping; the source and its credit below). Each transmission line (`power_line`) stands on lattice towers and each
  distribution line (`minor_line`) on wooden poles, one at each of its vertices (one within 25 m of the last is a bend,
  not a support), turned across the bisector of its spans; where lines meet, one. A tower is as tall as a mapped
  `power_tower` within 12 m says (15 to 90 m), else 36 m, with two crossarms (at 0.70 and 0.84 of its height), their
  insulators and an earth wire's peak: within the close range its legs, bracing, girts, arms and insulators
  (`Group::Near`, 624 triangles), past it its legs and arms as lines (`Group::Far`). The wires hang in
  catenaries, sagging 3% of the span (2% between poles), five a transmission span and three a distribution span, each
  span drawn by the chunk holding its middle, on chunks made near only (`Group::Detail`; the poles and their wires in
  `Group::Near`). A wire (material `Wire`) and a tower's or a mast's members afar (material `Lattice`) are lines: each
  segment a ribbon opened across the view in the vertex shader to its width or a pixel, whichever is more, lit at its
  vertices (`vs_infra_line`), and drawn after the opaque scene (the Transparent phase, farthest chunk first, writing no
  depth) over whatever stands behind it, as opaque as the share of the pixel it covers. A member no more, so a tower
  afar is a few faint strokes of dark steel that thin to nothing with distance; its colour is 0.6 of the paint's, the
  lattice's inner members and far faces being in the near ones' shade. A wire a little more (the share's root from a
  quarter of a pixel up, 2.8 times the share below), so a span a kilometre off across the sky is a faint line rather
  than a dotted one and a corridor's many spans afar never add up to a band. A line wider on the screen than it is
  takes the normal of its middle (toward the eye) at both sides, so a wire a pixel wide is one colour across and does
  not flicker light and dark as it steps from row to row. Temporal anti-aliasing follows a pixel by its depth, here the
  sky's or the ground's behind the line, so while the eye travels the lines are drawn into its motion pass
  (`vs_infra_line_motion`, `InfraLayer::draw_motion`, the chunks near enough to move a quarter of a pixel): where a
  line moves against the sky by a quarter of a pixel to a pixel or more it gives up its history, or a wire before the
  sky would trail; held still or turned, the lines keep it. The mesh keeps its lines after its faces, in the same
  groups (`InfraMesh::lines`, `line_range`); they cast no shadow. Until 2026-10-10 the lines were opaque and
  mixed 50 to 70% toward the sky's radiance along the view, which below the horizon is the pale haze at the horizon,
  not the ground behind: every tower afar was a solid pale blue slab two to four pixels wide and a corridor a curtain
  of them (over Redondo from 150 m a slab measured (138, 155, 170) in sRGB where the town behind is (126, 134, 142); the
  same towers with the mix left out (119, 130, 142)). Communication towers (`communication_tower`) are triangular
  lattice masts as tall as mapped (else 60 m), banded red and white from 60 m. Every line passing through a chunk is
  handed in whole, so every chunk it crosses places its towers alike. `NAVAL_POWER=off` leaves them out.

  **Source.** `naval-overture` reads Overture's GeoParquet on its public bucket (Amazon S3, us-west-2, anonymous) without
  downloading the files: the newest release from the bucket's listing, its 64 or so files' footers by range (about
  330 KB each, 21 MB in all, read once a release), then a box's row groups, only those whose `bbox` columns' statistics
  meet it and only the four columns used (`geometry`, `subtype`, `class`, `height`), each one ranged request, eight at
  once. It uses the `parquet` crate's low-level reader with zstd its only codec (no Arrow): about 20 s of compiling,
  once (parquet 14 s, its chrono and num-bigint 6 s), and about 1 MB of the client's binary. `theatre::power` reads
  half-degree cells on its own worker (`theatre-power`) and keeps them under the user data directory's
  `overture-cache/<release>/` (`files.txt`, the footers, a `ready` marker once they are all kept,
  `cells/<lat>_<lon>.bin`), so a scenario's region is read once and every scenario there shares it; offline it uses
  the newest release kept whole that opens, and a newer release's folder replaces the older's. The cells' longitudes
  are unwrapped about the theatre's, so a theatre on the date line reads the cells either side. A chunk made while its
  cells were being read is made again once they are (`power::epoch`); one whose cell could not be read is made again a
  minute later, the cell asked for again after ten.
  `theatre_state`'s `infra.power` reports the release, the cells and the last read. The data is OpenStreetMap's under
  the ODbL 1.0, through the Overture Maps Foundation; the game credits `naval_overture::ATTRIBUTION`: "Power lines and
  towers: Overture Maps Foundation, base theme (© OpenStreetMap contributors, ODbL 1.0)".
- **Levels and cost.** `InfraSettings::HIGH` (High and Ultra): chunks within 1.5 km made near (boats, hangers),
  their containers one by one and boats in full within 900 m of the eye (`close_m`; past it each row's run is a box
  and each boat a stand-in), everything to 8 km, into the sun's near and wide maps. `MEDIUM`: no near detail (no boats,
  the stacks as rows), to 4 km, no shadows. A mesh's index ranges lie in the order `Decal`, `Far`, `Structure`,
  `Detail`, `Near`, so each choice is one draw call or two a chunk. A full terminal chunk is about 2 MB and 35,000
  triangles. Measured 2026-10-08 on Ultra by taking the layer out and putting it back (16 interleaved rounds of 40
  frames, the GPU shared with other captures): over the port from 400 m (33 chunks, 495,000 triangles drawn) the
  scene's opaque pass cost 0.29 ms more at p10; low in a terminal it cost 1.9 ms less (the stacks hide the city
  behind); over King Harbor's marina the opaque pass was level and the water pass 0.27 ms dearer. Medium: 0.03 ms.
  The airfields are a few thousand triangles: over LAX from 800 m (27,500 triangles drawn with the bridges) the opaque
  pass cost 0.07 ms more at p10 and 0.03 ms less at p50; over Henderson Field (1,800) the difference was within the
  noise. The power lines: over the hills above Brentwood (925 towers held, 16,000 triangles drawn) the opaque pass was
  level at p10 (+0.45 ms at p50) and the frame 0.10 ms dearer at p10; along the corridor by the Baldwin Hills (2,471
  towers held, 130,000 triangles drawn with a few bridges) the opaque pass was 0.06 ms cheaper at p10, within the noise;
  on Medium there (towers as lines, no wires) 0.06 ms cheaper. The lines blended (2026-10-10, `naval-client`'s
  `--test all -- theatre_power_lines::` at 1280 × 720, 9 interleaved rounds of 40 frames with the layer and without it, old and
  new code measured back to back): over Redondo Beach from 150 m (2,332 towers held, 46 chunks drawn) the layer's GPU
  time was 0.19 ms against 0.20 ms opaque, 0.12 to 0.13 ms of it the blended lines' pass; along a line from 45 m and
  over its corridor from 3 km it was within the noise (-0.08 to +0.16 ms, against +0.19 and +0.37 ms opaque). A line
  is two triangles a segment where the opaque one took four (both faces), so 32% fewer triangles are drawn there
  (176,000 to 120,000) and the layer holds 0.9 MB less. Lit at every pixel the blended lines cost 0.86 ms over Redondo:
  blending hides no fragment, so a tower afar's overlapping members are each shaded, the sun's shadow maps 0.45 ms of
  it; lit at their vertices (`vs_infra_line`) they cost what their vertices do. While the eye travels, the motion pass
  draws the lines of the chunks near enough: 0.12 ms over Redondo with all of them in it, nothing while the eye holds
  still or turns. The frame times, CPU-bound with other builds running (load 13 to 95), moved by 0.1 to 1.2 ms either
  way. A cell over Los Angeles is 0.5 to 1.4 MB on
  disk, read from the bucket in under a second once the footers are kept; the four the region reads hold 4,637 lines,
  and 226 masts are kept in memory (towers with a mapped height and the communication masts). The chunks are made on
  the `theatre-infra` worker, 8 to a batch in 85 to 180 ms.

The tests: `infra`'s unit tests (the yard's test, stacks off the apron, roads and outlines; boats in the slips and
the fairway; a sea span's clearance, towers and channel, a creek's low bridge, an outline's width; a runway's
designators and its area laid once across its chunks, the airfield's heights; a power line's towers, their mapped
height and its wires' sag, made far, neighbouring towers' wires alike, one tower where lines meet, a mast's height)
and `tests/infra.rs` (the shaders for Metal, DirectX and Vulkan; a chunk drawn where it stands, made far and removed:
`.build/native/terrain/infra-gpu-test.png`; a runway painted and handed to its lights:
`.build/native/terrain/infra-runway-test.png`; a power line's towers and its wires against the sky, darker than it:
`.build/native/terrain/infra-power-test.png`; towers afar over a paved chunk faint and no bluer than the ground behind
them: `.build/native/terrain/infra-towers-afar-test.png`). `naval-client`'s `--test all -- theatre_power_lines::` (ignored,
network; the player's California scenario, `NAVAL_PYLON_LABEL`, `NAVAL_PYLON_VIEWS`, `NAVAL_PYLON_BARE`,
`NAVAL_PYLON_MOVING`, `NAVAL_PYLON_ROUNDS`) captures Redondo Beach from 150 m, a tower close by, a line running away
from the eye and its corridor from 3 km into `.build/pylons/<label>/` (each also without the layer, and with the eye
carried sideways half a metre a frame) and times each with the layer and without it (`summary-<mode>.json`).
`naval-overture`'s tests write GeoParquet in memory and read a box
from it (only its row group's four column chunks requested, the cell kept and read back) and parse the bucket's
listings.

**Open.** No cable-stayed or arch bridges (the Gerald Desmond's replacement stands as a girder bridge); a bridge
longer than its reach may be cut differently by two chunks; the yards are a guess from land use and depth (an
industrial waterfront with deep water but no containers gets stacks). Runways are straight from end to end and their thresholds are the line's ends (no displaced thresholds, no
blast pads); no taxiway signs, hold lines or apron stands; a runway longer than 4.5 km may miss its far end. Power
lines stand on towers at their mapped vertices only (a line traced with few vertices has long spans); where two ways
of a line meet at an angle the tower is turned for one of them; lines over open water (a chunk with no land near) are
not drawn; no substations, and lone poles without a mapped line are left out.

## Harbour boats

Every boat the land draws is a marina's yacht (`naval_terrain::infra::marina`, drawn in each chunk's infra mesh by
`infra::build_chunk`). They are scenery: the simulation never knows of them, and no ship preset plays them.

- **Where.** Within 900 m of a harbour point (`poi` class `harbor`, `MARINA_REACH_M`), across every pier line 20 m or
  longer (a narrow pier area's long axis likewise), both sides, bows in, about four in five slips taken (`TAKEN`);
  never across a dock or off the water, and no longer than the fairway leaves (two boats share it).
- **What.** Sailing yachts (8 to 16 m: hull, deck, cabin with its windows, mast and boom with its cover) and motor
  yachts (9 to 20 m, a flybridge from 12 m). Past the close range each is a stand-in (the hull's outline, the deck, the
  cabin).
- **In the period.** A past period (`Period::is_past`) keeps 15% of them (`infra::PERIOD_BOATS`), chosen by a hash of
  each boat's place and its chunk, so the same boats stay. They are today's yachts thinned, not period working craft:
  marina boats are not told from working craft, and no launch, lighter or sampan is drawn.
- **Levels.** `InfraSettings::HIGH` (High and Ultra) only, within the near range (chunks made near within 1.5 km), in
  full within 900 m of the eye (`close_m`); `MEDIUM` draws none ("Levels and cost", above).
- **At night.** Their cabins are lit; under a dimout at 15% of their usual share, under a blackout at 3% (night.md, "Night
  lights").
- **Tests.** `infra`'s unit tests put boats in the slips and keep them out of the fairway.
- **Open.** Boats stand only within a harbour point's reach and the near range.

## Generated bases

Hundreds of wartime air and naval bases are wanted; only about twenty famous
ones will be traced (Henderson Field stays as the period draws it, `infra::period::FIELDS`). The rest need only stand at
about the right place and look right. `naval_terrain::infra::bases` generates them, presentation only: a catalog, a
site fitted to the ground, its nation's template laid out on it, drawn by the infrastructure's chunks, its aircraft by
the aircraft layer. Only in a past period (Graphics → Buildings Period) whose year lies within a base's `from`-`to`;
`Period::Today` draws none.

```rust
let entry = &bases::catalog()[i];                                  // assets/bases/bases.v1.json, embedded
let input = SiteInput { seed: entry.seed(), size: entry.size, template: entry.template(),   // by its nation
    heading_hint: entry.heading_deg, ground: &|p| h(p), water: &|p| wet(p), others: &neighbours, runway: todays };
let site = bases::site::fit(&input)?;                              // metres about its point
let plan = bases::lay_out(site, &input);                           // once per base; plan.clearing, plan.parked
bases::draw::draw(&mut mesh, &Placed { plan: &plan, origin }, &ground);   // each chunk: the pieces it holds
land.set_clearings(&[ClearingGroup { min, max, capsules }], 10.0);  // no tree on it; its ground bare or worn
scene.air.set_parked(&[ParkedPlane { model, at, heading, number }]);       // its aircraft, near the camera
```

- **The catalog** (`assets/bases/bases.v1.json`): id, name, kind (`airfield`), nation (`US`, `Japan`, `UK` or
  `Germany`: it picks the template, `Template::of_nation`), latitude and longitude (roughly right, from Wikipedia, Pacific Wrecks and today's
  map), the years it stood (`from`, `to`, both drawn), its size (`fighter-strip` or `bomber-field`; the runway's length
  and width are the template's, `Template::runway`), an optional runway heading hint and a source. It holds six US
  fields of 1943 about Guadalcanal and the Russells (Fighter One, Fighter Two (Kukum), Carney and Koli Fields, Sunlight
  and Renard Fields on Banika) and seven Japanese navy fields of 1942-1943 in the Solomons: Munda, Vila on Kolombangara,
  Ballale in the Shortlands, Kahili and its satellite Kara by Buin, Buka and Bonis either side of Buka Passage (Rabaul
  and Truk are left for tracing); seven RAF stations of 1943 (Lincolnshire's Class A bomber stations East Kirkby,
  Spilsby, Kelstern and Ludford Magna; Norfolk's grass fighter stations Coltishall and Ludham; Kaldaðarnes in Iceland,
  `surface: gravel`) and eight Luftwaffe fields (the Fliegerhorste Leeuwarden, Aalborg, Karup, Jever and Sola; the
  field strips Bergen in North Holland, Husum and Kjevik). A European template's size picks its kind: an RAF
  `bomber-field` a Class A station, `fighter-strip` a grass fighter station; a Luftwaffe `bomber-field` a Fliegerhorst,
  `fighter-strip` a Feldflugplatz. Metheringham and Bardney, first chosen, lie 45 km and more from the sea: no scene's
  land window (96 km about a force at sea, its edge settling under the sea) reaches them, so Spilsby and Kelstern,
  nearer the coast, stand in their place. Munda is a famous field (`FIELDS`, today's runway from 1944, as the American field);
  in 1942 and 1943 the catalog's Japanese field stands there: a famous field holding a base's own point is neither its
  neighbour nor kept from its runway. The American Munda of August 1943 on is not generated (the period is a year). It
  is plain JSON for the scenario editor and simulation to read later; nothing but the client reads it now.
- **The site** (`site::fit`). Where the map has a runway within the base's search (1.5 km a fighter strip, 2 km a bomber
  field; not a famous field's but its own, judged by its middle) the base lies along it, cut to the template's length
  about its middle (Renard Field along Yandina's; Munda, Vila, Ballale, Kara and Buka along theirs). Otherwise centres
  125 m apart over the search and headings 15° apart, the best four refined to 25 m and 2.5°: the runway and 50 m past
  its ends and shoulders dry (no water, 2 m or more above the sea; a temperate base's ground may lie 12 m below it, a
  polder behind its dikes, the map's water alone keeping it off the sea: Bergen's point lies 5.9 m below), no steeper
  than 3% along and 5% across, the ground no more than 3 m off a straight line along it, and every point nearer its own base's point than any other base's or
  famous field's by 60 m. The cost: the ground's roughness, grade and cross slope, the dry land beside it (the dispersal
  goes on the drier side), its distance from the point and the heading's from the hint, with a tie-break hashed from the
  base's id, so a base is the same on every visit. The water is the window's heights under half a metre (12 m under the
  sea for a temperate base) and OpenStreetMap's rivers, canals, streams and water areas, on a 20 m grid about it
  (`theatre::bases`'s `WaterMask`); not its drains and ditches, which an airfield filled in or piped (the fens' and the
  polders' every field has them, and none of Lincolnshire's bases fitted). A base no site fits is not fitted again for
  a minute (each chunk about it asked, and each laid it out again).
- **The American template** (`strip::us_pacific_strip`, `us-pacific-strip`; 4,000 ft of Marston mat, 1,220 m, a
  fighter strip; 6,500 ft of coral, 1,980 m, a bomber field; both 150 ft wide), in metres about the base's point, every
  choice from its seed, every piece on dry, fair ground clear of the strips, every road joining another road, a taxiway
  or what it serves:
  - the runway: crushed coral with 45 m overruns and shoulders (7 m beside a fighter strip's mat, 12 m a bomber
    field's), the Marston mat over a fighter strip, dust beyond each end;
  - a parallel taxiway 18-24 m wide, each end joining the runway its own way, one to three exits between (square or
    angled either way);
  - four to six dispersal loops beyond it, wobbling (their depth and line swell and shake, a quarter of them spurs with
    a hammerhead askew); hardstands along them, rough rectangles pushed out by a bulldozer (a fighter's 16-21 m wide, a
    dive bomber's or an Avenger's up to 26 m), each turned up to 28° off square to its loop; most in a U of earth
    1.9-3.1 m high, its arms of their own lengths;
  - aircraft on 60-80% of the hardstands, nose out of the U (`Parked`): a fighter strip one fighter type mostly and
    another (the Corsair, the Wildcat, the Hellcat), a bomber field Dauntlesses and Avengers with a fighter type; a
    warm-up apron between the runway and the taxiway near one end, two to four on it angled to the runway; a service
    apron at the camp with two or three and its shop;
  - a perimeter road turning in to the runway's ends, a wooden control tower with its operations hut and vehicles;
  - a camp (300 × 200 m, 380 × 250 m a bomber field's) on whichever of six places is dry and level: ten to thirteen
    clusters scattered over it (none within 46 m of another), each turned its own way: company streets (two rows of five
    to eight pyramidal tents facing across a street trodden to earth with a duckboard down it, a row behind now and then,
    a few tents larger, a few askew, some walls rolled up, a ward tent across the street's head, a latrine behind, a
    sandbag wall at its foot), Quonsets side by side, mess halls with their water tanks, a motor pool's lot with trucks,
    jeeps and weapons carriers parked loosely, stores of crates and drums; jeeps, trucks, crates and drums about them;
    a spine road from the road it meets in among them, a road from each to it, worn short cuts between neighbours,
    telephone poles and their wires beside the spine, a water tower on a braced timber stand, vehicles on the roads.
    The land's own trees and ground stand between the clusters (in Lunga's plantations, the land's coconut palms in
    their rows);
  - beyond the camp the fuel dumps (blocks of standing drums) and a battery of four 90 mm guns in sandbagged pits, 40 mm
    pits off the runway; tyre tracks worn between the loops, from the dispersal to the runway's ends and as short cuts.

  A fighter strip lays out to 170-190 pieces (Fighter One's 70 tents) with about 20 aircraft, a bomber field 240-250
  with about 30.
- **The Japanese template** (`ijn::ijn_solomons_field`, `ijn-solomons-field`; a Japanese navy field of 1943: 1,100 m
  a fighter strip, 1,350 m a bomber field, 56-60 m wide; Kara's was 4,000 by 280 ft cleared, Kahili's 4,700 by 300).
  The same pieces, layout helpers and rules as the American, laid out otherwise:
  - one wide runway of coral and gravel rolled by hand, its ends rough, no mat, cleared only some 24 m beside it and 60
    m past its ends (the jungle close);
  - two taxiways of packed earth (11-13 m wide) leaving its edge near its ends on the dispersal's side, a third at a
    bomber field (and some fighter strips) from the other side past an end, each off at an angle and then winding on
    (26 m steps, wandering 10-22° a step, steered back to its own distance off the runway, 60-190 m) into the jungle for
    420-800 m, kept off the other strips by 25 m: no loops, no parallel taxiway;
  - horseshoe revetments of earth dug either side of them, their mouths on the taxiway (the inside 8.5-11 m across the
    middle for the field's widest type, banks 2.6-3.6 m high, arms 0.9-1.3 of the inside), nearly half of them under the
    trees (cleared only within their floor, the banks heaped about the trees' feet, the crowns over the plane), the
    rest cut clear to their banks' feet; a fifth under netting on poles, some planes strewn with palm fronds;
  - aircraft in 60-80% of them, nose out (Zeros at a fighter field, the A6M2 and some A6M5s; Vals, Kates and Zeros at a
    bomber field; there is no Betty in the client), three to six more lined up at the runway's edge by one end, a fuel
    bowser by them;
  - a wooden lookout tower (12-15 m) by the runway's middle on the camp's side (else the other), the headquarters (a
    long barracks) behind it with a thatched office hut, a road to the runway; a wooden hangar open toward the runway
    at a bomber field, its apron of earth;
  - the camp along a road winding 380-680 m from the headquarters away from the runway into the trees (else from the
    runway's edge on either side): long, low, dark plank barracks (18-32 m) with board or tar-paper roofs, thatched huts
    of woven palm on stilts, small sheds, each set off the road at its own angle on either side, a gap here and there, a
    path to some doors, a truck or two on the road; the land's trees all about them;
  - fuel drums in small lots dug in beside the taxiways and roads (a low bank of earth about each), hand carts by some;
    25 mm triple mounts in earth pits about the runway and past its ends, three or four heavier guns (a navy 12 cm) in
    rings of earth in a crescent some 300 m off.

  A field lays out to 60-100 pieces (14-25 huts) with 9-27 aircraft in 14-29 revetments that hold one.
- **The British template** (`raf::raf_station`, `raf-station`), in its farmland, the airfield's ground mown grass
  (`Plan::temperate`), all else cleared only where it stands:
  - a bomber station is the Class A standard: three runways of dark tarmac 46 m wide in a triangle, the main 1,830 m
    on the site and two of 1,280 m along the triangle's other sides (1,000 m each), each crossing the others 140 m past
    the corners, the apex on whichever side they lie dry and fair (else the main runway alone); the grass mown inside
    and about them; a perimeter track 15 m wide through the six ends, round the outside, its corners rounded;
  - 30-36 hardstands off the track, outward, none within 110 m of a runway's end: round frying pans 38 m across on a
    stub, and spectacles (two side by side on one stub);
  - two T2 hangars (73 by 35 m, steel walls, a low gable, doors across both ends) just outside the track on the side
    away from the apex, their tarmac aprons to it; the watch office between the track and the main runway (two storeys
    rendered, windows wide, a railed balcony across its front, a rail round its roof), a van and a fire tender by it;
  - the technical site beyond the hangars: two roads with Nissen huts (16 ft across, tarred dark) and brick workshops
    under slate along them, a water tower, fuel tanks, lorries and a bowser; three to five accommodation sites out
    among the fields 900-1,600 m off, Nissen huts in two or three rows with a brick ablutions block, a lane back to the
    roads; the bomb dump past the apex, a loop of lane with bays between earth traverses, bombs stacked in them; Bofors
    pits and pillboxes about the track;
  - a fighter station is grass: the field (1,300 by 900 m, its rim pushed out here and there) with one or two landing
    runs mown paler across it (gravel at Kaldaðarnes), a tarmac perimeter track round it, E-shaped blast pens (two
    bays, earth walls) and blister hangars off the track by turns, a smaller watch office, Nissen huts in groups beyond.

  A Class A station lays out to 90-105 pieces on 25-35 hardstands, a fighter station to some 90 with 28-30 bays.
- **The German template** (`luftwaffe::luftwaffe_field`, `luftwaffe-field`), likewise in farmland:
  - a Fliegerhorst: a broad field of grass (its rim pushed out here and there), one concrete runway in 5 m slabs 60 m
    wide and 1,500 m long, half the time a second of 1,150-1,300 m crossing it near an end at 50-80°; hedgerows painted
    across the grass along two old field lines at right angles (dark, broken, stopping at the paving); concrete taxiways
    18 m wide winding off into the country (the Japanese template's `winding`) to dispersal boxes either side, U walls
    of grassed earth, the larger (26-32 m inside) for bombers, a third under the trees, a few netted; three to five big
    hangars in a row with their concrete apron and a taxiway to the runway, a control building past their row (two
    storeys of brick, a glazed room on its roof), lorries and a bowser; brick technical buildings under red tiles along
    a lane behind; the barracks built as a village of farmhouses (render or brick, steep red tiled roofs, each at its
    own angle) along a bending lane further off; 2 cm Flak in earth rings about the field and four 8.8 cm guns in their
    rings off to one side;
  - a Feldflugplatz: a mown run across a grass field, small boxes at its edge (half under the trees), their ways out
    across the grass trodden (soft footprints: a tyre track's tan strip read as pasted on the grass), a few wooden huts and tents and fuel drums at one end, painted hedges, a little 2 cm Flak.

  A Fliegerhorst lays out to some 40-70 pieces with 15-30 boxes, a field strip to 25-30.
- **No aircraft at the European fields.** The client has no RAF or Luftwaffe type: Lancaster, Halifax, Stirling,
  Wellington, Spitfire, Hurricane, Typhoon, Mosquito, Beaufighter, Hudson and Swordfish; Bf 109, Fw 190, Bf 110, Ju 88,
  Ju 87, He 111, Do 217, Ju 52 and Fw 200 are wanted. The stands and boxes stand empty (`Layout::park` parks none
  without types). The Walrus, the one British type, is a flying boat whose model has no wheels: `resting_pose` now
  refuses an airframe whose lowest points either side stand no lower than its belly between them (its hull's chines
  above its keel), and it is not parked.
- **Drawn** by the chunks (`infra::build_chunk`'s `bases`, from `theatre::infra`'s `chunk_mesh`): each standing piece
  by the chunk holding its middle, each strip's stretch by the chunk holding the stretch's middle (16 m stretches on a
  straight strip, so the mat's planks start alike), each pad by its middle's, so a base split across chunks is drawn
  once. Coral and the mat are decals (`Group::Decal`; materials `Coral` and `Marston`): the mat over the coral, over the
  taxiways, the loops, the hardstands and lots, the roads, the tracks, the dust and a camp's trodden streets. They are
  pulled toward the eye by the runways' share of the distance but by no more than a metre within 2 km: pulled 4 m at a
  kilometre, as the runways are, the coral hid every plane parked on it. A pad is a fan in two rings (so a wide lot
  follows the ground), its `uv.y` how far in from its rim. The rest stands in `Group::Structure` at every range and
  casts: `Earth` (berms; sandbags with `param` 1; a Japanese gun's ring, sloped outside; netting, mottled and green
  where it faces up), `Iron` (corrugated, its ribs 76 mm apart resolved close up), `Canvas` (tents, ward tents,
  thatch and woven palm, palm fronds), `Wood` (boards, crates, duckboards, poles, the water tower, barracks' walls and
  board roofs, the hangar, the lookout, carts), `Drums` (a block's drum heads and columns, each drum's colour by its
  place), `Paint` and `Glass`; the poles' wires are lines. A berm is a bank bulldozed up along the U's back and arms
  (a horseshoe's back a half round, its line and height rougher), its section a trapezoid (sloped flanks, a crest 1.1
  m wide), each arm's end ramping down into the ground, cut every 2.5 m and shaded smooth along its length; its foot
  takes the ground's earth. A barracks stands on a skirt of planks with shutters propped open down its sides and a door
  at each end, a thatched hut on six stilts and a deck, both under gabled roofs that overhang (`gable_roof`); the
  hangar's walls are open at its end, the dark inside there. The lookout is four slim braced legs, a railed platform, a
  hipped thatch roof on posts and a ladder. In the shader (`infra.wgsl`) the American runway's coral is near white,
  patched and mended in rectangles, worn darker down its middle and darker still where wheels touch down (100-550 m in
  from either end, from its length in the colour's third byte), its shoulders rougher and yellower; the mat is planks
  15 in by 10 ft across the strip, staggered, three rows of holes each with the coral beneath (resolved only within a
  few metres, its seams within some tens, past that its mean), rusted in patches, a mat laid new here and there,
  rubber-dark where wheels touch down; taxiways, hardstands and roads are dirtier tan-grey coral with earth tracked over
  them, a road's ruts, a hardstand's oil; tracks are two ruts in the earth. A hand-built runway (look 7) is greyer coral
  rolled in strips a shade apart, the earth through it in patches, craters filled in ragged patches of darker earth or
  fresher coral, its shoulders earthier and its edge broken 2.4 times as far; packed earth (look 8) has coral grit and
  the wheels' broken paths. Europe's: tarmac (look 9, a pad 13) dark, its aggregate paler where wheels run, mended
  with fresh tar in patches of their own sizes, its edge crisp; concrete (10, a pad 14) pale in 5 m slabs each a shade
  off, its joints dark close up, tyre-marked down a runway's middle; a hedgerow painted on the grass (11) dark and
  broken; a landing run (12) mown a shade paler in broad stripes. A European look's edge frays into grass, not earth.
  The European pieces: a T2 (steel walls, doors across both ends, a low gable), a blister hangar (a steel arch to the
  ground, open, the dark inside its ends), brick buildings and farmhouses (`Concrete` walls in brick or render,
  windows, a door, a gabled roof whose boards stand for slates or red tiles, a farmhouse's steep), a watch office or a
  control building (two storeys under a flat roof, its windows in bands; the watch office's railed balcony and roof,
  the control building's glazed room on top), a Luftwaffe hall (rendered walls, its doors across its long side, a row
  of high windows at its back, a broad gable), banks of earth (a dump's traverses, a pen's walls: a revetment's section
  along a line, `earthwork`), a hexagonal concrete pillbox with its loopholes, a 2 cm Flak 38 and an 8.8 cm Flak 36 on
  its cruciform outriggers in rings of earth. Every edge frays unevenly into the earth beside it; a road's, a track's, a street's, dust's
  and the Japanese strips' edges break off into the land itself (discarded where they have frayed most of the way: the
  land beside them is the land's own, among the trees, and an opaque edge of earth met it in a hard line). No runway
  lights (the blackout): a base hands no `RunwaySpan`.
- **Aircraft** (`theatre::bases::parked`, `air::BattleAir::set_parked`): the planes of the bases within 2 km of the
  camera are handed to the aircraft layer with the battle's own (again only when the camera moves 20 m), as
  `PlaneDraw`s at rest on their three wheels, gear down, wings spread, propellers still, side numbers painted, no
  distant silhouette. Each type's rest is measured from its own finest level as it loads
  (`naval_aircraft::state::resting_pose`: the gear posed down, the lowest points of the main legs and of the tail
  brought level by pitching it nose up; the Corsair 0.227 rad and 1.70 m, the Hellcat 0.204 rad and 1.58 m, the
  Wildcat, Dauntless and Avenger within 0.01 of the catalog's poses). The Val's gear is fixed (no gear joints): it rests
  on its airframe's lowest points either side forward of its middle, its propeller left out. `parked_on` tilts it with
  the ground's slope under it and sets its height so its three wheels lie on the ground's plane. (The simulation's
  catalog carries ground poses only for its carrier types: the Corsair and the Hellcat had none, stood at the ground with
  no pitch, and sank to their wings.) The layer draws them as every airframe: instanced by type and level, its three
  levels by size on screen, casting. A type not loaded is read on its own thread as a packet's would be. No new mesh.
- **Kept clear.** OpenStreetMap's airfields keep their trees off only where the canopy map measures them bare (trees.md,
  "Trees where real trees stand"); a generated base has no such ground. The land's clearing (`set_clearing`, a theatre
  airfield's round) became a list as well: `TerrainLayer::set_clearings` takes up to 64 groups and 640 capsules. A
  base's `Plan::clearing` is handed in 700 m tiles (`theatre::bases::clearings`), each tile's group the capsules
  reaching into it, so a land fragment tests only the few about it. Hard capsules (an American airfield's runway, the
  ground between it and its taxiway, the taxiway, each loop in thirds; a Japanese field's runway alone) clear it to bare
  earth; soft ones (handed with their radius negative) only where something stands among the trees: a camp's tent rows
  and streets, each hut, crate, drum block, vehicle, tank, pit and pole not already in the clearing (`strip`'s
  `footprints`), the roads (their verges 4 m), the dumps and the battery's pits and paths, a Japanese field's taxiways,
  revetments and camp, a European base's all but its airfield (an American field 175-245 capsules, a Japanese 100-155,
  a European 60-200). A temperate base's groups are flagged (bit 16 of their count, `ClearingGroup::temperate`). `bindings.wgsl`'s `clearing_at` gives
  how far inside (1 within a capsule's radius, feathering out over the edge, 10 m, the outline wandering out past the
  radius by up to twice that in three octaves, less for a small capsule, so no outline is a capsule's), how deep, and
  how far into a hard one; bare earth wins where they overlap. No tree, cover or forest shade stands in either. The land
  (`terrain.wgsl`) in a hard clearing is bare earth and coral dust worn by wheels and boots, oil-dark here and there,
  kunai left toward the clearing's edge and in tufts where nothing drives; in a soft one it is the land's own ground at
  its edge, worn through in patches, trodden to earth with grass in tufts from about 5 m in (so a road's or a
  revetment's edge meets earth). In a temperate base's hard clearing the ground is the land's own colour a touch
  greener, evened an eighth of the way toward a mown green, so its fields' and hedges' marks still show through and
  its edge melts into the farmland (a fixed green laid at 62% made Leeuwarden's field a pale oval pasted on the
  polders, at 45% and at a quarter Kaldaðarnes's a bright disc on Iceland's dark grass), most of its relief and shading
  kept (its
  occlusion and bump flattened by a fifth only: flattened as a tropical base's, the disc stayed); in a soft one, grass
  trodden muddy here and there. (Round three cleared the whole camp, grassed it and planted rows of its own palms: from
  400 m it was a light-green rounded rectangle pasted on the land, its palms a lighter model on a perfect grid, its dumps
  and battery on hard-edged beige pads; now the land's trees, palms and ground photo stand between what stands, and the
  dumps and battery have worn ground only.) (Wandering inward too, as first tried, let trees thin back onto berms' outer
  flanks and beside a taxiway's exit at Koli. A measured crown's own tree was let stand by the clearing at its slot's
  foot, which in an air window's 35-70 m slot may lie outside a clearing its crown is in, and a land-cover tree grown to
  fill such a slot spread its crown onto the berms: `tree_place.wgsl` holds a measured crown to the clearing where it
  stands and tests every slot against the clearing reaching half a slot further, `cleared_within`.) `theatre::infra`'s
  feed hands the nearest bases' within the infrastructure's range and 4 km more each frame; the trees are placed again
  only when they change. The buildings' infill and today's footprints keep off the clearing in the period
  (`theatre::bases::clear`, from `theatre::buildings`), and today's aeroway within a base's reach is left out
  (`bases::suppress`; in the Solomons `FIELDLESS` already leaves it out, and Munda's `FIELDS` runway of 1943;
  Leeuwarden's, Aalborg's, Karup's, Sola's and Kjevik's runways of today).
- **Made** on the infrastructure's worker, the first time a chunk within a base's reach is made (the buildings' worker
  likewise when it needs the clearing): its map's tiles about it (zoom 14), the water grid, the site and the plan, kept
  for the scenario's frame (`theatre::bases::made`; again after a minute if a tile was missing).

**Cost** (Apple M5 Pro, Metal, `test-fast`, 1280 × 720, High, 2026-10-18, other agents' builds running). Without its
planted palms an American fighter strip draws about 14,000 triangles over all its chunks in full detail (6,200 of them
on the ground; 65,000 with the palms in round three), a Japanese field 10,000-17,700 (4,200-8,000 on the ground). Timed
three ways interleaved in 7 rounds of 40 frames, from 400 m, in three runs: over Fighter One (four bases about) the frame
median rose 0.11-0.27 ms with the base and its 20 parked aircraft, the GPU's total 0.00-0.03 ms (the aircraft's share
0.26-0.33 ms, on the CPU's side; with the planted palms the base cost 0.81 ms); over Munda 0.25 ms with 12 aircraft
(the GPU 0.09 ms), 0.78 ms in an earlier layout with 20 in view (the GPU 0.79 ms, the opaque pass 0.24 ms of it: the
planes); over East Kirkby (a Class A station, 12,200 triangles, 6,700 on the ground, no aircraft) 0.12 ms and over
Leeuwarden (a Fliegerhorst, 15,800 and 7,800) 0.21 ms, the GPU's total level within the noise both times (the tree
placement's 0.96 ms over East Kirkby is its placing again each time the clearings were handed, not every frame). The
clearing's capsules (an American field's 175-245 now, from 80-135) cost the tree placement 0.06-0.13 ms
over Fighter One. An earlier timing that handed the bases' clearings and aircraft every frame, under heavier load, read
+4.4 ms; the feed hands them only when the camera moves 20 m or a base is laid out. The fit and the plan take
milliseconds; a base's first laying out waits on its map's tiles.

The tests: `bases`' unit tests (`-p naval-terrain --lib bases`: the catalog parses, its ids its own, none on Henderson
Field, every nation's a template; a site on dry, fair ground along a synthetic coast, the same every time, none where
all is water, nearer its own point than another's, along today's runway cut to length; an American base laid out alike
for its seed and otherwise for another's, every piece and plane dry and in the clearing, no tent on a strip, most
hardstands holding a plane of the base's types off the runway and not all facing one way, every road ending at another,
a taxiway or what it serves; a Japanese field likewise, one wide runway of coral and no mat, no tent nor Quonset,
barracks, thatch, a hangar at a bomber field only, a lookout, 25 mm mounts, two or more winding taxiways with horseshoe
revetments on them, camouflage, most revetments holding a plane of its types and none on the runway, only the runway
cleared bare, its huts at three or more angles; an RAF station likewise on a synthetic farmland (`raf::tests::
farmland`), a Class A's three tarmac runways, two T2s, 24 or more frying pans, a bomb dump's traverses, a fighter
station's mown run, blisters and E pens, a watch office, ten or more Nissen huts, pillboxes and pits, nothing parked; a
Luftwaffe field likewise, painted hedges, U boxes, 2 cm Flak, a Fliegerhorst's concrete runway, halls, a village of
farmhouses, a control building, four 8.8 cm guns and bombers' boxes, a field strip's mown run and no concrete; every
template fitted to its own runway's length and width, along today's runway too; a base split across chunks drawn once,
its runway's coral laid once to a percent, a Japanese field's too and each of its huts drawn; a Quonset's faces out),
`--test all -- backends` (the land's shaders, and now the infrastructure's, translated for Metal, DirectX and Vulkan:
`patch`, a WGSL keyword, once failed only at the client's start), and `naval-aircraft`'s `--test all -- parked_types_rest_on_their_wheels` (each parked
type's resting pose from its model, the Zeros', the Val's and the Kate's too: nose up, its main wheels and tail wheel
within 0.1 m of level or sloping ground, nothing of it below; the Walrus none). `naval-client`'s `--test all -- theatre_bases::`
(ignored, network and the land store; `NAVAL_LAND_DIR` a copy of it when another client holds it) loads a copy of
Eastern Solomons, Buildings `period`, dated 15 November 1943, 10:00, off Lunga and off the Russells, or dated 15 June
1943 with its frame moved to New Georgia (Munda, Vila), the Shortlands (Ballale, Kahili, Kara) or Buka (Buka, Bonis),
or dated 15 June 1943, 11:00 on double summer time, its frame moved off Lincolnshire, Norfolk or each European field
(a copy of Denmark Strait off Iceland for Kaldaðarnes; a scene's land is cut about the scene, which is built at the
frame's origin while a new place's coast is baked, so a base 40 km or more from it never laid out; the land store's
4 GB bound evicts a region's squares, so `NAVAL_LAND_DIR` is best a store of its own for the round), waits up to four
minutes for each base to be laid out (Europe's map tiles are full), and captures each base obliquely from 1,500 m and 400 m, in a low pass at 60 m along its runway and from 8 m up 3 km
offshore into `.build/bases/stills/` (`NAVAL_BASES_CLOSE` adds the camp from 120 m and a revetment holding a plane
from 25 m, else an empty box, a pen's wall or a hardstand; the camp the thickest cluster of huts and tents within 1.2
km), checks every American berm lies in the clearing, and writes what each holds, its triangles and the aircraft
layer's last frame in `bases.json` and the frame's cost over Fighter One, Munda, East Kirkby and Leeuwarden in
`cost-<id>.json`.

**Open.** Four templates (no seaplane or naval bases, no Italian or Soviet fields, no American Munda after August
1943: the period is a year); no RAF or Luftwaffe aircraft in the client (above), so the European stands are empty; a
base deep inland (Metheringham, Bardney) shows only in a scene whose land reaches it; the scenario editor and the simulation do not read the catalog; no chocks under the
wheels; the coral is a decal over the land, which is not levelled under it (a runway on a gentle slope follows it);
past 2 km a base shows no aircraft; seen low from the runway's side, the planes in a U opening away are hidden by its
back; the sea views show only the beach's treeline. Close up (25 m) the land's own ground photo beside a soft clearing
is its canopy seen from above, flattened, and a Japanese taxiway's edge breaks off into it pixel by pixel: soft only
from some tens of metres up. The netting is an opaque sheet, not a mesh.

History (the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
