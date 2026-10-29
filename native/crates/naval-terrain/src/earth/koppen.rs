//! The climate of a place: its Köppen-Geiger class for 1931-1960, the war's own climate
//! (native/docs/terrain/ground.md, "Climate and rock maps").
//!
//! The data is Beck et al.'s 1 km maps ([`ATTRIBUTION`]): per 30 arc-second cell (43200 x 21600 over the globe, about
//! 0.9 km at the equator), one of the 30 classes, or the sea. `scripts/theatre/bake-climate-and-rock.py` bakes it into
//! `koppen.nkg` (1° tiles, each its own zlib stream, compiled in), and [`KoppenMap`] decodes a tile when a lookup first
//! needs it, keeping the last [`CACHED_TILES`].

use std::sync::{Mutex, OnceLock};

use super::METRES_PER_DEGREE;

/// What the game must show where it credits its sources.
pub const ATTRIBUTION: &str = "Climate: Köppen-Geiger classes 1931-1960 from Beck, H. E. et al. (2023), \
     High-resolution (1 km) Köppen-Geiger maps for 1901-2099 based on constrained CMIP6 projections, Scientific Data \
     10, 724, doi:10.1038/s41597-023-02549-6, CC BY 4.0";

/// The baked map (`bake-climate-and-rock.py`'s docstring gives the layout).
const BAKED: &[u8] = include_bytes!("koppen.nkg");
const MAGIC: &[u8; 4] = b"NKG1";
/// The 1° tiles: rows of latitude from 90° N, columns of longitude from 180° W.
const TILE_ROWS: usize = 180;
const TILE_COLS: usize = 360;
const TILES: usize = TILE_ROWS * TILE_COLS;
/// A tile's kind in the index: below it, the class (0 the sea) that fills it; this, stored.
const STORED: u8 = 255;
/// How many decoded tiles the map keeps (a tile is n² bytes: 14 KB at 30"). A window and its horizon touch a few.
pub const CACHED_TILES: usize = 48;

/// A Köppen-Geiger class, numbered as Beck et al.'s maps number it (their `legend.txt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Koppen {
    /// Tropical rainforest.
    Af = 1,
    /// Tropical monsoon: a short dry season.
    Am,
    /// Tropical savannah: a dry winter.
    Aw,
    /// Hot desert.
    BWh,
    /// Cold desert.
    BWk,
    /// Hot steppe.
    BSh,
    /// Cold steppe.
    BSk,
    /// Mediterranean, hot summer.
    Csa,
    /// Mediterranean, warm summer.
    Csb,
    /// Mediterranean, cold summer.
    Csc,
    /// Dry winter, hot summer (monsoon-influenced humid subtropical).
    Cwa,
    /// Dry winter, warm summer (subtropical highland).
    Cwb,
    /// Dry winter, cold summer.
    Cwc,
    /// Humid subtropical.
    Cfa,
    /// Oceanic.
    Cfb,
    /// Subpolar oceanic.
    Cfc,
    /// Continental with a dry, hot summer.
    Dsa,
    /// Continental with a dry, warm summer.
    Dsb,
    /// Continental with a dry, cold summer.
    Dsc,
    /// Continental with a dry summer and a very cold winter.
    Dsd,
    /// Continental with a dry winter, a hot summer.
    Dwa,
    /// Continental with a dry winter, a warm summer.
    Dwb,
    /// Subarctic with a dry winter.
    Dwc,
    /// Subarctic with a dry, very cold winter.
    Dwd,
    /// Humid continental, hot summer.
    Dfa,
    /// Humid continental, warm summer.
    Dfb,
    /// Subarctic.
    Dfc,
    /// Subarctic with a very cold winter.
    Dfd,
    /// Tundra (above the trees, at the poles or on high mountains).
    ET,
    /// Ice cap.
    EF,
}

/// A class's main group, its first letter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KoppenGroup {
    /// A: every month warm (18 °C and more).
    Tropical,
    /// B: dry, rain short of what the warmth evaporates.
    Arid,
    /// C: mild winters (the coldest month above 0 °C, or −3 °C in some definitions; Beck's uses 0 °C).
    Temperate,
    /// D: cold winters, warm summers.
    Continental,
    /// E: no month above 10 °C.
    Polar,
}

/// The season a class is dry in (its second letter for A, C and D: `s` summer, `w` winter; `m`, the monsoon's short
/// dry season, counts as winter's).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrySeason {
    Summer,
    Winter,
}

/// How warm a class's summer runs (its third letter): `h` and `a` hot, `b` warm, `c` and `k` cold, `d` cold with a
/// very cold winter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Warmth {
    Hot,
    Warm,
    Cold,
    Severe,
}

impl Koppen {
    /// Every class, in order of its number (`ALL[k - 1]` is class k).
    pub const ALL: [Koppen; 30] = [
        Koppen::Af,
        Koppen::Am,
        Koppen::Aw,
        Koppen::BWh,
        Koppen::BWk,
        Koppen::BSh,
        Koppen::BSk,
        Koppen::Csa,
        Koppen::Csb,
        Koppen::Csc,
        Koppen::Cwa,
        Koppen::Cwb,
        Koppen::Cwc,
        Koppen::Cfa,
        Koppen::Cfb,
        Koppen::Cfc,
        Koppen::Dsa,
        Koppen::Dsb,
        Koppen::Dsc,
        Koppen::Dsd,
        Koppen::Dwa,
        Koppen::Dwb,
        Koppen::Dwc,
        Koppen::Dwd,
        Koppen::Dfa,
        Koppen::Dfb,
        Koppen::Dfc,
        Koppen::Dfd,
        Koppen::ET,
        Koppen::EF,
    ];

    /// The class numbered `k` (1-30), none for the sea (0) or a number off the legend.
    pub fn from_number(k: u8) -> Option<Koppen> {
        Self::ALL.get(usize::from(k).wrapping_sub(1)).copied()
    }

    /// Its number in the legend (1-30).
    pub fn number(self) -> u8 {
        self as u8
    }

    /// Its letters ("Af", "BWh", "Cfb", "ET").
    pub fn code(self) -> &'static str {
        const CODES: [&str; 30] = [
            "Af", "Am", "Aw", "BWh", "BWk", "BSh", "BSk", "Csa", "Csb", "Csc", "Cwa", "Cwb", "Cwc",
            "Cfa", "Cfb", "Cfc", "Dsa", "Dsb", "Dsc", "Dsd", "Dwa", "Dwb", "Dwc", "Dwd", "Dfa",
            "Dfb", "Dfc", "Dfd", "ET", "EF",
        ];
        CODES[usize::from(self.number()) - 1]
    }

    /// Its main group.
    pub fn group(self) -> KoppenGroup {
        match self.number() {
            1..=3 => KoppenGroup::Tropical,
            4..=7 => KoppenGroup::Arid,
            8..=16 => KoppenGroup::Temperate,
            17..=28 => KoppenGroup::Continental,
            _ => KoppenGroup::Polar,
        }
    }

    /// Desert or steppe (B).
    pub fn is_arid(self) -> bool {
        self.group() == KoppenGroup::Arid
    }

    /// Desert (BW).
    pub fn is_desert(self) -> bool {
        matches!(self, Koppen::BWh | Koppen::BWk)
    }

    /// Steppe (BS).
    pub fn is_steppe(self) -> bool {
        matches!(self, Koppen::BSh | Koppen::BSk)
    }

    /// The season it is dry in, none where rain falls the year round (`f`) and for the arid and polar classes (dry, or
    /// frozen, in every season).
    pub fn dry_season(self) -> Option<DrySeason> {
        use Koppen::*;
        match self {
            Csa | Csb | Csc | Dsa | Dsb | Dsc | Dsd => Some(DrySeason::Summer),
            Am | Aw | Cwa | Cwb | Cwc | Dwa | Dwb | Dwc | Dwd => Some(DrySeason::Winter),
            _ => None,
        }
    }

    /// How warm its summer runs, none for the tropical and polar classes (every month warm; none warm).
    pub fn warmth(self) -> Option<Warmth> {
        use Koppen::*;
        match self {
            BWh | BSh | Csa | Cwa | Cfa | Dsa | Dwa | Dfa => Some(Warmth::Hot),
            Csb | Cwb | Cfb | Dsb | Dwb | Dfb => Some(Warmth::Warm),
            BWk | BSk | Csc | Cwc | Cfc | Dsc | Dwc | Dfc => Some(Warmth::Cold),
            Dsd | Dwd | Dfd => Some(Warmth::Severe),
            _ => None,
        }
    }
}

/// How many cells of each class (and of the sea) lie about a point ([`koppen_shares`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KoppenShares {
    /// Cells per class number: `counts[0]` the sea's, `counts[k]` class k's.
    pub counts: [u32; 31],
}

impl KoppenShares {
    /// Every cell counted, the sea's too.
    pub fn total(&self) -> u32 {
        self.counts.iter().sum()
    }

    /// The land's cells.
    pub fn land(&self) -> u32 {
        self.total() - self.counts[0]
    }

    /// The share of the land's cells (0-1) whose class meets `which`; none with no land.
    pub fn share_where(&self, which: impl Fn(Koppen) -> bool) -> Option<f32> {
        let land = self.land();
        (land > 0).then(|| {
            let n: u32 = Koppen::ALL
                .iter()
                .filter(|k| which(**k))
                .map(|k| self.counts[usize::from(k.number())])
                .sum();
            n as f32 / land as f32
        })
    }

    /// The share of the land's cells (0-1) of `class`; none with no land.
    pub fn share(&self, class: Koppen) -> Option<f32> {
        self.share_where(|k| k == class)
    }

    /// The commonest class on the land (the lowest-numbered of a tie), none with no land.
    pub fn mode(&self) -> Option<Koppen> {
        self.mode_where(|_| true)
    }

    /// The commonest class on the land of those meeting `which` (the lowest-numbered of a tie), none with none of them.
    pub fn mode_where(&self, which: impl Fn(Koppen) -> bool) -> Option<Koppen> {
        let (k, n) = Koppen::ALL
            .iter()
            .filter(|k| which(**k))
            .map(|k| (*k, self.counts[usize::from(k.number())]))
            .fold(
                (Koppen::Af, 0),
                |best, (k, n)| {
                    if n > best.1 { (k, n) } else { best }
                },
            );
        (n > 0).then_some(k)
    }
}

/// The decoded map: the tiles' index, and the stored tiles decoded as lookups need them.
pub struct KoppenMap<'a> {
    /// Cells a degree (120 at 30"); a tile is `per_degree`² cells.
    pub per_degree: usize,
    /// Per tile: the class filling it (0 the sea), or [`STORED`].
    kinds: Vec<u8>,
    /// Per tile: where its stream lies in `bytes` (empty unless stored).
    streams: Vec<(u32, u32)>,
    bytes: &'a [u8],
    /// The decoded tiles, the last read first.
    cache: Mutex<Vec<(u32, Box<[u8]>)>>,
}

impl<'a> KoppenMap<'a> {
    /// The map from its baked bytes (checked whole except the tiles' own streams, which are inflated when read).
    pub fn decode(bytes: &'a [u8]) -> Result<Self, String> {
        let body = bytes
            .strip_prefix(MAGIC)
            .ok_or("not a Köppen map (no NKG1)")?;
        let header = body.get(..8).ok_or("Köppen map: cut short")?;
        let per_degree = usize::from(u16::from_le_bytes([header[0], header[1]]));
        if !(1..=240).contains(&per_degree) || header[2] != 30 {
            return Err(format!(
                "Köppen map: {per_degree} cells a degree, {} classes",
                header[2]
            ));
        }
        let index_len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let index = body
            .get(8..8 + index_len)
            .ok_or("Köppen map: the index cut short")?;
        let index = miniz_oxide::inflate::decompress_to_vec_zlib(index)
            .map_err(|e| format!("Köppen map index: {e:?}"))?;
        let (kinds, lengths) = index
            .split_at_checked(TILES)
            .ok_or("Köppen map: the index cut short")?;
        if kinds.iter().any(|&k| k > 30 && k != STORED) {
            return Err("Köppen map: a tile of no class".into());
        }
        let stored = kinds.iter().filter(|&&k| k == STORED).count();
        if lengths.len() != 4 * stored {
            return Err(format!(
                "Köppen map: {} bytes of lengths for {stored} stored tiles",
                lengths.len()
            ));
        }
        let mut lengths = lengths
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b));
        let mut at = (MAGIC.len() + 8 + index_len) as u32;
        let streams = kinds
            .iter()
            .map(|&k| {
                if k != STORED {
                    return (at, at);
                }
                let start = at;
                at += lengths.next().unwrap_or(0);
                (start, at)
            })
            .collect();
        if at as usize != bytes.len() {
            return Err(format!(
                "Köppen map: the tiles end at {at} of {} bytes",
                bytes.len()
            ));
        }
        Ok(Self {
            per_degree,
            kinds: kinds.to_vec(),
            streams,
            bytes,
            cache: Mutex::new(Vec::with_capacity(CACHED_TILES)),
        })
    }

    /// Rows and columns of cells over the globe.
    pub fn rows(&self) -> usize {
        TILE_ROWS * self.per_degree
    }

    pub fn cols(&self) -> usize {
        TILE_COLS * self.per_degree
    }

    /// The cell a point stands in: its row from 90° N and column from 180° W (longitude wraps, latitude clamps), none
    /// for a point off the earth (NaN).
    pub fn cell_of(&self, lat: f64, lon: f64) -> Option<(usize, usize)> {
        if !lat.is_finite() || !lon.is_finite() {
            return None;
        }
        let n = self.per_degree as f64;
        let row = ((90.0 - lat) * n)
            .floor()
            .clamp(0.0, (self.rows() - 1) as f64) as usize;
        let col = ((lon + 180.0) * n).floor().rem_euclid(self.cols() as f64) as usize;
        Some((row, col.min(self.cols() - 1)))
    }

    /// The class numbers (0 the sea) of tile `tile` handed to `read` as rows of `per_degree`, decoded if stored.
    fn with_tile<T>(&self, tile: usize, read: impl FnOnce(TileCells) -> T) -> T {
        let kind = self.kinds[tile];
        if kind != STORED {
            return read(TileCells::Filled(kind));
        }
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let found = cache.iter().position(|(t, _)| *t == tile as u32);
        let at = match found {
            Some(at) => at,
            None => {
                let (start, end) = self.streams[tile];
                let n = self.per_degree;
                let cells = miniz_oxide::inflate::decompress_to_vec_zlib(
                    &self.bytes[start as usize..end as usize],
                )
                .ok()
                .filter(|cells| cells.len() == n * n)
                .unwrap_or_else(|| {
                    panic!("Köppen map: tile {tile} does not inflate to {n}² cells")
                });
                if cache.len() == CACHED_TILES {
                    cache.pop();
                }
                cache.push((tile as u32, cells.into_boxed_slice()));
                cache.len() - 1
            }
        };
        // The last read first, so the oldest falls off the end.
        cache[..=at].rotate_right(1);
        read(TileCells::Stored(&cache[0].1, self.per_degree))
    }

    /// The class number (0 the sea) of cell (`row`, `col`).
    pub fn cell(&self, row: usize, col: usize) -> u8 {
        let n = self.per_degree;
        self.with_tile((row / n) * TILE_COLS + col / n, |cells| {
            cells.at(row % n, col % n)
        })
    }

    /// The class at a point, none at sea.
    pub fn at(&self, lat: f64, lon: f64) -> Option<Koppen> {
        let (row, col) = self.cell_of(lat, lon)?;
        Koppen::from_number(self.cell(row, col))
    }

    /// The cells of each class whose middles lie within `radius_m` of a point north and south, and east and west (a
    /// square on the chart, `radius_m` from its middle to its sides; at least the point's own cell).
    pub fn shares(&self, lat: f64, lon: f64, radius_m: f64) -> KoppenShares {
        let mut counts = [0u32; 31];
        let Some((row, col)) = self.cell_of(lat, lon) else {
            return KoppenShares { counts };
        };
        let n = self.per_degree as f64;
        let cell_m = METRES_PER_DEGREE / n;
        let reach_rows = (radius_m.max(0.0) / cell_m).round() as isize;
        let cos = lat.to_radians().cos().max(0.05);
        let reach_cols =
            ((radius_m.max(0.0) / (cell_m * cos)).round() as isize).min(self.cols() as isize / 2);
        let rows = (row as isize - reach_rows).max(0) as usize
            ..=(row as isize + reach_rows).min(self.rows() as isize - 1) as usize;
        let per = self.per_degree;
        // Tile by tile: each row of tiles, and within it each column of tiles the square crosses.
        let mut r = *rows.start();
        while r <= *rows.end() {
            let tile_row = r / per;
            let r_end = ((tile_row + 1) * per - 1).min(*rows.end());
            let mut c = col as isize - reach_cols;
            let c_last = col as isize + reach_cols;
            while c <= c_last {
                let wrapped = c.rem_euclid(self.cols() as isize) as usize;
                let tile_col = wrapped / per;
                let run = ((tile_col + 1) * per - wrapped).min((c_last - c + 1) as usize);
                self.with_tile(tile_row * TILE_COLS + tile_col, |cells| {
                    for rr in r..=r_end {
                        for cc in wrapped..wrapped + run {
                            counts[usize::from(cells.at(rr % per, cc % per))] += 1;
                        }
                    }
                });
                c += run as isize;
            }
            r = r_end + 1;
        }
        KoppenShares { counts }
    }

    /// The game's map, decoded on first use.
    pub fn baked() -> &'static KoppenMap<'static> {
        static MAP: OnceLock<KoppenMap<'static>> = OnceLock::new();
        MAP.get_or_init(|| KoppenMap::decode(BAKED).expect("the baked Köppen map decodes"))
    }
}

/// A tile's cells as [`KoppenMap::with_tile`] hands them over.
enum TileCells<'t> {
    /// One class (0 the sea) throughout.
    Filled(u8),
    /// Rows of `n` cells.
    Stored(&'t [u8], usize),
}

impl TileCells<'_> {
    fn at(&self, row: usize, col: usize) -> u8 {
        match *self {
            TileCells::Filled(k) => k,
            TileCells::Stored(cells, n) => cells[row * n + col],
        }
    }
}

/// The Köppen-Geiger class at a point of the earth (its 30" cell's), none at sea.
pub fn koppen(lat: f64, lon: f64) -> Option<Koppen> {
    KoppenMap::baked().at(lat, lon)
}

/// The cells of each class about a point, within `radius_m` north, south, east and west ([`KoppenMap::shares`]).
pub fn koppen_shares(lat: f64, lon: f64, radius_m: f64) -> KoppenShares {
    KoppenMap::baked().shares(lat, lon, radius_m)
}

/// The commonest class on the land within `radius_m` of a point (as [`koppen_shares`] counts), none with no land
/// there: a cell of the land's grids takes this over its own extent rather than the one class at its middle.
pub fn koppen_mode(lat: f64, lon: f64, radius_m: f64) -> Option<Koppen> {
    koppen_shares(lat, lon, radius_m).mode()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map as the bake writes it, `per_degree` cells a degree, from the class of each cell.
    fn bake(per_degree: usize, class: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        let n = per_degree;
        let mut kinds = Vec::with_capacity(TILES);
        let mut lengths = Vec::new();
        let mut streams = Vec::new();
        for tr in 0..TILE_ROWS {
            for tc in 0..TILE_COLS {
                let cells: Vec<u8> = (0..n * n)
                    .map(|k| class(tr * n + k / n, tc * n + k % n))
                    .collect();
                if cells.iter().all(|&c| c == cells[0]) {
                    kinds.push(cells[0]);
                } else {
                    kinds.push(STORED);
                    let stream = miniz_oxide::deflate::compress_to_vec_zlib(&cells, 6);
                    lengths.extend_from_slice(&(stream.len() as u32).to_le_bytes());
                    streams.extend_from_slice(&stream);
                }
            }
        }
        kinds.extend_from_slice(&lengths);
        let index = miniz_oxide::deflate::compress_to_vec_zlib(&kinds, 6);
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&(n as u16).to_le_bytes());
        out.extend_from_slice(&[30, 0]);
        out.extend_from_slice(&(index.len() as u32).to_le_bytes());
        out.extend_from_slice(&index);
        out.extend_from_slice(&streams);
        out
    }

    #[test]
    fn decodes_what_the_bake_writes() {
        // Two cells a degree: Cfb north of the equator east of the meridian, the sea elsewhere, and one degree (the
        // one from 10° N, 20° E) split into its four cells: Af, BWh, Csa and the sea.
        let bytes = bake(2, |row, col| match (row, col) {
            (158, 400) => 1,
            (158, 401) => 4,
            (159, 400) => 8,
            (159, 401) => 0,
            (r, c) if r < 180 && c >= 360 => 15,
            _ => 0,
        });
        let map = KoppenMap::decode(&bytes).unwrap();
        assert_eq!(map.per_degree, 2);
        assert_eq!(map.at(10.75, 20.25), Some(Koppen::Af));
        assert_eq!(map.at(10.75, 20.75), Some(Koppen::BWh));
        assert_eq!(map.at(10.25, 20.25), Some(Koppen::Csa));
        assert_eq!(map.at(10.25, 20.75), None);
        assert_eq!(map.at(45.0, 100.0), Some(Koppen::Cfb));
        assert_eq!(map.at(-45.0, 100.0), None);
        assert_eq!(map.at(45.0, -100.0), None);
        assert_eq!(map.at(f64::NAN, 0.0), None);
        // Longitude wraps; the poles clamp.
        assert_eq!(map.at(45.0, 100.0 - 360.0), Some(Koppen::Cfb));
        assert_eq!(map.at(90.0, 10.0), Some(Koppen::Cfb));
        // A cut or a stray byte is refused.
        assert!(KoppenMap::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(KoppenMap::decode(&longer).is_err());
        assert!(KoppenMap::decode(b"NBH1").is_err());
    }

    #[test]
    fn the_shares_count_the_cells_about_a_point_across_tiles() {
        // Ten cells a degree: class 1 + (column mod 3) north of 10° S in the degrees of longitude about the meridian,
        // 50° E and the antimeridian; the sea elsewhere.
        let bytes = bake(10, |row, col| {
            let land = row < 1000 && [0, 179, 180, 229, 230, 359].contains(&(col / 10));
            if land { 1 + (col % 3) as u8 } else { 0 }
        });
        let map = KoppenMap::decode(&bytes).unwrap();
        let cell_m = METRES_PER_DEGREE / 10.0;
        // At the equator, two cells each way: a 5 x 5 square astride the meridian (two tiles), five columns.
        let shares = map.shares(0.05, 0.05, 2.0 * cell_m);
        assert_eq!(shares.total(), 25);
        assert_eq!(shares.land(), 25);
        let col = map.cell_of(0.05, 0.05).unwrap().1;
        let expect = |k: u8| {
            (col - 2..=col + 2)
                .filter(|c| 1 + (c % 3) as u8 == k)
                .count() as u32
                * 5
        };
        for k in 1..=3u8 {
            assert_eq!(shares.counts[usize::from(k)], expect(k), "class {k}");
        }
        // Five columns: two classes twice, one once; the mode is the lower of the two.
        let most = (1..=3).map(expect).max().unwrap();
        let lowest = (1..=3u8).find(|&k| expect(k) == most).unwrap();
        assert_eq!(shares.mode(), Koppen::from_number(lowest));
        // On 10° S (the first row of the sea): three rows of the five the sea's; the shares are of the land.
        let coast = map.shares(-10.0, 50.0, 2.0 * cell_m);
        assert_eq!(coast.counts[0], 15);
        assert_eq!(coast.land(), 10);
        assert!((coast.share_where(|k| k.number() <= 3).unwrap() - 1.0).abs() < 1e-6);
        // Across the antimeridian, and a radius below a cell keeps the point's own.
        assert_eq!(map.shares(0.05, 179.95, 2.0 * cell_m).total(), 25);
        assert_eq!(map.shares(0.05, 0.05, 0.0).total(), 1);
        // Toward the poles a degree of longitude shortens: more columns for the same radius.
        assert!(map.shares(60.0, 0.05, 2.0 * cell_m).total() > 25);
        assert_eq!(map.shares(-50.0, 0.0, cell_m).mode(), None);
    }

    #[test]
    fn the_classes_know_their_letters_groups_and_seasons() {
        for (k, class) in Koppen::ALL.iter().enumerate() {
            assert_eq!(usize::from(class.number()), k + 1);
            assert_eq!(Koppen::from_number(class.number()), Some(*class));
            assert_eq!(format!("{class:?}"), class.code());
            let group = match class.code().as_bytes()[0] {
                b'A' => KoppenGroup::Tropical,
                b'B' => KoppenGroup::Arid,
                b'C' => KoppenGroup::Temperate,
                b'D' => KoppenGroup::Continental,
                _ => KoppenGroup::Polar,
            };
            assert_eq!(class.group(), group, "{class:?}");
        }
        assert_eq!(Koppen::from_number(0), None);
        assert_eq!(Koppen::from_number(31), None);
        assert!(Koppen::BWk.is_arid() && Koppen::BWk.is_desert() && !Koppen::BWk.is_steppe());
        assert!(Koppen::BSh.is_steppe() && !Koppen::Csa.is_arid());
        assert_eq!(Koppen::Csb.dry_season(), Some(DrySeason::Summer));
        assert_eq!(Koppen::Dsa.dry_season(), Some(DrySeason::Summer));
        assert_eq!(Koppen::Cwa.dry_season(), Some(DrySeason::Winter));
        assert_eq!(Koppen::Aw.dry_season(), Some(DrySeason::Winter));
        assert_eq!(Koppen::Cfb.dry_season(), None);
        assert_eq!(Koppen::BWh.dry_season(), None);
        assert_eq!(Koppen::Dsa.warmth(), Some(Warmth::Hot));
        assert_eq!(Koppen::Cfb.warmth(), Some(Warmth::Warm));
        assert_eq!(Koppen::BWk.warmth(), Some(Warmth::Cold));
        assert_eq!(Koppen::Dfd.warmth(), Some(Warmth::Severe));
        assert_eq!(Koppen::ET.warmth(), None);
    }

    /// The places the land lanes judge the climate at, and their 1931-1960 classes in Beck's 30" cells.
    const PLACES: [(&str, f64, f64, Koppen); 10] = [
        ("Grand Canyon, South Rim", 36.06, -112.11, Koppen::Dsa),
        ("Grand Canyon, inner gorge", 36.10, -112.10, Koppen::BWk),
        ("Eiger", 46.58, 8.0, Koppen::ET),
        ("Interlaken", 46.69, 7.86, Koppen::Dfb),
        ("Taiwan, the high range", 24.13, 121.33, Koppen::Cfb),
        ("Hualien", 24.0, 121.6, Koppen::Cfa),
        ("Rinjani", -8.41, 116.46, Koppen::ET),
        ("Lombok, the coast", -8.25, 116.45, Koppen::Aw),
        ("Geiranger", 62.1, 7.1, Koppen::Dfc),
        ("Dalsnibba", 62.05, 7.27, Koppen::ET),
    ];

    #[test]
    fn the_baked_map_gives_each_place_its_climate() {
        let map = KoppenMap::baked();
        assert_eq!(map.per_degree, 120);
        for (name, lat, lon, class) in PLACES {
            assert_eq!(koppen(lat, lon), Some(class), "{name}");
        }
        // Mid-Pacific and mid-Atlantic: the sea. The Sahara, Antarctica's ice and the Amazon.
        assert_eq!(koppen(30.0, -140.0), None);
        assert_eq!(koppen(40.0, -40.0), None);
        assert_eq!(koppen(23.0, 10.0), Some(Koppen::BWh));
        assert_eq!(koppen(-80.0, 30.0), Some(Koppen::EF));
        assert_eq!(koppen(-3.0, -60.0), Some(Koppen::Af));
        // A 3 km cell about the South Rim: its own 30" cell is the rim's Dsa, but most of the cells about it lie below
        // the rim, in the canyon's steppe.
        assert_eq!(koppen_mode(36.06, -112.11, 1500.0), Some(Koppen::BSk));
        // 40 km about the canyon: the desert and steppe a large part of it.
        let around = koppen_shares(36.1, -112.1, 40_000.0);
        assert!(
            around.share_where(Koppen::is_arid).unwrap() > 0.2,
            "{around:?}"
        );
        assert!(around.land() > 7000);
    }

    /// `bun run rust:test -- --native -p naval-terrain --lib earth::koppen -- --nocapture --ignored` prints the
    /// places' classes and their surroundings'.
    #[test]
    #[ignore = "a report, not a check"]
    fn report_the_places() {
        for (name, lat, lon, _) in PLACES {
            let shares = koppen_shares(lat, lon, 40_000.0);
            println!(
                "{name:28} {:4} mode 1.5 km {:4} 40 km {:4} arid {:.2}",
                koppen(lat, lon).map_or("sea", Koppen::code),
                koppen_mode(lat, lon, 1500.0).map_or("sea", Koppen::code),
                shares.mode().map_or("sea", Koppen::code),
                shares.share_where(Koppen::is_arid).unwrap_or(0.0)
            );
        }
    }
}
