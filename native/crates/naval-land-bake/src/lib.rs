//! The land release bake's host (docs/earth.md; `release`): the network, the disk and the threads around
//! `naval_region_bake`, which is pure, and the `naval-land-bake` command ([`cli`], README.md), which
//! `naval-client land-release` also answers. It needs nothing of the client or the simulation: its caller names the
//! data directory the store and the sources live under ([`release::store_dir`], [`release::sources_dir`]).
//!
//! - **Threads.** Everything runs off the frame loop: DEM reads on [`FETCHERS`] threads (they wait on the network), the
//!   squares on a pool of the machine's cores less two ([`pool_threads`], at most [`MAX_POOL`]), since the game may be
//!   running. A cancel is seen between squares.
//! - **The pinned sources** a release's bake reads, fetched the first time they are needed from where they are hosted
//!   ([`hosted`]).

pub mod cli;
pub mod dem;
pub mod hosted;
pub mod release;

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use naval_region_bake::BakeError;

use self::dem::DemError;

/// DEM reads in flight at once.
pub const FETCHERS: usize = 12;
/// The most cores the bake takes, whatever the machine has: each tile in flight holds about 100 MB (measured on the
/// California coast: about 0.8 GB at the peak on four threads, 1.0 GB on eight), and the game may be running.
pub const MAX_POOL: usize = 6;
/// The bake's pool: the machine's cores less two (the game may be running), at least one, at most [`MAX_POOL`].
pub fn pool_threads() -> usize {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    cores.saturating_sub(2).clamp(1, MAX_POOL)
}

/// `f` over `items` on `threads` scoped threads, each item once, results in the items' order; stops taking items once
/// `cancel` is set (those left are `None`).
pub fn parallel<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    cancel: &AtomicBool,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<Option<R>> {
    let next = AtomicUsize::new(0);
    let results: Vec<Mutex<Option<R>>> = items.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..threads.clamp(1, items.len().max(1)) {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let k = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(k) else { break };
                    let r = f(item);
                    *results[k].lock().unwrap() = Some(r);
                }
            });
        }
    });
    results
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect()
}

/// Why a bake did not finish.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeFailure {
    Cancelled,
    /// The DEM could not be read (`offline`, `failed`).
    Dem(DemError),
    /// The bake refused (a pole, too many bytes) or the DEM did not decode.
    Bake(String),
    /// The disk.
    Io(String),
    /// A land release's pinned source changed since the release was made (`BakeError::Changed`): never tried again.
    Changed(String),
}

impl std::fmt::Display for BakeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeFailure::Cancelled => f.write_str("the bake was cancelled"),
            BakeFailure::Dem(e) => write!(f, "the elevation could not be read ({e})"),
            BakeFailure::Bake(m) | BakeFailure::Io(m) | BakeFailure::Changed(m) => f.write_str(m),
        }
    }
}

impl From<BakeError> for BakeFailure {
    fn from(e: BakeError) -> Self {
        match e {
            BakeError::Cancelled => BakeFailure::Cancelled,
            BakeError::Changed(m) => BakeFailure::Changed(m),
            other => BakeFailure::Bake(other.to_string()),
        }
    }
}

impl From<DemError> for BakeFailure {
    fn from(e: DemError) -> Self {
        BakeFailure::Dem(e)
    }
}
