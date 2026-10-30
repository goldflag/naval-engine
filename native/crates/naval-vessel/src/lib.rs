//! Any glTF ship on the sea, as a visual only: no weapons, no internals (native/docs/vessel.md).
//!
//! A model downloaded from anywhere (any scale, any axes, any origin, PBR metallic-roughness materials, open hulls)
//! comes with a setup file beside it ([`setup::Setup`], `<model>.vessel.json`: its length or scale, its up and bow
//! axes, its waterline or draft, its funnels and lamps), which `naval-vessel inspect <model>` starts from a guess.
//! [`VesselModel::load`] reads the two, refusing what it cannot draw with the reason (a Draco or meshopt mesh, a KTX2
//! or WebP texture, an unknown required extension), and resolves the setup into a [`Vessel`]: her length, beam,
//! draft, waterplane, funnels and lamps in ship space (+X starboard, +Y up from the waterline, −Z ahead, metres), which
//! the floating, the wake and the smoke of a host read.
//!
//! [`VesselLayer`] draws them: several ships at once, each a model and a full transform ([`VesselPose`] builds one
//! from a position, heading, pitch, roll and heave), lit as the scene's other meshes are (the sun or moon, the sky's
//! environment, the sun's shadow maps both ways, the scene's point lights, the air's haze) through the renderer's HDR
//! and display path, their navigation lamps glowing by night.

pub mod dfg;
#[cfg(not(target_arch = "wasm32"))]
pub mod files;
pub mod images;
pub mod inspect;
pub mod layer;
pub mod lighting;
pub mod model;
pub mod read;
pub mod setup;
pub mod vessel;

pub use inspect::{Inspection, inspect};
pub use layer::{ModelId, VesselInstance, VesselLayer, VesselStats};
pub use lighting::{VesselLighting, candela_scale};
pub use model::{Vertex, VesselModel, no_files};
pub use read::{Alpha, Material, Resolver};
pub use setup::{Axis, LampKind, Setup};
pub use vessel::{Bounds, Funnel, Lamp, Vessel, VesselPose, Waterplane};
