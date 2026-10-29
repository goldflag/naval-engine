//! The ground: the heightfield (and its cut from a land release without the simulation, `release`) and its preparation
//! for drawing, the level of detail over it, the fine relief near the camera, the real-world surface maps and their
//! textures, the tiling ground materials, the map features drawn on it and the period's country.

pub mod country;
pub mod features;
pub mod field;
pub mod ground_materials;
pub mod heightfield;
pub mod lod;
pub mod release;
pub mod relief;
pub mod surface_maps;
pub(crate) mod surface_textures;
pub mod textures;
