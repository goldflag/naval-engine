//! The sky's plain types the simulation and the native sky share: the volumetric layer's cloud genera and the high
//! sheet over it. The simulation's cloud model (`naval_sim::clouds`) and the native sky (`naval_sky::state`) both
//! re-export them; they live here, a crate with no dependencies, so the sky builds without the simulation and a
//! simulation edit does not rebuild it.

/// The cloud genera of the WMO cloud atlas the volumetric layer draws, at whatever height the scene puts the layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CloudGenus {
    Cumulus,
    Cumulonimbus,
    Stratocumulus,
    Stratus,
    Nimbostratus,
    Altocumulus,
    Altostratus,
}

impl CloudGenus {
    /// The genus by its web name (`"cumulus"`, …).
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "cumulus" => Self::Cumulus,
            "cumulonimbus" => Self::Cumulonimbus,
            "stratocumulus" => Self::Stratocumulus,
            "stratus" => Self::Stratus,
            "nimbostratus" => Self::Nimbostratus,
            "altocumulus" => Self::Altocumulus,
            "altostratus" => Self::Altostratus,
            _ => return None,
        })
    }
}

/// The high sheet's genera (`HighCloudGenus`): cirrus fibres, a cirrostratus veil, cirrocumulus grains.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighCloudGenus {
    Cirrus,
    Cirrostratus,
    Cirrocumulus,
}

impl HighCloudGenus {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "cirrus" => Self::Cirrus,
            "cirrostratus" => Self::Cirrostratus,
            "cirrocumulus" => Self::Cirrocumulus,
            _ => return None,
        })
    }
}

/// The high sheet over the layer as a scene asks for it (`HighClouds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighClouds {
    pub genus: HighCloudGenus,
    /// 0 none … 1 the most of the genus the sheet shows.
    pub amount: f64,
    /// Height of the sheet above the sea (m).
    pub altitude: f64,
}
