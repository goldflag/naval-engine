//! A recorded stretch of a battle, as the effects consume it: the ships' poses, speeds, fires and gun trains at a
//! fixed rate and every event with its tick. `examples/record_fixture.rs` writes one from a `naval-sim-host`
//! battle; [`Fixture::replay`] feeds it to [`CombatEffects`] at 60 Hz, so a test or a capture spawns exactly the
//! same effects on every run.

use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::camera::CameraView;
use crate::combat::{CombatEffects, SIM_DT};
use crate::events::CombatEvent;
use crate::math::ShipPose;
use crate::ships::{FireState, ShipEffectsProfile, ShipFrame};

/// A ship of the fixture: its id and preset (the definition comes from the installed content).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FixtureShip {
    pub id: String,
    pub preset: String,
}

/// One ship at one sample: pose, world velocity and speed, and its fires (index, intensity) where burning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FixtureShipState {
    /// x, y, z, heading, pitch, roll.
    pub pose: [f64; 6],
    pub velocity: [f64; 3],
    pub speed: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rooms: Vec<(u32, f32)>,
    /// (mount index, intensity, train).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<(u32, f32, f64)>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sunk: bool,
    /// Every mount's (train, elevation) in radians, on samples that carry them (once a second): for drawing the
    /// guns where they point; the effects read only burning mounts' trains, above.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attitudes: Vec<(f32, f32)>,
}

/// The ships at one tick, and the events of the ticks since the sample before.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FixtureFrame {
    pub tick: u64,
    pub ships: Vec<FixtureShipState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<CombatEvent>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    /// Where it came from (setup, seed, window).
    pub source: String,
    pub ships: Vec<FixtureShip>,
    /// Ticks between samples.
    pub every: u64,
    pub frames: Vec<FixtureFrame>,
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

impl FixtureShipState {
    /// Rounded to what the effects can show (millimetres, microradians), so the fixture stays small.
    pub fn compact(mut self) -> Self {
        self.pose = self.pose.map(|v| round(v, 4));
        self.velocity = self.velocity.map(|v| round(v, 3));
        self.speed = round(self.speed, 3);
        for m in &mut self.mounts {
            m.2 = round(m.2, 4);
        }
        for a in &mut self.attitudes {
            *a = (
                round(f64::from(a.0), 3) as f32,
                round(f64::from(a.1), 3) as f32,
            );
        }
        self
    }

    fn pose(&self) -> ShipPose {
        let p = self.pose;
        ShipPose {
            x: p[0],
            y: p[1],
            z: p[2],
            heading: p[3],
            pitch: p[4],
            roll: p[5],
        }
    }
}

/// What a replay step hands the caller: the tick and the ships the effects were stepped with.
pub struct ReplayStep<'a> {
    pub tick: u64,
    pub ships: &'a [ShipFrame],
}

impl Fixture {
    /// Ship `ship`'s mount attitudes (train, elevation) at `tick`, interpolated between the samples that carry them.
    pub fn attitudes_at(&self, ship: usize, tick: u64) -> Vec<(f64, f64)> {
        let carrying: Vec<&FixtureFrame> = self
            .frames
            .iter()
            .filter(|f| f.ships.get(ship).is_some_and(|s| !s.attitudes.is_empty()))
            .collect();
        let Some(first) = carrying.first() else {
            return Vec::new();
        };
        let after = carrying.iter().position(|f| f.tick > tick);
        let (a, b) = match after {
            Some(0) => (*first, *first),
            Some(i) => (carrying[i - 1], carrying[i]),
            None => (
                *carrying.last().expect("non-empty"),
                *carrying.last().expect("non-empty"),
            ),
        };
        let alpha = if b.tick > a.tick {
            ((tick.max(a.tick) - a.tick) as f64 / (b.tick - a.tick) as f64).min(1.0)
        } else {
            0.0
        };
        let (sa, sb) = (&a.ships[ship].attitudes, &b.ships[ship].attitudes);
        sa.iter()
            .zip(sb)
            .map(|(x, y)| {
                let tau = std::f64::consts::TAU;
                let (x0, x1) = (f64::from(x.0), f64::from(y.0));
                let train = x0
                    + ((x1 - x0 + std::f64::consts::PI).rem_euclid(tau) - std::f64::consts::PI)
                        * alpha;
                (
                    train,
                    f64::from(x.1) + (f64::from(y.1) - f64::from(x.1)) * alpha,
                )
            })
            .collect()
    }

    /// The first and last tick covered.
    pub fn span(&self) -> (u64, u64) {
        (
            self.frames.first().map_or(0, |f| f.tick),
            self.frames.last().map_or(0, |f| f.tick),
        )
    }

    /// Steps `effects` through the whole fixture at 60 Hz (see [`Replayer`]). `camera(tick, ships)` gives the camera
    /// for each step; `each(step, effects)` runs after every step.
    pub fn replay(
        &self,
        effects: &mut CombatEffects,
        profiles: &HashMap<String, Arc<ShipEffectsProfile>>,
        mut camera: impl FnMut(u64, &[ShipFrame]) -> CameraView,
        mut each: impl FnMut(&ReplayStep<'_>, &mut CombatEffects),
    ) {
        let mut replayer = Replayer::new(self, profiles);
        while let Some(tick) = replayer.step(effects, &mut camera) {
            each(
                &ReplayStep {
                    tick,
                    ships: replayer.ships(),
                },
                effects,
            );
        }
    }
}

/// Steps a fixture one 60 Hz tick at a time: ships interpolated between samples (angles along the shorter arc),
/// fires and trains held from the sample before, each event delivered at its own tick.
pub struct Replayer<'a> {
    fixture: &'a Fixture,
    events: Vec<&'a CombatEvent>,
    next_event: usize,
    frames: Vec<ShipFrame>,
    sample: usize,
    tick: u64,
    last: u64,
}

impl<'a> Replayer<'a> {
    pub fn new(fixture: &'a Fixture, profiles: &HashMap<String, Arc<ShipEffectsProfile>>) -> Self {
        let (first, last) = fixture.span();
        let mut events: Vec<&CombatEvent> = fixture
            .frames
            .iter()
            .flat_map(|f| f.events.iter())
            .collect();
        events.sort_by_key(|e| (e.tick, e.sequence));
        let frames = fixture
            .ships
            .iter()
            .map(|ship| {
                let profile = profiles
                    .get(&ship.preset)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(ShipEffectsProfile::default()));
                ShipFrame::new(&ship.id, profile, ShipPose::default())
            })
            .collect();
        Self {
            fixture,
            events,
            next_event: 0,
            frames,
            sample: 0,
            tick: first,
            last,
        }
    }

    /// The ships as the last step left them.
    pub fn ships(&self) -> &[ShipFrame] {
        &self.frames
    }

    /// The next tick to step, until the fixture ends.
    pub fn tick(&self) -> Option<u64> {
        (self.tick <= self.last && !self.fixture.frames.is_empty()).then_some(self.tick)
    }

    /// Steps `effects` by one tick; `None` once the fixture has ended.
    pub fn step(
        &mut self,
        effects: &mut CombatEffects,
        camera: impl FnOnce(u64, &[ShipFrame]) -> CameraView,
    ) -> Option<u64> {
        let tick = self.tick()?;
        let samples = &self.fixture.frames;
        while self.sample + 1 < samples.len() && samples[self.sample + 1].tick <= tick {
            self.sample += 1;
        }
        let a = &samples[self.sample];
        let b = samples.get(self.sample + 1).unwrap_or(a);
        let alpha = if b.tick > a.tick {
            (tick - a.tick) as f64 / (b.tick - a.tick) as f64
        } else {
            0.0
        };
        for (i, frame) in self.frames.iter_mut().enumerate() {
            let (Some(sa), Some(sb)) = (a.ships.get(i), b.ships.get(i)) else {
                continue;
            };
            let (pa, pb) = (sa.pose(), sb.pose());
            let angle = |x: f64, y: f64| {
                let tau = std::f64::consts::TAU;
                x + ((y - x + std::f64::consts::PI).rem_euclid(tau) - std::f64::consts::PI) * alpha
            };
            frame.pose = ShipPose {
                x: pa.x + (pb.x - pa.x) * alpha,
                y: pa.y + (pb.y - pa.y) * alpha,
                z: pa.z + (pb.z - pa.z) * alpha,
                heading: angle(pa.heading, pb.heading),
                pitch: angle(pa.pitch, pb.pitch),
                roll: angle(pa.roll, pb.roll),
            };
            frame.velocity =
                DVec3::from_array(sa.velocity).lerp(DVec3::from_array(sb.velocity), alpha);
            frame.speed = sa.speed + (sb.speed - sa.speed) * alpha;
            frame.sunk = sa.sunk;
            frame.room_fires.clear();
            for &(index, intensity) in &sa.rooms {
                let index = index as usize;
                if frame.room_fires.len() <= index {
                    frame.room_fires.resize(index + 1, FireState::default());
                }
                frame.room_fires[index] = FireState {
                    intensity: f64::from(intensity),
                    ..FireState::default()
                };
            }
            frame.mount_fires.clear();
            for &(index, intensity, train) in &sa.mounts {
                let index = index as usize;
                if frame.mount_fires.len() <= index {
                    frame.mount_fires.resize(index + 1, FireState::default());
                }
                if frame.mount_trains.len() <= index {
                    frame.mount_trains.resize(index + 1, 0.0);
                }
                frame.mount_fires[index] = FireState {
                    intensity: f64::from(intensity),
                    ..FireState::default()
                };
                frame.mount_trains[index] = train;
            }
        }
        let start = self.next_event;
        while self.next_event < self.events.len() && self.events[self.next_event].tick <= tick {
            self.next_event += 1;
        }
        let due: Vec<CombatEvent> = self.events[start..self.next_event]
            .iter()
            .map(|e| (*e).clone())
            .collect();
        let view = camera(tick, &self.frames);
        effects.update(SIM_DT, tick as f64 * SIM_DT, &view, &due, &self.frames);
        self.tick += 1;
        Some(tick)
    }
}
