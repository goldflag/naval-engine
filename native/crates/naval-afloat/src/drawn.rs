//! The drawn sea under each hull, read back from the GPU: what the surface draws about her beyond the sines.
//!
//! The sines a hull rides ([`crate::sea`]) are the drawn sea's long waves exactly; about her the surface adds the wave
//! field's shorter waves (and the wakes'). Those move a hull: measured (`tests/drawn_sea.rs`), a destroyer's roll by
//! about a degree and her heave by a few tenths of a metre in a moderate to rough sea, and a hull no longer than the
//! field's waves far more. The ocean already evaluates the drawn height on a patch about each coupled hull once a frame
//! for the hulls' wet band (`naval_ocean::hull_heights`, on with `Ocean::enable_hull_heights_in`): the surface's own
//! `height_at`, its choppy displacement inverted, with the wakes'. [`DrawnSea`] copies each followed hull's patch to
//! the CPU without waiting for it (it arrives a frame or two late), keeps what it holds beyond the sines it was drawn
//! on (the residual), and a hull rides the sines with it ([`DrawnSurface`]). Those sines are the ones the host coupled
//! the ocean by ([`DrawnSea::couple`]), kept until the heights drawn on them are copied: a new wind's sines taken from
//! the last frame's heights would leave the difference of two long seas in the residual, and jolt her. Until a read
//! arrives, once it is older than [`HELD_S`], and for a hull with no patch (out of the camera's view, beyond the
//! ocean's 16 slots), she rides the sines alone.
//!
//! Her own wake is in the patch too: she rides her bow wave's crest and the trough aft of it as she rides any other
//! water, a little sinkage and trim at speed.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use naval_effects::sea::SeaWaves;
use naval_ocean::hull_heights::{HullHeights, HullPatch, PATCH_ACROSS, PATCH_ALONG};
use naval_ocean::hull_sea::HullFootprint;

use crate::sea::SeaSurface;

/// A read is ridden this long after the frame it was drawn in (s), then dropped.
pub const HELD_S: f64 = 0.5;
/// A patch is the hull's whose centre lies within this of her footprint's (m: the uniforms' f32).
const SAME_PATCH_M: f64 = 0.05;
/// A copy's state: on its way, mapped, failed.
const WAITING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// One hull's patch as drawn, less the sines: metres per texel, row by row.
#[derive(Clone, Debug)]
pub struct Residual {
    pub patch: HullPatch,
    /// Texels along and across the patch as drawn (a distant hull's are fewer).
    pub texels: (u32, u32),
    pub values: Vec<f32>,
    /// The time the patch was drawn at (s).
    pub time: f64,
}

impl Residual {
    /// The residual at world (x, z), bilinearly; none off the patch.
    pub fn at(&self, x: f64, z: f64) -> Option<f64> {
        let p = &self.patch;
        let (dx, dz) = (x - p.x, z - p.z);
        let along = dx * p.axis_x + dz * p.axis_z;
        let across = -dx * p.axis_z + dz * p.axis_x;
        let (n, m) = self.texels;
        let fi = (along / p.half_length + 1.0) / 2.0 * f64::from(n - 1);
        let fj = (across / p.half_breadth + 1.0) / 2.0 * f64::from(m - 1);
        if !(0.0..=f64::from(n - 1)).contains(&fi) || !(0.0..=f64::from(m - 1)).contains(&fj) {
            return None;
        }
        let (i, j) = ((fi as u32).min(n - 2), (fj as u32).min(m - 2));
        let (s, t) = (fi - f64::from(i), fj - f64::from(j));
        let value = |i: u32, j: u32| f64::from(self.values[(j * n + i) as usize]);
        let near = value(i, j) * (1.0 - s) + value(i + 1, j) * s;
        let far = value(i, j + 1) * (1.0 - s) + value(i + 1, j + 1) * s;
        Some(near * (1.0 - t) + far * t)
    }
}

/// A copy on its way back.
struct Pending {
    id: String,
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    patch: HullPatch,
    texels: (u32, u32),
    sea: SeaWaves,
    time: f64,
}

/// What the ocean was last handed to draw the patches by: the sines, the clock and each followed hull's footprint.
struct Coupling {
    sea: SeaWaves,
    time: f64,
    hulls: Vec<(String, HullFootprint)>,
}

/// The patches' read-backs, and what each followed hull last read.
#[derive(Default)]
pub struct DrawnSea {
    held: HashMap<String, Residual>,
    pending: Vec<Pending>,
    /// Read-back buffers free for the next copies.
    free: Vec<wgpu::Buffer>,
    coupled: Option<Coupling>,
}

/// A layer row's bytes as a copy lays it out.
const ROW_BYTES: u32 = (PATCH_ALONG * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

impl DrawnSea {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the host has just handed the ocean (`Ocean::set_hull_sea`): `sea`'s sines (its `ocean_waves`) at `time`
    /// (s), and `hulls`, each followed hull's id and her footprint among those it was handed. The ocean's next heights
    /// are drawn on these; [`DrawnSea::read`] takes them less these sines, whatever the sea is by then.
    pub fn couple(&mut self, hulls: &[(&str, HullFootprint)], sea: &SeaWaves, time: f64) {
        let coupled = self.coupled.get_or_insert_with(|| Coupling {
            sea: *sea,
            time,
            hulls: Vec::new(),
        });
        coupled.sea = *sea;
        coupled.time = time;
        coupled.hulls.clear();
        coupled.hulls.extend(
            hulls
                .iter()
                .map(|(id, footprint)| ((*id).to_owned(), *footprint)),
        );
    }

    /// Take up the copies that have arrived, then copy the patches the last coupling placed ([`DrawnSea::couple`]) as
    /// `heights` holds them. Call it once a frame between the ocean's frames (its heights are the last frame's until
    /// the next one's prepare), before the frame's coupling; a hull whose last copy is still on its way is copied again
    /// once it arrives.
    pub fn read(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, heights: &HullHeights) {
        self.take(device);
        let Some(coupled) = &self.coupled else {
            return;
        };
        let uniforms = heights.uniforms();
        let mut encoder: Option<wgpu::CommandEncoder> = None;
        let first = self.pending.len();
        for (id, footprint) in &coupled.hulls {
            if self.pending.iter().any(|p| p.id == *id) {
                continue;
            }
            // Her patch's layer: the patches in view are packed in order.
            let Some(layer) = (0..uniforms.count()).find(|&i| {
                let at = uniforms.patches[i * 2];
                (f64::from(at[0]) - footprint.x).abs() < SAME_PATCH_M
                    && (f64::from(at[1]) - footprint.z).abs() < SAME_PATCH_M
            }) else {
                continue;
            };
            let [x, z, axis_x, axis_z] = uniforms.patches[layer * 2].map(f64::from);
            let reach = uniforms.patches[layer * 2 + 1];
            let texels = (reach[2] as u32, reach[3] as u32);
            if texels.0 < 2 || texels.1 < 2 {
                continue;
            }
            let buffer = self.free.pop().unwrap_or_else(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("afloat drawn sea read-back"),
                    size: u64::from(ROW_BYTES) * u64::from(PATCH_ACROSS),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            });
            let encoder = encoder.get_or_insert_with(|| {
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("afloat drawn sea read-back"),
                })
            });
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: heights.texture(),
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(ROW_BYTES),
                        rows_per_image: Some(PATCH_ACROSS),
                    },
                },
                wgpu::Extent3d {
                    width: texels.0,
                    height: texels.1,
                    depth_or_array_layers: 1,
                },
            );
            self.pending.push(Pending {
                id: id.clone(),
                buffer,
                state: Arc::new(AtomicU8::new(WAITING)),
                patch: HullPatch {
                    x,
                    z,
                    axis_x,
                    axis_z,
                    half_length: f64::from(reach[0]),
                    half_breadth: f64::from(reach[1]),
                },
                texels,
                sea: coupled.sea,
                time: coupled.time,
            });
        }
        let Some(encoder) = encoder else {
            return;
        };
        queue.submit([encoder.finish()]);
        for pending in &self.pending[first..] {
            let state = pending.state.clone();
            pending
                .buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    state.store(
                        if result.is_ok() { MAPPED } else { FAILED },
                        Ordering::Release,
                    );
                });
        }
    }

    /// Take up the copies that have arrived (polling the device without waiting).
    pub fn take(&mut self, device: &wgpu::Device) {
        if self.pending.is_empty() {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        let mut k = 0;
        while k < self.pending.len() {
            match self.pending[k].state.load(Ordering::Acquire) {
                WAITING => k += 1,
                state => {
                    let pending = self.pending.swap_remove(k);
                    if state == MAPPED {
                        let residual = pending.residual();
                        pending.buffer.unmap();
                        if let Some(residual) = residual {
                            self.held.insert(pending.id, residual);
                        }
                    }
                    self.free.push(pending.buffer);
                }
            }
        }
    }

    /// Whether a copy is on its way.
    pub fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    /// What hull `id` last read, if anything.
    pub fn residual(&self, id: &str) -> Option<&Residual> {
        self.held.get(id)
    }

    /// The sea hull `id` rides: `sea`'s sines and, while it holds, her last read of what is drawn beyond them.
    pub fn surface<'a>(&'a self, id: &str, sea: &'a SeaWaves) -> DrawnSurface<'a> {
        DrawnSurface {
            sea,
            residual: self.held.get(id),
        }
    }
}

impl Pending {
    /// The mapped patch less the sines at each texel at the time it was drawn.
    fn residual(&self) -> Option<Residual> {
        let mapped = self.buffer.slice(..).get_mapped_range().ok()?;
        let (n, m) = self.texels;
        let mut values = Vec::with_capacity((n * m) as usize);
        for (j, row) in mapped
            .chunks(ROW_BYTES as usize)
            .take(m as usize)
            .enumerate()
        {
            let row: &[f32] = bytemuck::cast_slice(&row[..(n * 4) as usize]);
            for (i, &drawn) in row.iter().enumerate() {
                let (x, z) = self.patch.texel_of(i as u32, j as u32, self.texels);
                values.push(drawn - self.sea.height(x, z, self.time) as f32);
            }
        }
        Some(Residual {
            patch: self.patch,
            texels: self.texels,
            values,
            time: self.time,
        })
    }
}

/// The sines with a hull's last read of the drawn sea beyond them.
#[derive(Clone, Copy, Debug)]
pub struct DrawnSurface<'a> {
    pub sea: &'a SeaWaves,
    pub residual: Option<&'a Residual>,
}

impl SeaSurface for DrawnSurface<'_> {
    fn height(&self, x: f64, z: f64, time: f64) -> f64 {
        let short = self
            .residual
            .filter(|r| (time - r.time).abs() <= HELD_S)
            .and_then(|r| r.at(x, z))
            .unwrap_or(0.0);
        self.sea.height(x, z, time) + short
    }

    /// The field's waves reach down to the patch's texels: the finest sampling, read back or not (so her stations do
    /// not change as reads come and go).
    fn shortest_wave(&self) -> Option<f64> {
        None
    }
}
