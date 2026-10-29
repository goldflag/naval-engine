//! The DEM a release's squares read, and the source manifest that freezes it (C1; §3.1, "Making a release").
//!
//! - **[`Reads`]**: what a candidate square's surveyed square reads, **by place**. Its bake grid (the square, its margin
//!   and the despike's halo, each sample's 2 × 2 supersampled points) is split at 60° N and 56° S: the part in
//!   [−56°, 60°) reads Terrarium tiles at the square's zoom, the rest Copernicus GLO-30 cells (each part with its
//!   interpolation's support). Every request is split at ±180° into its [`Side`]s: an object is named wrapped onto the
//!   world (`terrarium/11/0/1100.png`, cell `N71W180`) and placed in the square's continuous frame a whole turn east or
//!   west, so no mosaic's bounds ever see a wrapped span.
//! - **[`SourceManifest`]**: every Terrarium tile and every GLO-30 object any candidate square reads, each with the
//!   ETag and length the bucket listed when the release was made, and every GLO-30 cell it reads that the bucket does
//!   not have (`absent`: open sea, never requested). The release file holds its SHA-256. A read sends the ETag
//!   (`If-Match`) and fails when the object changed, is gone, or is not in the manifest; nothing falls back.
//!
//! - **The sea floor** (from `land-v2`, the manifest's second format): the one GEBCO object the second bake reads
//!   ([`super::gebco`]), its URL, ETag and length; every read of it sends that ETag.
//!
//! The file (`sources.json`): canonical JSON, one object a line, tiles and cells in key order; [`SourceManifest::parse`]
//! refuses any other spelling of the same listing, so one listing has one hash. A manifest with no sea floor is the
//! first format, byte for byte as `land-v1`'s was written.
use std::collections::BTreeMap;

use naval_land_format::SquareKey;
use serde::Deserialize;

use crate::geo::{mercator_x, mercator_y};
use crate::square::{SquareGrid, bake_grid, terrarium_zoom};
use crate::terrarium::TERRARIUM_URL;
use crate::{BakeError, Dem, SRTM_NORTH, SRTM_SOUTH, glo30};

pub const FORMAT: u32 = 1;
/// The format of a manifest that lists a sea floor.
pub const FORMAT_SEABED: u32 = 2;

/// A Terrarium tile, wrapped: zoom, x, y.
pub type TileKey = (u32, i64, i64);
/// A GLO-30 cell, wrapped: its south-west corner.
pub type CellKey = (i32, i32);

/// An object as the bucket listed it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Object {
    /// Its ETag, without the quotes.
    pub etag: String,
    pub bytes: u64,
}

/// One side of ±180°: `turn` whole turns from the square's own (−1 west, 0, 1 east), and the objects there, wrapped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side<K> {
    pub turn: i64,
    pub keys: Vec<K>,
}

/// A GLO-30 cell's name and URL from its wrapped key.
pub fn glo30_url(cell: CellKey) -> String {
    glo30::url(cell.0, cell.1)
}

/// A Terrarium tile's URL from its wrapped key.
pub fn terrarium_url(tile: TileKey) -> String {
    format!("{TERRARIUM_URL}/{}/{}/{}.png", tile.0, tile.1, tile.2)
}

/// The Terrarium part of a square's reads.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrariumPart {
    pub zoom: u32,
    /// Tiles x0..=x1 (continuous about the square) by y0..=y1.
    pub span: [i64; 4],
}

/// A GLO-30 part: the window over its latitudes (the part north of 60° N, or south of 56° S).
#[derive(Clone, Debug, PartialEq)]
pub struct Glo30Part {
    pub window: glo30::Window,
    /// The latitudes it serves, half-open as [`dem_at`] has them: [60, ∞) or (−∞, −56).
    pub north: bool,
}

impl Glo30Part {
    pub fn serves(&self, lat: f64) -> bool {
        if self.north {
            lat >= SRTM_NORTH
        } else {
            lat < SRTM_SOUTH
        }
    }

    /// The cells it reads, continuous about the square.
    pub fn cells(&self) -> Vec<CellKey> {
        let win = &self.window;
        let mut out = vec![];
        for s in win.s0..=win.s1 {
            for w in win.w0..=win.w1 {
                if glo30::slice(win, s, w, win.across).is_some() {
                    out.push((s, w));
                }
            }
        }
        out
    }
}

/// A longitude's turn and its wrapped value: −180 to 179.
pub fn wrap_lon(w: i32) -> (i64, i32) {
    (
        i64::from((w + 180).div_euclid(360)),
        (w + 180).rem_euclid(360) - 180,
    )
}

/// What square `key`'s surveyed square reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Reads {
    pub key: SquareKey,
    pub terrarium: Option<TerrariumPart>,
    pub glo30: Vec<Glo30Part>,
}

/// The extent of the bake grid's supersampled points: [lat_min, lat_max, lon_min, lon_max].
pub fn supersampled_bounds(grid: &SquareGrid) -> [f64; 4] {
    let [lat_min, lat_max, lon_min, lon_max] = grid.bounds();
    [
        lat_min - grid.dlat / 4.0,
        lat_max + grid.dlat / 4.0,
        lon_min - grid.dlon / 4.0,
        lon_max + grid.dlon / 4.0,
    ]
}

/// The Terrarium tiles over latitudes `lat_min..lat_max` and longitudes `lon_min..lon_max` at `zoom`, with the bilinear
/// interpolation's support: x0..=x1 by y0..=y1 (x continuous).
pub fn terrarium_span(zoom: u32, [lat_min, lat_max, lon_min, lon_max]: [f64; 4]) -> [i64; 4] {
    let scale = (1u64 << zoom) as f64 * 256.0;
    let (px0, px1) = (
        mercator_x(lon_min, scale) - 0.5,
        mercator_x(lon_max, scale) - 0.5,
    );
    let (py0, py1) = (
        mercator_y(lat_max, scale) - 0.5,
        mercator_y(lat_min, scale) - 0.5,
    );
    [
        (px0.floor() / 256.0).floor() as i64,
        ((px1.floor() + 1.0) / 256.0).floor() as i64,
        (py0.floor() / 256.0).floor() as i64,
        ((py1.floor() + 1.0) / 256.0).floor() as i64,
    ]
}

impl Reads {
    pub fn of(key: SquareKey) -> Self {
        let [lat_min, lat_max, lon_min, lon_max] = supersampled_bounds(&bake_grid(key));
        let terrarium = (lat_max >= SRTM_SOUTH && lat_min < SRTM_NORTH).then(|| {
            let zoom = terrarium_zoom(key);
            TerrariumPart {
                zoom,
                span: terrarium_span(
                    zoom,
                    [
                        lat_min.max(SRTM_SOUTH),
                        lat_max.min(SRTM_NORTH),
                        lon_min,
                        lon_max,
                    ],
                ),
            }
        });
        let mut glo30 = vec![];
        if lat_max >= SRTM_NORTH {
            glo30.push(Glo30Part {
                window: glo30::window(lat_min.max(SRTM_NORTH), lat_max, lon_min, lon_max),
                north: true,
            });
        }
        if lat_min < SRTM_SOUTH {
            glo30.push(Glo30Part {
                window: glo30::window(lat_min, lat_max.min(SRTM_SOUTH), lon_min, lon_max),
                north: false,
            });
        }
        Self {
            key,
            terrarium,
            glo30,
        }
    }

    /// The Terrarium tiles by side of ±180°: (x, y) wrapped.
    pub fn tile_sides(&self) -> Vec<Side<(i64, i64)>> {
        let Some(t) = &self.terrarium else {
            return vec![];
        };
        let n = 1i64 << t.zoom;
        let [x0, x1, y0, y1] = t.span;
        let mut sides: BTreeMap<i64, Vec<(i64, i64)>> = BTreeMap::new();
        for x in x0..=x1 {
            for y in y0..=y1 {
                sides
                    .entry(x.div_euclid(n))
                    .or_default()
                    .push((x.rem_euclid(n), y));
            }
        }
        sides
            .into_iter()
            .map(|(turn, keys)| Side { turn, keys })
            .collect()
    }

    /// Every Terrarium tile read, wrapped, in key order.
    pub fn tiles(&self) -> Vec<TileKey> {
        let Some(t) = &self.terrarium else {
            return vec![];
        };
        let mut out: Vec<TileKey> = self
            .tile_sides()
            .into_iter()
            .flat_map(|s| s.keys)
            .map(|(x, y)| (t.zoom, x, y))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// The GLO-30 cells by side of ±180°, wrapped.
    pub fn cell_sides(&self) -> Vec<Side<CellKey>> {
        let mut sides: BTreeMap<i64, Vec<CellKey>> = BTreeMap::new();
        for part in &self.glo30 {
            for (s, w) in part.cells() {
                let (turn, lon) = wrap_lon(w);
                let list = sides.entry(turn).or_default();
                if !list.contains(&(s, lon)) {
                    list.push((s, lon));
                }
            }
        }
        sides
            .into_iter()
            .map(|(turn, keys)| Side { turn, keys })
            .collect()
    }

    /// Every GLO-30 cell read, wrapped, in key order.
    pub fn cells(&self) -> Vec<CellKey> {
        let mut out: Vec<CellKey> = self.cell_sides().into_iter().flat_map(|s| s.keys).collect();
        out.sort();
        out.dedup();
        out
    }

    /// The blocks of wrapped cell `cell` (its image `header`) the reads take, in index order.
    pub fn glo30_blocks(&self, cell: CellKey, header: &glo30::Header) -> Vec<usize> {
        let mut out = vec![];
        for part in &self.glo30 {
            for (s, w) in part.cells() {
                if (s, wrap_lon(w).1) != cell {
                    continue;
                }
                if let Some(slice) = glo30::slice(&part.window, s, w, header.width) {
                    out.extend(glo30::slice_blocks(header, &slice));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Whether every place the reads serve reads one DEM, and which.
    pub fn only(&self) -> Option<Dem> {
        match (&self.terrarium, self.glo30.is_empty()) {
            (Some(_), true) => Some(Dem::Terrarium),
            (None, false) => Some(Dem::Glo30),
            _ => None,
        }
    }
}

/// The source manifest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceManifest {
    pub terrarium: BTreeMap<TileKey, Object>,
    /// None: the bucket has no such cell (open sea).
    pub glo30: BTreeMap<CellKey, Option<Object>>,
    /// The sea floor's object ([`super::gebco::url`]), from the second format on.
    pub seabed: Option<Object>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: u32,
    terrarium: String,
    glo30: String,
    #[serde(default)]
    seabed: Option<(String, String, u64)>,
    tiles: Vec<(u32, i64, i64, String, u64)>,
    cells: Vec<(i32, i32, Option<String>, Option<u64>)>,
}

fn quote(s: &str) -> String {
    serde_json::to_string(s).expect("a string serialises")
}

impl SourceManifest {
    /// The buckets the manifest names.
    pub fn glo30_base() -> String {
        format!("https://{}", glo30::HOST)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = String::new();
        let format = if self.seabed.is_some() {
            FORMAT_SEABED
        } else {
            FORMAT
        };
        out.push_str(&format!(
            "{{\"format\":{format},\"terrarium\":{},\"glo30\":{},",
            quote(TERRARIUM_URL),
            quote(&Self::glo30_base())
        ));
        if let Some(o) = &self.seabed {
            out.push_str(&format!(
                "\n\"seabed\":[{},{},{}],",
                quote(&super::gebco::url()),
                quote(&o.etag),
                o.bytes
            ));
        }
        out.push_str("\n\"tiles\":[");
        for (k, ((z, x, y), o)) in self.terrarium.iter().enumerate() {
            out.push_str(if k == 0 { "\n" } else { ",\n" });
            out.push_str(&format!("[{z},{x},{y},{},{}]", quote(&o.etag), o.bytes));
        }
        out.push_str("],\n\"cells\":[");
        for (k, ((lat, lon), o)) in self.glo30.iter().enumerate() {
            out.push_str(if k == 0 { "\n" } else { ",\n" });
            match o {
                Some(o) => out.push_str(&format!("[{lat},{lon},{},{}]", quote(&o.etag), o.bytes)),
                None => out.push_str(&format!("[{lat},{lon},null,null]")),
            }
        }
        out.push_str("]}\n");
        out.into_bytes()
    }

    /// A manifest from its file, refused unless it is the file [`Self::encode`] writes for it.
    pub fn parse(bytes: &[u8]) -> Result<Self, BakeError> {
        let bad = |why: String| BakeError::Corrupt(format!("not a source manifest: {why}"));
        let raw: Raw = serde_json::from_slice(bytes).map_err(|e| bad(e.to_string()))?;
        if raw.terrarium != TERRARIUM_URL || raw.glo30 != Self::glo30_base() {
            return Err(bad("other buckets".into()));
        }
        let mut out = Self::default();
        match (raw.format, raw.seabed) {
            (FORMAT, None) => {}
            (FORMAT_SEABED, Some((url, etag, bytes))) if url == super::gebco::url() => {
                out.seabed = Some(Object { etag, bytes });
            }
            _ => return Err(bad("another format, or a sea floor it does not know".into())),
        }
        for (z, x, y, etag, bytes) in raw.tiles {
            out.terrarium.insert((z, x, y), Object { etag, bytes });
        }
        for (lat, lon, etag, bytes) in raw.cells {
            let object = match (etag, bytes) {
                (Some(etag), Some(bytes)) => Some(Object { etag, bytes }),
                (None, None) => None,
                _ => return Err(bad(format!("cell {lat} {lon} is half listed"))),
            };
            out.glo30.insert((lat, lon), object);
        }
        if out.encode() != bytes {
            return Err(bad("not in its canonical spelling".into()));
        }
        Ok(out)
    }

    /// The tile's listing, or why it cannot be read.
    pub fn tile(&self, tile: TileKey) -> Result<&Object, BakeError> {
        self.terrarium.get(&tile).ok_or_else(|| {
            BakeError::Refused(format!(
                "Terrarium tile {}/{}/{} is not in the release's source manifest",
                tile.0, tile.1, tile.2
            ))
        })
    }

    /// The cell's listing (none: absent, open sea), or why it cannot be read.
    pub fn cell(&self, cell: CellKey) -> Result<Option<&Object>, BakeError> {
        self.glo30.get(&cell).map(Option::as_ref).ok_or_else(|| {
            BakeError::Refused(format!(
                "GLO-30 cell {} is not in the release's source manifest",
                glo30::name(cell.0, cell.1)
            ))
        })
    }

    /// The sea floor's listing, or why it cannot be read.
    pub fn seabed(&self) -> Result<&Object, BakeError> {
        self.seabed.as_ref().ok_or_else(|| {
            BakeError::Refused("the release's source manifest lists no sea floor".into())
        })
    }

    /// Whether it lists every object `reads` takes.
    pub fn covers(&self, reads: &Reads) -> Result<(), BakeError> {
        for tile in reads.tiles() {
            self.tile(tile)?;
        }
        for cell in reads.cells() {
            self.cell(cell)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_follow_the_place_and_split_at_the_date_line() {
        // Below 60° N: Terrarium alone; above 61°: GLO-30 alone; 59° and 60° read both, each its own side of 60° N.
        assert_eq!(
            Reads::of(SquareKey { lat: 36, lon: -123 }).only(),
            Some(Dem::Terrarium)
        );
        assert_eq!(
            Reads::of(SquareKey { lat: 62, lon: -20 }).only(),
            Some(Dem::Glo30)
        );
        assert_eq!(
            Reads::of(SquareKey { lat: -60, lon: -60 }).only(),
            Some(Dem::Glo30)
        );
        for lat in [59, 60, -57, -56] {
            let r = Reads::of(SquareKey { lat, lon: 10 });
            assert_eq!(r.only(), None, "{lat}");
            assert_eq!(r.glo30.len(), 1);
        }
        // 59° N reads GLO-30 only past 60° N (with its support): its cells are the row at 60° N and the row below for the
        // edge's interpolation.
        let r = Reads::of(SquareKey { lat: 59, lon: 10 });
        assert!(
            r.cells().iter().all(|c| c.0 >= 59 && c.0 <= 60),
            "{:?}",
            r.cells()
        );
        let t = r.terrarium.as_ref().unwrap();
        let top = mercator_lat_of_tile(t.zoom, t.span[2]);
        assert!(top > 60.0 && top < 60.5, "{top}");
        // 179° E: tiles and cells on both sides of ±180°, wrapped, the east side a turn on.
        let r = Reads::of(SquareKey { lat: 71, lon: 179 });
        let sides = r.cell_sides();
        assert_eq!(sides.iter().map(|s| s.turn).collect::<Vec<_>>(), [0, 1]);
        assert!(sides[1].keys.iter().all(|c| c.1 == -180));
        let r = Reads::of(SquareKey {
            lat: -17,
            lon: -180,
        });
        let sides = r.tile_sides();
        assert_eq!(sides.iter().map(|s| s.turn).collect::<Vec<_>>(), [-1, 0]);
        let n = 1i64 << r.terrarium.as_ref().unwrap().zoom;
        assert!(sides[0].keys.iter().all(|k| k.0 == n - 1));
        assert!(r.tiles().iter().all(|t| t.1 >= 0 && t.1 < n));
    }

    fn mercator_lat_of_tile(zoom: u32, y: i64) -> f64 {
        let scale = (1u64 << zoom) as f64 * 256.0;
        crate::geo::mercator_lat(y as f64 * 256.0, scale)
    }

    #[test]
    fn a_manifest_has_one_spelling_and_says_what_it_lacks() {
        let mut m = SourceManifest::default();
        m.terrarium.insert(
            (11, 2047, 1100),
            Object {
                etag: "6620d22e7b6ef92481fa4809db43d928".into(),
                bytes: 32692,
            },
        );
        m.glo30.insert(
            (71, -180),
            Some(Object {
                etag: "abc-3".into(),
                bytes: 9_000_000,
            }),
        );
        m.glo30.insert((72, 179), None);
        let bytes = m.encode();
        assert_eq!(SourceManifest::parse(&bytes).unwrap(), m);
        let spaced = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("\"format\":1", "\"format\": 1");
        assert!(SourceManifest::parse(spaced.as_bytes()).is_err());
        assert!(m.tile((11, 2047, 1101)).is_err());
        assert_eq!(m.cell((72, 179)).unwrap(), None);
        assert!(m.cell((72, 178)).is_err());
        let r = Reads::of(SquareKey { lat: 36, lon: -123 });
        assert!(m.covers(&r).is_err());
        // The first format, as land-v1 wrote it: no sea floor line.
        assert!(
            bytes.starts_with(b"{\"format\":1,")
                && !String::from_utf8_lossy(&bytes).contains("seabed")
        );
        assert!(m.seabed().is_err());
        // The second lists the sea floor's object, and is refused with the first's number or another object.
        let mut two = m.clone();
        two.seabed = Some(Object {
            etag: "7af0dab3bf255d0e474d17061e572ff6-392".into(),
            bytes: 3_286_944_666,
        });
        let b2 = two.encode();
        assert!(b2.starts_with(b"{\"format\":2,"));
        assert_eq!(SourceManifest::parse(&b2).unwrap(), two);
        let text = String::from_utf8(b2).unwrap();
        assert!(
            SourceManifest::parse(text.replacen("\"format\":2", "\"format\":1", 1).as_bytes())
                .is_err()
        );
        assert!(
            SourceManifest::parse(text.replace("GEBCO_2026", "GEBCO_2025").as_bytes()).is_err()
        );
        assert_eq!(
            terrarium_url((11, 5, 7)),
            format!("{TERRARIUM_URL}/11/5/7.png")
        );
    }
}
