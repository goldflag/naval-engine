use super::*;
use naval_osm::mvt::encode;

/// The California user scenario's origin (docs/archive/native-theatre-history.md, "OpenStreetMap features").
const MONTEREY_BAY: LatLon = LatLon {
    lat: 36.936204,
    lon: -122.418494,
};

/// The cache directory a test uses: its own under `.build/`, never the player's.
fn test_cache(name: &str) -> Cache {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/osm-test")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    Cache { dir }
}

/// A tile with one primary road from its north-west corner to the middle of its south edge, a lake over its north-east
/// quarter, and a 12 m building.
fn synthetic_tile() -> Vec<u8> {
    let road = encode::feature(
        1,
        &[(0, 0)],
        2,
        &encode::commands(&[(&[[0, 0], [2048, 4096]], false)]),
    );
    let transportation = encode::layer(
        "transportation",
        &["class"],
        &[encode::string_value("primary")],
        &[road],
    );
    let quarter: &[[i32; 2]] = &[[2048, 0], [4096, 0], [4096, 2048], [2048, 2048]];
    let lake = encode::feature(2, &[(0, 0)], 3, &encode::commands(&[(quarter, true)]));
    let water = encode::layer(
        "water",
        &["class"],
        &[encode::string_value("lake")],
        &[lake],
    );
    let square: &[[i32; 2]] = &[[100, 3000], [140, 3000], [140, 3040], [100, 3040]];
    let house = encode::feature(3, &[(0, 0)], 3, &encode::commands(&[(square, true)]));
    let building = encode::layer(
        "building",
        &["render_height"],
        &[encode::double_value(12.0)],
        &[house],
    );
    encode::tile(&[transportation, water, building])
}

#[test]
fn a_rectangle_of_the_frame_is_covered_by_its_tiles_nearest_first() {
    let (min, max) = ([-6_000.0, -2_500.0], [4_000.0, 7_500.0]);
    let tiles = tiles_over(MONTEREY_BAY, min, max, 14);
    // About 10 km by 10 km at 1.95 km a tile: six or seven a side.
    assert!((25..=64).contains(&tiles.len()), "{}", tiles.len());
    for k in 0..=20 {
        for m in 0..=20 {
            let p = [
                min[0] + (max[0] - min[0]) * f64::from(k) / 20.0,
                min[1] + (max[1] - min[1]) * f64::from(m) / 20.0,
            ];
            let ll = to_lat_lon(MONTEREY_BAY, p);
            let id = TileId::of(ll.lat, ll.lon, 14);
            assert!(tiles.contains(&id), "{p:?} in {id:?}");
        }
    }
    let middle = to_lat_lon(MONTEREY_BAY, [-1_000.0, 2_500.0]);
    assert_eq!(tiles[0], TileId::of(middle.lat, middle.lon, 14));
}

#[test]
fn a_tile_projects_into_the_frame_where_its_corners_lie() {
    let id = TileId::of(36.97, -122.03, 14);
    let tile = OsmTile::decode(MONTEREY_BAY, id, &synthetic_tile()).unwrap();
    // The corners: the frame's own projection of each corner's latitude and longitude, to the millimetre.
    let (lat, lon) = id.lat_lon(0.0, 0.0, 1.0);
    let nw = to_local(MONTEREY_BAY, LatLon { lat, lon });
    assert!((project(MONTEREY_BAY, id, [0.0, 0.0])[0] - nw[0]).abs() < 1e-3);
    assert!((project(MONTEREY_BAY, id, [0.0, 0.0])[1] - nw[1]).abs() < 1e-3);
    // Santa Cruz lies about 35 km east and 3.7 km north of the origin (z south: negative).
    assert!((30_000.0..40_000.0).contains(&nw[0]), "{nw:?}");
    assert!((-6_000.0..-2_000.0).contains(&nw[1]), "{nw:?}");
    // The tile is about 1.95 km a side, x east and z south.
    let [x0, z0, x1, z1] = tile.bounds;
    assert!(
        ((x1 - x0) - 1955.0).abs() < 15.0 && ((z1 - z0) - 1955.0).abs() < 15.0,
        "{:?}",
        tile.bounds
    );
    let road = &tile.features.transportation[0];
    assert_eq!(road.class, "primary");
    let Geometry::Lines(lines) = &road.geometry else {
        panic!()
    };
    let (a, b) = (lines[0][0], lines[0][1]);
    assert!((a[0] - nw[0]).abs() < 1e-3 && (a[1] - nw[1]).abs() < 1e-3);
    // Its end: the south edge's middle, south (greater z) and east of its start.
    assert!(
        b[1] - a[1] > 1900.0 && (b[0] - a[0] - 977.0).abs() < 15.0,
        "{a:?} {b:?}"
    );
    assert_eq!(tile.features.building[0].render_height, Some(12.0));

    // Queried by area: the lake lies in the north-east quarter, the house near the south-west corner.
    let ne = ([x0 + 1200.0, z0 + 100.0], [x1 - 100.0, z0 + 800.0]);
    assert_eq!(tile.water_in(ne.0, ne.1).count(), 1);
    assert_eq!(tile.building_in(ne.0, ne.1).count(), 0);
    let sw = ([x0, z1 - 600.0], [x0 + 200.0, z1]);
    assert_eq!(tile.building_in(sw.0, sw.1).count(), 1);
    assert_eq!(tile.water_in(sw.0, sw.1).count(), 0);
    let area = Area {
        tiles: vec![Arc::new(tile)],
        missing: vec![],
    };
    assert_eq!(area.transportation_in([x0, z0], [x1, z1]).count(), 1);
    assert_eq!(
        area.transportation_in([x1 + 500.0, z0], [x1 + 900.0, z1])
            .count(),
        0
    );

    // An empty body is a tile with nothing in it; bytes that are no tile are refused.
    let empty = OsmTile::decode(MONTEREY_BAY, id, &[]).unwrap();
    assert_eq!(empty.features.point_count(), 0);
    assert!(OsmTile::decode(MONTEREY_BAY, id, &[0x1b, 0x00]).is_err());
}

#[test]
fn the_cache_keeps_tiles_whole_and_remembers_the_absent() {
    let cache = test_cache("cache");
    let id = TileId::new(14, 2638, 6377);
    assert_eq!(cache.read(id), None);
    cache.write(id, Some(&synthetic_tile())).unwrap();
    assert_eq!(cache.read(id), Some(Some(synthetic_tile())));
    assert!(
        cache
            .tile_path(id)
            .ends_with("openfreemap/14/2638/6377.pbf")
    );
    // No temporary file is left beside it.
    let dir = cache.tile_path(id).parent().unwrap().to_path_buf();
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    let gone = TileId::new(14, 1, 1);
    cache.write(gone, None).unwrap();
    assert_eq!(cache.read(gone), Some(None));
    // Decoded from the cache: the tile, the absent one as an empty tile, a damaged one deleted.
    let bad = TileId::new(14, 2, 2);
    cache.write(bad, Some(&[0x1b, 0x00])).unwrap();
    let decoded = decode_all(
        &cache,
        MONTEREY_BAY,
        &[id, gone, bad, TileId::new(14, 3, 3)],
    );
    assert_eq!(
        decoded[0].as_ref().unwrap().features.transportation.len(),
        1
    );
    assert_eq!(decoded[1].as_ref().unwrap().features.point_count(), 0);
    assert!(decoded[2].is_none() && decoded[3].is_none());
    assert_eq!(cache.read(bad), None);
}

#[test]
fn the_tile_url_comes_from_the_tilejson() {
    let json = r#"{"tilejson":"3.0.0","tiles":["https://tiles.openfreemap.org/planet/20260927_080001_pt/{z}/{x}/{y}.pbf"],"maxzoom":14}"#;
    let t = template_of(json).unwrap();
    assert_eq!(
        tile_url(&t, TileId::new(14, 2638, 6377)),
        "https://tiles.openfreemap.org/planet/20260927_080001_pt/14/2638/6377.pbf"
    );
    assert_eq!(template_of(r#"{"tiles":[]}"#), None);
    assert_eq!(template_of(r#"{"tiles":["http://x/{z}/{x}/{y}"]}"#), None);
    assert_eq!(template_of("<html>"), None);
}

#[test]
fn nothing_is_fetched_with_streaming_off() {
    // The library's tests never turn streaming on: asking queues nothing and spawns no worker.
    assert!(!enabled());
    want("test", MONTEREY_BAY, &[TileId::new(14, 2638, 6377)]);
    assert_eq!(status().queued, 0);
}

/// On the network (`cargo test -p naval-client --release --lib theatre::osm -- --ignored --nocapture`): Santa Cruz's
/// tiles from OpenFreeMap into `.build/native/osm-test/`, decoded and projected.
#[test]
#[ignore = "fetches from OpenFreeMap"]
fn santa_cruzs_tiles_hold_roads_railways_rivers_and_buildings() {
    let cache = test_cache("network");
    let ids = tiles_over(MONTEREY_BAY, [33_000.0, -6_000.0], [37_000.0, -2_000.0], 14);
    let started = Instant::now();
    for &id in &ids {
        fetch(&cache, id).unwrap();
    }
    let fetched = started.elapsed();
    let started = Instant::now();
    let tiles: Vec<OsmTile> = decode_all(&cache, MONTEREY_BAY, &ids)
        .into_iter()
        .map(Option::unwrap)
        .collect();
    let decoded = started.elapsed();
    let bytes: u64 = ids
        .iter()
        .map(|&id| {
            std::fs::metadata(cache.tile_path(id))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum();
    let count = |f: &dyn Fn(&OsmTile) -> usize| tiles.iter().map(f).sum::<usize>();
    let roads = count(&|t| {
        t.features
            .transportation
            .iter()
            .filter(|r| r.class != "rail" && r.class != "transit")
            .count()
    });
    let rail = count(&|t| {
        t.features
            .transportation
            .iter()
            .filter(|r| r.class == "rail")
            .count()
    });
    let waterways = count(&|t| t.features.waterway.len());
    let water = count(&|t| t.features.water.len());
    let buildings = count(&|t| t.features.building.len());
    let heights = count(&|t| {
        t.features
            .building
            .iter()
            .filter(|b| b.render_height.is_some())
            .count()
    });
    let points = count(&|t| t.features.point_count());
    println!(
        "{} tiles, {:.1} MB, fetched in {:.2} s, decoded and projected in {:.0} ms: {roads} roads, {rail} railways, \
         {waterways} waterways, {water} water areas, {buildings} buildings ({heights} with a height), {points} points",
        ids.len(),
        bytes as f64 / 1e6,
        fetched.as_secs_f64(),
        decoded.as_secs_f64() * 1000.0
    );
    let classes: HashSet<&str> = tiles
        .iter()
        .flat_map(|t| t.features.transportation.iter().map(|r| r.class.as_str()))
        .collect();
    println!("transportation classes: {classes:?}");
    // (Santa Cruz is thinly mapped in buildings: about a hundred in these tiles.)
    assert!(roads > 200 && rail > 0 && waterways > 0 && water > 0 && buildings > 50);
    assert!(
        classes.contains("primary") && classes.contains("minor") && classes.contains("service")
    );
    // Everything lies about the asked rectangle (a tile's reach is its own square and its buffer).
    for t in &tiles {
        for r in &t.features.transportation {
            for p in r.geometry.points() {
                assert!(
                    p[0] > t.bounds[0] - 100.0 && p[0] < t.bounds[2] + 100.0,
                    "{p:?}"
                );
                assert!(
                    p[1] > t.bounds[1] - 100.0 && p[1] < t.bounds[3] + 100.0,
                    "{p:?}"
                );
            }
        }
    }
}
