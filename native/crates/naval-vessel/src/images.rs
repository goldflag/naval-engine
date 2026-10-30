//! A model's textures in CPU memory: decoded from PNG or JPEG, halved until they fit the setup's size, and their mip
//! chains filtered on the CPU (colour in linear light; a base colour whose alpha is coverage, a cut-out's or a blended
//! surface's, weighted by it so its edge keeps its colour, every other map a plain box filter).

use anyhow::{Result, bail};

/// One level of a texture: RGBA8 texels, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A texture ready to upload.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureImage {
    pub name: String,
    /// The file's own size, before halving.
    pub source_size: [u32; 2],
    /// Colour (sampled as sRGB) or data.
    pub srgb: bool,
    /// Its alpha is coverage: its colour filtered weighted by it.
    pub coverage: bool,
    /// The mip chain, largest first, down to one texel.
    pub levels: Vec<Level>,
}

impl TextureImage {
    pub fn width(&self) -> u32 {
        self.levels[0].width
    }

    pub fn height(&self) -> u32 {
        self.levels[0].height
    }

    /// Bytes on the GPU, every level.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.pixels.len()).sum()
    }
}

/// What an encoded image is, by its first bytes, for a refusal that names it.
pub fn sniff(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "PNG",
        [0xff, 0xd8, 0xff, ..] => "JPEG",
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => "WebP",
        [0xab, b'K', b'T', b'X', ..] => "KTX2",
        [
            _,
            _,
            _,
            _,
            b'f',
            b't',
            b'y',
            b'p',
            b'a',
            b'v',
            b'i',
            b'f',
            ..,
        ] => "AVIF",
        [b'D', b'D', b'S', b' ', ..] => "DDS",
        _ => "an unknown format",
    }
}

/// The size an encoded image says it is, without decoding it.
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Decodes `bytes` (PNG or JPEG), halves it until its larger side is at most `cap` and builds its mip chain, its colour
/// weighted by its alpha when that is `coverage`.
pub fn decode(
    bytes: &[u8],
    name: &str,
    srgb: bool,
    coverage: bool,
    cap: u32,
) -> Result<TextureImage> {
    let kind = sniff(bytes);
    if !matches!(kind, "PNG" | "JPEG") {
        bail!("{name} is {kind}: only PNG and JPEG textures are read");
    }
    let decoded = match image::load_from_memory(bytes) {
        Ok(image) => image.to_rgba8(),
        Err(e) => bail!("{name} ({kind}) does not decode: {e}"),
    };
    let (width, height) = decoded.dimensions();
    let mut top = Level {
        width,
        height,
        pixels: decoded.into_raw(),
    };
    while top.width.max(top.height) > cap.max(1) {
        top = halve(&top, srgb, coverage);
    }
    Ok(TextureImage {
        name: name.to_owned(),
        source_size: [width, height],
        srgb,
        coverage,
        levels: chain(top, srgb, coverage),
    })
}

/// A level and every level below it to one texel.
pub fn chain(top: Level, srgb: bool, coverage: bool) -> Vec<Level> {
    let mut levels = vec![top];
    while let Some(last) = levels.last() {
        if last.width == 1 && last.height == 1 {
            break;
        }
        let next = halve(last, srgb, coverage);
        levels.push(next);
    }
    levels
}

fn to_linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(v: f32) -> u8 {
    let s = if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5).clamp(0.0, 255.0) as u8
}

/// The level half the size (each side at least 1): every 2 × 2 texels averaged, the colour weighted by alpha when that
/// is `coverage` (a metal-rough, normal, occlusion or emissive map's alpha is unused, or another channel: no weight).
pub fn halve(level: &Level, srgb: bool, coverage: bool) -> Level {
    let linear: Vec<f32> = (0..=255u8).map(to_linear).collect();
    let (w, h) = ((level.width / 2).max(1), (level.height / 2).max(1));
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0f32; 3];
            let (mut alpha, mut weights) = (0f32, 0f32);
            let mut plain = [0f32; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let sx = (x * 2 + dx).min(level.width - 1);
                let sy = (y * 2 + dy).min(level.height - 1);
                let at = ((sy * level.width + sx) * 4) as usize;
                let texel = &level.pixels[at..at + 4];
                let a = f32::from(texel[3]) / 255.0;
                let weight = if coverage { a } else { 1.0 };
                for c in 0..3 {
                    let v = if srgb {
                        linear[texel[c] as usize]
                    } else {
                        f32::from(texel[c]) / 255.0
                    };
                    sum[c] += v * weight;
                    plain[c] += v;
                }
                alpha += a;
                weights += weight;
            }
            let out = ((y * w + x) * 4) as usize;
            for c in 0..3 {
                let v = if weights > 1e-3 {
                    sum[c] / weights
                } else {
                    plain[c] / 4.0
                };
                pixels[out + c] = if srgb {
                    to_srgb(v)
                } else {
                    (v * 255.0 + 0.5).clamp(0.0, 255.0) as u8
                };
            }
            pixels[out + 3] = (alpha / 4.0 * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    Level {
        width: w,
        height: h,
        pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chain_halves_to_one_texel() {
        let top = Level {
            width: 6,
            height: 3,
            pixels: vec![200; 6 * 3 * 4],
        };
        let levels = chain(top, true, false);
        let sizes: Vec<_> = levels.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, vec![(6, 3), (3, 1), (1, 1)]);
        // A flat colour stays itself through the sRGB round trip.
        assert!(
            levels
                .iter()
                .all(|l| l.pixels.iter().all(|&p| p.abs_diff(200) <= 1))
        );
    }

    #[test]
    fn coverage_keeps_a_cut_outs_colour() {
        // One red opaque texel among three transparent black ones: the average is red, a quarter covered.
        let mut pixels = vec![0u8; 16];
        pixels[..4].copy_from_slice(&[255, 0, 0, 255]);
        let half = halve(
            &Level {
                width: 2,
                height: 2,
                pixels,
            },
            true,
            true,
        );
        assert_eq!(&half.pixels[..3], &[255, 0, 0]);
        assert_eq!(half.pixels[3], 64);
    }

    #[test]
    fn a_data_maps_alpha_weighs_nothing() {
        // A metal-rough map's texels, roughness in green and an unused alpha: one texel's alpha 0 counts as the rest.
        let mut pixels = vec![0u8; 16];
        for (k, texel) in pixels.chunks_mut(4).enumerate() {
            texel.copy_from_slice(&[
                0,
                if k == 0 { 200 } else { 40 },
                0,
                if k == 0 { 0 } else { 255 },
            ]);
        }
        let level = Level {
            width: 2,
            height: 2,
            pixels,
        };
        assert_eq!(halve(&level, false, false).pixels[1], 80);
        // Read as coverage, the transparent texel would drop out.
        assert_eq!(halve(&level, false, true).pixels[1], 40);
    }

    #[test]
    fn webp_and_ktx2_are_named_when_refused() {
        let webp = b"RIFF\0\0\0\0WEBPVP8 ";
        let err = decode(webp, "hull.webp", true, false, 2048)
            .unwrap_err()
            .to_string();
        assert!(err.contains("WebP"), "{err}");
        let ktx = [0xab, b'K', b'T', b'X', b' ', b'2', b'0'];
        assert!(
            decode(&ktx, "hull.ktx2", true, false, 2048)
                .unwrap_err()
                .to_string()
                .contains("KTX2")
        );
    }
}
