//! The spray a bow throws as she drives into a sea, as water marched through on the GPU (native, 2026-10-20). It
//! replaced the web game's plunge spray, which read as something else: billows of white cotton heaping into smoke over
//! her forecastle, puffs of mist hanging as round pale discs abaft her bridge for seconds, and strands standing over
//! her bow as a glassy arch (`native/docs/water-effects.md`, "Bow spray as water").
//!
//! A plunge drives the sea up her flare and throws it off as sheets that open outboard and up like wings, tear into
//! strands and drops as they fly, and fall back alongside her forecastle as she runs on; the finest of it hangs in the
//! air and the wind takes it aft. A plunge whose spray reaches over her deck edge throws its sheet up over her stem head
//! and back across the forecastle.
//!
//! Each throw is one [`BowBurst`]: a fan of launch velocities in the plane across her side (outboard and up; across her
//! deck and up, over the bow), drawn out along the stretch of her side that met the sea. Its water is where ballistics
//! puts it, as a splash's column's ([`crate::columns`]): slowed by a drag in proportion to its speed and keeping some of
//! her way, a launch velocity v is at v·T − (0, fall, 0) at time t, so the shader finds each point's launch back from
//! where it is (`column_field.wgsl`'s `burst_launch`), its streaks lie along the water's flight and it tears as it
//! spreads. Bursts are marched with the splashes' columns (`shaders/column.wgsl`), sorted among them, and lit as they
//! are. This file works out each one's flight once a frame and the box its water fills; the shaders hold the shapes.
//! The constants shared with `column_field.wgsl` are marked; keep the two in step.

use bytemuck::Zeroable;
use naval_effects::CameraView;
use naval_effects::math::smooth;
use std::f64::consts::PI;

use glam::{DVec2, DVec3};

use crate::columns::{ColumnGpu, KIND_BURST, flight};

/// The drag on a burst's water (1/s): torn sheets and drops, as the bow's sheets' (`bow_spray`'s `SHEET_DRAG`).
pub const BURST_DRAG: f64 = 0.6;
/// When its sheets tear into strands and drops (s).
const BREAKUP: [f64; 2] = [0.2, 1.3];
/// Its mist: shed this long after the throw (s), swelling (a share of itself a second from `MIST_SWELLS`) and settling
/// (m/s) as it hangs, held until `MIST_HOLD` and fading over `MIST_FADE` seconds; the wind carries it from
/// `MIST_DRIFTS` (s), when it has left the sheets.
const MIST_SHED: f64 = 0.45;
const MIST_SWELL: f64 = 0.5;
const MIST_SWELLS: f64 = 0.3;
const MIST_SETTLE: f64 = 0.35;
const MIST_HOLD: f64 = 0.9;
const MIST_FADE: f64 = 1.0;
const MIST_DRIFTS: [f64; 2] = [0.2, 0.8];
/// How long a burst lasts (s): its water is back in the sea and its mist gone.
pub const BURST_LIFE: f64 = 4.0;
/// How its stretch's ends soften as its water spreads fore and aft (m: the spread and this much more, over twice that
/// either side; `column_field.wgsl`'s `fan`).
const END_SOFT: f64 = 1.0;
/// How much later than its fore end the after end of the stretch threw its water (s; `column_field.wgsl`'s
/// `BURST_DELAY`).
const BURST_DELAY: f64 = 0.3;
/// How much faster than its fastest its lumps are thrown, how far round the fan's middle its water reaches (in its
/// widths) and how much wider its mist stands (`column_field.wgsl`: `BURST_LUMPS`, `FAN_REACH`, `MIST_WIDE`).
const BURST_LUMPS: f64 = 0.12;
const FAN_REACH: f64 = 2.2;
const MIST_WIDE: f64 = 1.6;
/// The most of the fan's speed its water is thrown at anywhere along the stretch (`column_field.wgsl`'s
/// `ALONG_LUMPS`): the box holds it.
const ALONG_LUMPS: f64 = 1.15;
/// The box's margin about its water (m), how far the noise tears the stretch's ends past them (m) and pushes its
/// mist's outline about (m, and more as it swells: `column_field.wgsl`'s `burst_water`).
const MARGIN: f64 = 1.5;
const ENDS_TORN: f64 = 2.0;
const MIST_PUSH: [f64; 2] = [1.25, 1.0];

/// The `k`th draw (0–1) from a burst's own number (as a splash's: splitmix64's finaliser).
fn draw(seed: f64, k: u64) -> f64 {
    let mut x = seed.to_bits() ^ k.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

/// The speed (m/s) to throw water straight up for it to climb `rise` metres under the bursts' drag: its height under a
/// drag k is w/k − (g/k²) ln(1 + k w / g), solved for w by Newton's method from the drag-free speed.
pub fn launch_for(rise: f64) -> f64 {
    let (k, g) = (BURST_DRAG, crate::columns::GRAVITY);
    let rise = rise.max(0.0);
    let mut w = (2.0 * g * rise).sqrt();
    for _ in 0..6 {
        let height = w / k - g / (k * k) * (1.0 + k * w / g).ln();
        let slope = (w / g) / (1.0 + k * w / g);
        if slope <= 1e-9 {
            break;
        }
        w = (w + (rise - height) / slope).max(0.0);
    }
    w
}

/// One throw of spray off a bow.
#[derive(Clone, Debug, PartialEq)]
pub struct BowBurst {
    /// Where its water left (world m): her side at the sea's level where the plunge met her flare, or her stem head.
    pub origin: DVec3,
    /// The fan's plane across her (world xz, unit): outboard (or to starboard, over her bow), and aft along her side.
    pub out: DVec2,
    pub aft: DVec2,
    /// Seconds since it was thrown.
    pub age: f64,
    /// The hull that threw it: a hidden hull's (the lens's own) is not drawn.
    pub source: u32,
    /// Its own number (0–1): where its strands and lumps lie.
    pub seed: f64,
    /// How much water it throws (0–1).
    pub strength: f64,
    /// Its fastest water's launch speed (m/s), the fan's middle (radians up from outboard: π/2 straight up) and half
    /// width (radians), and its slowest water's share of the fastest's.
    pub speed: f64,
    pub elevation: f64,
    pub width: f64,
    pub slowest: f64,
    /// Her speed (m/s) and the share of it her thrown water keeps going ahead, and how far its water's speed
    /// fore and aft spreads about that (m/s): it draws out along her side.
    pub way: f64,
    pub carry: f64,
    pub scatter: f64,
    /// The stretch of her side that threw it: from `lead` ahead of the origin to `length` aft of it (m).
    pub lead: f64,
    pub length: f64,
    /// How far below the origin its water falls before the sea takes it (m).
    pub below: f64,
    /// How much of it is mist the wind takes (0–1).
    pub mist: f64,
    /// How far the wind has carried its water and its mist (world xz, m).
    pub drift: DVec2,
    pub mist_drift: DVec2,
}

impl BowBurst {
    pub fn alive(&self) -> bool {
        self.age < BURST_LIFE
    }

    /// Its water's flight: travel T and fall (m) under its drag.
    pub fn flight(&self) -> (f64, f64) {
        flight(self.age, BURST_DRAG)
    }

    /// How much mist hangs (0–1 of its own).
    pub fn mist_left(&self) -> f64 {
        let t = self.age;
        smooth((t - 0.1) / 0.45)
            * (-(t - MIST_HOLD).max(0.0) / MIST_FADE).exp()
            * (1.0 - smooth((t - (BURST_LIFE - 0.8)) / 0.8))
            * self.mist
    }

    /// The highest any of its water stands over the origin now (m): its lumps' at the fan's steepest reach.
    pub fn top(&self) -> f64 {
        let (travel, fall) = self.flight();
        let (low, high) = (
            self.elevation - self.width * FAN_REACH,
            self.elevation + self.width * FAN_REACH,
        );
        let steepest = if low <= PI * 0.5 && high >= PI * 0.5 {
            1.0
        } else {
            low.sin().max(high.sin())
        };
        self.speed * (1.0 + BURST_LUMPS) * ALONG_LUMPS * steepest * travel - fall
    }

    /// How high its fastest water `widths` of the fan's width above its middle stands over the origin now (m): the
    /// sheet's top as the eye sees it (a width up it is a third as dense as at its middle).
    pub fn height(&self, widths: f64) -> f64 {
        let (travel, fall) = self.flight();
        self.speed * (self.elevation + self.width * widths).min(PI * 0.5).sin() * travel - fall
    }

    /// A point in the box's frame (along `out`, up, along `aft`; m from the origin) in the world.
    fn local(&self, v: DVec2) -> DVec2 {
        DVec2::new(v.dot(self.out), v.dot(self.aft))
    }

    /// Its flight at its age, and the box its water fills, as the columns' march reads it (`column_field.wgsl`'s
    /// `Burst`: the lanes of a splash's record, read as below).
    pub fn state(&self) -> ColumnGpu {
        let t = self.age;
        let (travel, fall) = flight(t, BURST_DRAG);
        let breakup = smooth((t - BREAKUP[0]) / (BREAKUP[1] - BREAKUP[0]));
        let carried = self.carry * self.way * travel;
        let spread = self.scatter * travel;
        let drift = self.local(self.drift);
        // The mist: the fan as it stood when the mist left it, swelling, settling and carried off on the wind.
        let shed = t.min(MIST_SHED);
        let (mist_travel, mist_fall) = flight(shed, BURST_DRAG);
        let swelling = MIST_SWELL * (t - MIST_SWELLS).max(0.0);
        let settled = MIST_SETTLE * (t - MIST_SHED).max(0.0);
        let mist_carried = self.carry * self.way * mist_travel;
        let mist_spread = self.scatter * mist_travel * (1.0 + swelling);
        let mist_drift = self.local(self.mist_drift);
        let mist = self.mist_left();
        let mut low = DVec3::splat(f64::MAX);
        let mut high = DVec3::splat(f64::MIN);
        // Its water: the fan's reach at its fastest and slowest, round its width (`wide` times it), fore and aft over
        // the stretch and its softened ends, and `push` metres more all round.
        let mut fan = |travel: f64,
                       fall: f64,
                       drift: DVec2,
                       reach: f64,
                       wide: f64,
                       z: [f64; 2],
                       push: f64| {
            for k in 0..=12 {
                let angle =
                    self.elevation + self.width * wide * FAN_REACH * (f64::from(k) / 6.0 - 1.0);
                for share in [0.0, reach] {
                    let v = self.speed * share;
                    let (x, y) = (
                        v * angle.cos() * travel + drift.x,
                        v * angle.sin() * travel - fall,
                    );
                    low = low.min(DVec3::new(
                        x - push,
                        (y - push).max(-self.below),
                        z[0] - push,
                    ));
                    high = high.max(DVec3::new(x + push, y + push, z[1] + push));
                }
            }
        };
        let reach = (1.0 + BURST_LUMPS) * ALONG_LUMPS;
        if self.top() > -self.below {
            // The water aft along the stretch left up to `BURST_DELAY` later: younger, it may stand higher.
            for k in 0..=4 {
                let age = t - BURST_DELAY * f64::from(k) / 4.0;
                if age <= 0.0 {
                    break;
                }
                let (travel, fall) = flight(age, BURST_DRAG);
                let carried = self.carry * self.way * travel;
                let soft = 2.0 * (spread + END_SOFT) + ENDS_TORN;
                let z = [
                    -self.lead - soft - carried + drift.y,
                    self.length + soft - carried + drift.y,
                ];
                fan(travel, fall, drift, reach, 1.0, z, MARGIN);
            }
        }
        if mist > 0.002 {
            let swell = 1.0 + swelling;
            let soft = 2.0 * (mist_spread + END_SOFT);
            let z = [
                -self.lead - soft - mist_carried + mist_drift.y,
                self.length + soft - mist_carried + mist_drift.y,
            ];
            let push = MARGIN + MIST_PUSH[0] + MIST_PUSH[1] * swelling;
            fan(
                mist_travel * swell,
                mist_fall + settled,
                mist_drift,
                reach * 1.1,
                MIST_WIDE,
                z,
                push,
            );
        }
        if low.x > high.x {
            // Nothing left: an empty box at the origin.
            low = DVec3::ZERO;
            high = DVec3::ZERO;
        } else {
            low.y = low.y.min(-self.below);
        }
        // The box's frame is a turn about the vertical (its faces wound as a splash's): its z is `out` turned a right
        // angle, which runs aft along her starboard side and forward along her port side. `along` turns the burst's
        // own lengths, which run aft, into it.
        let (z, along) = self.frame_z();
        if along < 0.0 {
            (low.z, high.z) = (-high.z, -low.z);
        }
        let f = |v: f64| v as f32;
        ColumnGpu {
            origin: [f(self.origin.x), f(self.origin.y), f(self.origin.z), 1.0],
            low: [f(low.x), f(low.y), f(low.z), f(t)],
            high: [f(high.x), f(high.y), f(high.z), f(self.seed)],
            way: [f(drift.x), f(drift.y), f(mist_drift.x), f(mist_drift.y)],
            flight: [f(travel), f(fall), f(carried), f(breakup)],
            veil: [f(mist_travel), f(mist_fall), f(swelling), f(mist)],
            crown: [f(settled), f(self.lead), f(self.length), f(mist_carried)],
            surge: [
                f(self.speed),
                f(self.elevation),
                f(self.width),
                f(self.strength),
            ],
            tint: [1.0, 1.0, 1.0, 1.0],
            shape: [f(spread), f(mist_spread), f(self.slowest), f(self.below)],
            frame: [f(self.out.x), f(self.out.y), f(z.x), f(z.y)],
            kind: [KIND_BURST, f(along), 0.0, 0.0],
            ..ColumnGpu::zeroed()
        }
    }

    /// The box frame's z (world xz, unit: `out` turned a right angle, so the frame is a turn, not a mirror) and which way
    /// it runs along `aft` (1 with it, −1 against it).
    pub fn frame_z(&self) -> (DVec2, f64) {
        let z = DVec2::new(-self.out.y, self.out.x);
        (z, if z.dot(self.aft) < 0.0 { -1.0 } else { 1.0 })
    }
}

/// The bows' bursts in the air: a bounded list, the oldest given up when it is full.
pub struct BowBursts {
    bursts: Vec<BowBurst>,
    capacity: usize,
    published: Vec<(f64, ColumnGpu)>,
}

impl BowBursts {
    pub fn new(capacity: usize) -> Self {
        Self {
            bursts: Vec::with_capacity(capacity),
            capacity,
            published: Vec::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn bursts(&self) -> &[BowBurst] {
        &self.bursts
    }

    /// Bursts in view last frame, farthest first, each with its squared distance from the eye.
    pub fn instances(&self) -> &[(f64, ColumnGpu)] {
        &self.published
    }

    /// Throw `burst`, its age and drift reset; `seed` draws what differs between throws (how its strands lie).
    pub fn throw(&mut self, mut burst: BowBurst) {
        if self.bursts.len() >= self.capacity
            && let Some(oldest) = self
                .bursts
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.age.total_cmp(&b.1.age))
                .map(|(i, _)| i)
        {
            self.bursts.swap_remove(oldest);
        }
        burst.age = 0.0;
        burst.drift = DVec2::ZERO;
        burst.mist_drift = DVec2::ZERO;
        self.bursts.push(burst);
    }

    /// Age every burst by `dt`; the wind (m/s, world) carries their water and their mist.
    pub fn advance(&mut self, dt: f64, wind: DVec3) {
        let wind = DVec2::new(wind.x, wind.z);
        for burst in &mut self.bursts {
            burst.age += dt;
            let t = burst.age;
            // Slowed toward the air by its drag, the water comes to move with the wind (the exact drift of a drag in
            // proportion to the speed through the air); the mist moves with it once it has left the sheets.
            burst.drift += wind * (dt * (1.0 - (-BURST_DRAG * t).exp()));
            burst.mist_drift +=
                wind * (dt * smooth((t - MIST_DRIFTS[0]) / (MIST_DRIFTS[1] - MIST_DRIFTS[0])));
        }
        self.bursts.retain(BowBurst::alive);
    }

    /// The frame's bursts in view, farthest first, but `hidden`'s.
    pub fn publish(&mut self, camera: &CameraView, hidden: Option<u32>) {
        self.published.clear();
        let (p0, p5) = (camera.projection.x_axis.x, camera.projection.y_axis.y);
        for burst in &self.bursts {
            if Some(burst.source) == hidden {
                continue;
            }
            let state = burst.state();
            if state.high[0] <= state.low[0] {
                continue;
            }
            let (center, radius) = state.bounds();
            let view = camera.view.transform_point3(camera.drawn(center));
            let depth = -view.z;
            if depth + radius < 0.0
                || view.x.abs() > depth.max(0.0) / p0 + radius * 1.5
                || view.y.abs() > depth.max(0.0) / p5 + radius * 1.5
            {
                continue;
            }
            self.published
                .push(((center - camera.position).length_squared(), state));
        }
        self.published.sort_by(|a, b| b.0.total_cmp(&a.0));
    }

    pub fn reset(&mut self) {
        self.bursts.clear();
        self.published.clear();
    }
}

/// A burst as it is thrown, before [`BowBursts::throw`] sets it going: everything zero but its frame and what [`draw`]
/// gives its number.
pub fn thrown(seed: f64) -> BowBurst {
    BowBurst {
        origin: DVec3::ZERO,
        out: DVec2::X,
        aft: DVec2::Y,
        age: 0.0,
        source: 0,
        seed: draw(seed, 1),
        strength: 0.0,
        speed: 0.0,
        elevation: PI * 0.25,
        width: 0.3,
        slowest: 0.2,
        way: 0.0,
        carry: 0.0,
        scatter: 0.0,
        lead: 0.0,
        length: 0.0,
        below: 2.0,
        mist: 0.0,
        drift: DVec2::ZERO,
        mist_drift: DVec2::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::GRAVITY;

    fn side_burst() -> BowBurst {
        BowBurst {
            origin: DVec3::new(10.0, 2.0, -50.0),
            out: DVec2::X,
            aft: DVec2::Y,
            strength: 1.0,
            speed: 19.0,
            elevation: 0.75,
            width: 0.3,
            slowest: 0.18,
            way: 16.0,
            carry: 0.3,
            scatter: 4.5,
            lead: 6.0,
            length: 7.0,
            below: 2.5,
            mist: 0.8,
            ..thrown(0.37)
        }
    }

    #[test]
    fn a_burst_rises_to_her_deck_and_falls_back_into_the_sea() {
        let mut bursts = BowBursts::new(4);
        bursts.throw(side_burst());
        let (mut middle, mut edge): (f64, f64) = (0.0, 0.0);
        let mut back = None;
        for frame in 0..300 {
            bursts.advance(1.0 / 60.0, DVec3::ZERO);
            let Some(burst) = bursts.bursts().first() else {
                break;
            };
            middle = middle.max(burst.height(0.0));
            edge = edge.max(burst.height(1.0));
            if back.is_none() && frame > 30 && burst.top() < -burst.below {
                back = Some(f64::from(frame) / 60.0);
            }
        }
        // Thrown at 19 m/s at 43°, the fan's middle stands some 6 m over where it left and its steep edge 8 to 9 m (a
        // destroyer's deck edge over her crest, and a few metres more); none of it stands as high as it would without
        // the air's drag. Even its fastest lumps, thrown straight up, are back in the sea within its life, and the whole
        // of it, mist and all, is gone by its life's end.
        assert!(middle > 4.5 && middle < 7.5, "{middle}");
        assert!(edge > 7.0 && edge < 10.5, "{edge}");
        let without_drag = (19.0 * 1.05_f64.sin()).powi(2) / (2.0 * GRAVITY);
        assert!(edge < 0.8 * without_drag, "{edge} {without_drag}");
        let back = back.expect("back in the sea");
        assert!(back > 1.8 && back < BURST_LIFE, "{back}");
        assert!(bursts.bursts().is_empty());
    }

    #[test]
    fn as_she_runs_on_her_spray_falls_astern_of_her_bow() {
        // Her stem has run 16 m/s on; her water keeps 0.3 of her way, slowed by its drag: after a second it lies some
        // 10 m astern of her stem, alongside her forecastle.
        let mut burst = side_burst();
        burst.age = 1.0;
        let state = burst.state();
        let (travel, _) = burst.flight();
        let carried = f64::from(state.flight[2]);
        assert!((carried - 0.3 * 16.0 * travel).abs() < 1e-4);
        let astern = 16.0 * 1.0 - carried;
        assert!(astern > 8.0 && astern < 14.0, "{astern}");
        // The box holds it: its far end lies at least that far astern of where it left (the frame's z runs aft).
        assert!(f64::from(state.low[2]) < -burst.lead - carried + 1.0);
        assert!(f64::from(state.high[2]) > burst.length - carried);
    }

    #[test]
    fn the_box_holds_the_fan_and_follows_its_mist_downwind() {
        let mut bursts = BowBursts::new(4);
        bursts.throw(side_burst());
        for _ in 0..120 {
            // A wind blowing aft along her side.
            bursts.advance(1.0 / 60.0, DVec3::new(0.0, 0.0, 12.0));
        }
        let burst = &bursts.bursts()[0];
        assert!(
            burst.mist_drift.y > 12.0 && burst.mist_drift.y < 24.0,
            "{}",
            burst.mist_drift.y
        );
        assert!(burst.drift.y < burst.mist_drift.y);
        let state = burst.state();
        let (travel, fall) = burst.flight();
        // Its fastest water across the fan's middle lies inside the box.
        let x = burst.speed * burst.elevation.cos() * travel + f64::from(state.way[0]);
        let y = burst.speed * burst.elevation.sin() * travel - fall;
        assert!(
            x < f64::from(state.high[0]) && x > f64::from(state.low[0]),
            "{x} {state:?}"
        );
        assert!(y < f64::from(state.high[1]), "{y} {state:?}");
        // And the mist, carried aft and fading.
        assert!(f64::from(state.high[2]) > burst.mist_drift.y, "{state:?}");
        assert!(state.veil[3] > 0.0 && state.veil[3] < burst.mist as f32);
    }

    #[test]
    fn a_burst_lies_along_her_side_in_the_world() {
        // Heading north-east (her bow toward −z, +x): her starboard and port bursts' frames turned with her, each a turn
        // about the vertical (the box's faces wound as a splash's), its lengths running aft along her side.
        let forward = DVec2::new(1.0, -1.0).normalize();
        let aft = -forward;
        for side in [1.0, -1.0] {
            let mut burst = side_burst();
            burst.aft = aft;
            burst.out = DVec2::new(-forward.y, forward.x) * side;
            burst.age = 0.5;
            let state = burst.state();
            let [ax, az, bx, bz] = state.frame.map(f64::from);
            // Right-handed: x × y = z.
            assert!(
                (DVec3::new(ax, 0.0, az).cross(DVec3::Y) - DVec3::new(bx, 0.0, bz)).length() < 1e-6
            );
            assert_eq!(f64::from(state.kind[1]), burst.frame_z().1);
            let along =
                state.world(DVec3::new(0.0, 0.0, 10.0 * f64::from(state.kind[1]))) - burst.origin;
            assert!(
                (DVec2::new(along.x, along.z) - aft * 10.0).length() < 1e-4,
                "{side} {along}"
            );
            let outboard = state.world(DVec3::new(5.0, 0.0, 0.0)) - burst.origin;
            assert!((DVec2::new(outboard.x, outboard.z) - burst.out * 5.0).length() < 1e-4);
            assert!(state.low[2] < state.high[2]);
            let (center, radius) = state.bounds();
            assert!((center - burst.origin).length() < radius + 1e-6);
        }
    }

    /// `column_field.wgsl`'s burst field mirrored without its noise, at the noise's worst at every point (its lumps
    /// thrown fastest, its fibres widest, its stretch's ends torn furthest, its mist pushed furthest): its water and
    /// its mist's at `p`, in the box's frame.
    fn worst_water(state: &ColumnGpu, p: DVec3) -> (f64, f64) {
        let g = |a: [f32; 4]| a.map(f64::from);
        let [_, _, _, t] = g(state.low);
        let [dx, dz, mx, mz] = g(state.way);
        let [travel, _, carried, _] = g(state.flight);
        let [mist_travel, mist_fall, swelling, mist] = g(state.veil);
        let [settled, lead, length, mist_carried] = g(state.crown);
        let [speed, elevation, width, strength] = g(state.surge);
        let [spread, mist_spread, slowest, below] = g(state.shape);
        let along = f64::from(state.kind[1]);
        let q = DVec3::new(p.x, p.y, p.z * along);
        if q.y < -below {
            return (0.0, 0.0);
        }
        let ramp = |a: f64, b: f64, x: f64| smooth((x - a) / (b - a));
        let profile = |share: f64| {
            ramp(slowest * 0.4, slowest, share)
                * (1.0 - ramp(0.78, 1.0, share))
                * (1.3 + (0.5 - 1.3) * share.clamp(0.0, 1.0))
        };
        // The window along the stretch at its noise's worst: the most it holds anywhere within the tear.
        let window = |at: f64, spread: f64, torn: f64| {
            let soft = spread + END_SOFT;
            (0..=8)
                .map(|k| at + torn * (f64::from(k) / 4.0 - 1.0))
                .map(|z| {
                    ramp(-lead - 2.0 * soft, -lead + soft, z)
                        * (1.0 - ramp(length - soft, length + 2.0 * soft, z))
                })
                .fold(0.0, f64::max)
        };
        let mut water = 0.0;
        if travel > 1e-3 {
            let left = q.z - dz + carried;
            let age = t - BURST_DELAY * ((left + lead) / (lead + length).max(1.0)).clamp(0.0, 1.0);
            if age > 0.0 {
                let (travel, fall) = flight(age, BURST_DRAG);
                let carried = carried / f64::from(state.flight[0]).max(1e-3) * travel;
                let w = DVec2::new(q.x - dx, q.y + fall) / travel.max(1e-3);
                let share = w.length() / (speed * ALONG_LUMPS);
                let angle = (w.y.atan2(w.x) - elevation) / width;
                if angle.abs() < FAN_REACH && share < 1.0 + BURST_LUMPS {
                    let worst = (share - BURST_LUMPS).max(0.0);
                    let best = (0..=8)
                        .map(|k| profile(worst + (share - worst) * f64::from(k) / 8.0))
                        .fold(0.0, f64::max);
                    let across = (-(angle.abs() - 0.3).max(0.0).powi(2)).exp();
                    let stretch = (lead + length).max(1.0);
                    let mass = 0.12 * strength / (travel * travel).max(0.02) * stretch
                        / (stretch + 1.5 * spread);
                    water =
                        mass * across * best * window(q.z - dz + carried, spread, ENDS_TORN) * 1.45;
                }
            }
        }
        let mut haze: f64 = 0.0;
        if mist > 0.0 {
            let swell = 1.0 + swelling;
            let push = MIST_PUSH[0] + MIST_PUSH[1] * swelling;
            for (i, j, k) in (0..27).map(|n| (n % 3, n / 3 % 3, n / 9)) {
                let o = DVec3::new(q.x - mx, q.y + settled, q.z - mz)
                    + DVec3::new(f64::from(i) - 1.0, f64::from(j) - 1.0, f64::from(k) - 1.0) * push;
                let w = DVec2::new(o.x, o.y + mist_fall) / (mist_travel * swell).max(1e-3);
                let share = w.length() / speed / 1.1;
                let angle = (w.y.atan2(w.x) - elevation) / width / MIST_WIDE;
                let shape = (-angle * angle).exp()
                    * profile(share)
                    * window(o.z + mist_carried, mist_spread, 0.0);
                haze = haze.max(0.05 * mist / (swell * swell) * (shape * 1.6).min(1.0));
            }
        }
        (water, haze)
    }

    #[test]
    fn no_water_lies_outside_its_box() {
        // A destroyer's plunge off her flare and a sheet over her bow, in a gale blowing across her and aft, on her
        // starboard and port sides, through their lives: just outside every face of the box, nothing the eye could see
        // (the field's noise at its worst), where inside the water stands thick.
        let over = BowBurst {
            elevation: PI * 0.5,
            width: 0.38,
            slowest: 0.25,
            carry: 0.55,
            scatter: 0.35 * 14.0,
            speed: 14.0,
            lead: 1.7,
            length: 5.0,
            below: 8.0,
            ..side_burst()
        };
        for (burst, side) in [(side_burst(), 1.0), (side_burst(), -1.0), (over, 1.0)] {
            let mut bursts = BowBursts::new(2);
            bursts.throw(BowBurst {
                out: burst.out * side,
                ..burst
            });
            let mut thickest: f64 = 0.0;
            for frame in 0..230 {
                bursts.advance(1.0 / 60.0, DVec3::new(9.0, 0.0, 14.0));
                if frame % 10 != 0 {
                    continue;
                }
                let Some(burst) = bursts.bursts().first() else {
                    break;
                };
                let state = burst.state();
                let low = DVec3::new(
                    f64::from(state.low[0]),
                    f64::from(state.low[1]),
                    f64::from(state.low[2]),
                );
                let high = DVec3::new(
                    f64::from(state.high[0]),
                    f64::from(state.high[1]),
                    f64::from(state.high[2]),
                );
                let size = high - low;
                for n in 0..20 * 20 * 20 {
                    let at = DVec3::new(
                        f64::from(n % 20),
                        f64::from(n / 20 % 20),
                        f64::from(n / 400),
                    ) / 19.0;
                    let inside = low + size * at;
                    thickest = thickest.max(worst_water(&state, inside).0);
                    // Out through the nearest face.
                    let mut outside = inside;
                    let faces = [at.x, 1.0 - at.x, at.y, 1.0 - at.y, at.z, 1.0 - at.z];
                    let (face, _) = faces
                        .iter()
                        .enumerate()
                        .fold((0, f64::MAX), |b, (i, &d)| if d < b.1 { (i, d) } else { b });
                    let axis = face / 2;
                    let beyond = if face % 2 == 0 {
                        low[axis] - 0.05
                    } else {
                        high[axis] + 0.05
                    };
                    outside[axis] = beyond;
                    if axis == 1 && face == 2 {
                        // Under the box's floor the sea takes it.
                        continue;
                    }
                    let (water, haze) = worst_water(&state, outside);
                    assert!(
                        water < 0.004 && haze < 0.004,
                        "{side}: {water} {haze} at {outside} ({frame}: {state:?})"
                    );
                }
            }
            assert!(thickest > 0.5, "{side}: {thickest}");
        }
    }

    #[test]
    fn a_throw_climbs_as_high_as_it_was_thrown_for() {
        for rise in [0.5, 3.0, 9.7, 14.0] {
            let w = launch_for(rise);
            let (k, g) = (BURST_DRAG, GRAVITY);
            let height = w / k - g / (k * k) * (1.0 + k * w / g).ln();
            assert!(
                (height - rise).abs() < 1e-3 * rise.max(1.0),
                "{rise}: {w} {height}"
            );
            assert!(w > (2.0 * g * rise).sqrt());
        }
    }

    #[test]
    fn a_full_list_gives_up_its_oldest_and_bursts_publish_in_view() {
        let mut bursts = BowBursts::new(2);
        bursts.throw(side_burst());
        bursts.advance(0.5, DVec3::ZERO);
        bursts.throw(BowBurst {
            origin: DVec3::new(0.0, 2.0, -60.0),
            ..side_burst()
        });
        bursts.advance(0.2, DVec3::ZERO);
        bursts.throw(BowBurst {
            origin: DVec3::new(-10.0, 2.0, -70.0),
            ..side_burst()
        });
        let ages: Vec<f64> = bursts.bursts().iter().map(|b| b.age).collect();
        assert_eq!(ages.len(), 2);
        assert!(ages.iter().all(|&a| a < 0.5), "{ages:?}");
        let camera = CameraView::look_at(
            DVec3::new(60.0, 10.0, -20.0),
            DVec3::new(0.0, 2.0, -65.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        bursts.advance(0.3, DVec3::ZERO);
        bursts.publish(&camera, None);
        assert_eq!(bursts.instances().len(), 2);
        assert!(bursts.instances()[0].0 >= bursts.instances()[1].0);
        let away = CameraView::look_at(
            DVec3::new(60.0, 10.0, -20.0),
            DVec3::new(200.0, 2.0, 100.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        bursts.publish(&away, None);
        assert!(bursts.instances().is_empty());
        bursts.publish(&camera, Some(0));
        assert!(bursts.instances().is_empty());
    }
}
