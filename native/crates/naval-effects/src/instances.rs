//! Per-instance data the CPU publishes each frame and the shaders read from storage buffers (one quad of six
//! vertices per instance, `@builtin(instance_index)`). Each struct has a WGSL twin; keep the two in order.
//! Storage buffers replace three's instanced vertex attributes, so the web's eight-vertex-buffer limit and its
//! packing gymnastics do not apply.

use bytemuck::{Pod, Zeroable};

/// A gas puff that relights the baked atlas (`shaders/gas_sprite.wgsl`, twin `GasPuff`): what
/// `EffectParticlePool.publish` writes for a volume batch (`effectSphere`, `effectVolume`, `effectTint`,
/// `effectProgress`, `effectOpacity`) plus the sprite's placement, which three keeps in the instance matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GasPuff {
    /// Centre (m) and radius (m).
    pub sphere: [f32; 4],
    /// Age (s), seed, heat (0 cold … above 1 white-hot), density (extinction multiple).
    pub state: [f32; 4],
    /// Albedo and the major-to-minor ratio of an elongated puff.
    pub tint: [f32; 4],
    /// Life fraction, dispersal (0 fresh … 1 gone), yaw and height of the elongation axis.
    pub evolution: [f32; 4],
    /// Where the camera-facing quad is drawn (pulled half a radius toward the camera) and its edge length. A puff drawn
    /// from a simulation places its own quad: here a fire's flame carries its lean (xyz) and a link of a fire's column
    /// the way the column runs (xyz); a link or billow of a column, the height (m) over which it stands clear of the
    /// flames it rose from (w: above it, it hides them in the occlusion map, `gas_sprite.wgsl`'s `column_occluder`).
    pub quad: [f32; 4],
    /// Opacity, firelight from below (`glow`), the flipbook clip it plays (`flipbook::clip`), and what gas it is:
    /// [`GAS_SMOKE`], [`GAS_SCREEN`], [`GAS_PROPELLANT`], [`GAS_COLUMN`], [`GAS_FIRE`], [`GAS_FIREBALL`] or [`GAS_FLAK`].
    pub extra: [f32; 4],
    /// The warp trial: how far the puff has rolled (`warp::roll`) and the world direction it rolls about.
    pub warp: [f32; 4],
}

/// What gas a puff is (`GasPuff::extra[3]`): smoke, soot and an explosive's fireball, the default.
pub const GAS_SMOKE: f32 = 0.0;
/// A smoke screen the simulation owns, its billows rolled within its volume (`tactical.rs`).
pub const GAS_SCREEN: f32 = 1.0;
/// A gun's propellant gas, which burns in its own colours and for its own time (`gas_sprite.wgsl`'s
/// `propellant_flame`): a turret's fireball and the smoke it leaves (`blasts.rs`).
pub const GAS_PROPELLANT: f32 = 2.0;
/// A billow of a fire's smoke column, drawn among its links on their lean path (`fire_batch.rs`, `billow`): it stands
/// upright in its tile, never turned, as the links do.
pub const GAS_COLUMN: f32 = 3.0;
/// A fire's rolling flame body (`fire_batch.rs`, `FireMode::Gas`, drawn where the flipbook's flame is not): it burns as a
/// fire's flame does (`common.wgsl`'s `flame_light`), at the fires' flames' lift after dark (`EffectLighting::flames`).
pub const GAS_FIRE: f32 = 4.0;
/// A gun's fireball, its propellant's flash (`EffectParticle::fireball`): it burns as [`GAS_PROPELLANT`] does, but
/// white-hot through its first tenth of a second, and leaves fresh soot, dark until it mixes with the air
/// (`gas_sprite.wgsl`'s `propellant_flame` and `fresh_soot`).
pub const GAS_FIREBALL: f32 = 5.0;
/// A heavy AA round's burst (`EffectParticle::flak`, `flak.rs`): its soot is marched through the puff's sphere as one
/// lumpy cloud of its own (`gas_sprite.wgsl`'s `flak_soot`), not read from a tile of the atlas.
pub const GAS_FLAK: f32 = 6.0;

/// A textured camera-facing (or water-lying) sprite: flashes, flame jets, sparks, embers and the fire glows
/// (`shaders/sprite.wgsl`, twin `Sprite`). Also the funnel billows (`shaders/smoke_billow.wgsl`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Sprite {
    /// Centre (m) and roll about the view axis (radians).
    pub position: [f32; 4],
    /// Width and height (m), 1 when the sprite lies flat on the water, how far it has rolled (the warp trial's
    /// `warp::roll`, read by the funnel billows alone).
    pub size: [f32; 4],
    /// Linear colour and opacity.
    pub color: [f32; 4],
}

/// One quad of a ship-fire batch (`FireBatch`): a flame tongue or a puff of the fire's smoke column
/// (`shaders/fire_flame.wgsl`, `shaders/fire_smoke.wgsl`, twin `FireQuad`). The quad's axes are the batch's
/// shared basis (`FireBasis`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FireQuad {
    /// Quad centre (m) and quad width (m).
    pub center: [f32; 4],
    /// The particle's own centre (m, `fireCenter`) and its radius or half height (`fireCenter.w`).
    pub sphere: [f32; 4],
    /// Seed, life fraction, glow (temperature or underglow), opacity (`fireState`).
    pub state: [f32; 4],
    /// Albedo and age in seconds (`fireColor`); `w` of `size` below is the quad height.
    pub color: [f32; 4],
    /// Quad height (m), how far a smoke puff has rolled (the warp trial's `warp::roll`), unused ×2.
    pub size: [f32; 4],
}

/// The axes every quad of a flame or smoke batch shares this frame.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FireBasis {
    pub right: [f32; 4],
    pub up: [f32; 4],
    pub normal: [f32; 4],
}

/// One vertex of a funnel plume ribbon (`SmokeRibbons`, `shaders/smoke_ribbon.wgsl`, twin `RibbonVertex`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct RibbonVertex {
    /// World position and the fraction of the sample's life spent (`plumeSpent`).
    pub position: [f32; 4],
    /// Side vector scaled by how squarely the plume faces the camera (`plumeSide`), unused.
    pub side: [f32; 4],
    /// Across (−1 … 1), metres along the plume, age (s), width (m) (`plumeData`).
    pub data: [f32; 4],
    /// Albedo and opacity (`plumeTint`).
    pub tint: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GasPuff>() == 112);
const _: () = assert!(std::mem::size_of::<Sprite>() == 48);
const _: () = assert!(std::mem::size_of::<FireQuad>() == 80);
const _: () = assert!(std::mem::size_of::<RibbonVertex>() == 64);

/// One sprite of the water effects' spray as `naval-water-effects`' `shaders/sprite.wgsl` draws it (a vertex buffer
/// stepped per instance, not a storage buffer): a quad `center ± axis_u/2 ± axis_v/2` (world metres), its uv's +u along
/// `axis_u` and +v along `axis_v`, built on the CPU by [`crate::particles::ParticlePool::publish_water_into`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct WaterSprite {
    pub center: [f32; 3],
    pub opacity: f32,
    pub axis_u: [f32; 3],
    /// The water level the sprite meets the sea at (`effectSurface`).
    pub surface: f32,
    pub axis_v: [f32; 3],
    /// The share of its life the particle has spent, 0–1: lit spray thins into wisps as it ages.
    pub aged: f32,
    pub color: [f32; 3],
    /// A number of its own, 0–1, fixed for its life: where its shape tears.
    pub seed: f32,
}
