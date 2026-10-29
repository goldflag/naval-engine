//! Sun shadows on a real GPU: a square caster 10 m up throws its shadow where
//! the light says, `naval::shadow` reads it through both filters and at part
//! intensity, and shadows off leave everything lit. A second square 900 m off,
//! beyond the wide map, shades the ground and the sea through a view map on
//! every map setting, and a third beside it, outside the hull that view map is
//! fitted to (as the land is), shades the ground but not the sea.

use crate::common;

use common::{read_texture, texel};
use glam::{Vec3, Vec4};
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_render::{
    Camera, CasterPipeline, CasterSphere, DrawContext, FrameInput, FrameReport, FrameTime,
    LandView, Phase, PipelineId, RenderContext, RenderOutput, Renderer, SCENE_HDR, SceneLayer,
    ScenePipeline, ShadowMap, ShadowSettings, Sun,
};

const SHADER: &str = r#"
#import naval::camera::camera
#import naval::shadow::{sun_shadow, shadow_near_share, water_sun_shadow}

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

@fragment fn ground_fragment(in: Out) -> @location(0) vec4<f32> {
    return vec4<f32>(sun_shadow(in.world, vec3<f32>(0.0, 1.0, 0.0)), shadow_near_share(in.world),
        water_sun_shadow(in.world, 2u), 1.0);
}

// Two 10 x 10 m squares, 10 m up, drawn only into the shadow maps: one over the origin, one 900 m
// off along x. A third, like the land, stands 50 m sunward of the far one and 20 m to its side,
// outside the hull the far square's view map is fitted to.
@vertex fn caster_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let square = i / 6u;
    let c = corner(i % 6u) * 5.0;
    var centre = vec3<f32>(f32(min(square, 1u)) * 900.0, 10.0, 0.0);
    if square == 2u {
        centre += vec3<f32>(0.0, 0.0, 20.0) + normalize(vec3<f32>(0.3, 1.0, 0.2)) * 50.0;
    }
    return camera.view_projection * vec4<f32>(centre + vec3<f32>(c.x, 0.0, c.y), 1.0);
}
"#;

struct Ground {
    ground: PipelineId,
    caster: PipelineId,
}

impl Ground {
    fn new(ctx: &mut RenderContext<'_>) -> Self {
        ctx.library.add_source("test/shadows.wgsl", SHADER).unwrap();
        let ground = ctx
            .add_scene_pipeline(
                "ground",
                "test/shadows.wgsl",
                ScenePipeline {
                    primitive: wgpu::PrimitiveState::default(),
                    ..ScenePipeline::new("ground_vertex", "ground_fragment")
                },
            )
            .unwrap();
        let caster = ctx
            .add_caster_pipeline(
                "square caster",
                "test/shadows.wgsl",
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

    fn draw_shadow(
        &self,
        _map: &ShadowMap,
        pass: &mut wgpu::RenderPass<'_>,
        ctx: &DrawContext<'_>,
    ) {
        pass.set_pipeline(ctx.pipelines.render(self.caster));
        pass.draw(0..18, 0..1);
    }
}

const SIZE: u32 = 128;

fn toward_light() -> Vec3 {
    Vec3::new(0.3, 1.0, 0.2).normalize()
}

/// The ground's shadow term at world points, and the frame's report.
fn render(settings: ShadowSettings, points: &[Vec3]) -> (Vec<(f32, f32)>, FrameReport) {
    let (values, report) = render_at(
        settings,
        Sun::new(toward_light(), Vec3::ZERO),
        Vec3::ZERO,
        points,
    );
    (
        values
            .iter()
            .map(|&(shadow, near, _)| (shadow, near))
            .collect(),
        report,
    )
}

/// The ground's shadow term, the near map's share and the sea's shadow (Medium water shadows) at
/// world points, seen from 120 m over `look`, and the frame's report.
fn render_at(
    settings: ShadowSettings,
    sun: Sun,
    look: Vec3,
    points: &[Vec3],
) -> (Vec<(f32, f32, f32)>, FrameReport) {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let mut renderer = Renderer::new(&gpu.device, &gpu.queue, SIZE, SIZE, CAPTURE_FORMAT).unwrap();
    let mut ground = Ground::new(&mut renderer.context());
    renderer.set_shadow_settings(settings);
    renderer.set_sun(Some(sun));
    let camera = Camera::look_at(look + Vec3::new(0.0, 120.0, 1.0), look);
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
            (t[0], t[1], t[2])
        })
        .collect();
    (values, report)
}

/// The caster's shadow on the ground: its square moved away from the light by
/// 10 m × the light's slope, a square from (−8, −7) to (2, 3).
fn shadow_centre() -> Vec3 {
    let t = toward_light();
    Vec3::new(-10.0 * t.x / t.y, 0.0, -10.0 * t.z / t.y)
}

const INSIDE: [Vec3; 3] = [
    Vec3::new(-3.0, 0.0, -2.0),
    Vec3::new(-6.5, 0.0, -5.5),
    Vec3::new(0.5, 0.0, 1.5),
];
const OUTSIDE: [Vec3; 4] = [
    Vec3::new(4.0, 0.0, -2.0),
    Vec3::new(-10.0, 0.0, -2.0),
    Vec3::new(-3.0, 0.0, 6.0),
    Vec3::new(30.0, 0.0, 30.0),
];

#[test]
fn the_caster_shadows_where_the_light_says() {
    assert!((shadow_centre() - Vec3::new(-3.0, 0.0, -2.0)).length() < 1e-4);
    let points: Vec<Vec3> = INSIDE.iter().chain(&OUTSIDE).copied().collect();
    let (values, report) = render_at(
        ShadowSettings::with_map_size(2048),
        Sun::new(toward_light(), Vec3::ZERO),
        Vec3::ZERO,
        &points,
    );
    assert_eq!(report.shadow_maps, ["near", "wide"]);
    for (point, (shadow, near, sea)) in points.iter().zip(&values) {
        let inside = INSIDE.contains(point);
        if inside {
            assert!(*shadow < 0.02, "shadowed at {point}: {shadow}");
            assert!(*sea < 0.02, "sea shadowed at {point}: {sea}");
        } else {
            assert!(*shadow > 0.98, "lit at {point}: {shadow}");
            assert!(*sea > 0.98, "sea lit at {point}: {sea}");
        }
        assert!(*near > 0.99, "inside the near sphere at {point}: {near}");
    }
}

#[test]
fn the_single_comparison_and_part_intensity_agree() {
    let points: Vec<Vec3> = INSIDE.iter().chain(&OUTSIDE).copied().collect();
    let mut settings = ShadowSettings::with_map_size(1024);
    settings.soft = false;
    settings.intensity = 0.4;
    let (values, report) = render(settings, &points);
    assert_eq!(
        report.shadow_maps,
        ["near", "wide"],
        "no casters asked for, no view maps"
    );
    for (point, (shadow, _)) in points.iter().zip(&values) {
        let expected = if INSIDE.contains(point) { 0.6 } else { 1.0 };
        assert!(
            (shadow - expected).abs() < 0.02,
            "at {point}: {shadow}, expected {expected}"
        );
    }
}

#[test]
fn shadows_off_leave_everything_lit() {
    let points: Vec<Vec3> = INSIDE.iter().chain(&OUTSIDE).copied().collect();
    let (values, report) = render(ShadowSettings::default(), &points);
    assert!(report.shadow_maps.is_empty());
    assert!(values.iter().all(|(shadow, _)| *shadow == 1.0));
}

/// The far square, 900 m off along x: well beyond the wide map's 380 m.
const FAR: Vec3 = Vec3::new(900.0, 0.0, 0.0);

#[test]
fn a_hull_beyond_the_wide_map_shades_the_sea_through_a_view_map() {
    // The square over the origin is in the wide map; the far one only in view 1, which the sun's
    // casters ask for.
    let mut sun = Sun::new(toward_light(), Vec3::ZERO);
    sun.casters = vec![CasterSphere {
        center: FAR + Vec3::new(0.0, 10.0, 0.0),
        radius: 8.0,
    }];
    let inside: Vec<Vec3> = INSIDE.iter().map(|&p| p + FAR).collect();
    let outside: Vec<Vec3> = OUTSIDE.iter().map(|&p| p + FAR).collect();
    let mut points: Vec<Vec3> = inside.iter().chain(&outside).copied().collect();
    // Under the third square, the land's stand-in: in view 1 but not among its hull.
    let t = toward_light();
    let plate = FAR + Vec3::new(0.0, 10.0, 20.0) + t * 50.0;
    let under_plate = plate - t * (plate.y / t.y);
    points.push(under_plate);
    // A small wide map (the port's narrowest), which also keeps the near map, fitted where the
    // camera looks at the origin's distance, far from the square's shadow.
    let settings = |size: u32| ShadowSettings {
        wide_half_width: 60.0,
        ..ShadowSettings::with_map_size(size)
    };
    for size in [1024, 2048, 4096] {
        let (values, report) = render_at(settings(size), sun.clone(), FAR, &points);
        assert_eq!(
            report.shadow_maps,
            ["near", "wide", "view 1"],
            "{size}: the far square takes view 1"
        );
        for (point, (shadow, _, sea)) in points.iter().zip(&values) {
            if inside.contains(point) {
                assert!(
                    *shadow < 0.02,
                    "{size}: ground shadowed at {point}: {shadow}"
                );
                assert!(*sea < 0.02, "{size}: sea shadowed at {point}: {sea}");
            } else if *point == under_plate {
                // The ground takes every caster in the map; the sea only the hull's.
                assert!(*shadow < 0.02, "{size}: ground under the plate: {shadow}");
                assert!(*sea > 0.98, "{size}: sea under the plate: {sea}");
            } else {
                assert!(*shadow > 0.98, "{size}: ground lit at {point}: {shadow}");
                assert!(*sea > 0.98, "{size}: sea lit at {point}: {sea}");
            }
        }
    }
    // Without the view maps the far square's shadow is gone from both: no map covers it.
    let off = ShadowSettings {
        view_maps: false,
        ..settings(2048)
    };
    let (values, report) = render_at(off, sun, FAR, &points);
    assert_eq!(report.shadow_maps, ["near", "wide"]);
    assert!(
        values
            .iter()
            .all(|&(shadow, _, sea)| shadow == 1.0 && sea == 1.0)
    );
}

#[test]
fn the_land_maps_shade_the_ground_and_the_sea_beyond_the_wide_map() {
    // Over the far square with no hull to fit a view map to: on High and Ultra the land maps hold the camera's view
    // of the land, and its casters shade the ground and the sea there.
    let mut sun = Sun::new(toward_light(), Vec3::ZERO);
    sun.land = Some(LandView {
        top: 20.0,
        nearest: 0.0,
    });
    let inside: Vec<Vec3> = INSIDE.iter().map(|&p| p + FAR).collect();
    let outside: Vec<Vec3> = OUTSIDE.iter().map(|&p| p + FAR).collect();
    let points: Vec<Vec3> = inside.iter().chain(&outside).copied().collect();
    let settings = |land_maps: bool| ShadowSettings {
        wide_half_width: 60.0,
        land_maps,
        ..ShadowSettings::with_map_size(2048)
    };
    let (values, report) = render_at(settings(true), sun.clone(), FAR, &points);
    assert_eq!(&report.shadow_maps[..3], ["near", "wide", "land 1"]);
    for (point, (shadow, _, sea)) in points.iter().zip(&values) {
        if inside.contains(point) {
            assert!(*shadow < 0.02, "ground shadowed at {point}: {shadow}");
            assert!(*sea < 0.02, "sea shadowed at {point}: {sea}");
        } else {
            assert!(*shadow > 0.98, "ground lit at {point}: {shadow}");
            assert!(*sea > 0.98, "sea lit at {point}: {sea}");
        }
    }
    // Off (Medium), or with no land about the camera, no map holds the far square: all lit, as before.
    let mut no_land = sun.clone();
    no_land.land = None;
    for (settings, sun) in [(settings(false), sun), (settings(true), no_land)] {
        let (values, report) = render_at(settings, sun, FAR, &points);
        assert_eq!(report.shadow_maps, ["near", "wide"]);
        assert!(
            values
                .iter()
                .all(|&(shadow, _, sea)| shadow == 1.0 && sea == 1.0)
        );
    }
}
