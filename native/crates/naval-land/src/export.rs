//! A closure's land in one file (step C, §6): what makes a save portable. SAVE wraps it with the save, its scenario
//! and any user legacy region; this is the land's part.
//!
//! The bundle: `NLX1`, a u32 header length (little-endian), the header as JSON (the release's id; where its file, its
//! square table and its source manifest lie; each square's name, manifest entry and, for a data square, where its bytes
//! lie), then the bytes, squares in key order.
use std::collections::BTreeMap;

use naval_land_format::{Class, Digest, Entry, FinalSquare, ReleaseFile, ReleaseId, SquareKey};
use serde::{Deserialize, Serialize};

use crate::store::{Answer, Content, Store, StoreError};

pub const BUNDLE_MAGIC: &[u8; 4] = b"NLX1";
const FORMAT: u32 = 1;

/// The land a save or a clip depends on: its release and the manifest line of every candidate square it admitted,
/// `empty` ones included. Ocean and polar squares need none: the release's table answers them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closure {
    pub release: ReleaseId,
    pub squares: BTreeMap<SquareKey, Entry>,
}

impl Closure {
    /// Its data squares' contents: what its holder pins.
    pub fn contents(&self) -> impl Iterator<Item = Content> + '_ {
        self.squares
            .iter()
            .filter_map(|(&square, entry)| match entry {
                Entry::Data(sha256) => Some(Content {
                    release: self.release,
                    square,
                    sha256: *sha256,
                }),
                Entry::Empty => None,
            })
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Span {
    offset: u64,
    length: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    release: Span,
    table: Span,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sources: Option<Span>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Square {
    square: String,
    entry: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bytes: Option<Span>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Header {
    format: u32,
    release: ReleaseId,
    record: Record,
    squares: Vec<Square>,
}

fn refused(why: impl Into<String>) -> StoreError {
    StoreError::Refused(naval_land_format::Error::Corrupt(format!(
        "not a land bundle: {}",
        why.into()
    )))
}

impl Store {
    /// The closure of `squares` in release `id`: each candidate's manifest line (ocean and polar squares are the
    /// table's). Refused, naming the square, when a candidate has no line here: it was never baked on this computer.
    pub fn closure(
        &self,
        id: ReleaseId,
        squares: impl IntoIterator<Item = SquareKey>,
    ) -> Result<Closure, StoreError> {
        let mut out = BTreeMap::new();
        for key in squares {
            if self.class(id, key)? != Class::Candidate {
                continue;
            }
            let entry = self
                .recorded(id, key)?
                .ok_or(StoreError::Missing { square: key })?;
            out.insert(key, entry);
        }
        Ok(Closure {
            release: id,
            squares: out,
        })
    }

    /// One file holding a closure's land: the release's record, its squares' lines and their bytes. Refused, naming
    /// the square, when a data square's bytes are not on this computer, and when this store records other bytes.
    pub fn export(&self, closure: &Closure) -> Result<Vec<u8>, StoreError> {
        let record = self.record(closure.release)?;
        let mut payload = vec![];
        let mut put = |bytes: &[u8]| {
            let span = Span {
                offset: payload.len() as u64,
                length: bytes.len() as u64,
            };
            payload.extend_from_slice(bytes);
            span
        };
        let record = Record {
            release: put(&record.release),
            table: put(&record.table),
            sources: record.sources.as_deref().map(&mut put),
        };
        let mut squares = vec![];
        for (&key, &entry) in &closure.squares {
            let bytes = match entry {
                Entry::Empty => {
                    if self.recorded(closure.release, key)? != Some(Entry::Empty) {
                        return Err(StoreError::Missing { square: key });
                    }
                    None
                }
                Entry::Data(sha256) => match self.read(closure.release, key)? {
                    Answer::Land(lease) if lease.content().sha256 == sha256 => {
                        Some(put(lease.bytes()))
                    }
                    Answer::Land(lease) => {
                        return Err(StoreError::Conflict(
                            naval_land_format::manifest::Conflict {
                                square: key,
                                recorded: Entry::Data(lease.content().sha256),
                                offered: entry,
                            },
                        ));
                    }
                    Answer::Sea | Answer::Missing { .. } => {
                        return Err(StoreError::Missing { square: key });
                    }
                },
            };
            squares.push(Square {
                square: key.name(),
                entry: entry.text(),
                bytes,
            });
        }
        let header = Header {
            format: FORMAT,
            release: closure.release,
            record,
            squares,
        };
        let json = serde_json::to_vec(&header).expect("a bundle's header serialises");
        let mut out = Vec::with_capacity(8 + json.len() + payload.len());
        out.extend_from_slice(BUNDLE_MAGIC);
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&payload);
        Ok(out)
    }

    /// Keeps a bundle's land. Everything is checked before anything is kept: the release's file against its id, its
    /// table and source manifest against the file, each square's bytes against its line and its key, and each line
    /// against this store's manifest (a square this store records otherwise refuses the whole bundle). Then the
    /// release is installed and each line recorded. The closure, for its holder's pins.
    pub fn import(&self, bundle: &[u8]) -> Result<Closure, StoreError> {
        if bundle.len() < 8 || &bundle[..4] != BUNDLE_MAGIC {
            return Err(refused("no NLX1 magic"));
        }
        let length = u32::from_le_bytes(bundle[4..8].try_into().unwrap()) as usize;
        let json = bundle
            .get(8..8 + length)
            .ok_or_else(|| refused("short header"))?;
        let header: Header = serde_json::from_slice(json).map_err(|e| refused(e.to_string()))?;
        if header.format != FORMAT {
            return Err(refused(format!("format {}", header.format)));
        }
        let payload = &bundle[8 + length..];
        let slice = |span: Span| {
            let from = usize::try_from(span.offset).ok()?;
            payload.get(from..from.checked_add(usize::try_from(span.length).ok()?)?)
        };
        let release =
            slice(header.record.release).ok_or_else(|| refused("the release runs past it"))?;
        let table = slice(header.record.table).ok_or_else(|| refused("the table runs past it"))?;
        let sources = match header.record.sources {
            Some(span) => Some(slice(span).ok_or_else(|| refused("the sources run past it"))?),
            None => None,
        };
        let parsed = ReleaseFile::parse(release).map_err(StoreError::Refused)?;
        if parsed.id != header.release {
            return Err(refused("its release file is not the release it names"));
        }
        let mut squares: BTreeMap<SquareKey, (Entry, Option<&[u8]>)> = BTreeMap::new();
        for s in &header.squares {
            let key = SquareKey::parse(&s.square)
                .ok_or_else(|| refused(format!("{} is not a square", s.square)))?;
            let entry = Entry::parse(&s.entry)
                .ok_or_else(|| refused(format!("{key}'s entry {}", s.entry)))?;
            let bytes = match (entry, s.bytes) {
                (Entry::Empty, None) => None,
                (Entry::Data(sha256), Some(span)) => {
                    let bytes =
                        slice(span).ok_or_else(|| refused(format!("{key} runs past it")))?;
                    if Digest::of(bytes) != sha256 {
                        return Err(refused(format!("{key}'s bytes are not its line's")));
                    }
                    let square = FinalSquare::read(bytes.to_vec()).map_err(StoreError::Refused)?;
                    if square.key() != key {
                        return Err(StoreError::NotItsSquare {
                            square: key,
                            holds: square.key(),
                        });
                    }
                    Some(bytes)
                }
                _ => return Err(refused(format!("{key}'s bytes do not go with its entry"))),
            };
            if squares.insert(key, (entry, bytes)).is_some() {
                return Err(refused(format!("{key} twice")));
            }
        }
        let id = self.install(release, table, sources)?;
        for (&key, &(entry, _)) in &squares {
            let class = self.class(id, key)?;
            if class != Class::Candidate {
                return Err(StoreError::NotCandidate { square: key, class });
            }
            if let Some(recorded) = self.recorded(id, key)?
                && recorded != entry
            {
                return Err(StoreError::Conflict(
                    naval_land_format::manifest::Conflict {
                        square: key,
                        recorded,
                        offered: entry,
                    },
                ));
            }
        }
        for (&key, &(entry, bytes)) in &squares {
            match bytes {
                Some(bytes) => self.put(id, key, bytes)?,
                None => self.put_empty(id, key)?,
            };
            debug_assert_eq!(self.recorded(id, key)?, Some(entry));
        }
        Ok(Closure {
            release: id,
            squares: squares.into_iter().map(|(k, (e, _))| (k, e)).collect(),
        })
    }
}
