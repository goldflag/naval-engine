// The land's open sky, baked once on the GPU (`SkyBake` in the web game's `terrain/TerrainBake.ts`) as the game's generated
// WGSL runs it, not as the TypeScript reads.
//
// `TerrainBake.ts` means to walk 16 directions out to 2.8 km in 20 steps growing by a quarter each, find the horizon the
// land raises above the sample's own tangent plane in each, and average each direction's cosine-weighted share of the open
// sky. But its two TSL `Loop`s both name their counter `i` (LoopNode's `getVarName(0)`), so the inner
// `for (var i …)` shadows the outer one, and the direction, built only where the inner loop first uses it, is emitted
// inside the inner loop from the inner counter (`native/crates/naval-terrain/web/open-sky-bake-check.ts` dumps the WGSL
// and reads the bake back: it matches this to 8-bit rounding). So each step `s` walks its own direction, s × 22.5° +
// 0.2 rad, at 40 × 1.25^s m: one spiral of 20 samples whose highest rise over the tangent plane sets the horizon, and the
// outer loop repeats that same spiral 16 times. That is the darker open sky in valleys and gullies the game ships; the
// as-written walk is `open_sky_as_written` in `examples/open_sky_probe.rs`.
#import naval_terrain::bindings::{terrain, gradients, sample_height}

@group(0) @binding(0) var open_sky_out: texture_storage_2d<rgba8unorm, write>;

const SKY_DIRECTIONS: i32 = 16;
const SKY_STEPS: i32 = 20;
const SKY_FIRST_STEP_M: f32 = 40.0;
const SKY_STEP_GROWTH: f32 = 1.25;
// Samples of the field per sample of the attribute grid (`ATTRIBUTE_STRIDE`).
const STRIDE: i32 = 2;

@compute @workgroup_size(8, 8)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(open_sky_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let columns = i32(terrain.grid.x);
    let rows = i32(terrain.grid.y);
    let cell = terrain.field.z;
    // This texel's field sample, its height and the slope of its tangent plane.
    let texel = vec2<i32>(id.xy);
    let at = texel * STRIDE;
    let base = sample_height(at);
    let slope = textureLoad(gradients, at, 0).xy;
    // The spiral: step `s` along direction `s` (the shadowed counter), its rise over the tangent plane along that
    // direction. The web's 16 outer passes repeat it exactly, so it is walked once and its share taken whole.
    var horizon = 0.0;
    var distance = SKY_FIRST_STEP_M;
    for (var s = 0; s < SKY_STEPS; s++) {
        let angle = f32(s) * (2.0 * 3.141592653589793 / f32(SKY_DIRECTIONS)) + 0.2;
        let direction = vec2<f32>(cos(angle), sin(angle));
        let point = vec2<f32>(at) + direction * (distance / cell);
        let index = vec2<i32>(i32(clamp(round(point.x), 0.0, f32(columns - 1))), i32(clamp(round(point.y), 0.0, f32(rows - 1))));
        let height = sample_height(index);
        horizon = max(horizon, (height - base) / distance - dot(slope, direction));
        distance *= SKY_STEP_GROWTH;
    }
    textureStore(open_sky_out, texel, vec4<f32>(1.0 / (horizon * horizon + 1.0), 0.0, 0.0, 1.0));
}
