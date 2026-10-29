//! OpenStreetMap's features as vector tiles: the Mapbox Vector Tile decoder ([`mvt`]), the OpenMapTiles schema's
//! layers the game reads, typed ([`omt`]: roads and railways, waterways, water areas, land use and cover, buildings,
//! points of interest, aerodromes' runways, taxiways and aprons),
//! and the web-mercator tile math ([`tiles`]). The tiles come from OpenFreeMap (no key, no account; the attribution
//! is [`ATTRIBUTION`]).
//!
//! Portable (checked for `wasm32-unknown-unknown`): no file, no network. The native client's `theatre::osm` fetches,
//! caches and projects the tiles into a scenario's frame (native/docs/terrain/surface.md, "Map features on the
//! ground").

pub mod mvt;
pub mod omt;
pub mod tiles;

pub use mvt::{DecodeError, Geometry, Polygon};
pub use omt::{
    Aeroway, Brunnel, Building, Features, Landcover, Landuse, Poi, RoadSurface, TilePoint,
    Transportation, Water, Waterway,
};
pub use tiles::TileId;

/// What the game must credit wherever it shows something drawn from these tiles (OpenFreeMap's terms: "OpenFreeMap ©
/// OpenMapTiles Data from OpenStreetMap"; the data is under the ODbL).
pub const ATTRIBUTION: &str =
    "OpenFreeMap © OpenMapTiles, data from OpenStreetMap contributors (ODbL)";

/// Decodes a tile's bytes into its typed features, points as fractions of the tile.
pub fn features(bytes: &[u8]) -> Result<Features<TilePoint>, DecodeError> {
    Ok(Features::from_tile(&mvt::decode(bytes)?))
}
