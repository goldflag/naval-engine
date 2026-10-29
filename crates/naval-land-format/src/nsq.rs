//! NSQ1, the world square's file: `NSQ1`, a u32 header length (little-endian), the header as JSON, then the layers'
//! payloads, each layer's encoding, byte range and SHA-256 in the header's `layers`. A reader skips layers it does not
//! know. Two kinds of file share it:
//!
//! - a **surveyed** square, the bake's input cache (`naval_region_bake::square::Square`): its header adds the
//!   surveyed grid and the DEM it came from, and its `height` layer is on the surveyed layer's latitude bands;
//! - a **final** square, a release's ([`FinalSquare`]): the [`FINAL`] layer alone, on the global lattice, in sixteen
//!   blocks ([`crate::block`]). Its bytes' SHA-256 is its identity in the release's manifest and the store.
//!
//! [`FinalSquare::read`] reads the final layer from either kind, and decodes nothing until a block is asked for.
use std::ops::Range;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::block::{self, Block, BlockInfo, ENCODING, Window};
use crate::lattice::{BLOCKS, QUANTUM_M};
use crate::{Digest, Error, SquareKey, corrupt};

/// The square format's version, in its magic and its header.
pub const FORMAT: u32 = 1;
pub const MAGIC: &[u8; 4] = b"NSQ1";
/// The final layer's name.
pub const FINAL: &str = "final";

/// What a header says of a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerInfo {
    pub name: String,
    /// `i32-planar-zlib` (with `quantum`), `u8-zlib`, or the final layer's [`ENCODING`] (with `quantum` and `blocks`).
    pub encoding: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantum: Option<f64>,
    /// Its payload's byte range after the header.
    pub offset: usize,
    pub length: usize,
    pub sha256: String,
    /// The final layer's block table: sixteen blocks, row-major from the north-west, ranges within the layer's payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Vec<BlockInfo>>,
}

/// What every NSQ1 header says. A surveyed square's says more (its grid and its source), which this does not read.
#[derive(Deserialize)]
struct Common {
    format: u32,
    lat: i32,
    lon: i32,
    layers: Vec<LayerInfo>,
}

/// A final square's header, as [`FinalSquare::encode`] writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinalHeader {
    pub format: u32,
    pub lat: i32,
    pub lon: i32,
    /// Whether any final point is land.
    pub land: bool,
    pub layers: Vec<LayerInfo>,
    /// Anything a bake records beside the layer.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// An NSQ1 file's header JSON and where its payloads start.
pub fn split(bytes: &[u8]) -> Result<(&[u8], usize), Error> {
    let what = "a world square";
    if bytes.len() < 8 || &bytes[..4] != MAGIC {
        return Err(corrupt(what, "no NSQ1 magic"));
    }
    let length = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let header = bytes
        .get(8..8 + length)
        .ok_or_else(|| corrupt(what, "short header"))?;
    Ok((header, 8 + length))
}

/// An NSQ1 file from its header and its payloads.
pub fn frame(header: &impl Serialize, payloads: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).expect("a square's header serialises");
    let mut out = Vec::with_capacity(8 + json.len() + payloads.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(payloads);
    out
}

/// The final layer of a window: its header entry (at offset 0) and its payload.
pub fn final_layer(window: &Window) -> (LayerInfo, Vec<u8>) {
    let (blocks, payload) = block::encode(window);
    (
        LayerInfo {
            name: FINAL.into(),
            encoding: ENCODING.into(),
            quantum: Some(QUANTUM_M),
            offset: 0,
            length: payload.len(),
            sha256: Digest::of(&payload).hex(),
            blocks: Some(blocks),
        },
        payload,
    )
}

/// A square's final layer, read from an NSQ1 file: the header parsed and the block table checked, no block decoded
/// until one is asked for, and each checked against its own digest when it is.
#[derive(Clone, Debug)]
pub struct FinalSquare {
    key: SquareKey,
    bytes: Arc<[u8]>,
    /// The final layer's payload in `bytes`.
    payload: Range<usize>,
    sha256: String,
    blocks: Vec<BlockInfo>,
}

impl FinalSquare {
    /// A release's square: the final layer of `window` alone, with `extra` in its header.
    pub fn encode(
        key: SquareKey,
        window: &Window,
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Vec<u8> {
        assert!(key.valid(), "{key:?} is not a square");
        let (layer, payload) = final_layer(window);
        frame(
            &FinalHeader {
                format: FORMAT,
                lat: key.lat,
                lon: key.lon,
                land: window.land(),
                layers: vec![layer],
                extra,
            },
            &payload,
        )
    }

    /// The final layer of an NSQ1 file (a final square, or a surveyed one that carries it).
    pub fn read(bytes: impl Into<Arc<[u8]>>) -> Result<Self, Error> {
        let bytes: Arc<[u8]> = bytes.into();
        let what = "a final square";
        let (json, start) = split(&bytes)?;
        let header: Common = serde_json::from_slice(json).map_err(|e| corrupt(what, e))?;
        if header.format != FORMAT {
            return Err(corrupt(
                what,
                format!("format {}, not {FORMAT}", header.format),
            ));
        }
        let key = SquareKey {
            lat: header.lat,
            lon: header.lon,
        };
        if !key.valid() {
            return Err(corrupt(what, format!("{key:?} is not a square")));
        }
        let layer = header
            .layers
            .into_iter()
            .find(|l| l.name == FINAL)
            .ok_or_else(|| corrupt(what, "it has no final layer"))?;
        if layer.encoding != ENCODING || layer.quantum != Some(QUANTUM_M) {
            return Err(corrupt(
                what,
                format!(
                    "its final layer is {} at {:?} m",
                    layer.encoding, layer.quantum
                ),
            ));
        }
        let blocks = layer
            .blocks
            .ok_or_else(|| corrupt(what, "its final layer has no block table"))?;
        block::check_table(&blocks, layer.length)?;
        let payload = start
            .checked_add(layer.offset)
            .and_then(|from| Some(from..from.checked_add(layer.length)?))
            .filter(|range| range.end <= bytes.len())
            .ok_or_else(|| corrupt(what, "its final layer runs past the file"))?;
        Ok(Self {
            key,
            bytes,
            payload,
            sha256: layer.sha256,
            blocks,
        })
    }

    pub fn key(&self) -> SquareKey {
        self.key
    }

    /// The file it was read from.
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// The block table, row-major from the north-west.
    pub fn blocks(&self) -> &[BlockInfo] {
        &self.blocks
    }

    /// Block (`br`, `bc`)'s entry: its digest and its least and greatest quanta, without decoding it.
    pub fn block_info(&self, br: usize, bc: usize) -> &BlockInfo {
        &self.blocks[br * BLOCKS + bc]
    }

    /// Block (`br`, `bc`), decoded alone.
    pub fn block(&self, br: usize, bc: usize) -> Result<Block, Error> {
        block::decode_block(&self.bytes[self.payload.clone()], self.block_info(br, bc))
    }

    /// The whole window, every block decoded.
    pub fn window(&self) -> Result<Window, Error> {
        block::decode_window(&self.bytes[self.payload.clone()], &self.blocks)
    }

    /// Whether the layer's payload matches the header's digest and every block's bytes their own, without inflating
    /// any: what a writer checks before it records a square (a reader's blocks are checked as they are read).
    pub fn verify(&self) -> bool {
        let payload = &self.bytes[self.payload.clone()];
        Digest::of(payload).hex() == self.sha256
            && self
                .blocks
                .iter()
                .all(|b| Digest::of(&payload[b.offset..b.offset + b.length]) == b.sha256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_final_square_round_trips_and_reads_lazily() {
        let key = SquareKey { lat: -31, lon: 179 };
        let window = Window::of(key, |p| if (p.i() + p.j()) % 5 == 0 { 9 } else { -9 });
        let bytes = FinalSquare::encode(key, &window, Default::default());
        assert_eq!(&bytes[..4], MAGIC);
        let square = FinalSquare::read(bytes.clone()).unwrap();
        assert_eq!(square.key(), key);
        assert!(square.verify());
        assert_eq!(square.window().unwrap(), window);
        assert_eq!(square.block(3, 0).unwrap(), window.block(3, 0));
        assert_eq!(
            (square.block_info(0, 0).min, square.block_info(0, 0).max),
            (-9, 9)
        );
        // The same window gives the same bytes.
        assert_eq!(FinalSquare::encode(key, &window, Default::default()), bytes);
        // Not a square, not a final layer.
        assert!(FinalSquare::read(b"NSQ2....".to_vec()).is_err());
        let header = FinalHeader {
            format: FORMAT,
            lat: 0,
            lon: 0,
            land: false,
            layers: vec![],
            extra: Default::default(),
        };
        assert!(FinalSquare::read(frame(&header, &[])).is_err());
        // A layer said to run past the file.
        let mut short = bytes.clone();
        short.truncate(short.len() - 1);
        assert!(FinalSquare::read(short).is_err());
    }
}
