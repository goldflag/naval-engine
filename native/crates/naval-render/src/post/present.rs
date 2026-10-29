//! A finished frame onto a linear extended-range surface: the last step of HDR output where the surface takes linear
//! light (scRGB, `SurfaceColorSpace::ExtendedSrgbLinear`: Windows' HDR swap chains, 1 at 80 nits) rather than
//! extended sRGB (`ExtendedSrgb`, which Metal takes and needs none of this).
//!
//! The renderer writes its frames display-encoded, the sRGB curve carried on above SDR white for an HDR output, and
//! every layer composited over a frame after the post chain (the aim marks, the ship labels, the page) blends in that
//! space, as it does over an SDR frame. So the host draws the frame and everything over it into an encoded
//! `Rgba16Float` texture ([`FRAME_FORMAT`]) and presents it with [`LinearPresent`], which decodes it and scales SDR
//! white to the display's SDR white level (`shaders/post/present.wgsl`). One full-screen pass, reading and writing
//! half floats.
//!
//! It is the host's pass, not a scene's: it builds its own pipeline once (no hot reload) and outlives the renderers a
//! session makes per scene.

/// The format of the encoded frame [`LinearPresent`] reads, and of a linear HDR surface.
pub const FRAME_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

const SHADER: &str = include_str!("../../shaders/post/present.wgsl");

/// The present pass onto a linear extended-range surface of [`FRAME_FORMAT`].
pub struct LinearPresent {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
}

impl LinearPresent {
    pub fn new(device: &wgpu::Device) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("linear present"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("linear present"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(16),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("linear present"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("linear present"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FRAME_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("linear present"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            uniforms,
        }
    }

    /// Decodes `frame` (display-encoded, [`FRAME_FORMAT`], the target's size) onto `target`, SDR white at `white` in
    /// the target's units (1 on Apple's EDR; the SDR white level over 80 nits on Windows).
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::TextureView,
        target: &wgpu::TextureView,
        white: f32,
    ) {
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::cast_slice(&[white.max(0.0), 0.0, 0.0, 0.0]),
        );
        // A frame's view can change with every resize: the group is a few bytes, made each frame.
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("linear present"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(frame),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("linear present"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
}
