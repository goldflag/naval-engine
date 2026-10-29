//! Adjacent squares' final windows share their edges: the same lattice points, and the same quanta read back from each
//! square's own file, within a band, across the surveyed layer's band changes, across the DEM's change at 60° N and
//! 56° S, across ±180° and below the pole limit. And a reader decodes only the blocks it reads.
use naval_land_format::block::{Block, Window};
use naval_land_format::lattice::{self, BLOCK_STEPS, BLOCKS, Point, WINDOW};
use naval_land_format::{FinalSquare, SquareKey};

/// A height that is a function of the place alone and differs from point to point: its lattice point mixed.
fn place_quantum(p: Point) -> i16 {
    let mut h = (p.i() as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (p.j() as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
    h ^= h >> 29;
    let q = (h % 4001) as i16 - 2000;
    if q == 0 { 1 } else { q }
}

fn file(key: SquareKey) -> FinalSquare {
    FinalSquare::read(FinalSquare::encode(
        key,
        &Window::of(key, place_quantum),
        Default::default(),
    ))
    .unwrap()
}

#[derive(Clone, Copy)]
enum Side {
    North,
    South,
    West,
    East,
}

/// The quanta along one edge of a square's window as little-endian bytes, read from its file one block at a time
/// (each block's last point is the next block's first, so it is taken once).
fn edge(square: &FinalSquare, side: Side) -> Vec<u8> {
    let last = BLOCKS - 1;
    let mut out = vec![];
    for b in 0..BLOCKS {
        let (br, bc) = match side {
            Side::North => (0, b),
            Side::South => (last, b),
            Side::West => (b, 0),
            Side::East => (b, last),
        };
        let block: Block = square.block(br, bc).unwrap();
        let n = if b < last {
            BLOCK_STEPS
        } else {
            BLOCK_STEPS + 1
        };
        for k in 0..n {
            let q = match side {
                Side::North => block.at(0, k),
                Side::South => block.at(BLOCK_STEPS, k),
                Side::West => block.at(k, 0),
                Side::East => block.at(k, BLOCK_STEPS),
            };
            out.extend_from_slice(&q.to_le_bytes());
        }
    }
    out
}

#[test]
fn neighbours_files_agree_on_their_shared_edges() {
    let k = |lat, lon| SquareKey { lat, lon };
    // East neighbours: within a band, across ±180° (twice) and in the far north.
    let east = [k(10, 20), k(10, 179), k(-57, 179), k(61, -1)];
    // North neighbours: across the surveyed bands' changes at 30° and 45°, the DEM's at 60° N and 56° S (and ±180° on
    // the way), and the last row below the pole limit.
    let north = [k(29, 179), k(44, -180), k(59, 3), k(-57, 179), k(87, 0)];
    let mut files = std::collections::BTreeMap::new();
    let mut read = |key: SquareKey| files.entry(key).or_insert_with(|| file(key)).clone();
    for a in east {
        let b = a.east();
        let (fa, fb) = (read(a), read(b));
        assert_eq!(edge(&fa, Side::East), edge(&fb, Side::West), "{a} and {b}");
        assert_eq!(edge(&fa, Side::East).len(), 2 * WINDOW);
    }
    for a in north {
        let b = a.north().unwrap();
        let (fa, fb) = (read(a), read(b));
        assert_eq!(
            edge(&fa, Side::North),
            edge(&fb, Side::South),
            "{a} and {b}"
        );
    }
}

#[test]
fn a_reader_decodes_only_the_block_a_place_falls_in() {
    let key = SquareKey { lat: 59, lon: -180 };
    let window = Window::of(key, place_quantum);
    let bytes = FinalSquare::encode(key, &window, Default::default());
    let square = FinalSquare::read(bytes.clone()).unwrap();
    // Ruin every block but the one a place falls in: the place still reads, from that block alone.
    let (lat, lon) = (59.6, -179.3);
    let cell = lattice::cell(lattice::position(key, lat, lon));
    assert_eq!(cell.block, [1, 2]);
    let mut ruined = bytes.clone();
    let start = bytes.len() - square.blocks().iter().map(|b| b.length).sum::<usize>();
    for (k, info) in square.blocks().iter().enumerate() {
        if k != cell.block[0] * BLOCKS + cell.block[1] {
            ruined[start + info.offset..start + info.offset + info.length].fill(0);
        }
    }
    let ruined = FinalSquare::read(ruined).unwrap();
    let block = ruined.block(cell.block[0], cell.block[1]).unwrap();
    assert_eq!(block, window.block(1, 2));
    let h = block.bilinear_m(cell.at);
    let [r, c] = [cell.block[0] * BLOCK_STEPS, cell.block[1] * BLOCK_STEPS];
    let (r0, c0) = (
        r + cell.at[0].floor() as usize,
        c + cell.at[1].floor() as usize,
    );
    let corners = [
        window.at(r0, c0),
        window.at(r0, c0 + 1),
        window.at(r0 + 1, c0),
        window.at(r0 + 1, c0 + 1),
    ];
    let (lo, hi) = (
        *corners.iter().min().unwrap() as f64 * 0.25,
        *corners.iter().max().unwrap() as f64 * 0.25,
    );
    assert!((lo..=hi).contains(&h), "{h} outside [{lo}, {hi}]");
    // The others are refused, each by its own digest.
    assert!(ruined.block(0, 0).is_err());
    assert!(ruined.window().is_err());
}
