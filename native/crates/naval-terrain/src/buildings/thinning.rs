//! Which of today's buildings stood in a past year (native/docs/terrain/buildings.md, "Period"): the period thins
//! the footprints by whole neighbourhoods, keeping a town's old, dense core and leaving the thinner land at its edges
//! empty, so that about each 5-arc-minute cell of the built-up history's grid (some 9 km,
//! [`super::history::stood_share`]) about the share of today's footprints that stood then is kept.
//!
//! - **Score** ([`window_scores`]): each 100 m cell of the theatre (ten to a chunk's side, [`CELL_M`]) by the ground its
//!   footprints cover, each footprint's area counted to [`COUNTED_M2`] (an estate of large sheds, shops or blocks reads
//!   as thin ground, not as a core; a measured block of four storeys and more to [`TALL_COUNTED_M2`]), smoothed over
//!   some 400 m ([`REACH`] cells about, a Gaussian of [`SIGMA`] cells), plus a low noise in the theatre's metres
//!   ([`noise`]) so the kept land's edges wander as a town's do. The host counts Microsoft's footprints of the chunk and
//!   of its neighbours from its stores, the same whichever chunk, window or level of detail asks, so a cell scores the
//!   same from either side of a chunk's edge.
//! - **Bar** ([`bar`]): about each 5' cell, the score that keeps its share ([`period_share`]: the history's sharpened,
//!   since it counts post-war suburbs as built two or three times too early; three quarters of its neighbours' where it
//!   left a town's cell empty; by region where it has no data) of the cell's footprints ([`Distribution::bar`], over
//!   all the cell's 100 m cells), or where the cell holds too few to say ([`LEAST_SAMPLE`]) a bar by the share alone
//!   ([`absolute_bar`]); never under [`core_floor`]. Each 100 m cell blends the bars of the four 5' cells about it
//!   ([`hyde_about`], [`blend`]), so the kept land has no edge along the 5' cells' sides.
//! - **Kept** ([`Thinning::keeps`]): a footprint (or an infill lot) whose score at its middle (between the cells'
//!   centres, so the kept land's edge is a smooth line, not a staircase of cells), lifted by an old town's marks
//!   ([`lift`]: a place of worship and the streets about it, railway land, a harbour's front), reaches its cell's bar.
//! - **Heights** ([`height_cap`], [`period_storeys`]): measured heights capped to the period's limits (a building over
//!   its cap a block at the cap; the few that stood taller, [`TALL`], keep theirs), estimated storeys fewer.
//! - **Working waterfronts** stand whole, whatever their score: a listed dockyard or shipyard from its year ([`YARDS`]),
//!   and in a port whose town stood ([`PORT_SHARE`]) the industrial and military land and the wharves within
//!   [`WATERFRONT_REACH_M`] of the water ([`Waterfront`]), but a container terminal's open yard and land made or turned
//!   to industry after the period ([`LATER_LAND`]). The land they keep, and the ground about a building a land's lift
//!   kept, is handed to the period's ground ([`Kept`]).
//! - **Old towns** ([`super::towns::TOWNS`]): within a listed town's extent the ground stands and a footprint is kept
//!   by [`core_floor`] alone ([`Thinning::with_towns`]); the cell's distribution counts the extent apart
//!   ([`Distribution::add_town`]) and its bar keeps only what the share leaves beyond what the extent keeps
//!   ([`Distribution::outside_share`]), so the extent's buildings are taken from the rest of the cell.

use super::tags::{MapTags, PoiKind, Setting, Usage};
use super::towns::Towns;
use super::{CHUNK_M, Footprint, Region, geometry, hash01};
use crate::infra::containers::{APRON_M, YardTest, port_about};

/// A cell's side (m): ten to a chunk's. Chunk-local cell (i, j) of chunk (ci, cj) is the theatre's cell
/// (10 ci + i, 10 cj + j).
pub const CELL_M: f32 = 100.0;
/// Cells to a chunk's side.
pub const CELLS: usize = 10;
/// How many cells about a cell its score reads.
pub const REACH: usize = 4;
/// The smoothing's standard deviation (cells).
pub const SIGMA: f32 = 1.8;
/// The most of one footprint's area a cell counts (m²), and of a measured one at least [`TALL_COUNTED_FROM_M`] tall.
pub const COUNTED_M2: f32 = 2000.0;
pub const TALL_COUNTED_M2: f32 = 4000.0;
pub const TALL_COUNTED_FROM_M: f32 = 12.0;
/// The noise's amplitude (score).
pub const NOISE: f32 = 0.06;
/// The built-up history's grid: cells to the degree.
pub const HYDE_PER_DEGREE: f64 = 12.0;
/// The footprints a 5' cell needs for its own distribution to set its bar.
pub const LEAST_SAMPLE: u32 = 300;
/// A score from which the ground is an old town's dense core: a shop's big box stands only there.
pub const CORE: f32 = 0.45;
/// What raises a footprint's score: a place of worship itself, the streets within [`WORSHIP_REACH_M`] of one (fading),
/// railway land, military land (a naval base, barracks), a harbour's front (a pier, or industrial land within
/// [`HARBOUR_REACH_M`] of the water).
pub const LIFT_WORSHIP: f32 = 0.25;
pub const LIFT_NEAR_WORSHIP: f32 = 0.1;
pub const WORSHIP_REACH_M: f32 = 150.0;
pub const LIFT_RAILWAY: f32 = 0.15;
pub const LIFT_MILITARY: f32 = 0.15;
pub const LIFT_HARBOUR: f32 = 0.12;
pub const HARBOUR_REACH_M: f32 = 110.0;
/// How far from the water (the map's sea, docks and rivers; beyond the map's reach, sea floor deeper than [`WET_M`]) a
/// working waterfront's land stands whole ([`Waterfront`]): a yard's slips, docks, shops and stores.
pub const WATERFRONT_REACH_M: f32 = 350.0;
/// The least share of a 5' cell about a port ([`cell_share`]) for its working waterfront to stand whole: its town stood
/// (Honiara's 1942 is 0.003, Reykjavík's 1941 0.07).
pub const PORT_SHARE: f32 = 0.05;
/// The sea floor taken for water where the map's water does not reach (m below the sea): deep enough that land below
/// the sea (Tokyo's zero-metre belt, a polder) is not.
pub const WET_M: f32 = 5.0;
/// The ground about a building a land's lift or the waterfront kept that stands with it (m).
pub const KEPT_MARGIN_M: f32 = 20.0;

/// Buildings that stood taller than their place's cap: latitude, longitude, height (m).
pub const TALL: &[(f64, f64, f32)] = &[
    // Los Angeles City Hall (1928), the one exception to the city's 150 ft limit.
    (34.05352, -118.24284, 138.0),
    // The Empire State Building (1931), over New York's and Chicago's 320 m.
    (40.748441, -73.985664, 381.0),
    // The National Diet Building's tower (1936), over Japan's 31 m.
    (35.675888, 139.744858, 65.0),
    // The Palace of Westminster's clock tower and Victoria Tower, over London's 100 ft.
    (51.500729, -0.124625, 96.0),
    (51.4988, -0.12528, 98.0),
    // The Royal Liver Building (1911), Liverpool.
    (53.40583, -2.99583, 98.0),
    // The Aloha Tower (1926), Honolulu.
    (21.3071, -157.8659, 56.0),
];

/// Buildings built after a year (latitude, longitude, the year they stood from): none stands in an earlier period.
pub const LATER: &[(f64, f64, u16)] = &[
    // Hallgrímskirkja, Reykjavík: begun in 1945, finished in 1986.
    (64.141944, -21.926667, 1986),
];

/// A place on the list of [`YARDS`] or of [`LATER_LAND`]: its name, the year it stood from (a yard's founding on its
/// site, land's making), and its ground as circles (latitude, longitude, radius in m).
#[derive(Clone, Copy, Debug)]
pub struct Yard {
    pub name: &'static str,
    pub year: u16,
    pub circles: &'static [(f64, f64, f32)],
}

const fn yard(name: &'static str, year: u16, circles: &'static [(f64, f64, f32)]) -> Yard {
    Yard {
        name,
        year,
        circles,
    }
}

/// The historic naval dockyards and major shipyards: within one every building stands from its year (no taller than its
/// place allowed), and its ground is a yard's; before its year the working waterfront's rule does not keep it.
///
/// Sources (2026-10-10): each yard's Wikipedia article for its founding and place; OpenStreetMap's outlines of the yard
/// and its dry docks (through Nominatim and Overpass) for its extent; for Los Angeles and Long Beach the USGS 1:24,000
/// sheets Long Beach 1949, Torrance 1951 and San Pedro 1951 (TopoView), which show the yards as built about 1945; for
/// Tokyo Bay the Geographical Survey Institute's tiles of the US Army's 1945-50 air photographs (`ort_USA10`). Places
/// marked "placed from today's map" were not checked against wartime pictures (about ±150 m).
pub const YARDS: &[Yard] = &[
    // Los Angeles and Long Beach.
    // California Shipbuilding (Calship), Terminal Island's north shore, berths 210-213: ground broken 27 January 1941.
    yard(
        "Calship",
        1941,
        &[
            (33.7568, -118.2610, 230.0),
            (33.7585, -118.2565, 280.0),
            (33.7600, -118.2520, 300.0),
            (33.7608, -118.2470, 280.0),
        ],
    ),
    // Bethlehem's San Pedro yard (Southwestern Shipbuilding's, 1918), berth 240 on Fish Harbor's west mole.
    yard(
        "Bethlehem San Pedro",
        1918,
        &[(33.7322, -118.2688, 230.0), (33.7352, -118.2688, 170.0)],
    ),
    // Fish Harbor's canneries and East San Pedro's village (the canneries from the 1910s; the village razed in 1945).
    yard(
        "Fish Harbor",
        1918,
        &[(33.7395, -118.2680, 300.0), (33.7375, -118.2640, 200.0)],
    ),
    // Los Angeles Shipbuilding and Dry Dock (Todd's from 1943), Smith's Island, 1917; placed from today's map.
    yard(
        "Los Angeles Shipbuilding",
        1917,
        &[(33.7530, -118.2810, 350.0)],
    ),
    // Consolidated Steel's Wilmington yard, the West Basin's north end, 1941; placed on the ground it left by 1951.
    yard(
        "Consolidated Wilmington",
        1941,
        &[(33.7680, -118.2735, 260.0), (33.7680, -118.2700, 240.0)],
    ),
    // Western Pipe and Steel's San Pedro yard, the West Basin's north-west corner, 1941; gone by 1951.
    yard(
        "Western Pipe and Steel",
        1941,
        &[(33.7678, -118.2790, 220.0)],
    ),
    // Reeves Field (Allen Field, 1927, on new fill; the Navy's from 1935-36; Naval Air Base San Pedro).
    yard(
        "Reeves Field",
        1927,
        &[(33.7480, -118.2590, 380.0), (33.7480, -118.2535, 380.0)],
    ),
    // Roosevelt Base (Naval Operating Base Terminal Island): building from 1940, founded September 1942, finished 1943.
    yard("Roosevelt Base", 1942, &[(33.7550, -118.2455, 420.0)]),
    // Terminal Island Naval Dry Docks (Long Beach Naval Shipyard from 1948): authorised 1940, dock 1 in 1942.
    yard(
        "Terminal Island Naval Dry Docks",
        1942,
        &[(33.7570, -118.2365, 380.0), (33.7575, -118.2290, 330.0)],
    ),
    // Craig Shipbuilding (1906) and Long Beach Shipbuilding (1917, Consolidated's from 1941), Channel 3's south side.
    yard("Craig Shipbuilding", 1906, &[(33.7695, -118.2145, 230.0)]),
    // Tokyo Bay.
    // Yokosuka Naval Arsenal: the iron works from 1865, dock 1 in 1871; the main yard, the shops and Hakozaki.
    yard(
        "Yokosuka Naval Arsenal",
        1871,
        &[
            (35.2885, 139.6610, 380.0),
            (35.2845, 139.6650, 300.0),
            (35.2915, 139.6680, 350.0),
            (35.2990, 139.6690, 420.0),
        ],
    ),
    // The arsenal's east quays (Commander Fleet Activities' eastern piers today), land in the 1945-50 photographs; the
    // year of the fill is a guess.
    yard("Yokosuka east quays", 1940, &[(35.2925, 139.6760, 380.0)]),
    // The navy's Hemi, Nagaura, Funakoshi and Taura waterfronts (1900s-1930s; the years a guess).
    yard(
        "Yokosuka outlying yards",
        1920,
        &[
            (35.2870, 139.6540, 250.0),
            (35.3047, 139.6450, 450.0),
            (35.2950, 139.6455, 450.0),
            (35.2975, 139.6345, 380.0),
        ],
    ),
    // Oppama: the air group's field (1916) and the Naval Air Arsenal (1 April 1932, later the Kūgishō), not the fill
    // north of 35.330° and east of 139.651° (1960s).
    yard(
        "Oppama Naval Air Arsenal",
        1932,
        &[
            (35.3250, 139.6405, 550.0),
            (35.3215, 139.6460, 350.0),
            (35.3130, 139.6440, 350.0),
        ],
    ),
    // Uraga Dock, Uraga harbour's west shore, 1897; placed from today's map (its docks).
    yard(
        "Uraga Dock",
        1897,
        &[(35.2465, 139.7160, 250.0), (35.2435, 139.7170, 200.0)],
    ),
    // Yokohama Dock (Mitsubishi's from 1935), docks of 1896-99 (the Landmark Tower and the Nippon Maru's dock today).
    yard(
        "Yokohama Dock",
        1896,
        &[(35.4548, 139.6322, 230.0), (35.4585, 139.6292, 220.0)],
    ),
    // Asano Shipbuilding, Tsurumi (Suehiro-chō), 1916-17; placed from today's map (±300 m).
    yard("Asano Tsurumi", 1917, &[(35.4845, 139.6860, 400.0)]),
    // Nippon Kōkan's Kawasaki works, Minami-Watarida, 1912-14.
    yard(
        "Nippon Kokan Kawasaki",
        1914,
        &[(35.5070, 139.7150, 450.0), (35.5040, 139.7200, 350.0)],
    ),
    // Ishikawajima, Tsukudajima's north tip (the Mito domain's yard of 1853; Ishikawajima's from 1876).
    yard("Ishikawajima", 1876, &[(35.6680, 139.7865, 300.0)]),
    // Britain and Iceland.
    // Portsmouth Dockyard's old yard (a dock of 1495, the stone dock and basin of the 1690s).
    yard("Portsmouth Dockyard", 1495, &[(50.8000, -1.1065, 420.0)]),
    // Portsmouth's Great Extension (basins of 1867-81 on Portsea's mud) and the works north of it (1900s-1910s).
    yard(
        "Portsmouth Dockyard extension",
        1876,
        &[(50.8055, -1.1040, 450.0), (50.8065, -1.0960, 400.0)],
    ),
    // The Royal Clarence victualling yard, Gosport, 1828-31.
    yard("Royal Clarence Yard", 1828, &[(50.7955, -1.1180, 220.0)]),
    // HMS Dolphin, the submarine base at Fort Blockhouse, 1904.
    yard("HMS Dolphin", 1904, &[(50.7882, -1.1178, 300.0)]),
    // Reykjavík's harbour works of 1913-17 (much of Grandi and Örfirisey is later fill).
    yard(
        "Reykjavik harbour",
        1917,
        &[(64.1505, -21.9420, 250.0), (64.1535, -21.9480, 250.0)],
    ),
    // Devonport: South Yard (1691), Morice Yard and Keyham's North Yard (1844-1907).
    yard(
        "Devonport Dockyard",
        1691,
        &[
            (50.3680, -4.1830, 350.0),
            (50.3743, -4.1820, 230.0),
            (50.3850, -4.1860, 450.0),
            (50.3910, -4.1890, 400.0),
        ],
    ),
    // Chatham: the old yard (about 1570) and St Mary's Island's basins (1865-85).
    yard(
        "Chatham Dockyard",
        1570,
        &[
            (51.3950, 0.5285, 450.0),
            (51.4000, 0.5400, 450.0),
            (51.4000, 0.5520, 400.0),
        ],
    ),
    // Rosyth, built 1909-16.
    yard(
        "Rosyth Dockyard",
        1916,
        &[
            (56.0250, -3.4500, 450.0),
            (56.0230, -3.4400, 450.0),
            (56.0225, -3.4580, 300.0),
        ],
    ),
    // Lyness, Hoy: the base of the first war, built again in 1939-40; placed from its Wikipedia point.
    yard("Lyness", 1939, &[(58.8335, -3.1930, 450.0)]),
    // The Pacific and the Far East; placed from today's map.
    // Pearl Harbor Navy Yard, 1908, dry dock 1 in 1919 (not the south-west's buildings of 1943-44).
    yard(
        "Pearl Harbor Navy Yard",
        1908,
        &[(21.3505, -157.9575, 500.0), (21.3530, -157.9470, 450.0)],
    ),
    // Ford Island's air station (flying from 1919; the naval air station from 1923).
    yard("Ford Island", 1919, &[(21.3632, -157.9607, 950.0)]),
    // Kure: the naval district 1889, the arsenal 1903.
    yard(
        "Kure Naval Arsenal",
        1889,
        &[
            (34.2330, 132.5560, 450.0),
            (34.2400, 132.5580, 350.0),
            (34.2285, 132.5500, 300.0),
        ],
    ),
    // Sasebo, 1889, at Tategami (Wikipedia's point is about 3 km off; placed by OpenStreetMap's docks).
    yard(
        "Sasebo Naval Arsenal",
        1889,
        &[
            (33.1610, 129.6990, 450.0),
            (33.1640, 129.7060, 350.0),
            (33.1640, 129.7140, 400.0),
        ],
    ),
    // Maizuru, 1901.
    yard(
        "Maizuru Naval Arsenal",
        1901,
        &[
            (35.4800, 135.3740, 400.0),
            (35.4765, 135.3780, 350.0),
            (35.4740, 135.3705, 300.0),
        ],
    ),
    // Mare Island, 1854 (docks of 1891, 1910, 1940 and 1942).
    yard(
        "Mare Island Navy Yard",
        1854,
        &[
            (38.1060, -122.2730, 500.0),
            (38.0990, -122.2700, 450.0),
            (38.0920, -122.2640, 450.0),
            (38.0850, -122.2570, 400.0),
        ],
    ),
    // Puget Sound Navy Yard, Bremerton, 1891.
    yard(
        "Puget Sound Navy Yard",
        1891,
        &[(47.5590, -122.6400, 550.0), (47.5600, -122.6300, 450.0)],
    ),
    // The Brooklyn Navy Yard, 1801 (dry dock 1 in 1851).
    yard(
        "Brooklyn Navy Yard",
        1801,
        &[(40.7010, -73.9760, 400.0), (40.7030, -73.9690, 450.0)],
    ),
    // The Norfolk Navy Yard at Portsmouth, Virginia (Gosport, 1767; dry dock 1 in 1834), not its annexes.
    yard(
        "Norfolk Navy Yard",
        1767,
        &[
            (36.8210, -76.2960, 450.0),
            (36.8150, -76.3000, 450.0),
            (36.8250, -76.2920, 300.0),
        ],
    ),
    // The Mediterranean.
    // Malta: Dockyard Creek and French Creek (the navy's from 1800; docks of 1848-1909; not the post-war dock 6).
    yard(
        "Malta Dockyard",
        1800,
        &[(35.8850, 14.5175, 300.0), (35.8800, 14.5165, 250.0)],
    ),
    // Alexandria's arsenal and the Ras el-Tin naval base (the arsenal 1829).
    yard(
        "Alexandria Arsenal",
        1829,
        &[(31.1950, 29.8640, 400.0), (31.1880, 29.8560, 350.0)],
    ),
    // The Haliç arsenal (Tersane-i Amire), Kasımpaşa: Selim I's of 1515 (its dry docks of the 19th century), and the
    // Taşkızak yard.
    yard(
        "Halic Arsenal",
        1515,
        &[
            (41.0280, 28.9680, 250.0),
            (41.0315, 28.9640, 250.0),
            (41.0345, 28.9600, 250.0),
        ],
    ),
    // Gölcük, 1924-26 (much of today's base is post-war).
    yard(
        "Golcuk Naval Yard",
        1926,
        &[(40.7225, 29.8030, 450.0), (40.7240, 29.8120, 400.0)],
    ),
    // Singapore's Naval Base, Sembawang, 1938.
    yard(
        "Singapore Naval Base",
        1938,
        &[(1.4620, 103.8230, 500.0), (1.4630, 103.8320, 400.0)],
    ),
    // The Cavite Navy Yard (the Spanish arsenal; the US Navy's from 1898; Sangley Point's air station is post-war).
    yard("Cavite Navy Yard", 1898, &[(14.4820, 120.9150, 350.0)]),
    // Hong Kong's Admiralty Dockyard, 1859-1959 (Admiralty and Tamar today; the land north of Harcourt Road is later).
    yard(
        "Hong Kong Admiralty Dockyard",
        1859,
        &[(22.2800, 114.1650, 280.0)],
    ),
    // Taikoo Dockyard, Quarry Bay, 1908.
    yard("Taikoo Dockyard", 1908, &[(22.2860, 114.2175, 380.0)]),
];

/// Land made or turned to industry after the period about the listed ports (post-war fill, its terminals and works):
/// before its year its waterfront does not stand whole (its buildings thin as any others).
///
/// Sources as [`YARDS`']: water on the USGS sheets of 1949-51 and in the 1945-50 air photographs, and OpenStreetMap's
/// outlines of the piers and districts today; Tokyo's and Chiba's from the Tokyo Metropolitan Government's and Chiba
/// Prefecture's histories of the bay's reclamation, placed from today's map. The years are each place's finishing;
/// circles reach generously (a circle only withholds the waterfront rule: the buildings still thin by their score).
pub const LATER_LAND: &[Yard] = &[
    // Los Angeles and Long Beach.
    // Pier 400 (1990s fill) and Pier 300 (1970s-80s, short of Fish Harbor and Reeves Field).
    yard(
        "Pier 400",
        2002,
        &[(33.7240, -118.2540, 550.0), (33.7270, -118.2440, 550.0)],
    ),
    yard(
        "Pier 300",
        1983,
        &[(33.7355, -118.2530, 420.0), (33.7375, -118.2470, 380.0)],
    ),
    // Long Beach's piers J (open sea in 1949) and G (water east of the 1949 mole), 1960s-70s.
    yard("Pier J", 1970, &[(33.7420, -118.1950, 550.0)]),
    yard("Pier G", 1960, &[(33.7520, -118.2010, 350.0)]),
    // Tokyo Bay: Kawasaki's and Yokohama's.
    yard(
        "Ogishima",
        1976,
        &[(35.4800, 139.7230, 900.0), (35.4790, 139.7360, 700.0)],
    ),
    yard(
        "Higashi-Ogishima",
        1985,
        &[(35.4960, 139.7480, 800.0), (35.4960, 139.7680, 800.0)],
    ),
    yard("Ukishima", 1970, &[(35.5180, 139.7750, 1000.0)]),
    yard(
        "Daikoku Pier",
        1990,
        &[(35.4600, 139.6800, 900.0), (35.4570, 139.6980, 600.0)],
    ),
    yard("Honmoku Pier", 1970, &[(35.4390, 139.6760, 900.0)]),
    yard("Minato Mirai fill", 1990, &[(35.4600, 139.6370, 350.0)]),
    yard(
        "Negishi fill",
        1971,
        &[(35.4180, 139.6380, 900.0), (35.4100, 139.6500, 800.0)],
    ),
    yard("Kanazawa fill", 1988, &[(35.3550, 139.6420, 1000.0)]),
    // Tokyo's port: Ōi and Jōnanjima (1960s-80s), Shinagawa's Tōkai and Heiwajima, the Ariake, Aomi, Odaiba and
    // Chūōbōhatei fills, Yumenoshima, Shin-Kiba and Wakasu (1957-90s).
    yard(
        "Tokyo Oi and Jonanjima",
        1975,
        &[
            (35.5900, 139.7600, 1500.0),
            (35.5780, 139.7700, 1200.0),
            (35.5850, 139.7450, 800.0),
            (35.5650, 139.7500, 700.0),
        ],
    ),
    yard(
        "Tokyo Ariake and Aomi",
        1975,
        &[
            (35.6180, 139.7780, 1000.0),
            (35.6320, 139.7900, 1000.0),
            (35.6050, 139.8100, 1500.0),
        ],
    ),
    yard(
        "Tokyo Shin-Kiba and Wakasu",
        1974,
        &[
            (35.6450, 139.8300, 900.0),
            (35.6520, 139.8300, 600.0),
            (35.6300, 139.8350, 1200.0),
        ],
    ),
    // Chiba's Keiyō coast, 1950s-80s: Urayasu, Ichikawa's and Funabashi's port, Makuhari, Chiba's port and Kawasaki
    // Steel's works (1953), Ichihara's, Sodegaura's and Kisarazu's petrochemical and steel estates, Kimitsu's works
    // (1965), Futtsu.
    yard(
        "Keiyo coast",
        1975,
        &[
            (35.6350, 139.8950, 2500.0),
            (35.6700, 139.9500, 2000.0),
            (35.6850, 139.9800, 1500.0),
            (35.6450, 140.0400, 2500.0),
            (35.5850, 140.0900, 2500.0),
            (35.6000, 140.0800, 1200.0),
            (35.5300, 140.0600, 3000.0),
            (35.4900, 140.0300, 3000.0),
            (35.4500, 139.9800, 2500.0),
            (35.4400, 139.9500, 2500.0),
            (35.3800, 139.9000, 2000.0),
            (35.3400, 139.8600, 3000.0),
            (35.3200, 139.8200, 2000.0),
        ],
    ),
];

/// The 5' cell (the built-up history's) holding (`lat`, `lon`): its column east from 180° W and its row north from the
/// equator.
pub fn hyde_cell(lat: f64, lon: f64) -> [i32; 2] {
    let lon = (lon + 540.0).rem_euclid(360.0) - 180.0;
    [
        (lon * HYDE_PER_DEGREE).floor() as i32,
        (lat * HYDE_PER_DEGREE).floor() as i32,
    ]
}

/// The middle of 5' cell `cell`: latitude and longitude.
pub fn hyde_middle(cell: [i32; 2]) -> (f64, f64) {
    (
        (f64::from(cell[1]) + 0.5) / HYDE_PER_DEGREE,
        (f64::from(cell[0]) + 0.5) / HYDE_PER_DEGREE,
    )
}

/// The four 5' cells whose middles lie about (`lat`, `lon`) and their bilinear weights (summing to 1), for blending
/// their bars ([`blend`]) so the kept land has no edge along the cells' sides.
pub fn hyde_about(lat: f64, lon: f64) -> [([i32; 2], f32); 4] {
    let lon = (lon + 540.0).rem_euclid(360.0) - 180.0;
    let fy = lat * HYDE_PER_DEGREE - 0.5;
    let fx = lon * HYDE_PER_DEGREE - 0.5;
    let (r0, c0) = (fy.floor(), fx.floor());
    let (ty, tx) = ((fy - r0) as f32, (fx - c0) as f32);
    let cell = |dc: f64, dr: f64| {
        let (lat, lon) = hyde_middle([(c0 + dc) as i32, (r0 + dr) as i32]);
        hyde_cell(lat, lon)
    };
    [
        (cell(0.0, 0.0), (1.0 - tx) * (1.0 - ty)),
        (cell(1.0, 0.0), tx * (1.0 - ty)),
        (cell(0.0, 1.0), (1.0 - tx) * ty),
        (cell(1.0, 1.0), tx * ty),
    ]
}

/// The bar between 5' cells' bars, each with its weight ([`hyde_about`]) and its footprints: a cell holding fewer
/// than [`LEAST_SAMPLE`] (the sea's, the country's) counts a fiftieth as much, so a coast's town is not thinned by the
/// empty cell off it. Bars held to -1-3 (none kept from 2).
pub fn blend(parts: &[(f32, f32, u32)]) -> f32 {
    let (mut sum, mut weight) = (0.0f32, 0.0f32);
    for &(bar, w, footprints) in parts {
        if w <= 0.0 {
            continue;
        }
        let w = if footprints >= LEAST_SAMPLE {
            w
        } else {
            w * 0.02
        };
        sum += w * bar.clamp(-1.0, 3.0);
        weight += w;
    }
    if weight > 0.0 { sum / weight } else { 3.0 }
}

/// The cell (0-99, row-major) of a chunk holding chunk-local point `p` (one just past an edge the edge's).
pub fn cell_of(p: [f32; 2]) -> usize {
    let at = |v: f32| ((v / CELL_M).floor() as i64).clamp(0, CELLS as i64 - 1) as usize;
    at(p[1]) * CELLS + at(p[0])
}

/// The chunk-local middle of a chunk's cell `k` (0-99).
pub fn cell_middle(k: usize) -> [f32; 2] {
    [
        ((k % CELLS) as f32 + 0.5) * CELL_M,
        ((k / CELLS) as f32 + 0.5) * CELL_M,
    ]
}

/// A chunk's footprints by cell (by each one's middle): the area counted (m², each to [`COUNTED_M2`]) and how many.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub area: Vec<f32>,
    pub count: Vec<u32>,
}

impl Default for Built {
    fn default() -> Self {
        Self {
            area: vec![0.0; CELLS * CELLS],
            count: vec![0; CELLS * CELLS],
        }
    }
}

impl Built {
    pub fn of<'a>(footprints: impl IntoIterator<Item = &'a Footprint>) -> Self {
        let mut built = Self::default();
        for f in footprints {
            built.add(f);
        }
        built
    }

    pub fn add(&mut self, f: &Footprint) {
        if f.ring.len() < 3 {
            return;
        }
        let k = cell_of(geometry::centroid(&f.ring));
        // A measured building of four storeys and more counts to [`TALL_COUNTED_M2`]: a city's blocks, not sheds.
        let most = if f.height.is_some_and(|h| h >= TALL_COUNTED_FROM_M) {
            TALL_COUNTED_M2
        } else {
            COUNTED_M2
        };
        self.area[k] += geometry::area(&f.ring).min(most);
        self.count[k] += 1;
    }

    /// The footprints counted.
    pub fn footprints(&self) -> u32 {
        self.count.iter().sum()
    }
}

/// The smoothing's weights, from -[`REACH`] to [`REACH`] cells, summing to 1.
fn kernel() -> [f32; 2 * REACH + 1] {
    let mut g = [0f32; 2 * REACH + 1];
    for (k, w) in g.iter_mut().enumerate() {
        let d = k as f32 - REACH as f32;
        *w = (-d * d / (2.0 * SIGMA * SIGMA)).exp();
    }
    let sum: f32 = g.iter().sum();
    g.map(|w| w / sum)
}

/// A low noise (-1 to 1) at the theatre's cell `cell`: value noise over 700 m, and half as much over 300 m.
pub fn noise(cell: [i64; 2]) -> f32 {
    let octave = |size: i64, seed: i64| {
        let (x, z) = (cell[0].div_euclid(size), cell[1].div_euclid(size));
        let f = |v: i64| (v.rem_euclid(size) as f32 + 0.5) / size as f32;
        let ease = |t: f32| t * t * (3.0 - 2.0 * t);
        let (u, v) = (ease(f(cell[0])), ease(f(cell[1])));
        let c = |dx: i64, dz: i64| hash01(x + dx, z + dz, seed) * 2.0 - 1.0;
        let a = c(0, 0) + (c(1, 0) - c(0, 0)) * u;
        let b = c(0, 1) + (c(1, 1) - c(0, 1)) * u;
        a + (b - a) * v
    };
    (octave(7, 91) + 0.5 * octave(3, 92)) / 1.5
}

/// The theatre's cell of chunk `index`'s cell (`i`, `j`).
fn theatre_cell(index: [i32; 2], i: usize, j: usize) -> [i64; 2] {
    [
        i64::from(index[0]) * CELLS as i64 + i as i64,
        i64::from(index[1]) * CELLS as i64 + j as i64,
    ]
}

/// The scores over a window of `chunks` (columns, rows) chunks from chunk `first`, `built` its chunks row by row (none
/// for a chunk without footprints): each cell's share of the ground its footprints cover (counted), smoothed, plus the
/// noise; `chunks[0] × 10` cells to a row. A cell [`REACH`] cells or more inside the window scores the same whatever
/// window holds it (its sums run over the same cells in the same order).
pub fn window_scores(first: [i32; 2], chunks: [usize; 2], built: &[Option<&Built>]) -> Vec<f32> {
    let (w, h) = (chunks[0] * CELLS, chunks[1] * CELLS);
    let mut share = vec![0f32; w * h];
    for cj in 0..chunks[1] {
        for ci in 0..chunks[0] {
            let Some(b) = built[cj * chunks[0] + ci] else {
                continue;
            };
            for j in 0..CELLS {
                for i in 0..CELLS {
                    share[(cj * CELLS + j) * w + ci * CELLS + i] =
                        b.area[j * CELLS + i] / (CELL_M * CELL_M);
                }
            }
        }
    }
    let first = [
        i64::from(first[0]) * CELLS as i64,
        i64::from(first[1]) * CELLS as i64,
    ];
    smoothed_scores(first, [w, h], &share)
}

/// The scores over a window of `size` (columns, rows) cells from the theatre's cell `first`, `share` each cell's share
/// of its ground built on, row by row: smoothed over [`REACH`] cells about (a cell beyond the window counts none), plus
/// the noise. What [`window_scores`] reads from the footprints, and the period's ground (`naval-client`
/// `theatre::period_land`) from the land cover where there are none: a cell [`REACH`] cells or more inside the window
/// scores the same whatever window holds it.
pub fn smoothed_scores(first: [i64; 2], size: [usize; 2], share: &[f32]) -> Vec<f32> {
    let [w, h] = size;
    assert_eq!(share.len(), w * h, "a share a cell");
    let g = kernel();
    let reach = REACH as i64;
    let mut across = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (k, wk) in g.iter().enumerate() {
                let xx = x as i64 + k as i64 - reach;
                if (0..w as i64).contains(&xx) {
                    s += wk * share[y * w + xx as usize];
                }
            }
            across[y * w + x] = s;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0.0;
            for (k, wk) in g.iter().enumerate() {
                let yy = y as i64 + k as i64 - reach;
                if (0..h as i64).contains(&yy) {
                    s += wk * across[yy as usize * w + x];
                }
            }
            let cell = [first[0] + x as i64, first[1] + y as i64];
            out[y * w + x] = s + NOISE * noise(cell);
        }
    }
    out
}

/// The scores of the theatre about chunk `index` from `built`, its 3 × 3 chunks' footprints row by row from the one
/// west and north of it: [`window_scores`] over them, for [`Thinning::new`].
pub fn chunk_window(index: [i32; 2], built: &[Option<&Built>; 9]) -> Vec<f32> {
    window_scores([index[0] - 1, index[1] - 1], [3, 3], built)
}

/// The distribution's bins and their range (scores).
pub const BINS: usize = 1024;
pub const LOW: f32 = -0.25;
pub const HIGH: f32 = 1.25;

/// A 5' cell's footprints by their cells' scores: how many in each of [`BINS`] bins from [`LOW`] to [`HIGH`], those
/// outside the listed towns' extents ([`super::towns`]) and those within them apart.
#[derive(Clone, Debug, PartialEq)]
pub struct Distribution {
    bins: Vec<u32>,
    /// The footprints outside the towns' extents.
    pub total: u32,
    /// Those within one, by score likewise (empty while none is).
    town_bins: Vec<u32>,
    pub town: u32,
}

impl Default for Distribution {
    fn default() -> Self {
        Self {
            bins: vec![0; BINS],
            total: 0,
            town_bins: Vec::new(),
            town: 0,
        }
    }
}

impl Distribution {
    fn step() -> f32 {
        (HIGH - LOW) / BINS as f32
    }

    fn bin(score: f32) -> usize {
        ((score - LOW) / Self::step())
            .floor()
            .clamp(0.0, (BINS - 1) as f32) as usize
    }

    /// Counts `count` footprints in a cell scoring `score`.
    pub fn add(&mut self, score: f32, count: u32) {
        if count == 0 {
            return;
        }
        self.bins[Self::bin(score)] += count;
        self.total += count;
    }

    /// Counts `count` footprints in a cell scoring `score` within a listed town's extent.
    pub fn add_town(&mut self, score: f32, count: u32) {
        if count == 0 {
            return;
        }
        if self.town_bins.is_empty() {
            self.town_bins = vec![0; BINS];
        }
        self.town_bins[Self::bin(score)] += count;
        self.town += count;
    }

    /// Every footprint counted, in a town or not.
    pub fn footprints(&self) -> u32 {
        self.total + self.town
    }

    /// The distribution with the towns' footprints counted as any.
    pub fn merged(&self) -> Self {
        let mut bins = self.bins.clone();
        for (b, t) in bins.iter_mut().zip(&self.town_bins) {
            *b += t;
        }
        Self {
            bins,
            total: self.footprints(),
            ..Self::default()
        }
    }

    /// The footprints within the towns' extents that a bar of `bar` keeps (those in its bin and above).
    pub fn town_kept(&self, bar: f32) -> u32 {
        if self.town_bins.is_empty() {
            return 0;
        }
        self.town_bins[Self::bin(bar)..].iter().sum()
    }

    /// The share of the footprints outside the towns' extents to keep so that the cell keeps `share` of them all, the
    /// towns' kept by `town_bar` counted first; none when the towns already keep that much (and the cell keeps
    /// more), all when they keep none of it and the share is whole.
    pub fn outside_share(&self, share: f32, town_bar: f32) -> f32 {
        if self.town == 0 || self.total == 0 {
            return share;
        }
        let want = share * self.footprints() as f32 - self.town_kept(town_bar) as f32;
        (want / self.total as f32).clamp(0.0, 1.0)
    }

    /// The bar that keeps `share` (0-1) of the footprints: the lower edge of the bin where those scoring above it reach
    /// the share (a little more is kept, never less).
    pub fn bar(&self, share: f32) -> f32 {
        if share >= 1.0 {
            return f32::NEG_INFINITY;
        }
        if share <= 0.0 || self.total == 0 {
            return f32::INFINITY;
        }
        let want = share * self.total as f32;
        let mut kept = 0u32;
        for b in (1..BINS).rev() {
            kept += self.bins[b];
            if kept as f32 >= want {
                return LOW + b as f32 * Self::step();
            }
        }
        f32::NEG_INFINITY
    }
}

/// A bar by the share alone, where a 5' cell holds too few footprints for its own distribution: a town's dense core
/// (0.38 and more) at a share of 0.4, its thinner streets toward a share of 1; nothing below a share of 1 in 50.
pub fn absolute_bar(share: f32) -> f32 {
    if share < 0.02 {
        f32::INFINITY
    } else {
        0.62 - 0.6 * share.min(1.0)
    }
}

/// The least score kept, whatever the share: the scattered houses of the country and of the thinnest suburbs go even
/// where a 5' cell's share is whole (most of the Americas' low suburbs were built after the war).
pub fn core_floor(region: Region) -> f32 {
    match region {
        Region::American | Region::West | Region::Florida | Region::Hawaii => 0.15,
        _ => 0.1,
    }
}

/// A cell's bar outside the listed towns' extents: by its 5' cell's distribution when that holds enough footprints,
/// else by the share alone, and never under the region's floor; nothing kept where nothing stood. What the towns in the
/// cell keep (by the floor, [`Thinning::bar_at`]) is taken out of the share first ([`Distribution::outside_share`]),
/// so the cell keeps about its share in all unless its towns alone keep more.
pub fn bar(distribution: Option<&Distribution>, share: f32, region: Region) -> f32 {
    if share <= 0.0 {
        return f32::INFINITY;
    }
    let floor = core_floor(region);
    let share = distribution.map_or(share, |d| d.outside_share(share, floor));
    if share <= 0.0 {
        return f32::INFINITY;
    }
    let bar = match distribution {
        Some(d) if d.total >= LEAST_SAMPLE => d.bar(share),
        _ => absolute_bar(share),
    };
    bar.max(floor)
}

/// 5' cell `cell`'s bar in `year` from its `distribution` (none: by the share alone): its history's share
/// ([`history_about`], [`period_share`]) kept in its region ([`Region::at`] its middle, by `region`), as [`bar`].
pub fn cell_bar(
    cell: [i32; 2],
    distribution: Option<&Distribution>,
    year: u16,
    region: Region,
) -> f32 {
    bar(distribution, cell_share(cell, year, region), region)
}

/// A 5' cell's bars for [`bar_about`]: outside its listed towns' extents with what they keep paid out of the rest
/// (`bar`, [`bar`]), as if no town were listed (`plain`: every footprint thinned alike, as its neighbours read it),
/// and its footprints.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FiveBars {
    pub bar: f32,
    pub plain: f32,
    pub footprints: u32,
}

impl FiveBars {
    /// A cell nothing is known of: nothing kept, and a fiftieth of a weight in the blend.
    pub const NONE: Self = Self {
        bar: f32::INFINITY,
        plain: f32::INFINITY,
        footprints: 0,
    };
}

/// 5' cell `cell`'s bars in `year` from its `distribution`, as [`cell_bar`].
pub fn cell_bars(
    cell: [i32; 2],
    distribution: Option<&Distribution>,
    year: u16,
    region: Region,
) -> FiveBars {
    let share = cell_share(cell, year, region);
    let paid = bar(distribution, share, region);
    let plain = match distribution {
        Some(d) if d.town > 0 => bar(Some(&d.merged()), share, region),
        _ => paid,
    };
    FiveBars {
        bar: paid,
        plain,
        footprints: distribution.map_or(0, Distribution::footprints),
    }
}

/// The share 5' cell `cell` keeps in `year` in `region` ([`history_about`], [`period_share`]).
pub fn cell_share(cell: [i32; 2], year: u16, region: Region) -> f32 {
    let (own, around) = history_about(cell, year);
    period_share(own, around, region, year)
}

/// Whether a port's town stood about (`lat`, `lon`) in `year`: its 5' cell keeps at least [`PORT_SHARE`] ([`cell_share`],
/// so where the history put the town's people in the cells about, three quarters of theirs; `region` the cell's region
/// at its middle).
pub fn port_stood(lat: f64, lon: f64, year: u16, region: &dyn Fn(f64, f64) -> Region) -> bool {
    let cell = hyde_cell(lat, lon);
    let (lat, lon) = hyde_middle(cell);
    cell_share(cell, year, region(lat, lon)) >= PORT_SHARE
}

/// How far inside a 5' cell's sides (in its sides' lengths: some 900 m) the bar paid for its listed towns
/// ([`FiveBars::bar`]) is eased in from its plain bar.
pub const TOWN_EASE: f64 = 0.1;

/// The bar at (`lat`, `lon`): the bars of the four 5' cells about it blended ([`hyde_about`], [`blend`]), `five` each
/// one's bars and footprints (asked once a cell with weight). Each cell's plain bar is blended, but for the cell the
/// point lies in, whose bar paid for its towns is eased in over [`TOWN_EASE`] inside its sides: a cell's towns are
/// paid for by its own land, not its neighbours', with no step along its sides.
pub fn bar_about(lat: f64, lon: f64, five: &mut dyn FnMut([i32; 2]) -> FiveBars) -> f32 {
    let own = hyde_cell(lat, lon);
    let ease = {
        let lon = (lon + 540.0).rem_euclid(360.0) - 180.0;
        let inside = |v: f64| {
            let f = v * HYDE_PER_DEGREE - (v * HYDE_PER_DEGREE).floor();
            f.min(1.0 - f)
        };
        let t = (inside(lat).min(inside(lon)) / TOWN_EASE).clamp(0.0, 1.0) as f32;
        t * t * (3.0 - 2.0 * t)
    };
    let mut parts = [(0.0f32, 0.0f32, 0u32); 4];
    for (part, (cell, w)) in parts.iter_mut().zip(hyde_about(lat, lon)) {
        if w <= 0.0 {
            continue;
        }
        let f = five(cell);
        let plain = f.plain.clamp(-1.0, 3.0);
        let b = if cell == own && f.bar != f.plain {
            plain + (f.bar.clamp(-1.0, 3.0) - plain) * ease
        } else {
            f.plain
        };
        *part = (b, w, f.footprints);
    }
    blend(&parts)
}

/// The share a 5' cell keeps from the history's `share` there (none where it has no data) and `around`, the mean of
/// the history's shares over it and the eight cells about it that have data, in `region` in `year`.
///
/// The history spreads a province's people by today's pattern, so a cell of post-war suburbs reads two or three times
/// too built up while a core reads right: its shares below 1 are sharpened, to the power of 1.5 in the Americas and
/// 1.25 elsewhere. It also puts a town's people in a neighbour's cells now and then (Yokosuka's in Yokohama's, San
/// Pedro's 1942 in the cells north of it): a cell whose share is under a quarter of `around` keeps three quarters of
/// `around` instead. Where it has no data, [`default_share`].
pub fn period_share(share: Option<f32>, around: Option<f32>, region: Region, year: u16) -> f32 {
    let Some(s) = share else {
        return default_share(region, year);
    };
    let s = s.clamp(0.0, 1.0);
    if let Some(a) = around
        && s < 0.25 * a
    {
        return 0.75 * a.clamp(0.0, 1.0);
    }
    let power = match region {
        Region::American | Region::West | Region::Florida | Region::Hawaii => 1.5,
        _ => 1.25,
    };
    s.powf(power)
}

/// The history's share at 5' cell `cell`'s middle in `year`, and the mean of its shares over the cell and the eight
/// about it that have data (for [`period_share`]).
pub fn history_about(cell: [i32; 2], year: u16) -> (Option<f32>, Option<f32>) {
    let at = |c: [i32; 2]| {
        let (lat, lon) = hyde_middle(c);
        super::history::stood_share(lat, lon, year)
    };
    let (mut sum, mut n) = (0.0f32, 0u32);
    for dy in -1..=1 {
        for dx in -1..=1 {
            if let Some(s) = at([cell[0] + dx, cell[1] + dy]) {
                sum += s;
                n += 1;
            }
        }
    }
    (at(cell), (n > 0).then(|| sum / n as f32))
}

/// The share of today's built-up land that stood in `year` where the history has none (a town it counts as country,
/// off its grid): by region about 1942, fewer before and more after.
pub fn default_share(region: Region, year: u16) -> f32 {
    let share: f32 = match region {
        Region::British => 0.55,
        Region::Celtic | Region::NorthSea => 0.5,
        Region::Malta => 0.6,
        Region::Temperate => 0.45,
        Region::Nordic
        | Region::Mediterranean
        | Region::EastAsia
        | Region::Taiwan
        | Region::Aegean => 0.35,
        Region::American => 0.4,
        Region::West => 0.3,
        Region::Iceland | Region::NorthAfrica | Region::Nile | Region::Hawaii => 0.25,
        Region::Florida => 0.2,
        Region::Arid => 0.15,
        Region::Tropical | Region::Micronesia | Region::Atoll => 0.1,
    };
    let growth = 1.0 + (f32::from(year) - 1942.0) * 0.012;
    (share * growth.max(0.3)).clamp(0.0, 1.0)
}

/// The most a measured building stands in the period (m), at (`lat`, `lon`) in `region`: Los Angeles' and the West's
/// 150 ft (until 1957), the rest of America's 45 m, Japan's 100 shaku (31 m, until the 1960s), London's 80-100 ft,
/// continental Europe's, the Mediterranean's and the Nordic countries' 25 m, Hawaii's 20, the tropics' and the
/// islands' 12; New York's and Chicago's skyscrapers stood (to 320 m).
pub fn height_cap(region: Region, lat: f64, lon: f64) -> f32 {
    let lon = (lon + 540.0).rem_euclid(360.0) - 180.0;
    let within = |a: f64, b: f64, v: f64| (a..=b).contains(&v);
    let new_york = within(40.45, 41.0, lat) && within(-74.3, -73.65, lon);
    let chicago = within(41.6, 42.1, lat) && within(-87.95, -87.5, lon);
    if new_york || chicago {
        return 320.0;
    }
    match region {
        Region::West => 46.0,
        Region::American | Region::Florida => 45.0,
        // Japan's 100 shaku in Taiwan too, under Japanese rule.
        Region::EastAsia | Region::Taiwan => 31.0,
        Region::British | Region::Celtic => 30.0,
        Region::Hawaii => 20.0,
        Region::Tropical | Region::Micronesia | Region::Atoll => 12.0,
        _ => 25.0,
    }
}

/// The most storeys an estimated building has in the period: six in Europe and the Americas, five by the Nile, four in
/// East Asia, the dry countries, Iceland and Hawaii, three in the tropics and on the islands.
pub fn period_storeys(region: Region) -> u8 {
    match region {
        Region::Tropical | Region::Micronesia | Region::Atoll => 3,
        Region::EastAsia
        | Region::Taiwan
        | Region::Hawaii
        | Region::Iceland
        | Region::NorthAfrica
        | Region::Arid => 4,
        Region::Nile => 5,
        _ => 6,
    }
}

/// What lifts a footprint's score (`setting` its setting in `tags`, `ring` chunk-local): a place of worship and the
/// streets about one, railway land, a harbour's front.
pub fn lift(
    tags: &MapTags,
    setting: &Setting,
    ring: &[[f32; 2]],
    ground: &dyn Fn([f32; 2]) -> f32,
) -> f32 {
    let (worship, land) = lifts(tags, setting, ring, ground);
    worship + land
}

/// [`lift`]'s two parts: a place of worship's (and the streets about it), and its land's (railway or military land, a
/// harbour's front).
pub fn lifts(
    tags: &MapTags,
    setting: &Setting,
    ring: &[[f32; 2]],
    ground: &dyn Fn([f32; 2]) -> f32,
) -> (f32, f32) {
    let c = geometry::centroid(ring);
    let mut worship = 0f32;
    if matches!(setting.poi, Some(PoiKind::Worship(_))) {
        worship = LIFT_WORSHIP;
    }
    for p in &tags.pois {
        if matches!(p.kind, PoiKind::Worship(_)) {
            let d = geometry::distance(p.at, c);
            if d < WORSHIP_REACH_M {
                worship = worship.max(LIFT_NEAR_WORSHIP * (1.0 - d / WORSHIP_REACH_M));
            }
        }
    }
    let mut land = 0f32;
    if tags.zones.iter().any(|z| z.railway && z.holds(c)) {
        land = LIFT_RAILWAY;
    }
    let harbour = setting.pier.is_some()
        || (setting.usage == Usage::Industrial
            && [HARBOUR_REACH_M * 0.5, HARBOUR_REACH_M].iter().any(|r| {
                (0..8).any(|k| {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    tags.is_water([c[0] + r * a.cos(), c[1] + r * a.sin()], ground)
                })
            }));
    if harbour {
        land = land.max(LIFT_HARBOUR);
    }
    if setting.usage == Usage::Military {
        land = land.max(LIFT_MILITARY);
    }
    (worship, land)
}

/// Cells to a side of a [`Thinning`]'s scores: the chunk's and a ring of its neighbours'.
pub const RING: usize = CELLS + 2;

/// A value given at the centres of a chunk's cells and of the ring about them (`ring`, [`RING`]² row-major from the
/// cell west and north of its corner, as [`Thinning::score`]) at chunk-local point `p`: between the four nearest.
pub fn ring_at(ring: &[f32], p: [f32; 2]) -> f32 {
    let top = (RING - 1) as f32;
    let u = (p[0] / CELL_M + 0.5).clamp(0.0, top);
    let v = (p[1] / CELL_M + 0.5).clamp(0.0, top);
    let i = (u.floor() as usize).min(RING - 2);
    let j = (v.floor() as usize).min(RING - 2);
    let (fu, fv) = (u - i as f32, v - j as f32);
    let s = |i: usize, j: usize| ring[j * RING + i];
    let a = s(i, j) + (s(i + 1, j) - s(i, j)) * fu;
    let b = s(i, j + 1) + (s(i + 1, j + 1) - s(i, j + 1)) * fu;
    a + (b - a) * fv
}

/// Cells of [`Kept`] to a chunk's side, and a cell's side (m).
pub const KEPT_CELLS: usize = 40;
pub const KEPT_CELL_M: f32 = CHUNK_M as f32 / KEPT_CELLS as f32;
/// Cells of [`Kept`] beyond each edge of its chunk (100 m): the ground about a footprint whose middle lies in the
/// chunk and whose ring and margin reach over its edge, which the neighbouring chunk's ground reads.
pub const KEPT_BORDER: usize = 4;
const KEPT_SIDE: usize = KEPT_CELLS + 2 * KEPT_BORDER;

/// The land of a chunk a past period keeps whole whatever its score: a listed yard's, a working waterfront's
/// ([`Waterfront`]), and the ground about a building a land's lift kept ([`KEPT_MARGIN_M`], over the chunk's edge
/// too, [`KEPT_BORDER`]); a bit a [`KEPT_CELL_M`] cell. [`super::build_chunk`] makes it (`ChunkMesh::kept`); the
/// period's ground reads it (`naval-client`'s `theatre::period_land`), so a kept yard stands on a yard's ground.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    /// [`KEPT_CELLS`] + 2 [`KEPT_BORDER`] squared bits row by row, from the cell [`KEPT_BORDER`] west and north of the
    /// chunk's corner; empty when none is kept.
    bits: Vec<u64>,
}

impl Kept {
    const WORDS: usize = (KEPT_SIDE * KEPT_SIDE).div_ceil(64);

    /// The bit of cell (`i`, `j`) (the chunk's own from 0 to [`KEPT_CELLS`] - 1), none beyond the border.
    fn bit(i: i64, j: i64) -> Option<usize> {
        let b = KEPT_BORDER as i64;
        let cells = -b..KEPT_CELLS as i64 + b;
        (cells.contains(&i) && cells.contains(&j))
            .then(|| (j + b) as usize * KEPT_SIDE + (i + b) as usize)
    }

    /// The middle of cell (`i`, `j`), chunk-local.
    fn middle(i: i64, j: i64) -> [f32; 2] {
        [
            (i as f32 + 0.5) * KEPT_CELL_M,
            (j as f32 + 0.5) * KEPT_CELL_M,
        ]
    }

    pub fn is_empty(&self) -> bool {
        self.bits.iter().all(|w| *w == 0)
    }

    /// The chunk's own cells kept.
    pub fn count(&self) -> usize {
        let n = KEPT_CELLS as i64;
        (0..n)
            .flat_map(|j| (0..n).map(move |i| (i, j)))
            .filter(|&(i, j)| Self::bit(i, j).is_some_and(|b| self.get(b)))
            .count()
    }

    /// Whether a cell beyond the chunk's edge is kept (the neighbours' ground reads it).
    pub fn spills(&self) -> bool {
        let (b, n) = (KEPT_BORDER as i64, KEPT_CELLS as i64);
        (-b..n + b)
            .flat_map(|j| (-b..n + b).map(move |i| (i, j)))
            .filter(|&(i, j)| !(0..n).contains(&i) || !(0..n).contains(&j))
            .any(|(i, j)| Self::bit(i, j).is_some_and(|k| self.get(k)))
    }

    /// The middle of the chunk's own cell `k` (row by row).
    #[cfg(test)]
    fn cell(k: usize) -> [f32; 2] {
        Self::middle((k % KEPT_CELLS) as i64, (k / KEPT_CELLS) as i64)
    }

    fn set(&mut self, k: usize) {
        if self.bits.is_empty() {
            self.bits = vec![0; Self::WORDS];
        }
        self.bits[k / 64] |= 1 << (k % 64);
    }

    fn get(&self, k: usize) -> bool {
        self.bits
            .get(k / 64)
            .is_some_and(|w| w & (1 << (k % 64)) != 0)
    }

    /// Keeps the chunk's own cells whose middles `keep` holds.
    pub fn set_where(&mut self, keep: impl Fn([f32; 2]) -> bool) {
        let n = KEPT_CELLS as i64;
        for j in 0..n {
            for i in 0..n {
                let Some(k) = Self::bit(i, j) else {
                    continue;
                };
                if !self.get(k) && keep(Self::middle(i, j)) {
                    self.set(k);
                }
            }
        }
    }

    /// Keeps the cells, the border's too, whose middles `keep` holds: a footprint whose middle lies just over the
    /// chunk's edge (its ring over both) is read there.
    pub fn set_where_about(&mut self, keep: impl Fn([f32; 2]) -> bool) {
        let (b, n) = (KEPT_BORDER as i64, KEPT_CELLS as i64);
        for j in -b..n + b {
            for i in -b..n + b {
                let Some(k) = Self::bit(i, j) else {
                    continue;
                };
                if !self.get(k) && keep(Self::middle(i, j)) {
                    self.set(k);
                }
            }
        }
    }

    /// Keeps the cells (the border's too) whose middles lie within `ring` or `margin` m of it (chunk-local).
    pub fn set_ring(&mut self, ring: &[[f32; 2]], margin: f32) {
        if ring.len() < 3 {
            return;
        }
        let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
        for p in ring {
            for a in 0..2 {
                lo[a] = lo[a].min(p[a] - margin);
                hi[a] = hi[a].max(p[a] + margin);
            }
        }
        // The cells whose middles lie in the box.
        let (b, n) = (KEPT_BORDER as i64, KEPT_CELLS as i64);
        let span = |a: usize| {
            let first = ((lo[a] / KEPT_CELL_M - 0.5).ceil() as i64).max(-b);
            let last = ((hi[a] / KEPT_CELL_M - 0.5).floor() as i64).min(n + b - 1);
            first..=last
        };
        for j in span(1) {
            for i in span(0) {
                let Some(k) = Self::bit(i, j) else {
                    continue;
                };
                let c = Self::middle(i, j);
                if !self.get(k)
                    && (geometry::contains(ring, c) || geometry::ring_distance(ring, c) <= margin)
                {
                    self.set(k);
                }
            }
        }
    }

    /// Whether chunk-local point `p` is kept (by its cell; none beyond the border).
    pub fn holds(&self, p: [f32; 2]) -> bool {
        if self.bits.is_empty() {
            return false;
        }
        let at = |v: f32| (v / KEPT_CELL_M).floor() as i64;
        Self::bit(at(p[0]), at(p[1])).is_some_and(|k| self.get(k))
    }
}

/// What [`super::build_chunk`] thins a chunk by in a past year (the host's, from its stores and the history).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Thinning {
    /// The scores at the centres of the chunk's cells and of the ring about them (12 × 12, row-major from the cell
    /// west and north of its corner, (-1, -1)).
    pub score: Vec<f32>,
    /// Each of the chunk's cells' bar (10 × 10).
    pub bar: Vec<f32>,
    /// The most a measured building stands here (m, [`height_cap`]).
    pub cap_m: f32,
    /// Buildings that stood taller ([`TALL`]): chunk-local point, height (m).
    pub tall: Vec<([f32; 2], f32)>,
    /// Buildings built after the period ([`LATER`]): chunk-local points.
    pub later: Vec<[f32; 2]>,
    /// The listed yards standing in the period ([`YARDS`]): chunk-local middle and radius (m) of each circle about the
    /// chunk.
    pub yards: Vec<([f32; 2], f32)>,
    /// Where the working waterfront does not stand whole ([`Waterfront`]): a listed yard before its year, land made
    /// after the period ([`LATER_LAND`]); circles as `yards`'.
    pub not_yet: Vec<([f32; 2], f32)>,
    /// Whether a port's town stood about the chunk ([`port_stood`], or a listed town about it): its working waterfront
    /// stands whole.
    pub port: bool,
    /// The chunk's cells within a listed town's extent ([`super::towns::Towns::cells`]): their ground stood, and their
    /// buildings are kept by `town_bar` rather than their cells' bars.
    pub town: Kept,
    /// The bar within a town's extent: the region's floor ([`core_floor`]).
    pub town_bar: f32,
    /// The land kept whole whatever its score: the host's, from what the chunk's buildings kept (`ChunkMesh::kept`), for
    /// the period's ground ([`Self::stood`]). [`super::build_chunk`] does not read it.
    pub kept: Kept,
}

/// The listed places' circles in `year` (latitude, longitude, radius in m) and whether each stands: a [`YARDS`]' from
/// its year (true), before it (false); a [`LATER_LAND`]'s before its year (false), none from it.
pub fn listed(year: u16) -> impl Iterator<Item = (f64, f64, f32, bool)> {
    YARDS
        .iter()
        .map(move |place| (place, year >= place.year))
        .chain(
            LATER_LAND
                .iter()
                .filter(move |place| year < place.year)
                .map(|place| (place, false)),
        )
        .flat_map(|(place, stands)| {
            place
                .circles
                .iter()
                .map(move |&(lat, lon, r)| (lat, lon, r, stands))
        })
}

/// Whether one of `circles` (chunk-local middle, radius) holds chunk-local point `p`.
fn in_circles(circles: &[([f32; 2], f32)], p: [f32; 2]) -> bool {
    circles.iter().any(|(c, r)| geometry::distance(*c, p) <= *r)
}

impl Thinning {
    /// From the scores about the chunk ([`chunk_window`]), its cells' bars (10 × 10) and its cap.
    pub fn new(window: &[f32], bar: Vec<f32>, cap_m: f32) -> Self {
        let w = 3 * CELLS;
        assert_eq!(window.len(), w * w, "a 3 × 3 chunks' window");
        assert_eq!(bar.len(), CELLS * CELLS, "a bar a cell");
        let score = (0..RING * RING)
            .map(|k| window[(CELLS - 1 + k / RING) * w + CELLS - 1 + k % RING])
            .collect();
        Self {
            score,
            bar,
            cap_m,
            ..Self::default()
        }
    }

    /// The listed places' circles about chunk `index` (`local` a latitude and longitude's chunk-local point): the
    /// [`YARDS`] standing in `year` into `yards`, those not yet standing and the [`LATER_LAND`] not yet made into
    /// `not_yet`.
    pub fn with_listed(mut self, year: u16, local: &dyn Fn(f64, f64) -> [f32; 2]) -> Self {
        for (lat, lon, r, stands) in listed(year) {
            self.add_listed(local(lat, lon), r, stands);
        }
        self
    }

    /// Adds a listed circle ([`listed`]) about chunk-local point `p`, `r` m across, if it reaches the chunk.
    pub fn add_listed(&mut self, p: [f32; 2], r: f32, stands: bool) {
        let side = CHUNK_M as f32;
        if p[0] > -r && p[0] < side + r && p[1] > -r && p[1] < side + r {
            if stands {
                self.yards.push((p, r));
            } else {
                self.not_yet.push((p, r));
            }
        }
    }

    /// Whether chunk-local point `p` lies in a listed yard standing in the period.
    pub fn in_yard(&self, p: [f32; 2]) -> bool {
        in_circles(&self.yards, p)
    }

    /// Whether chunk-local point `p` lies where the working waterfront does not stand whole.
    pub fn not_yet(&self, p: [f32; 2]) -> bool {
        in_circles(&self.not_yet, p)
    }

    /// The listed towns' extents over the chunk whose corner is theatre point `corner` ([`Towns::cells`]), their
    /// buildings kept by `bar` (the region's floor, [`core_floor`]).
    pub fn with_towns(mut self, towns: &Towns, corner: [f64; 2], bar: f32) -> Self {
        self.town = towns.cells(corner);
        self.town_bar = bar;
        self
    }

    /// Whether chunk-local point `p` lies within a listed town's extent (by its [`KEPT_CELL_M`] cell).
    pub fn in_town(&self, p: [f32; 2]) -> bool {
        self.town.holds(p)
    }

    /// Whether the ground at chunk-local point `p` stood: in a listed town's extent, where its score reaches its bar
    /// unlifted, or where it is kept whole (a listed yard, [`Self::kept`]).
    pub fn stood(&self, p: [f32; 2]) -> bool {
        self.in_town(p) || self.keeps(p, 0.0) || self.in_yard(p) || self.kept.holds(p)
    }

    /// The scores raised by chunk `index`'s own footprints (`own`), where the host's stores hold none about it (a place
    /// Microsoft's footprints miss: OpenStreetMap's and the infill's). Smoothed as [`window_scores`] over the chunk's
    /// cells alone (each weighed over those it reaches; the ring takes its nearest cell's), so the chunk's edge thins
    /// as its middle does.
    pub fn with_own(mut self, index: [i32; 2], own: &Built) -> Self {
        if own.footprints() == 0 {
            return self;
        }
        let g = kernel();
        let reach = REACH as i64;
        let n = CELLS as i64;
        let share = |i: i64, j: i64| own.area[(j * n + i) as usize] / (CELL_M * CELL_M);
        let mut across = vec![0f32; CELLS * CELLS];
        for j in 0..n {
            for i in 0..n {
                let (mut s, mut weight) = (0.0, 0.0);
                for (k, wk) in g.iter().enumerate() {
                    let x = i + k as i64 - reach;
                    if (0..n).contains(&x) {
                        s += wk * share(x, j);
                        weight += wk;
                    }
                }
                across[(j * n + i) as usize] = s / weight;
            }
        }
        for k in 0..RING * RING {
            let i = (k as i64 % RING as i64 - 1).clamp(0, n - 1);
            let j = (k as i64 / RING as i64 - 1).clamp(0, n - 1);
            let (mut s, mut weight) = (0.0, 0.0);
            for (t, wk) in g.iter().enumerate() {
                let y = j + t as i64 - reach;
                if (0..n).contains(&y) {
                    s += wk * across[(y * n + i) as usize];
                    weight += wk;
                }
            }
            let mine = s / weight + NOISE * noise(theatre_cell(index, i as usize, j as usize));
            self.score[k] = self.score[k].max(mine);
        }
        self
    }

    /// The score at chunk-local point `p`: between the four nearest cells' centres.
    pub fn score_at(&self, p: [f32; 2]) -> f32 {
        ring_at(&self.score, p)
    }

    /// The bar at chunk-local point `p`: its cell's, or within a listed town's extent the town's.
    pub fn bar_at(&self, p: [f32; 2]) -> f32 {
        if self.in_town(p) {
            self.town_bar
        } else {
            self.bar[cell_of(p)]
        }
    }

    /// Whether a footprint whose middle is `p` stood, its score lifted by `lift` ([`lift`]).
    pub fn keeps(&self, p: [f32; 2], lift: f32) -> bool {
        self.score_at(p) + lift >= self.bar_at(p)
    }

    /// Whether `ring` is a building built after the period ([`LATER`]): it holds a listed point, or its middle lies
    /// within 35 m of one.
    pub fn later(&self, ring: &[[f32; 2]]) -> bool {
        self.later.iter().any(|p| geometry::contains(ring, *p))
            || self.later_at(geometry::centroid(ring))
    }

    /// Whether the building whose middle is `c` was built after the period: a listed point within 35 m of it (one of
    /// OSM's parts judged by its building's middle as a whole footprint is by its own, [`Thinning::later`]).
    pub fn later_at(&self, c: [f32; 2]) -> bool {
        self.later.iter().any(|p| geometry::distance(c, *p) < 35.0)
    }

    /// The most `ring`'s building stands (m): the cap, or what one of [`TALL`] on it stood.
    pub fn cap_for(&self, ring: &[[f32; 2]]) -> f32 {
        let c = geometry::centroid(ring);
        self.tall
            .iter()
            .filter(|(p, _)| geometry::contains(ring, *p) || geometry::distance(c, *p) < 25.0)
            .fold(self.cap_m, |cap, (_, h)| cap.max(*h))
    }
}

/// A cell of [`Waterfront`]'s distances to the water (m), and how far beyond the chunk they reach.
const WATER_CELL_M: f32 = 25.0;

/// A port's working waterfront about a chunk in a past period (`thinning`'s): the land of its docks, yards, stores and
/// naval bases that stands whole whatever its score. A point is on it where
///
/// - the period's port stood ([`Thinning::port`]) and the map holds a harbour about it (a ferry terminal or a harbour's
///   point, [`port_about`]);
/// - its land is industrial (railway yards and garages counted so, `Usage::of_landuse`) or military, or it stands on a
///   wharf (a pier mapped as an area);
/// - the water (the map's sea, docks and rivers; beyond the map's reach the sea floor below [`WET_M`]) lies within
///   [`WATERFRONT_REACH_M`] of it;
/// - it is not a container terminal's open yard (`infra::containers::YardTest`: a terminal's stacks stand on today's
///   fill and quays, no shed among them) nor the apron before one, and not land made or turned to industry after the
///   period ([`Thinning::not_yet`]).
///
/// OpenMapTiles carries no shipyard, dock or port land of its own (`industrial=shipyard`, `landuse=port` are folded into
/// industrial land or left out), so a shipyard and a post-war works on the water read alike but for their openness: the
/// lists ([`YARDS`], [`LATER_LAND`]) tell the theatres' ports apart.
pub struct Waterfront<'a> {
    tags: &'a MapTags,
    thinning: &'a Thinning,
    yard: Option<YardTest<'a>>,
    /// The distance to the water at each [`WATER_CELL_M`] cell's middle from [`WATERFRONT_REACH_M`] before the chunk's
    /// corner (`side` to a row); empty when the waterfront's rule does not run here.
    distance: Vec<f32>,
    side: usize,
}

impl<'a> Waterfront<'a> {
    /// The waterfront about a chunk whose map is `tags` and ground `ground` (chunk-local), thinned by `thinning`.
    pub fn new(
        tags: &'a MapTags,
        ground: &'a dyn Fn([f32; 2]) -> f32,
        thinning: &'a Thinning,
    ) -> Self {
        let mut out = Self {
            tags,
            thinning,
            yard: None,
            distance: Vec::new(),
            side: 0,
        };
        let side = CHUNK_M as f32;
        let over = |b: &[f32; 4]| b[0] <= side && b[2] >= 0.0 && b[1] <= side && b[3] >= 0.0;
        let land = tags
            .zones
            .iter()
            .any(|z| matches!(z.usage, Usage::Industrial | Usage::Military) && over(&z.bounds))
            || tags.piers.iter().any(|p| !p.narrow && over(&p.bounds));
        if !thinning.port || !port_about(tags) || !land {
            return out;
        }
        // The water on a grid over the chunk and the reach about it, then each cell's distance to the nearest
        // (a chamfer of 1 and √2 cells).
        let n = ((side + 2.0 * WATERFRONT_REACH_M) / WATER_CELL_M).ceil() as usize;
        let at = |i: usize| -WATERFRONT_REACH_M + (i as f32 + 0.5) * WATER_CELL_M;
        let mut d = vec![f32::INFINITY; n * n];
        let mut wet = false;
        for j in 0..n {
            for i in 0..n {
                let p = [at(i), at(j)];
                if tags.on_mapped_water(p) || ground(p) < -WET_M {
                    d[j * n + i] = 0.0;
                    wet = true;
                }
            }
        }
        if !wet {
            return out;
        }
        let (a, b) = (WATER_CELL_M, WATER_CELL_M * std::f32::consts::SQRT_2);
        for j in 0..n {
            for i in 0..n {
                let mut v = d[j * n + i];
                if i > 0 {
                    v = v.min(d[j * n + i - 1] + a);
                }
                if j > 0 {
                    v = v.min(d[(j - 1) * n + i] + a);
                    if i > 0 {
                        v = v.min(d[(j - 1) * n + i - 1] + b);
                    }
                    if i + 1 < n {
                        v = v.min(d[(j - 1) * n + i + 1] + b);
                    }
                }
                d[j * n + i] = v;
            }
        }
        for j in (0..n).rev() {
            for i in (0..n).rev() {
                let mut v = d[j * n + i];
                if i + 1 < n {
                    v = v.min(d[j * n + i + 1] + a);
                }
                if j + 1 < n {
                    v = v.min(d[(j + 1) * n + i] + a);
                    if i + 1 < n {
                        v = v.min(d[(j + 1) * n + i + 1] + b);
                    }
                    if i > 0 {
                        v = v.min(d[(j + 1) * n + i - 1] + b);
                    }
                }
                d[j * n + i] = v;
            }
        }
        out.yard = Some(YardTest::new(tags, ground));
        out.distance = d;
        out.side = n;
        out
    }

    /// Whether the rule runs about the chunk (a port that stood, with industrial or military land or a wharf by the
    /// water).
    pub fn active(&self) -> bool {
        !self.distance.is_empty()
    }

    /// The distance from chunk-local point `p` to the water (m, to a cell; infinite beyond the grid or without water).
    pub fn distance(&self, p: [f32; 2]) -> f32 {
        if self.distance.is_empty() {
            return f32::INFINITY;
        }
        let at = |v: f32| ((v + WATERFRONT_REACH_M) / WATER_CELL_M).floor();
        let (i, j) = (at(p[0]), at(p[1]));
        let n = self.side as f32;
        if i < 0.0 || j < 0.0 || i >= n || j >= n {
            return f32::INFINITY;
        }
        self.distance[j as usize * self.side + i as usize]
    }

    /// Whether the land at chunk-local point `p` (its use `usage`; on a wharf, `wharf`) is the working waterfront's.
    pub fn holds(&self, p: [f32; 2], usage: Usage, wharf: bool) -> bool {
        if !self.active() || !(wharf || matches!(usage, Usage::Industrial | Usage::Military)) {
            return false;
        }
        let distance = self.distance(p);
        if distance > WATERFRONT_REACH_M || self.thinning.not_yet(p) {
            return false;
        }
        let Some(yard) = &self.yard else {
            return true;
        };
        if yard.holds(p) {
            return false;
        }
        // A terminal's apron before its yard (the yard's test keeps its own apron off the water): its yard inland of it,
        // deeper than a patch of a shipyard's open ground.
        distance > APRON_M + WATER_CELL_M
            || !(0..8).any(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                [70.0f32, 110.0].iter().all(|r| {
                    let q = [p[0] + r * a.cos(), p[1] + r * a.sin()];
                    self.distance(q) > distance + 0.5 * r && yard.holds(q)
                })
            })
    }

    /// Whether a footprint whose setting is `setting` and middle `p` stands on the working waterfront.
    pub fn keeps(&self, p: [f32; 2], setting: &Setting) -> bool {
        let wharf = setting.pier.is_some_and(|deck| deck > 2.0);
        self.holds(p, setting.usage, wharf)
    }

    /// Whether the land at chunk-local point `p` is the working waterfront's (by the map's land use there and its
    /// wharves).
    pub fn land(&self, p: [f32; 2]) -> bool {
        if !self.active() {
            return false;
        }
        let wharf = self.tags.pier_at(p).is_some_and(|pier| !pier.narrow);
        self.holds(p, self.tags.usage_at(p), wharf)
    }
}

#[cfg(test)]
pub(crate) mod fixture {
    //! A chunk of every kind of building the period changes, for pinning what today's draws.

    use super::Thinning;
    use crate::buildings::mesh::ChunkInput;
    use crate::buildings::tags::{MapTags, Poi, PoiKind, Religion, Usage, WaterArea, Zone};
    use crate::buildings::{
        ChunkKey, ChunkMesh, Footprint, Kind, Lod, Period, Region, build_chunk,
    };

    pub fn rect(x: f32, z: f32, w: f32, d: f32) -> Vec<[f32; 2]> {
        vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]]
    }

    fn footprint(ring: Vec<[f32; 2]>, height: Option<f32>) -> Footprint {
        Footprint {
            ring,
            height,
            ..Footprint::default()
        }
    }

    /// Houses in rows, measured blocks and towers, a shop's box on commercial land, sheds on a port's quay, a church,
    /// a tank, and the infill's lots.
    pub fn footprints() -> (Vec<Footprint>, Vec<Footprint>) {
        let mut out = Vec::new();
        for j in 0..12 {
            for i in 0..12 {
                out.push(footprint(
                    rect(40.0 + 22.0 * i as f32, 40.0 + 24.0 * j as f32, 10.0, 12.0),
                    None,
                ));
            }
        }
        for (k, h) in [20.0, 50.0, 90.0, 150.0, 260.0, 8.0].iter().enumerate() {
            out.push(footprint(
                rect(360.0 + 55.0 * k as f32, 60.0, 40.0, 40.0),
                Some(*h),
            ));
        }
        for k in 0..6 {
            out.push(footprint(
                rect(360.0 + 60.0 * k as f32, 140.0, 35.0, 30.0),
                None,
            ));
        }
        // On commercial land: a shop's box and offices.
        out.push(footprint(rect(380.0, 420.0, 60.0, 45.0), None));
        out.push(footprint(rect(460.0, 420.0, 30.0, 25.0), Some(40.0)));
        out.push(footprint(rect(500.0, 420.0, 30.0, 25.0), None));
        // On the port's industrial land.
        for k in 0..5 {
            out.push(footprint(
                rect(620.0, 80.0 + 110.0 * k as f32, 70.0, 50.0),
                None,
            ));
        }
        // A church and a tank.
        out.push(footprint(rect(100.0, 380.0, 36.0, 16.0), None));
        out.push(footprint(
            crate::buildings::geometry::regular([200.0, 600.0], 15.0, 16, 0.0),
            None,
        ));
        let lots = (0..20)
            .map(|k| Footprint {
                ring: rect(
                    60.0 + 18.0 * (k % 10) as f32,
                    700.0 + 30.0 * (k / 10) as f32,
                    9.0,
                    11.0,
                ),
                height: Some(if k % 7 == 0 { 15.0 } else { 6.0 }),
                kind: Some(if k % 7 == 0 { Kind::Block } else { Kind::House }),
                colour: None,
                part: None,
            })
            .collect();
        (out, lots)
    }

    /// The map: commercial land, a port's industrial land along the sea east of x = 760, a church's point.
    pub fn tags() -> MapTags {
        MapTags {
            zones: vec![
                Zone::new(Usage::Commercial, rect(360.0, 400.0, 200.0, 100.0), vec![]),
                Zone::new(Usage::Industrial, rect(600.0, 0.0, 160.0, 900.0), vec![]),
            ],
            water: vec![WaterArea::new(rect(760.0, -200.0, 600.0, 1400.0), vec![])],
            pois: vec![Poi {
                at: [118.0, 388.0],
                kind: PoiKind::Worship(Religion::Christian),
            }],
            port: true,
            ..MapTags::default()
        }
    }

    pub fn ground(p: [f32; 2]) -> f32 {
        if p[0] > 760.0 { -14.0 } else { 3.0 }
    }

    /// The chunk's mesh in `region` at `lod` in `period`, thinned by `thinning`.
    pub fn mesh(
        region: Region,
        lod: Lod,
        period: Period,
        thinning: Option<&Thinning>,
    ) -> ChunkMesh {
        let (footprints, lots) = footprints();
        let tags = tags();
        build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [7, -3],
                lod,
            },
            footprints: &footprints,
            infill: &lots,
            tags: &tags,
            ground: &ground,
            region,
            period,
            thinning,
        })
    }

    /// What a mesh is pinned by: its buildings, vertices, indices and clutter, the vertices per kind, and its
    /// positions' sum (m).
    pub fn summary(mesh: &ChunkMesh) -> (u32, usize, usize, usize, [usize; 16], f64) {
        let mut kinds = [0usize; 16];
        let mut sum = 0.0f64;
        for v in &mesh.vertices {
            kinds[usize::from(v.style[3] & 15)] += 1;
            sum += f64::from(v.position[0]) + f64::from(v.position[1]) + f64::from(v.position[2]);
        }
        (
            mesh.buildings,
            mesh.vertices.len(),
            mesh.indices.len(),
            mesh.clutter.len(),
            kinds,
            sum,
        )
    }

    /// A terminal west of x = 700 by deep water with a marina's walkway (`tests/infra.rs`'s): its counts.
    pub fn infra(period: Period) -> crate::infra::InfraCounts {
        use crate::infra::{Airfield, ChunkInput, Marina, Power, Under, build_chunk};
        use glam::Vec2;
        let tags = MapTags {
            zones: vec![Zone::new(
                Usage::Industrial,
                rect(-400.0, 300.0, 1100.0, 1100.0),
                vec![],
            )],
            water: vec![WaterArea::new(rect(700.0, -200.0, 600.0, 1400.0), vec![])],
            pois: vec![Poi {
                at: [600.0, 700.0],
                kind: PoiKind::Harbour,
            }],
            ..MapTags::default()
        };
        let ground = |p: [f32; 2]| if p[0] > 700.0 { -15.0 } else { 3.0 };
        let marina = Marina {
            docks: vec![[Vec2::new(760.0, 800.0), Vec2::new(900.0, 800.0)]],
            marinas: vec![Vec2::new(800.0, 760.0)],
        };
        let water = |p: Vec2| p.x > 700.0;
        let under = |p: Vec2| if p.x > 700.0 { Under::Sea } else { Under::Dry };
        build_chunk(&ChunkInput {
            key: ChunkKey {
                index: [3, -2],
                lod: Lod::Full,
            },
            tags: &tags,
            ground: &ground,
            marina: &marina,
            water: &water,
            bridges: &[],
            bridge_outlines: &[],
            under: &under,
            airfield: &Airfield::default(),
            power: &Power::default(),
            period,
            bases: &[],
        })
        .counts
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{self, rect};
    use super::*;
    use crate::buildings::mesh::cranes;
    use crate::buildings::tags::{Quay, Zone};
    use crate::buildings::{Extra, Kind, Lod, Period, plan_building};

    /// A thinning that keeps everything (or, with `score` and `bar`, scores every cell alike against a bar).
    fn uniform(score: f32, bar: f32, cap_m: f32) -> Thinning {
        Thinning {
            score: vec![score; RING * RING],
            bar: vec![bar; CELLS * CELLS],
            cap_m,
            ..Thinning::default()
        }
    }

    #[test]
    fn a_later_building_drawn_by_its_parts_goes_whole() {
        // A listed later building as OSM's parts: a tower holding the listed point (400, 400), a nave whose middle is
        // 70 m off it; the building's middle 60 m off. Its whole footprint would have gone (it held the point).
        let ground = |_: [f32; 2]| 5.0;
        let part = |x: f32, w: f32, top: f32| Footprint {
            ring: vec![[x, 390.0], [x + w, 390.0], [x + w, 410.0], [x, 410.0]],
            height: Some(top),
            part: Some(crate::buildings::Part {
                base: 0.0,
                middle: [460.0, 400.0],
            }),
            ..Footprint::default()
        };
        let parts = vec![part(390.0, 20.0, 40.0), part(410.0, 120.0, 18.0)];
        let keep_all = uniform(0.9, f32::NEG_INFINITY, 100.0);
        let mesh = |t: &Thinning| {
            crate::buildings::build_chunk(&crate::buildings::mesh::ChunkInput {
                key: crate::buildings::ChunkKey {
                    index: [0, 0],
                    lod: Lod::Full,
                },
                footprints: &parts,
                infill: &[],
                tags: &MapTags::default(),
                ground: &ground,
                region: Region::Temperate,
                period: Period::Year(1942),
                thinning: Some(t),
            })
        };
        assert_eq!(mesh(&keep_all).buildings, 2);
        let mut later = keep_all.clone();
        later.later.push([400.0, 400.0]);
        assert!(!later.later_at([460.0, 400.0]));
        assert_eq!(mesh(&later).buildings, 0);
        // Listed within 35 m of the building's middle, though neither part holds it: gone too.
        let mut near = keep_all;
        near.later.push([455.0, 432.0]);
        assert!(!near.later(&parts[1].ring));
        assert_eq!(mesh(&near).buildings, 0);
    }

    #[test]
    fn todays_chunk_is_as_it_was() {
        // Pinned from master before the period's thinning (2026-10-10), again with the eaves, parapets, dormers and
        // grounds (2026-10-14), and again with a block drawn as one cut into its buildings (`parcels`, 2026-10-28; the
        // measured 20 m block of 40 by 40 m and the estimated blocks of 35 by 30 m, their walls against each other
        // drawn only above the lower): buildings, vertices, indices, clutter, the vertices per kind, and the
        // positions' sum.
        type Pin = (u32, usize, usize, usize, [usize; 16], f64);
        let box_kinds = [
            0, 3220, 100, 176, 80, 60, 52, 20, 120, 40, 0, 20, 0, 0, 0, 0,
        ];
        let pins: [(Region, Lod, Pin); 6] = [
            (
                Region::West,
                Lod::Full,
                (
                    197,
                    13216,
                    19380,
                    771,
                    [
                        0, 8534, 164, 192, 144, 258, 94, 36, 372, 140, 0, 50, 3232, 0, 0, 0,
                    ],
                    7991089.453788757,
                ),
            ),
            (
                Region::West,
                Lod::Box,
                (
                    189,
                    3840,
                    5760,
                    0,
                    [
                        0, 3220, 100, 128, 80, 60, 52, 20, 120, 40, 0, 20, 0, 0, 0, 0,
                    ],
                    1769663.3857803345,
                ),
            ),
            (
                Region::EastAsia,
                Lod::Full,
                (
                    200,
                    14756,
                    21552,
                    783,
                    [
                        0, 9982, 164, 272, 144, 258, 106, 36, 372, 140, 0, 50, 3232, 0, 0, 0,
                    ],
                    8657298.342308044,
                ),
            ),
            (
                Region::EastAsia,
                Lod::Box,
                (192, 3888, 5832, 0, box_kinds, 1795642.7094783783),
            ),
            (
                Region::British,
                Lod::Full,
                (
                    200,
                    15600,
                    22752,
                    922,
                    [
                        0, 10742, 164, 356, 144, 258, 106, 36, 372, 140, 0, 50, 3232, 0, 0, 0,
                    ],
                    8997762.68434143,
                ),
            ),
            (
                Region::British,
                Lod::Box,
                (192, 3888, 5832, 0, box_kinds, 1797041.0745048523),
            ),
        ];
        for (region, lod, pin) in pins {
            let mesh = fixture::mesh(region, lod, Period::Today, None);
            let got = fixture::summary(&mesh);
            assert_eq!(
                (got.0, got.1, got.2, got.3, got.4),
                (pin.0, pin.1, pin.2, pin.3, pin.4),
                "{region:?} {lod:?}"
            );
            assert!(
                (got.5 - pin.5).abs() < 0.5,
                "{region:?} {lod:?}: {} against {}",
                got.5,
                pin.5
            );
            // A thinning handed in with today's is not read.
            let thinned = fixture::mesh(region, lod, Period::Today, Some(&uniform(0.0, 1.0, 5.0)));
            assert_eq!(thinned.vertices, mesh.vertices, "{region:?} {lod:?}");
            assert_eq!(thinned.indices, mesh.indices);
            assert_eq!(thinned.clutter, mesh.clutter);
        }
        let infra = fixture::infra(Period::Today);
        assert_eq!((infra.containers, infra.boats), (14473, 39), "{infra:?}");
    }

    /// Footprints about a city's middle at theatre point `centre` (m) over chunks `0..n` each way: houses 10 × 12 m whose
    /// spacing grows from 16 m at the middle to 70 m 6 km out, none beyond.
    fn city(n: i32, centre: [f32; 2]) -> Vec<Vec<Vec<Footprint>>> {
        let mut chunks = vec![vec![Vec::new(); n as usize]; n as usize];
        let size = n as f32 * 1000.0;
        let mut z = 5.0f32;
        while z < size {
            let mut x = 5.0f32;
            let mut row = 16.0f32;
            while x < size {
                let d = ((x - centre[0]).powi(2) + (z - centre[1]).powi(2)).sqrt();
                let spacing = 16.0 + 54.0 * (d / 6000.0).min(1.0);
                row = row.min(spacing);
                if d < 6000.0 {
                    let (ci, cj) = ((x / 1000.0) as usize, (z / 1000.0) as usize);
                    let (lx, lz) = (x - ci as f32 * 1000.0, z - cj as f32 * 1000.0);
                    chunks[cj][ci].push(Footprint {
                        ring: rect(lx, lz, 10.0, 12.0),
                        ..Footprint::default()
                    });
                }
                x += spacing;
            }
            z += row.max(16.0);
        }
        chunks
    }

    #[test]
    fn a_cell_scores_the_same_from_any_window() {
        let chunks = city(6, [2500.0, 2500.0]);
        let built: Vec<Vec<Built>> = chunks
            .iter()
            .map(|row| row.iter().map(Built::of).collect())
            .collect();
        let at = |i: i32, j: i32| Some(&built[j as usize][i as usize]);
        // The whole 6 × 6 against each inner chunk's 3 × 3.
        let all: Vec<Option<&Built>> = (0..6)
            .flat_map(|j| (0..6).map(move |i| (i, j)))
            .map(|(i, j)| at(i, j))
            .collect();
        let whole = window_scores([0, 0], [6, 6], &all);
        for cj in 1..5 {
            for ci in 1..5 {
                let around: [Option<&Built>; 9] =
                    std::array::from_fn(|k| at(ci - 1 + k as i32 % 3, cj - 1 + k as i32 / 3));
                let mine = Thinning::new(&chunk_window([ci, cj], &around), vec![0.0; 100], 30.0);
                for k in 0..RING * RING {
                    let (x, y) = (
                        ci as usize * CELLS + k % RING - 1,
                        cj as usize * CELLS + k / RING - 1,
                    );
                    assert_eq!(
                        mine.score[k],
                        whole[y * 60 + x],
                        "chunk ({ci}, {cj}) cell {k}"
                    );
                }
                // Made again: the same.
                let again = Thinning::new(&chunk_window([ci, cj], &around), vec![0.0; 100], 30.0);
                assert_eq!(again, mine);
            }
        }
        // Across a chunk's edge the score is the same from either side.
        let around = |ci: i32, cj: i32| -> [Option<&Built>; 9] {
            std::array::from_fn(|k| at(ci - 1 + k as i32 % 3, cj - 1 + k as i32 / 3))
        };
        let a = Thinning::new(&chunk_window([2, 2], &around(2, 2)), vec![0.0; 100], 30.0);
        let b = Thinning::new(&chunk_window([3, 2], &around(3, 2)), vec![0.0; 100], 30.0);
        for z in [3.0, 250.0, 555.0, 990.0] {
            let (sa, sb) = (a.score_at([1000.0, z]), b.score_at([0.0, z]));
            assert!((sa - sb).abs() < 1e-5, "{sa} against {sb} at z {z}");
        }
    }

    #[test]
    fn the_kept_share_follows_the_history_and_keeps_the_core() {
        let n = 12;
        let centre = [6000.0, 6000.0];
        let chunks = city(n, centre);
        let built: Vec<Vec<Built>> = chunks
            .iter()
            .map(|row| row.iter().map(Built::of).collect())
            .collect();
        let all: Vec<Option<&Built>> = (0..n)
            .flat_map(|j| (0..n).map(move |i| (i, j)))
            .map(|(i, j)| Some(&built[j as usize][i as usize]))
            .collect();
        let whole = window_scores([0, 0], [n as usize, n as usize], &all);
        // One distribution over the inner 10 × 10 chunks (as a 5' cell's).
        let w = n as usize * CELLS;
        let mut distribution = Distribution::default();
        for cj in 1..n - 1 {
            for ci in 1..n - 1 {
                let b = &built[cj as usize][ci as usize];
                for k in 0..100 {
                    let (x, y) = (
                        ci as usize * CELLS + k % CELLS,
                        cj as usize * CELLS + k / CELLS,
                    );
                    distribution.add(whole[y * w + x], b.count[k]);
                }
            }
        }
        assert!(distribution.total > 20_000, "{}", distribution.total);
        for share in [0.15f32, 0.3, 0.5, 0.7] {
            let bar = distribution.bar(share);
            let (mut kept, mut all, mut core, mut core_kept) = (0, 0, 0, 0);
            let (mut edge, mut edge_kept) = (0, 0);
            for cj in 1..n - 1 {
                for ci in 1..n - 1 {
                    let around: [Option<&Built>; 9] = std::array::from_fn(|k| {
                        Some(
                            &built[(cj - 1 + k as i32 / 3) as usize]
                                [(ci - 1 + k as i32 % 3) as usize],
                        )
                    });
                    let thinning =
                        Thinning::new(&chunk_window([ci, cj], &around), vec![bar; 100], 30.0);
                    for f in &chunks[cj as usize][ci as usize] {
                        let c = geometry::centroid(&f.ring);
                        let keeps = thinning.keeps(c, 0.0);
                        all += 1;
                        kept += usize::from(keeps);
                        let t = [ci as f32 * 1000.0 + c[0], cj as f32 * 1000.0 + c[1]];
                        let d = ((t[0] - centre[0]).powi(2) + (t[1] - centre[1]).powi(2)).sqrt();
                        if d < 800.0 {
                            core += 1;
                            core_kept += usize::from(keeps);
                        }
                        if d > 5000.0 {
                            edge += 1;
                            edge_kept += usize::from(keeps);
                        }
                    }
                }
            }
            let got = kept as f32 / all as f32;
            assert!(
                (got - share).abs() < 0.06,
                "share {share}: kept {got} ({kept} of {all})"
            );
            assert_eq!(core_kept, core, "share {share}: the core stands whole");
            // The thin edge goes first.
            assert!(
                share > 0.5 || edge_kept * 100 <= edge,
                "share {share}: {edge_kept} of {edge} kept at the edge"
            );
        }
        // The region's floor clears a thin suburb whatever its 9 km share.
        let mut thin = Distribution::default();
        for k in 0..2000 {
            thin.add(0.08 + 0.08 * (k as f32 / 2000.0), 3);
        }
        assert!(bar(Some(&thin), 0.45, Region::West) >= core_floor(Region::West));
        assert!(thin.bar(0.45) < core_floor(Region::West));
        assert_eq!(bar(Some(&thin), 0.0, Region::British), f32::INFINITY);
        // Too few footprints for a distribution: the share alone.
        let few = Distribution::default();
        assert_eq!(bar(Some(&few), 0.4, Region::British), absolute_bar(0.4));
        assert_eq!(bar(None, 0.01, Region::Tropical), f32::INFINITY);
    }

    #[test]
    fn a_listed_towns_kept_is_taken_from_the_rest_of_its_cell() {
        let floor = core_floor(Region::British);
        // 1,000 footprints outside the towns scoring 0.2-0.6, and 600 in a town: 500 at 0.3, 100 under the floor.
        let mut plain = Distribution::default();
        for k in 0..1000 {
            plain.add(0.2 + 0.4 * k as f32 / 1000.0, 1);
        }
        let mut d = plain.clone();
        d.add_town(0.3, 500);
        d.add_town(0.05, 100);
        assert_eq!((d.total, d.town, d.footprints()), (1000, 600, 1600));
        assert_eq!(d.town_kept(floor), 500);
        // Half of the 1,600: the town keeps 500, the rest 300 of its 1,000.
        assert!((d.outside_share(0.5, floor) - 0.3).abs() < 1e-6);
        assert_eq!(
            bar(Some(&d), 0.5, Region::British),
            plain.bar(0.3).max(floor)
        );
        // The town keeps more than the cell's share: nothing else stands (the cell keeps the town's 500).
        assert_eq!(d.outside_share(0.25, floor), 0.0);
        assert_eq!(bar(Some(&d), 0.25, Region::British), f32::INFINITY);
        // Without a town, as before.
        assert_eq!(plain.outside_share(0.5, floor), 0.5);
        assert_eq!(
            bar(Some(&plain), 0.5, Region::British),
            plain.bar(0.5).max(floor)
        );
        // A whole share keeps everything outside.
        assert_eq!(d.outside_share(1.0, floor), 1.0);
    }

    #[test]
    fn a_cells_towns_are_paid_for_within_it_alone() {
        // 5' cell [100, 400] (lon 8.33-8.42, lat 33.33-33.42) keeps nothing outside its towns; its neighbours a bar of
        // 0.4, as it would be without its towns.
        let paid = [100, 400];
        let mut five = |cell: [i32; 2]| FiveBars {
            bar: if cell == paid { f32::INFINITY } else { 0.4 },
            plain: 0.4,
            footprints: 5000,
        };
        let step = 1.0 / HYDE_PER_DEGREE;
        let (lat0, lon0) = (400.0 * step, 100.0 * step);
        // Its middle keeps nothing; its neighbours are as they were, beside it too.
        assert_eq!(
            bar_about(lat0 + 0.5 * step, lon0 + 0.5 * step, &mut five),
            3.0
        );
        for (fy, fx) in [(-0.01, 0.5), (0.5, 1.01), (1.5, 0.5), (0.5, -0.4)] {
            let b = bar_about(lat0 + fy * step, lon0 + fx * step, &mut five);
            assert!((b - 0.4).abs() < 1e-6, "({fy}, {fx}): {b}");
        }
        // No step at its sides: the bar rises from its plain one inside them.
        let mut last = 0.4;
        for k in 0..=50 {
            let fx = 0.5 * k as f64 / 50.0;
            let b = bar_about(lat0 + 0.5 * step, lon0 + fx * step, &mut five);
            assert!(b >= last - 1e-6 && b - last < 0.4, "{fx}: {last} -> {b}");
            last = b;
        }
        assert!((last - 3.0).abs() < 1e-6);
        // A cell without towns blends as before.
        let mut plain = |_: [i32; 2]| FiveBars {
            bar: 0.3,
            plain: 0.3,
            footprints: 5000,
        };
        assert!((bar_about(lat0 + 0.02 * step, lon0, &mut plain) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn within_a_listed_town_the_ground_stands_and_buildings_keep_by_the_floor() {
        use crate::buildings::towns::{Shape, Town, Towns};
        // A chunk's square of 1 km on a chart of a kilometre a degree: a town over its middle, open ground at its heart.
        const TOWN: &[Town] = &[Town {
            name: "test",
            year: 1900,
            shapes: &[Shape::Polygon(&[
                (0.2, 0.2),
                (0.2, 0.8),
                (0.8, 0.8),
                (0.8, 0.2),
            ])],
            open: &[Shape::Circle(0.5, 0.5, 100.0)],
        }];
        let local = |lat: f64, lon: f64| [lon * 1000.0, lat * 1000.0];
        let towns = Towns::of(TOWN, 1942, &local, 1e6);
        let floor = core_floor(Region::British);
        // The ground scores 0.2 everywhere against a bar of 0.5: nothing stands but the town.
        let t = uniform(0.2, 0.5, 30.0).with_towns(&towns, [0.0, 0.0], floor);
        assert!(!t.town.is_empty());
        for (p, stands) in [
            ([300.0, 300.0], true),
            ([700.0, 650.0], true),
            ([500.0, 500.0], false),
            ([100.0, 500.0], false),
            ([900.0, 900.0], false),
        ] {
            assert_eq!(t.in_town(p), stands, "{p:?}");
            assert_eq!(t.stood(p), stands, "{p:?}");
            assert_eq!(t.keeps(p, 0.0), stands, "{p:?}");
        }
        // Within the town a building is kept by the floor: the thinnest ground goes, its ground still stands.
        let thin = uniform(0.05, 0.5, 30.0).with_towns(&towns, [0.0, 0.0], floor);
        assert!(!thin.keeps([300.0, 300.0], 0.0) && thin.stood([300.0, 300.0]));
        // A footprint whose middle lies just over a chunk's edge is the town's from either side.
        let west = towns.cells([-850.0, 0.0]);
        assert!(west.holds([1060.0, 500.0]) && !west.holds([990.0, 500.0]));
        // Before its year, and on a chunk it does not reach, none.
        assert!(Towns::of(TOWN, 1899, &local, 1e6).is_empty());
        assert!(towns.cells([5000.0, 0.0]).is_empty());
        assert!(Towns::of(TOWN, 1942, &local, 100.0).is_empty());
        // Its cells agree with the town's shape at their middles.
        for k in 0..KEPT_CELLS * KEPT_CELLS {
            let m = Kept::cell(k);
            let at = [f64::from(m[0]), f64::from(m[1])];
            assert_eq!(t.in_town(m), towns.holds(at), "{m:?}");
        }
        // Every building of a chunk wholly within a town is kept by the floor, whatever its cell's bar; as boxes too.
        let mut all = Kept::default();
        all.set_where(|_| true);
        for lod in [Lod::Full, Lod::Box] {
            let none = uniform(0.3, f32::INFINITY, 30.0);
            let town = Thinning {
                town: all.clone(),
                town_bar: floor,
                ..none.clone()
            };
            let today = fixture::mesh(Region::British, lod, Period::Today, None);
            let gone = fixture::mesh(Region::British, lod, Period::Year(1942), Some(&none));
            let stood = fixture::mesh(Region::British, lod, Period::Year(1942), Some(&town));
            let (n_today, n_gone, n_stood) = (
                fixture::summary(&today).0,
                fixture::summary(&gone).0,
                fixture::summary(&stood).0,
            );
            assert!(n_gone * 4 < n_today, "{lod:?}: {n_gone} of {n_today}");
            assert!(
                n_stood * 10 >= n_today * 9,
                "{lod:?}: {n_stood} of {n_today}"
            );
        }
    }

    #[test]
    fn a_chunk_without_microsofts_footprints_thins_by_its_own() {
        // Dense in the chunk's west half, thin in its east: the west kept, the east not.
        let mut own = Vec::new();
        for j in 0..40 {
            for i in 0..20 {
                own.push(Footprint {
                    ring: rect(5.0 + 22.0 * i as f32, 5.0 + 24.0 * j as f32, 12.0, 14.0),
                    ..Footprint::default()
                });
            }
            for i in 0..4 {
                own.push(Footprint {
                    ring: rect(560.0 + 100.0 * i as f32, 5.0 + 24.0 * j as f32, 10.0, 10.0),
                    ..Footprint::default()
                });
            }
        }
        let empty: [Option<&Built>; 9] = [None; 9];
        let thinning = Thinning::new(&chunk_window([4, 4], &empty), vec![0.25; 100], 12.0);
        assert!(
            !thinning.keeps([200.0, 500.0], 0.0),
            "no footprint: nothing"
        );
        let thinning = thinning.with_own([4, 4], &Built::of(&own));
        assert!(thinning.keeps([200.0, 500.0], 0.0));
        assert!(
            thinning.keeps([20.0, 20.0], 0.0),
            "the corner as the middle"
        );
        assert!(!thinning.keeps([900.0, 500.0], 0.0));
    }

    #[test]
    fn the_period_caps_heights_drops_modern_things_and_keeps_old_landmarks() {
        let year = Period::Year(1942);
        let keep_all = uniform(
            0.9,
            f32::NEG_INFINITY,
            height_cap(Region::EastAsia, 35.28, 139.67),
        );
        let today = fixture::mesh(Region::EastAsia, Lod::Full, Period::Today, None);
        let then = fixture::mesh(Region::EastAsia, Lod::Full, year, Some(&keep_all));
        let kind_of = |v: &crate::buildings::BuildingVertex| v.style[3] & 15;
        let tallest = |m: &crate::buildings::ChunkMesh, kinds: &[Kind]| {
            m.vertices
                .iter()
                .filter(|v| kinds.iter().any(|k| *k as u8 == kind_of(v)))
                .map(|v| v.position[1])
                .fold(0f32, f32::max)
        };
        let lived = [
            Kind::House,
            Kind::Terrace,
            Kind::Block,
            Kind::Tower,
            Kind::Office,
        ];
        assert!(tallest(&today, &lived) > 250.0);
        // 3 m of ground, 31 m, a pitched roof's rise.
        assert!(
            tallest(&then, &lived) <= 3.0 + 31.0 + 4.5,
            "{}",
            tallest(&then, &lived)
        );
        assert!(
            !then
                .vertices
                .iter()
                .any(|v| kind_of(v) == Kind::Tower as u8),
            "no towers in Japan's period"
        );
        // Offices without a curtain wall.
        assert!(
            then.vertices
                .iter()
                .filter(|v| kind_of(v) == Kind::Office as u8)
                .all(|v| v.style[3] & 0x80 == 0)
        );
        // The church, the tank and the factories' chimneys stand as they did.
        for kind in [Kind::Church, Kind::Tank, Kind::Chimney] {
            let count = |m: &crate::buildings::ChunkMesh| {
                m.vertices
                    .iter()
                    .filter(|v| kind_of(v) == kind as u8)
                    .count()
            };
            assert_eq!(count(&then), count(&today), "{kind:?}");
        }
        // A building that stood taller keeps its height.
        let mut tall = keep_all.clone();
        tall.tall.push(([600.0, 80.0], 150.0));
        let with_tall = fixture::mesh(Region::EastAsia, Lod::Full, year, Some(&tall));
        assert!(tallest(&with_tall, &lived) > 140.0);
        // Built later: gone.
        let mut later = keep_all.clone();
        later.later.push([118.0, 388.0]);
        let without = fixture::mesh(Region::EastAsia, Lod::Full, year, Some(&later));
        assert_eq!(without.buildings, then.buildings - 1);
        // A shop's big box stands only in a core.
        let thin = fixture::mesh(
            Region::EastAsia,
            Lod::Full,
            year,
            Some(&uniform(0.3, f32::NEG_INFINITY, 31.0)),
        );
        assert_eq!(thin.buildings, then.buildings - 1);
        // Nothing where nothing stood (but the quays' cranes).
        let none = fixture::mesh(
            Region::EastAsia,
            Lod::Full,
            year,
            Some(&uniform(0.0, f32::INFINITY, 31.0)),
        );
        assert!(
            none.vertices
                .iter()
                .all(|v| kind_of(v) == Kind::Crane as u8),
            "{}",
            none.buildings
        );
        // Containers gone from the terminal; the marina's boats fewer.
        let infra = fixture::infra(year);
        assert_eq!(infra.containers, 0);
        assert!(infra.boats < 39 / 3, "{infra:?}");
    }

    #[test]
    fn a_ports_gantries_are_jib_cranes_in_the_period() {
        let quay = |a: [f32; 2]| Quay {
            a,
            b: [a[0], a[1] + 600.0],
            out: [1.0, 0.0],
            land_m2: 300_000.0,
        };
        let gantry = |cranes: &[Extra]| {
            cranes
                .iter()
                .any(|c| matches!(c, Extra::Crane { gantry: true, .. }))
        };
        let mut found = 0;
        for k in 0..40 {
            let q = quay([600.0, 10.0 * k as f32]);
            let today = cranes(&q, [k, 2], Period::Today);
            let then = cranes(&q, [k, 2], Period::Year(1942));
            assert!(!then.is_empty());
            assert!(!gantry(&then));
            found += usize::from(gantry(&today));
        }
        assert!(found > 5, "today's ports have gantries: {found}");
    }

    #[test]
    fn estimated_storeys_are_fewer_and_no_tower_rises() {
        let flat = |_: [f32; 2]| 5.0f32;
        let setting = crate::buildings::Setting::default();
        let (mut towers_today, mut towers_then) = (0, 0);
        for k in 0..400 {
            let x = 10.0 + 2.3 * k as f32;
            let fp = Footprint {
                ring: rect(x, 100.0 + (k % 7) as f32, 32.0, 30.0),
                ..Footprint::default()
            };
            for (period, towers) in [
                (Period::Today, &mut towers_today),
                (Period::Year(1942), &mut towers_then),
            ] {
                let b = plan_building(&fp, [k, 0], 0.7, Region::Temperate, period, &setting, &flat)
                    .unwrap();
                *towers += usize::from(b.kind == Kind::Tower);
                if period.is_past() {
                    assert!(b.storeys <= 6, "{} storeys", b.storeys);
                }
            }
        }
        assert!(towers_today > 0);
        assert_eq!(towers_then, 0);
        // Japan's estimated blocks of four storeys at most; an infill lot's too.
        let lot = Footprint {
            ring: rect(0.0, 0.0, 30.0, 20.0),
            height: Some(20.0),
            kind: Some(Kind::Block),
            colour: None,
            part: None,
        };
        let b = plan_building(
            &lot,
            [0, 0],
            0.7,
            Region::EastAsia,
            Period::Year(1942),
            &setting,
            &flat,
        )
        .unwrap();
        assert!(b.storeys <= 4 && b.height <= 4.0 * 3.1 + 0.5, "{b:?}");
    }

    #[test]
    fn old_town_marks_lift_a_footprint() {
        let tags = fixture::tags();
        let ground = fixture::ground;
        let church = rect(100.0, 380.0, 36.0, 16.0);
        let setting = tags.setting(&church, &ground);
        assert!(lift(&tags, &setting, &church, &ground) >= LIFT_WORSHIP);
        let near = rect(150.0, 400.0, 10.0, 10.0);
        let setting = tags.setting(&near, &ground);
        let l = lift(&tags, &setting, &near, &ground);
        assert!(l > 0.0 && l < LIFT_NEAR_WORSHIP, "{l}");
        let quay = rect(700.0, 300.0, 40.0, 30.0);
        let setting = tags.setting(&quay, &ground);
        assert!((lift(&tags, &setting, &quay, &ground) - LIFT_HARBOUR).abs() < 1e-6);
        let mut railway = fixture::tags();
        let mut yard = Zone::new(Usage::Industrial, rect(200.0, 800.0, 200.0, 100.0), vec![]);
        yard.railway = true;
        railway.zones.push(yard);
        let shed = rect(250.0, 820.0, 30.0, 20.0);
        let setting = railway.setting(&shed, &ground);
        assert!(lift(&railway, &setting, &shed, &ground) >= LIFT_RAILWAY);
        railway.zones.push(Zone::new(
            Usage::Military,
            rect(0.0, 950.0, 200.0, 50.0),
            vec![],
        ));
        let barracks = rect(20.0, 960.0, 40.0, 12.0);
        let setting = railway.setting(&barracks, &ground);
        assert!(lift(&railway, &setting, &barracks, &ground) >= LIFT_MILITARY);
        let far = rect(300.0, 700.0, 10.0, 10.0);
        let setting = tags.setting(&far, &ground);
        assert_eq!(lift(&tags, &setting, &far, &ground), 0.0);
    }

    #[test]
    fn caps_shares_and_cells_by_place() {
        assert_eq!(height_cap(Region::EastAsia, 35.28, 139.67), 31.0);
        assert_eq!(height_cap(Region::West, 34.05, -118.25), 46.0);
        assert_eq!(height_cap(Region::American, 40.75, -73.98), 320.0);
        assert_eq!(height_cap(Region::American, 41.88, -87.63), 320.0);
        assert_eq!(height_cap(Region::American, 42.36, -71.06), 45.0);
        assert_eq!(height_cap(Region::British, 50.8, -1.09), 30.0);
        assert_eq!(height_cap(Region::Tropical, -9.43, 159.95), 12.0);
        assert_eq!(height_cap(Region::Iceland, 64.14, -21.93), 25.0);
        assert!(default_share(Region::Tropical, 1942) < default_share(Region::British, 1942));
        assert!(default_share(Region::West, 1914) < default_share(Region::West, 1942));
        assert!(default_share(Region::West, 1960) > default_share(Region::West, 1942));
        // Honiara's 5' cell, and its middle back in it.
        let cell = hyde_cell(-9.43, 159.95);
        let (lat, lon) = hyde_middle(cell);
        assert_eq!(hyde_cell(lat, lon), cell);
        assert!((lat + 9.43).abs() < 1.0 / 12.0 && (lon - 159.95).abs() < 1.0 / 12.0);
        assert_eq!(hyde_cell(10.0, 180.0), hyde_cell(10.0, -180.0));
        assert_eq!(cell_of([-3.0, 1003.0]), 90);
        assert_eq!(cell_of(cell_middle(37)), 37);
        // At a 5' cell's middle its bar alone; between cells blended, the empty ones barely counted.
        let about = hyde_about(lat, lon);
        assert_eq!(about[0].0, cell);
        assert!((about[0].1 - 1.0).abs() < 1e-3);
        let between = hyde_about(lat + 1.0 / 24.0, lon + 1.0 / 24.0);
        assert!(between.iter().all(|(_, w)| (w - 0.25).abs() < 1e-3));
        assert_eq!(between[0].0, cell);
        assert!((blend(&[(0.3, 0.5, 1000), (0.5, 0.5, 1000)]) - 0.4).abs() < 1e-6);
        assert!((blend(&[(0.3, 0.5, 1000), (f32::INFINITY, 0.5, 0)]) - 0.3).abs() < 0.06);
        assert_eq!(blend(&[(f32::INFINITY, 1.0, 0)]), 3.0);
        assert_eq!(hyde_about(10.06, 179.99)[1].0, hyde_cell(10.06, -179.99));
        // The history's shares sharpened, the Americas' the more; a cell the history left empty beside a town keeps
        // three quarters of the town's.
        let west = |s, a| period_share(s, a, Region::West, 1942);
        assert!((west(Some(0.45), None) - 0.45f32.powf(1.5)).abs() < 1e-6);
        assert!((west(Some(0.45), Some(0.3)) - 0.45f32.powf(1.5)).abs() < 1e-6);
        assert!((west(Some(0.1), Some(0.3)) - 0.1f32.powf(1.5)).abs() < 1e-6);
        assert!((west(Some(0.0), Some(0.3)) - 0.225).abs() < 1e-6);
        assert!(period_share(Some(0.6), None, Region::British, 1942) > 0.5);
        assert_eq!(period_share(Some(1.0), None, Region::EastAsia, 1942), 1.0);
        assert_eq!(
            period_share(None, None, Region::EastAsia, 1942),
            default_share(Region::EastAsia, 1942)
        );
        // San Pedro's cell: none in 1942 by the history, its neighbours' some.
        let (own, around) = history_about(hyde_cell(33.72, -118.29), 1942);
        assert!(own.is_some_and(|s| s < 0.05), "{own:?}");
        assert!(around.is_some_and(|s| s > 0.15), "{around:?}");
    }

    /// A port's chunk: the sea east of x = 700 (14 m deep, mapped), a shipyard's sheds on industrial land along it north
    /// of z = 450 (OpenStreetMap's outlines too), a container terminal's open industrial land south of z = 550 (amid the
    /// port's) with one gatehouse, houses on residential land west of x = 300; a ferry terminal about. Its footprints: the shipyard's
    /// sheds, the gatehouse, the houses.
    fn port() -> (MapTags, Vec<Footprint>, Vec<Footprint>, Vec<Footprint>) {
        let mut sheds = Vec::new();
        for j in 0..5 {
            for i in 0..4 {
                sheds.push(Footprint {
                    ring: rect(420.0 + 70.0 * i as f32, 30.0 + 80.0 * j as f32, 60.0, 45.0),
                    ..Footprint::default()
                });
            }
        }
        let gate = vec![Footprint {
            ring: rect(540.0, 790.0, 20.0, 15.0),
            ..Footprint::default()
        }];
        let houses: Vec<Footprint> = (0..40)
            .map(|k| Footprint {
                ring: rect(
                    30.0 + 60.0 * (k % 4) as f32,
                    40.0 + 24.0 * (k / 4) as f32,
                    10.0,
                    12.0,
                ),
                ..Footprint::default()
            })
            .collect();
        let tags = MapTags {
            zones: vec![
                Zone::new(Usage::Industrial, rect(400.0, 0.0, 300.0, 450.0), vec![]),
                Zone::new(
                    Usage::Industrial,
                    rect(-400.0, 550.0, 1100.0, 750.0),
                    vec![],
                ),
                Zone::new(Usage::Residential, rect(0.0, 0.0, 300.0, 500.0), vec![]),
            ],
            water: vec![super::super::tags::WaterArea::new(
                rect(700.0, -400.0, 800.0, 1800.0),
                vec![],
            )],
            outlines: sheds
                .iter()
                .map(|f| super::super::tags::Outline {
                    ring: f.ring.clone(),
                    height: None,
                    min_height: 0.0,
                    colour: None,
                    hide_3d: false,
                })
                .collect(),
            port: true,
            ..MapTags::default()
        };
        (tags, sheds, gate, houses)
    }

    fn port_ground(p: [f32; 2]) -> f32 {
        if p[0] > 700.0 { -14.0 } else { 3.0 }
    }

    /// The port's chunk in 1942 thinned by `t`: its buildings and the land kept.
    fn port_mesh(t: &Thinning) -> (crate::buildings::ChunkMesh, Vec<Footprint>) {
        port_mesh_at(t, Lod::Full)
    }

    /// The port's chunk in 1942 thinned by `t` at `lod`.
    fn port_mesh_at(t: &Thinning, lod: Lod) -> (crate::buildings::ChunkMesh, Vec<Footprint>) {
        let (tags, sheds, gate, houses) = port();
        let footprints: Vec<Footprint> = sheds.into_iter().chain(gate).chain(houses).collect();
        let mesh = crate::buildings::build_chunk(&crate::buildings::mesh::ChunkInput {
            key: crate::buildings::ChunkKey { index: [5, 5], lod },
            footprints: &footprints,
            infill: &[],
            tags: &tags,
            ground: &port_ground,
            region: Region::West,
            period: Period::Year(1942),
            thinning: Some(t),
        });
        (mesh, footprints)
    }

    #[test]
    fn a_ports_working_waterfront_stands_whole_but_its_terminal() {
        // Nothing scores to its bar: only what the waterfront keeps stands.
        let none = Thinning {
            port: true,
            ..uniform(0.0, 1.0, 46.0)
        };
        let (mesh, _) = port_mesh(&none);
        // The quays' cranes stand whatever stood (`fixture`'s): as many where the port's town did not.
        let (bare, _) = port_mesh(&uniform(0.0, 1.0, 46.0));
        let cranes = bare.buildings;
        assert!(cranes > 0 && bare.cranes == cranes && mesh.cranes == cranes);
        // The shipyard's 20 sheds; not the terminal's gatehouse, nor the houses.
        assert_eq!(mesh.buildings, cranes + 20);
        let kept = mesh.kept.as_ref().expect("the land kept in a past period");
        // The yard's land kept, its sheds' and between them; not the terminal's open yard, the houses' or the sea's.
        for p in [
            [450.0, 50.0],
            [600.0, 300.0],
            [690.0, 440.0],
            [550.0, 120.0],
        ] {
            assert!(kept.holds(p), "{p:?}");
            let answered = Thinning {
                kept: kept.clone(),
                ..none.clone()
            };
            assert!(answered.stood(p) && !none.stood(p), "{p:?}");
        }
        for p in [
            [550.0, 800.0],
            [690.0, 800.0],
            [400.0, 900.0],
            [100.0, 100.0],
            [850.0, 300.0],
        ] {
            assert!(!kept.holds(p), "{p:?}");
        }
        // The terminal's land within reach of the water: hardly any kept (the yard's test leaves an edge or two where
        // the land about is not the port's).
        let terminal: Vec<[f32; 2]> = (0..KEPT_CELLS * KEPT_CELLS)
            .map(Kept::cell)
            .filter(|p| p[0] > 360.0 && p[0] < 700.0 && p[1] > 550.0)
            .collect();
        let held = terminal.iter().filter(|p| kept.holds(**p)).count();
        assert!(held * 5 < terminal.len(), "{held} of {}", terminal.len());
        // Land beyond the reach of the water is not the waterfront's.
        let tags = port().0;
        let w = Waterfront::new(&tags, &port_ground, &none);
        assert!(w.active());
        assert!(w.distance([690.0, 300.0]) <= 25.0 && w.distance([400.0, 300.0]) >= 275.0);
        assert!(w.land([600.0, 300.0]) && !w.land([250.0, 300.0]));
        assert!(!w.land([300.0, 300.0]), "residential land");
        // With no water mapped, only the sea floor deeper than `WET_M` is water: not a polder's low land.
        let unmapped = MapTags {
            water: Vec::new(),
            ..tags.clone()
        };
        let polder = |p: [f32; 2]| if p[0] > 700.0 { -1.0 } else { 3.0 };
        assert!(!Waterfront::new(&unmapped, &polder, &none).active());
        assert!(Waterfront::new(&unmapped, &port_ground, &none).active());
        // Where the port's town did not stand, nothing: the houses' and the yard's alike thinned.
        assert!(bare.kept.as_ref().is_some_and(Kept::is_empty));
        // Land made or turned to industry after the period (`LATER_LAND`): thinned as any other.
        let later = Thinning {
            not_yet: vec![([550.0, 200.0], 400.0)],
            ..none.clone()
        };
        let (mesh, _) = port_mesh(&later);
        assert_eq!(mesh.buildings, cranes);
        let kept = mesh.kept.as_ref().expect("kept land");
        assert!(
            (0..KEPT_CELLS * KEPT_CELLS)
                .all(|k| !kept.holds(Kept::cell(k)) || !later.not_yet(Kept::cell(k)))
        );
        // Today's: every building, nothing kept for the ground.
        let (tags, sheds, gate, houses) = port();
        let footprints: Vec<Footprint> = sheds.into_iter().chain(gate).chain(houses).collect();
        let today = crate::buildings::build_chunk(&crate::buildings::mesh::ChunkInput {
            key: crate::buildings::ChunkKey {
                index: [5, 5],
                lod: Lod::Full,
            },
            footprints: &footprints,
            infill: &[],
            tags: &tags,
            ground: &port_ground,
            region: Region::West,
            period: Period::Today,
            thinning: Some(&none),
        });
        assert!(today.buildings >= 61, "{}", today.buildings);
        assert!(today.kept.is_none());
    }

    #[test]
    fn a_listed_yard_stands_from_its_year_and_not_before() {
        assert!(!YARDS.is_empty() && !LATER_LAND.is_empty());
        for list in [YARDS, LATER_LAND] {
            for place in list {
                assert!(!place.circles.is_empty(), "{}", place.name);
                assert!((1400..=2030).contains(&place.year), "{}", place.name);
                for &(lat, lon, r) in place.circles {
                    assert!(lat.abs() < 80.0 && lon.abs() <= 180.0, "{}", place.name);
                    assert!((50.0..=3000.0).contains(&r), "{} {r}", place.name);
                }
            }
        }
        // The first listed yard's first circle about the middle of a chunk (1 m a 1e-5 degree, near enough).
        let yard = YARDS[0];
        let (lat0, lon0, r) = yard.circles[0];
        let local = |lat: f64, lon: f64| {
            [
                500.0 + ((lon - lon0) * 1e5) as f32,
                500.0 - ((lat - lat0) * 1e5) as f32,
            ]
        };
        let none = uniform(0.0, 1.0, 46.0);
        let then = none.clone().with_listed(yard.year, &local);
        assert!(then.in_yard([500.0, 500.0]) && !then.not_yet([500.0, 500.0]));
        assert!(then.stood([500.0, 500.0]) && !then.stood([500.0, 500.0 + r + 30.0]));
        let before = none.clone().with_listed(yard.year - 1, &local);
        assert!(!before.in_yard([500.0, 500.0]) && before.not_yet([500.0, 500.0]));
        assert!(!before.stood([500.0, 500.0]));
        // A chunk's houses within the yard: all of them from its year, none before (nothing scores to its bar).
        let in_yard = |t: &Thinning| {
            let mut t = t.clone();
            t.yards.retain(|(p, _)| *p == [500.0, 500.0]);
            t.yards = t.yards.iter().map(|(p, _)| (*p, 2000.0)).collect();
            t.not_yet = t.not_yet.iter().map(|(p, _)| (*p, 2000.0)).collect();
            fixture::mesh(Region::EastAsia, Lod::Full, Period::Year(1942), Some(&t))
        };
        // Against the period's chunk kept whole (its blocks cut into their buildings as the period's are).
        let all = fixture::mesh(
            Region::EastAsia,
            Lod::Full,
            Period::Year(1942),
            Some(&uniform(1.0, 0.0, 46.0)),
        );
        let stood = in_yard(&then);
        assert_eq!(stood.buildings, all.buildings);
        let kept = stood.kept.as_ref().expect("kept land");
        assert_eq!(kept.count(), KEPT_CELLS * KEPT_CELLS);
        let gone = in_yard(&before);
        assert!(
            gone.buildings < 10,
            "{} before the yard's year",
            gone.buildings
        );
        // Land made after the period: not yet in its year's eve, nothing from its year.
        let land = LATER_LAND[0];
        let (lat1, lon1, _) = land.circles[0];
        let local = |lat: f64, lon: f64| {
            [
                500.0 + ((lon - lon1) * 1e5) as f32,
                500.0 - ((lat - lat1) * 1e5) as f32,
            ]
        };
        assert!(
            none.clone()
                .with_listed(land.year - 1, &local)
                .not_yet([500.0, 500.0])
        );
        let made = none.clone().with_listed(land.year, &local);
        assert!(!made.not_yet([500.0, 500.0]) && !made.in_yard([500.0, 500.0]));
    }

    #[test]
    fn both_levels_of_detail_keep_the_same() {
        // What a working waterfront, a listed yard and the lifts (a church, a quay's sheds; nothing else reaches the
        // bar) keep: the same buildings and land whole and as boxes, given the same map (the host reads one map at
        // both levels in a past period); only a whole chunk draws the quays' cranes.
        let waterfront = Thinning {
            port: true,
            ..uniform(0.0, 1.0, 46.0)
        };
        let yard = Thinning {
            yards: vec![([300.0, 300.0], 250.0)],
            ..uniform(0.0, 1.0, 46.0)
        };
        let lifts = uniform(0.35, 0.45, 46.0);
        for (name, t) in [
            ("waterfront", &waterfront),
            ("yard", &yard),
            ("lifts", &lifts),
        ] {
            let pairs = [
                (port_mesh_at(t, Lod::Full).0, port_mesh_at(t, Lod::Box).0),
                (
                    fixture::mesh(Region::West, Lod::Full, Period::Year(1942), Some(t)),
                    fixture::mesh(Region::West, Lod::Box, Period::Year(1942), Some(t)),
                ),
            ];
            for (k, (whole, boxes)) in pairs.iter().enumerate() {
                assert_eq!(boxes.cranes, 0);
                assert_eq!(
                    whole.buildings - whole.cranes,
                    boxes.buildings,
                    "{name} {k}"
                );
                assert_eq!(whole.kept, boxes.kept, "{name} {k}");
            }
            assert!(
                pairs
                    .iter()
                    .any(|(w, _)| w.kept.as_ref().is_some_and(|k| !k.is_empty())),
                "{name}"
            );
        }
        let kept = |t: &Thinning| {
            fixture::mesh(Region::West, Lod::Box, Period::Year(1942), Some(t)).buildings
        };
        assert!(kept(&lifts) > kept(&uniform(0.0, 1.0, 46.0)));
    }

    #[test]
    fn kept_land_is_held_by_its_cells() {
        let mut kept = Kept::default();
        assert!(kept.is_empty() && !kept.holds([10.0, 10.0]));
        kept.set_ring(&rect(100.0, 100.0, 40.0, 30.0), 20.0);
        assert!(
            kept.holds([120.0, 115.0]) && kept.holds([90.0, 90.0]) && kept.holds([145.0, 135.0])
        );
        assert!(!kept.holds([60.0, 115.0]) && !kept.holds([200.0, 115.0]));
        // A ring over the chunk's edge keeps the cells beyond it too, to the border, for the neighbour's ground.
        assert!(!kept.spills());
        kept.set_ring(&rect(980.0, -30.0, 40.0, 60.0), 0.0);
        assert!(kept.holds([990.0, 10.0]) && kept.holds([1010.0, -20.0]) && kept.spills());
        assert!(!kept.holds([1010.0, -40.0]) && !kept.holds([1200.0, 10.0]));
        let n = kept.count();
        kept.set_where(|p| p[0] < 50.0);
        assert_eq!(kept.count(), n + 2 * KEPT_CELLS);
    }

    #[test]
    fn a_port_stood_where_its_town_did() {
        let region = |_: f64, _: f64| Region::West;
        // San Pedro in 1942 (its cell three quarters of its neighbours'); Honiara in 1942 (a few huts).
        assert!(port_stood(33.745, -118.268, 1942, &region));
        assert!(!port_stood(-9.43, 159.95, 1942, &|_, _| Region::Tropical));
        // Yokosuka, where the history has none: its region's share.
        assert!(port_stood(35.29, 139.66, 1942, &|_, _| Region::EastAsia));
    }
}
