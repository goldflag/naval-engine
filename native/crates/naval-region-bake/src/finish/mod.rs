//! Coast finishing on the square grid (step C, BAKE-1): a release's final square baked alone, on the one global final
//! lattice (`naval_land_format::lattice`), so that **a final lattice point's value is a function of its place alone**.
//! Two squares baked apart, in a batch, twice or on any number of threads give the same bytes, and the same bytes on
//! every shared edge, within a band, across the surveyed layer's band changes, across the DEM's change at 60° N and
//! 56° S, across ±180° and at the pole limit.
//!
//! A point reads the surveyed heights by owner ([`surveyed`]), the water polygons ([`coverage`]: the sea is theirs),
//! and its operations' neighbourhoods. Every operation is one of two kinds (the open-ocean plan, step C §3.1):
//!
//! - **Local**, a fixed reach in metres, each row's columns by its latitude ([`lattice`]): the supersampling and the
//!   despike (in the surveyed squares, on their bands' grids), the coast reconciled with the DEM ([`coast::slide`],
//!   [`coast::cut`]), the coast rounded ([`coast::mean`], [`coast::rounded`]), the shelf and the heights
//!   ([`coast::heights`]), the quantization to 0.25 m.
//! - **Component**, decided by the component over its own window, capped at
//!   [`COMPONENT_CAP_M`](components::COMPONENT_CAP_M) ([`components`]): islands rounding would sink whole kept,
//!   specks removed, lakes (water the land encloses) filled; a larger component is kept as it is.
//!
//! The bake holds the square's window and a **halo** ([`Halo`]): the sum of its stages' reaches in rows, and in
//! columns at the narrowest column the halo reaches. Each stage computes only what the next can trust, a region its
//! reach inside its input, so the last stage's region is exactly the window; a point can never be computed from less
//! than its neighbourhood. Everything is `+ − × ÷` and square roots in a fixed order, or integers, so every target
//! makes the same quanta.
//!
//! Not in `land-v1` (the region bake's steps that are neither kind): river and dock joining (decided once for a whole
//! region: the water behind the coastline is land to the simulation), and the DEM deciding the coast where no water
//! tile was read (a release's snapshot covers every place). The drawn layers (the ETOPO sea floor, the shoreline band,
//! lake levels) stay the client's: the final layer is the simulation's heights alone. Enclosed water is filled only
//! within the cap: a closed lagoon larger than it stays sea, as the water polygons say.
//!
//! **The two bakes** ([`VERSIONS`]; a release names its own in its file and is baked by it, so a release's squares never
//! change):
//!
//! - **1** (`land-v1`): the sea floor is the shelf, 2.5 m deep at the coast and 0.12 m deeper a metre off it, flat at
//!   160 m from 1.3 km out; a candidate square holds data when final land lies within [`keep::KEEP_M`] of it, else it
//!   is `empty`.
//! - **2** (`land-v2`): the sea floor is the release's bathymetry by place ([`Seabed`]: GEBCO's grid,
//!   `release::gebco`) wherever the water polygons say sea ([`coast::seabed_depth`]): near the coast it rises to the
//!   coast as the shelf does, and where the bathymetry's own coast is not the polygons' (it reads land or a few metres
//!   at sea) the shelf fills it; it fades to the open sea's floor (160 m, what the simulation reads for a square with
//!   no data) within [`OPEN_FADE_M`] of a square the release calls open sea (`ocean` or `polar`), and every candidate
//!   square holds data, so the floor is one continuous surface across every edge. The coast rounding keeps the
//!   coastline's land ([`coast::rounded`]): it fills a notch but never cuts a headland's tip back, which the first bake's
//!   did wherever the tip was narrower than the mean's 280 m (Cape Point's lost its last 100 m). Every other land
//!   height is the first bake's, to the quantum.
//!
//! A polar square (past the pole limit) is open sea and never baked; its places, and the edge a baked square shares
//! with it (89° N and S), are sea in the bake's sources ([`lattice::polar_row`]), so the coast is drawn and rounded
//! short of that edge and every point of it is sea.
pub mod coast;
pub mod components;
pub mod coverage;
pub mod distance;
pub mod keep;
pub mod lattice;
pub mod surveyed;
pub mod synthetic;

use naval_land_format::SquareKey;
use naval_land_format::block::Window;
use naval_land_format::release::BakeSpec;
use naval_land_format::table::{POLE_LIMIT_DEG, polar};

pub use self::coast::seabed_depth;
use self::coast::{CUT_REACH_M, HeightInputs, OLD_COAST_M, SHELF_REACH_M, SLIDE_DISTANCE_M};
use self::coverage::Water;
use self::lattice::{Plane, ROW_M, Region, Spacing, column_m, polar_row};
use self::surveyed::Surveyed;
use crate::{BakeError, LAND_MIN_M};

/// The bake's name and version in a release's file: a change to any rule or constant here, or in what a release's
/// sources give the bake (`release::surveyed`'s heights by place, `release::pieces`' cut, `release::table`'s classes),
/// is a new version, and a new release.
pub const ALGORITHM: &str = "naval-region-bake";
/// The latest bake, the one a new release is made for.
pub const VERSION: u32 = 2;
/// Every bake this module still bakes: a release made for any of them is baked by it.
pub const VERSIONS: [u32; 2] = [1, 2];
/// The second bake's sea floor fades to the open sea's within this of a square the release calls open sea.
pub const OPEN_FADE_M: f64 = 20_000.0;

/// The release file's bake rules for squares the latest bake makes.
pub fn bake_spec() -> BakeSpec {
    bake_spec_for(VERSION)
}

/// The release file's bake rules for bake `version` ([`VERSIONS`]). The second keeps every candidate square
/// (`keep_m` 0).
pub fn bake_spec_for(version: u32) -> BakeSpec {
    assert!(VERSIONS.contains(&version), "no bake {version}");
    BakeSpec {
        algorithm: ALGORITHM.into(),
        version,
        keep_m: if version >= 2 { 0 } else { keep::KEEP_M as u32 },
        candidate_m: keep::CANDIDATE_M as u32,
        pole_limit_deg: POLE_LIMIT_DEG,
    }
}

/// The bake a release's file names, when this module bakes it.
pub fn version_of(spec: &BakeSpec) -> Option<u32> {
    VERSIONS.into_iter().find(|&v| *spec == bake_spec_for(v))
}

/// Whether bake `version` keeps every candidate square: a square's line is then always its data, never `empty`.
pub fn keeps_every_candidate(version: u32) -> bool {
    version >= 2
}

/// The sea floor the second bake reads ([`VERSIONS`]), and which squares are open sea.
pub trait Seabed {
    /// The bathymetry's height at each point of `region` (metres, below the sea negative), each a function of the
    /// point's place alone.
    fn floor(&self, region: &Region) -> Result<Plane<f64>, BakeError>;
    /// Whether square `key` is open sea in the release (its table's `ocean` or `polar`): the simulation reads it as the
    /// open sea's floor, [`crate::SHELF_DEPTH_M`] deep.
    fn open_sea(&self, key: SquareKey) -> bool;
}

/// Each point of `window`'s distance to the nearest square `sea` calls open sea, metres on the lattice
/// ([`keep::to_square_m`]), at most [`OPEN_FADE_M`]: a function of the point and the release's table alone. Every open
/// square within the fade of a point of the square is looked at (the squares about it, east and west as far as the fade
/// reaches at the window's poleward row, with a step to spare).
pub fn open_distance(key: SquareKey, window: Region, sea: &dyn Seabed) -> Plane<f32> {
    let poleward = window.grow(1, 0).poleward();
    let degree = column_m(poleward.clamp(
        -naval_land_format::lattice::POLE + 1,
        naval_land_format::lattice::POLE - 1,
    )) * f64::from(naval_land_format::lattice::PER_DEGREE);
    let across = ((OPEN_FADE_M / degree).ceil() as i32 + 1).min(180);
    let mut open = vec![];
    for lat in key.lat - 1..=key.lat + 1 {
        if !(-90..=89).contains(&lat) {
            continue;
        }
        for dl in -across..=across {
            let n = SquareKey {
                lat,
                lon: (key.lon + dl + 180).rem_euclid(360) - 180,
            };
            if n != key && !open.contains(&n) && sea.open_sea(n) {
                open.push(n);
            }
        }
    }
    Plane::from_fn(window, |r, c| {
        let (i, j) = (window.i(r), window.j(c));
        let mut d = OPEN_FADE_M;
        for &n in &open {
            d = d.min(keep::to_square_m(n, i, j));
        }
        d as f32
    })
}

/// The stages in order, each a reach in metres and the samples past it its arithmetic reads (a gradient, a slack
/// sample).
pub const STAGES: [(&str, f64, usize); 6] = [
    // The coasts' signed distances, their gradient, a sample of slack (the slid profile reads less).
    ("slide", SLIDE_DISTANCE_M, 2),
    ("cut", CUT_REACH_M, 1),
    ("islands kept", components::COMPONENT_CAP_M, 1),
    ("specks", components::COMPONENT_CAP_M, 1),
    ("lakes", components::COMPONENT_CAP_M, 1),
    ("shelf", SHELF_REACH_M, 1),
];

/// What a square's bake holds about its window, stage by stage.
#[derive(Clone, Debug, PartialEq)]
pub struct Halo {
    /// Each stage's reach, rows and columns.
    pub stages: Vec<(usize, usize)>,
    /// The narrowest column the halo reaches, metres: every stage's columns are counted at it.
    pub column_min_m: f64,
}

impl Halo {
    /// The halo of square `key`.
    pub fn of(key: SquareKey) -> Self {
        Self::with_slack(key, 0, 0)
    }

    /// The halo of square `key` with `rows` and `columns` more than its reach at every stage: the same window (a test
    /// of every stage reading no farther than its reach, and of no stage's arithmetic depending on where its region
    /// starts).
    pub fn with_slack(key: SquareKey, rows: usize, columns: usize) -> Self {
        let stage_rows: Vec<usize> = STAGES
            .iter()
            .map(|&(_, m, extra)| (m / ROW_M).ceil() as usize + extra + rows)
            .collect();
        let total: usize = stage_rows.iter().sum();
        let column_min_m = column_m(Region::window(key).grow(total, 0).poleward());
        let stages = STAGES
            .iter()
            .zip(stage_rows)
            .map(|(&(_, m, extra), r)| (r, (m / column_min_m).ceil() as usize + extra + columns))
            .collect();
        Self {
            stages,
            column_min_m,
        }
    }

    pub fn rows(&self) -> usize {
        self.stages.iter().map(|s| s.0).sum()
    }

    pub fn columns(&self) -> usize {
        self.stages.iter().map(|s| s.1).sum()
    }

    /// The region each stage computes, from the whole held region to the window.
    pub fn regions(&self, key: SquareKey) -> Vec<Region> {
        let mut region = Region::window(key).grow(self.rows(), self.columns());
        let mut out = vec![region];
        for &(r, c) in &self.stages {
            region = region.shrink(r, c);
            out.push(region);
        }
        assert_eq!(region, Region::window(key));
        out
    }
}

/// What a bake did in the square's own window, sample by sample: for a report, and for tests to see each operation act.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Samples whose heights the slide moved onto the coastline, and those the cut lowered.
    pub slid: usize,
    pub lowered: usize,
    /// Samples rounding made land, and sank.
    pub rounded_up: usize,
    pub rounded_down: usize,
    /// Samples of islands kept whole, of specks dropped, of lakes filled.
    pub islands_kept: usize,
    pub specks: usize,
    pub filled: usize,
    /// Land samples in the final window.
    pub land: usize,
    /// Sea samples whose floor the second bake read from its bathymetry (none in the first).
    #[serde(skip_serializing_if = "is_zero")]
    pub floor: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// A square's final window and what made it.
#[derive(Clone, Debug, PartialEq)]
pub struct Finished {
    pub key: SquareKey,
    pub window: Window,
    pub report: Report,
}

impl Finished {
    /// Its final square's bytes (NSQ1's final layer alone): what a release's manifest hashes.
    pub fn bytes(&self) -> Vec<u8> {
        naval_land_format::FinalSquare::encode(self.key, &self.window, Default::default())
    }
}

/// Counts the window points where `f` holds.
fn count(window: Region, f: impl Fn(i32, i32) -> bool) -> usize {
    let mut n = 0;
    for r in 0..window.rows {
        for c in 0..window.columns {
            n += usize::from(f(window.i(r), window.j(c)));
        }
    }
    n
}

/// Square `key`'s final window by the first bake, from the surveyed layer and the water polygons. A polar square is
/// refused: the release never bakes one.
pub fn bake(
    key: SquareKey,
    surveyed: &dyn Surveyed,
    water: &dyn Water,
) -> Result<Finished, BakeError> {
    refuse(key)?;
    bake_in(key, surveyed, water, &Halo::of(key))
}

/// Square `key`'s final window by bake `version` ([`VERSIONS`]): the second reads its sea floor from `seabed`, which it
/// must be given.
pub fn bake_version(
    version: u32,
    key: SquareKey,
    surveyed: &dyn Surveyed,
    water: &dyn Water,
    seabed: Option<&dyn Seabed>,
) -> Result<Finished, BakeError> {
    refuse(key)?;
    bake_with(
        key,
        surveyed,
        water,
        &Halo::of(key),
        sea_of(version, seabed)?,
    )
}

/// The sea floor bake `version` reads: none for the first, `seabed` for the second.
fn sea_of(version: u32, seabed: Option<&dyn Seabed>) -> Result<Option<&dyn Seabed>, BakeError> {
    match (version, seabed) {
        (1, _) => Ok(None),
        (2, Some(s)) => Ok(Some(s)),
        (2, None) => Err(BakeError::Refused(
            "the second bake reads a sea floor, and none was given".into(),
        )),
        (v, _) => Err(BakeError::Refused(format!("no bake {v}"))),
    }
}

/// A polar square, or a key that is not a square, is never baked.
fn refuse(key: SquareKey) -> Result<(), BakeError> {
    if !key.valid() || polar(key) {
        return Err(BakeError::Refused(format!(
            "{key} is polar or not a square: never baked"
        )));
    }
    Ok(())
}

/// [`bake`] holding `halo` about the window: [`Halo::of`]'s, or one with slack ([`Halo::with_slack`]), which gives the
/// same window. A halo short of a stage's reach is a mutant for tests: a local stage refuses it (a panic), a component
/// stage decides its components from what the square holds, and squares stop agreeing.
pub fn bake_in(
    key: SquareKey,
    surveyed: &dyn Surveyed,
    water: &dyn Water,
    halo: &Halo,
) -> Result<Finished, BakeError> {
    bake_with(key, surveyed, water, halo, None)
}

/// [`bake_in`] by the second bake when `seabed` is given (its sea floor), else by the first.
pub fn bake_with(
    key: SquareKey,
    surveyed: &dyn Surveyed,
    water: &dyn Water,
    halo: &Halo,
    seabed: Option<&dyn Seabed>,
) -> Result<Finished, BakeError> {
    refuse(key)?;
    let regions = halo.regions(key);
    let [held, slid_at, cut_at, kept_at, specks_at, lakes_at, window] = regions[..] else {
        unreachable!("six stages")
    };
    let spacing = Spacing::of(&held);
    let mut report = Report::default();

    // Sampling: the surveyed heights by owner and each point's share of the sea.
    let raw = surveyed::heights(&held, surveyed)?;
    let cover = coverage::ocean_cover(&held, water)?;
    // A polar place is water whatever the polygons say (its heights are the open sea's: `surveyed::heights`).
    let cover = Plane::from_fn(held, |r, c| {
        if polar_row(held.i(r)) {
            coverage::FULL
        } else {
            cover.at(r, c)
        }
    });
    let (land, outline) = coast::coastline(&cover);

    // The coast reconciled with the DEM (local).
    let (slid, _) = coast::slide(&raw, &land, &cover, &spacing, slid_at);
    report.slid = count(window, |i, j| slid.get(i, j) != raw.get(i, j));
    drop((raw, cover));
    let (cut, _) = coast::cut(&slid, &land, &spacing, cut_at);
    report.lowered = count(window, |i, j| cut.get(i, j) != slid.get(i, j));
    drop(slid);
    let ground = cut.map(|h| h.max(LAND_MIN_M as f32));
    drop(cut);

    // The coast rounded (local): the mean and the old coast's distance read the held region about the rounding's.
    let mean = coast::mean(&outline, &spacing, cut_at);
    drop(outline);
    let old = distance::distance(&land, &spacing, OLD_COAST_M, cut_at);
    let land = land.crop(cut_at);
    // The second bake (given its sea floor) keeps the coastline's land through the rounding.
    let rounding = coast::rounded(&land, &ground, &old, &mean, cut_at, seabed.is_some());
    drop(old);
    report.rounded_up = count(window, |i, j| rounding.rounded.get(i, j) > land.get(i, j));
    report.rounded_down = count(window, |i, j| rounding.rounded.get(i, j) < land.get(i, j));

    // Islands kept, specks and lakes (components).
    let kept = components::keep_islands(&land, &rounding.rounded, &spacing, kept_at);
    report.islands_kept = count(window, |i, j| kept.get(i, j) > rounding.rounded.get(i, j));
    let specks = components::drop_specks(&kept, &spacing, specks_at);
    report.specks = count(window, |i, j| specks.get(i, j) < kept.get(i, j));
    drop(kept);
    let finished = components::fill_lakes(&specks, &spacing, lakes_at);
    report.filled = count(window, |i, j| finished.get(i, j) > specks.get(i, j));
    drop(specks);

    // The second bake's sea floor over the window: the bathymetry by place, and how far each point lies from open sea.
    let sea = match seabed {
        Some(s) => {
            let floor = s.floor(&window)?;
            if floor.region != window || floor.values.iter().any(|v| !v.is_finite()) {
                return Err(BakeError::Corrupt(format!(
                    "{key}'s sea floor is not one finite height a point of its window"
                )));
            }
            Some(coast::SeaFloor {
                floor,
                open: open_distance(key, window, s),
            })
        }
        None => None,
    };
    // The heights and the shelf, or the sea floor (local), quantised.
    let heights = coast::heights(
        &HeightInputs {
            coast: &finished,
            land: &land,
            rounded: &rounding.rounded,
            ground: &ground,
            level: &rounding.level,
            mean: &mean,
            sea: sea.as_ref(),
        },
        &spacing,
        window,
    );
    report.floor = sea
        .as_ref()
        .map_or(0, |_| count(window, |i, j| finished.get(i, j) == 0));
    let window_quanta = quantised(&heights);
    report.land = window_quanta.quanta.iter().filter(|&&q| q > 0).count();
    Ok(Finished {
        key,
        window: window_quanta,
        report,
    })
}

/// A window's heights as NTF1 quanta of 0.25 m (half to even, never 0).
fn quantised(heights: &Plane<f32>) -> Window {
    Window {
        quanta: heights
            .values
            .iter()
            .map(|&h| crate::ntf::quantise(f64::from(h)))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_halo_is_the_stages_and_widens_poleward() {
        let equator = Halo::of(SquareKey { lat: 0, lon: 0 });
        let sixty = Halo::of(SquareKey { lat: 60, lon: 0 });
        let limit = Halo::of(SquareKey { lat: 88, lon: 0 });
        let south = Halo::of(SquareKey { lat: -89, lon: 0 });
        assert_eq!(equator.rows(), sixty.rows());
        assert_eq!(limit, south);
        assert!(equator.columns() < sixty.columns() && sixty.columns() < limit.columns());
        // Rows: the coasts' distances (7.5 samples) and 2, the cut (6) and 1, three components (the cap, 15 rows) and 1
        // each, the shelf (1,312.5 m, 19 rows) and 1.
        assert_eq!(equator.rows(), 10 + 7 + 3 * 16 + 20);
        for key in [
            SquareKey { lat: 0, lon: 0 },
            SquareKey { lat: 88, lon: 179 },
        ] {
            let regions = Halo::of(key).regions(key);
            assert_eq!(regions.len(), STAGES.len() + 1);
            assert_eq!(*regions.last().unwrap(), Region::window(key));
        }
    }
}
