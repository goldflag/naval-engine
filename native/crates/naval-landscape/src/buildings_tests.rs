use super::*;

/// Two lines as the dataset has them (Reykjavík's, without a height; Honolulu's, with one).
const LINES: [&str; 2] = [
    r#"{"type": "Feature", "properties": { "height": -1.0, "confidence": -1.0},"geometry": {"type": "Polygon","coordinates": [[[-21.939243507333398, 64.16035660171019], [-21.9393049747436, 64.16046030976038], [-21.93934269093763, 64.16045606317516], [-21.939329050493836, 64.16043304899603], [-21.939364252274924, 64.16042908551259], [-21.939316424072647, 64.16034838952899], [-21.939243507333398, 64.16035660171019]]]}}"#,
    r#"{"type": "Feature", "properties": { "height": 3.4525222778320312, "confidence": 0.9613999724388123},"geometry": {"type": "Polygon","coordinates": [[[-157.66602087971563, 21.294040403219256], [-157.66599368071778, 21.29388972155371], [-157.66575631375304, 21.29392691724304], [-157.66576903586494, 21.29399739737832], [-157.66588641513323, 21.293979003915595], [-157.66590089201915, 21.294059205417767], [-157.66602087971563, 21.294040403219256]]]}}"#,
];

/// A land style for the contexts below.
fn style(style: naval_conditions::LandStyle) -> SurfaceStyle {
    SurfaceStyle {
        style,
        shore: [0.0; 3],
        low: [0.0; 3],
        high: [0.0; 3],
        equator: [0.0; 2],
        climate: naval_conditions::Climate::default(),
    }
}

/// A gzip member of `text` (raw DEFLATE between a bare header and a trailer the reader skips).
fn gzip(text: &str) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(text.as_bytes(), 6));
    out.extend([0u8; 8]);
    out
}

#[test]
fn quadkeys_name_the_indexs_tiles() {
    // Reykjavík's and Honolulu's tiles as the index names them.
    assert_eq!(quadkey(64.146, -21.94, 9), "031102000");
    assert_eq!(quadkey(21.307, -157.858, 9), "022211111");
    // South and east too (Honiara), and the tile holds the point it was named for.
    let key = quadkey(-9.43, 159.95, 9);
    assert_eq!(key, "311102213");
    let [s, w, n, e] = quadkey_bounds(&key).unwrap();
    assert!(
        s < -9.43 && -9.43 < n && w < 159.95 && 159.95 < e,
        "{s} {w} {n} {e}"
    );
    // A level-9 tile is 360/512 degrees wide.
    assert!((e - w - 360.0 / 512.0).abs() < 1e-9);
    for (lat, lon) in [
        (64.146, -21.94),
        (21.307, -157.858),
        (-33.9, 151.2),
        (51.12, 1.3),
    ] {
        let [s, w, n, e] = quadkey_bounds(&quadkey(lat, lon, 9)).unwrap();
        assert!(s <= lat && lat < n && w <= lon && lon < e);
    }
    assert!(quadkey_bounds("0x1").is_none());
}

#[test]
fn the_index_lists_every_regions_file_for_a_quadkey() {
    let csv = "Location,QuadKey,Url,Size,UploadDate\n\
        Europe,031102000,https://example.org/Europe/031102000.csv.gz,7.0KB,2026-08-13\n\
        Iceland,031102000,https://example.org/Iceland/031102000.csv.gz,4.5MB,2026-08-13\n\
        UnitedStates,022211111,https://example.org/US/022211111.csv.gz,14.4MB,2026-08-13\n\
        Japan,133002112,https://example.org/JP/133002112.csv.gz,300.1MB,2026-08-13\n\
        broken line\n";
    let index = parse_index(csv);
    assert_eq!(index.files.len(), 3);
    let reykjavik = &index.files["031102000"];
    assert_eq!(reykjavik.len(), 2);
    assert_eq!(reykjavik[1].location, "Iceland");
    assert_eq!(reykjavik[1].bytes, (4.5 * 1024.0 * 1024.0) as u64);
    assert_eq!(reykjavik[0].bytes, 7 * 1024);
    assert!(index.files["133002112"][0].bytes > WHOLE_FILE_BYTES);
    assert_eq!(parse_size("697B"), 697);
}

#[test]
fn lines_give_their_ring_and_a_height_only_where_measured() {
    let a = parse_line(LINES[0]).unwrap();
    assert_eq!(a.ring.len(), 7);
    assert_eq!(a.ring[0], [-21.939243507333398, 64.16035660171019]);
    assert_eq!(a.height, None);
    let b = parse_line(LINES[1]).unwrap();
    assert_eq!(b.ring.len(), 7);
    assert!((b.height.unwrap() - 3.4525).abs() < 1e-4);
    // A multipolygon's first polygon; a polygon with a hole keeps its outer ring.
    let multi = r#"{"type": "Feature", "properties": {"height": 9.0}, "geometry": {"type": "MultiPolygon", "coordinates": [[[[1.0, 2.0], [1.001, 2.0], [1.001, 2.001], [1.0, 2.0]]], [[[5.0, 5.0], [5.1, 5.0], [5.1, 5.1], [5.0, 5.0]]]]}}"#;
    let m = parse_line(multi).unwrap();
    assert_eq!(m.ring.len(), 4);
    assert_eq!(m.ring[1], [1.001, 2.0]);
    let holed = r#"{"properties": {"height": -1}, "geometry": {"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]], [[0.2, 0.2], [0.3, 0.2], [0.3, 0.3], [0.2, 0.2]]]}}"#;
    assert_eq!(parse_line(holed).unwrap().ring.len(), 4);
    assert!(parse_line("{\"type\": \"Feature\"}").is_none());
    assert!(parse_line("").is_none());
}

#[test]
fn a_gzip_stream_is_read_line_by_line_across_members() {
    let one = format!("{}\n{}\n", LINES[0], LINES[1]);
    let mut bytes = gzip(&one);
    bytes.extend(gzip(LINES[0]));
    let mut lines = Vec::new();
    gunzip_lines(&bytes, |l| lines.push(l.to_owned())).unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1], LINES[1]);
    assert_eq!(lines[2], LINES[0]);
    assert!(gunzip_lines(b"not gzip at all", |_| {}).is_err());
    // A file's footprints into a store, by chunk, about Denmark Strait's origin.
    let geo = LatLon {
        lat: 64.6,
        lon: -28.4,
    };
    let mut store = QuadStore::default();
    store.add_file(&gzip(&one), geo).unwrap();
    assert_eq!(store.footprints, 2);
    assert!(store.covered);
}

/// A real gzip member of `text` (with its CRC-32 and length), as the dataset's files are.
fn real_gzip(text: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    out.write_all(text.as_bytes()).unwrap();
    out.finish().unwrap()
}

/// A fresh folder under the system's temporary one.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "naval-buildings-test-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_big_file_streams_into_tiles_and_reads_back() {
    // A square house every 300 m across 6 km of Los Angeles (a level-13 tile is about 4 km there), in two members.
    let mut lines = Vec::new();
    for j in 0..20 {
        for i in 0..20 {
            let (lat, lon) = (
                33.80 + f64::from(j) * 0.0027,
                -118.40 + f64::from(i) * 0.0032,
            );
            let d = 0.0001;
            let height = if (i + j) % 3 == 0 { "7.5" } else { "-1.0" };
            lines.push(format!(
                r#"{{"type": "Feature", "properties": {{ "height": {height}, "confidence": -1.0}},"geometry": {{"type": "Polygon","coordinates": [[[{lon}, {lat}], [{}, {lat}], [{}, {}], [{lon}, {}], [{lon}, {lat}]]]}}}}"#,
                lon + d,
                lon + d,
                lat + d,
                lat + d
            ));
        }
    }
    let mut gz = real_gzip(&(lines[..150].join("\n") + "\n"));
    gz.extend(real_gzip(&lines[150..].join("\n")));
    let root = temp_dir("split");
    let done = root.join("023012311.tiles");
    let count = AtomicU64::new(0);
    let summary = split::split_into(gz.as_slice(), &done, &count).unwrap();
    assert_eq!(summary.footprints, 400);
    assert_eq!(summary.bad, 0);
    assert!(summary.tiles >= 2, "{summary:?}");
    assert_eq!(count.load(Ordering::Relaxed), gz.len() as u64);
    assert!(done.is_dir());
    // Every footprint lands in the tile holding its middle, its ring within a centimetre and its height kept.
    let mut back = Vec::new();
    for entry in std::fs::read_dir(&done).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let tile = name.strip_suffix(".bin").unwrap().to_owned();
        assert_eq!(tile.len(), split::SPLIT_LEVEL as usize);
        for raw in split::read_tile(&done, &tile).unwrap() {
            assert_eq!(split::tile_of(&raw.ring), tile);
            back.push(raw);
        }
    }
    assert_eq!(back.len(), 400);
    let original = parse_line(&lines[3]).unwrap();
    let found = back
        .iter()
        .find(|r| {
            (r.ring[0][0] - original.ring[0][0]).abs() < 1e-6
                && (r.ring[0][1] - original.ring[0][1]).abs() < 1e-6
        })
        .unwrap();
    assert_eq!(found.height, original.height);
    assert_eq!(found.height, Some(7.5));
    for (a, b) in found.ring.iter().zip(&original.ring) {
        assert!((a[0] - b[0]).abs() < 1.5e-7 && (a[1] - b[1]).abs() < 1.5e-7);
    }
    let measured = (0..20)
        .flat_map(|j| (0..20).map(move |i| (i + j) % 3 == 0))
        .filter(|m| *m)
        .count();
    assert_eq!(back.iter().filter(|r| r.height.is_some()).count(), measured);
    // A tile with no footprints reads as none; a damaged one errors.
    assert!(split::read_tile(&done, "0000000000000").unwrap().is_empty());
    std::fs::write(done.join("0000000000001.bin"), b"NBFOOT01\x00\x00").unwrap();
    assert!(split::read_tile(&done, "0000000000001").is_err());
    std::fs::write(done.join("0000000000002.bin"), b"something else").unwrap();
    assert!(split::read_tile(&done, "0000000000002").is_err());
    // The chunks about a point read the tiles under them: a chunk's tiles hold every footprint whose middle it holds.
    let geo = LatLon {
        lat: 33.80,
        lon: -118.40,
    };
    let mut by_chunk: HashMap<[i32; 2], usize> = HashMap::new();
    for raw in &back {
        *by_chunk.entry(project(raw, geo).unwrap().0).or_default() += 1;
    }
    for (chunk, n) in by_chunk {
        let found: usize = quadkeys_over(geo, chunk, split::SPLIT_LEVEL)
            .iter()
            .flat_map(|tile| split::read_tile(&done, tile).unwrap_or_default())
            .filter(|raw| project(raw, geo).unwrap().0 == chunk)
            .count();
        assert_eq!(found, n, "chunk {chunk:?}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_cut_or_damaged_stream_is_never_kept() {
    let text = (0..200)
        .map(|i| LINES[i % 2])
        .collect::<Vec<_>>()
        .join("\n");
    let gz = real_gzip(&text);
    let root = temp_dir("cut");
    let count = AtomicU64::new(0);
    // Cut short anywhere: inside the deflate stream, or in the trailer.
    for cut in [gz.len() / 3, gz.len() - 12, gz.len() - 3] {
        let done = root.join(format!("cut-{cut}.tiles"));
        assert!(
            split::split_into(&gz[..cut], &done, &count).is_err(),
            "cut at {cut}"
        );
        assert!(!done.exists());
    }
    // A trailer's CRC-32 wrong.
    let mut bad = gz.clone();
    let n = bad.len();
    bad[n - 8] ^= 0xff;
    let done = root.join("crc.tiles");
    assert!(split::split_into(bad.as_slice(), &done, &count).is_err());
    assert!(!done.exists());
    // No working folder is left behind.
    let left: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().collect();
    assert!(left.is_empty(), "{left:?}");
    // The whole stream is kept.
    let done = root.join("whole.tiles");
    let summary = split::split_into(gz.as_slice(), &done, &count).unwrap();
    assert_eq!(summary.footprints, 200);
    assert!(done.is_dir());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_chunks_quadkeys_cover_it() {
    let geo = LatLon {
        lat: 33.62,
        lon: -118.48,
    };
    for index in [[0, 0], [17, 25], [-40, 3], [120, -90]] {
        // The dataset's level: the tiles of its corners and middle, as before.
        let c = chunk_corner(index);
        let mut corners: Vec<String> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)]
            .iter()
            .map(|(u, v)| {
                let p = to_lat_lon(geo, [c[0] + u * CHUNK_M, c[1] + v * CHUNK_M]);
                quadkey(p.lat, p.lon, QUADKEY_LEVEL)
            })
            .collect();
        corners.sort();
        corners.dedup();
        let keys = quadkeys_over(geo, index, QUADKEY_LEVEL);
        assert!(
            corners.iter().all(|k| keys.contains(k)),
            "{corners:?} {keys:?}"
        );
        // The tiles: every point of the chunk in one of them.
        let tiles = quadkeys_over(geo, index, split::SPLIT_LEVEL);
        for u in 0..=10 {
            for v in 0..=10 {
                let p = to_lat_lon(
                    geo,
                    [c[0] + f64::from(u) * 100.0, c[1] + f64::from(v) * 100.0],
                );
                assert!(tiles.contains(&quadkey(p.lat, p.lon, split::SPLIT_LEVEL)));
            }
        }
        assert!(tiles.len() <= 4, "{tiles:?}");
    }
    // Across the date line: the tiles either side of it, not the whole way round the world.
    let fiji = LatLon {
        lat: -17.0,
        lon: 179.9999,
    };
    for level in [QUADKEY_LEVEL, split::SPLIT_LEVEL] {
        let keys = quadkeys_over(fiji, [0, 0], level);
        assert!(keys.len() <= 4, "{keys:?}");
        let c = chunk_corner([0, 0]);
        for (u, v) in [(0.0, 0.0), (1.0, 1.0), (0.5, 0.5)] {
            let p = to_lat_lon(fiji, [c[0] + u * CHUNK_M, c[1] + v * CHUNK_M]);
            assert!(keys.contains(&quadkey(p.lat, p.lon, level)), "{keys:?}");
        }
    }
}

#[test]
fn a_gone_tile_folder_is_an_error_and_a_missing_tile_none() {
    let dir = std::env::temp_dir().join(format!("naval-tiles-gone-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(split::read_tile(&dir, "0230123110000").is_err());
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(
        split::read_tile(&dir, "0230123110000").map(|r| r.len()),
        Ok(0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_footprint_is_projected_into_its_chunk() {
    // A 10 m square 2.5 km east and 1.2 km south of the frame's origin (x east, z south).
    let geo = LatLon {
        lat: 64.0,
        lon: -22.0,
    };
    let corner = |x: f64, z: f64| {
        let p = naval_earth::geo::to_lat_lon(geo, [x, z]);
        [p.lon, p.lat]
    };
    let raw = RawFootprint {
        ring: vec![
            corner(2500.0, 1200.0),
            corner(2510.0, 1200.0),
            corner(2510.0, 1210.0),
            corner(2500.0, 1210.0),
            corner(2500.0, 1200.0),
        ],
        height: Some(7.0),
    };
    let (chunk, f) = project(&raw, geo).unwrap();
    assert_eq!(chunk, [2, 1]);
    assert_eq!(f.height, Some(7.0));
    assert!(
        (f.ring[0][0] - 500.0).abs() < 0.01 && (f.ring[0][1] - 200.0).abs() < 0.01,
        "{:?}",
        f.ring[0]
    );
    let ring = naval_terrain::buildings::geometry::simplify(&f.ring, 0.1).unwrap();
    assert!((naval_terrain::buildings::geometry::area(&ring) - 100.0).abs() < 0.1);
}

#[test]
fn the_wanted_chunks_are_nearest_first_whole_near_and_boxes_far() {
    let s = BuildingSettings::HIGH;
    let list = wanted([500.0, 500.0, 300.0], &s, &|_| None);
    assert_eq!(list[0].1.index, [0, 0]);
    assert!(list.windows(2).all(|w| w[0].0 <= w[1].0));
    let count = |list: &[(f32, ChunkKey)], lod| list.iter().filter(|(_, k)| k.lod == lod).count();
    assert!(
        list.iter()
            .all(|(d, k)| k.lod == Lod::Far || *d <= s.range_m)
    );
    let full = count(&list, Lod::Full);
    let boxes = count(&list, Lod::Box);
    let far = count(&list, Lod::Far);
    // About π · 3.5² km² whole, π · (10² - 3.5²) as boxes, π · (30² - 10²) far.
    assert!((30..60).contains(&full), "{full}");
    assert!((250..360).contains(&boxes), "{boxes}");
    assert!((2300..2700).contains(&far), "{far}");
    // Without the far level, nothing past the boxes (as before it).
    let plain = BuildingSettings { far_m: 0.0, ..s };
    let near = wanted([500.0, 500.0, 300.0], &plain, &|_| None);
    assert_eq!(near.len(), full + boxes);
    assert!(near.iter().all(|(d, _)| *d <= s.range_m));
    // From 12 km up: no whole chunks or boxes, but the far level's ring all the same.
    let high = wanted([500.0, 500.0, 12_000.0], &s, &|_| None);
    assert_eq!(count(&high, Lod::Far), high.len());
    assert!((2700..2950).contains(&high.len()), "{}", high.len());
    assert!(wanted([500.0, 500.0, 12_000.0], &plain, &|_| None).is_empty());
}

#[test]
fn the_far_ranges_override_is_read_and_held_within_the_layers_slots() {
    assert_eq!(far_range("off"), Some(0.0));
    assert_eq!(far_range(" False "), Some(0.0));
    assert_eq!(far_range("16"), Some(16_000.0));
    assert_eq!(far_range("-3"), Some(0.0));
    assert_eq!(far_range("nonsense"), None);
    // Past High's 30 km the chunks wanted would outrun the layer's slots: held at 30.
    assert_eq!(far_range("40"), Some(MOST_FAR_M));
    let s = BuildingSettings::HIGH;
    assert!(s.far_m <= MOST_FAR_M);
    let chunks = wanted([0.0, 0.0, 500.0], &s, &|_| None).len();
    assert!(
        chunks + chunks / 4 < naval_terrain::buildings::layer::CHUNK_CAPACITY,
        "{chunks}"
    );
}

#[test]
fn a_far_chunk_stands_its_town_from_the_cover() {
    // A tropical town's cover (built up within 1.2 km of theatre (0, 0)): masses on its chunks, none beyond.
    let context = tropical_town();
    let key = |index| ChunkKey {
        index,
        lod: Lod::Far,
    };
    let (mesh, done) = far_mesh(&context, None, key([0, 0]), &Clearing::default(), &[]);
    assert!(done.tiles && !done.awaiting);
    assert!(mesh.masses.len() > 50, "{}", mesh.masses.len());
    assert!(mesh.vertices.is_empty());
    let (none, _) = far_mesh(&context, None, key([1, 1]), &Clearing::default(), &[]);
    assert!(none.masses.is_empty());
    // Past the window's maps: none, and made again later.
    let (out, done) = far_mesh(&context, None, key([5, 0]), &Clearing::default(), &[]);
    assert!(out.masses.is_empty() && !done.tiles);
    // In a past period with nothing known of its ground: none yet, and made again later.
    let past = Context {
        period: Period::Year(1942),
        ..context
    };
    let (unknown, done) = far_mesh(&past, None, key([0, 0]), &Clearing::default(), &[]);
    assert!(unknown.masses.is_empty() && !done.tiles);
}

/// Maps over a 4 km window: built-up within 1.2 km of theatre (0, 0), grass elsewhere; the window's chart (0, 0) at
/// theatre (-2000, -2000).
fn tropical_town() -> Context {
    let size = 256u32;
    let extent = 4000.0;
    let texel = extent / f64::from(size);
    let mut cover = vec![30u8; (size * size) as usize];
    for j in 0..size {
        for i in 0..size {
            let x = (f64::from(i) + 0.5) * texel - 2000.0;
            let z = (f64::from(j) + 0.5) * texel - 2000.0;
            if x.hypot(z) < 1200.0 {
                cover[(j * size + i) as usize] = 50;
            }
        }
    }
    let maps = SurfaceMaps {
        origin: [0.0, 0.0],
        extent_m: extent,
        size,
        albedo: vec![0; (size * size * 4) as usize],
        cover,
    };
    let field = Arc::new(naval_terrain::Heightfield {
        columns: 101,
        rows: 101,
        cell: 40.0,
        origin_x: 0.0,
        origin_z: 0.0,
        step: 0.1,
        quanta: vec![80; 101 * 101],
    });
    Context {
        geo: LatLon { lat: 0.0, lon: 0.0 },
        ground: Ground::Battle {
            field,
            origin: [-2000.0, -2000.0],
        },
        cover: Some(Cover {
            maps: Arc::new(maps),
            origin: [-2000.0, -2000.0],
        }),
        region: Region::Tropical,
        style: style(naval_conditions::LandStyle::Tropical),
        period: Period::Today,
        relief: None,
    }
}

#[test]
fn built_up_cover_with_no_footprints_is_filled_in() {
    let context = tropical_town();
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Full,
    };
    let stores: Vec<Arc<QuadStore>> = Vec::new();
    let a = chunk_mesh(&context, key, &stores, &[], None);
    assert!(a.buildings > 100, "{} buildings", a.buildings);
    let b = chunk_mesh(&context, key, &stores, &[], None);
    assert_eq!(a.vertices, b.vertices);
    // Without the cover: nothing (the source has no footprints here).
    let bare = Context {
        cover: None,
        ..context.clone()
    };
    assert_eq!(chunk_mesh(&bare, key, &stores, &[], None).buildings, 0);
    // Footprints of the source in every 100 m cell: the infill leaves the chunk to them.
    let mut store = QuadStore::default();
    let mut fps = Vec::new();
    for j in 0..10 {
        for i in 0..10 {
            let (x, z) = (
                f32::from(i as u8) * 100.0 + 40.0,
                f32::from(j as u8) * 100.0 + 40.0,
            );
            fps.push(Footprint {
                ring: vec![[x, z], [x + 10.0, z], [x + 10.0, z + 10.0], [x, z + 10.0]],
                ..Footprint::default()
            });
        }
    }
    store.chunks.insert([0, 0], fps);
    let stores = vec![Arc::new(QuadStore {
        chunks: store.chunks.clone(),
        footprints: 100,
        covered: true,
    })];
    let held = chunk_mesh(&context, key, &stores, &[], None);
    assert_eq!(held.buildings, 100);
}

/// Taiwan's lots in chunk (0, 0) of the tropical town's built-up cover, laid between the map's buildings (`map`'s
/// outlines merged as the chunk's footprints), as the chunk's mesh lays them.
fn taiwans_lots(map: &MapTags) -> Vec<Footprint> {
    let context = tropical_town();
    let land = Land {
        ground: &context.ground,
        cover: context.cover.as_ref(),
    };
    let mut merged = Vec::new();
    tags::merge_outlines(&mut merged, &map.outlines, &map.outlines_about);
    infill::lots([0, 0], &land, &held_of(&merged, map), &|_| Region::Taiwan)
}

/// Whether a lot's middle lies in the 100 m cell east of (0, 3), beside the neighbours' buildings below.
fn in_cell_13(f: &Footprint) -> bool {
    let m = geometry::centroid(&f.ring);
    (100.0..200.0).contains(&m[0]) && (300.0..400.0).contains(&m[1])
}

/// A neighbour's large building reaching into a chunk is kept clear of by Taiwan's lots laid between the chunk's
/// buildings, however far off its middle: a 580 m shed whose middle lies 210 m west of the chunk, beyond the map's reach
/// of it, is read by its bounds (`MapTags::outlines_reaching`); one stopping 100 m short of the chunk is not read.
#[test]
fn taiwans_lots_keep_clear_of_a_large_neighbour_whose_middle_lies_far_off() {
    use naval_osm::{Building, Features, Polygon};
    // Taoyuan's tile, its middle at theatre (0, 0).
    let near = LatLon {
        lat: 24.99,
        lon: 121.31,
    };
    let id = TileId::of(near.lat, near.lon, 14);
    let geo = to_lat_lon(near, osm::project(near, id, [0.5, 0.5]));
    let n = f64::from(TileId::side(14));
    let frac = |x: f64, z: f64| {
        let ll = to_lat_lon(geo, [x, z]);
        let f = naval_osm::tiles::fraction(ll.lat, ll.lon);
        [f[0] * n - f64::from(id.x), f[1] * n - f64::from(id.y)]
    };
    let building = |x0: f64, z0: f64, x1: f64, z1: f64| Building {
        id: None,
        render_height: None,
        render_min_height: None,
        colour: None,
        hide_3d: false,
        geometry: Geometry::Polygons(vec![Polygon {
            exterior: vec![frac(x0, z0), frac(x1, z0), frac(x1, z1), frac(x0, z1)],
            holes: vec![],
        }]),
    };
    let features = Features {
        building: vec![
            building(-500.0, 300.0, 80.0, 400.0),
            building(-500.0, 500.0, -100.0, 600.0),
            building(90.0, 310.0, 95.0, 315.0),
        ],
        ..Features::default()
    };
    let tile = Arc::new(OsmTile::new(geo, id, &features));
    let map = tags_for(geo, [0, 0], &[tile]);
    assert_eq!(map.outlines.len(), 1);
    assert!(map.outlines_about.is_empty());
    assert_eq!(map.outlines_reaching.len(), 1);
    let shed = &map.outlines_reaching[0].ring;
    // Lots stand beside it (the cell it covers four fifths of, (0, 3), is the source's), none on it.
    let lots = taiwans_lots(&map);
    assert!(lots.iter().any(in_cell_13));
    assert!(lots.iter().all(|f| !geometry::overlaps(&f.ring, shed)));
}

/// A neighbour's outline a quarter-metre up (OSM's `min_height`) stands on the ground in its own chunk (under
/// `tags::PART_BASE_M`), and Taiwan's lots keep clear of it where it reaches in.
#[test]
fn taiwans_lots_keep_clear_of_a_neighbours_outline_a_little_off_the_ground() {
    let outline = |x0: f32, z0: f32, x1: f32, z1: f32, min_height: f32| Outline {
        ring: vec![[x0, z0], [x1, z0], [x1, z1], [x0, z1]],
        height: Some(9.0),
        min_height,
        colour: None,
        hide_3d: false,
    };
    // In its own chunk, (-1, 0): a building on the ground, not a part.
    let mut own = Vec::new();
    tags::merge_outlines(&mut own, &[outline(900.0, 300.0, 1080.0, 400.0, 0.25)], &[]);
    assert_eq!(own.len(), 1);
    assert!(own[0].part.is_none());
    let wide = outline(-100.0, 300.0, 80.0, 400.0, 0.25);
    let map = MapTags {
        outlines: vec![outline(90.0, 310.0, 95.0, 315.0, 0.0)],
        outlines_about: vec![wide.clone()],
        ..MapTags::default()
    };
    let lots = taiwans_lots(&map);
    assert!(lots.iter().any(in_cell_13));
    assert!(
        lots.iter()
            .all(|f| !geometry::overlaps(&f.ring, &wide.ring))
    );
}

/// The tests handing the host's clear areas (one list for the whole program), one at a time.
static CLEAR_AREAS: Mutex<()> = Mutex::new(());

#[test]
fn the_hosts_clear_areas_leave_no_building_at_any_level_and_remake_their_chunks() {
    let _one = CLEAR_AREAS.lock().unwrap_or_else(|e| e.into_inner());
    // Its own frame (no other test's chunks under its areas): the tropical town moved to 10° N, 10° E.
    let geo = LatLon {
        lat: 10.0,
        lon: 10.0,
    };
    let context = Context {
        geo,
        ..tropical_town()
    };
    let key = |lod| ChunkKey { index: [0, 0], lod };
    // A 10 m footprint in each 100 m cell.
    let mut fps = Vec::new();
    for j in 0..10 {
        for i in 0..10 {
            let (x, z) = (
                f32::from(i as u8) * 100.0 + 40.0,
                f32::from(j as u8) * 100.0 + 40.0,
            );
            fps.push(Footprint {
                ring: vec![[x, z], [x + 10.0, z], [x + 10.0, z + 10.0], [x, z + 10.0]],
                ..Footprint::default()
            });
        }
    }
    let stores = vec![Arc::new(QuadStore {
        chunks: HashMap::from([([0, 0], fps)]),
        footprints: 100,
        covered: true,
    })];
    let none: Vec<Arc<QuadStore>> = Vec::new();
    set_clear_areas(Vec::new());
    let made = |lod| chunk_mesh(&context, key(lod), &stores, &[], None);
    assert_eq!(made(Lod::Full).buildings, 100);
    let infill = chunk_mesh(&context, key(Lod::Full), &none, &[], None);
    let (far, _) = far_mesh(&context, None, key(Lod::Far), &clearing(), &[]);
    let made_then = Made {
        mesh: Arc::new(infill.clone()),
        complete: true,
        at: Instant::now(),
        awaiting: false,
        epoch: split_epoch(),
        answered: 0,
        relief: Some(0),
        cleared: clearing().signature(geo, [0, 0]),
        towers: None,
    };
    assert!(!made_then.stale_for(&context, key(Lod::Full)));
    // A circle 120 m about theatre (500, 500) and a square about (150, 850).
    let at = |x: f64, z: f64| to_lat_lon(geo, [x, z]);
    set_clear_areas(vec![
        ClearArea::Circle {
            middle: at(500.0, 500.0),
            radius_m: 120.0,
        },
        ClearArea::Polygon(vec![
            at(100.0, 800.0),
            at(200.0, 800.0),
            at(200.0, 900.0),
            at(100.0, 900.0),
        ]),
    ]);
    // The chunk under them is made again; one beyond is not.
    assert!(made_then.stale_for(&context, key(Lod::Full)));
    assert_eq!(clearing().signature(geo, [3, 3]), 0);
    // The four footprints whose middles lie in the circle and the one in the square, at both levels.
    assert_eq!(made(Lod::Full).buildings, 95);
    assert_eq!(made(Lod::Box).buildings, 95);
    // The infill leaves the areas empty: no wall or roof in them.
    let filled = chunk_mesh(&context, key(Lod::Full), &none, &[], None);
    assert!(filled.buildings < infill.buildings);
    // Within them by more than a few metres (a far run is judged by its middle lot, its mass a little off it).
    let inside = |p: [f32; 2]| {
        (f64::from(p[0]) - 500.0).hypot(f64::from(p[1]) - 500.0) < 115.0
            || ((105.0..195.0).contains(&p[0]) && (805.0..895.0).contains(&p[1]))
    };
    assert!(
        infill
            .vertices
            .iter()
            .any(|v| inside([v.position[0], v.position[2]]))
    );
    assert!(
        !filled
            .vertices
            .iter()
            .any(|v| inside([v.position[0], v.position[2]]))
    );
    // Nor a far mass.
    let (thinned, _) = far_mesh(&context, None, key(Lod::Far), &clearing(), &[]);
    let middle = |m: &naval_terrain::buildings::far::Mass| m.at();
    assert!(far.masses.iter().any(|m| inside(middle(m))));
    assert!(thinned.masses.len() < far.masses.len());
    let left: Vec<([f32; 2], bool)> = thinned
        .masses
        .iter()
        .filter(|m| inside(middle(m)))
        .map(|m| (m.at(), m.is_block()))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    // Taken away: as before, and made again.
    set_clear_areas(Vec::new());
    assert!(clear_areas().is_empty());
    assert_eq!(made(Lod::Full).buildings, 100);
    assert!(!made_then.stale_for(&context, key(Lod::Full)));
}

#[test]
fn a_drawn_chunk_whose_mesh_left_the_cache_is_made_again_under_new_clear_areas() {
    let _one = CLEAR_AREAS.lock().unwrap_or_else(|e| e.into_inner());
    // Its own frame: the tropical town at 11° N, 10° E.
    let geo = LatLon {
        lat: 11.0,
        lon: 10.0,
    };
    let context = Context {
        geo,
        ..tropical_town()
    };
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Full,
    };
    set_clear_areas(Vec::new());
    let made = Made {
        mesh: Arc::new(ChunkMesh::default()),
        complete: true,
        at: Instant::now(),
        awaiting: false,
        epoch: split_epoch(),
        answered: 0,
        relief: Some(context.relief_under(key)),
        cleared: clearing().signature(geo, key.index),
        towers: None,
    };
    // Evicted from the cache while still drawn: what it was made on is kept, and an area handed over it makes it again.
    let evicted = made.evicted();
    assert!(!evicted.stale_for(&context, key));
    set_clear_areas(vec![ClearArea::Circle {
        middle: to_lat_lon(geo, [500.0, 500.0]),
        radius_m: 50.0,
    }]);
    assert!(evicted.stale_for(&context, key));
    // Not one beyond the area.
    let beyond = ChunkKey {
        index: [4, 4],
        ..key
    };
    let quiet = Evicted {
        relief: Some(context.relief_under(beyond)),
        cleared: 0,
        towers: None,
    };
    assert!(!quiet.stale_for(&context, beyond));
    // Taken away again: as it was made.
    set_clear_areas(Vec::new());
    assert!(!evicted.stale_for(&context, key));
}

#[test]
fn a_chunk_is_made_and_marked_under_one_reading_of_the_clear_areas() {
    let _one = CLEAR_AREAS.lock().unwrap_or_else(|e| e.into_inner());
    // Its own frame: the tropical town at 12° N, 10° E, a footprint at (500, 500).
    let geo = LatLon {
        lat: 12.0,
        lon: 10.0,
    };
    let context = Context {
        geo,
        ..tropical_town()
    };
    let key = |lod| ChunkKey { index: [0, 0], lod };
    let stores = vec![Arc::new(QuadStore {
        chunks: HashMap::from([(
            [0, 0],
            vec![Footprint {
                ring: vec![
                    [495.0, 495.0],
                    [505.0, 495.0],
                    [505.0, 505.0],
                    [495.0, 505.0],
                ],
                ..Footprint::default()
            }],
        )]),
        footprints: 1,
        covered: true,
    })];
    let circle = || {
        vec![ClearArea::Circle {
            middle: to_lat_lon(geo, [500.0, 500.0]),
            radius_m: 120.0,
        }]
    };
    // A batch begun with no areas; a circle over the footprint handed before it is made, and taken away before it is
    // done. Made as begun, the footprint (and the infill about it) and the far masses stand, and the mesh is marked so.
    set_clear_areas(Vec::new());
    let begun = clearing();
    set_clear_areas(circle());
    let inside = |p: [f32; 2]| (f64::from(p[0]) - 500.0).hypot(f64::from(p[1]) - 500.0) < 100.0;
    let made = |c: &Clearing| {
        let mesh = chunk_mesh_under(&context, key(Lod::Full), &stores, &[], None, c);
        let near = mesh
            .vertices
            .iter()
            .any(|v| inside([v.position[0], v.position[2]]));
        let (far, _) = far_mesh(&context, None, key(Lod::Far), c, &[]);
        (near, mesh.buildings, far.masses.len())
    };
    let (stood, buildings, masses) = made(&begun);
    assert!(stood);
    let marked = Made {
        mesh: Arc::new(ChunkMesh::default()),
        complete: true,
        at: Instant::now(),
        awaiting: false,
        epoch: split_epoch(),
        answered: 0,
        relief: Some(context.relief_under(key(Lod::Full))),
        cleared: begun.signature(geo, [0, 0]),
        towers: None,
    };
    // Made again while the circle stands; right as it is once it is taken away.
    assert!(marked.stale_for(&context, key(Lod::Full)));
    let under = clearing();
    set_clear_areas(Vec::new());
    assert!(!marked.stale_for(&context, key(Lod::Full)));
    // Made under the circle (whatever is handed since): the footprint, the infill and the masses in it gone.
    let (near, fewer, fewer_masses) = made(&under);
    assert!(!near);
    assert!(fewer < buildings, "{fewer} {buildings}");
    assert!(fewer_masses < masses, "{fewer_masses} {masses}");
}

#[test]
fn chunks_without_microsofts_footprints_agree_along_their_edges() {
    // The tropical town about theatre (0, 0), scored by the infill its cover raises: chunk (0, 0) meets (-1, 0) along
    // x = 0 and (0, -1) along z = 0, all three in the town.
    let context = Context {
        period: Period::Year(1942),
        ..tropical_town()
    };
    let here = period::by_cover(&context, [0, 0]);
    let west = period::by_cover(&context, [-1, 0]);
    let north = period::by_cover(&context, [0, -1]);
    let mut built = 0;
    for k in 0..=20 {
        let t = k as f32 * 50.0;
        let pairs = [
            (west.score_at([1000.0, t]), here.score_at([0.0, t])),
            (north.score_at([t, 1000.0]), here.score_at([t, 0.0])),
        ];
        for (a, b) in pairs {
            assert!((a - b).abs() < 1e-4, "at {t} m: {a} and {b}");
            built += usize::from(a > 0.05);
        }
    }
    assert!(built > 20, "{built} of 42 points scored built over");
    // Out of the town, nothing but the noise.
    let far = period::by_cover(&context, [5, 5]).score_at([500.0, 500.0]);
    assert!(far < 0.1, "{far}");
}

#[test]
fn a_past_period_thins_a_chunk_by_its_prepared_thinning() {
    use naval_terrain::buildings::thinning::{self, Thinning};
    let today = tropical_town();
    let past = Context {
        period: Period::Year(1942),
        ..today.clone()
    };
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Full,
    };
    let stores: Vec<Arc<QuadStore>> = Vec::new();
    // No footprints of Microsoft's about: the chunk's own (the infill's lots) score it.
    let prepared = |bar: f32| period::Prepared {
        thinning: Thinning::new(
            &thinning::chunk_window(key.index, &[None; 9]),
            vec![bar; 100],
            12.0,
        ),
        own: true,
        lacking: period::Lacking::default(),
    };
    let all = chunk_mesh(&today, key, &stores, &[], None);
    // Today's ignores a thinning handed in.
    let same = chunk_mesh(&today, key, &stores, &[], Some(&prepared(10.0)));
    assert_eq!(same.vertices, all.vertices);
    let kept = |bar: f32| chunk_mesh(&past, key, &stores, &[], Some(&prepared(bar))).buildings;
    assert_eq!(kept(10.0), 0);
    let loose = kept(-1.0);
    assert!(
        loose * 10 >= all.buildings * 9,
        "{loose} of {}",
        all.buildings
    );
    // Fewer as the bar rises; at a bar between, the town's middle by theatre (0, 0) kept and its edge gone.
    let bars = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8];
    let counts: Vec<u32> = bars.iter().map(|&b| kept(b)).collect();
    assert!(
        counts.windows(2).all(|w| w[1] <= w[0]) && counts[0] <= loose,
        "{counts:?}"
    );
    let Some(k) = counts.iter().position(|&c| c > 0 && c * 10 < loose * 8) else {
        panic!("no bar between: {counts:?} of {loose}");
    };
    let some = chunk_mesh(&past, key, &stores, &[], Some(&prepared(bars[k])));
    let far = some
        .vertices
        .iter()
        .filter(|v| v.position[0].hypot(v.position[2]) > 1100.0)
        .count();
    let near = some
        .vertices
        .iter()
        .filter(|v| v.position[0].hypot(v.position[2]) < 600.0)
        .count();
    assert!(near > 0 && far == 0, "near {near}, far {far}");
    // Made again: the same.
    let again = chunk_mesh(&past, key, &stores, &[], Some(&prepared(bars[k])));
    assert_eq!(again.vertices, some.vertices);
    // Today's prepares nothing.
    let (none, fetched) = period::prepare(&today, &[key], &[true]);
    assert!(none[0].is_none() && fetched == 0);
}

/// A tile of OpenStreetMap's about Monterey (made here, not fetched): industrial land with a footprint on it, a church's
/// point on another, a lighthouse known by its name where no footprint stands, an outline Microsoft's data misses
/// (measured, coloured), a pier. The chunk's tags hold them in its own metres, held to the tile; the chunk's buildings
/// are typed from them.
#[test]
fn the_maps_tags_type_the_chunks_buildings() {
    use naval_osm::{Building, Features, Landuse, Poi, Polygon, Transportation};
    let geo = LatLon {
        lat: 36.6,
        lon: -121.9,
    };
    let id = TileId::of(36.6, -121.9, 14);
    let index = chunk_of(osm::project(geo, id, [0.5, 0.5]));
    let corner = chunk_corner(index);
    let middle = [corner[0] + 500.0, corner[1] + 500.0];
    // Theatre metres about the chunk's middle to the tile's fractions.
    let n = f64::from(TileId::side(14));
    let frac = |dx: f64, dz: f64| {
        let ll = to_lat_lon(geo, [middle[0] + dx, middle[1] + dz]);
        let f = naval_osm::tiles::fraction(ll.lat, ll.lon);
        [f[0] * n - f64::from(id.x), f[1] * n - f64::from(id.y)]
    };
    let square = |dx: f64, dz: f64, w: f64, d: f64| {
        vec![
            frac(dx, dz),
            frac(dx + w, dz),
            frac(dx + w, dz + d),
            frac(dx, dz + d),
        ]
    };
    let poi = |class: &str, subclass: &str, name: Option<&str>, dx: f64, dz: f64| Poi {
        class: class.into(),
        subclass: Some(subclass.into()),
        name: name.map(str::to_owned),
        rank: None,
        layer: 0,
        level: 0,
        geometry: Geometry::Points(vec![frac(dx, dz)]),
    };
    let features = Features {
        landuse: vec![Landuse {
            class: "industrial".into(),
            geometry: Geometry::Polygons(vec![Polygon {
                exterior: square(-400.0, -400.0, 200.0, 200.0),
                holes: vec![],
            }]),
        }],
        poi: vec![
            poi(
                "place_of_worship",
                "christian",
                Some("St Test"),
                115.0,
                107.0,
            ),
            poi(
                "attraction",
                "attraction",
                Some("Test Point Lighthouse"),
                300.0,
                -300.0,
            ),
            poi("bank", "bank", Some("Lighthouse Bank"), -200.0, 300.0),
        ],
        building: vec![Building {
            id: Some(1),
            render_height: Some(40.0),
            render_min_height: Some(0.0),
            colour: Some("red".into()),
            hide_3d: false,
            geometry: Geometry::Polygons(vec![Polygon {
                exterior: square(-100.0, 200.0, 20.0, 20.0),
                holes: vec![],
            }]),
        }],
        transportation: vec![Transportation {
            class: "pier".into(),
            subclass: None,
            brunnel: naval_osm::Brunnel::None,
            surface: None,
            layer: 0,
            ramp: false,
            oneway: 0,
            expressway: false,
            service: None,
            network: None,
            geometry: Geometry::Lines(vec![vec![frac(-450.0, 400.0), frac(-350.0, 400.0)]]),
        }],
        ..Features::default()
    };
    let tile = Arc::new(OsmTile::new(geo, id, &features));
    let tags = tags_for(geo, index, &[tile]);
    assert_eq!(tags.zones.len(), 1);
    assert_eq!(tags.zones[0].usage, Usage::Industrial);
    assert_eq!(tags.pois.len(), 2, "{:?}", tags.pois);
    assert_eq!(tags.outlines.len(), 1);
    assert_eq!(tags.outlines[0].height, Some(40.0));
    assert_eq!(tags.outlines[0].colour, Some([255, 0, 0]));
    assert_eq!(tags.piers.len(), 1);
    // The zone in the chunk's metres: about its middle less 400 m.
    let z = &tags.zones[0];
    assert!(
        (z.bounds[0] - 100.0).abs() < 1.0 && (z.bounds[2] - 300.0).abs() < 1.0,
        "{:?}",
        z.bounds
    );

    // Microsoft's footprints: a large one on the industrial land, the church's under its point.
    let rect = |x: f32, z: f32, w: f32, d: f32| Footprint {
        ring: vec![[x, z], [x + w, z], [x + w, z + d], [x, z + d]],
        ..Footprint::default()
    };
    let mut footprints = vec![
        rect(150.0, 150.0, 50.0, 30.0),
        rect(600.0, 600.0, 30.0, 14.0),
    ];
    tags::merge_outlines(&mut footprints, &tags.outlines, &tags.outlines_about);
    assert_eq!(footprints.len(), 3);
    let mesh = build_chunk(&ChunkInput {
        key: ChunkKey {
            index,
            lod: Lod::Full,
        },
        footprints: &footprints,
        infill: &[],
        tags: &tags,
        ground: &|_| 5.0,
        region: Region::Mediterranean,
        period: naval_terrain::buildings::Period::Today,
        thinning: None,
    });
    let kinds: HashSet<u8> = mesh.vertices.iter().map(|v| v.style[3] & 15).collect();
    use naval_terrain::buildings::Kind;
    assert!(
        kinds.contains(&(Kind::Industrial as u8)) || kinds.contains(&(Kind::Warehouse as u8)),
        "{kinds:?}"
    );
    assert!(kinds.contains(&(Kind::Church as u8)), "{kinds:?}");
    assert!(kinds.contains(&(Kind::Lighthouse as u8)), "{kinds:?}");
    assert!(kinds.contains(&(Kind::Deck as u8)), "{kinds:?}");
    // OSM's 40 m outline stands at its height, in its colour (the cube's red), a parapet of at most 1.1 m over it.
    let red = naval_terrain::buildings::cube_entry([255, 0, 0]);
    let top = mesh
        .vertices
        .iter()
        .filter(|v| v.style[0] == red)
        .map(|v| v.position[1])
        .fold(0.0, f32::max);
    assert!((44.5..46.6).contains(&top), "{top}");
    // The same chunk with no tiles: the landmarks alone (none here), the footprints typed by their shape.
    assert!(tags_for(geo, index, &[]).is_empty());
}

/// The landmarks stand where they are: Point Pinos's lighthouse in its chunk about the California scenario's origin.
#[test]
fn landmarks_stand_in_their_chunks() {
    let geo = LatLon {
        lat: 36.936204,
        lon: -122.418494,
    };
    let pinos = to_local(
        geo,
        LatLon {
            lat: 36.633389,
            lon: -121.933737,
        },
    );
    let index = chunk_of(pinos);
    let tags = tags_for(geo, index, &[]);
    assert_eq!(tags.landmarks.len(), 1);
    let at = tags.landmarks[0].at;
    let corner = chunk_corner(index);
    assert!((f64::from(at[0]) + corner[0] - pinos[0]).abs() < 0.01);
    assert!(matches!(
        tags.landmarks[0].kind,
        LandmarkKind::Lighthouse { keeper: true, .. }
    ));
    // Moss Landing's six stacks share a chunk or two.
    let moss = chunk_of(to_local(
        geo,
        LatLon {
            lat: 36.8047,
            lon: -121.7800,
        },
    ));
    let stacks: usize = (-1..=1)
        .flat_map(|dj| (-1..=1).map(move |di| [moss[0] + di, moss[1] + dj]))
        .map(|i| tags_for(geo, i, &[]).landmarks.len())
        .sum();
    assert_eq!(stacks, 6);
}

/// A tile of the map's over the tropical town (made here, not fetched): a tower of 250 m on commercial land, one
/// mapped as its parts (an outline drawn by them, a podium and a shaft to 150 m), a low block, and land use away from
/// them. The towers' reader keeps the towers alone, and the far chunk stands them as its boxes, the masses off them.
#[test]
fn a_far_chunk_stands_the_maps_towers() {
    use naval_osm::{Building, Features, Landuse, Polygon};
    let context = tropical_town();
    let geo = context.geo;
    let id = TileId::of(-0.004, 0.004, 14);
    let n = f64::from(TileId::side(14));
    let frac = |x: f64, z: f64| {
        let ll = to_lat_lon(geo, [x, z]);
        let f = naval_osm::tiles::fraction(ll.lat, ll.lon);
        [f[0] * n - f64::from(id.x), f[1] * n - f64::from(id.y)]
    };
    let square = |x: f64, z: f64, side: f64| {
        let h = side / 2.0;
        Geometry::Polygons(vec![Polygon {
            exterior: vec![
                frac(x - h, z - h),
                frac(x + h, z - h),
                frac(x + h, z + h),
                frac(x - h, z + h),
            ],
            holes: vec![],
        }])
    };
    let building = |x: f64, z: f64, side: f64, height: f64, base: f64, hide_3d: bool| Building {
        id: None,
        render_height: Some(height),
        render_min_height: Some(base),
        colour: None,
        hide_3d,
        geometry: square(x, z, side),
    };
    let features = Features {
        building: vec![
            building(500.0, 500.0, 40.0, 250.0, 0.0, false),
            building(300.0, 300.0, 30.0, 30.0, 0.0, false),
            building(700.0, 700.0, 60.0, 150.0, 0.0, true),
            building(700.0, 700.0, 60.0, 30.0, 0.0, false),
            building(700.0, 700.0, 30.0, 150.0, 30.0, false),
        ],
        landuse: vec![
            Landuse {
                class: "commercial".into(),
                geometry: square(500.0, 500.0, 200.0),
            },
            Landuse {
                class: "residential".into(),
                geometry: square(200.0, 200.0, 150.0),
            },
        ],
        ..Features::default()
    };
    let reduced = towers::reduce(&features);
    assert_eq!(reduced.building.len(), 4, "the low block left out");
    assert!(
        reduced
            .building
            .iter()
            .all(|b| b.render_height != Some(30.0) || b.geometry != square(300.0, 300.0, 30.0))
    );
    assert_eq!(reduced.landuse.len(), 1);
    assert_eq!(reduced.landuse[0].class, "commercial");
    assert!(
        towers::reduce(&Features {
            building: vec![building(300.0, 300.0, 30.0, 30.0, 0.0, false)],
            ..Features::default()
        })
        .building
        .is_empty()
    );
    let tile = Arc::new(OsmTile::new(geo, id, &reduced));
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Far,
    };
    let (mesh, done) = far_mesh(
        &context,
        None,
        key,
        &Clearing::default(),
        std::slice::from_ref(&tile),
    );
    assert!(done.tiles);
    // The tower at its height over its ground, an office on its commercial land; the parts' shaft to 150 m.
    let top = |x: f32, z: f32, r: f32| {
        mesh.vertices
            .iter()
            .filter(|v| (v.position[0] - x).hypot(v.position[2] - z) < r)
            .map(|v| v.position[1])
            .reduce(f32::max)
    };
    let ground = context.footing([500.0, 500.0]) as f32;
    let tower = top(500.0, 500.0, 30.0).expect("the tower");
    assert!(
        (tower - ground - 250.0).abs() < 1.5,
        "{tower} over {ground}"
    );
    let shaft = top(700.0, 700.0, 25.0).expect("the shaft");
    assert!((shaft - ground - 150.0).abs() < 1.5, "{shaft}");
    assert!(top(300.0, 300.0, 25.0).is_none());
    use naval_terrain::buildings::Kind;
    let office = mesh
        .vertices
        .iter()
        .filter(|v| (v.position[0] - 500.0).hypot(v.position[2] - 500.0) < 30.0)
        .all(|v| v.style[3] & 15 == Kind::Office as u8);
    assert!(office);
    // No mass on either.
    for (x, z, side) in [(500.0f32, 500.0f32, 40.0f32), (700.0, 700.0, 60.0)] {
        let h = side / 2.0 - 1.0;
        assert!(
            !mesh.masses.iter().filter(|m| !m.is_block()).any(|m| {
                let [mx, mz] = m.at();
                (mx - x).abs() < h && (mz - z).abs() < h
            }),
            "a run on the tower at {x}, {z}"
        );
    }
    // Made again the same; without its tile, no tower and made again once the tile is in.
    let (again, _) = far_mesh(&context, None, key, &Clearing::default(), &[tile]);
    assert_eq!(
        (&again.vertices, &again.masses),
        (&mesh.vertices, &mesh.masses)
    );
    let (bare, _) = far_mesh(&context, None, key, &Clearing::default(), &[]);
    assert!(bare.vertices.is_empty() && bare.masses.len() > mesh.masses.len());
}

/// A far chunk's towers stand on the relief as drawn (their boxes' footing): made again when a page under it comes in or
/// is made again, as a boxed chunk is, held or evicted from the cache; a far chunk of masses alone, on the window's
/// heights, is not.
#[test]
fn a_far_chunk_with_towers_is_made_again_when_the_relief_under_it_changes() {
    use naval_terrain::{ReliefGrid, ReliefPage, relief::PAGE_TEXELS};
    let _one = CLEAR_AREAS.lock().unwrap_or_else(|e| e.into_inner());
    let grid = ReliefGrid {
        origin: [0.0, 0.0],
        texel_m: 10.0,
        slots: 2,
    };
    let page = |m: f32| {
        Arc::new(ReliefPage::from_residual(
            &vec![m; (PAGE_TEXELS * PAGE_TEXELS) as usize],
            10.0,
        ))
    };
    let on = |page| Context {
        relief: Some(naval_terrain::ReliefHeights::of_pages(
            grid,
            [([0, 0], page)],
        )),
        ..tropical_town()
    };
    let (flat, risen) = (on(page(0.0)), on(page(5.0)));
    let far = ChunkKey {
        index: [0, 0],
        lod: Lod::Far,
    };
    // The page raises the towers' ground.
    assert!(risen.footing([500.0, 500.0]) > flat.footing([500.0, 500.0]) + 4.0);
    let towers = ChunkMesh {
        indices: vec![0, 1, 2],
        ..ChunkMesh::default()
    };
    let masses = ChunkMesh::default();
    let made = |mesh: &ChunkMesh| Made {
        mesh: Arc::new(mesh.clone()),
        complete: true,
        at: Instant::now(),
        awaiting: false,
        epoch: split_epoch(),
        answered: 0,
        relief: flat.relief_mark(far, mesh),
        cleared: clearing().signature(flat.geo, far.index),
        towers: Some(towers::epoch()),
    };
    assert_ne!(flat.relief_under(far), 0);
    assert_eq!(made(&masses).relief, None);
    assert!(!made(&towers).stale_for(&flat, far));
    assert!(made(&towers).stale_for(&risen, far));
    assert!(made(&towers).evicted().stale_for(&risen, far));
    assert!(!made(&masses).stale_for(&risen, far));
    assert!(!made(&masses).evicted().stale_for(&risen, far));
}

/// A drawn far chunk whose mesh left the cache before the towers' tile it lacked landed is made again once it lands, not
/// a minute later; one made with everything it wanted is not.
#[test]
fn a_drawn_far_chunk_whose_mesh_left_the_cache_is_made_again_when_its_towers_land() {
    let _one = CLEAR_AREAS.lock().unwrap_or_else(|e| e.into_inner());
    let context = tropical_town();
    let key = ChunkKey {
        index: [0, 0],
        lod: Lod::Far,
    };
    let made = |complete| Made {
        mesh: Arc::new(ChunkMesh::default()),
        complete,
        at: Instant::now(),
        awaiting: false,
        epoch: split_epoch(),
        answered: 0,
        relief: None,
        cleared: clearing().signature(context.geo, key.index),
        towers: Some(towers::epoch()),
    };
    let (lacking, whole) = (made(false).evicted(), made(true).evicted());
    assert!(!lacking.stale_for(&context, key) && !whole.stale_for(&context, key));
    towers::landed();
    assert!(lacking.stale_for(&context, key));
    assert!(!whole.stale_for(&context, key));
}

/// Measures the far level's towers over the map's tiles on disk (`osm::cache_dir`) within 30 km of a place
/// (`NAVAL_TOWERS_AT=lat,lon`, Taipei 101 by default): the tiles held, read and reduced, the towers kept, their memory,
/// and their boxes' meshing over the far chunks. `cargo test --profile dev-client -p naval-landscape --lib
/// measure_far_towers -- --ignored --nocapture`.
#[test]
#[ignore = "reads the map's tiles on disk"]
fn measure_far_towers() {
    let (lat, lon) = std::env::var("NAVAL_TOWERS_AT")
        .ok()
        .and_then(|v| {
            let (a, b) = v.split_once(',')?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        })
        .unwrap_or((25.0340, 121.5645));
    let geo = LatLon { lat, lon };
    let reach = 30_000.0;
    let ids = osm::tiles_over(geo, [-reach, -reach], [reach, reach], 14);
    let ids: Vec<TileId> = ids
        .into_iter()
        .filter(|&id| {
            let p = osm::project(geo, id, [0.5, 0.5]);
            p[0].hypot(p[1]) <= reach + 1500.0
        })
        .collect();
    let mut on_disk = 0;
    let mut bytes_on_disk = 0;
    let mut decode_s = 0.0;
    let mut whole_s = 0.0;
    let mut tiles = Vec::new();
    for &id in &ids {
        let Some(bytes) = osm::cached(id) else {
            continue;
        };
        let bytes = bytes.unwrap_or_default();
        on_disk += 1;
        bytes_on_disk += bytes.len();
        let t = Instant::now();
        let Some(tile) = towers::decode(geo, id, &bytes) else {
            continue;
        };
        decode_s += t.elapsed().as_secs_f64();
        let t = Instant::now();
        let _ = OsmTile::decode(geo, id, &bytes);
        whole_s += t.elapsed().as_secs_f64();
        tiles.push(Arc::new(tile));
    }
    let outlines: usize = tiles.iter().map(|t| t.features.building.len()).sum();
    let kept_bytes: usize = tiles.iter().map(|t| t.bytes).sum();
    let tall: usize = tiles
        .iter()
        .flat_map(|t| &t.features.building)
        .filter(|b| {
            b.render_height
                .is_some_and(|h| h >= f64::from(far::TOWER_M))
        })
        .count();
    eprintln!(
        "{lat},{lon}: {} tiles within 30 km, {on_disk} on disk ({:.1} MB); towers' read {:.0} ms ({:.2} ms a tile), the \
         map's whole decode {:.0} ms; {outlines} outlines kept ({tall} of {} m and more), {:.2} MB held",
        ids.len(),
        bytes_on_disk as f64 / 1e6,
        decode_s * 1e3,
        decode_s * 1e3 / on_disk.max(1) as f64,
        whole_s * 1e3,
        far::TOWER_M,
        kept_bytes as f64 / 1e6,
    );
    // The towers' boxes over the far chunks (10-30 km), on flat ground with no cover (no masses): the meshing and the
    // GPU bytes, through `far::build_far` as the far chunks make them.
    struct Bare;
    impl infill::Land for Bare {
        fn cover(&self, _: [f64; 2]) -> u8 {
            30
        }
        fn built_share(&self, _: [f64; 2], _: f64) -> f32 {
            0.0
        }
        fn height(&self, _: [f64; 2]) -> f32 {
            10.0
        }
    }
    let landuse_points: usize = tiles
        .iter()
        .flat_map(|t| &t.features.landuse)
        .map(|l| l.geometry.points().count())
        .sum();
    let start = Instant::now();
    let (mut chunks, mut with, mut triangles, mut gpu, mut most, mut buildings) =
        (0, 0, 0, 0, 0, 0);
    for j in -31..31 {
        for i in -31..31 {
            let index = [i, j];
            let c = chunk_corner(index);
            let d = (c[0] + 500.0).hypot(c[1] + 500.0);
            if !(10_000.0..=30_000.0).contains(&d) {
                continue;
            }
            chunks += 1;
            let key = ChunkKey {
                index,
                lod: Lod::Far,
            };
            let ids = osm_tiles_of(geo, key, Period::Today);
            let have: Vec<Arc<OsmTile>> = tiles
                .iter()
                .filter(|t| ids.contains(&t.id))
                .cloned()
                .collect();
            let mut tags = tags_for(geo, index, &have);
            tags.landmarks.clear();
            let mut footprints = Vec::new();
            tags::merge_outlines(&mut footprints, &tags.outlines, &tags.outlines_about);
            if footprints.is_empty() {
                continue;
            }
            let mesh = far::build_far(&far::FarInput {
                index,
                land: &Bare,
                stood: &|_| true,
                region: &|_| Region::Taiwan,
                chunk_region: Region::Taiwan,
                period: Period::Today,
                cap_m: 400.0,
                towers: Some(far::FarTowers {
                    footprints: &footprints,
                    about: &[],
                    tags: &tags,
                    ground: &|_| 10.0,
                    thinning: None,
                }),
            });
            if !mesh.indices.is_empty() {
                with += 1;
                triangles += mesh.triangles();
                gpu += mesh.bytes();
                most = most.max(mesh.bytes());
                buildings += mesh.buildings;
            }
        }
    }
    let building_points: usize = tiles
        .iter()
        .flat_map(|t| &t.features.building)
        .map(|b| b.geometry.points().count())
        .sum();
    let most_points = tiles
        .iter()
        .flat_map(|t| &t.features.building)
        .map(|b| b.geometry.points().count())
        .max()
        .unwrap_or(0);
    eprintln!(
        "kept: the land use's {landuse_points} points, the buildings' {building_points} (at most {most_points})"
    );
    eprintln!(
        "far chunks 10-30 km: {chunks}, {with} with towers ({buildings} buildings and parts), {triangles} triangles, \
         {:.2} MB on the GPU ({:.1} KB the most a chunk), meshed in {:.0} ms on one thread (the tags and merge \
         counted)",
        gpu as f64 / 1e6,
        most as f64 / 1e3,
        start.elapsed().as_secs_f64() * 1e3
    );
}

/// Taiwan's lots keep off all the land the map's roads take: a lot whose end reaches a carriageway's stroke, one in the
/// median between a pair of one-way carriageways, one on a footway mapped beside them; one behind the pavement stands.
/// Every other region's lots are left out only where a road runs through their middle, as before: all four stand.
#[test]
fn only_taiwans_lots_keep_off_a_roads_pavement_median_and_footway() {
    let road = |z: f32, half: f32, oneway: bool| tags::Road {
        a: [-200.0, z],
        b: [600.0, z],
        half,
        oneway,
    };
    let tags = MapTags {
        roads: vec![road(100.0, 4.5, true), road(124.0, 4.5, true)],
        paths: vec![road(86.0, 1.5, false)],
        ..MapTags::default()
    };
    let lot = |x: f32, z0: f32, z1: f32| Footprint {
        ring: vec![[x, z0], [x + 5.0, z0], [x + 5.0, z1], [x, z1]],
        ..Footprint::default()
    };
    let lots = vec![
        // Its end 2 m into the second carriageway's stroke, its middle 10 m off its middle line.
        lot(0.0, 126.5, 141.5),
        lot(20.0, 109.0, 115.0),
        lot(40.0, 74.0, 89.0),
        lot(60.0, 66.0, 81.0),
    ];
    let regions = [
        Region::Tropical,
        Region::Arid,
        Region::NorthAfrica,
        Region::Micronesia,
        Region::Atoll,
        Region::American,
        Region::Florida,
        Region::Hawaii,
        Region::West,
        Region::EastAsia,
        Region::Nordic,
        Region::Temperate,
        Region::Mediterranean,
        Region::Iceland,
        Region::British,
        Region::Celtic,
        Region::NorthSea,
        Region::Malta,
        Region::Aegean,
        Region::Nile,
    ];
    for region in regions {
        assert!(!infill::keeps_off_road_land(region));
        let mut kept = lots.clone();
        off_roads(&mut kept, &tags, region);
        assert_eq!(kept, lots, "{region:?}");
    }
    let mut kept = lots.clone();
    off_roads(&mut kept, &tags, Region::Taiwan);
    assert_eq!(kept, vec![lots[3].clone()]);
}
