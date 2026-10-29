//! What the sea keeps where a ship went down: her Carley floats and cork rafts, a boat or two that floated off, and
//! the planks and timber of her decks and fittings, riding the swell and drifting with the wind. No people: whether
//! survivors are drawn is not decided.
//!
//! The reference is what was seen: Hood's site five minutes after, "one large red raft and a considerable amount of
//! wreckage amidst a huge patch of oil" (Sunderland Z/201, AIR 15/415); two hours after, "wreckage of all
//! descriptions … hammocks, broken rafts, boots, clothes, caps" (Electra's Jack Taylor, hmshood.org.uk); her survivors
//! on "small grey 3 ft square rafts (not Carley rafts)", her Denton cork rafts, of which she carried over twenty.
//! Barham's: "a bubbling, oily-looking patch on the calm surface of the sea, dotted with wreckage" (Cunningham, *A
//! Sailor's Odyssey*, 1951). Queen Mary's: "nothing visible except wreckage" and "a piece of timber".
//!
//! Each piece is drawn as the strike debris is (`shaders/debris.wgsl`: lit, faceted, in the sun's shadow maps), as a
//! shape of its own: a Carley float a rounded ring of its tube, a raft a flat slab, a boat an open hull tapered to her
//! bow, a plank or a baulk a long box. It sits on the combat sea ([`crate::sea::SeaWaves`], the host's from the
//! simulation's `SeaState`: the long swell the drawn sea carries everywhere, `native/docs/ocean/waves.md`, "The swell") at its own freeboard, tilted to its slope,
//! and drifts with the surface (the oil's [`DRIFT_SHARE`] of the wind) and, for what stands out of the water, a
//! leeway of its own.

use glam::{DQuat, DVec3};

use crate::camera::CameraView;
use crate::debris::DebrisInstance;
use crate::math::Rgb;
use crate::particles::SourceId;
use crate::random::Lcg;

/// Pieces the sea keeps at once: a full set gives up its oldest.
pub const CAPACITY: usize = 1024;
/// Seconds a piece is kept: half an hour, as the oil's patches ([`crate::loss`]: the slick's `OIL_LIFETIME`), its
/// last minute spent going under (in fact wreckage floats for days; the scene keeps what an action needs).
pub const LIFETIME: f64 = 1800.0;
/// The surface's drift as a share of the wind (the oil's: the spill models' 3 % rule with the waves' Stokes drift,
/// naval-ocean's `oil::DRIFT_SHARE`): what floats awash goes with the water.
pub const DRIFT_SHARE: f64 = 0.035;

/// The shapes `shaders/debris.wgsl` draws (`DebrisInstance::extents.w`).
pub mod shape {
    /// A strike's shard: a box with its corners pulled in and out.
    pub const SHARD: f32 = 0.0;
    /// Torn plating: a plate folded at a crease (two boxes), crumpled at a second.
    pub const PLATE: f32 = 1.0;
    /// A torn girder: an angle section (flange and web), kinked part way along.
    pub const GIRDER: f32 = 2.0;
    /// A plank or a baulk of timber: a long box, its ends ragged.
    pub const PLANK: f32 = 3.0;
    /// A Carley float: a rounded rectangular ring of its tube.
    pub const CARLEY: f32 = 4.0;
    /// A cork raft: a flat square slab.
    pub const RAFT: f32 = 5.0;
    /// A ship's boat: an open hull tapered to her bow and narrowed to her keel.
    pub const BOAT: f32 = 6.0;
}

/// What a piece is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A Carley float: a ring of copper or steel tube cased in cork and canvas, a grating slung inside. Nos. 17 to
    /// 20, the sizes a capital ship carried, are 5 × 10 ft to 9 × 14 ft with a tube of 16 to 20 in (Ashton, Challenor
    /// and Courtney, Australian War Memorial technical paper, 1993).
    Carley,
    /// A Denton raft: a square of cork about 3 ft across (hmshood.org.uk: Hood's survivors' "small grey 3 ft square
    /// rafts").
    Raft,
    /// A boat that floated off as she went: a 27 ft whaler, a 32 ft cutter, a motor boat of 25 to 35 ft (Hood's in
    /// 1941, the Hood Association's list).
    Boat,
    /// Deck planking: teak or pine, 4 to 6 in wide and about 3 in thick, a few feet to 20 ft long.
    Plank,
    /// A baulk or spar: mess tables, chests, a boom, broken boat timbers.
    Timber,
}

/// One piece on the water.
#[derive(Clone, Debug)]
struct Piece {
    kind: Kind,
    x: f64,
    z: f64,
    /// Heading of its length, radians clockwise from −Z, and how fast it turns (rad/s).
    heading: f64,
    turn: f64,
    /// Half extents: along, up (thickness or tube radius), across (m).
    extents: DVec3,
    /// Its middle's height over the water (m): a float's tube rides half out, a plank awash.
    freeboard: f64,
    albedo: Rgb,
    /// Its own leeway beyond the surface's drift, as a share of the wind.
    leeway: f64,
    /// The way the boil threw it off her (m/s), dying away.
    thrown: DVec3,
    /// When it comes up (the clock), and when it goes.
    born: f64,
    seed: f32,
    source: Option<SourceId>,
}

/// The sea's long swell the pieces ride (the combat sea every hull rides), at the effects' clock.
#[derive(Clone, Debug)]
pub struct Swell {
    pub sea: crate::sea::SeaWaves,
    /// The sea's clock (s) this frame: the battle's (the host sets it each frame).
    pub time: f64,
}

impl Swell {
    fn height(&self, x: f64, z: f64, time: f64) -> f64 {
        self.sea.height(x, z, time)
    }
}

/// Where a lost ship's flotsam comes from: her length and beam, where she went under and her heading.
#[derive(Clone, Copy, Debug)]
pub struct Scatter {
    pub x: f64,
    pub z: f64,
    pub heading: f64,
    pub length: f64,
    pub beam: f64,
}

/// The pieces on the water.
#[derive(Clone, Debug, Default)]
pub struct Flotsam {
    pieces: Vec<Piece>,
    time: f64,
    /// Fraction of the pieces scattered (the combat effects setting).
    pub density: f64,
    thinning: f64,
}

/// How many of each a ship of `length` m leaves: estimates, scaled from Hood's (262 m): ten or so Carley floats and
/// over twenty cork rafts aboard (a model-maker's study of her 1941 fit; "over 20 Denton rafts"), a few of her sixteen
/// boats floating off, and her decks' planking and timber, "a considerable amount of wreckage". A destroyer of 118 m
/// leaves under half of that.
pub fn counts(length: f64) -> [(Kind, usize); 5] {
    let l = length.max(20.0);
    [
        (Kind::Carley, (l / 22.0).round() as usize),
        (Kind::Raft, (l / 12.0).round() as usize),
        (Kind::Boat, (l / 100.0).round().max(1.0) as usize),
        (Kind::Plank, (l / 1.6).round() as usize),
        (Kind::Timber, (l / 7.0).round() as usize),
    ]
}

impl Flotsam {
    pub fn new() -> Self {
        Self {
            density: 1.0,
            ..Self::default()
        }
    }

    /// Pieces on the water now (or still to come up).
    pub fn len(&self) -> usize {
        self.pieces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pieces.is_empty()
    }

    pub fn reset(&mut self) {
        self.pieces.clear();
        self.thinning = 0.0;
    }

    /// Moves every piece by `offset` (a theatre scene's re-centre).
    pub fn translate(&mut self, offset: DVec3) {
        for p in &mut self.pieces {
            p.x += offset.x;
            p.z += offset.z;
        }
    }

    /// Her floats, rafts, boats and timber come up where she went under: spread over her length and beam, thrown off
    /// her by the air boiling up after her (up to 2 m/s, dying away over half a minute), each coming up within the
    /// first minute (the floats first, the timber as it works free).
    pub fn scatter(&mut self, from: &Scatter, random: &mut Lcg, source: Option<&SourceId>) {
        let (sin, cos) = from.heading.sin_cos();
        let forward = DVec3::new(sin, 0.0, -cos);
        let right = DVec3::new(cos, 0.0, sin);
        for (kind, count) in counts(from.length) {
            for _ in 0..count {
                // Every draw is taken whether or not the piece is thinned out, so density does not reshuffle the rest.
                let along = (random.next() - 0.5) * 0.9 * from.length;
                let across = (random.next() - 0.5) * 1.2 * from.beam;
                let out = random.next() * std::f64::consts::TAU;
                let speed = 0.4 + 1.6 * random.next();
                let delay = -random.next().max(1e-6).ln()
                    * match kind {
                        Kind::Carley | Kind::Raft | Kind::Boat => 6.0,
                        Kind::Plank | Kind::Timber => 15.0,
                    };
                let heading = random.next() * std::f64::consts::TAU;
                let turn = (random.next() - 0.5) * 0.02;
                let a = random.next();
                let b = random.next();
                let shade = 0.8 + 0.4 * random.next();
                let seed = random.next() as f32;
                self.thinning += self.density;
                if self.thinning < 1.0 {
                    continue;
                }
                self.thinning -= 1.0;
                let at = DVec3::new(from.x, 0.0, from.z) + forward * along + right * across;
                let (extents, freeboard, albedo, leeway) = Self::make(kind, a, b, shade);
                let piece = Piece {
                    kind,
                    x: at.x,
                    z: at.z,
                    heading,
                    turn,
                    extents,
                    freeboard,
                    albedo,
                    leeway,
                    thrown: DVec3::new(out.cos(), 0.0, out.sin()) * speed,
                    born: self.time + delay.min(90.0),
                    seed,
                    source: source.cloned(),
                };
                self.push(piece);
            }
        }
    }

    /// A plank of the timber a blast threw, landed on the water at `at` (`Debris::landed`).
    pub fn plank(&mut self, at: DVec3, length: f64, random: &mut Lcg, source: Option<&SourceId>) {
        let heading = random.next() * std::f64::consts::TAU;
        let turn = (random.next() - 0.5) * 0.02;
        let shade = 0.8 + 0.4 * random.next();
        let seed = random.next() as f32;
        let (mut extents, freeboard, albedo, leeway) =
            Self::make(Kind::Plank, random.next(), random.next(), shade);
        extents.x = length.clamp(0.4, 3.0);
        self.push(Piece {
            kind: Kind::Plank,
            x: at.x,
            z: at.z,
            heading,
            turn,
            extents,
            freeboard,
            albedo,
            leeway,
            thrown: DVec3::ZERO,
            born: self.time,
            seed,
            source: source.cloned(),
        });
    }

    /// A piece of `kind` from two draws `a` and `b` (sizes) and a shade: its half extents, freeboard (m), albedo and
    /// leeway.
    fn make(kind: Kind, a: f64, b: f64, shade: f64) -> (DVec3, f64, Rgb, f64) {
        // The leeway of what stands out of the water, beyond the surface's drift: a float or raft about one percent of
        // the wind, a boat two (a raft's total, three to five percent, is the search-and-rescue manuals' range; an
        // estimate here).
        match kind {
            Kind::Carley => {
                // Nos. 17 to 20: 5 × 10 ft to 9 × 14 ft, a 16 to 20 in tube; grey, the ship's own paint.
                let length = 1.5 + 0.65 * a;
                let width = (0.75 + 0.6 * b).min(length * 0.9);
                let tube = 0.2 + 0.05 * b;
                (
                    DVec3::new(length, tube, width),
                    tube * 0.3,
                    Rgb::hex(0x7d8287).scale(shade),
                    0.012,
                )
            }
            Kind::Raft => (
                // Cork 3 ft square, a foot deep: grey.
                DVec3::new(0.45 + 0.05 * a, 0.15, 0.45 + 0.05 * b),
                0.04,
                Rgb::hex(0x6f706d).scale(shade),
                0.01,
            ),
            Kind::Boat => {
                // A whaler of 27 ft to a cutter of 32 ft, beam 6 to 8 ft, about 2.5 ft of side: swamped, low in the
                // water; grey topsides.
                let length = 4.1 + 0.8 * a;
                (
                    DVec3::new(length, 0.4, 0.9 + 0.3 * b),
                    0.15,
                    Rgb::hex(0x8c9196).scale(shade),
                    0.02,
                )
            }
            Kind::Plank => (
                // 4 to 6 in by 3 in, 3 to 20 ft: teak and deal, wet.
                DVec3::new(0.5 + 2.5 * a * a, 0.04, 0.05 + 0.025 * b),
                0.005,
                Rgb::hex(0x6e5a44).scale(shade),
                0.0,
            ),
            Kind::Timber => (
                // A baulk, a mess table, a spar: up to 6 m and a foot square.
                DVec3::new(0.6 + 2.4 * a, 0.08 + 0.1 * b, 0.12 + 0.25 * b),
                0.02,
                Rgb::hex(0x5d5348).scale(shade),
                0.005,
            ),
        }
    }

    fn push(&mut self, piece: Piece) {
        if self.pieces.len() >= CAPACITY {
            // The oldest goes.
            let oldest = self
                .pieces
                .iter()
                .enumerate()
                .min_by(|a, b| a.1.born.total_cmp(&b.1.born))
                .map_or(0, |(i, _)| i);
            self.pieces.swap_remove(oldest);
        }
        self.pieces.push(piece);
    }

    /// Advance by `dt`: every piece drifts with the surface and its own leeway in the `wind` (m/s, world), turns
    /// slowly, and goes once its time is up.
    pub fn update(&mut self, dt: f64, wind: DVec3) {
        if dt.is_nan() || dt <= 0.0 {
            return;
        }
        self.time += dt;
        let now = self.time;
        let wind = DVec3::new(wind.x, 0.0, wind.z);
        // The boil's throw dies away over this many seconds, taken exactly over a step.
        const THROWN_SECONDS: f64 = 30.0;
        let decay = (-dt / THROWN_SECONDS).exp();
        for p in &mut self.pieces {
            if p.born > now {
                continue;
            }
            let drift =
                wind * (DRIFT_SHARE + p.leeway) * dt + p.thrown * THROWN_SECONDS * (1.0 - decay);
            p.x += drift.x;
            p.z += drift.z;
            p.thrown *= decay;
            p.heading += p.turn * dt;
        }
        self.pieces.retain(|p| now - p.born < LIFETIME);
    }

    /// The pieces on the water in `camera`'s view, as instances, appended to `out`; `swell` the sea they ride (flat
    /// without one).
    pub fn publish_into(
        &self,
        camera: Option<&CameraView>,
        swell: Option<&Swell>,
        out: &mut Vec<DebrisInstance>,
    ) {
        let planes = camera.map(|c| (c, c.frustum()));
        for p in &self.pieces {
            let age = self.time - p.born;
            if age < 0.0 {
                continue;
            }
            let at = |x: f64, z: f64| swell.map_or(0.0, |s| s.height(x, z, s.time));
            let surface = at(p.x, p.z);
            // Its last minute it settles and goes.
            let sinking = ((age - (LIFETIME - 60.0)) / 60.0).clamp(0.0, 1.0);
            // It comes up over its first two seconds.
            let rising = 1.0 - (age / 2.0).clamp(0.0, 1.0);
            let y = surface + p.freeboard - (p.extents.y * 2.0 + 0.3) * (sinking + rising);
            let position = DVec3::new(p.x, y, p.z);
            if let Some((camera, planes)) = &planes {
                let radius = p.extents.x.max(p.extents.z) * 1.5;
                if planes
                    .iter()
                    .any(|plane| plane.distance(camera.drawn(position)) < -radius)
                {
                    continue;
                }
            }
            // Tilted to the swell's slope under it.
            let step = 1.0;
            let normal = DVec3::new(
                at(p.x - step, p.z) - at(p.x + step, p.z),
                2.0 * step,
                at(p.x, p.z - step) - at(p.x, p.z + step),
            )
            .normalize_or(DVec3::Y);
            let yaw = DQuat::from_rotation_y(-p.heading);
            let tilt = DQuat::from_rotation_arc(DVec3::Y, normal);
            let turn = tilt * yaw;
            let shape = match p.kind {
                Kind::Carley => shape::CARLEY,
                Kind::Raft => shape::RAFT,
                Kind::Boat => shape::BOAT,
                Kind::Plank | Kind::Timber => shape::PLANK,
            };
            out.push(DebrisInstance {
                position: [
                    position.x as f32,
                    position.y as f32,
                    position.z as f32,
                    p.seed,
                ],
                rotation: [turn.x as f32, turn.y as f32, turn.z as f32, turn.w as f32],
                extents: [
                    p.extents.x as f32,
                    p.extents.y as f32,
                    p.extents.z as f32,
                    shape,
                ],
                color: [p.albedo.r as f32, p.albedo.g as f32, p.albedo.b as f32, 0.0],
            });
        }
    }

    /// Pieces on the water that came off `ship`, for tools.
    pub fn count_of(&self, ship: &str) -> usize {
        self.pieces
            .iter()
            .filter(|p| p.born <= self.time && p.source.as_deref() == Some(ship))
            .count()
    }

    /// Where each piece on the water is (x, z) and what it is, for tools and tests.
    pub fn pieces(&self) -> impl Iterator<Item = (Kind, f64, f64)> + '_ {
        self.pieces
            .iter()
            .filter(|p| p.born <= self.time)
            .map(|p| (p.kind, p.x, p.z))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hood() -> Scatter {
        Scatter {
            x: 100.0,
            z: -50.0,
            heading: 0.6,
            length: 262.0,
            beam: 32.0,
        }
    }

    #[test]
    fn a_lost_ship_leaves_her_floats_rafts_boats_and_timber_where_she_went() {
        let mut flotsam = Flotsam::new();
        flotsam.scatter(&hood(), &mut Lcg::new(5), None);
        let total: usize = counts(262.0).iter().map(|(_, n)| n).sum();
        assert_eq!(flotsam.len(), total);
        let of = |kind: Kind| counts(262.0).iter().find(|(k, _)| *k == kind).unwrap().1;
        // Hood's: about a dozen Carley floats, over twenty cork rafts, a few boats, a couple of hundred pieces of
        // timber.
        assert!((10..=14).contains(&of(Kind::Carley)));
        assert!(of(Kind::Raft) > 20);
        assert!((2..=4).contains(&of(Kind::Boat)));
        assert!((150..=250).contains(&(of(Kind::Plank) + of(Kind::Timber))));
        // A destroyer's under half of it, a boat at least.
        let destroyer: usize = counts(118.0).iter().map(|(_, n)| n).sum();
        assert!(destroyer * 2 < total + 1);
        assert!(counts(118.0)[2].1 >= 1);
        // A minute and a half on, everything has come up, over her length and a little beyond.
        flotsam.update(90.0, DVec3::ZERO);
        let up: Vec<_> = flotsam.pieces().collect();
        assert_eq!(up.len(), total);
        let widest = up
            .iter()
            .map(|(_, x, z)| (x - 100.0).hypot(z + 50.0))
            .fold(0.0f64, f64::max);
        assert!(
            widest > 100.0 && widest < 0.5 * 262.0 + 80.0,
            "{widest:.0} m"
        );
        // Thinned by the effects setting, evenly.
        let mut thin = Flotsam::new();
        thin.density = 0.5;
        thin.scatter(&hood(), &mut Lcg::new(5), None);
        assert!((thin.len() as f64 - total as f64 * 0.5).abs() <= 1.0);
    }

    #[test]
    fn flotsam_drifts_downwind_with_the_surface_and_rides_the_swell() {
        let mut flotsam = Flotsam::new();
        flotsam.scatter(&hood(), &mut Lcg::new(9), None);
        flotsam.update(120.0, DVec3::ZERO);
        let mean = |f: &Flotsam| {
            let (n, x, z) = f.pieces().fold((0.0, 0.0, 0.0), |(n, sx, sz), (_, x, z)| {
                (n + 1.0, sx + x, sz + z)
            });
            (x / n, z / n)
        };
        let (x0, z0) = mean(&flotsam);
        // Ten minutes in a 10 m/s wind toward +x: the planks go with the water (3.5 %, 210 m), the floats and boats
        // further.
        for _ in 0..600 {
            flotsam.update(1.0, DVec3::new(10.0, 0.0, 0.0));
        }
        let (x1, z1) = mean(&flotsam);
        assert!((x1 - x0 - 230.0).abs() < 40.0, "drifted {:.0} m", x1 - x0);
        assert!((z1 - z0).abs() < 10.0);
        let drift = |kind: Kind| {
            let list: Vec<f64> = flotsam
                .pieces()
                .filter(|(k, ..)| *k == kind)
                .map(|(_, x, _)| x)
                .collect();
            list.iter().sum::<f64>() / list.len() as f64
        };
        assert!(drift(Kind::Boat) > drift(Kind::Plank));
        // On a 1.5 m swell each piece's middle sits at its freeboard over the water under it.
        let sea = crate::sea::SeaWaves {
            amplitude_m: 1.5,
            wavelength_m: 160.0,
            direction: 0.0,
            wind_mps: 10.0,
            phase: 0.3,
        };
        let swell = Swell { sea, time: 40.0 };
        let mut out = Vec::new();
        flotsam.publish_into(None, Some(&swell), &mut out);
        assert_eq!(out.len(), flotsam.pieces().count());
        for (instance, p) in out
            .iter()
            .zip(flotsam.pieces.iter().filter(|p| p.born <= flotsam.time))
        {
            let surface = sea.height(p.x, p.z, 40.0);
            assert!((f64::from(instance.position[1]) - surface - p.freeboard).abs() < 1e-3);
        }
        // Half an hour on the sea has taken them.
        flotsam.update(LIFETIME, DVec3::ZERO);
        assert!(flotsam.is_empty());
    }
}
