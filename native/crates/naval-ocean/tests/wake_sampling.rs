//! Sample the real wake shader: side wash must follow the drawn hull through heave, pitch and roll.
use glam::{DMat4, DQuat, DVec3, EulerRot, Vec3};
use native_offscreen::Gpu;
use naval_ocean::{
    hull_profile::SimpleHull,
    shaders::{self, Entry},
    ship_wake::{ShipWake, WakeCamera, WakeInputs, WakeShip},
};
use naval_render::{Camera, FrameTime};
use std::borrow::Cow;
use wgpu::util::DeviceExt;

const SAMPLE: &str = r#"
#import naval_ocean::wake::{wake_height, wake_surface}
@group(2) @binding(0) var<storage, read> points: array<vec4<f32>>;
@group(2) @binding(1) var<storage, read_write> samples: array<vec4<f32>>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= arrayLength(&points) { return; }
    let p = points[id.x];
    samples[id.x] = vec4<f32>(wake_height(p.xz), wake_surface(p.xz, p.xyz, 0.05).foam, 0.0, 0.0);
}
"#;
fn sample(gpu: &Gpu, wake: &ShipWake, camera: &Camera, points: &[[f32; 4]]) -> Vec<[f32; 4]> {
    let d = &gpu.device;
    let module = shaders::naga_module(
        Entry {
            path: "wake-probe.wgsl",
            source: SAMPLE,
        },
        &[],
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Naga(Cow::Owned(module)),
    });
    let uniform = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let camera_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[uniform(0)],
    });
    let mut entries = vec![uniform(5)];
    for binding in [6, 7, 9, 10] {
        entries.push(wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        });
    }
    entries.push(wgpu::BindGroupLayoutEntry {
        binding: 8,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    });
    let wake_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &entries,
    });
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let points_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: None,
        entries: &[storage(0, true), storage(1, false)],
    });
    let layout = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[
            Some(&camera_layout),
            Some(&wake_layout),
            Some(&points_layout),
        ],
        immediate_size: 0,
    });
    let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: Some(&layout),
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let camera_data = camera.uniforms(1280, 800, FrameTime::fixed(0, 1.0 / 60.0));
    let camera_buffer = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::bytes_of(&camera_data),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let point_buffer = d.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytemuck::cast_slice(points),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (points.len() * 16) as u64;
    let output = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = d.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let group0 = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &camera_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }],
    });
    let b = wake.bindings();
    let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let group1 = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &wake_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 5,
                resource: b.uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(b.displays[0]),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(b.displays[1]),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(b.trail_atlas),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: wgpu::BindingResource::TextureView(b.torpedo_field),
            },
        ],
    });
    let group2 = d.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &points_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: point_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: output.as_entire_binding(),
            },
        ],
    });
    let mut encoder = d.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group0, &[]);
        pass.set_bind_group(1, &group1, &[]);
        pass.set_bind_group(2, &group2, &[]);
        pass.dispatch_workgroups((points.len() as u32).div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &read, 0, size);
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    bytemuck::cast_slice(&slice.get_mapped_range().unwrap()).to_vec()
}

#[test]
fn side_wash_follows_the_drawn_hull() {
    let gpu = Gpu::headless().expect("a GPU");
    // A vertical parallel body through every sampled level: its side is the plane local x = beam/2.
    let mut profile = SimpleHull::warship(250.0, 36.0, 15.0).profile();
    let stations = naval_ocean::hull_profile::PROFILE_STATIONS;
    let levels = naval_ocean::hull_profile::PROFILE_LEVELS;
    let middle = (levels - 1) / 2 * stations;
    // Extrude the waterline vertically; SimpleHull's normal above-water flare is irrelevant to this pose test.
    for level in 0..levels {
        for station in 0..stations {
            profile.starboard[level * stations + station] = profile.starboard[middle + station];
            profile.port[level * stations + station] = profile.port[middle + station];
        }
    }
    for (position, heading, pitch, roll) in [
        ([0.0, 0.0, 0.0], 0.0_f64, 0.0_f64, 0.0_f64),
        ([50.0, 0.8, -25.0], 0.4, 0.04, 0.2),
        ([-25.0, 0.8, 50.0], -0.7, -0.04, -0.2),
    ] {
        let mut wake = ShipWake::new(&gpu.device, 0, 0.1);
        // Isolate wash from the bow breaker, the contact line and the Kelvin waves.
        wake.bow_waves.tuning.crest = 0.0;
        wake.bow_waves.tuning.kelvin_shading = 0.0;
        wake.bow_waves.tuning.breakup = 0.0;
        wake.hull_foam.tuning.rest = 0.0;
        wake.hull_foam.tuning.sea = 0.0;
        wake.hull_foam.tuning.underway = 0.0;
        wake.hull_foam.tuning.bow = 0.0;
        let ship = WakeShip {
            id: "hull",
            position,
            heading,
            pitch,
            roll,
            speed: 15.0,
            length: 250.0,
            beam: 36.0,
            draft: 15.0,
            center_x: 0.0,
            center_z: 0.0,
            forward_speed: 15.0,
            reverse_speed: 10.0,
            profile: Some(&profile),
            oil_leak_m3s: 0.0,
        };
        let base = DVec3::new(position[0], 0.0, position[2]);
        let right = DVec3::new(heading.cos(), 0.0, heading.sin());
        let camera = Camera::look_at(
            (base + right * 80.0 + DVec3::Y * 40.0).as_vec3(),
            base.as_vec3(),
        );
        wake.update(&WakeInputs {
            ships: &[ship],
            dt: 1.0 / 60.0,
            splashes: &[],
            torpedoes: &[],
            sea_height: 0.0,
            camera: WakeCamera {
                eye: camera.eye.to_array().map(f64::from),
                forward: [-1.0, 0.0, 0.0],
                fov_y: 52_f64.to_radians(),
                zoom: 1.0,
                drawing_height: 800.0,
            },
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        wake.encode(&gpu.device, &gpu.queue, &mut encoder);
        gpu.queue.submit([encoder.finish()]);
        let points: Vec<[f32; 4]> = (0..241)
            .map(|i| {
                let p = base + right * (14.0 + i as f64 * 0.05);
                [p.x as f32, p.y as f32, p.z as f32, 0.0]
            })
            .collect();
        let values = sample(&gpu, &wake, &camera, &points);
        let (peak, white) = values
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a[1].total_cmp(&b[1]))
            .unwrap();
        let at = 14.0 + peak as f64 * 0.05;
        let rotation = DQuat::from_euler(EulerRot::YXZ, -heading, pitch, roll);
        let inverse = DMat4::from_rotation_translation(rotation, DVec3::from(position)).inverse();
        let desired = 18.0 + wake.bow_waves.tuning.wash_width * 0.5;
        let expected =
            (desired - inverse.transform_point3(base).x) / inverse.transform_vector3(right).x;
        assert!(
            white[1] > 0.3,
            "the narrow wash band disappeared: {white:?}"
        );
        assert!(
            (at - expected).abs() < 0.12,
            "wash at {at:.2} m, hull-side band at {expected:.2} m; pose {position:?} {heading} {pitch} {roll}"
        );
    }
}

#[test]
fn kelvin_interior_height_takes_the_attenuation_its_slopes_take() {
    let gpu = Gpu::headless().expect("a GPU");
    let mut wake = ShipWake::new(&gpu.device, 0, 0.1);
    let camera = Camera::look_at(Vec3::new(120.0, 40.0, 0.0), Vec3::ZERO);
    let ship = WakeShip {
        id: "destroyer",
        position: [0.0; 3],
        heading: 0.0,
        pitch: 0.0,
        roll: 0.0,
        speed: 18.0,
        length: 115.0,
        beam: 12.0,
        draft: 3.8,
        center_x: 0.0,
        center_z: 0.0,
        forward_speed: 18.0,
        reverse_speed: 10.0,
        profile: None,
        oil_leak_m3s: 0.0,
    };
    wake.update(&WakeInputs {
        ships: &[ship],
        dt: 1.0 / 60.0,
        splashes: &[],
        torpedoes: &[],
        sea_height: 0.0,
        camera: WakeCamera {
            eye: [120.0, 40.0, 0.0],
            forward: [-1.0, 0.0, 0.0],
            fov_y: 52_f64.to_radians(),
            zoom: 1.0,
            drawing_height: 800.0,
        },
    });
    // Isolate the interior shape from the CPU amplitude limits and the bow crest.
    wake.bow_waves.slots[0][1][1] = 1.0;
    wake.bow_waves.slots[0][1][2] = 0.0;
    wake.bow_waves.mesh_growth = 0.0;
    let stem = wake.bow_waves.slots[0][0][1];
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    wake.encode(&gpu.device, &gpu.queue, &mut encoder);
    gpu.queue.submit([encoder.finish()]);
    let points: Vec<_> = (0..321)
        .map(|i| [20.0, 0.0, stem + 220.0 + i as f32 * 0.25, 0.0])
        .collect();
    let values = sample(&gpu, &wake, &camera, &points);
    let height = values.iter().map(|v| v[0].abs()).fold(0.0_f32, f32::max);
    assert!(
        height < 0.04,
        "the quiet interior still displaces the full Kelvin wave: {height} m"
    );
    assert!(
        height > 0.005,
        "the low interior waves should remain: {height} m"
    );
}

#[test]
fn a_fast_stern_raises_a_hollow_and_a_white_hump_behind_it() {
    let gpu = Gpu::headless().expect("a GPU");
    let camera = Camera::look_at(Vec3::new(60.0, 30.0, 120.0), Vec3::new(0.0, 0.0, 60.0));
    // A destroyer-sized hull with no slice of her own: her stern 0.98 of her length aft of her stem, which stands 0.49 of
    // it ahead of her middle (heading 0: forward is −Z).
    let (length, beam) = (114.7_f32, 12.0_f32);
    let stern = 0.49 * length;
    let mut runs = Vec::new();
    for speed in [16.0, 5.0] {
        let mut wake = ShipWake::new(&gpu.device, 0, 0.1);
        let ship = WakeShip {
            id: "destroyer",
            position: [0.0; 3],
            heading: 0.0,
            pitch: 0.0,
            roll: 0.0,
            speed,
            length: f64::from(length),
            beam: f64::from(beam),
            draft: 3.8,
            center_x: 0.0,
            center_z: 0.0,
            forward_speed: 18.0,
            reverse_speed: 10.0,
            profile: None,
            oil_leak_m3s: 0.0,
        };
        wake.update(&WakeInputs {
            ships: &[ship],
            dt: 1.0 / 60.0,
            splashes: &[],
            torpedoes: &[],
            sea_height: 0.0,
            camera: WakeCamera {
                eye: [60.0, 30.0, 120.0],
                forward: [-0.8, -0.2, -0.6],
                fov_y: 52_f64.to_radians(),
                zoom: 1.0,
                drawing_height: 800.0,
            },
        });
        // Only the stern's waves: no Kelvin V, no bow crest, a water mesh fine enough to hold the hump.
        wake.bow_waves.slots[0][1][1] = 0.0;
        wake.bow_waves.slots[0][1][2] = 0.0;
        wake.bow_waves.slots[0][3][1] = 0.0;
        wake.bow_waves.vertex_spacing = 0.25;
        wake.bow_waves.mesh_growth = 0.0;
        let row = wake.bow_waves.slots[0][6];
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        wake.encode(&gpu.device, &gpu.queue, &mut encoder);
        gpu.queue.submit([encoder.finish()]);
        // Her centreline from a fifth of her length forward of her stern to 60 m astern of it, every 0.25 m.
        let points: Vec<[f32; 4]> = (0..=(0.2 * length + 60.0) as usize * 4)
            .map(|i| [0.0, 0.0, stern - 0.2 * length + i as f32 * 0.25, 0.0])
            .collect();
        runs.push((row, points.clone(), sample(&gpu, &wake, &camera, &points)));
    }
    let (row, points, fast) = &runs[0];
    let [_, crest, peak, _] = *row;
    assert!(
        (1.6..1.8).contains(&crest),
        "a Fletcher at 31 kn: {crest} m"
    );
    let astern = |p: &[f32; 4]| p[2] - stern;
    let (top, highest) = points
        .iter()
        .zip(fast)
        .filter(|(p, _)| astern(p) > 0.0)
        .max_by(|a, b| a.1[0].total_cmp(&b.1[0]))
        .unwrap();
    assert!(
        (astern(top) - peak).abs() < 1.5 && highest[0] > 0.85 * crest && highest[0] < 1.05 * crest,
        "the hump stands {} m at {} m astern, its crest {crest} m at {peak} m",
        highest[0],
        astern(top)
    );
    let hollow = points
        .iter()
        .zip(fast)
        .filter(|(p, _)| (0.0..peak).contains(&astern(p)))
        .map(|(_, v)| v[0])
        .fold(f32::INFINITY, f32::min);
    assert!(
        hollow < -0.5 * crest,
        "the hollow behind her stern: {hollow} m"
    );
    let white = points
        .iter()
        .zip(fast)
        .filter(|(p, _)| (astern(p) - peak).abs() < 2.0)
        .map(|(_, v)| v[1])
        .fold(0.0_f32, f32::max);
    assert!(white > 0.5, "the hump breaks white at 31 kn: {white}");
    let (_, _, slow) = &runs[1];
    let calm = slow
        .iter()
        .map(|v| v[0].abs().max(v[1]))
        .fold(0.0_f32, f32::max);
    assert!(calm < 0.02, "at 10 kn her stern barely stirs: {calm}");
}
