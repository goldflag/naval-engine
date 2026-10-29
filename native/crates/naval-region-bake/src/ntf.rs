//! The bake's encodings: heights to quanta of 0.25 m, the planar prediction of a grid of quanta, zlib, base64 and
//! SHA-256. (NTF1 tiles and the NCL1 coarse mask went with the theatres' legacy regions, D-6.)

use std::io::{Read, Write};

use flate2::Compression;
use sha2::{Digest, Sha256};

use crate::BakeError;

/// The height quantum, metres.
pub const HEIGHT_STEP: f64 = 0.25;

/// A height to NTF1 quanta: halves to even, clamped to 16 bits, nothing on the waterline.
#[inline]
pub fn quantise(h: f64) -> i16 {
    let mut q = (h / HEIGHT_STEP).round_ties_even().clamp(-32768.0, 32767.0);
    if q == 0.0 {
        q = if h > 0.0 { 1.0 } else { -1.0 };
    }
    q as i16
}

/// zlib, as every bake's file is.
pub fn zlib(bytes: &[u8]) -> Vec<u8> {
    zlib_at(bytes, Compression::default())
}

/// zlib at `level`.
pub fn zlib_at(bytes: &[u8], level: Compression) -> Vec<u8> {
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::with_capacity(bytes.len() / 4), level);
    encoder.write_all(bytes).expect("zlib into memory");
    encoder.finish().expect("zlib into memory")
}

/// zlib inflated, refusing more than `most` bytes out.
pub fn inflate(bytes: &[u8], most: usize) -> Result<Vec<u8>, BakeError> {
    let mut out = Vec::with_capacity(most.min(64 << 20));
    flate2::read::ZlibDecoder::new(bytes)
        .take(most as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| BakeError::Corrupt(format!("a zlib stream does not inflate: {e}")))?;
    if out.len() > most {
        return Err(BakeError::Corrupt(
            "a zlib stream inflates past its size".into(),
        ));
    }
    Ok(out)
}

/// Bytes as standard base64 with padding (RFC 4648): a manifest carries a region's small files inline this way.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * k)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// NTF1 planar prediction: each sample stores q − (left + up − up_left), zeros outside the grid, little-endian i16.
pub fn encode_residuals(quanta: &[i16], columns: usize, rows: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(columns * rows * 2);
    for j in 0..rows {
        for i in 0..columns {
            let at = j * columns + i;
            let left = if i > 0 { quanta[at - 1] as i32 } else { 0 };
            let up = if j > 0 {
                quanta[at - columns] as i32
            } else {
                0
            };
            let up_left = if i > 0 && j > 0 {
                quanta[at - columns - 1] as i32
            } else {
                0
            };
            let residual = quanta[at] as i32 - (left + up - up_left);
            // Wrapping, as the decoder's i16 sums wrap: exact for every residual that fits, and the bake's never miss.
            out.extend_from_slice(&(residual as i16).to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quanta_never_sit_on_the_waterline() {
        assert_eq!(
            [0.0, 0.01, -0.01, 0.125, 0.375, -0.375, 10.0].map(quantise),
            [-1, 1, -1, 1, 2, -2, 40]
        );
    }
}
