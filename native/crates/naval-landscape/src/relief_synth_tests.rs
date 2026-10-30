use super::*;

const N: usize = PAGE_TEXELS as usize;

/// Steep mountains: ridges 1.4 km apart rising 800 m either side of 1,500 m (slopes to 48°).
fn mountains(p: [f64; 2]) -> f32 {
    (1500.0 + 800.0 * (p[0] / 700.0).sin() * (p[1] / 900.0).cos()) as f32
}

/// A lattice whose samples stand on texel middles (theatre x and z 2 m past a multiple of 40).
/// A place in the Alps (a theatre about it).
const ALPS: LatLon = LatLon {
    lat: 46.58,
    lon: 8.0,
};

const ON_TEXELS: Lattice = Lattice {
    node: [2.0, -38.0],
    cell: 40.0,
};

#[test]
fn a_page_is_the_same_each_time_it_is_made() {
    let params = Params::default();
    let a = residuals(&ON_TEXELS, [3, -2], &mountains, &params, [0.5; 4]);
    let b = residuals(&ON_TEXELS, [3, -2], &mountains, &params, [0.5; 4]);
    assert!(a == b, "two makings of one page differ");
    let peak = a.iter().fold(0f32, |m, v| m.max(v.abs()));
    assert!(peak > 2.0, "steep mountains hold little relief: {peak}");
}

#[test]
fn neighbouring_pages_agree_in_their_gutters() {
    let params = Params::default();
    let g = GUTTER as usize;
    let lattice = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    let page = |index| residuals(&lattice, index, &mountains, &params, [0.5; 4]);
    let (here, east, south) = (page([0, 0]), page([1, 0]), page([0, 1]));
    let mut worst = 0f32;
    for j in 0..N {
        // Page (0, 0)'s last 2g columns are page (1, 0)'s first 2g: its east gutter and the neighbour's own edge.
        for k in 0..2 * g {
            worst = worst.max((here[j * N + N - 2 * g + k] - east[j * N + k]).abs());
        }
    }
    for k in 0..2 * g {
        for i in 0..N {
            worst = worst.max((here[(N - 2 * g + k) * N + i] - south[k * N + i]).abs());
        }
    }
    assert!(worst < 1e-3, "pages disagree by {worst} m where they meet");
}

#[test]
fn a_page_is_nothing_at_the_samples() {
    let params = Params::default();
    let index = [-1, 2];
    let texels = residuals(&ON_TEXELS, index, &mountains, &params, [0.5; 4]);
    let side = page_m();
    let corner = [index[0] as f64 * side, index[1] as f64 * side];
    let g = f64::from(GUTTER);
    let (mut samples, mut worst, mut peak) = (0, 0f32, 0f32);
    for j in 0..N {
        for i in 0..N {
            let p = [
                corner[0] + (i as f64 - g + 0.5) * TEXEL_M,
                corner[1] + (j as f64 - g + 0.5) * TEXEL_M,
            ];
            let on =
                (0..2).all(|k| ((p[k] - ON_TEXELS.node[k]) / ON_TEXELS.cell).fract().abs() < 1e-9);
            let v = texels[j * N + i].abs();
            peak = peak.max(v);
            if on {
                samples += 1;
                worst = worst.max(v);
            }
        }
    }
    assert!(samples > 100, "{samples} samples on the page's texels");
    assert!(peak > 2.0, "no relief: {peak}");
    assert!(
        worst < 1e-3,
        "{worst} m at a sample (the page's peak {peak} m)"
    );
}

#[test]
fn flats_the_shore_and_the_sea_take_none() {
    let lattice = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    // A plateau at 500 m, a gentle slope (2 %), and steep land that never rises 3 m above the sea.
    let plateau = |_: [f64; 2]| 500.0f32;
    let gentle = |p: [f64; 2]| (300.0 + 0.02 * p[0]) as f32;
    let shore = |p: [f64; 2]| (-40.0 + 43.0 * (p[0] / 150.0).sin().abs()) as f32;
    for (name, height) in [
        ("a plateau", &plateau as &(dyn Fn([f64; 2]) -> f32 + Sync)),
        ("a gentle slope", &gentle),
        ("the shore", &shore),
    ] {
        let peak = residuals(&lattice, [0, 0], height, &Params::default(), [1.0; 4])
            .iter()
            .fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak < 0.05, "{name}: {peak} m of relief");
        assert!(
            make_page(ALPS, &lattice, [0, 0], height, &Water::none(ALPS, [0, 0])).is_none(),
            "{name}: a page"
        );
    }
    assert!(
        make_page(
            ALPS,
            &lattice,
            [0, 0],
            &mountains,
            &Water::none(ALPS, [0, 0])
        )
        .is_some()
    );
}

#[test]
fn the_relief_stays_within_a_share_of_the_land_about() {
    let lattice = Lattice {
        node: [13.0, -27.0],
        cell: 40.0,
    };
    let params = Params::default();
    // At most the octaves' amplitudes together at full strength (the curvature apart), whatever the slope.
    let most: f32 = (0..params.octaves)
        .map(|k| params.amplitude * params.wavelength_m * params.gain.powi(k as i32))
        .sum();
    for scale in [0.25, 1.0, 3.0] {
        let height = |p: [f64; 2]| mountains([p[0] / scale, p[1] / scale]);
        let texels = residuals(&lattice, [0, 0], &height, &params, [0.5; 4]);
        let peak = texels.iter().fold(0f32, |m, v| m.max(v.abs()));
        let rms = (texels.iter().map(|v| v * v).sum::<f32>() / texels.len() as f32).sqrt();
        assert!(peak < 2.0 * most + 40.0, "scale {scale}: peak {peak} m");
        assert!(rms < most, "scale {scale}: rms {rms} m");
    }
}

#[test]
fn parameters_read_from_their_spec() {
    let p = Params::parse("amplitude=0.1, slope=0.2:0.8,octaves=4").unwrap();
    assert_eq!(p.amplitude, 0.1);
    assert_eq!(p.slope, [0.2, 0.8]);
    assert_eq!(p.octaves, 4);
    assert_eq!(p.gain, Params::default().gain);
    assert!(Params::parse("wobble=3").is_err());
    assert!(Params::parse("slope=3").is_err());
}

#[test]
fn the_cheap_sine_is_close() {
    for k in -400..400 {
        let x = k as f32 * 0.01237;
        let e = (sin_turns(x) - (x * TAU).sin()).abs();
        assert!(e < 2e-3, "sin_turns({x}) is off by {e}");
    }
}
