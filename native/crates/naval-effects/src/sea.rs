//! The combat sea every hull rides, as plain numbers: what the flotsam and the burning oil ride
//! ([`crate::flotsam::Swell`]) and the water effects place their spray on (`naval-water-effects`' fixtures). The host
//! fills it from the simulation's sea (`naval_effects_sim::sea_waves`).

use serde::{Deserialize, Serialize};

/// naval-sim's combat sea (`SeaState`): two long-crested sines every hull rides. Its heights place spray at the water a
/// bow stands in.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SeaWaves {
    pub amplitude_m: f64,
    pub wavelength_m: f64,
    /// Radians: the waves run toward (cos d, sin d) in x and z.
    pub direction: f64,
    pub wind_mps: f64,
    pub phase: f64,
}

impl SeaWaves {
    /// `SeaState::height` (`crates/naval-sim/src/environment.rs`), term for term.
    pub fn height(&self, x: f64, z: f64, time: f64) -> f64 {
        if self.amplitude_m == 0.0 {
            return 0.0;
        }
        let component = |direction: f64, wavelength: f64, phase: f64| {
            let k = std::f64::consts::TAU / wavelength;
            let omega = (9.81 * k).sqrt();
            (k * (x * direction.cos() + z * direction.sin()) - omega * time + phase).sin()
        };
        self.amplitude_m
            * (0.7 * component(self.direction, self.wavelength_m, self.phase)
                + 0.3
                    * component(
                        self.direction + 0.8,
                        self.wavelength_m * 0.57,
                        self.phase + 2.0,
                    ))
    }
}
