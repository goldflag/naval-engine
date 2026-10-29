// The test scene's sea plane: a flat dark sea at y = 0 ruled every 10, 100 and
// 1000 m, with the X axis (red) and Z axis (blue) through the origin, fading
// into the horizon haze, shadowed by the sun's shadow maps. Opaque, depth writing.
// No TypeScript source; the ocean lane replaces it.
#import naval::camera::{camera, world_to_clip}
#import naval::shadow::sun_shadow
#import naval::test_scene::{SEA_DEEP, SUN_DIRECTION, SUN_RADIANCE, ambient, apply_haze}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
}

// A square reaching nearly to the far plane, centred under the camera.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, -1.0),
    );
    let corner = corners[index] * camera.far * 0.7;
    let world = vec3<f32>(camera.position.x + corner.x, 0.0, camera.position.z + corner.y);
    var out: VertexOut;
    out.clip = world_to_clip(world);
    out.world = world;
    return out;
}

// Coverage of lines every `spacing` meters about `width` pixels wide, fading out
// where they would crowd closer than a few pixels.
fn grid_lines(position: vec2<f32>, spacing: f32, width: f32) -> f32 {
    let coord = position / spacing;
    let footprint = fwidth(coord);
    let dist = abs(fract(coord - 0.5) - 0.5) / max(footprint, vec2<f32>(1e-6));
    let line = 1.0 - clamp(min(dist.x, dist.y) - (width - 1.0) * 0.5, 0.0, 1.0);
    let crowding = max(footprint.x, footprint.y);
    return line * (1.0 - smoothstep(0.08, 0.3, crowding));
}

fn axis_line(coordinate: f32, width: f32) -> f32 {
    let footprint = max(fwidth(coordinate), 1e-6);
    return 1.0 - clamp(abs(coordinate) / footprint - (width - 1.0) * 0.5, 0.0, 1.0);
}

@fragment
fn fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let p = in.world.xz;
    let minor = grid_lines(p, 10.0, 1.0);
    let major = grid_lines(p, 100.0, 1.5);
    let kilometer = grid_lines(p, 1000.0, 2.0);
    let x_axis = axis_line(p.y, 2.5);
    let z_axis = axis_line(p.x, 2.5);

    let normal = vec3<f32>(0.0, 1.0, 0.0);
    let sun = SUN_RADIANCE * max(dot(normal, SUN_DIRECTION), 0.0) * sun_shadow(in.world, normal);
    var color = SEA_DEEP * (ambient(normal) + sun);
    // The rules catch a little sun, so shadows read on them too.
    let line_light = ambient(normal) * 0.5 + sun * 0.12;
    color = mix(color, line_light * 0.35, minor * 0.5);
    color = mix(color, line_light * 0.6, major * 0.8);
    color = mix(color, line_light, kilometer);
    color = mix(color, vec3<f32>(0.5, 0.06, 0.04), x_axis);
    color = mix(color, vec3<f32>(0.05, 0.12, 0.6), z_axis);
    return vec4<f32>(apply_haze(color, in.world), 1.0);
}
