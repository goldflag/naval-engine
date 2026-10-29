//! Combat gas, fire and smoke for the native client, spawned from simulation events.
//!
//! A port of the web client's effects (`CombatEffects.ts` and what it drives): muzzle blasts, shell
//! strikes, HE bursts and magazine detonations as gas puffs that relight a GPU-baked atlas (`GasAtlas.ts`);
//! ship fires as flame tongues, rolling flame bodies, embers and buoyant smoke columns
//! (`LocalizedFireEffects.ts`, `FireBatch.ts`, `FireMaterials.ts`); funnel exhaust as billows over one
//! continuous ribbon (`ShipFunnelSmoke.ts`, `SmokeRibbons.ts`); and the quarter-resolution smoke occlusion map
//! that hides smoke and fire behind denser gun smoke (`SmokeOcclusion.ts`).
//!
//! Two halves:
//! - [`CombatEffects`] is the CPU simulation (portable, no GPU): feed it [`CombatEvent`]s and [`ShipFrame`]s
//!   each frame; it keeps the particle pools and publishes an [`EffectsFrame`] of instances for a camera.
//! - [`EffectsLayer`] is a `naval_render::SceneLayer`: it bakes the gas atlas, draws the occlusion map and a
//!   depth copy in `prepare`, and draws every batch in the transparent phase.
//! - [`ShellLayer`] draws shells in flight (first `CombatEffects.updateShells`, `ShellTrails.ts`): bodies, and the
//!   lighter rounds' tracers (a red spark, its streak and smoke), from the packet's rounds ([`ShellState`]), its CPU
//!   side in [`ShellVisuals`].
//! - [`ShockLayer`] draws the blast waves of guns and explosions (not a port): each front's bend of the scene behind
//!   it and the cloud that condenses behind it, from [`CombatEffects::shocks`] ([`ShockWaves`]).
//!
//! Portable: builds for `wasm32-unknown-unknown` (no file system, no threads, no blocking on the GPU).
//! The design, the event-to-effect mapping and what matches the web are in `native/docs/combat-effects/README.md`.

pub mod air_light;
pub mod blasts;
pub mod camera;
pub mod combat;
pub mod cookoffs;
pub mod damage;
pub mod debris;
pub mod events;
pub mod fire_batch;
pub mod fires;
pub mod fixture;
mod flak;
pub mod flipbook;
mod flipbook_blasts;
mod flipbook_fires;
pub mod flotsam;
pub mod funnel;
pub mod gas_atlas;
pub mod gpu;
pub mod illumination;
pub mod instances;
pub mod layer;
pub mod loss;
pub mod math;
pub mod particles;
pub mod random;
pub mod rules;
pub mod sea;
pub mod sea_fires;
pub mod shadow;
pub mod shell_layer;
pub mod shells;
pub mod ships;
pub mod shock;
pub mod shock_layer;
mod sun_casters;
pub mod tactical;
pub mod textures;
pub mod trial;
pub mod volume;
mod volume_pass;
pub mod warp;

pub use camera::CameraView;
pub use combat::{CombatEffects, EffectCounts, EffectLighting, EffectsFrame, SIM_DT};
pub use events::CombatEvent;
pub use fires::PointLight;
pub use layer::{DepthSource, DrawStats, EffectFog, EffectsLayer};
pub use math::{Rgb, ShipPose};
pub use rules::EffectRules;
pub use shell_layer::{ShellDrawStats, ShellLayer};
pub use shells::{ShellCounts, ShellFrame, ShellState, ShellVisuals};
pub use ships::{FireState, FireTrend, SectionFrame, ShipEffectsProfile, ShipFrame};
pub use shock::{Shock, ShockWaves};
pub use shock_layer::ShockLayer;
pub use trial::Trial;
