// A ship's navigation lamps (no source port): each a point seen from afar, drawn as a small glow facing the eye whose
// light at the eye is the lamp's (its candela on the scene's scale over the distance squared), spread over at least a
// pixel or two so a far lamp still shows, dimmed by the air between, and never brighter than the game's lamps on the
// land shine (`GLOW_LUMINANCE`). Added to the transparent pass; the arcs they show over are the CPU's (`Lamp::arc`).

#import naval::camera::{camera, eye_to_clip}
#import naval::aerial::aerial_apply_additive
#import naval_vessel::common::{frame, lamps}

struct LampOut {
    @builtin(position) clip: vec4<f32>,
    // Where in the glow, in its spread's units.
    @location(0) at: vec2<f32>,
    @location(1) @interpolate(flat) radiance: vec3<f32>,
    @location(2) @interpolate(flat) world: vec3<f32>,
}

// The glow's half width in its spread's units.
const REACH: f32 = 3.0;
// The most a glow shines at (its luminance, the scene's units): an obstruction light's core on the land (naval-terrain's
// `lamps.wgsl`, `BEACON_LUM`), the nearest kin the game draws to a ship's lamp. Close to, her 94 cd masthead lamp would
// shine at several times it, which through the twilight (the night's lift on real light in, the bloom's night limit not
// yet: `naval_render::post::bloom_limit`) blooms into a glare wider than her mast; held to it she shows a bright point
// on her mast as the land's lamps do. Beyond a kilometre or so, its light spread over a pixel or two, a lamp keeps it all.
const GLOW_LUMINANCE: f32 = 30.0;

@vertex
fn vs_lamp(@builtin(vertex_index) vertex: u32, @builtin(instance_index) index: u32) -> LampOut {
    let lamp = lamps[index];
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[vertex];
    let distance = max(length(lamp.offset.xyz), 0.1);
    // Its spread (radians): the lamp's own size, or under a pixel and a half at any distance.
    let pixel = 2.0 * camera.tan_half_fov_y / camera.viewport.y;
    let spread = max(lamp.offset.w / distance, 0.8 * pixel);
    let right = vec3<f32>(camera.view[0].x, camera.view[1].x, camera.view[2].x);
    let up = vec3<f32>(camera.view[0].y, camera.view[1].y, camera.view[2].y);
    // Drawn a little toward the eye, clear of the mast it stands on.
    let toward = -lamp.offset.xyz / distance;
    let centre = lamp.offset.xyz + toward * min(lamp.offset.w * 2.0, distance * 0.5);
    let half = REACH * spread * distance;
    var out: LampOut;
    out.clip = eye_to_clip(centre + (right * corner.x + up * corner.y) * half);
    out.at = corner * REACH;
    // Its light at the eye spread as a gaussian over the solid angle 2π spread².
    let illuminance = lamp.light.rgb / (distance * distance);
    let radiance = illuminance / (2.0 * 3.141592653589793 * spread * spread);
    let luminance = dot(radiance, vec3<f32>(0.2126, 0.7152, 0.0722));
    out.radiance = radiance * min(1.0, GLOW_LUMINANCE / max(luminance, 1e-12));
    out.world = lamp.offset.xyz + frame.eye.xyz;
    return out;
}

@fragment
fn fs_lamp(in: LampOut) -> @location(0) vec4<f32> {
    let glow = in.radiance * exp(-0.5 * dot(in.at, in.at));
    var color = glow;
    if frame.fog.params.w > 0.5 {
        color = aerial_apply_additive(glow, in.world);
    }
    return vec4<f32>(color, 0.0);
}
