//! The blast of a heavy turret on the sea under its guns: the sheet of spray its overpressure tears off the water.
//!
//! When a battleship fires, the blast that leaves her muzzles strikes the sea below and ahead of them. Its wind, some
//! tens to hundreds of metres a second behind the shock, flattens the waves and strips a sheet of fine spray off them,
//! which shows as a disc of white water with a sharp rim racing out from under the guns (the photographs of an Iowa's
//! broadside off Vieques in 1984 show it best: a white disc 50–60 m out from her side under each pair of turrets). The
//! water itself barely moves: the pressure pulse lasts a few hundredths of a second and presses the sea down by
//! millimetres, so nothing is laid on the wave field.
//!
//! **The blast's reach.** A muzzle blast is a strong explosion leaning along the bore. Its overpressure falls as
//! r^−[`FALLOFF`] in the middle range that reaches the sea, and off the line of fire as (½ + ½ cos φ)^[`DIRECTIVITY`]
//! (at right angles to the bore about 40 % of what it is along it at the same distance, behind the muzzle a tenth:
//! the measured contours of large guns' blasts). Where it is just strong enough to tear spray off the sea is its
//! *reach*: [`REACH_PER_CALIBRE`] calibres straight ahead for one barrel, growing as the cube root of the barrels
//! firing together (Hopkinson–Cranz: a blast's distances scale as the cube root of its charge, the charge as the cube
//! of the calibre). The sheet is where the sea lies within that reach of the muzzles: an egg under and ahead of them,
//! smaller for guns standing higher over the water or fired at a high elevation, and nothing for a light gun high up.
//!
//! **Its course.** The shock sweeps out from the muzzles at about the speed of sound, so the sheet races out to the
//! blast's reach in a tenth of a second or two; the spray coasts a little further ([`COAST`] of the reach) over half a
//! second, piled at its front into a bright rim. Behind the rim the sheet clears first, the rim lingers and tears into
//! clumps, and the whole drifts downwind with the rest of the spray and is gone in [`LIFE`] seconds. For its first
//! moments the flash and the fireball's glow over it light it orange: the scene's point lights, the renderer's list at
//! group 0 (`naval::lights`).
//!
//! The shading is `shaders/blast.wgsl`: a quad a sheet at the top of its mist, each fragment the column of mist
//! between the quad and the sea (or a hull) behind it on the backdrop's depth, its density the blast's at the foot of
//! the column. This module works out the footprint, ages and drifts the sheets and packs their instances.

use bytemuck::{Pod, Zeroable};
use glam::DVec3;

use crate::fixture::SeaWaves;
use naval_effects::CameraView;

/// The blast's reach over the sea straight ahead of one barrel, per metre of calibre (m): a 16-inch barrel's 39 m, an
/// Iowa's triple turret's 56 m. Fitted to the photographs of her broadside: the sheet reaches 50–55 m out from her
/// side with her guns 10–14 m over the sea.
pub const REACH_PER_CALIBRE: f64 = 95.0;
/// The blast's overpressure falls as r^−FALLOFF over the distances that reach the sea.
pub const FALLOFF: f64 = 1.6;
/// How the blast leans along the bore: (½ + ½ cos φ)^DIRECTIVITY off the line of fire, never below FLOOR (behind the
/// muzzle): 40 % at right angles, a tenth behind.
pub const DIRECTIVITY: f64 = 1.3;
pub const FLOOR: f64 = 0.18;
/// How far past the blast's reach its spray coasts (a share of the reach), and its time constant (s).
pub const COAST: f64 = 0.22;
pub const COAST_SECONDS: f64 = 0.45;
/// Seconds a sheet lasts.
pub const LIFE: f64 = 3.4;
/// The calibres over which the sheet comes in (m): a 5-inch mount's blast barely stirs the sea, an 8-inch turret's
/// tears a sheet off it.
pub const CALIBRES: [f64; 2] = [0.10, 0.24];
/// The share of the spray's drift (the lighting's wind: the ocean's × 0.35, as the splashes' mist) the sheet keeps.
const DRIFT: f64 = 1.0;
/// Sheets kept at once; the oldest goes first.
pub const MAX_SHEETS: usize = 48;
/// The quad lies this far over the water (m), and this share of the significant wave height further: the sheet's
/// mist lies between it and the sea, so only a heavy sea's crests break through it.
const TOP: f64 = 2.0;
const TOP_PER_WAVE: f64 = 0.5;
/// Azimuths and steps out the footprint is searched over for the quad's bounds, and the margin it is given.
const AZIMUTHS: usize = 48;
const STEPS: usize = 40;
const MARGIN: f64 = 1.04;
/// The least slope of a view ray the shader follows down a column (`shaders/blast.wgsl` `RAY_SLOPE`): a column seen at
/// a grazing angle stands this far out from where the ray enters the quad (top ÷ slope), so the quad reaches as far past
/// the footprint.
const RAY_SLOPE: f64 = 0.12;

/// A turret's blast, as the host hands it over (naval-effects' `TurretBlast`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunBlast {
    /// The muzzles' centre (world m) and the line of fire (unit, rising with the guns' elevation).
    pub muzzle: [f64; 3],
    pub bore: [f64; 3],
    pub caliber_m: f64,
    pub barrels: usize,
    /// The first barrel's event sequence: the sheet's own pattern is drawn from it.
    pub sequence: u64,
}

/// The blast's reach straight ahead of `barrels` barrels of `caliber_m` fired together (m).
pub fn reach_ahead(caliber_m: f64, barrels: usize) -> f64 {
    REACH_PER_CALIBRE * caliber_m.max(0.0) * (barrels.max(1) as f64).cbrt()
}

/// The blast's reach along `direction` (unit, from the muzzle) for a bore of `bore` (unit) and a reach `ahead`
/// straight along it: where the overpressure falls to the spray's threshold.
pub fn reach(direction: DVec3, bore: DVec3, ahead: f64) -> f64 {
    let lean = (0.5 + 0.5 * direction.dot(bore)).max(FLOOR);
    ahead * lean.powf(DIRECTIVITY / FALLOFF)
}

/// How far into the blast's reach the sea at `point` lies: its distance from the muzzle over the reach toward it (below
/// 1 inside the reach).
pub fn depth_in_reach(point: DVec3, muzzle: DVec3, bore: DVec3, ahead: f64) -> f64 {
    let v = point - muzzle;
    let d = v.length().max(1e-6);
    d / reach(v / d, bore, ahead).max(1e-6)
}

/// How dense a sheet of `caliber_m` is, 0–1.
pub fn strength(caliber_m: f64) -> f64 {
    let t = ((caliber_m - CALIBRES[0]) / (CALIBRES[1] - CALIBRES[0])).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The footprint on the sea at `level`, at its largest (its spray coasted out): its bounds along the horizontal line
/// of fire (`forward`) from the muzzle's foot, back (negative) and ahead, and its half width across it. None when the
/// blast does not reach the water.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Footprint {
    pub forward: DVec3,
    pub across: DVec3,
    pub back: f64,
    pub ahead: f64,
    pub half_width: f64,
}

pub fn footprint(muzzle: DVec3, bore: DVec3, ahead: f64, level: f64) -> Option<Footprint> {
    if ahead.is_nan() || ahead <= 0.0 || !muzzle.is_finite() {
        return None;
    }
    let forward = DVec3::new(bore.x, 0.0, bore.z)
        .try_normalize()
        .unwrap_or(DVec3::X);
    let across = DVec3::Y.cross(forward);
    let foot = DVec3::new(muzzle.x, level, muzzle.z);
    let edge = 1.0 + COAST;
    let far = ahead * edge * MARGIN;
    let (mut back, mut front, mut side) = (f64::INFINITY, f64::NEG_INFINITY, 0.0f64);
    for a in 0..AZIMUTHS {
        let angle = a as f64 / AZIMUTHS as f64 * std::f64::consts::TAU;
        let direction = forward * angle.cos() + across * angle.sin();
        let mut out = None;
        for s in 0..=STEPS {
            let r = far * s as f64 / STEPS as f64;
            if depth_in_reach(foot + direction * r, muzzle, bore, ahead) <= edge {
                out = Some(r);
            }
        }
        if let Some(r) = out {
            let along = angle.cos() * r;
            back = back.min(along);
            front = front.max(along);
            side = side.max((angle.sin() * r).abs());
        }
    }
    (front > back).then(|| Footprint {
        forward,
        across,
        back: back * MARGIN - far / STEPS as f64,
        ahead: front * MARGIN + far / STEPS as f64,
        half_width: side * MARGIN + far / STEPS as f64,
    })
}

/// One sheet on the sea.
#[derive(Clone, Copy, Debug)]
struct Sheet {
    muzzle: DVec3,
    bore: DVec3,
    ahead: f64,
    strength: f64,
    level: f64,
    top: f64,
    /// The footprint, grown by the columns' reach at a grazing view ([`RAY_SLOPE`]).
    footprint: Footprint,
    /// How far the wind has carried it (m).
    drift: DVec3,
    age: f64,
    seed: f32,
}

/// One sheet as `shaders/blast.wgsl` draws it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct SheetInstance {
    /// The quad's centre on the sheet's top (world m), and the sheet's age (s).
    pub center: [f32; 4],
    /// The quad's full edge along the line of fire, and the blast's reach straight ahead (m).
    pub axis_u: [f32; 4],
    /// The quad's full edge across it, and the sheet's own number (0–1).
    pub axis_v: [f32; 4],
    /// The muzzles' centre (world m), and how dense the sheet is (0–1).
    pub muzzle: [f32; 4],
    /// The line of fire (unit), and the water level the sheet lies on (world y).
    pub bore: [f32; 4],
}

/// Every turret's sheet on the sea.
#[derive(Default)]
pub struct BlastSheets {
    sheets: Vec<Sheet>,
    pending: Vec<GunBlast>,
    instances: Vec<SheetInstance>,
    order: Vec<(f64, usize)>,
}

impl BlastSheets {
    pub fn new() -> Self {
        Self::default()
    }

    /// Turrets fired since the last update: their sheets are laid on the next.
    pub fn queue(&mut self, blasts: &[GunBlast]) {
        self.pending.extend_from_slice(blasts);
    }

    /// Age and drift the sheets by `dt` seconds in `wind` (m/s), then lay the queued blasts' on the sea (`sea`, the
    /// combat sea and its time, places their water level; `waves` is the significant wave height on show).
    pub fn update(&mut self, dt: f64, wind: DVec3, sea: Option<(SeaWaves, f64)>, waves: f64) {
        let carried = DVec3::new(wind.x, 0.0, wind.z) * (DRIFT * dt);
        for sheet in &mut self.sheets {
            sheet.age += dt;
            sheet.drift += carried;
        }
        self.sheets.retain(|sheet| sheet.age < LIFE);
        for blast in std::mem::take(&mut self.pending) {
            let muzzle = DVec3::from(blast.muzzle);
            let Some(bore) = DVec3::from(blast.bore).try_normalize() else {
                continue;
            };
            let strength = strength(blast.caliber_m);
            if strength <= 0.0 {
                continue;
            }
            let level = sea.map_or(0.0, |(sea, time)| sea.height(muzzle.x, muzzle.z, time));
            let ahead = reach_ahead(blast.caliber_m, blast.barrels);
            let Some(mut footprint) = footprint(muzzle, bore, ahead, level) else {
                continue;
            };
            let top = TOP + TOP_PER_WAVE * waves.max(0.0);
            let reach_out = top / RAY_SLOPE;
            footprint.back -= reach_out;
            footprint.ahead += reach_out;
            footprint.half_width += reach_out;
            if self.sheets.len() >= MAX_SHEETS {
                let oldest = (0..self.sheets.len())
                    .max_by(|&a, &b| self.sheets[a].age.total_cmp(&self.sheets[b].age))
                    .expect("full");
                self.sheets.swap_remove(oldest);
            }
            self.sheets.push(Sheet {
                muzzle,
                bore,
                ahead,
                strength,
                level,
                top,
                footprint,
                drift: DVec3::ZERO,
                age: 0.0,
                seed: ((blast.sequence.wrapping_mul(2_654_435_761) >> 8) % 4096) as f32 / 4096.0,
            });
        }
    }

    /// The sheets `camera` sees, farthest first (they are blended over one another).
    pub fn publish(&mut self, camera: &CameraView) {
        self.instances.clear();
        self.order.clear();
        for (index, sheet) in self.sheets.iter().enumerate() {
            let f = &sheet.footprint;
            let middle = DVec3::new(sheet.muzzle.x, sheet.level + sheet.top, sheet.muzzle.z)
                + f.forward * ((f.back + f.ahead) * 0.5)
                + sheet.drift;
            let radius = (f.ahead - f.back).hypot(2.0 * f.half_width) * 0.5;
            if camera.sees_sphere(camera.drawn(middle), radius) {
                self.order
                    .push(((middle - camera.position).length_squared(), index));
            }
        }
        self.order.sort_by(|a, b| b.0.total_cmp(&a.0));
        for &(_, index) in &self.order {
            let sheet = &self.sheets[index];
            let f = &sheet.footprint;
            let middle = DVec3::new(sheet.muzzle.x, sheet.level + sheet.top, sheet.muzzle.z)
                + f.forward * ((f.back + f.ahead) * 0.5)
                + sheet.drift;
            let u = f.forward * (f.ahead - f.back);
            let v = f.across * (2.0 * f.half_width);
            let muzzle = sheet.muzzle + sheet.drift;
            let vec4 = |v: DVec3, w: f64| [v.x as f32, v.y as f32, v.z as f32, w as f32];
            self.instances.push(SheetInstance {
                center: vec4(middle, sheet.age),
                axis_u: vec4(u, sheet.ahead),
                axis_v: [v.x as f32, v.y as f32, v.z as f32, sheet.seed],
                muzzle: vec4(muzzle, sheet.strength),
                bore: vec4(sheet.bore, sheet.level),
            });
        }
    }

    /// The sheets the last publish drew.
    pub fn instances(&self) -> &[SheetInstance] {
        &self.instances
    }

    /// Sheets on the sea.
    pub fn count(&self) -> usize {
        self.sheets.len()
    }

    pub fn reset(&mut self) {
        self.sheets.clear();
        self.pending.clear();
        self.instances.clear();
        self.order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Iowa's triple 16-inch turret trained abeam (+x), her guns 12 m over the sea.
    fn iowa(elevation_deg: f64, height: f64) -> Option<Footprint> {
        let e = elevation_deg.to_radians();
        let bore = DVec3::new(e.cos(), e.sin(), 0.0);
        footprint(
            DVec3::new(0.0, height, 0.0),
            bore,
            reach_ahead(0.406, 3),
            0.0,
        )
    }

    #[test]
    fn a_battleships_broadside_lays_a_sheet_out_from_her_side() {
        let f = iowa(3.0, 12.0).expect("a sheet");
        // Out 55–70 m at its largest (the reach and the spray's coast), back to her side, about as wide as it is long.
        assert!(f.ahead > 55.0 && f.ahead < 75.0, "{f:?}");
        assert!(f.back < 0.0 && f.back > -20.0, "{f:?}");
        assert!(f.half_width > 25.0 && f.half_width < 50.0, "{f:?}");
        assert!((f.forward - DVec3::X).length() < 1e-9);
    }

    #[test]
    fn guns_higher_or_raised_lay_less_and_a_light_gun_high_up_none() {
        let low = iowa(3.0, 10.0).unwrap();
        let high = iowa(3.0, 25.0).unwrap();
        let raised = iowa(35.0, 12.0).unwrap();
        assert!(high.ahead < low.ahead && high.half_width < low.half_width);
        assert!(raised.ahead < low.ahead);
        // A 5-inch single mount 20 m up does not reach the sea.
        let five = footprint(
            DVec3::new(0.0, 20.0, 0.0),
            DVec3::X,
            reach_ahead(0.127, 1),
            0.0,
        );
        assert!(five.is_none(), "{five:?}");
        assert!(strength(0.127) < 0.2);
        assert_eq!(strength(0.406), 1.0);
    }

    #[test]
    fn the_blast_reaches_furthest_along_the_bore() {
        let ahead = reach_ahead(0.406, 3);
        let along = reach(DVec3::X, DVec3::X, ahead);
        let abeam = reach(DVec3::Z, DVec3::X, ahead);
        let behind = reach(DVec3::NEG_X, DVec3::X, ahead);
        assert!((along - ahead).abs() < 1e-9);
        // About 40 % of the overpressure at right angles, a tenth behind (Δp ∝ r^−1.6).
        assert!(
            (abeam / along - 0.4f64.powf(1.0 / FALLOFF)).abs() < 0.03,
            "{}",
            abeam / along
        );
        assert!(
            (behind / along - 0.1f64.powf(1.0 / FALLOFF)).abs() < 0.03,
            "{}",
            behind / along
        );
    }

    #[test]
    fn sheets_are_laid_drift_and_go() {
        let mut sheets = BlastSheets::new();
        let blast = GunBlast {
            muzzle: [100.0, 12.0, -50.0],
            bore: [1.0, 0.05, 0.0],
            caliber_m: 0.406,
            barrels: 3,
            sequence: 7,
        };
        let light = GunBlast {
            caliber_m: 0.076,
            ..blast
        };
        sheets.queue(&[blast, light]);
        sheets.update(1.0 / 60.0, DVec3::new(0.0, 0.0, 10.0), None, 2.0);
        // The 3-inch gun lays nothing.
        assert_eq!(sheets.count(), 1);
        let camera = CameraView::look_at(
            DVec3::new(130.0, 300.0, -50.0),
            DVec3::new(130.0, 0.0, -49.0),
            45.0,
            0.5,
            60000.0,
            1920.0,
            1080.0,
        );
        sheets.publish(&camera);
        let first = sheets.instances()[0];
        for _ in 0..60 {
            sheets.update(1.0 / 60.0, DVec3::new(0.0, 0.0, 10.0), None, 2.0);
        }
        sheets.publish(&camera);
        let later = sheets.instances()[0];
        // A second on: older, carried 10 m downwind, lying where it was across the wind.
        assert!((later.center[3] - first.center[3] - 1.0).abs() < 1e-3);
        assert!(
            (later.muzzle[2] - first.muzzle[2] - 10.0).abs() < 0.05,
            "{later:?}"
        );
        assert!((later.muzzle[0] - first.muzzle[0]).abs() < 1e-6);
        // Its water level and its top over the sea in a 2 m sea.
        assert_eq!(first.bore[3], 0.0);
        assert!((first.center[1] - 3.0).abs() < 1e-6);
        for _ in 0..(LIFE * 60.0) as usize {
            sheets.update(1.0 / 60.0, DVec3::ZERO, None, 2.0);
        }
        assert_eq!(sheets.count(), 0);
    }
}
