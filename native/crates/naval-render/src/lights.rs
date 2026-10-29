//! The scene's lights beyond the sun, and its fog, for every lit layer (`shaders/naval/lights.wgsl`,
//! `naval::lights`).
//!
//! The host gathers every light the frame has (the effects' flashes and fires, the star shells and searchlights,
//! lightning's stroke) and hands them to the renderer ([`crate::Renderer::set_point_lights`]), which keeps the
//! [`MAX_POINT_LIGHTS`] brightest at the camera ([`PointLights::brightest`]) in one list at group 0 that every lit layer
//! reads: the hulls, aircraft, wildlife, the land, the sea and the spray of the blast sheets. A clustered index culls it
//! for each view ([`LightClusters`]): the view is cut into [`CLUSTER_COLUMNS`] × [`CLUSTER_ROWS`] tiles and
//! [`CLUSTER_SLICES`] slices of depth, and each cell keeps a bit for every light whose light reaches it above
//! [`LIGHT_CUT`]. A pixel loops only over its cell's lights. The brightest few searchlights and star shells cast shadows
//! through maps of their own ([`light_shadow_maps`], layers of the sun's cascade array past its own maps), as many as
//! the Shadows setting allows ([`crate::shadows::ShadowSettings::light_maps`]).
//!
//! **Units.** A light's intensity is three's candela in the scene's units: the irradiance on a surface facing it at a
//! metre. By day the scene is the sun's: the noon sun at the sea is about 6 of its units for some 100,000 lux
//! ([`DAY_LUX`]). After dark the game lifts the night to a unit a lux: its full moon lights the sea at 0.27 units
//! ([`full_moon_scene`]), as the real one gives 0.27 lux ([`FULL_MOON_LUX`]). A real candela comes into the scene
//! through [`candela_scale`]: 1/[`DAY_LUX`] by day, the moon's lift after dark, and between them the game's moonlight over
//! the real twilight's (no light is lifted above the light the game shows the world by). Every light takes it: the star
//! shells and searchlights their real candela, the flashes (and what of them glows: their gas) the sun's units times
//! [`night_lift`]; after dark the fires and the flashes' light on the smoke take less of it (`naval_effects`'
//! `EffectLighting::set_lift`, `native/docs/lighting/lights.md`, "Fires and shots at night"). The eye closes down when they fill
//! the view (`crate::post`, "The night eye"). Their candela and colours are real values (`native/docs/lighting/lights.md`,
//! "Lights at night").
//!
//! [`SceneFog`] is the ocean's `scene.fogNode` (the web game's `ocean/screen/fog.ts`) as three applies it to every
//! material with `fog` on; layers keep it in their own uniforms ([`FogUniform`]).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

use crate::camera::{Camera, reversed_perspective};
use crate::shadows::{ShadowMap, ShadowMapKind};

/// The lights one frame hands the lit layers at most: the brightest at the camera ([`PointLights::brightest`]). A
/// busy night fight holds a few dozen (each turret's flash, every fire in view, the star shells and searchlights).
pub const MAX_POINT_LIGHTS: usize = 64;
/// The cluster index's words a cell: a bit for each light.
pub const LIGHT_WORDS: usize = MAX_POINT_LIGHTS / 32;
/// The clustered index's tiles across and down the view, and its slices of depth.
pub const CLUSTER_COLUMNS: u32 = 16;
pub const CLUSTER_ROWS: u32 = 9;
pub const CLUSTER_SLICES: u32 = 16;
pub const CLUSTER_COUNT: usize = (CLUSTER_COLUMNS * CLUSTER_ROWS * CLUSTER_SLICES) as usize;
/// The first slice ends this far from the eye (m), the last begins at [`CLUSTER_FARTHEST`]; the slices between share
/// the span geometrically.
pub const CLUSTER_NEAREST: f32 = 4.0;
pub const CLUSTER_FARTHEST: f32 = 40_000.0;
/// The irradiance (luminance, the scene's units) under which a light lights nothing in a cell: a ten-thousandth of
/// the full moon's light in the game's night (about 3 × 10⁻⁵ lux in a real one), a hundredth of what the moonless
/// overcast sky gives a deck.
pub const LIGHT_CUT: f32 = 5e-5;
/// The most lights that cast shadows (the Ultra setting's).
pub const MAX_SHADOWED_LIGHTS: usize = 8;
/// The least light (irradiance, luminance, the scene's units) a caster must give what the camera sees to take a map: a
/// hundredth of the game's full moon, under which its shadows would not show beside the sky's light. By day no lamp
/// comes near it (a star shell's light is a ten-thousandth of that under the sun's scale), so no map is drawn.
pub const SHADOW_LEAST: f32 = 0.005;

/// Lux per unit of the scene's irradiance by day: the noon sun at the sea is about 6 units for some 100,000 lux
/// (`naval-effects`' fires take the same scale).
pub const DAY_LUX: f32 = 16_700.0;
/// The full moon's illuminance at the sea, high in a clear sky (lux): Bond and Henderson, "The Conquest of Darkness"
/// (1963), and Krisciunas and Schaefer (1991), 0.25–0.3 lux.
pub const FULL_MOON_LUX: f32 = 0.27;
/// The real light on a level surface under a clear sky by the sun's elevation (degrees, log10 lux): the sun and the
/// sky's (Bond and Henderson 1963; the US Naval Observatory's twilight tables: 3.4 lux at the end of civil twilight,
/// 0.008 at nautical, 0.0007 at astronomical).
const TWILIGHT: [(f32, f32); 11] = [
    (-18.0, -3.15),
    (-12.0, -2.10),
    (-9.0, -0.82),
    (-6.0, 0.53),
    (-4.0, 1.30),
    (-2.0, 2.18),
    (0.0, 2.85),
    (5.0, 3.78),
    (10.0, 4.18),
    (20.0, 4.54),
    (30.0, 4.78),
];

/// The game's full moon at the sea, luminance in the scene's units, from the sky's constants: its light above the air
/// (`naval_sky::state::MOONLIGHT`) through the air (`moon_grade::TRANSMITTANCE`), on the dome's scale
/// (`sky_grade::GAIN`), in its tint (`moon_tint`, whose luminance is about 0.58). The host passes it on
/// ([`candela_scale`]).
pub fn full_moon_scene(moonlight: f32, transmittance: f32, gain: f32, tint: Vec3) -> f32 {
    moonlight * transmittance * gain * luminance(tint)
}

/// The scene's intensity for a light of one real candela with the sun `sun_elevation` degrees up: 1/[`DAY_LUX`] by day;
/// after dark the game's full moon (`full_moon_scene`, its luminance at the sea) over the real one's
/// ([`FULL_MOON_LUX`]); between them the game's moonlight over the real light of the sky at that elevation
/// ([`TWILIGHT`]), never less than by day. A lamp is lifted with the night as the moon is, and by no more.
pub fn candela_scale(sun_elevation: f32, full_moon_scene: f32) -> f32 {
    let moon = full_moon_scene.max(0.0) * DAY_LUX;
    let real = twilight_lux(sun_elevation).max(FULL_MOON_LUX);
    (moon / real).clamp(1.0, moon / FULL_MOON_LUX) / DAY_LUX
}

/// The night's lift on real light: [`candela_scale`] over the sun's scale, 1 by day and 16,700 (14 stops) after dark.
/// What is given in the sun's units (the flashes' and fires' light, the glow of their gas and flames) is multiplied by
/// it, so it stands beside the moon as the star shells and searchlights do; the eye closes down for it when it fills the
/// view (`crate::Renderer::set_night_lift`, `crate::post::EyeAdaptationSettings`).
pub fn night_lift(sun_elevation: f32, full_moon_scene: f32) -> f32 {
    candela_scale(sun_elevation, full_moon_scene) * DAY_LUX
}

/// [`TWILIGHT`] at `elevation` degrees, log-linear between its rows (lux).
pub fn twilight_lux(elevation: f32) -> f32 {
    let first = TWILIGHT[0];
    let last = TWILIGHT[TWILIGHT.len() - 1];
    let log = if elevation <= first.0 {
        first.1
    } else if elevation >= last.0 {
        last.1
    } else {
        let i = TWILIGHT
            .windows(2)
            .position(|pair| elevation <= pair[1].0)
            .unwrap_or(0);
        let (a, b) = (TWILIGHT[i], TWILIGHT[i + 1]);
        a.1 + (b.1 - a.1) * (elevation - a.0) / (b.0 - a.0)
    };
    10f32.powf(log)
}

/// Rec. 709 luminance of a linear colour.
pub fn luminance(color: Vec3) -> f32 {
    color.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The map a light may cast its shadows through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightCaster {
    /// None: a flash, a fire, lightning's stroke.
    #[default]
    None,
    /// A searchlight: a map along its beam, as wide as its cone.
    Beam,
    /// A star shell hundreds of metres up: a map looking down under it.
    Overhead,
    /// A fire after dark: no map; the hulls march toward it on screen through their own depth for the frame's camera
    /// (`naval-ships`' `ship.wgsl`, `fire_seen`), so her deck behind her superstructure from the fire is not lit through
    /// it.
    Screen,
}

/// A point light as three's `PointLight` has it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointLight {
    /// World position (m).
    pub position: Vec3,
    /// Linear colour.
    pub color: Vec3,
    /// three's intensity (candela, the scene's units).
    pub intensity: f32,
    /// Cut-off distance (m); 0 reaches everywhere (as every light of the game's does since 2026-10-20).
    pub distance: f32,
    /// Decay exponent (three's default and the game's lights: 2).
    pub decay: f32,
    /// A searchlight's outward unit direction; zero makes an ordinary point light.
    pub direction: Vec3,
    /// Cosine of the searchlight's outer half angle.
    pub cone_cos: f32,
    /// The shadow it casts ([`light_shadow_maps`], [`LightCaster::Screen`]).
    pub caster: LightCaster,
    /// The size of what gives the light (m; 0 a point): a surface nearer than this takes the light it would at this
    /// distance, as one beside a body of flame takes no more than the flame's luminance times its albedo however near
    /// it stands, and the hulls' march toward a [`LightCaster::Screen`] light stops this short of it.
    pub radius: f32,
}

impl PointLight {
    /// The lightning stroke as `VisualEnvironment.syncLighting` lights meshes with it: a point of
    /// `intensity × 10⁶` candela at the stroke (the sky's `bolt`: irradiance `intensity × (1 km / r)²`), capped
    /// so it never lights the scene at `viewpoint` more than the noon sun (`BOLT_CEILING` 1 at the eye), tinted
    /// `BOLT_TINT` (0.8, 0.87, 1), no cut-off, decay 2.
    pub fn lightning(position: Vec3, intensity: f32, viewpoint: Vec3) -> Self {
        let near = position.distance_squared(viewpoint);
        Self {
            position,
            color: Vec3::new(0.8, 0.87, 1.0),
            intensity: (intensity * 1e6).min(near.max(1.0)),
            distance: 0.0,
            decay: 2.0,
            ..Default::default()
        }
    }

    /// Restrict the light to a cone, feathering the outer 15 percent of its angular extent.
    pub fn with_cone(mut self, direction: Vec3, half_angle_rad: f32) -> Self {
        self.direction = direction.normalize_or_zero();
        self.cone_cos = half_angle_rad
            .clamp(0.001, std::f32::consts::FRAC_PI_2)
            .cos();
        self
    }

    pub fn angular_attenuation(&self, outward: Vec3) -> f32 {
        if self.direction == Vec3::ZERO {
            return 1.0;
        }
        let inner = self.cone_cos + (1.0 - self.cone_cos) * 0.15;
        let t = ((self.direction.dot(outward.normalize_or_zero()) - self.cone_cos)
            / (inner - self.cone_cos).max(1e-6))
        .clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// three's `getDistanceAttenuation` at `distance` metres, held at the light's [`PointLight::radius`] nearer it (CPU
    /// twin of `naval::lights::point_light_at`).
    pub fn attenuation(&self, distance: f32) -> f32 {
        let falloff = 1.0 / distance.max(self.radius).powf(self.decay).max(0.01);
        if self.distance > 0.0 {
            let ratio = distance / self.distance;
            let window = (1.0 - ratio.powi(4)).clamp(0.0, 1.0);
            falloff * window * window
        } else {
            falloff
        }
    }

    /// Its luminous intensity: the colour's luminance times the intensity.
    pub fn power(&self) -> f32 {
        luminance(self.color) * self.intensity
    }

    /// Its light at `eye`, as the list ranks lights ([`PointLights::brightest`]): the power over the squared distance
    /// (a searchlight's whole power: it lights what the eye sees whichever way it points).
    pub fn light_at(&self, eye: Vec3) -> f32 {
        self.power() / self.position.distance_squared(eye).max(1.0)
    }

    /// Its light at `point`: the power over the squared distance, through its cone and its cut-off's window.
    pub fn shining_on(&self, point: Vec3) -> f32 {
        let distance = self.position.distance(point);
        self.power()
            * self.attenuation(distance.max(1.0))
            * self.angular_attenuation(point - self.position)
    }

    /// How far its light reaches above [`LIGHT_CUT`] (m), its cut-off's window included.
    pub fn reach(&self) -> f32 {
        let reach = if self.decay > 0.0 {
            (self.power().max(0.0) / LIGHT_CUT).powf(1.0 / self.decay)
        } else {
            f32::INFINITY
        };
        if self.distance > 0.0 {
            reach.min(self.distance)
        } else {
            reach
        }
    }
}

/// The frame's point lights, brightest first, at most [`MAX_POINT_LIGHTS`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointLights {
    lights: [PointLight; MAX_POINT_LIGHTS],
    count: usize,
}

impl Default for PointLights {
    fn default() -> Self {
        Self::NONE
    }
}

impl PointLights {
    /// No lights.
    pub const NONE: Self = Self {
        lights: [PointLight {
            position: Vec3::ZERO,
            color: Vec3::ZERO,
            intensity: 0.0,
            distance: 0.0,
            decay: 2.0,
            direction: Vec3::ZERO,
            cone_cos: -1.0,
            caster: LightCaster::None,
            radius: 0.0,
        }; MAX_POINT_LIGHTS],
        count: 0,
    };

    /// The lights that shine (intensity above 0), the first [`MAX_POINT_LIGHTS`] of them in the order given.
    pub fn new(lights: impl IntoIterator<Item = PointLight>) -> Self {
        let mut out = Self::NONE;
        for light in lights.into_iter().filter(|l| l.intensity > 0.0) {
            if out.count == MAX_POINT_LIGHTS {
                break;
            }
            out.lights[out.count] = light;
            out.count += 1;
        }
        out
    }

    /// The lights that shine, the [`MAX_POINT_LIGHTS`] brightest at `eye` ([`PointLight::light_at`]), brightest first:
    /// a light past the budget is dropped by its light at the camera, not by the order it came in.
    pub fn brightest(lights: impl IntoIterator<Item = PointLight>, eye: Vec3) -> Self {
        let mut ranked: Vec<(f32, PointLight)> = lights
            .into_iter()
            .filter(|l| l.intensity > 0.0 && l.position.is_finite())
            .map(|l| (l.light_at(eye), l))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        Self::new(ranked.into_iter().map(|(_, light)| light))
    }

    pub fn as_slice(&self) -> &[PointLight] {
        &self.lights[..self.count]
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The WGSL `SceneLights` block for these lights, the maps `shadows` (from [`light_shadow_maps`]) shading those that
    /// have one, their layers from `first_layer` in the sun's cascade array.
    pub fn uniform(&self, shadows: &[ShadowMap], first_layer: u32) -> SceneLightsUniform {
        let mut out = SceneLightsUniform::zeroed();
        out.count = [self.count as u32, shadows.len() as u32, first_layer, 0];
        out.grid = [
            CLUSTER_COLUMNS,
            CLUSTER_ROWS,
            CLUSTER_SLICES,
            LIGHT_WORDS as u32,
        ];
        out.slices = [
            CLUSTER_NEAREST,
            (CLUSTER_SLICES - 1) as f32 / (CLUSTER_FARTHEST / CLUSTER_NEAREST).ln(),
            0.0,
            0.0,
        ];
        for (slot, light) in out.lights.iter_mut().zip(self.as_slice()) {
            let color = light.color * light.intensity;
            *slot = PointLightUniform {
                position: light.position.extend(light.distance).to_array(),
                color: color.extend(light.decay).to_array(),
                cone: light.direction.extend(light.cone_cos).to_array(),
                shadow: [
                    0.0,
                    0.0,
                    if light.caster == LightCaster::Screen {
                        1.0
                    } else {
                        0.0
                    },
                    light.radius,
                ],
            };
        }
        for (index, map) in shadows.iter().enumerate().take(MAX_SHADOWED_LIGHTS) {
            let ShadowMapKind::Light { light, .. } = map.kind else {
                continue;
            };
            if let Some(slot) = out.lights.get_mut(light) {
                // The map's texel at a metre's distance (its angular size), for the receiver's offsets.
                let texel = 2.0 * (map.half_width / map.near.max(1e-3)) / map.size.max(1) as f32;
                slot.shadow = [(index + 1) as f32, texel, 0.0, slot.shadow[3]];
            }
            out.shadows[index] = map.matrix().to_cols_array();
        }
        out
    }
}

/// `PointLight` in `naval::lights`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct PointLightUniform {
    /// xyz position (m), w cut-off distance (m).
    pub position: [f32; 4],
    /// rgb colour × intensity, w decay.
    pub color: [f32; 4],
    /// xyz outward cone direction (zero for a point light), w cosine of the outer half angle.
    pub cone: [f32; 4],
    /// x its shadow map + 1 (0: none), y the map's texel at a metre (its receivers' offsets grow with it), z 1 for a light
    /// the hulls march toward on screen ([`LightCaster::Screen`]), w its [`PointLight::radius`] (m).
    pub shadow: [f32; 4],
}

/// `SceneLights` in `naval::lights`, group 0 binding 10.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct SceneLightsUniform {
    /// x the lights in use, y the shadowed lights, z their maps' first layer in the sun's cascade array, w unused.
    pub count: [u32; 4],
    /// The cluster index: columns, rows, slices and words a cell.
    pub grid: [u32; 4],
    /// x where the first slice ends (m), y slices per natural log of depth past it, zw unused.
    pub slices: [f32; 4],
    pub lights: [PointLightUniform; MAX_POINT_LIGHTS],
    /// World to each light map's (u, v, reversed depth), v down ([`ShadowMap::matrix`]).
    pub shadows: [[f32; 16]; MAX_SHADOWED_LIGHTS],
}

/// The cluster index's cell of a point `depth` metres along the view at NDC `ndc` (x right, y up): the WGSL
/// `light_cell`'s twin.
pub fn cluster_cell(ndc: [f32; 2], depth: f32) -> usize {
    let column = (((ndc[0] * 0.5 + 0.5) * CLUSTER_COLUMNS as f32) as i32)
        .clamp(0, CLUSTER_COLUMNS as i32 - 1);
    let row =
        (((0.5 - ndc[1] * 0.5) * CLUSTER_ROWS as f32) as i32).clamp(0, CLUSTER_ROWS as i32 - 1);
    let slice = cluster_slice(depth);
    ((slice * CLUSTER_ROWS + row as u32) * CLUSTER_COLUMNS + column as u32) as usize
}

/// The slice holding `depth` metres along the view: 0 nearer than [`CLUSTER_NEAREST`], then geometric slices to
/// [`CLUSTER_FARTHEST`], the last holding everything past it.
pub fn cluster_slice(depth: f32) -> u32 {
    if depth <= CLUSTER_NEAREST {
        return 0;
    }
    let per_log = (CLUSTER_SLICES - 1) as f32 / (CLUSTER_FARTHEST / CLUSTER_NEAREST).ln();
    (1.0 + (depth / CLUSTER_NEAREST).ln() * per_log).min((CLUSTER_SLICES - 1) as f32) as u32
}

/// The view's clustered light index: a bit for each light in each cell its light reaches ([`LIGHT_CUT`]), the
/// storage buffer at group 0 binding 11.
#[derive(Clone, Debug)]
pub struct LightClusters {
    masks: Vec<u32>,
}

impl Default for LightClusters {
    fn default() -> Self {
        Self {
            masks: vec![0; CLUSTER_COUNT * LIGHT_WORDS],
        }
    }
}

impl LightClusters {
    /// The words, cell by cell (`LIGHT_WORDS` a cell).
    pub fn words(&self) -> &[u32] {
        &self.masks
    }

    /// The lights of cell `cell`, by their index in the list.
    pub fn lights_in(&self, cell: usize) -> impl Iterator<Item = usize> + '_ {
        (0..MAX_POINT_LIGHTS)
            .filter(move |&i| self.masks[cell * LIGHT_WORDS + i / 32] & (1 << (i % 32)) != 0)
    }

    /// Fills the index for `lights` seen through `camera` at `aspect`: each light's reach a sphere about where the
    /// camera draws it (lowered by the Earth's curve, as its receivers are), its bits set in the cells whose slices its
    /// depth spans and whose tiles its projection covers. Conservative: a cell may hold a light that lights nothing in
    /// it, never the reverse. The curve lowers the receivers across a light's reach a little differently;
    /// [`CURVE_ROOM`] widens each sphere for it.
    pub fn build(&mut self, lights: &PointLights, camera: &Camera, aspect: f32) {
        self.masks.fill(0);
        let (view, projection) = (camera.view(), camera.projection(aspect));
        for (index, light) in lights.as_slice().iter().enumerate() {
            let reach = light.reach();
            if reach.is_nan() || reach <= 0.0 {
                continue;
            }
            let at = view.transform_point3(camera.curved(light.position));
            let depth = -at.z;
            let reach = reach + CURVE_ROOM + reach.min(CLUSTER_FARTHEST) * CURVE_SHARE;
            if depth + reach <= 0.0 {
                continue;
            }
            let slices = (
                cluster_slice((depth - reach).max(0.0)),
                cluster_slice(depth + reach),
            );
            let (columns, rows) = match screen_bounds(at, reach, projection) {
                Some(bounds) => bounds,
                None => continue,
            };
            let (word, bit) = (index / 32, 1u32 << (index % 32));
            for slice in slices.0..=slices.1 {
                for row in rows.0..=rows.1 {
                    let first = ((slice * CLUSTER_ROWS + row) * CLUSTER_COLUMNS) as usize;
                    for column in columns.0..=columns.1 {
                        self.masks[(first + column as usize) * LIGHT_WORDS + word] |= bit;
                    }
                }
            }
        }
    }
}

/// Room added to every light's reach for the Earth's curve (m, and a share of the reach): its receivers a reach away
/// drop a little more or less than the light (about 2 m for a kilometre's reach 10 km off).
const CURVE_ROOM: f32 = 5.0;
const CURVE_SHARE: f32 = 0.004;
/// Room about a light's projected bounds, in NDC: a jittered frame's pixels and a cell's edge.
const TILE_ROOM: f32 = 0.02;

/// The tiles (columns, rows: first and last) covering the projection of a sphere at view-space `centre` of `radius`, or
/// none when it lies wholly off the view. A sphere reaching the eye's plane covers every tile.
fn screen_bounds(centre: Vec3, radius: f32, projection: Mat4) -> Option<((u32, u32), (u32, u32))> {
    // Wholly outside a side of the view's frustum (Gribb and Hartmann's planes from the projection's rows).
    let rows = [projection.row(0), projection.row(1), projection.row(3)];
    let point = Vec4::from((centre, 1.0));
    for plane in [
        rows[2] + rows[0],
        rows[2] - rows[0],
        rows[2] + rows[1],
        rows[2] - rows[1],
    ] {
        let length = plane.truncate().length();
        if length > 0.0 && plane.dot(point) / length < -radius {
            return None;
        }
    }
    if -centre.z - radius <= 0.1 {
        return Some(((0, CLUSTER_COLUMNS - 1), (0, CLUSTER_ROWS - 1)));
    }
    let (mut min, mut max) = (glam::Vec2::splat(f32::MAX), glam::Vec2::splat(f32::MIN));
    for corner in 0..8 {
        let offset = Vec3::new(
            if corner & 1 == 0 { -radius } else { radius },
            if corner & 2 == 0 { -radius } else { radius },
            if corner & 4 == 0 { -radius } else { radius },
        );
        let clip = projection * Vec4::from((centre + offset, 1.0));
        let ndc = clip.truncate().truncate() / clip.w;
        min = min.min(ndc);
        max = max.max(ndc);
    }
    if min.x > 1.0 || min.y > 1.0 || max.x < -1.0 || max.y < -1.0 {
        return None;
    }
    let column = |x: f32| {
        (((x * 0.5 + 0.5) * CLUSTER_COLUMNS as f32) as i32).clamp(0, CLUSTER_COLUMNS as i32 - 1)
            as u32
    };
    let row = |y: f32| {
        (((0.5 - y * 0.5) * CLUSTER_ROWS as f32) as i32).clamp(0, CLUSTER_ROWS as i32 - 1) as u32
    };
    Some((
        (column(min.x - TILE_ROOM), column(max.x + TILE_ROOM)),
        (row(max.y + TILE_ROOM), row(min.y - TILE_ROOM)),
    ))
}

/// A searchlight's map: from this far ahead of its lamp (m), clear of her own house top.
const BEAM_NEAR: f32 = 3.0;
/// How far a searchlight's map reaches along the beam (m): past the lamp's range, the ships it lights.
const BEAM_FAR: f32 = 12_000.0;
/// The map's half angle over the cone's: its feathered edge and a little room.
const BEAM_ROOM: f32 = 1.1;
/// A star shell's map: its half angle from straight down (a footprint 1.7 times its height across the sea each side of
/// the point under it, its texels growing toward the edge as the angle's secant squared), and how far below the sea it
/// reaches (m).
const OVERHEAD_HALF_ANGLE: f32 = 60.0 * std::f32::consts::PI / 180.0;
const OVERHEAD_BELOW: f32 = 60.0;
const OVERHEAD_NEAR: f32 = 5.0;

/// The light maps for `lights` seen by a camera at `eye` looking at `target`: of the lights that cast through a map
/// ([`LightCaster::Beam`], [`LightCaster::Overhead`]),
/// the brightest on what the camera sees ([`PointLight::shining_on`] at the eye or at its target, the larger: a
/// searchlight trained elsewhere lights neither) and at least [`SHADOW_LEAST`] there, at most `count` (the Shadows setting's:
/// [`crate::shadows::ShadowSettings::light_maps`]) and [`MAX_SHADOWED_LIGHTS`], each `size` texels square
/// (`ShadowMapKind::Light` holds the map's number and the light's index in the list).
pub fn light_shadow_maps(
    lights: &PointLights,
    count: usize,
    size: u32,
    eye: Vec3,
    target: Vec3,
) -> Vec<ShadowMap> {
    let mut maps = Vec::new();
    if size == 0 {
        return maps;
    }
    let mut ranked: Vec<(f32, usize)> = lights
        .as_slice()
        .iter()
        .enumerate()
        .filter(|(_, light)| matches!(light.caster, LightCaster::Beam | LightCaster::Overhead))
        .map(|(index, light)| (light.shining_on(eye).max(light.shining_on(target)), index))
        .filter(|&(light, _)| light >= SHADOW_LEAST)
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, index) in ranked {
        let light = &lights.as_slice()[index];
        if maps.len() >= count.min(MAX_SHADOWED_LIGHTS) {
            break;
        }
        let (view, projection, half, near, far) = match light.caster {
            LightCaster::None | LightCaster::Screen => continue,
            LightCaster::Beam => {
                if light.direction == Vec3::ZERO {
                    continue;
                }
                let half = (light.cone_cos.clamp(-1.0, 1.0).acos() * BEAM_ROOM).clamp(0.002, 1.2);
                let up = if light.direction.y.abs() > 0.99 {
                    Vec3::Z
                } else {
                    Vec3::Y
                };
                (
                    glam::camera::rh::view::look_to_mat4(light.position, light.direction, up),
                    reversed_perspective(2.0 * half, 1.0, BEAM_NEAR, BEAM_FAR),
                    half,
                    BEAM_NEAR,
                    BEAM_FAR,
                )
            }
            LightCaster::Overhead => {
                let height = light.position.y;
                if height <= OVERHEAD_NEAR {
                    continue;
                }
                let far = height + OVERHEAD_BELOW;
                (
                    glam::camera::rh::view::look_to_mat4(light.position, Vec3::NEG_Y, Vec3::NEG_Z),
                    reversed_perspective(2.0 * OVERHEAD_HALF_ANGLE, 1.0, OVERHEAD_NEAR, far),
                    OVERHEAD_HALF_ANGLE,
                    OVERHEAD_NEAR,
                    far,
                )
            }
        };
        let extent = half.tan() * near;
        maps.push(ShadowMap {
            kind: ShadowMapKind::Light {
                map: maps.len(),
                light: index,
            },
            size,
            on: true,
            drawn: true,
            position: light.position,
            target: light.position + view.inverse().transform_vector3(Vec3::NEG_Z),
            half_width: extent,
            half_height: extent,
            near,
            far,
            normal_bias: 0.0,
            bias: 0.0,
            hull_depths: None,
            view,
            projection,
        });
    }
    maps
}

/// Group 0's light bindings for a pipeline drawn outside the renderer core (a crate's stills and tests, which bind a
/// group 0 of their own): the lights (binding 10) and their index (11), and a 1 × 1 depth array (binding 2) and
/// comparison sampler (4) where the core binds the sun's maps, which no light shadows through here.
pub struct StandaloneLights {
    lights: wgpu::Buffer,
    clusters: wgpu::Buffer,
    _depth: wgpu::Texture,
    depth_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
    index: LightClusters,
}

impl StandaloneLights {
    pub fn new(device: &wgpu::Device) -> Self {
        let lights = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("standalone point lights"),
            size: std::mem::size_of::<SceneLightsUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let clusters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("standalone point light clusters"),
            size: CLUSTER_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let depth = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("standalone light maps"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: crate::shadows::SHADOW_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("standalone light maps"),
            compare: Some(wgpu::CompareFunction::GreaterEqual),
            ..Default::default()
        });
        Self {
            lights,
            clusters,
            _depth: depth,
            depth_view,
            sampler,
            index: LightClusters::default(),
        }
    }

    /// The layout entries: bindings 2, 4, 10 and 11.
    pub fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 4] {
        let visibility = wgpu::ShaderStages::FRAGMENT;
        [
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
                binding: 4,
                visibility,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                count: None,
            },
            scene_lights_entry(),
            clusters_entry(),
        ]
    }

    /// The group entries for [`StandaloneLights::layout_entries`].
    pub fn group_entries(&self) -> [wgpu::BindGroupEntry<'_>; 4] {
        [
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&self.depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: self.lights.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: self.clusters.as_entire_binding(),
            },
        ]
    }

    /// Writes `lights` indexed for `camera` at `aspect`, unshadowed.
    pub fn write(
        &mut self,
        queue: &wgpu::Queue,
        lights: &PointLights,
        camera: &Camera,
        aspect: f32,
    ) {
        self.index.build(lights, camera, aspect);
        queue.write_buffer(&self.lights, 0, bytemuck::bytes_of(&lights.uniform(&[], 0)));
        queue.write_buffer(&self.clusters, 0, bytemuck::cast_slice(self.index.words()));
    }
}

/// The clustered index's bytes (binding 11).
pub const CLUSTER_BYTES: u64 = (CLUSTER_COUNT * LIGHT_WORDS * std::mem::size_of::<u32>()) as u64;

/// Group 0 binding 10: the point lights (`naval::lights::scene_lights`), read where surfaces are shaded.
pub fn scene_lights_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 10,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(
                std::mem::size_of::<SceneLightsUniform>() as u64
            ),
        },
        count: None,
    }
}

/// Group 0 binding 11: their clustered index (`naval::lights::light_clusters`).
pub fn clusters_entry() -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: 11,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: wgpu::BufferSize::new(CLUSTER_BYTES),
        },
        count: None,
    }
}

/// The scene's fog as the map and weather author it. The air a layer draws through is the aerial perspective
/// ([`crate::aerial`]: the atmosphere and the weather's haze), which `enabled` switches on for the layer; `end` is the
/// visibility the host hands the sky's haze. `color`, `start`, `power` and `sky_blend_distance` were the web's ramp
/// and are no longer drawn. Off by default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneFog {
    /// Linear RGB.
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub power: f32,
    pub sky_blend_distance: f32,
    pub enabled: bool,
}

impl Default for SceneFog {
    /// `Game.ts`'s fog until a scene applies its own (`#8b8f92`, 2.5 to 16 km, power 1.4, blended into the sky
    /// over 10 km), switched off.
    fn default() -> Self {
        Self {
            color: [0.258, 0.275, 0.287],
            start: 2500.0,
            end: 16000.0,
            power: 1.4,
            sky_blend_distance: 10000.0,
            enabled: false,
        }
    }
}

impl SceneFog {
    /// The WGSL `SceneFog` block.
    pub fn uniform(&self) -> FogUniform {
        FogUniform {
            color: [self.color[0], self.color[1], self.color[2], self.power],
            params: [
                self.start,
                self.end,
                self.sky_blend_distance,
                f32::from(u8::from(self.enabled)),
            ],
        }
    }
}

/// `SceneFog` in `naval::lights`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct FogUniform {
    /// rgb colour, a power.
    pub color: [f32; 4],
    /// start, end, sky blend, 1 while on.
    pub params: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attenuation_is_three_s_windowed_inverse_square() {
        let light = PointLight {
            intensity: 1.0,
            distance: 100.0,
            decay: 2.0,
            ..Default::default()
        };
        assert!((light.attenuation(10.0) - 0.01 * (1.0 - 1e-4f32).powi(2)).abs() < 1e-7);
        assert_eq!(light.attenuation(100.0), 0.0);
        assert_eq!(light.attenuation(150.0), 0.0);
        // Closer than 10 cm the falloff stops at 100.
        assert_eq!(light.attenuation(0.01), 100.0);
        let open = PointLight {
            distance: 0.0,
            ..light
        };
        assert!((open.attenuation(1000.0) - 1e-6).abs() < 1e-12);
    }

    fn lamp(at: Vec3, intensity: f32) -> PointLight {
        PointLight {
            position: at,
            color: Vec3::ONE,
            intensity,
            decay: 2.0,
            ..Default::default()
        }
    }

    #[test]
    fn the_list_keeps_the_brightest_at_the_eye_whatever_their_order() {
        // 80 lights, the far ones first: the 64 kept are the brightest at the eye, brightest first.
        let lights = (0..80).map(|i| lamp(Vec3::new(0.0, 0.0, -(80 - i) as f32 * 100.0), 1e6));
        let kept = PointLights::brightest(lights, Vec3::ZERO);
        assert_eq!(kept.len(), MAX_POINT_LIGHTS);
        let nearest = kept.as_slice()[0].position.z;
        assert_eq!(nearest, -100.0);
        assert!(
            kept.as_slice()
                .windows(2)
                .all(|w| w[0].light_at(Vec3::ZERO) >= w[1].light_at(Vec3::ZERO))
        );
        assert!(kept.as_slice().iter().all(|l| l.position.z >= -6400.0));
        // A dark light is never kept, and `new` keeps the order given.
        let given = PointLights::new([lamp(Vec3::X, 0.0), lamp(Vec3::Y, 1.0)]);
        assert_eq!(given.len(), 1);
        assert_eq!(given.as_slice()[0].position, Vec3::Y);
    }

    #[test]
    fn the_uniform_block_matches_the_shader() {
        assert_eq!(std::mem::size_of::<PointLightUniform>(), 64);
        assert_eq!(
            std::mem::size_of::<SceneLightsUniform>(),
            48 + 64 * MAX_POINT_LIGHTS + 64 * MAX_SHADOWED_LIGHTS
        );
        let lights = PointLights::new([lamp(Vec3::splat(3.0), 10.0)]);
        let uniform = lights.uniform(&[], 4);
        assert_eq!(uniform.count, [1, 0, 4, 0]);
        assert_eq!(uniform.lights[0].color, [10.0, 10.0, 10.0, 2.0]);
        assert_eq!(uniform.lights[0].position, [3.0, 3.0, 3.0, 0.0]);
        assert_eq!(uniform.lights[0].shadow[0], 0.0);
        assert_eq!(std::mem::size_of::<FogUniform>(), 32);
    }

    #[test]
    fn lightning_never_outshines_the_noon_sun_at_the_eye() {
        let bolt = PointLight::lightning(Vec3::new(0.0, 1000.0, 0.0), 2.0, Vec3::ZERO);
        // 2 × 10⁶ cd would be 2 at the eye 1 km off; the ceiling holds it to 1 (10⁶ = r²).
        assert_eq!(bolt.intensity, 1e6);
        let far = PointLight::lightning(Vec3::new(0.0, 1000.0, 0.0), 0.5, Vec3::ZERO);
        assert_eq!(far.intensity, 5e5);
    }

    #[test]
    fn searchlights_only_light_surfaces_inside_their_cone() {
        let light = PointLight {
            intensity: 1.0,
            ..Default::default()
        }
        .with_cone(Vec3::X, 8.0f32.to_radians());
        assert_eq!(light.angular_attenuation(Vec3::X), 1.0);
        assert_eq!(light.angular_attenuation(Vec3::Y), 0.0);
        assert_eq!(light.angular_attenuation(-Vec3::X), 0.0);
        let edge = Vec3::new(8.0f32.to_radians().cos(), 8.0f32.to_radians().sin(), 0.0);
        assert!(light.angular_attenuation(edge) < 1e-4);
        let uniform = PointLights::new([light]).uniform(&[], 0);
        assert_eq!(uniform.lights[0].cone[0], 1.0);
        assert_eq!(PointLight::default().angular_attenuation(Vec3::Y), 1.0);
    }

    #[test]
    fn a_lamp_is_lifted_with_the_night_as_the_moon_is() {
        // The game's moon: 0.4173 above the air, 0.8 through it, the dome's 1.4, its tint's luminance: the real 0.27 lux.
        let moon = full_moon_scene(0.4173, 0.8, 1.4, Vec3::new(0.4564, 0.5841, 0.8714));
        assert!((moon - FULL_MOON_LUX).abs() < 1e-3, "{moon}");
        // By day a candela is the sun's scale; after dark the moon's: a unit for a lux.
        let noon = candela_scale(60.0, moon);
        assert!((noon * DAY_LUX - 1.0).abs() < 1e-6);
        let night = candela_scale(-20.0, moon);
        assert!(
            (night - moon / FULL_MOON_LUX).abs() < 1e-3 * night,
            "{night}"
        );
        // Twilight between, never darker than day and never brighter than night, and falling as the sun climbs.
        let mut last = night;
        for tenth in -200..=300 {
            let scale = candela_scale(tenth as f32 / 10.0, moon);
            assert!(scale <= last * 1.0001 && scale >= noon * 0.9999);
            last = scale;
        }
        // At sunset (the sun on the horizon) the lamps are lifted about six times, as the moon is over 700 lux.
        let sunset = candela_scale(0.0, moon) * DAY_LUX;
        assert!((5.0..8.0).contains(&sunset), "{sunset}");
        assert!((twilight_lux(-6.0) - 3.4).abs() < 0.1);
        // The night's lift: 1 by day, the day's lux a unit (14 stops) after dark. A 5-inch star shell burns at its real
        // 825,000 candela in the scene's.
        assert!((night_lift(60.0, moon) - 1.0).abs() < 1e-6);
        assert!((night_lift(-20.0, moon) - DAY_LUX).abs() < 0.01 * DAY_LUX);
        assert!((candela_scale(-20.0, moon) * 825_000.0 - 825_000.0).abs() < 0.01e6);
    }

    #[test]
    fn the_index_holds_each_light_where_it_reaches() {
        let camera = Camera {
            eye: Vec3::new(0.0, 10.0, 0.0),
            target: Vec3::new(0.0, 10.0, -100.0),
            ..Camera::default()
        };
        // A small light 50 m ahead (reach about 450 m), a bright one far to the right and one behind the eye.
        let lights = PointLights::new([
            lamp(Vec3::new(0.0, 10.0, -50.0), 10.0),
            lamp(Vec3::new(2000.0, 10.0, -1000.0), 1e3),
            lamp(Vec3::new(0.0, 10.0, 600.0), 1.0),
        ]);
        let mut clusters = LightClusters::default();
        clusters.build(&lights, &camera, 16.0 / 9.0);
        let cell = |ndc: [f32; 2], depth: f32| {
            clusters
                .lights_in(cluster_cell(ndc, depth))
                .collect::<Vec<_>>()
        };
        // At the centre 50 m off: the near light (and the bright far one, whose reach is 4.5 km).
        assert_eq!(cell([0.0, 0.0], 50.0), [0, 1]);
        // Past the near light's reach it is gone.
        assert!(!cell([0.0, 0.0], 3000.0).contains(&0));
        // The light behind the eye reaches only the first slices: 600 m behind with a reach of 141 m, none.
        assert!(!cell([0.0, 0.0], 2.0).contains(&2));
        // A dark cell far to the left holds nothing near.
        assert!(
            cell([-0.95, 0.9], 20_000.0).is_empty() || !cell([-0.95, 0.9], 20_000.0).contains(&0)
        );
        // The cell lookup's slices: the first ends at 4 m, the last begins at 40 km.
        assert_eq!(cluster_slice(1.0), 0);
        assert_eq!(cluster_slice(4.5), 1);
        assert_eq!(cluster_slice(50_000.0), CLUSTER_SLICES - 1);
    }

    #[test]
    fn the_brightest_casters_take_the_maps() {
        let flare = PointLight {
            caster: LightCaster::Overhead,
            ..lamp(Vec3::new(0.0, 500.0, 0.0), 1e6)
        };
        let beam = PointLight {
            caster: LightCaster::Beam,
            ..lamp(Vec3::new(0.0, 20.0, 3000.0), 1e9)
        }
        .with_cone(Vec3::NEG_Z, 1.5f32.to_radians());
        let flash = lamp(Vec3::new(0.0, 10.0, 10.0), 1e9);
        let lights = PointLights::brightest([flare, beam, flash], Vec3::new(0.0, 10.0, 0.0));
        let maps = light_shadow_maps(&lights, 1, 1024, Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO);
        // The flash casts none; of the two casters the beam is the brighter at the eye.
        assert_eq!(maps.len(), 1);
        let ShadowMapKind::Light {
            map: 0,
            light: index,
        } = maps[0].kind
        else {
            panic!("{:?}", maps[0].kind)
        };
        assert_eq!(lights.as_slice()[index].caster, LightCaster::Beam);
        let both = light_shadow_maps(&lights, 4, 1024, Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO);
        assert_eq!(both.len(), 2);
        // The flare's map looks straight down: the point under it is the map's centre.
        let overhead = both.iter().find(|m| m.position.y == 500.0).unwrap();
        let under = overhead.matrix() * Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!((under.x / under.w - 0.5).abs() < 1e-4 && (under.y / under.w - 0.5).abs() < 1e-4);
        assert_eq!(
            light_shadow_maps(&lights, 4, 0, Vec3::ZERO, Vec3::ZERO).len(),
            0
        );
        // A beam trained away from the eye and its target lights neither: the flare takes the one map.
        let away = PointLight {
            caster: LightCaster::Beam,
            ..lamp(Vec3::new(0.0, 20.0, 3000.0), 1e9)
        }
        .with_cone(Vec3::Z, 1.5f32.to_radians());
        let lights = PointLights::brightest([flare, away], Vec3::new(0.0, 10.0, 0.0));
        let maps = light_shadow_maps(&lights, 1, 1024, Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO);
        assert_eq!(maps.len(), 1);
        assert_eq!(maps[0].position.y, 500.0);
        let uniform = lights.uniform(&both, 7);
        assert_eq!(uniform.count[1], 2);
        assert!(uniform.lights.iter().filter(|l| l.shadow[0] > 0.0).count() == 2);
    }

    #[test]
    fn a_fire_s_light_is_held_at_its_size_and_marched_toward_not_mapped() {
        // A body of flame 3 m across lights a plate a metre from its middle as one 3 m off; farther off, as a point.
        let fire = PointLight {
            caster: LightCaster::Screen,
            radius: 3.0,
            ..lamp(Vec3::new(0.0, 10.0, 0.0), 1e6)
        };
        assert_eq!(fire.attenuation(1.0), fire.attenuation(3.0));
        assert!((fire.attenuation(3.0) - 1.0 / 9.0).abs() < 1e-6);
        assert!((fire.attenuation(10.0) - 0.01).abs() < 1e-7);
        // It takes no map, whatever its light, and tells the hulls to march toward it: its uniform's z, its size in w.
        let lights = PointLights::brightest([fire], Vec3::ZERO);
        assert!(
            light_shadow_maps(&lights, 4, 1024, Vec3::ZERO, Vec3::new(0.0, 10.0, 0.0)).is_empty()
        );
        let uniform = lights.uniform(&[], 7);
        assert_eq!(uniform.lights[0].shadow, [0.0, 0.0, 1.0, 3.0]);
        // A flash is neither.
        let flash = PointLights::brightest([lamp(Vec3::ZERO, 1e6)], Vec3::ZERO).uniform(&[], 7);
        assert_eq!(flash.lights[0].shadow, [0.0; 4]);
    }
}
