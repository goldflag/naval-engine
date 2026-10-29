//! The coast: a window of the synthetic land release (`naval_land_format::synthetic`, made in code, so nothing is
//! downloaded), cut into the land's heightfield by `naval_terrain::release`. A host with a real release reads the same
//! squares' files from its store instead.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use naval_land_format::synthetic;
use naval_ocean::shore::ShoreMap;
use naval_terrain::Heightfield;
use naval_terrain::release::{ReleaseLand, ReleaseWindow};

/// The window: the theatre's sea window (96 km at 40 m, the land settling below the sea over its outer 6 km) on Long
/// Point's south coast, the synthetic world's peninsula along 30° S, 30 km wide. The window's chart has its origin on
/// the shore: the land rises to the north (−Z) at 25 m a kilometre to the peninsula's spine 15 km in, and falls to its
/// north coast 30 km in; the sea to the south (+Z) shelves to 160 m.
pub fn window() -> ReleaseWindow {
    ReleaseWindow::sea(synthetic::place([0.0, -15.0]))
}

/// The window's heightfield, from the squares it meets.
pub fn heightfield() -> Result<Heightfield> {
    let started = Instant::now();
    let window = window();
    let squares = window.squares();
    let land = ReleaseLand::new(squares.iter().filter_map(|&key| synthetic::square(key)))
        .map_err(anyhow::Error::msg)?;
    let field = window
        .heightfield(&land)
        .context("the coast's window holds no land")?;
    println!(
        "coast: {} squares ({}), {}² samples, cut in {:.0} ms",
        squares.len(),
        squares
            .iter()
            .map(|key| key.name())
            .collect::<Vec<_>>()
            .join(", "),
        field.columns,
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(field)
}

/// What the waves know of the coast (they shoal and shelter against it), with the wind blowing toward `toward`
/// (radians from +X toward +Z).
pub fn shore(field: &Heightfield, toward: f64) -> Arc<ShoreMap> {
    let heights = field
        .quanta
        .iter()
        .map(|&q| f32::from(q) * field.step)
        .collect();
    Arc::new(ShoreMap::build(
        heights,
        field.columns as usize,
        field.rows as usize,
        f64::from(field.cell),
        [f64::from(field.origin_x), f64::from(field.origin_z)],
        toward,
    ))
}
