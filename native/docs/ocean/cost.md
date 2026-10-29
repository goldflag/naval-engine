# The sea's GPU cost

Index: [README.md](README.md). Each subject page measures its own feature's cost.

## The water pass on High

Measured with `bun native/tools/water-cost.ts` (a duel held still, five views, the ocean on and off) and an A/B of
shader variants swapped in one client through hot reload, on Windows (RTX 5070 Ti, Vulkan, 2560 × 1440, High). Another
client shared the GPU throughout, and its time slices land inside our passes and inflate the medians, so the numbers here
are each view's fastest frames (min/p10), which held steady from run to run.

- **Where it went**: the water pass took 1.0–1.8 ms. A flat-coloured sea costs 0.12 ms, so nearly all of it is the
  fragment shader. Screen-space reflections were about half of that: 0.37 ms on the bridge view with no ship in sight,
  0.9 ms overhead. Every other part costs 0.02–0.12 ms on its own (field reads, sky lobe, foam, wake, groups, gusts,
  fog). The shader is register-bound: removing code that never runs still saves about 0.1 ms.
- **Reflections trace only rays that can meet something** (`src/reflection_tiles.rs`, `shaders/reflection_tiles.wgsl`,
  `SceneLayer::after_copies`): after the opaque copies a compute pass marks the depth copy's 16-pixel tiles holding any
  surface and sums them into a table in a storage buffer (the fragment stage already binds WebGPU's 16 sampled
  textures). A ray whose clipped screen box holds no surface returns at once with what its march would have found
  (nothing). Only surfaces a ray can reach are marked (`reach`, since 2026-10-09): nearer than the drawn sea's edge (a
  20 m crest's distance past its horizon, where the surface closes its rows up) and the longest ray beyond, and never
  past a battle view's far plane (`REACH_M`, 60 km); a uniform holds that depth for the frame's camera, 0 when the far
  plane is no farther. A theatre view drawing the land beyond its window (`native/docs/terrain/layer.md`, "The horizon ring") had that
  land's band along the horizon in every ray's box from the bridge, and each was marched for nothing (the water pass
  +0.4-0.9 ms from a ship's bridge). Rays whose camera-facing trust is 0 are not traced, the march reads its depths
  four at a time, and the bisection reads two halvings' points at once. The picture is bit for bit the same in every
  view, and `check.ts` gives the same numbers as master on every check; only from high up is the ring's land past 60 km
  left unmirrored, as a battle view drew nothing there.
- **Sky reads share their work** (naval-sky's `environment.wgsl`, `reflection.wgsl`): one face lookup per direction for both
  blended levels, one roughness level for the lobe's three taps. The same up to rounding (a few pixels per million move
  a few levels).

| View | Water pass before | After |
| --- | --- | --- |
| chase | 1.57 ms | 1.29 ms |
| bridge | 1.00 | 0.75 |
| close | 1.50 | 1.23 |
| overhead | 1.83 | 1.15 |
| low | 0.94 | 0.70 |

Tried and left out: passing over each empty four-step batch inside a traced ray (about 1 %). Still open: reflections
cost about 0.55 ms in the chase and close views, for rays crossing the hull's tiles in front of it (a nearest-depth bound
per tile could pass over those), and the register pressure (splitting the shading into passes).

**On Apple's GPU (M5 Pro, Metal, 2592 × 1545, High at full scale, 2026-10-03, load 2–4).** The sea costs far more here
than on the RTX: 3.8–6.1 ms of a 9–13 ms frame by `water-cost.ts` (chase 6.07, bridge 4.21, low 3.75, close 5.14,
overhead 6.00), the water and sky pass 4.4–6.2 ms with the sea and 0.8–1.3 without. Shader variants swapped in one
client through hot reload (the fragment cut short at each stage, the compiler dropping everything after; overhead,
chase and bridge views, ms of that pass):

| Stage | Overhead | Chase | Bridge |
| --- | --- | --- | --- |
| A flat colour (raster, vertex, the sky's share of the pass) | 1.40 | 1.50 | 1.53 |
| + the wake | 0.19 | 0.42 | 0.40 |
| + the waves (three cascades) | 1.13 | 0.99 | 0.74 |
| + shadows, cloud shadow, rain, the lobe, the shelter | 0.47 | 0.28 | 0.30 |
| + the water body | 0.13 | 0.00 | 0.33 |
| + the sky lobe | 1.08 | 0.58 | 0.48 |
| + screen-space reflections | 1.40 | 1.74 | 1.18 |
| + glint and crests | 0.21 | 0.10 | 0.11 |
| + foam | 1.09 | 0.72 | 0.67 |

- **Reflections cost about 1 ms by being compiled in**: turned off at run time they still cost 0.9–1.4 ms of their
  1.1–2.0 (compiled out). Not the batched march (one read a step and one halving a round: within ±0.1), not where the
  trace sits in the shader (traced last, after the foam: +0.05–0.23), not unrolling (run-time loop bounds: ±0.15), not
  the tile table's storage reads (left out: no change). Most likely the register allocation of the whole shader; there is
  no Xcode here to read its pipeline statistics.
- **Not the cost**: anisotropic field reads (isotropic at the same level: 0 to −0.28), the bubble cloud's stretched read
  (~0). The sky lobe's three taps against one: 0.3 ms.
- **No look-neutral win found**; the sea's cost now falls with the render scale, which every preset below Ultra
  lowers (temporal upscaling, `aa-tracers.md`): at High's 70 % the water and sky pass is 3.9 ms (reflections about 0.7 of
  it, 0.37 of that by presence). The A/B: `bun native/tools/shader-ab.ts <variants.ts>` (variants as find-and-replace
  edits, swapped through hot reload in one client, each view's pass timed).
