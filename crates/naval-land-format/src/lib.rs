//! Land releases' formats (globe stage 2, step C: C1). The simulation reads land by latitude and longitude from
//! immutable, versioned **releases**; this crate is everything about a release that is bytes, shared by the simulation
//! (which decodes squares, on every target including wasm32), the bake (`naval-region-bake`, which writes them) and the
//! native store (`naval-land`, which keeps them). No files, threads or clock here.
//!
//! - **The square** ([`SquareKey`]): 1° × 1° by its south-west corner (`N36W123`), longitudes −180 to 179, so no
//!   square straddles ±180°. A place belongs to one square, half-open: `[lat, lat + 1) × [lon, lon + 1)`.
//! - **The final lattice** ([`lattice`]): one global lattice, a point every 1/1,600° of latitude and of longitude at
//!   every latitude. A square's final layer is its window of that lattice, 1,601 × 1,601 points with both edges, so two
//!   squares' shared edge is the same points (and the same bytes) whatever the latitude band, and 180° E is 180° W.
//! - **The final layer** ([`block`], [`nsq`]): the window's heights in 0.25 m quanta, kept as sixteen 0.25°
//!   blocks of 401 × 401 points (neighbouring blocks share their edge), each compressed and hashed apart, so a reader
//!   decodes only the blocks it reads. It is a layer of NSQ1, the world square's file; a release's square holds it alone
//!   ([`nsq::FinalSquare`]).
//! - **The release** ([`release`]): a small file naming the lattice, the bake's algorithm and version, and the hashes
//!   of its source snapshot, its source manifest and its square table; its id is the SHA-256 of the file.
//! - **The square table** ([`table`]): every one of the 64,800 squares `ocean` (open sea, authoritatively, never
//!   baked), `candidate` (baked) or `polar` (past the pole limit: open sea).
//! - **The manifest** ([`manifest`]): append-only, one line a square, `N36W123 <sha256>` or `N36W123 empty`. A line once
//!   written never changes: a second bake that gives other bytes is a [`manifest::Conflict`], never stored.
//! - **The synthetic release** (`synthetic`, feature `synthetic`): a deterministic coastal world with no sources, for
//!   every later lane's fixtures.

pub mod block;
pub mod digest;
pub mod key;
pub mod lattice;
pub mod manifest;
pub mod nsq;
pub mod release;
#[cfg(feature = "synthetic")]
pub mod synthetic;
pub mod table;

pub use digest::Digest;
pub use key::SquareKey;
pub use manifest::{Entry, Manifest};
pub use nsq::FinalSquare;
pub use release::{Release, ReleaseFile, ReleaseId};
pub use table::{Class, SquareTable};

/// Why bytes were refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Bytes that are not what they claim: a bad magic or length, a digest that does not match, a stream that does not
    /// inflate, a field out of its range.
    Corrupt(String),
    /// A manifest line that would change: the square's bake gave other bytes than the release records.
    Conflict(manifest::Conflict),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Corrupt(why) => f.write_str(why),
            Error::Conflict(conflict) => conflict.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

pub(crate) fn corrupt(what: &str, why: impl std::fmt::Display) -> Error {
    Error::Corrupt(format!("not {what}: {why}"))
}
