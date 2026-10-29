//! Where a building stands, for its palettes, kinds and streets: the regions ([`Region::at`]) and the borders between
//! them, drawn coarsely from latitude and longitude (a few kilometres off at most where a border runs through a town's
//! hinterland, never through a town the tests name).

use naval_conditions::{LandStyle, SurfaceStyle};

/// The palette families buildings are coloured from, by where the land is ([`Region::at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Region {
    /// Norway, Sweden and Finland: painted timber (white, Falu red, yellow and ochre, grey) under black and dark grey
    /// tile, red tile and sheet metal; rendered blocks in ochre, yellow and cream in the cities.
    Nordic,
    /// Western and central Europe but the British Isles and the North Sea's brick country (France, central and southern
    /// Germany, the Alps, Poland, the Baltic states): brick, stone and render under slate and dark tiles.
    Temperate,
    /// The Mediterranean and Latin America: white, cream and ochre render under terracotta, shutters, balconies.
    Mediterranean,
    /// North America but the American West and Florida: siding and brick under asphalt shingles, brick rows under flat
    /// roofs in the old cities.
    American,
    /// The humid tropics: render and painted boards under galvanised and rusted corrugated iron.
    Tropical,
    /// A dry country: pale render and stone under flat concrete roofs.
    Arid,
    /// The American West (California, Nevada, Arizona, Utah, New Mexico): light stucco under grey and brown asphalt
    /// shingles or flat gravel and tar; low single-storey ranch houses and two-storey houses, blocks only downtown.
    West,
    /// Japan, Korea, Taiwan and eastern China: light walls under grey kawara and dark glazed tiles or grey, blue and
    /// green sheet metal; small two-storey houses close together, flat concrete mid-rise blocks in the centres.
    EastAsia,
    /// Iceland, the Faroes and Greenland: painted corrugated iron and boards under red, green, blue and black roofs.
    Iceland,
    /// England and Wales: red and brown brick (yellow stock now and then, white and cream render, stone) under slate and
    /// grey, brown and red-brown tile; terraces of two storeys with their chimney stacks along the ridge.
    British,
    /// Scotland, Ireland and the Isle of Man: grey harl, white and cream render and grey or red sandstone under slate.
    Celtic,
    /// The Low Countries, north Germany and Denmark: red, brown, purple-brown and yellow brick under orange and red clay
    /// pantiles and black glazed tile, thatch now and then; steep roofs, rows of houses.
    NorthSea,
    /// Florida south of about 29 degrees: white and pastel stucco under grey and brown shingle, barrel tile, white flat
    /// roofs and metal; single-storey houses on wide lots.
    Florida,
    /// Malta and Gozo: honey limestone and cream render under flat pale roofs; terraced townhouses of two and three
    /// storeys, painted timber balconies, blocks of four to eight storeys in Sliema.
    Malta,
    /// Greece, its islands and Cyprus: white, cream and pale ochre concrete blocks of four to seven storeys under flat
    /// roofs, balconies on every floor; houses whitewashed, half under flat roofs and half under terracotta.
    Aegean,
    /// North Africa from the Atlantic to Suez and the Sahara to 20 degrees (Morocco, Algeria, Tunisia, Libya, Egypt
    /// beyond the Nile): white, sand and ochre cube houses under flat roofs, colonial quarters' white blocks with
    /// shutters and balconies, a terracotta or green-tiled roof now and then.
    NorthAfrica,
    /// The Nile's delta and valley, Cairo and Alexandria: red brick between concrete frames, beige and ochre render;
    /// flat-roofed blocks of three to eight storeys even in the villages.
    Nile,
    /// Hawaii's main islands: painted boards, siding and stucco (white, pale green, yellow, tan) under hipped roofs of
    /// grey and brown shingle and corrugated metal (galvanised, green, red); houses of one storey close together,
    /// Honolulu's walk-ups and Waikiki's white towers.
    Hawaii,
    /// The Marianas (Guam, Saipan, Tinian, Rota), Palau, the Carolines (Yap, Chuuk, Pohnpei, Kosrae), Wake and Midway:
    /// a typhoon's concrete houses of one storey under flat roofs (white, cream, pastels), corrugated iron on the rest.
    Micronesia,
    /// The Marshalls, Kiribati, Nauru, Tuvalu, Tokelau and the Line and Phoenix Islands: an atoll's small houses of
    /// one storey under galvanised and rusted iron and pandanus thatch, concrete block, plywood and sheet walls.
    Atoll,
}

/// Every region, in [`Region::index`] order.
pub const REGIONS: [Region; 20] = [
    Region::Nordic,
    Region::Temperate,
    Region::Tropical,
    Region::Arid,
    Region::Mediterranean,
    Region::American,
    Region::West,
    Region::EastAsia,
    Region::Iceland,
    Region::British,
    Region::Celtic,
    Region::NorthSea,
    Region::Florida,
    Region::Malta,
    Region::Aegean,
    Region::NorthAfrica,
    Region::Nile,
    Region::Hawaii,
    Region::Micronesia,
    Region::Atoll,
];

impl Region {
    /// The family for a land style alone: the snow style's countries are Nordic, the chalk and rock styles' temperate
    /// (Mediterranean with a dry summer), the tropical and volcanic styles' tropical (arid when dry).
    pub fn of(style: &SurfaceStyle) -> Self {
        let dry = style.climate.dry > 0.5;
        match style.style {
            LandStyle::Snow => Region::Nordic,
            LandStyle::Chalk | LandStyle::Rock if dry => Region::Mediterranean,
            LandStyle::Chalk | LandStyle::Rock => Region::Temperate,
            LandStyle::Tropical | LandStyle::Volcanic if dry => Region::Arid,
            LandStyle::Tropical | LandStyle::Volcanic => Region::Tropical,
        }
    }

    /// The family where a place is (`lat`, `lon` degrees), its land style deciding where the map does not: the British
    /// Isles (English and Welsh, or Scottish, Irish and Manx), Iceland, the Nordic countries, the North Sea's brick
    /// country, the American West, Florida, Latin America (Mediterranean, as the Mediterranean's own shores and
    /// Turkey and Madeira), Hawaii, the rest of North America, Malta, Greece and Cyprus, North Africa (the Nile's
    /// delta and valley apart), western and central Europe (temperate), Japan, Korea, Taiwan and eastern China, Arabia
    /// and the Middle East (arid), Micronesia, the atolls east of it, the tropics (tropical, or arid with a dry
    /// style).
    pub fn at(lat: f64, lon: f64, style: &SurfaceStyle) -> Self {
        let lon = (lon + 540.0).rem_euclid(360.0) - 180.0;
        let within = |a: f64, b: f64, v: f64| (a..=b).contains(&v);
        let dry = style.climate.dry > 0.5;
        // Great Britain, Ireland and their isles, Shetland to Scilly; north of the Channel's middle.
        if within(49.8, 61.0, lat) && within(-11.0, 1.8, lon) && lat >= piecewise(&CHANNEL, lon) {
            return if celtic(lat, lon) {
                Region::Celtic
            } else {
                Region::British
            };
        }
        // Iceland, the Faroes, Jan Mayen; Greenland.
        if (lat >= 61.3 && within(-32.0, -5.0, lon)) || (lat >= 59.0 && within(-75.0, -10.0, lon)) {
            return Region::Iceland;
        }
        // Norway, Sweden and Finland (and Estonia's north coast, Svalbard).
        if (lat >= 57.85 && within(4.0, 32.0, lon)) || swedish(lat, lon) {
            return Region::Nordic;
        }
        // The Low Countries and French Flanders, the Lower Rhine, north Germany, Denmark, Pomerania to the Oder.
        if (within(50.0, 52.3, lat) && within(1.9, 7.1, lon))
            || (within(52.3, 57.85, lat) && within(2.5, 11.5, lon))
            || (within(53.3, 57.85, lat) && within(11.5, 15.0, lon))
        {
            return Region::NorthSea;
        }
        // The American West: California to the Rockies' foot, north of the Mexican border, south of Oregon and Idaho
        // (the Pacific north-west's siding is the American style's), Colorado's Front Range left to the American.
        if within(-125.0, -103.0, lon)
            && lat <= 42.0
            && lat >= mexican_border(lon)
            && !(lat > 37.0 && lon > -109.05)
        {
            return Region::West;
        }
        // Florida's peninsula and Keys, Tampa and Daytona south (Jacksonville, Tallahassee and the Panhandle are the
        // South's, the American style's); not Cuba or the Bahamas.
        if within(24.3, 29.25, lat) && within(-83.3, -79.85, lon) {
            return Region::Florida;
        }
        // Hawaii's main islands, Niihau to the Big Island.
        if within(18.5, 22.6, lat) && within(-160.8, -154.3, lon) {
            return Region::Hawaii;
        }
        // Latin America: south of the United States (its border to the Gulf; the Gulf coast and Florida kept out).
        let south = if lon < -97.15 {
            lat < mexican_border(lon)
        } else {
            lat < 32.5 && !(lat >= 25.8 && lon <= -79.0)
        };
        if south && within(-118.0, -34.0, lon) {
            return Region::Mediterranean;
        }
        // The rest of North America.
        if lat >= 24.0 && within(-170.0, -50.0, lon) {
            return Region::American;
        }
        // Malta, Gozo and Comino.
        if within(35.78, 36.1, lat) && within(14.15, 14.6, lon) {
            return Region::Malta;
        }
        if greek(lat, lon) {
            return Region::Aegean;
        }
        if african(lat, lon) {
            return if nile(lat, lon) {
                Region::Nile
            } else {
                Region::NorthAfrica
            };
        }
        // Madeira: Portugal's render and terracotta, not the Canaries' or Africa's flat roofs.
        if within(32.35, 33.15, lat) && within(-17.35, -16.25, lon) {
            return Region::Mediterranean;
        }
        // The Mediterranean's shores, Iberia, Turkey.
        if within(35.0, 45.5, lat) && within(-10.0, 42.0, lon) {
            return Region::Mediterranean;
        }
        // North Africa, Arabia, the Middle East.
        if (within(12.0, 35.0, lat) && within(-18.0, 63.0, lon))
            || (within(35.0, 40.0, lat) && within(42.0, 63.0, lon))
        {
            return Region::Arid;
        }
        // Western and central Europe.
        if within(45.5, 58.0, lat) && within(-11.0, 32.0, lon) {
            return Region::Temperate;
        }
        // Japan (to Hokkaido and Okinawa), Korea, Taiwan and eastern China; not Russia's Primorye.
        if within(21.8, 46.0, lat)
            && within(100.0, 146.0, lon)
            && !(lat > 42.4 && within(130.6, 139.2, lon))
        {
            return Region::EastAsia;
        }
        if micronesian(lat, lon) {
            return Region::Micronesia;
        }
        if atoll(lat, lon) {
            return Region::Atoll;
        }
        if lat.abs() < 23.5 {
            return if dry { Region::Arid } else { Region::Tropical };
        }
        Region::of(style)
    }

    /// A byte for the host's keys and the shader's `REGION_*`.
    pub fn index(self) -> u8 {
        match self {
            Region::Nordic => 0,
            Region::Temperate => 1,
            Region::Tropical => 2,
            Region::Arid => 3,
            Region::Mediterranean => 4,
            Region::American => 5,
            Region::West => 6,
            Region::EastAsia => 7,
            Region::Iceland => 8,
            Region::British => 9,
            Region::Celtic => 10,
            Region::NorthSea => 11,
            Region::Florida => 12,
            Region::Malta => 13,
            Region::Aegean => 14,
            Region::NorthAfrica => 15,
            Region::Nile => 16,
            Region::Hawaii => 17,
            Region::Micronesia => 18,
            Region::Atoll => 19,
        }
    }
}

/// The line `points` ((x, y) by increasing x) at `x`, held level past its ends.
fn piecewise(points: &[[f64; 2]], x: f64) -> f64 {
    let first = points[0];
    if x <= first[0] {
        return first[1];
    }
    for w in points.windows(2) {
        let ([x0, y0], [x1, y1]) = (w[0], w[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1][1]
}

/// The middle of the Channel and the Strait of Dover (longitude, latitude), England north of it: west of 0.2 degrees
/// the French coast lies south of 49.8, then the line runs between Hastings and Dieppe, Dover and Cap Gris-Nez, Deal
/// and Calais.
const CHANNEL: [[f64; 2]; 5] = [
    [0.2, 49.8],
    [0.6, 50.3],
    [1.2, 50.75],
    [1.45, 51.0],
    [1.8, 51.25],
];

/// The Anglo-Scottish border (longitude, latitude), Scotland north of it: Galloway's shore west of Cumbria's, the
/// Solway Firth (Southerness, Annan and Gretna Scottish; Whitehaven, Workington, Maryport, Silloth and Carlisle
/// English), the Cheviots, the Tweed (Coldstream and Eyemouth Scottish, Berwick English).
const SCOTLAND: [[f64; 2]; 7] = [
    [-3.75, 54.55],
    [-3.65, 54.78],
    [-3.4, 54.92],
    [-3.05, 54.98],
    [-2.6, 55.15],
    [-2.25, 55.37],
    [-2.03, 55.8],
];

/// Whether a place of the British Isles is Scotland, Ireland or the Isle of Man: north of the border, or west of the
/// Irish Sea's middle (5.4 degrees west, Wales' St David's Head east of it) north of Cornwall, or on Man.
fn celtic(lat: f64, lon: f64) -> bool {
    (lon < -5.4 && lat > 51.3)
        || lat > piecewise(&SCOTLAND, lon)
        || ((54.0..=54.45).contains(&lat) && (-4.9..=-4.25).contains(&lon))
}

/// Whether a place south of 57.85 degrees is Sweden's (its west coast from Gothenburg, Skåne across the Øresund,
/// Öland and Gotland), not Denmark's: east of the Kattegat's and the Sound's middle (Helsingør, Copenhagen and Dragør
/// Danish, Helsingborg and Malmö Swedish), north of 55.33 (Bornholm Danish), west of the Baltic states.
fn swedish(lat: f64, lon: f64) -> bool {
    if !(55.33..57.85).contains(&lat) || lon >= 20.0 {
        return false;
    }
    let east_of = if lat >= 57.0 {
        11.4
    } else if lat >= 56.15 {
        12.2
    } else if lat >= 55.75 {
        12.66
    } else {
        12.75
    };
    lon >= east_of
}

/// The Aegean's middle between Greece's islands and Turkey's coast (latitude, the easternmost Greek longitude from it
/// north): Rhodes' sea west of Fethiye (Kastellorizo, off Kaş, apart in [`greek`]), Kos west of Knidos, Kalymnos and Farmakonisi west of Didim
/// and Bodrum, Samos to the Mycale strait, Chios and Oinousses west of Çeşme, Lesbos west of Ayvalık, Lemnos and
/// Samothrace west of Bozcaada, Gökçeada and Gallipoli; Crete, Karpathos and Gavdos south of Turkey's coast.
const AEGEAN: [[f64; 2]; 11] = [
    [34.75, 30.0],
    [36.0, 28.6],
    [36.65, 27.37],
    [36.95, 27.15],
    [37.55, 27.06],
    [37.82, 26.7],
    [38.1, 26.25],
    [38.6, 26.5],
    [39.0, 26.63],
    [39.5, 25.95],
    [40.0, 25.75],
];

/// Whether a place is Greece's (the mainland, the Ionian and Aegean islands, Crete) or Cyprus's: south of Albania (Corfu
/// Greek, Sarandë Albanian), North Macedonia and Bulgaria, west of the Evros (Alexandroupoli and Soufli Greek, Enez and
/// Edirne Turkish) and of the Aegean's middle ([`AEGEAN`]).
fn greek(lat: f64, lon: f64) -> bool {
    // Cyprus; Kastellorizo, two kilometres off Kaş.
    if ((34.5..=35.75).contains(&lat) && (32.2..=34.65).contains(&lon))
        || ((36.1..=36.17).contains(&lat) && (29.5..=29.62).contains(&lon))
    {
        return true;
    }
    if !(34.75..=41.6).contains(&lat) || lon < 19.3 {
        return false;
    }
    // Albania north of Epirus, Corfu's north shore across the strait from Ksamil Greek.
    if lat > 39.65 && lon < 20.65 && !(lat < 39.85 && lon < 19.97) {
        return false;
    }
    if lat >= 40.7 {
        // Macedonia under North Macedonia's border, Thrace under Bulgaria's, west of the Evros.
        let north = if lon < 22.9 {
            41.0
        } else if lon < 25.9 {
            41.35
        } else {
            41.6
        };
        return lat <= north && lon <= 26.04 + (lat - 40.7) * 0.62;
    }
    let east = AEGEAN
        .iter()
        .rev()
        .find(|[from, _]| lat >= *from)
        .map_or(30.0, |[_, east]| *east);
    lon <= east
}

/// Egypt's east (latitude, longitude): the Red Sea's middle off Berenice, Marsa Alam and Hurghada, Sharm el-Sheikh,
/// Dahab and Nuweiba west of the Gulf of Aqaba's middle, Taba (Eilat and Aqaba beyond), then the border to Rafah (El
/// Arish Egyptian, Gaza not).
const EGYPT_EAST: [[f64; 2]; 7] = [
    [20.0, 38.0],
    [22.0, 37.5],
    [25.0, 36.0],
    [27.9, 34.6],
    [29.0, 34.75],
    [29.5, 34.93],
    [31.32, 34.22],
];

/// Whether a place is North Africa's, the Atlantic to Suez and Sinai and the Sahara to 20 degrees: south of the Strait
/// (Tangier African, Tarifa and Gibraltar Spanish, Ceuta and Melilla Spanish though on its shore), of Spain's south-east,
/// of Sicily, Pantelleria, Lampedusa and Malta (Tunisia's coast north of 34.6 degrees lies west of 11.3), of Crete and
/// Cyprus; east of Madeira and the Canaries; west of Egypt's east ([`EGYPT_EAST`]).
fn african(lat: f64, lon: f64) -> bool {
    if !(20.0..=37.6).contains(&lat) || lon < -17.5 {
        return false;
    }
    if lat > 27.7 && lon < -13.3 {
        return false;
    }
    if lon > piecewise(&EGYPT_EAST, lat) {
        return false;
    }
    let ceuta = (35.86..=35.92).contains(&lat) && (-5.39..=-5.27).contains(&lon);
    let melilla = (35.26..=35.33).contains(&lat) && (-2.98..=-2.91).contains(&lon);
    if lon < -1.0 {
        lat < 35.95 && !ceuta && !melilla
    } else if lon < 11.3 {
        lat < 37.6
    } else {
        lat < 34.6
    }
}

/// The Nile's course up its valley (latitude, longitude): Wadi Halfa, Aswan, Luxor, the Qena bend, Sohag, Asyut, Minya,
/// Beni Suef, Cairo.
const NILE: [[f64; 2]; 10] = [
    [22.0, 31.4],
    [24.09, 32.9],
    [25.3, 32.55],
    [25.69, 32.64],
    [26.1, 32.5],
    [26.56, 31.7],
    [27.18, 31.18],
    [28.1, 30.75],
    [29.07, 31.1],
    [29.8, 31.25],
];

/// Whether a place of North Africa is the Nile's: its delta from Alexandria to Port Said and Ismailia, Cairo, the
/// Fayoum, its valley within about 35 km of the river (not the desert's coast, Siwa, the Red Sea's towns or Suez).
fn nile(lat: f64, lon: f64) -> bool {
    let delta = (29.8..=31.65).contains(&lat) && (29.85..=32.35).contains(&lon);
    let fayoum = (29.0..=29.6).contains(&lat) && (30.4..=31.1).contains(&lon);
    let valley = (22.0..29.8).contains(&lat) && (lon - piecewise(&NILE, lat)).abs() < 0.35;
    delta || fayoum || valley
}

/// The Rio Grande from El Paso to the Gulf (longitude, latitude): Fort Hancock, Presidio, Lajitas, Big Bend, Boquillas,
/// Langtry, Del Rio, Eagle Pass, Laredo, Zapata, Roma, Hidalgo, Brownsville, the mouth.
const RIO_GRANDE: [[f64; 2]; 15] = [
    [-106.53, 31.78],
    [-105.85, 31.29],
    [-104.37, 29.55],
    [-103.78, 29.25],
    [-103.15, 28.97],
    [-102.95, 29.17],
    [-101.56, 29.79],
    [-100.9, 29.35],
    [-100.5, 28.7],
    [-99.51, 27.49],
    [-99.27, 26.9],
    [-99.02, 26.39],
    [-98.26, 26.08],
    [-97.5, 25.87],
    [-97.15, 25.96],
];

/// The border between the United States and Mexico at longitude `lon` (degrees), as a latitude: along 32.5 degrees to
/// the Colorado, down to 31.33 at Nogales, the 31.33 and 31.78 parallels to El Paso, then the Rio Grande by Big Bend to
/// the Gulf ([`RIO_GRANDE`], a few kilometres off where the river runs north and south).
pub fn mexican_border(lon: f64) -> f64 {
    if lon < -114.72 {
        return 32.534 + (lon + 117.12).max(0.0) / 2.4 * 0.184;
    }
    if lon < -111.07 {
        return 32.72 - (lon + 114.72) / 3.65 * 1.39;
    }
    if lon < -108.21 {
        return 31.33;
    }
    if lon < -106.53 {
        return 31.78;
    }
    piecewise(&RIO_GRANDE, lon)
}

/// Whether a place is the Marianas', Palau's or the Carolines' (north of 0.5 degrees, so not New Guinea's islands; east
/// of 130.5, so not the Philippines' or the Moluccas'), Kosrae's among the Marshalls' longitudes, Wake's or Midway's.
fn micronesian(lat: f64, lon: f64) -> bool {
    let within = |a: f64, b: f64, v: f64| (a..=b).contains(&v);
    (within(0.5, 21.0, lat) && within(130.5, 160.5, lon))
        || (within(5.15, 5.5, lat) && within(162.8, 163.15, lon))
        || (within(19.1, 19.45, lat) && within(166.45, 166.8, lon))
        || (within(27.6, 28.6, lat) && within(-178.6, -177.1, lon))
}

/// Whether a place is the Marshalls', Kiribati's, Nauru's, Tuvalu's or Tokelau's: east of 160.5 degrees between 11 S
/// and 15 N (the Solomons' Malaita, Makira and Temotu, south of 7.5 S and west of 170 E, left out), and the Phoenix
/// and Line Islands east of the date line, north of Samoa.
fn atoll(lat: f64, lon: f64) -> bool {
    let within = |a: f64, b: f64, v: f64| (a..=b).contains(&v);
    (within(-11.0, 15.0, lat) && lon >= 160.5 && !(lat < -7.5 && lon < 170.0))
        || (within(-12.0, 6.5, lat) && lon <= -150.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_conditions::Climate;

    #[test]
    fn regions_follow_the_map() {
        let style = |style: LandStyle, dry: f32| SurfaceStyle {
            style,
            shore: [0.0; 3],
            low: [0.0; 3],
            high: [0.0; 3],
            equator: [0.0; 2],
            climate: Climate {
                dry,
                ..Climate::default()
            },
        };
        let chalk = style(LandStyle::Chalk, 0.0);
        let tropical = style(LandStyle::Tropical, 0.0);
        let at = |lat: f64, lon: f64| Region::at(lat, lon, &chalk);
        let all = |region: Region, places: &[(&str, f64, f64)]| {
            for &(name, lat, lon) in places {
                assert_eq!(at(lat, lon), region, "{name} ({lat}, {lon})");
            }
        };
        // England and Wales, to the Channel's middle and the Scottish border: Berwick, Carlisle and the Cumbrian
        // coast English, the Strait of Dover's towns English, Scilly English, Holyhead and St David's Welsh.
        all(
            Region::British,
            &[
                ("London", 51.5, -0.1),
                ("Portsmouth", 50.8, -1.09),
                ("Southsea", 50.78, -1.08),
                ("Plymouth", 50.37, -4.14),
                ("Hastings", 50.86, 0.57),
                ("Dover", 51.13, 1.31),
                ("Deal", 51.22, 1.4),
                ("Ramsgate", 51.33, 1.42),
                ("Cardiff", 51.48, -3.18),
                ("St David's", 51.88, -5.27),
                ("Holyhead", 53.31, -4.63),
                ("Liverpool", 53.41, -2.98),
                ("Whitehaven", 54.55, -3.59),
                ("Workington", 54.64, -3.55),
                ("Maryport", 54.71, -3.5),
                ("Silloth", 54.87, -3.39),
                ("Carlisle", 54.89, -2.93),
                ("Newcastle", 54.97, -1.61),
                ("Berwick", 55.77, -2.0),
                ("Hugh Town, Scilly", 49.92, -6.3),
            ],
        );
        // Scotland, Ireland and Man: Gretna, Annan and Southerness over the Solway, Coldstream and Eyemouth on the
        // Tweed, Orkney and Shetland, the Hebrides.
        all(
            Region::Celtic,
            &[
                ("Edinburgh", 55.95, -3.2),
                ("Glasgow", 55.86, -4.25),
                ("Southerness", 54.87, -3.6),
                ("Annan", 54.99, -3.26),
                ("Gretna", 55.0, -3.06),
                ("Coldstream", 55.65, -2.25),
                ("Eyemouth", 55.87, -2.09),
                ("Kirkwall", 58.98, -2.96),
                ("Stromness", 58.96, -3.3),
                ("Lerwick", 60.15, -1.14),
                ("Stornoway", 58.21, -6.39),
                ("Dublin", 53.35, -6.26),
                ("Belfast", 54.6, -5.93),
                ("Cork", 51.9, -8.47),
                ("Douglas", 54.15, -4.48),
            ],
        );
        all(
            Region::Iceland,
            &[
                ("Reykjavik", 64.15, -21.94),
                ("Akureyri", 65.68, -18.09),
                ("Torshavn", 62.01, -6.77),
                ("Nuuk", 64.18, -51.72),
            ],
        );
        // Norway, Sweden and Finland: Gothenburg, Helsingborg, Malmö and Ystad across the Kattegat and the Sound.
        all(
            Region::Nordic,
            &[
                ("Bergen", 60.39, 5.32),
                ("Stavanger", 58.97, 5.73),
                ("Kristiansand", 58.15, 8.0),
                ("Oslo", 59.91, 10.75),
                ("Gothenburg", 57.71, 11.97),
                ("Helsingborg", 56.05, 12.69),
                ("Malmo", 55.6, 13.0),
                ("Ystad", 55.43, 13.82),
                ("Stockholm", 59.33, 18.07),
                ("Helsinki", 60.17, 24.94),
            ],
        );
        // The Low Countries, Flanders, north Germany and Denmark: Helsingør, Copenhagen and Dragør on the Sound's
        // Danish side, Skagen, Bornholm.
        all(
            Region::NorthSea,
            &[
                ("Den Helder", 52.96, 4.76),
                ("Amsterdam", 52.37, 4.9),
                ("Rotterdam", 51.92, 4.48),
                ("Antwerp", 51.22, 4.4),
                ("Ostend", 51.23, 2.92),
                ("Dunkirk", 51.03, 2.37),
                ("Wilhelmshaven", 53.52, 8.11),
                ("Hamburg", 53.55, 9.99),
                ("Kiel", 54.32, 10.14),
                ("Rostock", 54.09, 12.1),
                ("Esbjerg", 55.47, 8.45),
                ("Aarhus", 56.16, 10.2),
                ("Skagen", 57.72, 10.59),
                ("Helsingor", 56.03, 12.61),
                ("Copenhagen", 55.68, 12.57),
                ("Dragor", 55.59, 12.67),
                ("Ronne", 55.1, 14.7),
            ],
        );
        // France beyond Flanders, the Channel Islands, Berlin, Gdańsk.
        all(
            Region::Temperate,
            &[
                ("Paris", 48.86, 2.35),
                ("Cherbourg", 49.64, -1.62),
                ("Brest", 48.39, -4.49),
                ("Le Havre", 49.49, 0.11),
                ("St Peter Port", 49.45, -2.54),
                ("Berlin", 52.52, 13.4),
                ("Gdansk", 54.35, 18.65),
            ],
        );
        // Florida's peninsula and Keys; not Jacksonville, Pensacola, Havana or the Bahamas.
        all(
            Region::Florida,
            &[
                ("Miami", 25.77, -80.19),
                ("Miami Beach", 25.79, -80.13),
                ("Key West", 24.56, -81.78),
                ("Tampa", 27.95, -82.46),
                ("Fort Myers", 26.64, -81.87),
                ("West Palm Beach", 26.71, -80.05),
                ("Daytona Beach", 29.21, -81.02),
            ],
        );
        // The East Coast: Boston to Savannah, Jacksonville and the Gulf coast.
        all(
            Region::American,
            &[
                ("Boston", 42.36, -71.06),
                ("Newport", 41.49, -71.31),
                ("New York", 40.7, -74.0),
                ("Norfolk", 36.85, -76.29),
                ("Charleston", 32.78, -79.93),
                ("Savannah", 32.08, -81.09),
                ("Jacksonville", 30.33, -81.66),
                ("Pensacola", 30.42, -87.22),
            ],
        );
        all(
            Region::Mediterranean,
            &[
                ("Havana", 23.11, -82.37),
                ("Nassau", 25.05, -77.35),
                ("Freeport", 26.53, -78.7),
                ("Bimini", 25.73, -79.28),
            ],
        );
        assert_eq!(Region::at(41.0, 28.98, &chalk), Region::Mediterranean);
        assert_eq!(Region::at(43.3, 5.4, &chalk), Region::Mediterranean);
        assert_eq!(Region::at(-12.05, -77.04, &tropical), Region::Mediterranean);
        assert_eq!(Region::at(29.76, -95.37, &chalk), Region::American);
        assert_eq!(Region::at(40.7, -74.0, &chalk), Region::American);
        // Malta.
        all(
            Region::Malta,
            &[
                ("Valletta", 35.9, 14.51),
                ("Sliema", 35.91, 14.5),
                ("Victoria, Gozo", 36.04, 14.24),
                ("Marsaxlokk", 35.84, 14.54),
            ],
        );
        // Greece and Cyprus to the Evros and the Aegean's middle: Corfu against Sarandë, Florina against Bitola,
        // Serres against Petrich, Alexandroupoli and Orestiada against Enez, Ipsala and Edirne, the islands off
        // Turkey's coast against Ayvalık, Çeşme, Kuşadası, Bodrum, Datça, Marmaris and Fethiye.
        all(
            Region::Aegean,
            &[
                ("Athens", 37.98, 23.73),
                ("Piraeus", 37.94, 23.64),
                ("Aegina", 37.75, 23.43),
                ("Thessaloniki", 40.64, 22.94),
                ("Kavala", 40.94, 24.41),
                ("Alexandroupoli", 40.85, 25.87),
                ("Orestiada", 41.5, 26.53),
                ("Florina", 40.78, 21.41),
                ("Serres", 41.09, 23.55),
                ("Corfu", 39.62, 19.92),
                ("Ioannina", 39.66, 20.85),
                ("Patras", 38.25, 21.73),
                ("Zakynthos", 37.78, 20.9),
                ("Heraklion", 35.34, 25.13),
                ("Gavdos", 34.84, 24.08),
                ("Myrina, Lemnos", 39.88, 25.07),
                ("Mytilene", 39.11, 26.555),
                ("Chios", 38.37, 26.14),
                ("Vathy, Samos", 37.76, 26.98),
                ("Kos", 36.89, 27.29),
                ("Symi", 36.61, 27.84),
                ("Rhodes", 36.43, 28.22),
                ("Kastellorizo", 36.15, 29.59),
                ("Mykonos", 37.45, 25.33),
                ("Santorini", 36.42, 25.43),
                ("Limassol", 34.68, 33.04),
                ("Nicosia", 35.17, 33.36),
                ("Famagusta", 35.12, 33.94),
            ],
        );
        // North Africa: Tangier across the Strait from Tarifa, Nador beside Melilla, Algiers, Bizerte, Tunis, Libya,
        // Egypt's desert coast, Siwa, the Red Sea's towns, Suez, Sinai to El Arish and Sharm el-Sheikh.
        all(
            Region::NorthAfrica,
            &[
                ("Tangier", 35.78, -5.81),
                ("Tetouan", 35.57, -5.37),
                ("Nador", 35.17, -2.93),
                ("Casablanca", 33.59, -7.62),
                ("Rabat", 34.02, -6.84),
                ("Agadir", 30.42, -9.6),
                ("Tan-Tan", 28.44, -11.1),
                ("Dakhla", 23.7, -15.94),
                ("Oran", 35.7, -0.63),
                ("Algiers", 36.75, 3.06),
                ("Annaba", 36.9, 7.76),
                ("Bizerte", 37.27, 9.87),
                ("Tunis", 36.8, 10.18),
                ("Kelibia", 36.85, 11.1),
                ("Sfax", 34.74, 10.76),
                ("Tripoli", 32.89, 13.19),
                ("Benghazi", 32.12, 20.07),
                ("Tobruk", 32.08, 23.98),
                ("Marsa Matruh", 31.35, 27.24),
                ("El Alamein", 30.83, 28.95),
                ("Siwa", 29.2, 25.52),
                ("Suez", 29.97, 32.55),
                ("Hurghada", 27.26, 33.81),
                ("Marsa Alam", 25.07, 34.89),
                ("Sharm el-Sheikh", 27.91, 34.33),
                ("Nuweiba", 29.03, 34.66),
                ("El Arish", 31.13, 33.8),
            ],
        );
        // The Nile: Alexandria, Port Said, the delta's towns, Cairo, the Fayoum and the valley to Aswan.
        all(
            Region::Nile,
            &[
                ("Alexandria", 31.2, 29.92),
                ("Port Said", 31.26, 32.3),
                ("Damietta", 31.42, 31.81),
                ("Tanta", 30.79, 31.0),
                ("Mansoura", 31.04, 31.38),
                ("Cairo", 30.04, 31.24),
                ("Giza", 30.01, 31.21),
                ("Faiyum", 29.31, 30.84),
                ("Minya", 28.1, 30.75),
                ("Asyut", 27.18, 31.18),
                ("Sohag", 26.56, 31.7),
                ("Nag Hammadi", 26.05, 32.24),
                ("Qena", 26.16, 32.72),
                ("Luxor", 25.69, 32.64),
                ("Aswan", 24.09, 32.9),
            ],
        );
        // Europe's shore of the Strait and the Spanish towns on Africa's, Sicily and the islands off Tunisia, Madeira,
        // the Turkish coast across from Greece's islands, Istanbul.
        all(
            Region::Mediterranean,
            &[
                ("Ceuta", 35.89, -5.32),
                ("Melilla", 35.29, -2.94),
                ("Tarifa", 36.01, -5.6),
                ("Gibraltar", 36.14, -5.35),
                ("Algeciras", 36.13, -5.45),
                ("Almeria", 36.84, -2.46),
                ("Cartagena", 37.6, -0.98),
                ("Marsala", 37.8, 12.44),
                ("Gela", 37.07, 14.25),
                ("Pantelleria", 36.83, 11.95),
                ("Lampedusa", 35.5, 12.6),
                ("Funchal", 32.65, -16.91),
                ("Naples", 40.84, 14.25),
                ("Istanbul", 41.0, 28.98),
                ("Edirne", 41.68, 26.56),
                ("Enez", 40.72, 26.08),
                ("Ipsala", 40.92, 26.38),
                ("Canakkale", 40.15, 26.41),
                ("Bozcaada", 39.83, 26.04),
                ("Ayvalik", 39.32, 26.69),
                ("Cesme", 38.32, 26.3),
                ("Izmir", 38.42, 27.14),
                ("Kusadasi", 37.86, 27.26),
                ("Bodrum", 37.03, 27.43),
                ("Datca", 36.73, 27.69),
                ("Marmaris", 36.85, 28.27),
                ("Fethiye", 36.62, 29.11),
                ("Kas", 36.2, 29.64),
                ("Antalya", 36.9, 30.7),
                ("Sarande", 39.87, 20.0),
                ("Vlore", 40.47, 19.49),
                ("Bitola", 41.03, 21.33),
                ("Petrich", 41.4, 23.21),
            ],
        );
        // The Levant, Arabia and the Canaries stay arid.
        all(
            Region::Arid,
            &[
                ("Gaza", 31.5, 34.45),
                ("Eilat", 29.55, 34.95),
                ("Aqaba", 29.53, 35.0),
                ("Haql", 29.29, 34.94),
                ("Duba", 27.35, 35.69),
                ("Haifa", 32.82, 34.99),
                ("Beirut", 33.89, 35.5),
                ("Las Palmas", 28.1, -15.43),
                ("Arrecife", 28.96, -13.55),
            ],
        );
        // Hawaii's islands; Guam, Saipan, Palau and the Carolines, Wake and Midway; the Marshalls, Kiribati, Nauru,
        // Tuvalu, Tokelau and the Line Islands; the Solomons (Malaita, Makira and Temotu too), New Guinea and its
        // islands, the Philippines, the Moluccas, Fiji, Samoa, Vanuatu and Tahiti tropical.
        let tropics = |region: Region, places: &[(&str, f64, f64)]| {
            for &(name, lat, lon) in places {
                assert_eq!(
                    Region::at(lat, lon, &tropical),
                    region,
                    "{name} ({lat}, {lon})"
                );
            }
        };
        tropics(
            Region::Hawaii,
            &[
                ("Honolulu", 21.3, -157.86),
                ("Waikiki", 21.28, -157.83),
                ("Pearl City", 21.39, -157.97),
                ("Kailua", 21.4, -157.74),
                ("Lihue", 21.98, -159.37),
                ("Hanalei", 22.2, -159.5),
                ("Lahaina", 20.88, -156.68),
                ("Kahului", 20.89, -156.47),
                ("Kaunakakai", 21.09, -157.02),
                ("Lanai City", 20.83, -156.92),
                ("Hilo", 19.72, -155.08),
                ("Kailua-Kona", 19.64, -155.99),
            ],
        );
        tropics(
            Region::Micronesia,
            &[
                ("Hagatna", 13.48, 144.75),
                ("Tamuning", 13.49, 144.78),
                ("Apra Harbor", 13.44, 144.66),
                ("Garapan", 15.21, 145.72),
                ("San Jose, Tinian", 14.97, 145.63),
                ("Rota", 14.14, 145.16),
                ("Koror", 7.34, 134.48),
                ("Peleliu", 7.0, 134.24),
                ("Colonia, Yap", 9.51, 138.13),
                ("Ulithi", 10.02, 139.71),
                ("Weno, Chuuk", 7.45, 151.85),
                ("Kolonia, Pohnpei", 6.96, 158.21),
                ("Kapingamarangi", 1.07, 154.78),
                ("Lelu, Kosrae", 5.33, 163.01),
                ("Wake", 19.29, 166.62),
                ("Midway", 28.21, -177.38),
            ],
        );
        tropics(
            Region::Atoll,
            &[
                ("Betio", 1.35, 172.93),
                ("Bairiki", 1.33, 172.98),
                ("Bonriki", 1.38, 173.15),
                ("Butaritari", 3.07, 172.79),
                ("Majuro", 7.09, 171.38),
                ("Kwajalein", 8.72, 167.73),
                ("Ebeye", 8.78, 167.74),
                ("Roi-Namur", 9.4, 167.47),
                ("Jaluit", 5.92, 169.64),
                ("Enewetak", 11.35, 162.35),
                ("Nauru", -0.53, 166.93),
                ("Funafuti", -8.52, 179.2),
                ("Fakaofo, Tokelau", -9.38, -171.22),
                ("Kanton", -2.81, -171.68),
                ("Kiritimati", 1.87, -157.4),
                ("Tabuaeran", 3.86, -159.36),
            ],
        );
        tropics(
            Region::Tropical,
            &[
                ("Honiara", -9.43, 159.95),
                ("Auki, Malaita", -8.77, 160.7),
                ("Kirakira, Makira", -10.45, 161.92),
                ("Lata, Temotu", -10.73, 165.83),
                ("Rabaul", -4.2, 152.17),
                ("Buka", -5.43, 154.67),
                ("Kavieng", -2.57, 150.8),
                ("Lae", -6.73, 147.0),
                ("Port Moresby", -9.48, 147.15),
                ("Jayapura", -2.53, 140.7),
                ("Tacloban", 11.24, 125.0),
                ("Manila", 14.6, 120.98),
                ("Davao", 7.07, 125.6),
                ("Morotai", 2.3, 128.4),
                ("Suva", -18.14, 178.44),
                ("Apia", -13.83, -171.76),
                ("Port Vila", -17.74, 168.31),
                ("Papeete", -17.53, -149.57),
            ],
        );
        // The American West: Monterey, Los Angeles, San Diego, Phoenix, Las Vegas, Salt Lake City, Albuquerque; not
        // Tijuana or Nogales (Mexico's), Seattle or Portland (the Pacific north-west's siding), Denver (the Front Range).
        for (lat, lon) in [
            (36.6, -121.9),
            (33.86, -118.4),
            (32.72, -117.16),
            (33.45, -112.07),
            (36.17, -115.14),
            (40.76, -111.89),
            (35.08, -106.65),
        ] {
            assert_eq!(Region::at(lat, lon, &chalk), Region::West, "{lat} {lon}");
        }
        assert_eq!(Region::at(32.52, -117.04, &chalk), Region::Mediterranean);
        assert_eq!(Region::at(31.3, -110.94, &chalk), Region::Mediterranean);
        assert_eq!(Region::at(47.6, -122.33, &chalk), Region::American);
        assert_eq!(Region::at(45.52, -122.68, &chalk), Region::American);
        assert_eq!(Region::at(39.74, -104.99, &chalk), Region::American);
        // El Paso is American soil, Ciudad Juárez across the river Mexico's; so down the Rio Grande: Laredo, McAllen
        // and Brownsville American, Reynosa, Matamoros and Monterrey Mexico's.
        assert_eq!(Region::at(31.8, -106.44, &chalk), Region::West);
        assert_eq!(Region::at(31.69, -106.42, &chalk), Region::Mediterranean);
        for (lat, lon) in [
            (27.55, -99.48),
            (26.2034, -98.23),
            (25.9017, -97.4975),
            (29.42, -98.49),
        ] {
            assert_eq!(
                Region::at(lat, lon, &chalk),
                Region::American,
                "{lat} {lon}"
            );
        }
        for (lat, lon) in [
            (26.05, -98.3),
            (25.85, -97.5),
            (25.67, -100.31),
            (27.45, -99.52),
        ] {
            assert_eq!(
                Region::at(lat, lon, &chalk),
                Region::Mediterranean,
                "{lat} {lon}"
            );
        }
        // East Asia: Tokyo, Kamakura, Yokosuka, Sapporo, Naha, Busan, Taipei, Shanghai; not Vladivostok.
        for (lat, lon) in [
            (35.68, 139.7),
            (35.32, 139.55),
            (35.28, 139.67),
            (43.06, 141.35),
            (26.21, 127.68),
            (35.18, 129.08),
            (25.03, 121.57),
            (31.23, 121.47),
        ] {
            assert_eq!(
                Region::at(lat, lon, &chalk),
                Region::EastAsia,
                "{lat} {lon}"
            );
        }
        assert_ne!(Region::at(43.12, 131.9, &chalk), Region::EastAsia);
    }
}
