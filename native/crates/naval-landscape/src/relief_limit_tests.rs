use super::*;
use crate::relief::{Coarse, Lattice};
use crate::relief_combined::{self, Base};
use crate::relief_glo30::EXTRA;
use crate::relief_synth::{self, Params};

const N: usize = PAGE_TEXELS as usize;
const G: usize = GUTTER as usize;

const ALPS: LatLon = LatLon {
    lat: 46.58,
    lon: 8.0,
};

/// A lattice whose samples stand on texel middles.
const ON_TEXELS: Lattice = Lattice {
    node: [2.0, -38.0],
    cell: 40.0,
};

const INDEX: [i64; 2] = [3, -2];

/// Steep mountains: ridges 1.4 km apart rising 800 m either side of 1,500 m, `low` m lower.
fn mountains(p: [f64; 2], low: f64) -> f32 {
    (1500.0 - low + 800.0 * (p[0] / 700.0).sin() * (p[1] / 900.0).cos()) as f32
}

/// A window of `height` over 8 km about pages [`INDEX`] and the next east, its samples on [`ON_TEXELS`].
fn coarse(height: &dyn Fn([f64; 2]) -> f32) -> Coarse {
    let (n, origin) = (200u32, [4002.0f32, -6038.0f32]);
    let quanta = (0..n * n)
        .map(|k| {
            let p = [
                f64::from(origin[0]) + f64::from(k % n) * 40.0,
                f64::from(origin[1]) + f64::from(k / n) * 40.0,
            ];
            (height(p) * 10.0).round() as i16
        })
        .collect();
    let field = naval_terrain::Heightfield {
        columns: n,
        rows: n,
        cell: 40.0,
        origin_x: origin[0],
        origin_z: origin[1],
        step: 0.1,
        quanta,
    };
    Coarse {
        field: std::sync::Arc::new(naval_terrain::field::PreparedField::new(&field)),
        origin: [0.0, 0.0],
    }
}

/// Page `index`'s base of weight 1, its band `band` at a theatre point.
fn base(index: [i64; 2], band: &dyn Fn([f64; 2]) -> f32) -> Base {
    let e = N + 2 * EXTRA;
    let b = (0..e * e)
        .map(|k| band(texel_at(index, EXTRA, k % e, k / e)))
        .collect();
    Base::from_grids(index, vec![1.0; e * e], b, vec![false; e * e])
}

/// GLO-30's band as the Alps' crests have it: ridges 30 m over the window, 300-400 m apart.
fn crests(p: [f64; 2]) -> f32 {
    (30.0 * ((p[0] / 55.0).sin() * (p[1] / 70.0).cos()).powi(2) - 8.0) as f32
}

fn largest(v: &[f32]) -> f32 {
    v.iter().fold(f32::MIN, |m, &x| m.max(x))
}

#[test]
fn a_held_page_stands_at_most_the_allowance_above_the_window_at_every_texel() {
    let params = Params::default();
    let window = coarse(&|p| mountains(p, 0.0));
    let dry = Water::none(ALPS, INDEX);
    // The synthetic alone, and over a band raising the crests 22 m.
    let banded = base(INDEX, &crests);
    for (name, b) in [("synthetic", None), ("combined", Some(&banded))] {
        let raw = relief_combined::residuals(ALPS, &ON_TEXELS, INDEX, b, &window, &params);
        let held = relief_combined::held(ALPS, &ON_TEXELS, INDEX, b, &window, &params, &dry);
        let (over, top) = (largest(&raw), largest(&held));
        assert!(top <= RAISE_M + 1e-3, "{name}: {top} m above the window");
        // It still carves, and the land it held is lowered no more than it stood over.
        let deepest = held.iter().fold(0f32, |m, &x| m.min(x));
        assert!(deepest < -2.0, "{name}: carves only {deepest} m");
        let lowered = raw
            .iter()
            .zip(&held)
            .map(|(r, h)| r - h)
            .fold(0f32, f32::max);
        assert!(
            lowered <= over - RAISE_M + KNEE_M + 1e-3,
            "{name}: lowered {lowered} m"
        );
        if b.is_some() {
            assert!(over > 15.0, "{name}: the band raises only {over} m");
        }
    }
}

#[test]
fn a_raised_crest_is_lowered_whole_its_fine_shape_kept() {
    let s = side(MARGIN);
    // A ridge 30 m over the window along the grid's middle, 1 m ripples 16 m apart over it.
    let at = |i: usize, j: usize| {
        let x = (i as f32 - s as f32 / 2.0) * TEXEL_M as f32;
        let ripple = (std::f32::consts::TAU * (i + j) as f32 / 4.0).sin();
        30.0 * (-(x / 60.0).powi(2)).exp() + ripple
    };
    let grid: Vec<f32> = (0..s * s).map(|k| at(k % s, k / s)).collect();
    let held = hold(&grid, &vec![1.0; s * s]);
    assert!(largest(&held) <= RAISE_M + 1e-3);
    // On the crest's top the ripples stand as they were: no flat cut.
    let (mut sum, mut count) = (0f64, 0usize);
    for j in 8..N - 8 {
        for i in 1..N - 1 {
            if grid[(j + MARGIN) * s + i + MARGIN] < 22.0 {
                continue;
            }
            let k = j * N + i;
            let d = held[k] - 0.5 * (held[k - 1] + held[k + 1]);
            sum += f64::from(d * d);
            count += 1;
        }
    }
    let rms = (sum / count.max(1) as f64).sqrt();
    // A ripple's own (a sine 4 texels long: its second difference over two is its height, RMS 0.71).
    assert!(
        count > 1000 && rms > 0.5,
        "the ripples on the crest: {rms} m over {count} texels"
    );
}

#[test]
fn a_narrow_lip_is_eased_and_the_land_about_it_left() {
    let s = side(MARGIN);
    // A lip 12 m wide standing 30 m over the window, a slope 0.5 m a texel under it.
    let at = |i: usize, _j: usize| {
        let x = (i as f32 - s as f32 / 2.0) * TEXEL_M as f32;
        30.0 * (-(x / 6.0).powi(2)).exp() + 0.5 * (x / TEXEL_M as f32).sin()
    };
    let grid: Vec<f32> = (0..s * s).map(|k| at(k % s, k / s)).collect();
    let held = hold(&grid, &vec![1.0; s * s]);
    assert!(largest(&held) <= RAISE_M);
    // Past 24 m from it the land is as made: no trench dug along it.
    let j = N / 2;
    for i in 0..N {
        let x = (i as f32 + MARGIN as f32 - s as f32 / 2.0) * TEXEL_M as f32;
        let raw = grid[(j + MARGIN) * s + i + MARGIN];
        if x.abs() > 24.0 {
            assert!(
                (held[j * N + i] - raw).abs() < 0.05,
                "{x} m off: {} for {raw}",
                held[j * N + i]
            );
        }
    }
}

#[test]
fn two_held_pages_agree_where_they_meet() {
    let params = Params::default();
    let window = coarse(&|p| mountains(p, 0.0));
    let east = [INDEX[0] + 1, INDEX[1]];
    let page = |index: [i64; 2]| {
        let b = base(index, &crests);
        relief_combined::held(
            ALPS,
            &ON_TEXELS,
            index,
            Some(&b),
            &window,
            &params,
            &Water::none(ALPS, index),
        )
    };
    let (a, b) = (page(INDEX), page(east));
    let own = N - 2 * G;
    let mut worst = 0f32;
    for j in 0..N {
        for i in own..N {
            worst = worst.max((a[j * N + i] - b[j * N + i - own]).abs());
        }
    }
    assert!(
        worst < 1e-3,
        "the pages differ by {worst} m where they meet"
    );
}

#[test]
fn no_made_detail_on_the_water_and_little_beside_it() {
    let params = Params::default();
    let window = coarse(&|p| mountains(p, 0.0));
    let dry = Water::none(ALPS, INDEX);
    let corner = [
        INDEX[0] as f64 * relief::page_m(),
        INDEX[1] as f64 * relief::page_m(),
    ];
    // A river across the page, a lake in its south-east.
    let mut wet = Water::none(ALPS, INDEX);
    wet.lines.push((
        vec![
            [corner[0] - 100.0, corner[1] + 600.0],
            [corner[0] + 2200.0, corner[1] + 900.0],
        ],
        8.0,
    ));
    let lake = [
        [1400.0, 1400.0],
        [1800.0, 1400.0],
        [1800.0, 1800.0],
        [1400.0, 1800.0],
    ]
    .map(|m| [corner[0] + m[0], corner[1] + m[1]]);
    wet.areas.push(vec![lake.to_vec()]);
    let a = relief_combined::held(ALPS, &ON_TEXELS, INDEX, None, &window, &params, &dry);
    let b = relief_combined::held(ALPS, &ON_TEXELS, INDEX, None, &window, &params, &wet);
    let river = |p: [f64; 2]| {
        let (x, z) = (p[0] - corner[0] + 100.0, p[1] - corner[1] - 600.0);
        (z - x * 300.0 / 2300.0).abs() * (2300.0f64 / 2300.0f64.hypot(300.0))
    };
    let (mut on, mut near, mut far_same) = (0f32, 0f32, true);
    let mut dry_near = 0f32;
    for j in G..N - G {
        for i in G..N - G {
            let p = texel_at(INDEX, 0, i, j);
            let m = [p[0] - corner[0], p[1] - corner[1]];
            let in_lake = (1400.0..1800.0).contains(&m[0]) && (1400.0..1800.0).contains(&m[1]);
            let d = river(p);
            let k = j * N + i;
            if d < 7.0 || in_lake {
                on = on.max(b[k].abs());
            } else if d < 20.0 {
                near = near.max(b[k].abs());
                dry_near = dry_near.max(a[k].abs());
            } else if d > 120.0 && !(1200.0..2000.0).contains(&m[0]) {
                far_same &= (a[k] - b[k]).abs() < 1e-4;
            }
        }
    }
    assert!(on == 0.0, "{on} m of made detail on the water");
    assert!(
        dry_near > 1.0 && near < 0.3 * dry_near,
        "beside the river {near} m against {dry_near} m"
    );
    assert!(far_same, "the water changes the land far from it");
}

#[test]
fn near_the_sea_the_made_detail_is_scaled_down() {
    let dry = Water::none(ALPS, INDEX);
    for (h, share) in [
        (-5.0, 0.0),
        (0.0, 0.0),
        (25.0, 0.5),
        (50.0, 1.0),
        (300.0, 1.0),
    ] {
        let w = weights(INDEX, &dry, &|_| h);
        assert!(
            w.iter().all(|&v| (v - share).abs() < 1e-6),
            "{h} m up: {} not {share}",
            w[0]
        );
    }
}

#[test]
#[ignore = "prints the hold's parts' times"]
fn times_of_the_parts() {
    use std::time::Instant;
    let params = Params::default();
    let window = coarse(&|p| mountains(p, 0.0));
    let height = |p: [f64; 2]| window.height(p);
    let t = Instant::now();
    let style = relief_synth::Style::of(ALPS, INDEX, &params);
    let a = t.elapsed();
    let t = Instant::now();
    let plain = relief_synth::residuals_on(&ON_TEXELS, INDEX, &height, &params, &style, None, 0);
    let b = t.elapsed();
    let t = Instant::now();
    let grid =
        relief_synth::residuals_on(&ON_TEXELS, INDEX, &height, &params, &style, None, MARGIN);
    let c = t.elapsed();
    let mut wet = Water::none(ALPS, INDEX);
    let corner = [
        INDEX[0] as f64 * relief::page_m(),
        INDEX[1] as f64 * relief::page_m(),
    ];
    wet.lines.push((
        vec![
            [corner[0], corner[1] + 600.0],
            [corner[0] + 2000.0, corner[1] + 900.0],
        ],
        8.0,
    ));
    wet.lines.push((
        vec![
            [corner[0], corner[1] + 1600.0],
            [corner[0] + 2000.0, corner[1] + 1200.0],
        ],
        1.5,
    ));
    let t = Instant::now();
    let w = weights(INDEX, &wet, &height);
    let d = t.elapsed();
    let t = Instant::now();
    let held = hold(&grid, &w);
    let e = t.elapsed();
    eprintln!(
        "style {a:?} page {b:?} wide page {c:?} weights {d:?} hold {e:?} ({} {})",
        plain.len(),
        held.len()
    );
}
