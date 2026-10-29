# naval-land-bake

The land release bake's host: the network, the disk and the threads around `naval-region-bake` (which is pure), as a
library and the `naval-land-bake` command. It makes a release from its pinned sources and bakes the release's squares
into a store (`naval-land`), with nothing of the game's client or simulation
(`bun run cargo -- tree -e normal -p naval-land-bake` names only `naval-region-bake`, `naval-land` and
`naval-land-format` of this repository's crates). The game runs the same code on the user data directory:
`naval-client land-release make|bake` takes the same flags ([land.md](../../../docs/theatre/land.md#land-releases-land-v1-and-land-v2)).

## Baking squares

```sh
# From the repository root. --data is the folder the store (land/) and the sources (land-sources/) are under.
bun run cargo -- run -p naval-land-bake --profile dev-client -- --data "$PWD/.build/land" \
  bake --release "$PWD/assets/land/land-v2.release.json" N36W123
```

(`bun run cargo` runs Cargo in `native/`; plain `cargo run --manifest-path native/Cargo.toml …` does the same.)

- **What a bake reads.** The release file and its square table (`land-v2.squares.nlt`, beside it in `assets/land/`);
  its source manifest (`land-v2.sources.json`) beside it, in `<sources>/land-v2/`, or already in the store; and the
  coast snapshot `<sources>/snapshot/water-polygons-split-4326.zip`, the one the release pins by length and SHA-256
  (another is refused). The game downloads the manifest and the snapshot from its private repository (`hosted`); a
  machine that has the game's `land-sources/` can point `--sources` at it (only read when `--cache` is elsewhere).
  `make` writes all three of a release's files from the snapshot and the buckets' listings (`<sources>/land-v2/`).
- **The heights** come from the public Terrarium and Copernicus GLO-30 buckets, each object read by the ETag the
  manifest pins, and are kept in `<sources>/cache/` (16 GB, least recently used first; `--cache` puts it elsewhere and
  an empty one bakes cold). N36W123 from cold took about 24 MB in 401 requests and 10 to 13 s on `dev-client`.
- **What it writes.** A JSON line a square (`baked.line` is its manifest line: the square and its SHA-256, or
  `empty` for open sea) and the square's file in the store, `<data>/land/<release id>/<square>/<sha256>.nsq`. A square
  already recorded is baked again and must give its recorded bytes; `NAVAL_LAND_DIR` and `NAVAL_LAND_SOURCES_DIR`
  override the store's and the sources' folders, `--store` and `--sources` a bake's.

## Reading the squares

A host cuts a window of land from the store's files with `naval_terrain::release`, as `engine-example` does from the
synthetic release's squares made in code (its `src/coast.rs`, `naval_land_format::synthetic::square`). With a real
release the files come from the store (`naval-land`) instead:

```rust
let release = naval_land::format::ReleaseFile::parse(&std::fs::read(release_file)?)?; // its id
let store = naval_land::Store::open(data.join("land"), naval_land::DEFAULT_BOUND_BYTES)?;
let window = naval_terrain::release::ReleaseWindow::sea([lat, lon]); // the theatre's 96 km window
let files = window.squares().into_iter().filter_map(|key| match store.read(release.id, key) {
    Ok(naval_land::Answer::Land(lease)) => Some(lease.bytes().to_vec()),
    _ => None, // open sea, or not baked yet (`Answer::Missing`: bake it)
});
let land = naval_terrain::release::ReleaseLand::new(files).map_err(anyhow::Error::msg)?;
let field = window.heightfield(&land); // None when the window holds no land
```
