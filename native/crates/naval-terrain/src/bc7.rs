//! BC7 on the CPU for the land's largest textures (native/docs/terrain/trees.md, "Memory"): the trees' impostor atlas
//! once it is baked, the surface photographs as they are made. A quarter of RGBA8's bytes; a device without
//! `TEXTURE_COMPRESSION_BC` (or a wasm32 build, which has no encoder) keeps RGBA8.

/// Bytes of a 4 × 4 block.
pub const BLOCK_BYTES: usize = 16;

/// Whether `device` samples BC7 and this build can encode it.
pub fn supported(device: &wgpu::Device) -> bool {
    cfg!(not(target_arch = "wasm32"))
        && device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC)
}

/// Bytes of a `width` × `height` level's blocks.
pub fn level_bytes(width: u32, height: u32) -> usize {
    width.div_ceil(4) as usize * height.div_ceil(4) as usize * BLOCK_BYTES
}

/// `rgba` (`width` × `height` texels, rows tightly packed; both multiples of 4) as BC7 blocks, block rows top to
/// bottom, at the encoder's fastest setting with alpha, split across up to four threads by rows of blocks (512² in
/// 11 ms, 4096² in 0.58 s on an M5 Pro with other agents building).
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    assert!(
        width.is_multiple_of(4)
            && height.is_multiple_of(4)
            && rgba.len() == width as usize * height as usize * 4,
        "BC7: {width} x {height} texels in {} bytes",
        rgba.len()
    );
    let settings = intel_tex_2::bc7::alpha_ultra_fast_settings();
    let block_rows = (height / 4) as usize;
    let row_bytes = (width / 4) as usize * BLOCK_BYTES;
    let mut out = vec![0u8; block_rows * row_bytes];
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 4)
        .min(block_rows.div_ceil(16));
    let per = block_rows.div_ceil(threads.max(1));
    let strip = |first: usize, blocks: &mut [u8]| {
        let rows = blocks.len() / row_bytes;
        let at = first * 4 * width as usize * 4;
        let surface = intel_tex_2::RgbaSurface {
            data: &rgba[at..at + rows * 4 * width as usize * 4],
            width,
            height: rows as u32 * 4,
            stride: width * 4,
        };
        intel_tex_2::bc7::compress_blocks_into(&settings, &surface, blocks);
    };
    if threads <= 1 {
        strip(0, &mut out);
    } else {
        std::thread::scope(|scope| {
            for (k, blocks) in out.chunks_mut(per * row_bytes).enumerate() {
                let strip = &strip;
                scope.spawn(move || strip(k * per, blocks));
            }
        });
    }
    out
}

/// Without an encoder (wasm32): never called, since [`supported`] is false there.
#[cfg(target_arch = "wasm32")]
pub fn encode(_rgba: &[u8], _width: u32, _height: u32) -> Vec<u8> {
    unreachable!("BC7 is never chosen on wasm32 (bc7::supported)")
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn strips_encode_as_the_whole_does() {
        let (width, height) = (64u32, 256u32);
        let rgba: Vec<u8> = (0..width * height * 4)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let settings = intel_tex_2::bc7::alpha_ultra_fast_settings();
        let whole = intel_tex_2::bc7::compress_blocks(
            &settings,
            &intel_tex_2::RgbaSurface {
                data: &rgba,
                width,
                height,
                stride: width * 4,
            },
        );
        assert_eq!(encode(&rgba, width, height), whole);
        assert_eq!(whole.len(), level_bytes(width, height));
    }
}
