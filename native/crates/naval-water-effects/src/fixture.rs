//! A recorded battle the water effects play back deterministically: per presentation frame, every hull's pose and
//! speed, the running torpedoes, and the one-shot events that throw water (splashes, torpedo hits, aircraft
//! crashes). Recorded from `naval-sim-host`'s render packets (`examples/record.rs`) or from the web game's own ship track
//! (`web/water-stills.ts`), stored as JSON under `.build/native/water-effects/fixtures/`.
//!
//! Units and frames are the simulation's: metres, world Y up, heading clockwise from above with 0 toward −Z, a hull's
//! model frame x starboard and z aft. `speed` is the speed along the bow.

use glam::{DMat4, DQuat, DVec3, EulerRot};
/// The combat sea every hull rides: the effects kit's (`naval_effects::sea`), which the flotsam rides too.
pub use naval_effects::sea::SeaWaves;
use serde::{Deserialize, Serialize};

/// One recorded battle.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recording {
    /// Where it came from, for the record: `naval-sim-host duel seed 7`, `web ocean-review wakeStern`, …
    pub source: String,
    pub map: String,
    pub wind_mps: f64,
    /// The combat sea every hull rides (naval-sim's `SeaState`), when known.
    pub sea: Option<SeaWaves>,
    /// Presentation seconds per frame.
    pub dt: f64,
    pub hulls: Vec<HullInfo>,
    pub frames: Vec<FixtureFrame>,
}

impl Recording {
    pub fn hull(&self, id: &str) -> Option<&HullInfo> {
        self.hulls.iter().find(|hull| hull.id == id)
    }

    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a recording always serialises")
    }
}

/// A hull's dimensions, as the wake and spray read them (`wakeHull(definition.hull)` and `definition.handling`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct HullInfo {
    pub id: String,
    pub preset: String,
    pub length: f64,
    pub beam: f64,
    pub draft: f64,
    /// Full speed ahead and astern (displayed m/s).
    pub forward_speed: f64,
    pub reverse_speed: f64,
    /// The hull's centre in its model frame (`wakeHull`'s `centerX`/`centerZ`; 0 for the premade presets).
    #[serde(default)]
    pub center_x: f64,
    #[serde(default)]
    pub center_z: f64,
}

/// Everything that throws or marks water in one presentation frame.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FixtureFrame {
    /// Simulated seconds at this frame.
    pub time: f64,
    pub ships: Vec<ShipPose>,
    pub torpedoes: Vec<TorpedoPose>,
    /// One-shot events since the previous frame, oldest first.
    pub events: Vec<WaterEvent>,
}

/// A hull's drawn pose and speed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ShipPose {
    pub id: String,
    pub position: [f64; 3],
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    /// Speed along the bow (m/s).
    pub speed: f64,
}

impl ShipPose {
    /// The hull's rotation as the game orients its root: `Euler(pitch, −heading, roll, 'YXZ')`.
    pub fn rotation(&self) -> DQuat {
        DQuat::from_euler(EulerRot::YXZ, -self.heading, self.pitch, self.roll)
    }

    /// Model to world.
    pub fn matrix(&self) -> DMat4 {
        DMat4::from_rotation_translation(self.rotation(), DVec3::from(self.position))
    }

    /// Unit vector along the bow in the horizontal plane: (sin h, 0, −cos h).
    pub fn forward(&self) -> DVec3 {
        DVec3::new(self.heading.sin(), 0.0, -self.heading.cos())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TorpedoPose {
    pub id: i64,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    /// It runs on oxygen: a faint track (`naval_ocean::torpedo_tracks::OXYGEN_TRACK`). The web's recordings have none.
    #[serde(default)]
    pub oxygen: bool,
}

/// What threw the water.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WaterEventKind {
    Splash,
    TorpedoEntry,
    TorpedoHit,
    AircraftCrash,
    DepthChargeBlast,
    DepthChargeSplash,
}

/// A one-shot event, as the combat effects and the wake read a `CombatEvent`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WaterEvent {
    /// The view's event sequence: strictly increasing.
    pub sequence: u64,
    pub kind: WaterEventKind,
    pub position: [f64; 3],
    /// The shell's (or aircraft's) velocity on arrival, when known.
    #[serde(default)]
    pub velocity: Option<[f64; 3]>,
    #[serde(default)]
    pub caliber_m: Option<f64>,
    #[serde(default)]
    pub shell_id: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heading_turns_the_bow() {
        let pose = ShipPose {
            heading: std::f64::consts::FRAC_PI_2,
            ..Default::default()
        };
        // Heading 90° steams toward +X; the model's bow (−Z) must land there.
        let bow = pose.matrix().transform_point3(DVec3::new(0.0, 0.0, -1.0));
        assert!((bow - DVec3::X).length() < 1e-12, "{bow}");
        assert!((pose.forward() - DVec3::X).length() < 1e-12);
    }

    #[test]
    fn recordings_round_trip() {
        let recording = Recording {
            source: "test".into(),
            dt: 0.1,
            frames: vec![FixtureFrame {
                time: 0.1,
                events: vec![WaterEvent {
                    sequence: 3,
                    kind: WaterEventKind::Splash,
                    position: [1.0, 0.0, 2.0],
                    velocity: Some([0.0, -400.0, 100.0]),
                    caliber_m: Some(0.38),
                    shell_id: Some(12),
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let back = Recording::from_json(&recording.to_json()).unwrap();
        assert_eq!(back.frames[0].events, recording.frames[0].events);
    }
}
