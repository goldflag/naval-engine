//! What the effects need to know about a ship: where its fires vent and its funnels exhaust (from the ship
//! definition, once), and its pose, speed and fires this frame (from the render packet).
//!
//! The game reads a [`ShipEffectsProfile`] from the simulation's `ShipDefinition` (`naval_effects_sim::ship_profile`)
//! as the web client read its TypeScript twin: compartment fire vents (`LocalizedFireEffects.collect`), mount frames
//! (`mountFrames.ts`) and funnel outlets (`funnelOutlets` in `ShipFunnelSmoke.ts`).

use std::sync::Arc;

use glam::DVec3;

use crate::math::ShipPose;
use crate::particles::SourceId;

/// A compartment that can burn where the crew and the smoke can see it.
#[derive(Clone, Debug, PartialEq)]
pub struct FireVent {
    /// Ship-local vent position (m): the definition's, lifted out of any superstructure standing over it to the roof
    /// it comes out on.
    pub position: DVec3,
    /// The compartment's box size (m): sets how widely its flames spread.
    pub size: [f64; 3],
    pub capacity_m3: f64,
}

/// Where an upperworks zone's fire shows (`naval_sim::fire_zones`): the roof of its broadest structure, its flames
/// spread over that roof.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneVent {
    /// The middle of the roof, ship-local (m).
    pub position: DVec3,
    /// Half its width and length (m), a little in from its edges.
    pub half: [f64; 2],
    /// How large the fire is drawn beside a room's (1 for a room about 11 m square).
    pub scale: f64,
}

/// A place smoke may seep out of a burning upperworks zone (`fires.rs`): a door, a scuttle or a window in one of its
/// structures' walls.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SeepPlace {
    /// Ship-local (m), on the wall.
    pub position: DVec3,
    /// The wall's outward normal, ship-local.
    pub outward: DVec3,
    /// Its draw (0–1): the places of the least draw leak first, the more of them the harder the zone burns.
    pub draw: f64,
}

/// A gun mount's frame and gunhouse, for fires burning on it.
#[derive(Clone, Debug, PartialEq)]
pub struct MountProfile {
    pub position: DVec3,
    pub bearing_deg: f64,
    /// A mount carried by another (earlier) mount.
    pub parent: Option<usize>,
    /// Gunhouse width, length, height (m).
    pub gunhouse: [f64; 3],
}

/// A funnel mouth, ship-local: the visual emission datum `funnelOutlets` reads.
#[derive(Clone, Debug, PartialEq)]
pub struct FunnelOutlet {
    pub id: String,
    pub position: DVec3,
    pub width: f64,
    pub length: f64,
    /// Turn of a raked or rotated casing from the bow (construction ships only).
    pub bearing_rad: Option<f64>,
    /// Its share of the smoke her load and soot make, 0 none (a dummy funnel's) to 1 (every funnel of the game's).
    pub smoke: f64,
}

/// The fixed, per-definition part of what the effects draw on a ship.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShipEffectsProfile {
    pub preset_id: String,
    pub hull_length: f64,
    pub hull_beam: f64,
    /// Full ahead and full astern (m/s).
    pub forward_speed: f64,
    pub reverse_speed: f64,
    /// One entry per compartment, in definition order; `None` where it has no vent.
    pub vents: Vec<Option<FireVent>>,
    /// One entry per upperworks zone, bow first; `None` where the zone does not burn.
    pub zones: Vec<Option<ZoneVent>>,
    /// Each upperworks zone's structures, as `zones` (`naval_sim::fire_zones::FireZone::boxes`): ship-local boxes
    /// (`[low, high]`, m), none where the zone does not burn. Its fire lights her windows and holes in them from within
    /// (the host's `naval_ships::FireInside`).
    pub zone_boxes: Vec<Vec<[DVec3; 2]>>,
    /// Each upperworks zone's secondary and anti-aircraft mounts, as `zones` (`FireZone::mounts`): their ready-use rounds
    /// cook off while it burns hard (`cookoffs.rs`).
    pub zone_mounts: Vec<Vec<usize>>,
    /// Each upperworks zone's places smoke may seep out of, as `zones`, the least draw first (`fires::seep_places`).
    pub zone_seeps: Vec<Vec<SeepPlace>>,
    /// Each compartment's box, ship-local (`[low, high]`, m), in definition order.
    pub room_boxes: Vec<[DVec3; 2]>,
    pub mounts: Vec<MountProfile>,
    pub funnels: Vec<FunnelOutlet>,
    /// The hull's volume, for where a strike or a burst stands: in the hull, on its plating or outside it.
    pub hull: HullVolume,
    /// Her draft (m under her water line), her moulded depth (keel to deck, m) and the top of her highest structure
    /// over her water line (m): the box she has gone under once all of it has (`loss::all_under`).
    pub hull_draft: f64,
    pub hull_depth: f64,
    pub top_m: f64,
    /// Her waterplane's area (m²): the air she drives out as she settles (`loss`).
    pub waterplane_m2: f64,
    /// Each funnel's base over her water line (m), as `funnels`: where the sea reaches her uptakes.
    pub funnel_bases: Vec<f64>,
    /// Her boiler rooms: the compartments her boilers stand in, in definition order.
    pub boiler_rooms: Vec<usize>,
    /// Each compartment's middle along her (m, her frame's z, definition order).
    pub room_z: Vec<f64>,
    /// The oil her bunkers hold full (m³; 0 for a definition without bunker data).
    pub bunker_m3: f64,
    /// Its places lie on the sea itself and ride it (burning oil, `sea_fires.rs`): none of them goes under.
    pub on_the_water: bool,
}

/// A hull's inside, as the host answers it (the game's: the simulation's `hull_contains` on her definition's hull,
/// `naval_effects_sim::SimHull`).
pub trait HullShape: std::fmt::Debug + Send + Sync {
    /// Whether a point in the ship's frame (m, +Y up from the waterline, −Z to the bow) is inside the hull, below its
    /// weather deck.
    fn contains(&self, local: DVec3) -> bool;
}

/// A ship's hull ([`HullShape`]), shared; equal only to itself.
#[derive(Clone, Debug, Default)]
pub struct HullVolume(Option<Arc<dyn HullShape>>);

impl PartialEq for HullVolume {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }
}

impl HullVolume {
    pub fn new(hull: impl HullShape + 'static) -> Self {
        Self(Some(Arc::new(hull)))
    }

    /// Whether a point in the ship's frame (m, +Y up from the waterline, −Z to the bow) is inside the hull, below
    /// its weather deck. Without a hull, nothing is.
    pub fn contains(&self, local: DVec3) -> bool {
        self.0.as_deref().is_some_and(|hull| hull.contains(local))
    }

    /// How far above `local` the weather deck stands (m), when `local` is inside the hull and the deck is within
    /// `reach`.
    pub fn deck_above(&self, local: DVec3, reach: f64) -> Option<f64> {
        if !self.contains(local) {
            return None;
        }
        let step = 0.25;
        let mut up = step;
        while up <= reach + step {
            if !self.contains(local + DVec3::Y * up) {
                return Some(up - step * 0.5);
            }
            up += step;
        }
        None
    }

    /// Which side of plating at `local` (normal `normal`, either way round) is open air: `Some(1.0)` along the normal,
    /// `Some(-1.0)` against it, `None` for plating with hull on both sides (a bulkhead or an armoured deck inside). A
    /// step of 0.7 m off each face is asked of the hull: her side, her weather deck and anything above the deck
    /// (superstructure, turrets) have open air on one side. A round's entry is open toward the shot, its exit away.
    /// Without a hull, the side along the normal.
    pub fn open_side(&self, local: DVec3, normal: DVec3) -> Option<f64> {
        if self.0.is_none() {
            return Some(1.0);
        }
        let step = normal.normalize_or_zero() * 0.7;
        if !self.contains(local + step) {
            Some(1.0)
        } else if !self.contains(local - step) {
            Some(-1.0)
        } else {
            None
        }
    }
}

impl ShipEffectsProfile {
    /// `mountFrame(def, index, trains)`: a mount's position and heading in the ship's frame, through any mount
    /// carrying it.
    pub fn mount_frame(&self, index: usize, trains: &[f64]) -> ShipPose {
        let mount = &self.mounts[index];
        let mut position = mount.position;
        let mut inherited = 0.0;
        if let Some(parent_index) = mount.parent {
            let parent = &self.mounts[parent_index];
            let pose = self.mount_frame(parent_index, trains);
            inherited = pose.heading - parent.bearing_deg.to_radians();
            position = ShipPose {
                heading: inherited,
                ..pose
            }
            .local_to_world(position - parent.position);
        }
        ShipPose {
            x: position.x,
            y: position.y,
            z: position.z,
            heading: mount.bearing_deg.to_radians()
                + inherited
                + trains.get(index).copied().unwrap_or(0.0),
            ..ShipPose::default()
        }
    }
}

/// How a fire is developing (the web sends it for the followed hull only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FireTrend {
    Growing,
    Contained,
    Cooling,
    Out,
}

/// One compartment's, mount's or upperworks zone's fire as the simulation reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FireState {
    /// 0 (out) to 1.
    pub intensity: f64,
    /// Heat, when the view discloses it (the render packet does for a live hull's burning places; intensity stands in
    /// elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trend: Option<FireTrend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suppressed: Option<bool>,
}

/// A ship as the effects see it this frame.
#[derive(Clone, Debug)]
pub struct ShipFrame {
    pub id: SourceId,
    pub profile: Arc<ShipEffectsProfile>,
    pub pose: ShipPose,
    /// World velocity (m/s), `motionVelocity`.
    pub velocity: DVec3,
    /// Speed through the water along the bow (m/s).
    pub speed: f64,
    /// Per compartment, definition order (may be shorter: missing entries are out).
    pub room_fires: Vec<FireState>,
    /// Per mount, definition order.
    pub mount_fires: Vec<FireState>,
    /// Per upperworks zone, bow first (may be shorter: missing entries are out).
    pub zone_fires: Vec<FireState>,
    /// Per compartment, the highest of its holes above the waterline (ship-local), where a fire in it shows its
    /// flames; a fire in a room without one shows only its smoke, at its vent.
    pub openings: Vec<Option<DVec3>>,
    /// Compartments a quarter full of water or more (a fire going out there goes in steam).
    pub flooded: Vec<bool>,
    /// Mount trains (radians), definition order.
    pub mount_trains: Vec<f64>,
    /// The engine plant's health 0–1 (`systemHealth(actor, def, 'engine')`): funnel smoke thickness and soot.
    pub engine_power: f64,
    pub sunk: bool,
    /// Lost: sinking, or out of the fight for good (the host's: `ShipVisual::sinking` or a defeat cause). Only a lost
    /// hull blows her air out of her funnels and, all under, is gone (a submarine's dive is neither).
    pub lost: bool,
    /// Her other section once she has broken (`ShipVisual::sections`): what lies on its side of her break is placed by
    /// its pose ([`ShipFrame::place`]), her funnels venting and her fires burning where that section is.
    pub section: Option<SectionFrame>,
}

/// A broken hull's other section ([`ShipFrame::section`]): her break's z in her hull frame, whether her own pose
/// (`ShipFrame::pose`) is her fore part's (z under the break), and the other section's pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SectionFrame {
    pub z: f64,
    pub fore: bool,
    pub pose: ShipPose,
}

impl ShipFrame {
    /// A ship at `pose` with no fires, full power and no way on.
    pub fn new(id: &str, profile: Arc<ShipEffectsProfile>, pose: ShipPose) -> Self {
        Self {
            id: Arc::from(id),
            profile,
            pose,
            velocity: DVec3::ZERO,
            speed: 0.0,
            room_fires: Vec::new(),
            mount_fires: Vec::new(),
            zone_fires: Vec::new(),
            openings: Vec::new(),
            flooded: Vec::new(),
            mount_trains: Vec::new(),
            engine_power: 1.0,
            sunk: false,
            lost: false,
            section: None,
        }
    }

    /// The pose of the part of her that holds her hull frame's point `local`: her own, or once she has broken, the
    /// other section's on its side of the break.
    pub fn pose_at(&self, local: DVec3) -> ShipPose {
        match self.section {
            Some(s) if (local.z < s.z) != s.fore => s.pose,
            _ => self.pose,
        }
    }

    /// Her hull frame's point `local` in the world, by the part of her that holds it ([`ShipFrame::pose_at`]).
    pub fn place(&self, local: DVec3) -> DVec3 {
        self.pose_at(local).local_to_world(local)
    }
}
