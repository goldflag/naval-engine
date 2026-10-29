//! The land release bake (docs/earth.md; docs/theatre/land.md, "Land releases"): a release's 1° final squares on one
//! global lattice (`finish`, `release`), made from pinned sources: the surveyed heights of the world's squares
//! ([`square`]: Terrarium or Copernicus GLO-30, 2 × 2 supersampled and despiked), OpenStreetMap's water polygons for the
//! coast and GEBCO's grid for the sea floor.
//!
//! Pure: no network, no files, no threads. The caller hands it the sources (decoded Terrarium tiles, GLO-30 headers and
//! blocks, the snapshot's polygons) and runs its squares on whatever threads it likes; its host, `naval-land-bake`
//! (which the game runs), fetches, keeps the store and the source cache on disk and runs them on a pool that leaves two
//! cores free. The theatres' legacy regions it baked before were retired on 2026-10-17 (D-6).

pub mod despike;
pub mod finish;
pub mod geo;
pub mod glo30;
pub mod ntf;
pub mod release;
pub mod square;
pub mod terrarium;

/// A final square's land is kept when surveyed land lies within this of it (`finish::keep::KEEP_M`).
pub const LAND_REACH_M: f64 = 40_000.0;
/// A square is a candidate when charted land lies within this of it (`finish::keep::CANDIDATE_M`).
pub const CANDIDATE_REACH_M: f64 = 65_000.0;
/// A sample not read from the DEM: sea this deep.
pub const DEEP_M: f64 = -500.0;
/// Every land sample stands at least this far above the sea.
pub const LAND_MIN_M: f64 = 1.0;
/// Every sea sample is at least this deep (negative).
pub const SEA_MAX_M: f64 = -2.0;
/// The open sea's floor, metres below the sea.
pub const SHELF_DEPTH_M: f64 = 160.0;
/// Metres deeper per metre off the coast, from 2.5 m at it.
pub const SHELF_SLOPE: f64 = 0.12;
/// A sample above this is surveyed land.
pub const LAND_THRESHOLD_M: f32 = 0.5;
/// The coast rounded over about this (metres).
pub const COAST_ROUND_M: f64 = 140.0;
/// The theatre's sphere (`naval_sim::theatre::geo`, src/theatre/geo.ts).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// The surveyed squares' sample spacing the DEM's zoom is chosen for, metres.
pub const SURVEY_CELL_M: f64 = 70.0;

/// SRTM's coverage, the 30–90 m source Terrarium has between these latitudes; past them Terrarium falls back to coarse
/// sources (Iceland's coast came out in blocks 70–300 m across), so land past either reads Copernicus GLO-30.
pub const SRTM_NORTH: f64 = 60.0;
pub const SRTM_SOUTH: f64 = -56.0;

/// The elevation sources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dem {
    /// AWS Terrain Tiles (Terrarium PNGs in web mercator).
    Terrarium,
    /// Copernicus DEM GLO-30 (Cloud-Optimized GeoTIFFs a degree cell).
    Glo30,
}

impl Dem {
    pub fn name(self) -> &'static str {
        match self {
            Dem::Terrarium => "terrarium",
            Dem::Glo30 => "glo30",
        }
    }
}

/// Why a bake stopped.
#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// A DEM tile or cell the bake needs was not handed to it.
    Missing(String),
    /// The DEM's bytes did not decode.
    Corrupt(String),
    /// The region cannot be baked (a pole within its reach, a bad frame, too many bytes).
    Refused(String),
    /// GLO-30 could not be read (the host's reads failed); the caller may bake from Terrarium instead.
    Glo30Unavailable(String),
    /// A pinned source is no longer the one its land release names (its ETag moved, or it is gone): the release cannot
    /// be baked from it again, so the bake is not tried again.
    Changed(String),
    /// Stopped by the caller.
    Cancelled,
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeError::Missing(m)
            | BakeError::Corrupt(m)
            | BakeError::Refused(m)
            | BakeError::Changed(m) => f.write_str(m),
            BakeError::Glo30Unavailable(m) => {
                write!(f, "Copernicus GLO-30 could not be read ({m})")
            }
            BakeError::Cancelled => f.write_str("the bake was cancelled"),
        }
    }
}

impl std::error::Error for BakeError {}

impl From<naval_land_format::Error> for BakeError {
    fn from(e: naval_land_format::Error) -> Self {
        match e {
            naval_land_format::Error::Corrupt(m) => BakeError::Corrupt(m),
            naval_land_format::Error::Conflict(c) => BakeError::Refused(c.to_string()),
        }
    }
}
