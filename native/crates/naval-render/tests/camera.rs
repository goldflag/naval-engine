//! The camera module's view ray on a real GPU: a camera kilometres from the origin, as a theatre's is between
//! re-centres of its render origin, still turns each pixel into its ray to a small fraction of a pixel. Through the
//! inverse view-projection the ray kept the f32 rounding of the eye's position, a few pixels there, changing as the
//! camera moved: the splash columns marched along it shook.

use glam::{DVec3, Vec3};
use native_offscreen::Gpu;
use naval_render::shaders::ShaderLibrary;
use naval_render::{Camera, FrameTime};
use wgpu::util::DeviceExt;

const SHADER: &str = r#"
#import naval::camera::{view_ray, ndc_from_pixel}

@group(1) @binding(0) var<storage, read> pixels: array<vec4<f32>>;
@group(1) @binding(1) var<storage, read_write> rays: array<vec4<f32>>;

@compute @workgroup_size(1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    rays[id.x] = vec4<f32>(view_ray(ndc_from_pixel(pixels[id.x].xy)), 0.0);
}
"#;

const WIDTH: u32 = 1920;
const HEIGHT: u32 = 1080;

/// `view_ray` through each pixel, as the GPU works it out for `camera`.
fn gpu_rays(gpu: &Gpu, camera: &Camera, pixels: &[[f32; 2]]) -> Vec<DVec3> {
    let device = &gpu.device;
    let mut library = ShaderLibrary::new();
    library.add_source("test/view_ray.wgsl", SHADER).unwrap();
    let module = library
        .create_module(device, "view ray", "test/view_ray.wgsl", &Vec::new())
        .unwrap();
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("view ray"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let uniforms = camera.uniforms(WIDTH, HEIGHT, FrameTime::fixed(0, 1.0 / 60.0));
    let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("camera"),
        contents: bytemuck::bytes_of(&uniforms),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let padded: Vec<[f32; 4]> = pixels.iter().map(|p| [p[0], p[1], 0.0, 0.0]).collect();
    let pixel_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("pixels"),
        contents: bytemuck::cast_slice(&padded),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let size = (pixels.len() * 16) as u64;
    let ray_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rays"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rays read-back"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let camera_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("camera"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: camera_buffer.as_entire_binding(),
        }],
    });
    let ray_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rays"),
        layout: &pipeline.get_bind_group_layout(1),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: pixel_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: ray_buffer.as_entire_binding(),
            },
        ],
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &camera_group, &[]);
        pass.set_bind_group(1, &ray_group, &[]);
        pass.dispatch_workgroups(pixels.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&ray_buffer, 0, &read, 0, size);
    gpu.queue.submit([encoder.finish()]);
    let slice = read.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("mapping"));
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting for the read-back");
    let mapped = slice.get_mapped_range().expect("mapped");
    bytemuck::cast_slice::<u8, [f32; 4]>(&mapped)
        .iter()
        .map(|r| DVec3::new(f64::from(r[0]), f64::from(r[1]), f64::from(r[2])))
        .collect()
}

/// The ray through a pixel, in f64 from the camera's own eye and target.
fn true_ray(camera: &Camera, pixel: [f32; 2]) -> DVec3 {
    let forward = (camera.target.as_dvec3() - camera.eye.as_dvec3()).normalize();
    let right = forward.cross(DVec3::Y).normalize();
    let up = right.cross(forward);
    let tan = (f64::from(camera.fov_y_degrees).to_radians() / 2.0).tan();
    let aspect = f64::from(WIDTH) / f64::from(HEIGHT);
    let x = f64::from(pixel[0]) / f64::from(WIDTH) * 2.0 - 1.0;
    let y = 1.0 - f64::from(pixel[1]) / f64::from(HEIGHT) * 2.0;
    (forward + right * (x * aspect * tan) + up * (y * tan)).normalize()
}

#[test]
fn a_camera_far_from_the_origin_keeps_its_rays_within_a_pixel() {
    let gpu = Gpu::headless().expect("a GPU");
    let pixels = [
        [960.5, 540.5],
        [100.5, 80.5],
        [1800.5, 1000.5],
        [1500.5, 300.5],
        [400.5, 900.5],
    ];
    // Out to the render origin's re-centring distance (16 km), stepped as a moving camera steps from frame to frame.
    for k in 0..8 {
        let eye = Vec3::new(
            14_000.0 + 0.37 * k as f32,
            35.0 + 0.011 * k as f32,
            -8_000.0,
        );
        let camera = Camera::look_at(eye, eye + Vec3::new(-400.0, -15.0, 300.0));
        let pixels_per_radian =
            f64::from(HEIGHT) / 2.0 / (f64::from(camera.fov_y_degrees).to_radians() / 2.0).tan();
        for (ray, pixel) in gpu_rays(&gpu, &camera, &pixels).iter().zip(pixels) {
            let truth = true_ray(&camera, pixel);
            let error = ray.cross(truth).length().atan2(ray.dot(truth)) * pixels_per_radian;
            assert!(
                error < 0.05,
                "eye {eye}, pixel {pixel:?}: the ray is {error:.3} px off ({ray} against {truth})"
            );
        }
    }
}
