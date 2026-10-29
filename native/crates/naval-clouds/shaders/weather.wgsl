// The weather composite (`naval_clouds::laying`): with more than one cloud anchor drawn, the anchors' weather laid about
// the camera, each texel what `weather_texel_direct` reads at its centre (every anchor's tile through its second-order
// map, blended), so every pass reads the weather with one fetch (`weather_texel`). One dispatch a composite and a
// level, over this frame's slices of rows: the near composite's level k holds the tile's level k, the far one's the
// tile's level k + 1. `clouds` is the uniforms of the frame the laying began (the layer keeps them); `live` this
// frame's, which name the slices.

#import naval_clouds::common::{clouds, CloudUniforms}
#import naval_clouds::field::weather_texel_direct

struct Bake {
    // 0 the near composite, 1 the far one.
    which: u32,
    // The composite's level.
    level: u32,
    padding: vec2<u32>,
}

@group(3) @binding(0) var composite_out: texture_storage_2d<rgba8unorm, write>;
@group(3) @binding(1) var<uniform> bake: Bake;
@group(3) @binding(2) var<uniform> live: CloudUniforms;

const NEAR_SIZE: u32 = 2048u;
const FAR_SIZE: u32 = 1536u;
const NEAR_TEXEL: f32 = 195.3125;

@compute @workgroup_size(8, 8)
fn weather_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let far = bake.which == 1u;
    let size = select(NEAR_SIZE, FAR_SIZE, far) >> bake.level;
    let rows = size / 8u;
    // This frame's slices of rows (`naval_clouds::laying::Laying::take_composite_slices`).
    let first = u32(live.composite_slices.x);
    let row = first * rows + id.y;
    if id.x >= size || id.y >= u32(live.composite_slices.y) * rows {
        return;
    }
    let texel = select(NEAR_TEXEL, 2.0 * NEAR_TEXEL, far) * f32(1u << bake.level);
    // Where the laying lays the composites, about the camera of the frame it began.
    let corners = clouds.composite;
    let corner = select(corners.xy, corners.zw, far);
    let d = corner + (vec2<f32>(f32(id.x), f32(row)) + 0.5) * texel;
    if far && bake.level == 0u {
        // Well inside the near composite the far one's finest level is never read: the near one serves every level it
        // holds there, and a coarser read takes the far one's levels 1 and 2 (`weather_texel`).
        let near = (d - corners.xy) / (f32(NEAR_SIZE) * NEAR_TEXEL);
        let margin = 32.0 / f32(NEAR_SIZE);
        if all(near > vec2<f32>(margin)) && all(near < vec2<f32>(1.0 - margin)) {
            return;
        }
    }
    let mip = f32(bake.level) + select(0.0, 1.0, far);
    textureStore(composite_out, vec2<u32>(id.x, row), weather_texel_direct(d, mip));
}
