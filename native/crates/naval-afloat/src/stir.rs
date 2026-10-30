//! What a moving hull stirs, as the effect crates take it: the same inputs the game's client builds for its ships
//! (`naval-client`'s `scene/effects.rs`), from her pose, her speed and her hull's numbers instead of a preset.
//!
//! - The wake and its foam ([`Stir::wake_ship`], `naval_ocean::ship_wake::WakeShip` for `ShipWake::update`): the
//!   wake field's bow and stern generators, the trail's foam and slick (her stern's wash in it), the bow waves and the
//!   contact foam along her sides, each scaled inside the ocean by her speed against her full speed, her length, beam
//!   and draft.
//! - The sea about her ([`Stir::sea_hull`], `naval_ocean::hull_sea::SeaHull` for `hull_footprints` and
//!   `Ocean::set_hull_sea`): where the drawn sea gives way to the sines she rides.
//! - The bow spray ([`Stir::effect_ship`], `naval_water_effects::EffectShip` for `WaterEffects::update`): the sheets
//!   off her bow's crests and what a plunge throws, from her proxy's waterline profile and her pose on the sea.
//! - Funnel smoke ([`Stir::ship_frame`], `naval_effects::ShipFrame` for `CombatEffects::update`): each funnel's
//!   exhaust from its rim, its density and colour from her speed against her full speed and how hard she works up,
//!   at full power (no damage here).

use std::sync::Arc;

use glam::DVec3;
use naval_effects::ships::FunnelOutlet;
use naval_effects::{ShipEffectsProfile, ShipFrame, ShipPose};
use naval_ocean::hull_profile::{SimpleHull, WaterlineProfile};
use naval_ocean::hull_sea::SeaHull;
use naval_ocean::ship_wake::WakeShip;
use naval_water_effects::EffectShip;
use naval_water_effects::fixture::{HullInfo, ShipPose as WaterPose};

use crate::hull::{BadHull, Hull};
use crate::motion::Course;

/// One hull's effect inputs, kept between frames (their lists and her shared profile), refreshed by
/// [`Stir::update`] with her pose.
#[derive(Clone, Debug)]
pub struct Stir {
    id: String,
    shape: SimpleHull,
    profile: WaterlineProfile,
    info: HullInfo,
    /// The pose the water effects read (her id, place, attitude and speed).
    water: WaterPose,
    frame: ShipFrame,
}

impl Stir {
    /// `hull`'s inputs, known to the effects as `id` (the wakes, the spray and the funnels keep their state per hull
    /// by it); refused unless [`Hull::check`] passes. Every one is in ship space about her origin, her centre's offset
    /// with it: the wake's generators and the sea about her from [`Hull::centre`], her profile moved by it.
    pub fn new(id: &str, hull: &Hull) -> Result<Self, BadHull> {
        hull.check()?;
        let shape = hull.shape();
        let profile = hull.profile();
        let info = HullInfo {
            id: id.to_owned(),
            preset: id.to_owned(),
            length: hull.length,
            beam: hull.beam,
            draft: hull.draft,
            forward_speed: hull.full_speed(),
            reverse_speed: hull.reverse_speed(),
            center_x: hull.centre[0],
            center_z: hull.centre[1],
        };
        let freeboard = hull.freeboard();
        let effects = ShipEffectsProfile {
            preset_id: id.to_owned(),
            hull_length: hull.length,
            hull_beam: hull.beam,
            forward_speed: hull.full_speed(),
            reverse_speed: hull.reverse_speed(),
            funnels: hull
                .funnels
                .iter()
                .enumerate()
                .map(|(i, funnel)| FunnelOutlet {
                    id: format!("funnel-{i}"),
                    position: funnel.position,
                    width: funnel.width,
                    length: funnel.length,
                    bearing_rad: None,
                    smoke: funnel.smoke,
                })
                .collect(),
            hull_draft: hull.draft,
            hull_depth: hull.draft + freeboard,
            top_m: hull.top(),
            waterplane_m2: hull.waterplane_area(),
            // Her uptakes rise from her deck.
            funnel_bases: hull.funnels.iter().map(|_| freeboard).collect(),
            ..ShipEffectsProfile::default()
        };
        Ok(Self {
            id: id.to_owned(),
            shape,
            profile,
            water: WaterPose {
                id: id.to_owned(),
                ..WaterPose::default()
            },
            frame: ShipFrame::new(id, Arc::new(effects), ShipPose::default()),
            info,
        })
    }

    /// Her pose this frame, on `course`, moving at `velocity` (world, m/s).
    pub fn update(&mut self, pose: &ShipPose, course: &Course, velocity: DVec3) {
        self.water.position = pose.position().to_array();
        self.water.heading = pose.heading;
        self.water.pitch = pose.pitch;
        self.water.roll = pose.roll;
        self.water.speed = course.speed;
        self.frame.pose = *pose;
        self.frame.velocity = velocity;
        self.frame.speed = course.speed;
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Her proxy hull, about her waterplane's centre: what her waterplane is read over, and a stand-in to draw
    /// (naval-water-effects' `HullLayer`) at her pose moved by [`Hull::centre`].
    pub fn shape(&self) -> &SimpleHull {
        &self.shape
    }

    /// Her proxy's waterline profile in ship space (the contact foam's, the bow waves' and the bow spray's).
    pub fn profile(&self) -> &WaterlineProfile {
        &self.profile
    }

    /// Her pose as the water effects read it.
    pub fn water_pose(&self) -> &WaterPose {
        &self.water
    }

    /// What the wakes read of her this frame (`ShipWake::update`'s ships, the focus hull first).
    pub fn wake_ship(&self) -> WakeShip<'_> {
        let info = &self.info;
        WakeShip {
            id: &self.id,
            position: self.water.position,
            heading: self.water.heading,
            pitch: self.water.pitch,
            roll: self.water.roll,
            speed: self.water.speed,
            length: info.length,
            beam: info.beam,
            draft: info.draft,
            center_x: info.center_x,
            center_z: info.center_z,
            forward_speed: info.forward_speed,
            reverse_speed: info.reverse_speed,
            profile: Some(&self.profile),
            oil_leak_m3s: 0.0,
        }
    }

    /// Where the sea about her gives way to the sines she rides (`hull_footprints`' hulls).
    pub fn sea_hull(&self) -> SeaHull {
        let [x, _, z] = self.water.position;
        SeaHull {
            x,
            z,
            heading: self.water.heading,
            length: self.info.length,
            beam: self.info.beam,
            center_x: self.info.center_x,
            center_z: self.info.center_z,
            contact: 1.0,
        }
    }

    /// What the bow spray reads of her this frame (`EffectsFrame::ships`).
    pub fn effect_ship(&self) -> EffectShip<'_> {
        EffectShip {
            pose: &self.water,
            hull: &self.info,
            profile: &self.profile,
            visible: true,
            lost: false,
            section: None,
        }
    }

    /// What the funnels read of her this frame (`CombatEffects::update`'s ships).
    pub fn ship_frame(&self) -> &ShipFrame {
        &self.frame
    }
}
