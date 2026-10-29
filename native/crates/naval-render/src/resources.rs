//! Labelled GPU textures and buffers with explicit lifetimes.
//!
//! The registry is how passes and external systems find shared resources by
//! name (the scene targets are `scene.hdr` and `scene.depth`). Every entry has
//! a [`Lifetime`]: it lives until removed, is remade at the viewport's size on
//! resize, or is dropped when the frame ends. An entry's `version` changes each
//! time it is remade, so a bind group built on it knows to rebuild.

use std::collections::BTreeMap;

/// How long a registered resource lives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lifetime {
    /// Until removed or replaced.
    Persistent,
    /// Remade whenever the viewport changes size, at `scale` × the viewport
    /// (textures only; a buffer with this lifetime lives as `Persistent`).
    Viewport { scale: f32 },
    /// Dropped when the frame that made it ends.
    Frame,
}

/// A texture to register. Its size comes from `size`, or from the viewport for
/// [`Lifetime::Viewport`].
#[derive(Clone, Debug)]
pub struct TextureDesc {
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
    pub size: (u32, u32),
    pub mip_level_count: u32,
    pub lifetime: Lifetime,
}

impl TextureDesc {
    /// A single-mip texture at `scale` × the viewport, remade on resize.
    pub fn viewport(format: wgpu::TextureFormat, usage: wgpu::TextureUsages, scale: f32) -> Self {
        Self {
            format,
            usage,
            size: (1, 1),
            mip_level_count: 1,
            lifetime: Lifetime::Viewport { scale },
        }
    }

    /// A single-mip texture of a fixed size.
    pub fn fixed(
        format: wgpu::TextureFormat,
        usage: wgpu::TextureUsages,
        width: u32,
        height: u32,
        lifetime: Lifetime,
    ) -> Self {
        Self {
            format,
            usage,
            size: (width, height),
            mip_level_count: 1,
            lifetime,
        }
    }
}

/// A registered texture and its default view.
pub struct TextureEntry {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub desc: TextureDesc,
    /// Changes every time the texture is remade.
    pub version: u64,
}

impl TextureEntry {
    /// Approximate bytes held: mip chains as 4/3 of the base, an unknown texel as 4 bytes.
    pub fn bytes(&self) -> u64 {
        let (width, height) = self.desc.size;
        let format = self.desc.format;
        let texel = format
            .block_copy_size(Some(wgpu::TextureAspect::All))
            .or_else(|| format.block_copy_size(Some(wgpu::TextureAspect::DepthOnly)))
            .unwrap_or(4) as u64;
        let base = u64::from(width) * u64::from(height) * texel;
        if self.desc.mip_level_count > 1 {
            base * 4 / 3
        } else {
            base
        }
    }
}

/// A registered buffer.
pub struct BufferEntry {
    pub buffer: wgpu::Buffer,
    pub lifetime: Lifetime,
    pub version: u64,
}

/// Counts for telemetry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceStats {
    pub textures: usize,
    pub buffers: usize,
    /// Approximate bytes held, mip chains and block formats counted roughly.
    pub bytes: u64,
}

/// The viewport textures of a second view of the same scene (`Renderer::render_view`), at that view's size: held here
/// while the main view draws and swapped into the registry while the second does ([`Resources::swap_view`]).
#[derive(Default)]
pub struct ViewTextures {
    textures: BTreeMap<String, TextureEntry>,
    viewport: (u32, u32),
}

impl ViewTextures {
    /// The size a view holding no textures (its first draw, or after [`ViewTextures::release`]) makes them at, from the
    /// next swap. A view that holds some keeps their size: the registry's `resize` after the swap remakes them at a new
    /// one, as it would the main view's.
    pub fn set_viewport(&mut self, width: u32, height: u32) {
        if self.textures.is_empty() {
            self.viewport = (width.max(1), height.max(1));
        }
    }

    /// Lets the textures go (the view is no longer drawn); they are made again at the next swap.
    pub fn release(&mut self) {
        self.textures.clear();
    }

    /// Approximate bytes held.
    pub fn bytes(&self) -> u64 {
        self.textures.values().map(TextureEntry::bytes).sum()
    }
}

pub struct Resources {
    textures: BTreeMap<String, TextureEntry>,
    buffers: BTreeMap<String, BufferEntry>,
    viewport: (u32, u32),
    next_version: u64,
}

impl Resources {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            textures: BTreeMap::new(),
            buffers: BTreeMap::new(),
            viewport: (width.max(1), height.max(1)),
            next_version: 1,
        }
    }

    /// Creates (or replaces) the texture `name`.
    pub fn create_texture(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        desc: TextureDesc,
    ) -> &TextureEntry {
        let entry = self.make_texture(device, name, desc);
        self.textures.insert(name.to_owned(), entry);
        &self.textures[name]
    }

    fn make_texture(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        desc: TextureDesc,
    ) -> TextureEntry {
        let (width, height) = match desc.lifetime {
            Lifetime::Viewport { scale } => (
                ((self.viewport.0 as f32 * scale).round() as u32).max(1),
                ((self.viewport.1 as f32 * scale).round() as u32).max(1),
            ),
            _ => desc.size,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(name),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: desc.mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: desc.format,
            usage: desc.usage,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(name),
            ..Default::default()
        });
        let version = self.next_version;
        self.next_version += 1;
        TextureEntry {
            texture,
            view,
            desc: TextureDesc {
                size: (width, height),
                ..desc
            },
            version,
        }
    }

    /// Creates (or replaces) the buffer `name`.
    pub fn create_buffer(
        &mut self,
        device: &wgpu::Device,
        name: &str,
        size: u64,
        usage: wgpu::BufferUsages,
        lifetime: Lifetime,
    ) -> &BufferEntry {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(name),
            size,
            usage,
            mapped_at_creation: false,
        });
        let version = self.next_version;
        self.next_version += 1;
        self.buffers.insert(
            name.to_owned(),
            BufferEntry {
                buffer,
                lifetime,
                version,
            },
        );
        &self.buffers[name]
    }

    pub fn texture(&self, name: &str) -> Option<&TextureEntry> {
        self.textures.get(name)
    }

    pub fn buffer(&self, name: &str) -> Option<&BufferEntry> {
        self.buffers.get(name)
    }

    /// Removes a texture or buffer called `name`; the GPU frees it once no
    /// submitted work uses it.
    pub fn remove(&mut self, name: &str) {
        self.textures.remove(name);
        self.buffers.remove(name);
    }

    /// Each registered texture's name and approximate bytes ([`TextureEntry::bytes`]), largest first.
    pub fn texture_bytes(&self) -> Vec<(&str, u64)> {
        let mut sizes: Vec<_> = self
            .textures
            .iter()
            .map(|(name, entry)| (name.as_str(), entry.bytes()))
            .collect();
        sizes.sort_by_key(|e| std::cmp::Reverse(e.1));
        sizes
    }

    /// The viewport size viewport textures follow.
    pub fn viewport(&self) -> (u32, u32) {
        self.viewport
    }

    /// Remakes every viewport texture at the new size. Returns whether the size changed.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) -> bool {
        let size = (width.max(1), height.max(1));
        if size == self.viewport {
            return false;
        }
        self.viewport = size;
        let names: Vec<String> = self
            .textures
            .iter()
            .filter(|(_, entry)| matches!(entry.desc.lifetime, Lifetime::Viewport { .. }))
            .map(|(name, _)| name.clone())
            .collect();
        for name in names {
            let desc = self.textures[&name].desc.clone();
            let entry = self.make_texture(device, &name, desc);
            self.textures.insert(name, entry);
        }
        true
    }

    /// Exchanges the registry's viewport textures and viewport size with `view`'s: the second view's in for its draw,
    /// the main view's back after it. Whatever viewport texture the set coming in lacks (the view's first swap, or one
    /// registered since) is made at its size, and one the set going out no longer holds is let go, so each view finds
    /// every viewport texture the other registered. Other lifetimes stay where they are.
    pub fn swap_view(&mut self, device: &wgpu::Device, view: &mut ViewTextures) {
        let names: Vec<String> = self
            .textures
            .iter()
            .filter(|(_, entry)| matches!(entry.desc.lifetime, Lifetime::Viewport { .. }))
            .map(|(name, _)| name.clone())
            .collect();
        let outgoing: BTreeMap<String, TextureEntry> = names
            .into_iter()
            .filter_map(|name| self.textures.remove(&name).map(|entry| (name, entry)))
            .collect();
        let mut incoming = std::mem::take(&mut view.textures);
        let size = if view.viewport == (0, 0) {
            self.viewport
        } else {
            view.viewport
        };
        view.viewport = std::mem::replace(&mut self.viewport, size);
        for (name, entry) in &outgoing {
            let entry = match incoming.remove(name) {
                Some(held) => held,
                None => self.make_texture(device, name, entry.desc.clone()),
            };
            self.textures.insert(name.clone(), entry);
        }
        view.textures = outgoing;
    }

    /// Drops every [`Lifetime::Frame`] resource.
    pub fn end_frame(&mut self) {
        self.textures
            .retain(|_, entry| entry.desc.lifetime != Lifetime::Frame);
        self.buffers
            .retain(|_, entry| entry.lifetime != Lifetime::Frame);
    }

    pub fn stats(&self) -> ResourceStats {
        let texture_bytes: u64 = self.textures.values().map(TextureEntry::bytes).sum();
        let buffer_bytes: u64 = self.buffers.values().map(|entry| entry.buffer.size()).sum();
        ResourceStats {
            textures: self.textures.len(),
            buffers: self.buffers.len(),
            bytes: texture_bytes + buffer_bytes,
        }
    }

    /// Registered texture names, for telemetry.
    pub fn texture_names(&self) -> impl Iterator<Item = &str> {
        self.textures.keys().map(String::as_str)
    }

    /// Registered buffer names, for telemetry.
    pub fn buffer_names(&self) -> impl Iterator<Item = &str> {
        self.buffers.keys().map(String::as_str)
    }
}
