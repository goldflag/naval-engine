//! What the host sets on the renderer and reads from it between frames: the pipelines and hot reload, photo mode
//! and the look, the display, the eye, bloom and the sun's glare, edge smoothing and the render scale, shadows and
//! lights.

use crate::aerial::AerialVolume;
use crate::antialiasing::{
    Antialiasing, OutputStage, capped_scale, clamp_render_scale, scene_size,
};
use crate::copies::SceneCopies;
use crate::debug_view::DebugView;
use crate::display::DisplaySettings;
use crate::pipelines::{PipelineRegistry, PipelineStatus};
use crate::post::{
    BloomSettings, EYE_STATE_BUFFER, ExposureControl, EyeAdaptationSettings, SkyMean,
};
use crate::resources::Resources;
use crate::shaders::{ShaderError, ShaderLibrary};
use crate::shadows::{ShadowFit, ShadowMap, ShadowReport, ShadowSettings, Sun};
use crate::taa::TaaSettings;
use crate::timing::GpuTimer;
use crate::upload::{UploadStats, Uploads};

use super::{FrameReport, ReloadReport, Renderer, SCENE_DEPTH};

impl Renderer {
    /// Hands back a report [`Renderer::render`] returned, for the next frame's to be written over (its lists keep
    /// their room): a host that keeps only the latest report passes the one it replaces.
    pub fn recycle_report(&mut self, report: FrameReport) {
        self.spare_report = Some(report);
    }

    /// Builds, ahead of the first frame, every pipeline the output passes can need for an output
    /// of `format`: the display pass onto it and onto the post frame, each edge-smoothing stage
    /// (FXAA, SMAA's blend, temporal AA's present) onto either, the render scale's upscale onto
    /// either, the look's film pass and the overlays. Otherwise each is built by the first frame
    /// that uses it, and a change of antialiasing, render scale or look mid-battle would stall a
    /// frame while it compiles. With build
    /// threads this returns at once; the frame that first needs a pipeline waits for it.
    pub fn prepare_output(&mut self, format: wgpu::TextureFormat) -> Result<(), ShaderError> {
        for display_format in [format, crate::taa::FRAME_FORMAT] {
            self.display.ensure_format(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                display_format,
            )?;
        }
        for stage_format in [format, crate::taa::FRAME_FORMAT] {
            for stage in [OutputStage::Fxaa, OutputStage::SmaaBlend, OutputStage::Taa] {
                self.chain.ensure_stage(
                    &self.device,
                    &mut self.library,
                    &mut self.pipelines,
                    stage,
                    stage_format,
                )?;
            }
        }
        // The upscale writes the output, or the film's frame under a look's film, which the film pass puts on the
        // output (`crate::film`).
        for upscale_format in [format, crate::taa::FRAME_FORMAT] {
            self.chain.ensure_stage(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                OutputStage::Upscale,
                upscale_format,
            )?;
        }
        // The film onto the output, and onto the Look tab's previews (`crate::look_preview`).
        for film_format in [format, crate::look_preview::PREVIEW_FORMAT] {
            self.film.ensure_format(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                film_format,
            )?;
        }
        for entry in &mut self.overlays {
            entry.ensure(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                &self.camera.layout,
                format,
            )?;
        }
        Ok(())
    }

    /// Waits for every pipeline still compiling on a build thread, returning the first one wgpu
    /// rejected. `render` does this itself; a host calls it to end a load with every pipeline
    /// compiled, or to see a first build's error where it registered it.
    pub fn finish_pipelines(&mut self) -> Result<(), ShaderError> {
        self.pipelines.finish()
    }

    /// Pipelines still compiling on build threads.
    pub fn pending_pipelines(&self) -> usize {
        self.pipelines.pending()
    }

    /// Build threads for pipelines registered from now on (native; 0 builds each on the calling
    /// thread). `Renderer::new` starts [`crate::pipelines::default_build_threads`].
    pub fn set_pipeline_build_threads(&mut self, threads: usize) {
        self.pipelines.set_build_threads(threads);
    }

    /// The pipeline cache handed to every pipeline registered from now on (Vulkan's
    /// `Features::PIPELINE_CACHE`; see `crate::pipelines::PipelineBuild::cache`).
    pub fn set_pipeline_cache(&mut self, cache: Option<wgpu::PipelineCache>) {
        self.pipelines.set_cache(cache);
    }

    /// The pipeline cache, whose `get_data` a host persists.
    pub fn pipeline_cache(&self) -> Option<&wgpu::PipelineCache> {
        self.pipelines.cache()
    }

    /// Applies new text for library files and rebuilds the pipelines that
    /// import them, keeping the last good pipeline where a rebuild fails. The
    /// `hot-reload` feature feeds this from disk; any host can call it.
    pub fn apply_shader_changes(&mut self, changes: Vec<(String, String)>) -> ReloadReport {
        let mut report = ReloadReport::default();
        let mut changed = std::collections::BTreeSet::new();
        for (path, text) in changes {
            match self.library.update(&path, text) {
                // The file itself, plus module edits an earlier broken module held back.
                Ok(live) => {
                    for path in live {
                        if changed.insert(path.clone()) {
                            report.changed.push(path);
                        }
                    }
                }
                Err(error) => report.library_errors.push((path, error.message)),
            }
        }
        if !changed.is_empty() {
            report.pipelines =
                self.pipelines
                    .rebuild_affected(&self.device, &mut self.library, &changed);
        }
        report
    }

    /// Checks registered WGSL files on disk (at most every quarter second) and
    /// applies any that changed. `None` when nothing changed.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn poll_shader_files(&mut self) -> Option<ReloadReport> {
        let changes = self.watcher.poll(&self.library);
        if changes.is_empty() {
            return None;
        }
        Some(self.apply_shader_changes(changes))
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// The aerial-perspective volume group 0 binds ([`crate::aerial`]).
    pub fn aerial(&self) -> &AerialVolume {
        &self.camera.aerial
    }

    pub fn library(&self) -> &ShaderLibrary {
        &self.library
    }

    /// The pipeline registry, for a host's own passes outside the layers (labels drawn onto the output) to look up
    /// what it registered through [`Renderer::context`].
    pub fn pipelines(&self) -> &PipelineRegistry {
        &self.pipelines
    }

    pub fn pipeline_status(&self) -> Vec<PipelineStatus> {
        self.pipelines.status()
    }

    /// Photo mode's post (`crate::photo`): its depth of field, exposure compensation, look (in place of the game's,
    /// [`Renderer::set_look`]), grain, vignette and roll; `None` draws the frame with the game's look, with nothing of
    /// photo mode's allocated.
    pub fn set_photo(&mut self, photo: Option<crate::photo::PhotoPost>) {
        if photo.and_then(|p| p.dof).is_none() {
            self.dof.release();
        }
        // The film camera in play keeps the lens's and the shutter's targets while it uses them.
        let play = self.play_camera;
        if photo.and_then(|p| p.optics).is_none() && !play.is_some_and(|play| play.veil) {
            self.optics.release();
        }
        if photo.and_then(|p| p.shutter).is_none() {
            if !play.is_some_and(|play| play.exposure_s > 0.0) {
                self.shutter.release();
            }
            self.photo_bodies = Vec::new();
        }
        self.photo = photo;
    }

    /// The film camera in play (Graphics → Film camera, `crate::play_camera`), or none. Photo mode's camera replaces it
    /// while photo mode is up. Its shutter needs temporal AA's motion: without temporal AA it draws no blur.
    pub fn set_play_camera(&mut self, camera: Option<crate::play_camera::PlayCamera>) {
        if camera != self.play_camera {
            self.play_camera = camera;
            // Let go of what neither it nor photo mode uses now.
            if !camera.is_some_and(|play| play.veil) && self.photo.and_then(|p| p.optics).is_none()
            {
                self.optics.release();
            }
            if !camera.is_some_and(|play| play.exposure_s > 0.0)
                && self.photo.and_then(|p| p.shutter).is_none()
            {
                self.shutter.release();
            }
        }
    }

    pub fn play_camera(&self) -> Option<crate::play_camera::PlayCamera> {
        self.play_camera
    }

    pub fn photo(&self) -> Option<crate::photo::PhotoPost> {
        self.photo
    }

    /// The game's look (`crate::photo::Look`, the Graphics menu's Look): its grade, its halation and diffusion, and
    /// its film after edge smoothing (`crate::film`). Natural draws the frame as it was, with no film pass.
    pub fn set_look(&mut self, look: crate::photo::Look) {
        self.look = look;
    }

    pub fn look(&self) -> crate::photo::Look {
        self.look
    }

    /// Photo mode's moving bodies at the frozen moment, which its shutter blurs by their velocities (world space; the
    /// renderer keeps those in view). Read only while [`crate::photo::PhotoPost::shutter`] is set.
    pub fn set_photo_bodies(&mut self, bodies: &[crate::shutter::MovingBody]) {
        self.photo_bodies.clear();
        self.photo_bodies.extend_from_slice(bodies);
    }

    /// Photo mode's focus probe: the reversed depth the last frame drew at scene texel `at`, copied into a new 16-byte
    /// read-back buffer (the depth first) in a submission of its own, for the caller to map without waiting. `None`
    /// before a frame has been drawn.
    pub fn probe_depth(&mut self, at: (u32, u32)) -> Option<wgpu::Buffer> {
        let depth = self.resources.texture(SCENE_DEPTH)?;
        Some(
            self.dof
                .probe_depth(&self.device, &self.queue, &self.pipelines, &depth.view, at),
        )
    }

    pub fn display_settings(&self) -> DisplaySettings {
        self.display.settings
    }

    pub fn set_display_settings(&mut self, settings: DisplaySettings) {
        self.display.settings = settings;
    }

    /// The display's headroom: how much brighter than SDR white it can show now (Apple's EDR
    /// headroom, or a Windows display's peak over its SDR white), 1 for SDR (the default). Only a
    /// float output ([`crate::extended_range`]) is drawn for it, with AgX's top raised to it (see
    /// [`crate::display`]); 8- and 10-bit outputs are always drawn for SDR. Anything below 1 or
    /// not finite is 1.
    pub fn set_headroom(&mut self, headroom: f32) {
        self.headroom = if headroom.is_finite() {
            headroom.max(1.0)
        } else {
            1.0
        };
    }

    pub fn headroom(&self) -> f32 {
        self.headroom
    }

    /// GPU timings per pass (see [`crate::timing`]): `latest()` and `history()`
    /// hold frames whose timestamps have arrived, two or three frames late.
    pub fn gpu_timer(&self) -> &GpuTimer {
        &self.timer
    }

    pub fn gpu_timer_mut(&mut self) -> &mut GpuTimer {
        &mut self.timer
    }

    /// The renderer's uploads, for a host writing a layer's buffers outside its `prepare`: they go with the next frame's
    /// submission, ahead of its passes.
    pub fn uploads(&mut self) -> &mut Uploads {
        &mut self.uploads
    }

    /// What the last frame uploaded through the renderer's staging belt.
    pub fn upload_stats(&self) -> UploadStats {
        self.upload_stats
    }

    /// Frames submitted so far: the `frame` numbers of [`crate::FrameTimings`].
    pub fn frames_submitted(&self) -> u64 {
        self.submitted
    }

    /// Puts `commands`, recorded outside the renderer, first in the next frame's submission ([`Renderer::render`]),
    /// where a submission of their own just before it would go: a read-back of what the last frame drew, say. A frame
    /// that fails before its submission drops them.
    pub fn submit_with_frame(&mut self, commands: wgpu::CommandBuffer) {
        self.leading.push(commands);
    }

    /// Where [`Renderer::render`] finishes its frame's parts as it records them (`crate::finish`); `None` finishes the
    /// frame in one encoder, where it is submitted.
    pub fn set_finisher(&mut self, finisher: Option<std::sync::Arc<dyn crate::finish::Finisher>>) {
        self.finisher = finisher;
    }

    /// Reads back what timings it can without rendering a frame. Call it after
    /// waiting for the GPU (native: `device.poll(wait)`) to collect the last
    /// frames' timings.
    pub fn drain_gpu_timings(&mut self) {
        self.timer.drain(&self.device, &self.queue);
    }

    /// Bloom on HDR highlights (`BLOOM` in `DisplayTransform.ts`), or `None` for
    /// none (the default, and the Low preset's).
    pub fn set_bloom(&mut self, bloom: Option<BloomSettings>) {
        self.bloom_on = bloom.is_some();
        if let Some(settings) = bloom {
            self.bloom.settings = settings;
        }
    }

    /// The flashes on the screen this frame, about which the bloom glares ([`crate::post::BloomGlare`]); the first
    /// [`crate::post::BLOOM_GLARES`] are taken. They stand until set again: set an empty list when none burns.
    pub fn set_bloom_glares(&mut self, glares: &[crate::post::BloomGlare]) {
        self.bloom.glares.clear();
        self.bloom.glares.extend_from_slice(glares);
    }

    /// The sun the eye glares at (`crate::sun_glare`: a veil about it by what the frame shows of its disc, added by
    /// the bloom, so it follows the bloom's setting); none once it has set, or where no sky is drawn.
    pub fn set_sun_glare(&mut self, sun: Option<crate::photo_optics::FlareSun>) {
        self.sun_glare.sun = sun;
    }

    pub fn bloom(&self) -> Option<BloomSettings> {
        self.bloom_on.then_some(self.bloom.settings)
    }

    /// The bloom's composite (half resolution) once a frame has drawn it.
    pub fn bloom_texture(&self) -> Option<&wgpu::Texture> {
        self.bloom.output_texture()
    }

    /// The eye adaptation's settings. Off by default; turning it off eases the
    /// eye back to the fixed exposure.
    pub fn set_eye_adaptation(&mut self, settings: EyeAdaptationSettings) {
        self.eye.settings = settings;
    }

    pub fn eye_adaptation(&self) -> EyeAdaptationSettings {
        self.eye.settings
    }

    /// Where the eye adaptation reads the sky's mean radiance (the web's
    /// `pmremTexture(environment, up, 1)`). Until one is set the eye holds.
    pub fn set_sky_mean(&mut self, sky_mean: SkyMean) {
        self.eye.sky_mean = sky_mean;
    }

    /// The sun's elevation (degrees) the scene draws, set each frame: the eye's opening and the night grade follow it
    /// about the sunset (`crate::post::daylight`). `None` (the default) follows the clear sky's luminance.
    pub fn set_sun_elevation(&mut self, elevation: Option<f32>) {
        self.eye.sun_elevation = elevation;
    }

    /// The night's lift on real light this frame (`crate::lights::night_lift`: 1 by day, 16,700 after dark), by
    /// which the lights in the sun's units are lifted: after dark the eye may close by as much again when they fill the
    /// view (`crate::post::EyeAdaptationSettings::max_down`). 1 (the default) keeps it to the day's reach.
    pub fn set_night_lift(&mut self, lift: f32) {
        self.eye.night_lift = lift.max(1.0);
    }

    /// The night's light at the sea this frame (lux: the sun's, the moon's beam and the sky's), which the dark-adapted eye
    /// opens for after dark (`crate::post::night_open`), as the night comes in (`set_night_share`).
    pub fn set_night_light(&mut self, lux: f32) {
        self.eye.night_light = lux.max(0.0);
    }

    /// How far the night has come this frame (0 by day and through dusk, toward 1 as the twilight fades), by which the
    /// eye's ceiling goes to its night's (`crate::post::EyeAdaptationSettings::night_highlight`). 0 (the default) keeps the
    /// day's.
    pub fn set_night_share(&mut self, share: f32) {
        self.eye.night_share = share.clamp(0.0, 1.0);
    }

    pub fn sky_mean(&self) -> SkyMean {
        self.eye.sky_mean
    }

    /// The camera's eye at full precision (the f32 [`crate::Camera`] rounds it), for layers that draw about the eye
    /// ([`crate::PrepareContext::eye`]) and temporal AA's reprojection: kilometres from the world's origin an f32 eye
    /// steps by millimetres, which shakes a body seen from a few metres by pixels. Kept until set again; a frame whose
    /// camera's eye it does not round to takes that camera's own.
    pub fn set_eye(&mut self, eye: Option<glam::DVec3>) {
        self.precise_eye = eye;
    }

    /// The frame's eye at full precision ([`crate::PrepareContext::eye`]): the host's ([`Renderer::set_eye`]) while it
    /// rounds to `camera`'s, else that camera's own (a second view, a harness's camera, a host that gives none).
    pub(crate) fn frame_eye(&self, camera: &crate::Camera) -> glam::DVec3 {
        self.precise_eye
            .filter(|eye| eye.as_vec3() == camera.eye)
            .unwrap_or_else(|| camera.eye.as_dvec3())
    }

    /// Where the night grade reads which pixels look through water: a texture over the screen, r the metres under
    /// water where each pixel's view ray starts (positive under), as the ocean's underwater probe writes it. Those
    /// pixels take the grade of the world near the eye whatever their depth: the open deep has none and is not sky.
    /// `None` (the default) for a scene without a sea.
    pub fn set_underwater(&mut self, probe: Option<&wgpu::TextureView>) {
        if self.underwater.0.as_ref() != probe {
            self.underwater = (probe.cloned(), self.underwater.1 + 1);
        }
    }

    /// The air over the sea a long glass sees shimmer in (`crate::shimmer::SeaAir::of_weather`); a typical day's until
    /// the host sets it.
    pub fn set_sea_air(&mut self, air: crate::shimmer::SeaAir) {
        self.sea_air = air;
    }

    /// Whether a long glass shows the air's shimmer (Graphics → Heat shimmer); off until the host turns it on.
    pub fn set_shimmer(&mut self, on: bool) {
        self.shimmer_on = on;
    }

    /// The next frame's eye settles at once on its target (the web game's
    /// `EyeAdaptation.snap()`): a capture must not depend on the frames before it.
    /// Who sets the exposure over the fixed one: the eye ([`ExposureControl::Auto`]), held where it is, or set.
    pub fn set_exposure_control(&mut self, control: ExposureControl) {
        self.eye.control = control;
    }

    pub fn exposure_control(&self) -> ExposureControl {
        self.eye.control
    }

    /// Reads the eye's state back after the next frame, without waiting; [`Renderer::eye_reading`] has it a few
    /// frames later.
    pub fn request_eye_reading(&mut self) {
        self.eye.request_reading();
    }

    /// The eye's state as last read back: stops open (the display's exposure is the fixed one × `exp2` of them),
    /// metered, target, brightest tenth.
    pub fn eye_reading(&self) -> Option<[f32; 4]> {
        self.eye.reading()
    }

    /// The eye readings arrived so far, and whether another is in flight (see `EyeAdaptation::reading_serial`).
    pub fn eye_reading_serial(&self) -> (u64, bool) {
        self.eye.reading_serial()
    }

    /// The debug views this device can show (the wireframe needs `Features::POLYGON_MODE_LINE`).
    pub fn debug_views(&self) -> Vec<DebugView> {
        DebugView::offered(self.device.features())
    }

    pub fn debug_view(&self) -> DebugView {
        self.debug.view
    }

    /// Shows a debug view (see [`crate::debug_view`]); `Off` is the frame as the game draws it. Refused, with the
    /// reason, for a view this device cannot show. A view's pipelines are made when it is first chosen.
    pub fn set_debug_view(&mut self, view: DebugView) -> Result<(), String> {
        if !self.debug_views().contains(&view) {
            return Err(format!(
                "this GPU cannot draw the {} view (it lacks line polygons, wgpu's POLYGON_MODE_LINE)",
                view.name()
            ));
        }
        if view != DebugView::Off && view != DebugView::Wireframe {
            let mut debug = std::mem::take(&mut self.debug);
            let built = debug.build(&mut self.context());
            self.debug = debug;
            built.map_err(|error| format!("the {} view's shader: {error}", view.name()))?;
        }
        let wireframe = view == DebugView::Wireframe;
        if self.pipelines.wireframe() != wireframe {
            self.pipelines
                .set_wireframe(&self.device, &mut self.library, wireframe);
        }
        self.debug.view = view;
        Ok(())
    }

    pub fn snap_eye_adaptation(&mut self) {
        self.eye.snap();
        self.sun_glare.snap();
    }

    /// Back to the fixed exposure (a new scene).
    pub fn reset_eye_adaptation(&mut self) {
        self.eye.reset(&self.queue, &self.resources);
    }

    /// The eye as `from` left it, for a renderer that takes over from `from` in the same view (a theatre's scene swapped
    /// in): the stops it has open and the sun's glare it eases, copied on the GPU by `encoder` (submit it before this
    /// renderer's first frame). A new renderer's eye otherwise opens at the fixed exposure, and the whole frame jumps by
    /// the stops the old one held, then eases back.
    pub fn take_eye_from(&self, encoder: &mut wgpu::CommandEncoder, from: &Renderer) {
        for name in [EYE_STATE_BUFFER, crate::sun_glare::SUN_GLARE_BUFFER] {
            if let (Some(source), Some(target)) =
                (from.resources.buffer(name), self.resources.buffer(name))
            {
                let size = source.buffer.size().min(target.buffer.size());
                encoder.copy_buffer_to_buffer(&source.buffer, 0, &target.buffer, 0, size);
            }
        }
    }

    /// Edge smoothing by the Graphics setting's names (see [`crate::antialiasing`]): none (the default),
    /// FXAA, SMAA, or temporal AA with FXAA where it takes no history. Turning temporal AA on
    /// starts without history.
    pub fn set_antialiasing(&mut self, antialiasing: Antialiasing) {
        if antialiasing == Antialiasing::Taa && self.antialiasing != Antialiasing::Taa {
            self.taa.reset();
        }
        self.antialiasing = antialiasing;
    }

    pub fn antialiasing(&self) -> Antialiasing {
        self.antialiasing
    }

    /// The scene's size over its base's: the Graphics setting's `renderScale` / 100, clamped to
    /// [`crate::RENDER_SCALE_MIN`]–1. Below 1 the scene and the post chain draw smaller and an upscale stretches the
    /// frame onto the output; overlays stay at full resolution (see [`crate::antialiasing`]).
    pub fn set_render_scale(&mut self, scale: f32) {
        self.render_scale = clamp_render_scale(scale);
    }

    pub fn render_scale(&self) -> f32 {
        self.render_scale
    }

    /// The Graphics setting's resolution cap: the most pixels the scene's base holds before the render scale (`None`
    /// for the output's own). An output with more draws its scene from a base of that many pixels in its shape, so a
    /// Retina or 4K screen costs a capped preset no more than a 2560 x 1440 one.
    pub fn set_scene_pixel_cap(&mut self, cap: Option<u64>) {
        self.scene_pixel_cap = cap.filter(|&pixels| pixels > 0);
    }

    pub fn scene_pixel_cap(&self) -> Option<u64> {
        self.scene_pixel_cap
    }

    /// The scene's size for an output of `output` pixels: the render scale of a base the resolution cap may shrink.
    pub fn scene_size(&self, output: (u32, u32)) -> (u32, u32) {
        scene_size(output, self.scene_scale(output))
    }

    /// The scene's size over the output's for an output of `output` pixels (the render scale times the cap's share).
    pub fn scene_scale(&self, output: (u32, u32)) -> f32 {
        capped_scale(output, self.render_scale, self.scene_pixel_cap)
    }

    /// Temporal anti-aliasing (`TemporalAntialiasing.ts`) with these settings, or `None`
    /// to turn it off (edge smoothing then goes to none; FXAA and SMAA are left as they
    /// are). The same as [`Renderer::set_antialiasing`] with [`Antialiasing::Taa`], plus
    /// its tunables.
    pub fn set_taa(&mut self, taa: Option<TaaSettings>) {
        match taa {
            Some(settings) => {
                self.set_antialiasing(Antialiasing::Taa);
                self.taa.settings = settings;
            }
            None if self.antialiasing == Antialiasing::Taa => {
                self.antialiasing = Antialiasing::None;
            }
            None => {}
        }
    }

    pub fn taa(&self) -> Option<TaaSettings> {
        (self.antialiasing == Antialiasing::Taa).then_some(self.taa.settings)
    }

    /// Whether the latest frame blended history (false on the first frame, after a
    /// reset or a resize, and without temporal AA).
    pub fn taa_history_valid(&self) -> bool {
        self.antialiasing == Antialiasing::Taa && self.taa.history_valid()
    }

    /// Drops temporal history: after a camera cut or a scene change a capture must
    /// not smear.
    pub fn reset_temporal(&mut self) {
        self.taa.reset();
    }

    /// Scene copies to take every frame whatever the layers ask for: for
    /// measuring what copies cost, and for looking at them.
    pub fn set_extra_scene_copies(&mut self, copies: SceneCopies) {
        self.extra_copies = copies;
    }

    /// The shadow maps' graphics settings (off by default).
    pub fn shadow_settings(&self) -> ShadowSettings {
        self.shadow_fit.settings
    }

    /// Applies shadow settings from the next frame; a new map size remakes the maps.
    pub fn set_shadow_settings(&mut self, settings: ShadowSettings) {
        self.shadow_fit.settings = settings;
    }

    /// The light the shadow maps are drawn for, and what they follow. `None` draws
    /// no shadows. Set it every frame the sun, the subject or the ships in view move.
    pub fn set_sun(&mut self, sun: Option<Sun>) {
        self.sun = sun;
    }

    pub fn sun(&self) -> Option<&Sun> {
        self.sun.as_ref()
    }

    /// The frame's point lights, every lit layer's (`crate::lights`): the host gathers them (flashes, fires, star shells,
    /// searchlights, lightning) and hands the brightest at the camera here ([`crate::PointLights::brightest`]) before
    /// [`Renderer::render`], which indexes them for the view and draws the maps of the brightest casters.
    pub fn set_point_lights(&mut self, lights: crate::lights::PointLights) {
        self.point_lights = lights;
    }

    pub fn point_lights(&self) -> &crate::lights::PointLights {
        &self.point_lights
    }

    /// The point lights' maps drawn in the latest frame.
    pub fn light_maps(&self) -> &[ShadowMap] {
        &self.light_maps
    }

    /// The maps as fitted for the latest frame.
    pub fn shadow_fit(&self) -> &ShadowFit {
        &self.shadow_fit
    }

    /// The shadow maps' textures: the near, view and land maps' array (layers 0, 1, 2,
    /// then 3, 4, 5 with the land maps on) and the wide map, for diagnostics.
    pub fn shadow_textures(&self) -> (&wgpu::Texture, &wgpu::Texture) {
        (&self.shadow_targets.cascades, &self.shadow_targets.wide)
    }

    pub fn shadow_report(&self) -> ShadowReport {
        let fit = &self.shadow_fit;
        ShadowReport {
            enabled: fit.settings.enabled() && self.sun.is_some(),
            map_size: fit.settings.map_size,
            soft: fit.settings.soft,
            maps: fit
                .maps
                .iter()
                .map(|map| (map.kind, map.size, map.on, map.drawn, map.half_width))
                .collect(),
        }
    }
}
