use super::*;

#[test]
fn a_tiles_quadkey_is_the_maps_file_name() {
    // Malibu's tile at zoom 9, and the corners.
    assert_eq!(quadkey(87, 204), "023012311");
    assert_eq!(quadkey(0, 0), "000000000");
    assert_eq!(quadkey(511, 511), "333333333");
    let px = surface::mercator_px(34.04, -118.68, PX_ZOOM);
    let key = ChunkKey::of([px[0] as i64, px[1] as i64]);
    assert_eq!(key.tile, [87, 204]);
    assert!(key.chunk.iter().all(|c| (0..CHUNKS).contains(c)));
}

/// A BigTIFF directory as the map's tiles have it, with `width` pixels a side and `bits` a pixel.
fn header(magic: u16, width: u64, bits: u64) -> Vec<u8> {
    // (tag, type, count, value): shorts 3, longs 4, long8s 16.
    let entries: [(u16, u16, u64, u64); 9] = [
        (256, 4, 1, width),
        (257, 4, 1, width),
        (258, 3, 1, bits),
        (259, 3, 1, 8),
        (273, 16, 65_536, 524_660),
        (277, 3, 1, 1),
        (278, 3, 1, 1),
        (279, 16, 65_536, 372),
        (317, 3, 1, 2),
    ];
    let mut b = Vec::new();
    b.extend(b"II");
    b.extend(magic.to_le_bytes());
    b.extend(8u16.to_le_bytes());
    b.extend(0u16.to_le_bytes());
    b.extend(16u64.to_le_bytes());
    b.extend((entries.len() as u64).to_le_bytes());
    for (tag, kind, count, value) in entries {
        b.extend(tag.to_le_bytes());
        b.extend(kind.to_le_bytes());
        b.extend(count.to_le_bytes());
        b.extend(value.to_le_bytes());
    }
    b.extend(0u64.to_le_bytes());
    b
}

#[test]
fn a_tiles_header_says_where_its_strips_lie() {
    let parsed = parse_header(&header(43, 65_536, 8)).unwrap();
    assert_eq!(
        parsed,
        Header {
            compression: 8,
            predictor: 2,
            offsets: (524_660, 65_536, 8),
            counts: (372, 65_536, 8),
        }
    );
    // A classic TIFF, another size or depth are not read.
    assert!(parse_header(&header(42, 65_536, 8)).is_err());
    assert!(parse_header(&header(43, 4096, 8)).is_err());
    assert!(parse_header(&header(43, 65_536, 16)).is_err());
    assert!(parse_header(&header(43, 65_536, 8)[..40]).is_err());
}

#[test]
fn a_row_decodes_and_the_strips_keep() {
    let n = TILE_PX as usize;
    let row: Vec<u8> = (0..n).map(|k| ((k / 7) % 40) as u8).collect();
    // The horizontal predictor: each byte the difference from the one before.
    let mut differenced = row.clone();
    for k in (1..n).rev() {
        differenced[k] = differenced[k].wrapping_sub(differenced[k - 1]);
    }
    let stored = miniz_oxide::deflate::compress_to_vec_zlib(&differenced, 6);
    let strips = Strips {
        compression: 8,
        predictor: 2,
        offsets: (0..n as u64).map(|k| 1000 + k * 10).collect(),
        counts: vec![10; n],
    };
    assert_eq!(decode_row(&strips, &stored).unwrap(), row);
    // An empty strip is bare ground.
    assert_eq!(decode_row(&strips, &[]).unwrap(), vec![0; n]);
    assert_eq!(Strips::from_bytes(&strips.to_bytes()), Some(strips));
}

/// Every chunk under page `index` about `geo`, bare, but for `tall` (m) at theatre point `at`.
fn chunks_with(geo: LatLon, index: [i64; 2], at: [f64; 2], tall: u8) -> HashMap<ChunkKey, Chunk> {
    let mut chunks: HashMap<ChunkKey, Chunk> = PageMap::new(geo, index)
        .chunks()
        .into_iter()
        .map(|k| (k, Some(Arc::new(vec![0u8; CHUNK_CELLS * CHUNK_CELLS]))))
        .collect();
    let p = to_lat_lon(geo, at);
    let g = surface::mercator_px(p.lat, p.lon, PX_ZOOM);
    let px = [g[0].floor() as i64, g[1].floor() as i64];
    let cells = chunks.get_mut(&ChunkKey::of(px)).unwrap().as_mut().unwrap();
    let inside = px.map(|p| (p.rem_euclid(TILE_PX) % CHUNK_PX / 2) as usize);
    Arc::make_mut(cells)[inside[1] * CHUNK_CELLS + inside[0]] = tall;
    chunks
}

#[test]
fn a_pages_cells_find_the_maps_pixels_where_they_stand() {
    // Malibu's hills, a page some kilometres east and south of the theatre's origin (its frame turned a little against
    // the mercator's), a 30 m tree at a known point on it.
    let geo = LatLon {
        lat: 34.05,
        lon: -118.9,
    };
    let index = [7, 3];
    let side = page_m();
    let at = [7.0 * side + 700.0, 3.0 * side + 1300.0];
    let chunks = chunks_with(geo, index, at, 30);
    let heights = page_heights(geo, index, &chunks);
    let width = PAGE_TEXELS as usize * CELLS + 2 * MARGIN;
    let cell = |x: f64, z: f64| {
        let (cx, cz) = (
            (x / CELL_M) as usize + MARGIN,
            (z / CELL_M) as usize + MARGIN,
        );
        heights[cz * width + cx]
    };
    // The tree in the cell under it or one beside it, nothing a few cells off.
    let near: u8 = (-1..=1)
        .flat_map(|dz| (-1..=1).map(move |dx| (dx, dz)))
        .map(|(dx, dz)| {
            cell(
                700.0 + f64::from(dx) * CELL_M,
                1300.0 + f64::from(dz) * CELL_M,
            )
        })
        .max()
        .unwrap();
    assert_eq!(near, 30);
    assert_eq!(cell(700.0 + 4.0 * CELL_M, 1300.0), 0);
    assert_eq!(cell(700.0, 1300.0 - 4.0 * CELL_M), 0);
    assert!(heights.iter().filter(|h| **h == 30).count() <= 4);
    // Its crown on the page, where it stands.
    let page = make_page(geo, index, &chunks);
    let crowns = page.crowns(TEXEL_M);
    assert_eq!(crowns.len(), 1, "{crowns:?}");
    assert_eq!(crowns[0].height, 30);
    let off = (crowns[0].at[0] - 700.0).hypot(crowns[0].at[1] - 1300.0);
    assert!(
        off < 3.5,
        "the crown {off} m from the tree: {:?}",
        crowns[0]
    );

    // A tile the map does not hold: nothing known.
    let missing: HashMap<ChunkKey, Chunk> = chunks.keys().map(|k| (*k, None)).collect();
    assert!(
        page_heights(geo, index, &missing)
            .iter()
            .all(|h| *h == UNKNOWN)
    );
    assert!(!make_page(geo, index, &missing).known());
}

#[test]
fn the_block_is_nearest_first_and_the_grid_counts_from_the_layers_origin() {
    let side = page_m();
    let pages = block([3.2 * side, -1.7 * side], SLOTS);
    assert_eq!(pages.len(), (SLOTS * SLOTS) as usize);
    assert_eq!(pages[0], [3, -2]);
    let (grid, base) = grid([1000.0, -5000.0]);
    assert_eq!(base, [0, -3]);
    assert_eq!(grid.origin, [-1000.0, -3.0 * side + 5000.0]);
    assert_eq!(grid.texel_m, TEXEL_M);
}

#[test]
fn a_page_across_the_antimeridian_reads_the_chunks_about_it() {
    // A theatre a few hundred metres west of the antimeridian: its first page reaches past it, a tree east of it.
    let geo = LatLon {
        lat: -1.0,
        lon: 179.996,
    };
    let index = [0, 0];
    let keys = PageMap::new(geo, index).chunks();
    assert!((1..=9).contains(&keys.len()), "{} chunks", keys.len());
    assert!(
        keys.iter()
            .all(|k| (0..WORLD_PX / TILE_PX).contains(&k.tile[0]))
    );
    let at = [1500.0, 900.0];
    assert!(to_lat_lon(geo, at).lon < 0.0);
    let chunks = chunks_with(geo, index, at, 25);
    let heights = page_heights(geo, index, &chunks);
    assert!(heights.contains(&25));
    assert_eq!(make_page(geo, index, &chunks).crowns(TEXEL_M).len(), 1);
}
