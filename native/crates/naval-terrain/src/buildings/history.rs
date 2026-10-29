//! How much of today's built-up land stood in a past year (native/docs/terrain/period.md, "Built-up history"):
//! the share the period's buildings keep about a place.
//!
//! The data is HYDE 3.3's built-up area ([`ATTRIBUTION`]): per 5 arc-minute cell (4320 x 2160 over the globe, about
//! 9 km at the equator), the share of the cell's 2023 built-up area that stood in each decade 1900-1960, for every cell
//! built over by more than a hectare in 2023. `scripts/theatre/bake-built-up-history.py` bakes it into `history.nbh`
//! (about 1 MB, compiled in: no file is read at run time), decoded once on the first lookup.

use std::sync::OnceLock;

/// What the game must show where it credits its sources.
pub const ATTRIBUTION: &str = "Built-up history: HYDE 3.3, History Database of the Global Environment (Klein Goldewijk \
     et al., Utrecht University and PBL Netherlands Environmental Assessment Agency, doi:10.24416/UU01-94FNH0), \
     CC BY 4.0";

/// The baked history (`bake-built-up-history.py`'s header gives the layout).
const BAKED: &[u8] = include_bytes!("history.nbh");
const MAGIC: &[u8; 4] = b"NBH1";
/// The grid's columns (5 arc-minutes of longitude each, from 180° W) and rows (of latitude, from 90° N).
pub const COLS: u32 = 4320;
pub const ROWS: u32 = 2160;
const CELLS_PER_DEGREE: f64 = 12.0;

/// The decoded history: cells with data in ascending index (row × [`COLS`] + col), each with a share per decade.
#[derive(Debug, PartialEq)]
pub struct History {
    /// The decades with a share, ascending.
    pub decades: Vec<u16>,
    /// The year the shares are of (share 1 then).
    pub latest: u16,
    /// Per row, where its cells start in `cols` (`ROWS + 1` entries).
    row_starts: Vec<u32>,
    /// Each cell's column, ascending within its row.
    cols: Vec<u16>,
    /// Each cell's shares (× 255), one per decade, cell after cell.
    shares: Vec<u8>,
}

impl History {
    /// The history from its baked bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let body = bytes
            .strip_prefix(MAGIC)
            .ok_or("not a built-up history (no NBH1)")?;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(body)
            .map_err(|e| format!("built-up history: {e:?}"))?;
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let slice = raw.get(at..at + n).ok_or("built-up history: cut short")?;
            at += n;
            Ok(slice)
        };
        let u16_at = |b: &[u8]| u16::from_le_bytes([b[0], b[1]]);
        let n = usize::from(u16_at(take(2)?));
        let decades: Vec<u16> = take(2 * n)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        if n == 0 || decades.windows(2).any(|w| w[0] >= w[1]) {
            return Err("built-up history: decades not ascending".into());
        }
        let latest = u16_at(take(2)?);
        if latest <= decades[n - 1] {
            return Err("built-up history: the latest year precedes a decade".into());
        }
        let count = take(4)?;
        let m = u32::from_le_bytes([count[0], count[1], count[2], count[3]]) as usize;
        let mut row_starts = vec![0u32; ROWS as usize + 1];
        let mut cols = Vec::with_capacity(m);
        let mut cell: i64 = -1;
        for _ in 0..m {
            let mut delta: u64 = 0;
            let mut shift = 0;
            loop {
                let byte = take(1)?[0];
                delta |= u64::from(byte & 0x7F) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
                if shift > 35 {
                    return Err("built-up history: a cell index overruns".into());
                }
            }
            cell += delta as i64 + 1;
            if cell >= i64::from(COLS) * i64::from(ROWS) {
                return Err("built-up history: a cell off the grid".into());
            }
            let row = (cell / i64::from(COLS)) as usize;
            row_starts[row + 1] += 1;
            cols.push((cell % i64::from(COLS)) as u16);
        }
        for row in 0..ROWS as usize {
            row_starts[row + 1] += row_starts[row];
        }
        let planes = take(n * m)?;
        if at != raw.len() {
            return Err("built-up history: bytes after the shares".into());
        }
        let mut shares = vec![0u8; n * m];
        for (decade, plane) in planes.chunks_exact(m.max(1)).enumerate().take(n) {
            for (cell, &share) in plane.iter().enumerate() {
                shares[cell * n + decade] = share;
            }
        }
        Ok(Self {
            decades,
            latest,
            row_starts,
            cols,
            shares,
        })
    }

    /// The game's history, decoded on first use.
    pub fn baked() -> &'static Self {
        static HISTORY: OnceLock<History> = OnceLock::new();
        HISTORY.get_or_init(|| Self::decode(BAKED).expect("the baked built-up history decodes"))
    }

    /// The cells with data.
    pub fn len(&self) -> usize {
        self.cols.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.is_empty()
    }

    /// Cell (`row`, `col`)'s position among the cells with data.
    fn find(&self, row: u32, col: u32) -> Option<usize> {
        let (start, end) = (
            self.row_starts[row as usize] as usize,
            self.row_starts[row as usize + 1] as usize,
        );
        let at = self.cols[start..end].binary_search(&(col as u16)).ok()?;
        Some(start + at)
    }

    /// Cell (`row`, `col`)'s shares (× 255) per decade, none where it has no data.
    pub fn cell(&self, row: u32, col: u32) -> Option<&[u8]> {
        let n = self.decades.len();
        self.find(row, col)
            .map(|i| &self.shares[i * n..(i + 1) * n])
    }

    /// A cell's share in `year`: its first decade's before it, linear between decades, toward 1 at the latest year.
    fn share_in(&self, shares: &[u8], year: u16) -> f32 {
        let decades = &self.decades;
        let q = |i: usize| f32::from(shares[i]) / 255.0;
        if year <= decades[0] {
            return q(0);
        }
        if year >= self.latest {
            return 1.0;
        }
        let next = decades.partition_point(|&d| d <= year);
        let (from, to, a, b) = match decades.get(next) {
            Some(&to) => (decades[next - 1], to, q(next - 1), q(next)),
            None => (decades[next - 1], self.latest, q(next - 1), 1.0),
        };
        let t = f32::from(year - from) / f32::from(to - from);
        a + (b - a) * t
    }

    /// The share (0-1) of the land built up at the latest year about (`lat`, `lon`) that stood in `year`: bilinear
    /// between the four nearest cells' centres, weighted over those with data; none where none of them has any.
    pub fn stood_share(&self, lat: f64, lon: f64, year: u16) -> Option<f32> {
        if !lat.is_finite() || !lon.is_finite() {
            return None;
        }
        // Cell (row, col)'s centre is at fractional (row, col); rows from 90° N, columns from 180° W.
        let fy = (90.0 - lat.clamp(-90.0, 90.0)) * CELLS_PER_DEGREE - 0.5;
        let fx = (lon + 180.0).rem_euclid(360.0) * CELLS_PER_DEGREE - 0.5;
        let (r0, c0) = (fy.floor(), fx.floor());
        let (ty, tx) = ((fy - r0) as f32, (fx - c0) as f32);
        let (r0, c0) = (r0 as i64, c0 as i64);
        let mut sum = 0.0f32;
        let mut weight = 0.0f32;
        for (dr, wy) in [(0, 1.0 - ty), (1, ty)] {
            let row = r0 + dr;
            if !(0..i64::from(ROWS)).contains(&row) {
                continue;
            }
            for (dc, wx) in [(0, 1.0 - tx), (1, tx)] {
                let w = wy * wx;
                if w <= 0.0 {
                    continue;
                }
                let col = (c0 + dc).rem_euclid(i64::from(COLS));
                if let Some(shares) = self.cell(row as u32, col as u32) {
                    sum += w * self.share_in(shares, year);
                    weight += w;
                }
            }
        }
        (weight > 0.0).then(|| (sum / weight).clamp(0.0, 1.0))
    }
}

/// The share (0-1) of the land built up today at (`lat`, `lon`) that was built up in `year`; none where the history
/// has no data there (no built-up land today about it). Years before the first decade (1900) take its share; a year at
/// or after the history's latest (2023) is 1 wherever there is data.
pub fn stood_share(lat: f64, lon: f64, year: u16) -> Option<f32> {
    History::baked().stood_share(lat, lon, year)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes in the bake's layout (`bake-built-up-history.py`) for `cells` of (index, shares per decade), ascending.
    fn bake(decades: &[u16], latest: u16, cells: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend((decades.len() as u16).to_le_bytes());
        for d in decades {
            body.extend(d.to_le_bytes());
        }
        body.extend(latest.to_le_bytes());
        body.extend((cells.len() as u32).to_le_bytes());
        let mut previous: i64 = -1;
        for (cell, _) in cells {
            let mut delta = (i64::from(*cell) - previous - 1) as u64;
            previous = i64::from(*cell);
            loop {
                let byte = (delta & 0x7F) as u8;
                delta >>= 7;
                if delta == 0 {
                    body.push(byte);
                    break;
                }
                body.push(byte | 0x80);
            }
        }
        for decade in 0..decades.len() {
            body.extend(cells.iter().map(|(_, shares)| shares[decade]));
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend(miniz_oxide::deflate::compress_to_vec_zlib(&body, 6));
        bytes
    }

    fn index(row: u32, col: u32) -> u32 {
        row * COLS + col
    }

    /// The centre of cell (`row`, `col`).
    fn centre(row: u32, col: u32) -> (f64, f64) {
        (
            90.0 - (f64::from(row) + 0.5) / 12.0,
            -180.0 + (f64::from(col) + 0.5) / 12.0,
        )
    }

    #[test]
    fn decodes_what_the_bake_writes() {
        let cells = vec![
            (index(0, 0), vec![0, 10]),
            (index(0, 4319), vec![1, 11]),
            (index(700, 17), vec![2, 12]),
            (index(700, 300), vec![3, 13]),
            (index(2159, 4319), vec![255, 255]),
        ];
        let history = History::decode(&bake(&[1940, 1950], 2023, &cells)).unwrap();
        assert_eq!(history.decades, [1940, 1950]);
        assert_eq!(history.latest, 2023);
        assert_eq!(history.len(), cells.len());
        for (cell, shares) in &cells {
            assert_eq!(
                history.cell(cell / COLS, cell % COLS),
                Some(shares.as_slice()),
                "cell {cell}"
            );
        }
        assert_eq!(history.cell(700, 18), None);
        assert_eq!(history.cell(1, 0), None);
        assert!(History::decode(b"NBH0").is_err());
        let bytes = bake(&[1940, 1950], 2023, &cells);
        assert!(
            History::decode(&bytes[..bytes.len() - 3]).is_err(),
            "a cut stream is refused"
        );
    }

    #[test]
    fn a_cell_centre_reads_its_own_share_between_decades() {
        let history = History::decode(&bake(
            &[1940, 1950, 1960],
            2023,
            &[(index(600, 900), vec![51, 102, 204])],
        ))
        .unwrap();
        let (lat, lon) = centre(600, 900);
        let at = |year| history.stood_share(lat, lon, year).unwrap();
        assert!(
            (at(1900) - 0.2).abs() < 1e-6,
            "before the first decade, the first"
        );
        assert!((at(1940) - 0.2).abs() < 1e-6);
        assert!(
            (at(1945) - 0.3).abs() < 1e-6,
            "halfway between 1940 and 1950"
        );
        assert!((at(1960) - 0.8).abs() < 1e-6);
        assert!(
            at(1990) > 0.8 && at(1990) < 1.0,
            "toward 1 at the latest year"
        );
        assert_eq!(at(2023), 1.0);
        assert_eq!(at(2040), 1.0);
    }

    #[test]
    fn bilinear_over_the_neighbours_with_data() {
        // Two cells side by side in a row; the cells below have no data.
        let history = History::decode(&bake(
            &[1940],
            2023,
            &[(index(600, 900), vec![0]), (index(600, 901), vec![255])],
        ))
        .unwrap();
        let (lat, lon) = centre(600, 900);
        let step = 1.0 / 12.0;
        let quarter = history.stood_share(lat, lon + step / 4.0, 1940).unwrap();
        assert!(
            (quarter - 0.25).abs() < 1e-4,
            "a quarter of the way: {quarter}"
        );
        // A quarter of a cell south, still a quarter east: the no-data row below takes no weight, so the same share.
        let south = history
            .stood_share(lat - step / 4.0, lon + step / 4.0, 1940)
            .unwrap();
        assert!(
            (south - 0.25).abs() < 1e-4,
            "renormalised over the cells with data: {south}"
        );
        // Past the second cell's centre, toward a cell with no data: the second's share alone.
        let east = history.stood_share(lat, lon + step * 1.5, 1940).unwrap();
        assert!((east - 1.0).abs() < 1e-6);
        // Two cells away from both: no data.
        assert_eq!(history.stood_share(lat, lon + step * 3.0, 1940), None);
        assert_eq!(history.stood_share(lat + step * 2.0, lon, 1940), None);
        assert_eq!(history.stood_share(f64::NAN, lon, 1940), None);
    }

    #[test]
    fn longitude_wraps_and_the_poles_clamp() {
        let history = History::decode(&bake(
            &[1940],
            2023,
            &[(index(0, 0), vec![64]), (index(1000, 4319), vec![128])],
        ))
        .unwrap();
        let (lat, _) = centre(1000, 4319);
        // Just east of 180° W: between the last column (179.96° E) and the first; only the last has data.
        let share = history.stood_share(lat, -179.99, 1940).unwrap();
        assert!((share - 128.0 / 255.0).abs() < 1e-6);
        assert!(
            (history.stood_share(lat, 180.0 - 1.0 / 24.0, 1940).unwrap() - 128.0 / 255.0).abs()
                < 1e-6
        );
        assert!(
            (history
                .stood_share(90.0, -180.0 + 1.0 / 24.0, 1940)
                .unwrap()
                - 64.0 / 255.0)
                .abs()
                < 1e-6
        );
    }

    #[test]
    fn the_baked_history_decodes_with_the_decades_and_places() {
        let history = History::baked();
        assert_eq!(history.decades, [1900, 1910, 1920, 1930, 1940, 1950, 1960]);
        assert_eq!(history.latest, 2023);
        assert!(history.len() > 10_000, "{} cells", history.len());
        // Mid-Pacific and mid-Atlantic: nothing built up today.
        assert_eq!(stood_share(30.0, -140.0, 1942), None);
        assert_eq!(stood_share(40.0, -40.0, 1942), None);
        // The places the period lanes look at have data (but Yokosuka: HYDE's 2023 puts its urban people in
        // Yokohama's cells and none in its own), and today's year keeps all of it.
        for (name, lat, lon) in PLACES.into_iter().filter(|(name, ..)| *name != "Yokosuka") {
            let share = stood_share(lat, lon, 1942).unwrap_or_else(|| panic!("{name} has data"));
            assert!((0.0..=1.0).contains(&share), "{name}: {share}");
            assert_eq!(stood_share(lat, lon, 2023), Some(1.0), "{name}");
            assert!(
                stood_share(lat, lon, 1950).unwrap() >= stood_share(lat, lon, 1930).unwrap() - 0.05,
                "{name}"
            );
        }
        // As history has them: central Tokyo stood in 1942, Honiara hardly at all.
        assert!(stood_share(35.68, 139.76, 1942).unwrap() > 0.9);
        assert!(stood_share(-9.43, 159.95, 1942).unwrap() < 0.05);
    }

    /// The places the period buildings are judged at.
    const PLACES: [(&str, f64, f64); 10] = [
        ("Los Angeles, downtown", 34.05, -118.25),
        ("San Fernando Valley", 34.20, -118.45),
        ("Long Beach", 33.77, -118.19),
        ("Tokyo", 35.68, 139.76),
        ("Yokosuka", 35.28, 139.67),
        ("Reykjavik", 64.15, -21.94),
        ("Honiara", -9.43, 159.95),
        ("Portsmouth", 50.80, -1.09),
        ("Valletta", 35.90, 14.51),
        ("Honolulu", 21.31, -157.86),
    ];

    /// `bun run rust:test -- --native -p naval-terrain --lib history -- --nocapture --ignored` prints the places'
    /// shares.
    #[test]
    #[ignore = "a report, not a check"]
    fn report_the_places() {
        for (name, lat, lon) in PLACES {
            let shares: Vec<String> = [1900, 1930, 1940, 1942, 1950, 1960]
                .iter()
                .map(|&y| format!("{y} {:.2}", stood_share(lat, lon, y).unwrap_or(f32::NAN)))
                .collect();
            println!("{name:24} {}", shares.join("  "));
        }
    }
}
