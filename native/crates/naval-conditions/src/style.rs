//! The land's look in a scene: the map's style, the palette the terrain shades it with, and the place and season it is
//! drawn in. The conditions carry it ([`crate::MapScene::style`]); the land (`naval-terrain`) draws by it.

use serde::Deserialize;

/// The land's look (`OceanMap.land.style`): what covers the ground in `TerrainSurface.ts`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LandStyle {
    Tropical,
    Snow,
    Volcanic,
    Chalk,
    Rock,
}

impl LandStyle {
    /// The shader's index for the style (`terrain.wgsl`'s `STYLE_*`).
    pub fn index(self) -> u32 {
        match self {
            LandStyle::Tropical => 0,
            LandStyle::Snow => 1,
            LandStyle::Volcanic => 2,
            LandStyle::Chalk => 3,
            LandStyle::Rock => 4,
        }
    }
}

/// A map's land palette in linear light and the chart direction toward the equator (`SurfaceStyle`).
/// Where on the earth and when in the year a land is drawn, for the styles that change with them (the snow and chalk
/// styles); [`Climate::AUTHORED`] draws each style as it was authored for the removed land maps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climate {
    /// Height (m) the snow lies down to on a level open slope: 230 m, a Norwegian April's.
    pub snowline_m: f32,
    /// 0-1, how far the thaw has gone below the snowline: the snow lying in patches gone, the heath greening.
    pub thaw: f32,
    /// 0-1, the leaves out: the woods green, the fields grown (0 the chalk style's February, the birch bare).
    pub leaf: f32,
    /// 0-1, how dry the country looks (a dry summer, a desert): straw downland, ochre stubble, stone walls for hedges,
    /// olive and maquis for woods. A theatre's from the Köppen-Geiger classes about it (`theatre::land::climate`).
    pub dry: f32,
    /// Where on the earth the land stands (latitude, longitude in degrees), from which its trees' species are chosen
    /// (`species::Biome`); none keeps the style's own trees. A theatre's land gives its chart's
    /// origin's place, from which the placement finds each tree's latitude (the chart's z runs south: redwoods only
    /// north of Big Sur, the plantations of Honshu and not Hokkaido's).
    pub place: Option<[f32; 2]>,
}

impl Climate {
    /// Each style as authored for its land map: northern Norway in April, Kent in February, a wet country.
    pub const AUTHORED: Self = Self {
        snowline_m: 230.0,
        thaw: 0.0,
        leaf: 0.0,
        dry: 0.0,
        place: None,
    };
}

impl Default for Climate {
    fn default() -> Self {
        Self::AUTHORED
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceStyle {
    pub style: LandStyle,
    pub shore: [f32; 3],
    pub low: [f32; 3],
    pub high: [f32; 3],
    pub equator: [f32; 2],
    /// The season and climate it is drawn in (the snowline, the leaves, a dry summer).
    pub climate: Climate,
}
