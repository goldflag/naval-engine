//! Rain near the camera, splashes on the sea, lightning and the thunder cue, ported from
//! the web game's `sky/weather/WeatherSystem.ts` with `rain.ts`, `splashes.ts` and `boltMesh.ts` (the CPU side of each) and
//! their shaders (`shaders/rain.wgsl`, `splashes.wgsl`, `bolt.wgsl`). Since 2026-10-20 the rain is drawn as a camera
//! records it (`native/docs/clouds/weather.md`, "Rain as a camera records it"): the rain's own large drops at their rate's
//! sizes and speeds, each as faint as one exposure makes it, lit by the environment behind it; the splashes are
//! crowns at their real size; and the rain's share of the haze (`crate::aerial`) is its distance, where the web's
//! veil was a second fog over the finished scene.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use naval_render::{
    MakePipelines, MotionPipeline, Pipeline, PipelineId, PipelineRegistry, PlaceholderPipelines,
    RenderContext, ShaderError, UploadSink, VertexLayout,
};
use naval_sky::{BoltLight, SkyQuality, SkyScene};
use wgpu::util::DeviceExt;

use crate::bolt::{self, MAX_BOLT_SEGMENTS};
use crate::gpu;
use crate::layer::CloudView;
use crate::lightning::{
    self, LightningScheduler, Random, Strike, StrikeClouds, StrikeKind, Thunder,
};
use crate::quality::{self, MOST_RAIN_DROPS};
use crate::smoothstep;
use crate::{Targets, shaders};

/// Drops live in four nested boxes about the camera: edge (m), share of the drops. Most are in the nearest, where a
/// drop's streak shows; the farther boxes thin out with depth, and serve magnified views, which move every box out
/// along the view.
const RAIN_CLASSES: [(f32, f32); 4] = [(5.0, 0.7), (12.0, 0.2), (28.0, 0.07), (60.0, 0.03)];
/// How far each box's centre stands ahead of the camera, in half its edge.
const FORWARD_SHIFT: f32 = 0.8;
/// The rain rate (mm/h) at a precipitation of 1: a downpour. The storm's 0.236 is 2.2 mm/h, light rain, the rate its
/// 8 km visibility allows (`naval_conditions::STORM_RAIN`), and its cells' 0.9 (in the shafts) 32 mm/h, a heavy
/// shower's; the overcast's drizzle (0.15) 0.9 mm/h.
pub const RAIN_RATE_FULL: f64 = 40.0;
/// Marshall–Palmer's intercept (drops per m³ per mm of diameter; Marshall & Palmer 1948).
const DROP_INTERCEPT: f64 = 8000.0;
/// The smallest and largest drops drawn (mm): smaller ones are too faint to show one by one (the haze holds them),
/// larger ones break up as they fall.
const SMALLEST_DROP: f64 = 1.0;
const LARGEST_DROP: f64 = 6.0;
/// The lens's aperture (m) at a 52° field: a 26 mm-equivalent lens at about f/4, focused far. A drop 2 m off blurs
/// over three pixels at 1030 px; the aperture grows with the magnification (the same f-number).
const APERTURE: f32 = 0.006;
const SURFACE_WIND: f64 = 0.75;
const VELOCITY_SMOOTHING: f64 = 0.12;
const MAX_CAMERA_SPEED: f32 = 250.0;
const RAIN_CEILING: [f64; 2] = [1000.0, 1500.0];
/// A camera this high (m) sees no splash: the nearest sea in view lies beyond where a crown fades.
const SPLASH_CEILING: [f64; 2] = [8.0, 16.0];
const SPLASH_SHARE: f64 = 0.15;
const SPLASH_ZOOM: f32 = 3.0;
const SUBMERGED: f32 = -0.3;
const BOLT_VISIBILITY_DRY: f64 = 40000.0;
const BOLT_VISIBILITY_RAIN: f64 = 11000.0;
const BOLT_FOOT: f64 = -3.0;
const BOLT_RADIANCE: f64 = 150.0;
/// The square of sea the splashes are laid on (m; `REGION` in splashes.wgsl), and how far ahead its centre may stand.
const SPLASH_REGION: f32 = 24.0;
const SPLASH_FAR: f32 = 12.0;
/// The crowns' lives (s) per class: a crown rises and falls back in about a tenth of a second.
const SPLASH_PERIODS: [f64; 4] = [0.08, 0.1, 0.12, 0.15];
const SPLASH_CLOCK: f64 = 60.0;

/// The rain rate (mm/h) of a precipitation (0 dry … 1 a downpour).
pub fn rain_rate(precipitation: f64) -> f64 {
    RAIN_RATE_FULL * precipitation.clamp(0.0, 1.0).powi(2)
}

/// Marshall–Palmer's slope Λ (per mm of diameter) at a rain rate (mm/h): heavier rain has larger drops. The rate is
/// held over a millionth of a mm/h only so the power stays finite: the drops' density and extinction it gives fall
/// smoothly to nothing as the rain stops (no step when a weather drifts in or out of rain).
pub fn drop_slope(rate: f64) -> f64 {
    4.1 * rate.max(1e-6).powf(-0.21)
}

/// Drops of `smallest` mm and larger per m³ at a rain rate (mm/h).
pub fn drops_per_cubic_metre(rate: f64, smallest: f64) -> f64 {
    let slope = drop_slope(rate);
    DROP_INTERCEPT / slope * (-slope * smallest).exp()
}

/// Rain's extinction (per m) at a rain rate (mm/h), as an image takes it: the drops' geometric cross-section. Half of
/// what a drop stops is diffracted within a hundredth of a degree, inside the pixel, so it stays in the beam. 0.30 per
/// km in the storm's 2.2 mm/h (the game's visibility, `naval_sky::haze::SIGHTING_DEPTH` over it, 8 km; Koschmieder's
/// 13 km), 1.6 in a 32 mm/h downpour (1.5 km; Koschmieder's 2.4 km), 0.17 in the overcast's drizzle.
pub fn rain_extinction(rate: f64) -> f64 {
    if rate <= 0.0 {
        return 0.0;
    }
    let slope = drop_slope(rate) * 1000.0;
    DROP_INTERCEPT * 1000.0 * std::f64::consts::PI / (2.0 * slope.powi(3))
}

/// Terminal fall speed (m/s) of a drop `diameter` mm across (Atlas, Srivastava & Sekhon 1973; `rain.wgsl`'s twin).
pub fn terminal_speed(diameter: f64) -> f64 {
    9.65 - 10.3 * (-0.6 * diameter).exp()
}

/// How many of the rain's drops each drop drawn stands for, when a tier draws `drops` at a downpour's rate: a
/// downpour's drops from the smallest drawn up per m³ over the density drawn about the camera (every box's share over
/// its volume). A camera records many drops a few metres off, each covering a pixel for a fraction of a per cent of
/// the exposure; a tier draws a few per cent of them, each standing for the rest about it, so the light they take
/// from the scene is the rain's (coarser grained). Lighter rain draws proportionally fewer drops, so the weight holds
/// at every rate: about 14 on Medium's 8000 drops.
pub fn drop_weight(drops: u32) -> f32 {
    let drawn: f64 = RAIN_CLASSES
        .iter()
        .map(|&(edge, share)| f64::from(share) / f64::from(edge).powi(3))
        .sum::<f64>()
        * f64::from(drops.max(1));
    (drops_per_cubic_metre(RAIN_RATE_FULL, SMALLEST_DROP) / drawn).max(1.0) as f32
}

fn direct_share(kind: StrikeKind) -> f64 {
    match kind {
        StrikeKind::Ground => 1.0,
        StrikeKind::Cloud => 0.2,
    }
}

fn wrap(value: f32) -> f32 {
    value - value.floor()
}

/// `RainUniforms` in `shaders/rain.wgsl`, 416 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct RainUniforms {
    boxes: [[f32; 4]; 12],
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    camera_velocity: [f32; 3],
    opacity: f32,
    wind: [f32; 3],
    /// How many of the rain's drops each drop drawn stands for.
    weight: f32,
    /// Marshall–Palmer's slope (per mm), the smallest drop drawn (mm), the truncation at the largest, the aperture (m).
    drops: [f32; 4],
    bolt_direction: [f32; 3],
    bolt_irradiance: f32,
    camera: [f32; 3],
    padding: f32,
    screen: [f32; 2],
    padding_b: [f32; 2],
}

/// `SplashUniforms` in `shaders/splashes.wgsl`, 192 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct SplashUniforms {
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    cycles: [f32; 4],
    region: [f32; 4],
    camera_xz: [f32; 2],
    camera_height: f32,
    opacity: f32,
    /// The crowns' size over a 3 mm drop's.
    size: f32,
    /// The drawing buffer's height (px).
    screen_height: f32,
    padding: f32,
    /// The span (m) of the sea's heights about the camera the splashes stand on (`Weather::set_sea_heights`), 0 for
    /// the flat mean sea.
    sea_span: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct BoltUniforms {
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    origin: [f32; 3],
    main: f32,
    screen: [f32; 2],
    branches: f32,
    padding: f32,
}

/// What the weather lays on the rest of the scene this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WeatherLight {
    /// The strike lighting the clouds: channel position and radiant intensity (0 dark).
    pub position: Vec3,
    pub intensity: f32,
    /// The scene flash, a multiple of the ambient.
    pub flash: f32,
    /// The flash's direct light.
    pub bolt: BoltLight,
    /// The strike's irradiance at the camera and the direction toward it (for its light through the rain's drops).
    pub irradiance: f32,
    pub direction: Vec3,
}

/// Rain, splashes, lightning and thunder. See the module docs.
pub struct Weather {
    tier: quality::CloudTier,
    scheduler: LightningScheduler,
    active: Option<Strike>,
    bolt_visible: bool,
    channel_count: u32,
    precipitation: f64,
    clouds: StrikeClouds,
    // Rain state.
    drift: [Vec3; 4],
    velocity: Vec3,
    last_position: Option<Vec3>,
    wind: Vec3,
    rain_uniforms: RainUniforms,
    rain_count: u32,
    /// The rain's large drops drawn at full count: the share of the tier's drops the rate's density asks.
    rain_share: f64,
    // Splash state.
    clock: f64,
    splash_uniforms: SplashUniforms,
    splash_count: u32,
    bolt_uniforms: BoltUniforms,
    light: WeatherLight,
    /// Thunder due this frame for the game's audio (`onThunder`).
    pub thunder: Vec<Thunder>,
    // GPU.
    rain_buffer: wgpu::Buffer,
    splash_buffer: wgpu::Buffer,
    bolt_buffer: wgpu::Buffer,
    drops: wgpu::Buffer,
    splashes: wgpu::Buffer,
    bolt_start: wgpu::Buffer,
    bolt_end: wgpu::Buffer,
    rain_group: wgpu::BindGroup,
    splash_group: wgpu::BindGroup,
    bolt_group: wgpu::BindGroup,
    /// The draws' group layout, and the sea's heights about the camera with their repeating sampler (a flat placeholder
    /// until [`Weather::set_sea_heights`]).
    layout: wgpu::BindGroupLayout,
    sea_heights: (wgpu::TextureView, wgpu::Sampler),
    rain_pipeline: wgpu::RenderPipeline,
    splash_pipeline: wgpu::RenderPipeline,
    bolt_pipeline: wgpu::RenderPipeline,
    /// The draws' group layouts, for the registry's recipes.
    layouts: WeatherLayouts,
    targets: Targets,
}

/// The groups the weather's draws read: 0 the clouds' shadow (the rain and the splashes: the sun's beam through the
/// clouds), 1 the sky's, 2 the draw's own (its uniforms and the sea's heights), 3 the sky's environment (the rain and
/// the splashes: the light about them). The bolt reads 1 and 2.
#[derive(Clone, Debug)]
pub struct WeatherLayouts {
    pub shadow: wgpu::BindGroupLayout,
    pub sky: wgpu::BindGroupLayout,
    pub environment: wgpu::BindGroupLayout,
    weather: Option<wgpu::BindGroupLayout>,
}

impl WeatherLayouts {
    /// The layouts the weather is built over: the clouds' shadow's (`CloudShadow::bind_group_layout`), the sky's
    /// group 1 and its environment's (`naval_sky::EnvironmentMap::bind_group_layout`).
    pub fn new(
        shadow: &wgpu::BindGroupLayout,
        sky: &wgpu::BindGroupLayout,
        environment: &wgpu::BindGroupLayout,
    ) -> Self {
        Self {
            shadow: shadow.clone(),
            sky: sky.clone(),
            environment: environment.clone(),
            weather: None,
        }
    }

    fn weather(&self) -> &wgpu::BindGroupLayout {
        self.weather
            .as_ref()
            .expect("set when the weather is built")
    }
}

/// The groups the weather draws with this frame: the clouds' shadow, the sky's and its environment's.
#[derive(Clone, Copy)]
pub struct WeatherGroups<'a> {
    pub shadow: &'a wgpu::BindGroup,
    pub sky: &'a wgpu::BindGroup,
    pub environment: &'a wgpu::BindGroup,
}

/// The weather's pipelines on the renderer's registry ([`Weather::register_pipelines`]), taken in by
/// [`Weather::adopt_pipelines`].
#[derive(Clone, Copy, Debug)]
pub struct WeatherPipelines {
    draws: [PipelineId; 3],
    /// The rain's streaks in temporal AA's motion pass ([`Weather::draw_motion`]).
    pub rain_motion: PipelineId,
}

/// The weather's three draws.
#[derive(Clone, Copy, Debug)]
enum Draw {
    Rain,
    Splashes,
    Bolt,
}

const PREMULTIPLIED: wgpu::BlendState = wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
        operation: wgpu::BlendOperation::Add,
    },
};
const ADDITIVE: wgpu::BlendState = wgpu::BlendState {
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
const DROP: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
    format: wgpu::VertexFormat::Float32x4,
    offset: 0,
    shader_location: 0,
}];
const START: [wgpu::VertexAttribute; 1] = DROP;
const END: [wgpu::VertexAttribute; 1] = [wgpu::VertexAttribute {
    format: wgpu::VertexFormat::Float32x4,
    offset: 0,
    shader_location: 1,
}];

/// A draw's group: its uniforms and the sea's heights about the camera.
fn weather_group(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
    (heights, sampler): &(wgpu::TextureView, wgpu::Sampler),
) -> wgpu::BindGroup {
    gpu::bind_group(
        device,
        label,
        layout,
        &[
            (0, buffer.as_entire_binding()),
            (1, wgpu::BindingResource::TextureView(heights)),
            (2, wgpu::BindingResource::Sampler(sampler)),
        ],
    )
}

fn repeating_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("weather sea heights"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::Repeat,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// The flat mean sea: a 1×1 texture of zeroes.
fn flat_sea(device: &wgpu::Device) -> (wgpu::TextureView, wgpu::Sampler) {
    let view = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("weather flat sea"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    (view, repeating_sampler(device))
}

fn instance_layout(
    attributes: &'static [wgpu::VertexAttribute],
) -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: 16,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes,
    }
}

impl Draw {
    const ALL: [Draw; 3] = [Draw::Rain, Draw::Splashes, Draw::Bolt];

    fn label(self) -> &'static str {
        match self {
            Draw::Rain => "rain",
            Draw::Splashes => "rain splashes",
            Draw::Bolt => "lightning bolt",
        }
    }

    /// The WGSL source and the library path it is registered under.
    fn shader(self) -> (&'static str, &'static str) {
        match self {
            Draw::Rain => (shaders::RAIN_WGSL, shaders::RAIN_PATH),
            Draw::Splashes => (shaders::SPLASHES_WGSL, shaders::SPLASHES_PATH),
            Draw::Bolt => (shaders::BOLT_WGSL, shaders::BOLT_PATH),
        }
    }

    fn pipeline(
        self,
        device: &wgpu::Device,
        module: &wgpu::ShaderModule,
        layouts: &WeatherLayouts,
        targets: Targets,
    ) -> wgpu::RenderPipeline {
        let lit = [
            Some(&layouts.shadow),
            Some(&layouts.sky),
            Some(layouts.weather()),
            Some(&layouts.environment),
        ];
        let bolt = [None, Some(&layouts.sky), Some(layouts.weather())];
        let groups: &[Option<&wgpu::BindGroupLayout>] = match self {
            Draw::Rain | Draw::Splashes => &lit,
            Draw::Bolt => &bolt,
        };
        let (vertex, fragment, buffers, blend) = match self {
            Draw::Rain => (
                "rain_vertex",
                "rain_fragment",
                vec![Some(instance_layout(&DROP))],
                PREMULTIPLIED,
            ),
            Draw::Splashes => (
                "splash_vertex",
                "splash_fragment",
                vec![Some(instance_layout(&DROP))],
                PREMULTIPLIED,
            ),
            Draw::Bolt => (
                "bolt_vertex",
                "bolt_fragment",
                vec![Some(instance_layout(&START)), Some(instance_layout(&END))],
                ADDITIVE,
            ),
        };
        gpu::scene_pipeline(
            device,
            self.label(),
            module,
            vertex,
            fragment,
            groups,
            &buffers,
            Some(blend),
            targets,
        )
    }
}

impl Weather {
    /// The weather over `layouts` (the clouds' shadow's, the sky's and its environment's group layouts).
    pub fn new(
        device: &wgpu::Device,
        layouts: WeatherLayouts,
        quality: SkyQuality,
        targets: Targets,
    ) -> Self {
        Self::build(device, layouts, quality, targets, MakePipelines::Now)
    }

    /// The same with `placeholders` for its pipelines until [`Weather::adopt_pipelines`] takes the registry's.
    pub fn new_deferred(
        device: &wgpu::Device,
        layouts: WeatherLayouts,
        quality: SkyQuality,
        targets: Targets,
        placeholders: &PlaceholderPipelines,
    ) -> Self {
        Self::build(
            device,
            layouts,
            quality,
            targets,
            MakePipelines::Deferred(placeholders),
        )
    }

    /// Builds the three draws' pipelines through the renderer's registry, as [`Weather::new`] builds them. The
    /// clouds' files must be registered first (`crate::shaders::register`).
    pub fn register_pipelines(
        &self,
        ctx: &mut RenderContext<'_>,
    ) -> Result<WeatherPipelines, ShaderError> {
        let mut draws = Vec::new();
        for draw in Draw::ALL {
            let layouts = self.layouts.clone();
            let targets = self.targets;
            draws.push(ctx.add_pipeline(
                draw.label(),
                draw.shader().1,
                Vec::new(),
                Box::new(move |build| {
                    Pipeline::Render(draw.pipeline(build.device, build.module, &layouts, targets))
                }),
            )?);
        }
        // The motion pass binds the camera at group 0: its entries read neither the clouds' shadow nor the environment.
        let rain_motion = ctx.add_motion_pipeline(
            "rain motion",
            Draw::Rain.shader().1,
            MotionPipeline {
                vertex_buffers: vec![VertexLayout {
                    stride: 16,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: DROP.to_vec(),
                }],
                bind_group_layouts: vec![self.layouts.sky.clone(), self.layouts.weather().clone()],
                primitive: wgpu::PrimitiveState::default(),
                ..MotionPipeline::new("rain_motion_vertex", "rain_motion")
            },
        )?;
        Ok(WeatherPipelines {
            draws: [draws[0], draws[1], draws[2]],
            rain_motion,
        })
    }

    /// Takes the registry's current pipelines in place of the weather's own; whether any changed.
    pub fn adopt_pipelines(&mut self, registry: &PipelineRegistry, ids: &WeatherPipelines) -> bool {
        let mut changed = false;
        for (slot, id) in [
            &mut self.rain_pipeline,
            &mut self.splash_pipeline,
            &mut self.bolt_pipeline,
        ]
        .into_iter()
        .zip(ids.draws)
        {
            let new = registry.render(id);
            if slot != new {
                *slot = new.clone();
                changed = true;
            }
        }
        changed
    }

    fn build(
        device: &wgpu::Device,
        mut layouts: WeatherLayouts,
        quality: SkyQuality,
        targets: Targets,
        make: MakePipelines<'_>,
    ) -> Self {
        let uniform = |label: &str, size: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let rain_buffer = uniform("rain", std::mem::size_of::<RainUniforms>());
        let splash_buffer = uniform("splashes", std::mem::size_of::<SplashUniforms>());
        let bolt_buffer = uniform("bolt", std::mem::size_of::<BoltUniforms>());
        // Every drop's seed: its place in the box and its class with a fraction for its own size and brightness.
        let mut random = Random::new(7331);
        let total: f32 = RAIN_CLASSES.iter().map(|c| c.1).sum();
        let mut cumulative = [0f32; 4];
        let mut sum = 0.0;
        for (i, class) in RAIN_CLASSES.iter().enumerate() {
            sum += class.1;
            cumulative[i] = sum;
        }
        let mut drops = Vec::with_capacity(MOST_RAIN_DROPS as usize);
        for _ in 0..MOST_RAIN_DROPS {
            let pick = random.draw() as f32 * total;
            let k = cumulative.iter().position(|&edge| pick < edge).unwrap_or(0);
            drops.push([
                random.draw() as f32,
                random.draw() as f32,
                random.draw() as f32,
                k as f32 + random.draw() as f32 * 0.999,
            ]);
        }
        let capacity = (f64::from(MOST_RAIN_DROPS) * SPLASH_SHARE).ceil() as usize;
        let mut random = Random::new(4217);
        let splashes: Vec<[f32; 4]> = (0..capacity)
            .map(|_| {
                let (x, y, z) = (
                    random.draw() as f32,
                    random.draw() as f32,
                    random.draw() as f32,
                );
                [
                    x,
                    y,
                    z,
                    (random.draw() * 4.0).floor() as f32 + random.draw() as f32 * 0.999,
                ]
            })
            .collect();
        let instance = |label: &str, data: &[[f32; 4]]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(data),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            })
        };
        let drops = instance("rain drops", &drops);
        let splash_instances = instance("rain splashes", &splashes);
        let segments = vec![[0f32; 4]; MAX_BOLT_SEGMENTS];
        let bolt_start = instance("bolt starts", &segments);
        let bolt_end = instance("bolt ends", &segments);
        let layout = gpu::layout(
            device,
            "weather",
            &[
                gpu::uniform_entry(0),
                gpu::texture_2d_entry(1),
                gpu::sampler_entry(2),
            ],
        );
        let sea_heights = flat_sea(device);
        let group = |label: &str, buffer: &wgpu::Buffer| {
            weather_group(device, label, &layout, buffer, &sea_heights)
        };
        let rain_group = group("rain", &rain_buffer);
        let splash_group = group("splashes", &splash_buffer);
        let bolt_group = group("bolt", &bolt_buffer);
        layouts.weather = Some(layout.clone());
        let [rain_pipeline, splash_pipeline, bolt_pipeline] = Draw::ALL.map(|draw| {
            make.render(|| {
                let (source, path) = draw.shader();
                let module = shaders::shader_module(device, source, path, &[]);
                draw.pipeline(device, &module, &layouts, targets)
            })
        });
        let mut weather = Self {
            tier: quality::tier(quality),
            scheduler: LightningScheduler::new(1941),
            active: None,
            bolt_visible: false,
            channel_count: 0,
            precipitation: 0.0,
            clouds: StrikeClouds {
                altitude: 650.0,
                thickness: 3000.0,
            },
            drift: [Vec3::ZERO; 4],
            velocity: Vec3::ZERO,
            last_position: None,
            wind: Vec3::ZERO,
            rain_uniforms: RainUniforms {
                weight: 1.0,
                ..RainUniforms::default()
            },
            rain_count: 0,
            rain_share: 0.0,
            clock: 0.0,
            splash_uniforms: SplashUniforms::default(),
            splash_count: 0,
            bolt_uniforms: BoltUniforms::default(),
            light: WeatherLight::default(),
            thunder: Vec::new(),
            rain_buffer,
            splash_buffer,
            bolt_buffer,
            drops,
            splashes: splash_instances,
            bolt_start,
            bolt_end,
            rain_group,
            splash_group,
            bolt_group,
            layout,
            sea_heights,
            rain_pipeline,
            splash_pipeline,
            bolt_pipeline,
            layouts,
            targets,
        };
        weather.set_quality(quality);
        weather
    }

    /// Stand the splashes on the drawn sea: `heights` the ocean's heights about the camera (an `Rgba16Float` texture,
    /// metres in red, laid on the world in a square of `span` metres that repeats, `naval_ocean::rain_heights`), or the
    /// flat mean sea with `None`.
    pub fn set_sea_heights(
        &mut self,
        device: &wgpu::Device,
        heights: Option<(&wgpu::TextureView, f32)>,
    ) {
        self.sea_heights = match heights {
            Some((view, _)) => (view.clone(), repeating_sampler(device)),
            None => flat_sea(device),
        };
        self.splash_uniforms.sea_span = heights.map_or(0.0, |(_, span)| span);
        self.rain_group = weather_group(
            device,
            "rain",
            &self.layout,
            &self.rain_buffer,
            &self.sea_heights,
        );
        self.splash_group = weather_group(
            device,
            "splashes",
            &self.layout,
            &self.splash_buffer,
            &self.sea_heights,
        );
        self.bolt_group = weather_group(
            device,
            "bolt",
            &self.layout,
            &self.bolt_buffer,
            &self.sea_heights,
        );
    }

    pub fn set_quality(&mut self, quality: SkyQuality) {
        self.tier = quality::tier(quality);
        self.rain_uniforms.weight = drop_weight(self.tier.rain_drops);
    }

    /// Apply a scene's weather (`WeatherSystem.apply`).
    pub fn apply(&mut self, scene: &SkyScene) {
        let (weather, clouds) = (&scene.weather, &scene.clouds);
        self.precipitation = weather.precipitation.clamp(0.0, 1.0);
        self.scheduler.rate = weather.lightning.max(0.0);
        self.clouds = StrikeClouds {
            altitude: clouds.altitude,
            thickness: clouds.thickness,
        };
        // The drops the rain's rate makes: Marshall–Palmer's sizes from the smallest drawn, and as many of the tier's
        // drops as the rate's density of them asks against a downpour's (a drizzle's drops are few and small).
        let rate = rain_rate(self.precipitation);
        let slope = drop_slope(rate);
        self.rain_uniforms.drops[0] = slope as f32;
        self.rain_uniforms.drops[1] = SMALLEST_DROP as f32;
        self.rain_uniforms.drops[2] =
            (1.0 - (-slope * (LARGEST_DROP - SMALLEST_DROP)).exp()) as f32;
        self.rain_share = if rate > 0.0 {
            (drops_per_cubic_metre(rate, SMALLEST_DROP)
                / drops_per_cubic_metre(RAIN_RATE_FULL, SMALLEST_DROP))
            .min(1.0)
        } else {
            0.0
        };
        let heading = clouds.wind_heading.to_radians();
        self.wind = Vec3::new(heading.sin() as f32, 0.0, heading.cos() as f32)
            * (clouds.wind_speed * SURFACE_WIND) as f32;
        // A crown scales with its drop: the rate's mean drop over a 3 mm drop's.
        self.splash_uniforms.size =
            ((SMALLEST_DROP + 1.0 / slope).min(LARGEST_DROP) / 3.0).clamp(0.4, 1.4) as f32;
    }

    /// The rain's extinction (per m) the haze takes as its share (`crate::aerial`), at the scene's rain rate.
    pub fn extinction(&self) -> f64 {
        rain_extinction(rain_rate(self.precipitation))
    }

    /// Diagnostics (`forceStrike`): fire a flash of `kind` at `position` (where a ground bolt meets the sea) and show
    /// it `age` seconds after its first return stroke, its flicker and bolt from `seed`. No thunder is cued; frames
    /// with `dt` 0 hold it.
    pub fn force_strike(
        &mut self,
        queue: &wgpu::Queue,
        position: Vec3,
        age: f64,
        kind: StrikeKind,
        seed: u32,
    ) {
        let mut random = Random::new(seed);
        let strokes = lightning::plan_strokes(kind, &mut random);
        let strike_seed = (random.draw() * 4_294_967_296.0).floor() as u32;
        let mut strike = Strike::new(kind, strokes, strike_seed);
        strike.ground = Vec3::new(position.x, 0.0, position.z);
        lightning::place_channel(&mut strike, self.clouds, &mut random);
        strike.age = age.max(0.0);
        strike.expose(strike.age, strike.age);
        self.show(&mut UploadSink::Queue(queue), strike, false);
    }

    /// Diagnostics: end any flash now.
    pub fn clear_strike(&mut self) {
        self.active = None;
        self.bolt_visible = false;
        self.light = WeatherLight::default();
    }

    pub fn strike(&self) -> Option<&Strike> {
        self.active.as_ref()
    }

    /// Lightning's light on the rest of the scene this frame.
    pub fn light(&self) -> WeatherLight {
        self.light
    }

    /// Advance the weather by `dt` for `view` (`WeatherSystem.update`); `sheltered` keeps rain out of hull views. Its
    /// writes go through `queue` (the renderer core's staging belt in a layer's `prepare`).
    pub fn update(
        &mut self,
        queue: &mut UploadSink<'_>,
        view: &CloudView,
        dt: f64,
        cut: bool,
        sheltered: bool,
    ) {
        self.thunder.clear();
        let position = view.position;
        self.update_lightning(queue, position, dt);
        let altitude = f64::from(position.y);
        let indoors = position.y < SUBMERGED || sheltered;
        let aloft = 1.0 - smoothstep(RAIN_CEILING[0], RAIN_CEILING[1], altitude);
        let rain = if indoors {
            0.0
        } else {
            self.precipitation * aloft
        };
        self.update_rain(view, dt, cut);
        // Each drop is as faint as its own exposure makes it; the rate sets how many there are (`apply`).
        self.rain_uniforms.opacity = aloft as f32;
        self.rain_count = if rain > 0.0 {
            (self.rain_share * f64::from(self.tier.rain_drops)).round() as u32
        } else {
            0
        }
        .min(MOST_RAIN_DROPS);
        let magnification = view.projection.y_axis.y * 26f32.to_radians().tan();
        self.rain_uniforms.drops[3] = APERTURE * magnification;
        let splash = if indoors || magnification > SPLASH_ZOOM {
            0.0
        } else {
            rain * (1.0 - smoothstep(SPLASH_CEILING[0], SPLASH_CEILING[1], altitude))
        };
        self.update_splashes(view, dt);
        self.splash_uniforms.opacity = (splash / self.precipitation.max(1e-3)).min(1.0) as f32;
        let capacity = (f64::from(MOST_RAIN_DROPS) * SPLASH_SHARE).ceil() as u32;
        self.splash_count = if splash > 0.0 {
            (self.precipitation * f64::from(self.tier.rain_drops) * SPLASH_SHARE).round() as u32
        } else {
            0
        }
        .min(capacity);
        // The drawing buffer and camera the meshes project with.
        let screen = [view.viewport[0] as f32, view.viewport[1] as f32];
        let view_matrix = view.view.to_cols_array_2d();
        let projection = view.projection.to_cols_array_2d();
        self.rain_uniforms.view = view_matrix;
        self.rain_uniforms.projection = projection;
        self.rain_uniforms.screen = screen;
        self.rain_uniforms.camera = position.to_array();
        self.splash_uniforms.screen_height = screen[1];
        self.rain_uniforms.bolt_irradiance = self.light.irradiance;
        self.rain_uniforms.bolt_direction = self.light.direction.to_array();
        self.splash_uniforms.view = view_matrix;
        self.splash_uniforms.projection = projection;
        self.bolt_uniforms.view = view_matrix;
        self.bolt_uniforms.projection = projection;
        self.bolt_uniforms.screen = screen;
        queue.write_buffer(
            &self.rain_buffer,
            0,
            bytemuck::bytes_of(&self.rain_uniforms),
        );
        queue.write_buffer(
            &self.splash_buffer,
            0,
            bytemuck::bytes_of(&self.splash_uniforms),
        );
        queue.write_buffer(
            &self.bolt_buffer,
            0,
            bytemuck::bytes_of(&self.bolt_uniforms),
        );
    }

    fn update_rain(&mut self, view: &CloudView, dt: f64, cut: bool) {
        let position = view.position;
        match self.last_position {
            Some(last) if !cut && dt > 0.0 => {
                let mut raw = (position - last) / dt as f32;
                if raw.length() > MAX_CAMERA_SPEED {
                    raw = raw.normalize() * MAX_CAMERA_SPEED;
                }
                self.velocity = self
                    .velocity
                    .lerp(raw, (1.0 - (-dt / VELOCITY_SMOOTHING).exp()) as f32);
            }
            Some(_) if !cut => {}
            _ => self.velocity = Vec3::ZERO,
        }
        self.last_position = Some(position);
        self.rain_uniforms.camera_velocity = self.velocity.to_array();
        // Zoomed optics look past the near drops: every box moves out along the view by the magnification.
        let magnification = (view.projection.y_axis.y * 26f32.to_radians().tan()).max(1.0);
        let forward = view
            .view
            .inverse()
            .transform_vector3(Vec3::NEG_Z)
            .normalize();
        let wind = self.wind;
        // Every box falls at the largest drop's speed; each drop at its own whole tenths of it (`rain.wgsl`).
        let speed = terminal_speed(LARGEST_DROP) as f32;
        for (k, &(size, _)) in RAIN_CLASSES.iter().enumerate() {
            let drift = &mut self.drift[k];
            if dt > 0.0 {
                let dt = dt as f32;
                *drift = Vec3::new(
                    wrap(drift.x + wind.x * dt / size),
                    (drift.y + speed * dt / size) % 10.0,
                    wrap(drift.z + wind.z * dt / size),
                );
            }
            let offset = forward * (FORWARD_SHIFT * size / 2.0 * magnification);
            let x = wrap(drift.x - (position.x + offset.x) / size + 0.5);
            let y = wrap(0.5 - (position.y + offset.y) / size);
            let z = wrap(drift.z - (position.z + offset.z) / size + 0.5);
            self.rain_uniforms.boxes[k * 3] = [x, y, z, size];
            self.rain_uniforms.boxes[k * 3 + 1] = [offset.x, offset.y, offset.z, speed];
            self.rain_uniforms.boxes[k * 3 + 2] = [drift.y, 0.0, 0.0, 0.0];
        }
        self.rain_uniforms.wind = wind.to_array();
    }

    fn update_splashes(&mut self, view: &CloudView, dt: f64) {
        if dt > 0.0 {
            self.clock = (self.clock + dt) % SPLASH_CLOCK;
        }
        let u = &mut self.splash_uniforms;
        u.cycles = SPLASH_PERIODS.map(|period| (self.clock / period) as f32);
        let position = view.position;
        let mut forward = view.view.inverse().transform_vector3(Vec3::NEG_Z);
        forward.y = 0.0;
        if forward.length_squared() < 1e-6 {
            forward = Vec3::Z;
        }
        // A level camera sees the sea from height / tan(half the vertical field) ahead.
        let nearest = position.y.max(0.0) * view.projection.y_axis.y;
        let forward = forward.normalize()
            * SPLASH_FAR.min((SPLASH_REGION * 0.3).max(nearest * 0.8 + SPLASH_REGION * 0.4));
        u.region = [
            wrap((position.x + forward.x) / SPLASH_REGION + 0.5),
            wrap((position.z + forward.z) / SPLASH_REGION + 0.5),
            forward.x,
            forward.z,
        ];
        u.camera_height = position.y;
        u.camera_xz = [position.x, position.z];
    }

    fn update_lightning(&mut self, queue: &mut UploadSink<'_>, camera: Vec3, dt: f64) {
        let due = self.scheduler.advance(dt);
        let mut fresh = false;
        for _ in 0..due {
            let strike = self.scheduler.next(camera, self.clouds);
            self.show(queue, strike, true);
            fresh = true;
        }
        // A strike shows its first stroke's peak the frame it fires, then each frame's exposure.
        if fresh {
            if let Some(active) = &mut self.active {
                active.expose(0.0, 0.0);
            }
        } else if let Some(active) = &mut self.active {
            active.advance(dt);
        }
        if self.active.as_ref().is_some_and(Strike::done) {
            self.active = None;
        }
        self.bolt_visible = false;
        let Some(active) = &self.active else {
            self.light = WeatherLight::default();
            self.bolt_uniforms.main = 0.0;
            self.bolt_uniforms.branches = 0.0;
            return;
        };
        let to_strike = active.position - camera;
        let distance = to_strike.length().max(1.0);
        let air =
            BOLT_VISIBILITY_DRY + (BOLT_VISIBILITY_RAIN - BOLT_VISIBILITY_DRY) * self.precipitation;
        let bolt_position = if active.kind == StrikeKind::Ground {
            active.top.lerp(active.ground, 0.5)
        } else {
            active.top
        };
        let bolt_intensity = active.intensity
            * direct_share(active.kind)
            * (-f64::from(bolt_position.distance(camera)) / air).exp();
        self.light = WeatherLight {
            position: active.position,
            intensity: active.intensity as f32,
            flash: lightning::flash_at(active.kind, active.brightness, f64::from(distance)) as f32,
            bolt: BoltLight {
                position: bolt_position,
                intensity: bolt_intensity as f32,
            },
            irradiance: (active.intensity * (1000.0 / f64::from(distance)).powi(2)) as f32,
            direction: to_strike / distance,
        };
        if active.kind == StrikeKind::Ground {
            let clarity = (-f64::from(active.ground.distance(camera)) / air).exp() * BOLT_RADIANCE;
            self.bolt_uniforms.origin = (active.ground - camera).to_array();
            self.bolt_uniforms.main = (active.brightness * clarity) as f32;
            self.bolt_uniforms.branches = (active.first_brightness * clarity) as f32;
            // From above the deck the cloud hides the bolt; its glow inside the cloud still shows.
            self.bolt_visible = f64::from(camera.y) < self.clouds.altitude;
        }
    }

    /// Make `strike` the active flash: build its bolt, and cue thunder for a natural one.
    fn show(&mut self, queue: &mut UploadSink<'_>, strike: Strike, thunder: bool) {
        if strike.kind == StrikeKind::Ground {
            let (top, ground) = (strike.top, strike.ground);
            let mut random = Random::new(strike.seed);
            let channel = bolt::build_bolt(
                &mut random,
                [
                    f64::from(top.x - ground.x),
                    f64::from(top.y),
                    f64::from(top.z - ground.z),
                ],
                [0.0, BOLT_FOOT, 0.0],
            );
            self.channel_count = channel.start.len() as u32;
            queue.write_buffer(&self.bolt_start, 0, bytemuck::cast_slice(&channel.start));
            queue.write_buffer(&self.bolt_end, 0, bytemuck::cast_slice(&channel.end));
        }
        if thunder {
            self.thunder
                .push(lightning::thunder_for(strike.kind, strike.distance));
        }
        self.active = Some(strike);
    }

    /// Splashes and the bolt, then the rain over them (the `Transparent` phase). The rain and the splashes bind the
    /// clouds' shadow at group 0 in place of the renderer's camera, which they do not read (the renderer binds the
    /// camera again before the next layer).
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, groups: WeatherGroups<'_>) {
        if self.splash_count > 0 {
            pass.set_pipeline(&self.splash_pipeline);
            pass.set_bind_group(0, groups.shadow, &[]);
            pass.set_bind_group(1, groups.sky, &[]);
            pass.set_bind_group(2, &self.splash_group, &[]);
            pass.set_bind_group(3, groups.environment, &[]);
            pass.set_vertex_buffer(0, self.splashes.slice(..));
            pass.draw(0..6, 0..self.splash_count);
        }
        if self.bolt_visible && self.channel_count > 0 {
            pass.set_pipeline(&self.bolt_pipeline);
            pass.set_bind_group(1, groups.sky, &[]);
            pass.set_bind_group(2, &self.bolt_group, &[]);
            pass.set_vertex_buffer(0, self.bolt_start.slice(..));
            pass.set_vertex_buffer(1, self.bolt_end.slice(..));
            pass.draw(0..6, 0..self.channel_count);
        }
        if self.rain_count > 0 {
            pass.set_pipeline(&self.rain_pipeline);
            pass.set_bind_group(0, groups.shadow, &[]);
            pass.set_bind_group(1, groups.sky, &[]);
            pass.set_bind_group(2, &self.rain_group, &[]);
            pass.set_bind_group(3, groups.environment, &[]);
            pass.set_vertex_buffer(0, self.drops.slice(..));
            pass.draw(0..6, 0..self.rain_count);
        }
    }

    /// The rain's streaks into temporal AA's motion pass with `pipeline` (the registry's
    /// [`WeatherPipelines::rain_motion`]): each falls somewhere new every frame, so it takes no history there.
    pub fn draw_motion(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        pipeline: &wgpu::RenderPipeline,
        sky_group: &wgpu::BindGroup,
    ) {
        if self.rain_count > 0 {
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, sky_group, &[]);
            pass.set_bind_group(2, &self.rain_group, &[]);
            pass.set_vertex_buffer(0, self.drops.slice(..));
            pass.draw(0..6, 0..self.rain_count);
        }
    }

    /// Drops and splashes drawn this frame.
    pub fn counts(&self) -> (u32, u32) {
        (self.rain_count, self.splash_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size of the struct `name` in the composed `source`.
    fn wgsl_size(source: &str, path: &str, name: &str) -> usize {
        let module =
            crate::shaders::compose(source, path, &[]).unwrap_or_else(|report| panic!("{report}"));
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref().is_some_and(|n| n.starts_with(name)))
            .unwrap_or_else(|| panic!("{name} in {path}"));
        ty.inner.size(module.to_ctx()) as usize
    }

    #[test]
    fn rust_and_wgsl_layouts_agree() {
        assert_eq!(
            wgsl_size(
                crate::shaders::RAIN_WGSL,
                crate::shaders::RAIN_PATH,
                "RainUniforms"
            ),
            std::mem::size_of::<RainUniforms>()
        );
        assert_eq!(
            wgsl_size(
                crate::shaders::SPLASHES_WGSL,
                crate::shaders::SPLASHES_PATH,
                "SplashUniforms"
            ),
            std::mem::size_of::<SplashUniforms>()
        );
    }

    /// Rain's extinction follows the drops' cross-section: Koschmieder's visibility about 2 km in a downpour of
    /// 30 mm/h and over 15 km in light rain (measured: 1–3 km in heavy rain).
    #[test]
    fn rain_stops_light_as_its_drops_do() {
        let downpour = rain_extinction(30.0);
        assert!((1.0e-3..2.5e-3).contains(&downpour), "{downpour}");
        assert!(rain_extinction(1.0) < 2.5e-4);
        assert_eq!(rain_extinction(0.0), 0.0);
        assert!(rain_extinction(rain_rate(0.9)) > rain_extinction(rain_rate(0.5)));
        // It rises from nothing as the rain starts: a trace of rain stops a trace of light, and the drops drawn follow.
        assert!(rain_extinction(rain_rate(0.005)) < 5.0e-6);
        assert!(rain_extinction(1e-4) < rain_extinction(1e-3));
        assert!(drops_per_cubic_metre(rain_rate(0.005), SMALLEST_DROP) < 1e-3);
    }

    /// Heavier rain has more and larger drops; a drizzle a few hundredths of a downpour's millimetre drops.
    #[test]
    fn heavier_rain_has_more_and_larger_drops() {
        assert!(drop_slope(30.0) < drop_slope(1.0));
        let storm = drops_per_cubic_metre(rain_rate(0.9), SMALLEST_DROP);
        let drizzle = drops_per_cubic_metre(rain_rate(0.15), SMALLEST_DROP);
        let full = drops_per_cubic_metre(RAIN_RATE_FULL, SMALLEST_DROP);
        assert!(storm < full && storm > 0.7 * full, "{storm} of {full}");
        assert!(drizzle < 0.1 * full, "{drizzle} of {full}");
    }

    /// Terminal speeds rise with size to about 9.4 m/s for the largest drops (Gunn & Kinzer 1949).
    #[test]
    fn large_drops_fall_faster() {
        assert!((terminal_speed(1.0) - 4.0).abs() < 0.2);
        assert!((terminal_speed(3.0) - 8.0).abs() < 0.2);
        assert!(terminal_speed(LARGEST_DROP) < 9.5);
    }
}
