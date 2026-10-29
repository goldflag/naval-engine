use super::*;
use naval_earth::geo::to_local;
use naval_osm::{Features, TileId, Water};

const MONTEREY_BAY: LatLon = LatLon {
    lat: 36.936204,
    lon: -122.418494,
};

fn way(class: &str) -> Transportation<[f64; 2]> {
    Transportation {
        class: class.into(),
        subclass: None,
        brunnel: Brunnel::None,
        surface: None,
        layer: 0,
        ramp: false,
        oneway: 0,
        expressway: false,
        service: None,
        network: None,
        geometry: Geometry::Empty,
    }
}

#[test]
fn each_class_is_drawn_at_its_width_and_surface() {
    let paved = |c: &str| style_of(&way(c)).map(|(k, h, _)| (k, h));
    assert_eq!(paved("motorway"), Some((StrokeKind::Paved, 6.5)));
    assert_eq!(paved("minor"), Some((StrokeKind::Paved, 3.0)));
    assert_eq!(paved("track"), Some((StrokeKind::Unpaved, 1.6)));
    assert_eq!(paved("rail"), Some((StrokeKind::Rail, 0.0)));
    // Wider roads are darker.
    let tone = |c: &str| style_of(&way(c)).unwrap().2;
    assert!(tone("motorway") > tone("primary") && tone("primary") > tone("minor"));
    // Not drawn: footpaths, ferries, ways being built, trams, anything in a tunnel.
    for c in ["path", "ferry", "minor_construction", "aerialway", "pier"] {
        assert_eq!(style_of(&way(c)), None, "{c}");
    }
    let tram = Transportation {
        subclass: Some("tram".into()),
        ..way("transit")
    };
    assert_eq!(style_of(&tram), None);
    let tunnel = Transportation {
        brunnel: Brunnel::Tunnel,
        ..way("primary")
    };
    assert_eq!(style_of(&tunnel), None);
    // The surface turns a road unpaved, a track paved; a ramp is narrower.
    let dirt = Transportation {
        surface: Some(RoadSurface::Unpaved),
        ..way("tertiary")
    };
    assert_eq!(style_of(&dirt).map(|s| s.0), Some(StrokeKind::Unpaved));
    let sealed = Transportation {
        surface: Some(RoadSurface::Paved),
        ..way("track")
    };
    assert_eq!(style_of(&sealed).map(|s| s.0), Some(StrokeKind::Paved));
    let ramp = Transportation {
        ramp: true,
        ..way("motorway")
    };
    assert_eq!(style_of(&ramp).map(|s| s.1), Some(3.5));
    // Waterways by class; an intermittent stream is left out, an intermittent river narrower.
    let water = |class: &str, intermittent: bool| {
        waterway_style(&Waterway::<[f64; 2]> {
            class: class.into(),
            brunnel: Brunnel::None,
            intermittent,
            geometry: Geometry::Empty,
        })
    };
    assert_eq!(water("river", false), Some(8.0));
    assert_eq!(water("river", true), Some(4.0));
    assert_eq!(water("stream", true), None);
    assert_eq!(water("canal", false), Some(6.0));
    assert!(water_area_drawn("river", false) && water_area_drawn("dock", false));
    assert!(!water_area_drawn("lake", false) && !water_area_drawn("ocean", false));
}

#[test]
fn the_blocks_keep_the_camera_in_their_middle_and_the_grid_matches_the_theatres_pages() {
    for level in 0..LEVELS {
        let side = page_m(level);
        let at = [2.3 * side, -2.6 * side];
        let pages = block(level, at, SLOTS);
        assert_eq!(pages.len(), 64);
        assert_eq!(pages[0], page_of(level, at));
        let lo = pages.iter().map(|p| p[0]).min().unwrap() as f64 * side;
        let hi = (pages.iter().map(|p| p[0]).max().unwrap() + 1) as f64 * side;
        assert!(at[0] - lo >= 3.5 * side && hi - at[0] >= 3.5 * side);
    }
    assert_eq!(page_m(0), 512.0);
    assert_eq!(page_m(1), 2048.0);
    // A theatre page's chart corner is its grid corner, whatever the window's origin.
    let origin = [-37_123.4, 81_906.7];
    let (g, base) = grid(origin);
    for (level, base) in base.iter().enumerate() {
        let index = [-12, 22];
        let l = local(index, *base);
        let chart = [
            g.origin[level][0] + f64::from(l[0]) * page_m(level),
            g.origin[level][1] + f64::from(l[1]) * page_m(level),
        ];
        let (min, _) = page_rect(level, index);
        assert!((chart[0] + origin[0] - min[0]).abs() < 1e-6);
        assert!((chart[1] + origin[1] - min[1]).abs() < 1e-6);
    }
}

#[test]
fn a_page_holds_the_roads_and_rivers_of_its_tiles() {
    // A tile about Santa Cruz with a primary road across it from west to east at its middle, a railway down it, and a
    // riverbank square.
    let id = TileId::of(36.97, -122.03, 14);
    let mut features: Features<naval_osm::TilePoint> = Features::default();
    features.transportation.push(Transportation {
        geometry: Geometry::Lines(vec![vec![[-0.1, 0.5], [1.1, 0.5]]]),
        ..way("primary")
    });
    features.transportation.push(Transportation {
        geometry: Geometry::Lines(vec![vec![[0.3, -0.1], [0.3, 1.1]]]),
        ..way("rail")
    });
    features.water.push(Water {
        class: "river".into(),
        brunnel: Brunnel::None,
        intermittent: false,
        id: None,
        geometry: Geometry::Polygons(vec![naval_osm::Polygon {
            exterior: vec![[0.7, 0.7], [0.8, 0.7], [0.8, 0.8], [0.7, 0.8]],
            holes: vec![],
        }]),
    });
    let tile = Arc::new(OsmTile::new(MONTEREY_BAY, id, &features));
    // The level-0 page under the road's middle at the tile's middle.
    let road_mid = osm::project(MONTEREY_BAY, id, [0.5, 0.5]);
    let index = page_of(0, road_mid);
    let page = make_page(0, index, std::slice::from_ref(&tile), &Roads::today());
    assert!(!page.empty);
    let (min, _) = page_rect(0, index);
    let texel = |p: [f64; 2]| {
        (
            ((p[0] - min[0]) / TEXEL_M[0]) as u32,
            ((p[1] - min[1]) / TEXEL_M[0]) as u32,
        )
    };
    let (i, j) = texel(road_mid);
    let on_road = page.distance(i, j, StrokeKind::Paved, TEXEL_M[0]);
    assert!(on_road < -3.0, "{on_road}");
    let (i, j) = texel([road_mid[0], road_mid[1] + 12.0]);
    assert!(page.distance(i, j, StrokeKind::Paved, TEXEL_M[0]) > 6.0);
    // The railway's page: its middle line.
    let rail = osm::project(MONTEREY_BAY, id, [0.3, 0.5]);
    let page = make_page(
        0,
        page_of(0, rail),
        std::slice::from_ref(&tile),
        &Roads::today(),
    );
    let (min, _) = page_rect(0, page_of(0, rail));
    let (i, j) = (
        ((rail[0] - min[0]) / TEXEL_M[0]) as u32,
        ((rail[1] - min[1]) / TEXEL_M[0]) as u32,
    );
    assert!(page.distance(i, j, StrokeKind::Rail, TEXEL_M[0]).abs() < 1.5);
    // The riverbank, at the coarse level: inside it is water.
    let pool = osm::project(MONTEREY_BAY, id, [0.75, 0.75]);
    let index = page_of(1, pool);
    let page = make_page(1, index, std::slice::from_ref(&tile), &Roads::today());
    let (min, _) = page_rect(1, index);
    let (i, j) = (
        ((pool[0] - min[0]) / TEXEL_M[1]) as u32,
        ((pool[1] - min[1]) / TEXEL_M[1]) as u32,
    );
    assert!(page.distance(i, j, StrokeKind::Water, TEXEL_M[1]) < -50.0);
    // Far from it all: an empty page.
    assert!(
        make_page(
            0,
            [index[0] + 40, index[1]],
            std::slice::from_ref(&tile),
            &Roads::today()
        )
        .empty
    );
}

#[test]
fn nothing_is_asked_with_streaming_off() {
    assert!(!enabled());
    want(MONTEREY_BAY, Period::Today, &[(0, [0, 0])]);
    assert_eq!(status().queued, 0);
}

/// A tile about Los Angeles with one of each kind of way the period decides on: a motorway and its ramp, a primary, a
/// residential street, a driveway, a track, a primary on a bridge, a railway and a river.
fn fixture_tile() -> (Arc<OsmTile>, LatLon) {
    let geo = LatLon {
        lat: 34.0,
        lon: -118.3,
    };
    (fixture_tile_at(geo, 34.05, -118.25), geo)
}

/// The fixture's ways in the tile holding (`lat`, `lon`), in the frame about `geo`.
fn fixture_tile_at(geo: LatLon, lat: f64, lon: f64) -> Arc<OsmTile> {
    let id = TileId::of(lat, lon, 14);
    let mut features: Features<naval_osm::TilePoint> = Features::default();
    let line = |points: &[[f64; 2]]| Geometry::Lines(vec![points.to_vec()]);
    features.transportation.push(Transportation {
        geometry: line(&[[-0.05, 0.2], [0.5, 0.25], [1.05, 0.3]]),
        ..way("motorway")
    });
    features.transportation.push(Transportation {
        ramp: true,
        geometry: line(&[[0.5, 0.25], [0.55, 0.35]]),
        ..way("motorway")
    });
    features.transportation.push(Transportation {
        geometry: line(&[[0.1, -0.05], [0.12, 0.5], [0.1, 1.05]]),
        ..way("primary")
    });
    features.transportation.push(Transportation {
        geometry: line(&[[0.12, 0.6], [0.4, 0.62], [0.9, 0.6]]),
        ..way("minor")
    });
    features.transportation.push(Transportation {
        service: Some("driveway".into()),
        geometry: line(&[[0.4, 0.62], [0.41, 0.66]]),
        ..way("service")
    });
    features.transportation.push(Transportation {
        geometry: line(&[[0.6, 0.7], [0.8, 0.9]]),
        ..way("track")
    });
    features.transportation.push(Transportation {
        brunnel: Brunnel::Bridge,
        geometry: line(&[[0.7, 0.4], [0.8, 0.45]]),
        ..way("primary")
    });
    features.transportation.push(Transportation {
        geometry: line(&[[0.3, -0.05], [0.3, 1.05]]),
        ..way("rail")
    });
    features.waterway.push(Waterway {
        class: "river".into(),
        brunnel: Brunnel::None,
        intermittent: false,
        geometry: line(&[[0.95, -0.05], [0.9, 1.05]]),
    });
    Arc::new(OsmTile::new(geo, id, &features))
}

/// A page's bytes and lamps, hashed (FNV-1a).
fn page_hash(page: &FeaturePage) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
        }
    };
    eat(&page.edges);
    eat(&page.attrs);
    for lamp in &page.lamps {
        for x in [lamp.at[0], lamp.at[1], lamp.height] {
            eat(&x.to_le_bytes());
        }
        eat(&lamp.info.to_le_bytes());
    }
    eat(&[u8::from(page.empty)]);
    h
}

/// Every page of `level` the fixture's tile reaches, made by `make`, hashed and folded in order.
fn fixture_hash(
    level: usize,
    make: impl Fn(usize, [i64; 2], &[Arc<OsmTile>], LatLon) -> FeaturePage,
) -> (usize, u64) {
    let (tile, geo) = fixture_tile();
    let id = tile.id;
    let a = page_of(level, osm::project(geo, id, [0.0, 0.0]));
    let b = page_of(level, osm::project(geo, id, [1.0, 1.0]));
    let mut hashes = Vec::new();
    for j in a[1]..=b[1] {
        for i in a[0]..=b[0] {
            hashes.push(page_hash(&make(
                level,
                [i, j],
                std::slice::from_ref(&tile),
                geo,
            )));
        }
    }
    let all = hashes.iter().fold(0u64, |h, x| h.rotate_left(7) ^ x);
    (hashes.len(), all)
}

/// A page as the worker makes it: its strokes and, at the lamps' level, its lamps.
fn page_with_lamps(
    level: usize,
    index: [i64; 2],
    tiles: &[Arc<OsmTile>],
    geo: LatLon,
    roads: &Roads<'_>,
) -> FeaturePage {
    let mut page = make_page(level, index, tiles, roads);
    if level == naval_terrain::night::LAMP_LEVEL {
        page.lamps = super::super::lamps::page_lamps(geo, index, tiles, roads);
    }
    page
}

#[test]
fn todays_pages_are_as_they_were() {
    // Every page the fixture's tile reaches, its bytes and lamps, as master made them before the period's roads
    // (2026-10-10): today's roads are the period's rules' identity, whatever the ground's answers.
    let expected = [(25, 0x8557_c5b0_064e_8c07u64), (4, 0x4611_a88d_75cd_5c9e)];
    for (level, expected) in expected.into_iter().enumerate() {
        let today = fixture_hash(level, |level, index, tiles, geo| {
            page_with_lamps(level, index, tiles, geo, &Roads::today())
        });
        assert_eq!(today, expected, "level {level}");
        let nothing_stood = |_: [f64; 2]| false;
        let ignored = fixture_hash(level, |level, index, tiles, geo| {
            let roads = Roads::new(Period::Today, geo, [0.0, 0.0], &nothing_stood);
            page_with_lamps(level, index, tiles, geo, &roads)
        });
        assert_eq!(ignored, expected, "level {level}, the ground ignored");
    }
}

/// The San Fernando Valley (Van Nuys): a frame, and the fixture's ways in its tile.
const VALLEY: LatLon = LatLon {
    lat: 34.19,
    lon: -118.45,
};

/// The strokes of `tile`'s ways under `roads`, over the whole tile.
fn strokes(tile: &Arc<OsmTile>, geo: LatLon, roads: &Roads<'_>) -> Vec<Stroke> {
    let quad = tile.quad(geo);
    let min = [
        quad.iter().map(|p| p[0]).fold(f64::MAX, f64::min) - 200.0,
        quad.iter().map(|p| p[1]).fold(f64::MAX, f64::min) - 200.0,
    ];
    let max = [
        quad.iter().map(|p| p[0]).fold(f64::MIN, f64::max) + 200.0,
        quad.iter().map(|p| p[1]).fold(f64::MIN, f64::max) + 200.0,
    ];
    shapes(std::slice::from_ref(tile), min, max, roads).0
}

/// Strokes by their road's look: (motorway, primary, minor, service, track, railway) counts.
fn census(strokes: &[Stroke]) -> [usize; 6] {
    let mut out = [0; 6];
    for s in strokes {
        let k = match (s.kind, s.tone) {
            (StrokeKind::Rail, _) => 5,
            (StrokeKind::Water, _) => continue,
            (_, t) if t >= 1.0 => 0,
            (_, t) if t >= 0.75 => 1,
            (_, t) if (t - 0.45).abs() < 1e-6 => 2,
            (_, t) if (t - 0.35).abs() < 1e-6 => 3,
            _ => 4,
        };
        out[k] += 1;
    }
    out
}

#[test]
fn a_past_period_leaves_out_the_motorways_and_keeps_the_arterials() {
    let tile = fixture_tile_at(VALLEY, VALLEY.lat, VALLEY.lon);
    let every = |_: [f64; 2]| true;
    let today = census(&strokes(&tile, VALLEY, &Roads::today()));
    assert_eq!(today, [2, 2, 1, 1, 1, 1]);
    // 1942: no motorway nor ramp (none stood in the Valley), every street on ground that stood.
    let roads = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &every);
    assert_eq!(census(&strokes(&tile, VALLEY, &roads)), [0, 2, 1, 1, 1, 1]);
    // Nothing stood: the streets and the driveway go too; the arterials, the track and the railway stay.
    let nothing = |_: [f64; 2]| false;
    let roads = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &nothing);
    assert_eq!(census(&strokes(&tile, VALLEY, &roads)), [0, 2, 0, 0, 1, 1]);
    // A busway is later than every period.
    let busway = way("busway");
    assert!(Roads::today().style(&busway).is_some());
    assert!(roads.style(&busway).is_none());
}

#[test]
fn a_street_ends_where_its_ground_stopped_standing() {
    let tile = fixture_tile_at(VALLEY, VALLEY.lat, VALLEY.lon);
    let id = tile.id;
    // The ground stood west of the tile's middle.
    let cut = osm::project(VALLEY, id, [0.5, 0.6])[0];
    let west = move |p: [f64; 2]| p[0] < cut;
    let roads = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &west);
    let all = strokes(&tile, VALLEY, &roads);
    let street: Vec<&Stroke> = all
        .iter()
        .filter(|s| (s.tone - 0.45).abs() < 1e-6)
        .collect();
    assert_eq!(street.len(), 1);
    let xs: Vec<f64> = street[0].points.iter().map(|p| p[0]).collect();
    let start = osm::project(VALLEY, id, [0.12, 0.6])[0];
    assert!((xs[0] - start).abs() < 1e-6, "{xs:?}");
    let end = *xs.last().unwrap();
    assert!((end - cut).abs() < 2.0, "{end} against {cut}");
    // The arterial crossing the cut is whole.
    assert_eq!(census(&all)[1], 2);
}

#[test]
fn runs_close_short_gaps_and_drop_stubs() {
    use super::period::{GAP_M, runs};
    let line = vec![[0.0, 0.0], [600.0, 0.0], [1000.0, 0.0]];
    let both = [true, true];
    // Kept whole: the line itself.
    assert_eq!(runs(&line, both, |_, _| true), vec![line.clone()]);
    assert!(runs(&line, both, |_, _| false).is_empty());
    // A gap shorter than GAP_M is closed.
    let gap = 0.75 * GAP_M;
    assert_eq!(
        runs(&line, both, |p, _| p[0] < 300.0 || p[0] > 300.0 + gap),
        vec![line.clone()]
    );
    // An island 40 m long, a stub 50 m long at the way's own end: dropped.
    assert!(runs(&line, both, |p, _| p[0] > 480.0 && p[0] < 520.0).is_empty());
    assert!(runs(&line, both, |p, _| p[0] < 50.0).is_empty());
    // Kept to 400 m: the run ends at the cut, through the line's own vertices.
    let kept = runs(&line, both, |p, _| p[0] < 400.0);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0][0], [0.0, 0.0]);
    assert!((kept[0].last().unwrap()[0] - 400.0).abs() < 1.5, "{kept:?}");
    let kept = runs(&line, both, |p, _| p[0] > 450.0);
    assert_eq!(kept[0].len(), 3, "{kept:?}");
    // An end at the tile's edge is a cut: 100 m between it and a cut is a stub's island.
    assert!(runs(&line, [false, true], |p, _| p[0] < 100.0).is_empty());
    assert_eq!(runs(&line, [true, true], |p, _| p[0] < 100.0).len(), 1);
}

#[test]
fn the_period_puts_out_the_street_lights_of_what_it_leaves_out() {
    let tile = fixture_tile_at(VALLEY, VALLEY.lat, VALLEY.lon);
    let level = naval_terrain::night::LAMP_LEVEL;
    let a = page_of(level, osm::project(VALLEY, tile.id, [0.0, 0.0]));
    let b = page_of(level, osm::project(VALLEY, tile.id, [1.0, 1.0]));
    let tiles = std::slice::from_ref(&tile);
    // The tile's lamps by their road's class, over every page it reaches.
    let by_class = |roads: &Roads<'_>| {
        let mut out = [0usize; 8];
        for j in a[1]..=b[1] {
            for i in a[0]..=b[0] {
                for l in super::super::lamps::page_lamps(VALLEY, [i, j], tiles, roads) {
                    out[usize::from(l.class())] += 1;
                }
            }
        }
        out
    };
    let today = by_class(&Roads::today());
    assert!(today[0] > 0 && today[2] > 0 && today[5] > 0, "{today:?}");
    let nothing = |_: [f64; 2]| false;
    let roads = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &nothing);
    let period = by_class(&roads);
    // No motorway's lights, no street's; the primary's as today.
    assert_eq!((period[0], period[5], period[6]), (0, 0, 0), "{period:?}");
    assert_eq!(period[2], today[2]);
}

#[test]
fn a_made_page_is_kept_by_its_period_and_stale_once_the_ground_changes() {
    assert_ne!(
        key(VALLEY, Period::Today, 0, [1, 2]),
        key(VALLEY, Period::Year(1942), 0, [1, 2])
    );
    assert_ne!(
        key(VALLEY, Period::Year(1941), 0, [1, 2]),
        key(VALLEY, Period::Year(1942), 0, [1, 2])
    );
    let mut made = Made {
        page: Arc::new(FeaturePage::empty()),
        version: 3,
        hash: 0,
        at: Instant::now(),
    };
    let long = REMAKE_AFTER * 2;
    let past = Period::Year(1942);
    assert!(made.fresh_with(past, 3, long, |_| true));
    // The ground changed under it since version 3: stale, unless made again too lately.
    assert!(!made.fresh_with(past, 4, long, |since| since < 4));
    assert!(made.fresh_with(past, 4, Duration::ZERO, |_| true));
    // The ground changed elsewhere: fresh, its version brought up to date.
    assert!(made.fresh_with(past, 5, long, |_| false));
    assert_eq!(made.version, 5);
    assert!(made.fresh_with(past, 5, long, |_| panic!("not asked")));
    // Today's pages never read the ground.
    assert!(made.fresh_with(Period::Today, 9, long, |_| true));
    // A page's ground reaches past it by its strokes' reach and an island's length.
    let c = page_chunks(0, [0, 0]);
    assert!(c[0] < 0 && c[1] < 0 && c[2] >= 0 && c[3] >= 0, "{c:?}");
    // A page's strokes' hash tells a change.
    let tile = fixture_tile_at(VALLEY, VALLEY.lat, VALLEY.lon);
    let every = |_: [f64; 2]| true;
    let nothing = |_: [f64; 2]| false;
    let a = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &every);
    let b = Roads::new(Period::Year(1942), VALLEY, [0.0, 0.0], &nothing);
    let hash = |roads: &Roads<'_>| super::period::strokes_hash(&strokes(&tile, VALLEY, roads));
    assert_eq!(hash(&a), hash(&a));
    assert_ne!(hash(&a), hash(&b));
    // And its lamps' (a flood dropped with the ground under it, the strokes the same).
    let lamp = naval_terrain::night::Lamp {
        at: [10.0, 20.0],
        height: 24.0,
        info: 7,
    };
    let strokes = hash(&a);
    assert_eq!(
        super::period::lamps_hash(strokes, &[lamp]),
        super::period::lamps_hash(strokes, &[lamp])
    );
    assert_ne!(
        super::period::lamps_hash(strokes, &[lamp]),
        super::period::lamps_hash(strokes, &[])
    );
}

/// The lengths of `runs` (m), and how many are drawn as trunk roads.
fn run_lengths(runs: &[super::period::Run]) -> (f64, usize) {
    let length = runs
        .iter()
        .flat_map(|r| r.points.windows(2))
        .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
        .sum();
    (length, runs.iter().filter(|r| r.trunk).count())
}

#[test]
fn a_listed_motorway_stands_from_its_year_and_a_crossing_one_does_not() {
    let geo = LatLon {
        lat: 34.09,
        lon: -118.21,
    };
    let at = |lat, lon| to_local(geo, LatLon { lat, lon });
    let every = |_: [f64; 2]| true;
    let roads = |year| Roads::new(Period::Year(year), geo, [0.0, 0.0], &every);
    // The Arroyo Seco Parkway's carriageway through Highland Park (1941), a way 40 m off its route's middle.
    let along = vec![
        at(34.0943, -118.2058),
        at(34.0915, -118.2064),
        at(34.0888, -118.2084),
    ];
    let whole = run_lengths(&Roads::today().runs(&way("motorway"), &along, [true, true])).0;
    let kept = |year| run_lengths(&roads(year).runs(&way("motorway"), &along, [true, true]));
    assert_eq!(kept(1942), (whole, 0));
    assert_eq!(kept(1938).0, 0.0);
    // A freeway across it is not one of the period's, nor a ramp off it a block away.
    let across = vec![at(34.0915, -118.2140), at(34.0915, -118.1995)];
    assert_eq!(
        run_lengths(&roads(1942).runs(&way("motorway"), &across, [true, true])).0,
        0.0
    );
    let ramp = Transportation {
        ramp: true,
        ..way("motorway")
    };
    let near = vec![at(34.0915, -118.2066), at(34.0921, -118.2061)];
    let far = vec![at(34.0915, -118.2030), at(34.0918, -118.2020)];
    assert!(run_lengths(&roads(1942).runs(&ramp, &near, [true, true])).0 > 0.0);
    assert_eq!(
        run_lengths(&roads(1942).runs(&ramp, &far, [true, true])).0,
        0.0
    );
    // Nowhere else did one stand: the Valley's freeways are left out.
    let valley = Roads::new(Period::Year(1960), VALLEY, [0.0, 0.0], &every);
    let line = vec![[0.0, 0.0], [500.0, 0.0]];
    assert!(
        valley
            .runs(&way("motorway"), &line, [true, true])
            .is_empty()
    );
}

#[test]
fn a_motorway_that_was_a_boulevard_is_drawn_as_one() {
    let geo = LatLon {
        lat: 34.06,
        lon: -118.2,
    };
    let at = |lat, lon| to_local(geo, LatLon { lat, lon });
    let every = |_: [f64; 2]| true;
    let roads = |year| Roads::new(Period::Year(year), geo, [0.0, 0.0], &every);
    // The Ramona Parkway's line in Boyle Heights: a boulevard from 1935, a freeway from 1944.
    let line = vec![at(34.0558, -118.1997), at(34.0556, -118.1965)];
    let motorway = way("motorway");
    let runs = |year| run_lengths(&roads(year).runs(&motorway, &line, [true, true]));
    let whole = run_lengths(&Roads::today().runs(&motorway, &line, [true, true])).0;
    assert_eq!(runs(1934).0, 0.0);
    assert_eq!(runs(1940), (whole, 1));
    assert_eq!(runs(1944), (whole, 0));
    // As a trunk road: a trunk's width and tone, and no ramps.
    let run = roads(1940).runs(&motorway, &line, [true, true]).remove(0);
    let style = roads(1940).run_style(&motorway, style_of(&motorway).unwrap(), &run);
    assert_eq!(style, trunk_style());
    let ramp = Transportation {
        ramp: true,
        ..way("motorway")
    };
    assert!(roads(1940).runs(&ramp, &line, [true, true]).is_empty());
}

#[test]
fn the_period_paves_as_its_region_did() {
    let every = |_: [f64; 2]| true;
    // Japan: the arterials paved, the streets earth.
    let yokohama = LatLon {
        lat: 35.44,
        lon: 139.64,
    };
    let tile = fixture_tile_at(yokohama, yokohama.lat, yokohama.lon);
    let unpaved = |roads: &Roads<'_>| {
        strokes(&tile, yokohama, roads)
            .iter()
            .filter(|s| s.kind == StrokeKind::Unpaved)
            .map(|s| s.tone)
            .collect::<Vec<_>>()
    };
    // Today: only the track.
    assert_eq!(unpaved(&Roads::today()), vec![0.0]);
    let roads = Roads::new(Period::Year(1942), yokohama, [0.0, 0.0], &every);
    let mut tones = unpaved(&roads);
    tones.sort_by(f32::total_cmp);
    // The track, the driveway and the street.
    assert_eq!(tones, vec![0.0, 0.35, 0.45]);
    // Iceland: Reykjavík's streets paved, the country's roads gravel; the Solomons: none paved.
    let frame = LatLon {
        lat: 64.14,
        lon: -21.94,
    };
    let reykjavik = to_local(
        frame,
        LatLon {
            lat: 64.1466,
            lon: -21.9426,
        },
    );
    let roads = Roads::new(Period::Year(1942), frame, reykjavik, &every);
    assert_eq!(
        roads.paved("minor", StrokeKind::Paved, reykjavik),
        StrokeKind::Paved
    );
    let out = [reykjavik[0] + 5000.0, reykjavik[1]];
    assert_eq!(
        roads.paved("primary", StrokeKind::Paved, out),
        StrokeKind::Unpaved
    );
    // A street across the paved streets' edge is paved to the edge and gravel beyond, whichever way it runs.
    let minor = way("minor");
    let style = style_of(&minor).unwrap();
    let across = super::period::Run {
        points: vec![
            [reykjavik[0] + 1000.0, reykjavik[1] + 10.0],
            [reykjavik[0] + 2000.0, reykjavik[1] - 10.0],
        ],
        trunk: false,
    };
    let drawn = |run: &super::period::Run| {
        roads
            .paving_pieces(run.clone())
            .into_iter()
            .map(|piece| (roads.run_style(&minor, style, &piece).0, piece.points))
            .collect::<Vec<_>>()
    };
    let forth = drawn(&across);
    let mut back = drawn(&super::period::Run {
        points: across.points.iter().rev().copied().collect(),
        trunk: false,
    });
    back.reverse();
    assert_eq!(forth.len(), 2);
    assert_eq!(
        forth.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        vec![StrokeKind::Paved, StrokeKind::Unpaved]
    );
    let edge = forth[0].1[1];
    assert!(((edge[0] - reykjavik[0]).hypot(edge[1] - reykjavik[1]) - 1500.0).abs() < 1e-6);
    for ((k, a), (l, b)) in forth.iter().zip(&back) {
        assert_eq!(k, l);
        let (a0, a1) = (a[0], a[a.len() - 1]);
        let (b0, b1) = (b[b.len() - 1], b[0]);
        assert!(
            (a0[0] - b0[0]).hypot(a0[1] - b0[1]) < 1e-6
                && (a1[0] - b1[0]).hypot(a1[1] - b1[1]) < 1e-6
        );
    }
    let guadalcanal = LatLon {
        lat: -9.43,
        lon: 160.05,
    };
    let roads = Roads::new(Period::Year(1943), guadalcanal, [0.0, 0.0], &every);
    assert_eq!(
        roads.paved("primary", StrokeKind::Paved, [0.0, 0.0]),
        StrokeKind::Unpaved
    );
    // Today's are today's everywhere.
    assert_eq!(
        Roads::today().paved("minor", StrokeKind::Paved, out),
        StrokeKind::Paved
    );
}

#[test]
fn a_bridge_built_after_the_period_is_left_out_with_its_road() {
    // A primary road's bridge where the Yokohama Bay Bridge (1989) stands.
    let geo = LatLon {
        lat: 35.45,
        lon: 139.67,
    };
    let (lat, lon) = (35.4544, 139.6747);
    let id = TileId::of(lat, lon, 14);
    let n = f64::from(1u32 << 14);
    let x = (lon + 180.0) / 360.0 * n - f64::from(id.x);
    let y =
        (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) / 2.0 * n - f64::from(id.y);
    let mut features: Features<naval_osm::TilePoint> = Features::default();
    features.transportation.push(Transportation {
        brunnel: Brunnel::Bridge,
        geometry: Geometry::Lines(vec![vec![[x - 0.05, y], [x + 0.05, y]]]),
        ..way("primary")
    });
    let tile = Arc::new(OsmTile::new(geo, id, &features));
    let every = |_: [f64; 2]| true;
    let count = |year| {
        let roads = Roads::new(Period::Year(year), geo, [0.0, 0.0], &every);
        census(&strokes(&tile, geo, &roads))[1]
    };
    assert_eq!(census(&strokes(&tile, geo, &Roads::today()))[1], 1);
    assert_eq!(count(1942), 0);
    assert_eq!(count(1990), 1);
}
