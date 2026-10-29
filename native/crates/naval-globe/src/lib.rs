//! The round Earth seen from space (native/docs/ui-contract/maps.md, "The editor's globe"): an orthographic globe,
//! north up, seen from straight above a view centre, and the world chart on it, drawn on wgpu with no simulation. The
//! game's client draws it under the scenario editor's chart; `engine-example --globe` shows it on its own.
//!
//! - [`globe`]: the view's maths: latitude and longitude as unit vectors, the camera and its inverse (picking), the
//!   grab that keeps a point under the pointer, and a region frame's metres (azimuthal equidistant about an origin) on
//!   the sphere.
//! - [`world`]: the world chart (`public/theatre/world/world-v1.bin`: Natural Earth's land, coasts, reefs and depth
//!   bands), decoded and made into meshes of unit vectors once, on a thread, in two levels of detail.
//! - [`pass`]: its drawing (`shaders/globe_world.wgsl`): the disc and space round it, the depth bands, the graticule,
//!   the land, the coasts and the reefs, each vertex projected on the GPU, so a camera change tessellates nothing. The
//!   colours are the page's (`GlobeStyle::parse` of its CSS text) or the chart's own (`GlobeStyle::chart`).

pub mod globe;
pub mod pass;
pub mod world;

pub use globe::{EARTH_RADIUS_M, GeoFrame, Globe};
pub use pass::{GlobeDraw, GlobePass, GlobePipelines, GlobeStyle, Pipes, StyleText};
pub use world::{WorldMeshes, WorldStore};
