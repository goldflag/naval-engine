//! Blue Water patch (native/vendor/README.md): compiled DXIL kept on disk between launches.
//!
//! DXC compiling the generated HLSL was about two thirds of every scene load in a fresh process (24-34 s of a theatre's
//! 38-50 s on the RTX 5070 Ti machine, 2026-10-26), all of it the same work as the last launch's. The application
//! names a directory ([`set_shader_disk_cache`]); each compiled stage is stored there under a hash of everything
//! DXC's output depends on: the HLSL, its source name, entry point and target, the compile flags the device chooses
//! (debug information, 16-bit types), this crate's version and [`FORMAT`]. Only the statically linked DXC is cached,
//! whose version is this build's own; FXC and a `dxcompiler.dll` found at run time are not. A file is written to a
//! staging name and renamed into place, and carries its key's hashes and its payload's, so a damaged or foreign file is
//! compiled again instead. The hashes are std's `DefaultHasher`, which a Rust release may change: that only costs one
//! launch's compiles.

use alloc::{string::String, vec::Vec};
use core::hash::{Hash, Hasher};
use core::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Bump when the key or the file layout changes, or when the linked DXC (`mach-dxcompiler-rs`) changes and this crate's
/// version does not.
const FORMAT: u32 = 1;
const MAGIC: &[u8; 8] = b"BWDXIL\0\x01";

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Keeps compiled DXIL in `dir` (created as needed) from now on, for every DX12 device in the process. The first call
/// wins; without one nothing is read or written.
pub fn set_shader_disk_cache(dir: PathBuf) {
    let _ = DIR.set(dir);
}

/// One compiled stage's place on disk.
pub(super) struct Entry {
    path: PathBuf,
    key: [u64; 2],
}

impl Entry {
    /// The file for a stage with these inputs; `None` without a directory or when the compiler is not the linked DXC.
    pub(super) fn new(
        static_dxc: bool,
        source: &str,
        source_name: Option<&str>,
        entry_point: &str,
        full_stage: &str,
        flags: &str,
    ) -> Option<Self> {
        if !static_dxc {
            return None;
        }
        let dir = DIR.get()?;
        let hash = |seed: u64| {
            let mut hasher = std::hash::DefaultHasher::new();
            (seed, FORMAT, env!("CARGO_PKG_VERSION")).hash(&mut hasher);
            (source, source_name, entry_point, full_stage, flags).hash(&mut hasher);
            hasher.finish()
        };
        let key = [hash(0x6e61_7661_6c31), hash(0x6e61_7661_6c32)];
        Some(Self {
            path: dir.join(format!("{:016x}{:016x}.dxil", key[0], key[1])),
            key,
        })
    }

    /// The stored DXIL, when the file is whole and is this key's.
    pub(super) fn read(&self) -> Option<Vec<u8>> {
        let data = std::fs::read(&self.path).ok()?;
        let header = MAGIC.len() + 8 * 4;
        if data.len() < header || &data[..MAGIC.len()] != MAGIC {
            return None;
        }
        let word = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
        let at = MAGIC.len();
        let (key0, key1, len, sum) = (word(at), word(at + 8), word(at + 16), word(at + 24));
        let payload = &data[header..];
        if [key0, key1] != self.key || payload.len() as u64 != len || checksum(payload) != sum {
            return None;
        }
        Some(payload.to_vec())
    }

    /// Stores `dxil`; a failure only costs the next launch a compile.
    pub(super) fn write(&self, dxil: &[u8]) {
        static STAGING: AtomicU64 = AtomicU64::new(0);
        let Some(dir) = self.path.parent() else {
            return;
        };
        let mut data = Vec::with_capacity(MAGIC.len() + 32 + dxil.len());
        data.extend_from_slice(MAGIC);
        for word in [self.key[0], self.key[1], dxil.len() as u64, checksum(dxil)] {
            data.extend_from_slice(&word.to_le_bytes());
        }
        data.extend_from_slice(dxil);
        let mut staging: String = self.path.to_string_lossy().into_owned();
        staging += &alloc::format!(
            ".{}.{}.tmp",
            std::process::id(),
            STAGING.fetch_add(1, Ordering::Relaxed)
        );
        let written = std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&staging, &data))
            .and_then(|()| std::fs::rename(&staging, &self.path));
        if written.is_err() {
            let _ = std::fs::remove_file(&staging);
        }
    }
}

fn checksum(data: &[u8]) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish()
}
