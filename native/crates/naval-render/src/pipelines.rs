//! Pipelines built from the shader library, kept so they can be rebuilt.
//!
//! Each pipeline is registered with the entry shader it is built from, its
//! defines and a recipe. When a file it depends on changes (hot reload), the
//! registry rebuilds it; when the rebuild fails, the last good pipeline stays
//! and the error is kept for telemetry.
//!
//! **Parallel builds (native).** Compiling a pipeline is mostly the driver's
//! work (Metal compiles each new shader for 50–400 ms the first time a build
//! runs on a machine), and it runs on any thread. With build threads
//! ([`PipelineRegistry::set_build_threads`], which `Renderer::new` turns on),
//! [`PipelineRegistry::add`] composes the shader on the calling thread (so a
//! composition error is still returned at once), hands the module and the
//! recipe to a pool of threads and returns the id. A lookup
//! ([`PipelineRegistry::render`], [`PipelineRegistry::compute`]) waits for its
//! pipeline; [`PipelineRegistry::finish`] waits for all of them and returns a
//! first build's validation error. So a layer that registers its pipelines and
//! looks them up only when it draws lets every layer's pipelines compile
//! together during a load. Hot reload rebuilds in parallel too, then waits.
//! Without build threads (the default, and always on the web), every build runs
//! on the calling thread as it is registered.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::pin;
use std::sync::{Arc, OnceLock};
use std::task::{Context, Poll, Waker};

use crate::shaders::{ShaderDefs, ShaderError, ShaderLibrary};

/// A registered pipeline. Look it up with [`PipelineRegistry::render`] or
/// [`PipelineRegistry::compute`] each frame: a rebuild replaces what it names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PipelineId(usize);

/// What a recipe builds.
pub enum Pipeline {
    Render(wgpu::RenderPipeline),
    Compute(wgpu::ComputePipeline),
}

/// What a recipe is given: the device, the freshly composed shader module and the
/// device's pipeline cache, where it has one (Vulkan; see [`PipelineBuild::cache`]).
pub struct PipelineBuild<'a> {
    pub device: &'a wgpu::Device,
    pub module: &'a wgpu::ShaderModule,
    pub label: &'a str,
    /// Put this in the descriptor's `cache` field. `Some` only where the backend keeps a
    /// pipeline cache (wgpu's `Features::PIPELINE_CACHE`: Vulkan) and the host gave one
    /// ([`PipelineRegistry::set_cache`]); Metal and DX12 cache compiled shaders in the driver.
    pub cache: Option<&'a wgpu::PipelineCache>,
    /// How triangles are rasterized: `Fill`, or `Line` for the wireframe twin of a pipeline that allows one
    /// ([`PipelineRegistry::allow_wireframe`]). A recipe that allows a twin puts it in its `PrimitiveState`.
    pub polygon_mode: wgpu::PolygonMode,
}

/// The function behind a [`PipelineRecipe`]. On native it must be `Send + Sync`, since
/// build threads run it (wgpu's handles are both there).
#[cfg(not(target_arch = "wasm32"))]
pub type RecipeFn = dyn Fn(&PipelineBuild<'_>) -> Pipeline + Send + Sync;
/// The function behind a [`PipelineRecipe`].
#[cfg(target_arch = "wasm32")]
pub type RecipeFn = dyn Fn(&PipelineBuild<'_>) -> Pipeline;

/// Builds a pipeline from a composed module. It captures whatever layouts it
/// needs (wgpu handles are cheap to clone) and runs again on every rebuild.
pub type PipelineRecipe = Box<RecipeFn>;

struct Entry {
    label: String,
    shader: String,
    defs: ShaderDefs,
    recipe: Arc<RecipeFn>,
    built: Built,
    error: Option<String>,
    builds: u32,
    /// Wall time of the latest successful build (module and recipe, on whichever thread ran them).
    build_ms: f64,
    /// Whether the wireframe debug view draws this pipeline as lines ([`PipelineRegistry::allow_wireframe`]).
    wire_allowed: bool,
    /// The wireframe twin, built while the view is on.
    wire: Option<Twin>,
}

/// A pipeline's wireframe twin: built on a pool thread and taken once done, never waited for (the frames before it
/// is ready draw the filled pipeline). `None` inside when its build failed.
struct Twin {
    pipeline: OnceLock<Option<Pipeline>>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Arc<pool::Job>>,
}

impl Twin {
    /// The twin when its build is done, without waiting.
    fn ready(&self, label: &str) -> Option<&Pipeline> {
        if let Some(done) = self.pipeline.get() {
            return done.as_ref();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(job) = self.pending.as_ref().filter(|job| job.finished()) {
            return self
                .pipeline
                .get_or_init(|| match job.take() {
                    Ok(pipeline) => Some(pipeline),
                    Err(error) => {
                        eprintln!("wireframe: {label} has no line pipeline: {error}");
                        None
                    }
                })
                .as_ref();
        }
        let _ = label;
        None
    }
}

/// A pipeline, or the build on a pool thread that will make it.
struct Built {
    pipeline: OnceLock<Pipeline>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Option<Arc<pool::Job>>,
}

impl Built {
    fn ready(pipeline: Pipeline) -> Self {
        let cell = OnceLock::new();
        let _ = cell.set(pipeline);
        Self {
            pipeline: cell,
            #[cfg(not(target_arch = "wasm32"))]
            pending: None,
        }
    }

    /// The pipeline, waiting for its build when it is still on a pool thread. `name` (its label) only names it in
    /// a panic; it is not called `label`, or `tests/gpu_labels.rs` would read every `.get(…)` call in the crates
    /// as one passing a GPU label (URLs in naval-overture, map keys in tests).
    ///
    /// # Panics
    /// If the first build failed (see [`PipelineRegistry::finish`]).
    fn get(&self, name: &str) -> &Pipeline {
        if let Some(pipeline) = self.pipeline.get() {
            return pipeline;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(job) = &self.pending {
            return self.pipeline.get_or_init(|| match job.take() {
                Ok(pipeline) => pipeline,
                Err(error) => panic!("pipeline {name:?} failed its first build: {error}"),
            });
        }
        panic!("pipeline {name:?} has no build")
    }
}

/// A pipeline's state for telemetry.
#[derive(Clone, Debug, PartialEq)]
pub struct PipelineStatus {
    pub label: String,
    pub shader: String,
    /// Successful builds, the first included. 0 while the first is on a build thread.
    pub builds: u32,
    /// Why the latest rebuild failed; the pipeline in use is the last good one.
    pub error: Option<String>,
    /// Wall time of the latest successful build, milliseconds (0 on the web).
    pub build_ms: f64,
}

/// The outcome of rebuilding the pipelines a change touched.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RebuildReport {
    pub rebuilt: Vec<String>,
    /// Label and error of each pipeline that kept its last good build.
    pub failed: Vec<(String, String)>,
}

/// Characters a GPU object label must not contain, besides anything outside ASCII. wgpu's DX12 backend hands a
/// pipeline's label to DXC as the shader's file name, and DXC fails to compile a name Windows cannot open
/// (`error reading 'eye adaptation: meter'`; a `→` in a label once stopped every frame on Windows). Every
/// label in `native/crates` follows this rule (`tests/gpu_labels.rs` scans them).
pub const LABEL_FORBIDDEN: &[char] = &[':', '<', '>', '"', '/', '\\', '|', '?', '*'];

/// Whether `label` is safe as a GPU object label on every backend: ASCII, without [`LABEL_FORBIDDEN`].
pub fn portable_label(label: &str) -> bool {
    label.is_ascii() && !label.contains(LABEL_FORBIDDEN)
}

/// Stand-ins for pipelines still compiling on the registry's build threads. A layer built against
/// the registry (`GameSky::new_in`, `Ocean::new_in`, `ShipRenderer::for_core_in`) keeps these in
/// its pipeline fields and takes the real ones from the registry at its first prepare
/// (`adopt_pipelines`), so its constructor never waits for a compile and every layer's pipelines
/// compile together. They are never bound in a pass: a trivial compute pipeline and a trivial
/// render pipeline (onto `Rgba8Unorm`), made once per renderer
/// ([`crate::RenderContext::placeholders`]).
#[derive(Clone, Debug)]
pub struct PlaceholderPipelines {
    pub compute: wgpu::ComputePipeline,
    pub render: wgpu::RenderPipeline,
}

impl PlaceholderPipelines {
    const WGSL: &'static str = "@compute @workgroup_size(1) fn placeholder_compute() {}
@vertex fn placeholder_vertex() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }
@fragment fn placeholder_fragment() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
";

    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("placeholder pipelines"),
            source: wgpu::ShaderSource::Wgsl(Self::WGSL.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("placeholder pipelines"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let compute = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("placeholder compute"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("placeholder_compute"),
            compilation_options: Default::default(),
            cache: None,
        });
        let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("placeholder render"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("placeholder_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("placeholder_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { compute, render }
    }
}

/// How a layer's constructor gets its pipelines: compiled now on the device (standalone use, tests,
/// tools), or [`PlaceholderPipelines`] it replaces with the renderer registry's pipelines at its first
/// prepare, so that every layer's pipelines compile together on the build threads.
#[derive(Clone, Copy, Debug)]
pub enum MakePipelines<'a> {
    Now,
    Deferred(&'a PlaceholderPipelines),
}

impl<'a> MakePipelines<'a> {
    /// `Deferred` when `placeholders` are given.
    pub fn from_placeholders(placeholders: Option<&'a PlaceholderPipelines>) -> Self {
        placeholders.map_or(Self::Now, Self::Deferred)
    }

    pub fn is_now(self) -> bool {
        matches!(self, Self::Now)
    }

    /// A shader module to build pipelines from now; `None` when they are deferred (no composition).
    pub fn module(self, make: impl FnOnce() -> wgpu::ShaderModule) -> Option<wgpu::ShaderModule> {
        self.is_now().then(make)
    }

    /// `build`'s pipeline now, or the compute stand-in.
    pub fn compute(self, build: impl FnOnce() -> wgpu::ComputePipeline) -> wgpu::ComputePipeline {
        match self {
            Self::Now => build(),
            Self::Deferred(placeholders) => placeholders.compute.clone(),
        }
    }

    /// `build`'s pipeline now, or the render stand-in.
    pub fn render(self, build: impl FnOnce() -> wgpu::RenderPipeline) -> wgpu::RenderPipeline {
        match self {
            Self::Now => build(),
            Self::Deferred(placeholders) => placeholders.render.clone(),
        }
    }
}

/// Build threads [`Renderer::new`](crate::Renderer::new) starts on native: one per core the
/// system reports, at most 16 (0 on the web). The driver does most of the work (Metal compiles in
/// its own service processes), so the threads mostly wait on it.
pub fn default_build_threads() -> usize {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 16))
    }
    #[cfg(target_arch = "wasm32")]
    0
}

/// Pipelines by id. Ids stay valid until [`PipelineRegistry::remove`]; a
/// removed id is never reused.
#[derive(Default)]
pub struct PipelineRegistry {
    entries: Vec<Option<Entry>>,
    cache: Option<wgpu::PipelineCache>,
    /// The wireframe debug view: lookups of pipelines that allow it answer with their line twins.
    wireframe: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pool: Option<pool::BuildPool>,
}

impl PipelineRegistry {
    /// Builds pipelines on `threads` pool threads from now on (native); 0 builds each one on the
    /// calling thread as it is registered, the default. Pending builds keep their threads. On the
    /// web this does nothing.
    pub fn set_build_threads(&mut self, threads: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.pool.as_ref().map_or(0, pool::BuildPool::threads) != threads {
                self.pool = (threads > 0).then(|| pool::BuildPool::new(threads));
            }
        }
        #[cfg(target_arch = "wasm32")]
        let _ = threads;
    }

    /// Build threads in use (0: builds run on the calling thread).
    pub fn build_threads(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pool.as_ref().map_or(0, pool::BuildPool::threads)
        }
        #[cfg(target_arch = "wasm32")]
        0
    }

    /// The pipeline cache recipes are handed from now on ([`PipelineBuild::cache`]). Only a
    /// device with `Features::PIPELINE_CACHE` makes one (Vulkan).
    pub fn set_cache(&mut self, cache: Option<wgpu::PipelineCache>) {
        self.cache = cache;
    }

    /// The pipeline cache, if the host gave one: its `get_data` is what to persist.
    pub fn cache(&self) -> Option<&wgpu::PipelineCache> {
        self.cache.as_ref()
    }

    /// Builds and registers a pipeline. The first build must succeed: there is
    /// no last good pipeline to fall back on. A composition error is returned at
    /// once; with build threads, a pipeline wgpu rejects is reported by
    /// [`PipelineRegistry::finish`] (and a lookup of it panics).
    pub fn add(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        label: impl Into<String>,
        shader: impl Into<String>,
        defs: ShaderDefs,
        recipe: PipelineRecipe,
    ) -> Result<PipelineId, ShaderError> {
        let label = label.into();
        assert!(
            portable_label(&label),
            "pipeline label {label:?} is not portable: GPU labels are ASCII without {LABEL_FORBIDDEN:?} \
             (DX12 passes them to DXC as file names)"
        );
        let shader = shader.into();
        let recipe: Arc<RecipeFn> = Arc::from(recipe);
        let composed = library.compose(&shader, &defs)?;
        let (built, builds, build_ms) = match self.start(
            device,
            &label,
            &shader,
            composed,
            &recipe,
            wgpu::PolygonMode::Fill,
        ) {
            Started::Done(pipeline, ms) => (Built::ready(pipeline?), 1, ms),
            #[cfg(not(target_arch = "wasm32"))]
            Started::Pending(job) => (
                Built {
                    pipeline: OnceLock::new(),
                    pending: Some(job),
                },
                0,
                0.0,
            ),
        };
        self.entries.push(Some(Entry {
            label,
            shader,
            defs,
            recipe,
            built,
            error: None,
            builds,
            build_ms,
            wire_allowed: false,
            wire: None,
        }));
        Ok(PipelineId(self.entries.len() - 1))
    }

    /// Lets the wireframe debug view draw pipeline `id` as lines: its recipe must take
    /// [`PipelineBuild::polygon_mode`] into its primitive state. Opaque scene geometry (hulls, land, aircraft) allows
    /// it; the sky, the sea and blended effects do not. The twin is built when the view is on.
    pub fn allow_wireframe(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        id: PipelineId,
    ) {
        if let Some(Some(entry)) = self.entries.get_mut(id.0) {
            entry.wire_allowed = true;
        }
        if self.wireframe {
            self.start_twin(device, library, id.0);
        }
    }

    /// Turns the wireframe debug view on or off. On, every pipeline that allows it starts building its line twin
    /// (the device needs `Features::POLYGON_MODE_LINE`); lookups answer with a twin once it is built.
    pub fn set_wireframe(&mut self, device: &wgpu::Device, library: &mut ShaderLibrary, on: bool) {
        self.wireframe = on;
        if on {
            for index in 0..self.entries.len() {
                self.start_twin(device, library, index);
            }
        }
    }

    /// Whether the wireframe debug view is on.
    pub fn wireframe(&self) -> bool {
        self.wireframe
    }

    /// Pipelines with a line twin built (for telemetry and tests).
    pub fn wireframe_twins(&self) -> usize {
        self.entries
            .iter()
            .flatten()
            .filter(|entry| {
                entry
                    .wire
                    .as_ref()
                    .is_some_and(|twin| twin.ready(&entry.label).is_some())
            })
            .count()
    }

    /// Starts entry `index`'s line twin when it allows one and has none.
    fn start_twin(&mut self, device: &wgpu::Device, library: &mut ShaderLibrary, index: usize) {
        let Some(Some(entry)) = self.entries.get(index) else {
            return;
        };
        if !entry.wire_allowed || entry.wire.is_some() {
            return;
        }
        let mut module = match library.compose(&entry.shader, &entry.defs) {
            Ok(module) => module,
            Err(error) => {
                eprintln!("wireframe: {} does not compose: {error}", entry.label);
                return;
            }
        };
        flat_fragments(&mut module, WIRE_RADIANCE);
        let label = format!("{} wireframe", entry.label);
        let (shader, recipe) = (entry.shader.clone(), entry.recipe.clone());
        let twin = match self.start(
            device,
            &label,
            &shader,
            module,
            &recipe,
            wgpu::PolygonMode::Line,
        ) {
            Started::Done(result, _) => {
                let cell = OnceLock::new();
                let _ = cell.set(result.map_err(|error| eprintln!("wireframe: {error}")).ok());
                Twin {
                    pipeline: cell,
                    #[cfg(not(target_arch = "wasm32"))]
                    pending: None,
                }
            }
            #[cfg(not(target_arch = "wasm32"))]
            Started::Pending(job) => Twin {
                pipeline: OnceLock::new(),
                pending: Some(job),
            },
        };
        if let Some(Some(entry)) = self.entries.get_mut(index) {
            entry.wire = Some(twin);
        }
    }

    /// Builds `module` with `recipe`: on a pool thread when there is one, else here.
    fn start(
        &self,
        device: &wgpu::Device,
        label: &str,
        shader: &str,
        module: wgpu::naga::Module,
        recipe: &Arc<RecipeFn>,
        polygon_mode: wgpu::PolygonMode,
    ) -> Started {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(pool) = &self.pool {
            return Started::Pending(pool.submit(pool::Task {
                device: device.clone(),
                label: label.to_owned(),
                shader: shader.to_owned(),
                module,
                recipe: recipe.clone(),
                cache: self.cache.clone(),
                polygon_mode,
            }));
        }
        let started = clock::now();
        let pipeline = build_module(
            device,
            label,
            shader,
            module,
            recipe.as_ref(),
            self.cache.as_ref(),
            polygon_mode,
        );
        Started::Done(pipeline, clock::since(started))
    }

    /// Drops a pipeline a layer no longer needs.
    pub fn remove(&mut self, id: PipelineId) {
        if let Some(slot) = self.entries.get_mut(id.0) {
            *slot = None;
        }
    }

    fn entry(&self, id: PipelineId) -> &Entry {
        self.entries
            .get(id.0)
            .and_then(Option::as_ref)
            .unwrap_or_else(|| panic!("pipeline {} was removed or never registered", id.0))
    }

    /// The render pipeline `id` names, waiting for its build if it is on a build thread.
    ///
    /// # Panics
    /// If `id` names a compute pipeline or a removed one, or its first build failed.
    pub fn render(&self, id: PipelineId) -> &wgpu::RenderPipeline {
        let entry = self.entry(id);
        if self.wireframe
            && let Some(Pipeline::Render(twin)) = entry
                .wire
                .as_ref()
                .and_then(|twin| twin.ready(&entry.label))
        {
            return twin;
        }
        match entry.built.get(&entry.label) {
            Pipeline::Render(pipeline) => pipeline,
            Pipeline::Compute(_) => panic!("{} is a compute pipeline", entry.label),
        }
    }

    /// The compute pipeline `id` names, waiting for its build if it is on a build thread.
    ///
    /// # Panics
    /// If `id` names a render pipeline or a removed one, or its first build failed.
    pub fn compute(&self, id: PipelineId) -> &wgpu::ComputePipeline {
        let entry = self.entry(id);
        match entry.built.get(&entry.label) {
            Pipeline::Compute(pipeline) => pipeline,
            Pipeline::Render(_) => panic!("{} is a render pipeline", entry.label),
        }
    }

    /// Pipelines whose first build is still on a build thread (or finished there and not yet
    /// taken in by [`PipelineRegistry::finish`] or a lookup).
    pub fn pending(&self) -> usize {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.entries
                .iter()
                .flatten()
                .filter(|entry| entry.built.pipeline.get().is_none())
                .count()
        }
        #[cfg(target_arch = "wasm32")]
        0
    }

    /// Waits for every first build still on a build thread and takes the pipelines in. Returns
    /// the first one wgpu rejected; that pipeline stays unusable (a lookup panics).
    pub fn finish(&mut self) -> Result<(), ShaderError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut first_error = None;
            for entry in self.entries.iter_mut().flatten() {
                let Some(job) = entry.built.pending.take() else {
                    continue;
                };
                let ms = job.ms();
                if entry.built.pipeline.get().is_some() {
                    // A lookup took it already.
                    entry.builds += 1;
                    entry.build_ms = ms;
                    continue;
                }
                match job.take() {
                    Ok(pipeline) => {
                        let _ = entry.built.pipeline.set(pipeline);
                        entry.builds += 1;
                        entry.build_ms = ms;
                    }
                    Err(error) => {
                        entry.error = Some(error.message.clone());
                        first_error.get_or_insert(error);
                    }
                }
            }
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        Ok(())
    }

    /// Rebuilds every pipeline whose shader imports any of `changed` (library
    /// paths), keeping the last good pipeline where a rebuild fails. With build
    /// threads the rebuilds run in parallel; this returns when all are done.
    pub fn rebuild_affected(
        &mut self,
        device: &wgpu::Device,
        library: &mut ShaderLibrary,
        changed: &BTreeSet<String>,
    ) -> RebuildReport {
        // First builds still running finish first, so a rebuild replaces a real pipeline.
        let _ = self.finish();
        let mut report = RebuildReport::default();
        let mut started = Vec::new();
        for index in 0..self.entries.len() {
            let Some(entry) = &self.entries[index] else {
                continue;
            };
            if entry.built.pipeline.get().is_none()
                || library.dependencies(&entry.shader).is_disjoint(changed)
            {
                continue;
            }
            match library.compose(&entry.shader, &entry.defs) {
                Ok(module) => {
                    let (label, shader) = (entry.label.clone(), entry.shader.clone());
                    let recipe = entry.recipe.clone();
                    started.push((
                        index,
                        self.start(
                            device,
                            &label,
                            &shader,
                            module,
                            &recipe,
                            wgpu::PolygonMode::Fill,
                        ),
                    ));
                }
                Err(error) => started.push((index, Started::Done(Err(error), 0.0))),
            }
        }
        let mut twins = Vec::new();
        for (index, start) in started {
            let (result, ms) = match start {
                Started::Done(result, ms) => (result, ms),
                #[cfg(not(target_arch = "wasm32"))]
                Started::Pending(job) => (job.take(), job.ms()),
            };
            let entry = self.entries[index]
                .as_mut()
                .expect("an entry being rebuilt stays registered");
            match result {
                Ok(pipeline) => {
                    entry.built = Built::ready(pipeline);
                    entry.error = None;
                    entry.builds += 1;
                    entry.build_ms = ms;
                    // Its line twin is built again from the new shader.
                    entry.wire = None;
                    report.rebuilt.push(entry.label.clone());
                    twins.push(index);
                }
                Err(error) => {
                    entry.error = Some(error.to_string());
                    report.failed.push((entry.label.clone(), error.to_string()));
                }
            }
        }
        if self.wireframe {
            for index in twins {
                self.start_twin(device, library, index);
            }
        }
        report
    }

    pub fn status(&self) -> Vec<PipelineStatus> {
        self.entries
            .iter()
            .flatten()
            .map(|entry| PipelineStatus {
                label: entry.label.clone(),
                shader: entry.shader.clone(),
                builds: entry.builds,
                error: entry.error.clone(),
                build_ms: entry.build_ms,
            })
            .collect()
    }

    /// Live pipelines.
    pub fn len(&self) -> usize {
        self.entries.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Where a build went: done here (with its wall time), or to a pool thread.
enum Started {
    Done(Result<Pipeline, ShaderError>, f64),
    #[cfg(not(target_arch = "wasm32"))]
    Pending(Arc<pool::Job>),
}

/// The wireframe's lines: one linear HDR radiance, a pale cyan that reads over sea and sky at the game's exposure.
pub const WIRE_RADIANCE: [f32; 4] = [0.45, 0.95, 1.3, 1.0];

/// Every fragment entry of `module` returning one `vec4<f32>` made to return `colour` and nothing else: a line twin's
/// shading. A layer's own fragment code cannot run on lines: shading built from screen-space derivatives (the hulls'
/// normals, for one) is degenerate along a line and gives NaN, which the post chain would spread. Entries returning
/// anything else are left as they are.
pub fn flat_fragments(module: &mut wgpu::naga::Module, colour: [f32; 4]) {
    use wgpu::naga::{
        Block, Expression, Function, Literal, Range, Scalar, ShaderStage, Span, Statement,
        TypeInner, VectorSize,
    };
    for entry in &mut module.entry_points {
        if entry.stage != ShaderStage::Fragment {
            continue;
        }
        let Some(result) = entry.function.result.clone() else {
            continue;
        };
        let vec4 = matches!(
            module.types[result.ty].inner,
            TypeInner::Vector {
                size: VectorSize::Quad,
                scalar: Scalar::F32
            }
        );
        if !vec4 {
            continue;
        }
        let mut function = Function {
            name: entry.function.name.clone(),
            arguments: entry.function.arguments.clone(),
            result: Some(result.clone()),
            ..Default::default()
        };
        let components = colour
            .iter()
            .map(|c| {
                function
                    .expressions
                    .append(Expression::Literal(Literal::F32(*c)), Span::UNDEFINED)
            })
            .collect();
        let value = function.expressions.append(
            Expression::Compose {
                ty: result.ty,
                components,
            },
            Span::UNDEFINED,
        );
        function.body = Block::from_vec(vec![
            Statement::Emit(Range::new_from_bounds(value, value)),
            Statement::Return { value: Some(value) },
        ]);
        entry.function = function;
    }
}

/// Makes the shader module and runs the recipe inside a validation error scope
/// (error scopes are per thread in wgpu, so this works on any thread).
fn build_module(
    device: &wgpu::Device,
    label: &str,
    shader: &str,
    module: wgpu::naga::Module,
    recipe: &RecipeFn,
    cache: Option<&wgpu::PipelineCache>,
    polygon_mode: wgpu::PolygonMode,
) -> Result<Pipeline, ShaderError> {
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
    });
    let pipeline = recipe(&PipelineBuild {
        device,
        module: &module,
        label,
        cache,
        polygon_mode,
    });
    // Native wgpu reports the scope at once. On the web the answer comes later,
    // so the pipeline is taken and any error surfaces as an uncaptured one.
    if let Some(Some(error)) = poll_now(scope.pop()) {
        return Err(ShaderError {
            path: shader.to_owned(),
            message: error.to_string(),
        });
    }
    Ok(pipeline)
}

/// Polls a future once without blocking: `Some` if it was already done.
fn poll_now<F: Future>(future: F) -> Option<F::Output> {
    let mut future = pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => Some(value),
        Poll::Pending => None,
    }
}

/// Build times: `std::time::Instant` panics on `wasm32-unknown-unknown`, so the web reports 0.
mod clock {
    #[cfg(not(target_arch = "wasm32"))]
    pub struct Stamp(std::time::Instant);
    #[cfg(target_arch = "wasm32")]
    pub struct Stamp;

    pub fn now() -> Stamp {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Stamp(std::time::Instant::now())
        }
        #[cfg(target_arch = "wasm32")]
        {
            Stamp
        }
    }

    pub fn since(stamp: Stamp) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            stamp.0.elapsed().as_secs_f64() * 1e3
        }
        #[cfg(target_arch = "wasm32")]
        {
            let Stamp = stamp;
            0.0
        }
    }
}

/// The build threads (native only).
#[cfg(not(target_arch = "wasm32"))]
mod pool {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc::{Sender, channel};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread::JoinHandle;

    use super::{Pipeline, RecipeFn, ShaderError, build_module, clock};

    /// One pipeline to build.
    pub struct Task {
        pub device: wgpu::Device,
        pub label: String,
        pub shader: String,
        pub module: wgpu::naga::Module,
        pub recipe: Arc<RecipeFn>,
        pub cache: Option<wgpu::PipelineCache>,
        pub polygon_mode: wgpu::PolygonMode,
    }

    /// A build's result, waited for by whoever needs the pipeline first.
    #[derive(Default)]
    pub struct Job {
        state: Mutex<State>,
        done: Condvar,
    }

    #[derive(Default)]
    struct State {
        finished: bool,
        result: Option<Result<Pipeline, ShaderError>>,
        ms: f64,
    }

    impl Job {
        fn finish(&self, result: Result<Pipeline, ShaderError>, ms: f64) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.result = Some(result);
            state.ms = ms;
            state.finished = true;
            self.done.notify_all();
        }

        fn wait(&self) -> std::sync::MutexGuard<'_, State> {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            while !state.finished {
                state = self.done.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            state
        }

        /// Waits for the build and takes its result (once).
        pub fn take(&self) -> Result<Pipeline, ShaderError> {
            self.wait()
                .result
                .take()
                .expect("a build's result is taken once")
        }

        /// Whether the build is done (its result may be taken without waiting).
        pub fn finished(&self) -> bool {
            self.state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finished
        }

        /// Waits for the build and returns its wall time, milliseconds.
        pub fn ms(&self) -> f64 {
            self.wait().ms
        }
    }

    type Work = Box<dyn FnOnce() + Send>;

    pub struct BuildPool {
        sender: Option<Sender<Work>>,
        workers: Vec<JoinHandle<()>>,
    }

    impl BuildPool {
        pub fn new(threads: usize) -> Self {
            let (sender, receiver) = channel::<Work>();
            let receiver = Arc::new(Mutex::new(receiver));
            let workers = (0..threads)
                .map(|index| {
                    let receiver = receiver.clone();
                    std::thread::Builder::new()
                        .name(format!("pipeline build {index}"))
                        .spawn(move || {
                            loop {
                                // The guard drops at the end of the statement, before the work runs.
                                let work =
                                    receiver.lock().unwrap_or_else(|e| e.into_inner()).recv();
                                match work {
                                    Ok(work) => work(),
                                    Err(_) => break,
                                }
                            }
                        })
                        .expect("spawning a pipeline build thread")
                })
                .collect();
            Self {
                sender: Some(sender),
                workers,
            }
        }

        pub fn threads(&self) -> usize {
            self.workers.len()
        }

        pub fn submit(&self, task: Task) -> Arc<Job> {
            let job = Arc::new(Job::default());
            let done = job.clone();
            let work: Work = Box::new(move || {
                let started = clock::now();
                let result = catch_unwind(AssertUnwindSafe(|| {
                    build_module(
                        &task.device,
                        &task.label,
                        &task.shader,
                        task.module,
                        task.recipe.as_ref(),
                        task.cache.as_ref(),
                        task.polygon_mode,
                    )
                }))
                .unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<String>()
                        .cloned()
                        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                        .unwrap_or_else(|| "the build panicked".to_owned());
                    Err(ShaderError {
                        path: task.shader.clone(),
                        message,
                    })
                });
                done.finish(result, clock::since(started));
            });
            self.sender
                .as_ref()
                .expect("the pool is open")
                .send(work)
                .expect("the pipeline build threads are running");
            job
        }
    }

    /// Closing the queue ends each thread once its current build is done. They are not joined: a renderer replaced
    /// mid-load (a session's first renderer, superseded by the battle's) must not wait for builds nobody needs.
    impl Drop for BuildPool {
        fn drop(&mut self) {
            drop(self.sender.take());
            self.workers.clear();
        }
    }
}
