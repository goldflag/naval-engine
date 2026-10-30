//! A second view of the scene in the same frame ([`Renderer::render_view`]): another camera drawn the shortest way
//! to its own output.

use bytemuck::bytes_of;

use crate::camera::CameraMotion;
use crate::display::DisplayInputs;
use crate::pipelines::PipelineId;
use crate::post::{SKY_MEAN_BUFFER, SkyMean};
use crate::shaders::ShaderError;
use crate::shadows::ShadowFit;
use crate::upload::UploadSink;

use super::layer::{DrawContext, Phase, PrepareContext, SceneLayer};
use super::scene_pass::{begin_scene_pass, copy_scene, ensure_scene_copies, scene_pass_groups};
use super::{FrameInput, RenderOutput, Renderer, SCENE_DEPTH, SCENE_HDR};

/// What a second view of the scene keeps between frames ([`Renderer::render_view`]): its viewport textures at its size,
/// its sun's fit (every map off) and its own eye. Dropped, or [`SecondView::release`]d, when the view is no longer drawn.
#[derive(Default)]
pub struct SecondView {
    textures: crate::resources::ViewTextures,
    shadow_fit: Option<ShadowFit>,
    eye: Option<crate::post::ViewEye>,
}

impl SecondView {
    /// Lets its textures and its eye go; they are made again by its next draw, whose eye settles at once.
    pub fn release(&mut self) {
        self.textures.release();
        self.eye = None;
    }
}

impl Renderer {
    /// A second view of the scene in the same frame, after [`Renderer::render`]: another camera drawn into `output` at
    /// its own size, with its own viewport textures ([`SecondView`]), and nothing of the main view's history touched. It
    /// takes the shortest way to the output: no sun shadows (the maps are the main view's, some held across frames), no
    /// temporal AA, bloom, sun glare, photo mode, film, debug view or overlays, and the scene at the output's size
    /// whatever the render scale. Its eye is its own, metering what its camera sees with the main eye's settings
    /// (`crate::post::ViewEye`), so the main camera looking into the sun or into shadow leaves it as it was. The layers
    /// are those that
    /// can draw a second camera, each told the view first ([`SceneLayer::set_view`]); `input.time.dt` should be 0, so
    /// nothing a layer steps in `prepare` steps twice.
    pub fn render_view(
        &mut self,
        view: &mut SecondView,
        input: FrameInput<'_>,
        layers: &mut [&mut dyn SceneLayer],
        output: RenderOutput<'_>,
    ) -> Result<(), ShaderError> {
        let size = (output.width.max(1), output.height.max(1));
        let display = self.display.ensure_format(
            &self.device,
            &mut self.library,
            &mut self.pipelines,
            output.format,
        )?;
        self.pipelines.finish()?;
        view.textures.set_viewport(size.0, size.1);
        self.resources.swap_view(&self.device, &mut view.textures);
        self.resources.resize(&self.device, size.0, size.1);
        self.draw_view(view, input, layers, output, display);
        self.resources.swap_view(&self.device, &mut view.textures);
        Ok(())
    }

    fn draw_view(
        &mut self,
        view: &mut SecondView,
        input: FrameInput<'_>,
        layers: &mut [&mut dyn SceneLayer],
        output: RenderOutput<'_>,
        display: PipelineId,
    ) {
        let viewport = self.resources.viewport();
        let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
        let unjittered = input.camera.projection(aspect) * input.camera.view();
        let uniforms = input.camera.uniforms(viewport.0, viewport.1, input.time);
        let motion = CameraMotion {
            unjittered_view_projection: unjittered.to_cols_array(),
            previous_view_projection: unjittered.to_cols_array(),
            jitter: [0.0; 2],
            padding: [0.0; 2],
            previous_from_eye: (input.camera.projection(aspect) * input.camera.view_about_eye())
                .to_cols_array(),
            precise_step: [0.0; 4],
        };
        let eye = self.frame_eye(input.camera);
        self.uploads
            .write_buffer(&self.camera.motion, 0, bytes_of(&motion));
        self.uploads
            .write_buffer(&self.camera.buffer, 0, bytes_of(&uniforms));
        // No sun maps of its own: every map off, so its hulls are lit unshadowed. The main view writes its own again.
        let fit = view
            .shadow_fit
            .get_or_insert_with(|| self.shadow_fit.clone());
        fit.update(input.camera, aspect, None);
        let toward_light = self
            .sun
            .as_ref()
            .map_or(glam::Vec3::Y, |sun| sun.toward_light);
        self.uploads.write_buffer(
            &self.shadow_targets.uniforms,
            0,
            bytes_of(&fit.uniforms(toward_light)),
        );
        self.uploads.write_buffer(
            &self.shadow_targets.occluders.uniforms,
            0,
            bytes_of(&crate::shadows::SunOccluderUniform::default()),
        );
        // The lights indexed for its camera, unshadowed (the light maps are the main view's). The main view writes its
        // own again.
        self.write_lights(input.camera, aspect, 0, false);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("second view"),
            });
        self.dither.write(&mut self.uploads, input.time.seconds);
        let copies = layers
            .iter()
            .fold(self.extra_copies, |set, layer| set | layer.scene_copies());
        ensure_scene_copies(&self.device, &mut self.resources, copies);
        let shadow_maps = &fit.maps;
        for layer in layers.iter_mut() {
            encoder.push_debug_group(layer.label());
            layer.prepare(&mut PrepareContext {
                device: &self.device,
                queue: &self.queue,
                encoder: &mut encoder,
                pipelines: &self.pipelines,
                resources: &mut self.resources,
                camera: &uniforms,
                eye,
                camera_group: &self.camera.group,
                time: input.time,
                motion_frame: input.time.frame,
                viewport,
                timer: &self.timer,
                uploads: &mut self.uploads,
                shadow_maps,
                light_maps: &[],
                aerial: &self.camera.aerial,
                sun_occluders: &self.shadow_targets.occluders,
            });
            encoder.pop_debug_group();
        }
        let groups = scene_pass_groups(copies);
        let night_readability = self.eye.sky_mean != SkyMean::None;
        for (index, (phases, point)) in groups.iter().enumerate() {
            {
                let mut pass = begin_scene_pass(
                    &mut encoder,
                    &self.resources,
                    "second view scene",
                    index > 0,
                    None,
                );
                let ctx = DrawContext {
                    pipelines: &self.pipelines,
                    resources: &self.resources,
                    camera: &uniforms,
                    time: input.time,
                    viewport,
                };
                for &phase in phases {
                    if phase == Phase::Transparent && night_readability {
                        self.display.draw_cover(&mut pass, &self.pipelines);
                    }
                    for layer in layers.iter() {
                        pass.push_debug_group(layer.label());
                        pass.set_bind_group(0, &self.camera.group, &[]);
                        layer.draw(phase, &mut pass, &ctx);
                        pass.pop_debug_group();
                    }
                }
                if point.is_some() && night_readability {
                    self.display.draw_hold(&mut pass, &self.pipelines);
                }
            }
            if let Some(point) = *point {
                copy_scene(&mut encoder, &self.resources, copies & point.copies());
                for layer in layers.iter_mut() {
                    encoder.push_debug_group(layer.label());
                    layer.after_copies(
                        point,
                        &mut PrepareContext {
                            device: &self.device,
                            queue: &self.queue,
                            encoder: &mut encoder,
                            pipelines: &self.pipelines,
                            resources: &mut self.resources,
                            camera: &uniforms,
                            eye,
                            camera_group: &self.camera.group,
                            time: input.time,
                            motion_frame: input.time.frame,
                            viewport,
                            timer: &self.timer,
                            uploads: &mut self.uploads,
                            shadow_maps,
                            light_maps: &[],
                            aerial: &self.camera.aerial,
                            sun_occluders: &self.shadow_targets.occluders,
                        },
                    );
                    encoder.pop_debug_group();
                }
            }
        }
        // Its own eye on its own scene, its glare by its own camera.
        let sun_away = self.sun_away(input.camera);
        let eye = view
            .eye
            .get_or_insert_with(|| crate::post::ViewEye::new(&self.device));
        self.eye.encode_view(
            &self.device,
            &mut self.uploads,
            &mut encoder,
            &self.pipelines,
            &self.resources,
            eye,
            input.time.seconds,
            sun_away,
        );
        let hdr = self
            .resources
            .texture(SCENE_HDR)
            .expect("registered in new");
        let depth = self
            .resources
            .texture(SCENE_DEPTH)
            .expect("registered in new");
        let inputs = DisplayInputs {
            source: (&hdr.view, hdr.version),
            depth: (&depth.view, depth.version),
            clip_planes: [input.camera.near, input.camera.far],
            bloom: (&self.black.1, 0),
            bloom_on: false,
            sampler: self.bloom.sampler(),
            eye: eye.state(),
            sky_mean: &self
                .resources
                .buffer(SKY_MEAN_BUFFER)
                .expect("registered by the eye adaptation")
                .buffer,
            night_readability,
            night: crate::post::night_of(self.eye.sun_elevation),
            night_vision: crate::post::night_vision_of(self.eye.sun_elevation),
            underwater: (&self.black.1, 0),
            headroom: 1.0,
            dither: &self.dither,
            photo: None,
            look: self.look,
            preview: 0.0,
            fringes: 0.0,
            shimmer: self
                .shimmer_on
                .then(|| {
                    crate::shimmer::Shimmer::new(
                        input.camera,
                        viewport.0,
                        viewport.1,
                        &self.sea_air,
                        &self.shimmer_clock,
                    )
                })
                .flatten(),
        };
        self.display.encode(
            &self.device,
            &mut UploadSink::Belt(&mut self.uploads),
            &mut encoder,
            self.pipelines.render(display),
            &inputs,
            output.view,
            None,
        );
        let uploads = self.uploads.finish();
        self.queue
            .submit(uploads.into_iter().chain([encoder.finish()]));
        self.uploads.recall();
        self.resources.end_frame();
    }
}
