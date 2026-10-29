//! The Settings menu's Look tab previews ([`Renderer::look_previews`]): each look on the frame last drawn.

use crate::display::DisplayInputs;
use crate::post::{EYE_STATE_BUFFER, SKY_MEAN_BUFFER, SkyMean};
use crate::upload::UploadSink;

use super::{Renderer, SCENE_DEPTH, SCENE_HDR};

impl Renderer {
    /// The Look tab's previews (`crate::look_preview`): each of `looks` on the frame last drawn (its radiance, bloom
    /// and eye), about `width` pixels across at its aspect, through the display pass's grade and the film pass's
    /// film. Each look is a submit of its own, as both passes write their uniforms per draw; the tiles land one after
    /// another in a buffer the host maps. `None` before the first frame, or with a pipeline that failed to build.
    pub fn look_previews(
        &mut self,
        looks: &[crate::photo::Look],
        width: u32,
    ) -> Option<crate::look_preview::LookPreviews> {
        use crate::look_preview::{PREVIEW_FORMAT, preview_film, tile_size};
        if looks.is_empty() || self.submitted == 0 {
            return None;
        }
        let display_id = self
            .display
            .ensure_format(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                crate::taa::FRAME_FORMAT,
            )
            .ok()?;
        let film_id = self
            .film
            .ensure_format(
                &self.device,
                &mut self.library,
                &mut self.pipelines,
                PREVIEW_FORMAT,
            )
            .ok()?;
        self.pipelines.finish().ok()?;
        let hdr = self.resources.texture(SCENE_HDR)?;
        let frame = (hdr.texture.width(), hdr.texture.height());
        let (width, height) = tile_size(width, frame);
        let target = |label, format, usage| {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
                view_formats: &[],
            })
        };
        let graded = target(
            "look preview graded",
            crate::taa::FRAME_FORMAT,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let graded_view = graded.create_view(&wgpu::TextureViewDescriptor::default());
        let tile = target(
            "look preview tile",
            PREVIEW_FORMAT,
            wgpu::TextureUsages::COPY_SRC,
        );
        let tile_view = tile.create_view(&wgpu::TextureViewDescriptor::default());
        let row_bytes = width * 4;
        let tile_bytes = u64::from(row_bytes) * u64::from(height);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("look previews"),
            size: tile_bytes * looks.len() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let depth = self
            .resources
            .texture(SCENE_DEPTH)
            .expect("registered in new");
        let bloom = match self.bloom.output().filter(|_| self.bloom_on) {
            Some(view) => (view, self.bloom.version),
            None => (&self.black.1, 0),
        };
        let eye = &self
            .resources
            .buffer(EYE_STATE_BUFFER)
            .expect("registered by the eye adaptation")
            .buffer;
        let sky_mean = &self
            .resources
            .buffer(SKY_MEAN_BUFFER)
            .expect("registered by the eye adaptation")
            .buffer;
        for (index, look) in looks.iter().enumerate() {
            let inputs = DisplayInputs {
                source: (&hdr.view, hdr.version),
                depth: (&depth.view, depth.version),
                clip_planes: self.clip_planes,
                bloom,
                bloom_on: self.bloom_on,
                sampler: self.bloom.sampler(),
                eye,
                sky_mean,
                night_readability: self.eye.sky_mean != SkyMean::None,
                night: crate::post::night_of(self.eye.sun_elevation),
                night_vision: crate::post::night_vision_of(self.eye.sun_elevation),
                underwater: (
                    self.underwater.0.as_ref().unwrap_or(&self.black.1),
                    self.underwater.1,
                ),
                headroom: 1.0,
                dither: &self.dither,
                photo: None,
                look: *look,
                preview: frame.0 as f32 / width as f32,
                fringes: 0.0,
                shimmer: None,
            };
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("look preview"),
                });
            self.display.encode(
                &self.device,
                &mut UploadSink::Queue(&self.queue),
                &mut encoder,
                self.pipelines.render(display_id),
                &inputs,
                &graded_view,
                None,
            );
            self.film.encode_preview(
                &self.device,
                &self.queue,
                &mut encoder,
                self.pipelines.render(film_id),
                &self.dither,
                &preview_film(*look, height),
                &graded_view,
                &tile_view,
            );
            encoder.copy_texture_to_buffer(
                tile.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: tile_bytes * index as u64,
                        bytes_per_row: Some(row_bytes),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            self.queue.submit([encoder.finish()]);
        }
        Some(crate::look_preview::LookPreviews {
            buffer,
            looks: looks.to_vec(),
            width,
            height,
            row_bytes,
        })
    }
}
