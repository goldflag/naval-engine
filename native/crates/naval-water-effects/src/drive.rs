//! Turning a frame of simulation state (a [`FixtureFrame`], which is what a live render packet reduces to) into what the
//! ocean's wakes read: [`naval_ocean::ship_wake::WakeShip`]s with their proxy profiles, the splashes that ring the trails
//! and the running torpedoes. The vertical slice builds the same inputs from `naval-sim-host`'s packets each frame.

use naval_ocean::hull_profile::{SimpleHull, WaterlineProfile};
use naval_ocean::ship_wake::{WakeShip, WakeSplash};
use naval_ocean::torpedo_tracks::TrackTorpedo;

use crate::fixture::{FixtureFrame, HullInfo, Recording, WaterEventKind};

/// Aircraft wreckage rings the trails as a shell of this calibre would (`FleetWakeFoam.update`).
const AIRCRAFT_CALIBER: f64 = 0.65;
/// A splash with no calibre is a 38 cm shell's.
const DEFAULT_CALIBER: f64 = 0.38;

/// Every hull of a recording with its proxy shape and waterline profile, made once.
pub struct Hulls {
    pub hulls: Vec<(HullInfo, SimpleHull, WaterlineProfile)>,
}

impl Hulls {
    pub fn new(recording: &Recording) -> Self {
        Self {
            hulls: recording
                .hulls
                .iter()
                .map(|info| {
                    let shape = SimpleHull::warship(
                        info.length as f32,
                        info.beam as f32,
                        info.draft as f32,
                    );
                    let profile = shape.profile();
                    (info.clone(), shape, profile)
                })
                .collect(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&(HullInfo, SimpleHull, WaterlineProfile)> {
        self.hulls.iter().find(|(info, ..)| info.id == id)
    }

    /// The wake's view of a frame's hulls, in the frame's order (the focus first).
    pub fn wake_ships<'a>(&'a self, frame: &'a FixtureFrame) -> Vec<WakeShip<'a>> {
        frame
            .ships
            .iter()
            .filter_map(|pose| {
                let (info, _, profile) = self.get(&pose.id)?;
                Some(WakeShip {
                    id: &pose.id,
                    position: pose.position,
                    heading: pose.heading,
                    pitch: pose.pitch,
                    roll: pose.roll,
                    speed: pose.speed,
                    length: info.length,
                    beam: info.beam,
                    draft: info.draft,
                    center_x: info.center_x,
                    center_z: info.center_z,
                    forward_speed: info.forward_speed,
                    reverse_speed: info.reverse_speed,
                    profile: Some(profile),
                    // The fixtures record no damage.
                    oil_leak_m3s: 0.0,
                })
            })
            .collect()
    }
}

/// The frame's splashes and aircraft crashes, as the trails ring them.
pub fn splashes(frame: &FixtureFrame) -> Vec<WakeSplash> {
    frame
        .events
        .iter()
        .filter_map(|event| {
            let caliber_m = match event.kind {
                WaterEventKind::Splash => event.caliber_m.unwrap_or(DEFAULT_CALIBER),
                WaterEventKind::TorpedoEntry => crate::splash::TORPEDO_ENTRY_CALIBER,
                WaterEventKind::AircraftCrash => AIRCRAFT_CALIBER,
                _ => return None,
            };
            Some(WakeSplash {
                sequence: event.sequence,
                x: event.position[0],
                z: event.position[2],
                caliber_m,
            })
        })
        .collect()
}

/// The frame's running torpedoes.
pub fn torpedoes(frame: &FixtureFrame) -> Vec<TrackTorpedo> {
    frame
        .torpedoes
        .iter()
        .map(|t| TrackTorpedo {
            id: t.id,
            position: t.position,
            velocity: t.velocity,
            oxygen: t.oxygen,
        })
        .collect()
}
