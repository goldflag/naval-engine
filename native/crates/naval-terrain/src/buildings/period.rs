//! The age the buildings are drawn in (native/docs/terrain/buildings.md, "Period"): today's world as the sources map
//! it, or the world as it stood in a year (the scenario's), with fewer buildings in an older manner. The game's Graphics
//! menu chooses (`buildings`: `period` or `today`); the host hands the scenario's year in.

/// What the buildings are drawn as.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Period {
    /// Today's, as the footprints and the map have them.
    #[default]
    Today,
    /// As they stood in this year (a scenario's start, [`Period::FIRST_YEAR`] to [`Period::LAST_YEAR`]).
    Year(u16),
}

impl Period {
    /// The earliest year drawn: an earlier scenario's buildings are drawn as of this one.
    pub const FIRST_YEAR: u16 = 1900;
    /// The latest year drawn as a period: a scenario set after it draws today's buildings.
    pub const LAST_YEAR: u16 = 1960;
    /// The year a scenario whose start gives none is drawn in.
    pub const DEFAULT_YEAR: u16 = 1942;

    /// The period of a scenario starting in `year` (none: [`Period::DEFAULT_YEAR`]); today's after
    /// [`Period::LAST_YEAR`].
    pub fn of_year(year: Option<u16>) -> Self {
        let year = year.unwrap_or(Self::DEFAULT_YEAR);
        if year > Self::LAST_YEAR {
            Self::Today
        } else {
            Self::Year(year.max(Self::FIRST_YEAR))
        }
    }

    /// The year drawn, none for today's.
    pub fn year(self) -> Option<u16> {
        match self {
            Self::Today => None,
            Self::Year(year) => Some(year),
        }
    }

    /// Whether the buildings are drawn as of a past year.
    pub fn is_past(self) -> bool {
        matches!(self, Self::Year(_))
    }

    /// A key for made meshes: 0 today, else the year.
    pub fn key(self) -> u16 {
        self.year().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scenarios_year_picks_the_period() {
        assert_eq!(Period::of_year(Some(1942)), Period::Year(1942));
        assert_eq!(Period::of_year(None), Period::Year(Period::DEFAULT_YEAR));
        assert_eq!(
            Period::of_year(Some(1850)),
            Period::Year(Period::FIRST_YEAR)
        );
        assert_eq!(Period::of_year(Some(1985)), Period::Today);
        assert!(Period::Year(1941).is_past());
        assert!(!Period::Today.is_past());
        assert_eq!(Period::Today.key(), 0);
        assert_eq!(Period::Year(1941).key(), 1941);
    }
}
