//! The synthetic release (step C, §7.3): a coastal world made by a deterministic generator from no source at all, at
//! its own place in the empty South Pacific about [`CENTRE`], for the fixtures of every later lane (late admissions,
//! route frontiers, the legacy seam, holds, saves and replays).
//!
//! **Its land** is a function of the lattice point alone. A point's place is laid on a flat local map in kilometres
//! about the centre (x east, y north; [`local_km`]); the land is a union of [`Shape`]s less the bay's water, read as a
//! signed distance `d` (negative on land); the height is `1 m + 25 m/km · (−d)` up to 600 m on land, and
//! `−(2.5 m + 120 m/km · d)` down to −160 m at sea, or a bank's floor where that is shallower ([`BANKS`]: `−(crest +
//! 20 m/km · e)`, `e` the distance off the bank's shape), quantized to 0.25 m (never 0). Only `+ − × ÷`, square roots
//! and comparisons, all correctly rounded, so every target makes the same quanta, and miniz's deflate makes the same
//! blocks.
//!
//! **Its features** ([`LAND`], [`WATER`]; places in local kilometres):
//! - Westland, a mainland, x from −260 to −140 km;
//! - Long Point, a peninsula 30 km wide running 300 km east from Westland's coast at x = −140 to its tip at x = 160,
//!   along the 30° S parallel: a route between its sides that stays within 100 km of them cannot round it;
//! - the Gate, a strait 2 km wide (x from −40 to −38) and 60 km long between West Gate and East Gate;
//! - the Stepping Stones, a chain of seven islands from (40, 70) to (220, 160);
//! - Deep Bay, cut into Westland's coast: its basin about (−170, −135) lies in S32W142, its mouth at (−140, −105)
//!   in S31W142;
//! - Ring Reef, an atoll whose closed lagoon (9 km across the radius) no water route reaches;
//! - Middle Bank, a shoal in open water south of Long Point (version 2), 10 m over its crest: a battleship's route goes
//!   round it, a destroyer's may cross it.
//!
//! **Its table and manifest**: a square is a candidate when a land shape's bounding box lies within 65 km of it and holds
//! data when within 40 km (the rest of its candidates `empty`); every other square is ocean or polar.
use std::collections::BTreeMap;

use crate::block::Window;
use crate::lattice::{BLOCK_SIDE, BLOCK_STEPS, BLOCKS, Point, QUANTUM_M, WINDOW, window_point};
use crate::release::{BakeSpec, FileRef, LatticeSpec, ReleaseFile, ReleaseId};
use crate::table::POLE_LIMIT_DEG;
use crate::{Entry, FinalSquare, Manifest, SquareKey, SquareTable};

/// The release's name.
pub const NAME: &str = "synthetic-v2";
/// The generator's version: a change to the world is a new version and a new release. Version 2 adds Middle Bank.
pub const VERSION: u32 = 2;
/// The world's centre, degrees: latitude, longitude.
pub const CENTRE: [f64; 2] = [-30.0, -140.0];
/// Kilometres a degree of latitude on the theatre's sphere (π · 6,371 km / 180), and of longitude on the local map.
pub const KM_PER_DEGREE: f64 = 111.194_926_644_558_73;
const COS_CENTRE: f64 = 0.866_025_403_784_438_6;
/// A candidate square holds data when land lies within this of it, metres (the bake's keep distance).
pub const KEEP_M: u32 = 40_000;
/// A square is a candidate when land lies within this of it, metres.
pub const CANDIDATE_M: u32 = 65_000;

/// A shape on the local map, kilometres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Every point within `r` of `c`.
    Disc { c: [f64; 2], r: f64 },
    /// Every point within `r` of the segment from `a` to `b`.
    Capsule { a: [f64; 2], b: [f64; 2], r: f64 },
    /// Every point whose distance from `c` is within `w` of `r`.
    Ring { c: [f64; 2], r: f64, w: f64 },
}

impl Shape {
    /// The signed distance from `p` to the shape's edge, negative inside.
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        let length = |x: f64, y: f64| (x * x + y * y).sqrt();
        match *self {
            Shape::Disc { c, r } => length(p[0] - c[0], p[1] - c[1]) - r,
            Shape::Capsule { a, b, r } => {
                let (px, py) = (p[0] - a[0], p[1] - a[1]);
                let (bx, by) = (b[0] - a[0], b[1] - a[1]);
                let h = ((px * bx + py * by) / (bx * bx + by * by)).clamp(0.0, 1.0);
                length(px - bx * h, py - by * h) - r
            }
            Shape::Ring { c, r, w } => (length(p[0] - c[0], p[1] - c[1]) - r).abs() - w,
        }
    }

    /// Its bounding box: [x0, y0, x1, y1].
    pub fn bounds(&self) -> [f64; 4] {
        match *self {
            Shape::Disc { c, r } => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
            Shape::Capsule { a, b, r } => [
                a[0].min(b[0]) - r,
                a[1].min(b[1]) - r,
                a[0].max(b[0]) + r,
                a[1].max(b[1]) + r,
            ],
            Shape::Ring { c, r, w } => [c[0] - r - w, c[1] - r - w, c[0] + r + w, c[1] + r + w],
        }
    }
}

const fn stone(t: usize, r: f64) -> Shape {
    Shape::Disc {
        c: [40.0 + 30.0 * t as f64, 70.0 + 15.0 * t as f64],
        r,
    }
}

/// The land, each shape with its name.
pub const LAND: &[(&str, Shape)] = &[
    (
        "Westland",
        Shape::Capsule {
            a: [-200.0, -150.0],
            b: [-200.0, 150.0],
            r: 60.0,
        },
    ),
    (
        "Long Point",
        Shape::Capsule {
            a: [-150.0, 0.0],
            b: [145.0, 0.0],
            r: 15.0,
        },
    ),
    (
        "West Gate",
        Shape::Capsule {
            a: [-60.0, 120.0],
            b: [-60.0, 180.0],
            r: 20.0,
        },
    ),
    (
        "East Gate",
        Shape::Capsule {
            a: [-18.0, 120.0],
            b: [-18.0, 180.0],
            r: 20.0,
        },
    ),
    ("Stepping Stone 1", stone(0, 6.0)),
    ("Stepping Stone 2", stone(1, 4.0)),
    ("Stepping Stone 3", stone(2, 8.0)),
    ("Stepping Stone 4", stone(3, 5.0)),
    ("Stepping Stone 5", stone(4, 7.0)),
    ("Stepping Stone 6", stone(5, 4.0)),
    ("Stepping Stone 7", stone(6, 9.0)),
    (
        "Ring Reef",
        Shape::Ring {
            c: [110.0, -150.0],
            r: 12.0,
            w: 3.0,
        },
    ),
];

/// The water cut from the land: Deep Bay's basin and the channel to its mouth.
pub const WATER: &[(&str, Shape)] = &[
    (
        "Deep Bay",
        Shape::Disc {
            c: [-170.0, -135.0],
            r: 15.0,
        },
    ),
    (
        "Deep Bay mouth",
        Shape::Capsule {
            a: [-170.0, -135.0],
            b: [-125.0, -90.0],
            r: 2.5,
        },
    ),
];

/// The banks: shoals in open water, each with its crest's depth below the sea (metres). Over its shape the floor is the
/// crest; off it the floor falls [`BANK_FALL_M_PER_KM`] a kilometre to the sea's. Never land, and never in a square's
/// class: the table and manifest are the land's alone.
pub const BANKS: &[(&str, Shape, f64)] = &[(
    "Middle Bank",
    Shape::Capsule {
        a: [30.0, -100.0],
        b: [70.0, -70.0],
        r: 2.0,
    },
    10.0,
)];
/// How fast a bank's floor falls off its shape, metres a kilometre.
const BANK_FALL_M_PER_KM: f64 = 20.0;

/// A place on the local map, kilometres: x east and y north of [`CENTRE`].
pub fn local_km(lat: f64, lon: f64) -> [f64; 2] {
    [
        (lon - CENTRE[1]) * (KM_PER_DEGREE * COS_CENTRE),
        (lat - CENTRE[0]) * KM_PER_DEGREE,
    ]
}

/// The place of a point on the local map: latitude, longitude.
pub fn place(km: [f64; 2]) -> [f64; 2] {
    [
        CENTRE[0] + km[1] / KM_PER_DEGREE,
        CENTRE[1] + km[0] / (KM_PER_DEGREE * COS_CENTRE),
    ]
}

/// Land within this of a block decides its heights: past it the height has reached the land's top or the sea floor.
const REACH_KM: f64 = 25.0;
const LAND_BASE_M: f64 = 1.0;
const LAND_RISE_M_PER_KM: f64 = 25.0;
const LAND_TOP_M: f64 = 600.0;
const SEA_EDGE_M: f64 = 2.5;
const SEA_FALL_M_PER_KM: f64 = 120.0;
const SEA_FLOOR_M: f64 = -160.0;

/// The signed distance to the coast at `p` (local km) over the given shapes: the land's union less the water.
fn distance(p: [f64; 2], land: &[Shape], water: &[Shape]) -> f64 {
    let l = land.iter().fold(f64::INFINITY, |d, s| d.min(s.distance(p)));
    let w = water
        .iter()
        .fold(f64::INFINITY, |d, s| d.min(s.distance(p)));
    l.max(-w)
}

/// The height at signed distance `d` (km) from the coast, metres.
fn height_at(d: f64) -> f64 {
    if d < 0.0 {
        (LAND_BASE_M - LAND_RISE_M_PER_KM * d).min(LAND_TOP_M)
    } else {
        (-(SEA_EDGE_M + SEA_FALL_M_PER_KM * d)).max(SEA_FLOOR_M)
    }
}

/// The height at `p` (local km) over the given land, water and banks: the coast's, and at sea a bank's floor where it
/// is shallower.
fn height_over(p: [f64; 2], land: &[Shape], water: &[Shape], banks: &[(Shape, f64)]) -> f64 {
    let d = distance(p, land, water);
    let h = height_at(d);
    if d < 0.0 {
        return h;
    }
    banks.iter().fold(h, |h, (shape, crest)| {
        h.max(-(crest + BANK_FALL_M_PER_KM * shape.distance(p).max(0.0)))
    })
}

fn banks() -> Vec<(Shape, f64)> {
    BANKS.iter().map(|(_, s, crest)| (*s, *crest)).collect()
}

fn quantize(h: f64) -> i16 {
    (h / QUANTUM_M).round_ties_even() as i16
}

fn all(shapes: &[(&str, Shape)]) -> Vec<Shape> {
    shapes.iter().map(|(_, s)| *s).collect()
}

/// The world's height at any place, metres, before quantization.
pub fn height_m(lat: f64, lon: f64) -> f64 {
    height_over(local_km(lat, lon), &all(LAND), &all(WATER), &banks())
}

/// A lattice point's quantum, every shape asked.
pub fn quantum(p: Point) -> i16 {
    quantize(height_over(
        local_km(p.lat(), p.lon()),
        &all(LAND),
        &all(WATER),
        &banks(),
    ))
}

/// Whether two boxes ([x0, y0, x1, y1]) are within `km` of each other.
fn boxes_within(a: [f64; 4], b: [f64; 4], km: f64) -> bool {
    let dx = (a[0] - b[2]).max(b[0] - a[2]).max(0.0);
    let dy = (a[1] - b[3]).max(b[1] - a[3]).max(0.0);
    dx * dx + dy * dy < km * km
}

/// A square's box on the local map.
fn square_box(key: SquareKey) -> [f64; 4] {
    let [x0, y0] = local_km(f64::from(key.lat), f64::from(key.lon));
    let [x1, y1] = local_km(f64::from(key.lat + 1), f64::from(key.lon + 1));
    [x0, y0, x1, y1]
}

fn land_within(key: SquareKey, m: u32) -> bool {
    let b = square_box(key);
    LAND.iter()
        .any(|(_, s)| boxes_within(s.bounds(), b, f64::from(m) / 1000.0))
}

/// The release's square table.
pub fn table() -> SquareTable {
    SquareTable::new(|key| land_within(key, CANDIDATE_M))
}

/// What the manifest records for a candidate square: data when land lies within the keep distance, else empty.
pub fn holds_data(key: SquareKey) -> bool {
    land_within(key, KEEP_M)
}

/// Square `key`'s final window. Each block asks only the shapes (and banks) within [`REACH_KM`] of it, which gives the
/// quanta of [`quantum`] exactly: a shape farther than that from every point changes no height. Each row's y and each column's
/// x are taken once, so a polar square (whose pole row is one place) is refused: no release bakes one.
pub fn window(key: SquareKey) -> Window {
    assert!(!crate::table::polar(key), "{key} is polar: never baked");
    let (land, water, banks) = (all(LAND), all(WATER), banks());
    let mut quanta = vec![0i16; WINDOW * WINDOW];
    for br in 0..BLOCKS {
        for bc in 0..BLOCKS {
            let (r0, c0) = (br * BLOCK_STEPS, bc * BLOCK_STEPS);
            // Each row's y and each column's x, from the lattice point alone.
            let ys: Vec<f64> = (0..BLOCK_SIDE)
                .map(|r| local_km(window_point(key, r0 + r, c0).lat(), CENTRE[1])[1])
                .collect();
            let xs: Vec<f64> = (0..BLOCK_SIDE)
                .map(|c| local_km(CENTRE[0], window_point(key, r0, c0 + c).lon())[0])
                .collect();
            let fold = |v: &[f64]| {
                v.iter()
                    .fold([f64::INFINITY, f64::NEG_INFINITY], |[lo, hi], &x| {
                        [lo.min(x), hi.max(x)]
                    })
            };
            let ([x0, x1], [y0, y1]) = (fold(&xs), fold(&ys));
            let near = |shapes: &[Shape]| -> Vec<Shape> {
                shapes
                    .iter()
                    .filter(|s| boxes_within(s.bounds(), [x0, y0, x1, y1], REACH_KM))
                    .copied()
                    .collect()
            };
            let (land, water) = (near(&land), near(&water));
            let banks: Vec<(Shape, f64)> = banks
                .iter()
                .filter(|(s, _)| boxes_within(s.bounds(), [x0, y0, x1, y1], REACH_KM))
                .copied()
                .collect();
            for r in 0..BLOCK_SIDE {
                for c in 0..BLOCK_SIDE {
                    let q = if land.is_empty() && banks.is_empty() {
                        quantize(SEA_FLOOR_M)
                    } else {
                        quantize(height_over([xs[c], ys[r]], &land, &water, &banks))
                    };
                    quanta[(r0 + r) * WINDOW + c0 + c] = q;
                }
            }
        }
    }
    Window { quanta }
}

/// Square `key`'s final square file, when the release holds data for it.
pub fn square(key: SquareKey) -> Option<Vec<u8>> {
    (table().class(key) == crate::Class::Candidate && holds_data(key))
        .then(|| FinalSquare::encode(key, &window(key), Default::default()))
}

/// The release file for a square table's file.
pub fn release_file(table: &[u8]) -> Vec<u8> {
    ReleaseFile {
        format: crate::release::FORMAT,
        name: NAME.into(),
        lattice: LatticeSpec::current(),
        bake: BakeSpec {
            algorithm: "synthetic".into(),
            version: VERSION,
            keep_m: KEEP_M,
            candidate_m: CANDIDATE_M,
            pole_limit_deg: POLE_LIMIT_DEG,
        },
        snapshot: None,
        sources: None,
        square_table: FileRef::of(table),
        synthetic: true,
    }
    .encode()
}

/// The whole release, as a release is published and baked: its file, its square table, its manifest (every candidate
/// square in key order) and every data square's bytes.
#[derive(Clone, Debug, PartialEq)]
pub struct Synthetic {
    pub id: ReleaseId,
    pub release: Vec<u8>,
    pub table: Vec<u8>,
    pub manifest: String,
    pub squares: BTreeMap<SquareKey, Vec<u8>>,
}

/// Makes the release. Twice gives the same bytes.
pub fn release() -> Synthetic {
    let table = table();
    let table_bytes = table.encode();
    let release = release_file(&table_bytes);
    let mut manifest = Manifest::default();
    let mut squares = BTreeMap::new();
    for key in table.candidates() {
        let entry = if holds_data(key) {
            let bytes = FinalSquare::encode(key, &window(key), Default::default());
            let entry = Entry::Data(crate::Digest::of(&bytes));
            squares.insert(key, bytes);
            entry
        } else {
            Entry::Empty
        };
        manifest
            .record(key, entry)
            .expect("each candidate is recorded once");
    }
    Synthetic {
        id: ReleaseId::of(&release),
        release,
        table: table_bytes,
        manifest: manifest.text(),
        squares,
    }
}
