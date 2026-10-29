//! Helpers shared by the GPU tests.
#![allow(dead_code)]

use half::f16;
use native_offscreen::Gpu;

/// Reads a whole single-mip texture of 4-byte or 8-byte texels (`Rgba16Float`,
/// `R32Float`, `Depth32Float`) back as rows of f32 channels, once all submitted
/// work is done.
pub fn read_texture(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<f32> {
    let size = texture.size();
    let format = texture.format();
    let aspect = if format.is_depth_stencil_format() {
        wgpu::TextureAspect::DepthOnly
    } else {
        wgpu::TextureAspect::All
    };
    let texel = format
        .block_copy_size(Some(aspect))
        .expect("a copyable format");
    let row = size.width * texel;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test read-back"),
        size: u64::from(padded) * u64::from(size.height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(size.height),
            },
        },
        wgpu::Extent3d {
            depth_or_array_layers: 1,
            ..size
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.expect("mapping"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("waiting for the read-back");
    let mapped = slice.get_mapped_range().expect("mapped");
    let mut values = Vec::new();
    for chunk in mapped.chunks(padded as usize) {
        let row = &chunk[..row as usize];
        match format {
            wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::R16Float => values.extend(
                row.chunks(2)
                    .map(|b| f16::from_le_bytes([b[0], b[1]]).to_f32()),
            ),
            _ => values.extend(
                row.chunks(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            ),
        }
    }
    values
}

/// The channels of texel (x, y) in a read-back of `channels`-channel texels `width` wide.
pub fn texel(values: &[f32], width: u32, channels: usize, x: u32, y: u32) -> Vec<f32> {
    let start = (y as usize * width as usize + x as usize) * channels;
    values[start..start + channels].to_vec()
}

pub fn assert_near(actual: &[f32], expected: &[f32], tolerance: f32, what: &str) {
    assert_eq!(actual.len(), expected.len(), "{what}");
    for (a, e) in actual.iter().zip(expected) {
        assert!(
            (a - e).abs() <= tolerance,
            "{what}: got {actual:?}, expected {expected:?}"
        );
    }
}
