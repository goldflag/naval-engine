use native_offscreen::{Capture, Gpu, build_dir};

/// Clearing a capture to a known linear color reads back that color, sRGB
/// encoded, and writes a PNG under `.build/native/offscreen/`.
#[test]
fn a_cleared_capture_reads_back_its_color() {
    let gpu = Gpu::headless().expect("a GPU adapter");
    let capture = Capture::new(&gpu, 64, 48);
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("clear"),
        });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &capture.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.5,
                        b: 1.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
    gpu.queue.submit([encoder.finish()]);

    let pixels = capture.read_rgba(&gpu).expect("read-back");
    assert_eq!(pixels.len(), 64 * 48 * 4);
    // Linear 0.5 is sRGB 188.
    let (r, g, b, a) = (pixels[0], pixels[1], pixels[2], pixels[3]);
    assert_eq!((r, b, a), (0, 255, 255));
    assert!((186..=190).contains(&g), "green {g}");

    capture
        .save_png(&gpu, &build_dir("offscreen").join("smoke.png"))
        .expect("png");
}
