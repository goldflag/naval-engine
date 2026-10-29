// How the wind moves the trees, and which of a tree's two pictures draws its pixels near the camera: shared by the
// impostors (`trees.wgsl`), their shadow casters (`tree_caster.wgsl`) and the near trees' meshes (`near_trees.wgsl`), so
// a tree sways the same in each and the crossfade between mesh and impostor shows no seam.
#define_import_path naval_terrain::tree_wind

#import naval::camera::{camera}
#import naval_terrain::bindings::{terrain}

const TAU: f32 = 6.283185307179586;

// An integer hash of a chart point's metre, 0-1.
fn foot_hash(foot: vec2<f32>) -> f32 {
    var v = bitcast<vec2<u32>>(vec2<i32>(floor(foot))) * 1664525u + vec2<u32>(1013904223u, 374761393u);
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32((v.x + v.y) >> 8u) / 16777216.0;
}

// How the wind sways a tree standing at chart point `foot`, `height` metres tall: its top's horizontal offset (world
// metres) now. The drag grows with the square of the wind, so a 20 m tree's top leans 0.2 m in a 5 m/s breeze and
// 1.2 m in a 12 m/s wind; gusts run downwind across the land at the wind's speed; each tree sways at its own phase and
// at a frequency of its height (a tall tree slower, a shrub quicker). A calm stands still.
fn tree_sway(foot: vec2<f32>, height: f32) -> vec2<f32> {
    let speed = terrain.wind.z;
    if speed < 0.4 {
        return vec2<f32>(0.0);
    }
    let dir = terrain.wind.xy;
    let t = terrain.wind.w;
    let lean = min(0.0004 * speed * speed, 0.06) * height * smoothstep(0.4, 2.5, speed);
    let along = dot(foot, dir) - speed * t;
    let gust = 0.5 + 0.3 * sin(along * 0.105) + 0.2 * sin(along * 0.273 + 1.7);
    let phase = foot_hash(foot) * TAU;
    let frequency = 0.32 * sqrt(20.0 / max(height, 1.5));
    let sway = sin(t * frequency * TAU + phase);
    let across = vec2<f32>(-dir.y, dir.x);
    return dir * lean * (0.4 + 0.6 * gust + 0.3 * sway) + across * lean * 0.18 * sin(t * frequency * 4.7 + phase * 1.7);
}

// The world offset of a tree's point at height share `h` (0 at its foot, 1 at its top) for its top's sway `top`: a
// bend growing with the square of the height, dropping a little so the trunk keeps its length.
fn bend(top: vec2<f32>, h: f32, height: f32) -> vec3<f32> {
    let k = h * h;
    let drop = 0.5 * dot(top, top) / max(height, 1.0) * h;
    return vec3<f32>(top.x * k, -drop, top.y * k);
}

// The world offset of a tree's point at height share `h` for its own lean `lean` (its top's offset over its height,
// chart x and z: `impostor::form_of`): a trunk leaning from its foot and bowing a little toward its top by `bow` (1 a
// tree's; 0 a palm's, straight from its foot as its card leans, so its mesh and card agree), dropping as the wind's
// bend does so it keeps its length.
fn lean_by(lean: vec2<f32>, bow: f32, h: f32, height: f32) -> vec3<f32> {
    let top = lean * height;
    let k = h * (1.0 - 0.55 * bow + 0.55 * bow * h);
    let drop = 0.5 * dot(top, top) / max(height, 1.0) * h;
    return vec3<f32>(top.x * k, -drop, top.y * k);
}

// The meshes' reach for a tree `height` metres tall (m; 0 with no meshes): a share of the largest reach, by the
// height, so a small tree's mesh gives way to its impostor where the impostor's texels are as fine on screen as a
// large one's.
fn near_reach(height: f32) -> f32 {
    return min(terrain.tree_near.w, height * terrain.tree_lod.x);
}

// The shares of a tree's pixels its mesh draws (x) and, of those, its full detail (y), by its middle's (world)
// distance from the eye; the impostor draws the rest. The placement's binning (`tree_near_bin.wgsl`) puts the tree in
// each level's list where its share can be above 0.
fn near_shares(middle: vec3<f32>, height: f32) -> vec2<f32> {
    let reach = near_reach(height);
    if reach <= 0.0 {
        return vec2<f32>(0.0);
    }
    let d = distance(terrain.tree_near.xyz, middle);
    let band = reach * terrain.tree_lod.y;
    let full_end = reach * terrain.tree_lod.z;
    let full_band = full_end * terrain.tree_lod.w;
    return vec2<f32>(1.0 - smoothstep(reach - band, reach, d), 1.0 - smoothstep(full_end - full_band, full_end, d));
}

// A pixel's place in a crossfade, 0-1: interleaved gradient noise (Jimenez 2014), moved each frame so temporal
// anti-aliasing blends a dithered crossfade smooth.
fn pixel_dither(pixel: vec2<f32>) -> f32 {
    let f = f32(camera.frame % 64u) * 5.588238;
    let p = floor(pixel) + vec2<f32>(f, f);
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}
