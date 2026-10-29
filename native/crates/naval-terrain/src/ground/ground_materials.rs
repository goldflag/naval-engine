//! The land's tiling ground materials (native/docs/terrain/ground.md, "Ground materials"): grass, dry grass, soil,
//! scrub, dry
//! scrub, forest floor, rock, cliff, sand, wet sand, shingle, snow, mud, paving and fine sand, CC0 photographs from Poly
//! Haven tiled under the land's own colour close up (`shaders/materials.wgsl`).
//!
//! Each material's sources are two JPEGs in [`MATERIAL_DIR`], made by `tools/ground_materials.py` from Poly Haven's
//! maps: its colour as a ratio to its own local mean (halved, linear: 128 is the mean; the forest floor's squeezed
//! toward it, [`GroundMaterial::squeeze`]), and its normal's x and y with its height. On the GPU each material is two
//! layers of one array texture: layer 2 k its colour ratio with its height, layer 2 k + 1 its normal (x, y, 0.5, 1).
//! [`GroundMaterials::from_sources`] builds the array as RGBA8 with every level (the stills, the tests and a checkout
//! without a pack); the asset pack carries it encoded as BC7 at 1024² (`naval-assets`, `Kind::GroundMaterials`). The
//! crate reads no file: the host hands the bytes in.

use serde::{Deserialize, Serialize};

use crate::earth::lithology::{Lithology, rock_near};
use crate::textures::mips_rgba8;

/// Where the materials' sources are, in the repository (`<name>-albedo.jpg`, `<name>-surface.jpg`).
pub const MATERIAL_DIR: &str = "public/terrain/materials";
/// The array's side when built from the sources for a device (`from_sources`), and when packed.
pub const MATERIAL_SIDE: u32 = 1024;
/// Room for the materials in the land's uniforms (`Terrain::material_table`); the last slot is `M_NONE`.
pub const MATERIAL_SLOTS: usize = 16;

/// One ground material: its name (its files' and the shader's `M_*`), its source, how much ground one repeat covers,
/// how strongly its normal tilts the land's and how much of its colour's contrast is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundMaterial {
    pub name: &'static str,
    /// The Poly Haven asset its maps come from (CC0: <https://polyhaven.com/license>).
    pub source: &'static str,
    pub period_m: f32,
    pub normal: f32,
    pub contrast: f32,
    /// How far its file's colour ratio is squeezed toward 1 (its departures from 1 over this) so a contrasty photograph
    /// fits the bytes: 1 for most; 2.5 for the forest floor's leaf litter, whose leaves and the dark gaps between them
    /// span a ratio of 0.1 to 2.7, and 2 for its needles (`tools/ground_materials.py`'s `LOOKS`). The land's uniforms
    /// multiply its contrast by it, so the shader reads the ratio back whole.
    pub squeeze: f32,
}

const fn material(
    name: &'static str,
    source: &'static str,
    period_m: f32,
    normal: f32,
    contrast: f32,
) -> GroundMaterial {
    GroundMaterial {
        name,
        source,
        period_m,
        normal,
        contrast,
        squeeze: 1.0,
    }
}

impl GroundMaterial {
    /// The material with its file's colour ratio squeezed toward 1 by `squeeze` ([`GroundMaterial::squeeze`]).
    const fn squeezed(self, squeeze: f32) -> GroundMaterial {
        GroundMaterial { squeeze, ..self }
    }

    /// Its contrast as the shader reads its file's ratio: its own, the squeeze multiplied back.
    pub fn read_contrast(&self) -> f32 {
        self.contrast * self.squeeze
    }
}

/// The materials, in the order of the array's layers and the shader's ids (`M_GRASS` 0 … `M_FINE_SAND` 14), then the
/// cliffs' regional rocks ([`CliffRock`]: limestone, basalt, granite) and the forest floor's needles, which the land's
/// weights never choose: its cliff (`M_CLIFF`) reads the rock its place gives (`terrain.cliff`), and its forest floor
/// (`M_FOREST`) the needles where its trees are conifers ([`needle_floor`]).
pub const GROUND_MATERIALS: [GroundMaterial; 19] = [
    material("grass", "aerial_grass_rock", 12.0, 1.0, 1.0),
    material("dry-grass", "aerial_ground_rock", 16.0, 1.0, 0.9),
    material("soil", "farm_soil", 6.0, 1.0, 0.9),
    material("scrub", "coast_sand_rocks_02", 15.0, 1.0, 0.9),
    material("dry-scrub", "aerial_rocks_01", 30.0, 1.0, 0.9),
    material("forest", "leaves_forest_ground", 2.0, 0.8, 0.8).squeezed(2.5),
    material("rock", "aerial_rocks_02", 24.0, 1.0, 0.8),
    material("cliff", "marble_cliff_05", 20.0, 1.0, 1.0),
    material("sand", "coast_sand_01", 15.0, 1.0, 0.9),
    material("wet-sand", "coast_sand_02", 15.0, 0.8, 0.8),
    material("shingle", "clean_pebbles", 5.0, 1.0, 1.0),
    material("snow", "snow_02", 10.0, 0.8, 0.6),
    material("mud", "brown_mud_02", 6.0, 1.0, 0.9),
    material("paving", "aerial_asphalt_01", 30.0, 0.6, 0.8),
    material("fine-sand", "sand_03", 3.0, 0.8, 0.8),
    // Photographed face on (12.6 m, 2.0 m and 2.0 m of rock), read on the walls with up kept up.
    material("limestone", "marble_cliff_04", 16.0, 1.0, 1.0),
    material("basalt", "dark_rock_02", 11.0, 1.0, 1.0),
    material("granite", "marble_rock_01", 13.0, 1.0, 1.0),
    // Photographed from above at 2.0 m, at its own size: needles a millimetre or two across, finer than the leaves.
    material("needles", "forrest_ground_03", 2.0, 0.8, 0.8).squeezed(2.0),
];

/// The forest floor's needles in [`GROUND_MATERIALS`].
pub const NEEDLES: usize = 18;

/// The needles' row of the land's uniforms (`bindings.wgsl`'s `Terrain::needles`): their layer in the array, period
/// (m), normal strength and contrast (as their file's ratio is read: [`GroundMaterial::read_contrast`]). The forest
/// floor reads them for the share of it under conifers (`surface.wgsl`'s `floor_at`, `materials.wgsl`'s `floor_read`).
pub fn needle_floor() -> [f32; 4] {
    let m = &GROUND_MATERIALS[NEEDLES];
    [NEEDLES as f32, m.period_m, m.normal, m.read_contrast()]
}

/// How many of [`GROUND_MATERIALS`] the land's weights choose among (`materials.wgsl`'s `M_*` and `Mats`' slots); the
/// rest are the cliffs' regional rocks.
pub const CHOSEN_MATERIALS: usize = 15;
// The chosen materials and `M_NONE` fit the uniforms' slots.
const _: () = assert!(CHOSEN_MATERIALS < MATERIAL_SLOTS);

/// The rock a land's cliffs and steep faces are made of (native/docs/terrain/ground.md, "Cliffs"): which material their
/// walls
/// read, the colour that stands in for the photograph on them, and how strongly they are bedded. Chosen by where the
/// land stands ([`CliffRock::at`]); a battle map, with no place, keeps its style's cliffs ([`CliffRock::Authored`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliffRock {
    /// No place: the style's own cliffs and colours, the cliff rock's (`marble_cliff_05`) structure.
    Authored,
    /// Chalk: the Channel's white cliffs (Dover, the Seven Sisters, Étretat), Rügen and Møn.
    Chalk,
    /// Pale limestone and dolomite: Gibraltar, the Aegean and the Adriatic's karst, the Levant, the Alps' Helvetic
    /// front (the Eiger), Normandy's Bessin; GLiM's carbonate sediments.
    Limestone,
    /// Malta's and Gozo's golden Globigerina and Coralline limestone.
    GoldenLimestone,
    /// Sandstone, shale and the glacial and alluvial clays and sands, buff and bedded: California's bluffs, the
    /// Baltic's shores, the Grand Canyon; GLiM's siliciclastic, mixed and unconsolidated sediments and evaporites.
    Sandstone,
    /// Old Red Sandstone: Orkney, Caithness and Shetland's red, bedded cliffs (Hoy, Duncansby).
    RedSandstone,
    /// Basalt and andesite, dark and layered by its flows: Hawaii, Iceland and the Faroes, Japan, the Azores and the
    /// Canaries, the Pacific's volcanic islands; GLiM's volcanic and pyroclastic rocks and gabbro.
    Basalt,
    /// Pale grey granite: Brittany and Cornwall, Corsica, Big Sur, the Sierra; GLiM's acid and intermediate plutonic
    /// rocks (and the rock under its ice).
    Granite,
    /// Dark grey gneiss and schist, unbedded: Norway's fjords, the Highlands, the Alps' crystalline massifs; GLiM's
    /// metamorphic rocks.
    Gneiss,
    /// Pale grey marble, faintly banded: Taroko's gorge and the Qingshui cliffs.
    Marble,
}

/// The rock GLiM's class gives the cliffs (native/docs/terrain/ground.md, "Cliffs"), none for water and its gaps.
fn rock_of(lithology: Lithology) -> Option<CliffRock> {
    use Lithology::*;
    Some(match lithology {
        CarbonateSediments => CliffRock::Limestone,
        SiliciclasticSediments | MixedSediments | UnconsolidatedSediments | Evaporites => {
            CliffRock::Sandstone
        }
        Metamorphics => CliffRock::Gneiss,
        AcidPlutonics | IntermediatePlutonics | IceAndGlaciers => CliffRock::Granite,
        BasicVolcanics | IntermediateVolcanics | AcidVolcanics | Pyroclastics | BasicPlutonics => {
            CliffRock::Basalt
        }
        WaterBodies | NoData => return None,
    })
}

/// What a [`CliffRock`] gives the land's walls: its material's index in [`GROUND_MATERIALS`], its colour (linear albedo,
/// as the calibrated photograph's: bare chalk and sand stand at 0.19 of luminance there), how strongly it is bedded
/// (0-1) and its beds' usual thickness (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CliffLook {
    pub material: usize,
    pub colour: [f32; 3],
    pub strata: f32,
    pub bed_m: f32,
}

/// A box of latitude and longitude.
fn within(lat: f32, lon: f32, lat0: f32, lat1: f32, lon0: f32, lon1: f32) -> bool {
    (lat0..=lat1).contains(&lat) && (lon0..=lon1).contains(&lon)
}

impl CliffRock {
    /// The rock of the cliffs at a point of the earth: a coast or a mountain chosen by hand where GLiM's half degree is
    /// too coarse for it ([`CliffRock::named`]), else the rock GLiM gives the place's cell or the nearest cell with
    /// rock (`earth::lithology::rock_near`), else, far out on an ocean, by latitude.
    pub fn at(lat: f32, lon: f32) -> CliffRock {
        if let Some(rock) = Self::named(lat, lon) {
            return rock;
        }
        if let Some(rock) = rock_near(f64::from(lat), f64::from(lon)).and_then(rock_of) {
            return rock;
        }
        // The oceans' islands, past GLiM's cells: in the tropics mostly volcanic, toward the poles the shield's
        // crystalline rock, elsewhere a bedded sedimentary rock.
        if lat.abs() < 24.0 {
            CliffRock::Basalt
        } else if lat.abs() > 55.0 {
            CliffRock::Granite
        } else {
            CliffRock::Sandstone
        }
    }

    /// The coasts and mountains whose rock is named by hand: a cliff a few kilometres long that a half-degree cell
    /// (55 km) takes for its neighbour's rock, or an island too small for a cell of its own.
    pub fn named(lat: f32, lon: f32) -> Option<CliffRock> {
        // Chalk: the Channel's English shore from the Needles to Dover and its French shore from Normandy to Cap
        // Blanc-Nez, Rügen and Møn.
        if within(lat, lon, 50.55, 51.4, -1.65, 1.6)
            || within(lat, lon, 49.35, 51.0, -0.25, 1.9)
            || within(lat, lon, 54.4, 55.1, 12.1, 13.9)
        {
            return Some(CliffRock::Chalk);
        }
        // Orkney, Caithness and Shetland: Old Red Sandstone.
        if within(lat, lon, 58.2, 60.95, -4.6, -0.6) {
            return Some(CliffRock::RedSandstone);
        }
        // Malta and Gozo.
        if within(lat, lon, 35.75, 36.12, 14.15, 14.62) {
            return Some(CliffRock::GoldenLimestone);
        }
        // Limestone: the Marianas' raised reefs (Guam's northern cliffs, Saipan, Tinian); the Rock of Gibraltar (its
        // cell is the sea's, the nearest Andalusia's mixed sediments); the Bernese Oberland's Helvetic front, from
        // Lauterbrunnen's walls and the Schilthorn to the Eiger's and the Mönch's north faces and the Wetterhorn
        // (GLiM puts the Eiger in a carbonate cell, the Mönch and Lauterbrunnen in the unconsolidated sediments of
        // Interlaken's lakes).
        if within(lat, lon, 13.2, 15.4, 144.5, 146.0)
            || within(lat, lon, 36.1, 36.16, -5.37, -5.33)
            || within(lat, lon, 46.53, 46.72, 7.75, 8.2)
        {
            return Some(CliffRock::Limestone);
        }
        // The Aar massif's gneiss and granite south of the Helvetic front: the Jungfrau's and the Finsteraarhorn's
        // crests, the Aletsch (whose cell GLiM gives the glaciers' moraines, unconsolidated).
        if within(lat, lon, 46.3, 46.53, 7.75, 8.45) {
            return Some(CliffRock::Gneiss);
        }
        // Taroko's gorge and the Qingshui cliffs: the Tananao complex's marble (the coast's cell is the sea's, the
        // nearest the west's sandstone).
        if within(lat, lon, 23.95, 24.35, 121.4, 121.75) {
            return Some(CliffRock::Marble);
        }
        // Granite: Corsica and Sardinia's north-east, Brittany and Cornwall.
        if within(lat, lon, 41.3, 43.1, 8.5, 9.65)
            || within(lat, lon, 47.2, 48.95, -5.2, -1.9)
            || within(lat, lon, 49.9, 50.75, -5.8, -4.2)
        {
            return Some(CliffRock::Granite);
        }
        // Basalt and andesite: Hawaii, Japan (and the Ryukyus), the Aleutians, Iceland and the Faroes, the Azores,
        // Madeira and the Canaries; the bay of Naples and Etna's coast.
        if within(lat, lon, 18.5, 22.6, -160.6, -154.5)
            || within(lat, lon, 24.0, 45.6, 122.8, 146.0)
                && !within(lat, lon, 33.0, 39.0, 124.0, 129.4)
                && !(lat > 38.5 && lon < 138.5)
            || within(lat, lon, 51.0, 60.0, -180.0, -150.0)
            || within(lat, lon, 61.3, 67.5, -25.0, -6.2)
            || within(lat, lon, 27.5, 40.0, -31.5, -13.0)
            || within(lat, lon, 40.55, 41.0, 13.85, 14.6)
            || within(lat, lon, 37.3, 38.0, 14.9, 15.35)
        {
            return Some(CliffRock::Basalt);
        }
        None
    }

    /// Its material, colour and bedding.
    pub fn look(self) -> CliffLook {
        let (name, colour, strata, bed_m) = match self {
            CliffRock::Authored => ("cliff", [0.0, 0.0, 0.0], 0.6, 3.0),
            CliffRock::Chalk => ("limestone", [0.42, 0.42, 0.39], 0.45, 2.0),
            CliffRock::Limestone => ("limestone", [0.28, 0.26, 0.21], 0.7, 3.0),
            CliffRock::GoldenLimestone => ("limestone", [0.29, 0.21, 0.115], 0.8, 3.5),
            CliffRock::Sandstone => ("cliff", [0.15, 0.12, 0.085], 0.8, 2.5),
            CliffRock::RedSandstone => ("cliff", [0.12, 0.065, 0.045], 1.0, 1.5),
            CliffRock::Basalt => ("basalt", [0.06, 0.055, 0.05], 0.55, 6.0),
            CliffRock::Granite => ("granite", [0.12, 0.115, 0.11], 0.0, 4.0),
            // Granite's structure at half its colour: Geiranger's walls stand near basalt's grey, a little warmer
            // (at 0.6 of granite's the faces drew only a tenth darker than granite's).
            CliffRock::Gneiss => ("granite", [0.06, 0.058, 0.054], 0.0, 4.0),
            // The marble cliff's own photograph, paler and greyer than limestone's buff, its banding faint.
            CliffRock::Marble => ("limestone", [0.26, 0.26, 0.25], 0.25, 6.0),
        };
        CliffLook {
            material: GROUND_MATERIALS
                .iter()
                .position(|m| m.name == name)
                .expect("a ground material"),
            colour,
            strata,
            bed_m,
        }
    }

    /// Every rock, in the order of their ids in the land's grid (`cliff_grid`) and rows (`cliff_rocks`).
    pub const ALL: [CliffRock; CLIFF_ROCKS] = [
        CliffRock::Authored,
        CliffRock::Chalk,
        CliffRock::Limestone,
        CliffRock::GoldenLimestone,
        CliffRock::Sandstone,
        CliffRock::RedSandstone,
        CliffRock::Basalt,
        CliffRock::Granite,
        CliffRock::Gneiss,
        CliffRock::Marble,
    ];
}

/// How many rocks the land tells apart ([`CliffRock::ALL`]), and their rows in the land's uniforms (`bindings.wgsl`'s
/// `cliff_rocks` array holds `CLIFF_ROCK_ROWS`; the grid's four bits a cell, sixteen).
pub const CLIFF_ROCKS: usize = 10;
const _: () = assert!(CLIFF_ROCKS <= 16);
pub const CLIFF_ROCK_ROWS: usize = CLIFF_ROCKS * 3;
/// The cliffs' rock grid about the eye: cells a side, a cell (m). 128 km a side: as far as a cliff's colour shows.
pub const CLIFF_CELLS: usize = 32;
pub const CLIFF_CELL_M: f32 = 4000.0;
/// The grid's words: four bits a cell.
pub const CLIFF_GRID_WORDS: usize = CLIFF_CELLS * CLIFF_CELLS / 32;

/// The rocks' rows of the land's uniforms (`bindings.wgsl`'s `Terrain::cliff_rocks`), three per rock in the order of
/// [`CliffRock::ALL`]: its material (its layer in the array, period, normal strength, contrast); its colour, and 1
/// where it stands in for the photograph's and the style's (0 for [`CliffRock::Authored`]); its bedding and its beds'
/// thickness (m).
pub fn cliff_rocks() -> [[f32; 4]; CLIFF_ROCK_ROWS] {
    let mut rows = [[0.0; 4]; CLIFF_ROCK_ROWS];
    for (k, rock) in CliffRock::ALL.iter().enumerate() {
        let look = rock.look();
        let m = &GROUND_MATERIALS[look.material];
        let own = if *rock == CliffRock::Authored {
            0.0
        } else {
            1.0
        };
        rows[k * 3] = [
            look.material as f32,
            m.period_m,
            m.normal,
            m.read_contrast(),
        ];
        rows[k * 3 + 1] = [look.colour[0], look.colour[1], look.colour[2], own];
        rows[k * 3 + 2] = [look.strata, look.bed_m, 0.0, 0.0];
    }
    rows
}

/// The cliffs' rock of a square of [`CLIFF_CELLS`]² cells of [`CLIFF_CELL_M`] from cell `corner` of the chart (cell
/// (i, j) spans chart x from (corner.x + i) cells, z likewise), each the rock at its middle (`CliffRock::at`), the
/// chart's origin standing at `place`: four bits a cell, eight to a word, cell k = j * CLIFF_CELLS + i in word k / 8 at
/// bits 4 (k mod 8).
pub fn cliff_grid(place: [f32; 2], corner: [i32; 2]) -> [[u32; 4]; CLIFF_GRID_WORDS] {
    let mut words = [[0u32; 4]; CLIFF_GRID_WORDS];
    for j in 0..CLIFF_CELLS {
        for i in 0..CLIFF_CELLS {
            let middle = [
                (corner[0] as f32 + i as f32 + 0.5) * CLIFF_CELL_M,
                (corner[1] as f32 + j as f32 + 0.5) * CLIFF_CELL_M,
            ];
            let [lat, lon] = crate::species::chart_place(place, middle);
            let rock = CliffRock::ALL
                .iter()
                .position(|r| *r == CliffRock::at(lat, lon))
                .expect("a rock") as u32;
            let k = j * CLIFF_CELLS + i;
            words[k / 32][(k / 8) % 4] |= rock << (4 * (k % 8));
        }
    }
    words
}

/// Every source file, in order: each material's colour, then its surface.
pub fn material_files() -> Vec<String> {
    GROUND_MATERIALS
        .iter()
        .flat_map(|m| {
            [
                format!("{MATERIAL_DIR}/{}-albedo.jpg", m.name),
                format!("{MATERIAL_DIR}/{}-surface.jpg", m.name),
            ]
        })
        .collect()
}

/// The materials' row of the land's uniforms: per material the land's weights choose among (`CHOSEN_MATERIALS`) its
/// period (m), normal strength, colour contrast (as its file's ratio is read: [`GroundMaterial::read_contrast`]), 0.
pub fn material_table() -> [[f32; 4]; MATERIAL_SLOTS] {
    let mut table = [[1.0, 0.0, 0.0, 0.0]; MATERIAL_SLOTS];
    for (row, m) in table.iter_mut().zip(&GROUND_MATERIALS[..CHOSEN_MATERIALS]) {
        *row = [m.period_m, m.normal, m.read_contrast(), 0.0];
    }
    table
}

/// How the array's texels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MaterialTexels {
    /// RGBA8, linear (`Rgba8Unorm`).
    Rgba8,
    /// BC7, linear, 16 bytes a 4 × 4 block (`Bc7RgbaUnorm`; needs `TEXTURE_COMPRESSION_BC`). A level smaller than a
    /// block is one block.
    Bc7,
}

impl MaterialTexels {
    pub fn format(self) -> wgpu::TextureFormat {
        match self {
            MaterialTexels::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            MaterialTexels::Bc7 => wgpu::TextureFormat::Bc7RgbaUnorm,
        }
    }

    /// The bytes of one layer of a level of `side` texels.
    pub fn layer_bytes(self, side: u32) -> usize {
        match self {
            MaterialTexels::Rgba8 => side as usize * side as usize * 4,
            MaterialTexels::Bc7 => (side.div_ceil(4) as usize).pow(2) * 16,
        }
    }
}

/// The materials' array, GPU-ready: every level, each level's layers one after another.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GroundMaterials {
    pub texels: MaterialTexels,
    pub side: u32,
    pub layers: u32,
    /// Each level a byte string (postcard copies it whole rather than visiting every byte).
    #[serde(with = "byte_strings")]
    pub levels: Vec<Vec<u8>>,
}

/// `Vec<Vec<u8>>` as a sequence of byte strings.
mod byte_strings {
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serialize, Serializer};

    struct Bytes<'a>(&'a [u8]);

    impl Serialize for Bytes<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_bytes(self.0)
        }
    }

    pub fn serialize<S: Serializer>(levels: &[Vec<u8>], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(levels.iter().map(|l| Bytes(l)))
    }

    struct Level(Vec<u8>);

    impl<'de> serde::Deserialize<'de> for Level {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct LevelVisitor;
            impl<'de> Visitor<'de> for LevelVisitor {
                type Value = Level;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("a byte string")
                }
                fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Level, E> {
                    Ok(Level(bytes.to_vec()))
                }
                fn visit_byte_buf<E: serde::de::Error>(self, bytes: Vec<u8>) -> Result<Level, E> {
                    Ok(Level(bytes))
                }
                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Level, A::Error> {
                    let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                    while let Some(b) = seq.next_element::<u8>()? {
                        bytes.push(b);
                    }
                    Ok(Level(bytes))
                }
            }
            deserializer.deserialize_bytes(LevelVisitor)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Vec<u8>>, D::Error> {
        let levels: Vec<Level> = serde::Deserialize::deserialize(deserializer)?;
        Ok(levels.into_iter().map(|l| l.0).collect())
    }
}

impl GroundMaterials {
    /// The array's layers: two a material.
    pub const LAYERS: u32 = 2 * GROUND_MATERIALS.len() as u32;

    /// Level `level`'s side in texels (at least one).
    pub fn level_side(&self, level: usize) -> u32 {
        (self.side >> level).max(1)
    }

    /// Whether every level holds what its size says, down to one texel.
    pub fn is_consistent(&self) -> bool {
        self.layers >= 1
            && self.side >= 1
            && self.levels.len() as u32 == crate::textures::mip_count(self.side, self.side)
            && self.levels.iter().enumerate().all(|(level, bytes)| {
                bytes.len()
                    == self.texels.layer_bytes(self.level_side(level)) * self.layers as usize
            })
    }

    /// Bytes on the GPU.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// Every layer's base level at `side` texels, RGBA8, from the sources (the encoded bytes of [`material_files`], in
    /// order): material k's colour ratio and height at 2 k (each channel of the ratio scaled so its mean is exactly 128,
    /// whatever the JPEG and the resampling moved), its normal at 2 k + 1.
    pub fn base_layers(files: &[&[u8]], side: u32) -> Result<Vec<Vec<u8>>, String> {
        if files.len() != 2 * GROUND_MATERIALS.len() {
            return Err(format!(
                "ground materials: {} files, {} wanted",
                files.len(),
                2 * GROUND_MATERIALS.len()
            ));
        }
        let decode = |bytes: &[u8], what: &str| -> Result<image::RgbImage, String> {
            let image = image::load_from_memory(bytes)
                .map_err(|e| format!("ground material {what}: {e}"))?
                .to_rgb8();
            Ok(if image.width() == side && image.height() == side {
                image
            } else {
                image::imageops::resize(&image, side, side, image::imageops::FilterType::Triangle)
            })
        };
        let mut layers = Vec::with_capacity(files.len());
        for (k, m) in GROUND_MATERIALS.iter().enumerate() {
            let albedo = decode(files[2 * k], &format!("{} colour", m.name))?;
            let surface = decode(files[2 * k + 1], &format!("{} surface", m.name))?;
            let mut sums = [0f64; 3];
            for p in albedo.pixels() {
                for c in 0..3 {
                    sums[c] += f64::from(p[c]);
                }
            }
            let texels = f64::from(side) * f64::from(side);
            let gain = sums.map(|s| 127.5 / (s / texels).max(1.0));
            let mut colour = Vec::with_capacity((side * side * 4) as usize);
            let mut normal = Vec::with_capacity((side * side * 4) as usize);
            for (a, s) in albedo.pixels().zip(surface.pixels()) {
                for c in 0..3 {
                    colour.push((f64::from(a[c]) * gain[c]).round().clamp(0.0, 255.0) as u8);
                }
                colour.push(s[2]);
                normal.extend_from_slice(&[s[0], s[1], 128, 255]);
            }
            layers.push(colour);
            layers.push(normal);
        }
        Ok(layers)
    }

    /// The array as RGBA8 from every layer's base level (`base_layers`), its levels box-filtered in the stored values
    /// (the ratio and height are linear; the normals shorten as they average, so a distant material is flatter).
    pub fn from_base_layers(layers: &[Vec<u8>], side: u32) -> Self {
        let chains: Vec<Vec<Vec<u8>>> = layers.iter().map(|l| mips_rgba8(l, side, side)).collect();
        let count = crate::textures::mip_count(side, side) as usize;
        let levels = (0..count)
            .map(|level| {
                chains
                    .iter()
                    .flat_map(|c| c[level].iter().copied())
                    .collect()
            })
            .collect();
        Self {
            texels: MaterialTexels::Rgba8,
            side,
            layers: layers.len() as u32,
            levels,
        }
    }

    /// The array as RGBA8 at `side` texels from the sources (the encoded bytes of [`material_files`], in order).
    pub fn from_sources(files: &[&[u8]], side: u32) -> Result<Self, String> {
        Ok(Self::from_base_layers(
            &Self::base_layers(files, side)?,
            side,
        ))
    }

    /// A 1 × 1 array of one neutral layer (the mean ratio, mid height, a flat normal): what the land binds with no
    /// materials (their uniform is off then, so it is never read).
    pub fn placeholder() -> Self {
        Self {
            texels: MaterialTexels::Rgba8,
            side: 1,
            layers: 1,
            levels: vec![vec![128, 128, 128, 255]],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_names_the_materials_in_their_order() {
        let shader = include_str!("../../shaders/materials.wgsl");
        for (k, m) in GROUND_MATERIALS[..CHOSEN_MATERIALS].iter().enumerate() {
            let constant = format!(
                "const M_{}: u32 = {k}u;",
                m.name.to_uppercase().replace('-', "_")
            );
            assert!(shader.contains(&constant), "{constant}");
        }
        assert!(shader.contains(&format!("const M_NONE: u32 = {}u;", MATERIAL_SLOTS - 1)));
        // The cliffs' rocks are read through `M_CLIFF` alone, the needles through `M_FOREST`.
        for m in &GROUND_MATERIALS[CHOSEN_MATERIALS..] {
            let name = format!("const M_{}:", m.name.to_uppercase().replace('-', "_"));
            assert!(!shader.contains(&name), "{name}");
        }
    }

    #[test]
    fn the_forest_floor_reads_the_needles_from_their_row() {
        assert_eq!(GROUND_MATERIALS[NEEDLES].name, "needles");
        const { assert!(NEEDLES >= CHOSEN_MATERIALS) };
        let m = &GROUND_MATERIALS[NEEDLES];
        assert_eq!(
            needle_floor(),
            [NEEDLES as f32, m.period_m, m.normal, m.read_contrast()]
        );
        let shader = include_str!("../../shaders/materials.wgsl");
        assert!(shader.contains("i32(terrain.needles.x + 0.5)"));
    }

    #[test]
    fn each_coast_has_its_rock() {
        for (place, lat, lon, rock) in [
            // Named by hand.
            ("Dover", 51.13, 1.35, CliffRock::Chalk),
            ("Beachy Head", 50.735, 0.24, CliffRock::Chalk),
            ("Étretat", 49.71, 0.2, CliffRock::Chalk),
            ("Dingli, Malta", 35.86, 14.38, CliffRock::GoldenLimestone),
            ("Gibraltar", 36.14, -5.35, CliffRock::Limestone),
            ("Guam", 13.45, 144.75, CliffRock::Limestone),
            ("Diamond Head", 21.26, -157.8, CliffRock::Basalt),
            ("Jōgasaki, Izu", 34.9, 139.14, CliffRock::Basalt),
            ("Hoy", 58.88, -3.43, CliffRock::RedSandstone),
            ("Land's End", 50.07, -5.71, CliffRock::Granite),
            ("the Eiger", 46.5775, 8.0053, CliffRock::Limestone),
            ("the Mönch's north face", 46.565, 7.99, CliffRock::Limestone),
            ("Lauterbrunnen's walls", 46.59, 7.91, CliffRock::Limestone),
            ("the Aletsch", 46.45, 8.06, CliffRock::Gneiss),
            ("Taroko", 24.16, 121.6, CliffRock::Marble),
            ("the Qingshui cliffs", 24.2, 121.68, CliffRock::Marble),
            // From GLiM: its cell's class, or the nearest cell's where the coast's own is the sea's.
            (
                "Palos Verdes (unconsolidated)",
                33.75,
                -118.41,
                CliffRock::Sandstone,
            ),
            (
                "the Grand Canyon (siliciclastic)",
                36.1,
                -112.1,
                CliffRock::Sandstone,
            ),
            // Was sandstone by its California box; the Salinian block's granodiorite.
            (
                "Big Sur (intermediate plutonic)",
                36.3,
                -121.9,
                CliffRock::Granite,
            ),
            // Was granite by the Scandinavian box; the Bergen Arcs' and the fjords' gneiss.
            (
                "Bergen (metamorphic, nearest)",
                60.39,
                5.32,
                CliffRock::Gneiss,
            ),
            ("Geiranger (metamorphic)", 62.1, 7.1, CliffRock::Gneiss),
            // Was granite by the Korean box; the Cretaceous Yucheon volcanics about the harbour.
            ("Busan (acid volcanic)", 35.1, 129.05, CliffRock::Basalt),
            (
                "Pointe du Hoc (carbonate)",
                49.4,
                -0.99,
                CliffRock::Limestone,
            ),
            (
                "Rinjani (intermediate volcanic)",
                -8.41,
                116.46,
                CliffRock::Basalt,
            ),
            (
                "Guadalcanal (basic volcanic)",
                -9.43,
                159.95,
                CliffRock::Basalt,
            ),
            // Past GLiM's cells: the latitude's.
            ("Midway", 28.21, -177.37, CliffRock::Sandstone),
            ("Tarawa", 1.42, 173.0, CliffRock::Basalt),
        ] {
            assert_eq!(CliffRock::at(lat, lon), rock, "{place}");
        }
        // Every class of rock GLiM names gives a rock; water and its gaps give none (the nearest cell's is taken).
        for lithology in Lithology::ALL {
            assert_eq!(
                rock_of(lithology).is_none(),
                !lithology.is_ground(),
                "{lithology:?}"
            );
        }
        // The gneiss is granite's structure, clearly darker, unbedded.
        let (gneiss, granite) = (CliffRock::Gneiss.look(), CliffRock::Granite.look());
        assert_eq!(gneiss.material, granite.material);
        assert!(
            gneiss
                .colour
                .iter()
                .zip(granite.colour)
                .all(|(a, b)| *a < 0.7 * b)
        );
        assert_eq!(gneiss.strata, 0.0);
    }

    #[test]
    fn the_grid_holds_each_cell_rock_where_the_shader_reads_it() {
        let rows = cliff_rocks();
        let id = |rock: CliffRock| CliffRock::ALL.iter().position(|r| *r == rock).unwrap() as u32;
        // Only the authored rock keeps the style's colours; every rock's material is in the array.
        for (k, rock) in CliffRock::ALL.iter().enumerate() {
            assert_eq!(rows[k * 3 + 1][3] == 0.0, *rock == CliffRock::Authored);
            assert!((rows[k * 3][0] as usize) < GROUND_MATERIALS.len());
        }
        // The shader's uniforms hold every rock's rows.
        let bindings = include_str!("../../shaders/bindings.wgsl");
        let array = format!("cliff_rocks: array<vec4<f32>, {CLIFF_ROCK_ROWS}>,");
        assert!(bindings.contains(&array), "{array}");
        // Dover's chart origin: the cell about it chalk, the one a cell south of it too; 60 km west, Sussex's chalk.
        let half = (CLIFF_CELLS / 2) as i32;
        let words = cliff_grid([51.13, 1.35], [-half, -half]);
        let at = |i: usize, j: usize| {
            let k = j * CLIFF_CELLS + i;
            (words[k / 32][(k / 8) % 4] >> (4 * (k % 8))) & 15
        };
        assert_eq!(at(16, 16), id(CliffRock::Chalk));
        assert_eq!(at(16, 17), id(CliffRock::Chalk));
        // 62 km north, Essex's coast: its cell the sea's in GLiM, the nearest the London Basin's siliciclastic
        // sediments.
        assert_eq!(at(16, 0), id(CliffRock::Sandstone));
        // Geiranger's: the gneiss, at the last id but one.
        let words = cliff_grid([62.1, 7.1], [-half, -half]);
        let k = 16 * CLIFF_CELLS + 16;
        assert_eq!(
            (words[k / 32][(k / 8) % 4] >> (4 * (k % 8))) & 15,
            id(CliffRock::Gneiss)
        );
    }

    #[test]
    fn the_sources_build_a_consistent_array_with_its_ratio_at_its_mean() {
        let Some(root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .map(std::path::Path::to_path_buf)
        else {
            return;
        };
        let files: Vec<Vec<u8>> = material_files()
            .iter()
            .map(|f| std::fs::read(root.join(f)).unwrap_or_else(|e| panic!("{f}: {e}")))
            .collect();
        let refs: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
        let materials = GroundMaterials::from_sources(&refs, 64).unwrap();
        assert!(materials.is_consistent());
        assert_eq!(materials.layers, GroundMaterials::LAYERS);
        // The last level is each layer's mean: the colour ratio at 128 (within the rounding), the normal nearly flat.
        let last = materials.levels.last().unwrap();
        for (k, m) in GROUND_MATERIALS.iter().enumerate() {
            let colour = &last[8 * k..8 * k + 4];
            let normal = &last[8 * k + 4..8 * k + 8];
            for c in 0..3 {
                assert!(colour[c].abs_diff(128) <= 2, "{}: {colour:?}", m.name);
            }
            for c in 0..2 {
                assert!(normal[c].abs_diff(128) <= 24, "{}: {normal:?}", m.name);
            }
        }
    }
}
