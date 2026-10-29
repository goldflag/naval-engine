//! A land release's file (`land-v1.release.json`): small, committed, published only once every identity it depends
//! on is frozen. It names the lattice and the final layer's encoding, the bake's algorithm and version with its rules,
//! and the hashes of the source snapshot (OpenStreetMap's water polygons), the source manifest (every DEM object a
//! candidate square can read, with its ETag and length) and the square table. Its id is the SHA-256 of the file's
//! bytes, so the file travels byte for byte; a changed source, rule or table is a new release.
//!
//! A synthetic release (`synthetic: true`, the `synthetic` module's) has no snapshot and no source manifest: its land
//! is a function of the lattice point. Every other release has both.
use serde::{Deserialize, Serialize};

use crate::block::ENCODING;
use crate::lattice::{BLOCKS, PER_DEGREE, QUANTUM_M};
use crate::table::POLE_LIMIT_DEG;
use crate::{Digest, Error, corrupt};

/// The release file's version.
pub const FORMAT: u32 = 1;

/// A release's id: the SHA-256 of its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReleaseId(pub Digest);

impl ReleaseId {
    pub fn of(file: &[u8]) -> Self {
        Self(Digest::of(file))
    }
    pub fn parse(hex: &str) -> Option<Self> {
        Digest::parse(hex).map(Self)
    }
}

impl std::fmt::Display for ReleaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// The lattice and the final layer's encoding a release's squares are on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LatticeSpec {
    pub per_degree: i32,
    pub blocks: usize,
    pub quantum_m: f64,
    pub encoding: String,
}

impl LatticeSpec {
    /// This build's.
    pub fn current() -> Self {
        Self {
            per_degree: PER_DEGREE,
            blocks: BLOCKS,
            quantum_m: QUANTUM_M,
            encoding: ENCODING.into(),
        }
    }
}

/// The bake that made a release's squares: its algorithm and version, and the rules that decide which squares hold
/// land.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BakeSpec {
    pub algorithm: String,
    pub version: u32,
    /// A candidate square is written as data when final land lies within this of it, else `empty`.
    pub keep_m: u32,
    /// A square is a candidate when the snapshot's land lies within this of it.
    pub candidate_m: u32,
    /// Squares reaching past this latitude are polar.
    pub pole_limit_deg: f64,
}

/// A file a release depends on, by its digest and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileRef {
    pub sha256: Digest,
    pub bytes: u64,
}

impl FileRef {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            sha256: Digest::of(bytes),
            bytes: bytes.len() as u64,
        }
    }
    /// Whether `bytes` are the file it names.
    pub fn matches(&self, bytes: &[u8]) -> bool {
        bytes.len() as u64 == self.bytes && Digest::of(bytes) == self.sha256
    }
}

/// The source snapshot: OpenStreetMap's water polygons, as downloaded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    /// Where it came from (its file's name on osmdata.openstreetmap.de).
    pub source: String,
    /// Its date, `YYYY-MM-DD`.
    pub date: String,
    pub sha256: Digest,
    pub bytes: u64,
}

/// The release file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReleaseFile {
    pub format: u32,
    /// Its name (`land-v1`): for people; the id is the identity.
    pub name: String,
    pub lattice: LatticeSpec,
    pub bake: BakeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<Snapshot>,
    /// The source manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<FileRef>,
    pub square_table: FileRef,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub synthetic: bool,
}

impl ReleaseFile {
    /// The file's bytes: pretty JSON and a newline.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = serde_json::to_vec_pretty(self).expect("a release file serialises");
        out.push(b'\n');
        out
    }

    /// A release from its file's bytes, refused unless it is on this build's lattice and complete for its kind.
    pub fn parse(bytes: &[u8]) -> Result<Release, Error> {
        let what = "a land release";
        let file: ReleaseFile = serde_json::from_slice(bytes).map_err(|e| corrupt(what, e))?;
        if file.format != FORMAT {
            return Err(corrupt(
                what,
                format!("format {}, not {FORMAT}", file.format),
            ));
        }
        if file.lattice != LatticeSpec::current() {
            return Err(corrupt(
                what,
                "its squares are on another lattice or encoding",
            ));
        }
        if file.name.is_empty()
            || !file
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err(corrupt(
                what,
                format!("its name {:?} is not a name", file.name),
            ));
        }
        if file.bake.pole_limit_deg != POLE_LIMIT_DEG {
            return Err(corrupt(what, "its pole limit is not the simulation's"));
        }
        let pinned = file.snapshot.is_some() && file.sources.is_some();
        let sourceless = file.snapshot.is_none() && file.sources.is_none();
        if (file.synthetic && !sourceless) || (!file.synthetic && !pinned) {
            return Err(corrupt(
                what,
                "a release pins its snapshot and source manifest, and only a synthetic one has neither",
            ));
        }
        Ok(Release {
            id: ReleaseId::of(bytes),
            file,
        })
    }
}

/// A release read from its file.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub id: ReleaseId,
    pub file: ReleaseFile,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pinned() -> ReleaseFile {
        ReleaseFile {
            format: FORMAT,
            name: "land-v1".into(),
            lattice: LatticeSpec::current(),
            bake: BakeSpec {
                algorithm: "naval-region-bake".into(),
                version: 1,
                keep_m: 40_000,
                candidate_m: 65_000,
                pole_limit_deg: POLE_LIMIT_DEG,
            },
            snapshot: Some(Snapshot {
                source: "water-polygons-split-4326.zip".into(),
                date: "2026-10-01".into(),
                sha256: Digest::of(b"snapshot"),
                bytes: 8,
            }),
            sources: Some(FileRef::of(b"sources")),
            square_table: FileRef::of(b"table"),
            synthetic: false,
        }
    }

    #[test]
    fn a_release_is_its_file() {
        let bytes = pinned().encode();
        let release = ReleaseFile::parse(&bytes).unwrap();
        assert_eq!(release.id, ReleaseId(Digest::of(&bytes)));
        assert_eq!(release.file, pinned());
        assert_eq!(release.file.encode(), bytes);
        assert!(release.file.square_table.matches(b"table"));
        assert!(!release.file.square_table.matches(b"tablE"));
    }

    #[test]
    fn an_incomplete_or_foreign_release_is_refused() {
        let mut unpinned = pinned();
        unpinned.sources = None;
        let mut synthetic_with_sources = pinned();
        synthetic_with_sources.synthetic = true;
        let mut other_lattice = pinned();
        other_lattice.lattice.per_degree = 1200;
        let mut bad_name = pinned();
        bad_name.name = "../x".into();
        for file in [unpinned, synthetic_with_sources, other_lattice, bad_name] {
            assert!(ReleaseFile::parse(&file.encode()).is_err(), "{file:?}");
        }
        let mut text = String::from_utf8(pinned().encode()).unwrap();
        text = text.replacen('{', "{\n  \"surprise\": 1,", 1);
        assert!(ReleaseFile::parse(text.as_bytes()).is_err());
    }
}
