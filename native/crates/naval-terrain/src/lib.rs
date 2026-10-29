//! The land for the native client: real-elevation terrain drawn with the web's land shading, trees and level of
//! detail, on the heightfields the theatre's windows build, in the scene's conditions (`naval-conditions`: the map,
//! the land's style and climate, the sky, the sea and the fog). See `native/docs/terrain/README.md`.
//!
//! Portable (checked for `wasm32-unknown-unknown`): the crate reads no file; the host hands it its heightfields and the
//! bytes of the detail photographs and of the ground materials.

pub mod bc7;
pub mod buildings;
pub mod dfg;
pub mod earth;
pub mod ground;
pub mod horizon;
pub mod infra;
pub mod land;
pub mod land_shadows;
pub mod layer;
pub mod night;
pub mod noise;
pub mod open_sky;
pub mod review;
pub mod sunlight;
pub mod trees;

// The tree and ground modules at their old paths (`naval_terrain::species`, `crate::field`).
use ground::surface_textures;
pub use ground::{
    country, features, field, ground_materials, heightfield, lod, release, relief, surface_maps,
    textures,
};
use trees::tree_shadows;
pub use trees::{
    canopy, crown, forest_shade, grass, impostors, near_trees, species, tree_growth, tree_meshes,
};

pub use canopy::{CanopyGrid, CanopyPage};
pub use country::{Country, CountryLook};
pub use features::{FeatureGrid, FeaturePage, Stroke, StrokeKind, WaterArea};
pub use ground_materials::GroundMaterials;
pub use heightfield::Heightfield;
pub use horizon::{HorizonLayer, HorizonMaps};
pub use impostors::TreeAtlas;
pub use land::PreparedLand;
pub use layer::{
    CLEARING_CAPSULES, CLEARING_GROUPS, ClearingCapsule, ClearingGroup, SurfacePhoto, TerrainDebug,
    TerrainLayer, TerrainLight, TerrainSettings, TreePart,
};
pub use lod::TerrainQuality;
pub use night::{Lamp, LampKind, LampPalette, LightsRule, NightLights};
pub use relief::{ReliefGrid, ReliefHeights, ReliefPage};
pub use sunlight::LandWorker;
pub use surface_maps::{CoverLayer, CoverPatch, NearGrid, NearTile, SurfaceMaps};
