//! Real-world surface maps for a land window: satellite colour and land cover on a square grid over the chart, which
//! the land shades from in place of (or over) its procedural cover where they hold data (`TerrainLayer::set_surface_maps`,
//! native/docs/terrain/surface.md). A theatre window's come from the streamed sources
//! (`naval-client` `theatre::surface`); a battle map has none.

/// ESA WorldCover 2021 (v200) classes, as the maps store them (one byte a texel; 0 where nothing is known).
pub mod cover {
    pub const UNKNOWN: u8 = 0;
    pub const TREE: u8 = 10;
    pub const SHRUB: u8 = 20;
    pub const GRASS: u8 = 30;
    pub const CROP: u8 = 40;
    pub const BUILT: u8 = 50;
    pub const BARE: u8 = 60;
    pub const SNOW: u8 = 70;
    pub const WATER: u8 = 80;
    pub const WETLAND: u8 = 90;
    pub const MANGROVE: u8 = 95;
    pub const MOSS: u8 = 100;

    /// Set over a texel's class where today's land there did not stand built up in a past period
    /// (native/docs/terrain/period.md,
    /// "The period's ground"): the host marks the built-up land the period leaves empty, and the land draws it as the
    /// period's country. The class beneath ([`today`]) is the one the texel holds today. Never set for `Period::Today`.
    pub const PAST_COUNTRY: u8 = 0x80;

    /// The class a texel holds today, its period's mark taken off.
    pub const fn today(class: u8) -> u8 {
        class & !PAST_COUNTRY
    }

    /// Whether the texel's land was country in the period drawn ([`PAST_COUNTRY`]).
    pub const fn is_past_country(class: u8) -> bool {
        class & PAST_COUNTRY != 0
    }
}

/// One window's surface maps: `size`² texels over the square of `extent_m` metres whose corner is chart point `origin`
/// (texel (i, j) covers chart x from `origin[0] + i * extent_m / size`, z likewise; row j = 0 at the smallest z).
#[derive(Clone, Debug)]
pub struct SurfaceMaps {
    pub origin: [f64; 2],
    pub extent_m: f64,
    pub size: u32,
    /// Satellite colour, sRGB RGBA8, `size * size * 4` bytes; alpha 255 where imagery covers the texel, 0 where none.
    pub albedo: Vec<u8>,
    /// Land cover, a [`cover`] class a texel, `size * size` bytes.
    pub cover: Vec<u8>,
}

impl SurfaceMaps {
    /// Whether the buffers hold what `size` says.
    pub fn is_consistent(&self) -> bool {
        let n = self.size as usize * self.size as usize;
        self.size > 0 && self.extent_m > 0.0 && self.albedo.len() == n * 4 && self.cover.len() == n
    }

    /// What the land's shading needs to know of the window's imagery as a whole ([`SurfaceStatistics`]), from every
    /// other texel of every other row.
    pub fn statistics(&self) -> SurfaceStatistics {
        self.statistics_with(&self.cover)
    }

    /// [`Self::statistics`] with `classes` (`size`² of them) in place of the maps' cover: a past period's marks over it
    /// ([`cover::PAST_COUNTRY`], `TerrainLayer::set_surface_maps_marked`).
    pub fn statistics_with(&self, classes: &[u8]) -> SurfaceStatistics {
        assert_eq!(classes.len(), self.cover.len(), "a class a texel");
        let decode: Vec<f32> = (0..256)
            .map(|i| {
                let c = i as f32 / 255.0;
                if c < 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            })
            .collect();
        let n = self.size as usize;
        let texels = || {
            (0..n)
                .step_by(2)
                .flat_map(move |j| (0..n).step_by(2).map(move |i| j * n + i))
        };
        // The haze: the darkest land's colour per channel (its first percentile), less what the darkest land of a
        // clear, well-corrected mosaic holds.
        let mut histograms = [[0u64; 256]; 3];
        let mut counted = 0u64;
        for land_only in [true, false] {
            for k in texels() {
                let class = cover::today(classes[k]);
                let land = !matches!(class, cover::UNKNOWN | cover::WATER | cover::SNOW);
                if self.albedo[k * 4 + 3] == 255 && (land || !land_only) {
                    for (c, histogram) in histograms.iter_mut().enumerate() {
                        histogram[self.albedo[k * 4 + c] as usize] += 1;
                    }
                    counted += 1;
                }
            }
            if counted > 0 {
                break;
            }
        }
        let mut haze = [0f32; 3];
        if counted > 0 {
            for (c, histogram) in histograms.iter().enumerate() {
                let mut seen = 0u64;
                let byte = histogram
                    .iter()
                    .position(|&count| {
                        seen += count;
                        seen * 100 >= counted
                    })
                    .unwrap_or(0);
                haze[c] = (decode[byte] - DARK_LAND[c]).clamp(0.0, HAZE_MAX);
            }
        }
        // Each class's mean colour, the haze taken out, leaving out what is far brighter than vegetation can be (a
        // cloud the mosaic kept) in the green classes, and the land a past period leaves as country (its photograph is
        // today's town).
        let mut sums = [[0f64; 4]; COVER_CLASSES];
        for k in texels() {
            if self.albedo[k * 4 + 3] != 255 || cover::is_past_country(classes[k]) {
                continue;
            }
            let class = classes[k];
            let rgb =
                [0, 1, 2].map(|c| (decode[self.albedo[k * 4 + c] as usize] - haze[c]).max(0.0));
            let green = matches!(
                class,
                cover::TREE
                    | cover::SHRUB
                    | cover::GRASS
                    | cover::CROP
                    | cover::WETLAND
                    | cover::MANGROVE
                    | cover::MOSS
            );
            let luminance = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            if green && luminance > 0.25 {
                continue;
            }
            let sum = &mut sums[cover_index(class)];
            for c in 0..3 {
                sum[c] += f64::from(rgb[c]);
            }
            sum[3] += 1.0;
        }
        let class_means = sums.map(|[r, g, b, count]| {
            if count > 0.0 {
                [
                    (r / count) as f32,
                    (g / count) as f32,
                    (b / count) as f32,
                    1.0,
                ]
            } else {
                [0.0; 4]
            }
        });
        SurfaceStatistics { haze, class_means }
    }
}

/// Texels on a side of a near tile ([`NearTile`]).
pub const NEAR_TILE_TEXELS: u32 = 512;

/// Mip levels of a near tile (512² down to 1²), and of the layer's surface textures, which hold the window's maps and
/// the near tiles side by side.
pub const NEAR_LEVELS: u32 = 10;

/// Mip levels of a near tile in BC7, and of the surface textures then: 512² down to 4², the smallest a slot's square of
/// whole 4 × 4 blocks (the textures' smallest level is a slot's 4² a side: 32² for a sea window's 4096²).
pub const NEAR_BC7_LEVELS: u32 = 8;

/// One square of the near surface: a finer patch of the same sources about the camera (a theatre's 8 m tiles, where
/// the window's maps are 23 m or 117 m a texel), [`NEAR_TILE_TEXELS`]² texels, made off the frame loop's thread with
/// its colour's mip chain already built, so the land layer only uploads it (`TerrainLayer::queue_near_tile`).
#[derive(Clone, Debug)]
pub struct NearTile {
    /// The colour's [`NEAR_LEVELS`] levels, sRGB RGBA8 premultiplied by the coverage in linear light (as the window's
    /// maps are uploaded), level 0 first; with `bc7`, its first [`NEAR_BC7_LEVELS`] in BC7 (block rows top to bottom).
    pub levels: Vec<Vec<u8>>,
    /// Whether `levels` are BC7 ([`NearTile::from_maps_bc7`]).
    pub bc7: bool,
    /// Land cover, a [`cover`] class a texel, `NEAR_TILE_TEXELS²` bytes.
    pub cover: Vec<u8>,
    /// The host's number for the marks over `cover` ([`cover::PAST_COUNTRY`]; 0 none), which the land keeps with the tile
    /// (`TerrainLayer::near_tile_marks`) so the host knows when to mark it again.
    pub marks: u64,
}

impl NearTile {
    /// The tile of `maps`, which must be [`NEAR_TILE_TEXELS`]² texels.
    pub fn from_maps(maps: &SurfaceMaps) -> Result<Self, String> {
        if !maps.is_consistent() || maps.size != NEAR_TILE_TEXELS {
            return Err(format!(
                "a near tile is {NEAR_TILE_TEXELS}² texels, not {}²",
                maps.size
            ));
        }
        let mut levels =
            crate::textures::mips_srgb8_premultiplied(&maps.albedo, maps.size, maps.size);
        levels.truncate(NEAR_LEVELS as usize);
        Ok(Self {
            levels,
            bc7: false,
            cover: maps.cover.clone(),
            marks: 0,
        })
    }

    /// As [`NearTile::from_maps`], its colour in BC7 (a quarter of the bytes, made here on the caller's thread: about
    /// 15 ms of encoding on four threads), for a land whose device samples it (`crate::bc7::supported`). The land
    /// encodes a tile made by [`NearTile::from_maps`] as it uploads it.
    pub fn from_maps_bc7(maps: &SurfaceMaps) -> Result<Self, String> {
        let mut tile = Self::from_maps(maps)?;
        if cfg!(target_arch = "wasm32") {
            return Ok(tile);
        }
        tile.levels.truncate(NEAR_BC7_LEVELS as usize);
        for (level, bytes) in tile.levels.iter_mut().enumerate() {
            let side = NEAR_TILE_TEXELS >> level;
            *bytes = crate::bc7::encode(bytes, side, side);
        }
        tile.bc7 = true;
        Ok(tile)
    }

    /// Bytes held (what the upload writes).
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum::<usize>() + self.cover.len()
    }
}

/// Which cover a [`CoverPatch`] rewrites: the window's maps (the cover array's layer 0), or near tile `index` (its slot
/// of layer 1, while the slot holds it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverLayer {
    Window,
    Near([i32; 2]),
}

/// New classes for a rectangle of a land layer's cover (`TerrainLayer::queue_cover_patch`): a past period's marks laid
/// again where what the host knows of the period's ground has changed (native/docs/terrain/period.md, "The period's
/// ground"), without handing in the maps or the tile again.
#[derive(Clone, Debug)]
pub struct CoverPatch {
    pub layer: CoverLayer,
    /// The rectangle's first texel (column, row) and its size (columns, rows), in the window's maps or the near tile.
    pub at: [u32; 2],
    pub size: [u32; 2],
    /// Its classes, row by row.
    pub classes: Vec<u8>,
    /// The host's number for the marks: a near tile's ([`NearTile::marks`]) once it is written.
    pub marks: u64,
}

impl CoverPatch {
    /// Whether the classes fill the rectangle and it lies within `side` texels.
    pub fn fits(&self, side: u32) -> bool {
        let [w, h] = self.size;
        w > 0
            && h > 0
            && self.at[0].checked_add(w).is_some_and(|x| x <= side)
            && self.at[1].checked_add(h).is_some_and(|y| y <= side)
            && self.classes.len() == (w * h) as usize
    }
}

/// Where a land layer's near tiles lie on its chart: tile (i, j) covers chart x from `origin[0] + i * tile_m`, z
/// likewise, and the layer holds `slots`² of them at once, tile (i, j) in slot (i mod `slots`, j mod `slots`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearGrid {
    pub origin: [f64; 2],
    pub tile_m: f64,
    pub slots: u32,
}

impl NearGrid {
    /// The layer's surface textures' side for this grid, texels.
    pub fn texels(&self) -> u32 {
        self.slots * NEAR_TILE_TEXELS
    }

    /// The slot tile `index` takes.
    pub fn slot(&self, index: [i32; 2]) -> [u32; 2] {
        let n = self.slots as i32;
        [index[0].rem_euclid(n) as u32, index[1].rem_euclid(n) as u32]
    }

    /// The chart rectangle (x0, z0, x1, z1) whose shading tile `index` can change by coming or going: the tile grown by
    /// a tile (it feathers into its neighbours, `surface.wgsl`'s `NEAR_FEATHER` of a tile, and their filtered texels
    /// reach into it).
    pub fn reach(&self, index: [i32; 2]) -> [f64; 4] {
        let side = self.tile_m;
        let [x, z] = [
            self.origin[0] + f64::from(index[0]) * side,
            self.origin[1] + f64::from(index[1]) * side,
        ];
        [x - side, z - side, x + 2.0 * side, z + 2.0 * side]
    }
}

/// The darkest land (first percentile, linear) of a clear, well-corrected mosaic (EOX s2cloudless 2020 over Kent,
/// Guadalcanal and Lofoten: red 0.001-0.003, green 0.005-0.017, blue 0.000-0.002): what is above it in a window's
/// darkest land is haze the mosaic kept (the 2016 edition's blue stands at 0.011-0.020).
pub const DARK_LAND: [f32; 3] = [0.002, 0.010, 0.002];

/// The most haze taken out of a channel: a window with no dark land (a desert, an icefield) is not darkened for it.
pub const HAZE_MAX: f32 = 0.02;

/// Slots of [`SurfaceStatistics::class_means`]: unknown, the classes 10 to 100 by tens, mangroves (95) last.
pub const COVER_CLASSES: usize = 12;

/// The slot of a WorldCover class in [`SurfaceStatistics::class_means`] (an unknown class is slot 0).
pub fn cover_index(class: u8) -> usize {
    match class {
        cover::MANGROVE => 11,
        c if c % 10 == 0 && c <= 100 => usize::from(c / 10),
        _ => 0,
    }
}

/// A window's imagery as a whole, for the shading (`TerrainLayer::set_surface_maps` hands it to the shaders).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceStatistics {
    /// Linear colour to take out of every texel: the haze a mosaic's atmospheric correction left (dark-object
    /// subtraction).
    pub haze: [f32; 3],
    /// Each class's mean linear colour with the haze taken out, w 1 where the window has the class (w 0, none):
    /// what a texel of the class looks like when its own colour is a cloud the mosaic kept.
    pub class_means: [[f32; 4]; COVER_CLASSES],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_statistics_find_the_haze_and_each_class_colour() {
        // A hazy mosaic: every texel's blue lifted; trees dark green, fields lighter, a cloud over one tree texel, sea.
        let size = 20u32;
        let n = (size * size) as usize;
        let mut albedo = Vec::with_capacity(n * 4);
        let mut classes = Vec::with_capacity(n);
        for k in 0..n {
            // In 2 × 2 blocks: the statistics read every other texel of every other row.
            let (i, j) = (k % size as usize, k / size as usize);
            let (rgb, class) = match (i / 2 + j / 2) % 4 {
                0 => ([30u8, 60, 40], cover::TREE),
                1 => ([70, 90, 70], cover::CROP),
                2 => ([20, 40, 60], cover::WATER),
                _ => ([30, 60, 40], cover::TREE),
            };
            let rgb = if k == 40 { [240, 240, 240] } else { rgb };
            albedo.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            classes.push(class);
        }
        let maps = SurfaceMaps {
            origin: [0.0; 2],
            extent_m: 1000.0,
            size,
            albedo,
            cover: classes,
        };
        let stats = maps.statistics();
        // The darkest land is the trees': its blue (40 → 0.021) well above a clear mosaic's.
        assert!(
            (stats.haze[2] - (0.0212 - DARK_LAND[2])).abs() < 0.002,
            "{:?}",
            stats.haze
        );
        assert!(stats.haze[0] < stats.haze[2]);
        let tree = stats.class_means[cover_index(cover::TREE)];
        let crop = stats.class_means[cover_index(cover::CROP)];
        assert_eq!(tree[3], 1.0);
        assert_eq!(stats.class_means[cover_index(cover::BUILT)][3], 0.0);
        // The cloud left out: the trees' mean stays dark.
        assert!(tree[1] < 0.06 && crop[1] > tree[1], "{tree:?} {crop:?}");
        assert_eq!(cover_index(cover::MOSS), 10);
        assert_eq!(cover_index(cover::MANGROVE), 11);
        assert_eq!(cover_index(255), 0);
    }
}
