# The render core and harness

Crates: `native/crates/naval-render` (the renderer core: passes and targets, cameras, the shader library, the display
transform, post and shadows) and `native/crates/naval-harness` (its own harness: shot, view, serve). Outputs go to
`.build/native/render-core/`.

| Question | Page |
| --- | --- |
| What does the core give a frame (targets, camera, shader library, registries, display transform)? How do I run the harness, its debug RPC and the image test? | This page |
| How does a layer join the frame: `SceneLayer` and phases, pipelines and hot reload, scene copies, a second view, work on other threads, the Earth's curvature? | [frame.md](frame.md) |
| Sun shadows and contact shadows? | [shadows.md](shadows.md) |
| Bloom, eye adaptation, exposure and debug views, temporal anti-aliasing, HDR output and the dither, photo mode's post, GPU timings? | [post.md](post.md) |
| Edge smoothing presets, render scale, shell tracers? | [aa-tracers.md](../aa-tracers.md) |
| The aerial perspective over everything drawn? | [haze.md](../haze.md) |

History (the lanes' build notes and measurements, the slice's follow-ups, removed effects):
[native-render-core-history.md](../../../docs/archive/native-render-core-history.md).

## What works

- **A frame of explicit passes over owned targets.** Layers prepare, then one scene pass draws into
  `scene.hdr` (`Rgba16Float`, linear radiance, cleared to black) with `scene.depth` (`Depth32Float`,
  reversed: cleared to 0, tested `GreaterEqual`), then the display pass writes the output. The scene
  targets follow the output's size.
- **The camera matches the web's `Game.ts` (historical).** `THREE.PerspectiveCamera(52, aspect, 0.5, 60000)` with
  `reversedDepthBuffer: true`: a finite far plane, depth 1 at the near plane and 0 at the far plane,
  the same matrix as three's `makePerspective(..., reversedDepth = true)` (tested). Defaults are the
  battle's (near 0.5 m, far 60 km); port and air-map values are the caller's to set.
- **Camera uniforms** at group 0 binding 0 of every scene pipeline: view, projection, view-projection
  and their inverses, position, time, dt, frame, viewport, near, far, tan(fov/2), aspect.
- **A shader library** on `naga_oil`, with shared modules any crate imports (table below), sources
  embedded with `include_str!` so it works on the web.
- **A pipeline registry** that knows each pipeline's entry shader, defines and recipe, rebuilds
  exactly the pipelines a changed file reaches (through its imports) and keeps the last good pipeline
  when a rebuild fails.
- **A resource registry**: labelled textures and buffers with a lifetime (`Persistent`, `Viewport`
  remade on resize, `Frame` dropped at the end of the frame) and a `version` that changes when an
  entry is remade, so bind groups on it know to rebuild.
- **The display transform**: `displayGrade` with `DISPLAY_LOOK` (exposure 0.9, contrast 1.25,
  saturation 1: the web's 1.25 was dropped on 2026-10-04, when a graded comparison found the extra saturation turned
  the sea a postcard navy and contrast alone read most like a photograph) and three.js r185's `agxToneMapping`, line for line, onto an sRGB target (or
  three's sRGB transfer in the shader for a target that is not sRGB). A GPU test renders 32
  radiances from black to 100 through the real pass and matches a CPU transcription within one
  8-bit level on both paths. An 8- or 10-bit output is dithered, and a float output can be drawn
  for an HDR display's headroom (section "HDR output and the output dither").
- **A test scene**: a ruled sea plane (10/100/1000 m lines, X axis red, Z axis blue, distance haze),
  an 18 × 12 × 60 m box lit by a low sun and sky ambient that turns 0.1 rad/s from a seed-chosen
  heading, and a sky gradient at the far plane drawn in the Sky phase.
- **Shader hot reload** (the client's and harness's `hot-reload` feature, on by default): WGSL files are polled
  on disk, the affected pipelines rebuild, and a file that does not compile keeps the last good
  pipeline and reports the naga error with a source snippet.
- **The harness**: `shot` (deterministic offscreen PNG), `view` (a window with an orbit camera),
  `serve` (headless), and the debug RPC v0 served by `view` and `serve`, with a Bun client.

## Running it

From `native/` with `~/.cargo/bin` on `PATH`:

```sh
cargo run -p naval-harness -- shot --scene test --seed 1 --frames 60 --dt 0.0166667 \
  --size 1920x1080 --out .build/native/render-core/test.png   # --camera broadside | 150,12,300
cargo run -p naval-harness -- view                 # RPC on 127.0.0.1:47800; --port 0, --no-rpc, --no-vsync
cargo run -p naval-harness -- serve --port 0       # headless; prints {"listening":...,"port":N} first
bun tools/rpc-client.ts --spawn                    # spawn serve, load, step 60, capture, telemetry, quit
bun tools/rpc-client.ts --port 47800 'set_camera {"preset":"broadside"}' 'capture {"path":".build/native/render-core/side.png"}'
cargo test -p naval-render -p naval-harness
cargo check -p naval-render --target wasm32-unknown-unknown
```

A relative `--out` or capture path is taken from the repository root, whatever the working
directory. `shot` prints one JSON line with the absolute path, frame, time and adapter.

Frame N of a shot is at N × dt seconds, computed from the index, so a frame depends only on the
scene, seed, camera, size and clock: two runs, or 60 frames stepped as 10 + 20 + 30, give
identical bytes, and the release and debug builds agree.

`view` controls: left-drag orbits, right- or middle-drag pans, scroll zooms, Space toggles the
live clock, `.` steps one frame, R resets the camera, S saves a capture, Esc quits. An RPC `step`
or `load_scene` pauses the live clock so the requested frame stays on screen.

## Harness graphics settings and scenes

`shot`, `view` and `serve` take `--graphics low|medium|high|ultra` (the web's presets for what is
ported: shadows off/low/medium/high, bloom off/on/on/on, eye adaptation on) and per-setting
overrides `--shadows`, `--bloom on|off`, `--eye-adaptation on|off`, `--antialiasing none|fxaa|smaa|taa` and
`--render-scale` (aa-tracers lane). The default is the web's High preset, FXAA included (SMAA on Ultra). The RPC's
`set_graphics {"preset"?, "shadows"?, "bloom"?, "eye_adaptation"?, "antialiasing"?,
"scene_copies"?}` changes them live (unnamed fields keep their value; `scene_copies` is a
debugging field that forces copies to measure them). `telemetry` gained `graphics`, `shadows`,
`gpu` and `exposure`; `capture` gained `snap` (default true: the eye settles on the view first).

Scenes: `test` and `sky` (the test scene's grid and box under naval-sky's real sky, the sun where
the test scene lights from, the eye metering the sky's environment on the GPU).

```sh
cargo run -p naval-harness --release -- shot --scene sky --camera 280,4,160 --out .build/native/render-core-2/sky-sun.png
cargo run -p naval-harness --release -- shot --antialiasing taa --camera 250,30,70 --out .build/native/render-core-2/taa.png
bun tools/rpc-client.ts --spawn 'set_graphics {"antialiasing":"taa"}' 'measure_frames {"frames":120}'
cargo run -p naval-harness --release --example timestamp_probe
```

## Debug RPC v0

JSON lines over TCP on 127.0.0.1 (default port 47800; `--port 0` picks one and the first stdout line
says which). Chosen over WebSocket because a client is a socket and a line splitter in any language
(`nc`, Bun's `Bun.connect`, Rust's `TcpStream`), with no handshake, framing or dependency; nothing
needs a browser to connect. Requests on one connection are answered in order; connections are served
one at a time.

```text
→ {"id": 1, "cmd": "step", "frames": 60}
← {"id": 1, "ok": true, "result": {"scene": "test", "seed": 1, "frame": 60, "time": 1.0, "dt": 0.0166…}}
← {"id": 2, "ok": false, "error": "unknown command \"warp\"; commands: version, ready, …"}
```

| Command | Parameters | Browser hook |
| --- | --- | --- |
| `version` | | protocol name and version, commands, scenes, adapter |
| `ready` | | `review.ready`, `review.stage` |
| `load_scene` | `scene` (`test`, `sky`), `seed` | loads a scene; clock to frame 0, camera to the scene's, the eye settled |
| `step` | `frames` (1), `dt`? | advances the manual clock, rendering every frame |
| `set_camera` | `preset` \| `eye` \| `azimuth`+`elevation`[+`distance`], `target`?, `fov`?, `near`?, `far`? | `review.placeCamera`, same presets and conventions as the web's `scripts/browser/cameraPoses.ts` (historical) |
| `capture` | `path`?, `snap`? (true) | `review.capture`: the current frame to a PNG (the clock does not move), the eye adaptation settled on the view unless `snap` is false |
| `telemetry` | | clock, camera, size, CPU frame timings, adapter, resources, pipelines and their errors, last hot reload, display settings, graphics, shadow maps, the latest frame's GPU pass timings, the eye adaptation's state and sky mean |
| `measure_frames` | `frames` (60) | `review.measureFrames`: renders the current frame N times, waiting for the GPU after each; CPU and whole-frame median, p90, max, and per-pass GPU serial cost and span (`gpu`) |
| `resize` | `width`, `height` | the offscreen capture size (`serve`) |
| `set_graphics` | `preset`?, `shadows`?, `bloom`?, `eye_adaptation`?, `antialiasing`?, `scene_copies`? | graphics settings by the web's names; unnamed fields keep their value (render core II) |
| `quit` | | stops `serve`, closes `view` |

Unknown commands and unknown parameters are refused with the list of what is accepted. The version
stays 0 while commands are only added; a change to an existing command's meaning bumps it.

`native/tools/rpc-client.ts` exports `NativeHarness` (`spawn`, `connect`, `call`, `request`, `close`)
and runs command lists from the command line.

## The image test

`cargo test -p naval-harness --test all -- image::` renders the test scene at 256 × 144, frame 60, seed 1, and
checks it two ways:

- **Against `crates/naval-harness/tests/reference/test-scene-256x144.png`** within a tolerance: mean
  absolute error at most 1 level over all channels, and at most 0.5 % of pixels off by more than
  24 levels. That absorbs edge rasterization and derivative (`fwidth`) differences between GPUs but not
  a changed shader, camera or pass. On failure the render and a ×4 difference image go to
  `.build/native/render-core/image-test/`. `NAVAL_UPDATE_REFERENCE=1` rewrites the reference after an
  intended change; look at it before committing.
- **By statistics** any correct render has on any adapter: a bright, blue-leaning sky across the top,
  a dark sea across the bottom, the sunlit starboard side brighter than the shaded bow and both
  brighter than the sea, every pixel opaque. When the reference fails, these say what broke.

The same file checks determinism (two sessions, batched steps, a later frame and another seed).
