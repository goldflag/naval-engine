//! The water against each hull's sides, remembered: how high it stands up her side now, how high it has lately climbed
//! (the wet band's memory: plating it reached stays wet, drying downward) and how hard it is rushing up her side (the
//! slap the contact foam whitens with). The ocean keeps it for the hulls whose sea it evaluates ([`crate::hull_heights`]:
//! the nearest [`SIDE_SLOTS`] to the camera), at [`SIDE_STATIONS`] stations from her stem aft on each side, in her own
//! frame, so what she has wetted stays on her plating as she heaves, pitches and rolls.
//!
//! The history lives on the GPU (`shaders/hull_heights.wgsl`, `sides_main`, in the heights' pass): each frame reads the
//! drawn sea (`drawn_height_at` + `wake_height`, as the patches do) where her side meets her design waterline at each
//! station, takes it into her frame and steps the memory by the frame's presentation step (0 holds it, so a paused
//! picture stands). This module keeps the CPU half: which slot each hull holds (hers while she stays among the hulls; a
//! new hull, or one that jumped, starts dry), her pose and waterline, and the pass's uniforms.
//!
//! What the readers find, per slot, station and side: [`SideChannel`] rows of the heights' layer [`SIDES_LAYER`]
//! (`R32Float`), texel (station + side × [`SIDE_STATIONS`], slot × [`SIDE_CHANNELS`] + channel), side 0 port and 1
//! starboard, station 0 at her stem; and her pose's origin and stations in the ships' uniform
//! ([`crate::hull_heights::HullHeightsUniforms::sides`]), to know her by.

use bytemuck::{Pod, Zeroable};
use glam::{DMat4, DQuat, DVec3, EulerRot};

use crate::hull_profile::{PROFILE_LEVEL_STEP, PROFILE_LOWEST, PROFILE_STATIONS, WaterlineProfile};

/// Hulls whose sides are remembered at once: one per patch of the heights.
pub const SIDE_SLOTS: usize = crate::hull_heights::HULL_HEIGHT_SLOTS;
/// Stations along each side, stem to stern over her design waterline: about two metres along a battleship, one along a
/// destroyer, whatever the patches' thinning.
pub const SIDE_STATIONS: u32 = 128;
/// Rows a hull takes in the sides' layer, one per [`SideChannel`].
pub const SIDE_CHANNELS: u32 = 4;
/// The heights' layer that holds the sides (after the patches' [`crate::hull_heights::HULL_HEIGHT_SLOTS`]).
pub const SIDES_LAYER: u32 = crate::hull_heights::HULL_HEIGHT_SLOTS as u32;

/// What each row of a hull's four in the sides' layer holds, per station and side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SideChannel {
    /// The height the water has lately climbed to up her side, in her frame (m over her design waterline): the highest it
    /// has stood, dried downward at [`SIDE_DRYING`]. Never under [`SideChannel::Now`].
    Climb = 0,
    /// How hard the water is rushing up her side (m/s): the fastest it has lately risen against her, fading over
    /// [`SLAP_FADING`]. Zero while it falls or stands.
    Slap = 1,
    /// The water's height up her side this frame, in her frame (m).
    Now = 2,
    /// How fast it rises up her side (m/s, negative falling), eased over [`RISE_EASING`].
    Rise = 3,
}

/// Metres a second the plating the water climbed over dries downward once it falls back: a crest's metre up her side
/// stays wet for about eight seconds, so the band holds the last few crests' reach and runs down between them.
pub const SIDE_DRYING: f32 = 0.12;
/// Seconds over which a slap fades once the water stops rising (to 1/e): the white water a slap leaves against her
/// plating (`hull_foam` in `shaders/wake.wgsl`) lasts two to four seconds after the water rushed up.
pub const SLAP_FADING: f32 = 2.5;
/// Seconds over which the rise is eased (the frames' steps differ; a frame's rise alone jitters).
pub const RISE_EASING: f32 = 0.1;
/// Metres a hull may move between two frames, and radians she may turn, before her history starts again: a staged
/// battle's restart or a jump of the view's subject, never a ship's way (at 40 knots, two metres in a tenth of a second).
pub const SIDE_JUMP: f64 = 25.0;
pub const SIDE_TURN: f64 = 0.5;

/// The level of the profile at the design waterline.
const DESIGN_LEVEL: usize = ((0.0 - PROFILE_LOWEST) / PROFILE_LEVEL_STEP) as usize;

/// A hull whose sides the ocean remembers: one of the hulls the sea around hulls couples (the client hands the same
/// nearest ones), as drawn.
#[derive(Clone, Copy, Debug)]
pub struct SideHull<'a> {
    /// Who she is: her history stays hers while she stays among the hulls (the ships' ids, as the contact foam's).
    pub id: &'a str,
    /// Her drawn pose: the hull's origin (m) and heading, pitch and roll (radians), as her world matrix takes them
    /// (`rotation.set(pitch, −heading, roll, 'YXZ')`, the contact foam's).
    pub position: [f64; 3],
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    /// Her waterline (its ends and half-breadths at the design waterline).
    pub profile: &'a WaterlineProfile,
}

/// `SideHull` in `shaders/hull_heights.wgsl`: one slot of [`HullSidesUniforms`].
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SideHullUniform {
    /// World from hull: her x (starboard), y (up) and z (aft) axes, each with that component of her origin in w.
    pub axes: [[f32; 4]; 3],
    /// x 1 in use, y 1 to start her history afresh, z her stem's z on her design waterline (her frame, m), w a station's
    /// span (m).
    pub stations: [f32; 4],
    /// Her profile's stations: x the first one's z, y 1 / their spacing (m), z their count.
    pub profile: [f32; 4],
    /// Her half-breadths on her design waterline at each profile station, two stations a vec4: starboard, port, the next
    /// station's starboard, port (m, outboard; 0 where she has none).
    pub breadths: [[f32; 4]; PROFILE_STATIONS / 2],
}

/// `HullSides` in `shaders/hull_heights.wgsl`: the slots, then the frame's step and rates.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct HullSidesUniforms {
    pub hulls: [SideHullUniform; SIDE_SLOTS],
    /// x the frame's presentation step (s; 0 holds every history), y [`SIDE_DRYING`], z [`SLAP_FADING`],
    /// w [`RISE_EASING`].
    pub step: [f32; 4],
}

impl Default for HullSidesUniforms {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// A slot's hull, as last placed.
#[derive(Clone, Debug)]
struct Held {
    id: String,
    position: [f64; 3],
    heading: f64,
    /// Her history starts again at the next pass (a new hull in the slot, or one that jumped).
    fresh: bool,
}

/// Which slot each remembered hull holds, and the pass's uniforms (see the module docs).
#[derive(Clone, Debug, Default)]
pub struct HullSides {
    held: [Option<Held>; SIDE_SLOTS],
    uniforms: HullSidesUniforms,
}

impl HullSides {
    /// The hulls to remember from the next pass on (at most [`SIDE_SLOTS`]): each keeps her slot and history while she
    /// stays among them; one gone gives hers back; a new one takes a free slot and starts dry, as does one that jumped
    /// more than [`SIDE_JUMP`] or turned more than [`SIDE_TURN`] since the last call.
    pub fn place(&mut self, hulls: &[SideHull<'_>]) {
        let hulls = &hulls[..hulls.len().min(SIDE_SLOTS)];
        for slot in &mut self.held {
            if slot
                .as_ref()
                .is_some_and(|held| !hulls.iter().any(|hull| hull.id == held.id))
            {
                *slot = None;
            }
        }
        for hull in hulls {
            let slot = match self.slot(hull.id) {
                Some(slot) => slot,
                None => match self.held.iter().position(Option::is_none) {
                    Some(free) => free,
                    None => continue,
                },
            };
            let fresh = match &self.held[slot] {
                Some(held) => {
                    let moved = (hull.position[0] - held.position[0])
                        .hypot(hull.position[2] - held.position[2]);
                    let turned = (hull.heading - held.heading + std::f64::consts::PI)
                        .rem_euclid(std::f64::consts::TAU)
                        - std::f64::consts::PI;
                    held.fresh || !(moved <= SIDE_JUMP && turned.abs() <= SIDE_TURN)
                }
                None => true,
            };
            self.held[slot] = Some(Held {
                id: hull.id.to_owned(),
                position: hull.position,
                heading: hull.heading,
                fresh,
            });
            self.uniforms.hulls[slot] = Self::record(hull, fresh);
        }
        for (slot, held) in self.held.iter().enumerate() {
            if held.is_none() {
                self.uniforms.hulls[slot] = SideHullUniform::zeroed();
            }
        }
    }

    /// The slot `id`'s history is kept in, if she is remembered.
    pub fn slot(&self, id: &str) -> Option<usize> {
        self.held
            .iter()
            .position(|held| held.as_ref().is_some_and(|held| held.id == id))
    }

    /// Whether any hull is remembered.
    pub fn any(&self) -> bool {
        self.held.iter().any(Option::is_some)
    }

    /// The pass's uniforms for a frame `step` seconds after the last (0 holds every history but the fresh ones'):
    /// [`HullSides::passed`] once the pass is encoded.
    pub fn uniforms(&mut self, step: f64) -> &HullSidesUniforms {
        self.uniforms.step = [step.max(0.0) as f32, SIDE_DRYING, SLAP_FADING, RISE_EASING];
        &self.uniforms
    }

    /// The pass has started every fresh history.
    pub fn passed(&mut self) {
        for (slot, held) in self.held.iter_mut().enumerate() {
            if let Some(held) = held {
                held.fresh = false;
                self.uniforms.hulls[slot].stations[1] = 0.0;
            }
        }
    }

    /// The ships' records of the slots ([`crate::hull_heights::HullHeightsUniforms::sides`]): per slot her origin (x,
    /// y, z, w 1 in use) and her stations (x her stem's z, y 1 / a station's span).
    pub fn ship_records(&self) -> [[f32; 4]; SIDE_SLOTS * 2] {
        let mut records = [[0.0; 4]; SIDE_SLOTS * 2];
        for (slot, hull) in self.uniforms.hulls.iter().enumerate() {
            if hull.stations[0] > 0.5 {
                records[slot * 2] = [hull.axes[0][3], hull.axes[1][3], hull.axes[2][3], 1.0];
                records[slot * 2 + 1] = [hull.stations[2], 1.0 / hull.stations[3], 0.0, 0.0];
            }
        }
        records
    }

    fn record(hull: &SideHull<'_>, fresh: bool) -> SideHullUniform {
        let rotation = DQuat::from_euler(EulerRot::YXZ, -hull.heading, hull.pitch, hull.roll);
        let matrix = DMat4::from_rotation_translation(rotation, DVec3::from(hull.position));
        let profile = hull.profile;
        let axis = |column: glam::DVec4, origin: f64| {
            [
                column.x as f32,
                column.y as f32,
                column.z as f32,
                origin as f32,
            ]
        };
        let length = (profile.stern - profile.bow).max(1.0);
        let breadth = |values: &[f32], station: usize| {
            let value = values[DESIGN_LEVEL * PROFILE_STATIONS + station];
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        let mut breadths = [[0.0; 4]; PROFILE_STATIONS / 2];
        for (i, pair) in breadths.iter_mut().enumerate() {
            *pair = [
                breadth(&profile.starboard, 2 * i),
                breadth(&profile.port, 2 * i),
                breadth(&profile.starboard, 2 * i + 1),
                breadth(&profile.port, 2 * i + 1),
            ];
        }
        SideHullUniform {
            axes: [
                axis(matrix.x_axis, hull.position[0]),
                axis(matrix.y_axis, hull.position[1]),
                axis(matrix.z_axis, hull.position[2]),
            ],
            stations: [
                1.0,
                if fresh { 1.0 } else { 0.0 },
                profile.bow,
                length / SIDE_STATIONS as f32,
            ],
            profile: [
                profile.start,
                1.0 / profile.spacing.max(1e-3),
                PROFILE_STATIONS as f32,
                0.0,
            ],
            breadths,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull_profile::SimpleHull;

    fn hull<'a>(id: &'a str, x: f64, profile: &'a WaterlineProfile) -> SideHull<'a> {
        SideHull {
            id,
            position: [x, -0.3, 40.0],
            heading: 1.2,
            pitch: 0.01,
            roll: -0.04,
            profile,
        }
    }

    #[test]
    fn a_hull_keeps_her_slot_while_she_stays_and_a_new_one_starts_dry() {
        let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
        let mut sides = HullSides::default();
        sides.place(&[hull("a", 0.0, &profile), hull("b", 300.0, &profile)]);
        assert_eq!((sides.slot("a"), sides.slot("b")), (Some(0), Some(1)));
        assert_eq!(sides.uniforms(0.016).hulls[1].stations[..2], [1.0, 1.0]);
        sides.passed();
        assert_eq!(sides.uniforms.hulls[1].stations[..2], [1.0, 0.0]);
        // She moves on a frame's way: her history carries on. The other leaves and a third takes the free slot, fresh.
        sides.place(&[hull("c", -500.0, &profile), hull("b", 300.5, &profile)]);
        assert_eq!(
            (sides.slot("a"), sides.slot("b"), sides.slot("c")),
            (None, Some(1), Some(0))
        );
        assert_eq!(sides.uniforms.hulls[1].stations[1], 0.0);
        assert_eq!(sides.uniforms.hulls[0].stations[1], 1.0);
        // Placed twice before a pass: still fresh.
        sides.place(&[hull("c", -500.0, &profile), hull("b", 301.0, &profile)]);
        assert_eq!(sides.uniforms.hulls[0].stations[1], 1.0);
        sides.passed();
        // A jump starts her again; one gone frees her slot.
        sides.place(&[hull("b", 301.0 + SIDE_JUMP * 2.0, &profile)]);
        assert_eq!(sides.uniforms.hulls[1].stations[..2], [1.0, 1.0]);
        assert_eq!(sides.uniforms.hulls[0].stations[0], 0.0);
        assert_eq!(sides.ship_records()[0], [0.0; 4]);
        assert!(sides.any());
        sides.place(&[]);
        assert!(!sides.any());
    }

    #[test]
    fn a_slot_holds_her_pose_and_waterline() {
        let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
        let mut sides = HullSides::default();
        let ship = hull("a", 20.0, &profile);
        sides.place(&[ship]);
        let record = sides.uniforms.hulls[0];
        // Her world matrix (as the fleet's `hull_matrix`): a point in her frame lands where the record's axes put it.
        let matrix = DMat4::from_rotation_translation(
            DQuat::from_euler(EulerRot::YXZ, -ship.heading, ship.pitch, ship.roll),
            DVec3::from(ship.position),
        );
        let p = DVec3::new(5.0, 1.0, -30.0);
        let want = matrix.transform_point3(p);
        let origin = DVec3::from_array([0, 1, 2].map(|k| f64::from(record.axes[k][3])));
        let axes = |k: usize| {
            DVec3::new(
                f64::from(record.axes[k][0]),
                f64::from(record.axes[k][1]),
                f64::from(record.axes[k][2]),
            )
        };
        let placed = origin + axes(0) * p.x + axes(1) * p.y + axes(2) * p.z;
        assert!((placed - want).length() < 1e-4, "{placed} against {want}");
        // Her stations span her design waterline from the stem; the ships find her by her origin.
        assert_eq!(record.stations[2], profile.bow);
        let span = record.stations[3] * SIDE_STATIONS as f32;
        assert!((span - (profile.stern - profile.bow)).abs() < 1e-3);
        assert_eq!(sides.ship_records()[0], [20.0, -0.3, 40.0, 1.0]);
        // Amidships she is her half beam out on both sides.
        let mid = PROFILE_STATIONS / 2;
        let pair = record.breadths[mid / 2];
        assert!(
            (pair[0] - 6.0).abs() < 0.3 && (pair[1] - 6.0).abs() < 0.3,
            "{pair:?}"
        );
    }
}
