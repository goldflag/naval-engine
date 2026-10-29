//! The sheet of spray a bow throws off her crest as she runs, as water marched through on the GPU (native,
//! 2026-10-20). It replaced the web game's strands (flat ribbons along the water's flight, which read as a glassy arch
//! over the crest), the heap of billows riding on the crest and the tufts flying with the strands (puffs of cotton)
//! (`native/docs/water-effects.md`, "Bow spray as water").
//!
//! The crest climbing her side from the stem throws its water off a little over itself, still climbing at the speed
//! that carries it to the crest's peak and outboard a little faster than the crest line leaves her track
//! ([`crate::bow_spray`]); slowed by its drag, it passes her at her way and falls back onto the crest's outer face
//! astern of where it left. Seen from her it stands steady: each station of the crest throws the same flight frame
//! after frame. So each side's sheet is one [`BowSheet`] in her own frame, its flight given at four stations along the
//! stretch that throws (the stem, a third, two thirds and the whole of it), and the shader finds a point's water from
//! where it is: the water now z aft of the stem left station s (z − s) / U seconds ago, U its speed aft past her, from
//! where the station's water leaves (her stem's rake at that height and her trim move it fore and aft:
//! [`BowSheet::shifts`]; `column_field.wgsl`'s `sheet_cross`). Its strands are fixed to the station and the moment their water left, so they
//! lie along its flight and stream aft with it; it tears as it ages. It sheds no mist of its own (drawn as a halo
//! about it, the mist stood as smoke along her side; a plunge's bursts shed theirs). Sheets are marched with
//! the splashes' columns and the bursts, sorted among them and lit as they are. This file works out each sheet's record
//! and the box its water fills; the shaders hold the shapes. The constants shared with `column_field.wgsl` are marked;
//! keep the two in step.

use bytemuck::Zeroable;
use glam::{DVec2, DVec3};
use naval_effects::CameraView;
use naval_effects::math::smooth;

use crate::columns::{ColumnGpu, KIND_SHEET, flight};

/// The sheets' drag (1/s, as the bursts' and the drops'), the oldest water drawn (s), its spread about its line where
/// it leaves (m), how far a metre of its flight spreads it every way (a share of its launch speed) and its climb up and
/// down (a share of the climb) (`column_field.wgsl`: `SHEET_DRAG`, `SHEET_AGE`, `SHEET_ROOT`, `SHEET_SPREAD`,
/// `SHEET_CLIMB_SPREAD`).
pub const SHEET_DRAG: f64 = 0.6;
const SHEET_AGE: f64 = 2.2;
const SHEET_ROOT: f64 = 0.08;
const SHEET_SPREAD: f64 = 0.06;
const SHEET_CLIMB_SPREAD: f64 = 0.12;
/// How much farther than its spread its fingers throw its water (in its spreads; `SHEET_FINGERS`).
const SHEET_FINGERS: f64 = 0.45;
/// How large a view draws her bow (pixels a metre) when her sheets start to show in it and when they show fully: a frame
/// at 1080p shows none past about 1.7 km, where a sheet would be a few pixels marched through a box.
const SHEET_PIXELS: [f64; 2] = [0.8, 1.6];
/// The ages its kicks are given at (s apart; `KICK_AGE`).
pub const KICK_AGE: f64 = 0.4;
/// Its clock wraps here (s): every rate its noise is read at turns this into a whole number of the noise's tiles.
pub const CLOCK_WRAP: f64 = 400.0;
/// The box's margin about its water (m), and the cross-sections it is read from.
const MARGIN: f64 = 0.5;
const SECTIONS: usize = 32;

/// One side's sheet of a bow, this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct BowSheet {
    /// Her stem at the sea's level (world m), and her frame on the sea: outboard on this side, square to her centre
    /// line, and aft along it (world xz, unit).
    pub origin: DVec3,
    pub out: DVec2,
    pub aft: DVec2,
    /// The hull that throws it: a hidden hull's (the lens's own) is not drawn.
    pub source: u32,
    /// Its own number (0–1): where its strands and lumps lie.
    pub seed: f64,
    /// Its clock (s, wrapped at [`CLOCK_WRAP`]): its water streams aft by it.
    pub clock: f64,
    /// At its four stations (the stem, a third, two thirds and the whole of [`BowSheet::reach`] aft of it): how far
    /// out of her centre line the crest stands (m), how high over the origin its water leaves (m), and how fast it
    /// climbs (m/s).
    pub lines: [f64; 4],
    pub heights: [f64; 4],
    pub climbs: [f64; 4],
    /// What the seas she met added to its climb (m/s), by the water's age: now, and [`KICK_AGE`], twice and three
    /// times that ago.
    pub kicks: [f64; 4],
    /// How far aft of its station (m; forward, less than nothing) each station's water leaves: her stem's rake at the
    /// height it leaves, and her trim.
    pub shifts: [f64; 4],
    /// Its speed outboard at the stem and at the stretch's end, and aft past her (m/s); the stretch that throws (m aft
    /// of the stem).
    pub out_stem: f64,
    pub out_end: f64,
    pub speed: f64,
    pub reach: f64,
    /// How much water it throws (0–1, a little more in a sea), and how far below the origin its water falls before the
    /// sea takes it (m).
    pub strength: f64,
    pub below: f64,
}

/// A value at the four stations, at a share of the way from the first to the last, in line between them
/// (`at_station`).
fn at_station(v: &[f64; 4], share: f64) -> f64 {
    let f = share.clamp(0.0, 1.0) * 3.0;
    let a = v[0] + (v[1] - v[0]) * f.clamp(0.0, 1.0);
    let b = a + (v[2] - a) * (f - 1.0).clamp(0.0, 1.0);
    b + (v[3] - b) * (f - 2.0).clamp(0.0, 1.0)
}

/// The water that left a station and is now some way aft: where it is across her side (x out, y up; m), how far it has
/// spread about its line across her side and up and down (m), and its age (s) (`Parcel`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parcel {
    pub at: DVec2,
    pub spread: DVec2,
    pub age: f64,
}

/// A cross-section: its youngest, middle and oldest water, and the stations the youngest and oldest left (m aft of the
/// stem) (`Cross`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cross {
    pub parcels: [Parcel; 3],
    pub near: f64,
    pub far: f64,
}

impl BowSheet {
    /// The water that left station `s` (m aft of the stem) and is now `z` m aft (`sheet_parcel`).
    pub fn parcel(&self, z: f64, s: f64) -> Parcel {
        let share = s / self.reach.max(1e-3);
        let t = (z - s - at_station(&self.shifts, share)).max(0.0) / self.speed.max(0.5);
        let (travel, fall) = flight(t, SHEET_DRAG);
        let out = self.out_stem + (self.out_end - self.out_stem) * share.clamp(0.0, 1.0);
        let up = at_station(&self.climbs, share) + at_station(&self.kicks, t / (3.0 * KICK_AGE));
        let spread = SHEET_ROOT + SHEET_SPREAD * DVec2::new(out, up).length() * travel;
        Parcel {
            at: DVec2::new(
                at_station(&self.lines, share) + out * travel,
                at_station(&self.heights, share) + up * travel - fall,
            ),
            spread: DVec2::new(spread, spread + SHEET_CLIMB_SPREAD * up.max(0.0) * travel),
            age: t,
        }
    }

    /// The cross-section `z` m aft of the stem (`sheet_cross`).
    pub fn cross(&self, z: f64) -> Cross {
        let reach = self.reach.max(1e-3);
        let near = (z - at_station(&self.shifts, z / reach)).clamp(0.0, self.reach);
        let oldest = z - self.speed.max(0.5) * SHEET_AGE;
        let far = (oldest - at_station(&self.shifts, oldest / reach)).clamp(0.0, near);
        Cross {
            parcels: [
                self.parcel(z, near),
                self.parcel(z, 0.5 * (near + far)),
                self.parcel(z, far),
            ],
            near,
            far,
        }
    }

    /// How far aft of the stem its oldest water is drawn (m).
    pub fn length(&self) -> f64 {
        self.reach
            + self.shifts.iter().fold(0.0_f64, |a, &b| a.max(b))
            + self.speed.max(0.5) * SHEET_AGE
    }

    /// The box frame's z (world xz, unit: `out` turned a right angle, so the frame is a turn, not a mirror) and which way
    /// it runs along `aft` (1 with it, −1 against it).
    pub fn frame_z(&self) -> (DVec2, f64) {
        let z = DVec2::new(-self.out.y, self.out.x);
        (z, if z.dot(self.aft) < 0.0 { -1.0 } else { 1.0 })
    }

    /// The record the columns' march reads (`column_field.wgsl`'s `Sheet`: the lanes of a splash's record, read as
    /// below) and the box its water fills.
    pub fn state(&self) -> ColumnGpu {
        let mut low = DVec3::splat(f64::MAX);
        let mut high = DVec3::splat(f64::MIN);
        let length = self.length();
        let step = length / SECTIONS as f64;
        for k in 1..=SECTIONS {
            let z = step * k as f64;
            let cross = self.cross(z);
            if cross.near <= cross.far {
                continue;
            }
            for parcel in cross.parcels {
                // Its water, its fingers thrown farthest.
                let reach = (1.0 + SHEET_FINGERS) * parcel.spread.max_element();
                let at = parcel.at;
                if at.y + reach < -self.below {
                    continue;
                }
                low = low.min(DVec3::new(at.x - reach, at.y - reach, z - step));
                high = high.max(DVec3::new(at.x + reach, at.y + reach, z + step));
            }
        }
        if low.x > high.x {
            // Nothing above the sea: an empty box at the origin.
            low = DVec3::ZERO;
            high = DVec3::ZERO;
        } else {
            low = (low - DVec3::splat(MARGIN)).max(DVec3::new(f64::MIN, -self.below, 0.0));
            high += DVec3::splat(MARGIN);
        }
        let (z, along) = self.frame_z();
        if along < 0.0 {
            (low.z, high.z) = (-high.z, -low.z);
        }
        let f = |v: f64| v as f32;
        let four = |v: [f64; 4]| v.map(f);
        ColumnGpu {
            origin: [f(self.origin.x), f(self.origin.y), f(self.origin.z), 1.0],
            low: [f(low.x), f(low.y), f(low.z), f(self.clock)],
            high: [f(high.x), f(high.y), f(high.z), f(self.seed)],
            way: four(self.lines),
            flight: four(self.heights),
            veil: four(self.climbs),
            crown: four(self.kicks),
            surge: [
                f(self.out_stem),
                f(self.out_end),
                f(self.speed),
                f(self.reach),
            ],
            tint: [1.0, 1.0, 1.0, 1.0],
            shape: four(self.shifts),
            frame: [f(self.out.x), f(self.out.y), f(z.x), f(z.y)],
            kind: [KIND_SHEET, f(along), f(self.strength), f(self.below)],
            ..ColumnGpu::zeroed()
        }
    }
}

/// The bows' sheets this frame: rebuilt each frame from the bows, at most `capacity`.
pub struct BowSheets {
    sheets: Vec<BowSheet>,
    capacity: usize,
    published: Vec<(f64, ColumnGpu)>,
}

impl BowSheets {
    pub fn new(capacity: usize) -> Self {
        Self {
            sheets: Vec::with_capacity(capacity),
            capacity,
            published: Vec::with_capacity(capacity),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn sheets(&self) -> &[BowSheet] {
        &self.sheets
    }

    /// Sheets in view last frame, farthest first, each with its squared distance from the eye.
    pub fn instances(&self) -> &[(f64, ColumnGpu)] {
        &self.published
    }

    /// Start the frame's sheets afresh.
    pub fn begin(&mut self) {
        self.sheets.clear();
    }

    pub fn push(&mut self, sheet: BowSheet) {
        if self.sheets.len() < self.capacity {
            self.sheets.push(sheet);
        }
    }

    /// The frame's sheets in view, farthest first, but `hidden`'s; each fading out where `camera` draws its bow too small
    /// to show it.
    pub fn publish(&mut self, camera: &CameraView, hidden: Option<u32>) {
        self.published.clear();
        let (p0, p5) = (camera.projection.x_axis.x, camera.projection.y_axis.y);
        let pixel_angle = camera.pixel_angle();
        for sheet in &self.sheets {
            if Some(sheet.source) == hidden || sheet.strength <= 0.0 {
                continue;
            }
            let shown = if pixel_angle > 0.0 {
                let pixels =
                    1.0 / ((sheet.origin - camera.position).length().max(1.0) * pixel_angle);
                smooth((pixels - SHEET_PIXELS[0]) / (SHEET_PIXELS[1] - SHEET_PIXELS[0]))
            } else {
                1.0
            };
            if shown <= 0.0 {
                continue;
            }
            let mut state = sheet.state();
            state.kind[2] *= shown as f32;
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
        self.sheets.clear();
        self.published.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::GRAVITY;

    /// The age its oldest water fades from (s), and its water per metre at full (`column_field.wgsl`: `SHEET_FADE`,
    /// `SHEET_DENSITY`).
    const SHEET_FADE: f64 = 1.3;
    const SHEET_DENSITY: f64 = 3.0;

    /// The water per metre on a cross-section's line at full where its spread is `spread` (`sheet_mass`).
    fn mass(cross: &Cross, spread: DVec2, sheet: &BowSheet) -> f64 {
        let [young, middle, old] = cross.parcels;
        let line = (middle.at - young.at).length() + (old.at - middle.at).length();
        let wide = (spread.x * spread.y).sqrt();
        SHEET_DENSITY * sheet.strength * (cross.near - cross.far)
            / sheet.speed.max(0.5)
            / ((line + 1.5 * wide) * 1.5 * wide)
    }

    /// How high (m over the sea she rides) water climbing at `up` m/s from `height` stands at its top under the sheets'
    /// drag: what a sheet's top reaches.
    fn top_of(height: f64, up: f64) -> f64 {
        let (k, g) = (SHEET_DRAG, GRAVITY);
        let up = up.max(0.0);
        height + up / k - g / (k * k) * (1.0 + k * up / g).ln()
    }

    /// How much of its water a sheet's oldest keeps as it fades (`column_field.wgsl`'s `1 − smoothstep(SHEET_FADE,
    /// SHEET_AGE, age)`).
    fn ageing(age: f64) -> f64 {
        1.0 - smooth((age - SHEET_FADE) / (SHEET_AGE - SHEET_FADE))
    }

    /// Where a point across her side lies by a cross-section's line: how far along it (0–1), and its offset (`on_line`).
    fn on_line(p: DVec2, cross: &Cross) -> (f64, DVec2) {
        let [young, middle, old] = cross.parcels;
        let first = middle.at - young.at;
        let second = old.at - middle.at;
        let u = ((p - young.at).dot(first) / first.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let v = ((p - middle.at).dot(second) / second.length_squared().max(1e-6)).clamp(0.0, 1.0);
        let a = p - (young.at + first * u);
        let b = p - (middle.at + second * v);
        if a.length_squared() <= b.length_squared() {
            (0.5 * u, a)
        } else {
            (0.5 + 0.5 * v, b)
        }
    }

    /// A cross-section's value `along` its line (`along_line`).
    fn along_line(values: [f64; 3], along: f64) -> f64 {
        let [young, middle, old] = values;
        let first = young + (middle - young) * (along * 2.0).clamp(0.0, 1.0);
        first + (old - first) * (along * 2.0 - 1.0).clamp(0.0, 1.0)
    }

    /// `column_field.wgsl`'s sheet mirrored without its noise, at the noise's worst (its fingers thrown farthest, its
    /// clumps densest, its swell at its fullest): its water at `p`, in the box's frame (for the tests).
    fn worst_water(sheet: &BowSheet, p: DVec3) -> f64 {
        let (_, along) = sheet.frame_z();
        let q = DVec3::new(p.x, p.y, p.z * along);
        if q.z <= 0.0 || q.y < -sheet.below || sheet.strength <= 0.0 {
            return 0.0;
        }
        let cross = sheet.cross(q.z);
        if cross.near <= cross.far {
            return 0.0;
        }
        let (at, off) = on_line(DVec2::new(q.x, q.y), &cross);
        let spread = DVec2::new(
            along_line(cross.parcels.map(|p| p.spread.x), at),
            along_line(cross.parcels.map(|p| p.spread.y), at),
        );
        let age = along_line(cross.parcels.map(|p| p.age), at);
        // None past its reach; at its worst its fingers thrown farthest, its fibres, clumps and swell at their fullest.
        if off.length() >= (1.0 + SHEET_FINGERS) * spread.max_element() {
            return 0.0;
        }
        let r = ((off / spread).length() - SHEET_FINGERS).max(0.0);
        let profile = 1.0 - smooth((r - 0.45) / 0.55);
        mass(&cross, spread, sheet) * profile * 1.65 * 1.4 * 1.3 * ageing(age)
    }

    /// A destroyer's starboard sheet at about 37 kn: the crest climbing from the stem to its peak 9 m aft, 2.5 m
    /// over the sea, the water leaving it at 8 to 10 m/s outboard and climbing to the peak's height.
    fn destroyer_sheet() -> BowSheet {
        BowSheet {
            origin: DVec3::new(0.0, 0.0, -56.0),
            out: DVec2::X,
            aft: DVec2::Y,
            source: 0,
            seed: 0.37,
            clock: 12.0,
            lines: [0.3, 1.5, 2.6, 3.4],
            heights: [0.15, 1.3, 2.5, 2.8],
            climbs: [6.7, 5.1, 1.5, 0.0],
            kicks: [0.0; 4],
            shifts: [0.0; 4],
            out_stem: 7.9,
            out_end: 9.6,
            speed: 16.8,
            reach: 11.5,
            strength: 0.8,
            below: 1.5,
        }
    }

    #[test]
    fn a_sheet_rises_off_the_crest_and_falls_back_astern() {
        let sheet = destroyer_sheet();
        // Abreast of each station its youngest water is the crest's own, leaving it.
        for (i, s) in [0.0, 1.0 / 3.0, 2.0 / 3.0].iter().enumerate() {
            let z = sheet.reach * s + 1e-3;
            let young = sheet.cross(z).parcels[0];
            assert!(
                (young.at - DVec2::new(sheet.lines[i], sheet.heights[i])).length() < 0.05,
                "{i}: {young:?}"
            );
            assert!(young.age < 1e-3);
        }
        // It climbs no higher than its stations' water could (about the crest's peak), and is back in the sea astern.
        let mut top: f64 = 0.0;
        let mut landed = None;
        for k in 1..400 {
            let z = 0.1 * f64::from(k);
            let cross = sheet.cross(z);
            let highest = cross
                .parcels
                .iter()
                .map(|p| p.at.y)
                .fold(f64::MIN, f64::max);
            top = top.max(highest);
            if landed.is_none() && highest < 0.0 {
                landed = Some(z);
            }
        }
        let climb = (0..4)
            .map(|i| top_of(sheet.heights[i], sheet.climbs[i]))
            .fold(f64::MIN, f64::max);
        assert!(top <= climb + 0.1 && top > 2.0, "{top} {climb}");
        let landed = landed.expect("back in the sea");
        assert!(landed > 18.0 && landed < 32.0, "{landed}");
        // Outboard of the crest it leaves.
        let cross = sheet.cross(20.0);
        assert!(cross.parcels.iter().all(|p| p.at.x > sheet.lines[0]));
        // Nothing ahead of the stem.
        assert_eq!(worst_water(&sheet, DVec3::new(0.5, 0.5, -0.5)), 0.0);
    }

    #[test]
    fn a_kick_lifts_the_water_it_threw_as_it_runs_aft() {
        // A sea met a second ago: the water thrown then stands higher, the water thrown since does not.
        let calm = destroyer_sheet();
        let kicked = BowSheet {
            kicks: [0.0, 0.0, 3.0, 3.0],
            ..destroyer_sheet()
        };
        let young = 0.15 * calm.speed;
        assert_eq!(calm.cross(young), kicked.cross(young));
        let older = 0.6 * calm.speed + calm.reach * 0.5;
        let lifted = kicked.cross(older).parcels[1].at.y - calm.cross(older).parcels[1].at.y;
        assert!(lifted > 0.5, "{lifted}");
    }

    #[test]
    fn a_raked_stem_moves_where_each_station_throws() {
        // Her stem raked forward over the crest's height: each station's water leaves where the crest stands against
        // her side, forward of the station, and the youngest water abreast of it is the crest's own there.
        let raked = BowSheet {
            shifts: [-0.1, -0.7, -1.2, -1.4],
            ..destroyer_sheet()
        };
        for i in 0..4 {
            let station = raked.reach * i as f64 / 3.0;
            let z = station + raked.shifts[i] + 1e-3;
            if z <= 0.0 {
                continue;
            }
            let young = raked.cross(z).parcels[0];
            assert!(
                (young.at - DVec2::new(raked.lines[i], raked.heights[i])).length() < 0.05,
                "{i}: {young:?}"
            );
            assert!(young.age < 0.01, "{i}: {}", young.age);
        }
        // So its water lies forward of the unraked sheet's by about the shift.
        let z = 12.0;
        assert!(raked.cross(z).parcels[0].age > destroyer_sheet().cross(z).parcels[0].age);
    }

    #[test]
    fn no_water_lies_outside_its_box() {
        // Both sides of a bow, in still water and in a sea she met: just outside every face of the box, nothing the eye
        // could see (the field's noise at its worst), where inside the water stands thick.
        let kicked = BowSheet {
            kicks: [2.5, 1.0, 0.0, 0.0],
            shifts: [-0.1, -0.7, -1.2, -1.4],
            ..destroyer_sheet()
        };
        for (sheet, side) in [
            (destroyer_sheet(), 1.0),
            (destroyer_sheet(), -1.0),
            (kicked.clone(), 1.0),
            (kicked, -1.0),
        ] {
            let sheet = BowSheet {
                out: sheet.out * side,
                ..sheet
            };
            let state = sheet.state();
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
            let mut thickest: f64 = 0.0;
            for n in 0..24 * 24 * 48 {
                let at = DVec3::new(
                    f64::from(n % 24) / 23.0,
                    f64::from(n / 24 % 24) / 23.0,
                    f64::from(n / 576) / 47.0,
                );
                let inside = low + size * at;
                thickest = thickest.max(worst_water(&sheet, inside));
                let mut outside = inside;
                let faces = [at.x, 1.0 - at.x, at.y, 1.0 - at.y, at.z, 1.0 - at.z];
                let (face, _) = faces
                    .iter()
                    .enumerate()
                    .fold((0, f64::MAX), |b, (i, &d)| if d < b.1 { (i, d) } else { b });
                let axis = face / 2;
                outside[axis] = if face % 2 == 0 {
                    low[axis] - 0.05
                } else {
                    high[axis] + 0.05
                };
                if axis == 1 && face == 2 {
                    // Under the box's floor the sea takes it.
                    continue;
                }
                let water = worst_water(&sheet, outside);
                assert!(water < 0.004, "{side}: {water} at {outside} ({state:?})");
            }
            assert!(thickest > 1.0, "{side}: {thickest}");
        }
    }

    #[test]
    fn a_sheet_lies_along_her_side_in_the_world() {
        // Heading north-east: the starboard sheet's box lies outboard to starboard and runs aft from the stem.
        let forward = DVec2::new(1.0, -1.0).normalize();
        let aft = -forward;
        for (side, out) in [
            (1.0, DVec2::new(-forward.y, forward.x)),
            (-1.0, DVec2::new(forward.y, -forward.x)),
        ] {
            let sheet = BowSheet {
                out,
                aft,
                ..destroyer_sheet()
            };
            let state = sheet.state();
            let world = state.world(DVec3::new(3.0, 1.0, 15.0 * sheet.frame_z().1));
            let stem = sheet.origin;
            let offset = DVec2::new(world.x - stem.x, world.z - stem.z);
            assert!((offset.dot(out) - 3.0).abs() < 1e-4, "{side}");
            assert!((offset.dot(aft) - 15.0).abs() < 1e-4, "{side}");
            let (center, _) = state.bounds();
            let middle = DVec2::new(center.x - stem.x, center.z - stem.z);
            assert!(
                middle.dot(aft) > 5.0 && middle.dot(out) > 0.0,
                "{side}: {middle}"
            );
        }
    }

    #[test]
    fn sheets_publish_in_view_and_not_the_hidden_hulls() {
        let mut sheets = BowSheets::new(2);
        sheets.begin();
        sheets.push(destroyer_sheet());
        sheets.push(BowSheet {
            source: 1,
            origin: DVec3::new(0.0, 0.0, -200.0),
            ..destroyer_sheet()
        });
        sheets.push(destroyer_sheet());
        assert_eq!(sheets.sheets().len(), 2);
        let camera = CameraView::look_at(
            DVec3::new(60.0, 10.0, 40.0),
            DVec3::new(0.0, 2.0, -100.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        sheets.publish(&camera, None);
        assert_eq!(sheets.instances().len(), 2);
        assert!(sheets.instances()[0].0 >= sheets.instances()[1].0);
        sheets.publish(&camera, Some(1));
        assert_eq!(sheets.instances().len(), 1);
        let away = CameraView::look_at(
            DVec3::new(60.0, 10.0, 40.0),
            DVec3::new(300.0, 2.0, 200.0),
            40.0,
            0.5,
            60000.0,
            1280.0,
            720.0,
        );
        sheets.publish(&away, None);
        assert!(sheets.instances().is_empty());
        // Each view fades them by how large it draws her bow: none from 3 km, whole from 300 m.
        let view = |from: f64| {
            CameraView::look_at(
                DVec3::new(from, 10.0, -56.0),
                DVec3::new(0.0, 2.0, -56.0),
                40.0,
                0.5,
                60000.0,
                1280.0,
                720.0,
            )
        };
        sheets.publish(&view(3000.0), None);
        assert!(sheets.instances().is_empty());
        sheets.publish(&view(300.0), None);
        let strengths: Vec<f32> = sheets.instances().iter().map(|(_, s)| s.kind[2]).collect();
        assert!(strengths.contains(&0.8), "{strengths:?}");
    }
}
