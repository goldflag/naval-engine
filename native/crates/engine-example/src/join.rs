//! A ship from any glTF afloat: naval-vessel's `Vessel` (what her setup file resolves to) to naval-afloat's `Hull`, and
//! her pose afloat back to her model's `VesselPose`. The two functions a host floating its own models copies.
//!
//! The two crates share one ship space (metres, +X to starboard, +Y up from the waterline, −Z ahead, the origin on her
//! waterline amidships) and one pose (heading clockwise from north seen from above, pitch bow up, roll starboard side
//! up, heave up, turned as three's Euler YXZ of −heading, pitch and roll), so nothing here turns an axis or changes a
//! sign: `tests` below hold the model's points to naval-afloat's placing of them.

use glam::DVec3;
use naval_afloat::{Funnel, Hull};
use naval_effects::ShipPose;
use naval_vessel::{Vessel, VesselPose};

/// A funnel's mouth across, a share of her beam, when her setup gives it no radius.
pub const FUNNEL_BEAM: f64 = 0.15;

/// Her hull by the numbers of her model: the waterline's length and its middle where her surfaces cross the sea (her
/// bounds' ends where none does), her beam there (no more than her setup's), her draft to her lowest point, and her
/// funnels' mouths, round (the setup's radius, or [`FUNNEL_BEAM`] of her beam across), each with its share of her
/// smoke (more than 1 smokes as 1). naval-afloat guesses the rest from these: her displacement, freeboard, full speed
/// and metacentric height. Put afloat, it is checked (`Afloat::new` refuses a waterline of no length or a draft of
/// none, as a model wholly above its waterline has).
pub fn hull(vessel: &Vessel) -> Hull {
    let plane = &vessel.waterplane;
    let beam = f64::from(match plane.cut {
        true => plane.beam.min(vessel.beam),
        false => vessel.beam,
    });
    let funnels = vessel
        .funnels
        .iter()
        .map(|funnel| {
            let across = funnel
                .radius
                .map_or(beam * FUNNEL_BEAM, |r| 2.0 * f64::from(r));
            Funnel {
                smoke: f64::from(funnel.smoke).clamp(0.0, 1.0),
                ..Funnel::round(funnel.position.as_dvec3(), across)
            }
        })
        .collect();
    Hull {
        centre: [0.0, f64::from(plane.bow + plane.stern) / 2.0],
        funnels,
        ..Hull::new(f64::from(plane.length()), beam, f64::from(vessel.draft))
    }
}

/// Her model's pose for her pose afloat (`Afloat::pose`, her origin's): the same place, heading, pitch and roll, and
/// her origin's height over the mean sea as her heave.
pub fn pose(afloat: &ShipPose) -> VesselPose {
    VesselPose {
        position: DVec3::new(afloat.x, 0.0, afloat.z),
        heading: afloat.heading,
        pitch: afloat.pitch,
        roll: afloat.roll,
        heave: afloat.y,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use glam::{Mat4, Vec3};
    use naval_afloat::{Afloat, Course, SeaSurface};
    use naval_vessel::{Bounds, Waterplane};

    /// A destroyer as her setup resolves: 112 m overall, her waterline 106 m from 54 m forward of her middle, 10.8 m
    /// across it, a fore funnel of 1.4 m radius, a dummy aft and one smoking more than hard.
    pub fn destroyer() -> Vessel {
        let funnel = |name: &str, z: f32, smoke: f32, radius: Option<f32>| naval_vessel::Funnel {
            name: name.into(),
            position: Vec3::new(0.0, 12.0, z),
            smoke,
            radius,
        };
        Vessel {
            name: "destroyer".into(),
            length: 112.0,
            beam: 11.4,
            draft: 3.6,
            air_draft: 30.0,
            bounds: Bounds {
                min: Vec3::new(-5.7, -3.6, -56.0),
                max: Vec3::new(5.7, 30.0, 56.0),
            },
            model_to_ship: Mat4::IDENTITY,
            scale: 1.0,
            waterplane: Waterplane {
                bow: -54.0,
                stern: 52.0,
                beam: 10.8,
                half_breadths: vec![5.0; naval_vessel::vessel::STATIONS],
                cut: true,
            },
            funnels: vec![
                funnel("fore", -10.0, 1.0, Some(1.4)),
                funnel("dummy", 4.0, 0.0, None),
                funnel("hard", 10.0, 1.6, None),
            ],
            lamps: Vec::new(),
        }
    }

    #[test]
    fn her_hull_is_her_waterline() {
        let hull = hull(&destroyer());
        hull.check().unwrap();
        assert_eq!(
            (hull.length, hull.beam, hull.draft),
            (106.0, f64::from(10.8f32), 3.6f32.into())
        );
        // Her waterline's middle is a metre forward of her model's.
        assert_eq!(hull.centre, [0.0, -1.0]);
        let mouths: Vec<(f64, f64, f64)> = hull
            .funnels
            .iter()
            .map(|f| (f.width, f.length, f.smoke))
            .collect();
        let guessed = hull.beam * FUNNEL_BEAM;
        assert_eq!(
            mouths,
            [
                (2.0 * f64::from(1.4f32), 2.0 * f64::from(1.4f32), 1.0),
                (guessed, guessed, 0.0),
                (guessed, guessed, 1.0),
            ]
        );
        assert_eq!(hull.funnels[0].position, DVec3::new(0.0, 12.0, -10.0));
        // A model none of whose surfaces crossed the sea floats on her bounds' box at the setup's beam.
        let mut uncut = destroyer();
        uncut.waterplane = Waterplane {
            bow: -56.0,
            stern: 56.0,
            beam: 11.4,
            half_breadths: vec![5.7; naval_vessel::vessel::STATIONS],
            cut: false,
        };
        let hull = super::hull(&uncut);
        assert_eq!(
            (hull.length, hull.beam, hull.centre),
            (112.0, f64::from(11.4f32), [0.0, 0.0])
        );
        // One wholly above her waterline has no draft: refused afloat.
        uncut.draft = 0.0;
        assert!(Afloat::new("dry", super::hull(&uncut)).is_err());
    }

    #[test]
    fn her_model_stands_where_afloat_places_her() {
        let points = [
            DVec3::new(0.0, 0.0, -56.0),
            DVec3::new(5.7, 0.0, 0.0),
            DVec3::new(-3.0, 20.0, 40.0),
        ];
        for (i, heading) in [0.0, 0.7, 2.9, -2.2, 5.5].into_iter().enumerate() {
            let afloat = ShipPose {
                x: 120.0 * i as f64 - 300.0,
                y: 0.4 * i as f64 - 0.9,
                z: -75.0 * i as f64 + 40.0,
                heading,
                pitch: 0.03 * i as f64 - 0.05,
                roll: 0.15 - 0.07 * i as f64,
            };
            let model = pose(&afloat).transform();
            for local in points {
                let (drawn, placed) = (model.transform_point3(local), afloat.local_to_world(local));
                assert!(
                    drawn.distance(placed) < 1e-9,
                    "{local} at {afloat:?}: {drawn} against {placed}"
                );
            }
        }
        // Heading 0 is north (−Z), a quarter turn to starboard east (+X); her starboard side then faces south.
        let east = pose(&ShipPose {
            heading: std::f64::consts::FRAC_PI_2,
            ..ShipPose::default()
        })
        .transform();
        assert!(east.transform_vector3(DVec3::NEG_Z).distance(DVec3::X) < 1e-12);
        assert!(east.transform_vector3(DVec3::X).distance(DVec3::Z) < 1e-12);
    }

    /// A long-crested sea of one wave, `amplitude` high and `wavelength` long, running toward `toward` (world x, z).
    struct Swell {
        amplitude: f64,
        wavelength: f64,
        toward: (f64, f64),
    }

    impl SeaSurface for Swell {
        fn height(&self, x: f64, z: f64, time: f64) -> f64 {
            let k = std::f64::consts::TAU / self.wavelength;
            let omega = (naval_afloat::hull::GRAVITY * k).sqrt();
            self.amplitude * (k * (x * self.toward.0 + z * self.toward.1) - omega * time).sin()
        }

        fn shortest_wave(&self) -> Option<f64> {
            Some(self.wavelength)
        }
    }

    /// Her model's two points' heights against the sea's at them, sampled every half second for two minutes after her
    /// first half minute: how often the higher point is the one the sea is higher at, the two differences' correlation,
    /// and her middle's height's with the sea's there.
    fn rides(sea: &Swell, ends: [DVec3; 2], label: &str) -> (f64, f64, f64) {
        let vessel = destroyer();
        let mut afloat = Afloat::new("destroyer", hull(&vessel)).unwrap();
        let course = Course {
            x: 0.0,
            z: 0.0,
            heading: 0.0,
            speed: 0.0,
        };
        let (mut agree, mut samples) = (0, 0);
        let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
        let (mut hxy, mut hxx, mut hyy) = (0.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        for frame in 0..(150.0 / dt) as usize {
            let time = frame as f64 * dt;
            afloat.update(course, sea, time);
            if time < 30.0 || frame % 30 != 0 {
                continue;
            }
            let model = pose(&afloat.pose()).transform();
            let [a, b] = ends.map(|p| model.transform_point3(p));
            let drawn = a.y - b.y;
            let wave = sea.height(a.x, a.z, time) - sea.height(b.x, b.z, time);
            if frame % 240 == 0 {
                let m = afloat.motion();
                eprintln!(
                    "{label} {time:5.1} s: sea {wave:+.2} m higher at the first, she {drawn:+.2} m (heave {:+.2} m, pitch \
                     {:+.2}°, roll {:+.2}°)",
                    m.heave,
                    m.pitch.to_degrees(),
                    m.roll.to_degrees()
                );
            }
            agree += usize::from(drawn * wave > 0.0);
            samples += 1;
            sxy += drawn * wave;
            sxx += drawn * drawn;
            syy += wave * wave;
            let (middle, under) = (
                model.transform_point3(DVec3::ZERO).y,
                sea.height(0.0, 0.0, time),
            );
            hxy += middle * under;
            hxx += middle * middle;
            hyy += under * under;
        }
        (
            agree as f64 / samples as f64,
            sxy / (sxx * syy).sqrt(),
            hxy / (hxx * hyy).sqrt(),
        )
    }

    #[test]
    fn her_bow_rises_as_she_climbs_a_swell() {
        // A 300 m swell 2 m high from dead ahead (she heads north, it runs south): longer than her pitch period, so
        // she lies along its face, her bow up where the sea is higher at her bow.
        let sea = Swell {
            amplitude: 2.0,
            wavelength: 300.0,
            toward: (0.0, 1.0),
        };
        let (agree, correlation, heave) = rides(
            &sea,
            [DVec3::new(0.0, 0.0, -53.0), DVec3::new(0.0, 0.0, 51.0)],
            "bow",
        );
        assert!(
            agree > 0.9 && correlation > 0.9,
            "bow over stern with the sea {agree:.2} of the time, correlation {correlation:.2}"
        );
        // And up with the crest under her.
        assert!(
            heave > 0.9,
            "her middle's height with the sea's, correlation {heave:.2}"
        );
    }

    #[test]
    fn she_rolls_to_the_side_the_wave_lifts() {
        // A 600 m swell 2 m high from her starboard beam (it runs west): her starboard side up where the sea is higher
        // on it.
        let sea = Swell {
            amplitude: 2.0,
            wavelength: 600.0,
            toward: (-1.0, 0.0),
        };
        let (agree, correlation, _) = rides(
            &sea,
            [DVec3::new(5.4, 0.0, 0.0), DVec3::new(-5.4, 0.0, 0.0)],
            "starboard",
        );
        assert!(
            agree > 0.9 && correlation > 0.9,
            "starboard over port with the sea {agree:.2} of the time, correlation {correlation:.2}"
        );
    }
}
