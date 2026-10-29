//! The scene's point lights on a real GPU (`naval::lights`): a ground lit through the clustered index by a star shell
//! 300 m over a 10 m square held 10 m up, which shades the ground under it through the shell's own map while a light
//! that casts no shadow lights it whole; shadows off light it whole too; and a weak light far off reaches none of the
//! ground near the eye.

use crate::common;

use common::{read_texture, texel};
use glam::{Vec3, Vec4};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::lights::{LightCaster, PointLight, PointLights};
use naval_render::{
    Camera, CasterPipeline, DrawContext, FrameInput, FrameReport, FrameTime, Phase, PipelineId,
    RenderContext, RenderOutput, Renderer, SCENE_HDR, SceneLayer, ScenePipeline, ShadowMap,
    ShadowSettings,
};

const SHADER: &str = r#"
#import naval::camera::camera
#import naval::lights::{LIGHT_WORDS, scene_lights, light_cell, light_word, shadowed_point_light}

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

fn corner(i: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    return corners[i];
}

@vertex fn ground_vertex(@builtin(vertex_index) i: u32) -> Out {
    let c = corner(i) * 1500.0;
    let world = vec3<f32>(c.x, 0.0, c.y);
    var out: Out;
    out.clip = camera.view_projection * vec4<f32>(world, 1.0);
    out.world = world;
    return out;
}

// r: the irradiance (luminance) the cell's lights give the ground facing up, g: how many lights its cell holds.
@fragment fn ground_fragment(in: Out) -> @location(0) vec4<f32> {
    let up = vec3<f32>(0.0, 1.0, 0.0);
    var lit = 0.0;
    var count = 0.0;
    let cell = light_cell(in.world);
    for (var word = 0u; word < LIGHT_WORDS; word += 1u) {
        var bits = light_word(cell, word);
        while bits != 0u {
            let index = word * 32u + firstTrailingBit(bits);
            bits &= bits - 1u;
            let light = shadowed_point_light(scene_lights.lights[index], in.world, up);
            lit += dot(light.radiance, vec3<f32>(0.2126, 0.7152, 0.0722)) * max(dot(light.direction, up), 0.0);
            count += 1.0;
        }
    }
    return vec4<f32>(lit, count, 0.0, 1.0);
}

// A 10 x 10 m square 10 m up over the origin, drawn only into the lights' maps.
@vertex fn caster_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let c = corner(i) * 5.0;
    return camera.view_projection * vec4<f32>(c.x, 10.0, c.y, 1.0);
}
"#;

struct Ground {
    ground: PipelineId,
    caster: PipelineId,
}

impl Ground {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/lights.wgsl", SHADER).unwrap();
        let ground = ctx
            .add_scene_pipeline(
                "lit ground",
                "test/lights.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new("ground_vertex", "ground_fragment")
                },
            )
            .unwrap();
        let caster = ctx
            .add_caster_pipeline(
                "light caster",
                "test/lights.wgsl",
                CasterPipeline::new("caster_vertex"),
            )
            .unwrap();
        Self { ground, caster }
    }
}

impl SceneLayer for Ground {
    fn label(&self) -> &str {
        "ground"
    }

    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.ground));
            pass.draw(0..6, 0..1);
        }
    }

    fn draw_light_shadow(
        &self,
        _map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        pass.set_pipeline(ctx.pipelines.render(self.caster));
        pass.draw(0..6, 0..1);
    }
}

const SIZE: u32 = 128;

/// The ground's light and its cell's light count at world points, seen from 120 m over the origin.
fn render(
    settings: ShadowSettings,
    lights: &[PointLight],
    points: &[Vec3],
) -> (Vec<(f32, f32)>, FrameReport) {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, SIZE, SIZE, CAPTURE_FORMAT).unwrap();
    let mut ground = Ground::new(&mut renderer.context());
    renderer.set_shadow_settings(settings);
    let camera = Camera::look_at(Vec3::new(0.0, 120.0, 1.0), Vec3::ZERO);
    renderer.set_point_lights(PointLights::brightest(lights.iter().copied(), camera.eye));
    let capture = Capture::new(&gpu, SIZE, SIZE);
    let report = renderer
        .render(
            FrameInput {
                camera: &camera,
                time: FrameTime::default(),
            },
            &mut [&mut ground],
            RenderOutput {
                view: &capture.view,
                format: CAPTURE_FORMAT,
                width: SIZE,
                height: SIZE,
            },
        )
        .unwrap();
    let hdr = read_texture(
        &gpu,
        &renderer.resources().texture(SCENE_HDR).unwrap().texture,
    );
    let view_projection = camera.projection(1.0) * camera.view();
    let values = points
        .iter()
        .map(|point| {
            let clip = view_projection * Vec4::new(point.x, point.y, point.z, 1.0);
            let ndc = clip.truncate() / clip.w;
            let x = ((ndc.x * 0.5 + 0.5) * SIZE as f32) as u32;
            let y = ((0.5 - ndc.y * 0.5) * SIZE as f32) as u32;
            let t = texel(&hdr, SIZE, 4, x, y);
            (t[0], t[1])
        })
        .collect();
    (values, report)
}

fn star_shell(caster: LightCaster) -> PointLight {
    PointLight {
        position: Vec3::new(0.0, 300.0, 0.0),
        color: Vec3::ONE,
        intensity: 9e4,
        decay: 2.0,
        caster,
        ..Default::default()
    }
}

const UNDER: [Vec3; 3] = [
    Vec3::new(0.0, 0.0, 0.0),
    Vec3::new(3.5, 0.0, -3.5),
    Vec3::new(-3.0, 0.0, 2.0),
];
const BESIDE: [Vec3; 3] = [
    Vec3::new(12.0, 0.0, 0.0),
    Vec3::new(0.0, 0.0, -14.0),
    Vec3::new(-25.0, 0.0, 20.0),
];

#[test]
fn a_star_shell_shades_the_ground_under_a_caster_through_its_map() {
    let points: Vec<Vec3> = UNDER.iter().chain(&BESIDE).copied().collect();
    let (values, report) = render(
        ShadowSettings::for_quality("medium").unwrap(),
        &[star_shell(LightCaster::Overhead)],
        &points,
    );
    assert_eq!(report.shadow_maps, ["light"]);
    // 9 × 10⁴ cd 300 m up: about 1 on the ground below it.
    for (point, (lit, count)) in points.iter().zip(&values) {
        assert_eq!(*count, 1.0, "the light's cell at {point}");
        if UNDER.contains(point) {
            assert!(*lit < 0.05, "shadowed at {point}: {lit}");
        } else {
            assert!(*lit > 0.9 && *lit < 1.1, "lit at {point}: {lit}");
        }
    }
}

#[test]
fn a_light_without_a_map_and_shadows_off_light_it_whole() {
    let points: Vec<Vec3> = UNDER.iter().chain(&BESIDE).copied().collect();
    let (shadowless, report) = render(
        ShadowSettings::for_quality("medium").unwrap(),
        &[star_shell(LightCaster::None)],
        &points,
    );
    assert!(report.shadow_maps.is_empty());
    let (off, report) = render(
        ShadowSettings::for_quality("off").unwrap(),
        &[star_shell(LightCaster::Overhead)],
        &points,
    );
    assert!(report.shadow_maps.is_empty());
    for (point, ((a, _), (b, _))) in points.iter().zip(shadowless.iter().zip(&off)) {
        assert!(*a > 0.9 && *b > 0.9, "lit at {point}: {a}, {b}");
    }
}

#[test]
fn a_weak_light_far_off_reaches_no_cell_near_the_eye() {
    // 1 cd 3 km off reaches 141 m: none of the ground about the origin, which the bright shell lights.
    let far = PointLight {
        position: Vec3::new(3000.0, 10.0, 0.0),
        color: Vec3::ONE,
        intensity: 1.0,
        decay: 2.0,
        ..Default::default()
    };
    let points: Vec<Vec3> = BESIDE.to_vec();
    let (values, _) = render(
        ShadowSettings::for_quality("off").unwrap(),
        &[star_shell(LightCaster::None), far],
        &points,
    );
    for (point, (_, count)) in points.iter().zip(&values) {
        assert_eq!(*count, 1.0, "lights in the cell at {point}");
    }
}
