//! Where a square's bake spends its time (`--ignored`; needs a Terrarium cache in `.build/region-bake/rust-dem`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use naval_region_bake::square::{self, SquareKey};
use naval_region_bake::terrarium;

#[test]
#[ignore]
fn a_square_from_terrarium() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.build/region-bake/rust-dem/terrarium");
    let key = SquareKey { lat: 36, lon: -122 };
    let (zoom, list) = square::terrarium_tiles(key);
    let t = Instant::now();
    let mut decoded = HashMap::new();
    for (x, y) in list {
        let Ok(png) = std::fs::read(dir.join(terrarium::tile_path(zoom, x, y))) else {
            eprintln!("skipped: no cache");
            return;
        };
        decoded.insert((x, y), Arc::new(terrarium::decode(&png).unwrap()));
    }
    println!("decode {} tiles: {:?}", decoded.len(), t.elapsed());
    let t = Instant::now();
    let sq = square::bake_terrarium(key, &|k| decoded.get(&k).cloned(), false).unwrap();
    println!("bake: {:?}", t.elapsed());
    for level in [1, 3, 6, 9] {
        let t = Instant::now();
        let bytes = sq.encode_at(level);
        println!(
            "encode at {level}: {} bytes, {:?}",
            bytes.len(),
            t.elapsed()
        );
    }
    let bytes = sq.encode();
    // An NTF1 tile of the same ground at levels 6 and 9.
    let q: Vec<i16> = sq.heights()[..1000 * 1000]
        .iter()
        .map(|&h| naval_region_bake::ntf::quantise(h as f64))
        .collect();
    for level in [6, 9] {
        let t = Instant::now();
        let z = naval_region_bake::ntf::zlib_at(
            &naval_region_bake::ntf::encode_residuals(&q, 1000, 1000),
            flate2::Compression::new(level),
        );
        println!(
            "an NTF1 tile at {level}: {} bytes, {:?}",
            z.len(),
            t.elapsed()
        );
    }
    let t = Instant::now();
    let back = square::Square::decode(&bytes).unwrap();
    println!("decode: {:?}", t.elapsed());
    let h = back.heights();
    let (c, r) = (back.grid.columns, back.grid.rows);
    let t = Instant::now();
    let (_, n) = naval_region_bake::despike::despike(h, c, r, 2);
    println!("despike again: {:?} ({n} spikes)", t.elapsed());
}
