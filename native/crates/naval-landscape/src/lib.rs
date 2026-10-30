//! The real world's look on a theatre's land (native/docs/terrain/pipeline.md, "The land's modules"): what the
//! land layer (`naval_terrain::TerrainLayer`) is handed beside a window's heights, and the buildings' and the
//! infrastructure's chunks over it, fetched from the open sources, cached on disk and made on worker threads.
//! Presentation only: nothing here is read by the simulation, and it needs nothing of the client or the simulation. Its
//! host names the cache folder and the workers' threads ([`host`]).
//!
//! - **The surface** ([`surface`], [`near`], [`naip`]): Sentinel-2 cloudless colour and WorldCover's land cover over a
//!   window, NAIP's in the United States, finer tiles about the camera; the fetch, the cache and the worker the rest
//!   share (`surface::get`, `surface::Cache`, `surface::parallel_map`).
//! - **Map features on the ground** ([`osm`], [`features`]): OpenStreetMap's roads, railways and waters as distance-field
//!   pages, in a past period only where they stood; the lamps along them at night ([`lamps`]).
//! - **Relief, trees and the horizon** ([`relief`], [`canopy`], [`horizon`]): finer elevation pages near the camera
//!   (real, synthetic [`relief_synth`], GLO-30's [`relief_glo30`] or the two combined [`relief_combined`], held to the
//!   window and off the water [`relief_limit`]), the canopy height pages, the horizon ring's maps.
//! - **The period's ground** ([`period_land`]): the country a past period's land cover marks, and whether a place's
//!   ground stood then.
//! - **The sea's colour** ([`water`]): an atlas of coastal waters.
//! - **What stands on the land** ([`buildings`], [`infra`], [`bases`], [`power`]): the building footprints
//!   (Microsoft's, OpenStreetMap's) in chunks, a past period's thinned; the ports' yards, quays and boats, the bridges
//!   and the airfields; the period's generated bases, their clearings and parked aircraft; Overture's power lines and
//!   masts. The host makes their layers with a scene and feeds them about its camera (`buildings::feed`,
//!   `infra::feed`).

pub mod bases;
pub mod buildings;
pub mod canopy;
pub mod features;
pub mod horizon;
pub mod host;
pub mod infra;
pub mod lamps;
pub mod naip;
pub mod near;
pub mod osm;
pub mod period_land;
pub mod power;
pub mod relief;
pub mod relief_combined;
pub mod relief_glo30;
pub mod relief_limit;
pub mod relief_synth;
pub mod surface;
pub mod water;
