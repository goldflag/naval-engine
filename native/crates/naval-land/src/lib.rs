//! Land releases on this machine (globe stage 2, step C: C1, §3.1, §6). The formats are `naval-land-format`'s, which the
//! simulation decodes with; this crate keeps them on disk.
//!
//! - **The store** ([`Store`]) keeps each installed release's record (its file, square table and source manifest), its
//!   append-only manifest, and its final squares **by content**, at `<data>/land/<release>/<square>/<sha256>.nsq`.
//!   A square's manifest line is written before its bytes, under a lock on the manifest file, so two processes never
//!   record two answers for one square; a bake whose bytes differ from the recorded line is refused and never stored.
//! - **Pins and leases.** A holder (a running world, a save, a checkpoint, a clip: SAVE builds them from their files)
//!   pins the squares of its closure; a reader holds a [`Lease`] while it uses a square; a world's [`Requests`] lease
//!   each square it asks for by its key, from the request on, its bytes still to be baked included. Nothing pinned,
//!   leased or requested is evicted, and nothing at all until the owner has pinned what is on disk and
//!   [settled](Store::settle) the store. Pins and leases are a store's own, so a folder has one store at a time: it
//!   stays locked until the store and every lease and request of it are dropped, and a second open is refused.
//! - **The size bound.** Past it the store evicts its least recently used squares that nothing pins, leases or
//!   requests; when only those remain it stays over the bound rather than lose one, never waits, and says so
//!   ([`Usage::over`], and a line when it first goes over). An evicted square's manifest line stays: it is baked again
//!   (and must give the same bytes) or imported.
//! - **Export** ([`Store::export`], [`Closure`]): a closure's land in one file (the release record, the manifest lines
//!   of its squares, `empty` ones included, and their bytes), which [`Store::import`] checks whole before it keeps any.
//!
//! The binary (`cargo run -p naval-land -- synthetic <data>/land`) installs the synthetic release with its squares.

mod export;
mod store;

pub use export::{BUNDLE_MAGIC, Closure};
pub use naval_land_format as format;
pub use store::{
    Answer, Content, DEFAULT_BOUND_BYTES, LOCK_FILE, Lease, MANIFEST_FILE, RELEASE_FILE,
    ReleaseRecord, Requests, SOURCES_FILE, Store, StoreError, TABLE_FILE, Usage,
};

use naval_land_format::{ReleaseId, synthetic};

/// Installs the synthetic release (`naval_land_format::synthetic`) in `store` with every one of its squares: its
/// manifest's lines are recorded in the store's manifest in its order.
pub fn install_synthetic(store: &Store) -> Result<ReleaseId, StoreError> {
    let made = synthetic::release();
    let id = store.install(&made.release, &made.table, None)?;
    let manifest =
        naval_land_format::Manifest::parse(&made.manifest).map_err(StoreError::Refused)?;
    for (key, entry) in manifest.iter() {
        match entry {
            naval_land_format::Entry::Empty => store.put_empty(id, key)?,
            naval_land_format::Entry::Data(_) => store.put(id, key, &made.squares[&key])?,
        };
    }
    Ok(id)
}
