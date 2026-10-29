//! Water off a hull coming out of the sea: what a lost ship lifts out as she rolls over, rises by an end or floats
//! keel up pours off her deck edges, her sides and her bottom. Not a port.
//!
//! Each lost hull ([`EffectShip::lost`]) is watched at points over her surface, per station of her waterline profile:
//! her deck edges, the turn of her bilge and her keel. A point that has been under the sea a while and comes up well
//! clear of it sheds what it brought up: a deck edge the water standing on her deck, poured off over the edge for some
//! seconds (as green water drains, doubled for a whole deck lifted out at once), her plating the film on it, running
//! off within a few. It falls as the bow's thrown water does (`bow_bursts`, marched with the splashes' columns: no
//! sprites, which read as pale discs): a burst off each draining point every [`THROW_EVERY`] seconds, a fan down and
//! outboard along her side over the stretch about the point, its mist the wind takes. The rule is the water's, not the
//! motion's: a hull heaving in a seaway lifts her water line clear of a crest for a second or two, too briefly and too
//! little to count; a capsizing battleship lifts a whole side and deck out of the sea, and a hull floating keel up has
//! her bottom out of it.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::bow_bursts::{BowBurst, BowBursts, thrown};
use crate::bow_spray::EffectShip;
use crate::fixture::SeaWaves;

/// Seconds under the sea before a point brings water up with it (a wave's crest over her water line is shorter).
const WETTED_SECONDS: f64 = 3.0;
/// How far clear of the sea (m) a point must come before it sheds: more than a heave in a seaway lifts her water line
/// (at least this, and the sea's amplitude more).
const CLEAR_M: f64 = 0.8;
/// Seconds a deck edge pours off the water it brought up (green water's freeing time aboard, `bow_spray`'s 3 s, doubled
/// for a whole deck lifted out at once), and the film on her plating takes to run off.
const DECK_SECONDS: f64 = 6.0;
const FILM_SECONDS: f64 = 2.5;
/// Seconds between a draining point's bursts (each lasts `bow_bursts::BURST_LIFE`), and the least draining that throws.
pub const THROW_EVERY: f64 = 1.5;
const LEAST: f64 = 0.1;
/// Hulls watched: the nearest lost ones.
const MOST_HULLS: usize = 2;
/// Stations along her watched (her profile's interior ones are thinned to this many).
const STATIONS: usize = 8;

/// What a point is: where it lies decides what it brings up and how it falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spot {
    Deck,
    Plating,
}

/// One watched point: where it lies in her frame, and what it has been doing.
#[derive(Clone, Copy, Debug)]
struct Point {
    local: DVec3,
    spot: Spot,
    /// Seconds it has been under the sea (capped), how much it still sheds (0 to 1) and seconds to its next burst.
    wet: f64,
    draining: f64,
    next: f64,
}

#[derive(Clone, Debug)]
struct Watched {
    points: Vec<Point>,
    seen: u64,
    /// Her length over the stations: the stretch of her side each point's burst spreads along.
    stretch: f64,
}

/// The water lost hulls shed as they come out of the sea.
#[derive(Default)]
pub struct HullRunoff {
    hulls: HashMap<String, Watched>,
    frame: u64,
    /// Bursts thrown (for tests and tools).
    pub shed: u64,
}

/// A point's sea height: the combat sea it rides, at the effects' time, or the flat sea.
fn sea_at(sea: &Option<(SeaWaves, f64)>, x: f64, z: f64) -> f64 {
    sea.as_ref()
        .map_or(0.0, |(waves, time)| waves.height(x, z, *time))
}

impl HullRunoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Lost hulls watched now.
    pub fn watched(&self) -> usize {
        self.hulls.len()
    }

    /// How hard hull `id` sheds the sea now, 0 … 1: the most any of her points is draining (her plating's film streams
    /// on after the bursts have stopped: naval-ships' `HullWetting::shedding`).
    pub fn shedding(&self, id: &str) -> f64 {
        self.hulls.get(id).map_or(0.0, |w| {
            w.points.iter().map(|p| p.draining).fold(0.0, f64::max)
        })
    }

    pub fn reset(&mut self) {
        self.hulls.clear();
    }

    /// Her points: her deck edges, bilge turns and keel at [`STATIONS`] stations.
    fn points(ship: &EffectShip<'_>) -> Vec<Point> {
        let profile = ship.profile;
        let levels = naval_ocean::hull_profile::PROFILE_LEVELS;
        let stations = profile.starboard_side.len();
        let mut points = Vec::new();
        let point = |local: DVec3, spot: Spot| Point {
            local,
            spot,
            wet: 0.0,
            draining: 0.0,
            next: 0.0,
        };
        let draft = ship.hull.draft.max(1.0);
        for k in 0..STATIONS {
            // Interior stations only (the profile has an empty one beyond each end), each in the middle of its stretch.
            let station = 1 + ((2 * k + 1) * (stations - 2)) / (2 * STATIONS);
            let z = f64::from(profile.start + profile.spacing * station as f32);
            for (side, sign, deck) in [
                (&profile.starboard, 1.0, profile.starboard_side[station]),
                (&profile.port, -1.0, profile.port_side[station]),
            ] {
                // The half-breadth at a level, where the hull crosses it.
                let breadth = |level: usize| {
                    let value = side[level * stations + station];
                    value.is_finite().then_some(f64::from(value.abs()))
                };
                let Some(water) = breadth(levels / 2) else {
                    continue;
                };
                if deck > 0.0 {
                    let top = breadth(levels - 1).unwrap_or(water);
                    points.push(point(
                        DVec3::new(sign * top, f64::from(deck), z),
                        Spot::Deck,
                    ));
                }
                let bilge = breadth(0).unwrap_or(water * 0.8);
                points.push(point(
                    DVec3::new(sign * bilge * 0.9, -0.65 * draft, z),
                    Spot::Plating,
                ));
            }
            points.push(point(DVec3::new(0.0, -draft, z), Spot::Plating));
        }
        points
    }

    /// A frame of `dt` seconds: every lost hull's points watched against the `sea` (its `amplitude` the combat sea's),
    /// and the water they shed thrown as bursts into `bursts`. The hulls nearest `eye` first.
    pub fn update(
        &mut self,
        ships: &[EffectShip<'_>],
        dt: f64,
        sea: &Option<(SeaWaves, f64)>,
        amplitude: f64,
        eye: DVec3,
        bursts: &mut BowBursts,
    ) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.frame += 1;
        let mut lost: Vec<(f64, &EffectShip<'_>)> = ships
            .iter()
            .filter(|s| s.lost && s.visible)
            .map(|s| {
                let [x, y, z] = s.pose.position;
                ((DVec3::new(x, y, z) - eye).length(), s)
            })
            .collect();
        lost.sort_by(|a, b| a.0.total_cmp(&b.0));
        lost.truncate(MOST_HULLS);
        let clear = CLEAR_M + amplitude.max(0.0);
        for (_, ship) in lost {
            let watched = self
                .hulls
                .entry(ship.pose.id.clone())
                .or_insert_with(|| Watched {
                    points: Self::points(ship),
                    seen: 0,
                    stretch: ship.hull.length.max(10.0) / STATIONS as f64,
                });
            watched.seen = self.frame;
            let rotation = ship.pose.rotation();
            let origin = DVec3::from_array(ship.pose.position);
            let along = rotation * DVec3::Z;
            let stretch = watched.stretch;
            for (k, p) in watched.points.iter_mut().enumerate() {
                // Broken, a point on her other section's side of the break stands where that section is.
                let (origin, rotation) = match ship.section {
                    Some(s) if s.holds(p.local.z) => (DVec3::from_array(s.position), s.rotation),
                    _ => (origin, rotation),
                };
                let world = origin + rotation * p.local;
                let water = sea_at(sea, world.x, world.z);
                let height = world.y - water;
                if height < 0.0 {
                    p.wet = (p.wet + dt).min(60.0);
                    p.draining = 0.0;
                    continue;
                }
                if p.wet >= WETTED_SECONDS && height >= clear {
                    // Up and clear: what it brought up starts to come off, the more the longer it was under.
                    p.draining = (p.wet / 20.0).clamp(0.3, 1.0);
                    p.wet = 0.0;
                    p.next = 0.0;
                } else if height >= clear {
                    p.wet = 0.0;
                }
                let seconds = match p.spot {
                    Spot::Deck => DECK_SECONDS,
                    Spot::Plating => FILM_SECONDS,
                };
                if p.draining >= LEAST {
                    p.next -= dt;
                    if p.next <= 0.0 {
                        p.next += THROW_EVERY;
                        // Outboard from her as she lies now (a keel's, to the side it leans), and along her length.
                        let mut outward = rotation * DVec3::new(p.local.x.signum(), 0.0, 0.0);
                        if p.local.x == 0.0 {
                            outward = rotation * DVec3::Y;
                        }
                        let out = DVec2::new(outward.x, outward.z).normalize_or(DVec2::X);
                        let aft = DVec2::new(along.x, along.z);
                        let aft = (aft - out * aft.dot(out)).normalize_or(DVec2::Y);
                        // A deck's water pours over its edge, outboard and down; a film runs down her plating.
                        let (speed, elevation, width, mist) = match p.spot {
                            Spot::Deck => (3.0, -0.35, 0.45, 0.5),
                            Spot::Plating => (1.5, -1.0, 0.35, 0.3),
                        };
                        let strength = p.draining
                            * match p.spot {
                                Spot::Deck => 0.8,
                                Spot::Plating => 0.4,
                            };
                        bursts.throw(BowBurst {
                            origin: world,
                            out,
                            aft,
                            strength,
                            speed,
                            elevation,
                            width,
                            slowest: 0.4,
                            source: u32::MAX,
                            scatter: 0.6,
                            lead: 0.5 * stretch,
                            length: 0.5 * stretch,
                            below: height + 1.0,
                            mist,
                            ..thrown(k as f64 * 0.618 + self.shed as f64 * 0.137)
                        });
                        self.shed += 1;
                    }
                }
                p.draining *= (-dt / seconds).exp();
            }
        }
        let frame = self.frame;
        self.hulls.retain(|_, w| w.seen == frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bow_spray::EffectShip;
    use crate::fixture::{HullInfo, ShipPose};
    use naval_ocean::hull_profile::SimpleHull;

    fn run(roll_to: f64, lost: bool, sea: Option<(SeaWaves, f64)>) -> (HullRunoff, f64) {
        let hull = HullInfo {
            id: "bismarck".into(),
            preset: "bismarck".into(),
            length: 250.0,
            beam: 36.0,
            draft: 9.5,
            forward_speed: 15.0,
            reverse_speed: 5.0,
            center_x: 0.0,
            center_z: 0.0,
        };
        let profile = SimpleHull::warship(250.0, 36.0, 9.5).profile();
        let mut runoff = HullRunoff::new();
        let mut bursts = BowBursts::new(256);
        let mut shedding = 0.0f64;
        // Thirty seconds as she was, then over to `roll_to` in twelve seconds, then twenty seconds more.
        for frame in 0..62 * 30 {
            let t = f64::from(frame) / 30.0;
            let roll = roll_to * ((t - 30.0) / 12.0).clamp(0.0, 1.0);
            let pose = ShipPose {
                id: "bismarck".into(),
                position: [0.0, -3.0 * (roll / std::f64::consts::PI), 0.0],
                heading: 0.3,
                pitch: 0.0,
                roll,
                speed: 0.0,
            };
            let ship = EffectShip {
                pose: &pose,
                hull: &hull,
                profile: &profile,
                visible: true,
                lost,
                section: None,
            };
            bursts.advance(1.0 / 30.0, DVec3::ZERO);
            let sea = sea.map(|(waves, start)| (waves, start + t));
            runoff.update(
                std::slice::from_ref(&ship),
                1.0 / 30.0,
                &sea,
                sea.map_or(0.0, |(w, _)| w.amplitude_m),
                DVec3::new(0.0, 30.0, 400.0),
                &mut bursts,
            );
            shedding = shedding.max(runoff.shedding("bismarck"));
        }
        (runoff, shedding)
    }

    #[test]
    fn a_hull_rolling_over_pours_water_off_what_comes_out_of_the_sea() {
        // Keel up: her bottom and her rising side came out of the sea and threw what they brought up, and she shed it.
        let (over, shedding) = run(std::f64::consts::PI, true, None);
        assert!(over.shed > 20, "{} bursts", over.shed);
        assert!(shedding > 0.9);
        // Upright and lost in a seaway, heaving with it: nothing.
        let sea = SeaWaves {
            amplitude_m: 0.8,
            wavelength_m: 128.0,
            direction: 0.4,
            wind_mps: 9.0,
            phase: 0.0,
        };
        let (still, _) = run(0.0, true, Some((sea, 0.0)));
        assert_eq!(still.shed, 0);
        // A hull that is not lost sheds nothing, however she rolls.
        let (afloat, _) = run(std::f64::consts::PI, false, None);
        assert_eq!(afloat.shed, 0);
        assert_eq!(afloat.watched(), 0);
    }
}
