//! A release's square table: what each of the 64,800 squares is, computed once when the release is made.
//!
//! - `ocean`: the release's water covers the square and its consideration margin round it (65 km in `land-v1`): open
//!   sea, authoritatively, never baked.
//! - `candidate`: baked; the bake then writes the square's manifest line, `empty` or its bytes' digest.
//! - `polar`: a square meeting the simulation's pole limit (within [`POLE_LIMIT_DEG`] of a pole): open sea.
//!
//! The file ([`SquareTable::encode`]): `NLT1`, a u32 count of runs, then each run as a class byte and a u32 length
//! (little-endian), over the squares in [`SquareKey::index`] order (south to north, west to east). Runs are maximal
//! and never empty, so one table has one file and one digest.
use crate::{Error, SquareKey, corrupt};

pub const MAGIC: &[u8; 4] = b"NLT1";
/// The simulation's pole limit (`earth::sphere`'s `POLE_LIMIT_Z`, half a degree from a pole): a square reaching past
/// this latitude is polar.
pub const POLE_LIMIT_DEG: f64 = 89.5;

/// What the release says a square is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Class {
    Ocean = 0,
    Candidate = 1,
    Polar = 2,
}

impl Class {
    fn from_byte(b: u8) -> Option<Self> {
        match b {
            0 => Some(Class::Ocean),
            1 => Some(Class::Candidate),
            2 => Some(Class::Polar),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Ocean => "ocean",
            Class::Candidate => "candidate",
            Class::Polar => "polar",
        }
    }
}

/// Whether a square meets the pole limit: its north edge past 89.5° N or its south edge past 89.5° S.
pub fn polar(key: SquareKey) -> bool {
    f64::from(key.lat + 1) > POLE_LIMIT_DEG || f64::from(key.lat) < -POLE_LIMIT_DEG
}

/// Every square's class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SquareTable {
    classes: Vec<Class>,
}

impl SquareTable {
    /// The table whose non-polar squares are candidates where `candidate` says so, ocean elsewhere.
    pub fn new(mut candidate: impl FnMut(SquareKey) -> bool) -> Self {
        let classes = SquareKey::all()
            .map(|key| {
                if polar(key) {
                    Class::Polar
                } else if candidate(key) {
                    Class::Candidate
                } else {
                    Class::Ocean
                }
            })
            .collect();
        Self { classes }
    }

    /// A table from every square's class in index order: polar exactly where [`polar`] says.
    pub fn from_classes(classes: Vec<Class>) -> Result<Self, Error> {
        let what = "a square table";
        if classes.len() != SquareKey::COUNT {
            return Err(corrupt(
                what,
                format!("{} squares, not 64,800", classes.len()),
            ));
        }
        for (k, &class) in classes.iter().enumerate() {
            let key = SquareKey::from_index(k);
            if (class == Class::Polar) != polar(key) {
                return Err(corrupt(what, format!("{key} is {}", class.name())));
            }
        }
        Ok(Self { classes })
    }

    pub fn class(&self, key: SquareKey) -> Class {
        self.classes[key.index()]
    }

    /// The candidate squares, in index order.
    pub fn candidates(&self) -> impl Iterator<Item = SquareKey> + '_ {
        self.classes
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == Class::Candidate)
            .map(|(k, _)| SquareKey::from_index(k))
    }

    /// The table's file.
    pub fn encode(&self) -> Vec<u8> {
        let mut runs: Vec<(Class, u32)> = vec![];
        for &class in &self.classes {
            match runs.last_mut() {
                Some((last, n)) if *last == class => *n += 1,
                _ => runs.push((class, 1)),
            }
        }
        let mut out = Vec::with_capacity(8 + 5 * runs.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(runs.len() as u32).to_le_bytes());
        for (class, n) in runs {
            out.push(class as u8);
            out.extend_from_slice(&n.to_le_bytes());
        }
        out
    }

    /// A table from its file, refused unless it is exactly the file [`Self::encode`] writes for it.
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let what = "a square table";
        if bytes.len() < 8 || &bytes[..4] != MAGIC {
            return Err(corrupt(what, "no NLT1 magic"));
        }
        let runs = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        if Some(bytes.len()) != runs.checked_mul(5).and_then(|n| n.checked_add(8)) {
            return Err(corrupt(what, "its length is not its runs'"));
        }
        let mut classes = Vec::with_capacity(SquareKey::COUNT);
        let mut previous = None;
        for run in bytes[8..].chunks(5) {
            let class =
                Class::from_byte(run[0]).ok_or_else(|| corrupt(what, "a run of no class"))?;
            let n = u32::from_le_bytes(run[1..5].try_into().unwrap()) as usize;
            if n == 0 || previous == Some(class) || classes.len() + n > SquareKey::COUNT {
                return Err(corrupt(what, "its runs are not the table's own"));
            }
            classes.resize(classes.len() + n, class);
            previous = Some(class);
        }
        Self::from_classes(classes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_has_one_file_and_its_poles_are_polar() {
        let table = SquareTable::new(|k| {
            (k.lat == 36 && k.lon < -120) || k == SquareKey { lat: -5, lon: 179 }
        });
        assert_eq!(
            table.class(SquareKey { lat: 36, lon: -123 }),
            Class::Candidate
        );
        assert_eq!(table.class(SquareKey { lat: 36, lon: -120 }), Class::Ocean);
        assert_eq!(table.class(SquareKey { lat: 89, lon: 0 }), Class::Polar);
        assert_eq!(table.class(SquareKey { lat: -90, lon: 12 }), Class::Polar);
        assert_eq!(table.class(SquareKey { lat: 88, lon: 0 }), Class::Ocean);
        assert_eq!(table.candidates().count(), 61);
        let bytes = table.encode();
        assert_eq!(SquareTable::decode(&bytes).unwrap(), table);
        // Two runs of one class where one would do, a run of nothing, a short table, a polar square called ocean.
        let mut split = bytes[..8].to_vec();
        split[4..8].copy_from_slice(
            &(u32::from_le_bytes(bytes[4..8].try_into().unwrap()) + 1).to_le_bytes(),
        );
        split.extend_from_slice(&[2, 1, 0, 0, 0]);
        let first = u32::from_le_bytes(bytes[9..13].try_into().unwrap());
        split.extend_from_slice(&[2]);
        split.extend_from_slice(&(first - 1).to_le_bytes());
        split.extend_from_slice(&bytes[13..]);
        assert!(SquareTable::decode(&split).is_err());
        let mut short = bytes.clone();
        let last = short.len() - 4;
        short[last] -= 1;
        assert!(SquareTable::decode(&short).is_err());
        let mut classes = vec![Class::Ocean; SquareKey::COUNT];
        assert!(SquareTable::from_classes(classes.clone()).is_err());
        for (k, c) in classes.iter_mut().enumerate() {
            if polar(SquareKey::from_index(k)) {
                *c = Class::Polar;
            }
        }
        assert_eq!(
            SquareTable::from_classes(classes)
                .unwrap()
                .candidates()
                .count(),
            0
        );
    }
}
