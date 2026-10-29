// The test scene's sky: a gradient at the far plane (reversed depth 0), drawn in
// the Sky phase where opaque geometry left the depth clear. No TypeScript
// source; the sky lane replaces it.
#import naval::camera::{ndc_from_pixel, view_ray}
#import naval::fullscreen::{FullscreenVertex, fullscreen_vertex}
#import naval::test_scene::sky_radiance

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> FullscreenVertex {
    return fullscreen_vertex(index, 0.0);
}

@fragment
fn fragment(in: FullscreenVertex) -> @location(0) vec4<f32> {
    return vec4<f32>(sky_radiance(view_ray(ndc_from_pixel(in.position.xy))), 1.0);
}
