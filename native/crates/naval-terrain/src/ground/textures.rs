//! Texture helpers (the web game's `terrain/TerrainTextures.ts`): uploads with a CPU mip chain (three generates them on the
//! GPU; a 2 × 2 box per level is what its blit does on even sizes), and the detail photographs packed into one
//! texture of four luminance channels.

use half::f16;

/// Levels of a full mip chain for a `width × height` texture.
pub fn mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).leading_zeros()
}

fn next_size(size: u32) -> u32 {
    (size / 2).max(1)
}

/// The next level of a `channels`-channel f32 image by a 2 × 2 box (clamped at an odd edge).
fn halve_f32(data: &[f32], width: u32, height: u32, channels: usize) -> (Vec<f32>, u32, u32) {
    let (w, h) = (next_size(width), next_size(height));
    let mut out = vec![0f32; w as usize * h as usize * channels];
    let at = |x: u32, y: u32, c: usize| {
        data[(y.min(height - 1) as usize * width as usize + x.min(width - 1) as usize) * channels
            + c]
    };
    for y in 0..h {
        for x in 0..w {
            for c in 0..channels {
                out[(y as usize * w as usize + x as usize) * channels + c] = (at(2 * x, 2 * y, c)
                    + at(2 * x + 1, 2 * y, c)
                    + at(2 * x, 2 * y + 1, c)
                    + at(2 * x + 1, 2 * y + 1, c))
                    * 0.25;
            }
        }
    }
    (out, w, h)
}

/// Mip levels of RGBA8 bytes (level 0 included), box-filtered in the stored values.
pub fn mips_rgba8(data: &[u8], width: u32, height: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![data.to_vec()];
    let mut current: Vec<f32> = data.iter().map(|&b| f32::from(b)).collect();
    let (mut w, mut h) = (width, height);
    for _ in 1..mip_count(width, height) {
        let (next, nw, nh) = halve_f32(&current, w, h, 4);
        levels.push(
            next.iter()
                .map(|&v| v.round().clamp(0.0, 255.0) as u8)
                .collect(),
        );
        current = next;
        w = nw;
        h = nh;
    }
    levels
}

fn srgb_to_linear(c: f32) -> f32 {
    if c < 0.04045 {
        c * 0.077_399_38
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c < 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Mip levels of sRGB RGBA8 bytes, averaged in linear light (as the GPU filters an sRGB view); alpha is linear.
pub fn mips_srgb8(data: &[u8], width: u32, height: u32) -> Vec<Vec<u8>> {
    let mut levels = vec![data.to_vec()];
    let mut current: Vec<f32> = data
        .chunks(4)
        .flat_map(|p| {
            [
                srgb_to_linear(f32::from(p[0]) / 255.0),
                srgb_to_linear(f32::from(p[1]) / 255.0),
                srgb_to_linear(f32::from(p[2]) / 255.0),
                f32::from(p[3]) / 255.0,
            ]
        })
        .collect();
    let (mut w, mut h) = (width, height);
    for _ in 1..mip_count(width, height) {
        let (next, nw, nh) = halve_f32(&current, w, h, 4);
        levels.push(
            next.chunks(4)
                .flat_map(|p| {
                    let e = |v: f32| (linear_to_srgb(v) * 255.0).round().clamp(0.0, 255.0) as u8;
                    [
                        e(p[0]),
                        e(p[1]),
                        e(p[2]),
                        (p[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                    ]
                })
                .collect(),
        );
        current = next;
        w = nw;
        h = nh;
    }
    levels
}

/// Mip levels of sRGB RGBA8 bytes with the colour premultiplied by alpha in linear light (level 0 included), for an
/// `Rgba8UnormSrgb` texture whose filtering then never bleeds the colour of an empty texel (alpha 0) into its
/// neighbours: a shader divides the filtered colour by the filtered alpha. The surface maps' satellite colour
/// (`TerrainLayer::set_surface_maps`); a 256-entry table decodes, so a 4096² map takes a few hundred milliseconds.
pub fn mips_srgb8_premultiplied(data: &[u8], width: u32, height: u32) -> Vec<Vec<u8>> {
    let decode: Vec<f32> = (0..256).map(|i| srgb_to_linear(i as f32 / 255.0)).collect();
    let encode = |v: f32| (linear_to_srgb(v) * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut level0 = data.to_vec();
    for p in level0.chunks_mut(4) {
        if p[3] < 255 {
            let a = f32::from(p[3]) / 255.0;
            for c in &mut p[..3] {
                *c = encode(decode[*c as usize] * a);
            }
        }
    }
    let mut levels = Vec::with_capacity(mip_count(width, height) as usize);
    if mip_count(width, height) > 1 {
        // Level 1 straight from level 0's bytes (no f32 copy of level 0), then 2 × 2 boxes in f32.
        let (w, h) = (next_size(width), next_size(height));
        let mut current = vec![0f32; w as usize * h as usize * 4];
        let at = |x: u32, y: u32| {
            (y.min(height - 1) as usize * width as usize + x.min(width - 1) as usize) * 4
        };
        for y in 0..h {
            for x in 0..w {
                let out = (y as usize * w as usize + x as usize) * 4;
                for k in [
                    at(2 * x, 2 * y),
                    at(2 * x + 1, 2 * y),
                    at(2 * x, 2 * y + 1),
                    at(2 * x + 1, 2 * y + 1),
                ] {
                    for c in 0..3 {
                        current[out + c] += decode[level0[k + c] as usize] * 0.25;
                    }
                    current[out + 3] += f32::from(level0[k + 3]) / 255.0 * 0.25;
                }
            }
        }
        levels.push(level0);
        let (mut w, mut h) = (w, h);
        loop {
            levels.push(
                current
                    .chunks(4)
                    .flat_map(|p| {
                        [
                            encode(p[0]),
                            encode(p[1]),
                            encode(p[2]),
                            (p[3] * 255.0).round().clamp(0.0, 255.0) as u8,
                        ]
                    })
                    .collect(),
            );
            if w == 1 && h == 1 {
                break;
            }
            let (next, nw, nh) = halve_f32(&current, w, h, 4);
            current = next;
            w = nw;
            h = nh;
        }
    } else {
        levels.push(level0);
    }
    levels
}

/// Mip levels of single floats stored as half floats (R16Float, level 0 included), box-filtered in f32.
pub fn mips_r16f(data: &[f32], width: u32, height: u32) -> Vec<Vec<u16>> {
    let mut levels = vec![data.iter().map(|&v| f16::from_f32(v).to_bits()).collect()];
    let mut current = data.to_vec();
    let (mut w, mut h) = (width, height);
    for _ in 1..mip_count(width, height) {
        let (next, nw, nh) = halve_f32(&current, w, h, 1);
        levels.push(next.iter().map(|&v| f16::from_f32(v).to_bits()).collect());
        current = next;
        w = nw;
        h = nh;
    }
    levels
}

/// Mip levels of half-float pairs (Rg16Float), box-filtered in f32.
pub fn mips_rg16f(data: &[u16], width: u32, height: u32) -> Vec<Vec<u16>> {
    let mut levels = vec![data.to_vec()];
    let mut current: Vec<f32> = data.iter().map(|&b| f16::from_bits(b).to_f32()).collect();
    let (mut w, mut h) = (width, height);
    for _ in 1..mip_count(width, height) {
        let (next, nw, nh) = halve_f32(&current, w, h, 2);
        levels.push(next.iter().map(|&v| f16::from_f32(v).to_bits()).collect());
        current = next;
        w = nw;
        h = nh;
    }
    levels
}

/// A 2D texture with every level uploaded; `levels[k]` holds level k's texels as bytes.
#[allow(clippy::too_many_arguments)]
pub fn texture_with_levels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    levels: &[&[u8]],
    usage: wgpu::TextureUsages,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: usage | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_levels(queue, &texture, levels);
    texture
}

/// Rewrites a texture's levels in place.
pub fn write_levels(queue: &wgpu::Queue, texture: &wgpu::Texture, levels: &[&[u8]]) {
    let block = texture.format().block_copy_size(None).unwrap_or(4);
    let (mut w, mut h) = (texture.width(), texture.height());
    for (level, bytes) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * block),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        w = next_size(w);
        h = next_size(h);
    }
}

/// Side of the packed detail texture (`DETAIL_SIZE`).
pub const DETAIL_SIZE: u32 = 1024;

/// The detail photographs (`DETAIL_LAYERS`): the repository files the host reads, in channel order.
pub const DETAIL_PHOTOS: [&str; 4] = [
    "public/terrain/rock-color.jpg",
    "public/terrain/ground-color.jpg",
    "public/terrain/sand-color.jpg",
    "public/terrain/snow-color.jpg",
];

/// Packs four photographs (encoded bytes: rock, grass, sand, snow) into the detail texture's RGBA bytes: each
/// channel the luminance of one, normalised to its mean (`packDetail`: 0.5 is the mean).
pub fn pack_detail(photos: [&[u8]; 4]) -> Result<Vec<u8>, String> {
    let size = DETAIL_SIZE;
    let mut out = vec![0u8; (size * size * 4) as usize];
    for (channel, bytes) in photos.iter().enumerate() {
        let image = image::load_from_memory(bytes)
            .map_err(|e| format!("detail photograph {channel}: {e}"))?
            .to_rgb8();
        let image =
            image::imageops::resize(&image, size, size, image::imageops::FilterType::Triangle);
        let luminance: Vec<f64> = image
            .pixels()
            .map(|p| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]))
            .collect();
        let total: f64 = luminance.iter().sum();
        let mean = if total > 0.0 {
            total / luminance.len() as f64
        } else {
            1.0
        };
        for (i, value) in luminance.iter().enumerate() {
            out[i * 4 + channel] = (value / mean * 127.5).round().min(255.0) as u8;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mip_chains_run_to_one_texel() {
        assert_eq!(mip_count(2401, 2401), 12);
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_count(512, 256), 10);
        let levels = mips_rgba8(&[200u8; 5 * 3 * 4], 5, 3);
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[1].len(), 2 * 4);
        assert!(levels[2].iter().all(|&b| b == 200));
    }

    #[test]
    fn premultiplied_mips_keep_the_colour_of_covered_texels() {
        // A 4 × 2 map: the left half imagery (a mid green), the right half none (garbage colour, alpha 0).
        let mut data = Vec::new();
        for _ in 0..2 {
            data.extend_from_slice(&[90, 140, 60, 255, 90, 140, 60, 255]);
            data.extend_from_slice(&[255, 0, 255, 0, 255, 0, 255, 0]);
        }
        let levels = mips_srgb8_premultiplied(&data, 4, 2);
        assert_eq!(levels.len(), 3);
        // Empty texels hold no colour at level 0.
        assert_eq!(&levels[0][8..12], &[0, 0, 0, 0]);
        // Level 1: the left texel unchanged, the right one empty.
        assert_eq!(&levels[1][..4], &[90, 140, 60, 255]);
        assert_eq!(&levels[1][4..8], &[0, 0, 0, 0]);
        // Level 2 (1 × 1): half covered; colour / alpha recovers the green in linear light.
        let p = &levels[2];
        assert!((126..=129).contains(&p[3]), "{p:?}");
        let lin = |b: u8| srgb_to_linear(f32::from(b) / 255.0);
        let a = f32::from(p[3]) / 255.0;
        for (c, want) in [(0, 90u8), (1, 140), (2, 60)] {
            let got = linear_to_srgb(lin(p[c]) / a) * 255.0;
            assert!((got - f32::from(want)).abs() < 3.0, "{c}: {got} vs {want}");
        }
    }
}
