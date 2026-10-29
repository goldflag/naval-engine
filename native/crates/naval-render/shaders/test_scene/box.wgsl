// The test scene's lit box: a hull-sized block on the sea, lit by the sun and
// the sky's ambient, shadowed by the sun's shadow maps. Opaque, depth writing, a
// shadow caster (`caster_vertex`) and a moving surface for temporal AA's motion pass
// (`motion_vertex`). No TypeScript source; the ship lane replaces it.
#import naval::camera::{camera, world_to_clip}
#import naval::motion::surface_motion
#import naval::shadow::sun_shadow
#import naval::test_scene::{SUN_DIRECTION, SUN_RADIANCE, ambient, apply_haze}

struct Model {
    model: mat4x4<f32>,
    // The inverse transpose of `model`, for normals.
    normal: mat4x4<f32>,
    albedo: vec4<f32>,
    // The model matrix a frame earlier.
    previous: mat4x4<f32>,
}

@group(1) @binding(0) var<uniform> box_model: Model;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let world = (box_model.model * vec4<f32>(in.position, 1.0)).xyz;
    var out: VertexOut;
    out.clip = world_to_clip(world);
    out.world = world;
    out.normal = (box_model.normal * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

// The box in a shadow map: there `camera` holds the map's shadow camera.
@vertex
fn caster_vertex(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return world_to_clip((box_model.model * vec4<f32>(position, 1.0)).xyz);
}

struct MotionOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) previous: vec3<f32>,
}

// The box in the motion pass: where each point is now and was a frame ago.
@vertex
fn motion_vertex(@location(0) position: vec3<f32>) -> MotionOut {
    let world = (box_model.model * vec4<f32>(position, 1.0)).xyz;
    var out: MotionOut;
    out.clip = world_to_clip(world);
    out.world = world;
    out.previous = (box_model.previous * vec4<f32>(position, 1.0)).xyz;
    return out;
}

@fragment
fn motion_fragment(in: MotionOut) -> @location(0) vec4<f32> {
    return surface_motion(in.world, in.previous, 0.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let normal = normalize(in.normal);
    let sun = SUN_RADIANCE * max(dot(normal, SUN_DIRECTION), 0.0) * sun_shadow(in.world, normal);
    let color = box_model.albedo.rgb * (sun + ambient(normal));
    return vec4<f32>(apply_haze(color, in.world), 1.0);
}
