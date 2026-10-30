use super::*;
use crate::relief::{Mode, Source, page_source_in};

const N: usize = PAGE_TEXELS as usize;

#[test]
fn glo30_is_a_test_source_chosen_by_name_only() {
    assert_eq!(Mode::parse("glo30"), Some(Mode::Glo30));
    assert_eq!(Mode::parse("GLO-30"), Some(Mode::Glo30));
    let geo = LatLon {
        lat: 46.58,
        lon: 8.0,
    };
    assert_eq!(
        page_source_in(Mode::Glo30, geo, [0, 0]),
        Some(Source::Glo30)
    );
    // Not in auto: the Alps take the combined there (GLO-30's band under the synthetic's detail).
    assert_eq!(
        page_source_in(Mode::Auto, geo, [0, 0]),
        Some(Source::Combined)
    );
}

#[test]
fn the_mask_is_none_near_closed_ground_and_all_past_the_fade() {
    assert_eq!(weight_at(0.0), 0.0);
    assert_eq!(weight_at(DEAD_M as f32), 0.0);
    assert!((weight_at((DEAD_M + FADE_M / 2.0) as f32) - 0.5).abs() < 1e-6);
    assert_eq!(weight_at((DEAD_M + FADE_M) as f32), 1.0);
    // A forest west of x = 1000 m on the page: none in it or within DEAD_M of it, rising with the distance, all of it
    // past the fade.
    let w = weights(&|m| {
        if m[0] < 1000.0 {
            cover::TREE
        } else {
            cover::GRASS
        }
    });
    let row = N / 2;
    let at = |x: f64| w[row * N + ((x / TEXEL_M) - 0.5 + f64::from(GUTTER)).round() as usize];
    assert_eq!(at(500.0), 0.0);
    assert_eq!(at(1000.0 + DEAD_M - 8.0), 0.0);
    assert!(at(1000.0 + DEAD_M + FADE_M / 2.0) > 0.3);
    assert!(at(1000.0 + DEAD_M + FADE_M / 2.0) < 0.7);
    assert_eq!(at(1000.0 + DEAD_M + FADE_M + 8.0), 1.0);
    let mut last = 0.0;
    for i in 0..N {
        let v = w[row * N + i];
        assert!(v >= last - 1e-6, "rising at texel {i}");
        last = v;
    }
    // Open everywhere: all of it, the gutters too.
    assert!(weights(&|_| cover::BARE).iter().all(|&v| v == 1.0));
    assert!(weights(&|_| cover::SNOW).iter().all(|&v| v == 1.0));
    assert!(weights(&|_| cover::BUILT).iter().all(|&v| v == 0.0));
}

#[test]
fn a_forest_edge_makes_no_step_and_a_bare_ridge_between_samples_stands_up() {
    let lattice = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    let index = [1, -2];
    let side = relief::page_m();
    let corner = index[0] as f64 * side;
    // A slope with a 25 m canopy over its west half, and a ridge 20 m high and 12 m thick in the middle of a lattice cell
    // on the bare east half.
    let k = ((corner + 1500.0 - lattice.node[0]) / lattice.cell).floor();
    let ridge = lattice.node[0] + k * lattice.cell + lattice.cell / 2.0 - corner;
    let height = |m: [f64; 2]| {
        let ground = 800.0 + 0.3 * m[0] as f32;
        let canopy = if m[0] < 1000.0 { 25.0 } else { 0.0 };
        let crest = if (m[0] - ridge).abs() < 6.0 {
            20.0
        } else {
            0.0
        };
        ground + canopy + crest
    };
    let class = |m: [f64; 2]| {
        if m[0] < 1000.0 {
            cover::TREE
        } else {
            cover::BARE
        }
    };
    let p = parts(&lattice, index, &height, &class);
    let masked: Vec<f32> = p
        .residual
        .iter()
        .zip(&p.weight)
        .map(|(r, w)| r * w)
        .collect();
    let row = N / 2;
    let texel = |x: f64| ((x / TEXEL_M) - 0.5 + f64::from(GUTTER)).round() as usize;
    // Unmasked, the canopy's edge stands as a step between the samples; masked, nothing within the fade of it.
    let raw_edge = (texel(960.0)..texel(1040.0))
        .map(|i| p.residual[row * N + i].abs())
        .fold(0f32, f32::max);
    assert!(raw_edge > 5.0, "the canopy's edge unmasked: {raw_edge}");
    let edge = (0..texel(1000.0 + DEAD_M + FADE_M))
        .map(|i| masked[row * N + i].abs())
        .fold(0f32, f32::max);
    assert!(edge < 0.01, "about the forest masked: {edge}");
    // The bare ridge stands up by most of its height, a cell off it nothing.
    let on = masked[row * N + texel(ridge)];
    let off = masked[row * N + texel(ridge + lattice.cell)];
    assert!(on > 12.0, "on the ridge: {on}");
    assert!(off.abs() < 0.5, "a cell off it: {off}");
}

#[test]
fn the_options_read_fills_and_band() {
    assert_eq!(Options::parse(""), Ok(Options::default()));
    assert_eq!(
        Options::parse("band, fills"),
        Ok(Options {
            keep_fills: true,
            band: true
        })
    );
    assert!(Options::parse("bands").is_err());
    // TanDEM-X's own heights (edited or not) are kept; voids and fills from other models are not.
    assert!(!filled(1) && !filled(2));
    assert!(
        [0, 3, 4, 5, 6, 7, 8, 9, 100, 101]
            .iter()
            .all(|&v| filled(v))
    );
}

#[test]
fn the_band_keeps_glo30s_short_shape_and_the_windows_long() {
    // GLO-30 a plane 30 m above and steeper than the window's: the difference is long, no band.
    let plane = band_texels(&|m| 530.0 + 0.3 * m[0] as f32 - 0.1 * m[1] as f32, &|m| {
        500.0 + 0.25 * m[0] as f32
    });
    let most = plane.iter().map(|v| v.abs()).fold(0f32, f32::max);
    assert!(most < 0.05, "a plane's band: {most}");
    // A ridge 20 m high and 40 m wide GLO-30 has and the window does not: most of it, and nothing far off it.
    let ridge = 1000.0;
    let band = band_texels(
        &|m| {
            600.0
                + if (m[0] - ridge).abs() < 20.0 {
                    20.0
                } else {
                    0.0
                }
        },
        &|_| 600.0,
    );
    let texel = |x: f64| ((x / TEXEL_M) - 0.5 + f64::from(GUTTER)).round() as usize;
    let row = N / 2;
    let on = band[row * N + texel(ridge)];
    let far = band[row * N + texel(ridge + 400.0)];
    assert!(on > 14.0, "on the ridge: {on}");
    assert!(far.abs() < 0.05, "far off it: {far}");
}
