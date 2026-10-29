// The forest's shade, baked on the attribute grid: the trees standing per tree cell and their mean height, from the
// masks the placement reads (`tree_cover`, as `tree_place.wgsl` finds a slot's tree), averaged over 2 × 2 points of each
// texel. `naval_terrain::forest` darkens the forest floor and the lower crowns by it where the shadow maps carry no tree.
// Baked whole when the layer is made (after the open sky its masks read) and when the surface maps change, then only
// over the rectangles whose inputs changed (`naval_terrain::forest_shade`): a map feature page or a near tile in or out
// of its slot. Into the open sky's spare green and blue channels: written here into another texture, with the open sky's
// red, and each rectangle copied over the open sky. A clearing is applied where it is read.
#import naval_terrain::bindings::{terrain, gradients, attributes, open_sky, clamp_sampler, height_at, field_uv, coarse_uv, chart_origin}
#import naval_terrain::surface::{SurfaceInputs, tree_cover}

// The open sky (red, as baked), the trees standing per cell (green) and their mean height over `FOREST_HEIGHT_M` (blue).
@group(0) @binding(0) var shade: texture_storage_2d<rgba8unorm, write>;

// The rectangles this bake covers (`forest_shade::SHADE_RECTS`): each one's first texel and size; one layer of the
// dispatch's workgroups each.
const SHADE_RECTS: u32 = 8u;
struct ShadeRects {
    rects: array<vec4<u32>, SHADE_RECTS>,
}
@group(0) @binding(1) var<uniform> shade_rects: ShadeRects;

const FOREST_HEIGHT_M: f32 = 50.0;

@compute @workgroup_size(8, 8)
fn bake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.z >= SHADE_RECTS {
        return;
    }
    let rect = shade_rects.rects[id.z];
    if any(id.xy >= rect.zw) {
        return;
    }
    let texel = rect.xy + id.xy;
    let cell = terrain.params.x;
    let centre = chart_origin() + vec2<f32>(texel) * cell;
    var standing = 0.0;
    var tall = 0.0;
    for (var k = 0u; k < 4u; k += 1u) {
        let at = centre + (vec2<f32>(f32(k & 1u), f32(k >> 1u)) - 0.5) * cell * 0.5;
        let ground = height_at(at);
        // The placement stands no tree below 1.5 m.
        if ground < 1.5 {
            continue;
        }
        let cuv = coarse_uv(at);
        let ground_data = textureSampleLevel(attributes, clamp_sampler, cuv, 0.0);
        var inputs: SurfaceInputs;
        inputs.chart = at;
        inputs.height = ground;
        inputs.gradient = textureSampleLevel(gradients, clamp_sampler, field_uv(at), 0.0).xy;
        inputs.footprint = 0.0;
        inputs.sky = textureSampleLevel(open_sky, clamp_sampler, cuv, 0.0).r;
        inputs.drainage = ground_data.r;
        inputs.broad = ground_data.g;
        inputs.coast = ground_data.b;
        inputs.fine = ground_data.a;
        let cover = tree_cover(inputs, 0.5);
        standing += cover.density;
        tall += cover.density * cover.height;
    }
    let height = tall / max(standing, 1e-4);
    let sky = textureLoad(open_sky, vec2<i32>(texel), 0).r;
    textureStore(shade, vec2<i32>(texel), vec4<f32>(sky, standing * 0.25, height / FOREST_HEIGHT_M, 1.0));
}
