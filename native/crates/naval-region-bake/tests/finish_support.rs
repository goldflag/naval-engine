//! What the coast-finishing tests share: synthetic scenes across edges, bakes on fresh sources, and the edges of a
//! final square read back from its file.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Instant;

use naval_land_format::block::{Block, Window};
use naval_land_format::lattice::{BLOCK_STEPS, BLOCKS, WINDOW};
use naval_land_format::{FinalSquare, SquareKey};
use naval_region_bake::finish::synthetic::{Scene, SyntheticSources, World};
use naval_region_bake::finish::{self, Finished};

/// A world of one scene across the edge `a` and `b` share.
pub fn world_across(a: SquareKey, b: SquareKey, seed: u64) -> Arc<World> {
    Arc::new(World::new(vec![Scene::across(a, b, seed)]))
}

/// Square `key` baked on sources of its own (nothing shared with any other bake but the world's definition).
pub fn bake_alone(key: SquareKey, world: &Arc<World>) -> Finished {
    let sources = SyntheticSources::new(world.clone());
    let t = Instant::now();
    let finished = finish::bake(key, &sources, &sources).unwrap();
    eprintln!(
        "{key}: {:.1} s, {:?}",
        t.elapsed().as_secs_f64(),
        finished.report
    );
    finished
}

#[derive(Clone, Copy, Debug)]
pub enum Side {
    North,
    South,
    West,
    East,
}

/// The quanta along one edge of a final square, read from its file a block at a time, as little-endian bytes.
pub fn edge_bytes(square: &FinalSquare, side: Side) -> Vec<u8> {
    let last = BLOCKS - 1;
    let mut out = vec![];
    for b in 0..BLOCKS {
        let (br, bc) = match side {
            Side::North => (0, b),
            Side::South => (last, b),
            Side::West => (b, 0),
            Side::East => (b, last),
        };
        let block = square.block(br, bc).unwrap();
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

/// A square's blocks, each decoded once when first read.
pub struct Blocks<'a> {
    square: &'a FinalSquare,
    decoded: std::collections::HashMap<[usize; 2], Block>,
}

impl<'a> Blocks<'a> {
    pub fn new(square: &'a FinalSquare) -> Self {
        Self {
            square,
            decoded: Default::default(),
        }
    }

    /// The limiting bilinear height along an edge of the square's window, at `t` lattice steps along it (west to east
    /// for north and south edges, north to south for west and east): the square's own interpolation, in the block a
    /// reader takes for that place, approaching the edge from inside.
    pub fn limiting_height(&mut self, side: Side, t: f64) -> f64 {
        let along = t.clamp(0.0, (WINDOW - 1) as f64);
        let edge = match side {
            Side::North | Side::West => 0.0,
            Side::South | Side::East => (WINDOW - 1) as f64,
        };
        let position = match side {
            Side::North | Side::South => [edge, along],
            Side::West | Side::East => [along, edge],
        };
        let cell = naval_land_format::lattice::cell(position);
        let square = self.square;
        self.decoded
            .entry(cell.block)
            .or_insert_with(|| square.block(cell.block[0], cell.block[1]).unwrap())
            .bilinear_m(cell.at)
    }
}

/// Checks two neighbours' final squares share their edge: the same bytes at every edge point, and the same limiting
/// height at every metre of the edge. `b` is east of `a` (`east`) or north of it. Returns the edge's land points.
pub fn assert_shared_edge(a: &Finished, b: &Finished, east: bool) -> usize {
    let (fa, fb) = (
        FinalSquare::read(a.bytes()).unwrap(),
        FinalSquare::read(b.bytes()).unwrap(),
    );
    let (sa, sb) = if east {
        (Side::East, Side::West)
    } else {
        (Side::North, Side::South)
    };
    let (ea, eb) = (edge_bytes(&fa, sa), edge_bytes(&fb, sb));
    assert_eq!(ea.len(), 2 * WINDOW);
    let first = ea.chunks(2).zip(eb.chunks(2)).position(|(x, y)| x != y);
    assert_eq!(
        first, None,
        "{} and {}: the edge's bytes differ from point {first:?}",
        a.key, b.key
    );
    // Every metre of the edge: a meridian is ROW_M a step, a parallel its latitude's column.
    let step_m = if east {
        finish::lattice::ROW_M
    } else {
        finish::lattice::column_m((a.key.lat + 1) * 1600)
    };
    let metres = ((WINDOW - 1) as f64 * step_m).floor() as usize;
    let (mut ba, mut bb) = (Blocks::new(&fa), Blocks::new(&fb));
    for m in 0..=metres {
        let t = m as f64 / step_m;
        let (ha, hb) = (ba.limiting_height(sa, t), bb.limiting_height(sb, t));
        assert!(
            ha == hb,
            "{} and {}: {ha} and {hb} at {m} m along the edge",
            a.key,
            b.key
        );
    }
    ea.chunks(2)
        .filter(|q| i16::from_le_bytes([q[0], q[1]]) > 0)
        .count()
}

/// A window's quanta along its north row (or any row), for a quick look.
pub fn land_points(window: &Window) -> usize {
    window.quanta.iter().filter(|&&q| q > 0).count()
}
