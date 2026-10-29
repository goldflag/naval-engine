//! The shells as a `naval_render::SceneLayer`: [`ShellVisuals`] published for the frame's camera and drawn in
//! `shaders/shells.wgsl`. Bodies are opaque (the lathe of the web game's `assets/effects/naval/shellGeometry.ts`, detailed or
//! silhouette, a tracer burning red in its base); trails and a tracer's spark and streak are transparent, drawn in that
//! order (the removed web game's creation order, kept); the spark and streak write depth, the trails do not. Last, the
//! helm's heavy rounds' markers, dots painted over the scene. Bodies, sparks, streaks and markers take no temporal
//! history (the web game's `rejectTemporalHistory`).
//!
//! ```ignore
//! let mut shells = ShellLayer::new(&mut renderer.context())?;
//! // each frame, after the combat effects have their lighting and fog:
//! shells.follow(&effects);
//! shells.visuals.update(&shell_states, dt);
//! renderer.render(input, &mut [/* sea, ships, sky, */ &mut shells, &mut effects], output)?;
//! ```

use bytemuck::{Pod, Zeroable, bytes_of, cast_slice};
use naval_render::{
    DrawContext, MotionPipeline, Phase, PipelineId, PrepareContext, RenderContext, SceneLayer,
    ScenePipeline, ShaderError, UploadSink, VertexLayout,
};

use crate::camera::CameraView;
use crate::combat::EffectLighting;
use crate::gpu::{
    GrowBuffer, sampler, sampler_entry, storage_entry, texture_entry, uniform_entry, upload_image,
};
use crate::layer::{EffectFog, EffectsLayer, placeholder_environment};
use crate::math::Rgb;
use crate::shells::{
    BodyInstance, GlowInstance, ShellCounts, ShellFrame, ShellVisuals, StripInstance,
};
use crate::textures::{SpriteMap, effect_texture};

const COMMON: naval_render::ShaderFile = naval_render::shader_file!("shaders/common.wgsl");
const SHELLS: naval_render::ShaderFile = naval_render::shader_file!("shaders/shells.wgsl");
/// A tracer's intensities (linear, against a sunlit white near 2): its spark (about 14,000 cd over its three-calibre
/// sprite, as bright as a large tracer burns), its streak (the same light spread along a frame's flight) and its
/// burning face.
const TRACER_SPARK: f64 = 510.0;
const TRACER_STREAK: f64 = 20.0;
const TRACER_FACE: f64 = 30.0;
/// The marker on the helm's heavy rounds: orange, dark enough by day to keep its colour through the display transform
/// (a brighter amber washes out to beige) and so show against white cloud as on the sea, and the share of that kept
/// at night (where the exposure stays a day's).
const MARKER: u32 = 0xff8c1a;
const MARKER_DAY: f64 = 0.5;
const MARKER_NIGHT: f32 = 0.3;

/// A vertex of the shell lathe (twin: `body_vertex`'s inputs).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ShellVertex {
    /// Calibres, +Y forward.
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Linear vertex colour (the engraved finish).
    pub color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LightingUniform {
    sun_direction: [f32; 4],
    direct: [f32; 4],
    ambient: [f32; 4],
    fog_color: [f32; 4],
    fog: [f32; 4],
    fog_sky: [f32; 4],
    flashes: [[f32; 4]; 4],
    /// The combat effects' clouds in front (`common.wgsl`'s `cloud_visibility`), which the shells do not read.
    clouds: [f32; 4],
    /// The combat effects' trial looks, which the shells do not read.
    trial: [f32; 4],
    /// x the night's lift on what glows (`EffectLighting::lift`): the tracers' burning compositions.
    emission: [f32; 4],
    /// The flashes' colour on the smoke, which the shells do not read.
    flash_color: [f32; 4],
    /// The flashes' own glow's lift (`EffectLighting::flash_glow`), which the shells do not read.
    flash_glow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    glow: [f32; 4],
    streak: [f32; 4],
    trail: [f32; 4],
    face: [f32; 4],
    alpha_test: [f32; 4],
    marker: [f32; 4],
}

/// A tracer's light, the strontium red of its burning composition: the spark (alpha test 1e-5, where its glare is
/// already dark), the streak (0.02), the burning face in the base; and every round's trail, the web game's pale vapour
/// (not tested: it writes no depth). The spark fills its pixels out to about 3 km and fades with the square of the distance beyond (`shells.rs`),
/// so at night it reads several kilometres off, and against a daylit sky only as a faint red point past a kilometre or
/// two.
fn material() -> MaterialUniform {
    let rgba = |color: Rgb, opacity: f32| {
        let [r, g, b] = color.to_f32();
        [r, g, b, opacity]
    };
    MaterialUniform {
        glow: rgba(Rgb::hex(0xff4a1c).scale(TRACER_SPARK), 1.0),
        streak: rgba(Rgb::hex(0xff6a30).scale(TRACER_STREAK), 0.9),
        trail: rgba(Rgb::hex(0xe2ddd0), 1.0),
        face: rgba(Rgb::hex(0xff5a24).scale(TRACER_FACE), 0.0),
        alpha_test: [1e-5, 0.02, 0.0, 0.0],
        marker: rgba(Rgb::hex(MARKER).scale(MARKER_DAY), MARKER_NIGHT),
    }
}

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

impl Mesh {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, detailed: bool) -> Self {
        let (vertices, indices) = shell_geometry(detailed);
        let make = |label, bytes: &[u8], usage| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        Self {
            vertices: make(
                "shell lathe vertices",
                cast_slice(&vertices),
                wgpu::BufferUsages::VERTEX,
            ),
            indices: make(
                "shell lathe indices",
                cast_slice(&indices),
                wgpu::BufferUsages::INDEX,
            ),
            count: indices.len() as u32,
        }
    }
}

/// What the last frame drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShellDrawStats {
    pub bodies: u32,
    pub detailed: u32,
    pub glows: u32,
    pub streaks: u32,
    pub trail_segments: u32,
    pub markers: u32,
}

pub struct ShellLayer {
    /// The CPU side: give it the frame's shells with `visuals.update(&shells, dt)` before rendering.
    pub visuals: ShellVisuals,
    /// The frame's instances and their upload order, written over from frame to frame.
    frame: ShellFrame,
    bodies_order: Vec<BodyInstance>,
    glows_order: Vec<GlowInstance>,
    strips_order: Vec<StripInstance>,
    /// The scene's light on the bodies (the combat effects'; see [`ShellLayer::follow`]).
    pub lighting: EffectLighting,
    pub fog: EffectFog,
    body: PipelineId,
    glow: PipelineId,
    streak: PipelineId,
    trail: PipelineId,
    marker: PipelineId,
    body_motion: PipelineId,
    glow_motion: PipelineId,
    streak_motion: PipelineId,
    marker_motion: PipelineId,
    simple: Mesh,
    detailed: Mesh,
    batch_layout: wgpu::BindGroupLayout,
    /// Group 3: the sky's environment as the combat effects read it (bindings 4 to 6, `EFFECT_SKY` in
    /// `shaders/common.wgsl`), taken from them in [`ShellLayer::follow`] and bound over it in `prepare`.
    sky_layout: wgpu::BindGroupLayout,
    environment: Option<(u64, (wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, bool))>,
    /// The bound group and the environment version it holds (`None`: the black placeholder).
    sky_group: (Option<u64>, wgpu::BindGroup),
    lighting_buffer: wgpu::Buffer,
    lighting_group: wgpu::BindGroup,
    material: wgpu::Buffer,
    tracer_map: wgpu::TextureView,
    sampler: wgpu::Sampler,
    bodies: GrowBuffer,
    glows: GrowBuffer,
    strips: GrowBuffer,
    group: Option<((u64, u64, u64), wgpu::BindGroup)>,
    stats: ShellDrawStats,
}

fn blend(additive: bool) -> wgpu::BlendState {
    let color = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::SrcAlpha,
        dst_factor: if additive {
            wgpu::BlendFactor::One
        } else {
            wgpu::BlendFactor::OneMinusSrcAlpha
        },
        operation: wgpu::BlendOperation::Add,
    };
    let alpha = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: if additive {
            wgpu::BlendFactor::One
        } else {
            wgpu::BlendFactor::OneMinusSrcAlpha
        },
        operation: wgpu::BlendOperation::Add,
    };
    wgpu::BlendState { color, alpha }
}

fn body_vertices() -> VertexLayout {
    VertexLayout {
        stride: std::mem::size_of::<ShellVertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3]
            .to_vec(),
    }
}

impl ShellLayer {
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        crate::layer::register_environment(ctx)?;
        for file in [COMMON, SHELLS] {
            ctx.add_shader(file)?;
        }
        let vf = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let fragment = wgpu::ShaderStages::FRAGMENT;
        let d2 = wgpu::TextureViewDimension::D2;
        let lighting_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shell lighting"),
            entries: &[uniform_entry(0, vf)],
        });
        let batch_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shell batch"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX),
                storage_entry(1, wgpu::ShaderStages::VERTEX),
                storage_entry(2, wgpu::ShaderStages::VERTEX),
                uniform_entry(3, fragment),
                texture_entry(4, fragment, d2, true),
                sampler_entry(5, fragment),
            ],
        });
        let sky_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shell sky"),
            entries: &[
                texture_entry(4, fragment, d2, true),
                sampler_entry(5, fragment),
                uniform_entry(6, fragment),
            ],
        });
        let layouts = vec![lighting_layout.clone(), batch_layout.clone()];
        // The drawn materials read the sky's environment for their sky light and fog (group 3); the motion vectors do
        // not.
        let lit_layouts = vec![
            lighting_layout.clone(),
            batch_layout.clone(),
            sky_layout.clone(),
        ];
        // The sky's environment (group 3) and the renderer's sun shadow maps (group 0), which the bodies' light reads.
        let sky_defs = crate::layer::scene_defs();
        let body = ctx.add_scene_pipeline(
            "shell bodies",
            SHELLS.path,
            ScenePipeline {
                vertex_buffers: vec![body_vertices()],
                bind_group_layouts: lit_layouts.clone(),
                primitive: wgpu::PrimitiveState::default(),
                defs: sky_defs.clone(),
                ..ScenePipeline::new("body_vertex", "body_fragment")
            },
        )?;
        let transparent = |vertex, fragment, additive, depth_write| ScenePipeline {
            bind_group_layouts: lit_layouts.clone(),
            defs: sky_defs.clone(),
            primitive: wgpu::PrimitiveState::default(),
            // Glows and streaks write depth, alpha tested (kept from the web game's `depthWrite: true`; not a product
            // decision). Trails do not: faint at their edges, depth there would cut the clouds and the air's haze
            // behind them into a visible line.
            depth_write,
            blend: Some(blend(additive)),
            ..ScenePipeline::new(vertex, fragment)
        };
        let glow = ctx.add_scene_pipeline(
            "shell glows",
            SHELLS.path,
            transparent("glow_vertex", "glow_fragment", true, true),
        )?;
        let streak = ctx.add_scene_pipeline(
            "shell streaks",
            SHELLS.path,
            transparent("streak_vertex", "streak_fragment", true, true),
        )?;
        let trail = ctx.add_scene_pipeline(
            "shell trails",
            SHELLS.path,
            transparent("trail_vertex", "trail_fragment", false, false),
        )?;
        let marker = ctx.add_scene_pipeline(
            "shell markers",
            SHELLS.path,
            transparent("marker_vertex", "marker_fragment", false, false),
        )?;
        let motion = |vertex, fragment, buffers: Vec<VertexLayout>| MotionPipeline {
            vertex_buffers: buffers,
            bind_group_layouts: layouts.clone(),
            primitive: wgpu::PrimitiveState::default(),
            ..MotionPipeline::new(vertex, fragment)
        };
        let body_motion = ctx.add_motion_pipeline(
            "shell bodies motion",
            SHELLS.path,
            motion("body_vertex", "body_motion", vec![body_vertices()]),
        )?;
        let glow_motion = ctx.add_motion_pipeline(
            "shell glows motion",
            SHELLS.path,
            motion("glow_vertex", "glow_motion", Vec::new()),
        )?;
        let streak_motion = ctx.add_motion_pipeline(
            "shell streaks motion",
            SHELLS.path,
            motion("streak_vertex", "streak_motion", Vec::new()),
        )?;
        let marker_motion = ctx.add_motion_pipeline(
            "shell markers motion",
            SHELLS.path,
            motion("marker_vertex", "marker_motion", Vec::new()),
        )?;
        let lighting_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shell lighting"),
            size: std::mem::size_of::<LightingUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let lighting_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shell lighting"),
            layout: &lighting_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: lighting_buffer.as_entire_binding(),
            }],
        });
        let material_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("shell materials"),
            size: std::mem::size_of::<MaterialUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&material_buffer, 0, bytes_of(&material()));
        let storage = |label| GrowBuffer::new(&device, label, wgpu::BufferUsages::STORAGE, 4096);
        Ok(Self {
            visuals: ShellVisuals::new(),
            frame: ShellFrame::default(),
            bodies_order: Vec::new(),
            glows_order: Vec::new(),
            strips_order: Vec::new(),
            lighting: EffectLighting::default(),
            fog: EffectFog::default(),
            body,
            glow,
            streak,
            trail,
            marker,
            body_motion,
            glow_motion,
            streak_motion,
            marker_motion,
            simple: Mesh::new(&device, &queue, false),
            detailed: Mesh::new(&device, &queue, true),
            batch_layout,
            sky_group: (
                None,
                sky_group(&device, &sky_layout, &placeholder_environment(&device)),
            ),
            sky_layout,
            environment: None,
            lighting_buffer,
            lighting_group,
            material: material_buffer,
            tracer_map: upload_image(
                &device,
                &queue,
                "shell tracer",
                &effect_texture(SpriteMap::Tracer),
            ),
            sampler: sampler(
                &device,
                "shell sprite",
                wgpu::AddressMode::ClampToEdge,
                true,
                1,
            ),
            bodies: storage("shell bodies"),
            glows: storage("shell glows"),
            strips: storage("shell strips"),
            group: None,
            stats: ShellDrawStats::default(),
        })
    }

    /// Takes the combat effects' light and fog, so the shells sit in the same scene.
    pub fn follow(&mut self, effects: &EffectsLayer) {
        self.lighting = effects.effects.lighting;
        self.fog = effects.fog;
        let (version, environment) = effects.environment();
        if self.environment.as_ref().map(|(v, _)| *v) != Some(version) {
            self.environment = Some((version, environment.clone()));
        }
    }

    pub fn stats(&self) -> ShellDrawStats {
        self.stats
    }

    pub fn counts(&self) -> ShellCounts {
        self.visuals.counts()
    }

    /// Whether the sky's environment is bound (rather than the effects' black placeholder).
    fn sky_bound(&self) -> bool {
        self.sky_group.0.is_some() && self.environment.as_ref().is_some_and(|(_, e)| e.3)
    }

    fn lighting_uniform(&self) -> LightingUniform {
        let lighting = &self.lighting;
        let v = |x: glam::DVec3, w: f32| [x.x as f32, x.y as f32, x.z as f32, w];
        let fog = &self.fog;
        LightingUniform {
            sun_direction: v(lighting.sun_direction, lighting.daylight as f32),
            direct: v(lighting.direct, 0.0),
            ambient: v(lighting.ambient, lighting.flash as f32),
            fog_color: [fog.color[0], fog.color[1], fog.color[2], fog.power],
            fog: [
                fog.start,
                fog.end,
                fog.sky_blend_distance,
                f32::from(fog.enabled),
            ],
            fog_sky: [
                fog.sky[0],
                fog.sky[1],
                fog.sky[2],
                f32::from(u8::from(self.sky_bound())),
            ],
            flashes: [[0.0; 4]; 4],
            clouds: [0.0; 4],
            trial: [0.0; 4],
            emission: [
                lighting.lift as f32,
                lighting.flames as f32,
                lighting.glow as f32,
                lighting.flash_smoke as f32,
            ],
            flash_color: [0.0; 4],
            flash_glow: [lighting.flash_glow as f32, 0.0, 0.0, 0.0],
        }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &mut UploadSink<'_>, frame: &ShellFrame) {
        // Detailed bodies first, then the silhouettes; sparks, then markers; streaks, then trail segments: each drawn
        // as a range.
        let bodies = &mut self.bodies_order;
        bodies.clear();
        bodies.extend_from_slice(&frame.detailed);
        bodies.extend_from_slice(&frame.bodies);
        let glows = &mut self.glows_order;
        glows.clear();
        glows.extend_from_slice(&frame.glows);
        glows.extend_from_slice(&frame.markers);
        let strips = &mut self.strips_order;
        strips.clear();
        strips.extend_from_slice(&frame.streaks);
        strips.extend_from_slice(&frame.trails);
        self.bodies
            .write_to(device, queue, cast_slice(&self.bodies_order));
        self.glows
            .write_to(device, queue, cast_slice(&self.glows_order));
        self.strips
            .write_to(device, queue, cast_slice(&self.strips_order));
        let versions = (self.bodies.version, self.glows.version, self.strips.version);
        if self
            .group
            .as_ref()
            .is_none_or(|(built, _)| *built != versions)
        {
            let entries = [
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bodies.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.glows.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.strips.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.material.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&self.tracer_map),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ];
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("shell batch"),
                layout: &self.batch_layout,
                entries: &entries,
            });
            self.group = Some((versions, group));
        }
        self.stats = ShellDrawStats {
            bodies: frame.bodies.len() as u32,
            detailed: frame.detailed.len() as u32,
            glows: frame.glows.len() as u32,
            streaks: frame.streaks.len() as u32,
            trail_segments: frame.trails.len() as u32,
            markers: frame.markers.len() as u32,
        };
    }

    fn draw_bodies(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
        pipeline: PipelineId,
    ) {
        let s = self.stats;
        if s.bodies + s.detailed == 0 {
            return;
        }
        pass.set_pipeline(ctx.pipelines.render(pipeline));
        for (mesh, instances) in [
            (&self.detailed, 0..s.detailed),
            (&self.simple, s.detailed..s.detailed + s.bodies),
        ] {
            if instances.is_empty() {
                continue;
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.count, 0, instances);
        }
    }

    fn bind(&self, pass: &mut wgpu::RenderPass<'_>) -> bool {
        let Some((_, group)) = &self.group else {
            return false;
        };
        pass.set_bind_group(1, &self.lighting_group, &[]);
        pass.set_bind_group(2, group, &[]);
        true
    }
}

impl SceneLayer for ShellLayer {
    fn label(&self) -> &str {
        "shells"
    }

    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        let camera = CameraView::from_uniforms(ctx.camera);
        let mut frame = std::mem::take(&mut self.frame);
        self.visuals.publish_into(&camera, &mut frame);
        let device = ctx.device.clone();
        self.upload(&device, &mut UploadSink::Belt(ctx.uploads), &frame);
        self.frame = frame;
        if let Some((version, environment)) = &self.environment
            && self.sky_group.0 != Some(*version)
        {
            self.sky_group = (
                Some(*version),
                sky_group(&device, &self.sky_layout, environment),
            );
        }
        ctx.uploads
            .write_buffer(&self.lighting_buffer, 0, bytes_of(&self.lighting_uniform()));
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.bind(pass) {
            return;
        }
        pass.set_bind_group(3, &self.sky_group.1, &[]);
        let s = self.stats;
        match phase {
            Phase::Opaque => self.draw_bodies(pass, ctx, self.body),
            Phase::Transparent => {
                if s.trail_segments > 0 {
                    pass.set_pipeline(ctx.pipelines.render(self.trail));
                    pass.draw(0..6, s.streaks..s.streaks + s.trail_segments);
                }
                if s.glows > 0 {
                    pass.set_pipeline(ctx.pipelines.render(self.glow));
                    pass.draw(0..6, 0..s.glows);
                }
                if s.streaks > 0 {
                    pass.set_pipeline(ctx.pipelines.render(self.streak));
                    pass.draw(0..6, 0..s.streaks);
                }
                if s.markers > 0 {
                    pass.set_pipeline(ctx.pipelines.render(self.marker));
                    pass.draw(0..6, s.glows..s.glows + s.markers);
                }
            }
            Phase::Water | Phase::Sky => {}
        }
    }

    fn draw_motion(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if !self.bind(pass) {
            return;
        }
        let s = self.stats;
        self.draw_bodies(pass, ctx, self.body_motion);
        if s.glows > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.glow_motion));
            pass.draw(0..6, 0..s.glows);
        }
        if s.streaks > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.streak_motion));
            pass.draw(0..6, 0..s.streaks);
        }
        if s.markers > 0 {
            pass.set_pipeline(ctx.pipelines.render(self.marker_motion));
            pass.draw(0..6, s.glows..s.glows + s.markers);
        }
    }
}

/// `shellGeometry(detailed)` (the web game's `assets/effects/naval/shellGeometry.ts`, since removed: see the git
/// history) as three's `LatheGeometry` builds it: the original illustrative projectile in calibres, +Y forward, with its
/// engraved vertex finish. Detailed: 57 profile points × 48 segments; the silhouette: 14 points × 12.
pub fn shell_geometry(detailed: bool) -> (Vec<ShellVertex>, Vec<u32>) {
    let mut profile: Vec<[f64; 2]> = vec![
        [0.0, -1.88],
        [0.15, -1.88],
        [0.17, -1.92],
        [0.17, -2.05],
        [0.36, -2.05],
        [0.43, -2.01],
        [0.48, -1.88],
        [0.49, -1.66],
        [0.5, -1.64],
        [0.515, -1.61],
        [0.515, -1.55],
        [0.503, -1.54],
        [0.503, -1.51],
        [0.515, -1.5],
        [0.515, -1.44],
        [0.5, -1.41],
        [0.49, -1.39],
        [0.49, -1.26],
        [0.515, -1.24],
        [0.515, -1.18],
        [0.503, -1.17],
        [0.503, -1.14],
        [0.515, -1.13],
        [0.515, -1.07],
        [0.49, -1.04],
        [0.49, 0.45],
        [0.5, 0.48],
        [0.5, 0.63],
        [0.488, 0.66],
        [0.488, 0.73],
        [0.48, 0.75],
        [0.48, 0.78],
        [0.488, 0.8],
    ];
    for i in 1..=24 {
        let t = f64::from(i) / 24.0;
        profile.push([0.488 * (1.0 - t.powf(1.65)), 0.8 + 1.55 * t]);
    }
    let points: Vec<[f64; 2]> = if detailed {
        profile
    } else {
        let mut silhouette = vec![
            [0.0, -2.05],
            [0.36, -2.05],
            [0.48, -1.88],
            [0.49, -1.64],
            [0.51, -1.61],
            [0.51, -1.07],
            [0.49, -1.04],
            [0.49, 0.8],
        ];
        silhouette.extend(
            profile[32..]
                .iter()
                .enumerate()
                .filter(|(i, _)| i % 4 == 3)
                .map(|(_, p)| *p),
        );
        silhouette
    };
    let segments = if detailed { 48 } else { 12 };
    lathe(&points, segments)
}

/// three r185's `LatheGeometry(points, segments)` (phi 0 … 2π) with the shell's vertex colours.
fn lathe(points: &[[f64; 2]], segments: u32) -> (Vec<ShellVertex>, Vec<u32>) {
    // The profile's normals: each point's the average of its two segments' (the ends take their one).
    let mut normals = Vec::with_capacity(points.len());
    let mut previous = [0.0f64; 2];
    for j in 0..points.len() {
        let normal = if j == 0 {
            let (dx, dy) = (points[1][0] - points[0][0], points[1][1] - points[0][1]);
            previous = [dy, -dx];
            previous
        } else if j == points.len() - 1 {
            previous
        } else {
            let (dx, dy) = (
                points[j + 1][0] - points[j][0],
                points[j + 1][1] - points[j][1],
            );
            let current = [dy, -dx];
            let sum = [current[0] + previous[0], current[1] + previous[1]];
            previous = current;
            sum
        };
        let length = normal[0].hypot(normal[1]).max(1e-12);
        normals.push([normal[0] / length, normal[1] / length]);
    }
    let [steel, cap, copper, groove, recess, brass] =
        [0x6c7779, 0x8c9492, 0xb77b46, 0x68442d, 0x292d2b, 0xb7a16f].map(Rgb::hex);
    let mut vertices = Vec::with_capacity(points.len() * (segments as usize + 1));
    for i in 0..=segments {
        let phi = f64::from(i) / f64::from(segments) * std::f64::consts::TAU;
        let (sin, cos) = phi.sin_cos();
        for (point, normal) in points.iter().zip(&normals) {
            let (x, y, z) = (point[0] * sin, point[1], point[0] * cos);
            let band = (y > -1.65 && y < -1.4) || (y > -1.25 && y < -1.05);
            let radius = x.hypot(z);
            let angle = x.atan2(z);
            let base = if band {
                if radius < 0.51 { groove } else { copper }
            } else if y < -1.87 && radius < 0.18 {
                recess
            } else if y < -2.02 {
                brass
            } else if y > 0.77 {
                cap
            } else {
                steel
            };
            // Fine longitudinal engraving in the copper and subtle machining on the steel.
            let finish = if band {
                0.88 + 0.12 * (angle * 24.0 + y * 3.0).cos()
            } else {
                0.96 + 0.04 * (y * 180.0).sin()
            };
            vertices.push(ShellVertex {
                position: [x as f32, y as f32, z as f32],
                normal: [
                    (normal[0] * sin) as f32,
                    normal[1] as f32,
                    (normal[0] * cos) as f32,
                ],
                color: base.scale(finish).to_f32(),
            });
        }
    }
    let rows = points.len() as u32;
    let mut indices = Vec::with_capacity((segments * (rows - 1) * 6) as usize);
    for i in 0..segments {
        for j in 0..rows - 1 {
            let base = j + i * rows;
            let (a, b, c, d) = (base, base + rows, base + rows + 1, base + 1);
            indices.extend_from_slice(&[a, b, d, c, d, b]);
        }
    }
    (vertices, indices)
}

/// Group 3 over the sky's environment (or the effects' black placeholder): atlas, sampler, uniforms.
fn sky_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    (view, sampler, uniforms, _): &(wgpu::TextureView, wgpu::Sampler, wgpu::Buffer, bool),
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("shell sky"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: uniforms.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lathe_has_threes_counts_and_the_rounds_colours() {
        let (vertices, indices) = shell_geometry(true);
        assert_eq!(vertices.len(), 57 * 49);
        assert_eq!(indices.len(), 48 * 56 * 6);
        let (vertices, indices) = shell_geometry(false);
        assert_eq!(vertices.len(), 14 * 13);
        assert_eq!(indices.len(), 12 * 13 * 6);
        // The base is brass, the driving bands copper, the nose the cap's grey.
        let (vertices, _) = shell_geometry(true);
        let at = |y: f32| {
            vertices
                .iter()
                .find(|v| (v.position[1] - y).abs() < 1e-4)
                .unwrap()
        };
        let brass = Rgb::hex(0xb7a16f).to_f32();
        // The heel's outer ring (the tracer cup inside it is the dark recess).
        let base = vertices
            .iter()
            .find(|v| {
                (v.position[1] + 2.05).abs() < 1e-4 && v.position[0].hypot(v.position[2]) > 0.3
            })
            .unwrap()
            .color;
        assert!((base[0] - brass[0] * 0.96).abs() < 0.03, "{base:?}");
        let band = at(-1.55).color;
        assert!(band[0] > band[2] * 2.0, "copper is red: {band:?}");
        let tip = vertices
            .iter()
            .map(|v| v.position[1])
            .fold(f32::MIN, f32::max);
        assert!(
            (tip - 2.35).abs() < 1e-5,
            "the ogive ends 2.35 calibres forward: {tip}"
        );
        // Normals are unit and point outward along the body.
        let side = at(0.45);
        let n = side.normal;
        assert!(((n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - 1.0).abs() < 1e-4);
        assert!(n[0] * side.position[0] + n[2] * side.position[2] > 0.0);
    }
}
