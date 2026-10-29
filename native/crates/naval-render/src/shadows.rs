//! The sun's shadow maps.
//!
//! Ports three pieces of the web client:
//!
//! - **Fitting**, from the web game's `FocusShadowNode.ts`: a *near* map fitted each frame
//!   around where the camera looks (centimetre texels in close-ups), or around the hull the
//!   camera follows when she fills less of the frame, its texel grid moving with her (aboard her, the part of her the
//!   view sees); the *wide*
//!   map the light itself describes (a 380 m square around the player's ship at sea,
//!   `Game.fitSunShadow`), on a texel grid fixed in the world; and two *view* maps fitted around
//!   the ships in view beyond the wide map (bands to 1.5 km and 6 km), both redrawn every frame; and,
//!   with land in reach on High and Ultra, three *land* maps fitted to slices of the camera's view of the
//!   land out to 4 km (cascades), so buildings, trees and the land shade what the camera sees of them
//!   however far it is from the followed hull.
//! - **Caster passes**, from the web game's `ShadowCasterPass.ts`: depth-only passes,
//!   both faces, reversed depth cleared to 0 and tested `GreaterEqual`, no raster bias.
//!   Layers draw their casters in [`crate::SceneLayer::draw_shadow`].
//! - **Sampling**, from three r185's `ShadowNode`, with the client's own soft filter (a cubic
//!   B-spline over 6×6 texels, where the web had three's PCFSoft): the `naval::shadow` WGSL
//!   module, whose bindings sit in group 0 beside the camera.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3, Vec4};

use crate::camera::{Camera, CameraUniforms, FrameTime, reversed_orthographic};

/// The shadow maps' depth format: reversed floating-point depth, as the scene's.
pub const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Smallest half-width of the near map, in meters.
pub const NEAR_SHADOW_MIN: f32 = 12.0;
/// How far the near map's shadow camera stands back along the light, so tall masts and a low sun
/// still cast into it.
const NEAR_STANDOFF: f32 = 800.0;
/// How far the near map reaches past its centre along the light, beyond its half-width: under a low
/// sun a hull's shadow lies far across the sea, and all of it comes from the near map.
const NEAR_BEHIND: f32 = 1200.0;
/// Room about the followed hull's sphere when the near map closes on her (m).
const SUBJECT_MARGIN: f32 = 2.0;
/// How far outside the followed hull's box the eye may stand and still be aboard her ([`Sun::hull`]; m): at a bridge
/// wing's rail or on her highest platform.
pub const ABOARD_MARGIN: f32 = 1.0;
/// Aboard her, the halvings that find where along the view the near map hands her to the aboard map
/// ([`aboard_parts`]): a 300 m hull to about a centimetre.
const ABOARD_SPLIT_STEPS: usize = 16;
/// Aboard her, how much finer the near map is than the aboard map, in quarter octaves: one octave, its texels half
/// as wide.
const ABOARD_NEAR_FINER: i32 = 4;
/// Aboard her, the share of the near map's depth along the view over which it hands her to the aboard map
/// (`seam` in `shaders/naval/shadow.wgsl`): her shadows soften gradually across that band, with no line where they
/// change.
const ABOARD_BLEND: f32 = 0.3;
/// The near map leaves a quarter-octave step only once the frame is this far (in quarter octaves)
/// past the halfway point to the next, so a camera resting at a boundary does not flip the map's
/// texel size from one frame to the next.
const NEAR_STEP_HYSTERESIS: f32 = 0.2;
/// The near map covers at most a few hundred meters, so 2048 texels already reach centimetres.
const NEAR_MAP_MAX: u32 = 2048;
/// Camera distances (m, at 1x magnification) the view maps cover beyond the wide map.
pub const VIEW_SHADOW_BANDS: [f32; 2] = [1500.0, 6000.0];
const VIEW_MAP_MAX: u32 = 2048;
/// Smallest map setting that draws view maps: every setting that draws shadows. Low's view maps are 1024 texels, the
/// near map's size, so they take no memory the near map's array did not already hold (its three layers are one size).
const VIEW_MIN_SETTING: u32 = 1024;
const VIEW_MIN_HALF: f32 = 60.0;
const VIEW_MARGIN: f32 = 8.0;
const VIEW_ABOVE: f32 = 100.0;
const VIEW_BELOW: f32 = 1200.0;
/// Share of a square map's half-width that samples it alone (also in `shadow.wgsl`).
const EDGE_START: f32 = 0.9;
/// Receiver offsets of the near map, in texels.
const NEAR_NORMAL_BIAS_TEXELS: f32 = 1.5;
const NEAR_DEPTH_BIAS_TEXELS: f32 = 2.0;
/// Receiver offsets of the view maps, in texels.
const VIEW_NORMAL_BIAS_TEXELS: f32 = 0.75;
const VIEW_DEPTH_BIAS_TEXELS: f32 = 2.0;
/// Half-width of the wide map at sea (`Game.ts` `BATTLE_SHADOW_HALF`).
pub const BATTLE_SHADOW_HALF: f32 = 380.0;
/// The wide map's depth range (`Game.fitSunShadow`).
const WIDE_NEAR: f32 = 1.0;
const WIDE_FAR: f32 = 1800.0;
/// The sun's own depth bias (`Game.ts`: `sunLight.shadow.bias = -0.0005`), over the depth range above: 0.9 m along the
/// light, which a longer box keeps ([`depth_reach`]).
const WIDE_BIAS: f32 = -0.0005;
/// Room above and below the sea's footprint of a square map (m): masts, aircraft low over the water, the land by the
/// shore.
const SQUARE_DEPTH_MARGIN: f32 = 100.0;
/// The most a square map's box reaches along the light either side of its centre for its footprint on the sea (m): a
/// sun on the horizon stretches that footprint without bound.
const SQUARE_DEPTH_MAX: f32 = 6000.0;
/// How far the sun stands from its target along the light at sea (`VisualEnvironment`: 500, 800 in port).
pub const SUN_STANDOFF_AT_SEA: f32 = 500.0;

/// Index of each map in the uniform block and in [`ShadowFit::maps`].
pub const NEAR: usize = 0;
pub const WIDE: usize = 1;
/// The aboard map: the part of the hull the camera stands aboard beyond the near map ([`Sun::hull`]).
pub const ABOARD: usize = 2;
pub const VIEW_FIRST: usize = 3;
/// The first land map (cascade); the land maps follow it.
pub const LAND_FIRST: usize = 5;
/// How many land maps there are.
pub const LAND_CASCADES: usize = 3;
pub const MAP_COUNT: usize = LAND_FIRST + LAND_CASCADES;

/// How far along the camera's view (m, at 1x magnification) the land maps reach: past it a house's shadow is about a
/// pixel at 1080p through the game's lens.
pub const LAND_REACH: f32 = 4000.0;
/// The farthest the land maps reach through a narrow lens (binoculars stretch [`LAND_REACH`] by their magnification).
const LAND_REACH_MAX: f32 = 24000.0;
/// The land maps' slices start no nearer the camera than this (m along its view).
const LAND_NEAREST: f32 = 10.0;
/// The share of each split between the land maps' slices taken geometrically (the rest evenly): a geometric split alone
/// gives the nearest slice too little depth when the view starts close.
const LAND_SPLIT_GEOMETRIC: f32 = 0.75;
/// The land maps stop at 2048 texels, as the near and view maps do (they are layers of the same array).
const LAND_MAP_MAX: u32 = 2048;
/// Room above the land's highest point for what stands on it (m): rooftops and crowns, the receivers the land maps hold.
const LAND_RECEIVER_TOP: f32 = 80.0;
/// Room below the sea (m): the sea's troughs and the shore receive too.
const LAND_RECEIVER_BOTTOM: f32 = -10.0;
/// Room along the light beyond the land's slab (m), toward the light for casters standing above it (towers) and past it.
const LAND_DEPTH_MARGIN: f32 = 300.0;
/// The most a land map's box reaches along the light (m): a sun on the horizon lays the slab's footprint out without end.
const LAND_DEPTH_MAX: f32 = 30000.0;
/// Receiver offsets of the land maps, in texels: kept small, since a house's shadow is only metres long and the
/// coarsest map's texel is a few metres (the land's own caster stands back by itself: native/docs/terrain/shadows.md).
const LAND_NORMAL_BIAS_TEXELS: f32 = 1.0;
const LAND_DEPTH_BIAS_TEXELS: f32 = 1.5;
/// A light this near the horizon (the sine of its elevation) draws no land maps: the land's footprint along it is
/// unbounded, and the land's caster selection takes none either.
const LAND_LOWEST_SUN: f32 = 0.02;

/// The graphics settings the maps follow (`GraphicsController.applyShadows`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowSettings {
    /// Map edge in texels: 0 (off), 1024 (Low), 2048 (Medium), 4096 (High, Ultra), as
    /// `shadowMapSize` in `src/game/settings/graphicsSettings.ts`. The near and view maps stop at 2048: the
    /// view maps draw at 1024 on Low, 2048 from Medium up.
    pub map_size: u32,
    /// The soft filter (a cubic B-spline over 6×6 texels); off, one bilinear comparison.
    pub soft: bool,
    /// Draw the view maps (`FocusShadowNode.viewShadows`).
    pub view_maps: bool,
    /// Half-width of the wide map (`Game.fitSunShadow`: 380 m at sea).
    pub wide_half_width: f32,
    /// three's `shadow.intensity`: 1 for full shadows, 0 for none.
    pub intensity: f32,
    /// The land maps (cascades fitted to the camera's view of the land, [`LAND_CASCADES`] of them): High and Ultra.
    /// Off, the near and view maps' array keeps its three layers and nothing changes.
    pub land_maps: bool,
    /// Contact shadows ([`crate::contact`]): the sun's shadow from the parts within a metre above a surface, which
    /// the maps' texels and offsets leave out, marched through the depth of the layers that take them. On wherever
    /// the maps are.
    pub contact: bool,
    /// The most point lights that cast shadows (searchlights and star shells, the brightest at the camera:
    /// [`crate::lights::light_shadow_maps`]), each a map in the near map's array at its size: none while the maps are
    /// off, 1 on Low, 4 on Medium (the High preset's), 6 on High, 8 on Ultra.
    pub light_maps: u32,
}

impl Default for ShadowSettings {
    /// Off, so a renderer that never asks for shadows draws none.
    fn default() -> Self {
        Self::with_map_size(0)
    }
}

impl ShadowSettings {
    /// The game's settings for a map size (0 turns shadows off).
    pub fn with_map_size(map_size: u32) -> Self {
        Self {
            map_size,
            soft: true,
            view_maps: true,
            wide_half_width: BATTLE_SHADOW_HALF,
            intensity: 1.0,
            land_maps: false,
            contact: true,
            light_maps: match map_size {
                0 => 0,
                1..=1024 => 1,
                1025..=2048 => 4,
                _ => 6,
            },
        }
    }

    /// `shadowMapSize(quality)` for the Graphics menu's names: off, low, medium, high, ultra.
    pub fn for_quality(quality: &str) -> Option<Self> {
        let size = match quality {
            "off" => 0,
            "low" => 1024,
            "medium" => 2048,
            "high" | "ultra" => 4096,
            _ => return None,
        };
        let mut settings = Self::with_map_size(size);
        if quality == "ultra" {
            settings.light_maps = crate::lights::MAX_SHADOWED_LIGHTS as u32;
        }
        Some(settings)
    }

    pub fn enabled(&self) -> bool {
        self.map_size > 0 && self.intensity > 0.0
    }

    /// Whether contact shadows draw: asked for, with the maps on.
    pub fn contact_on(&self) -> bool {
        self.contact && self.enabled()
    }

    fn near_size(&self) -> u32 {
        self.map_size.clamp(1, NEAR_MAP_MAX)
    }

    fn view_size(&self) -> u32 {
        if self.map_size >= VIEW_MIN_SETTING {
            self.map_size.min(VIEW_MAP_MAX)
        } else {
            1
        }
    }

    /// The land maps' size: the setting's, at most [`LAND_MAP_MAX`]; 1 with them off.
    fn land_size(&self) -> u32 {
        if self.land_maps && self.enabled() {
            self.map_size.min(LAND_MAP_MAX)
        } else {
            1
        }
    }

    /// The lights' maps that may draw: [`ShadowSettings::light_maps`] while the maps are on.
    pub fn light_map_count(&self) -> usize {
        if self.enabled() {
            (self.light_maps as usize).min(crate::lights::MAX_SHADOWED_LIGHTS)
        } else {
            0
        }
    }

    /// The lights' maps' size: the near map's array's, whose layers they are.
    pub fn light_map_size(&self) -> u32 {
        self.near_size().max(self.view_size()).max(self.land_size())
    }

    /// Layers of the near, aboard, view and land maps' array: the near, aboard and two view maps, and the land maps when
    /// on (map `i` past the near map is layer `i` − 1). The lights' maps follow them ([`ShadowTargets::ensure`]).
    pub fn cascade_layers(&self) -> u32 {
        if self.land_maps && self.enabled() {
            (MAP_COUNT - 1) as u32
        } else {
            (LAND_FIRST - 1) as u32
        }
    }
}

/// What the host knows of the land about the camera, for the land maps ([`Sun::land`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LandView {
    /// The highest land within the land maps' reach of the camera (m above the sea).
    pub top: f32,
    /// How far the nearest land above the sea lies from the camera, across the ground (m).
    pub nearest: f32,
}

/// A caster's bounding sphere, for fitting the view maps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CasterSphere {
    pub center: Vec3,
    pub radius: f32,
}

/// A hull as a box in her own frame (+X starboard, +Y up from her waterline, −Z toward her bow), where she stands as
/// drawn this frame ([`Sun::hull`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullBox {
    /// Her frame to the world: a turn and a move.
    pub matrix: Mat4,
    pub low: Vec3,
    pub high: Vec3,
}

impl HullBox {
    /// Whether world point `point` lies inside the box grown by `margin` metres every way.
    pub fn holds(&self, point: Vec3, margin: f32) -> bool {
        let local = self.matrix.inverse().transform_point3(point);
        let margin = Vec3::splat(margin);
        local.cmpge(self.low - margin).all() && local.cmple(self.high + margin).all()
    }

    /// Its corners in the world, corner `k` on the high side along x, y and z where bits 1, 2 and 4 of `k` are set.
    fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|k| {
            let pick = |bit: usize, low: f32, high: f32| if k & bit == 0 { low } else { high };
            self.matrix.transform_point3(Vec3::new(
                pick(1, self.low.x, self.high.x),
                pick(2, self.low.y, self.high.y),
                pick(4, self.low.z, self.high.z),
            ))
        })
    }
}

/// The light and what it follows this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Sun {
    /// Unit vector toward the light (the sun, or the moon when it is the brighter).
    pub toward_light: Vec3,
    /// What the wide map is centred on and the near map's focus distance follows: the
    /// player's ship, or the distant ship under binoculars.
    pub target: Vec3,
    /// How far the light stands from `target` along `toward_light` ([`SUN_STANDOFF_AT_SEA`]), at least: under a low
    /// sun the wide map's box stands farther back, so all of its square's footprint on the sea lies inside it.
    pub distance: f32,
    /// Bounds of the ships in view, for the view maps.
    pub casters: Vec<CasterSphere>,
    /// Optical magnification: binoculars stretch every view band by it.
    pub lens: f32,
    /// The hull the camera follows (or examines), as drawn this frame: her origin and a sphere about it holding her
    /// whole model. The near map's texel grid moves with her, so a hull under way casts the same texels every frame
    /// rather than crawling across a grid fixed in the world, and when she fills less than the frame the near map
    /// closes on her alone.
    pub subject: Option<CasterSphere>,
    /// The subject's hull as a box. With the camera's eye inside it (aboard her: a walk, her bridge view) the near map
    /// holds the part of her box the view sees, from the eye to her far end, rather than the frame at her origin's
    /// distance, past which her own forecastle seen from her bridge fell to the wide map's coarse texels.
    pub hull: Option<HullBox>,
    /// The land about the camera, when there is any within the land maps' reach ([`LAND_REACH`] × `lens`): the land
    /// maps are fitted to the camera's view of it ([`ShadowSettings::land_maps`]). `None` draws none.
    pub land: Option<LandView>,
}

impl Sun {
    /// A sun toward `toward_light` over `target`, at sea, with no view-map casters.
    pub fn new(toward_light: Vec3, target: Vec3) -> Self {
        Self {
            toward_light: toward_light.normalize_or(Vec3::Y),
            target,
            distance: SUN_STANDOFF_AT_SEA,
            casters: Vec::new(),
            lens: 1.0,
            subject: None,
            hull: None,
            land: None,
        }
    }
}

/// Which of the four maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowMapKind {
    Near,
    Wide,
    /// What the view sees of the hull the camera stands aboard beyond the near map's part ([`aboard_parts`],
    /// [`Sun::hull`]); off unless aboard.
    Aboard,
    /// View band 0 (to 1.5 km) or 1 (to 6 km).
    View(usize),
    /// Land map (cascade) 0, 1 or 2, nearest the camera first: the camera's view of the land in slices out to
    /// [`LAND_REACH`]. The land layers (the ground, trees, buildings) cast into them; the hulls, aircraft and effects do
    /// not (their own maps shade them): a layer's `draw_shadow` skips them with [`ShadowMapKind::is_land`].
    Land(usize),
    /// A point light's map (`map` of [`crate::lights::MAX_SHADOWED_LIGHTS`]), for the light `light` in the frame's list
    /// ([`crate::lights::light_shadow_maps`]): a searchlight's along its beam, a star shell's looking down. Drawn by
    /// [`crate::SceneLayer::draw_light_shadow`], never `draw_shadow`.
    Light {
        map: usize,
        light: usize,
    },
}

impl ShadowMapKind {
    pub fn index(self) -> usize {
        match self {
            ShadowMapKind::Near => NEAR,
            ShadowMapKind::Wide => WIDE,
            ShadowMapKind::Aboard => ABOARD,
            ShadowMapKind::View(band) => VIEW_FIRST + band,
            ShadowMapKind::Land(cascade) => LAND_FIRST + cascade,
            ShadowMapKind::Light { map, .. } => MAP_COUNT + map,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ShadowMapKind::Near => "near",
            ShadowMapKind::Wide => "wide",
            ShadowMapKind::Aboard => "aboard",
            ShadowMapKind::View(0) => "view 1",
            ShadowMapKind::View(_) => "view 2",
            ShadowMapKind::Land(0) => "land 1",
            ShadowMapKind::Land(1) => "land 2",
            ShadowMapKind::Land(_) => "land 3",
            ShadowMapKind::Light { .. } => "light",
        }
    }

    /// One of the land maps, which only the land's layers cast into.
    pub fn is_land(self) -> bool {
        matches!(self, ShadowMapKind::Land(_))
    }

    /// A point light's map.
    pub fn is_light(self) -> bool {
        matches!(self, ShadowMapKind::Light { .. })
    }
}

/// One map's shadow camera this frame. A layer's `draw_shadow` gets it, to cull casters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowMap {
    pub kind: ShadowMapKind,
    /// Texels per edge.
    pub size: u32,
    /// Sampled this frame.
    pub on: bool,
    /// Its casters are drawn this frame.
    pub drawn: bool,
    /// Where the shadow camera stands, and what it looks at.
    pub position: Vec3,
    pub target: Vec3,
    /// Half extents across the light, in meters.
    pub half_width: f32,
    pub half_height: f32,
    pub near: f32,
    pub far: f32,
    /// Receiver offset along the surface normal, in meters.
    pub normal_bias: f32,
    /// three's `shadow.bias`, in depth units (negative moves the receiver toward the light).
    pub bias: f32,
    /// A view map's hulls: how far along the light from its camera the spheres it was fitted to begin and end (m).
    /// The sea takes from a view map only the shadows of what lies there (`water_sun_shadow`), so the land drawn into it
    /// (clamped onto its near plane when it stands sunward) shades a distant hull but not the water in her square,
    /// where its shadow would end at the square's edges. `None` for the near and wide maps: the sea takes all of theirs.
    pub hull_depths: Option<(f32, f32)>,
    pub view: Mat4,
    pub projection: Mat4,
}

impl ShadowMap {
    fn idle(kind: ShadowMapKind) -> Self {
        Self {
            kind,
            size: 1,
            on: false,
            drawn: false,
            position: Vec3::Y,
            target: Vec3::ZERO,
            half_width: 1.0,
            half_height: 1.0,
            near: 1.0,
            far: 2.0,
            normal_bias: 0.0,
            bias: 0.0,
            hull_depths: None,
            view: Mat4::IDENTITY,
            projection: Mat4::IDENTITY,
        }
    }

    /// Reversed depth (1 at the near plane, 0 at the far) of a point `distance` m from the camera along the light.
    fn depth_at(&self, distance: f32) -> f32 {
        (self.far - distance) / (self.far - self.near)
    }

    /// World to shadow-map coordinates: u and v in 0..1 (v down, as WebGPU textures are
    /// addressed) and reversed depth (1 at the near plane). three's `shadow.matrix` with
    /// `shadowCoord.y.oneMinus()` folded in.
    pub fn matrix(&self) -> Mat4 {
        let to_uv = Mat4::from_cols_array(&[
            0.5, 0.0, 0.0, 0.0, //
            0.0, -0.5, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.5, 0.5, 0.0, 1.0,
        ]);
        to_uv * self.projection * self.view
    }

    /// The block a caster pass binds at group 0 (`naval::camera::Camera`).
    pub fn camera_uniforms(&self, time: FrameTime) -> CameraUniforms {
        CameraUniforms::from_matrices(
            self.view,
            self.projection,
            self.position,
            (self.size, self.size),
            (self.near, self.far),
            time,
        )
    }

    fn place(
        &mut self,
        position: Vec3,
        target: Vec3,
        half: (f32, f32),
        (near, far): (f32, f32),
        orientation: Mat3,
    ) {
        self.position = position;
        self.target = target;
        self.half_width = half.0;
        self.half_height = half.1;
        self.near = near;
        self.far = far;
        self.view = view_matrix(orientation, position);
        self.projection = reversed_orthographic(-half.0, half.0, -half.1, half.1, near, far);
    }
}

/// three's `Matrix4.lookAt(eye, target, up)` rotation for a light at `toward_light` from its
/// target: columns x, y and z, z toward the light. Straight overhead it nudges z as three does.
pub fn light_orientation(toward_light: Vec3) -> Mat3 {
    let up = Vec3::Y;
    let mut z = toward_light.normalize_or(Vec3::Z);
    let mut x = up.cross(z);
    if x.length_squared() == 0.0 {
        z.z += 0.0001;
        z = z.normalize();
        x = up.cross(z);
    }
    let x = x.normalize();
    let y = z.cross(x);
    Mat3::from_cols(x, y, z)
}

fn view_matrix(orientation: Mat3, position: Vec3) -> Mat4 {
    let rotation = Mat4::from_mat3(orientation.transpose());
    rotation * Mat4::from_translation(-position)
}

/// `nearShadowFocus`: where the camera looks at the subject's distance, and the half-width that
/// covers the visible frame there, in quarter-octave steps between [`NEAR_SHADOW_MIN`] and
/// `max_radius`.
pub fn near_shadow_focus(
    camera: &Camera,
    aspect: f32,
    subject: Vec3,
    max_radius: f32,
) -> (Vec3, f32) {
    let (focus, radius) = near_frame(camera, aspect, subject);
    let stepped = 2f32.powf((radius.log2() * 4.0).round() / 4.0);
    (
        focus,
        stepped.clamp(NEAR_SHADOW_MIN, max_radius.max(NEAR_SHADOW_MIN)),
    )
}

/// Where the camera looks at the subject's distance, and the half-width of the visible frame there (unstepped).
fn near_frame(camera: &Camera, aspect: f32, subject: Vec3) -> (Vec3, f32) {
    let distance = camera.eye.distance(subject).max(1.0);
    let forward = (camera.target - camera.eye).normalize_or(Vec3::NEG_Z);
    let focus = camera.eye + forward * distance;
    let tan_v = (camera.fov_y_degrees.to_radians() / 2.0).tan();
    (focus, distance * tan_v.max(tan_v * aspect) * 1.1)
}

/// The quarter-octave step (`4 log2` of a half-width) for `radius`, holding `previous` until the radius has moved
/// [`NEAR_STEP_HYSTERESIS`] past the halfway point to the next step.
fn hold_step(radius: f32, previous: Option<i32>) -> i32 {
    let exact = radius.max(1e-3).log2() * 4.0;
    match previous {
        Some(step) if (exact - step as f32).abs() < 0.5 + NEAR_STEP_HYSTERESIS => step,
        _ => exact.round() as i32,
    }
}

/// The quarter-octave step (`4 log2` of a half-width) at least `needed`: up at once when it no longer holds, down only
/// once it is more than [`NEAR_STEP_HYSTERESIS`] past a whole step too large, so a land map holds its texel while the
/// camera wanders.
fn hold_step_up(needed: f32, previous: Option<i32>) -> i32 {
    let exact = needed.max(1e-3).log2() * 4.0;
    let up = exact.ceil() as i32;
    match previous {
        Some(step) if step >= up && (step as f32 - exact) < 1.0 + NEAR_STEP_HYSTERESIS => step,
        _ => up,
    }
}

/// How far the land maps reach along the camera's view through a lens magnifying `lens` times (m): [`LAND_REACH`]
/// stretched by it, at most [`LAND_REACH_MAX`]. A host looks for land about the camera this far.
pub fn land_reach(lens: f32) -> f32 {
    (LAND_REACH * lens.max(1.0)).min(LAND_REACH_MAX)
}

/// The camera's view of the land cut into the land maps' slices: for each, the corners of its part between the sea and
/// the land's top (`LAND_RECEIVER_BOTTOM` to `land.top + LAND_RECEIVER_TOP`) and where its edges cross that slab's
/// faces, in world metres. The slices run from where the view can first meet the land (no nearer than the nearest land
/// across the ground, nor than the camera's height above the land's top) to [`LAND_REACH`] × `lens`, split between
/// geometrically and evenly ([`LAND_SPLIT_GEOMETRIC`]); the depth they span is first narrowed to the slab's. Empty
/// slices hold no land.
fn land_slices(camera: &Camera, aspect: f32, lens: f32, land: LandView) -> Vec<Vec<Vec3>> {
    let (bottom, top) = (LAND_RECEIVER_BOTTOM, land.top.max(0.0) + LAND_RECEIVER_TOP);
    let world = camera.view().inverse();
    let (right, up) = (world.x_axis.truncate(), world.y_axis.truncate());
    let forward = -world.z_axis.truncate();
    let tan_v = (camera.fov_y_degrees.to_radians() / 2.0).tan();
    let tan_h = tan_v * aspect;
    // A point at distance d across the ground lies at least d cos(the frame's half-diagonal) along the view.
    let diagonal = 1.0 / (1.0 + tan_v * tan_v + tan_h * tan_h).sqrt();
    let reach = land_reach(lens);
    let start = LAND_NEAREST
        .max(camera.near)
        .max(land.nearest * diagonal)
        .max((camera.eye.y - top) * diagonal);
    if start >= reach {
        return Vec::new();
    }
    // The frustum between depths `near` and `far`, clipped to the slab: its corners inside the slab, and where its
    // twelve edges cross the slab's faces.
    let clipped = |near: f32, far: f32| -> Vec<Vec3> {
        let corner = |k: usize| -> Vec3 {
            let t = if k & 4 == 0 { near } else { far };
            let x = if k & 1 == 0 { -tan_h } else { tan_h };
            let y = if k & 2 == 0 { -tan_v } else { tan_v };
            camera.eye + (forward + right * x + up * y) * t
        };
        let corners: [Vec3; 8] = std::array::from_fn(corner);
        let mut points: Vec<Vec3> = corners
            .iter()
            .copied()
            .filter(|c| (bottom..=top).contains(&c.y))
            .collect();
        for a in 0..8 {
            for bit in [1, 2, 4] {
                let b = a | bit;
                if b == a {
                    continue;
                }
                let (p, q) = (corners[a], corners[b]);
                for height in [bottom, top] {
                    if (p.y - height) * (q.y - height) <= 0.0 && p.y != q.y {
                        points.push(p + (q - p) * ((height - p.y) / (q.y - p.y)));
                    }
                }
            }
        }
        points
    };
    // The depths the slab's part of the view spans, within the reach.
    let whole = clipped(start, reach);
    let depths = whole
        .iter()
        .map(|p| (*p - camera.eye).dot(forward))
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), d| {
            (a.min(d), b.max(d))
        });
    if whole.is_empty() || depths.0 >= depths.1 {
        return Vec::new();
    }
    let (first, last) = (depths.0.max(start), depths.1.min(reach));
    let split = |k: usize| -> f32 {
        let share = k as f32 / LAND_CASCADES as f32;
        let geometric = first * (last / first).powf(share);
        let even = first + (last - first) * share;
        LAND_SPLIT_GEOMETRIC * geometric + (1.0 - LAND_SPLIT_GEOMETRIC) * even
    };
    (0..LAND_CASCADES)
        .map(|k| clipped(split(k), split(k + 1)))
        .collect()
}

/// Places `map` as the near map is placed: a square of half-width `reach` about `focus` (world), its centre snapped to the
/// texel grid laid from `origin` (light space), the light standing far enough back for masts and a low sun, with the
/// near map's receiver offsets.
fn place_square(
    map: &mut ShadowMap,
    size: u32,
    focus: Vec3,
    reach: f32,
    origin: Vec3,
    orientation: Mat3,
) {
    let inverse = orientation.transpose();
    let toward = orientation.z_axis;
    let texel = 2.0 * reach / size as f32;
    let center = orientation * snap_to_grid(inverse * focus, origin, (texel, texel));
    // Its box holds its square's whole footprint on the sea too (`depth_reach`).
    let footprint = depth_reach(toward, reach);
    let standoff = NEAR_STANDOFF.max(footprint);
    map.size = size;
    map.place(
        center + toward * standoff,
        center,
        (reach, reach),
        (1.0, standoff + (reach + NEAR_BEHIND).max(footprint)),
        orientation,
    );
    map.normal_bias = NEAR_NORMAL_BIAS_TEXELS * texel;
    map.bias = -NEAR_DEPTH_BIAS_TEXELS * texel / (map.far - map.near);
    map.on = true;
    map.drawn = true;
}

/// The light-space centre and width across the light (the larger of its two sides) of what the camera's view sees of
/// `hull`'s box between `depths` along the view (m); `None` where that slice misses her. `inverse` turns the world into
/// light space.
fn seen_square(
    camera: &Camera,
    aspect: f32,
    hull: &HullBox,
    inverse: Mat3,
    depths: (f32, f32),
) -> Option<(Vec3, f32)> {
    let mut seen = Bounds::EMPTY;
    for point in view_of_hull(camera, aspect, hull, depths) {
        seen.expand(inverse * point);
    }
    (!seen.is_empty()).then(|| {
        (
            (seen.low + seen.high) / 2.0,
            (seen.high - seen.low).truncate().max_element(),
        )
    })
}

/// The `seam` when the near map holds what it holds whole (no aboard map).
const NO_SEAM: Vec4 = Vec4::new(0.0, 0.0, 0.0, -1.0);

/// The half-width a square needs to hold a part `across` wide (light-space metres) whole, inside its fade.
fn square_needed(across: f32) -> f32 {
    across / 2.0 / EDGE_START + SUBJECT_MARGIN
}

/// What the view sees of the hull the camera stands aboard, in two parts for the near and the aboard map.
#[derive(Clone, Copy, Debug)]
struct AboardParts {
    /// The near part's light-space centre and width across ([`seen_square`]): from the eye to the band's end, or all
    /// the view sees of her when that is too little to cut.
    near: (Vec3, f32),
    /// The aboard part's (from the band's start on), and the band along the view (m from the eye) over which the near
    /// map hands her to the aboard map; `None` where the near map holds all the view sees of her.
    far: Option<((Vec3, f32), (f32, f32))>,
}

/// What the view sees of `hull` (the hull the camera stands aboard) in two parts along it, overlapping across the band
/// where the near map hands her to the aboard map: cut where the near part needs a square [`ABOARD_NEAR_FINER`] finer
/// than the aboard part's, so the near map stays an octave finer through any lens. (A cut fixed 40 m along the view
/// left the near map a sliver of her through the walk's narrow lens, several times finer than the aboard map, and the
/// line where the two met showed across her deck.) `None` where the view misses her.
fn aboard_parts(
    camera: &Camera,
    aspect: f32,
    hull: &HullBox,
    inverse: Mat3,
) -> Option<AboardParts> {
    let forward = -camera.view().inverse().z_axis.truncate();
    let far = hull
        .corners()
        .iter()
        .map(|corner| (*corner - camera.eye).dot(forward))
        .fold(0.0, f32::max);
    let part = |depths| seen_square(camera, aspect, hull, inverse, depths);
    let needed = |depths| part(depths).map_or(0.0, |(_, across)| square_needed(across));
    let finer = 2f32.powf(ABOARD_NEAR_FINER as f32 / 4.0);
    // The last end of the near part found to need no more than its share.
    let (mut low, mut high) = (0.0, far);
    for _ in 0..ABOARD_SPLIT_STEPS {
        let end = (low + high) / 2.0;
        if needed((0.0, end)) * finer <= needed((end * (1.0 - ABOARD_BLEND), f32::INFINITY)) {
            low = end;
        } else {
            high = end;
        }
    }
    let band = (low * (1.0 - ABOARD_BLEND), low);
    // So little of her in view (a steep look through a narrow lens) that no part of it needs a square an octave finer
    // than the rest's, the squares' margins outweighing her: the near map holds it all.
    let cut = part((0.0, band.1)).zip(part((band.0, f32::INFINITY)));
    Some(match cut {
        Some((near, far)) => AboardParts {
            near,
            far: Some((far, band)),
        },
        None => AboardParts {
            near: part((0.0, f32::INFINITY))?,
            far: None,
        },
    })
}

/// Where the camera's view between `depths` along it (m; from its near plane at the least, to the box's farthest corner
/// at the most) meets `hull`'s box, for their intersection's bounds: the ends of each one's edges clipped to the other
/// (the corners of either inside the other, and where an edge of either crosses a face of the other, which are all the
/// intersection's corners). Empty where they miss.
fn view_of_hull(camera: &Camera, aspect: f32, hull: &HullBox, depths: (f32, f32)) -> Vec<Vec3> {
    let world = camera.view().inverse();
    let (right, up) = (world.x_axis.truncate(), world.y_axis.truncate());
    let forward = -world.z_axis.truncate();
    let tan_v = (camera.fov_y_degrees.to_radians() / 2.0).tan();
    let tan_h = tan_v * aspect;
    let boxed = hull.corners();
    let near = depths.0.max(camera.near).max(1e-3);
    let far = boxed
        .iter()
        .map(|corner| (*corner - camera.eye).dot(forward))
        .fold(f32::NEG_INFINITY, f32::max)
        .min(depths.1);
    if far <= near {
        return Vec::new();
    }
    let view: [Vec3; 8] = std::array::from_fn(|k| {
        let t = if k & 4 == 0 { near } else { far };
        let x = if k & 1 == 0 { -tan_h } else { tan_h };
        let y = if k & 2 == 0 { -tan_v } else { tan_v };
        camera.eye + (forward + right * x + up * y) * t
    });
    let mut points = Vec::new();
    for (solid, other) in [(&view, &boxed), (&boxed, &view)] {
        let planes = hexahedron_planes(other);
        for a in 0..8 {
            for bit in [1, 2, 4] {
                if a & bit == 0
                    && let Some((p, q)) = clip_segment(&planes, solid[a], solid[a | bit])
                {
                    points.push(p);
                    points.push(q);
                }
            }
        }
    }
    points
}

/// The inward planes (`normal · p + w ≥ 0` inside) of a convex hexahedron whose corners are indexed as
/// [`HullBox::corners`]'s: a face holds the four corners alike in one bit.
fn hexahedron_planes(corners: &[Vec3; 8]) -> [Vec4; 6] {
    let centre = corners.iter().copied().sum::<Vec3>() / 8.0;
    std::array::from_fn(|f| {
        let (bit, high) = ([1, 2, 4][f / 2], f % 2 == 1);
        let face: Vec<Vec3> = (0..8)
            .filter(|k| (k & bit != 0) == high)
            .map(|k| corners[k])
            .collect();
        // In index order the face's corners run a, a + one bit, a + the other, a + both: its diagonals join the
        // first and last, and the middle two.
        let middle = face.iter().copied().sum::<Vec3>() / 4.0;
        let mut normal = (face[3] - face[0])
            .cross(face[2] - face[1])
            .normalize_or_zero();
        if normal.dot(centre - middle) < 0.0 {
            normal = -normal;
        }
        normal.extend(-normal.dot(middle))
    })
}

/// The part of the segment from `p` to `q` inside every plane (`normal · x + w ≥ 0`), if any.
fn clip_segment(planes: &[Vec4; 6], p: Vec3, q: Vec3) -> Option<(Vec3, Vec3)> {
    let along = q - p;
    let (mut first, mut last) = (0f32, 1f32);
    for plane in planes {
        let normal = plane.truncate();
        let at = normal.dot(p) + plane.w;
        let rate = normal.dot(along);
        if rate.abs() < 1e-9 {
            if at < 0.0 {
                return None;
            }
            continue;
        }
        let t = -at / rate;
        if rate > 0.0 {
            first = first.max(t);
        } else {
            last = last.min(t);
        }
        if first > last {
            return None;
        }
    }
    Some((p + along * first, p + along * last))
}

/// `value` moved to the nearest point `origin + k × texel` along light-space x and y (z kept).
fn snap_to_grid(value: Vec3, origin: Vec3, texel: (f32, f32)) -> Vec3 {
    Vec3::new(
        origin.x + ((value.x - origin.x) / texel.0).round() * texel.0,
        origin.y + ((value.y - origin.y) / texel.1).round() * texel.1,
        value.z,
    )
}

/// How far along the light (m, either side of its centre) a square map `half` across must reach to hold its whole
/// footprint on the sea: under a sun at elevation e the square's sides lie `half / tan(e)` along the light from its
/// centre at sea level. A receiver past a box's near or far plane takes no shadow from it, so a shorter box left part of
/// its square unshaded at a low sun (a hull 600 m sunward of the wide map's target under a 10° sun, and the land's
/// shadow on the sea, which ended there in a straight line).
fn depth_reach(toward: Vec3, half: f32) -> f32 {
    let sine = toward.y;
    if sine <= 0.0 {
        return SQUARE_DEPTH_MAX;
    }
    let cotangent = (1.0 - sine * sine).max(0.0).sqrt() / sine;
    (half * cotangent + SQUARE_DEPTH_MARGIN).min(SQUARE_DEPTH_MAX)
}

/// Rounds a view map's half-width up to a quarter octave, at least [`VIEW_MIN_HALF`].
fn step_up(half: f32) -> f32 {
    2f32.powf((half.max(VIEW_MIN_HALF).log2() * 4.0).ceil() / 4.0)
}

/// Axis-aligned bounds in light space.
#[derive(Clone, Copy, Debug)]
struct Bounds {
    low: Vec3,
    high: Vec3,
}

impl Bounds {
    const EMPTY: Self = Self {
        low: Vec3::splat(f32::INFINITY),
        high: Vec3::splat(f32::NEG_INFINITY),
    };

    fn is_empty(&self) -> bool {
        self.low.x > self.high.x
    }

    fn expand(&mut self, point: Vec3) {
        self.low = self.low.min(point);
        self.high = self.high.max(point);
    }
}

/// The four maps' fitting state (the CPU half of `FocusShadowNode`).
#[derive(Clone, Debug)]
pub struct ShadowFit {
    pub settings: ShadowSettings,
    pub maps: [ShadowMap; MAP_COUNT],
    /// The near map's centre (the camera's focus or the followed hull's origin, unsnapped) and half-width.
    pub focus: Vec3,
    pub reach: f32,
    /// The near map's quarter-octave step when it follows the frame, held against small changes
    /// ([`NEAR_STEP_HYSTERESIS`]). Fitting the same camera again keeps it, so a frame fitted twice
    /// fits the same maps.
    near_step: Option<i32>,
    /// The aboard map's quarter-octave step, held as the near map's is; the near map's is [`ABOARD_NEAR_FINER`] less.
    aboard_step: Option<i32>,
    /// Aboard her, the band along the view over which the near map hands her to the aboard map: `seam · p + w` runs
    /// from 0 to 1 across it ([`ShadowUniforms::seam`]); (0, 0, 0, -1), the near map whole, otherwise.
    seam: Vec4,
    /// Each land map's quarter-octave step, held as the near map's is.
    land_steps: [Option<i32>; LAND_CASCADES],
}

impl Default for ShadowFit {
    fn default() -> Self {
        Self::new(ShadowSettings::default())
    }
}

impl ShadowFit {
    pub fn new(settings: ShadowSettings) -> Self {
        Self {
            settings,
            maps: [
                ShadowMap::idle(ShadowMapKind::Near),
                ShadowMap::idle(ShadowMapKind::Wide),
                ShadowMap::idle(ShadowMapKind::Aboard),
                ShadowMap::idle(ShadowMapKind::View(0)),
                ShadowMap::idle(ShadowMapKind::View(1)),
                ShadowMap::idle(ShadowMapKind::Land(0)),
                ShadowMap::idle(ShadowMapKind::Land(1)),
                ShadowMap::idle(ShadowMapKind::Land(2)),
            ],
            focus: Vec3::ZERO,
            reach: NEAR_SHADOW_MIN,
            near_step: None,
            aboard_step: None,
            seam: NO_SEAM,
            land_steps: [None; LAND_CASCADES],
        }
    }

    /// Fits every map for this frame's camera and sun (`focusOn`, `fitView` and `updateBefore`).
    /// Maps whose casters draw this frame have `drawn` set: every map sampled is redrawn, so a
    /// moving hull's shadow never lags her. Fitting the same camera and sun again fits the same
    /// maps.
    pub fn update(&mut self, camera: &Camera, aspect: f32, sun: Option<&Sun>) {
        for map in &mut self.maps {
            map.drawn = false;
        }
        let settings = self.settings;
        let Some(sun) = sun.filter(|_| settings.enabled()) else {
            for map in &mut self.maps {
                map.on = false;
            }
            return;
        };
        let orientation = light_orientation(sun.toward_light);
        let inverse = orientation.transpose();
        let toward = orientation.z_axis;
        let size = settings.map_size;

        // Wide: the light's own map, its centre snapped to a texel grid fixed in the world, so land
        // shadows hold still while the ship it follows moves.
        let half = settings.wide_half_width;
        let wide_texel = 2.0 * half / size as f32;
        let target =
            orientation * snap_to_grid(inverse * sun.target, Vec3::ZERO, (wide_texel, wide_texel));
        // The light stands `sun.distance` back with its far plane `WIDE_FAR` from it, or farther both ways under a low
        // sun, so the box holds the square's whole footprint on the sea (`depth_reach`).
        let wide_reach = depth_reach(toward, half);
        let standoff = sun.distance.max(wide_reach);
        let far = standoff + (WIDE_FAR - sun.distance).max(wide_reach);
        let wide = &mut self.maps[WIDE];
        wide.size = size;
        wide.place(
            target + toward * standoff,
            target,
            (half, half),
            (WIDE_NEAR, far),
            orientation,
        );
        // GraphicsController.applyShadows: the receiver offset follows the texel in meters.
        wide.normal_bias = 0.75 * wide_texel;
        wide.bias = WIDE_BIAS * (WIDE_FAR - WIDE_NEAR) / (far - WIDE_NEAR);
        wide.on = true;
        wide.drawn = true;

        // Near: a square of half-width `reach` around where the camera looks, or around the followed
        // hull alone when she fills less than the frame (her sphere inside the square's fade). Its texel
        // grid is laid from her origin, so she casts the same texels whatever she and the camera do but
        // turn, pitch and roll (the world's origin when nobody is followed). Aboard her (the eye inside her box), the
        // near map holds what the view sees of her nearer the eye and the aboard map the rest of her, each whole, the
        // near map an octave finer, handing her over across a band along the view: two cascades over her, as a
        // first-person view needs, where one square at her origin's distance left her forecastle seen from her bridge
        // to the wide map.
        let aboard = sun
            .hull
            .filter(|hull| hull.holds(camera.eye, ABOARD_MARGIN))
            .and_then(|hull| aboard_parts(camera, aspect, &hull, inverse));
        let size_of = |step: i32| {
            2f32.powf(step as f32 / 4.0)
                .clamp(NEAR_SHADOW_MIN, half.max(NEAR_SHADOW_MIN))
        };
        // The near and aboard maps' half-widths aboard her.
        let mut sizes = None;
        self.seam = NO_SEAM;
        let previous = self.aboard_step.take();
        match aboard.map(|parts| (parts.near, parts.far)) {
            Some((_, Some(((_, across), (start, end))))) => {
                let step = hold_step_up(square_needed(across), previous);
                self.aboard_step = Some(step);
                self.near_step = Some(step - ABOARD_NEAR_FINER);
                sizes = Some((size_of(step - ABOARD_NEAR_FINER), size_of(step)));
                let forward = -camera.view().inverse().z_axis.truncate();
                self.seam = (forward / (end - start))
                    .extend(-(forward.dot(camera.eye) + start) / (end - start));
            }
            Some(((_, across), None)) => {
                let step = hold_step_up(square_needed(across), self.near_step);
                self.near_step = Some(step);
                sizes = Some((size_of(step), 0.0));
            }
            None => {}
        }
        let (focus, reach) = if let (Some(parts), Some((reach, _))) = (&aboard, sizes) {
            (orientation * parts.near.0, reach)
        } else {
            let (frame_focus, frame_radius) = near_frame(camera, aspect, sun.target);
            let step = hold_step(frame_radius, self.near_step);
            self.near_step = Some(step);
            let frame_reach = 2f32
                .powf(step as f32 / 4.0)
                .clamp(NEAR_SHADOW_MIN, half.max(NEAR_SHADOW_MIN));
            sun.subject
                .map(|subject| {
                    let reach = (subject.radius / EDGE_START + SUBJECT_MARGIN).max(NEAR_SHADOW_MIN);
                    (subject.center, reach)
                })
                .filter(|&(center, reach)| {
                    reach < frame_reach && center.distance(frame_focus) < frame_reach
                })
                .unwrap_or((frame_focus, frame_reach))
        };
        self.focus = focus;
        self.reach = reach;
        let near_size = settings.near_size();
        let origin = inverse * sun.subject.map_or(Vec3::ZERO, |subject| subject.center);
        place_square(
            &mut self.maps[NEAR],
            near_size,
            focus,
            reach,
            origin,
            orientation,
        );

        // Aboard: the rest of her the view sees, from where the near map begins to hand her over.
        let aboard_map = &mut self.maps[ABOARD];
        aboard_map.size = near_size;
        if let (Some(((center, _), _)), Some((_, reach))) =
            (aboard.and_then(|parts| parts.far), sizes)
        {
            place_square(
                aboard_map,
                near_size,
                orientation * center,
                reach,
                origin,
                orientation,
            );
        } else {
            aboard_map.on = false;
        }

        self.fit_views(camera, sun, orientation);
        self.fit_land(camera, aspect, sun, orientation);
    }

    /// The land maps: the camera's view of the land (the frustum between the sea and the land's top, out to
    /// [`LAND_REACH`]) cut into [`LAND_CASCADES`] slices along its depth, each held by a square across the light. A
    /// square's size moves in quarter-octave steps, held against small changes, and its centre is snapped to a texel
    /// grid fixed in the world, so the shadows hold still as the camera moves and turns, re-rasterized only when a
    /// step changes. A slice holding no land, or a land map the settings leave off, samples nothing.
    fn fit_land(&mut self, camera: &Camera, aspect: f32, sun: &Sun, orientation: Mat3) {
        let settings = self.settings;
        let size = settings.land_size();
        let land = sun
            .land
            .filter(|_| settings.land_maps && orientation.z_axis.y > LAND_LOWEST_SUN);
        let slices = land.map(|land| land_slices(camera, aspect, sun.lens, land));
        for cascade in 0..LAND_CASCADES {
            let map = &mut self.maps[LAND_FIRST + cascade];
            map.size = size;
            let Some(points) = slices
                .as_ref()
                .and_then(|slices| slices.get(cascade))
                .filter(|points| !points.is_empty())
            else {
                map.on = false;
                self.land_steps[cascade] = None;
                continue;
            };
            let Some(land) = land else {
                continue;
            };
            let inverse = orientation.transpose();
            let mut bounds = Bounds::EMPTY;
            for &point in points {
                bounds.expand(inverse * point);
            }
            let Bounds { low, high } = bounds;
            // The slice inside the square's fade, with a texel's room for the snap below.
            let needed = (high.x - low.x).max(high.y - low.y) / 2.0 / EDGE_START;
            let step = hold_step_up(needed * (1.0 + 2.0 / size as f32), self.land_steps[cascade]);
            self.land_steps[cascade] = Some(step);
            let half = 2f32.powf(step as f32 / 4.0);
            let texel = 2.0 * half / size as f32;
            let centre = snap_to_grid(
                Vec3::new((low.x + high.x) / 2.0, (low.y + high.y) / 2.0, 0.0),
                Vec3::ZERO,
                (texel, texel),
            );
            // Along the light: where the square's sides cross the slab the receivers stand in. A point (x, y, z) in light
            // space stands at height y Y.y + z Z.y (the light's x is level).
            let (y_up, z_up) = (orientation.y_axis.y, orientation.z_axis.y);
            let (bottom, top) = (LAND_RECEIVER_BOTTOM, land.top.max(0.0) + LAND_RECEIVER_TOP);
            let along = |y: f32, height: f32| (height - y * y_up) / z_up;
            let mut nearest = f32::NEG_INFINITY;
            let mut farthest = f32::INFINITY;
            for y in [centre.y - half, centre.y + half] {
                for height in [bottom, top] {
                    nearest = nearest.max(along(y, height));
                    farthest = farthest.min(along(y, height));
                }
            }
            // Whole metres, so the depths a caster writes do not wander as the square slides.
            let mut light_z = (nearest + LAND_DEPTH_MARGIN).ceil();
            let mut far_end = farthest - LAND_DEPTH_MARGIN;
            if light_z - far_end > LAND_DEPTH_MAX {
                // Under a sun a few degrees up the square's footprint runs tens of kilometres along the light. The box
                // keeps this slice's own receivers (the view's part of the slab, `low.z` its farthest) and gives up
                // first the square's corners beyond them, then its reach toward the sun, where the land's caster is
                // clamped onto the near plane anyway.
                far_end = far_end.max(low.z - LAND_DEPTH_MARGIN);
                light_z = light_z.min((far_end + LAND_DEPTH_MAX).floor());
            }
            let depth = (light_z - far_end).ceil().min(LAND_DEPTH_MAX);
            let position = orientation * Vec3::new(centre.x, centre.y, light_z);
            let target = orientation * Vec3::new(centre.x, centre.y, along(centre.y, top / 2.0));
            map.place(position, target, (half, half), (1.0, depth), orientation);
            map.normal_bias = LAND_NORMAL_BIAS_TEXELS * texel;
            map.bias = -LAND_DEPTH_BIAS_TEXELS * texel / (depth - 1.0);
            map.hull_depths = None;
            map.on = true;
            map.drawn = true;
        }
    }

    /// Whether any land map samples this frame.
    pub fn land_on(&self) -> bool {
        self.maps[LAND_FIRST..].iter().any(|map| map.on)
    }

    /// `fitView` and `place`: the view maps around the ships in view that the wide map misses, each
    /// band redrawn every frame. A band holding one ship is centred on her, so she casts the same
    /// texels as she steams; one holding several is snapped to a grid fixed in the world.
    fn fit_views(&mut self, camera: &Camera, sun: &Sun, orientation: Mat3) {
        let settings = self.settings;
        let inverse = orientation.transpose();
        let mut bounds = [Bounds::EMPTY; 2];
        let mut ships = [0usize; 2];
        let view_size = settings.view_size();
        if settings.view_maps && settings.map_size >= VIEW_MIN_SETTING {
            let wide_center = inverse * sun.target;
            let wide_inner = settings.wide_half_width * EDGE_START;
            for caster in &sun.casters {
                let local = inverse * caster.center;
                let radius = caster.radius;
                if (local.x - wide_center.x).abs() + radius <= wide_inner
                    && (local.y - wide_center.y).abs() + radius <= wide_inner
                {
                    continue;
                }
                let range = (camera.eye.distance(caster.center) - radius) / sun.lens.max(1e-3);
                let Some(band) = VIEW_SHADOW_BANDS.iter().position(|&reach| range <= reach) else {
                    continue;
                };
                bounds[band].expand(local + Vec3::splat(radius));
                bounds[band].expand(local - Vec3::splat(radius));
                ships[band] += 1;
            }
        }
        for band in 0..VIEW_SHADOW_BANDS.len() {
            let map = &mut self.maps[VIEW_FIRST + band];
            map.size = view_size;
            if bounds[band].is_empty() {
                map.on = false;
                continue;
            }
            let Bounds { low, high } = bounds[band];
            let half_x = step_up((high.x - low.x) / 2.0 + VIEW_MARGIN);
            let half_y = step_up((high.y - low.y) / 2.0 + VIEW_MARGIN);
            let size = view_size as f32;
            let (texel_x, texel_y) = (2.0 * half_x / size, 2.0 * half_y / size);
            let middle = Vec3::new(
                (low.x + high.x) / 2.0,
                (low.y + high.y) / 2.0,
                high.z + VIEW_ABOVE,
            );
            let local = if ships[band] == 1 {
                middle
            } else {
                snap_to_grid(middle, Vec3::ZERO, (texel_x, texel_y))
            };
            // The box reaches past the hulls to the sea under its square's far side, which the sea reads it at: in
            // light space the sea is the plane y cos(e) + z sin(e) = 0, so a low sun lays it far along the light (a
            // 30 m mast's shadow under a 1° sun runs 1.7 km, past the 1.2 km the box kept), and a hull's shadow on
            // the water would end there in a straight line.
            let sine = orientation.z_axis.y;
            let sea_far = if sine > 0.0 {
                -(local.y + half_y) * (1.0 - sine * sine).max(0.0).sqrt() / sine
            } else {
                f32::NEG_INFINITY
            };
            let below = (low.z - sea_far + VIEW_MARGIN).clamp(VIEW_BELOW, SQUARE_DEPTH_MAX);
            let depth = VIEW_ABOVE + (high.z - low.z) + below;
            let position = orientation * local;
            let texel = texel_x.max(texel_y);
            map.place(
                position,
                position - orientation.z_axis,
                (half_x, half_y),
                (1.0, depth),
                orientation,
            );
            map.normal_bias = VIEW_NORMAL_BIAS_TEXELS * texel;
            map.bias = -VIEW_DEPTH_BIAS_TEXELS * texel / (depth - 1.0);
            map.hull_depths = Some((
                VIEW_ABOVE - VIEW_MARGIN,
                VIEW_ABOVE + (high.z - low.z) + VIEW_MARGIN,
            ));
            map.on = true;
            map.drawn = true;
        }
    }

    /// The uniform block `naval::shadow` reads.
    pub fn uniforms(&self, toward_light: Vec3) -> ShadowUniforms {
        let mut maps = [ShadowMapUniform::zeroed(); MAP_COUNT];
        for (uniform, map) in maps.iter_mut().zip(&self.maps) {
            let size = map.size.max(1) as f32;
            *uniform = ShadowMapUniform {
                matrix: map.matrix().to_cols_array(),
                // Depth bias as added to the receiver's reversed depth: toward the light is up.
                params: [size, 1.0 / size, map.normal_bias, -map.bias],
                state: [
                    if map.on { 1.0 } else { 0.0 },
                    map.half_width,
                    map.half_height,
                    0.0,
                ],
                hulls: match map.hull_depths {
                    Some((first, last)) => [map.depth_at(first), map.depth_at(last), 1.0, 0.0],
                    None => [0.0; 4],
                },
            };
        }
        let enabled = self.settings.enabled() && self.maps[WIDE].on;
        ShadowUniforms {
            maps,
            focus: [self.focus.x, self.focus.y, self.focus.z, self.reach],
            settings: [
                if enabled {
                    self.settings.intensity
                } else {
                    0.0
                },
                if self.settings.soft { 1.0 } else { 0.0 },
                if enabled { 1.0 } else { 0.0 },
                if enabled && self.land_on() { 1.0 } else { 0.0 },
            ],
            toward_light: [toward_light.x, toward_light.y, toward_light.z, 0.0],
            seam: self.seam.to_array(),
        }
    }
}

/// One map in the uniform block. WGSL twin: `ShadowMap` in `shaders/naval/shadow.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ShadowMapUniform {
    /// World to (u, v, reversed depth): [`ShadowMap::matrix`].
    pub matrix: [f32; 16],
    /// Texels per edge, 1 / that, normal bias (m), depth bias (added to the receiver's depth).
    pub params: [f32; 4],
    /// On (0 or 1), half width and half height (m).
    pub state: [f32; 4],
    /// A view map's hulls ([`ShadowMap::hull_depths`]) in its reversed depth: where they begin (nearer the light) and
    /// end, and 1 when it has them (0 for the near and wide maps).
    pub hulls: [f32; 4],
}

/// The block at group 0 binding 1. WGSL twin: `Shadows` in `shaders/naval/shadow.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ShadowUniforms {
    /// Near, wide, aboard, view 1, view 2, land 1, land 2, land 3.
    pub maps: [ShadowMapUniform; MAP_COUNT],
    /// The near map's focus (xyz) and half-width (w).
    pub focus: [f32; 4],
    /// Intensity (0 when off), soft filter (0 or 1), enabled (0 or 1), any land map on (0 or 1).
    pub settings: [f32; 4],
    /// Toward the light (xyz).
    pub toward_light: [f32; 4],
    /// Aboard a hull, the band along the view over which the near map hands her to the aboard map: `seam · p + w`
    /// (p in world metres) runs from 0 to 1 across it, the near map's share falling from whole to none; (0, 0, 0, -1)
    /// otherwise.
    pub seam: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<ShadowUniforms>() == 960);

/// Texels per edge of the soft occluders' map ([`SunOccluders`]); `SUN_OCCLUDER_SIZE` in `shaders/naval/shadow.wgsl` twins it.
pub const SUN_OCCLUDER_SIZE: u32 = 256;

/// Where the soft occluders' map lies along the sun's light. WGSL twin: `SunOccluders` in `shaders/naval/shadow.wgsl`.
/// Zeroed, the map is off and every receiver is lit by it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct SunOccluderUniform {
    /// World to the map's u and v (0 to 1 across it): `dot(axis.xyz, p) + axis.w`.
    pub axis_u: [f32; 4],
    pub axis_v: [f32; 4],
    /// World to depth along the light (m, away from it): `dot(axis.xyz, p) + axis.w`.
    pub axis_d: [f32; 4],
    /// The depth the texels' fronts count back from (m), the depth their backs count on from (m), unused, on (0 or 1).
    pub params: [f32; 4],
}

/// Soft occluders in the sun's light, which the depth maps cannot hold (smoke): a light-space map of
/// `SUN_OCCLUDER_SIZE`² texels in two planes of half floats, row by row. The first, [`Self::SPAN_BYTES`] at offset 0
/// (an `Rg16Float` target's copy): how far before `params.x` the first occluder begins along the light, and how far
/// after `params.y` the last ends. The second, [`Self::OPTICAL_BYTES`] at [`Self::SPAN_BYTES`] (an `R16Float`
/// target's copy): the optical depth between them, spread evenly. `naval::shadow`'s `sun_shadow` and
/// `water_sun_shadow` multiply in what it lets through, so every receiver takes it without a binding of its own (a
/// storage buffer: no stage spends a sampled texture on it). The renderer zeroes the uniform each frame before the
/// layers prepare; a layer that draws occluders (the combat effects' smoke) copies its targets into `texels` and writes
/// the uniform in its `prepare`.
pub struct SunOccluders {
    pub texels: wgpu::Buffer,
    pub uniforms: wgpu::Buffer,
}

impl SunOccluders {
    /// Bytes of the first plane (four a texel).
    pub const SPAN_BYTES: u64 = (SUN_OCCLUDER_SIZE * SUN_OCCLUDER_SIZE * 4) as u64;
    /// Bytes of the second plane (two a texel).
    pub const OPTICAL_BYTES: u64 = (SUN_OCCLUDER_SIZE * SUN_OCCLUDER_SIZE * 2) as u64;

    fn new(device: &wgpu::Device) -> Self {
        Self {
            texels: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun occluders"),
                size: Self::SPAN_BYTES + Self::OPTICAL_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniforms: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sun occluders"),
                size: std::mem::size_of::<SunOccluderUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        }
    }
}

/// The maps' GPU side: textures, a comparison sampler, the uniform block and the caster passes'
/// cameras.
pub(crate) struct ShadowTargets {
    /// Near, view 1 and view 2 as layers 0, 1 and 2, then the land maps as 3, 4 and 5 when they are on, at the
    /// largest of their sizes. Map index `i` past the near map is layer `i - 1`.
    pub cascades: wgpu::Texture,
    pub cascades_view: wgpu::TextureView,
    cascade_layers: Vec<wgpu::TextureView>,
    /// The wide map, at the setting's size.
    pub wide: wgpu::Texture,
    pub wide_view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub uniforms: wgpu::Buffer,
    casters: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    /// Soft occluders (smoke) at group 0 bindings 8 and 9.
    pub occluders: SunOccluders,
    /// The array's size and layers, and the wide map's size.
    sizes: (u32, u32, u32),
    /// The lights' maps' first layer in the array (after the sun's), and how many it holds.
    light_layers: (u32, u32),
    /// Changes when the textures are remade, so group 0 is rebuilt.
    pub version: u64,
}

impl ShadowTargets {
    pub(crate) fn new(device: &wgpu::Device, caster_layout: &wgpu::BindGroupLayout) -> Self {
        let (cascades, cascades_view, cascade_layers, wide, wide_view) =
            make_textures(device, (1, 3, 1));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("sun shadows"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::GreaterEqual),
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sun shadows"),
            size: std::mem::size_of::<ShadowUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let casters = (0..MAP_COUNT + crate::lights::MAX_SHADOWED_LIGHTS)
            .map(|index| {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("shadow caster camera"),
                    size: std::mem::size_of::<CameraUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(&format!("shadow caster camera {index}")),
                    layout: caster_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                });
                (buffer, group)
            })
            .collect();
        Self {
            cascades,
            cascades_view,
            cascade_layers,
            wide,
            wide_view,
            sampler,
            uniforms,
            casters,
            occluders: SunOccluders::new(device),
            sizes: (1, 3, 1),
            light_layers: (3, 0),
            version: 1,
        }
    }

    /// Remakes the textures when the settings change their sizes, with room after the sun's maps for `lights` light maps
    /// (which, once made, stay until the settings change: a night's star shells come and go without remaking it). Returns
    /// whether they changed.
    pub(crate) fn ensure(
        &mut self,
        device: &wgpu::Device,
        settings: &ShadowSettings,
        lights: usize,
    ) -> bool {
        let sun_layers = settings.cascade_layers();
        let wanted = (lights as u32)
            .max(self.light_layers.1)
            .min(settings.light_map_count() as u32);
        let sizes = if settings.enabled() {
            (
                settings.light_map_size(),
                sun_layers + wanted,
                settings.map_size,
            )
        } else {
            (1, sun_layers, 1)
        };
        // The lights' layers follow the sun's, whose count the land maps change (the array's may stay the same).
        self.light_layers = (sun_layers, sizes.1 - sun_layers);
        if sizes == self.sizes {
            return false;
        }
        let (cascades, cascades_view, cascade_layers, wide, wide_view) =
            make_textures(device, sizes);
        self.cascades = cascades;
        self.cascades_view = cascades_view;
        self.cascade_layers = cascade_layers;
        self.wide = wide;
        self.wide_view = wide_view;
        self.sizes = sizes;
        self.version += 1;
        true
    }

    /// The depth attachment a map's casters draw into.
    pub(crate) fn target(&self, kind: ShadowMapKind) -> &wgpu::TextureView {
        match kind {
            ShadowMapKind::Near => &self.cascade_layers[0],
            ShadowMapKind::Wide => &self.wide_view,
            ShadowMapKind::Light { map, .. } => {
                &self.cascade_layers[(self.light_layers.0 as usize) + map]
            }
            kind => &self.cascade_layers[kind.index() - 1],
        }
    }

    /// The lights' maps' first layer in the array, and how many layers it holds for them.
    pub(crate) fn light_layers(&self) -> (u32, u32) {
        self.light_layers
    }

    /// Group 0 of a map's caster pass, after writing its camera.
    pub(crate) fn caster_group(
        &self,
        uploads: &mut crate::upload::Uploads,
        map: &ShadowMap,
        time: FrameTime,
    ) -> (&wgpu::BindGroup, CameraUniforms) {
        let uniforms = map.camera_uniforms(time);
        let (buffer, group) = &self.casters[map.kind.index()];
        uploads.write_buffer(buffer, 0, bytemuck::bytes_of(&uniforms));
        (group, uniforms)
    }
}

type Textures = (
    wgpu::Texture,
    wgpu::TextureView,
    Vec<wgpu::TextureView>,
    wgpu::Texture,
    wgpu::TextureView,
);

fn make_textures(
    device: &wgpu::Device,
    (cascade_size, layers, wide_size): (u32, u32, u32),
) -> Textures {
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC;
    let cascades = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sun shadows near and view maps"),
        size: wgpu::Extent3d {
            width: cascade_size,
            height: cascade_size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage,
        view_formats: &[],
    });
    let cascades_view = cascades.create_view(&wgpu::TextureViewDescriptor {
        label: Some("sun shadows near and view maps"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let layer_views = (0..layers)
        .map(|layer| {
            cascades.create_view(&wgpu::TextureViewDescriptor {
                label: Some("sun shadow layer"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
        .collect();
    let wide = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sun shadows wide map"),
        size: wgpu::Extent3d {
            width: wide_size,
            height: wide_size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: SHADOW_FORMAT,
        usage,
        view_formats: &[],
    });
    let wide_view = wide.create_view(&wgpu::TextureViewDescriptor::default());
    (cascades, cascades_view, layer_views, wide, wide_view)
}

/// The group 0 layout entries of the soft occluders (bindings 8 and 9: [`SunOccluders`]).
pub(crate) fn occluder_layout_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
    let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
    [
        wgpu::BindGroupLayoutEntry {
            binding: 8,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 9,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(
                    std::mem::size_of::<SunOccluderUniform>() as u64
                ),
            },
            count: None,
        },
    ]
}

/// The group 0 layout entries the shadow maps add (bindings 1 to 4).
pub(crate) fn view_layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
    let visibility = wgpu::ShaderStages::VERTEX_FRAGMENT | wgpu::ShaderStages::COMPUTE;
    [
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(
                    std::mem::size_of::<ShadowUniforms>() as u64
                ),
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2Array,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 3,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 4,
            visibility,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
            count: None,
        },
    ]
}

/// A pipeline drawn into the shadow maps: depth only, both faces, reversed depth, the shadow
/// camera at group 0 (`naval::camera::camera`, holding the map's orthographic matrices) and the
/// layer's own groups after it. Build it with [`crate::RenderContext::add_caster_pipeline`].
#[derive(Clone, Debug)]
pub struct CasterPipeline {
    pub vertex_entry: &'static str,
    pub vertex_buffers: Vec<crate::VertexLayout>,
    /// Groups 1, 2, …; group 0 is the shadow camera.
    pub bind_group_layouts: Vec<wgpu::BindGroupLayout>,
    /// `cull_mode: None` by default: casters draw both faces (kept from the web game; not a product decision).
    pub primitive: wgpu::PrimitiveState,
    pub defs: crate::ShaderDefs,
    /// A fragment stage with no colour targets, for casters that `discard` (an alpha-tested card: a tree's crown);
    /// `None` (the default) draws depth from the vertices alone.
    pub fragment_entry: Option<&'static str>,
}

impl CasterPipeline {
    pub fn new(vertex_entry: &'static str) -> Self {
        Self {
            fragment_entry: None,
            vertex_entry,
            vertex_buffers: Vec::new(),
            bind_group_layouts: Vec::new(),
            primitive: wgpu::PrimitiveState::default(),
            defs: Vec::new(),
        }
    }
}

/// The caster passes' depth state: write, `GreaterEqual`, no raster bias (the receivers are offset).
pub fn caster_depth_state() -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: SHADOW_FORMAT,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }
}

/// What telemetry reports about the maps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadowReport {
    pub enabled: bool,
    pub map_size: u32,
    pub soft: bool,
    /// Kind, texels, on, drawn this frame, half-width (m).
    pub maps: Vec<(ShadowMapKind, u32, bool, bool, f32)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sun() -> Sun {
        Sun::new(Vec3::new(0.5, 0.6, 0.3).normalize(), Vec3::ZERO)
    }

    #[test]
    fn the_orientation_looks_down_the_light_as_three_does() {
        let toward = Vec3::new(0.5, 0.6, 0.3).normalize();
        let r = light_orientation(toward);
        assert!((r.z_axis - toward).length() < 1e-6);
        assert!(r.x_axis.y.abs() < 1e-6, "x stays level: {:?}", r.x_axis);
        assert!((r.x_axis.cross(r.y_axis) - r.z_axis).length() < 1e-5);
        // Overhead, three nudges z toward +Z.
        let r = light_orientation(Vec3::Y);
        assert!(r.x_axis.is_finite() && (r.z_axis - Vec3::Y).length() < 1e-3);
    }

    #[test]
    fn a_map_projects_its_target_to_the_centre_and_its_near_plane_to_depth_one() {
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        fit.update(&Camera::default(), 16.0 / 9.0, Some(&sun()));
        let wide = &fit.maps[WIDE];
        let centre = wide.matrix().project_point3(Vec3::ZERO);
        assert!((centre.x - 0.5).abs() < 1e-5 && (centre.y - 0.5).abs() < 1e-5);
        // The target is 500 m from the light: reversed depth (1800 − 500) / 1799.
        assert!(
            (centre.z - (1800.0 - 500.0) / 1799.0).abs() < 1e-4,
            "{centre}"
        );
        // A point nearer the light is nearer: greater depth.
        let above = wide.matrix().project_point3(sun().toward_light * 10.0);
        assert!(above.z > centre.z);
        // +v is down the texture: a point with larger light-space y has smaller v.
        let r = light_orientation(sun().toward_light);
        let up = wide.matrix().project_point3(r.y_axis * 100.0);
        assert!(up.y < 0.5);
        assert!((wide.normal_bias - 0.75 * 760.0 / 2048.0).abs() < 1e-6);
    }

    #[test]
    fn the_near_map_steps_by_quarter_octaves_and_snaps_to_its_texels() {
        let camera = Camera::orbit(Vec3::ZERO, 35.0, 12.0, 110.0);
        let (focus, radius) = near_shadow_focus(&camera, 16.0 / 9.0, Vec3::ZERO, 380.0);
        assert!(focus.length() < 1e-3, "looking at the subject: {focus}");
        let octaves = radius.log2() * 4.0;
        assert!((octaves - octaves.round()).abs() < 1e-4);
        assert!((NEAR_SHADOW_MIN..=380.0).contains(&radius));

        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        fit.update(&camera, 16.0 / 9.0, Some(&sun()));
        assert_eq!(fit.reach, radius);
        let before = fit.maps[NEAR].matrix();
        let texel = 2.0 * fit.reach / 2048.0;
        // With nobody followed the grid is the world's: moving the camera a fraction of a texel
        // across the light leaves the map where it was, unless the focus crosses a grid line's midpoint.
        let r = light_orientation(sun().toward_light);
        let mut moved = camera;
        let nudge = r.x_axis * texel * 0.1;
        moved.eye += nudge;
        moved.target += nudge;
        fit.update(&moved, 16.0 / 9.0, Some(&sun()));
        let after = fit.maps[NEAR].matrix();
        let crossed = {
            let c = r.transpose() * (fit.focus - nudge);
            let d = r.transpose() * fit.focus;
            (c.x / texel).round() != (d.x / texel).round()
                || (c.y / texel).round() != (d.y / texel).round()
        };
        if !crossed {
            assert!(before.abs_diff_eq(after, 1e-6));
        }
        assert!((fit.maps[NEAR].normal_bias - 1.5 * texel).abs() < 1e-6);
    }

    #[test]
    fn the_near_map_holds_its_step_at_a_boundary() {
        // A camera whose frame is a hair either side of the midpoint between two quarter-octave
        // steps keeps the step it had.
        let at = |distance: f32| Camera::orbit(Vec3::ZERO, 35.0, 12.0, distance);
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        let (_, radius) = near_frame(&at(110.0), 16.0 / 9.0, Vec3::ZERO);
        let midpoint = 2f32.powf(((radius.log2() * 4.0).floor() + 0.5) / 4.0);
        let distance = 110.0 * midpoint / radius;
        fit.update(&at(distance * 0.999), 16.0 / 9.0, Some(&sun()));
        let held = fit.reach;
        fit.update(&at(distance * 1.001), 16.0 / 9.0, Some(&sun()));
        assert_eq!(fit.reach, held, "a hair past the midpoint");
        // Well past it, the step moves.
        fit.update(&at(distance * 1.1), 16.0 / 9.0, Some(&sun()));
        assert!(fit.reach > held);
    }

    #[test]
    fn the_near_map_closes_on_the_followed_hull_and_moves_with_her() {
        // The chase camera's frame (some 300 m across) is wider than a 250 m hull: the map closes
        // on her sphere.
        let hull = |center: Vec3| CasterSphere {
            center,
            radius: 126.0,
        };
        let follow = |center: Vec3| {
            let mut sun = sun();
            sun.target = Vec3::new(center.x, 0.0, center.z);
            sun.subject = Some(hull(center));
            sun
        };
        let start = Vec3::new(1.5, 0.3, -290.0);
        let camera = |center: Vec3| Camera::orbit(center, 210.0, 14.0, 345.0);
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        fit.update(&camera(start), 16.0 / 9.0, Some(&follow(start)));
        let reach = 126.0 / EDGE_START + SUBJECT_MARGIN;
        assert!((fit.reach - reach).abs() < 1e-4, "{}", fit.reach);
        // A point of her hull falls at the same place within its texel however far she steams.
        let mast = Vec3::new(3.0, 41.0, -17.0);
        let texel_place = |fit: &ShadowFit, center: Vec3| {
            let uv = fit.maps[NEAR].matrix().project_point3(center + mast) * 2048.0;
            Vec3::new(uv.x.fract(), uv.y.fract(), 0.0)
        };
        let before = texel_place(&fit, start);
        for step in [0.037, 0.21, 3.9, 117.3] {
            let moved = start + Vec3::new(step, 0.05 * step, -1.7 * step);
            fit.update(&camera(moved), 16.0 / 9.0, Some(&follow(moved)));
            let after = texel_place(&fit, moved);
            assert!(
                (after - before).abs().max_element() < 2e-3,
                "{step} m: {before} → {after}"
            );
        }
        // Close up, her sphere is wider than the frame: the frame's own fit.
        let close = Camera::orbit(start, 200.0, 25.0, 55.0);
        fit.update(&close, 16.0 / 9.0, Some(&follow(start)));
        assert!(fit.reach < reach);
    }

    #[test]
    fn aboard_the_near_map_holds_the_part_of_her_the_view_sees() {
        // An Iowa-sized hull (270 m, 33 m beam, 55 m to her masthead) heading 30° off north, the eye on her bridge 29 m
        // forward of her origin and 17 m up, looking down her forecastle; the morning sun.
        let heading = Mat4::from_rotation_y(-30f32.to_radians());
        let origin = Vec3::new(40.0, 0.2, -310.0);
        let matrix = Mat4::from_translation(origin) * heading;
        let hull = HullBox {
            matrix,
            low: Vec3::new(-16.5, -2.0, -135.0),
            high: Vec3::new(16.5, 55.0, 135.0),
        };
        let at = |x: f32, y: f32, z: f32| matrix.transform_point3(Vec3::new(x, y, z));
        let mut aboard = sun();
        aboard.target = Vec3::new(origin.x, 0.0, origin.z);
        aboard.subject = Some(CasterSphere {
            center: origin,
            radius: 146.0,
        });
        aboard.hull = Some(hull);
        let look = |pitch: f32, lens: f32| {
            let pitch = pitch.to_radians();
            let mut camera = Camera::look_at(
                at(0.0, 17.0, -29.0),
                at(0.0, 17.0 + 60.0 * pitch.tan(), -89.0),
            );
            camera.fov_y_degrees = 70.0 / lens;
            camera.near = 0.08;
            camera
        };
        // Her deck from below the eye to her stem, where the view shows it, is held whole (inside a square's fade) by the
        // near map short of the seam's band, by the aboard map past its start, and so by both across it.
        let whole = |map: &ShadowMap, p: Vec3| {
            let uv = map.matrix().project_point3(p);
            let edge = (uv.x - 0.5).abs().max((uv.y - 0.5).abs()) * 2.0;
            map.on && edge <= EDGE_START && (0.0..=1.0).contains(&uv.z)
        };
        let holds = |fit: &ShadowFit, camera: &Camera| {
            let view = camera.projection(16.0 / 9.0) * camera.view();
            let mut seen = 0;
            for z in (-134..=-35).step_by(3) {
                for x in [-16.0, 0.0, 16.0] {
                    let deck = at(x, 10.0, z as f32);
                    let clip = view * deck.extend(1.0);
                    let ndc = clip.truncate() / clip.w;
                    if clip.w <= 0.0 || ndc.x.abs() > 1.0 || ndc.y.abs() > 1.0 {
                        continue;
                    }
                    seen += 1;
                    let seam = fit.seam.truncate().dot(deck) + fit.seam.w;
                    if seam < 1.0 {
                        assert!(whole(&fit.maps[NEAR], deck), "{deck} short of the seam");
                    }
                    if seam > 0.0 {
                        assert!(
                            whole(&fit.maps[ABOARD], deck),
                            "{deck} past the seam's start"
                        );
                    }
                }
            }
            assert!(seen > 20, "the view sees her deck: {seen} points");
            // The near map an octave finer than the aboard map, through any lens.
            assert!(fit.maps[ABOARD].on);
            let ratio = fit.maps[ABOARD].half_width / fit.maps[NEAR].half_width;
            assert!((ratio - 2.0).abs() < 1e-3, "{ratio}");
        };
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        let wide = look(-10.0, 1.0);
        fit.update(&wide, 16.0 / 9.0, Some(&aboard));
        holds(&fit, &wide);
        // Finer than the wide map's 37 cm by far: centimetres.
        let texel = |map: &ShadowMap| 2.0 * map.half_width / 2048.0;
        assert!(
            texel(&fit.maps[NEAR]) < 0.04,
            "near: {} m texels",
            texel(&fit.maps[NEAR])
        );
        assert!(
            texel(&fit.maps[ABOARD]) < 0.08,
            "aboard: {} m texels",
            texel(&fit.maps[ABOARD])
        );
        assert!(fit.maps[ABOARD].drawn);
        // Turning the view a little holds the square's size, and its texels stay on her grid.
        let reach = fit.reach;
        fit.update(&look(-11.0, 1.0), 16.0 / 9.0, Some(&aboard));
        assert_eq!(fit.reach, reach);
        let grid = |fit: &ShadowFit| {
            let uv = fit.maps[NEAR].matrix().project_point3(at(3.0, 41.0, -17.0)) * 2048.0;
            Vec3::new(uv.x.fract(), uv.y.fract(), 0.0)
        };
        let before = grid(&fit);
        fit.update(&look(-11.5, 1.0), 16.0 / 9.0, Some(&aboard));
        assert_eq!(fit.reach, reach);
        assert!((grid(&fit) - before).abs().max_element() < 2e-3);
        // Through the walk's lens at x3 the same: the forecastle the narrow view sees is held whole.
        let narrow = look(-9.0, 3.0);
        fit.update(&narrow, 16.0 / 9.0, Some(&aboard));
        holds(&fit, &narrow);
        // Left of her box (a camera beside her), the old fit: the frame at her origin's distance.
        let mut beside = wide;
        beside.eye = at(-60.0, 17.0, -29.0);
        let mut alone = ShadowFit::new(ShadowSettings::with_map_size(2048));
        alone.update(&beside, 16.0 / 9.0, Some(&aboard));
        let mut unboxed = aboard.clone();
        unboxed.hull = None;
        let mut before = ShadowFit::new(ShadowSettings::with_map_size(2048));
        before.update(&beside, 16.0 / 9.0, Some(&unboxed));
        assert_eq!(alone.reach, before.reach);
        assert!((alone.focus - before.focus).length() < 1e-4);
        assert!(!alone.maps[ABOARD].on && !alone.maps[ABOARD].drawn);
        // Off her, no seam: the near map holds what it holds whole.
        assert_eq!(alone.seam, NO_SEAM);
        // So little of her in view that the squares' margins outweigh her (here a box 2 m a side about the eye): no cut
        // gives the near map an octave finer than the rest, so it holds it all.
        let wide = look(-10.0, 1.0);
        let mut small = aboard.clone();
        small.hull = Some(HullBox {
            matrix: Mat4::from_translation(wide.eye),
            low: Vec3::splat(-1.0),
            high: Vec3::splat(1.0),
        });
        let mut tight = ShadowFit::new(ShadowSettings::with_map_size(2048));
        tight.update(&wide, 16.0 / 9.0, Some(&small));
        assert!(!tight.maps[ABOARD].on);
        assert_eq!(tight.seam, NO_SEAM);
        let ahead = wide.eye + (wide.target - wide.eye).normalize() * 0.8;
        assert!(whole(&tight.maps[NEAR], ahead));
        assert!((tight.focus - ahead).length() < 2.0);
    }

    #[test]
    fn under_a_low_sun_the_square_maps_hold_their_whole_footprint_on_the_sea() {
        // A sun 10° up: the wide square's sides lie 380 / tan 10° = 2155 m along the light from its target.
        let elevation = 10f32.to_radians();
        let toward = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        fit.update(
            &Camera::default(),
            16.0 / 9.0,
            Some(&Sun::new(toward, Vec3::ZERO)),
        );
        let level = Vec3::new(toward.x, 0.0, toward.z).normalize();
        for map in [&fit.maps[WIDE], &fit.maps[NEAR]] {
            // Points on the sea at the square's sunward and far edges, a little inside them, are in the box.
            let reach = map.half_width * 0.95 / elevation.sin();
            for along in [reach, -reach] {
                let p = map.matrix().project_point3(map.target + level * along);
                assert!(
                    (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
                    "{:?} {along}: {p}",
                    map.kind
                );
                assert!(
                    (0.0..=1.0).contains(&p.z),
                    "{:?} {along}: depth {}",
                    map.kind,
                    p.z
                );
            }
        }
        // The sun's depth bias stays 0.9 m along the light.
        let wide = &fit.maps[WIDE];
        let metres = -wide.bias * (wide.far - wide.near);
        assert!((metres - 0.0005 * 1799.0).abs() < 1e-3, "{metres}");
        // A high sun keeps the web's box.
        fit.update(&Camera::default(), 16.0 / 9.0, Some(&sun()));
        assert_eq!(
            (fit.maps[WIDE].near, fit.maps[WIDE].far),
            (WIDE_NEAR, WIDE_FAR)
        );
        assert_eq!(fit.maps[WIDE].bias, WIDE_BIAS);
    }

    #[test]
    fn the_wide_map_snaps_to_a_grid_fixed_in_the_world() {
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        let texel = 2.0 * BATTLE_SHADOW_HALF / 2048.0;
        let r = light_orientation(sun().toward_light);
        let mut moved = sun();
        fit.update(&Camera::default(), 1.5, Some(&moved));
        let before = fit.maps[WIDE].matrix();
        // A target a fifth of a texel along the light's x leaves the map where it was.
        moved.target += r.x_axis * texel * 0.2;
        fit.update(&Camera::default(), 1.5, Some(&moved));
        assert!(before.abs_diff_eq(fit.maps[WIDE].matrix(), 1e-6));
        // Most of a texel moves it by exactly one.
        moved.target += r.x_axis * texel * 0.5;
        fit.update(&Camera::default(), 1.5, Some(&moved));
        let shift =
            fit.maps[WIDE].matrix().project_point3(Vec3::ZERO) - before.project_point3(Vec3::ZERO);
        assert!((shift.x.abs() * 2048.0 - 1.0).abs() < 1e-3, "{shift}");
    }

    #[test]
    fn view_maps_take_ships_beyond_the_wide_map_every_frame() {
        let camera = Camera::look_at(Vec3::new(0.0, 20.0, 50.0), Vec3::ZERO);
        let mut sun = sun();
        sun.casters = vec![
            CasterSphere {
                center: Vec3::ZERO,
                radius: 100.0,
            }, // inside the wide map
            CasterSphere {
                center: Vec3::new(0.0, 0.0, -1200.0),
                radius: 100.0,
            }, // band 1
            CasterSphere {
                center: Vec3::new(0.0, 0.0, -4000.0),
                radius: 120.0,
            }, // band 2
            CasterSphere {
                center: Vec3::new(0.0, 0.0, -9000.0),
                radius: 100.0,
            }, // beyond every band
        ];
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        for _ in 0..2 {
            fit.update(&camera, 1.5, Some(&sun));
            let drawn: Vec<bool> = fit.maps.iter().map(|map| map.drawn).collect();
            assert_eq!(
                [
                    drawn[NEAR],
                    drawn[WIDE],
                    drawn[VIEW_FIRST],
                    drawn[VIEW_FIRST + 1]
                ],
                [true; 4],
                "both bands, every frame"
            );
            assert!(!drawn[ABOARD], "no hull aboard");
            assert!(!fit.land_on(), "no land maps without land");
        }
        // A band with one ship is centred on her.
        let p = fit.maps[VIEW_FIRST]
            .matrix()
            .project_point3(Vec3::new(0.0, 0.0, -1200.0));
        assert!((p.x - 0.5).abs() < 1e-4 && (p.y - 0.5).abs() < 1e-4, "{p}");
        assert!((0.0..=1.0).contains(&p.z));
        // Low draws both, at its own 1024 texels.
        let mut low = ShadowFit::new(ShadowSettings::with_map_size(1024));
        low.update(&camera, 1.5, Some(&sun));
        let views = &low.maps[VIEW_FIRST..LAND_FIRST];
        assert!(
            views
                .iter()
                .all(|map| map.on && map.drawn && map.size == 1024)
        );
        // From Medium up the view maps stop at 2048, as the near map does.
        let mut high = ShadowFit::new(ShadowSettings::with_map_size(4096));
        high.update(&camera, 1.5, Some(&sun));
        assert!(
            high.maps[VIEW_FIRST..LAND_FIRST]
                .iter()
                .all(|map| map.on && map.size == 2048)
        );
        // Ships gone: bands stop sampling at once.
        sun.casters.clear();
        fit.update(&camera, 1.5, Some(&sun));
        assert!(!fit.maps[VIEW_FIRST].on && !fit.maps[VIEW_FIRST + 1].on);
    }

    #[test]
    fn under_a_low_sun_a_view_map_holds_the_sea_under_its_square() {
        // A sun 1° up: a hull's shadow on the sea runs kilometres down-sun, past the 1.2 km below her a view map used to
        // reach.
        let elevation = 1f32.to_radians();
        let toward = Vec3::new(elevation.cos(), elevation.sin(), 0.0);
        let camera = Camera::look_at(Vec3::new(0.0, 30.0, 0.0), Vec3::new(0.0, 0.0, -1200.0));
        let hull = Vec3::new(0.0, 5.0, -1200.0);
        let mut sun = Sun::new(toward, Vec3::ZERO);
        sun.casters = vec![CasterSphere {
            center: hull,
            radius: 60.0,
        }];
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        fit.update(&camera, 1.5, Some(&sun));
        let view = &fit.maps[VIEW_FIRST];
        assert!(view.on);
        // The sea down-sun of her that a 30 m mast shades, 1.7 km off, is inside the box.
        let mast_shadow = Vec3::new(hull.x - 30.0 / elevation.tan(), 0.0, hull.z);
        let p = view.matrix().project_point3(mast_shadow);
        assert!(
            (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y),
            "{p}"
        );
        assert!((0.0..=1.0).contains(&p.z), "depth {}", p.z);
        // The depth bias stays two texels along the light.
        let metres = -view.bias * (view.far - view.near);
        let texel = 2.0 * view.half_width.max(view.half_height) / view.size as f32;
        assert!(
            (metres - 2.0 * texel).abs() < 1e-3,
            "{metres} m, texel {texel}"
        );
        // A high sun keeps the box 1.2 km below her.
        sun.toward_light = Vec3::new(0.5, 0.6, 0.3).normalize();
        fit.update(&camera, 1.5, Some(&sun));
        let view = &fit.maps[VIEW_FIRST];
        let span = 2.0 * 60.0 * 1.0001;
        assert!(
            view.far - 1.0 - VIEW_ABOVE - VIEW_BELOW <= span,
            "{}",
            view.far
        );
    }

    #[test]
    fn fitting_one_frame_again_fits_the_same_maps() {
        let camera = Camera::look_at(Vec3::new(0.0, 20.0, 50.0), Vec3::ZERO);
        let mut sun = sun();
        sun.casters = vec![
            CasterSphere {
                center: Vec3::new(0.0, 0.0, -1200.0),
                radius: 100.0,
            },
            CasterSphere {
                center: Vec3::new(30.0, 0.0, -1300.0),
                radius: 60.0,
            },
            CasterSphere {
                center: Vec3::new(0.0, 0.0, -4000.0),
                radius: 120.0,
            },
        ];
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(2048));
        let state = |fit: &ShadowFit| {
            fit.maps
                .iter()
                .map(|map| (map.on, map.drawn, map.matrix()))
                .collect::<Vec<_>>()
        };
        fit.update(&camera, 1.5, Some(&sun));
        let first = state(&fit);
        // The renderer and the hulls' culling each fit a copy from the same state.
        let mut copy = fit.clone();
        copy.update(&camera, 1.5, Some(&sun));
        fit.update(&camera, 1.5, Some(&sun));
        assert_eq!(state(&fit), first);
        assert_eq!(state(&copy), first);
    }

    /// A plane 150 m up off a coast, looking a little down toward the land, under a morning sun.
    fn aloft() -> (Camera, Sun, ShadowSettings) {
        let camera = Camera::look_at(Vec3::new(0.0, 150.0, 0.0), Vec3::new(0.0, 0.0, -1200.0));
        let mut sun = Sun::new(
            Vec3::new(0.6, 0.5, 0.2).normalize(),
            Vec3::new(0.0, 0.0, 9000.0),
        );
        sun.land = Some(LandView {
            top: 120.0,
            nearest: 300.0,
        });
        let mut settings = ShadowSettings::with_map_size(2048);
        settings.land_maps = true;
        (camera, sun, settings)
    }

    /// Points of the ground (at `height`) the camera sees, from `near` to `far` along its view.
    fn seen_ground(camera: &Camera, aspect: f32, near: f32, far: f32, height: f32) -> Vec<Vec3> {
        let view = camera.projection(aspect) * camera.view();
        let forward = (camera.target - camera.eye).normalize();
        let mut points = Vec::new();
        for i in 0..=40 {
            for j in 0..=40 {
                let p = Vec3::new(
                    camera.eye.x + (i as f32 - 20.0) * far / 20.0,
                    height,
                    camera.eye.z + (j as f32 - 20.0) * far / 20.0,
                );
                let clip = view * p.extend(1.0);
                let ndc = clip.truncate() / clip.w;
                let depth = (p - camera.eye).dot(forward);
                if clip.w > 0.0
                    && ndc.x.abs() <= 1.0
                    && ndc.y.abs() <= 1.0
                    && (near..=far).contains(&depth)
                {
                    points.push(p);
                }
            }
        }
        points
    }

    fn held(map: &ShadowMap, p: Vec3) -> bool {
        let c = map.matrix().project_point3(p);
        let edge = (c.x - 0.5).abs().max((c.y - 0.5).abs()) * 2.0;
        edge <= EDGE_START && (0.0..=1.0).contains(&c.z)
    }

    #[test]
    fn the_land_maps_hold_the_ground_the_camera_sees_out_to_their_reach() {
        let (camera, sun, settings) = aloft();
        let mut fit = ShadowFit::new(settings);
        fit.update(&camera, 16.0 / 9.0, Some(&sun));
        let land = &fit.maps[LAND_FIRST..];
        assert!(
            land.iter()
                .all(|map| map.on && map.drawn && map.size == 2048)
        );
        assert_eq!(fit.uniforms(sun.toward_light).settings[3], 1.0);
        // Finer near the camera, coarser away from it.
        assert!(land[0].half_width < land[1].half_width && land[1].half_width < land[2].half_width);
        // Every point of the ground and the land's top in view to the reach is held whole by a land map.
        for height in [0.0, 120.0] {
            let ground = seen_ground(&camera, 16.0 / 9.0, 1.0, LAND_REACH * 0.98, height);
            assert!(ground.len() > 100, "{}", ground.len());
            for p in ground {
                assert!(land.iter().any(|map| held(map, p)), "{p} at {height} m");
            }
        }
        // The square sizes are quarter octaves.
        for map in land {
            let steps = map.half_width.log2() * 4.0;
            assert!((steps - steps.round()).abs() < 1e-4, "{}", map.half_width);
        }
    }

    #[test]
    fn the_land_maps_are_off_without_land_or_the_setting() {
        let (camera, sun, mut settings) = aloft();
        let mut no_land = sun.clone();
        no_land.land = None;
        let mut fit = ShadowFit::new(settings);
        fit.update(&camera, 16.0 / 9.0, Some(&no_land));
        assert!(!fit.land_on());
        assert_eq!(fit.uniforms(sun.toward_light).settings[3], 0.0);
        // Land in reach with the setting off (Medium): the same four maps as ever, and none of the land's.
        settings.land_maps = false;
        let mut plain = ShadowFit::new(settings);
        plain.update(&camera, 16.0 / 9.0, Some(&sun));
        assert!(!plain.land_on());
        let mut without = ShadowFit::new(settings);
        without.update(&camera, 16.0 / 9.0, Some(&no_land));
        for (a, b) in plain.maps.iter().zip(&without.maps) {
            assert_eq!((a.on, a.drawn, a.matrix()), (b.on, b.drawn, b.matrix()));
        }
        // The array keeps its four layers (near, aboard and view maps) with the land maps off.
        assert_eq!(settings.cascade_layers(), 4);
        assert_eq!(settings.land_size(), 1);
        // Land past the reach: nothing to hold.
        settings.land_maps = true;
        let mut far = sun.clone();
        far.land = Some(LandView {
            top: 50.0,
            nearest: 20_000.0,
        });
        let mut fit = ShadowFit::new(settings);
        fit.update(&camera, 16.0 / 9.0, Some(&far));
        assert!(!fit.land_on());
        // A sun at the horizon draws none.
        let mut low = sun.clone();
        low.toward_light = Vec3::new(1.0, 0.01, 0.0).normalize();
        fit.update(&camera, 16.0 / 9.0, Some(&low));
        assert!(!fit.land_on());
    }

    #[test]
    fn the_land_maps_hold_still_as_the_camera_moves_and_turns_a_little() {
        let (camera, sun, settings) = aloft();
        let mut fit = ShadowFit::new(settings);
        fit.update(&camera, 16.0 / 9.0, Some(&sun));
        let before: Vec<(f32, Mat4)> = fit.maps[LAND_FIRST..]
            .iter()
            .map(|map| (map.half_width, map.matrix()))
            .collect();
        // Flying on a few metres and turning a degree: each square keeps its size, and a point of the land falls on
        // the same place within its texel (the squares move by whole texels).
        let mut moved = camera;
        moved.eye += Vec3::new(2.3, 0.4, -3.1);
        let turn = glam::Quat::from_rotation_y(1f32.to_radians());
        moved.target = moved.eye + turn * (camera.target - camera.eye);
        fit.update(&moved, 16.0 / 9.0, Some(&sun));
        let point = Vec3::new(37.0, 4.0, -900.0);
        for (k, map) in fit.maps[LAND_FIRST..].iter().enumerate() {
            assert_eq!(map.half_width, before[k].0, "land {k} kept its size");
            let place = |m: Mat4| {
                let uv = m.project_point3(point) * map.size as f32;
                Vec3::new(uv.x.fract(), uv.y.fract(), 0.0)
            };
            let (a, b) = (place(before[k].1), place(map.matrix()));
            let wrap = |d: f32| d.abs().min(1.0 - d.abs());
            assert!(
                wrap(a.x - b.x) < 1e-2 && wrap(a.y - b.y) < 1e-2,
                "land {k}: {a} -> {b}"
            );
        }
        // A frame fitted again fits the same maps.
        let state = |fit: &ShadowFit| {
            fit.maps
                .iter()
                .map(|map| (map.on, map.matrix()))
                .collect::<Vec<_>>()
        };
        let first = state(&fit);
        fit.update(&moved, 16.0 / 9.0, Some(&sun));
        assert_eq!(state(&fit), first);
    }

    #[test]
    fn looking_down_the_land_maps_close_on_the_ground_below() {
        // 500 m over a flat town, straight down: the ground the camera sees is some 600 m across, all of it in the land
        // maps, the first with texels of a few decimetres.
        let mut camera = Camera::look_at(Vec3::new(0.0, 500.0, 0.0), Vec3::new(0.0, 0.0, -0.5));
        camera.fov_y_degrees = 55.0;
        let mut sun = Sun::new(
            Vec3::new(0.3, 0.8, 0.4).normalize(),
            Vec3::new(0.0, 0.0, 9000.0),
        );
        sun.land = Some(LandView {
            top: 30.0,
            nearest: 0.0,
        });
        let mut settings = ShadowSettings::with_map_size(2048);
        settings.land_maps = true;
        let mut fit = ShadowFit::new(settings);
        fit.update(&camera, 16.0 / 9.0, Some(&sun));
        let first = &fit.maps[LAND_FIRST];
        assert!(first.on);
        let texel = 2.0 * first.half_width / first.size as f32;
        assert!(texel < 0.6, "texel {texel} m");
        for p in seen_ground(&camera, 16.0 / 9.0, 1.0, 2000.0, 0.0) {
            assert!(fit.maps[LAND_FIRST..].iter().any(|map| held(map, p)), "{p}");
        }
    }

    #[test]
    fn under_a_low_sun_a_land_map_holds_the_slab_along_the_light() {
        // Suns a few degrees up on several bearings: the land's top and the sea under a square lie kilometres apart
        // along the light, the farthest square's footprint tens of kilometres (past the box's 30 km at 3° and lower:
        // the box keeps the view's ground and gives up the square's far corners).
        let (camera, mut sun, settings) = aloft();
        for (elevation, bearing) in [
            (5f32, 0f32),
            (3.0, 45.0),
            (2.0, 120.0),
            (1.5, 250.0),
            (3.0, 300.0),
        ] {
            let (e, b) = (elevation.to_radians(), bearing.to_radians());
            sun.toward_light = Vec3::new(b.cos() * e.cos(), e.sin(), b.sin() * e.cos());
            let mut fit = ShadowFit::new(settings);
            fit.update(&camera, 16.0 / 9.0, Some(&sun));
            assert!(fit.land_on());
            for height in [0.0, 120.0] {
                for p in seen_ground(&camera, 16.0 / 9.0, 1.0, LAND_REACH * 0.98, height) {
                    assert!(
                        fit.maps[LAND_FIRST..].iter().any(|map| held(map, p)),
                        "{elevation}° from {bearing}°: {p} at {height} m"
                    );
                }
            }
            for map in &fit.maps[LAND_FIRST..] {
                assert!(map.far - map.near <= LAND_DEPTH_MAX);
                // The depth bias stays a texel and a half along the light.
                let texel = 2.0 * map.half_width / map.size as f32;
                let metres = -map.bias * (map.far - map.near);
                assert!(
                    (metres - 1.5 * texel).abs() < 1e-3 * texel.max(1.0),
                    "{metres} {texel}"
                );
            }
        }
    }

    #[test]
    fn off_samples_nothing() {
        let mut fit = ShadowFit::new(ShadowSettings::default());
        fit.update(&Camera::default(), 1.5, Some(&sun()));
        assert!(fit.maps.iter().all(|map| !map.on && !map.drawn));
        assert_eq!(fit.uniforms(Vec3::Y).settings[0], 0.0);
        let mut fit = ShadowFit::new(ShadowSettings::with_map_size(4096));
        fit.update(&Camera::default(), 1.5, None);
        assert!(fit.maps.iter().all(|map| !map.on));
        assert_eq!(
            ShadowSettings::for_quality("medium").unwrap().map_size,
            2048
        );
        assert_eq!(ShadowSettings::for_quality("ultra").unwrap().map_size, 4096);
    }
}
