//! A hull's floating (`naval_afloat::motion`): a hull the sea cannot carry refused, at rest at her draft in a calm, heaving with a long swell, a long hull
//! pitching less than a short one in short waves, her motion settling once the sea goes, and the same motion at any
//! frame rate with nothing drifting over minutes.

use glam::DVec3;
use naval_afloat::{Afloat, Calm, Course, Floating, Funnel, Hull, SeaSurface, Stir, swell};
use naval_ocean::calibration::{NORTH_ATLANTIC, scene_parameters};

/// A destroyer's and a battleship's waterlines.
fn destroyer() -> Hull {
    Hull::new(110.0, 11.0, 3.6)
}

fn battleship() -> Hull {
    Hull::new(250.0, 36.0, 10.0)
}

/// One long-crested sine: `amplitude` m, `wavelength` m, running toward (cos d, sin d).
struct Sine {
    amplitude: f64,
    wavelength: f64,
    direction: f64,
}

impl SeaSurface for Sine {
    fn height(&self, x: f64, z: f64, time: f64) -> f64 {
        let k = std::f64::consts::TAU / self.wavelength;
        let omega = (9.81 * k).sqrt();
        self.amplitude
            * (k * (x * self.direction.cos() + z * self.direction.sin()) - omega * time).sin()
    }

    fn shortest_wave(&self) -> Option<f64> {
        Some(self.wavelength)
    }
}

/// A sea tilted (`slope` per metre toward +x) and raised (`rise` m) until `until` s, then flat.
struct Released {
    slope: f64,
    rise: f64,
    until: f64,
}

impl SeaSurface for Released {
    fn height(&self, x: f64, _: f64, time: f64) -> f64 {
        if time < self.until {
            self.rise + self.slope * x
        } else {
            0.0
        }
    }
}

/// A small deterministic stream in 0 … 1.
fn stream(seed: u64) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Lying still, heading 0 (her bow toward −z).
const STILL: Course = Course {
    x: 0.0,
    z: 0.0,
    heading: 0.0,
    speed: 0.0,
};

/// A circle of `radius` m about the origin at `speed` m/s, `time` s in.
fn circling(radius: f64, speed: f64, time: f64) -> Course {
    let angle = speed * time / radius;
    // Counter-clockwise seen from above in x, z: her heading the tangent's, clockwise from −z.
    let (sin, cos) = angle.sin_cos();
    let (vx, vz) = (-sin, cos);
    Course {
        x: radius * cos,
        z: radius * sin,
        heading: vx.atan2(-vz),
        speed,
    }
}

#[test]
fn periods_follow_her_size() {
    let small = destroyer().periods();
    let large = battleship().periods();
    // A destroyer heaves and pitches in 4–6 s and rolls in about 10; a battleship in 8–10 and about 18.
    assert!((4.0..6.5).contains(&small.heave), "{small:?}");
    assert!((4.0..6.5).contains(&small.pitch), "{small:?}");
    assert!((9.0..11.0).contains(&small.roll), "{small:?}");
    assert!((7.0..10.5).contains(&large.heave), "{large:?}");
    assert!((7.0..10.5).contains(&large.pitch), "{large:?}");
    assert!((16.0..20.0).contains(&large.roll), "{large:?}");
}

#[test]
fn a_hull_the_sea_cannot_carry_is_refused() {
    let good = destroyer();
    let bad = [
        Hull::new(0.0, 11.0, 3.6),
        Hull::new(110.0, -11.0, 3.6),
        Hull::new(110.0, 11.0, f64::NAN),
        Hull::new(f64::INFINITY, 11.0, 3.6),
        Hull {
            displacement: Some(0.0),
            ..good.clone()
        },
        Hull {
            full_speed: Some(f64::NAN),
            ..good.clone()
        },
        Hull {
            centre: [0.0, f64::INFINITY],
            ..good.clone()
        },
        Hull {
            funnels: vec![Funnel::round(DVec3::new(0.0, 12.0, f64::NAN), 2.6)],
            ..good.clone()
        },
        Hull {
            funnels: vec![Funnel {
                smoke: f64::NAN,
                ..Funnel::round(DVec3::new(0.0, 12.0, -10.0), 2.6)
            }],
            ..good.clone()
        },
        Hull {
            funnels: vec![Funnel {
                smoke: 1.5,
                ..Funnel::round(DVec3::new(0.0, 12.0, -10.0), 2.6)
            }],
            ..good.clone()
        },
    ];
    for hull in bad {
        assert!(hull.check().is_err(), "{hull:?}");
        assert!(Afloat::new("bad", hull.clone()).is_err(), "{hull:?}");
        assert!(Floating::new(hull.clone()).is_err(), "{hull:?}");
        assert!(Stir::new("bad", &hull).is_err(), "{hull:?}");
    }
    // What is put afloat has finite periods.
    let periods = Afloat::new("good", good).unwrap().hull().periods();
    for period in [periods.heave, periods.pitch, periods.roll] {
        assert!(period.is_finite() && period > 0.0, "{periods:?}");
    }
}

#[test]
fn rests_at_her_draft_in_a_calm() {
    for speed in [0.0, 12.0] {
        let mut hull = Afloat::new("calm", destroyer()).unwrap();
        let mut next = stream(7);
        let mut time = 0.0;
        while time < 600.0 {
            let course = Course {
                z: -speed * time,
                speed,
                ..STILL
            };
            hull.update(course, &Calm, time);
            let pose = hull.pose();
            assert_eq!((pose.y, pose.pitch, pose.roll), (0.0, 0.0, 0.0), "{time} s");
            assert_eq!((pose.x, pose.z), (course.x, course.z));
            time += 1.0 / 240.0 + next() * (1.0 / 15.0 - 1.0 / 240.0);
        }
    }
}

#[test]
fn heave_follows_a_long_swell() {
    // A 600 m swell (a 19.6 s period) 2 m high, from ahead and from abeam: a destroyer rises and falls with it.
    for direction in [std::f64::consts::FRAC_PI_2, 0.0] {
        let sea = Sine {
            amplitude: 2.0,
            wavelength: 600.0,
            direction,
        };
        let mut hull = Afloat::new("swell", destroyer()).unwrap();
        let (mut error, mut height) = (0.0, 0.0);
        let dt = 1.0 / 60.0;
        for frame in 0..(120.0 / dt) as usize {
            let time = frame as f64 * dt;
            hull.update(STILL, &sea, time);
            if time > 30.0 {
                let under = sea.height(0.0, 0.0, time);
                error += (hull.motion().heave - under).powi(2);
                height += under * under;
            }
        }
        let miss = (error / height).sqrt();
        assert!(miss < 0.1, "heave misses the swell by {miss} of its height");
    }
}

#[test]
fn a_long_hull_pitches_less_in_short_waves() {
    // A 50 m sea 1 m high from ahead: a 60 m hull pitches through it, a 250 m one hardly at all.
    let sea = Sine {
        amplitude: 1.0,
        wavelength: 50.0,
        direction: -std::f64::consts::FRAC_PI_2,
    };
    let pitching = |hull: Hull| {
        let mut hull = Afloat::new("short", hull).unwrap();
        let (mut pitch, mut heave, mut n) = (0.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        for frame in 0..(90.0 / dt) as usize {
            let time = frame as f64 * dt;
            hull.update(STILL, &sea, time);
            if time > 30.0 {
                pitch += hull.motion().pitch.powi(2);
                heave += hull.motion().heave.powi(2);
                n += 1.0;
            }
        }
        ((pitch / n).sqrt(), (heave / n).sqrt())
    };
    let short = pitching(Hull::new(60.0, 7.0, 2.4));
    let long = pitching(battleship());
    assert!(short.0 > 0.01, "a 60 m hull pitches {} rad", short.0);
    assert!(
        long.0 < 0.2 * short.0,
        "pitch: long {} short {}",
        long.0,
        short.0
    );
    assert!(
        long.1 < 0.3 * short.1,
        "heave: long {} short {}",
        long.1,
        short.1
    );
}

#[test]
fn her_motion_settles_once_the_sea_goes() {
    for hull in [destroyer(), battleship()] {
        let periods = hull.periods();
        // Heeled 6°, trimmed and lifted a metre by a tilted sea that goes flat at 10 s.
        let slope = 6f64.to_radians().tan();
        let sea = Released {
            slope,
            rise: 1.0,
            until: 10.0,
        };
        let mut afloat = Afloat::new("released", hull).unwrap();
        let dt = 1.0 / 60.0;
        let mut rolls = Vec::new();
        let mut crossings = Vec::new();
        let mut last = 0.0;
        let end = 10.0 + 12.0 * periods.roll;
        let mut frame = 0;
        loop {
            let time = frame as f64 * dt;
            if time > end {
                break;
            }
            afloat.update(STILL, &sea, time);
            let motion = afloat.motion();
            if time < 10.0 {
                // Settled on the tilted sea.
                assert!((motion.roll - slope.atan()).abs() < 1e-6, "{motion:?}");
                assert!((motion.heave - 1.0).abs() < 1e-6, "{motion:?}");
            } else {
                rolls.push((time, motion.roll));
                if last > 0.0 && motion.roll <= 0.0 || last < 0.0 && motion.roll >= 0.0 {
                    crossings.push(time);
                }
            }
            last = motion.roll;
            frame += 1;
        }
        let first = slope.atan();
        let tail = afloat.motion();
        assert!(tail.roll.abs() < 0.01 * first, "{tail:?}");
        assert!(
            tail.heave.abs() < 1e-3 && tail.pitch.abs() < 1e-5,
            "{tail:?}"
        );
        // She rolls at her natural period (zero crossings half a period apart) …
        let half = (crossings[3] - crossings[1]) / 2.0;
        let damped = periods.roll / 2.0 / (1.0f64 - 0.08 * 0.08).sqrt();
        assert!(
            (half / damped - 1.0).abs() < 0.05,
            "{half} s against {damped} s"
        );
        // … and each roll is smaller than the last by at least her linear damping's decrement.
        let peak = |from: f64, to: f64| {
            rolls
                .iter()
                .filter(|(t, _)| (from..to).contains(t))
                .fold(0.0f64, |m, (_, r)| m.max(r.abs()))
        };
        let a = peak(crossings[0], crossings[1]);
        let b = peak(crossings[2], crossings[3]);
        let linear = (-std::f64::consts::TAU * 0.08 / (1.0f64 - 0.08 * 0.08).sqrt()).exp();
        assert!(
            b / a < linear * 1.01 && b / a > 0.3,
            "{a} then {b}: {} against {linear}",
            b / a
        );
    }
}

#[test]
fn the_same_at_any_frame_rate_and_no_drift() {
    // A destroyer circling 500 m about the origin at 12 m/s for five minutes in a moderate sea.
    let sea = swell(&scene_parameters(&NORTH_ATLANTIC, 9.0, false).waves);
    let run = |frames: &mut dyn FnMut() -> f64| {
        let mut hull = Afloat::new("circling", destroyer()).unwrap();
        let mut samples = Vec::new();
        let (mut time, mut offset, mut n) = (0.0f64, 0.0, 0.0);
        loop {
            let course = circling(500.0, 12.0, time);
            hull.update(course, &sea, time);
            let pose = hull.pose();
            assert_eq!(
                (pose.x, pose.z, pose.heading),
                (course.x, course.z, course.heading)
            );
            offset += hull.motion().heave - hull.floating().sea_under().height;
            n += 1.0;
            if time == time.round() {
                samples.push((time, hull.motion()));
            }
            if time >= 300.0 {
                break;
            }
            // Every run draws a frame at each whole second, whatever its frames between.
            let next = time + frames();
            time = if next.floor() > time.floor() {
                time.floor() + 1.0
            } else {
                next
            };
        }
        (samples, offset / n)
    };
    type Frames = Box<dyn FnMut() -> f64>;
    let mut rates: Vec<(&str, Frames)> = vec![
        ("60", Box::new(|| 1.0 / 60.0)),
        ("20", Box::new(|| 1.0 / 20.0)),
        ("30", Box::new(|| 1.0 / 30.0)),
        ("144", Box::new(|| 1.0 / 144.0)),
        ("uneven", {
            let mut next = stream(11);
            Box::new(move || 1.0 / 200.0 + next() * (1.0 / 12.0 - 1.0 / 200.0))
        }),
    ];
    let mut reference: Option<Vec<(f64, naval_afloat::Motion)>> = None;
    for (name, frames) in &mut rates {
        let (samples, offset) = run(frames.as_mut());
        // Her heave stays about the sea's level under her: nothing builds up between them.
        assert!(
            offset.abs() < 0.01,
            "{name}: heave {offset} m off the sea's level on average"
        );
        let Some(reference) = &reference else {
            reference = Some(samples);
            continue;
        };
        assert_eq!(samples.len(), reference.len(), "{name}");
        let (mut heave, mut angle) = (0.0f64, 0.0f64);
        for ((t, a), (_, b)) in reference.iter().zip(&samples) {
            heave = heave.max((a.heave - b.heave).abs());
            angle = angle
                .max((a.pitch - b.pitch).abs())
                .max((a.roll - b.roll).abs());
            assert!(
                (a.heave - b.heave).abs() < 1e-3
                    && (a.pitch - b.pitch).abs() < 1e-4
                    && (a.roll - b.roll).abs() < 1e-4,
                "{name} at {t} s: {b:?} against {a:?}"
            );
        }
        eprintln!(
            "{name} frames a second: within {heave:.1e} m and {angle:.1e} rad of 60's over five minutes"
        );
    }
}

#[test]
fn a_hull_that_appears_underway_is_already_working() {
    // One hull sailing since 0 s, another first seen at 120 s on the same course: within a degree and 20 cm.
    let sea = swell(&scene_parameters(&NORTH_ATLANTIC, 15.0, false).waves);
    let course = |time: f64| Course {
        x: 30.0,
        z: -10.0 * time,
        heading: 0.0,
        speed: 10.0,
    };
    let mut sailing = Afloat::new("sailing", destroyer()).unwrap();
    let dt = 1.0 / 60.0;
    let mut frame = 0;
    while frame as f64 * dt < 120.0 {
        sailing.update(course(frame as f64 * dt), &sea, frame as f64 * dt);
        frame += 1;
    }
    let time = frame as f64 * dt;
    sailing.update(course(time), &sea, time);
    let mut appearing = Afloat::new("appearing", destroyer()).unwrap();
    appearing.update(course(time), &sea, time);
    let (a, b) = (sailing.motion(), appearing.motion());
    assert!((a.roll - b.roll).abs() < 1f64.to_radians(), "{a:?} {b:?}");
    assert!(
        (a.pitch - b.pitch).abs() < 0.5f64.to_radians(),
        "{a:?} {b:?}"
    );
    assert!((a.heave - b.heave).abs() < 0.2, "{a:?} {b:?}");
}
