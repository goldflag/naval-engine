//! The weather texture's laying about the camera, as every pass reads it: one set of uniforms ([`LayingUniforms`])
//! serves the march, the shadow maps, the ground map and the reconstruction. With no simulated field the texture lies
//! on the scene's own grid, drifting by the sky's wind ([`Laying::flat`]). A simulated field's lies on its cloud
//! anchors (step C's C13), laid about the camera by a [`Placement`] the sky asks every frame (the game's is
//! `naval-clouds-sim`, which builds on the simulation; this crate does not): for each of up to [`MAX_ANCHORS`] anchors
//! the second-order map from the scene's metres about the camera to that anchor's chart, the noise frame the shapes and
//! the detail are laid on, and the anchors' weights over the scene, normalized, in a small map about the camera
//! ([`WEIGHTS_SIZE`]).
//!
//! A re-lay of the scene (its chart laid about another place) or a change of the anchors drawn drops every history
//! ([`Laying::relaid`]).
//!
//! Where more than one anchor holds a place their values are blended keeping the texture's statistics, as the
//! simulation blends them (`naval_cloud_field::blend`, with its tables in [`WeatherTables`]).
//!
//! **The composite.** Read anchor by anchor, every weather sample in a cell's band takes a fetch an anchor, at up to
//! four times a single anchor's cost. With more than one anchor drawn, a compute pass (`shaders/weather.wgsl`) lays the
//! anchors' blended weather into two textures about the camera, each texel the weights, maps and blend above read at
//! its centre, and every pass reads the weather with one fetch from them: the near one ([`COMPOSITE_NEAR_SIZE`] texels
//! of the tile's own texel, 195 m, 200 km each side of the camera, where the march reads the tile's finest level) and
//! the far one (half as fine, 300 km each side), each with [`COMPOSITE_LEVELS`] levels read from the tile's levels,
//! four channels of eight bits as the tile's. What lies past them, or asks a coarser level than they hold, is read
//! anchor by anchor.
//!
//! The composites are laid again only when they must be: when the camera is [`COMPOSITE_MOVE_M`] from where they were
//! laid, or the drift has carried an anchor's texture [`COMPOSITE_DRIFT_M`] from where the composites have it, over
//! [`COMPOSITE_SLICES`] frames, a slice of rows a frame ([`Laying::take_composite_slices`]), every slice for the frame
//! the laying began (the layer keeps that frame's uniforms for it); whole at once at a re-lay, a change of the anchors
//! drawn, a weights map laid again, or before any is laid. They are read moved by the clouds' drift in the scene since the frame they were laid
//! for (the noise anchor's, which every anchor's drift is near: `LayingUniforms::composite`), the slices laid again
//! by this laying's, the rest by the last's, so an anchor whose drift differs strays from its place by no more than
//! that before they are laid again.

use crate::layers::Layers;
use bytemuck::{Pod, Zeroable};
use naval_cloud_field::WEATHER_TILE_M;

/// The most anchors drawn at once: where four cells meet. Within the weights map's reach of any place no more cells'
/// grown regions meet (`naval-clouds-sim`'s `tests/laying.rs`).
pub const MAX_ANCHORS: usize = 4;
/// The period the shapes, the detail and the turbulence share, metres: the shape volume's (3,333 m), the detail's
/// (100 m) and the turbulence's (5 km) each divide it, so the noise frame's origin is sent with it removed.
pub const NOISE_PERIOD_M: f64 = 10_000.0;
/// Texels per edge of the anchors' weights map.
pub const WEIGHTS_SIZE: u32 = 128;
/// How far across the weights map is, metres: the clouds' longest ray (200 km) each side of the camera, and the
/// camera's way before the map is laid again about it.
pub const WEIGHTS_EXTENT_M: f64 = 800_000.0;
/// Texels per edge of the near composite: the tile's texel ([`WEATHER_TILE_M`] / 512, 195.3 m), 200 km each side of
/// the camera (the march's longest ray, and where its level-of-detail first asks the tile's second level).
pub const COMPOSITE_NEAR_SIZE: u32 = 2048;
/// Texels per edge of the far composite: twice the tile's texel, 300 km each side of the camera.
pub const COMPOSITE_FAR_SIZE: u32 = 1536;
/// The levels each composite holds: the near one the tile's 0 to 2, the far one its 1 to 3.
pub const COMPOSITE_LEVELS: u32 = 3;
/// The near composite's texel, metres: the tile's own.
pub const COMPOSITE_NEAR_TEXEL_M: f64 = WEATHER_TILE_M / 512.0;
/// The far composite's texel, metres.
pub const COMPOSITE_FAR_TEXEL_M: f64 = 2.0 * COMPOSITE_NEAR_TEXEL_M;
/// The camera's way from where the composites were laid before they are laid again about it, metres: eight of the far
/// one's texels, so their edges stay within 3 km of where a camera in the middle has them.
pub const COMPOSITE_MOVE_M: f64 = 8.0 * COMPOSITE_FAR_TEXEL_M;
/// How far an anchor's texture may drift from where the moved composite has it before the composites are laid again,
/// metres: half the tile's texel.
pub const COMPOSITE_DRIFT_M: f64 = COMPOSITE_NEAR_TEXEL_M / 2.0;
/// The slices of rows the composites are laid again in, one a frame.
pub const COMPOSITE_SLICES: usize = 8;

/// The blend's tables (`naval_cloud_field::blend::Tables`) as the shader reads them (`shaders/common.wgsl`,
/// `WeatherTables`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct WeatherTables {
    pub forward: [[f32; 4]; naval_cloud_field::blend::LEVELS],
    pub inverse: [[f32; 4]; naval_cloud_field::blend::INVERSE],
    pub mean: [f32; 4],
}

impl WeatherTables {
    /// The tables the simulation blends with (`naval_cloud_field::blend::tables`), in `f32`.
    pub fn of_simulation() -> Self {
        let t = naval_cloud_field::blend::tables();
        let mut out = Self::zeroed();
        for (o, f) in out.forward.iter_mut().zip(&t.forward) {
            *o = f.map(|v| v as f32);
        }
        for (o, i) in out.inverse.iter_mut().zip(&t.inverse) {
            *o = i.map(|v| v as f32);
        }
        out.mean = t.mean.map(|v| v as f32);
        out
    }
}

/// The laying's part of `CloudUniforms` (`shaders/common.wgsl`), field for field.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct LayingUniforms {
    /// x how many anchors lay the weather (1 to 4: one reads no weights), y the weights map's 1 / extent (per m), zw
    /// the camera less the weights map's corner (m, the scene's axes).
    pub anchors: [f32; 4],
    /// Each anchor's map at the camera: xy its value (m on its chart, its drift off and the tile's period removed), zw
    /// its south row's mixed and z-z second derivatives.
    pub anchor_values: [[f32; 4]; MAX_ANCHORS],
    /// Each anchor's Jacobian: (∂east/∂x, ∂east/∂z, ∂south/∂x, ∂south/∂z).
    pub anchor_jacobians: [[f32; 4]; MAX_ANCHORS],
    /// Each anchor's second derivatives: (east xx, east xz, east zz, south xx).
    pub anchor_hessians: [[f32; 4]; MAX_ANCHORS],
    /// The noise frame's Jacobian, as an anchor's.
    pub noise_jacobian: [f32; 4],
    /// xy the camera in the noise frame (m, its period removed), zw its south row's mixed and z-z second derivatives.
    pub noise_origin: [f32; 4],
    /// The noise frame's second derivatives, as an anchor's: (east xx, east xz, east zz, south xx).
    pub noise_hessian: [f32; 4],
    /// The composites' corners less the camera (m, the scene's axes) as the latest laying lays them: xy the near one's,
    /// zw the far one's, on the chart's grid of the far texel (so the two nest), moved by the clouds' drift since.
    pub composite: [f32; 4],
    /// The same for the laying before it, which the rows the latest has not reached yet still hold.
    pub composite_last: [f32; 4],
    /// x the first slice of the composites' rows laid this frame, y how many (none: 0), z the share of their rows the
    /// latest laying holds (those below it).
    pub composite_slices: [f32; 4],
}

impl LayingUniforms {
    /// The texture laid on the scene's own grid and drifting by `wind` (m, the scene's axes): one anchor whose chart is
    /// the scene's. The port's sky and every scene with no simulated field draw so, as they did before the anchors.
    pub fn flat(camera: [f64; 2], wind: [f64; 2]) -> Self {
        let value = [camera[0] - wind[0], camera[1] - wind[1]];
        let mut u = Self {
            anchors: [1.0, 0.0, 0.0, 0.0],
            noise_jacobian: [1.0, 0.0, 0.0, 1.0],
            noise_origin: [
                value[0].rem_euclid(NOISE_PERIOD_M) as f32,
                value[1].rem_euclid(NOISE_PERIOD_M) as f32,
                0.0,
                0.0,
            ],
            ..Self::default()
        };
        u.anchor_values[0] = [
            value[0].rem_euclid(WEATHER_TILE_M) as f32,
            value[1].rem_euclid(WEATHER_TILE_M) as f32,
            0.0,
            0.0,
        ];
        u.anchor_jacobians[0] = [1.0, 0.0, 0.0, 1.0];
        u
    }
}

/// What lays a simulated field's weather texture on its cloud anchors about the camera (the game's is
/// `naval_clouds_sim::SimClouds`). The sky asks it every frame it prepares, with the render camera, so the laying
/// follows the camera between the field's updates.
pub trait Placement: std::any::Any + Send {
    /// Lays the field about the camera at the scene metres `camera` (its render origin's) into `laying`, and gives the
    /// field's cloud layers; none when it has no field now, and the sky lays the texture flat ([`Laying::flat`]).
    fn lay(&mut self, laying: &mut Laying, camera: [f64; 2]) -> Option<Layers>;
}

/// The weather texture's laying about the camera, kept from frame to frame (see the module's notes): the flat laying's
/// or what a [`Placement`] wrote.
#[derive(Clone, Debug, Default)]
pub struct Laying {
    /// Whether the last update was a placement's (none: the flat laying).
    pub anchored: bool,
    /// What the shader reads.
    pub uniforms: LayingUniforms,
    /// The weights map's texels, each four weights in the anchors' order, when it was laid again since it was last
    /// taken ([`Laying::take_weights`]).
    pub weights: Option<Vec<[f32; 4]>>,
    /// The last update re-laid the texture: the scene laid on another chart, other anchors drawn or another noise
    /// anchor. Every history drops.
    pub relaid: bool,
    /// The clouds' drift in the scene (m, the scene's axes): what the ground map is laid out by and the reconstruction
    /// follows a drifting cloud with.
    pub wind_offset: [f64; 2],
    /// The slices to lay this frame (first, count, whether a laying begins with them), for the layer's weather pass
    /// ([`Laying::take_composite_slices`]): a placement joins a frame's to any still due.
    pub slices_due: Option<(usize, usize, bool)>,
    /// How far the shapes have evolved (takram's `localWeatherSpeed × 2e4`, m): the uniform drift's length.
    pub evolution: f64,
    /// The layer's weather pass was rebuilt ([`Laying::relay_composite`]): the placement lays the composites whole at
    /// its next update, and clears this.
    pub relay: bool,
}

impl Laying {
    /// The flat laying ([`LayingUniforms::flat`]) about the camera at `camera`, drifting by the sky's `wind` (m, the
    /// scene's axes).
    pub fn flat(&mut self, camera: [f64; 2], wind: [f64; 2]) {
        self.relaid = self.anchored;
        self.anchored = false;
        self.slices_due = None;
        self.uniforms = LayingUniforms::flat(camera, wind);
        self.wind_offset = wind;
        self.evolution = wind[0].hypot(wind[1]);
    }

    /// The slices of the composites' rows to lay this frame, `(first, count, begins)`, once: the layer's weather pass
    /// lays them for the frame their laying began, keeping that frame's uniforms when `begins`.
    pub fn take_composite_slices(&mut self) -> Option<(usize, usize, bool)> {
        self.slices_due.take()
    }

    /// Lay the composites whole again at the next update (the layer's weather pass was rebuilt).
    pub fn relay_composite(&mut self) {
        self.relay = true;
    }

    /// The weights map's texels, when it was laid again since they were last taken: [`WEIGHTS_SIZE`]² of them, rows
    /// along the scene's z, each the anchors' weights in order.
    pub fn take_weights(&mut self) -> Option<Vec<[f32; 4]>> {
        self.weights.take()
    }
}

/// `x` (0 to 1) as a half float's bits: the weights map is `Rgba16Float`.
pub fn half_bits(x: f32) -> u16 {
    let bits = x.max(0.0).to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        return 0;
    }
    if exponent >= 31 {
        return 0x7bff;
    }
    let mantissa = (bits >> 13) & 0x3ff;
    // Round to nearest, carrying into the exponent.
    let round = (bits >> 12) & 1;
    ((exponent as u32) << 10 | mantissa) as u16 + round as u16
}
