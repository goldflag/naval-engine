# Frame hygiene

The native client's per-frame garbage and its rare long frames: heap allocations every frame, fresh pages every
frame, and the single frames the load lane (`docs/archive/native-load-results.md`, "What is left") saw take 30 ms in the scorch and 53–67 ms in the
effects step. Code in `naval-client` (the frame loop, `src/debug/alloc_count.rs`), `naval-render` (`src/upload.rs`, the frame
report, the GPU timer, bloom's labels), `naval-ships` (`src/surface.rs`, the pose, prepare, scorch, impact marks,
ensigns and labels), `naval-effects` (publishing), `naval-water-effects`, `naval-aircraft`, `naval-audio` and
`naval-ocean` (the wake trails). Tools: `naval-client`'s `alloc_sites` and `upload_faults` examples, `naval-ships`'
`strike_cost` example, `native/tools/load/ab.ts`. Outputs under `.build/native/frame-hygiene/` and
`.build/native/load/h2/`.

**Status (2026-09-30):** done. A steady frame of the 30-ship battle makes about 1,090 heap allocations and 340 KB on
the render thread (from 7,790 and 5.1 MB), nearly all of them wgpu's own; it faults 7–8 pages (p50; from 185); the
first-strike scorch measurement and the impact marks' surface search cost about 3 % of what they did, with the same
results bit for bit; and under the heaviest synthetic load native misses 2–5 refreshes in 30 s, from 37–40, with p99.9
frame time 32–34 ms (from 66–82) and no frame over 34 ms (from 84–150). Rendering is unchanged: `check.ts` passes with
nothing re-blessed, and deterministic shots of the duel and the 30-ship battle are byte-identical to master's.

## Method

- **Allocations per frame.** The client's executable installs a counting allocator (`alloc_count::CountingAlloc`: the
  allocator underneath, mimalloc on Windows since 2026-10-10 and the system's elsewhere, with two thread-local counters,
  no atomics; on macOS a block of 1 MB or more is its own `mmap`,
  unmapped when freed, since the system allocator keeps freed large blocks dirty without limit, see that file), and
  `bench` records each frame's allocations and bytes
  on the render thread around the frame's work (`sched.per_frame.allocs`, `alloc_kb`). The report also lists the 12
  slowest frames with their stages, faults and allocations (`worst_frames`), and every spread has a p99.9.
- **Where they come from.** `cargo run --release -p naval-client --example alloc_sites -- --frames 20 --every 4
  --depth 2 [--warmup 30] [--scene one-ship] [--off effects,...]` draws the gate-B battle as `bench` does and samples
  the render thread's allocations with backtraces, grouped by the code that made them (build with
  `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` for lines). `--off` switches layers off, which priced the page faults
  per layer.
- **What an upload costs in faults.** `cargo run --release -p naval-client --example upload_faults`: on this machine
  (Apple M5 Pro, Metal) `Queue::write_buffer` faults one fresh 16 KB page per 16 KB written, every time (wgpu-core
  makes and frees a staging `MTLBuffer` per call): 2 MB a frame in two writes is 128 faults a frame; small writes are
  nearly free (80 writes of 256 B: 2 faults; Metal suballocates them). Through the staging belt, 0 in every case.
- **The strike searches.** `cargo run --release -p naval-ships --example strike_cost -- @presets.txt` loads each preset at
  full detail and times the scorch's first measurement of a hull (bounds and main deck), 48 fire vents' seats, and 470
  impact marks (a fixed set on the belt, superstructure and every mount, and 400 seeded random strikes anywhere about
  the hull, normals and rounds in every direction), and prints a digest of every result: the measured bounds and deck
  bit for bit, every spot, every mark's vertices.
- **Before and after under load.** `bun native/tools/load/ab.ts --tag h2 --before <master build> --after
  native/target/release/naval-client --levels idle,cpu54,cpu108 --seconds 30 --rounds 2`: the load lane's hog and
  levels (`load.md`), the two builds alternating at each level, `bench` on `measure30` at 2592 × 1545, High, paced to
  60 Hz, two frames in flight, the app role and interactive render thread on both. "Before" is master (d9a60d5d9) with
  only the measurement (the counting allocator, the bench's new fields) added; "after" is this branch on the same
  master.
- **Machine**: Apple M5 Pro (6 + 12 cores, 48 GB), macOS 26.6, 2026-09-30 20:28–20:40 local, other agents' builds
  running throughout (1-minute load 6–8 at idle before the hog).

## Results

Set h2 (30 s measured, two rounds, each row one run):

| Level | Build | Load | Frame p50 / p99 / p99.9 / max ms | Missed | CPU p99 / max ms | Faults p50 / mean | Allocs a frame | Heap KB a frame | Effects max ms | Scorch max ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| idle | before | 6 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 7.4 / 11.9 | 185 / 153 | 7788 | 5089 | 3.8 | 7.0 |
| idle | after | 6 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 5.1 / 6.9 | 7 / 14 | 1087 | 342 | 3.4 | 0.9 |
| idle | before | 8 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 5.8 / 11.3 | 185 / 154 | 7790 | 5087 | 3.4 | 6.7 |
| idle | after | 7 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 4.4 / 4.8 | 8 / 14 | 1088 | 341 | 2.9 | 0.4 |
| cpu54 | before | 14 | 16.7 / 17.6 / 33.3 / 33.3 | 6 | 9.8 / 19.8 | 184 / 156 | 7808 | 5121 | 5.6 | 11.8 |
| cpu54 | after | 45 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 7.3 / 8.9 | 7 / 15 | 1094 | 345 | 5.6 | 0.7 |
| cpu54 | before | 73 | 16.7 / 17.7 / 33.4 / 34.0 | 12 | 12.3 / 24.6 | 186 / 160 | 7830 | 5160 | 6.3 | 13.2 |
| cpu54 | after | 76 | 16.7 / 17.6 / 17.7 / 17.7 | 0 | 8.4 / 10.9 | 7 / 15 | 1094 | 345 | 5.4 | 0.6 |
| cpu108 | before | 103 | 16.7 / 33.3 / 82.5 / 84.2 | 40 | 17.5 / 82.0 | 190 / 171 | 7841 | 5178 | 73.7 | 51.8 |
| cpu108 | after | 120 | 16.7 / 17.6 / 32.4 / 32.8 | 2 | 7.3 / 31.4 | 9 / 18 | 1098 | 346 | 7.0 | 11.8 |
| cpu108 | before | 127 | 16.7 / 33.3 / 66.4 / 149.5 | 37 | 17.5 / 144.6 | 191 / 171 | 7838 | 5171 | 83.9 | 57.7 |
| cpu108 | after | 131 | 16.7 / 17.6 / 33.6 / 34.0 | 5 | 9.3 / 31.5 | 8 / 18 | 1099 | 346 | 20.9 | 18.7 |

Set h3 is the same comparison rerun at 21:03–21:15 after the coordinator cleared a nearly full disk (in case set h2
had run with the disk at 99 %), with other agents' work on the machine heavier (1-minute load 24–33 before the hog)
and their GPU work contending with the benchmark's (GPU busy p99 22–27 ms in every run, both builds):

| Level | Build | Load | Frame p50 / p99 / p99.9 / max ms | Missed | CPU p99 / max ms | Faults p50 / mean | Allocs a frame | Heap KB a frame | Effects max ms | Scorch max ms |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| idle | before | 24 | 16.7 / 35.0 / 68.6 / 82.6 | 52 | 8.8 / 23.4 | 179 / 149 | 7779 | 5078 | 3.9 | 11.2 |
| idle | after | 18 | 16.7 / 18.6 / 50.2 / 52.0 | 7 | 6.0 / 7.7 | 8 / 16 | 1089 | 344 | 4.8 | 0.9 |
| idle | before | 27 | 16.7 / 35.1 / 66.8 / 84.2 | 79 | 8.9 / 21.2 | 187 / 159 | 7828 | 5159 | 4.7 | 10.4 |
| idle | after | 33 | 16.7 / 34.9 / 49.1 / 51.3 | 61 | 6.2 / 8.8 | 7 / 15 | 1093 | 347 | 4.9 | 2.8 |
| cpu54 | before | 39 | 16.7 / 35.2 / 35.3 / 100.3 | 95 | 8.4 / 16.1 | 198 / 182 | 7916 | 5347 | 5.8 | 9.6 |
| cpu54 | after | 73 | 16.7 / 18.6 / 18.7 / 18.7 | 0 | 4.5 / 6.0 | 8 / 15 | 1091 | 343 | 3.5 | 0.4 |
| cpu54 | before | 80 | 16.7 / 18.6 / 49.8 / 101.3 | 10 | 7.4 / 14.0 | 185 / 156 | 7802 | 5101 | 3.7 | 7.7 |
| cpu54 | after | 84 | 16.7 / 34.0 / 34.8 / 84.5 | 27 | 6.1 / 10.6 | 8 / 15 | 1094 | 345 | 5.1 | 0.9 |
| cpu108 | before | 104 | 16.7 / 33.2 / 66.9 / 83.0 | 31 | 15.3 / 51.9 | 193 / 176 | 7840 | 5161 | 31.9 | 24.5 |
| cpu108 | after | 129 | 16.7 / 34.2 / 34.3 / 50.3 | 51 | 6.5 / 29.2 | 9 / 19 | 1099 | 348 | 6.5 | 12.3 |
| cpu108 | before | 135 | 16.7 / 34.2 / 83.6 / 100.0 | 83 | 13.1 / 81.8 | 189 / 172 | 7837 | 5171 | 57.7 | 28.8 |
| cpu108 | after | 134 | 16.7 / 34.3 / 50.0 / 84.3 | 138 | 8.4 / 27.2 | 9 / 20 | 1101 | 348 | 10.3 | 10.3 |

In set h3 the missed refreshes of both builds are other processes' GPU work: the slow frames' average GPU wait is
11–17 ms in every run, and their CPU work 4–5 ms after (7–12 before); no CPU-side change reaches that (`docs/archive/native-load-results.md`, "Other
processes' GPU work"). Everything measured on the CPU reproduces set h2: faults 179–198 → 7–9, allocations 7,800 →
1,090, heap 5.1 MB → 345 KB a frame, the worst frame's CPU time 14–82 → 6–29 ms, the effects' worst frame at cpu108
32–58 → 6.5–10 ms and the scorch's 24–29 → 10–12 ms.

Reading set h2:

- **The 53–67 ms "effects step" frames were impact marks.** In set h1 (the same runs, one round) the slowest `before`
  frame at cpu108 spent 63.2 of its 64.6 ms of effects in `marks_ms`: one `Fleet::add_impact`, its surface search
  walking the struck body's every triangle, with 1,311 faults in the frame. The 30–58 ms scorch frames were the scorch's
  first measurement of a hull (its bounds over every vertex, then eight deck rays over every hull triangle) under the
  same load. After: the worst effects frame at cpu108 is 7–21 ms and the worst scorch frame 12–19 ms, both off the
  CPU (descheduled by 108 spinning threads: those frames' CPU time is about 10 ms of 30, with 200–600 faults
  reclaiming pages the hog's churners pushed out), not the work.
- **Faults.** 185 a frame (p50) before at every level, as the load lane saw; 7–9 after. Switching layers off at 30 s
  priced them: 158 of 177 were the water effects' spray instances, about 2.5 MB a frame through `write_buffer`; the
  rest the fleet's part matrices, ensigns and impact marks, the combat effects', shells' and aircraft's instances.
- **Allocations.** 7,790 a frame (5.1 MB) before; 1,090 (340 KB) after, of which about 900 are wgpu's own bookkeeping
  (encoders, passes, trackers, debug groups). The largest before were the audio's plane tracks (2,360 a frame: each
  airframe's id, group, phase and role formatted twice a frame), the hulls' poses (1,470: every node's local and world
  matrix list, the gun covers' weights, the pose itself cloned, twice a hull for hulls with radars), the airframes'
  states (1,000), the fleet's frame lists and the impact marks' vertices (up to 10 MB a frame late in a battle, as
  marks accumulate; since 2026-10-10 marks stay in each ship's own buffers and are written only when one is added or
  cooling, `fleet.md` step 5).
- **Frame time.** At idle both builds hold the 60 Hz boundary (p99.9 17.7 ms); the CPU's worst frame drops from 11–12
  to 5–7 ms. Under load the spikes were what missed refreshes: cpu54 6–12 → 0, cpu108 37–40 → 2–5.

### The strike searches

| | Before (median over presets / worst preset) | After |
| --- | --- | --- |
| A hull's first scorch measurement | 5.4 / 12.3 ms | 0.19 / 0.46 ms |
| 48 fire vents' seats | 25.8 / 63.7 ms | 0.81 / 2.3 ms |
| An impact mark, mean | 0.54 / 0.96 ms | 0.019 / 0.034 ms |
| An impact mark, slowest of 470 | 1.3 / 26.4 ms | 0.10 / 0.20 ms |

Every digest is identical (33 of 33 presets; 15,176 strikes, 3,322 of them leaving marks). The presets now have fewer
triangles than when `fleet.md` measured 1.7 ms a strike (the wires lane moved thin members out of the hull meshes).

### The allocation guard

`cargo test -p naval-client --test all -- frame_alloc::` draws a live battle offscreen (the sim on its own thread), counts the
render thread's allocations per frame, and traces three frames' allocations to the code that made them: an allocation
whose innermost frame past the allocator, the standard library and the generic containers is wgpu's, naga's or the
platform's is wgpu's bookkeeping; the rest are the client's own. Budgets (test build, which allocates more than a
release build):

| Scene | Measured (2026-09-30) | Budget |
| --- | --- | --- |
| `one-ship`, the client's own | 92 a frame | 130 |
| `one-ship`, all | 1,750 a frame, 420 KB | 2,200, 640 KB |
| `measure30` after 60 frames, the client's own | 360 a frame | 700 |
| `measure30`, all | 2,180 a frame, 570 KB | 3,000, 1.5 MB |

Per-ship, per-plane or per-particle churn (what this lane removed) fails it; the totals catch anything larger. The
test prints the client's twelve largest sites when it runs (`--nocapture`). Raise a budget only with the reason here.

2026-10-02: `one-ship` read 132 of the client's own, 39 of them in `naval_clouds::cascades::Cascade::escapes` (#833's
coverage test: a convex hull and a Sutherland–Hodgman clip as `Vec`s, about 13 allocations a cascade an update). They
are a small polygon on the stack now (`cascades::Polygon`), and the frame reads 94 (`measure30` 103); the budget stands.

2026-10-03: `measure30` read 1,065 of the client's own on a machine at load average 35 (169 at load 4), 324 of them in
`PacketReader::accept`. A frame takes in every packet published since the last (`PacketReader::poll`), and a test build
under that load draws a frame in a second or more, so the event-driven allocations grow with the packets, not with any
churn. The `measure30` budgets hold for up to six packets a frame (a tenth of a second) and scale with the packets
beyond that: the own budget by the traced frames' packets, the totals by the counted frames'. The test prints the
packets a traced frame.

## The fixes

1. **Uploads through a staging belt** (`naval_render::Uploads`, `upload.rs`). `PrepareContext::uploads` writes through
   a `wgpu::util::StagingBelt` (1 MB chunks, mapped again once the GPU has copied from them) into an encoder the
   renderer submits first in the frame's submission, so the data is in place before any pass, as `write_buffer`'s is.
   `UploadSink` lets a layer's helpers take either path. Converted: the large per-frame writes (the fleet's part
   matrices, ship records, morph weights, ensigns and impact marks; the combat effects', shells' and water effects'
   instances; the aircraft's instances, matrices and sprites). Small uniform writes stayed on `write_buffer` (Metal
   suballocates them: they fault almost nothing) until 2026-10-09: on Windows (DX12) every `write_buffer` makes and
   frees a driver resource for its staging buffer (`StagingBuffer::new` → `dx12::Device::create_buffer`, about 7 % of
   the main thread in a theatre), so the per-frame uniforms go through the belt too: the renderer's own (camera,
   shadow maps' cameras, dither, the eye, bloom, display, TAA, film), the ships' frame, shadow and occlusion
   uniforms, the ocean's (waves, wake steps and stamps, the heights, caustics and probe, surface, spindrift, reach),
   the sky's and clouds' (tables, environment, weather, the cloud layer, aerial perspective), the land's patches and
   uniforms, the aircraft's and vapour's, the effects' and smoke shadows' (the soft occluders' uniform with the
   renderer's zeroing of it: both through the belt, so the layer's still lands after it). Passes drawn after the frame
   in submissions of their own (the page's composite, the ship labels', theatre tags' and maps' text, the chart's
   ground) record their writes into their own encoder ahead of the pass through a belt of their own
   (`naval_render::PassUploads`; the compositor holds a `StagingBelt` itself, as naval-ui does not depend on the
   renderer). Rare writes (a rebuilt spectrum, the sky's precompute, page and texture uploads, photo mode's passes,
   the Look tab's previews) stay on the queue. `Renderer::upload_stats` reports a frame's writes and bytes.
2. **Strike searches over surface runs** (`naval-ships`' `surface.rs`, the web's `SurfaceChunks`): each part's
   triangles in runs of 32 with their bounds in the part's frame, built once per preset at `add_preset` (2–3 ms per
   preset). The impact marks' ray search skips runs its segment does not pass within a centimetre of, and the decal's
   clipping skips runs whose box lies beyond a face of the decal's box; the scorch's deck rays and vent seats skip runs
   their drop does not reach; the hull's bounds start from the fixed parts' bounds (worked out once: their matrix is
   the identity in every pose) and walk only the moving parts, the furthest-reaching runs first and then any run not
   inside the bounds found. A run is skipped only when no triangle in it could pass the full walk's own test, and the
   rest are visited in their original order with the original arithmetic, so the answers are the full walk's
   (`strike_cost`'s digests).
3. **Allocation-free steady frames.** Written over last frame's storage instead of rebuilt: the hulls' poses (the
   joints' locals, world matrices and gun covers in a per-preset `PoseScratch`; each hull's pose held in the client),
   the fleet's frame lists (records, matrices, weights, frustums, casters, gathers, ensign and mark geometry; the
   motion history's three lists trade places), the scorch's publish, the client's frame lists (ship frames, water
   poses, wake ships, shells, events, the scorch queue's fires), the audio frame (plane tracks, mounts, helm; one
   aircraft pass where there were two), the airframes' states and draws (exchanged with the aircraft layer), the
   aircraft layer's gather (instances in one flat list grouped by model, as the per-model lists were), gunfire, the
   effects' publish (each pool's visible order kept, sorted with an unstable sort whose slot tie-break gives the stable
   order without a buffer; the frame held by the layer, the fires' lists traded), the shells' frame, funnel trails,
   bow spray (shapes shared, not cloned; profiles compared without copying), the wake trails' impacts and stamps,
   ship labels (names written in place, text laid out without a copy), the helm's context (targets, contacts, mount
   states made once per definition), the GPU timer's labels and timings, the frame report (recycled by the session)
   and its scene-pass labels, bloom's pass labels, and the hot-reload watcher's poll.

## What is left

2026-10-08: the combat and water sprite pools keep a sparse list of emitted slots until they expire. Ageing,
visibility publication and combat counts visit this list, so the funnel's 6,144-slot pool and the other visual
pools no longer scan empty storage every frame. The fixed ring claim and the full pool's reuse ranking stay the
same; draw order remains slot order for additive sprites and distance then slot for blended sprites. The lists
reserve the pool's capacity at construction and keep it through resets. Dense-scan equivalence tests cover
saturation, ring reuse, delayed starts, waterline death, thinning, hidden sources and resets.

- **wgpu's own allocations**: about 900 a frame in a release build (command encoders, render and compute passes'
  command lists and trackers, debug-group labels, Metal pipeline state lookups), proportional to the passes and draws.
  Fewer passes or debug groups would cut them; not this lane's to change.
- **Event-driven allocations**: each event a packet carries is copied by the packet reader (`naval-sim-host`'s
  `PacketReader::accept`) and converted by the effects (`naval_effects_sim::combat_event`) and the sound (`AudioEvent`,
  rate-limit keys formatted per cue in the director's watch); a busy battle frame makes a few hundred. Taking the
  events out of a uniquely held packet instead of copying them would remove the first.
- **The aim overlay and the camera rig** (`naval-presentation`: about 40 allocations a frame, the gun-aim points and
  indicator groups rebuilt per frame), the session's per-frame layer lists (3), and the audio's flight grouping.
- **The first scorch measurement under heavy load**: 0.2–0.5 ms per hull at idle, but a frame where several hulls take
  their first strike together still reached 12–19 ms at cpu108, mostly off the CPU. Measuring each preset's deck at
  load would take it out, at the cost of exactness in the last bit (the deck rays depend on the posed bounds).
- **Not measured**: the vsynced window (`bench --window`) and a CEF build; Windows and Linux (the belt is wgpu's own
  and portable; `upload_faults` would show whether their drivers fault per staging buffer as Metal does).

## Running it

```sh
cd native && cargo build --release -p naval-client && cd ..
native/target/release/naval-client bench --scene measure30 --size 2592x1545 --seconds 30   # allocs, faults, worst frames
bun native/tools/load/ab.ts --tag h2 --before <baseline naval-client> --levels idle,cpu54,cpu108 --seconds 30 --rounds 2
cargo test -p naval-client --test all -- frame_alloc:: --nocapture                              # the guard (from native/)
```
