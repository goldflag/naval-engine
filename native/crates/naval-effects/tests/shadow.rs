//! The smoke shadows (the `shadows` look, drawn while the sun's shadow maps are): the opacity shadow map draws (a front
//! map and eight array layers), and a bank of gun smoke read through it is darker on its far side and no
//! brighter anywhere.

use glam::Vec3;
use native_offscreen::{CAPTURE_FORMAT, Capture, Gpu};
use naval_effects::events::ShellInfo;
use naval_effects::{CameraView, CombatEvent, EffectsLayer, Trial};
use naval_render::{
    Camera, FrameInput, FrameTime, RenderOutput, Renderer, SceneLayer, ShadowSettings, Sun,
};

const WIDTH: u32 = 320;
const HEIGHT: u32 = 180;

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
            velocity: [820.0, 30.0, 0.0],
            shell_type: Some("AP".into()),
            explosive_kg: None,
        }),
        ..Default::default()
    }
}

/// The bank a turret leaves 2.5 s after firing, drawn with or without the trial.
fn bank(gpu: &Gpu, shadows: bool) -> (Vec<u8>, naval_effects::shadow::ShadowStats) {
    let mut renderer =
        Renderer::new(&gpu.device, &gpu.queue, WIDTH, HEIGHT, CAPTURE_FORMAT).expect("renderer");
    let mut layer = EffectsLayer::new(&mut renderer.context()).expect("the effect shaders compose");
    layer.effects.trial = Trial {
        shadows,
        ..Trial::default()
    };
    // The smoke's shadows go with the sun's.
    renderer.set_shadow_settings(ShadowSettings::for_quality("medium").unwrap());
    let toward = layer.effects.lighting.sun_direction.as_vec3().normalize();
    renderer.set_sun(Some(Sun::new(toward, Vec3::new(40.0, 14.0, -60.0))));
    let capture = Capture::new(gpu, WIDTH, HEIGHT);
    let camera = Camera::look_at(Vec3::new(-150.0, 40.0, 60.0), Vec3::new(40.0, 14.0, -60.0));
    let view = CameraView::from_uniforms(&camera.uniforms(WIDTH, HEIGHT, FrameTime::default()));
    let events = [shot(1, 0.0), shot(2, 1.2), shot(3, 2.4)];
    layer
        .effects
        .update(1.0 / 60.0, 10.0 / 60.0, &view, &events, &[]);
    for _ in 0..150 {
        layer
            .effects
            .update(1.0 / 60.0, 10.0 / 60.0, &view, &[], &[]);
    }
    // Twice: the maps are made on the first frame with the trial on.
    for frame in 0..2 {
        let layers: &mut [&mut dyn SceneLayer] = &mut [&mut layer];
        renderer
            .render(
                FrameInput {
                    camera: &camera,
                    time: FrameTime::fixed(151 + frame, 1.0 / 60.0),
                },
                layers,
                RenderOutput {
                    view: &capture.view,
                    format: CAPTURE_FORMAT,
                    width: WIDTH,
                    height: HEIGHT,
                },
            )
            .expect("renders");
    }
    (capture.read_rgba(gpu).unwrap(), layer.shadow_stats())
}

#[test]
fn a_smoke_bank_shades_itself() {
    let gpu = Gpu::headless().expect("a GPU");
    let (plain, none) = bank(&gpu, false);
    let (shaded, stats) = bank(&gpu, true);
    assert_eq!(none.casters, 0, "nothing is drawn with the trial off");
    assert!(stats.casters > 5 && stats.texel > 0.0, "{stats:?}");
    let light = |p: &[u8]| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]);
    let (mut darker, mut brighter, mut smoke) = (0, 0, 0);
    for (a, b) in plain.chunks(4).zip(shaded.chunks(4)) {
        if light(a) > 12 {
            smoke += 1;
        }
        if light(b) + 9 < light(a) {
            darker += 1;
        }
        if light(b) > light(a) + 6 {
            brighter += 1;
        }
    }
    assert!(smoke > 500, "only {smoke} pixels of smoke");
    assert!(
        darker * 20 > smoke,
        "only {darker} of {smoke} smoke pixels are shaded"
    );
    assert!(
        darker * 10 < smoke * 9,
        "{darker} of {smoke} smoke pixels are shaded: the sun side is lit as it was"
    );
    assert_eq!(brighter, 0, "shadows only take light away");
}

/// Exercise the production sampler across slice boundaries, the first/last slice, and the map's fading edges.
/// The array's two bilinear reads must preserve the old volume's trilinear optical depth.
#[test]
fn array_slices_preserve_volume_filtering() {
    use wgpu::util::DeviceExt;

    let gpu = Gpu::headless().expect("a GPU");
    let source = include_str!("../shaders/shadow.wgsl");
    let sampling = source
        [source.find("struct SmokeShadow").unwrap()..source.find("// What other smoke").unwrap()]
        .replace("@group(3)", "@group(0)");
    let shader = gpu.device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smoke shadow sampling comparison"),
        source: wgpu::ShaderSource::Wgsl(format!(r#"
{sampling}
@group(0) @binding(0) var volume: texture_3d<f32>;
@group(0) @binding(1) var<storage, read_write> results: array<vec2<f32>>;

// The former 3D texture read, including the nonlinear depth knots and the side fade.
fn original(uv: vec2<f32>, depth: f32) -> f32 {{
    let front = smoke_shadow.params.x - textureSampleLevel(shadow_front, shadow_sampler, uv, 0.0).r;
    let behind = max(depth - front, 0.0);
    let a = smoke_shadow.params.y;
    let b = smoke_shadow.params.z;
    let i = floor((sqrt(a * a + 4.0 * b * behind) - a) / (2.0 * b));
    let along = (behind - (a * i + b * i * i)) / (a + b * (2.0 * i + 1.0));
    let w = (min(i + along, SHADOW_SLICES - 1.0) + 0.5) / SHADOW_SLICES;
    let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
    return textureSampleLevel(volume, shadow_sampler, vec3<f32>(uv, w), 0.0).r * clamp(edge * 16.0, 0.0, 1.0);
}}
@compute @workgroup_size(64) fn probe(@builtin(global_invocation_id) id: vec3<u32>) {{
    if id.x >= 1536u {{ return; }}
    let axis = array<f32, 8>(-0.1, 0.01, 0.04, 0.2, 0.8, 0.96, 0.99, 1.1);
    let uv = vec2<f32>(axis[(id.x / 24u) % 8u], axis[id.x / 192u]);
    let slice = f32(id.x % 24u) * 0.5 - 1.0;
    let depth = 2.5 * slice + 1.8 * slice * slice;
    results[id.x] = vec2<f32>(optical_before(uv, depth), original(uv, depth));
}}
"#).into()),
    });
    let texture = |dimension, size| {
        gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke shadow sampling fixture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    };
    let size = wgpu::Extent3d {
        width: 3,
        height: 3,
        depth_or_array_layers: 8,
    };
    let array = texture(wgpu::TextureDimension::D2, size);
    let volume = texture(wgpu::TextureDimension::D3, size);
    // Finite half floats with gradients along all three axes; both textures receive the same texels.
    let texels: Vec<u16> = (0..8)
        .flat_map(|z| {
            (0..3).flat_map(move |y| (0..3).map(move |x| 0x3800 + z * 0x200 + y * 0x80 + x * 0x20))
        })
        .collect();
    for target in [&array, &volume] {
        gpu.queue.write_texture(
            target.as_image_copy(),
            bytemuck::cast_slice(&texels),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(6),
                rows_per_image: Some(3),
            },
            size,
        );
    }
    let front = texture(
        wgpu::TextureDimension::D2,
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let uniform = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("smoke shadow sampling uniform"),
            contents: bytemuck::cast_slice(&[
                0.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 2.5, 1.8, 1.0,
                0.0, 0.0, 0.0, 0.0,
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let output = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke shadow sampling results"),
        size: 1536 * 8,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke shadow sampling readback"),
        size: output.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let pipeline = gpu
        .device
        .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("smoke shadow sampling comparison"),
            layout: None,
            module: &shader,
            entry_point: Some("probe"),
            compilation_options: Default::default(),
            cache: None,
        });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("smoke shadow sampling comparison"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &volume.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 40,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 41,
                resource: wgpu::BindingResource::TextureView(
                    &front.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 42,
                resource: wgpu::BindingResource::TextureView(
                    &array.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 43,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(24, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, output.size());
    gpu.queue.submit([encoder.finish()]);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let data = readback
        .slice(..)
        .get_mapped_range()
        .expect("mapped results");
    let results: &[[f32; 2]] = bytemuck::cast_slice(&data);
    let (worst_sample, worst_values) = results
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| (a[0] - a[1]).abs().total_cmp(&(b[0] - b[1]).abs()))
        .unwrap();
    let max_error = (worst_values[0] - worst_values[1]).abs();
    assert!(
        results.iter().any(|[value, _]| *value > 0.5),
        "fixture must exercise nonzero optical depths"
    );
    assert!(
        results
            .iter()
            .all(|values| values.iter().all(|v| v.is_finite())),
        "samples must be finite"
    );
    // Texture coordinates may be snapped to eight fractional bits (D3D texture addressing precision).
    // A 1/256 weight step across this fixture's largest slice jump (2.0) is 1/128 optical depth;
    // the shader's explicit mix doesn't snap that weight. Do not require bit-identical reads.
    assert!(
        max_error <= 1.0 / 128.0,
        "array/volume optical depth differs by {max_error} at sample {worst_sample}: {worst_values:?}"
    );
}
