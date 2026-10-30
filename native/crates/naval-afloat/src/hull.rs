//! A hull as the sea and the effects see her: her waterline's length and beam, her draft, her displacement (or a guess
//! from the three), her freeboard, full speed, metacentric height and funnels, in ship space. Nothing of her model: a
//! host fills it from its own setup (a Sketchfab model's measured waterline, say) and the rest follows.
//!
//! Ship space is the game's model frame: metres, x to starboard, y up from the design waterline, z aft (the bow toward
//! −z).
//!
//! Her natural periods follow from these ([`Hull::periods`]): heave and pitch from her waterplane against her mass and
//! the water that moves with her, roll from her metacentric height against her beam (the simulation's, `roll_decay` in
//! `crates/naval-sim/src/flotation/stability.rs`). A destroyer heaves in about 4 s and rolls in about 10; a
//! battleship in about 8 and 18.

use std::fmt;

use glam::DVec3;
use naval_ocean::hull_profile::{SimpleHull, WaterlineProfile};

/// Gravity (m/s²).
pub const GRAVITY: f64 = 9.81;
/// Sea water (t/m³).
pub const SEA_WATER: f64 = 1.025;
/// The block coefficient (displacement over length × beam × draft) a hull without a displacement is given: a
/// warship's, between a destroyer's 0.48 and a battleship's 0.6.
pub const DEFAULT_BLOCK: f64 = 0.55;
/// A given displacement is read as a block coefficient within these: a racing hull's to a barge's.
pub const BLOCK_RANGE: (f64, f64) = (0.35, 0.85);
/// The water that heaves and pitches with her, as a share of her own mass (about her displacement again at her
/// natural periods, strip theory's two-dimensional sections).
pub const ADDED_MASS: f64 = 1.0;
/// Her pitch gyradius, a share of her length.
pub const PITCH_GYRADIUS: f64 = 0.25;
/// Her roll gyradius, a share of her beam (`roll_gyradius` in the simulation's stability), the water that rolls with
/// her included.
pub const ROLL_GYRADIUS: f64 = 0.4;
/// Her metacentric height without one given, a share of her beam (`ROLL_DAMPING_GM_BEAM`: every hull of the game's
/// catalog within a tenth of it, a Fletcher's 0.85 m, Bismarck's 2.52 m).
pub const GM_BEAM: f64 = 0.07;
/// Full speed without one given: this Froude number on her length, no faster than [`TOP_SPEED`] (m/s, 31 knots):
/// a destroyer's 31 knots, a 30 m launch's 17.
pub const FULL_SPEED_FROUDE: f64 = 0.5;
pub const TOP_SPEED: f64 = 16.0;
/// Full astern, a share of full ahead.
pub const ASTERN: f64 = 0.4;

/// One funnel's mouth, in ship space.
#[derive(Clone, Debug, PartialEq)]
pub struct Funnel {
    /// The middle of its rim.
    pub position: DVec3,
    /// Across and along its mouth (m).
    pub width: f64,
    pub length: f64,
    /// Its share of her smoke, 0 none (a dummy funnel) to 1; the smoke itself follows her speed.
    pub smoke: f64,
}

impl Funnel {
    /// A round mouth `width` across at `position`, smoking its whole share.
    pub fn round(position: DVec3, width: f64) -> Self {
        Self {
            position,
            width,
            length: width,
            smoke: 1.0,
        }
    }
}

/// A hull, by the numbers. Optional fields are guessed from her dimensions where none is given. Put afloat
/// ([`crate::Afloat::new`], [`crate::Floating::new`], [`crate::Stir::new`]) only once [`Hull::check`] passes.
#[derive(Clone, Debug, PartialEq)]
pub struct Hull {
    /// Waterline length and beam, and keel below the design waterline (m).
    pub length: f64,
    pub beam: f64,
    pub draft: f64,
    /// Displacement (t); none, [`DEFAULT_BLOCK`] of her length × beam × draft in sea water.
    pub displacement: Option<f64>,
    /// Deck edge above the design waterline amidships (m); none, a warship's for her length.
    pub freeboard: Option<f64>,
    /// Full speed ahead (m/s): the wake, the bow spray and the funnels' exhaust read her speed as a share of it.
    pub full_speed: Option<f64>,
    /// Metacentric height (m): her roll's stiffness.
    pub metacentric_height: Option<f64>,
    /// The waterplane's centre in ship space (x, z): 0 for a model whose origin is amidships on the centre line.
    pub centre: [f64; 2],
    pub funnels: Vec<Funnel>,
}

/// A hull the sea cannot carry ([`Hull::check`]): which number, and what it was given as.
#[derive(Clone, Debug, PartialEq)]
pub struct BadHull {
    pub what: &'static str,
    pub value: f64,
}

impl fmt::Display for BadHull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a hull's {} cannot be {}", self.what, self.value)
    }
}

impl std::error::Error for BadHull {}

/// Natural periods (s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Periods {
    pub heave: f64,
    pub pitch: f64,
    pub roll: f64,
}

impl Hull {
    /// A hull of these waterline dimensions, every other number guessed, no funnels (checked when she is put afloat).
    pub fn new(length: f64, beam: f64, draft: f64) -> Self {
        Self {
            length,
            beam,
            draft,
            displacement: None,
            freeboard: None,
            full_speed: None,
            metacentric_height: None,
            centre: [0.0, 0.0],
            funnels: Vec::new(),
        }
    }

    /// Whether the sea can carry her: her length, beam and draft, and each number given (displacement, freeboard, full
    /// speed, metacentric height, a funnel's width and length), positive and finite; her centre and her funnels' rims
    /// finite, and each funnel's share of her smoke within 0 … 1. Without it a zero length's pitch period is infinite and the ocean's wake divides by it, a non-finite
    /// number reaches every effect.
    pub fn check(&self) -> Result<(), BadHull> {
        let finite = |what, value: f64| match value.is_finite() {
            true => Ok(()),
            false => Err(BadHull { what, value }),
        };
        let positive = |what, value: f64| match value > 0.0 {
            true => finite(what, value),
            false => Err(BadHull { what, value }),
        };
        positive("length", self.length)?;
        positive("beam", self.beam)?;
        positive("draft", self.draft)?;
        for (what, given) in [
            ("displacement", self.displacement),
            ("freeboard", self.freeboard),
            ("full speed", self.full_speed),
            ("metacentric height", self.metacentric_height),
        ] {
            given.map_or(Ok(()), |value| positive(what, value))?;
        }
        self.centre.iter().try_for_each(|&c| finite("centre", c))?;
        for funnel in &self.funnels {
            funnel
                .position
                .to_array()
                .iter()
                .try_for_each(|&p| finite("funnel's rim", p))?;
            positive("funnel's width", funnel.width)?;
            positive("funnel's length", funnel.length)?;
            if !(0.0..=1.0).contains(&funnel.smoke) {
                return Err(BadHull {
                    what: "funnel's smoke",
                    value: funnel.smoke,
                });
            }
        }
        Ok(())
    }

    /// Her block coefficient: from her displacement, held within [`BLOCK_RANGE`], else [`DEFAULT_BLOCK`].
    pub fn block(&self) -> f64 {
        let boxed = SEA_WATER * self.length * self.beam * self.draft;
        match self.displacement {
            Some(tonnes) if boxed > 0.0 => (tonnes / boxed).clamp(BLOCK_RANGE.0, BLOCK_RANGE.1),
            _ => DEFAULT_BLOCK,
        }
    }

    /// Her displacement (t), given or guessed.
    pub fn displacement(&self) -> f64 {
        self.displacement
            .unwrap_or(DEFAULT_BLOCK * SEA_WATER * self.length * self.beam * self.draft)
    }

    /// Her waterplane's area coefficient (area over length × beam), from her block's fullness: (1 + 2 Cb) / 3.
    pub fn waterplane_coefficient(&self) -> f64 {
        (1.0 + 2.0 * self.block()) / 3.0
    }

    /// Her waterplane's area (m²).
    pub fn waterplane_area(&self) -> f64 {
        self.waterplane_coefficient() * self.length * self.beam
    }

    /// Deck edge above the design waterline amidships (m), given or a warship's (`SimpleHull::warship`).
    pub fn freeboard(&self) -> f64 {
        self.freeboard.unwrap_or(1.5 + 0.03 * self.length)
    }

    /// Full speed ahead (m/s), given or guessed from her length.
    pub fn full_speed(&self) -> f64 {
        self.full_speed.unwrap_or_else(|| {
            (FULL_SPEED_FROUDE * (GRAVITY * self.length.max(1.0)).sqrt()).min(TOP_SPEED)
        })
    }

    /// Full speed astern (m/s).
    pub fn reverse_speed(&self) -> f64 {
        ASTERN * self.full_speed()
    }

    /// Metacentric height (m), given or [`GM_BEAM`] of her beam.
    pub fn metacentric_height(&self) -> f64 {
        self.metacentric_height.unwrap_or(GM_BEAM * self.beam)
    }

    /// Her Froude number at `speed` (m/s).
    pub fn froude(&self, speed: f64) -> f64 {
        speed.abs() / (GRAVITY * self.length.max(1.0)).sqrt()
    }

    /// Her undamped natural angular frequencies (rad/s): heave, pitch and roll.
    pub fn frequencies(&self) -> (f64, f64, f64) {
        let block = self.block();
        let area = self.waterplane_coefficient();
        let draft = self.draft.max(0.1);
        // Heave: the waterplane's stiffness ρ g A_w against her mass and the water moving with her.
        let heave = (GRAVITY * area / (block * draft * (1.0 + ADDED_MASS))).sqrt();
        // Pitch: her longitudinal metacentric radius (the waterplane's moment of inertia about its middle over her
        // volume; 3 Cw² / ((1 + Cw)(1 + 2 Cw)) of the rectangle's, the usual estimate) against her gyradius.
        let moment = 3.0 * area * area / ((1.0 + area) * (1.0 + 2.0 * area));
        let radius = moment * self.length * self.length / (12.0 * block * draft);
        let gyradius = PITCH_GYRADIUS * self.length.max(1.0);
        let pitch = (GRAVITY * radius / (gyradius * gyradius * (1.0 + ADDED_MASS))).sqrt();
        // Roll: as the simulation's.
        let roll = (GRAVITY * self.metacentric_height().max(0.01)).sqrt()
            / (ROLL_GYRADIUS * self.beam.max(0.1));
        (heave, pitch, roll)
    }

    /// Her natural periods (s).
    pub fn periods(&self) -> Periods {
        let (heave, pitch, roll) = self.frequencies();
        let tau = std::f64::consts::TAU;
        Periods {
            heave: tau / heave,
            pitch: tau / pitch,
            roll: tau / roll,
        }
    }

    /// The proxy hull of her dimensions (naval-ocean's `SimpleHull`, a warship's lines), in its own frame about her
    /// waterplane's centre (the bow at −length / 2): the shape her waterplane is sampled over, and the stand-in a host
    /// draws before it has her model, at her pose moved by [`Hull::centre`].
    pub fn shape(&self) -> SimpleHull {
        SimpleHull {
            freeboard: self.freeboard() as f32,
            ..SimpleHull::warship(self.length as f32, self.beam as f32, self.draft as f32)
        }
    }

    /// Her waterline profile, as the wakes' contact foam, the bow waves and the bow spray read a hull's sides: her
    /// proxy's, sliced, in ship space (moved by [`Hull::centre`], as a model's slice is in the model's frame), so her
    /// stem is where the wake's generators and the sea about her put it.
    pub fn profile(&self) -> WaterlineProfile {
        let mut profile = self.shape().profile();
        let [x, z] = self.centre.map(|c| c as f32);
        profile.start += z;
        profile.bow += z;
        profile.stern += z;
        for stem in &mut profile.stems {
            *stem += z;
        }
        // The half-breadths are outboard of the model's centre line.
        for breadth in &mut profile.starboard {
            *breadth += x;
        }
        for breadth in &mut profile.port {
            *breadth -= x;
        }
        profile.half_beam += x.abs();
        profile
    }

    /// The highest of her above her waterline (m): her bow's deck edge or a funnel's rim.
    pub fn top(&self) -> f64 {
        let shape = self.shape();
        let bow = f64::from(shape.freeboard + shape.sheer);
        self.funnels
            .iter()
            .map(|f| f.position.y)
            .fold(bow, f64::max)
    }
}
