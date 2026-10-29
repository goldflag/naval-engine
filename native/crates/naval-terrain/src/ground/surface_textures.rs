//! The land layer's real-world surface textures (native/docs/terrain/surface.md): one colour and one
//! cover texture array, bound at the terrain group's bindings 12 and 13. Layer 0 holds the window's maps
//! (`TerrainLayer::set_surface_maps`), read clamped; layer 1, there only with a near grid, the near tiles about the
//! camera (`TerrainLayer::set_near_grid`, `queue_near_tile`), read repeating, tile (i, j) in slot (i mod slots, j mod
//! slots), so a tile coming into the block overwrites only the slot of the one leaving it and the rest stay in place.
//! The near tiles take no binding of their own: the land pipelines already sample 16 textures a stage, the default
//! limit.
//!
//! A tile is uploaded whole within one frame's `prepare` (its mips built off the frame loop's thread), and the slot's
//! entry in the uniforms changes in the same frame, so a slot never draws half a tile.
//!
//! On a device that samples BC7 the colour is BC7 (`crate::bc7`, a quarter of RGBA8's bytes: 45 MB in place of 179 MB
//! for a sea window's two 4096² layers) with [`NEAR_BC7_LEVELS`] levels, each slot's down to whole blocks: the window's
//! maps are encoded as they are handed in, the near tiles where they are made (`NearTile::from_maps_bc7`).

use std::collections::VecDeque;
use std::sync::Arc;

use crate::surface_maps::{
    CoverLayer, CoverPatch, NEAR_BC7_LEVELS, NEAR_LEVELS, NEAR_TILE_TEXELS, NearGrid, NearTile,
    SurfaceMaps,
};
use crate::textures::{mip_count, mips_srgb8_premultiplied};

/// Most slots a side (the uniforms' table holds 64 slots: 8 × 8).
pub const NEAR_MAX_SLOTS: u32 = 8;

/// Bytes of near tiles uploaded in a frame at most (at least one tile): a tile is 1.66 MB, so two a frame, a sea
/// window's 64 in about half a second at 60 frames a second.
pub const NEAR_UPLOAD_BYTES: usize = 4 << 20;

/// What the shaders read for a slot holding no tile.
pub const NO_TILE: i32 = i32::MIN;

struct Arrays {
    albedo: wgpu::Texture,
    cover: wgpu::Texture,
    views: (wgpu::TextureView, wgpu::TextureView),
    size: u32,
    levels: u32,
    layers: u32,
    /// The colour is BC7 (else RGBA8).
    bc7: bool,
}

struct Near {
    grid: NearGrid,
    /// The tile each slot holds, and the host's number for the marks over its cover (`NearTile::marks`).
    held: Vec<Option<[i32; 2]>>,
    marks: Vec<u64>,
    /// Tiles waiting for their upload, oldest first.
    queue: VecDeque<([i32; 2], Arc<NearTile>)>,
}

pub(crate) struct SurfaceTextures {
    placeholder: (wgpu::TextureView, wgpu::TextureView),
    arrays: Option<Arrays>,
    far: bool,
    near: Option<Near>,
    /// Rewrites of the cover waiting for the next frame's upload (`TerrainLayer::queue_cover_patch`).
    patches: VecDeque<CoverPatch>,
}

fn array_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    })
}

fn array_texture(
    device: &wgpu::Device,
    label: &str,
    format: wgpu::TextureFormat,
    size: u32,
    levels: u32,
    layers: u32,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// How a written square's bytes are laid out: texels of so many bytes, or BC7 blocks.
#[derive(Clone, Copy)]
enum Texels {
    Bytes(u32),
    Bc7,
}

/// Writes `bytes` (`side`² texels laid out as `texels`) at texel (`x`, `y`) of `level` in `layer`.
#[allow(clippy::too_many_arguments)]
fn write(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    level: u32,
    layer: u32,
    x: u32,
    y: u32,
    side: u32,
    texels: Texels,
    bytes: &[u8],
) {
    let (bytes_per_row, rows) = match texels {
        Texels::Bytes(texel) => (side * texel, side),
        Texels::Bc7 => (side / 4 * crate::bc7::BLOCK_BYTES as u32, side / 4),
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: level,
            origin: wgpu::Origin3d { x, y, z: layer },
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(rows),
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
}

impl SurfaceTextures {
    /// No maps: 1 × 1 placeholders of two layers (an empty colour, an unknown cover).
    pub fn new(device: &wgpu::Device) -> Self {
        let albedo = array_texture(
            device,
            "terrain surface colour placeholder",
            wgpu::TextureFormat::Rgba8UnormSrgb,
            1,
            1,
            2,
        );
        let cover = array_texture(
            device,
            "terrain surface cover placeholder",
            wgpu::TextureFormat::R8Uint,
            1,
            1,
            2,
        );
        // wgpu clears a texture never written to zeros: alpha 0, cover 0.
        Self {
            placeholder: (array_view(&albedo), array_view(&cover)),
            arrays: None,
            far: false,
            near: None,
            patches: VecDeque::new(),
        }
    }

    /// The views bindings 12 and 13 take.
    pub fn views(&self) -> (&wgpu::TextureView, &wgpu::TextureView) {
        let (a, c) = match &self.arrays {
            Some(arrays) => (&arrays.views.0, &arrays.views.1),
            None => (&self.placeholder.0, &self.placeholder.1),
        };
        (a, c)
    }

    /// Textures `size` texels a side of `layers` layers (2 with near tiles), made when they are not; true when the views
    /// changed (and what they held is gone).
    fn ensure(&mut self, device: &wgpu::Device, size: u32, layers: u32) -> bool {
        if self
            .arrays
            .as_ref()
            .is_some_and(|a| a.size == size && a.layers == layers)
        {
            return false;
        }
        // BC7 where every level of every slot is whole blocks (the near tiles' grid is 512 texels a slot).
        let bc7 = crate::bc7::supported(device)
            && size.is_multiple_of(NEAR_TILE_TEXELS)
            && (NEAR_TILE_TEXELS >> (NEAR_BC7_LEVELS - 1)).is_multiple_of(4);
        let levels = mip_count(size, size).min(if bc7 { NEAR_BC7_LEVELS } else { NEAR_LEVELS });
        let albedo = array_texture(
            device,
            "terrain surface colour",
            if bc7 {
                wgpu::TextureFormat::Bc7RgbaUnormSrgb
            } else {
                wgpu::TextureFormat::Rgba8UnormSrgb
            },
            size,
            levels,
            layers,
        );
        let cover = array_texture(
            device,
            "terrain surface cover",
            wgpu::TextureFormat::R8Uint,
            size,
            1,
            layers,
        );
        self.arrays = Some(Arrays {
            views: (array_view(&albedo), array_view(&cover)),
            albedo,
            cover,
            size,
            levels,
            layers,
            bc7,
        });
        // Every slot was in the old textures.
        if let Some(near) = &mut self.near {
            near.held.iter_mut().for_each(|h| *h = None);
        }
        true
    }

    /// Uploads the window's maps to layer 0 (their colour's mips built here), their cover `classes` (the maps' own, or
    /// marked over them); true when the views changed. Near tiles of another size are dropped, and every rewrite of the
    /// window's cover waiting.
    pub fn set_far(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        maps: &SurfaceMaps,
        classes: &[u8],
    ) -> bool {
        self.patches.retain(|p| p.layer != CoverLayer::Window);
        if self
            .near
            .as_ref()
            .is_some_and(|n| n.grid.texels() != maps.size)
        {
            self.near = None;
        }
        let layers = if self.near.is_some() { 2 } else { 1 };
        let changed = self.ensure(device, maps.size, layers);
        let arrays = self.arrays.as_ref().expect("made");
        let mut levels = mips_srgb8_premultiplied(&maps.albedo, maps.size, maps.size);
        levels.truncate(arrays.levels as usize);
        for (k, bytes) in levels.iter().enumerate() {
            let side = (maps.size >> k).max(1);
            if arrays.bc7 {
                let blocks = crate::bc7::encode(bytes, side, side);
                write(
                    queue,
                    &arrays.albedo,
                    k as u32,
                    0,
                    0,
                    0,
                    side,
                    Texels::Bc7,
                    &blocks,
                );
            } else {
                write(
                    queue,
                    &arrays.albedo,
                    k as u32,
                    0,
                    0,
                    0,
                    side,
                    Texels::Bytes(4),
                    bytes,
                );
            }
        }
        write(
            queue,
            &arrays.cover,
            0,
            0,
            0,
            0,
            maps.size,
            Texels::Bytes(1),
            classes,
        );
        self.far = true;
        changed
    }

    /// No window maps: true when the views changed (back to the placeholders with no near tiles either). With near
    /// tiles, layer 0 stays as it was and the shaders read none of it (`terrain.surface` is zero).
    pub fn clear_far(&mut self) -> bool {
        self.far = false;
        if self.near.is_none() && self.arrays.is_some() {
            self.arrays = None;
            return true;
        }
        false
    }

    pub fn has_far(&self) -> bool {
        self.far
    }

    /// Takes near tiles on `grid`; true when the views changed. Refused when the window's maps are another size, or were
    /// handed in first (their textures have no layer for the tiles: set the grid before them).
    pub fn set_near_grid(&mut self, device: &wgpu::Device, grid: NearGrid) -> Result<bool, String> {
        if grid.slots == 0 || grid.slots > NEAR_MAX_SLOTS || grid.tile_m <= 0.0 {
            return Err(format!(
                "near tiles: {} a side of {} m",
                grid.slots, grid.tile_m
            ));
        }
        if self.far
            && let Some(arrays) = &self.arrays
        {
            if arrays.size != grid.texels() {
                return Err(format!(
                    "near tiles: {}² texels where the window's maps are {}²",
                    grid.texels(),
                    arrays.size
                ));
            }
            if arrays.layers < 2 {
                return Err("near tiles: the grid comes before the window's maps".into());
            }
        }
        let largest = device.limits().max_texture_dimension_2d;
        if grid.texels() > largest {
            return Err(format!(
                "near tiles: {0}² texels, more than this device's {largest}² textures",
                grid.texels()
            ));
        }
        if self.near.as_ref().is_some_and(|n| n.grid == grid) {
            return Ok(false);
        }
        self.near = Some(Near {
            grid,
            held: vec![None; (grid.slots * grid.slots) as usize],
            marks: vec![0; (grid.slots * grid.slots) as usize],
            queue: VecDeque::new(),
        });
        Ok(self.ensure(device, grid.texels(), 2))
    }

    /// No near tiles: true when the views changed.
    pub fn clear_near(&mut self) -> bool {
        self.near = None;
        if !self.far && self.arrays.is_some() {
            self.arrays = None;
            return true;
        }
        false
    }

    pub fn near_grid(&self) -> Option<NearGrid> {
        self.near.as_ref().map(|n| n.grid)
    }

    /// Whether tile `index` is drawn or waiting for its upload.
    pub fn near_holds(&self, index: [i32; 2]) -> bool {
        let Some(near) = &self.near else {
            return false;
        };
        let [i, j] = near.grid.slot(index);
        near.held[(j * near.grid.slots + i) as usize] == Some(index)
            || near.queue.iter().any(|(k, _)| *k == index)
    }

    /// Tiles drawn, and waiting for their upload.
    pub fn near_counts(&self) -> (usize, usize) {
        self.near
            .as_ref()
            .map_or((0, 0), |n| (n.held.iter().flatten().count(), n.queue.len()))
    }

    /// Queues tile `index` for its upload (a tile queued for the same slot before is dropped).
    pub fn queue_near(&mut self, index: [i32; 2], tile: Arc<NearTile>) {
        let Some(near) = &mut self.near else {
            return;
        };
        let level_bytes = if tile.bc7 {
            crate::bc7::level_bytes(NEAR_TILE_TEXELS, NEAR_TILE_TEXELS)
        } else {
            (NEAR_TILE_TEXELS * NEAR_TILE_TEXELS * 4) as usize
        };
        // A BC7 tile cannot go into RGBA8 textures (an RGBA8 tile into BC7 ones is encoded as it is uploaded).
        if tile.levels.is_empty()
            || tile.levels[0].len() != level_bytes
            || tile.cover.len() != (NEAR_TILE_TEXELS * NEAR_TILE_TEXELS) as usize
            || (tile.bc7 && self.arrays.as_ref().is_some_and(|a| !a.bc7))
        {
            return;
        }
        let grid = near.grid;
        let slot = grid.slot(index);
        near.queue
            .retain(|(k, _)| *k == index || grid.slot(*k) != slot);
        if !near.queue.iter().any(|(k, _)| *k == index) {
            near.queue.push_back((index, tile));
        }
    }

    /// Uploads queued tiles, up to [`NEAR_UPLOAD_BYTES`] (at least one); how many.
    /// `changed` takes the chart rectangle each tile uploaded, and each tile it put out of its slot, can change
    /// (`NearGrid::reach`).
    pub fn upload_near(&mut self, queue: &wgpu::Queue, changed: &mut Vec<[f64; 4]>) -> usize {
        let (Some(near), Some(arrays)) = (&mut self.near, &self.arrays) else {
            return 0;
        };
        let mut bytes = 0;
        let mut uploaded = 0;
        while bytes < NEAR_UPLOAD_BYTES
            && let Some((index, tile)) = near.queue.pop_front()
        {
            let [si, sj] = near.grid.slot(index);
            if tile.bc7 && !arrays.bc7 {
                continue;
            }
            for (k, level) in tile.levels.iter().enumerate().take(arrays.levels as usize) {
                let side = NEAR_TILE_TEXELS >> k;
                let encoded;
                let (texels, bytes) = match (arrays.bc7, tile.bc7) {
                    (false, _) => (Texels::Bytes(4), level.as_slice()),
                    (true, true) => (Texels::Bc7, level.as_slice()),
                    (true, false) => {
                        encoded = crate::bc7::encode(level, side, side);
                        (Texels::Bc7, encoded.as_slice())
                    }
                };
                write(
                    queue,
                    &arrays.albedo,
                    k as u32,
                    1,
                    si * side,
                    sj * side,
                    side,
                    texels,
                    bytes,
                );
            }
            write(
                queue,
                &arrays.cover,
                0,
                1,
                si * NEAR_TILE_TEXELS,
                sj * NEAR_TILE_TEXELS,
                NEAR_TILE_TEXELS,
                Texels::Bytes(1),
                &tile.cover,
            );
            let slot = (sj * near.grid.slots + si) as usize;
            let held = &mut near.held[slot];
            if let Some(gone) = held.filter(|gone| *gone != index) {
                changed.push(near.grid.reach(gone));
            }
            changed.push(near.grid.reach(index));
            *held = Some(index);
            near.marks[slot] = tile.marks;
            bytes += tile.bytes();
            uploaded += 1;
        }
        uploaded
    }

    /// Queues a rewrite of a rectangle of the cover for the next upload ([`Self::upload_patches`]); one that does not fit
    /// its layer is dropped.
    pub fn queue_patch(&mut self, patch: CoverPatch) {
        let side = match patch.layer {
            CoverLayer::Window => self.arrays.as_ref().filter(|_| self.far).map(|a| a.size),
            CoverLayer::Near(_) => self.near.as_ref().map(|_| NEAR_TILE_TEXELS),
        };
        if side.is_some_and(|side| patch.fits(side)) {
            self.patches.push_back(patch);
        }
    }

    /// The host's number for the marks over near tile `index`'s cover ([`NearTile::marks`]): the latest queued for it
    /// (its tile or a rewrite), else the drawn tile's; none when the layer neither draws nor waits for it.
    pub fn near_marks(&self, index: [i32; 2]) -> Option<u64> {
        let near = self.near.as_ref()?;
        let patched = self
            .patches
            .iter()
            .filter(|p| p.layer == CoverLayer::Near(index))
            .map(|p| p.marks)
            .max();
        if let Some((_, tile)) = near.queue.iter().rev().find(|(k, _)| *k == index) {
            return Some(patched.map_or(tile.marks, |m| m.max(tile.marks)));
        }
        let [i, j] = near.grid.slot(index);
        let slot = (j * near.grid.slots + i) as usize;
        (near.held[slot] == Some(index))
            .then(|| patched.map_or(near.marks[slot], |m| m.max(near.marks[slot])))
    }

    /// Writes the queued rewrites of the cover (after the near tiles of the frame: a rewrite of a tile still waiting for
    /// its upload waits with it; one for a tile its slot no longer holds is dropped). The layer and texel rectangle
    /// (x, y, columns, rows) of each written.
    pub fn upload_patches(&mut self, queue: &wgpu::Queue) -> Vec<(CoverLayer, [u32; 4])> {
        let mut written = Vec::new();
        let Some(arrays) = &self.arrays else {
            self.patches.clear();
            return written;
        };
        let mut waiting = VecDeque::new();
        while let Some(patch) = self.patches.pop_front() {
            let [w, h] = patch.size;
            let (layer, x, y) = match patch.layer {
                CoverLayer::Window if self.far && patch.fits(arrays.size) => {
                    (0, patch.at[0], patch.at[1])
                }
                CoverLayer::Window => continue,
                CoverLayer::Near(index) => {
                    let Some(near) = &mut self.near else {
                        continue;
                    };
                    let [si, sj] = near.grid.slot(index);
                    let slot = (sj * near.grid.slots + si) as usize;
                    if near.held[slot] != Some(index) {
                        if near.queue.iter().any(|(k, _)| *k == index) {
                            waiting.push_back(patch);
                        }
                        continue;
                    }
                    near.marks[slot] = patch.marks;
                    (
                        1,
                        si * NEAR_TILE_TEXELS + patch.at[0],
                        sj * NEAR_TILE_TEXELS + patch.at[1],
                    )
                }
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &arrays.cover,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                &patch.classes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
            written.push((patch.layer, [patch.at[0], patch.at[1], w, h]));
        }
        self.patches = waiting;
        written
    }

    /// The near tiles' uniforms: the grid, and the tile each slot holds.
    pub fn near_uniforms(&self, grid: &mut [f32; 4], slots: &mut [[i32; 4]; 32]) {
        for entry in slots.iter_mut() {
            *entry = [NO_TILE; 4];
        }
        let Some(near) = &self.near else {
            *grid = [0.0; 4];
            return;
        };
        *grid = [
            near.grid.origin[0] as f32,
            near.grid.origin[1] as f32,
            near.grid.tile_m as f32,
            near.grid.slots as f32,
        ];
        for (k, held) in near.held.iter().enumerate() {
            if let Some([i, j]) = held {
                let half = (k % 2) * 2;
                slots[k / 2][half] = *i;
                slots[k / 2][half + 1] = *j;
            }
        }
    }
}
