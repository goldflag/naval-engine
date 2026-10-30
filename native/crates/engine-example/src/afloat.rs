//! Ships afloat: the stand-in (`--afloat <knots>`, a fleet destroyer's numbers sailing a circle, naval-water-effects'
//! proxy hull marking where she is) and the ships from glTF files (`--ship <setup.json>`, their hulls from their
//! models' numbers by `crate::join`, sailing straight on until put about). Each is a naval-afloat hull. The ocean draws
//! the sea they ride about them (`Ocean::set_hull_sea`, the combat sea's sines and their footprints) and they ride it
//! with the field's shorter waves read back from its heights about each (`naval_afloat::DrawnSea`); its wakes take them,
//! the followed ship first as their focus hull, the water effects throw their bows' spray and the combat effects smoke
//! their funnels. Each is the effect crates' own input, as naval-afloat builds it from her pose, speed and size.

use anyhow::{Result, anyhow};
use glam::{DMat4, DVec3, Mat4, Vec3};
use naval_afloat::{Afloat, Course, DrawnSea, Funnel, Hull, ocean_waves};
use naval_effects::ShipFrame;
use naval_effects::sea::SeaWaves;
use naval_ocean::Ocean;
use naval_ocean::hull_sea::{HULL_SEA_SLOTS, HullFootprint, SeaHull, hull_footprints};
use naval_ocean::ship_wake::{WakeCamera, WakeInputs, WakeShip};
use naval_render::{Camera, RenderContext, SceneLayer};
use naval_sky::CelestialLight;
use naval_water_effects::EffectShip;
use naval_water_effects::hulls::{HullDraw, HullLayer, HullLighting};

/// A knot (m/s).
pub const KNOT: f64 = 1852.0 / 3600.0;
/// The stand-in's circle, on the open sea before the window's first view: its centre (world x, z) and radius (m), a
/// destroyer's turning circle at speed.
const CENTRE: (f64, f64) = (-900.0, 700.0);
const RADIUS: f64 = 600.0;
/// A ship's speed's bounds (knots) for the window's keys.
const KNOTS: (f64, f64) = (0.0, 36.0);
/// Her turning circle's radius putting about, in her waterline's lengths (a tactical diameter of five).
const TURNING: f64 = 2.5;

/// How a ship is steered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Helm {
    /// Round a circle to starboard (the stand-in's): its centre (world x, z), its radius (m) and how far round she is
    /// (radians from +x toward +z about its centre).
    Circle {
        centre: (f64, f64),
        radius: f64,
        angle: f64,
    },
    /// Straight on: her origin (world x, z), her heading (radians clockwise from north) and the turn still to make
    /// putting about (radians, to starboard).
    Straight {
        x: f64,
        z: f64,
        heading: f64,
        turn: f64,
    },
}

impl Helm {
    /// Where the helm has her at `speed` (m/s).
    pub fn course(&self, speed: f64) -> Course {
        match *self {
            Helm::Circle {
                centre,
                radius,
                angle,
            } => {
                let (sin, cos) = angle.sin_cos();
                Course {
                    x: centre.0 + radius * cos,
                    z: centre.1 + radius * sin,
                    // Along the tangent (−sin, cos): the heading whose bow (sin h, −cos h) points there.
                    heading: (-sin).atan2(-cos),
                    speed,
                }
            }
            Helm::Straight { x, z, heading, .. } => Course {
                x,
                z,
                heading,
                speed,
            },
        }
    }

    /// Sail `dt` seconds on at `speed` (m/s), turning (if she is putting about) on a circle of [`TURNING`] of her
    /// `length`: with no way on she does not turn.
    pub fn sail(&mut self, speed: f64, dt: f64, length: f64) {
        match self {
            Helm::Circle { radius, angle, .. } => *angle += speed * dt / *radius,
            Helm::Straight {
                x,
                z,
                heading,
                turn,
            } => {
                let step = (speed.abs() * dt / (TURNING * length)).min(*turn);
                // Along the chord: the heading halfway through the step.
                let (sin, cos) = (*heading + step / 2.0).sin_cos();
                *x += sin * speed * dt;
                *z -= cos * speed * dt;
                *heading = (*heading + step).rem_euclid(std::f64::consts::TAU);
                *turn -= step;
            }
        }
    }

    /// Put her about: half a turn to starboard from where she heads (or from where a turn under way would end).
    pub fn put_about(&mut self) {
        if let Helm::Straight { turn, .. } = self {
            *turn += std::f64::consts::PI;
        }
    }
}

/// One ship afloat.
pub struct Sailing {
    /// What the title calls her.
    pub name: String,
    pub afloat: Afloat,
    pub knots: f64,
    pub helm: Helm,
}

impl Sailing {
    /// Where her helm has her.
    pub fn course(&self) -> Course {
        self.helm.course(self.knots * KNOT)
    }

    /// Faster or slower by `knots`.
    pub fn throttle(&mut self, knots: f64) {
        self.knots = (self.knots + knots).clamp(KNOTS.0, KNOTS.1);
    }

    /// The chase camera's eye and target: off her port bow, looking aft along her side, her bow wave and spray near
    /// and her wake and smoke beyond, at distances of her length (for a 110 m destroyer, 95 m ahead, 80 m out and 27 m
    /// up). From where her helm has her, so the window's first view finds her before her first frame.
    pub fn chase(&self) -> (Vec3, Vec3) {
        let course = self.course();
        let length = self.afloat.hull().length;
        let ((fx, fz), (sx, sz)) = course.axes();
        let (forward, starboard) = (DVec3::new(fx, 0.0, fz), DVec3::new(sx, 0.0, sz));
        let at = DVec3::new(course.x, 0.0, course.z);
        let eye = at + forward * (0.85 * length) - starboard * (0.75 * length)
            + DVec3::Y * (0.25 * length);
        let target = at - forward * (0.25 * length) + DVec3::Y * (0.03 * length);
        (eye.as_vec3(), target.as_vec3())
    }
}

/// The ships afloat, the drawn sea's read-backs about them, and the stand-in's proxy.
pub struct Fleet {
    pub ships: Vec<Sailing>,
    /// The ship the window's keys and chase camera follow, and the wakes take first.
    pub followed: usize,
    /// The stand-in's index and her proxy's layer.
    proxy: Option<(usize, HullLayer)>,
    /// The drawn sea's read-backs (and the sines, time and footprints the ocean last coupled them by: what its heights
    /// hold).
    drawn: DrawnSea,
}

impl Fleet {
    pub fn new() -> Self {
        Self {
            ships: Vec::new(),
            followed: 0,
            proxy: None,
            drawn: DrawnSea::new(),
        }
    }

    /// The stand-in's hull: a fleet destroyer's waterline (114 m by 12, 4 m draft, 2,500 t, 36 knots) and her two
    /// funnels' rims 12 m over it.
    pub fn stand_in_hull() -> Hull {
        Hull {
            displacement: Some(2500.0),
            full_speed: Some(36.0 * KNOT),
            funnels: vec![
                Funnel {
                    length: 3.6,
                    ..Funnel::round(DVec3::new(0.0, 12.5, -13.0), 2.6)
                },
                Funnel {
                    length: 3.6,
                    ..Funnel::round(DVec3::new(0.0, 12.0, 3.0), 2.6)
                },
            ],
            ..Hull::new(114.0, 12.0, 4.0)
        }
    }

    /// Put the stand-in to sea at `knots` on her circle, her proxy's layer on the renderer.
    pub fn stand_in(&mut self, ctx: &mut RenderContext<'_>, knots: f64) -> Result<()> {
        let layer = HullLayer::new(ctx).map_err(|e| anyhow!("the proxy hull's shader: {e:?}"))?;
        self.proxy = Some((self.ships.len(), layer));
        self.launch(
            "the stand-in",
            Self::stand_in_hull(),
            knots,
            Helm::Circle {
                centre: CENTRE,
                radius: RADIUS,
                angle: 0.0,
            },
        )
    }

    /// Put `hull` to sea at `knots` under `helm`, called `name` (the effects know her by it, made unique).
    pub fn launch(&mut self, name: &str, hull: Hull, knots: f64, helm: Helm) -> Result<()> {
        let taken = |id: &str| self.ships.iter().any(|s| s.afloat.stir().id() == id);
        let mut id = name.to_owned();
        for n in 2.. {
            if !taken(&id) {
                break;
            }
            id = format!("{name} {n}");
        }
        let afloat = Afloat::new(&id, hull).map_err(|e| anyhow!("{name}'s hull: {e}"))?;
        let (hull, periods) = (afloat.hull(), afloat.hull().periods());
        println!(
            "{id} afloat: {:.1} m waterline, {:.1} m beam, {:.1} m draft, {:.0} t, full speed {:.0} knots; heaves in \
             {:.1} s, pitches in {:.1} s, rolls in {:.1} s",
            hull.length,
            hull.beam,
            hull.draft,
            hull.displacement(),
            hull.full_speed() / KNOT,
            periods.heave,
            periods.pitch,
            periods.roll
        );
        self.ships.push(Sailing {
            name: id,
            afloat,
            knots: knots.clamp(KNOTS.0, KNOTS.1),
            helm,
        });
        Ok(())
    }

    /// The ship the keys and the chase camera follow, if any is afloat.
    pub fn followed(&mut self) -> Option<&mut Sailing> {
        self.ships.get_mut(self.followed)
    }

    /// Follow the next ship.
    pub fn follow_next(&mut self) {
        self.followed = (self.followed + 1) % self.ships.len().max(1);
    }

    /// Sail them `dt` seconds on to the world's `time` on `sea` (the combat sea the ocean couples) and the drawn sea's
    /// last read about each, and hand the ocean the sea they ride and their wakes, for a frame of `camera` at
    /// `viewport`.
    #[allow(clippy::too_many_arguments)]
    pub fn advance(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        dt: f64,
        time: f64,
        sea: &SeaWaves,
        ocean: &mut Ocean,
        camera: &Camera,
        viewport: (u32, u32),
    ) {
        if self.ships.is_empty() {
            return;
        }
        // The heights about them the ocean drew last frame, copied back; the reads that have arrived taken up, each
        // less the sines it was drawn on (last frame's sea, should the wind have changed since).
        if let Some(heights) = &ocean.hull_heights {
            self.drawn.read(device, queue, heights);
        }
        for ship in &mut self.ships {
            ship.helm
                .sail(ship.knots * KNOT, dt, ship.afloat.hull().length);
            let course = ship.course();
            let surface = self.drawn.surface(ship.afloat.stir().id(), sea);
            ship.afloat.update(course, &surface, time);
        }
        // The sea about them gives way to the sines they ride, at the clock they ride them on.
        let eye = camera.eye.as_dvec3();
        let hulls: Vec<SeaHull> = self
            .ships
            .iter()
            .map(|s| s.afloat.stir().sea_hull())
            .collect();
        let footprints = hull_footprints(&hulls, (eye.x, eye.z), HULL_SEA_SLOTS);
        let waves = ocean_waves(sea);
        ocean.set_hull_sea(
            waves.as_ref().map_or(&[][..], |w| &w[..]),
            time,
            &footprints,
            (eye.x, eye.z),
        );
        // Each ship's footprint among those handed over (nearest the camera first, not in their order): where her own
        // alone is.
        let followed: Vec<(&str, HullFootprint)> = self
            .ships
            .iter()
            .zip(&hulls)
            .filter_map(|(ship, hull)| {
                let own =
                    *hull_footprints(std::slice::from_ref(hull), (eye.x, eye.z), 1).first()?;
                let handed = footprints.iter().find(|f| (f.x, f.z) == (own.x, own.z))?;
                Some((ship.afloat.stir().id(), *handed))
            })
            .collect();
        self.drawn.couple(&followed, sea, time);
        let sea_height = ocean.parameters.waves.significant_height;
        if let Some(wake) = ocean.wake.as_mut() {
            let mut ships: Vec<WakeShip<'_>> = self
                .ships
                .iter()
                .map(|s| s.afloat.stir().wake_ship())
                .collect();
            // The focus hull first.
            let focus = self.followed.min(ships.len() - 1);
            ships.swap(0, focus);
            let fov_y = f64::from(camera.fov_y_degrees).to_radians();
            let forward = (camera.target - camera.eye).normalize_or(glam::Vec3::NEG_Z);
            wake.update(&WakeInputs {
                ships: &ships,
                dt,
                splashes: &[],
                torpedoes: &[],
                camera: WakeCamera {
                    eye: eye.to_array(),
                    forward: forward.as_dvec3().to_array(),
                    fov_y,
                    zoom: 1.0 / (fov_y / 2.0).tan(),
                    drawing_height: f64::from(viewport.1),
                },
                sea_height,
            });
        }
    }

    /// Pose the stand-in's proxy for this frame, lit by the sky's `light` and the scene's `ambient`.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        light: &CelestialLight,
        ambient: f64,
    ) {
        let Some((index, hulls)) = &mut self.proxy else {
            return;
        };
        let ship = &self.ships[*index].afloat;
        let stir = ship.stir();
        // Her proxy is about her waterplane's centre; her pose is her origin's.
        let [x, z] = ship.hull().centre;
        let model =
            (stir.water_pose().matrix() * DMat4::from_translation(DVec3::new(x, 0.0, z))).as_mat4();
        let sun = light.color * light.intensity;
        let fill = sun * (0.2 * ambient as f32);
        hulls.set_hulls(
            device,
            queue,
            &[HullDraw {
                shape: *stir.shape(),
                model: Mat4::from_cols_array(&model.to_cols_array()),
            }],
            HullLighting {
                sun_direction: light.direction,
                sun,
                sky_up: fill,
                sky_side: fill * 0.6,
            },
        );
    }

    /// What the combat effects read of them this frame (their funnels' smoke).
    pub fn ship_frames(&self) -> Vec<ShipFrame> {
        self.ships
            .iter()
            .map(|s| s.afloat.stir().ship_frame().clone())
            .collect()
    }

    /// What the water effects read of them this frame (their bows' spray).
    pub fn effect_ships(&self) -> Vec<EffectShip<'_>> {
        self.ships
            .iter()
            .map(|s| s.afloat.stir().effect_ship())
            .collect()
    }

    /// The stand-in's proxy's layer, in the opaque phase, if she is afloat.
    pub fn layer(&mut self) -> Option<&mut dyn SceneLayer> {
        self.proxy
            .as_mut()
            .map(|(_, layer)| layer as &mut dyn SceneLayer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join;

    #[test]
    fn putting_about_turns_her_to_starboard_and_back_along_her_track() {
        // Heading north at 15 knots, put about: she turns through east (her bow to starboard) to south, then holds it.
        let mut helm = Helm::Straight {
            x: 0.0,
            z: 0.0,
            heading: 0.0,
            turn: 0.0,
        };
        helm.put_about();
        let vessel = join::tests::destroyer();
        let mut afloat = Afloat::new("destroyer", join::hull(&vessel)).unwrap();
        let length = afloat.hull().length;
        let (speed, dt) = (15.0 * KNOT, 1.0 / 30.0);
        let (mut headings, mut north) = (Vec::new(), f64::INFINITY);
        for frame in 0..(150.0 / dt) as usize {
            helm.sail(speed, dt, length);
            let course = helm.course(speed);
            headings.push(course.heading);
            north = north.min(course.z);
            afloat.update(course, &naval_afloat::Calm, frame as f64 * dt);
        }
        // Her heading only grows (clockwise from above), to half a turn.
        assert!(headings.windows(2).all(|w| w[1] >= w[0]));
        assert!((headings.last().unwrap() - std::f64::consts::PI).abs() < 1e-9);
        // On a circle of her turning radius about a point off her starboard beam: as far north as that radius, then
        // back south two radii east of where she began.
        let radius = TURNING * length;
        assert!((north + radius).abs() < 0.5, "she went {north:.1} m north");
        let Helm::Straight { x, z, .. } = helm else {
            unreachable!()
        };
        assert!(
            (x - 2.0 * radius).abs() < 0.5 && z > 0.0,
            "she is at {x:.1}, {z:.1}"
        );
        // Her model, drawn at her pose afloat, points where she heads: south.
        let bow = join::pose(&afloat.pose())
            .transform()
            .transform_vector3(DVec3::NEG_Z);
        assert!(bow.distance(DVec3::Z) < 1e-6, "her bow points {bow}");
    }
}
