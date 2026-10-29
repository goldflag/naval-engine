//! The pipeline registry's build threads: builds finish off the calling thread and lookups wait for
//! them, a first build wgpu rejects is reported by `finish`, and a parallel rebuild keeps the last good
//! pipeline where it fails, as the serial path does.

use std::collections::BTreeSet;

use native_offscreen::Gpu;
use naval_render::{Pipeline, PipelineRecipe, PipelineRegistry, ShaderLibrary, compute_recipe};

const SHADER: &str = "
@group(0) @binding(0) var<storage, read_write> values: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    values[id.x] = id.x * FACTOR;
}
";

fn library(factor: u32) -> ShaderLibrary {
    let mut library = ShaderLibrary::new();
    library
        .add_source(
            "tests/pipelines.wgsl",
            SHADER.replace("FACTOR", &format!("{factor}u")),
        )
        .unwrap();
    library
}

fn storage_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("pipelines test values"),
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
    })
}

/// Runs `pipeline` over 64 values and reads them back.
fn run(gpu: &Gpu, layout: &wgpu::BindGroupLayout, pipeline: &wgpu::ComputePipeline) -> Vec<u32> {
    let size = 64 * 4;
    let values = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pipelines test values"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let read = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("pipelines test read-back"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("pipelines test values"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: values.as_entire_binding(),
        }],
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor::default());
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&values, 0, &read, 0, size);
    gpu.queue.submit([encoder.finish()]);
    read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    let bytes = read.slice(..).get_mapped_range().unwrap().to_vec();
    bytemuck::cast_slice(&bytes).to_vec()
}

#[test]
fn build_threads_compile_off_the_calling_thread_and_lookups_wait() {
    let gpu = Gpu::headless().unwrap();
    let layout = storage_layout(&gpu.device);
    let mut library = library(3);
    let mut registry = PipelineRegistry::default();
    registry.set_build_threads(4);
    assert_eq!(registry.build_threads(), 4);
    let ids: Vec<_> = (0..12)
        .map(|index| {
            registry
                .add(
                    &gpu.device,
                    &mut library,
                    format!("pipelines test {index}"),
                    "tests/pipelines.wgsl",
                    Vec::new(),
                    compute_recipe(vec![layout.clone()], "main"),
                )
                .unwrap()
        })
        .collect();
    // A lookup waits for its own build; `finish` takes in the rest.
    let first = run(&gpu, &layout, registry.compute(ids[0]));
    assert_eq!(first[5], 15);
    registry.finish().unwrap();
    assert_eq!(registry.pending(), 0);
    for id in &ids {
        assert_eq!(run(&gpu, &layout, registry.compute(*id))[7], 21);
    }
    let status = registry.status();
    assert!(status.iter().all(|s| s.builds == 1 && s.error.is_none()));
    assert!(status.iter().all(|s| s.build_ms >= 0.0));
}

/// A recipe whose layout does not match the shader (no group 0): wgpu rejects the pipeline.
fn mismatched_recipe() -> PipelineRecipe {
    Box::new(|build| {
        let layout = build
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(build.label),
                bind_group_layouts: &[],
                immediate_size: 0,
            });
        Pipeline::Compute(
            build
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&layout),
                    module: build.module,
                    entry_point: Some("main"),
                    compilation_options: Default::default(),
                    cache: build.cache,
                }),
        )
    })
}

#[test]
fn a_rejected_first_build_is_an_error_on_either_path() {
    let gpu = Gpu::headless().unwrap();
    let mut library = library(1);
    // On the calling thread: `add` returns it.
    let mut serial = PipelineRegistry::default();
    let error = serial
        .add(
            &gpu.device,
            &mut library,
            "pipelines test mismatched",
            "tests/pipelines.wgsl",
            Vec::new(),
            mismatched_recipe(),
        )
        .expect_err("wgpu rejects the layout");
    assert_eq!(error.path, "tests/pipelines.wgsl");
    // On build threads: `add` returns the id and `finish` the error.
    let mut parallel = PipelineRegistry::default();
    parallel.set_build_threads(2);
    parallel
        .add(
            &gpu.device,
            &mut library,
            "pipelines test mismatched",
            "tests/pipelines.wgsl",
            Vec::new(),
            mismatched_recipe(),
        )
        .expect("composition succeeds; the build is on a thread");
    let error = parallel.finish().expect_err("the build failed");
    assert_eq!(error.path, "tests/pipelines.wgsl");
    assert!(parallel.status()[0].error.is_some());
}

#[test]
fn a_parallel_rebuild_takes_good_edits_and_keeps_the_last_good_pipeline() {
    let gpu = Gpu::headless().unwrap();
    let layout = storage_layout(&gpu.device);
    let mut library = library(2);
    let mut registry = PipelineRegistry::default();
    registry.set_build_threads(3);
    let id = registry
        .add(
            &gpu.device,
            &mut library,
            "pipelines test rebuild",
            "tests/pipelines.wgsl",
            Vec::new(),
            compute_recipe(vec![layout.clone()], "main"),
        )
        .unwrap();
    let changed = BTreeSet::from(["tests/pipelines.wgsl".to_owned()]);
    // A good edit.
    library
        .update("tests/pipelines.wgsl", SHADER.replace("FACTOR", "5u"))
        .unwrap();
    let report = registry.rebuild_affected(&gpu.device, &mut library, &changed);
    assert_eq!(report.rebuilt, vec!["pipelines test rebuild".to_owned()]);
    assert_eq!(run(&gpu, &layout, registry.compute(id))[3], 15);
    // An edit wgpu rejects (a binding the layout lacks) keeps the last good pipeline.
    let bad = SHADER.replace("FACTOR", "7u")
        + "@group(0) @binding(1) var<storage, read_write> more: array<u32>;\n\
           @compute @workgroup_size(1) fn other() { more[0] = 1u; }\n";
    let bad = bad.replace(
        "values[id.x] = id.x * 7u;",
        "values[id.x] = id.x * 7u; more[0] = 2u;",
    );
    library.update("tests/pipelines.wgsl", bad).unwrap();
    let report = registry.rebuild_affected(&gpu.device, &mut library, &changed);
    assert!(report.rebuilt.is_empty(), "{report:?}");
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert_eq!(run(&gpu, &layout, registry.compute(id))[3], 15);
    assert_eq!(registry.status()[0].builds, 2);
}
