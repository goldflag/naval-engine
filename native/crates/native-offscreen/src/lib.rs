//! Headless wgpu for tests and offscreen captures: a device with no window, a
//! color target, and a read-back to PNG. Lanes use it until the renderer core
//! has its own capture path. [`fixture`] holds the recorded expectations tests check against.

pub mod fixture;

use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};

use anyhow::{Context, Result, bail};

/// The format of every capture: 8-bit sRGB, which is what a PNG stores.
pub const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Every wgpu instance's descriptor here, a window's with its display handle added. DX12 on Windows: wgpu 30's Vulkan
/// presentation fence can return `TIMEOUT` as an unexpected device error instead of a surface timeout. `WGPU_BACKEND`
/// still overrides the default for driver diagnosis and comparisons.
pub fn instance_descriptor() -> wgpu::InstanceDescriptor {
    let descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    #[cfg(windows)]
    let descriptor = wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        ..descriptor
    };
    descriptor.with_env()
}

/// The process's one headless instance, which every headless device comes from ([`Gpu::headless`], the client's shots
/// and RPC, the harness). wgpu tells devices apart only by their id within their instance (`Device`'s `==`), and the
/// per-device caches rely on that (`naval_render::DeviceCache`, the fleet's shared geometry): the first devices of two
/// instances compare equal, so a test program running many GPU tests at once would hand one device another's textures.
pub fn instance() -> wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| wgpu::Instance::new(instance_descriptor()))
        .clone()
}

pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// A high-performance adapter with default limits and no surface.
    pub fn headless() -> Result<Self> {
        pollster::block_on(Self::headless_async())
    }

    pub async fn headless_async() -> Result<Self> {
        Self::headless_with_async(wgpu::Features::empty()).await
    }

    /// As [`Gpu::headless`], with whichever of `features` the adapter has (`device.features()` says which).
    pub fn headless_with(features: wgpu::Features) -> Result<Self> {
        pollster::block_on(Self::headless_with_async(features))
    }

    async fn headless_with_async(features: wgpu::Features) -> Result<Self> {
        let instance = instance();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                ..Default::default()
            })
            .await
            .context("no GPU adapter")?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("native-offscreen"),
                required_features: adapter.features() & features,
                // The effects trials together bind more textures than the default allows, and the fleet binds every
                // preset's gun covers' stored shapes as one storage buffer, over the default 128 MiB with a mixed fleet
                // of battleships (naval-effects' trial.rs).
                required_limits: wgpu::Limits {
                    max_sampled_textures_per_shader_stage: adapter
                        .limits()
                        .max_sampled_textures_per_shader_stage
                        .min(24),
                    max_storage_buffer_binding_size: adapter
                        .limits()
                        .max_storage_buffer_binding_size,
                    max_buffer_size: adapter.limits().max_buffer_size,
                    ..Default::default()
                },
                ..Default::default()
            })
            .await
            .context("requesting the device")?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

/// A color target in [`CAPTURE_FORMAT`] that can be rendered into, sampled and
/// copied back to the CPU.
pub struct Capture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub width: u32,
    pub height: u32,
}

impl Capture {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("capture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    /// Tightly packed RGBA8 rows, top row first, once all submitted work is done.
    pub fn read_rgba(&self, gpu: &Gpu) -> Result<Vec<u8>> {
        let row = self.width * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture read-back"),
            size: u64::from(padded) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("capture copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([encoder.finish()]);

        let slice = buffer.slice(..);
        let (sender, receiver) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        gpu.device
            .poll(wgpu::PollType::wait_indefinitely())
            .context("waiting for the read-back")?;
        match receiver.recv() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => bail!("mapping the read-back failed: {error}"),
            Err(_) => bail!("the read-back never finished"),
        }

        let mapped = slice
            .get_mapped_range()
            .context("reading the mapped range")?;
        let mut pixels = Vec::with_capacity((row * self.height) as usize);
        for chunk in mapped.chunks(padded as usize) {
            pixels.extend_from_slice(&chunk[..row as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }

    /// Writes the target as a PNG, creating parent directories.
    pub fn save_png(&self, gpu: &Gpu, path: &Path) -> Result<()> {
        let pixels = self.read_rgba(gpu)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        image::save_buffer(
            path,
            &pixels,
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )
        .with_context(|| format!("writing {}", path.display()))
    }
}

/// The repository root: the ignored `.build/` output tree lives there.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("native/crates/native-offscreen sits three levels below the repo root")
}

/// `.build/native/<lane>/` at the repository root, for images, captures and logs.
pub fn build_dir(lane: &str) -> PathBuf {
    repo_root().join(".build/native").join(lane)
}
