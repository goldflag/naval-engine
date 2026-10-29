//! The smoke's shadow on everything that takes the sun's: a turret's gun smoke darkens the ground under it along the
//! light (through the renderer's soft occluders, which `naval::shadow::sun_shadow` reads), where the smoke itself is
//! not drawn, and with the shadows off it does not.

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::events::ShellInfo;
use naval_effects::{CameraView, CombatEvent, EffectsLayer};
use naval_render::{
    Camera, DrawContext, FrameInput, FrameTime, Phase, PipelineId, RenderContext, RenderOutput,
    Renderer, SceneLayer, ScenePipeline, ShadowSettings, Sun,
};

const SIZE: u32 = 256;

const SHADER: &str = r#"
#import naval::camera::{camera, world_to_clip}
#import naval::shadow::sun_shadow

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

@vertex fn ground_vertex(@builtin(vertex_index) i: u32) -> Out {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[i] * 300.0;
    let world = vec3<f32>(c.x, 0.0, c.y - 60.0);
    var out: Out;
    out.clip = world_to_clip(world);
    out.world = world;
    return out;
}

// The ground's sunlight as grey.
@fragment fn ground_fragment(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(vec3<f32>(0.4 * sun_shadow(in.world, vec3<f32>(0.0, 1.0, 0.0))), 1.0);
}
"#;

struct Ground {
    pipeline: PipelineId,
}

impl Ground {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library
            .add_source("test/sun_shadow.wgsl", SHADER)
            .unwrap();
        let pipeline = ctx
            .add_scene_pipeline(
                "ground",
                "test/sun_shadow.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new("ground_vertex", "ground_fragment")
                },
            )
            .unwrap();
        Self { pipeline }
    }
}

impl SceneLayer for Ground {
    fn label(&self) -> &str {
        "ground"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.pipeline));
            pass.draw(0..6, 0..1);
        }
    }
}

fn shot(sequence: u64, x: f64) -> CombatEvent {
    CombatEvent {
        sequence,
        tick: 10,
        kind: "shot".into(),
        ship_id: "player".into(),
        position: [x, 14.0, -60.0],
        shell: Some(ShellInfo {
            id: 1000 + sequence as i64,
            caliber_m: 0.38,
            velocity: [0.0, 30.0, -820.0],
            shell_type: Some("AP".into()),
            explosive_kg: None,
        }),
        ..Default::default()
    }
}

#[test]
fn gun_smoke_shades_the_ground_under_it_along_the_light() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, SIZE, SIZE, CAPTURE_FORMAT).expect("renderer");
    let mut ground = Ground::new(&mut renderer.context());
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer.effects.trial = naval_effects::Trial::GAME;
    let capture = Capture::new(&gpu, SIZE, SIZE);
    // From above, the light low in the east: the smoke's shadow falls tens of metres west of it.
    let camera = Camera::look_at(Vec3::new(-40.0, 420.0, 20.0), Vec3::new(-40.0, 0.0, -60.0));
    let toward = Vec3::new(1.0, 0.7, 0.0).normalize();
    renderer.set_sun(Some(Sun::new(toward, Vec3::new(-30.0, 0.0, -60.0))));
    let view = CameraView::from_uniforms(&camera.uniforms(SIZE, SIZE, FrameTime::default()));
    let events = [shot(1, 0.0), shot(2, 1.2), shot(3, 2.4)];
    layer
        .effects
        .update(1.0 / 60.0, 10.0 / 60.0, &view, &events, &[]);
    for _ in 0..50 {
        layer
            .effects
            .update(1.0 / 60.0, 10.0 / 60.0, &view, &[], &[]);
    }

    let mut frame = 0;
    let mut draw = |renderer: &mut Renderer, layer: &mut EffectsLayer, ground: &mut Ground| {
        // A few frames, so temporal noise in the casters is drawn more than once.
        for _ in 0..3 {
            frame += 1;
            let layers: &mut [&mut dyn SceneLayer] = &mut [ground, layer];
            renderer
                .render(
                    FrameInput {
                        camera: &camera,
                        time: FrameTime::fixed(frame, 1.0 / 60.0),
                    },
                    layers,
                    RenderOutput {
                        view: &capture.view,
                        format: CAPTURE_FORMAT,
                        width: SIZE,
                        height: SIZE,
                    },
                )
                .expect("renders");
        }
        capture.read_rgba(&gpu).unwrap()
    };
    renderer.set_shadow_settings(ShadowSettings::for_quality("off").unwrap());
    let lit = draw(&mut renderer, &mut layer, &mut ground);
    assert_eq!(layer.shadow_stats().casters, 0, "no maps, no smoke shadows");
    renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
    let shaded = draw(&mut renderer, &mut layer, &mut ground);
    // Every pipeline built, the casters' among them.
    let errors: Vec<_> = renderer
        .pipelines()
        .status()
        .into_iter()
        .filter_map(|status| status.error.map(|error| (status.label, error)))
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let casters = layer.shadow_stats().casters;
    assert!(casters > 5, "only {casters} balls of smoke cast");

    // The bare ground's value without shadows: the most common pixel.
    let mut counts = std::collections::HashMap::new();
    for p in lit.chunks(4) {
        *counts.entry([p[0], p[1], p[2]]).or_insert(0u32) += 1;
    }
    let bare = *counts.iter().max_by_key(|(_, n)| **n).unwrap().0;
    // Pixels where no smoke is drawn (the bare ground without shadows) that the shadows darkened.
    let darkened = lit
        .chunks(4)
        .zip(shaded.chunks(4))
        .filter(|(l, s)| [l[0], l[1], l[2]] == bare && (s[0] as i32) < l[0] as i32 - 12)
        .count();
    assert!(
        darkened > 200,
        "only {darkened} ground pixels in the smoke's shadow (bare ground {bare:?})"
    );
}
