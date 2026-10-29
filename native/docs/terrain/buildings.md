# Buildings

Towns stand as buildings, as flight simulators draw them, the modern world included. `naval_terrain::buildings` is
portable (no file, no network: the host hands it footprints and the ground) and holds the meshes and the layer; the
host's side (the footprints' source, the infill's cover, the chunks about the camera) is `naval-client`'s
`theatre::buildings` (docs/archive/native-theatre-history.md, "Buildings").

```rust
tags::merge_outlines(&mut real, &tags.outlines);  // OSM's heights, colours and the buildings Microsoft misses
let mesh = build_chunk(&ChunkInput { key, footprints: &real, infill: &lots, tags: &tags, ground: &|p| h(p), region });
let mut layer = BuildingsLayer::new(&mut ctx, &environment.bind_group, BuildingSettings::for_quality(&quality))?;
layer.queue_chunk(key, Arc::new(mesh));   // uploaded from its prepare, 8 MB a frame at most
layer.set_offset(offset);                 // world x, z = theatre x, z + offset
layer.set_light(&light); layer.set_fog(fog); layer.set_cloud_shadow(Some(&clouds));
layer.camera_theatre(); layer.held_lod(index); layer.holds_mesh(key, &mesh); layer.remove_chunk(index); layer.counts();
```

`tags` is a `MapTags` (`buildings::tags`, chunk-local metres, the host fills it from OpenStreetMap: docs/archive/native-theatre-history.md,
"Buildings"): land-use zones, the points of interest that type a building, OSM's building outlines, the water, the
piers, the roads, a port's presence and the landmarks the host knows by name. An empty one types the footprints by
their shape and size alone, as before 2026-10-05. `region` is the chunk's place's palettes (`Region::at`, below).

- **Chunks.** Fixed 1 km squares of the theatre's metres (`CHUNK_M`; chunk (i, j) from (i, j) km), so a chunk made once
  serves every window and scene that comes back to it. A footprint belongs to the chunk holding its middle; its ring is
  in metres from the chunk's corner (x east, z south, as the chart). Each chunk is one vertex and index buffer drawn by
  one indexed draw, placed by a per-chunk translation (an instance-rate vertex buffer of the chunk's corner plus the
  layer's offset, rewritten when either changes), culled by its bounds against the view and drawn nearest first in the
  `Opaque` phase after the land. Three levels: `Lod::Full` (every roof and facade), `Lod::Box` (outlines simplified to
  2 m, each building a box under a flat top at its roof's mean height: about 30% fewer triangles) and, past the boxes,
  `Lod::Far` (the town as coarse masses from the land cover: "The far level", below). Which chunk stands at which level
  is the host's choice (`BuildingSettings::lod_at`, 250 m of margin each way so a chunk does not flicker). A chunk
  changing level keeps drawing the old one until the new one is uploaded; the two then cross-fade over 0.6 s
  (`FADE_S`): each pixel goes to one level or the other by a dither that moves every frame, the two levels taking
  complementary pixels (the chunk's translation carries the fade; `BUILDINGS_FADE` pipelines discard the other level's
  pixels), so nothing pops in or leaves a hole, and the old level goes once the fade is done.
- **Plan** (`plan_building`). The ring cleaned (repeats and collinear points within 0.4 m dropped, counter-clockwise);
  footprints under 6 m², standing in the sea by the window's heights, or across a cliff (more than 6 m plus a quarter of
  the length between the highest and lowest ground under them: the heights' 40-140 m samples put coastal buildings on
  the shelf's face) are left out. The source's height is taken between 2.5 and 400 m (`MIN_DATA_HEIGHT_M`,
  `MAX_DATA_HEIGHT_M`); otherwise it is estimated from the area, the elongation and the share of the ground built over
  about it (`DensityGrid`: 100 m cells of the chunk, each with its neighbours): sheds under 30 m² one 2.6 m storey;
  houses under 220 m² one or two storeys of 2.8 m (three in a dense terrace; one in the tropics' and dry countries'
  suburbs); 220-1300 m² terraces of two or three, blocks of three to six where the ground is built over; larger, blocks
  of five to eight in a dense core (`MAX_ESTIMATED_STOREYS`; a rare tower of 10-18 storeys, 3 in 100 of a dense core's
  compact 500-2,500 m² blocks, but in the tropics, dry countries, Micronesia and on atolls), else an industrial shed of 9 m. Measured: a tower
  from 36 m, an industrial shed over 1500 m² under 13 m. A footprint on a pier stands on its deck. A block or a row
  the source drew as one is cut into the buildings it holds ("Parcels", below).
- **Types** (2026-10-05, from the map's `Setting`: the land use at a footprint's middle, the most specific zone
  winning, a point of interest on it or within 6 m, the sea within 150 m of a round one, the pier under it):
  - *Church* on a `place_of_worship` point (40-8,000 m²): a nave of 5-20 m eaves under a steep gable (42-52 degrees,
    20-28 in the Mediterranean and dry countries) when it is near rectangular and no wider than 34 m, with its tower
    (a fifth to a quarter of the nave's width, at the west end, to 1.5-2 times the ridge plus 4 m, or to OSM's measured
    height: Hallgrímskirkja's 74.5 m) under an octagonal spire (north and west of Europe, the Americas: patina, lead,
    slate, black; Nordic red iron or black) or a campanile's low pyramid in terracotta (the Mediterranean, dry
    countries), and over a large Mediterranean church (900 m² and more, three in five) a dome on a drum at the
    crossing. A *mosque* (`muslim`) is a flat-roofed hall under a dome (0.36 of its width) with one minaret, two over
    600 m², four over 3,000, at its corners (to 1.7 times the dome's top plus 8-22 m: a ten-sided shaft, its balcony,
    a pencil cap). Synagogues and temples: a hipped roof, no tower.
  - *Warehouse* and *factory* on industrial land (`industrial`, `railway`, `garages`): sheds under 50 m², warehouses
    (a low gable of 8-14 degrees, big doors on the ground floor's bays, 7 m bays) and, on two in five over 900 m²,
    factories: a sawtooth roof (teeth 7-10 m apart rising 0.45 of that to glazed north lights) on three in five
    rectangles, else a low gable or flat, and on a third of those over 900 m² one or two chimneys (25-55 m, brick or
    concrete, red and white bands at the head over 40 m) through the roof near one end. An infill lot on industrial
    land is a warehouse.
  - *Oil tank*: a footprint at least 0.86 round (4π area over the perimeter squared) and 9-110 m across, unmeasured or
    under 30 m, on industrial land, or by the sea, or with no land use mapped (12 m and more): a twenty-sided cylinder
    of 6-22 m (4 + 0.3 of its width) under a shallow cone (seven in ten) or a floating roof, white, silver or sand.
  - *Chimney*: a round footprint up to 25 m across measured at least 20 m and 2.5 times its width.
  - *Lighthouse*: on a lighthouse's point (by name: OpenMapTiles has no `man_made`) or a landmark: a tapering
    twelve-sided tower, white or banded red or black, its gallery and dark rail, a glazed lantern (lit at night, bright)
    under a red or black cap; one over 70 m² is a keeper's house with the tower through its roof (Point Pinos). One
    the heights put in the sea (a breakwater's light) stands on a concrete plinth. A lighthouse's point with no
    footprint under it stands a 16 m tower.
  - *Office* on commercial land (three storeys and more; measured ones from 11 m): ribbons of glass across the bays
    or, on half, a curtain wall; a measured tower there an office tower. A large low building on commercial land is a
    shop's box (a warehouse); on residential or civic land a long block.
  - *Pier* (`Kind::Deck`): OSM's pier areas as decks 2.4 m over the sea, its lines as 4 m jetties 1.1 m up, their
    sides 2.5 m into the water; buildings on them stand on the deck.
  - *Crane*: along a port's quays (industrial land's edge with the water 18 m out and land 10 m in, runs of 100 m and
    more, 2 ha of industrial land behind, a ferry terminal in a tile about: OpenMapTiles maps no port land, and a
    marina's or a power station's waterfront is industrial land at the water too) every 70-130 m, 12 m back: jib
    cranes (a portal on four legs, its house, A-frame and raised jib), or container gantries along a 300 m quay before
    10 ha; at most eight a quay, whole chunks only.
  - *Landmarks* the host knows by place (`LandmarkKind`): a lighthouse, a free-standing stack; each takes the footprint
    it stands on.
- **Regions** (`region.rs`, `Region::at`, the chunk's middle's latitude and longitude, the land style deciding
  elsewhere): British (England and Wales north of the Channel's middle, Scilly to Berwick and Carlisle), Celtic
  (Scotland north of the border by seven points over the Solway and the Cheviots to the Tweed, Ireland west of the Irish
  Sea's middle, Man, Orkney and Shetland), Iceland (with the Faroes and Greenland), Nordic (Norway, Sweden from
  Gothenburg and Helsingborg, Finland), North Sea (the Low Countries and Flanders, the Lower Rhine, north Germany,
  Denmark with Helsingør, Copenhagen and Bornholm, Pomerania to the Oder), Florida (the peninsula and the Keys, Tampa
  and Daytona south; not Jacksonville or the Panhandle, Cuba or the Bahamas), American West (California, Nevada, Arizona, Utah, New
  Mexico and west Texas: 125-103° W, north of the Mexican border (`mexican_border`: the parallels and the Rio Grande
  by fifteen points, a few kilometres off) and south of 42° N, Colorado left out), Malta (with Gozo), Aegean (Greece,
  its islands and Cyprus: south of Albania, North Macedonia and Bulgaria, west of the Evros and of the Aegean's middle
  between the Greek islands and Turkey's coast by eleven bands), North Africa (the Atlantic to Suez and Sinai, the
  Sahara to 20° N: south of the Strait with Ceuta and Melilla left Spanish, of Spain's south-east, Sicily, Pantelleria,
  Lampedusa and Malta; east of Madeira and the Canaries; west of Egypt's border and the Red Sea's middle), Nile (its
  delta from Alexandria to Port Said, Cairo, the Fayoum, the valley within 35 km of the river to Aswan), Mediterranean
  (its northern shores, Iberia, Turkey, Madeira, Latin America south of that border), American
  (the rest of North America: the East Coast, the South, the Midwest, the Pacific north-west's siding), Temperate
  (France, central and southern Germany, the Alps, Poland, the Baltic states), East Asian (Japan from Hokkaido to Okinawa, Korea, Taiwan, eastern China; not
  Russia's Primorye), Arid (Arabia, the Levant, the Middle East, the Canaries; a dry style in the tropics), Hawaii (its
  main islands, Niihau to the Big Island), Micronesia (the Marianas, Palau and the Carolines north of 0.5° N and east
  of 130.5° E, Kosrae, Wake, Midway), Atoll (the Marshalls, Kiribati, Nauru, Tuvalu, Tokelau: east of 160.5° E
  between 11° S and 15° N, the Solomons' Malaita, Makira and Temotu left out; the Phoenix and Line Islands), Tropical
  (the humid tropics: the Solomons, New Guinea, the Philippines, Fiji, Samoa, Tahiti), `Region::of` the style
  elsewhere. Until 2026-10-08 California was Mediterranean (terracotta over Los
  Angeles) and Japan temperate; until 2026-10-09 Britain, Ireland, the Low Countries and Denmark were temperate (red and
  orange tile over white render), Iceland and Norway shared painted iron, and Florida was American; until 2026-10-09
  (later) Malta, Greece and the Maghreb's coast were Mediterranean (terracotta), Egypt and Libya arid, a warm
  country's flat roofs were dark gravel, tar and asphalt, and Hawaii, Micronesia and the atolls were tropical.
- **Roofs** (`palette::roof_rules`). A house, shed or terrace whose footprint fills at least 80% of its smallest
  rectangle, no wider than 16 m and under 13 m tall takes that rectangle and a pitched roof along its long side: gabled,
  or hipped (15% of Nordic, Icelandic and Scottish roofs, 25% North Sea, 30% British, 35% temperate, 45%
  Mediterranean, 45% Greek, 40% Maltese and North African, 50% Egyptian, 35% American, 55% American West, 70%
  Floridian, 70% Hawaiian, 40% Micronesian, 35% on an atoll, 50% East Asian, 60% tropical, 50% dry), pitched
  26-42 degrees (30-44 British, 35-47 Scottish and Irish, 40-54 North Sea, 18-27 in the Mediterranean, 20-28 Greek,
  18-28 Maltese and North African, 16-24 Egyptian, 20-32 Hawaiian, 14-24 Micronesian, 16-30 on an atoll, 22-36 American,
  17-27 American West (a ranch house's 4:12 to 6:12), 18-26 Floridian, 16-28 East Asian, 18-28 in the tropics, 16-24 in
  a dry country, 14-24 on a shed; four in five of a dry country's houses flat, one in five of the American West's
  (gravel and tar), 15% of Florida's, 95% of Malta's, 97% of Egypt's, 88% of North Africa's, 55% of Greece's, 65% of
  Micronesia's, 10% of Hawaii's and an atoll's). A
  terraced house (a terrace's footprint no longer than 18 m and a quarter
  longer than wide: one house of a row, as OSM maps Britain's and Holland's) turns its ridge across its width, along
  the street, gabled at a quarter of its region's spread of pitch and at most 4.2 m high, so a row's roofs run on as
  one instead of a sawtooth of gables. A low block (six storeys at most, 22 m wide, near rectangular) wears a hipped
  tile roof of 18-26 degrees on 40% of Nordic blocks, 35% Mediterranean and North Sea, 30% Scottish and Irish, 25%
  temperate, 20% British, 15% Icelandic, 10% American, American West and Floridian, 5% Greek and Hawaiian, 3% North
  African, none in Malta, Egypt, Micronesia or on an atoll. A measured height is the
  roof's mean, so the eaves stand half the rise below it. Everything else is a flat roof over the ring (ear clipping;
  holes, such as courtyards, are roofed over). The roofs' colours by region (houses; blocks and others in brackets):

  | Region | Houses' roofs | Walls |
  | --- | --- | --- |
  | Nordic | anthracite and black tile, slate, red and red-brown tile now and then, green-grey (blocks: anthracite, copper, black, slate, red iron, red tile) | painted timber: white, Falu red, yellow and ochre, light and blue-grey boards, dark timber (blocks: cream, yellow, ochre, pink and white render, some brick) |
  | Iceland | red, green and blue painted iron, black, asphalt, slate, galvanised (blocks: red iron, black, slate) | white and coloured painted iron and boards, Falu red, render |
  | British | slate and blue slate (half), red-brown and brown concrete tile, anthracite, a dark terracotta now and then; no orange | red and brown brick, yellow stock, a purple-brown klinker now and then, white and cream render, limestone |
  | Celtic | slate, blue slate, anthracite, asphalt | grey harl (a new entry), white and cream render, grey and red sandstone (new), limestone, warm yellow now and then |
  | North Sea | orange and red clay pantiles, black glazed tile, red-brown and dark terracotta, reed thatch (new) now and then, slate | red, brown, purple-brown klinker (new) and yellow brick, white and cream now and then |
  | Temperate | slate and blue slate, anthracite, dark and red-brown tile, brown tile, some terracotta | red and brown brick, render, cream, yellow brick, stone, limestone |
  | Florida | grey and brown shingle, white, orange and faded terracotta barrel tile, sand tile, dark shingle, galvanised (blocks: white, orange terracotta, grey shingle) | white and bright white stucco, cream, peach, pink, pastel blue and green, warm yellow, sand, beige siding |
  | Mediterranean | terracotta, orange and faded terracotta, dark terracotta, sand and red-brown tile | whitewash, white and cream render, warm yellow, light ochre, peach, salmon, ochre, pink |
  | American | grey, brown and dark asphalt shingles, asphalt, black, slate | light grey, beige and blue-grey siding, white, brick, pastels |
  | American West | grey, brown and dark asphalt shingles, asphalt, one in ten concrete (sand) tile; one in five flat, gravel or tar (blocks: shingles, some terracotta, mostly flat) | white and cream stucco, whitewash, light ochre, peach, sand, beige and grey siding, stone, a pastel now and then |
  | East Asian | grey kawara (the colour cube's 102 grey), anthracite and black glazed tile, blue slate, lead, galvanised, blue, green and green-grey sheet metal (blocks: kawara and anthracite on the few pitched, most flat) | white, cream, light concrete, bright white, grey and beige siding, green-grey, stone, sand, dark timber now and then |
  | Tropical | galvanised and rusted corrugated iron, light galvanised, red, blue and green iron, a little terracotta | white render, cream, pastels, sand, painted boards |
  | Arid | flat concrete, white, gravel, membrane (a pitched one terracotta or concrete) | sand, cream, white, limestone, ochre |
  | Malta | flat (limestone screed, a new entry, pale concrete, white, membrane); terracotta on the few pitched | honey limestone (a new entry, ashlar), limestone, cream, pale ochre, sand, white |
  | Aegean | half flat (pale), half terracotta, orange, faded and dark terracotta, red-brown tile (blocks: flat) | whitewash, bright white, white, cream, pale ochre, peach, warm yellow, a pastel blue now and then (blocks: white, cream, pale ochre, light concrete, limestone) |
  | North Africa | flat (pale); terracotta or a green-glazed roof on a colonial villa | white, whitewash, cream, sand, pale ochre, ochre, peach, a Marrakech pink now and then; no brick |
  | Nile | flat (dusty concrete, gravel, tan) | sand, cream, pale and dark ochre, light and mid concrete, white, peach; red and brown brick between concrete frames on a quarter of the houses and a sixth of the blocks |
  | Hawaii | grey, dark and brown shingle, galvanised and light galvanised, green-grey, green and red iron, white, concrete tile (blocks: flat) | white, cream, pale green, plantation yellow, tan and grey siding and boards, light ochre, pastel blue, sand, peach, dark timber (blocks: white, cream, concrete, sand, limestone) |
  | Micronesia | flat (pale); galvanised, rusted, red, green and blue iron, white on the pitched | white, cream, light and mid concrete, pastel blue, green and pink, peach, sand, ochre, yellow |
  | Atoll | galvanised and rusted iron, pandanus thatch on one in six (reed thatch's entry), red, blue and green iron | concrete block, white, cream, grey sheet, pastels, dark timber (plywood), stone grey |

  Flat roofs: gravel, tar, concrete, pale or white membrane, asphalt; in a warm, dry country (the Mediterranean,
  Greece, Malta, North Africa, the arid lands) pale concrete, membranes, limewash and light gravel only (a warm
  limestone screed on half of Malta's and on a fifth of North Africa's), in Egypt dusty concrete, gravel and tan
  (`palette::flat_roofs`). Warehouses and factories galvanised, grey,
  coloured sheet or concrete (brick mills and warehouses beside them in Britain and the North Sea's country); churches
  by region (slate, patina and lead in the north and west, a square tower without a spire on more than half of
  England's and Wales', red sandstone in Britain and Scotland now and then, brick
  Gothic under copper and black tile by the North Sea, black and copper in the Nordic countries and red iron in
  Iceland under a spire, terracotta in the Mediterranean, a dome over most Greek churches from 150 m² (blue,
  terracotta or white) and over Malta's from 400 m² (lead and red), a white steeple in Florida and Hawaii, an iron
  spire in Micronesia and on the atolls); offices glass and
  concrete. OSM's `building:colour` (a CSS
  name or hex) paints the walls (`tags::parse_colour`, then the colour cube's nearest entry). What a footprint is
  (`estimate`) follows its region too: in the American West a small footprint is a ranch house of one storey (two on
  two in five) where the ground is thinly built, a townhouse terrace where it is thicker, a block of three to five
  storeys only downtown (built over above 55%; four to eight on larger footprints), garden apartments of two storeys
  otherwise; in East Asia a small footprint is a two-storey house (one in seven of one, one in seventeen of three), and
  over 30% built a flat concrete block of three to six storeys (five to eight on larger footprints); in Britain,
  Ireland and the North Sea's country a small footprint over 30% built is a terrace of two storeys (three on one in
  seven), elsewhere a house of two storeys (one in eight of one; Scottish and Irish cottages one in two of one); in
  Florida a house of one storey (two on one in five) where under 45% is built; in Malta a small footprint over 30%
  built is a townhouse terrace of two or three storeys; in Greece a narrow polykatoikia of four to six storeys over
  45% built (four to seven on larger footprints over 30%), a house of one or two elsewhere; in North Africa a medina's
  house of two or three storeys over 45% built, a village's of one; by the Nile a narrow block of three to five
  storeys over 25% built (four to eight on larger footprints); in Hawaii a house of one storey (two on two in five)
  under 55% built, a walk-up of three or four over it (three to six on larger footprints over 45%); in Micronesia and
  on an atoll a house of one storey (two now and then), Guam's blocks of two to three storeys on larger footprints
  over 40% built, an atoll's meeting house or store of one or two.
- **Parcels** (2026-10-28, `parcels.rs`, both near levels). Microsoft's model traces a dense core's roofs as one
  outline a block (a terrace's row of houses, a street's palazzi, a city block's flats), which stood as one box of one
  height, colour, window grid and flat roof: Taranto's, Valletta's and Athens' cores seas of pale slabs, Dover's rows
  brick blocks of four to six storeys. A terrace, a block or (in the period) an office on residential, commercial or
  unmapped land is cut back into the buildings it holds, each planned as its own (`plan_building` with the piece's
  ring, the block's kind and a height stepped from the block's: its own walls, roof, bays and seed). Along its length
  into frontages of its region's widths (`frontage`: a terrace's 4.6-6.4 m in Britain, Ireland and the North Sea,
  4.8-7 m in America, 5.5-9 m about the Mediterranean, 4.5-7.5 m in East Asia; a block's 7-16 m in Britain, Ireland
  and the North Sea, 9-20 m about the Mediterranean, 9-16 m in Greece, 7-13 m in Malta, 13-24 m in temperate Europe,
  Scandinavia and Iceland, 8-22 m in America, 12-30 m in the American West); a block deeper than 24 m (`ONE_ROW_M`) as
  two rows back to back, parted within 15% of its depth of its middle; one deeper than 46 m (`COURT_M`) as its two
  street rows, a building across each end and a court of one storey between (a perimeter block's yards and
  workshops). A piece's storeys step from the block's by its region's shares (`steps`, of two down, one down, one up
  and two up): a British, Irish, North Sea or American terrace's 5% one down and 8% one up, Malta's 10% and 12%,
  another terrace's 15% and 20% (3% two up); a temperate, Nordic, Icelandic, British, Irish, North Sea or American
  block's 20% each way (2% two down, 5% two up), Malta's 18% (3% and 4%), elsewhere a street built house by house,
  23% and 22% (7% and 10%); never under one storey (two for a block). Three in four of a British, Irish or American
  terrace's houses keep the row's walls (and its roof's colour when both are pitched), 55% of a North Sea, temperate,
  Nordic or Icelandic row's, three in ten elsewhere (`keeps_row`). What stands whole: a footprint filling less than
  0.6 of its rectangle; a terrace under 7 m deep (`ROW_DEEP_M`: one deep house, OSM's terraced house, a pair of
  semis), shorter than 2.5 times its depth or than three frontages; a block of one row shorter than 2.5 frontages; a
  kind given (the infill's lots), a point of interest or a pier on it, industrial, civic, military or open land; and
  any footprint a piece of which would be under 14 m² (`LEAST_PIECE_M2`) or would not stand (in the sea, across a
  cliff), or whose pieces' areas miss the whole's by 2%. A cut that would cross the outline more than twice
  (`crossings`: an L's or a U's arms) is left out, so every piece is the outline's whole part between two cuts
  (`geometry::clip_polygon`). Everything hashes the footprint's middle in the theatre and reads the outline alone (not
  the map's roads, which the two levels read at different zooms), so both near levels cut a block alike and
  cross-fade without a jump. Where two pieces' edges run back to back (within 5 cm, `MeshBuilder::parties`), each
  one's wall there stands only above the other's walls' top (`Party`; at the box level too): below it the wall is
  inside its neighbour, drawn and shaded for nothing, and above a lower neighbour's roof it would fight that roof's
  parapet, in the same plane and facing the same way, for the same pixels. Costs under "Costs" (2026-10-28).
- **Walls** reach 1.5 m below the lowest ground under the building (`SINK_M`; the drawn land's level of detail lies a
  little off the heights' bilinear surface) and up to its eaves over the mean ground, never less than 2.2 m over the
  highest. Every face is flat-shaded with its own vertices (round parts smooth), 24 bytes each (`BuildingVertex`:
  position, snorm8 normal with the window rows in w, f16 facade coordinates, palette entries, hash, and a byte of kind
  (bits 0-3), part (4-6: wall, roof, gable, banded, glass, steel, dome) and detail (7)).
- **Parts** (`parts.rs`, `mesh.rs`): prisms (tanks, chimneys, lighthouses, minarets, drums), cones and pyramids
  (spires, a tank's roof, caps), domes (16 sides, four bands), boxes and beams (towers, cranes), a sawtooth roof, a
  pier's deck. Their triangles: a tank 60 (22 as a box), a church's tower and spire 18, a dome on its drum 150, a
  minaret 86, a chimney 28, a lighthouse 122, a jib crane about 115, a gantry about 200. A box chunk keeps towers, spires
  (four-sided), minarets, chimneys and lighthouses (six-sided) and tanks (eight-sided), and drops domes and cranes.
- **Facade** (`shaders/buildings.wgsl`). A wall's coordinates are window bays (whole bays from corner to corner, 3 m on
  houses, 3.3 m on blocks, 6 m on sheds) and storeys up from the mean ground, so the shader lays the windows out per
  wall: a window in the middle of each bay and storey (wider on blocks and towers, shop fronts on half the blocks'
  ground floors, a high ribbon on an industrial shed's top storey, none on a shed or a gable), dark glass with the sky's
  reflection (roughness 0.08), a drawn blind now and then, a sill's line under each storey, the ground's shade over the
  lowest 0.4 storey; all of it fading to its mean as the pixel outgrows a window (no moiré from a ship 2 km off). At
  night (the sun more than about 2 degrees down, or the moon the light) the windows glow warm as their rooms are lit
  ("Lamps in the windows" below). Roofs: tile courses every 0.32 m down a pitched slope (0.24 m on shingles, ribs every
  0.6 m on iron), metal a little glossy, faint 4 m patches on flat roofs (their wear: "Weathering" below); spires and domes smooth, lead and copper glossier. Variety (2026-10-05), all by the
  building's hash: the bay's width 0.85-1.2 of its kind's; a house's windows tall and narrow, wide and low or the plain
  sash; a block's wide or narrow; shutters either side of a house's windows (green, blue, brown, grey or white, one in
  seven closed over its window) on 60% of Mediterranean houses, 35% American, 20% temperate, 15% tropical, 10% dry;
  balconies on a block's upper storeys (a slab, a dark or solid rail, the shade under it, stacked in seven bays in ten)
  on 70% of Mediterranean blocks, 40-50% elsewhere, 25% American; an office's curtain wall (glass floor to floor) or
  ribbons; a warehouse's doors on about half its ground floor's bays; a church's tall windows, its tower's belfry
  openings at the top. A banded part (a lighthouse, a stack's head) alternates the roof's colour and white; glass is a
  lantern (lit at night, about 9 times a window's light) or a factory's north lights (wired glass between pale
  bars); steel takes its colour as painted. 32 wall and 32 roof colours, then a 6 × 6 × 6 colour cube (entries
  32-247, `cube_entry`) for OSM's colours and cranes, then a few more authored from 248 (`palette::EXTRA_FIRST`; walls:
  harl, klinker, red sandstone, Malta's honey limestone; roofs: reed thatch, coursed by no tiles, soft streaks down its slope and matte, a flat roof's limestone screed, and since
  2026-10-14 smoked (ibushi) kawara with a silvered sheen and dark glazed kawara, glossy); the
  region picks among them (`palette.rs`, the table above), each building
  tinted 0.88-1.12 by its hash. The colours take a milder form of the land's tone (80% of their saturation, 10% darker
  below a luminance of 0.35). That is Medium's facade; High and Ultra's is textured (below).
- **Facade textures** (2026-10-08, High and Ultra; `facades.rs`, `FACADE_TEXTURES` in `buildings.wgsl`). Drawn on the
  CPU when the first layer is made (120 ms on its building thread, once a run; nothing fetched or licensed) into three
  `Rgba8Unorm` texture arrays with their mips, 6.7 MB on the GPU, read through one repeating, trilinear, 8× anisotropic
  sampler. *Walls*: nine materials 256² a 4 m tile, laid along a wall in metres (chunk-local, so steady at a city's far
  side): render, stucco, brick (stretcher bond with its mortar, a clinker one in nine), lap siding, boards and battens,
  corrugated sheet, precast panels with their tie holes, ashlar, glazed tile; a texel holds the shade on the palette's
  colour, the joints (mortar a pale grey on masonry, the wall's colour darkened elsewhere) and the relief's normal. Each
  material repeats exactly over its tile (a test holds it). The palette entry chooses the material (brick for the
  brick colours, siding, timber, sheet, panels, stone; harl stucco, klinker brick, sandstone ashlar), then the region:
  render becomes stucco in warm countries (Malta, Greece, North Africa, Egypt, Micronesia and the atolls too) and
  Florida, boards or siding on two Hawaiian houses in three, corrugated sheet on one atoll house in four, siding on an
  American house, siding or boards on a Nordic one,
  painted sheet, siding or render on an Icelandic one and siding on half of East Asia's; panels on a house are render
  and on most East Asian blocks tile. *Windows*: 22 cells of one bay by one storey, 128², drawn with
  4 × 4 samples a texel: the colour premultiplied by its cover (the wall shows round it), and the normal, the glass
  and the share the building's accent colour tints (shutters, doors, a shop's sign, a curtain wall's glass). Sash and shuttered sash
  (temperate, British, American), French windows between shutters and casements in a stone surround (the
  Mediterranean, Malta and Greece, the tropics, North Africa's houses behind grilles otherwise; casements alone on a
  North Sea house), Nordic cross-framed windows in wide white trim
  (Iceland's too), a ranch house's aluminium sliders (Florida's and Hawaii's, or jalousies), a Japanese house's
  aluminium sash under its rain hood, jalousies (the tropics), small windows behind grilles (dry countries), a house's
  panelled door and an American garage on the ground floor now and then, a block's windows, French doors onto iron
  balconies (a Japanese block's balconies with solid parapets before sliding doors, its windows under hoods), a shop
  front under its sign, a curtain wall, ribbons, a factory's high ribbon, roller doors, a church's leaded arches and its
  belfry's louvres. Which one a bay and storey takes: the region and the kind, the
  building's hash and the bay's (`window_style`). The glass reflects the sky (roughness 0.04 since 2026-10-13), the
  room behind it shows through (below; until 2026-10-13 a blind drawn in one in six and a third of the windows glowing
  flat at night), and the relief leans the shading normal (the shadow's lookup keeps the face's). Derivatives are taken once, before any branch, and the reads take them explicitly
  (`textureSampleGrad`), so the windows fade to their mean through the mips as the pixel outgrows them.
- **Rooms behind the windows** (2026-10-13, High and Ultra; `room`, `wall_textured` in `buildings.wgsl`). The owner
  found the windows the least real part of the towns: every one the same black pane in a white frame, flat on the
  wall, the same size whatever the wall, and at night the same cream square. Now:
  - *A room behind the glass* (interior mapping, van Dongen 2008): a box a bay wide, a storey high and 3.5-6.5 m deep,
    met by the ray from the eye, so its floor, ceiling, side and back walls shift with the view. Each room takes its
    own colours (distemper, paper or panelling; boards, a red or green carpet, tatami in a Japanese house), a doorway
    in its back wall now and then, and a sideboard before it. It reflects the light that falls on its window from
    outside (the sky, the fill and a share of the sun: `Surface::interior`, lit in `fs_building`), falling off into
    its depth, and is seen through what the glass does not reflect. Paper (`Shoji`) shows no room, only its lamp.
  - *The window's dressing*, at the glass: drapes either side, drawn back or closed (one in eight); net curtains over
    the glass or its lower half (`net_share`: 60% of Britain's and Ireland's windows, 55% North Sea, 35% northern
    Europe, Scandinavia and Iceland, none in Japan, 12% elsewhere); a holland blind part down. The dressing takes the
    sun whole; the room only as a patch (`Room::dressed`).
  - *A lamp at night* in the rooms a household has lit ("Lamps in the windows" below): incandescent, warmer or
    whiter and brighter or dimmer by the room, lighting the room's surfaces about it and glowing through its drapes,
    nets and blinds.
  - *The reveal.* The window texture's second half (`facades::Facades::window_surface`, layer 29 on) holds each
    window's recess: the parts that lie back in the opening (frames, glass), not the trim, sill, surround, shutters or
    rails proud of the wall. Seen at a slant, what lies back slides behind the opening's edge by a reveal of 0.16 m
    (0.08 m in boards, siding and sheet), and the reveal's side shows; in the sun its head and a jamb shade the frame
    (`Surface::sun_shade`). Windows running from bay to bay (balconies, shop fronts, curtain walls, ribbons) lie on
    the wall's face. Far off (`resolved`), none of this: the window is the flat cell and the room its mean.
  - *A window keeps its size in metres* (`window_half`, `nominal_bay`), centred in its bay, whatever the bay's width
    on the wall (a wall's bays are its length over the building's bay, rounded, so they ran 0.5-1.5 times the
    nominal and stretched their windows with them): squeezed only as far as its bay needs, and left out of a bay
    it would fill at under three fifths of its width, so a footprint's short jog no longer carries a sliver of a
    window. The bay's and storey's metres come from the facade's coordinates against the wall's metres (their screen
    derivatives' ratio: each is linear over the face).
  - *Each window its own*: the room, dressing, lamp and the pane's tilt (each pane a little out of true, so the sky
    it shows differs; a past period's glass wavy) are hashed by the bay, the storey, the building and the wall
    (`VertexOut::wall`, from the wall's first corner, which every triangle of a face takes as its first vertex, so it
    holds over the wall exactly; every wall's bays count from 0, so on a building of short walls every first bay
    matched). A bay wider than its window shows the wall past the window's cell, not the next cell's window.
  Medium's untextured windows take each room's mean colour.

  Cost (Apple M5 Pro, Metal, `test-fast`, 1600 × 900, High, 1942, Istanbul by day; the scene's opaque pass by the
  GPU's own timestamps, medians of 40 frames in five rounds alternating this shader and the one before, swapped by
  hot reload in one client; other agents on the machine): 0.0 ms at 30 m over Fatih, +0.02 from the Golden Horn,
  +0.07 from 150 m, +0.10-0.18 in telephoto views (8-10°) whose screens are streets of windows. The window texture's
  recess layers double its surface array on the GPU (29 more 128² RGBA8 layers with their mips, 2.5 MB). Stills (not committed) in
  `.build/facades/`: `look/` and `night/` hold `<place>-<view>-<day|night>-period-<before|v3>.png` for Los Angeles
  (Hermosa Beach, Redondo, Palos Verdes' trees), Istanbul (Fatih, Sirkeci, the Golden Horn) and Reykjavík.
- **Weathering** (2026-10-14; `grime_mean`, `wall_grime`, `weathered`, `roof_surface` in `buildings.wgsl`). The owner
  found the walls and roofs too clean, every tile and wall one flat colour. Kept light, as on the ships. Each building
  takes an age by its hash (0.15-1 today, offices and towers under half of it; 0.45-1 in a past period), and its region
  a wetness (`region_wet`: Britain and Ireland 1, the tropics 0.9, the North Sea 0.85, East Asia 0.75, Florida 0.65,
  Nordic and temperate Europe 0.6, Iceland and America 0.45, the Mediterranean 0.25, Malta and Greece 0.15, the American
  West and North Africa 0.1, the deserts none) and a sun (`region_sun`: the deserts 1 down to the tropics 0.6, none north
  of the Mediterranean).
  - *Walls* (High and Ultra; drawn under the windows' frames and glass): dirt run down from the head in streaks a
    fifth of a metre apart, a few dark and most faint, their runs longer where the region is wet, more down a flat
    roof's parapet than under a pitched roof's eaves; a wash under each window's sill (the storey's own, or the next
    storey's over the pier above), darkest under its ends; splashback up the foot, and in a damp country rising damp to
    a ragged tide line; algae in blotches on a damp north face (black mould in the tropics, every face alike); rust
    run down a sheet wall from its laps every 2.4 m; paint and render bleached toward grey on a sunny country's
    southern faces. A material shows its dirt by its colour: render, stucco and concrete the most, stone 85%, brick and
    boards half, sheet a third (its rust apart), glazed tile a quarter. A past coal town's walls darken to 0.86 (0.8
    until 2026-10-14) and carry the rest of their soot in the runs. The streaks and runs fade to their mean as they
    outgrow the pixel (by 2.5 m a pixel), and Medium's walls take that mean.
  - *Roofs*: each tile its own shade in broken bond, a newer one in forty; rolled tiles (kawara; Spanish and Roman
    barrel tile in the south, the American West, Florida and Hawaii; the North Sea's pantiles) show their rolls down
    the slope, shading across them with the sun; a ridge of capping tiles with their joints (a slate roof's in red
    clay in Britain, Ireland and northern Europe); moss on a damp country's north slopes in patches along the courses
    (grey lichen on slate and shingle); an iron roof's rust in patches and down its ribs; weathered patches metres
    across. A flat roof's sheets fainter than before (4 m, ±4%), its stains washed and blown, darker where water ponds.
- **Lamps in the windows** (2026-10-14; `room_lit`, `rule_light` in `buildings.wgsl`, `BuildingsLayer::set_sun`).
  Until 2026-10-14 a third of the windows were lit at random whatever the hour, each window apart, and a dimout or
  blackout only lit fewer. Now:
  - *By household.* A house lights whole or stays dark, a terrace's houses two bays wide, a block's flats three bays
    of a storey: the sitting rooms downstairs in the evening, the bedrooms upstairs at bedtime (a bungalow's alike); a
    block's stair one bay in ten up a wall, its landings dim; a shop through the evening (three in four) and a few
    after; an office's storey working late now and then (18% in the evening, 4% at night); a works on its night shift
    (three in ten); a church dark.
  - *By the hour.* The households lit: 42% for three and a half hours after dusk, 22% by 23:00, 6% by 01:00, 16% from
    about 04:30 (the early risers). The hour is the sun's hour angle about the celestial pole (`hours_after_dusk`, the
    sky's `celestial_pole`: the place's), from −6 at noon to 18 at the next, so a winter's dark 17:00 is evening and
    its dark 07:00 morning; handed to the layer each frame by `SeaScene::update_land`; until it is, 22:00.
  - *Under the rule* (the scenario's `lights`, night.md, "Night lights" below; `params.z`): under a dimout 15% of the lit rooms
    stand unshaded and the rest show 6% of their light through their shades; under a blackout 3% unshaded and the
    rest dark but for a thin line where the blackout cloth's halves meet, in one window in five (a box-filtered line,
    so it keeps its light far off).
  Far off every window takes its kind's and storey's mean (`mean_lit`, `rule_mean`). An evening's lit windows come to
  within a few percent of before.

  Cost (Apple M5 Pro, Metal, `test-fast`, 1600 × 900, High, 1942, Istanbul by day; the scene's opaque pass by the
  GPU's own timestamps, medians of 40 frames in seven rounds alternating this shader and the one before by hot reload in
  one client): within the noise. From the sea at Sirkeci +0.02 ms (two runs alike); over Fatih and in a telephoto view
  the rounds' paired differences ran from −1 to +1 ms with the machine loaded (load average about 20, other agents'
  clients on the GPU), their medians −0.6 to +0.6 ms. Stills (not committed) in `.build/town-lanes/W/`: `before/` and `after/` hold
  `<place>-<view>-<day|night>-period-<tag>.png` for Los Angeles (Hermosa Beach), Istanbul (Fatih), Reykjavík,
  Southsea (Portsmouth's terraces; the night under a dimout and a blackout too) and Yokosuka; `pairs/` the crops.
- **Roof clutter** (2026-10-08, High and Ultra; `clutter.rs`). What stands on a whole building's roof, made with its
  chunk (36 bytes a piece, sorted by shape) and drawn as instances of two shapes (a box of 10 triangles, and a water tower
  of 76: legs, a 12-sided drum and a cone) within 1.5 km of the camera (`CLUTTER_M`; a piece past it is dropped whole on
  the GPU, judged at its foot), and a third for the eaves' downpipes and the grounds below (`Shape::Near`, the box,
  drawn only in chunks within 800 m: `NEAR_M`), each placed by its chunk's translation read from a storage buffer by its chunk's slot.
  On flat roofs: a stair and lift housing on blocks, towers and offices of four storeys or more, air-conditioning units
  (one per 140 m² of roof; coolers on the American West's and dry countries' flat houses), vents, a Japanese block's
  water tank on its stand (55% of those of three storeys or more), a New York-style timber or tarred water tower (half
  the American blocks of six or more), water tanks on a warm, dry country's flat roofs (one on seven houses in ten,
  one per 120 m² of a block's roof up to eight; a solar heater's white tank over its dark panel in Greece, Malta and
  the Mediterranean, white and black tanks in North Africa, Egypt and the arid lands); on pitched roofs, chimneys by
  region (nineteen houses in twenty in Britain and
  Ireland, four in five in temperate countries, three in five by the North Sea, one in ten in Florida and none in East
  Asia, Hawaii, Micronesia or on an atoll, brick where brick is usual) and a stack every 5.5 m along a British or Irish terrace's ridge (a terraced
  house's own on its party wall). The sides
  take the building's walls (or brick, timber, steel), the tops its roof. They cast into the sun's near map and the
  nearest land map ("land 1", texels of 0.2-1 m; native/docs/render-core/shadows.md, "Land maps"); the farther maps would not resolve them.
- **Eaves, parapets and dormers** (2026-10-14, the whole level, every quality; `eaves.rs`, `MeshBuilder::pitched_roof`
  and `parapet`). A gabled or hipped roof is a slab 0.14-0.25 m thick at the walls, held out past them by its region's
  overhang: East Asia's 0.65-0.95 m in the past and 0.45-0.7 m today, the tropics' and Hawaii's 0.5-0.8, the American
  West's 0.45-0.8, Scandinavia's 0.35-0.6, Britain's and Ireland's 0.12-0.3, the Mediterranean's 0.2-0.4; a terrace's
  four fifths of that, a shed's three fifths, a block's at least 0.35, a factory's and church's 0.3. A hipped roof's
  ends are held out as far below the walls' tops as its sides, so its eaves run level all round. Under the overhang a
  soffit, along its edge a fascia with a gutter's 6 cm drop below it on nine roofs in ten in wet countries (four in ten
  in the tropics, three in ten in dry ones), in the region's trim: Britain's black or white, America's white, cream or
  brown, Scandinavia's white or zinc (a quarter Falu red), Europe's zinc, copper or white, Japan's copper or brown in the
  past; the soffit timber in Japan and Scandinavia. With no gutter, a dark timber fascia over the walls' own render or
  timber. A gable's verge is the slab's end, flush with the gable; a gutter has a downpipe at two diagonal corners
  (`Shape::Near`, High and Ultra). A flat roof has a parapet, its walls run on past the roof with their inner faces drawn:
  0.6-1.1 m on most blocks, towers and offices (85%), 0.5-0.9 m on seven terraces in ten, 0.4-0.9 m on four in five of a
  warm, dry country's flat houses, fewer elsewhere; the box level adds it to its top so the skyline holds from one level
  to the next. Dormers stand on a house or terrace of one or two storeys in Britain, Ireland, the North Sea, Europe,
  Scandinavia, Iceland and America, on a roof of 30° or steeper deep enough to hold one under its ridge: one to three on
  the slope facing the nearest road, each a gabled box whose face is a bay of the attic's storey (a window, as the
  walls'), its cheeks and roof running back into the slope. Triangles: a gabled roof 22 for 6 (the soffits, fascias
  and the slab's ends), a hipped one 22 for 6, a dormer 9, a parapet 2 an edge.
- **Grounds** (2026-10-14, High and Ultra; `grounds.rs`). About a house or terrace, as `Shape::Near` pieces (boxes,
  stepped down a slope in runs of at most 6 m, each piece on the ground under it): its front boundary along the road
  nearest it within 45 m, set back from the road's edge by the region's pavement (1.6 m; America's 2.4, the tropics' and
  dry countries' 0.8, Japan's 0.3) when 1-30 m remain, with a 1.1 m gate before the door; a detached house's sides from
  that line back past the house, half way to its neighbours; a step under every door the shader draws (the same hash
  of the bay, `grounds::door_at`, mirroring `buildings.wgsl`'s `hash13`), a stoop of several risers where the ground
  falls away below it, none where a neighbour's wall stands before it (a terrace's party wall); blocks and towers have
  their steps too. Nothing stands on a road or in another building. By region (`grounds::boundaries`): England's and Wales'
  brick or stone front walls with slate copings (three in five), hedges, timber side fences 1.8 m high; Scotland's and
  Ireland's stone walls; the North Sea's low hedges; Europe's rendered walls and hedges; Scandinavia's
  white or Falu red picket fences; America's open lawns (three in five) or a white picket fence; the American West's
  hedges and low stucco walls; the Mediterranean's and dry countries' rendered walls 1-2.2 m high; Japan's tall hedges,
  board fences and concrete or stone walls; the tropics' mostly open. Walls are the building's own brick or render where
  it is masonry; hedges are boxes shaded as leaves (`PART_LEAF`: speckled clumps of their green, the top as the sides). Yards (a garden's
  own ground) are not drawn: the photograph under the town carries them.
- **Footing** (2026-10-14; naval-client `theatre::buildings`, `Context::footing`). Within the fine relief's reach
  (ground.md, "Fine relief near the camera") a chunk's buildings, grounds and clutter stand on the land as drawn: the window's heights
  with the relief's residual added (`ReliefHeights`, the land layer's own pages), so a house on Palos Verdes' or
  Istanbul's hills no longer stands on a plinth of air or sinks into the slope. A chunk made before the pages under it
  arrived is made again when they do (`Made::relief`, the order-free `ReliefHeights::signature` of the pages within a
  page of it); far chunks stand on the window's heights, as the land does there.
  Cost of the three (Apple M5 Pro, Metal, `test-fast`, 1600 × 900, High, 1942 by day; the scene's opaque pass by the
  GPU's own timestamps, medians of 40 frames, two rounds alternating the binary before and after; other agents on the
  machine): Southsea at 12 and 40 m -0.03 to +0.09 ms, Fatih from 150 m +0.08, Hermosa Beach at 40 and 150 m
  +0.13-0.14. The buildings' triangles rise by 29-38% (the roofs' edges above), their GPU buffers by 17-27 MB (48-68 MB
  before), the clutter drawn by 4,000-36,000 pieces (the grounds and downpipes, 36 bytes each). Stills (not
  committed): `.build/town-lanes/E/pairs/`, `<place>-<view>-before.jpg` and `-after.jpg` (Southsea, Reykjavík, Hermosa
  Beach, Yokosuka, Fatih), in the lane's worktree.
- **Light** as the land's: the sun through the shadow maps and the clouds' shadow, the hemisphere fill and the sky's
  environment (diffuse, and its reflection with Schlick's Fresnel, no DFG table), the scene fog with the sky behind; plus
  the light the sunlit land throws back onto walls (albedo 0.12, a little warm, over the lower half a wall faces):
  without it a shaded white wall took the sky's blue alone.
- **Shadows.** On High the chunks cast into the sun's near and wide maps (`shaders/buildings_caster.wgsl`, the scene's
  own vertex buffers, culled against each map's sides), and every building receives them; Medium casts none. Those
  maps reach about the focused hull (380 m for the wide map in a theatre scene); since 2026-10-08 the render core's land
  maps (native/docs/render-core/shadows.md, "Land maps") follow the camera's view of the land out to 4 km, and the chunks cast into them
  too, culled against each map's sides and against the view's moved out by a 150 m building's shadow (at most
  1.5 km), so a town seen from the air or the free camera shades its streets wherever the ship is.
- **Infill** (`infill`). Where the source has none, generic buildings over built-up cover (WorldCover's class 50 in
  the window's maps): the theatre cut into districts of 420 m, each with its own street grid at a hashed heading and
  offset, blocks of 96 × 52 m between 12 m streets, two rows of lots back to back; a lot is built on where the cover is
  built-up there, more often where the cover about it is (an empty lot now and then in a thin suburb), on land over 1 m
  and slopes under 0.22, wholly inside its district (districts meet along a gap, as along a main road), and never in a
  100 m cell holding any footprint of the source (`occupied`). What stands: houses on 12-17 m lots, set back 3-6 m and
  turned a little; terraces where the town thickens; in a European or Nordic core blocks of three to six storeys filling
  their lots (in the tropics and dry countries rarely, and two or three storeys; three to five in Malta's, four to
  seven in Athens' and the Nile's, two or three in North Africa's, whose houses are of one storey). By region (`infill::Grid`, the region of each
  district's middle, so a district reads the same from either chunk it spans): the American West's blocks 150 × 76 m between 14 m streets (Los Angeles' 50 by 125 ft lots), ranch houses 9-17 m wide on
  14-18 m lots set back 6-9 m, two in three of one storey, a terrace only where the town is thick, a downtown block
  rarely; Florida's blocks 160 × 80 m between 14 m streets, ranch houses on 18-24 m lots, four in five of one storey;
  Britain's, Ireland's and the North Sea's blocks 110 × 58 m between 10 m streets, 8-12 m lots, a terrace on most where
  the town is built over at all (a Scottish or Irish cottage of one storey on two in five of the rest), three to five
  storeys in a core; East Asia's blocks 64 × 34 m between 6 m lanes, houses 6-10.5 m on 9-12 m lots a metre or two from the lane
  and a metre or two apart, of two storeys, a small apartment house now and then, and in its centres three- to
  six-storey blocks standing apart (82% of the lot's width); Hawaii's blocks 120 × 62 m between 12 m streets (Honolulu's 50
  ft lots), 14-18 m lots, three houses in five of one storey, a core's blocks of two or three now and then; Micronesia's
  and the atolls' houses of one storey, a core's blocks rarely, as the tropics'. Every choice hashes the lot's theatre position, so a lot
  gets the same building whichever window or chunk makes it.
- **Settings.** `BuildingSettings::HIGH` (High and Ultra): whole buildings to 3.5 km and boxes to 10 km from the
  camera (its height over 40 m counted), the far level's masses to 30 km across the ground (`far_m`, the height not
  counted: "The far level"), nothing beyond (the photograph carries the town there), casting, the facade textures and
  the roof clutter. `MEDIUM` (Medium, the terrain setting with no trees): whole to 1.5 km, boxes to 6 km, no far level,
  no shadows, the drawn windows, no clutter (1 × 1 placeholders bound, the shader compiled without `FACADE_TEXTURES`:
  as before). `NAVAL_BUILDINGS=plain` draws High without the textures and the clutter, to measure them;
  `NAVAL_BUILDINGS_FAR=off` draws without the far level, and a number sets its range in kilometres, at most 30 (Medium too, past
  its boxes).
- **Period** (2026-10-10, `period.rs`). Graphics → Buildings: `period` (the default) draws the theatre's buildings as
  they stood in the scenario's year (`Period::Year`, from its start; a scenario after 1960 draws today's), `today` as
  the footprints and the map have them now (`Period::Today`, everything above). Taken when a theatre scene is built
  (`Graphics::buildings_period`, the layer's `BuildingSettings::period`, the host's `Context::period` and its made
  meshes' key), so the menu marks the row for the next scene; quality presets keep it. Fewer buildings, older ones:
  [period.md](period.md) says how.
- **Big cities' footprints** (2026-10-08, branch `goldflag/land-buildings`; the host's `theatre::buildings` and its
  `split` module). Microsoft's file for a level-9 quadkey is fetched whole and parsed in memory up to 80 MB compressed
  (`WHOLE_FILE_BYTES`), as before. The densest cities' are larger (105 of the index's files: Los Angeles' `023012311`
  171 MB, Tokyo's `133002112` 300 MB) and were skipped, the infill standing in for the whole city. They are now
  streamed by a thread of their own (`theatre-footprints`, one file at a time): inflated as they arrive (`flate2`'s
  multi-member gzip reader, which checks every member's CRC-32 and length), parsed a line at a time and each footprint
  written to the level-13 quadkey tile holding its middle (about 4 km at Los Angeles' latitude, 256 to a file;
  `buildings-cache/footprints/<region>/<quadkey>.tiles/<tile>.bin`: each footprint its height and its ring in 1e-7
  degrees). Memory stays bounded however big the file: 64 KB a tile buffered before it is appended, 16 MB at most. The
  tiles go into a working folder that is moved into place only once the whole stream was read and checked, so a cut or
  damaged download is thrown away, never kept (a damaged tile read later throws its folder away to be fetched again; a
  failed stream is tried again after 5 minutes). A chunk reads only the tiles under it (1-4), kept in memory with the
  whole files' stores (64 stores or 3 million footprints at most). Meanwhile the chunks under the file are made with the
  infill alone and marked as waiting; once the split lands they are made again (a counter of the splits finished, read
  when a chunk is begun) and the layer swaps them in. The RPC's `buildings.state` is `waiting` while a file streams
  (`worker.streaming`, `worker.streamed_mb`), so a capture waits for the real footprints.

**Costs** (Windows 11, RTX 5070 Ti, release, 2026-10-04). Meshing on the CPU: 49 chunks of Reykjavík (8,032 buildings)
in 20 ms on one thread, 190,000 triangles and 10.9 MB whole, 134,000 and 7.6 MB as boxes (the dataset's footprints
about Tjörnin). In the theatre's cruise scene (1280 × 720, `measure_frames` medians of 40, with the buildings and
without, the same frame; noise about ±0.2 ms): Reykjavík at 150 m 3.53-3.62 against 3.17 ms, at 300 m 3.46-3.71 against
3.28, at 1,000 m 2.61-2.95 against 2.59, looking down from 800 m 2.14-2.32 against 2.08, from a ship 2 km off
2.34-2.59 against 2.28: 0.0-0.45 ms, about 0.45-0.50 million triangles in 80-84 chunks drawn at 150-300 m; Honiara
(the infill alone) 0.04-0.4 ms. Memory on the GPU 29-38 MB for Reykjavík's 352-365 chunks within 10 km (0.51-0.66
million triangles, 30,000-37,000 buildings), 8-10 MB for Honiara's. Uploads go through `queue.write_buffer`, at most
8 MB a frame.

The facade textures and the roof clutter (Apple M5 Pro, Metal, `test-fast`, 1920 × 1080, 2026-10-08, other agents
building and rendering on the same GPU): the scene's opaque pass, its p50 over 240 frames of the GPU's own timestamps,
High against `NAVAL_BUILDINGS=plain` in alternating runs over Los Angeles (Redondo Beach from 150 m, Hermosa Beach's
street from 40 m and from 500 m overhead, San Pedro's roofs from 120 m). In the quietest pair +0.21-0.25 ms on three
views and -0.05 on the fourth; the noisier pairs scattered -0.8 to +0.6 ms. 121-6,000 clutter pieces drawn; the near
shadow map's cost and the nearest land map's (0.01-0.8 ms a frame with them, 0.06-0.5 without, the GPU shared with
three other lanes' builds) not told from without. The bake 120 ms once a run, off the frame's thread; 6.7 MB of
textures on the GPU and 36 bytes a clutter piece. Medium is unchanged (neither drawn nor bound beyond 1 × 1).

With the map's types (Apple M5 Pro, release, 2026-10-05, other agents building: load average 7-12): meshing 49 whole
chunks about Tjörnin 18 ms with the map against 7 ms without on one thread (9,919 buildings and 225,000 triangles
against 8,036 and 191,000: OSM's outlines add a fifth), the tags 5 ms of it (`the_maps_types_cost_over_reykjavik`).
Drawn over a town at the same view, the triangles grow 4-9% over Monterey and 6-8% over Reykjavík (the parts and
OSM's outlines together; box chunks read zoom 13, whose small buildings are left out); Honiara's 2-14 times, Microsoft
having no footprints there and OSM thousands (the 1,300-4,100 buildings held within 10 km, the infill's, become
7,300-12,500, most of them mapped; 6-12 MB on the GPU). Whole frames with the buildings and without could not be told apart from the machine's
noise (±0.5 ms at 6-13 ms a frame, the same as before the change): docs/archive/native-theatre-history.md, "Buildings",
Cost.

The parcels (Windows 11, i7-12700K and RTX 5070 Ti, `dev-client`, 2026-10-28, 1600 × 900 at High in 1942, other
sessions building and running their clients on the machine; `theatre_town_look` on master and on the change, each
view against itself): the buildings held grow by a tenth to two fifths over Taranto, Piraeus, Reykjavík and Dover
(Taranto's 37,700-38,400 become 42,600-43,300, Piraeus' 125,000-166,000 become 151,000-234,000) and nearly double
over Valletta (35,600-37,100 become 67,000-71,400); the triangles drawn by 16-17% over Reykjavík, 26-40% over
Piraeus, 36-43% over Taranto, 43-47% over Dover and 49-79% over Valletta; the buildings' GPU memory by 16-17%,
24-34%, 20-25%, 31-37% and 46-54% (Piraeus' 52-137 MB become 65-180, Valletta's 31-53 become 46-78). Honiara's 15-49
buildings of 1942 are unchanged, none a block or a row. The party walls take back 1-2% of the triangles over Dover,
2-3% over Taranto, 3-5% over Piraeus and 6-7% over Valletta. The GPU's time for the buildings (timestamps about its
passes, seven rounds with them and without; the parcels on and off in alternating runs of one binary, before the
party walls were culled) moved -1.8 to +1.6 ms in the opaque pass over eleven views of Piraeus, Valletta and Dover,
-0.25 ms on average, and -0.2 to +0.5 ms in the shadows': not told from the other sessions' load on the GPU. Whole
frames in `dev-client` are the CPU's (15-20 ms against the GPU's 4-10) and scatter by several milliseconds between
runs, more than the buildings change them. Meshing on the worker (`dev-client`, opt-level 1, one thread, the fastest
of four rounds; the parcels on and off in one binary by a switch since removed; 7 × 7 chunks about each town with the
map's tags, today's buildings and no land cover, so a block is cut wherever its outline allows): Valletta's 49 whole
chunks take 493 ms against 384 (+28%; 29,300 buildings against 14,900), Piraeus' 661 against 545 (+21%), Taranto's
216 against 179 (+21%) and Reykjavík's 412 against 373 (+11%); as boxes 176 against 116 ms (+52%), 198 against 122
(+62%), 50 against 35 (+44%) and 77 against 57 (+35%). The slowest whole chunk is Piraeus' at 44 ms against 39; one
chunk gains at most 10 ms whole and 7 as boxes.

**Checked.** `cargo test -p naval-terrain --lib buildings`: rings cleaned, oriented and simplified; the rectangle about
a rotated box; an L-shape triangulated whole; a square extruded to a closed box (normals out, every triangle wound
counter-clockwise about its normal, the walls from 1.5 m under the lowest ground, three bays on a 10 m wall); an L
extruded with its 300 m² roof; gabled and hipped roofs closed over the walls with the ridge where it belongs; heights
from the data when sane and from the footprint otherwise; the density grid; the infill deterministic, off the sea and
off the cells the source holds; the level of detail's margins; culling by planes. Since 2026-10-05: on industrial land
a large footprint is a warehouse or factory (never a house), factories' chimneys within their footprints; a place of
worship's footprint a church with its tower at the west end under a spire (north of Europe) or a campanile's pyramid
and a dome (now and then, the Mediterranean), a mosque's dome and two minarets, OSM's 74.5 m the tower's top; a round
footprint on industrial land a tank (not in a residential street); a lighthouse's point a lighthouse; blocks on
commercial land offices; OSM's colour on the walls; the regions by place (`region.rs`'s tests: London, Portsmouth,
Plymouth, Dover, Cardiff, Holyhead, Carlisle, Whitehaven and Berwick British, Edinburgh, Glasgow, Gretna, Annan,
Eyemouth, Kirkwall, Lerwick, Stornoway, Dublin, Belfast, Cork and Douglas Celtic, Reykjavík, Tórshavn and Nuuk
Icelandic, Bergen, Oslo, Gothenburg, Helsingborg, Malmö and Stockholm Nordic, Den Helder, Amsterdam, Antwerp,
Dunkirk, Hamburg, Kiel, Aarhus, Skagen, Helsingør, Copenhagen and Rønne North Sea, Paris, Cherbourg, St Peter Port,
Berlin and Gdańsk temperate, Miami, Key West, Tampa and Daytona Beach Floridian but Jacksonville, Pensacola, Savannah,
Charleston, Norfolk and Boston American and Havana, Nassau, Freeport and Bimini not, Valletta, Sliema and Gozo
Maltese, Athens, Piraeus, Thessaloniki, Kavala, Alexandroupoli, Orestiada, Florina, Serres, Corfu, Ioannina, Crete,
Gavdos, Lemnos, Mytilene, Chios, Samos, Kos, Symi, Rhodes, Kastellorizo, the Cyclades and Cyprus Greek against Sarandë, Vlorë,
Bitola, Petrich, Edirne, Enez, Ipsala, Çanakkale, Bozcaada, Ayvalık, Çeşme, İzmir, Kuşadası, Bodrum, Datça, Marmaris, Kaş,
Fethiye and Antalya, Tangier, Tetouan, Nador, Casablanca, Rabat, Agadir, Tan-Tan, Dakhla, Oran, Algiers, Annaba,
Bizerte, Tunis, Kelibia, Sfax, Tripoli, Benghazi, Tobruk, Marsa Matruh, El Alamein, Siwa, Suez, Hurghada, Marsa
Alam, Sharm el-Sheikh, Nuweiba and El Arish North African but Ceuta, Melilla, Tarifa, Gibraltar, Algeciras, Almería,
Cartagena, Marsala, Gela, Pantelleria, Lampedusa and Funchal Mediterranean and Gaza, Eilat, Aqaba, Haql, Duba, Haifa,
Beirut and the Canaries arid, Alexandria, Port Said, Damietta, Tanta, Mansoura, Cairo, Giza, Faiyum, Minya, Asyut,
Sohag, Nag Hammadi, Qena, Luxor and Aswan the Nile's, Istanbul, Marseille and Lima Mediterranean, Houston, New York, Seattle, Portland and Denver American,
Monterey, Los Angeles, San Diego, Phoenix, Las Vegas, Salt Lake City, Albuquerque and El Paso American West but
Tijuana, Nogales and Ciudad Juárez across the border Mediterranean, Tokyo, Kamakura, Yokosuka, Sapporo, Naha, Busan,
Taipei and Shanghai East Asian but Vladivostok not, Honolulu, Waikiki, Pearl City, Kailua, Lihue, Hanalei, Lahaina,
Kahului, Kaunakakai, Lanai City, Hilo and Kona Hawaiian, Hagåtña, Tamuning, Apra Harbor, Garapan, Tinian, Rota,
Koror, Peleliu, Yap, Ulithi, Chuuk, Pohnpei, Kapingamarangi, Kosrae, Wake and Midway Micronesian, Betio, Bairiki,
Bonriki, Butaritari, Majuro, Kwajalein, Ebeye, Roi-Namur, Jaluit, Enewetak, Nauru, Funafuti, Fakaofo, Kanton,
Kiritimati and Tabuaeran atolls, Honiara, Auki, Kirakira, Lata, Rabaul, Buka, Kavieng, Lae, Port Moresby, Jayapura,
Tacloban, Manila, Davao, Morotai, Suva, Apia, Port Vila and Papeete tropical) and the roofs by
region (terracotta on more than 85% of Mediterranean houses, slate and dark tile on 60% of temperate ones, iron on 75%
of tropical ones, four in five dry country's flat, shingles on more than 55% of the American West's with 8-40% flat
and under 15% terracotta, kawara, glazed tile and sheet metal on more than 85% of East Asia's and no terracotta,
slate and dark tile on 85% of British houses with brick on half their walls and no orange, slate on 90% of Scottish
and Irish ones and no brick, pantiles on 40% of the North Sea's and brick on 65%, Florida's half hipped and never
brick, Malta's flat over honey limestone, Greece's 35-75% flat with terracotta on the rest, North Africa's
flat and never brick, the Nile's flat over brick, no tar or asphalt on a warm country's flat roof, Hawaii's mostly
hipped shingle and iron and never terracotta, Micronesia's 50-80% flat, an atoll's tin and thatch); British, Irish and
Dutch terraces where built over, Florida's houses of one storey, Malta's townhouses, Athens' and the delta's narrow
blocks, Hawaii's houses of one and two storeys, Micronesia's and the atolls' of one; a terraced house's
ridge along its street; the
infill's suburbs by region (the American West's houses 1.3 times the temperate's area and mostly of one storey, East
Asia's smaller, of two storeys and 1.3 times as many, with mid-rise blocks in the middle), walls,
seeds and shutters varied; prisms, cones, domes, beams and boxes closed with their normals out and within their
circles; a church's tower, spire and dome within its nave's footprint, a lighthouse within its circle, a crane and a
minaret faced out and the crane dropped from a box chunk; sawtooth roofs and tanks closed within their footprints; a
chunk with a church's point, a lighthouse landmark in the sea (on its plinth, its lantern lit) and one taking a
footprint, a port's quay and a pier raising each; the setting off the map (a school within a residential zone, a hole
in a zone, the sea by a round footprint, a pier's deck); OSM outlines lending heights (none or capped) and colours
and adding what the footprints miss (not parts above the ground or `hide_3d`); CSS colours parsed; quays found along
industrial land at the water (by the ground, or by mapped water); a street through an infill lot; features cut to
their tiles (a square across two tiles halved, a line's pieces). Since 2026-10-08: the facade textures whole (every
layer and mip, under 16 MB), every window holding glass (doors, a garage, a roller shutter and a belfry little or
none) with the wall round it (but a curtain wall, a shop front and a Japanese balcony), shutters and doors taking the
accent, each wall material the same a tile
over and brick's mortar 8-30% of it, the shader's layer constants in the textures' order; the clutter's shapes closed
with their normals out, a block's housing, units and vents on its roof within its walls, Japanese blocks' tanks and
New York's water towers in their shares, a British terrace's stacks on its ridge and no chimney on a Japanese house.
Since 2026-10-28 (`parcels`): a Mediterranean block of 72 by 20 m drawn as one stands as four to eight buildings
within it, their areas the block's, their storeys within two of its and not all alike, each with its own seed; a
British row of 48 by 9 m as seven to eleven houses 4-11 m wide, within a storey of the row's, most of them pitched
and in its brick; a block of 90 by 60 m as its rows and ends about one court of one storey at its middle; a U's arms
never parted by a cut; a line's crossings counted on the outline's edges; a house, a deep terraced house, a pair of
semis, a short block, a block on industrial, civic or military land and an infill lot left whole; a block cut alike
twice; the fixture chunk's pins again (`todays_chunk_is_as_it_was`: its measured 20 m block and estimated blocks
cut); two pieces back to back, the low one's wall against the tall one gone and the tall one's standing from the
low one's top with its window rows running on (`a_wall_against_its_neighbour_stands_only_above_it`), and a block's
two rows parted at different places finding every neighbour along each edge
(`a_blocks_pieces_find_their_neighbours_across_both_rows`). `--test all -- theatre_town_look::` (ignored, network;
`NAVAL_LOOK_PLACES`, `NAVAL_LOOK_TAG`, `NAVAL_LOOK_FRAMES`; each place in a process of its own) moves a copy of Île de
Batz to Taranto, Valletta, Piraeus, Reykjavík, Honiara and Dover and captures each from the sea, a plane and the
street into `.build/native/towns/<tag>/`, with the buildings each view held and, with `NAVAL_LOOK_FRAMES`, its frame
and the GPU's passes timed with them and without.
`--test all -- backends::` translates `buildings.wgsl` with and without `FACADE_TEXTURES` and `buildings_caster.wgsl` with
and without `CLUTTER_CASTER` for Metal, DirectX and Vulkan. `--test all -- buildings::` (GPU, High: the textures and the
clutter bound and drawn): a chunk
handed in is drawn where its corner and the offset put it, lit, culled when out of view and gone once removed
(`.build/native/terrain/buildings-gpu-test.png`); a 60 m tower shades a low block's roof through the sun maps
(`buildings-gpu-shadow.png`). Captures in the theatre: docs/archive/native-theatre-history.md, "Buildings".

**Open.** Shadows onto the land beyond the land maps' 4 km (a town farther off stands on unshaded ground; a stand-in,
the footprints projected along the sun darkening the land once each, wants a mask pass of its own, since blending
them would darken their overlaps twice; at that distance a 10 m house's shadow is a few pixels long). Courtyards are
roofed over (a block drawn as one and deep enough has its court since 2026-10-28, `parcels`; a footprint's own holes
are dropped) and a multipolygon keeps its first
polygon. The far level's own open items are in "The far level", below. The infill's streets are its own grid, not the roads in the photograph (its lots keep
off the map's roads since 2026-10-05; the land under the buildings has drawn no lots of its own since the same day:
surface.md, "Under real buildings"). Medium's shorter range was not looked at in a capture. The types
(2026-10-05): no roof shapes or roof colours from OSM (the schema carries neither); a church's tower is a square on its
footprint's west end whatever the church's own plan (Hallgrímskirkja's stepped wings are a plain tower); domes, minarets
and towers stand on a church however the real one is built; a measured footprint's kind follows its height first, so a
measured church with no point on it is a block or a tower; the balconies and shutters are drawn, not modelled, and read
only within a few hundred metres; cranes and lighthouses are guesses or a short list
(docs/archive/native-theatre-history.md, "Buildings", Open). The parcels (2026-10-28): a block is cut across its long
side only, so an L's or a U's other arm stands whole beside its cut arm, and a block's corner buildings are no taller
than its others (a real street's tall corners are its own); the frontages and steps are a region's spread, not the
town's own (Valletta's planned grid and Taranto's old town alike broken); a wall against a pitched neighbour stands
from that neighbour's eaves, its gable not counted, and a piece of a block whose box level simplifies its outline
keeps the walls it no longer shares; Britain's merged rows over 220 m² where more than 45% is built are still the
estimate's blocks of four to six storeys, cut into houses of that height rather than lowered to a terrace's two or
three.

## The far level

`Lod::Far` (`buildings/far.rs`) stands the town past the boxes as coarse masses, to
30 km across the ground on High, made from what the window already holds: the land cover (WorldCover's built-up class
in the window's maps), its heights and, in a past period, the period's ground. Nothing is fetched and no footprint is
read: past 10 km a chunk's footprints would cost their files, the map's tiles and the meshing for buildings a pixel or
two across. The footprints held in memory are not used either: they cover only the chunks the near levels made, so the
masses would change their grain where they end.

- **Masses.** On the infill's own street grid (its districts, headings, blocks and dense cores: `infill::Grid`), each
  row of lots on built-up land is cut into runs of about 30 m (`RUN_M`; at most 1,000 runs a square kilometre,
  `MOST_RUNS_KM2`, so East Asia's small blocks run whole rows), and each run is one mass: as long as the share of its
  lots built on, as deep and as high as the infill's buildings there (houses, terraces or a core's blocks by the
  region's rules), set back as they are, under a gable at the region's pitch or flat, in the region's palette for the
  period. A mass is 24 bytes (`far::Mass`), an instance of one shape (`far_shape`: 30 vertices) sized, turned and
  roofed by `buildings.wgsl`'s `vs_far` and lit by the buildings' own fragment shader (the windows' mean by day, lit
  at night). A built-up chunk holds 700-900 runs and 115-215 blocks' masses, about 19-27 KB.
- **Blocks.** From 10.5-12 km across the ground (each block by its hash) a block's runs give way to one flat mass over
  them at their roofs' mean height, in its longest run's colours (`Mass::BLOCK`; East Asia's 64 × 34 m blocks two by
  two, `GROUP_M`), the runs standing on 500 m past it so no block stands empty between the two
  (`BuildingSettings::blocks`, the uniforms' `far`). A chunk's runs come first in its masses and its blocks after: the
  layer draws its runs, its blocks or both by its distance (`mass_range`), the blocks without gables, and the shader
  picks each block's. Over the far level's last 3 km the masses sink into the ground (`FAR_FADE_M`), so none pops in or
  out at its edge.
- **Across the ground.** Whole and boxes are chosen by the distance from the camera, as before; the far level by the
  distance across the ground (`lod_at`'s `across`), so a climbing camera keeps its ring of masses (from 10 km up there
  are no boxes, only masses below). `wanted` reaches 30.25 km (`reach_m`): about 2,650 chunks about the camera,
  nearest first. Going from boxes to masses and back the chunk cross-fades ("Chunks", above).
- **Period.** Both Graphics → Buildings settings draw it, and today's near levels are unchanged
  (`todays_chunk_is_as_it_was`). In a past period a mass stands only where `period_land` says the ground stood (the
  buildings' thinning where it ran, the cover's estimate beyond), no taller than the region's period storeys and the
  place's cap (`thinning::height_cap`); a chunk whose period ground is not known yet stands empty and is made again a
  minute later, and one whose ground is answered anew (the buildings' thinning as the camera comes near it, a window's
  estimate) is made again (`period_land::changed_since`).
- **The host** (`theatre::buildings`). Far chunks are made 64 at a time on up to four threads (`make_far`, nothing
  fetched), after the near chunks queued before them: about 2,600 in 1.4 s once a scene's footprints are in. A chunk
  the window's maps cover only in part is made again a minute later. With thousands of chunks wanted, the feed looks
  them over again only when the camera has moved 100 m, the worker has made something (`MADE_VERSION`), a big file's
  split has landed, or every half second. While a far chunk's masses are made, its boxes stand in (`other`).

**Costs** (Apple M5 Pro, Metal, `test-fast`, 1280 × 720, High, 2026-10-10; `measure_frames` medians of 40 in seven
alternating rounds of the far chunks drawn, not drawn and no buildings at all, the same frame; other agents building
and rendering on the machine, noise about ±0.5 ms; `cargo test --profile dev-client -p naval-client --test
theatre_buildings_far -- --ignored --nocapture`). Today's buildings, ms a frame:

| View | Masses drawn | With the far level | Without it | No buildings |
| --- | --- | --- | --- | --- |
| Istanbul from 3,000 m, looking down | 46,000 | 11.88 | 11.27 | 9.51 |
| Istanbul from 3,000 m, looking out | 104,000 | 11.23 | 9.65 | 8.66 |
| Istanbul from 4,100 m, looking down | 77,000 | 11.91 | 10.85 | 9.21 |
| Istanbul from 4,100 m, looking out | 115,000 | 10.87 | 9.12 | 8.44 |
| The Los Angeles basin from 4,000 m | 116,000 | 9.85 | 7.56 | 6.71 |
| Yokohama and Tokyo Bay from 3,000 m | 133,000 | 10.92 | 9.08 | 8.14 |
| Portsmouth from 1,500 m | 29,000 | 10.34 | 9.74 | 8.71 |

So 0.6-2.3 ms, most looking out across a large city, by the masses drawn. In the period (1941-42, fewer masses: 9,700-
44,000 drawn) 0.1-0.5 ms over Istanbul and Portsmouth, 1.3-1.4 over Los Angeles and Yokohama. On the GPU the masses
are their 24 bytes and nothing else (the shape about 560 bytes, once): today 16-17 MB over Istanbul (680,000-720,000 masses in
about 2,650 chunks), 21 MB over Los Angeles (870,000), 23 MB over Yokohama (946,000), 5 MB over Portsmouth; in the
period 2.6-7 MB. The way there: runs alone to 30 km cost 2.3-2.9 ms looking out over Istanbul (227,000-279,000 masses
drawn); blocks from 12-14 km 1.9-2.5; from 10.5-12 km as now. Yokohama with East Asia's blocks one by one and its runs
of 30 m held 2.03 million masses (49 MB) and drew 341,000 for 3.1 ms. Drawing the masses with a fragment shader that
does nothing saved 0.5-1 ms of Istanbul's worst 2.5: the rest is their vertices. Medium has no far level: drawn to
16 km (`NAVAL_BUILDINGS_FAR=16`) it cost 1.1-2.7 ms over Istanbul, since from 4 km up Medium's boxes end 4.4 km across
the ground and the masses from there are large on the screen (0.4 ms over Los Angeles).

**Checked.** `--lib buildings::far`: masses on built-up dry land only, a chunk the same made twice and no run in two
chunks, the period's ground and caps, the shape closed with its normals out and its gables last, a core higher than
its suburbs, pitched and flat roofs by region, each block's mass over its runs (its top within theirs, its foot under
theirs), 600-1,000 runs and 100-240 blocks to a built-up chunk in every region. `--lib buildings::layer`: the levels'
margins (boxes and masses both ways, the far level's edge), a climbing camera keeping its ring, a far chunk's runs and
blocks by its distance, the fade's codes complementary and carried in the translation. `--native -p naval-client --lib
theatre::buildings`: the wanted chunks (masses from 10 km across the ground, from 12 km up nothing but masses), a far
chunk made from a tropical town's cover. `--test all -- backends::` translates `buildings.wgsl` with `BUILDINGS_FADE` too.

**Open.** The cost looking out over a large city (1.5-2.3 ms on High); the next levers are a cheaper fragment path for
the masses (no windows' mean or sky reflection) and a flat block's top of four vertices rather than two slopes of
eight. The masses stand on the infill's grid, not the city's streets: the photograph's boulevards and parks show only
where the cover is not built up (Los Angeles' grid of boulevards is covered). The towers and skyline end with the
boxes: the far level makes a core's blocks to its region's storeys, not its measured towers. In a past period the far
level is as good as `period_land`'s ground: in these stills Istanbul's 1942 ground left most of the historic
peninsula, Galata and Üsküdar as fields, as the near levels did, until the old towns were listed (period.md, "Old towns", under
"Fewer buildings in the period"); a city whose old extent is not listed still thins by today's density. The ground under a mass is the window's
heights (40-140 m samples): on a slope a mass sinks or overhangs a little. A scene built again after the camera has come
nearer does not let a chunk's cached masses stand in for its boxes: that chunk stands empty until its boxes are made,
as any chunk did before the far level.

History (stills and the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
