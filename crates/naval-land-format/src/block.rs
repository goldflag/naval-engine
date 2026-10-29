//! The final layer's heights: a square's [`Window`] of the global lattice, kept as sixteen 0.25° [`Block`]s.
//!
//! Each block is [`BLOCK_SIDE`]² i16 quanta of [`QUANTUM_M`] (row-major, its own north-west point first), sharing its
//! edge points with its neighbours, so the lattice cell a place falls in lies whole in one block. On disk a block is the
//! planar prediction (each quantum less `left + up − up_left`, zeros outside the block, wrapping in i16), its
//! little-endian bytes split into two planes (every low byte, then every high byte), deflated in a zlib stream at
//! [`ZLIB_LEVEL`]; its bytes' SHA-256 and its least and greatest quanta are in the layer's block table ([`BlockInfo`]).
//! Blocks follow one another in the layer's payload in row-major order, north-west first.
use crate::lattice::{self, BLOCK_SIDE, BLOCK_STEPS, BLOCKS, Point, QUANTUM_M, WINDOW};
use crate::{Digest, Error, SquareKey, corrupt};
use serde::{Deserialize, Serialize};

/// The final layer's encoding, as its layer entry names it.
pub const ENCODING: &str = "i16-blocks-planar-zlib";
/// The deflate level blocks are written at.
pub const ZLIB_LEVEL: u8 = 6;

/// What a layer's block table says of one block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockInfo {
    /// Its bytes' range in the layer's payload.
    pub offset: usize,
    pub length: usize,
    /// Its bytes' SHA-256.
    pub sha256: Digest,
    /// Its least and greatest quanta: all sea when `max < 0`, all land when `min > 0` (a final height is never 0), mixed
    /// otherwise. The planner's block summary (C4), read without decoding the block.
    pub min: i16,
    pub max: i16,
}

/// A square's final window: [`WINDOW`]² quanta, row-major, row 0 the north edge, column 0 the west edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub quanta: Vec<i16>,
}

impl Window {
    /// The window of square `key` with each lattice point's quantum from `quantum`: a function of the point alone, so
    /// neighbouring squares hold the same quanta on their shared edge.
    pub fn of(key: SquareKey, mut quantum: impl FnMut(Point) -> i16) -> Self {
        let mut quanta = Vec::with_capacity(WINDOW * WINDOW);
        for r in 0..WINDOW {
            for c in 0..WINDOW {
                quanta.push(quantum(lattice::window_point(key, r, c)));
            }
        }
        Self { quanta }
    }

    #[inline]
    pub fn at(&self, r: usize, c: usize) -> i16 {
        self.quanta[r * WINDOW + c]
    }

    /// Block (`br`, `bc`)'s quanta, copied out.
    pub fn block(&self, br: usize, bc: usize) -> Block {
        let mut quanta = Vec::with_capacity(BLOCK_SIDE * BLOCK_SIDE);
        for r in 0..BLOCK_SIDE {
            let at = (br * BLOCK_STEPS + r) * WINDOW + bc * BLOCK_STEPS;
            quanta.extend_from_slice(&self.quanta[at..at + BLOCK_SIDE]);
        }
        Block { quanta }
    }

    /// Whether any point is land.
    pub fn land(&self) -> bool {
        self.quanta.iter().any(|&q| q > 0)
    }
}

/// One block's quanta: [`BLOCK_SIDE`]², row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub quanta: Vec<i16>,
}

impl Block {
    #[inline]
    pub fn at(&self, r: usize, c: usize) -> i16 {
        self.quanta[r * BLOCK_SIDE + c]
    }

    /// The height in metres at (`r`, `c`).
    #[inline]
    pub fn height_m(&self, r: usize, c: usize) -> f64 {
        f64::from(self.at(r, c)) * QUANTUM_M
    }

    /// The bilinear height in metres at a fractional in-block position ([`lattice::cell`]'s `at`, each in [0, 400]).
    pub fn bilinear_m(&self, at: [f64; 2]) -> f64 {
        let last = (BLOCK_SIDE - 2) as f64;
        let (r0, c0) = (
            at[0].floor().clamp(0.0, last),
            at[1].floor().clamp(0.0, last),
        );
        let (wr, wc) = (at[0] - r0, at[1] - c0);
        let (r0, c0) = (r0 as usize, c0 as usize);
        let top = self.height_m(r0, c0) * (1.0 - wc) + self.height_m(r0, c0 + 1) * wc;
        let bottom = self.height_m(r0 + 1, c0) * (1.0 - wc) + self.height_m(r0 + 1, c0 + 1) * wc;
        top * (1.0 - wr) + bottom * wr
    }
}

/// The window's sixteen blocks encoded: their table and the layer's payload (the blocks one after another).
pub fn encode(window: &Window) -> (Vec<BlockInfo>, Vec<u8>) {
    assert_eq!(
        window.quanta.len(),
        WINDOW * WINDOW,
        "a final window is 1,601 × 1,601 quanta"
    );
    let mut infos = Vec::with_capacity(BLOCKS * BLOCKS);
    let mut payload = vec![];
    for br in 0..BLOCKS {
        for bc in 0..BLOCKS {
            let block = window.block(br, bc);
            let bytes = encode_block(&block);
            infos.push(BlockInfo {
                offset: payload.len(),
                length: bytes.len(),
                sha256: Digest::of(&bytes),
                min: *block.quanta.iter().min().unwrap(),
                max: *block.quanta.iter().max().unwrap(),
            });
            payload.extend_from_slice(&bytes);
        }
    }
    (infos, payload)
}

/// One block's bytes: the planar prediction in two byte planes, deflated.
pub fn encode_block(block: &Block) -> Vec<u8> {
    let n = BLOCK_SIDE * BLOCK_SIDE;
    assert_eq!(block.quanta.len(), n, "a block is 401 × 401 quanta");
    let q = &block.quanta;
    let mut planes = vec![0u8; 2 * n];
    for r in 0..BLOCK_SIDE {
        for c in 0..BLOCK_SIDE {
            let at = r * BLOCK_SIDE + c;
            let left = if c > 0 { q[at - 1] } else { 0 };
            let up = if r > 0 { q[at - BLOCK_SIDE] } else { 0 };
            let up_left = if r > 0 && c > 0 {
                q[at - BLOCK_SIDE - 1]
            } else {
                0
            };
            let [lo, hi] = q[at]
                .wrapping_sub(left.wrapping_add(up).wrapping_sub(up_left))
                .to_le_bytes();
            planes[at] = lo;
            planes[n + at] = hi;
        }
    }
    miniz_oxide::deflate::compress_to_vec_zlib(&planes, ZLIB_LEVEL)
}

/// Block `info` of a layer's `payload`, alone: its range read, its SHA-256 checked, inflated and its quanta rebuilt,
/// its least and greatest quanta checked against the table.
pub fn decode_block(payload: &[u8], info: &BlockInfo) -> Result<Block, Error> {
    let what = "a final block";
    let bytes = info
        .offset
        .checked_add(info.length)
        .and_then(|end| payload.get(info.offset..end))
        .ok_or_else(|| corrupt(what, "it runs past its layer"))?;
    if Digest::of(bytes) != info.sha256 {
        return Err(corrupt(what, "its bytes do not match their SHA-256"));
    }
    let n = BLOCK_SIDE * BLOCK_SIDE;
    let planes =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(bytes, 2 * n).map_err(|e| {
            corrupt(
                what,
                format!("its stream does not inflate ({:?})", e.status),
            )
        })?;
    if planes.len() != 2 * n {
        return Err(corrupt(what, "it does not hold one quantum a point"));
    }
    let mut q = vec![0i16; n];
    for r in 0..BLOCK_SIDE {
        for c in 0..BLOCK_SIDE {
            let at = r * BLOCK_SIDE + c;
            let left = if c > 0 { q[at - 1] } else { 0 };
            let up = if r > 0 { q[at - BLOCK_SIDE] } else { 0 };
            let up_left = if r > 0 && c > 0 {
                q[at - BLOCK_SIDE - 1]
            } else {
                0
            };
            q[at] = i16::from_le_bytes([planes[at], planes[n + at]])
                .wrapping_add(left.wrapping_add(up).wrapping_sub(up_left));
        }
    }
    let (min, max) = (*q.iter().min().unwrap(), *q.iter().max().unwrap());
    if (min, max) != (info.min, info.max) {
        return Err(corrupt(
            what,
            "its quanta do not span the range its table gives",
        ));
    }
    Ok(Block { quanta: q })
}

/// Checks a block table against its layer's payload length: sixteen blocks, one after another from 0, filling it.
pub fn check_table(blocks: &[BlockInfo], length: usize) -> Result<(), Error> {
    let what = "a final layer";
    if blocks.len() != BLOCKS * BLOCKS {
        return Err(corrupt(
            what,
            format!("it has {} blocks, not 16", blocks.len()),
        ));
    }
    let mut next = 0;
    for b in blocks {
        if b.offset != next || b.min > b.max {
            return Err(corrupt(
                what,
                "its block table is not its blocks one after another",
            ));
        }
        next = b
            .offset
            .checked_add(b.length)
            .ok_or_else(|| corrupt(what, "a block's length overflows"))?;
    }
    if next != length {
        return Err(corrupt(what, "its blocks do not fill its payload"));
    }
    Ok(())
}

/// The whole window from a layer's payload and its block table.
pub fn decode_window(payload: &[u8], blocks: &[BlockInfo]) -> Result<Window, Error> {
    check_table(blocks, payload.len())?;
    let mut quanta = vec![0i16; WINDOW * WINDOW];
    for br in 0..BLOCKS {
        for bc in 0..BLOCKS {
            let block = decode_block(payload, &blocks[br * BLOCKS + bc])?;
            for r in 0..BLOCK_SIDE {
                let at = (br * BLOCK_STEPS + r) * WINDOW + bc * BLOCK_STEPS;
                let row = &block.quanta[r * BLOCK_SIDE..(r + 1) * BLOCK_SIDE];
                let into = &mut quanta[at..at + BLOCK_SIDE];
                // The points this row shares with blocks already read (the whole row on a block's north edge below
                // the first block row, else its first point right of the first block column) must agree: blocks
                // that differ on their shared edge are not one window.
                let shared = match (br > 0 && r == 0, bc > 0) {
                    (true, _) => BLOCK_SIDE,
                    (false, true) => 1,
                    (false, false) => 0,
                };
                if into[..shared] != row[..shared] {
                    return Err(corrupt(
                        "a final window",
                        "two of its blocks differ on their shared edge",
                    ));
                }
                into.copy_from_slice(row);
            }
        }
    }
    Ok(Window { quanta })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wavy(key: SquareKey) -> Window {
        Window::of(key, |p| {
            let v = (p.i() % 977 - 488) * 7 + (p.j() % 613 - 306) * 3;
            if v == 0 { -1 } else { v as i16 }
        })
    }

    #[test]
    fn a_window_round_trips_its_blocks() {
        let window = wavy(SquareKey { lat: 12, lon: -40 });
        let (blocks, payload) = encode(&window);
        assert_eq!(blocks.len(), 16);
        assert_eq!(decode_window(&payload, &blocks).unwrap(), window);
        // Each block is the window's own quanta, edges shared with the next.
        let b = decode_block(&payload, &blocks[5]).unwrap();
        assert_eq!(b, window.block(1, 1));
        assert_eq!(b.at(0, 0), window.at(400, 400));
        assert_eq!(b.at(400, 400), window.at(800, 800));
        let east = decode_block(&payload, &blocks[6]).unwrap();
        for r in 0..BLOCK_SIDE {
            assert_eq!(b.at(r, 400), east.at(r, 0));
        }
    }

    #[test]
    fn a_block_decodes_alone_and_its_damage_is_its_own() {
        let window = wavy(SquareKey { lat: -3, lon: 100 });
        let (blocks, mut payload) = encode(&window);
        // Every other block's bytes ruined: block 9 still decodes, from its own bytes alone.
        for (k, info) in blocks.iter().enumerate() {
            if k != 9 {
                payload[info.offset..info.offset + info.length].fill(0xa5);
            }
        }
        assert_eq!(
            decode_block(&payload, &blocks[9]).unwrap(),
            window.block(2, 1)
        );
        assert!(decode_block(&payload, &blocks[8]).is_err());
        // Its own damage is caught by its digest.
        let at = blocks[9].offset + blocks[9].length / 2;
        payload[at] ^= 1;
        assert!(decode_block(&payload, &blocks[9]).is_err());
        // A table whose range is wrong is caught before any byte is read.
        let mut table = blocks.clone();
        table[3].min -= 1;
        assert!(check_table(&table, payload.len()).is_ok());
        assert!(decode_block(&encode(&window).1, &table[3]).is_err());
        table.pop();
        assert!(check_table(&table, payload.len()).is_err());
    }

    #[test]
    fn the_bilinear_height_meets_the_lattice_points() {
        let window = wavy(SquareKey { lat: 0, lon: 0 });
        let b = window.block(3, 2);
        assert_eq!(b.bilinear_m([17.0, 33.0]), b.height_m(17, 33));
        assert_eq!(b.bilinear_m([400.0, 400.0]), b.height_m(400, 400));
        let mid = b.bilinear_m([17.5, 33.5]);
        let mean =
            (b.height_m(17, 33) + b.height_m(17, 34) + b.height_m(18, 33) + b.height_m(18, 34))
                / 4.0;
        assert!((mid - mean).abs() < 1e-12);
    }
}
