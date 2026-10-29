//! The one-shot simulation events effects are spawned from.
//!
//! [`CombatEvent`] carries the fields of the web client's `CombatEvent` (`src/game/session/elements.ts`) that the
//! effect systems read, under the same JSON names, so an event the simulation serialised for the browser (or the
//! web effects stage staged) deserialises straight into it. The game builds one from the render packet's
//! `RenderEvent`, the simulation's own `DamageEvent` with its geometry in the packet's scene frame
//! (`naval_effects_sim::combat_event`); a hull's strike stays in her frame or her mount's ([`SurfaceImpactInfo`]).

use serde::{Deserialize, Serialize};

/// A shell's part in an event: the round that was fired, splashed, struck or burst.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellInfo {
    pub id: i64,
    pub caliber_m: f64,
    pub velocity: [f64; 3],
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub shell_type: Option<String>,
    /// Its bursting charge (kg): a burst's mark on the plating is sized from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explosive_kg: Option<f64>,
}

/// A heavy AA round set to burst `flight_time` seconds after the event.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AirburstInfo {
    pub flight_time: f64,
    pub caliber_m: f64,
}

/// An aircraft's part in an event: a crash's velocity, an AA burst's aim point.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AircraftInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub airburst: Option<AirburstInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caliber_m: Option<f64>,
}

/// A torpedo's part in an event: the one that struck (for its mark on the hull).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TorpedoInfo {
    pub id: i64,
    pub diameter_m: f64,
}

/// Where a round or a torpedo met a hull, in the hull's frame (for scorch and impact marks).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceImpactInfo {
    pub position: [f64; 3],
    pub normal: [f64; 3],
    pub direction: [f64; 3],
    /// The struck mount, when the round hit a gunhouse: the vectors are then in its frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<String>,
    pub outcome: String,
}

/// One simulation event as the effects read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CombatEvent {
    /// The view's event sequence: strictly increasing.
    pub sequence: u64,
    pub tick: u64,
    /// The simulation's kebab-case kind: `shot`, `splash`, `penetration`, `burst`, `module`, `aircraft-fire`, …
    pub kind: String,
    #[serde(default)]
    pub ship_id: String,
    pub position: [f64; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ShellInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detonation: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blast_radius_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub water_burst_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull_damage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aircraft: Option<AircraftInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_impact: Option<SurfaceImpactInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torpedo: Option<TorpedoInfo>,
}

impl CombatEvent {
    /// A shell that struck the land (the simulation's `stopped` of no ship's, "Shell struck the coast").
    pub fn struck_land(&self) -> bool {
        self.kind == "stopped" && self.ship_id.is_empty()
    }

    /// The event with what the effects do not read dropped (the hull-frame impact, the shell's type and charge, the
    /// torpedo) and positions rounded to millimetres, for small recorded fixtures.
    pub fn compact(mut self) -> Self {
        let round = |v: [f64; 3], places: i32| {
            let scale = 10f64.powi(places);
            v.map(|x| (x * scale).round() / scale)
        };
        self.position = round(self.position, 3);
        self.normal = self.normal.map(|n| round(n, 5));
        if let Some(shell) = &mut self.shell {
            shell.velocity = round(shell.velocity, 2);
            shell.shell_type = None;
            shell.explosive_kg = None;
        }
        self.surface_impact = None;
        self.torpedo = None;
        self
    }

    /// The shell's calibre, or 0.38 m when the event names none (the web game's default, kept; not a product decision).
    pub fn caliber_m(&self) -> f64 {
        self.shell.as_ref().map_or(0.38, |shell| shell.caliber_m)
    }

    /// Whether any effect system in this crate draws something for this event (water effects are another
    /// lane's: splashes, near-miss contacts, crashes, torpedo and depth-charge columns).
    pub fn spawns_gas_or_fire(&self) -> bool {
        match self.kind.as_str() {
            "shot" => true,
            "aircraft-crash" => self.detonation == Some(true),
            "aircraft-fire" => self
                .aircraft
                .as_ref()
                .is_some_and(|a| a.airburst.is_some() && a.target.is_some()),
            "burst" if self.detonation == Some(true) => self.water_burst_y.is_none(),
            "contact" if self.hull_damage.is_some() => false,
            "splash"
            | "depth-charge-blast"
            | "depth-charge-splash"
            | "torpedo-hit"
            | "torpedo-launch"
            | "torpedo-dud" => false,
            _ => self.detonation == Some(true) || self.normal.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialises_the_web_event_json() {
        let json = r#"{"kind":"shot","shipId":"player","position":[1,2,3],"sequence":7,"tick":40,"message":"",
            "shell":{"id":1000001,"caliberM":0.38,"type":"AP","velocity":[0,10,-820]}}"#;
        let event: CombatEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.kind, "shot");
        assert_eq!(event.shell.as_ref().unwrap().caliber_m, 0.38);
        assert!(event.spawns_gas_or_fire());
    }
}
