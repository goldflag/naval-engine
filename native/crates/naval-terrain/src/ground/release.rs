//! A land release's heights as the land layer draws them, without the simulation: a window about a place
//! ([`ReleaseWindow`]), the squares it meets ([`ReleaseWindow::squares`]) read from their final square files
//! ([`ReleaseLand`]: `naval_land_format::nsq::FinalSquare`'s bytes, from a store, a download or the synthetic
//! release), and the [`Heightfield`] it cuts from them ([`ReleaseWindow::heightfield`]), ready for
//! [`crate::PreparedLand::new`] or [`crate::TerrainLayer::new`].
//!
//! The game's theatre cuts its windows through the simulation's chart and land (`naval_client`'s
//! `land_release::release_window`); this is the same cut for a host without the simulation, and it reads the same
//! heights: the chart is the theatre's (azimuthal equidistant about the window's place on a sphere of
//! [`EARTH_RADIUS_M`], x east and z south), a place's height is its square's lattice read bilinearly within its block
//! (`ReleaseLand::height` in `naval_sim::land::release`), a square the host has not given is the open sea's floor
//! ([`RELEASE_SEA_M`]), and the window's outer band settles toward [`OPEN_SEA_M`] and is quantised as the theatre's
//! windows are (`settled_window`). Unlike the theatre's, it raises no sample to the lattice's summits, so a peak a
//! sample misses reads up to half a cell low.

use std::collections::BTreeMap;

use naval_land_format::SquareKey;
use naval_land_format::block::Block;
use naval_land_format::lattice;
use naval_land_format::nsq::FinalSquare;

use crate::heightfield::{Heightfield, OPEN_SEA_M};

/// The sphere the theatre's chart is laid on, metres (`naval_sim::terrain::EARTH_RADIUS_M`).
pub const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// The height of a place in a square the release holds no data for, metres (`naval_sim::land::release::RELEASE_SEA_M`).
pub const RELEASE_SEA_M: f64 = -160.0;
/// A window's height quantum, metres (the theatre's `WINDOW_STEP_M`).
pub const WINDOW_STEP_M: f32 = 0.25;

/// A window of a release: `samples` on a side `cell` metres apart, centred on `place` (latitude, longitude, degrees),
/// its land settling below the sea over its outer `falloff` metres. Its chart's origin is the place, so the field's
/// sample (0, 0) stands at chart (−half, −half) ([`ReleaseWindow::half`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReleaseWindow {
    pub place: [f64; 2],
    pub samples: u32,
    pub cell: f64,
    pub falloff: f64,
}

impl ReleaseWindow {
    /// The theatre's sea window about `place`: 96 km at 40 m, the land settling over its outer 6 km (`naval_client`'s
    /// `WINDOW_SAMPLES`, `WINDOW_CELL_M`, `WINDOW_FALLOFF_M`).
    pub fn sea(place: [f64; 2]) -> Self {
        Self {
            place,
            samples: 2401,
            cell: 40.0,
            falloff: 6000.0,
        }
    }

    /// Half the window's side, metres.
    pub fn half(&self) -> f64 {
        f64::from(self.samples - 1) * self.cell / 2.0
    }

    /// The latitude and longitude (degrees, longitude in [−180, 180)) of chart point `q` (metres east and south of the
    /// place): the azimuthal equidistant chart about it, as the theatre's (`naval_sim::earth::sphere::Chart::inverse`).
    pub fn lat_lon(&self, q: [f64; 2]) -> [f64; 2] {
        let (east, north) = (q[0] / EARTH_RADIUS_M, -q[1] / EARTH_RADIUS_M);
        let c = east.hypot(north);
        if c == 0.0 {
            return self.place;
        }
        let (lat0, lon0) = (self.place[0].to_radians(), self.place[1].to_radians());
        let (sin_c, cos_c) = c.sin_cos();
        let (sin0, cos0) = lat0.sin_cos();
        let lat = (sin0 * cos_c + cos0 * sin_c * north / c)
            .clamp(-1.0, 1.0)
            .asin();
        let lon = lon0 + (east / c * sin_c * cos0).atan2(cos_c - sin0 * lat.sin());
        let lon = lon.to_degrees();
        [
            lat.to_degrees(),
            lon - ((lon + 180.0) / 360.0).floor() * 360.0,
        ]
    }

    /// Every square the window meets, in key order: the squares of the places along its edges, a sample apart, and of a
    /// kilometre's grid within (a square the window holds whole is far wider than that). The host reads these
    /// squares' files for [`ReleaseLand::new`].
    pub fn squares(&self) -> Vec<SquareKey> {
        let half = self.half();
        let mut keys = std::collections::BTreeSet::new();
        let mut at = |q: [f64; 2]| {
            let [lat, lon] = self.lat_lon(q);
            keys.insert(SquareKey::at(lat, lon));
        };
        for i in 0..self.samples {
            let t = -half + f64::from(i) * self.cell;
            for q in [[t, -half], [t, half], [-half, t], [half, t]] {
                at(q);
            }
        }
        let steps = (2.0 * half / 1000.0).ceil().max(1.0) as u32;
        for j in 0..=steps {
            for i in 0..=steps {
                let step = 2.0 * half / f64::from(steps);
                at([-half + f64::from(i) * step, -half + f64::from(j) * step]);
            }
        }
        keys.into_iter().collect()
    }

    /// The window's heightfield on `land`: each sample its place's height, settled toward [`OPEN_SEA_M`] over the
    /// outer `falloff` and quantised to [`WINDOW_STEP_M`] (nothing on the waterline); `None` when no sample is land.
    pub fn heightfield(&self, land: &ReleaseLand) -> Option<Heightfield> {
        let n = self.samples as usize;
        let half = self.half();
        let mut reader = Reader::default();
        let mut quanta = Vec::with_capacity(n * n);
        for j in 0..n {
            let z = -half + j as f64 * self.cell;
            for i in 0..n {
                let x = -half + i as f64 * self.cell;
                let [lat, lon] = self.lat_lon([x, z]);
                let h = reader.height(land, lat, lon);
                let edge = i.min(n - 1 - i).min(j).min(n - 1 - j) as f64 * self.cell;
                let keep = smoothstep(self.falloff, edge);
                quanta.push(quantum(
                    h * keep + OPEN_SEA_M * (1.0 - keep),
                    f64::from(WINDOW_STEP_M),
                ));
            }
        }
        quanta.iter().any(|&q| q > 0).then(|| Heightfield {
            columns: self.samples,
            rows: self.samples,
            cell: self.cell as f32,
            origin_x: -half as f32,
            origin_z: -half as f32,
            step: WINDOW_STEP_M,
            quanta,
        })
    }
}

/// The squares of a release a window reads: their final square files by key, every other square the open sea's
/// floor ([`RELEASE_SEA_M`]: the release's ocean squares, its empty ones, and any the host did not give).
#[derive(Clone, Debug, Default)]
pub struct ReleaseLand {
    squares: BTreeMap<SquareKey, FinalSquare>,
}

impl ReleaseLand {
    /// The land of these final square files (each square's file holds its key); refused when one does not read.
    pub fn new(files: impl IntoIterator<Item = Vec<u8>>) -> Result<Self, String> {
        let mut squares = BTreeMap::new();
        for bytes in files {
            let square = FinalSquare::read(bytes).map_err(|e| e.to_string())?;
            squares.insert(square.key(), square);
        }
        Ok(Self { squares })
    }

    /// The squares it holds, in key order.
    pub fn keys(&self) -> impl Iterator<Item = SquareKey> + '_ {
        self.squares.keys().copied()
    }

    /// The height at (`lat`, `lon`), metres, decoding the block it lies in.
    pub fn height(&self, lat: f64, lon: f64) -> f64 {
        Reader::default().height(self, lat, lon)
    }
}

/// The blocks a window has read, each decoded once (a block that does not decode reads as the open sea's floor).
#[derive(Default)]
struct Reader {
    blocks: BTreeMap<(SquareKey, [usize; 2]), Option<Block>>,
}

impl Reader {
    fn height(&mut self, land: &ReleaseLand, lat: f64, lon: f64) -> f64 {
        let key = SquareKey::at(lat, lon);
        let Some(square) = land.squares.get(&key) else {
            return RELEASE_SEA_M;
        };
        let cell = lattice::cell(lattice::position(key, lat, lon));
        self.blocks
            .entry((key, cell.block))
            .or_insert_with(|| square.block(cell.block[0], cell.block[1]).ok())
            .as_ref()
            .map_or(RELEASE_SEA_M, |block| block.bilinear_m(cell.at))
    }
}

fn smoothstep(width: f64, x: f64) -> f64 {
    let t = (x / width).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Metres to a stored quantum: nothing rounds onto the waterline.
fn quantum(h: f64, step: f64) -> i16 {
    match (h / step).round().clamp(i16::MIN as f64, i16::MAX as f64) as i16 {
        0 if h > 0.0 => 1,
        0 => -1,
        q => q,
    }
}
