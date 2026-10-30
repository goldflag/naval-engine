//! The foam line where each hull meets the water, the CPU half of the web game's `HullContactFoam.ts`: per slot, a bounding
//! circle, the world-to-hull transform of the drawn pose, the hull's waterline profile ([`crate::hull_profile`]) and
//! what drives the line (rest, sea, speed, the bow). The surface shader reads it (`hull_foam` in `shaders/wake.wgsl`) and
//! finds the hull's breadth where the water actually stands against it, so the line hugs the drawn hull at any heave,
//! pitch and roll. The same slots carry each side's height for the shelter the sides give the water beside them (`hull_shelter`),
//! and where the hull's sides are remembered ([`crate::hull_sides`]), which slot of that memory is hers and which of her
//! sides faces the wind, for the white water the water leaves where it has just rushed up her side (the slap).

use glam::{DMat4, DQuat, DVec3, EulerRot};

use crate::hull_profile::{PROFILE_LEVELS, PROFILE_STATIONS, WaterlineProfile};
use crate::hull_sides::HullSides;

/// Hulls with contact foam and wash at once; underway hulls take priority, then camera distance.
pub const HULL_FOAM_SLOTS: usize = 8;
/// Beyond this range the line is narrower than a pixel and fades out anyway.
const REACH: f64 = 4000.0;
/// Empty stations read as this share of the half beam inside the centreline.
const EMPTY: f32 = 0.25;
/// Per slot: a bounding circle, three rows of the world-to-hull matrix, the shape, the drive, the shelter, the slap, one
/// vec4 per level and station (that station's and the next one's starboard and port half-breadths), one per station (its
/// and the next one's side heights).
pub const SLAP: usize = 7;
pub const PROFILE: usize = 8;
pub const SIDES: usize = PROFILE + PROFILE_LEVELS * PROFILE_STATIONS;
pub const STRIDE: usize = SIDES + PROFILE_STATIONS;
/// Distances from a hull's side, in heights of the side above the water, over which its shelter fades out.
const SHELTER_REACH: f64 = 5.0;
/// Share of `REACH` beyond which the shelter fades out with the camera's range.
const SHELTER_RANGE_FADE: f64 = 0.75;
/// Wind speeds (m/s) over which one side of a hull comes to face it: in a calm neither side is the windward one.
const WINDWARD_WIND: [f64; 2] = [1.0, 5.0];
/// Heels (radians, from upright) over which the shelter fades out: past them a side is no longer a wall standing over
/// the water but lies along it, and a capsized hull's sides are under it.
const SHELTER_HEEL: [f64; 2] = [
    35.0 * std::f64::consts::PI / 180.0,
    55.0 * std::f64::consts::PI / 180.0,
];

/// Live-tunable look of the foam that hugs a hull (`HULL_FOAM_TUNING`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HullFoamTuning {
    pub rest: f64,
    pub sea: f64,
    pub underway: f64,
    pub bow: f64,
    pub width: f64,
    pub sea_width: f64,
    pub speed_width: f64,
    pub bow_length: f64,
}

impl Default for HullFoamTuning {
    fn default() -> Self {
        Self {
            rest: 0.12,
            sea: 0.025,
            underway: 0.3,
            bow: 0.4,
            width: 0.9,
            sea_width: 0.1,
            speed_width: 1.2,
            bow_length: 0.3,
        }
    }
}

fn smoothstep(low: f64, high: f64, x: f64) -> f64 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A hull at the water: its drawn pose, speed, dimensions and profile.
#[derive(Clone, Copy, Debug)]
pub struct ContactShip<'a> {
    pub id: &'a str,
    pub position: [f64; 3],
    pub heading: f64,
    pub pitch: f64,
    pub roll: f64,
    pub speed: f64,
    pub beam: f64,
    pub draft: f64,
    pub forward_speed: f64,
    pub profile: &'a WaterlineProfile,
}

/// The contact foam's slots (`HullContactFoam`).
pub struct HullContactFoam {
    pub tuning: HullFoamTuning,
    pub enabled: bool,
    /// Scale of every hull's shelter of the water beside it.
    pub shelter_strength: f64,
    /// Significant wave height of the sea on show (m).
    pub sea_height: f64,
    pub time: f64,
    pub pixel_angle: f64,
    /// Width of the bow system's hull-side wash, which shares this profile and pose.
    pub wash_width: f64,
    /// `HULL_FOAM_SLOTS × STRIDE` vec4s, `count` of them in use.
    pub values: Vec<[f32; 4]>,
    pub count: usize,
    speeds: Vec<(String, f64)>,
    /// The hull in each slot in use, by id.
    held: Vec<String>,
}

impl Default for HullContactFoam {
    fn default() -> Self {
        Self {
            tuning: HullFoamTuning::default(),
            enabled: true,
            shelter_strength: 1.0,
            sea_height: 0.0,
            time: 0.0,
            pixel_angle: 0.001,
            wash_width: crate::bow_waves::BowWaveTuning::default().wash_width,
            values: vec![[0.0; 4]; HULL_FOAM_SLOTS * STRIDE],
            count: 0,
            speeds: Vec::new(),
            held: Vec::new(),
        }
    }
}

impl HullContactFoam {
    /// `camera`: the eye and its vertical field of view (radians); `drawing_height` in pixels.
    pub fn update(
        &mut self,
        ships: &[ContactShip<'_>],
        dt: f64,
        camera: Option<([f64; 3], f64)>,
        drawing_height: f64,
    ) {
        let tuning = self.tuning;
        if dt > 0.0 {
            self.time += dt;
        }
        if let Some((_, fov)) = camera {
            self.pixel_angle = 2.0 * (fov / 2.0).tan() / drawing_height.max(1.0);
        }
        self.speeds
            .retain(|(id, _)| ships.iter().any(|s| s.id == id));
        let mut candidates = Vec::new();
        if self.enabled {
            for ship in ships {
                // A laden surface hull may ride metres below its design waterline; a diving submarine leaves the surface.
                let surface = crate::ship_wake::surfaced(ship.position[1] + ship.draft * 0.6);
                let target = (ship.speed.abs() / ship.forward_speed.max(1.0)).min(1.0) * surface;
                let previous = self
                    .speeds
                    .iter()
                    .find(|(id, _)| id == ship.id)
                    .map_or(target, |(_, s)| *s);
                let speed = if dt > 0.0 {
                    previous + (target - previous) * (1.0 - (-dt / 2.5).exp())
                } else {
                    previous
                };
                match self.speeds.iter_mut().find(|(id, _)| id == ship.id) {
                    Some(entry) => entry.1 = speed,
                    None => self.speeds.push((ship.id.to_owned(), speed)),
                }
                let distance = camera.map_or(0.0, |(eye, _)| {
                    (ship.position[0] - eye[0]).hypot(ship.position[2] - eye[2])
                });
                if distance < REACH && surface > 0.2 {
                    candidates.push((ship, speed, distance, surface));
                }
            }
        }
        // Wash shares these slots with contact foam: stopped hulls must not evict an underway hull's bands.
        candidates.sort_by(|a, b| {
            (b.0.speed > 0.0)
                .cmp(&(a.0.speed > 0.0))
                .then_with(|| a.2.total_cmp(&b.2))
        });
        let mut slot = 0;
        self.held.clear();
        for (ship, speed, _, surface) in candidates {
            if slot >= HULL_FOAM_SLOTS {
                break;
            }
            let sea = self.sea_height.min(8.0);
            let scale = (ship.beam / 32.0).clamp(0.6, 1.3);
            let width =
                (tuning.width + tuning.sea_width * sea + tuning.speed_width * speed) * scale;
            if let Some((eye, _)) = camera
                && self.sub_pixel(width.max(ship.beam * 0.3), ship, eye)
            {
                continue;
            }
            let profile = ship.profile;
            let rotation = DQuat::from_euler(EulerRot::YXZ, -ship.heading, ship.pitch, ship.roll);
            let matrix = DMat4::from_rotation_translation(rotation, DVec3::from(ship.position));
            let inverse = matrix.inverse();
            let base = slot * STRIDE;
            for row in 0..3 {
                let r = inverse.row(row);
                self.values[base + 1 + row] = [r.x as f32, r.y as f32, r.z as f32, r.w as f32];
            }
            self.write_profile(slot, profile);
            let length = f64::from(profile.stern - profile.bow);
            self.values[base + 4] = [
                profile.start,
                profile.spacing,
                profile.half_beam,
                profile.bow,
            ];
            self.values[base + 5] = [
                ((tuning.rest + tuning.sea * sea + tuning.underway * speed) * surface) as f32,
                (tuning.bow * speed * surface) as f32,
                width as f32,
                length.max(1.0) as f32,
            ];
            let range = camera.map_or(0.0, |(eye, _)| {
                (ship.position[0] - eye[0]).hypot(ship.position[2] - eye[2])
            });
            let shelter_reach = SHELTER_REACH * f64::from(profile.tallest_side()).max(1.0);
            let upright = (rotation * DVec3::Y).y;
            self.values[base + 6] = [
                (self.shelter_strength
                    * surface
                    * (1.0 - smoothstep(REACH * SHELTER_RANGE_FADE, REACH, range))
                    * smoothstep(SHELTER_HEEL[1].cos(), SHELTER_HEEL[0].cos(), upright))
                    as f32,
                shelter_reach as f32,
                if ship.speed > 0.0 {
                    smoothstep(0.0, 1.0, speed) as f32
                } else {
                    0.0
                },
                0.0,
            ];
            // Her place in the sides' memory and the wind's side: `HullContactFoam::find_sides`, at the upload.
            self.values[base + SLAP] = [0.0; 4];
            self.held.push(ship.id.to_owned());
            // Every open-water pixel tests these circles alone; only water near a hull reads the rest.
            let mid = f64::from(profile.start)
                + f64::from(profile.spacing) * (PROFILE_STATIONS as f64 - 1.0) / 2.0;
            let hull = (f64::from(profile.spacing) * PROFILE_STATIONS as f64 / 2.0)
                .hypot(f64::from(profile.half_beam));
            // The side wash shares the profile and pose, and may be wider than the contact line.
            let reach = hull + 3.0 * width.max(self.wash_width);
            let shelter = hull + shelter_reach;
            let axis = matrix.z_axis;
            self.values[base] = [
                (matrix.w_axis.x + axis.x * mid) as f32,
                (matrix.w_axis.z + axis.z * mid) as f32,
                (reach * reach) as f32,
                (shelter * shelter) as f32,
            ];
            slot += 1;
        }
        self.count = slot;
    }

    /// The slap row of each slot in use (`hull_foam`'s slap in `shaders/wake.wgsl`): x her slot in the sides' memory
    /// plus one (0: her sides are not remembered, as with no `sides`), y how squarely her starboard side faces the wind
    /// (1 it blows straight onto it, −1 onto her port side, 0 along her or in a calm), the wind blowing toward
    /// `wind_direction` (radians from +x toward +z, `WaveParameters::wind_direction`) at `wind_speed` (m/s). The ocean
    /// calls it after the frame's update and before the wakes' upload, so the slots named are the ones the sides' pass
    /// steps this frame.
    pub fn find_sides(&mut self, sides: Option<&HullSides>, wind_speed: f64, wind_direction: f64) {
        let (toward_z, toward_x) = wind_direction.sin_cos();
        let windy = smoothstep(WINDWARD_WIND[0], WINDWARD_WIND[1], wind_speed);
        for (slot, id) in self.held.iter().enumerate().take(self.count) {
            let base = slot * STRIDE;
            let memory = sides.and_then(|sides| sides.slot(id));
            // The world-to-hull matrix's first row is her starboard axis in the world.
            let [x, _, z, _] = self.values[base + 1].map(f64::from);
            let level = x.hypot(z);
            let lean = if level > 1e-6 && windy > 0.0 {
                -(x * toward_x + z * toward_z) / level * windy
            } else {
                0.0
            };
            self.values[base + SLAP] = [
                memory.map_or(0.0, |memory| (memory + 1) as f32),
                lean as f32,
                0.0,
                0.0,
            ];
        }
    }

    /// A line this far off spans a small share of the three pixels the fragment stage needs.
    fn sub_pixel(&self, width: f64, ship: &ContactShip<'_>, eye: [f64; 3]) -> bool {
        let range = ((ship.position[0] - eye[0]).powi(2)
            + eye[1].powi(2)
            + (ship.position[2] - eye[2]).powi(2))
        .sqrt()
        .max(1.0);
        let finest = range * self.pixel_angle / (eye[1].abs() / range).max(0.04) * 3.0;
        width / finest < 0.05
    }

    fn write_profile(&mut self, slot: usize, profile: &WaterlineProfile) {
        let empty = -EMPTY * profile.half_beam;
        let read = |values: &[f32], i: usize| if values[i].is_nan() { empty } else { values[i] };
        for level in 0..PROFILE_LEVELS {
            for station in 0..PROFILE_STATIONS {
                let i = level * PROFILE_STATIONS + station;
                let next = level * PROFILE_STATIONS + (station + 1).min(PROFILE_STATIONS - 1);
                self.values[slot * STRIDE + PROFILE + i] = [
                    read(&profile.starboard, i),
                    read(&profile.port, i),
                    read(&profile.starboard, next),
                    read(&profile.port, next),
                ];
            }
        }
        for station in 0..PROFILE_STATIONS {
            let next = (station + 1).min(PROFILE_STATIONS - 1);
            self.values[slot * STRIDE + SIDES + station] = [
                profile.starboard_side[station],
                profile.port_side[station],
                profile.starboard_side[next],
                profile.port_side[next],
            ];
        }
    }

    pub fn reset(&mut self) {
        self.speeds.clear();
        self.held.clear();
        self.count = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull_profile::SimpleHull;

    #[test]
    fn underway_wash_keeps_a_slot_among_nearer_stopped_hulls() {
        let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
        let ids: Vec<_> = (0..=HULL_FOAM_SLOTS).map(|i| format!("hull-{i}")).collect();
        let ships: Vec<_> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| ContactShip {
                id,
                position: [0.0, 0.0, 100.0 + 60.0 * i as f64],
                heading: 0.0,
                pitch: 0.0,
                roll: 0.0,
                speed: if i == HULL_FOAM_SLOTS { 15.0 } else { 0.0 },
                beam: 12.0,
                draft: 4.0,
                forward_speed: 18.0,
                profile: &profile,
            })
            .collect();
        let mut foam = HullContactFoam::default();
        foam.update(&ships, 0.0, Some(([0.0, 40.0, 0.0], 0.8)), 1000.0);
        assert_eq!(foam.count, HULL_FOAM_SLOTS);
        assert!(
            (0..foam.count).any(|slot| foam.values[slot * STRIDE + 6][2] > 0.0),
            "eight nearer stopped hulls displaced the underway hull's wash"
        );
        // After the underway hull, keep the nearest stopped hulls in camera-distance order.
        for slot in 1..HULL_FOAM_SLOTS {
            assert_eq!(
                foam.values[slot * STRIDE + 3][3],
                -ships[slot - 1].position[2] as f32
            );
        }
    }

    #[test]
    fn each_slot_names_her_sides_memory_and_the_side_the_wind_blows_onto() {
        let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
        let ship = |id, z: f64, heading: f64| ContactShip {
            id,
            position: [0.0, 0.0, z],
            heading,
            pitch: 0.0,
            roll: 0.0,
            speed: 0.0,
            beam: 12.0,
            draft: 4.0,
            forward_speed: 18.0,
            profile: &profile,
        };
        // Heading 0 her bow lies along −z and her starboard side faces +x; at a quarter turn it faces +z.
        let ships = [
            ship("near", 0.0, 0.0),
            ship("far", 300.0, std::f64::consts::FRAC_PI_2),
        ];
        let mut foam = HullContactFoam::default();
        foam.update(&ships, 0.0, Some(([0.0, 40.0, -100.0], 0.8)), 1000.0);
        assert_eq!(foam.count, 2);
        let row = |foam: &HullContactFoam, slot: usize| foam.values[slot * STRIDE + SLAP];
        // Nothing remembered: no slot named, whatever the wind.
        foam.find_sides(None, 12.0, 0.0);
        assert_eq!((row(&foam, 0)[0], row(&foam, 1)[0]), (0.0, 0.0));
        // Only the far hull is remembered, in the memory's second slot (a stranger holds the first).
        let side = |id| crate::hull_sides::SideHull {
            id,
            position: [0.0, 0.0, 0.0],
            heading: 0.0,
            pitch: 0.0,
            roll: 0.0,
            profile: &profile,
        };
        let mut sides = HullSides::default();
        sides.place(&[side("stranger"), side("far")]);
        // The wind blows toward +x: onto the near hull's port side, along the far one.
        foam.find_sides(Some(&sides), 12.0, 0.0);
        assert_eq!(row(&foam, 0)[0], 0.0);
        assert_eq!(row(&foam, 1)[0], 2.0);
        assert!((row(&foam, 0)[1] + 1.0).abs() < 1e-5, "{:?}", row(&foam, 0));
        assert!(row(&foam, 1)[1].abs() < 1e-5, "{:?}", row(&foam, 1));
        // Toward −z: onto the far hull's starboard side, along the near one.
        foam.find_sides(Some(&sides), 12.0, -std::f64::consts::FRAC_PI_2);
        assert!(row(&foam, 0)[1].abs() < 1e-5);
        assert!((row(&foam, 1)[1] - 1.0).abs() < 1e-5, "{:?}", row(&foam, 1));
        // In a calm neither side faces the wind.
        foam.find_sides(Some(&sides), 0.5, 0.0);
        assert_eq!((row(&foam, 0)[1], row(&foam, 1)[1]), (0.0, 0.0));
        // A new frame's update forgets the rows until the next upload names them again.
        foam.update(&ships, 0.0, Some(([0.0, 40.0, -100.0], 0.8)), 1000.0);
        assert_eq!(row(&foam, 1), [0.0; 4]);
    }

    /// The shelter a hull at `roll` gives the water beside her.
    fn shelter_at(roll: f64) -> f32 {
        let profile = SimpleHull::warship(115.0, 12.0, 4.0).profile();
        let mut foam = HullContactFoam::default();
        let ship = ContactShip {
            id: "fletcher",
            position: [0.0, -0.5, 0.0],
            heading: 0.0,
            pitch: 0.0,
            roll,
            speed: 0.0,
            beam: 12.0,
            draft: 4.0,
            forward_speed: 18.0,
            profile: &profile,
        };
        foam.update(&[ship], 0.0, Some(([0.0, 20.0, 300.0], 0.8)), 1000.0);
        assert_eq!(foam.count, 1);
        foam.values[6][0]
    }

    #[test]
    fn a_capsized_hull_shelters_no_water() {
        assert_eq!(shelter_at(0.0), 1.0);
        assert_eq!(shelter_at(20f64.to_radians()), 1.0);
        assert!(shelter_at(45f64.to_radians()) < 1.0);
        assert_eq!(shelter_at(150f64.to_radians()), 0.0);
        assert_eq!(shelter_at(-90f64.to_radians()), 0.0);
    }
}
