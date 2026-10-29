//! The simulation's numbers the effects draw by: the host fills them from its own (the game's
//! `naval_effects_sim::rules`), so the effects build without the simulation.

/// What the effects take from the simulation's rules ([`crate::CombatEffects::rules`]). The defaults are the
/// simulation's own (`naval-effects-sim`'s tests hold them to it), for the tests and tools that run the effects without
/// a host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectRules {
    /// A smoke screen's extinction per metre inside a puff at full density
    /// (`naval_sim::battle_effects::SMOKE_EXTINCTION_PER_M`): a searchlight's beam dims through it (`air_light.rs`).
    pub smoke_extinction_per_m: f64,
    /// How much a fire's heat grows each second it burns (`naval_sim::damage_control::FIRE_GROWTH_PER_SECOND`).
    pub fire_growth_per_second: f64,
    /// How much heat a damage-control party at its full rate takes off a fire each second
    /// (`naval_sim::damage_control::SUPPRESSION_PER_SECOND`).
    pub suppression_per_second: f64,
}

impl Default for EffectRules {
    fn default() -> Self {
        Self {
            smoke_extinction_per_m: 0.025,
            fire_growth_per_second: 0.0012,
            suppression_per_second: 0.004,
        }
    }
}

impl EffectRules {
    /// How fast a fought fire's heat falls (per second) when its party plainly has it: a party at its full rate against
    /// a fire burning at its full growth (`fires.rs`: how the party is doing).
    pub fn hoses_gaining(&self) -> f64 {
        self.suppression_per_second - self.fire_growth_per_second
    }
}
