//! The bake's commands: making a land release from its pinned sources, and baking its squares into the store. The
//! `naval-land-bake` binary runs them on the data directory it is given (src/main.rs); `naval-client land-release`
//! runs them on the game's, beside its own commands that play a scenario on a release.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::json;

#[derive(clap::Subcommand, Debug)]
pub enum Command {
    /// Make `land-v<bake>` from the snapshot: the square table (one pass over it), the source manifest (listed from the
    /// buckets, nothing downloaded) and the release file, written to `--out` and installed in the store.
    Make {
        /// The snapshot (the sources folder's `snapshot/water-polygons-split-4326.zip` by default).
        #[arg(long)]
        snapshot: Option<PathBuf>,
        /// Where to write the release's files (`<sources>/land-v<bake>/` by default).
        #[arg(long)]
        out: Option<PathBuf>,
        /// The bake the release is made for: 2 (`land-v2`, a real sea floor) by default, 1 for `land-v1`'s.
        #[arg(long, default_value_t = naval_region_bake::finish::VERSION)]
        bake: u32,
        /// Listing requests in flight.
        #[arg(long)]
        jobs: Option<usize>,
    },
    /// Bake squares of a release into the store, each timed; a square already recorded is baked again and must give
    /// its recorded bytes.
    Bake {
        /// The release file; its square table and source manifest beside it (`<name>.squares.nlt`,
        /// `<name>.sources.json`), in `<sources>/<name>/`, or in the store.
        #[arg(long)]
        release: PathBuf,
        /// The store's folder (`<data>/land` by default).
        #[arg(long)]
        store: Option<PathBuf>,
        /// The sources folder (`<data>/land-sources` by default): its snapshot, and its cache unless `--cache`.
        #[arg(long)]
        sources: Option<PathBuf>,
        /// The source cache's folder (`<sources>/cache` by default); an empty one bakes cold.
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        jobs: Option<usize>,
        /// Squares by name (`N36W123`).
        squares: Vec<String>,
    },
}

/// Runs `command` on the data directory `data` (`<data>/land`, `<data>/land-sources`; `NAVAL_LAND_DIR` and
/// `NAVAL_LAND_SOURCES_DIR` override either).
pub fn run(command: Command, data: &Path) -> Result<()> {
    use crate::release::{
        DEFAULT_SOURCE_BOUND, ReleaseSources, SourceCache, bake_square, make_release,
        snapshot_path, sources_dir, store_dir,
    };
    use naval_land::format::{ReleaseFile, SquareKey};
    let failed = |e: crate::BakeFailure| anyhow!("{e}");
    match command {
        Command::Make {
            snapshot,
            out,
            bake,
            jobs,
        } => {
            let sources = sources_dir(data);
            let snapshot = snapshot.unwrap_or_else(|| snapshot_path(&sources));
            let made = make_release(
                &snapshot,
                &crate::dem::Https,
                jobs.unwrap_or(16),
                &|line: &str| eprintln!("  {line}"),
                bake,
            )
            .map_err(failed)?;
            let name = &naval_region_bake::release::name(bake);
            let out = out.unwrap_or_else(|| sources.join(name));
            std::fs::create_dir_all(&out)?;
            for (file, bytes) in [
                (format!("{name}.release.json"), &made.release),
                (format!("{name}.squares.nlt"), &made.table),
                (format!("{name}.sources.json"), &made.sources),
            ] {
                std::fs::write(out.join(&file), bytes)?;
            }
            let store = naval_land::Store::open(store_dir(data), naval_land::DEFAULT_BOUND_BYTES)?;
            store.install(&made.release, &made.table, Some(&made.sources))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "release": name,
                    "id": made.id.to_string(),
                    "out": out.display().to_string(),
                    "stats": made.stats,
                }))?
            );
            Ok(())
        }
        Command::Bake {
            release,
            store,
            sources,
            cache,
            jobs,
            squares,
        } => {
            let bytes =
                std::fs::read(&release).with_context(|| format!("{}", release.display()))?;
            let parsed =
                ReleaseFile::parse(&bytes).map_err(|e| anyhow!("{}: {e}", release.display()))?;
            let name = parsed.file.name.clone();
            let sources = sources.unwrap_or_else(|| sources_dir(data));
            let store = naval_land::Store::open(
                store.unwrap_or_else(|| store_dir(data)),
                naval_land::DEFAULT_BOUND_BYTES,
            )?;
            let record = match store.record(parsed.id) {
                Ok(record) => record,
                Err(_) => {
                    let beside = release.parent().unwrap_or(Path::new("."));
                    let find = |file: String| -> Result<Vec<u8>> {
                        for dir in [beside.to_path_buf(), sources.join(&name)] {
                            if let Ok(b) = std::fs::read(dir.join(&file)) {
                                return Ok(b);
                            }
                        }
                        bail!(
                            "{file}: not beside the release nor in {}",
                            sources.join(&name).display()
                        )
                    };
                    let table = find(format!("{name}.squares.nlt"))?;
                    let manifest = find(format!("{name}.sources.json"))?;
                    store.install(&bytes, &table, Some(&manifest))?;
                    store.record(parsed.id)?
                }
            };
            let cache = Arc::new(
                SourceCache::open(
                    cache.unwrap_or_else(|| sources.join("cache")),
                    DEFAULT_SOURCE_BOUND,
                )
                .map_err(failed)?,
            );
            let net: Arc<dyn crate::dem::Transport> = Arc::new(crate::dem::Https);
            let at = ReleaseSources::open(&record, snapshot_path(&sources), cache.clone(), net)
                .map_err(failed)?;
            let threads = jobs.unwrap_or_else(crate::pool_threads);
            let cancel = AtomicBool::new(false);
            for square in squares {
                let key = SquareKey::parse(&square)
                    .ok_or_else(|| anyhow!("{square} is not a square's name"))?;
                let before = (
                    at.dem.downloaded.load(std::sync::atomic::Ordering::Relaxed),
                    at.dem.requests.load(std::sync::atomic::Ordering::Relaxed),
                );
                let baked = bake_square(&store, &at, key, threads, &cancel).map_err(failed)?;
                let stats = at.stats.lock().unwrap().clone();
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "baked": baked,
                        "downloadedBytes": at.dem.downloaded.load(std::sync::atomic::Ordering::Relaxed) - before.0,
                        "requests": at.dem.requests.load(std::sync::atomic::Ordering::Relaxed) - before.1,
                        "sources": stats,
                    }))?
                );
            }
            let (files, bytes) = cache.trim();
            let (held, size) = cache.usage();
            eprintln!(
                "  source cache: {held} files, {size} bytes ({files} files, {bytes} bytes evicted)"
            );
            Ok(())
        }
    }
}
