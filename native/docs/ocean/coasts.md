# Coasts and regional water

The theatre's coasts (fetch, surf, the sea under time compression) and its geographic water optics. Index:
[README.md](README.md).

## Coasts, and the sea under time compression

Near land the theatre drew the open ocean's sea: the same swell and whitecaps ran up to every beach, lee or not, and
under time compression the waves ran at the world's speed (64 times as fast at 64×), so the land seemed to slide
through the water. Now:

- **The coast** (`naval_ocean::shore`; `waves.wgsl`'s `shore_at`, `surf`). The theatre builds a `ShoreMap` of the
  sea's land window on a thread of its own (`naval-client`'s `theatre/land/shore.rs`), again when the window is cut afresh
  or the wind turns 4°: the land's heights (40 m) and, on a 160 m grid, the open water upwind for the wind sea
  (Saville's effective fetch, seven bearings within 45°, cos²) and for the swell (±10°). A beach sample holds the
  sea's fetch that runs onto it, so the water between it and the last sea sample is not read as sheltered. Each
  cascade keeps the share of its amplitude the JONSWAP fetch law leaves at that fetch (`band_fetch_table`, tabled
  over 16 fetches from 50 m to 100 km): in a lee the long waves are not raised and the short ones stand. The swell
  is gone in the land's shadow and fills back in over 15 km. No wave stands higher than 0.42 of the depth. The far
  whitecap share takes the energy the cascades keep.
- **The surf.** Where the water is shallower than the arriving waves break in (their height ÷ 0.42), bores run in
  once a swell period at √(g·d); over the seabed's slope `s` one takes 2·√d / (s·√g) from depth `d` to the
  waterline, so the lines lie a shallow-water wavelength apart: several on a gentle beach, less than one on a steep
  one (a white band that runs in and drains). They are their own foam layer (`WaveSample::breakers`), drawn as
  their mean where the pixel is too large for the lines or their tear.
- **The clock.** The theatre draws its sea at no more than twice real time (`theatre::SEA_RATE_MOST`; the FFT and the
  swell's spread run on the ocean's own clock). Above that (`Ocean::hull_rate` beyond `time_scale`) the hulls are let
  go over half a second of wall time (`Ocean::racing`): the combat sines' energy moves into the spread, which is drawn
  everywhere, and the hulls ride the world's sea loose of the drawn one.

Hulls ride the simulation's sea, which knows no coast, so within a hull's coupling the swell is kept whole.

Measured on the "Japan" save (7 m/s toward +x, Hs 1.2 m): a beach 20 km downwind keeps a 43 km fetch to its waterline
(cascades 0.40/0.85/1.0, swell 0.90, 1.28 m arriving, surf inside 3 m of water: 13 m of a 1:4 shore); a beach in the
lee keeps 2.7 km and 0.2 km (0.0/0.12/0.80, swell 0.01, 0.14 m). The mean frame-to-frame change of the sea's pixels
(0–255, 48 frames at 60 Hz) at 64× fell from 10.2 to 2.8 from the sea and from 15.2 to 3.5 beside the surf (2.4 at
1×). The harness's `water` returns the coast at its `sample` (`coast`: depth, both fetches, scales, arriving) and
the clocks (`sea_rate`, `hull_rate`, `racing`).

Not done: the simulation's sea near land (a hull in a calm lee still rides the open swell, and draws it about her);
refraction (crests do not turn to the shore); caustics and spindrift take no coast; battle scenes have none.

## Geographic water

Theatre no longer inherits one North Atlantic water body everywhere. `naval-client`'s
`theatre/land/water.rs` selects representative offshore optics and an authored atlas of broad coastal waters, bays and
river plumes. An arbitrary editor region uses a smooth latitude-based offshore fallback, with a clear Mediterranean
override; latitude is a visual fallback, not a water-colour measurement. The atlas's positions, sizes and strengths
are approximate, not historical observations. It has no seasonal biology or moving sediment simulation.

`src/optics.rs` owns five profiles (`clear`, `oceanic`, `productive`, `sediment`, `organic`): absorption and
backscatter coefficients per metre, at approximately 620, 550 and 460 nm. `WaterField` holds a baseline and up to
twelve elliptical areas; broad waters blend first, local plumes last. Each has a uniform middle and a smooth outer
edge. The host projects geographic centres into the persistent scenario frame, then subtracts the render origin in
f64 before sending them to the GPU. The shader samples the coefficients at each water point, so different waters
can appear in one view and do not move with the camera. Distant atlas entries are culled in the scenario frame.

The surface, underwater composite and motes share `shaders/water_properties.wgsl`. Colour comes from the existing
sun/sky upwelling model; the seabed still supplies shallow-water colour. Geographic transparency uses a fixed
8:1 scattering/backscatter approximation. The underwater view retains its approximate illumination, adjusted by
the same optical reflectance ratio, and samples visible paths at their midpoint (capped at 100 m); this is suitable
for kilometre-scale blend widths, not detailed volume transport. Submerged hull lighting takes the water near the
camera. Weather changes light and waves without changing the geographic field. Developer map fixtures keep their
existing optics unless the harness's `water` command replaces them.

`bun native/tools/water-atlas.ts` captures all profiles with the same ship, camera, wind and noon lighting, a plume
transition, clear/murky underwater views and both built-in theatres. Outputs and diagnostics go to ignored
`.build/native/water-atlas/`; `--no-build` reuses the current client. It also checks that malformed areas leave the
live field unchanged and that water varies across a plume. Unit tests cover boundaries, invalid inputs, capacity,
theatre recentering and longitude wrapping; the ocean's shader composition test covers the shared GPU layout.
