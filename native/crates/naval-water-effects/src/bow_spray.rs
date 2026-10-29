//! White water off the bows of hulls underway (`BowSpray` in the web game's `BowSpray.ts`). The crest climbing each side
//! from the stem throws a thin sheet that overturns outboard and plunges into the sea close aboard, astern of the stem,
//! tearing into strands and drops as it flies; the seas a bow meets make it surge. A bow plunging into one throws it off
//! her flare in sheets and, when it reaches over her deck edge, up over her stem head and back across her forecastle,
//! the finest of it downwind. Both are water marched through on the GPU with the splashes' columns and lit as they are
//! (2026-10-20): each side's sheet ([`crate::bow_sheets`]) and each plunge's bursts ([`crate::bow_bursts`]); since
//! 2026-10-21 each sea she meets under way throws a lighter burst off her flare too, its mist carried aft on the wind.
//! The web game drew them as sprites that read as something else: its strands along the sheet's flight as a glassy arch
//! over the crest, the billows heaped on the crest and the tufts flying with the sheet as cotton, a plunge's billows
//! and mist as smoke, and its drops (thrown off the sheets, out of a plunge, over the bow and off the deck) as large
//! pale discs close by: the bow throws no sprites now. The spray rises from the crest the bow waves draw, from the hull's own
//! sliced sides ([`WaterlineProfile`]), and fades into the sea where it stands. Visual only.
//!
//! Low, not a fountain (2026-10-12): the sheet is the water climbing the side, so it rises only as far as the crest
//! does. Its water leaves the crest a little over it ([`BowSprayTuning::rise`]), still climbing at the speed that
//! carries it to the crest's peak ([`Bow::climb_at`]): from the stem it climbs aft to the peak and falls outboard of
//! the crest line; from the peak it only falls. The web game threw every strand up at 0.45 of the hull's speed from a
//! crest already at its full height at the stem: a fan arching 2 m over the crest and 8 m out. A sea met throws it
//! higher, the water it threw then running aft along the sheet ([`BowSheet::kicks`]).
//!
//! What a bow ships over her forecastle ([`Aboard`], 2026-10-07): the sea's head on her stem (the sea drawn against it,
//! which the host reads back from the ocean's heights about the hulls into [`BowSpray::drawn_sea`], else the sea she
//! rides with the bow wave's crest on it; and the climb a plunge drives up the flare) against her deck edge decides
//! whether a plunge throws a sheet up and back over the forecastle, and whether green water comes over the deck edge and
//! runs off it afterwards; the apparent wind along her bow carries the spray aft. The ships lane wets her deck from it
//! (`naval_ships::Fleet::wet`).
//!
//! When a plunge throws ([`Peak`]): as it passes its height, for the bow's slam (her stem driving into the sea she
//! rides) and for the sheet over her bow, at most once in [`BURST_BETWEEN`] seconds; the burst's water is the height's.
//! A sea she meets throws as it kicks her sheets up (every few seconds, as heavy as the sea is rough), unless a plunge
//! throws then.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{DMat4, DQuat, DVec2, DVec3};
use naval_ocean::bow_waves::{BowWaveTuning, crest_peak, crest_profile, entrance_slope};
use naval_ocean::hull_profile::{
    PROFILE_LEVEL_STEP, PROFILE_LEVELS, PROFILE_LOWEST, PROFILE_STATIONS, WaterlineProfile,
};

use crate::bow_bursts::{BowBurst, BowBursts, launch_for, thrown};
use crate::bow_sheets::{BowSheet, BowSheets, CLOCK_WRAP, KICK_AGE};
use crate::fixture::{HullInfo, SeaWaves, ShipPose};
use naval_effects::CameraView;
use naval_effects::instances::WaterSprite;
use naval_effects::particles::{ParticlePool, PoolKind};
use naval_effects::random::{Lcg, to_uint32};

const GRAVITY: f64 = 9.81;

/// Live-tunable spray (`BOW_SPRAY_TUNING`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BowSprayTuning {
    pub start: f64,
    pub full: f64,
    /// The sheets' lip: how high they stand over the sea where they leave the crest, as a share of its height there.
    pub rise: f64,
    /// How fast the sheets leave the crest outboard, as a share of the hull's speed: a little faster than the crest
    /// line itself leaves her track, so they come down on its outer face.
    pub flare: f64,
    pub carry: f64,
    /// How much water the sheets throw at full speed (0–1).
    pub sheet: f64,
    pub surge_every: f64,
    /// What a sea met adds to the sheets' upward speed at its hardest, as a share of the hull's speed.
    pub surge_rise: f64,
    pub plunge_start: f64,
    pub plunge_full: f64,
}

impl Default for BowSprayTuning {
    fn default() -> Self {
        Self {
            start: 5.0,
            full: 15.0,
            rise: 1.1,
            flare: 0.45,
            carry: 0.15,
            sheet: 0.85,
            surge_every: 3.0,
            surge_rise: 0.2,
            plunge_start: 1.0,
            plunge_full: 4.0,
        }
    }
}

/// The bow waves' crest shape the spray rises from: the part of `naval_ocean::bow_waves::BowWaveTuning` it reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrestShape {
    pub crest: f64,
    pub speed_scale: f64,
    pub hump: f64,
    pub crest_angle: f64,
    pub crest_peak: f64,
}

impl Default for CrestShape {
    fn default() -> Self {
        Self::from(&BowWaveTuning::default())
    }
}

impl From<&BowWaveTuning> for CrestShape {
    fn from(tuning: &BowWaveTuning) -> Self {
        Self {
            crest: tuning.crest,
            speed_scale: tuning.speed_scale,
            hump: tuning.hump,
            crest_angle: tuning.crest_angle,
            crest_peak: tuning.crest_peak,
        }
    }
}

impl CrestShape {
    /// The bow waves' tuning with this shape in it.
    fn tuning(&self) -> BowWaveTuning {
        BowWaveTuning {
            crest: self.crest,
            speed_scale: self.speed_scale,
            hump: self.hump,
            crest_angle: self.crest_angle,
            crest_peak: self.crest_peak,
            ..Default::default()
        }
    }
}

/// The bow waves' stem crest (naval-ocean's): height above the sea and decay length astern, in metres. Use the same
/// preset hull length as the ocean's Froude bound; the sliced bow's shorter length only places the spray.
pub fn stem_crest(speed: f64, hull: &HullInfo, bow: &BowShape, shape: &CrestShape) -> (f64, f64) {
    naval_ocean::bow_waves::stem_crest(speed, hull.draft, bow.slope, hull.length, &shape.tuning())
}

/// Seawater over the bow ([`Aboard`]). Visual, not a seakeeping model: green water comes aboard when the sea at the stem
/// stands over the deck edge (the deck-wetness criterion of seakeeping, Ochi 1964: relative bow motion over the
/// freeboard), here on the sea drawn against the stem. How far a plunge's spray reaches over the deck edge (m) from
/// which it throws a sheet over the bow, and at which the sheet is at its fullest (calibrated on the drawn sea: a
/// Fletcher at full power into the North Atlantic's 22 m/s storm throws one every few waves, into a 12 m/s sea rarely,
/// in a following sea or calm water never; "Calibration" in `native/docs/water-effects.md`):
const SHEET_MARGIN: [f64; 2] = [-1.5, 1.0];
/// The sea over the deck edge at the stem (m) at which green water starts to come aboard and at which it pours aboard.
const GREEN_MARGIN: [f64; 2] = [0.0, 1.5];
/// Apparent wind along her bow (m/s, the world's: her way through the air and the true wind against her) from which
/// her spray carries aft over her deck rather than falling back into the sea, and at which it all does.
const CARRY_WIND: [f64; 2] = [4.0, 18.0];
/// Speed (m/s) from which a plunge throws its sheet up the flare, and at which it throws it fully.
const SHEET_SPEED: [f64; 2] = [2.0, 8.0];
/// Share of a full sheet's wetting that the fine spray torn off the bow's sheets in a heavy sea brings aboard, and the
/// significant wave height (m) from which it does and at which it does fully.
const BLOWN: f64 = 0.35;
const BLOWN_SEAS: [f64; 2] = [1.5, 6.0];
/// Forward speed the water thrown over the bow keeps of the hull's way through the world as it leaves the stem head.
const OVER_CARRY: f64 = 0.55;
/// The forecastle the sheets land on: stations from the stem aft over this share of her length.
const DECK_REACH: f64 = 0.3;
const DECK_SAMPLES: usize = 9;
/// Bursts in the air at once: two for each plunge off her flare and each sea she meets, and one over her bow, a few
/// seconds each, for the bows seen largest; and the water lost hulls shed as they come out of the sea (`runoff`).
pub const BURST_CAPACITY: usize = 128;

/// A plunge throws once its slam has passed this height (0–1) and its sheet over the bow this, at most once in
/// `BURST_BETWEEN` seconds a bow.
const BURST_FROM: f64 = 0.1;
const SHEET_FROM: f64 = 0.06;
pub const BURST_BETWEEN: f64 = 0.8;
/// A plunge's sheets off her flare: the fan's middle and half width (radians up from outboard), its slowest water's
/// share of its fastest's, the share of her way its water keeps, how far its speed fore and aft spreads (a share of
/// the fan's fastest, and of her way), and the share of a full burst that is mist.
const SIDE_ELEVATION: f64 = 0.75;
const SIDE_WIDTH: f64 = 0.3;
const SIDE_SLOWEST: f64 = 0.18;
const SIDE_CARRY: f64 = 0.3;
const SIDE_SCATTER: [f64; 2] = [0.15, 0.12];
const SIDE_MIST: f64 = 0.6;
/// Its sheet over her bow: the fan's half width across her deck (radians either side of straight up), its slowest
/// water, how far its speed fore and aft spreads (a share of the fan's fastest: about as far as across her deck), its
/// mist, where on her stem head it leaves (a share of her length aft of the stem) and how long a stretch of it throws
/// (a share of her length).
const OVER_WIDTH: f64 = 0.38;
const OVER_SLOWEST: f64 = 0.25;
const OVER_SCATTER: f64 = 0.35;
const OVER_MIST: f64 = 0.6;
const OVER_HEAD: f64 = 0.012;
const OVER_REACH: f64 = 0.03;
/// A sea she meets under way throws a burst off her flare too (2026-10-21), once the sea's kick times her way's share of
/// her spray passes `MET_FROM`: as high up her flare as a plunge `MET_RISE` as heavy, `MET_WATER` of a plunge's water
/// for its height and `MET_MIST` of it mist (a plunge's is `SIDE_MIST`).
const MET_FROM: f64 = 0.08;
const MET_RISE: f64 = 0.5;
const MET_WATER: f64 = 0.6;
const MET_MIST: f64 = 0.8;
/// The bow runs on through the plunge for about this long (s) on either side of its height: the stretch that throws
/// reaches that much of her way ahead of where she stood and astern of it.
const PLUNGE_TIME: f64 = 0.15;

/// Hulls throwing spray at once: those whose bows the camera sees largest; a sheet off each side of each.
pub const BOW_SPRAY_SLOTS: usize = 12;
pub const SHEET_CAPACITY: usize = BOW_SPRAY_SLOTS * 2;
/// How far below the sea at her stem a sheet's water falls before the sea takes it (m).
const SHEET_BELOW: f64 = 1.5;
/// What the seas she met added to her sheets' climb, kept this often (s) for as long as their water flies
/// (`BowSheet::kicks`).
const KICK_STEP: f64 = 0.1;
const KICK_SAMPLES: usize = 13;
const MIN_PIXELS_PER_METRE: f64 = 0.3;
const CLEAR: f64 = -2.5;
const CONTACT: f64 = -0.5;
const SAMPLES: usize = 9;
const BOW_REACH: f64 = 0.15;
const SURGE_ATTACK: f64 = 0.12;
const SURGE_DECAY: f64 = 0.45;
const SURGE_SEAS: [f64; 2] = [0.2, 2.5];

/// `MathUtils.smoothstep(value, low, high)`.
fn smooth(low: f64, high: f64, value: f64) -> f64 {
    if value <= low {
        0.0
    } else if value >= high {
        1.0
    } else {
        let x = (value - low) / (high - low);
        x * x * (3.0 - 2.0 * x)
    }
}

/// A fixed pseudo-random number in [0, 1) for an integer (`hash` of BowSpray.ts).
fn hash(n: f64) -> f64 {
    let h = (to_uint32(n) ^ 0x9e37_79b9).wrapping_mul(0x85eb_ca6b);
    f64::from((h ^ (h >> 13)).wrapping_mul(0xc2b2_ae35)) / 4_294_967_296.0
}

/// JavaScript's `Math.round`.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// A hull's bow, read once from its waterline profile (`BowShape`).
#[derive(Clone, Debug, PartialEq)]
pub struct BowShape {
    /// Per profile level, the stem's model z.
    pub stems: [f64; PROFILE_LEVELS],
    /// Per level, sample and side (starboard, port): half-breadth `sample × step` metres aft of that level's stem.
    pub breadths: Vec<f64>,
    pub step: f64,
    pub length: f64,
    pub beam: f64,
    pub freeboard: f64,
    pub entrance: f64,
    /// Her waterline's entrance angle as the bow waves read it (`naval_ocean::bow_waves::entrance_slope`), which sets
    /// their crest's height.
    pub slope: f64,
    /// Along the forecastle, [`DECK_SAMPLES`] stations from the stem aft over [`DECK_REACH`] of her length: each one's
    /// model z, the hull's half-breadth at her deck and her deck edge's height over the design waterline.
    pub deck: Vec<[f64; 3]>,
}

/// `bowShape`.
pub fn bow_shape(profile: &WaterlineProfile) -> BowShape {
    let length = f64::from(profile.stern - profile.bow).max(1.0);
    let step = length * BOW_REACH / (SAMPLES - 1) as f64;
    let mut stems = [0.0; PROFILE_LEVELS];
    let mut breadths = vec![0.0; PROFILE_LEVELS * SAMPLES * 2];
    let crossed: Vec<usize> = (0..PROFILE_LEVELS)
        .filter(|&level| !profile.stems[level].is_nan())
        .collect();
    for level in 0..PROFILE_LEVELS {
        // A level the hull misses takes the nearest level it crosses.
        let first = crossed.first().copied().unwrap_or(level);
        let source = crossed.iter().fold(first, |best, &other| {
            if (other as isize - level as isize).abs() < (best as isize - level as isize).abs() {
                other
            } else {
                best
            }
        });
        let stem = if profile.stems[source].is_nan() {
            f64::from(profile.bow)
        } else {
            f64::from(profile.stems[source])
        };
        stems[level] = stem;
        for side in 0..2 {
            let values = if side == 1 {
                &profile.port
            } else {
                &profile.starboard
            };
            let mut points: Vec<(f64, f64)> = vec![(stem, 0.0)];
            for station in 0..PROFILE_STATIONS {
                let z = f64::from(profile.start) + station as f64 * f64::from(profile.spacing);
                let value = values[source * PROFILE_STATIONS + station];
                if z > stem + 1e-3 && !value.is_nan() {
                    points.push((z, f64::from(value).max(0.0)));
                }
            }
            for sample in 0..SAMPLES {
                let z = stem + sample as f64 * step;
                let mut value = points[points.len() - 1].1;
                for i in 1..points.len() {
                    if z <= points[i].0 {
                        let (z0, b0) = points[i - 1];
                        let (z1, b1) = points[i];
                        value = b0 + (b1 - b0) * (z - z0) / (z1 - z0).max(1e-6);
                        break;
                    }
                }
                breadths[(level * SAMPLES + sample) * 2 + side] = value;
            }
        }
    }
    let middle = (PROFILE_LEVELS - 1) / 2;
    let station = |z: f64| {
        js_round((z - f64::from(profile.start)) / f64::from(profile.spacing))
            .clamp(0.0, (PROFILE_STATIONS - 1) as f64) as usize
    };
    let mut freeboard: f64 = 0.0;
    for s in station(stems[middle])..=station(stems[middle] + length * 0.08) {
        freeboard = freeboard
            .max(f64::from(profile.starboard_side[s]))
            .max(f64::from(profile.port_side[s]));
    }
    let reach = js_round(0.05 / BOW_REACH * (SAMPLES - 1) as f64) as usize;
    let opening = (breadths[(middle * SAMPLES + reach) * 2]
        + breadths[(middle * SAMPLES + reach) * 2 + 1])
        / 2.0;
    let freeboard = if freeboard > 0.0 {
        freeboard
    } else {
        1.5 + 0.045 * length
    };
    // The forecastle's edge: each station's side height, and its breadth at the highest slice the hull crosses there.
    let deck = (0..DECK_SAMPLES)
        .map(|k| {
            let z = stems[middle] + k as f64 / (DECK_SAMPLES - 1) as f64 * DECK_REACH * length;
            let s = station(z);
            let side = f64::from(profile.starboard_side[s].max(profile.port_side[s]));
            let breadth = (0..PROFILE_LEVELS)
                .rev()
                .map(|level| {
                    let i = level * PROFILE_STATIONS + s;
                    f64::from(profile.starboard[i].max(profile.port[i]))
                })
                .find(|b| b.is_finite() && *b > 0.0)
                .unwrap_or(0.0);
            [z, breadth, if side > 0.0 { side } else { freeboard }]
        })
        .collect();
    BowShape {
        stems,
        breadths,
        step,
        length,
        beam: f64::from(profile.half_beam) * 2.0,
        freeboard,
        entrance: opening / (reach as f64 * step).max(1e-3),
        slope: entrance_slope(Some(profile)),
        deck,
    }
}

/// Her forecastle `aft` metres astern of the stem: its model z, her half-breadth at the deck and the deck edge's height.
fn deck_at(shape: &BowShape, aft: f64) -> [f64; 3] {
    let step = DECK_REACH * shape.length / (DECK_SAMPLES - 1) as f64;
    let at = (aft / step.max(1e-6)).clamp(0.0, (DECK_SAMPLES - 1) as f64);
    let first = (at.floor() as usize).min(DECK_SAMPLES - 2);
    let t = at - first as f64;
    let (a, b) = (shape.deck[first], shape.deck[first + 1]);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// What a hull's bow ships over her forecastle this frame ([`BowSpray::aboard`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Aboard {
    /// Seawater landing on her forecastle, 0 none … 1 soaking it at once.
    pub wetting: f64,
    /// Share of her length aft of the stem it reaches, and metres above her deck edge it climbs what faces the bow.
    pub reach: f64,
    pub climb: f64,
    /// Green water standing on her deck, 0 … 1: it runs off within seconds.
    pub deck: f64,
    /// The sheet a plunge throws up and back over her bow (0 … 1, carried aft by the apparent wind), the sea's head on
    /// her stem over her deck edge (m; negative below it) and how far the plunge's spray reaches over it.
    pub sheet: f64,
    pub margin: f64,
    pub jet: f64,
    /// Her stem's model z at the design waterline, her length and her deck edge's height over the design waterline at
    /// the bow.
    pub stem_z: f64,
    pub length: f64,
    pub freeboard: f64,
    /// Her stem at the design waterline this frame (world), and whether the sea's head came from the drawn sea
    /// ([`BowSpray::drawn_sea`]) rather than the sea she rides.
    pub stem: DVec3,
    pub drawn: bool,
    /// The sea's head on her stem over its design waterline (m) and how fast it climbs (m/s).
    pub head: f64,
    pub rising: f64,
}

/// `bowSide`: where a side of the bow stands `aft` metres astern of the stem, `height` above the design waterline:
/// model z of that station and the half-breadth there.
pub fn bow_side(shape: &BowShape, side: f64, aft: f64, height: f64) -> (f64, f64) {
    let level = ((height - f64::from(PROFILE_LOWEST)) / f64::from(PROFILE_LEVEL_STEP))
        .clamp(0.0, (PROFILE_LEVELS - 1) as f64);
    let lower = (level.floor() as usize).min(PROFILE_LEVELS - 2);
    let rise = level - lower as f64;
    let sample = (aft / shape.step).clamp(0.0, (SAMPLES - 1) as f64);
    let first = (sample.floor() as usize).min(SAMPLES - 2);
    let along = sample - first as f64;
    let column = if side > 0.0 { 0 } else { 1 };
    let at = |lvl: usize| {
        let row = lvl * SAMPLES;
        let a = shape.breadths[(row + first) * 2 + column];
        let b = shape.breadths[(row + first + 1) * 2 + column];
        a + (b - a) * along
    };
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    (
        lerp(shape.stems[lower], shape.stems[lower + 1], rise) + aft,
        lerp(at(lower), at(lower + 1), rise),
    )
}

/// A hull as the spray reads it this frame.
#[derive(Clone, Copy, Debug)]
pub struct EffectShip<'a> {
    pub pose: &'a ShipPose,
    pub hull: &'a HullInfo,
    pub profile: &'a WaterlineProfile,
    /// Whether the hull is drawn (a hidden one, the lens's own, keeps its spray out of the view).
    pub visible: bool,
    /// Lost (foundering, capsized or blown up): what she lifts out of the sea as she goes pours off her
    /// (`crate::runoff`).
    pub lost: bool,
    /// Her other section once she has broken (the host's `ShipVisual::sections`): what lies on its side of her break
    /// stands where that section is, and sheds its water there (`crate::runoff`). None while she is whole.
    pub section: Option<HullSection>,
}

/// A broken hull's other section ([`EffectShip::section`]): her break's z in her hull frame, whether her own pose is
/// her fore part's (z under the break), and the other section's place and rotation (as `ShipPose::rotation`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullSection {
    pub z: f64,
    pub fore: bool,
    pub position: [f64; 3],
    pub rotation: DQuat,
}

impl HullSection {
    /// Whether her hull frame's `z` lies in this other section.
    pub fn holds(&self, z: f64) -> bool {
        (z < self.z) != self.fore
    }
}

/// One frame of what the spray reads (`BowSprayFrame`).
pub struct BowSprayFrame<'a> {
    pub ships: &'a [EffectShip<'a>],
    /// Presentation seconds since the last frame; 0 holds the spray where it is.
    pub dt: f64,
    pub camera: &'a CameraView,
    /// The combat sea at a presentation time, which the hulls ride; `None` in still water.
    pub sea: Option<(SeaWaves, f64)>,
    /// The bow waves' crest shape; `None` while they are off.
    pub crest: Option<CrestShape>,
    /// Significant wave height of the sea on show (m).
    pub waves: f64,
}

#[derive(Clone, Debug)]
struct BowState {
    speed: f64,
    stem: DVec3,
    immersion: f64,
    plunge: f64,
    kick: f64,
    surge: f64,
    /// What the seas she met added to her sheets' climb (m/s), kept every [`KICK_STEP`] (the newest first), and how long
    /// ago the newest was kept (s).
    kicks: [f64; KICK_SAMPLES],
    kept: f64,
    next: f64,
    fresh: bool,
    /// Her bow's shape, kept with the profile's key it was read for.
    shape: Option<(ProfileKey, Arc<BowShape>)>,
    /// What her bow ships this frame.
    aboard: Aboard,
    /// The sea's head on her stem last frame (m over her design waterline) and how fast it climbs (m/s, eased), from the
    /// drawn sea or the sea she rides (`drawn`: which; a change of source starts the climb afresh).
    head: f64,
    rising: f64,
    drawn: bool,
    /// Her slam's and her sheet over the bow's heights as they pass ([`Peak`]): when a plunge throws its bursts.
    slams: Peak,
    sheets: Peak,
}

impl BowState {
    /// Whether the sea's head on her stem starts afresh (a new hull, or one that jumped).
    fn fresh_head(&self) -> bool {
        self.head.is_nan()
    }

    /// Keep `kick` (m/s) as what the seas she met add to her sheets' climb `step` seconds on.
    fn keep_kick(&mut self, kick: f64, step: f64) {
        self.kept += step;
        while self.kept >= KICK_STEP {
            self.kept -= KICK_STEP;
            self.kicks.rotate_right(1);
            self.kicks[0] = kick;
        }
    }

    /// What the seas she met added to the climb of her sheets' water now `age` seconds old, `kick` the seas' now.
    fn kick_at(&self, age: f64, kick: f64) -> f64 {
        if age <= self.kept {
            return kick + (self.kicks[0] - kick) * age / self.kept.max(1e-6);
        }
        let at = ((age - self.kept) / KICK_STEP).min((KICK_SAMPLES - 1) as f64);
        let first = (at.floor() as usize).min(KICK_SAMPLES - 2);
        let t = at - first as f64;
        self.kicks[first] + (self.kicks[first + 1] - self.kicks[first]) * t
    }
}

/// A throw as it passes its height: a value that rises past `from` and turns to fall gives that height, at most once in
/// `between` seconds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Peak {
    last: f64,
    rising: bool,
    quiet: f64,
}

impl Peak {
    fn pass(&mut self, value: f64, dt: f64, from: f64, between: f64) -> Option<f64> {
        self.quiet = (self.quiet - dt).max(0.0);
        let mut height = None;
        if value > self.last {
            self.rising = true;
        } else if value < self.last && self.rising {
            self.rising = false;
            if self.last >= from && self.quiet <= 0.0 {
                height = Some(self.last);
                self.quiet = between;
            }
        }
        self.last = value;
        height
    }
}

/// A profile's identity for the bow shape kept with a hull (her preset's slice does not change under her).
type ProfileKey = [u32; 5];

fn profile_key(profile: &WaterlineProfile) -> ProfileKey {
    [
        profile.start.to_bits(),
        profile.spacing.to_bits(),
        profile.bow.to_bits(),
        profile.stern.to_bits(),
        profile.half_beam.to_bits(),
    ]
}

struct Bow {
    index: usize,
    source: u32,
    matrix: DMat4,
    /// The hull's bow, shared with the cache (no copy per frame).
    shape: Arc<BowShape>,
    strength: f64,
    pixels: f64,
    crest: f64,
    decay: f64,
    /// How far aft of the stem the crest stands highest (m), and the bow waves' shape it is drawn with.
    peak: f64,
    waves: BowWaveTuning,
    peel: f64,
    /// The crest's height at its peak (m), and what a sea met adds to the sheets' upward speed (m/s): now, and to the
    /// water [`KICK_AGE`], twice and three times that old.
    top: f64,
    kicks: [f64; 4],
    out: f64,
    surge: f64,
    speed: f64,
    immersion: f64,
    /// What a plunge throws this frame as it passes its height (0: none): its bursts off her flare, and over her bow;
    /// and what a sea she meets this frame throws off her flare (0: none).
    burst: f64,
    burst_over: f64,
    met: f64,
}

/// Where `place` puts a point on the bow's side.
struct Placed {
    point: DVec3,
    outward: DVec3,
    forward: DVec3,
}

impl Bow {
    /// The crest's height above the sea on its line `aft` metres astern of the stem (`crestAt`): the bow waves' own
    /// ([`crest_profile`]), which climbs from the stem to its peak.
    fn crest_at(&self, aft: f64) -> f64 {
        self.crest
            * crest_profile(
                aft,
                self.peak,
                self.decay,
                self.shape.length,
                self.shape.beam,
                &self.waves,
            )
    }

    /// How far aft of the stem the sheets leave the crest (m): over its climb and the start of its fall.
    fn sheet_reach(&self) -> f64 {
        (2.2 * self.peak).min(0.1 * self.shape.length)
    }

    /// How fast the sheet's water is still climbing where it leaves the crest `aft` metres astern of the stem (m/s):
    /// what carries it on up to the crest's peak, nothing from the peak aft.
    fn climb_at(&self, aft: f64) -> f64 {
        if aft < self.peak {
            (2.0 * GRAVITY * (self.top - self.crest_at(aft)).max(0.0)).sqrt()
        } else {
            0.0
        }
    }

    /// `place`: the side of the bow `aft` metres astern of the stem, `height` above the sea there, `outboard` metres off
    /// the plating and the crest's peel.
    fn place(
        &self,
        side: f64,
        aft: f64,
        height: f64,
        outboard: f64,
        sea: &Option<(SeaWaves, f64)>,
    ) -> Placed {
        let (z, breadth) = bow_side(&self.shape, side, aft, self.immersion + height);
        let mut point = self.matrix.transform_point3(DVec3::new(
            side * (breadth + outboard + aft * self.peel),
            0.0,
            z,
        ));
        let water = sea
            .map(|(waves, time)| waves.height(point.x, point.z, time))
            .unwrap_or(0.0);
        point.y = water + height;
        let mut outward = self
            .matrix
            .transform_vector3(DVec3::new(side, 0.0, -self.shape.entrance))
            .normalize();
        outward.y = 0.0;
        let outward = outward.normalize_or_zero();
        let forward =
            DVec3::new(-self.matrix.z_axis.x, 0.0, -self.matrix.z_axis.z).normalize_or_zero();
        Placed {
            point,
            outward,
            forward,
        }
    }

    /// The forecastle `aft` metres astern of the stem, `across` (0 … 1) of the way from the centre line to the deck edge
    /// on `side`: the point on the deck, the way outboard and the bow's way.
    fn on_deck(&self, side: f64, aft: f64, across: f64) -> Placed {
        let [z, breadth, height] = deck_at(&self.shape, aft);
        let point = self
            .matrix
            .transform_point3(DVec3::new(side * breadth * across, height, z));
        let mut outward = self.matrix.transform_vector3(DVec3::new(side, 0.0, 0.0));
        outward.y = 0.0;
        let forward =
            DVec3::new(-self.matrix.z_axis.x, 0.0, -self.matrix.z_axis.z).normalize_or_zero();
        Placed {
            point,
            outward: outward.normalize_or_zero(),
            forward,
        }
    }
}

/// The plane a burst's fan stands in, from where it leaves her (`outward` and her way `forward`, world): across it on
/// the sea (unit) and aft along her side (unit, square to it).
fn fan_plane(outward: DVec3, forward: DVec3) -> (DVec2, DVec2) {
    let out = DVec2::new(outward.x, outward.z).normalize_or_zero();
    let ahead = DVec2::new(forward.x, forward.z);
    (out, (-ahead + out * ahead.dot(out)).normalize_or_zero())
}

pub struct BowSpray {
    pub tuning: BowSprayTuning,
    pub enabled: bool,
    /// Each side's sheet off the crest ([`crate::bow_sheets`]), marched with the splashes' columns.
    pub sheets: BowSheets,
    /// The bursts her plunges throw ([`crate::bow_bursts`]), marched with the splashes' columns.
    pub bursts: BowBursts,
    /// Bursts thrown so far: each one's number is drawn from it.
    thrown: u64,
    /// The hull whose spray the last update kept out of the view (the lens's own), which a second view's
    /// [`BowSpray::publish_marched`] keeps out too.
    hidden: Option<u32>,
    /// Lit billows of spray, each meeting the sea at its own water: the bows throw none since 2026-10-20 (their
    /// sheets and bursts are marched water), but a whale's blow is drawn with them (naval-client's wildlife).
    pub billows: ParticlePool,
    /// The billows' sprites as last published, back to front.
    pub billow_sprites: Vec<WaterSprite>,
    states: HashMap<String, BowState>,
    sources: HashMap<String, u32>,
    shapes: Vec<(WaterlineProfile, Arc<BowShape>)>,
    /// The frame's bows, kept as working room between frames.
    bows: Vec<Bow>,
    /// The sheets' clock (s, wrapped at [`CLOCK_WRAP`]).
    clock: f64,
    slots: usize,
    slamming: usize,
    /// Hulls throwing a sheet over the bow last frame, and shipping green water.
    over_bow: usize,
    green: usize,
    random: Lcg,
    /// The highest the drawn sea stands against each hull's stem (world y, by hull id): the host reads it back from the
    /// ocean's heights about the hulls (the very heights the wet band reads: the waves as drawn, the wake and the bow
    /// wave), a few frames late. Where a hull has none (out of view, the heights off), what comes aboard is judged on
    /// the sea she rides and the bow wave's crest on it.
    pub drawn_sea: HashMap<String, f64>,
}

impl Default for BowSpray {
    fn default() -> Self {
        Self::new()
    }
}

impl BowSpray {
    pub fn new() -> Self {
        Self {
            tuning: BowSprayTuning::default(),
            enabled: true,
            sheets: BowSheets::new(SHEET_CAPACITY),
            bursts: BowBursts::new(BURST_CAPACITY),
            thrown: 0,
            hidden: None,
            billows: ParticlePool::new(
                2048,
                PoolKind::Water {
                    fine: false,
                    offscreen: true,
                },
            ),
            billow_sprites: Vec::new(),
            states: HashMap::new(),
            sources: HashMap::new(),
            shapes: Vec::new(),
            bows: Vec::new(),
            clock: 0.0,
            slots: 0,
            slamming: 0,
            over_bow: 0,
            green: 0,
            random: Lcg::new(1),
            drawn_sea: HashMap::new(),
        }
    }

    /// Hulls throwing spray last frame, and those plunging.
    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn slamming(&self) -> usize {
        self.slamming
    }

    /// Hulls throwing a sheet over the bow last frame, and shipping green water over the deck edge.
    pub fn over_bow(&self) -> usize {
        self.over_bow
    }

    pub fn shipping_green_water(&self) -> usize {
        self.green
    }

    /// What hull `id`'s bow shipped over her forecastle in the last update ([`Aboard`]); `None` for a hull the spray has
    /// not met (or no longer has).
    pub fn aboard(&self, id: &str) -> Option<&Aboard> {
        self.states.get(id).map(|state| &state.aboard)
    }

    /// Every hull's [`Aboard`] from the last update, by id.
    pub fn aboards(&self) -> impl Iterator<Item = (&str, &Aboard)> {
        self.states
            .iter()
            .map(|(id, state)| (id.as_str(), &state.aboard))
    }

    fn shape_of(&mut self, profile: &WaterlineProfile) -> Arc<BowShape> {
        // Profiles hold NaN at empty stations, so compare their bits: `==` would never find even an identical clone.
        if let Some((_, shape)) = self.shapes.iter().find(|(p, _)| same_profile(p, profile)) {
            return shape.clone();
        }
        let shape = Arc::new(bow_shape(profile));
        self.shapes.push((profile.clone(), shape.clone()));
        shape
    }

    /// Bow shapes cached so far (one per distinct profile).
    pub fn cached_shapes(&self) -> usize {
        self.shapes.len()
    }

    fn source_of(&mut self, id: &str) -> u32 {
        if let Some(&source) = self.sources.get(id) {
            return source;
        }
        let next = self.sources.len() as u32 + 1;
        self.sources.insert(id.to_owned(), next);
        next
    }

    /// Advance the spray one frame, emit what the bows throw and lay the frame's sheets and sprites (`update`), lit and
    /// drifted by `wind` (m/s).
    pub fn update(&mut self, frame: &BowSprayFrame<'_>, wind: DVec3) {
        let step = frame.dt.clamp(0.0, 0.1);
        let tuning = self.tuning;
        self.clock = (self.clock + step).rem_euclid(CLOCK_WRAP);
        self.billows.advance(step, wind);
        self.bursts.advance(step, wind);
        let camera = frame.camera;
        let eye = camera.position;
        self.sheets.begin();
        let pixel_angle = camera.pixel_angle();
        self.states
            .retain(|id, _| frame.ships.iter().any(|s| s.pose.id == *id));
        let mut hidden: Option<u32> = None;
        let mut bows = std::mem::take(&mut self.bows);
        bows.clear();
        let seas = smooth(SURGE_SEAS[0], SURGE_SEAS[1], frame.waves);
        self.slamming = 0;
        self.over_bow = 0;
        self.green = 0;
        for (index, ship) in frame.ships.iter().enumerate() {
            let source = self.source_of(&ship.pose.id);
            if !ship.visible && hidden.is_none() {
                hidden = Some(source);
            }
            if !self.states.contains_key(&ship.pose.id) {
                let next = self.random.next() * tuning.surge_every;
                self.states.insert(
                    ship.pose.id.clone(),
                    BowState {
                        speed: 0.0,
                        stem: DVec3::ZERO,
                        immersion: 0.0,
                        plunge: 0.0,
                        kick: 0.0,
                        surge: 0.0,
                        kicks: [0.0; KICK_SAMPLES],
                        kept: 0.0,
                        next,
                        fresh: true,
                        shape: None,
                        aboard: Aboard::default(),
                        head: f64::NAN,
                        rising: 0.0,
                        drawn: false,
                        slams: Peak::default(),
                        sheets: Peak::default(),
                    },
                );
            }
            // A submarine's bow throws nothing once it is under; going astern throws nothing either.
            let surface = (1.0 + ship.pose.position[1] / 3.0).clamp(0.0, 1.0);
            let target = ship.pose.speed.max(0.0) * surface;
            let random = &mut self.random;
            let state = self.states.get_mut(&ship.pose.id).expect("inserted above");
            state.speed = if state.fresh {
                target
            } else if step > 0.0 {
                state.speed + (target - state.speed) * (1.0 - (-step / 2.5).exp())
            } else {
                state.speed
            };
            // A sea met this frame, as heavy as the kick it gives (0–1): it throws a burst off her flare.
            let mut met = 0.0;
            if step > 0.0 {
                // Each sea the bow meets kicks the spray up; how hard, with how rough the sea is.
                state.next -= step;
                if state.next <= 0.0 {
                    met = (0.35 + 0.65 * random.next()) * seas;
                    state.kick = state.kick.max(met);
                    state.next = tuning.surge_every * (0.6 + 0.8 * random.next());
                }
                state.kick *= (-step / SURGE_DECAY).exp();
                state.surge += (state.kick - state.surge) * (1.0 - (-step / SURGE_ATTACK).exp());
                // What it adds to her sheets' climb, kept as their water runs aft.
                state.keep_kick(state.speed * tuning.surge_rise * state.surge, step);
            }
            if surface <= 0.0 {
                // Under the sea a submarine's deck ships nothing; she starts afresh when she surfaces.
                state.fresh = true;
                state.aboard = Aboard::default();
                continue;
            }
            // Every hull's bow in the sea, seen or not: her deck wets whether or not her spray is drawn.
            let matrix = ship.pose.matrix();
            let key = profile_key(ship.profile);
            let held = state
                .shape
                .as_ref()
                .filter(|(held, _)| *held == key)
                .map(|(_, shape)| shape.clone());
            let shape = match held {
                Some(shape) => shape,
                None => {
                    let shape = self.shape_of(ship.profile);
                    let state = self.states.get_mut(&ship.pose.id).expect("inserted above");
                    state.shape = Some((key, shape.clone()));
                    shape
                }
            };
            let state = self.states.get_mut(&ship.pose.id).expect("inserted above");
            let stem = matrix.transform_point3(DVec3::new(
                0.0,
                0.0,
                shape.stems[(PROFILE_LEVELS - 1) / 2],
            ));
            let immersion = frame
                .sea
                .map(|(waves, time)| waves.height(stem.x, stem.z, time))
                .unwrap_or(0.0)
                - stem.y;
            // A hull that jumps starts afresh rather than streaking spray across the gap.
            if state.fresh || state.stem.distance_squared(stem) > 100.0 * 100.0 {
                state.stem = stem;
                state.immersion = immersion;
                state.plunge = 0.0;
                state.kicks = [0.0; KICK_SAMPLES];
                state.kept = 0.0;
                state.head = f64::NAN;
                state.fresh = false;
            }
            if step > 0.0 {
                state.plunge += ((immersion - state.immersion) / step - state.plunge)
                    * (1.0 - (-step / 0.15).exp());
            }
            state.immersion = immersion;
            let contact = smooth(CLEAR, CONTACT, immersion);
            let strength = smooth(tuning.start, tuning.full, state.speed) * contact;
            // A plunge throws spray at any speed, most when she drives into the sea.
            let slam = smooth(tuning.plunge_start, tuning.plunge_full, state.plunge)
                * contact
                * (0.4 + 0.6 * smooth(2.0, 10.0, state.speed));
            if slam > 0.05 {
                self.slamming += 1;
            }
            // What comes aboard: the sea's head on her stem against her deck edge, and the apparent wind along her bow.
            // The head is the drawn sea against her stem where the host read it, else the sea she rides with the bow
            // wave's crest on it.
            let crest = frame
                .crest
                .map_or(0.0, |c| stem_crest(state.speed, ship.hull, &shape, &c).0);
            let drawn = self.drawn_sea.get(&ship.pose.id).copied();
            let head = drawn.map_or(immersion + crest, |sea| sea - stem.y);
            if state.drawn != drawn.is_some() || state.fresh_head() {
                state.head = head;
                state.rising = 0.0;
                state.drawn = drawn.is_some();
            }
            // Over the whole interval the head moved in (a sped-up theatre's frames run longer than the particles' capped
            // step), so a faster clock does not steepen the climb.
            let interval = frame.dt.max(0.0);
            if interval > 0.0 {
                state.rising += ((head - state.head) / interval - state.rising)
                    * (1.0 - (-interval / 0.15).exp());
            }
            state.head = head;
            let head_wind = state.speed
                - frame.sea.map_or(0.0, |(waves, _)| {
                    waves.wind_mps
                        * (waves.direction.cos() * ship.pose.forward().x
                            + waves.direction.sin() * ship.pose.forward().z)
                });
            let mut aboard = aboard(
                &shape,
                head,
                state.rising,
                state.speed,
                head_wind,
                strength,
                smooth(BLOWN_SEAS[0], BLOWN_SEAS[1], frame.waves),
                &tuning,
            );
            aboard.stem = stem;
            aboard.drawn = drawn.is_some();
            // A plunge throws as it passes its height: off her flare as her slam does, over her bow as its sheet does.
            let (burst, burst_over) = if step > 0.0 {
                (
                    state
                        .slams
                        .pass(slam, step, BURST_FROM, BURST_BETWEEN)
                        .unwrap_or(0.0),
                    state
                        .sheets
                        .pass(aboard.sheet, interval, SHEET_FROM, BURST_BETWEEN)
                        .unwrap_or(0.0),
                )
            } else {
                (0.0, 0.0)
            };
            state.aboard = aboard;
            if aboard.sheet > 0.05 {
                self.over_bow += 1;
            }
            if aboard.margin > GREEN_MARGIN[0] {
                self.green += 1;
            }
            let position = DVec3::from(ship.pose.position);
            let range = (position - eye).length().max(1.0);
            let seen = self.enabled
                && pixel_angle > 0.0
                && 1.0 / (range * pixel_angle) >= MIN_PIXELS_PER_METRE;
            if !seen {
                state.stem = stem;
                continue;
            }
            let pixels = 1.0 / ((stem - eye).length().max(1.0) * pixel_angle);
            if !ship.visible
                || (strength < 0.01 && burst <= 0.0 && burst_over <= 0.0)
                || pixels < MIN_PIXELS_PER_METRE
            {
                state.stem = stem;
                continue;
            }
            let speed = state.speed;
            let (crest, decay) = match &frame.crest {
                Some(c) => stem_crest(speed, ship.hull, &shape, c),
                None => (0.0, 1.0),
            };
            let surge = state.surge;
            let kick = speed * tuning.surge_rise * surge;
            let kicks = [0.0, 1.0, 2.0, 3.0].map(|k| state.kick_at(k * KICK_AGE, kick));
            let waves = frame.crest.unwrap_or_default().tuning();
            let peak = crest_peak(speed, ship.hull.draft, shape.length, &waves);
            let peel = frame.crest.map_or(0.0, |c| c.crest_angle);
            bows.push(Bow {
                index,
                source,
                matrix,
                strength,
                pixels,
                crest,
                decay,
                peak,
                waves,
                peel,
                top: 0.0,
                // A plunge's own water is its bursts': the sheets only swell with the seas she meets.
                kicks,
                out: tuning.flare * speed * (1.0 + 0.25 * surge),
                surge,
                speed,
                immersion,
                burst,
                burst_over,
                met,
                shape,
            });
        }
        for bow in &mut bows {
            bow.top = bow.crest_at(bow.peak);
        }
        // The bows seen largest throw spray; the rest keep their state for when they come nearer.
        bows.sort_by(|a, b| b.pixels.total_cmp(&a.pixels));
        self.slots = bows.len().min(BOW_SPRAY_SLOTS);
        for (i, bow) in bows.iter().enumerate() {
            let id = &frame.ships[bow.index].pose.id;
            let stem = bow.matrix.transform_point3(DVec3::new(
                0.0,
                0.0,
                bow.shape.stems[(PROFILE_LEVELS - 1) / 2],
            ));
            if i < BOW_SPRAY_SLOTS {
                self.sheet(bow, &frame.sea);
                if step > 0.0 {
                    self.throw(bow, &frame.sea);
                }
            }
            if let Some(state) = self.states.get_mut(id) {
                state.stem = stem;
            }
        }
        bows.clear();
        self.bows = bows;
        // No billow comes off a hull (none carries a source), so none is the lens's own.
        self.billow_sprites.clear();
        self.billows
            .publish_water_into(camera, None, &mut self.billow_sprites);
        self.bursts.publish(camera, hidden);
        self.sheets.publish(camera, hidden);
        self.hidden = hidden;
    }

    /// The bursts and the sheets placed for `camera` again, without moving them on (a second view of the frame), the
    /// hull the last update hid still hidden.
    pub fn publish_marched(&mut self, camera: &CameraView) {
        self.bursts.publish(camera, self.hidden);
        self.sheets.publish(camera, self.hidden);
    }

    /// Each side's sheet off the crest ([`crate::bow_sheets`]): its water's flight from four stations along the stretch
    /// that throws, as the web's strands flew.
    fn sheet(&mut self, bow: &Bow, sea: &Option<(SeaWaves, f64)>) {
        let tuning = self.tuning;
        if bow.strength < 0.01 {
            return;
        }
        let reach = bow.sheet_reach();
        let stem = bow.matrix.transform_point3(DVec3::new(
            0.0,
            0.0,
            bow.shape.stems[(PROFILE_LEVELS - 1) / 2],
        ));
        let water = sea
            .map(|(waves, time)| waves.height(stem.x, stem.z, time))
            .unwrap_or(0.0);
        let origin = DVec3::new(stem.x, water, stem.z);
        let forward = DVec2::new(-bow.matrix.z_axis.x, -bow.matrix.z_axis.z).normalize_or_zero();
        let across = DVec2::new(bow.matrix.x_axis.x, bow.matrix.x_axis.z);
        let starboard = (across - forward * across.dot(forward)).normalize_or_zero();
        // As much as the web's strands threw, swelling with the seas she meets.
        let strength = tuning.sheet * bow.strength.powf(1.3) * 0.8 * (1.0 + 0.3 * bow.surge);
        for side in [1.0_f64, -1.0] {
            let out = starboard * side;
            let (mut lines, mut heights, mut climbs, mut shifts) =
                ([0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4]);
            for i in 0..4 {
                // The crest's water leaves a little over it, at the sea there, where the crest stands against her side
                // (forward of the station where her stem rakes, and as she trims).
                let station = reach * i as f64 / 3.0;
                let placed =
                    bow.place(side, station, bow.crest_at(station) * tuning.rise, 0.0, sea);
                let local = placed.point - origin;
                let local_xz = DVec2::new(local.x, local.z);
                lines[i] = local_xz.dot(out);
                heights[i] = local.y;
                climbs[i] = bow.climb_at(station);
                shifts[i] = local_xz.dot(-forward) - station;
            }
            self.sheets.push(BowSheet {
                origin,
                out,
                aft: -forward,
                source: bow.source,
                seed: hash(f64::from(bow.source) * 31.0 + side),
                clock: self.clock,
                lines,
                heights,
                climbs,
                kicks: bow.kicks,
                shifts,
                // The strands' outboard speeds, slowest at the stem.
                out_stem: bow.out * 0.925,
                out_end: bow.out * 1.125,
                // Past her at her way, less the little of it the water keeps.
                speed: bow.speed * (1.0 - tuning.carry * 0.75),
                reach,
                strength,
                below: SHEET_BELOW,
            });
        }
    }

    /// What a plunge throws as it passes its height ([`crate::bow_bursts`]): a burst of spray off each side of her flare,
    /// and one up over her stem head and back across her forecastle when its spray reaches over her deck edge.
    fn throw(&mut self, bow: &Bow, sea: &Option<(SeaWaves, f64)>) {
        let around = PLUNGE_TIME * bow.speed;
        let met = bow.met * bow.strength;
        if bow.burst > 0.0 {
            let strength = bow.burst;
            // Its steep sheets climb her flare toward her deck edge, and over it at the heaviest plunge: as high as the
            // web's plunge threw its spray.
            let rise = (bow.shape.freeboard + 2.0) * (0.15 + 0.95 * strength);
            self.side_bursts(bow, sea, strength, rise, SIDE_MIST * strength);
        } else if met >= MET_FROM {
            // A sea she meets under way throws a lighter one, lower up her flare and more of it mist.
            let rise = (bow.shape.freeboard + 2.0) * (0.15 + 0.95 * MET_RISE * met);
            self.side_bursts(bow, sea, MET_WATER * met, rise, MET_MIST * met);
        }
        if bow.burst_over > 0.0 {
            let strength = bow.burst_over;
            let length = bow.shape.length;
            // Across her deck from the stem head: its middle straight up, spreading to either side.
            let placed = bow.on_deck(1.0, OVER_HEAD * length, 0.0);
            let (out, along) = fan_plane(placed.outward, placed.forward);
            // Up to 2 m over the deck for a light throw, 10 m for the heaviest (the web's sheet over the bow).
            let rise = 1.5 + 8.5 * strength;
            self.thrown += 1;
            self.bursts.throw(BowBurst {
                origin: placed.point,
                out,
                aft: along,
                source: bow.source,
                strength,
                speed: launch_for(rise),
                elevation: std::f64::consts::FRAC_PI_2,
                width: OVER_WIDTH,
                slowest: OVER_SLOWEST,
                way: bow.speed,
                carry: OVER_CARRY,
                scatter: OVER_SCATTER * launch_for(rise),
                lead: around,
                length: OVER_REACH * length + around,
                below: bow.shape.freeboard + 2.0,
                mist: OVER_MIST * strength,
                ..thrown(hash(f64::from(bow.source) * 977.0 + self.thrown as f64))
            });
        }
    }

    /// A burst off each side of her flare: `strength` its water (0–1), climbing `rise` m over where it leaves, `mist` of
    /// it the mist the wind takes (0–1).
    fn side_bursts(
        &mut self,
        bow: &Bow,
        sea: &Option<(SeaWaves, f64)>,
        strength: f64,
        rise: f64,
        mist: f64,
    ) {
        let around = PLUNGE_TIME * bow.speed;
        let reach = bow.sheet_reach();
        let speed = launch_for(rise) / (SIDE_ELEVATION + SIDE_WIDTH).sin();
        let aft = 0.3 * reach;
        let height = bow.crest_at(aft) * 0.6;
        for side in [1.0_f64, -1.0] {
            let placed = bow.place(side, aft, height, 0.3, sea);
            let (out, along) = fan_plane(placed.outward, placed.forward);
            self.thrown += 1;
            self.bursts.throw(BowBurst {
                origin: placed.point,
                out,
                aft: along,
                source: bow.source,
                strength,
                speed,
                elevation: SIDE_ELEVATION,
                width: SIDE_WIDTH,
                slowest: SIDE_SLOWEST,
                way: bow.speed,
                carry: SIDE_CARRY,
                scatter: SIDE_SCATTER[0] * speed + SIDE_SCATTER[1] * bow.speed,
                lead: aft + around,
                length: reach - aft + around,
                below: height + 2.0,
                mist,
                ..thrown(hash(f64::from(bow.source) * 977.0 + self.thrown as f64))
            });
        }
    }

    pub fn reset(&mut self) {
        self.billows.reset();
        self.billow_sprites.clear();
        self.sheets.reset();
        self.bursts.reset();
        self.thrown = 0;
        self.hidden = None;
        self.states.clear();
        self.slots = 0;
        self.slamming = 0;
        self.over_bow = 0;
        self.green = 0;
        self.random = Lcg::new(1);
        self.clock = 0.0;
    }
}

/// What a bow ships over her forecastle ([`Aboard`]) from her bow (`shape`) in the sea this frame: `head` the sea's
/// height against her stem over its design waterline (m; the bow wave's crest included), `plunge` how fast it climbs
/// her stem (m/s), `speed` her speed, `head_wind` the apparent wind along her bow (m/s), `strength` her bow
/// sheets' and `seas` how heavy the sea is (0 … 1).
#[allow(clippy::too_many_arguments)]
fn aboard(
    shape: &BowShape,
    head: f64,
    plunge: f64,
    speed: f64,
    head_wind: f64,
    strength: f64,
    seas: f64,
    tuning: &BowSprayTuning,
) -> Aboard {
    // The spray a plunge throws: the water meeting the flare leaves it in a jet at about twice the speed the sea climbs
    // the stem (the spray root of a wedge entering water, Wagner 1932), so it rises (2w)² / 2g over the sea against the
    // stem. How far that reaches over her deck edge decides whether a sheet goes over the bow.
    let rising = plunge.max(0.0);
    let jet = head + 2.0 * rising * rising / GRAVITY - shape.freeboard;
    let margin = head - shape.freeboard;
    let carry = smooth(CARRY_WIND[0], CARRY_WIND[1], head_wind);
    let thrown = smooth(SHEET_MARGIN[0], SHEET_MARGIN[1], jet)
        * smooth(tuning.plunge_start, tuning.plunge_full, plunge)
        * smooth(SHEET_SPEED[0], SHEET_SPEED[1], speed);
    let sheet = thrown * carry;
    // Green water: the sea itself over the deck edge.
    let green = smooth(GREEN_MARGIN[0], GREEN_MARGIN[1], margin);
    // Fine spray torn off the bow's sheets in a heavy sea and blown aboard by the apparent wind.
    let blown = BLOWN * strength * seas * carry;
    // A sheet that falls back into the sea short of the deck still wets the flare and the stem head.
    let wetting = (thrown * (0.35 + 0.65 * carry))
        .max(green)
        .max(blown)
        .min(1.0);
    Aboard {
        wetting,
        reach: (0.04 + 0.26 * sheet)
            .max(0.06 + 0.16 * green)
            .max(0.04 + 0.08 * blown / BLOWN),
        climb: (1.5 + 10.0 * sheet)
            .max(1.0 + 2.0 * green)
            .max(1.0 + 3.0 * blown / BLOWN),
        deck: green.max(0.6 * sheet),
        sheet,
        margin,
        jet,
        stem_z: shape.stems[(PROFILE_LEVELS - 1) / 2],
        length: shape.length,
        freeboard: shape.freeboard,
        stem: DVec3::ZERO,
        drawn: false,
        head,
        rising: plunge,
    }
}

/// Whether two profiles are the same slice, bit for bit (NaN marks empty stations, and NaN != NaN).
fn same_profile(a: &WaterlineProfile, b: &WaterlineProfile) -> bool {
    let same = |a: &[f32], b: &[f32]| {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
    };
    a.start.to_bits() == b.start.to_bits()
        && a.spacing.to_bits() == b.spacing.to_bits()
        && a.bow.to_bits() == b.bow.to_bits()
        && a.stern.to_bits() == b.stern.to_bits()
        && a.half_beam.to_bits() == b.half_beam.to_bits()
        && same(&a.stems, &b.stems)
        && same(&a.starboard, &b.starboard)
        && same(&a.port, &b.port)
        && same(&a.starboard_side, &b.starboard_side)
        && same(&a.port_side, &b.port_side)
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_ocean::hull_profile::SimpleHull;

    fn destroyer() -> (HullInfo, WaterlineProfile) {
        let hull = HullInfo {
            id: "fletcher".into(),
            preset: "fletcher".into(),
            length: 112.5,
            beam: 12.0,
            draft: 4.0,
            forward_speed: 18.5,
            reverse_speed: 6.0,
            ..Default::default()
        };
        let profile =
            SimpleHull::warship(hull.length as f32, hull.beam as f32, hull.draft as f32).profile();
        (hull, profile)
    }

    #[test]
    fn spray_and_ocean_share_the_crest_through_the_froude_transition() {
        use naval_ocean::bow_waves::{BowShip, BowWaves};

        let (hull, profile) = destroyer();
        let camera = CameraView::look_at(
            DVec3::new(-47.0, 6.0, -60.0),
            DVec3::new(0.0, 3.0, -47.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        // The sliced profile is 110.65 m long; the preset is 112.5 m. In the transition that difference matters.
        for speed in [14.8, 14.4, 15.2, 18.5] {
            let mut waves = BowWaves::default();
            waves.update(
                &[BowShip {
                    id: &hull.id,
                    position: [0.0; 3],
                    heading: 0.0,
                    speed,
                    length: hull.length,
                    beam: hull.beam,
                    draft: hull.draft,
                    center_x: hull.center_x,
                    center_z: hull.center_z,
                    forward_speed: hull.forward_speed,
                    profile: Some(&profile),
                }],
                0.0,
                None,
                720.0,
            );
            let pose = ShipPose {
                id: hull.id.clone(),
                speed,
                ..Default::default()
            };
            let mut spray = BowSpray::new();
            spray.update(
                &BowSprayFrame {
                    ships: &[EffectShip {
                        pose: &pose,
                        hull: &hull,
                        profile: &profile,
                        visible: true,
                        lost: false,
                        section: None,
                    }],
                    dt: 0.0,
                    camera: &camera,
                    sea: None,
                    crest: Some(CrestShape::from(&waves.tuning)),
                    waves: 0.0,
                },
                DVec3::ZERO,
            );
            let drawn = f64::from(waves.slots[0][1][2]);
            let head = spray.aboard(&hull.id).unwrap().head;
            assert!(
                (head - drawn).abs() < 1e-6,
                "{speed} m/s: spray {head}, ocean {drawn}"
            );
        }
    }

    /// A profile with empty (NaN) stations finds its cached shape again, clone or not: the cache holds one per hull.
    #[test]
    fn the_bow_shape_cache_does_not_grow() {
        let (_, profile) = destroyer();
        let mut spray = BowSpray::new();
        for _ in 0..10 {
            let copy = profile.clone();
            spray.shape_of(&copy);
        }
        assert_eq!(spray.cached_shapes(), 1);
    }

    /// `bowShape` and `bowSide` of BowSpray.ts on the same proxy profile, computed in Bun.
    #[test]
    fn the_bow_shape_is_the_web_one() {
        let (_, profile) = destroyer();
        let shape = bow_shape(&profile);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-5 * b.abs().max(1.0);
        assert!(close(shape.step, 2.074_655_118_75) && close(shape.length, 110.648_273));
        assert!(
            close(shape.freeboard, 6.116_182_327_270_508)
                && close(shape.entrance, 0.321_514_146_725_951_5)
        );
        let breadths = [
            0.0,
            0.0,
            0.107_235_267_758,
            0.107_235_267_758,
            0.209_944_933_652,
            0.209_944_933_652,
            0.304_592_013_359,
        ];
        for (i, b) in breadths.iter().enumerate() {
            assert!(
                close(shape.breadths[i], *b),
                "{i}: {} against {b}",
                shape.breadths[i]
            );
        }
        let stems = [-53.85, -53.85, -55.05, -56.25, -56.95, -57.65, -58.35];
        for (a, b) in shape.stems.iter().zip(stems) {
            assert!(close(*a, b), "{a} against {b}");
        }
        for (side, aft, height, z, breadth) in [
            (
                1.0,
                3.3,
                0.8,
                -53.230_000_305_175_786,
                1.097_749_081_809_463,
            ),
            (
                -1.0,
                9.1,
                -1.5,
                -46.249_999_427_795_41,
                2.800_581_733_688_740_6,
            ),
            (
                1.0,
                14.0,
                3.7,
                -43.545_001_411_437_994,
                3.954_309_223_125_123,
            ),
        ] {
            let (az, ab) = bow_side(&shape, side, aft, height);
            assert!(
                close(az, z) && close(ab, breadth),
                "{side} {aft} {height}: {az} {ab}"
            );
        }
    }

    fn run(
        spray: &mut BowSpray,
        hull: &HullInfo,
        profile: &WaterlineProfile,
        frames: usize,
        waves: f64,
        sea: Option<SeaWaves>,
    ) {
        run_from(spray, hull, profile, 0, frames, waves, sea);
    }

    /// [`run`] from frame `first` of her passage on.
    fn run_from(
        spray: &mut BowSpray,
        hull: &HullInfo,
        profile: &WaterlineProfile,
        first: usize,
        frames: usize,
        waves: f64,
        sea: Option<SeaWaves>,
    ) {
        let camera = CameraView::look_at(
            DVec3::new(-47.0, 6.0, -60.0),
            DVec3::new(0.0, 3.0, -47.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        for frame in first..first + frames {
            let time = frame as f64 / 60.0;
            // 28 kn along −Z.
            let pose = ShipPose {
                id: hull.id.clone(),
                position: [0.0, 0.0, -14.4 * time],
                speed: 14.4,
                ..Default::default()
            };
            let moved = CameraView {
                position: camera.position + DVec3::new(0.0, 0.0, pose.position[2]),
                ..camera
            };
            let moved = CameraView::look_at(
                moved.position,
                DVec3::new(0.0, 3.0, -47.0 + pose.position[2]),
                40.0,
                0.5,
                60000.0,
                1280.0,
                720.0,
            );
            let ships = [EffectShip {
                pose: &pose,
                hull,
                profile,
                visible: true,
                lost: false,
                section: None,
            }];
            spray.update(
                &BowSprayFrame {
                    ships: &ships,
                    dt: 1.0 / 60.0,
                    camera: &moved,
                    sea: sea.map(|s| (s, time)),
                    crest: Some(CrestShape::default()),
                    waves,
                },
                DVec3::new(2.4, 0.0, 0.9),
            );
        }
    }

    #[test]
    fn a_destroyer_at_28_knots_throws_spray() {
        let (hull, profile) = destroyer();
        let mut spray = BowSpray::new();
        run(&mut spray, &hull, &profile, 180, 0.0, None);
        assert_eq!(spray.slots(), 1);
        assert_eq!(spray.sheets.sheets().len(), 2);
        assert_eq!(spray.sheets.instances().len(), 2);
        // Deterministic: the same run throws the same spray.
        let mut again = BowSpray::new();
        run(&mut again, &hull, &profile, 180, 0.0, None);
        assert_eq!(spray.sheets.sheets(), again.sheets.sheets());
        // No sprites: the drops thrown off the sheets stood close by as large pale discs.
        assert!(spray.billow_sprites.is_empty());
        // Sheets stand at the stem and fall back into the sea astern of it: no fountain over the crest (3.9 m on the
        // proxy's full parabolic bow); one off each side, outboard of her.
        let crest = stem_crest(14.4, &hull, &bow_shape(&profile), &CrestShape::default()).0;
        let [starboard, port] = [&spray.sheets.sheets()[0], &spray.sheets.sheets()[1]];
        assert!((starboard.out + port.out).length() < 1e-9);
        for sheet in [starboard, port] {
            let parcels: Vec<_> = (1..300)
                .flat_map(|k| sheet.cross(0.2 * f64::from(k)).parcels)
                .collect();
            let top = parcels.iter().map(|p| p.at.y).fold(f64::MIN, f64::max);
            assert!(top > 0.5 && top < 2.0 * crest, "{top} {crest}");
            assert!(parcels.iter().all(|p| p.at.x > 0.0));
            assert!(parcels.iter().any(|p| p.at.y < -1.0));
        }
    }

    /// The most a bow shipped over `seconds` at 28 kn in `sea` (`None`: still water): the largest sheet over the bow,
    /// wetting, green-water margin, and the frames that threw a sheet.
    fn shipped(sea: Option<SeaWaves>, seconds: usize) -> (f64, f64, f64, usize) {
        let (hull, profile) = destroyer();
        let mut spray = BowSpray::new();
        let (mut sheet, mut wetting, mut margin, mut frames) = (0.0_f64, 0.0_f64, f64::MIN, 0);
        let waves = if sea.is_some() { 7.0 } else { 0.0 };
        for half in 0..seconds * 2 {
            run_from(&mut spray, &hull, &profile, half * 30, 30, waves, sea);
            let aboard = spray.aboard(&hull.id).expect("seen");
            sheet = sheet.max(aboard.sheet);
            wetting = wetting.max(aboard.wetting);
            margin = margin.max(aboard.margin);
            frames += spray.over_bow();
        }
        (sheet, wetting, margin, frames)
    }

    /// A storm's sea (22 m/s: Hs 7.5 m, the combat sea's 242 m long component) running toward `direction`.
    fn storm(direction: f64) -> SeaWaves {
        SeaWaves {
            amplitude_m: 3.46,
            wavelength_m: 242.0,
            direction,
            wind_mps: 22.0,
            phase: 0.0,
        }
    }

    #[test]
    fn still_water_ships_nothing_over_the_bow() {
        let (sheet, wetting, margin, frames) = shipped(None, 6);
        assert_eq!((sheet, wetting, frames), (0.0, 0.0, 0), "{margin}");
        // Her bow wave's crest stands well below her deck edge at 28 kn.
        assert!(margin < -2.0, "{margin}");
    }

    #[test]
    fn a_head_sea_throws_sheets_over_the_bow_and_a_following_one_does_not() {
        // She steams along -Z: a sea running toward +Z meets her head on.
        let (sheet, wetting, margin, frames) =
            shipped(Some(storm(std::f64::consts::FRAC_PI_2)), 12);
        assert!(
            sheet > 0.2 && wetting > 0.2 && frames > 0,
            "{sheet} {wetting} {margin} {frames}"
        );
        let (sheet, wetting, _, frames) = shipped(Some(storm(-std::f64::consts::FRAC_PI_2)), 12);
        assert!(sheet < 0.05 && frames == 0, "{sheet} {wetting} {frames}");
    }

    #[test]
    fn green_water_comes_aboard() {
        let (_, profile) = destroyer();
        let shape = bow_shape(&profile);
        // The sea a metre over her deck edge at the stem: green water over the forecastle.
        let tuning = BowSprayTuning::default();
        let aboard = aboard(
            &shape,
            shape.freeboard + 1.0,
            0.0,
            10.0,
            15.0,
            0.5,
            0.0,
            &tuning,
        );
        assert_eq!(aboard.margin, 1.0);
        assert!(aboard.deck > 0.5 && aboard.wetting > 0.5, "{aboard:?}");
        assert!(
            aboard.reach > 0.1 && aboard.reach < DECK_REACH,
            "{aboard:?}"
        );
    }

    #[test]
    fn a_sea_met_under_way_throws_a_burst_off_her_flare() {
        // The sea's weight alone (no swell under her: she does not plunge): every few seconds a sea she meets throws a
        // burst off each side of her flare; in still water none.
        let (hull, profile) = destroyer();
        let mut rough = BowSpray::new();
        run(&mut rough, &hull, &profile, 600, 3.0, None);
        assert_eq!(rough.slamming(), 0);
        assert!(
            rough.thrown >= 4 && rough.thrown.is_multiple_of(2),
            "{}",
            rough.thrown
        );
        let burst = &rough.bursts.bursts()[0];
        assert!(burst.mist > 0.0 && burst.strength < 1.0, "{burst:?}");
        let mut still = BowSpray::new();
        run(&mut still, &hull, &profile, 600, 0.0, None);
        assert_eq!(still.thrown, 0);
    }

    #[test]
    fn a_heavy_sea_makes_the_bow_plunge() {
        let (hull, profile) = destroyer();
        let sea = SeaWaves {
            amplitude_m: 3.0,
            wavelength_m: 90.0,
            direction: std::f64::consts::FRAC_PI_2,
            wind_mps: 22.0,
            phase: 0.0,
        };
        let mut spray = BowSpray::new();
        let mut slammed = 0;
        for _ in 0..12 {
            run(&mut spray, &hull, &profile, 30, 5.0, Some(sea));
            slammed += spray.slamming();
        }
        assert!(slammed > 0);
    }
}
