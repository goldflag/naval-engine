# Naval engine

The sky, atmosphere, clouds, ocean, terrain and combat effects from the naval game Blue Water, as Rust crates on
[wgpu](https://wgpu.rs): a physically based sky with the sun, moon and stars, volumetric clouds and weather, an FFT
ocean with foam and wakes, real-elevation land with trees, and gun blasts, splashes, fire and smoke; the round Earth
they stand on (latitude and longitude, the sun's and the moon's places) and the bake that turns the world's elevation
and coastline data into land. None of it needs the game's simulation; `engine-example` is a window that draws them all.

Exported from the game at <!-- exported-commit -->`eaf2835938c9824e862fe0d1cd51f83f5387aa0c`.

## Where changes go

This repository is an export. The crates live in the game's repository, and changes go there first; then
`tools/export-from-game.sh <game-checkout> [git-ref]` re-exports them here (from the committed files at the ref, default
`origin/master`) and makes the same trims again. The paths it takes are lists at the top of the script.

## The crates

| Crate | What it is |
| --- | --- |
| `naval-render` | The renderer core: passes into HDR and reversed-depth targets, cameras, shadows, bloom, the eye, AgX display; the `SceneLayer` trait every other layer implements |
| `naval-sky` | The atmosphere (precomputed scattering), the sun, moon and stars, and the environment lighting the other layers read |
| `naval-clouds` | Volumetric clouds and weather (`GameSky`, the sky with its weather), cloud shadows, rain and lightning |
| `naval-conditions` | A map, an hour, a weather and a wind resolved into what the sky, the sea, the fog and the land read (`MapSelection` to `MapScene`) |
| `naval-ocean` | The sea: wave spectrum, GPU FFT cascades, foam, the clipmap surface and its shading, wakes, the coast's shelter |
| `naval-terrain` | Land and trees from a heightfield, with ground materials, climate and rock; `naval_terrain::release` cuts a coast from a land release |
| `naval-effects` | Combat gas, fire and smoke (blasts, bursts, strikes, smoke screens), shells, blast waves |
| `naval-water-effects` | Shell splashes, the spray of gun blasts and bows, and their particles |
| `naval-land` | Land releases on disk: final squares kept by content, manifests, pins, a size bound |
| `naval-region-bake` | The land release bake: the world's 1-degree squares from elevation tiles, finished into a release |
| `naval-osm` | OpenStreetMap vector tiles: the MVT decoder, typed layers and web-mercator tile math |
| `naval-overture` | Overture Maps' infrastructure (GeoParquet): power lines, towers and masts over a box |
| `naval-land-bake` | The bake's host and the `naval-land-bake` command: makes a land release and bakes its squares into a store, fetching the elevation tiles over the network ([its README](native/crates/naval-land-bake/README.md)) |
| `naval-landscape` | The real world's look on the land, fetched from open sources, cached on disk and made on worker threads: Sentinel-2 colour and land cover (NAIP in the United States), OpenStreetMap's roads, railways and waters, finer relief, tree canopy, the horizon's land, and the buildings, ports, bridges, airfields, bases and power lines on it |
| `naval-globe` | The round Earth seen from space: an orthographic globe with the world chart's land, coasts, reefs, depth bands and graticule, drawn on the GPU |
| `engine-example` | A window with a fly camera over the sky, clouds, sea, a coast and a few effects: the template a host copies |
| `native-offscreen` | A headless device and PNG read-back, for stills and tests |
| `native-unify` | No code: one feature set for the shared dependencies, so every build compiles them once |
| `naval-sky-types` | The sky's plain types (cloud genera, the high sheet); no dependencies |
| `naval-cloud-field` | The clouds' weather data: the cloud layers, weathers' profiles, the local weather texture, the anchors' blend |
| `naval-land-format` | Land releases' formats (square keys, the lattice, the manifest, the release file) and a `synthetic` coastal world made in code |
| `naval-earth` | The round Earth over plain values: latitude and longitude, the sphere's mathematics and chart, the 1° squares, the Earth's lattices, the sun, the moon and sidereal time |

`native/crates/engine-example/README.md` says what each layer needs from its host, in the order the example feeds them.

## Layout

The game's directory layout is kept, so the crates' relative paths work unchanged.

| Path | Holds |
| --- | --- |
| `Cargo.toml`, `crates/` | The root workspace: the three portable crates with no GPU code |
| `native/Cargo.toml`, `native/crates/` | The native workspace (every crate under `native/crates`), its profiles and the wgpu-hal patch |
| `native/vendor/wgpu-hal` | wgpu-hal 30.0.1 with a larger DX12 compiled-shader cache (`native/vendor/README.md`) |
| `native/docs/` | The crates' docs; their links into the game's other docs are broken here |
| `assets/maps/`, `assets/bases/` | The maps, weather presets and naval bases, compiled into `naval-conditions` and `naval-terrain` |
| `assets/land/` | The game's land releases' files, read by `naval-region-bake`'s tests |
| `public/theatre/world/world-v1.bin` | The world chart (Natural Earth 10 m, public domain) a host hands `naval-globe`; `engine-example`'s globe reads it |
| `public/terrain/` | Ground photographs and materials a host hands `naval-terrain` (the crate reads no file) |
| `src/ui/tokens.css` | The game interface's colour tokens, read by `naval-render`'s palette test |
| `tools/export-from-game.sh` | The export |

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.98.1, edition 2024).

## Running the example

```sh
cd native
cargo run -p engine-example                                  # the window
cargo run -p engine-example -- --still ../.build/still.png   # one frame offscreen, written as a PNG
```

`--profile dev-client` (opt-level 1 for these crates) or `--release` makes the frames faster than the default dev
profile. In the window: WASD to fly, Q and E down and up, a mouse button held to look, Shift eight times as fast, the
wheel for the speed. T and G move the clock an hour on and back, B steps the weather (clear, partly cloudy, overcast,
fog, storm), N and M make the sea calmer and rougher, and F spawns a gun's blast, a fall of splashes, an HE burst, a
shell on the coast and a smoke screen. Escape quits; `--exit-after <seconds>` closes it by itself. `--still <out.png>`
renders the window's first view (700 m up, 2.5 km off the coast) in fixed conditions, writes it and exits; `--size
<w>x<h>` sets either's size. The coast is cut from the `synthetic` land release, so nothing is downloaded. O switches
the window to the globe and back (a drag turns it, the wheel zooms about the pointer); `--globe` starts there, and with
`--still` renders the whole Earth over the North Atlantic.

## Baking real land

`naval-land-bake` turns the world's elevation (the public Terrarium and Copernicus GLO-30 buckets, read over HTTPS) and
OpenStreetMap's coastline into a land release's squares, which `naval_terrain::release` cuts a coast from
([its README](native/crates/naval-land-bake/README.md)). A bake reads the coast snapshot and the source manifest its
release pins. The game's own are hosted on the game's private repository, so a clone of this one makes its own release
first, from a fresh coastline (`water-polygons-split-4326.zip` from osmdata.openstreetmap.de, ODbL):

```sh
mkdir -p .build/land/land-sources/snapshot   # from the repository root
mv <download>/water-polygons-split-4326.zip .build/land/land-sources/snapshot/
cd native
cargo run -p naval-land-bake --profile dev-client -- --data ../.build/land make   # the release, its table and manifest
cargo run -p naval-land-bake --profile dev-client -- --data ../.build/land \
  bake --release ../.build/land/land-sources/land-v2/land-v2.release.json N36W123
```

## Using the crates in another project

A git dependency on this repository, by crate name (Cargo finds each package in the repository):

```toml
[dependencies]
naval-render = { git = "https://github.com/goldflag/naval-engine" }
naval-sky = { git = "https://github.com/goldflag/naval-engine" }
naval-clouds = { git = "https://github.com/goldflag/naval-engine" }
naval-conditions = { git = "https://github.com/goldflag/naval-engine" }
naval-ocean = { git = "https://github.com/goldflag/naval-engine" }
naval-terrain = { git = "https://github.com/goldflag/naval-engine" }
naval-effects = { git = "https://github.com/goldflag/naval-engine" }
naval-water-effects = { git = "https://github.com/goldflag/naval-engine" }
naval-land-format = { git = "https://github.com/goldflag/naval-engine", features = ["synthetic"] }
naval-earth = { git = "https://github.com/goldflag/naval-engine" }
# The same majors as the crates' (native/Cargo.toml, [workspace.dependencies]): their types cross the boundary.
wgpu = "30"
winit = "0.30.13"
glam = "0.33"
```

Pin a `rev` for a reproducible build. For local work, path dependencies on a clone instead, such as
`naval-sky = { path = "<clone>/native/crates/naval-sky" }` (`crates/naval-land-format` for the root workspace's).

- **Workspace inheritance resolves here.** The crates' manifests say `version.workspace = true`, `wgpu.workspace = true`
  and the like; Cargo reads those from this repository's `native/Cargo.toml` (the root `Cargo.toml` for `crates/`),
  not from the consuming project.
- **Profiles and the patch do not carry over.** Only the consuming project's own `[profile]` and `[patch]` sections
  apply. Give the dev profile an opt-level of at least 1 for these crates (and 2 for dependencies), or the sea's and the
  clouds' CPU work is slow. The `[patch.crates-io]` of wgpu-hal matters only on Windows (DX12); a Windows project that
  wants it adds `wgpu-hal = { git = "https://github.com/goldflag/naval-engine" }` under its own `[patch.crates-io]`.
- **`native-unify`** comes along with every crate: the union of the features the game's builds ask of the shared
  dependencies, so those build with a few more features than a project would ask for on its own.
- The crates read no files: the host hands them bytes (the ground photographs in `public/terrain/`, a land release's
  squares). Shaders and the maps' JSON are compiled in.

## What the export leaves out

Everything else in the game, and the few tests and examples that need its simulation: `naval-water-effects`'
recorded-battle playback test and its `record` and `battle_sequence` examples. `tools/export-from-game.sh` lists each removal.
`naval-effects`' flipbook test skips without the game's flipbooks (Git LFS, not exported).

## Licences

The crates' own code has no licence yet: all rights reserved (the manifests say `UNLICENSED`). Third-party work in them
keeps its own licence, with the notices beside it:

- `native/crates/naval-sky/THIRD_PARTY.md`: the atmosphere, ported from takram's three-geospatial (MIT), itself based
  on Eric Bruneton's Precomputed Atmospheric Scattering (BSD 3-Clause); the stars, the Yale Bright Star Catalogue.
- `native/crates/naval-clouds/THIRD_PARTY.md` and `crates/naval-cloud-field/THIRD_PARTY.md`: the clouds, the cloud
  layers and the local weather texture, from takram's three-clouds (three-geospatial, MIT) and the works it builds on.
- `public/terrain/ASSETS.md`: the ground photographs and materials (Poly Haven, CC0), the climate map
  (`naval-terrain/src/earth/koppen.nkg`, Köppen-Geiger, CC BY 4.0) and the rock map (`glim.ngl`, GLiM, CC BY 3.0).
- `native/crates/naval-render/assets/smaa_*.zlib`: SMAA's area and search textures, as three.js embeds them (MIT).
- `native/vendor/wgpu-hal`: wgpu-hal (MIT or Apache-2.0, its `LICENSE.*` files).
- `public/theatre/world/world-v1.bin`: the world chart, from Natural Earth (public domain).

`naval-landscape` and `naval-land-bake` bundle no map data: they fetch it at run time from the services' own hosts
(EOX's Sentinel-2 cloudless, ESA WorldCover, the USGS NAIP, OpenFreeMap's OpenStreetMap tiles, Microsoft's and
OpenStreetMap's building footprints, Meta and WRI's canopy heights, Overture's power lines, the AWS Terrain Tiles and
Copernicus GLO-30). Each module's `ATTRIBUTION` constant is the credit its source asks for, and its doc comment links
the source's terms; a project that shows the land shows those credits.
