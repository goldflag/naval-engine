// One mip level from the one above it, a 2 × 2 box: what three's generated mipmaps give the gas atlas and its
// detail map after each store (the web game's `GasAtlas.ts` sets `generateMipmaps` on its storage textures).

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var destination: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(destination);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let p = vec2<i32>(id.xy) * 2;
    let sum = textureLoad(source, p, 0) + textureLoad(source, p + vec2<i32>(1, 0), 0)
        + textureLoad(source, p + vec2<i32>(0, 1), 0) + textureLoad(source, p + vec2<i32>(1, 1), 0);
    textureStore(destination, id.xy, sum * 0.25);
}
