//! The round Earth over plain values, without the simulation: what the simulation's world types
//! (`naval_sim::earth`) compute with, and what the native renderer can place things on the real Earth by.
//!
//! - [`geo`]: latitude and longitude ([`geo::LatLon`], [`geo::wrap_lon`]) and the theatre's local frame, the azimuthal
//!   equidistant projection about a scenario's origin ([`geo::to_local`], [`geo::to_lat_lon`]), with great-circle
//!   distances, bearings and destinations.
//! - [`sphere`]: the sphere's mathematics over unit vectors (a place, a frame laid at it, a carry between two places,
//!   long-range forms) and its chart ([`sphere::Chart`]: chart metres, latitude and longitude, the Jacobian).
//! - [`squares`]: the 1° latitude–longitude squares and the footprints that meet them; [`lattice`]: the Earth's
//!   lattices, the cube-sphere cells ([`lattice::CubeCell`]), the sea anchors every whole degree and the cloud anchors.
//! - [`sun`] and [`moon`]: their positions and the local sidereal time, from a scenario's local start or its UTC clock.
//! - [`vector`]: the `[f64; 3]` arithmetic the sphere uses, the simulation's `geometry` too.
//!
//! naval-sim re-exports every module at its old path (`earth::geo`, `earth::sphere`, `earth::squares`,
//! `earth::lattice`, `environment::sun`, `environment::moon`, `theatre::geo`, `theatre::sun`), so the simulation and
//! its callers did not change; docs/earth.md says what each decision is and why.
#[cfg(feature = "earth-census")]
pub mod census;
pub mod geo;
pub mod lattice;
pub mod moon;
pub mod sphere;
pub mod squares;
pub mod sun;
pub mod vector;

/// The Earth's radius, metres: the sphere's ([`sphere::R`]), the theatre's local frame's ([`geo`]) and the radius for
/// the sight-line drop of an observer's horizon (`naval_sim::terrain`).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
