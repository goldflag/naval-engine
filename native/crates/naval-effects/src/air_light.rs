//! The light in the air at night: what the haze, the rain and the smoke screens scatter toward the eye from the
//! searchlights' beams and the burning star shells, and what those lights show of themselves (a lamp seen down its
//! beam, a flare's flame, the smoke trailing above it, the cloud base it lights), drawn by `shaders/air_light.wgsl`
//! as one pass over the screen in the effects layer's transparent phase, after every other effect.
//!
//! Single scattering, as the aerial perspective takes the sun's (naval-clouds' `aerial.wgsl`): along each view ray, up
//! to what it hits (the scene's depth), the air scatters `σ_s p(θ) I / r²` toward the eye, `I` the light's candela
//! (the light the lit layers take: [`Searchlight::light`], [`Flare::light`], so a beam and the spot it lights stay one
//! light), `σ_s` the weather's haze and rain ([`AirMedium`]: the visibility's, at the same heights) and the smoke
//! screens' `0.025 × density` per metre with their billows, `p` the haze's Cornette–Shanks phase (strongly forward:
//! a beam seen from inside it is far brighter than from abeam), each point's light dimmed by the air and the screens
//! between it and the lamp, and by the air, the clouds and the gas in front of the eye (the aerial volume, the smoke
//! occlusion map). The integral is taken in the scattering angle about the light: samples spaced evenly in the phase
//! function's integral over the angle (the table here), which takes the inverse square and the phase exactly, so a few
//! carry the rest (the haze's height, the light's path, a beam's edge).
//!
//! Nothing burns, nothing is drawn: no pass, no upload. True night keeps only these lights in its air.

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Vec3};
use naval_render::{Pipeline, PipelineId, PointLight, RenderContext, ScenePipeline, ShaderError};

use crate::camera::CameraView;
use crate::gpu::{sampler_entry, texture_entry, uniform_entry};
use crate::tactical::{FLAME_RADIUS_M, Flare, LAMP_APERTURE_M, TacticalEffects};

const AIR_LIGHT: naval_render::ShaderFile = naval_render::shader_file!("shaders/air_light.wgsl");

/// The beams, flares and smoke screens one frame draws at most: the brightest at the eye. The simulation's lights and
/// sight never depend on it.
pub const MAX_BEAMS: usize = 8;
pub const MAX_FLARES: usize = 8;
pub const MAX_SCREENS: usize = 32;
/// Entries in the phase table (`AirUniform::cdf`), at the angles π (k / (TABLE − 1))²: finest ahead of a light, where
/// the haze's forward peak falls fast.
const TABLE: usize = 256;

/// A star shell's smoke (the burning candle's magnesium oxide and sodium salts): the mass it gives off a second (g/s),
/// that mass's extinction (m²/g, white metal-oxide smokes take 2–4), its single-scattering albedo and its phase's
/// asymmetry (particles of a few tenths of a micron), and how fast it spreads (m²/s, the turbulent diffusivity at a few
/// metres aloft). Estimates: the trail they make is a column some metres across a few tens of metres above the flame,
/// optically thick near it and thin above, which is how the photographs show it.
const TRAIL_MASS_RATE: f64 = 8.0;
const TRAIL_EXTINCTION: f64 = 3.0;
const TRAIL_ALBEDO: f64 = 0.9;
const TRAIL_ASYMMETRY: f64 = 0.6;
const TRAIL_DIFFUSIVITY: f64 = 0.5;
/// The longest trail drawn (m): above it the smoke has spread thin and its light from the flame below is gone.
const TRAIL_LONGEST: f64 = 200.0;
/// The points the trail is drawn through (`Flare::trail`, the candle and its smoke, a straight piece of the column
/// between each two): one every 2 s of the candle's burning, 8 m at a star shell's sink, more apart for a slower one
/// so the longest trail never needs more.
const TRAIL_POINTS: usize = 28;
const TRAIL_STEP_S: f64 = 2.0;
/// The share of the light from below a thick cloud deck sends back down (its albedo, 0.6–0.8 for a thick stratiform
/// deck): the patch a low flare lights on the cloud base.
const CLOUD_BASE_ALBEDO: f64 = 0.7;
/// Smoke screens' single-scattering albedo (white screening smoke scatters nearly all it stops).
const SCREEN_ALBEDO: f64 = 0.95;

/// The air the lights shine through, as the aerial perspective has it (naval-clouds' `aerial.wgsl`, `naval_sky::haze`):
/// the weather's haze (a marine boundary layer, densest at the sea), the rain's share of it, and the atmosphere's own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirMedium {
    /// The haze's extinction at the sea (per m, 550 nm): what the visibility asks less the atmosphere's own there.
    pub haze: f64,
    /// The boundary layer's top (m), the haze's scale height within it (m) and its taper's above it (m).
    pub top: f64,
    pub scale_height: f64,
    pub taper: f64,
    /// The haze's single-scattering albedo and its Cornette–Shanks asymmetry.
    pub albedo: f64,
    pub asymmetry: f64,
    /// The rain's extinction at the sea (per m, its share of the haze) and the clouds' base it falls from (m).
    pub rain: f64,
    pub rain_top: f64,
    /// The atmosphere's extinction at the sea as a view sees it (per m).
    pub atmosphere: f64,
}

impl Default for AirMedium {
    /// A fair day's air (`naval_sky::haze::Haze`'s default), no rain.
    fn default() -> Self {
        let haze = naval_sky::haze::Haze::default();
        Self {
            haze: haze.sighting_extinction(),
            top: haze.top,
            scale_height: haze.scale_height,
            taper: haze.taper,
            albedo: haze.albedo,
            asymmetry: haze.asymmetry,
            rain: 0.0,
            rain_top: 0.0,
            atmosphere: 0.0,
        }
    }
}

impl AirMedium {
    /// The sky's air and the weather's rain (`GameSky`'s rain in the haze: its extinction at the sea and its top), as
    /// the aerial perspective takes them this frame.
    pub fn of(sky: &naval_sky::Sky, rain: [f32; 2]) -> Self {
        let haze = sky.weather_haze();
        let air = sky.air();
        Self {
            haze: haze.extinction_at_sea(air),
            top: haze.top,
            scale_height: haze.scale_height,
            taper: haze.taper,
            albedo: haze.albedo,
            asymmetry: haze.asymmetry,
            rain: f64::from(rain[0].max(0.0)),
            rain_top: f64::from(rain[1].max(0.0)),
            atmosphere: air.aerial_scale * naval_sky::model::extinction(air, 0.0)[1] / 1000.0,
        }
    }
}

/// Cornette–Shanks' phase function (per steradian) at the cosine `mu` of the scattering angle.
pub fn phase(g: f64, mu: f64) -> f64 {
    let g2 = g * g;
    3.0 / (8.0 * std::f64::consts::PI) * (1.0 - g2) * (1.0 + mu * mu)
        / ((2.0 + g2) * (1.0 + g2 - 2.0 * g * mu).max(1e-9).powf(1.5))
}

/// The phase function's integral over the scattering angle (not the solid angle): `P(θ) = ∫₀^θ p(cos θ') dθ'` at
/// `TABLE` angles from 0 to π, spaced as the square of their index (`table_angle`), and its whole, `P(π)`. The shader
/// takes it as linear between them in the index and inverts that by search, so its samples follow the very phase their
/// span is measured with. The phase the table holds is constant over each step: where it changed by up to a third from
/// one step to the next, the glow about a light seen from inside its beam, whose brightness along each ray runs over the
/// table's angles from the ray's own angle to the light, broke into rings at the steps (64 steps of 2.9° until
/// 2026-10-20). At 256 steps spaced so, by about 5 % a step at most; a beam weighs each of its samples by the true phase
/// over the held one, so its glow follows the true phase whatever the table holds.
pub fn phase_table(g: f64) -> ([f32; TABLE], f64) {
    const FINE: usize = 4096;
    let step = std::f64::consts::PI / FINE as f64;
    let mut fine = vec![0.0f64; FINE + 1];
    for i in 0..FINE {
        let (a, b) = (i as f64 * step, (i + 1) as f64 * step);
        let mid = 0.5 * (a + b);
        // Simpson over each fine step.
        fine[i + 1] = fine[i]
            + step / 6.0 * (phase(g, a.cos()) + 4.0 * phase(g, mid.cos()) + phase(g, b.cos()));
    }
    let total = fine[FINE];
    let mut cdf = [0.0f32; TABLE];
    for (i, value) in cdf.iter_mut().enumerate() {
        let x = table_angle(i) / step;
        let k = (x.floor() as usize).min(FINE - 1);
        *value = (fine[k] + (fine[k + 1] - fine[k]) * (x - k as f64)) as f32;
    }
    (cdf, total)
}

/// The angle (rad) of the phase table's entry `index`: π (index / (TABLE − 1))², as `air_light.wgsl`'s `angle_at`
/// reads it.
pub fn table_angle(index: usize) -> f64 {
    let x = index as f64 / (TABLE - 1) as f64;
    std::f64::consts::PI * x * x
}

/// `Beam` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct BeamUniform {
    /// xyz the lamp (world m), w the light's reach (m; 0 none).
    origin: [f32; 4],
    /// xyz the beam's unit axis, w tan² of its outer half angle.
    axis: [f32; 4],
    /// rgb colour × candela, w the near field's radius (m): the mirror's radius over the half angle's tangent.
    color: [f32; 4],
    /// x cos of the outer half angle, y of the inner (the feathered edge), z the first of its screens, w their count.
    cone: [f32; 4],
}

/// `Flare` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct FlareUniform {
    /// xyz the burning candle (world m), w the light's reach (m; 0 none).
    position: [f32; 4],
    /// rgb colour × candela, w the flame's radius (m).
    color: [f32; 4],
    /// x the trail's length (m), y the smoke's extinction on its axis times its width (σ ρ², m), z its points, w 0.
    trail: [f32; 4],
    /// xyz the candle in the air (m from where it lit, the wind's drift taken out: the smoke holds still there, so its
    /// billows are rolled in it), w the billows' seed.
    air: [f32; 4],
    /// xyz the centre of a sphere about the flame and its smoke (m from the candle), w its radius (m).
    bound: [f32; 4],
    /// The trail's points from the candle up: xyz from the candle (m), w the smoke's spread there (ρ², m²).
    points: [[f32; 4]; TRAIL_POINTS],
}

/// `Screen` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct ScreenUniform {
    /// xyz the ellipsoid's centre (world m), w its horizontal radius (m).
    centre: [f32; 4],
    /// x its vertical half height (m), y its bulk extinction (per m), z its age (s), w its billows' seed.
    shape: [f32; 4],
}

/// `AirLight` in the shader.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AirUniform {
    /// Beams, flares and screens drawn.
    counts: [u32; 4],
    /// x the haze's extinction at the sea (per m), y the layer's top, z its scale height, w the taper's (m).
    haze: [f32; 4],
    /// x the haze's albedo, y the rain's extinction at the sea (per m), z the rain's top (m), w the atmosphere's
    /// extinction at the sea (per m).
    medium: [f32; 4],
    /// x the haze's asymmetry, y `P(π)`, z the cloud base's albedo, w the screens' albedo.
    phase: [f32; 4],
    /// x the trail's albedo, y its phase's asymmetry, z the pixel's angle at the screen's centre (rad), w 0.
    trail: [f32; 4],
    cdf: [[f32; 4]; TABLE / 4],
    beams: [BeamUniform; MAX_BEAMS],
    flares: [FlareUniform; MAX_FLARES],
    screens: [ScreenUniform; MAX_SCREENS],
}

const _: () = assert!(std::mem::size_of::<AirUniform>().is_multiple_of(16));

fn packed(table: &[f32; TABLE]) -> [[f32; 4]; TABLE / 4] {
    std::array::from_fn(|i| {
        [
            table[4 * i],
            table[4 * i + 1],
            table[4 * i + 2],
            table[4 * i + 3],
        ]
    })
}

/// A burning flare as the shader draws it: its flame, and its smoke through the trail's points
/// ([`Flare::trail`]).
fn flare_uniform(flare: &Flare) -> FlareUniform {
    let light = flare.light();
    let candle = flare.candle();
    // The smoke rises from the candle at the flare's sink: its mass per metre the candle's rate over that speed, so σ
    // on the axis = k λ / (π ρ²), λ = ṁ / w, and σ ρ² is constant up the trail. It spreads as it ages, ρ² = ρ₀² + 2 D t.
    let sink = flare.sink();
    let step = TRAIL_STEP_S.max(TRAIL_LONGEST / sink / (TRAIL_POINTS - 3) as f64);
    let trail = flare.trail(TRAIL_LONGEST, step);
    let axis_area = TRAIL_EXTINCTION * TRAIL_MASS_RATE / (sink * std::f64::consts::PI);
    let mut points = [[0.0f32; 4]; TRAIL_POINTS];
    let (mut low, mut high) = (DVec3::ZERO, DVec3::ZERO);
    let mut widest: f64 = FLAME_RADIUS_M;
    for (point, &(at, age)) in points.iter_mut().zip(&trail) {
        let offset = at - candle;
        let spread = FLAME_RADIUS_M * FLAME_RADIUS_M + 2.0 * TRAIL_DIFFUSIVITY * age;
        *point = offset.as_vec3().extend(spread as f32).to_array();
        (low, high) = (low.min(offset), high.max(offset));
        widest = widest.max(spread.sqrt());
    }
    let length = trail.last().map_or(0.0, |&(at, _)| at.y - candle.y);
    let centre = (low + high) * 0.5;
    // The billows swell the column by no more than half again (the shader's `TRAIL_SWELL`), three widths out.
    let radius = (high - centre).length() + 4.5 * widest;
    let seed = (flare.id as f64 * 0.3779).fract().abs();
    FlareUniform {
        position: light.position.extend(light.distance).to_array(),
        color: rgb_candela(&light).extend(FLAME_RADIUS_M as f32).to_array(),
        trail: [
            length as f32,
            axis_area as f32,
            trail.len().min(TRAIL_POINTS) as f32,
            0.0,
        ],
        air: (candle - flare.position - DVec3::Y * (sink * flare.burning))
            .as_vec3()
            .extend(seed as f32)
            .to_array(),
        bound: centre.as_vec3().extend(radius as f32).to_array(),
        points,
    }
}

fn rgb_candela(light: &PointLight) -> Vec3 {
    light.color * light.intensity
}

/// Whether the sphere at `centre` of `radius` touches the cone from `apex` along the unit `axis` with half angle
/// `half` out to `reach` (m).
fn sphere_in_cone(
    centre: DVec3,
    radius: f64,
    apex: DVec3,
    axis: DVec3,
    half: f64,
    reach: f64,
) -> bool {
    let d = centre - apex;
    if d.length() > reach + radius {
        return false;
    }
    let along = d.dot(axis);
    if along < -radius {
        return false;
    }
    let across = (d - axis * along).length();
    // The distance from the centre to the cone's surface, measured square to it.
    (across - along.max(0.0) * half.tan()) * half.cos() <= radius
}

/// The star shells' halo at half the resolution, for one view (the main one, or a second: `SceneView`).
struct Halo {
    size: (u32, u32),
    view: wgpu::TextureView,
    /// The composite's group over it.
    group: wgpu::BindGroup,
}

/// The pass: which lights and screens a frame draws, packed for the shader; the star shells' halo drawn at half the
/// resolution at the backdrop's copy point ([`AirLight::draw_halo`]); the composite added over the transparents
/// ([`AirLight::draw`]).
pub(crate) struct AirLight {
    pub medium: AirMedium,
    pipeline: PipelineId,
    halo_pipeline: PipelineId,
    layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    buffer: wgpu::Buffer,
    volume: Option<(wgpu::TextureView, wgpu::Sampler)>,
    group: Option<wgpu::BindGroup>,
    /// Each view's halo: the main view's and a second's.
    halos: [Option<Halo>; 2],
    /// The view this frame draws (0 the main, 1 a second).
    slot: usize,
    /// The asymmetry the phase table was made for, the table and its whole.
    table: Option<(f64, [[f32; 4]; TABLE / 4], f64)>,
    /// Whether this frame draws (a beam or a burning flare in view), and whether it has a halo to draw.
    live: bool,
    flares: bool,
}

const HALO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl AirLight {
    /// Registers the shader and its pipeline over the effect layer's lighting (group 1) and scene reads (group 3).
    pub(crate) fn new(
        ctx: &mut RenderContext<'_>,
        lighting_layout: &wgpu::BindGroupLayout,
        reads_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, ShaderError> {
        ctx.add_shader(AIR_LIGHT)?;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let entries = [
            uniform_entry(0, fragment),
            texture_entry(1, fragment, wgpu::TextureViewDimension::D3, true),
            sampler_entry(2, fragment),
        ];
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("air light"),
                entries: &entries,
            });
        let composite_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("air light composite"),
                    entries: &[
                        entries[0],
                        entries[1],
                        entries[2],
                        texture_entry(3, fragment, wgpu::TextureViewDimension::D2, false),
                    ],
                });
        // Added to the scene's light; the transparents' cover in alpha is left as it is (light, not a cover).
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = ctx.add_scene_pipeline(
            "air light",
            AIR_LIGHT.path,
            ScenePipeline {
                bind_group_layouts: vec![
                    lighting_layout.clone(),
                    composite_layout.clone(),
                    reads_layout.clone(),
                ],
                primitive: wgpu::PrimitiveState::default(),
                depth_write: false,
                blend: Some(add),
                defs: crate::layer::scene_defs(),
                ..ScenePipeline::new("air_vertex", "air_fragment")
            },
        )?;
        // The halo: the scene's camera at group 0, the layer's lighting and scene reads, into a half-resolution target
        // with no depth (the fragment reads the backdrop's).
        let layouts = [
            ctx.camera_layout.clone(),
            lighting_layout.clone(),
            layout.clone(),
            reads_layout.clone(),
        ];
        let halo_pipeline = ctx.add_pipeline(
            "air light halo",
            AIR_LIGHT.path,
            crate::layer::scene_defs(),
            Box::new(move |build| {
                let groups: Vec<Option<&wgpu::BindGroupLayout>> =
                    layouts.iter().map(Some).collect();
                let layout = build
                    .device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(build.label),
                        bind_group_layouts: &groups,
                        immediate_size: 0,
                    });
                Pipeline::Render(build.device.create_render_pipeline(
                    &wgpu::RenderPipelineDescriptor {
                        label: Some(build.label),
                        layout: Some(&layout),
                        vertex: wgpu::VertexState {
                            module: build.module,
                            entry_point: Some("air_vertex"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: wgpu::PrimitiveState::default(),
                        depth_stencil: None,
                        multisample: wgpu::MultisampleState::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: build.module,
                            entry_point: Some("halo_fragment"),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: HALO_FORMAT,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: build.cache,
                    },
                ))
            }),
        )?;
        let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("air light"),
            size: std::mem::size_of::<AirUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            medium: AirMedium::default(),
            pipeline,
            halo_pipeline,
            layout,
            composite_layout,
            buffer,
            volume: None,
            group: None,
            halos: [None, None],
            slot: 0,
            table: None,
            live: false,
            flares: false,
        })
    }

    /// Picks and packs this frame's lights for `camera` and uploads them; nothing when no beam or flare burns in view.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &mut self,
        (enabled, second): (bool, bool),
        device: &wgpu::Device,
        uploads: &mut naval_render::UploadSink<'_>,
        camera: &CameraView,
        (pixel_angle, viewport): (f64, (u32, u32)),
        (tactical, rules): (&TacticalEffects, &crate::EffectRules),
        (volume, volume_sampler): (&wgpu::TextureView, &wgpu::Sampler),
    ) {
        self.live = false;
        self.flares = false;
        self.slot = usize::from(second);
        // A camera under the sea sees none of the air's light.
        if !enabled || camera.position.y < 0.0 {
            return;
        }
        let frustum = camera.frustum();
        let in_view = |centre: DVec3, radius: f64| {
            frustum
                .iter()
                .all(|plane| plane.distance(camera.drawn(centre)) >= -radius)
        };
        let eye = camera.position;
        // The beams: those whose cone reaches into the view, the brightest at the eye first (the light at the
        // nearer of the lamp and the beam's middle).
        let mut beams: Vec<(f64, usize)> = tactical
            .searchlights
            .iter()
            .enumerate()
            .filter(|(_, lamp)| lamp.range > 0.0 && lamp.intensity > 0.0)
            .filter(|(_, lamp)| {
                let half = lamp.range * 0.5;
                in_view(
                    lamp.origin + lamp.direction * half,
                    half + lamp.range * lamp.half_angle.tan(),
                )
            })
            .map(|(i, lamp)| {
                let near = lamp
                    .origin
                    .distance(eye)
                    .min((lamp.origin + lamp.direction * lamp.range * 0.5).distance(eye));
                (lamp.light().intensity as f64 / (near * near).max(1.0), i)
            })
            .collect();
        beams.sort_by(|a, b| b.0.total_cmp(&a.0));
        beams.truncate(MAX_BEAMS);
        // The burning flares: all of the air about them can glow, so every one ignited, the brightest at the eye.
        let mut flares: Vec<(f64, usize)> = tactical
            .flares
            .iter()
            .enumerate()
            .filter(|(_, flare)| flare.ignited && flare.range > 0.0)
            .map(|(i, flare)| {
                let d = flare.position.distance(eye);
                (flare.light().intensity as f64 / (d * d).max(1.0), i)
            })
            .collect();
        flares.sort_by(|a, b| b.0.total_cmp(&a.0));
        flares.truncate(MAX_FLARES);
        if beams.is_empty() && flares.is_empty() {
            return;
        }
        let medium = self.medium;
        let g = medium.asymmetry.clamp(-0.95, 0.95);
        if self.table.as_ref().is_none_or(|t| t.0 != g) {
            let (cdf, total) = phase_table(g);
            self.table = Some((g, packed(&cdf), total));
        }
        let (_, cdf, total) = self.table.expect("made above");
        let mut uniform = AirUniform::zeroed();
        uniform.haze = [
            medium.haze as f32,
            medium.top as f32,
            medium.scale_height.max(1.0) as f32,
            medium.taper.max(1.0) as f32,
        ];
        uniform.medium = [
            medium.albedo as f32,
            medium.rain as f32,
            medium.rain_top as f32,
            medium.atmosphere as f32,
        ];
        uniform.phase = [
            g as f32,
            total as f32,
            CLOUD_BASE_ALBEDO as f32,
            SCREEN_ALBEDO as f32,
        ];
        uniform.trail = [
            TRAIL_ALBEDO as f32,
            TRAIL_ASYMMETRY as f32,
            pixel_angle as f32,
            0.0,
        ];
        uniform.cdf = cdf;
        // Each beam's screens: the puffs its cone passes through, nearest the lamp first.
        let mut screens = 0usize;
        for (slot, &(_, i)) in beams.iter().enumerate() {
            let lamp = &tactical.searchlights[i];
            let light = lamp.light();
            let first = screens;
            let mut touching: Vec<(f64, usize)> = tactical
                .smoke
                .iter()
                .enumerate()
                .filter(|(_, puff)| puff.density > 0.0 && puff.radius > 0.0 && puff.height > 0.0)
                .filter(|(_, puff)| {
                    sphere_in_cone(
                        puff.position,
                        puff.radius.max(puff.height * 0.5),
                        lamp.origin,
                        lamp.direction,
                        lamp.half_angle,
                        lamp.range,
                    )
                })
                .map(|(k, puff)| (puff.position.distance(lamp.origin), k))
                .collect();
            touching.sort_by(|a, b| a.0.total_cmp(&b.0));
            for &(_, k) in touching.iter().take(MAX_SCREENS - screens) {
                let puff = &tactical.smoke[k];
                uniform.screens[screens] = ScreenUniform {
                    centre: puff
                        .position
                        .as_vec3()
                        .extend(puff.radius as f32)
                        .to_array(),
                    shape: [
                        (puff.height * 0.5) as f32,
                        (rules.smoke_extinction_per_m * puff.density.clamp(0.0, 1.0)) as f32,
                        puff.age as f32,
                        (puff.id as f64 * 0.3779).fract() as f32,
                    ],
                };
                screens += 1;
            }
            let (outer, inner) = cone_cosines(light.cone_cos);
            let half = lamp.half_angle.clamp(1e-3, 1.5);
            uniform.beams[slot] = BeamUniform {
                origin: light.position.extend(light.distance).to_array(),
                axis: light
                    .direction
                    .extend((half.tan() * half.tan()) as f32)
                    .to_array(),
                color: rgb_candela(&light)
                    .extend((LAMP_APERTURE_M / half.tan()) as f32)
                    .to_array(),
                cone: [outer, inner, first as f32, (screens - first) as f32],
            };
        }
        for (slot, &(_, i)) in flares.iter().enumerate() {
            let flare = &tactical.flares[i];
            uniform.flares[slot] = flare_uniform(flare);
        }
        // The halo is drawn at the backdrop's copy point (`draw_halo`), which a frame without a backdrop never reaches:
        // the composite reads it only when it will be there.
        self.flares = !flares.is_empty();
        uniform.counts = [beams.len() as u32, flares.len() as u32, screens as u32, 0];
        uploads.write_buffer(&self.buffer, 0, bytemuck::bytes_of(&uniform));
        if self.volume.is_none() {
            self.volume = Some((volume.clone(), volume_sampler.clone()));
        }
        if self.group.is_none() {
            self.group = Some(self.group_over(device, &self.layout, None));
        }
        self.ensure_halo(device, viewport);
        self.live = true;
    }

    /// This view's halo target at half of `viewport` and the composite's group over it, remade when the size changes.
    fn ensure_halo(&mut self, device: &wgpu::Device, viewport: (u32, u32)) {
        let size = (viewport.0.div_ceil(2).max(1), viewport.1.div_ceil(2).max(1));
        if self.halos[self.slot]
            .as_ref()
            .is_some_and(|halo| halo.size == size)
        {
            return;
        }
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("air light halo"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HALO_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.group_over(device, &self.composite_layout, Some(&view));
        self.halos[self.slot] = Some(Halo { size, view, group });
    }

    /// Group 2 over the uniform and the screens' noise, and for the composite the halo too.
    fn group_over(
        &self,
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        halo: Option<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        let (volume, volume_sampler) = self.volume.as_ref().expect("bound in prepare");
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(volume),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(volume_sampler),
            },
        ];
        if let Some(halo) = halo {
            entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(halo),
            });
        }
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("air light"),
            layout,
            entries: &entries,
        })
    }

    /// The star shells' halo at half the resolution, at the backdrop's copy point (its depth taken, the aerial volume
    /// filled, the occlusion map drawn): `lighting` and `reads` are the effect layer's groups 1 and 3.
    pub(crate) fn draw_halo(
        &mut self,
        ctx: &mut naval_render::PrepareContext<'_>,
        lighting: &wgpu::BindGroup,
        reads: &wgpu::BindGroup,
    ) {
        if !self.live || !self.flares {
            return;
        }
        let (Some(halo), Some(group)) = (&self.halos[self.slot], &self.group) else {
            return;
        };
        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("air light halo"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &halo.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            timestamp_writes: ctx.timer.render_pass("air light halo"),
            ..Default::default()
        });
        pass.set_pipeline(ctx.pipelines.render(self.halo_pipeline));
        pass.set_bind_group(0, ctx.camera_group, &[]);
        pass.set_bind_group(1, lighting, &[]);
        pass.set_bind_group(2, group, &[]);
        pass.set_bind_group(3, reads, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        // The composite reads it this frame.
        ctx.uploads
            .write_buffer(&self.buffer, 12, bytemuck::bytes_of(&1u32));
    }

    /// One triangle over the screen, in the transparent phase with the layer's groups 1 and 3 bound: the composite
    /// over the halo this view's last drew (read only when drawn this frame: `counts.w`).
    pub(crate) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipelines: &naval_render::PipelineRegistry,
    ) {
        if !self.live {
            return;
        }
        let Some(halo) = &self.halos[self.slot] else {
            return;
        };
        pass.set_pipeline(pipelines.render(self.pipeline));
        pass.set_bind_group(2, &halo.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The cone's outer and inner cosines as `naval::lights` feathers it: the outer 15 % of `1 − cos` (`PointLight`'s
/// `with_cone`).
fn cone_cosines(outer: f32) -> (f32, f32) {
    (outer, outer + (1.0 - outer) * 0.15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phase_table_holds_the_phase() {
        let (cdf, total) = phase_table(0.75);
        // ∫ p dΩ = 1 over the sphere: 2π ∫ p sin θ dθ.
        let steps = 20_000;
        let h = std::f64::consts::PI / steps as f64;
        let sphere: f64 = (0..steps)
            .map(|i| {
                let t = (i as f64 + 0.5) * h;
                phase(0.75, t.cos()) * t.sin() * h
            })
            .sum::<f64>()
            * std::f64::consts::TAU;
        assert!((sphere - 1.0).abs() < 1e-3, "{sphere}");
        assert!((f64::from(cdf[TABLE - 1]) - total).abs() < 1e-4 * total);
        assert_eq!(cdf[0], 0.0);
        assert!(cdf.windows(2).all(|w| w[1] >= w[0]));
        // Strongly forward: half of the integral over the angle lies within the first 20°.
        let twenty = (0..TABLE)
            .find(|&k| table_angle(k) >= 20f64.to_radians())
            .expect("an entry past 20°");
        assert!(
            f64::from(cdf[twenty]) > 0.5 * total,
            "{} of {total}",
            cdf[twenty]
        );
        // The phase the table holds over each step changes by about 5 % at most from one step to the next.
        let held =
            |k: usize| f64::from(cdf[k + 1] - cdf[k]) / (table_angle(k + 1) - table_angle(k));
        for k in 0..TABLE - 2 {
            let (a, b) = (held(k), held(k + 1));
            assert!((a / b - 1.0).abs() < 0.055, "step {k}: {a} then {b}");
        }
        // Down the beam, far brighter than abeam.
        assert!(phase(0.75, 1.0) > 100.0 * phase(0.75, 0.0));
    }

    #[test]
    fn a_screen_across_the_beam_is_in_it_and_one_beside_it_is_not() {
        let apex = DVec3::ZERO;
        let axis = DVec3::NEG_Z;
        let half = 1.5f64.to_radians();
        assert!(sphere_in_cone(
            DVec3::new(0.0, 0.0, -2000.0),
            50.0,
            apex,
            axis,
            half,
            9000.0
        ));
        // 2 km out the beam is 52 m in radius: a puff of 50 m whose centre is 150 m off it does not touch it.
        assert!(!sphere_in_cone(
            DVec3::new(150.0, 0.0, -2000.0),
            50.0,
            apex,
            axis,
            half,
            9000.0
        ));
        assert!(sphere_in_cone(
            DVec3::new(90.0, 0.0, -2000.0),
            50.0,
            apex,
            axis,
            half,
            9000.0
        ));
        // Behind the lamp and beyond its reach.
        assert!(!sphere_in_cone(
            DVec3::new(0.0, 0.0, 500.0),
            50.0,
            apex,
            axis,
            half,
            9000.0
        ));
        assert!(!sphere_in_cone(
            DVec3::new(0.0, 0.0, -9500.0),
            50.0,
            apex,
            axis,
            half,
            9000.0
        ));
    }

    #[test]
    fn a_flares_bound_holds_its_flame_and_all_its_smoke() {
        let flare = Flare {
            id: 5,
            position: DVec3::new(10.0, 400.0, -30.0),
            velocity: DVec3::new(3.0, -4.0, 1.0),
            burning: 70.0,
            ignited: true,
            range: 3000.0,
            fresh: crate::illumination::StarShellCandle {
                candela: 825_500.0,
                reach_m: 3000.0,
            },
            candela: 1.0,
        };
        let drawn = flare_uniform(&flare);
        // The longest trail, its points from the candle up, within the points the shader holds.
        let count = drawn.trail[2] as usize;
        assert!(count > 20 && count <= TRAIL_POINTS, "{count}");
        assert!((drawn.trail[0] - TRAIL_LONGEST as f32).abs() < 1.0);
        assert_eq!(drawn.points[0][..3], [0.0; 3]);
        let centre = Vec3::from_slice(&drawn.bound[..3]);
        for point in &drawn.points[..count] {
            let reach = Vec3::from_slice(&point[..3]).distance(centre) + 4.5 * point[3].sqrt();
            assert!(reach <= drawn.bound[3] + 1e-3);
        }
    }
}
