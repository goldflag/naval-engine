//! What a model's setup resolves to ([`Vessel`]): her size, her waterline and the points on her the floating, the wake,
//! the smoke and the lamps read, all in ship space; and where she stands in the world ([`VesselPose`]).
//!
//! Ship space is the engine's, as the game's hulls have it: metres, +X starboard, +Y up, −Z ahead (her bow), the origin
//! on the waterline amidships, half way between her bow and stern on her centreline. The world is Y up with −Z north,
//! so a ship heading 0 points her bow north.

use anyhow::{Result, ensure};
use glam::{DMat3, DMat4, DQuat, DVec3, EulerRot, Mat4, Vec3};

use crate::setup::{LampKind, Setup};

/// The waterline's stations, bow to stern ([`Waterplane::half_breadths`]).
pub const STATIONS: usize = 21;

/// When the setup gives neither a waterline nor a draft, her draft is this share of her length (about a warship's).
pub const GUESSED_DRAFT: f32 = 0.04;

/// A box in ship space (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    pub const EMPTY: Bounds = Bounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub fn of(points: impl IntoIterator<Item = Vec3>) -> Self {
        points.into_iter().fold(Self::EMPTY, |b, p| Bounds {
            min: b.min.min(p),
            max: b.max.max(p),
        })
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    pub fn centre(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn is_empty(&self) -> bool {
        !(self.min.cmple(self.max).all())
    }
}

/// Her hull where the sea meets it at rest, from the model's own surfaces cut at y = 0.
#[derive(Clone, Debug, PartialEq)]
pub struct Waterplane {
    /// The waterline's fore and after ends (z, m: the bow's the lesser).
    pub bow: f32,
    pub stern: f32,
    /// Its greatest breadth (m).
    pub beam: f32,
    /// Half her breadth at [`STATIONS`] stations evenly from `bow` to `stern` (m).
    pub half_breadths: Vec<f32>,
    /// False when no surface of the model crossed the waterline (it is wholly above or below the sea, or her hull is
    /// not there): the waterplane is then her bounds' box.
    pub cut: bool,
}

impl Waterplane {
    /// The waterline's length (m).
    pub fn length(&self) -> f32 {
        self.stern - self.bow
    }

    /// Station `i`'s z (m).
    pub fn station(&self, i: usize) -> f32 {
        self.bow + self.length() * i as f32 / (STATIONS - 1) as f32
    }
}

/// A funnel: where her smoke leaves her.
#[derive(Clone, Debug, PartialEq)]
pub struct Funnel {
    pub name: String,
    /// The middle of its mouth, ship space (m).
    pub position: Vec3,
    /// 0 none to 1 a ship steaming hard.
    pub smoke: f32,
    /// Its mouth's radius (m), if the setup gives it.
    pub radius: Option<f32>,
}

/// A navigation lamp.
#[derive(Clone, Debug, PartialEq)]
pub struct Lamp {
    pub name: String,
    /// Ship space (m).
    pub position: Vec3,
    pub kind: LampKind,
    /// Linear RGB, brightest channel 1.
    pub color: Vec3,
    /// Candela.
    pub intensity: f32,
}

impl Lamp {
    /// How much of its light reaches a viewer `bearing` radians off her bow (0 ahead, positive to starboard, ±π
    /// astern): 1 inside its arc, 0 outside, over 2° at its edges (the rules allow up to 3).
    pub fn arc(&self, bearing: f32) -> f32 {
        let edge = 2f32.to_radians();
        let abeam_aft = 112.5f32.to_radians();
        let inside = |from: f32, to: f32| {
            let a = ((bearing - from) / edge + 0.5).clamp(0.0, 1.0);
            let b = ((to - bearing) / edge + 0.5).clamp(0.0, 1.0);
            a.min(b)
        };
        match self.kind {
            LampKind::Masthead => inside(-abeam_aft, abeam_aft),
            LampKind::Side if self.position.x >= 0.0 => inside(0.0, abeam_aft),
            LampKind::Side => inside(-abeam_aft, 0.0),
            LampKind::Stern => 1.0 - inside(-abeam_aft, abeam_aft),
            LampKind::Anchor => 1.0,
        }
    }
}

/// A model's setup resolved against its geometry: what the floating, the wake, the smoke and the lamps of a host read.
/// Everything is in ship space ([`crate::vessel`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Vessel {
    /// The model's file name without its extension.
    pub name: String,
    /// Length overall, bow to stern (m).
    pub length: f32,
    /// The setup's beam, or the model's greatest breadth (m).
    pub beam: f32,
    /// From the waterline down to her lowest point (m).
    pub draft: f32,
    /// From the waterline up to her highest point, her masts' tops (m).
    pub air_draft: f32,
    /// The model's bounds in ship space.
    pub bounds: Bounds,
    /// The model's own coordinates (its scene, after its nodes' transforms) to ship space: the axes turned, scaled to
    /// metres and moved so the origin stands on the waterline amidships. The drawn model's vertices are already in
    /// ship space; this is for a host that reads points off the file itself.
    pub model_to_ship: Mat4,
    /// Metres per model unit.
    pub scale: f32,
    pub waterplane: Waterplane,
    pub funnels: Vec<Funnel>,
    pub lamps: Vec<Lamp>,
}

impl Vessel {
    /// Ship space to the model's own coordinates.
    pub fn ship_to_model(&self) -> Mat4 {
        self.model_to_ship.inverse()
    }

    /// Resolves `setup` against the model's bounds in its own coordinates. The waterplane is left as the bounds' box
    /// until [`Vessel::cut_waterplane`] cuts it from her surfaces.
    pub(crate) fn resolve(
        setup: &Setup,
        name: &str,
        model_bounds: Bounds,
    ) -> Result<(Self, Vec<String>)> {
        ensure!(!model_bounds.is_empty(), "the model draws nothing");
        let mut notes = Vec::new();
        let rotation = setup.rotation();
        // An axis-aligned turn takes a box to a box.
        let turned = Bounds::of(
            [model_bounds.min, model_bounds.max]
                .into_iter()
                .map(|p| rotation * p),
        );
        let model_length = turned.size().z;
        ensure!(
            model_length > 0.0,
            "the model has no length along {}",
            setup.forward
        );
        let scale = match (setup.length, setup.scale) {
            (Some(length), _) => length / model_length,
            (None, Some(scale)) => scale,
            (None, None) => {
                notes.push(format!(
                    "no `length` or `scale`: one model unit is a metre ({model_length:.1} m long)"
                ));
                1.0
            }
        };
        let scaled = Bounds {
            min: turned.min * scale,
            max: turned.max * scale,
        };
        let length = scaled.size().z;
        let waterline = match (setup.waterline, setup.draft) {
            (Some(at), _) => at * setup.up.sign() * scale,
            (None, Some(draft)) => scaled.min.y + draft,
            (None, None) => {
                notes.push(format!(
                    "no `waterline` or `draft`: her draft is guessed at {:.1} m ({} of her length)",
                    length * GUESSED_DRAFT,
                    GUESSED_DRAFT
                ));
                scaled.min.y + length * GUESSED_DRAFT
            }
        };
        let centre = scaled.centre();
        let origin = Vec3::new(centre.x, waterline, centre.z);
        let model_to_ship = Mat4::from_translation(-origin)
            * Mat4::from_scale(Vec3::splat(scale))
            * Mat4::from_mat3(rotation);
        let bounds = Bounds {
            min: scaled.min - origin,
            max: scaled.max - origin,
        };
        let beam = setup.beam.unwrap_or(bounds.size().x);
        let funnels = setup
            .funnels
            .iter()
            .map(|f| Funnel {
                name: f.name.clone(),
                position: Vec3::from(f.at),
                smoke: f.smoke,
                radius: f.radius,
            })
            .collect();
        let lamps = setup
            .lights
            .iter()
            .map(|l| {
                let position = Vec3::from(l.at);
                let color = l.color.map(Vec3::from).unwrap_or(match l.kind {
                    LampKind::Side if position.x < 0.0 => Vec3::new(1.0, 0.03, 0.02),
                    LampKind::Side => Vec3::new(0.05, 1.0, 0.45),
                    _ => Vec3::new(1.0, 0.88, 0.7),
                });
                Lamp {
                    name: l.name.clone(),
                    position,
                    kind: l.kind,
                    color: color / color.max_element().max(1e-6),
                    intensity: l.intensity.unwrap_or(l.kind.intensity()),
                }
            })
            .collect();
        let vessel = Self {
            name: name.to_owned(),
            length,
            beam,
            draft: -bounds.min.y,
            air_draft: bounds.max.y,
            bounds,
            model_to_ship,
            scale,
            waterplane: Waterplane {
                bow: bounds.min.z,
                stern: bounds.max.z,
                beam,
                half_breadths: vec![beam * 0.5; STATIONS],
                cut: false,
            },
            funnels,
            lamps,
        };
        Ok((vessel, notes))
    }

    /// Cuts her waterplane at y = 0 from her triangles in ship space: where each crosses the sea, and her half breadth
    /// at each station the greatest |x| of those crossings there. A hull need not be closed: either side's plating
    /// gives its own crossings.
    pub(crate) fn cut_waterplane(&mut self, positions: &[Vec3], triangles: &[u32]) {
        let mut segments: Vec<[glam::Vec2; 2]> = Vec::new();
        for tri in triangles.as_chunks::<3>().0 {
            let p = [0, 1, 2].map(|k| positions[tri[k] as usize]);
            let mut ends = Vec::with_capacity(2);
            for (a, b) in [(p[0], p[1]), (p[1], p[2]), (p[2], p[0])] {
                if (a.y < 0.0) != (b.y < 0.0) {
                    let t = a.y / (a.y - b.y);
                    let q = a.lerp(b, t);
                    ends.push(glam::Vec2::new(q.x, q.z));
                }
            }
            if ends.len() == 2 {
                segments.push([ends[0], ends[1]]);
            }
        }
        if segments.is_empty() {
            return;
        }
        let (bow, stern) = segments
            .iter()
            .flatten()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                (lo.min(p.y), hi.max(p.y))
            });
        let mut plane = Waterplane {
            bow,
            stern,
            beam: 0.0,
            half_breadths: vec![0.0; STATIONS],
            cut: true,
        };
        for i in 0..STATIONS {
            let z = plane.station(i);
            let mut half = 0f32;
            for [a, b] in &segments {
                let (lo, hi) = (a.y.min(b.y), a.y.max(b.y));
                if z < lo || z > hi {
                    continue;
                }
                let x = if hi - lo < 1e-6 {
                    a.x.abs().max(b.x.abs())
                } else {
                    (a.x + (b.x - a.x) * (z - a.y) / (b.y - a.y)).abs()
                };
                half = half.max(x);
            }
            plane.half_breadths[i] = half;
        }
        plane.beam = segments
            .iter()
            .flatten()
            .fold(0f32, |m, p| m.max(p.x.abs()))
            * 2.0;
        self.waterplane = plane;
    }
}

/// Where a ship stands: her origin (on her waterline amidships) in the world, her heading, and the pitch, roll and
/// heave of her motion on the sea. As the game places its hulls (`hull_matrix`: position, then Euler YXZ of −heading,
/// pitch, roll).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VesselPose {
    /// World (m); y 0 floats her at her waterline on the mean sea.
    pub position: DVec3,
    /// Radians clockwise from north (−Z) seen from above: 0 north, π/2 east.
    pub heading: f64,
    /// Radians, her bow up.
    pub pitch: f64,
    /// Radians, her starboard side up (a heel to port).
    pub roll: f64,
    /// Metres up from `position`.
    pub heave: f64,
}

impl VesselPose {
    /// Ship space to the world.
    pub fn transform(&self) -> DMat4 {
        DMat4::from_rotation_translation(
            DQuat::from_euler(EulerRot::YXZ, -self.heading, self.pitch, self.roll),
            self.position + DVec3::Y * self.heave,
        )
    }
}

/// How a transform turns normals: the inverse transpose of its linear part (a scale stretching her one way tilts her
/// sloped faces' normals the other), over the cube root of its determinant's size so a turn and an even scale give the
/// turn itself. `None` when it flattens her (no inverse).
pub(crate) fn normal_turn(transform: &DMat4) -> Option<DMat3> {
    let m = DMat3::from_mat4(*transform);
    let det = m.determinant();
    (det.is_finite() && det.abs() > 1e-12).then(|| m.inverse().transpose() * det.abs().cbrt())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setup::{Axis, LampSetup};

    fn lamp(kind: LampKind, x: f32) -> Lamp {
        Lamp {
            name: String::new(),
            position: Vec3::new(x, 10.0, 0.0),
            kind,
            color: Vec3::ONE,
            intensity: 1.0,
        }
    }

    #[test]
    fn lamps_show_over_their_arcs() {
        let d = |deg: f32| deg.to_radians();
        let masthead = lamp(LampKind::Masthead, 0.0);
        assert_eq!(masthead.arc(0.0), 1.0);
        assert_eq!(masthead.arc(d(110.0)), 1.0);
        assert_eq!(masthead.arc(d(-115.0)), 0.0);
        let stern = lamp(LampKind::Stern, 0.0);
        assert_eq!(stern.arc(d(180.0)), 1.0);
        assert_eq!(stern.arc(d(-170.0)), 1.0);
        assert_eq!(stern.arc(0.0), 0.0);
        let starboard = lamp(LampKind::Side, 5.0);
        let port = lamp(LampKind::Side, -5.0);
        assert_eq!(starboard.arc(d(60.0)), 1.0);
        assert_eq!(starboard.arc(d(-60.0)), 0.0);
        assert_eq!(port.arc(d(-60.0)), 1.0);
        assert_eq!(port.arc(d(60.0)), 0.0);
        // Right ahead both show, half each at the edge.
        assert!((starboard.arc(0.0) - 0.5).abs() < 1e-6);
        assert_eq!(lamp(LampKind::Anchor, 0.0).arc(d(77.0)), 1.0);
    }

    #[test]
    fn a_setup_resolves_to_her_size_and_waterline() {
        // A model 0..12000 cm long along +X, Z up, 0..900 high, -600..600 wide, her keel at z 0.
        let setup = Setup {
            up: Axis::Z,
            forward: Axis::X,
            length: Some(120.0),
            waterline: Some(400.0),
            lights: vec![LampSetup {
                name: "port".into(),
                at: [-5.0, 9.0, -30.0],
                kind: LampKind::Side,
                color: None,
                intensity: None,
            }],
            ..Setup::new("x.glb")
        };
        let bounds = Bounds {
            min: Vec3::new(0.0, -600.0, 0.0),
            max: Vec3::new(12000.0, 600.0, 900.0),
        };
        let (vessel, notes) = Vessel::resolve(&setup, "x", bounds).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert!((vessel.scale - 0.01).abs() < 1e-6);
        assert!((vessel.length - 120.0).abs() < 1e-3);
        assert!((vessel.beam - 12.0).abs() < 1e-3);
        assert!((vessel.draft - 4.0).abs() < 1e-3);
        assert!((vessel.air_draft - 5.0).abs() < 1e-3);
        // Her bow (the model's x 12000) is at z −60, on the waterline amidships.
        let bow = vessel
            .model_to_ship
            .transform_point3(Vec3::new(12000.0, 0.0, 400.0));
        assert!(bow.abs_diff_eq(Vec3::new(0.0, 0.0, -60.0), 1e-3), "{bow}");
        let starboard = vessel
            .model_to_ship
            .transform_point3(Vec3::new(6000.0, -600.0, 400.0));
        assert!(
            starboard.abs_diff_eq(Vec3::new(6.0, 0.0, 0.0), 1e-3),
            "{starboard}"
        );
        assert!(vessel.lamps[0].color.x > 0.9 && vessel.lamps[0].color.y < 0.1);
        assert_eq!(vessel.lamps[0].intensity, 12.0);
    }

    #[test]
    fn the_waterplane_is_cut_from_her_surfaces() {
        let setup = Setup {
            draft: Some(2.0),
            ..Setup::new("x.glb")
        };
        let bounds = Bounds {
            min: Vec3::new(-5.0, 0.0, -50.0),
            max: Vec3::new(5.0, 8.0, 50.0),
        };
        let (mut vessel, _) = Vessel::resolve(&setup, "x", bounds).unwrap();
        assert!(!vessel.waterplane.cut);
        // One side of a wedge bow to stern: her half breadth 0 at the bow growing to 5 aft, crossing y = 0.
        let positions = [
            Vec3::new(0.0, -2.0, -50.0),
            Vec3::new(0.0, 6.0, -50.0),
            Vec3::new(5.0, -2.0, 50.0),
            Vec3::new(5.0, 6.0, 50.0),
        ];
        vessel.cut_waterplane(&positions, &[0, 2, 1, 1, 2, 3]);
        let plane = &vessel.waterplane;
        assert!(plane.cut);
        assert!((plane.bow + 50.0).abs() < 1e-3 && (plane.stern - 50.0).abs() < 1e-3);
        assert!(plane.half_breadths[0] < 1e-3);
        assert!((plane.half_breadths[STATIONS - 1] - 5.0).abs() < 1e-3);
        assert!((plane.half_breadths[STATIONS / 2] - 2.5).abs() < 1e-3);
        assert!((plane.beam - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_pose_heads_her_bow_and_lifts_her() {
        let pose = VesselPose {
            position: DVec3::new(100.0, 0.0, -20.0),
            heading: std::f64::consts::FRAC_PI_2,
            heave: 0.5,
            ..Default::default()
        };
        let bow = pose
            .transform()
            .transform_point3(DVec3::new(0.0, 0.0, -50.0));
        assert!(
            bow.abs_diff_eq(DVec3::new(150.0, 0.5, -20.0), 1e-9),
            "{bow}"
        );
        let up = VesselPose {
            pitch: 0.1,
            ..Default::default()
        };
        assert!(
            up.transform()
                .transform_point3(DVec3::new(0.0, 0.0, -50.0))
                .y
                > 4.9
        );
        let heel = VesselPose {
            roll: 0.1,
            ..Default::default()
        };
        assert!(
            heel.transform()
                .transform_point3(DVec3::new(5.0, 0.0, 0.0))
                .y
                > 0.49
        );
    }
}
