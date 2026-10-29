// Light and haze shared by the test scene's sky, sea grid and box. No
// TypeScript source.
#define_import_path naval::test_scene

#import naval::camera::camera

// Toward the sun: 20 degrees up, azimuth 100 (clockwise from -Z), so it lights
// the box's starboard side and leaves its bow in shade from the default view.
const SUN_DIRECTION: vec3<f32> = vec3<f32>(0.925416, 0.342020, 0.163176);
const SUN_RADIANCE: vec3<f32> = vec3<f32>(2.6, 2.4, 2.1);

const ZENITH: vec3<f32> = vec3<f32>(0.035, 0.09, 0.26);
const HORIZON: vec3<f32> = vec3<f32>(0.38, 0.46, 0.58);
const SEA_DEEP: vec3<f32> = vec3<f32>(0.006, 0.022, 0.040);

// Linear HDR sky radiance seen along `ray`: a gradient from the horizon haze to
// the zenith, a glow and a small disk about the sun, and the haze again below.
fn sky_radiance(ray: vec3<f32>) -> vec3<f32> {
    let up = ray.y;
    var sky = mix(HORIZON, ZENITH, pow(clamp(up, 0.0, 1.0), 0.45));
    // Below the horizon the view meets the far haze over the sea.
    sky = mix(sky, HORIZON * 0.55, smoothstep(0.0, -0.08, up));
    let mu = max(dot(ray, SUN_DIRECTION), 0.0);
    sky += SUN_RADIANCE * (0.08 * pow(mu, 12.0) + 0.25 * pow(mu, 300.0));
    // The disk, half a degree across: HDR well past white, for AgX to roll off.
    sky += SUN_RADIANCE * 40.0 * smoothstep(0.999962, 0.999976, mu);
    return sky;
}

// Sky light on a surface facing `normal`: the zenith above, the sea below.
fn ambient(normal: vec3<f32>) -> vec3<f32> {
    return mix(SEA_DEEP * 4.0, (ZENITH + HORIZON) * 0.6, normal.y * 0.5 + 0.5);
}

// Distance haze toward the horizon color along the view, so geometry meets the
// sky gradient at the horizon.
fn apply_haze(color: vec3<f32>, world: vec3<f32>) -> vec3<f32> {
    let offset = world - camera.position;
    let dist = length(offset);
    let haze = 1.0 - exp(-dist / 9000.0);
    return mix(color, sky_radiance(offset / max(dist, 1e-3)), haze);
}
