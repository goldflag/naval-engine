//! The game frame the stills draw the land in: the game's sky with its clouds, the eye adaptation's sky mean, and the
//! sea in the `Water` phase reading copies of the opaque scene (so the land meets it with shoreline foam and
//! shallows). This is the render core II wiring the vertical slice still has to do; the stills do it here with small
//! wrapper layers, touching neither naval-ocean nor naval-clouds.

use naval_clouds::GameSky;
use naval_ocean::Ocean;
use naval_render::{
    CopyPoint, DrawContext, Phase, PipelineId, PrepareContext, RenderContext, SCENE_OPAQUE_COLOR,
    SCENE_OPAQUE_DEPTH, SKY_MEAN_BUFFER, SceneCopies, SceneLayer, ShaderError, compute_recipe,
};

/// The sky as `GameSky` draws itself: its clouds march at the opaque copy point. (It ran the rain's veil at the
/// backdrop copy point until 2026-10-20, when the veil became the rain's share of the haze.)
pub struct SkyWithPost(pub GameSky);

impl SceneLayer for SkyWithPost {
    fn label(&self) -> &str {
        "game sky"
    }
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        self.0.prepare(ctx);
    }
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        self.0.draw(phase, pass, ctx);
    }
    fn scene_copies(&self) -> SceneCopies {
        // The clouds' own copies (the opaque depth their march reads).
        self.0.scene_copies()
    }
    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        // The clouds' march, reconstruction, light shafts and haze over the opaque scene's depth.
        self.0.after_copies(point, ctx);
    }
}

/// The sea drawn after the opaque scene, reading its copies.
pub struct SeaInWater {
    pub ocean: Ocean,
    bound: Option<(u64, u64)>,
}

impl SeaInWater {
    pub fn new(ocean: Ocean) -> Self {
        Self { ocean, bound: None }
    }
}

impl SceneLayer for SeaInWater {
    fn label(&self) -> &str {
        "ocean"
    }
    fn scene_copies(&self) -> SceneCopies {
        SceneCopies::OPAQUE_COLOR | SceneCopies::OPAQUE_DEPTH
    }
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        if let (Some(color), Some(depth)) = (
            ctx.resources.texture(SCENE_OPAQUE_COLOR),
            ctx.resources.texture(SCENE_OPAQUE_DEPTH),
        ) {
            let versions = (color.version, depth.version);
            if self.bound != Some(versions) {
                self.ocean.set_scene(ctx.device, &color.view, &depth.view);
                self.bound = Some(versions);
            }
        }
        self.ocean.prepare(ctx);
    }
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, _ctx: &DrawContext<'_>) {
        if phase == Phase::Water {
            self.ocean.draw(pass);
        }
    }
}

const SKY_MEAN_WGSL: &str = "#import naval_sky::environment::{SkyEnvironment, env_radiance}
@group(0) @binding(0) var sky_atlas: texture_2d<f32>;
@group(0) @binding(1) var sky_sampler: sampler;
@group(0) @binding(2) var<uniform> sky_env: SkyEnvironment;
@group(1) @binding(0) var<storage, read_write> sky_mean: vec4<f32>;
@compute @workgroup_size(1)
fn main() {
    sky_mean = vec4<f32>(env_radiance(sky_atlas, sky_sampler, sky_env, vec3<f32>(0.0, 1.0, 0.0), 1.0), 0.0);
}
";

/// The sky's mean radiance for the eye adaptation (naval-harness's `SkyMeanLayer`): put it after the sky.
pub struct SkyMean {
    pipeline: PipelineId,
    environment: wgpu::BindGroup,
    output: wgpu::BindGroup,
}

impl SkyMean {
    pub fn new(ctx: &mut RenderContext<'_>, sky: &GameSky) -> Result<Self, ShaderError> {
        if ctx
            .library
            .text("naval-sky/shaders/environment.wgsl")
            .is_none()
        {
            ctx.library.add_source(
                "naval-sky/shaders/environment.wgsl",
                naval_sky::environment::ENVIRONMENT_WGSL,
            )?;
        }
        ctx.library
            .add_source("naval-terrain/examples/sky_mean.wgsl", SKY_MEAN_WGSL)?;
        let layout = ctx
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("sky mean"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let environment = sky.sky.environment();
        let pipeline = ctx.add_pipeline(
            "sky mean",
            "naval-terrain/examples/sky_mean.wgsl",
            Vec::new(),
            compute_recipe(
                vec![environment.bind_group_layout.clone(), layout.clone()],
                "main",
            ),
        )?;
        let buffer = ctx
            .resources
            .buffer(SKY_MEAN_BUFFER)
            .expect("the renderer's sky mean");
        let output = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sky mean"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.buffer.as_entire_binding(),
            }],
        });
        Ok(Self {
            pipeline,
            environment: environment.bind_group.clone(),
            output,
        })
    }
}

impl SceneLayer for SkyMean {
    fn label(&self) -> &str {
        "sky mean"
    }
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let mut pass = ctx
            .encoder
            .begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("sky mean"),
                timestamp_writes: None,
            });
        pass.set_pipeline(ctx.pipelines.compute(self.pipeline));
        pass.set_bind_group(0, &self.environment, &[]);
        pass.set_bind_group(1, &self.output, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    fn draw(&self, _phase: Phase, _pass: &mut wgpu::RenderPass<'_>, _ctx: &DrawContext<'_>) {}
}

/// Web left, native right, and the per-channel mean absolute difference and share of pixels more than 24 levels
/// apart over the whole frame.
pub fn side_by_side(
    web: &std::path::Path,
    native: &std::path::Path,
    out: &std::path::Path,
) -> anyhow::Result<(f64, f64)> {
    let web = image::open(web)?.to_rgba8();
    let native = image::open(native)?.to_rgba8();
    let native = if native.dimensions() == web.dimensions() {
        native
    } else {
        image::imageops::resize(
            &native,
            web.width(),
            web.height(),
            image::imageops::FilterType::Triangle,
        )
    };
    let (w, h) = web.dimensions();
    let mut both = image::RgbaImage::new(w * 2, h);
    image::imageops::replace(&mut both, &web, 0, 0);
    image::imageops::replace(&mut both, &native, i64::from(w), 0);
    both.save(out)?;
    let (mut total, mut far) = (0f64, 0usize);
    for (a, b) in web.pixels().zip(native.pixels()) {
        let mut worst = 0i32;
        for c in 0..3 {
            let d = (i32::from(a[c]) - i32::from(b[c])).abs();
            total += f64::from(d);
            worst = worst.max(d);
        }
        if worst > 24 {
            far += 1;
        }
    }
    let n = f64::from(w * h);
    Ok((total / (n * 3.0), far as f64 / n))
}
