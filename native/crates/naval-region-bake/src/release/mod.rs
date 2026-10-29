//! A real land release's sources (step C, BAKE-2: the open-ocean plan's C1 and §3.1, "Making a release"). The finish
//! ([`crate::finish`]) bakes a square from a surveyed layer and the water polygons; this module says what those are for
//! a release, every one **pinned**: two machines on one release id read the same source bytes or fail.
//!
//! - **The coast** ([`snapshot`]): one OpenStreetMap water-polygon snapshot from osmdata.openstreetmap.de
//!   (`water-polygons-split-4326.zip`, its date and SHA-256 in the release file), read as it is: a shapefile of
//!   polygons split into a 1° grid with a slight overlap. [`pieces`] cuts it per square into the water that square's
//!   bake reads; the client keeps the pieces in a bounded cache and cuts them again from the snapshot when they are gone.
//! - **The square table** ([`table`]): every square `ocean`, `candidate` or `polar`, computed once from the whole
//!   snapshot when the release is made.
//! - **The heights, by place** ([`dem_at`], [`surveyed`]): Terrarium for latitudes in [−56°, 60°), Copernicus GLO-30
//!   elsewhere, whichever square's bake asks, so a halo crossing 60° N reads GLO-30 beyond it.
//! - **The sea floor** (from `land-v2`, the second bake: `finish`'s `Seabed`): GEBCO's global grid ([`gebco`]), one
//!   pinned object read by HTTP range wherever the water polygons say sea.
//! - **The source manifest** ([`sources`]): a listing, made before the release id is published, of every GLO-30 object
//!   and every Terrarium tile a candidate square's surveyed square can read, each with its ETag and length, GLO-30's
//!   absent cells included. Every read sends the frozen ETag (`If-Match`): a changed or unreadable object fails the
//!   bake, and there is no fallback (the editor's coasts bake Terrarium where GLO-30 cannot be read; a release never).
//!
//! **The date line.** A square never straddles ±180°. Inside a bake longitudes stay continuous about the square (a halo
//! place at 180.3° is 180.3°); every source request is split at ±180° into its sides ([`sources::Side`]), each side's
//! objects named wrapped onto the world and placed in the square's continuous frame, so no mosaic's bounds see a
//! wrapped span. **The pole limit** is the table's: a square meeting it is `polar`, never baked.
use crate::{Dem, SRTM_NORTH, SRTM_SOUTH};

pub mod fixtures;
pub mod gebco;
pub mod pieces;
pub mod snapshot;
pub mod sources;
pub mod surveyed;
pub mod table;

/// The snapshot's file on osmdata.openstreetmap.de.
pub const SNAPSHOT_FILE: &str = "water-polygons-split-4326.zip";
pub const SNAPSHOT_URL: &str =
    "https://osmdata.openstreetmap.de/download/water-polygons-split-4326.zip";
/// The first release's name (the `land-v1` the first bake made), and the name of the release bake `version` makes:
/// `land-v<version>`.
pub const NAME: &str = "land-v1";
pub fn name(version: u32) -> String {
    format!("land-v{version}")
}
/// `land-v1`'s id: the SHA-256 of `assets/land/land-v1.release.json`, published once its snapshot, source manifest and
/// square table were frozen (2026-10-14). Its square table is `assets/land/land-v1.squares.nlt`; its source manifest
/// (32 MB, every object a candidate square reads) is not committed: the release names its hash, and it lives with the
/// release in the store (`<data>/land/<id>/sources.json`) and beside the snapshot (`<data>/land-sources/land-v1/`).
pub const LAND_V1: &str = "82f7bab8fb8114bd79c2fa25a76c399b839d351cafdf7eab5e53a6446d105e8c";
/// `land-v2`'s id: the SHA-256 of `assets/land/land-v2.release.json`, made for the second bake (a real sea floor,
/// GEBCO 2026's) on `land-v1`'s snapshot and square table, published once its source manifest (`land-v1`'s objects and
/// the sea floor's) was frozen (2026-10-15). Its manifest lives as `land-v1`'s does.
pub const LAND_V2: &str = "d309e9a9a6cc3175c407fd84c7f948028f264730e17a6ea5c20af9c53cba1e91";
/// The attribution the squares carry (the snapshot's licence, ODbL).
pub const OSM_ATTRIBUTION: &str =
    "© OpenStreetMap contributors (ODbL), water polygons from osmdata.openstreetmap.de";

/// The DEM a place reads: Terrarium for latitudes in [−56°, 60°), Copernicus GLO-30 elsewhere.
#[inline]
pub fn dem_at(lat: f64) -> Dem {
    if (SRTM_SOUTH..SRTM_NORTH).contains(&lat) {
        Dem::Terrarium
    } else {
        Dem::Glo30
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dem_is_chosen_by_place_half_open() {
        assert_eq!(dem_at(59.999_999), Dem::Terrarium);
        assert_eq!(dem_at(60.0), Dem::Glo30);
        assert_eq!(dem_at(-56.0), Dem::Terrarium);
        assert_eq!(dem_at(-56.000_001), Dem::Glo30);
        assert_eq!(dem_at(0.0), Dem::Terrarium);
    }
}
