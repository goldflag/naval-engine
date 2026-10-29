//! Small wgpu helpers the effect layer shares: textures uploaded with their mip chains, storage buffers that
//! grow with their contents, and bind group layout entries.

use crate::textures::{Image2d, Volume};

/// An RGBA8 texture with its CPU-made mip chain.
pub fn upload_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    image: &Image2d,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: image.size,
            height: image.size,
            depth_or_array_layers: 1,
        },
        mip_level_count: image.levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (level, pixels) in image.levels.iter().enumerate() {
        let edge = (image.size >> level).max(1);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(edge * 4),
                rows_per_image: Some(edge),
            },
            wgpu::Extent3d {
                width: edge,
                height: edge,
                depth_or_array_layers: 1,
            },
        );
    }
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// An RGBA8 volume without mips.
pub fn upload_volume(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    volume: &Volume,
) -> wgpu::TextureView {
    let size = wgpu::Extent3d {
        width: volume.size,
        height: volume.size,
        depth_or_array_layers: volume.size,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &volume.texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(volume.size * 4),
            rows_per_image: Some(volume.size),
        },
        size,
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// A storage (or vertex/index) buffer that is remade larger when its contents outgrow it. `version` changes on
/// every remake, so bind groups on it know to rebuild.
pub struct GrowBuffer {
    pub buffer: wgpu::Buffer,
    pub version: u64,
    label: &'static str,
    usage: wgpu::BufferUsages,
}

impl GrowBuffer {
    pub fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
        bytes: u64,
    ) -> Self {
        Self {
            buffer: Self::make(device, label, usage, bytes),
            version: 0,
            label,
            usage,
        }
    }

    fn make(
        device: &wgpu::Device,
        label: &str,
        usage: wgpu::BufferUsages,
        bytes: u64,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bytes.max(256).next_power_of_two(),
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Writes `data`, growing first when it does not fit.
    pub fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        self.write_to(device, &mut naval_render::UploadSink::Queue(queue), data);
    }

    /// [`GrowBuffer::write`] through `sink` (the renderer core's staging belt, in a layer's `prepare`).
    pub fn write_to(
        &mut self,
        device: &wgpu::Device,
        sink: &mut naval_render::UploadSink<'_>,
        data: &[u8],
    ) {
        if data.len() as u64 > self.buffer.size() {
            self.buffer = Self::make(device, self.label, self.usage, data.len() as u64);
            self.version += 1;
        }
        if !data.is_empty() {
            sink.write_buffer(&self.buffer, 0, data);
        }
    }
}

pub fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub fn storage_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub fn texture_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    dimension: wgpu::TextureViewDimension,
    filterable: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub fn depth_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Depth,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub fn storage_texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::Rgba8Unorm,
            view_dimension: dimension,
        },
        count: None,
    }
}

pub fn sampler_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// A linear sampler with the given wrap and mip filtering (`anisotropy` 1 for none).
pub fn sampler(
    device: &wgpu::Device,
    label: &str,
    wrap: wgpu::AddressMode,
    mipmaps: bool,
    anisotropy: u16,
) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some(label),
        address_mode_u: wrap,
        address_mode_v: wrap,
        address_mode_w: wrap,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: if mipmaps {
            wgpu::MipmapFilterMode::Linear
        } else {
            wgpu::MipmapFilterMode::Nearest
        },
        anisotropy_clamp: anisotropy,
        ..Default::default()
    })
}
