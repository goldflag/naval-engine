//! The trees' octahedral impostor atlas (native/docs/terrain/trees.md): every species' grown tree drawn on the GPU
//! as the near trees draw it at full detail ([`crate::tree_meshes`]) from `IMPOSTOR_FRAMES`² directions over the upper
//! hemisphere (`shaders/tree_bake.wgsl`, `shaders/impostor.wgsl`) onto a scratch twice the atlas's side, brought down
//! into a 2D array texture (`shaders/tree_resolve.wgsl`), then mip-mapped (`shaders/tree_mips.wgsl`). Layer `k` holds
//! species `k`'s colour (sRGB-encoded, premultiplied) and coverage, layer `models + k` its normal, depth and ambient
//! occlusion, and the last layers the leaf sprites the near trees' meshes are drawn with
//! (`tree_meshes::leaf_sprites`). It never changes and it is about 200 MB, so a host that builds land
//! after land (a theatre's scenes, one every 16 km of passage) makes one [`TreeAtlas`] for its device and hands it to
//! each (`TerrainLayer::for_scene_sharing`): the first land to draw bakes it in its first `prepare`, before its trees are
//! first placed, and a land made after that registers no bake at all. A land given none makes and bakes its own.
//!
//! On a device that samples BC7 the baked atlas is then read back a few layers at a time and encoded on the CPU by a
//! thread of its own (`compress`), and every land draws the BC7 copy from the frame after it is ready (a quarter of the
//! bytes: about 50 MB in place of 198 MB); the RGBA8 bake is let go once the last land drawing it has moved over.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use naval_render::{PipelineId, PrepareContext, RenderContext, ShaderError, compute_recipe};

use crate::species::models;
use crate::tree_meshes::{MESH_VERTEX_BYTES, leaf_sprites, tree_meshes};

/// Frames a side of each species' square (`impostor.wgsl`'s `IMPOSTOR_FRAMES`).
pub const IMPOSTOR_FRAMES: u32 = 8;
/// A frame's texels a side (`FRAME_TEXELS`): 96 since 2026-10-08 (64 before; 128 would take 4 times the memory).
pub const FRAME_TEXELS: u32 = 96;
/// The atlas's side.
pub const ATLAS_TEXELS: u32 = IMPOSTOR_FRAMES * FRAME_TEXELS;
/// Mip levels: a frame from 96 texels down to 3 (a level further would split a texel between two frames).
pub const ATLAS_LEVELS: u32 = 6;
/// Layers after the species' colour and normal layers: the leaf sprites (`impostor.wgsl`'s `LEAF_LAYERS`).
pub const LEAF_LAYERS: u32 = crate::tree_meshes::SPRITE_LAYERS;

/// The atlas's layers: each species' colour, then each one's normal, then the leaf sprites.
pub fn atlas_layers() -> u32 {
    models().len() as u32 * 2 + LEAF_LAYERS
}

/// The atlas's bytes on the GPU, every level.
pub fn atlas_bytes() -> u64 {
    (0..ATLAS_LEVELS)
        .map(|level| u64::from((ATLAS_TEXELS >> level).pow(2)) * 4)
        .sum::<u64>()
        * u64::from(atlas_layers())
}
/// The atlas's format: sRGB colour is encoded by the bake and decoded by the draw, since the normals share the
/// texture.
pub const ATLAS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The atlas's format once compressed ([`crate::bc7`]): the same unorm values, so the draw reads it unchanged.
pub const COMPRESSED_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bc7RgbaUnorm;

/// The trees' impostor atlas on a device, for the lands that share it, and whether one of them has baked it. Cloning it
/// shares it.
#[derive(Clone, Debug)]
pub struct TreeAtlas {
    shared: Arc<Shared>,
}

#[derive(Debug)]
struct Shared {
    /// Whether a land has baked it (in a frame already submitted, or the one being prepared).
    baked: AtomicBool,
    /// The texture the lands draw: the bake's RGBA8 until its BC7 copy is ready.
    current: Mutex<wgpu::Texture>,
    /// Bumped each time `current` changes, so each land knows to bind it again.
    generation: AtomicU32,
    /// Whether the BC7 copy is being made, and a signal when it is done.
    compression: Mutex<Compression>,
    done: std::sync::Condvar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compression {
    NotStarted,
    Running,
    Done,
}

impl TreeAtlas {
    /// An atlas on `device`, unbaked (the first land to draw with it bakes it), copied to BC7 once baked where the device
    /// samples it.
    pub fn new(device: &wgpu::Device) -> Self {
        Self::make(device, Compression::NotStarted)
    }

    /// An atlas kept as the RGBA8 bake, for tests and probes that read it back.
    pub fn uncompressed(device: &wgpu::Device) -> Self {
        Self::make(device, Compression::Done)
    }

    fn make(device: &wgpu::Device, compression: Compression) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tree impostors"),
            size: wgpu::Extent3d {
                width: ATLAS_TEXELS,
                height: ATLAS_TEXELS,
                depth_or_array_layers: atlas_layers(),
            },
            mip_level_count: ATLAS_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: ATLAS_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            shared: Arc::new(Shared {
                baked: AtomicBool::new(false),
                current: Mutex::new(texture),
                generation: AtomicU32::new(0),
                compression: Mutex::new(compression),
                done: std::sync::Condvar::new(),
            }),
        }
    }

    /// Whether a land has baked it (in a frame already submitted, or the one being prepared).
    pub fn baked(&self) -> bool {
        self.shared.baked.load(Ordering::Acquire)
    }

    /// The atlas the lands draw now (colour layers, normal layers, the leaf sprites): the bake, or its BC7 copy once
    /// that is ready. For tests and probes.
    pub fn texture(&self) -> wgpu::Texture {
        self.current().0
    }

    /// Waits until the BC7 copy is ready, or returns at once when none is being made (unbaked, or a device without
    /// BC7). For tests and probes; the lands never wait.
    pub fn wait_compressed(&self) {
        let mut state = self
            .shared
            .compression
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while *state == Compression::Running {
            state = self
                .shared
                .done
                .wait(state)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// The texture drawn now and its generation.
    fn current(&self) -> (wgpu::Texture, u32) {
        let texture = self
            .shared
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        (
            texture.clone(),
            self.shared.generation.load(Ordering::Acquire),
        )
    }
}

/// What bakes the atlas: the pipelines, their groups and the scratch the frames are drawn on, held only by a land made
/// before it was baked.
struct Bake {
    raster: PipelineId,
    raster_group: wgpu::BindGroup,
    resolve: PipelineId,
    resolve_group: wgpu::BindGroup,
    /// Each species' index and radius, a uniform at a dynamic offset of `stride` each.
    stride: u32,
    _models: wgpu::Buffer,
    /// The trees' meshes at full detail, drawn from every frame.
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// The scratch: colour, normal, depth, twice the atlas's side.
    colors: wgpu::TextureView,
    normals: wgpu::TextureView,
    depth: wgpu::TextureView,
    mips: PipelineId,
    /// One group per level after the first: the level above read, this one written, and every frame's coverage at
    /// the base level (`tree_mips.wgsl`'s `targets`).
    mip_groups: Vec<wgpu::BindGroup>,
    _targets: wgpu::Buffer,
}

/// A land's hold on its atlas.
pub(crate) struct Impostors {
    /// The texture this land binds, of `generation`.
    pub texture: wgpu::Texture,
    generation: u32,
    atlas: TreeAtlas,
    bake: Option<Bake>,
}

/// The view of one level, every layer.
fn level_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("tree impostors level"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

/// A 1 x 1 placeholder with two layers, for a layer without trees (every land pipeline keeps one layout).
pub(crate) fn placeholder(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("tree impostors placeholder"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 2,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: ATLAS_FORMAT,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

/// The view every land pipeline binds (binding 11): all levels and layers.
pub(crate) fn atlas_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("tree impostors"),
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

impl Impostors {
    /// `shared` (a host's, made on this device), else an atlas of the land's own, with what bakes it unless a land has
    /// baked it already.
    pub fn new(
        ctx: &mut RenderContext<'_>,
        shared: Option<&TreeAtlas>,
    ) -> Result<Self, ShaderError> {
        let atlas = shared
            .cloned()
            .unwrap_or_else(|| TreeAtlas::new(ctx.device));
        let (texture, generation) = atlas.current();
        let bake = if atlas.baked() {
            None
        } else {
            Some(Bake::new(ctx, &texture)?)
        };
        Ok(Self {
            texture,
            generation,
            atlas,
            bake,
        })
    }

    /// The atlas's texture when it changed since this land last bound it (its BC7 copy became ready): the land binds
    /// it again.
    pub fn refresh(&mut self) -> Option<wgpu::Texture> {
        if self.atlas.shared.generation.load(Ordering::Acquire) == self.generation {
            return None;
        }
        let (texture, generation) = self.atlas.current();
        self.texture = texture.clone();
        self.generation = generation;
        Some(texture)
    }

    /// Whether this land holds what bakes the atlas (it was made before any land baked it), for tests and probes.
    pub fn holds_bake(&self) -> bool {
        self.bake.is_some()
    }

    /// Bakes the atlas and its levels, once a device: nothing when a land sharing it has.
    pub fn bake(&mut self, ctx: &mut PrepareContext<'_>) {
        let Some(bake) = self.bake.take() else {
            return;
        };
        if self.atlas.shared.baked.swap(true, Ordering::AcqRel) {
            return;
        }
        let compress = crate::bc7::supported(ctx.device)
            && *self
                .atlas
                .shared
                .compression
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                == Compression::NotStarted;
        // With a BC7 copy to make, the bake goes to the queue on its own now, so the copy's readback (submitted from
        // its thread) follows it; else in the frame's encoder.
        let mut own = compress.then(|| {
            ctx.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("tree impostor bake"),
                })
        });
        let timestamps = if own.is_some() {
            None
        } else {
            ctx.timer.compute_pass("tree impostor mips")
        };
        let count = models().len() as u32;
        let meshes = tree_meshes();
        {
            let encoder = match own.as_mut() {
                Some(own) => own,
                None => &mut *ctx.encoder,
            };
            // Each species' frames drawn on the scratch, then brought down into its layers.
            for (species, ranges) in meshes.ranges.iter().enumerate() {
                let offset = species as u32 * bake.stride;
                let range = ranges[0];
                {
                    let clear = wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    };
                    let target = |view| {
                        Some(wgpu::RenderPassColorAttachment {
                            view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: clear,
                        })
                    };
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("tree impostor bake"),
                        color_attachments: &[target(&bake.colors), target(&bake.normals)],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &bake.depth,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(0.0),
                                store: wgpu::StoreOp::Discard,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    pass.set_pipeline(ctx.pipelines.render(bake.raster));
                    pass.set_bind_group(0, &bake.raster_group, &[offset]);
                    pass.set_vertex_buffer(0, bake.vertices.slice(..));
                    pass.set_index_buffer(bake.indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(
                        range.first_index..range.first_index + range.index_count,
                        range.base_vertex,
                        0..IMPOSTOR_FRAMES * IMPOSTOR_FRAMES,
                    );
                }
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("tree impostor resolve"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(ctx.pipelines.compute(bake.resolve));
                pass.set_bind_group(0, &bake.resolve_group, &[offset]);
                pass.dispatch_workgroups(ATLAS_TEXELS / 8, ATLAS_TEXELS / 8, 1);
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("tree impostor mips"),
                timestamp_writes: timestamps,
            });
            pass.set_pipeline(ctx.pipelines.compute(bake.mips));
            // One workgroup a frame of a species' layer, each level from the one above.
            for group in &bake.mip_groups {
                pass.set_bind_group(0, group, &[]);
                pass.dispatch_workgroups(IMPOSTOR_FRAMES * IMPOSTOR_FRAMES, 1, count);
            }
        }
        if let Some(own) = own {
            ctx.queue.submit([own.finish()]);
            compress::start(ctx.device, ctx.queue, &self.atlas, self.texture.clone());
        }
    }
}

impl Bake {
    fn new(ctx: &mut RenderContext<'_>, texture: &wgpu::Texture) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        // The leaf sprites, painted on the CPU with their levels, into the last layers.
        let leaf_layer = models().len() as u32 * 2;
        for (layer, levels) in leaf_sprites(ATLAS_TEXELS, ATLAS_LEVELS).iter().enumerate() {
            for (level, bytes) in levels.iter().enumerate() {
                let side = ATLAS_TEXELS >> level;
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: leaf_layer + layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    bytes,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(side * 4),
                        rows_per_image: Some(side),
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let buffer = |label: &str, bytes: &[u8], usage: wgpu::BufferUsages| {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: bytes.len() as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytes);
            buffer
        };
        let meshes = tree_meshes();
        let vertices = buffer(
            "tree impostor bake vertices",
            bytemuck::cast_slice(&meshes.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer(
            "tree impostor bake indices",
            bytemuck::cast_slice(&meshes.indices),
            wgpu::BufferUsages::INDEX,
        );
        // Each species' index and radius, one to a uniform's dynamic offset.
        let stride = device.limits().min_uniform_buffer_offset_alignment.max(16);
        let mut table = vec![0u8; (stride as usize) * models().len()];
        for (k, model) in models().iter().enumerate() {
            let at = k * stride as usize;
            table[at..at + 4].copy_from_slice(&(k as u32).to_le_bytes());
            table[at + 4..at + 8].copy_from_slice(&model.radius.to_le_bytes());
        }
        let models_buffer = buffer(
            "tree impostor bake models",
            &table,
            wgpu::BufferUsages::UNIFORM,
        );
        let model_entry = |visibility: wgpu::ShaderStages| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(16),
            },
            count: None,
        };
        let model_binding = wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &models_buffer,
                offset: 0,
                size: wgpu::BufferSize::new(16),
            }),
        };
        // The scratch, twice the atlas's side: four samples an atlas texel.
        let scratch = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: ATLAS_TEXELS * 2,
                        height: ATLAS_TEXELS * 2,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let colors = scratch(
            "tree impostor bake colour",
            SCRATCH_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let normals = scratch(
            "tree impostor bake normal",
            SCRATCH_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = scratch(
            "tree impostor bake depth",
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureUsages::empty(),
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tree impostor bake sprites"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let raster_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tree impostor bake"),
            entries: &[
                model_entry(wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sprites = atlas_view(texture);
        let raster_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tree impostor bake"),
            layout: &raster_layout,
            entries: &[
                model_binding.clone(),
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sprites),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let storage_texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::StorageTexture {
                access: wgpu::StorageTextureAccess::WriteOnly,
                format: ATLAS_FORMAT,
                view_dimension: wgpu::TextureViewDimension::D2Array,
            },
            count: None,
        };
        let read_texture =
            |binding: u32, dimension: wgpu::TextureViewDimension| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: dimension,
                    multisampled: false,
                },
                count: None,
            };
        let resolve_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tree impostor resolve"),
            entries: &[
                model_entry(wgpu::ShaderStages::COMPUTE),
                read_texture(1, wgpu::TextureViewDimension::D2),
                read_texture(2, wgpu::TextureViewDimension::D2),
                storage_texture(3),
            ],
        });
        let base = level_view(texture, 0);
        let resolve_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tree impostor resolve"),
            layout: &resolve_layout,
            entries: &[
                model_binding,
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&colors),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&normals),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&base),
                },
            ],
        });
        let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tree impostor mips"),
            entries: &[
                read_texture(0, wgpu::TextureViewDimension::D2Array),
                storage_texture(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        // Every species' frames' share of texels at the impostors' edge at the base level, as the first level counts
        // it for the levels after it.
        let targets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tree impostor coverage"),
            size: u64::from(IMPOSTOR_FRAMES * IMPOSTOR_FRAMES) * models().len() as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let mip_groups = (1..ATLAS_LEVELS)
            .map(|level| {
                let above = level_view(texture, level - 1);
                let this = level_view(texture, level);
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("tree impostor mips"),
                    layout: &mip_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&above),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&this),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: targets.as_entire_binding(),
                        },
                    ],
                })
            })
            .collect();
        let raster = ctx.add_pipeline(
            "tree impostor bake",
            "naval-terrain/shaders/tree_bake.wgsl",
            Vec::new(),
            raster_recipe(raster_layout),
        )?;
        let resolve = ctx.add_pipeline(
            "tree impostor resolve",
            "naval-terrain/shaders/tree_resolve.wgsl",
            Vec::new(),
            compute_recipe(vec![resolve_layout], "resolve"),
        )?;
        let mips = ctx.add_pipeline(
            "tree impostor mips",
            "naval-terrain/shaders/tree_mips.wgsl",
            Vec::new(),
            compute_recipe(vec![mip_layout], "downsample"),
        )?;
        Ok(Self {
            raster,
            raster_group,
            resolve,
            resolve_group,
            stride,
            _models: models_buffer,
            vertices,
            indices,
            colors,
            normals,
            depth,
            mips,
            mip_groups,
            _targets: targets,
        })
    }
}

/// The scratch's format: the colour linear and the normals' code exact enough to average.
const SCRATCH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The bake's draw of the trees' meshes into the scratch (`tree_bake.wgsl`): both faces, the nearest kept.
fn raster_recipe(layout: wgpu::BindGroupLayout) -> naval_render::PipelineRecipe {
    Box::new(move |build| {
        let pipeline_layout =
            build
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(build.label),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                });
        let attribute =
            |location: u32, format: wgpu::VertexFormat, offset: u64| wgpu::VertexAttribute {
                format,
                offset,
                shader_location: location,
            };
        use wgpu::VertexFormat as F;
        let attributes = [
            attribute(0, F::Float32x3, 0),
            attribute(1, F::Snorm8x4, 12),
            attribute(2, F::Snorm8x4, 16),
            attribute(3, F::Unorm16x2, 20),
            attribute(4, F::Unorm8x4, 24),
            attribute(5, F::Unorm8x4, 28),
        ];
        let target = Some(wgpu::ColorTargetState {
            format: SCRATCH_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        });
        naval_render::Pipeline::Render(build.device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some(build.label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: build.module,
                    entry_point: Some("vs_bake"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: MESH_VERTEX_BYTES,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Greater),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: build.module,
                    entry_point: Some("fs_bake"),
                    compilation_options: Default::default(),
                    targets: &[target.clone(), target],
                }),
                multiview_mask: None,
                cache: build.cache,
            },
        ))
    })
}

/// No BC7 copy on wasm32 (`crate::bc7::supported` is false there, and its wgpu handles cannot go to another thread).
#[cfg(target_arch = "wasm32")]
mod compress {
    use super::*;

    pub(super) fn start(_: &wgpu::Device, _: &wgpu::Queue, _: &TreeAtlas, _: wgpu::Texture) {}
}

/// The BC7 copy of a baked atlas, made on a thread of its own.
#[cfg(not(target_arch = "wasm32"))]
mod compress {
    use super::*;

    /// Layers read back at a time: about 25 MB of staging.
    const LAYERS_AT_ONCE: u32 = 8;

    /// Starts making `atlas`'s BC7 copy from `baked` (its bake submitted already), once an atlas.
    pub(super) fn start(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &TreeAtlas,
        baked: wgpu::Texture,
    ) {
        {
            let mut state = atlas
                .shared
                .compression
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if *state != Compression::NotStarted {
                return;
            }
            *state = Compression::Running;
        }
        let (device, queue, shared) = (device.clone(), queue.clone(), atlas.shared.clone());
        let finish = |shared: &Shared| {
            *shared.compression.lock().unwrap_or_else(|e| e.into_inner()) = Compression::Done;
            shared.done.notify_all();
        };
        let work = move || {
            let started = std::time::Instant::now();
            match compressed(&device, &queue, &baked) {
                Some(texture) => {
                    *shared.current.lock().unwrap_or_else(|e| e.into_inner()) = texture;
                    shared.generation.fetch_add(1, Ordering::AcqRel);
                    eprintln!(
                        "[terrain] tree impostors: BC7, {:.0} MB in place of {:.0} MB, in {:.1} s",
                        (atlas_bytes() / 4) as f64 / 1e6,
                        atlas_bytes() as f64 / 1e6,
                        started.elapsed().as_secs_f64()
                    );
                }
                None => eprintln!("[terrain] tree impostors: no BC7 copy; drawing the RGBA8 bake"),
            }
            finish(&shared);
        };
        if let Err(error) = std::thread::Builder::new()
            .name("tree impostors bc7".into())
            .spawn(work)
        {
            eprintln!(
                "[terrain] tree impostors: no thread for the BC7 copy ({error}); drawing the RGBA8 bake"
            );
            finish(&atlas.shared);
        }
    }

    /// Bytes a row of a level takes in the readback (rows are 256-byte aligned).
    fn padded_row(side: u32) -> u32 {
        (side * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    }

    /// `baked` read back a few layers at a time and every level encoded, into a new BC7 texture; None if a readback
    /// fails or the GPU had no room for the copy or its staging (the error is this thread's, caught here: the lands keep
    /// the RGBA8 bake rather than the client stopping).
    fn compressed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        baked: &wgpu::Texture,
    ) -> Option<wgpu::Texture> {
        let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let invalid = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let made = encoded(device, queue, baked);
        let invalid = pollster::block_on(invalid.pop());
        let out_of_memory = pollster::block_on(out_of_memory.pop());
        if let Some(error) = out_of_memory.or(invalid) {
            eprintln!("[terrain] tree impostors: the BC7 copy failed ({error})");
            return None;
        }
        made
    }

    fn encoded(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        baked: &wgpu::Texture,
    ) -> Option<wgpu::Texture> {
        let layers = atlas_layers();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tree impostors bc7"),
            size: wgpu::Extent3d {
                width: ATLAS_TEXELS,
                height: ATLAS_TEXELS,
                depth_or_array_layers: layers,
            },
            mip_level_count: ATLAS_LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COMPRESSED_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut first = 0;
        while first < layers {
            let count = LAYERS_AT_ONCE.min(layers - first);
            // Each level's layers one after another, rows padded.
            let mut offsets = Vec::with_capacity(ATLAS_LEVELS as usize);
            let mut size = 0u64;
            for level in 0..ATLAS_LEVELS {
                let side = ATLAS_TEXELS >> level;
                offsets.push(size);
                size += u64::from(padded_row(side)) * u64::from(side) * u64::from(count);
            }
            let staging = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("tree impostors readback"),
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tree impostors readback"),
            });
            for level in 0..ATLAS_LEVELS {
                let side = ATLAS_TEXELS >> level;
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: baked,
                        mip_level: level,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: first,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &staging,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: offsets[level as usize],
                            bytes_per_row: Some(padded_row(side)),
                            rows_per_image: Some(side),
                        },
                    },
                    wgpu::Extent3d {
                        width: side,
                        height: side,
                        depth_or_array_layers: count,
                    },
                );
            }
            queue.submit([encoder.finish()]);
            let (sender, receiver) = std::sync::mpsc::channel();
            staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let _ = sender.send(result);
                });
            device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
            receiver.recv().ok()?.ok()?;
            {
                let mapped = staging.slice(..).get_mapped_range().ok()?;
                for level in 0..ATLAS_LEVELS {
                    let side = ATLAS_TEXELS >> level;
                    let row = padded_row(side) as usize;
                    let tight = side as usize * 4;
                    for k in 0..count {
                        let at =
                            offsets[level as usize] as usize + k as usize * row * side as usize;
                        let mut texels = Vec::with_capacity(tight * side as usize);
                        for y in 0..side as usize {
                            texels.extend_from_slice(&mapped[at + y * row..at + y * row + tight]);
                        }
                        let blocks = crate::bc7::encode(&texels, side, side);
                        queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture: &texture,
                                mip_level: level,
                                origin: wgpu::Origin3d {
                                    x: 0,
                                    y: 0,
                                    z: first + k,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &blocks,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(side / 4 * crate::bc7::BLOCK_BYTES as u32),
                                rows_per_image: Some(side / 4),
                            },
                            wgpu::Extent3d {
                                width: side,
                                height: side,
                                depth_or_array_layers: 1,
                            },
                        );
                    }
                }
            }
            staging.unmap();
            first += count;
        }
        // The writes reach the GPU with the next submission, before any frame that binds the copy.
        queue.submit([]);
        Some(texture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shaders_share_the_atlas_layout() {
        let impostor = include_str!("../../shaders/impostor.wgsl");
        assert!(impostor.contains(&format!(
            "const IMPOSTOR_FRAMES: f32 = {IMPOSTOR_FRAMES}.0;"
        )));
        assert!(impostor.contains(&format!("const FRAME_TEXELS: f32 = {FRAME_TEXELS}.0;")));
        assert!(impostor.contains(&format!("const LEAF_LAYERS: u32 = {LEAF_LAYERS}u;")));
        assert!(impostor.contains(&format!(
            "const LEAN_MOST: f32 = {:?};",
            crate::trees::TREE_LEAN_MOST
        )));
        assert!(impostor.contains(&format!(
            "const STRETCH_MOST: f32 = {:?};",
            crate::trees::TREE_STRETCH_MOST
        )));
        let bin = include_str!("../../shaders/tree_near_bin.wgsl");
        for shader in [impostor, bin] {
            assert!(shader.contains(&format!(
                "const PALM_LEAN_MOST: f32 = {:?};",
                crate::trees::TREE_PALM_LEAN_MOST
            )));
            assert!(shader.contains("const PALM_FORM: u32 = 16384u;"));
        }
        assert!(bin.contains(&format!(
            "const LEAN_MOST: f32 = {:?};",
            crate::trees::TREE_LEAN_MOST
        )));
        assert!(bin.contains(&format!(
            "const STRETCH_MOST: f32 = {:?};",
            crate::trees::TREE_STRETCH_MOST
        )));
        // Each level halves every frame, down to whole texels.
        assert_eq!(FRAME_TEXELS % (1 << (ATLAS_LEVELS - 1)), 0);
        const {
            assert!(
                ATLAS_TEXELS.is_multiple_of(8)
                    && ATLAS_TEXELS.is_multiple_of(crate::tree_meshes::SPRITE_GRID)
            )
        };
        // The levels keep the coverage at the edge the impostors (and their casters) cut at.
        let edge = "const EDGE: f32 = 0.45;";
        assert!(include_str!("../../shaders/trees.wgsl").contains(edge));
        assert!(include_str!("../../shaders/tree_mips.wgsl").contains(edge));
        assert!(include_str!("../../shaders/tree_caster.wgsl").contains("coverage < 0.45"));
        let surface = include_str!("../../shaders/tree_surface.wgsl");
        assert!(surface.contains(&format!(
            "const TWIG_RED: f32 = {:?};",
            crate::tree_meshes::TWIG_RED
        )));
        assert!(surface.contains(&format!(
            "const BARK_RIDGES: f32 = {:?};",
            crate::tree_meshes::BARK_RIDGES
        )));
        assert!(surface.contains(&format!(
            "const BARK_ALONG_M: f32 = {:?};",
            crate::tree_meshes::BARK_ALONG_M
        )));
    }
}
