// The passes' screen helpers over the clouds' uniforms (`march_pass.wgsl`, `composite.wgsl`, `resolve.wgsl`,
// `shadow_pass.wgsl`).
#define_import_path naval_clouds::screen

#import naval_clouds::common::clouds

// Metres along the view of a reversed depth (`naval::camera::view_depth`).
fn view_depth(depth: f32) -> f32 {
    let near = clouds.planes.x;
    let far = clouds.planes.y;
    return far * near / (depth * (far - near) + near);
}

// A clip position's texture coordinates (y down).
fn clip_to_uv(clip: vec4<f32>) -> vec2<f32> {
    let ndc = clip.xy / clip.w;
    return vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
}
