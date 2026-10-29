//! A release's surveyed squares: the square grid's heights read **by place** ([`super::dem_at`]). Each supersampled
//! point reads Terrarium if its latitude lies in [−56°, 60°) and GLO-30 otherwise, so a square whose halo crosses 60° N
//! reads GLO-30 beyond it, and the despike sees the same DEM at a place whichever square asks. A square wholly on one
//! side gives the bytes `square::bake_terrarium` or `square::bake_glo30` give it: the same mosaic, the same arithmetic.
//!
//! Each DEM's mosaic is built in the square's continuous frame from its [`Side`](super::sources::Side)s: a tile or cell
//! is asked for by its wrapped name and placed a whole turn east or west, so the interpolation across ±180° reads the
//! two sides' pixels as neighbours. A GLO-30 cell the manifest lists absent is sea (0 m), as the bucket's absence is.
//!
//! An `ocean` or `polar` square's surveyed square is never read from the DEM ([`sea`]): a candidate's bake reads a
//! neighbour's surveyed heights only within its halo, and an ocean square has no coastline within 65 km of it, so no
//! operation's reach brings those heights into a final height (the sea's are the shelf's, which reads no DEM).
use std::collections::HashMap;
use std::sync::Arc;

use naval_land_format::SquareKey;

use super::sources::{CellKey, Reads, TileKey};
use super::{OSM_ATTRIBUTION, dem_at};
use crate::finish::surveyed::POLAR_SEA_M;
use crate::geo::{mercator_x, mercator_y};
use crate::square::bilinear;
use crate::square::{
    MARGIN, Square, SquareGrid, SquareSource, bake_heights, source_for, square_of,
};
use crate::terrarium::{DemTile, TILE_PX};
use crate::{BakeError, DEEP_M, Dem, glo30};

/// A Terrarium tile by its wrapped key, decoded.
pub type TileSource<'a> = &'a (dyn Fn(TileKey) -> Result<Arc<DemTile>, BakeError> + Sync);
/// A GLO-30 cell by its wrapped key, with every block the reads take (none: absent, sea).
pub type CellSource<'a> =
    &'a (dyn Fn(CellKey) -> Result<Option<Arc<dyn glo30::Cell + Send + Sync>>, BakeError> + Sync);

/// The Terrarium mosaic in the square's continuous frame.
struct TerrariumMosaic {
    data: Vec<f32>,
    columns: usize,
    rows: usize,
    scale: f64,
    bx: f64,
    by: f64,
}

fn terrarium_mosaic(reads: &Reads, tile: TileSource) -> Result<Option<TerrariumMosaic>, BakeError> {
    let Some(part) = &reads.terrarium else {
        return Ok(None);
    };
    let [x0, x1, y0, y1] = part.span;
    let n = 1i64 << part.zoom;
    let (columns, rows) = (
        ((x1 - x0 + 1) * 256) as usize,
        ((y1 - y0 + 1) * 256) as usize,
    );
    let mut data = vec![DEEP_M as f32; columns * rows];
    for side in reads.tile_sides() {
        for (x, y) in side.keys {
            let t = tile((part.zoom, x, y))?;
            if t.len() != TILE_PX * TILE_PX {
                return Err(BakeError::Corrupt(format!(
                    "Terrarium tile {}/{x}/{y} holds {} heights",
                    part.zoom,
                    t.len()
                )));
            }
            // The wrapped tile's place in the square's frame: a turn east or west.
            let at = x + side.turn * n;
            let (ox, oy) = (((at - x0) * 256) as usize, ((y - y0) * 256) as usize);
            for r in 0..256 {
                data[(oy + r) * columns + ox..(oy + r) * columns + ox + 256]
                    .copy_from_slice(&t[r * 256..(r + 1) * 256]);
            }
        }
    }
    Ok(Some(TerrariumMosaic {
        data,
        columns,
        rows,
        scale: (1u64 << part.zoom) as f64 * 256.0,
        bx: (x0 * 256) as f64,
        by: (y0 * 256) as f64,
    }))
}

/// Each GLO-30 part's mosaic in the square's continuous frame.
fn glo30_mosaics(reads: &Reads, cell: CellSource) -> Result<Vec<glo30::Mosaic>, BakeError> {
    let mut held: HashMap<CellKey, Option<Arc<dyn glo30::Cell + Send + Sync>>> = HashMap::new();
    for side in reads.cell_sides() {
        for key in side.keys {
            held.insert(key, cell(key)?);
        }
    }
    reads
        .glo30
        .iter()
        .map(|part| {
            glo30::mosaic(
                &part.window,
                |s, w| {
                    let wrapped = super::sources::wrap_lon(w).1;
                    held.get(&(s, wrapped))
                        .and_then(|c| c.as_ref())
                        .map(|c| c.as_ref() as &dyn glo30::Cell)
                },
                0.0,
            )
        })
        .collect()
}

/// Square `reads.key`'s surveyed square from the DEM by place.
pub fn bake(reads: &Reads, tile: TileSource, cell: CellSource) -> Result<Square, BakeError> {
    let key = reads.key;
    let terrarium = terrarium_mosaic(reads, tile)?;
    let glo = glo30_mosaics(reads, cell)?;
    let failed = std::cell::Cell::new(None);
    let heights = bake_heights(
        key,
        |lon| lon,
        |lat| lat,
        |lon, lat| match dem_at(lat) {
            Dem::Terrarium => match &terrarium {
                Some(t) => bilinear(
                    &t.data,
                    t.columns,
                    t.rows,
                    mercator_x(lon, t.scale) - 0.5 - t.bx,
                    mercator_y(lat, t.scale) - 0.5 - t.by,
                ),
                None => {
                    failed.set(Some(lat));
                    f64::NAN
                }
            },
            Dem::Glo30 => match reads.glo30.iter().zip(&glo).find(|(p, _)| p.serves(lat)) {
                Some((part, mosaic)) => mosaic.bilinear(&part.window, lat, lon),
                None => {
                    failed.set(Some(lat));
                    f64::NAN
                }
            },
        },
    );
    if let Some(lat) = failed.get() {
        return Err(BakeError::Refused(format!(
            "{key}'s reads have no DEM for latitude {lat}"
        )));
    }
    let attribution = match reads.only() {
        Some(Dem::Terrarium) => crate::square::TERRARIUM_ATTRIBUTION.to_string(),
        Some(Dem::Glo30) => format!("Copernicus DEM GLO-30, {}", glo30::ATTRIBUTION),
        None => format!(
            "{}; Copernicus DEM GLO-30, {}",
            crate::square::TERRARIUM_ATTRIBUTION,
            glo30::ATTRIBUTION
        ),
    };
    Ok(square_of(
        key,
        SquareSource {
            dem: source_for(key),
            zoom: reads.terrarium.as_ref().map(|t| t.zoom),
            fallback: false,
            attribution,
        },
        heights,
    ))
}

/// An `ocean` or `polar` square's surveyed square: the shelf's floor, read from nothing.
pub fn sea(key: SquareKey) -> Square {
    let grid = SquareGrid::new(key, MARGIN);
    square_of(
        key,
        SquareSource {
            dem: source_for(key),
            zoom: None,
            fallback: false,
            attribution: OSM_ATTRIBUTION.into(),
        },
        vec![POLAR_SEA_M; grid.rows * grid.columns],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glo30::{Cell, Header, HeldCell};
    use crate::square::{bake_glo30, bake_terrarium};

    use super::super::fixtures::{glo30_pixel, reference, terrarium_tile};

    /// GLO-30 cell (s, w wrapped) of the fixture's DEM, each block made when first read.
    struct LazyCell {
        header: Header,
        s: i32,
        w: i32,
        blocks: Vec<std::sync::OnceLock<Vec<f32>>>,
    }

    impl Cell for LazyCell {
        fn header(&self) -> &Header {
            &self.header
        }
        fn block(&self, k: usize) -> Option<&[f32]> {
            let h = &self.header;
            let (by, bx) = (k / h.across, k % h.across);
            let made = self.blocks.get(k)?.get_or_init(|| {
                let mut b = vec![0f32; h.block * h.block];
                for r in 0..h.block {
                    for c in 0..h.block {
                        let (rr, cc) = (by * h.block + r, bx * h.block + c);
                        if rr < h.height && cc < h.width {
                            b[r * h.block + c] = glo30_pixel(self.s, self.w, rr, cc);
                        }
                    }
                }
                b
            });
            Some(made)
        }
    }

    fn cell_of(s: i32, w: i32) -> Arc<dyn Cell + Send + Sync> {
        let width = glo30::columns_at(s);
        let block = 1024;
        let across = width.div_ceil(block);
        let down = 3600usize.div_ceil(block);
        Arc::new(LazyCell {
            header: Header {
                width,
                height: 3600,
                block,
                across,
                offsets: vec![0; across * down],
                counts: vec![0; across * down],
                dx: 1.0 / width as f64,
                dy: 1.0 / 3600.0,
                west: f64::from(w),
                north: f64::from(s) + 1.0,
            },
            s,
            w,
            blocks: (0..across * down)
                .map(|_| std::sync::OnceLock::new())
                .collect(),
        })
    }

    fn tile_of(z: u32, x: i64, y: i64) -> Arc<DemTile> {
        Arc::new(terrarium_tile(z, x, y))
    }

    fn tile_source() -> impl Fn(TileKey) -> Result<Arc<DemTile>, BakeError> + Sync {
        |(z, x, y): TileKey| {
            assert!(x >= 0 && x < 1 << z, "a tile asked for unwrapped: {x}");
            Ok(tile_of(z, x, y))
        }
    }

    type SharedCell = Arc<dyn Cell + Send + Sync>;

    fn cell_source() -> impl Fn(CellKey) -> Result<Option<SharedCell>, BakeError> + Sync {
        |(s, w): CellKey| {
            assert!((-180..180).contains(&w), "a cell asked for unwrapped: {w}");
            Ok(Some(cell_of(s, w)))
        }
    }

    #[test]
    fn across_the_date_line_the_bake_is_the_unwrapped_resample() {
        let (tile, cell) = (tile_source(), cell_source());
        // Terrarium at 17° S and GLO-30 at 86° N (one band of cells), each side of ±180°.
        for key in [
            SquareKey { lat: -17, lon: 179 },
            SquareKey {
                lat: -17,
                lon: -180,
            },
            SquareKey { lat: 86, lon: 179 },
            SquareKey { lat: 86, lon: -180 },
        ] {
            let reads = Reads::of(key);
            let got = bake(&reads, &tile, &cell).unwrap();
            let want = reference(&reads);
            let q = crate::square::HEIGHT_QUANTUM as f32;
            let worst = got
                .heights()
                .iter()
                .zip(&want)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(worst <= q, "{key}: {worst} m");
        }
    }

    #[test]
    fn a_square_on_one_side_of_the_dem_change_is_its_dems_own_bake() {
        let (tile, cell) = (tile_source(), cell_source());
        let key = SquareKey { lat: 36, lon: -123 };
        let reads = Reads::of(key);
        let got = bake(&reads, &tile, &cell).unwrap();
        let old = bake_terrarium(
            key,
            &|(x, y)| {
                Some(tile_of(
                    reads.terrarium.as_ref().unwrap().zoom,
                    x.rem_euclid(1 << 11),
                    y,
                ))
            },
            false,
        )
        .unwrap();
        assert_eq!(got.heights(), old.heights());
        let key = SquareKey { lat: 62, lon: 5 };
        let got = bake(&Reads::of(key), &tile, &cell).unwrap();
        let old = bake_glo30(key, &|(s, w)| Some(cell_of(s, w))).unwrap();
        assert_eq!(got.heights(), old.heights());
    }

    #[test]
    fn a_halo_across_60_north_reads_glo30_beyond_it() {
        // Terrarium and GLO-30 disagree by a kilometre here: the square at 59° N reads each on its own side of 60° N.
        let tile = |(z, x, y): TileKey| -> Result<Arc<DemTile>, BakeError> {
            let _ = (z, x, y);
            Ok(Arc::new(vec![-1000f32; 256 * 256]))
        };
        let cell = |(_, _): CellKey| -> Result<Option<Arc<dyn Cell + Send + Sync>>, BakeError> {
            Ok(Some(cell_flat(1000.0)))
        };
        let key = SquareKey { lat: 59, lon: 10 };
        let square = bake(&Reads::of(key), &tile, &cell).unwrap();
        let g = square.grid;
        let row = |lat: f64| ((g.lat(0) - lat) / g.dlat).round() as usize;
        // The margin's rows past 60° N are GLO-30's, the square's own Terrarium's (the despike leaves both flat).
        assert_eq!(
            square.heights()[row(60.0 + 1.0 / 1600.0) * g.columns + 10],
            1000.0
        );
        assert_eq!(square.heights()[row(59.5) * g.columns + 10], -1000.0);
        // The row on 60° N itself: two of its four points north of it (GLO-30), two south (Terrarium).
        assert_eq!(square.heights()[row(60.0) * g.columns + 10], 0.0);
    }

    fn cell_flat(v: f32) -> Arc<dyn Cell + Send + Sync> {
        Arc::new(HeldCell {
            header: Header {
                width: 2400,
                height: 3600,
                block: 1024,
                across: 3,
                offsets: vec![0; 12],
                counts: vec![0; 12],
                dx: 1.0 / 2400.0,
                dy: 1.0 / 3600.0,
                west: 0.0,
                north: 0.0,
            },
            blocks: (0..12)
                .map(|k| (k, Arc::new(vec![v; 1024 * 1024])))
                .collect(),
        })
    }

    #[test]
    fn an_ocean_square_is_the_shelf_floor() {
        let s = sea(SquareKey { lat: 10, lon: 10 });
        assert!(s.heights().iter().all(|&h| h == POLAR_SEA_M));
        assert!(!s.land);
    }
}
