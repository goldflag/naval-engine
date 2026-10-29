//! The surface's tile math, the window's texels and the GeoTIFF reader on a TIFF made here (a Reykjavik window's maps
//! from the real sources are the client's test: its window is the client's).
use super::*;
use naval_earth::geo::to_local;

const REYKJAVIK: LatLon = LatLon {
    lat: 64.1466,
    lon: -21.9426,
};

#[test]
fn imagery_tiles_and_zooms_follow_the_mercator_grid() {
    // Reykjavik's tiles as the server names them (fetched by hand: z/y/x).
    assert_eq!(imagery_tile(64.13, -21.9, 13), [3597, 2177]);
    assert_eq!(imagery_tile(64.13, -21.9, 10), [449, 272]);
    // The tile's corner is north-west of the point, the next tile's south-east of it.
    let nw = imagery_corner(3597, 2177, 13);
    let se = imagery_corner(3598, 2178, 13);
    assert!(nw.lat > 64.13 && se.lat < 64.13 && nw.lon < -21.9 && se.lon > -21.9);
    // Corners and tiles agree both ways, south and west too.
    for (lat, lon, z) in [(-9.43, 160.03, 13), (35.9, 14.5, 11), (-33.9, -70.6, 9)] {
        let [x, y] = imagery_tile(lat, lon, z);
        let c = imagery_corner(x, y, z);
        let d = imagery_corner(x + 1, y + 1, z);
        assert!(c.lat >= lat && lat > d.lat && c.lon <= lon && lon < d.lon);
    }
    // A pixel no coarser than a texel: a sea window's 23 m texel takes zoom 12 at 64 N (17 m pixels) and 13 at the
    // equator (19 m); an air window's 117 m, zoom 10 (67 m) at 64 N; never past the source's own 10 m.
    let sea = 96_000.0 / f64::from(SEA_TEXELS);
    let air = 239_680.0 / f64::from(AIR_TEXELS);
    assert_eq!(imagery_zoom(sea, 64.13), 12);
    assert_eq!(imagery_zoom(sea, -9.4), 13);
    assert_eq!(imagery_zoom(air, 64.13), 10);
    assert_eq!(imagery_zoom(air, -9.4), 11);
    assert_eq!(imagery_zoom(1.0, 0.0), MAX_ZOOM);
    // WorldCover's overview: 18.5 m pixels for the sea window, 74 m for the air window, the full 9 m for finer.
    assert_eq!(cover_level(sea), 1);
    assert_eq!(cover_level(air), 3);
    assert_eq!(cover_level(5.0), 0);
}

#[test]
fn worldcover_squares_are_named_by_their_south_west_corner() {
    assert_eq!(cover_square_of(64.13, -21.9), "N63W024");
    assert_eq!(cover_square_of(-9.4, 160.0), "S12E159");
    assert_eq!(cover_square_of(-0.5, -80.2), "S03W081");
    assert_eq!(cover_square_of(0.5, 6.1), "N00E006");
    assert_eq!(cover_square_of(51.1, 1.3), "N51E000");
    assert_eq!(cover_square_of(51.1, -1.3), "N51W003");
    assert_eq!(cover_square_of(-42.9, 147.3), "S45E147");
    assert_eq!(cover_square_of(28.2, -177.4), "N27W180");
    // The global pixel grid lands in the same square, at every level.
    for (lat, lon) in [(64.13, -21.9), (-9.4, 160.0), (-0.5, -80.2), (35.9, 14.5)] {
        for level in 0..COVER_LEVELS {
            let [x, y] = cover_px(lat, lon, level);
            let side = cover_side(level) as f64;
            let name = cover_square_at((x / side).floor() as i64, (y / side).floor() as i64);
            assert_eq!(name, cover_square_of(lat, lon), "{lat} {lon} level {level}");
        }
    }
    assert_eq!(parse_square("S12E159"), (-12, 159));
    assert_eq!(parse_square("N63W024"), (63, -24));
}

#[test]
fn a_windows_texels_map_to_the_theatre_and_back() {
    let geo = LatLon {
        lat: 64.6,
        lon: -28.4,
    };
    let centre = to_local(geo, REYKJAVIK);
    let key = SurfaceKey {
        geo,
        centre,
        origin: [-48_000.0, -48_000.0],
        extent_m: 96_000.0,
        size: SEA_TEXELS,
    };
    let t = key.texel_m();
    assert!((t - 23.4375).abs() < 1e-9);
    // Texel (0, 0)'s centre is half a texel in from the corner; row 0 at the smallest z (the north edge).
    assert_eq!(
        key.chart_of_texel(0.0, 0.0),
        [-48_000.0 + t / 2.0, -48_000.0 + t / 2.0]
    );
    for (i, j) in [(0, 0), (4095, 17), (2048, 2048), (100, 4095)] {
        let c = key.chart_of_texel(f64::from(i), f64::from(j));
        assert_eq!(key.texel_of_chart(c), Some([i, j]));
    }
    assert_eq!(key.texel_of_chart([48_000.5, 0.0]), None);
    assert_eq!(key.texel_of_chart([0.0, -48_000.5]), None);
    // Chart point c is theatre point centre + c: the window's middle is Reykjavik, east is east and south is south.
    let middle = key.lat_lon_of_chart([0.0, 0.0]);
    assert!((middle.lat - REYKJAVIK.lat).abs() < 1e-9 && (middle.lon - REYKJAVIK.lon).abs() < 1e-9);
    let east = key.lat_lon_of_chart([10_000.0, 0.0]);
    let south = key.lat_lon_of_chart([0.0, 10_000.0]);
    assert!(east.lon > middle.lon && south.lat < middle.lat);
    assert!((south.lat - (middle.lat - 10_000.0 / DEGREE_M)).abs() < 0.01);
    let c = [-31_000.0, 12_500.0];
    let theatre = [centre[0] + c[0], centre[1] + c[1]];
    assert_eq!(key.lat_lon_of_chart(c), to_lat_lon(geo, theatre));
}

#[test]
fn a_plan_fetches_only_about_the_land() {
    // A 96 km window with one island 4 km across in its north-west quarter, sea elsewhere.
    let (n, cell) = (241usize, 400.0);
    let quanta: Vec<i16> = (0..n * n)
        .map(|k| {
            let (i, j) = ((k % n) as f64, (k / n) as f64);
            if (i - 60.0).hypot(j - 60.0) < 5.0 {
                10
            } else {
                -50
            }
        })
        .collect();
    let heights = Heights {
        columns: n,
        rows: n,
        cell,
        origin: [-48_000.0, -48_000.0],
        quanta: &quanta,
    };
    let geo = LatLon {
        lat: 64.6,
        lon: -28.4,
    };
    let key = SurfaceKey::of(geo, to_local(geo, REYKJAVIK), &heights, SEA_TEXELS);
    assert_eq!(key.extent_m, 96_000.0);
    let plan = Plan::new(key, &heights);
    assert!(plan.has_land());
    // The island and the blocks about it: a few percent of the window.
    assert!(plan.wanted_share() < 0.03, "{}", plan.wanted_share());
    assert_eq!(plan.zoom, 12);
    assert_eq!(plan.level, 1);
    // Its imagery: a handful of zoom-12 tiles (each about 4.3 km at 64 N), all north-west of the window's middle.
    assert!(
        (1..=12).contains(&plan.imagery.len()),
        "{}",
        plan.imagery.len()
    );
    let [mx, my] = imagery_tile(REYKJAVIK.lat, REYKJAVIK.lon, 12);
    assert!(plan.imagery.iter().all(|&[x, y]| x < mx && y < my));
    assert!(plan.cover.len() <= 4);
    assert_eq!(plan.squares().into_iter().collect::<Vec<_>>(), ["N63W024"]);
    // Open sea: nothing.
    let sea = vec![-50i16; n * n];
    let open = Heights {
        quanta: &sea,
        ..heights
    };
    assert!(!Plan::new(key, &open).has_land());
}

#[test]
fn lookup_draws_nothing_with_streaming_off() {
    let quanta = vec![10i16; 9];
    let heights = Heights {
        columns: 3,
        rows: 3,
        cell: 100.0,
        origin: [-100.0, -100.0],
        quanta: &quanta,
    };
    let key = SurfaceKey::of(REYKJAVIK, [0.0, 0.0], &heights, 64);
    // The library's tests never switch it on.
    assert!(!enabled());
    assert!(matches!(lookup(key, &heights), Surface::None));
    assert!(!ready(&key));
    assert_eq!(status(&key), Status::Off);
}

/// A little-endian GeoTIFF of one byte a pixel, tiled and DEFLATE-compressed, with an overview per extra level: what a
/// WorldCover file is, small.
fn synthetic_tiff(
    levels: &[(u32, u32, Vec<u8>)],
    tile: u32,
    corner: [f64; 2],
    scale: [f64; 2],
) -> Vec<u8> {
    let mut out = b"II\x2a\x00\0\0\0\0".to_vec();
    let push_u32 = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    // Each level's tiles, stored.
    let mut tables = Vec::new();
    for (w, h, pixels) in levels {
        let (across, down) = (w.div_ceil(tile), h.div_ceil(tile));
        let (mut offsets, mut counts) = (Vec::new(), Vec::new());
        for ty in 0..down {
            for tx in 0..across {
                let mut raw = vec![0u8; (tile * tile) as usize];
                for y in 0..tile {
                    for x in 0..tile {
                        let (px, py) = (tx * tile + x, ty * tile + y);
                        if px < *w && py < *h {
                            raw[(y * tile + x) as usize] = pixels[(py * w + px) as usize];
                        }
                    }
                }
                let stored = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
                offsets.push(out.len() as u32);
                counts.push(stored.len() as u32);
                out.extend_from_slice(&stored);
            }
        }
        tables.push((offsets, counts));
    }
    // The IFDs, each with its arrays just before it.
    let mut link = 4usize;
    for (k, ((w, h, _), (offsets, counts))) in levels.iter().zip(&tables).enumerate() {
        let array = |out: &mut Vec<u8>, values: &[u32]| {
            let at = out.len() as u32;
            for &v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
            at
        };
        let doubles = |out: &mut Vec<u8>, values: &[f64]| {
            let at = out.len() as u32;
            for &v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
            at
        };
        let (offsets_at, counts_at) = if offsets.len() > 1 {
            (array(&mut out, offsets), array(&mut out, counts))
        } else {
            (offsets[0], counts[0])
        };
        let mut entries: Vec<(u16, u16, u32, u32)> = vec![
            (256, 3, 1, *w),
            (257, 3, 1, *h),
            (258, 3, 1, 8),
            (259, 3, 1, 8),
            (262, 3, 1, 1),
            (277, 3, 1, 1),
            (317, 3, 1, 1),
            (322, 3, 1, tile),
            (323, 3, 1, tile),
            (324, 4, offsets.len() as u32, offsets_at),
            (325, 4, counts.len() as u32, counts_at),
        ];
        if k == 0 {
            let s = doubles(&mut out, &[scale[0], scale[1], 0.0]);
            let t = doubles(&mut out, &[0.0, 0.0, 0.0, corner[0], corner[1], 0.0]);
            entries.push((33550, 12, 3, s));
            entries.push((33922, 12, 6, t));
        } else {
            entries.push((254, 4, 1, 1));
        }
        entries.sort();
        if out.len() % 2 == 1 {
            out.push(0);
        }
        let ifd = out.len() as u32;
        out[link..link + 4].copy_from_slice(&ifd.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&kind.to_le_bytes());
            push_u32(&mut out, count);
            if kind == 3 && count == 1 {
                out.extend_from_slice(&(value as u16).to_le_bytes());
                out.extend_from_slice(&[0, 0]);
            } else {
                push_u32(&mut out, value);
            }
        }
        link = out.len();
        push_u32(&mut out, 0);
    }
    out
}

#[test]
fn the_geotiff_reader_finds_the_overviews_and_their_tiles() {
    // 40×24 classes over a 0.4° by 0.24° square at 63.24 N 22 W, with a 20×12 overview; tiles of 16.
    let (w, h) = (40u32, 24u32);
    let full: Vec<u8> = (0..w * h)
        .map(|k| [cover::TREE, cover::GRASS, cover::WATER, cover::BUILT][((k % w) / 10) as usize])
        .collect();
    let half: Vec<u8> = (0..(w / 2) * (h / 2))
        .map(|k| full[((k / (w / 2)) * 2 * w + (k % (w / 2)) * 2) as usize])
        .collect();
    let bytes = synthetic_tiff(
        &[(w, h, full.clone()), (w / 2, h / 2, half.clone())],
        16,
        [-22.0, 63.24],
        [0.01, 0.01],
    );
    let cog = parse_cog(&bytes).expect("the synthetic header");
    assert_eq!(cog.levels.len(), 2);
    assert_eq!(cog.corner, [-22.0, 63.24]);
    assert_eq!(cog.scale, [0.01, 0.01]);
    let (l0, l1) = (&cog.levels[0], &cog.levels[1]);
    assert_eq!(
        (l0.width, l0.height, l0.tile_width, l0.tile_height),
        (40, 24, 16, 16)
    );
    assert_eq!((l0.compression, l0.predictor), (8, 1));
    assert_eq!(l0.tiles_across(), 3);
    assert_eq!(l0.offsets.len(), 6);
    assert_eq!((l1.width, l1.height, l1.offsets.len()), (20, 12, 2));
    // Each tile decodes to its pixels: the full raster's tile (2, 1) holds pixels from (32, 16).
    let index = 3 + 2;
    let at = l0.offsets[index] as usize;
    let tile = decode_cog_tile(l0, &bytes[at..at + l0.counts[index] as usize]).unwrap();
    assert_eq!(tile.len(), 256);
    assert_eq!(tile[0], full[(16 * w + 32) as usize]);
    assert_eq!(tile[0], cover::BUILT);
    assert_eq!(tile[(7 * 16 + 7) as usize], full[(23 * w + 39) as usize]);
    // Past the raster's edge the tile is padding.
    assert_eq!(tile[(7 * 16 + 9) as usize], 0);
    // The overview's first tile.
    let at = l1.offsets[0] as usize;
    let tile = decode_cog_tile(l1, &bytes[at..at + l1.counts[0] as usize]).unwrap();
    assert_eq!(tile[3 * 16 + 6], half[(3 * 20 + 6) as usize]);
    // An empty tile is all nodata.
    assert_eq!(decode_cog_tile(l1, &[]).unwrap(), vec![0; 256]);
    // A header cut short asks for the bytes it needs; garbage is refused.
    let ifd_end = bytes.len() as u64;
    match parse_cog(&bytes[..bytes.len() - 10]) {
        Err(CogError::Short(need)) => assert!(need > ifd_end - 10 && need <= ifd_end, "{need}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(parse_cog(b"GIF89a.."), Err(CogError::Invalid(_))));
}

#[test]
fn a_worldcover_header_fits_the_reader_only_on_its_square() {
    let level = |w: u32| CogLevel {
        width: w,
        height: w,
        tile_width: 1024,
        tile_height: 1024,
        compression: 8,
        predictor: 1,
        offsets: Vec::new(),
        counts: Vec::new(),
    };
    let cog = Cog {
        levels: (0..7).map(|k| level(36_000 >> k)).collect(),
        corner: [-24.0, 66.0],
        scale: [3.0 / 36_000.0; 2],
    };
    assert!(header_fits(&cog, "N63W024", 1));
    assert!(header_fits(&cog, "N63W024", 3));
    assert!(!header_fits(&cog, "N60W024", 1));
    assert!(!header_fits(&cog, "N63W024", 7));
    // Global tile (x, y) at level 1 (18 tiles a square): square and tile within.
    let [x, y] = cover_px(64.13, -21.9, 1);
    let g = |p: f64| {
        let p = p.floor() as i64;
        p.div_euclid(18_000) * 18 + p.rem_euclid(18_000) / 1024
    };
    let (sq, tile) = cover_tile_place(g(x), g(y), 1);
    assert_eq!(cover_square_at(sq[0], sq[1]), "N63W024");
    // 64.13 N is 1.87° below the square's top (66 N), 11220 pixels at 18.5 m: tile row 10; 21.9 W is 2.1° east of
    // 24 W: tile column 12.
    assert_eq!(tile, [12, 10]);
}

#[test]
fn naips_colour_is_matched_to_sentinel_2s_at_the_broad_scale() {
    // A 64² tile: Sentinel-2 a flat (0.10, 0.08, 0.05) in linear light, NAIP twice as bright and bluer with a
    // checkerboard of detail; the left half's NAIP missing.
    let size = 64;
    let s2: Vec<Option<[f32; 3]>> = vec![Some([0.10, 0.08, 0.05]); size * size];
    let naip: Vec<Option<[f32; 3]>> = (0..size * size)
        .map(|k| {
            let (i, j) = (k % size, k / size);
            let detail = if (i + j) % 2 == 0 { 1.2 } else { 0.8 };
            (i >= size / 2).then_some([0.2 * detail, 0.16 * detail, 0.15 * detail])
        })
        .collect();
    let (cells, ratios) = match_ratios(size, &s2, &naip);
    assert_eq!(cells, 2);
    // Every square's ratio is Sentinel-2's mean over NAIP's: the right half's measured, the left half's filled from it.
    for r in &ratios {
        assert!(
            (r[0] - 0.5).abs() < 0.01 && (r[1] - 0.5).abs() < 0.01,
            "{r:?}"
        );
        assert!((r[2] - 1.0 / 3.0).abs() < 0.01, "{r:?}");
    }
    // A ratio past four is held at four; none measured anywhere is none (1).
    let dark: Vec<Option<[f32; 3]>> = vec![Some([0.001, 0.001, 0.001]); size * size];
    let (_, held) = match_ratios(size, &s2, &dark);
    assert!(held.iter().all(|r| r.iter().all(|&v| v == 4.0)), "{held:?}");
    let (_, none) = match_ratios(size, &s2, &vec![None; size * size]);
    assert!(none.iter().all(|r| *r == [1.0; 3]));
    // Round trips of the colour conversions.
    for b in [0u8, 1, 10, 54, 128, 200, 255] {
        assert_eq!(linear_to_srgb_byte(srgb_to_linear(f32::from(b) / 255.0)), b);
    }
}
