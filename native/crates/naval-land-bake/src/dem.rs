//! The network under the land release's bake (`release`; the bake's own crate is pure): GET and ranged GET over HTTPS,
//! an `If-Match` read of a pinned source, and files written whole. Only GET is ever sent, to the buckets a release's
//! sources name. No network answers `offline`, a server's error `failed`. (The DEM cache the theatres' legacy region
//! bake kept went with it, D-6.)

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// The largest range one read may ask for (a GLO-30 block is a few MiB compressed).
pub const MAX_READ_BYTES: u64 = 16 << 20;
const USER_AGENT: &str = concat!(
    "naval-native/",
    env!("CARGO_PKG_VERSION"),
    " (land release bake; sources cache on disk)"
);

/// Why a read was not answered with bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct DemError {
    /// `offline` (no network) or `failed` (the server's error, or the cache's).
    pub code: &'static str,
    pub message: String,
}

impl DemError {
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            code: "failed",
            message: message.into(),
        }
    }

    pub fn offline(message: impl Into<String>) -> Self {
        Self {
            code: "offline",
            message: message.into(),
        }
    }
}

impl std::fmt::Display for DemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// What the network answered.
#[derive(Clone, Debug, PartialEq)]
pub enum Got {
    Bytes(Vec<u8>),
    /// 403 or 404: no such file.
    Absent,
    /// 412: the object's ETag is not the one asked for (a pinned read, [`Transport::get_matching`]).
    Changed,
}

/// The network, behind a trait so the tests answer for it.
pub trait Transport: Send + Sync {
    /// A byte range (`Some((offset, length))`) or the whole file.
    fn get(&self, url: &str, range: Option<(u64, u64)>) -> Result<Got, DemError>;

    /// [`Self::get`] sending `If-Match: "<etag>"`: a land release's pinned read (`release::PinnedDem`). An object
    /// whose ETag is no longer `etag` answers [`Got::Changed`]. A transport that cannot send it refuses.
    fn get_matching(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        etag: &str,
    ) -> Result<Got, DemError> {
        let _ = (range, etag);
        Err(DemError::failed(format!(
            "{url}: this transport cannot send If-Match"
        )))
    }
}

/// HTTPS through ureq (rustls), as the theatre's surface streaming fetches.
pub struct Https;

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .max_idle_connections_per_host(16)
            .build()
            .into()
    })
}

impl Transport for Https {
    fn get(&self, url: &str, range: Option<(u64, u64)>) -> Result<Got, DemError> {
        https_get(url, range, None)
    }

    fn get_matching(
        &self,
        url: &str,
        range: Option<(u64, u64)>,
        etag: &str,
    ) -> Result<Got, DemError> {
        https_get(url, range, Some(etag))
    }
}

fn https_get(url: &str, range: Option<(u64, u64)>, etag: Option<&str>) -> Result<Got, DemError> {
    let mut request = agent().get(url);
    if let Some(etag) = etag {
        request = request.header("If-Match", format!("\"{etag}\""));
    }
    if let Some((offset, length)) = range {
        request = request.header("Range", format!("bytes={offset}-{}", offset + length - 1));
    }
    let mut response = request.call().map_err(|e| match e {
        ureq::Error::Io(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Timeout(_) => DemError::offline(format!("{url}: {e}")),
        other => DemError::failed(format!("{url}: {other}")),
    })?;
    let limit = range.map_or(64 << 20, |(_, length)| length + 1);
    match (response.status().as_u16(), range) {
        (206, Some(_)) | (200, None) => response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map(Got::Bytes)
            .map_err(|e| DemError::failed(format!("{url}: {e}"))),
        // The whole file in place of a range: never read (a GLO-30 file is up to 50 MB).
        (200, Some(_)) => Err(DemError::failed(format!(
            "{url}: the range was not honoured"
        ))),
        (403 | 404, _) => Ok(Got::Absent),
        (412, _) if etag.is_some() => Ok(Got::Changed),
        (status, _) => Err(DemError::failed(format!("{url}: HTTP {status}"))),
    }
}

/// Written whole, then moved into place: a reader never sees half a file.
pub fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(
        ".{}.{:?}.part",
        std::process::id(),
        std::thread::current().id()
    ));
    let temp = PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}
