//! The orchestrator: the web client's `CombatEffects` (`CombatEffects.ts`) for gas, fire and smoke.
//!
//! Each frame it ages every pool, reads the simulation's new events (each exactly once, by sequence) and spawns
//! from them, then steps the ship fires and funnel smoke from the ships' state. Visual randomness is seeded by
//! the event (`sequence × 7919 + shell id`), so the same event stream draws the same effects on every run.
//! Ballistics, splashes and spray are not here: shells and tracers are the renderer's, water another lane's.

use std::collections::BTreeMap;

use glam::DVec3;

use crate::blasts::{BlastEffects, BlastPools, FlashRequest};
use crate::camera::CameraView;
use crate::debris::DebrisInstance;
use crate::events::CombatEvent;
use crate::fire_batch::FirePublish;
use crate::fires::{FireCounts, LocalizedFires, PointLight};
use crate::funnel::FunnelSmoke;
use crate::instances::{GasPuff, RibbonVertex, Sprite};
use crate::math::{Rgb, v3};
use crate::particles::{ParticlePool, PoolKind};
use crate::random::Lcg;
use crate::ships::ShipFrame;

/// The simulation's fixed step (`naval_sim::rules::DT`, the web's `FIXED_DT`).
pub const SIM_DT: f64 = 1.0 / 60.0;
/// Flash light reaching smoke, per unit of point-light power.
const FLASH_ON_SMOKE: f64 = 0.06;
/// Transient flash lights shining at once: a ring, the newest replacing the oldest (four in the web game; since
/// 2026-10-20 enough for a night action's salvos, each turret's flash and each strike's, whose fireballs glow for up to
/// a second). The renderer keeps the brightest at the camera of every light the scene has
/// (`naval_render::PointLights::brightest`).
pub const FLASH_LIGHTS: usize = 32;
/// The flashes whose light the gas takes ([`EffectsFrame::flashes`]): the four brightest, as the web game's four.
pub const SMOKE_FLASHES: usize = 4;
/// Heavy AA bursts' clouds of soot alive at once, one a round (`flak.rs`): Shōkaku's strike on the Enterprise's task
/// force draws some 150 rounds in its busiest 10 s (`effects-trial.ts --only flak-battle`), and each hangs for up to
/// 110 s (`flak.rs`' `LINGER`). Past this the most faded give way.
pub const FLAK_PUFFS: usize = 1536;

/// The scene's light and wind as every effect reads them: the web's `EffectLighting`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectLighting {
    /// Toward the sun, or toward the moon when it is the brighter light.
    pub sun_direction: DVec3,
    /// Direct radiance scattered by smoke: celestial colour × intensity, about 1.25 at a clear noon.
    pub direct: DVec3,
    /// Sky fill that reaches shaded folds, about (0.3, 0.35, 0.4) by day: the stand-in for the sky's environment where
    /// a material cannot read it (`sky_fill` in `common.wgsl`).
    pub ambient: DVec3,
    /// Lightning's flash, a multiple of the sky's light on the effects (0 without).
    pub flash: f64,
    /// 0–1 overall scene brightness.
    pub daylight: f64,
    /// The weather's wind in m/s, ten metres over the sea (`SeaState::wind_mps`, the wind the waves and whitecaps are
    /// drawn for): smoke drifts with it at its own height ([`wind_share`]), what floats with a share of it. It was 0.35
    /// of the weather's until 2026-10-24, a choice the web game made when its wind was a parameter of the waves'
    /// shader; a burning ship's column stood upright over a whitecapped sea.
    pub wind: DVec3,
    /// The night's lift on real light (`naval_render::lights::night_lift`: 1 by day, 16,700 after dark), which the host
    /// sets each frame ([`EffectLighting::set_lift`]). What the effects give in the sun's units takes it: the glow of the
    /// gas, the propellant, the sparks, the hot fragments and the tracers, so a flash stands beside the moon as the star
    /// shells and searchlights do (`native/docs/lighting/lights.md`, "Lights at night"). The fires and the flashes' light on the
    /// scene and the smoke take less of it (below).
    pub lift: f64,
    /// After dark, at Bill's word (2026-10-21: "make fires significantly less bright but still a bit brighter than
    /// before"), a fire takes less of the night's lift than a flash does: its flames and embers ([`FLAMES_AT_NIGHT`]
    /// their day's glow), its glow on its own smoke ([`FIRE_GLOW_AT_NIGHT`]) and its light on the scene
    /// ([`FIRELIGHT_AT_NIGHT`]); by day and through dusk the lift as before ([`EffectLighting::set_lift`];
    /// `native/docs/lighting/lights.md`, "Fires and shots at night").
    pub flames: f64,
    pub glow: f64,
    pub firelight: f64,
    /// The flashes' and blasts' own glow (the flash sprites and sparks, the gas of a gun or a blast, a detonation's
    /// fireball), their light on the scene (the hulls, the sea) and on the smoke and gas about them (`FLASH_ON_SMOKE`):
    /// the night's lift by day and through dusk; after dark [`FLASH_GLOW_AT_NIGHT`] times their day's glow, and
    /// [`FLASHLIGHT_AT_NIGHT`] and [`FLASH_SMOKE_AT_NIGHT`] of the lift (Bill: "make shots at night a bit less bright",
    /// "make light at night a bit [less] bright so it doesnt instantly make the screen all white").
    pub flash_glow: f64,
    pub flashlight: f64,
    pub flash_smoke: f64,
}

/// What of their day's glow and light the fires keep at the night's full lift (16,700): their flames and embers 32
/// times it, their glow on their own smoke twenty times, their light on the scene 500 times. The light takes some
/// sixteen times the flames' share, where the flames the flipbook draws would give it six to eight times
/// (`native/docs/combat-effects/fire-and-smoke.md`, "At night"); the look `bright-flames`
/// ([`BRIGHT_FLAMES_AT_NIGHT`]) has them in that proportion. Until 2026-10-25 the flames took sixteen times and the
/// light 2,000 (125 times their share): the deck beside a fire shone as bright as its flames, and a fire's light lit
/// her plating through her turrets. Taking the whole lift (2026-10-20 to 21), a burning ship closed the eye 8.3 stops
/// when she filled a tenth of the view and stood as a red glare on a black sky; before that her flames kept their day's
/// glow, her glow on her smoke four to six times it and her light some 470 times it (`FIRE_NIGHT_LIGHT`). The flames'
/// four times became sixteen on 2026-10-24, when their day's glow fell to about a quarter (a black body's,
/// `common.wgsl`'s `flame_light`).
pub const FLAMES_AT_NIGHT: f64 = 32.0;
pub const FIRE_GLOW_AT_NIGHT: f64 = 20.0;
pub const FIRELIGHT_AT_NIGHT: f64 = 500.0;

/// What of their day's glow and their day's light on the scene a fire's flames keep at the night's full lift.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FireNight {
    pub flames: f64,
    pub light: f64,
}

/// The fires' night levels: [`FLAMES_AT_NIGHT`] and [`FIRELIGHT_AT_NIGHT`].
pub const FIRES_AT_NIGHT: FireNight = FireNight {
    flames: FLAMES_AT_NIGHT,
    light: FIRELIGHT_AT_NIGHT,
};
/// The `bright-flames` look's night levels (`crate::Trial::bright_flames`): the flames a stop over the game's, their
/// light on the scene as the game's, the two as the flames the flipbook draws would light the scene (the light some
/// eight times the flames' share of the lift). Where the flames fill enough of the view to close the eye, it closes a
/// stop more, and what they light shows a stop darker beside them.
pub const BRIGHT_FLAMES_AT_NIGHT: FireNight = FireNight {
    flames: FLAMES_AT_NIGHT * 2.0,
    light: FIRELIGHT_AT_NIGHT,
};
/// Once the fires' light on the scene is lifted more than this many times its day's (from about sunset on), the hulls
/// march toward it on screen (`naval_effects::fires::PointLight::marched`, `naval-ships`' `fire_seen`), so it does not
/// light her through her own superstructure; by day it lights nothing but its own flames and smoke, and the march would
/// cost for nothing.
pub const FIRELIGHT_MARCHED: f64 = 4.0;
/// What of their day's glow a flash and a blast keep at the night's full lift: 64 times it, some eight stops under the
/// lift, and the shares of the lift their light on the scene and on the smoke takes, a sixteenth and a thirty-second (the
/// smoke a stop under the scene as before). At the whole lift (2026-10-20 to 25) a 38 cm salvo's flash stood some 16 stops
/// over the moonlit sea and its bloom alone whited out the view from 240 m astern with the eye off, and veiled it with the
/// eye on, which closed some 12 stops for it; its light lit the ship and smoke 80 m off as a day would (about 14,000 lux),
/// the smoke at half the lift.
pub const FLASH_GLOW_AT_NIGHT: f64 = 64.0;
pub const FLASHLIGHT_AT_NIGHT: f64 = 1.0 / 16.0;
pub const FLASH_SMOKE_AT_NIGHT: f64 = 1.0 / 32.0;

impl EffectLighting {
    /// The night's lift this frame (`naval_render::lights::night_lift`) and how far the night has come (`night`: 0 by
    /// day and through dusk, toward 1 as the twilight fades; `naval_sky::model::night_share`), and what the fires and the
    /// flashes' light on the smoke take of the lift: all of it through dusk as before, their night's levels after dark,
    /// between them geometrically.
    pub fn set_lift(&mut self, lift: f64, night: f64) {
        self.set_lift_with(lift, night, FIRES_AT_NIGHT);
    }

    /// [`EffectLighting::set_lift`] with the fires' night levels `fires` ([`FIRES_AT_NIGHT`], or the `bright-flames`
    /// look's [`BRIGHT_FLAMES_AT_NIGHT`]: [`CombatEffects::set_lift`]).
    pub fn set_lift_with(&mut self, lift: f64, night: f64, fires: FireNight) {
        let lift = lift.max(1.0);
        let night = night.clamp(0.0, 1.0);
        let toward = |level: f64| lift * (level / lift).powf(night);
        self.lift = lift;
        self.flames = toward(fires.flames);
        self.glow = toward(FIRE_GLOW_AT_NIGHT);
        self.firelight = toward(fires.light);
        self.flash_glow = toward(FLASH_GLOW_AT_NIGHT);
        self.flashlight = lift * FLASHLIGHT_AT_NIGHT.powf(night);
        self.flash_smoke = lift * FLASH_SMOKE_AT_NIGHT.powf(night);
    }
}

impl Default for EffectLighting {
    fn default() -> Self {
        Self {
            sun_direction: DVec3::new(-0.55, 0.74, -0.39).normalize(),
            direct: DVec3::new(1.25, 1.19, 1.08),
            ambient: DVec3::new(0.3, 0.35, 0.4),
            flash: 0.0,
            daylight: 1.0,
            wind: DVec3::new(2.4, 0.0, 0.9),
            lift: 1.0,
            flames: 1.0,
            glow: 1.0,
            firelight: 1.0,
            flash_glow: 1.0,
            flashlight: 1.0,
            flash_smoke: 1.0,
        }
    }
}

/// The wind over the sea at a height, as a share of the wind ten metres up ([`EffectLighting::wind`]): the sea's
/// power law in near-neutral air, u(z) = u₁₀·(z/10)^0.11 (Hsu, Meindl and Gilhousen, 1994), from two metres up, to the
/// clouds' drift, 4/3 of it (`naval_sim::environment`, `cloud_velocity`), which it reaches some 140 m up: the wind at
/// the top of the sea's boundary layer, where a fire's pall spreads.
pub const WIND_EXPONENT: f64 = 0.11;
pub const WIND_ALOFT: f64 = 4.0 / 3.0;

/// What of the wind ten metres up ([`EffectLighting::wind`]) blows `height` metres over the sea ([`WIND_EXPONENT`]).
pub fn wind_share(height: f64) -> f64 {
    (height.max(2.0) / 10.0).powf(WIND_EXPONENT).min(WIND_ALOFT)
}

impl EffectLighting {
    /// `setWind(speed, direction)`: the weather's wind ten metres up (m/s), toward radians from +X toward +Z, as the
    /// ocean and funnels use.
    pub fn set_wind(&mut self, speed: f64, direction: f64) {
        self.wind = DVec3::new(direction.cos(), 0.0, direction.sin()) * speed;
    }

    /// `setIllumination(color, intensity, ambient)`: the scene's daylight or moonlight.
    pub fn set_illumination(&mut self, color: Rgb, intensity: f64, ambient: f64) {
        self.direct = DVec3::new(color.r, color.g, color.b) * (intensity.max(0.0) * 1.25 / 5.8);
        self.ambient = DVec3::new(0.3, 0.35, 0.4) * (ambient.max(0.0) / 1.75);
    }
}

#[derive(Clone, Copy, Debug)]
struct FlashLight {
    position: DVec3,
    age: f64,
    power: f64,
    /// Seconds at full power before it decays over `duration`.
    hold: f64,
    duration: f64,
    distance: f64,
    /// The fireball's glow after the flash: its share of the power and its time constant (s).
    glow: f64,
    glow_seconds: f64,
    intensity: f64,
}

impl Default for FlashLight {
    fn default() -> Self {
        Self {
            position: DVec3::ZERO,
            age: 1.0,
            power: 0.0,
            hold: 0.0,
            duration: 0.2,
            distance: 145.0,
            glow: 0.0,
            glow_seconds: 0.0,
            intensity: 0.0,
        }
    }
}

/// A fireball's glow is put out once it is this share of its flash's power.
const GLOW_OUT: f64 = 0.004;
/// A flash's light: the burning propellant gas of the secondary flash, about 2,500 K (`#ffa64a`; it was `#ffd29a`,
/// some 3,900 K, before 2026-10-20: `native/docs/lighting/lights.md`, "Lights at night"). A fireball's glow after it: the
/// fires' orange, about 2,000 K (`fires.rs`).
const FLASH_COLOR: u32 = 0xffa64a;
const GLOW_COLOR: u32 = 0xff8a3c;

impl FlashLight {
    /// The flash's and the fireball's glow's shares of the power at the light's age: the flash held, then decaying over
    /// its duration, then what the glow keeps.
    fn parts(&self) -> (f64, f64) {
        let decaying = (self.age - self.hold).max(0.0);
        let flash = if decaying < self.duration {
            (-decaying / self.duration * 5.0).exp()
        } else {
            0.0
        };
        let glow = if self.glow > 0.0 && self.glow_seconds > 0.0 {
            let glow = self.glow * (-self.age / self.glow_seconds).exp();
            if glow > GLOW_OUT { glow } else { 0.0 }
        } else {
            0.0
        };
        (flash, glow)
    }

    fn intensity_at(&self) -> f64 {
        let (flash, glow) = self.parts();
        self.power * (flash + glow)
    }

    /// The light's colour: the flash's, reddening to the glow's as the flash fades.
    fn color(&self) -> Rgb {
        let (flash, glow) = self.parts();
        let glowing = if flash + glow > 0.0 {
            glow / (flash + glow)
        } else {
            0.0
        };
        Rgb::hex(FLASH_COLOR).lerp(Rgb::hex(GLOW_COLOR), glowing)
    }
}

/// Everything the effects draw this frame, its batches in draw order (see the lane doc).
#[derive(Clone, Debug, Default)]
pub struct EffectsFrame {
    /// Additive flashes, flame jets, sparks and burning grains.
    pub fire: Vec<Sprite>,
    /// Gun gas, fireballs, strike and explosion smoke, flak: the occluding gas batches.
    pub gas: Vec<GasPuff>,
    /// Smouldering holes: gas that only reads the occlusion map.
    pub soot: Vec<GasPuff>,
    /// The game's look: the funnels' exhaust as puffs of simulated smoke, back to front (`funnel.rs`, `draw_puffs`),
    /// occluding and merged as the gas is.
    pub funnel_gas: Vec<GasPuff>,
    pub fire_smoke: FirePublish,
    pub funnel_ribbon_vertices: Vec<RibbonVertex>,
    pub funnel_ribbon_indices: Vec<u32>,
    pub funnel_billows: Vec<Sprite>,
    /// Trial `flipbook`: the funnels' exhaust as chains of plume links, back to front (`funnel.rs`, `draw_links`).
    pub funnel_links: Vec<GasPuff>,
    pub fire_bodies: Vec<GasPuff>,
    pub fire_flames: FirePublish,
    pub fire_embers: Vec<Sprite>,
    /// Strike debris: lit solid shards, drawn in the opaque phase.
    pub debris: Vec<DebrisInstance>,
    /// Every piece of debris in flight, in view or not: the sun's shadow casters.
    pub debris_casters: Vec<DebrisInstance>,
    /// The four brightest flashes as the gas shader sees them: xyz position, w power on smoke.
    pub flashes: [[f32; 4]; SMOKE_FLASHES],
    /// Their light's colour (linear), weighed by their power: the flash's 2,500 K reddening to the fireball's glow, as
    /// their light on the scene is (`FlashLight::color`).
    pub flash_color: [f32; 3],
    /// Lights for the layers that light hulls and the sea: the flashes, then the fire lights.
    pub lights: Vec<PointLight>,
    /// The bodies marched as volumes in place of their sprites (the volume trial; none live otherwise).
    pub volumes: crate::volume::VolumeFrame,
}

/// Counts for telemetry and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EffectCounts {
    pub gas: usize,
    pub fire: usize,
    pub soot: usize,
    pub smoulders: usize,
    pub flak: usize,
    pub aircraft_smoke: usize,
    pub fires: FireCounts,
    pub funnel_billows: usize,
    pub funnel_segments: usize,
    /// Strike debris in the air.
    pub debris: usize,
    pub events: u64,
}

pub struct CombatEffects {
    pub tactical: crate::tactical::TacticalEffects,
    pub lighting: EffectLighting,
    /// The blast waves of the guns and explosions in the air ([`crate::ShockLayer`] draws them); none while
    /// `trial.shock` is off.
    pub shocks: crate::shock::ShockWaves,
    /// The fronts' last steps, as the smoke they passed takes them (kept for its room).
    shoves: Vec<crate::shock::Shove>,
    /// The looks switched on (`crate::Trial`; the default, every look off, until the host sets the game's).
    pub trial: crate::Trial,
    /// Look `flipbook`: whether the fire's simulation is loaded (`EffectsLayer::set_flipbooks`). Without it the
    /// ship fires keep the tongues of flame.
    pub flipbook_fire: bool,
    /// Look `flipbook`: whether the smoke column's simulation is loaded; without it the columns keep the still puffs.
    pub flipbook_plume: bool,
    /// Look `flipbook`: the loaded clips' table, so the smoke's shadow takes each puff's gas at the size it is drawn.
    pub flipbook_sizes: crate::flipbook::ClipSizes,
    /// Events a harness staged ([`CombatEffects::stage`]), spawned from on the next update.
    staged: Vec<CombatEvent>,
    smoke: ParticlePool,
    flak_smoke: ParticlePool,
    /// Burning aircraft's smoke trails (the web's `aircraftSmoke` pool, filled by `updateAircraftSmoke`). Emitted
    /// from outside through [`CombatEffects::aircraft_pools`] (naval-aircraft's `SmokeTrails`); drawn in the gas
    /// batch between the combat smoke and the flak.
    aircraft_smoke: ParticlePool,
    fire: ParticlePool,
    blasts: BlastEffects,
    fires: LocalizedFires,
    funnels: FunnelSmoke,
    lights: [FlashLight; FLASH_LIGHTS],
    light_cursor: usize,
    requests: Vec<FlashRequest>,
    airbursts: BTreeMap<u64, CombatEvent>,
    sequence: u64,
    events_seen: u64,
    /// The hull the lens sits on: its own smoke is left out so the view from its bridge stays clear.
    pub optics_ship: Option<String>,
    /// The fires' last publish, kept as room (its lists are traded into each frame's).
    fires_room: crate::fires::FiresPublish,
    /// The volume trial's bodies between frames (`volume.rs`); idle unless `trial.volume`.
    volumes: crate::volume::Volumes,
    /// The losses followed: funnels venting as the sea reaches her boilers and uptakes, the moment she has gone, and
    /// her flotsam on the water (`loss.rs`, `flotsam.rs`).
    pub losses: crate::loss::Losses,
    /// The sea's long swell the flotsam rides (the combat sea every hull rides), set by the host; flat without one.
    pub swell: Option<crate::flotsam::Swell>,
    /// The simulation's numbers the effects draw by, set by the host (`naval_effects_sim::rules`); the defaults are
    /// the simulation's.
    pub rules: crate::EffectRules,
    /// Oil burning on the water (`sea_fires.rs`), set by the host from its slicks before each update.
    pub sea_fires: crate::sea_fires::SeaFires,
    /// Ready-use rounds cooking off in burning upperworks (`cookoffs.rs`): the host draws their tracers and sounds
    /// their cracks.
    pub cookoffs: crate::cookoffs::CookOffs,
}

impl Default for CombatEffects {
    fn default() -> Self {
        Self::new()
    }
}

impl CombatEffects {
    pub fn new() -> Self {
        Self {
            tactical: Default::default(),
            lighting: EffectLighting::default(),
            shocks: Default::default(),
            shoves: Vec::new(),
            trial: crate::Trial::default(),
            flipbook_fire: false,
            flipbook_plume: false,
            flipbook_sizes: Default::default(),
            staged: Vec::new(),
            smoke: ParticlePool::new(1024, PoolKind::Gas),
            flak_smoke: ParticlePool::new(FLAK_PUFFS, PoolKind::Gas),
            aircraft_smoke: ParticlePool::new(768, PoolKind::Gas),
            fire: ParticlePool::new(1024, PoolKind::Additive),
            blasts: BlastEffects::new(),
            fires: LocalizedFires::new(),
            funnels: FunnelSmoke::new(),
            lights: [FlashLight::default(); FLASH_LIGHTS],
            light_cursor: 0,
            requests: Vec::new(),
            airbursts: BTreeMap::new(),
            sequence: 0,
            events_seen: 0,
            optics_ship: None,
            fires_room: Default::default(),
            volumes: Default::default(),
            losses: crate::loss::Losses::new(),
            swell: None,
            rules: Default::default(),
            sea_fires: Default::default(),
            cookoffs: Default::default(),
        }
    }

    /// Carry the continuous ship effects into a rebuilt theatre scene. GPU batches belong to the new layer; the
    /// released smoke and each emitter's age belong to the world, carried into the new scene by `shift` (the old scene's
    /// origin less the new one's, `[x, z]` m: each point `q` becomes `q + (x, 0, z)`; the host's `SceneShift::xz`).
    pub fn take_ship_history_from(&mut self, old: &mut Self, shift: [f64; 2]) {
        let [x, z] = shift;
        let offset = DVec3::new(x, 0.0, z);
        std::mem::swap(&mut self.fires, &mut old.fires);
        std::mem::swap(&mut self.funnels, &mut old.funnels);
        std::mem::swap(&mut self.losses, &mut old.losses);
        std::mem::swap(&mut self.sea_fires, &mut old.sea_fires);
        std::mem::swap(&mut self.cookoffs, &mut old.cookoffs);
        self.fires.translate(offset);
        self.funnels.translate(offset);
        self.losses.translate(offset);
        self.sea_fires.translate(offset);
        self.cookoffs.translate(offset);
    }

    /// [`Self::take_ship_history_from`] into a scene laid on another chart (a theatre's re-lay): the released smoke and
    /// the emitters re-expressed by their kinds (`naval_render::relay::SceneRelay`) rather than translated.
    pub fn relay_ship_history_from(
        &mut self,
        old: &mut Self,
        relay: &dyn naval_render::relay::SceneRelay,
    ) {
        std::mem::swap(&mut self.fires, &mut old.fires);
        std::mem::swap(&mut self.funnels, &mut old.funnels);
        std::mem::swap(&mut self.cookoffs, &mut old.cookoffs);
        self.fires.relay(relay);
        self.funnels.relay(relay);
        self.cookoffs.relay();
    }

    /// Spawns from an event the battle did not send on the next update, outside the sequence its own events are
    /// read by: a harness staging what a battle rarely does (a magazine's detonation). The event's `sequence` only
    /// seeds its random draws.
    pub fn stage(&mut self, event: CombatEvent) {
        self.staged.push(event);
    }

    /// Advance the effects by `dt` seconds and spawn from every event newer than the last one read.
    ///
    /// `now` is the presentation clock in simulation seconds (`(tick − 1 + alpha) × DT`, as the web's
    /// `updateAirbursts` measures it): delayed AA bursts go off against it. `camera` is the one the frame will
    /// be drawn with: fire relevance and funnel detail follow it.
    pub fn update(
        &mut self,
        dt: f64,
        now: f64,
        camera: &CameraView,
        events: &[CombatEvent],
        ships: &[ShipFrame],
    ) {
        let wind = self.lighting.wind;
        // Advance before emitting: a slow frame still gets one visible muzzle flash.
        for light in &mut self.lights {
            light.age += dt;
            light.intensity = light.intensity_at();
        }
        self.blasts.clear_fired();
        self.shocks.enabled = self.trial.shock;
        self.shocks.advance(dt);
        self.smoke.advance(dt, wind);
        self.flak_smoke.advance(dt, wind);
        self.aircraft_smoke.advance(dt, wind);
        self.fire.advance(dt, wind);
        self.blasts.update(dt, wind, ships, &mut self.fire);
        // The fronts carry the smoke they passed this step: gun smoke, fires' columns, funnel smoke.
        self.shocks.shoves(&mut self.shoves);
        if !self.shoves.is_empty() {
            self.smoke.shove(&self.shoves);
            self.flak_smoke.shove(&self.shoves);
            self.aircraft_smoke.shove(&self.shoves);
            self.blasts.soot.shove(&self.shoves);
            self.fires.shove(&self.shoves);
            self.funnels.shove(&self.shoves);
        }
        self.blasts.flipbook = self.trial.flipbook;
        self.fires.flipbook = self.trial.flipbook && self.flipbook_fire;
        self.fires.column = self.trial.flipbook && self.flipbook_plume;
        self.fires.billows = self.flipbook_sizes.has(crate::flipbook::clip::SMOKE);
        // The funnels' plume chain is a look of its own (`funnel-plume`); the game's are puffs of the smoke clip.
        self.funnels.plume = self.trial.flipbook && self.trial.funnel_plume && self.flipbook_plume;
        self.funnels.puffs = self.trial.flipbook
            && self.trial.funnel_puffs
            && self.flipbook_sizes.has(crate::flipbook::clip::SMOKE);
        if self.funnels.puffs {
            self.funnels.set_smoke_sizes(&self.flipbook_sizes);
        }
        self.funnels.daylight = self.lighting.daylight;
        for event in events {
            if event.sequence <= self.sequence {
                continue;
            }
            self.sequence = event.sequence;
            self.events_seen += 1;
            self.emit(event, ships);
        }
        for event in std::mem::take(&mut self.staged) {
            self.emit(&event, ships);
        }
        {
            let mut pools = BlastPools {
                volumes: &mut self.smoke,
                fire: &mut self.fire,
                flashes: &mut self.requests,
            };
            self.blasts.flush_shots(&mut pools);
            self.cookoffs
                .update(dt, camera.position, ships, &mut pools, &self.lighting);
        }
        // Each turret's blast and each explosion in the air sends its front out.
        for blast in self.blasts.fired() {
            self.shocks.gun(blast);
        }
        for burst in self.blasts.bursts() {
            self.shocks.burst(burst.at, burst.charge_kg, burst.sequence);
        }
        // Before the lights are lit, so a burst's flash lights the frame it bursts in.
        self.update_airbursts(now);
        for request in std::mem::take(&mut self.requests) {
            self.illuminate(request);
        }
        let hidden = self.optics_ship.clone();
        self.fires.swell.clone_from(&self.swell);
        self.fires.rules = self.rules;
        self.fires.update_with_sea(
            dt,
            camera,
            wind,
            hidden.as_deref(),
            ships,
            self.sea_fires.frames(),
        );
        self.funnels
            .update(ships, dt, camera, wind, hidden.as_deref());
        // A hull burning above the water takes fire down with her if she goes soon after.
        for ship in ships {
            if self.fires.burning_above_water(&ship.id) {
                self.losses.note_fire(&ship.id);
            }
        }
        // Her funnels venting, the moment she is gone and what she leaves on the water; the planks a blast threw that
        // landed on the water float there.
        self.losses.flotsam.density = self.smoke.density;
        let billows = self.trial.flipbook && self.flipbook_sizes.has(crate::flipbook::clip::SMOKE);
        self.losses
            .update(dt, wind, ships, &mut self.smoke, billows);
        for (at, length, seed, source) in self.blasts.debris.take_landed() {
            let mut random = Lcg::from_number(f64::from(seed) * 7919.0);
            self.losses
                .flotsam
                .plank(at, length, &mut random, source.as_ref());
        }
        if self.trial.volume {
            self.volumes.advance(dt);
        }
    }

    fn emit(&mut self, event: &CombatEvent, ships: &[ShipFrame]) {
        if event.kind == "aircraft-fire" {
            if event
                .aircraft
                .as_ref()
                .is_some_and(|a| a.airburst.is_some() && a.target.is_some())
            {
                self.airbursts.insert(event.sequence, event.clone());
            }
            return;
        }
        let seed =
            event.sequence as f64 * 7919.0 + event.shell.as_ref().map_or(0.0, |s| s.id as f64);
        let mut random = Lcg::from_number(seed);
        let mut pools = BlastPools {
            volumes: &mut self.smoke,
            fire: &mut self.fire,
            flashes: &mut self.requests,
        };
        match event.kind.as_str() {
            "aircraft-crash" if event.detonation == Some(true) => {
                self.blasts.he_burst(event, &mut random, &mut pools, ships);
            }
            // Water: another lane's (splash columns, spray, foam).
            "aircraft-crash"
            | "depth-charge-blast"
            | "depth-charge-splash"
            | "torpedo-hit"
            | "torpedo-launch"
            | "torpedo-dud"
            | "splash" => {}
            "shot" => self.blasts.queue_shot(event),
            // Its column is the water lane's too; the air takes a share of its blast.
            "burst" if event.detonation == Some(true) && event.water_burst_y.is_some() => {
                self.blasts.water_burst(event)
            }
            // A shell's contact (an HE round on the plating, holing it or not) is remembered for the burst that
            // follows; a ramming or grounding contact is the water lane's.
            "contact" if event.shell.is_some() => {
                if event.normal.is_some() {
                    self.blasts.impact(event, &mut random, &mut pools, ships)
                }
            }
            "contact" if event.hull_damage.is_some() => {}
            "burst" if event.detonation == Some(true) => {
                self.blasts.burst(event, &mut random, &mut pools, ships)
            }
            _ if event.detonation == Some(true) => {
                // The volume trial marches the newest detonation: its gas is tagged as it is emitted.
                if self.trial.volume {
                    let mut seat = v3(event.position);
                    seat.y = seat.y.max(3.0);
                    pools.volumes.tag = self.volumes.detonate(seat);
                }
                self.blasts.magazine(event, &mut random, &mut pools, ships);
                pools.volumes.tag = 0;
            }
            "stopped" if event.struck_land() => {
                self.blasts.land_strike(event, &mut random, &mut pools)
            }
            _ if event.normal.is_some() => {
                self.blasts.impact(event, &mut random, &mut pools, ships)
            }
            // Internal damage and sinking are state changes, not external fireballs.
            _ => {}
        }
    }

    /// A brief point light, held for `hold`, then decaying over `duration`.
    fn illuminate(&mut self, request: FlashRequest) {
        let light = &mut self.lights[self.light_cursor % FLASH_LIGHTS];
        self.light_cursor += 1;
        *light = FlashLight {
            position: request.position,
            age: 0.0,
            power: request.power,
            hold: request.hold,
            duration: request.duration,
            distance: request.distance,
            glow: request.glow,
            glow_seconds: request.glow_seconds,
            intensity: 0.0,
        };
        light.intensity = light.intensity_at();
    }

    /// The turrets that fired in the last [`CombatEffects::update`], one blast each: the host hands them to
    /// naval-water-effects' blast sheets, the spray each tears off the sea under its guns.
    pub fn turret_blasts(&self) -> &[crate::blasts::TurretBlast] {
        self.blasts.fired()
    }

    /// Heavy AA bursts whose fuze time has run (`flak.rs`): a flash, its light and the soot it leaves.
    fn update_airbursts(&mut self, now: f64) {
        let due: Vec<u64> = self.airbursts.keys().copied().collect();
        for sequence in due {
            let event = &self.airbursts[&sequence];
            let data = event.aircraft.as_ref().expect("queued with aircraft data");
            let burst = data.airburst.as_ref().expect("queued with an airburst");
            let age = now - event.tick as f64 * SIM_DT - burst.flight_time;
            if age < 0.0 {
                continue;
            }
            let event = self.airbursts.remove(&sequence).expect("present");
            let data = event.aircraft.as_ref().expect("queued with aircraft data");
            // A late frame draws the burst at its age, without replaying an old flash.
            crate::flak::burst(
                &crate::flak::Airburst {
                    at: v3(data.target.expect("queued with a target")),
                    caliber_m: data
                        .airburst
                        .as_ref()
                        .expect("queued with an airburst")
                        .caliber_m,
                    age,
                    wind: self.lighting.wind,
                },
                &mut Lcg::from_number(event.sequence as f64 * 7919.0),
                crate::flak::FlakPools {
                    soot: &mut self.flak_smoke,
                    fire: &mut self.fire,
                    flashes: &mut self.requests,
                },
            );
        }
    }

    /// The lights the effects shine on the scene this frame, after [`CombatEffects::update`]: the flashes still
    /// burning (`#ffa64a`, at most [`FLASH_LIGHTS`]), then every fire burning in view (`#ff8a3c`, from up to four points
    /// among its flames, marched toward on the hulls after sunset: [`FIRELIGHT_MARCHED`]), in the scene's units (the
    /// sun's, `naval_render::lights::DAY_LUX` lux a unit, times the night's lift, [`EffectLighting::lift`]).
    /// They are point lights, which light every lit material and the sea: hand them to the renderer
    /// (`naval_render::Renderer::set_point_lights`). Also in [`EffectsFrame::lights`] once published.
    pub fn point_lights(&self) -> Vec<PointLight> {
        let mut lights = Vec::new();
        self.point_lights_into(&mut lights);
        lights
    }

    /// [`CombatEffects::point_lights`] written over `lights` (which keeps its room).
    pub fn point_lights_into(&self, lights: &mut Vec<PointLight>) {
        lights.clear();
        let flashlight = self.lighting.flashlight;
        let lifted = |light: PointLight| PointLight {
            intensity: light.intensity * flashlight,
            ..light
        };
        lights.extend(self.flash_lights().map(lifted));
        let firelight = self.lighting.firelight;
        let marched = firelight > FIRELIGHT_MARCHED;
        lights.extend(self.fires.lights().iter().map(|light| PointLight {
            intensity: light.intensity * firelight,
            marched,
            ..*light
        }));
    }

    /// The flashes still burning (a muzzle flash, a strike, a burst, a magazine and the fireball each leaves), for
    /// what glares about them: the host's bloom. In the sun's units, without the night's lift.
    pub fn flashes(&self) -> impl Iterator<Item = PointLight> + '_ {
        self.flash_lights()
    }

    /// The colour of the light the flashes at `indices` give the smoke: theirs, weighed by their power (the flash's own
    /// with none burning). Until 2026-10-21 the gas took the flash's old colour, `#ffd29a` (some 3,900 K), after its light
    /// on the scene went to 2,500 K: the smoke beside a turret firing at night came out cream where the deck under it was
    /// orange.
    fn smoke_flash_color(&self, indices: &[usize]) -> [f32; 3] {
        let (mut sum, mut power) = (Rgb::new(0.0, 0.0, 0.0), 0.0);
        for &index in indices {
            let light = &self.lights[index];
            if light.intensity > 0.0 {
                let color = light.color();
                sum = Rgb::new(
                    sum.r + color.r * light.intensity,
                    sum.g + color.g * light.intensity,
                    sum.b + color.b * light.intensity,
                );
                power += light.intensity;
            }
        }
        let color = if power > 0.0 {
            sum.scale(1.0 / power)
        } else {
            Rgb::hex(FLASH_COLOR)
        };
        [color.r as f32, color.g as f32, color.b as f32]
    }

    /// The flashes still burning, as point lights (`#ffa64a`, a fireball's glow `#ff8a3c`).
    fn flash_lights(&self) -> impl Iterator<Item = PointLight> + '_ {
        self.lights
            .iter()
            .filter(|light| light.intensity > 0.0)
            .map(|light| PointLight {
                position: light.position,
                color: light.color(),
                intensity: light.intensity,
                distance: light.distance,
                radius: 0.0,
                marched: false,
            })
    }

    /// Cull, sort and pack every batch for `camera`.
    pub fn publish(&mut self, camera: &CameraView) -> EffectsFrame {
        let mut frame = EffectsFrame::default();
        self.publish_into(camera, &mut frame);
        frame
    }

    /// [`CombatEffects::publish`] written over `frame`, whose lists keep their room (a host keeping its frame
    /// allocates nothing per frame).
    pub fn publish_into(&mut self, camera: &CameraView, frame: &mut EffectsFrame) {
        let hidden = self.optics_ship.take();
        let ship = hidden.as_deref();
        if self.trial.volume {
            // Before the sprites publish: the bodies' particles are left out of them.
            self.volumes.publish(
                camera,
                ship,
                self.fires.smoke_batch_mut(),
                &mut self.smoke,
                self.lighting.sun_direction,
                self.lighting.wind,
                &mut frame.volumes,
            );
        }
        frame.gas.clear();
        self.smoke.publish_gas_into(camera, ship, &mut frame.gas);
        self.aircraft_smoke
            .publish_gas_into(camera, None, &mut frame.gas);
        // Flak is its own pool so distant bursts stay visible in optics; one occluding batch draws both, flak
        // after (it sorts only within its own pool).
        self.flak_smoke
            .publish_gas_into(camera, None, &mut frame.gas);
        self.tactical.publish_smoke(camera, &mut frame.gas);
        self.point_lights_into(&mut frame.lights);
        // The fires' lists traded with the room's (last frame's become its room), not copied.
        let room = &mut self.fires_room;
        self.fires.publish_into(camera, ship, room);
        std::mem::swap(&mut frame.fire_smoke, &mut room.smoke);
        // The lamps' and the star shells' own light, their beams and their glow in the air are the air light's
        // (`air_light.rs`), drawn by the layer from `tactical`.
        std::mem::swap(&mut frame.fire_bodies, &mut room.bodies);
        std::mem::swap(&mut frame.fire_flames, &mut room.flames);
        std::mem::swap(&mut frame.fire_embers, &mut room.embers);
        self.funnels.publish_into(
            camera,
            ship,
            &mut frame.funnel_billows,
            &mut frame.funnel_ribbon_vertices,
            &mut frame.funnel_ribbon_indices,
        );
        self.funnels.publish_links_into(&mut frame.funnel_links);
        self.funnels
            .publish_puffs_into(camera, ship, &mut frame.funnel_gas);
        frame.fire.clear();
        self.fire
            .publish_sprites_into(camera, None, &mut frame.fire);
        frame.soot.clear();
        self.blasts
            .soot
            .publish_gas_into(camera, ship, &mut frame.soot);
        self.blasts.debris.publish_into(camera, &mut frame.debris);
        self.blasts
            .debris
            .publish_casters_into(&mut frame.debris_casters);
        // The flotsam on the water is drawn as the debris is; it casts no shadow worth its draw (flat on the sea).
        self.losses
            .flotsam
            .publish_into(Some(camera), self.swell.as_ref(), &mut frame.debris);
        // The four brightest flashes light the gas (the web's four).
        let mut order: [usize; FLASH_LIGHTS] = std::array::from_fn(|i| i);
        order.sort_unstable_by(|&a, &b| {
            self.lights[b]
                .intensity
                .total_cmp(&self.lights[a].intensity)
        });
        frame.flashes = std::array::from_fn(|k| {
            let light = &self.lights[order[k]];
            [
                light.position.x as f32,
                light.position.y as f32,
                light.position.z as f32,
                (light.intensity * FLASH_ON_SMOKE * self.lighting.flash_smoke) as f32,
            ]
        });
        frame.flash_color = self.smoke_flash_color(&order[..SMOKE_FLASHES]);
        self.optics_ship = hidden;
    }

    /// Trial (shadows): every puff of smoke that stops the sun's light, in the camera's view or not, as the shadow
    /// map's casters (`shadow.rs`): the gas pools, the fires' columns and bodies, the funnels' billows and trails.
    pub(crate) fn shadow_blobs(&self, out: &mut Vec<crate::shadow::Blob>) {
        let hidden = self.optics_ship.as_deref();
        let sizes = self.trial.flipbook.then_some(&self.flipbook_sizes);
        for pool in [
            &self.smoke,
            &self.aircraft_smoke,
            &self.flak_smoke,
            &self.blasts.soot,
        ] {
            crate::shadow::gas_blobs(pool, hidden, sizes, out);
        }
        self.fires.shadow_blobs(hidden, out);
        self.funnels.shadow_blobs(hidden, out);
    }

    /// The burning-aircraft smoke pool and the shared fire pool, for a trail emitter outside this crate
    /// (naval-aircraft's `SmokeTrails`, the port of the web's `CombatEffects.updateAircraftSmoke`). Emit after
    /// `update` has advanced the pools for the frame.
    pub fn aircraft_pools(&mut self) -> (&mut ParticlePool, &mut ParticlePool) {
        (&mut self.aircraft_smoke, &mut self.fire)
    }

    /// An airframe coming apart in the air (naval-aircraft's `Wreckage`, called after [`CombatEffects::update`]): its
    /// smoke, flame and pieces go into the blasts' pools and the strike debris, its flash lights the next update.
    pub fn airframe_burst(&mut self, burst: &crate::blasts::AirframeBurst) {
        let mut random = Lcg::from_number(burst.seed);
        let mut pools = BlastPools {
            volumes: &mut self.smoke,
            fire: &mut self.fire,
            flashes: &mut self.requests,
        };
        self.blasts.airframe(burst, &mut random, &mut pools);
    }

    /// The night's lift this frame and how far the night has come ([`EffectLighting::set_lift`]), the fires' night
    /// levels the look's ([`FIRES_AT_NIGHT`], or [`BRIGHT_FLAMES_AT_NIGHT`] under `bright-flames`).
    pub fn set_lift(&mut self, lift: f64, night: f64) {
        let fires = if self.trial.bright_flames {
            BRIGHT_FLAMES_AT_NIGHT
        } else {
            FIRES_AT_NIGHT
        };
        self.lighting.set_lift_with(lift, night, fires);
    }

    /// Fraction of combat particles emitted (the combat effects setting); funnel smoke has its own rate.
    pub fn set_density(&mut self, density: f64) {
        self.smoke.density = density;
        self.flak_smoke.density = density;
        self.aircraft_smoke.density = density;
        self.fire.density = density;
        self.fires.set_density(density);
        self.blasts.set_density(density);
    }

    /// Fraction of funnel smoke emitted (the web game's `ShipFunnelSmoke.density`); the client sets it from the combat
    /// effects setting too.
    pub fn set_funnel_density(&mut self, density: f64) {
        self.funnels.density = density;
    }

    /// Every funnel's plume spine ([`FunnelSmoke::spines`]).
    pub fn funnel_spines(&self) -> Vec<crate::funnel::FunnelSpine> {
        self.funnels.spines()
    }

    /// Every burning fire's smoke column ([`LocalizedFires::columns`]).
    pub fn fire_columns(&self) -> Vec<crate::fires::FireColumn> {
        self.fires.columns()
    }

    pub fn counts(&self) -> EffectCounts {
        let (funnel_billows, funnel_segments) = self.funnels.counts();
        EffectCounts {
            gas: self.smoke.alive() + self.tactical.smoke.len(),
            fire: self.fire.alive(),
            soot: self.blasts.soot.alive(),
            smoulders: self.blasts.smoulders(),
            flak: self.flak_smoke.alive(),
            aircraft_smoke: self.aircraft_smoke.alive(),
            fires: self.fires.counts(),
            funnel_billows,
            funnel_segments,
            debris: self.blasts.debris.alive(),
            events: self.events_seen,
        }
    }

    /// The last event sequence read: events at or below it are skipped.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn set_sequence(&mut self, sequence: u64) {
        self.sequence = sequence;
    }

    /// A digest of every live particle's state, for determinism tests.
    pub fn digest(&self) -> u64 {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |value: f64| {
            for byte in value.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for pool in [
            &self.smoke,
            &self.flak_smoke,
            &self.aircraft_smoke,
            &self.fire,
            &self.blasts.soot,
        ] {
            for p in pool.particles() {
                if p.age < p.life {
                    mix(p.position.x);
                    mix(p.position.y);
                    mix(p.position.z);
                    mix(p.age);
                    mix(p.size);
                    mix(p.color.r);
                }
            }
        }
        hash
    }

    pub fn reset(&mut self) {
        self.smoke.reset();
        self.flak_smoke.reset();
        self.aircraft_smoke.reset();
        self.fire.reset();
        self.blasts.reset();
        self.fires.reset();
        self.funnels.reset();
        self.losses.reset();
        self.sea_fires.reset();
        self.cookoffs.reset();
        self.lights = [FlashLight::default(); FLASH_LIGHTS];
        self.light_cursor = 0;
        self.requests.clear();
        self.airbursts.clear();
        self.sequence = 0;
        self.events_seen = 0;
        self.volumes.reset();
    }

    /// What the volume trial's last publish held and cost.
    pub fn volume_stats(&self) -> crate::volume::VolumeStats {
        self.volumes.stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use naval_render::{Camera, FrameTime};

    /// By day and through dusk every effect takes the whole lift; after dark the fires keep their night's levels of their
    /// day's glow and light, and the flashes' light on the scene and the smoke their shares of the lift; between them as
    /// the night comes.
    #[test]
    fn fires_and_the_flashes_light_take_their_share_of_the_night() {
        let mut lighting = EffectLighting::default();
        let levels = |l: &EffectLighting| {
            (
                l.lift,
                l.flames,
                l.glow,
                l.firelight,
                l.flash_glow,
                l.flashlight,
                l.flash_smoke,
            )
        };
        lighting.set_lift(1.0, 0.0);
        assert_eq!(levels(&lighting), (1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0));
        // By day the hulls do not march toward the fires' light; after dark they do.
        assert!(lighting.firelight <= FIRELIGHT_MARCHED);
        // Dusk: the twilight's lift, all of it.
        lighting.set_lift(3.4, 0.0);
        assert_eq!(levels(&lighting), (3.4, 3.4, 3.4, 3.4, 3.4, 3.4, 3.4));
        let night = f64::from(naval_render::lights::DAY_LUX);
        lighting.set_lift(night, 1.0);
        assert!((lighting.flames - FLAMES_AT_NIGHT).abs() < 1e-9);
        assert!((lighting.glow - FIRE_GLOW_AT_NIGHT).abs() < 1e-9);
        assert!((lighting.firelight - FIRELIGHT_AT_NIGHT).abs() < 1e-6);
        assert!(lighting.firelight > FIRELIGHT_MARCHED);
        assert!((lighting.flash_glow - FLASH_GLOW_AT_NIGHT).abs() < 1e-9);
        assert!((lighting.flashlight - night * FLASHLIGHT_AT_NIGHT).abs() < 1e-6);
        assert!((lighting.flash_smoke - night * FLASH_SMOKE_AT_NIGHT).abs() < 1e-6);
        lighting.set_lift(night, 0.5);
        assert!(lighting.flames > FLAMES_AT_NIGHT && lighting.flames < night);
        assert!(
            lighting.flash_smoke > night * FLASH_SMOKE_AT_NIGHT && lighting.flash_smoke < night
        );
    }

    /// The effects take the weather's wind whole (it was 0.35 of it), and it freshens with height: a tenth less on
    /// the water than at ten metres, a quarter more at a hundred, the clouds' drift above.
    #[test]
    fn smoke_drifts_with_the_weathers_wind_at_its_height() {
        let mut lighting = EffectLighting::default();
        lighting.set_wind(9.0, std::f64::consts::FRAC_PI_2);
        assert!((lighting.wind - DVec3::new(0.0, 0.0, 9.0)).length() < 1e-9);
        assert!((wind_share(10.0) - 1.0).abs() < 1e-12);
        assert!((wind_share(0.0) - 0.84).abs() < 0.01);
        assert!((wind_share(100.0) - 1.29).abs() < 0.01);
        assert_eq!(wind_share(600.0), WIND_ALOFT);
        let mut last = 0.0;
        for height in [2.0, 5.0, 20.0, 50.0, 137.0, 140.0, 1000.0] {
            let share = wind_share(height);
            assert!(share >= last, "{height} m");
            last = share;
        }
    }

    #[test]
    fn aircraft_crashes_on_land_emit_fire_and_smoke_at_the_surface() {
        let camera = Camera::look_at(
            glam::Vec3::new(0.0, 400.0, 100.0),
            glam::Vec3::new(0.0, 350.0, 0.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        let mut effects = CombatEffects::new();
        let mut event = CombatEvent {
            kind: "aircraft-crash".into(),
            sequence: 1,
            position: [0.0, 350.0, 0.0],
            normal: Some([0.0, 1.0, 0.0]),
            ..Default::default()
        };
        assert!(!event.spawns_gas_or_fire());
        effects.update(1.0 / 60.0, 0.0, &view, &[event.clone()], &[]);
        assert_eq!(effects.counts().fire, 0);
        event.sequence = 2;
        event.detonation = Some(true);
        event.blast_radius_m = Some(12.0);
        assert!(event.spawns_gas_or_fire());
        effects.update(1.0 / 60.0, 0.0, &view, &[event], &[]);
        assert!(effects.counts().fire > 0);
        assert!(effects.counts().gas > 0);
        assert!((effects.lights[0].position.y - (350.0 + 2.0 * 2.2 * 0.55)).abs() < 1e-9);
    }

    #[test]
    fn timed_flak_flashes_in_its_own_frame_then_hangs_and_a_late_frame_draws_it_at_its_age() {
        use crate::events::{AirburstInfo, AircraftInfo};
        let event = CombatEvent {
            sequence: 1,
            tick: 0,
            kind: "aircraft-fire".into(),
            aircraft: Some(AircraftInfo {
                target: Some([0.0, 50.0, 0.0]),
                airburst: Some(AirburstInfo {
                    flight_time: 1.0,
                    caliber_m: 0.127,
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let camera = Camera::look_at(
            glam::Vec3::new(0.0, 50.0, 300.0),
            glam::Vec3::new(0.0, 50.0, 0.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        let mut effects = CombatEffects::new();
        effects.update(0.0, 0.0, &view, std::slice::from_ref(&event), &[]);
        assert_eq!(effects.counts().flak, 0);
        effects.update(1.0, 1.0, &view, &[], &[]);
        assert_eq!(effects.counts().flak, 1, "one cloud of soot a round");
        assert!(
            effects.lights.iter().any(|light| light.intensity > 0.0),
            "the flash lights the frame it bursts in"
        );
        for second in 2..=31 {
            effects.update(1.0, f64::from(second), &view, &[], &[]);
        }
        assert_eq!(effects.counts().flak, 1, "still hanging half a minute on");
        let gas = effects.publish(&view).gas;
        let soot = gas
            .iter()
            .find(|puff| puff.extra[3] == crate::instances::GAS_FLAK)
            .expect("drawn");
        assert!(soot.extra[0] > 0.3, "{}", soot.extra[0]);
        effects.update(90.0, 121.0, &view, &[], &[]);
        assert_eq!(effects.counts().flak, 0, "gone two minutes on");
        let mut late = CombatEffects::new();
        late.update(0.0, 9.0, &view, &[event], &[]);
        assert_eq!(late.counts().flak, 1);
        assert!(
            late.lights.iter().all(|light| light.intensity == 0.0),
            "no old flash replayed"
        );
    }

    #[test]
    fn a_turrets_blast_is_handed_on_once_and_its_fireball_glows_after_the_flash() {
        use crate::events::ShellInfo;
        let shot = |sequence: u64, x: f64| CombatEvent {
            sequence,
            tick: 10,
            kind: "shot".into(),
            ship_id: "player".into(),
            position: [x, 14.0, -60.0],
            shell: Some(ShellInfo {
                id: 1000 + sequence as i64,
                caliber_m: 0.406,
                velocity: [820.0, 30.0, 0.0],
                shell_type: Some("AP".into()),
                explosive_kg: None,
            }),
            ..Default::default()
        };
        let camera = Camera::look_at(
            glam::Vec3::new(-150.0, 40.0, 60.0),
            glam::Vec3::new(40.0, 14.0, -60.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        let mut effects = CombatEffects::new();
        let salvo = [shot(1, 0.0), shot(2, 1.2), shot(3, 2.4)];
        effects.update(1.0 / 60.0, 10.0 / 60.0, &view, &salvo, &[]);
        // Three barrels of one turret: one blast, at their muzzles' centre, along the line of fire.
        let blasts = effects.turret_blasts();
        assert_eq!(blasts.len(), 1);
        let blast = &blasts[0];
        assert_eq!(
            (blast.ship_id.as_str(), blast.barrels, blast.sequence),
            ("player", 3, 1)
        );
        assert_eq!(blast.caliber_m, 0.406);
        assert!((blast.muzzle - DVec3::new(1.2, 14.0, -60.0)).length() < 1e-9);
        assert!((blast.bore - DVec3::new(820.0, 30.0, 0.0).normalize()).length() < 1e-9);
        // Handed on once.
        effects.update(1.0 / 60.0, 11.0 / 60.0, &view, &[], &[]);
        assert!(effects.turret_blasts().is_empty());
        let scale: f64 = 0.406 / 0.38;
        let power = 3500.0 * scale.powf(0.8).powi(2) * 3f64.sqrt();
        // The flash's light is held at its full power while its ball burns white.
        for frame in 0..3 {
            effects.update(1.0 / 60.0, f64::from(12 + frame) / 60.0, &view, &[], &[]);
        }
        let held = effects.point_lights();
        assert_eq!(held.len(), 1);
        assert!(
            held[0].intensity > power,
            "{} of {power}",
            held[0].intensity
        );
        // 0.35 s on the flash (held 0.1 s × √(its size), then 0.22 s) is spent and the fireball still glows: its
        // share of the flash's power, falling over its seconds × √(the flash's size), in the fires' orange.
        for frame in 3..20 {
            effects.update(1.0 / 60.0, f64::from(12 + frame) / 60.0, &view, &[], &[]);
        }
        let glow = crate::blasts::FIREBALL_GLOW
            * (-(21.0 / 60.0) / (crate::blasts::FIREBALL_GLOW_SECONDS * scale.powf(1.35).sqrt()))
                .exp();
        let lights = effects.point_lights();
        assert_eq!(lights.len(), 1);
        assert!((lights[0].intensity / (power * glow) - 1.0).abs() < 1e-6);
        let (color, orange) = (lights[0].color, Rgb::hex(0xff8a3c));
        assert!(
            (color.r - orange.r).abs() + (color.g - orange.g).abs() + (color.b - orange.b).abs()
                < 1e-9
        );
        // Out within a second.
        for frame in 0..45 {
            effects.update(1.0 / 60.0, f64::from(32 + frame) / 60.0, &view, &[], &[]);
        }
        assert!(effects.point_lights().is_empty());
    }

    #[test]
    fn the_flashes_light_the_smoke_in_their_own_colour() {
        use crate::events::ShellInfo;
        let shot = |sequence: u64, x: f64| CombatEvent {
            sequence,
            tick: 10,
            kind: "shot".into(),
            ship_id: "player".into(),
            position: [x, 14.0, -60.0],
            shell: Some(ShellInfo {
                id: 1000 + sequence as i64,
                caliber_m: 0.38,
                velocity: [820.0, 30.0, 0.0],
                shell_type: Some("AP".into()),
                explosive_kg: None,
            }),
            ..Default::default()
        };
        let camera = Camera::look_at(
            glam::Vec3::new(-150.0, 40.0, 60.0),
            glam::Vec3::new(40.0, 14.0, -60.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        let mut effects = CombatEffects::new();
        let near = |a: [f32; 3], b: Rgb| {
            (f64::from(a[0]) - b.r).abs()
                + (f64::from(a[1]) - b.g).abs()
                + (f64::from(a[2]) - b.b).abs()
                < 1e-5
        };
        // With nothing burning, the flash's own.
        assert!(near(
            effects.publish(&view).flash_color,
            Rgb::hex(FLASH_COLOR)
        ));
        effects.update(
            1.0 / 60.0,
            10.0 / 60.0,
            &view,
            &[shot(1, 0.0), shot(2, 1.2)],
            &[],
        );
        // As the flash's light on the hulls and the sea: its 2,500 K, not the old #ffd29a's cream.
        let check = |effects: &mut CombatEffects| {
            let frame = effects.publish(&view);
            let light = effects.point_lights()[0].color;
            assert!(
                near(frame.flash_color, light),
                "{:?} against {light:?}",
                frame.flash_color
            );
            frame.flash_color
        };
        let flash = check(&mut effects);
        assert!(flash[2] < 0.15 && flash[1] < 0.45, "{flash:?}");
        // Reddening to the fireball's glow as the flash fades (held 0.1 s, then spent over 0.22 s).
        for frame in 0..21 {
            effects.update(1.0 / 60.0, f64::from(11 + frame) / 60.0, &view, &[], &[]);
        }
        assert!(near(check(&mut effects), Rgb::hex(GLOW_COLOR)));
    }

    #[test]
    fn a_turrets_fireball_burns_as_propellant_and_leaves_a_grey_cloud_that_lingers() {
        use crate::events::ShellInfo;
        use crate::instances::{GAS_FIREBALL, GAS_PROPELLANT, GasPuff};
        let camera = Camera::look_at(
            glam::Vec3::new(-300.0, 150.0, 300.0),
            glam::Vec3::new(50.0, 14.0, -60.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        // Three barrels of one turret of `caliber_m`, fired at tick 10.
        let fired = |caliber_m: f64| {
            let shot = |sequence: u64, x: f64| CombatEvent {
                sequence,
                tick: 10,
                kind: "shot".into(),
                ship_id: "player".into(),
                position: [x, 14.0, -60.0],
                shell: Some(ShellInfo {
                    id: 1000 + sequence as i64,
                    caliber_m,
                    velocity: [820.0, 10.0, 0.0],
                    shell_type: Some("AP".into()),
                    explosive_kg: None,
                }),
                ..Default::default()
            };
            let mut effects = CombatEffects::new();
            effects.update(
                1.0 / 60.0,
                10.0 / 60.0,
                &view,
                &[shot(1, 0.0), shot(2, 1.2), shot(3, 2.4)],
                &[],
            );
            (effects, 10u32)
        };
        // The gas `seconds` after the salvo, stepped there at 60 Hz.
        fn at(
            (effects, tick): &mut (CombatEffects, u32),
            view: &CameraView,
            seconds: f64,
        ) -> Vec<GasPuff> {
            while f64::from(*tick - 10) / 60.0 < seconds {
                *tick += 1;
                effects.update(1.0 / 60.0, f64::from(*tick) / 60.0, view, &[], &[]);
            }
            effects.publish(view).gas
        }
        // Grey: no channel more than a fifth from the others' mean.
        let grey = |p: &GasPuff| {
            let mean = (p.tint[0] + p.tint[1] + p.tint[2]) / 3.0;
            p.tint[..3].iter().all(|c| (c - mean).abs() < mean * 0.2)
        };
        let mut heavy = fired(0.406);
        // A twentieth of a second on, every puff of the blast is the propellant's, its flash's white ball and its
        // fireball's lobes burning.
        let young = at(&mut heavy, &view, 0.05);
        assert!(
            young
                .iter()
                .all(|p| p.extra[3] == GAS_PROPELLANT || p.extra[3] == GAS_FIREBALL)
        );
        let flash = young
            .iter()
            .filter(|p| p.extra[3] == GAS_FIREBALL && p.state[2] > 1.0)
            .count();
        assert!(flash >= 9, "{flash} burning");
        // One white ball a turret, as tall as her superstructure: an Iowa's triple 16-inch turret's some 38 m across.
        let balls: Vec<_> = young
            .iter()
            .filter(|p| p.extra[3] == GAS_FIREBALL && p.state[2] > 1.4)
            .collect();
        assert_eq!(balls.len(), 1);
        assert!(
            (16.0..21.0).contains(&balls[0].sphere[3]),
            "a ball {} m across",
            balls[0].sphere[3] * 2.0
        );
        // Its fireball is red in pockets by a fifth of a second and out in about a third: no puff burns through.
        let pockets = at(&mut heavy, &view, 0.2);
        assert!(
            pockets.iter().all(|p| p.state[2] < 0.9),
            "a fireball still burning whole at 0.2 s"
        );
        let spent = at(&mut heavy, &view, 0.4);
        assert!(
            spent
                .iter()
                .filter(|p| p.extra[3] == GAS_FIREBALL)
                .all(|p| p.state[2] < 0.05),
            "a fireball still burning at 0.4 s"
        );
        // Its last flame is out within a second, as films of battleships' broadsides show it.
        let out = at(&mut heavy, &view, 0.8);
        assert!(out.iter().all(|p| p.state[2] < 0.01));
        let smoke = at(&mut heavy, &view, 3.0);
        assert!(smoke.iter().all(|p| p.state[2] < 0.01 && grey(p)));
        // Five seconds on its cloud is the films' size: billows under 30 m across their middle and a bank under 140 m
        // along the line of fire (until 2026-10-23 they grew to 45–100 m and the bank to some 200 m).
        let grown = at(&mut heavy, &view, 5.0);
        let standing: Vec<_> = grown.iter().filter(|p| p.extra[0] > 0.2).collect();
        assert!(
            standing.iter().all(|p| p.sphere[3] < 30.0),
            "a billow wider than the films'"
        );
        let ends = standing
            .iter()
            .fold((f32::MAX, f32::MIN), |(near, far), p| {
                (
                    near.min(p.sphere[0] - p.sphere[3]),
                    far.max(p.sphere[0] + p.sphere[3]),
                )
            });
        assert!(ends.1 - ends.0 < 140.0, "a bank {} m long", ends.1 - ends.0);
        // Eight seconds on, its grey cloud still stands, thinning.
        let cloud = at(&mut heavy, &view, 8.0);
        let standing: Vec<_> = cloud.iter().filter(|p| p.extra[0] > 0.2).collect();
        assert!(standing.len() >= 4, "{} standing", standing.len());
        assert!(standing.iter().all(|p| grey(p)));
        // And it is gone in twenty seconds.
        assert!(at(&mut heavy, &view, 20.0).is_empty());
        // A destroyer's 12.7 cm mount leaves a smaller cloud, gone in ten seconds.
        let mut light = fired(0.127);
        let small = at(&mut light, &view, 3.0);
        assert!(!small.is_empty() && small.len() < smoke.len());
        assert!(at(&mut light, &view, 10.0).is_empty());
    }

    #[test]
    fn aircraft_smoke_is_drawn_with_the_combat_gas() {
        let camera = Camera::look_at(
            glam::Vec3::new(0.0, 50.0, 300.0),
            glam::Vec3::new(0.0, 50.0, 0.0),
        );
        let view = CameraView::from_uniforms(&camera.uniforms(1280, 720, FrameTime::default()));
        let mut effects = CombatEffects::new();
        effects.update(1.0 / 60.0, 0.0, &view, &[], &[]);
        assert!(effects.publish(&view).gas.is_empty());
        let (smoke, fire) = effects.aircraft_pools();
        let puff = smoke.emit(DVec3::new(0.0, 50.0, 0.0), None);
        puff.size = 3.0;
        puff.life = 6.0;
        puff.opacity = 0.8;
        fire.emit(DVec3::new(0.0, 50.0, 0.0), None).size = 2.0;
        let frame = effects.publish(&view);
        assert_eq!(frame.gas.len(), 1);
        assert_eq!(frame.fire.len(), 1);
        assert_eq!(effects.counts().aircraft_smoke, 1);
        effects.reset();
        assert_eq!(effects.counts().aircraft_smoke, 0);
    }
}
