//! The sea a hull rides, and the same sea drawn.
//!
//! The wave field (naval-ocean's FFT cascades) is a GPU spectrum no CPU reads. What the game's hulls ride is the
//! combat sea, two long-crested sines (`SeaState` in naval-sim's `environment.rs`, here as naval-effects'
//! [`SeaWaves`]), and the ocean draws it under them: given the sines and the hulls ([`ocean_waves`],
//! `naval_ocean::Ocean::set_hull_sea`), the surface replaces the field's long waves with the sines, everywhere on the
//! realistic sea state and exactly so about each hull (`naval_ocean::hull_sea`), and keeps the field's shorter waves
//! on top. So a host that couples the sea draws what its hulls ride, and the CPU height under a hull is the drawn
//! one less the field's short waves, which a hull's length averages away (`tests/drawn_sea.rs` measures what is
//! left). Without the coupling the drawn sea is the field alone and nothing on the CPU knows it.
//!
//! [`swell`] makes the sines from the drawn sea's own parameters, as the simulation makes them from the same wind:
//! the calibrated significant height in two sines, four times the peak wavelength long, running with the wind.

use naval_effects::sea::SeaWaves;
use naval_ocean::hull_sea::HullSeaWave;
use naval_ocean::spectrum::WaveParameters;

/// What a hull reads of the sea: its height (m above the mean sea) at a world point and time (s).
pub trait SeaSurface {
    fn height(&self, x: f64, z: f64, time: f64) -> f64;

    /// The shortest wave in it (m), so a hull samples it finely enough; none, the finest sampling.
    fn shortest_wave(&self) -> Option<f64> {
        None
    }
}

/// A flat calm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Calm;

impl SeaSurface for Calm {
    fn height(&self, _: f64, _: f64, _: f64) -> f64 {
        0.0
    }

    fn shortest_wave(&self) -> Option<f64> {
        Some(f64::INFINITY)
    }
}

impl SeaSurface for SeaWaves {
    fn height(&self, x: f64, z: f64, time: f64) -> f64 {
        SeaWaves::height(self, x, z, time)
    }

    fn shortest_wave(&self) -> Option<f64> {
        Some(if self.amplitude_m == 0.0 {
            f64::INFINITY
        } else {
            SHORT_SINE * self.wavelength_m
        })
    }
}

/// The second sine's length, a share of the first's (`SeaState::waves`).
const SHORT_SINE: f64 = 0.57;
/// The sines' variance is amplitude² (0.7² + 0.3²) / 2, so Hm0 = 4 √(0.29) amplitude (`SeaState`'s calibration).
const HM0_PER_AMPLITUDE: f64 = 2.154_065_922_853_802;
/// The swell's first sine, a multiple of the wind sea's peak wavelength (`SeaState`: 128 m at 9 m/s).
const SWELL_PER_PEAK: f64 = 4.0;

/// The combat sea for the drawn sea's `waves` (the ocean's `parameters.waves`, the calibrated wind sea): as the
/// simulation makes it of the same wind, its phase from the waves' seed.
pub fn swell(waves: &WaveParameters) -> SeaWaves {
    SeaWaves {
        amplitude_m: waves.significant_height.max(0.0) / HM0_PER_AMPLITUDE,
        wavelength_m: SWELL_PER_PEAK * waves.peak_wavelength.max(1.0),
        direction: waves.wind_direction,
        wind_mps: waves.wind_speed,
        phase: (waves.seed * 0.618_033_988_749_894_9).fract() * std::f64::consts::TAU,
    }
}

/// The combat sea's two sines as the ocean draws them about hulls (`Ocean::set_hull_sea`'s `waves`: the client's
/// `hull_sea_waves`): 0.7 of the amplitude along the sea's direction at its wavelength, 0.3 turned 0.8 rad at 0.57 of
/// it, so the drawn sines are [`SeaWaves::height`] term for term. None on a calm sea.
pub fn ocean_waves(sea: &SeaWaves) -> Option<[HullSeaWave; 2]> {
    if sea.amplitude_m.is_nan() || sea.amplitude_m <= 0.0 {
        return None;
    }
    Some([
        HullSeaWave {
            amplitude: sea.amplitude_m * 0.7,
            wavelength: sea.wavelength_m,
            direction: sea.direction,
            phase: sea.phase,
        },
        HullSeaWave {
            amplitude: sea.amplitude_m * 0.3,
            wavelength: sea.wavelength_m * SHORT_SINE,
            direction: sea.direction + 0.8,
            phase: sea.phase + 2.0,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_ocean::hull_sea::hull_sea_height;

    #[test]
    fn the_drawn_sines_are_the_ones_ridden() {
        let sea = swell(&WaveParameters::default());
        let waves = ocean_waves(&sea).unwrap();
        for (x, z, t) in [
            (0.0, 0.0, 0.0),
            (1234.5, -987.0, 61.25),
            (-40.0, 3000.0, 600.0),
        ] {
            let ridden = sea.height(x, z, t);
            let drawn = hull_sea_height(&waves, t, x, z);
            assert!((ridden - drawn).abs() < 1e-9, "{ridden} {drawn}");
        }
        // Hm0 = 4σ of the sines is the drawn sea's significant height.
        let variance = sea.amplitude_m.powi(2) * (0.7f64.powi(2) + 0.3f64.powi(2)) / 2.0;
        assert!((4.0 * variance.sqrt() - 1.8).abs() < 1e-9);
        assert!(
            ocean_waves(&swell(&WaveParameters {
                significant_height: 0.0,
                ..WaveParameters::default()
            }))
            .is_none()
        );
    }
}
