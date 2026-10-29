use super::*;

#[test]
fn the_gsi_tiles_read_in_hundredths_with_none_at_two_to_the_23rd() {
    // 1 m, none, -0.01 m, 1,000 m, the highest and lowest the encoding holds.
    let rgb = [
        0, 0, 100, 128, 0, 0, 255, 255, 255, 0x01, 0x86, 0xa0, 0x7f, 0xff, 0xff, 0x80, 0x00, 0x01,
    ];
    let h = gsi_heights(&rgb);
    assert_eq!(h[0], 1.0);
    assert!(h[1].is_nan());
    assert!((h[2] + 0.01).abs() < 1e-6);
    assert!((h[3] - 1000.0).abs() < 1e-3);
    assert!((h[4] - 83_886.07).abs() < 0.01);
    assert!((h[5] + 83_886.07).abs() < 0.01);
}

#[test]
fn the_sources_cover_where_they_are_finer_than_the_window() {
    // Palos Verdes, Malibu, Diamond Head, Dover, Bergen, Narvik, Wellington: the Terrain Tiles.
    for (lat, lon) in [
        (33.745, -118.41),
        (34.04, -118.80),
        (21.262, -157.806),
        (51.13, 1.34),
        (60.39, 5.33),
        (68.43, 17.42),
        (-41.29, 174.78),
    ] {
        assert_eq!(source_at(lat, lon), Some(Source::Terrarium), "{lat}, {lon}");
    }
    // Hakone, Miura, Izu: the GSI's.
    for (lat, lon) in [(35.23, 139.03), (35.16, 139.62), (34.75, 138.85)] {
        assert_eq!(source_at(lat, lon), Some(Source::Gsi), "{lat}, {lon}");
    }
    // Malta, Gibraltar, Orkney, Guadalcanal: none finer than the window's.
    for (lat, lon) in [
        (35.83, 14.45),
        (36.13, -5.345),
        (58.93, -3.36),
        (-9.43, 160.0),
    ] {
        assert_eq!(source_at(lat, lon), None, "{lat}, {lon}");
    }
    // Longitude wraps.
    assert_eq!(source_at(21.262, -157.806 + 360.0), Some(Source::Terrarium));
}

#[test]
fn pages_tile_the_theatre_about_the_camera() {
    let side = page_m();
    assert_eq!(side, 2016.0);
    let block = block([10.0, -3000.0], SLOTS);
    assert_eq!(block.len(), 64);
    assert_eq!(block[0], page_of([10.0, -3000.0]));
    let (grid, base) = grid([5000.0, -7000.0]);
    assert_eq!(base, [2, -4]);
    assert_eq!(grid.origin, [2.0 * side - 5000.0, -4.0 * side + 7000.0]);
    assert_eq!(grid.page_m(), side);
}

/// Every tile page `index` reads, filled from `height` at each pixel's middle (global zoom-15 pixels).
fn tiles_of(
    geo: LatLon,
    lattice: &Lattice,
    index: [i64; 2],
    height: impl Fn(f64, f64) -> f32,
) -> HashMap<TileKey, Tile> {
    page_tiles(geo, lattice, index, Source::Terrarium)
        .into_iter()
        .map(|key| {
            let (_, x, y) = key;
            let values = (0..TILE_PX * TILE_PX)
                .map(|k| {
                    let gx = (x * TILE_PX + k % TILE_PX) as f64 + 0.5;
                    let gy = (y * TILE_PX + k / TILE_PX) as f64 + 0.5;
                    height(gx, gy)
                })
                .collect();
            (key, Some(Arc::new(values)))
        })
        .collect()
}

#[test]
fn a_plane_has_no_relief_and_a_wall_between_samples_stands_out() {
    let geo = LatLon {
        lat: 33.75,
        lon: -118.40,
    };
    let lattice = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    let index = [1, -2];
    let middle = PageMap::new(geo, index).px([page_m() / 2.0; 2]);
    // A slope rising 40 m a kilometre east and 10 m south (the pixels are 4 m): bilinear between the samples, no relief.
    let plane =
        |gx: f64, gy: f64| (200.0 + (gx - middle[0]) * 0.16 + (gy - middle[1]) * 0.04) as f32;
    let page = make_page(
        geo,
        &lattice,
        index,
        Source::Terrarium,
        &tiles_of(geo, &lattice, index, plane),
    );
    assert!(page.peak < 0.05, "a plane's relief: {}", page.peak);
    // A wall 30 m high and 12 m thick across the page in the middle of a lattice cell: the land between the samples
    // the window draws flat stands up by nearly its height, and a cell away it does not.
    let corner = index[0] as f64 * page_m();
    let k = ((corner + page_m() / 2.0 - lattice.node[0]) / lattice.cell).floor();
    let wall_x = lattice.node[0] + k * lattice.cell + lattice.cell / 2.0 - corner;
    let wall = PageMap::new(geo, index).px([wall_x, page_m() / 2.0])[0];
    let walled = |gx: f64, _gy: f64| {
        if (gx - wall).abs() < 1.5 {
            130.0
        } else {
            100.0
        }
    };
    let page = make_page(
        geo,
        &lattice,
        index,
        Source::Terrarium,
        &tiles_of(geo, &lattice, index, walled),
    );
    let texel = |m: f64| (m / TEXEL_M - 0.5 + f64::from(GUTTER)).round() as u32;
    let on = page.at(texel(wall_x), PAGE_TEXELS / 2);
    let off = page.at(texel(wall_x + lattice.cell), PAGE_TEXELS / 2);
    assert!(on > 20.0, "on the wall: {on}");
    assert!(off.abs() < 0.5, "a cell off it: {off}");
}

#[test]
fn the_sea_and_missing_tiles_are_no_relief() {
    let geo = LatLon {
        lat: 21.26,
        lon: -157.8,
    };
    let lattice = Lattice {
        node: [0.0, 0.0],
        cell: 40.0,
    };
    let index = [0, 0];
    // Tiles the source does not hold: heights 0 (the sea), so no relief.
    let none: HashMap<TileKey, Tile> = page_tiles(geo, &lattice, index, Source::Terrarium)
        .into_iter()
        .map(|k| (k, None))
        .collect();
    let page = make_page(geo, &lattice, index, Source::Terrarium, &none);
    assert_eq!(page.peak, 0.0);
    // Spikes deep below the sea are cut at the floor.
    let deep = tiles_of(geo, &lattice, index, |gx, _| {
        if gx as i64 % 7 == 0 { -300.0 } else { 0.0 }
    });
    let page = make_page(geo, &lattice, index, Source::Terrarium, &deep);
    assert!(page.peak <= -FLOOR_M + 0.01, "{}", page.peak);
}

#[test]
fn the_lattice_is_the_same_whichever_sample_names_it() {
    let a = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    let b = Lattice {
        node: [13.0 + 40.0 * 7.0, -27.0 - 40.0 * 3.0],
        cell: 40.0,
    };
    assert_eq!(a.key(), b.key());
    let c = Lattice {
        node: [14.0, -27.0],
        cell: 40.0,
    };
    assert_ne!(a.key(), c.key());
}

#[test]
fn a_page_is_drawn_where_the_window_has_land() {
    // 401² samples 40 m apart from chart (-8000, -8000): land (20 m) only east of chart x 4000.
    let n = 401u32;
    let quanta = (0..n * n)
        .map(|k| {
            if (k % n) as f32 * 40.0 - 8000.0 > 4000.0 {
                20
            } else {
                -20
            }
        })
        .collect();
    let field = naval_terrain::Heightfield {
        columns: n,
        rows: n,
        cell: 40.0,
        origin_x: -8000.0,
        origin_z: -8000.0,
        step: 1.0,
        quanta,
    };
    let prepared = naval_terrain::field::PreparedField::new(&field);
    // Chart (0, 0) at theatre point (100000, 50000).
    let origin = [100_000.0, 50_000.0];
    let side = page_m();
    let page_at = |chart: [f64; 2]| page_of([chart[0] + origin[0], chart[1] + origin[1]]);
    assert!(page_has_land(&prepared, origin, page_at([6000.0, 0.0])));
    assert!(!page_has_land(&prepared, origin, page_at([-4000.0, 0.0])));
    // Past the field: none.
    assert!(!page_has_land(
        &prepared,
        origin,
        page_at([-8000.0 - 2.0 * side, 0.0])
    ));
    assert!(!page_has_land(
        &prepared,
        origin,
        page_at([9000.0 + side, 0.0])
    ));
}

#[test]
fn the_cache_keeps_to_its_bound_dropping_the_least_recently_read() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/native/relief-test/prune");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    let now = SystemTime::now();
    for k in 0..8u64 {
        let path = root
            .join(if k % 2 == 0 { "a" } else { "a/b" })
            .join(format!("{k}.png"));
        std::fs::write(&path, vec![0u8; 1000]).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(now - Duration::from_secs(3600 * (10 - k)))
            .unwrap();
    }
    // Under the bound: all kept.
    assert_eq!(prune(&root, 10_000), (8000, 0));
    // Over it: the oldest go until three quarters of it are left.
    assert_eq!(prune(&root, 4000), (3000, 5));
    for k in 0..8u64 {
        let path = root
            .join(if k % 2 == 0 { "a" } else { "a/b" })
            .join(format!("{k}.png"));
        assert_eq!(path.exists(), k >= 5, "{k}");
    }
    let _ = std::fs::remove_dir_all(&root);
}
