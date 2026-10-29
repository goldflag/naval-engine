//! The effect side of lasting hull damage: what scorch (the web game's `ShipScorch.ts`) and impact marks
//! (the web game's `ShipImpactMarks.ts`) read from the simulation's events, in a form the ship renderer can draw from.
//!
//! Drawing them belongs with the ship meshes (`naval-ships`; the fleet lane's step 5, "damage visuals"): impact marks
//! are decals projected onto the struck mesh from a small front-facing subset of its triangles, and scorch is a
//! per-hull list of spots the ship shader darkens. What both need from the effects is here: every strike on a hull
//! in the struck body's own frame (the hull, or a mount's gunhouse), with its mark's sizing ([`impact_style`]) and the
//! atlas tile it takes, and each fire's place and intensity, which [`crate::ShipFrame`] already carries. See the lane
//! doc ("Scorch and impact marks") for the interface the ship side is expected to offer.

use glam::DVec3;

use crate::events::CombatEvent;
use crate::math::{clamp, v3};

/// How a round met the plating.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrikeOutcome {
    Penetration,
    Stopped,
    Ricochet,
}

impl StrikeOutcome {
    /// A surface impact's outcome word (`penetration`, `ricochet`, `stopped`).
    pub fn of(word: &str) -> Self {
        match word {
            "penetration" => Self::Penetration,
            "ricochet" => Self::Ricochet,
            _ => Self::Stopped,
        }
    }
}

/// What burst against the plating, as far as its mark shows it: a shell's filling, or a torpedo's warhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filling {
    ArmourPiercing,
    HighExplosive,
    /// A torpedo's warhead, burst against her side under water.
    Torpedo,
}

/// The mark a strike leaves: its kind (the atlas tile naval-ships draws) and its width and height (m).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImpactStyle {
    /// 0 an armour-piercing hole, 1 a round stopped by armour, 2 a ricochet's gouge, 3 a high-explosive burst that
    /// holed the plate, 4 one that did not, 5 a torpedo's burst against her side. naval-ships draws a burst or a hole
    /// on a deck or a roof as its own radial mark.
    pub tile: u32,
    pub width: f64,
    pub height: f64,
}

/// A high-explosive burst's mark across, per cube root of its charge (m/∛kg): the tear (`naval_sim`'s `tear_m2`,
/// about 0.56·∛charge across) a sixth of it, the blackened paint about half, fragment pocking out to its edge. 5 m
/// for a five-inch shell, 6 m for a 15 cm, 12.8 m for a 38 cm HE shell, 15.8 m for a 500 lb bomb. A burst the
/// armour stopped scours a crater a little smaller.
const BURST_WIDTH_PER_CUBE_ROOT_KG: f64 = 3.2;
const CRATER_WIDTH_PER_CUBE_ROOT_KG: f64 = 3.0;
/// A torpedo's mark across per cube root of its warhead: the hole torn in her side about two fifths of it (5.9 m for
/// a 533 mm torpedo's 300 kg), the plating dished and stripped round it, an oily black wash over the rest.
const TORPEDO_WIDTH_PER_CUBE_ROOT_KG: f64 = 2.2;
/// The warhead the event's torpedo carries, from its diameter: 300 kg at 533 mm (a Mk 15's 375 kg, a G7a's 280 kg),
/// growing with the cube of the diameter (450 kg at 610 mm; a Type 93 carried 490 kg).
const TORPEDO_CHARGE_KG: f64 = 300.0;
const TORPEDO_CHARGE_DIAMETER_M: f64 = 0.533;

/// The bursting charge (kg) of a shell whose event gives none: a common shell's, about 1000·calibre³ (2 kg at five
/// inches, 3.4 at 15 cm, 55 at 38 cm, as the catalogue's HE shells carry).
pub fn estimated_charge_kg(caliber_m: f64) -> f64 {
    1000.0 * caliber_m.powi(3)
}

/// The warhead (kg) of a torpedo of `diameter_m`: [`TORPEDO_CHARGE_KG`] at 533 mm, growing with the cube of the
/// diameter.
pub fn torpedo_charge_kg(diameter_m: f64) -> f64 {
    TORPEDO_CHARGE_KG * (diameter_m / TORPEDO_CHARGE_DIAMETER_M).powi(3)
}

/// A strike's mark, sized from what burst there: an armour-piercing hole about its calibre with the burnt paint round
/// it and its smoke stain climbing above (taller than wide, as naval-ships' `MARK_ASPECT` lays the tiles out), a dish
/// of scoured steel where armour stopped it, a gouge along a glance; a high-explosive burst's tear or crater, scorch
/// and fragment pocking from the cube root of its charge (`charge_kg`, else [`estimated_charge_kg`]), as a blast's
/// reach scales; a torpedo's torn and dished side from its warhead's.
pub fn impact_style(
    caliber_m: f64,
    charge_kg: Option<f64>,
    filling: Filling,
    outcome: StrikeOutcome,
) -> ImpactStyle {
    let caliber = clamp(caliber_m, 0.02, 2.0);
    let (tile, width, aspect) = match (filling, outcome) {
        (Filling::Torpedo, _) => {
            let charge = torpedo_charge_kg(caliber);
            (5, TORPEDO_WIDTH_PER_CUBE_ROOT_KG * charge.cbrt(), 1.0)
        }
        (_, StrikeOutcome::Ricochet) => (2, caliber * 9.0, 1.0 / 3.0),
        (Filling::HighExplosive, _) => {
            let charge = charge_kg
                .filter(|c| c.is_finite() && *c > 0.0)
                .unwrap_or_else(|| estimated_charge_kg(caliber));
            let root = clamp(charge, 1e-3, 1e4).cbrt();
            if outcome == StrikeOutcome::Penetration {
                (3, BURST_WIDTH_PER_CUBE_ROOT_KG * root, 1.2)
            } else {
                (4, CRATER_WIDTH_PER_CUBE_ROOT_KG * root, 1.15)
            }
        }
        (_, StrikeOutcome::Penetration) => (0, caliber * 4.5, 1.6),
        (_, StrikeOutcome::Stopped) => (1, caliber * 4.0, 1.25),
    };
    ImpactStyle {
        tile,
        width,
        height: width * aspect,
    }
}

/// One strike on a hull, as `ShipImpactMarks` and `ShipScorch` read it from an event's `surfaceImpact`.
#[derive(Clone, Debug, PartialEq)]
pub struct HullStrike {
    pub ship_id: String,
    /// The struck mount when the round hit a gunhouse: the point and vectors are then in that mount's frame.
    pub mount_id: Option<String>,
    /// Where it struck, the plate's outward normal and the round's direction, in the struck body's frame
    /// (hull frame: metres, +X starboard, +Y up from the waterline, −Z toward the bow).
    pub position: DVec3,
    pub normal: DVec3,
    pub direction: DVec3,
    pub outcome: StrikeOutcome,
    pub filling: Filling,
    /// The shell's calibre, or the torpedo's diameter.
    pub caliber_m: f64,
    /// The shell's id: repeated strikes of one round are one mark; its id also turns a mark's decal. A torpedo's is
    /// its own id negated, less one, so it never stands for a shell's.
    pub shell_id: i64,
    pub style: ImpactStyle,
}

impl HullStrike {
    /// The strike an event records, if it records one on a hull: a shell's (`surfaceImpact` and `shell` both present)
    /// or a torpedo's burst (a `torpedo-hit` with `surfaceImpact` and `torpedo`).
    pub fn from_event(event: &CombatEvent) -> Option<Self> {
        let impact = event.surface_impact.as_ref()?;
        let outcome = StrikeOutcome::of(&impact.outcome);
        let (filling, caliber_m, shell_id, charge_kg) = match (&event.shell, &event.torpedo) {
            (Some(shell), _) => (
                if shell.shell_type.as_deref() == Some("HE") {
                    Filling::HighExplosive
                } else {
                    Filling::ArmourPiercing
                },
                shell.caliber_m,
                shell.id,
                shell.explosive_kg,
            ),
            (None, Some(torpedo)) if event.kind == "torpedo-hit" => {
                (Filling::Torpedo, torpedo.diameter_m, -1 - torpedo.id, None)
            }
            _ => return None,
        };
        Self::new(
            &event.ship_id,
            impact.mount_id.as_deref(),
            [impact.position, impact.normal, impact.direction],
            outcome,
            (filling, caliber_m, shell_id, charge_kg),
        )
    }

    /// A strike at `[position, normal, direction]` of a round that is `(filling, calibre, id, charge)`: none where
    /// the geometry is not finite or the normal not a unit's. The host makes one so from a scar a hull keeps
    /// (`naval_effects_sim::scar_strike`), for a hull marked again after a restore.
    pub fn new(
        ship_id: &str,
        mount_id: Option<&str>,
        [position, normal, direction]: [[f64; 3]; 3],
        outcome: StrikeOutcome,
        (filling, caliber_m, shell_id, charge_kg): (Filling, f64, i64, Option<f64>),
    ) -> Option<Self> {
        let (position, normal, direction) = (v3(position), v3(normal), v3(direction));
        let finite = position.is_finite()
            && normal.is_finite()
            && direction.is_finite()
            && caliber_m.is_finite();
        if !finite || normal.length_squared() < 0.5 {
            return None;
        }
        Some(Self {
            ship_id: ship_id.to_owned(),
            mount_id: mount_id.map(str::to_owned),
            position,
            normal,
            direction,
            outcome,
            filling,
            caliber_m,
            shell_id,
            style: impact_style(caliber_m, charge_kg, filling, outcome),
        })
    }

    /// The decal's in-plane turn: ricochet gouges follow the round's path; other marks turn by the shell's id
    /// (`(id × 2.399963) mod 2π`), so neighbouring marks never repeat.
    pub fn turn(&self) -> f64 {
        match self.outcome {
            StrikeOutcome::Ricochet => 0.0,
            _ => (self.shell_id as f64 * 2.399_963).rem_euclid(std::f64::consts::TAU),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ShellInfo, SurfaceImpactInfo, TorpedoInfo};

    #[test]
    fn styles_size_marks_from_the_round() {
        let ap = impact_style(
            0.38,
            Some(20.0),
            Filling::ArmourPiercing,
            StrikeOutcome::Penetration,
        );
        assert_eq!(ap.tile, 0);
        assert!((ap.width - 1.71).abs() < 1e-9 && ap.height > ap.width);
        let gouge = impact_style(0.38, None, Filling::ArmourPiercing, StrikeOutcome::Ricochet);
        assert_eq!(gouge.tile, 2);
        assert!(gouge.width > 3.0 * gouge.height - 1e-9);
        let torn = impact_style(
            0.15,
            Some(3.624),
            Filling::HighExplosive,
            StrikeOutcome::Penetration,
        );
        assert_eq!(torn.tile, 3);
        assert!((torn.width - 4.9).abs() < 0.05, "{}", torn.width);
        let burst = impact_style(
            0.15,
            Some(3.624),
            Filling::HighExplosive,
            StrikeOutcome::Stopped,
        );
        assert_eq!(burst.tile, 4);
        assert!(burst.width < torn.width);
        // A bomb's charge, not its calibre, sizes its burst: a 500 lb bomb's 120 kg against a 15 cm shell's 3.6.
        let bomb = impact_style(
            0.35,
            Some(120.0),
            Filling::HighExplosive,
            StrikeOutcome::Penetration,
        );
        assert!((bomb.width - 15.8).abs() < 0.1, "{}", bomb.width);
        // With no charge given, a common shell's.
        let guessed = impact_style(
            0.15,
            None,
            Filling::HighExplosive,
            StrikeOutcome::Penetration,
        );
        assert!((guessed.width / torn.width - 1.0).abs() < 0.03);
        let torpedo = impact_style(0.533, None, Filling::Torpedo, StrikeOutcome::Penetration);
        assert_eq!(torpedo.tile, 5);
        assert!((torpedo.width - 14.7).abs() < 0.1, "{}", torpedo.width);
    }

    #[test]
    fn an_event_with_a_surface_impact_is_a_strike() {
        let event = CombatEvent {
            kind: "penetration".into(),
            ship_id: "target".into(),
            shell: Some(ShellInfo {
                id: 7,
                caliber_m: 0.203,
                velocity: [0.0, -100.0, 600.0],
                shell_type: Some("AP".into()),
                explosive_kg: Some(2.3),
            }),
            surface_impact: Some(SurfaceImpactInfo {
                position: [5.0, 3.0, -40.0],
                normal: [1.0, 0.0, 0.0],
                direction: [-0.9, -0.2, 0.3],
                mount_id: None,
                outcome: "penetration".into(),
            }),
            ..Default::default()
        };
        let strike = HullStrike::from_event(&event).expect("a strike");
        assert_eq!(strike.outcome, StrikeOutcome::Penetration);
        assert_eq!(strike.style.tile, 0);
        assert!(HullStrike::from_event(&CombatEvent::default()).is_none());
    }

    #[test]
    fn a_torpedo_hit_on_her_side_is_a_strike() {
        let side = SurfaceImpactInfo {
            position: [9.0, -2.0, 10.0],
            normal: [1.0, 0.0, 0.0],
            direction: [-1.0, 0.0, 0.0],
            mount_id: None,
            outcome: "penetration".into(),
        };
        let torpedo = TorpedoInfo {
            id: 7,
            diameter_m: 0.61,
        };
        let event = CombatEvent {
            kind: "torpedo-hit".into(),
            ship_id: "target".into(),
            surface_impact: Some(side.clone()),
            torpedo: Some(torpedo.clone()),
            ..Default::default()
        };
        let strike = HullStrike::from_event(&event).expect("a strike");
        assert_eq!(strike.filling, Filling::Torpedo);
        assert_eq!(strike.style.tile, 5);
        // Never the same id as shell 7's.
        assert_eq!(strike.shell_id, -8);
        // A dud leaves nothing.
        let dud = CombatEvent {
            kind: "torpedo-dud".into(),
            ..event
        };
        assert!(HullStrike::from_event(&dud).is_none());
    }
}
