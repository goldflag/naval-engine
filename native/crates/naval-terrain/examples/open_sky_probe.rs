//! Probe: the open sky on the CPU around the test land's steepest coast (the terrain review's `cliffs` target), as the
//! web baked it (`naval_terrain::open_sky::open_sky`, the spiral its generated WGSL walks) and as `TerrainBake.ts`
//! reads (`open_sky_as_written`), to check the GPU bake against (native only):
//! `cargo run -p naval-terrain --example open_sky_probe --release`.

#[path = "../tests/support.rs"]
mod support;

use naval_terrain::field::PreparedField;
use naval_terrain::open_sky::{BakeInput, open_sky, open_sky_as_written};
use naval_terrain::review;

fn main() {
    let field = support::land();
    let prepared = PreparedField::new(&field);
    let steep = review::steepest_coast(&field);
    let input = BakeInput {
        grid: prepared.grid,
        heights: &prepared.heights,
        gradients: &prepared.gradients,
    };
    for (dx, dz) in [
        (0.0, 0.0),
        (-200.0, 0.0),
        (200.0, 0.0),
        (0.0, -300.0),
        (-400.0, -400.0),
    ] {
        let (x, z) = (steep.x + dx, steep.z + dz);
        // The coarse sample under (x, z) and its field sample.
        let ci = ((x - f64::from(prepared.origin[0])) / prepared.coarse.cell).round() as u32;
        let cj = ((z - f64::from(prepared.origin[1])) / prepared.coarse.cell).round() as u32;
        let (i, j) = (ci * 2, cj * 2);
        let k = (j * prepared.grid.columns + i) as usize * 2;
        let slope = half::f16::from_bits(prepared.gradients[k])
            .to_f32()
            .hypot(half::f16::from_bits(prepared.gradients[k + 1]).to_f32());
        println!(
            "({x:.0}, {z:.0}) height {:.0} slope {slope:.2} open sky {:.3} (as written {:.3})",
            prepared.heights[(j * prepared.grid.columns + i) as usize],
            open_sky(&input, i, j),
            open_sky_as_written(&input, i, j),
        );
    }
}
