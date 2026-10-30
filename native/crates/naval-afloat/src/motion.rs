//! Floating, visual only: a hull's heave, pitch and roll on the sea under her, about the course her host steers.
//!
//! Each of the three is a damped oscillator driven toward the sea's level under her waterplane: the plane fitted to the
//! sea's heights at points over it ([`Waterplane`]), every station weighted by her waterplane's breadth there, and
//! enough stations for the sea's shortest wave. The fit is what averages the waves away: under a long hull a short
//! sea's crests and troughs cancel and she hardly pitches, and a swell many times her length lifts her bodily. The
//! oscillators' natural periods are her hull's ([`Hull::periods`]); each is damped on its motion relative to that
//! level (strip theory's relative motion: the water she makes waves in moves with the sea), so a long swell carries
//! her with it without lagging, heave and pitch by the waves she makes ([`HEAVE_DAMPING`], [`PITCH_DAMPING`]), roll as
//! the simulation damps it (`roll_decay`: lightly at rest, more with her way, a quadratic term for a large roll), so
//! a beam sea near her roll period works her up and she settles over a few rolls once it goes.
//!
//! The host moves her: her x, z, heading and speed each frame ([`Course`]). The motion steps on its own clock in
//! fixed [`STEP`]s, the course drawn straight between the frames' (and on at its own speed past the last) and the sea
//! read at each step's time, and the pose is the motion between the two steps about the frame's time; so she moves the
//! same at 20 frames a second as at 144,
//! and nothing accumulates between her course and her pose: her heave is about the sea's level, never her own last
//! one. A clock that jumps or goes back finds her settled where she is ([`Floating::settle`]): her motion of the last
//! two of her longest periods on her present course, so a hull that appears underway is already working in the sea.

use glam::DVec3;
use naval_effects::ShipPose;

use crate::hull::{BadHull, Hull};
use crate::sea::SeaSurface;

/// The motion's own step (s).
pub const STEP: f64 = 1.0 / 120.0;
/// A frame further than this from the last (s), or before it, settles her afresh rather than stepping through.
pub const CATCH_UP: f64 = 2.0;
/// Her motion's past a settle steps through: this many of her longest natural periods, no more than
/// [`SETTLE_MAX`] seconds.
pub const SETTLE_PERIODS: f64 = 2.0;
pub const SETTLE_MAX: f64 = 40.0;
/// Heave and pitch damping, fractions of critical: the waves a heaving, pitching hull makes carry its energy away, a
/// quarter to a third of critical about her natural periods.
pub const HEAVE_DAMPING: f64 = 0.3;
pub const PITCH_DAMPING: f64 = 0.3;
/// Roll damping, as the simulation's (`roll_decay` in `crates/naval-sim/src/flotation/stability.rs`): 0.08 of
/// critical at rest (bilge keels), her way's lift adding 0.25 per unit of her Froude number, and eddies off her bilge
/// keels a decay of 0.35 per second per radian a second of roll rate.
pub const ROLL_DAMPING_AT_REST: f64 = 0.08;
pub const ROLL_DAMPING_PER_FROUDE: f64 = 0.25;
pub const ROLL_DAMPING_QUADRATIC: f64 = 0.35;
/// The sea's level moving faster than this (m/s for heave, rad/s for pitch and roll) between two steps is a sea
/// changed (a new wind, a sea handed over), not a sea moving: her damping does not chase it.
pub const LEVEL_RATE_LIMIT: (f64, f64) = (4.0, 0.4);
/// Stations along her waterplane: three to the sea's shortest wave, within these.
pub const STATIONS: (usize, usize) = (5, 24);
const PER_WAVE: f64 = 3.0;
/// Across, points at this share of her half-breadth either side of her centre line, and one on it.
const ACROSS: f64 = 0.7;

/// Where the host has her this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Course {
    /// World x and z (m) of her origin.
    pub x: f64,
    pub z: f64,
    /// Heading (radians), clockwise from above, 0 steaming toward −z.
    pub heading: f64,
    /// Speed along her bow (m/s), astern negative.
    pub speed: f64,
}

impl Course {
    /// Unit vectors (world x, z) along her bow and to her starboard.
    pub fn axes(&self) -> ((f64, f64), (f64, f64)) {
        let (sin, cos) = self.heading.sin_cos();
        ((sin, -cos), (cos, sin))
    }

    /// Her world velocity (m/s, level).
    pub fn velocity(&self) -> DVec3 {
        let ((fx, fz), _) = self.axes();
        DVec3::new(fx * self.speed, 0.0, fz * self.speed)
    }

    /// The course `t` of the way from `self` to `next` (0 … 1): straight between them, the heading the short way round.
    pub fn toward(&self, next: &Course, t: f64) -> Course {
        let turn = (next.heading - self.heading + std::f64::consts::PI)
            .rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        Course {
            x: self.x + (next.x - self.x) * t,
            z: self.z + (next.z - self.z) * t,
            heading: self.heading + turn * t,
            speed: self.speed + (next.speed - self.speed) * t,
        }
    }

    /// The course `seconds` on at her speed and heading.
    pub fn ahead(&self, seconds: f64) -> Course {
        let ((fx, fz), _) = self.axes();
        Course {
            x: self.x + fx * self.speed * seconds,
            z: self.z + fz * self.speed * seconds,
            ..*self
        }
    }
}

/// Her heave (m: her waterline's centre of flotation over the mean sea), pitch (radians, bow up), roll (radians,
/// starboard side up), and their rates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Motion {
    pub heave: f64,
    pub pitch: f64,
    pub roll: f64,
    pub heave_rate: f64,
    pub pitch_rate: f64,
    pub roll_rate: f64,
}

impl Motion {
    fn lerp(&self, other: &Motion, t: f64) -> Motion {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        Motion {
            heave: mix(self.heave, other.heave),
            pitch: mix(self.pitch, other.pitch),
            roll: mix(self.roll, other.roll),
            heave_rate: mix(self.heave_rate, other.heave_rate),
            pitch_rate: mix(self.pitch_rate, other.pitch_rate),
            roll_rate: mix(self.roll_rate, other.roll_rate),
        }
    }
}

/// The sea's level under her waterplane: the plane fitted to its heights there.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeaUnder {
    /// Its height (m) at her waterplane's centre of flotation.
    pub height: f64,
    /// Its rise per metre toward her bow and toward her starboard side.
    pub along: f64,
    pub across: f64,
}

impl SeaUnder {
    /// The level she would lie at on it: her heave, pitch and roll.
    pub fn level(&self) -> (f64, f64, f64) {
        (self.height, self.along.atan(), self.across.atan())
    }
}

/// The points her waterplane reads the sea at: per station along her, three across, each weighted by its share of
/// the waterplane's area; and the sums of the plane's fit.
#[derive(Clone, Debug, PartialEq)]
pub struct Waterplane {
    /// (forward of the waterplane's centre, to starboard, weight): metres, metres, a share of the whole.
    points: Vec<(f64, f64, f64)>,
    /// Her centre of flotation's distance forward of the waterplane's centre (m).
    flotation: f64,
    /// Σ w (u − ū)² and Σ w v², over the whole weight.
    along_moment: f64,
    across_moment: f64,
    stations: usize,
}

impl Waterplane {
    /// `hull`'s waterplane read at `stations` stations, her proxy's (`Hull::shape`) breadths.
    pub fn new(hull: &Hull, stations: usize) -> Self {
        let shape = hull.shape();
        let stations = stations.max(2);
        let length = hull.length.max(0.1);
        let mut points = Vec::with_capacity(stations * 3);
        for i in 0..stations {
            // Ship z (aft) of the station's middle; forward is −z.
            let z = (-0.5 + (i as f64 + 0.5) / stations as f64) * length;
            let half = f64::from(shape.half_breadth(z as f32, 0.0).unwrap_or(0.0));
            for side in [-1.0, 0.0, 1.0] {
                points.push((-z, side * ACROSS * half, half));
            }
        }
        let total: f64 = points.iter().map(|p| p.2).sum::<f64>().max(1e-9);
        for p in &mut points {
            p.2 /= total;
        }
        let flotation: f64 = points.iter().map(|p| p.0 * p.2).sum();
        let along_moment = points
            .iter()
            .map(|p| p.2 * (p.0 - flotation).powi(2))
            .sum::<f64>()
            .max(1e-9);
        let across_moment = points
            .iter()
            .map(|p| p.2 * p.1 * p.1)
            .sum::<f64>()
            .max(1e-9);
        Self {
            points,
            flotation,
            along_moment,
            across_moment,
            stations,
        }
    }

    /// The stations `hull` needs for a sea whose shortest wave is `shortest` (m): three to it, within [`STATIONS`].
    pub fn stations_for(hull: &Hull, shortest: Option<f64>) -> usize {
        match shortest {
            Some(wave) if wave.is_infinite() => STATIONS.0,
            Some(wave) if wave > 0.0 => {
                ((PER_WAVE * hull.length / wave).ceil() as usize).clamp(STATIONS.0, STATIONS.1)
            }
            _ => STATIONS.1,
        }
    }

    pub fn stations(&self) -> usize {
        self.stations
    }

    /// Her centre of flotation's distance forward of the waterplane's centre (m).
    pub fn flotation(&self) -> f64 {
        self.flotation
    }

    /// Every point's world (x, z) for a waterplane centred at `centre` with her bow along `forward` and her starboard
    /// along `starboard` (unit, world x and z), with its weight.
    pub fn points(
        &self,
        centre: (f64, f64),
        forward: (f64, f64),
        starboard: (f64, f64),
    ) -> impl Iterator<Item = (f64, f64, f64)> + '_ {
        self.points.iter().map(move |&(u, v, w)| {
            (
                centre.0 + forward.0 * u + starboard.0 * v,
                centre.1 + forward.1 * u + starboard.1 * v,
                w,
            )
        })
    }

    /// The plane fitted to `sea`'s heights at `time` over the waterplane so placed.
    pub fn sea_under(
        &self,
        sea: &impl SeaSurface,
        centre: (f64, f64),
        forward: (f64, f64),
        starboard: (f64, f64),
        time: f64,
    ) -> SeaUnder {
        self.fit(
            self.points(centre, forward, starboard)
                .map(|(x, z, _)| sea.height(x, z, time)),
        )
    }

    /// The plane fitted to `heights`, one for each point in order.
    pub fn fit(&self, heights: impl Iterator<Item = f64>) -> SeaUnder {
        let (mut mean, mut along, mut across) = (0.0, 0.0, 0.0);
        for (&(u, v, w), h) in self.points.iter().zip(heights) {
            mean += w * h;
            along += w * (u - self.flotation) * h;
            across += w * v * h;
        }
        SeaUnder {
            height: mean,
            along: along / self.along_moment,
            across: across / self.across_moment,
        }
    }
}

/// One hull's floating: her motion on her own clock, the course it was stepped along and the sea's sampling.
#[derive(Clone, Debug)]
pub struct Floating {
    hull: Hull,
    /// Natural angular frequencies (rad/s): heave, pitch, roll.
    frequencies: (f64, f64, f64),
    waterplane: Waterplane,
    /// The motion at the last step's time and the step before it.
    current: Motion,
    previous: Motion,
    /// The last step's time (s); none before the first frame.
    clock: Option<f64>,
    /// The last frame's course and time.
    course: Course,
    course_time: f64,
    /// The motion at the last frame's time, and the sea under her at the last step.
    shown: Motion,
    under: SeaUnder,
}

impl Floating {
    /// `hull` afloat (her pose is the world's origin until her first update); refused unless [`Hull::check`] passes.
    pub fn new(hull: Hull) -> Result<Self, BadHull> {
        hull.check()?;
        let waterplane = Waterplane::new(&hull, STATIONS.1);
        Ok(Self {
            frequencies: hull.frequencies(),
            hull,
            waterplane,
            current: Motion::default(),
            previous: Motion::default(),
            clock: None,
            course: Course::default(),
            course_time: 0.0,
            shown: Motion::default(),
            under: SeaUnder::default(),
        })
    }

    pub fn hull(&self) -> &Hull {
        &self.hull
    }

    pub fn waterplane(&self) -> &Waterplane {
        &self.waterplane
    }

    /// Her motion at the last frame's time.
    pub fn motion(&self) -> Motion {
        self.shown
    }

    /// The sea's level under her at the last step.
    pub fn sea_under(&self) -> SeaUnder {
        self.under
    }

    /// The course of the last frame.
    pub fn course(&self) -> Course {
        self.course
    }

    /// Step her to `time` (s, the clock the sea is drawn at) on `course`, riding `sea`. A first frame, or one more than
    /// [`CATCH_UP`] from the last or before it, settles her ([`Floating::settle`]).
    pub fn update(&mut self, course: Course, sea: &impl SeaSurface, time: f64) {
        self.resample(sea);
        let Some(mut clock) = self.clock else {
            return self.settle(course, sea, time);
        };
        if time > clock + CATCH_UP || time < clock - STEP - 1e-9 {
            return self.settle(course, sea, time);
        }
        let (from, since) = (self.course, self.course_time);
        let span = time - since;
        while clock < time - 1e-12 {
            let next = clock + STEP;
            // Between the frames, straight from the last's course to this one's; past this one, on at its speed.
            let along = if next > time {
                course.ahead(next - time)
            } else if span > 1e-12 {
                from.toward(&course, ((next - since) / span).clamp(0.0, 1.0))
            } else {
                course
            };
            self.previous = self.current;
            self.current = self.step(self.current, &along, sea, next, STEP);
            clock = next;
        }
        self.clock = Some(clock);
        self.course = course;
        self.course_time = time;
        let t = 1.0 - (clock - time) / STEP;
        self.shown = self.previous.lerp(&self.current, t.clamp(0.0, 1.0));
    }

    /// Settle her at `time` on `course`: her motion of the last [`SETTLE_PERIODS`] of her longest natural period
    /// (to [`SETTLE_MAX`] s), stepped from rest on the sea's level along her course run back that far. In a calm she
    /// lies exactly at her draft, level.
    pub fn settle(&mut self, course: Course, sea: &impl SeaSurface, time: f64) {
        self.resample(sea);
        let (heave, pitch, roll) = self.frequencies;
        let longest = std::f64::consts::TAU / heave.min(pitch).min(roll).max(1e-3);
        let steps = ((SETTLE_PERIODS * longest).min(SETTLE_MAX) / STEP).ceil() as usize;
        let ((fx, fz), _) = course.axes();
        let back = |seconds: f64| Course {
            x: course.x - fx * course.speed * seconds,
            z: course.z - fz * course.speed * seconds,
            ..course
        };
        let start = time - steps as f64 * STEP;
        let first = back(steps as f64 * STEP);
        self.under = self.under_at(&first, sea, start);
        let (heave, pitch, roll) = self.under.level();
        let mut motion = Motion {
            heave,
            pitch,
            roll,
            ..Motion::default()
        };
        let mut previous = motion;
        for k in 1..=steps {
            previous = motion;
            let at = start + k as f64 * STEP;
            motion = self.step(motion, &back(time - at), sea, at, STEP);
        }
        self.previous = previous;
        self.current = motion;
        self.shown = motion;
        self.clock = Some(time);
        self.course = course;
        self.course_time = time;
    }

    /// The pose of her origin for her motion `motion` on `course`: her waterline's centre of flotation at her heave.
    pub fn pose_of(&self, motion: &Motion, course: &Course) -> ShipPose {
        let mut pose = ShipPose {
            x: course.x,
            y: 0.0,
            z: course.z,
            heading: course.heading,
            pitch: motion.pitch,
            roll: motion.roll,
        };
        // Her centre of flotation in ship space (z aft), lifted to her heave.
        let flotation = DVec3::new(
            self.hull.centre[0],
            0.0,
            self.hull.centre[1] - self.waterplane.flotation,
        );
        pose.y = motion.heave - pose.rotate(flotation).y;
        pose
    }

    /// Her pose at the last frame's time.
    pub fn pose(&self) -> ShipPose {
        self.pose_of(&self.shown, &self.course)
    }

    /// Her world velocity at the last frame's time (m/s): her course's, and her heave's.
    pub fn velocity(&self) -> DVec3 {
        self.course.velocity() + DVec3::Y * self.shown.heave_rate
    }

    /// Enough stations for the sea's shortest wave.
    fn resample(&mut self, sea: &impl SeaSurface) {
        let stations = Waterplane::stations_for(&self.hull, sea.shortest_wave());
        if stations != self.waterplane.stations() {
            self.waterplane = Waterplane::new(&self.hull, stations);
        }
    }

    /// The sea's level under her waterplane on `course` at `time`.
    fn under_at(&self, course: &Course, sea: &impl SeaSurface, time: f64) -> SeaUnder {
        let (forward, starboard) = course.axes();
        let [cx, cz] = self.hull.centre;
        // The waterplane's centre: her origin and her centre's offset in ship space (z aft).
        let centre = (
            course.x + starboard.0 * cx - forward.0 * cz,
            course.z + starboard.1 * cx - forward.1 * cz,
        );
        self.waterplane
            .sea_under(sea, centre, forward, starboard, time)
    }

    /// `motion` one step `dt` on, to `time`, on `course`: each degree driven toward the sea's level under her, then
    /// its rate relative to that level's damped over the step.
    fn step(
        &mut self,
        motion: Motion,
        course: &Course,
        sea: &impl SeaSurface,
        time: f64,
        dt: f64,
    ) -> Motion {
        let (was_heave, was_pitch, was_roll) = self.under.level();
        self.under = self.under_at(course, sea, time);
        let (to_heave, to_pitch, to_roll) = self.under.level();
        let level_rate = |from: f64, to: f64, limit: f64| {
            let rate = (to - from) / dt;
            if rate.abs() > limit { 0.0 } else { rate }
        };
        let (heave, pitch, roll) = self.frequencies;
        let froude = self.hull.froude(course.speed);
        let degree = |x: f64, rate: f64, (target, moving): (f64, f64), omega: f64, decay: f64| {
            let kept = (-decay * dt).exp();
            let rate = (rate + omega * omega * (target - x) * dt - moving) * kept + moving;
            (x + rate * dt, rate)
        };
        let (h, hr) = degree(
            motion.heave,
            motion.heave_rate,
            (
                to_heave,
                level_rate(was_heave, to_heave, LEVEL_RATE_LIMIT.0),
            ),
            heave,
            2.0 * HEAVE_DAMPING * heave,
        );
        let (p, pr) = degree(
            motion.pitch,
            motion.pitch_rate,
            (
                to_pitch,
                level_rate(was_pitch, to_pitch, LEVEL_RATE_LIMIT.1),
            ),
            pitch,
            2.0 * PITCH_DAMPING * pitch,
        );
        let rolling = level_rate(was_roll, to_roll, LEVEL_RATE_LIMIT.1);
        let roll_decay = 2.0 * (ROLL_DAMPING_AT_REST + ROLL_DAMPING_PER_FROUDE * froude) * roll
            + ROLL_DAMPING_QUADRATIC * (motion.roll_rate - rolling).abs();
        let (r, rr) = degree(
            motion.roll,
            motion.roll_rate,
            (to_roll, rolling),
            roll,
            roll_decay,
        );
        Motion {
            heave: h,
            pitch: p,
            roll: r,
            heave_rate: hr,
            pitch_rate: pr,
            roll_rate: rr,
        }
    }
}
