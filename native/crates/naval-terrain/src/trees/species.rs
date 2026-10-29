//! The trees' species: a procedural 3D model of each (trunk and limbs as capsules, the crown as clumps of leaves in
//! lobes), which `shaders/tree_bake.wgsl` ray-casts once into an octahedral impostor atlas the cards draw from
//! (native/docs/terrain/trees.md), and the climate a theatre's land stands in ([`Biome`]), from which the placement
//! chooses each tree's species with the land cover, the height, the slope and the coast.
//!
//! Every model is built in metres as the tree stands, then normalised: its foot at y = 0, its top at y = 1, and its
//! radius ([`Model::radius`]) the widest reach of its crown from the trunk's axis over its height. The placement scales
//! a model by the tree's height (and a width of its own), turns it about its axis, and tints it.

use std::sync::OnceLock;

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

use naval_conditions::{Climate, LandStyle};

use crate::earth::{DrySeason, Koppen, KoppenGroup, Warmth};

/// One model's species, in the atlas's layer order (the `SPECIES_*` constants in `shaders/tree_place.wgsl` match).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Species {
    /// A rainforest canopy tree: a broad, uneven crown of big lobes.
    Rainforest,
    /// A rainforest emergent: a tall bare bole under a flat, spreading crown.
    Emergent,
    CoconutPalm,
    /// Mangrove: a low dense dome on arching prop roots.
    Mangrove,
    /// Oak or beech: a broad round crown.
    Oak,
    /// Ash, elm or lime: taller and narrower than the oak.
    Ash,
    Birch,
    /// The oak in winter: limbs and a haze of twigs.
    BareOak,
    /// The birch in winter.
    BareBirch,
    /// Spruce or fir: a narrow cone of drooping whorls.
    Spruce,
    /// Scots pine: a tall bare bole, a flat uneven crown of needle pads.
    ScotsPine,
    /// Stone (umbrella) pine.
    StonePine,
    /// Italian cypress: a dark column.
    Cypress,
    /// Olive (and holm oak, tinted): low, crooked, a grey-green open crown.
    Olive,
    /// Coast live oak (and the evergreen oaks of the subtropics): a low wide dome on sweeping limbs.
    LiveOak,
    /// Monterey pine (and the coastal pines of the subtropics): a dense, uneven dome-cone.
    MontereyPine,
    /// Monterey cypress: flat-topped, in wind-shorn tiers.
    MontereyCypress,
    /// Eucalyptus (blue gum): tall, pale-barked, an open crown of separate clumps.
    Eucalyptus,
    /// Coast redwood: very tall, a narrow cone on a thick red trunk.
    Redwood,
    /// Acacia: a flat-topped thorn tree of the dry country.
    Acacia,
    /// A shrub: chaparral, maquis, scrub (a dense rounded bush, wider than tall).
    Shrub,
    /// Garrigue and heath: a low grey-green cushion.
    Garrigue,
    /// A tuft of tall grass (kunai, tussock, dry bunchgrass).
    GrassTuft,
    /// Mexican fan palm (Washingtonia robusta): a very tall slender trunk under a small round head of fans and a short
    /// skirt of dead ones, the street palm of Southern California.
    FanPalm,
    /// Canary Island date palm: a stout trunk under a great round crown of arching fronds.
    DatePalm,
    /// California sycamore: leaning pale trunks, an open, irregular crown of separate masses.
    Sycamore,
    /// Chaparral: a thicket of evergreen shrubs (chamise, ceanothus, manzanita), low and wide, one model a patch, so the
    /// placement's 20 m slots close a hillside's shrub cover.
    Chaparral,
    /// Sugi (Cryptomeria) in a plantation: a block of narrow dark cones of one age about a model (hinoki, tinted), so
    /// one slot stands a stand's worth of them, as dense as a plantation is.
    Sugi,
    /// Evergreen broadleaf (Castanopsis, Machilus, camphor; a town's camphor, ficus or magnolia): a dense, lumpy dome
    /// of tight masses.
    Laurel,
    /// Japanese black pine (and red pine): a crooked, leaning trunk under flat, cloud-like pads.
    BlackPine,
    /// Bamboo: a grove of tall culms with feathery, arching tops.
    Bamboo,
    /// Date palm (Phoenix dactylifera), the Nile's, the oases' and the Maghreb's: a tall, slender, straight trunk rough
    /// with old leaf bases under an open crown of stiff grey-green fronds, the young ones standing steeply up.
    OasisPalm,
}

impl Species {
    pub const ALL: [Species; 32] = [
        Species::Rainforest,
        Species::Emergent,
        Species::CoconutPalm,
        Species::Mangrove,
        Species::Oak,
        Species::Ash,
        Species::Birch,
        Species::BareOak,
        Species::BareBirch,
        Species::Spruce,
        Species::ScotsPine,
        Species::StonePine,
        Species::Cypress,
        Species::Olive,
        Species::LiveOak,
        Species::MontereyPine,
        Species::MontereyCypress,
        Species::Eucalyptus,
        Species::Redwood,
        Species::Acacia,
        Species::Shrub,
        Species::Garrigue,
        Species::GrassTuft,
        Species::FanPalm,
        Species::DatePalm,
        Species::Sycamore,
        Species::Chaparral,
        Species::Sugi,
        Species::Laurel,
        Species::BlackPine,
        Species::Bamboo,
        Species::OasisPalm,
    ];

    /// The `SPECIES_*` name the shaders use.
    pub fn constant(self) -> &'static str {
        match self {
            Species::Rainforest => "SPECIES_RAINFOREST",
            Species::Emergent => "SPECIES_EMERGENT",
            Species::CoconutPalm => "SPECIES_PALM",
            Species::Mangrove => "SPECIES_MANGROVE",
            Species::Oak => "SPECIES_OAK",
            Species::Ash => "SPECIES_ASH",
            Species::Birch => "SPECIES_BIRCH",
            Species::BareOak => "SPECIES_BARE_OAK",
            Species::BareBirch => "SPECIES_BARE_BIRCH",
            Species::Spruce => "SPECIES_SPRUCE",
            Species::ScotsPine => "SPECIES_SCOTS_PINE",
            Species::StonePine => "SPECIES_STONE_PINE",
            Species::Cypress => "SPECIES_CYPRESS",
            Species::Olive => "SPECIES_OLIVE",
            Species::LiveOak => "SPECIES_LIVE_OAK",
            Species::MontereyPine => "SPECIES_MONTEREY_PINE",
            Species::MontereyCypress => "SPECIES_MONTEREY_CYPRESS",
            Species::Eucalyptus => "SPECIES_EUCALYPTUS",
            Species::Redwood => "SPECIES_REDWOOD",
            Species::Acacia => "SPECIES_ACACIA",
            Species::Shrub => "SPECIES_SHRUB",
            Species::Garrigue => "SPECIES_GARRIGUE",
            Species::GrassTuft => "SPECIES_GRASS",
            Species::FanPalm => "SPECIES_FAN_PALM",
            Species::DatePalm => "SPECIES_DATE_PALM",
            Species::Sycamore => "SPECIES_SYCAMORE",
            Species::Chaparral => "SPECIES_CHAPARRAL",
            Species::Sugi => "SPECIES_SUGI",
            Species::Laurel => "SPECIES_LAUREL",
            Species::BlackPine => "SPECIES_BLACK_PINE",
            Species::Bamboo => "SPECIES_BAMBOO",
            Species::OasisPalm => "SPECIES_OASIS_PALM",
        }
    }

    /// Its height (m) where it grows well, the middle of the placement's range; the battle maps keep their styles'.
    pub fn height_m(self) -> f32 {
        match self {
            Species::Rainforest => 30.0,
            Species::Emergent => 44.0,
            Species::CoconutPalm => 23.0,
            Species::Mangrove => 8.0,
            Species::Oak => 19.0,
            Species::Ash => 22.0,
            Species::Birch => 15.0,
            Species::BareOak => 19.0,
            Species::BareBirch => 15.0,
            Species::Spruce => 24.0,
            Species::ScotsPine => 20.0,
            Species::StonePine => 15.0,
            Species::Cypress => 17.0,
            Species::Olive => 7.0,
            Species::LiveOak => 13.0,
            Species::MontereyPine => 24.0,
            Species::MontereyCypress => 14.0,
            Species::Eucalyptus => 34.0,
            Species::Redwood => 55.0,
            Species::Acacia => 8.0,
            Species::Shrub => 2.6,
            Species::Garrigue => 0.8,
            Species::GrassTuft => 0.7,
            Species::FanPalm => 24.0,
            Species::DatePalm => 14.0,
            Species::Sycamore => 18.0,
            Species::Chaparral => 3.0,
            Species::Sugi => 26.0,
            Species::Laurel => 16.0,
            Species::BlackPine => 15.0,
            Species::Bamboo => 15.0,
            Species::OasisPalm => 17.0,
        }
    }

    /// Whether it sheds its leaves, and its winter self (the placement turns it bare by the climate's leaves).
    pub fn bare(self) -> Option<Species> {
        match self {
            Species::Oak | Species::Ash => Some(Species::BareOak),
            Species::Birch => Some(Species::BareBirch),
            Species::Sycamore => Some(Species::BareOak),
            _ => None,
        }
    }
}

/// The climate and flora a theatre's land stands in: the climate zone from the Köppen-Geiger class of the land there
/// ([`crate::earth::koppen`]), the flora of a region where one of the hand-drawn boxes takes it (Japan, Britain, the
/// American West); the placement chooses each tree's species from it (`tree_place.wgsl`). The battle maps' lands keep
/// their styles' own trees ([`Biome::Authored`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Biome {
    /// A battle map: the style's trees as authored (rainforest and palms, birch and spruce, oak and pine).
    Authored = 0,
    /// The wet tropics (Af, Am): rainforest, emergents, palms, mangroves on the shore.
    Tropical = 1,
    /// Deserts and steppe (BW, BS): acacia, palms by the sea, scrub.
    Arid = 2,
    /// The Mediterranean (Cs, the Old World): stone pine, cypress, olive, holm oak; maquis and garrigue.
    Mediterranean = 3,
    /// California and central Chile (Csb with fog): coast live oak, Monterey pine and cypress, eucalyptus, redwoods.
    California = 4,
    /// Humid subtropics (Cfa: southern Japan, the China coast, the south-eastern United States): evergreen oaks,
    /// coastal pines, deciduous oaks, a few palms.
    Subtropical = 5,
    /// Temperate (Cfb, Dfb): oak, ash, birch, pine, spruce in the hills.
    Temperate = 6,
    /// Boreal (Dfc): spruce, pine, birch.
    Boreal = 7,
    /// Subarctic coasts and the tundra's edge (Iceland): low birch, a few spruce.
    Subarctic = 8,
    /// Japan, the Ryukyus and Taiwan: sugi and hinoki plantations on the slopes, the evergreen broadleaf (laurel)
    /// forest of Castanopsis, Machilus and camphor below the beech and fir of the mountains and the north, black pine
    /// on the coast, bamboo about the villages. Korea is not in it: its forests are red pine and deciduous oak, the
    /// temperate biome's.
    Japan = 9,
    /// Great Britain, Ireland and their islands: the fields' and hedgerows' oaks and ashes, oak, ash and beech woods,
    /// Sitka spruce plantations in compartments where the photograph shows a wood dark, the Highlands' birch and Scots
    /// pine, and the windswept Northern Isles and Outer Hebrides all but bare.
    Britain = 10,
    /// The continent's north-west and middle, from Brittany to the Baltic and the Black Sea (and Spain's Atlantic north):
    /// beech and oak woods, Norway spruce and Scots pine plantations where the photograph shows a wood dark, pines on the
    /// North Sea's and the Baltic's dunes, the fields' oaks and ashes; eucalyptus and maritime pine plantations in
    /// Galicia and northern Portugal.
    NorthernEurope = 11,
    /// The Maghreb and Libya to the Gulf of Sirte and Cyrenaica: olive and citrus groves in rows, eucalyptus windbreaks
    /// and plantations, Aleppo pine and cork oak on the hills, date and fan palms in the towns and date palms in the
    /// oases; the desert bare.
    NorthAfrica = 12,
    /// Egypt and Libya's Marmarica (Tobruk, Bardia): date palms along the Nile and its canals and about the villages,
    /// casuarina and eucalyptus windbreaks, citrus and mango groves, the towns' ficus, palms and casuarinas; the desert
    /// bare.
    Egypt = 13,
    /// The north-eastern United States and Canada's Maritimes: maple, oak, beech and birch woods, white pine stands and
    /// hemlock, spruce and fir in Maine and the north, pitch pine on the sandy coast.
    Northeast = 14,
    /// The south-eastern United States' coastal plain, Hampton Roads to Florida and the Gulf: loblolly and slash pine
    /// plantations in rows, hardwoods in the bottomlands, live oaks and cabbage palms on the coast and in the towns,
    /// South Florida's mangroves and coconut palms.
    Southeast = 15,
    /// Hawaii: kiawe on the dry leeward lowlands, the wet windward and upland forest, monkeypods, banyans and palms in
    /// the towns, coconut palms and ironwoods by the shore.
    Hawaii = 16,
    /// The interior West of North America (the Great Basin, the Colorado Plateau, the Rockies; the Sierra Nevada's and
    /// the Cascades' high and eastern sides; the Sierra Madre's north), in belts by height: sagebrush and desert scrub
    /// low, pinyon and juniper woodland about 1,500-2,200 m, ponderosa pine about 2,000-2,700 m, Douglas fir, aspen and
    /// spruce-fir above (at 36 N, lower to the north), cottonwoods along the rivers. Its hot deserts (BWh, BSh: the
    /// Mojave's and the Sonoran's lowlands) are the arid biome's.
    InteriorWest = 17,
}

/// The tallest a tree of the land's species stands at the middle of its range (m): a battle map's style's (the tropical
/// style's 30 m rainforest), a theatre's tallest species (55 m redwoods).
pub fn tallest_m(biome: Biome) -> f32 {
    match biome {
        Biome::Authored => 30.0,
        _ => Species::ALL
            .iter()
            .map(|s| s.height_m())
            .fold(0.0, f32::max),
    }
}

/// The tallest palm's height (m): a palm leans far (`trees::TREE_PALM_LEAN_MOST`).
pub fn tallest_palm_m() -> f32 {
    [
        Species::CoconutPalm,
        Species::FanPalm,
        Species::DatePalm,
        Species::OasisPalm,
    ]
    .iter()
    .map(|s| s.height_m())
    .fold(0.0, f32::max)
}

/// A box of latitude and longitude.
fn within(lat: f32, lon: f32, lat0: f32, lat1: f32, lon0: f32, lon1: f32) -> bool {
    lat >= lat0 && lat <= lat1 && lon >= lon0 && lon <= lon1
}

/// Japan (Kyushu and the Goto islands, Tsushima, Shikoku and Honshu, Hokkaido), the Ryukyus and Taiwan, and none of
/// Korea, Jeju or the Russian coast: the boxes keep west of Korea's south and east coasts (Busan 129.0 E, Ulsan and
/// Pohang 129.3-129.4 E) and south of Primorye.
fn japanese(lat: f32, lon: f32) -> bool {
    // Kyushu, the Goto islands, Shikoku's and Honshu's south, the Izu islands (Jeju, at 126.5 E, stays out).
    within(lat, lon, 30.0, 34.0, 128.0, 146.0)
        // Tsushima.
        || within(lat, lon, 34.0, 34.8, 129.1, 129.6)
        // Honshu.
        || within(lat, lon, 34.0, 41.6, 130.7, 142.2)
        // Hokkaido.
        || within(lat, lon, 41.3, 45.6, 139.3, 146.0)
        // The Ryukyus.
        || within(lat, lon, 24.0, 30.0, 122.85, 131.5)
        // Taiwan: the same evergreen broadleaf (camphor, Castanopsis, Machilus), bamboo everywhere, cypress and
        // Japanese-era sugi in the mountains.
        || within(lat, lon, 21.8, 25.4, 119.9, 122.1)
}

/// Piecewise-linear `y` at `x` through `points` (x ascending), held at the ends.
fn polyline(points: &[[f32; 2]], x: f32) -> f32 {
    let first = points[0];
    let last = points[points.len() - 1];
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    for pair in points.windows(2) {
        let ([x0, y0], [x1, y1]) = (pair[0], pair[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    last[1]
}

/// Africa's side of the Mediterranean: south of a line between its coast and Europe's islands and shores, from the
/// Atlantic through the Strait of Gibraltar (Tangier, Ceuta and Tarifa's mid-strait 35.95 N), between Spain and Morocco
/// and Algeria, Sardinia and Tunisia, Sicily, Pantelleria, Lampedusa and Malta (Europe's) and Tunisia and Libya, Crete
/// and Cyrenaica, to Egypt; Gaza and Israel stay out (east of 34.25 E).
fn african(lat: f32, lon: f32) -> bool {
    let shore = if lon < -2.0 {
        35.95
    } else if lon < 0.5 {
        36.3
    } else if lon < 11.6 {
        38.0
    } else if lon < 15.6 {
        34.5
    } else if lon < 23.0 {
        34.0
    } else {
        33.5
    };
    lat < shore && !(lon > 34.25 && lat > 31.2)
}

/// The Mediterranean's European and Asian shores whose summers are dry (Csa, Csb) and whose woods are holm oak, pines and
/// maquis: Iberia but its Atlantic north, the French Midi, Corsica and Sardinia, Liguria and Italy south of the Po's
/// plain, Malta and the Pelagie islands, the Adriatic's east shore under the Dinaric Alps, Albania and Greece, Turkey's
/// Aegean, Marmara and Mediterranean coasts (not the Anatolian plateau or the Black Sea's), Cyprus, the Levant's coast,
/// Crimea's south shore; and the Cape.
fn mediterranean(lat: f32, lon: f32) -> bool {
    // Iberia, without Galicia, northern Portugal and the Cantabrian and Basque coast.
    let iberia = within(lat, lon, 35.95, 42.6, -9.6, 4.4)
        && !(lat > 40.3 && lon < -7.8)
        && !(lat > 41.7 && lon < -6.6)
        && !(lat > 42.8 && lon < -1.4);
    // The Adriatic's east shore: the Dinaric Alps' seaward side, Istria to Albania.
    let adriatic = (13.3..=20.0).contains(&lon) && lat >= 39.5 && lat <= 45.4 - 0.55 * (lon - 13.5);
    iberia
        || adriatic
        // The French Midi; Corsica and Sardinia.
        || within(lat, lon, 42.3, 44.25, 2.5, 7.8)
        || within(lat, lon, 38.8, 43.1, 8.0, 9.9)
        // Liguria; the peninsula south of the Po's plain, Sicily; Malta; Pantelleria and the Pelagie islands.
        || within(lat, lon, 43.75, 44.55, 7.5, 10.2)
        || within(lat, lon, 36.6, 44.0, 9.9, 18.6)
        || within(lat, lon, 35.7, 36.15, 14.1, 14.65)
        || within(lat, lon, 35.4, 36.9, 11.8, 12.9)
        // Greece and the Aegean's islands, Crete.
        || within(lat, lon, 34.8, 41.0, 19.3, 28.3)
        // Turkey: the Aegean coast, the Sea of Marmara (to Istanbul; the Bosphorus' Black Sea end and the Belgrad
        // forest's oak and hornbeam are northern Europe's), the Mediterranean coast under the Taurus.
        || within(lat, lon, 36.5, 40.0, 25.8, 28.6)
        || within(lat, lon, 40.0, 41.12, 26.0, 30.6)
        || within(lat, lon, 36.0, 37.2, 28.0, 36.6)
        // Cyprus; the Levant's coast; Crimea's south shore.
        || within(lat, lon, 34.5, 35.75, 32.2, 34.65)
        || within(lat, lon, 31.2, 36.95, 34.2, 36.6)
        || within(lat, lon, 44.35, 44.8, 33.4, 35.2)
        // The Cape.
        || within(lat, lon, -35.5, -32.0, 17.0, 21.0)
}

/// Great Britain, Ireland and their islands, north of a line up the Channel to the Strait of Dover (Dover, Dungeness and
/// the Lizard in; Calais, Boulogne, Dieppe and Cherbourg out) and west of 1.8 E.
fn british(lat: f32, lon: f32) -> bool {
    let channel = polyline(
        &[
            [-6.0, 49.6],
            [-2.0, 49.9],
            [0.0, 50.0],
            [1.0, 50.45],
            [1.45, 50.95],
            [1.7, 51.25],
        ],
        lon,
    );
    within(lat, lon, 49.6, 61.0, -10.7, 1.8) && lat > channel
}

impl Biome {
    /// The biome of a land drawn in `climate` (where it stands: [`Climate::place`]) with style `style`; a battle map
    /// (no place) keeps its style's trees.
    pub fn of(style: LandStyle, climate: &Climate) -> Biome {
        match climate.place {
            Some([lat, lon]) => Biome::at(style, lat, lon),
            None => Biome::Authored,
        }
    }

    /// The biome of a theatre's land at a point of the earth: as [`Biome::with_class`] for the climate of the
    /// [`REGION_CELL_M`] about it ([`cell_climate`]).
    pub fn at(style: LandStyle, lat: f32, lon: f32) -> Biome {
        Biome::with_class(style, lat, lon, cell_climate(lat, lon))
    }

    /// The biome of a theatre's land at a point whose Köppen-Geiger class is `class` (none at sea, or where the map has
    /// no land): the flora boxes first where their flora grows (California's but in its deserts and above its oaks'
    /// and pines' climate, Japan's, Hawaii's, the American north-east's and south-east's, North Africa's, the
    /// Mediterranean's, Britain's, northern Europe's, Iceland's and the Faroes'), then the interior West, then the
    /// class's zone ([`by_class`]); with no class the latitude decides ([`by_latitude`]).
    pub fn with_class(style: LandStyle, lat: f32, lon: f32, class: Option<Koppen>) -> Biome {
        // The New World's Mediterranean coasts: California and central Chile (Monterey pine and eucalyptus planted
        // there by the million), south-western Australia (eucalyptus at home). Not their deserts (the Mojave, Death
        // Valley, the Nullarbor), nor the Sierra Nevada's and the Andes' high country (D and E) or the Great Basin's
        // side of the Sierra's crest: the interior West's, the arid biome's, the class's.
        let californian = within(lat, lon, 32.0, 42.5, -125.0, -116.5)
            || within(lat, lon, -38.5, -30.0, -75.0, -70.0)
            || within(lat, lon, -38.0, -30.0, 114.0, 140.0);
        let highland_or_desert = class.is_some_and(|k| {
            k.is_desert() || matches!(k.group(), KoppenGroup::Continental | KoppenGroup::Polar)
        });
        if californian && !highland_or_desert && !beyond_the_sierra(lat, lon) {
            return Biome::California;
        }
        if japanese(lat, lon) {
            return Biome::Japan;
        }
        if within(lat, lon, 18.5, 22.6, -160.6, -154.5) {
            return Biome::Hawaii;
        }
        // The United States' east: the broadleaf forest of New England, New York and Pennsylvania to the Potomac and
        // the Maritimes; the coastal plain from Hampton Roads to Florida, round the Gulf to Texas.
        if within(lat, lon, 38.6, 48.5, -80.6, -52.0) {
            return Biome::Northeast;
        }
        if within(lat, lon, 24.3, 38.6, -98.0, -74.5) {
            return Biome::Southeast;
        }
        // North Africa: Egypt (the Nile, its delta, Sinai, the Red Sea's west shore) and Libya's Marmarica east of 23 E;
        // west of it the Maghreb and Libya from the Atlantic to Cyrenaica, as far south as the Saharan Atlas' oases.
        if african(lat, lon) {
            if within(lat, lon, 21.5, 33.5, 23.0, 34.9) {
                return Biome::Egypt;
            }
            if within(lat, lon, 27.0, 38.0, -13.5, 23.0) {
                return Biome::NorthAfrica;
            }
        }
        if mediterranean(lat, lon) {
            return Biome::Mediterranean;
        }
        if british(lat, lon) {
            return Biome::Britain;
        }
        // North-western and middle Europe: Brittany to the Baltic below Scandinavia's forests (Göteborg and
        // Kristiansand in, Stavanger, Oslo and Stockholm boreal), the Alps, the Balkans' interior, Spain's Atlantic
        // north; and Turkey's Black Sea coast (beech, hornbeam, hazelnut).
        if within(lat, lon, 40.0, 58.5, -10.7, 32.0) || within(lat, lon, 40.5, 42.2, 30.6, 42.0) {
            return Biome::NorthernEurope;
        }
        // Iceland and the Faroes: birch scrub, a few planted spruce.
        if within(lat, lon, 62.5, 67.5, -25.0, -12.0) || within(lat, lon, 61.3, 62.5, -7.8, -6.2) {
            return Biome::Subarctic;
        }
        let Some(k) = class else {
            return by_latitude(style, lat);
        };
        // The interior West, from the Cascades' and the Coast Mountains' crests (their wet western slopes are the
        // class's, temperate) to the Front Range, Mexico's Sierra Madre to southern Canada: all but its hot deserts and
        // steppe, the arid biome's.
        if within(lat, lon, 28.0, 52.0, -125.0, -104.0)
            && !(k.group() == KoppenGroup::Temperate && lon < polyline(&COAST_CREST, lat))
        {
            return if k.is_arid() && k.warmth() == Some(Warmth::Hot) {
                Biome::Arid
            } else {
                Biome::InteriorWest
            };
        }
        by_class(style, lat, lon, k)
    }
}

/// The Cascades' and British Columbia's Coast Mountains' crests (latitude, longitude), north of California's box: the
/// wet side west of them, the dry interior east.
const COAST_CREST: [[f32; 2]; 9] = [
    [42.0, -122.2],
    [44.0, -121.85],
    [45.4, -121.7],
    [46.9, -121.6],
    [48.0, -121.2],
    [49.0, -121.0],
    [50.0, -122.6],
    [51.0, -124.0],
    [52.0, -125.8],
];

/// The Sierra Nevada's crest (latitude, longitude), Tejon Pass to the Cascades: Mount Whitney, Mammoth, Sonora Pass,
/// Tahoe's western rim, Lassen.
const SIERRA_CREST: [[f32; 2]; 10] = [
    [34.8, -118.9],
    [35.4, -118.25],
    [36.6, -118.3],
    [37.6, -119.0],
    [38.3, -119.65],
    [39.1, -120.15],
    [39.8, -120.5],
    [40.5, -121.5],
    [41.3, -121.7],
    [42.5, -122.0],
];

/// Whether a point in California's box stands on the Great Basin's side of the Sierra Nevada (Owens Valley, Mono Lake,
/// Tahoe's eastern shore, Death Valley, the Modoc plateau) or in the Mojave north of the San Gabriel Mountains.
fn beyond_the_sierra(lat: f32, lon: f32) -> bool {
    if !within(lat, lon, 34.3, 42.5, -125.0, -116.5) {
        return false;
    }
    if lat < 34.8 {
        return lon > -118.6;
    }
    lon > polyline(&SIERRA_CREST, lat)
}

/// The climate a cell of the biomes' grid (or a point) stands in: the commonest Köppen-Geiger class of the land within
/// half a [`REGION_CELL_M`] of it, of the classes trees grow in (not the tundra's or the ice's, E) where there are any,
/// so a fjord's valley under a plateau above the treeline, a mountain's forest under its summit, is the forest's. None
/// with no land (the sea, or an islet the map does not hold).
pub fn cell_climate(lat: f32, lon: f32) -> Option<Koppen> {
    let shares = crate::earth::koppen_shares(
        f64::from(lat),
        f64::from(lon),
        f64::from(REGION_CELL_M) * 0.5,
    );
    shares
        .mode_where(|k| k.group() != KoppenGroup::Polar)
        .or_else(|| shares.mode())
}

/// The biome of a Köppen-Geiger class's zone outside the flora boxes and the interior West: the tropics' (A, and the
/// tropics' mountains' C and E: the montane forest's); the deserts' and the hot steppes' (B, and the cold steppe's south
/// of 37 degrees: Iraq's, Iran's); the Old World's dry-summer coasts' (Cs; the New World's, the Pacific north-west's,
/// are temperate); the humid subtropics' (Cfa, Cwa, south of 35 degrees: Korea is temperate); the temperate (C and the
/// warm-summer D south of 57 degrees, the cold steppe north of 37) and the boreal (the cold-summer D, Dfc and Dfd, and
/// every C and D past 57 degrees), the subarctic past 70 degrees and on the tundra (E) past 45 (Kerguelen's, 49 S).
fn by_class(style: LandStyle, lat: f32, lon: f32, k: Koppen) -> Biome {
    let a = lat.abs();
    let tropics =
        a < 23.5 || matches!(style, LandStyle::Tropical | LandStyle::Volcanic) && a < 30.0;
    let forest = || {
        if a >= 70.0 {
            Biome::Subarctic
        } else if a < 57.0 {
            Biome::Temperate
        } else {
            Biome::Boreal
        }
    };
    match k.group() {
        KoppenGroup::Tropical => Biome::Tropical,
        KoppenGroup::Arid if k.warmth() == Some(Warmth::Hot) || a < 37.0 => Biome::Arid,
        KoppenGroup::Arid => forest(),
        KoppenGroup::Temperate | KoppenGroup::Polar if tropics => Biome::Tropical,
        KoppenGroup::Temperate => match (k.dry_season(), k.warmth()) {
            (Some(DrySeason::Summer), _) if a < 57.0 && lon > -30.0 => Biome::Mediterranean,
            (_, Some(Warmth::Hot)) if a < 35.0 => Biome::Subtropical,
            _ => forest(),
        },
        KoppenGroup::Continental => match k.warmth() {
            Some(Warmth::Cold | Warmth::Severe) if a < 70.0 => Biome::Boreal,
            _ => forest(),
        },
        KoppenGroup::Polar if a >= 45.0 => Biome::Subarctic,
        KoppenGroup::Polar => forest(),
    }
}

/// The biome by the latitude alone, where the climate map has no land (the open sea, an islet it misses): the tropics,
/// the subtropics to 35 degrees, the temperate to 57, the boreal, and the subarctic past 70.
fn by_latitude(style: LandStyle, lat: f32) -> Biome {
    let a = lat.abs();
    if a < 23.5 || matches!(style, LandStyle::Tropical | LandStyle::Volcanic) && a < 30.0 {
        return Biome::Tropical;
    }
    if a < 35.0 {
        return Biome::Subtropical;
    }
    if a >= 70.0 {
        return Biome::Subarctic;
    }
    if a < 57.0 {
        return Biome::Temperate;
    }
    Biome::Boreal
}

/// Cells a side of the biomes' grid about the eye ([`region_grid`]).
pub const REGION_CELLS: usize = 16;
/// A cell of the biomes' grid (m): fine enough to put a strait's two shores in their own biomes (the Strait of
/// Gibraltar is 14 km across, Dover to Calais 34 km), the grid wide enough (48 km) for the trees' reach.
pub const REGION_CELL_M: f32 = 3000.0;
/// Metres of the chart a degree of latitude spans (`tree_place.wgsl`'s `METRES_PER_DEGREE`).
const METRES_PER_DEGREE: f32 = 111_195.0;

/// Where chart point `chart` (x east, z south, m from the chart's origin) stands on the earth, the origin standing at
/// `place` (latitude, longitude): near enough for the tens of kilometres about a land window's origin.
pub fn chart_place(place: [f32; 2], chart: [f32; 2]) -> [f32; 2] {
    let lat = place[0] - chart[1] / METRES_PER_DEGREE;
    let lon = place[1] + chart[0] / (METRES_PER_DEGREE * place[0].to_radians().cos().max(0.05));
    [lat, (lon + 540.0).rem_euclid(360.0) - 180.0]
}

/// The biomes of a square of [`REGION_CELLS`]² cells of [`REGION_CELL_M`] from cell `corner` of the chart (cell
/// (i, j) spans chart x from (corner.x + i) cells, z likewise), each the biome at its middle: a land window spans 96 km,
/// and a strait or a frontier through it (Tarifa and Tangier, Dover and Calais, Gaza and Sinai) puts its two shores in
/// different biomes, so the placement reads each tree's from the grid. A byte a cell, four to a word, cell
/// k = j * REGION_CELLS + i in word k / 4 at bits 8 (k mod 4).
pub fn region_grid(
    style: LandStyle,
    place: [f32; 2],
    corner: [i32; 2],
) -> [[u32; 4]; REGION_CELLS * REGION_CELLS / 16] {
    let mut words = [[0u32; 4]; REGION_CELLS * REGION_CELLS / 16];
    for j in 0..REGION_CELLS {
        for i in 0..REGION_CELLS {
            let middle = [
                (corner[0] as f32 + i as f32 + 0.5) * REGION_CELL_M,
                (corner[1] as f32 + j as f32 + 0.5) * REGION_CELL_M,
            ];
            let [lat, lon] = chart_place(place, middle);
            let biome = Biome::at(style, lat, lon) as u32;
            let k = j * REGION_CELLS + i;
            words[k / 16][(k / 4) % 4] |= biome << (8 * (k % 4));
        }
    }
    words
}

/// One shape of a model, as `tree_bake.wgsl` reads it.
///
/// - `a`: centre (a clump) or start (a capsule), and radius.
/// - `b`: a capsule's end, or a clump's lobe centre (its normal leans away from it); w: the kind (0 a leaf clump, 1 a
///   capsule of bark, 2 a clump of twigs).
/// - `c`: linear albedo; w: how much of a clump's rim the leaves leave open (0-1).
/// - `d`: x its ambient occlusion (0 hidden, 1 open), y how far a clump's normal leans from its own centre toward its
///   lobe's (0-1), z its own random value; w unused.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct Primitive {
    pub a: [f32; 4],
    pub b: [f32; 4],
    pub c: [f32; 4],
    pub d: [f32; 4],
}

pub const KIND_CLUMP: f32 = 0.0;
pub const KIND_BARK: f32 = 1.0;
pub const KIND_TWIGS: f32 = 2.0;

/// The most shapes a model may have (`tree_bake.wgsl`'s workgroup list).
pub const MODEL_CAPACITY: usize = 1024;

/// A species' model, normalised (foot at 0, top at 1): its design, and the tree grown from it
/// ([`crate::tree_growth`]), which the near meshes draw and the impostors are baked from.
#[derive(Clone, Debug)]
pub struct Model {
    pub species: Species,
    pub primitives: Vec<Primitive>,
    /// The crown's widest reach from the axis over the height, the grown tree's.
    pub radius: f32,
    /// The grown tree's limbs (the design's trunk and limbs, and the branches to its lobes and clumps) and its sprays.
    pub limbs: Vec<crate::tree_growth::Limb>,
    pub sprays: Vec<crate::tree_growth::Spray>,
}

/// Every species' model, in [`Species::ALL`]'s order, built and grown once.
pub fn models() -> &'static [Model] {
    static MODELS: OnceLock<Vec<Model>> = OnceLock::new();
    MODELS.get_or_init(|| Species::ALL.iter().map(|s| grown(build(*s))).collect())
}

/// A design grown into its limbs and sprays, its radius the grown tree's.
fn grown(mut model: Model) -> Model {
    let (limbs, sprays) = crate::tree_growth::grow(&model);
    model.radius = model.radius.max(crate::tree_growth::reach(&limbs, &sprays));
    model.limbs = limbs;
    model.sprays = sprays;
    model
}

/// The web's LCG, as the old atlas used: deterministic per species.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / 16_777_216.0
    }
    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
    /// A point in the unit ball, denser toward its shell by `shell` (0 uniform).
    fn ball(&mut self, shell: f32) -> Vec3 {
        loop {
            let p = Vec3::new(
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
                self.next() * 2.0 - 1.0,
            );
            let l = p.length();
            if l <= 1.0 && l > 1e-3 {
                return p / l * l.powf(1.0 - shell * 0.8);
            }
        }
    }
}

/// sRGB bytes to linear albedo.
fn srgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    let f = |v: u8| {
        let c = f32::from(v) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [f(r), f(g), f(b)]
}

fn shade(color: [f32; 3], k: f32) -> [f32; 3] {
    [color[0] * k, color[1] * k, color[2] * k]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

struct Builder {
    rng: Rng,
    primitives: Vec<Primitive>,
}

/// A crown's leaves: their colour and its spread, how open their rims are, how far their normals lean to the lobe's.
#[derive(Clone, Copy)]
struct Leaves {
    color: [f32; 3],
    /// A second colour the clumps lean toward at random (young growth, sunlit tips, the underside's grey).
    other: [f32; 3],
    spread: f32,
    open: f32,
    bend: f32,
}

impl Builder {
    fn new(seed: u32) -> Self {
        Self {
            rng: Rng(seed.wrapping_mul(2_654_435_761).wrapping_add(97)),
            primitives: Vec::new(),
        }
    }

    fn capsule(&mut self, a: Vec3, b: Vec3, radius: f32, color: [f32; 3], occlusion: f32) {
        let seed = self.rng.next();
        self.primitives.push(Primitive {
            a: [a.x, a.y, a.z, radius],
            b: [b.x, b.y, b.z, KIND_BARK],
            c: [color[0], color[1], color[2], 0.0],
            d: [occlusion, 0.0, seed, 0.0],
        });
    }

    /// A trunk or limb from `a` to `b` in `segments`, tapering from `r0` to `r1`, bending by `bow` (m) sideways.
    #[allow(clippy::too_many_arguments)]
    fn limb(
        &mut self,
        a: Vec3,
        b: Vec3,
        r0: f32,
        r1: f32,
        segments: u32,
        bow: Vec3,
        color: [f32; 3],
    ) {
        let mut previous = a;
        for k in 1..=segments {
            let t = k as f32 / segments as f32;
            let bend = bow * (std::f32::consts::PI * t).sin();
            let wobble = Vec3::new(self.rng.range(-1.0, 1.0), 0.0, self.rng.range(-1.0, 1.0))
                * (b - a).length()
                * 0.012;
            let p = a.lerp(b, t) + bend + if k < segments { wobble } else { Vec3::ZERO };
            let r = r0 + (r1 - r0) * (t - 0.5 / segments as f32);
            let tone = self.rng.range(0.85, 1.12);
            self.capsule(previous, p, r, shade(color, tone), 0.6);
            previous = p;
        }
    }

    fn clump(&mut self, centre: Vec3, radius: f32, lobe: Vec3, leaves: &Leaves, kind: f32) {
        let k = 1.0 + (self.rng.next() - 0.5) * 3.2 * leaves.spread;
        let color = shade(mix3(leaves.color, leaves.other, self.rng.next().powi(2)), k);
        let seed = self.rng.next();
        self.primitives.push(Primitive {
            a: [centre.x, centre.y, centre.z, radius],
            b: [lobe.x, lobe.y, lobe.z, kind],
            c: [color[0], color[1], color[2], leaves.open],
            d: [1.0, leaves.bend, seed, 0.0],
        });
    }

    /// A lobe of `count` clumps of `radius` about `centre`, its clumps toward the lobe's shell, flattened by `flat`
    /// (vertical scale).
    fn lobe(
        &mut self,
        centre: Vec3,
        size: f32,
        count: u32,
        radius: (f32, f32),
        flat: f32,
        leaves: &Leaves,
    ) {
        for _ in 0..count {
            let mut p = self.rng.ball(0.6) * size;
            p.y *= flat;
            let r = self.rng.range(radius.0, radius.1);
            self.clump(centre + p, r, centre, leaves, KIND_CLUMP);
        }
    }

    /// Ambient occlusion per clump and capsule: the share of 12 directions over the upper hemisphere and the horizon
    /// in which a ray from its outer side meets no other clump.
    fn occlude(&mut self) {
        let directions: Vec<Vec3> = (0..12)
            .map(|k| {
                let azimuth = k as f32 * 2.399_963;
                let up = 0.15 + 0.85 * ((k % 4) as f32 / 3.0);
                let level = (1.0 - up * up).sqrt();
                Vec3::new(azimuth.cos() * level, up, azimuth.sin() * level)
            })
            .collect();
        let clumps: Vec<(Vec3, f32)> = self
            .primitives
            .iter()
            .filter(|p| p.b[3] != KIND_BARK)
            .map(|p| (Vec3::new(p.a[0], p.a[1], p.a[2]), p.a[3]))
            .collect();
        for index in 0..self.primitives.len() {
            let p = self.primitives[index];
            let (centre, radius) = if p.b[3] == KIND_BARK {
                let a = Vec3::new(p.a[0], p.a[1], p.a[2]);
                let b = Vec3::new(p.b[0], p.b[1], p.b[2]);
                ((a + b) * 0.5, p.a[3])
            } else {
                (Vec3::new(p.a[0], p.a[1], p.a[2]), p.a[3])
            };
            let mut open = 0usize;
            for d in &directions {
                let origin = centre + *d * radius * 1.05;
                let blocked = clumps.iter().any(|(c, r)| {
                    let to = *c - origin;
                    let along = to.dot(*d);
                    if along <= 0.0 {
                        return false;
                    }
                    // Leaves let some light through: a ray grazing a clump passes.
                    (to - *d * along).length_squared() < r * r * 0.55
                });
                if !blocked {
                    open += 1;
                }
            }
            let occlusion = open as f32 / directions.len() as f32;
            let base = if p.b[3] == KIND_BARK {
                p.d[0].min(occlusion * 0.8 + 0.2)
            } else {
                occlusion
            };
            self.primitives[index].d[0] = base.clamp(0.0, 1.0);
        }
    }

    /// Normalised: the foot at 0, the top at 1, the axis at x = z = 0.
    fn finish(mut self, species: Species) -> Model {
        self.occlude();
        // The trunk's foot stands on the ground (y = 0 as built), not the bottom of its capsule.
        let low = 0.0;
        let mut high = f32::MIN;
        for p in &self.primitives {
            high = high.max(p.a[1] + p.a[3]);
            if p.b[3] == KIND_BARK {
                high = high.max(p.b[1] + p.a[3]);
            }
        }
        let scale = 1.0 / (high - low).max(1e-3);
        let mut radius: f32 = 0.0;
        for p in &mut self.primitives {
            p.a[1] = (p.a[1] - low) * scale;
            p.a[0] *= scale;
            p.a[2] *= scale;
            p.a[3] *= scale;
            p.b[0] *= scale;
            p.b[1] = (p.b[1] - low) * scale;
            p.b[2] *= scale;
            let reach = |x: f32, z: f32| (x * x + z * z).sqrt();
            radius = radius.max(reach(p.a[0], p.a[2]) + p.a[3]);
            if p.b[3] == KIND_BARK {
                radius = radius.max(reach(p.b[0], p.b[2]) + p.a[3]);
            }
        }
        Model {
            species,
            primitives: self.primitives,
            radius,
            limbs: Vec::new(),
            sprays: Vec::new(),
        }
    }
}

/// Bark's colours are measured: most bark a linear albedo of 0.06-0.18 (sRGB about 70-118); pale by nature birch
/// 0.45-0.55, blue gum 0.3-0.45, the rainforest's smooth grey 0.18-0.3, a coconut palm's 0.18-0.25.
fn bark_brown() -> [f32; 3] {
    srgb(82, 74, 62)
}

/// A broadleaf crown on a trunk: `lobes` lobes over an ellipsoid of half-widths `crown` (x, y) centred `middle` up,
/// each of `clumps` clumps, the trunk forking toward them from `fork`.
#[allow(clippy::too_many_arguments)]
fn broadleaf(
    b: &mut Builder,
    height: f32,
    fork: f32,
    middle: f32,
    crown: (f32, f32),
    lobes: u32,
    clumps: u32,
    lobe_size: (f32, f32),
    leaves: Leaves,
    bark: [f32; 3],
    trunk: f32,
) {
    let base = Vec3::ZERO;
    let fork_at = Vec3::new(b.rng.range(-0.3, 0.3), fork, b.rng.range(-0.3, 0.3));
    b.limb(base, fork_at, trunk, trunk * 0.8, 3, Vec3::ZERO, bark);
    let centre = Vec3::new(0.0, middle, 0.0);
    let mut placed: Vec<Vec3> = Vec::new();
    for l in 0..lobes {
        // Lobes over the crown's shell, the upper ones first in the list so they are lit.
        let mut p = b.rng.ball(0.85);
        if l == 0 {
            p = Vec3::new(0.0, 0.8, 0.0);
        }
        let at = centre + Vec3::new(p.x * crown.0, p.y * crown.1, p.z * crown.0);
        let size = b.rng.range(lobe_size.0, lobe_size.1);
        let clump_radius = (size * 0.32, size * 0.55);
        b.lobe(at, size, clumps, clump_radius, 0.85, &leaves);
        placed.push(at);
    }
    // Limbs from the fork to the lobes' inner sides.
    for at in placed.iter().take(lobes.min(7) as usize) {
        let toward = *at - (*at - fork_at) * 0.3;
        let bow = Vec3::new(0.0, (at.y - fork_at.y).abs() * 0.08, 0.0);
        b.limb(fork_at, toward, trunk * 0.55, trunk * 0.2, 2, bow, bark);
    }
    let _ = height;
}

fn build(species: Species) -> Model {
    let mut b = Builder::new(species as u32 * 131 + 17);
    match species {
        Species::Rainforest => {
            let leaves = Leaves {
                color: srgb(50, 76, 36),
                other: srgb(72, 92, 40),
                spread: 0.18,
                open: 0.45,
                bend: 0.55,
            };
            // A deep, uneven crown from a third of the way up (a closed canopy shows crowns, not boles: the lower
            // crown and the climbers and epiphytes on the trunk fill in under the top).
            broadleaf(
                &mut b,
                30.0,
                10.0,
                20.0,
                (10.0, 7.0),
                18,
                22,
                (3.4, 5.0),
                leaves,
                // Smooth grey (a linear albedo of 0.10: the measured 0.18-0.3 read pale against the wood's dark floor).
                srgb(92, 89, 82),
                0.55,
            );
            let lower = Leaves {
                color: srgb(44, 68, 34),
                ..leaves
            };
            for k in 0..3 {
                let a = k as f32 * 2.1 + 0.3;
                let at = Vec3::new(a.cos() * 2.4, 9.0 + k as f32 * 2.0, a.sin() * 2.4);
                let size = b.rng.range(2.6, 3.2);
                b.lobe(at, size, 16, (size * 0.32, size * 0.52), 0.8, &lower);
            }
        }
        Species::Emergent => {
            let leaves = Leaves {
                color: srgb(56, 80, 38),
                other: srgb(80, 96, 44),
                spread: 0.16,
                open: 0.5,
                bend: 0.6,
            };
            // A straight pale bole to 30 m, buttressed, and a flat crown 22 m across.
            b.limb(
                Vec3::ZERO,
                Vec3::new(0.0, 31.0, 0.0),
                1.0,
                0.6,
                4,
                Vec3::ZERO,
                srgb(98, 94, 86),
            );
            for k in 0..4 {
                let a = k as f32 * 1.57 + 0.4;
                b.capsule(
                    Vec3::new(a.cos() * 1.8, 0.0, a.sin() * 1.8),
                    Vec3::new(0.0, 3.5, 0.0),
                    0.35,
                    srgb(94, 90, 82),
                    0.5,
                );
            }
            let top = Vec3::new(0.0, 31.0, 0.0);
            for k in 0..9 {
                let a = k as f32 * 2.399_963;
                let r = 4.0 + 6.5 * ((k % 3) as f32 / 2.0);
                let at = Vec3::new(a.cos() * r, 36.0 + b.rng.range(-1.0, 2.0), a.sin() * r);
                b.limb(
                    top,
                    at - Vec3::Y * 1.5,
                    0.45,
                    0.2,
                    2,
                    Vec3::Y * 1.0,
                    srgb(94, 90, 82),
                );
                let size = b.rng.range(3.4, 4.6);
                b.lobe(at, size, 22, (size * 0.32, size * 0.52), 0.6, &leaves);
            }
        }
        Species::CoconutPalm => {
            // Cocos nucifera, the tall type (measured: a trunk 15-25 m tall, 25-40 cm across over a foot swollen to
            // about 60 cm, ringed with old leaf scars; 25-35 living fronds 4-6 m long in a crown 8-10 m across). The
            // trunk grey-brown, weathered (a linear albedo of about 0.17, 0.15 after the land's tone: measured
            // 0.18-0.25; a beach's palms seen against the sun stood pale from the sea while every trunk was lit from
            // inside, and 0.08 only hid that), its foot darker.
            let trunk = srgb(122, 114, 102);
            // Its foot leaning along x, its top turned up toward the light: the placement lays this curve along the
            // tree's own lean, which varies far more (`tree_place.wgsl`'s palms), one in four against it.
            let height = 18.0;
            let slope = 0.18;
            let axis = |y: f32| Vec3::new(slope * (y - 0.5 * y * y / height), y, 0.0);
            let radius = |y: f32| 0.135 + 0.035 * (1.0 - y / height) + 0.11 * (-y / 0.6).exp();
            let heights = [
                0.0, 0.3, 0.8, 1.6, 2.8, 4.4, 6.2, 8.0, 9.8, 11.6, 13.4, 15.2, 16.8, 18.0,
            ];
            for pair in heights.windows(2) {
                let (y0, y1) = (pair[0], pair[1]);
                b.capsule(
                    axis(y0),
                    axis(y1),
                    radius(y0 + 0.3 * (y1 - y0)),
                    shade(trunk, 0.86 + 0.14 * y1 / height),
                    0.75,
                );
            }
            let top = axis(height);
            let living = Leaves {
                color: srgb(64, 86, 36),
                other: srgb(90, 104, 42),
                spread: 0.15,
                open: 0.72,
                bend: 0.25,
            };
            let dead = Leaves {
                color: srgb(110, 92, 64),
                other: srgb(94, 80, 58),
                spread: 0.1,
                open: 0.7,
                bend: 0.2,
            };
            // Thirty living fronds round the crown a golden angle apart, the youngest first (they stand highest), each
            // a run of clumps along its rachis, narrow at its stalk, widest over its middle and narrowing to its tip
            // (the leaflets' sprite narrows): the youngest rise and arch over, those after them stand out level and
            // arch down, the oldest start below level and hang; then four dead ones hanging brown under the crown.
            const LIVING: usize = 30;
            const STEPS: usize = 12;
            for k in 0..LIVING + 4 {
                let old = k >= LIVING;
                let age = (k as f32 / (LIVING - 1) as f32).min(1.0);
                let azimuth = k as f32 * 2.399_963 + b.rng.range(-0.12, 0.12);
                let side = Vec3::new(azimuth.cos(), 0.0, azimuth.sin());
                let (rise, droop, length) = if old {
                    (
                        b.rng.range(-80.0f32, -66.0).to_radians(),
                        b.rng.range(5.0f32, 12.0).to_radians(),
                        b.rng.range(3.6, 4.4),
                    )
                } else {
                    (
                        (60.0 - 100.0 * age.powf(0.75) + b.rng.range(-6.0, 6.0)).to_radians(),
                        (38.0 + 40.0 * age + b.rng.range(-6.0, 6.0)).to_radians(),
                        3.5 + 1.2 * (age / 0.4).min(1.0) + b.rng.range(-0.3, 0.3),
                    )
                };
                let mut at =
                    top + side * 0.3 + Vec3::Y * (0.4 - 1.2 * age - if old { 0.4 } else { 0.0 });
                for s in 1..=STEPS {
                    let t = (s as f32 - 0.5) / STEPS as f32;
                    let angle = (rise - droop * t.powf(1.4)).max(-88.0f32.to_radians());
                    at += (side * angle.cos() + Vec3::Y * angle.sin()) * (length / STEPS as f32);
                    let u = s as f32 / STEPS as f32;
                    let e = (u / 0.45).min(1.0);
                    let r = 0.52 * (0.35 + 0.65 * e * e * (3.0 - 2.0 * e)) * (1.0 - 0.1 * u);
                    b.clump(at, r, top, if old { &dead } else { &living }, KIND_CLUMP);
                }
            }
            // The crown's heart: the frond bases and the nuts among them.
            b.clump(top + Vec3::Y * 0.2, 0.8, top, &living, KIND_CLUMP);
        }
        Species::Mangrove => {
            let bark = srgb(92, 82, 70);
            let leaves = Leaves {
                color: srgb(42, 64, 34),
                other: srgb(60, 78, 38),
                spread: 0.14,
                open: 0.35,
                bend: 0.6,
            };
            let hub = Vec3::new(0.0, 2.2, 0.0);
            b.limb(
                Vec3::new(0.0, 0.8, 0.0),
                Vec3::new(0.0, 3.4, 0.0),
                0.22,
                0.16,
                2,
                Vec3::ZERO,
                bark,
            );
            // Prop roots arching from the trunk into the mud.
            for k in 0..9 {
                let a = k as f32 * 2.399_963;
                let r = b.rng.range(1.6, 2.6);
                let foot = Vec3::new(a.cos() * r, 0.0, a.sin() * r);
                b.limb(
                    hub,
                    foot,
                    0.1,
                    0.07,
                    3,
                    (foot - hub).normalize() * 0.0 + Vec3::Y * 0.6,
                    bark,
                );
            }
            for l in 0..8 {
                let p = b.rng.ball(0.7);
                let at = Vec3::new(p.x * 2.8, 5.2 + p.y * 1.4, p.z * 2.8);
                let size = b.rng.range(1.6, 2.2);
                b.lobe(at, size, 22, (size * 0.34, size * 0.55), 0.75, &leaves);
                if l < 5 {
                    b.limb(
                        Vec3::new(0.0, 3.4, 0.0),
                        at,
                        0.12,
                        0.06,
                        1,
                        Vec3::ZERO,
                        bark,
                    );
                }
            }
        }
        Species::Oak => {
            // Separate masses over a wide crown, the sky and the limbs between them (until 2026-10-13 twelve lobes of
            // 24 clumps closing into one lumpy dome).
            let leaves = Leaves {
                color: srgb(56, 82, 40),
                other: srgb(82, 102, 48),
                spread: 0.18,
                open: 0.5,
                bend: 0.5,
            };
            broadleaf(
                &mut b,
                19.0,
                5.0,
                12.0,
                (8.0, 5.4),
                14,
                16,
                (2.4, 3.4),
                leaves,
                bark_brown(),
                0.5,
            );
        }
        Species::Ash => {
            let leaves = Leaves {
                color: srgb(60, 86, 42),
                other: srgb(86, 106, 50),
                spread: 0.18,
                open: 0.55,
                bend: 0.5,
            };
            broadleaf(
                &mut b,
                22.0,
                7.0,
                14.5,
                (5.0, 7.0),
                13,
                16,
                (2.3, 3.2),
                leaves,
                srgb(78, 74, 66),
                0.42,
            );
        }
        Species::Birch => {
            let leaves = Leaves {
                color: srgb(72, 96, 46),
                other: srgb(96, 112, 54),
                spread: 0.15,
                open: 0.58,
                bend: 0.45,
            };
            let bark = srgb(188, 182, 172);
            b.limb(
                Vec3::ZERO,
                Vec3::new(0.3, 14.5, 0.0),
                0.2,
                0.06,
                5,
                Vec3::new(0.25, 0.0, 0.0),
                bark,
            );
            for _ in 0..12 {
                let p = b.rng.ball(0.8);
                let at = Vec3::new(p.x * 2.6, 9.5 + p.y * 4.2, p.z * 2.6);
                let size = b.rng.range(1.4, 2.0);
                // Drooping: the lobes hang lower than they spread.
                b.lobe(at, size, 18, (size * 0.3, size * 0.5), 1.25, &leaves);
                b.limb(
                    Vec3::new(0.0, at.y - 2.0, 0.0),
                    at,
                    0.05,
                    0.03,
                    1,
                    Vec3::ZERO,
                    bark,
                );
            }
        }
        Species::BareOak | Species::BareBirch => {
            let birch = species == Species::BareBirch;
            let bark = if birch {
                srgb(196, 188, 178)
            } else {
                srgb(92, 82, 72)
            };
            // Fine twigs: a linear albedo of about 0.10-0.18, a grey brown (the birch's purpler), which a bare crown's
            // mass takes from afar.
            let twig = Leaves {
                color: if birch {
                    srgb(108, 88, 86)
                } else {
                    srgb(108, 92, 78)
                },
                other: if birch {
                    srgb(116, 95, 92)
                } else {
                    srgb(116, 99, 84)
                },
                spread: 0.12,
                open: 0.5,
                bend: 0.3,
            };
            let (top, spread, fork) = if birch {
                (14.5, 2.8, 4.0)
            } else {
                (18.0, 7.0, 4.5)
            };
            let fork_at = Vec3::new(0.0, fork, 0.0);
            b.limb(
                Vec3::ZERO,
                fork_at,
                if birch { 0.22 } else { 0.5 },
                0.4,
                2,
                Vec3::ZERO,
                bark,
            );
            // Recursive limbs, then a haze of twigs over the crown's shell.
            let mut tips: Vec<(Vec3, Vec3, f32)> = Vec::new();
            for k in 0..(if birch { 5 } else { 6 }) {
                let a = k as f32 * 2.399_963;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let end =
                    fork_at + out * spread * 0.55 + Vec3::Y * (top - fork) * b.rng.range(0.45, 0.7);
                tips.push((fork_at, end, if birch { 0.12 } else { 0.32 }));
            }
            let mut generation = 0;
            while generation < 3 {
                let mut next = Vec::new();
                for (a, end, r) in &tips {
                    let tone = b.rng.range(0.85, 1.1);
                    b.capsule(*a, *end, *r, shade(bark, tone), 0.6);
                    let dir = (*end - *a).normalize();
                    for _ in 0..2 {
                        let turn = Vec3::new(
                            b.rng.range(-0.7, 0.7),
                            b.rng.range(0.0, 0.5),
                            b.rng.range(-0.7, 0.7),
                        );
                        let d = (dir + turn).normalize();
                        let length = (*end - *a).length() * 0.62;
                        next.push((*end, *end + d * length, r * 0.6));
                    }
                }
                tips = next;
                generation += 1;
            }
            let centre = Vec3::new(0.0, top * 0.62, 0.0);
            for (a, end, r) in &tips {
                b.capsule(*a, *end, *r, bark, 0.7);
                let size = if birch { 1.3 } else { 1.9 };
                b.clump(*end, size, centre, &twig, KIND_TWIGS);
            }
            for _ in 0..(if birch { 36 } else { 64 }) {
                let p = b.rng.ball(0.9);
                let at = centre + Vec3::new(p.x * spread, p.y * (top - fork) * 0.42, p.z * spread);
                b.clump(at, if birch { 1.2 } else { 1.7 }, centre, &twig, KIND_TWIGS);
            }
        }
        Species::Spruce | Species::Redwood => {
            let redwood = species == Species::Redwood;
            let height = if redwood { 55.0 } else { 24.0 };
            let base = if redwood { 0.3 } else { 0.1 };
            let half = if redwood { 6.0 } else { 3.6 };
            let bark = if redwood {
                srgb(112, 72, 54)
            } else {
                srgb(80, 70, 60)
            };
            let leaves = Leaves {
                color: if redwood {
                    srgb(38, 60, 38)
                } else {
                    srgb(34, 54, 44)
                },
                other: if redwood {
                    srgb(52, 74, 44)
                } else {
                    srgb(46, 68, 52)
                },
                spread: 0.14,
                open: 0.4,
                bend: 0.35,
            };
            b.limb(
                Vec3::ZERO,
                Vec3::new(0.0, height * 0.97, 0.0),
                if redwood { 1.2 } else { 0.32 },
                0.1,
                5,
                Vec3::ZERO,
                bark,
            );
            let whorls = if redwood { 26 } else { 22 };
            for k in 0..whorls {
                let t = k as f32 / (whorls - 1) as f32;
                let y = height * (base + (0.97 - base) * t);
                // The cone; a redwood's rounds off at the top and is irregular.
                let profile = if redwood {
                    (1.0 - t).powf(0.75) * (0.55 + 0.45 * (t * 9.0).sin().abs())
                } else {
                    (1.0 - t).powf(0.95)
                };
                let reach = half * profile * b.rng.range(0.8, 1.15) + 0.4;
                let branches = if redwood { 5 } else { 6 };
                for j in 0..branches {
                    let a = j as f32 * std::f32::consts::TAU / branches as f32 + k as f32 * 0.9;
                    let out = Vec3::new(a.cos(), 0.0, a.sin());
                    let steps = (reach / 1.1).ceil().clamp(1.0, 5.0) as u32;
                    for s in 1..=steps {
                        let f = s as f32 / steps as f32;
                        // Drooping toward the tip.
                        let at = Vec3::new(0.0, y, 0.0) + out * reach * f
                            - Vec3::Y * reach * 0.25 * f * f;
                        let r = (0.75 + 0.35 * (1.0 - f)) * if redwood { 1.25 } else { 0.85 };
                        b.clump(at, r, Vec3::new(0.0, y + 1.0, 0.0), &leaves, KIND_CLUMP);
                    }
                }
            }
            b.clump(
                Vec3::new(0.0, height * 0.985, 0.0),
                0.5,
                Vec3::new(0.0, height * 0.9, 0.0),
                &leaves,
                KIND_CLUMP,
            );
        }
        Species::ScotsPine | Species::StonePine | Species::MontereyPine => {
            let (height, bole, crown_w, crown_h, flat, color, other, bark) = match species {
                Species::ScotsPine => (
                    20.0,
                    12.0,
                    3.8,
                    4.4,
                    0.55,
                    srgb(44, 64, 42),
                    srgb(58, 78, 48),
                    srgb(132, 84, 62),
                ),
                Species::StonePine => (
                    15.0,
                    10.5,
                    7.0,
                    2.4,
                    0.35,
                    srgb(46, 70, 40),
                    srgb(62, 84, 46),
                    srgb(100, 80, 68),
                ),
                _ => (
                    24.0,
                    7.0,
                    7.0,
                    8.0,
                    0.55,
                    srgb(36, 58, 38),
                    srgb(50, 72, 44),
                    srgb(84, 70, 58),
                ),
            };
            let leaves = Leaves {
                color,
                other,
                spread: 0.14,
                open: 0.42,
                bend: 0.5,
            };
            let lean = Vec3::new(b.rng.range(-0.6, 0.6), 0.0, b.rng.range(-0.6, 0.6));
            let top = Vec3::new(lean.x, bole, lean.z);
            b.limb(
                Vec3::ZERO,
                top,
                if species == Species::MontereyPine {
                    0.55
                } else {
                    0.4
                },
                0.25,
                4,
                lean * 0.4,
                bark,
            );
            let centre = top + Vec3::Y * crown_h * 0.6;
            let pads = if species == Species::MontereyPine {
                16
            } else {
                12
            };
            for k in 0..pads {
                let mut p = b.rng.ball(0.75);
                if species == Species::StonePine {
                    // A flat-topped umbrella: the pads on the dome's upper shell.
                    p.y = p.y.abs() * 0.6 + 0.2;
                }
                let at = centre + Vec3::new(p.x * crown_w, p.y * crown_h, p.z * crown_w);
                let size = b.rng.range(1.8, 2.7)
                    * if species == Species::StonePine {
                        1.2
                    } else {
                        1.0
                    };
                // Needle pads: wide and flat.
                b.lobe(at, size, 16, (size * 0.32, size * 0.5), flat, &leaves);
                if k < 8 {
                    let from = top - Vec3::Y * b.rng.range(0.0, crown_h * 0.4);
                    b.limb(
                        from,
                        at - Vec3::Y * size * 0.3,
                        0.16,
                        0.08,
                        2,
                        Vec3::Y * 0.4,
                        bark,
                    );
                }
            }
            let _ = height;
        }
        Species::Cypress => {
            let leaves = Leaves {
                color: srgb(32, 52, 34),
                other: srgb(44, 64, 40),
                spread: 0.12,
                open: 0.3,
                bend: 0.4,
            };
            b.limb(
                Vec3::ZERO,
                Vec3::new(0.0, 16.5, 0.0),
                0.3,
                0.08,
                3,
                Vec3::ZERO,
                srgb(90, 74, 60),
            );
            for k in 0..60 {
                let t = k as f32 / 59.0;
                let y = 1.2 + 15.6 * t;
                // A spindle: widest a third of the way up, pointed at the top.
                let reach = 1.45 * (t * 3.0).min(1.0).powf(0.5) * (1.0 - t).powf(0.7) + 0.25;
                let a = k as f32 * 2.399_963;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                b.clump(
                    Vec3::new(0.0, y, 0.0) + out * reach * 0.6,
                    reach * 0.75 + 0.2,
                    Vec3::new(0.0, y, 0.0),
                    &leaves,
                    KIND_CLUMP,
                );
            }
        }
        Species::Olive => {
            // Grey-green, the leaves' silver undersides the paler colour; as dark on the whole as any broadleaf's crown
            // (until 2026-10-13 twice as bright: a town's olive in the summer sun read nearly white).
            let leaves = Leaves {
                color: srgb(74, 86, 64),
                other: srgb(100, 108, 88),
                spread: 0.14,
                open: 0.58,
                bend: 0.5,
            };
            let bark = srgb(96, 90, 80);
            // A short crooked trunk, splitting low.
            let fork = Vec3::new(b.rng.range(-0.4, 0.4), 1.8, b.rng.range(-0.4, 0.4));
            b.limb(
                Vec3::ZERO,
                fork,
                0.38,
                0.32,
                3,
                Vec3::new(0.4, 0.0, 0.2),
                bark,
            );
            for k in 0..9 {
                let p = b.rng.ball(0.8);
                let at = Vec3::new(p.x * 3.2, 4.4 + p.y * 1.6, p.z * 3.2);
                let size = b.rng.range(1.2, 1.7);
                b.lobe(at, size, 16, (size * 0.32, size * 0.52), 0.8, &leaves);
                if k < 5 {
                    b.limb(
                        fork,
                        at - Vec3::Y * 0.5,
                        0.18,
                        0.08,
                        2,
                        Vec3::new(0.2, 0.3, 0.0),
                        bark,
                    );
                }
            }
        }
        Species::LiveOak => {
            // A short trunk dividing low into five massive, sinuous limbs that spread and arch far out, each carrying
            // its own billowing masses of small leaves at its elbow and its end, the lowest sweeping down toward the
            // ground: sky and the dark limbs between the masses, a crown wider than the tree is tall. (Until
            // 2026-10-13 one dense, dark dome of fourteen lobes: a head of broccoli from any distance.)
            let leaves = Leaves {
                color: srgb(50, 70, 38),
                other: srgb(88, 102, 54),
                spread: 0.2,
                open: 0.5,
                bend: 0.45,
            };
            let bark = srgb(92, 86, 78);
            let fork = Vec3::new(b.rng.range(-0.3, 0.3), 1.9, b.rng.range(-0.3, 0.3));
            b.limb(
                Vec3::ZERO,
                fork,
                0.6,
                0.5,
                2,
                Vec3::new(0.15, 0.0, 0.1),
                bark,
            );
            let limbs = 5;
            let turn = b.rng.range(0.0, std::f32::consts::TAU);
            for k in 0..limbs {
                let a = turn
                    + k as f32 * std::f32::consts::TAU / limbs as f32
                    + b.rng.range(-0.45, 0.45);
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                // Some limbs rise steeply into the crown's top, most reach out low and level.
                let rise = b.rng.range(0.35, 1.0);
                let reach = b.rng.range(5.5, 8.0) * (1.25 - 0.35 * rise);
                let elbow = fork + out * reach * 0.45 + Vec3::Y * (2.2 + 3.4 * rise);
                let end = fork + out * reach + Vec3::Y * (2.6 + 5.6 * rise)
                    - Vec3::Y * b.rng.range(0.0, 1.2);
                b.limb(fork, elbow, 0.34, 0.26, 2, Vec3::Y * 0.5 + out * 0.2, bark);
                b.limb(elbow, end, 0.26, 0.11, 2, Vec3::Y * 0.4, bark);
                let size = b.rng.range(2.5, 3.3);
                b.lobe(
                    end + Vec3::Y * size * 0.35,
                    size,
                    15,
                    (size * 0.3, size * 0.5),
                    0.62,
                    &leaves,
                );
                let size = b.rng.range(1.9, 2.5);
                b.lobe(
                    elbow + Vec3::Y * (size * 0.6 + 0.4) - out * 0.4,
                    size,
                    11,
                    (size * 0.3, size * 0.5),
                    0.66,
                    &leaves,
                );
                if rise < 0.6 {
                    // The low limb's tips hanging past its end.
                    let size = b.rng.range(1.6, 2.2);
                    b.lobe(
                        end + out * size * 0.9 - Vec3::Y * size * 0.45,
                        size,
                        9,
                        (size * 0.3, size * 0.5),
                        0.6,
                        &leaves,
                    );
                }
            }
            // The crown's top over the fork, apart from the limbs' masses.
            let size = b.rng.range(2.4, 3.0);
            let top = fork + Vec3::new(b.rng.range(-1.0, 1.0), 8.6, b.rng.range(-1.0, 1.0));
            b.lobe(top, size, 13, (size * 0.3, size * 0.5), 0.65, &leaves);
        }
        Species::MontereyCypress => {
            let leaves = Leaves {
                color: srgb(34, 56, 36),
                other: srgb(48, 70, 42),
                spread: 0.12,
                open: 0.34,
                bend: 0.55,
            };
            let bark = srgb(98, 84, 72);
            // Wind-shorn: a leaning trunk and flat tiers swept to one side.
            let fork = Vec3::new(1.0, 4.5, 0.0);
            b.limb(
                Vec3::ZERO,
                fork,
                0.6,
                0.45,
                3,
                Vec3::new(0.5, 0.0, 0.0),
                bark,
            );
            for tier in 0..3 {
                let y = 8.0 + tier as f32 * 2.2;
                let width = 7.0 - tier as f32 * 1.6;
                for _ in 0..5 {
                    let p = b.rng.ball(0.6);
                    let at =
                        Vec3::new(p.x * width + width * 0.35, y + p.y * 0.6, p.z * width * 0.8);
                    let size = b.rng.range(1.8, 2.5);
                    b.lobe(at, size, 16, (size * 0.32, size * 0.5), 0.42, &leaves);
                    b.limb(fork, at - Vec3::Y * 0.6, 0.2, 0.1, 2, Vec3::Y * 0.5, bark);
                }
            }
        }
        Species::Eucalyptus => {
            let leaves = Leaves {
                color: srgb(80, 98, 78),
                other: srgb(100, 110, 90),
                spread: 0.14,
                open: 0.62,
                bend: 0.45,
            };
            let bark = srgb(170, 162, 144);
            // A tall pale trunk forking at a third of its height into leaders that lean out; a ragged, open crown of
            // hanging clumps along them from there to the top, the sky between (a blue gum's crown is half its height
            // and more, not a tuft on a pole).
            let fork = Vec3::new(b.rng.range(-0.6, 0.6), 11.0, b.rng.range(-0.6, 0.6));
            b.limb(
                Vec3::ZERO,
                fork,
                0.6,
                0.42,
                4,
                Vec3::new(0.4, 0.0, 0.0),
                bark,
            );
            for k in 0..4 {
                let a = k as f32 * 1.57 + b.rng.range(-0.4, 0.4);
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let leader = fork + out * b.rng.range(2.5, 4.5) + Vec3::Y * b.rng.range(11.0, 16.0);
                b.limb(fork, leader, 0.32, 0.12, 3, Vec3::new(0.0, 0.0, 0.5), bark);
                for j in 0..6 {
                    let t = (j as f32 + b.rng.next()) / 6.0;
                    let along = fork.lerp(leader, 0.3 + 0.7 * t);
                    let reach = out * b.rng.range(0.5, 3.5)
                        + Vec3::new(b.rng.range(-2.0, 2.0), 0.0, b.rng.range(-2.0, 2.0));
                    let at = along + reach + Vec3::Y * b.rng.range(-1.0, 1.5);
                    let size = b.rng.range(1.4, 2.4);
                    b.lobe(at, size, 12, (size * 0.28, size * 0.48), 1.4, &leaves);
                    b.limb(along, at, 0.08, 0.04, 1, Vec3::ZERO, bark);
                }
            }
        }
        Species::Acacia => {
            let leaves = Leaves {
                color: srgb(84, 98, 52),
                other: srgb(108, 112, 62),
                spread: 0.16,
                open: 0.55,
                bend: 0.4,
            };
            let bark = srgb(80, 70, 60);
            let fork = Vec3::new(0.0, 2.4, 0.0);
            b.limb(
                Vec3::ZERO,
                fork,
                0.24,
                0.2,
                2,
                Vec3::new(0.3, 0.0, 0.0),
                bark,
            );
            for k in 0..10 {
                let p = b.rng.ball(0.9);
                let at = Vec3::new(p.x * 4.5, 6.8 + p.y.abs() * 0.6, p.z * 4.5);
                let size = b.rng.range(1.4, 2.0);
                b.lobe(at, size, 14, (size * 0.32, size * 0.5), 0.32, &leaves);
                if k < 6 {
                    b.limb(fork, at - Vec3::Y * 0.4, 0.1, 0.05, 2, Vec3::Y * 0.6, bark);
                }
            }
        }
        Species::Shrub => {
            let leaves = Leaves {
                color: srgb(56, 70, 42),
                other: srgb(78, 86, 54),
                spread: 0.18,
                open: 0.4,
                bend: 0.65,
            };
            for _ in 0..9 {
                let mut p = b.rng.ball(0.6);
                p.y = p.y * 0.5 + 0.1;
                let at = Vec3::new(p.x * 1.2, 1.1 + p.y * 1.0, p.z * 1.2);
                let size = b.rng.range(0.6, 0.85);
                b.lobe(at, size, 12, (size * 0.34, size * 0.55), 0.85, &leaves);
            }
            b.capsule(
                Vec3::ZERO,
                Vec3::new(0.0, 0.8, 0.0),
                0.08,
                srgb(80, 70, 58),
                0.3,
            );
        }
        Species::Garrigue => {
            let leaves = Leaves {
                color: srgb(86, 96, 68),
                other: srgb(108, 110, 82),
                spread: 0.16,
                open: 0.5,
                bend: 0.7,
            };
            for _ in 0..7 {
                let p = b.rng.ball(0.5);
                let at = Vec3::new(p.x * 0.55, 0.38 + p.y.abs() * 0.15, p.z * 0.55);
                let size = b.rng.range(0.28, 0.4);
                b.lobe(at, size, 10, (size * 0.36, size * 0.56), 0.7, &leaves);
            }
        }
        Species::FanPalm => {
            let trunk = srgb(122, 110, 94);
            let height = 22.0;
            let lean = Vec3::new(b.rng.range(0.3, 0.9), 0.0, b.rng.range(-0.3, 0.3));
            // A tall, slender, nearly straight trunk, flared at its foot.
            let mut previous = Vec3::ZERO;
            for k in 1..=12 {
                let t = k as f32 / 12.0;
                let p = Vec3::new(lean.x * t * t, height * t, lean.z * t * t);
                let r = 0.2 - 0.06 * t + if k == 1 { 0.08 } else { 0.0 };
                b.capsule(previous, p, r, shade(trunk, 0.9 + 0.15 * t), 0.75);
                previous = p;
            }
            let top = previous;
            // The skirt of dead fans hanging under the head (trimmed short, as a street's are).
            let dead = Leaves {
                color: srgb(130, 106, 72),
                other: srgb(106, 88, 64),
                spread: 0.1,
                open: 0.35,
                bend: 0.3,
            };
            for k in 0..14 {
                let a = k as f32 * 2.399_963;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let y = b.rng.range(-2.2, -0.3);
                b.clump(
                    top + out * 0.5 + Vec3::Y * y,
                    0.48,
                    top + Vec3::Y * y,
                    &dead,
                    KIND_CLUMP,
                );
            }
            // The head: a ball of fans on their stalks all round, the upper ones standing, the lower drooping; a fan
            // is a small flat disc across its stalk.
            let fan = Leaves {
                color: srgb(74, 98, 44),
                other: srgb(100, 112, 54),
                spread: 0.14,
                open: 0.45,
                bend: 0.3,
            };
            let heart = top + Vec3::Y * 0.9;
            for k in 0..30 {
                let azimuth = k as f32 * 2.399_963 + b.rng.range(-0.15, 0.15);
                let up = 0.92 - (k as f32 + 0.5) / 30.0 * 1.5;
                let level = (1.0 - up * up).max(0.0).sqrt();
                let dir = Vec3::new(azimuth.cos() * level, up, azimuth.sin() * level);
                let stalk = heart + dir * 0.8;
                let across = Vec3::new(-azimuth.sin(), 0.0, azimuth.cos());
                for f in 0..4 {
                    let o = f as f32 - 1.5;
                    let at = stalk + dir * (0.5 - 0.08 * o.abs()) + across * (o * 0.3);
                    b.clump(at, 0.36, heart, &fan, KIND_CLUMP);
                }
            }
            b.clump(heart, 0.7, top, &fan, KIND_CLUMP);
        }
        Species::DatePalm => {
            let trunk = srgb(112, 94, 72);
            let height = 9.5;
            // A stout trunk, a little swollen.
            for k in 1..=6 {
                let (t0, t) = ((k - 1) as f32 / 6.0, k as f32 / 6.0);
                let r = 0.52 - 0.06 * t + 0.05 * (t * 3.1).sin();
                b.capsule(
                    Vec3::Y * height * t0,
                    Vec3::Y * height * t,
                    r,
                    shade(trunk, 0.9 + 0.12 * t),
                    0.7,
                );
            }
            let top = Vec3::Y * height;
            // The knuckle of old leaf bases under the crown.
            let bases = Leaves {
                color: srgb(132, 100, 62),
                other: srgb(110, 86, 58),
                spread: 0.1,
                open: 0.2,
                bend: 0.4,
            };
            for k in 0..10 {
                let a = k as f32 * 2.399_963;
                let at = top + Vec3::new(a.cos() * 0.45, b.rng.range(-0.3, 0.5), a.sin() * 0.45);
                b.clump(at, 0.45, top, &bases, KIND_CLUMP);
            }
            // A great round crown: arching fronds from steeply up to drooping below the horizon.
            let frond = Leaves {
                color: srgb(60, 86, 38),
                other: srgb(86, 106, 46),
                spread: 0.14,
                open: 0.55,
                bend: 0.3,
            };
            let heart = top + Vec3::Y * 0.8;
            let fronds = 64;
            for k in 0..fronds {
                let azimuth = k as f32 * 2.399_963;
                let up = 0.75 - (k as f32 + 0.5) / fronds as f32 * 1.25;
                let level = (1.0 - up * up).max(0.0).sqrt();
                let dir = Vec3::new(azimuth.cos() * level, up, azimuth.sin() * level);
                let length = b.rng.range(4.6, 5.6);
                for s in 1..=7 {
                    let t = s as f32 / 7.0;
                    let at =
                        heart + dir * length * t - Vec3::Y * (1.6 * t * t * (0.4 + 0.6 * level));
                    b.clump(at, 0.5 * (1.0 - 0.5 * t), heart, &frond, KIND_CLUMP);
                }
            }
        }
        Species::Sycamore => {
            let bark = srgb(150, 144, 130);
            let leaves = Leaves {
                color: srgb(86, 110, 56),
                other: srgb(112, 124, 66),
                spread: 0.16,
                open: 0.52,
                bend: 0.55,
            };
            // Three pale trunks leaning out from one foot, each forking into limbs that carry separate masses.
            let mut tips = Vec::new();
            for k in 0..3 {
                let a = k as f32 * 2.2 + b.rng.range(-0.3, 0.3);
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let lean = b.rng.range(0.25, 0.55);
                let length = b.rng.range(7.0, 10.0);
                let end = out * length * lean + Vec3::Y * length;
                let tone = b.rng.range(0.9, 1.05);
                b.limb(
                    Vec3::ZERO,
                    end,
                    if k == 0 { 0.42 } else { 0.32 },
                    0.24,
                    3,
                    out * 0.8,
                    shade(bark, tone),
                );
                for j in 0..3 {
                    let turn = a + (j as f32 - 1.0) * 0.9 + b.rng.range(-0.3, 0.3);
                    let dir = Vec3::new(turn.cos(), 0.0, turn.sin());
                    let tip = end + dir * b.rng.range(3.0, 5.5) + Vec3::Y * b.rng.range(2.5, 6.0);
                    b.limb(end, tip, 0.2, 0.08, 2, Vec3::Y * 0.6, bark);
                    tips.push(tip);
                }
            }
            for tip in &tips {
                let size = b.rng.range(2.2, 3.2);
                b.lobe(
                    *tip + Vec3::Y * 0.6,
                    size,
                    18,
                    (size * 0.32, size * 0.52),
                    0.75,
                    &leaves,
                );
            }
        }
        Species::Chaparral => {
            let leaves = Leaves {
                color: srgb(52, 62, 38),
                other: srgb(86, 94, 64),
                spread: 0.2,
                open: 0.32,
                bend: 0.65,
            };
            let darker = Leaves {
                color: srgb(42, 52, 34),
                other: srgb(64, 72, 46),
                ..leaves
            };
            // Shrubs over a disc 16 m across, highest toward its middle and lumpy, closing over all but a few gaps.
            let reach = 7.5;
            for _ in 0..56 {
                let p = b.rng.ball(0.0);
                let at = Vec3::new(p.x, 0.0, p.z) * reach;
                let d = at.length() / reach;
                let size = b.rng.range(1.15, 2.0) * (1.0 - 0.25 * d);
                let top = 1.2 + 1.3 * (1.0 - d * d) + b.rng.range(-0.3, 0.4);
                let centre = at + Vec3::Y * (top - size * 0.55).max(size * 0.5);
                let kind = if b.rng.next() < 0.35 { darker } else { leaves };
                b.lobe(centre, size, 9, (size * 0.36, size * 0.58), 0.75, &kind);
            }
            // Manzanita's red limbs here and there at the edge.
            for k in 0..6 {
                let a = k as f32 * 2.399_963;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let foot = out * reach * b.rng.range(0.5, 0.9);
                b.capsule(
                    foot,
                    foot + out * 0.6 + Vec3::Y * 1.1,
                    0.07,
                    srgb(120, 62, 46),
                    0.5,
                );
            }
        }
        Species::Sugi => {
            let bark = srgb(112, 74, 54);
            let leaves = Leaves {
                color: srgb(34, 50, 32),
                other: srgb(58, 70, 40),
                spread: 0.12,
                open: 0.3,
                bend: 0.4,
            };
            // Nineteen trees of one age on a hexagonal grid 4.2 m apart (a plantation after its thinnings), each a
            // straight bole pruned of its lower branches under a narrow cone of tufted, slightly rising branches.
            let spacing = 4.2;
            let mut feet = vec![Vec3::ZERO];
            for ring in 1..=2 {
                for side in 0..6 {
                    let a0 = side as f32 * std::f32::consts::TAU / 6.0;
                    let a1 = (side + 1) as f32 * std::f32::consts::TAU / 6.0;
                    let c0 = Vec3::new(a0.cos(), 0.0, a0.sin()) * ring as f32;
                    let c1 = Vec3::new(a1.cos(), 0.0, a1.sin()) * ring as f32;
                    for step in 0..ring {
                        let p = c0.lerp(c1, step as f32 / ring as f32) * spacing;
                        let jitter = Vec3::new(b.rng.range(-0.5, 0.5), 0.0, b.rng.range(-0.5, 0.5));
                        feet.push(p + jitter);
                    }
                }
            }
            for foot in feet {
                let h = 26.0 * b.rng.range(0.9, 1.0);
                b.limb(
                    foot,
                    foot + Vec3::Y * h * 0.97,
                    0.22,
                    0.08,
                    2,
                    Vec3::ZERO,
                    bark,
                );
                let whorls = 8;
                let base = 0.44;
                let turn = b.rng.range(0.0, 6.0);
                for k in 0..whorls {
                    let t = k as f32 / (whorls - 1) as f32;
                    let y = h * (base + (0.95 - base) * t);
                    let reach = 2.0 * (1.0 - t).powf(0.85) + 0.3;
                    for j in 0..3 {
                        let a = turn + j as f32 * std::f32::consts::TAU / 3.0 + k as f32 * 0.8;
                        let out = Vec3::new(a.cos(), 0.0, a.sin());
                        let steps = if reach > 1.3 { 2 } else { 1 };
                        for s in 1..=steps {
                            let f = s as f32 / steps as f32;
                            let at =
                                foot + Vec3::Y * y + out * reach * f + Vec3::Y * reach * 0.12 * f;
                            let r = 0.75 + 0.3 * (1.0 - f);
                            b.clump(at, r, foot + Vec3::Y * (y + 1.0), &leaves, KIND_CLUMP);
                        }
                    }
                }
                b.clump(
                    foot + Vec3::Y * h * 0.985,
                    0.45,
                    foot + Vec3::Y * h * 0.9,
                    &leaves,
                    KIND_CLUMP,
                );
            }
        }
        Species::Laurel => {
            let leaves = Leaves {
                color: srgb(38, 58, 30),
                other: srgb(66, 84, 40),
                spread: 0.16,
                open: 0.26,
                bend: 0.72,
            };
            let bark = srgb(78, 72, 64);
            let fork = Vec3::new(b.rng.range(-0.3, 0.3), 3.0, b.rng.range(-0.3, 0.3));
            b.limb(Vec3::ZERO, fork, 0.45, 0.38, 2, Vec3::ZERO, bark);
            // A deep, dense dome of tight, bulging masses (a cauliflower's) down nearly to the fork, the masses toward
            // its shell, so a wood of them closes into one lumpy canopy with no trunks showing.
            let centre = Vec3::new(0.0, 9.0, 0.0);
            let mut placed = Vec::new();
            for l in 0..30 {
                let mut p = b.rng.ball(0.9);
                p.y = p.y.abs() * 0.45 + p.y * 0.55;
                if l == 0 {
                    p = Vec3::new(0.0, 0.95, 0.0);
                }
                let at = centre + Vec3::new(p.x * 6.0, p.y * 5.6, p.z * 6.0);
                let size = b.rng.range(1.6, 2.5);
                b.lobe(at, size, 16, (size * 0.34, size * 0.56), 0.9, &leaves);
                placed.push(at);
            }
            for at in placed.iter().skip(1).take(6) {
                b.limb(fork, at.lerp(fork, 0.35), 0.28, 0.1, 2, Vec3::Y * 0.5, bark);
            }
        }
        Species::BlackPine => {
            let bark = srgb(80, 72, 64);
            let leaves = Leaves {
                color: srgb(36, 54, 34),
                other: srgb(54, 70, 40),
                spread: 0.14,
                open: 0.36,
                bend: 0.5,
            };
            // A crooked trunk leaning (seaward) in three bends.
            let lean = Vec3::new(b.rng.range(1.5, 3.0), 0.0, b.rng.range(-0.8, 0.8));
            let p1 = Vec3::new(lean.x * 0.15, 4.5, lean.z * 0.1 + 0.4);
            let p2 = Vec3::new(lean.x * 0.55, 8.5, lean.z * 0.5 - 0.3);
            let p3 = Vec3::new(lean.x, 12.0, lean.z);
            b.limb(Vec3::ZERO, p1, 0.36, 0.3, 2, Vec3::new(0.3, 0.0, 0.0), bark);
            b.limb(p1, p2, 0.3, 0.24, 2, Vec3::new(-0.3, 0.0, 0.2), bark);
            b.limb(p2, p3, 0.24, 0.15, 2, Vec3::new(0.2, 0.0, -0.2), bark);
            // Flat cloud pads: the leader's, and on long limbs out to the sides at several heights.
            let mut pads = vec![(p3 + Vec3::Y * 1.2, 2.6)];
            for k in 0..6 {
                let a = k as f32 * 2.399_963 + b.rng.range(-0.3, 0.3);
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let from = p1.lerp(p3, 0.35 + 0.65 * (k as f32 / 5.0));
                let tip = from + out * b.rng.range(2.5, 4.5) + Vec3::Y * b.rng.range(0.5, 1.8);
                b.limb(from, tip, 0.14, 0.07, 2, Vec3::Y * 0.4, bark);
                pads.push((tip, b.rng.range(1.7, 2.5)));
            }
            for (at, size) in pads {
                b.lobe(at, size, 18, (size * 0.32, size * 0.5), 0.38, &leaves);
            }
        }
        Species::Bamboo => {
            let culm = srgb(118, 128, 70);
            let leaves = Leaves {
                color: srgb(76, 106, 44),
                other: srgb(104, 122, 56),
                spread: 0.16,
                open: 0.6,
                bend: 0.3,
            };
            // A grove of thirty culms, the outer ones shorter and arching outward, feathery leaves over their upper
            // halves.
            for k in 0..30 {
                let a = k as f32 * 2.399_963;
                let r = 3.6 * ((k as f32 + 0.5) / 30.0).sqrt();
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let side = Vec3::new(-out.z, 0.0, out.x);
                let foot = out * r;
                let height = b.rng.range(12.0, 15.5) * (1.0 - 0.18 * r / 3.6);
                let lean = (0.06 + 0.22 * r / 3.6) * height;
                let mut previous = foot;
                for s in 1..=5 {
                    let t = s as f32 / 5.0;
                    let p = foot + Vec3::Y * height * t + out * lean * t * t;
                    b.capsule(previous, p, 0.07, shade(culm, 0.9 + 0.2 * t), 0.7);
                    previous = p;
                }
                for s in 0..6 {
                    let t = 0.45 + 0.55 * s as f32 / 5.0;
                    let at = foot
                        + Vec3::Y * height * t
                        + out * (lean * t * t + 0.5 + 0.4 * t)
                        + side * b.rng.range(-0.4, 0.4);
                    let r = b.rng.range(0.55, 0.85);
                    b.clump(at, r, foot + Vec3::Y * height * t, &leaves, KIND_CLUMP);
                }
            }
        }
        Species::OasisPalm => {
            let trunk = srgb(120, 102, 82);
            let height = 15.0;
            let lean = Vec3::new(b.rng.range(0.2, 0.7), 0.0, b.rng.range(-0.2, 0.2));
            // A slender, straight trunk, rough with the stubs of old fronds.
            let mut previous = Vec3::ZERO;
            for k in 1..=10 {
                let t = k as f32 / 10.0;
                let p = Vec3::new(lean.x * t * t, height * t, lean.z * t * t);
                b.capsule(
                    previous,
                    p,
                    0.3 - 0.07 * t,
                    shade(trunk, 0.86 + 0.18 * t),
                    0.75,
                );
                previous = p;
            }
            let top = previous;
            // The knuckle of leaf bases and the hanging date clusters under the crown.
            let bases = Leaves {
                color: srgb(142, 110, 66),
                other: srgb(156, 104, 48),
                spread: 0.1,
                open: 0.25,
                bend: 0.4,
            };
            for k in 0..8 {
                let a = k as f32 * 2.399_963;
                let at = top + Vec3::new(a.cos() * 0.45, b.rng.range(-0.9, 0.1), a.sin() * 0.45);
                b.clump(at, 0.36, top, &bases, KIND_CLUMP);
            }
            // The crown: stiff grey-green fronds, the young standing steeply up, the old arching out and down; open
            // enough that the sky shows between them.
            let frond = Leaves {
                color: srgb(84, 100, 68),
                other: srgb(114, 122, 86),
                spread: 0.12,
                open: 0.7,
                bend: 0.25,
            };
            let heart = top + Vec3::Y * 0.4;
            let fronds = 34;
            for k in 0..fronds {
                let azimuth = k as f32 * 2.399_963 + b.rng.range(-0.1, 0.1);
                let up = 0.85 - (k as f32 + 0.5) / fronds as f32 * 1.2;
                let level = (1.0 - up * up).max(0.0).sqrt();
                let dir = Vec3::new(azimuth.cos() * level, up, azimuth.sin() * level);
                let length = b.rng.range(3.6, 4.6);
                for s in 1..=8 {
                    let t = s as f32 / 8.0;
                    let at =
                        heart + dir * length * t - Vec3::Y * (0.9 * t * t * (0.3 + 0.7 * level));
                    b.clump(at, 0.42 * (1.0 - 0.5 * t), heart, &frond, KIND_CLUMP);
                }
            }
        }
        Species::GrassTuft => {
            let blade = Leaves {
                color: srgb(104, 116, 62),
                other: srgb(140, 132, 78),
                spread: 0.18,
                open: 0.55,
                bend: 0.1,
            };
            // Blades as chains of small clumps arching out from the tuft's heart.
            for k in 0..22 {
                let a = k as f32 * 2.399_963;
                let out = Vec3::new(a.cos(), 0.0, a.sin());
                let lean = b.rng.range(0.15, 0.55);
                let length = b.rng.range(0.5, 0.75);
                for s in 1..=6 {
                    let t = s as f32 / 6.0;
                    let at = out * (0.03 + lean * length * t * t)
                        + Vec3::Y * length * (t - lean * 0.35 * t * t);
                    b.clump(at, 0.045 * (1.0 - t * 0.5), Vec3::ZERO, &blade, KIND_CLUMP);
                }
            }
        }
    }
    let model = b.finish(species);
    debug_assert!(model.primitives.len() <= MODEL_CAPACITY, "{species:?}");
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_model_is_normalised_and_fits_the_bake() {
        for model in models() {
            let n = model.primitives.len();
            assert!(
                n > 20 && n <= MODEL_CAPACITY,
                "{:?}: {n} shapes",
                model.species
            );
            // A thicket is a patch of shrubs wider than it is tall; a tree's crown reaches no further than its height.
            let most = if model.species == Species::Chaparral {
                3.0
            } else {
                1.3
            };
            assert!(
                model.radius > 0.05 && model.radius < most,
                "{:?}: radius {}",
                model.species,
                model.radius
            );
            let mut top: f32 = 0.0;
            for p in &model.primitives {
                top = top.max(p.a[1] + p.a[3]);
                assert!(
                    p.a[1] + p.a[3] > -0.05,
                    "{:?} below its foot",
                    model.species
                );
                assert!(
                    (0.0..=1.0).contains(&p.d[0]),
                    "{:?}: occlusion",
                    model.species
                );
            }
            assert!((top - 1.0).abs() < 1e-3, "{:?}: top {top}", model.species);
        }
        // Built once, the same every time.
        assert_eq!(models()[0].primitives, build(Species::ALL[0]).primitives);
    }

    #[test]
    fn the_shaders_name_every_species_in_order() {
        let place = include_str!("../../shaders/tree_place.wgsl");
        let impostors = include_str!("../../shaders/trees.wgsl");
        for (index, species) in Species::ALL.iter().enumerate() {
            let line = format!("const {}: f32 = {index}.0;", species.constant());
            assert!(place.contains(&line), "tree_place.wgsl lacks `{line}`");
            // The impostors light a bare species' crown as its twigs.
            if crate::tree_meshes::foliage(*species) == crate::tree_meshes::Foliage::Twigs {
                let line = format!("const {}: u32 = {index}u;", species.constant());
                assert!(impostors.contains(&line), "trees.wgsl lacks `{line}`");
            }
        }
    }

    #[test]
    fn the_biome_follows_where_the_land_stands() {
        let at = |lat: f32, lon: f32, style: LandStyle| {
            Biome::of(
                style,
                &Climate {
                    place: Some([lat, lon]),
                    ..Climate::AUTHORED
                },
            )
        };
        assert_eq!(
            Biome::of(LandStyle::Tropical, &Climate::AUTHORED),
            Biome::Authored
        );
        assert_eq!(at(36.94, -122.42, LandStyle::Rock), Biome::California);
        assert_eq!(at(35.9, 14.5, LandStyle::Chalk), Biome::Mediterranean);
        assert_eq!(at(-9.4, 160.0, LandStyle::Tropical), Biome::Tropical);
        assert_eq!(at(51.1, 1.3, LandStyle::Chalk), Biome::Britain);
        assert_eq!(at(64.3, -22.3, LandStyle::Snow), Biome::Subarctic);
        assert_eq!(at(68.0, 14.4, LandStyle::Snow), Biome::Boreal);
        // Japan, the Ryukyus and Taiwan share a biome; Korea, Jeju, the China coast and Primorye do not.
        assert_eq!(at(33.6, 130.4, LandStyle::Rock), Biome::Japan);
        assert_eq!(at(35.31, 139.55, LandStyle::Rock), Biome::Japan);
        assert_eq!(at(43.06, 141.35, LandStyle::Snow), Biome::Japan);
        assert_eq!(at(34.2, 129.3, LandStyle::Rock), Biome::Japan);
        assert_eq!(at(26.2, 127.7, LandStyle::Tropical), Biome::Japan);
        assert_eq!(at(25.0, 121.5, LandStyle::Tropical), Biome::Japan);
        assert_eq!(at(35.1, 129.04, LandStyle::Rock), Biome::Temperate);
        assert_eq!(at(35.55, 129.32, LandStyle::Rock), Biome::Temperate);
        // Jeju's town is humid-subtropical (Cfa; Hallasan's upper slopes are Dfb, temperate).
        assert_eq!(at(33.51, 126.52, LandStyle::Rock), Biome::Subtropical);
        assert_eq!(at(31.2, 121.5, LandStyle::Rock), Biome::Subtropical);
        assert_eq!(at(43.1, 131.9, LandStyle::Rock), Biome::Temperate);
        assert_eq!(at(25.3, 55.3, LandStyle::Rock), Biome::Arid);
        // The climate map's zones where no box draws the flora: the boreal forest (Dfc) south of 57 degrees, the
        // cold steppe north of 37 temperate, the tundra past 45 subarctic.
        assert_eq!(at(53.3, -60.4, LandStyle::Rock), Biome::Boreal);
        assert_eq!(at(52.3, 104.3, LandStyle::Rock), Biome::Boreal);
        assert_eq!(at(53.0, 85.0, LandStyle::Rock), Biome::Temperate);
        assert_eq!(at(39.0, 33.0, LandStyle::Chalk), Biome::Temperate);
        assert_eq!(at(35.7, 51.4, LandStyle::Chalk), Biome::Arid);
        assert_eq!(at(78.2, 15.6, LandStyle::Snow), Biome::Subarctic);
        assert_eq!(at(-49.35, 70.22, LandStyle::Rock), Biome::Subarctic);
        // A fjord's 3 km under a plateau above the treeline is the valley's forest (Dfc), the plateau's own tundra.
        assert_eq!(cell_climate(62.1, 7.1).map(Koppen::code), Some("Dfc"));
        assert_eq!(at(62.1, 7.1, LandStyle::Snow), Biome::Boreal);
    }

    /// The interior West from the Sierra Nevada's and the Cascades' crests to the Front Range: the Grand Canyon's rims
    /// and gorge, the plateau's towns, the Great Basin, the Rockies; the hot deserts arid, California's coast and
    /// valleys and the Pacific north-west's wet side their own.
    #[test]
    fn the_interior_west_is_its_own() {
        let check = |towns: &[(&str, f32, f32)], biome: Biome| {
            for &(name, lat, lon) in towns {
                assert_eq!(
                    Biome::at(LandStyle::Chalk, lat, lon),
                    biome,
                    "{name} ({lat}, {lon})"
                );
            }
        };
        check(
            &[
                ("South Rim", 36.06, -112.11),
                ("Inner gorge", 36.1, -112.09),
                ("North Rim", 36.2, -112.05),
                ("Flagstaff", 35.2, -111.65),
                ("Santa Fe", 35.69, -105.94),
                ("Denver", 39.74, -104.99),
                ("Salt Lake City", 40.76, -111.89),
                ("Boise", 43.6, -116.2),
                ("Spokane", 47.66, -117.42),
                ("Yakima", 46.6, -120.5),
                ("Bend", 44.06, -121.31),
                ("Reno", 39.53, -119.81),
                ("Lake Tahoe", 39.1, -120.0),
                ("Owens Valley", 36.6, -118.06),
                ("Banff", 51.18, -115.57),
            ],
            Biome::InteriorWest,
        );
        check(
            &[
                ("Phoenix", 33.45, -112.07),
                ("Las Vegas", 36.17, -115.14),
                ("Death Valley", 36.46, -116.87),
                ("Barstow", 34.9, -117.02),
            ],
            Biome::Arid,
        );
        check(
            &[
                ("Santa Cruz", 36.94, -122.42),
                ("Monterey", 36.6, -121.9),
                ("Los Angeles", 34.05, -118.25),
                ("Malibu", 34.03, -118.7),
                ("San Diego", 32.7, -117.16),
                ("Yosemite Valley", 37.75, -119.6),
            ],
            Biome::California,
        );
        check(
            &[
                ("Seattle", 47.6, -122.3),
                ("Portland", 45.52, -122.68),
                ("Vancouver", 49.28, -123.12),
            ],
            Biome::Temperate,
        );
    }

    /// Real towns, each in its region's biome, and the towns either side of a strait or a frontier on their own sides.
    #[test]
    fn the_towns_stand_in_their_regions() {
        let check = |towns: &[(&str, f32, f32)], style: LandStyle, biome: Biome| {
            for &(name, lat, lon) in towns {
                assert_eq!(Biome::at(style, lat, lon), biome, "{name} ({lat}, {lon})");
            }
        };
        check(
            &[
                ("Portsmouth", 50.8, -1.09),
                ("Plymouth", 50.37, -4.14),
                ("Dover", 51.13, 1.31),
                ("Dungeness", 50.91, 0.98),
                ("The Lizard", 49.96, -5.2),
                ("Rosyth", 56.03, -3.43),
                ("Scapa Flow", 58.9, -3.05),
                ("Kirkwall", 58.98, -2.96),
                ("Stornoway", 58.21, -6.39),
                ("Lerwick", 60.15, -1.15),
                ("Belfast", 54.6, -5.93),
                ("Cork", 51.9, -8.47),
                ("London", 51.5, -0.12),
            ],
            LandStyle::Chalk,
            Biome::Britain,
        );
        check(
            &[
                ("Calais", 50.95, 1.86),
                ("Boulogne", 50.73, 1.6),
                ("Dieppe", 49.92, 1.08),
                ("Cherbourg", 49.64, -1.62),
                ("Brest", 48.39, -4.49),
                ("Den Helder", 52.96, 4.76),
                ("Wilhelmshaven", 53.52, 8.13),
                ("Kiel", 54.32, 10.14),
                ("Copenhagen", 55.68, 12.57),
                ("Gdansk", 54.35, 18.65),
                ("Goteborg", 57.7, 11.97),
                ("Bilbao", 43.26, -2.93),
                ("Ferrol", 43.48, -8.24),
                ("Porto", 41.15, -8.61),
                ("Bordeaux", 44.84, -0.58),
                ("Bologna", 44.49, 11.34),
                ("Venice", 45.44, 12.33),
                ("Trieste", 45.65, 13.78),
                ("Varna", 43.21, 27.91),
                ("Odessa", 46.48, 30.73),
                ("Trabzon", 41.0, 39.72),
                ("Belgrad forest", 41.18, 28.98),
            ],
            LandStyle::Chalk,
            Biome::NorthernEurope,
        );
        check(
            &[
                ("Bergen", 60.39, 5.32),
                ("Narvik", 68.44, 17.43),
                ("Stockholm", 59.33, 18.07),
            ],
            LandStyle::Snow,
            Biome::Boreal,
        );
        check(
            &[("Torshavn", 62.01, -6.77), ("Reykjavik", 64.15, -21.94)],
            LandStyle::Snow,
            Biome::Subarctic,
        );
        check(
            &[
                ("Valletta", 35.9, 14.51),
                ("Naples", 40.85, 14.27),
                ("Taranto", 40.47, 17.24),
                ("Toulon", 43.12, 5.93),
                ("Marseille", 43.3, 5.37),
                ("Barcelona", 41.39, 2.17),
                ("Cartagena", 37.6, -0.98),
                ("Malaga", 36.72, -4.42),
                ("Tarifa", 36.01, -5.6),
                ("Gibraltar", 36.14, -5.35),
                ("Algeciras", 36.13, -5.45),
                ("Piraeus", 37.94, 23.65),
                ("Souda Bay", 35.49, 24.07),
                ("Izmir", 38.42, 27.14),
                ("Istanbul", 41.01, 28.98),
                ("Haifa", 32.8, 34.99),
                ("Beirut", 33.89, 35.5),
                ("Gaza", 31.5, 34.47),
                ("Tel Aviv", 32.08, 34.78),
                ("Split", 43.51, 16.44),
                ("Palermo", 38.12, 13.36),
                ("Cagliari", 39.22, 9.11),
                ("Lisbon", 38.72, -9.14),
                ("Sevastopol", 44.6, 33.52),
                ("Pantelleria", 36.83, 11.94),
                ("Lampedusa", 35.5, 12.6),
            ],
            LandStyle::Chalk,
            Biome::Mediterranean,
        );
        check(
            &[
                ("Tangier", 35.77, -5.8),
                ("Ceuta", 35.89, -5.32),
                ("Melilla", 35.29, -2.94),
                ("Oran", 35.7, -0.63),
                ("Algiers", 36.75, 3.06),
                ("Annaba", 36.9, 7.76),
                ("Bizerte", 37.27, 9.87),
                ("Tunis", 36.8, 10.18),
                ("Sousse", 35.83, 10.64),
                ("Tripoli", 32.89, 13.19),
                ("Benghazi", 32.12, 20.07),
                ("Casablanca", 33.57, -7.59),
                ("Ghardaia", 32.49, 3.67),
            ],
            LandStyle::Chalk,
            Biome::NorthAfrica,
        );
        check(
            &[
                ("Tobruk", 32.08, 23.96),
                ("Bardia", 31.76, 25.08),
                ("El Alamein", 30.83, 28.95),
                ("Alexandria", 31.2, 29.92),
                ("Tanta", 30.79, 31.0),
                ("Cairo", 30.04, 31.24),
                ("Port Said", 31.26, 32.3),
                ("Suez", 29.97, 32.55),
                ("Luxor", 25.69, 32.64),
            ],
            LandStyle::Chalk,
            Biome::Egypt,
        );
        check(
            &[
                ("Aqaba", 29.53, 35.0),
                ("Jeddah", 21.49, 39.19),
                ("Kuwait", 29.37, 47.98),
                ("Khartoum", 15.5, 32.56),
            ],
            LandStyle::Rock,
            Biome::Arid,
        );
        check(
            &[
                ("Boston", 42.36, -71.06),
                ("Newport", 41.49, -71.31),
                ("New York", 40.71, -74.0),
                ("Philadelphia", 39.95, -75.17),
                ("Baltimore", 39.29, -76.61),
                ("Washington", 38.9, -77.04),
                ("Portland, Maine", 43.66, -70.26),
                ("Halifax", 44.65, -63.57),
            ],
            LandStyle::Chalk,
            Biome::Northeast,
        );
        check(
            &[
                ("Norfolk", 36.85, -76.29),
                ("Wilmington", 34.23, -77.94),
                ("Charleston", 32.78, -79.93),
                ("Savannah", 32.08, -81.09),
                ("Jacksonville", 30.33, -81.66),
                ("Miami", 25.76, -80.19),
                ("Key West", 24.55, -81.78),
                ("Pensacola", 30.42, -87.22),
                ("New Orleans", 29.95, -90.07),
                ("Galveston", 29.3, -94.8),
            ],
            LandStyle::Chalk,
            Biome::Southeast,
        );
        check(
            &[
                ("Honolulu", 21.31, -157.86),
                ("Pearl Harbor", 21.36, -157.95),
                ("Hilo", 19.72, -155.08),
            ],
            LandStyle::Tropical,
            Biome::Hawaii,
        );
        check(
            &[
                ("Guam", 13.44, 144.79),
                ("Saipan", 15.18, 145.75),
                ("Chuuk", 7.42, 151.78),
                ("Tarawa", 1.35, 172.98),
                ("Kwajalein", 8.72, 167.73),
                ("Tacloban", 11.24, 125.0),
                ("Manila", 14.6, 120.98),
                ("Port Moresby", -9.44, 147.18),
                ("Rabaul", -4.2, 152.17),
                ("Wake", 19.28, 166.65),
                ("Midway", 28.21, -177.37),
            ],
            LandStyle::Tropical,
            Biome::Tropical,
        );
    }

    /// The biomes' grid about a point puts each shore of a strait in its own biome.
    #[test]
    fn the_grid_parts_the_straits() {
        let cell = |words: &[[u32; 4]], i: usize, j: usize| {
            let k = j * REGION_CELLS + i;
            (words[k / 16][(k / 4) % 4] >> (8 * (k % 4))) & 255
        };
        // About the Strait of Gibraltar, the chart's origin at Gibraltar: Tarifa (36.01 N, 5.6 W) some 23 km west
        // and 15 km south, Ceuta (35.89 N, 5.32 W) 3 km east and 28 km south.
        let place = [36.14, -5.35];
        let corner = [-8, -4];
        let words = region_grid(LandStyle::Chalk, place, corner);
        let at = |x: f32, z: f32| {
            let i = (x / REGION_CELL_M).floor() as i32 - corner[0];
            let j = (z / REGION_CELL_M).floor() as i32 - corner[1];
            cell(&words, i as usize, j as usize)
        };
        assert_eq!(at(-22_500.0, 14_000.0), Biome::Mediterranean as u32);
        assert_eq!(at(2_700.0, 27_800.0), Biome::NorthAfrica as u32);
        assert_eq!(at(0.0, 0.0), Biome::Mediterranean as u32);
        let [lat, lon] = chart_place(place, [2_700.0, 27_800.0]);
        assert!(
            (lat - 35.89).abs() < 0.01 && (lon - -5.32).abs() < 0.01,
            "{lat} {lon}"
        );
    }
}
