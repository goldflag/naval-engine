//! Effects on and above the water, driven by simulation state (see `native/docs/water-effects.md`).
//!
//! The effects that live on the sea surface itself (the wake field, trail and torpedo foam, bow waves, hull contact foam)
//! are the ocean's (`naval_ocean::ship_wake`), since the surface shades them. This crate holds what flies above it (shell
//! splashes and bow spray, their particles and their lighting), the [`fixture`] format a recorded battle is played back
//! from, the [`drive`] conversion from a frame of simulation state to the wakes' inputs, and proxy [`hulls`] for stills.
//!
//! - [`splash`]: the water every combat event throws, seeded per event as the web seeds it: a column of water each
//!   ([`columns`]: marched through on the GPU, its column, veil, crown and surge where ballistics puts them).
//! - [`bow_spray`]: the spray off the bows of hulls underway, marched through with the splashes' columns: the sheet off
//!   each side's crest ([`bow_sheets`]) and what a plunge throws ([`bow_bursts`], off her flare and over her stem head);
//!   and the drops flying out of them.
//! - [`blast_sheet`]: the sheet of spray a heavy turret's blast tears off the sea under its guns.
//! - [`lighting`]: the scene light the spray is shaded with. The drops and billows are particles of the effects kit
//!   `naval-effects` shares (`naval_effects::particles`, `PoolKind::Water`), with its sprite maps
//!   (`naval_effects::textures`), its random stream (`naval_effects::random`) and its camera
//!   (`naval_effects::CameraView`, placed by `CameraView::from_render`).
//! - [`layer`]: [`WaterEffects`], the `naval_render::SceneLayer` that draws them all in the transparent phase.
//!
//! Portable: passes `cargo check -p naval-water-effects --target wasm32-unknown-unknown`. The recorder and the stills are
//! native-only examples.

pub mod blast_sheet;
pub mod bow_bursts;
pub mod bow_sheets;
pub mod bow_spray;
pub mod columns;
pub mod drive;
pub mod fixture;
pub mod hulls;
pub mod layer;
pub mod lighting;
pub mod runoff;
pub mod splash;

pub use blast_sheet::{BlastSheets, GunBlast};
pub use bow_spray::{Aboard, BowSpray, CrestShape, EffectShip, HullSection as WaterHullSection};
pub use layer::{EffectCounts, EffectsFrame, WaterEffects};
pub use lighting::EffectLighting;
pub use splash::Splashes;
