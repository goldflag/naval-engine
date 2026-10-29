//! The output dither (`shaders/naval/output.wgsl`, `naval::output`): whichever pass writes an 8- or 10-bit output
//! (the display pass writing it directly, edge smoothing's last stage, the upscale) adds up to one step of triangular
//! blue noise in the encoded domain before the value is stored, so dusk and night skies and fog gradients, shallower
//! than a step a pixel, show as fine grain rather than bands. A float output (the post chain's frames, an HDR surface,
//! the HDR capture) is not dithered: half floats hold the gradients.
//!
//! The mask is naval-render's own 64 × 64 void-and-cluster blue noise (`examples/blue_noise.rs`, `assets/`), its 256
//! levels 16 texels each; the noise moves with the presentation clock, offset by the golden ratio every sixtieth of a
//! second, so a frame drawn again (a capture, a paused battle's frames, which hold one presentation time) gets the same
//! noise to the byte, and the manual clock's frames are deterministic.

use crate::shaders::{ShaderDefValue, ShaderDefs};

/// The mask: 64 × 64 `R8Unorm` texels, rows top first.
pub const BLUE_NOISE: &[u8] = include_bytes!("../../assets/blue_noise_64x64_r8.bin");
/// The mask's side in texels.
pub const BLUE_NOISE_SIZE: u32 = 64;
/// Group 0 bindings of the mask and of the frame's phase in every pass that dithers (`naval::output`).
pub const MASK_BINDING: u32 = 8;
pub const PHASE_BINDING: u32 = 9;

/// The steps an output format's channels are stored in, when the output dither applies to it: 255 for 8-bit unorm
/// formats, 1023 for 10-bit; `None` for float formats and anything else.
pub fn dither_levels(format: wgpu::TextureFormat) -> Option<u32> {
    use wgpu::TextureFormat as F;
    match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb | F::Bgra8Unorm | F::Bgra8UnormSrgb => Some(255),
        F::Rgb10a2Unorm => Some(1023),
        _ => None,
    }
}

/// The defines a pass writing to `format` dithers with (`DITHER_LEVELS`), if it dithers.
pub(crate) fn dither_defs(format: wgpu::TextureFormat) -> ShaderDefs {
    dither_levels(format)
        .map(|levels| vec![("DITHER_LEVELS".to_owned(), ShaderDefValue::UInt(levels))])
        .unwrap_or_default()
}

/// The golden-ratio phase of the noise at presentation time `seconds`, in 0..1: one golden-ratio step a sixtieth of a
/// second.
pub fn dither_phase(seconds: f64) -> f32 {
    const GOLDEN_RATIO: f64 = 0.618_033_988_749_894_9;
    let steps = (seconds.max(0.0) * 60.0).round();
    (steps * GOLDEN_RATIO).fract() as f32
}

/// The mask on the GPU and the frame's phase, bound by every pass that dithers.
pub(crate) struct Dither {
    view: wgpu::TextureView,
    phase: wgpu::Buffer,
}

impl Dither {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let size = wgpu::Extent3d {
            width: BLUE_NOISE_SIZE,
            height: BLUE_NOISE_SIZE,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("dither mask"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            BLUE_NOISE,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(BLUE_NOISE_SIZE),
                rows_per_image: Some(BLUE_NOISE_SIZE),
            },
            size,
        );
        let phase = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("dither phase"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            view: texture.create_view(&Default::default()),
            phase,
        }
    }

    /// The noise for a frame at presentation time `seconds`.
    pub(crate) fn write(&self, uploads: &mut crate::upload::Uploads, seconds: f64) {
        uploads.write_buffer(
            &self.phase,
            0,
            bytemuck::cast_slice(&[dither_phase(seconds), 0.0, 0.0, 0.0]),
        );
    }

    /// The layout entries a pass that dithers adds to its group 0.
    pub(crate) fn layout_entries() -> [wgpu::BindGroupLayoutEntry; 2] {
        [
            wgpu::BindGroupLayoutEntry {
                binding: MASK_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: PHASE_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(16),
                },
                count: None,
            },
        ]
    }

    /// The bind group entries for [`Dither::layout_entries`].
    pub(crate) fn entries(&self) -> [wgpu::BindGroupEntry<'_>; 2] {
        [
            wgpu::BindGroupEntry {
                binding: MASK_BINDING,
                resource: wgpu::BindingResource::TextureView(&self.view),
            },
            wgpu::BindGroupEntry {
                binding: PHASE_BINDING,
                resource: self.phase.as_entire_binding(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mask is a dither mask: every level held by the same number of texels, and little low-frequency energy
    /// (each 8 × 8 block's mean stays near the whole mask's).
    #[test]
    fn the_mask_is_even_and_fine_grained() {
        assert_eq!(
            BLUE_NOISE.len(),
            (BLUE_NOISE_SIZE * BLUE_NOISE_SIZE) as usize
        );
        let mut counts = [0usize; 256];
        for &b in BLUE_NOISE {
            counts[usize::from(b)] += 1;
        }
        assert!(counts.iter().all(|&c| c == 16), "levels {:?}", &counts[..8]);
        let size = BLUE_NOISE_SIZE as usize;
        let (mut worst, mut total): (f64, f64) = (0.0, 0.0);
        for by in 0..size / 8 {
            for bx in 0..size / 8 {
                let mut sum = 0.0;
                for y in 0..8 {
                    for x in 0..8 {
                        sum += f64::from(BLUE_NOISE[(by * 8 + y) * size + bx * 8 + x]);
                    }
                }
                let stray = (sum / 64.0 - 127.5).abs();
                worst = worst.max(stray);
                total += stray;
            }
        }
        let mean = total / ((size / 8) * (size / 8)) as f64;
        // The same levels shuffled into white noise stray 6.8 levels on average and 21.5 at worst; the mask 1.8 and 5.6.
        assert!(
            mean < 3.0 && worst < 10.0,
            "block means stray {mean} on average, {worst} at worst"
        );
    }

    #[test]
    fn eight_and_ten_bit_outputs_dither_and_floats_do_not() {
        use wgpu::TextureFormat as F;
        assert_eq!(dither_levels(F::Bgra8UnormSrgb), Some(255));
        assert_eq!(dither_levels(F::Rgba8Unorm), Some(255));
        assert_eq!(dither_levels(F::Rgb10a2Unorm), Some(1023));
        assert_eq!(dither_levels(F::Rgba16Float), None);
        assert!(dither_defs(F::Rgba16Float).is_empty());
        assert_eq!(dither_defs(F::Rgba8UnormSrgb).len(), 1);
    }

    #[test]
    fn the_phase_follows_the_presentation_clock() {
        assert_eq!(dither_phase(0.0), 0.0);
        // A held presentation time (a pause, a capture) keeps its noise; the next sixtieth moves it.
        assert_eq!(dither_phase(12.5), dither_phase(12.5));
        assert_ne!(dither_phase(1.0), dither_phase(1.0 + 1.0 / 60.0));
        assert!((0..600).all(|frame| (0.0..1.0).contains(&dither_phase(f64::from(frame) / 60.0))));
    }
}
