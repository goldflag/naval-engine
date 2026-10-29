//! A release's manifest: append-only, one line a square, `N36W123 <sha256>` (its final square's bytes) or
//! `N36W123 empty` (the bake found no land within the keep distance). A line once written never changes: a square baked
//! again must give the bytes its line names, and one that gives other bytes is a [`Conflict`], reported and never
//! stored. Lines are in the order they were written; every line ends in a newline.
use std::collections::BTreeMap;

use crate::{Digest, Error, SquareKey, corrupt};

/// What the manifest records for a square.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Entry {
    /// No final land within the keep distance of the square: open sea, with no file.
    Empty,
    /// The SHA-256 of its final square's bytes.
    Data(Digest),
}

impl Entry {
    /// As a line writes it: the digest's hex, or `empty`.
    pub fn text(self) -> String {
        match self {
            Entry::Empty => "empty".into(),
            Entry::Data(digest) => digest.hex(),
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        if text == "empty" {
            Some(Entry::Empty)
        } else {
            Digest::parse(text).map(Entry::Data)
        }
    }
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

/// A square baked again whose result is not what the manifest records: the release's land would change, so the result
/// is refused and kept nowhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub square: SquareKey,
    pub recorded: Entry,
    pub offered: Entry,
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} baked to {} where the release's manifest records {}: not stored",
            self.square, self.offered, self.recorded
        )
    }
}

impl std::error::Error for Conflict {}

/// What recording an entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recorded {
    /// The square had no line: this one is to be appended.
    New(String),
    /// The square's line already says this.
    Already,
}

/// A manifest in memory: its lines in order, each square once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manifest {
    entries: BTreeMap<SquareKey, Entry>,
    order: Vec<SquareKey>,
}

impl Manifest {
    /// One line, newline included.
    pub fn line(key: SquareKey, entry: Entry) -> String {
        format!("{} {}\n", key.name(), entry.text())
    }

    /// A manifest's text: every line well formed and ended, each square once (a second line for a square, even one
    /// that agrees, is not something an append-only writer makes).
    pub fn parse(text: &str) -> Result<Self, Error> {
        let what = "a land manifest";
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(corrupt(what, "its last line is not ended"));
        }
        let mut manifest = Self::default();
        for (n, line) in text.split_terminator('\n').enumerate() {
            let (name, value) = line.split_once(' ').ok_or_else(|| {
                corrupt(what, format!("line {} is not `<square> <entry>`", n + 1))
            })?;
            let key = SquareKey::parse(name)
                .ok_or_else(|| corrupt(what, format!("line {} names no square", n + 1)))?;
            let entry = Entry::parse(value).ok_or_else(|| {
                corrupt(
                    what,
                    format!("line {} is neither a SHA-256 nor `empty`", n + 1),
                )
            })?;
            if manifest.entries.insert(key, entry).is_some() {
                return Err(corrupt(
                    what,
                    format!("{key} has a second line ({})", n + 1),
                ));
            }
            manifest.order.push(key);
        }
        Ok(manifest)
    }

    pub fn get(&self, key: SquareKey) -> Option<Entry> {
        self.entries.get(&key).copied()
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Every line's square and entry, in the order written.
    pub fn iter(&self) -> impl Iterator<Item = (SquareKey, Entry)> + '_ {
        self.order.iter().map(|&k| (k, self.entries[&k]))
    }

    /// What recording `entry` for `key` would do, changing nothing.
    pub fn check(&self, key: SquareKey, entry: Entry) -> Result<Recorded, Conflict> {
        match self.get(key) {
            None => Ok(Recorded::New(Self::line(key, entry))),
            Some(recorded) if recorded == entry => Ok(Recorded::Already),
            Some(recorded) => Err(Conflict {
                square: key,
                recorded,
                offered: entry,
            }),
        }
    }

    /// Records `entry` for `key`: a new line to append, nothing when the line already says it, a conflict (and no
    /// change) when it says otherwise.
    pub fn record(&mut self, key: SquareKey, entry: Entry) -> Result<Recorded, Conflict> {
        let recorded = self.check(key, entry)?;
        if let Recorded::New(_) = recorded {
            self.entries.insert(key, entry);
            self.order.push(key);
        }
        Ok(recorded)
    }

    /// The whole text, lines in the order written.
    pub fn text(&self) -> String {
        self.iter().map(|(k, e)| Self::line(k, e)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: SquareKey = SquareKey { lat: 36, lon: -123 };
    const B: SquareKey = SquareKey { lat: -1, lon: 0 };

    #[test]
    fn a_line_once_written_never_changes() {
        let x = Entry::Data(Digest::of(b"x"));
        let y = Entry::Data(Digest::of(b"y"));
        let mut m = Manifest::default();
        assert_eq!(
            m.record(A, x),
            Ok(Recorded::New(format!("N36W123 {}\n", x)))
        );
        assert_eq!(
            m.record(B, Entry::Empty),
            Ok(Recorded::New("S01E000 empty\n".into()))
        );
        // The same result again changes nothing.
        assert_eq!(m.record(A, x), Ok(Recorded::Already));
        // Another result is refused, and nothing changes.
        let before = m.clone();
        let conflict = m.record(A, y).unwrap_err();
        assert_eq!((conflict.recorded, conflict.offered), (x, y));
        assert!(conflict.to_string().contains("not stored"));
        assert_eq!(m.record(B, x).unwrap_err().recorded, Entry::Empty);
        assert_eq!(m, before);
        // Its text reads back as itself, in the order written.
        let text = m.text();
        assert_eq!(Manifest::parse(&text).unwrap(), m);
        assert_eq!(m.iter().map(|(k, _)| k).collect::<Vec<_>>(), [A, B]);
    }

    #[test]
    fn a_malformed_manifest_is_refused() {
        let x = Digest::of(b"x");
        for bad in [
            format!("N36W123 {x}"),
            format!("N36W123 {x}\nN36W123 {x}\n"),
            format!("N36W123  {x}\n"),
            "N36W123 full\n".into(),
            format!("N36W12 {x}\n"),
            format!("N36W123 {}\n", x.hex().to_uppercase()),
        ] {
            assert!(Manifest::parse(&bad).is_err(), "{bad:?}");
        }
        assert!(Manifest::parse("").unwrap().is_empty());
    }
}
