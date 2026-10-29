// The planting grid's cell hash, shared by the trees' placement (`tree_place.wgsl`) and the grass's (`grass_place.wgsl`).
#define_import_path naval_terrain::cells

// A float hash of a cell index in 0–1, stable per cell and uncorrelated between salts (`hash` in TerrainTrees.ts).
// An integer hash (pcg2d, Jarzynski and Olano 2020): the web's `fract(sin(dot(cell, k)) * 43758.5)` ran out of f32
// precision at cells some thousands from the chart's corner, and the jitter it gave lined the trees up in diagonal rows.
fn cell_hash(cell: vec2<f32>, salt: f32) -> f32 {
    let s = u32(salt);
    var v = bitcast<vec2<u32>>(vec2<i32>(cell)) * 1664525u + vec2<u32>(1013904223u + s * 2654435769u, 1013904223u ^ (s * 2246822519u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1664525u;
    v = v ^ (v >> vec2<u32>(16u));
    return f32(v.x >> 8u) / 16777216.0;
}
