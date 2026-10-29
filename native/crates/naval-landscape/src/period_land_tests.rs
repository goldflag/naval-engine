use super::*;

use naval_conditions::{Climate, LandStyle};
use naval_terrain::buildings::thinning::{
    HYDE_PER_DEGREE, TOWN_EASE, cell_middle, history_about, hyde_cell, hyde_middle, period_share,
};

/// The answers are the process's: one test at a time touches them.
fn lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn style() -> SurfaceStyle {
    SurfaceStyle {
        style: LandStyle::Chalk,
        shore: [0.0; 3],
        low: [0.0; 3],
        high: [0.0; 3],
        equator: [0.0; 2],
        climate: Climate::default(),
    }
}

/// 0-1 by texel, the same for the same texel whatever window holds it.
fn hash(i: i64, j: i64) -> f32 {
    let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 31;
    h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
    h ^= h >> 29;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

/// A window's maps of `size`² texels of `texel_m` from chart (and theatre) point `origin`, each texel's class
/// `class(texel centre, texel)` by its theatre texel (`texel_m` steps from the theatre's origin).
fn maps(
    size: u32,
    texel_m: f64,
    origin: [f64; 2],
    class: impl Fn([f64; 2], [i64; 2]) -> u8,
) -> SurfaceMaps {
    let n = size as usize;
    let mut cover = vec![0u8; n * n];
    for j in 0..n {
        for i in 0..n {
            let p = [
                origin[0] + (i as f64 + 0.5) * texel_m,
                origin[1] + (j as f64 + 0.5) * texel_m,
            ];
            let texel = [
                (p[0] / texel_m).floor() as i64,
                (p[1] / texel_m).floor() as i64,
            ];
            cover[j * n + i] = class(p, texel);
        }
    }
    SurfaceMaps {
        origin,
        extent_m: texel_m * f64::from(size),
        size,
        albedo: vec![0; n * n * 4],
        cover,
    }
}

/// A town about the theatre's origin: built up wholly within 800 m, thinning to none at 3.3 km, grass about.
fn town(p: [f64; 2], texel: [i64; 2]) -> u8 {
    let r = p[0].hypot(p[1]);
    let built = if r < 800.0 {
        1.0
    } else {
        (1.0 - (r - 800.0) / 2500.0).max(0.0) as f32
    };
    if hash(texel[0], texel[1]) < built {
        cover::BUILT
    } else {
        cover::GRASS
    }
}

/// The middle of a 5' cell in the open Atlantic, where the history has no data (each share by its region's).
fn atlantic() -> LatLon {
    let (lat, lon) = hyde_middle(hyde_cell(40.02, -40.02));
    LatLon { lat, lon }
}

const YEAR: u16 = 1942;

/// The share the buildings keep in `geo`'s 5' cell in `YEAR`.
fn wanted_share(geo: LatLon) -> f32 {
    let cell = hyde_cell(geo.lat, geo.lon);
    let (lat, lon) = hyde_middle(cell);
    let (own, around) = history_about(cell, YEAR);
    period_share(own, around, Region::at(lat, lon, &style()), YEAR)
}

#[test]
fn today_never_marks() {
    let _g = lock();
    let geo = atlantic();
    let maps = maps(256, 25.0, [-3200.0, -3200.0], town);
    assert!(mark_window(geo, Period::Today, &style(), &maps, [0.0, 0.0]).is_none());
    assert!(answers(geo, Period::Today).is_none());
    assert_eq!(marks(Period::Today, 99), 0);
    assert_eq!(marks_period(0), Period::Today);
    assert!(stood(geo, Period::Today, [0.0, 0.0]));
    assert!(stood(geo, Period::Today, [2300.0, 0.0]));
    assert!(!changed_since(
        geo,
        Period::Today,
        0,
        [-100, -100, 100, 100],
        true
    ));
}

#[test]
fn marks_numbers_round_trip() {
    let m = marks(Period::Year(1941), 123_456);
    assert_eq!(marks_period(m), Period::Year(1941));
    assert_eq!(marks_version(m), 123_456);
    assert_ne!(m, 0);
}

#[test]
fn spans_partition_the_texels() {
    for (corner, texel_m, size) in [
        ([-6400.0, -6400.0], 25.0, [512, 512]),
        ([-1234.5, 777.25], 23.4375, [300, 200]),
        ([0.0, 0.0], 117.1875, [2048, 64]),
        ([-2560.0, 2560.0], 5.0, [512, 512]),
    ] {
        let grid = Grid {
            corner,
            texel_m,
            size,
        };
        let (first, last) = grid.chunks();
        for a in 0..2 {
            let mut next = 0;
            for c in first[a]..=last[a] {
                let (s, e) = grid.span(a, c);
                assert_eq!(s, next, "axis {a} chunk {c} starts where the last ended");
                assert!(e >= s);
                for t in s..e {
                    let centre = corner[a] + (t as f64 + 0.5) * texel_m;
                    assert_eq!((centre / CHUNK_M).floor() as i32, c);
                }
                next = e;
            }
            assert_eq!(next, size[a], "axis {a}: every texel in a chunk");
        }
    }
}

/// The share of a 5' cell's built-up land kept (counted as the estimate counts it), and the kept share of the core's
/// and the edge's land.
#[test]
fn estimate_keeps_the_period_share_core_first() {
    let _g = lock();
    let geo = atlantic();
    let maps = maps(512, 25.0, [-6400.0, -6400.0], town);
    let e = estimate(geo, YEAR, &style(), &maps, [0.0, 0.0]);
    assert!(e.built_chunks() >= 20, "{} chunks", e.built_chunks());
    let (mut total, mut kept) = (0u32, 0u32);
    let (mut core, mut core_kept, mut edge, mut edge_kept) = (0u32, 0u32, 0u32, 0u32);
    for cz in e.first[1]..e.first[1] + e.size[1] as i32 {
        for cx in e.first[0]..e.first[0] + e.size[0] as i32 {
            let Some(c) = e.get([cx, cz]) else {
                continue;
            };
            let corner = chunk_corner([cx, cz]);
            for k in 0..CELLS * CELLS {
                let m = cell_middle(k);
                let count = (thinning::ring_at(&c.built, m) * FOOTPRINTS_PER_CELL).round() as u32;
                let stood = c.thinning.keeps(m, 0.0);
                total += count;
                kept += if stood { count } else { 0 };
                let r = (corner[0] + f64::from(m[0])).hypot(corner[1] + f64::from(m[1]));
                if r < 500.0 {
                    core += 1;
                    core_kept += u32::from(stood);
                } else if r > 2800.0 && count > 0 {
                    edge += 1;
                    edge_kept += u32::from(stood);
                }
            }
        }
    }
    let want = wanted_share(geo);
    let share = kept as f32 / total as f32;
    eprintln!(
        "kept {share:.3} of {total} (the period's {want:.3}); core {core_kept}/{core}, edge {edge_kept}/{edge}"
    );
    assert!(total >= LEAST_SAMPLE_FOR_TEST, "{total} footprints counted");
    assert!(
        (share - want).abs() < 0.05,
        "kept {share:.3} of the built-up land, the period's share {want:.3}"
    );
    assert_eq!(core_kept, core, "the core stood");
    assert!(edge > 50);
    assert_eq!(
        edge_kept, 0,
        "the edge went ({edge_kept} of {edge} cells kept)"
    );
}

const LEAST_SAMPLE_FOR_TEST: u32 = thinning::LEAST_SAMPLE;

/// The scores of neighbouring chunks agree over the ring they share, within a window (across its blocks) and between
/// windows over the same land.
#[test]
fn scores_are_seam_free_across_chunks_and_windows() {
    let _g = lock();
    let geo = atlantic();
    let a = estimate(
        geo,
        YEAR,
        &style(),
        &maps(512, 25.0, [-6400.0, -6400.0], town),
        [0.0, 0.0],
    );
    let mut pairs = 0;
    for cz in a.first[1]..a.first[1] + a.size[1] as i32 {
        for cx in a.first[0]..a.first[0] + a.size[0] as i32 {
            let Some(here) = a.get([cx, cz]) else {
                continue;
            };
            if let Some(east) = a.get([cx + 1, cz]) {
                for j in 0..RING {
                    for (mine, theirs) in [(RING - 1, 1), (RING - 2, 0)] {
                        let (s, t) = (
                            here.thinning.score[j * RING + mine],
                            east.thinning.score[j * RING + theirs],
                        );
                        assert!(
                            (s - t).abs() < 1e-6,
                            "chunk {cx},{cz} row {j}: {s} against {t}"
                        );
                    }
                }
                pairs += 1;
            }
            if let Some(south) = a.get([cx, cz + 1]) {
                for i in 0..RING {
                    for (mine, theirs) in [(RING - 1, 1), (RING - 2, 0)] {
                        let (s, t) = (
                            here.thinning.score[mine * RING + i],
                            south.thinning.score[theirs * RING + i],
                        );
                        assert!(
                            (s - t).abs() < 1e-6,
                            "chunk {cx},{cz} column {i}: {s} against {t}"
                        );
                    }
                }
                pairs += 1;
            }
        }
    }
    assert!(pairs > 40, "{pairs} neighbours");
    // Another window over the same land, 2 km east: the chunks well inside both score alike.
    let b = estimate(
        geo,
        YEAR,
        &style(),
        &maps(512, 25.0, [-4400.0, -6400.0], town),
        [0.0, 0.0],
    );
    let mut same = 0;
    for cz in a.first[1] + 2..a.first[1] + a.size[1] as i32 - 2 {
        for cx in b.first[0] + 2..a.first[0] + a.size[0] as i32 - 2 {
            if let (Some(x), Some(y)) = (a.get([cx, cz]), b.get([cx, cz])) {
                assert_eq!(x.thinning.score, y.thinning.score, "chunk {cx},{cz}");
                same += 1;
            }
        }
    }
    assert!(same > 10, "{same} chunks in both");
}

/// Along a row across a 5' cell's side, from a dense town to a thin one: the cells' bars change smoothly.
#[test]
fn bars_are_seam_free_across_five_minute_cells() {
    let _g = lock();
    // On the side between two 5' cells, in the middle of their row.
    let row = hyde_cell(40.02, -40.02)[1];
    let side = (f64::from(hyde_cell(40.02, -40.02)[0]) + 1.0) / thinning::HYDE_PER_DEGREE - 180.0;
    let geo = LatLon {
        lat: (f64::from(row) + 0.5) / thinning::HYDE_PER_DEGREE,
        lon: side,
    };
    let east = to_lat_lon(geo, [1000.0, 0.0]).lon > geo.lon;
    let dense_west = move |p: [f64; 2], texel: [i64; 2]| {
        if p[0].abs() > 3000.0 || p[1].abs() > 3000.0 {
            return cover::GRASS;
        }
        let dense = (p[0] < 0.0) == east;
        if dense || hash(texel[0], texel[1]) < 0.3 {
            cover::BUILT
        } else {
            cover::GRASS
        }
    };
    let e = estimate(
        geo,
        YEAR,
        &style(),
        &maps(320, 25.0, [-4000.0, -4000.0], dense_west),
        [0.0, 0.0],
    );
    // The bars along the cells' row at z 50-150 m, west to east of the window.
    let mut bars = Vec::new();
    for cx in -3..3 {
        let c = e
            .get([cx, 0])
            .expect("built-up land in every chunk of the row");
        bars.extend((0..CELLS).map(|i| c.thinning.bar[CELLS + i]));
    }
    let step = bars
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    let (dense, thin) = (bars[5], bars[bars.len() - 6]);
    eprintln!("bars {dense:.3} to {thin:.3}, the largest step {step:.4}");
    assert!(
        (dense - thin).abs() > 0.03,
        "the towns' bars differ: {dense} and {thin}"
    );
    assert!(
        step < 0.01,
        "a step of {step} between neighbouring cells: {bars:?}"
    );
}

/// Answers whose chunk [0, 0] holds no ground that stood: its cells west of x = 500 m mostly built up today.
fn nothing_stood(geo: LatLon) -> Answers {
    let built = (0..RING * RING)
        .map(|k| if k % RING <= 5 { 0.8 } else { 0.2 })
        .collect();
    let estimate = Estimate {
        first: [0, 0],
        size: [1, 1],
        texel_m: 10.0,
        chunks: vec![Some(Arc::new(Estimated {
            thinning: Thinning {
                score: vec![0.0; RING * RING],
                bar: vec![1.0; CELLS * CELLS],
                cap_m: 0.0,
                ..Thinning::default()
            },
            built,
        }))],
    };
    Answers {
        frame: frame(geo),
        year: YEAR,
        version: 1,
        thinned: HashMap::new(),
        estimates: vec![(1, Arc::new(estimate))],
    }
}

#[test]
fn marks_only_the_right_classes() {
    let answers = nothing_stood(atlantic());
    let grid = Grid {
        corner: [0.0, 0.0],
        texel_m: 10.0,
        size: [100, 100],
    };
    let classes_west_east = [
        (cover::BUILT, true, true),
        (cover::TREE, true, false),
        (cover::GRASS, true, false),
        (cover::CROP, true, false),
        (cover::WETLAND, true, false),
        (cover::WATER, false, false),
        (cover::SNOW, false, false),
        (cover::UNKNOWN, false, false),
    ];
    for (class, west, east) in classes_west_east {
        let mut classes = vec![class; 100 * 100];
        mark(&answers, &grid, &mut classes);
        for (i, want) in [(20, west), (80, east)] {
            let got = classes[50 * 100 + i];
            assert_eq!(cover::today(got), class);
            assert_eq!(
                cover::is_past_country(got),
                want,
                "class {class} at x {}0 m",
                i
            );
        }
        // Marked again from the same answers: the same; from none: taken off.
        let again = classes.clone();
        mark(&answers, &grid, &mut classes);
        assert_eq!(classes, again);
        mark(&Answers::default(), &grid, &mut classes);
        assert!(classes.iter().all(|&c| c == class));
    }
    // A chunk the buildings answered for stands as they thin it, whatever the cover's estimate says.
    let mut answers = answers;
    answers.thinned.insert(
        [0, 0],
        Arc::new(Thinning {
            score: vec![0.5; RING * RING],
            bar: vec![0.2; CELLS * CELLS],
            cap_m: 0.0,
            ..Thinning::default()
        }),
    );
    let mut classes = vec![cover::BUILT; 100 * 100];
    assert_eq!(mark(&answers, &grid, &mut classes), 0);
}

/// A near tile marked over the window's cover (upsampled) agrees with the window's marks texel for texel where their
/// centres meet, and nearly everywhere.
#[test]
fn near_tiles_agree_with_the_window() {
    let _g = lock();
    let geo = atlantic();
    let window = maps(512, 25.0, [-6400.0, -6400.0], town);
    let (classes, m) = mark_window(geo, Period::Year(YEAR), &style(), &window, [0.0, 0.0])
        .expect("a past period's marks");
    assert_eq!(marks_period(m), Period::Year(YEAR));
    let answers = answers(geo, Period::Year(YEAR)).expect("the answers held");
    assert_eq!(marks_version(m), answers.version());
    let marked = classes
        .iter()
        .filter(|&&c| cover::is_past_country(c))
        .count();
    assert!(marked > 1000, "{marked} texels marked");
    // A near tile of 5 m texels from theatre (0, 0), as `near` lays one (index × side).
    let n = 512;
    let near = Grid {
        corner: [0.0, 0.0],
        texel_m: 5.0,
        size: [n, n],
    };
    let at_window = |p: f64| ((p + 6400.0) / 25.0).floor() as usize;
    let mut tile = vec![0u8; n * n];
    for b in 0..n {
        for a in 0..n {
            let (x, z) = ((a as f64 + 0.5) * 5.0, (b as f64 + 0.5) * 5.0);
            tile[b * n + a] = window.cover[at_window(z) * 512 + at_window(x)];
        }
    }
    mark(&answers, &near, &mut tile);
    let (mut met, mut agreed, mut all) = (0, 0, 0);
    for b in 0..n {
        for a in 0..n {
            let (x, z) = ((a as f64 + 0.5) * 5.0, (b as f64 + 0.5) * 5.0);
            let w = classes[at_window(z) * 512 + at_window(x)];
            let t = tile[b * n + a];
            assert_eq!(cover::today(w), cover::today(t));
            if a % 5 == 2 && b % 5 == 2 {
                assert_eq!(w, t, "near texel {a},{b} on window texel's centre");
                met += 1;
            }
            agreed += usize::from(w == t);
            all += 1;
        }
    }
    assert!(met > 10_000);
    let share = agreed as f64 / all as f64;
    eprintln!("{marked} window texels marked; {share:.4} of the near texels agree, {met} met");
    assert!(share > 0.97, "{share:.4} of the near texels agree");
    // Stood answers as the marks: the core stood, the outskirts' marked land not.
    assert!(stood(geo, Period::Year(YEAR), [0.0, 0.0]));
}

#[test]
fn buildings_answers_are_told_and_take_over() {
    let _g = lock();
    let geo = LatLon {
        lat: 12.5,
        lon: 101.25,
    };
    let period = Period::Year(YEAR);
    let thinning = |score: f32| Thinning {
        score: vec![score; RING * RING],
        bar: vec![0.3; CELLS * CELLS],
        cap_m: 25.0,
        tall: vec![([1.0, 1.0], 40.0)],
        later: vec![[2.0, 2.0]],
        ..Thinning::default()
    };
    thinned(geo, YEAR, vec![([3, 4], thinning(0.1))]);
    let held = answers(geo, period).expect("held");
    let first = held.version();
    assert_eq!(version(), first.max(version()));
    assert!(!stood(geo, period, [3500.0, 4500.0]));
    assert!(stood(geo, period, [5500.0, 4500.0]), "nothing known: stood");
    assert!(
        stood(geo, Period::Year(YEAR + 1), [3500.0, 4500.0]),
        "another year's: none held"
    );
    assert!(changed_since(geo, period, first - 1, [3, 4, 3, 4], false));
    assert!(!changed_since(geo, period, first - 1, [5, 4, 9, 9], true));
    assert!(!changed_since(geo, period, first, [0, 0, 9, 9], true));
    // The same again changes nothing; another score does.
    thinned(geo, YEAR, vec![([3, 4], thinning(0.1))]);
    assert_eq!(answers(geo, period).unwrap().version(), first);
    thinned(geo, YEAR, vec![([3, 4], thinning(0.5))]);
    let second = answers(geo, period).unwrap().version();
    assert!(second > first);
    assert!(stood(geo, period, [3500.0, 4500.0]));
    assert_eq!(thinned_since(geo, YEAR, first), Some(vec![[3, 4]]));
    assert!(changed_since(geo, period, first, [0, 0, 9, 9], false));
    // What the ground keeps of a thinning: its scores and bars.
    let kept = answers(geo, period).unwrap();
    let t = kept.ground([3, 4]).thinned.expect("the buildings' answer");
    assert!(t.tall.is_empty() && t.later.is_empty());
}

/// A town just west of the date line: its cells' bars read the 5' cells east of the line (their columns wrapped) as
/// any other neighbour's, so they keep level to its edge.
#[test]
fn bars_hold_level_at_the_date_line() {
    let _g = lock();
    let row = hyde_cell(-9.43, 179.97)[1];
    let geo = LatLon {
        lat: (f64::from(row) + 0.5) / thinning::HYDE_PER_DEGREE,
        lon: 179.97,
    };
    assert!(
        to_lat_lon(geo, [3000.0, 0.0]).lon < 180.0 && to_lat_lon(geo, [3000.0, 0.0]).lon > 179.99
    );
    let town = |p: [f64; 2], _: [i64; 2]| {
        if p[0].abs() < 3000.0 && p[1].abs() < 3000.0 {
            cover::BUILT
        } else {
            cover::GRASS
        }
    };
    let e = estimate(
        geo,
        YEAR,
        &style(),
        &maps(320, 25.0, [-4000.0, -4000.0], town),
        [0.0, 0.0],
    );
    let mut bars = Vec::new();
    for cx in -3..3 {
        let c = e
            .get([cx, 0])
            .expect("built-up land in every chunk of the row");
        bars.extend((0..CELLS).map(|i| c.thinning.bar[CELLS + i]));
    }
    let (low, high) = bars
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), &b| {
            (l.min(b), h.max(b))
        });
    eprintln!("bars from {low:.4} to {high:.4} at the date line");
    assert!(high < 1.0 && high - low < 0.012, "{bars:?}");
}

/// A batch of the buildings' for a scenario left behind answers for it alone; a third scenario lets the least lately
/// used go, and one made again tells every land marked from it to mark again.
#[test]
fn each_scenario_keeps_its_own_answers() {
    let _g = lock();
    let (a, b, c) = (
        LatLon {
            lat: 20.0,
            lon: 20.0,
        },
        LatLon {
            lat: 21.0,
            lon: 21.0,
        },
        LatLon {
            lat: 22.0,
            lon: 22.0,
        },
    );
    let period = Period::Year(YEAR);
    let window = || {
        Arc::new(Estimate {
            first: [0, 0],
            size: [1, 1],
            texel_m: 10.0,
            chunks: vec![None],
        })
    };
    let held = hold_estimate(a, YEAR, window());
    let thinning = Thinning {
        score: vec![0.5; RING * RING],
        bar: vec![0.2; CELLS * CELLS],
        cap_m: 0.0,
        ..Thinning::default()
    };
    thinned(b, YEAR, vec![([0, 0], thinning)]);
    let still = answers(a, period).expect("a's answers held");
    assert_eq!(still.version(), held.version());
    assert_eq!(still.estimates.len(), 1);
    assert!(answers(b, period).is_some_and(|b| b.thinned_chunks().count() == 1));
    assert!(!changed_since(
        a,
        period,
        held.version(),
        [-5, -5, 5, 5],
        true
    ));
    // A third: a (least lately used) goes.
    hold_estimate(c, YEAR, window());
    assert!(answers(a, period).is_none());
    assert!(answers(b, period).is_some() && answers(c, period).is_some());
    // a again: what was marked from its old answers is marked again.
    hold_estimate(a, YEAR, window());
    assert!(changed_since(
        a,
        period,
        held.version(),
        [100, 100, 100, 100],
        false
    ));
}

/// The buildings' answer holds the land they kept whole (`Thinning::kept`: a working waterfront, a listed yard, the
/// ground about what a land's lift kept): its ground stands though its score is a field's.
#[test]
fn the_ground_stands_under_what_the_buildings_kept() {
    let _g = lock();
    let geo = LatLon {
        lat: 13.5,
        lon: 102.25,
    };
    let period = Period::Year(YEAR);
    let field = |kept: thinning::Kept, yards: Vec<([f32; 2], f32)>| Thinning {
        score: vec![0.1; RING * RING],
        bar: vec![0.3; CELLS * CELLS],
        cap_m: 25.0,
        yards,
        kept,
        ..Thinning::default()
    };
    let mut kept = thinning::Kept::default();
    kept.set_ring(
        &[
            [200.0, 200.0],
            [300.0, 200.0],
            [300.0, 300.0],
            [200.0, 300.0],
        ],
        20.0,
    );
    thinned(
        geo,
        YEAR,
        vec![([3, 4], field(kept, vec![([700.0, 700.0], 100.0)]))],
    );
    let first = answers(geo, period).expect("held").version();
    assert!(stood(geo, period, [3250.0, 4250.0]), "a kept shed's ground");
    assert!(stood(geo, period, [3190.0, 4250.0]), "about it");
    assert!(stood(geo, period, [3700.0, 4750.0]), "a listed yard's");
    assert!(!stood(geo, period, [3500.0, 4100.0]), "the field");
    // A shed over the chunk's east edge: its ground over the edge stands in the neighbour's answer too, and the
    // neighbour is told it changed.
    thinned(
        geo,
        YEAR,
        vec![([4, 4], field(thinning::Kept::default(), Vec::new()))],
    );
    assert!(!stood(geo, period, [4020.0, 4520.0]));
    let before = answers(geo, period).unwrap().version();
    let mut over = thinning::Kept::default();
    over.set_ring(
        &[
            [960.0, 500.0],
            [1030.0, 500.0],
            [1030.0, 540.0],
            [960.0, 540.0],
        ],
        20.0,
    );
    thinned(geo, YEAR, vec![([3, 4], field(over, Vec::new()))]);
    assert!(stood(geo, period, [3990.0, 4520.0]) && stood(geo, period, [4020.0, 4520.0]));
    assert!(stood(geo, period, [4040.0, 4545.0]), "its margin");
    assert!(!stood(geo, period, [4100.0, 4520.0]) && !stood(geo, period, [4020.0, 4700.0]));
    assert!(changed_since(geo, period, before, [4, 4, 4, 4], false));
    assert!(thinned_since(geo, YEAR, before).is_some_and(|c| c.contains(&[4, 4])));
    // Kept no more (the chunk made again without its lift): its ground goes to the country.
    thinned(
        geo,
        YEAR,
        vec![([3, 4], field(thinning::Kept::default(), Vec::new()))],
    );
    assert!(answers(geo, period).unwrap().version() > first);
    assert!(!stood(geo, period, [3250.0, 4250.0]) && !stood(geo, period, [3700.0, 4750.0]));
    assert!(!stood(geo, period, [4020.0, 4520.0]));
}

/// The cover's estimate stands a listed yard whole from its year (`thinning::YARDS`), not before.
#[test]
fn the_estimate_stands_a_listed_yard_from_its_year() {
    let _g = lock();
    let yard = &thinning::YARDS[0];
    let (lat, lon, r) = yard.circles[0];
    let geo = LatLon { lat, lon };
    // Built up all about, a third of its texels.
    let town = |_: [f64; 2], texel: [i64; 2]| {
        if hash(texel[0], texel[1]) < 0.33 {
            cover::BUILT
        } else {
            cover::GRASS
        }
    };
    let maps = maps(256, 25.0, [-3200.0, -3200.0], town);
    for (year, stands) in [(yard.year, true), (yard.year - 1, false)] {
        let e = estimate(geo, year, &style(), &maps, [0.0, 0.0]);
        let (mut inside, mut held) = (0, 0);
        for cz in -2..2 {
            for cx in -2..2 {
                let c = e.get([cx, cz]).expect("built-up land");
                let corner = chunk_corner([cx, cz]);
                for k in 0..CELLS * CELLS {
                    let m = cell_middle(k);
                    if (corner[0] + f64::from(m[0])).hypot(corner[1] + f64::from(m[1]))
                        < f64::from(r) - 60.0
                    {
                        inside += 1;
                        held += usize::from(c.thinning.in_yard(m) && c.thinning.stood(m));
                    }
                }
            }
        }
        assert!(inside > 0, "{}", yard.name);
        assert_eq!(
            held,
            if stands { inside } else { 0 },
            "{} in {year}",
            yard.name
        );
    }
}

/// The cover's estimate stands a listed town's extent whole (`naval_terrain::buildings::towns::TOWNS`: Istanbul's
/// walled city, Galata and the rest) and takes what the towns keep out of the rest of their 5' cells, which here (the
/// towns keeping more than the history's share) keeps nothing.
#[test]
fn the_estimate_stands_a_listed_town_and_thins_the_rest_for_it() {
    let _g = lock();
    let geo = LatLon {
        lat: 41.0165,
        lon: 28.955,
    };
    // Built up all about, three texels in five (scoring well over the region's floor).
    let built = |_: [f64; 2], texel: [i64; 2]| {
        if hash(texel[0], texel[1]) < 0.6 {
            cover::BUILT
        } else {
            cover::GRASS
        }
    };
    let maps = maps(480, 25.0, [-6000.0, -6000.0], built);
    let e = estimate(geo, YEAR, &style(), &maps, [0.0, 0.0]);
    let towns = super::towns(geo, YEAR);
    let (mut inside, mut inside_stood, mut outside, mut outside_stood) = (0u32, 0u32, 0u32, 0u32);
    let mut agree = 0u32;
    // By 5' cell: the cells about the towns, of them stood, the towns' cells, and the cells about the towns farther
    // than `TOWN_EASE` inside the 5' cell's sides and of them stood.
    let mut by_cell: HashMap<[i32; 2], [u32; 5]> = HashMap::new();
    let inside_sides = |v: f64| {
        let f = v * HYDE_PER_DEGREE - (v * HYDE_PER_DEGREE).floor();
        f.min(1.0 - f)
    };
    for cz in e.first[1]..e.first[1] + e.size[1] as i32 {
        for cx in e.first[0]..e.first[0] + e.size[0] as i32 {
            let Some(c) = e.get([cx, cz]) else {
                continue;
            };
            let corner = chunk_corner([cx, cz]);
            for k in 0..CELLS * CELLS {
                let m = cell_middle(k);
                let p = [corner[0] + f64::from(m[0]), corner[1] + f64::from(m[1])];
                let stood = c.thinning.stood(m);
                agree += u32::from(towns.holds(p) == c.thinning.in_town(m));
                let at = to_lat_lon(geo, p);
                let t = by_cell.entry(hyde_cell(at.lat, at.lon)).or_default();
                if c.thinning.in_town(m) {
                    inside += 1;
                    inside_stood += u32::from(stood);
                    t[2] += 1;
                } else if !c.thinning.in_yard(m) {
                    outside += 1;
                    outside_stood += u32::from(stood);
                    t[0] += 1;
                    t[1] += u32::from(stood);
                    if inside_sides(at.lat).min(inside_sides(at.lon)) >= TOWN_EASE {
                        t[3] += 1;
                        t[4] += u32::from(stood);
                    }
                }
            }
        }
    }
    eprintln!(
        "in the towns {inside_stood} of {inside} cells stood, about them {outside_stood} of {outside} ({by_cell:?}); \
         {agree} agree"
    );
    assert!(inside > 2000 && outside > 2000, "{inside}, {outside}");
    assert!(agree * 100 >= (inside + outside) * 99, "{agree} agree");
    assert_eq!(inside_stood, inside, "the towns stood whole");
    // Where the towns hold more than a 5' cell's share, nothing else in it stands, but along its sides, where its bar is
    // eased in from its neighbours'.
    let mut full = 0;
    for (cell, [about, _, towns, deep, stood]) in &by_cell {
        let (own, around) = history_about(*cell, YEAR);
        let (lat, lon) = hyde_middle(*cell);
        let share = period_share(own, around, Region::at(lat, lon, &style()), YEAR);
        if *towns as f32 >= 1.5 * share * (*about + *towns) as f32 && *deep > 300 {
            full += 1;
            assert!(
                stood * 25 < *deep,
                "{cell:?}: {stood} of {deep} stood about the towns, inside the cell's sides"
            );
        }
    }
    assert!(full >= 2, "{full} cells the towns fill");
    // The marks leave the towns' built-up texels unmarked.
    let answers = hold_estimate(geo, YEAR, Arc::new(e));
    let grid = Grid::of_maps(&maps, [0.0, 0.0]);
    let mut classes = maps.cover.clone();
    mark(&answers, &grid, &mut classes);
    let n = maps.size as usize;
    let (mut town_marked, mut town_built) = (0, 0);
    for j in 0..n {
        for i in 0..n {
            let p = [
                grid.corner[0] + (i as f64 + 0.5) * grid.texel_m,
                grid.corner[1] + (j as f64 + 0.5) * grid.texel_m,
            ];
            if maps.cover[j * n + i] == cover::BUILT && towns.holds(p) {
                town_built += 1;
                town_marked += usize::from(classes[j * n + i] & cover::PAST_COUNTRY != 0);
            }
        }
    }
    assert!(town_built > 10_000, "{town_built}");
    // A texel whose middle is in a town but whose 25 m cell's middle is not may be marked, along the towns' edges.
    assert!(
        town_marked * 100 < town_built,
        "{town_marked} of {town_built} of the towns' built-up texels marked"
    );
}
