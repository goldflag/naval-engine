//! The cover's test without `atan2` ([`within`]) against `gap` itself, and a kept cover ([`Kept`]) against a fresh one
//! wherever it says it holds.
use super::*;

/// A deterministic stream of numbers in [0, 1) (SplitMix64).
struct Stream(u64);
impl Stream {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    /// A place: anywhere, or on a whole degree of latitude or longitude (a square's edge), or near a pole.
    fn place(&mut self) -> Geo {
        let lat = match (self.next() * 4.0) as u32 {
            0 => (self.next() * 178.0 - 89.0).round(),
            1 => 89.0 - self.next() * 2.0,
            _ => self.next() * 178.0 - 89.0,
        };
        let lon = match (self.next() * 4.0) as u32 {
            0 => (self.next() * 360.0 - 180.0).round(),
            1 => 179.0 + self.next() * 2.0,
            _ => self.next() * 360.0 - 180.0,
        };
        Geo::degrees(lat, lon)
    }
    /// A cap's radius, metres: a query's few hundred metres to a body's few hundred kilometres, and past.
    fn radius(&mut self) -> f64 {
        10f64.powf(2.0 + self.next() * 4.5)
    }
}

/// `p` moved `m` metres along the great circle leaving it on the course `course` (radians from north).
fn travel(p: &Geo, course: f64, m: f64) -> Geo {
    let v = p.vector();
    let east = {
        let e = [-v[1], v[0], 0.0];
        let k = 1.0 / dot(e, e).sqrt();
        [e[0] * k, e[1] * k, 0.0]
    };
    let north = cross(v, east);
    let (s, c) = course.sin_cos();
    let t = add(scale(north, c), scale(east, s));
    let (sin, cos) = (m / R).sin_cos();
    Geo::of(add(scale(v, cos), scale(t, sin)))
}

/// `within` is `gap`'s answer for every square near a cap, at random radii and at radii on the gap itself and a
/// rounding either side of it, where its products tie and it asks `gap`.
#[test]
fn within_answers_as_gap_does() {
    let mut s = Stream(7);
    let (mut asked, mut ties) = (0u64, 0u64);
    for _ in 0..4_000 {
        let c = s.place();
        let r = s.radius().min(5_000_000.0);
        let range = Candidates::of(&c, (r + 50_000.0) / R);
        for sq in range.each().take(400) {
            let g = gap(&c, sq);
            for rho in [
                r / R,
                g,
                g + 1e-15,
                g - 1e-15,
                g + 2e-12,
                g - 2e-12,
                g * (1.0 + 1e-9),
            ] {
                let probe = Probe::new(rho);
                assert_eq!(
                    within(&c, sq, &probe),
                    gap(&c, sq) <= rho,
                    "{c:?} {sq:?} ρ {rho} gap {g}"
                );
                asked += 1;
                ties += u64::from(rho == g);
            }
        }
    }
    assert!(asked > 1_000_000 && ties > 100_000, "{asked} {ties}");
}

/// A kept cover is a fresh cover wherever it says it holds: its centre moved and its radius changed by up to its
/// hold, in every direction, and at the hold's edge.
#[test]
fn a_kept_cover_is_a_fresh_cover_wherever_it_holds() {
    let mut s = Stream(11);
    let (mut checked, mut held) = (0u64, 0u64);
    for _ in 0..3_000 {
        let c = s.place();
        let r = 1_000.0 + s.next() * 400_000.0;
        let kept = Kept::cap(&c, r, 5_000.0);
        assert_eq!(kept.squares(), cap(&c, r), "{c:?} {r}");
        if kept.hold_m() < 0.0 {
            continue;
        }
        held += 1;
        assert!(kept.hold_m() <= 5_000.0);
        for k in 0..24 {
            let share = if k < 4 { 1.0 } else { s.next() };
            let budget = kept.hold_m() * if k % 3 == 0 { 1.0 } else { s.next() };
            let moved = budget * share;
            let grown = (budget - moved) * if s.next() < 0.5 { -1.0 } else { 1.0 };
            let to = travel(&c, s.next() * std::f64::consts::TAU, moved);
            if !kept.holds(to.vector(), r + grown) {
                // The travel's own rounding put it a hair past the hold.
                continue;
            }
            assert_eq!(
                kept.squares(),
                cap(&to, r + grown),
                "{c:?} r {r} moved {moved} grown {grown} hold {}",
                kept.hold_m()
            );
            checked += 1;
        }
    }
    assert!(held > 2_000 && checked > 40_000, "{held} {checked}");
}

/// A kept cover's hold reaches the nearest square edge: a cap moved a little past it, toward that edge, meets other
/// squares (the hold is not loose).
#[test]
fn a_kept_covers_hold_reaches_the_nearest_square_edge() {
    // 100 km about a place whose cap's south edge lies 900 m short of the 9° N parallel, well inside its column: that
    // parallel is the nearest square edge it crosses (the next, its corners, lie 10 km and more away).
    let r = 100_000.0;
    let metres_per_degree = R * PI / 180.0;
    let c = Geo::degrees(9.0 + (r + 900.0) / metres_per_degree, 20.5);
    let kept = Kept::cap(&c, r, 5_000.0);
    let expected = 900.0 - COVER_SLACK_M - KEEP_SLACK_M;
    assert!(
        (kept.hold_m() - expected).abs() < 1e-3,
        "{} {expected}",
        kept.hold_m()
    );
    let past = travel(&c, PI, expected + 1.0);
    assert!(!kept.holds(past.vector(), r));
    assert_ne!(cap(&past, r), kept.squares());
    let short = travel(&c, PI, expected - 1.0);
    assert!(kept.holds(short.vector(), r));
    assert_eq!(cap(&short, r), kept.squares());
}

/// One set takes the squares each footprint would take alone.
#[test]
fn one_set_is_every_covers_union() {
    let mut s = Stream(13);
    let mut one = Cover::new();
    for _ in 0..300 {
        one.clear();
        let mut union = BTreeSet::new();
        for _ in 0..(1 + (s.next() * 12.0) as usize) {
            let c = s.place();
            let r = s.radius().min(3_000_000.0);
            if s.next() < 0.7 {
                one.cap(&c, r);
                union.extend(cap(&c, r));
            } else {
                let b = travel(&c, s.next() * 6.0, s.next() * 300_000.0);
                let w = r.min(60_000.0);
                one.corridor(&c, &b, w);
                union.extend(corridor(&c, &b, w));
            }
        }
        assert_eq!(one.iter().collect::<BTreeSet<_>>(), union);
        assert_eq!(one.len(), union.len());
    }
    one.clear();
    one.cap(&Geo::degrees(0.0, 0.0), 2.1e7);
    assert_eq!(one.len(), Square::COUNT);
    assert!(one.iter().eq(Square::all()));
}
