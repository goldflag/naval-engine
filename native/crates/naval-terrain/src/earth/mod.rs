//! What the land is like at a point of the earth, from global maps compiled into the game (no file is read at run
//! time): its climate ([`koppen`], Köppen-Geiger classes for 1931-1960) and its rock ([`lithology`], GLiM's dominant
//! class per half degree). Both are baked by `scripts/theatre/bake-climate-and-rock.py` and decoded on first use
//! (native/docs/terrain/ground.md, "Climate and rock maps").

pub mod koppen;
pub mod lithology;

pub use koppen::{
    DrySeason, Koppen, KoppenGroup, KoppenShares, Warmth, koppen, koppen_mode, koppen_shares,
};
pub use lithology::{Lithology, lithology, rock_near};

/// Metres of the earth's surface in a degree of latitude (the chart's own, `species::chart_place`'s).
pub const METRES_PER_DEGREE: f64 = 111_195.0;
