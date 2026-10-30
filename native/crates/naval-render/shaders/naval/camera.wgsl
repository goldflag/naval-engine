// The camera uniform block every scene pipeline binds at group 0, binding 0.
// Its Rust twin is `CameraUniforms` in src/camera.rs; keep the two in order.
// Depth is reversed as in the web game's `Game.ts` (`reversedDepthBuffer: true`): 1 at
// the near plane, 0 at the far plane, cleared to 0 and tested GreaterEqual.
#define_import_path naval::camera

struct Camera {
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view: mat4x4<f32>,
    inverse_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    // World meters.
    position: vec3<f32>,
    // Seconds since the scene started, from the manual clock.
    time: f32,
    // Width, height, 1/width, 1/height in pixels.
    viewport: vec4<f32>,
    near: f32,
    far: f32,
    tan_half_fov_y: f32,
    aspect: f32,
    dt: f32,
    frame: u32,
    // How the world curves away from the camera (`curved`): 1 / (2 R) for a view of the sea, 0 for a chart and for a
    // camera built from matrices (the sun's shadow cameras).
    curvature: f32,
    padding: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;

// A world point as the camera draws it (`naval_render::camera::curved`): lowered by the drop of the Earth's surface
// below the eye's tangent plane, the curvature times the square of its horizontal distance from the eye. The
// simulation's world is a plane and shading keeps its points (lights, shadow maps, the sea's fields); only where a
// point lands on the screen curves, so every pipeline projects through `world_to_clip`.
fn curved(world: vec3<f32>) -> vec3<f32> {
    let across = world.xz - camera.position.xz;
    return vec3<f32>(world.x, world.y - camera.curvature * dot(across, across), world.z);
}

fn world_to_clip(world: vec3<f32>) -> vec4<f32> {
    return camera.view_projection * vec4<f32>(curved(world), 1.0);
}

// A point given less the eye's position, as `world_to_clip` draws the world point (`naval_render::camera::eye_to_clip`):
// curved about the eye, turned by the view alone and projected. A layer that keeps its positions about the eye (the
// aircraft, `PrepareContext::eye`) draws them through this: past 16 km from the world's origin an f32 world point steps
// by 2 mm and `view_projection * world` cancels kilometres against kilometres, which shook a plane seen from a few
// metres by pixels.
fn eye_to_clip(offset: vec3<f32>) -> vec4<f32> {
    let drawn = vec3<f32>(offset.x, offset.y - camera.curvature * dot(offset.xz, offset.xz), offset.z);
    let turn = mat3x3<f32>(camera.view[0].xyz, camera.view[1].xyz, camera.view[2].xyz);
    return camera.projection * vec4<f32>(turn * drawn, 1.0);
}

// NDC (x right, y up, both -1..1) of a framebuffer position (pixels, y down),
// such as a fragment's @builtin(position).xy.
fn ndc_from_pixel(pixel: vec2<f32>) -> vec2<f32> {
    let uv = pixel * camera.viewport.zw;
    return vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
}

// The world direction of the view ray through an NDC position: the near plane's point in the camera's own frame,
// turned onto the world's (no translation). Through the inverse view-projection, that point lay half a metre from an
// eye kilometres from the origin, and their difference kept the f32 rounding of the eye's position: 2–9 pixels at
// 8–30 km, changing each frame the camera moved, so the splash columns and smoke marched along these rays shook.
fn view_ray(ndc: vec2<f32>) -> vec3<f32> {
    let near_point = camera.inverse_projection * vec4<f32>(ndc, 1.0, 1.0);
    return normalize((camera.inverse_view * vec4<f32>(near_point.xyz / near_point.w, 0.0)).xyz);
}

// Distance along the view axis, in meters, of a reversed depth value: `near` at
// depth 1, `far` at depth 0 (the far plane and the clear). three's
// `perspectiveDepthToViewZ` for a reversed buffer, negated.
fn view_depth(depth: f32) -> f32 {
    return camera.far * camera.near / (depth * (camera.far - camera.near) + camera.near);
}
