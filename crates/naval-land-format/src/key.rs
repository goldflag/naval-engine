//! A square's key: its south-west corner in whole degrees.
use serde::{Deserialize, Serialize};

/// A 1° square by its south-west corner: latitude −90 to 89, longitude −180 to 179 (a square never straddles ±180°).
/// Ordered by latitude, then longitude: the square table's order and the order squares are admitted in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SquareKey {
    pub lat: i32,
    pub lon: i32,
}

impl SquareKey {
    /// Squares on the Earth.
    pub const COUNT: usize = 180 * 360;

    /// The square holding (lat, lon), longitude wrapped onto the world: half-open, `[lat, lat + 1) × [lon, lon + 1)`.
    pub fn at(lat: f64, lon: f64) -> Self {
        Self {
            lat: lat.floor().clamp(-90.0, 89.0) as i32,
            lon: (lon + 180.0).rem_euclid(360.0).floor() as i32 - 180,
        }
    }

    /// Whether it names a square of the world.
    pub fn valid(self) -> bool {
        (-90..=89).contains(&self.lat) && (-180..=179).contains(&self.lon)
    }

    /// Its name: `N36W123`.
    pub fn name(self) -> String {
        format!(
            "{}{:02}{}{:03}",
            if self.lat >= 0 { 'N' } else { 'S' },
            self.lat.abs(),
            if self.lon >= 0 { 'E' } else { 'W' },
            self.lon.abs()
        )
    }

    /// Its file's name in a folder of surveyed squares: `N36W123.nsq`.
    pub fn file_name(self) -> String {
        format!("{}.nsq", self.name())
    }

    /// The square a name gives, as [`Self::name`] writes it and no other way (`S00` and `W000` are refused: one square,
    /// one name).
    pub fn parse(name: &str) -> Option<Self> {
        let b = name.as_bytes();
        if b.len() != 7 || !b[1..3].iter().chain(&b[4..7]).all(u8::is_ascii_digit) {
            return None;
        }
        let number = |digits: &[u8]| {
            digits
                .iter()
                .fold(0i32, |n, d| n * 10 + i32::from(d - b'0'))
        };
        let (lat, lon) = (number(&b[1..3]), number(&b[4..7]));
        let lat = match b[0] {
            b'N' => lat,
            b'S' if lat > 0 => -lat,
            _ => return None,
        };
        let lon = match b[3] {
            b'E' => lon,
            b'W' if lon > 0 => -lon,
            _ => return None,
        };
        let key = Self { lat, lon };
        key.valid().then_some(key)
    }

    /// Its place in the square table: south to north, west to east.
    pub fn index(self) -> usize {
        debug_assert!(self.valid(), "{self:?} is not a square");
        ((self.lat + 90) * 360 + (self.lon + 180)) as usize
    }

    /// The square at a place in the table.
    pub fn from_index(index: usize) -> Self {
        assert!(index < Self::COUNT, "square {index} is past the table");
        Self {
            lat: (index / 360) as i32 - 90,
            lon: (index % 360) as i32 - 180,
        }
    }

    /// Every square, in the table's order.
    pub fn all() -> impl Iterator<Item = Self> {
        (0..Self::COUNT).map(Self::from_index)
    }

    /// The square east of it, over ±180°.
    pub fn east(self) -> Self {
        Self {
            lat: self.lat,
            lon: if self.lon == 179 { -180 } else { self.lon + 1 },
        }
    }

    /// The square north of it, none past the pole.
    pub fn north(self) -> Option<Self> {
        (self.lat < 89).then_some(Self {
            lat: self.lat + 1,
            lon: self.lon,
        })
    }
}

impl std::fmt::Display for SquareKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_square_has_one_name_and_one_index() {
        for (k, key) in SquareKey::all().enumerate() {
            assert_eq!(key.index(), k);
            assert_eq!(SquareKey::parse(&key.name()), Some(key), "{key}");
        }
        assert_eq!(SquareKey::all().count(), SquareKey::COUNT);
        assert_eq!(
            SquareKey::parse("N36W123"),
            Some(SquareKey { lat: 36, lon: -123 })
        );
        for bad in [
            "S00E000", "N00W000", "N90E000", "S91E000", "N00E180", "N00W181", "n36W123", "N36W12",
            "N36W1234", "N3xW123", "N36X123",
        ] {
            assert_eq!(SquareKey::parse(bad), None, "{bad}");
        }
        assert_eq!(
            SquareKey::parse("S90W180"),
            Some(SquareKey {
                lat: -90,
                lon: -180
            })
        );
        assert_eq!(
            SquareKey::parse("N89E179"),
            Some(SquareKey { lat: 89, lon: 179 })
        );
    }

    #[test]
    fn a_place_has_one_square_half_open_and_wrapped() {
        assert_eq!(
            SquareKey::at(36.9, -122.4),
            SquareKey { lat: 36, lon: -123 }
        );
        assert_eq!(SquareKey::at(-9.2, 160.1).name(), "S10E160");
        assert_eq!(SquareKey::at(10.0, 181.5), SquareKey { lat: 10, lon: -179 });
        assert_eq!(SquareKey::at(10.0, 180.0), SquareKey { lat: 10, lon: -180 });
        assert_eq!(
            SquareKey::at(10.0, -180.0),
            SquareKey { lat: 10, lon: -180 }
        );
        assert_eq!(
            SquareKey::at(37.0, -122.0),
            SquareKey { lat: 37, lon: -122 }
        );
        assert_eq!(SquareKey::at(90.0, 0.0), SquareKey { lat: 89, lon: 0 });
        assert_eq!(
            SquareKey { lat: 5, lon: 179 }.east(),
            SquareKey { lat: 5, lon: -180 }
        );
        assert_eq!(SquareKey { lat: 89, lon: 0 }.north(), None);
    }
}
