//! The old towns' extents in the period (native/docs/terrain/period.md, "Old towns"): where a city of the time
//! stood, from period maps, censuses and city histories. The period's thinning keeps a town's ground as town inside
//! its extent and keeps its buildings by the region's floor alone ([`super::thinning::core_floor`]: all but the
//! thinnest ground, a park's or a cemetery's), and takes what the extent keeps out of the rest of its 5' cell's share
//! ([`super::thinning::bar`]), so the old city stands where it stood and the cell's total stays near the history's
//! where it can.
//!
//! Today's footprints alone cannot tell an old quarter from a post-war one: Istanbul's walled city and its 1960s
//! districts are as dense as each other, and the 1940s grid of Los Angeles is as covered as its 1950s tracts; and the
//! built-up history (HYDE) puts most of a 9 km cell's people where the cell's land is densest today. So the list names
//! the towns whose extent the history and the footprints lose (`report_old_cities` in `naval-client`'s buildings tests
//! measures the share kept in each).

use super::CHUNK_M;
use super::thinning::{KEPT_BORDER, KEPT_CELL_M, Kept};

/// A town's ground on the chart: a circle (latitude, longitude, radius in m) or a polygon (its corners' latitudes and
/// longitudes, either way about).
#[derive(Clone, Copy, Debug)]
pub enum Shape {
    Circle(f64, f64, f32),
    Polygon(&'static [(f64, f64)]),
}

/// A town of the period: its name, the year it stood to this extent from, its ground, and ground within it that stood
/// open (a fire's cleared quarter), which is thinned as any land.
#[derive(Clone, Copy, Debug)]
pub struct Town {
    pub name: &'static str,
    pub year: u16,
    pub shapes: &'static [Shape],
    pub open: &'static [Shape],
}

const fn town(name: &'static str, year: u16, shapes: &'static [Shape]) -> Town {
    Town {
        name,
        year,
        shapes,
        open: &[],
    }
}

use Shape::{Circle, Polygon};

/// The towns of the period whose extents stand.
///
/// Sources (2026-10-10) are given with each city; corners on the water are pushed out to sea (the sea holds no
/// buildings), so only a town's landward edge is placed, about ±150 m unless its comment says otherwise.
pub const TOWNS: &[Town] = &[
    // Istanbul, 1940: 793,949 people in the city (the census of 20 October 1940). The built city is that of Jacques
    // Pervititch's insurance maps (1922-45: plates of Fatih, Eminönü, Galata, Pera, Şişli, Beşiktaş, Üsküdar, Kadıköy
    // and Moda) and the Alman Mavileri (1913-14: the walled city, Eyüp, Beyoğlu, Beşiktaş, Kadıköy and Üsküdar), as the
    // Istanbul Encyclopedia's (İstanbul Ansiklopedisi) and Wikipedia's district histories describe them: the walled city
    // to the Theodosian land walls (OpenStreetMap's `historic=citywalls`); Galata, Pera and Tophane, Kasımpaşa,
    // Kurtuluş and Feriköy, Pangaltı and Şişli to the Şişli mosque's square (Mecidiyeköy and Okmeydanı still fields),
    // Nişantaşı, Maçka, Beşiktaş and Ortaköy; Hasköy and Eyüp on the Golden Horn; Üsküdar, Harem and Selimiye;
    // Haydarpaşa, Kadıköy, Yeldeğirmeni and Moda to Kalamış; the Bosphorus villages, Bakırköy (Makriköy), Yeşilköy (San
    // Stefano) and the Princes' Islands' towns, each a circle about its pier (Nominatim). The coasts are OpenStreetMap's
    // (`natural=coastline`) pushed 100 m out; Yenikapı's and Kennedy Avenue's fill lie inside the walled city's
    // polygon and hold no buildings. The fires of 1908-18 (Cibali, Fatih, Aksaray: some 7,500 houses in 1918) left
    // burned ground in the walled city into the 1940s, but no source found places it, so none of it is left open.
    town(
        "Istanbul, the walled city",
        1910,
        &[Polygon(&[
            (40.9921, 28.9225),
            (40.9942, 28.9221),
            (41.0000, 28.9205),
            (41.0059, 28.9216),
            (41.0141, 28.9221),
            (41.0192, 28.9244),
            (41.0242, 28.9291),
            (41.0287, 28.9340),
            (41.0306, 28.9356),
            (41.0337, 28.9403),
            (41.0390, 28.9413),
            (41.0422, 28.9432),
            (41.0414, 28.9457),
            (41.0357, 28.9503),
            (41.0319, 28.9517),
            (41.0300, 28.9538),
            (41.0291, 28.9583),
            (41.0203, 28.9669),
            (41.0205, 28.9700),
            (41.0176, 28.9771),
            (41.0181, 28.9874),
            (41.0101, 28.9890),
            (41.0053, 28.9865),
            (41.0010, 28.9783),
            (40.9995, 28.9650),
            (40.9975, 28.9540),
            (40.9953, 28.9462),
            (40.9965, 28.9397),
            (40.9935, 28.9320),
            (40.9905, 28.9275),
            (40.9900, 28.9240),
        ])],
    ),
    town(
        "Istanbul, Galata to Şişli and Ortaköy",
        1910,
        &[Polygon(&[
            (41.0328, 28.9599),
            (41.0395, 28.9640),
            (41.0420, 28.9700),
            (41.0440, 28.9745),
            (41.0475, 28.9770),
            (41.0530, 28.9785),
            (41.0600, 28.9790),
            (41.0640, 28.9860),
            (41.0630, 28.9910),
            (41.0570, 28.9935),
            (41.0545, 28.9985),
            (41.0500, 29.0030),
            (41.0475, 29.0080),
            (41.0470, 29.0130),
            (41.0490, 29.0200),
            (41.0505, 29.0260),
            (41.0510, 29.0325),
            (41.0492, 29.0338),
            (41.0468, 29.0285),
            (41.0367, 28.9973),
            (41.0347, 28.9949),
            (41.0322, 28.9954),
            (41.0320, 28.9930),
            (41.0275, 28.9894),
            (41.0203, 28.9774),
            (41.0216, 28.9694),
            (41.0246, 28.9660),
            (41.0300, 28.9654),
        ])],
    ),
    town(
        "Istanbul, the Golden Horn's towns",
        1910,
        &[
            // Eyüp about its mosque and pier; Hasköy.
            Circle(41.0470, 28.9355, 450.0),
            Circle(41.0425, 28.9520, 400.0),
        ],
    ),
    town(
        "Istanbul, Üsküdar",
        1910,
        &[Polygon(&[
            (41.0333, 29.0221),
            (41.0340, 29.0265),
            (41.0300, 29.0310),
            (41.0240, 29.0360),
            (41.0200, 29.0350),
            (41.0190, 29.0270),
            (41.0150, 29.0210),
            (41.0100, 29.0200),
            (41.0050, 29.0170),
            (41.0030, 29.0110),
            (41.0050, 29.0055),
            (41.0110, 29.0050),
            (41.0222, 29.0045),
            (41.0269, 29.0115),
            (41.0306, 29.0168),
        ])],
    ),
    town(
        "Istanbul, Kadıköy and Moda",
        1910,
        &[Polygon(&[
            (41.0030, 29.0110),
            (41.0035, 29.0215),
            (41.0000, 29.0320),
            (40.9945, 29.0370),
            (40.9895, 29.0400),
            (40.9845, 29.0405),
            (40.9800, 29.0390),
            (40.9787, 29.0367),
            (40.9813, 29.0336),
            (40.9788, 29.0299),
            (40.9774, 29.0226),
            (40.9810, 29.0187),
            (40.9850, 29.0191),
            (40.9926, 29.0137),
            (40.9998, 29.0148),
            (41.0001, 29.0110),
        ])],
    ),
    town(
        "Istanbul, the Bosphorus villages",
        1910,
        &[
            // The European shore: Kuruçeşme, Arnavutköy, Bebek, Rumelihisarı, Emirgan, İstinye, Yeniköy, Tarabya,
            // Büyükdere, Sarıyer.
            Circle(41.0575, 29.0370, 300.0),
            Circle(41.0670, 29.0430, 400.0),
            Circle(41.0775, 29.0430, 450.0),
            Circle(41.0870, 29.0555, 400.0),
            Circle(41.1030, 29.0545, 350.0),
            Circle(41.1139, 29.0590, 400.0),
            Circle(41.1217, 29.0700, 400.0),
            Circle(41.1393, 29.0550, 400.0),
            Circle(41.1608, 29.0450, 450.0),
            Circle(41.1672, 29.0565, 400.0),
            // The Asian shore: Kuzguncuk, Beylerbeyi, Çengelköy, Vaniköy, Kandilli, Anadoluhisarı, Kanlıca, Çubuklu,
            // Paşabahçe, Beykoz.
            Circle(41.0369, 29.0310, 350.0),
            Circle(41.0450, 29.0460, 400.0),
            Circle(41.0510, 29.0530, 400.0),
            Circle(41.0635, 29.0590, 300.0),
            Circle(41.0745, 29.0595, 350.0),
            Circle(41.0832, 29.0680, 350.0),
            Circle(41.1003, 29.0665, 400.0),
            Circle(41.1085, 29.0840, 300.0),
            Circle(41.1167, 29.0940, 400.0),
            Circle(41.1335, 29.0935, 450.0),
        ],
    ),
    town(
        "Istanbul, Bakırköy, Yeşilköy and the Islands",
        1910,
        &[
            Circle(40.9770, 28.8740, 700.0),
            Circle(40.9585, 28.8230, 500.0),
            // Büyükada, Heybeliada, Burgazada, Kınalıada.
            Circle(40.8730, 29.1270, 600.0),
            Circle(40.8765, 29.0990, 450.0),
            Circle(40.8810, 29.0690, 300.0),
            Circle(40.9100, 29.0545, 300.0),
        ],
    ),
    // Reykjavík, 1940: 38,196 people (Statistics Iceland). The town stood within Hringbraut, the ring road of the 1927
    // plan (Guðjón Samúelsson's), from Vesturbær and the old harbour (1913-17) to Skúlagata, with Norðurmýri east of
    // Snorrabraut to Rauðarárstígur and Miklabraut (built 1930-42, as Wikipedia's district history has it) and the
    // National Hospital's (1930) frontage just south of the road; Melar, the Teigar and Laugarnes were built in the
    // 1940s and after. The roads are OpenStreetMap's, the edge 50-80 m beyond them; the harbour's side crosses the
    // basin, leaving out Örfirisey.
    town(
        "Reykjavík",
        1930,
        &[Polygon(&[
            (64.1523, -21.9627),
            (64.1531, -21.9576),
            (64.1544, -21.9559),
            (64.1548, -21.9480),
            (64.1540, -21.9400),
            (64.1522, -21.9313),
            (64.1508, -21.9289),
            (64.1475, -21.9187),
            (64.1475, -21.9121),
            (64.1462, -21.9100),
            (64.1440, -21.9115),
            (64.1410, -21.9135),
            (64.1385, -21.9155),
            (64.1355, -21.9160),
            (64.1352, -21.9190),
            (64.1352, -21.9236),
            (64.1351, -21.9279),
            (64.1352, -21.9320),
            (64.1358, -21.9352),
            (64.1366, -21.9375),
            (64.1380, -21.9408),
            (64.1409, -21.9468),
            (64.1430, -21.9505),
            (64.1493, -21.9605),
            (64.1505, -21.9625),
        ])],
    ),
    // Yokohama, 1940: 968,091 people (the census). Its old city, the 5.4 km² of 1889 (Yokohama City's history): Kannai
    // between the Ōoka and Nakamura rivers and the harbour, Kangai beyond them (Isezakichō, Yoshida, Hatsunechō to
    // Koganechō and Bandōbashi), Noge under Nogeyama, Shinkō's warehouses (1899-1914) and the Bluff (Yamate) with
    // Motomachi; rebuilt after the earthquake of 1923 by 1930, and burned in the raid of 29 May 1945. The rivers are
    // placed by their stations (Nominatim), so the edge is about ±200 m; the harbour's side leaves out Minato Mirai
    // (Mitsubishi's yard then), Ōsanbashi and the post-war Yamashita pier.
    town(
        "Yokohama, Kannai, Kangai and the Bluff",
        1930,
        &[Polygon(&[
            (35.4530, 139.6285),
            (35.4545, 139.6345),
            (35.4560, 139.6420),
            (35.4515, 139.6470),
            (35.4475, 139.6520),
            (35.4440, 139.6545),
            (35.4410, 139.6590),
            (35.4370, 139.6600),
            (35.4320, 139.6540),
            (35.4340, 139.6440),
            (35.4365, 139.6330),
            (35.4362, 139.6235),
            (35.4385, 139.6205),
            (35.4440, 139.6205),
            (35.4490, 139.6235),
            (35.4520, 139.6260),
        ])],
    ),
];

/// One of [`TOWNS`]' shapes on the theatre: a circle (middle, radius) or a polygon (its corners), in metres.
#[derive(Clone, Debug)]
enum Local {
    Circle([f64; 2], f64),
    Ring(Vec<[f64; 2]>),
}

/// A shape on the theatre and its bounds (least and most x and z), and whether it is a town's open ground.
#[derive(Clone, Debug)]
struct Placed {
    shape: Local,
    lo: [f64; 2],
    hi: [f64; 2],
    open: bool,
}

impl Placed {
    fn holds(&self, p: [f64; 2]) -> bool {
        if p[0] < self.lo[0] || p[0] > self.hi[0] || p[1] < self.lo[1] || p[1] > self.hi[1] {
            return false;
        }
        match &self.shape {
            Local::Circle(c, r) => (p[0] - c[0]).hypot(p[1] - c[1]) <= *r,
            Local::Ring(ring) => {
                let mut inside = false;
                let mut j = ring.len() - 1;
                for i in 0..ring.len() {
                    let (a, b) = (ring[i], ring[j]);
                    if (a[1] > p[1]) != (b[1] > p[1])
                        && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
                    {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
        }
    }

    fn reaches(&self, lo: [f64; 2], hi: [f64; 2]) -> bool {
        self.lo[0] <= hi[0] && lo[0] <= self.hi[0] && self.lo[1] <= hi[1] && lo[1] <= self.hi[1]
    }
}

/// The towns standing in a year, placed on a theatre ([`TOWNS`]).
#[derive(Clone, Debug, Default)]
pub struct Towns {
    placed: Vec<Placed>,
}

impl Towns {
    /// The towns standing in `year` (from their years) on the theatre, `local` a latitude and longitude's theatre point
    /// (m), those within `reach_m` of the theatre's origin.
    pub fn new(year: u16, local: &dyn Fn(f64, f64) -> [f64; 2], reach_m: f64) -> Self {
        Self::of(TOWNS, year, local, reach_m)
    }

    /// [`Self::new`] over `towns`.
    pub fn of(
        towns: &[Town],
        year: u16,
        local: &dyn Fn(f64, f64) -> [f64; 2],
        reach_m: f64,
    ) -> Self {
        let mut placed = Vec::new();
        for town in towns.iter().filter(|t| year >= t.year) {
            for (shapes, open) in [(town.shapes, false), (town.open, true)] {
                for shape in shapes {
                    let (shape, lo, hi) = match *shape {
                        Shape::Circle(lat, lon, r) => {
                            let c = local(lat, lon);
                            let r = f64::from(r);
                            (
                                Local::Circle(c, r),
                                [c[0] - r, c[1] - r],
                                [c[0] + r, c[1] + r],
                            )
                        }
                        Shape::Polygon(corners) => {
                            let ring: Vec<[f64; 2]> =
                                corners.iter().map(|&(lat, lon)| local(lat, lon)).collect();
                            let mut lo = [f64::INFINITY; 2];
                            let mut hi = [f64::NEG_INFINITY; 2];
                            for p in &ring {
                                for a in 0..2 {
                                    lo[a] = lo[a].min(p[a]);
                                    hi[a] = hi[a].max(p[a]);
                                }
                            }
                            (Local::Ring(ring), lo, hi)
                        }
                    };
                    let near = |a: usize| lo[a] < reach_m && hi[a] > -reach_m;
                    if near(0) && near(1) && lo.iter().chain(&hi).all(|v| v.is_finite()) {
                        placed.push(Placed {
                            shape,
                            lo,
                            hi,
                            open,
                        });
                    }
                }
            }
        }
        Self { placed }
    }

    /// Whether no town stands about the theatre.
    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// The towns' shapes that reach the chunk whose corner is theatre point `corner` or its border (for asking about
    /// many points in it, [`Self::cells`] too).
    pub fn about(&self, corner: [f64; 2]) -> Self {
        let border = KEPT_BORDER as f64 * f64::from(KEPT_CELL_M);
        let lo = [corner[0] - border, corner[1] - border];
        let hi = [corner[0] + CHUNK_M + border, corner[1] + CHUNK_M + border];
        Self {
            placed: self
                .placed
                .iter()
                .filter(|s| s.reaches(lo, hi))
                .cloned()
                .collect(),
        }
    }

    /// Whether theatre point `p` lies in a town's extent, not on its open ground.
    pub fn holds(&self, p: [f64; 2]) -> bool {
        self.placed.iter().any(|s| !s.open && s.holds(p))
            && !self.placed.iter().any(|s| s.open && s.holds(p))
    }

    /// The cells ([`Kept`]'s, 25 m) of the chunk whose corner is theatre point `corner`, and of its border, that lie in
    /// a town's extent ([`Self::holds`] at their middles); empty where none reaches them. A footprint over the chunk's
    /// edge may be the chunk's though its middle lies beyond it.
    pub fn cells(&self, corner: [f64; 2]) -> Kept {
        let mut kept = Kept::default();
        let border = KEPT_BORDER as f64 * f64::from(KEPT_CELL_M);
        let lo = [corner[0] - border, corner[1] - border];
        let hi = [corner[0] + CHUNK_M + border, corner[1] + CHUNK_M + border];
        let near: Vec<&Placed> = self.placed.iter().filter(|s| s.reaches(lo, hi)).collect();
        if !near.iter().any(|s| !s.open) {
            return kept;
        }
        kept.set_where_about(|p| {
            let t = [corner[0] + f64::from(p[0]), corner[1] + f64::from(p[1])];
            near.iter().any(|s| !s.open && s.holds(t)) && !near.iter().any(|s| s.open && s.holds(t))
        });
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chart about (`lat`, `lon`) in metres, east and north.
    fn chart(lat0: f64, lon0: f64) -> impl Fn(f64, f64) -> [f64; 2] {
        move |lat, lon| {
            [
                (lon - lon0) * 111_000.0 * lat0.to_radians().cos(),
                (lat - lat0) * 111_000.0,
            ]
        }
    }

    /// Each listed town's shapes are whole and plausible: on the globe, circles of 100 m to 3 km, polygons of three
    /// corners or more no wider than 15 km, its open ground within it; the names are each once.
    #[test]
    fn the_listed_towns_are_whole() {
        let mut names: Vec<&str> = TOWNS.iter().map(|t| t.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TOWNS.len(), "a name twice");
        let on_globe =
            |lat: f64, lon: f64| (-80.0..80.0).contains(&lat) && (-180.0..180.0).contains(&lon);
        for town in TOWNS {
            assert!(!town.shapes.is_empty(), "{}", town.name);
            assert!((1800..=1945).contains(&town.year), "{}", town.name);
            for shape in town.shapes.iter().chain(town.open) {
                match *shape {
                    Shape::Circle(lat, lon, r) => {
                        assert!(on_globe(lat, lon), "{}", town.name);
                        assert!((100.0..=3000.0).contains(&r), "{}: {r} m", town.name);
                    }
                    Shape::Polygon(corners) => {
                        assert!(corners.len() >= 3, "{}", town.name);
                        let (lat0, lon0) = corners[0];
                        let local = chart(lat0, lon0);
                        for &(lat, lon) in corners {
                            assert!(on_globe(lat, lon), "{}", town.name);
                            let p = local(lat, lon);
                            assert!(p[0].hypot(p[1]) < 15_000.0, "{}: ({lat}, {lon})", town.name);
                        }
                    }
                }
            }
            // Its open ground lies in it.
            let first = match town.shapes[0] {
                Shape::Circle(lat, lon, _) => (lat, lon),
                Shape::Polygon(c) => c[0],
            };
            let local = chart(first.0, first.1);
            let whole = Towns::of(&[Town { open: &[], ..*town }], 1945, &local, 1e7);
            let with_open = Towns::of(std::slice::from_ref(town), 1945, &local, 1e7);
            for shape in town.open {
                let at = match *shape {
                    Shape::Circle(lat, lon, _) => local(lat, lon),
                    Shape::Polygon(c) => local(c[0].0, c[0].1),
                };
                assert!(whole.holds(at), "{}: open ground outside it", town.name);
                assert!(!with_open.holds(at), "{}", town.name);
            }
        }
    }

    /// The walled city of Istanbul holds the Süleymaniye, Hagia Sophia and the Fatih mosque, not Zeytinburnu or
    /// Bayrampaşa (post-war districts outside the walls) or Mecidiyeköy; Galata, Üsküdar and Kadıköy stand.
    #[test]
    fn istanbuls_old_city_is_listed() {
        let local = chart(41.0165, 28.955);
        let towns = Towns::new(1942, &local, 1e6);
        for (name, lat, lon, stands) in [
            ("the Süleymaniye", 41.0162, 28.9639, true),
            ("Hagia Sophia", 41.0086, 28.9802, true),
            ("the Fatih mosque", 41.0195, 28.9497, true),
            ("Galata Tower", 41.0256, 28.9741, true),
            ("Kadıköy's pier", 40.9915, 29.0240, true),
            ("Üsküdar's Mihrimah mosque", 41.0268, 29.0160, true),
            ("Zeytinburnu", 40.9950, 28.9050, false),
            ("Bayrampaşa", 41.0450, 28.9100, false),
            ("Mecidiyeköy", 41.0670, 28.9950, false),
        ] {
            assert_eq!(towns.holds(local(lat, lon)), stands, "{name}");
        }
        assert!(Towns::new(1900, &local, 1e6).is_empty());
    }

    #[test]
    fn reykjaviks_and_yokohamas_old_towns_are_listed() {
        for (geo, places) in [
            (
                (64.147, -21.94),
                [
                    ("Hallgrímskirkja", 64.1418, -21.9267, true),
                    ("Austurvöllur", 64.1470, -21.9405, true),
                    ("Norðurmýri", 64.1391, -21.9187, true),
                    ("Örfirisey", 64.1590, -21.9420, false),
                    ("Melar's Hagamelur", 64.1430, -21.9600, false),
                    ("Breiðholt", 64.1050, -21.8500, false),
                ],
            ),
            (
                (35.445, 139.64),
                [
                    ("Isezakichō", 35.4430, 139.6300, true),
                    ("the Bluff's cemetery", 35.4397, 139.6519, true),
                    ("Yokohama Stadium", 35.4433, 139.6408, true),
                    ("Pacifico", 35.4588, 139.6365, false),
                    ("Yokohama station", 35.4660, 139.6223, false),
                    ("Honmoku", 35.4180, 139.6600, false),
                ],
            ),
        ] {
            let local = chart(geo.0, geo.1);
            let towns = Towns::new(1942, &local, 1e6);
            for (name, lat, lon, stands) in places {
                assert_eq!(towns.holds(local(lat, lon)), stands, "{name}");
            }
            assert!(Towns::new(1920, &local, 1e6).is_empty());
        }
    }
}
