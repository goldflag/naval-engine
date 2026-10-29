//! What of today's aerodromes and bridges a past period draws (native/docs/terrain/period.md, "The period's ground",
//! "The roads"): an aerodrome is drawn only from the year its paved runways first stood ([`FIELDS`]; in Iceland and the
//! Solomons none but those listed before 1946, [`FIELDLESS`]), its runways no longer and no wider than the period's
//! ([`runway_cap`], a field's own length and grounds where known), painted as they were then ([`RunwayPaint`]); a
//! bridge over the sea built after the period is left out ([`LATER_BRIDGES`]), and so is a span of [`LONG_SPAN_M`] or
//! more over the sea not listed as standing ([`LONG_SPANS`]). Places are latitude and longitude, so the host decides
//! with the theatre's frame; the sources are in the doc.

/// A place: its name, latitude and longitude, and how far about it a part belongs to it (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub name: &'static str,
    pub lat: f64,
    pub lon: f64,
    pub radius_m: f64,
}

impl Place {
    /// Whether (`lat`, `lon`) lies within its radius.
    pub fn holds(&self, lat: f64, lon: f64) -> bool {
        distance_m(self.lat, self.lon, lat, lon) <= self.radius_m
    }
}

/// The distance between two places (m): an equirectangular approximation, within a fraction of a percent over the few
/// kilometres it is asked about.
pub fn distance_m(lat0: f64, lon0: f64, lat1: f64, lon1: f64) -> f64 {
    const EARTH_M: f64 = 6_371_008.8;
    let mid = (0.5 * (lat0 + lat1)).to_radians();
    let dlon = ((lon1 - lon0 + 540.0).rem_euclid(360.0) - 180.0).to_radians();
    let x = dlon * mid.cos();
    let y = (lat1 - lat0).to_radians();
    EARTH_M * x.hypot(y)
}

/// An aerodrome whose history the period needs: its place; the year its paved runways first stood (nothing before
/// it); and until the year it `grew`, its runways no longer than `length_m` and within `grounds` (south, west, north,
/// east degrees) when given.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    pub place: Place,
    pub built: u16,
    pub grew: u16,
    pub length_m: f32,
    pub grounds: Option<[f64; 4]>,
}

const fn field(name: &'static str, lat: f64, lon: f64, radius_m: f64, built: u16) -> Field {
    Field {
        place: Place {
            name,
            lat,
            lon,
            radius_m,
        },
        built,
        grew: 0,
        length_m: f32::MAX,
        grounds: None,
    }
}

/// The aerodromes of the theatres' regions built (or first paved) after a period, and the few whose wartime size is
/// known. The first holding a place is its own (a runway built later on an older field is listed before the field).
pub const FIELDS: &[Field] = &[
    // Los Angeles. Mines Field was the eastern square mile of today's airport (Century Boulevard to Imperial Highway,
    // Sepulveda to Aviation Boulevard); its main runway was extended to about 4,600 ft in 1942, and the field went west
    // of Sepulveda after the war.
    Field {
        place: Place {
            name: "Mines Field (Los Angeles International)",
            lat: 33.9425,
            lon: -118.4081,
            radius_m: 3000.0,
        },
        built: 1929,
        grew: 1946,
        length_m: 1400.0,
        grounds: Some([33.9301, -118.3965, 33.9454, -118.3786]),
    },
    field("Los Alamitos", 33.7900, -118.0516, 2000.0, 1942),
    field(
        "Lomita Flight Strip (Torrance)",
        33.8034,
        -118.3396,
        1500.0,
        1943,
    ),
    field("El Toro", 33.6761, -117.7314, 3000.0, 1943),
    field(
        "Oxnard Flight Strip (Camarillo)",
        34.2137,
        -119.0944,
        1500.0,
        1942,
    ),
    field("Point Mugu", 34.1203, -119.1212, 2500.0, 1946),
    field("Whiteman", 34.2593, -118.4135, 1000.0, 1946),
    field(
        "Catalina's Airport in the Sky",
        33.4049,
        -118.4158,
        1200.0,
        1946,
    ),
    // Oahu: Honolulu's Reef Runway (08R/26L) on the reef, 14 October 1977.
    field("Honolulu's Reef Runway", 21.3068, -157.9282, 900.0, 1977),
    field("Barbers Point", 21.3073, -158.0703, 2500.0, 1942),
    field("Kaneohe", 21.4505, -157.7680, 1500.0, 1942),
    // Kanto: Haneda's runways all lie on land reclaimed from 1984 (the A runway 1988; C 1997, B 2000, D 2010); the
    // field of 1931-1945 (two runways of 800 by 80 m) lay at its western tip.
    field(
        "Tokyo International (Haneda)",
        35.5494,
        139.7798,
        3500.0,
        1988,
    ),
    field("Narita", 35.7647, 140.3864, 3500.0, 1978),
    field("Shimofusa", 35.7984, 140.0114, 1500.0, 1945),
    // Southern England: Heathrow's first paved runway at the end of 1945; Gatwick's in 1958 (grass and steel track in
    // the war); Eastleigh's in 1963; Thorney Island's concrete in 1942.
    field("Heathrow", 51.4700, -0.4543, 3500.0, 1946),
    field("Gatwick", 51.1481, -0.1903, 2500.0, 1958),
    field("Southampton (Eastleigh)", 50.9503, -1.3568, 1500.0, 1963),
    field("Thorney Island", 50.8144, -0.9206, 1500.0, 1942),
    // Iceland: Reykjavík's No. 1 runway finished on 4 June 1941; Keflavík's Meeks Field in 1943.
    field("Reykjavík", 64.1300, -21.9406, 1500.0, 1941),
    field("Keflavík (Meeks Field)", 63.9850, -22.6056, 3500.0, 1943),
    // The Solomons: Henderson Field 2,600 ft on 12 August 1942, 3,800 by 150 ft by September; Munda's Japanese strip
    // (about 1,000 m) usable in December 1942 (in 1942 and 1943 the bases' catalog's, generated: `infra::bases`), the
    // American field from August 1943; Segi and Barakoma in 1943.
    Field {
        place: Place {
            name: "Henderson Field (Honiara)",
            lat: -9.4282,
            lon: 160.0545,
            radius_m: 1800.0,
        },
        built: 1942,
        grew: 1943,
        length_m: 1160.0,
        grounds: None,
    },
    Field {
        place: Place {
            name: "Munda",
            lat: -8.3280,
            lon: 157.2631,
            radius_m: 1500.0,
        },
        built: 1943,
        grew: 1944,
        length_m: 1000.0,
        grounds: None,
    },
    field("Segi", -8.5786, 157.8761, 1500.0, 1943),
    field("Barakoma", -7.913, 156.708, 1500.0, 1943),
];

/// Regions where no aerodrome but those of [`FIELDS`] stood before a year (south, west, north, east degrees): Iceland
/// (Reykjavík's the first, 1940-1941) and the Solomons (Henderson Field the first, August 1942), until 1946.
pub const FIELDLESS: &[(&str, [f64; 4], u16)] = &[
    ("Iceland", [63.0, -25.0, 67.0, -13.0], 1946),
    ("the Solomon Islands", [-12.5, 154.0, -4.5, 170.5], 1946),
];

/// Bridges over the sea built after a period (the year they opened): neither the bridge nor its road is drawn in an
/// earlier one. Within its radius of the middle of a bridge's line.
pub const LATER_BRIDGES: &[(Place, u16)] = &[
    (
        Place {
            name: "Vincent Thomas Bridge, Los Angeles",
            lat: 33.7497,
            lon: -118.2716,
            radius_m: 1000.0,
        },
        1963,
    ),
    (
        Place {
            name: "Long Beach International Gateway (the Gerald Desmond Bridge, 1968, before it)",
            lat: 33.7634,
            lon: -118.2208,
            radius_m: 1300.0,
        },
        1968,
    ),
    (
        Place {
            name: "Admiral Clarey Bridge, Ford Island",
            lat: 21.3695,
            lon: -157.9426,
            radius_m: 800.0,
        },
        1998,
    ),
    (
        Place {
            name: "Yokohama Bay Bridge",
            lat: 35.4544,
            lon: 139.6747,
            radius_m: 600.0,
        },
        1989,
    ),
    (
        Place {
            name: "Tsurumi Tsubasa Bridge",
            lat: 35.475,
            lon: 139.696,
            radius_m: 600.0,
        },
        1994,
    ),
    (
        Place {
            name: "Rainbow Bridge, Tokyo",
            lat: 35.6364,
            lon: 139.7639,
            radius_m: 700.0,
        },
        1993,
    ),
    (
        Place {
            name: "Tokyo Gate Bridge",
            lat: 35.6025,
            lon: 139.8316,
            radius_m: 1400.0,
        },
        2012,
    ),
    (
        Place {
            name: "Tokyo Bay Aqua-Line's bridge",
            lat: 35.4481,
            lon: 139.8990,
            radius_m: 2300.0,
        },
        1997,
    ),
    (
        Place {
            name: "Itchen Bridge, Southampton",
            lat: 50.8988,
            lon: -1.3843,
            radius_m: 450.0,
        },
        1977,
    ),
    (
        Place {
            name: "Borgarfjarðarbrú, Borgarnes",
            lat: 64.5337,
            lon: -21.8926,
            radius_m: 400.0,
        },
        1981,
    ),
];

/// The least run over the sea (m) that makes a bridge a long span: the infrastructure's suspension bridge
/// (`bridges::profile`'s towers).
pub const LONG_SPAN_M: f64 = 300.0;

/// Bridges with a span of [`LONG_SPAN_M`] or more over the sea that stood from their year: no other such span is
/// drawn in a past period (in the theatres' regions none stood before 1946; these are the world's few).
pub const LONG_SPANS: &[(Place, u16)] = &[
    (place("Forth Bridge", 56.0006, -3.3884, 1500.0), 1890),
    (
        place("Hercílio Luz Bridge", -27.5935, -48.5655, 600.0),
        1926,
    ),
    (place("Carquinez Bridge", 38.0610, -122.2260, 1000.0), 1927),
    (place("Mount Hope Bridge", 41.6403, -71.2581, 1000.0), 1929),
    (place("Bayonne Bridge", 40.6420, -74.1420, 900.0), 1931),
    (
        place("Sydney Harbour Bridge", -33.8523, 151.2108, 900.0),
        1932,
    ),
    (
        place(
            "San Francisco-Oakland Bay Bridge",
            37.7983,
            -122.3778,
            4500.0,
        ),
        1936,
    ),
    (
        place("Golden Gate Bridge", 37.8199, -122.4783, 1500.0),
        1937,
    ),
    (place("Lions Gate Bridge", 49.3150, -123.1386, 1000.0), 1938),
    (
        place("Bronx-Whitestone Bridge", 40.8013, -73.8290, 1000.0),
        1939,
    ),
    // The first Tacoma Narrows Bridge fell on 7 November 1940; its successor opened in 1950.
    (
        place("Tacoma Narrows Bridge", 47.2690, -122.5517, 1200.0),
        1950,
    ),
];

const fn place(name: &'static str, lat: f64, lon: f64, radius_m: f64) -> Place {
    Place {
        name,
        lat,
        lon,
        radius_m,
    }
}

/// What a runway at a place is in a year: not there, or there no longer than `length_m` and no wider than twice
/// `half_m`, within `grounds` when given, painted `paint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FieldThen {
    None,
    Stood {
        length_m: f32,
        half_m: f32,
        grounds: Option<[f64; 4]>,
        paint: RunwayPaint,
    },
}

/// A runway's paint in a past period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunwayPaint {
    /// Plain pavement: wartime runways were mostly unmarked (and some camouflaged); the US standard for numbers,
    /// threshold and centre line marks came in 1948, ICAO's Annex 14 in 1951.
    Plain,
    /// Numbers, threshold bars, the centre line and the side stripes, but no touchdown zone or aiming point.
    Numbered,
}

/// The largest runway of `year` (length, half width; m): before the war 1,500 m; in it 2,150 m (Hickam's main runway,
/// 7,048 ft, the longest found; the RAF's Class A main runway of 1942 2,000 yd) by 46 m (150 ft); after it 2,750 m;
/// from 1953 3,400 m (the jets'), 60 m wide.
pub fn runway_cap(year: u16) -> (f32, f32) {
    match year {
        ..=1938 => (1500.0, 23.0),
        1939..=1945 => (2150.0, 23.0),
        1946..=1952 => (2750.0, 30.0),
        _ => (3400.0, 30.0),
    }
}

/// The paint of a runway in `year`.
pub fn runway_paint(year: u16) -> RunwayPaint {
    if year < 1948 {
        RunwayPaint::Plain
    } else {
        RunwayPaint::Numbered
    }
}

/// What a runway at (`lat`, `lon`) was in `year`.
pub fn field_then(year: u16, lat: f64, lon: f64) -> FieldThen {
    let (cap, half_m) = runway_cap(year);
    let paint = runway_paint(year);
    let stood = |length_m: f32, grounds| FieldThen::Stood {
        length_m: length_m.min(cap),
        half_m,
        grounds,
        paint,
    };
    if let Some(f) = FIELDS.iter().find(|f| f.place.holds(lat, lon)) {
        return if year < f.built {
            FieldThen::None
        } else if year < f.grew {
            stood(f.length_m, f.grounds)
        } else {
            stood(cap, None)
        };
    }
    let within = |b: &[f64; 4]| (b[0]..=b[2]).contains(&lat) && (b[1]..=b[3]).contains(&lon);
    if FIELDLESS
        .iter()
        .any(|(_, b, until)| year < *until && within(b))
    {
        return FieldThen::None;
    }
    stood(cap, None)
}

/// Whether a bridge over the water at (`lat`, `lon`), its longest run over the sea `sea_m`, stood in `year`: not one
/// built after it, nor a long span not listed as standing.
pub fn bridge_stood(year: u16, lat: f64, lon: f64, sea_m: f64) -> bool {
    if LATER_BRIDGES
        .iter()
        .any(|(p, built)| *built > year && p.holds(lat, lon))
    {
        return false;
    }
    sea_m < LONG_SPAN_M
        || LONG_SPANS
            .iter()
            .any(|(p, built)| *built <= year && p.holds(lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_metres() {
        // A minute of latitude is a nautical mile.
        let d = distance_m(34.0, -118.0, 34.0 + 1.0 / 60.0, -118.0);
        assert!((d - 1853.0).abs() < 5.0, "{d}");
        // Across the date line.
        let d = distance_m(-9.0, 179.999, -9.0, -179.999);
        assert!(d < 300.0, "{d}");
    }

    #[test]
    fn aerodromes_stand_from_their_year_at_the_periods_size() {
        // Keflavík: none in the Denmark Strait's 1941, from 1943 the period's largest.
        assert_eq!(field_then(1941, 63.9850, -22.62), FieldThen::None);
        assert_eq!(
            field_then(1944, 63.9850, -22.62),
            FieldThen::Stood {
                length_m: 2150.0,
                half_m: 23.0,
                grounds: None,
                paint: RunwayPaint::Plain
            }
        );
        // Reykjavík stands in 1941; Akureyri (not listed, in Iceland) not until 1946.
        assert!(matches!(
            field_then(1941, 64.13, -21.94),
            FieldThen::Stood { .. }
        ));
        assert_eq!(field_then(1941, 65.66, -18.07), FieldThen::None);
        assert!(matches!(
            field_then(1950, 65.66, -18.07),
            FieldThen::Stood { .. }
        ));
        // Henderson Field in August 1942: 3,800 ft; Munda not yet; the rest of the Solomons none.
        assert!(matches!(
            field_then(1942, -9.4282, 160.0545),
            FieldThen::Stood {
                length_m: 1160.0,
                ..
            }
        ));
        assert_eq!(field_then(1942, -8.328, 157.263), FieldThen::None);
        assert_eq!(field_then(1942, -10.45, 161.9), FieldThen::None);
        // Mines Field: its square mile, 1,400 m; a field nowhere listed (Burbank) the period's size; Haneda's
        // reclaimed runways not before 1988; the Reef Runway, not the rest of Honolulu's, after 1977.
        match field_then(1942, 33.9377, -118.4009) {
            FieldThen::Stood {
                length_m, grounds, ..
            } => {
                assert_eq!(length_m, 1400.0);
                assert!(grounds.is_some());
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            field_then(1942, 34.2007, -118.3587),
            FieldThen::Stood {
                length_m: 2150.0,
                ..
            }
        ));
        assert_eq!(field_then(1960, 35.5485, 139.7772), FieldThen::None);
        assert_eq!(field_then(1960, 21.3068, -157.9282), FieldThen::None);
        assert!(matches!(
            field_then(1941, 21.3214, -157.9166),
            FieldThen::Stood { .. }
        ));
        // Caps and paint by the year.
        assert_eq!(runway_cap(1936), (1500.0, 23.0));
        assert_eq!(runway_cap(1955), (3400.0, 30.0));
        assert_eq!(runway_paint(1945), RunwayPaint::Plain);
        assert_eq!(runway_paint(1952), RunwayPaint::Numbered);
    }

    #[test]
    fn bridges_built_later_and_unlisted_long_spans_are_left_out() {
        // The Vincent Thomas Bridge: not in 1942, standing from 1963 (its 457 m span over the sea).
        assert!(!bridge_stood(1942, 33.7497, -118.2716, 457.0));
        assert!(!bridge_stood(1962, 33.7497, -118.2716, 0.0));
        assert!(bridge_stood(1963, 33.7497, -118.2716, 0.0));
        // Yokohama Bay Bridge's lower deck (a trunk road) goes with it.
        assert!(!bridge_stood(1942, 35.4544, 139.6747, 0.0));
        // A short harbour bridge anywhere stands; a long span stands only where listed, from its year.
        assert!(bridge_stood(1942, 33.7771, -118.2412, 120.0));
        assert!(!bridge_stood(1942, 10.0, 10.0, 400.0));
        assert!(bridge_stood(1942, 37.8199, -122.4783, 1280.0));
        assert!(!bridge_stood(1936, 37.8199, -122.4783, 1280.0));
        assert!(!bridge_stood(1942, 47.2690, -122.5517, 853.0));
    }
}
