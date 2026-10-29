// The impostor bake's scratch (`tree_bake.wgsl`: one species' frames drawn at twice the atlas's side) brought down
// into the atlas, four samples a texel: layer `index` takes the colour (sRGB-encoded, premultiplied by the coverage) and
// the coverage, layer `models + index` the normal and depth, each the mean of the samples the tree covers, and the
// material the more of them are with their mean sky, premultiplied by the coverage as the colour is (a filtered edge
// never takes an empty texel's material, and a texel never blends bark's and leaves'). A texel the tree leaves empty takes the crown's envelope there for its normal (outward and toward
// the frame), so a filtered edge leans its normal outward as a crown's rim does.
#import naval_terrain::impostor::{IMPOSTOR_FRAMES, FRAME_TEXELS, LEAF_LAYERS, view_of, frame_direction, encode_normal, decode_normal}

struct BakeModel {
    index: u32,
    radius: f32,
    pad: vec2<u32>,
}

@group(0) @binding(0) var<uniform> model: BakeModel;
@group(0) @binding(1) var colors: texture_2d<f32>;
@group(0) @binding(2) var normals: texture_2d<f32>;
@group(0) @binding(3) var atlas: texture_storage_2d_array<rgba8unorm, write>;

fn srgb_encode(c: vec3<f32>) -> vec3<f32> {
    let low = c * 12.92;
    let high = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, c <= vec3<f32>(0.0031308));
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let texels = u32(FRAME_TEXELS);
    let side = u32(IMPOSTOR_FRAMES) * texels;
    if any(id.xy >= vec2<u32>(side)) {
        return;
    }
    var hits = 0.0;
    var color = vec3<f32>(0.0);
    var normal = vec3<f32>(0.0);
    var depth = 0.0;
    var leaf_hits = 0.0;
    var leaf_packed = 0.0;
    var bark_packed = 0.0;
    for (var k = 0u; k < 4u; k += 1u) {
        let at = vec2<i32>(id.xy * 2u + vec2<u32>(k & 1u, k >> 1u));
        let c = textureLoad(colors, at, 0);
        if c.a > 0.5 {
            let n = textureLoad(normals, at, 0);
            hits += 1.0;
            color += c.rgb;
            normal += decode_normal(n.xy);
            depth += n.z;
            let leaf = n.w > 0.5;
            leaf_hits += select(0.0, 1.0, leaf);
            leaf_packed += select(0.0, n.w, leaf);
            bark_packed += select(n.w, 0.0, leaf);
        }
    }
    let layer = i32(model.index);
    let models = i32((textureNumLayers(atlas) - LEAF_LAYERS) / 2u);
    if hits <= 0.0 {
        let cell = vec2<f32>(id.xy / texels);
        let frame = view_of(frame_direction(cell), model.radius);
        let sub = (vec2<f32>(id.xy % vec2<u32>(texels)) + 0.5) / FRAME_TEXELS * 2.0 - 1.0;
        let envelope = normalize(frame.right * sub.x * 0.8 - frame.up * sub.y * 0.8 + frame.toward * 0.6);
        textureStore(atlas, vec2<i32>(id.xy), layer, vec4<f32>(0.0));
        textureStore(atlas, vec2<i32>(id.xy), models + layer, vec4<f32>(encode_normal(envelope), 0.5, 0.0));
        return;
    }
    let coverage = hits * 0.25;
    let bark_hits = hits - leaf_hits;
    let packed = select(bark_packed / max(bark_hits, 1.0), leaf_packed / max(leaf_hits, 1.0), leaf_hits >= bark_hits);
    textureStore(atlas, vec2<i32>(id.xy), layer, vec4<f32>(srgb_encode(color / hits) * coverage, coverage));
    textureStore(
        atlas,
        vec2<i32>(id.xy),
        models + layer,
        vec4<f32>(encode_normal(normalize(normal + vec3<f32>(0.0, 1e-5, 0.0))), depth / hits, packed * coverage),
    );
}
