// Proxy hulls for the water-effects stills: the SimpleHull of naval_ocean::hull_profile swept into a mesh, painted grey
// with a lighter deck and lit by the sun and a sky ambient. Not a port of the game's ship materials (the naval-ships lane
// owns those); it stands in for the hull the wakes, contact foam and spray are fitted to, and gives the ocean an opaque
// scene to see through at the waterline.

#import naval::camera::{camera, world_to_clip}

struct HullLight {
    // xyz toward the sun, w unused.
    sun_direction: vec4<f32>,
    // Sun radiance (colour × intensity).
    sun: vec4<f32>,
    // Sky irradiance on an upward face, and on a vertical one (rgb).
    sky_up: vec4<f32>,
    sky_side: vec4<f32>,
}

@group(1) @binding(0) var<uniform> light: HullLight;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    // The hull's model-to-world matrix, one column per attribute.
    @location(3) m0: vec4<f32>,
    @location(4) m1: vec4<f32>,
    @location(5) m2: vec4<f32>,
    @location(6) m3: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) albedo: vec3<f32>,
}

@vertex
fn vertex(input: VertexIn) -> VertexOut {
    let model = mat4x4<f32>(input.m0, input.m1, input.m2, input.m3);
    let world = model * vec4<f32>(input.position, 1.0);
    var output: VertexOut;
    output.clip = world_to_clip(world.xyz);
    output.normal = normalize((model * vec4<f32>(input.normal, 0.0)).xyz);
    output.albedo = input.albedo;
    return output;
}

@fragment
fn fragment(input: VertexOut) -> @location(0) vec4<f32> {
    let n = normalize(input.normal);
    let direct = light.sun.rgb * max(dot(n, light.sun_direction.xyz), 0.0) / 3.141592653589793;
    let ambient = mix(light.sky_side.rgb, light.sky_up.rgb, n.y * 0.5 + 0.5) / 3.141592653589793;
    return vec4<f32>(input.albedo * (direct + ambient), 1.0);
}
