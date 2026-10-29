# The period

Graphics → Buildings: `period` (the default) draws a theatre as it stood in the scenario's year (`Period::Year`),
`today` as mapped now (`Period::Today`). The period changes the buildings' finish, thins them to what stood, lays the
period's country over the ground they leave, and draws the period's roads. The buildings themselves are in
[buildings.md](buildings.md); the ports' and bases' period in [infra.md](infra.md).
## Built-up history

`buildings::history::stood_share(lat,
lon, year)` is the share (0-1) of the land built up today about a place that stood in `year`, which the period's
thinning keeps; none where the history has no data.

- **Source.** HYDE 3.3 (History Database of the Global Environment, Klein Goldewijk et al., Utrecht University and
  PBL, doi:10.24416/UU01-94FNH0, CC BY 4.0; `history::ATTRIBUTION`), baseline scenario, `uopp`: each 5 arc-minute
  cell's built-up area in km² (4320 × 2160 cells, 9.3 km north-south, 7.7 km east-west at 34° N). The game has no
  data-sources credits page; the attribution lives with the code, as the other sources' do.
- **Bake.** `python3 scripts/theatre/bake-built-up-history.py` fetches the 1900-1960 decades and 2023 (eight
  `<year>AD_pop.zip`, 317 MB, about a minute side by side; the data platform's browser check is passed by doing its
  proof of work) into the ignored `.build/history/` and writes `buildings/history.nbh`: every cell built over by more
  than a hectare in 2023 (157,469), each with one byte per decade, clamp(uopp(year) / uopp(2023), 0, 1) × 255. 474 KB
  committed (1.27 MB unpacked), compiled in with `include_bytes!` and decoded once on the first lookup (`OnceLock`): a
  row index and a binary search in the row. 1940's kept cells hold 185,000 km² of the 832,000 built up in 2023.
- **Lookup.** Bilinear between the four nearest cell centres, weighted over those with data (a place beside a town's
  last cell reads the town's share); linear between decades, toward 1 at 2023; a year before 1900 reads 1900's.
- **How far to trust it.** HYDE spreads each country's historical urban population over today's settlement pattern
  and turns it into area by a national, time-dependent urban density, so a cell's past follows its province and
  today's ranking, not that town's own story. At 9 km a city's share is right in kind and often wrong in its
  suburbs. 1942 / 1950: Los Angeles downtown 0.67 / 0.79, the San Fernando Valley 0.45 / 0.55 (two or three times
  too many: it was mostly farms), Long Beach 0.43 / 0.53, central Tokyo 1.0 / 1.0 (its core filled the cell by
  1940), Yokosuka none (HYDE puts its urban people in Yokohama's cells, none in its own, even today), Reykjavík
  0.12 / 0.15, Honiara 0.01 / 0.02 (right: a few buildings), Portsmouth 0.60 / 0.66, Valletta 0.61 / 0.67, Honolulu
  0.75 / 0.75.
- **Checked** by `--lib history`: the decoder against the bake's layout, a cell centre's share between decades,
  the weights renormalised over the cells with data, the date line and the poles, and the baked file's decades and
  places. `report_the_places` (ignored) prints the table above.

## Period styling

A building planned in a past period (`Period::Year`) keeps the footprint, kind and height the thinning and the heights
give it and takes an older finish; `Period::Today` draws exactly as before (fingerprints below hold it).

- **Palettes** (`palette::past_colours`, `past_flat_roofs`, `past_roof_rules`). More brick, stone, timber and render and
  fewer pastels (under two fifths of today's share on houses and blocks); flat roofs in tar, gravel, bitumen felt and
  concrete (limewash and screed in the warm countries), never a membrane; no glass, panel or coloured-sheet walls. By
  region: the American West's houses stucco and clapboard under clay tile (two roofs in five), wood and composition
  shingle, no concrete tile; America's houses painted clapboard (white, cream, yellow, grey) with brick in the East's
  towns, its rows brick and brownstone; Britain's brick, slate and harl (no white render), the North Sea's and Europe's
  brick, stone and render, all sootier; Nordic timber as now; Japan's houses charred and dark-stained boards
  (`palette::CHARRED`, a new wall entry) and white shikkui plaster under kawara (since 2026-10-14 smoked ibushi
  kawara on two in five, weathered grey on nearly a quarter, black glazed on one in seven: `palette::IBUSHI`, `GLAZED`,
  new roof entries), anthracite or black tile and rusted tin now and then, no blue or green sheet, its few blocks Western brick and stone; the Solomons' and New Guinea's (the tropics')
  timber under corrugated iron and sago-leaf thatch (three in ten); Hawaii's and Micronesia's plantation timber under
  iron and shingle, Micronesia's houses one in seven flat (today's two in three are post-war concrete); offices
  limestone, stone, brick and cream render; Malta, the Aegean, North Africa and the arid lands much as now, their
  pastels and bright whites dropped. More low blocks are hipped: Europe's and the Mediterranean's under slate and tile
  (two in five to more than half), the tropics' and the atolls' under iron (two in three, one in two), Japan's under
  kawara (nearly half), Hawaii's and Micronesia's under iron (about two in five). OSM's wall colours (today's paint) are
  not taken.
- **Walls** (`buildings.wgsl`'s `wall_material_past`). Panels and glazed tile become render (stucco in a warm country);
  render is clapboard on an American house and a third of the American West's and Florida's, boards or weatherboards on
  most of Hawaii's, Micronesia's, the tropics' and the atolls' houses (corrugated sheet on a few), plaster on a Japanese
  house, whose dark boards lie as weatherboards (shitami-ita) on half; Nordic and Icelandic houses as now. Britain's,
  Ireland's, the North Sea's and temperate Europe's walls are darkened to 0.8 (soot).
- **Windows** (`window_style_past`, seven facade cells after today's 22: `CarriageDoor`, `Shoji`, `Amado`, `Punched`,
  `Awning`, `Signboard`, `Louvres`). No aluminium sliders, curtain walls, ribbons, garage or roller doors, jalousies, or
  Japanese aluminium windows and balconies. A house's ground floor has its door, and in America a wooden carriage door
  on one bay in twelve; a Japanese house glazed wooden sliding doors (small lights over paper panels) with their amado
  closed over one bay in five; the American West sashes and casements, Florida and Hawaii sashes and wooden louvres, the
  tropics louvres and shutters; Europe's sashes and casements as now. A block's or office's upper floors are punched
  six-over-six sashes in stone surrounds (casements and shutters in the south, grilles in dry countries, louvres in the
  tropics), iron balconies where they stand today (none in Japan), and a stone cornice with its shade along the top
  storey's head; half the blocks' ground floors are shops under a striped awning or a painted signboard. A warehouse's
  big doors are boarded. Windows keep their night lighting; a scenario's dimout or blackout darkens them as before.
  Medium's drawn windows: an office's are punched, Japan's blocks carry no balconies, and the cornice and soot are drawn
  too.
- **Roof clutter** (`clutter.rs`). No air-conditioning units, coolers, solar heaters or tanks (Japan's blocks' tanks
  too); vents stay where they stand today. Housings on two in five of the blocks of five storeys or more. New York's
  water towers as now. Chimneys on 97% of British and Irish pitched houses, 90% of temperate, 85% of North Sea, American
  and Nordic, 60% of Icelandic and the American West's, none on Japanese; Europe's blocks carry brick stacks, every 8-10
  m along a pitched block's ridge and two to eight on a flat one.
- **The shader's period**: the chunk translation's w is its region plus 32 (`layer::PAST_BIT`) when its mesh was made in
  a past period (`ChunkMesh::period`); the vertex shaders split it into the region and `past`, and every period branch
  keys on `past`, so a chunk always draws in the period its colours and clutter were made in. The caster is unchanged.

**Checked.** `--lib buildings`: fingerprints of today's palettes, roof rules and detail shares over every region and
kind, of today's roof clutter on eleven shapes in every region, and of today's 22 window cells and nine wall materials,
each as on master; the period's palettes hold no membrane, glass or coloured-sheet wall and under two fifths of the
pastels, the West's no concrete tile, Japan's no blue or green sheet; the period's clutter no steel wider than a vent,
fewer housings, New York's towers, chimneys on most pitched houses, stacks on European blocks, and no more pieces than
today's where it adds no chimneys; the new cells whole (doors and amado little glass, shops edge to edge);
`window_style_past` names none of today's modern cells and `wall_material_past` turns panels and tile to render; the
translation's w today is the region alone. `--test all -- backends::` translates `buildings.wgsl` for Metal, DirectX and Vulkan.

**Costs.** The facade arrays hold 29 window cells (seven new): 7.8 MB with their mips, 1.2 MB more, baked in 157 ms
here. A chunk's mesh is as today's but for the blocks newly hipped (Southsea's near view 232,532 triangles drawn against
232,570). Roof clutter falls by a fifth to two thirds (drawn pieces: Hermosa's near view 4,235 against 5,278, Southsea's
5,512 against 6,968, Reykjavík's 2,592 against 5,314, Yokosuka's 1,584 against 5,202, Honiara's 220 against 410), so the
period's GPU bytes are 0.1-0.3% under today's. The shader gains a flat varying and two branch functions, no bindings.

**Open.** A tropical town's 1942 colonial stores and Chinese shops are drawn as its houses, its blocks in stucco rather
than boards; OSM's mapped churches, mosques and temples keep today's colours. A second headless `Session` in one process
fails the ships' frame bind group (a `Depth32Float` view where a filterable float one belongs), so the stills test runs
itself again for each place and mode.

## Fewer buildings in the period

Code: `buildings::thinning`, the host's `theatre::buildings::period`. In `Period::Year` a chunk draws only the footprints and infill lots that stood, no
taller than their place allowed; `Period::Today` is untouched (`todays_chunk_is_as_it_was` pins a fixture chunk's
vertices, indices and clutter at both levels of detail in three regions, and the infra counts, from master).

- **Whole neighbourhoods.** Each 100 m cell scores the ground Microsoft's footprints cover (each counted to 2,000 m²,
  a measured one of 12 m and more to 4,000), smoothed over some 400 m, plus a low noise in theatre metres so the
  kept land's edge wanders. The host reads the chunk's and its eight neighbours' footprints from its stores
  (`period::Gather`), never the drawn level of detail's, so a cell scores the same from any chunk or window and the
  kept land has no seam. Where none of Microsoft's footprints lie about a chunk (Honiara), it and its neighbours score
  by the infill the cover would raise on all their built-up land (`Gather::cover_built`), seam-free likewise; without
  the cover maps, by the chunk's own drawn footprints. A footprint (or lot) stands when its score, read between the
  cells' centres and lifted by an old town's marks (on a place of worship +0.25, within 150 m of one up to +0.1,
  railway and military land +0.15, a pier or industrial land at the water +0.12), reaches its cell's bar.
- **The bar.** Per 5' cell of the history's grid: the score that keeps its share of the cell's footprints (a 1024-bin
  distribution of the cell's footprints by score, made over every chunk the cell reaches and kept, 256 cells), never
  under a floor (0.15 in the Americas, 0.10 elsewhere: the country's scattered houses go); with fewer than 300
  footprints, a bar by the share alone. A 100 m cell blends the four 5' cells' bars about it, the nearly empty ones a
  fiftieth as much, so there is no edge along the 5' grid and a coast's town is not thinned by the sea's cell.
- **The share** (`period_share`), for the coordinator's HYDE bias: the history's share sharpened, `s^1.5` in the
  Americas and `s^1.25` elsewhere (post-war suburbs read two or three times too built up; a core near 1 stays near 1),
  so within a cell the kept footprints are the densest; where it gives a cell under a quarter of the mean of the nine
  about it (San Pedro's 1942 is 0.0 beside 0.37 and 0.45), three quarters of that mean; where it has no data
  (Yokosuka) a share by region (0.55 Britain, 0.35 East Asia, 0.4 the US east, 0.3 the West, 0.1 the tropics, about
  1942, more after).
- **Working waterfronts** (`thinning::Waterfront`; branch `goldflag/period-waterfronts`, 2026-10-10). A port whose
  town stood keeps its working waterfront of the time whole: every footprint on industrial or military land or a broad
  pier within 350 m of water (the map's sea, docks and rivers, or ground 5 m under the sea; a 25 m chamfer grid over
  the chunk and 350 m about it), big sheds and all, whatever its score, under the period's caps.
  - *A port* is a chunk with a ferry terminal or a harbour point about it (`infra::containers::port_about`) whose own
    5' cell keeps at least 5% of today's built-up land in the year (`port_stood`): San Pedro and Yokosuka (by its
    region's share) in 1942, not Honiara.
  - *Not lifted:* a container terminal's yard as the stacks' rule finds it (`infra::containers::YardTest`: deep water
    within 600 m, amid the port's land, few of OSM's outlines), a terminal's apron (within 80 m of the water with that
    yard 70 and 110 m inland of it), and the land in a listed circle not yet made (`LATER_LAND`, below) or a listed
    yard before its year. A post-war logistics or petrochemical estate inland of 350 m is thinned as any land; one on
    the water is caught only by the list.
  - *On top of the share.* These footprints stand on top of the 5' cell's share rather than inside it. HYDE counts
    urban settlement, which a dockyard's acres seldom were. The share's distribution is gathered without the map's
    tags (taking the yards out would need zoom-14 tiles over every 5' cell, 9 km across, for a few percent). And most
    of a yard's sheds score under the bar anyway (each counted to 2,000 m², few to a cell), so standing them takes no
    house's place.
- **Listed yards** (`thinning::YARDS`): the historic naval dockyards and shipyards, each a few circles (latitude,
  longitude, radius) and the year it opened. Within one, every footprint stands from its year (caps apply), whatever
  the town or the map says; before it, the land is thinned as any and the waterfront rule leaves it. `LATER_LAND` lists
  land made or turned to industry after the period (a container terminal on fill, a post-war estate) where the
  waterfront rule is not applied before its year. Both lists give each entry's source in its comment: the yard's
  Wikipedia article for its founding and place, OpenStreetMap's outlines of the yard and its dry docks for its extent,
  the USGS 1:24,000 sheets Long Beach 1949, Torrance 1951 and San Pedro 1951 for Los Angeles' harbour, and the
  Geographical Survey Institute's tiles of the US Army's 1945-50 air photographs for Tokyo Bay; the rest are placed
  from today's map (about ±150 m) and say so.
  - *Los Angeles and Long Beach:* Calship (1941), Bethlehem's San Pedro yard (1918), Fish Harbor's canneries (1918),
    Los Angeles Shipbuilding on Smith's Island (1917), Consolidated's Wilmington and Western Pipe and Steel's yards on
    the West Basin (1941), Reeves Field (1927), Roosevelt Base and the Terminal Island Naval Dry Docks (1942: both
    building that year; the dry docks were Long Beach Naval Shipyard from 1948), Craig Shipbuilding (1906).
  - *Tokyo Bay:* the Yokosuka Naval Arsenal (1871) with its east quays (1940, the year a guess) and outlying yards
    (Hemi, Nagaura, Funakoshi, Taura), Oppama's Naval Air Arsenal (1932), Uraga Dock (1897), Yokohama Dock (1896),
    Asano's Tsurumi yard (1917), Nippon Kōkan's Kawasaki works (1914), Ishikawajima (1876).
  - *Britain and Iceland:* Portsmouth's old yard and its Great Extension, the Royal Clarence Yard and HMS Dolphin at
    Gosport, Devonport, Chatham, Rosyth (1916), Lyness (1939), Reykjavík's harbour works (1917).
  - *Elsewhere:* Pearl Harbor's Navy Yard (1908) and Ford Island (1919), Kure, Sasebo, Maizuru, Mare Island, Puget
    Sound, Brooklyn, Norfolk, Malta, Alexandria, the Haliç arsenal and Gölcük (1926), Singapore's Naval Base at
    Sembawang (1938), Cavite, Hong Kong's Admiralty and Taikoo dockyards.
  - *Later land:* Los Angeles' Piers 300 and 400 and Long Beach's Piers G and J; Kawasaki's Ōgishima,
    Higashi-Ōgishima and Ukishima; Yokohama's Daikoku and Honmoku piers, Minato Mirai's, Negishi's and Kanazawa's fill;
    Tokyo's Ōi, Jōnanjima, Ariake, Aomi, Odaiba, Shin-Kiba and Wakasu; Chiba's Keiyō coast from Urayasu to Futtsu (its
    steel works, petrochemical estates and ports, 1950s-80s), in circles up to 3 km that reach generously: a circle only
    withholds the waterfront rule, and the buildings in it still thin by their score.
- **Old towns** (`towns::TOWNS`; branch `goldflag/period-old-cities`, 2026-10-10). Today's footprints cannot tell an
  old quarter from a post-war one, and the history's share is a whole 5' cell's, so within a cell the land densest
  today stood, whatever its age. Istanbul's walled city is no denser than its 1960s districts: in 1942 it kept 27% of
  its footprints (Fatih's 25 chunks 12,156 of 23,096; the far level's views 22,713 of 125,272) and read as fields with
  mosques standing in them. A listed town is a few polygons and circles with the year it stood to that extent from,
  each city's sources in its comment (censuses, period maps, city histories; OpenStreetMap's walls, roads and coasts):
  - *Inside an extent* the ground stands (`Thinning::stood`) and a footprint is kept by the region's floor alone
    (`core_floor`, `Thinning::with_towns`): all but a park's or a cemetery's thin ground, 94-98% of the footprints.
    Ground a source says stood open (a fire's cleared quarter, `Town::open`) is thinned as any land; none is listed.
  - *Out of the rest of the cell.* A 5' cell's distribution counts the footprints within extents apart
    (`Distribution::add_town`), and its bar keeps what the share leaves beyond what the extents keep
    (`Distribution::outside_share`). Where the extents keep more than the cell's share, nothing else in the cell
    stands but along its sides: that bar holds within the cell alone (`FiveBars`, `bar_about`), eased in over some
    900 m inside its sides (`TOWN_EASE`) from its plain bar (as if no town were listed), which its neighbours blend.
    So a neighbouring cell's town (Hafnarfjörður beside Reykjavík's) does not pay for it, and there is no step along
    the cells' sides. The distributions are kept by year as well, since the towns standing depend on it. A town's
    cells reach 100 m over the chunk's edges (`Kept`'s border), so a footprint over an edge whose middle lies beyond
    it stands too.
  - *Listed:* Istanbul (the walled city to the land walls; Galata, Pera, Kasımpaşa, Kurtuluş and Şişli to Nişantaşı,
    Beşiktaş and Ortaköy; Eyüp and Hasköy; Üsküdar; Kadıköy and Moda; the Bosphorus villages, Bakırköy, Yeşilköy and
    the Princes' Islands as circles about their piers); Reykjavík within Hringbraut, with Norðurmýri; Yokohama's
    Kannai, Kangai, Noge and the Bluff.
  - *Measured* (`report_old_cities`, ignored; `NAVAL_OLD_CITIES` a comma list of the cities): the share kept of the
    footprints in a few circles about each city's old core, and in the 5' cells about them outside the circles,
    before → after.

    | City | Its core's circles | The rest of its cells | Its 5' cells' total (the history's share) |
    | --- | --- | --- | --- |
    | Istanbul, 1942 | 27 → 98% | 13 → 2% | 19,582 → 38,702 of 116,556 (some 14,800) |
    | Reykjavík, 1941 | 32 → 83% | 8 → 0% | 1,021 → 1,800 of 6,719 (5%) |
    | Yokohama, 1942 | 35 → 87% | 40 → 39% | 45,713 → 47,851 of 115,140 (36%) |
    | Los Angeles, 1942 | 64% | 42% | not listed |
    | Long Beach and San Pedro, 1942 | 57% | 28% | not listed |
    | Yokosuka, 1942 | 53% | 39% | not listed |
    | Portsmouth, 1942 | 84% | 63% | not listed |
    | Tokyo, 1942 | 95% | 98% | not listed (its share near 1) |
    | Hafnarfjörður, 1941, beside Reykjavík | 12 → 12% | 16 → 16% | 1,084 → 1,084 of 6,620 (25%) |

    Where the share is near right the rest pays for the town (Yokohama). Where the history's share is far under the
    town's extent it does not: Istanbul's cells keep 2.6 times the share, Reykjavík's five times. The share is wrong
    there, not the extents: 794,000 people in Istanbul in 1940 would be 54 to a footprint at the history's share and
    about 20 at the extents', for a city of two- and three-storey houses; Reykjavík's 38,000 would be some 110 to
    one. Los Angeles' and Long Beach's cores are favoured already, Portsmouth and Tokyo stand nearly whole, and
    Yokosuka's town is favoured though thinner than it stood (its sources are thin); Honiara had no town in 1942.
  - *Stills* (`NAVAL_TOWNS=off` for the before; in `.build/period/old-cities/`, not committed). The far level's views
    of Istanbul (`tests/theatre_buildings_far.rs`): the near levels' buildings 40,131 → 53,817 looking down from 3 km,
    26,254 → 44,436 looking out, 33,359 → 51,055 and 14,229 → 23,716 from 4.1 km, the far level's masses made
    96,564-102,676 → 101,080-107,714. The peninsula, Galata and Beyoğlu, Üsküdar and Kadıköy stand as towns; the
    1960s districts beyond the walls, Bayrampaşa's and Mecidiyeköy's, are country. The buildings' GPU memory rises
    with them: with the far level 36 → 48, 23 → 37, 29 → 43 and 14 → 22 MB; the triangles drawn 1.4-1.9 times; the
    buildings' frame time (with them less without any, the median of seven turns, on a machine loaded by other
    builds) 1.0-2.1 → 1.2-3.1 ms. `theatre_period_thinning`: Fatih from
    3 km 120,458 → 135,535 buildings in range, from 4.1 km 119,739 → 134,482; Reykjavík from 1 km 10,046 → 10,909;
    Yokohama from 1.5 km 363,723 → 366,734; today's counts unchanged. `theatre_period_roads`: inside Hringbraut
    Reykjavík's streets are drawn again (0 → 3.9 km of streets, 0 → 4.5 km of service roads in the airfield's view),
    and Kannai's (6.9 → 45.7 km of streets from 300 m).
- **The land kept** (`thinning::Kept`, `ChunkMesh::kept`): a chunk's 40 × 40 cells of 25 m that stood whatever their
  score: a listed yard's, the working waterfront's, and 20 m about each footprint the railway, military or harbour lift
  kept (once it is planned and stands), with a border of 100 m beyond the chunk's edges for a footprint over them,
  which the neighbouring chunk's ground reads. The period's ground reads it (below), so a kept yard stands on a yard's
  ground and its service roads and floods are drawn (a page whose floods change is made again, its lamps in its
  hash). A place of worship's lift is left out: its plot unmarked would let today's photograph show through a field about it.
- **Both levels of detail alike.** A chunk drawn as boxes read OpenStreetMap at zoom 13, which drops most places of
  worship and small land use, so above about 3.5 km (every chunk boxes) whole quarters about the mosques, churches and
  stations went: Fatih's 25 chunks about the Süleymaniye in 1942 kept 11,852 buildings whole and 4,785 as boxes. In a
  past period a box reads zoom 14 too (`osm_zoom`), and both keep the same (12,156 with the Haliç arsenal listed). It
  costs about four times the boxes' tiles in a past period (some 70 more over a 10 km range at 35°), fetched once into
  the disk cache, the map's memory held to its 192 MB as before; `Period::Today` reads as it did.
- **Heights.** A measured building over its place's cap keeps the kind its height gives and stands at the cap
  (`Setting::cap`, read by `plan_building`), a tower a block: 46 m in the American West (Los Angeles' 150 ft), 45 m
  elsewhere in the US, 320 m in New York and Chicago, 31 m in Japan, 30 m in Britain, 25 m in continental Europe, the
  Mediterranean and the Nordic countries, 20 m in Hawaii, 12 m in the tropics and on the islands. `TALL` keeps the few that stood taller (Los Angeles City Hall 138 m, the Empire
  State, the Diet's tower, Westminster's towers, the Royal Liver Building, the Aloha Tower); `LATER` drops the few
  landmarks built after (Hallgrímskirkja, before 1986). Estimated buildings and the infill's lots stand at most six
  storeys (five by the Nile, four in East Asia, the dry countries, Iceland and Hawaii, three in the tropics); no rare
  estimated tower. Churches, chimneys, tanks and lighthouses keep their heights.
- **Modern things.** A port's container gantries are jib cranes, painted black, grey or red lead; no container stacks
  (their yards stand empty); a marina keeps 15% of its boats ([Harbour boats](infra.md#harbour-boats)); a shop's big box (a
  warehouse of 1,500 m² and more on commercial land) stands only where the score is a core's (0.45); an office has no curtain wall.
- **Places** (`tests/theatre_period_thinning.rs`, ignored, one process per place and period: a second headless session
  in one process fails wgpu's validation on the ship frame's depth binding; stills in `.build/period/T/`), buildings
  drawn today → in 1942 (1941 at Reykjavík), within the layer's range of each view: San Pedro from 600 m 61,148 →
  10,062 (16%: its old town by the harbour); Redondo Beach 159,036 → 44,728 (28%); the San Fernando Valley from 3 km
  157,608 → 44,743 (28%, patches about the old town centres and the boulevards), from 1.2 km 216,730 → 51,220 (24%);
  downtown Los Angeles 377,886 → 185,219 (49%, its towers 46 m blocks); Yokosuka 153,181 → 56,113 (37%), its town
  from 150 m 186,112 → 63,788 (34%, no tower over 31 m); Yokohama 247,858 → 75,061 (30%); Portsmouth 84,205 → 48,260
  (57%); Reykjavík 39,972 → 8,209 (21%, the old town by the harbour, no Hallgrímskirkja); Honiara 12,384 → 12. The
  buildings' GPU memory falls with them (downtown Los Angeles 283 → 140 MB, the Valley 133 → 34 MB, Yokosuka's town
  135 → 48 MB).
  - *With the working waterfronts, the listed yards and one map at both levels* (2026-10-10, stills in
    `.build/period/waterfront/theatre_period_thinning/`), the period's counts over the same views rise: San Pedro from
    600 m 10,062 → 13,355, from 1.5 km 5,878 → 8,379; Redondo Beach 44,728 → 51,157 and 30,767 → 35,083 (its boxes'
    lifts); Yokosuka from 600 m 56,113 → 65,240 (its arsenal whole), from 1.5 km 46,118 → 52,208, its town from 150 m
    63,788 → 75,096; Yokohama 75,061 → 96,312; Portsmouth 48,260 → 51,865 and 34,260 → 38,594. New views, today → in
    1942: Terminal Island from 600 m 88,408 → 22,057, Long Beach from 1.5 km 98,202 → 39,745, Yokosuka's arsenal from
    400 m 180,607 → 74,116, Uraga from 600 m 109,026 → 49,419, Fatih from 3 km 151,595 → 50,554 and from 4.1 km
    137,418 → 48,169 (the same quarters standing).
  - *About the yards* (`the_period_keeps_the_same_at_both_levels`, the chunks within 2-3 km whole and as boxes, before
    → after; today's in brackets):

    | Place | Chunks | Whole | As boxes | By 5' cell, whole |
    | --- | --- | --- | --- | --- |
    | Terminal Island and Wilmington | 7 × 7 | 2,958 → 4,148 (12,890) | 2,267 → 4,148 | 30 → 37%, 8 → 21%, 20 → 36% |
    | Long Beach's inner harbour | 5 × 5 | 2,257 → 2,316 (4,616) | 1,977 → 2,316 | 50 → 52% |
    | Yokosuka's arsenal | 5 × 5 | 13,109 → 14,173 (27,262) | 11,688 → 14,173 | 29 → 33%, 67 → 71% |
    | Uraga | 3 × 3 | 4,121 → 4,527 (14,639) | 3,269 → 4,527 | 28 → 31% |
    | Portsmouth's dockyard | 5 × 5 | 9,165 → 9,325 (10,995) | 7,729 → 9,325 | 77 → 79%, 96% |
    | Fatih | 5 × 5 | 11,852 → 12,156 (23,096) | 4,785 → 12,156 | 51 → 53% |

    The land kept whole is 57% and 48% of Terminal Island's two 5' cells' area (its yards' circles take in their
    water), 25% and 19% of Yokosuka's, 15% of Portsmouth's.
- **Cost.** Gathering runs on the buildings' worker before a batch is meshed: nothing once a place's chunks and 5'
  cells are kept, 0.1-0.5 s a batch over a new town, up to 2 s where a 5' cell's distribution first reads a big city's
  level-13 tiles (Los Angeles). A distribution is kept once every tile under it is had; until a big file under it is
  split the chunk is made as waiting and made again after. A file it cannot have is asked for once a batch (offline,
  the footprints' fetching waits, as the meshing's does). It may fetch a neighbouring quadkey's file, or ask for a big
  file to be split, before the drawn chunks need it. Kept: at most 8,192 chunks' cell counts (0.8 kB each, 7 MB)
  and 256 distributions (4 kB, 1 MB); the footprints' own caches are unchanged.
- **Checked** by `--lib buildings::thinning` (Today pinned; a synthetic city keeps the history's share within 0.06 at
  0.15-0.7, its core whole and its thin edge gone; a cell scores the same from any window, across a chunk's edge too;
  a chunk without Microsoft's footprints scores by its own; caps, `TALL`, `LATER`, shop boxes, offices' detail,
  estimated storeys, no rare tower; gantries never in the period; no stacks and few boats; the lifts; the share's
  rules and the blend; `a_capped_building_keeps_the_kind_its_height_gives`; a synthetic port's shipyard stands whole
  and its container terminal, apron, houses and later land do not, `a_listed_yard_stands_from_its_year_and_not_before`,
  `both_levels_of_detail_keep_the_same`; the old towns: `a_listed_towns_kept_is_taken_from_the_rest_of_its_cell`,
  `a_cells_towns_are_paid_for_within_it_alone`,
  `within_a_listed_town_the_ground_stands_and_buildings_keep_by_the_floor`, and `--lib buildings::towns`, the list's
  shapes whole and Istanbul's, Reykjavík's and Yokohama's landmarks in and their post-war districts out) and
  `--lib theatre::buildings`
  (`a_past_period_thins_a_chunk_by_its_prepared_thinning`,
  `chunks_without_microsofts_footprints_agree_along_their_edges`).
  `report_the_period_about_a_place` (ignored; `NAVAL_PERIOD_AT=lat,lon[,year[,reach]]`) prints the share kept per
  chunk and each 5' cell's footprints, shares, bar and score deciles, for calibrating;
  `the_period_keeps_the_same_at_both_levels` (ignored, the same variable, Fatih by default) makes the chunks about a
  place today and in the period at both levels and asserts the two keep the same buildings and land.
- **Open.** Coverage hardly tells Los Angeles' 1920s bungalow grid from a 1950s tract, so its kept land is the
  densest patches and the lifted ones, not the true pre-war city; an old industrial district of whole-block roofs
  (downtown Los Angeles' east side) reads thin and goes unless it is railway land or a working waterfront. Los Angeles
  City Hall keeps its source's height, which is not always its 138 m. Only three old towns are listed; Los Angeles'
  1939 extent (the HOLC's survey maps it), Yokosuka's town and Kadıköy's railway suburbs (Erenköy to Bostancı, whose
  cell the history leaves empty in 1942) are not. Istanbul's fires of 1908-18 left burned
  ground in the walled city into the 1940s that no source found places, so its extent stands whole; the extents'
  landward edges are placed to about ±150-200 m. The working waterfront and the listed yards stand
  today's footprints: a yard since cleared or paved for containers (Uraga Dock's site, Calship's and Terminal Island's
  terminals) stands with what is left there, not its sheds of the time, on today's photograph; an old quay now a
  container yard (San Pedro's west bank) is thinned as a terminal is; most of the lists' circles outside Los Angeles
  and Tokyo Bay are placed from today's map; a petrochemical or logistics estate on the water in a port the lists do
  not reach stands whole; the infra layer's jib cranes still stand on land made later (Pier 400). Where Microsoft's footprints end (a country the dataset lacks
  beside one it has), the cover's score meets theirs with a step. Power lines and their towers and today's piers stand
  in the period (runways and bridges follow "The roads", below). Marina boats: [Harbour boats](infra.md#harbour-boats).

## The period's ground

With the buildings thinned, the ground under the land they leave empty would still be today's: the satellite
photograph's roofs and pools, its freeways and the map's every residential street. In `Period::Year` (Graphics →
Buildings, Period) that ground is drawn as the period's country, and the map's roads are the period's. `Period::Today`
draws exactly as before.

- **Which ground** (`naval-client` `theatre::period_land`): `stood(geo, period, p)` is false where today's built-up
  land was country in the period. `version()` rises whenever an answer changes. Answers are held for the scenario on
  screen and the one before it, each with its own year. A late batch from a scenario left behind therefore changes only
  that scenario's answers, and a scenario returned to finds its answers still there. `Period::Today` holds no answers,
  and `stood` is always true for it.
  - *The buildings' answer.* Where the buildings' thinning has run, the ground answers as the buildings do. The
    buildings' worker hands in each chunk it gathered in full once it is meshed (`buildings_period::answer`), with the
    land its buildings kept whole (`ChunkMesh::kept`). A chunk thinned by its own drawn footprints, or still lacking a
    5' cell's summary, is left out. A point stands where the score between the cells' centres reaches its cell's bar,
    or in a listed town's extent or a listed yard, or on the kept land (`Thinning::stood`): a working waterfront's
    yards, wharves and the ground between their sheds, and 20 m about what the railway, military and harbour lifts
    kept. A church the worship's lift
    keeps still stands on marked ground. Both levels of detail read one map in the period, so a chunk answers alike
    whole or as boxes.
  - *The cover's estimate.* Beyond the buildings' reach, the answer is estimated from the window's land cover alone, by
    the buildings' own functions (`thinning::smoothed_scores`, `cell_bar`, `bar_about`):
    - Each 100 m cell of a chunk holding WorldCover built-up land (class 50) is sampled at its window texels (four by
      four at a sea window's 23 m). Its score is `COVERAGE` (0.3) times its share built up, smoothed over blocks of
      8 × 8 chunks with a chunk's halo, plus the buildings' noise. Neighbouring chunks' rings agree to the bit.
    - Each 5' cell's distribution counts a cell wholly built up as 12 footprints. Its bar keeps the period's share of
      that land, densest first, with the region's floor, and each cell's bar is blended between the four 5' cells about
      it.
    - A listed town's extent (`towns::TOWNS`) stands and the rest of its 5' cell pays for it, its distribution
      counting the extent apart, as the buildings have it ("Old towns", above;
      `the_estimate_stands_a_listed_town_and_thins_the_rest_for_it`).
    - A listed yard (`thinning::YARDS`) stands whole from its year, as the buildings have it. The working waterfront
      cannot be told from the cover alone (it needs the map's land use and the water), so beyond the buildings' reach
      it is thinned by its cover.
    - A newer window's estimate replaces an older one's over the same chunks, and a finer one is read before a coarser
      one. So a sea scene's near tiles read the sea window's estimate, not an air window's.
  - *Calibration.* `tests/theatre_period_ground.rs` (network, ignored) compares the two answers over the 100 m cells at
    least a quarter built up, in the buildings' chunks within 8 km of each town, with Graphics → Buildings, Period, on
    2026-10-10. Results at `COVERAGE` 0.3 (kept share is the share of cells that stood):

    | Town | Kept: buildings | Kept: estimate | Cells agreeing | IoU of stood cells |
    | --- | --- | --- | --- | --- |
    | San Fernando Valley, 1942 | 21% | 25% | 78% | 0.35 |
    | San Pedro, 1942 | 8% | 18% | 86% | 0.30 |
    | Yokosuka, 1942 | 24% | 32% | 82% | 0.51 |
    | Portsmouth, 1942 | 46% | 52% | 85% | 0.73 |
    | Reykjavík, 1941 | 18% | 18% | 83% | 0.35 |
    | Honiara, 1942 | none | 2% | 98% | — |

    - Sweeping `COVERAGE`: from 0.2 to 0.5 the mean IoU over the five towns stays between 0.44 and 0.45, with 0.25 and
      0.3 best. At 0.15 the American region's floor (0.15) cuts the Valley's kept share to 16%.
    - Where the town keeps a distinct old core on a coast (Portsmouth, Yokosuka) the two answers draw the same towns.
      Where today's built-up land is uniform (the Valley's suburbs, Reykjavík's), the cover cannot tell a 1920s street
      from a 1950s one. The estimate then keeps about the right share in blobs of the right size, placed by the noise.
    - The estimate counts land where the buildings count footprints. San Pedro's container yards and the Valley's
      industrial land weigh more in it, so it keeps up to twice the buildings' share there.
- **The mark** (`naval_terrain::surface_maps::cover::PAST_COUNTRY`, 0x80): set over a cover texel's class where the
  ground did not stand; `cover::today` takes it off.
  - *What is marked.* A built-up texel is marked where its ground did not stand. Any other land (trees, grass, crops,
    bare ground, wetland) is marked likewise inside a cell whose land is half or more built up today (`MOSTLY_BUILT`):
    the gardens, lawns, street trees and parks of the town that went. Water, snow and unknown texels are never marked.
    A chunk with no answer has its marks taken off.
  - *The window's maps.* The scene's build (off the frame loop's thread) estimates the window, holds the estimate and
    hands the land a marked copy of the cover (`set_window_maps`, `TerrainLayer::set_surface_maps_marked`). The land's
    statistics leave marked texels out. A sea window's 4096² was estimated in 10–38 ms and marked in 3–11 ms in the
    `test-fast` profile (Los Angeles' 4096 built-up chunks the slowest). Air windows (2048² at 117 m, one sample a cell)
    take the same path.
  - *Near tiles.* Each tile is marked as it is made, NAIP's 2 m tiles included. The period is part of the tile cache's
    key. Of the near texels holding the same class as the window texel under them, 98.7–99.6% carry the same mark.
  - *Refinements.* The land keeps a number for its marks (`TerrainLayer::set_cover_marks`, `NearTile::marks`: the
    period's year and the answers' version). When answers change, two paths re-mark without rebuilding anything:
    - The driver's `period_land::Feed` re-marks the window's chunks the buildings answered for. It does at most 32
      chunks a frame within 0.5 ms, each a `CoverPatch` of about 43 × 43 texels on layer 0.
    - The near tiles' `Feed` asks their worker to re-mark the drawn tiles whose chunks changed (ahead of new tiles).
      The frame loop then rewrites each re-marked tile's cover on layer 1 (`TerrainLayer::queue_cover_patch`).
    - The layer writes queued patches in `prepare`, after the frame's near tiles. A patch for a tile its slot no longer
      holds is dropped. The trees are placed again, and the forest's shade is baked again over the rectangle.
  - *Drawing and reading.* The land draws a marked texel as the period's country rather than its photograph. The
    buildings read today's class (their own thinning decides what stands). A street light stands only on unmarked
    built-up land. A window's class means leave marked texels out. The horizon ring is not marked.
  - *Stills.* `.build/period/ground-mask/` holds each town's window, near tiles and buildings from above.

### The country

What a marked texel draws as: the region's farmland of the 1940s, worked out in the shaders from
the window's own rural colours (`naval_terrain::country`, `surface.wgsl`'s country section, `tree_place.wgsl`'s
`country_slot`). Nothing is stored or streamed for it: no texture, no buffer, 16 bytes of the land's uniform
(`TerrainUniforms::country`), so 0 MB.

- **Which country** (`CountryLook`, `TerrainLayer::set_country`): the host sets it with the scene's land
  (`theatre/mod.rs`'s `country_look`), in a past period only: the buildings' region at the window's place
  (`Region::at`) and the scenario's start for the season. `Period::Today` has none, and with none
  every marked texel draws as its class today. The eight countries:

  | Country | Regions | Fields | Kinds | Edges |
  | --- | --- | --- | --- | --- |
  | West | California, the South-West | 400 × 200 m, 80% of districts on the section lines | citrus and walnut groves 36%, grain 18%, pasture 16%, truck farms 14%, ploughed 9%, alfalfa 7% | dirt tracks 4 m, eucalyptus windbreaks |
  | East | the rest of North America | 320 × 180 m | pasture 28%, grain 24%, hay 14%, woodlots 14%, ploughed 10%, truck, orchards | fencerows 2.5 m with trees |
  | Britain | Britain, Ireland | irregular, 150 m | pasture 50%, grain 16%, ploughed 12%, hay 12%, woods 6%, orchards | hedgerows 4.5 m with oaks and ashes |
  | Europe | the North Sea's coasts, temperate Europe | strips 320 × 70 m | grain 30%, pasture 28%, ploughed 16%, hay 10%, truck 8%, woods, orchards | grass balks 1.5 m, few trees |
  | Japan | East Asia | 110 × 60 m | paddies 62%, dry fields' strips 18%, groves 9%, gardens 6%, mulberry 5% | bunds 1.6 m |
  | Tropics | the tropics, Hawaii, the Pacific's islands | 380 × 260 m | kunai 34%, coconut plantations 30%, gardens 16%, bush 14%, pasture | none |
  | Nordic | Iceland, the Nordic countries | irregular, 120 m | home fields (the tún) 48%, heath and moss 34%, pasture 12%, potatoes | drainage ditches 1.4 m |
  | Mediterranean | the Mediterranean, North Africa, the Nile, the arid lands | 190 × 110 m | olives 30%, grain 28%, ploughed 14%, garrigue 12%, vines 10%, orchards | dry-stone walls 1.2 m |

- **The fields** (`country_field`): districts of about 2.6 km (cells about scattered points fixed to the chart), each
  with its own heading, its own run of the kinds (a citrus belt, a flat of truck farms) and its own scale (0.75 to
  1.25 of the region's field). In a district, rows of fields shifted along, a field whole or split into two or three;
  their lines bent a few metres over hundreds by a smooth noise. Britain's and Iceland's fields are irregular cells on a
  gently warped plane. Where the land steepens from 1 in 10 to 1 in 5, more and more of the fields are rough grazing (the West,
  the Mediterranean, the north), pasture (Britain, Europe, the East) or woods (Japan, the tropics). Within 90 to 260 m
  of the shore they are yards (packed earth, cinders and gravel, sandier in the dry countries), so a quay's marked
  ground beside the land the buildings kept reads as yard rather than field; the tropics' palms run down to the beach.
  The kept land itself is not marked and draws today's photograph: a dockyard's sheds, slips and docks, its rail
  sidings and paving.
- **Its ground** (`country_ground`): each kind drawn near the camera with its texture (furrows, drill rows, mown
  swaths, crowns on an orchard's lattice, paddies' plots of 32 × 16 m between bunds, a garden's beds), each field a
  little lighter or darker than its neighbours and blotched within; far off, its mean. The colours come from the
  window's mean colours for its own grass, cropland and bare ground (`cover_means`, held within ranges), so the country
  takes the photograph's light and white balance, with a soil for each country (the valleys' adobe, Japan's dark
  volcanic loam, the Mediterranean's red earth, Iceland's peat).
- **The season**: the months since mid-January of the scenario's start, half a year on south of the equator, each
  field a little ahead or behind. The dry countries' grass is tawny in summer while their alfalfa is watered green;
  grain greens, ripens and is cut at the region's harvest (early June in California, the Mediterranean and Japan, July
  in the East, August in Britain, late August in Iceland); the paddies flood in May, the rice closes over the water in
  June and is gold in September; the climate's winter dulls the northern countries' grass and bares their woods.
- **The edge**: whole fields decide, so the town's edge runs along the fields' edges rather than the cover's texels. A
  field is the country where the marked share about its middle (`country_past`, bilinear over the four texels there)
  passes a level of the field's own (0.2 to 0.8). An unmarked green texel (trees, shrubs, grass, crops, moss) counts
  for 0.15 against a marked one, so a park left among the marks goes with the farms; water, snow, wetland and
  mangroves keep their class. A point looks for a mark among its four texels, then on a ring of eight out to 1.5 of the
  region's field length; with none the cover is the same arithmetic as before. Whole fields reach up to that far past
  the marks, over unmarked town at the edge of a kept one. The ring and a field's middle are read on the window's maps
  (`country_layer`; the near tiles' only where there are none), which are whole about any point they cover, so the
  window's and the near tiles' lookups of a point decide alike.
- **The trees** (`country_slot`): the canopy map's trees (today's gardens and street trees) are not stood on the
  country. Each kind plants its own: orchards and plantations in rows on a lattice along the field's heading (citrus 7
  m and walnuts 11 m apart in the West, apples 8 m, olives 9 m, coconut palms 9 m), every tree out to 700 m and fewer
  beyond; windbreaks of eucalyptus in the West and cypresses in the Mediterranean, hedgerow oaks and ashes in Britain
  and the East's fencerows, along a share of the fields' edges; a farmstead's trees within 45 m of a share of the
  fields' middles; now and then a lone tree. Woodlots and groves take the region's woods at full density; yards stand
  none. A cover rewrite of the marks bakes the forest's shade again out to the country's reach (`Country::reach_m`,
  1.35 km in the West).
- **At night**: a marked texel is not built-up land to the street lights (`lamps.wgsl`) or the towns' glow (the land's
  `built` share), so the farms are dark between the lit towns.
- **Today**: with no country set, or no mark in reach, a pixel's cover is computed exactly as before.
  `gpu.rs`'s `the_period_country_draws_on_marked_land_alone` draws a town half marked (94 m texels, a Channel coast):
  with the country off, and on with nothing marked, the image is the town's to the bit; on, it changes 98% of the
  marked side's pixels and none beyond the marks' reach.

- **Its cost**: GPU time at 1280 × 720 on this Mac (Apple silicon, `test-fast`, High), each place's first view drawn
  with the country and without it in turn (the same marks drawn as today's classes), 30 frames a side five times; the
  frame's 10th percentile and the opaque pass's median. All of it is the land's opaque pass; the trees' placement,
  shadows and night lights did not change measurably.

  | View | Frame without (ms) | Frame with (ms) | Difference | Opaque pass (ms) |
  | --- | --- | --- | --- | --- |
  | San Fernando Valley, 3 km | 4.10 | 5.26 | +1.16 | 2.60 → 3.85 |
  | San Fernando Valley, 1.2 km, at night | 4.30 | 5.23 | +0.93 | 2.75 → 3.78 |
  | Redondo Beach, 300 m | 4.51 | 4.98 | +0.47 | 1.58 → 2.07 |
  | Yokosuka, 150 m | 3.50 | 3.67 | +0.17 | 1.22 → 1.38 |
  | Southsea, 150 m | 3.24 | 3.53 | +0.28 | 1.47 → 1.73 |
  | Reykjavík, 150 m | 3.98 | 4.39 | +0.41 | 1.74 → 2.29 |
  | Honiara, 300 m | 3.93 | 4.24 | +0.31 | 0.87 → 1.11 |

  The cost follows the screen the country covers: a 3 km view over a valley of farms pays about 1.2 ms, a harbour or a
  town or a beach from a few hundred metres 0.2 to 0.5 ms. Cut out in turn in the Valley's view, the fields' layout
  (`country_field`, worked out once a pixel) is about 0.5 ms of it, the ground's kinds (`country_ground`) 0.3 ms and the
  ring about a point 0.25 ms. The first version paid 1.8 ms there (with the test hook's marks, whose noise is about
  0.25 ms of it), before the districts took one pass of nine cheap hashes, the field was worked out once a pixel and
  the ring was eight texels shared by the window's lookup and the near tiles'. Deciding on the window's maps
  (`country_layer`, so a point's two lookups agree) put 0.15 ms back. Recording a direct mark in the ring's answer, to
  spare the near tiles' lookup its ring, cost 0.5 ms with the country on and 0.2 ms with it off (the shader's
  registers, not its work), and is not done.

Open: a church the worship's lift keeps stands in a field with no yard of its own, and a country tree can stand
through its roof (the land does not know that lift); whole fields decide, so a field whose middle is marked reaches up
to a field's length into a kept yard's inland edge; today's bare and industrial texels the host
leaves unmarked still show the photograph among the fields; from 900 m Japan's flooded paddies read alike, one sheet
of the sky's reflection. The streets over the country go with the period's roads (below); the arterials, tracks and
railways stand on it as they were laid.

### The roads

In `Period::Year` the features' pages, the
street lights, the airfields and the bridges draw the period's roads. `Period::Today` draws exactly as before:
`todays_pages_are_as_they_were` pins a fixture page's bytes and lamps at both levels, with a ground that answers false
too. The road rules are in `naval-client` `theatre::features::period`; the lists of aerodromes and bridges are in
`naval_terrain::infra::period`, which the host's `theatre::infra` applies to each chunk's airfield and bridges.

- **Motorways** stand only along a listed section (`MOTORWAYS`, by place and the year each section opened; a section
  that opened late in a year counts from the next).
  - *The test.* A motorway's way is kept within 120 m of a listed route and within 30° of its heading; a ramp within
    90 m whatever its heading. A freeway crossing a listed one, or one built beside it later, goes.
  - *The list.* In the theatres' regions only Los Angeles had any before 1948. The Arroyo Seco Parkway: Glenarm Street
    to Fair Oaks Avenue on 10 December 1938, on to the Figueroa Street viaduct on 30 December 1940 (South Pasadena's
    part on 30 January 1941), through the Figueroa Street tunnels in December 1943, to the Four Level in 1953. The
    Cahuenga Pass Freeway: Highland Avenue to Barham Boulevard in 1940-41, on to Vineland Avenue in 1949. The Ramona
    Parkway (I-10): Ramona Boulevard from 1935, a freeway in the city from 1944, to Alhambra by 1953. The routes are
    OpenStreetMap's (by Overpass), thinned to 300 m.
  - *Roads before freeways.* A section that was an ordinary road before it was a freeway (the Figueroa Street tunnels'
    line, a city street from 1937; Ramona Boulevard) is drawn as a trunk road until then, without its ramps.
  - *Elsewhere* the first came later: Los Angeles' Santa Ana and Terminal Island Freeways in 1948, Hawaii's Mauka
    Arterial (the H-1) in 1953, Japan's KK Line in 1959, the Shuto Expressway in 1962 and the Meishin in 1963,
    Britain's Preston bypass in 1958 and the Chiswick flyover in 1959. Iceland and the Solomons have none. Busways are
    later than every period.
  - *Sources.* Wikipedia's article on each road; cahighways.org for routes 10, 101 and 110; AARoads; PBS SoCal on the
    Arroyo Seco; Toyo Keizai on Japan's first expressways.
- **Streets.** OpenMapTiles folds residential and unclassified roads, living streets and plain roads into `minor`.
  These, `service` (driveways, alleys, parking aisles) and `raceway` are drawn only where the ground stood
  (`period_land::stood`).
  - A way is sampled every 20 m and cut where the answer changes, the cut found to 1.5 m.
  - Gaps under 40 m are closed. What is left at a cut shorter than 60 m is dropped, and shorter than 150 m with a cut
    at both ends, so a street ends where its town ended rather than as a stub into the country.
  - A line's end at its tile's edge counts as a cut, so the two tiles a street crosses judge it alike.
  - The arterials (trunk to tertiary), tracks, railways and waterways stand as they are: the county roads and the
    arterial grid were laid before the suburbs.
- **Paving**, where the evidence is clear, through 1960. An unpaved road takes no street lights.
  - *Japan:* the minor roads, service roads and tracks are earth. Tokyo's streets were 52.7% paved in 1937, its
    arterials first; Watkins' survey (1956) found 23% of the first-class national roads paved, and in 1970 some 15% of
    all general roads were (MLIT's road statistics).
  - *Iceland:* every road is gravel beyond 1.5 km of Reykjavík's old town (a road is cut where it crosses that
    circle). No road outside Reykjavík was paved before the Reykjanesbraut (1965, the Althingi); Reykjavík laid
    asphalt from 1927 (Vegagerðin).
  - *The Solomons:* none paved. The roads were coral, clay and gravel (Building the Navy's Bases in World War II,
    chapter 25).
  - *Elsewhere* as today. Britain's roads were sealed (RAC Foundation). The United States' coasts and Hawaii are too
    mixed to place: FHWA's HM-212 counts most rural mileage of 1941 unsurfaced and most cities' streets paved.
- **Street lights** follow the pages: none on a street or motorway the period leaves out, none on an unpaved road; a
  motorway drawn as a trunk road is lit as one.
- **The made pages** are kept by period (the cache's key), each with the `period_land::version` it was begun at, its
  strokes' hash and when it was made.
  - A past period's page is made again once the ground under it changed after it was begun
    (`period_land::changed_since` over the buildings' chunks its page, its strokes' reach and 150 m beyond reach), at
    most once a second: the buildings' answers arrive up to 32 chunks a frame. A page whose ground changed elsewhere
    only takes the new version.
  - A page made again the same keeps the one already uploaded. A stale page stays drawn until its remake arrives.
  - The driver's feed takes its period from its land's cover marks (`period_land::marks_period`) and hands a held
    page again when its strokes' hash differs from the one it last handed; a page it never handed (installed with the
    scene) is handed once more, since what the land holds is not known.
- **Aerodromes** (`FIELDS`, `field_then`). A listed aerodrome is not drawn before the year it was paved. Until it grew,
  its runways are cut to its grounds then and to its runway's length.
  - *With their wartime size:* Mines Field, Los Angeles' airport from 1929: the square mile north of today's south
    runways, one runway of about 4,600 ft in 1942 (1,400 m here), grown past it in 1946. Henderson Field: August 1942,
    3,800 × 150 ft by September (1,160 m until 1943). Munda: 1943 (1,000 m until 1944).
  - *Not before their years:*
    - Los Angeles: Los Alamitos 1942, Lomita 1943, El Toro 1943, the Oxnard flight strip 1942, Point Mugu 1946,
      Whiteman 1946, Catalina 1946.
    - Oahu: Honolulu's Reef Runway 1977, Barbers Point 1942, Kaneohe 1942.
    - Kanto: Haneda's runways, all on land reclaimed from 1984, 1988 (the field of 1931-45, two runways of 800 × 80 m,
      lay at its western tip); Narita 1978; Shimofusa 1945.
    - Southern England: Heathrow 1946, Gatwick 1958, Eastleigh 1963, Thorney Island 1942.
    - Iceland: Reykjavík 1941, Keflavík (Meeks Field) 1943. The Solomons: Segi and Barakoma 1943.
    - No unlisted aerodrome stood in Iceland or the Solomons before 1946.
  - *Every runway* is no longer and no wider than its period's (`runway_cap`, shortened alike from both ends): 1,500 ×
    46 m to 1938; 2,150 × 46 m in 1939-45 (the RAF's Class A runway of 1942 was 2,000 × 50 yd); 2,750 × 60 m in
    1946-52; 3,400 × 60 m to 1960. The taxiways and aprons of a field that stood are kept on its grounds; a runway's
    mapped area (today's length and width) is not drawn, its capped line is.
  - *Paint* (`runway_paint`). Before 1948 a runway is plain pavement: no number, centre line or threshold bars (the
    infra shader's flag 4). The US marking standard (TSO-N10) came in 1948 and ICAO's Annex 14 in 1951; from 1948 a
    runway has its numbers and centre line but no touchdown zone or aiming point.
  - Runway lights follow the runways drawn (`InfraLayer::runways`).
- **Bridges** (`LATER_BRIDGES`, `LONG_SPANS`, `bridge_stood`).
  - *Built after the period:* not drawn, nor their roads on the land. The Vincent Thomas Bridge (1963), the Gerald
    Desmond and Long Beach International Gateway (1968), the Admiral Clarey Bridge to Ford Island (1998), the Yokohama
    Bay Bridge (1989), the Tsurumi Tsubasa Bridge (1994), the Rainbow Bridge (1993), the Tokyo Gate Bridge (2012), the
    Aqua-Line's bridge (1997), the Itchen Bridge (1977) and Borgarfjarðarbrú (1981). Positions from OpenStreetMap.
  - *Long spans:* an unlisted bridge spanning 300 m or more of sea stands only where listed as standing: the Forth
    Bridge 1890, Hercílio Luz 1926, Carquinez 1927, Mount Hope 1929, Bayonne 1931, Sydney Harbour 1932, San
    Francisco-Oakland Bay 1936, Golden Gate 1937, Lions Gate 1938, Bronx-Whitestone 1939, Tacoma Narrows 1950.
  - *With their roads:* a bridge's ways are the pages' runs, so a motorway's deck goes with its motorway and a street's
    with its street. A chunk whose bridges read the ground is made again as a page is, when the ground under them
    changed.
- **Checked** by `--lib theatre::features` (Today pinned; motorways out and arterials kept; a street ends within 2 m
  of where its ground stopped standing; runs close gaps and drop stubs; the Arroyo Seco from 1941 and a freeway across
  it never; the Ramona Parkway a trunk road in 1940; the paving; a bridge where the Yokohama Bay Bridge stands left
  out with its road in 1942; the street lights; the cache by period and a page made again only when its ground
  changed), `--lib theatre::infra` (Los Angeles' runway on Mines Field in 1942 and none in 1928; Keflavík none in 1941;
  the bridges) and `-p naval-terrain --lib infra` (the lists, the runway paint).
- **Places** (`tests/theatre_period_roads.rs`, ignored, one process per place and period; stills in
  `.build/period/roads/`). `counts.json` holds the roads drawn within 1 km of each view's target (km by look) and the
  street lights on its lamp pages, today → in 1942:

    | View | Motorway, km | Streets, km | Service, km | Street lights |
    | --- | --- | --- | --- | --- |
    | San Fernando Valley from 1.2 km | 0 | 30.2 → 15.9 | 36.2 → 14.7 | 5,112 → 2,554 |
    | San Fernando Valley from 300 m | 0 | 31.5 → 16.9 | 37.7 → 20.8 | 5,478 → 2,669 |
    | Sherman Oaks' interchange from 800 m | 19.1 → 0 | 31.3 → 9.5 | 20.6 → 6.5 | 3,349 → 1,370 |
    | The Four Level from 800 m | 20.9 → 0 (3.3 km trunk road) | 19.1 → 3.0 | 52.3 → 2.7 | 4,986 → 2,524 |
    | The Arroyo Seco from 600 m | 6.2 → 6.1 | 31.4 → 6.0 | 20.0 → 7.6 | 3,664 → 1,409 |
    | Los Angeles' airport from 800 m | 0 | 1.3 → 0 | 42.2 → 0 | 3,408 → 1,383 |
    | San Pedro's harbour from 1.5 km | 7.8 → 0 | 5.2 → 1.7 | 34.2 → 23.1 | 1,877 → 874 |
    | Yokohama Bay Bridge from 800 m | 11.6 → 0 | 0.3 → 0 | 5.4 → 0 | 1,304 → 554 |
    | Yokohama's streets from 300 m | 8.6 → 0 | 46.3 → 6.8 (unpaved) | 12.2 → 1.2 (unpaved) | 5,577 → 1,853 |
    | Reykjavík's airfield from 800 m (1941) | 0 | 17.5 → 0 | 25.5 → 0 | 2,000 → 444 |

    - The Vincent Thomas and Yokohama Bay Bridges' decks go with their motorways. Los Angeles' airport keeps two
      runways about 1,400 m long, plain, on Mines Field's square mile. Reykjavík's airfield stood in 1941.
    - The streets follow the ground's answer: Yokohama keeps 15% of its streets. San Pedro's port kept none until its
      working waterfront and listed yards stood (2026-10-10, `.build/period/waterfront/theatre_period_roads/`); now
      its yards' service roads are two thirds of today's, and its floods follow the ground too (1,720 → 896 within the
      view's pages, where all 1,720 burned before).
    - Remaking: in Los Angeles' run, the first view's settling made 194 pages again (56 of them came out the same), and
      475 by the sixth view. That is about one remake a page as the buildings' answers arrive, and none once they have.
- **Open.**
  - The photograph under a dropped freeway or runway still shows today's pavement where its ground is not marked.
  - The motorway list is whole only to 1947. A later period draws only the sections listed and none of Los Angeles'
    other freeways (the Hollywood, Harbor and Santa Ana Freeways' 1948-60 sections). The optional rule for Germany's
    Reichsautobahn (3,860 km by December 1941) and Italy's autostrade (from 1924) is not built.
  - Los Angeles' airport keeps both of today's south runways where they cross Mines Field's grounds.
  - The pages judge a bridge's sea span per way, the infra layer per joined bridge, so a long bridge mapped as many
    short ways can lose its deck and keep its approaches' roads.
  - Japan's country arterials are drawn paved though most were gravel, and its towns' streets unlit.

History (stills and the lanes): [docs/archive/terrain-history.md](../../../docs/archive/terrain-history.md).
