//! (b) One square, one set of bytes (the open-ocean plan, step C §7.2, "The bake"): baked alone, in a batch with others
//! on shared sources (in another order), twice, and on one thread or three, a square gives the same bytes.
use std::sync::Mutex;

use naval_land_format::SquareKey;
use naval_region_bake::finish::synthetic::SyntheticSources;
use naval_region_bake::finish::{self, Finished};

use crate::finish_support::world_across;

fn k(lat: i32, lon: i32) -> SquareKey {
    SquareKey { lat, lon }
}

/// Bakes `keys` on `threads` threads sharing one set of sources, each thread taking the next square as it frees.
fn on_threads(keys: &[SquareKey], sources: &SyntheticSources, threads: usize) -> Vec<Vec<u8>> {
    let next = Mutex::new(0usize);
    let out = Mutex::new(vec![vec![]; keys.len()]);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let n = {
                        let mut next = next.lock().unwrap();
                        *next += 1;
                        *next - 1
                    };
                    let Some(&key) = keys.get(n) else {
                        break;
                    };
                    let bytes = finish::bake(key, sources, sources).unwrap().bytes();
                    out.lock().unwrap()[n] = bytes;
                }
            });
        }
    });
    out.into_inner().unwrap()
}

#[test]
fn a_square_is_one_set_of_bytes_alone_in_a_batch_twice_and_on_any_threads() {
    let a = k(-30, -141);
    let world = world_across(a, a.east(), 77);
    let keys = [a, a.east(), a.north().unwrap(), k(-31, -140)];
    // Alone: each on sources of its own.
    let alone: Vec<Finished> = keys
        .iter()
        .map(|&key| {
            let sources = SyntheticSources::new(world.clone());
            finish::bake(key, &sources, &sources).unwrap()
        })
        .collect();
    assert!(alone[0].report.land > 0 && alone[1].report.land > 0);
    // Twice: again, alone.
    let again = {
        let sources = SyntheticSources::new(world.clone());
        finish::bake(a, &sources, &sources).unwrap()
    };
    assert_eq!(again.bytes(), alone[0].bytes());
    // In a batch, in the other order, every surveyed square shared.
    let shared = SyntheticSources::new(world.clone());
    for (n, &key) in keys.iter().enumerate().rev() {
        let batch = finish::bake(key, &shared, &shared).unwrap();
        assert_eq!(batch.bytes(), alone[n].bytes(), "{key} in a batch");
    }
    // On one thread and on three, sharing fresh sources.
    for threads in [1, 3] {
        let sources = SyntheticSources::new(world.clone());
        let bytes = on_threads(&keys, &sources, threads);
        for (n, b) in bytes.iter().enumerate() {
            assert_eq!(b, &alone[n].bytes(), "{} on {threads} threads", keys[n]);
        }
    }
}
