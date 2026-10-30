//! [`Renderer::render`]: one frame, recorded phase by phase in the order the `frame` module's header lists, then submitted.

use bytemuck::bytes_of;

use crate::antialiasing::{Antialiasing, OutputStage, POST_FRAME, PostChain, StageTarget};
use crate::camera::{CameraMotion, CameraUniforms};
use crate::copies::SceneCopies;
use crate::display::DisplayInputs;
use crate::pipelines::PipelineId;
use crate::post::{EYE_STATE_BUFFER, SKY_MEAN_BUFFER, SkyMean};
use crate::shaders::ShaderError;
use crate::taa::{DISPLAY_FRAME, SCENE_MOTION, TAA_RESOLVED, Taa};
use crate::upload::UploadSink;

use super::layer::{
    DrawContext, OverlayContext, OverlayPipelineId, Phase, PrepareContext, SceneLayer,
};
use super::scene_pass::{
    begin_scene_pass, copy_scene, ensure_scene_copies, scene_pass_groups, shadow_label,
};
use super::{
    DEPTH_CLEAR, FrameInput, FrameReport, RenderOutput, Renderer, SCENE_DEPTH, SCENE_HDR,
    extended_range,
};

/// What a frame draws through, settled before its first pass: the scene's size, the chain after the display pass, the
/// look and its film, and the pipelines each step draws with.
#[derive(Clone, Copy)]
struct FramePlan {
    scene: (u32, u32),
    taa_on: bool,
    output_size: (u32, u32),
    upscale: bool,
    stage: Option<OutputStage>,
    display_direct: bool,
    look: crate::photo::Look,
    film: crate::film::FilmFinish,
    film_on: bool,
    display: PipelineId,
    stage_pipeline: Option<PipelineId>,
    upscale_pipeline: Option<PipelineId>,
    film_pipeline: Option<PipelineId>,
    overlay: bool,
    debug_pass: Option<OverlayPipelineId>,
}

/// This frame's camera as the scene draws it: the viewport, the uniforms (jittered under temporal AA), the eye at full
/// precision, temporal AA's frame and the frame the motion is for.
#[derive(Clone, Copy)]
struct FrameCamera {
    viewport: (u32, u32),
    uniforms: CameraUniforms,
    /// The eye at full precision ([`PrepareContext::eye`]).
    eye: glam::DVec3,
    taa_frame: Option<crate::taa::TaaFrame>,
    motion_frame: u64,
}

impl Renderer {
    /// Records and submits one frame: prepare, the sun's shadow maps, the scene
    /// pass, the display pass.
    /// The scene targets are resized to the output first.
    pub fn render(
        &mut self,
        input: FrameInput<'_>,
        layers: &mut [&mut dyn SceneLayer],
        output: RenderOutput<'_>,
    ) -> Result<FrameReport, ShaderError> {
        // Dropped unsubmitted if the frame fails before its submission.
        let leading = std::mem::take(&mut self.leading);
        self.parts.forget();
        let plan = self.plan_frame(input, layers, output)?;
        self.size_targets(&plan);
        let cam = self.place_camera(input, &plan);
        self.fit_shadows(input, cam.viewport);
        let (mut encoder, copies) = self.begin_encoder(input, layers);
        self.prepare_layers(input, layers, &cam, &mut encoder);
        // The frame's report, written over one handed back (its lists keep their room).
        let mut report = self.spare_report.take().unwrap_or_default();
        self.draw_shadow_maps(input, layers, &mut encoder, &mut report);
        self.draw_contact_shadows(input, layers, &cam, &mut encoder, &mut report);
        self.draw_scene(input, layers, &cam, copies, &mut encoder, &mut report);
        self.draw_motion(input, layers, &cam, &mut encoder);
        self.draw_shutter(input, &cam, &mut encoder);
        self.meter_eye_and_bloom(input, &mut encoder);
        self.draw_display(input, output, &plan, &cam, &mut encoder);
        self.draw_output_chain(output, &plan, &cam, &mut encoder);
        self.draw_debug_view(output, &plan, &mut encoder);
        self.draw_overlays(input, layers, output, &plan, &cam, &mut encoder);
        self.submit_frame(encoder, leading);
        report.frame = input.time.frame;
        report.time = input.time.seconds;
        report.viewport = cam.viewport;
        report.copies = copies;
        report.layers.truncate(layers.len());
        for (index, layer) in layers.iter().enumerate() {
            match report.layers.get_mut(index) {
                Some(held) => {
                    held.clear();
                    held.push_str(layer.label());
                }
                None => report.layers.push(layer.label().to_owned()),
            }
        }
        Ok(report)
    }

    /// The frame's output chain and look, and the pipelines they draw with, built for this output; pipelines still
    /// compiling are waited for here.
    fn plan_frame(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
        output: RenderOutput<'_>,
    ) -> Result<FramePlan, ShaderError> {
        // The scene is drawn at the render scale's size. Without edge smoothing at full
        // scale the display pass writes the output; otherwise it writes `display.frame`
        // for the chain's last stage (FXAA, SMAA's blend, temporal AA's output), which
        // writes the output, or `post.frame` for the bilinear upscale below full scale
        // (temporal AA resolves at the output's size instead).
        let taa_on = self.antialiasing == Antialiasing::Taa;
        let scene = self.scene_size((output.width, output.height));
        let output_size = (output.width.max(1), output.height.max(1));
        // Below full scale temporal AA upscales the frame itself (its resolve and output pass are at
        // the output's size); FXAA and SMAA are stretched onto the output bilinearly.
        let temporal_upscale = taa_on && scene != output_size;
        let upscale = scene != output_size && !temporal_upscale;
        let stage = match self.antialiasing {
            Antialiasing::None => None,
            Antialiasing::Fxaa => Some(OutputStage::Fxaa),
            Antialiasing::Smaa => Some(OutputStage::SmaaBlend),
            Antialiasing::Taa => Some(OutputStage::Taa),
        };
        let display_direct = stage.is_none() && !upscale;
        // A look's film goes on last, at the output's size (`crate::film`): whichever pass would write the output
        // writes `film.frame` instead, and the film pass writes the output.
        let look = self.photo.as_ref().map_or(self.look, |photo| photo.look);
        // The grade comes before temporal AA, whose history (kept while time is held) would blend the last look's
        // colours into the new one's.
        if look != self.drawn_look {
            self.drawn_look = look;
            self.taa.reset();
        }
        let film = crate::film::FilmFinish::new(look, self.photo.as_ref(), input.time.seconds);
        let film_on = film.is_on();
        let last_format = if film_on {
            crate::taa::FRAME_FORMAT
        } else {
            output.format
        };
        let display_format = if display_direct {
            last_format
        } else {
            crate::taa::FRAME_FORMAT
        };
        let display = self.display.ensure_format(
            &self.device,
            &mut self.library,
            &mut self.pipelines,
            display_format,
        )?;
        let stage_format = if upscale {
            crate::taa::FRAME_FORMAT
        } else {
            last_format
        };
        let stage_pipeline = match stage {
            Some(stage) => Some(self.chain.ensure_stage(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                stage,
                stage_format,
            )?),
            None => None,
        };
        let upscale_pipeline = if upscale {
            Some(self.chain.ensure_stage(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                OutputStage::Upscale,
                last_format,
            )?)
        } else {
            None
        };
        let film_pipeline = if film_on {
            Some(self.film.ensure_format(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                output.format,
            )?)
        } else {
            None
        };
        self.output_format = output.format;
        let overlay = layers.iter().any(|layer| layer.has_overlay());
        let debug_pass = self.debug.pipeline();
        if overlay || debug_pass.is_some() {
            for entry in &mut self.overlays {
                entry.ensure(
                    &self.device,
                    &mut self.library,
                    &mut self.pipelines,
                    &self.camera.layout,
                    output.format,
                )?;
            }
        }
        // Pipelines still compiling on build threads are waited for here, before any is drawn.
        self.pipelines.finish()?;
        Ok(FramePlan {
            scene,
            taa_on,
            output_size,
            upscale,
            stage,
            display_direct,
            look,
            film,
            film_on,
            display,
            stage_pipeline,
            upscale_pipeline,
            film_pipeline,
            overlay,
            debug_pass,
        })
    }

    /// The scene's targets at its size, and the contact shadows', the post chain's and the film's as this frame
    /// needs them.
    fn size_targets(&mut self, plan: &FramePlan) {
        let FramePlan {
            scene,
            output_size,
            upscale,
            stage,
            display_direct,
            film_on,
            ..
        } = *plan;
        self.resources.resize(&self.device, scene.0, scene.1);
        self.contact.ensure_targets(
            &self.device,
            &mut self.resources,
            self.shadow_fit.settings.contact_on(),
        );
        let chain_dropped = PostChain::ensure_targets(
            &self.device,
            &mut self.resources,
            !display_direct,
            upscale && stage.is_some(),
            self.antialiasing == Antialiasing::Smaa,
        );
        if chain_dropped {
            // Its cached groups bind the dropped targets; they would keep them alive.
            self.chain.forget_groups();
        }
        self.film
            .ensure_target(&self.device, &mut self.resources, output_size, film_on);
    }

    /// This frame's camera: the scene's viewport, the camera's uniforms (jittered under temporal AA) and its motion,
    /// written through the frame's staging belt.
    fn place_camera(&mut self, input: FrameInput<'_>, plan: &FramePlan) -> FrameCamera {
        let FramePlan {
            taa_on,
            output_size,
            ..
        } = *plan;
        let viewport = self.resources.viewport();
        let unjittered = input
            .camera
            .projection(viewport.0 as f32 / viewport.1.max(1) as f32)
            * input.camera.view();
        let eye = self.frame_eye(input.camera);
        let (uniforms, taa_frame) = if taa_on {
            Taa::ensure_targets(&self.device, &mut self.resources, output_size);
            // No time passed (the presentation held: a pause): a still camera repeats the last
            // frame, so the picture holds.
            let held = input.time.dt <= 0.0;
            // A scene drawn smaller than the output takes more jitter phases to cover it.
            let phases = crate::taa::jitter_phases(viewport.0 as f32 / output_size.0 as f32);
            let frame = self.taa.begin(
                input.time.frame,
                input.camera,
                unjittered,
                viewport,
                held,
                phases,
                eye,
            );
            // A new frame's step is the interval its motion spans; a frame drawn again keeps the last one's.
            if frame.new_frame {
                self.motion_interval = input.time.dt.max(0.0);
            }
            let uniforms =
                input
                    .camera
                    .uniforms_jittered(viewport.0, viewport.1, input.time, frame.jitter);
            (uniforms, Some(frame))
        } else {
            // The post chain's cached groups bind `taa.resolved`; they would keep it alive.
            if self.taa.release_targets(&mut self.resources) {
                self.chain.forget_groups();
            }
            (
                input.camera.uniforms(viewport.0, viewport.1, input.time),
                None,
            )
        };
        let motion_frame = taa_frame.map_or(input.time.frame, |frame| frame.frame);
        let motion = CameraMotion {
            unjittered_view_projection: unjittered.to_cols_array(),
            previous_view_projection: taa_frame
                .map_or(unjittered, |frame| frame.previous_view_projection)
                .to_cols_array(),
            jitter: taa_frame.map_or([0.0; 2], |frame| frame.jitter.to_array()),
            padding: [0.0; 2],
            previous_from_eye: taa_frame
                .map_or_else(
                    || {
                        input
                            .camera
                            .projection(viewport.0 as f32 / viewport.1.max(1) as f32)
                            * input.camera.view_about_eye()
                    },
                    |frame| frame.previous_from_eye,
                )
                .to_cols_array(),
            precise_step: taa_frame
                .map_or(glam::Vec3::ZERO, |frame| frame.precise_step)
                .extend(0.0)
                .to_array(),
        };
        // The frame's own uniforms through its staging belt too: in place before its passes, as the layers' are.
        self.uploads
            .write_buffer(&self.camera.motion, 0, bytes_of(&motion));
        self.uploads
            .write_buffer(&self.camera.buffer, 0, bytes_of(&uniforms));
        FrameCamera {
            viewport,
            uniforms,
            eye,
            taa_frame,
            motion_frame,
        }
    }

    /// Fits the sun's maps to this frame's camera and picks the point lights' maps, remaking the shadow targets
    /// their sizes ask for, and writes the lights' and the maps' uniforms.
    fn fit_shadows(&mut self, input: FrameInput<'_>, viewport: (u32, u32)) {
        // Fit the sun's maps to this frame's camera, and remake them if their size changed.
        let aspect = viewport.0 as f32 / viewport.1.max(1) as f32;
        self.shadow_fit
            .update(input.camera, aspect, self.sun.as_ref());
        // The brightest casters among the point lights take maps of their own, layers of the same array.
        let settings = &self.shadow_fit.settings;
        let mut light_maps = std::mem::take(&mut self.light_maps);
        light_maps.clear();
        light_maps.extend(crate::lights::light_shadow_maps(
            &self.point_lights,
            settings.light_map_count(),
            settings.light_map_size(),
            input.camera.eye,
            input.camera.target,
        ));
        if self
            .shadow_targets
            .ensure(&self.device, &self.shadow_fit.settings, light_maps.len())
        {
            // New textures hold nothing: draw every map sampled this frame.
            for map in &mut self.shadow_fit.maps {
                map.drawn |= map.on;
            }
        }
        self.camera.follow(&self.device, &self.shadow_targets);
        let (first_light_layer, light_layers) = self.shadow_targets.light_layers();
        light_maps.truncate(light_layers as usize);
        self.light_maps = light_maps;
        self.write_lights(input.camera, aspect, first_light_layer, true);
        let toward_light = self
            .sun
            .as_ref()
            .map_or(glam::Vec3::Y, |sun| sun.toward_light);
        self.uploads.write_buffer(
            &self.shadow_targets.uniforms,
            0,
            bytes_of(&self.shadow_fit.uniforms(toward_light)),
        );
        // No soft occluders until a layer draws them this frame (a layer's own write, through the same belt, follows).
        self.uploads.write_buffer(
            &self.shadow_targets.occluders.uniforms,
            0,
            bytes_of(&crate::shadows::SunOccluderUniform::default()),
        );
    }

    /// The frame's command encoder with its timer begun, the eye's reading polled, the sky's mean and the dither
    /// written, and the scene copies the layers ask for made.
    fn begin_encoder(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
    ) -> (wgpu::CommandEncoder, SceneCopies) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        self.timer
            .begin_frame(&self.device, &mut encoder, self.submitted);
        self.eye.poll_reading(&self.device);
        self.eye.write_sky_mean(&mut self.uploads, &self.resources);
        self.dither.write(&mut self.uploads, input.time.seconds);
        let copies = layers
            .iter()
            .fold(self.extra_copies, |set, layer| set | layer.scene_copies());
        ensure_scene_copies(&self.device, &mut self.resources, copies);
        (encoder, copies)
    }

    /// Each layer's `prepare`, then the frame so far handed over to finish.
    fn prepare_layers(
        &mut self,
        input: FrameInput<'_>,
        layers: &mut [&mut dyn SceneLayer],
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let FrameCamera {
            viewport,
            uniforms,
            eye,
            motion_frame,
            ..
        } = *cam;
        for layer in layers.iter_mut() {
            encoder.push_debug_group(layer.label());
            layer.prepare(&mut PrepareContext {
                device: &self.device,
                queue: &self.queue,
                encoder,
                pipelines: &self.pipelines,
                resources: &mut self.resources,
                camera: &uniforms,
                eye,
                camera_group: &self.camera.group,
                time: input.time,
                motion_frame,
                viewport,
                timer: &self.timer,
                uploads: &mut self.uploads,
                shadow_maps: &self.shadow_fit.maps,
                light_maps: &self.light_maps,
                aerial: &self.camera.aerial,
                sun_occluders: &self.shadow_targets.occluders,
            });
            encoder.pop_debug_group();
        }
        // The layers' own passes finish while the shadow maps are recorded.
        self.parts
            .hand_over(&self.device, self.finisher.as_ref(), encoder);
    }

    /// The sun's shadow maps due this frame, then the point lights', each a depth-only pass every layer draws
    /// into; then the frame so far handed over to finish.
    fn draw_shadow_maps(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
        encoder: &mut wgpu::CommandEncoder,
        report: &mut FrameReport,
    ) {
        // The sun's shadow maps due this frame.
        let shadow_maps = &mut report.shadow_maps;
        shadow_maps.clear();
        for map in self.shadow_fit.maps.iter().filter(|map| map.drawn) {
            let (group, light) =
                self.shadow_targets
                    .caster_group(&mut self.uploads, map, input.time);
            let label = shadow_label(map.kind);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                timestamp_writes: self.timer.render_pass(label),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.shadow_targets.target(map.kind),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let ctx = DrawContext {
                pipelines: &self.pipelines,
                resources: &self.resources,
                camera: &light,
                time: input.time,
                viewport: (map.size, map.size),
            };
            for layer in layers.iter() {
                pass.push_debug_group(layer.label());
                pass.set_bind_group(0, group, &[]);
                layer.draw_shadow(map, &mut pass, &ctx);
                pass.pop_debug_group();
            }
            shadow_maps.push(map.kind.name());
        }
        // The point lights' maps: a perspective camera at each light, the hulls cast.
        for map in &self.light_maps {
            let (group, light) =
                self.shadow_targets
                    .caster_group(&mut self.uploads, map, input.time);
            let label = shadow_label(map.kind);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(label),
                timestamp_writes: self.timer.render_pass(label),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.shadow_targets.target(map.kind),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            let ctx = DrawContext {
                pipelines: &self.pipelines,
                resources: &self.resources,
                camera: &light,
                time: input.time,
                viewport: (map.size, map.size),
            };
            for layer in layers.iter() {
                pass.push_debug_group(layer.label());
                pass.set_bind_group(0, group, &[]);
                layer.draw_light_shadow(map, &mut pass, &ctx);
                pass.pop_debug_group();
            }
            shadow_maps.push(map.kind.name());
        }
        self.parts
            .hand_over(&self.device, self.finisher.as_ref(), encoder);
    }

    /// Contact shadows ([`crate::contact`]): their depth and march, or their mask and depth cleared.
    fn draw_contact_shadows(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
        report: &mut FrameReport,
    ) {
        let FrameCamera {
            viewport, uniforms, ..
        } = *cam;
        // Contact shadows (`crate::contact`): the surfaces that take them drawn depth-only for this frame's camera, then
        // the march toward the light into the mask; the mask cleared to lit, and the depth to the far plane, when no
        // layer draws them or no light shines.
        report.contact = false;
        if self.shadow_fit.settings.contact_on() {
            let march =
                self.sun.is_some() && layers.iter().any(|layer| layer.draws_contact_depth());
            let mask = self
                .resources
                .texture(crate::contact::CONTACT_MASK)
                .expect("made while contact shadows are on");
            if march {
                let depth = self
                    .resources
                    .texture(crate::contact::CONTACT_DEPTH)
                    .expect("made while contact shadows are on");
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("contact depth"),
                        timestamp_writes: self.timer.render_pass("contact depth"),
                        color_attachments: &[],
                        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                            view: &depth.view,
                            depth_ops: Some(wgpu::Operations {
                                load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                                store: wgpu::StoreOp::Store,
                            }),
                            stencil_ops: None,
                        }),
                        ..Default::default()
                    });
                    let ctx = DrawContext {
                        pipelines: &self.pipelines,
                        resources: &self.resources,
                        camera: &uniforms,
                        time: input.time,
                        viewport,
                    };
                    for layer in layers.iter().filter(|layer| layer.draws_contact_depth()) {
                        pass.push_debug_group(layer.label());
                        pass.set_bind_group(0, self.contact.camera_group(), &[]);
                        layer.draw_contact_depth(&mut pass, &ctx);
                        pass.pop_debug_group();
                    }
                }
                let pipeline = self.pipelines.render(self.contact.pipeline());
                let group = self.contact.group(&self.device, depth);
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("contact shadows"),
                    timestamp_writes: self.timer.render_pass("contact shadows"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mask.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.camera.group, &[]);
                pass.set_bind_group(1, group, &[]);
                pass.draw(0..3, 0..1);
                report.contact = true;
                self.contact.mark_marched();
            } else if self.contact.needs_clear(mask.version) {
                // The depth too, to the far plane: the hulls march the fires' light through it (`naval-ships`'
                // `fire_seen`), and what the last march left must not hide them.
                let depth = self
                    .resources
                    .texture(crate::contact::CONTACT_DEPTH)
                    .expect("made while contact shadows are on");
                encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("contact shadows cleared"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &mask.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth.view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Clear(DEPTH_CLEAR),
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
            }
        }
    }

    /// The scene pass in its groups: the copies taken where a group ends and the layers' passes on them, each
    /// group handed over to finish while the next is recorded.
    fn draw_scene(
        &mut self,
        input: FrameInput<'_>,
        layers: &mut [&mut dyn SceneLayer],
        cam: &FrameCamera,
        copies: SceneCopies,
        encoder: &mut wgpu::CommandEncoder,
        report: &mut FrameReport,
    ) {
        let FrameCamera {
            viewport,
            uniforms,
            eye,
            motion_frame,
            ..
        } = *cam;
        // The scene pass, ended at each copy point a layer asked for.
        if self.scene_groups.1.is_empty() || self.scene_groups.0 != copies {
            self.scene_groups = (
                copies,
                scene_pass_groups(copies)
                    .into_iter()
                    .map(|(phases, point)| {
                        let names: Vec<&str> = phases.iter().map(|phase| phase.name()).collect();
                        let label = format!("scene {}", names.join(", "));
                        (phases, point, label)
                    })
                    .collect(),
            );
        }
        let scene_groups = std::mem::take(&mut self.scene_groups.1);
        let night_readability = self.eye.sky_mean != SkyMean::None;
        let scene_passes = &mut report.scene_passes;
        scene_passes.truncate(scene_groups.len());
        for (index, (phases, point, label)) in scene_groups.iter().enumerate() {
            let point = *point;
            {
                let mut pass = begin_scene_pass(
                    encoder,
                    &self.resources,
                    label,
                    index > 0,
                    self.timer.render_pass(label),
                );
                let ctx = DrawContext {
                    pipelines: &self.pipelines,
                    resources: &self.resources,
                    camera: &uniforms,
                    time: input.time,
                    viewport,
                };
                for &phase in phases {
                    // The night's grade reads the transparents' cover from the scene's alpha (`transparent_cover.wgsl`).
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
                // What a copy holds for the layers that read it, held as the frame is.
                if point.is_some() && night_readability {
                    self.display.draw_hold(&mut pass, &self.pipelines);
                }
            }
            match scene_passes.get_mut(index) {
                Some(held) => held.clone_from(label),
                None => scene_passes.push(label.clone()),
            }
            if let Some(point) = point {
                copy_scene(encoder, &self.resources, copies & point.copies());
                for layer in layers.iter_mut() {
                    encoder.push_debug_group(layer.label());
                    layer.after_copies(
                        point,
                        &mut PrepareContext {
                            device: &self.device,
                            queue: &self.queue,
                            encoder,
                            pipelines: &self.pipelines,
                            resources: &mut self.resources,
                            camera: &uniforms,
                            eye,
                            camera_group: &self.camera.group,
                            time: input.time,
                            motion_frame,
                            viewport,
                            timer: &self.timer,
                            uploads: &mut self.uploads,
                            shadow_maps: &self.shadow_fit.maps,
                            light_maps: &self.light_maps,
                            aerial: &self.camera.aerial,
                            sun_occluders: &self.shadow_targets.occluders,
                        },
                    );
                    encoder.pop_debug_group();
                }
            }
            // Each group of the scene pass finishes while the next is recorded.
            self.parts
                .hand_over(&self.device, self.finisher.as_ref(), encoder);
        }
        self.scene_groups.1 = scene_groups;
    }

    /// Temporal AA's motion pass.
    fn draw_motion(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let FrameCamera {
            viewport,
            uniforms,
            taa_frame,
            ..
        } = *cam;
        // Temporal AA: moving surfaces' motion, where each is the nearest thing drawn.
        if taa_frame.is_some() {
            let motion = self.resources.texture(SCENE_MOTION).expect("made above");
            let depth = self
                .resources
                .texture(SCENE_DEPTH)
                .expect("registered in new");
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("motion"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &motion.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth.view,
                    depth_ops: None,
                    stencil_ops: None,
                }),
                timestamp_writes: self.timer.render_pass("motion"),
                ..Default::default()
            });
            let ctx = DrawContext {
                pipelines: &self.pipelines,
                resources: &self.resources,
                camera: &uniforms,
                time: input.time,
                viewport,
            };
            for layer in layers.iter() {
                pass.push_debug_group(layer.label());
                pass.set_bind_group(0, &self.camera.group, &[]);
                layer.draw_motion(&mut pass, &ctx);
                pass.pop_debug_group();
            }
            // Then what lies in front of every surface (the rain), over all their motion.
            for layer in layers.iter() {
                pass.set_bind_group(0, &self.camera.group, &[]);
                layer.draw_motion_over(&mut pass, &ctx);
            }
        }
    }

    /// Photo mode's shutter, or the film camera's in play.
    fn draw_shutter(
        &mut self,
        input: FrameInput<'_>,
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let FrameCamera {
            viewport,
            taa_frame,
            ..
        } = *cam;
        // Photo mode's shutter: the scene blurred in place by the frozen moment's velocities, before the eye, the bloom,
        // the depth of field and the finish read it.
        if let Some(shutter) = self.photo.as_ref().and_then(|photo| photo.shutter) {
            let hdr = self
                .resources
                .texture(SCENE_HDR)
                .expect("registered in new");
            let depth = self
                .resources
                .texture(SCENE_DEPTH)
                .expect("registered in new");
            self.shutter.encode(
                &self.device,
                &self.queue,
                encoder,
                &self.pipelines,
                &self.timer,
                (&hdr.texture, &hdr.view, hdr.version),
                (&depth.view, depth.version),
                viewport,
                input.camera,
                taa_frame.map_or(glam::Vec2::ZERO, |frame| frame.jitter),
                crate::shutter::Velocity::Bodies {
                    shutter: &shutter,
                    bodies: &self.photo_bodies,
                },
            );
        } else if let (Some(play), Some(frame)) = (self.play_camera, taa_frame)
            && self.photo.is_none()
            && frame.motion_holds
            && play.travel_share(self.motion_interval) > 0.0
        {
            // The film camera in play: the same shutter on temporal AA's motion since the last frame.
            let texture = |name| self.resources.texture(name).expect("registered");
            let (hdr, depth, motion) = (
                texture(SCENE_HDR),
                texture(SCENE_DEPTH),
                texture(SCENE_MOTION),
            );
            self.shutter.encode(
                &self.device,
                &self.queue,
                encoder,
                &self.pipelines,
                &self.timer,
                (&hdr.texture, &hdr.view, hdr.version),
                (&depth.view, depth.version),
                viewport,
                input.camera,
                frame.jitter,
                crate::shutter::Velocity::Motion {
                    motion: (&motion.view, motion.version),
                    reproject: frame.previous_view_projection * input.camera.view().inverse(),
                    share: play.travel_share(self.motion_interval),
                },
            );
        }
    }

    /// Post: the eye's exposure and the bloom.
    fn meter_eye_and_bloom(&mut self, input: FrameInput<'_>, encoder: &mut wgpu::CommandEncoder) {
        // Post: the eye's exposure for this frame, then the bloom on it. Its glare reads how far the view looks from the
        // sun, by the sun's height (`crate::post::sun_away`).
        self.eye.sun_away = self.sun_away(input.camera);
        self.eye.encode(
            &self.device,
            &mut self.uploads,
            encoder,
            &self.pipelines,
            &self.resources,
            &self.timer,
            input.time.frame,
            input.time.dt,
        );
        self.eye
            .copy_reading(&self.device, encoder, &self.resources);
        if self.bloom_on {
            // What the frame shows of the sun, for its glare in the eye.
            let sun = self.sun_glare.encode(
                &self.device,
                &mut self.uploads,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                input.camera,
                input.time.dt,
                // The night grade's cover pass ran: the scene's alpha holds the transparents' cover.
                self.eye.sky_mean != SkyMean::None,
            );
            self.bloom.encode(
                &self.device,
                &mut self.uploads,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                // Photo mode's compensation too, so the glow follows the exposure.
                self.display.settings.exposure * self.photo.map_or(1.0, |photo| photo.ev.exp2()),
                sun,
                self.eye.night_share,
            );
        } else {
            self.bloom.release();
        }
    }

    /// The display pass: the scene's radiance (photo mode's depth of field and lens over it) through the look,
    /// the bloom and AgX, onto the output or the chain's frame.
    fn draw_display(
        &mut self,
        input: FrameInput<'_>,
        output: RenderOutput<'_>,
        plan: &FramePlan,
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let FramePlan {
            display_direct,
            look,
            film_on,
            display,
            ..
        } = *plan;
        let viewport = cam.viewport;
        let hdr = self
            .resources
            .texture(SCENE_HDR)
            .expect("registered in new");
        // Photo mode's depth of field: the display reads its result in place of the scene's radiance.
        let source = match self.photo.as_ref().and_then(|photo| photo.dof) {
            Some(dof) => {
                let depth = self
                    .resources
                    .texture(SCENE_DEPTH)
                    .expect("registered in new");
                self.dof.encode(
                    &self.device,
                    &self.queue,
                    encoder,
                    &self.pipelines,
                    &self.timer,
                    ((&hdr.view, hdr.version), (&depth.view, depth.version)),
                    viewport,
                    (input.camera.near, input.camera.far),
                    &dof,
                    crate::photo_optics::aperture_shape(
                        self.photo.as_ref().and_then(|photo| photo.optics.as_ref()),
                        self.photo.as_ref().map_or(0.0, |photo| photo.roll),
                    ),
                )
            }
            None => (&hdr.view, hdr.version),
        };
        // Photo mode's lens: its flare and glare over that, from the scene's radiance.
        let source = match self
            .photo
            .as_ref()
            .and_then(|photo| photo.optics.map(|o| (o, photo)))
        {
            Some((optics, photo)) => {
                let depth = self
                    .resources
                    .texture(SCENE_DEPTH)
                    .expect("registered in new");
                let eye = &self
                    .resources
                    .buffer(EYE_STATE_BUFFER)
                    .expect("registered by the eye adaptation")
                    .buffer;
                self.optics.encode(
                    &self.device,
                    &self.queue,
                    encoder,
                    &self.pipelines,
                    &self.timer,
                    &crate::photo_optics::OpticsInputs {
                        scene: &hdr.view,
                        depth: &depth.view,
                        source: source.0,
                        eye,
                        viewport,
                        exposure: self.display.settings.exposure * photo.ev.exp2(),
                        frame_scale: photo.roll_scale,
                        roll: photo.roll,
                        camera: input.camera,
                        dof: photo.dof,
                    },
                    &optics,
                    true,
                )
            }
            // The film camera in play: a coated lens's veil of the sun, without its ghosts and spikes.
            None => match self
                .play_camera
                .filter(|play| play.veil && self.photo.is_none())
            {
                Some(_) => {
                    let depth = self
                        .resources
                        .texture(SCENE_DEPTH)
                        .expect("registered in new");
                    let eye = &self
                        .resources
                        .buffer(EYE_STATE_BUFFER)
                        .expect("registered by the eye adaptation")
                        .buffer;
                    self.optics.encode(
                        &self.device,
                        &self.queue,
                        encoder,
                        &self.pipelines,
                        &self.timer,
                        &crate::photo_optics::OpticsInputs {
                            scene: &hdr.view,
                            depth: &depth.view,
                            source: source.0,
                            eye,
                            viewport,
                            exposure: self.display.settings.exposure,
                            frame_scale: 1.0,
                            roll: 0.0,
                            camera: input.camera,
                            dof: None,
                        },
                        &crate::play_camera::veil_lens(input.camera, self.sun_glare.sun),
                        false,
                    )
                }
                None => source,
            },
        };
        let bloom = match self.bloom.output().filter(|_| self.bloom_on) {
            Some(view) => (view, self.bloom.version),
            None => (&self.black.1, 0),
        };
        self.shimmer_clock
            .advance(input.camera, &self.sea_air, input.time.dt);
        let inputs = DisplayInputs {
            source,
            depth: {
                let depth = self
                    .resources
                    .texture(SCENE_DEPTH)
                    .expect("registered in new");
                (&depth.view, depth.version)
            },
            clip_planes: [input.camera.near, input.camera.far],
            bloom,
            bloom_on: self.bloom_on,
            sampler: self.bloom.sampler(),
            eye: &self
                .resources
                .buffer(EYE_STATE_BUFFER)
                .expect("registered by the eye adaptation")
                .buffer,
            sky_mean: &self
                .resources
                .buffer(SKY_MEAN_BUFFER)
                .expect("registered by the eye adaptation")
                .buffer,
            night_readability: self.eye.sky_mean != SkyMean::None,
            night: crate::post::night_of(self.eye.sun_elevation),
            night_vision: crate::post::night_vision_of(self.eye.sun_elevation),
            underwater: (
                self.underwater.0.as_ref().unwrap_or(&self.black.1),
                self.underwater.1,
            ),
            headroom: if extended_range(output.format) {
                self.headroom
            } else {
                1.0
            },
            dither: &self.dither,
            photo: self.photo.as_ref(),
            look,
            preview: 0.0,
            fringes: self
                .play_camera
                .filter(|_| self.photo.is_none())
                .map_or(0.0, |play| play.fringe_shift(input.camera)),
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
        self.clip_planes = inputs.clip_planes;
        let film_frame = film_on.then(|| {
            &self
                .resources
                .texture(crate::film::FILM_FRAME)
                .expect("made above")
                .view
        });
        let display_target = if display_direct {
            film_frame.unwrap_or(output.view)
        } else {
            &self
                .resources
                .texture(DISPLAY_FRAME)
                .expect("made above")
                .view
        };
        self.display.encode(
            &self.device,
            &mut UploadSink::Belt(&mut self.uploads),
            encoder,
            self.pipelines.render(display),
            &inputs,
            display_target,
            self.timer.render_pass("display"),
        );
    }

    /// The chain after the display pass: temporal AA's resolve, SMAA, the last stage, the upscale and the look's
    /// film.
    fn draw_output_chain(
        &mut self,
        output: RenderOutput<'_>,
        plan: &FramePlan,
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let FramePlan {
            upscale,
            stage,
            film,
            film_on,
            stage_pipeline,
            upscale_pipeline,
            film_pipeline,
            ..
        } = *plan;
        let FrameCamera {
            uniforms,
            taa_frame,
            ..
        } = *cam;
        if let Some(frame) = taa_frame {
            self.taa.resolve(
                &self.device,
                &mut self.uploads,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                &frame,
                &uniforms,
            );
        }
        if self.antialiasing == Antialiasing::Smaa {
            self.chain.encode_smaa(
                &self.device,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                &self.dither,
                DISPLAY_FRAME,
            );
        }
        // The last stage onto the output, or into `post.frame` for the upscale.
        let mut last = DISPLAY_FRAME;
        if let (Some(stage), Some(pipeline)) = (stage, stage_pipeline) {
            let source = if stage == OutputStage::Taa {
                TAA_RESOLVED
            } else {
                DISPLAY_FRAME
            };
            let target = if upscale {
                StageTarget::Texture(POST_FRAME)
            } else if film_on {
                StageTarget::Texture(crate::film::FILM_FRAME)
            } else {
                StageTarget::Output(output.view)
            };
            self.chain.encode_stage(
                &self.device,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                &self.dither,
                stage,
                pipeline,
                source,
                target,
            );
            last = POST_FRAME;
        }
        if let Some(pipeline) = upscale_pipeline {
            self.chain.encode_stage(
                &self.device,
                encoder,
                &self.pipelines,
                &self.resources,
                &self.timer,
                &self.dither,
                OutputStage::Upscale,
                pipeline,
                last,
                if film_on {
                    StageTarget::Texture(crate::film::FILM_FRAME)
                } else {
                    StageTarget::Output(output.view)
                },
            );
        }
        if let Some(pipeline) = film_pipeline {
            self.film.encode(
                &self.device,
                &mut self.uploads,
                encoder,
                self.pipelines.render(pipeline),
                &self.resources,
                &self.timer,
                &self.dither,
                &film,
                output.view,
            );
        }
    }

    /// The developer panel's debug view.
    fn draw_debug_view(
        &mut self,
        output: RenderOutput<'_>,
        plan: &FramePlan,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let debug_pass = plan.debug_pass;
        // The developer panel's full-screen debug view, over the finished frame and under the overlays.
        if let Some(id) = debug_pass {
            let depth = self
                .resources
                .texture(SCENE_DEPTH)
                .expect("registered in new");
            let pipeline = self.overlays[id.0]
                .built
                .iter()
                .find(|(format, _)| *format == output.format)
                .map(|(_, built)| *built);
            if let (Some(group), Some(pipeline)) = (self.debug.group(&self.device, depth), pipeline)
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("debug view"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    timestamp_writes: self.timer.render_pass("debug view"),
                    ..Default::default()
                });
                pass.set_pipeline(self.pipelines.render(pipeline));
                pass.set_bind_group(0, &self.camera.group, &[]);
                pass.set_bind_group(1, group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }

    /// The layers' overlays, last, at the output's full resolution.
    fn draw_overlays(
        &mut self,
        input: FrameInput<'_>,
        layers: &[&mut dyn SceneLayer],
        output: RenderOutput<'_>,
        plan: &FramePlan,
        cam: &FrameCamera,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let overlay = plan.overlay;
        let FrameCamera {
            viewport, uniforms, ..
        } = *cam;
        if overlay {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                timestamp_writes: self.timer.render_pass("overlay"),
                ..Default::default()
            });
            let ctx = OverlayContext {
                pipelines: &self.pipelines,
                resources: &self.resources,
                camera: &uniforms,
                time: input.time,
                viewport: (output.width.max(1), output.height.max(1)),
                scene_viewport: viewport,
                format: output.format,
                overlays: &self.overlays,
            };
            for layer in layers.iter().filter(|layer| layer.has_overlay()) {
                pass.push_debug_group(layer.label());
                pass.set_bind_group(0, &self.camera.group, &[]);
                layer.draw_overlay(&mut pass, &ctx);
                pass.pop_debug_group();
            }
        }
    }

    /// The frame's submission: the leading command buffers, its uploads, its finished parts and its last part, in
    /// that order; then the belt recalled, the eye's reading mapped and the timer and the resources' frame ended.
    fn submit_frame(
        &mut self,
        encoder: wgpu::CommandEncoder,
        mut leading: Vec<wgpu::CommandBuffer>,
    ) {
        // The frame's uploads first, so they are in place before its passes (as `write_buffer`'s are).
        self.upload_stats = self.uploads.take_stats();
        let uploads = self.uploads.finish();
        // The last part finishes here, while the others may still be finishing, then all go in their order.
        let last = encoder.finish();
        self.queue.submit(
            leading
                .drain(..)
                .chain(uploads)
                .chain(self.parts.collect())
                .chain([last]),
        );
        // Its room for the next frame's.
        self.leading = leading;
        self.uploads.recall();
        self.eye.map_reading();
        self.timer.end_frame(&self.queue);
        self.submitted += 1;
        self.resources.end_frame();
    }
}
