//! The aerial-perspective volume in the sky's own group: a buffer copy of the renderer's (`naval_render::aerial`, which
//! every material samples at group 0) for the sky's passes, bound at group 1 beside the tables (binding 12, read through
//! `naval_sky::aerial`). The dome takes the haze along each pixel's ray from the dome slice; the clouds' march takes the
//! air in front of the clouds. A buffer rather than textures: the passes that bind the sky's group (the march, the
//! environment bake) already read as many textures as a stage may. The pass that fills the renderer's volume
//! (naval-clouds' aerial perspective) copies it here after it ([`AerialCopy::copy_from`]).
//!
//! After the volume the same buffer holds the towns' light in the air at night ([`GLOW_OFFSET`]): a header (the field's
//! frame and the lit land's radiance), then the light the lit land below sends through the air at each point of a grid
//! about the window (`GLOW_SIDE`² columns of `GLOW_LEVELS` heights, read through `naval_sky::aerial`'s `glow_field`),
//! which naval-clouds' `glow` pass fills when the land's lit map or the air changes. The aerial perspective scatters it
//! toward the eye (the sky's glow over a lit town) and the clouds' march lights their bases with it from below
//! (native/docs/terrain/night.md, "The towns' light in the sky").

use naval_render::{AERIAL_SKY_BYTES, AERIAL_VOLUME_BYTES, AerialVolume};

/// Where the towns' light starts in the buffer: after the volume and its dome slice (a multiple of 256 bytes, so the pass
/// that fills the field binds it at an offset).
pub const GLOW_OFFSET: u64 = AERIAL_VOLUME_BYTES + AERIAL_SKY_BYTES;
/// The towns' light's header: its frame (world x, z of the grid's corner, a column's side in m, columns a side) and
/// the lit land's radiance (rgb, w 1 while it burns); 256 bytes, so the field after it starts aligned.
pub const GLOW_HEADER_BYTES: u64 = 256;
/// Columns a side of the field's grid.
pub const GLOW_SIDE: u32 = 80;
/// Heights of the field's grid: `GLOW_FLOOR_M` × 2^k for k = 0..GLOW_LEVELS (31 m to 8 km).
pub const GLOW_LEVELS: u32 = 9;
pub const GLOW_FLOOR_M: f32 = 31.25;
/// The field: a `vec4` (the light's fluence, then its net flux, world x, y, z: per unit radiance of wholly lit land)
/// a grid point, columns in rows of `GLOW_SIDE`, levels of rows.
pub const GLOW_FIELD_BYTES: u64 = (GLOW_SIDE * GLOW_SIDE * GLOW_LEVELS * 16) as u64;

/// The towns' light's header ([`GLOW_OFFSET`]), as `naval_sky::aerial` reads it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlowHeader {
    /// World x, z of the grid's corner (m), a column's side (m), columns a side.
    pub frame: [f32; 4],
    /// The radiance of wholly lit land (linear rgb, the land's glow's own), w 1 while it burns (0: nothing read).
    pub radiance: [f32; 4],
}

/// The sky's copy of the volume and its dome slice: clear air until the first copy.
pub struct AerialCopy {
    pub buffer: wgpu::Buffer,
}

impl AerialCopy {
    pub fn new(device: &wgpu::Device) -> Self {
        // wgpu zeroes a new buffer: clear air.
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky aerial"),
            size: GLOW_OFFSET + GLOW_HEADER_BYTES + GLOW_FIELD_BYTES,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer }
    }

    /// Writes the towns' light's header (its frame and radiance; a zero radiance's `w` puts it out).
    pub fn write_glow_header(&self, queue: &mut naval_render::UploadSink<'_>, header: &GlowHeader) {
        queue.write_buffer(&self.buffer, GLOW_OFFSET, bytemuck::bytes_of(header));
    }

    /// Copies the renderer's volume and dome slice here (after the pass that filled them).
    pub fn copy_from(&self, encoder: &mut wgpu::CommandEncoder, volume: &AerialVolume) {
        volume.copy_to_buffer(encoder, &self.buffer, 0);
    }

    /// Group 1's entry for the copy: read-only storage, for the fragment and compute stages.
    pub fn layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
        wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }
    }
}
