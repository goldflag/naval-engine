# The frame: layers, pipelines and passes

## The integration surface

Another crate (ships, ocean, sky) draws by implementing `SceneLayer`, building its pipelines through
the `RenderContext` the renderer lends, and being passed to `Renderer::render` each frame. It never
owns the scene targets or the frame order.

```rust
use naval_render::{shader_file, DrawContext, Phase, PipelineId, PrepareContext, RenderContext,
    SceneLayer, ScenePipeline, ShaderError};

pub struct Ocean { surface: PipelineId /* , buffers, bind groups */ }

impl Ocean {
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        ctx.add_shader(shader_file!("shaders/waves.wgsl"))?; // a module: #define_import_path ocean::waves
        let surface_wgsl = shader_file!("shaders/surface.wgsl"); // an entry: #import naval::camera::{camera, world_to_clip}
        ctx.add_shader(surface_wgsl)?;
        let surface = ctx.add_scene_pipeline("ocean surface", surface_wgsl.path,
            ScenePipeline { bind_group_layouts: vec![/* group 1, 2, … */], ..ScenePipeline::new("vertex", "fragment") })?;
        Ok(Self { surface })
    }
}

impl SceneLayer for Ocean {
    fn label(&self) -> &str { "ocean" }
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        // queue writes from ctx.time and ctx.camera; compute passes on ctx.encoder
    }
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Opaque {
            pass.set_pipeline(ctx.pipelines.render(self.surface));
            // pass.set_bind_group(1, …); pass.draw(…);
        }
    }
}

// The host:
let mut renderer = Renderer::new(&device, &queue, width, height, surface_format)?;
let mut ocean = Ocean::new(&mut renderer.context())?;
let mut sky = Sky::new(&mut renderer.context())?;
renderer.render(FrameInput { camera: &camera, time }, &mut [&mut ocean, &mut sky],
    RenderOutput { view: &surface_view, format: surface_format, width, height })?;
```

The rules:

- **Phases, in order**: every layer's `Opaque` draws (depth writing: hulls, terrain), then every
  `Water` draw (depth writing, after the opaque copies: the sea; added by render core II, see
  "Scene copies"), then every `Sky` draw (backgrounds at clip depth 0, where `GreaterEqual` against
  the clear passes only on uncovered pixels), then every `Transparent` draw (blended, depth tested,
  not written). Within a phase, layers draw in the order passed.
- **Group 0 is the camera** and is rebound before each layer draws. Layers use groups 1 and up.
  Build group 0 of any pipeline from `ctx.camera_layout`, never a layout of your own: render core
  II adds the sun's shadow maps to it (bindings 1 and up; see "Sun shadows").
- **Radiance is linear HDR.** Never tone map or encode sRGB in a layer; the display pass does it.
- **Pipelines go through the context** (`add_scene_pipeline`, or `add_pipeline` with a recipe for
  compute, shadow maps and offscreen targets; match `HDR_FORMAT` and `scene_depth_state` there when
  drawing into the scene). That is what gives them hot reload. Look pipelines up by id each frame
  (`ctx.pipelines.render(id)`), since a rebuild replaces what the id names. `PipelineRegistry::remove`
  drops one a layer no longer needs. Builds run on the registry's threads: `add` returns before the
  pipeline exists and a lookup waits for it, so look up when recording, not in the constructor (see
  "Pipelines: no first-launch stall").
- **`prepare` gets the frame's encoder** before the scene pass, for compute (FFT waves, wake fields,
  particle simulation) and a layer's own passes (shadow maps, LUTs). Frame-lifetime textures can be
  made there in `ctx.resources`.
- **Shared resources are found by name**: `scene.hdr` and `scene.depth` (both `TEXTURE_BINDING`), and
  anything a layer registers. Check an entry's `version` before reusing a bind group on it.
- **Reading the scene** (refraction, soft particles, fog on depth) inside the scene pass is not
  possible, since those targets are attachments there. Ask for scene copies instead (see "Scene
  copies" below).
- **Contact shadows** are for the layers that draw their surfaces into the contact depth
  (`draws_contact_depth`, `draw_contact_depth`); such a layer reads `contact.mask` at its own pixels for its sun
  term, and no other layer may read it (see "Contact shadows" below).

### Shader library

| Import path | Gives |
| --- | --- |
| `naval::camera` | `Camera`, `camera` (group 0 binding 0), `world_to_clip` and `curved` (a world point where the camera draws it: "The Earth's curvature" below), `ndc_from_pixel`, `view_ray`, `view_depth` (meters from reversed depth) |
| `naval::color` | `luminance` (Rec. 709), `srgb_oetf`, `srgb_eotf` (three's transfer functions) |
| `naval::display` | `display_grade(radiance, exposure)`, `agx_tone_mapping(color, exposure)` |
| `naval::fullscreen` | `FullscreenVertex`, `fullscreen_vertex(index, depth)` (one triangle) |
| `naval::aerial` | `aerial_apply(color, world)`, `aerial_apply_additive(color, world)`, `aerial_at(world)`: the air between the eye and a fragment, from the volume at group 0 bindings 6 and 7 (`haze.md`) |
| `naval::lights` | the scene's point lights at group 0 (`scene_lights`, binding 10: the 64 brightest at the camera, `Renderer::set_point_lights`) and their clustered index (`light_clusters`, binding 11; `light_cell(world)`, `light_word(cell, word)`), `point_light_at`, `shadowed_point_light` and `light_shadow` (the brightest searchlights' and star shells' maps, layers of the sun's near-map array past its own), `SceneFog` (`native/docs/lighting/lights.md`, "Lights at night") |
| `naval::aerial_volume` | the volume's slices and its binding-free read (`aerial_sample`), for a crate that binds a copy of it elsewhere |
| `naval::pbr` | `f_schlick`, `v_ggx_smith_correlated`, `d_ggx`: three's BSDF terms, which the ships' paint (`naval_ships::bsdf`) and the land (`naval_terrain::lighting`) are lit with |
| `naval::encode` | `encode(color, pixel)`: a display-encoded colour as the chain's last stages write it, dithered onto an 8- or 10-bit output (`DITHER_LEVELS`) and decoded after for an sRGB target (`DECODE_SRGB`) |
| `naval::math` | `modulo` (GLSL's `mod`), `rotate` (a vector turned by a unit quaternion) |
| `naval::mx_noise` | `mx_rotl32`, `mx_negate_if`, `mx_gradient_float3`, `mx_fade`: MaterialX noise's helpers, which the ships', the sky's and the sea's Perlin noise (`naval_ships::noise`, `naval_sky::noise`, `naval_ocean::mx_noise`) share |

`shader_file!("shaders/x.wgsl")` embeds a file and keys it `<crate>/shaders/x.wgsl`. naga_oil traps
found on the way:

- A composable module's identifiers may not end in a digit or start with `_` (`pad0`, `_pad` fail
  with "must not require substitution according to naga writeback rules"). Pad with `padding: vec2<u32>`.
- A module is parsed only when a shader uses one of its items; the library composes each module's
  own text once when it registers or changes, so a broken module is reported at once and the last
  good composer kept.
- Import items explicitly (`#import naval::camera::{camera, view_ray}`); a bare `#import naval::camera`
  imports nothing until `camera::item` is used.

### Hot reload

`Renderer::apply_shader_changes(Vec<(path, text)>)` is the portable core: it updates the library,
rebuilds every pipeline whose entry shader imports a changed file (directly or through modules) and
returns a `ReloadReport`. Off wasm32 the renderer also has `Renderer::poll_shader_files` (the client and harness call it
under their `hot-reload` feature; naval-render itself has no feature, so every build shares one), which checks
the modification times of every registered file with a disk path at most every quarter second and
feeds changes in. On a file's first sighting (a new renderer, as `load_scene` makes) its disk text is
compared with the embedded text, so a scene reload keeps shader edits made since the build. Polling
was chosen over a watcher thread: no dependency, no thread, it sees editors that save by renaming,
and it costs a few `stat` calls a second. A module edit made while another module is broken is held
back and goes live, with its pipelines rebuilt, when the broken module is fixed. A shader that no longer composes, or
a pipeline wgpu rejects (caught with a validation error scope, which native wgpu answers at once),
keeps its last good pipeline; `telemetry` lists it under `pipelines[].error`. On the web the error
scope answers later, so a bad rebuild would surface as an uncaptured error; the web has no file
watcher anyway.

## Pipelines

Lane pipelines, branch `goldflag/native-pipelines` (2026-09-30). Every pipeline a battle draws with is now built
through the registry, on build threads, before the first frame; every WGSL file it uses hot-reloads; and nothing
compiles mid-battle. Code: `naval-render/src/pipelines.rs` (build threads, placeholders, `MakePipelines`),
`shaders.rs` (lazy composer), each layer's `register_pipelines`/`adopt_pipelines`, `naval-client/src/app/pipeline_cache.rs`.
Outputs under `.build/native/pipelines/`.

### The driver's compile and its cache

A pipeline's first compile on a machine is the driver's work: on Metal 50–400 ms each (MSL to AIR, then AIR to the
GPU's code), about 1 ms once the system's shader cache holds it. A battle has about a hundred pipelines (94 before the wires lane), and the old load built
them one after another on the calling thread, a quarter of them twice (the sky and clouds built their own, then the
registry's copies; the second was a Metal cache hit because the MSL is identical). macOS keeps the compiled shaders
per app (a bundle's own `$(getconf DARWIN_USER_CACHE_DIR)/<bundle id>/com.apple.metal`, keyed by the shader source;
unbundled executables share one folder), so "cold" is a new install or changed shaders. Measured with a probe bundle
of its own id whose cache folder is deleted before each cold run (`.build/native/pipelines/probe`).

### How pipelines are built

- **Build threads.** `Renderer::new` gives the registry one thread per core (at most 16;
  `pipelines::default_build_threads`, `Renderer::set_pipeline_build_threads`). `PipelineRegistry::add` still composes
  on the calling thread (a composition error is returned at once), then hands the module and the recipe to the pool
  and returns the id. A lookup (`render`, `compute`) waits for its pipeline; `Renderer::render` and
  `PipelineRegistry::finish` wait for all of them and return a rejected first build as an error (error scopes are per
  thread in wgpu, so each build is validated on its own thread). Hot reload rebuilds in parallel too and returns when
  all are done; the last good pipeline stays wherever a rebuild fails. Recipes must be `Send + Sync` on native
  (`RecipeFn`); wgpu's handles are. On the web there are no threads and every build runs as it is registered.
- **Placeholders instead of own builds.** A layer whose pipelines live in its structs (the sky, clouds, weather, post
  passes, ocean, ships) is built with `MakePipelines::Deferred(ctx.placeholders())`: its constructors put the
  renderer's two trivial placeholder pipelines in those fields and compose nothing, `register_pipelines` registers the
  real ones, and the first `prepare` takes them in with `adopt_pipelines` (which hot reload already used), so a
  layer's constructor never waits for a compile and every layer's pipelines compile together. `MakePipelines::Now` is
  the crates' standalone path (tests, tools, examples), unchanged. Entry points: `GameSky::new_in`, `Ocean::new_in`
  and `enable_wake_in`, `ShipRenderer::for_core_in` (`ShipLayer::with_core_shadows`). A placeholder is never bound:
  each layer adopts at the top of its `prepare`, before any pass; `Sky::set_quality` asserts the sky adopted first.
- **The rest of the battle on the registry.** The weather's rain, splashes and bolt (`WeatherPipelines`; the post
  passes' shafts and veil, `PostPipelines`, are gone), the clouds' noise fill (run at the first prepare for a deferred sky, and
  again when `noise.wgsl` changes), the ocean's field, surface, wake field and stamp painters, the ships' hull,
  impact marks, gather, backdrop, core casters and motion, own sun-map casters and occlusion, and the labels (per
  output format: `ShipLabels::register_pipeline`). The ocean, the ships and the sea's spray import the sky's own
  environment module (`naval_sky::environment`), registered once by whichever layer comes first.
- **Everything at load.** `Renderer::prepare_output(format)` builds the display pass, every edge-smoothing stage
  (FXAA, SMAA's blend, TAA's present) onto the output and the post frame, the upscale onto either, the look's film
  pass and the overlays for a format;
  the session calls it, and registers the labels' pipeline, for the capture format and the window's
  (`Session::set_window_format`, set by `view` and the windowed bench) at every load. Before, each was built by the
  first frame that needed it: a change of antialiasing, render scale or labels mid-battle compiled then.
- **A lazy library.** Registering a module no longer rebuilds the composer (each rebuild parses every module and
  composed each one on its own as a probe): the first composition after registrations builds it once, probes only
  modules not probed before (by text hash), and registering a file again with the same text changes nothing. Hot
  reload's `update` stays eager, so a broken module is still reported at once while the last good composer stands.
  This alone was about 0.8 s of a warm load once the ocean registered its modules.
- **Shared modules keep their disk paths.** naval-sky's environment (`naval_sky::shaders::ENVIRONMENT_FILE`), the
  clouds' shadow (`naval_clouds::shaders::SHADOW_FILE`) and the ships' BSDF are registered from their own crates,
  only when absent; the aircraft, the sky mean and the harness's sky scene used to re-register them without a disk
  path, which turned their hot reload off.
- **wgpu's pipeline cache where it exists (Vulkan).** `Renderer::with_pipeline_cache` hands a `wgpu::PipelineCache` to
  every recipe (`PipelineBuild::cache`; the core's, the shared recipes', the ocean's, ships', labels' and effects'
  descriptors use it). naval-client asks for `Features::PIPELINE_CACHE` where the adapter has it, reads the cache from
  the user's cache directory (`~/Library/Caches/gg.tomato.naval/pipelines`, `%LOCALAPPDATA%\naval\cache\pipelines`,
  `$XDG_CACHE_HOME/naval/pipelines`) under wgpu's adapter key and this build's identity (the executable's size and
  modification time; other builds' files are deleted), lets wgpu check the data's header against the driver and
  adapter (`fallback: true` starts empty on a mismatch), and writes it back after the first frame of a load. The clouds'
  and the sky's own descriptor helpers still pass no cache.
- **Telemetry.** `pipelines[].build_ms` (the latest build's wall time on its thread) in both RPCs' `telemetry`;
  `Renderer::pending_pipelines`, `Renderer::pipelines` (for a host's own passes, as the labels').

### Backends

| Backend | wgpu `PipelineCache` (30.0.1) | What persists compiled shaders | What this lane does |
| --- | --- | --- | --- |
| Metal | no-op (`create_pipeline_cache` returns an empty cache; `supports_binary_archives` is detected but unused, so no `MTLBinaryArchive`) | macOS's per-app Metal cache (MSL→AIR by source, AIR→GPU code per GPU), kept across launches and builds whose shaders did not change | builds every pipeline at once on build threads; nothing to persist |
| DX12 | no-op | the driver's own PSO caches (NVIDIA, AMD, Intel keep them on disk); upstream wgpu keeps up to 200 DXC blobs per device in memory only, so DXC recompiles HLSL every launch | build threads parallelise DXC and PSO creation (wgpu-hal compiles outside its cache lock); the patched wgpu-hal keeps up to 4096 blobs in memory and DXC's output on disk (`%LOCALAPPDATA%\naval\cache\dxil`, [native/vendor/README.md](../../vendor/README.md)) |
| Vulkan | supported (`Features::PIPELINE_CACHE`, validation key of driver and adapter) | wgpu's cache (persisted by naval-client) and the driver's (Mesa, NVIDIA) | persists it per adapter and build |

DX12 on a Windows runner (historical run 36797617983, WARP through DX12, the branch with build threads on): the
packaged client drew one-ship and measure30 from a fresh folder (257 s and 302 s on the software rasterizer), so
concurrent pipeline creation (DXC and PSOs on build threads) works there. The workspace tests then ran until the job's
120-minute limit stopped them inside `naval-client --test pipelines`: on WARP a battle second takes minutes, so those
two tests now skip on a software adapter (they run on any GPU). That was the lane's one Windows run; the rest of the
workspace's tests on WARP are unconfirmed for this branch. **On a real Windows GPU** (`native/docs/windows.md`,
section 3; RTX 5070 Ti) the Vulkan cache works: written, read back, accepted, and worth 0.5 s of a 2.8 s cold first
frame, as much as NVIDIA's own cache. DX12 there takes 12 s to its first frame at every launch, 8.1 s of it FXC
compiling the ocean surface: wgpu's `Auto` compiler falls back to FXC without a current `dxcompiler.dll`, and keeps no
compiled HLSL between launches. naval-client now links DXC on Windows (`static-dxc`): 2.9–3.1 s. DXC itself was then
24–34 s of a theatre's 38–50 s load in a fresh process (2026-10-26); with its output kept on disk, a launch compiles only
the stages whose HLSL changed (`native/docs/windows.md`, section 3).

### Mid-battle hitches

`cargo test -p naval-client --test pipelines`: `a_battle_creates_every_pipeline_while_it_loads` loads measure30, counts
the device's render and compute pipelines after the first frame (wgpu-core's report), then plays until aircraft fly
and guns fire and switches every graphics preset × antialiasing mode and the render scale and labels: the count
does not change, no pipeline is registered late, and every pipeline alive is the registry's (besides the two
placeholders; the wires lane's tubes, strips and their motion included). `every_file_a_battle_registers_hot_reloads` edits every registered file (all have disk paths): every
pipeline rebuilds and compiles, and a broken edit keeps the last good ones. Still built on demand: the clouds' and
the sky's quality change (`GameSky::set_quality`: the next battle takes it in the client).
Outside the renderer, the UI host's compositor (`naval-ui`, `--features ui`) builds its composite pipeline per output
format on its first frame and its shader is not in the library; it would join through a `prewarm(format)` and the
registry when the UI lane takes it on.

### Rules for a new layer

- Build pipelines through `RenderContext` (`add_scene_pipeline`, `add_caster_pipeline`, `add_motion_pipeline`,
  `add_overlay_pipeline`, `add_pipeline` with `compute_recipe`/`fullscreen_recipe` or a recipe of your own passing
  `build.cache`), from files registered with `shader_file!` (disk paths, for hot reload). Look pipelines up when
  recording, not in the constructor, so they compile while the rest loads; a layer that keeps pipelines in its own
  structs follows the placeholder pattern above.
- Register another crate's module from that crate's `ShaderFile` and only when absent
  (`if ctx.library.text(file.path).is_none()`), never with `add_source`.
- Create every pipeline a scene can need before its first frame: an effect type, preset or setting first seen
  mid-battle must not compile then. The client test above fails when one does.
- A layer that keeps state for one camera or one viewport size (screen targets, history across frames, what it culled
  against the camera) handles `SceneLayer::set_view` if it may draw a second view (below): it keeps a copy for that
  view, or draws nothing there.

## Scene copies

Lane render-core-2, branch `goldflag/native-render-core-2`. The sea, and soon waterline foam, soft
particles and a smoke occlusion map, sample the scene drawn before them. A render attachment cannot
be sampled in the pass that draws into it, so the scene pass ends at two **copy points**, copies
what layers asked for into textures of their own, lets layers record passes on them, and resumes
in a new pass that loads what was drawn. This is what the web gets from three's viewport copies.

```text
prepare ─ scene pass [Opaque] ─┬─ copy point Opaque ─ [Water, Sky] ─┬─ copy point Backdrop ─ [Transparent] ─ display
                               │   scene.opaque.color              │   scene.backdrop.color
                               │   scene.opaque.depth              │   scene.backdrop.depth
                               └─ after_copies(Opaque)             └─ after_copies(Backdrop)
```

| Copy | Registry name | Format | Holds | Web counterpart |
| --- | --- | --- | --- | --- |
| `SceneCopies::OPAQUE_COLOR` | `scene.opaque.color` (`SCENE_OPAQUE_COLOR`) | `Rgba16Float`, linear HDR | every `Opaque` draw; black where water and sky will be | the ocean's `viewportTexture()` (`OceanSurfaceMaterial.sceneColor`) |
| `SceneCopies::OPAQUE_DEPTH` | `scene.opaque.depth` (`SCENE_OPAQUE_DEPTH`) | `Depth32Float`, reversed | the same; 0 (far) where nothing opaque is | the ocean's `EffectDepthTextureNode` (`sceneDepth`) |
| `SceneCopies::BACKDROP_COLOR` | `scene.backdrop.color` (`SCENE_BACKDROP_COLOR`) | `Rgba16Float` | opaque, water and sky: what transparents draw over | none; read by naval-effects' blast waves (`ShockLayer`), only while a front is in the air (`native/docs/combat-effects/gun-blasts.md`, "Blast waves") |
| `SceneCopies::BACKDROP_DEPTH` | `scene.backdrop.depth` (`SCENE_BACKDROP_DEPTH`) | `Depth32Float` | opaque and water (the sky writes no depth) | `EffectLighting.sceneDepth` (soft particles: the web's copy is taken when the first effect draws, after the sea) |

All four are full-size, single-mip viewport textures (`TEXTURE_BINDING | COPY_DST | COPY_SRC`),
remade on resize with a new `version`.

```rust
use naval_render::{CopyPoint, SceneCopies, SCENE_OPAQUE_COLOR, SCENE_OPAQUE_DEPTH};
use naval_render::copies::{color_copy_layout_entry, depth_copy_layout_entry};

impl Ocean {
    pub fn new(ctx: &mut RenderContext<'_>) -> Result<Self, ShaderError> {
        // Make the textures now so the surface can bind them from the start.
        ctx.request_scene_copies(SceneCopies::OPAQUE_COLOR | SceneCopies::OPAQUE_DEPTH);
        let color = ctx.resources.texture(SCENE_OPAQUE_COLOR).unwrap();   // .view, .version
        // Layout entries: color_copy_layout_entry(binding, FRAGMENT) is texture_2d<f32>
        // (filterable), depth_copy_layout_entry(binding, FRAGMENT) is texture_depth_2d.
        …
    }
}

impl SceneLayer for Ocean {
    fn scene_copies(&self) -> SceneCopies {           // asked every frame, before prepare
        SceneCopies::OPAQUE_COLOR | SceneCopies::OPAQUE_DEPTH
    }
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) {
        // Rebind when ctx.resources.texture(SCENE_OPAQUE_COLOR).version changed (a resize).
    }
    fn after_copies(&mut self, point: CopyPoint, ctx: &mut PrepareContext<'_>) {
        // Optional: passes on the copies before the scene resumes, e.g. a depth pyramid
        // for screen-space reflections, on ctx.encoder.
    }
    fn draw(&self, phase: Phase, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        if phase == Phase::Water { /* the surface, depth test and write on */ }
    }
}
```

```wgsl
@group(1) @binding(4) var scene_color: texture_2d<f32>;      // scene.opaque.color
@group(1) @binding(5) var scene_depth: texture_depth_2d;     // scene.opaque.depth
let pixel = vec2<i32>(in.position.xy);
let behind = textureLoad(scene_color, pixel, 0).rgb;          // linear HDR
let meters = view_depth(textureLoad(scene_depth, pixel, 0));  // naval::camera; far where 0
```

The rules:

- **A copy is taken only when some layer's `scene_copies()` asks for it that frame.** A frame
  nobody asks copies of is one render pass, as before. The pass ends only at a point where a copy
  is taken; asking for only `BACKDROP_DEPTH` gives two passes (`opaque, water, sky` and
  `transparent`). `FrameReport::copies` and `FrameReport::scene_passes` say what happened.
- **`Phase::Water`** is the new phase between `Opaque` and `Sky` for depth-writing surfaces that
  see the opaque scene through themselves. The sea belongs there: it samples the opaque copies,
  writes depth, and the sky then fills only what neither covered, as the web's order does (sea at
  render order −30, dome −29.5, clouds −29, effects 0 and up). A layer that does not match on
  `Water` is unaffected (`phase == Phase::Opaque` checks keep working).
- **Copies are separate textures**, so they can be bound in any phase, any group, and in compute.
  Never bind `scene.hdr` or `scene.depth` themselves in a scene pipeline.
- **`after_copies(point, ctx)`** runs at each point where copies were taken, with the frame's
  encoder and mutable resources (the same `PrepareContext` as `prepare`), after the copies and
  before the scene resumes. Use it for passes on the copies: a depth pyramid, a half-resolution
  depth for particles, a depth-tested smoke occlusion map. Layers run in the order passed. A pass
  drawn there with the scene's camera binds `ctx.camera_group` at group 0 and builds its pipelines
  with `add_scene_pipeline_into` (a scene pipeline into targets of the layer's own); it may attach
  `scene.depth` read-only (load and store, no depth writes) to test against the opaque and sea
  depth: the effects' merged smoke does (`native/docs/combat-effects/renderer.md`, "Merged, not layered").
- **Color copies are filterable** (`Rgba16Float`), so `textureSample` with a linear sampler works
  for blurred refraction; the web reads level 0 with `textureLoad`-like screen reads. A mip chain
  (three's `viewportMipTexture`) is a layer's own pass in `after_copies`.
- **Depth copies read with `textureLoad`** (texel exact, as the web's reflections do) or a
  comparison sampler. `naval::camera::view_depth(d)` gives meters along the view; depth 0 is the
  far plane and "nothing here".
- **Cost**: each copy is a full-screen texture copy (16 MB for color and 8 MB for depth at
  1920 × 1080), and each split stores and reloads the scene targets once (on a tile GPU this is
  the real cost). Ask only for what you read. Measured cost is in [post.md](post.md), "GPU timings".
- **Not the smoke occlusion map itself**: the web draws that before the scene pass without depth
  (`src/game/SmokeOcclusion.ts` (historical), a quarter-resolution MRT of four nearest-front levels and summed
  optical depth), which a layer's `prepare` already does. A depth-tested variant would draw in
  `after_copies(Backdrop)` against `scene.backdrop.depth`.

For the ocean (`docs/archive/native-ocean-history.md`, "What the ocean needs from the renderer for the slice"): move the surface
to `Phase::Water`, return `OPAQUE_COLOR | OPAQUE_DEPTH` from `scene_copies`, and hand
`scene.opaque.color` and `scene.opaque.depth` to `OceanSurface::set_scene` at creation and when
their `version` changes.

## A second view in the same frame

`Renderer::render_view(&mut SecondView, FrameInput, layers, output)` draws another camera of the same scene after
`render`, into an output of its own size: theatre's fall of shot window (native/docs/theatre.md, "The fall of shot
window") is the first. The registry swaps its viewport textures for the view's (`Resources::swap_view`,
`ViewTextures`): whatever viewport texture one set lacks is made at its size, so neither view finds one missing, and
each keeps its versions, so the layers' cached bind groups rebuild only on a switch. It takes the shortest way to the
output and touches no history of the main view's: no sun maps (the main view's are held across frames; every map is off
in its own fit, so its hulls are lit unshadowed), no temporal AA, motion pass, bloom, sun glare, photo mode, film, debug
view or overlays, the scene at the output's size whatever the render scale, its passes untimed (the GPU timer's frame
has ended). Its eye is its own (`post::ViewEye`): the main eye's settings, sky mean and night meter the view's own
scene into a state buffer its display pass reads, stepped by the scene time since the view's last draw, and settled at
once on its first draw, after `SecondView::release` or after a pause of more than a second. Until 2026-10-25 it was
graded with the main view's exposure, so the fall of shot window brightened when the player's camera looked into
shadow and dimmed when it looked into the sun, however far away its target was.

The host chooses its layers and tells each the view first (`SceneLayer::set_view(SceneView::Second)`, and `Main`
after), with `time.dt` 0 so nothing a layer steps in `prepare` steps twice. Those with per-view state: the game sky draws
its dome alone, its tables and aerial perspective for that camera, and leaves the clouds, weather, clock and cut alone;
the ocean skips the wave field's update and keeps its own scene-copy bindings and reflection tiles; the ships turn their
occlusion off (its targets are the main view's) and skip the outline; the combat effects keep their own occlusion and
depth targets and cast no smoke shadow; the water effects keep their own column target, march divisor and depth group,
and the host places the splashes for each camera (`WaterEffects::publish`). The land, the aircraft (their pose history
for temporal AA is the main view's), the sky's mean, the underwater veil, the blast waves and the aim overlay are not
drawn there.

## The frame's parts finish on other threads

wgpu 30 keeps a pass's commands as they are written and does the work when their encoder finishes: it validates and
encodes every pass then, and DX12 records the command lists, two a pass (the pass and the barriers before it). A fleet's
frame was one encoder of 47 passes and 99 command lists, and its `finish` a quarter of the main thread.
`Renderer::render` now hands each part of its frame to a `Finisher` (`naval_render::finish`) as soon as the part is
recorded: the layers' prepare, the shadow maps, each group of the scene pass. The parts finish beside one another and
beside the rest of the recording; the post finishes where it is submitted, and all go in one submission, in order. The
crate spawns no threads: the client installs `gpu::FinishPool` (three threads at the frame loop's priority;
`NAVAL_FINISH_THREADS=0` finishes the frame in one encoder, as before). Without a finisher nothing changes.
`copies::a_frame_finished_in_parts_on_other_threads_draws_the_same` holds the picture to the one encoder's, bit for bit.
A part (`naval_render::Part`) carries its index and a sender of one channel the renderer keeps, so handing it over
allocates nothing (a channel a part was ten of the quiet frame's allocations, `frame_alloc`); dropped unfinished, it
comes back empty and the frame fails rather than waits.

Each part costs the submission a barrier list of its own (wgpu inserts one a command buffer), so the frame is cut where
whole groups of passes end, not at every pass.

Measured on Windows (RTX 5070 Ti, 3840 × 1600, the player's graphics, release, the machine otherwise idle), Eastern
Solomons in the Ship view on Saratoga, three alternating rounds of 40 s in one package: 100.1 → 106.2 fps, the lowest
5 s 94.3 → 99.4, the frame's CPU 6.95 → 6.34 ms, its `submit` stage 2.05 → 1.32 ms. A census of one frame (a temporary
counter in `wgpu-hal`'s DX12 encoder) found 121 command lists in 8 submissions (resetting the lists alone took 0.6 ms),
38 render and 9 compute passes, 345 draws and 67 indirect ones, 356 bind groups and 81 pipelines. The draws cost the
driver little; the lists, the submissions and the present cost it most. The GPU's own frame was 4.45 ms of it.

Cutting the frame's last part again did not pay: handing the bloom's dozen passes to a finisher of their own (before
and after them) left the main thread waiting on that part, which finished after the display's end it had recorded and
finished meanwhile, and gave the submission two more barrier lists: 126.0 → 121.2 fps (2026-10-26, two clean rounds).

**Fewer submissions.** A submission costs the main thread about 0.1 ms before its work (wgpu's trackers merged into the
device's, a barrier list per command buffer, the fence), and `queue.write_buffer` makes and later frees a staging
buffer of its own each call. So:

- The tags, the chart's ground and the plot map draw into one tail encoder (`Session::tail`), submitted at the
  frame's end in place of the empty submission that marked it (`gpu::Frames::after_frame`); a draw that must stand over
  them (the editor's world, the fall-of-shot window) submits it first. 116.0 → 118.7 fps. The tags' part is finished
  within the frame's time (`Session::tail_finished`), so `labels_ms` and `total_ms` still count its encoding as they
  did when it was a submission of its own. The maps' draws go on a second encoder in the same submission. The text
  atlas is written by a copy on the encoder (`PassUploads::write_texture`), not by `Queue::write_texture`: an atlas
  that fills and starts again between two passes of one submission would otherwise change under the earlier pass.
- The layers' per-frame uniform writes and the buildings' 64 KB of chunk translations go through the frame's staging
  belt (`PrepareContext::uploads`), which lands before the frame's commands as the queue's writes do.
- The wet decks' copy of the stems' seas goes first in the frame's own submission
  (`Renderer::submit_with_frame`), still before the ocean's pass, mapped once that is in. With the belt's writes,
  123.7 → 126.0 fps.

Still a submission of its own: the page's composite (its own encoder in `naval-ui`, about 1.5 % of the main thread),
which could join the tail if the window's frame left the tail open for it.

## The Earth's curvature

The simulation's world is a plane. The client draws it curving away from the camera over the Earth, so the sea has a
horizon at the right dip and a hull beyond it goes hull-down: her waterline behind the sea first, then her decks, then
her upperworks, her mast tops last. Before this the sea's visible edge already lay at a sphere's horizon (the sky's)
while the hulls stood on the plane, so past about half the true horizon distance a ship floated above the sea line with
her underwater hull showing.

- **The rule.** A world point is drawn lowered by `curvature × d²`, `d` its horizontal distance from the eye:
  `naval_render::camera::curved` (and `curved_f64`, `flattened_f64`, `Camera::curved`) and `naval::camera::curved` in
  WGSL. `curvature` is `EARTH_CURVATURE`, 1 / (2 R) with R = 6,371,000 m, the simulation's
  (`naval_sim::terrain::EARTH_RADIUS_M`; `naval-presentation`'s `view` test holds the two equal). That is the drop of a
  sphere's surface below the eye's tangent plane: 0.08 m at 1 km, 7.8 m at 10 km, 31 m at 20 km, 71 m at 30 km. From an
  eye `h` metres up the horizon is 3570 √h metres off, and a feature `H` metres up is on it at 3570 (√h + √H), which is
  the range `naval-sim`'s lookouts see her to (`sensors.rs`): what the lookouts' rule says is in sight is what is drawn
  above the sea (tested in `camera.rs`).
- **Where it is applied.** `Camera::curvature` (per camera) goes to the shaders in `CameraUniforms::curvature`, and
  `world_to_clip` applies it; every pipeline that places world geometry projects through it (ships and their wires,
  the sea and its spindrift, wakes and water effects, combat effects, aircraft, terrain and trees, the torpedo overlay).
  A camera built from matrices (`CameraUniforms::from_matrices`: the sun's shadow cameras, the casters) is flat, so
  shadow maps, the sea's fields and all shading keep the plane's positions: only where a point lands on the screen
  curves. Positions rebuilt from depth (the sea's water column and reflections, soft particles, the clouds' march,
  temporal reprojection) are drawn positions, consistent with each other; where a shader compares one with a world
  point it takes `curved(world)` (the effects' view depth, the sea's reflection origin, `aerial_at`'s screen lookup).
- **On the CPU.** Whatever the client projects itself takes the same drop: the frustum culls (`fleet.rs` in
  `naval-client` and `naval-ships`, the sun's casters in `scene.rs`, terrain patches in `lod.rs`, the effects', water
  effects' and aircraft's cameras, each with a `drawn`), the GPU labels, the late latch, and `ViewCamera::project` and
  the aim circles in `naval-presentation`. A ray from the screen is straight among drawn points: the gun sight
  (`sight::sight_aim`) takes the camera's curvature, meets the sea where the screen shows it and a hull where she is
  drawn, and returns the simulation's point; the rig lays the sight on where a point is drawn (`aim_at`, the range
  lock, the optics transition). A harness camera placed with an eye and a target, or orbiting a hull from far off,
  is turned onto the drawn target (`Camera::on_drawn_target`).
- **The page.** `ScreenAnchors::curvature` carries the camera's; the anchors stay the simulation's points and the page
  projects them through `drawnPoint` (`src/game/contract/curvature.ts`: `projectShipLabel`, `projectAnchor` and so the
  hull pick, `bearingMark`).
- **Flat cameras.** The fleet chart (`BattlefieldCamera`) is a diagram of the simulation's plane: its camera's
  curvature is 0, so its projections and its sea hits stay exact, and the glide into and out of it blends the
  curvature with the lens. A bare `ViewCamera` and the ships crate's standalone camera are flat too.
- **Not done.** No atmospheric refraction (the simulation's horizon is geometric as well). The sky's own sphere is
  6,360 km, 0.09 % under the dip the sea is drawn with: under a pixel at any lens. Shading uses the plane's view
  direction to a point, off the drawn one by d / 2R (0.09° at 20 km). The theatre layer's sighting between forces
  has no horizon term (its eye ranges are under the horizon of the ships that use them); only a battle's lookouts
  do.
- **The sea to its horizon** (`naval-ocean`, 2026-10-04). The drop is applied per vertex, so the sea's mesh has to be
  fine enough for it and reach far enough. Its ring had rows at 4, 8 and 16 km and one at 57 km (95 % of the far plane):
  the water between the last two was a chord 30 m under the curve, and from an aircraft the ring's edge at 57 km stood
  in for a horizon 216 km off (from 12,000 ft), twice as far below eye level, so it bowed across a wide view and ended
  dark, with no far water in the haze. The ring's circles now stand 3.5 km apart (a chord sags 0.24 m), and from 60 to
  480 km 15 km apart (4.4 m) for a theatre view that draws the land beyond its window (2026-10-09), and a skirt of six
  rows beyond it runs to the sea's horizon (√(height / curvature) from the eye) whenever that lies past the ring. The
  skirt's first row stands where the water reaches the far plane, so water within it keeps its own depth; water beyond
  is drawn along its own line of sight just inside the far plane (`surface.wgsl`), so it covers the pixels it would,
  takes the air of its true distance, and is shaded as the open deep (a clear depth behind it is no shore). From 12,000
  ft the horizon is straight to a few pixels across the view and the sea runs up into the haze under it; a hull 40 km
  off sits on her water. Left: where the clipmap is centred ahead of the eye, the ring's own rows past the far plane are
  drawn just inside it too, so in the last ring step before the far plane (3.5 km) the water can stand a little in front
  of a hull. Below it the other way (2026-10-09): rows past the sea's horizon, which the water before them hides, close
  up a 20 m crest's distance beyond it (`CREST_BEYOND`), so a far plane hundreds of kilometres off shades no water only
  for it to fail the depth test (a band two degrees deep under the horizon from a bridge). Each closed-up vertex reads
  its waves at the spacing the ring has there, so the rows closed onto that circle are one line and draw nothing.
- **The far land** (2026-10-09). A theatre view drawing the land beyond its window (`terrain/layer.md`, "The horizon ring")
  renders with its far plane at `HORIZON_FAR` (450 km): `SeaScene::drawn_far` raises it for the drawn frame only,
  while the ring has land on the GPU and the camera is curved. The session's own camera keeps 60 km for the page's
  anchors, tags, sounds and the sun's shadow fit. Depth is reversed, so precision near the eye is the near plane's. The
  sea's ring and the fleet's cull follow the drawn far plane, the aerial volume reaches 450 km in every view
  (`haze.md`), and the clouds' shadow cascades keep a battle view's span (`CascadeSettings::max_far`).
- **A new layer** projects with `world_to_clip`, never `camera.view_projection * vec4(world, 1.0)`, and culls with
  the drawn position.
