//! A hull on the drawn sea, visual only (see `native/docs/afloat.md`): no model, no weapons, no insides, nothing of the
//! game's simulation. A host describes her by the numbers ([`Hull`]: length, beam, draft, a displacement or a guess
//! at one, funnels in ship space), steers her (her x, z, heading and speed each frame, [`Course`]), and gets back:
//!
//! - her pose on the sea ([`motion`]): heave, pitch and roll from the sea's level under her waterplane, with her
//!   hull's natural periods and damping, the same at any frame rate;
//! - what she stirs ([`stir`]): the inputs of the ocean's wake, foam and bow waves, the water effects' bow spray and
//!   the combat effects' funnel smoke, as the game's client builds them for its own ships.
//!
//! The sea she rides is the one the ocean draws about her ([`sea`]): the combat sea's two sines, which the host hands
//! the ocean with her footprint (`Ocean::set_hull_sea`) so the surface gives its long waves way to them, and the
//! field's shorter waves on them, read back from the ocean's heights about her a frame or two late ([`drawn`]).
//!
//! Portable: passes `cargo check -p naval-afloat --target wasm32-unknown-unknown`.

pub mod drawn;
pub mod hull;
pub mod motion;
pub mod sea;
pub mod stir;

pub use drawn::{DrawnSea, DrawnSurface};
pub use hull::{BadHull, Funnel, Hull, Periods};
pub use motion::{Course, Floating, Motion, SeaUnder, Waterplane};
pub use sea::{Calm, SeaSurface, ocean_waves, swell};
pub use stir::Stir;

use glam::DVec3;
use naval_effects::ShipPose;

/// One hull afloat: her floating and what she stirs, behind one call a frame.
#[derive(Clone, Debug)]
pub struct Afloat {
    floating: Floating,
    stir: Stir,
}

impl Afloat {
    /// `hull`, known to the effects as `id`; refused unless [`Hull::check`] passes.
    pub fn new(id: &str, hull: Hull) -> Result<Self, BadHull> {
        Ok(Self {
            stir: Stir::new(id, &hull)?,
            floating: Floating::new(hull)?,
        })
    }

    /// Step her to `time` (s, the clock the sea is drawn at, the one handed to `Ocean::set_hull_sea`) on `course`,
    /// riding `sea`, and refresh what she stirs.
    pub fn update(&mut self, course: Course, sea: &impl SeaSurface, time: f64) {
        self.floating.update(course, sea, time);
        let pose = self.floating.pose();
        self.stir
            .update(&pose, &self.floating.course(), self.floating.velocity());
    }

    pub fn hull(&self) -> &Hull {
        self.floating.hull()
    }

    pub fn floating(&self) -> &Floating {
        &self.floating
    }

    pub fn stir(&self) -> &Stir {
        &self.stir
    }

    /// Her pose this frame: her origin (her design waterline, amidships) in the world, heading, pitch and roll.
    pub fn pose(&self) -> ShipPose {
        self.floating.pose()
    }

    /// Her world velocity this frame (m/s).
    pub fn velocity(&self) -> DVec3 {
        self.floating.velocity()
    }

    /// Her heave, pitch and roll this frame, and their rates.
    pub fn motion(&self) -> Motion {
        self.floating.motion()
    }
}
