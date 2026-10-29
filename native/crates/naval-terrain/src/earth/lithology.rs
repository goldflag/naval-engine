//! The rock of a place: GLiM's dominant lithological class per half degree (native/docs/terrain/ground.md, "Climate
//! and rock maps"), which chooses the cliffs' rock ([`crate::ground_materials::CliffRock::at`]).
//!
//! The data is Hartmann and Moosdorf's Global Lithological Map ([`ATTRIBUTION`]) as their 0.5° raster: 720 x 360
//! cells, each the class covering most of it, none where the sea does. `scripts/theatre/bake-climate-and-rock.py`
//! bakes it into `glim.ngl` (26 KB, compiled in), decoded once on the first lookup. Half a degree is 55 km: a coast's
//! own cell is often the sea's, so [`rock_near`] takes the nearest cell with rock.

use std::sync::OnceLock;

/// What the game must show where it credits its sources.
pub const ATTRIBUTION: &str = "Rock: GLiM, the Global Lithological Map, Hartmann, J. and Moosdorf, N. (2012), \
     Geochemistry, Geophysics, Geosystems 13, Q12004, doi:10.1029/2012GC004370; data doi:10.1594/PANGAEA.788537, \
     CC BY 3.0";

/// The baked map (`bake-climate-and-rock.py`'s docstring gives the layout).
const BAKED: &[u8] = include_bytes!("glim.ngl");
const MAGIC: &[u8; 4] = b"NGL1";
/// The grid's columns (half a degree of longitude each, from 180° W) and rows (of latitude, from 90° N).
pub const COLS: usize = 720;
pub const ROWS: usize = 360;
const CELLS_PER_DEGREE: f64 = 2.0;
/// How far [`rock_near`] looks for a cell with rock: two cells (a degree) each way.
pub const NEAREST_CELLS: isize = 2;

/// A GLiM class, numbered as the map numbers it (its `Classnames.txt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Lithology {
    /// su: alluvium, glacial and aeolian deposits, loess.
    UnconsolidatedSediments = 1,
    /// vb: basalt.
    BasicVolcanics,
    /// ss: sandstone, shale, conglomerate.
    SiliciclasticSediments,
    /// pb: gabbro.
    BasicPlutonics,
    /// sm: sedimentary sequences with carbonate and siliciclastic beds.
    MixedSediments,
    /// sc: limestone and dolomite.
    CarbonateSediments,
    /// va: rhyolite and dacite.
    AcidVolcanics,
    /// mt: gneiss, schist, quartzite, marble.
    Metamorphics,
    /// pa: granite.
    AcidPlutonics,
    /// vi: andesite.
    IntermediateVolcanics,
    /// wb: lakes and rivers.
    WaterBodies,
    /// py: tuff and ignimbrite.
    Pyroclastics,
    /// pi: diorite.
    IntermediatePlutonics,
    /// ev: gypsum and salt.
    Evaporites,
    /// nd: no data.
    NoData,
    /// ig: ice and glaciers.
    IceAndGlaciers,
}

impl Lithology {
    /// Every class, in order of its number (`ALL[k - 1]` is class k).
    pub const ALL: [Lithology; 16] = [
        Lithology::UnconsolidatedSediments,
        Lithology::BasicVolcanics,
        Lithology::SiliciclasticSediments,
        Lithology::BasicPlutonics,
        Lithology::MixedSediments,
        Lithology::CarbonateSediments,
        Lithology::AcidVolcanics,
        Lithology::Metamorphics,
        Lithology::AcidPlutonics,
        Lithology::IntermediateVolcanics,
        Lithology::WaterBodies,
        Lithology::Pyroclastics,
        Lithology::IntermediatePlutonics,
        Lithology::Evaporites,
        Lithology::NoData,
        Lithology::IceAndGlaciers,
    ];

    /// The class numbered `k` (1-16), none for no cell (0) or a number off the list.
    pub fn from_number(k: u8) -> Option<Lithology> {
        Self::ALL.get(usize::from(k).wrapping_sub(1)).copied()
    }

    pub fn number(self) -> u8 {
        self as u8
    }

    /// GLiM's two letters for it ("su", "sc", "mt").
    pub fn code(self) -> &'static str {
        const CODES: [&str; 16] = [
            "su", "vb", "ss", "pb", "sm", "sc", "va", "mt", "pa", "vi", "wb", "py", "pi", "ev",
            "nd", "ig",
        ];
        CODES[usize::from(self.number()) - 1]
    }

    /// Whether it says what the ground is made of: not water, not a gap in the data.
    pub fn is_ground(self) -> bool {
        !matches!(self, Lithology::WaterBodies | Lithology::NoData)
    }
}

/// The decoded map: a byte a cell (0 none, else the class), rows from 90° N.
pub struct LithologyMap {
    cells: Vec<u8>,
}

impl LithologyMap {
    /// The map from its baked bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        let body = bytes
            .strip_prefix(MAGIC)
            .ok_or("not a lithological map (no NGL1)")?;
        let cells = miniz_oxide::inflate::decompress_to_vec_zlib(body)
            .map_err(|e| format!("lithological map: {e:?}"))?;
        if cells.len() != COLS * ROWS {
            return Err(format!(
                "lithological map: {} cells, not {}",
                cells.len(),
                COLS * ROWS
            ));
        }
        if cells.iter().any(|&c| c > 16) {
            return Err("lithological map: a cell of no class".into());
        }
        Ok(Self { cells })
    }

    /// The game's map, decoded on first use.
    pub fn baked() -> &'static Self {
        static MAP: OnceLock<LithologyMap> = OnceLock::new();
        MAP.get_or_init(|| Self::decode(BAKED).expect("the baked lithological map decodes"))
    }

    /// The cell a point stands in (longitude wraps, latitude clamps), none off the earth (NaN).
    pub fn cell_of(lat: f64, lon: f64) -> Option<(usize, usize)> {
        if !lat.is_finite() || !lon.is_finite() {
            return None;
        }
        let row = ((90.0 - lat) * CELLS_PER_DEGREE)
            .floor()
            .clamp(0.0, (ROWS - 1) as f64) as usize;
        let col = ((lon + 180.0) * CELLS_PER_DEGREE)
            .floor()
            .rem_euclid(COLS as f64) as usize;
        Some((row, col.min(COLS - 1)))
    }

    /// Cell (`row`, `col`)'s class, none where the map has no cell (the sea).
    pub fn cell(&self, row: usize, col: usize) -> Option<Lithology> {
        Lithology::from_number(self.cells[row * COLS + col])
    }

    /// The class of the cell a point stands in, none at sea.
    pub fn at(&self, lat: f64, lon: f64) -> Option<Lithology> {
        let (row, col) = Self::cell_of(lat, lon)?;
        self.cell(row, col)
    }

    /// The ground's class at a point: its cell's where that says what the ground is made of
    /// ([`Lithology::is_ground`]), else the nearest such cell's middle within [`NEAREST_CELLS`] (the first of a tie,
    /// north to south and west to east); none past that, as on an ocean's islands.
    pub fn rock_near(&self, lat: f64, lon: f64) -> Option<Lithology> {
        let (row, col) = Self::cell_of(lat, lon)?;
        if let Some(rock) = self.cell(row, col).filter(|l| l.is_ground()) {
            return Some(rock);
        }
        let cos = lat.to_radians().cos().max(0.05);
        let mut best: Option<(f64, Lithology)> = None;
        for dr in -NEAREST_CELLS..=NEAREST_CELLS {
            let r = row as isize + dr;
            if !(0..ROWS as isize).contains(&r) {
                continue;
            }
            for dc in -NEAREST_CELLS..=NEAREST_CELLS {
                let c = (col as isize + dc).rem_euclid(COLS as isize) as usize;
                let Some(rock) = self.cell(r as usize, c).filter(|l| l.is_ground()) else {
                    continue;
                };
                let middle_lat = 90.0 - (r as f64 + 0.5) / CELLS_PER_DEGREE;
                let middle_lon = -180.0 + (col as f64 + dc as f64 + 0.5) / CELLS_PER_DEGREE;
                let d = (middle_lat - lat).powi(2) + ((middle_lon - lon) * cos).powi(2);
                if best.is_none_or(|(b, _)| d < b) {
                    best = Some((d, rock));
                }
            }
        }
        best.map(|(_, rock)| rock)
    }
}

/// GLiM's class at a point of the earth (its 0.5° cell's), none at sea.
pub fn lithology(lat: f64, lon: f64) -> Option<Lithology> {
    LithologyMap::baked().at(lat, lon)
}

/// The ground's class at a point, from the nearest cell that says ([`LithologyMap::rock_near`]).
pub fn rock_near(lat: f64, lon: f64) -> Option<Lithology> {
    LithologyMap::baked().rock_near(lat, lon)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bake(cells: &[(usize, usize, u8)]) -> Vec<u8> {
        let mut grid = vec![0u8; COLS * ROWS];
        for &(row, col, class) in cells {
            grid[row * COLS + col] = class;
        }
        let mut out = MAGIC.to_vec();
        out.extend_from_slice(&miniz_oxide::deflate::compress_to_vec_zlib(&grid, 6));
        out
    }

    #[test]
    fn decodes_what_the_bake_writes_and_finds_the_nearest_rock() {
        // 40.25° N 10.25° E's cell carbonate; one west of it water, one east of it metamorphic; the antimeridian's
        // first column basalt.
        let map = LithologyMap::decode(&bake(&[
            (99, 380, 6),
            (99, 379, 11),
            (99, 381, 8),
            (99, 0, 2),
        ]))
        .unwrap();
        assert_eq!(map.at(40.25, 10.25), Some(Lithology::CarbonateSediments));
        assert_eq!(map.at(40.25, 9.75), Some(Lithology::WaterBodies));
        assert_eq!(map.at(40.25, 12.0), None);
        assert_eq!(map.at(f64::NAN, 0.0), None);
        // On the water, the nearer of its two neighbours with rock; off the map's cells, likewise; too far, none.
        assert_eq!(
            map.rock_near(40.25, 9.6),
            Some(Lithology::CarbonateSediments)
        );
        assert_eq!(map.rock_near(40.25, 11.2), Some(Lithology::Metamorphics));
        assert_eq!(map.rock_near(40.25, 11.6), Some(Lithology::Metamorphics));
        assert_eq!(map.rock_near(40.25, 13.0), None);
        assert_eq!(map.rock_near(42.0, 10.25), None);
        // Across the antimeridian.
        assert_eq!(map.rock_near(40.25, 179.9), Some(Lithology::BasicVolcanics));
        assert!(LithologyMap::decode(b"NGL1").is_err());
        assert!(LithologyMap::decode(&bake(&[(0, 0, 17)])).is_err());
    }

    #[test]
    fn the_classes_know_their_letters() {
        for (k, class) in Lithology::ALL.iter().enumerate() {
            assert_eq!(usize::from(class.number()), k + 1);
            assert_eq!(Lithology::from_number(class.number()), Some(*class));
        }
        assert_eq!(Lithology::CarbonateSediments.code(), "sc");
        assert_eq!(Lithology::IceAndGlaciers.code(), "ig");
        assert_eq!(Lithology::from_number(0), None);
        assert!(!Lithology::WaterBodies.is_ground() && Lithology::IceAndGlaciers.is_ground());
    }

    #[test]
    fn the_baked_map_gives_each_place_its_rock() {
        use Lithology::*;
        for (name, lat, lon, rock) in [
            // The Eiger's cell: the Helvetic limestone (the cell west of it, Lauterbrunnen's, unconsolidated).
            ("Eiger", 46.58, 8.0, Some(CarbonateSediments)),
            ("Grand Canyon", 36.1, -112.1, Some(SiliciclasticSediments)),
            ("Geiranger", 62.1, 7.1, Some(Metamorphics)),
            ("Rinjani", -8.41, 116.46, Some(IntermediateVolcanics)),
            ("Guadalcanal", -9.43, 159.95, Some(BasicVolcanics)),
            // Mid-Pacific.
            ("30° N 140° W", 30.0, -140.0, None),
        ] {
            assert_eq!(lithology(lat, lon), rock, "{name}");
        }
        // Coasts whose own cell is the sea's take the nearest rock: Bergen the gneiss about it, Dover Kent's chalk.
        assert_eq!(lithology(60.39, 5.32), None);
        assert_eq!(rock_near(60.39, 5.32), Some(Metamorphics));
        assert_eq!(rock_near(51.13, 1.35), Some(CarbonateSediments));
    }

    /// `bun run rust:test -- --native -p naval-terrain --lib earth::lithology -- --nocapture --ignored` prints the
    /// class at the places the cliffs are judged at.
    #[test]
    #[ignore = "a report, not a check"]
    fn report_the_places() {
        for (name, lat, lon) in [
            ("Eiger", 46.5775, 8.0053),
            ("Mönch", 46.5584, 7.9975),
            ("Jungfrau", 46.5367, 7.9625),
            ("Lauterbrunnen", 46.59, 7.91),
            ("Grand Canyon", 36.1, -112.1),
            ("Geiranger", 62.1, 7.1),
            ("Taroko", 24.16, 121.6),
            ("Rinjani", -8.41, 116.46),
            ("Gibraltar", 36.14, -5.35),
            ("Big Sur", 36.3, -121.9),
            ("Busan", 35.1, 129.05),
            ("Palos Verdes", 33.75, -118.41),
        ] {
            println!(
                "{name:16} cell {:3} near {:3}",
                lithology(lat, lon).map_or("--", Lithology::code),
                rock_near(lat, lon).map_or("--", Lithology::code)
            );
        }
    }
}
