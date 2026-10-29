//! The period's country (native/docs/terrain/period.md, "The period's ground", "The country"): what the land draws
//! today's
//! built-up land as where a past period leaves it empty (the cover's mark, `surface_maps::cover::PAST_COUNTRY`). Not the
//! photograph's roofs and pools but the region's farmland of the time, a patchwork of fields synthesised from the
//! window's own rural colours (`surface.wgsl`'s `country_field`, `country_ground`), and the trees of that farmland
//! (`tree_place.wgsl`'s `country_slot`). The host sets it per scene from where the land is
//! (`TerrainLayer::set_country`); with none, or for `Period::Today`, a mark draws as today's class.

use crate::buildings::Region;

/// A region's country in the 1940s, as the land draws it (`surface.wgsl`'s `COUNTRY_*`, in this order from 1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Country {
    /// California and the American South-West in 1942: citrus and walnut groves in rows, truck farms' strips, dry
    /// grain and grass on a section-line grid, dirt roads between, windbreaks of eucalyptus.
    West,
    /// The rest of North America: fields and pasture, hay, woodlots, fencerows.
    East,
    /// Britain and Ireland: small fields of pasture, hay and corn between dark hedgerows with their trees.
    Britain,
    /// The Low Countries, France, Germany: open fields of corn and pasture in long strips, few hedges.
    Europe,
    /// Japan, Korea, Taiwan and eastern China: rice paddies in small plots between bunds, dry fields' strips, village
    /// groves.
    Japan,
    /// The tropics and the Pacific's islands: kunai grass, coconut plantations in rows, gardens, bush.
    Tropics,
    /// Iceland and the Nordic countries: hayfields (the tún), heath and moss, drainage ditches.
    Nordic,
    /// The Mediterranean and the dry countries: olive groves, dry grain and stubble, vines, garrigue, stone walls.
    Mediterranean,
}

impl Country {
    /// The country of a buildings region (`Region::at`).
    pub fn of(region: Region) -> Self {
        match region {
            Region::West => Country::West,
            Region::American | Region::Florida => Country::East,
            Region::British | Region::Celtic => Country::Britain,
            Region::NorthSea | Region::Temperate => Country::Europe,
            Region::EastAsia => Country::Japan,
            Region::Tropical | Region::Hawaii | Region::Micronesia | Region::Atoll => {
                Country::Tropics
            }
            Region::Iceland | Region::Nordic => Country::Nordic,
            Region::Mediterranean
            | Region::Malta
            | Region::Aegean
            | Region::NorthAfrica
            | Region::Nile
            | Region::Arid => Country::Mediterranean,
        }
    }

    /// Its index in the shaders (`surface.wgsl`'s `COUNTRY_*`): 1 and up, 0 for none.
    pub fn index(self) -> u32 {
        match self {
            Country::West => 1,
            Country::East => 2,
            Country::Britain => 3,
            Country::Europe => 4,
            Country::Japan => 5,
            Country::Tropics => 6,
            Country::Nordic => 7,
            Country::Mediterranean => 8,
        }
    }

    /// Its fields' size, metres (`surface.wgsl`'s `country_layout`: a laid out field's length and breadth before its
    /// district's scale of 0.75 to 1.25, an irregular one's spacing twice).
    pub fn field_m(self) -> [f64; 2] {
        match self {
            Country::West => [400.0, 200.0],
            Country::East => [320.0, 180.0],
            Country::Britain => [150.0, 150.0],
            Country::Europe => [320.0, 70.0],
            Country::Japan => [110.0, 60.0],
            Country::Tropics => [380.0, 260.0],
            Country::Nordic => [120.0, 120.0],
            Country::Mediterranean => [190.0, 110.0],
        }
    }

    /// How far a mark's change can change the drawn ground, metres: a point looks for marks out to 1.5 of a field's
    /// length (`COUNTRY_REACH`), and a field is the country by the marks about its middle, up to the largest field's
    /// diagonal away.
    pub fn reach_m(self) -> f64 {
        let [x, y] = self.field_m();
        1.5 * x + 1.25 * x.hypot(y)
    }
}

/// What the land draws the period's country as: the region's farmland at a time of the year.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CountryLook {
    pub country: Country,
    /// The season, months since mid-January in the north (a field's crops ripen and are cut by it), half a year on
    /// south of the equator: 0-12.
    pub months: f32,
}

impl CountryLook {
    /// The country of `region` on the date `start` (`YYYY-MM-DD…`, a scenario's start) at latitude `lat`.
    pub fn at(region: Region, start: &str, lat: f64) -> Self {
        Self {
            country: Country::of(region),
            months: months(start, lat),
        }
    }

    /// The uniform's row (`terrain.country`): the country's index, the months, the test's marks (0), 1 while drawn.
    pub(crate) fn uniform(self, test_marks: f32) -> [f32; 4] {
        [self.country.index() as f32, self.months, test_marks, 1.0]
    }
}

/// Months since the 15th of January (north of the equator; half a year on south of it) of the date `start`
/// (`YYYY-MM-DD…`; mid-June where it cannot be read), as the theatre's climate counts them.
pub fn months(start: &str, lat: f64) -> f32 {
    let month = start
        .get(5..7)
        .and_then(|m| m.parse::<f64>().ok())
        .unwrap_or(6.0)
        .clamp(1.0, 12.0);
    let day = start
        .get(8..10)
        .and_then(|d| d.parse::<f64>().ok())
        .unwrap_or(15.0)
        .clamp(1.0, 31.0);
    let shift = if lat < 0.0 { 6.0 } else { 0.0 };
    (month - 1.0 + (day - 15.0) / 30.5 + shift).rem_euclid(12.0) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buildings::region::REGIONS;

    #[test]
    fn every_region_has_a_country() {
        let mut seen = std::collections::HashSet::new();
        for region in REGIONS {
            let country = Country::of(region);
            assert!((1..=8).contains(&country.index()), "{region:?}");
            seen.insert(country.index());
        }
        assert_eq!(seen.len(), 8, "every country is some region's");
        assert_eq!(Country::of(Region::West), Country::West);
        assert_eq!(Country::of(Region::EastAsia), Country::Japan);
        assert_eq!(Country::of(Region::Iceland), Country::Nordic);
    }

    #[test]
    fn the_months_count_from_mid_january_and_turn_south_of_the_equator() {
        assert!((months("1942-01-15T10:00", 35.0) - 0.0).abs() < 1e-6);
        assert!((months("1942-06-01T10:00", 35.0) - 4.541).abs() < 0.01);
        assert!((months("1942-08-08T10:00", -9.4) - 0.77).abs() < 0.01);
        assert!((months("1942-12-31", 50.0) - 11.52).abs() < 0.01);
        assert!((months("", 50.0) - 5.0).abs() < 1e-6);
        let look = CountryLook::at(Region::British, "1942-06-01", 50.8);
        assert_eq!(look.uniform(0.0)[0], 3.0);
        assert_eq!(look.uniform(0.0)[3], 1.0);
    }
}
