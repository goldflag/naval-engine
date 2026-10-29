//! The land releases' pinned sources where the game downloads them (docs/theatre/land.md, "Where the sources are
//! hosted"): the coast snapshot and each release's source manifest, kept as the assets of one release of the game's
//! private repository ([`REPO`], tag [`TAG`]), each named by its SHA-256 (`<sha256>.zip`, `<sha256>.json`).
//!
//! - **When.** The first time one is needed: a release's manifest when the store installs the release
//!   (`land_release`), the snapshot at the first bake (`ReleaseSources::check_snapshot`). Kept after, where the bake
//!   reads it (`<data>/land-sources`), and never fetched again while it is there and right.
//! - **The login** ([`token`]): the player's GitHub login, `GITHUB_TOKEN` when set, else `gh auth token`. No token is
//!   carried in the game or the repository.
//! - **The pin** ([`fetch_pinned`]). A file streams to a `.part` beside its place while its SHA-256 is taken, and is
//!   moved into place only when its length and SHA-256 are the ones the release file pins; else it is deleted and the
//!   fetch refused. Nothing falls back to another host.
//! - **Refusals** ([`Unfetched`]) are a few words for the player (the reason a held world waits); the detail goes to the
//!   log.
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use naval_land::format::Digest;
use sha2::Digest as _;

/// The repository the sources are hosted on, and the release (its tag) whose assets they are.
pub const REPO: &str = "goldflag/ship-game";
pub const TAG: &str = "land-v2-sources";

/// Why a pinned source was not fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unfetched {
    /// No GitHub login on this computer.
    NoLogin,
    /// The login cannot see the game's repository.
    NoAccess,
    /// No connection (or the host did not answer).
    Offline(String),
    /// The host does not have it, or what came is not the file pinned.
    Refused(String),
}

impl fmt::Display for Unfetched {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Unfetched::NoLogin => NO_LOGIN,
            Unfetched::NoAccess => NO_ACCESS,
            Unfetched::Offline(_) => OFFLINE,
            Unfetched::Refused(_) => REFUSED,
        })
    }
}

impl std::error::Error for Unfetched {}

impl Unfetched {
    /// What went wrong in full, for the log.
    pub fn detail(&self) -> String {
        match self {
            Unfetched::NoLogin => format!("{NO_LOGIN}: set GITHUB_TOKEN or run gh auth login"),
            Unfetched::NoAccess => {
                format!("{NO_ACCESS}: the login cannot read {REPO}'s release {TAG}")
            }
            Unfetched::Offline(why) | Unfetched::Refused(why) => format!("{self}: {why}"),
        }
    }
}

/// The words for each refusal.
pub const NO_LOGIN: &str = "Land download: no GitHub login";
pub const NO_ACCESS: &str = "Land download: GitHub access denied";
pub const OFFLINE: &str = "Land download: offline";
pub const REFUSED: &str = "Land download: check failed";

/// Where a hosted file comes from: [`GitHub`] in play, a stand-in in the tests.
pub trait Host: Send + Sync {
    /// Writes the file `name` into `out`.
    fn get(&self, name: &str, out: &mut dyn Write) -> Result<(), Unfetched>;
}

/// The hosted file of a pinned source: its SHA-256 and its extension.
pub fn hosted_name(sha256: &Digest, extension: &str) -> String {
    format!("{}.{extension}", sha256.hex())
}

/// The fetches under way in this process, one a place: a second fetch of the same file (two releases share the
/// snapshot) waits for the first and takes what it kept.
fn fetching(dest: &Path) -> Arc<Mutex<()>> {
    static FETCHING: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    FETCHING
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(dest.to_path_buf())
        .or_default()
        .clone()
}

/// `dest`, fetched from `host` as `name` when it is not there, and checked against its pin (`sha256`, `bytes`) before it
/// is moved into place. A file already at `dest` is taken as it is (its reader checks it).
pub fn fetch_pinned(
    host: &dyn Host,
    name: &str,
    sha256: &Digest,
    bytes: u64,
    dest: &Path,
) -> Result<(), Unfetched> {
    if dest.exists() {
        return Ok(());
    }
    let place = fetching(dest);
    let _one = place.lock().unwrap_or_else(|e| e.into_inner());
    if dest.exists() {
        return Ok(());
    }
    let io = |e: std::io::Error| Unfetched::Refused(format!("{}: {e}", dest.display()));
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    // A part of this fetch's own (another process may be fetching the same file): the first to finish moves its own,
    // checked, into place.
    let part = part_of(dest);
    let mut out = Pinned {
        file: File::create(&part).map_err(io)?,
        hasher: sha2::Sha256::new(),
        bytes: 0,
        most: bytes,
    };
    eprintln!(
        "naval-client: downloading {name} ({} MB) from {REPO}'s release {TAG}",
        bytes >> 20
    );
    let got = host.get(name, &mut out).and_then(|()| {
        out.file.flush().map_err(io)?;
        let digest = Digest(out.hasher.clone().finalize().into());
        if out.bytes != bytes || digest != *sha256 {
            return Err(Unfetched::Refused(format!(
                "{name}: {} bytes with SHA-256 {}, where its release pins {bytes} bytes with {}",
                out.bytes,
                digest.hex(),
                sha256.hex()
            )));
        }
        Ok(())
    });
    drop(out);
    match got {
        Ok(()) => std::fs::rename(&part, dest).map_err(io),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            eprintln!("naval-client: {name}: {}", e.detail());
            Err(e)
        }
    }
}

/// The part a file is fetched into beside its place: this fetch's own.
fn part_of(dest: &Path) -> PathBuf {
    static FETCHES: AtomicU64 = AtomicU64::new(0);
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}-{}.part",
        std::process::id(),
        FETCHES.fetch_add(1, Ordering::Relaxed)
    ));
    dest.with_file_name(name)
}

/// A fetch's file, its SHA-256 taken as it is written; never more than its pin's length (and one byte, to tell).
struct Pinned {
    file: File,
    hasher: sha2::Sha256,
    bytes: u64,
    most: u64,
}

impl Write for Pinned {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.bytes + buf.len() as u64 > self.most + 1 {
            return Err(std::io::Error::other("longer than its pin"));
        }
        let n = self.file.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// The player's GitHub login: `GITHUB_TOKEN` when set, else what `gh auth token` answers (`gh` on the path, or where
/// Homebrew puts it: an app opened from the Finder has a short path).
pub fn token() -> Option<String> {
    if let Some(token) = std::env::var("GITHUB_TOKEN")
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    {
        return Some(token);
    }
    ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"]
        .into_iter()
        .find_map(|gh| {
            let out = std::process::Command::new(gh)
                .args(["auth", "token"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
                .ok()?;
            let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
            (out.status.success() && !token.is_empty()).then_some(token)
        })
}

/// The release's assets on GitHub, read with the player's login.
pub struct GitHub {
    pub token: String,
}

impl GitHub {
    /// With this computer's login; none without one.
    pub fn signed_in() -> Result<Self, Unfetched> {
        token()
            .map(|token| Self { token })
            .ok_or(Unfetched::NoLogin)
    }
}

/// The game's host: [`GitHub`] with this computer's login, looked for when a file is fetched (never before one is
/// needed).
pub struct Hosted;

impl Host for Hosted {
    fn get(&self, name: &str, out: &mut dyn Write) -> Result<(), Unfetched> {
        GitHub::signed_in()?.get(name, out)
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(concat!("naval-client/", env!("CARGO_PKG_VERSION")))
            .timeout_connect(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            // The asset's bytes come from a signed address the API redirects to: the login is not sent there.
            .redirect_auth_headers(ureq::config::RedirectAuthHeaders::Never)
            .build()
            .into()
    })
}

fn offline(e: ureq::Error) -> Unfetched {
    Unfetched::Offline(e.to_string())
}

impl GitHub {
    fn api(&self, url: &str, accept: &str) -> Result<ureq::http::Response<ureq::Body>, Unfetched> {
        agent()
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
            .map_err(offline)
    }

    /// The asset's API address and length.
    fn asset(&self, name: &str) -> Result<(String, u64), Unfetched> {
        let url = format!("https://api.github.com/repos/{REPO}/releases/tags/{TAG}");
        let mut response = self.api(&url, "application/vnd.github+json")?;
        match response.status().as_u16() {
            200 => {}
            401 => return Err(Unfetched::NoLogin),
            403 | 404 => return Err(Unfetched::NoAccess),
            code => return Err(Unfetched::Offline(format!("{url}: HTTP {code}"))),
        }
        let text = response
            .body_mut()
            .with_config()
            .limit(4 << 20)
            .read_to_string()
            .map_err(offline)?;
        let release: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| Unfetched::Refused(format!("{url}: {e}")))?;
        release["assets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| Some((a["url"].as_str()?.to_owned(), a["size"].as_u64()?)))
            .ok_or_else(|| {
                Unfetched::Refused(format!("{name} is not hosted on {REPO}'s release {TAG}"))
            })
    }
}

impl Host for GitHub {
    fn get(&self, name: &str, out: &mut dyn Write) -> Result<(), Unfetched> {
        let (url, size) = self.asset(name)?;
        let mut response = self.api(&url, "application/octet-stream")?;
        match response.status().as_u16() {
            200 => {}
            401 => return Err(Unfetched::NoLogin),
            403 | 404 => return Err(Unfetched::NoAccess),
            code => return Err(Unfetched::Offline(format!("{name}: HTTP {code}"))),
        }
        let mut reader = response.body_mut().with_config().limit(size + 1).reader();
        let mut buffer = vec![0u8; 1 << 20];
        let mut done = 0u64;
        let mut told = 0u64;
        loop {
            let n = reader
                .read(&mut buffer)
                .map_err(|e| Unfetched::Offline(format!("{name}: {e}")))?;
            if n == 0 {
                return Ok(());
            }
            out.write_all(&buffer[..n])
                .map_err(|e| Unfetched::Refused(format!("{name}: {e}")))?;
            done += n as u64;
            if done - told >= 64 << 20 {
                told = done;
                eprintln!("naval-client: {name}: {} of {} MB", done >> 20, size >> 20);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host holding files by name.
    struct Shelf(Vec<(String, Vec<u8>)>);
    impl Host for Shelf {
        fn get(&self, name: &str, out: &mut dyn Write) -> Result<(), Unfetched> {
            let (_, bytes) = self
                .0
                .iter()
                .find(|(n, _)| n == name)
                .ok_or_else(|| Unfetched::Refused(format!("{name} is not hosted")))?;
            out.write_all(bytes)
                .map_err(|e| Unfetched::Refused(e.to_string()))
        }
    }

    #[test]
    fn a_source_is_kept_only_when_it_is_the_file_pinned() {
        let dir = std::env::temp_dir().join(format!("naval-hosted-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = b"the coast".to_vec();
        let pin = Digest::of(&good);
        let name = hosted_name(&pin, "zip");
        assert_eq!(name, format!("{}.zip", pin.hex()));
        // Another file under the pinned name: refused, nothing left behind.
        let forged = Shelf(vec![(name.clone(), b"another coast".to_vec())]);
        let dest = dir.join("snapshot").join("water.zip");
        let refused = fetch_pinned(&forged, &name, &pin, good.len() as u64, &dest).unwrap_err();
        assert!(matches!(refused, Unfetched::Refused(_)), "{refused:?}");
        assert!(!dest.exists());
        let parts = std::fs::read_dir(dest.parent().unwrap()).unwrap().count();
        assert_eq!(parts, 0, "a refused fetch leaves no part");
        // A file not hosted: refused.
        let empty = Shelf(vec![]);
        assert!(fetch_pinned(&empty, &name, &pin, good.len() as u64, &dest).is_err());
        // The file pinned: kept where the bake reads it.
        let shelf = Shelf(vec![(name.clone(), good.clone())]);
        fetch_pinned(&shelf, &name, &pin, good.len() as u64, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), good);
        // There already: not fetched again.
        fetch_pinned(&empty, &name, &pin, good.len() as u64, &dest).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A host that takes its time, counting what it is asked for.
    struct Slow(Vec<u8>, std::sync::atomic::AtomicUsize);
    impl Host for Slow {
        fn get(&self, _: &str, out: &mut dyn Write) -> Result<(), Unfetched> {
            self.1.fetch_add(1, Ordering::Relaxed);
            for chunk in self.0.chunks(4) {
                std::thread::sleep(Duration::from_millis(20));
                out.write_all(chunk)
                    .map_err(|e| Unfetched::Refused(e.to_string()))?;
            }
            Ok(())
        }
    }

    /// Two releases share the snapshot: two bakers asking for it at once fetch it once, and both take it whole (Codex's
    /// review: one part file for both, the second fetch truncating the first's).
    #[test]
    fn a_file_asked_for_twice_at_once_is_fetched_once() {
        let dir = std::env::temp_dir().join(format!("naval-hosted-twice-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = b"the coast, read slowly".to_vec();
        let pin = Digest::of(&good);
        let host = Arc::new(Slow(good.clone(), Default::default()));
        let dest = dir.join("snapshot").join("water.zip");
        let fetches: Vec<_> = (0..2)
            .map(|_| {
                let (host, dest) = (host.clone(), dest.clone());
                std::thread::spawn(move || {
                    fetch_pinned(host.as_ref(), "x.zip", &pin, good_len(), &dest)
                })
            })
            .collect();
        for fetch in fetches {
            fetch.join().unwrap().unwrap();
        }
        assert_eq!(host.1.load(Ordering::Relaxed), 1);
        assert_eq!(std::fs::read(&dest).unwrap(), good);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn good_len() -> u64 {
        b"the coast, read slowly".len() as u64
    }

    #[test]
    fn the_refusals_are_a_few_words_and_the_detail_is_logged() {
        assert_eq!(Unfetched::NoLogin.to_string(), NO_LOGIN);
        let offline = Unfetched::Offline("api.github.com: dns".into());
        assert_eq!(offline.to_string(), OFFLINE);
        assert!(offline.detail().ends_with("api.github.com: dns"));
        assert_eq!(Unfetched::Refused("a".into()).to_string(), REFUSED);
    }
}
