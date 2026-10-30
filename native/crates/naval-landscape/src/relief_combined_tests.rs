use super::*;
use crate::relief::{Mode, Source, page_source_in};

const N: usize = PAGE_TEXELS as usize;
const E: usize = N + 2 * EXTRA;

/// Steep mountains: ridges 1.4 km apart rising 800 m either side of 1,500 m (slopes to 48°).
fn mountains(p: [f64; 2]) -> f32 {
    (1500.0 + 800.0 * (p[0] / 700.0).sin() * (p[1] / 900.0).cos()) as f32
}

const ALPS: LatLon = LatLon {
    lat: 46.58,
    lon: 8.0,
};

/// A lattice whose samples stand on texel middles (theatre x and z 2 m past a multiple of 40).
const ON_TEXELS: Lattice = Lattice {
    node: [2.0, -38.0],
    cell: 40.0,
};

const INDEX: [i64; 2] = [3, -2];

/// A window of the mountains about page [`INDEX`], its samples on [`ON_TEXELS`], its chart's (0, 0) at the theatre's.
fn coarse() -> Coarse {
    let (n, origin) = (120u32, [4962.0f32, -5238.0f32]);
    let quanta = (0..n * n)
        .map(|k| {
            let p = [
                f64::from(origin[0]) + f64::from(k % n) * 40.0,
                f64::from(origin[1]) + f64::from(k / n) * 40.0,
            ];
            (mountains(p) * 10.0).round() as i16
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

/// The page's texel (i, j) (with its gutters) as a theatre point.
fn texel_point(index: [i64; 2], i: usize, j: usize) -> [f64; 2] {
    let side = relief::page_m();
    let g = f64::from(GUTTER);
    [
        index[0] as f64 * side + (i as f64 - g + 0.5) * TEXEL_M,
        index[1] as f64 * side + (j as f64 - g + 0.5) * TEXEL_M,
    ]
}

/// A base of `weight` throughout, its band `band` at a theatre point.
fn base(weight: f32, band: &dyn Fn([f64; 2]) -> f32) -> Base {
    let side = relief::page_m();
    let g = f64::from(GUTTER) + EXTRA as f64;
    let at = |i: usize, j: usize| {
        [
            INDEX[0] as f64 * side + (i as f64 - g + 0.5) * TEXEL_M,
            INDEX[1] as f64 * side + (j as f64 - g + 0.5) * TEXEL_M,
        ]
    };
    let b = (0..E * E).map(|k| band(at(k % E, k / E))).collect();
    Base::from_grids(INDEX, vec![weight; E * E], b, vec![false; E * E])
}

#[test]
fn the_cap_passes_small_bands_and_holds_large_ones_within_it() {
    assert_eq!(capped(12.0, 0.0), 12.0);
    assert!((capped(5.0, 35.0) - 5.0).abs() < 0.01);
    assert!((capped(-10.0, 35.0) + 10.0).abs() < 0.1);
    let mut last = 0.0;
    for k in 1..400 {
        let b = k as f32 * 0.5;
        let c = capped(b, 35.0);
        assert!(c > last && c < 35.0, "{b}: {c}");
        assert_eq!(capped(-b, 35.0), -c);
        last = c;
    }
}

#[test]
fn the_combined_source_is_chosen_by_name() {
    assert_eq!(Mode::parse("combined"), Some(Mode::Combined));
    assert_eq!(
        page_source_in(Mode::Combined, ALPS, [0, 0]),
        Some(Source::Combined)
    );
    assert!(Source::Combined.synthetic() && !Source::Combined.real());
    assert!(Source::Terrarium.real() && Source::Gsi.real());
    assert!(!Source::Synthetic.real() && !Source::Glo30.real());
}

#[test]
fn without_glo30_a_page_is_the_synthetic() {
    let params = Params::default();
    let coarse = coarse();
    let plain = residuals(ALPS, &ON_TEXELS, INDEX, None, &coarse, &params);
    let height = |p: [f64; 2]| coarse.height(p);
    let style = relief_synth::Style::of(ALPS, INDEX, &params);
    let synth = relief_synth::residuals_on(&ON_TEXELS, INDEX, &height, &params, &style, None, 0);
    assert!(
        plain == synth,
        "the combined source without GLO-30 is not the synthetic"
    );
    // A base masked throughout is none.
    let masked = base(0.0, &|p| 20.0 * (p[0] / 90.0).sin() as f32);
    let none = residuals(ALPS, &ON_TEXELS, INDEX, Some(&masked), &coarse, &params);
    assert!(none == synth, "a base of no weight changes the page");
}

#[test]
fn over_glo30_the_band_moves_the_samples_and_the_detail_is_only_fine() {
    let params = Params::default();
    let coarse = coarse();
    let wave = |p: [f64; 2]| 12.0 * (p[0] / 50.0).sin() as f32 * (p[1] / 70.0).cos() as f32;
    let full = base(1.0, &wave);
    let r = residuals(ALPS, &ON_TEXELS, INDEX, Some(&full), &coarse, &params);
    // The detail over the band (the residual less it): zero at the samples, so the samples move by the band alone.
    let mut at_samples = 0f32;
    let mut detail_peak = 0f32;
    for j in GUTTER as usize..N - GUTTER as usize {
        for i in GUTTER as usize..N - GUTTER as usize {
            let p = texel_point(INDEX, i, j);
            let d = r[j * N + i] - full.at(p).1;
            detail_peak = detail_peak.max(d.abs());
            let on =
                [0, 1].map(|k| ((p[k] - ON_TEXELS.node[k]) / ON_TEXELS.cell).fract().abs() < 1e-6);
            if on[0] && on[1] {
                at_samples = at_samples.max(d.abs());
            }
        }
    }
    assert!(
        at_samples < 1e-3,
        "the detail moves a sample {at_samples} m"
    );
    assert!(
        detail_peak > 0.5,
        "no fine detail over the band: {detail_peak}"
    );
    // The synthetic's coarse octaves are gone: its detail is a fraction of the synthetic's own.
    let height = |p: [f64; 2]| coarse.height(p);
    let style = relief_synth::style(ALPS, INDEX, &params);
    let synth = relief_synth::residuals(&ON_TEXELS, INDEX, &height, &params, style);
    let synth_peak = synth.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(
        detail_peak < 0.5 * synth_peak,
        "the detail over the band {detail_peak} m against the synthetic's {synth_peak} m"
    );
}

#[test]
fn a_step_at_the_fills_edge_becomes_a_slope_and_the_rest_is_kept() {
    let n = 200;
    let filled: Vec<bool> = (0..n * n).map(|k| k % n >= 100).collect();
    let band: Vec<f32> = (0..n * n)
        .map(|k| if k % n >= 100 { 10.0 } else { 0.0 } + 0.01 * (k / n) as f32)
        .collect();
    let out = across_seams(&band, &filled, n, 60.0);
    let row = 100 * n;
    // Across the edge the largest step a texel is a fraction of the 10 m step.
    let step = (1..n)
        .map(|i| (out[row + i] - out[row + i - 1]).abs())
        .fold(0f32, f32::max);
    assert!(step < 2.5, "the step at the edge {step} m a texel");
    // Past the seam's reach, unchanged.
    for i in [0, 20, 180, 199] {
        assert_eq!(out[row + i], band[row + i]);
    }
    // No edge: unchanged.
    assert!(across_seams(&band, &vec![false; n * n], n, 60.0) == band);
}

#[test]
fn held_samples_move_no_further_than_the_cap_and_small_bands_pass() {
    let params = Params::default();
    let coarse = coarse();
    // A band 80 m high in places: the samples held within 35 m, a 5 m band nearly untouched.
    for (amplitude, cap) in [(80.0f32, 35.0f32), (5.0, 35.0)] {
        let wave = move |p: [f64; 2]| {
            amplitude * (p[0] / 150.0).sin() as f32 * (p[1] / 210.0).cos() as f32
        };
        let mut held = base(1.0, &wave);
        held.cap_m = cap;
        let r = residuals(ALPS, &ON_TEXELS, INDEX, Some(&held), &coarse, &params);
        let mut most = 0f32;
        for j in GUTTER as usize..N - GUTTER as usize {
            for i in GUTTER as usize..N - GUTTER as usize {
                let p = texel_point(INDEX, i, j);
                let on = [0, 1]
                    .map(|k| ((p[k] - ON_TEXELS.node[k]) / ON_TEXELS.cell).fract().abs() < 1e-6);
                if on[0] && on[1] {
                    most = most.max(r[j * N + i].abs());
                    let b = wave(p);
                    assert!(
                        (r[j * N + i] - capped(b, cap)).abs() < 1e-2,
                        "{p:?}: {} against {b}",
                        r[j * N + i]
                    );
                }
            }
        }
        assert!(most < cap, "a sample moved {most} m");
        if amplitude < 10.0 {
            assert!(most > 0.95 * amplitude, "a small band held back: {most}");
        }
    }
}
