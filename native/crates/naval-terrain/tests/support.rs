//! Synthetic land for this crate's tests and examples, in place of the battle maps' baked heightfields (removed on
//! 2026-10-27): one chart built in code, the same on every machine (no file; only sums, products, quotients, floors
//! and roundings).
//!
//! The chart (x east, z south, metres) is 24 km square at 40 m, the battle's lane at its origin on open sea, and a coast
//! running north and south about [`COAST_X`] east of the lane, the sea shelving to 50 m within 1.8 km of it (500 m
//! under the cliffs):
//! - north of z = −3 km, cliffs [`CLIFF_TOP_M`] tall straight out of the sea, the downs behind them, and a ravine cut
//!   into the downs ([`RAVINE_Z`]) whose walls shut out most of the sky;
//! - between z = −1.5 and 4 km, a beach and a lowland, then a wooded scarp ([`SCARP`]) rising 290 m with slopes to
//!   0.27, to rolling downs: the coast that stands highest over the sea from the lane (`review::main_coast`);
//! - south of z = 5 km, the lowland rising slowly inland, flat enough for a town, and a hill ([`HILL`]) that shades its
//!   own far side in a low sun;
//! - over the last 2 km to the chart's north, east and south edges, the land going gently down into the sea.
//!
//! A test chooses the land's look by its style on the North Atlantic's scene ([`scene`], [`styled`]).

#![allow(dead_code)]

use std::sync::OnceLock;

use naval_conditions::{Climate, LandStyle, MapScene, MapSelection, SurfaceStyle};
use naval_terrain::Heightfield;

/// Metres between samples, samples a side, the chart position of sample (0, 0) and metres per quantum.
pub const CELL: f32 = 40.0;
pub const SAMPLES: u32 = 601;
pub const ORIGIN: [f32; 2] = [-8000.0, -12000.0];
pub const STEP: f32 = 0.25;
/// The coastline's mean x (m); it wanders 150 m either side.
pub const COAST_X: f64 = 3000.0;
/// The cliffs' height at their top (m).
pub const CLIFF_TOP_M: f64 = 180.0;
/// The ravine's line (chart z, m): on a row the open sky test reads (every 14th sample from the chart's corner).
pub const RAVINE_Z: f64 = -4160.0;
/// The middle of the wooded scarp, chart x and z (m).
pub const SCARP: [f64; 2] = [5300.0, 1200.0];
/// The hill's centre (chart x, z), its radius and its height over the lowland (m).
pub const HILL: [f64; 2] = [11500.0, 8500.0];
pub const HILL_RADIUS_M: f64 = 2200.0;
pub const HILL_TOP_M: f64 = 380.0;
/// The rolling downs behind the scarp's crest (chart x, z), the land falling away to the sea west of them.
pub const DOWNS: [f64; 2] = [7200.0, 1200.0];
/// A point of the south's lowland (chart x, z), 15–25 m up and flat for kilometres.
pub const LOWLAND: [f64; 2] = [7000.0, 6200.0];

fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// 0 below `a`, 1 above `b`, smooth between.
fn ramp(a: f64, b: f64, v: f64) -> f64 {
    smooth((v - a) / (b - a))
}

/// A value in −1..1, fixed per lattice point and salt (splitmix64's finaliser).
fn hash(i: i64, j: i64, salt: u64) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (j as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ salt.wrapping_mul(0x1656_67b1_9e37_79f9);
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    (h >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

/// Smooth value noise in −1..1 with features `scale` metres across.
fn noise(x: f64, z: f64, scale: f64, salt: u64) -> f64 {
    let (u, v) = (x / scale, z / scale);
    let (i, j) = (u.floor(), v.floor());
    let (a, b) = (smooth(u - i), smooth(v - j));
    let (i, j) = (i as i64, j as i64);
    let top = hash(i, j, salt) * (1.0 - a) + hash(i + 1, j, salt) * a;
    let bottom = hash(i, j + 1, salt) * (1.0 - a) + hash(i + 1, j + 1, salt) * a;
    top * (1.0 - b) + bottom * b
}

/// Rolling ground: two octaves of [`noise`], about −1..1.
fn rolling(x: f64, z: f64, scale: f64, salt: u64) -> f64 {
    noise(x, z, scale, salt) * 0.7 + noise(x, z, scale / 2.3, salt + 1) * 0.3
}

/// The land's height at chart (x, z), metres, before it is quantised.
pub fn height(x: f64, z: f64) -> f64 {
    let coast = COAST_X + 150.0 * noise(0.0, z, 1700.0, 1);
    // Metres inland of the coastline.
    let s = x - coast;
    let north = 1.0 - ramp(-3500.0, -2500.0, z);
    if s <= 0.0 {
        // The sea floor shelves to 50 m within 1.8 km, within 500 m under the cliffs.
        let t = (-s / (1800.0 - 1300.0 * north)).min(1.0);
        return -50.0 * (1.0 - (1.0 - t) * (1.0 - t));
    }
    // The lowland: a beach, flats rising 1 in 170 to 1.5 km inland and 1 in 330 beyond, rolling a metre or two.
    let lowland = 2.0 * ramp(0.0, 60.0, s)
        + 0.006 * (s.min(1500.0) - 60.0).max(0.0)
        + 0.003 * (s - 1500.0).max(0.0)
        + 2.0 * rolling(x, z, 700.0, 2) * ramp(100.0, 500.0, s);
    // The scarp: 290 m over 1.6 km from 1.5 km inland, then downs falling 1 in 250 and rolling 15 m.
    let scarp = 290.0 * ramp(1500.0, 3100.0, s) - 0.004 * (s - 3100.0).max(0.0)
        + 15.0 * rolling(x, z, 1100.0, 3) * ramp(2600.0, 3600.0, s);
    // The cliffs: the downs over them brought down to the sea within 220 m.
    let downs = CLIFF_TOP_M + 0.003 * s + 10.0 * rolling(x, z, 1300.0, 4) * ramp(400.0, 1200.0, s);
    let mut cliffs = downs * ramp(0.0, 220.0, s);
    // The ravine, from 600 m to 3.6 km inland: its floor 30 m up rising 1 in 100, its walls 120 m off its line.
    let across = ((z - RAVINE_Z) / 120.0).abs();
    if across < 1.0 {
        let depth = (1.0 - across * across).powi(2)
            * ramp(500.0, 800.0, s)
            * (1.0 - ramp(3200.0, 3800.0, s));
        let floor = 30.0 + 0.01 * (s - 600.0).max(0.0);
        cliffs = cliffs * (1.0 - depth) + floor * depth;
    }
    let middle = ramp(-2500.0, -1500.0, z) * (1.0 - ramp(4000.0, 5000.0, z));
    let mut h = (lowland + middle * scarp) * (1.0 - north) + cliffs * north;
    let r = ((x - HILL[0]) * (x - HILL[0]) + (z - HILL[1]) * (z - HILL[1])).sqrt() / HILL_RADIUS_M;
    if r < 1.0 {
        h += HILL_TOP_M * smooth(1.0 - r);
    }
    // The land goes down into the sea over the last 2 km to the chart's other edges: no wall where the chart ends.
    let edge = (f64::from(ORIGIN[0]) + f64::from(SAMPLES - 1) * f64::from(CELL) - x)
        .min(z - f64::from(ORIGIN[1]))
        .min(f64::from(ORIGIN[1]) + f64::from(SAMPLES - 1) * f64::from(CELL) - z);
    if h > -20.0 {
        h = -20.0 + (h + 20.0) * ramp(0.0, 2000.0, edge);
    }
    h
}

/// The land as a heightfield: [`height`] at every sample, in quanta of [`STEP`], none on the waterline.
pub fn land() -> Heightfield {
    static LAND: OnceLock<Heightfield> = OnceLock::new();
    LAND.get_or_init(|| {
        let mut quanta = Vec::with_capacity((SAMPLES * SAMPLES) as usize);
        for j in 0..SAMPLES {
            for i in 0..SAMPLES {
                let h = height(
                    f64::from(ORIGIN[0]) + f64::from(i) * f64::from(CELL),
                    f64::from(ORIGIN[1]) + f64::from(j) * f64::from(CELL),
                );
                let q = (h / f64::from(STEP)).round().clamp(-32768.0, 32767.0) as i16;
                quanta.push(match q {
                    0 if h < 0.0 => -1,
                    0 => 1,
                    q => q,
                });
            }
        }
        Heightfield {
            columns: SAMPLES,
            rows: SAMPLES,
            cell: CELL,
            origin_x: ORIGIN[0],
            origin_z: ORIGIN[1],
            step: STEP,
            quanta,
        }
    })
    .clone()
}

/// The palettes the removed land maps gave each style (sRGB hex: shore, low, high): the tropical style's Kawah
/// Lagoon's, the snow style's Norway's, the volcanic style's the Sunda Strait's, the chalk style's the Strait of Dover's
/// and the rock style's the North Atlantic's own.
fn palette(style: LandStyle) -> [&'static str; 3] {
    match style {
        LandStyle::Tropical => ["#5a554c", "#33472c", "#6f5b48"],
        LandStyle::Snow => ["#3b3935", "#3f3a34", "#5b5d5c"],
        LandStyle::Volcanic => ["#4b4843", "#3b5230", "#5d564c"],
        LandStyle::Chalk => ["#7f786c", "#6d7a4b", "#d6d2c4"],
        LandStyle::Rock => ["#a39b86", "#657151", "#7d8281"],
    }
}

/// `scene` with its land drawn in `style`, as the style was authored (the seasonal styles' equator due south, the
/// North Atlantic's chart's top bearing north).
pub fn styled(mut scene: MapScene, style: LandStyle) -> MapScene {
    let [shore, low, high] = palette(style);
    let bearing = scene.map.bearing.to_radians();
    scene.style = SurfaceStyle {
        style,
        shore: naval_conditions::srgb_hex(shore),
        low: naval_conditions::srgb_hex(low),
        high: naval_conditions::srgb_hex(high),
        equator: if matches!(style, LandStyle::Snow | LandStyle::Chalk) {
            [bearing.sin() as f32, bearing.cos() as f32]
        } else {
            [0.0, 0.0]
        },
        climate: Climate::AUTHORED,
    };
    scene
}

/// A battle on the North Atlantic at its own time and weather, its land drawn in `style`.
pub fn scene(style: LandStyle) -> MapScene {
    styled(
        MapSelection::battle(naval_conditions::DEFAULT_MAP)
            .resolve()
            .unwrap(),
        style,
    )
}

/// The world point over chart point `chart` at height `y` in `scene`'s battle (world = chart + offset).
pub fn world(scene: &MapScene, chart: [f64; 2], y: f64) -> glam::Vec3 {
    glam::Vec3::new(
        (chart[0] + scene.offset[0]) as f32,
        y as f32,
        (chart[1] + scene.offset[1]) as f32,
    )
}

/// Whether texel (`x`, `z`) of the attribute grid of `field` lies within a kilometre of [`DOWNS`] in x and 1.5 km in z.
pub fn on_the_downs(field: &naval_terrain::field::PreparedField, x: u32, z: u32) -> bool {
    let chart = [
        f64::from(field.origin[0]) + f64::from(x) * field.coarse.cell,
        f64::from(field.origin[1]) + f64::from(z) * field.coarse.cell,
    ];
    (chart[0] - DOWNS[0]).abs() < 1000.0 && (chart[1] - DOWNS[1]).abs() < 1500.0
}

/// An eye `up` metres over the land `up` metres south of the scarp's middle, looking down at it at 45°, in `scene`'s
/// world.
pub fn over_the_scarp(scene: &MapScene, up: f64) -> glam::Vec3 {
    world(scene, [SCARP[0], SCARP[1] + up], up)
}

/// The test land holds what the tests look at: the coast the review's views find highest from the lane is the scarp's,
/// its steepest is the cliffs', the ravine shuts out most of the sky, and the hill and the lowland stand where they
/// are said to.
#[test]
fn the_test_land_has_its_cliffs_scarp_ravine_hill_and_lowland() {
    use naval_terrain::review;
    let field = land();
    assert_eq!(field, land(), "built the same every time");
    let coast = review::main_coast(&field);
    let hit = [
        coast.direction[0] * coast.hit,
        coast.direction[1] * coast.hit,
    ];
    assert!(
        (-1500.0..4000.0).contains(&hit[1]) && (COAST_X - 400.0..COAST_X + 400.0).contains(&hit[0]),
        "the main coast is the lowland's under the scarp: {coast:?}"
    );
    let steep = review::steepest_coast(&field);
    assert!(
        steep.z < -3500.0 && steep.dx < -0.9 && steep.height > 100.0,
        "the steepest coast is the cliffs': {steep:?}"
    );
    let prepared = naval_terrain::field::PreparedField::new(&field);
    let input = naval_terrain::open_sky::BakeInput {
        grid: prepared.grid,
        heights: &prepared.heights,
        gradients: &prepared.gradients,
    };
    let row = ((RAVINE_Z - f64::from(ORIGIN[1])) / f64::from(CELL)) as u32;
    assert_eq!(row % 14, 0, "on a row the open sky test reads");
    let darkest = (0..SAMPLES)
        .step_by(14)
        .map(|i| naval_terrain::open_sky::open_sky(&input, i, row))
        .fold(1.0f32, f32::min);
    assert!(darkest < 0.45, "the ravine shuts out the sky: {darkest}");
    let top = field.height(HILL[0], HILL[1]);
    assert!(
        (HILL_TOP_M..HILL_TOP_M + 60.0).contains(&top),
        "the hill: {top} m"
    );
    let low = field.height(LOWLAND[0], LOWLAND[1]);
    assert!((15.0..25.0).contains(&low), "the lowland: {low} m");
    assert!(field.height(0.0, 0.0) < -40.0, "the lane on open sea");
}
