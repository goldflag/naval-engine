//! Puffs of smoke and fire, lit six ways and baked once on the GPU: the web client's `GasAtlas`
//! (the web game's `GasAtlas.ts`). See `shaders/gas_bake.wgsl` for the kernels.
//!
//! Each 256² tile of the 1024² atlas is one puff: round lobes swollen into billows by tiling Worley noise, seen
//! along −Z and raymarched finely enough to keep every fold. A sprite relights it for any sun, moon, sky or flash
//! with two texture reads (the "six-way" lighting of film and game effects) instead of marching the gas per
//! pixel. A small tiling `detail` map of billows lets close sprites erode their thin rims into cauliflower.

use bytemuck::{Pod, Zeroable};
use naval_render::{Pipeline, PipelineId, PipelineRecipe, RenderContext, ShaderError};

use crate::gpu::{sampler, sampler_entry, storage_texture_entry, texture_entry, uniform_entry};
use crate::random::Lcg;

/// Texels across one baked puff, puffs across the atlas, and so the atlas edge.
pub const GAS_TILE: u32 = 256;
pub const GAS_GRID: u32 = 4;
pub const GAS_VARIANTS: u32 = GAS_GRID * GAS_GRID;
pub const GAS_ATLAS: u32 = GAS_TILE * GAS_GRID;
/// Round lobes each puff is built from before the billows erode it.
pub const LOBES: usize = 12;
/// Texels per edge of the tiling detail map.
pub const DETAIL: u32 = 256;
/// Texels per edge of the tiling noise volume the bake reads.
pub const NOISE: u32 = 64;
/// Optical depth that fills the atlas's depth channel.
pub const GAS_DEPTH: f32 = 12.0;

const SHADER: naval_render::ShaderFile = naval_render::shader_file!("shaders/gas_bake.wgsl");
const MIPS: naval_render::ShaderFile = naval_render::shader_file!("shaders/mips.wgsl");

/// Each puff's lobes as (centre, radius) in its unit sphere: a large body with smaller lobes swelling out of it,
/// all inside radius 0.92 so the billows that grow on them never reach the tile's edge. `gasLobes()`, from the
/// removed web game's seeded stream (1941), so every load draws the same puffs.
pub fn gas_lobes() -> Vec<[f32; 4]> {
    let mut random = Lcg::new(1941);
    let mut lobes: Vec<[f64; 4]> = Vec::with_capacity(GAS_VARIANTS as usize * LOBES);
    for v in 0..GAS_VARIANTS as usize {
        let twin = v % 5 == 3;
        let lean = if v % 4 == 1 { 0.16 } else { 0.0 };
        let body = if twin {
            0.4
        } else {
            0.47 + random.next() * 0.08
        };
        let heading = random.next() * std::f64::consts::TAU;
        if twin {
            lobes.push([heading.cos() * 0.22, -0.05, heading.sin() * 0.22, body]);
            lobes.push([
                -heading.cos() * 0.24,
                0.04,
                -heading.sin() * 0.24,
                body * 0.92,
            ]);
        } else {
            lobes.push([heading.cos() * lean, -0.04, heading.sin() * lean, body]);
        }
        while lobes.len() < (v + 1) * LOBES {
            // Lobes swell mostly outward and up, as a rising, rolling cloud's do.
            let k = lobes.len() - v * LOBES;
            let small = k > 6;
            let yaw = random.next() * std::f64::consts::TAU;
            let y = -0.55 + random.next() * 1.4;
            let flat = (1.0 - (y * y).min(1.0)).max(0.0).sqrt();
            let reach = (if small { 0.42 } else { 0.26 }) + random.next() * 0.2;
            let radius = (0.92 - reach).min(
                (if small { 0.12 } else { 0.2 }) + random.next() * if small { 0.12 } else { 0.17 },
            );
            lobes.push([
                yaw.cos() * flat * reach,
                y * reach,
                yaw.sin() * flat * reach,
                radius,
            ]);
        }
    }
    lobes
        .into_iter()
        .map(|l| [l[0] as f32, l[1] as f32, l[2] as f32, l[3] as f32])
        .collect()
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BakeParams {
    row: u32,
    padding: [u32; 3],
}

/// The baked atlas: `sides` (light from +X, −X, +Y, −Y), `body` (front, back ÷ 2, optical depth ÷ 12,
/// temperature) and the tiling `detail` map, each with its mip chain. Cloning it shares the textures.
#[derive(Clone)]
pub struct GasAtlas {
    pub sides: wgpu::Texture,
    pub body: wgpu::Texture,
    pub detail: wgpu::Texture,
    pub sides_view: wgpu::TextureView,
    pub body_view: wgpu::TextureView,
    pub detail_view: wgpu::TextureView,
}

fn atlas_texture(device: &wgpu::Device, label: &str, size: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: size.ilog2() + 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// The atlas's three textures as a pack stores them, in this order: name and edge (texels). Each is `Rgba8Unorm`
/// with its whole mip chain (`edge.ilog2() + 1` levels).
pub const BAKED_TEXTURES: [(&str, u32); 3] = [
    ("sides", GAS_ATLAS),
    ("body", GAS_ATLAS),
    ("detail", DETAIL),
];

/// The baked atlas read back from the GPU, for a pack (naval-assets bakes it at pack time and the client loads it
/// instead of baking, [`GasAtlas::from_baked`]): every level of [`BAKED_TEXTURES`], RGBA8, rows tightly packed.
/// `levels[t][l]` is texture `t`'s level `l`.
#[derive(Clone, Debug, PartialEq)]
pub struct BakedGasAtlas {
    pub levels: [Vec<Vec<u8>>; 3],
}

impl BakedGasAtlas {
    /// Level `level`'s edge of a texture whose base is `edge` texels.
    pub fn level_edge(edge: u32, level: u32) -> u32 {
        (edge >> level).max(1)
    }

    /// The bytes of every level of every texture.
    pub fn byte_len() -> usize {
        BAKED_TEXTURES
            .iter()
            .flat_map(|&(_, edge)| {
                (0..=edge.ilog2()).map(move |l| (Self::level_edge(edge, l) as usize).pow(2) * 4)
            })
            .sum()
    }

    /// Every level, texture after texture, as one byte string (a pack's payload).
    pub fn to_bytes(&self) -> Vec<u8> {
        self.levels.iter().flatten().flatten().copied().collect()
    }

    /// The atlas from [`BakedGasAtlas::to_bytes`]'s bytes, refused unless they are exactly the layout's length.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() != Self::byte_len() {
            return Err(format!(
                "a baked gas atlas is {} bytes, these are {}",
                Self::byte_len(),
                bytes.len()
            ));
        }
        let mut at = 0;
        let levels = BAKED_TEXTURES.map(|(_, edge)| {
            (0..=edge.ilog2())
                .map(|l| {
                    let len = (Self::level_edge(edge, l) as usize).pow(2) * 4;
                    at += len;
                    bytes[at - len..at].to_vec()
                })
                .collect()
        });
        Ok(Self { levels })
    }

    /// Whether every level holds its edge's texels.
    pub fn is_consistent(&self) -> bool {
        self.levels
            .iter()
            .zip(BAKED_TEXTURES)
            .all(|(levels, (_, edge))| {
                levels.len() == edge.ilog2() as usize + 1
                    && levels.iter().enumerate().all(|(l, data)| {
                        data.len() == (Self::level_edge(edge, l as u32) as usize).pow(2) * 4
                    })
            })
    }
}

fn level_view(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

fn compute_recipe(layout: wgpu::BindGroupLayout, entry: &'static str) -> PipelineRecipe {
    Box::new(move |build| {
        let pipeline_layout =
            build
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(build.label),
                    bind_group_layouts: &[Some(&layout)],
                    immediate_size: 0,
                });
        Pipeline::Compute(
            build
                .device
                .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(build.label),
                    layout: Some(&pipeline_layout),
                    module: build.module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: build.cache,
                }),
        )
    })
}

impl GasAtlas {
    /// The atlas from a pack's bake ([`BakedGasAtlas`]): each texture's levels uploaded as they are, no pipeline
    /// built and nothing dispatched. `None` when the levels do not fit the layout (the caller bakes instead).
    pub fn from_baked(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        baked: &BakedGasAtlas,
    ) -> Option<Self> {
        if !baked.is_consistent() {
            return None;
        }
        let labels = ["gas puffs, sides", "gas puffs, body", "gas puff detail"];
        let [sides, body, detail] = std::array::from_fn(|t| {
            let edge = BAKED_TEXTURES[t].1;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(labels[t]),
                size: wgpu::Extent3d {
                    width: edge,
                    height: edge,
                    depth_or_array_layers: 1,
                },
                mip_level_count: edge.ilog2() + 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            for (level, data) in baked.levels[t].iter().enumerate() {
                let side = BakedGasAtlas::level_edge(edge, level as u32);
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    data,
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
            texture
        });
        Some(Self {
            sides_view: sides.create_view(&wgpu::TextureViewDescriptor::default()),
            body_view: body.create_view(&wgpu::TextureViewDescriptor::default()),
            detail_view: detail.create_view(&wgpu::TextureViewDescriptor::default()),
            sides,
            body,
            detail,
        })
    }

    /// The atlas's textures in [`BAKED_TEXTURES`] order (each readable with `COPY_SRC`, for a pack's builder).
    pub fn textures(&self) -> [&wgpu::Texture; 3] {
        [&self.sides, &self.body, &self.detail]
    }

    /// Makes the textures and submits the bake: the noise volume, one submission per row of puffs (a whole
    /// atlas in one dispatch would hold a slow GPU for too long), the detail map and every mip level. Nothing
    /// waits for the GPU; the first frame that samples the atlas follows the bake in queue order.
    pub fn bake(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        let device = ctx.device.clone();
        let queue = ctx.queue.clone();
        ctx.add_shader(SHADER)?;
        ctx.add_shader(MIPS)?;
        let compute = wgpu::ShaderStages::COMPUTE;
        let fill_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gas bake noise"),
            entries: &[storage_texture_entry(0, wgpu::TextureViewDimension::D3)],
        });
        let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gas bake puffs"),
            entries: &[
                texture_entry(1, compute, wgpu::TextureViewDimension::D3, true),
                sampler_entry(2, compute),
                uniform_entry(3, compute),
                uniform_entry(4, compute),
                storage_texture_entry(5, wgpu::TextureViewDimension::D2),
                storage_texture_entry(6, wgpu::TextureViewDimension::D2),
            ],
        });
        let detail_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gas bake detail"),
            entries: &[storage_texture_entry(7, wgpu::TextureViewDimension::D2)],
        });
        let mips_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gas atlas mips"),
            entries: &[
                texture_entry(0, compute, wgpu::TextureViewDimension::D2, false),
                storage_texture_entry(1, wgpu::TextureViewDimension::D2),
            ],
        });
        let fill: PipelineId = ctx.add_pipeline(
            "gas bake noise",
            SHADER.path,
            Vec::new(),
            compute_recipe(fill_layout.clone(), "fill_noise"),
        )?;
        let bake: PipelineId = ctx.add_pipeline(
            "gas bake puffs",
            SHADER.path,
            Vec::new(),
            compute_recipe(bake_layout.clone(), "bake"),
        )?;
        let detail_pipeline: PipelineId = ctx.add_pipeline(
            "gas bake detail",
            SHADER.path,
            Vec::new(),
            compute_recipe(detail_layout.clone(), "bake_detail"),
        )?;
        let downsample: PipelineId = ctx.add_pipeline(
            "gas atlas mips",
            MIPS.path,
            Vec::new(),
            compute_recipe(mips_layout.clone(), "downsample"),
        )?;

        let noise = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gas puff noise"),
            size: wgpu::Extent3d {
                width: NOISE,
                height: NOISE,
                depth_or_array_layers: NOISE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let noise_view = noise.create_view(&wgpu::TextureViewDescriptor::default());
        let sides = atlas_texture(&device, "gas puffs, sides", GAS_ATLAS);
        let body = atlas_texture(&device, "gas puffs, body", GAS_ATLAS);
        let detail = atlas_texture(&device, "gas puff detail", DETAIL);
        let lobes = gas_lobes();
        let lobes_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gas lobes"),
            size: (lobes.len() * 16) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&lobes_buffer, 0, bytemuck::cast_slice(&lobes));
        let noise_sampler = sampler(&device, "gas noise", wgpu::AddressMode::Repeat, false, 1);

        let pipelines = &*ctx.pipelines;
        // The noise volume.
        let fill_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gas bake noise"),
            layout: &fill_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&noise_view),
            }],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gas bake noise"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gas bake noise"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipelines.compute(fill));
            pass.set_bind_group(0, &fill_group, &[]);
            pass.dispatch_workgroups((NOISE * NOISE * NOISE).div_ceil(64), 1, 1);
        }
        queue.submit([encoder.finish()]);

        // One row of puffs per submission.
        let sides_level = level_view(&sides, 0);
        let body_level = level_view(&body, 0);
        for row in 0..GAS_GRID {
            let params = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gas bake row"),
                size: std::mem::size_of::<BakeParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(
                &params,
                0,
                bytemuck::bytes_of(&BakeParams {
                    row,
                    padding: [0; 3],
                }),
            );
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gas bake puffs"),
                layout: &bake_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&noise_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&noise_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: lobes_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&sides_level),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&body_level),
                    },
                ],
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gas bake row"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("gas bake row"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipelines.compute(bake));
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups((GAS_ATLAS * GAS_TILE).div_ceil(64), 1, 1);
            }
            queue.submit([encoder.finish()]);
        }

        // The detail map, then every mip chain.
        let detail_level = level_view(&detail, 0);
        let detail_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gas bake detail"),
            layout: &detail_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&detail_level),
            }],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gas atlas detail and mips"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gas bake detail"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipelines.compute(detail_pipeline));
            pass.set_bind_group(0, &detail_group, &[]);
            pass.dispatch_workgroups((DETAIL * DETAIL).div_ceil(64), 1, 1);
        }
        for texture in [&sides, &body, &detail] {
            for level in 1..texture.mip_level_count() {
                let source = level_view(texture, level - 1);
                let target = level_view(texture, level);
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("gas atlas mip"),
                    layout: &mips_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&source),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&target),
                        },
                    ],
                });
                let edge = (texture.width() >> level).max(1);
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("gas atlas mip"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(pipelines.compute(downsample));
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(edge.div_ceil(8), edge.div_ceil(8), 1);
            }
        }
        queue.submit([encoder.finish()]);
        // The kernels only served the bake.
        for id in [fill, bake, detail_pipeline, downsample] {
            ctx.pipelines.remove(id);
        }
        Ok(Self {
            sides_view: sides.create_view(&wgpu::TextureViewDescriptor::default()),
            body_view: body.create_view(&wgpu::TextureViewDescriptor::default()),
            detail_view: detail.create_view(&wgpu::TextureViewDescriptor::default()),
            sides,
            body,
            detail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobes_stay_inside_the_tile() {
        let lobes = gas_lobes();
        assert_eq!(lobes.len(), 192);
        for lobe in &lobes {
            let reach =
                (lobe[0] * lobe[0] + lobe[1] * lobe[1] + lobe[2] * lobe[2]).sqrt() + lobe[3];
            assert!(reach <= 0.92 + 1e-6, "{lobe:?}");
        }
    }
}
